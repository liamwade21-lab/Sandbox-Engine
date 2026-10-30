// Gravitational lensing by black holes, neutron stars and white dwarfs, with the volumetric
// accretion disk and jets, as an HDR post-process (SE's einstein.glsl + gr_geodesic.glh +
// gr_intersection.glh, KERR_NEWMAN_METRIC, single-pass "COMPUTE + RENDER" mode).
//
// For every pixel near such an object the (reversed) light ray is traced like SE's
// TraceGeodesic: straight up to SE's integration sphere, inside it a Hamiltonian geodesic of
// the Kerr-Newman metric in Kerr-Schild coordinates (spin KerrSpin, charge KerrCharge; the
// flattened, shifted shadow and frame dragging of a spinning hole), then straight again. Like
// SE's EvaluateIntersections, each step's straight chord is ray-marched through the disk and jet
// volume (accretion_disk.wesl) and tested against the star's surface (STAR). The ray then
// either falls through the horizon (black shadow), hits the star, or escapes, and its direction
// is looked up in the rendered frame or, off screen and close to the hole, in the environment
// map (Skybox()). The pass runs before bloom and tone mapping.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::accretion_disk::{Hole, VolRay, VOLUMETRIC_STOP, SPEED_OF_LIGHT, volume_step, sd_bounding, star_surface, transmission, linearstep, sstep};

