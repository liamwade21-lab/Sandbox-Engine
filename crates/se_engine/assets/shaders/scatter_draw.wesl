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
    // The street objects: x their range (m), y the traffic's clock.
    street: vec4<f32>,
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
    // x 1 for a broadleaf tree (its leaves follow the seasons: scatter_draw.wesl tree_season)
    flags: vec4<u32>,
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
    // x the far fade, y the mesh's share of the LOD band, z the tree's hash
    @location(3) @interpolate(flat) fades: vec4<f32>,
    @location(4) @interpolate(flat) up: vec3<f32>,
    @location(5) ball: vec3<f32>,
    @location(6) l_sun: vec3<f32>,
    @location(7) l_sky: vec3<f32>,
    @location(8) l_flat: vec3<f32>,
    @location(9) haze_t: vec3<f32>,
    @location(10) haze_a: vec3<f32>,
    @location(11) @interpolate(flat) tint: vec3<f32>,
    @location(12) @interpolate(flat) season: vec4<f32>,
}

// A tree's season (the plan's step 11): x its leaves' autumn colour, y the share of its leaves gone
// (fallen, or not yet out), z spring's fresh green, w the snow on it (`snow`: the ground's snow at its
// place, from the cull). The seasons follow the star's declination at the tree's latitude
// (`Body::sun_body` y, w +1 while it climbs; `tint_gain.w` the sine of the axial tilt; `wind.w` 1 when
// the body has seasons): broadleaf models alone (`Model::flags.x`), none in the tropics, fading out
// towards the trees' far range, where the ground's own forest colour (it has no seasons) takes over.
// `h` the tree's hash (each turns a little early or late), `d` its distance (m).
fn tree_season(md: Model, up: vec3<f32>, h: f32, d: f32, snow: f32) -> vec4<f32> {
    var o = vec4<f32>(0.0, 0.0, 0.0, snow);
    if md.flags.x == 0u || body.wind.w < 0.5 {
        return o;
    }
    let sl = dot(up, normalize(body.m1.xyz));
    let sin_t = body.tint_gain.w;
    let amp = smoothstep(0.3, 0.55, abs(sl)) * smoothstep(0.05, 0.15, sin_t) * (1.0 - smoothstep(0.5, 0.95, d / max(body.lod.z, 1.0)));
    if amp <= 0.0 {
        return o;
    }
    let hemi = select(-1.0, 1.0, sl >= 0.0);
    let s = clamp(body.sun_body.y / max(sin_t, 0.05), -1.0, 1.0) * hemi;
    // The local year's angle (degrees): -90 midwinter, 0 the spring equinox, 90 midsummer, 180 the
    // autumn equinox; then -40..320.
    var p = asin(s) * 57.29578;
    if body.sun_body.w * hemi < 0.0 {
        p = 180.0 - p;
    }
    p += (h - 0.5) * 24.0;
    if p < -40.0 {
        p += 360.0;
    }
    // (The autumn colour fades out while the tree is bare, before spring: a tree that keeps some
    // leaves would otherwise turn from orange to green at once at the wrap.)
    o.x = max(smoothstep(150.0, 195.0, p), 1.0 - smoothstep(-40.0, -20.0, p)) * amp;
    o.y = max(1.0 - smoothstep(-20.0, 30.0, p), smoothstep(190.0, 235.0, p)) * amp;
    o.z = smoothstep(-20.0, 0.0, p) * (1.0 - smoothstep(30.0, 70.0, p)) * amp;
    return o;
}

// Leaves' colour in the season `s` (tree_season): autumn's yellow, orange or red by the tree's hash,
// as bright as the green they replace and a half more; spring's lighter, yellower green.
fn season_albedo(albedo: vec3<f32>, s: vec4<f32>, h: f32) -> vec3<f32> {
    var c = albedo;
    if s.x > 0.0 {
        let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
        var aut = vec3<f32>(0.60, 0.26, 0.04);
        if h < 0.4 {
            aut = vec3<f32>(0.58, 0.44, 0.06);
        } else if h > 0.8 {
            aut = vec3<f32>(0.52, 0.10, 0.05);
        }
        c = mix(c, aut * (dot(c, lum) * 1.5 / dot(aut, lum)), s.x);
    }
    return mix(c, c * vec3<f32>(1.2, 1.3, 0.85), s.z);
}

