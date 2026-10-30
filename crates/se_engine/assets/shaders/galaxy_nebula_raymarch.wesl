// Ray-marched nebula (`nebula_raymarch.rs`): SE's `nebula_raymarch.glsl` with the routines of
// `raymarch_common.glh` it uses, translated from the decrypted shader (re/se_shaders). The back faces of a
// sphere around the model, drawn at infinity like the galaxy layers, give each pixel its view ray; SE draws
// the bounding shape's mesh instead.
//
// Model space: nebula radii / Scale, rotated by the nebula's orientation. Internal space (the script's
// units, SE's `ro`): model space * Internal_scale.

import bevy_pbr::render::mesh_view_bindings::view;

struct Vol {
    // xyz: nebula centre relative to the camera (pc), w: radius (pc)
    center_rel: vec4<f32>,
    rot0: vec4<f32>,
    rot1: vec4<f32>,
    rot2: vec4<f32>,
    // x: 0 box, 1 sphere, 2 cylinder (SE's BBOX / BSPH / BCYL); yzw: uShapeDims (BaseShapeDims)
    shape: vec4<f32>,
    // xyz: Scale, w: radius of the drawn sphere (nebula radii)
    scale: vec4<f32>,
    // xyz: uRandomize (the nebula's Randomize + the model's)
    rnd: vec4<f32>,
    // x: colour brightness (uParamsImg.z), y: alpha brightness (uParamsImg.w)
    img: vec4<f32>,
    // x: uFlags.x, y: Raymarch_step_count (uIntData[0].x)
    ints: vec4<u32>,
    // uFloatData of SE's CustomModelUniforms2 (names in nebula_raymarch.rs FLOATS)
    f: array<vec4<f32>, 39>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> vol: Vol;
// SE's IqNoiseTex: 16 x 16 random texture with G, A = R, B shifted by (11, 5) (blackholes::noise_texture).
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var iq_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var iq_samp: sampler;

// uFlags bits (nebula_raymarch_uniforms.glh).
const SINGLE_STAR: u32 = 0x1u;
const RENDER_STAR: u32 = 0x2u;
const RENDER_STAR_BLOOM: u32 = 0x4u;
const GAS_EMISSION: u32 = 0x8u;
const EMISSION_GAS_1: u32 = 0x10u;
const ABS_BOTH: u32 = 0x80u;
const ABS_X: u32 = 0x100u;
const ABS_Y: u32 = 0x200u;
const F_PLANE: u32 = 0x400u;
const F_DSK: u32 = 0x800u;
const F_CYL: u32 = 0x1000u;
const F_SPH: u32 = 0x2000u;
const F_CAPSULE: u32 = 0x4000u;
const F_TORUS: u32 = 0x8000u;
const F_SPIR: u32 = 0x10000u;
const F_NOISE: u32 = 0x20000u;
const F_FBM: u32 = 0x40000u;
const F_SPIR_NOISE: u32 = 0x80000u;
const F_SPIR_IQ: u32 = 0x100000u;
const F_SPIR_3D: u32 = 0x200000u;
const TWIST: u32 = 0x400000u;

fn flag(bit: u32) -> bool {
    return (vol.ints.x & bit) != 0u;
}

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ray: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let rel = vol.center_rel.xyz + v.position * vol.center_rel.w * vol.scale.w;
    var clip = view.clip_from_world * vec4<f32>(rel, 0.0);
    clip.z = 0.0;
    var o: VertexOut;
    o.clip = clip;
    o.ray = rel;
    return o;
}

// ---- raymarch_common.glh -------------------------------------------------------------------------------

fn hash_dot_2_to_1(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 443.8975);
    p3 = p3 + dot(p3, p3.yzx + 19.19);
    return fract((p3.x + p3.y) * p3.z);
}

fn iq_noise_2(x: vec3<f32>) -> vec2<f32> {
    let i = floor(x);
    var f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    let uv = (i.xy + vec2<f32>(11.0, 5.0) * i.z) + f.xy;
    let rg = textureSampleLevel(iq_tex, iq_samp, (uv + 0.5) / 16.0 + vol.rnd.xy, 0.0).yxwz;
    return vec2<f32>(mix(rg.x, rg.y, f.z), mix(rg.z, rg.w, f.z));
}

