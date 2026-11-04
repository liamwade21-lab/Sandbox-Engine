// The active body's live weather (volumetric_clouds/weather_sim.rs).
//
// A cube of cells over the body (SE's tile faces, y the pole, as the cover map), each holding
//   r: thunderstorm activity c (0..1),
//   g: the cooled, stable air storms leave behind s (0..1), which keeps new ones away,
//   b: anvils and leftover cloud a (0..1),
// stepped by the derived weather's convection and carried by its winds (sim_seed, sim_step), and
// the output pass writing the weather cube the clouds read (volumetric_clouds.wesl `weather_at`):
// the derived weather, two maps blended by the date, shaped by the cells, and the fog and dust
// cube (sim_out); then the lightning flashes of the moment where the storms are (sim_flash).

struct SimUniform {
    // x: the derived maps' blend, y: the body's radius (m), z: humidity (0..1), w: the step (s).
    maps: vec4<f32>,
    // x: the step's index (mod 2^20), y: the body's seed fraction, z: the trigger noise's frequency
    // (per radius), w: the seed's weight (1: at once).
    step: vec4<f32>,
    // x, y: the seed pattern's slice (mod 4096) and how far into it, z: the output's interpolation
    // from the previous state to the current one, w: 1 to use the cells (0: the derived weather alone).
    seed: vec4<f32>,
    // x: the effects' clock (s), y, z: the heights flashes are between (m above the radius), w: 1 on,
    // 2 on over a gas giant (`giant_storm`).
    flash: vec4<f32>,
    // xyz: towards the camera (sphere space, unit), w: the angle around it near flashes are looked
    // for in (rad).
    cam: vec4<f32>,
};

@group(0) @binding(0) var<uniform> su: SimUniform;
@group(0) @binding(1) var state_tex: texture_cube<f32>;
@group(0) @binding(2) var prev_tex: texture_cube<f32>;
// Two dates' derived maps (se_core weather.rs `weather_map`), each its weather and its wind layer:
// equirectangular, rows from the north.
@group(0) @binding(3) var derived_tex: texture_2d_array<f32>;
@group(0) @binding(4) var smp: sampler;
@group(0) @binding(5) var state_out: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(6) var weather_out: texture_storage_2d_array<rgba8unorm, write>;
// R fog, G dust, B the storms' activity (as `sim_out` shapes it).
@group(0) @binding(7) var fx_out: texture_storage_2d_array<rgba8unorm, write>;
// The weather cube (as sim_out wrote it) and the flashes: xyz where (sphere space, m from the
// centre), w how bright now (0: none).
@group(0) @binding(8) var weather_in: texture_cube<f32>;
@group(0) @binding(9) var<storage, read_write> flashes: array<vec4<f32>, 8>;
// The fog, dust and storms cube (as sim_out wrote it).
@group(0) @binding(10) var fx_in: texture_cube<f32>;

const PI: f32 = 3.14159265358979;
// se_core weather.rs `WIND_RANGE` (m/s).
const WIND_RANGE: f32 = 100.0;
// Steps per trigger slice (2 hours of 5-minute steps).
const TRIGGER_STEPS: f32 = 24.0;

// SE's planet tile faces (as volumetric_clouds.wesl `face_dir`): u right, v down.
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

fn cell_dir(id: vec3<u32>, n: u32) -> vec3<f32> {
    let uv = (vec2<f32>(id.xy) + 0.5) / f32(n);
    return normalize(face_dir(i32(id.z), uv.x, uv.y));
}

struct Derived {
    // x the cover multiplier, y the cover offset, z convection, w precipitation.
    w: vec4<f32>,
    // Towards the east and the north (m/s).
    wind: vec2<f32>,
    // Fog, dust.
    fx: vec2<f32>,
};

fn derived_at(d: vec3<f32>) -> Derived {
    let lon = atan2(-d.z, d.x);
    let lat = asin(clamp(d.y, -1.0, 1.0));
    let rows = f32(textureDimensions(derived_tex).y);
    let uv = vec2<f32>((lon + PI) / (2.0 * PI), clamp((0.5 * PI - lat) / PI, 0.5 / rows, 1.0 - 0.5 / rows));
    let f = su.maps.x;
    let w = mix(textureSampleLevel(derived_tex, smp, uv, 0, 0.0), textureSampleLevel(derived_tex, smp, uv, 2, 0.0), f);
    let v = mix(textureSampleLevel(derived_tex, smp, uv, 1, 0.0), textureSampleLevel(derived_tex, smp, uv, 3, 0.0), f);
    var out: Derived;
    out.w = vec4<f32>(w.r * 2.0, w.g * 2.0 - 1.0, w.b, w.a);
    out.wind = (v.rg - 0.5) * 2.0 * WIND_RANGE;
    out.fx = v.ba;
    return out;
}

fn hash(p: vec3<i32>, s: u32) -> f32 {
    let h = bitcast<vec3<u32>>(p) * vec3<u32>(1597334673u, 3812015801u, 2798796415u);
    var x = (h.x ^ h.y ^ h.z ^ s) * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x) / 4294967295.0;
}

