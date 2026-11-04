// Volumetric clouds (volumetric_clouds.rs).
//
// The active body's cover map, built once per body: a cube (SE's tile faces, y the pole) holding in
// each channel one cloud layer's cover in that layer's frame: `cover_gen` per face (SE's terra
// clouds, the port's procedural clouds, or a catalog cloud map), `cover_down` its mips.
//
// Then four full-screen passes per frame over the solids' corner of the main texture, after the main
// pass and before the solids' upscale:
//   depth_down: the scene's distance at the cloud resolution (each block's centre, for the
//               depth-aware upscale),
//   trace:      the ray-march through the layers up to the scene, lit by the star through the
//               atmosphere and the sky, then the haze between the camera and the clouds,
//   resolve:    blended with the reprojected previous result, clamped to the new neighbourhood,
//   composite:  upscaled by depth and blended over the scene,
//   weather_fx: falling rain or snow around the camera and lightning's light on the ground, at full
//               resolution over the scene.
// The trace also draws the weather in the air (rain and snow shafts under the clouds, fog, dust) and
// lightning glowing inside the clouds (weather_sim.wesl's fog and dust cube and flashes).
// Light is premultiplied: rgb the light the clouds add, a the transmittance of what lies behind.
// Positions in "sphere space" are the body frame divided by the ellipsoid's semi-axes (m from the
// centre), so a flattened planet's layers are spheres there.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::atmosphere_common::{AtmParams, ATM_AMBIENT_SCALE, atm_transmittance_sun, atm_irradiance, atm_path};
import super::cloud_cover::{se_terra_clouds, proc_clouds, weather_cover, weather_shaped, fbm_aa, climb_dist};
import super::gas_giant::{GiantParams, gas_giant_cloud};

const PI: f32 = 3.14159265358979;
const FAR: f32 = 1.0e30;
const MAX_STEPS: i32 = 112;
// The near pass's steps (its rays are at most `CloudView::lod.z` long).
const NEAR_STEPS: f32 = 32.0;
const LIGHT_STEPS: i32 = 5;
const LAYERS: i32 = 4;
// volumetric_clouds/detail.rs `DETAIL_SIZE`, `DETAIL_LEVELS`.
const DETAIL_SIZE: f32 = 2048.0;
const DETAIL_LEVELS: i32 = 3;

struct CloudView {
    // Camera-relative world (universe axes, the camera at the origin) from clip.
    world_from_clip: mat4x4<f32>,
    // The previous frame's clip from this frame's camera-relative world, through the body's frame.
    prev_clip_from_world: mat4x4<f32>,
    // Sphere space from world (upper 3x3).
    sphere_from_world: mat4x4<f32>,
    // xyz: the camera in sphere space (m), w: the body's reference radius (m).
    cam: vec4<f32>,
    // x: |cam| (m), y: the camera's altitude in sphere space (m), z: lowest layer bottom, w: 1 to drop
    // the history.
    cam2: vec4<f32>,
    // Per layer: x bottom, y top (m above the radius), z opacity (0: no layer), w how cumuliform (0..1).
    layer_a: array<vec4<f32>, 4>,
    // Per layer: x, y cos and sin of the layer's turn about the pole, z extinction per m, w unused.
    layer_b: array<vec4<f32>, 4>,
    // Per layer: rgb albedo, w SE's DayAmbient.
    layer_c: array<vec4<f32>, 4>,
    // xyz: towards the star (world), w: 1 when lit.
    sun_dir: vec4<f32>,
    // rgb: the star's light above the atmosphere, in the tables' scale.
    sun_light: vec4<f32>,
    // x: 1 when drawn, y: shape noise frequency (1/m), z: detail noise frequency (1/m), w: divisor.
    params: vec4<f32>,
    // x: atmosphere on, y: the haze's light relative to the sun's, z: the cover map's texel (m), w:
    // highest layer top.
    atm_flags: vec4<f32>,
    // The ground's shadow map: xyz its centre in sphere space (m), w its half-width (m); its axes.
    shadow_c: vec4<f32>,
    shadow_e1: vec4<f32>,
    shadow_e2: vec4<f32>,
    // xyz: the shape and the detail noise's drift with the wind (sphere space, m, wrapped to their
    // periods).
    noise_off: vec4<f32>,
    detail_off: vec4<f32>,
    // xyz: the clouds' drift since the last frame (world, m).
    motion: vec4<f32>,
    // The weather effects: x rain and snow, y lightning, z fog, w dust (1 on).
    fx: vec4<f32>,
    // x: the effects' clock (s), y: the body's mean temperature (K), z: lightning's brightness, w: 1
    // to show the weather map.
    fx2: vec4<f32>,
    // xyz: the wind at the camera (sphere space, m/s).
    wind: vec4<f32>,
    // A gas giant's decks: x 1 for a giant, y their density (fading in as the camera comes down), z
    // the scale height (m).
    giant: vec4<f32>,
    // The far cross-fade (volumetric_clouds.rs, 40 to 60 radii): x the clouds' share (the flat shells
    // draw the rest), y the weather in the air's; z, w the near pass's range and divisor.
    lod: vec4<f32>,
    // Each layer's detail cover patches (volumetric_clouds/detail.rs, DETAIL_LEVELS a layer, finest
    // first): xyz the centre in the layer's frame, w the tangent of the half-width (0: none).
    detail: array<vec4<f32>, 12>,
    atm: AtmParams,
};

struct CloudSize {
    // xy: the solids' size (px), zw: the cloud buffers' used size.
    size: vec4<f32>,
    // x: divisor (the near pass 1 or 2, the rest 2 or 4), y: 1 to drop the history, z: frame number,
    // w: the part of the ray traced (0 all, 1 beyond the near pass's range, 2 within it).
    params: vec4<f32>,
    // The trace buffer: xy its used size, z this frame's phase (0..3), w 1 when it holds a quarter of
    // the cloud texels (one of each 2 x 2 block, taking turns), 0 when it holds them all.
    trace: vec4<f32>,
};

struct CoverLayer {
    // x kind (0 none, 1 catalog map, 2 SE's terra clouds, 3 the port's procedural clouds, 4 overcast),
    // y the mapped cover's channel, z coverage.
    kind: vec4<f32>,
    se: array<vec4<f32>, 3>,
    proc: vec4<f32>,
    proc2: vec4<f32>,
};

struct CoverGen {
    layers: array<CoverLayer, 4>,
    // xyz: the star in the layers' frame (tidally locked worlds' substellar storm), w: the body's seed
    // (0..1, the weather's cloud pattern).
    sun: vec4<f32>,
    // A gas giant's SE preset (gas_giant.wesl `GiantParams`).
    giant_palette: array<vec4<f32>, 8>,
    giant: array<vec4<f32>, 5>,
};

@group(0) @binding(0) var<uniform> cv: CloudView;
@group(0) @binding(11) var<uniform> cs: CloudSize;
@if(MSAA) @group(0) @binding(1) var scene_depth: texture_depth_multisampled_2d;
@if(NO_MSAA) @group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var low_depth: texture_2d<f32>;
@group(0) @binding(3) var shape_tex: texture_3d<f32>;
@group(0) @binding(4) var detail_tex: texture_3d<f32>;
@group(0) @binding(5) var repeat_smp: sampler;
@group(0) @binding(6) var trace_tex: texture_2d<f32>;
@group(0) @binding(7) var trace_dist_tex: texture_2d<f32>;
@group(0) @binding(8) var history_tex: texture_2d<f32>;
@group(0) @binding(9) var clamp_smp: sampler;
@group(0) @binding(10) var resolved_tex: texture_2d<f32>;
@group(0) @binding(12) var inscatter_tex: texture_3d<f32>;
@group(0) @binding(13) var transmittance_tex: texture_2d<f32>;
@group(0) @binding(14) var irradiance_tex: texture_2d<f32>;
@group(0) @binding(15) var cover_tex: texture_cube<f32>;
// The body's weather cube (weather_sim.wesl `sim_out`, in the body frame on SE's tile faces): R the
// cover multiplier / 2, G the cover offset x 0.5 + 0.5, B convection, A precipitation.
@group(0) @binding(16) var weather_tex: texture_cube<f32>;
// The fog and dust cube (weather_sim.wesl `sim_out`): R fog, G dust.
@group(0) @binding(17) var fx_tex: texture_cube<f32>;
// The lightning flashes (weather_sim.wesl `sim_flash`): xyz where (sphere space, m), w brightness.
@group(0) @binding(18) var<storage, read> flashes: array<vec4<f32>, 8>;
// The weather's own cloud's pattern (`weather_pattern`), on SE's tile faces in the body frame.
@group(0) @binding(27) var pattern_tex: texture_cube<f32>;
// The light volume (volumetric_clouds.rs `LightVolume`; blackrack's design, docs/ksp-clouds-notes.md
// section 3): a grid over the cloud layers round the spot under the camera, its texels along the ground
// a gnomonic map about its centre (finer near it, `light_warp`), up the layers' height (finer near their
// bottom, where the heaps are); R the star's optical depth from each point to the clouds' top, G the
// cloud's straight up (the sky light's). `light_gen` fills a slice of the one being built; the march
// reads the other (`light_at`).
struct LightFrame {
    // xyz the unit direction of its centre (sphere space), w its half-width (m).
    c: vec4<f32>,
    e1: vec4<f32>,
    e2: vec4<f32>,
    // x, y the heights of its bottom and top (m above the radius), z 1 when it holds light, w the radius.
    h: vec4<f32>,
    // The star's direction (sphere space) it was built for.
    sun: vec4<f32>,
};
struct LightGen {
    frame: LightFrame,
    // x the slice, y the texels a side, z the slices.
    slice: vec4<f32>,
};
@group(0) @binding(28) var light_tex: texture_3d<f32>;
@group(0) @binding(29) var<uniform> lv: LightFrame;
@group(0) @binding(30) var<uniform> lgen: LightGen;
// The detail cover patches, a layer each (R: the cover after SE's 5% cut).
@group(0) @binding(19) var detail_cover: texture_2d_array<f32>;
// The cover map's passes.
@group(0) @binding(20) var<uniform> cg: CoverGen;
@group(0) @binding(21) var mapped_tex: texture_2d_array<f32>;
// x: the face, y: its size (texels).
@group(0) @binding(23) var<uniform> cover_face: vec4<f32>;
@group(0) @binding(24) var cover_src: texture_2d<f32>;
// The detail cover's generator pass: xyz the patch's centre, w the tangent of its half-width; the
// layer (info.x) and the patch's texels a side (info.y).
struct DetailGen {
    frame: vec4<f32>,
    info: vec4<f32>,
};
@group(0) @binding(25) var<uniform> dg: DetailGen;
// The resolve's cover optical depth per cloud texel (`Resolved::tau`), for the composite.
@group(0) @binding(26) var tau_tex: texture_2d<f32>;

// ---------------------------------------------------------------------------------------------
// The cover map.

// SE's planet tile faces (se_core planet_tiles `uv_to_cube`, the cube-map convention): u right, v down.
fn face_dir(face: i32, u: f32, v: f32) -> vec3<f32> {
    let s = 2.0 * u - 1.0;
    let t = 2.0 * v - 1.0;
    switch face {
        case 0: { return vec3<f32>(1.0, -t, -s); }
        case 1: { return vec3<f32>(-1.0, -t, s); }
        case 2: { return vec3<f32>(s, 1.0, t); }
        case 3: { return vec3<f32>(s, -1.0, -t); }
        case 4: { return vec3<f32>(s, -t, 1.0); }
        default: { return vec3<f32>(-s, -t, -1.0); }
    }
}

// The generators' cover averaged over 2 x 2 points of the texel: their wisps are far finer than a
// texel, and the flat shells see them as a pixel-wide average, a partial cover.
const COVER_SUB: i32 = 2;

// The weather's own cloud's pattern: warped fBm from about a thousand km across down to the texels
// (Earth), stretched along the parallels, with a finer one over it (ragged edges), about 0.5 on
// average; `fw` the texel's angle.
fn weather_pattern(d: vec3<f32>, fw: f32) -> f32 {
    let freq = 6.0;
    let seed = vec3<f32>(71.0, 13.0, 37.0) * cg.sun.w + vec3<f32>(3.1, 7.7, 1.3);
    let ps = vec3<f32>(d.x, d.y * 1.7, d.z);
    let f = fw * freq;
    let warp = vec3<f32>(
        fbm_aa(ps * freq * 0.5 + seed, 4, f * 0.5),
        fbm_aa(ps * freq * 0.5 + seed + vec3<f32>(5.2), 4, f * 0.5),
        fbm_aa(ps * freq * 0.5 + seed + vec3<f32>(9.1), 4, f * 0.5)) - vec3<f32>(0.5);
    let coarse = fbm_aa(ps * freq + warp * 2.2 + seed, 10, f);
    let fine = fbm_aa(ps * freq * 6.0 + warp * 9.0 + seed.zxy, 7, f * 6.0);
    return coarse * 0.65 + fine * 0.35;
}