// ---- nebula_raymarch.glsl ------------------------------------------------------------------------------

fn noise_2(p: vec3<f32>) -> vec2<f32> {
    return iq_noise_2(p);
}

fn fbm_2(p: vec3<f32>) -> vec2<f32> {
    return noise_2(p * 0.064) * 0.4 + noise_2(p * 0.16) * 0.16 + noise_2(p * 0.4) * 0.064;
}

fn length8(p_in: vec2<f32>) -> f32 {
    var p = p_in * p_in;
    p = p * p;
    p = p * p;
    return pow(p.x + p.y, 0.125);
}

fn disk_sdf(p: vec3<f32>, t: vec3<f32>) -> f32 {
    let q = vec2<f32>(length(p.xy) - t.x, p.z * 0.5);
    return max(length8(q) - t.y, abs(p.z) - t.z);
}

fn cylinder_sdf(p: vec3<f32>, h: vec2<f32>) -> f32 {
    return max(length(p.xz) - h.x, abs(p.y) - h.y);
}

fn sphere_sdf(p: vec3<f32>, r: f32) -> f32 {
    return length(p) - r;
}

fn box_sdf(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let d = abs(p) - b;
    return min(max(d.x, max(d.y, d.z)), 0.0) + length(max(d, vec3<f32>(0.0)));
}

fn torus_sdf(p: vec3<f32>, t: vec2<f32>) -> f32 {
    return length(vec2<f32>(length(p.xz) - t.x, p.y)) - t.y;
}

