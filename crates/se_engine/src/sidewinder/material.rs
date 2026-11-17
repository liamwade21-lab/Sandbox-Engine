//! The ship's material (ARCHITECTURE.md A6): `ShipMaterial` with `sidewinder_ship.wesl`, one shader family per Elite effect
//! (`Family`): each material's surface is worked out by its family's port of Elite's G-buffer shader
//! (`sidewinder_surface_hull.wesl` for the outside, `sidewinder_surface_cockpit.wesl` for the cockpit; their headers name every
//! variant and why) or, for the glass, of its forward shader (`sidewinder_glass.wesl`), from Elite's own textures and
//! attributes, and lit by Elite's lit maths (`sidewinder_lit.wesl`) with the sandbox's light record
//! (`crate::sandbox::light::PropLight`, the same two buffers `PropMaterial` binds) and the ship's own lights buffer
//! (`ShipLights`, A6's contract).
//!
//! Made once per Elite material at the first spawn, twice for a material both the outside and the seat's view use (the
//! seat's draws in its own depth slice: `SEAT_SLICE`), and once more for a skinned model's draws (`ShipMaterial::skinned`:
//! the pilot chair seen from the seat); never changed after: per-frame values live in the storage buffers (`ShipLights`,
//! `ShipScene`, `ShipGlow`, which `lights.rs` writes in place, and `ShipGlass` and `ShipPalette`, which `draw.rs` does).

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{UVec4, Vec4};
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::*;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use se_core::elite_ship::{Material as EliteMaterial, Paint, SlotValue};

use crate::sandbox::light::PropLight;

/// Elite's third texture coordinate (`DecodedVertex::uv[2]`: the paint mask's on the painted hull). Position, normal,
/// tangent (with its sign in w), UV0 and UV1 are Bevy's own attributes.
pub const ATTRIBUTE_UV_2: MeshVertexAttribute = MeshVertexAttribute::new("ShipUv2", 0x5E5D_0002, VertexFormat::Float32x2);

/// Elite's vertex colour (`DecodedVertex::colour`, bytes 20-23 / 255): only the glass's shaders read it (its masks), so only
/// the glass's meshes carry it.
pub const ATTRIBUTE_COLOUR: MeshVertexAttribute = MeshVertexAttribute::new("ShipColour", 0x5E5D_0003, VertexFormat::Unorm8x4);

/// The vertex's bone (`Edm::vertex_bone`: an index into its model's bones, the palette's order): only the skinned model's
/// meshes carry it (the pilot chair seen from the seat, whose vertices each hang on one bone), and only a skinned material
/// (`ShipMaterial::skinned`) reads it.
pub const ATTRIBUTE_BONE: MeshVertexAttribute = MeshVertexAttribute::new("ShipBone", 0x5E5D_0004, VertexFormat::Uint32);

/// Elite's shader families not drawn yet, by a piece of the shader's name, and why: each comes with its own shader in a later
/// task.
pub const NOT_YET_SHADERS: &[(&str, &str)] = &[
    ("ShieldAlpha_", "the shield bubble"),
    ("Schematic_", "the HUD's hologram"),
    ("EDGeometryShadow", "the shadow caster"),
    ("EmissiveMapPulse", "the tutorial's highlight overlays"),
];

/// Materials of a built family not drawn yet, and why.
pub const NOT_YET_MATERIALS: &[(&str, &str)] = &[];

/// Models not drawn yet, by the start of the model's file name, and why.
pub const NOT_YET_MODELS: &[(&str, &str)] = &[
    ("shard0", "the canopy blowout's 20 shards"),
    ("sidewinder_schematic", "the HUD's hologram"),
    ("sidewinder_shield", "the shield bubble"),
    ("cockpit_ui_hightlight_", "the tutorial's highlight overlays"),
    ("cockpitchair_bone_rig", "a 3-vertex placeholder"),
    ("commandercreator_cockpit_low", "a 4-vertex placeholder"),
];

/// The spot lights' IES profile: the one the ship's spot lights name (the headlights', `SpotLightActivity`).
pub const IES_FILE: &str = "tex/ies_emergency_elmre_lp220l_fxo.dds";

/// The painted hull's slot the engine fills with the paint job's mask at run time (the material's own value is a
/// placeholder constant; `ship.json`'s paint record names the texture).
const PAINT_MASK_SLOT: &str = "pPaintPrimary_ColourTexture";

/// Is this model drawn in this task?
pub fn model_drawn(model: &str) -> bool {
    !NOT_YET_MODELS.iter().any(|(start, _)| model.starts_with(start))
}

/// Is this material drawn in this task? Not a name Elite draws with its fallback (`fallback_to`), nor a family or material
/// above, nor a customisation decal slot whose maths shows nothing (`decal_slot_coverage`).
pub fn material_drawn(name: &str, m: &EliteMaterial, paint: &Paint) -> bool {
    let Some(shader) = m.shader.as_deref() else { return false };
    if m.fallback_to.is_some() || NOT_YET_SHADERS.iter().any(|(part, _)| shader.contains(part)) || NOT_YET_MATERIALS.iter().any(|(n, _)| name.eq_ignore_ascii_case(n)) {
        return false;
    }
    Family::of(shader) != Family::Decal || decal_slot_coverage(m, paint).is_none_or(|c| c > 0.0)
}

/// The Elite shader families: the outside's (`sidewinder_surface_hull.wesl`), the cockpit's (`sidewinder_surface_cockpit.wesl`),
/// the glass (`sidewinder_glass.wesl`), the customisation decals (drawn only where their maths can show something), and
/// `Plain` for the families not built yet (their base colour alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Painted,
    SingleTile,
    Opaque,
    Emissive,
    Overlay,
    Stamp,
    Decal,
    /// The chair's body and the cockpit's bespoke parts (`..._CockpitOpaqueDeferred_BaseColourOverride`).
    Cockpit,
    /// The consoles and the chair's controls (`..._BaseColourOverride_Emissive`).
    CockpitEmissive,
    /// The cockpit's tiles (`..._BaseColourOverride_DeferredAOMapUV2_RoughnessUV1`).
    CockpitTile,
    /// The canopy seen from the seat (`CockpitGlass`).
    GlassCockpit,
    /// The canopy seen from outside (`GlassSimple_Tintable_Roughness_Alpha`).
    GlassTinted,
    /// The canopy's edges (`GlassSimple_Roughness_Alpha_SingleSource`).
    GlassEdges,
    Plain,
}

impl Family {
    pub fn of(shader: &str) -> Family {
        match shader {
            "Metalness_Roughness_OpaqueDeferred_SpacecraftTilePainted_Normals" => Family::Painted,
            "Metalness_Roughness_OpaqueDeferred_SpacecraftSingleTile" => Family::SingleTile,
            "Metalness_Roughness_OpaqueDeferred" => Family::Opaque,
            "Metalness_Roughness_OpaqueDeferred_Emissive" => Family::Emissive,
            "Metalness_Roughness_AlphaDeferred_Overlay" => Family::Overlay,
            "DetailStamp_NormalAO" => Family::Stamp,
            "Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride" => Family::Cockpit,
            "Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_Emissive" => Family::CockpitEmissive,
            "Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_DeferredAOMapUV2_RoughnessUV1" => Family::CockpitTile,
            "CockpitGlass" => Family::GlassCockpit,
            "GlassSimple_Tintable_Roughness_Alpha" => Family::GlassTinted,
            "GlassSimple_Roughness_Alpha_SingleSource" => Family::GlassEdges,
            s if s.contains("CustomisationDecals") => Family::Decal,
            _ => Family::Plain,
        }
    }

    /// The glass: drawn by its own entry points (`vertex_glass`, `fragment_glass`), see-through, with the vertex's colour.
    pub fn is_glass(self) -> bool {
        matches!(self, Family::GlassCockpit | Family::GlassTinted | Family::GlassEdges)
    }

