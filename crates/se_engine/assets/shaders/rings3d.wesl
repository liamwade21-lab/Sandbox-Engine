// Volumetric rings (`rings3d.rs`): SE's rings_raymarch.glsl (decrypted, re/se_shaders) with its
// rings_common.glh, emu_double.glh and eclipse_common.glh parts, as SE builds it (STONES_ENABLE,
// VOLUMETRIC_DUST_ENABLE, EMULATED_DOUBLES, LAYERS_SHIFT, DEBUG_SPHERE, STONE_NOISE, TEXTURED,
// SMOOTH_SPECULAR, SMOOTH_LIGHTING, DUST_SELF_SHADOW, DUST_RINGS_DETAILED_NOISE, BLEND_MODE 3).
//
// The ring is a grid of cells RocksMaxSize across (SE's SCALE); every cell of a subring holds at most
// one rock, shifted and sized by a hash of the cell, as a sphere far away and SE's noisy stone
// distance field close up, with triplanar PBR from DefaultRingsRockTexture. Each subring's grid turns
// at its own Kepler rate (SE's MarchMap rows 0 and 1, alternate layers offset by
// SubringLayerRotationAdjust), its bounds and density in rows 2 and 3. The camera's place is an
// emulated double (hi + lo floats). Dust is marched in 32 steps through the bounding cylinder with the
// 2D ring's colours, phase functions and self-shadow. Every star is shadowed by the planet (an eclipse
// of its disc, flattened as the planet) and the moons, and the planet lights the rings (SE's host
// light). Port differences: the full screen at full resolution (SE marches at a reduced, checkered
// resolution), the scene depth is the planet's ellipsoid plus the depth test against the written
// depth (SE reads its depth buffer), the blue noise is interleaved gradient noise, the dust's detail
// noise is the 2D ring's, and there is no ambient light (as the port's 2D rings).

import bevy_pbr::render::mesh_view_bindings::view;
import super::eclipse::{EclipseLight, eclipse_shadow, eclipse_value};

struct Rings3D {
    // xyz: the camera in the ring frame (cells), the high floats; w: SE's ONE (1.0)
    ro_hi: vec4<f32>,
    // xyz: the low floats; w: the frame number
    ro_lo: vec4<f32>,
    // rows of the world -> ring rotation; to_ring0.w the eclipse intensity (SE's AmbientColor.a)
    to_ring0: vec4<f32>,
    to_ring1: vec4<f32>,
    to_ring2: vec4<f32>,
    // SE's RingsSize: inner and outer radius, half thickness (cells), 1 / half thickness
    rings_size: vec4<f32>,
    // SE's PlanetParams: x planet radius, y the host light's radius (cells), z cells -> planet radii,
    // w RocksSpacing
    planet: vec4<f32>,
    // SE's Oblateness: 1 / (1 - flattening) per axis
    obl: vec4<f32>,
    // SE's HapkeParams: SpotBright, 1 / SpotWidth, Hapke; w SpotBrightCB
    hapke: vec4<f32>,
    // x 1 / SpotWidthCB, y the outer radius (cells, SE's HapkeParams2.z), z the detail noise frequency,
    // w its fade per cell (SE's DetailParams.y)
    hapke2: vec4<f32>,
    // SE's DensityParams: Density (lit and back rows), Opacity, Opacity x SelfShadow
    density: vec4<f32>,
    // SE's LightingParams: front and back phase scale, stone light scale
    lighting: vec4<f32>,
    // SE's SubringParams: inner and outer adjust, subring count, RingRockMaxSpacing
    subring: vec4<f32>,
    // SE's TexSampleParams: the radius (cells) to the texture coordinate (xy) and the subring (zw)
    tex_sample: vec4<f32>,
    // SE's FadeParams (cells): rocks fade start and end, dust fade start and end
    fade: vec4<f32>,
    // SE's ScaleParams: x km per cell, y cells per km; z metres per cell
    scale: vec4<f32>,
    // The stars in the ring frame (xyz) and their angular radius (w, 0 unused); their light
    light_dir: array<vec4<f32>, 4>,
    light_color: array<vec4<f32>, 4>,
    // SE's host light: the planet's light at the outer edge (rgb), w 1 when on
    host: vec4<f32>,
    eclipse: array<EclipseLight, 4>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> u: Rings3D;
// SE's MarchMap: 4 rows of `subring.z` texels (rotation 1, rotation 2, bounds, density).
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> march: array<vec4<f32>>;
// The 2D ring's texture: back row (BackMap) at v 0.25, lit row (FrontMap) at v 0.75.
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var ring_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var ring_smp: sampler;
// SE's StoneTextureAlbedo, StoneTextureNormal (BC5 xy), StoneTextureRHAO (one layer each).
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var rock_albedo: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var rock_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var rock_normal: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var rock_rhao: texture_2d_array<f32>;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
};

// A screen-covering triangle.
@vertex
fn vertex(v: Vertex) -> VertexOut {
    var o: VertexOut;
    o.clip = vec4<f32>(v.position.xy, 0.0, 1.0);
    return o;
}