struct CoverOut {
    @location(0) cover: vec4<f32>,
    @location(1) pattern: vec4<f32>,
};

@fragment
fn cover_gen(in: FullscreenVertexOutput) -> CoverOut {
    var out: CoverOut;
    let face = i32(cover_face.x);
    let texel = 1.0 / cover_face.y;
    out.pattern = vec4<f32>(weather_pattern(normalize(face_dir(face, in.uv.x, in.uv.y)), 0.5 * PI * texel), 0.0, 0.0, 1.0);
    // A gas giant (its main deck, kind 5): SE's band colour (rgb, as its square root) and cloud height
    // (a, a third of it), one sample a texel (the generator is costly and its bands smooth).
    if i32(cg.layers[0].kind.x + 0.5) == 5 {
        let gc = gas_giant_cloud(normalize(face_dir(face, in.uv.x, in.uv.y)), GiantParams(cg.giant_palette, cg.giant));
        out.cover = vec4<f32>(sqrt(gc.rgb), clamp(gc.a / 3.0, 0.0, 1.0));
        return out;
    }
    // A sub-sample's angular size.
    let fp = 0.5 * PI * texel / f32(COVER_SUB);
    out.cover = vec4<f32>(0.0);
    for (var i = 0; i < LAYERS; i++) {
        let l = cg.layers[i];
        let kind = i32(l.kind.x + 0.5);
        var c = 0.0;
        if kind == 1 {
            let m = textureSampleLevel(mapped_tex, clamp_smp, in.uv, face, 0.0);
            c = m[i32(l.kind.y + 0.5)];
        } else if kind == 2 || kind == 3 {
            for (var k = 0; k < COVER_SUB * COVER_SUB; k++) {
                let o = (vec2<f32>(f32(k % COVER_SUB), f32(k / COVER_SUB)) + 0.5) / f32(COVER_SUB) - 0.5;
                let uv = in.uv + o * texel;
                let d = normalize(face_dir(face, uv.x, uv.y));
                if kind == 2 {
                    c += se_terra_clouds(d, l.kind.z, fp, cg.sun.xyz, l.se);
                } else {
                    c += proc_clouds(d, fp, l.proc, l.proc2);
                }
            }
            c /= f32(COVER_SUB * COVER_SUB);
        } else if kind == 4 {
            c = 1.0;
        }
        // SE drops the faintest 5% and rescales the rest (`cloudsFade`, as the flat shells).
        out.cover[i] = clamp((c - 0.05) / 0.95, 0.0, 1.0);
    }
    return out;
}

// A detail patch's axes around its centre c (as detail.rs `frame_axes`).
fn detail_e1(c: vec3<f32>) -> vec3<f32> {
    let a = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(c.y) > 0.99);
    return normalize(cross(a, c));
}

// A layer's detail patch from its generator: 2 x 2 points a texel, after SE's 5% cut.
@fragment
fn cover_patch(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = dg.frame.xyz;
    let e1 = detail_e1(c);
    let e2 = cross(c, e1);
    let l = cg.layers[i32(dg.info.x + 0.5)];
    let kind = i32(l.kind.x + 0.5);
    let texel = 2.0 / dg.info.y;
    let fp = dg.frame.w * texel * 0.5;
    var acc = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = (vec2<f32>(f32(k % 2), f32(k / 2)) - 0.5) * 0.5 * texel;
        let uv = in.uv * 2.0 - 1.0 + o;
        let d = normalize(c + (uv.x * e1 + uv.y * e2) * dg.frame.w);
        if kind == 2 {
            acc += se_terra_clouds(d, l.kind.z, fp, cg.sun.xyz, l.se);
        } else {
            acc += proc_clouds(d, fp, l.proc, l.proc2);
        }
    }
    return vec4<f32>(clamp((acc * 0.25 - 0.05) / 0.95, 0.0, 1.0), 0.0, 0.0, 1.0);
}

@fragment
fn cover_down(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy) * 2;
    let a = textureLoad(cover_src, p, 0) + textureLoad(cover_src, p + vec2<i32>(1, 0), 0);
    let b = textureLoad(cover_src, p + vec2<i32>(0, 1), 0) + textureLoad(cover_src, p + vec2<i32>(1, 1), 0);
    return (a + b) * 0.25;
}

// ---------------------------------------------------------------------------------------------
// The frame's passes.

fn divisor() -> i32 {
    return i32(cs.params.x);
}

// The solids' uv of a full-resolution pixel centre.
fn solid_uv(p: vec2<f32>) -> vec2<f32> {
    return p / cs.size.xy;
}

// The centre of a cloud texel's block, in the solids' uv.
fn cloud_uv(lp: vec2<i32>) -> vec2<f32> {
    return (vec2<f32>(lp) + 0.5) * f32(divisor()) / cs.size.xy;
}

fn quarter() -> bool {
    return cs.trace.w > 0.5;
}

// This frame's texel of each 2 x 2 block: (0, 0), (1, 1), (1, 0), (0, 1) in turn.
fn phase_offset() -> vec2<i32> {
    let k = i32(cs.trace.z + 0.5);
    return vec2<i32>(select(0, 1, k == 1 || k == 2), select(0, 1, k == 1 || k == 3));
}

// The cloud texel a trace texel stands for.
fn traced_texel(tp: vec2<i32>) -> vec2<i32> {
    if !quarter() {
        return tp;
    }
    return min(tp * 2 + phase_offset(), vec2<i32>(cs.size.zw) - 1);
}

fn ray_dir(uv: vec2<f32>) -> vec3<f32> {
    let p = cv.world_from_clip * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0);
    return normalize(p.xyz / p.w);
}

// The distance (m) to the scene at a reversed-depth value (0: nothing drawn, infinitely far).
fn scene_dist(uv: vec2<f32>, d: f32) -> f32 {
    if d <= 0.0 {
        return FAR;
    }
    let p = cv.world_from_clip * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return length(p.xyz / p.w);
}

fn depth_at(p: vec2<i32>) -> f32 {
    return textureLoad(scene_depth, p, 0);
}

@fragment
fn depth_down(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let lp = vec2<i32>(in.position.xy);
    let div = divisor();
    let p = min(lp * div + vec2<i32>(div / 2), vec2<i32>(cs.size.xy) - 1);
    return vec4<f32>(scene_dist(solid_uv(vec2<f32>(p) + 0.5), depth_at(p)), 0.0, 0.0, 0.0);
}

// Interleaved gradient noise (Jimenez), moved on each frame: the ray's start jitter.
fn jitter(p: vec2<f32>) -> f32 {
    let q = p + 5.588238 * (cs.params.z % 64.0);
    return fract(52.9829189 * fract(dot(q, vec2<f32>(0.06711056, 0.00583715))));
}

// Where the ray o + t d (o in sphere space at radius |o| = cam2.x) enters and leaves the sphere of
// radius R + h; (1, -1) when it misses. The constant term comes from the altitude computed in double
// precision on the CPU, so it stays exact near the camera's own layer.
fn sphere_hits(h: f32, a: f32, b: f32) -> vec2<f32> {
    let big_r = cv.cam.w;
    let c = (cv.cam2.y - h) * (cv.cam2.x + big_r + h);
    let disc = b * b - a * c;
    if disc < 0.0 {
        return vec2<f32>(1.0, -1.0);
    }
    let q = -(b + select(-1.0, 1.0, b >= 0.0) * sqrt(disc));
    if abs(q) < 1e-20 {
        return vec2<f32>(1.0, -1.0);
    }
    let t1 = q / a;
    let t2 = c / q;
    return vec2<f32>(min(t1, t2), max(t1, t2));
}

fn remap(v: f32, lo: f32, hi: f32, nlo: f32, nhi: f32) -> f32 {
    return nlo + (v - lo) * (nhi - nlo) / max(hi - lo, 1e-6);
}

// The mip level of a volume of `n` texels repeating at `freq` (1/m) for a footprint of `fp` m.
fn lod_for(fp: f32, freq: f32, n: f32) -> f32 {
    return clamp(log2(max(fp * freq * n, 1e-6)), 0.0, log2(n));
}

// The weather at the sphere-space point x (the body frame): x the cover multiplier, y the cover
// offset, z convection, w precipitation.
fn weather_at(x: vec3<f32>) -> vec4<f32> {
    let w = textureSampleLevel(weather_tex, clamp_smp, x, 0.0);
    return vec4<f32>(w.r * 2.0, w.g * 2.0 - 1.0, w.b, w.a);
}

// Layer i's cover at the sphere-space point x (in the layer's frame, turned by its wind).
fn cover_at(i: i32, x: vec3<f32>, fp: f32) -> f32 {
    let cs2 = cv.layer_b[i].xy;
    let d = vec3<f32>(cs2.x * x.x - cs2.y * x.z, x.y, cs2.y * x.x + cs2.x * x.z);
    // Around the camera its detail patches (Earth: texels from 400 m to 1.6 km), the finest that holds
    // x first, each blending into the next towards its edge and as the footprint outgrows its texels,
    // the cover map last.
    var c = 0.0;
    var rest = 1.0;
    let dn = normalize(d);
    for (var k = 0; k < DETAIL_LEVELS; k++) {
        let n = i * DETAIL_LEVELS + k;
        let dp = cv.detail[n];
        if dp.w <= 0.0 {
            continue;
        }
        let kd = dot(dn, dp.xyz);
        if kd < 0.5 {
            continue;
        }
        let e1 = detail_e1(dp.xyz);
        let p = dn / kd - dp.xyz;
        let uv = vec2<f32>(dot(p, e1), dot(p, cross(dp.xyz, e1))) / dp.w;
        let texel_m = 2.0 * dp.w * cv.cam.w / DETAIL_SIZE;
        let w = smoothstep(1.0, 0.8, max(abs(uv.x), abs(uv.y))) * (1.0 - smoothstep(1.0, 2.0, fp / texel_m));
        if w > 0.0 {
            c += rest * w * textureSampleLevel(detail_cover, clamp_smp, uv * 0.5 + 0.5, n, 0.0).r;
            rest *= 1.0 - w;
            if rest <= 0.0 {
                return c;
            }
        }
    }
    let lod = clamp(log2(max(fp / cv.atm_flags.z, 1e-6)), 0.0, 11.0);
    return c + rest * textureSampleLevel(cover_tex, clamp_smp, d, lod)[i];
}

// Layer i's cover over the region round x (about 20 km on Earth: the cover map's mip 2) under the weather wx,
// which picks its cloud types: SE's maps fill each cumulus field's blobs with a cover near 1, which the
// local cover would take for an overcast.
fn regional_cover(i: i32, x: vec3<f32>, wx: vec4<f32>) -> f32 {
    // (A stratiform layer's clouds are all veils, whatever the cover.)
    if cv.layer_a[i].w <= 0.0 {
        return 0.0;
    }
    let cs2 = cv.layer_b[i].xy;
    let d = vec3<f32>(cs2.x * x.x - cs2.y * x.z, x.y, cs2.y * x.x + cs2.x * x.z);
    return weather_cover(textureSampleLevel(cover_tex, clamp_smp, d, 2.0)[i], wx.x, wx.y, cv.layer_a[i].z);
}

// The vertical profile of a layer at height fraction hf: cumuliform clouds round at the base and
// taper to the top, stratiform ones fill the layer with soft edges.
fn profile(hf: f32, cumulus: f32) -> f32 {
    let strat = smoothstep(0.0, 0.2, hf) * smoothstep(1.0, 0.7, hf);
    let cumu = smoothstep(0.0, 0.08, hf) * smoothstep(1.0, 0.35, hf);
    return mix(strat, cumu, cumulus);
}

// The cloud types (blackrack's, docs/ksp-clouds-notes.md section 1, and Guerrilla's Nubis): 0 fair-weather
// cumulus (separate heaps), 1 stratocumulus (heaps merged into a lumpy sheet with gaps), 2 an overcast deck
// (flat base, lumpy top), 3 towers (through the layer, with an anvil) and 4 veils (the stratiform share).
// The solid types (0-3) are fields of heaps (`heaps`): a coarse noise read on the layer's base, spread
// evenly, picks where the heaps stand (the share of the area the type's field covers under the cover) and
// each point's strength (0 at a heap's edge, 1 at its core); the cloud reaches up to the type's top for
// that strength (`type_top`: domes rising steeply from the edge and rounding over the core) from a flat
// base, its surface lumped by a finer noise and billowed by the erosion. (A 3D noise thresholded by the
// cover filled each of SE's cover blobs, near 1 inside, from the ground up: slabs with flat sides in the
// blob's outline.) Each type has a depth above the layer's bottom, a mean density curve (the share of its
// heaps reaching a height: the far clouds' mean and the light from the cloud above) and an erosion curve,
// small at the base and strong higher up, where the detail noise turns billowy.
const TYPES: i32 = 5;