    /// The shader def that compiles this family's surface (`sidewinder_ship.wesl`'s `@if`s).
    fn def(self) -> &'static str {
        match self {
            Family::Painted => "SHIP_PAINTED",
            Family::SingleTile => "SHIP_SINGLE_TILE",
            Family::Opaque => "SHIP_OPAQUE",
            Family::Emissive => "SHIP_EMISSIVE",
            Family::Overlay => "SHIP_OVERLAY",
            Family::Stamp => "SHIP_STAMP",
            Family::Cockpit => "SHIP_COCKPIT",
            Family::CockpitEmissive => "SHIP_COCKPIT_EMISSIVE",
            Family::CockpitTile => "SHIP_COCKPIT_TILE",
            Family::GlassCockpit => "SHIP_GLASS_COCKPIT",
            Family::GlassTinted => "SHIP_GLASS_TINTED",
            Family::GlassEdges => "SHIP_GLASS_EDGES",
            Family::Decal | Family::Plain => "SHIP_PLAIN",
        }
    }

    /// The texture slots its surface function takes, in its binding order (tex0, tex1, ...).
    pub fn slots(self) -> &'static [&'static str] {
        match self {
            Family::Painted => &["pNormalMapUV1", "pDirt_Flake_0", "pBaseColour_Roughness_0", "pBaseColour_Roughness_1", PAINT_MASK_SLOT, "pDirtMaskMaterialMaskScruffinessAO"],
            Family::SingleTile => &["pNormalTexture", "pDirt_Flake_0", "pBaseColour_Roughness_1", "pDirtMaskMaterialMaskScruffinessAO"],
            Family::Opaque => &["pNormalTexture", "pMetalnessMap", "pBaseColour_Roughness"],
            Family::Emissive => &["pNormalTexture", "pMetalnessMap", "pBaseColour_Roughness", "pEmissiveTexture"],
            Family::Overlay => &["pMetalnessAlphaMap", "pBaseColour_Roughness"],
            Family::Stamp => &["pNormalDetailMap", "pAOMap"],
            Family::Cockpit => &["pNormalTexture", "pMetalnessMap", "pBaseColour_Roughness"],
            Family::CockpitEmissive => &["pNormalTexture", "pMetalnessMap", "pBaseColour_Roughness", "pEmissiveTexture"],
            Family::CockpitTile => &["pNormalTexture", "pMetalnessMap", "pBaseColour_Roughness", "pNormalDetailMap", "pRoughnessDetailMap", "pAOMap"],
            // (Its other four maps, the cracks' interior normal and ID and the ice's and condensation's, are read only by the
            // parts not ported: sidewinder_glass.wesl's header.)
            Family::GlassCockpit => &["pPackedCockpitGlassBlemishAlphaMaps", "pPackedCracksNormalAlpha"],
            Family::GlassTinted | Family::GlassEdges => &["pNormalTexture", "pMetalnessAlphaMap", "pBaseColour_Roughness"],
            Family::Decal | Family::Plain => &[],
        }
    }

    /// The attributes its variant reads, into `ShipParams` (the painted hull's paint constants come from the paint record).
    fn reads(self) -> &'static [&'static str] {
        const TILES: &[&str] = &["pDiffuseGlossTweak_UV2", "pDirtDiffuse", "pDirtDiffuseOverride", "pDirtLevelScale", "pDirtMapScales", "pLayerBottom_BaseColourOverride", "pTile1Metalness", "pDeferredFlags"];
        match self {
            Family::Painted => &[
                "pDiffuseGlossTweak_UV2",
                "pDirtDiffuse",
                "pDirtDiffuseOverride",
                "pDirtLevelScale",
                "pDirtMapScales",
                "pLayerTop_BaseColourOverride",
                "pLayerBottom_BaseColourOverride",
                "pLayerMaskCutoff",
                "pLayerMaskUV1Scale",
                "pLayerMaskSoftness",
                "pTile1Metalness",
                "pDeferredFlags",
                "pEmissiveRescalingEnable",
                "pEmissiveRescalingType",
                "pEmissiveRescalingEVOffset",
                "pEmissiveRescalingEnableClamping",
                "pEmissiveRescalingEVClamps",
            ],
            Family::SingleTile => TILES,
            Family::Opaque => &["gStandardMapScales", "pDeferredFlags"],
            Family::Overlay => &["gStandardMapScales", "pDepthBias"],
            Family::Emissive => &[
                "gStandardMapScales",
                "pDeferredFlags",
                "pEmissiveColour",
                "pEmissiveScale",
                "pUVScaleOffset",
                "pEmissiveRescalingEnable",
                "pEmissiveRescalingType",
                "pEmissiveRescalingEVOffset",
                "pEmissiveRescalingEnableClamping",
                "pEmissiveRescalingEVClamps",
            ],
            Family::Stamp => &["pNormalDetailMapUVScale", "pNormalDetailMapStrength", "pDepthBias"],
            Family::Cockpit => &["gStandardMapScales", "pDeferredFlags", "pOverrideBaseColour"],
            Family::CockpitEmissive => &[
                "gStandardMapScales",
                "pDeferredFlags",
                "pOverrideBaseColour",
                "pEmissiveColour",
                "pEmissiveScale",
                "pUVScaleOffset",
                "pEmissiveRescalingEnable",
                "pEmissiveRescalingType",
                "pEmissiveRescalingEVOffset",
                "pEmissiveRescalingEnableClamping",
                "pEmissiveRescalingEVClamps",
            ],
            Family::CockpitTile => &[
                "gStandardMapScales",
                "pDeferredFlags",
                "pOverrideBaseColour",
                "pNormalDetailMapUVScale",
                "pNormalDetailMapStrength",
                "pRoughnessDetailMapUVScale",
                "pRoughnessDetailMapStrength",
                "pBaseColourDetailMapStrength",
            ],
            Family::GlassCockpit => GLASS_COCKPIT_READS,
            Family::GlassTinted => &["gStandardMapScales", "pGlassColourTint", "pGlassColourTint_Distant", "pDistanceToOpaque", "pFadeRange", "pFresnel_ScalePowerBias", "pFresnelVal"],
            Family::GlassEdges => &["gStandardMapScales"],
            Family::Decal | Family::Plain => &[],
        }
    }

    /// The attributes its variant does not read, by the start of their names, and why (`sidewinder_surface_hull.wesl`'s and
    /// `sidewinder_surface_cockpit.wesl`'s headers): nothing is dropped without being listed here.
    fn ignores(self) -> &'static [(&'static str, &'static str)] {
        const ATLAS: [(&str, &str); 3] = [("bfrAtlasModel", "the atlas models' vertex extrusion"), ("pVerticalTiling", "the atlas models' vertex extrusion"), ("pRenderLayerOverride", "the engine's draw order")];
        const MODULATION: (&str, &str) = (
            "pMaterialModulationSlot",
            "a slot of the ship's modulation record, which the mod has none of: without one the shader skips it whatever the slot (-1 on every material: `cockpit_override_is_identity`)",
        );
        match self {
            Family::Painted => &[
                ATLAS[0],
                ATLAS[1],
                ATLAS[2],
                ("pPaintPrimary_", "the paint job's globals replace them (ship.json's paint record)"),
                ("pPaint_WSToVesselSpaceTransform_", "the iridescence variants"),
                ("pPaintSecondary_", "the paint swap variant"),
                ("pPaintBlendPlaneWS", "the paint swap variant"),
                ("pSwapPaintEffect", "the paint swap variant"),
            ],
            Family::SingleTile => &[ATLAS[0], ATLAS[1], ATLAS[2], ("pLayerTop_BaseColourOverride", "not in the single tile's variant"), ("pLayerMask", "not in the single tile's variant")],
            Family::Opaque => &[ATLAS[0], ATLAS[1], ATLAS[2]],
            Family::Emissive => &[ATLAS[0], ATLAS[1], ATLAS[2], ("pClipVertices", "the vertex shader's")],
            Family::Overlay | Family::Stamp => &[ATLAS[0], ATLAS[1], ATLAS[2]],
            Family::Cockpit | Family::CockpitEmissive | Family::CockpitTile => &[ATLAS[0], ATLAS[1], ATLAS[2], MODULATION],
            Family::GlassCockpit => &[
                ATLAS[0],
                ATLAS[1],
                ATLAS[2],
                ("pUseCockpitIBL", "picks Elite's cockpit probe for the cracks' reflection, which is not ported (sidewinder_glass.wesl's header)"),
                ("pDefaultShadowBrightness", "not one of CockpitGlass_Forward's variables (the effect's constant table)"),
            ],
            Family::GlassTinted | Family::GlassEdges => &[
                ATLAS[0],
                ATLAS[1],
                ATLAS[2],
                ("pUseCockpitIBL", "picks which of Elite's probes the reflection reads; the sky light stands in for them (sidewinder_glass.wesl's header)"),
                ("pEmissiveRescaling", "declared by the effect, read by none of its shaders"),
            ],
            Family::Decal | Family::Plain => &[],
        }
    }
}

/// The canopy's attributes (`ps_CockpitGlass_Forward` and its vertex shader's), every one kept in `CockpitParams` (the parts
/// that read the ice's and condensation's are not ported yet: `sidewinder_glass.wesl`'s header).
const GLASS_COCKPIT_READS: &[&str] = &[
    "pCockpitTile",
    "pPSTB_GridTileRate",
    "pCockpitRefractionAmount",
    "pCracksVertexMask",
    "pIceVertexMask",
    "pDirtVertexMask",
    "pImperfectionVertexMask",
    "pCondensationVertexMask",
    "pIceMaskScaleOffset",
    "pImperfectionMaskScaleOffset",
    "pCondensationMaskScaleOffset",
    "pCondensationTile",
    "pCracksOcclusionScale",
    "pIceOcclusionScale",
    "pCondensationOcclusionScale",
    "pDirtAlphaMultiplier",
    "pCracksColour",
    "pCracksMaskThreshold",
    "pIceColour",
    "pIceSceneColourDistortion",
    "pCondensationColour",
    "pCondensationSceneColourDistortion",
    "pDirtDiffuse",
    "pLayerMaskCutoff",
    "pCracksRoughness",
    "pIceRoughness",
    "pCondensationRoughness",
    "pImperfectionsRoughness",
    "pDirtRoughness",
    "pGlassThickness",
    "pSmearsPeriods",
    "pSmearsScaleOffset",
];

/// Where a texture slot's texels come from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TexSource {
    /// A texture in the mod's folder, sampled as sRGB or not by its flag.
    File { file: String, srgb: bool },
    /// A constant colour: its bytes / 255, linear, as Elite's shaders decode a constant colour (the emissive shaders'
    /// material-parameter path: the three low bytes, `utof` x 1/255). The first of Elite's two RGBA8: in every material of
    /// the ship the second differs only in alpha, and no drawn family reads a constant's alpha where they differ (the normal
    /// maps' constants are read for x and y; the paint and decal colour slots are filled by the engine at run time).
    Constant([u8; 4]),
}

/// The texture of each of a material's family's slots (`Family::slots`), in order; the painted hull's paint mask from the
/// paint record.
pub fn sources(m: &EliteMaterial, paint: &Paint) -> Result<Vec<TexSource>, String> {
    let family = Family::of(m.shader.as_deref().unwrap_or(""));
    family
        .slots()
        .iter()
        .map(|&slot| {
            if family == Family::Painted && slot == PAINT_MASK_SLOT {
                // (B8G8R8A8_UNORM: not sRGB.)
                return Ok(TexSource::File { file: paint.texture_file.clone(), srgb: false });
            }
            let t = m.textures.iter().find(|t| t.slot == slot).ok_or_else(|| format!("it has no {slot}, which its family reads"))?;
            Ok(match &t.value {
                SlotValue::Tex { file, srgb, .. } => TexSource::File { file: file.clone(), srgb: *srgb },
                SlotValue::Colour { colour } => TexSource::Constant(colour[0]),
            })
        })
        .collect()
}

/// Where a plain material's base colour comes from (the families not built yet).
#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    /// A texture in the mod's folder, sampled as sRGB or not by its flag; its rgb is the colour.
    Texture { file: String, srgb: bool },
    /// One linear colour.
    Colour([f32; 3]),
}

