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

// The vertical profile of a layer at height fraction hf: cumuliform clouds round at the base and
// taper to the top, stratiform ones fill the layer with soft edges.
fn profile(hf: f32, cumulus: f32) -> f32 {
    let strat = smoothstep(0.0, 0.2, hf) * smoothstep(1.0, 0.7, hf);
    let cumu = smoothstep(0.0, 0.08, hf) * smoothstep(1.0, 0.35, hf);
    return mix(strat, cumu, cumulus);
}

// Layer i's density at the sphere-space point x seen with a footprint of fp m. The cover is the
// flat shells' opacity, so the clouds keep it as their mean opacity seen from above: they fill an
// area share f with an optical depth inside them that gives a mean transmittance of 1 - cover. A
// cumuliform layer's clouds are solid, over the cover's share of the sky, dense inside with crisp
// edges; a stratiform layer's are veils over the whole area, textured by the shape noise. A cumuliform layer's clouds rise higher where
// the cover is denser and spread into sheets under a full overcast. Beyond the noise's resolution
// only the mean is left (the flat shells' look from afar). `erode` false leaves the detail noise out
// (the light march's farther samples).
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
    // Fair-weather cumulus about a kilometre deep, towers only under a dense cover.
    // Convection raises towers out of a thinner cover.
    let top = a.x + (a.y - a.x) * mix(1.0, 0.12 + 0.88 * max(cover * cover * cover, wx.z * 0.9), a.w);
    let hf = (h - a.x) / (top - a.x);
    if hf >= 1.0 {
        return 0.0;
    }
    // A full cover is a flat sheet, unless it is thunderstorms: they stay heaped, towers with gaps.
    let cumulus = a.w * max(1.0 - smoothstep(0.8, 0.98, cover), wx.z);
    let prof = profile(hf, cumulus);
    let solid = a.w * smoothstep(0.02, 0.25, cover);
    let share = mix(1.0, cover, solid);
    let column = 0.6 * max(top - a.x, 1.0) * cv.layer_b[i].z;
    // Inside the clouds, and averaged over the whole area (seen from afar).
    let inside = -log(max(1.0 - min(cover / share, 0.995), 0.005)) / column;
    let avg = -log(1.0 - 0.995 * cover) / column * prof;
    // (The shape noise's 1.5 km cells are averaged out before they shrink to a few pixels: from
    // orbit they showed as a fine honeycomb over thin cover.)
    let far = smoothstep(0.1, 0.5, fp * cv.params.y);
    var near = avg;
    if far < 1.0 {
        let s = textureSampleLevel(shape_tex, repeat_smp, fract((x - cv.noise_off.xyz) * cv.params.y), lod_for(fp, cv.params.y, 128.0));
        let fbm = s.g * 0.625 + s.b * 0.25 + s.a * 0.125;
        let base = clamp(remap(s.r, fbm - 1.0, 1.0, 0.0, 1.0), 0.0, 1.0);
        // The shape noise thresholded to the area share (SE's cover over the Perlin-Worley shape),
        // its edges eroded by the detail noise; normalised to a mean of about one.
        var m = remap(mix(base, base * prof, solid), 1.0 - share, 1.0, 0.0, 1.0);
        // The erosion only where its 900 m cells are larger than the footprint (else averaged out).
        if erode && m > 0.0 && fp * cv.params.z < 0.5 {
            let e = textureSampleLevel(detail_tex, repeat_smp, fract((x - cv.detail_off.xyz) * cv.params.z), lod_for(fp, cv.params.z, 32.0));
            let efbm = e.r * 0.625 + e.g * 0.25 + e.b * 0.125;
            m = remap(m, efbm * (0.15 + 0.25 * solid) * (1.0 - hf * 0.5), 1.0, 0.0, 1.0);
        }
        // A near-overcast closes smoothly (its last holes would be pinpricks).
        m = mix(clamp(m * mix(2.0, 4.0, solid), 0.0, 1.0), 1.0, smoothstep(0.75, 0.95, cover) * (1.0 - wx.z)) / mix(0.8, 0.9, solid);
        near = inside * mix(prof, 1.0, solid) * m;
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
    if wx.y > 0.02 {
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
            wx.y = select(0.5 * wx.y, 1.0, wx.y > 0.02);
            read = true;
        }
        if weather_cover(cover_at(i, x, fp), wx.x, wx.y, a.z) > 0.01 {
            return true;
        }
    }
    return false;
}

// The share of a layer's profile above height fraction hf (its integral from hf to 1, Simpson's rule).
fn profile_above(hf: f32, cumulus: f32) -> f32 {
    let m = 0.5 * (hf + 1.0);
    return (1.0 - hf) * (profile(hf, cumulus) + 4.0 * profile(m, cumulus)) / 6.0;
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
        let top = a.x + (a.y - a.x) * mix(1.0, 0.12 + 0.88 * max(cover * cover * cover, wx.z * 0.9), a.w);
        let hf = clamp((h - a.x) / (top - a.x), 0.0, 1.0);
        let cumulus = a.w * max(1.0 - smoothstep(0.8, 0.98, cover), wx.z);
        od += -log(1.0 - 0.995 * cover) * profile_above(hf, cumulus) / 0.6;
    }
    return od * slant;
}

fn hg(cos_t: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cos_t, 1e-4), 1.5));
}

// ---------------------------------------------------------------------------------------------
// The weather in the air.

