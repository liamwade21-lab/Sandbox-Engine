// The ground's biome weights, shared by the ground (planet.wesl) and the objects placed on it (the
// scatter's cull pass: trees now, buildings later): which of the 12 biome slots a point of a body's
// surface shows, from SE's terra colour generator (ColorMapTerra) or, on designed worlds, the port's
// own reading of the climate. A forest must stand where the ground under it is drawn as forest, so
// both read these functions. Like the other shared modules this one has no bindings: the material's
// values (`Biomes::climate`, `Biomes::flags`, `Planet::proc_params`, ...) and the star's direction are
// passed in, one vec4 or scalar at a time (the ground is the port's most expensive shader: no copies
// of whole uniforms).
//
// Slots: 0 bottom, 1 shelf, 2 beach, 3 lowland, 4 upland, 5 rock, 6 snow, 7 ice, 8 lava, 9 steppe,
// 10 grass, 11 forest. Points are unit directions in the body frame (y the pole); `fp` is the
// footprint (radians on the unit sphere) under which octaves are dropped (0: all of them).
//
// SE's terra worlds (`Biomes::climate[4].w` 1, `climate[7].z` 0), as the ground runs them (planet.wesl
// `se_terra_mix_v`; the fragment's `se_terra_mix_n` widens two transitions by screen derivatives,
// `ws` and `cw`, 0 here); `plants` is `Biomes::flags.z > 0.5`, `rnd` `climate[6].xyz`, `h` SE's height
// map value `climate[5].z + height (m) / max(climate[4].z, 1)`:
//   n   = terra_noise_at(point, fp, rnd, plants)
//   cv  = terra_climate_noise_at(point, fp, rnd, climate[7], climate[5].y)
//   s   = se_terra_snow_at(point, h, slope, n.x, snow_vary_at(point, fp, rnd), climate[0], climate[1],
//                          climate[2], climate[6].w > 0 (tidally locked), star direction in the body frame)
//   c   = se_terra_climate_at(slope, s, ws, cv, climate[0], climate[1])
//   ptf = plants_ptf_at(point, fp) where plants, else 0
//   m   = se_terra_weights_at(point, h, slope, fp, c, cw, n, ptf, climate[0], climate[2], climate[3],
//                             climate[4], plants, rock_detail)
// Designed worlds (`climate[4].w` 0): `designed_noise_at` then `biome_weights_at`.

import super::gas_giant::{gg_noise};

// ---------------------------------------------------------------------------
// The port's own value noise (designed worlds).

fn hash31(p: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 0x8da6b343u) ^ (bitcast<u32>(p.y) * 0xd8163841u) ^ (bitcast<u32>(p.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967295.0);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let b = vec3<i32>(i);
    let a000 = hash31(b);
    let a100 = hash31(b + vec3<i32>(1, 0, 0));
    let a010 = hash31(b + vec3<i32>(0, 1, 0));
    let a110 = hash31(b + vec3<i32>(1, 1, 0));
    let a001 = hash31(b + vec3<i32>(0, 0, 1));
    let a101 = hash31(b + vec3<i32>(1, 0, 1));
    let a011 = hash31(b + vec3<i32>(0, 1, 1));
    let a111 = hash31(b + vec3<i32>(1, 1, 1));
    let x0 = mix(mix(a000, a100, u.x), mix(a010, a110, u.x), u.y);
    let x1 = mix(mix(a001, a101, u.x), mix(a011, a111, u.x), u.y);
    return mix(x0, x1, u.z) * 2.0 - 1.0;
}

fn fbm3(p: vec3<f32>, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var o = 0; o < octaves; o = o + 1) {
        sum += amp * vnoise(q);
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
        amp *= 0.5;
    }
    return sum;
}

// ---------------------------------------------------------------------------
// SE's terrain noise for the colour map (tg_common.glh Fbm, Fbm3D and RidgedMultifractal over SE's
// hash Perlin): octaves finer than about a pixel (`fp`, radians on the unit sphere) are dropped,
// where SE's coarser tiles average them.
fn ter_oct(octaves: f32, freq: f32, lac: f32, fp: f32) -> f32 {
    return clamp(floor(log2(0.5 / max(freq * fp, 1e-30)) / log2(lac)) + 1.0, 0.0, octaves);
}