// Type t's depth (m) in a layer `deep` m deep: fair-weather cumulus up to 1.8 km tall (physical sizes:
// blackrack's are the same on his small planets), towering cumulus up to 4 km, veils through the whole
// layer. (Storm cells through the whole layer are the clouds plan's step 7: on heaps a kilometre wide
// the towers stood as spires.)
fn type_depth(t: i32, deep: f32) -> f32 {
    switch t {
        case 0: { return min(1800.0, deep); }
        case 1: { return min(1000.0, deep); }
        case 2: { return min(900.0, deep); }
        case 3: { return min(4000.0, deep); }
        default: { return deep; }
    }
}

// The share of the area type t's heaps cover under a cover of 1: fair-weather cumulus and towers keep gaps
// between them, a deck closes.
fn type_field(t: i32) -> f32 {
    switch t {
        case 0: { return 0.9; }
        case 1: { return 0.95; }
        case 3: { return 0.7; }
        default: { return 1.0; }
    }
}

// The height type t's heaps reach (a fraction of its depth) in a field covering a share f of the area
// under convection conv: fair-weather cumulus about 0.6 times as tall as they are wide (their width the
// heap noise's 1.5 km cells times about the root of f; the full 1.8 km for all of them made spires),
// taller with convection.
fn type_height(t: i32, f: f32, conv: f32) -> f32 {
    switch t {
        case 0: { return clamp(0.6 * sqrt(f) + 0.5 * conv, 0.15, 1.0); }
        case 3: { return clamp(0.9 * sqrt(f) + 0.5 * conv, 0.2, 1.0); }
        default: { return 1.0; }
    }
}

// The top of type t's cloud (a fraction of its height) over a point of a heap of strength s: with s even
// over a heap's area (1 - s grows with the square of the distance from its core), the root is a round
// dome with steep sides.
fn type_top(t: i32, s: f32) -> f32 {
    let q = max(s, 1e-6);
    switch t {
        case 0: { return sqrt(q); }
        case 1: { return 0.35 + 0.65 * pow(q, 0.6); }
        case 2: { return 0.7 + 0.3 * q; }
        default: { return sqrt(q); }
    }
}

// The towers' anvil: a sheet over the whole field near their top.
fn type_anvil(t: i32, hf: f32) -> f32 {
    if t != 3 {
        return 0.0;
    }
    return 0.6 * smoothstep(0.74, 0.82, hf) * (1.0 - smoothstep(0.9, 0.97, hf));
}

// Type t's mean density at the fraction hf of its height (`type_height`): the share of its heaps reaching
// above hf (the strength spread evenly over a heap: `type_top` inverted) over the flat base; a veil's soft
// profile.
fn type_density(t: i32, hf: f32) -> f32 {
    if hf <= 0.0 || hf >= 1.0 {
        return 0.0;
    }
    let base = smoothstep(0.0, 0.04, hf);
    switch t {
        case 0: { return base * (1.0 - hf * hf); }
        case 1: { return base * (1.0 - pow(max(hf - 0.35, 0.0) / 0.65, 1.0 / 0.6)); }
        case 2: { return base * (1.0 - max(hf - 0.7, 0.0) / 0.3); }
        case 3: { return base * max(1.0 - hf * hf, type_anvil(3, hf)); }
        default: { return smoothstep(0.0, 0.2, hf) * smoothstep(1.0, 0.7, hf); }
    }
}

fn type_erosion(t: i32, hf: f32) -> f32 {
    switch t {
        case 0: { return mix(0.15, 0.75, smoothstep(0.02, 0.2, hf)); }
        case 1: { return mix(0.1, 0.35, smoothstep(0.1, 0.6, hf)); }
        case 2: { return mix(0.06, 0.25, smoothstep(0.3, 1.0, hf)); }
        case 3: { return mix(0.1, 0.35, smoothstep(0.05, 0.5, hf)); }
        default: { return 0.15 * (1.0 - 0.5 * hf); }
    }
}

// The types' weights for a layer whose clouds are `cumulus` cumuliform, under a regional cover
// (`regional_cover`) and convection:
// towers where there is convection, else cumulus under a thin cover, stratocumulus under a broken one, a
// deck under an overcast; veils for the stratiform share.
fn type_weights(cumulus: f32, cover: f32, conv: f32) -> array<f32, 5> {
    let towers = cumulus * conv;
    let rest = cumulus - towers;
    let sc = smoothstep(0.45, 0.75, cover);
    let ov = smoothstep(0.8, 0.97, cover);
    return array<f32, 5>(rest * (1.0 - sc), rest * sc * (1.0 - ov), rest * ov, towers, 1.0 - cumulus);
}

// The types blended at a height above a layer's bottom.
struct Shape {
    // The mean density curve, the heaps' share of the area under a cover of 1, the erosion, the types'
    // mean depth (m), their heaps' mean height (m) and the height fraction in the depth.
    d: f32,
    f: f32,
    md: f32,
    e: f32,
    depth: f32,
    hf: f32,
};

// (The types' weights from the regional cover rc, their heaps' height from the local cover.)
fn layer_shape(a: vec4<f32>, rc: f32, cover: f32, conv: f32, h: f32) -> Shape {
    var s: Shape;
    s.d = 0.0;
    s.f = 0.0;
    s.md = 0.0;
    s.e = 0.0;
    s.depth = 0.0;
    var w = type_weights(a.w, rc, conv);
    let deep = a.y - a.x;
    for (var t = 0; t < TYPES; t++) {
        if w[t] <= 0.0 {
            continue;
        }
        let dt = type_depth(t, deep);
        let hf = (h - a.x) / dt;
        let hh = type_height(t, type_field(t) * cover, conv);
        s.d += w[t] * type_density(t, hf / hh);
        s.f += w[t] * type_field(t);
        s.md += w[t] * dt * hh;
        s.e += w[t] * type_erosion(t, clamp(hf, 0.0, 1.0));
        s.depth += w[t] * dt;
    }
    s.hf = clamp((h - a.x) / max(s.depth, 1.0), 0.0, 1.0);
    return s;
}

// The solid types' cloud (0..1) at height h over a point whose heap noise, spread evenly, is u, under a
// cover and convection conv, the heaps' strength moved by `lump` (a 3D noise: bulging sides and tops):
// inside below the types' top, a 350 m soft skin under its surface (the erosion's room: the billows), the
// base flat. The types blend their tops and strengths by their weights w, so each point has one surface.
// (Blending their clouds instead, a 1 % share of towers stood faint 8 km ghosts over every heap.) The
// towers' anvil only where they are most of the mix.
fn heaps(a: vec4<f32>, w: array<f32, 5>, cover: f32, conv: f32, u: f32, h: f32, lump: f32) -> f32 {
    let deep = a.y - a.x;
    let z = h - a.x;
    var top = 0.0;
    var side = 0.0;
    var ws = 0.0;
    for (var t = 0; t < 4; t++) {
        if w[t] <= 0.0 {
            continue;
        }
        ws += w[t];
        let f = max(type_field(t) * cover, 1e-3);
        let s = clamp((u + f - 1.0) / f + lump, 0.0, 1.0);
        top += w[t] * type_top(t, s) * type_height(t, f, conv) * type_depth(t, deep) * smoothstep(0.0, 0.03, s);
        side += w[t] * s;
    }
    top /= max(ws, 1e-6);
    side /= max(ws, 1e-6);
    // The depth under the surface: below the top, and in from the side (the outer half of a heap's
    // strength). (Below the top alone, the erosion never reached the sides: smooth walls.)
    var m = clamp(min((top - z) / 350.0, 2.0 * side), 0.0, 1.0);
    if w[3] > 0.2 * ws {
        m = max(m, type_anvil(3, z / type_depth(3, deep)) * smoothstep(0.0, 0.05, side) * smoothstep(0.2, 0.5, w[3] / ws));
    }
    return m * smoothstep(0.0, 70.0, z);
}

// The highest the solid types' heaps reach (m above a layer's bottom) under the weights w, a cover and
// convection conv (`heaps`'s top at full strength; with an anvil, the towers' top).
fn heaps_ceiling(a: vec4<f32>, w: array<f32, 5>, cover: f32, conv: f32) -> f32 {
    let deep = a.y - a.x;
    var top = 0.0;
    var ws = 0.0;
    for (var t = 0; t < 4; t++) {
        if w[t] <= 0.0 {
            continue;
        }
        ws += w[t];
        top += w[t] * type_height(t, max(type_field(t) * cover, 1e-3), conv) * type_depth(t, deep);
    }
    return select(top / max(ws, 1e-6), type_depth(3, deep), w[3] > 0.2 * ws);
}

// The share of layer a's cloud (its types' density curves' integral, Simpson's rule) above height h, and
// the heaps' share of the area (`Shape::f`).
fn shape_above(a: vec4<f32>, rc: f32, cover: f32, conv: f32, h: f32) -> vec2<f32> {
    var w = type_weights(a.w, rc, conv);
    let deep = a.y - a.x;
    var above = 0.0;
    var total = 0.0;
    var f = 0.0;
    for (var t = 0; t < TYPES; t++) {
        if w[t] <= 0.0 {
            continue;
        }
        let dt = type_depth(t, deep) * type_height(t, type_field(t) * cover, conv);
        let hf = clamp((h - a.x) / dt, 0.0, 1.0);
        let m = 0.5 * (hf + 1.0);
        above += w[t] * dt * (1.0 - hf) * (type_density(t, hf) + 4.0 * type_density(t, m)) / 6.0;
        total += w[t] * dt;
        f += w[t] * type_field(t);
    }
    return vec2<f32>(above / max(0.6 * total, 1.0), f);
}

// The turn of the heaps' second layout (`layer_density`): 37 degrees about an arbitrary axis.
const HEAP_TURN: mat3x3<f32> = mat3x3<f32>(vec3<f32>(0.814429, 0.363554, -0.452254), vec3<f32>(-0.265714, 0.926538, 0.266315), vec3<f32>(0.515851, -0.096724, 0.851201));

// The shape volume's value at a texel (Perlin-Worley lifted by Worley fBm, about 0.64-0.97).
fn shape_value(s: vec4<f32>) -> f32 {
    let fbm = s.g * 0.625 + s.b * 0.25 + s.a * 0.125;
    return clamp(remap(s.r, fbm - 1.0, 1.0, 0.0, 1.0), 0.0, 1.0);
}

// The shape value spread evenly over 0..1 (its distribution, se_core cloud_noise.rs
// `print_shape_quantiles`: nearly normal about 0.833, deviation 0.041, the same to mip 3), so a share c of
// the area lies above 1 - c. (As it was, it lay within 0.77-0.89: any cover over a fifth filled the whole
// area.)
fn spread(v: f32) -> f32 {
    return 1.0 / (1.0 + exp(-41.5 * (v - 0.833)));
}

// The share of a layer's clouds seen as their mean (`layer_density`'s `avg`) at a footprint of fp m, for a
// layer whose clouds are `cumulus` cumuliform: solid heaps (up to about half the noise's 1.5 km cells wide)
// from a quarter of a cell on, before they shrink below the traced texels (which then caught some and
// missed others, the noise's tiles in stripes from 400 km); veils, a texture over the whole area, from
// a tenth on.
fn far_share(fp: f32, cumulus: f32) -> f32 {
    return smoothstep(mix(0.1, 0.015, cumulus), mix(0.5, 0.06, cumulus), fp * cv.params.y);
}

