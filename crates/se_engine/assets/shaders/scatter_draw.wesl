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
    // x how plants are decided (scatter.rs VegKind: 0 none, 1 the test, 2 SE's terra bands), y the
    // noises' footprint on the unit sphere
    veg: vec4<f32>,
    // towards the star in the body frame
    sun_body: vec4<f32>,
    // SE's climate parameters (surface_tex.rs climate_uniform 0-7) and the biomes' flags
    climate: array<vec4<f32>, 8>,
    biome_flags: vec4<f32>,
    // veg 3 (a colour map's plants, scatter/earth.rs): the patch's centre (body frame) and the tan of
    // its half-angle, its two axes
    patch_c: vec4<f32>,
    patch_e1: vec4<f32>,
    patch_e2: vec4<f32>,
    // the wind (render frame, m/s)
    wind: vec4<f32>,
    // SE's forest slot colour (linear), the trees' tint gain (scatter_draw.wesl inst_tint)
    forest: vec4<f32>,
    tint_gain: vec4<f32>,
    // rgb SE's grass slot colour (linear), w the grass's range (m)
    grass: vec4<f32>,
    // x Earth's species' first model, y the flora's, z 1 when the flora is this body's
    species: vec4<u32>,
    // x the ground's relief (m)
    flora_k: vec4<f32>,
    // Cities (scatter.wesl's Body): the civilization, the buildings' range
    city: vec4<f32>,
    city2: vec4<f32>,
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
    // The model's mean colour (linear, from its impostor views) and how much of its quad it covers.
    mean: vec4<f32>,
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
// Per drawn mesh instance (as `insts`), its light (PointLight in f16): `light` writes it, once per
// instance (per vertex it was a third of the trees' cost on the ground).
struct LightRec {
    a: vec4<u32>,
    b: vec4<u32>,
}
@group(0) @binding(4) var<storage, read> inst_lights: array<LightRec>;
@group(0) @binding(5) var<storage, read_write> inst_lights_w: array<LightRec>;
@group(0) @binding(6) var<storage, read> light_counts: array<u32>;
@group(0) @binding(7) var<storage, read> light_buckets: array<vec4<u32>>;

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

fn pack_light(pl: PointLight) -> LightRec {
    return LightRec(
        vec4<u32>(pack2x16float(pl.sun.xy), pack2x16float(vec2<f32>(pl.sun.z, pl.sky.x)), pack2x16float(pl.sky.yz), pack2x16float(pl.flat_e.xy)),
        vec4<u32>(pack2x16float(vec2<f32>(pl.flat_e.z, pl.haze_t.x)), pack2x16float(pl.haze_t.yz), pack2x16float(pl.haze_a.xy), pack2x16float(vec2<f32>(pl.haze_a.z, 0.0))),
    );
}

fn unpack_light(r: LightRec) -> PointLight {
    let a0 = unpack2x16float(r.a.x);
    let a1 = unpack2x16float(r.a.y);
    let a2 = unpack2x16float(r.a.z);
    let a3 = unpack2x16float(r.a.w);
    let b0 = unpack2x16float(r.b.x);
    let b1 = unpack2x16float(r.b.y);
    let b2 = unpack2x16float(r.b.z);
    let b3 = unpack2x16float(r.b.w);
    var o: PointLight;
    o.sun = vec3<f32>(a0, a1.x);
    o.sky = vec3<f32>(a1.y, a2);
    o.flat_e = vec3<f32>(a3, b0.x);
    o.haze_t = vec3<f32>(b0.y, b1);
    o.haze_a = vec3<f32>(b2, b3.x);
    return o;
}

// The meshes' light, one invocation per drawn instance of a mesh bucket (gid.y the bucket pair):
// the ground's light at the middle of its crown (or its half height).
@compute @workgroup_size(64)
fn light(@builtin(global_invocation_id) gid: vec3<u32>) {
    let b = light_buckets[gid.y * 2u];
    if gid.x >= min(light_counts[gid.y * 2u], b.y) {
        return;
    }
    let idx = b.x + gid.x;
    let inst = insts[idx];
    let md = models[inst.model & 0xffu];
    let up = oct_decode(inst.up);
    let h = select(md.centre_y, md.crown_y, md.crown_y > 0.0);
    inst_lights_w[idx] = pack_light(point_light(inst.pos + up * h * inst.scale));
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
    @location(11) @interpolate(flat) tint: vec3<f32>,
}

