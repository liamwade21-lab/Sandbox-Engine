// Black-hole gravitational lensing + volumetric accretion disk and jets, as an HDR
// post-process (SE's einstein.glsl + gr_geodesic.glh + gr_intersection.glh, single-pass
// "COMPUTE + RENDER" mode).
//
// For every pixel near a black hole the (reversed) light ray is traced through the
// Schwarzschild geometry in the plane spanned by the observer, the hole and the ray,
// integrating   u'' + u = 3/2 u^2   (u = r_s / r, derivative w.r.t. the polar angle phi in
// that plane). Like SE's EvaluateIntersections, each step's straight chord is ray-marched
// through the disk and jet volume (accretion_disk.wesl). The ray then either falls through
// the horizon (black shadow) or escapes, and its direction at infinity is looked up in the
// rendered frame or, off screen and close to the hole, in the environment map (Skybox()).
// The pass runs before bloom and tone mapping.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::accretion_disk::{Hole, VolRay, PI, VOLUMETRIC_STOP, volume_step, sd_bounding, transmission, linearstep, sstep};

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

const MAX_STEPS: i32 = 220;
const MAX_MARCH: i32 = 512;

struct Trace {
    // Disk/jet emission accumulated along the ray (before volumBright).
    emission: vec3<f32>,
    // Optical depth in front of the background.
    sigma: f32,
    // Direction (view space) at which the ray leaves to infinity.
    dir: vec3<f32>,
    captured: bool,
};

fn accel(u: f32) -> f32 {
    return 1.5 * u * u - u;
}

// Disk frame of SE's shader: x = reference axis, y = spin axis, z = y x x (the gas then
// orbits prograde about y with SE's velocity (-z, 0, x)).
fn to_disk(h: Hole, v: vec3<f32>) -> vec3<f32> {
    let n = h.axis_in.xyz;
    let x = h.xref_out.xyz;
    return vec3<f32>(dot(v, x), dot(v, n), dot(v, cross(n, x)));
}

// Ray-march one straight chord (positions relative to the hole, view space, r_s) through the
// disk and jet volume. `s0`: path length at `a`; `next`: path length of the next sample of
// the marching lattice (kept across chords like SE's rayProp.dist).
fn march_chord(h: Hole, a: vec3<f32>, b: vec3<f32>, s0: f32, next: ptr<function, f32>, vr: ptr<function, VolRay>) {
    let seg = b - a;
    let len = length(seg);
    if len <= 0.0 {
        return;
    }
    let dir = to_disk(h, seg / len);
    let o = to_disk(h, a);
    let step = h.disk2.w;
    var t = *next - s0;
    loop {
        if t >= len || (*vr).steps >= MAX_MARCH || (*vr).sigma > VOLUMETRIC_STOP {
            break;
        }
        (*vr).steps = (*vr).steps + 1;
        let p = o + dir * t;
        let sd = sd_bounding(h, p);
        if sd < 0.0 {
            // Emission time: the disk loop phase minus the light travel time (SE's
            // rayProp.time / SPEED_OF_LIGHT, geodesic units of M = r_s / 2).
            let time = fract(h.disk1.w - 2.0 * (s0 + t) / 16000.0);
            volume_step(h, p, dir, step, s0 + t, time, vr);
            t = t + step;
        } else {
            t = t + step * max(ceil(sd / step), 1.0);
        }
    }
    *next = s0 + t;
}

fn trace_ray(h: Hole, d: vec3<f32>, jitter: f32) -> Trace {
    var res: Trace;
    res.emission = vec3<f32>(0.0);
    res.sigma = 0.0;
    res.dir = d;
    res.captured = false;

    let c = h.dir_d.xyz;
    let dist = h.dir_d.w;
    let e1 = -c;
    let cosa = dot(d, c);
    let t = d - cosa * c;
    var sina = length(t);
    var e2: vec3<f32>;
    if sina < 1.0e-6 {
        sina = 1.0e-6;
        let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(c.y) > 0.9);
        e2 = normalize(cross(c, helper));
    } else {
        e2 = t / sina;
    }

    let u0 = 1.0 / dist;
    var u = u0;
    var w = u0 * sqrt(max(1.0 - u0, 1.0e-4)) * cosa / sina;

    let vol = h.disk1.z > 0.0;
    var vr: VolRay;
    vr.color = vec3<f32>(0.0);
    vr.transm = vec3<f32>(1.0);
    vr.sigma = 0.0;
    vr.steps = 0;
    vr.volume_steps = 0;
    var next = jitter * h.disk2.w;
    var s = 0.0;
    var p_prev = dist * e1;

    // Stop integrating once outbound and beyond twice the volume's reach.
    let u_stop = min(0.02, 0.5 / max(h.disk5.w, 1.0));

    var phi = 0.0;
    var captured = false;
    var escaped = false;
    for (var i = 0; i < MAX_STEPS; i = i + 1) {
        if u >= 1.0 {
            captured = true;
            break;
        }
        // Outbound and far enough from the turning point for the asymptotic formula.
        if w < 0.0 && u < u_stop && w * w > 16.0 * abs(u * u * u) {
            escaped = true;
            break;
        }
        var h_step = 0.05;
        if u < 0.02 {
            h_step = 0.3;
        } else if u < 0.06 {
            h_step = 0.12;
        }
        // RK4 on (u, w)
        let k1u = w;
        let k1w = accel(u);
        let k2u = w + 0.5 * h_step * k1w;
        let k2w = accel(u + 0.5 * h_step * k1u);
        let k3u = w + 0.5 * h_step * k2w;
        let k3w = accel(u + 0.5 * h_step * k2u);
        let k4u = w + h_step * k3w;
        let k4w = accel(u + h_step * k3u);
        u = u + h_step / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u);
        w = w + h_step / 6.0 * (k1w + 2.0 * k2w + 2.0 * k3w + k4w);
        phi = phi + h_step;

        if vol && u > 1.0e-6 && vr.steps < MAX_MARCH && vr.sigma <= VOLUMETRIC_STOP {
            let p = (cos(phi) * e1 + sin(phi) * e2) / u;
            march_chord(h, p_prev, p, s, &next, &vr);
            s = s + length(p - p_prev);
            p_prev = p;
        }
    }
    res.emission = vr.color;
    res.sigma = vr.sigma;
    if !captured && !escaped {
        // Ran out of steps while circling the photon sphere: treat as captured.
        captured = true;
    }
    if captured {
        res.captured = true;
        return res;
    }
    // Remaining sweep to infinity: flat-space arcsine plus the first-order
    // correction of the u^3 term in (du/dphi)^2 = K - u^2 + u^3. The last (large)
    // step may overshoot u = 0 slightly; the odd extension handles that.
    let ua = abs(u);
    let k = w * w + ua * ua * (1.0 - ua);
    var rem = asin(clamp(ua * inverseSqrt(max(k, 1.0e-20)), 0.0, 1.0));
    let q2 = w * w - ua * ua * ua;
    if q2 > 4.0 * ua * ua * ua {
        let q = sqrt(q2);
        rem = rem - (0.5 * ua * ua / q + q - sqrt(k));
    }
    let phi_end = phi + select(-rem, rem, u >= 0.0);
    res.dir = cos(phi_end) * e1 + sin(phi_end) * e2;
    return res;
}