struct Lens {
    // x,y: tan(fov/2) in x and y, z: frame counter (noise animation), w: hole count.
    view: vec4<f32>,
    // Columns of the view -> environment-map rotation (SE's SkyRot); sky0.w: 1 when the
    // environment map holds the sky.
    sky0: vec4<f32>,
    sky1: vec4<f32>,
    sky2: vec4<f32>,
    holes: array<Hole, 4>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> lens: Lens;
// Environment map (SE's EnvMap): six cube faces, +x -x +y -y +z -z of the world axes, each
// rendered by a camera looking along the face axis with the up vector of `face_axes`.
@group(0) @binding(6) var env_px: texture_2d<f32>;
@group(0) @binding(7) var env_nx: texture_2d<f32>;
@group(0) @binding(8) var env_py: texture_2d<f32>;
@group(0) @binding(9) var env_ny: texture_2d<f32>;
@group(0) @binding(10) var env_pz: texture_2d<f32>;
@group(0) @binding(11) var env_nz: texture_2d<f32>;

const MAX_MARCH: i32 = 512;
// gr_geodesic.glh: max_scale, the numerical derivative eps.
const MAX_SCALE: f32 = 100.0;
const GEO_EPS: f32 = 0.02;

struct Trace {
    // Disk/jet emission accumulated along the ray (before volumBright).
    emission: vec3<f32>,
    // Optical depth in front of the background.
    sigma: f32,
    // Direction (view space) at which the ray leaves to infinity.
    dir: vec3<f32>,
    // Fell through the horizon (SE: passParam 1).
    captured: bool,
    // Hit the star's surface; `surface` is its emission (before volumBright).
    hit: bool,
    surface: vec3<f32>,
};

fn empty_trace(d: vec3<f32>) -> Trace {
    return Trace(vec3<f32>(0.0), 0.0, d, false, false, vec3<f32>(0.0));
}

// Disk frame of SE's shader: x = reference axis, y = spin axis, z = y x x (the gas then
// orbits prograde about y with SE's velocity (-z, 0, x)).
fn to_disk(h: Hole, v: vec3<f32>) -> vec3<f32> {
    let n = h.axis_in.xyz;
    let x = h.xref_out.xyz;
    return vec3<f32>(dot(v, x), dot(v, n), dot(v, cross(n, x)));
}

fn from_disk(h: Hole, v: vec3<f32>) -> vec3<f32> {
    let n = h.axis_in.xyz;
    let x = h.xref_out.xyz;
    return v.x * x + v.y * n + v.z * cross(n, x);
}

// SphereIntersection (entry, exit distances; 1e6 for a miss), with the closest-approach form
// of the discriminant so that a camera far outside keeps its precision.
fn sphere_hit(ro: vec3<f32>, rd: vec3<f32>, r: f32) -> vec2<f32> {
    let b = dot(ro, rd);
    let perp = ro - b * rd;
    let h2 = r * r - dot(perp, perp);
    if h2 < 0.0 {
        return vec2<f32>(1.0e6);
    }
    let s = sqrt(h2);
    return vec2<f32>(-s - b, s - b);
}

// ---------------------------------------------------------------------------
// Kerr-Newman metric in Kerr-Schild coordinates (gr_geodesic.glh): g = eta + f l l with
// l = (1, k), coordinates q = (t, x, y, z) = (t, X, Z, Y) of the disk frame in units of
// M = gravRadius / 2, mass KerrM, spin a = KerrA, charge^2 = KerrQ2.

fn kerr_r2(h: Hole, p: vec3<f32>) -> f32 {
    let a2 = h.metric.z;
    let rho = dot(p, p) - a2;
    return abs(0.5 * (rho + sqrt(rho * rho + 4.0 * a2 * p.z * p.z)));
}

fn kerr_k(h: Hole, p: vec3<f32>) -> vec4<f32> {
    let a = h.metric.x;
    let a2 = h.metric.z;
    let r2 = kerr_r2(h, p);
    let r = sqrt(r2);
    let inv = 1.0 / (r2 + a2 + 1.0e-5);
    let k = vec3<f32>((r * p.x - a * p.y) * inv, (r * p.y + a * p.x) * inv, p.z / (r + 1.0e-5));
    let f = r2 * (2.0 * h.gr.x * r - h.metric.w) / (r2 * r2 + a2 * p.z * p.z + 1.0e-5);
    return vec4<f32>(k, f);
}

// L(qt, G(q)) = qt . G qt.
fn kn_lagrangian(h: Hole, v: vec4<f32>, pos: vec3<f32>) -> f32 {
    let k = kerr_k(h, pos);
    let l = v.x + dot(k.xyz, v.yzw);
    return -v.x * v.x + dot(v.yzw, v.yzw) + k.w * l * l;
}

// G qt (ToMomentum before normalising).
fn kn_lower(h: Hole, v: vec4<f32>, pos: vec3<f32>) -> vec4<f32> {
    let k = kerr_k(h, pos);
    let l = v.x + dot(k.xyz, v.yzw);
    return vec4<f32>(-v.x, v.yzw) + k.w * l * vec4<f32>(1.0, k.xyz);
}

// Ginv p, SE's inverse: eta + f / |k|^2 (k.p - p0) (1, -k).
fn kn_raise(h: Hole, p: vec4<f32>, pos: vec3<f32>) -> vec4<f32> {
    let k = kerr_k(h, pos);
    let s = k.w / dot(k.xyz, k.xyz) * (dot(k.xyz, p.yzw) - p.x);
    return vec4<f32>(-p.x, p.yzw) + s * vec4<f32>(1.0, -k.xyz);
}

// GeodesicRay: position q, its derivative qt, momentum p.
struct Geo {
    q: vec4<f32>,
    qt: vec4<f32>,
    p: vec4<f32>,
};

// GetTimeStep: longer steps far out, shorter near the ring singularity; 0 (captured) inside
// r = 1.1 unless the spin makes the hole naked.
fn kn_time_step(h: Hole, g: Geo) -> f32 {
    let r = sqrt(kerr_r2(h, g.q.yzw));
    let dt1 = clamp(inverseSqrt(dot(g.p, g.p)), 0.1, 4.0);
    var dt = (h.gr.z * 0.8333 + 0.4) * dt1 * mix(0.06, 20.0, max(r - 1.0, 0.0) / 120.0);
    if r < 1.0 {
        dt = dt * (0.05 + 0.95 * r * r * r);
    }
    if r < 1.1 && h.metric.x < 1.0 {
        return 0.0;
    }
    return dt;
}

// One step of TraceGeodesic: p += HamiltonianGradient dt (forward differences of the
// Lagrangian at fixed qt = Ginv p), q += 2 qt dt.
fn kn_advance(h: Hole, g: ptr<function, Geo>, dt: f32) {
    let pos = (*g).q.yzw;
    let qt = kn_raise(h, (*g).p, pos);
    let h0 = dot((*g).p, qt);
    let delta = GEO_EPS * max(length(pos), 1.0e-4);
    let lx = kn_lagrangian(h, qt, pos + vec3<f32>(delta, 0.0, 0.0));
    let ly = kn_lagrangian(h, qt, pos + vec3<f32>(0.0, delta, 0.0));
    let lz = kn_lagrangian(h, qt, pos + vec3<f32>(0.0, 0.0, delta));
    (*g).p = (*g).p + vec4<f32>(0.0, (vec3<f32>(lx, ly, lz) - h0) / delta) * dt;
    (*g).q = (*g).q + 2.0 * qt * dt;
    (*g).qt = qt;
}

// SE's integration sphere (M units) and step amplification: max_distance = clamp(2 |ro|,
// 100, 300), dt_amplify = 1 + 0.1 (max_distance / 100 - 1), at most invGravRadius.
fn kn_reach(h: Hole, ro: vec3<f32>) -> vec2<f32> {
    let reach = clamp(2.0 * length(ro), MAX_SCALE, 3.0 * MAX_SCALE);
    return vec2<f32>(min(reach, h.gr.y), 1.0 + 0.1 * (reach / MAX_SCALE - 1.0));
}

fn kn_start(h: Hole, ro: vec3<f32>, rd: vec3<f32>) -> Geo {
    var g: Geo;
    g.q = vec4<f32>(0.0, ro.x, ro.z, ro.y);
    g.qt = 2.0 * vec4<f32>(1.0, rd.x, rd.z, rd.y);
    g.p = normalize(kn_lower(h, g.qt, g.q.yzw));
    return g;
}

// The surface of the star along a chord from `a` (units, disk frame) in direction `dir` over
// `len`: the entry distance, or -1 (StarSurface's SphereIntersection test).
fn star_entry(h: Hole, a: vec3<f32>, dir: vec3<f32>, len: f32) -> f32 {
    if h.star.y <= 0.0 {
        return -1.0;
    }
    let t = sphere_hit(a, dir, h.star.y).x;
    if t > 1.0e5 || t < 0.0 || t >= len {
        return -1.0;
    }
    return t;
}

// Ray-march one straight chord (disk frame, units) through the disk and jet volume
// (VolumetricIntegration). `s0`: path length at `a`; `t0`, `t1`: SE's timeSegment (light time in
// M units, negative); `next`: path length of the next sample of the marching lattice (kept
// across chords like SE's rayProp.dist).
fn march_chord(h: Hole, a: vec3<f32>, dir: vec3<f32>, len: f32, s0: f32, t0: f32, t1: f32, next: ptr<function, f32>, vr: ptr<function, VolRay>) {
    if len <= 0.0 {
        return;
    }
    let step = h.disk2.w;
    let c = SPEED_OF_LIGHT * h.gr.x;
    var t = *next - s0;
    loop {
        if t >= len || (*vr).steps >= MAX_MARCH || (*vr).sigma > VOLUMETRIC_STOP {
            break;
        }
        (*vr).steps = (*vr).steps + 1;
        let p = a + dir * t;
        let sd = sd_bounding(h, p);
        if sd < 0.0 {
            // Emission time: the disk loop phase plus the (negative) light time.
            let time = fract(h.disk1.w + mix(t0, t1, t / len) / c);
            volume_step(h, p, dir, step, s0 + t, time, vr);
            t = t + step;
        } else {
            t = t + step * max(ceil(sd / step), 1.0);
        }
    }
    *next = s0 + t;
}

// TraceGeodesic with the volume and the star's surface (the centre ray of a pixel).
fn trace_ray(h: Hole, d: vec3<f32>, jitter: f32) -> Trace {
    var res = empty_trace(d);
    let vol = h.disk1.z > 0.0;
    var vr: VolRay;
    vr.color = vec3<f32>(0.0);
    vr.transm = vec3<f32>(1.0);
    vr.sigma = 0.0;
    vr.steps = 0;
    vr.volume_steps = 0;
    var next = jitter * h.disk2.w;
    var s = 0.0;

    // Camera in geodesic units (M), disk frame.
    let ro_cam = to_disk(h, -h.dir_d.xyz) * (2.0 * h.dir_d.w);
    let rd_init = to_disk(h, d);
    let reach = kn_reach(h, ro_cam);
    let maxd = reach.x;
    var t_in = sphere_hit(ro_cam, rd_init, maxd);
    if t_in.x < 0.0 && t_in.y < 0.0 {
        t_in = vec2<f32>(1.0e6);
    }
    var ro = ro_cam;
    var time = 0.0;
    if t_in.x >= 0.0 {
        // Outside: the straight part up to the sphere (the whole ray when it misses it).
        if vol {
            march_chord(h, ro_cam * 0.5, rd_init, t_in.x * 0.5, s, 0.0, -t_in.x, &next, &vr);
        }
        if t_in.x >= 1.0e6 {
            res.emission = vr.color;
            res.sigma = vr.sigma;
            return res;
        }
        s = t_in.x * 0.5;
        let b = dot(ro_cam, rd_init);
        ro = (ro_cam - b * rd_init) - (-t_in.x - b) * rd_init;
        time = t_in.x;
    }

    var g = kn_start(h, ro, rd_init);
    var rd = rd_init;
    var rd0 = rd_init;
    var r0 = ro;
    var minl = 1.0e10;
    var dist = 1.0e10;
    var t_end = -time;
    let max_steps = select(256, 1500, h.metric.x > 1.0);
    for (var i = 0; i < max_steps; i = i + 1) {
        let dt = reach.y * kn_time_step(h, g);
        if dt <= 0.0 {
            res.captured = true;
            break;
        }
        let t_a = -g.q.x - time;
        kn_advance(h, &g, dt);
        rd0 = rd;
        rd = normalize(g.qt.ywz);
        let r1 = g.q.ywz;
        t_end = -g.q.x - time;
        dist = length(r1);
        minl = min(minl, dist);
        if dist >= maxd {
            // Fix the direction discontinuity at the sphere.
            let td2 = sphere_hit(r1, rd, maxd);
            let delta = distance(r1, r0);
            let lerp = clamp((delta + td2.y) / max(delta, 1.0e-6), 0.0, 1.0);
            rd = mix(rd, rd0, 1.0 - lerp);
        }
        // EvaluateIntersections on the chord r0 -> r1 (geodesic units -> units: x 0.5).
        let seg = r1 - r0;
        let seg_len = length(seg);
        if seg_len > 0.0 {
            let sd = seg / seg_len;
            if vol {
                march_chord(h, r0 * 0.5, sd, seg_len * 0.5, s, t_a, t_end, &next, &vr);
            }
            let e = star_entry(h, r0 * 0.5, sd, seg_len * 0.5);
            if e >= 0.0 {
                res.hit = true;
                res.surface = star_surface(h, normalize(r0 * 0.5 + sd * e), sd);
                break;
            }
            s = s + seg_len * 0.5;
        }
        r0 = r1;
        if vr.sigma > VOLUMETRIC_STOP || dist >= maxd {
            break;
        }
    }
    if !res.hit && !res.captured && dist >= maxd && vol {
        // Escaped: the straight rest of the ray.
        march_chord(h, r0 * 0.5, rd, 0.5e5, s, t_end, t_end - 1.0e5, &next, &vr);
    }
    // Avoid a sharp transition at the sphere.
    rd = mix(rd_init, rd, sstep(maxd, 0.0, minl));
    res.emission = vr.color;
    res.sigma = vr.sigma;
    res.dir = from_disk(h, rd);
    return res;
}

// The same geodesic without the volume (background sub-samples).
fn trace_geodesic(h: Hole, d: vec3<f32>) -> Trace {
    var res = empty_trace(d);
    let ro_cam = to_disk(h, -h.dir_d.xyz) * (2.0 * h.dir_d.w);
    let rd_init = to_disk(h, d);
    let reach = kn_reach(h, ro_cam);
    let maxd = reach.x;
    let t_in = sphere_hit(ro_cam, rd_init, maxd);
    if t_in.y < 0.0 || t_in.x >= 1.0e6 {
        return res;
    }
    var ro = ro_cam;
    if t_in.x >= 0.0 {
        let b = dot(ro_cam, rd_init);
        ro = (ro_cam - b * rd_init) - (-t_in.x - b) * rd_init;
    }
    var g = kn_start(h, ro, rd_init);
    var rd = rd_init;
    var rd0 = rd_init;
    var r0 = ro;
    var minl = 1.0e10;
    let max_steps = select(256, 1500, h.metric.x > 1.0);
    for (var i = 0; i < max_steps; i = i + 1) {
        let dt = reach.y * kn_time_step(h, g);
        if dt <= 0.0 {
            res.captured = true;
            break;
        }
        kn_advance(h, &g, dt);
        rd0 = rd;
        rd = normalize(g.qt.ywz);
        let r1 = g.q.ywz;
        let dist = length(r1);
        minl = min(minl, dist);
        if dist >= maxd {
            let td2 = sphere_hit(r1, rd, maxd);
            let delta = distance(r1, r0);
            let lerp = clamp((delta + td2.y) / max(delta, 1.0e-6), 0.0, 1.0);
            rd = mix(rd, rd0, 1.0 - lerp);
        }
        let seg = r1 - r0;
        let seg_len = length(seg);
        if seg_len > 0.0 {
            let sd = seg / seg_len;
            let e = star_entry(h, r0 * 0.5, sd, seg_len * 0.5);
            if e >= 0.0 {
                res.hit = true;
                res.surface = star_surface(h, normalize(r0 * 0.5 + sd * e), sd);
                break;
            }
        }
        r0 = r1;
        if dist >= maxd {
            break;
        }
    }
    rd = mix(rd_init, rd, sstep(maxd, 0.0, minl));
    res.dir = from_disk(h, rd);
    return res;
}

fn load_scene(p: vec2<i32>) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(scene));
    return textureLoad(scene, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0).rgb;
}