// The trees' colour where they stand (scatter.wesl: the map's forest colour, or SE's forest slot)
// over the model's mean, times the body's gain (`Body::tint_gain`, scatter.rs): the trees then have
// the ground's own forest colour, and it never changes as they come and go. 1 when the instance has
// none. Earth's gain was matched by eye and by pixel means to its ground with the trees off, 0.7-3
// km away (2026-11-04, Black Forest): the ground brightens its colour map by its biome detail
// textures' brightness (planet.wesl `luma(biome)`), and the crowns' ball shading is darker than
// level ground.

fn inst_tint(packed: u32, md: Model) -> vec3<f32> {
    if (packed >> 8u) == 0u {
        return vec3<f32>(1.0);
    }
    let s = unpack4x8unorm(packed).yzw;
    return clamp(s * s / max(md.mean.rgb, vec3<f32>(1e-3)) * body.tint_gain.xyz, vec3<f32>(0.0), vec3<f32>(16.0));
}

// How far the wind moves a tree's point `local` (model space, m; y up) of height `height`: the
// whole tree leans and sways about its base, more towards the top, at its own phase; the leaves
// flutter as well. `up` is the tree's up, the result in the render frame.
fn wind_offset(local: vec3<f32>, height: f32, up: vec3<f32>, phase: f32, leaves: bool) -> vec3<f32> {
    let w = body.wind.xyz - up * dot(body.wind.xyz, up);
    let ws = min(length(w), 25.0);
    if ws < 0.05 {
        return vec3<f32>(0.0);
    }
    let dir = w / length(w);
    let t = body.lod2.z;
    let h = clamp(local.y / max(height, 1.0), 0.0, 1.2);
    let gust = 0.6 + 0.4 * sin(t * 0.31 + phase * 0.7) * sin(t * 0.17 + phase);
    let lean = (0.0012 * ws + 0.00015 * ws * ws) * gust;
    // (Every frequency a whole multiple of 0.01 rad/s: the clock wraps at 200 pi s, scatter.rs.)
    let sway = 0.0012 * ws * sin(t * (round((1.1 + 0.3 * fract(phase)) * 100.0) * 0.01) + phase);
    var o = dir * ((lean + sway) * h * h * height);
    if leaves {
        let f = sin(t * 7.0 + dot(local, vec3<f32>(2.3, 1.7, 2.9)) + phase);
        o += (dir * 0.6 + up * 0.4) * (0.004 * ws * f * h);
    }
    return o;
}

struct VertexIn {
    @builtin(instance_index) instance: u32,
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

@if(!IMPOSTOR && !BUILDING)
@vertex
fn vertex_mesh(v: VertexIn) -> MeshOut {
    let inst = insts[v.instance];
    let md = models[inst.model & 0xffu];
    let up = oct_decode(inst.up);
    let xa = oct_decode(inst.xa);
    let za = cross(xa, up);
    let basis = mat3x3<f32>(xa, up, za);
    let height = md.centre_y * 2.0 * inst.scale;
    let phase = f32(inst.xa & 0xffffu) * (6.2831853 / 65536.0);
    var leaves = false;
@if(LEAVES) {
    leaves = true;
}
    let world = inst.pos + basis * (v.pos * inst.scale) + wind_offset(v.pos * inst.scale, height, up, phase, leaves);
    var o: MeshOut;
    o.clip = cam.clip_from_world * vec4<f32>(world, 1.0);
    o.world = world;
    o.normal = basis * v.normal;
    o.uv = v.uv;
    o.fades = unpack2x16unorm(inst.fades);
    o.up = up;
    // The crown shades as a ball (Megaton Rainfall's leaves): the direction from its centre.
    o.ball = basis * ((v.pos - vec3<f32>(0.0, md.crown_y, 0.0)) / max(md.crown_r, 0.1));
    let pl = unpack_light(inst_lights[v.instance]);
    o.l_sun = pl.sun;
    o.l_sky = pl.sky;
    o.l_flat = pl.flat_e;
    o.haze_t = pl.haze_t;
    o.haze_a = pl.haze_a;
    o.tint = inst_tint(inst.model, md);
    return o;
}

@if(!IMPOSTOR && !BUILDING) @group(1) @binding(0) var color_tex: texture_2d<f32>;
@if(!IMPOSTOR) @group(1) @binding(1) var color_smp: sampler;
@if(!IMPOSTOR) @group(1) @binding(2) var<uniform> material: Material;
// The buildings: MR's window array (18 facade tiles, the windows' mask in alpha).
@if(BUILDING) @group(1) @binding(0) var windows_tex: texture_2d_array<f32>;

@if(!IMPOSTOR && !BUILDING)
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
    // The tint in full on the leaves, half on the bark.
    var tint = sqrt(in.tint);
@if(LEAVES) {
    tint = in.tint;
}
    let albedo = tex.rgb * material.tint.rgb * tint;
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
    @location(12) @interpolate(flat) tint: vec3<f32>,
}