// Layer i's density at the sphere-space point x seen with a footprint of fp m. The cover is the
// flat shells' opacity, so the clouds keep it as their mean opacity seen from above. A cumuliform layer's
// clouds are solid heaps over a share of the area (`heaps`), dense inside (an optical depth of about 60
// through them: crisp edges, as real cumulus'); a stratiform layer's are veils over the whole area,
// textured by the shape noise, their optical depth the cover's. Beyond the noise's resolution only the
// mean is left (the flat shells' look from afar). `erode` false leaves the detail noise out (the light
// march's farther samples).
fn layer_density(i: i32, x: vec3<f32>, h: f32, fp: f32, wx: vec4<f32>, erode: bool) -> f32 {
    let a = cv.layer_a[i];
    if h <= a.x || h >= a.y || a.z <= 0.0 {
        return 0.0;
    }
    // SE's cover, shaped by the weather (belts, storms, the day's convection).
    let cover = weather_cover(cover_at(i, x, fp), wx.x, wx.y, a.z);
    if cover <= 0.01 {
        return 0.0;
    }
    let rc = regional_cover(i, x, wx);
    let sh = layer_shape(a, rc, cover, wx.z, h);
    if sh.d <= 0.0 {
        return 0.0;
    }
    let solid = a.w;
    let column = 0.6 * max(sh.depth, 1.0) * cv.layer_b[i].z;
    // Averaged over the whole area (seen from afar): the cover's opacity over the heaps' share of it
    // (after the erosion, which takes about a sixth of it).
    let avg = -log(1.0 - 0.995 * cover * sh.f * mix(1.0, 0.85, a.w)) / (0.6 * max(sh.md, 1.0) * cv.layer_b[i].z) * sh.d;
    // The heaps' noise (1.5 km cells), averaged out where they shrink to a few pixels (`far_share`).
    let freq = cv.params.y;
    let far = far_share(fp, solid);
    var near = avg;
    if far < 1.0 {
        // Read on the layer's base for solid clouds (each heap a column whose top its strength sets), in
        // 3D for veils.
        let xb = mix(x, x * ((cv.cam.w + a.x) / (cv.cam.w + h)), solid);
        let n1 = shape_value(textureSampleLevel(shape_tex, repeat_smp, fract((xb - cv.noise_off.xyz) * freq), lod_for(fp, freq, 128.0)));
        var ms = 0.0;
        var lump = 0.0;
        // (Left out of the march for a body whose clouds are all veils, `HEAPS` not set: compiled in,
        // its registers slowed every veil sample by about a third.)
        @if(HEAPS) {
        if solid > 0.0 {
            // A second layout, its cells 1.37 times as large and turned, the heaps from either (the
            // larger of two even values, squared, is even again): the noise's 6 km tile alone lined the
            // gaps up in rows towards the horizon.
            let n1b = shape_value(textureSampleLevel(shape_tex, repeat_smp, fract((HEAP_TURN * (xb - cv.noise_off.xyz)) * (0.73 * freq)), lod_for(fp, 0.73 * freq, 128.0)));
            let ub = max(spread(n1), spread(n1b));
            // Lumps: the shape noise at 500 m cells moving the heaps' strength by up to 0.3, faded out
            // before its cells shrink below the footprint.
            let freq2 = 3.0 * freq;
            let lw = 1.0 - far_share(fp * 3.0, 1.0);
            if lw > 0.0 {
                let n2 = shape_value(textureSampleLevel(shape_tex, repeat_smp, fract((x - cv.noise_off.xyz) * freq2), lod_for(fp, freq2, 128.0)));
                lump = 0.6 * lw * (spread(n2) - 0.5);
            }
            ms = heaps(a, type_weights(a.w, rc, wx.z), cover, wx.z, ub * ub, h, lump);
        }
        }
        // Veils: the noise times their curve.
        var mv = 0.0;
        if solid < 1.0 {
            mv = n1 * type_density(4, (h - a.x) / (a.y - a.x));
        }
        // The erosion only where its cells are larger than the footprint (else averaged out, faded so its
        // end makes no line): wispy at a solid cloud's base, billowy (the noise inverted) above it, its
        // tile 450 m for solid clouds (cells of 225, 112 and 56 m: cauliflower; at 900 m smooth lobes)
        // and 900 m for veils.
        let dfreq = cv.params.z * mix(1.0, 2.0, solid);
        let ew = 1.0 - smoothstep(0.1, 0.4, fp * dfreq);
        if erode && max(ms, mv) > 0.0 && ew > 0.0 {
            let e = textureSampleLevel(detail_tex, repeat_smp, fract((x - cv.detail_off.xyz) * dfreq), lod_for(fp, dfreq, 32.0));
            let efbm = e.r * 0.625 + e.g * 0.25 + e.b * 0.125;
            ms = remap(ms, mix(0.5, mix(efbm, 1.0 - efbm, clamp(sh.hf * 10.0, 0.0, 1.0)), ew) * sh.e, 1.0, 0.0, 1.0);
            mv = remap(mv, mix(0.5, efbm, ew) * sh.e, 1.0, 0.0, 1.0);
        } else if max(ms, mv) > 0.0 {
            // (Its mean where it is averaged out, else the clouds grew fatter there: a white band.)
            ms = remap(ms, 0.5 * sh.e, 1.0, 0.0, 1.0);
            mv = remap(mv, 0.5 * sh.e, 1.0, 0.0, 1.0);
        }
        // A near-overcast veil closes smoothly in its lower part (its last holes would be pinpricks).
        let close = smoothstep(0.75, 0.95, cover) * (1.0 - wx.z) * (1.0 - smoothstep(0.25, 0.6, sh.hf));
        // A solid cloud's surface sharpened to about 20 m of the skin (four times as much, about 60 m, made
        // a soft halo some 30 pixels wide round a cumulus 5 km away), softer where the march's steps are
        // long (its footprint is at least a quarter of a step: steps of 300 m far away through a 20 m
        // skin made grainy clouds); inside it an optical depth of 60 through 0.6 of its types' depth
        // (0.055 /m in a fair-weather cumulus, as real ones: 24 lit their bases nearly as brightly as
        // their tops).
        let sharp = clamp(350.0 / (3.0 * fp), 3.0, 16.0);
        near = (solid * 60.0 * clamp(ms * sharp, 0.0, 1.0) / 0.9
            + (1.0 - solid) * -log(1.0 - min(cover, 0.995)) * mix(clamp(mv * 2.0, 0.0, 1.0), 1.0, close) / 0.8) / column;
    }
    return mix(near, avg, far);
}

struct Sample {
    // The densest layer's index and the summed density (0..) and extinction (1/m).
    layer: i32,
    sigma: f32,
    // The cloud's albedo there.
    albedo: vec3<f32>,
    // The weather there (`weather_at`, the weather's own cloud textured), for the light march.
    wx: vec4<f32>,
};

// A gas giant's decks at x (layers: 0 the main deck, 1 the haze, 2 the deep deck): the main deck in
// SE's band colours, its top following SE's cloud height (zones above belts) and billowing, opaque
// below; the haze a thin veil; the deep deck dark and opaque.
fn giant_sample(x: vec3<f32>, h: f32, fp: f32) -> Sample {
    var s: Sample;
    s.layer = 0;
    s.sigma = 0.0;
    s.albedo = vec3<f32>(1.0);
    let fade = cv.giant.y;
    let a0 = cv.layer_a[0];
    var main = 0.0;
    if h < a0.y && h > a0.x {
        let lod = clamp(log2(max(fp / cv.atm_flags.z, 1e-6)), 0.0, 11.0);
        let gc = textureSampleLevel(cover_tex, clamp_smp, x, lod);
        var top = a0.x + (a0.y - a0.x) * (0.45 + 0.55 * gc.a);
        let far = smoothstep(0.3, 1.5, fp * cv.params.y);
        if far < 1.0 {
            let n = textureSampleLevel(shape_tex, repeat_smp, fract((x - cv.noise_off.xyz) * cv.params.y), lod_for(fp, cv.params.y, 128.0));
            let fbm = n.g * 0.625 + n.b * 0.25 + n.a * 0.125;
            top -= (a0.y - a0.x) * 0.12 * (1.0 - far) * (1.0 - fbm) * a0.w;
        }
        let hf = (h - a0.x) / max(top - a0.x, 1.0);
        main = cv.layer_b[0].z * smoothstep(1.0, 0.92, hf) * smoothstep(0.0, 0.05, hf);
        s.albedo = gc.rgb * gc.rgb;
    }
    let a1 = cv.layer_a[1];
    var haze = 0.0;
    if h < a1.y && h > a1.x {
        haze = cv.layer_b[1].z * a1.z * profile((h - a1.x) / (a1.y - a1.x), 0.0);
    }
    let a2 = cv.layer_a[2];
    var deep = 0.0;
    if h < a2.y && h > a2.x {
        deep = cv.layer_b[2].z * smoothstep(1.0, 0.85, (h - a2.x) / (a2.y - a2.x));
    }
    s.sigma = (main + haze + deep) * fade;
    if s.sigma > 0.0 {
        s.albedo = (s.albedo * main + cv.layer_c[1].rgb * haze + cv.layer_c[2].rgb * deep) * fade / s.sigma;
        s.layer = select(select(2, 1, haze > deep), 0, main >= max(haze, deep));
    }
    return s;
}

// A gas giant's optical depth straight up from height h to the top of its decks.
fn giant_column(h: f32) -> f32 {
    var col = 0.0;
    for (var i = 0; i < 3; i++) {
        let a = cv.layer_a[i];
        // The main deck's mean top.
        let top = select(a.y, a.x + (a.y - a.x) * 0.72, i == 0);
        col += cv.layer_b[i].z * a.z * max(top - max(h, a.x), 0.0);
    }
    return col * cv.giant.y;
}

// The weather's own cloud (storms, anvils, fronts: the weather cube's coarse cells) shaped by its
// pattern (`weather_pattern`, read at the footprint; cloud_cover.wesl `weather_shaped`): cloud masses
// with ragged edges and clear gaps at every scale, from orbit too. (The shape noise's Worley cells once
// made a honeycomb veil from orbit, and the coarse cells alone soft round blobs.)
fn weather_textured(x: vec3<f32>, fp: f32) -> vec4<f32> {
    var wx = weather_at(x);
    if wx.y > 0.0 {
        let m = textureSampleLevel(pattern_tex, clamp_smp, x, clamp(log2(max(fp / cv.atm_flags.z, 1e-6)), 0.0, 11.0)).r;
        wx.y = weather_shaped(wx.y, m);
    }
    return wx;
}

// The clouds at x under the weather wx (`weather_textured` there or nearby).
fn sample_with(x: vec3<f32>, fp: f32, wx: vec4<f32>, erode: bool) -> Sample {
    var s: Sample;
    s.layer = 0;
    s.sigma = 0.0;
    s.wx = wx;
    let h = length(x) - cv.cam.w;
    if cv.giant.x > 0.5 {
        s = giant_sample(x, h, fp);
        s.wx = wx;
        return s;
    }
    var best = 0.0;
    for (var i = 0; i < LAYERS; i++) {
        let d = layer_density(i, x, h, fp, wx, erode);
        let sg = d * cv.layer_b[i].z;
        s.sigma += sg;
        if sg > best {
            best = sg;
            s.layer = i;
        }
    }
    s.albedo = cv.layer_c[s.layer].rgb;
    return s;
}

fn sample_at(x: vec3<f32>, fp: f32) -> Sample {
    if cv.giant.x > 0.5 {
        return giant_sample(x, length(x) - cv.cam.w, fp);
    }
    return sample_with(x, fp, weather_textured(x, fp), true);
}

// Whether any layer could hold cloud at x: its cover there (averaged over the footprint fp) under the
// weather, the weather's own cloud solid and its clearing at half, without the noise. The march's long
// steps through clear air test it.
fn maybe_cloud(x: vec3<f32>, fp: f32) -> bool {
    if cv.giant.x > 0.5 {
        return true;
    }
    let h = length(x) - cv.cam.w;
    var wx = vec4<f32>(0.0);
    var read = false;
    for (var i = 0; i < LAYERS; i++) {
        let a = cv.layer_a[i];
        if h <= a.x || h >= a.y || a.z <= 0.0 {
            continue;
        }
        if !read {
            wx = weather_at(x);
            wx.y = select(0.5 * wx.y, 1.0, wx.y > 0.002);
            read = true;
        }
        let c = weather_cover(cover_at(i, x, fp), wx.x, wx.y, a.z);
        if c <= 0.01 {
            continue;
        }
        // A cumuliform layer's heaps reach no higher than their types' tops (`type_height`): above
        // them the air is clear. (Marched finely up to the layer's top, 9 km on Earth over heaps a
        // kilometre tall, a cumulus field cost twice as much.)
        @if(HEAPS) {
        if a.w > 0.99 && h - a.x > heaps_ceiling(a, type_weights(a.w, regional_cover(i, x, wx), wx.z), c, wx.z) + 150.0 {
            continue;
        }
        }
        return true;
    }
    return false;
}