fn ter_fbm(point: vec3<f32>, octaves: f32, lac: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    let gain = pow(lac, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        sum += gg_noise(pt) * ampl;
        ampl *= gain;
        pt *= lac;
    }
    return sum;
}

fn ter_fbm3d(point: vec3<f32>, octaves: f32) -> vec3<f32> {
    var pt = point;
    var sum = vec3<f32>(0.0);
    var ampl = 1.0;
    let lac = 2.218281828459;
    let gain = pow(lac, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        sum += vec3<f32>(gg_noise(pt), gg_noise(pt + vec3<f32>(3.33, 5.71, 1.96)), gg_noise(pt + vec3<f32>(7.77, 2.65, 4.37))) * ampl;
        ampl *= gain;
        pt *= lac;
    }
    return sum;
}

// RidgedMultifractal with SE's noiseOffset 0.8, noiseRidgeSmooth 0.0001 and noiseH 0.5.
fn ter_rmf(point: vec3<f32>, gain: f32, octaves: f32) -> f32 {
    var signal = 1.0;
    var sum = 0.0;
    var freq = 1.0;
    for (var i = 0; f32(i) < octaves; i++) {
        let weight = saturate(signal * gain);
        signal = gg_noise(point * freq);
        signal = 0.8 - sqrt(0.0001 + signal * signal);
        signal *= signal * weight;
        sum += signal * pow(freq, -0.5);
        freq *= 2.218281828459;
    }
    return sum;
}

// ---------------------------------------------------------------------------
// The slots' weights.

// SE's PlantsBlendCoeff, its 0.025 ramps widened by `cw` (the climate's change across the pixel).
fn plants_coeff(lo: f32, hi: f32, climate: f32, fade: f32, trans_fractal: f32, cw: f32) -> f32 {
    let t = fade * smoothstep(lo - cw, lo + 0.025 + cw, climate) * (1.0 - smoothstep(hi - 0.025 - cw, hi + cw, climate));
    return saturate(t + trans_fractal * pow(abs(t - 0.5), 0.125));
}

// SE's sequential BlendMaterials as slot weights: the surface so far gives way to slot `k` by `t`.
fn blend_slot(w: ptr<function, array<f32, 12>>, k: i32, t: f32) {
    for (var j = 0; j < 12; j++) {
        (*w)[j] *= 1.0 - t;
    }
    (*w)[k] += t;
}

struct TerraMix {
    w: array<f32, 12>,
    // The cliff layers' tint of the rock slot (SE: cliff colour x (1 - (0, 0.5, 1) layers)).
    rock_tint: vec3<f32>,
    // The colour then goes `mix(colorVary, 1, vary)` x `dark`, then towards white by `white`.
    vary: f32,
    dark: f32,
    white: f32,
};

// SE's ColorMapTerra noises that vary only over kilometres: x the latitude's noise, y the albedo
// variation (albedoVary before its curve), z on worlds with `plants` the humidity's Fbm (else 0).
// SE warps the humidity and the albedo variation alike, by 1.3 Fbm3D((point + Randomize) 0.07).
// `rnd`: SE's Randomize (`Biomes::climate[6].xyz`).
fn terra_noise_at(point: vec3<f32>, fp: f32, rnd: vec3<f32>, plants: bool) -> vec3<f32> {
    let lac = 2.218281828459;
    let lat_noise = 0.15 * (ter_fbm(point * 0.7 + rnd, ter_oct(6.0, 0.7, lac, fp), lac) - 1.0);
    let avd = ter_fbm3d((point + rnd) * 0.07, ter_oct(4.0, 0.07, lac, fp)) * 1.3;
    let av = 1.0 - 0.5 * ter_fbm(point + avd, ter_oct(8.0, 1.0, lac, fp), lac);
    var humidity = 0.0;
    if plants {
        humidity = ter_fbm((point + avd) * 1.73, ter_oct(8.0, 1.73, lac, fp), lac);
    }
    return vec3<f32>(lat_noise, av, humidity);
}