/// A plain material's base colour: its `pBaseColour_Roughness`, else the tile families' first (`pBaseColour_Roughness_0`),
/// else `pDiffuse_AlphaTexture`. A constant slot's first RGBA8 is read as sRGB, as the textures in those slots are. None when
/// it has none of them.
pub fn base_colour(m: &EliteMaterial) -> Option<Base> {
    let slot = ["pBaseColour_Roughness", "pBaseColour_Roughness_0", "pDiffuse_AlphaTexture"].iter().find_map(|name| m.textures.iter().find(|t| t.slot == *name))?;
    Some(match &slot.value {
        SlotValue::Tex { file, srgb, .. } => Base::Texture { file: file.clone(), srgb: *srgb },
        SlotValue::Colour { colour } => Base::Colour([0, 1, 2].map(|i| srgb_to_linear(colour[0][i]))),
    })
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The textures a material's family reads, in `Family::slots`' order; a plain material its base colour's texture, if it is
/// one. An error names the slot its family reads that the material lacks.
pub fn textures_of(m: &EliteMaterial, paint: &Paint) -> Result<Vec<TexSource>, String> {
    if Family::of(m.shader.as_deref().unwrap_or("")) == Family::Plain {
        return Ok(match base_colour(m) {
            Some(Base::Texture { file, srgb }) => vec![TexSource::File { file, srgb }],
            _ => Vec::new(),
        });
    }
    sources(m, paint)
}

/// Every texture file the materials `named` read (`textures_of`), each once, and the IES profile. An error names the
/// material and the slot it lacks.
pub fn texture_files<'a>(named: impl Iterator<Item = (&'a str, &'a EliteMaterial)>, paint: &Paint) -> Result<Vec<(String, bool)>, String> {
    let mut files: Vec<(String, bool)> = vec![(IES_FILE.to_string(), true)];
    for (name, m) in named {
        for s in textures_of(m, paint).map_err(|e| format!("material {name}: {e}"))? {
            if let TexSource::File { file, srgb } = s {
                if !files.iter().any(|(f, s)| *f == file && *s == srgb) {
                    files.push((file, srgb));
                }
            }
        }
    }
    Ok(files)
}

/// A drawn material worked out once, when the mod's files are read (`resolve`): its family, its textures, its params (for
/// the outside; `ShipParams::in_seat` gives the seat's copy) and its `CockpitParams`.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub family: Family,
    pub textures: Vec<TexSource>,
    pub params: ShipParams,
    pub cockpit: CockpitParams,
}

/// Resolves material `name`: everything its family reads (slots and attributes) or an error naming the material and what
/// it lacks. `mask_size` is the paint job's mask's (the painted hull's filter reads it).
pub fn resolve(name: &str, m: &EliteMaterial, paint: &Paint, mask_size: UVec2) -> Result<Resolved, String> {
    let named = |e: String| format!("material {name}: {e}");
    let textures = textures_of(m, paint).map_err(named)?;
    let params = ShipParams::new(m, paint, mask_size, false, !textures.is_empty()).map_err(named)?;
    let cockpit = CockpitParams::new(m).map_err(named)?;
    Ok(Resolved { family: Family::of(m.shader.as_deref().unwrap_or("")), textures, params, cockpit })
}

// ---- The paint job ----

/// The paint job's RenderMode (R, G, B, A) and UseSharpening (R, G, B) as the exe builds them from texture block A
/// (FUN_142875b80; block B only once a high-quality stream has loaded, which the default's null name never does): R its
/// byte; G and B their byte, raised to 1 when below 2 and the previous key's sharpening flag is set; A 1 when the Blue
/// key's flag is set. A key's flag counts when its byte is above 0 as a signed char.
pub fn paint_modes(paint: &Paint) -> ([f32; 4], [f32; 3]) {
    let flag = |name: &str| paint.keys.iter().find(|k| k.name == name).and_then(|k| k.sharpening_flag).is_some_and(|f| (f as i8) > 0);
    let (red, green, blue) = (flag("RedKey"), flag("GreenKey"), flag("BlueKey"));
    let lift = |byte: u8, previous: bool| if byte < 2 && (byte == 1 || previous) { 1 } else { byte };
    let [r, g, b] = paint.texture_block_a.render_mode_rgb;
    let modes = [r, lift(g, red), lift(b, green), blue as u8].map(|m| m as f32);
    (modes, [red, green, blue].map(|f| if f { 1.0 } else { 0.0 }))
}

/// The paint rule on one mask texel (r, g, b, a in 0..1), as `sidewinder_surface_hull.wesl` `paint_weights` does it (the
/// single-texture path of the painted hull's shader): per channel by its render mode, 1 a hard threshold at 0.5, 2 none,
/// anything else the value as it is; sharpening takes a channel's minimum with the next; red first, green limited to what
/// red leaves, blue to what both leave, the background the remainder. The weights in the record's order: red, green, blue,
/// background.
#[cfg(test)]
pub fn paint_weights(texel: [f32; 4], modes: [f32; 4], sharpen: [f32; 3]) -> [f32; 4] {
    let modes = modes.map(|m| (m + 0.5).floor() as i32);
    if !modes.iter().any(|&m| m <= 1) {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let c: [f32; 4] = std::array::from_fn(|i| match modes[i] {
        1 => (texel[i] >= 0.5) as u8 as f32,
        2 => 0.0,
        _ => texel[i],
    });
    let sharp = sharpen.map(|s| s > 0.5);
    let red = if sharp[0] { c[1].min(c[0]) } else { c[0] };
    let green = if sharp[0] { 0.0 } else if sharp[1] { c[2].min(c[1]) } else { c[1] };
    let blue = if sharp[1] { 0.0 } else if sharp[2] { c[3].min(c[2]) } else { c[2] };
    let g = green.min(1.0 - red);
    let b = blue.min((1.0 - red - g).clamp(0.0, 1.0));
    [red, g, b, (1.0 - (red + g + b)).clamp(0.0, 1.0)]
}

// ---- The customisation decals ----

/// The most coverage a customisation decal slot's shader (`ps_..._CustomisationDecalsN_DeferredAlphaBlendEx_GBuffer0_0_0_0_0`)
/// can give anywhere, when no decal is fitted (its colour texture a constant): the mask's weights (each channel thresholded
/// at 0.5 where pDecalNUseDistanceField0 says so, sharpened where pDecalNUseSharpen0 says so, then red first, green, blue,
/// the fourth group the alpha or, where its distance field flag is set, the remainder) times the groups' alphas (the paint
/// job's: with no decal fitted none is overridden) times pDecalNAlpha0, times the shape mask's term, which is at most 1.
/// None when a decal is fitted (a texture in the slot) or a value is missing: then only drawing it tells.
pub fn decal_slot_coverage(m: &EliteMaterial, paint: &Paint) -> Option<f32> {
    let shader = m.shader.as_deref()?;
    let slot = shader.split("CustomisationDecals").nth(1)?.chars().next()?.to_digit(10)?;
    let colour = m.textures.iter().find(|t| t.slot == format!("pDecal{slot}Colour0Texture"))?;
    let SlotValue::Colour { colour } = &colour.value else { return None };
    let texel = colour[0].map(|c| c as f32 / 255.0);
    let attr = |name: String| m.attrs.iter().find(|a| a.name == name).map(|a| a.values.iter().map(|&v| v as f32).collect::<Vec<f32>>());
    let df = attr(format!("pDecal{slot}UseDistanceField0")).filter(|v| v.len() == 4)?;
    let sharpen = attr(format!("pDecal{slot}UseSharpen0")).filter(|v| v.len() == 3)?;
    let alpha0 = *attr(format!("pDecal{slot}Alpha0"))?.first()?;
    let dist: [bool; 4] = std::array::from_fn(|i| df[i] > 0.5);
    let sharp: [bool; 3] = std::array::from_fn(|i| sharpen[i] > 0.5);
    let c: [f32; 4] = std::array::from_fn(|i| if dist[i] { (texel[i] >= 0.5) as u8 as f32 } else { texel[i] });
    let first = [if sharp[0] { c[1].min(c[0]) } else { c[0] }, if sharp[1] { c[2].min(c[1]) } else { c[1] }, if sharp[2] { c[3].min(c[2]) } else { c[2] }];
    let plain = [first[0], if sharp[0] { 0.0 } else { first[1] }, if sharp[1] { 0.0 } else { first[2] }, if sharp[2] { 0.0 } else { c[3] }];
    let mut left = 1.0 - plain[0];
    let g = plain[1].min(left);
    left = (left - g).clamp(0.0, 1.0);
    let b = plain[2].min(left);
    left = (left - b).clamp(0.0, 1.0);
    let chained = [plain[0], g, b, plain[3].min(left)];
    let mut w: [f32; 4] = std::array::from_fn(|i| if dist[i] { chained[i] } else { plain[i] });
    if dist[3] {
        w[3] = (1.0 - (w[0] + w[1] + w[2])).clamp(0.0, 1.0);
    }
    if paint.decal_colour_groups.len() != 4 {
        return None;
    }
    let alpha: f32 = w.iter().zip(&paint.decal_colour_groups).map(|(w, g)| w * g.alpha as f32).sum();
    Some((alpha * alpha0).max(0.0))
}

// ---- The ship's wear and dirt ----

/// The ship's wear and dirt: Elite's per-ship parameter object (built by FUN_142ce6b50, vtable 0x145323ce8, applied to the
/// ship's materials by its slot FUN_142d4faf0) replaces three of the painted hull's and bare metal's attributes at run time,
/// whatever the material files say. Its owner writes it every update (FUN_142de6090, from FUN_142dc3bb0):
/// - pDirtLevelScale = clamp(lerp(dirt_lo, dirt_hi, blend) + wear, 0, 1) x 1.2 + 0.3;
/// - pLayerMaskCutoff = 0.95 - 0.9 wear (the bare metal shows through as the wear grows);
/// - pDirtDiffuseOverride = lerp(colour_a, colour_b, blend).
///
/// `wear` is clamp(counter / 2^32, 0, 1) x a scale from the component's definition (0x142dcf770), the counter a 32-bit
/// value kept XOR-encoded at owner +0x138 (FUN_142ddd720 re-keys it, taking the old masking key off and a new random one
/// on, so it keeps its value). A new ship has no wear because its state starts zeroed: the owner's constructor
/// (FUN_142db0aa0, lines 183-188) zeroes +0x160 .. +0x18b, the dirt range, the two colours and the blend among them (the
/// code that raises the wear later, and what fills the dirt values afterwards, were not traced: open, grade C). The check: the user's screenshots of their own ship show a clean, unworn hull.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShipWear {
    pub dirt_level_scale: f32,
    pub layer_mask_cutoff: f32,
    pub dirt_diffuse_override: Vec4,
}

