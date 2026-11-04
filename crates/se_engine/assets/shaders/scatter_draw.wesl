// The GPU scatter's draws (scatter/gpu.rs): the instances the cull pass appended (scatter.wesl),
// drawn as meshes (bark, alpha-tested leaves) or impostor quads from the baked atlas
// (scatter/impostor.rs). The LOD hand-over is a screen-door dither: the mesh draws where the pixel's
// dither is under its share of the band, the impostor elsewhere, so they never overlap or leave holes.
// They are lit as the ground under them (ground_light.wesl, planet.wesl lit_ground and finish): the
// same stars, eclipses, cloud shadows, sky light, ambient light and haze.

import super::atmosphere_common::{AtmParams, ATM_AMBIENT_SCALE, ATM_PI, atm_transmittance_sun, atm_irradiance, atm_apply_aerial};
import super::eclipse::eclipse_light;
import super::ground_light::{PatchLight, CloudShadow, cloud_shadow_at, horizon_shadow_at, ground_haze};

struct Body {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    centre: vec4<f32>,
    lod: vec4<f32>,
    lod2: vec4<f32>,
    counts: vec4<u32>,
    sun: vec4<f32>,
    sun_color: vec4<f32>,
    levels: vec4<f32>,
    atm_flags: vec4<f32>,
    // x, y the ground's cloud key (radius m, seed fraction); z 1 with atmosphere tables bound.
    key: vec4<f32>,
    atm: AtmParams,
    light: PatchLight,
}

struct Camera {
    clip_from_world: mat4x4<f32>,
    planes: array<vec4<f32>, 5>,
    viewport: vec4<f32>,
}

struct Inst {
    pos: vec3<f32>,
    fades: u32,
    xa: u32,
    up: u32,
    scale: f32,
    model: u32,
}

struct Model {
    half: f32,
    centre_y: f32,
    radius: f32,
    layer0: u32,
    crown_y: f32,
    crown_r: f32,
    mesh_bucket: u32,
    impostor_bucket: u32,
}

struct Material {
    tint: vec4<f32>,
    flags: vec4<f32>,
}

const PI: f32 = 3.14159265358979;
const AZIMUTHS: f32 = 8.0;
// The impostors' elevations (scatter/impostor.rs ELEVATION_DEG), in radians.
const EL1: f32 = 0.610865;
const EL2: f32 = 1.221730;

@group(0) @binding(0) var<uniform> cam: Camera;
@group(0) @binding(1) var<uniform> body: Body;
@group(0) @binding(2) var<storage, read> insts: array<Inst>;
@group(0) @binding(3) var<storage, read> models: array<Model>;

fn oct_decode(p: u32) -> vec3<f32> {
    let e = unpack2x16snorm(p);
    var v = vec3<f32>(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    if v.z < 0.0 {
        let xy = (1.0 - abs(v.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), v.xy >= vec2<f32>(0.0));
        v = vec3<f32>(xy, v.z);
    }
    return normalize(v);
}

// Interleaved gradient noise: a stable per-pixel threshold in 0..1.
fn dither(frag: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(floor(frag), vec2<f32>(0.06711056, 0.00583715))));
}

// A second, independent one (for the far fade and thinning).
fn dither2(frag: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(floor(frag) + vec2<f32>(17.0, 59.0), vec2<f32>(0.00583715, 0.06711056))));
}

@group(2) @binding(0) var inscatter_tex: texture_3d<f32>;
@group(2) @binding(1) var transmittance_tex: texture_2d<f32>;
@group(2) @binding(2) var irradiance_tex: texture_2d<f32>;
@group(2) @binding(3) var atm_smp: sampler;
@group(2) @binding(4) var cloud_cover: texture_cube<f32>;
@group(2) @binding(5) var cloud_cover_smp: sampler;
@group(2) @binding(6) var<storage, read> cloud_sh: CloudShadow;
@group(2) @binding(7) var cloud_map: texture_2d<f32>;
@group(2) @binding(8) var cloud_map_smp: sampler;
@group(2) @binding(9) var cloud_weather: texture_cube<f32>;
@group(2) @binding(10) var cloud_pattern: texture_cube<f32>;

// The light at a point that does not depend on the surface's normal: worked out per vertex (the
// atmosphere tables, the clouds' shadow and the haze barely change across a tree), so the pixels
// only add the normal's part.
struct PointLight {
    // The brightest star's direct light at the point: shadowed (eclipses, clouds), through the
    // atmosphere, in the ground's scale.
    sun: vec3<f32>,
    // Its sky light (times 0.5 + 0.5 n.up in the pixel).
    sky: vec3<f32>,
    // Day ambient, the further stars (taken on level ground) and SE's AmbientLighting.
    flat_e: vec3<f32>,
    // The haze between the point and the camera: transmittance and inscattered light.
    haze_t: vec3<f32>,
    haze_a: vec3<f32>,
}

