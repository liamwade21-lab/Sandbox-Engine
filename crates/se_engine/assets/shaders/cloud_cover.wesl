// Cloud cover generators, shared by the flat cloud shells (clouds.wesl) and the volumetric clouds'
// cover map (volumetric_clouds.wesl): SE's terra clouds (`se_terra_clouds`) and the port's own
// procedural clouds (`proc_clouds`). Points are unit directions in the cloud layer's frame (y the pole).

const PI: f32 = 3.14159265358979;

// SE's 5% cut (`cloudsFade`: the flat shells drop texels under it and rescale the rest) of a map's or
// generator's opacity. The volumetric clouds' cover map holds it already cut.
fn se_cut(raw: f32) -> f32 {
    return max(raw - 0.05, 0.0) / 0.95;
}

// A layer's cover from SE's cut cover `c`, shaped by the weather (`mul`, `add`) and scaled by the
// layer's opacity: the flat shells, the volumetric clouds, their rain and the ground's cloud shadows.
fn weather_cover(c: f32, mul: f32, add: f32, opacity: f32) -> f32 {
    return clamp((c * mul + add) * opacity, 0.0, 1.0);
}

// The weather's own cloud (its cover offset `add` where positive, taken as the share of the area its
// clouds fill) shaped by its pattern's value `m` there (volumetric_clouds.wesl `weather_pattern`,
// spread about 0.08 around 0.5): the pattern above the level that leaves that share, denser towards
// its crests. A clearing (`add` negative) is left as it is. The volumetric clouds, their rain and the
// ground's cloud shadows (planet.wesl) all shape it so.
fn weather_shaped(add: f32, m: f32) -> f32 {
    if add <= 0.02 {
        return add;
    }
    let th = 0.5 + 0.22 * (0.5 - min(add, 1.0));
    return smoothstep(th - 0.05, th + 0.05, m) * (0.55 + 0.45 * smoothstep(th, th + 0.12, m));
}

// How far a ray from radius `r0`, rising at `mu` (the cosine of its angle from the vertical, >= 0), goes
// to climb `dh` higher over a round body: the ground's cloud shadows (planet.wesl `cloud_shadow`,
// volumetric_clouds.wesl `shadow_gen`). (Taken as dh / mu, as over flat ground, a low star's ray from
// the ground passed far above the clouds that shade it: a square of sunlit ground around the camera.)
fn climb_dist(r0: f32, mu: f32, dh: f32) -> f32 {
    let c = dh * (2.0 * r0 + dh);
    return c / max(r0 * mu + sqrt(r0 * r0 * mu * mu + c), 1e-6);
}

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
    let x0 = mix(mix(hash31(b), hash31(b + vec3<i32>(1, 0, 0)), u.x),
                 mix(hash31(b + vec3<i32>(0, 1, 0)), hash31(b + vec3<i32>(1, 1, 0)), u.x), u.y);
    let x1 = mix(mix(hash31(b + vec3<i32>(0, 0, 1)), hash31(b + vec3<i32>(1, 0, 1)), u.x),
                 mix(hash31(b + vec3<i32>(0, 1, 1)), hash31(b + vec3<i32>(1, 1, 1)), u.x), u.y);
    return mix(x0, x1, u.z);
}

// fBm in [0, 1] whose finest octaves fade once smaller than a pixel (`fw` = pixel
// footprint in the same units as `p`).
fn fbm_aa(p: vec3<f32>, octaves: i32, fw: f32) -> f32 {
    var s = 0.0;
    var norm = 0.0;
    var amp = 0.5;
    var q = p;
    var f = 1.0;
    for (var i = 0; i < octaves; i++) {
        let w = clamp(1.0 - fw * f * 1.5, 0.0, 1.0);
        s += amp * mix(0.5, vnoise(q), w);
        norm += amp;
        amp *= 0.58;
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
        f *= 2.03;
    }
    return s / norm;
}

// ---------------------------------------------------------------------------
// SE's terra clouds (tg_common.glh HeightMapCloudsTerra, CycloneNoiseTerra, TurbulenceTerra,
// FbmClouds; tg_clouds_color.glsl), with SE's hash Perlin noise and spherical Fibonacci cells as in
// planet.wesl's gas giants. The point is the unit direction in the layer's frame (y the pole).

