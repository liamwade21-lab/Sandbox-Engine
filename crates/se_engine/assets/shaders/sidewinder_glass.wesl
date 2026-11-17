// The Sidewinder's glass (ARCHITECTURE.md A6): the canopy seen from the seat, the canopy seen from outside, the canopy's
// edges. Elite draws them with forward lit shaders; each function here is that shader's surface maths, lit by the parts of
// sidewinder_lit.wesl that are the same maths (`specular_brdf`, `shade` with the ship's lamps' loop), and returns the colour and
// coverage of a premultiplied-over blend (material.rs: `AlphaMode::Premultiplied`, drawn after everything opaque of its
// view, depth-tested, no depth written; from the seat in the seat's depth slice and sorted after the world's see-through
// things, `SEAT_GLASS_BIAS`). The glass has its own vertex stage (`glass_vertex`): its shaders read the vertex's colour
// (bytes 20-23), which no other family reads.
//
// The variant of each family a stock draw uses; files in re/elite/frea_export/sidewinder/decoded/shaders/:
//
// | Family (material)                                        | Variant (pixel shader; vertex shader)                         | Why this one |
// |----------------------------------------------------------|---------------------------------------------------------------|--------------|
// | CockpitGlass (cockpit_glass_int)                          | ps_CockpitGlass_Forward_0_1_0_3_0; vs_..._0_1_0_0_0           | digit 3 is gCockpitGlassState (0 clean, 1 ice, 2 condensation: FUN_1434dd710 writes it, material.rs `GlassEffects::state`), 0 for the stock ship; digit 4 the quality (2 adds the blemish map's stochastic tiling): 3. It has no lamp loop: its loops are the star's shadow cascades, the directional lights and a probe loop inside the crack block, and it reads none of the clustered lights' tables (cb1[249..252]), so the ship's lamps never light the canopy seen from the seat |
// | GlassSimple_Tintable_Roughness_Alpha (shipexternalglassmaterial) | ps_..._Forward_0_1_0_3_0; vs_..._Forward_0_1_0_0_0      | digit 3 = 1 folds the specular colour into the diffuse for metals; digit 1 and 4 the lights' loops (3 the full set; both read the clustered lights' tables, so the ship's lamps light them) |
// | GlassSimple_Roughness_Alpha_SingleSource (cockpit_glass_edges) | ps_..._Forward_0_1_0_3_0                                | as above |
//
// What has no counterpart in this forward pass, and how much of the look depends on it:
// - The canopy from the seat, stock state: nothing. Its shader samples no scene texture and no probe there: its colour is
//   the star's and the lamps' light on its dirt and imperfections plus their glints, its coverage the layers' (with
//   gCockpitGlassTintAlpha 0 the clean glass covers nothing, so the world shows through untouched). The refraction amount
//   only bends the view vector its glints are seen along (Elite's own maths, ported).
// - The cracks: gCockpitGlassCrackAmount comes each frame from `ship_glass` (the cracks activity's level, parts.rs
//   `Cracks`; 0 while the ship's damage is 0, which is all the mod feeds: its writer was not found), so the crack key
//   (colour . pCracksVertexMask + 1/255) x amount passes pCracksMaskThreshold only on a damaged canopy. Drawn: the crack
//   layer (its alpha, normal, colour, occlusion and roughness, all above). Not ported: their interior reflection (state 0's
//   last block, lines 713-1047: run only where pCracksIDTexture's texel differs between the pixel's uv and the uv moved
//   along the refracted view by cb2[14].x (lines 144-155); it reads pCracksInteriorNormTexture, then loops over the clustered
//   light probes (the probe index tables t0 / t2, Elite's probe cube array t3) weighting each by its box or sphere, with the
//   split-sum LUT g_Tex_SpecularFresnelLUTTexture (t4), and lifts the coverage by 0.2 x the crack's alpha towards 0.95).
//   It is not an addition to what is drawn: it needs Elite's probes (no counterpart: the scene is SpaceEngine's) and the
//   LUT (not among the mod's files); the two crack textures it reads are (tex/cockpit_glass_int__pcracksidtexture.dds,
//   __pcracksinteriornormtexture.dds). The ice and condensation variants (digit 3 = 1, 2) are not ported either: they
//   sample gBlurSceneTexture, the blurred scene behind the glass, distorted by their normals; nothing writes their scales
//   (material.rs `GlassEffects`), so the state stays 0 (`stock_glass_shows_no_damage`).
// - The outside glass's environment reflection: Elite's light probes stand in as the sandbox's sky light seen along the
//   reflected ray, and its split-sum LUT as the usual analytic fit of it (`shade`'s specular ambient, `env_brdf`: ours).
//   This is most of what the outside canopy shows by day away from the sun: a grey reflection of the sky over the dimmed
//   cockpit.
// - The outside glass's transmission is a second render target that Elite's dual-source blend multiplies the scene by
//   (o1: the glass's colour x the tint, faded to 0 with distance). Dual-source blending is not enabled in this game, so
//   it is the blend's coverage (1 - transmission) instead, which would be exact if the transmission's channels were equal:
//   both tints are grey (material.rs `outside_glass_tints_are_grey`, which proves only that) and the stock glass's colour
//   texture is grey to within 3 of 255 between its channels (measured), so the grey coverage is within that of Elite's
//   coloured transmission.
// - Blend states are not in the disassembly. The canopy's shader writes its albedo already multiplied by its coverage and
//   its glints whole: a premultiplied-over output. The outside and edge glasses write reflected light and a coverage of 1
//   everywhere on the stock ship (a constant slot): premultiplied and straight blending agree there.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world, mesh_tangent_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::view;
import super::sidewinder_lit::{Surface, Scene, INV_PI, shade, specular_brdf};
import super::sidewinder_surface_hull::{ShipParams, Frame, ship_depth, tangent_normal, metal_f0};
import super::sidewinder_surface_cockpit::cockpit;

