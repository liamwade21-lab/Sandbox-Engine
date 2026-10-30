// Black-hole gravitational lensing + accretion disk, as an HDR post-process.
//
// For every pixel near a black hole we trace the (reversed) light ray through the
// Schwarzschild geometry in the plane spanned by the observer, the hole and the
// ray, integrating   u'' + u = 3/2 u^2   (u = r_s / r, derivative w.r.t. the
// polar angle phi in that plane). While tracing we
//   * detect crossings of the disk's equatorial plane and accumulate the disk's
//     emission (Doppler + gravitational shift of the Keplerian gas),
//   * stop when the ray falls through the horizon (-> black shadow), or escapes,
//     in which case the asymptotic direction is looked up in the already rendered
//     scene (the lensed background).
// The pass runs before bloom and tone mapping, so the photon ring and the beamed
// side of the disk bloom naturally.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::accretion_disk::{DiskSample, disk_sample, PI};

struct Hole {
    // xyz: unit view-space direction camera -> hole, w: camera distance in r_s.
    dir_d: vec4<f32>,
    // xyz: disk normal (spin axis), w: inner radius in r_s (<= 0: no disk).
    axis_in: vec4<f32>,
    // xyz: disk azimuth reference axis, w: outer radius in r_s.
    xref_out: vec4<f32>,
    // x: influence radius (rad), y: brightness, z: peak colour temperature (K),
    // w: turbulence contrast.
    params: vec4<f32>,
};

