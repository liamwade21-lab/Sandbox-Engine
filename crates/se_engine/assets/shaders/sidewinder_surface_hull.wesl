// The Sidewinder's outside surfaces (ARCHITECTURE.md A6): one function per Elite shader family, each computing one texel of
// Elite's G-buffer (sidewinder_lit.wesl `Surface`) exactly as that family's G-buffer pixel shader does: which UV set each
// texture takes, the channel packing, the scales, every attribute. sidewinder_ship.wesl calls the material's own (its family
// key, material.rs `Family`); sidewinder_lit.wesl lights the result.
//
// The variant of each family a default draw of the stock ship uses (no damage, no paint swap, no hyperspace sweep, no
// material debug view), from its inputs, textures and constants against the material's slots and attributes. Files are in
// re/elite/frea_export/sidewinder/decoded/shaders/ unless marked (w): the export missed the variants whose bytecode Elite's
// effect archive stores under another effect's name (it keeps each shader once, by hash); they were found by that hash and
// disassembled into re/elite/work/paint/t4a2a_shaders/ (`t4a2a_effshaders.py`).
//
// | Family                                                    | Variant (pixel shader)                                          | Why this one |
// |-----------------------------------------------------------|-----------------------------------------------------------------|--------------|
// | Metalness_Roughness_OpaqueDeferred_SpacecraftTilePainted_Normals | ps_..._SpacecraftTilePainted_Normals_GBuffer_0_0_0_0_0_0 | digit 5 = the paint swap (reads pPaintBlendPlaneWS, the secondary job), digit 4 = 1 iridescence / 2 iridescence and the Dynamic effect (the paint job has neither), digit 3 = the material debug view (cb1[92]), digit 1 = the atlas path without the emissive rescaling |
// | Metalness_Roughness_OpaqueDeferred_SpacecraftSingleTile    | ps_..._SpacecraftSingleTile_GBuffer_0_0_0_0                    | 0_0_1_0 is the debug view |
// | Metalness_Roughness_OpaqueDeferred                         | (w) ps_Debug_IBL_CockpitPartition_GBuffer_0_0_0_0 (effect Metalness_Roughness_OpaqueDeferred_GBuffer's own) | 0_0_1_0 the debug view; 0_1_0_0 folds the specular colour into the diffuse for metals (o1 = lerp(diffuse, F0, metalness)): possible only if that switch were a quality setting, which nothing shows; the forward reference lights the unfolded pair |
// | Metalness_Roughness_OpaqueDeferred_Emissive                | ps_..._Emissive_GBuffer_0_0_0_0                                | 0_0_1_0 the debug view; 1_0_0_0 the same fold as above |
// | Metalness_Roughness_AlphaDeferred_Overlay                  | (w) ps_Metalness_Roughness_AlphaDeferred_GBuffer_0_0_0 (effect ..._AlphaDeferred_Overlay_GBuffer's own) | 0_1_0 the debug view; 1_0_0 the same fold |
// | DetailStamp_NormalAO                                       | ps_DetailStamp_NormalAO_GBufferStampNormals_0_0                | the only one |
// | ...CustomisationDecals0/1/2_DeferredAlphaBlendEx           | ps_..._GBuffer0_0_0_0_0 (one layer; 0_0_0_1 cross-fades two decals) | not drawn: its maths gives coverage 0 everywhere with no decal fitted (material.rs `decal_slot_coverage`, proven on the CPU from the constants: pDecalNAlpha0 0 and the paint job's group 3 alpha 0) |
//
// Constant slots are 1x1 textures of the slot's colour / 255, linear (the emissive shaders' material-parameter path decodes a
// constant colour exactly so: `and 255`, `ubfe 8, 8`, `ubfe 8, 16`, `utof`, x 1/255). One sampler for every texture: Elite's
// QualitySettingSampler (s0) is the only one these shaders declare, but the detail stamp's occlusion map takes its
// BilinearSampler (s1) and here the same anisotropic one.
//
// Attributes the chosen variants do not read (material.rs `Family::ignores` lists them, and a test checks that every
// attribute of every drawn material is either read or listed): bfrAtlasModel* and pVerticalTiling (the atlas models' vertex
// extrusion), pRenderLayerOverride (the engine's draw order), pClipVertices (the vertex shader's), the painted hull's pPaintPrimary_* / pPaint_WSToVesselSpaceTransform_* (the paint job's globals replace
// them, from ship.json's paint record), pPaintSecondary_*, pPaintBlendPlaneWS and pSwapPaintEffect* (the swap variant),
// the iridescence and Dynamic attributes (their variants), pDiffuseGlossTweak_UV2 on nothing else.
//
// The overlays (decals) and the stamp: Elite blends them into the G-buffer (base colour, specular colour, roughness by
// their alpha; the stamp writes only the normal and the occlusion under it). Here the decals are drawn after the opaque
// surfaces, alpha-blended, depth-tested without writing depth, lit by the same lighting: blending lit colours instead of
// G-buffer values differs slightly where the alpha is between 0 and 1 (the lighting is not linear in roughness). Their own
// normal is the mesh's (Elite leaves the G-buffer's normal as it is, o0.yz not written; the decal mesh lies on the hull,
// whose normal maps are flat constants). The stamp is drawn the same way with a multiplying blend: the pixel under it is
// multiplied by how much the stamped normal and its occlusion change the diffuse light there (sidewinder_lit.wesl
// `stamp_factor`), the surface's colour and specular kept; it differs from Elite where the surface under it is shiny (its
// highlights are not re-aimed by the stamped normal) and in using the stamp mesh's normal for the surface's.
// Correction: a multiplying blend scales the whole pixel under the stamp, so its specular, its glow and the haze's light
// are scaled by the factor too, not only its diffuse light.
//
// The overlays' and the stamp's depth bias (pDepthBias; vs_Metalness_Roughness_AlphaDeferred_GBuffer_0_0_0 and
// vs_DetailStamp_NormalAO_GBufferStampNormals_0_0 alike): Elite moves the clip depth to z - (bias.x z 2^-23 + bias.z) (the
// immediate is 0x34000000, which the disassembly prints as 0.000000), and when bias.w is not 0 limits that shift to
// |w| bias.w. Its depth is reversed (1 near) and z / w = near / distance, so bias.x is in units of the depth value's
// float steps: -20354 (the chair's decals) moves them 20354 x 2^-23 = 0.24 % nearer, 1.2 mm at half a metre. Bevy's depth
// is reversed the same way, so `ship_depth` applies the same rule to the same quantity: to the world's clip depth outside,
// and from the seat to the slice's own near-plane depth before it is squeezed into the slice (the slice's compression
// would turn the same shift of the final depth into centimetres).