// Snow on the faces that look up (`top`: the normal's part along the up).
fn snow_on(albedo: vec3<f32>, snow: f32, top: f32) -> vec3<f32> {
    return mix(albedo, vec3<f32>(0.8), snow * smoothstep(0.15, 0.6, top));
}

// A stable number in 0..1 for a cell of a texture's or quad's coordinates (`seed`: the tree's hash).
fn cell_hash(cell: vec2<f32>, seed: f32) -> f32 {
    return fract(sin(dot(cell, vec2<f32>(12.9898, 78.233)) + seed * 437.0) * 43758.5453);
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
    // A street object stands rigid.
    let rigid = md.flags.y != 0u;
    let sway = select(wind_offset(v.pos * inst.scale, height, up, phase, leaves), vec3<f32>(0.0), rigid);
    let world = inst.pos + basis * (v.pos * inst.scale) + sway;
    var o: MeshOut;
    o.clip = cam.clip_from_world * vec4<f32>(world, 1.0);
    o.world = world;
    o.normal = basis * v.normal;
    o.uv = v.uv;
    let fd = unpack4x8unorm(inst.fades);
    let h = phase / 6.2831853;
    o.fades = vec4<f32>(fd.xy, h, f32(md.flags.y));
    o.season = tree_season(md, up, h, length(inst.pos), fd.z);
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
    if rigid {
        // For `street_surface`: its place in the model, its numbers, a car's paint.
        o.ball = v.pos;
        o.season = md.mean;
        let s = unpack4x8unorm(inst.model).yzw;
        o.tint = s * s;
    }
    return o;
}

@if(!IMPOSTOR && !BUILDING) @group(1) @binding(0) var color_tex: texture_2d<f32>;
@if(!IMPOSTOR) @group(1) @binding(1) var color_smp: sampler;
@if(!IMPOSTOR) @group(1) @binding(2) var<uniform> material: Material;
// The buildings: MR's window array (18 facade tiles, the windows' mask in alpha).
@if(BUILDING) @group(1) @binding(0) var windows_tex: texture_2d_array<f32>;

// A street object's surface (library.rs `street_models`; `Model::flags` y: 1 plain, 2 a painted car, 3 a
// car with its paint in its texture, 4 a lamp, whose glass is its one bright colour). `in.ball` its place
// in the model (m), `in.season` a car's numbers: x half its length, y its lights' height, z their offset
// from its middle, w half its width. A car's windows are where its texture's alpha is low (as Megaton
// Rainfall's). By night the lamps' glass and the cars' lights shine.
@if(!IMPOSTOR && !BUILDING)
fn street_surface(in: MeshOut, tex: vec4<f32>, n: vec3<f32>) -> vec4<f32> {
    let kind = u32(in.fades.w + 0.5);
    let pl = PointLight(in.l_sun, in.l_sky, in.l_flat, in.haze_t, in.haze_a);
    let l = normalize(body.light.sun_dir.xyz);
    let night = 1.0 - smoothstep(-0.12, 0.03, dot(in.up, l));
    let p = in.ball;
    let s = in.season;
    var albedo = tex.rgb * material.tint.rgb;
    var emit = vec3<f32>(0.0);
    var spec = 0.04;
    var sheen = 0.02;
    var gloss = 20.0;
    if kind == 4u {
        if dot(albedo, vec3<f32>(1.0 / 3.0)) > 0.4 {
            emit = civ_light() * 3.0 * night;
        }
    } else if kind == 2u || kind == 3u {
        if p.y < 0.06 {
            // The dark patch it stands on, its edges dithered soft.
            let edge = min(s.x - 0.2 - abs(p.x), s.w - 0.1 - abs(p.z));
            if dither2(in.clip.xy) >= smoothstep(0.0, 0.12, edge) {
                discard;
            }
            return vec4<f32>(pl.haze_a, 1.0);
        }
        if tex.a < 0.7 {
            albedo = vec3<f32>(0.02);
            spec = 0.5;
            sheen = 0.25;
            gloss = 120.0;
        } else {
            if kind == 2u {
                albedo *= in.tint;
            }
            spec = 0.2;
            sheen = 0.06;
            gloss = 50.0;
        }
        let d = vec2<f32>(abs(p.z) - s.z, p.y - s.y);
        let spot = 1.0 - smoothstep(0.02, 0.06, dot(d, d));
        if p.x > s.x - 0.35 {
            emit = vec3<f32>(1.0, 0.92, 0.75) * 3.0 * spot * night;
        } else if p.x < 0.35 - s.x {
            emit = vec3<f32>(1.0, 0.06, 0.02) * 1.5 * spot * night;
        }
    }
    // The star's highlight and the sky in what shines.
    let v = normalize(-in.world);
    let h = normalize(l + v);
    let fres = pow(1.0 - max(dot(n, v), 0.0), 5.0);
    let shine = (in.l_sun * pow(max(dot(n, h), 0.0), gloss) * max(dot(n, l), 0.0) * spec + in.l_sky * (1.0 + 2.0 * fres) * sheen) / PI;
    // By night the street lamps light them from above, as they do the road (its `lamp`).
    emit += albedo * civ_light() * night * (0.25 + 0.2 * max(dot(n, in.up), 0.0));
    return vec4<f32>(surface_color(pl, albedo, n, in.up, 0.0, 0.0) + (shine + emit) * in.haze_t, 1.0);
}

