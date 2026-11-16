// The Sidewinder's cockpit surfaces (ARCHITECTURE.md A6): the cockpit's tiles, the chair's and the consoles' families, each
// computing one texel of Elite's G-buffer (sidewinder_lit.wesl `Surface`) exactly as that family's G-buffer pixel shader
// does, as sidewinder_surface_hull.wesl does for the outside (its header has the method). sidewinder_ship.wesl calls the
// material's own (material.rs `Family`); sidewinder_lit.wesl lights the result. This file also holds `CockpitParams`, the
// attributes these families and the glass (sidewinder_glass.wesl) read beyond `ShipParams`.
//
// The variant each family's stock draw uses; files in re/elite/frea_export/sidewinder/decoded/shaders/:
//
// | Family (Elite effect)                                                                   | Variant (pixel shader)                    | Why this one |
// |-----------------------------------------------------------------------------------------|-------------------------------------------|--------------|
// | Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride                            | ps_..._BaseColourOverride_GBuffer_0_0_0_0 | 0_0_1_0 the material debug view; 0_1_0_0 folds the specular colour into the diffuse for metals (as the hull's header says of its families) |
// | Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_Emissive                   | ps_..._Emissive_GBuffer_0_0_0_0           | 0_0_1_0 the debug view; 1_0_0_0 the same fold |
// | Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_DeferredAOMapUV2_RoughnessUV1 | ps_..._RoughnessUV1_GBuffer_0_0_0_0    | 0_0_1_0 the debug view (no fold variant exists) |
//
// Textures in each variant's own order (material.rs `Family::slots`); which UV set each takes is the disassembly's (the
// vertex shader's outputs: TEXCOORD0.xy = UV0, TEXCOORD0.zw = UV1, TEXCOORD2 = UV2). Constant slots are 1x1 textures as on
// the outside. One sampler: the tiles' occlusion map takes Elite's BilinearSampler (s1), here the same anisotropic one as
// every other map (the hull's stamp does the same).
//
// The cockpit colour override (the three families alike): the vertex shader writes __USER_MATERIALMODULATION_DATAID = (the
// ship's modulation record from its instance data, the instance id with the tangent's sign, the material's record from the
// vertex's byte 15); the pixel shader then
// - when a modulation record is set (x != 0), the material's pMaterialModulationSlot is >= 0 and that slot's bit is on in
//   the record, replaces or tints the base colour, the roughness and the metalness by the record's half-float values per
//   slot (mode 1 the value x the texel's luminance, 2 the value x the texel, 3 the value);
// - otherwise mixes the base colour toward its luminance (0.3086, 0.6094, 0.082) x pOverrideBaseColour.rgb by
//   pOverrideBaseColour.w.
// The mod has no modulation record (the customisation that fills gMaterialModulationInstanceDataBuffer is not built) and
// every cockpit material's slot is -1, so only the second applies, and with pOverrideBaseColour (1, 1, 1, 0) on every
// cockpit material it leaves the colour as it is (material.rs `cockpit_override_is_identity` proves both from the files).
// Byte 15 picks the draw's own material, which here is each draw's own `ShipMaterial`. If Elite set the override at run time
// (w > 0) the cockpit's colours would turn toward a grey of their own brightness tinted by the override's colour; a
// modulation record would recolour whole slots (the cockpit colour customisation).
//
// What Elite's G-buffer resolve does after these (the cockpit's own light probe, pUseCockpitIBL's partition) has no
// counterpart: the cockpit's ambient is the sandbox's sky light as on the outside (sidewinder_lit.wesl's header, ours).

import super::sidewinder_lit::{Surface, ship_scene, emissive_rescale};
import super::sidewinder_surface_hull::{ShipParams, Frame, tangent_normal, metal_f0};

