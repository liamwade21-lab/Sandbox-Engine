// Moment shadow mapping as Elite filters its shadowed directional light, and its pick of a cascade: the pure maths both the
// Sidewinder mod's own lighting (sidewinder_lit.wesl `star_shadow`, through sidewinder_shadow_common.wesl) and the world's
// receivers of the ship's shadow (ground_light.wesl `ship_shade_at`: the ground, the scatter's objects, the hero's and the
// props' light) read the ship's maps through. One implementation: whatever reads a moments map goes through these. No
// bindings and no records here: each caller passes its texture, sampler and the values of its own record.
//
// Ported from Elite's reference lit shader
// `ps_Metalness_Roughness_OpaqueDeferred_EmissivePulse_TransparentBox_Forward_1_1_0_0` (re/elite/frea_export/sidewinder/
// decoded/shaders/, lines 182-400 and 578-589; grade C but where marked). Each texel holds four moments of its casters' depth
// in an optimised quantisation; a 4 x 4 cubic B-spline filter of them is read with 16 gathers (`moment_filtered`, its taps
// kept inside the map's region as Elite's `c_gatherClampMin/Max` keep a cascade's inside its cell); they are turned back,
// pulled toward 0.5 by the moment bias, and the visibility at the point's depth is the "Hamburger" four-moment reconstruction
// (`moment_visibility`), then Elite's light-bleeding rescale and smoothstep. The values the caller passes: `msm` x Elite's
// quantisation offset, y the moment bias (3e-5), z the light-bleeding cutoff (0.3) (C: `FUN_1428903f0`); `r0`..`r3` the rows
// of Elite's dequantisation (FWD 346-349); a map's `region` xy its corner in the texture (texels), z its size (moment texels
// a side); `texel_inv` 1 / the texture's width and height.
//
// `cascade_pick` goes through Elite's cascades in order and takes the FIRST whose box in light space holds the point, its
// weight the smoothstep of the point's distance to that box's edge (in the box's half-width units) between the fade's floor
// and its top (Elite's gDirectionalShadow_FadeThresholds, cb1[326]); while that weight is under 1 the next cascade holding
// the point is blended in (`cascade_blend`). Each box is centred `ahead[k]` m along the view from the camera, `half[k]` m wide
// either side across the light (E: taken as unbounded along the light). Its CPU copy is sidewinder/shadow.rs's test `pick` (and
// `blend`), and `the_pick_is_elites` fails if these lines change without it.

// D3D's `_sat` (FWD's mad_sat, mul_sat, movc_sat): 0..1, and a NaN gives 0. WGSL's `saturate` leaves a NaN's result to the
// implementation (on Vulkan it stays NaN here), so the port uses this wherever Elite's lines saturate.
fn sat(x: f32) -> f32 {
    return select(0.0, min(x, 1.0), x > 0.0);
}

fn sat4(x: vec4<f32>) -> vec4<f32> {
    return select(vec4<f32>(0.0), min(x, vec4<f32>(1.0)), x > vec4<f32>(0.0));
}

// FWD 236-241: the cubic B-spline's four weights at the offset f (-0.5..0.5) from the nearest texel corner.
fn bspline(f: f32) -> vec4<f32> {
    var w = f * vec4<f32>(-0.166667, 0.5, -0.5, 0.166667) + vec4<f32>(0.25, -0.25, -0.25, 0.25);
    w = f * w + vec4<f32>(-0.125, -0.625, 0.625, 0.125);
    return sat4(f * w + vec4<f32>(0.020833, 0.479167, 0.479167, 0.020833));
}

// FWD 253-256: one channel of a gather (x the lower left, y lower right, z upper right, w upper left) interpolated: along each
// row by fx, then between the rows by fy.
fn gathered(g: vec4<f32>, fx: f32, fy: f32) -> f32 {
    return mix(mix(g.w, g.z, fx), mix(g.x, g.y, fx), fy);
}