@if(!IMPOSTOR && !BUILDING)
@fragment
fn fragment_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // The mesh's share of the LOD band.
    if dither(in.clip.xy) >= in.fades.y {
        discard;
    }
    let tex = textureSample(color_tex, color_smp, in.uv);
    var n = normalize(in.normal);
@if(!LEAVES) {
    if in.fades.w > 0.5 {
        return street_surface(in, tex, n);
    }
}
    var wrap = 0.0;
    // The snow lies on what looks up: the crown's top, the branches' upper sides.
    var top = dot(n, in.up);
@if(LEAVES) {
    if tex.a < 0.5 {
        discard;
    }
    // The leaves gone (fallen in autumn, not yet out in spring): patches of the card at a time.
    if in.season.y > 0.0 && cell_hash(floor(in.uv * 6.0), in.fades.z) < in.season.y {
        discard;
    }
    top = dot(normalize(in.ball + vec3<f32>(1e-5)), in.up);
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
    var albedo = tex.rgb * material.tint.rgb * tint;
@if(LEAVES) {
    albedo = season_albedo(albedo, in.season, in.fades.z);
}
    albedo = snow_on(albedo, in.season.w, top);
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
    // x the far fade, y the mesh's share of the LOD band, z the tree's hash
    @location(1) @interpolate(flat) fades: vec4<f32>,
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
    @location(13) @interpolate(flat) season: vec4<f32>,
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
    let fd = unpack4x8unorm(inst.fades);
    let h = f32(inst.xa & 0xffffu) / 65536.0;
    o.fades = vec4<f32>(fd.xy, h, 0.0);
    o.season = tree_season(md, up, h, dist, fd.z);
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
    var albedo = season_albedo(c.rgb * in.tint, in.season, in.fades.z);
    if in.season.y > 0.0 {
        // A bare tree: a third of its crown's texels left (patches fixed on the tree), in the
        // branches' grey brown.
        if cell_hash(floor(in.quad * 40.0), in.fades.z) < in.season.y * 0.7 {
            discard;
        }
        albedo = mix(albedo, vec3<f32>(0.075, 0.068, 0.06), in.season.y);
    }
    albedo = snow_on(albedo, in.season.w, dot(n, up));
    return vec4<f32>(surface_color(pl, albedo, n, up, 0.5, leaf_trans(in.world)), 1.0);
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
    // the point on the model (m from its base's middle) and the model: 0 the box, 1 a round hut,
    // 2 a longhouse (villages), 3-10 level 3's (the future box, four towers, the arcology, the
    // skybridge, the flying lanes: city.wesl M_FUTURE..M_LANE)
    @location(11) local: vec3<f32>,
    @location(12) @interpolate(flat) kind: u32,
    // Level 3: what the face is (library.rs `tower`: 0, 1 a wall, 2 the spire, 3 a roof), and across
    // the wall from -1 at one edge to 1 at the other (round towers: 0)
    @location(13) @interpolate(flat) flag: f32,
    @location(14) edge: f32,
    // The hero models' uv (metres, the texture layer in v's ten-thousands).
    @location(15) uv: vec2<f32>,
}

// Level 3's models (city.wesl M_FUTURE..M_LANE, offsets from the box).
const K_FUTURE: u32 = 3u;
const K_ARCO: u32 = 8u;
const K_BRIDGE: u32 = 9u;
const K_LANE: u32 = 10u;
const K_TEMPLE: u32 = 11u;
const K_LANDMARK: u32 = 13u;
const K_DOME: u32 = 14u;

