// SpaceEngine's volumetric accretion disk and jets (gr_intersection.glh: DiskProperties,
// JetProperties, ShellFbm, VolumeStep, IntegrationStep) and the light-bent surface of a neutron
// star or white dwarf (StarSurface), used by blackhole_lens.wesl.
//
// SE works in units of its render sphere (DistortionRadius); here every length is in units of
// SE's gravRadius (the Schwarzschild radius Rg, r_s = 1; the star's radius for a white dwarf).
// SE's sphere-unit expressions reduce to these: rho * invGravRadius -> rho,
// 1300 * r * gravRadius * distRadCorr -> 1.3 r, 1e-3 * rayDistance * invGravRadius -> 1e-3 * path
// length. Densities keep SE's per-sphere-unit scale; the step length is multiplied by gravS
// (= gravRadius / sphere radius) where SE integrates.

const PI: f32 = 3.14159265359;

struct Hole {
    // xyz: unit view-space direction camera -> hole, w: camera distance (units).
    dir_d: vec4<f32>,
    // xyz: disk normal (spin axis, SE's +y), w: inner radius (units); <= 0: no disk.
    axis_in: vec4<f32>,
    // xyz: disk x axis, w: outer radius (units).
    xref_out: vec4<f32>,
    // x: influence radius (rad), y: gravS (gravRadius / render sphere radius), z: KerrSpin,
    // w: volumBright (ImpParams.z).
    params: vec4<f32>,
    // MetricParams: KerrSpin, KerrCharge and their squares.
    metric: vec4<f32>,
    // StarParams: surface temperature (K), surface radius (units; 0: a black hole), equator
    // v/c, starTime (0..256).
    star: vec4<f32>,
    // x: KerrM (DiskParams5.w: 1, Rg / R for a white dwarf), y: invGravRadius (render sphere
    // radius in units), z: warpStep, w: starRotAng (rad).
    gr: vec4<f32>,
    // x: diskTemp (K), y: TwistMagn, z: 1 disk volume + 2 jets, w: disk animation time (0..1).
    disk1: vec4<f32>,
    // x: tempShift, y: diskBright, z: diskDensity, w: volume step length (r_s).
    disk2: vec4<f32>,
    // x, y: inner / outer thickness (r_s), z: DetailScaleR, w: DetailScaleV.
    disk3: vec4<f32>,
    // xyz: OctaveDistortion, w: OctaveScale.
    disk4: vec4<f32>,
    // x: DiskNoiseContrast, y: DiskOuterSpin, z: DiskTempContrast, w: bounding radius (units).
    disk5: vec4<f32>,
    // x, y: jet start / end temperature, z: ShadowContrast, w: ShadowLength.
    jet1: vec4<f32>,
    // x, y: jet start / end radius (units), z: jet length (units), w: JetVelocity (c).
    jet2: vec4<f32>,
    // x: jet brightness, y: jet density, z: JetDistortion, w: JetThickness.
    jet3: vec4<f32>,
    // rgb: LightColor, w: frame / environment blend factor.
    light: vec4<f32>,
};

// SE's Planck table (FUN_1401a60d0): 4096 RGB radiances, log-spaced in temperature.
@group(0) @binding(3) var planck_tex: texture_2d<f32>;
// SE's NoiseIqMap: 16 x 16 random texture with G, A = R, B shifted by (11, 5).
@group(0) @binding(4) var noise_tex: texture_2d<f32>;
@group(0) @binding(5) var noise_sampler: sampler;

// Accretion disk optical density multipliers for R, G, B.
const DISK_OPTICAL_DENSITY: vec3<f32> = vec3<f32>(0.7, 0.85, 1.0);
const VOLUMETRIC_STOP: f32 = 12.0;
const MAX_VOLUME_STEPS: i32 = 256;
const SPEED_OF_LIGHT: f32 = 16000.0;

fn linearstep(e0: f32, e1: f32, x: f32) -> f32 {
    return clamp((x - e0) / (e1 - e0), 0.0, 1.0);
}

// GLSL smoothstep by its formula, so that SE's reversed edges (e0 > e1) are well defined.
fn sstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = linearstep(e0, e1, x);
    return t * t * (3.0 - 2.0 * t);
}