import super::sidewinder_lit::{Surface, ship_scene, emissive_rescale};

// Everything the families' attributes give (material.rs `ShipParams`, field for field).
struct ShipParams {
    // x 1 for the seat's view, y its near plane (m), z its share of the depth range, w pDeferredFlags.
    view: vec4<f32>,
    // x first, y count: the range of the ship's lights buffer that lights this material.
    lights: vec4<u32>,
    // The plain path (families not built yet): rgb the base colour, w 1 when it is the texture's.
    colour: vec4<f32>,
    // gStandardMapScales: x diffuse, y specular, z roughness.
    map_scales: vec4<f32>,
    // pEmissiveColour rgb, pEmissiveScale w.
    emissive: vec4<f32>,
    // pUVScaleOffset.
    uv_scale_offset: vec4<f32>,
    // pEmissiveRescalingEnable, Type, EVOffset, EnableClamping.
    rescale: vec4<f32>,
    // pEmissiveRescalingEVClamps.
    rescale_clamps: vec4<f32>,
    // pDirtDiffuse rgb, pDirtLevelScale w.
    dirt: vec4<f32>,
    // pDirtDiffuseOverride.
    dirt_override: vec4<f32>,
    // pDirtMapScales xy, pDiffuseGlossTweak_UV2 zw.
    dirt_gloss: vec4<f32>,
    // pLayerTop_BaseColourOverride, pLayerBottom_BaseColourOverride.
    layer_top: vec4<f32>,
    layer_bottom: vec4<f32>,
    // pLayerMaskCutoff, pLayerMaskUV1Scale, pLayerMaskSoftness, pTile1Metalness.
    layer_mask: vec4<f32>,
    // The paint job's keys Background, Red, Green, Blue (the engine's order): rgb the colour, a the roughness.
    keys: array<vec4<f32>, 4>,
    // Their metalness and emissive scale (Background, Red, Green, Blue).
    key_metal: vec4<f32>,
    key_emissive: vec4<f32>,
    // The paint job's RenderMode (R, G, B, A), UseSharpening (R, G, B), and the mask texture's 1/width, 1/height,
    // Use3x3Filter, SeparateTexturePerMask.
    render_mode: vec4<f32>,
    sharpening: vec4<f32>,
    paint_tex: vec4<f32>,
    // pNormalDetailMapUVScale xy, pNormalDetailMapStrength z.
    stamp: vec4<f32>,
    // pDepthBias (the overlays and the stamp; `ship_depth`).
    depth_bias: vec4<f32>,
};