// Bilinear lookup at a continuous pixel position (pixel centres at +0.5).
fn sample_scene(pos: vec2<f32>) -> vec3<f32> {
    let q = pos - vec2<f32>(0.5);
    let i = vec2<i32>(floor(q));
    let f = fract(q);
    let a = load_scene(i);
    let b = load_scene(i + vec2<i32>(1, 0));
    let c = load_scene(i + vec2<i32>(0, 1));
    let d = load_scene(i + vec2<i32>(1, 1));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Mirror-repeat a screen coordinate in 0..1 so lookups that leave the screen land
// on a reflected copy of it instead of smearing the border row.
fn mirror01(t: f32) -> f32 {
    let s = t - 2.0 * floor(t * 0.5);
    return select(2.0 - s, s, s <= 1.0);
}

// Forward and up axes of environment face `f` (blackholes.rs ENV_FACES).
fn face_axes(f: i32) -> array<vec3<f32>, 2> {
    switch f {
        case 0: { return array<vec3<f32>, 2>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0)); }
        case 1: { return array<vec3<f32>, 2>(vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0)); }
        case 2: { return array<vec3<f32>, 2>(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, -1.0)); }
        case 3: { return array<vec3<f32>, 2>(vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0)); }
        case 4: { return array<vec3<f32>, 2>(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0)); }
        default: { return array<vec3<f32>, 2>(vec3<f32>(0.0, 0.0, -1.0), vec3<f32>(0.0, 1.0, 0.0)); }
    }
}