fn transmission(sigma: f32) -> vec3<f32> {
    return sstep(VOLUMETRIC_STOP, 0.0, sigma) * exp(-sigma * DISK_OPTICAL_DENSITY);
}

// texture(PlanckFunction, log(temp * 0.001 + tempShift) * 0.188 + 0.1316), GL_LINEAR, clamped.
fn thermal_emission(temp: f32, shift: f32) -> vec3<f32> {
    let x = log(max(temp, 1.0) * 0.001 + shift) * 0.188 + 0.1316;
    let p = clamp(x * 4096.0 - 0.5, 0.0, 4095.0);
    let i = i32(floor(p));
    let a = textureLoad(planck_tex, vec2<i32>(i, 0), 0).rgb;
    let b = textureLoad(planck_tex, vec2<i32>(min(i + 1, 4095), 0), 0).rgb;
    return mix(a, b, p - floor(p));
}

fn wrap16(v: vec2<f32>) -> vec2<f32> {
    return v - 16.0 * floor(v / 16.0);
}

// ---------------------------------------------------------------------------
// Noise (gr_intersection.glh)

// Angular shift of the shell at radius r at time t (looped 0..1; integer multiples of the
// texture period, so the loop is seamless).
fn radial_angle(t: f32, r: f32) -> f32 {
    let velocity = ceil(524288.0 / (r * r * r + 1.0));
    return 16.0 * velocity * t;
}

fn sample_noise(p_in: vec2<f32>, shell: f32, scale: f32, t: f32, outer_spin: f32) -> vec2<f32> {
    var p = p_in;
    p.x = p.x + scale * radial_angle(t, shell / scale);
    p.x = p.x * round(1.0 + shell / scale * outer_spin);
    let ip = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    let uv = wrap16(ip + vec2<f32>(11.0, 5.0) * shell) + f;
    return textureSampleLevel(noise_tex, noise_sampler, uv * 0.0625 + 0.03125, 0.0).xz;
}

fn shell_noise(p_in: vec3<f32>, t: f32, scale: f32, outer_spin: f32) -> vec2<f32> {
    let p = p_in * scale;
    let shell = floor(p.x);
    let r0 = sample_noise(p.yz, shell, scale, t, outer_spin);
    let r1 = sample_noise(p.yz, shell + 1.0, scale, t, outer_spin);
    return mix(r0, r1, fract(p.x));
}

// `ray_dist`: path length from the camera (r_s), fades out the octaves finer than a pixel.
fn shell_fbm(h: Hole, p_in: vec3<f32>, t: f32, ray_dist: f32) -> f32 {
    var p = p_in;
    var sum = vec3<f32>(0.0);
    let w = 1.0e-3 * ray_dist;
    var amplitude = 1.0;
    var scale = 1.0;
    for (var i = 0; i < 3; i = i + 1) {
        var k = linearstep(1.0, 0.5, w * scale);
        if i == 0 {
            k = linearstep(1.0, 0.5, 0.05 * w * scale);
        }
        sum = sum + amplitude * vec3<f32>(shell_noise(p, t, scale, h.disk5.y) * k, 1.0);
        p = p + sum.xyx * h.disk4.xyz / scale;
        scale = scale * 3.0;
        amplitude = amplitude * h.disk4.w;
    }
    let q = sum.x / sum.z;
    return 3.0 * mix(1.0, q * q * q * q, h.disk5.x);
}

// Fast LUT based 3D noise.
fn noise3(x: vec3<f32>) -> vec2<f32> {
    let ip = floor(x);
    var p = fract(x);
    p = p * p * (3.0 - 2.0 * p);
    let uv = wrap16(ip.xy + vec2<f32>(11.0, 5.0) * ip.z) + p.xy;
    let res = textureSampleLevel(noise_tex, noise_sampler, uv * 0.0625 + 0.03125, 0.0);
    return mix(res.yw, res.xz, vec2<f32>(p.z));
}