fn capsule_sdf(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

fn spiral_sdf(pos: vec3<f32>, params: vec4<f32>) -> f32 {
    let r = params.x;
    let ang = pos.z / (params.z * params.y);
    let dx = pos.x - r * cos(ang);
    let dy = pos.y - r * sin(ang);
    return params.w * sqrt(dx * dx + dy * dy);
}

// SpiralNoiseC2: one point, rotated by the (dust, gas) nudges component-wise, as SE's vec2 arithmetic does.
fn spiral_noise_c2(p_in: vec3<f32>) -> vec2<f32> {
    let nudge = vol.f[19].zw;
    let normalizer = 1.0 / sqrt(1.0 + nudge * nudge);
    var n = vol.f[20].xy;
    var iter = vol.f[20].zw;
    var p = p_in + vol.rnd.xyz * 10.0;
    for (var i = 0; i < 8; i = i + 1) {
        n = n - abs(sin(p.y * iter) + cos(p.x * iter)) / iter;
        let xy = (p.xy + vec2<f32>(p.y, -p.x) * nudge) * normalizer;
        p = vec3<f32>(xy, p.z);
        let xz = (p.xz + vec2<f32>(p.z, -p.x) * nudge) * normalizer;
        p = vec3<f32>(xz.x, p.y, xz.y);
        iter = iter * vol.f[21].xy;
    }
    return n;
}

fn spiral_noise_3d_2(p_in: vec3<f32>) -> vec2<f32> {
    let nudge = vol.f[21].zw;
    let normalizer = 1.0 / sqrt(1.0 + nudge * nudge);
    var n = vol.f[22].xy;
    var iter = vol.f[22].zw;
    var p = p_in + vol.rnd.xyz * 10.0;
    for (var i = 0; i < 5; i = i + 1) {
        n = n + (sin(p.y * iter) + cos(p.x * iter)) / iter;
        let xz = (p.xz + vec2<f32>(p.z, -p.x) * nudge) * normalizer;
        p = vec3<f32>(xz.x, p.y, xz.y);
        iter = iter * vol.f[23].xy;
    }
    return n;
}

fn nebula_form(p: vec3<f32>) -> vec2<f32> {
    var fin = vec2<f32>(0.0);
    if flag(F_PLANE) {
        fin = fin + vol.f[23].zw * (dot(p, vol.f[15].xyz) + vol.f[34].y);
    }
    if flag(F_DSK) {
        fin = fin + vol.f[24].xy * disk_sdf(p.xzy, vol.f[16].xyz);
    }
    if flag(F_CYL) {
        fin = fin + vol.f[24].zw * cylinder_sdf(p, vol.f[25].xy);
    }
    if flag(F_CAPSULE) {
        fin = fin + vol.f[26].xy * capsule_sdf(p, vol.f[17].xyz, vol.f[18].xyz, vol.f[34].w);
    }
    if flag(F_SPH) {
        fin = fin + vol.f[25].zw * sphere_sdf(p, vol.f[34].z);
    }
    if flag(F_TORUS) {
        fin = fin + vol.f[26].zw * torus_sdf(p, vol.f[35].xy);
    }
    if flag(F_SPIR) {
        fin = fin + vol.f[27].xy * spiral_sdf(p.yxz, vol.f[0]);
    }
    if flag(F_NOISE) {
        fin = fin + vol.f[27].zw * noise_2(p * vol.f[35].z) * vol.f[35].w;
    }
    if flag(F_FBM) {
        fin = fin + vol.f[28].xy * fbm_2(p * vol.f[36].x);
    }
    if flag(F_SPIR_NOISE) {
        fin = fin + vol.f[28].zw * spiral_noise_c2(p.zxy * vol.f[36].y + vol.f[36].z) * vol.f[36].w;
    }
    if flag(F_SPIR_IQ) {
        fin = fin + vol.f[29].xy * spiral_noise_c2(p.zxy * vol.f[37].x * fbm_2(p * vol.f[37].w).x + vol.f[37].y) * vol.f[37].z;
    }
    if flag(F_SPIR_3D) {
        fin = fin + vol.f[29].zw * spiral_noise_3d_2(p * vol.f[38].x);
    }
    return fin;
}

fn nebula_scale(p: vec3<f32>) -> vec2<f32> {
    let ns = vol.f[34].x;
    var fin = nebula_form(p / ns) * ns + vol.f[19].xy;
    if flag(ABS_BOTH) {
        fin = abs(fin);
    }
    if flag(ABS_X) {
        fin = vec2<f32>(abs(fin.x), fin.y);
    }
    if flag(ABS_Y) {
        fin = vec2<f32>(fin.x, abs(fin.y));
    }
    return fin;
}

fn nebula_map_raw(p_in: vec3<f32>) -> vec2<f32> {
    let p = p_in - vol.f[14].xyz;
    var fin: vec2<f32>;
    if flag(TWIST) {
        // mat2(c, -s, s, c) * p.xz, with p.y as the third axis.
        let c = cos(vol.f[38].y * p.y);
        let s = sin(vol.f[38].z * p.y);
        fin = nebula_scale(vec3<f32>(c * p.x + s * p.z, -s * p.x + c * p.z, p.y));
    } else {
        fin = nebula_scale(p);
    }
    return fin + vol.f[30].xy;
}

fn bound_sdf(p: vec3<f32>) -> f32 {
    let s = vol.shape;
    if s.x > 0.5 && s.x < 1.5 {
        return sphere_sdf(p, s.y);
    }
    if s.x > 1.5 {
        return cylinder_sdf(p, s.yz);
    }
    return box_sdf(p, s.yzw);
}

// (dust distance, gas distance, bounding object)
fn nebula_map(p: vec3<f32>, star_empty: f32, noise: vec2<f32>) -> vec3<f32> {
    var fin = nebula_map_raw(p);
    let boundary = bound_sdf(p) + abs(1.0 - 1.2 * noise.x);
    fin = max(fin, vec2<f32>(boundary));
    fin.x = max(fin.x, -select(fin.x, star_empty, fin.x < 0.0));
    return vec3<f32>(fin, boundary);
}

fn remap(v: f32, a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    return b0 + (v - a0) / (a1 - a0) * (b1 - b0);
}

// SE's HGPhase, with its comma expression `(1 + g2 - 2 g c, 1.5)` evaluating to 1.5.
fn hg_phase(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 + c * c) * (1.0 - g2) * 1.5 / (1.5 * (2.0 + g2)) + g * c;
}