// SE's flatland climate distortion (tg_terra_color.glsl): ridged multifractals by hillsFreq /
// hillsMagn (`hills`: `climate[7]`), then an Fbm by colorDistFreq (`dist_freq`: `climate[5].y`), warped.
fn terra_climate_noise_at(point: vec3<f32>, fp: f32, rnd: vec3<f32>, hills: vec4<f32>, dist_freq: f32) -> f32 {
    let lac = 2.218281828459;
    let pf = 0.0005 * hills.x / max(hills.y * hills.y, 1e-6);
    let pp = (point + rnd) * pf;
    let fr = 0.20 * (1.5 - ter_rmf(pp, 2.0, ter_oct(4.0, pf, lac, fp)))
        + 0.05 * (1.5 - ter_rmf(pp * 10.0, 2.0, ter_oct(4.0, pf * 10.0, lac, fp)))
        + 0.02 * (1.5 - ter_rmf(pp * 100.0, 2.0, ter_oct(4.0, pf * 100.0, lac, fp)));
    let df = dist_freq * 0.005;
    var p = point * df + vec3<f32>(fr);
    p += ter_fbm3d(p * 0.38, ter_oct(4.0, df * 0.38, lac, fp)) * 1.2;
    return ter_fbm(p, ter_oct(4.0, df, lac, fp), lac) * 0.35 + 0.245;
}

// The snow line's variation (`se_terra_snow_at`'s `vary`): metre-scale, so the objects on the ground
// work it out once where they stand.
fn snow_vary_at(point: vec3<f32>, fp: f32, rnd: vec3<f32>) -> f32 {
    return ter_fbm(point * 1700.0 + rnd, ter_oct(5.0, 1700.0, 3.5, fp), 3.5);
}

// x the climate (before the snow), y the snow line, z the latitude. `lat_noise`: `terra_noise_at`'s x;
// `vary`: `snow_vary_at`; `c0`..`c2`: `climate[0]`..`climate[2]`; `tidal`: the world is tidally locked
// (`climate[6].w > 0`), its climate then from the substellar point, `sun_body` (the star's direction
// in the body frame).
fn se_terra_snow_at(point: vec3<f32>, h: f32, slope: f32, lat_noise: f32, vary: f32, c0: vec4<f32>, c1: vec4<f32>, c2: vec4<f32>, tidal: bool, sun_body: vec3<f32>) -> vec3<f32> {
    let sea = c0.x;
    let snow_level = c0.y;
    let pole = c1.x;
    let tropic = c1.y;
    let equator = c1.z;
    let lat_tropic = c1.w;
    let tropic_width = c2.x;
    var latitude: f32;
    var climate: f32;
    if !tidal {
        latitude = saturate(abs(point.y) + lat_noise);
        if latitude < lat_tropic - tropic_width {
            climate = mix(tropic, equator, saturate((lat_tropic - latitude - tropic_width) / lat_tropic));
        } else if latitude > lat_tropic + tropic_width {
            climate = mix(tropic, pole, saturate((latitude - lat_tropic - tropic_width) / (1.0 - lat_tropic)));
        } else {
            climate = tropic;
        }
    } else {
        latitude = 1.0 - dot(point, sun_body) + lat_noise;
        climate = mix(tropic, pole, saturate(latitude));
    }
    let snow_line = h + 0.25 * vary * slope;
    let mont_height = saturate((h - sea) / (snow_level - sea));
    climate = min(climate + c2.y * mont_height, pole - 0.125);
    return vec3<f32>(climate, snow_line, latitude);
}

// x the climate, y the ice cap. `s`: `se_terra_snow_at`; `ws`: the snow line's change over a pixel
// (0: SE's sharp line); `vary`: `terra_climate_noise_at`; `c0`, `c1`: `climate[0]`, `climate[1]`.
fn se_terra_climate_at(slope: f32, s: vec3<f32>, ws: f32, vary: f32, c0: vec4<f32>, c1: vec4<f32>) -> vec2<f32> {
    let snow_level = c0.y;
    let lat_ice = c0.z;
    let pole = c1.x;
    // SE's sharp transitions, softened where they would change within a pixel: the slope comes from
    // the node's normal map, coarser than SE's colour tiles, and would flip whole texels.
    var climate = mix(s.x, pole, saturate((s.y - snow_level) * min(100.0, 0.5 / max(ws, 1e-6))));
    let ice_cap = saturate((s.z / lat_ice - 1.0) * 50.0);
    climate = mix(climate, pole, ice_cap);
    climate += vary * saturate(1.0 - 3.0 * slope) * saturate(1.0 - 1.333 * climate);
    return vec2<f32>(climate, ice_cap);
}