// A vertex's clip depth (sidewinder_ship.wesl's and sidewinder_glass.wesl's vertex stages): the seat's view in the nearest
// slice of the depth range, as Source draws a view model (sandbox_viewmodel.wesl), with the world's field of view: x, y and
// w (the distance along the view) stay the world's; the depth is a near plane of its own squeezed into the slice (reverse z:
// 1 is the near plane), so the port's near plane never cuts the cockpit, it always hides the world behind it, and its own
// order is kept. `k` is the mesh's scale (scaled space shrinks the ship about the camera): taken out, the near plane is in
// true metres. Elite's depth bias moves the unsqueezed depth (the header).
fn ship_depth(clip: vec4<f32>, k: f32, p: ShipParams) -> f32 {
    if p.view.x > 0.5 {
        return clip.w * (1.0 - p.view.z) + p.view.z * biased_depth(p.view.y * k, clip.w, p.depth_bias);
    }
    return biased_depth(clip.z, clip.w, p.depth_bias);
}

// Elite's overlay depth bias on a clip depth `z` (the header).
fn biased_depth(z: f32, w: f32, bias: vec4<f32>) -> f32 {
    var shift = bias.x * z * 1.1920929e-7 + bias.z;
    if bias.w != 0.0 {
        let most = abs(w) * bias.w;
        shift = select(max(most, shift), min(most, shift), bias.w > 0.0);
    }
    return z - shift;
}

// The vertex's frame as Elite's pixel shaders build it: n normalised, t as interpolated, b = cross(n, t), turned round when
// the tangent's sign is negative (FACEINVARIANT's top bit; the mod's sign already accounts for the mirror to its axes).
struct Frame {
    n: vec3<f32>,
    t: vec3<f32>,
    b: vec3<f32>,
};

// A BC5 normal map's texel (x, y) in the frame: (x, y) x 2.007874 - 0.999985, z = sqrt(max(1 - x^2 - y^2, 0)), the
// normal x t + y b + z n normalised (every family's lines 'mad r.xy, r.xy, l(2.007874...)' to the 'rsq').
fn tangent_normal(f: Frame, texel: vec2<f32>) -> vec3<f32> {
    let xy = texel * 2.007874 - 0.999985;
    let z = sqrt(max(1.0 - dot(xy, xy), 0.0));
    return normalize(f.n * z + xy.x * f.t + xy.y * f.b);
}

// The specular colour of a base colour at a metalness: lerp(0.04, base, metalness).
fn metal_f0(base: vec3<f32>, metal: f32) -> vec3<f32> {
    return metal * (base - vec3<f32>(0.04)) + vec3<f32>(0.04);
}

// The paint job's mask texel at uv2 (the hull's third UV set): one tap, or the 3 x 3 filter spaced one mask texel apart
// (weights 1/16 corners, 1/8 sides, 1/4 middle) when Use3x3Filter is set. Gradients are taken by the caller in uniform
// control flow, so the taps may sit in a branch.
fn paint_texel(t: texture_2d<f32>, smp: sampler, uv: vec2<f32>, ddx: vec2<f32>, ddy: vec2<f32>, p: ShipParams) -> vec4<f32> {
    if p.paint_tex.z == 0.0 {
        return textureSampleGrad(t, smp, uv, ddx, ddy);
    }
    let d = p.paint_tex.xy;
    var c = textureSampleGrad(t, smp, uv - d, ddx, ddy) * 0.0625;
    c += textureSampleGrad(t, smp, uv + vec2<f32>(0.0, -d.y), ddx, ddy) * 0.125;
    c += textureSampleGrad(t, smp, uv + vec2<f32>(d.x, -d.y), ddx, ddy) * 0.0625;
    c += textureSampleGrad(t, smp, uv + vec2<f32>(-d.x, 0.0), ddx, ddy) * 0.125;
    c += textureSampleGrad(t, smp, uv, ddx, ddy) * 0.25;
    c += textureSampleGrad(t, smp, uv + vec2<f32>(d.x, 0.0), ddx, ddy) * 0.125;
    c += textureSampleGrad(t, smp, uv + vec2<f32>(-d.x, d.y), ddx, ddy) * 0.0625;
    c += textureSampleGrad(t, smp, uv + vec2<f32>(0.0, d.y), ddx, ddy) * 0.125;
    c += textureSampleGrad(t, smp, uv + d, ddx, ddy) * 0.0625;
    return c;
}

