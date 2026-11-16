//! The ship's material (ARCHITECTURE.md A6): `ShipMaterial` with `sidewinder_ship.wesl`, lit by the sandbox's light record as
//! the props are (`crate::sandbox::light::PropLight`, the same two buffers `PropMaterial` binds), and what of Elite's
//! materials it draws yet.
//!
//! The shader's fragment stage is two functions: a surface (base colour, normal, roughness, metalness, emissive, ambient
//! occlusion) and its lighting. Today only the base colour is real (`Base`): the material's `pBaseColour_Roughness`, the tile
//! families' first base colour, the painted hull's Background key from the paint job; the lighting is `sandbox_prop.wesl`'s.
//! Elite's own material maths (`decoded/shaders/`) and lit maths fill those two functions per shader family in T4b.
//!
//! Made once per Elite material at the first spawn, twice for a material both the outside and the seat's view use (the
//! seat's draws in its own depth slice: `SEAT_SLICE`); never changed after (per-frame values live in the light buffers).

use bevy::asset::RenderAssetUsages;
use bevy::math::Vec4;
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

/// Elite's shader families not drawn yet, by a piece of the shader's name, and why: each comes with its own shader in a later
/// task (glass needs a see-through pass, decals and overlays blend over what is under them).
pub const NOT_YET_SHADERS: &[(&str, &str)] = &[
    ("CockpitGlass", "the canopy's inside glass (see-through: T4b)"),
    ("GlassSimple_", "the canopy's outside and edge glass (see-through: T4b)"),
    ("_AlphaDeferred_Overlay", "decals blended over the hull and chair"),
    ("CustomisationDecals", "the commander's decal slots, blended over the hull"),
    ("DetailStamp_NormalAO", "the Watson trim: a normal and occlusion stamp over the hull"),
    ("ShieldAlpha_", "the shield bubble"),
    ("Schematic_", "the HUD's hologram"),
    ("EDGeometryShadow", "the shadow caster"),
    ("EmissiveMapPulse", "the tutorial's highlight overlays"),
];

/// Models not drawn yet, by the start of the model's file name, and why.
pub const NOT_YET_MODELS: &[(&str, &str)] = &[
    ("shard0", "the canopy blowout's 20 shards"),
    ("sidewinder_screen", "the canopy glass and its damaged twin"),
    ("sidewinder_schematic", "the HUD's hologram"),
    ("sidewinder_shield", "the shield bubble"),
    ("cockpit_ui_hightlight_", "the tutorial's highlight overlays"),
    ("cockpitchair_bone_rig", "a 3-vertex placeholder"),
    ("commandercreator_cockpit_low", "a 4-vertex placeholder"),
];

/// Is this model drawn in this task?
pub fn model_drawn(model: &str) -> bool {
    !NOT_YET_MODELS.iter().any(|(start, _)| model.starts_with(start))
}

/// Is this material drawn in this task? Not a name Elite draws with its fallback (`fallback_to`), nor a family above.
pub fn material_drawn(m: &EliteMaterial) -> bool {
    m.fallback_to.is_none() && m.shader.as_deref().is_some_and(|s| !NOT_YET_SHADERS.iter().any(|(part, _)| s.contains(part)))
}

/// Where a material's base colour comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    /// A texture in the mod's folder, sampled as sRGB or not by its flag; its rgb is the colour.
    Texture { file: String, srgb: bool },
    /// One linear colour.
    Colour([f32; 3]),
}

/// The paint key whose colour the painted hull shows where the mask has none of the three keys (the default paint job's
/// mask leaves most of the hull there).
const BACKGROUND_KEY: &str = "Background";