impl ShipWear {
    /// The override object's values from its owner's state, by FUN_142de6090's rule.
    pub fn from_state(wear: f32, dirt_lo: f32, dirt_hi: f32, colours: [Vec4; 2], blend: f32) -> ShipWear {
        ShipWear {
            dirt_level_scale: (dirt_lo + (dirt_hi - dirt_lo) * blend + wear).clamp(0.0, 1.0) * 1.2 + 0.3,
            layer_mask_cutoff: 0.95 - wear * 0.9,
            dirt_diffuse_override: colours[0] + (colours[1] - colours[0]) * blend,
        }
    }

    /// The stock ship: no wear, the dirt range, colours and blend as the owner is made (see above).
    pub fn stock() -> ShipWear {
        ShipWear::from_state(0.0, 0.0, 0.0, [Vec4::ZERO; 2], 0.0)
    }
}

// ---- The canopy's damage and weather ----

/// The cockpit glass's effects object (`CockpitGlassEffects`, made by FUN_1434dcba0, owned by the cockpit's render object at
/// +0x90): FUN_1434dd710 writes its fields into six shader globals every frame. Made with the ice, condensation, crack and
/// tint 0 and the alpha multiplier 1.0. The crack amount (+0x48) is the cracks activity's level (`CockpitCracksActivity`,
/// FUN_1434dd8b0, written every frame it runs and kept after it stops: `parts::Cracks`, `Parts::crack`); a few of the
/// cockpit's activities write the ice and the condensation (calls of its slot 0x90 followed by a store to +0x40 / +0x44:
/// 0x1434d949b, 0x1434debdb, 0x1434dfcd0; not run); no store to the tint (+0x4c) or the alpha multiplier (+0x50) was found
/// among those callers. Its values reach the glass through `ShipGlass`, a buffer written in place (`draw.rs`), never a
/// material edit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassEffects {
    /// gCockpitGlassIceScale (+0x40).
    pub ice: f32,
    /// gCockpitGlassCondensationScale (+0x44).
    pub condensation: f32,
    /// gCockpitGlassCrackAmount (+0x48).
    pub crack: f32,
    /// gCockpitGlassTintAlpha (+0x4c).
    pub tint_alpha: f32,
    /// gCockpitGlassAlphaMultiplier (+0x50).
    pub alpha_multiplier: f32,
}

impl GlassEffects {
    /// As the object is made.
    pub fn stock() -> GlassEffects {
        GlassEffects { ice: 0.0, condensation: 0.0, crack: 0.0, tint_alpha: 0.0, alpha_multiplier: 1.0 }
    }

    /// gCockpitGlassState (+0x54), the canopy shader's variant (FUN_1434dd710): 0 unless the ice or the condensation is above
    /// f32::EPSILON (DAT_14506b228, 1.19e-7); then 1 when the ice is at least the condensation, else 2.
    pub fn state(&self) -> u32 {
        if self.ice > f32::EPSILON || self.condensation > f32::EPSILON {
            if self.condensation <= self.ice { 1 } else { 2 }
        } else {
            0
        }
    }

    /// The globals as the glass reads them (`sidewinder_glass.wesl` `ShipGlass`).
    pub fn gpu(&self) -> GlassEffectsGpu {
        GlassEffectsGpu { effects: Vec4::new(self.crack, self.ice, self.condensation, self.tint_alpha), more: Vec4::new(self.alpha_multiplier, self.state() as f32, 0.0, 0.0) }
    }
}

/// `sidewinder_glass.wesl` `ShipGlass`: x gCockpitGlassCrackAmount, y gCockpitGlassIceScale, z
/// gCockpitGlassCondensationScale, w gCockpitGlassTintAlpha; `more` x gCockpitGlassAlphaMultiplier, y gCockpitGlassState.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct GlassEffectsGpu {
    pub effects: Vec4,
    pub more: Vec4,
}

/// The buffer holding `GlassEffectsGpu`, bound to every `ShipMaterial` (only the canopy's family reads it), written in place
/// by `draw.rs` when the glass effects change.
#[derive(Resource)]
pub struct ShipGlass {
    pub buffer: Handle<ShaderBuffer>,
    written: GlassEffects,
}

impl ShipGlass {
    pub fn new(buffers: &mut Assets<ShaderBuffer>) -> ShipGlass {
        let stock = GlassEffects::stock();
        ShipGlass { buffer: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&stock.gpu()), RenderAssetUsages::default())), written: stock }
    }

    /// Writes `value` in place if it differs from what was last written.
    pub fn write(&mut self, buffers: &mut Assets<ShaderBuffer>, value: GlassEffects) {
        if value != self.written {
            write_in_place(buffers, &self.buffer, &value.gpu());
            self.written = value;
        }
    }
}

/// An attribute of `m` with exactly `n` values, as a `Vec4` (the rest 0).
fn attr4(m: &EliteMaterial, name: &str, n: usize) -> Result<Vec4, String> {
    let a = m.attrs.iter().find(|a| a.name == name).ok_or_else(|| format!("it has no {name}, which its family reads"))?;
    if a.values.len() != n {
        return Err(format!("{name} has {} values, not {n}", a.values.len()));
    }
    Ok(Vec4::from_array(std::array::from_fn(|i| a.values.get(i).copied().unwrap_or(0.0) as f32)))
}

/// What the cockpit's and the glass's families read beyond `ShipParams` (`sidewinder_surface_cockpit.wesl` `CockpitParams`,
/// field for field); zero for every other family.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct CockpitParams {
    pub override_colour: Vec4,
    pub detail: Vec4,
    pub glass_tile: Vec4,
    pub masks: [Vec4; 5],
    pub mask_scale_offset: Vec4,
    pub condensation: Vec4,
    pub occlusion: Vec4,
    pub cracks_colour: Vec4,
    pub ice_colour: Vec4,
    pub condensation_colour: Vec4,
    pub dirt: Vec4,
    pub roughness: Vec4,
    pub dirt_roughness: Vec4,
    pub smears: Vec4,
    pub tint: Vec4,
    pub tint_distant: Vec4,
    pub fresnel: Vec4,
}

impl CockpitParams {
    /// A material's: its family's attributes by name (the canopy's glass effects' globals change while it is drawn: they are
    /// `ShipGlass`'s). Refuses a material lacking an attribute its family reads.
    pub fn new(m: &EliteMaterial) -> Result<CockpitParams, String> {
        let get = |name: &str, n: usize| attr4(m, name, n);
        let one = |name: &str| Ok::<f32, String>(get(name, 1)?.x);
        let mut c = CockpitParams::default();
        match Family::of(m.shader.as_deref().unwrap_or("")) {
            Family::Cockpit | Family::CockpitEmissive => c.override_colour = get("pOverrideBaseColour", 4)?,
            Family::CockpitTile => {
                c.override_colour = get("pOverrideBaseColour", 4)?;
                let scale = get("pRoughnessDetailMapUVScale", 2)?;
                c.detail = Vec4::new(scale.x, scale.y, one("pRoughnessDetailMapStrength")?, one("pBaseColourDetailMapStrength")?);
            }
            Family::GlassCockpit => {
                let tile = get("pCockpitTile", 2)?;
                c.glass_tile = Vec4::new(tile.x, tile.y, one("pPSTB_GridTileRate")?, one("pCockpitRefractionAmount")?);
                for (slot, name) in c.masks.iter_mut().zip(["pCracksVertexMask", "pIceVertexMask", "pDirtVertexMask", "pImperfectionVertexMask", "pCondensationVertexMask"]) {
                    *slot = get(name, 4)?;
                }
                let (ice, imperfection) = (get("pIceMaskScaleOffset", 2)?, get("pImperfectionMaskScaleOffset", 2)?);
                c.mask_scale_offset = Vec4::new(ice.x, ice.y, imperfection.x, imperfection.y);
                let (mask, tile) = (get("pCondensationMaskScaleOffset", 2)?, get("pCondensationTile", 2)?);
                c.condensation = Vec4::new(mask.x, mask.y, tile.x, tile.y);
                c.occlusion = Vec4::new(one("pCracksOcclusionScale")?, one("pIceOcclusionScale")?, one("pCondensationOcclusionScale")?, one("pDirtAlphaMultiplier")?);
                c.cracks_colour = get("pCracksColour", 3)?.truncate().extend(one("pCracksMaskThreshold")?);
                c.ice_colour = get("pIceColour", 3)?.truncate().extend(one("pIceSceneColourDistortion")?);
                c.condensation_colour = get("pCondensationColour", 3)?.truncate().extend(one("pCondensationSceneColourDistortion")?);
                c.dirt = get("pDirtDiffuse", 3)?.truncate().extend(one("pLayerMaskCutoff")?);
                c.roughness = Vec4::new(one("pCracksRoughness")?, one("pIceRoughness")?, one("pCondensationRoughness")?, one("pImperfectionsRoughness")?);
                c.dirt_roughness = Vec4::new(one("pDirtRoughness")?, one("pGlassThickness")?, 0.0, 0.0);
                let (periods, scale_offset) = (get("pSmearsPeriods", 2)?, get("pSmearsScaleOffset", 2)?);
                c.smears = Vec4::new(periods.x, periods.y, scale_offset.x, scale_offset.y);
            }
            Family::GlassTinted => {
                c.tint = get("pGlassColourTint", 3)?.truncate().extend(one("pDistanceToOpaque")?);
                c.tint_distant = get("pGlassColourTint_Distant", 3)?.truncate().extend(one("pFadeRange")?);
                c.fresnel = get("pFresnel_ScalePowerBias", 3)?.truncate().extend(one("pFresnelVal")?);
            }
            _ => {}
        }
        Ok(c)
    }
}