// The optical depth from x towards the star through the clouds' mean above it, without a march: each
// layer's cover there as the area's mean cloud (as `layer_density`'s `avg`; a low star's ray crosses
// the gaps between clouds too), over the part of its profile above x, along the star's slant. Beyond
// the light march, and alone far away, where its self-shadows are smaller than a pixel.
fn light_column(x: vec3<f32>, fp: f32, mu: f32, wx: vec4<f32>) -> f32 {
    let h = length(x) - cv.cam.w;
    let slant = 1.0 / max(mu, 0.08);
    var od = 0.0;
    for (var i = 0; i < LAYERS; i++) {
        let a = cv.layer_a[i];
        if h >= a.y || a.z <= 0.0 {
            continue;
        }
        let cover = weather_cover(cover_at(i, x, fp), wx.x, wx.y, a.z);
        if cover <= 0.01 {
            continue;
        }
        let sa = shape_above(a, regional_cover(i, x, wx), cover, wx.z, h);
        od += -log(1.0 - 0.995 * cover * sa.y) * sa.x;
    }
    return od * slant;
}

// The light volume's distance along the ground (a fraction of its half-width) at a texel coordinate
// u (-1..1): texels a quarter as long at its centre as at its edge (`light_unwarp` the inverse).
fn light_warp(u: f32) -> f32 {
    return sign(u) * (0.25 * abs(u) + 0.75 * u * u);
}

fn light_unwarp(a: f32) -> f32 {
    return sign(a) * (sqrt(0.0625 + 3.0 * abs(a)) - 0.25) / 1.5;
}

// The sphere-space point of the light volume f at texture coordinates q (0..1).
fn light_point(f: LightFrame, q: vec3<f32>) -> vec3<f32> {
    let a = light_warp(q.x * 2.0 - 1.0) * f.c.w;
    let b = light_warp(q.y * 2.0 - 1.0) * f.c.w;
    let dir = normalize(f.c.xyz * f.h.w + f.e1.xyz * a + f.e2.xyz * b);
    return dir * (f.h.w + f.h.x + (f.h.y - f.h.x) * q.z * q.z);
}

// The light volume at x: R the star's optical depth on from x to the clouds' top, G the cloud's above
// x, B how much of it to take (1 inside it, fading to 0 at its edges, 0 where it holds none).
fn light_at(x: vec3<f32>) -> vec3<f32> {
    if lv.h.z < 0.5 {
        return vec3<f32>(0.0);
    }
    let r = length(x);
    let dir = x / r;
    let k = dot(dir, lv.c.xyz);
    if k < 0.9 {
        return vec3<f32>(0.0);
    }
    let g = (dir / k - lv.c.xyz) * (lv.h.w / lv.c.w);
    let q = vec2<f32>(light_unwarp(dot(g, lv.e1.xyz)), light_unwarp(dot(g, lv.e2.xyz)));
    let w = 1.0 - smoothstep(0.85, 0.97, max(abs(q.x), abs(q.y)));
    if w <= 0.0 {
        return vec3<f32>(0.0);
    }
    let z = sqrt(clamp((r - lv.h.w - lv.h.x) / max(lv.h.y - lv.h.x, 1.0), 0.0, 1.0));
    let s = textureSampleLevel(light_tex, clamp_smp, vec3<f32>(q * 0.5 + 0.5, z), 0.0).rg;
    return vec3<f32>(s, w);
}

// A texel of the light volume being built: the star's optical depth from its point to the clouds' top
// along the star's ray over the round body (24 steps, their length growing with the square of their
// number; past 60 km the cloud above's mean, `light_column`), and the cloud's straight up (10 steps),
// the noise blurred to the texel (the detail noise left out).
@fragment
fn light_gen(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let f = lgen.frame;
    let q = vec3<f32>(in.position.xy / lgen.slice.y, (lgen.slice.x + 0.5) / lgen.slice.z);
    let x = light_point(f, q);
    let r = length(x);
    let up = x / r;
    let h = r - f.h.w;
    let l = f.sun.xyz;
    let mu = dot(up, l);
    let u = max(abs(q.x * 2.0 - 1.0), abs(q.y * 2.0 - 1.0));
    let texel = f.c.w * (0.25 + 1.5 * u) * 2.0 / lgen.slice.y;
    let dh = max(f.h.y - h, 0.0);
    let total = climb_dist(r, max(mu, 0.0), dh);
    let span = min(total, 60000.0);
    var od = 0.0;
    for (var k = 0; k < 24; k++) {
        let s0 = span * f32(k * k) / 576.0;
        let s1 = span * f32((k + 1) * (k + 1)) / 576.0;
        let xs = x + l * (0.5 * (s0 + s1));
        let fp = max(s1 - s0, texel);
        od += sample_with(xs, fp, weather_textured(xs, fp), false).sigma * (s1 - s0);
    }
    if total > span {
        let xe = x + l * span;
        od += light_column(xe, max(texel, 3000.0), max(dot(normalize(xe), l), 0.0), weather_textured(xe, 3000.0));
    }
    var col = 0.0;
    for (var k = 0; k < 10; k++) {
        let s0 = dh * f32(k * k) / 100.0;
        let s1 = dh * f32((k + 1) * (k + 1)) / 100.0;
        let xs = x + up * (0.5 * (s0 + s1));
        let fp = max(s1 - s0, texel);
        col += sample_with(xs, fp, weather_textured(xs, fp), false).sigma * (s1 - s0);
    }
    return vec4<f32>(od, col, 0.0, 1.0);
}

fn hg(cos_t: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cos_t, 1e-4), 1.5));
}

// ---------------------------------------------------------------------------------------------
// The weather in the air.

const FLASH_COLOR: vec3<f32> = vec3<f32>(0.75, 0.82, 1.0);
// The ground's albedo under the clouds, for the light it sends up to their bases: a mean land's.
const GROUND_ALBEDO: vec3<f32> = vec3<f32>(0.10, 0.11, 0.08);
const FREEZE_K: f32 = 273.15;
const DUST_ALBEDO: vec3<f32> = vec3<f32>(0.68, 0.5, 0.34);

// Lightning's light at x (sphere space): each flash glows through the cloud around it.
fn lightning_at(x: vec3<f32>) -> vec3<f32> {
    if cv.fx.y < 0.5 {
        return vec3<f32>(0.0);
    }
    var e = 0.0;
    for (var k = 0; k < 8; k++) {
        let f = flashes[k];
        if f.w > 0.0 {
            e += f.w * exp(-length(x - f.xyz) / 6000.0);
        }
    }
    return FLASH_COLOR * (e * cv.fx2.z);
}

// The air's temperature (K) at x: the body's mean, warmer at the latitudes under the sun, colder
// towards the poles and with height (6.5 K a km).
fn air_temp(x: vec3<f32>, l_s: vec3<f32>) -> f32 {
    let r = length(x);
    let b = asin(clamp(x.y / r, -1.0, 1.0)) - 0.5 * asin(clamp(l_s.y, -1.0, 1.0));
    let c = cos(b);
    let t0 = cv.fx2.y + 40.0 * (c * c - 0.65) - 15.0 * max(abs(sin(b)) - 0.7, 0.0) / 0.3;
    return t0 - 0.0065 * (r - cv.cam.w);
}

// The lowest cloud layer (the one rain falls from), -1 if none.
fn rain_layer() -> i32 {
    var best = -1;
    var h = 1e30;
    for (var i = 0; i < LAYERS; i++) {
        let a = cv.layer_a[i];
        if a.z > 0.0 && a.x < h {
            h = a.x;
            best = i;
        }
    }
    return best;
}

// The star's light at radius r (sphere space) with the star at height mu (cosine), as a part: gone
// below the horizon of that height, fading in over about 3 degrees above it. The atmosphere's own
// edge is a sun disc wide (about 30 km), which from orbit drew the clouds' shadow line as a hard
// line, and their tops stayed bright orange right up to it.
fn twilight(r: f32, mu: f32) -> f32 {
    let rr = clamp(cv.cam.w / r, 0.0, 1.0);
    let mu_h = -sqrt(1.0 - rr * rr);
    return smoothstep(mu_h, mu_h + 0.06, mu);
}

struct AirLight {
    sun: vec3<f32>,
    sky: vec3<f32>,
    // The star's height there (cosine).
    mu: f32,
};

// The light on the air at x (sphere space) under a cloud cover `shade` (0..1): the star's beam
// through the atmosphere, dimmed under the cover, and the sky's (as a white surface's radiance).
fn air_light(x: vec3<f32>, l_s: vec3<f32>, shade: f32) -> AirLight {
    let r = length(x);
    let mu = dot(x / r, l_s);
    var e_sun = cv.sun_light.rgb * cv.sun_dir.w * twilight(r, mu);
    var sky = vec3<f32>(0.0);
    if cv.atm_flags.x > 0.5 {
        let rk = r * cv.atm.radii.x / cv.cam.w;
        e_sun *= atm_transmittance_sun(cv.atm, transmittance_tex, clamp_smp, rk, mu);
        sky = atm_irradiance(cv.atm, irradiance_tex, clamp_smp, rk, mu) * cv.atm.look.w;
    } else {
        let rr = clamp(cv.cam.w / r, 0.0, 1.0);
        e_sun *= clamp((mu + sqrt(1.0 - rr * rr)) * 200.0, 0.0, 1.0);
    }
    var out: AirLight;
    out.sun = e_sun * (1.0 - 0.85 * shade);
    out.sky = cv.sun_light.rgb * cv.sun_dir.w * ATM_AMBIENT_SCALE * sky * (1.0 - 0.5 * shade) / PI;
    out.mu = mu;
    return out;
}

// The light the air's drops, fog or dust send towards the eye per unit of extinction (times their
// albedo): the beam once scattered (phase g, part of it evenly), the light scattered many times in a
// thick layer (as a white surface lit by the star), and the sky's.
fn air_scatter(al: AirLight, cos_t: f32, g: f32) -> vec3<f32> {
    return al.sun * (mix(hg(cos_t, g), 0.25 / PI, 0.4) + 0.5 * max(al.mu, 0.0) / PI) + al.sky;
}

// Rain or snow at x (h its height) under layer li: falling from its base where the weather rains
// and the cloud is thick enough, in shafts slanted by the wind. x: extinction (1/m), y: 1 for snow,
// z: the cover overhead.
fn precip_at(x: vec3<f32>, h: f32, li: i32, l_s: vec3<f32>) -> vec3<f32> {
    let base = cv.layer_a[li].x;
    if h >= base {
        return vec3<f32>(0.0);
    }
    let snow = air_temp(x, l_s) < FREEZE_K;
    let fall = select(7.0, 1.5, snow);
    // Where it left the cloud base, drifting with the wind on the way down.
    let xb = normalize(x - cv.wind.xyz * ((base - h) / fall)) * (cv.cam.w + base);
    let wb = weather_textured(xb, 2000.0);
    if wb.w < 0.02 {
        return vec3<f32>(0.0);
    }
    let cover = weather_cover(cover_at(li, xb, 2000.0), wb.x, wb.y, cv.layer_a[li].z);
    let n = textureSampleLevel(shape_tex, repeat_smp, fract((xb - cv.noise_off.xyz) * (cv.params.y / 1.7)), 2.0).g;
    // A deck's steady rain falls everywhere under it, showers in shafts.
    let shaft = mix(1.0, smoothstep(0.3, 0.65, n), clamp(wb.z * 1.5, 0.0, 1.0)) * smoothstep(0.45, 0.85, cover);
    let sigma = wb.w * 2.5e-3 * shaft * smoothstep(base, base - 300.0, h) * select(1.0, 1.6, snow);
    return vec3<f32>(sigma, select(0.0, 1.0, snow), cover);
}

// The part of the ray o + t d below height h, up to t_max and at most `cap` long.
fn below(h: f32, a: f32, b: f32, t_max: f32, cap: f32) -> vec2<f32> {
    let s = sphere_hits(h, a, b);
    if s.x > s.y || s.y <= 0.0 {
        return vec2<f32>(1.0, -1.0);
    }
    let t0 = max(s.x, 0.0);
    return vec2<f32>(t0, min(min(s.y, t_max), t0 + cap));
}

struct Media {
    light: vec3<f32>,
    trans: f32,
};