@if(BUILDING)
@vertex
fn vertex_building(v: VertexIn) -> BuildingOut {
    let inst = insts[v.instance];
    let up = oct_decode(inst.up);
    let xa = oct_decode(inst.xa);
    let za = cross(xa, up);
    let wd = unpack2x16float(inst.fades);
    let kind = (inst.model & 0xffu) - body.species.w;
    var world = inst.pos + xa * (v.pos.x * wd.x) + up * (v.pos.y * inst.scale) + za * (v.pos.z * wd.y);
    var o: BuildingOut;
    // The code-built models' normals are the unit box's: through the instance's stretch (the inverse
    // scale) for the tapering and twisted walls' slopes. The hero models' are their real ones.
    var nl = v.normal;
    if (kind < K_TEMPLE || kind == K_DOME) && kind != K_LANE {
        nl = nl / vec3<f32>(max(wd.x, 1e-3), max(inst.scale, 1e-3), max(wd.y, 1e-3));
    }
    o.normal = normalize(xa * nl.x + up * nl.y + za * nl.z);
    var roof = v.normal.y > 0.5;
    let across = select(v.pos.z * wd.y, v.pos.x * wd.x, abs(v.normal.z) > 0.5);
    o.facade = vec3<f32>(select(across, v.pos.x * wd.x, roof), (1.0 - v.pos.y) * inst.scale, v.pos.z * wd.y);
    o.flag = 0.0;
    o.edge = 0.0;
    o.uv = v.uv;
    var lane_towards = 0.0;
    if kind == K_LANE {
        // A flying lane's ribbon, widened to a car (1.6 m high, 2.2 m wide) or 1.5 pixels.
        let base = inst.pos + xa * (v.pos.x * wd.x) + za * (v.pos.z * wd.y);
        let row1 = vec3<f32>(cam.clip_from_world[0][1], cam.clip_from_world[1][1], cam.clip_from_world[2][1]);
        let px = 2.0 * length(base) / (cam.viewport.y * max(length(row1), 1e-6));
        let flat_part = v.uv.y > 0.5;
        let half = max(select(0.8, 1.1, flat_part), 0.75 * px);
        world = base + select(up, za, flat_part) * (v.uv.x * half);
        // along the segment from its middle (m), across the ribbon (-1..1), the ribbon (-0.5, 0.5)
        o.facade = vec3<f32>(v.pos.x * wd.x, v.uv.x, v.pos.z);
        o.edge = px;
        roof = false;
        // Whether its cars come towards the camera (the ribbon at -z goes along x).
        lane_towards = select(0.0, 1.0, dot(-base, xa) * select(-1.0, 1.0, v.pos.z < 0.0) > 0.0);
    } else if kind >= K_FUTURE && kind < K_TEMPLE {
        // Level 3's towers: across from the mesh's uv (a wall's own width, scaled by the footprint's
        // width or depth; round ones along the perimeter).
        o.flag = v.uv.y;
        roof = v.uv.y > 2.5;
        let a_m = v.uv.x * select(wd.x, wd.y, v.uv.y > 0.5 && v.uv.y < 1.5);
        o.facade = vec3<f32>(select(a_m, v.pos.x * wd.x, roof), (1.0 - v.pos.y) * inst.scale, v.pos.z * wd.y);
        o.edge = select(0.0, sign(v.uv.x), kind != 7u && v.uv.y < 1.5);
    }
    o.clip = cam.clip_from_world * vec4<f32>(world, 1.0);
    o.world = world;
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
    if kind == K_LANE {
        o.size.y = lane_towards;
    }
    o.local = vec3<f32>(v.pos.x * wd.x, v.pos.y * inst.scale, v.pos.z * wd.y);
    o.kind = kind;
    return o;
}

// The culture's light colour (scatter.rs `civ_light`, its brightest channel 1).
fn civ_light() -> vec3<f32> {
    let v = u32(body.flora_k.z + 0.5);
    return vec3<f32>(f32(v >> 12u), f32((v >> 6u) & 63u), f32(v & 63u)) / 63.0;
}

// Level 3's facades (layers 0-2 of their array: glass curtain walls of 8 storeys and 4 bays a tile,
// white panels with ribbon windows of 7 storeys, gold fins over dark glass of 8 storeys and 8 bays).
const FUT_ROWS = array<f32, 3>(8.0, 7.0, 8.0);
const FUT_BAYS = array<f32, 3>(4.0, 4.0, 8.0);
const FUT_STOREY: f32 = 3.6;