// ---- The material ----

/// What the shader reads besides the light and the textures: every attribute the families read (`sidewinder_surface_hull.wesl`
/// `ShipParams`, field for field), one block all share.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct ShipParams {
    /// x 1 for the seat's view, y its near plane (`SEAT_NEAR_M`), z its share of the depth range (`SEAT_SLICE`), w
    /// pDeferredFlags.
    pub view: Vec4,
    /// x first, y count: the range of the ship's lights buffer that lights this material (A6; `lights::LightList::range`).
    pub lights: UVec4,
    /// The plain path: rgb the base colour when it is one colour, w 1 when it is the texture's.
    pub colour: Vec4,
    pub map_scales: Vec4,
    pub emissive: Vec4,
    pub uv_scale_offset: Vec4,
    pub rescale: Vec4,
    pub rescale_clamps: Vec4,
    pub dirt: Vec4,
    pub dirt_override: Vec4,
    pub dirt_gloss: Vec4,
    pub layer_top: Vec4,
    pub layer_bottom: Vec4,
    pub layer_mask: Vec4,
    /// The paint job's keys in the engine's order Background, Red, Green, Blue: rgb, roughness.
    pub keys: [Vec4; 4],
    pub key_metal: Vec4,
    pub key_emissive: Vec4,
    pub render_mode: Vec4,
    pub sharpening: Vec4,
    pub paint_tex: Vec4,
    pub stamp: Vec4,
    /// pDepthBias (`sidewinder_surface_hull.wesl` `ship_depth`).
    pub depth_bias: Vec4,
    /// x 1: the heat effect drives this material's pEmissiveColour and gStandardMapScales (`ShipGlow`).
    pub heat_glow: UVec4,
}

impl ShipParams {
    /// The same material's params for the seat's view (its depth slice).
    pub fn in_seat(self) -> ShipParams {
        ShipParams { view: Vec4::new(1.0, self.view.y, self.view.z, self.view.w), ..self }
    }

    /// Plain white, in the seat's slice or not (the base every material's params start from).
    pub fn white(seat_view: bool) -> ShipParams {
        ShipParams { view: Vec4::new(if seat_view { 1.0 } else { 0.0 }, SEAT_NEAR_M, SEAT_SLICE, 0.0), colour: Vec4::new(1.0, 1.0, 1.0, 0.0), ..default() }
    }

    /// A material's params: its family's attributes by name, the painted hull's paint constants from the paint record (and
    /// the mask's size, `mask_size`), the plain path's base colour. Refuses a material lacking an attribute its family reads,
    /// or a paint job using a path the mod doesn't carry (separate per-mask textures, iridescence, the Dynamic effect).
    pub fn new(m: &EliteMaterial, paint: &Paint, mask_size: UVec2, seat_view: bool, plain_textured: bool) -> Result<ShipParams, String> {
        let family = Family::of(m.shader.as_deref().unwrap_or(""));
        let get = |name: &str, n: usize| attr4(m, name, n);
        // (The canopy's attributes are all `CockpitParams`', some under the names the hull's tiles read here.)
        let read = |name: &str| family != Family::GlassCockpit && family.reads().contains(&name);
        let mut p = ShipParams::white(seat_view);
        if read("pDeferredFlags") {
            p.view.w = get("pDeferredFlags", 1)?.x;
        }
        if read("gStandardMapScales") {
            p.map_scales = get("gStandardMapScales", 3)?;
        }
        if read("pEmissiveColour") {
            p.emissive = get("pEmissiveColour", 3)?.truncate().extend(get("pEmissiveScale", 1)?.x);
            p.uv_scale_offset = get("pUVScaleOffset", 4)?;
        }
        if read("pEmissiveRescalingEnable") {
            p.rescale = Vec4::new(get("pEmissiveRescalingEnable", 1)?.x, get("pEmissiveRescalingType", 1)?.x, get("pEmissiveRescalingEVOffset", 1)?.x, get("pEmissiveRescalingEnableClamping", 1)?.x);
            p.rescale_clamps = get("pEmissiveRescalingEVClamps", 2)?;
        }
        if read("pDirtDiffuse") {
            p.dirt = get("pDirtDiffuse", 3)?.truncate().extend(get("pDirtLevelScale", 1)?.x);
            p.dirt_override = get("pDirtDiffuseOverride", 4)?;
            let (scales, tweak) = (get("pDirtMapScales", 2)?, get("pDiffuseGlossTweak_UV2", 2)?);
            p.dirt_gloss = Vec4::new(scales.x, scales.y, tweak.x, tweak.y);
            p.layer_bottom = get("pLayerBottom_BaseColourOverride", 4)?;
            p.layer_mask.w = get("pTile1Metalness", 1)?.x;
        }
        if read("pLayerMaskCutoff") {
            p.layer_top = get("pLayerTop_BaseColourOverride", 4)?;
            p.layer_mask = Vec4::new(get("pLayerMaskCutoff", 1)?.x, get("pLayerMaskUV1Scale", 1)?.x, get("pLayerMaskSoftness", 1)?.x, p.layer_mask.w);
        }
        if matches!(family, Family::Painted | Family::SingleTile) {
            // (The ship's wear and dirt override replaces the material's own values, as in the running game.)
            let wear = ShipWear::stock();
            p.dirt.w = wear.dirt_level_scale;
            p.dirt_override = wear.dirt_diffuse_override;
            if family == Family::Painted {
                p.layer_mask.x = wear.layer_mask_cutoff;
            }
        }
        if read("pDepthBias") {
            p.depth_bias = get("pDepthBias", 4)?;
        }
        if read("pNormalDetailMapUVScale") {
            let scale = get("pNormalDetailMapUVScale", 2)?;
            p.stamp = Vec4::new(scale.x, scale.y, get("pNormalDetailMapStrength", 1)?.x, 0.0);
        }
        if family == Family::Painted {
            let block = &paint.texture_block_a;
            if block.separate_texture_per_mask != 0 || paint.use_iridescence != 0 || paint.extended_effect {
                return Err("its paint job uses separate per-mask textures, iridescence or the Dynamic effect, whose variants aren't built".into());
            }
            let key = |name: &str| paint.keys.iter().find(|k| k.name == name).ok_or_else(|| format!("the paint job has no key {name}"));
            let order = ["Background", "RedKey", "GreenKey", "BlueKey"].map(key);
            let mut keys = [Vec4::ZERO; 4];
            for (i, k) in order.iter().enumerate() {
                let k = k.as_ref().map_err(|e| e.clone())?;
                keys[i] = Vec4::new(k.colour[0] as f32, k.colour[1] as f32, k.colour[2] as f32, k.roughness as f32);
                p.key_metal[i] = k.metalness as f32;
                p.key_emissive[i] = k.emissive_scale as f32;
            }
            p.keys = keys;
            let (modes, sharpen) = paint_modes(paint);
            p.render_mode = Vec4::from_array(modes);
            p.sharpening = Vec4::new(sharpen[0], sharpen[1], sharpen[2], 0.0);
            // (ColourTextureDimensionsRcp: 1 / the mask's size, as the exe writes it.)
            p.paint_tex = Vec4::new(1.0 / mask_size.x.max(1) as f32, 1.0 / mask_size.y.max(1) as f32, block.use_3x3_filter as f32, 0.0);
        }
        if family == Family::Plain {
            p.colour = match base_colour(m) {
                Some(Base::Colour(c)) => Vec4::new(c[0], c[1], c[2], 0.0),
                Some(Base::Texture { .. }) => Vec4::new(1.0, 1.0, 1.0, if plain_textured { 1.0 } else { 0.0 }),
                // (None of the slots: plain white, so it shows rather than vanishes.)
                None => Vec4::new(1.0, 1.0, 1.0, 0.0),
            };
        }
        Ok(p)
    }
}

/// An attribute of `m` its family neither reads nor lists as ignored (none should be: the test `no_attribute_dropped`).
pub fn unaccounted_attributes(m: &EliteMaterial) -> Vec<String> {
    let family = Family::of(m.shader.as_deref().unwrap_or(""));
    if matches!(family, Family::Plain | Family::Decal) {
        return Vec::new();
    }
    m.attrs.iter().map(|a| a.name.clone()).filter(|n| !family.reads().contains(&n.as_str()) && !family.ignores().iter().any(|(start, _)| n.starts_with(start))).collect()
}

/// The seat's view (ARCHITECTURE.md A6): the cockpit and its chair are drawn in the nearest slice of the depth range, as the
/// sandbox's weapon view models are (`sandbox_viewmodel.wesl`), so the port's near plane, which grows with the height and at
/// the switch to scaled space, never cuts them and they always hide the world behind them. Their x, y and w are the
/// world's (the same field of view); their depth comes from a near plane of their own (`SEAT_NEAR_M`), squeezed into this
/// share of the range nearest the eye (reverse z: 1 is the near plane), so their order among themselves is kept.
pub const SEAT_SLICE: f32 = 0.1;
/// The seat view's own near plane (m): nearer than any of the cockpit's or the chair's points from the eye (the nearest is
/// the chair's headrest, 0.151 m away), far enough that their depths keep a fine resolution over the cockpit's 12 m.
pub const SEAT_NEAR_M: f32 = 0.1;
/// The canopy from the seat lies in front of everything else the view draws, so among the see-through draws (sorted by
/// distance plus this bias, back to front) it comes last: after the world's own, up to the aurora's 3.6e15
/// (`aurora.rs`; render distances stay below ~3e8). The seat's other see-through draws (the chair's decals) lie behind it.
const SEAT_GLASS_BIAS: f32 = 4.0e15;