const PI: f32 = 3.141592653589793;
const TO_GRAY = vec3<f32>(0.299, 0.587, 0.114);
const CH: f32 = 0.5;
const CH2: f32 = 0.51;
const EPS_CLOSE_DISTANCE: f32 = 0.00025;
const EPS_CLOSEST_DISTANCE: f32 = 0.00001;
const EPS_FAR_DISTANCE: f32 = 0.01;
const EPS_SWITCH_DISTANCE: f32 = 7.0;
const FAR: f32 = 300.0;
const STONES_RAYMARCH_CYCLE_STEPS: i32 = 256;
const DUST_RAYMARCH_CYCLE_STEPS: i32 = 32;
const STONES_INNER_CUT_FIX: f32 = 0.49;
const STONES_OUTER_CUT_FIX: f32 = 0.866;
const STONE_DOWN_SIZE_LIMIT: f32 = 0.25;
const STONE_MIN_SIZE_BUG_FIX: f32 = 0.175;
const NORM_SMP_DIST: f32 = 0.001;
const STONE_NOISE_SCALE_COEF: f32 = 1.4;
const TEXTURE_SWITCH_DISTANCE: f32 = 25.0;
const TEXTURE_TRIPLANAR_BLENDING_REGION: f32 = 0.8;
const TEXTURE_SCALE: f32 = 6.0;
const TEXTURE_MIPMAP_SELECTION_FIX: f32 = 1.0;
const DENSITY_EXP_FALLOFF: f32 = 3.0;
// (1 - exp(-3)) / 3
const FALLOFF_INTEGRAL: f32 = 0.31673764;
const ECLIPSE_SCENE_FAR: f32 = 1.0e30;

fn one() -> f32 {
    return u.ro_hi.w;
}

// --- emu_double.glh -------------------------------------------------------------------------------

struct Df3 {
    hi: vec3<f32>,
    lo: vec3<f32>,
};

fn quick_two_sum(a: f32, b: f32) -> vec2<f32> {
    let s = (a + b) * one();
    let e = b - (s - a) * one();
    return vec2<f32>(s, e);
}

fn quick_two_sum3(a: vec3<f32>, b: vec3<f32>) -> Df3 {
    let s = (a + b) * one();
    let e = b - (s - a) * one();
    return Df3(s, e);
}

fn two_sum3(a: vec3<f32>, b: vec3<f32>) -> Df3 {
    let s = (a + b) * one();
    let v = s - a;
    let e = (a - (s - v) * one()) * one() + (b - v);
    return Df3(s, e);
}

fn two_sum_comp(a: vec2<f32>, b: vec2<f32>) -> vec4<f32> {
    let s = (a + b) * one();
    let v = s - a;
    let e = (a - (s - v) * one()) * one() + (b - v);
    return vec4<f32>(s.x, e.x, s.y, e.y);
}

fn two_diff3(a: vec3<f32>, b: vec3<f32>) -> Df3 {
    let s = a - b;
    let v = s - a;
    let err = (a - (s - v)) - (b + v);
    return Df3(s, err);
}

// SE's twoProd relies on the compiler fusing `a * b - x`; the fma says so.
fn two_prod(a: f32, b: f32) -> vec2<f32> {
    let x = (a * b) * one();
    return vec2<f32>(x, fma(a, b, -x));
}

fn df64_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    var p = two_prod(a.x, b.x);
    p.y += a.x * b.y;
    p.y += a.y * b.x;
    return quick_two_sum(p.x, p.y);
}

fn df64_add(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    var st = two_sum_comp(a, b);
    st.y += st.z;
    let q = quick_two_sum(st.x, st.y);
    return quick_two_sum(q.x, q.y + st.w);
}

fn df64_add3(a: Df3, b: vec3<f32>) -> Df3 {
    var s = two_sum3(a.hi, b);
    s.lo += a.lo;
    return quick_two_sum3(s.hi, s.lo);
}

fn df64_diff_slow3(a: Df3, b: Df3) -> Df3 {
    var s = two_diff3(a.hi, b.hi);
    let t = two_diff3(a.lo, b.lo);
    s.lo += t.hi;
    s = quick_two_sum3(s.hi, s.lo);
    s.lo += t.lo;
    return quick_two_sum3(s.hi, s.lo);
}

fn df64_floor3(a: Df3) -> Df3 {
    var hi = floor(a.hi);
    var lo = floor(a.lo);
    for (var c = 0; c < 3; c++) {
        if hi[c] == a.hi[c] {
            let t = quick_two_sum(hi[c], lo[c]);
            hi[c] = t.x;
            lo[c] = t.y;
        } else {
            lo[c] = 0.0;
        }
    }
    return Df3(hi, lo);
}

// --- shapes ---------------------------------------------------------------------------------------

fn ihash43(p: vec3<i32>) -> vec4<f32> {
    let ui4 = vec4<u32>(1597334673u, 3812015801u, 2798796415u, 3299493293u);
    var q = vec4<u32>(vec4<i32>(p.xyz, p.x)) * ui4;
    q = vec4<u32>(q.x ^ q.y ^ q.z ^ q.w) * ui4;
    return vec4<f32>(q) * (1.0 / f32(0xffffffffu));
}

fn d_box(ro: vec3<f32>, rd: vec3<f32>) -> f32 {
    let m = 1.0 / rd;
    let t = -m * ro + abs(m) * CH2;
    return min(min(t.x, t.y), t.z);
}

// SE's RayCylinderIntersect with the bounding cylinder (RingsSize.y + 1 wide, RingsSize.z + 1 high).
fn ray_cylinder(org: vec3<f32>, dir: vec3<f32>, nf: ptr<function, vec2<f32>>) -> bool {
    let radius = u.rings_size.y + 1.0;
    let height = u.rings_size.z + 1.0;
    let a = dot(dir.xz, dir.xz);
    let b = dot(org.xz, dir.xz);
    let c = dot(org.xz, org.xz) - radius * radius;
    let delta = b * b - a * c;
    if delta < 0.0 {
        return false;
    }
    let ds = sqrt(delta);
    let arcp = 1.0 / a;
    var near = (-b - ds) * arcp;
    var far = (-b + ds) * arcp;
    let temp = min(far, near);
    far = max(far, near);
    near = temp;
    let znear = org.y + near * dir.y;
    let zfar = org.y + far * dir.y;
    let zcap = vec2<f32>(height, -height);
    let cap = (zcap - org.y) / dir.y;
    if znear < zcap.y {
        near = cap.y;
    } else if znear > zcap.x {
        near = cap.x;
    }
    if zfar < zcap.y {
        far = cap.y;
    } else if zfar > zcap.x {
        far = cap.x;
    }
    *nf = vec2<f32>(near, far);
    return far > 0.0 && far > near;
}