fn fbm(x_in: vec3<f32>) -> f32 {
    var x = x_in;
    var sum = vec2<f32>(0.0);
    var ampl = 1.0;
    for (var i = 0; i < 3; i = i + 1) {
        sum = sum + ampl * noise3(x);
        x = x + 1.4 * sum.xyx * vec3<f32>(0.7, 0.6, 1.3);
        ampl = ampl * 0.74;
        x = x * 4.0;
    }
    return sum.x;
}

// ---------------------------------------------------------------------------
// Bounding volume

fn sd_capped_cylinder(p: vec3<f32>, hr: f32, hh: f32) -> f32 {
    let d = abs(vec2<f32>(length(p.xz), p.y)) - vec2<f32>(hr, hh);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0)));
}

fn dot2(v: vec2<f32>) -> f32 {
    return dot(v, v);
}

fn sd_capped_cone(p: vec3<f32>, hh: f32, r1: f32, r2: f32) -> f32 {
    let q = vec2<f32>(length(p.xz), p.y);
    let k1 = vec2<f32>(r2, hh);
    let k2 = vec2<f32>(r2 - r1, 2.0 * hh);
    let ca = vec2<f32>(q.x - min(q.x, select(r2, r1, q.y < 0.0)), abs(q.y) - hh);
    let cb = q - k1 + k2 * clamp(dot(k1 - q, k2) / dot2(k2), 0.0, 1.0);
    let s = select(1.0, -1.0, cb.x < 0.0 && ca.y < 0.0);
    return s * sqrt(min(dot2(ca), dot2(cb)));
}

// sdBoundingVolume: the disk's cylinder and the two jet cones (disk frame, y = normal).
fn sd_bounding(h: Hole, p_in: vec3<f32>) -> f32 {
    var p = p_in;
    p.y = abs(p.y);
    var sdf = 1.0e10;
    if h.disk2.z > 0.0 {
        sdf = sd_capped_cylinder(p, h.xref_out.w * 0.95, max(h.disk3.x, h.disk3.y));
    }
    if h.jet3.y > 0.0 {
        let len = h.jet2.z;
        let thickness = 2.0 * h.jet3.w * h.jet2.x;
        let jsdf = abs(sd_capped_cone(p - vec3<f32>(0.0, 0.5 * len, 0.0), 0.5 * len, h.jet2.x + thickness, 1.25 * h.jet2.y + thickness)) - 2.0 * thickness;
        sdf = min(sdf, jsdf);
    }
    return sdf;
}

// ---------------------------------------------------------------------------
// Disk and jet gas

struct VolProps {
    density: f32,
    temp: f32,
    emission: f32,
    gradient: f32,
    // Disk frame.
    velocity: vec3<f32>,
};

fn no_props() -> VolProps {
    return VolProps(0.0, 0.0, 0.0, 0.0, vec3<f32>(0.0));
}

fn average_disk_density(h: Hole, pos: vec3<f32>, rho: f32) -> f32 {
    let thickness = mix(h.disk3.x, h.disk3.y, clamp((rho - 1.0) / h.xref_out.w, 0.0, 1.0));
    let hgt = pos.y / thickness;
    let outer = sstep(h.xref_out.w * 0.95, 0.0, rho);
    return sstep(1.0, 0.0, abs(hgt)) * linearstep(h.axis_in.w, h.axis_in.w * 1.2, rho) * outer;
}