// The paint rule (the single-texture path; material.rs `paint_weights` is the same in Rust, tested): per mask channel by
// its render mode, 1 a hard threshold at 0.5, 2 none, anything else the value as it is; sharpening takes the channel's
// minimum with the next; then red first, green limited to what red leaves, blue to what both leave, the background the
// remainder. Returns (background, red, green, blue), the order of the paint job's packed constants. (The separate-texture
// path needs SeparateTexturePerMask, which material.rs refuses: the mod ships no per-mask textures.)
fn paint_weights(texel: vec4<f32>, p: ShipParams) -> vec4<f32> {
    let modes = vec4<i32>(floor(p.render_mode + 0.5));
    if !any(modes <= vec4<i32>(1)) {
        return vec4<f32>(1.0, 0.0, 0.0, 0.0);
    }
    var c = select(texel, step(vec4<f32>(0.5), texel), modes == vec4<i32>(1));
    c = select(c, vec4<f32>(0.0), modes == vec4<i32>(2));
    let sharp = p.sharpening.xyz > vec3<f32>(0.5);
    let red = select(c.r, min(c.g, c.r), sharp.x);
    let green = select(select(c.g, min(c.b, c.g), sharp.y), 0.0, sharp.x);
    let blue = select(select(c.b, min(c.a, c.b), sharp.z), 0.0, sharp.y);
    let g = min(green, 1.0 - red);
    let b = min(blue, saturate(1.0 - red - g));
    return vec4<f32>(saturate(1.0 - (red + g + b)), red, g, b);
}

// The painted hull. Port of ps_Metalness_Roughness_OpaqueDeferred_SpacecraftTilePainted_Normals_GBuffer_0_0_0_0_0_0.
// Textures: n_map pNormalMapUV1 (UV1), flake pDirt_Flake_0 (UV0), tile0 pBaseColour_Roughness_0 (UV0, green and alpha
// only), tile1 pBaseColour_Roughness_1 (UV0), paint the paint job's mask (UV2), dirt pDirtMaskMaterialMaskScruffinessAO
// (UV1: r dirt, g which tile, b scruffiness, a occlusion).
fn surface_painted(f: Frame, uv0: vec2<f32>, uv1: vec2<f32>, uv2: vec2<f32>, n_map: texture_2d<f32>, flake_map: texture_2d<f32>, tile0: texture_2d<f32>, tile1: texture_2d<f32>, paint: texture_2d<f32>, dirt_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    let ddx2 = dpdx(uv2);
    let ddy2 = dpdy(uv2);
    let t0 = textureSample(tile0, smp, uv0);
    let shade0 = mix(t0.g, p.layer_top.y, p.layer_top.w);
    let t1 = textureSample(tile1, smp, uv0);
    var c1 = mix(t1.rgb, p.layer_bottom.rgb, p.layer_bottom.w);
    let flake = textureSample(flake_map, smp, uv0).xy;
    let m = textureSample(dirt_map, smp, uv1);
    let dirt_level = saturate(m.r * p.dirt.w);
    // Which tile: the mask's green moved by the flake, cut at pLayerMaskCutoff over the pixel's footprint (coarse
    // derivatives, at least pLayerMaskSoftness); tile 1 (bare metal) is w1; a pixel on the cut takes 0.04 for tile 1's colour.
    let layer = saturate((flake.y * 2.0 - 1.0) * p.layer_mask.y + m.g);
    let dx = max(abs(dpdxCoarse(layer)), p.layer_mask.z);
    let dy = max(abs(dpdyCoarse(layer)), p.layer_mask.z);
    let both = saturate((dx * 0.5 + layer - p.layer_mask.x) / dx) + saturate((dy * 0.5 + layer - p.layer_mask.x) / dy);
    let w1 = both * 0.5;
    if both > 0.00002 && both <= 1.99998 {
        c1 = vec3<f32>(0.04);
    }
    // The paint job on tile 0: its keys mixed by the mask's weights, times tile 0's green.
    let w = paint_weights(paint_texel(paint, smp, uv2, ddx2, ddy2, p), p);
    let key = p.keys[1] * w.y + p.keys[0] * w.x + p.keys[2] * w.z + p.keys[3] * w.w;
    var metal = max(dot(w, p.key_metal), 0.0);
    var glow = p.keys[0].rgb * p.key_emissive.x * w.x + p.keys[1].rgb * p.key_emissive.y * w.y + p.keys[2].rgb * p.key_emissive.z * w.z + p.keys[3].rgb * p.key_emissive.w * w.w;
    let tweak = p.dirt_gloss.z * (m.b - 1.0) + 1.0;
    let c0 = tweak * (shade0 * key.rgb);
    var rough = 1.0 - (1.0 - key.a) * (1.0 - t0.a);
    rough = p.dirt_gloss.w * (m.b - 0.5) + rough;
    let base = saturate(c0 + w1 * (c1 - c0));
    rough = saturate(rough + w1 * (t1.a - rough));
    metal = metal + w1 * (p.layer_mask.w - metal);
    glow = glow * (1.0 - w1);
    // The dirt over both.
    let dirt = saturate(dirt_level * flake.x);
    let dirt_colour = mix(p.dirt.rgb, p.dirt_override.rgb, p.dirt_override.w);
    var s: Surface;
    s.diffuse = mix(base * (1.0 - metal), dirt_colour, dirt);
    s.specular = mix(metal_f0(base, metal), vec3<f32>(0.04), dirt);
    s.roughness = max(mix(rough, p.dirt_gloss.y, dirt), 0.001);
    s.normal = tangent_normal(f, textureSample(n_map, smp, uv1).xy);
    s.emissive = glow * (1.0 - dirt) * emissive_rescale(p.rescale, p.rescale_clamps.xy);
    s.ao = m.a;
    s.alpha = 1.0;
    s.flags = p.view.w;
    return s;
}