// The canopy's glass effects' globals this frame (material.rs `GlassEffects`, `ShipGlass`, written in place by draw.rs):
// `effects` gCockpitGlassCrackAmount, gCockpitGlassIceScale, gCockpitGlassCondensationScale, gCockpitGlassTintAlpha; `more`
// gCockpitGlassAlphaMultiplier x, gCockpitGlassState y.
struct ShipGlass {
    effects: vec4<f32>,
    more: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(18) var<storage, read> ship_glass: ShipGlass;

struct GlassVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // xyz Elite's tangent, w the frame's sign (both already in the mod's axes).
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    // Bytes 20-23 / 255 (material.rs `ATTRIBUTE_COLOUR`).
    @location(6) colour: vec4<f32>,
};

struct GlassOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_tangent: vec4<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) world_pos: vec3<f32>,
    // The canopy: Elite's __USER_KEY_COCKPITGLASSMASKS (x the crack key, y the dirt's, z the imperfections'). The outside
    // glass: its __USER_GLASSCOLOURTINTRANGE (rgb the tint, w how far it has turned opaque).
    @location(4) key: vec4<f32>,
};

// The vertex stage: placed as sidewinder_ship.wesl's `vertex` places every other draw (`ship_depth`), and the vertex
// shaders' keys.
fn glass_vertex(v: GlassVertex, p: ShipParams) -> GlassOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    // (Scaled space shrinks the ship about the camera: k turns render-frame lengths back into metres.)
    let k = length(m[0].xyz);
    var o: GlassOut;
    o.clip = position_world_to_clip(world.xyz);
    o.clip.z = ship_depth(o.clip, k, p);
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.world_tangent = mesh_tangent_local_to_world(m, v.tangent, v.instance_index);
    o.uv0 = v.uv0;
    o.world_pos = world.xyz;
    @if(SHIP_GLASS_COCKPIT) {
        // vs_CockpitGlass_Forward_0_1_0_0_0: crack = (colour . pCracksVertexMask + 1/255) x gCockpitGlassCrackAmount; dirt =
        // saturate((colour . pDirtVertexMask + offset) x scale) x pDirtAlphaMultiplier and imperfection the same without
        // the multiplier, both by pImperfectionMaskScaleOffset (scale .z, offset .w: the shader's own choice for both).
        let so = cockpit.mask_scale_offset;
        o.key = vec4<f32>(
            (dot(v.colour, cockpit.masks[0]) + 0.003922) * ship_glass.effects.x,
            saturate((dot(v.colour, cockpit.masks[2]) + so.w) * so.z) * cockpit.occlusion.w,
            saturate((dot(v.colour, cockpit.masks[3]) + so.w) * so.z),
            0.0,
        );
    }
    @if(SHIP_GLASS_TINTED) {
        // vs_GlassSimple_Tintable_Roughness_Alpha_Forward_0_1_0_0_0: t = saturate((the view depth - pDistanceToOpaque) /
        // max(pFadeRange, 0.001)); the tint mix(pGlassColourTint, pGlassColourTint_Distant, t) kept inside (1e-6, 0.999999).
        let forward = -normalize(view.world_from_view[2].xyz);
        let depth = dot(forward, world.xyz - view.world_position) / k;
        let t = saturate((depth - cockpit.tint.w) / max(cockpit.tint_distant.w, 0.001));
        o.key = vec4<f32>(clamp(mix(cockpit.tint.rgb, cockpit.tint_distant.rgb, t), vec3<f32>(0.000001), vec3<f32>(0.999999)), t);
    }
    @if(SHIP_GLASS_EDGES) {
        o.key = vec4<f32>(0.0);
    }
    return o;
}