// A level 3 building's surface (not the lanes): its albedo (rgb) and light (the second).
struct FutSurface {
    albedo: vec3<f32>,
    emit: vec3<f32>,
}

@if(BUILDING)
fn future_surface(in: BuildingOut, n: vec3<f32>, night: f32, glow_t: f32) -> FutSurface {
    var o: FutSurface;
    let s0 = in.style.x;
    let s1 = in.style.y;
    let h = in.style.w;
    let lc = civ_light();
    let time = body.lod2.z;
    // The sign's and facade's texture coordinates and their derivatives in uniform control flow.
    let layer = select(select(2u, 1u, s0 < 0.75), 0u, s0 < 0.45);
    let tile = FUT_ROWS[layer] * FUT_STOREY;
    let uv = in.facade.xy / tile;
    let tex = textureSampleGrad(windows_tex, color_smp, uv, layer, dpdx(uv), dpdy(uv));
    // Neon signs on the lower storeys (4-28 m up): 8 m by 6 m cells, a sign in 45 % of them.
    let up_m = in.local.y;
    let sc = vec2<f32>(in.facade.x / 8.0, (up_m - 4.0) / 6.0);
    let cell = floor(sc);
    let hs = bd_hash((u32(cell.x + 4096.0) * 7919u) ^ (u32(cell.y + 64.0) * 104729u) ^ (u32(s0 * 255.0) * 31u) ^ (u32(s1 * 255.0) * 65537u));
    let pick = u32(hs * 4093.0) % 16u;
    let su = (fract(sc) - 0.1) / 0.8;
    let auv = (vec2<f32>(f32(pick % 4u), f32(pick / 4u)) + clamp(su, vec2<f32>(0.0), vec2<f32>(1.0))) * 0.25;
    let sgn = textureSampleGrad(windows_tex, color_smp, vec2<f32>(auv.x, auv.y), 3u, dpdx(sc) * 0.25 / 0.8, dpdy(sc) * 0.25 / 0.8);
    let fp = length(fwidth(in.facade.xy));
    if in.flag > 2.5 {
        // Roofs and terraces: dark composite; the arcology's terraces gardens; a tall tower's roof a
        // landing pad (a ring of lights by night) and a light strip round its edge.
        var c = mix(vec3<f32>(0.10, 0.105, 0.11), vec3<f32>(0.2, 0.2, 0.21), s1);
        if in.kind == K_ARCO {
            c = mix(vec3<f32>(0.05, 0.09, 0.035), vec3<f32>(0.09, 0.12, 0.05), s1);
        }
        let r = length(in.facade.xz);
        // The top roof's share of the footprint (library.rs's future box and towers: tapering,
        // stepped, twisted, round).
        let top = array<f32, 5>(1.0, 0.55, 0.56, 0.8, 0.45);
        let pad_r = 0.3 * top[min(in.kind - K_FUTURE, 4u)] * min(in.size.x, in.size.y);
        let ring = 1.0 - smoothstep(0.3, 0.3 + fp, abs(r - pad_r));
        let tall = h > 300.0 && in.kind <= 7u && in.local.y > 0.8 * h;
        if tall {
            c = mix(c, vec3<f32>(0.6, 0.6, 0.55), ring);
            o.emit += lc * ring * 1.2 * night;
        }
        o.albedo = c;
    } else if in.flag > 1.5 {
        // The spire: metal, a red beacon at its top blinking by night.
        o.albedo = vec3<f32>(0.42, 0.43, 0.45);
        let top = h - in.local.y;
        let blink = step(0.0, sin(time * 2.6 + s0 * 30.0));
        o.emit = vec3<f32>(1.0, 0.05, 0.02) * (1.0 - smoothstep(0.0, max(4.0, 3.0 * fp), top)) * 12.0 * blink * night;
    } else {
        // The walls: the facade's tile over the wall (its windows in alpha); by night lit windows (a
        // hash a window, about 40 %). Flat colours when the facades are missing (a blank array).
        let flat_c = array<vec3<f32>, 3>(vec3<f32>(0.10, 0.12, 0.14), vec3<f32>(0.6, 0.6, 0.58), vec3<f32>(0.12, 0.11, 0.09));
        o.albedo = select(flat_c[layer], tex.rgb, dot(tex, vec4<f32>(1.0)) > 0.002);
        if in.kind == K_BRIDGE {
            o.albedo = mix(tex.rgb, vec3<f32>(0.06, 0.08, 0.1), 0.6);
        }
        let bays = FUT_BAYS[layer];
        let rows = FUT_ROWS[layer];
        let cx = uv.x * bays;
        let cy = uv.y * rows;
        let fpx = length(vec2<f32>(dpdx(cx), dpdy(cx)));
        let fpy = length(vec2<f32>(dpdx(cy), dpdy(cy)));
        let wid = u32(floor(cx) + 4096.0) * 7919u + u32(floor(cy) + 4096.0) * 104729u + u32(s0 * 255.0) * 31u + u32(s1 * 255.0) * 65537u;
        let b = bd_hash(wid ^ 0x5bd1e995u);
        let on = select(0.0, 0.15 + 0.85 * b * b, bd_hash(wid) < 0.4);
        let far = smoothstep(0.35, 1.0, max(fpx, fpy));
        let lit_w = mix(on, 0.4 * 0.433, far) * tex.a;
        // Each window's light: warm, neutral, cool or the culture's own (their mean from afar).
        let tc = bd_hash(wid ^ 0x68e31da4u);
        var wc = select(select(vec3<f32>(0.8, 0.9, 1.0), vec3<f32>(1.0, 0.95, 0.9), tc < 0.6), vec3<f32>(1.0, 0.8, 0.58), tc < 0.3);
        wc = select(wc, lc, tc > 0.85);
        wc = mix(wc, mix(vec3<f32>(0.95, 0.88, 0.8), lc, 0.15), far);
        o.emit = wc * (0.35 * lit_w * night);
        // Light strips up the towers' corners (0.6 m), the culture's colour; brighter by night.
        if in.edge != 0.0 || in.kind == 7u {
            let half_w = abs(in.facade.x) / max(abs(in.edge), 1e-3);
            let d_edge = (1.0 - abs(in.edge)) * half_w;
            var strip = clamp((0.6 - d_edge) / max(fp, 1e-3) + 0.5, 0.0, 1.0);
            // Far: the strip's share of the wall (it never vanishes into the pixel).
            strip = mix(strip, min(0.6 / max(fp, 0.6), 1.0), smoothstep(0.5, 3.0, fp));
            if in.kind == 7u {
                // Round towers: rings of light every 8 storeys.
                let ry = in.facade.y / (8.0 * FUT_STOREY);
                let fw = max(fwidth(ry), 1e-4);
                let d = 0.5 - abs(fract(ry) - 0.5);
                strip = clamp((0.021 - d) / fw + 0.5, 0.0, 1.0);
                strip = mix(strip, 0.042, smoothstep(0.02, 0.1, fw));
            }
            if in.kind >= 4u && in.kind <= 8u {
                o.emit += lc * strip * mix(0.3, 1.5, night) * glow_t;
                o.albedo = mix(o.albedo, lc * 0.6, strip * 0.5);
            }
        }
        // Neon on the lower storeys of the future box and the towers' bases.
        if in.kind <= 7u && up_m > 4.0 && up_m < 28.0 && hs < 0.45 && all(fract(sc) > vec2<f32>(0.1)) && all(fract(sc) < vec2<f32>(0.9)) {
            o.albedo = mix(o.albedo, vec3<f32>(0.03), 0.8);
            o.emit += sgn.rgb * mix(0.6, 2.5, night);
        }
    }
    return o;
}