// One star's light on level ground at the point, as lit_ground gives the ground: (direct through
// the atmosphere, sky light) for irradiance `e`.
fn star_terms(up: vec3<f32>, pos_km: vec3<f32>, height: f32, l: vec3<f32>, e: vec3<f32>, has_atm: bool) -> array<vec3<f32>, 2> {
    if has_atm {
        let r = length(pos_km);
        let mu_s = dot(pos_km / r, l);
        let t_sun = atm_transmittance_sun(body.atm, transmittance_tex, atm_smp, r, mu_s);
        let e_sky = atm_irradiance(body.atm, irradiance_tex, atm_smp, r, mu_s) * (body.atm.look.w * ATM_PI * ATM_AMBIENT_SCALE);
        return array<vec3<f32>, 2>(e * t_sun, e * e_sky);
    }
    return array<vec3<f32>, 2>(e * horizon_shadow_at(body.centre.w, height, dot(up, l)), vec3<f32>(0.0));
}

// The normal-free light at `world` (render frame, the camera at the origin), as the ground there
// takes it (planet.wesl lit_ground and finish).
fn point_light(world: vec3<f32>) -> PointLight {
    let centre = body.centre.xyz;
    let radius = body.centre.w;
    let rel = world - centre;
    // The body's axes and the ellipsoid's scale on them: its sphere frame, as the ground's table_local.
    let ax = normalize(body.m0.xyz);
    let ay = normalize(body.m1.xyz);
    let az = normalize(body.m2.xyz);
    let alen = vec3<f32>(length(body.m0.xyz), length(body.m1.xyz), length(body.m2.xyz));
    let local = vec3<f32>(dot(rel, ax) / alen.x, dot(rel, ay) / alen.y, dot(rel, az) / alen.z);
    let bdir = normalize(local);
    let height = length(local) - radius;
    let up = normalize(rel);
    let p_rel = rel / radius;
    let l = normalize(body.light.sun_dir.xyz);
    let has_atm = body.atm_flags.x > 0.5 && body.key.z > 0.5;
    let pos_km = up * body.atm.radii.x * (1.0 + max(height, 0.0) / radius);
    // At the height the ground gives its cloud shadow (lit_ground: the radial distance in the render
    // frame, the ellipsoid's), not the sphere frame's: on a flattened planet they differ by up to
    // flattening x radius.
    let clouds = cloud_shadow_at(cloud_sh, cloud_cover, cloud_cover_smp, cloud_map, cloud_map_smp, cloud_weather, cloud_pattern, radius, body.key.xy, bdir, length(rel) - radius);
    let shadow = eclipse_light(body.light.eclipse[0], p_rel, l) * clouds;
    let e_sun = body.light.sun_color.rgb * body.light.sun_dir.w * shadow * body.atm_flags.y;
    var o: PointLight;
    let t0 = star_terms(up, pos_km, height, l, e_sun, has_atm);
    o.sun = t0[0];
    o.sky = t0[1];
    // SE's day ambient (DayAmbient) on the day side, and its AmbientLighting.
    o.flat_e = e_sun * (PI * ATM_AMBIENT_SCALE * body.atm_flags.w * max(dot(up, l), 0.0)) + vec3<f32>(body.light.levels.x * body.atm_flags.y);
    // The further stars (no cloud shadow, as the ground's), on level ground.
    for (var i = 0; i < 3; i++) {
        if body.light.light_dir[i].w < 0.5 {
            break;
        }
        let lx = normalize(body.light.light_dir[i].xyz);
        let e_x = body.light.light_color[i].rgb * eclipse_light(body.light.eclipse[i + 1], p_rel, lx) * body.atm_flags.y;
        let tx = star_terms(up, pos_km, height, lx, e_x, has_atm);
        o.flat_e += tx[0] * max(dot(up, lx), 0.0) + tx[1] + e_x * (PI * ATM_AMBIENT_SCALE * body.atm_flags.w * max(dot(up, lx), 0.0));
    }
    o.haze_t = vec3<f32>(1.0);
    o.haze_a = vec3<f32>(0.0);
    if has_atm {
        let to_tables = body.atm.radii.x / max(radius, 1e-30);
        let c = -centre;
        let cam_km = vec3<f32>(dot(c, ax) / alen.x, dot(c, ay) / alen.y, dot(c, az) / alen.z) * to_tables;
        var view_km = local * to_tables;
        let r_ground = body.atm.radii.x * (1.0 + height / max(radius, 1.0));
        view_km *= max(r_ground / max(length(view_km), 1e-6), 1.0);
        o.haze_a = ground_haze(body.atm, inscatter_tex, transmittance_tex, atm_smp, vec3<f32>(0.0), cam_km, view_km, ax, ay, az, l, e_sun, body.light, body.atm_flags);
        o.haze_t = atm_apply_aerial(body.atm, inscatter_tex, transmittance_tex, atm_smp, vec3<f32>(1.0), cam_km, view_km, l, vec3<f32>(0.0));
    }
    return o;
}

