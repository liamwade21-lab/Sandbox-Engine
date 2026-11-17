// The Sidewinder's shadow (sidewinder/shadow.rs): what its map pass, its pass over the world (sidewinder_shadow.wesl) and the
// ship's own lighting (sidewinder_lit.wesl `star_shadow`) share. No bindings here: each caller binds the record and the maps.
//
// The map: orthographic along the star's direction, in the ship's space, over a sphere round every caster in every pose of
// the gear (`ShipShadow::sphere`, ours), so only the light's direction changes it. Its axes come from the star's direction
// alone (`light_axes`), worked out the same way by every shader that reads it. Its depth `d` runs from 0 on the star's side
// of the sphere to 1 on the far side: the quantity Elite's lookup compares (FWD 352: 1 - the map's z).
//
// The filter is Elite's own for its shadowed directional light (light 0), ported from the reference lit shader
// `ps_Metalness_Roughness_OpaqueDeferred_EmissivePulse_TransparentBox_Forward_1_1_0_0` (re/elite/frea_export/sidewinder/
// decoded/shaders/, lines 233-400; grade C): moment shadow mapping. Each texel holds four moments of its caster's depth in an
// optimised quantisation; a 4 x 4 cubic B-spline filter of them is read with 16 gathers (`filtered_moments`); they are turned
// back (`dequantised`), pulled toward 0.5 by the moment bias, and the visibility at the pixel's depth is the "Hamburger"
// four-moment reconstruction (`visibility`), then Elite's light-leak rescale and smoothstep. Elite's cascades (selection and the
// blend at their borders, lines 182-231 and 578-589) are not needed: one map holds the whole ship. Elite's moment bias and
// leak bound (cb1[261].y, .x) are engine values never found: stand-ins in the record (shadow.rs, the known gap).

// One storage buffer (shadow.rs `ShadowRecord`).
struct ShipShadow {
    // x 1 once this frame's maps are drawn (the map pass sets it after drawing them; the render world writes it 0 first each
    // frame), so nothing reads a map that is not this frame's.
    live: vec4<u32>,
    // The ship's space from the render frame's: p_ship = (dot(rows[0], p), dot(rows[1], p), dot(rows[2], p)), p = (x, y, z, 1).
    to_ship: array<vec4<f32>, 3>,
    // xyz the map's middle in the ship's space, w its radius (m).
    sphere: vec4<f32>,
    // x the map's texels a side, y 1 / that, z a texel in metres, w the depth from which a pixel is the seat's slice.
    map: vec4<f32>,
    // x the lookup's offset along the normal (m), y how far past the furthest caster a pixel must be to be behind the ship
    // (map depth), z the deepest depth a pixel behind the ship is compared at, w the write's slope bias (texels of slope).
    bias: vec4<f32>,
    // x Elite's quantisation offset, y the moment bias (stand-in), z the light-leak bound (stand-in).
    msm: vec4<f32>,
    // Elite's moments from the quantised ones (FWD 346-349), row by row; and the write's, its inverse.
    read: array<vec4<f32>, 4>,
    write: array<vec4<f32>, 4>,
};

fn to_ship_point(s: ShipShadow, p: vec3<f32>) -> vec3<f32> {
    let h = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(s.to_ship[0], h), dot(s.to_ship[1], h), dot(s.to_ship[2], h));
}

fn to_ship_dir(s: ShipShadow, v: vec3<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(dot(s.to_ship[0].xyz, v), dot(s.to_ship[1].xyz, v), dot(s.to_ship[2].xyz, v)));
}

// The map's axes for the star's direction `l` (ship space, towards the star): columns u, v (the map's x and y) and l. Ours.
fn light_axes(l: vec3<f32>) -> mat3x3<f32> {
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(l.y) < 0.99);
    let u = normalize(cross(helper, l));
    return mat3x3<f32>(u, cross(l, u), l);
}

// A point of the ship's space on the map: x, y in -1..1 across the sphere, z the depth d (0 the star's side).
fn map_point(s: ShipShadow, p: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let a = light_axes(l);
    let q = (p - s.sphere.xyz) / s.sphere.w;
    return vec3<f32>(dot(q, a[0]), dot(q, a[1]), 0.5 - 0.5 * dot(q, a[2]));
}

// The map's texture coordinates of map x, y.
fn map_uv(xy: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
}