@if(IMPOSTOR)
@vertex
fn vertex_impostor(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> ImpostorOut {
    let inst = insts[ii];
    let md = models[inst.model & 0xffu];
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
    o.tint = inst_tint(inst.model, md);
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
    return vec4<f32>(surface_color(pl, c.rgb * in.tint, n, up, 0.5, leaf_trans(in.world)), 1.0);
}

// ---- buildings (cities) ----------------------------------------------------------------------
// One box an instance (scatter.wesl cull_building): `scale` its height from the bottom of its plinth,
// `fades` its width and depth (f16), the model word its fade (bits 8-15) and a hash (16-31). The
// facade is drawn by the pixel: storeys of about 3 m, bays of windows, the walls' material and the
// roof by the building's hash and height; by night a share of its windows lit (one hash a window,
// Megaton Rainfall's rule: no state, stable).

struct BuildingOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // x across the face (m), y down from the roof (m), z along the roof's second axis (m)
    @location(2) facade: vec3<f32>,
    @location(3) @interpolate(flat) up: vec3<f32>,
    @location(4) l_sun: vec3<f32>,
    @location(5) l_sky: vec3<f32>,
    @location(6) l_flat: vec3<f32>,
    @location(7) haze_t: vec3<f32>,
    @location(8) haze_a: vec3<f32>,
    // x, y the hash's two bytes (0-1), z the fade, w the height (m)
    @location(9) @interpolate(flat) style: vec4<f32>,
    // the footprint (m) and whether this is the roof
    @location(10) @interpolate(flat) size: vec3<f32>,
}

@if(BUILDING)
@vertex
fn vertex_building(v: VertexIn) -> BuildingOut {
    let inst = insts[v.instance];
    let up = oct_decode(inst.up);
    let xa = oct_decode(inst.xa);
    let za = cross(xa, up);
    let wd = unpack2x16float(inst.fades);
    let world = inst.pos + xa * (v.pos.x * wd.x) + up * (v.pos.y * inst.scale) + za * (v.pos.z * wd.y);
    var o: BuildingOut;
    o.clip = cam.clip_from_world * vec4<f32>(world, 1.0);
    o.world = world;
    o.normal = xa * v.normal.x + up * v.normal.y + za * v.normal.z;
    let roof = v.normal.y > 0.5;
    let across = select(v.pos.z * wd.y, v.pos.x * wd.x, abs(v.normal.z) > 0.5);
    o.facade = vec3<f32>(select(across, v.pos.x * wd.x, roof), (1.0 - v.pos.y) * inst.scale, v.pos.z * wd.y);
    o.up = up;
    let pl = unpack_light(inst_lights[v.instance]);
    o.l_sun = pl.sun;
    o.l_sky = pl.sky;
    o.l_flat = pl.flat_e;
    o.haze_t = pl.haze_t;
    o.haze_a = pl.haze_a;
    let hash = inst.model >> 16u;
    o.style = vec4<f32>(f32(hash & 0xffu) / 255.0, f32(hash >> 8u) / 255.0, f32((inst.model >> 8u) & 0xffu) / 255.0, inst.scale);
    o.size = vec3<f32>(wd, select(0.0, 1.0, roof));
    return o;
}