fn sd_disk(p: vec3<f32>, t: vec3<f32>) -> f32 {
    let l1 = length(p.xz);
    let t1 = max(l1 - t.y, t.x - l1);
    let d = vec2<f32>(t1, abs(p.y) - t.z);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0)));
}

fn sd_box(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let d = abs(p) - b;
    return min(max(d.x, max(d.y, d.z)), 0.0) + length(max(d, vec3<f32>(0.0)));
}

// SE's sdStone (its `time` is the constant 1).
fn sd_stone(p_in: vec3<f32>, rad: vec2<f32>, rnd: vec4<f32>) -> f32 {
    var p = p_in / rad.y;
    p.x += sin(p.y * 6.0 * rnd.x + 1.0 + sin(p.z)) * 0.15 * rnd.x;
    var d = length(p) - rad.x;
    let st = sin(0.62 * rnd.y) * 0.5 + 0.5;
    let frq = clamp(rnd.w * 20.0, 6.0, 12.0);
    d += sin(p.x * frq + 0.3 + sin(p.z * frq * rnd.z + 0.5 + sin(p.y * frq + 0.7 * rnd.z))) * 0.075 * st;
    return d * rad.y;
}

fn stone_normal(p: vec3<f32>, rad: vec2<f32>, rnd: vec4<f32>) -> vec3<f32> {
    let h = NORM_SMP_DIST;
    let k = vec2<f32>(1.0, -1.0);
    return normalize(k.xyy * sd_stone(p + k.xyy * h, rad, rnd) + k.yyx * sd_stone(p + k.yyx * h, rad, rnd)
        + k.yxy * sd_stone(p + k.yxy * h, rad, rnd) + k.xxx * sd_stone(p + k.xxx * h, rad, rnd));
}

// SE's Stone_raymarch: the march's step counter is shared with the cell walk.
fn stone_raymarch(ro: vec3<f32>, rd: vec3<f32>, mint: f32, maxt: f32, rad: vec2<f32>, rnd: vec4<f32>, p: ptr<function, vec3<f32>>, hit: ptr<function, bool>, i: ptr<function, i32>, eps: f32) -> f32 {
    var t = mint;
    loop {
        if *i >= STONES_RAYMARCH_CYCLE_STEPS {
            break;
        }
        if t > maxt {
            *hit = false;
            break;
        }
        *p = ro + rd * t;
        let res = sd_stone(*p, rad, rnd);
        t += res;
        if abs(res) < eps {
            *hit = true;
            return t;
        }
        *i += 1;
    }
    return -1.0;
}

// --- triplanar textures ---------------------------------------------------------------------------

fn rnm_blend_unpacked(n1_in: vec3<f32>, n2: vec3<f32>) -> vec3<f32> {
    let n1 = n1_in + vec3<f32>(0.0, 0.0, 1.0);
    return n1 * dot(n1, n2) / n1.z - n2;
}

fn unpack_normal(t: vec2<f32>) -> vec3<f32> {
    let norm_one = 1.0 - 1.0 / 255.0;
    let rg = 2.0 * t - norm_one;
    return vec3<f32>(rg, sqrt(clamp(1.0 - dot(rg, rg), 0.0, 1.0)));
}

fn triplanar_weights(normal: vec3<f32>) -> vec3<f32> {
    var w = normal * normal;
    let max_blend = max(w.x, max(w.y, w.z));
    w = max(w - max_blend * TEXTURE_TRIPLANAR_BLENDING_REGION, vec3<f32>(0.0));
    return w / (w.x + w.y + w.z);
}

fn triplanar_rnm(pos: vec3<f32>, normal: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>) -> vec3<f32> {
    let blend = triplanar_weights(normal);
    let abs_n = abs(normal);
    let sgn = sign(normal);
    var world = vec3<f32>(0.0);
    if blend.x > 0.0 {
        var t = unpack_normal(textureSampleGrad(rock_normal, rock_smp, pos.zy, 0, dx.zy, dy.zy).rg);
        t = rnm_blend_unpacked(vec3<f32>(normal.zy, abs_n.x), t);
        t.z *= sgn.x;
        world = t.zyx * blend.x;
    }
    if blend.y > 0.0 {
        var t = unpack_normal(textureSampleGrad(rock_normal, rock_smp, pos.xz, 0, dx.xz, dy.xz).rg);
        t = rnm_blend_unpacked(vec3<f32>(normal.xz, abs_n.y), t);
        t.z *= sgn.y;
        world += t.xzy * blend.y;
    }
    if blend.z > 0.0 {
        var t = unpack_normal(textureSampleGrad(rock_normal, rock_smp, pos.xy, 0, dx.xy, dy.xy).rg);
        t = rnm_blend_unpacked(vec3<f32>(normal.xy, abs_n.z), t);
        t.z *= sgn.z;
        world += t.xyz * blend.z;
    }
    return normalize(world + normal);
}

fn triplanar_albedo(pos: vec3<f32>, normal: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>) -> vec3<f32> {
    let blend = triplanar_weights(normal);
    var c = vec3<f32>(0.0);
    if blend.x > 0.0 {
        c = textureSampleGrad(rock_albedo, rock_smp, pos.zy, 0, dx.zy, dy.zy).rgb * blend.x;
    }
    if blend.y > 0.0 {
        c += textureSampleGrad(rock_albedo, rock_smp, pos.xz, 0, dx.xz, dy.xz).rgb * blend.y;
    }
    if blend.z > 0.0 {
        c += textureSampleGrad(rock_albedo, rock_smp, pos.xy, 0, dx.xy, dy.xy).rgb * blend.z;
    }
    return c;
}