// Smooth value noise (0..1).
fn vnoise(p: vec3<f32>, s: u32) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(hash(i, s), hash(i + vec3<i32>(1, 0, 0), s), u.x);
    let x10 = mix(hash(i + vec3<i32>(0, 1, 0), s), hash(i + vec3<i32>(1, 1, 0), s), u.x);
    let x01 = mix(hash(i + vec3<i32>(0, 0, 1), s), hash(i + vec3<i32>(1, 0, 1), s), u.x);
    let x11 = mix(hash(i + vec3<i32>(0, 1, 1), s), hash(i + vec3<i32>(1, 1, 1), s), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

// Noise at direction d that changes from slice to slice (index k, fraction f), blended across.
fn sliced(d: vec3<f32>, freq: f32, k: f32, f: f32, s: u32) -> f32 {
    let k0 = k % 1024.0;
    let k1 = (k + 1.0) % 1024.0;
    let a = vnoise(d * freq + vec3<f32>(17.3, 5.1, 11.7) * k0, s);
    let b = vnoise(d * freq + vec3<f32>(17.3, 5.1, 11.7) * k1, s);
    return mix(a, b, smoothstep(0.0, 1.0, f));
}

fn body_seed() -> u32 {
    return u32(su.step.y * 10000.0);
}

// The state a cell starts from at the seed's date: storms where the convection allows them, part
// way through their lives.
fn seed_state(d: vec3<f32>, p: f32) -> vec4<f32> {
    let blob = sliced(d, su.step.z * 0.8, su.seed.x, su.seed.y, body_seed() ^ 0x5eedu);
    let c = p * smoothstep(0.55, 0.85, blob);
    return vec4<f32>(c, 0.6 * c, 0.8 * c, 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn sim_seed(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = textureDimensions(state_out).x;
    if id.x >= n || id.y >= n {
        return;
    }
    let d = cell_dir(id, n);
    let dw = derived_at(d);
    let want = seed_state(d, clamp(dw.w.z, 0.0, 1.0));
    var v = want;
    if su.step.w < 1.0 {
        v = mix(textureSampleLevel(state_tex, smp, d, 0.0), want, su.step.w);
    }
    textureStore(state_out, vec2<i32>(id.xy), i32(id.z), v);
}

// One step. Rates per hour: storms start where the trigger noise is high and the convection
// allows, grow towards twice the convection the stable air leaves, and die where that air has built
// up and at night; the stable air follows the storms over 3 hours; anvils grow from the storms and
// thin out over 6 hours. Everything carried by the derived winds (from upwind, semi-Lagrangian).
@compute @workgroup_size(8, 8, 1)
fn sim_step(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = textureDimensions(state_out).x;
    if id.x >= n || id.y >= n {
        return;
    }
    let d = cell_dir(id, n);
    let dw = derived_at(d);
    let east = select(normalize(vec3<f32>(d.z, 0.0, -d.x)), vec3<f32>(0.0, 0.0, 1.0), abs(d.y) > 0.9999);
    let north = cross(d, east);
    let dt = su.maps.w;
    let upwind = normalize(d - (east * dw.wind.x + north * dw.wind.y) * (dt / su.maps.y));
    let l = textureSampleLevel(state_tex, smp, upwind, 0.0);
    var c = l.r;
    var s = l.g;
    var a = l.b;
    let h = dt / 3600.0;
    let p = clamp(dw.w.z, 0.0, 1.0);
    let pe = p * (1.0 - s);
    let k = floor(su.step.x / TRIGGER_STEPS);
    let trig = sliced(d, su.step.z, k, su.step.x / TRIGGER_STEPS - k, body_seed() ^ 0x7419u);
    let grow = c * (2.0 * pe - c);
    let start = 0.3 * pe * smoothstep(0.62, 0.9, trig);
    // Storm systems outlive the afternoon, fading over the night.
    let die = c * s + 0.12 * c * (1.0 - p);
    c = clamp(c + h * (grow + start - die), 0.0, 1.0);
    s = clamp(s + h * (c - s) / 3.0, 0.0, 1.0);
    a = clamp(a + h * (0.25 * c - a / 6.0), 0.0, 1.0);
    textureStore(state_out, vec2<i32>(id.xy), i32(id.z), vec4<f32>(c, s, a, 0.0));
}

// The weather cube: the derived weather with the storms' cloud, towers and rain, and clearer air
// where the stable air has settled.
@compute @workgroup_size(8, 8, 1)
fn sim_out(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = textureDimensions(weather_out).x;
    if id.x >= n || id.y >= n {
        return;
    }
    let d = cell_dir(id, n);
    let dw = derived_at(d);
    let l = mix(textureSampleLevel(prev_tex, smp, d, 0.0), textureSampleLevel(state_tex, smp, d, 0.0), su.seed.z) * su.seed.w;
    // Storms and anvils with soft edges (sharper cut-outs drew the cells as squares from orbit): an
    // active storm closes the deck and thickens the cloud already there.
    let c = smoothstep(0.04, 0.6, l.r);
    let s = l.g;
    let a = smoothstep(0.05, 0.7, l.b);
    let mul = dw.w.x * (1.0 + 0.6 * c);
    let add = dw.w.y + 0.55 * c + 0.25 * a - 0.15 * s * (1.0 - c);
    let conv = clamp(0.35 * dw.w.z + 0.9 * c, 0.0, 1.0);
    let precip = clamp(0.6 * dw.w.w + 0.8 * c * sqrt(su.maps.z) + 0.1 * a, 0.0, 1.0);
    let w = vec4<f32>(mul * 0.5, add * 0.5 + 0.5, conv, precip);
    textureStore(weather_out, vec2<i32>(id.xy), i32(id.z), clamp(w, vec4<f32>(0.0), vec4<f32>(1.0)));
    // Storms clear the fog; their outflows lift dust on dry worlds.
    let dry = 1.0 - su.maps.z;
    let fx = vec4<f32>(dw.fx.x * (1.0 - c), dw.fx.y + 0.5 * c * dry * dry, c, 0.0);
    textureStore(fx_out, vec2<i32>(id.xy), i32(id.z), clamp(fx, vec4<f32>(0.0), vec4<f32>(1.0)));
}

// A gas giant's storm activity at d for its lightning (it has no weather maps: SE's bands and storms
// are its own): scattered storm cells in its deep water clouds, a 400th of its radius across (Jupiter's
// about 180 km), changing over ten minutes of the effects' clock.
fn giant_storm(d: vec3<f32>) -> f32 {
    let t = su.flash.x / 600.0;
    let k = floor(t);
    return smoothstep(0.68, 0.9, sliced(d, 400.0, k, t - k, body_seed() ^ 0x6a11u));
}

// A random direction (h: three uniforms).
fn rand_dir(h: vec3<f32>) -> vec3<f32> {
    let z = 2.0 * h.x - 1.0;
    let a = 2.0 * PI * h.y;
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3<f32>(r * cos(a), z, r * sin(a));
}

// A random direction within `angle` of `c` (unit).
fn rand_near(c: vec3<f32>, angle: f32, h: vec3<f32>) -> vec3<f32> {
    let e1 = normalize(select(cross(c, vec3<f32>(0.0, 1.0, 0.0)), cross(c, vec3<f32>(1.0, 0.0, 0.0)), abs(c.y) > 0.9));
    let e2 = cross(c, e1);
    let r = angle * sqrt(h.x);
    let a = 2.0 * PI * h.y;
    return normalize(c + (e1 * cos(a) + e2 * sin(a)) * tan(r));
}

fn hash1(a: u32, b: u32) -> f32 {
    return hash(vec3<i32>(i32(a), i32(b), 7), body_seed());
}

// The flashes: each slot has its own rhythm (a flash every 1.5 to 5 s of the effects' clock, three
// strokes within a quarter second, each fading over 50 ms: at 30 ms a frame mostly caught a stroke
// nearly gone), and at each flash a new place: the most active storm of 32
// tries, near the camera for six slots, anywhere for the other two; none where storms are weak, the
// stronger the more often. (The best of 32 tries, half near and half anywhere, was nearly always on
// the afternoon side, where storms are strongest; and the weather's convection, which the test once
// read, is low at night even in a storm: no lightning near the camera at night.)
@compute @workgroup_size(8, 1, 1)
fn sim_flash(@builtin(local_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if su.flash.w < 0.5 {
        flashes[k] = vec4<f32>(0.0);
        return;
    }
    let period = mix(1.5, 5.0, hash1(k, 1u));
    let phase = (su.flash.x + hash1(k, 2u) * period) / period;
    let epoch = u32(floor(phase) % 65536.0);
    let t = fract(phase) * period;
    var bright = 0.0;
    for (var j = 0u; j < 3u; j++) {
        let at = f32(j) * mix(0.05, 0.12, hash1(k * 7u + j, epoch));
        bright += select(0.0, exp(-(t - at) / 0.05) * mix(0.4, 1.0, hash1(k * 11u + j, epoch + 1u)), t >= at);
    }
    if bright < 1e-3 {
        flashes[k] = vec4<f32>(0.0);
        return;
    }
    var best = vec3<f32>(0.0, 1.0, 0.0);
    var conv = 0.0;
    let near = k < 6u;
    for (var i = 0u; i < 32u; i++) {
        let s = epoch * 64u + i;
        let h = vec3<f32>(hash1(k * 101u + i, s), hash1(k * 103u + i, s), hash1(k * 107u + i, s));
        let d = select(rand_dir(h), rand_near(su.cam.xyz, su.cam.w, h), near);
        let c = select(textureSampleLevel(fx_in, smp, d, 0.0).b, giant_storm(d), su.flash.w > 1.5);
        if c > conv {
            conv = c;
            best = d;
        }
    }
    // Only real storms flash, the stronger the more often.
    if conv < 0.15 || hash1(k, epoch + 7u) > (conv - 0.08) * 2.0 {
        flashes[k] = vec4<f32>(0.0);
        return;
    }
    let h = mix(su.flash.y, su.flash.z, mix(0.2, 0.7, hash1(k, epoch + 9u)));
    flashes[k] = vec4<f32>(best * (su.maps.y + h), bright);
}