// SE's plants' transition fractal (`se_terra_weights_at`'s `ptf`), metre-scale: worked out once where
// an object stands.
fn plants_ptf_at(point: vec3<f32>, fp: f32) -> f32 {
    let lac = 2.218281828459;
    return abs(0.125 * ter_fbm(point * 3.0e5, ter_oct(2.0, 3.0e5, lac, fp), lac) + 0.125 * ter_fbm(point * 1.0e3, ter_oct(2.0, 1.0e3, lac, fp), lac));
}

// The slots' weights. `c`: `se_terra_climate_at`; `cw`: the climate's change over a pixel (0: SE's
// sharp ranges); `n`: `terra_noise_at`; `ptf`: `plants_ptf_at` (unused without `plants`); `c0`, `c2`,
// `c3`, `c4`: `climate[0]`, `climate[2]`..`climate[4]`. `rock_detail` false leaves the cliff layers'
// tint (`rock_tint`, an expensive noise) at 1; the weights are the same.
fn se_terra_weights_at(point: vec3<f32>, h: f32, slope: f32, fp: f32, c: vec2<f32>, cw: f32, n: vec3<f32>, ptf: f32, c0: vec4<f32>, c2: vec4<f32>, c3: vec4<f32>, c4: vec4<f32>, plants: bool, rock_detail: bool) -> TerraMix {
    let sea = c0.x;
    let lac = 2.218281828459;
    var o: TerraMix;
    o.rock_tint = vec3<f32>(1.0);
    var w: array<f32, 12>;
    o.dark = 1.0;
    let climate = c.x;
    let ice_cap = c.y;

    // The base surface: beach at sea level, then lowland, upland and rock up to the top; the shelf
    // and bottom below.
    let hb = (h - sea) / max(1.0 - sea, 1e-3) * 4.0 + 2.0;
    let h0 = clamp(i32(floor(hb)), 0, 5);
    let h1 = clamp(h0 + 1, 0, 5);
    let dh = fract(hb);
    w[h0] += 1.0 - dh;
    w[h1] += dh;

    if plants {
        let humidity = n.z - 1.0 + c2.z * 2.0;
        let beach = c2.w;
        let fade = smoothstep(beach, beach * 2.0, h - sea) * (1.0 - smoothstep(0.65, 0.75, slope)) * smoothstep(-0.5, 0.5, humidity);
        var t = plants_coeff(c3.x, c3.y, climate, fade, ptf, cw);
        if t > 0.0 {
            blend_slot(&w, 9, t);
        }
        t = plants_coeff(c4.x, c4.y, climate, fade, ptf, cw);
        if t > 0.0 {
            blend_slot(&w, 11, t);
        }
        t = plants_coeff(c3.z, c3.w, climate, fade, ptf, cw);
        if t > 0.0 {
            blend_slot(&w, 10, t);
        }
    }
    if climate > 0.9 - cw {
        blend_slot(&w, 6, smoothstep(0.9 - cw, 0.92 + cw, climate));
    }
    if ice_cap == 0.0 && slope > 0.35 {
        let t = smoothstep(0.35, 0.65, slope);
        if rock_detail {
            let q = point * 378.3 + vec3<f32>(gg_noise(point * 967.7), gg_noise(point * 967.7 + vec3<f32>(3.33, 5.71, 1.96)), gg_noise(point * 967.7 + vec3<f32>(7.77, 2.65, 4.37))) * 10.0;
            let v = saturate(ter_fbm(q, ter_oct(5.0, 378.3, lac, fp), lac) * 0.7 + 0.5);
            let layers = ter_fbm(vec3<f32>(h * 168.4 + 0.17 * v, 0.43 * (q.x + q.y), 0.43 * (q.z - q.y)), ter_oct(4.0, 168.4 * 0.43 * 378.3, lac, fp), lac);
            o.rock_tint = mix(vec3<f32>(1.0), vec3<f32>(1.0) - vec3<f32>(0.0, 0.5, 1.0) * layers, t / max(w[5] * (1.0 - t) + t, 1e-4));
        }
        blend_slot(&w, 5, t);
    }
    // SE's albedoVary (`terra_noise_at`'s y), cubed and smoothstepped.
    let av = n.y;
    o.vary = smoothstep(0.0, 1.0, av * av * av);
    o.w = w;
    return o;
}