// Bare metal. Port of ps_Metalness_Roughness_OpaqueDeferred_SpacecraftSingleTile_GBuffer_0_0_0_0. Textures: n_map
// pNormalTexture (UV0), flake pDirt_Flake_0 (UV0, x), tile1 pBaseColour_Roughness_1 (UV0: the one tile, by its metalness
// pTile1Metalness and pLayerBottom, as tile 1 of the painted hull), dirt pDirtMaskMaterialMaskScruffinessAO (UV1: r, b, a).
fn surface_single_tile(f: Frame, uv0: vec2<f32>, uv1: vec2<f32>, n_map: texture_2d<f32>, flake_map: texture_2d<f32>, tile1: texture_2d<f32>, dirt_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    let flake = textureSample(flake_map, smp, uv0).x;
    let m = textureSample(dirt_map, smp, uv1);
    let dirt = saturate(saturate(m.r * p.dirt.w) * flake);
    let t = textureSample(tile1, smp, uv0);
    let tile_rough = saturate(p.dirt_gloss.w * (m.b - 0.5) + t.a);
    let tweak = p.dirt_gloss.z * (m.b - 1.0) + 1.0;
    let colour = saturate(tweak * mix(t.rgb, p.layer_bottom.rgb, p.layer_bottom.w));
    let metal = p.layer_mask.w;
    let dirt_colour = mix(p.dirt.rgb, p.dirt_override.rgb, p.dirt_override.w);
    var s: Surface;
    s.diffuse = mix(colour * (1.0 - metal), dirt_colour, dirt);
    s.specular = mix(metal_f0(colour, metal), vec3<f32>(0.04), dirt);
    s.roughness = max(mix(tile_rough, p.dirt_gloss.y, dirt), 0.001);
    s.normal = tangent_normal(f, textureSample(n_map, smp, uv0).xy);
    s.emissive = vec3<f32>(0.0);
    s.ao = m.a;
    s.alpha = 1.0;
    s.flags = p.view.w;
    return s;
}