// A hero model (kinds 11-13): its texture layer and metres from its uv. The temple and the watchtower
// take the villages' textures (3 m a tile: 0 thatch, 1 mud brick, 2 timber, 3 stone); the landmark
// level 3's facades (layers 0-2, lit windows by night), metal (4) and light rings (5, the culture's
// colour), a red beacon on its needle.
@if(BUILDING)
fn hero_surface(in: BuildingOut, night: f32) -> FutSurface {
    var o: FutSurface;
    let layer = u32(clamp(floor(in.uv.y / 10000.0), 0.0, 7.0));
    let t = vec2<f32>(in.uv.x, in.uv.y - f32(layer) * 10000.0 - 5000.0);
    let s1 = in.style.y;
    if in.kind < K_LANDMARK {
        let l = min(layer, 3u);
        let tex = textureSampleGrad(windows_tex, color_smp, t / 3.0, l, dpdx(t) / 3.0, dpdy(t) / 3.0);
        let flat_c = array<vec3<f32>, 4>(vec3<f32>(0.42, 0.33, 0.18), vec3<f32>(0.42, 0.30, 0.19), vec3<f32>(0.30, 0.23, 0.16), vec3<f32>(0.40, 0.38, 0.34));
        o.albedo = mix(flat_c[l], tex.rgb, tex.a) * (0.85 + 0.3 * s1);
        return o;
    }
    let lc = civ_light();
    if layer >= 5u {
        o.albedo = lc * 0.5;
        o.emit = lc * mix(0.8, 4.0, night);
    } else if layer == 4u {
        o.albedo = vec3<f32>(0.42, 0.43, 0.45);
        let top = in.style.w - in.local.y;
        let blink = step(0.0, sin(body.lod2.z * 2.6));
        let fp = length(fwidth(in.local));
        o.emit = vec3<f32>(1.0, 0.05, 0.02) * (1.0 - smoothstep(0.0, max(6.0, 3.0 * fp), top)) * 12.0 * blink * night;
    } else {
        let l = min(layer, 2u);
        let tile = FUT_ROWS[l] * FUT_STOREY;
        let uv = t / tile;
        let tex = textureSampleGrad(windows_tex, color_smp, uv, l, dpdx(uv), dpdy(uv));
        o.albedo = tex.rgb;
        let cx = uv.x * FUT_BAYS[l];
        let cy = uv.y * FUT_ROWS[l];
        let fpw = max(length(vec2<f32>(dpdx(cx), dpdy(cx))), length(vec2<f32>(dpdx(cy), dpdy(cy))));
        let wid = u32(floor(cx) + 65536.0) * 7919u + u32(floor(cy) + 65536.0) * 104729u;
        let b = bd_hash(wid ^ 0x5bd1e995u);
        let on = select(0.0, 0.15 + 0.85 * b * b, bd_hash(wid) < 0.4);
        let lit_w = mix(on, 0.4 * 0.433, smoothstep(0.35, 1.0, fpw)) * tex.a;
        o.emit = mix(vec3<f32>(1.0, 0.86, 0.68), lc, 0.3) * (0.45 * lit_w * night);
    }
    return o;
}