// Ray against the bounding shape (internal units): (near, far), or far < near and far <= 0 on a miss.
fn intersect(o: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let s = vol.shape;
    if s.x > 0.5 && s.x < 1.5 {
        let b = dot(d, o);
        let c = dot(o, o) - s.y * s.y;
        let delta = b * b - c;
        if delta < 0.0 {
            return vec3<f32>(0.0, 0.0, 0.0);
        }
        let k = sqrt(delta);
        return vec3<f32>(select(0.0, 1.0, -b + k > 0.0), -b - k, -b + k);
    }
    if s.x > 1.5 {
        let a = dot(d.xz, d.xz);
        let b = dot(o.xz, d.xz);
        let c = dot(o.xz, o.xz) - s.y * s.y;
        let delta = b * b - a * c;
        if delta < 0.0 {
            return vec3<f32>(0.0, 0.0, 0.0);
        }
        let k = sqrt(delta);
        let arcp = 1.0 / a;
        var near = (-b - k) * arcp;
        var far = (-b + k) * arcp;
        let tmp = min(far, near);
        far = max(far, near);
        near = tmp;
        let znear = o.y + near * d.y;
        let zfar = o.y + far * d.y;
        let zcap = vec2<f32>(s.z, -s.z);
        let cap = (zcap - o.y) / d.y;
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
        return vec3<f32>(select(0.0, 1.0, far > 0.0 && far > near), near, far);
    }
    let m = 1.0 / d;
    let n = m * o;
    let k = abs(m) * s.yzw;
    let t1 = -n - k;
    let t2 = -n + k;
    let near = max(max(t1.x, t1.y), t1.z);
    let far = min(min(t2.x, t2.y), t2.z);
    return vec3<f32>(select(1.0, 0.0, near > far || far < 0.0), near, far);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let rot = mat3x3<f32>(vol.rot0.xyz, vol.rot1.xyz, vol.rot2.xyz);
    let internal = vol.f[1].w;
    let k = internal / max(vol.scale.xyz, vec3<f32>(1.0e-3));
    let ro = rot * (-vol.center_rel.xyz / vol.center_rel.w) * k;
    let rd = normalize(rot * normalize(in.ray) * k);
    // SE's DITHERING: seed = the clip position of the ray (per pixel here), uvd its scaled copy reading
    // DitherNoiseTex, whose contents were not traced (a hash stands in).
    let seed = in.clip.xy;
    let uvd = in.clip.xy * vec2<f32>(280.0, 120.0) / 1000.0;
    let dither1 = 0.4 + 0.10 * hash_dot_2_to_1(seed);
    let dither2 = 0.8 + 0.08 * hash_dot_2_to_1(vec2<f32>(uvd.y, -uvd.x + 0.5 * sin(4.0 + uvd.y * 4.0)));

    let light_color = vol.f[2].xyz * vol.f[3].w;
    let ambient_color = vol.f[3].xyz * vol.f[5].w;
    let star_pos = vol.f[4].xyz;
    let ext_color = vec3<f32>(0.748, 1.0, 1.324);
    let dust_h = vol.f[12].w;
    let dust_density = vol.f[15].w;
    let gas_h = vol.f[30].w;

    var sum = vec3<f32>(0.0);
    var sum_t = vec3<f32>(1.0);
    var step_length = 1.0e-6;
    var t = 0.0;
    let hit = intersect(ro, rd);
    let max_dist = hit.z;
    if hit.x > 0.5 {
        t = hit.y * step(t, hit.y);
    }
    let steps = i32(clamp(vol.ints.y, 1u, 512u));
    for (var i = 0; i < steps; i = i + 1) {
        let pos = ro + t * rd;
        if t > max_dist {
            break;
        }
        if all(sum_t < vec3<f32>(0.01)) {
            break;
        }
        let helper_noise = noise_2(pos * 0.25) * vec2<f32>(4.0);
        // SampleStar
        var star_empty = 99999999.0;
        var light_radius = 1.0e-8;
        var ldl = 999999.0;
        if flag(SINGLE_STAR) {
            ldl = distance(pos, star_pos);
            star_empty = sphere_sdf(pos - star_pos, vol.f[6].w) + abs(1.0 - 1.2 * helper_noise.x) * 1.25;
            light_radius = vol.f[4].w;
        }
        let nd = nebula_map(pos, star_empty, helper_noise);
        // Dust_SDF_Volume
        var d = nd.x;
        let ld = remap(clamp(-min(d, 0.0), -0.0025, dust_h), -0.0025, dust_h, 0.0, 1.0) * dither1;
        let sigma_abs = vec3<f32>(ld * vol.f[13].w);
        let sigma_sc = vec3<f32>(ld * vol.f[14].w);
        let sigma_ext = max(vec3<f32>(1.0e-6), (sigma_sc + sigma_abs) * dust_density);
        let light_dir = star_pos - pos;
        let ray_length = distance(pos, ro);
        if flag(RENDER_STAR) {
            sum = sum + light_color / (ldl * ldl * vol.f[8].w) / vol.f[9].w * sum_t;
        }
        if flag(RENDER_STAR_BLOOM) {
            sum = sum + light_color * exp(-ldl * ldl * ldl * vol.f[10].w) / vol.f[11].w * sum_t;
        }
        if flag(GAS_EMISSION) {
            var gas_color = vec3<f32>(0.0);
            // A negative distance term makes SE's pow() NaN, and its clamp() then zeroes the sum.
            var invalid = false;
            let ndb = max(nd.y, nd.z);
            for (var g = 0u; g < 3u; g = g + 1u) {
                if !flag(EMISSION_GAS_1 << g) {
                    continue;
                }
                let dir = vol.f[6u + 3u * g].xyz;
                let c0 = vol.f[7u + 3u * g].xyz;
                let col = vol.f[5u + 3u * g].xyz;
                // coef_1..3 of gas g: uFloatData[31].z onwards.
                let base = 31u * 4u + 2u + 3u * g;
                let c1 = vol.f[base / 4u][base % 4u];
                let c2 = vol.f[(base + 1u) / 4u][(base + 1u) % 4u];
                let c3 = vol.f[(base + 2u) / 4u][(base + 2u) % 4u];
                let e = (max(ndb, 1.0e-4) + c0.x) * dir.x + ((abs(min(ndb, -1.0e-4)) + c0.y) * dir.y + (abs(ndb) + c0.z) * dir.z);
                invalid = invalid || (e < 0.0 && c2 != floor(c2));
                gas_color = gas_color + e * col * c1 * exp(-pow(abs(e), c2) * c3) * sum_t;
            }
            gas_color = select(clamp(gas_color, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.0), invalid);
            if ndb < gas_h {
                let gas_density = gas_h - min(ndb, -1.0e-5);
                let sigma_gas = vec3<f32>(max(gas_density * vol.f[31].x, 1.0e-12));
                let trans_gas = exp(-sigma_gas * step_length * vol.f[31].y);
                gas_color = gas_color * vol.f[33].w * sigma_gas;
                let gas_col = (gas_color - gas_color * trans_gas) / sigma_gas;
                let gas_opacity = 1.0 - sigma_gas;
                sum = sum + gas_col * gas_opacity * sum_t;
                sum_t = sum_t * trans_gas;
            }
        }
        if d < dust_h {
            let trans = exp(-sigma_ext * ext_color * step_length * dust_density);
            var scattered: vec3<f32>;
            let amb = exp(min(d, 0.0) * ld * ld) * ambient_color * sigma_sc;
            if ldl < light_radius && light_radius > 0.0 {
                // SE leaves the light direction unnormalised here.
                let c = dot(rd, light_dir);
                let phase = mix(hg_phase(c, vol.f[17].w), hg_phase(c, vol.f[18].w), vol.f[30].z);
                let r1 = ldl / light_radius;
                let r2 = clamp(1.0 - r1 * r1 * r1 * r1, 0.0, 1.0);
                let attenuation = r2 * r2 / (ldl * ldl + 1.0);
                scattered = light_color * attenuation * phase * sigma_sc + amb;
            } else {
                scattered = amb;
                d = 1.1 * d;
            }
            let col = (scattered - scattered * vol.f[16].w * trans) / (sigma_ext * ext_color);
            sum = sum + col * (1.0 - trans) * sum_t;
            sum_t = sum_t * trans;
        }
        d = max(d, 0.75);
        d = abs(d) * dither2;
        step_length = max(d * 0.1 * clamp(1.65 - min(ray_length, 0.0), 0.125, ldl), 0.1);
        t = t + step_length;
    }
    let s = sum;
    let tone = s * s * s * (s * (s * 6.0 - 15.0) + 10.0);
    let grey = dot(sum_t, vec3<f32>(0.299, 0.587, 0.114));
    // Premultiplied blending: SE's OutTransm per colour channel becomes one grey transmittance.
    return vec4<f32>(tone * vol.img.x, (1.0 - grey) * vol.img.y);
}