fn se_hash(gridcell_in: vec3<f32>) -> array<vec4<f32>, 6> {
    const OFFSET = vec2<f32>(50.0, 161.0);
    const DOMAIN: f32 = 69.0;
    const SOMELARGEFLOATS = vec3<f32>(635.298681, 682.357502, 668.926525);
    const ZINC = vec3<f32>(48.500388, 65.294118, 63.934599);
    var gridcell = gridcell_in;
    gridcell = gridcell - floor(gridcell * (1.0 / DOMAIN)) * DOMAIN;
    let inc1 = select(gridcell + vec3<f32>(1.0), vec3<f32>(0.0), gridcell > vec3<f32>(DOMAIN - 1.5));
    var P = vec4<f32>(gridcell.xy, inc1.xy) + OFFSET.xyxy;
    P *= P;
    P = P.xzxz * P.yyww;
    let lowz = vec4<f32>(1.0) / (SOMELARGEFLOATS.xyzx + vec2<f32>(gridcell.z, inc1.z).xxxy * ZINC.xyzx);
    let highz = vec2<f32>(1.0) / (SOMELARGEFLOATS.yz + inc1.zz * ZINC.yz);
    return array<vec4<f32>, 6>(fract(P * lowz.xxxx), fract(P * lowz.yyyy), fract(P * lowz.zzzz), fract(P * lowz.wwww), fract(P * highz.xxxx), fract(P * highz.yyyy));
}

fn se_noise(p: vec3<f32>) -> f32 {
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf1 = Pf - 1.0;
    let h = se_hash(Pi);
    let C = vec4<f32>(0.49999);
    let gx0 = h[0] - C;
    let gy0 = h[1] - C;
    let gz0 = h[2] - C;
    let gx1 = h[3] - C;
    let gy1 = h[4] - C;
    let gz1 = h[5] - C;
    let r0 = inverseSqrt(gx0 * gx0 + gy0 * gy0 + gz0 * gz0) * (vec2<f32>(Pf.x, Pf1.x).xyxy * gx0 + vec2<f32>(Pf.y, Pf1.y).xxyy * gy0 + Pf.zzzz * gz0);
    let r1 = inverseSqrt(gx1 * gx1 + gy1 * gy1 + gz1 * gz1) * (vec2<f32>(Pf.x, Pf1.x).xyxy * gx1 + vec2<f32>(Pf.y, Pf1.y).xxyy * gy1 + Pf1.zzzz * gz1);
    let b = Pf * Pf * Pf * (Pf * (Pf * 6.0 - 15.0) + 10.0);
    let m0 = mix(r0, r1, b.z);
    let m1 = mix(m0.xy, m0.zw, b.y);
    return mix(m1.x, m1.y, b.x) * 1.1547005383792515;
}

fn se_hash1(p: f32) -> f32 {
    return fract(sin(p) * 158.5453123);
}

fn se_rotate(angle: f32, axis: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    let t = 1.0 - c;
    let m = mat3x3<f32>(
        t * axis.x * axis.x + c, t * axis.x * axis.y - s * axis.z, t * axis.x * axis.z + s * axis.y,
        t * axis.x * axis.y + s * axis.z, t * axis.y * axis.y + c, t * axis.y * axis.z - s * axis.x,
        t * axis.x * axis.z - s * axis.y, t * axis.y * axis.z + s * axis.x, t * axis.z * axis.z + c);
    return v * m;
}

