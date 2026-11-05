// A generated planet's settlements (the vegetation and civilizations plan, step 6): where its
// civilization's cities, towns and villages are, as a density field over the sphere, and the
// farmland round them. The ground draws them from orbit (planet.wesl: towns' colour by day, their
// lights by night, the fields' patchwork); the city sectors of later steps will read the same field.
// No bindings: the level and the culture seed are passed in.
//
// Settlements are blobs in cells of three sizes (villages, towns, cities), each cell holding one
// with a chance, at a random point, of a random size (log-uniform). The level sets which sizes it
// has and how big: a primitive people villages alone (under 0.5 % of the land), a modern one towns
// and cities (about 3 %), a futuristic one megacities (about 10 %), an interstellar one megalopolises
// (about 20 %). The ground keeps them where it suits (low, flat, fertile land, near the coast).
// Blobs smaller than a pixel keep their light as dim dots; cells smaller than a pixel fade into
// their mean, so they never sparkle from orbit.

fn st_hash(c: vec3<i32>, seed: u32) -> vec4<f32> {
    var h = (bitcast<u32>(c.x) * 0x8da6b343u) ^ (bitcast<u32>(c.y) * 0xd8163841u) ^ (bitcast<u32>(c.z) * 0xcb1ab31fu) ^ (seed * 0x9e3779b9u);
    var o: vec4<f32>;
    for (var i = 0; i < 4; i++) {
        h = (h ^ (h >> 16u)) * 0x7feb352du;
        h = (h ^ (h >> 15u)) * 0x846ca68bu;
        h = h ^ (h >> 16u);
        o[i] = f32(h >> 8u) * (1.0 / 16777216.0);
    }
    return o;
}

// One size of settlement: cells of `cell` km, a settlement in each with `chance`, of radius
// `r_lo`..`r_hi` km (log-uniform), at `p` km on the body. Returns (urban density, farmland halo),
// the halo `halo` times wider. `fp` is the pixel's footprint (km).
//
// A settlement is a Gaussian blob (density 1 at its heart, its area pi r^2). Seen from afar each
// blob is blurred to the pixel's size keeping its whole light (radius sqrt(r^2 + s^2), height
// r^2 / (r^2 + s^2)), so a town smaller than a pixel stays a dim dot, as towns' lights do from
// orbit. Once the pixel spans a good part of a cell, the 8 cells round the point no longer hold
// all the light a pixel gathers: the field turns into its mean (no sparkle).
fn st_size(p: vec3<f32>, cell: f32, chance: f32, r_lo: f32, r_hi: f32, halo: f32, seed: u32, fp: f32) -> vec2<f32> {
    let q = p / cell;
    let base = vec3<i32>(floor(q - 0.5));
    let s2 = 0.25 * fp * fp;
    var urban = 0.0;
    var farm = 0.0;
    for (var k = 0; k < 8; k++) {
        let c = base + vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        let h = st_hash(c, seed);
        if h.w >= chance {
            continue;
        }
        let r = r_lo * pow(r_hi / r_lo, fract(h.w * 7.31 + h.x * 3.17));
        let dq = (q - (vec3<f32>(c) + h.xyz)) * cell;
        let d2 = dot(dq, dq);
        let re2 = r * r + s2;
        urban += r * r / re2 * exp(-d2 / re2);
        let rh2 = r * r * halo * halo;
        let fe2 = rh2 + s2;
        farm += rh2 / fe2 * exp(-d2 / fe2);
    }
    // The field's mean: chance x E[pi r^2] / cell^2 (r log-uniform: E[r^2] = (b^2 - a^2) / 2 ln(b/a)).
    let er2 = (r_hi * r_hi - r_lo * r_lo) / (2.0 * log(r_hi / r_lo));
    let mean_u = chance * 3.14159 * er2 / (cell * cell);
    let m = smoothstep(0.15 * cell, 0.4 * cell, 0.5 * fp);
    return vec2<f32>(mix(urban, mean_u, m), mix(farm, min(mean_u * halo * halo, 1.0), m));
}

// The settlements of a civilization of `level` (1-4) with culture `seed` at body-frame direction
// `dir` on a body of `radius_m`, the pixel's footprint `fp_m`: (urban density 0..1, farmland 0..1).
fn settlements(dir: vec3<f32>, radius_m: f32, level: u32, seed: u32, fp_m: f32) -> vec2<f32> {
    let p = dir * (radius_m / 1000.0);
    let fp = fp_m / 1000.0;
    // The field's mean (the urban share where the land suits fully): villages 0.2 %; with towns
    // and cities about 3 % (level 2); bigger towns and megacities about 8 % (level 3);
    // megalopolises about 17 % (level 4).
    var s = st_size(p, 14.0, 0.45, 0.25, 0.9, 6.0, seed, fp);
    if level >= 2u {
        let big = level >= 3u;
        s += st_size(p, 55.0, 0.9, select(1.5, 2.5, big), select(7.0, 10.0, big), 4.0, seed + 1u, fp);
        s += st_size(p, 280.0, select(0.65, 0.7, big), select(10.0, 20.0, big), select(35.0, 70.0, big), 3.0, seed + 2u, fp);
    }
    if level >= 4u {
        s += st_size(p, 420.0, 0.7, 45.0, 130.0, 2.0, seed + 3u, fp);
    }
    // Primitive villages are small and sparse; the others' cores are dense.
    let k = select(1.0, 0.6, level == 1u);
    return vec2<f32>(min(s.x * k, 1.0), min(s.y, 1.0));
}

// The countryside's lights (farms, hamlets, roads): points of light about 400 m apart whose mean
// over the field is 1. From afar they blur into an even glow over the farmland (Earth's populated
// land glows far beyond its towns at night); close up they are separate lights.
fn st_points(dir: vec3<f32>, radius_m: f32, seed: u32, fp_m: f32) -> f32 {
    let r_lo = 0.015;
    let r_hi = 0.035;
    let pts = st_size(dir * (radius_m / 1000.0), 0.4, 0.6, r_lo, r_hi, 1.0, seed + 5u, fp_m / 1000.0);
    let er2 = (r_hi * r_hi - r_lo * r_lo) / (2.0 * log(r_hi / r_lo));
    return pts.x / (0.6 * 3.14159 * er2 / (0.4 * 0.4));
}

// The fields' patchwork round settlements: a brightness and hue factor per field of about 300 m
// (squares in the cube face's own frame), fading to 1 when the fields are smaller than the pixel.
fn st_fields(dir: vec3<f32>, radius_m: f32, seed: u32, fp_m: f32) -> vec3<f32> {
    let p = dir * (radius_m / 300.0);
    let h = st_hash(vec3<i32>(floor(p)), seed + 7u);
    let tint = vec3<f32>(0.75 + 0.5 * h.x, 0.8 + 0.4 * h.y, 0.7 + 0.4 * h.z * h.x);
    return mix(tint, vec3<f32>(1.0), smoothstep(150.0, 600.0, fp_m));
}
