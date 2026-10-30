// SpaceEngine 0.991 procedural terrain HEIGHT generator, translated 1:1 from
// re/se_shaders/tg_common.glh + tg_terra_height.glsl + tg_selena_height.glsl.
// Compute version: one invocation per sample of an N x N tile grid.
//
// Translation rules followed throughout:
//  - same expression text / operation order as the GLSL (no algebraic rewrites);
//  - GLSL mod(x, y) -> glsl_mod*(x, y) = x - y * floor(x / y);
//  - `for (int i = 0; i < floatVar; ++i)` -> i32 counter compared as f32(i) < floatVar;
//  - float loop counters (cell noises) stay f32;
//  - mutable GLSL globals -> var<private> (re-initialised per invocation, like per fragment);
//  - #if/#ifdef -> the branch SE compiles (IMPROVED_TEX_PERLIN is NOT defined: hash Perlin);
//  - texture(Noise3DSampler, (cell + d) / 64.0) (NEAREST, REPEAT) -> textureLoad at
//    the floored-mod-64 integer cell;
//  - smoothstep with reversed edges (edge0 > edge1, undefined in WGSL) -> glsl_smoothstep,
//    which is the GLSL spec formula.
// Colour / glow / cloud / biome-palette code is not translated.

// ----------------------------------------------------------------------------
// Uniforms

struct TgParams {
    Randomize:      vec4<f32>, // (Randomize.x,      Randomize.y,      Randomize.z,         unused)
    mainParams:     vec4<f32>, // (mainFreq,         terraceProb,      oceanType,           tidalLock)
    colorParams:    vec4<f32>, // (colorDistMagn,    colorDistFreq,    latIceCaps,          latTropic)
    climateParams:  vec4<f32>, // (climatePole,      climateTropic,    climateEquator,      tropicWidth)
    mareParams:     vec4<f32>, // (seaLevel,         mareFreq,         sqrt(mareDensity),   icecapHeight)
    montesParams:   vec4<f32>, // (montesMagn,       montesFreq,       montesFraction,      montesSpiky)
    dunesParams:    vec4<f32>, // (dunesMagn,        dunesFreq,        dunesFraction,       drivenDarkening)
    hillsParams:    vec4<f32>, // (hillsMagn,        hillsFreq,        hillsFraction,       hills2Fraction)
    canyonsParams:  vec4<f32>, // (canyonsMagn,      canyonsFreq,      canyonsFraction,     erosion)
    riversParams:   vec4<f32>, // (riversMagn,       riversFreq,       riversSin,           beachWidth)
    riftsParams:    vec4<f32>, // (riftsMagn,        riftsFreq,        riftsSin,            0.0)
    eqridgeParams:  vec4<f32>, // (eqridgeMagn,      eqridgeWidth,     eqridgeModMagn,      eqridgeModFreq)
    cracksParams:   vec4<f32>, // (cracksMagn,       cracksFreq,       cracksOctaves,       craterRayedFactor)
    craterParams:   vec4<f32>, // (craterMagn,       craterFreq,       sqrt(craterDensity), craterOctaves)
    volcanoParams1: vec4<f32>, // (volcanoMagn,      volcanoFreq,      volcanoDensity,      volcanoOctaves)
    volcanoParams2: vec4<f32>, // (volcanoActivity,  volcanoFlows,     volcanoRadius,       volcanoTemp)
    textureParams:  vec4<f32>, // (texScale,         0.0,              venusMagn,           venusFreq)
    // Tile: sample (i, j) of an N x N grid is at the unit point
    // normalize(grid_origin.xyz + i * grid_du.xyz + j * grid_dv.xyz) (SE's body frame, f32).
    grid_origin:    vec4<f32>, // w: N
    grid_du:        vec4<f32>, // w: unused
    grid_dv:        vec4<f32>, // w: generator, 0 = HeightMapTerra, 1 = HeightMapSelena
}

@group(0) @binding(0) var<storage, read_write> heights: array<f32>;   // heights[j * N + i]
@group(0) @binding(1) var<uniform> tg: TgParams;
@group(0) @binding(2) var noise3d: texture_3d<f32>;                   // SE's Noise3DSampler, 64^3 RGBA

// SE's `#define name xxxParams.c` table (tg_common.glh 150-223)
fn Randomize() -> vec3<f32>       { return tg.Randomize.xyz; }
fn mainFreq() -> f32              { return tg.mainParams.x; }
fn terraceProb() -> f32           { return tg.mainParams.y; }
fn oceanType() -> f32             { return tg.mainParams.z; }
fn tidalLock() -> f32             { return tg.mainParams.w; }
fn colorDistMagn() -> f32         { return tg.colorParams.x; }
fn colorDistFreq() -> f32         { return tg.colorParams.y; }
fn latIceCaps() -> f32            { return tg.colorParams.z; }
fn latTropic() -> f32             { return tg.colorParams.w; }
fn climatePole() -> f32           { return tg.climateParams.x; }
fn climateTropic() -> f32         { return tg.climateParams.y; }
fn climateEquator() -> f32        { return tg.climateParams.z; }
fn tropicWidth() -> f32           { return tg.climateParams.w; }
fn seaLevel() -> f32              { return tg.mareParams.x; }
fn mareFreq() -> f32              { return tg.mareParams.y; }
fn mareSqrtDensity() -> f32       { return tg.mareParams.z; }
fn icecapHeight() -> f32          { return tg.mareParams.w; }
fn montesMagn() -> f32            { return tg.montesParams.x; }
fn montesFreq() -> f32            { return tg.montesParams.y; }
fn montesFraction() -> f32        { return tg.montesParams.z; }
fn montesSpiky() -> f32           { return tg.montesParams.w; }
fn dunesMagn() -> f32             { return tg.dunesParams.x; }
fn dunesFreq() -> f32             { return tg.dunesParams.y; }
fn dunesFraction() -> f32         { return tg.dunesParams.z; }
fn drivenDarkening() -> f32       { return tg.dunesParams.w; }
fn hillsMagn() -> f32             { return tg.hillsParams.x; }
fn hillsFreq() -> f32             { return tg.hillsParams.y; }
fn hillsFraction() -> f32         { return tg.hillsParams.z; }
fn hills2Fraction() -> f32        { return tg.hillsParams.w; }
fn canyonsMagn() -> f32           { return tg.canyonsParams.x; }
fn canyonsFreq() -> f32           { return tg.canyonsParams.y; }
fn canyonsFraction() -> f32       { return tg.canyonsParams.z; }
fn erosion() -> f32               { return tg.canyonsParams.w; }
fn riversMagn() -> f32            { return tg.riversParams.x; }
fn riversFreq() -> f32            { return tg.riversParams.y; }
fn riversSin() -> f32             { return tg.riversParams.z; }
fn beachWidth() -> f32            { return tg.riversParams.w; }
fn riftsMagn() -> f32             { return tg.riftsParams.x; }
fn riftsFreq() -> f32             { return tg.riftsParams.y; }
fn riftsSin() -> f32              { return tg.riftsParams.z; }
fn eqridgeMagn() -> f32           { return tg.eqridgeParams.x; }
fn eqridgeWidth() -> f32          { return tg.eqridgeParams.y; }
fn eqridgeModMagn() -> f32        { return tg.eqridgeParams.z; }
fn eqridgeModFreq() -> f32        { return tg.eqridgeParams.w; }
fn cracksMagn() -> f32            { return tg.cracksParams.x; }
fn cracksFreq() -> f32            { return tg.cracksParams.y; }
fn cracksOctaves() -> f32         { return tg.cracksParams.z; }
fn craterRayedFactor() -> f32     { return tg.cracksParams.w; }
fn craterMagn() -> f32            { return tg.craterParams.x; }
fn craterFreq() -> f32            { return tg.craterParams.y; }
fn craterSqrtDensity() -> f32     { return tg.craterParams.z; }
fn craterOctaves() -> f32         { return tg.craterParams.w; }
fn volcanoMagn() -> f32           { return tg.volcanoParams1.x; }
fn volcanoFreq() -> f32           { return tg.volcanoParams1.y; }
fn volcanoDensity() -> f32        { return tg.volcanoParams1.z; }
fn volcanoOctaves() -> f32        { return tg.volcanoParams1.w; }
fn volcanoActivity() -> f32       { return tg.volcanoParams2.x; }
fn volcanoFlows() -> f32          { return tg.volcanoParams2.y; }
fn volcanoRadius() -> f32         { return tg.volcanoParams2.z; }
fn volcanoTemp() -> f32           { return tg.volcanoParams2.w; }
fn texScale() -> f32              { return tg.textureParams.x; }
fn venusMagn() -> f32             { return tg.textureParams.z; }
fn venusFreq() -> f32             { return tg.textureParams.w; }

// ----------------------------------------------------------------------------
// Constants (tg_common.glh 142-147, 2171-2176)

const pi: f32  = 3.14159265358;
const pi2: f32 = pi * 2.0;
const NOISE_TEX_3D_SIZE: f32 = 64.0;

const craterFreqPower: f32      = 1.8;
const craterRadFactorPower: f32 = sqrt(craterFreqPower);
const craterRadiusPower: f32    = 0.7;
const craterAmplPower: f32      = 0.6;
const craterPeakPower: f32      = 0.5;
const craterFloorPower: f32     = 1.2;