// The four channels' bilinear value at a texel corner `c` (texels).
fn gather4(t: texture_2d<f32>, smp: sampler, texel_inv: vec2<f32>, c: vec2<f32>, fx: f32, fy: f32) -> vec4<f32> {
    let uv = c * texel_inv;
    let r = textureGather(0, t, smp, uv);
    let g = textureGather(1, t, smp, uv);
    let b = textureGather(2, t, smp, uv);
    let a = textureGather(3, t, smp, uv);
    return vec4<f32>(gathered(r, fx, fy), gathered(g, fx, fy), gathered(b, fx, fy), gathered(a, fx, fy));
}

// FWD 233-344: a map's moments filtered by the 4 x 4 cubic B-spline round `uv` (0..1 across the map), as four bilinear reads
// of 2 x 2 texels each, each weighted by its pair of weights' sums. Each read's corner is kept inside the map's region, so no
// tap reaches a neighbour's texels.
fn moment_filtered(t: texture_2d<f32>, smp: sampler, texel_inv: vec2<f32>, region: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    let g = uv * region.z;
    let base = floor(g + 0.5);
    let f = g - base;
    let wx = bspline(f.x);
    let wy = bspline(f.y);
    let sx = vec2<f32>(wx.x + wx.y, wx.z + wx.w);
    let sy = vec2<f32>(wy.x + wy.y, wy.z + wy.w);
    let fx = vec2<f32>(wx.y / sx.x, wx.w / sx.y);
    let fy = vec2<f32>(wy.y / sy.x, wy.w / sy.y);
    let inside_lo = region.xy + 1.0;
    let inside_hi = region.xy + region.z - 1.0;
    let lo = clamp(region.xy + base - 1.0, inside_lo, inside_hi);
    let hi = clamp(region.xy + base + 1.0, inside_lo, inside_hi);
    var q = sy.x * sx.x * gather4(t, smp, texel_inv, lo, fx.x, fy.x);
    q += sy.x * sx.y * gather4(t, smp, texel_inv, vec2<f32>(hi.x, lo.y), fx.y, fy.x);
    q += sy.y * sx.x * gather4(t, smp, texel_inv, vec2<f32>(lo.x, hi.y), fx.x, fy.y);
    q += sy.y * sx.y * gather4(t, smp, texel_inv, hi, fx.y, fy.y);
    return q;
}

// A map point's texture coordinates (x, y in -1..1 across the map, y up) in 0..1 across the map.
fn moment_map_uv(xy: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
}

// FWD 345-400: the visibility (1 lit) at depth `z` from the filtered quantised moments `q`.
fn moment_visibility(q: vec4<f32>, z: f32, msm: vec4<f32>, r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, r3: vec4<f32>) -> f32 {
    let c = q - vec4<f32>(msm.x, 0.0, 0.0, 0.0);
    let raw = vec4<f32>(dot(c, r0), dot(c, r1), dot(c, r2), dot(c, r3));
    // (The moment bias: toward 0.5.)
    let b = mix(raw, vec4<f32>(0.5), msm.y);
    // The Hankel matrix's Cholesky factors (353-356).
    let d22 = b.y - b.x * b.x;
    let l32d22 = b.z - b.x * b.y;
    let var2 = b.w - b.y * b.y;
    let l32 = l32d22 / d22;
    let d33 = var2 - l32 * l32 * d22;
    // (1, z, z^2) solved through them (357-367).
    let c1a = z - b.x;
    let c2a = z * z - b.y - l32 * c1a;
    let c1b = c1a / d22;
    let c2 = c2a / d33;
    let c1 = c1b - l32 * c2;
    let c0 = 1.0 - b.x * c1b + (b.x * l32 - b.y) * c2;
    // The quadratic's two roots (368-375).
    let root = sqrt(c1 * c1 - 4.0 * c2 * c0) / (2.0 * c2);
    let mid = -c1 / (2.0 * c2);
    let z1 = mid - abs(root);
    let z2 = mid + abs(root);
    // The weights (376-395).
    let beyond = z2 >= z;
    let sw0 = select(z1, z, beyond);
    let sw1 = select(z, z1, beyond);
    let quotient = (sw0 * z2 - b.x * (sw0 + z2) + b.y) / ((z2 - sw1) * (z - z1));
    var v = select(sat(-quotient), sat(1.0 - quotient), beyond);
    v = select(1.0, v, z1 < z);
    // Elite's light-bleeding rescale and smoothstep (396-400; 154-155).
    let x = sat((v - msm.z) / (1.0 - msm.z));
    return x * x * (3.0 - 2.0 * x);
}