// Environment map lookup in direction `v` (world axes): the face camera saw direction
// F + x R + y U at NDC (x, y), R = F x U, 90 degree field of view.
fn sample_env(v: vec3<f32>) -> vec3<f32> {
    let a = abs(v);
    var f = 4;
    if a.x >= a.y && a.x >= a.z {
        f = select(1, 0, v.x > 0.0);
    } else if a.y >= a.z {
        f = select(3, 2, v.y > 0.0);
    } else {
        f = select(5, 4, v.z > 0.0);
    }
    let ax = face_axes(f);
    let fw = ax[0];
    let up = ax[1];
    let rt = cross(fw, up);
    let k = 1.0 / dot(v, fw);
    let uv = vec2<f32>(0.5 + 0.5 * dot(v, rt) * k, 0.5 - 0.5 * dot(v, up) * k);
    switch f {
        case 0: { return textureSampleLevel(env_px, scene_sampler, uv, 0.0).rgb; }
        case 1: { return textureSampleLevel(env_nx, scene_sampler, uv, 0.0).rgb; }
        case 2: { return textureSampleLevel(env_py, scene_sampler, uv, 0.0).rgb; }
        case 3: { return textureSampleLevel(env_ny, scene_sampler, uv, 0.0).rgb; }
        case 4: { return textureSampleLevel(env_pz, scene_sampler, uv, 0.0).rgb; }
        default: { return textureSampleLevel(env_nz, scene_sampler, uv, 0.0).rgb; }
    }
}