struct Lens {
    // x,y: tan(fov/2) in x and y, z: animation time (s), w: hole count.
    view: vec4<f32>,
    holes: array<Hole, 4>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> lens: Lens;

const MAX_STEPS: i32 = 220;
const B_CRIT: f32 = 2.598076;

struct Trace {
    // Disk emission accumulated along the ray (already attenuated).
    emission: vec3<f32>,
    // Fraction of the background still visible (0 if captured or hidden by the disk).
    trans: f32,
    // Direction (view space) at which the ray leaves to infinity.
    dir: vec3<f32>,
};

fn accel(u: f32) -> f32 {
    return 1.5 * u * u - u;
}

fn trace_ray(h: Hole, d: vec3<f32>) -> Trace {
    var res: Trace;
    res.emission = vec3<f32>(0.0);
    res.trans = 1.0;
    res.dir = d;

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

    let r_in = h.axis_in.w;
    let r_out = h.xref_out.w;
    let has_disk = r_in > 0.0 && r_out > r_in;
    let nrm = h.axis_in.xyz;
    let xd = h.xref_out.xyz;
    let yd = cross(nrm, xd);
    let bright = h.params.y;
    let t_peak = h.params.z;
    let contrast = h.params.w;
    let anim = lens.view.z;

    // Plane crossings happen at phi_c + k pi.
    var phi_c = 1.0e9;
    let ca = dot(e1, nrm);
    let cb = dot(e2, nrm);
    if has_disk && (abs(ca) > 1.0e-6 || abs(cb) > 1.0e-6) {
        phi_c = atan2(-ca, cb);
        if phi_c < 1.0e-4 {
            phi_c = phi_c + PI;
        }
    }
    let ez = cross(e1, e2);
    let lz_dir = dot(ez, nrm);

    var u_stop = 0.02;
    if has_disk {
        u_stop = min(u_stop, 0.5 / r_out);
    }

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
        var crossing = false;
        if phi + h_step >= phi_c {
            h_step = phi_c - phi;
            crossing = true;
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

        if crossing {
            phi_c = phi_c + PI;
            if u > 1.0e-5 {
                let r = 1.0 / u;
                if r > r_in && r < r_out {
                    let p = cos(phi) * e1 + sin(phi) * e2;
                    let psi = atan2(dot(p, yd), dot(p, xd));
                    // Impact parameter from the conserved energy.
                    let inv_b2 = w * w + u * u * (1.0 - u);
                    let b = inverseSqrt(max(inv_b2, 1.0e-12));
                    // Photon angular momentum about the disk axis (real photon travels
                    // against the traced direction), per unit energy.
                    let lz = -b * lz_dir;
                    let omega = 0.70710678 * u * sqrt(u);
                    let denom = max(1.0 - omega * lz, 0.08);
                    let g = sqrt(max(1.0 - 1.5 * u, 0.02)) / denom;
                    let ds = disk_sample(r, psi, g, r_in, r_out, t_peak, bright, contrast, anim);
                    res.emission = res.emission + res.trans * ds.alpha * ds.rgb;
                    res.trans = res.trans * (1.0 - ds.alpha);
                }
            }
        }
    }
    if !captured && !escaped {
        // Ran out of steps while circling the photon sphere: treat as captured.
        captured = true;
    }
    if captured {
        res.trans = 0.0;
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

// Colour seen along view-space direction `d` (unit) through hole `h`.
fn shade(h: Hole, d: vec3<f32>, dims: vec2<f32>) -> vec3<f32> {
    let tr = trace_ray(h, d);
    var bg = vec3<f32>(0.0);
    if tr.trans > 0.0 {
        // Sources outside the rendered view (or behind the camera) have no pixels.
        // Reflect small overshoots back onto the screen (keeps star-field statistics)
        // and fade to black further out, which reads as the hole's deep shadow.
        let o = tr.dir;
        let zf = max(abs(o.z), 0.15);
        let ndc = vec2<f32>(o.x / zf / lens.view.x, o.y / zf / lens.view.y);
        let over = max(abs(ndc.x), abs(ndc.y));
        var vis = 1.0 - smoothstep(1.0, 2.4, over);
        if o.z > -0.05 {
            vis = 0.0;
        }
        if vis > 0.0 {
            let uv = vec2<f32>(mirror01(ndc.x * 0.5 + 0.5), mirror01(0.5 - ndc.y * 0.5));
            bg = vis * sample_scene(clamp(uv * dims, vec2<f32>(0.5), dims - vec2<f32>(0.5)));
        }
    }
    return bg * tr.trans + tr.emission;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(scene));
    let px = in.position.xy;
    let count = i32(lens.view.w);

    // Pick the hole whose influence region contains this pixel most deeply.
    let ndc0 = vec2<f32>(px.x / dims.x * 2.0 - 1.0, 1.0 - px.y / dims.y * 2.0);
    let d0 = normalize(vec3<f32>(ndc0.x * lens.view.x, ndc0.y * lens.view.y, -1.0));
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
    if best < 0 {
        return original;
    }
    let h = lens.holes[best];

    // Impact parameter of the centre ray decides how many sub-samples we spend.
    let cosa = dot(d0, h.dir_d.xyz);
    let sina = length(d0 - cosa * h.dir_d.xyz);
    let b0 = h.dir_d.w * sina;
    var colour = vec3<f32>(0.0);
    if b0 < 14.0 || h.dir_d.w < 20.0 {
        // 4 rotated-grid samples for crisp, anti-aliased shadow and disk edges.
        var offs = array<vec2<f32>, 4>(
            vec2<f32>(-0.125, -0.375), vec2<f32>(0.375, -0.125),
            vec2<f32>(0.125, 0.375), vec2<f32>(-0.375, 0.125));
        for (var s = 0; s < 4; s = s + 1) {
            let p = px + offs[s];
            let ndc = vec2<f32>(p.x / dims.x * 2.0 - 1.0, 1.0 - p.y / dims.y * 2.0);
            let d = normalize(vec3<f32>(ndc.x * lens.view.x, ndc.y * lens.view.y, -1.0));
            colour = colour + shade(h, d, dims);
        }
        colour = colour * 0.25;
    } else {
        colour = shade(h, d0, dims);
    }
    // Blend to the untouched scene at the border of the influence region so the
    // pass has no visible seam.
    let fade = smoothstep(0.7, 1.0, best_score);
    colour = mix(colour, original.rgb, fade);
    return vec4<f32>(colour, original.a);
}