// DiskProperties: an alpha-disk (alpha = 1) with Keplerian shear and shell noise.
fn disk_properties(h: Hole, pos: vec3<f32>, time: f32, ray_dist: f32) -> VolProps {
    var prop = no_props();
    if h.disk2.z <= 0.0 {
        return prop;
    }
    let rho = length(pos);
    let r = (1.25 + 0.8 * h.params.z) * rho;
    prop.density = average_disk_density(h, pos, rho);
    if prop.density < 1.0e-4 {
        return prop;
    }
    // v/c = sqrt(Rg / 2(R - Rg)).
    let beta = inverseSqrt(max(2.0 * (r - 1.0), 1.0e-3));
    let rr = 0.75 * sqrt(r) * h.disk3.z;
    let teta0 = atan2(pos.x, pos.z);
    let hh = pos.y * h.disk3.w;
    let teta = teta0 * (8.0 / PI) + beta * (1.0 + 0.02 * h.disk1.y - 1.3 * r);
    let turbulence = shell_fbm(h, vec3<f32>(rr, teta, hh), time, ray_dist);

    let f = 1.0 - 1.73205 * inverseSqrt(r);
    let rpow = pow(r, -0.65);
    var temp = 5.1293 * pow(max(f, 0.0), 0.3) * rpow;
    let dens = rpow * linearstep(1.0, 3.0, r);
    if r < 4.32 {
        temp = linearstep(1.0, 4.32, r);
    }
    // Gradient shadows: the noise a shadow length further out.
    if r > 12.0 {
        let sl = h.jet1.w;
        let rg = r + sl;
        let rrg = 0.75 * sqrt(rg) * h.disk3.z;
        let tetag = teta0 * (8.0 / PI) + beta * (1.0 + 0.02 * h.disk1.y - 1.3 * rg);
        prop.gradient = smoothstep(12.0, 30.0, r) * (shell_fbm(h, vec3<f32>(rrg, tetag, hh), time, ray_dist) - turbulence) / (turbulence * sl + 1.0e-4);
    }
    prop.density = abs(5.0e-4 * dens * prop.density * turbulence * h.disk2.z);
    prop.temp = mix(1.0, turbulence, h.disk5.z) * temp * h.disk1.x;
    prop.velocity = normalize(vec3<f32>(-pos.z, 0.0, pos.x)) * beta;
    prop.emission = h.disk2.y;
    return prop;
}

fn jet_properties(h: Hole, pos: vec3<f32>, time: f32) -> VolProps {
    var prop = no_props();
    let jet_density = h.jet3.y;
    let jet_length = h.jet2.z;
    if jet_density <= 0.0 {
        return prop;
    }
    var pj = -sign(pos.y) * pos;
    let jet_rad = mix(h.jet2.x, h.jet2.y, abs(pos.y) / jet_length);
    if dot(pj.xz, pj.xz) > (1.5 * jet_rad) * (1.5 * jet_rad) {
        return prop;
    }
    let r = max(length(pj), 1.0e-4);
    // Rotation angle 0.15 pi / speed (unbounded where log(0.85 r) = 0; kept finite).
    let speed = log(r * 0.85) * 0.075;
    let a = 0.15 * PI / select(min(speed, -1.0e-4), max(speed, 1.0e-4), speed >= 0.0) + time * 2480.0;
    let ca = cos(a);
    let sa = sin(a);
    pj = vec3<f32>(pj.x * ca + pj.z * sa, pj.y, -pj.x * sa + pj.z * ca);
    let inv_y = 1.0 / (abs(pj.y) + 0.1);
    let noise_pos = 0.1 * pj * vec3<f32>(10.0 * inv_y, 2.0, 10.0 * inv_y) + vec3<f32>(0.0, time * 1024.0, 0.0);
    let distort = fbm(noise_pos) * clamp(exp(pj.y * 0.025), 0.0, 0.5);
    let sd_jet = sd_capped_cylinder(pj, jet_rad, jet_length) - h.jet3.z * jet_rad * linearstep(1.7, 5.0, abs(pj.y)) * abs(distort);
    var jets = sstep(h.jet3.w * jet_rad, 0.0, abs(sd_jet)) * inv_y;
    jets = jets * linearstep(jet_length, 0.0, abs(pos.y));
    prop.density = linearstep(0.5, 1.0, abs(pj.y)) * linearstep(1.2, 1.5, r) * pow(r, -0.75) * jets * jet_density;
    prop.temp = mix(h.jet1.x, h.jet1.y, 2.0 * abs(pos.y) / jet_length);
    prop.velocity = vec3<f32>(0.0, sign(pos.y), 0.0) * h.jet2.w;
    prop.emission = h.jet3.x;
    return prop;
}

// Accumulated volume along a ray (SE's RayProperties).
struct VolRay {
    color: vec3<f32>,
    transm: vec3<f32>,
    sigma: f32,
    steps: i32,
    volume_steps: i32,
};