// SkyBlueShift: the sky seen from deep in the potential well is blue-shifted and brighter.
fn sky_blue_shift(h: Hole, c_in: vec3<f32>) -> vec3<f32> {
    let eye = h.dir_d.w;
    let fade = sstep(1.0, 0.9, eye * h.params.y);
    // gravRadius * starRadiusScale / EyeDist: KerrM / distance in units.
    let one_plus_z = inverseSqrt(max(1.0 - h.gr.x * fade / eye, 1.0e-3));
    let z2 = one_plus_z * one_plus_z;
    let flux = z2 * z2;
    let shift = (one_plus_z - 1.0) * 2.0;
    let r2g = 0.299 / 0.587;
    let g2b = 0.587 / 0.114;
    var c = c_in;
    c = mix(c, vec3<f32>(0.3 * c.r, c.r * r2g, c.g * g2b), clamp(shift, 0.0, 1.0));
    c = mix(c, vec3<f32>(0.7 * c.r, c.r * r2g, c.g * g2b), clamp(shift - 1.0, 0.0, 1.0));
    return c * flux;
}

// Skybox(): the rendered frame where the escaping ray lands on screen, the environment map at
// and beyond the screen edges and close to the hole (blendFactor), then SkyBlueShift.
fn sky(h: Hole, o: vec3<f32>, d0: vec3<f32>, dims: vec2<f32>) -> vec3<f32> {
    let front = o.z < -1.0e-4;
    let zf = max(-o.z, 1.0e-4);
    let ndc = vec2<f32>(o.x / zf / lens.view.x, o.y / zf / lens.view.y);
    var colour = vec3<f32>(0.0);
    if lens.sky0.w > 0.5 {
        let fb = ndc * 0.5 + 0.5;
        var frame = vec3<f32>(0.0);
        var edge = 0.0;
        if front {
            let deflection = dot(o - d0, o - d0);
            let inner = 0.15 * clamp((0.2 - h.dir_d.w * 1.0e-3) * 10.0, 0.0001, 1.0);
            let zone = mix(0.001, inner, linearstep(0.0, 0.01, deflection));
            let dd = (fb - clamp(fb, vec2<f32>(zone), vec2<f32>(1.0 - zone))) / zone;
            edge = clamp(1.0 - sqrt(dot(dd, dd)), 0.0, 1.0);
            if edge > 0.0 {
                let uv = clamp(vec2<f32>(fb.x, 1.0 - fb.y), vec2<f32>(0.0), vec2<f32>(1.0));
                frame = sample_scene(clamp(uv * dims, vec2<f32>(0.5), dims - vec2<f32>(0.5)));
            }
        }
        let blend = edge * h.light.w;
        let ev = o.x * lens.sky0.xyz + o.y * lens.sky1.xyz + o.z * lens.sky2.xyz;
        var env = vec3<f32>(0.0);
        if blend < 1.0 {
            env = sample_env(ev);
        }
        colour = mix(env, frame, blend);
    } else {
        // No environment map yet: reflect small overshoots back onto the screen and fade to
        // black further out.
        let over = max(abs(ndc.x), abs(ndc.y));
        var vis = 1.0 - smoothstep(1.0, 2.4, over);
        if o.z > -0.05 {
            vis = 0.0;
        }
        if vis > 0.0 {
            let uv = vec2<f32>(mirror01(ndc.x * 0.5 + 0.5), mirror01(0.5 - ndc.y * 0.5));
            colour = vis * sample_scene(clamp(uv * dims, vec2<f32>(0.5), dims - vec2<f32>(0.5)));
        }
    }
    return sky_blue_shift(h, colour);
}