fn bd_hash(a: u32) -> f32 {
    var h = a * 0x9e3779b9u;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// A box's share of a cell (0..1 over `x`) covered by the middle `frac` of it, box-filtered by the
// footprint `fp` (in cells).
fn bd_band(x: f32, frac: f32, fp: f32) -> f32 {
    let lo = 0.5 - 0.5 * frac;
    let hi = 0.5 + 0.5 * frac;
    let f = max(fp, 1e-4);
    let a = clamp((min(x + 0.5 * f, hi) - max(x - 0.5 * f, lo)) / f, 0.0, 1.0);
    return mix(a, frac, smoothstep(0.35, 1.0, fp));
}

// MR's window tiles (`windows`, 18 layers): storeys a tile holds (its width is the same), and which
// suit houses (big windows), blocks (rows of flats) and offices and towers (ribbons, curtain walls).
const WIN_ROWS = array<f32, 18>(4.0, 4.0, 8.0, 7.0, 4.0, 6.0, 8.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 8.0, 8.0, 8.0);
const WIN_HOUSE = array<u32, 4>(10u, 11u, 1u, 9u);
const WIN_BLOCK = array<u32, 6>(0u, 2u, 4u, 7u, 12u, 17u);
const WIN_TOWER = array<u32, 8>(6u, 5u, 8u, 13u, 14u, 15u, 16u, 3u);

@if(BUILDING)
@fragment
fn fragment_building(in: BuildingOut) -> @location(0) vec4<f32> {
    let s0 = in.style.x;
    let s1 = in.style.y;
    let h = in.style.w;
    let n = normalize(in.normal);
    let l = normalize(body.light.sun_dir.xyz);
    // By night (the star under the horizon there) a share of the windows is lit.
    let night = 1.0 - smoothstep(-0.12, 0.03, dot(in.up, l));
    var albedo: vec3<f32>;
    var emit = vec3<f32>(0.0);
    if in.size.z > 0.5 {
        // The roof: tar and gravel, a parapet round its edge; houses' roofs tiled or slate.
        let edge = min(0.5 * in.size.x - abs(in.facade.x), 0.5 * in.size.y - abs(in.facade.z));
        let house = h < 12.0;
        var roof = mix(vec3<f32>(0.16, 0.16, 0.155), vec3<f32>(0.24, 0.235, 0.22), s1);
        if house {
            roof = select(vec3<f32>(0.20, 0.21, 0.23), vec3<f32>(0.38, 0.19, 0.13), s1 > 0.45);
        }
        let fp = length(fwidth(in.facade.xz));
        albedo = mix(roof, vec3<f32>(0.32, 0.31, 0.29), (1.0 - smoothstep(0.5 - fp, 0.5 + fp, edge)) * select(1.0, 0.0, house));
    } else {
        // The walls: storeys of 3.1 m (houses 2.9), bays of windows, the material by the hash and
        // the height (houses rendered or brick, blocks concrete or brick, towers glass or panels).
        let tall = smoothstep(25.0, 45.0, h);
        let storey = select(3.1, 2.9, h < 12.0);
        let bay = mix(mix(2.6, 4.2, s1), 1.6, tall * step(0.5, s0));
        let wall_a = array<vec3<f32>, 4>(vec3<f32>(0.55, 0.50, 0.42), vec3<f32>(0.42, 0.24, 0.17), vec3<f32>(0.60, 0.59, 0.56), vec3<f32>(0.38, 0.37, 0.36));
        var wall = wall_a[min(u32(s0 * 4.0), 3u)];
        let glassy = tall * step(0.5, s0);
        wall = mix(wall, vec3<f32>(0.10, 0.12, 0.14), glassy);
        // MR's tile for the building's use: houses, blocks, offices and towers.
        let pick = u32(s1 * 255.0);
        var layer = WIN_BLOCK[pick % 6u];
        if h < 12.0 {
            layer = WIN_HOUSE[pick % 4u];
        } else if tall > 0.5 || glassy > 0.5 {
            layer = WIN_TOWER[pick % 8u];
        }
        let rows = WIN_ROWS[layer];
        let uv = in.facade.xy / (rows * storey);
        let tex = textureSample(windows_tex, color_smp, uv, layer);
        // Its windows over the wall (glassy towers: the tile over dark glass).
        let win = tex.a;
        albedo = mix(wall, tex.rgb * mix(1.0, 0.6, glassy), win);
        let cx = uv.x * rows * 2.0;
        let cy = uv.y * rows;
        let fpx = length(vec2<f32>(dpdx(cx), dpdy(cx)));
        let fpy = length(vec2<f32>(dpdx(cy), dpdy(cy)));
        // The window's light by night: a hash per window, about 45 % lit, warm, of various strength.
        let wid = u32(floor(cx) + 4096.0) * 7919u + u32(floor(cy)) * 104729u + u32(s0 * 255.0) * 31u + u32(s1 * 255.0) * 65537u;
        let lit = bd_hash(wid);
        let on = select(0.0, 0.35 + 0.65 * bd_hash(wid ^ 0x5bd1e995u), lit < 0.45);
        // Far: the lit share of the windows' area.
        let far = smoothstep(0.35, 1.0, max(fpx, fpy));
        let lit_w = mix(on, 0.45 * 0.68, far) * win;
        emit = vec3<f32>(1.0, 0.74, 0.46) * (0.55 * lit_w * night);
    }
    let pl = PointLight(in.l_sun, in.l_sky, in.l_flat, in.haze_t, in.haze_a);
    // The far fade (the range's last 15 %), after the texture's samples (their derivatives).
    if dither2(in.clip.xy) >= in.style.z {
        discard;
    }
    return vec4<f32>(surface_color(pl, albedo, n, in.up, 0.0, 0.0) + emit * in.haze_t, 1.0);
}