fn triplanar_rhao(pos: vec3<f32>, normal: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>) -> vec3<f32> {
    let blend = triplanar_weights(normal);
    var c = vec3<f32>(0.0);
    if blend.x > 0.0 {
        c = textureSampleGrad(rock_rhao, rock_smp, pos.zy, 0, dx.zy, dy.zy).rgb * blend.x;
    }
    if blend.y > 0.0 {
        c += textureSampleGrad(rock_rhao, rock_smp, pos.xz, 0, dx.xz, dy.xz).rgb * blend.y;
    }
    if blend.z > 0.0 {
        c += textureSampleGrad(rock_rhao, rock_smp, pos.xy, 0, dx.xy, dy.xy).rgb * blend.z;
    }
    return c;
}

fn calc_dp_dxy(rd: vec3<f32>, rdx: vec3<f32>, rdy: vec3<f32>, t: f32, nor: vec3<f32>, dpdx: ptr<function, vec3<f32>>, dpdy: ptr<function, vec3<f32>>) {
    *dpdx = t * (rdx * dot(rd, nor) / dot(rdx, nor) - rd);
    *dpdy = t * (rdy * dot(rd, nor) / dot(rdy, nor) - rd);
}

// --- the cell walk (SE's map_edf64) ---------------------------------------------------------------

struct Subring {
    rot1: vec4<f32>,
    rot2: vec4<f32>,
    // the disk's inner and outer radius and half height (cells)
    disk: vec3<f32>,
    dens: vec4<f32>,
};

struct Hit {
    t: f32,
    hit: bool,
    p: vec3<f32>,
    norm: vec3<f32>,
    norm_rot: vec3<f32>,
};

