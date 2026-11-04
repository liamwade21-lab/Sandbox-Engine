// SE's gas giant clouds, shared by the ground (planet.wesl), the brown dwarfs (brown_dwarf.wesl) and
// the volumetric clouds' giant decks (volumetric_clouds.wesl): the noise primitives and the cloud
// height and colour, from a giant's parameters (`surface_tex::BiomeUniform::giant`).

const PI: f32 = 3.14159265359;

// The giant's preset: CloudsPalette (gamma encoded) and the parameters (`BiomeUniform::giant`).
struct GiantParams {
    palette: array<vec4<f32>, 8>,
    giant: array<vec4<f32>, 5>,
};

// ---------------------------------------------------------------------------
// SE's gas giant clouds (tg_common.glh HeightMapCloudsGasGiant, TurbulenceGasGiant,
// CycloneNoiseGasGiant; tg_gasgiant_color.glsl), with SE's hash Perlin noise, Fbm (lacunarity
// 2.218, H 0.5) and spherical Fibonacci cells as in tg_height.wesl. The point is the unit
// direction in the body frame (y the pole).

fn gg_hash(gridcell_in: vec3<f32>) -> array<vec4<f32>, 6> {
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

fn gg_noise(p: vec3<f32>) -> f32 {
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf1 = Pf - 1.0;
    let h = gg_hash(Pi);
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

fn gg_fbm(point: vec3<f32>, octaves: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    let gain = pow(2.218281828459, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        sum += gg_noise(pt) * ampl;
        ampl *= gain;
        pt *= 2.218281828459;
    }
    return sum;
}

fn gg_hash1(p: f32) -> f32 {
    return fract(sin(p) * 158.5453123);
}

fn gg_rotate(angle: f32, axis: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
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
fn gg_isf(p: vec3<f32>, n: f32, nearest: ptr<function, vec3<f32>>) -> vec2<f32> {
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

fn gg_turbulence(point_in: vec3<f32>, g: GiantParams) -> vec3<f32> {
    let twist = g.giant[0].w;
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    var strength = 5.5;
    var freq = 800.0 * 0.7;
    var size = 15.0 * 0.7;
    let dens = 0.8;
    for (var i = 0; i < 5; i++) {
        let cell = gg_isf(point, freq, &center);
        let rnd = gg_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.5 * dens - rnd);
            let dist = clamp(1.0 - r, 0.0, 1.0);
            let dist2 = clamp(0.5 - r, 0.0, 1.0);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.25);
            twisted = gg_rotate(dir * twist * sign(center.y) * fi, center, point);
        }
        freq = min(freq * 2.0, 1600.0);
        size = min(size * 1.2, 30.0);
        strength = strength * 1.5;
        point = twisted;
    }
    return twisted;
}

fn gg_cyclones(point_in: vec3<f32>, offset: ptr<function, f32>, g: GiantParams) -> vec3<f32> {
    let cy = g.giant[2];
    let cy2 = g.giant[3];
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    // Equatorial oval cyclones.
    var offs = 0.6;
    var squeeze = 2.27;
    var strength = 2.5;
    var freq = cy.y * 50.0;
    var dens = cy.z * 0.02;
    var size = 5.0;
    for (var i = 0; f32(i) < cy.w; i++) {
        let cell = gg_isf(vec3<f32>(point.x, point.y * squeeze, point.z), freq, &center);
        let rnd = gg_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.7 * dens - rnd);
            let dist = clamp(1.0 - r, 0.0, 1.0);
            let dist2 = clamp(0.5 - r, 0.0, 1.0);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.5);
            twisted = gg_rotate(cy.x * dir * sign(center.y + 0.001) * fi, center, point);
            *offset += offs * fi * dir;
        }
        freq = min(freq * 2.0, 6400.0);
        dens = min(dens * 3.5, 0.3);
        size = min(size * 1.5, 15.0);
        offs = offs * 0.85;
        squeeze = max(squeeze - 0.3, 1.0);
        strength *= 1.3;
        point = twisted;
    }
    // Polar round cyclones.
    strength = 7.125;
    freq = cy2.y * 800.0;
    dens = 0.5;
    size = 8.0;
    offs = cy2.x * smoothstep(cy2.z - 0.1, cy2.z + 0.1, abs(point.y));
    for (var i = 0; f32(i) < cy2.w; i++) {
        let cell = gg_isf(point, freq, &center);
        let rnd = gg_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.7 * dens - rnd);
            let dist = clamp(1.0 - r, 0.0, 1.0);
            let dist2 = clamp(0.5 - r, 0.0, 1.0);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.5);
            twisted = gg_rotate(cy.x * dir * sign(center.y + 0.001) * fi, center, point);
            *offset += offs * fi * dir;
        }
        freq = min(freq * 1.8, 6400.0);
        size = min(size * 1.2, 15.0);
        offs = offs * 0.95;
        strength *= 1.3;
        point = twisted;
    }
    return twisted;
}

fn gg_height(point: vec3<f32>, g: GiantParams) -> f32 {
    let g0 = g.giant[0];
    let randomize = g.giant[4].xyz;
    var twisted = point;
    // Zones.
    var zp = randomize * 7.1563;
    zp.y += twisted.y * g0.z * 0.5;
    let zones = gg_noise(zp) * 0.6 + 0.25;
    var offset = 0.0;
    if g.giant[2].w > 0.0 {
        twisted = gg_cyclones(twisted, &offset, g);
    }
    twisted = gg_turbulence(twisted, g);
    // Stripes.
    let turb = gg_fbm(twisted * 0.2, g0.y);
    twisted = twisted * (0.00515 * g0.x) - turb * 0.0005;
    twisted.y *= 100.0 + turb;
    let height = g.giant[1].x * (gg_fbm(twisted, g0.y) * 0.7 + 0.5);
    return zones + height + offset;
}

// tg_gasgiant_color: the cloud height times the palette's colour at that height (SE's
// GetGasGiantCloudsColor over its 8 layers), then from gamma to linear (`colorGamma`).
fn gas_giant_albedo(dir: vec3<f32>, g: GiantParams) -> vec3<f32> {
    return gas_giant_cloud(dir, g).rgb;
}

// The colour (rgb, linear) and the cloud height (a) at dir.
fn gas_giant_cloud(dir: vec3<f32>, g: GiantParams) -> vec4<f32> {
    let h = gg_height(dir, g);
    let hs = 7.0 / 21.0;
    let y = clamp(h * hs, 0.5 * hs, 1.0 - 0.5 * hs);
    let f = (y - 0.5 * hs) / (1.0 - hs) * 7.0;
    let i0 = u32(clamp(floor(f), 0.0, 7.0));
    let i1 = min(i0 + 1u, 7u);
    let c = mix(g.palette[i0].rgb, g.palette[i1].rgb, fract(f));
    return vec4<f32>(pow(clamp(h * c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2)), h);
}