// ----------------------------------------------------------------------------
// Mutable globals (tg_common.glh 1233-1237, 2178-2187)

var<private> noiseOctaves: f32     = 4.0;
var<private> noiseLacunarity: f32  = 2.218281828459;
var<private> noiseH: f32           = 0.5;
var<private> noiseOffset: f32      = 0.8;
var<private> noiseRidgeSmooth: f32 = 0.0001;

// uninitialised in GLSL; WGSL zero-initialises
var<private> radPeak: f32;
var<private> radInner: f32;
var<private> radRim: f32;
var<private> radOuter: f32;
var<private> heightFloor: f32;
var<private> heightPeak: f32;
var<private> heightRim: f32;
var<private> craterRoundDist: f32;
var<private> craterDistortion: f32;

// ----------------------------------------------------------------------------
// GLSL helpers

fn glsl_mod3(x: vec3<f32>, y: f32) -> vec3<f32> { return x - y * floor(x / y); }
fn glsl_mod4(x: vec4<f32>, y: f32) -> vec4<f32> { return x - y * floor(x / y); }

// GLSL spec smoothstep, valid for edge0 > edge1 too
fn glsl_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// texture(Noise3DSampler, c / NOISE_TEX_3D_SIZE) with NEAREST + REPEAT, c integer-valued.
// The floored mod by 64 is exact in f32 for any integer-valued c.
fn NoiseNearestUVec4(c: vec3<f32>) -> vec4<f32> {
    let w = c - NOISE_TEX_3D_SIZE * floor(c / NOISE_TEX_3D_SIZE);
    return textureLoad(noise3d, vec3<i32>(w), 0);
}

// ----------------------------------------------------------------------------
// tg_common.glh 299-311

fn softPolyMin(a: f32, b: f32, k: f32) -> f32 {
    let diff = b - a;
    let h = saturate(0.5 + 0.5 * diff / k);
    return b - h * (diff + k * (1.0 - h));
}

fn softPolyMax(a: f32, b: f32, k: f32) -> f32 {
    let diff = a - b;
    let h = saturate(0.5 + 0.5 * diff / k);
    return b + h * (diff + k * (1.0 - h));
}

// tg_common.glh 323
fn hash1(p: f32) -> f32 { return fract(sin(p) * 158.5453123); }

// tg_common.glh 328-346
fn Rotate(Angle: f32, Axis: vec3<f32>, Vector: vec3<f32>) -> vec3<f32> {
    let cosa = cos(Angle);
    let sina = sin(Angle);
    let t = 1.0 - cosa;

    let M = mat3x3<f32>(
        t * Axis.x * Axis.x + cosa,
        t * Axis.x * Axis.y - sina * Axis.z,
        t * Axis.x * Axis.z + sina * Axis.y,
        t * Axis.x * Axis.y + sina * Axis.z,
        t * Axis.y * Axis.y + cosa,
        t * Axis.y * Axis.z - sina * Axis.x,
        t * Axis.x * Axis.z - sina * Axis.y,
        t * Axis.y * Axis.z + sina * Axis.x,
        t * Axis.z * Axis.z + cosa);

    return Vector * M;
}

// ----------------------------------------------------------------------------
// Brian Sharpe hash Perlin noise (tg_common.glh 890-920, 947-983, 1047-1108)

struct FastHash3DOut {
    lowz_hash_0: vec4<f32>,
    lowz_hash_1: vec4<f32>,
    lowz_hash_2: vec4<f32>,
    highz_hash_0: vec4<f32>,
    highz_hash_1: vec4<f32>,
    highz_hash_2: vec4<f32>,
}

fn FastHash3D(gridcell_in: vec3<f32>) -> FastHash3DOut {
    // gridcell is assumed to be an integer coordinate
    const OFFSET = vec2<f32>(50.0, 161.0);
    const DOMAIN: f32 = 69.0;
    const SOMELARGEFLOATS = vec3<f32>(635.298681, 682.357502, 668.926525);
    const ZINC = vec3<f32>(48.500388, 65.294118, 63.934599);

    // truncate the domain
    var gridcell = gridcell_in;
    gridcell = gridcell - floor(gridcell * (1.0 / DOMAIN)) * DOMAIN;
    // mix(a, b, greaterThan(..)) -> select(a, b, ..)
    let gridcell_inc1 = select(gridcell + vec3<f32>(1.0), vec3<f32>(0.0), gridcell > vec3<f32>(DOMAIN - 1.5));

    // calculate the noise
    var P = vec4<f32>(gridcell.xy, gridcell_inc1.xy) + OFFSET.xyxy;
    P *= P;
    P = P.xzxz * P.yyww;
    let lowz_hash_2 = vec4<f32>(1.0) / (SOMELARGEFLOATS.xyzx + vec2<f32>(gridcell.z, gridcell_inc1.z).xxxy * ZINC.xyzx);
    let highz_hash_2_xy = vec2<f32>(1.0) / (SOMELARGEFLOATS.yz + gridcell_inc1.zz * ZINC.yz);

    var o: FastHash3DOut;
    o.lowz_hash_0  = fract(P * lowz_hash_2.xxxx);
    o.highz_hash_0 = fract(P * lowz_hash_2.wwww);
    o.lowz_hash_1  = fract(P * lowz_hash_2.yyyy);
    o.highz_hash_1 = fract(P * highz_hash_2_xy.xxxx);
    o.lowz_hash_2  = fract(P * lowz_hash_2.zzzz);
    o.highz_hash_2 = fract(P * highz_hash_2_xy.yyyy);
    return o;
}

fn InterpC2(x: vec3<f32>) -> vec3<f32> { return x * x * x * (x * (x * 6.0 - 15.0) + 10.0); }

// 3D Perlin noise without lookup textures
fn Noise(p: vec3<f32>) -> f32 {
    // Establish our grid cell and unit position
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf_min1 = Pf - 1.0;

    // Calculate the hash
    let hsh = FastHash3D(Pi);
    let hashx0 = hsh.lowz_hash_0;
    let hashy0 = hsh.lowz_hash_1;
    let hashz0 = hsh.lowz_hash_2;
    let hashx1 = hsh.highz_hash_0;
    let hashy1 = hsh.highz_hash_1;
    let hashz1 = hsh.highz_hash_2;

    // Calculate the gradients
    let C = vec4<f32>(0.49999);
    let grad_x0 = hashx0 - C;
    let grad_y0 = hashy0 - C;
    let grad_z0 = hashz0 - C;
    let grad_x1 = hashx1 - C;
    let grad_y1 = hashy1 - C;
    let grad_z1 = hashz1 - C;
    let grad_results_0 = inverseSqrt(grad_x0 * grad_x0 + grad_y0 * grad_y0 + grad_z0 * grad_z0) * (vec2<f32>(Pf.x, Pf_min1.x).xyxy * grad_x0 + vec2<f32>(Pf.y, Pf_min1.y).xxyy * grad_y0 + Pf.zzzz * grad_z0);
    let grad_results_1 = inverseSqrt(grad_x1 * grad_x1 + grad_y1 * grad_y1 + grad_z1 * grad_z1) * (vec2<f32>(Pf.x, Pf_min1.x).xyxy * grad_x1 + vec2<f32>(Pf.y, Pf_min1.y).xxyy * grad_y1 + Pf_min1.zzzz * grad_z1);

    // Classic Perlin Interpolation
    let blend = InterpC2(Pf);
    let res0 = mix(grad_results_0, grad_results_1, blend.z);
    let res1 = mix(res0.xy, res0.zw, blend.y);
    var final_ = mix(res1.x, res1.y, blend.x);
    final_ *= 1.1547005383792515290182975610039; // scale to strict -1.0->1.0 range: *= 1.0/sqrt(0.75)
    return final_;
}