// ---------------------------------------------------------------------------
// Designed worlds (bodies without SE's generator parameters).

// The noises `biome_weights_at` takes: x its `n`, y its `moist`. `radius` (m): `Planet::proc_params2.x`;
// `seed_frac`: `proc_params2.y`.
fn designed_noise_at(dir: vec3<f32>, radius: f32, seed_frac: f32) -> vec2<f32> {
    let seed = vec3<f32>(seed_frac * 917.0, seed_frac * 331.0, seed_frac * 613.0);
    let n = fbm3(dir * (radius / 30000.0) + seed, 4);
    let moist = fbm3(dir * (radius / 400000.0) + seed.zxy, 4) * 0.8 + 0.5;
    return vec2<f32>(n, moist);
}

// Relative weights of the 12 biome slots. `h`: the height (m), `lat`: |dir.y|; `proc_params`:
// `Planet::proc_params` (x surface kind, y relief (m), z temperature (K)); `sea`: the body has water
// (`Planet::ocean_sea.w > 0.5`); `plants`: `Biomes::flags.z > 0.5`.
fn biome_weights_at(h: f32, slope: f32, lat: f32, n: f32, moist: f32, proc_params: vec4<f32>, sea: bool, plants: bool) -> array<f32, 12> {
    var w: array<f32, 12>;
    let relief = max(proc_params.y, 1.0);
    let temp = proc_params.z;
    let hn = h / relief + n * 0.08;
    let polar = smoothstep(0.75, 0.95, lat + n * 0.05);
    let cliff = smoothstep(0.3, 0.5, slope);
    var under = 0.0;
    if sea {
        under = 1.0 - smoothstep(-5.0, 0.0, h);
        let bottom = 1.0 - smoothstep(-0.5, -0.15, hn);
        w[0] = under * bottom;
        w[1] = under * (1.0 - bottom);
        let beach_h = max(relief * 0.01, 15.0);
        w[2] = (1.0 - under) * (1.0 - smoothstep(beach_h * 0.5, beach_h, h)) * (1.0 - cliff);
    }
    let land = (1.0 - under) * (1.0 - w[2]);
    let low = 1.0 - smoothstep(0.2, 0.4, hn);
    let high = smoothstep(0.55, 0.75, hn);
    let up = (1.0 - low) * (1.0 - high);
    // Snow line: high and polar ground on temperate worlds, everything on cold ones.
    let cold = clamp((270.0 - temp) / 60.0, 0.0, 1.0);
    let snow_line = mix(1.1, -0.3, cold) - polar * 0.7;
    let snow = smoothstep(snow_line, snow_line + 0.1, hn) * (1.0 - cliff * 0.7);
    // Ice: frozen worlds, and polar caps on cool ones.
    let ice = max(1.0 - smoothstep(130.0, 170.0, temp), polar * (1.0 - smoothstep(250.0, 290.0, temp))) * (1.0 - cliff);
    var lava = 0.0;
    if abs(proc_params.x - 3.0) < 0.5 {
        lava = 1.0 - smoothstep(0.1, 0.3, hn);
    }
    let bare = land * (1.0 - snow) * (1.0 - ice) * (1.0 - lava);
    w[5] = bare * max(cliff, high);
    var veg = 0.0;
    if plants {
        veg = smoothstep(250.0, 270.0, temp) * (1.0 - smoothstep(310.0, 330.0, temp)) * (1.0 - polar);
    }
    let ground = bare * (1.0 - cliff);
    w[3] = ground * low * (1.0 - veg);
    w[4] = ground * up * (1.0 - veg * 0.5);
    let lv = ground * low * veg;
    w[9] = lv * (1.0 - smoothstep(0.3, 0.5, moist)) + ground * up * veg * 0.5;
    w[10] = lv * smoothstep(0.3, 0.5, moist) * (1.0 - smoothstep(0.55, 0.7, moist));
    w[11] = lv * smoothstep(0.55, 0.7, moist);
    w[6] = land * snow * (1.0 - ice);
    w[7] = land * ice;
    w[8] = land * lava * (1.0 - snow);
    return w;
}