// Interleaved gradient noise, animated per frame (SE: BlueNoiseMap at NoiseParams.y).
fn step_jitter(px: vec2<f32>) -> f32 {
    let f = lens.view.z;
    let p = px + vec2<f32>(5.588238, 5.588238) * (f - 64.0 * floor(f / 64.0));
    return fract(52.9829189 * fract(0.06711056 * p.x + 0.00583715 * p.y));
}

fn view_dir(p: vec2<f32>, dims: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(p.x / dims.x * 2.0 - 1.0, 1.0 - p.y / dims.y * 2.0);
    return normalize(vec3<f32>(ndc.x * lens.view.x, ndc.y * lens.view.y, -1.0));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(scene));
    let px = in.position.xy;
    let count = i32(lens.view.w);

    // Pick the hole whose influence region contains this pixel most deeply.
    let d0 = view_dir(px, dims);
    var best = -1;
    var best_score = 1.0;
    for (var k = 0; k < count; k = k + 1) {
        let hk = lens.holes[k];
        let chord = length(d0 - hk.dir_d.xyz);
        let ang = 2.0 * asin(clamp(0.5 * chord, 0.0, 1.0));
        let score = ang / hk.params.x;
        if score < best_score {
            best_score = score;
            best = k;
        }
    }
    let original = textureLoad(scene, vec2<i32>(px), 0);
    // Debug (SE_BH_ENVTEST): show the environment map instead of the frame.
    if lens.sky1.w > 0.5 {
        let ev = d0.x * lens.sky0.xyz + d0.y * lens.sky1.xyz + d0.z * lens.sky2.xyz;
        return vec4<f32>(sample_env(ev), 1.0);
    }
    if best < 0 {
        return original;
    }
    let h = lens.holes[best];

    // The centre ray carries the volume (disk and jets).
    let tc = trace_ray(h, d0, step_jitter(px));
    let emission = tc.emission * h.params.w;
    let trans = transmission(tc.sigma);

    // Impact parameter of the centre ray decides how many background sub-samples we spend.
    let cosa = dot(d0, h.dir_d.xyz);
    let sina = length(d0 - cosa * h.dir_d.xyz);
    let b0 = h.dir_d.w * sina;
    var bg = vec3<f32>(0.0);
    if b0 < 14.0 || h.dir_d.w < 20.0 {
        // 4 rotated-grid samples for a crisp, anti-aliased shadow edge, photon ring and star
        // limb (the sky, or the star's surface behind the centre ray's volume).
        var offs = array<vec2<f32>, 4>(
            vec2<f32>(-0.125, -0.375), vec2<f32>(0.375, -0.125),
            vec2<f32>(0.125, 0.375), vec2<f32>(-0.375, 0.125));
        for (var s = 0; s < 4; s = s + 1) {
            let d = view_dir(px + offs[s], dims);
            let ts = trace_geodesic(h, d);
            if ts.hit {
                bg = bg + ts.surface * h.params.w;
            } else if !ts.captured {
                bg = bg + sky(h, ts.dir, d, dims);
            }
        }
        bg = bg * 0.25;
    } else if tc.hit {
        bg = tc.surface * h.params.w;
    } else if !tc.captured {
        bg = sky(h, tc.dir, d0, dims);
    }
    var colour = bg * trans + emission;    // Keep NaN / overflow out of the frame (bloom would spread it over the screen).
    if any(colour != colour) {
        colour = vec3<f32>(0.0);
    }
    colour = min(colour, vec3<f32>(60000.0));
    // Blend to the untouched scene at the border of the influence region so the
    // pass has no visible seam.
    let fade = smoothstep(0.7, 1.0, best_score);
    colour = mix(colour, original.rgb, fade);
    return vec4<f32>(colour, original.a);
}