const FLASH_COLOR: vec3<f32> = vec3<f32>(0.75, 0.82, 1.0);
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
    // steps (the first quarter of the steps covers an eighth of the path).
    let dt0 = max(total / select(f32(MAX_STEPS), NEAR_STEPS, mode > 1.5), 50.0);
    // Longer steps where a sample covers more cloud (far away), at least 24 over the path (fair-weather
    // cumulus fill only the layer's lowest kilometre or so: fewer steps over it thinned them).
    let dt_cap = max(dt0, total / 24.0);
    let has_atm = cv.atm_flags.x > 0.5;
    let to_km = cv.atm.radii.x / cv.cam.w;
    // A cloud pixel's angular size: the footprint of a sample is its distance times this.
    let px_angle = length(ray_dir(cloud_uv(lp + vec2<i32>(1, 0))) - dir);
    var light = vec3<f32>(0.0);
    var trans = 1.0;
    var t_sum = 0.0;
    var w_sum = 0.0;
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
    for (var i = 0; i < MAX_STEPS + 48; i++) {
        if t >= total || trans < 0.005 || fine >= MAX_STEPS {
            break;
        }
        let tw = select(seg1.x + (t - len0), seg0.x + t, t < len0);
        let x = o + d * tw;
        let fp = max(tw * px_angle, dt0 * 0.25);
        let dt = clamp(0.4 * tw * px_angle, dt0, dt_cap);
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
        } else {
            empty = 0;
            let li = smp.layer;
            let r = length(x);
            let up = x / r;
            let mu = dot(up, l_s);
            // The light towards the star: near the camera a march, steps doubling from 100 m over 3.1 km
            // (the first two with the detail noise, the rest without), then the cloud above's mean
            // (`light_column`) beyond it; from 20 to 40 km on the mean alone (the march's
            // self-shadows are smaller than a pixel there).
            var od = 0.0;
            let lw = select(smoothstep(20000.0, 40000.0, tw), 0.0, cv.giant.x > 0.5);
            if lw < 1.0 {
                var ls = 100.0;
                var lx = x;
                for (var k = 0; k < LIGHT_STEPS; k++) {
                    lx += l_s * ls;
                    od += sample_with(lx, max(fp, ls), smp.wx, k < 2).sigma * ls;
                    ls *= 2.0;
                }
                if cv.giant.x < 0.5 {
                    od += light_column(lx, max(fp, 3100.0), dot(normalize(lx), l_s), smp.wx);
                }
            }
            if lw > 0.0 {
                od = mix(od, light_column(x, fp, mu, smp.wx), lw);
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
            // The star's light scattered here: the direct beam (forward and back lobes), one octave of
            // weaker extinction and flatter phase for the first bounces (Wrenninge), and the diffuse
            // light deep in the cloud, as a thick conservative slab transmits it (two-stream:
            // 1 / (1 + 3/4 (1 - g) tau), g 0.85), radiating evenly: a cloud top is as bright as a white
            // Lambert surface, a thick cloud's base a sixth of that. The slab takes the star's light as
            // a horizontal surface does (by the sun's height), towers' sides some of it at any height.
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
            let direct = mix(hg(cos_t, 0.8), hg(cos_t, -0.25), 0.35) * exp(-od)
                + 0.5 * mix(hg(cos_t, 0.4), hg(cos_t, -0.12), 0.35) * exp(-od * 0.35);
            let flux = clamp(mu * 1.2 + 0.15 * tw_l, 0.0, 1.0);
            // (A giant's deep decks under a bright top: a white surface's light at the top.)
            let ms = direct + select(0.8, 1.2, cv.giant.x > 0.5) * flux / (PI * (1.0 + 0.11 * od_d)) * absorb;
            // Sky light from above, more of it near the tops; SE's day ambient as the flat shells.
            let la = cv.layer_a[li];
            let hf = clamp((r - cv.cam.w - la.x) / max(la.y - la.x, 1.0), 0.0, 1.0);
            let e_top = cv.sun_light.rgb * cv.sun_dir.w;
            let ambient = e_top * ATM_AMBIENT_SCALE * (max(mu, 0.0) * cv.layer_c[li].w + sky) * (0.35 + 0.65 * hf) / PI * 2.0 * absorb;
            let albedo = smp.albedo;
            let src = albedo * (e_sun * ms + ambient + lightning_at(x)) * smp.sigma;
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
    var lo = vec4<f32>(1e30);
    var hi = vec4<f32>(-1e30);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let n = textureLoad(trace_tex, clamp(tp + vec2<i32>(i, j), vec2<i32>(0), last), 0);
            lo = min(lo, n);
            hi = max(hi, n);
        }
    }
    let dist = textureLoad(trace_dist_tex, tp, 0).r;
    // Near clouds drift with the wind, so last frame they were upwind of here; far ones (the cover's
    // average, `layer_density`'s `far`) stay put.
    let fp = dist * length(ray_dir(cloud_uv(lp + vec2<i32>(1, 0))) - dir);
    let p = dir * dist - cv.motion.xyz * (1.0 - smoothstep(0.1, 0.5, fp * cv.params.y));
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
    var hist = textureSampleLevel(history_tex, clamp_smp, huv, 0.0);
    if hist.a < 0.0 {
        return cur;
    }
    hist = clamp(hist, lo, hi);
    if !fresh {
        return hist;
    }
    // (A quarter's texel is fresh every fourth frame: its history counts for less.)
    return mix(cur, hist, select(0.9, 0.8, quarter()));
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
        guide = min(guide, smoothstep(0.1, 0.5, t * far_px * cv.params.y) * smoothstep(0.15, 0.35, steep));
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