// SE's inverseSF: the nearest of n spherical Fibonacci points (cell index, distance) and the point.
fn se_isf(p: vec3<f32>, n: f32, nearest: ptr<function, vec3<f32>>) -> vec2<f32> {
    const phi: f32 = 1.61803398875;
    let m = 1.0 - 1.0 / n;
    let fi = min(atan2(p.y, p.x), PI);
    let cosTheta = p.z;
    let k = max(2.0, floor(log(n * PI * sqrt(5.0) * (1.0 - cosTheta * cosTheta)) / log(phi + 1.0)));
    let Fk = pow(phi, k) / sqrt(5.0);
    let F = vec2<f32>(floor(Fk + 0.5), floor(Fk * phi + 0.5));
    let ka = 2.0 * F / n;
    let kb = 2.0 * PI * (fract((F + 1.0) * phi) - (phi - 1.0));
    let det = ka.y * kb.x - ka.x * kb.y;
    let iB = mat2x2<f32>(ka.y / det, -ka.x / det, kb.y / det, -kb.x / det);
    let c = floor(iB * vec2<f32>(fi, cosTheta - m));
    var d = 8.0;
    var j = 0.0;
    for (var s = 0; s < 4; s++) {
        let uv = vec2<f32>(f32(s - 2 * (s / 2)), f32(s / 2));
        let i = dot(F, uv + c);
        let fs = 2.0 * PI * fract(i * phi);
        let ct = m - 2.0 * i / n;
        // Past the pole SE's sqrt is NaN and the candidate never wins.
        if abs(ct) > 1.0 {
            continue;
        }
        let st = sqrt(1.0 - ct * ct);
        let q = vec3<f32>(cos(fs) * st, sin(fs) * st, ct);
        let rr = q - p;
        let d2 = dot(rr, rr);
        if d2 < d {
            *nearest = q;
            d = d2;
            j = i;
        }
    }
    return vec2<f32>(j, sqrt(d));
}

// FbmClouds / FbmClouds3D: amplitude x 0.333 and frequency x pi per octave.
fn se_fbm_clouds(point: vec3<f32>, octaves: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    for (var i = 0; f32(i) < octaves; i++) {
        sum += se_noise(pt) * ampl;
        ampl *= 0.333;
        pt *= 3.1416;
    }
    return sum;
}

fn se_fbm_clouds3(point: vec3<f32>, octaves: f32) -> vec3<f32> {
    var pt = point;
    var sum = vec3<f32>(0.0);
    var ampl = 1.0;
    for (var i = 0; f32(i) < octaves; i++) {
        sum += vec3<f32>(se_noise(pt), se_noise(pt + vec3<f32>(3.33, 5.71, 1.96)), se_noise(pt + vec3<f32>(7.77, 2.65, 4.37))) * ampl;
        ampl *= 0.333;
        pt *= 3.1416;
    }
    return sum;
}

struct CycloneOut {
    p: vec3<f32>,
    weight: f32,
    coverage: f32,
};

// CycloneNoiseTerra: cyclones in random spherical Fibonacci cells, octave by octave (twice the
// frequency and density each), twisting the point around the cell centre.
fn se_cyclones(point_in: vec3<f32>, coverage_in: f32, se: array<vec4<f32>, 3>) -> CycloneOut {
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    var weight = 1.0;
    var coverage = coverage_in;
    let mag = -se[2].w * se[1].x;
    var freq = se[1].y * 50.0;
    var dens = se[1].z * 0.02;
    var size = 8.0;
    for (var i = 0; f32(i) < se[1].w; i++) {
        let cell = se_isf(point, freq, &center);
        let rnd = se_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dist = 1.0 - r;
            let fi = mix(log(r), dist * dist * dist, r);
            twisted = se_rotate(mag * sign(center.y + 0.001) * fi, center, point);
            let w = saturate(1.0 - r * 10.0);
            weight = min(weight, 1.0 - w * w);
            coverage = mix(coverage, 1.0, dist);
        }
        freq *= 2.0;
        dens *= 2.0;
        size *= 2.0;
        point = twisted;
    }
    var o: CycloneOut;
    o.p = twisted;
    o.weight = saturate(weight);
    o.coverage = coverage;
    return o;
}

// TurbulenceTerra: two octaves of small vortices.
fn se_turbulence(point_in: vec3<f32>) -> vec3<f32> {
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    var strength = 5.5;
    var freq = 20.0 * 0.7;
    var size = 4.0 * 0.7;
    let dens = 0.3;
    for (var i = 0; i < 2; i++) {
        let cell = se_isf(point, freq, &center);
        let rnd = se_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.5 * dens - rnd);
            let dist = saturate(1.0 - r);
            let dist2 = saturate(0.5 - r);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.25);
            twisted = se_rotate(dir * 15.0 * sign(center.y + 0.001) * fi, center, point);
        }
        freq = min(freq * 2.0, 1600.0);
        size = min(size * 1.2, 30.0);
        strength *= 1.5;
        point = twisted;
    }
    return twisted;
}