// The glass's frame as Elite's pixel shaders build it (sidewinder_surface_hull.wesl `Frame`).
fn frame_of(in: GlassOut) -> Frame {
    let n = normalize(in.world_normal);
    return Frame(n, in.world_tangent.xyz, cross(n, in.world_tangent.xyz) * select(1.0, -1.0, in.world_tangent.w < 0.0));
}

// pPSTB_GridTileRate's stochastic tiling of the blemish map (ps_CockpitGlass_Forward_0_1_0_3_0 lines 51-117): the uv on a
// skewed grid of triangles `rate` to a tile, each corner's tap offset by a hash of its cell, the three taps weighted by the
// point's barycentric weights, every tap with the gradients of the plain uv (coarse, as the shader takes them).
fn pstb_hash(cell: vec2<f32>) -> vec2<f32> {
    return fract(sin(vec2<f32>(dot(cell, vec2<f32>(127.1, 311.7)), dot(cell, vec2<f32>(269.5, 183.3)))) * 43758.546875);
}

fn blemish_stochastic(t: texture_2d<f32>, smp: sampler, uv: vec2<f32>, rate: f32) -> vec2<f32> {
    let ddx = dpdxCoarse(uv);
    let ddy = dpdyCoarse(uv);
    let g = uv * rate;
    let skewed = vec2<f32>(g.x, dot(vec2<f32>(-0.57735, 1.154701), g));
    let cell = floor(skewed);
    let fr = fract(skewed);
    let w0 = 1.0 - fr.x - fr.y;
    var a = cell;
    var b = cell + vec2<f32>(0.0, 1.0);
    var c = cell + vec2<f32>(1.0, 0.0);
    var w = vec3<f32>(w0, fr.y, fr.x);
    if w0 <= 0.0 {
        a = cell + vec2<f32>(1.0, 1.0);
        b = cell + vec2<f32>(1.0, 0.0);
        c = cell + vec2<f32>(0.0, 1.0);
        w = vec3<f32>(-w0, 1.0 - fr.y, 1.0 - fr.x);
    }
    let ta = textureSampleGrad(t, smp, uv + pstb_hash(a), ddx, ddy).xy;
    let tb = textureSampleGrad(t, smp, uv + pstb_hash(b), ddx, ddy).xy;
    let tc = textureSampleGrad(t, smp, uv + pstb_hash(c), ddx, ddy).xy;
    return ta * w.x + tb * w.y + tc * w.z;
}