// The gear, the bespoke parts, the strip plating, the outside chair. Port of (w) ps_Debug_IBL_CockpitPartition_GBuffer_0_0_0_0
// (Metalness_Roughness_OpaqueDeferred). Textures: n_map pNormalTexture, metal_map pMetalnessMap (x), base_map
// pBaseColour_Roughness (rgb, a roughness), all UV0.
fn surface_opaque(f: Frame, uv0: vec2<f32>, n_map: texture_2d<f32>, metal_map: texture_2d<f32>, base_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    let base = textureSample(base_map, smp, uv0);
    let metal = textureSample(metal_map, smp, uv0).x;
    var s: Surface;
    s.diffuse = base.rgb * (1.0 - metal) * p.map_scales.x;
    s.specular = metal_f0(base.rgb, metal) * p.map_scales.y;
    s.roughness = max(base.a * p.map_scales.z, 0.001);
    s.normal = tangent_normal(f, textureSample(n_map, smp, uv0).xy);
    s.emissive = vec3<f32>(0.0);
    s.ao = 1.0;
    s.alpha = 1.0;
    s.flags = p.view.w;
    return s;
}

// The glowing strips and the outside cockpit. Port of ps_Metalness_Roughness_OpaqueDeferred_Emissive_GBuffer_0_0_0_0: as
// `surface_opaque`, and the emitted light pEmissiveTexture (UV0 x pUVScaleOffset.xy + .zw) x pEmissiveColour x
// gGlobalSelfIllumScale x pEmissiveScale x the rescaling (the scene's values are sidewinder_lit.wesl's `ShipScene`).
fn surface_emissive(f: Frame, uv0: vec2<f32>, n_map: texture_2d<f32>, metal_map: texture_2d<f32>, base_map: texture_2d<f32>, glow_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    var s = surface_opaque(f, uv0, n_map, metal_map, base_map, smp, p);
    let glow = textureSample(glow_map, smp, uv0 * p.uv_scale_offset.xy + p.uv_scale_offset.zw).rgb;
    s.emissive = glow * p.emissive.rgb * (ship_scene.self_illum_scale * p.emissive.w) * emissive_rescale(p.rescale, p.rescale_clamps.xy);
    return s;
}

// The hull's decals. Port of (w) ps_Metalness_Roughness_AlphaDeferred_GBuffer_0_0_0 (Metalness_Roughness_AlphaDeferred_Overlay).
// Textures: ma_map pMetalnessAlphaMap (x metalness, y coverage), base_map pBaseColour_Roughness, both UV0. The normal is the
// mesh's (the header).
fn surface_overlay(f: Frame, uv0: vec2<f32>, ma_map: texture_2d<f32>, base_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    let base = textureSample(base_map, smp, uv0);
    let ma = textureSample(ma_map, smp, uv0).xy;
    var s: Surface;
    s.diffuse = base.rgb * (1.0 - ma.x) * p.map_scales.x;
    s.specular = metal_f0(base.rgb, ma.x) * p.map_scales.y;
    s.roughness = max(base.a * p.map_scales.z, 0.001);
    s.normal = f.n;
    s.emissive = vec3<f32>(0.0);
    s.ao = 1.0;
    s.alpha = ma.y;
    s.flags = 0.0;
    return s;
}

// The Watson trim's stamp. Port of ps_DetailStamp_NormalAO_GBufferStampNormals_0_0: the detail normal pNormalDetailMap
// (UV0 x pNormalDetailMapUVScale) in the frame with the surface's normal for its z, moved from the surface's normal by
// pNormalDetailMapStrength; the occlusion pAOMap (UV0). Returns (the stamped normal, the occlusion); the surface's normal
// is the stamp mesh's (the header).
struct Stamp {
    normal: vec3<f32>,
    ao: f32,
};

fn surface_stamp(f: Frame, uv0: vec2<f32>, detail_map: texture_2d<f32>, ao_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Stamp {
    let under = f.n;
    let stamped = tangent_normal(Frame(under, f.t, f.b), textureSample(detail_map, smp, uv0 * p.stamp.xy).xy);
    var out: Stamp;
    out.normal = normalize(under + p.stamp.z * (stamped - under));
    out.ao = textureSample(ao_map, smp, uv0).x;
    return out;
}

// The families not built yet (the cockpit's, the chair's): their base colour alone, as before this task, lit as a matte
// dielectric (roughness 1, F0 0.04).
fn surface_plain(f: Frame, uv0: vec2<f32>, base_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> Surface {
    let texel = textureSample(base_map, smp, uv0).rgb;
    var s: Surface;
    s.diffuse = select(p.colour.rgb, texel, p.colour.w > 0.5);
    s.specular = vec3<f32>(0.04);
    s.roughness = 1.0;
    s.normal = f.n;
    s.emissive = vec3<f32>(0.0);
    s.ao = 1.0;
    s.alpha = 1.0;
    s.flags = 0.0;
    return s;
}