/// The ship's material: the sandbox's two light buffers, its family's textures (unused ones white) with one repeating,
/// trilinear, anisotropic sampler, `ShipParams`, the ship's lights buffer and pose, the IES profile, `CockpitParams`, the
/// scene's exposure inputs (`ShipScene`), the heat effect's glow (`ShipGlow`), the automatic exposure's adapted compensation
/// (`ShipScene::adapt`), the canopy's glass effects (`ShipGlass`), the pilot chair's bone palette (`ShipPalette`: bound to
/// every material, read only by a skinned one). Made once.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(ShipKey)]
pub struct ShipMaterial {
    #[storage(0, read_only)]
    pub frame: Handle<ShaderBuffer>,
    #[storage(1, read_only)]
    pub light: Handle<ShaderBuffer>,
    #[texture(2)]
    #[sampler(3)]
    pub tex0: Handle<Image>,
    #[texture(4)]
    pub tex1: Handle<Image>,
    #[texture(5)]
    pub tex2: Handle<Image>,
    #[texture(6)]
    pub tex3: Handle<Image>,
    #[texture(7)]
    pub tex4: Handle<Image>,
    #[texture(8)]
    pub tex5: Handle<Image>,
    #[uniform(9)]
    pub params: ShipParams,
    #[storage(10, read_only)]
    pub lights: Handle<ShaderBuffer>,
    #[storage(11, read_only)]
    pub pose: Handle<ShaderBuffer>,
    #[texture(12)]
    #[sampler(13)]
    pub ies: Handle<Image>,
    #[uniform(14)]
    pub cockpit: CockpitParams,
    #[storage(15, read_only)]
    pub scene: Handle<ShaderBuffer>,
    #[storage(16, read_only)]
    pub glow: Handle<ShaderBuffer>,
    #[storage(17, read_only)]
    pub adapt: Handle<ShaderBuffer>,
    #[storage(18, read_only)]
    pub glass: Handle<ShaderBuffer>,
    #[storage(19, read_only)]
    pub palette: Handle<ShaderBuffer>,
    pub family: Family,
    /// Its meshes carry the vertex's bone (`ATTRIBUTE_BONE`) and its vertex stage skins them by the palette (the shader def
    /// `SHIP_SKINNED`). Never a glass family's (the skinned chair has no glass: `assets.rs` refuses it).
    pub skinned: bool,
}

/// A pipeline per family, and per skinning: each compiles only its own surface code, and only a skinned one the palette.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShipKey {
    family: Family,
    skinned: bool,
}

impl From<&ShipMaterial> for ShipKey {
    fn from(m: &ShipMaterial) -> Self {
        ShipKey { family: m.family, skinned: m.skinned }
    }
}

impl ShipMaterial {
    /// A material of family `family` with its slots' images `textures` (in `Family::slots`' order; `white` for the rest),
    /// `params` and `cockpit`; unskinned (`skinned` set after for a skinned model's draw).
    #[allow(clippy::too_many_arguments)]
    pub fn new(light: &PropLight, lights: &ShipLights, scene: &ShipScene, glow: &ShipGlow, glass: &ShipGlass, palette: &ShipPalette, family: Family, textures: &[Handle<Image>], white: &Handle<Image>, ies: &Handle<Image>, params: ShipParams, cockpit: CockpitParams) -> ShipMaterial {
        let tex = |i: usize| textures.get(i).cloned().unwrap_or_else(|| white.clone());
        ShipMaterial {
            frame: light.frame.clone(),
            light: light.light.clone(),
            tex0: tex(0),
            tex1: tex(1),
            tex2: tex(2),
            tex3: tex(3),
            tex4: tex(4),
            tex5: tex(5),
            params,
            lights: lights.buffer.clone(),
            pose: lights.pose.clone(),
            ies: ies.clone(),
            cockpit,
            scene: scene.buffer.clone(),
            glow: glow.buffer.clone(),
            adapt: scene.adapt.clone(),
            glass: glass.buffer.clone(),
            palette: palette.buffer.clone(),
            family,
            skinned: false,
        }
    }
}

/// The sampler every texture of the ship takes (as `mr_assets::dds_image` gives the files): repeating, trilinear, 16x
/// anisotropic (Elite's QualitySettingSampler).
fn ship_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    })
}

/// A 1x1 texture of a constant colour (`TexSource::Constant`: linear), with the ship's sampler.
pub fn constant(images: &mut Assets<Image>, rgba: [u8; 4]) -> Handle<Image> {
    let mut image = Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, rgba.to_vec(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.sampler = ship_sampler();
    images.add(image)
}

/// The IES profile's sampler: linear, clamped (Elite's BilinearSampler; its address mode was not found, and inside the
/// cone the lookup never leaves the profile), level 0 only (the shader asks for it).
pub fn ies_sampler(image: &mut Image) {
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
}

impl Material for ShipMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sidewinder_ship.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sidewinder_ship.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        match self.family {
            Family::Overlay => AlphaMode::Blend,
            Family::Stamp => AlphaMode::Multiply,
            // (Premultiplied over: `sidewinder_glass.wesl`'s header. Drawn after the opaque, not writing depth.)
            f if f.is_glass() => AlphaMode::Premultiplied,
            _ => AlphaMode::Opaque,
        }
    }

    fn depth_bias(&self) -> f32 {
        if self.family.is_glass() && self.params.view.x > 0.5 { SEAT_GLASS_BIAS } else { 0.0 }
    }

    fn specialize(_pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let family = key.bind_group_data.family;
        let mut defs = vec![family.def()];
        let vertex_layout = if family.is_glass() {
            // (The glass's own entry points: the module has two of each, so they are named.)
            defs.push("SHIP_GLASS");
            descriptor.vertex.entry_point = Some("vertex_glass".into());
            if let Some(f) = descriptor.fragment.as_mut() {
                f.entry_point = Some("fragment_glass".into());
            }
            layout.0.get_layout(&[
                Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
                Mesh::ATTRIBUTE_TANGENT.at_shader_location(2),
                Mesh::ATTRIBUTE_UV_0.at_shader_location(3),
                ATTRIBUTE_COLOUR.at_shader_location(6),
            ])?
        } else {
            let mut attributes = vec![
                Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
                Mesh::ATTRIBUTE_TANGENT.at_shader_location(2),
                Mesh::ATTRIBUTE_UV_0.at_shader_location(3),
                Mesh::ATTRIBUTE_UV_1.at_shader_location(4),
                ATTRIBUTE_UV_2.at_shader_location(5),
            ];
            if key.bind_group_data.skinned {
                defs.push("SHIP_SKINNED");
                attributes.push(ATTRIBUTE_BONE.at_shader_location(7));
            }
            layout.0.get_layout(&attributes)?
        };
        descriptor.vertex.buffers = vec![vertex_layout];
        for def in defs {
            descriptor.vertex.shader_defs.push(def.into());
            if let Some(f) = descriptor.fragment.as_mut() {
                f.shader_defs.push(def.into());
            }
        }
        Ok(())
    }
}

/// A 1x1 white texture for the slots a family doesn't use.
pub fn white(images: &mut Assets<Image>) -> Handle<Image> {
    constant(images, [255; 4])
}

// ---- The ship's lights buffer (ARCHITECTURE.md A6's contract) ----

/// The most lights the buffer holds.
pub const MAX_SHIP_LIGHTS: usize = 96;

/// One light, in the ship's space, metres, linear colour (A6): `pos_range` xyz position, w range; `colour` rgb colour x level
/// (the exposure factor is the shader's: `ShipScene`'s lamp terms, so it follows the exposure the frame is drawn with), w
/// source size; `dir_cos` xyz the spot's axis, w the cosine of its half cone (-2: a point light);
/// `spot` x the cosine of the inner half angle, y K = pi / cone for the IES lookup, z 1 if the IES profile applies, w the
/// specular scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct ShipLight {
    pub pos_range: Vec4,
    pub colour: Vec4,
    pub dir_cos: Vec4,
    pub spot: Vec4,
}

/// `ShipLights { count: u32, _pad: vec3<u32>, lights: array<ShipLight, 96> }` (the WGSL layout puts the vec3 at byte 16, so
/// the lights start at byte 32; encase lays it out the same).
#[derive(ShaderType)]
struct ShipLightsGpu {
    count: u32,
    _pad: UVec3,
    lights: [ShipLight; MAX_SHIP_LIGHTS],
}

/// The ship's space from the render frame's (`sidewinder_lit.wesl` `ShipPose`): three rows of the affine inverse.
#[derive(ShaderType, Default)]
struct ShipPoseGpu {
    rows: [Vec4; 3],
}

/// The ship's lights buffer and pose, bound to every `ShipMaterial`, written in place (never a material edit): `lights.rs`
/// fills the list with `write`; `draw` writes the pose each frame while any light is on.
#[derive(Resource)]
pub struct ShipLights {
    pub buffer: Handle<ShaderBuffer>,
    pub pose: Handle<ShaderBuffer>,
    on: bool,
}

