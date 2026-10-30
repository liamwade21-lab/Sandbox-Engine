// Star photosphere: SE's GlowMapSun (tg_common.glh 3203-3240, with SolarSpotsTempNoise 2803-2850)
// evaluated per pixel, where SE renders it into a temperature texture, then SE's thermal colour
// (planet.glsl THERM 1285-1310): the temperature times 0.86 at the limb to 1.1 at the centre,
// coloured as a blackbody. The noise is SE's (hash Perlin and its 64^3 noise texture, translated as
// in tg_height.wesl). Detail smaller than a pixel fades to its mean, as SE's coarser tiles average
// it.

import bevy_pbr::render::mesh_functions::get_world_from_local;
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct StarSurface {
    // rgb the star's colour at its Teff, w HDR intensity
    color: vec4<f32>,
    // x Teff (K), y SE's limb darkening amount (SurfParams1.y), z SE's spotTemp (relative), w unused
    temp: vec4<f32>,
    // xyz SE's Randomize
    randomize: vec4<f32>,
    // x sunFlowDistFreq, y sunGranMagn (scales the flows), z sunGranFreq, w sunGranOctaves
    gran: vec4<f32>,
    // x sunGranTempDistort, y sunGranBotTemp, z sunGranTopTemp, w unused
    gran2: vec4<f32>,
    // x sunSpotSqrtDensity, y sunSpotFreq, z sunSpotFilamentFreq, w unused
    spots: vec4<f32>,
    // x sunSpotMaxLat, y sunSpotMinLat
    spots2: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> star: StarSurface;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var noise3d: texture_3d<f32>;

const NOISE_TEX_3D_SIZE: f32 = 64.0;
const PI2: f32 = 6.283185307179586;
// Mean of SE's granule value smoothstep(0.1, 1, sqrt(F2 - F1)) over the surface.
const GRAN_MEAN: f32 = 0.3;

var<private> noiseOctaves: f32 = 4.0;
var<private> noiseLacunarity: f32 = 2.218281828459;
var<private> noiseH: f32 = 0.5;

fn glsl_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn NoiseNearestUVec4(c: vec3<f32>) -> vec4<f32> {
    let w = c - NOISE_TEX_3D_SIZE * floor(c / NOISE_TEX_3D_SIZE);
    return textureLoad(noise3d, vec3<i32>(w), 0);
}

// Brian Sharpe hash Perlin noise (tg_common.glh 890-1108, as tg_height.wesl).
struct FastHash3DOut {
    lowz_hash_0: vec4<f32>,
    lowz_hash_1: vec4<f32>,
    lowz_hash_2: vec4<f32>,
    highz_hash_0: vec4<f32>,
    highz_hash_1: vec4<f32>,
    highz_hash_2: vec4<f32>,
}

fn FastHash3D(gridcell_in: vec3<f32>) -> FastHash3DOut {
    const OFFSET = vec2<f32>(50.0, 161.0);
    const DOMAIN: f32 = 69.0;
    const SOMELARGEFLOATS = vec3<f32>(635.298681, 682.357502, 668.926525);
    const ZINC = vec3<f32>(48.500388, 65.294118, 63.934599);
    var gridcell = gridcell_in;
    gridcell = gridcell - floor(gridcell * (1.0 / DOMAIN)) * DOMAIN;
    let gridcell_inc1 = select(gridcell + vec3<f32>(1.0), vec3<f32>(0.0), gridcell > vec3<f32>(DOMAIN - 1.5));
    var P = vec4<f32>(gridcell.xy, gridcell_inc1.xy) + OFFSET.xyxy;
    P *= P;
    P = P.xzxz * P.yyww;
    let lowz_hash_2 = vec4<f32>(1.0) / (SOMELARGEFLOATS.xyzx + vec2<f32>(gridcell.z, gridcell_inc1.z).xxxy * ZINC.xyzx);
    let highz_hash_2_xy = vec2<f32>(1.0) / (SOMELARGEFLOATS.yz + gridcell_inc1.zz * ZINC.yz);
    var o: FastHash3DOut;
    o.lowz_hash_0 = fract(P * lowz_hash_2.xxxx);
    o.highz_hash_0 = fract(P * lowz_hash_2.wwww);
    o.lowz_hash_1 = fract(P * lowz_hash_2.yyyy);
    o.highz_hash_1 = fract(P * highz_hash_2_xy.xxxx);
    o.lowz_hash_2 = fract(P * lowz_hash_2.zzzz);
    o.highz_hash_2 = fract(P * highz_hash_2_xy.yyyy);
    return o;
}

fn InterpC2(x: vec3<f32>) -> vec3<f32> { return x * x * x * (x * (x * 6.0 - 15.0) + 10.0); }

fn Noise(p: vec3<f32>) -> f32 {
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf_min1 = Pf - 1.0;
    let hsh = FastHash3D(Pi);
    let C = vec4<f32>(0.49999);
    let grad_x0 = hsh.lowz_hash_0 - C;
    let grad_y0 = hsh.lowz_hash_1 - C;
    let grad_z0 = hsh.lowz_hash_2 - C;
    let grad_x1 = hsh.highz_hash_0 - C;
    let grad_y1 = hsh.highz_hash_1 - C;
    let grad_z1 = hsh.highz_hash_2 - C;
    let grad_results_0 = inverseSqrt(grad_x0 * grad_x0 + grad_y0 * grad_y0 + grad_z0 * grad_z0) * (vec2<f32>(Pf.x, Pf_min1.x).xyxy * grad_x0 + vec2<f32>(Pf.y, Pf_min1.y).xxyy * grad_y0 + Pf.zzzz * grad_z0);
    let grad_results_1 = inverseSqrt(grad_x1 * grad_x1 + grad_y1 * grad_y1 + grad_z1 * grad_z1) * (vec2<f32>(Pf.x, Pf_min1.x).xyxy * grad_x1 + vec2<f32>(Pf.y, Pf_min1.y).xxyy * grad_y1 + Pf_min1.zzzz * grad_z1);
    let blend = InterpC2(Pf);
    let res0 = mix(grad_results_0, grad_results_1, blend.z);
    let res1 = mix(res0.xy, res0.zw, blend.y);
    return mix(res1.x, res1.y, blend.x) * 1.1547005383792515290182975610039;
}

fn NoiseU(p: vec3<f32>) -> f32 { return Noise(p) * 0.5 + 0.5; }

const vyd = vec3<f32>(3.33, 5.71, 1.96);
const vzd = vec3<f32>(7.77, 2.65, 4.37);

fn NoiseVec3(p: vec3<f32>) -> vec3<f32> { return vec3<f32>(Noise(p), Noise(p + vyd), Noise(p + vzd)); }

fn Fbm(point: vec3<f32>) -> f32 {
    var pt = point;
    var summ: f32 = 0.0;
    var ampl: f32 = 1.0;
    let gain = pow(noiseLacunarity, -noiseH);
    for (var i: i32 = 0; f32(i) < noiseOctaves; i++) {
        summ += Noise(pt) * ampl;
        ampl *= gain;
        pt *= noiseLacunarity;
    }
    return summ;
}

fn Fbm3D(point: vec3<f32>) -> vec3<f32> {
    var pt = point;
    var summ = vec3<f32>(0.0);
    var ampl: f32 = 1.0;
    let gain = pow(noiseLacunarity, -noiseH);
    for (var i: i32 = 0; f32(i) < noiseOctaves; i++) {
        summ += NoiseVec3(pt) * ampl;
        ampl *= gain;
        pt *= noiseLacunarity;
    }
    return summ;
}

// The octaves (of at most `n`) whose noise at base frequency `freq` stays wider than two pixels of
// footprint `fp`; the finer ones average to 0.
fn octaves(n: f32, freq: f32, fp: f32) -> f32 {
    let k = log2(0.5 / max(freq * fp, 1e-30)) / log2(noiseLacunarity) + 1.0;
    return clamp(floor(k), 0.0, n);
}

fn Cell3Noise2(p: vec3<f32>) -> vec2<f32> {
    let cell = floor(p);
    let offs = p - cell;
    var distMin1: f32 = 1.0e38;
    var distMin2: f32 = 1.0e38;
    for (var dz: f32 = -1.0; dz < 2.0; dz += 1.0) {
        for (var dy: f32 = -1.0; dy < 2.0; dy += 1.0) {
            for (var dx: f32 = -1.0; dx < 2.0; dx += 1.0) {
                let d = vec3<f32>(dx, dy, dz);
                let pos = NoiseNearestUVec4(cell + d).xyz + d - offs;
                let dist = dot(pos, pos);
                if (dist < distMin1) {
                    distMin2 = distMin1;
                    distMin1 = dist;
                } else {
                    distMin2 = min(distMin2, dist);
                }
            }
        }
    }
    return sqrt(vec2<f32>(distMin1, distMin2));
}

// tg_common.glh 1710-1738: the nearest cell point on the sphere and the distance to it (in cells).
fn Cell2NoiseSphere(p_in: vec3<f32>, Radius: f32) -> vec4<f32> {
    let p = p_in * Radius;
    let cell = floor(p);
    let offs = p - cell - vec3<f32>(0.5);
    var point = vec3<f32>(0.0);
    var distMin: f32 = 1.0e38;
    for (var dz: f32 = -1.0; dz < 1.0; dz += 1.0) {
        for (var dy: f32 = -1.0; dy < 1.0; dy += 1.0) {
            for (var dx: f32 = -1.0; dx < 1.0; dx += 1.0) {
                let d = vec3<f32>(dx, dy, dz);
                let rnd = NoiseNearestUVec4(cell + d).xyz + d;
                let pos = rnd - offs;
                let dist = dot(pos, pos);
                if (distMin > dist) {
                    distMin = dist;
                    point = rnd;
                }
            }
        }
    }
    point = normalize(point + cell + vec3<f32>(0.5));
    return vec4<f32>(point, length(point * Radius - p));
}

struct Spots {
    botMask: f32,
    filMask: f32,
    filaments: f32,
}

// SolarSpotsTempNoise: umbrae (botMask 0) inside penumbrae of radial filaments, in a latitude band.
// Spots narrower than about a pixel fade out, filaments finer than one to their mean.
fn solar_spots_temp(point_in: vec3<f32>, fp: f32) -> Spots {
    var s: Spots;
    s.botMask = 1.0;
    s.filMask = 1.0;
    s.filaments = 0.0;
    let sqrt_density = star.spots.x;
    let craterSphereRadius = star.spots.y * sqrt_density;
    // A spot's radius on the unit sphere, in pixels.
    let spot_px = 0.5 / max(star.spots.y * fp, 1e-30);
    let vis = smoothstep(0.3, 1.0, spot_px);
    if vis <= 0.0 {
        s.filMask = 0.0;
        return s;
    }
    var point = point_in;
    let binormal = normalize(vec3<f32>(-point.z, 0.0, point.x));
    let radFactor = 2.0 / sqrt_density;
    let radInner = 0.4;
    let radOuter = 1.0;
    let dist = 0.01 * Fbm3D(point * 7.6);
    point += dist * 0.5;

    let cell = Cell2NoiseSphere(point, craterSphereRadius);
    let fi = acos(clamp(dot(binormal, normalize(cell.xyz - point)), -1.0, 1.0)) / PI2;
    let rnd = 48.3 * dot(cell.xyz, star.randomize.xyz);
    let t = saturate((cell.w * radFactor - radInner) / (radOuter - radInner));
    let botmask = smoothstep(0.0, 0.2, t);
    let filmask = (1.0 - smoothstep(0.7, 1.0, t)) * smoothstep(0.0, 0.1, t) * 0.85;
    var filam = NoiseU(vec3<f32>(star.spots.z * fi, rnd, rnd));
    // Filament noise cycles per pixel at this distance from the spot centre.
    let cycles = star.spots.z * fp * craterSphereRadius / (3.14159265 * max(cell.w, 1e-6));
    filam = mix(0.5, filam, 1.0 - smoothstep(0.3, 0.6, cycles)) * t * 0.75;
    s.filMask *= filmask;
    s.botMask *= botmask;
    s.filaments += filam;

    let lat = abs(point.y);
    let polFade = glsl_smoothstep(star.spots2.x + 0.1, star.spots2.x - 0.1, lat);
    let eqFade = glsl_smoothstep(star.spots2.y - 0.1, star.spots2.y + 0.1, lat);
    let fade = polFade * eqFade * vis;
    s.botMask = mix(1.0, s.botMask, fade);
    s.filMask *= fade;
    s.filaments *= fade;
    return s;
}

// GlowMapSun: the temperature relative to the star's (surfTemperature).
fn glow_map_sun(point: vec3<f32>, fp: f32) -> f32 {
    let rnd = star.randomize.xyz;
    let bot = star.gran2.y;
    let top = star.gran2.z;
    // Flows (giants; a constant offset on dwarfs, whose flow frequency is 0)
    noiseOctaves = octaves(5.0, star.gran.x * 0.5, fp);
    var p = point * star.gran.x + rnd;
    var dist = 2.5 * Fbm3D(p * 0.5);
    noiseOctaves = octaves(3.0, star.gran.x * 7.5, fp);
    let flows = star.gran.y * Fbm(p * 7.5 + dist);

    // Granulation
    noiseLacunarity = 2.218281828459;
    noiseH = 0.5;
    var gran = GRAN_MEAN;
    let gran_w = 1.0 - smoothstep(0.25, 0.5, star.gran.z * fp);
    if gran_w > 0.0 {
        noiseOctaves = octaves(star.gran.w, star.gran.z * 0.2, fp);
        p = point * star.gran.z + rnd;
        dist = star.gran2.x * Fbm3D(p * 0.2);
        let cell = Cell3Noise2(p + dist);
        gran = mix(GRAN_MEAN, smoothstep(0.1, 1.0, sqrt(abs(cell.y - cell.x))), gran_w);
    }

    // Solar spots
    var spots: Spots;
    spots.botMask = 1.0;
    spots.filMask = 0.0;
    spots.filaments = 0.0;
    if star.spots.x > 0.01 {
        noiseOctaves = 5.0;
        spots = solar_spots_temp(point, fp);
    }

    let spotTemp = star.temp.z;
    let surfTemp = 1.0;
    let filTemp = top;
    return (flows + mix(bot, top, gran) * (1.0 - spots.filMask)) * mix(spotTemp, surfTemp, spots.botMask)
        + spots.filMask * mix(spotTemp, filTemp, spots.filaments);
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // The point on the unit sphere in the star's frame (y along the spin axis), for its surface.
    @location(2) local: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let wp = (m * vec4<f32>(v.position, 1.0)).xyz;
    var o: VertexOut;
    o.clip = position_world_to_clip(wp);
    o.world_position = wp;
    // The mesh is a unit sphere, so its local position is the normal. Rotating it by the model
    // matrix avoids the inverse transpose, whose determinant overflows f32 (black disc) once a
    // star is wider than about 47 AU.
    o.normal = (m * vec4<f32>(v.position, 0.0)).xyz;
    o.local = v.position;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let v = normalize(-in.world_position.xyz);
    let mu = clamp(dot(n, v), 0.0, 1.0);
    let point = normalize(in.local);
    // The pixel's footprint on the unit sphere.
    let fp = max(length(fwidth(point)), 1e-9);

    let ts = star.temp.x;
    var tk = glow_map_sun(point, fp) * ts;
    // SE's limb darkening: the apparent temperature 1.1x at the centre, 0.86x at the limb.
    tk *= mix(0.86, 1.1, mix(1.0, mu, star.temp.y));
    tk = max(tk, 100.0);
    // A blackbody at tk at 610 / 550 / 465 nm (SE samples its Planck texture), relative to the disc
    // centre's mean temperature, where the star's colour and the port's photosphere brightness
    // apply (SE's Planck table scale times hdrStarSurfaceBright is not decoded).
    let tc = ts * 1.1 * mix(star.gran2.y, star.gran2.z, GRAN_MEAN);
    let c = vec3<f32>(23587.0, 26160.0, 30942.0);
    let ratio = exp(c * (1.0 / tc - 1.0 / tk)) * (1.0 - exp(-c / tc)) / (1.0 - exp(-c / tk));
    let col = star.color.rgb * star.color.w * ratio;
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