// A map's visibility (1 lit) at the map point `mp` (x, y across the map, z the depth, 0 on the star's side); 1 off the map.
fn moment_map_visibility(t: texture_2d<f32>, smp: sampler, texel_inv: vec2<f32>, region: vec4<f32>, msm: vec4<f32>, r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, r3: vec4<f32>, mp: vec3<f32>) -> f32 {
    if abs(mp.x) >= 1.0 || abs(mp.y) >= 1.0 || mp.z < 0.0 || mp.z > 1.0 {
        return 1.0;
    }
    return moment_visibility(moment_filtered(t, smp, texel_inv, region, moment_map_uv(mp.xy)), mp.z, msm, r0, r1, r2, r3);
}

// No cascade, in `cascade_pick`'s answer.
const NO_CASCADE: f32 = 4.0;

// FWD 182-231: Elite's cascades a point `p` reads (all in one space: `a` the light's axes, columns across, across and along;
// the camera at `eye` looking along `dir`; `fade` x the cascades' count, y the floor, z an inner cascade's fade top, w the
// last's): x the first cascade whose box holds it with a weight (`NO_CASCADE`: none), y that weight, z the next such cascade
// if the first's weight is not 1 (else `NO_CASCADE`), w its weight (0 if none).
fn cascade_pick(ahead: vec4<f32>, half: vec4<f32>, fade: vec4<f32>, p: vec3<f32>, a: mat3x3<f32>, eye: vec3<f32>, dir: vec3<f32>) -> vec4<f32> {
    var pick = vec4<f32>(NO_CASCADE, 0.0, NO_CASCADE, 0.0);
    var found = false;
    var second = false;
    let count = u32(fade.x);
    for (var k = 0u; k < count; k++) {
        // (183-187: the fade's floor and top, the last cascade's top its own.)
        let lo = fade.y;
        let hi = select(fade.z, fade.w, k + 1u == count);
        // (188-195: the point in the box's units; inside if within 1 across the light.)
        let q = p - (eye + dir * ahead[k]);
        let b = vec2<f32>(dot(q, a[0]), dot(q, a[1])) / half[k];
        let inside = abs(b.x) <= 1.0 && abs(b.y) <= 1.0;
        // (196-210: the distance to the box's edge, its smoothstep between the floor and the top, a step if they are equal.)
        let edge = min(1.0 - abs(b.x), 1.0 - abs(b.y));
        let t = sat((edge - lo) / (hi - lo));
        let w = select(select(0.0, 1.0, edge >= lo), t * t * (3.0 - 2.0 * t), hi != lo);
        // (211-229: the first with a weight is read; the next with one is kept only while the first's is not 1.)
        if inside && w != 0.0 {
            if !found {
                pick = vec4<f32>(f32(k), w, NO_CASCADE, 0.0);
            } else if !second && pick.y != 1.0 {
                pick = vec4<f32>(pick.xy, f32(k), w);
                second = true;
            }
            found = true;
        }
    }
    return pick;
}

// FWD 578-589: the visibility from the pick's two cascades' (`v1` the first's, `v2` the second's; 1 for a cascade that is
// none or has no map): the second faded to lit by its own weight, the first over it by its weight.
fn cascade_blend(pick: vec4<f32>, v1: f32, v2: f32) -> f32 {
    var second = 0.0;
    if pick.y != 1.0 {
        second = select(v2, 1.0 + pick.w * (v2 - 1.0), pick.w != 1.0);
    }
    return pick.y * (v1 - second) + second;
}