// A surface's colour from its point's light, albedo and normal `n`: `wrap` softens the star's term
// (leaves), `trans` adds the light through them.
fn surface_color(pl: PointLight, albedo: vec3<f32>, n: vec3<f32>, up: vec3<f32>, wrap: f32, trans: f32) -> vec3<f32> {
    let l = normalize(body.light.sun_dir.xyz);
    let direct = max((dot(n, l) + wrap) / (1.0 + wrap), 0.0) + trans;
    let e = pl.sun * direct + pl.sky * (0.5 + 0.5 * dot(n, up)) + pl.flat_e;
    return albedo / PI * e * pl.haze_t + pl.haze_a;
}

// Leaves let the star through when it is behind them (seen from `world`).
fn leaf_trans(world: vec3<f32>) -> f32 {
    let v = normalize(-world);
    return 0.35 * pow(max(dot(-v, normalize(body.light.sun_dir.xyz)), 0.0), 4.0);
}

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) fades: vec2<f32>,
    @location(4) @interpolate(flat) up: vec3<f32>,
    @location(5) ball: vec3<f32>,
    @location(6) l_sun: vec3<f32>,
    @location(7) l_sky: vec3<f32>,
    @location(8) l_flat: vec3<f32>,
    @location(9) haze_t: vec3<f32>,
    @location(10) haze_a: vec3<f32>,
}

struct VertexIn {
    @builtin(instance_index) instance: u32,
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

@if(!IMPOSTOR)
@vertex
fn vertex_mesh(v: VertexIn) -> MeshOut {
    let inst = insts[v.instance];
    let md = models[inst.model];
    let up = oct_decode(inst.up);
    let xa = oct_decode(inst.xa);
    let za = cross(xa, up);
    let basis = mat3x3<f32>(xa, up, za);
    let world = inst.pos + basis * (v.pos * inst.scale);
    var o: MeshOut;
    o.clip = cam.clip_from_world * vec4<f32>(world, 1.0);
    o.world = world;
    o.normal = basis * v.normal;
    o.uv = v.uv;
    o.fades = unpack2x16unorm(inst.fades);
    o.up = up;
    // The crown shades as a ball (Megaton Rainfall's leaves): the direction from its centre.
    o.ball = basis * ((v.pos - vec3<f32>(0.0, md.crown_y, 0.0)) / max(md.crown_r, 0.1));
    let pl = point_light(world);
    o.l_sun = pl.sun;
    o.l_sky = pl.sky;
    o.l_flat = pl.flat_e;
    o.haze_t = pl.haze_t;
    o.haze_a = pl.haze_a;
    return o;
}

@if(!IMPOSTOR) @group(1) @binding(0) var color_tex: texture_2d<f32>;
@if(!IMPOSTOR) @group(1) @binding(1) var color_smp: sampler;
@if(!IMPOSTOR) @group(1) @binding(2) var<uniform> material: Material;

@if(!IMPOSTOR)
@fragment
fn fragment_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // The mesh's share of the LOD band.
    if dither(in.clip.xy) >= in.fades.y {
        discard;
    }
    let tex = textureSample(color_tex, color_smp, in.uv);
    var n = normalize(in.normal);
    var wrap = 0.0;
@if(LEAVES) {
    if tex.a < 0.5 {
        discard;
    }
    if !front {
        n = -n;
    }
    n = normalize(n * 0.3 + normalize(in.ball + vec3<f32>(1e-5)) * 0.7);
    wrap = 0.5;
}
    let albedo = tex.rgb * material.tint.rgb;
    let pl = PointLight(in.l_sun, in.l_sky, in.l_flat, in.haze_t, in.haze_a);
    var trans = 0.0;
    if wrap > 0.0 {
        trans = leaf_trans(in.world);
    }
    return vec4<f32>(surface_color(pl, albedo, n, in.up, wrap, trans), 1.0);
}

// ---- impostors -------------------------------------------------------------------------------

struct ImpostorOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) quad: vec2<f32>,
    @location(1) @interpolate(flat) fades: vec2<f32>,
    @location(2) @interpolate(flat) up: vec3<f32>,
    // The atlas views to blend: azimuth pair, elevation pair (layers), and their weights.
    @location(3) @interpolate(flat) layers: vec4<u32>,
    @location(4) @interpolate(flat) weights: vec2<f32>,
    @location(5) @interpolate(flat) xa: vec3<f32>,
    @location(6) world: vec3<f32>,
    @location(7) l_sun: vec3<f32>,
    @location(8) l_sky: vec3<f32>,
    @location(9) l_flat: vec3<f32>,
    @location(10) haze_t: vec3<f32>,
    @location(11) haze_a: vec3<f32>,
}