/// A material's base colour: the painted hull's (`*SpacecraftTilePainted*`) is the paint job's Background key; any other's
/// its `pBaseColour_Roughness`, else the tile families' first (`pBaseColour_Roughness_0`), else `pDiffuse_AlphaTexture`.
/// A constant slot's first RGBA8 is read as sRGB, as the textures in those slots are. None when it has none of them.
pub fn base_colour(m: &EliteMaterial, paint: &Paint) -> Option<Base> {
    if m.shader.as_deref().is_some_and(|s| s.contains("SpacecraftTilePainted")) {
        return paint.keys.iter().find(|k| k.name == BACKGROUND_KEY).map(|k| Base::Colour(k.colour.map(|c| c as f32)));
    }
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

/// What the shader reads besides the light and the texture.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct ShipParams {
    /// rgb the base colour when it is one colour (`Base::Colour`).
    pub colour: Vec4,
    /// x 1 when the base colour is the texture's, 0 when it is `colour`; y 1 for the seat's view, z its near plane
    /// (`SEAT_NEAR_M`), w its share of the depth range (`SEAT_SLICE`).
    pub flags: Vec4,
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

/// The ship's material: the sandbox's two light buffers, one base colour texture with a repeating trilinear sampler, and
/// `ShipParams`. Made once; never changed.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ShipMaterial {
    #[storage(0, read_only)]
    pub frame: Handle<ShaderBuffer>,
    #[storage(1, read_only)]
    pub light: Handle<ShaderBuffer>,
    #[texture(2)]
    #[sampler(3)]
    pub base: Handle<Image>,
    #[uniform(4)]
    pub params: ShipParams,
}

impl ShipMaterial {
    /// A material of base colour `base`: `texture` is that texture's image when it is one, `white` any other's; `seat_view`
    /// for the pieces seen from the seat (drawn in the seat's depth slice).
    pub fn new(light: &PropLight, base: Option<&Base>, texture: Option<Handle<Image>>, white: &Handle<Image>, seat_view: bool) -> ShipMaterial {
        let (colour, textured) = match base {
            Some(Base::Colour(c)) => (Vec4::new(c[0], c[1], c[2], 1.0), false),
            Some(Base::Texture { .. }) => (Vec4::ONE, texture.is_some()),
            // (None of the slots: plain white, so it shows rather than vanishes.)
            None => (Vec4::ONE, false),
        };
        ShipMaterial {
            frame: light.frame.clone(),
            light: light.light.clone(),
            base: texture.filter(|_| textured).unwrap_or_else(|| white.clone()),
            params: ShipParams { colour, flags: Vec4::new(if textured { 1.0 } else { 0.0 }, if seat_view { 1.0 } else { 0.0 }, SEAT_NEAR_M, SEAT_SLICE) },
        }
    }
}

/// A 1x1 white texture for materials whose base colour is one colour.
pub fn white(images: &mut Assets<Image>) -> Handle<Image> {
    images.add(Image::new(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![255; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

impl Material for ShipMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sidewinder_ship.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sidewinder_ship.wesl".into()
    }

    fn specialize(_pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, _key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_TANGENT.at_shader_location(2),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(3),
            Mesh::ATTRIBUTE_UV_1.at_shader_location(4),
            ATTRIBUTE_UV_2.at_shader_location(5),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

pub fn build(app: &mut App) {
    app.add_plugins(MaterialPlugin::<ShipMaterial>::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables keep what this task doesn't draw out, and the families it does in.
    #[test]
    fn drawn_now() {
        let m = |shader: &str| EliteMaterial { shader: Some(shader.into()), textures: Vec::new(), attrs: Vec::new(), fallback_to: None, why: None };
        for s in ["Metalness_Roughness_OpaqueDeferred", "Metalness_Roughness_OpaqueDeferred_SpacecraftTilePainted_Normals", "Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_Emissive"] {
            assert!(material_drawn(&m(s)), "{s}");
        }
        for s in ["CockpitGlass", "GlassSimple_Tintable_Roughness_Alpha", "Metalness_Roughness_AlphaDeferred_Overlay", "DetailStamp_NormalAO", "OpaquePhysicalDeferred_LegacyGlossOption_CustomisationDecals0_DeferredAlphaBlendEx"] {
            assert!(!material_drawn(&m(s)), "{s}");
        }
        let fallback = EliteMaterial { shader: None, textures: Vec::new(), attrs: Vec::new(), fallback_to: Some("null".into()), why: None };
        assert!(!material_drawn(&fallback));
        assert!(model_drawn("sidewinder") && model_drawn("landinggearfront") && model_drawn("cockpit_chair"));
        assert!(!model_drawn("shard014") && !model_drawn("sidewinder_screen_damagestate_02") && !model_drawn("cockpit_ui_hightlight_radar"));
    }
}