impl ShipLights {
    pub fn new(buffers: &mut Assets<ShaderBuffer>) -> ShipLights {
        let empty = ShipLightsGpu { count: 0, _pad: UVec3::ZERO, lights: [ShipLight::default(); MAX_SHIP_LIGHTS] };
        let buffer = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&empty), RenderAssetUsages::default()));
        let pose = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&ShipPoseGpu::default()), RenderAssetUsages::default()));
        ShipLights { buffer, pose, on: false }
    }

    /// Is any light of the list on (a colour above zero)?
    pub fn any_on(&self) -> bool {
        self.on
    }

    /// Writes the list in place (at most `MAX_SHIP_LIGHTS`; the count is the list's length).
    pub fn write(&mut self, buffers: &mut Assets<ShaderBuffer>, lights: &[ShipLight]) {
        let mut gpu = ShipLightsGpu { count: lights.len().min(MAX_SHIP_LIGHTS) as u32, _pad: UVec3::ZERO, lights: [ShipLight::default(); MAX_SHIP_LIGHTS] };
        for (slot, l) in gpu.lights.iter_mut().zip(lights) {
            *slot = *l;
        }
        self.on = lights.iter().any(|l| l.colour.truncate() != Vec3::ZERO);
        write_in_place(buffers, &self.buffer, &gpu);
    }

    /// The ship's pose this frame (`ship`: the ship's space to the render frame, its scale the scaled space's), for the lights'
    /// loop. Called only while a light is on.
    pub fn write_pose(&self, buffers: &mut Assets<ShaderBuffer>, ship: &Transform) {
        let inv = Mat4::from_scale_rotation_translation(ship.scale, ship.rotation, ship.translation).inverse();
        write_in_place(buffers, &self.pose, &ShipPoseGpu { rows: [inv.row(0), inv.row(1), inv.row(2)] });
    }
}

// ---- Elite's scene exposure inputs (ARCHITECTURE.md A6's known gap) ----

/// Elite's scene exposure inputs (`sidewinder_lit.wesl` `ShipScene`, its first four fields): `lights.rs` works them out from
/// the camera's exposure (`ColorGrading`, the compensation's stops) each frame (its doc gives the rule and the one measured
/// constant) and writes them in place. In photo mode 2 the automatic exposure darkens the frame further by the adapted
/// compensation, which lives on the GPU only (`exposure.rs`'s state buffer): the shader adds it (`ShipScene::adapt`), so
/// these four are the values without it.
/// - `scene_ev`: the lighting manager's scene value, which Elite feeds into each lamp's exposure factor,
///   2^(clamp(value + bias, min, max) - 3) (FUN_1435a5500 on the CPU; here the shader, from each slot's terms).
/// - `self_illum_scale`: gGlobalSelfIllumScale (cb1[1].w), which the emissive families multiply their glow by.
/// - `emissive_multiplier`: gEmissiveMultiplier (cb1[61].z), the emissive rescaling's scene value for type 0.
/// - `luminance`: the texel of gAverageSceneLuminanceTexture that the rescaling reads, x 0.125, for types 1 and 2.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct SceneExposure {
    pub scene_ev: f32,
    pub self_illum_scale: f32,
    pub emissive_multiplier: f32,
    pub luminance: f32,
}

/// Writes `value`'s GPU layout into `buffer`'s bytes in place (the buffers below: never a material edit).
fn write_in_place<T: ShaderType + encase::internal::WriteInto>(buffers: &mut Assets<ShaderBuffer>, buffer: &Handle<ShaderBuffer>, value: &T) {
    if let Some(mut b) = buffers.get_mut(buffer) {
        if let Some(d) = b.cast_slice_mut::<u8>() {
            let _ = encase::StorageBuffer::new(d).write(value);
        }
    }
}

/// `sidewinder_lit.wesl` `ShipScene`: the inputs, x 1 when the automatic exposure's compensation applies (photo mode 2),
/// and each slot's exposure term (`lights::Lamp::exposure_terms`: x 1 with the exposure term on, y its bias, z, w its clamp).
#[derive(ShaderType)]
struct ShipSceneGpu {
    inputs: SceneExposure,
    adapt_on: Vec4,
    lamps: [Vec4; MAX_SHIP_LIGHTS],
}

/// The buffer holding `ShipSceneGpu`, bound to every `ShipMaterial`, written in place by `lights.rs`; and `adapt`, a copy of
/// the automatic exposure's state (`exposure::AutoExposureBuffers::state`: x the adapted compensation in stops, positive
/// darker), which only the GPU writes (`super::exposure`, before the opaque pass). Extracted for that copy.
#[derive(Resource, Clone, bevy::render::extract_resource::ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct ShipScene {
    pub buffer: Handle<ShaderBuffer>,
    pub adapt: Handle<ShaderBuffer>,
}

impl ShipScene {
    pub fn new(buffers: &mut Assets<ShaderBuffer>, first: &SceneExposure) -> ShipScene {
        let gpu = ShipSceneGpu { inputs: *first, adapt_on: Vec4::ZERO, lamps: [Vec4::ZERO; MAX_SHIP_LIGHTS] };
        ShipScene {
            buffer: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&gpu), RenderAssetUsages::default())),
            adapt: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&Vec4::ZERO), RenderAssetUsages::RENDER_WORLD)),
        }
    }

    /// The inputs, whether the adapted compensation applies (`auto`) and the slots' exposure terms (at most
    /// `MAX_SHIP_LIGHTS`, in the lights buffer's order).
    pub fn write(&self, buffers: &mut Assets<ShaderBuffer>, value: &SceneExposure, auto: bool, lamps: &[Vec4]) {
        let mut gpu = ShipSceneGpu { inputs: *value, adapt_on: Vec4::new(auto as u32 as f32, 0.0, 0.0, 0.0), lamps: [Vec4::ZERO; MAX_SHIP_LIGHTS] };
        for (slot, t) in gpu.lamps.iter_mut().zip(lamps) {
            *slot = *t;
        }
        write_in_place(buffers, &self.buffer, &gpu);
    }
}

// ---- The pilot chair's bone palette ----

/// The palette's length: the bones of the seat's chair (`models/cockpit_chair.edm`, 142; `sidewinder_ship.wesl` `palette`
/// has the same fixed length, so the binding's size is known from the start and the same for every material). A test checks
/// the shipped chair against it.
pub const PALETTE_BONES: usize = 142;

/// `sidewinder_ship.wesl` `palette`: a matrix per bone, from the bind pose to the posed one, both in the model's space in
/// the mod's axes (the vertices' as `assets.rs` decodes them).
#[derive(ShaderType)]
struct ShipPaletteGpu {
    bones: [Mat4; PALETTE_BONES],
}

/// The buffer holding the pilot chair's palette (`ShipPaletteGpu`), bound to every `ShipMaterial` (only a skinned one reads
/// it), identities until the first pose; `draw.rs` writes it in place from `chair::ChairPose` when the pose changes.
#[derive(Resource)]
pub struct ShipPalette {
    pub buffer: Handle<ShaderBuffer>,
    /// The pose's revision last written (`ChairPose::revision`).
    written: u64,
}

impl ShipPalette {
    pub fn new(buffers: &mut Assets<ShaderBuffer>) -> ShipPalette {
        let rest = ShipPaletteGpu { bones: [Mat4::IDENTITY; PALETTE_BONES] };
        ShipPalette { buffer: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&rest), RenderAssetUsages::default())), written: 0 }
    }

    /// Writes the palette in place if the pose's `revision` is not the one last written: `bones` (the mod's axes,
    /// `assets::ShipSpace::skin`), identities for the bones it lacks (none without a ship: the bind pose).
    pub fn write(&mut self, buffers: &mut Assets<ShaderBuffer>, revision: u64, bones: impl Iterator<Item = Mat4>) {
        if revision == self.written {
            return;
        }
        let mut gpu = ShipPaletteGpu { bones: [Mat4::IDENTITY; PALETTE_BONES] };
        for (slot, m) in gpu.bones.iter_mut().zip(bones) {
            *slot = m;
        }
        write_in_place(buffers, &self.buffer, &gpu);
        self.written = revision;
    }
}

// ---- The heat effect's glow ----

/// The values Elite's heat effect (`ShipHeatEffectActivity`, re/elite/work/csm/vfx/notes.md) writes over its material
/// element's attributes every frame (`sidewinder_lit.wesl` `ShipGlow`): rgb pEmissiveColour and gStandardMapScales, each
/// lerp(cold, hot, glow). Read only by a material whose `ShipParams::heat_glow` is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct HeatGlow {
    pub emissive: Vec4,
    pub map_scales: Vec4,
}

/// The buffer holding `HeatGlow`, bound to every `ShipMaterial`, written in place by `lights.rs`.
#[derive(Resource)]
pub struct ShipGlow {
    pub buffer: Handle<ShaderBuffer>,
}

impl ShipGlow {
    pub fn new(buffers: &mut Assets<ShaderBuffer>) -> ShipGlow {
        ShipGlow { buffer: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&HeatGlow::default()), RenderAssetUsages::default())) }
    }

    pub fn write(&self, buffers: &mut Assets<ShaderBuffer>, value: &HeatGlow) {
        write_in_place(buffers, &self.buffer, value);
    }
}