// DopplerGravShiftedEmission: `rho` distance from the hole, `eye` camera distance (units).
fn shifted_emission(h: Hole, rho: f32, eye: f32, ray_dir: vec3<f32>, velocity: vec3<f32>, temp_in: f32) -> vec3<f32> {
    var one_plus_z = (rho * (eye - 1.0)) / (eye * max(rho - 1.0, 1.0e-3));
    one_plus_z = one_plus_z * (1.0 + dot(ray_dir, velocity)) * inverseSqrt(max(1.0 - dot(velocity, velocity), 1.0e-3));
    return thermal_emission(temp_in / max(one_plus_z, 1.0e-3), h.disk2.x);
}

// VolumeStep + IntegrationStep at disk-frame position `pos`, ray direction `ray_dir`, step
// `dt` (units), at path length `ray_dist` and emission time `time` (0..1 loop).
fn volume_step(h: Hole, pos: vec3<f32>, ray_dir: vec3<f32>, dt: f32, ray_dist: f32, time: f32, vr: ptr<function, VolRay>) {
    if (*vr).volume_steps > MAX_VOLUME_STEPS {
        return;
    }
    var prop = disk_properties(h, pos, time, ray_dist);
    if (h.disk1.z >= 2.0) {
        let jp = jet_properties(h, pos, time);
        if jp.density > prop.density {
            prop = jp;
        }
    }
    if prop.density < 1.0e-3 {
        return;
    }
    if prop.density > 0.4 {
        (*vr).volume_steps = (*vr).volume_steps + 1;
    }
    let r = length(pos);
    let illum = prop.density * (1.0 + clamp(h.jet1.z * prop.gradient, -1.0, 1.0)) * h.light.rgb / (100.0 + r * r);
    let temp = pow(2.0e14 * length(illum) + pow(prop.temp, 4.0), 0.25);
    let emiss = shifted_emission(h, r, h.dir_d.w, ray_dir, prop.velocity, temp) * prop.emission + illum;
    // Densities are per render-sphere unit, steps in r_s.
    let dts = dt * h.params.y;
    let ext = exp(-dts * prop.density * DISK_OPTICAL_DENSITY);
    (*vr).color = (*vr).color + emiss * (1.0 - ext) * (*vr).transm;
    (*vr).transm = (*vr).transm * ext;
    (*vr).sigma = (*vr).sigma + dts * prop.density;
}

// StarSurface (STAR define: neutron stars and white dwarfs): the surface at unit normal `n`
// (disk frame) seen along `ray_dir`. Gravitational shift with SE's starR = starRadius /
// starRadiusScale, Doppler shift of the equator speed, Fbm temperature spots drifting with
// starTime, poles 1.25x hotter, the pattern turned by starRotAng; the caller applies
// volumBright (SE: rayProp.color *= volumBright).
fn star_surface(h: Hole, n: vec3<f32>, ray_dir: vec3<f32>) -> vec3<f32> {
    let eye = h.dir_d.w;
    let star_r = h.star.y / h.gr.x;
    var one_plus_z = sqrt(max((star_r * (eye - 1.0)) / (eye * (star_r - 1.0)), 1.0e-6));
    let eq = vec3<f32>(n.z, 0.0, n.x);
    var cost = 0.0;
    if dot(eq, eq) > 1.0e-12 {
        cost = dot(ray_dir, normalize(eq));
    }
    let v = h.star.z;
    one_plus_z = one_plus_z * (1.0 - v * cost) * inverseSqrt(max(1.0 - v * v, 1.0e-6));
    let ca = cos(h.gr.w);
    let sa = sin(h.gr.w);
    var p = vec3<f32>(n.x * ca + n.z * sa, n.y, -n.x * sa + n.z * ca);
    let lat = abs(p.y);
    p.y = lat * lat - h.star.w + step(0.0, n.y);
    var temp = h.star.x * (fbm(p * 4.0) * 0.3 + 0.7);
    temp = mix(h.star.x * 1.25, temp, clamp(lat * 4.0 - 0.5, 0.0, 1.0));
    temp = temp * pow(max(one_plus_z, 1.0e-3), -1.25) * 0.75;
    return thermal_emission(temp, h.disk2.x);
}