// The weather in the air along the ray up to t_max: rain and snow shafts and dust in ten samples,
// fog near the ground integrated over its height (120 m scale), all lit by the star and the sky.
fn media(o: vec3<f32>, d: vec3<f32>, a: f32, b: f32, t_max: f32, cos_t: f32, l_s: vec3<f32>, jit: f32) -> Media {
    var m: Media;
    m.light = vec3<f32>(0.0);
    m.trans = 1.0;
    let rain_on = cv.fx.x > 0.5;
    let dust_on = cv.fx.w > 0.5;
    let li = select(-1, rain_layer(), rain_on);
    let top = max(select(0.0, cv.layer_a[max(li, 0)].x, li >= 0), select(0.0, 8000.0, dust_on));
    let seg = below(top, a, b, t_max, 60000.0);
    if top > 0.0 && seg.y > seg.x {
        let dt = (seg.y - seg.x) / 10.0;
        for (var i = 0; i < 10; i++) {
            let x = o + d * (seg.x + (f32(i) + jit) * dt);
            let h = length(x) - cv.cam.w;
            var pr = vec3<f32>(0.0);
            if li >= 0 {
                pr = precip_at(x, h, li, l_s);
            }
            var dust = 0.0;
            if dust_on {
                dust = textureSampleLevel(fx_tex, clamp_smp, x, 0.0).g * 1.2e-3 * exp(-max(h, 0.0) / 2500.0);
            }
            let sigma = pr.x + dust;
            if sigma > 1e-8 {
                let al = air_light(x, l_s, pr.z);
                let snow = pr.y > 0.5;
                let src = pr.x * select(vec3<f32>(0.6), vec3<f32>(0.95), snow) * air_scatter(al, cos_t, select(0.6, 0.4, snow))
                    + dust * DUST_ALBEDO * air_scatter(al, cos_t, 0.6);
                let st = exp(-sigma * dt);
                m.light += m.trans * src * (1.0 - st) / sigma;
                m.trans *= st;
            }
        }
    }
    if cv.fx.z > 0.5 {
        let fs = below(1500.0, a, b, t_max, 40000.0);
        if fs.y > fs.x {
            // Four pieces along which the height changes linearly.
            let dt = (fs.y - fs.x) / 4.0;
            var h0 = length(o + d * fs.x) - cv.cam.w;
            var tau = 0.0;
            for (var k = 1; k <= 4; k++) {
                let x1 = o + d * (fs.x + f32(k) * dt);
                let h1 = length(x1) - cv.cam.w;
                let s0 = textureSampleLevel(fx_tex, clamp_smp, x1, 0.0).r * 0.012;
                let e0 = exp(-max(h0, 0.0) / 120.0);
                let e1 = exp(-max(h1, 0.0) / 120.0);
                var piece = e0;
                if abs(h1 - h0) > 0.5 {
                    piece = (e0 - e1) * 120.0 / (h1 - h0);
                }
                tau += s0 * piece * dt;
                h0 = h1;
            }
            if tau > 1e-5 {
                let al = air_light(o + d * mix(fs.x, fs.y, 0.7), l_s, 0.0);
                let tf = exp(-tau);
                let fog = (1.0 - tf) * 0.9 * air_scatter(al, cos_t, 0.7);
                // Seen from inside the fog it is in front of the rest, from above behind it.
                if cv.cam2.y < 1500.0 {
                    m.light = fog + tf * m.light;
                } else {
                    m.light += m.trans * fog;
                }
                m.trans *= tf;
            }
        }
    }
    return m;
}

struct Trace {
    @location(0) light: vec4<f32>,
    @location(1) dist: vec4<f32>,
};

@fragment
fn trace(in: FullscreenVertexOutput) -> Trace {
    var out: Trace;
    let lp = traced_texel(vec2<i32>(in.position.xy));
    let uv = cloud_uv(lp);
    let dir = ray_dir(uv);
    var limit = textureLoad(low_depth, lp, 0).r;
    out.light = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    // (The resolve reprojects a clear texel by the scene behind it: capped at 10,000 km, the ground of a
    // planet seen from farther moved wrongly as the camera orbited, and last frame's clouds beside it
    // were kept there.)
    out.dist = vec4<f32>(min(limit, 1.0e20), 0.0, 0.0, 0.0);
    if cv.params.x < 0.5 {
        return out;
    }
    let o = cv.cam.xyz;
    let d = (cv.sphere_from_world * vec4<f32>(dir, 0.0)).xyz;
    // A gas giant's flat surface (SE's bands at its radius) lies inside its decks: they replace it.
    if cv.giant.x > 0.5 && limit < FAR && length(o + d * limit) < cv.cam.w * 1.002 {
        limit = FAR;
    }
    let a = dot(d, d);
    let b = dot(o, d);
    let bottom_h = cv.cam2.z;
    let top_h = cv.atm_flags.w;
    let ground = sphere_hits(min(bottom_h, 0.0) - 500.0, a, b);
    var t_max = limit;
    if ground.x <= ground.y && ground.y > 0.0 {
        t_max = min(t_max, max(ground.x, 0.0));
    }
    // The part of the ray (`CloudSize::params` w): all of it, beyond the near pass's range, within it.
    let mode = cs.params.w;
    let r_near = cv.lod.z;
    if mode > 1.5 {
        t_max = min(t_max, r_near);
    }
    let l_world = normalize(cv.sun_dir.xyz);
    let l_s = normalize((cv.sphere_from_world * vec4<f32>(l_world, 0.0)).xyz);
    let cos_t = dot(dir, l_world);
    // The phase of the star's light and of its three octaves of bounces (`trace`'s light), the same
    // along the ray: forward and back lobes, flatter for each octave.
    let phase = vec4<f32>(
        mix(hg(cos_t, 0.8), hg(cos_t, -0.25), 0.35),
        mix(hg(cos_t, 0.4), hg(cos_t, -0.125), 0.35),
        mix(hg(cos_t, 0.2), hg(cos_t, -0.0625), 0.35),
        mix(hg(cos_t, 0.1), hg(cos_t, -0.03125), 0.35),
    );
    // The weather in the air, drawn even where the ray meets no cloud.
    // (The near pass draws the clouds only: the far one has the air's weather over the whole ray.)
    var m: Media;
    m.light = vec3<f32>(0.0);
    m.trans = 1.0;
    if mode < 1.5 {
        m = media(o, d, a, b, t_max, cos_t, l_s, fract(jitter(in.position.xy) + 0.5));
        m.light *= cv.lod.y;
        m.trans = mix(1.0, m.trans, cv.lod.y);
    }
    out.light = vec4<f32>(m.light, m.trans);
    // An atmosphere without cloud layers (its fog and dust only), or beyond the far cross-fade (the
    // flat shells draw the clouds).
    if top_h <= 0.0 || cv.lod.x <= 0.0 {
        out.light = giant_dark(out.light);
        return out;
    }
    let top = sphere_hits(top_h, a, b);
    if top.y <= 0.0 || top.x > top.y {
        out.light = giant_dark(out.light);
        return out;
    }
    let bottom = sphere_hits(bottom_h, a, b);
    // Up to two pieces of the shell along the ray: before the bottom sphere and after it.
    var seg0 = vec2<f32>(max(top.x, 0.0), min(top.y, t_max));
    var seg1 = vec2<f32>(1.0, -1.0);
    if bottom.x <= bottom.y && bottom.y > 0.0 {
        if cv.cam2.y < bottom_h {
            seg0.x = max(seg0.x, bottom.y);
        } else {
            seg1 = vec2<f32>(bottom.y, min(top.y, t_max));
            seg0.y = min(seg0.y, bottom.x);
        }
    }
    if mode > 0.5 && mode < 1.5 {
        seg0.x = max(seg0.x, r_near);
        seg1.x = max(seg1.x, r_near);
    }
    let len0 = max(seg0.y - seg0.x, 0.0);
    let len1 = max(seg1.y - seg1.x, 0.0);
    let total = len0 + len1;
    if total <= 0.0 {
        out.light = giant_dark(out.light);
        return out;
    }
    // Steps of at least 1/MAX_STEPS of the path and no shorter than 50 m; near the camera shorter
    // steps (the first quarter of the steps covers an eighth of the path). The reference
    // (volumetric_clouds.rs `reference_on`) takes twice as many.
    let fine_n = select(1.0, 2.0, cs.params.y > 1.5);
    let dt0 = max(total / (select(f32(MAX_STEPS), NEAR_STEPS, mode > 1.5) * fine_n), 50.0 / fine_n);
    // Longer steps where a sample covers more cloud (far away), at least 24 over the path (fair-weather
    // cumulus fill only the layer's lowest kilometre or so: fewer steps over it thinned them).
    let dt_cap = max(dt0, total / (24.0 * fine_n));
    let has_atm = cv.atm_flags.x > 0.5;
    let to_km = cv.atm.radii.x / cv.cam.w;
    // A cloud pixel's angular size: the footprint of a sample is its distance times this.
    let px_angle = length(ray_dir(cloud_uv(lp + vec2<i32>(1, 0))) - dir);
    var light = vec3<f32>(0.0);
    var trans = 1.0;
    var t_sum = 0.0;
    var w_sum = 0.0;
    // The last lit sample's light per unit of extinction, reused for the next sample when that one is
    // optically thin (a veil's light changes little over a step; a veil that fills every ray made the
    // light most of the march's cost).
    var last_k = vec3<f32>(0.0);
    var reuse_ok = false;
    let t_first = jitter(in.position.xy) * dt0;
    var t = t_first;
    // Through clear air long steps (four fine ones, at least 24 over the path) that only ask whether a
    // cloud could be there (`maybe_cloud`); on a hit back one long step and on in fine ones, until a
    // few fine samples in a row past the hit find nothing. (Never back over what the fine steps
    // sampled: in a deck's empty upper part that re-marched it over and over, and the deck below was
    // never reached.)
    var coarse = true;
    var fine_until = 0.0;
    var fine_end = t_first;
    var empty = 0;
    var fine = 0;
    let max_fine = i32(fine_n) * MAX_STEPS;
    for (var i = 0; i < max_fine + 48 * i32(fine_n); i++) {
        if t >= total || trans < 0.005 || fine >= max_fine {
            break;
        }
        let tw = select(seg1.x + (t - len0), seg0.x + t, t < len0);
        let x = o + d * tw;
        let fp = max(tw * px_angle, dt0 * 0.25);
        let dt = clamp(0.4 / fine_n * tw * px_angle, dt0, dt_cap);
        if coarse {
            let long = min(4.0 * dt, dt_cap);
            if maybe_cloud(x, max(fp, long)) {
                coarse = false;
                empty = 0;
                fine_until = t + long;
                t = max(t - long, fine_end);
            } else {
                t += long;
            }
            continue;
        }
        fine += 1;
        let smp = sample_at(x, fp);
        if smp.sigma <= 1e-7 {
            empty += 1;
            coarse = empty >= 3 && t > fine_until;
            reuse_ok = false;
        } else {
            empty = 0;
            let reuse = reuse_ok && smp.sigma * dt < 0.05;
            reuse_ok = !reuse;
            if !reuse {
            let li = smp.layer;
            let r = length(x);
            let up = x / r;
            let mu = dot(up, l_s);
            // The light towards the star. Near the camera three steps of 40, 80 and 180 m (the first two
            // with the detail noise: the billows' own shadows), then the light volume (`light_at`: the long
            // self-shadows) where it reaches, elsewhere (and at its edges, blended) a march on, steps
            // doubling to 3.1 km without the detail noise, then the cloud above's mean (`light_column`).
            // From 20 to 40 km on the light volume at the point alone, beyond it the mean (the steps'
            // self-shadows are smaller than a pixel there). (Taking the steps through the whole light
            // volume, 60 km round, doubled the march's cost under a veil that fills every ray.)
            let lw = select(smoothstep(20000.0, 40000.0, tw), 0.0, cv.giant.x > 0.5);
            var vx = vec3<f32>(0.0);
            if lv.h.z > 0.5 {
                vx = light_at(x);
            }
            var od = 0.0;
            if lw < 1.0 {
                var lx = x;
                for (var k = 0; k < 3; k++) {
                    let ls = select(select(180.0, 80.0, k == 1), 40.0, k == 0);
                    lx += l_s * ls;
                    od += sample_with(lx - l_s * (0.5 * ls), max(fp, ls), smp.wx, k < 2).sigma * ls;
                }
                var vs = vec3<f32>(0.0);
                if vx.z > 0.0 {
                    vs = light_at(lx);
                }
                var od_m = od;
                if vs.z < 1.0 {
                    var ls = 400.0;
                    for (var k = 2; k < LIGHT_STEPS; k++) {
                        lx += l_s * ls;
                        od_m += sample_with(lx, max(fp, ls), smp.wx, false).sigma * ls;
                        ls *= 2.0;
                    }
                    if cv.giant.x < 0.5 {
                        od_m += light_column(lx, max(fp, 3100.0), dot(normalize(lx), l_s), smp.wx);
                    }
                }
                od = mix(od_m, od + vs.x, vs.z);
            }
            if lw > 0.0 {
                var od_f = vx.x;
                if vx.z < 1.0 {
                    od_f = mix(light_column(x, fp, mu, smp.wx), vx.x, vx.z);
                }
                od = mix(od, od_f, lw);
            }
            // The star's light at this point: through the atmosphere (zero below the horizon).
            let tw_l = twilight(r, mu);
            var e_sun = cv.sun_light.rgb * cv.sun_dir.w * tw_l;
            var sky = vec3<f32>(0.0);
            if has_atm {
                let rk = r * to_km;
                e_sun *= atm_transmittance_sun(cv.atm, transmittance_tex, clamp_smp, rk, mu);
                sky = atm_irradiance(cv.atm, irradiance_tex, clamp_smp, rk, mu) * cv.atm.look.w;
            } else {
                let rr = clamp(cv.cam.w / r, 0.0, 1.0);
                e_sun *= clamp((mu + sqrt(1.0 - rr * rr)) * 200.0, 0.0, 1.0);
            }
            // The star's light scattered here: the direct beam (forward and back lobes), three octaves of
            // ever weaker extinction, scattering and phase for the first bounces (Wrenninge's, as
            // blackrack's four: a soft glow deep inside), and the diffuse light deep in the cloud, as a
            // thick conservative slab transmits it (two-stream: 1 / (1 + 3/4 (1 - g) tau), g 0.85),
            // radiating evenly: a cloud top about as bright as a white Lambert surface, the base of a
            // cumulus (an optical depth of about 60 through it) a tenth of that. The slab takes the
            // star's light as a horizontal surface does (by the sun's height), towers' sides some of it
            // at any height. The bounces grow with the local density (blackrack's: dense cores glow, thin
            // edges stay dim).
            // In a gas giant the cloud above (deeper than the light march) dims the light, absorbing
            // the blue most: deep down it is dim and red, then dark.
            var absorb = vec3<f32>(1.0);
            var od_d = od;
            if cv.giant.x > 0.5 {
                let col = giant_column(r - cv.cam.w);
                od = max(od, col / max(mu, 0.08));
                // The deck's top is as bright as SE's flat bands: diffuse light dims only with the
                // cloud above it.
                od_d = col;
                absorb = exp(-col * vec3<f32>(0.012, 0.018, 0.03));
            }
            let direct = phase.x * exp(-od);
            let bounce = 0.5 * phase.y * exp(-0.5 * od) + 0.25 * phase.z * exp(-0.25 * od) + 0.125 * phase.w * exp(-0.125 * od);
            let flux = clamp(mu * 1.2 + 0.15 * tw_l, 0.0, 1.0);
            let dense = mix(0.5, 1.0, 1.0 - exp(-40.0 * smp.sigma));
            // (A giant's deep decks under a bright top: a white surface's light at the top.)
            let ms = direct + dense * (bounce + select(0.5, 1.2, cv.giant.x > 0.5) * flux / (PI * (1.0 + 0.11 * od_d)) * absorb);
            // Sky light from above, more of it near the tops; SE's day ambient as the flat shells.
            let la = cv.layer_a[li];
            let hf = clamp((r - cv.cam.w - la.x) / max(la.y - la.x, 1.0), 0.0, 1.0);
            let e_top = cv.sun_light.rgb * cv.sun_dir.w;
            // The sky's light through the cloud above (the light volume's, as a thick slab passes diffuse
            // light: 1 / (1 + 0.11 tau)); beyond it, more of it near the layer's top.
            let open = mix(0.35 + 0.65 * hf, 1.0 / (1.0 + 0.11 * vx.y), vx.z);
            let ambient = e_top * ATM_AMBIENT_SCALE * (max(mu, 0.0) * cv.layer_c[li].w + sky) * open / PI * 2.0 * absorb;
            // The ground's light on the lower parts (those with cloud above them see it): the star's on
            // a mean land's colour below.
            let ground = e_sun * (max(mu, 0.0) * (1.0 - open) * 0.8 / PI) * GROUND_ALBEDO * absorb;
            let albedo = smp.albedo;
            last_k = albedo * (e_sun * ms + ambient + ground + lightning_at(x));
            }
            let src = last_k * smp.sigma;
            let step_t = exp(-smp.sigma * dt);
            light += trans * (src - src * step_t) / max(smp.sigma, 1e-9);
            t_sum += tw * trans * (1.0 - step_t);
            w_sum += trans * (1.0 - step_t);
            trans *= step_t;
        }
        t += dt;
        fine_end = t;
    }
    var dist = min(seg0.x + len0 * 0.5, out.dist.x);
    if w_sum > 1e-4 {
        dist = t_sum / w_sum;
    }
    // The haze between the camera and the clouds: their light dimmed by it, and the air's own light
    // in front of them (the sky behind already holds it, so only for the part the clouds hide).
    // (Not over a gas giant's decks: they are its cloud tops, which SE's flat bands draw without it;
    // their own high haze stands in.)
    if has_atm && trans < 0.999 && cv.giant.x < 0.5 {
        let cam_km = o * to_km;
        let v = normalize(d);
        let ps = atm_path(cv.atm, inscatter_tex, transmittance_tex, clamp_smp, cam_km, v, l_s, length(d) * dist * to_km);
        let e_haze = cv.sun_light.rgb * cv.sun_dir.w * cv.atm_flags.y;
        light = light * ps.transmittance + (1.0 - trans) * e_haze * ps.inscatter;
    }
    // Within the far cross-fade the flat shells draw the rest.
    light *= cv.lod.x;
    trans = mix(1.0, trans, cv.lod.x);
    // The weather's air below the clouds is in front of them seen from under them, behind them from
    // above.
    if cv.cam2.y < bottom_h {
        out.light = vec4<f32>(m.light + m.trans * light, m.trans * trans);
    } else {
        out.light = vec4<f32>(light + trans * m.light, trans * m.trans);
    }
    out.light = giant_dark(out.light);
    out.dist.x = dist;
    return out;
}