// GLSL's `v.xz *= mat2(c, s, -s, c)` (SE's RotMat; `-s` gives its invRotMat).
fn rot3(v: vec3<f32>, c: f32, s: f32) -> vec3<f32> {
    return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

fn map_edf64(p: Df3, rd_in: vec3<f32>, rdx_in: vec3<f32>, rdy_in: vec3<f32>, i: ptr<function, i32>, stone_surface_dist: f32, albedo: ptr<function, vec3<f32>>, rhao: ptr<function, vec3<f32>>, sub: Subring, eps_in: f32) -> Hit {
    var out: Hit;
    out.hit = false;
    out.norm = vec3<f32>(1.0);
    out.norm_rot = vec3<f32>(1.0);
    var q = p;
    var eps = eps_in;
    var stone = 3.0;
    // LAYERS_SHIFT: odd layers use the second rotation.
    let layer = floor(q.hi.y);
    let data_rot = select(sub.rot2, sub.rot1, layer - 2.0 * floor(layer * 0.5) == 0.0);
    let c = data_rot.x;
    let s = data_rot.y;
    let cos_a = vec2<f32>(data_rot.x, data_rot.z);
    let sin_a = vec2<f32>(data_rot.y, data_rot.w);
    let nsin_a = vec2<f32>(-data_rot.y, -data_rot.w);
    let t1 = df64_mul(vec2<f32>(q.hi.x, q.lo.x), cos_a);
    let t2 = df64_mul(vec2<f32>(q.hi.z, q.lo.z), sin_a);
    let t3 = df64_mul(vec2<f32>(q.hi.x, q.lo.x), nsin_a);
    let t4 = df64_mul(vec2<f32>(q.hi.z, q.lo.z), cos_a);
    let q_x = select(df64_add(t2, t1), df64_add(t1, t2), abs(t1.x) > abs(t2.x));
    let q_z = select(df64_add(t4, t3), df64_add(t3, t4), abs(t3.x) > abs(t4.x));
    q.hi.x = q_x.x;
    q.lo.x = q_x.y;
    q.hi.z = q_z.x;
    q.lo.z = q_z.y;
    let rd = rot3(rd_in, c, s);
    let tmp_floor = df64_floor3(q);
    let tmp_floor2 = df64_diff_slow3(q, tmp_floor);
    let qd = (tmp_floor2.hi + tmp_floor2.lo) - CH;
    let qid = (tmp_floor.hi + tmp_floor.lo) + CH;
    var qd_shifted = vec3<f32>(0.0);
    var t = d_box(qd, rd);
    let disk_dist = sd_disk(qid, sub.disk);
    if disk_dist < 0.0 {
        var rnd = ihash43(vec3<i32>(tmp_floor.hi + vec3<f32>(0.0, 1e4, 0.0)) + vec3<i32>(tmp_floor.lo));
        rnd = rnd * 2.0 - 1.0;
        rnd.w *= sub.dens.w;
        let shift = rnd.xyz * rnd.w;
        let shift_len2 = dot(shift, shift);
        if shift_len2 < STONES_INNER_CUT_FIX * STONES_INNER_CUT_FIX && disk_dist <= -STONES_OUTER_CUT_FIX && rnd.w > abs(STONE_DOWN_SIZE_LIMIT) {
            qd_shifted = qd - shift;
            let border = abs(sd_box(shift, vec3<f32>(0.49)));
            if sub.dens.w <= u.subring.w {
                // DEBUG_SPHERE: the rock's bounding sphere (half size far away).
                let oc = qd_shifted;
                let b = dot(oc, normalize(rd));
                var cc = dot(oc, oc);
                let to_sphere = u.fade.x * 0.5;
                if stone_surface_dist < to_sphere {
                    cc -= border * border;
                } else {
                    cc -= border * border * 0.25;
                }
                var h1 = b * b - cc;
                if h1 > 0.0 {
                    h1 = sqrt(h1);
                    let far = -b + h1;
                    let near = -b - h1;
                    if far > 0.0 {
                        if stone_surface_dist < to_sphere {
                            // STONE_NOISE: march the rock's own distance field.
                            let border2 = clamp(border * STONE_NOISE_SCALE_COEF, STONE_MIN_SIZE_BUG_FIX, 0.5);
                            if stone_surface_dist <= 0.0 && near < 0.0 {
                                eps = EPS_CLOSEST_DISTANCE;
                            }
                            let rad = vec2<f32>(border, border2);
                            var hit = false;
                            stone = stone_raymarch(qd - shift, rd, max(near, 0.0), far, rad, rnd, &qd_shifted, &hit, i, eps);
                            if hit && t > 0.0 {
                                if stone < 0.0 && stone_surface_dist <= 0.0 {
                                    hit = false;
                                } else {
                                    t = max(near, 0.0) + stone;
                                    let n = stone_normal(qd_shifted, rad, rnd);
                                    var nr = n;
                                    if stone_surface_dist < TEXTURE_SWITCH_DISTANCE {
                                        var dpdx: vec3<f32>;
                                        var dpdy: vec3<f32>;
                                        calc_dp_dxy(rd, rot3(rdx_in, c, s), rot3(rdy_in, c, s), stone_surface_dist + max(near, 0.0) + stone, n, &dpdx, &dpdy);
                                        let tp = qd_shifted * TEXTURE_SCALE;
                                        nr = triplanar_rnm(tp, n, dpdx * TEXTURE_SCALE, dpdy * TEXTURE_SCALE);
                                        *rhao = triplanar_rhao(tp, n, dpdx * TEXTURE_SCALE, dpdy * TEXTURE_SCALE);
                                        *albedo = triplanar_albedo(tp, n, dpdx * TEXTURE_SCALE, dpdy * TEXTURE_SCALE);
                                    }
                                    // LAYERS_SHIFT: back from the cell's frame (invRotMat).
                                    out.norm = rot3(n, c, -s);
                                    out.norm_rot = rot3(nr, c, -s);
                                }
                            } else {
                                out.norm = vec3<f32>(1.0);
                            }
                            out.hit = hit;
                        } else {
                            out.hit = true;
                            stone = near;
                            t = stone;
                            qd_shifted = qd - shift + rd * near;
                            out.norm = rot3(normalize(qd_shifted), c, -s);
                            out.norm_rot = out.norm;
                        }
                    }
                }
            }
        }
    }
    out.p = qd_shifted;
    out.t = t;
    return out;
}

// --- dust -----------------------------------------------------------------------------------------

fn subring_at(i: i32) -> Subring {
    let n = i32(u.subring.z);
    let k = clamp(i, 0, n - 1);
    var s: Subring;
    s.rot1 = march[k];
    s.rot2 = march[n + k];
    let size = floor(march[2 * n + k] * u.scale.y);
    s.dens = march[3 * n + k];
    s.disk = vec3<f32>(size.x + 2.0, size.y - 1.0, max(floor(s.dens.x * u.rings_size.z), 1.0));
    s.dens.w *= u.planet.w;
    return s;
}

fn hash1(i: f32) -> f32 {
    return fract(sin(i * 127.1 + 311.7) * 43758.5453);
}

fn noise1(x: f32) -> f32 {
    let i = floor(x);
    let f = x - i;
    let w = f * f * (3.0 - 2.0 * f);
    return mix(hash1(i), hash1(i + 1.0), w);
}

// The 2D ring's radial detail noise (rings.wesl `detail_noise`).
fn detail_noise(x: f32) -> f32 {
    var sum = 0.0;
    var p = x;
    var amp = 1.0;
    for (var i = 0; i < 4; i++) {
        sum += amp * noise1(p);
        p *= 2.218281828459;
        amp *= 0.671415927;
    }
    return sum * 0.85 / 2.4;
}

fn lambert_brdf(mu: f32) -> f32 {
    return (mu + 1.0) * 0.5;
}

fn hapke_brdf(mu: f32) -> f32 {
    let t = sqrt((1.0 - mu) / (1.0 + mu));
    let s = 1.0 + u.hapke.x / (1.0 + t * u.hapke.y) + u.hapke.w / (1.0 + t * u.hapke2.x);
    return mix(1.0, s * 0.5, u.hapke.z);
}

fn mie_brdf(mu: f32, g: f32) -> f32 {
    let k = 1.5 / 806.202 * ((1.0 - g * g) / (2.0 + g * g));
    return k * (1.0 + mu * mu) * pow(1.0 + g * g - 2.0 * g * mu, -1.5);
}

fn front_color(fetch_uf: f32) -> vec4<f32> {
    return textureSampleLevel(ring_tex, ring_smp, vec2<f32>(fetch_uf, 0.75), 0.0);
}

fn back_color(fetch_uf: f32) -> vec4<f32> {
    return textureSampleLevel(ring_tex, ring_smp, vec2<f32>(fetch_uf, 0.25), 0.0);
}

fn falloff_at(p: vec3<f32>) -> f32 {
    return exp(-DENSITY_EXP_FALLOFF * abs(p.y) * u.rings_size.w);
}

fn optical_density(c: vec4<f32>, ray_length: f32) -> vec4<f32> {
    return -log(c) / (ray_length * FALLOFF_INTEGRAL);
}

fn volumetric_shadow(p0: vec3<f32>, p1: vec3<f32>, tau: f32) -> f32 {
    let dv = p1 - p0;
    return exp(-tau * falloff_at(p0 + 0.5 * dv) * length(dv));
}

// SE's MainPlanetShadow for a distant light from `l`: an eclipse of its disc by the planet's.
fn main_planet_shadow(p_length: f32, p: vec3<f32>, l: vec3<f32>, lar: f32) -> f32 {
    var lp = l;
    lp.y *= u.obl.y;
    let ld = length(lp);
    let caster = asin(clamp(u.planet.x / p_length, 0.0, 1.0));
    let angle = acos(clamp(-dot(p, lp) / (p_length * ld), 0.0, 1.0));
    return clamp(1.0 - eclipse_value(lar, caster, angle), 0.0, 1.0);
}

fn ring_to_world(v: vec3<f32>) -> vec3<f32> {
    return u.to_ring0.xyz * v.x + u.to_ring1.xyz * v.y + u.to_ring2.xyz * v.z;
}

// The moons' eclipse of light `j` at `p` (SE's EclipseShadowRingDustAuto / ...StonesAuto).
fn moon_shadow(j: i32, p: vec3<f32>, l: vec3<f32>) -> f32 {
    let e = u.eclipse[j];
    if e.info.y < 0.5 {
        return 1.0;
    }
    return 1.0 - u.to_ring0.w * eclipse_shadow(e, ring_to_world(p) * u.planet.z, ring_to_world(l));
}

// SE's host light seen from `p`: the planet's phase from there and its disc (`l0` the first star).
fn host_shadow(p: vec3<f32>, l0: vec3<f32>, l_out: ptr<function, vec3<f32>>) -> f32 {
    let inv = inverseSqrt(dot(p, p));
    let l = -p * inv;
    *l_out = l;
    let dist_corr = u.hapke2.y * inv;
    var shadow = dist_corr * dist_corr;
    let cos_chi = clamp(-dot(l, l0), -0.999999, 0.999999);
    let chi = acos(cos_chi);
    shadow *= (1.0 - chi / PI) * cos_chi + sqrt(1.0 - cos_chi * cos_chi) / PI;
    let ang = asin(clamp(u.planet.y * inv, 0.0, 0.999999));
    return shadow * clamp(PI - ang - chi, 0.0, 1.0);
}

fn volumetric_dust(org: vec3<f32>, dir: vec3<f32>, mm: vec2<f32>, stones_depth: f32, random_seed: f32, scene_depth: f32, hit_scene_depth: bool, hit_stones: bool) -> vec4<f32> {
    var color = vec3<f32>(0.0);
    var transp = vec3<f32>(1.0);
    var total_distance = 0.0;
    var total_in_dust = 0.0;
    let min_dist = mm.x;
    let max_dist = mm.y;
    let start = max(min_dist, 0.0);
    var p = org + start * dir;
    total_distance += start;
    var end = min(length(p), max_dist);
    if min_dist != 0.0 {
        end = min(end, u.fade.w);
    }
    let blend_end = mix(stones_depth, min(scene_depth, min(max_dist, u.fade.y + 2e3)), smoothstep(u.fade.x, u.fade.y, stones_depth));
    if hit_stones && stones_depth > u.fade.x && stones_depth < scene_depth {
        end = min(blend_end, 2.5e4);
    }
    if (min_dist == 0.0 && hit_stones && stones_depth < u.fade.y) || (min_dist != 0.0 && hit_stones && stones_depth < u.fade.x) || hit_scene_depth {
        end = min(blend_end, 2.5e4);
    }
    if !hit_stones && min_dist == 0.0 {
        end = min(end, min(scene_depth, 2.5e4));
    }
    let expected = end - start;
    let step_s = expected / f32(DUST_RAYMARCH_CYCLE_STEPS);
    let dir_step = dir * step_s;
    p += dir_step * random_seed;
    total_in_dust += step_s * random_seed;
    total_distance += total_in_dust;
    let fade = step(total_distance, u.fade.z);
    var l0 = vec3<f32>(0.0, 1.0, 0.0);
    for (var i = 0; i < DUST_RAYMARCH_CYCLE_STEPS; i++) {
        if total_distance > end || total_distance - total_in_dust > u.rings_size.x || total_distance > u.rings_size.x || total_in_dust > expected {
            break;
        }
        let p_dist = length(p.xz);
        let fetch_uf = p_dist * u.tex_sample.x + u.tex_sample.y;
        let detail = clamp(1.0 - total_distance * u.hapke2.w, 0.0, 1.0);
        let fade_to_stones = smoothstep(0.0, u.fade.y, total_distance);
        // SE's DustDensityDetail.
        var front = front_color(fetch_uf);
        front = vec4<f32>(front.rgb * front.a, front.a);
        var back = back_color(fetch_uf);
        back.a = front.a;
        if detail > 0.0 {
            let m = detail_noise(fetch_uf * u.hapke2.z);
            let e_ring = smoothstep(0.0, 0.01, front.a);
            back = vec4<f32>(back.rgb * mix(1.0, m, 0.5 * detail * e_ring), back.a);
            front *= mix(1.0, m, 0.5 * detail);
        }
        let falloff = falloff_at(p);
        let dens = clamp(1.0 - vec4<f32>(dot(front.rgb, TO_GRAY), dot(back.rgb, TO_GRAY), front.a, back.a) * u.density, vec4<f32>(0.01), vec4<f32>(1.0));
        var od = optical_density(dens, u.rings_size.z * 2.0);
        od.x *= fade_to_stones;
        od.z *= fade_to_stones;
        od *= falloff;
        var lit_front = vec3<f32>(0.0);
        var lit_back = vec3<f32>(0.0);
        for (var j = 0; j < 4; j++) {
            let ld = u.light_dir[j];
            if ld.w <= 0.0 {
                continue;
            }
            let l = ld.xyz;
            if j == 0 {
                l0 = l;
            }
            let mu = clamp(-dot(l, dir), -0.999999, 1.0);
            let phase_back = u.lighting.y * mie_brdf(mu, -0.95);
            var shadow = main_planet_shadow(p_dist, p, l, ld.w) * moon_shadow(j, p, l);
            var nf = mm;
            if ray_cylinder(p, l, &nf) {
                shadow *= volumetric_shadow(p, p + l * nf.y, od.w);
            }
            let phase_front = u.lighting.x * hapke_brdf(mu) * lambert_brdf(mu);
            lit_front += u.light_color[j].rgb * front.rgb * phase_front * shadow;
            lit_back += u.light_color[j].rgb * back.rgb * phase_back * shadow;
        }
        if u.host.w > 0.5 {
            var l: vec3<f32>;
            let shadow = host_shadow(p, l0, &l);
            let mu = clamp(-dot(l, dir), -0.999999, 1.0);
            lit_front += u.host.rgb * front.rgb * (u.lighting.x * hapke_brdf(mu) * lambert_brdf(mu)) * shadow;
            lit_back += u.host.rgb * back.rgb * (u.lighting.y * mie_brdf(mu, -0.7)) * shadow;
        }
        let ext = exp(-step_s * od.xyz);
        color += lit_front * (1.0 - ext.x) * transp.x + lit_back * (1.0 - ext.y) * transp.y;
        transp *= ext;
        p += dir_step;
        total_distance += step_s;
        total_in_dust += step_s;
    }
    return vec4<f32>(color * fade, (1.0 - transp.z) * fade);
}

// --- rocks ----------------------------------------------------------------------------------------

struct StonesOut {
    pc: vec4<f32>,
    hit_scene_depth: bool,
    transparency: bool,
    depth: f32,
    hit: bool,
};

fn ggx(n_dot_h: f32, r4: f32) -> f32 {
    let d = n_dot_h * n_dot_h * (r4 - 1.0) + 1.0;
    return r4 / (PI * d * d);
}

fn schlick_ggx(n_dot_v: f32, rk: f32) -> f32 {
    return n_dot_v / (n_dot_v * (1.0 - rk) + rk);
}

fn specular_occlusion(ao: f32, n_dot_v: f32, v_dot_l: f32, dist: f32) -> f32 {
    let d = ao - n_dot_v;
    var occlude = 1.0 - clamp(d * d - 1.0 + ao, 0.0, 1.0);
    // SMOOTH_SPECULAR
    let dist_fade = smoothstep(u.fade.x * 0.1, u.fade.x * 0.05, dist);
    occlude *= mix(smoothstep(0.85, 0.7, v_dot_l), 1.0, dist_fade);
    return occlude;
}

// SE's PBR stone light from direction `l`, with `incoming` its light (shadowed).
fn stone_light(n: vec3<f32>, rd: vec3<f32>, l: vec3<f32>, albedo: vec3<f32>, rough: f32, ao: f32, t: f32, incoming: vec3<f32>) -> vec3<f32> {
    let r2 = rough * rough;
    let r4 = r2 * r2;
    let r1 = rough + 1.0;
    let rk = r1 * r1 * 0.125;
    let f0 = vec3<f32>(0.04);
    let h = normalize(l - rd);
    let v_dot_h = clamp(-dot(rd, h), 0.0, 1.0);
    let n_dot_v = clamp(-dot(n, rd), 0.0, 1.0);
    let n_dot_l = clamp(dot(n, l), 0.0, 1.0);
    let v_dot_l = clamp(dot(rd, l), -0.999999, 1.0);
    let n_dot_h = clamp(dot(n, h), 0.0, 1.0);
    let f = f0 + (1.0 - f0) * pow(clamp(1.0 - v_dot_h, 0.0, 1.0), 5.0);
    var spec = ggx(n_dot_h, r4) * schlick_ggx(n_dot_v, rk) * schlick_ggx(n_dot_l, rk) * f / (4.0 * n_dot_v * n_dot_l + 0.001);
    spec *= specular_occlusion(ao, n_dot_v, v_dot_l, t);
    let diffuse = (vec3<f32>(1.0) - f) / PI * albedo;
    var lambert = n_dot_l;
    // SMOOTH_LIGHTING: far rocks lit as a Lambert sphere's phase.
    lambert = mix(lambert, (1.0 - v_dot_l) * 0.5, smoothstep(u.fade.x * 0.5, u.fade.x * 0.8, t));
    let light = lambert * u.lighting.z * incoming;
    return diffuse * light + spec * light;
}

fn stones(ero: Df3, rd: vec3<f32>, rdx: vec3<f32>, rdy: vec3<f32>, mm: vec2<f32>, scene_depth: f32, tex_avg: vec4<f32>) -> StonesOut {
    var o: StonesOut;
    o.pc = vec4<f32>(0.0);
    o.hit_scene_depth = false;
    o.hit = false;
    o.transparency = false;
    o.depth = -1.0;
    let ro = ero.hi + ero.lo;
    var albedo = tex_avg.rgb;
    var rhao = vec3<f32>(tex_avg.a, 0.5, 1.0);
    let min_dist = max(mm.x, 0.0);
    let max_dist = mm.y;
    var t = min_dist;
    var mint = -1.0;
    var p = ro;
    var norm_rot = vec3<f32>(1.0);
    var i = 0;
    loop {
        if i >= STONES_RAYMARCH_CYCLE_STEPS {
            break;
        }
        let pf = ro + rd * t;
        let fetch_ui = i32(floor(length(pf.xz) * u.tex_sample.z + u.tex_sample.w));
        let sub = subring_at(fetch_ui);
        let eps = mix(EPS_CLOSE_DISTANCE, EPS_FAR_DISTANCE, smoothstep(0.0, EPS_SWITCH_DISTANCE, t));
        let pe = df64_add3(ero, rd * t);
        p = pe.hi + pe.lo;
        let h = map_edf64(pe, rd, rdx, rdy, &i, t, &albedo, &rhao, sub, eps);
        if h.hit {
            t += h.t;
            mint = t;
            norm_rot = h.norm_rot;
            o.hit = true;
            break;
        }
        if t > scene_depth {
            t = scene_depth;
            mint = t;
            o.pc.a = 1.0;
            o.hit_scene_depth = true;
            break;
        }
        t += h.t;
        if t > max_dist || t > min_dist + FAR || min_dist > u.fade.y {
            mint = -1.0;
            break;
        }
        i += 1;
    }
    if mint > 0.0 && !o.hit_scene_depth {
        let pf = ro + rd * t;
        let p_dist = length(pf.xz);
        let fetch_uf = p_dist * u.tex_sample.x + u.tex_sample.y;
        let front = front_color(fetch_uf);
        let falloff = falloff_at(pf);
        let density = clamp(1.0 - front.a * u.density.w, 0.01, 1.0);
        let od = -log(density) / (u.rings_size.z * 2.0 * FALLOFF_INTEGRAL) * falloff;
        o.pc.a = 1.0;
        // TEXTURED
        let col = front.rgb * pow(albedo, vec3<f32>(1.0 / 2.2));
        let rough = rhao.x;
        let ao = rhao.z;
        var lit = vec3<f32>(0.0);
        var l0 = vec3<f32>(0.0, 1.0, 0.0);
        for (var j = 0; j < 4; j++) {
            let ld = u.light_dir[j];
            if ld.w <= 0.0 {
                continue;
            }
            let l = ld.xyz;
            if j == 0 {
                l0 = l;
            }
            var shadow = main_planet_shadow(p_dist, pf, l, ld.w) * moon_shadow(j, pf, l);
            var nf = mm;
            if ray_cylinder(p, l, &nf) {
                shadow *= volumetric_shadow(p, p + l * nf.y, od);
            }
            lit += stone_light(norm_rot, rd, l, col, rough, ao, t, shadow * u.light_color[j].rgb);
        }
        if u.host.w > 0.5 {
            var l: vec3<f32>;
            let shadow = host_shadow(pf, l0, &l);
            lit += stone_light(norm_rot, rd, l, col, rough, ao, t, shadow * u.host.rgb);
        }
        o.pc = vec4<f32>(lit, 1.0);
    }
    if mint > u.fade.x {
        o.pc.a = smoothstep(u.fade.y, u.fade.x, mint);
        o.transparency = true;
    }
    if max_dist > scene_depth && mint < 0.0 {
        o.hit_scene_depth = true;
        o.depth = scene_depth;
    } else {
        o.depth = mint;
    }
    return o;
}

// --- the planet (SE's scene depth) ----------------------------------------------------------------

// Where the ray first meets the planet's ellipsoid (cells), or ECLIPSE_SCENE_FAR.
fn planet_depth(ro: vec3<f32>, rd: vec3<f32>) -> f32 {
    let o = ro * vec3<f32>(1.0, u.obl.y, 1.0);
    let d = rd * vec3<f32>(1.0, u.obl.y, 1.0);
    let a = dot(d, d);
    let b = dot(o, d);
    let c = dot(o, o) - u.planet.x * u.planet.x;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return ECLIPSE_SCENE_FAR;
    }
    let t = (-b - sqrt(disc)) / a;
    return select(ECLIPSE_SCENE_FAR, t, t > 0.0);
}