// The same geodesic without the volume (background sub-samples).
fn trace_geodesic(h: Hole, d: vec3<f32>) -> Trace {
    var res: Trace;
    res.emission = vec3<f32>(0.0);
    res.sigma = 0.0;
    res.dir = d;
    res.captured = false;

    let c = h.dir_d.xyz;
    let dist = h.dir_d.w;
    let e1 = -c;
    let cosa = dot(d, c);
    let t = d - cosa * c;
    var sina = length(t);
    var e2: vec3<f32>;
    if sina < 1.0e-6 {
        sina = 1.0e-6;
        let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(c.y) > 0.9);
        e2 = normalize(cross(c, helper));
    } else {
        e2 = t / sina;
    }

    let u0 = 1.0 / dist;
    var u = u0;
    var w = u0 * sqrt(max(1.0 - u0, 1.0e-4)) * cosa / sina;

    // Stop integrating once outbound and beyond twice the volume's reach.
    let u_stop = min(0.02, 0.5 / max(h.disk5.w, 1.0));

    var phi = 0.0;
    var captured = false;
    var escaped = false;
    for (var i = 0; i < MAX_STEPS; i = i + 1) {
        if u >= 1.0 {
            captured = true;
            break;
        }
        // Outbound and far enough from the turning point for the asymptotic formula.
        if w < 0.0 && u < u_stop && w * w > 16.0 * abs(u * u * u) {
            escaped = true;
            break;
        }
        var h_step = 0.05;
        if u < 0.02 {
            h_step = 0.3;
        } else if u < 0.06 {
            h_step = 0.12;
        }
        // RK4 on (u, w)
        let k1u = w;
        let k1w = accel(u);
        let k2u = w + 0.5 * h_step * k1w;
        let k2w = accel(u + 0.5 * h_step * k1u);
        let k3u = w + 0.5 * h_step * k2w;
        let k3w = accel(u + 0.5 * h_step * k2u);
        let k4u = w + h_step * k3w;
        let k4w = accel(u + h_step * k3u);
        u = u + h_step / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u);
        w = w + h_step / 6.0 * (k1w + 2.0 * k2w + 2.0 * k3w + k4w);
        phi = phi + h_step;
    }
    if !captured && !escaped {
        // Ran out of steps while circling the photon sphere: treat as captured.
        captured = true;
    }
    if captured {
        res.captured = true;
        return res;
    }
    // Remaining sweep to infinity: flat-space arcsine plus the first-order
    // correction of the u^3 term in (du/dphi)^2 = K - u^2 + u^3. The last (large)
    // step may overshoot u = 0 slightly; the odd extension handles that.
    let ua = abs(u);
    let k = w * w + ua * ua * (1.0 - ua);
    var rem = asin(clamp(ua * inverseSqrt(max(k, 1.0e-20)), 0.0, 1.0));
    let q2 = w * w - ua * ua * ua;
    if q2 > 4.0 * ua * ua * ua {
        let q = sqrt(q2);
        rem = rem - (0.5 * ua * ua / q + q - sqrt(k));
    }
    let phi_end = phi + select(-rem, rem, u >= 0.0);
    res.dir = cos(phi_end) * e1 + sin(phi_end) * e2;
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
    let one_plus_z = inverseSqrt(max(1.0 - fade / eye, 1.0e-3));
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
        // 4 rotated-grid samples for a crisp, anti-aliased shadow edge and photon ring.
        var offs = array<vec2<f32>, 4>(
            vec2<f32>(-0.125, -0.375), vec2<f32>(0.375, -0.125),
            vec2<f32>(0.125, 0.375), vec2<f32>(-0.375, 0.125));
        for (var s = 0; s < 4; s = s + 1) {
            let d = view_dir(px + offs[s], dims);
            let ts = trace_geodesic(h, d);
            if !ts.captured {
                bg = bg + sky(h, ts.dir, d, dims);
            }
        }
        bg = bg * 0.25;
    } else if !tc.captured {
        bg = sky(h, tc.dir, d0, dims);
    }
    var colour = bg * trans + emission;
    // Keep NaN / overflow out of the frame (bloom would spread it over the screen).
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