// Below a gas giant's deep deck the light is gone: the view fades to dark over two scale heights.
fn giant_dark(l: vec4<f32>) -> vec4<f32> {
    if cv.giant.x < 0.5 {
        return l;
    }
    let dark = smoothstep(cv.cam2.z, cv.cam2.z - 2.0 * cv.giant.z, cv.cam2.y) * cv.giant.y;
    return l * (1.0 - dark);
}

// The ground's shadow map: the light the clouds let through to a point on the reference sphere from
// the star, through every layer: the direct beam, plus a quarter of the daylight passed on diffusely,
// so the ground under a thick cloud keeps a quarter of it, as the cover's shadow farther out gives
// (planet.wesl `cloud_shadow` reads it). (A two-stream share, as in `trace`, let three times more
// through than the cover's shadow: the shadows faded as the camera came down to the ground.)
@fragment
fn shadow_gen(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let w = cv.shadow_c.w;
    let p = cv.shadow_c.xyz + (cv.shadow_e1.xyz * (in.uv.x * 2.0 - 1.0) + cv.shadow_e2.xyz * (in.uv.y * 2.0 - 1.0)) * w;
    let g = normalize(p) * cv.cam.w;
    let l_s = normalize((cv.sphere_from_world * vec4<f32>(normalize(cv.sun_dir.xyz), 0.0)).xyz);
    // (Past the terminator as for a grazing sun: the ground there still takes some of its light.)
    let mu = max(dot(normalize(g), l_s), 0.0);
    if cv.params.x < 0.5 || cv.atm_flags.w <= 0.0 || cv.giant.x > 0.5 {
        return vec4<f32>(1.0, 0.0, 0.0, 0.0);
    }
    // From the lowest bottom to the highest top along the sun's ray over the round body, in 24 steps, the
    // noise blurred to a step. (Taken over flat ground and cut at 40 km, a low sun's ray passed above the
    // clouds: a square of sunlit ground under them around the camera.)
    let t0 = climb_dist(cv.cam.w, mu, max(cv.cam2.z, 0.0));
    let t1 = climb_dist(cv.cam.w, mu, cv.atm_flags.w);
    let dt = (t1 - t0) / 24.0;
    let fp = max(2.0 * w / 512.0, dt);
    var od = 0.0;
    for (var k = 0; k < 24; k++) {
        let x = g + l_s * (t0 + (f32(k) + 0.5) * dt);
        od += sample_at(x, fp).sigma * dt;
    }
    return vec4<f32>(0.25 + 0.75 * exp(-od), 0.0, 0.0, 0.0);
}

// The cloud texels from this frame's trace and the reprojected history. When the trace holds a quarter
// of them, a block's other three texels come from the history (clamped to the fresh texels around),
// or where it has none (the view turned, something came out from behind a mountain) from the fresh
// texels around, interpolated.
struct Resolved {
    @location(0) light: vec4<f32>,
    // The cover's optical depth along the texel's ray (`cover_tau`), for the composite.
    @location(1) tau: vec4<f32>,
};

@fragment
fn resolve(in: FullscreenVertexOutput) -> Resolved {
    var out: Resolved;
    let lp = vec2<i32>(in.position.xy);
    out.light = resolve_light(lp);
    out.tau = vec4<f32>(0.0);
    // (The near pass's clouds keep their noise: the composite doesn't sharpen them.)
    if cs.params.w < 1.5 && out.light.a < 0.999 {
        let dir = ray_dir(cloud_uv(lp));
        let px = length(ray_dir(cloud_uv(lp + vec2<i32>(1, 0))) - dir);
        out.tau.x = cover_tau(dir, px, px).x;
    }
    return out;
}

fn resolve_light(lp: vec2<i32>) -> vec4<f32> {
    let last = vec2<i32>(cs.trace.xy) - 1;
    var tp = lp;
    var fresh = true;
    var cur: vec4<f32>;
    if quarter() {
        let ph = phase_offset();
        tp = min(lp / 2, last);
        fresh = all(lp == tp * 2 + ph);
        if fresh {
            cur = textureLoad(trace_tex, tp, 0);
        } else {
            let f = clamp(vec2<f32>(lp - ph) * 0.5, vec2<f32>(0.0), vec2<f32>(last));
            cur = textureSampleLevel(trace_tex, clamp_smp, (f + 0.5) / vec2<f32>(textureDimensions(trace_tex)), 0.0);
        }
    } else {
        cur = textureLoad(trace_tex, lp, 0);
    }
    if cs.params.y > 0.5 || cv.cam2.w > 0.5 {
        return cur;
    }
    let dir = ray_dir(cloud_uv(lp));
    // A ray that misses the clouds' shell and the air's weather under it (the dust's 8 km) is clear,
    // whatever the history or the neighbours say (they once hung the clouds' ghosts in space by a
    // planet's edge).
    if cv.giant.x < 0.5 {
        let ds = (cv.sphere_from_world * vec4<f32>(dir, 0.0)).xyz;
        let s = sphere_hits(max(cv.atm_flags.w, 8000.0) + 500.0, dot(ds, ds), dot(cv.cam.xyz, ds));
        if s.x > s.y || s.y <= 0.0 {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
    }
    // The fresh neighbours' range, of those whose scene lies within 10 % of this texel's distance
    // (blackrack's: across a mountain's edge the sky's clouds once bled into its slope's), all of them
    // if none does. (Clear ones stay in it: the range widens to clear beside clear neighbours.)
    let own = min(textureLoad(low_depth, lp, 0).r, FAR * 0.5);
    let ph = select(vec2<i32>(0), phase_offset(), quarter());
    let scale = select(1, 2, quarter());
    var lo = vec4<f32>(1e30);
    var hi = vec4<f32>(-1e30);
    var lo_all = vec4<f32>(1e30);
    var hi_all = vec4<f32>(-1e30);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let q = clamp(tp + vec2<i32>(i, j), vec2<i32>(0), last);
            let n = textureLoad(trace_tex, q, 0);
            lo_all = min(lo_all, n);
            hi_all = max(hi_all, n);
            let nd = min(textureLoad(low_depth, min(q * scale + ph, vec2<i32>(cs.size.zw) - 1), 0).r, FAR * 0.5);
            if abs(nd - own) <= 0.1 * max(min(nd, own), 1.0) {
                lo = min(lo, n);
                hi = max(hi, n);
            }
        }
    }
    if lo.a > hi.a {
        lo = lo_all;
        hi = hi_all;
    }
    let dist = textureLoad(trace_dist_tex, tp, 0).r;
    // Near clouds drift with the wind, so last frame they were upwind of here; far ones (the cover's
    // average, `layer_density`'s `far`) stay put.
    let fp = dist * length(ray_dir(cloud_uv(lp + vec2<i32>(1, 0))) - dir);
    let p = dir * dist - cv.motion.xyz * (1.0 - far_share(fp, cv.layer_a[0].w));
    let prev = cv.prev_clip_from_world * vec4<f32>(p, 1.0);
    if prev.w <= 0.0 {
        return cur;
    }
    let prev_uv = vec2<f32>(prev.x / prev.w * 0.5 + 0.5, 0.5 - prev.y / prev.w * 0.5);
    if any(prev_uv < vec2<f32>(0.0)) || any(prev_uv > vec2<f32>(1.0)) {
        return cur;
    }
    // The previous frame's cloud texel under that point (the buffer covers a whole number of blocks).
    let huv = prev_uv * cs.size.xy / (f32(divisor()) * vec2<f32>(textureDimensions(history_tex)));
    if textureSampleLevel(history_tex, clamp_smp, huv, 0.0).a < 0.0 {
        return cur;
    }
    let hist = clamp(history_at(huv), lo, hi);
    if !fresh {
        return hist;
    }
    // (A quarter's texel is fresh every fourth frame: its history counts for a little less.)
    return mix(cur, hist, select(0.9, 0.85, quarter()));
}

