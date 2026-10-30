// Accretion-disk emission model, used by blackhole_lens.wesl.
//
// All radii are in Schwarzschild radii (r_s = 1). The disk is an infinitely thin
// annulus in the hole's equatorial plane; the lens shader finds where each
// (curved) light ray crosses that plane and asks this module for the radiance
// of the gas at that point.

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;

// ---------------------------------------------------------------------------
// Noise

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(0.1031, 0.1030));
    q = q + dot(q, q.yx + 33.33);
    return fract((q.x + q.y) * q.x);
}

// Periodic value noise: x wraps with period `period` (an integer number of cells).
fn vnoise(p: vec2<f32>, period: f32) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let x0 = i.x - period * floor(i.x / period);
    let x1 = (i.x + 1.0) - period * floor((i.x + 1.0) / period);
    let a = hash21(vec2<f32>(x0, i.y));
    let b = hash21(vec2<f32>(x1, i.y));
    let c = hash21(vec2<f32>(x0, i.y + 1.0));
    let d = hash21(vec2<f32>(x1, i.y + 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// Turbulence in (azimuth, ln r): three octaves, streaky along the orbit.
fn disk_turbulence(psi: f32, ln_r: f32) -> f32 {
    let a = psi / TAU;
    var n = 0.0;
    var amp = 0.5;
    var nphi = 28.0;
    var nr = 9.0;
    var norm = 0.0;
    for (var o = 0; o < 3; o = o + 1) {
        let p = vec2<f32>(a * nphi, ln_r * nr + f32(o) * 17.3);
        n = n + amp * vnoise(p, nphi);
        norm = norm + amp;
        amp = amp * 0.5;
        nphi = nphi * 2.0;
        nr = nr * 1.9;
    }
    return n / norm;
}

// ---------------------------------------------------------------------------
// Colour

// Linear sRGB colour of a black body, normalised so the largest channel is 1
// (Tanner Helland's fit, valid roughly 1000 K - 40000 K).
fn blackbody(temp: f32) -> vec3<f32> {
    let t = clamp(temp, 1000.0, 40000.0) / 100.0;
    var c: vec3<f32>;
    if t <= 66.0 {
        c.x = 1.0;
        c.y = 0.390081579 * log(t) - 0.631841444;
    } else {
        c.x = 1.292936186 * pow(t - 60.0, -0.1332047592);
        c.y = 1.129890861 * pow(t - 60.0, -0.0755148492);
    }
    if t >= 66.0 {
        c.z = 1.0;
    } else if t <= 19.0 {
        c.z = 0.0;
    } else {
        c.z = 0.543206789 * log(t - 10.0) - 1.196254089;
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    // sRGB -> linear
    return pow(c, vec3<f32>(2.2));
}

// ---------------------------------------------------------------------------
// Disk

// Shakura-Sunyaev / Novikov-Thorne-like temperature run, normalised to 1 at its
// peak (r = 1.36 r_in).
fn disk_temperature_profile(r: f32, r_in: f32) -> f32 {
    let x = r_in / r;
    return pow(x, 0.75) * pow(max(1.0 - sqrt(x), 0.0), 0.25) / 0.4878;
}

struct DiskSample {
    // Radiance (linear HDR) already weighted by the cell's opacity.
    rgb: vec3<f32>,
    // Opacity of the gas at this crossing (0..1).
    alpha: f32,
};

// Emission of the disk gas at radius `r` and azimuth `psi`, seen with total
// frequency shift `g` (Doppler x gravitational; g > 1 is blue-shifted).
//   t_peak:   colour temperature (K) at the hottest ring
//   bright:   overall brightness multiplier
//   contrast: turbulence contrast (catalog DiskNoiseContrast)
//   anim:     animation phase (seconds)
fn disk_sample(r: f32, psi: f32, g: f32, r_in: f32, r_out: f32, t_peak: f32, bright: f32, contrast: f32, anim: f32) -> DiskSample {
    var s: DiskSample;
    s.rgb = vec3<f32>(0.0);
    s.alpha = 0.0;
    if r <= r_in || r >= r_out {
        return s;
    }
    let prof = disk_temperature_profile(r, r_in);
    let t_emit = t_peak * prof;

    // Keplerian shear: inner gas laps the outer gas, which stretches the noise
    // into spiral streaks. Slowed down enormously for visibility.
    let omega = 0.7071 * pow(r, -1.5);
    let psi_t = psi - omega * anim * 6.0;
    let ln_r = log(r);
    let n = disk_turbulence(psi_t, ln_r);
    let clump = 1.0 + contrast * 1.6 * (n - 0.5) * 2.0;
    let rings = 1.0;

    // Bolometric surface brightness ~ T^4 would be far too contrasty for display;
    // use a compressed power and the beaming law I_obs = g^3 I_emit.
    let g3 = g * g * g;
    let intensity = 7.0 * bright * prof * prof * max(clump, 0.05) * rings * g3;

    let t_obs = g * t_emit;
    s.rgb = blackbody(t_obs) * intensity;

    // Opacity: fades at both edges.
    let fin = smoothstep(0.0, 0.12, r / r_in - 1.0);
    let fout = 1.0 - smoothstep(0.7, 1.0, (r - r_in) / (r_out - r_in));
    let thin = mix(1.0, 0.4, smoothstep(r_in * 2.0, r_out, r));
    s.alpha = 0.92 * thin * fin * fout * clamp(0.55 + 0.9 * n, 0.0, 1.0);
    return s;
}