// HeightMapCloudsTerra, then tg_clouds_color's `saturate(h^2 3.5 + h)` opacity. On tidally locked
// worlds one giant cyclone sits at the substellar point `sun` (SE's cloud frame puts it on y; here
// the star's direction in the layer's frame) and the night side stays clear. `fp`: the pixel's
// footprint (radians), finer noise octaves dropped.
fn se_terra_clouds(point: vec3<f32>, coverage_in: f32, fp: f32, sun: vec3<f32>, se: array<vec4<f32>, 3>) -> f32 {
    let se0 = se[0];
    let zones = cos(point.y * se0.z);
    let ang = zones * se0.w;
    var twisted = point;
    var coverage = coverage_in;
    var weight = 1.0;
    if se[2].w > 0.0 {
        let center = sun;
        let r = length(center - point);
        let mag = -se[2].w * se[1].x;
        if r < 1.0 {
            let dist = 1.0 - r;
            let fi = mix(log(r), dist * dist * dist, r);
            twisted = se_rotate(mag * fi, center, point);
            weight = saturate(r * 40.0 - 0.05);
            weight = weight * weight;
            coverage = mix(coverage, 1.0, dist);
        }
        weight *= smoothstep(-0.2, 0.0, dot(point, sun));
    } else {
        let c = se_cyclones(point, coverage, se);
        twisted = c.p;
        weight = c.weight;
        coverage = c.coverage;
    }
    twisted = se_turbulence(twisted);
    let sa = sin(ang);
    let ca = cos(ang);
    twisted = vec3<f32>(ca * twisted.x - sa * twisted.z, twisted.y, sa * twisted.x + ca * twisted.z);
    twisted = twisted * se0.x + se[2].xyz;
    let p = twisted * se0.x * 6.37;
    // Octaves down to about a tenth of a pixel (frequency x pi each): SE bakes every octave into its
    // tiles, and with only the coarse ones left, thresholded Perlin lobes show as a checkerboard.
    let base = se0.x * se0.x * 6.37;
    let oct = clamp(floor(log2(0.5 / max(base * fp, 1e-30)) / log2(3.1416)) + 3.0, 1.0, se0.y);
    let q = p + se_fbm_clouds3(p, oct);
    let r = p + se_fbm_clouds3(q, oct);
    let f = se_fbm_clouds(r, oct) * 0.7 + coverage - 0.3;
    let h = saturate(f) * weight;
    return saturate(h * h * 3.5 + h);
}

// Cloud cover of a procedural planet at direction `d` in the layer's frame (which turns with the
// layer's wind). `fwd`: the pixel's footprint on the unit sphere (`length(fwidth(d))`); `proc` and
// `proc2` as the layer material's (terrain.rs `CloudUniform`).
fn proc_clouds(d: vec3<f32>, fwd: f32, proc: vec4<f32>, proc2: vec4<f32>) -> f32 {
    let p = d;
    let freq = 7.0 * max(proc.y, 0.1);
    // Weather systems are stretched along the parallels.
    let ps = vec3<f32>(p.x, p.y * 1.7, p.z);
    let seed = vec3<f32>(proc2.x * 71.0, proc2.x * 13.0, proc2.x * 37.0);
    let fw = fwd * freq;
    let warp = vec3<f32>(
        fbm_aa(ps * freq * 0.5 + seed, 4, fw * 0.5),
        fbm_aa(ps * freq * 0.5 + seed + vec3<f32>(5.2), 4, fw * 0.5),
        fbm_aa(ps * freq * 0.5 + seed + vec3<f32>(9.1), 4, fw * 0.5)) - vec3<f32>(0.5);
    let n = fbm_aa(ps * freq + warp * 2.2 + seed, i32(clamp(proc.z + 2.0, 4.0, 9.0)), fw);
    // Cloudier near the equator and at mid latitudes, clearer in the subtropics.
    let belts = 1.0 + proc2.z * (0.5 * cos(d.y * 9.42) - 0.1);
    let cov = clamp(proc.x * belts, 0.0, 1.0);
    let t = 0.5 + (0.5 - cov) * 0.45;
    let cover = smoothstep(t - 0.03, t + 0.1, n) * (0.55 + 0.45 * smoothstep(t, t + 0.2, n));
    // Full coverage closes the layer (Titan's haze, Coverage 1).
    return mix(cover, 1.0, smoothstep(0.9, 1.0, proc.x));
}