// The canopy from the seat. Port of ps_CockpitGlass_Forward_0_1_0_3_0, state 0 (the header). Textures: blemish
// pPackedCockpitGlassBlemishAlphaMaps (x the dirt, y the imperfections, stochastically tiled), cracks
// pPackedCracksNormalAlpha (xy a normal, w the crack's alpha), both at UV0 x pCockpitTile.
// - Layers: crack = saturate([crack key > pCracksMaskThreshold] x crack alpha); the dirt and the imperfections are the
//   blemish channels x their keys, x (1 - saturate(pLayerMaskCutoff / 0.95)), with the material's own pLayerMaskCutoff
//   (0.5: the hull's wear override, FUN_142d4faf0, writes the attribute by name into every material of the set it is
//   handed, but which models' sets those are was not traced, so the canopy keeps its file's value); the crack's occlusion saturate(crack x
//   pCracksOcclusionScale). The albedo is mix(the dirt x gCockpitGlassAlphaMultiplier x pDirtDiffuse, pCracksColour x
//   gCockpitGlassAlphaMultiplier, occlusion), the roughness saturate(pCracksRoughness x occlusion + pImperfectionsRoughness
//   x imperfection + pDirtRoughness x dirt), both x (1 - gCockpitGlassTintAlpha); the coverage min(occlusion + dirt +
//   imperfection, 1), raised to gCockpitGlassTintAlpha's floor.
// - Normals: the crack's decoded normal (t, n, b weights x, z, y) mixed by the crack from (0.5, 0.5, 1): as Elite does,
//   so a clean pane's normal leans toward its bitangent. `outer` takes +n, `inner` -n (the stored normal faces the pilot).
// - Light: the star on `inner`, seen along V' = normalize(outer x pCockpitRefractionAmount + the direction to the eye):
//   Lambert on the albedo, Elite's specular with F0 0.04 and the roughness (at least 0.001). No ambient and no lamps: the
//   shader has neither.
fn glass_cockpit(in: GlassOut, blemish: texture_2d<f32>, cracks: texture_2d<f32>, smp: sampler, p: ShipParams, sc: Scene) -> vec4<f32> {
    let f = frame_of(in);
    let uv = in.uv0 * cockpit.glass_tile.xy;
    let blem = blemish_stochastic(blemish, smp, uv, cockpit.glass_tile.z);
    let c = textureSample(cracks, smp, uv);
    let keep = 1.0 - saturate(cockpit.dirt.w * 1.052632);
    let dirt = keep * saturate(blem.x * in.key.y);
    let imperfection = keep * saturate(blem.y * in.key.z);
    let crack = saturate(select(0.0, 1.0, cockpit.cracks_colour.w < in.key.x) * c.w);
    let xy = c.xy * 2.007874 - 0.999985;
    let z = sqrt(max(1.0 - dot(xy, xy), 0.0));
    let w = mix(vec3<f32>(0.5, 0.5, 1.0), vec3<f32>(xy.x, z, xy.y), crack);
    let outer = normalize(w.x * f.t + w.y * f.n + w.z * f.b);
    let inner = normalize(w.x * f.t - w.y * f.n + w.z * f.b);
    let occlusion = saturate(crack * cockpit.occlusion.x);
    let coverage = min(occlusion + dirt + imperfection, 1.0);
    let am = ship_glass.more.x;
    let tint_alpha = ship_glass.effects.w;
    let albedo = mix(dirt * am * cockpit.dirt.rgb, cockpit.cracks_colour.rgb * am, occlusion) * (1.0 - tint_alpha);
    let rough = saturate(cockpit.roughness.x * occlusion + cockpit.roughness.w * imperfection + cockpit.dirt_roughness.x * dirt) * (1.0 - tint_alpha);
    let alpha = tint_alpha * (1.0 - coverage) + coverage;
    let a = max(rough, 0.001);
    let a2 = a * a;
    let toward = normalize(in.world_pos - view.world_position);
    let v = normalize(outer * cockpit.glass_tile.w - toward);
    let f0 = vec3<f32>(0.04);
    // The star.
    let l = sc.sun_dir;
    let nl = saturate(dot(inner, l));
    let nv = saturate(dot(inner, v));
    let colour = albedo * (nl * INV_PI) * sc.sun + specular_brdf(inner, v, l, f0, a2, nv) * nl * sc.sun;
    // (Through the place's haze as the opaque surfaces are, the haze's light for what the glass covers.)
    return vec4<f32>(colour * sc.haze_t + sc.haze_a * alpha, alpha);
}