// The attributes the cockpit's and the glass's families read beyond `ShipParams` (material.rs `CockpitParams`, field for
// field). Zero for every other family.
struct CockpitParams {
    // pOverrideBaseColour.
    override_colour: vec4<f32>,
    // pRoughnessDetailMapUVScale xy, pRoughnessDetailMapStrength z, pBaseColourDetailMapStrength w.
    detail: vec4<f32>,
    // The cockpit glass: pCockpitTile xy, pPSTB_GridTileRate z, pCockpitRefractionAmount w.
    glass_tile: vec4<f32>,
    // pCracksVertexMask, pIceVertexMask, pDirtVertexMask, pImperfectionVertexMask, pCondensationVertexMask.
    masks: array<vec4<f32>, 5>,
    // pIceMaskScaleOffset xy, pImperfectionMaskScaleOffset zw.
    mask_scale_offset: vec4<f32>,
    // pCondensationMaskScaleOffset xy, pCondensationTile zw.
    condensation: vec4<f32>,
    // pCracksOcclusionScale, pIceOcclusionScale, pCondensationOcclusionScale, pDirtAlphaMultiplier.
    occlusion: vec4<f32>,
    // pCracksColour rgb, pCracksMaskThreshold w.
    cracks_colour: vec4<f32>,
    // pIceColour rgb, pIceSceneColourDistortion w.
    ice_colour: vec4<f32>,
    // pCondensationColour rgb, pCondensationSceneColourDistortion w.
    condensation_colour: vec4<f32>,
    // pDirtDiffuse rgb, pLayerMaskCutoff w.
    dirt: vec4<f32>,
    // pCracksRoughness, pIceRoughness, pCondensationRoughness, pImperfectionsRoughness.
    roughness: vec4<f32>,
    // pDirtRoughness x, pGlassThickness y.
    dirt_roughness: vec4<f32>,
    // pSmearsPeriods xy, pSmearsScaleOffset zw.
    smears: vec4<f32>,
    // The glass effects' globals (material.rs `GlassEffects`): gCockpitGlassCrackAmount, gCockpitGlassIceScale,
    // gCockpitGlassCondensationScale, gCockpitGlassTintAlpha.
    effects: vec4<f32>,
    // gCockpitGlassAlphaMultiplier x, gCockpitGlassState y.
    effects_more: vec4<f32>,
    // The outside glass: pGlassColourTint rgb, pDistanceToOpaque w.
    tint: vec4<f32>,
    // pGlassColourTint_Distant rgb, pFadeRange w.
    tint_distant: vec4<f32>,
    // pFresnel_ScalePowerBias xyz, pFresnelVal w.
    fresnel: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(14) var<uniform> cockpit: CockpitParams;

// gCockpitEmissiveScale (cb1[55].x, light_cb_layout.json): the Sidewinder's cockpit state machine sets the generic parameter
// CockpitEmissiveControl to 1.0 when it starts (SetGenericParamActivity) and DriveShaderGlobalActivity copies it into this
// global every frame (re/elite/work/csm/logic/notes.md); nothing else of the ship writes it.
const COCKPIT_EMISSIVE_SCALE: f32 = 1.0;

// The weights of the override's luminance (every cockpit family's 'dp3 ..., l(0.308600, 0.609400, 0.082000, 0)').
const OVERRIDE_LUMA: vec3<f32> = vec3<f32>(0.3086, 0.6094, 0.082);

// The colour override with no modulation record (the header): the base colour mixed toward its luminance x
// pOverrideBaseColour.rgb by pOverrideBaseColour.w.
fn override_base(c: vec3<f32>) -> vec3<f32> {
    return mix(c, dot(c, OVERRIDE_LUMA) * cockpit.override_colour.rgb, cockpit.override_colour.w);
}

// The chair's body, back, details and seat, and the cockpit's bespoke parts. Port of
// ps_Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_GBuffer_0_0_0_0. Textures (all UV0): n_map pNormalTexture,
// metal_map pMetalnessMap (x), base_map pBaseColour_Roughness (rgb, a roughness). The occlusion is 1 (o2.w), nothing glows.
fn surface_cockpit(f: Frame, uv0: vec2<f32>, n_map: texture_2d<f32>, metal_map: texture_2d<f32>, base_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    let metal = textureSample(metal_map, smp, uv0).x;
    let base = textureSample(base_map, smp, uv0);
    let colour = override_base(base.rgb);
    var s: Surface;
    s.diffuse = colour * (1.0 - metal) * p.map_scales.x;
    s.specular = metal_f0(colour, metal) * p.map_scales.y;
    s.roughness = max(base.a * p.map_scales.z, 0.001);
    s.normal = tangent_normal(f, textureSample(n_map, smp, uv0).xy);
    s.emissive = vec3<f32>(0.0);
    s.ao = 1.0;
    s.alpha = 1.0;
    s.flags = p.view.w;
    return s;
}

// The consoles, the chair's controls and their details. Port of
// ps_Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_Emissive_GBuffer_0_0_0_0: as `surface_cockpit`, and the
// emitted light pEmissiveTexture (UV0 x pUVScaleOffset.xy + .zw; a constant slot would be the material record's packed
// colour, which no cockpit material has) x pEmissiveColour x gGlobalSelfIllumScale x pEmissiveScale x gCockpitEmissiveScale x
// the rescaling. gGlobalSelfIllumScale and the rescaling's scene values are sidewinder_lit.wesl's `ShipScene`; the
// output's gHDRScalingWrite is left out as on the outside (it scales every light alike).
fn surface_cockpit_emissive(f: Frame, uv0: vec2<f32>, n_map: texture_2d<f32>, metal_map: texture_2d<f32>, base_map: texture_2d<f32>, glow_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    var s = surface_cockpit(f, uv0, n_map, metal_map, base_map, smp, p);
    let glow = textureSample(glow_map, smp, uv0 * p.uv_scale_offset.xy + p.uv_scale_offset.zw).rgb;
    s.emissive = glow * p.emissive.rgb * (ship_scene.self_illum_scale * p.emissive.w) * COCKPIT_EMISSIVE_SCALE * emissive_rescale(p.rescale, p.rescale_clamps.xy);
    return s;
}

// The 13 tiles. Port of ps_Metalness_Roughness_CockpitOpaqueDeferred_BaseColourOverride_DeferredAOMapUV2_RoughnessUV1_GBuffer_0_0_0_0.
// Textures: n_map pNormalTexture, metal_map pMetalnessMap (x), base_map pBaseColour_Roughness, all UV0; detail_map
// pNormalDetailMap (UV1 x pNormalDetailMapUVScale) and rough_map pRoughnessDetailMap (UV1 x pRoughnessDetailMapUVScale,
// x); ao_map pAOMap (UV2, x). The roughness detail moves both the roughness (+ detail x pRoughnessDetailMapStrength) and the
// base colour (toward itself x pBaseColourDetailMapStrength by the detail); the detail normal is built on the mapped normal
// and the normal moved toward it by pNormalDetailMapStrength (`ShipParams::stamp`: the same two attributes the hull's stamp
// reads). This variant applies gStandardMapScales to the roughness only.
fn surface_cockpit_tile(f: Frame, uv0: vec2<f32>, uv1: vec2<f32>, uv2: vec2<f32>, n_map: texture_2d<f32>, metal_map: texture_2d<f32>, base_map: texture_2d<f32>, detail_map: texture_2d<f32>, rough_map: texture_2d<f32>, ao_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    let metal = textureSample(metal_map, smp, uv0).x;
    let base = textureSample(base_map, smp, uv0);
    var colour = override_base(base.rgb);
    let mapped = tangent_normal(f, textureSample(n_map, smp, uv0).xy);
    let detail_rough = textureSample(rough_map, smp, uv1 * cockpit.detail.xy).x;
    let rough = saturate(base.a * p.map_scales.z + detail_rough * cockpit.detail.z);
    colour = saturate(colour + detail_rough * (colour * cockpit.detail.w - colour));
    let detail = tangent_normal(Frame(mapped, f.t, f.b), textureSample(detail_map, smp, uv1 * p.stamp.xy).xy);
    var s: Surface;
    s.diffuse = colour * (1.0 - metal);
    s.specular = metal_f0(colour, metal);
    s.roughness = max(rough, 0.001);
    s.normal = normalize(mapped + p.stamp.z * (detail - mapped));
    s.emissive = vec3<f32>(0.0);
    s.ao = textureSample(ao_map, smp, uv2).x;
    s.alpha = 1.0;
    s.flags = p.view.w;
    return s;
}