// 3D Perlin noise with derivatives, returns vec4(xderiv, yderiv, zderiv, noise)
fn NoiseDeriv(p: vec3<f32>) -> vec4<f32> {
    // establish our grid cell and unit position
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf_min1 = Pf - 1.0;

    // calculate the hash
    let hsh = FastHash3D(Pi);
    let hashx0 = hsh.lowz_hash_0;
    let hashy0 = hsh.lowz_hash_1;
    let hashz0 = hsh.lowz_hash_2;
    let hashx1 = hsh.highz_hash_0;
    let hashy1 = hsh.highz_hash_1;
    let hashz1 = hsh.highz_hash_2;

    // calculate the gradients
    let C = vec4<f32>(0.49999);
    var grad_x0 = hashx0 - C;
    var grad_y0 = hashy0 - C;
    var grad_z0 = hashz0 - C;
    let norm_0 = inverseSqrt(grad_x0 * grad_x0 + grad_y0 * grad_y0 + grad_z0 * grad_z0);
    grad_x0 *= norm_0;
    grad_y0 *= norm_0;
    grad_z0 *= norm_0;
    var grad_x1 = hashx1 - C;
    var grad_y1 = hashy1 - C;
    var grad_z1 = hashz1 - C;
    let norm_1 = inverseSqrt(grad_x1 * grad_x1 + grad_y1 * grad_y1 + grad_z1 * grad_z1);
    grad_x1 *= norm_1;
    grad_y1 *= norm_1;
    grad_z1 *= norm_1;
    let grad_results_0 = vec2<f32>(Pf.x, Pf_min1.x).xyxy * grad_x0 + vec2<f32>(Pf.y, Pf_min1.y).xxyy * grad_y0 + Pf.zzzz * grad_z0;
    let grad_results_1 = vec2<f32>(Pf.x, Pf_min1.x).xyxy * grad_x1 + vec2<f32>(Pf.y, Pf_min1.y).xxyy * grad_y1 + Pf_min1.zzzz * grad_z1;

    // get lengths in the x+y plane
    let Pf_sq = Pf * Pf;
    let Pf_min1_sq = Pf_min1 * Pf_min1;
    let vecs_len_sq = vec2<f32>(Pf_sq.x, Pf_min1_sq.x).xyxy + vec2<f32>(Pf_sq.y, Pf_min1_sq.y).xxyy;

    // evaluate the surflet
    var m_0 = vecs_len_sq + Pf_sq.zzzz;
    m_0 = max(1.0 - m_0, vec4<f32>(0.0));
    let m2_0 = m_0 * m_0;
    let m3_0 = m_0 * m2_0;

    var m_1 = vecs_len_sq + Pf_min1_sq.zzzz;
    m_1 = max(1.0 - m_1, vec4<f32>(0.0));
    let m2_1 = m_1 * m_1;
    let m3_1 = m_1 * m2_1;

    // calculate the derivatives
    let temp_0 = -6.0 * m2_0 * grad_results_0;
    let xderiv_0 = dot(temp_0, vec2<f32>(Pf.x, Pf_min1.x).xyxy) + dot(m3_0, grad_x0);
    let yderiv_0 = dot(temp_0, vec2<f32>(Pf.y, Pf_min1.y).xxyy) + dot(m3_0, grad_y0);
    let zderiv_0 = dot(temp_0, Pf.zzzz) + dot(m3_0, grad_z0);

    let temp_1 = -6.0 * m2_1 * grad_results_1;
    let xderiv_1 = dot(temp_1, vec2<f32>(Pf.x, Pf_min1.x).xyxy) + dot(m3_1, grad_x1);
    let yderiv_1 = dot(temp_1, vec2<f32>(Pf.y, Pf_min1.y).xxyy) + dot(m3_1, grad_y1);
    let zderiv_1 = dot(temp_1, Pf_min1.zzzz) + dot(m3_1, grad_z1);

    const FINAL_NORMALIZATION: f32 = 2.3703703703703703703703703703704; // scales the final result to a strict (-1.0, 1.0) range
    return vec4<f32>(vec3<f32>(xderiv_0, yderiv_0, zderiv_0) + vec3<f32>(xderiv_1, yderiv_1, zderiv_1),
                     dot(m3_0, grad_results_0) + dot(m3_1, grad_results_1)) * FINAL_NORMALIZATION;
}

// ----------------------------------------------------------------------------
// 3D simplex noise (tg_common.glh 1114-1194)

fn permute(x: vec4<f32>) -> vec4<f32> { return glsl_mod4((x * 34.0 + 1.0) * x, 289.0); }
fn taylorInvSqrt(r: vec4<f32>) -> vec4<f32> { return 1.79284291400159 - 0.85373472095314 * r; }