// A colony's dome (kind 14): dark glass in a lattice of struts (uv: around and down from the top),
// glinting in the star's light; by night the town's light glows through it and lights stud the
// struts.
@if(BUILDING)
fn dome_surface(in: BuildingOut, n: vec3<f32>, night: f32) -> FutSurface {
    var o: FutSurface;
    let g = vec2<f32>(in.uv.x * 48.0, in.uv.y * 9.0);
    let fw = max(fwidth(g), vec2<f32>(1e-4));
    let d = abs(fract(g + 0.5) - 0.5);
    // The struts (0.06 of a cell), box-filtered: their share of the pixel from afar.
    let sx = clamp((0.03 - d.x) / fw.x + 0.5, 0.0, 1.0);
    let sy = clamp((0.03 - d.y) / fw.y + 0.5, 0.0, 1.0);
    let strut = mix(max(sx, sy), 0.12, smoothstep(0.3, 1.0, max(fw.x, fw.y)));
    o.albedo = mix(vec3<f32>(0.035, 0.045, 0.06), vec3<f32>(0.55, 0.56, 0.58), strut);
    // The glass's glint (a tight highlight of the star round its mirror direction, in its light).
    let l = normalize(body.light.sun_dir.xyz);
    let v = -normalize(in.world);
    let spec = pow(max(dot(reflect(-v, n), l), 0.0), 400.0) * (1.0 - strut);
    o.emit = in.l_sun * (spec * 0.6);
    // By night: the town's glow through the glass, lights where the struts cross.
    let lc = civ_light();
    let cross = clamp((0.08 - d.x) / fw.x + 0.5, 0.0, 1.0) * clamp((0.08 - d.y) / fw.y + 0.5, 0.0, 1.0);
    o.emit += (lc * 0.07 * (1.0 - strut) + lc * 2.0 * mix(cross, 0.02, smoothstep(0.3, 1.0, max(fw.x, fw.y)))) * night;
    return o;
}