// The history at uv through a 5-tap Catmull-Rom filter (Jimenez's; as blackrack's): sharper than the
// bilinear one, which blurred moving clouds a little more each frame.
fn history_at(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(history_tex));
    let sp = uv * size;
    let t1 = floor(sp - 0.5) + 0.5;
    let f = sp - t1;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (t1 - 1.0) / size;
    let t3 = (t1 + 2.0) / size;
    let t12 = (t1 + w2 / w12) / size;
    let c = textureSampleLevel(history_tex, clamp_smp, vec2<f32>(t12.x, t0.y), 0.0) * (w12.x * w0.y)
        + textureSampleLevel(history_tex, clamp_smp, vec2<f32>(t0.x, t12.y), 0.0) * (w0.x * w12.y)
        + textureSampleLevel(history_tex, clamp_smp, t12, 0.0) * (w12.x * w12.y)
        + textureSampleLevel(history_tex, clamp_smp, vec2<f32>(t3.x, t12.y), 0.0) * (w3.x * w12.y)
        + textureSampleLevel(history_tex, clamp_smp, vec2<f32>(t12.x, t3.y), 0.0) * (w12.x * w3.y);
    return c / (w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y);
}

// The cover's optical depth along a ray (world direction dir): each layer's cover where the ray crosses
// its clouds' mean height, as the far clouds' mean (`layer_density`'s `avg`) gives it through the
// layer, read with the footprint of a pixel of angle px. x the optical depth, y how far the clouds
// there are from their noise (`layer_density`'s `far`, for pixels of angle far_px) and steep enough
// a crossing: the composite's guide.
fn cover_tau(dir: vec3<f32>, px: f32, far_px: f32) -> vec2<f32> {
    let d = (cv.sphere_from_world * vec4<f32>(dir, 0.0)).xyz;
    let a = dot(d, d);
    let b = dot(cv.cam.xyz, d);
    var tau = 0.0;
    var guide = 1.0;
    var hit = false;
    for (var i = 0; i < LAYERS; i++) {
        let la = cv.layer_a[i];
        if la.z <= 0.0 {
            continue;
        }
        let s = sphere_hits(mix(la.x, la.y, 0.25 + 0.25 * (1.0 - la.w)), a, b);
        if s.x > s.y || s.y <= 0.0 {
            continue;
        }
        let t = select(s.y, s.x, s.x > 0.0);
        let x = cv.cam.xyz + d * t;
        let wx = weather_at(x);
        tau += -log(1.0 - 0.995 * weather_cover(cover_at(i, x, t * px), wx.x, wx.y, la.z));
        let steep = abs(dot(normalize(x), normalize(d)));
        guide = min(guide, far_share(t * far_px, la.w) * smoothstep(0.15, 0.35, steep));
        hit = true;
    }
    return vec2<f32>(tau, select(0.0, guide, hit));
}

@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    let uv = solid_uv(in.position.xy);
    let dist = scene_dist(uv, depth_at(p));
    let div = f32(divisor());
    let c = in.position.xy / div - 0.5;
    let base = vec2<i32>(floor(c));
    let f = c - floor(c);
    let last = vec2<i32>(cs.size.zw) - 1;
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    var qs: array<vec2<i32>, 4>;
    var ws: array<f32, 4>;
    var rs: array<vec4<f32>, 4>;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1u);
        let q = clamp(base + o, vec2<i32>(0), last);
        let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let ld = textureLoad(low_depth, q, 0).r;
        let rel = abs(min(ld, FAR * 0.5) - min(dist, FAR * 0.5)) / max(min(dist, ld), 1.0);
        let w = (bw + 1e-3) / (1e-3 + rel);
        let r = textureLoad(resolved_tex, q, 0);
        acc += w * r;
        wsum += w;
        qs[k] = q;
        ws[k] = w;
        rs[k] = r;
    }
    var c4 = acc / max(wsum, 1e-8);
    // Far away the clouds are their cover's mean, so their opacity follows the cover where each pixel's
    // ray crosses the layers, finer than the cloud texels: the texels' optical depth per unit of the
    // cover's (`cover_tau`) and light per unit of opacity, carried to this pixel's own cover (edges and
    // wisps as sharp as the cover is).
    if cs.params.w < 1.5 && c4.a < 0.999 {
        let dir = ray_dir(uv);
        let px = length(ray_dir(solid_uv(in.position.xy + vec2<f32>(1.0, 0.0))) - dir);
        let g = cover_tau(dir, px, px * div);
        if g.y > 0.0 {
            var kd = 0.0;
            var s = vec3<f32>(0.0);
            var kw = 0.0;
            for (var k = 0; k < 4; k++) {
                let tq = textureLoad(tau_tex, qs[k], 0).r;
                let tr = clamp(rs[k].a, 0.0, 1.0);
                if tq > 0.02 && tr < 0.995 {
                    kd += ws[k] * -log(max(tr, 1e-4)) / tq;
                    s += ws[k] * rs[k].rgb / (1.0 - tr);
                    kw += ws[k];
                }
            }
            if kw > 0.0 {
                let tr = exp(-kd / kw * g.x);
                c4 = mix(c4, vec4<f32>(s / kw * (1.0 - tr), tr), g.y * min(2.0 * kw / wsum, 1.0));
            }
        }
    }
    return vec4<f32>(c4.rgb, clamp(c4.a, 0.0, 1.0));
}

fn hash3(p: vec3<f32>) -> vec3<f32> {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yxz + 33.33);
    return fract((q.xxy + q.yxx) * q.zyx);
}

// The weather near the camera, over the scene and its clouds at full resolution: lightning's light
// on the ground around the flashes, and falling rain (streaks) or snow (flakes) in four layers 2 to
// 16 m away when the camera is under a raining cloud.
@fragment
fn weather_fx(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var light = vec3<f32>(0.0);
    var trans = 1.0;
    if cv.params.x < 0.5 {
        return vec4<f32>(light, trans);
    }
    let pix = vec2<i32>(in.position.xy);
    let uv = solid_uv(in.position.xy);
    // The weather map (tuning): red rain, green convection, blue fog, orange dust, cyan the flashes, a
    // white dot at the camera; equirectangular, north up.
    if cv.fx2.w > 0.5 {
        let mw = 0.3 * cs.size.x;
        let m0 = vec2<f32>(0.35 * cs.size.x, 0.02 * cs.size.y);
        let mp = (in.position.xy - m0) / vec2<f32>(mw, mw * 0.5);
        if all(mp >= vec2<f32>(0.0)) && all(mp <= vec2<f32>(1.0)) {
            let lon = mp.x * 2.0 * PI - PI;
            let lat = 0.5 * PI - mp.y * PI;
            let md = vec3<f32>(cos(lat) * cos(lon), sin(lat), -cos(lat) * sin(lon));
            let w = textureSampleLevel(weather_tex, clamp_smp, md, 0.0);
            let c = normalize(cv.cam.xyz);
            let cl = vec2<f32>((atan2(-c.z, c.x) + PI) / (2.0 * PI), (0.5 * PI - asin(clamp(c.y, -1.0, 1.0))) / PI);
            let dot_at = length((mp - cl) * vec2<f32>(mw, mw * 0.5)) < 4.0;
            let fxw = textureSampleLevel(fx_tex, clamp_smp, md, 0.0);
            var col = select(vec3<f32>(w.a + 0.8 * fxw.g, w.b + 0.45 * fxw.g, 0.15 + fxw.r), vec3<f32>(1.0), dot_at);
            // A gas giant: its decks' band colour instead.
            if cv.giant.x > 0.5 && !dot_at {
                col = textureSampleLevel(cover_tex, clamp_smp, md, 0.0).rgb;
            }
            // The flashes of the moment, cyan.
            for (var k = 0; k < 8; k++) {
                let f = flashes[k];
                if f.w > 0.0 {
                    let fd = normalize(f.xyz);
                    let fl = vec2<f32>((atan2(-fd.z, fd.x) + PI) / (2.0 * PI), (0.5 * PI - asin(clamp(fd.y, -1.0, 1.0))) / PI);
                    if length((mp - fl) * vec2<f32>(mw, mw * 0.5)) < 5.0 {
                        col = vec3<f32>(0.0, 1.0, 1.0);
                    }
                }
            }
            return vec4<f32>(col * cv.sun_light.rgb * 0.3, 0.0);
        }
    }
    let dir = ray_dir(uv);
    let dist = scene_dist(uv, depth_at(pix));
    let o = cv.cam.xyz;
    let ds = (cv.sphere_from_world * vec4<f32>(dir, 0.0)).xyz;
    let l_s = normalize((cv.sphere_from_world * vec4<f32>(normalize(cv.sun_dir.xyz), 0.0)).xyz);
    if cv.fx.y > 0.5 && dist < 1.0e7 {
        let xs = o + ds * dist;
        var e = 0.0;
        for (var k = 0; k < 8; k++) {
            let f = flashes[k];
            if f.w > 0.0 {
                let q = length(xs - f.xyz) / 4000.0;
                e += f.w / (1.0 + q * q);
            }
        }
        // As on a surface of albedo 0.15.
        light += FLASH_COLOR * (e * cv.fx2.z * 0.15 / PI);
    }
    let li = rain_layer();
    if cv.fx.x < 0.5 || li < 0 {
        return vec4<f32>(light, trans);
    }
    let pr = precip_at(o, cv.cam2.y, li, l_s);
    // Even light rain shows its drops.
    let amount = clamp(sqrt(pr.x / (2.5e-3 * select(1.0, 1.6, pr.y > 0.5))) * 1.2, 0.0, 1.0);
    if amount < 0.01 {
        return vec4<f32>(light, trans);
    }
    let snow = pr.y > 0.5;
    let up = normalize(o);
    let east = normalize(select(cross(vec3<f32>(0.0, 1.0, 0.0), up), vec3<f32>(1.0, 0.0, 0.0), abs(up.y) > 0.999));
    let north = cross(up, east);
    let v = normalize(ds);
    let el = dot(v, up);
    let hz = v - up * el;
    let hl = length(hz);
    if hl < 0.05 {
        return vec4<f32>(light, trans);
    }
    let az = atan2(dot(hz, north), dot(hz, east));
    // Lit mostly by the sky: the clouds overhead hide the star.
    let al = air_light(o, l_s, 1.0);
    let lum = select(vec3<f32>(0.5), vec3<f32>(0.95), snow) * (al.sun * 0.3 + al.sky);
    let px = length(ray_dir(solid_uv(in.position.xy + vec2<f32>(1.0, 0.0))) - dir);
    let clock = cv.fx2.x;
    let fall = select(7.0, 1.2, snow);
    let cell = select(vec2<f32>(0.22, 1.4), vec2<f32>(0.2, 0.2), snow);
    for (var k = 0; k < 4; k++) {
        let rk = 2.0 * exp2(f32(k));
        let tk = rk / hl;
        if tk > dist {
            continue;
        }
        var q = vec2<f32>(az * rk, el * tk + fall * clock);
        if snow {
            q.x += 0.12 * sin(clock * 1.3 + floor(q.y / cell.y) * 2.1);
        }
        let id = floor(q / cell);
        let f = q / cell - id;
        let hsh = hash3(vec3<f32>(id, f32(k) * 17.0));
        if hsh.x > amount * select(0.5, 0.8, snow) {
            continue;
        }
        let dx = abs(f.x - hsh.y) * cell.x;
        let dy = (f.y - hsh.z) * cell.y;
        var alpha = 0.0;
        if snow {
            let rad = max(0.006, px * tk * 0.7);
            alpha = smoothstep(rad, rad * 0.4, length(vec2<f32>(dx, dy))) * 0.8;
        } else {
            let w = max(0.002, px * tk * 0.8);
            alpha = smoothstep(w, 0.0, dx) * smoothstep(-0.02, 0.03, dy) * smoothstep(0.4, 0.25, dy) * 0.3;
        }
        alpha *= 1.0 - 0.15 * f32(k);
        light += trans * alpha * lum;
        trans *= 1.0 - alpha;
    }
    return vec4<f32>(light, trans);
}