// The quantised moments of depth `d` (ours: the inverse of Elite's read, so its lookup gets back d, d^2, d^3, d^4).
fn quantised_moments(s: ShipShadow, d: f32) -> vec4<f32> {
    let d2 = d * d;
    let b = vec4<f32>(d, d2, d2 * d, d2 * d2);
    return vec4<f32>(dot(s.write[0], b), dot(s.write[1], b), dot(s.write[2], b), dot(s.write[3], b)) + vec4<f32>(s.msm.x, 0.0, 0.0, 0.0);
}

// FWD 236-241: the cubic B-spline's four weights at the offset f (-0.5..0.5) from the nearest texel corner.
fn bspline(f: f32) -> vec4<f32> {
    var w = f * vec4<f32>(-0.166667, 0.5, -0.5, 0.166667) + vec4<f32>(0.25, -0.25, -0.25, 0.25);
    w = f * w + vec4<f32>(-0.125, -0.625, 0.625, 0.125);
    return saturate(f * w + vec4<f32>(0.020833, 0.479167, 0.479167, 0.020833));
}

// FWD 253-256: one channel of a gather (x the lower left, y lower right, z upper right, w upper left) interpolated: along each
// row by fx, then between the rows by fy.
fn gathered(g: vec4<f32>, fx: f32, fy: f32) -> f32 {
    return mix(mix(g.w, g.z, fx), mix(g.x, g.y, fx), fy);
}

// The four channels' bilinear value at a texel corner `uv` of layer `layer`.
fn gather4(t: texture_2d_array<f32>, smp: sampler, layer: i32, uv: vec2<f32>, fx: f32, fy: f32) -> vec4<f32> {
    let r = textureGather(0, t, smp, uv, layer);
    let g = textureGather(1, t, smp, uv, layer);
    let b = textureGather(2, t, smp, uv, layer);
    let a = textureGather(3, t, smp, uv, layer);
    return vec4<f32>(gathered(r, fx, fy), gathered(g, fx, fy), gathered(b, fx, fy), gathered(a, fx, fy));
}

// FWD 233-344: the moments filtered by the 4 x 4 cubic B-spline round `uv`, as four bilinear reads of 2 x 2 texels each, each
// weighted by its pair of weights' sums.
fn filtered_moments(t: texture_2d_array<f32>, smp: sampler, layer: i32, uv: vec2<f32>, s: ShipShadow) -> vec4<f32> {
    let g = uv * s.map.x;
    let base = floor(g + 0.5);
    let f = g - base;
    let wx = bspline(f.x);
    let wy = bspline(f.y);
    let sx = vec2<f32>(wx.x + wx.y, wx.z + wx.w);
    let sy = vec2<f32>(wy.x + wy.y, wy.z + wy.w);
    let fx = vec2<f32>(wx.y / sx.x, wx.w / sx.y);
    let fy = vec2<f32>(wy.y / sy.x, wy.w / sy.y);
    let lo = (base - 1.0) * s.map.y;
    let hi = (base + 1.0) * s.map.y;
    var m = sy.x * sx.x * gather4(t, smp, layer, lo, fx.x, fy.x);
    m += sy.x * sx.y * gather4(t, smp, layer, vec2<f32>(hi.x, lo.y), fx.y, fy.x);
    m += sy.y * sx.x * gather4(t, smp, layer, vec2<f32>(lo.x, hi.y), fx.x, fy.y);
    m += sy.y * sx.y * gather4(t, smp, layer, hi, fx.y, fy.y);
    return m;
}

// FWD 345-400: the visibility (1 lit) at depth `z` from the filtered quantised moments `q`.
fn visibility(s: ShipShadow, q: vec4<f32>, z: f32) -> f32 {
    let c = q - vec4<f32>(s.msm.x, 0.0, 0.0, 0.0);
    let raw = vec4<f32>(dot(c, s.read[0]), dot(c, s.read[1]), dot(c, s.read[2]), dot(c, s.read[3]));
    // (The moment bias: toward 0.5.)
    let b = mix(raw, vec4<f32>(0.5), s.msm.y);
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
    var v = select(saturate(-quotient), saturate(1.0 - quotient), beyond);
    v = select(1.0, v, z1 < z);
    // Elite's light-leak rescale and smoothstep (396-400; 154-155).
    let x = saturate((v - s.msm.z) / (1.0 - s.msm.z));
    return x * x * (3.0 - 2.0 * x);
}

// Is there any light from the star to take away (`HeroLight::sun`)?
fn star_lit(sun: vec3<f32>) -> bool {
    return max(sun.r, max(sun.g, sun.b)) > 0.0;
}