// The glass's surface as the two GlassSimple shaders work it out: diffuse base x (1 - metalness) x gStandardMapScales.x,
// specular metal_f0 x .y, roughness alpha x .z; the textures n_map pNormalTexture, ma_map pMetalnessAlphaMap (x metalness, y
// coverage), base_map pBaseColour_Roughness, all UV0.
struct GlassSurface {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
    roughness: f32,
    normal: vec3<f32>,
    metal: f32,
    coverage: f32,
    base: vec3<f32>,
};

fn glass_surface(in: GlassOut, n_map: texture_2d<f32>, ma_map: texture_2d<f32>, base_map: texture_2d<f32>, smp: sampler, p: ShipParams) -> GlassSurface {
    let ma = textureSample(ma_map, smp, in.uv0).xy;
    let base = textureSample(base_map, smp, in.uv0);
    var g: GlassSurface;
    g.diffuse = base.rgb * (1.0 - ma.x) * p.map_scales.x;
    g.specular = metal_f0(base.rgb, ma.x) * p.map_scales.y;
    g.roughness = max(base.a * p.map_scales.z, 0.001);
    g.normal = tangent_normal(frame_of(in), textureSample(n_map, smp, in.uv0).xy);
    g.metal = ma.x;
    g.coverage = ma.y;
    g.base = base.rgb;
    return g;
}

// The reflected light: the star's, the lamps' and the sky's specular on a surface of specular colour `f0` (Elite's direct
// loops add no diffuse for these glasses), through `shade` (the sky's part is ours: the header).
fn reflected(g: GlassSurface, f0: vec3<f32>, in: GlassOut, p: ShipParams, sc: Scene) -> vec3<f32> {
    var s: Surface;
    s.diffuse = vec3<f32>(0.0);
    s.specular = f0;
    s.roughness = g.roughness;
    s.normal = g.normal;
    s.emissive = vec3<f32>(0.0);
    s.ao = 1.0;
    s.alpha = 1.0;
    s.flags = 0.0;
    return shade(s, in.world_pos, view.world_position, sc, p.lights.x, p.lights.y);
}

// The canopy from outside. Port of ps_GlassSimple_Tintable_Roughness_Alpha_Forward_0_1_0_3_0: the tinted diffuse
// saturate(diffuse x tint); the specular colour mix(specular, saturate(base x tinted diffuse), max(metalness, t)), so the
// glass turns into a tinted mirror past pDistanceToOpaque; o0 the light it reflects; o1 the transmission, the tinted
// diffuse x (1 - t) x the Fresnel factor 1 + pFresnelVal (saturate(pow(min(|-V.N + bias|, 1), power) x scale) - 1).
fn glass_tinted(in: GlassOut, n_map: texture_2d<f32>, ma_map: texture_2d<f32>, base_map: texture_2d<f32>, smp: sampler, p: ShipParams, sc: Scene) -> vec4<f32> {
    let g = glass_surface(in, n_map, ma_map, base_map, smp, p);
    let tinted = saturate(g.diffuse * in.key.rgb);
    let f0 = mix(g.specular, saturate(g.base * tinted), max(g.metal, in.key.w));
    let v = normalize(view.world_position - in.world_pos);
    let fr = cockpit.fresnel;
    let fresnel = 1.0 + fr.w * (saturate(pow(min(abs(dot(-v, g.normal) + fr.z), 1.0), fr.y) * fr.x) - 1.0);
    let through = tinted * (1.0 - in.key.w) * fresnel;
    // (Grey on the stock glass: the header.)
    let cover = 1.0 - (through.r + through.g + through.b) / 3.0;
    return vec4<f32>(reflected(g, f0, in, p, sc) - sc.haze_a * (1.0 - cover), cover);
}

// The canopy's edges. Port of ps_GlassSimple_Roughness_Alpha_SingleSource_Forward_0_1_0_3_0: the light the surface reflects
// (its specular colour alone), its coverage pMetalnessAlphaMap's y.
fn glass_edges(in: GlassOut, n_map: texture_2d<f32>, ma_map: texture_2d<f32>, base_map: texture_2d<f32>, smp: sampler, p: ShipParams, sc: Scene) -> vec4<f32> {
    let g = glass_surface(in, n_map, ma_map, base_map, smp, p);
    return vec4<f32>(reflected(g, g.specular, in, p, sc) - sc.haze_a * (1.0 - g.coverage), g.coverage);
}