// The view ray through `frag` (world axes).
fn world_ray(frag: vec2<f32>) -> vec3<f32> {
    let vp = view.viewport;
    let ndc = vec2<f32>((frag.x - vp.x) / vp.z * 2.0 - 1.0, 1.0 - (frag.y - vp.y) / vp.w * 2.0);
    let p = view.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(p.xyz / p.w - view.world_position);
}

fn to_ring(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(u.to_ring0.xyz, v), dot(u.to_ring1.xyz, v), dot(u.to_ring2.xyz, v));
}

struct FragOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOut) -> FragOut {
    let frag = in.clip.xy;
    let dir_w = world_ray(frag);
    let rd = normalize(to_ring(dir_w));
    let rdx = normalize(to_ring(world_ray(frag + vec2<f32>(TEXTURE_MIPMAP_SELECTION_FIX, 0.0))));
    let rdy = normalize(to_ring(world_ray(frag + vec2<f32>(0.0, TEXTURE_MIPMAP_SELECTION_FIX))));
    let ero = Df3(u.ro_hi.xyz, u.ro_lo.xyz);
    let ro = ero.hi + ero.lo;
    let scene_depth = planet_depth(ro, rd);
    // Interleaved gradient noise, a new pattern each frame (SE: BlueNoiseMap at NoiseParams).
    let fr = frag + 5.588238 * (u.ro_lo.w % 64.0);
    let random_seed = fract(52.9829189 * fract(dot(fr, vec2<f32>(0.06711056, 0.00583715))));
    var mm = vec2<f32>(0.0);
    if !ray_cylinder(ro, rd, &mm) {
        discard;
    }
    let tex_avg = vec4<f32>(textureSampleLevel(rock_albedo, rock_smp, vec2<f32>(0.5), 0, 16.0).rgb, textureSampleLevel(rock_rhao, rock_smp, vec2<f32>(0.5), 0, 16.0).r);
    let st = stones(ero, rd, rdx, rdy, mm, scene_depth, tex_avg);
    let dust = volumetric_dust(ro, rd, mm, st.depth, random_seed, scene_depth, st.hit_scene_depth, st.hit);
    // BLEND_MODE 3: premultiplied, the dust over the rocks.
    var c = dust;
    if st.hit {
        if st.transparency {
            c = vec4<f32>(st.pc.rgb * st.pc.a * (1.0 - dust.a) + dust.rgb, st.pc.a + dust.a);
        } else {
            c = st.pc * (1.0 - dust.a) + dust;
        }
        c.a = clamp(c.a, 0.0, 1.0);
    }
    if c.a <= 0.0 && max(c.r, max(c.g, c.b)) <= 0.0 {
        discard;
    }
    // The depth the depth test keeps: the rock, or where the dust begins.
    var t = max(mm.x, 0.0);
    if st.hit && st.depth > 0.0 {
        t = st.depth;
    }
    let world = view.world_position + dir_w * max(t * u.scale.z, 0.1);
    let clip = view.clip_from_world * vec4<f32>(world, 1.0);
    var o: FragOut;
    o.color = c;
    o.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return o;
}