fn sNoise(v_in: vec3<f32>) -> f32 {
    var v = v_in;
    v *= 0.25;

    let C = vec2<f32>(1.0f / 6.0f, 1.0f / 3.0f);
    let D = vec4<f32>(0.0, 0.5, 1.0, 2.0);

    // First corner
    var i = floor(v + dot(v, C.yyy));
    let x0 = v - i + dot(i, C.xxx);

    // Other corners
    let g = step(x0.yzx, x0.xyz);
    let l = 1.0 - g;
    let i1 = min(g.xyz, l.zxy);
    let i2 = max(g.xyz, l.zxy);

    let x1 = x0 - i1 + C.xxx;
    let x2 = x0 - i2 + C.yyy; // 2.0*C.x = 1/3 = C.y
    let x3 = x0 - D.yyy;      // -1.0+3.0*C.x = -0.5 = -D.y

    // Permutations
    i = glsl_mod3(i, 289.0);
    let p = permute(permute(permute(
              i.z + vec4<f32>(0.0, i1.z, i2.z, 1.0))
            + i.y + vec4<f32>(0.0, i1.y, i2.y, 1.0))
            + i.x + vec4<f32>(0.0, i1.x, i2.x, 1.0));

    // Gradients: 7x7 points over a square, mapped onto an octahedron.
    let n_: f32 = 0.142857142857; // 1.0/7.0
    let ns = n_ * D.wyz - D.xzx;

    let j = p - 49.0 * floor(p * ns.z * ns.z);  //  mod(p,7*7)

    let x_ = floor(j * ns.z);
    let y_ = floor(j - 7.0 * x_);    // mod(j,N)

    let x = x_ * ns.x + ns.yyyy;
    let y = y_ * ns.x + ns.yyyy;
    let h = 1.0 - abs(x) - abs(y);

    let b0 = vec4<f32>(x.xy, y.xy);
    let b1 = vec4<f32>(x.zw, y.zw);

    let s0 = floor(b0) * 2.0 + 1.0;
    let s1 = floor(b1) * 2.0 + 1.0;
    let sh = -step(h, vec4<f32>(0.0));

    let a0 = b0.xzyw + s0.xzyw * sh.xxyy;
    let a1 = b1.xzyw + s1.xzyw * sh.zzww;

    var p0 = vec3<f32>(a0.xy, h.x);
    var p1 = vec3<f32>(a0.zw, h.y);
    var p2 = vec3<f32>(a1.xy, h.z);
    var p3 = vec3<f32>(a1.zw, h.w);

    // Normalise gradients
    let norm = taylorInvSqrt(vec4<f32>(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
    p0 *= norm.x;
    p1 *= norm.y;
    p2 *= norm.z;
    p3 *= norm.w;

    // Mix final noise value
    var m = max(0.6 - vec4<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), vec4<f32>(0.0));
    m = m * m;
    return 42.0 * dot(m * m, vec4<f32>(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
}

// ----------------------------------------------------------------------------
// tg_common.glh 1198-1217

const vyd = vec3<f32>(3.33, 5.71, 1.96);
const vzd = vec3<f32>(7.77, 2.65, 4.37);
const vwd = vec3<f32>(1.13, 2.73, 6.37);

fn NoiseVec3(p: vec3<f32>) -> vec3<f32> { return vec3<f32>(Noise(p), Noise(p + vyd), Noise(p + vzd)); }

fn DistNoise(p: vec3<f32>, d: f32) -> f32 { return Noise(p + NoiseVec3(p + 0.5) * d); }
fn DistNoise3D(p: vec3<f32>, d: f32) -> vec3<f32> { return NoiseVec3(p + NoiseVec3(p + 0.5) * d); }

// ----------------------------------------------------------------------------
// Fractals (tg_common.glh 1240-1544)

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

fn DistFbm(point: vec3<f32>, dist: f32) -> f32 {
    var pt = point;
    var summ: f32 = 0.0;
    var ampl: f32 = 1.0;
    let gain = pow(noiseLacunarity, -noiseH);
    for (var i: i32 = 0; f32(i) < noiseOctaves; i++) {
        summ += DistNoise(pt, dist) * ampl;
        ampl *= gain;
        pt *= noiseLacunarity;
    }
    return summ;
}

fn RidgedMultifractal(point: vec3<f32>, gain: f32) -> f32 {
    var signal: f32 = 1.0;
    var summ: f32 = 0.0;
    var frequency: f32 = 1.0;
    var weight: f32;
    for (var i: i32 = 0; f32(i) < noiseOctaves; i++) {
        weight = saturate(signal * gain);
        signal = Noise(point * frequency);
        signal = noiseOffset - sqrt(noiseRidgeSmooth + signal * signal);
        signal *= signal * weight;
        summ += signal * pow(frequency, -noiseH);
        frequency *= noiseLacunarity;
    }
    return summ;
}

// Ridged multifractal with "procedural erosion" by Giliam de Carpentier
fn RidgedMultifractalEroded(point: vec3<f32>, gain: f32, warp: f32) -> f32 {
    var frequency: f32 = 1.0;
    var amplitude: f32 = 1.0;
    var summ: f32 = 0.0;
    var signal: f32 = 1.0;
    var weight: f32;
    var dsum = vec3<f32>(0.0);
    var noiseDeriv: vec4<f32>;
    for (var i: i32 = 0; f32(i) < noiseOctaves; i++) {
        noiseDeriv = NoiseDeriv((point + warp * dsum) * frequency);
        weight = saturate(signal * gain);
        signal = noiseOffset - sqrt(noiseRidgeSmooth + noiseDeriv.w * noiseDeriv.w);
        signal *= signal * weight;
        amplitude = pow(frequency, -noiseH);
        summ += signal * amplitude;
        frequency *= noiseLacunarity;
        dsum -= amplitude * noiseDeriv.xyz * noiseDeriv.w;
    }
    return summ;
}

fn RidgedMultifractalErodedDetail(point: vec3<f32>, gain: f32, warp: f32, firstOctaveValue: f32) -> f32 {
    var frequency: f32 = 1.0;
    var amplitude: f32 = 1.0;
    var summ: f32 = firstOctaveValue;
    var signal: f32 = firstOctaveValue;
    var weight: f32;
    var dsum = vec3<f32>(0.0);
    var noiseDeriv: vec4<f32>;
    for (var i: i32 = 0; f32(i) < noiseOctaves; i++) {
        noiseDeriv = NoiseDeriv((point + warp * dsum) * frequency);
        weight = saturate(signal * gain);
        signal = noiseOffset - sqrt(noiseRidgeSmooth + noiseDeriv.w * noiseDeriv.w);
        signal *= signal * weight;
        amplitude = pow(frequency, -noiseH);
        summ += signal * amplitude;
        frequency *= noiseLacunarity;
        dsum -= amplitude * noiseDeriv.xyz * noiseDeriv.w;
    }
    return summ;
}

// "Jordan turbulence" function by Giliam de Carpentier
fn JordanTurbulence(point: vec3<f32>,
                    gain0: f32, gain: f32,
                    warp0: f32, warp: f32,
                    damp0: f32, damp: f32,
                    dampScale: f32) -> f32 {
    var noiseDeriv = NoiseDeriv(point);
    var noiseDeriv2 = noiseDeriv * noiseDeriv.w;
    var summ: f32 = noiseDeriv2.w;
    var dsumWarp = warp0 * noiseDeriv2.xyz;
    var dsumDamp = damp0 * noiseDeriv2.xyz;

    var amp: f32 = gain0;
    var freq: f32 = noiseLacunarity;
    var dampedAmp: f32 = amp * gain;

    for (var i: i32 = 1; f32(i) < noiseOctaves; i++) {
        noiseDeriv = NoiseDeriv(point * freq + dsumWarp.xyz);
        noiseDeriv2 = noiseDeriv * noiseDeriv.w;
        summ += dampedAmp * noiseDeriv2.w;
        dsumWarp += warp * noiseDeriv2.xyz;
        dsumDamp += damp * noiseDeriv2.xyz;
        freq *= noiseLacunarity;
        amp *= gain;
        dampedAmp = amp * (1.0 - dampScale / (1.0 + dot(dsumDamp, dsumDamp)));
    }
    return summ;
}

// "iqTurbulence" function by Inigo Quilez (SE uses n.x as value, n.yz as derivs: kept)
fn iqTurbulence(point: vec3<f32>, gain: f32) -> f32 {
    var n: vec4<f32>;
    var summ: f32 = 0.5;
    var freq: f32 = 1.0;
    var amp: f32 = 1.0;
    var dsum = vec2<f32>(0.0, 0.0);
    for (var i: i32 = 0; f32(i) < noiseOctaves; i++) {
        n = NoiseDeriv(point * freq);
        dsum += n.yz;
        summ += amp * n.x / (1.0 + dot(dsum, dsum));
        freq *= noiseLacunarity;
        amp *= gain;
    }
    return summ;
}

// ----------------------------------------------------------------------------
// Cellular noises (tg_common.glh 1580-1639, 1822-1906, 1976-2009)

fn Cell2Noise(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    let offs = p - cell - vec3<f32>(0.5);
    var pos: vec3<f32>;
    var rnd: vec3<f32>;
    var dist: f32;
    var distMin: f32 = 1.0e38;
    for (var dz: f32 = -1.0; dz < 1.0; dz += 1.0) {
        for (var dy: f32 = -1.0; dy < 1.0; dy += 1.0) {
            for (var dx: f32 = -1.0; dx < 1.0; dx += 1.0) {
                let d = vec3<f32>(dx, dy, dz);
                rnd = NoiseNearestUVec4(cell + d).xyz + d;
                pos = rnd - offs;
                dist = dot(pos, pos);
                distMin = min(distMin, dist);
            }
        }
    }
    return sqrt(distMin);
}

fn Cell2Noise2(p: vec3<f32>) -> vec2<f32> {
    let cell = floor(p);
    let offs = p - cell - vec3<f32>(0.5);
    var pos: vec3<f32>;
    var rnd: vec3<f32>;
    var dist: f32;
    var distMin1: f32 = 1.0e38;
    var distMin2: f32 = 1.0e38;
    for (var dz: f32 = -1.0; dz < 1.0; dz += 1.0) {
        for (var dy: f32 = -1.0; dy < 1.0; dy += 1.0) {
            for (var dx: f32 = -1.0; dx < 1.0; dx += 1.0) {
                let d = vec3<f32>(dx, dy, dz);
                rnd = NoiseNearestUVec4(cell + d).xyz + d;
                pos = rnd - offs;
                dist = dot(pos, pos);
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

fn Cell3Noise(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    let offs = p - cell;
    var pos: vec3<f32>;
    var rnd: vec3<f32>;
    var dist: f32;
    var distMin: f32 = 1.0e38;
    for (var dz: f32 = -1.0; dz < 2.0; dz += 1.0) {
        for (var dy: f32 = -1.0; dy < 2.0; dy += 1.0) {
            for (var dx: f32 = -1.0; dx < 2.0; dx += 1.0) {
                let d = vec3<f32>(dx, dy, dz);
                rnd = NoiseNearestUVec4(cell + d).xyz + d;
                pos = rnd - offs;
                dist = dot(pos, pos);
                distMin = min(distMin, dist);
            }
        }
    }
    return sqrt(distMin);
}

fn Cell3Noise2(p: vec3<f32>) -> vec2<f32> {
    let cell = floor(p);
    let offs = p - cell;
    var pos: vec3<f32>;
    var rnd: vec3<f32>;
    var dist: f32;
    var distMin1: f32 = 1.0e38;
    var distMin2: f32 = 1.0e38;
    for (var dz: f32 = -1.0; dz < 2.0; dz += 1.0) {
        for (var dy: f32 = -1.0; dy < 2.0; dy += 1.0) {
            for (var dx: f32 = -1.0; dx < 2.0; dx += 1.0) {
                let d = vec3<f32>(dx, dy, dz);
                rnd = NoiseNearestUVec4(cell + d).xyz + d;
                pos = rnd - offs;
                dist = dot(pos, pos);
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

fn Cell3Noise2Color(p: vec3<f32>, color: ptr<function, vec4<f32>>) -> vec2<f32> {
    let cell = floor(p);
    let offs = p - cell;
    var pos: vec3<f32>;
    var rnd: vec4<f32>;
    var rndM = vec4<f32>(1.0);
    var dist: f32;
    var distMin1: f32 = 1.0e38;
    var distMin2: f32 = 1.0e38;
    for (var dz: f32 = -1.0; dz < 2.0; dz += 1.0) {
        for (var dy: f32 = -1.0; dy < 2.0; dy += 1.0) {
            for (var dx: f32 = -1.0; dx < 2.0; dx += 1.0) {
                let d = vec3<f32>(dx, dy, dz);
                rnd = NoiseNearestUVec4(cell + d);
                pos = rnd.xyz + d - offs;
                dist = dot(pos, pos);
                if (dist < distMin1) {
                    distMin2 = distMin1;
                    distMin1 = dist;
                    rndM = rnd;
                } else {
                    distMin2 = min(distMin2, dist);
                }
            }
        }
    }
    *color = rndM;
    return sqrt(vec2<f32>(distMin1, distMin2));
}

// ----------------------------------------------------------------------------
// Inverse spherical Fibonacci (tg_common.glh 2057-2149); round(x) = floor(x + 0.5)

fn inverseSF(p: vec3<f32>, n: f32) -> vec2<f32> {
    const phi: f32 = 1.61803398875;

    let m = 1.0 - 1.0 / n;

    let fi = min(atan2(p.y, p.x), pi);
    let cosTheta = p.z;

    let k  = max(2.0, floor(log(n * pi * sqrt(5.0f) * (1.0 - cosTheta * cosTheta)) / log(phi + 1.0)));
    let Fk = pow(phi, k) / sqrt(5.0f);
    let F  = vec2<f32>(floor(Fk + 0.5), floor(Fk * phi + 0.5)); // k, k+1

    let ka = 2.0 * F / n;
    let kb = pi2 * (fract((F + 1.0) * phi) - (phi - 1.0));

    // mat2(ka.y, -ka.x, kb.y, -kb.x) / det  (WGSL has no mat / scalar: divide per element)
    let det = (ka.y * kb.x - ka.x * kb.y);
    let iB = mat2x2<f32>(ka.y / det, -ka.x / det, kb.y / det, -kb.x / det);

    let c = floor(iB * vec2<f32>(fi, cosTheta - m));
    var d: f32 = 8.0;
    var j: f32 = 0.0;
    for (var s: i32 = 0; s < 4; s++) {
        let uv = vec2<f32>(f32(s - 2 * (s / 2)), f32(s / 2));

        let i = dot(F, uv + c);

        let fi_s = pi2 * fract(i * phi);
        let cosTheta_s = m - 2.0 * i / n;
        let sinTheta = sqrt(1.0 - cosTheta_s * cosTheta_s);

        let q  = vec3<f32>(cos(fi_s) * sinTheta, sin(fi_s) * sinTheta, cosTheta_s);
        let r  = q - p;
        let d2 = dot(r, r);
        if (d2 < d) {
            d = d2;
            j = i;
        }
    }

    return vec2<f32>(j, sqrt(d));
}

// GLSL overload inverseSF(vec3 p, float n, out vec3 NearestPoint)
fn inverseSF_np(p: vec3<f32>, n: f32, NearestPoint: ptr<function, vec3<f32>>) -> vec2<f32> {
    const phi: f32 = 1.61803398875;

    let m = 1.0 - 1.0 / n;

    let fi = min(atan2(p.y, p.x), pi);
    let cosTheta = p.z;

    let k  = max(2.0, floor(log(n * pi * sqrt(5.0f) * (1.0 - cosTheta * cosTheta)) / log(phi + 1.0)));
    let Fk = pow(phi, k) / sqrt(5.0f);
    let F  = vec2<f32>(floor(Fk + 0.5), floor(Fk * phi + 0.5)); // k, k+1

    let ka = 2.0 * F / n;
    let kb = pi2 * (fract((F + 1.0) * phi) - (phi - 1.0));

    let det = (ka.y * kb.x - ka.x * kb.y);
    let iB = mat2x2<f32>(ka.y / det, -ka.x / det, kb.y / det, -kb.x / det);

    let c = floor(iB * vec2<f32>(fi, cosTheta - m));
    var d: f32 = 8.0;
    var j: f32 = 0.0;
    for (var s: i32 = 0; s < 4; s++) {
        let uv = vec2<f32>(f32(s - 2 * (s / 2)), f32(s / 2));

        let i = dot(F, uv + c);

        let fi_s = pi2 * fract(i * phi);
        let cosTheta_s = m - 2.0 * i / n;
        let sinTheta = sqrt(1.0 - cosTheta_s * cosTheta_s);

        let q  = vec3<f32>(cos(fi_s) * sinTheta, sin(fi_s) * sinTheta, cosTheta_s);
        let r  = q - p;
        let d2 = dot(r, r);
        if (d2 < d) {
            *NearestPoint = q;
            d = d2;
            j = i;
        }
    }

    return vec2<f32>(j, sqrt(d));
}

// ----------------------------------------------------------------------------
// Craters (tg_common.glh 2191-2209, 2224-2340)

fn CraterHeightFunc(lastlastLand: f32, lastLand: f32, height: f32, r: f32) -> f32 {
    let distHeight = craterDistortion * height;

    var t = 1.0 - r / radPeak;
    let peak = heightPeak * craterDistortion * smoothstep(0.0, 1.0, t);

    t = smoothstep(0.0, 1.0, (r - radInner) / (radRim - radInner));
    let inoutMask = t * t * t;
    let innerRim = heightRim * distHeight * smoothstep(0.0, 1.0, inoutMask);

    t = smoothstep(0.0, 1.0, (radOuter - r) / (radOuter - radRim));
    let outerRim = distHeight * mix(0.05, heightRim, t * t);

    t = saturate((1.0 - r) / (1.0 - radOuter));
    let halo = 0.05 * distHeight * t;

    return mix(lastlastLand + height * heightFloor + peak + innerRim, lastLand + outerRim + halo, inoutMask);
}

fn CraterNoise(point_in: vec3<f32>, cratMagn: f32, cratFreq: f32, cratSqrtDensity: f32, cratOctaves: f32) -> f32 {
    var point = (point_in * cratFreq + Randomize()) * cratSqrtDensity;

    var newLand: f32 = 0.0;
    var lastLand: f32 = 0.0;
    var lastlastLand: f32 = 0.0;
    var lastlastlastLand: f32 = 0.0;
    var amplitude: f32 = 1.0;
    var cell: f32;
    let radFactor = 1.0 / cratSqrtDensity;

    // Craters roundness distortion
    noiseH           = 0.5;
    noiseLacunarity  = 2.218281828459;
    noiseOffset      = 0.8;
    noiseOctaves     = 3.0;
    craterDistortion = 1.0;
    craterRoundDist  = 0.03;

    radPeak  = 0.03;
    radInner = 0.15;
    radRim   = 0.2;
    radOuter = 0.8;

    for (var i: i32 = 0; f32(i) < cratOctaves; i++) {
        lastlastlastLand = lastlastLand;
        lastlastLand = lastLand;
        lastLand = newLand;

        cell = Cell3Noise(point + craterRoundDist * Fbm3D(point * 2.56));
        newLand = CraterHeightFunc(lastlastlastLand, lastLand, amplitude, cell * radFactor);

        if (cratOctaves > 1.0) {
            point       *= craterFreqPower;
            amplitude   *= craterAmplPower;
            heightPeak  *= craterPeakPower;
            heightFloor *= craterFloorPower;
            radInner    *= craterRadiusPower;
        }
    }

    return cratMagn * newLand;
}

fn RayedCraterNoise(point_in: vec3<f32>, cratMagn: f32, cratFreq: f32, cratSqrtDensity: f32, cratOctaves: f32) -> f32 {
    var point = point_in;
    let rotVec = normalize(Randomize());

    // Craters roundness distortion
    noiseH           = 0.5;
    noiseLacunarity  = 2.218281828459;
    noiseOffset      = 0.8;
    noiseOctaves     = 3.0;
    craterDistortion = 1.0;
    craterRoundDist  = 0.03;
    let shapeDist = 1.0 + 2.5 * craterRoundDist * Fbm(point * 419.54);

    radPeak  = 0.004;
    radInner = 0.015;
    radRim   = 0.03;
    radOuter = 0.8;

    var newLand: f32 = 0.0;
    var lastLand: f32 = 0.0;
    var lastlastLand: f32 = 0.0;
    var lastlastlastLand: f32 = 0.0;
    var amplitude: f32 = 1.0;
    var cell: vec2<f32>;
    var rad: f32;
    var radFactor: f32 = shapeDist / cratSqrtDensity;
    var fibFreq: f32 = 2.0 * cratFreq;

    for (var i: i32 = 0; f32(i) < cratOctaves; i++) {
        lastlastlastLand = lastlastLand;
        lastlastLand = lastLand;
        lastLand = newLand;

        cell    = inverseSF(point, fibFreq);
        rad     = hash1(cell.x * 743.1) * 0.9 + 0.1;
        newLand = CraterHeightFunc(lastlastlastLand, lastLand, amplitude, cell.y * radFactor / rad);

        if (cratOctaves > 1.0) {
            point = Rotate(pi2 * hash1(f32(i)), rotVec, point);
            fibFreq     *= craterFreqPower;
            radFactor   *= craterRadFactorPower;
            amplitude   *= craterAmplPower;
            heightPeak  *= craterPeakPower;
            heightFloor *= craterFloorPower;
            radInner    *= craterRadiusPower;
        }
    }

    return cratMagn * newLand;
}

// ----------------------------------------------------------------------------
// Volcanoes (tg_common.glh 2402-2407, 2420-2438, 2460-2514)

fn VolcanoRidges(r: f32, fi1: f32, fi2: f32, rnd: f32) -> f32 {
    let ridges1 = iqTurbulence(vec3<f32>(fi1, r, rnd + 1.126), 0.55);
    let ridges2 = iqTurbulence(vec3<f32>(fi2, r, rnd + 0.754), 0.55);
    return ridges1 * saturate(abs(fi2)) + ridges2 * saturate(abs(fi1));
}

fn VolcanoHeightFunc(r: f32, fi1: f32, fi2: f32, rnd: f32, size: f32) -> f32 {
    let rs = 0.25 * r / size;
    let shape  = saturate(2.0 * size);
    let height = 0.75 + 0.75 * shape * shape;

    var cone = saturate(1.0 - pow(r, 0.5 + 0.5 * shape));

    const calderaRadius: f32 = 0.14;
    let t = rs * (10.0 / calderaRadius) - 2.5;
    let caldera = 0.85 + mix(0.07, 0.025, shape);
    let calderaMask = smoothstep(0.0, 1.0, t);

    noiseOctaves = 8.0;
    let ridges = VolcanoRidges(rs, fi1, fi2, rnd);
    cone += mix(0.02, 0.06, shape) * saturate(2.0 * sqrt(r)) * ridges;

    return height * mix(caldera, cone, calderaMask);
}

fn VolcanoNoise(point_in: vec3<f32>, globalLand: f32, localLand: f32) -> f32 {
    var point = point_in;
    noiseLacunarity = 2.218281828459;
    noiseH          = 0.5;
    noiseOffset     = 0.8;

    var frequency: f32 = 150.0 * volcanoFreq();
    let density: f32   = volcanoDensity();
    var size: f32      = volcanoRadius();
    var newLand: f32   = localLand;
    let globLand: f32  = globalLand - 1.0;
    var amplitude: f32 = 2.0 * volcanoMagn();
    var cell: vec2<f32>;
    var cellCenter = vec3<f32>(0.0);
    let rotVec   = normalize(Randomize());
    let binormal = normalize(vec3<f32>(-point.z, 0.0, point.x)); // = normalize(cross(point, vec3(0, 1, 0)));
    var distFreq: f32 = 18.361 * volcanoFreq();
    var distMagn: f32 = 0.003;

    for (var i: i32 = 0; f32(i) < volcanoOctaves(); i++) {
        noiseOctaves = 4.0;
        let p = point + distMagn * Fbm3D(point * distFreq);

        cell = inverseSF_np(p, frequency, &cellCenter);

        let h = hash1(cell.x);
        let r = 40.0 * cell.y;
        if ((h < density) && (r < 1.0)) {
            let rnd = 48.3 * dot(cellCenter, Randomize());
            let cen = normalize(cellCenter - p);
            let a   = dot(p, cross(cen, binormal));
            let b   = dot(cen, binormal);
            let fi1 = atan2( a,  b) / pi;
            let fi2 = atan2(-a, -b) / pi;

            let volcano = globLand + amplitude * VolcanoHeightFunc(r, fi1, fi2, rnd, size);
            newLand = softPolyMax(newLand, volcano, 0.3);
        }

        if (volcanoOctaves() > 1.0) {
            point = Rotate(pi2 * hash1(f32(i)), rotVec, point);
            frequency *= 2.0;
            size      *= 0.5;
            amplitude *= 1.2;
            distFreq  *= 2.0;
            distMagn  *= 0.5;
        }
    }

    return newLand;
}

// ----------------------------------------------------------------------------
// Mare (tg_common.glh 2571-2631)

fn MareHeightFunc(lastLand: f32, lastlastLand: f32, height: f32, r: f32, mareFloor: ptr<function, f32>) -> f32 {
    var t: f32;
    var res: f32;   // GLSL returns from each branch; single exit here

    if (r < radInner) {
        // crater bottom
        *mareFloor = 1.0;
        res = lastlastLand + height * heightFloor;
    } else if (r < radRim) {
        // inner rim
        t = (r - radInner) / (radRim - radInner);
        t = smoothstep(0.0, 1.0, t);
        *mareFloor = 1.0 - t;
        res = mix(lastlastLand + height * heightFloor, lastLand + height * heightRim * craterDistortion, t);
    } else if (r < radOuter) {
        // outer rim
        t = 1.0 - (r - radRim) / (radOuter - radRim);
        *mareFloor = 0.0;
        res = mix(lastLand, lastLand + height * heightRim * craterDistortion, smoothstep(0.0, 1.0, t * t));
    } else {
        *mareFloor = 0.0;
        res = lastLand;
    }
    return res;
}

fn MareNoise(point_in: vec3<f32>, globalLand: f32, bottomLand: f32, mareFloor: ptr<function, f32>) -> f32 {
    var point = (point_in * mareFreq() + Randomize()) * mareSqrtDensity();

    var amplitude: f32 = 0.7;
    var newLand: f32 = globalLand;
    var lastLand: f32;
    var cell: f32;
    var radFactor: f32 = 1.0 / mareSqrtDensity();

    radPeak  = 0.0;
    radInner = 0.5;
    radRim   = 0.6;
    radOuter = 1.0;
    heightFloor = 0.0;
    heightRim   = 0.2;

    for (var i: i32 = 0; i < 3; i++) {
        cell = Cell2Noise(point + 0.07 * Fbm3D(point));
        lastLand = newLand;
        newLand = MareHeightFunc(lastLand, bottomLand, amplitude, cell * radFactor, mareFloor);
        point = point * 1.3 + Randomize();
        amplitude *= 0.62;
        radFactor *= 1.2;
    }

    *mareFloor = 1.0 - *mareFloor;
    return newLand;
}

// ----------------------------------------------------------------------------
// Cracks (tg_common.glh 2635-2678)

fn CrackHeightFunc(lastLand: f32, lastlastLand: f32, height: f32, r: f32, p_in: vec3<f32>) -> f32 {
    var p = p_in;
    p.x += 0.05 * r;
    let inner = smoothstep(0.0, 0.5, r);
    let outer = smoothstep(0.5, 1.0, r);
    let cracks = height * (0.4 * Noise(p * 625.7) * (1.0 - inner) + inner * (1.0 - outer));
    let land = mix(lastLand, lastlastLand, r);
    return mix(cracks, land, outer);
}

fn CrackNoise(point_in: vec3<f32>, mask: ptr<function, f32>) -> f32 {
    var point = (point_in + Randomize()) * cracksFreq();

    var newLand: f32 = 0.0;
    var lastLand: f32 = 0.0;
    var lastlastLand: f32 = 0.0;
    var cell: vec2<f32>;
    var r: f32;
    var ampl: f32 = 0.4 * cracksMagn();
    *mask = 1.0;

    // Rim shape and height distortion
    noiseH          = 0.5;
    noiseLacunarity = 2.218281828459;
    noiseOffset     = 0.8;
    noiseOctaves    = 6.0;

    for (var i: i32 = 0; f32(i) < cracksOctaves(); i++) {
        cell = Cell2Noise2(point + 0.02 * Fbm3D(1.8 * point));
        r    = smoothstep(0.0, 1.0, 250.0 * abs(cell.y - cell.x));
        lastlastLand = lastLand;
        lastLand = newLand;
        newLand = CrackHeightFunc(lastlastLand, lastLand, ampl, r, point);
        point = point * 1.2 + Randomize();
        ampl *= 0.8333;
        *mask *= smoothstep(0.6, 1.0, r);
    }

    return newLand;
}

// ----------------------------------------------------------------------------
// Dunes (tg_common.glh 2715-2746)

fn DunesNoise(point: vec3<f32>, octaves: f32) -> f32 {
    var dir = sNoise(point * 3.86) * 197.3 * dunesFreq();
    var p = point;

    let glob = saturate(Fbm(p * 7.21) + 0.3);
    let dwin = 1.0 / (octaves - 1.0);
    var win: f32 = 0.0;

    var wave: f32;
    var fade: f32;
    var dist: f32;
    var dunes: f32 = 0.0;
    var ampl: f32  = 0.05;
    let lac: f32   = 1.17;

    for (var i: i32 = 0; f32(i) < octaves; i++) {
        dist = dir + sNoise(p * dunesFreq() * 25.0) * 12.7 + sNoise(p * dunesFreq() * 300.0) * 1.2;
        wave = fract(dist / pi);
        wave = cos(pi * wave * wave);
        fade = smoothstep(win - 0.5 * dwin, win, glob) * (1.0 - smoothstep(win + dwin, win + 1.5 * dwin, glob));
        dunes += (1.0 - sqrt(wave * wave + 0.005)) * (ampl + Fbm(p * dunesFreq() * 150.0) * 0.05 - 0.03) * fade;
        p = p * lac + vec3<f32>(3.17, 5.38, 8.79);
        dir  *= lac;
        ampl /= lac;
        win  += dwin;
    }

    return max(dunes, 0.0);
}

// ----------------------------------------------------------------------------
// Rodrigo's rivers and rifts (tg_common.glh 3245-3301)

fn PseudoRivers(point: vec3<f32>, global_: f32, damping: f32, height: ptr<function, f32>) {
    noiseOctaves    = 5.0;
    noiseH          = 1.0;
    noiseLacunarity = 4.0;
    noiseOffset     = 0.95;

    let p = point * 2.17 + Randomize();
    var distort = 0.325 * Fbm3D(p * riversSin());
    distort += vec3<f32>(0.003 * RidgedMultifractalEroded(point * 5.0 * canyonsFreq() + Randomize(), 2.0, erosion()));

    let cell = 2.5 * Cell3Noise2(riversFreq() * p + distort);

    var valleys = 1.0 - (saturate(0.4 * abs(cell.y - cell.x) * riversMagn()));
    valleys = smoothstep(0.25, 0.99, valleys) * damping;
    *height = mix(*height, seaLevel() + 0.003, valleys);

    var rivers = 1.0 - (saturate(4.5 * abs(cell.y - cell.x) * riversMagn()));
    rivers = smoothstep(0.0, 1.0, rivers) * damping;
    *height = mix(*height, seaLevel() - 0.0015, rivers);
}

fn Rifts(point: vec3<f32>, height: ptr<function, f32>) {
    let riftsBottom = seaLevel() - 0.07;

    noiseOctaves    = 5.0;
    noiseH          = 1.0;
    noiseLacunarity = 4.0;
    noiseOffset     = 0.95;

    // 2 slightly different octaves to make ridges inside rifts
    var p = point * 0.12;
    var rifts: f32 = 0.0;
    for (var i: i32 = 0; i < 2; i++) {
        let distort = 0.2 * Fbm3D(p * riftsSin());
        let cell = Cell3Noise2(riftsFreq() * p + distort);
        let width = riftsMagn() * abs(cell.y - cell.x);
        rifts = softPolyMax(rifts, 1.0 - 2.75 * width, 0.2);
        p *= 1.02;
    }

    let riftsModulate = smoothstep(-0.1, 0.1, Fbm(point * 2.3 + Randomize()));
    rifts = smoothstep(0.0, 0.65, rifts * riftsModulate);

    *height = mix(*height, riftsBottom, rifts);

    // Slope modulation
    let slope = smoothstep(0.1, 0.9, 1.0 - 2.0 * abs(rifts * 0.65 - 0.5));
    var slopeMod = 0.1 * slope * RidgedMultifractalErodedDetail(point * 7.0 * canyonsFreq() + Randomize(), 2.0, erosion(), 1.0);
    slopeMod *= riftsModulate;
    *height = softPolyMax(*height - slopeMod, riftsBottom, 0.2);
}

// ----------------------------------------------------------------------------
// tg_terra_height.glsl  (GLSL: void HeightMapTerra(vec3 point, out vec4 HeightBiomeMap))

fn HeightMapTerra(point: vec3<f32>) -> vec4<f32> {
    // Assign a climate type
    noiseOctaves    = select(12.0f, 5.0f, oceanType() == 1.0); // Reduce terrain octaves on oceanic planets
    noiseH          = 0.5;
    noiseLacunarity = 2.218281828459;
    noiseOffset     = 0.8;
    var climate: f32;
    var latitude: f32;
    if (tidalLock() <= 0.0) {
        latitude = abs(point.y);
        latitude += 0.15 * (Fbm(point * 0.7 + Randomize()) - 1.0);
        latitude = saturate(latitude);
        if (latitude < latTropic() - tropicWidth()) {
            climate = mix(climateTropic(), climateEquator(), (latTropic() - tropicWidth() - latitude) / latTropic());
        } else if (latitude > latTropic() + tropicWidth()) {
            climate = mix(climateTropic(), climatePole(), (latitude - latTropic() - tropicWidth()) / (1.0 - latTropic()));
        } else {
            climate = climateTropic();
        }
    } else {
        latitude = 1.0 - point.x;
        latitude += 0.15 * (Fbm(point * 0.7 + Randomize()) - 1.0);
        climate = mix(climateTropic(), climatePole(), saturate(latitude));
    }

    // Global landscape
    let p = point * mainFreq() + Randomize();
    noiseOctaves = 5.0;
    var distort = 0.35 * Fbm3D(p * 0.73);
    noiseOctaves = 4.0;
    distort += 0.005 * (1.0 - abs(Fbm3D(p * 132.3)));
    var global_ = 1.0 - Cell3Noise(p + distort);

    // Make sea bottom more flat
    global_ = softPolyMax(global_, 0.0, 0.1);
    global_ = pow(global_, 1.5);

    // Venus-like structure
    var venus: f32 = 0.0;
    if (venusMagn() > 0.05) {
        noiseOctaves = 4.0;
        distort = Fbm3D(point * 0.3) * 1.5;
        noiseOctaves = 6.0;
        venus = Fbm((point + distort) * venusFreq()) * venusMagn();
    }

    global_ = (global_ + venus - seaLevel()) * 0.5 + seaLevel();
    let shore = saturate(70.0 * (global_ - seaLevel()));

    // ***** Rodrigo *****
    noiseOctaves = 10.0;
    noiseH       = 1.0;
    noiseLacunarity = 2.3;
    noiseOffset  = montesSpiky();
    var rocks = -0.013 * iqTurbulence(point * 80.0, 1.0);
    rocks = smoothstep(-0.9, 0.1, rocks);

    // Biome domains
    noiseOctaves = 6.0;
    let pb = p * 2.3 + 0.5 * Fbm3D(p * 1.5);
    var col: vec4<f32>;
    let cell = Cell3Noise2Color(pb, &col);
    let biome = col.r;
    let biomeScale = saturate(2.0 * (pow(abs(cell.y - cell.x), 0.7) - 0.05));
    let terrace = col.g;
    var terraceLayers = max(col.b * 10.0 + 3.0, 3.0);
    terraceLayers += Fbm(pb * 5.41);

    var montRange = saturate(DistNoise(point * 22.6 + Randomize(), 2.5) + 0.5);
    montRange *= montRange;
    let montBiomeScale = min(pow(2.2 * biomeScale, 3.5), 1.0) * montRange;

    let inv2montesSpiky = 1.0 / (montesSpiky() * montesSpiky());
    var heightD: f32  = 0.0;
    var height: f32   = 0.0;
    var landform: f32 = 0.0;
    var dist: f32;

    if (biome < dunesFraction()) {
        // Dunes
        noiseOctaves = 2.0;
        dist = dunesFreq() + Fbm(p * 4.21);
        let desert = max(Fbm(p * dist), 0.0);
        let dunes  = DunesNoise(point, 3.0);
        landform = (0.0002 * desert + dunes) * pow(biomeScale, 3.0);
        heightD = dunesMagn() * landform;
    } else if (biome < hillsFraction()) {
        // "Eroded" hills
        noiseOctaves = 10.0;
        noiseH       = 1.0;
        noiseOffset  = 1.0;
        landform     = 1.5 - RidgedMultifractal(point * hillsFreq() + Randomize(), 2.0);
        height = biomeScale * hillsMagn() * landform;
    } else if (biome < hills2Fraction()) {
        // "Eroded" hills 2
        noiseOctaves = 10.0;
        noiseLacunarity = 2.0;
        landform = JordanTurbulence(point * hillsFreq() + Randomize(), 0.8, 0.5, 0.6, 0.35, 1.0, 0.8, 1.0);
        height = biomeScale * hillsMagn() * landform;
    } else if (biome < canyonsFraction()) {
        // Canyons
        noiseOctaves = 5.0;
        noiseH       = 0.9;
        noiseLacunarity = 4.0;
        noiseOffset  = montesSpiky();
        landform = RidgedMultifractalErodedDetail(point * 4.0 * canyonsFreq() * inv2montesSpiky + Randomize(), 2.0, erosion(), montBiomeScale);
        height = -0.5 * canyonsMagn() * montRange * landform;

        if (terrace < terraceProb()) {
            let h = height * terraceLayers * 5.0;
            height = (floor(h) + smoothstep(0.5, 0.6, fract(h))) / (terraceLayers * 5.0);
        }
    } else {
        // Mountains
        noiseOctaves = 10.0;
        noiseH       = 1.0;
        noiseLacunarity = 2.0;
        noiseOffset  = montesSpiky();
        landform = RidgedMultifractalErodedDetail(point * montesFreq() * inv2montesSpiky + Randomize(), 2.0, erosion(), montBiomeScale);
        height = 0.5 * montesMagn() * montRange * landform;

        if (terrace < terraceProb()) {
            let h = height * terraceLayers * 1.437;
            height = (floor(h) + smoothstep(0.0, 1.0, fract(h))) / (terraceLayers * 1.437);
        }
    }

    // Mare
    var mare: f32 = global_;
    var mareFloor: f32 = global_;
    var mareSuppress: f32 = 1.0;
    if (mareSqrtDensity() > 0.05) {
        noiseH           = 0.5;
        noiseLacunarity  = 2.218281828459;
        noiseOffset      = 0.8;
        craterDistortion = 1.0;
        noiseOctaves     = 6.0;  // Mare roundness distortion
        mare = MareNoise(point, global_, 0.0, &mareSuppress);
    }

    height *= saturate(20.0 * mare);        // suppress mountains, canyons and hills (but not dunes) inside mare
    height = (height + heightD) * shore;    // suppress all landforms inside seas

    // Ice caps
    let oceaniaFade = select(1.0f, 0.2f, oceanType() == 1.0);
    let iceCap = smoothstep(0.0, 1.0, saturate((latitude / latIceCaps() - 1.0) * 50.0 * oceaniaFade));

    // Ice cracks
    var mask: f32 = 1.0;
    if (cracksOctaves() > 0.0) {
        landform = CrackNoise(point, &mask) * iceCap;
        height += landform;
    }

    // Craters
    var crater: f32 = 0.0;
    if (craterSqrtDensity() > 0.05) {
        heightFloor = -0.1;
        heightPeak  =  0.6;
        heightRim   =  1.0;
        crater = CraterNoise(point, 0.5 * craterMagn(), craterFreq(), craterSqrtDensity(), craterOctaves());
        noiseOctaves    = 10.0;
        noiseLacunarity = 2.0;
        crater = 0.25 * crater + 0.05 * crater * iqTurbulence(point * montesFreq() + Randomize(), 0.55);

        // Young terrain - suppress craters
        noiseOctaves = 4.0;
        let youngDistort = Fbm3D((point - Randomize()) * 0.07) * 1.1;
        noiseOctaves = 4.0;
        var young = 1.0 - Fbm(point + youngDistort);
        young = smoothstep(0.0, 1.0, young * young * young);
        crater *= young;
    }

    height += mare + crater;

    // ***** Rodrigo *****
    var rodrigoDamping = rocks * (0.000000008 * hillsFreq() * hillsFreq() + 0.0072);
    rodrigoDamping = global_ - seaLevel() - rodrigoDamping;

    // Pseudo rivers
    if (riversMagn() > 0.0) {
        let damping = (glsl_smoothstep(0.065, 0.055, rodrigoDamping)) *    // disable rivers inside continents
                      (glsl_smoothstep(0.005, 0.000, seaLevel() - height));  // disable rivers inside oceans
        PseudoRivers(point, global_, damping, &height);
    }

    // Rifts
    if (riftsMagn() > 0.0) {
        Rifts(point, &height);
    }

    // Shield volcano
    if (volcanoOctaves() > 0.0) {
        height = VolcanoNoise(point, global_, height);
    }

    // Apply ice caps
    height = height * oceaniaFade + icecapHeight() * iceCap;

    // smoothly limit the height
    height = softPolyMin(height, 0.99, 0.3);

    return vec4<f32>(height);
}

// ----------------------------------------------------------------------------
// tg_selena_height.glsl

fn HeightMapSelena(point: vec3<f32>) -> f32 {
    // Biome domains
    var p = point * mainFreq() + Randomize();
    var col: vec4<f32>;
    noiseOctaves = 6.0;
    var distort = p * 2.3 + 13.5 * Fbm3D(p * 0.06);
    var cell = Cell3Noise2Color(distort, &col);
    let biome = col.r;
    let biomeScale = saturate(2.0 * (pow(abs(cell.y - cell.x), 0.7) - 0.05));

    var montRage = saturate(DistNoise(point * 22.6 + Randomize(), 2.5) + 0.5);
    montRage *= montRage;
    let montBiomeScale = min(pow(2.2 * biomeScale, 2.5), 1.0) * montRage;
    let inv2montesSpiky = 1.0 / (montesSpiky() * montesSpiky());

    // Global landscape
    noiseOctaves = 4.0;
    p = point * mainFreq() + Randomize();
    distort = 0.35 * Fbm3D(p * 2.37);
    p += distort;
    var global_ = 0.6 * (1.0 - Cell3Noise(p));

    // Venus-like structure
    var venus: f32 = 0.0;
    if (venusMagn() > 0.05) {
        noiseOctaves = 4.0;
        distort = Fbm3D(point * 0.3) * 1.5;
        noiseOctaves = 6.0;
        venus = Fbm((point + distort) * venusFreq()) * venusMagn();
    }
    global_ += venus;

    // Mare
    var mare: f32 = global_;
    var mareFloor: f32 = global_;
    var mareSuppress: f32 = 1.0;
    if (mareSqrtDensity() > 0.05) {
        noiseOctaves = 2.0;
        mareFloor = 0.6 * (1.0 - Cell3Noise(0.3 * p));
        craterDistortion = 1.0;
        noiseOctaves = 6.0;  // Mare roundness distortion
        mare = MareNoise(point, global_, mareFloor, &mareSuppress);
    }

    // Old craters
    var crater: f32 = 0.0;
    if (craterSqrtDensity() > 0.05) {
        heightFloor = -0.1;
        heightPeak  =  1.6;
        heightRim   =  1.0;
        crater = mareSuppress * CraterNoise(point, craterMagn(), craterFreq(), craterSqrtDensity(), craterOctaves());
        noiseOctaves    = 10.0;
        noiseLacunarity = 2.0;
        crater = 0.25 * crater + 0.05 * crater * iqTurbulence(point * montesFreq() + Randomize(), 0.55);

        // Young terrain - suppress craters
        noiseOctaves = 4.0;
        let youngDistort = Fbm3D((point - Randomize()) * 0.07) * 1.1;
        noiseOctaves = 8.0;
        var young = 1.0 - Fbm(point + youngDistort);
        young = smoothstep(0.0, 1.0, young * young * young);
        crater *= young;
    }

    var height = mare + crater;

    // Ice cracks
    var mask: f32 = 1.0;
    if (cracksOctaves() > 0.0) {
        height += CrackNoise(point, &mask);
    }

    if (biome > hillsFraction()) {
        if (biome < hills2Fraction()) {
            // "Freckles" (structures like on Europa)
            noiseOctaves    = 10.0;
            noiseLacunarity = 2.0;
            height += 0.2 * hillsMagn() * mask * biomeScale * JordanTurbulence(point * hillsFreq() + Randomize(), 0.8, 0.5, 0.6, 0.35, 1.0, 0.8, 1.0);
        } else if (biome < canyonsFraction()) {
            // Rimae
            noiseOctaves     = 3.0;
            noiseLacunarity  = 2.218281828459;
            noiseH           = 0.9;
            noiseOffset      = 0.5;
            p = point * mainFreq() + Randomize();
            distort  = 0.035 * Fbm3D(p * riversSin() * 5.0);
            distort += 0.350 * Fbm3D(p * riversSin());
            cell = Cell3Noise2(canyonsFreq() * 0.05 * p + distort);
            var rima = 1.0 - saturate(abs(cell.y - cell.x) * 250.0 * canyonsMagn());
            rima = biomeScale * smoothstep(0.0, 1.0, rima);
            height = mix(height, height - 0.02, rima);
        } else {
            // Mountains
            noiseOctaves    = 10.0;
            noiseLacunarity = 2.0;
            height += montesMagn() * montBiomeScale * iqTurbulence(point * 0.5 * montesFreq() + Randomize(), 0.45);
        }
    }

    // Equatorial ridge
    var ridgeHeight: f32 = 0.0;
    if (eqridgeMagn() > 0.0) {
        noiseOctaves    = 4.0;
        noiseLacunarity = 2.218281828459;
        noiseH          = 0.5;

        ridgeHeight = glsl_smoothstep(eqridgeWidth() * 2.0, 0.0, abs(point.y)); // this is simply faster

        var ridgeModulate = 1.0 - eqridgeModMagn() * (Fbm(point * eqridgeModFreq() - Randomize()) * 0.5 + 0.5);

        // smoothly limit the height and smoothly blend with the surface
        ridgeModulate = softPolyMax(ridgeModulate, 0.01, 0.3);

        height += eqridgeMagn() * ridgeHeight * ridgeModulate;
    }

    // Rayed craters
    if (craterSqrtDensity() * craterSqrtDensity() * craterRayedFactor() > 0.05f * 0.05f) {
        heightFloor = -0.5;
        heightPeak  =  1.6;
        heightRim   =  1.0;
        let craterRayedSqrtDensity = craterSqrtDensity() * sqrt(craterRayedFactor());
        let craterRayedOctaves = floor(craterOctaves() * craterRayedFactor());
        let craterRayedMagn = craterMagn() * 200.0 * pow(craterAmplPower, craterOctaves() - craterRayedOctaves);
        crater = RayedCraterNoise(point, craterRayedMagn, craterFreq(), craterRayedSqrtDensity, craterRayedOctaves);

        noiseOctaves    = 10.0;
        noiseLacunarity = 2.0;
        crater += 0.25 * crater * iqTurbulence(point * montesFreq() + Randomize(), 0.55);

        // Reduce crater depth on top of equatorial ridges
        crater *= 1.0 - 0.9 * ridgeHeight;

        height += crater;
    }

    // Shield volcano
    if (volcanoOctaves() > 0.0) {
        height = VolcanoNoise(point, global_, height);
    }

    // smoothly limit the height
    height = softPolyMin(height, 0.99, 0.3);
    height = softPolyMax(height, 0.05, 0.1);

    return height;
}

// ----------------------------------------------------------------------------
// tg_asteroid_height.glsl

fn HeightMapAsteroid(point: vec3<f32>) -> f32 {
    // Global landscape
    let p = point * venusFreq() + Randomize();
    var height = venusMagn() * (0.5 - Noise(p) * 2.0);

    noiseOctaves = 10.0;
    noiseLacunarity = 2.0;
    height += 0.05 * iqTurbulence(point * 2.0 * mainFreq() + Randomize(), 0.35);

    // Hills
    noiseOctaves = 5.0;
    noiseLacunarity = 2.218281828459;
    var hills = (0.5 + 1.5 * Fbm(p * 0.0721)) * hillsFreq();
    hills = Fbm(p * hills) * 0.15;
    noiseOctaves = 2.0;
    let hillsMod = glsl_smoothstep(0.0, 1.0, Fbm(p * hillsFraction()) * 3.0);
    height *= 1.0 + hillsMagn() * hills * hillsMod;

    // Craters
    heightFloor = -0.1;
    heightPeak  =  0.6;
    heightRim   =  0.4;
    var crater = 0.4 * CraterNoise(point, craterMagn(), craterFreq(), craterSqrtDensity(), craterOctaves());

    noiseOctaves = 10.0;
    noiseLacunarity = 2.0;
    crater += montesMagn() * crater * iqTurbulence(point * montesFreq(), 0.52);

    height += crater;

    // Equatorial ridge
    if (eqridgeMagn() > 0.0) {
        noiseOctaves = 4.0;
        let x = point.y / eqridgeWidth();
        let ridgeHeight = exp(-0.5 * x * x);
        let ridgeModulate = saturate(1.0 - eqridgeModMagn() * (Fbm(point * eqridgeModFreq() - Randomize()) * 0.5 + 0.5));
        height += eqridgeMagn() * ridgeHeight * ridgeModulate;
    }

    // smoothly limit the height
    height = softPolyMin(height, 0.99, 0.3);
    height = softPolyMax(height, 0.01, 0.3);

    return height;
}

// ----------------------------------------------------------------------------

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let N = u32(tg.grid_origin.w);
    if (id.x >= N || id.y >= N) {
        return;
    }
    let point = normalize(tg.grid_origin.xyz + f32(id.x) * tg.grid_du.xyz + f32(id.y) * tg.grid_dv.xyz);

    var height: f32;
    if (tg.grid_dv.w < 0.5) {
        height = HeightMapTerra(point).x;
    } else if (tg.grid_dv.w < 1.5) {
        height = HeightMapSelena(point);
    } else {
        height = HeightMapAsteroid(point);
    }
    heights[id.y * N + id.x] = height;
}