// A flying lane's ribbon (kind 10): cars of 4.8 m in slots of 32 face m moving at about 52 face m/s, each slot holding a car with chance 0.55; the two ribbons go opposite ways. Near:
// the car's body with its headlights at the front and tail lights at the back; far (a car under two
// pixels): a point of light, white coming towards the camera and red going away. Discards elsewhere.
@if(BUILDING)
fn lane_car(in: BuildingOut, night: f32) -> FutSurface {
    var o: FutSurface;
    let hash = u32(in.style.x * 255.0 + 0.5) | (u32(in.style.y * 255.0 + 0.5) << 8u);
    let seg = f32(hash & 0xffu);
    let high = (hash >> 14u) & 1u;
    let len_m = in.size.x;
    let px = in.edge;
    let fwd = select(-1.0, 1.0, in.facade.z < 0.0);
    // Along the line in face metres (each segment exactly 128 of them, so the cars run on across the
    // joins), true metres per face metre this segment's.
    let km = len_m / 128.0;
    let x = (in.facade.x / len_m + 0.5 + seg) * 128.0;
    // About 52 face m/s: the 1,024 slots of the segments' wrap (256 of 128 face m) exactly once over
    // the clock's wrap (200 pi s), the slots keyed modulo them: no jump when either wraps.
    let period = 32.0;
    let v = fwd * 1024.0 * period / 628.3185307;
    let xm = x - v * body.lod2.z;
    let slot = floor(xm / period);
    let key = (hash >> 8u) * 0x9e3779b9u + select(0u, 0x632be5abu, fwd > 0.0) + u32(i32(slot) & 1023) * 0x85ebca6bu;
    if bd_hash(key) >= 0.55 {
        discard;
    }
    let car = max(4.8, 2.0 * px) / km;
    let cx = xm - slot * period - bd_hash(key ^ 0x27d4eb2fu) * (period - car);
    if cx < 0.0 || cx > car {
        discard;
    }
    // Its front (the way it moves).
    let t = select(car - cx, cx, fwd < 0.0) / car;
    let paint = mix(vec3<f32>(0.75, 0.76, 0.78), civ_light() * 0.5, bd_hash(key ^ 0x165667b1u));
    let far = smoothstep(1.0, 3.0, px / 1.6);
    let head = (1.0 - smoothstep(0.08, 0.16, t)) * (1.0 - far);
    let tail = smoothstep(0.84, 0.92, t) * (1.0 - far);
    // Towards the camera: the car's front faces it.
    let towards = in.size.y;
    var e = vec3<f32>(1.0, 0.95, 0.85) * head * 6.0 + vec3<f32>(1.0, 0.05, 0.02) * tail * 4.0;
    e += mix(vec3<f32>(1.0, 0.06, 0.03) * 3.0, vec3<f32>(1.0, 0.95, 0.85) * 5.0, towards) * far;
    o.albedo = paint * (1.0 - far);
    o.emit = e * mix(0.35, 1.0, night);
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
    if in.kind >= K_FUTURE {
        var f: FutSurface;
        if in.kind == K_DOME {
            f = dome_surface(in, n, night);
        } else if in.kind >= K_TEMPLE {
            f = hero_surface(in, night);
        } else if in.kind == K_LANE {
            f = lane_car(in, night);
        } else {
            f = future_surface(in, n, night, 1.0);
        }
        albedo = f.albedo;
        emit = f.emit;
    } else if in.kind >= 1u {
        // A village's hut or longhouse: thatch on the roof, the walls mud brick, timber or stone
        // (the port's textures, layers 0-3; flat colours without them).
        let hut = in.kind == 1u;
        let roofed = select(n.y > 0.3, in.local.y > 0.37 * in.style.w && dot(n, in.up) > 0.2, hut) || dot(n, in.up) > 0.3;
        let flat_c = array<vec3<f32>, 4>(vec3<f32>(0.42, 0.33, 0.18), vec3<f32>(0.42, 0.30, 0.19), vec3<f32>(0.30, 0.23, 0.16), vec3<f32>(0.40, 0.38, 0.34));
        var layer = 1u + min(u32(s0 * 3.0), 2u);
        var uv: vec2<f32>;
        if roofed {
            layer = 0u;
            if hut {
                let r = 0.5 * in.size.x;
                uv = vec2<f32>(atan2(in.local.z, in.local.x) * r / 2.5, (in.style.w - in.local.y) / 2.5);
            } else {
                uv = vec2<f32>(in.local.z / 2.5, abs(in.local.x) / 2.0);
            }
        } else if hut {
            uv = vec2<f32>(atan2(in.local.z, in.local.x) * 0.5 * in.size.x / 3.0, in.local.y / 3.0);
        } else {
            uv = vec2<f32>((in.local.x + in.local.z) / 3.0, in.local.y / 3.0);
        }
        let tex = textureSample(windows_tex, color_smp, uv, layer);
        albedo = mix(flat_c[layer], tex.rgb, tex.a) * (0.85 + 0.3 * s1);
    } else if in.size.z > 0.5 {
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