pub fn build(app: &mut App) {
    app.add_plugins(MaterialPlugin::<ShipMaterial>::default());
    super::exposure::build(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::elite_ship::{parse_materials, parse_ship};

    fn shipped() -> (se_core::elite_ship::Materials, Paint) {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).unwrap();
        (parse_materials(&read("materials.json")).unwrap(), parse_ship(&read("ship.json")).unwrap().paint)
    }

    /// The tables keep what this task doesn't draw out, and the families it does in.
    #[test]
    fn drawn_now() {
        let (_, paint) = shipped();
        let m = |shader: &str| EliteMaterial { shader: Some(shader.into()), textures: Vec::new(), attrs: Vec::new(), fallback_to: None, why: None };
        for s in [
            "Metalness_Roughness_OpaqueDeferred",
            "Metalness_Roughness_OpaqueDeferred_SpacecraftTilePainted_Normals",
            "Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_Emissive",
            "Metalness_Roughness_AlphaDeferred_Overlay",
            "DetailStamp_NormalAO",
        ] {
            assert!(material_drawn("x", &m(s), &paint), "{s}");
        }
        for s in ["CockpitGlass", "GlassSimple_Tintable_Roughness_Alpha", "GlassSimple_Roughness_Alpha_SingleSource"] {
            assert!(material_drawn("x", &m(s), &paint), "{s}");
            assert!(Family::of(s).is_glass(), "{s}");
        }
        for s in ["ShieldAlpha_X", "Schematic_X"] {
            assert!(!material_drawn("x", &m(s), &paint), "{s}");
        }
        assert!(material_drawn("cockpit_chair_decals", &m("Metalness_Roughness_AlphaDeferred_Overlay"), &paint));
        let fallback = EliteMaterial { shader: None, textures: Vec::new(), attrs: Vec::new(), fallback_to: Some("null".into()), why: None };
        assert!(!material_drawn("x", &fallback, &paint));
        assert!(model_drawn("sidewinder") && model_drawn("landinggearfront") && model_drawn("cockpit_chair") && model_drawn("sidewinder_screen"));
        assert!(model_drawn("sidewinder_screen_damagestate_02"), "the breached windscreen (its two glass families are built)");
        assert!(!model_drawn("shard014") && !model_drawn("cockpit_ui_hightlight_radar"));
        // Every material the cockpit and the glass use has a family of its own now: none is drawn plain.
        let (materials, _) = shipped();
        for (name, mat) in &materials.materials {
            let shader = mat.shader.as_deref().unwrap_or("");
            if shader.contains("Cockpit") || shader.contains("Glass") {
                assert_ne!(Family::of(shader), Family::Plain, "{name}: {shader}");
            }
        }
    }

    /// The cockpit colour override leaves every cockpit material's colour as it is (`sidewinder_surface_cockpit.wesl`'s
    /// header): no modulation slot (-1, stored as u32) and pOverrideBaseColour's weight 0.
    #[test]
    fn cockpit_override_is_identity() {
        let (materials, _) = shipped();
        let mut n = 0;
        for (name, m) in &materials.materials {
            if !matches!(Family::of(m.shader.as_deref().unwrap_or("")), Family::Cockpit | Family::CockpitEmissive | Family::CockpitTile) {
                continue;
            }
            assert_eq!(attr4(m, "pMaterialModulationSlot", 1).unwrap().x, u32::MAX as f32, "{name}");
            let c = CockpitParams::new(m).unwrap();
            assert_eq!(c.override_colour.w, 0.0, "{name}: {:?}", c.override_colour);
            n += 1;
        }
        assert_eq!(n, 21, "the 13 tiles, the chair's 4 and the bespoke parts, the 3 emissive");
    }

    /// The stock canopy shows no damage or weather (`sidewinder_glass.wesl`'s header): the crack amount is 0, so every
    /// vertex's crack key, (colour . mask + 1/255) x 0, is under pCracksMaskThreshold; neither ice nor condensation, so the
    /// state is 0. And the state's rule, FUN_1434dd710's.
    #[test]
    fn stock_glass_shows_no_damage() {
        let (materials, _) = shipped();
        let glass = materials.get("cockpit_glass_int").unwrap();
        let stock = GlassEffects::stock();
        let c = CockpitParams::new(glass).unwrap();
        assert_eq!(stock.gpu().effects, Vec4::new(0.0, 0.0, 0.0, 0.0), "crack, ice, condensation, tint");
        assert_eq!(stock.gpu().more, Vec4::new(1.0, 0.0, 0.0, 0.0), "the alpha multiplier, the state");
        assert!(c.cracks_colour.w > 0.0, "the threshold {} is above any crack key", c.cracks_colour.w);
        assert_eq!(stock.state(), 0);
        let with = |ice: f32, condensation: f32| GlassEffects { ice, condensation, ..stock }.state();
        assert_eq!((with(0.5, 0.0), with(0.5, 0.5), with(0.2, 0.5), with(1.0e-8, 0.0)), (1, 1, 2, 0));
    }

    /// The outside glass's two tints are grey (each one's channels equal). That is all this proves: the transmission is
    /// grey only as far as the glass's colour texture is too, whose channels differ by up to 3 of 255 (measured;
    /// `sidewinder_glass.wesl`'s header), so the blend's coverage carries the transmission to within that.
    #[test]
    fn outside_glass_tints_are_grey() {
        let (materials, _) = shipped();
        let c = CockpitParams::new(materials.get("shipexternalglassmaterial").unwrap()).unwrap();
        for tint in [c.tint, c.tint_distant] {
            assert!(tint.x == tint.y && tint.y == tint.z, "{tint:?}");
        }
        assert_eq!((c.tint.w, c.tint_distant.w), (256.0, 200.0), "pDistanceToOpaque, pFadeRange");
    }

    /// The paint rule: red first, then green limited to what red leaves, then blue, the background the remainder; each
    /// channel cut at 0.5 in render mode 1.
    #[test]
    fn paint_rule() {
        let hard = [1.0, 1.0, 1.0, 0.0];
        let none = [0.0; 3];
        assert_eq!(paint_weights([0.0, 0.0, 0.0, 1.0], hard, none), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(paint_weights([0.6, 0.0, 0.0, 1.0], hard, none), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(paint_weights([0.49, 0.0, 0.0, 1.0], hard, none), [0.0, 0.0, 0.0, 1.0], "0.49 is under the threshold");
        assert_eq!(paint_weights([0.5, 0.0, 0.0, 1.0], hard, none), [1.0, 0.0, 0.0, 0.0], "0.5 is at it: in");
        assert_eq!(paint_weights([0.9, 0.9, 0.9, 1.0], hard, none), [1.0, 0.0, 0.0, 0.0], "red takes it all");
        assert_eq!(paint_weights([0.1, 0.9, 0.9, 1.0], hard, none), [0.0, 1.0, 0.0, 0.0], "then green");
        assert_eq!(paint_weights([0.1, 0.2, 0.7, 1.0], hard, none), [0.0, 0.0, 1.0, 0.0], "then blue");
        // Mode 0 (the value as it is): shares in order.
        let soft = [0.0, 0.0, 0.0, 0.0];
        let w = paint_weights([0.25, 0.5, 0.5, 1.0], soft, none);
        assert!((w[0] - 0.25).abs() < 1e-6 && (w[1] - 0.5).abs() < 1e-6 && (w[2] - 0.25).abs() < 1e-6 && w[3].abs() < 1e-6, "{w:?}");
        // Mode 2 forces a channel to 0; no channel at or under 1: all background.
        assert_eq!(paint_weights([0.9, 0.9, 0.0, 1.0], [2.0, 1.0, 1.0, 0.0], none), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(paint_weights([0.9, 0.9, 0.9, 1.0], [2.0, 2.0, 2.0, 2.0], none), [0.0, 0.0, 0.0, 1.0]);
    }

    /// The default paint job gives the render modes (1, 1, 1, 0), no sharpening, one tap; its keys reach the params in the
    /// engine's order.
    #[test]
    fn default_paint_job() {
        let (materials, paint) = shipped();
        assert_eq!(paint_modes(&paint), ([1.0, 1.0, 1.0, 0.0], [0.0; 3]));
        assert_eq!(paint.texture_block_a.use_3x3_filter, 0);
        let hull = materials.get("sidewinderhull_paintedmetal_dmg").unwrap();
        let p = ShipParams::new(hull, &paint, UVec2::new(256, 256), false, false).unwrap();
        assert!((p.keys[0].x - 0.515723).abs() < 1e-5 && (p.keys[0].w - 0.4).abs() < 1e-6, "Background first: {:?}", p.keys[0]);
        assert!((p.keys[1].x - 0.279061).abs() < 1e-5 && (p.keys[1].w - 0.1).abs() < 1e-6, "then Red: {:?}", p.keys[1]);
        assert_eq!(p.key_metal, Vec4::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(p.paint_tex, Vec4::new(1.0 / 256.0, 1.0 / 256.0, 0.0, 0.0));
    }

    /// Every drawn material of a built family has its family's slots and attributes, and every attribute it holds is read
    /// or listed as ignored.
    #[test]
    fn no_attribute_dropped() {
        let (materials, paint) = shipped();
        for (name, m) in &materials.materials {
            if !material_drawn(name, m, &paint) {
                continue;
            }
            sources(m, &paint).unwrap_or_else(|e| panic!("{name}: {e}"));
            ShipParams::new(m, &paint, UVec2::new(256, 256), false, true).unwrap_or_else(|e| panic!("{name}: {e}"));
            CockpitParams::new(m).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(unaccounted_attributes(m).is_empty(), "{name}: {:?}", unaccounted_attributes(m));
        }
    }

    /// The stock ship's wear override by FUN_142de6090's rule: dirt 0.3, the cutoff 0.95, no override colour; and the rule's
    /// ends (full wear: dirt 1.5, cutoff 0.05).
    #[test]
    fn wear_override() {
        let w = ShipWear::stock();
        assert!((w.dirt_level_scale - 0.3).abs() < 1e-6 && (w.layer_mask_cutoff - 0.95).abs() < 1e-6 && w.dirt_diffuse_override == Vec4::ZERO, "{w:?}");
        let worn = ShipWear::from_state(1.0, 0.0, 0.0, [Vec4::ZERO; 2], 0.0);
        assert!((worn.dirt_level_scale - 1.5).abs() < 1e-6 && (worn.layer_mask_cutoff - 0.05).abs() < 1e-6, "{worn:?}");
        let (materials, paint) = shipped();
        let p = ShipParams::new(materials.get("sidewinderhull_paintedmetal_dmg").unwrap(), &paint, UVec2::new(256, 256), false, false).unwrap();
        assert!((p.dirt.w - 0.3).abs() < 1e-6 && (p.layer_mask.x - 0.95).abs() < 1e-6, "{p:?}");
    }

    /// With no decal fitted the three customisation decal slots' shader covers nothing (pDecalNAlpha0 0; the paint job's
    /// fourth group, which the empty mask's remainder picks, has alpha 0), so none is drawn.
    #[test]
    fn empty_decal_slots_show_nothing() {
        let (materials, paint) = shipped();
        let mut slots = 0;
        for (name, m) in &materials.materials {
            if Family::of(m.shader.as_deref().unwrap_or("")) == Family::Decal {
                assert_eq!(decal_slot_coverage(m, &paint), Some(0.0), "{name}");
                assert!(!material_drawn(name, m, &paint), "{name}");
                slots += 1;
            }
        }
        assert_eq!(slots, 3);
    }
}
