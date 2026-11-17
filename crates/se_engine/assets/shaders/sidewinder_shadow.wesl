// The Sidewinder's shadow map passes (sidewinder/shadow.rs; the record and the map's frame: sidewinder_shadow_common.wesl; the
// filter that reads the maps: moment_shadow.wesl).
//
// - Each map is made as Elite makes a cascade's (re/elite/notes/sidewinder_shadow.md, section 3; grade C but where marked):
//   1. The casters' depth only (`map_vertex` alone; Elite's `ps_*_Shadow` write depth only) at twice the map's moment size:
//      the mod's own copy of the casters' triangles (shadow.rs `Casters`: a ship-space position, its rig node and, for the
//      seat's chair, its bone), each node at its matrix this frame (`parts`, written in place when the pose changes) and the
//      chair's vertices posed by the same palette the ship's material reads.
//   2. The moments at half that resolution (`resolve_fragment`, Elite's `Pass_MakeHalfResFilterable`): each depth moved
//      away from the star by the resolve depth bias, 1e-4 (C: `c_resolveDepthBias`) of the map's own depth range (E: Elite's
//      is the light's, whose range was not read), kept in 0..1 (ours: the notes do not say what Elite's does; the moments'
//      UNORM target keeps them in 0..1 anyway), and the four 2 x 2 depths' moments averaged into one texel (E: the study's
//      assumption; how Elite combines them was not read).
//   3. The separable Gaussian blur of the moments (`blur_fragment`, Elite's `Pass_SeparableConvolution`): across into the
//      scratch texture, then down back into the map's region, each tap kept inside the region; the kernel and radius are in
//      the pass's record (shadow.rs `gaussian`).
//   The lookups then read them through the B-spline (moment_shadow.wesl), unchanged: the ship's own lighting
//   (sidewinder_lit.wesl `star_shadow`) and the world's receivers (ground_light.wesl `ship_shade_at`), which read the outside
//   set's maps from the engine's texture they are drawn into.
// Every vertex here collapses to one point outside the view (nothing is drawn) while the star gives no light at the ship's
// place (night, an eclipse: `star_lit`, on the light record without the ship's shadow on the world).

import super::hero_frame::{HeroFrame, HeroLight};
import super::sidewinder_shadow_common::{ShipShadow, to_ship_dir, map_point, quantised_moments, star_lit};

@group(0) @binding(0) var<storage, read> ship_shadow: ShipShadow;
// Each rig node's matrix in the ship's space now (a hidden node's is zero: its triangles have no area).
@group(0) @binding(1) var<storage, read> parts: array<mat4x4<f32>>;
// The pilot chair's palette (material.rs `ShipPalette`, `PALETTE_BONES` long).
@group(0) @binding(2) var<storage, read> palette: array<mat4x4<f32>, 142>;
// The sandbox's light records at the player's place, as the ship's material binds them (bindings 0 and 1 there: the light
// without the ship's shadow on the world).
@group(0) @binding(3) var<storage, read> frame: HeroFrame;
@group(0) @binding(4) var<storage, read> light: HeroLight;
// The making of the moments: the casters' depth (the resolve's source), the moments (the blur's), and the passes' records.
@group(0) @binding(10) var depth_src: texture_depth_2d;
@group(0) @binding(11) var moments_src: texture_2d<f32>;
@group(0) @binding(12) var<storage, read> passes: array<MomentPass>;

// shadow.rs `NO_BONE`: a vertex not posed by the palette.
const NO_BONE: u32 = 0xffffffffu;

// Every vertex here: outside the clip volume, all at one point.
const NOWHERE: vec4<f32> = vec4<f32>(2.0, 2.0, 2.0, 1.0);

// One pass of the making of the moments (shadow.rs `MomentPass`).
struct MomentPass {
    // xy where the source's square starts (texels), z the map's moment size, w 0 the resolve, 1 the blur across, 2 down.
    src: vec4<f32>,
    // xy where the target's square starts (texels), z the blur's radius.
    dst: vec4<f32>,
    // The blur's weights at offsets 0, 1 and 2.
    kernel: vec4<f32>,
};

struct MapOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn map_vertex(@location(0) position: vec3<f32>, @location(1) part: u32, @location(2) bone: u32) -> MapOut {
    var o: MapOut;
    if !star_lit(light.sun.rgb) {
        o.clip = NOWHERE;
        return o;
    }
    var p = vec4<f32>(position, 1.0);
    if bone != NO_BONE {
        p = palette[bone] * p;
    }
    let m = map_point(ship_shadow, (parts[part] * p).xyz, to_ship_dir(ship_shadow, frame.light.sun_dir.xyz));
    o.clip = vec4<f32>(m.xy, m.z, 1.0);
    return o;
}

struct PassOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) pass_index: u32,
};

// One triangle over the pass's square (its viewport); the instance is the pass's record. Nothing while the star gives no
// light at the ship's place: the maps then keep the last lit frame's moments, which nothing reads but times a star term of 0
// (the ship's own pixels and the world's receivers, whose star term there is 0 too); the first lit frame makes them anew
// before anything reads them.
@vertex
fn pass_vertex(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PassOut {
    var o: PassOut;
    o.pass_index = ii;
    if !star_lit(light.sun.rgb) {
        o.clip = NOWHERE;
        return o;
    }
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    o.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

// Step 2: the moments of a texel's four depths, averaged.
@fragment
fn resolve_fragment(in: PassOut) -> @location(0) vec4<f32> {
    let job = passes[in.pass_index];
    let i = vec2<i32>(in.clip.xy) - vec2<i32>(job.dst.xy);
    let base = vec2<i32>(job.src.xy) + 2 * i;
    var q = vec4<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let d = textureLoad(depth_src, base + vec2<i32>(k & 1, k / 2), 0);
        q += quantised_moments(ship_shadow, clamp(d + ship_shadow.msm.w, 0.0, 1.0));
    }
    return q * 0.25;
}

// Step 3: one direction of the separable Gaussian, its taps kept inside the map's square.
@fragment
fn blur_fragment(in: PassOut) -> @location(0) vec4<f32> {
    let job = passes[in.pass_index];
    let i = vec2<i32>(in.clip.xy) - vec2<i32>(job.dst.xy);
    let n = i32(job.src.z);
    let step = select(vec2<i32>(1, 0), vec2<i32>(0, 1), job.src.w > 1.5);
    let r = i32(job.dst.z);
    var q = vec4<f32>(0.0);
    for (var k = -r; k <= r; k++) {
        let at = clamp(i + step * k, vec2<i32>(0), vec2<i32>(n - 1));
        q += job.kernel[abs(k)] * textureLoad(moments_src, vec2<i32>(job.src.xy) + at, 0);
    }
    return q;
}