@if(IMPOSTOR)
@vertex
fn vertex_impostor(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> ImpostorOut {
    let inst = insts[ii];
    let md = models[inst.model];
    let up = oct_decode(inst.up);
    let xa = oct_decode(inst.xa);
    let za = cross(xa, up);
    let s = inst.scale;
    var centre = inst.pos + up * md.centre_y * s;
    let f = normalize(-centre);
    var right = cross(up, f);
    if length(right) < 1e-4 {
        right = xa;
    }
    right = normalize(right);
    let bup = cross(f, right);
    let corner = vec2<f32>(f32(vi & 1u) * 2.0 - 1.0, f32((vi >> 1u) & 1u) * 2.0 - 1.0);
    // Pulled towards the camera by its half size so the ground behind never cuts it.
    let dist = length(centre);
    let pull = min(md.half * s, dist * 0.5);
    let pulled = centre + f * pull;
    // Shrunk as much as it is pulled, so it covers the angle the tree does.
    let world = pulled + (right * corner.x + bup * corner.y) * md.half * s * (dist - pull) / max(dist, 1e-3);
    // The view of the model the camera has: azimuth about its up from +z towards +x, elevation.
    let lx = dot(f, xa);
    let ly = dot(f, up);
    let lz = dot(f, za);
    var az = atan2(lx, lz) / (2.0 * PI) * AZIMUTHS;
    az = az - floor(az / AZIMUTHS) * AZIMUTHS;
    let a0 = u32(floor(az)) % 8u;
    let a1 = (a0 + 1u) % 8u;
    let el = asin(clamp(ly, -1.0, 1.0));
    var e0 = 0u;
    var we = 0.0;
    if el > EL1 {
        e0 = 1u;
        we = clamp((el - EL1) / (EL2 - EL1), 0.0, 1.0);
    } else {
        we = clamp(el / EL1, 0.0, 1.0);
    }
    let e1 = min(e0 + 1u, 2u);
    var o: ImpostorOut;
    o.clip = cam.clip_from_world * vec4<f32>(world, 1.0);
    o.quad = vec2<f32>(corner.x * 0.5 + 0.5, 0.5 - corner.y * 0.5);
    o.fades = unpack2x16unorm(inst.fades);
    o.up = up;
    o.layers = vec4<u32>(md.layer0 + e0 * 8u + a0, md.layer0 + e0 * 8u + a1, md.layer0 + e1 * 8u + a0, md.layer0 + e1 * 8u + a1);
    o.weights = vec2<f32>(fract(az), we);
    o.xa = xa;
    o.world = world;
    // The light at the tree's centre (the same for its four corners).
    let pl = point_light(centre);
    o.l_sun = pl.sun;
    o.l_sky = pl.sky;
    o.l_flat = pl.flat_e;
    o.haze_t = pl.haze_t;
    o.haze_a = pl.haze_a;
    return o;
}

@if(IMPOSTOR) @group(1) @binding(0) var imp_color: texture_2d_array<f32>;
@if(IMPOSTOR) @group(1) @binding(1) var imp_normal: texture_2d_array<f32>;
@if(IMPOSTOR) @group(1) @binding(2) var imp_smp: sampler;

@if(IMPOSTOR)
@fragment
fn fragment_impostor(in: ImpostorOut) -> @location(0) vec4<f32> {
    // The impostor's share of the LOD band, and the far fade and thinning.
    if dither(in.clip.xy) < in.fades.y || dither2(in.clip.xy) >= in.fades.x {
        discard;
    }
    let w = in.weights;
    let ws = vec4<f32>((1.0 - w.x) * (1.0 - w.y), w.x * (1.0 - w.y), (1.0 - w.x) * w.y, w.x * w.y);
    var c = vec4<f32>(0.0);
    var nl = vec3<f32>(0.0);
    for (var k = 0u; k < 4u; k++) {
        if ws[k] > 0.001 {
            c += textureSample(imp_color, imp_smp, in.quad, in.layers[k]) * ws[k];
            nl += (textureSample(imp_normal, imp_smp, in.quad, in.layers[k]).xyz * 2.0 - 1.0) * ws[k];
        }
    }
    if c.a < 0.5 {
        discard;
    }
    let up = in.up;
    let xa = in.xa;
    let za = cross(xa, up);
    let n = normalize(mat3x3<f32>(xa, up, za) * nl + vec3<f32>(1e-6));
    let pl = PointLight(in.l_sun, in.l_sky, in.l_flat, in.haze_t, in.haze_a);
    // The bake stores straight colour (spread into the clear texels), not premultiplied.
    return vec4<f32>(surface_color(pl, c.rgb, n, up, 0.5, leaf_trans(in.world)), 1.0);
}
