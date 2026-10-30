// Ray-marched nebula (`nebula_raymarch.rs`): SE's `CustomModel` parameters, the port's own marcher (SE's
// `nebula_raymarch.glsl` is encrypted). The back faces of a sphere around the model, drawn at infinity like
// the galaxy layers, give each pixel its view ray; the marcher integrates two density fields, dust and
// emitting gas, front to back. Additive output, so it works from outside and from inside.
//
// Model space: nebula radii, rotated by the nebula's orientation. Internal space (the script's units):
// model space * Internal_scale / Scale.

import bevy_pbr::render::mesh_view_bindings::view;

const SLOTS: u32 = 52u;
// Brightness calibration against SE (see GAIN in nebula_raymarch.rs).
const TUNE: f32 = 1.0;

struct Vol {
    // xyz: nebula centre relative to the camera (pc), w: radius (pc)
    center_rel: vec4<f32>,
    rot0: vec4<f32>,
    rot1: vec4<f32>,
    rot2: vec4<f32>,
    p: array<vec4<f32>, 52>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> vol: Vol;

// Slots (see `pack` in nebula_raymarch.rs).
const P_SHAPE: u32 = 0u;     // x: 0 box, 1 sphere, 2 cylinder; yzw: dims (internal)
const P_SCALE: u32 = 1u;     // xyz: Scale, w: Internal_scale
const P_MARCH: u32 = 2u;     // x: steps, y: td_break, z: bound radius (nebula radii), w: gain
const P_TBREAK: u32 = 3u;    // xyz: sum_transparency_break, w: fade
const P_DUST: u32 = 4u;      // x: on, y: height, z: absorption, w: scattering
const P_DUST2: u32 = 5u;     // x: density factor, y: albedo, z: forward g, w: backward g
const P_GAS: u32 = 6u;       // x: HG mix, y: on, z: height, w: scattering
const P_GAS2: u32 = 7u;      // x: density factor, y: emission coefficient, z: form scale (NebScale), w: twist on
const P_LIGHT: u32 = 8u;     // rgb: star light, w: light radius
const P_AMB: u32 = 9u;       // rgb: ambient light, w: single star
const P_STAR: u32 = 10u;     // xyz: star position, w: empty sphere radius
const P_STARR: u32 = 11u;    // x: render star, y: coef 1, z: coef 2, w: render bloom
const P_BLOOM: u32 = 12u;    // x: bloom coef 1, y: bloom coef 2, z/w: twist coefs
const P_EM: u32 = 13u;       // 3 per gas: (colour, on), (direction, coef 1), (coef 0, coef 2)
const P_EM3: u32 = 22u;      // xyz: coef 3 of the gases
const P_THICK: u32 = 23u;    // xy: NebScaleCoef, zw: NebulaMapThickCoef (dust, gas)
const P_SN1: u32 = 24u;      // xy: nudge, zw: amount (dust, gas)
const P_SN2: u32 = 25u;      // xy: start frequency, zw: frequency factor
const P_S3D1: u32 = 26u;
const P_S3D2: u32 = 27u;
const P_CENT: u32 = 28u;
const F_PLANE: u32 = 29u;
const F_DSK: u32 = 31u;
const F_CYL: u32 = 33u;
const F_SPH: u32 = 35u;
const F_CAP: u32 = 36u;
const F_TOR: u32 = 39u;
const F_SPIR: u32 = 41u;
const F_NOISE: u32 = 43u;
const F_FBM: u32 = 45u;
const F_SNOISE: u32 = 46u;
const F_IQ: u32 = 48u;
const F_S3D: u32 = 50u;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ray: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let rel = vol.center_rel.xyz + v.position * vol.center_rel.w * vol.p[P_MARCH].z;
    var clip = view.clip_from_world * vec4<f32>(rel, 0.0);
    clip.z = 0.0;
    var o: VertexOut;
    o.clip = clip;
    o.ray = rel;
    return o;
}

// ---- noise --------------------------------------------------------------------------------------------

fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.3183099 + vec3<f32>(0.71, 0.113, 0.419));
    q = q * 17.0;
    return fract(q.x * q.y * q.z * (q.x + q.y + q.z)) * 2.0 - 1.0;
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = mix(mix(hash3(i), hash3(i + vec3(1.0, 0.0, 0.0)), u.x), mix(hash3(i + vec3(0.0, 1.0, 0.0)), hash3(i + vec3(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(hash3(i + vec3(0.0, 0.0, 1.0)), hash3(i + vec3(1.0, 0.0, 1.0)), u.x), mix(hash3(i + vec3(0.0, 1.0, 1.0)), hash3(i + vec3(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i = i + 1) {
        s = s + a * vnoise(q);
        q = q * 2.03;
        a = a * 0.5;
    }
    return s;
}

// Duke's spiral noise (shadertoy "Dusty nebula"): ridged sines on rotating axes.
fn spiral_c(p_in: vec3<f32>, nudge: f32, amount: f32, iter0: f32, mul: f32) -> f32 {
    var p = p_in;
    var n = amount;
    var it = max(iter0, 0.1);
    let norm = 1.0 / sqrt(1.0 + nudge * nudge);
    for (var i = 0; i < 8; i = i + 1) {
        n = n - abs(sin(p.y * it) + cos(p.x * it)) / it;
        p = vec3<f32>((p.xy + vec2<f32>(p.y, -p.x) * nudge) * norm, p.z);
        p = vec3<f32>((p.x + p.z * nudge) * norm, p.y, (p.z - p.x * nudge) * norm);
        it = it * max(mul, 1.01);
    }
    return n;
}

fn spiral_3d(p_in: vec3<f32>, nudge: f32, amount: f32, iter0: f32, mul: f32) -> f32 {
    var p = p_in;
    var n = amount;
    var it = max(iter0, 0.1);
    let norm = 1.0 / sqrt(1.0 + nudge * nudge);
    for (var i = 0; i < 5; i = i + 1) {
        n = n + (sin(p.y * it) + cos(p.x * it)) / it;
        p = vec3<f32>((p.x + p.z * nudge) * norm, p.y, (p.z - p.x * nudge) * norm);
        it = it * max(mul, 1.01);
    }
    return n;
}

// ---- shape -----------------------------------------------------------------------------------------------

fn sel(i: u32, v: u32) -> f32 {
    let f = vol.p[i];
    if f.z < 0.5 {
        return 0.0;
    }
    return select(f.x, f.y, v == 1u);
}

// Signed sum of the forms for the dust (x) and the gas (y) volume at internal position q.
fn fields(q_in: vec3<f32>) -> vec2<f32> {
    var q = q_in - vol.p[P_CENT].xyz;
    if vol.p[P_GAS2].w > 0.5 {
        let a = vol.p[P_BLOOM].z * q.y * vol.p[P_BLOOM].w * 0.1;
        let c = cos(a);
        let s = sin(a);
        q = vec3<f32>(c * q.x - s * q.z, q.y, s * q.x + c * q.z);
    }
    q = q * vol.p[P_GAS2].z;
    var d = vec2<f32>(0.0);
    var w = vec2<f32>(sel(F_PLANE, 0u), sel(F_PLANE, 1u));
    if any(w != vec2<f32>(0.0)) {
        d = d + w * (dot(q, normalize(vol.p[F_PLANE + 1u].xyz + vec3<f32>(1.0e-6))) - vol.p[F_PLANE].w);
    }
    w = vec2<f32>(sel(F_DSK, 0u), sel(F_DSK, 1u));
    if any(w != vec2<f32>(0.0)) {
        let t = vol.p[F_DSK + 1u].xyz;
        let r = vec2<f32>(length(q.xz) - t.x, q.y * 0.5);
        d = d + w * max(length(r) - t.y, abs(q.y) - t.z);
    }
    w = vec2<f32>(sel(F_CYL, 0u), sel(F_CYL, 1u));
    if any(w != vec2<f32>(0.0)) {
        let t = vol.p[F_CYL + 1u].xy;
        d = d + w * max(length(q.xz) - t.x, abs(q.y) - t.y);
    }
    w = vec2<f32>(sel(F_SPH, 0u), sel(F_SPH, 1u));
    if any(w != vec2<f32>(0.0)) {
        d = d + w * (length(q) - vol.p[F_SPH].w);
    }
    w = vec2<f32>(sel(F_CAP, 0u), sel(F_CAP, 1u));
    if any(w != vec2<f32>(0.0)) {
        let a = vol.p[F_CAP + 1u].xyz;
        let b = vol.p[F_CAP + 2u].xyz;
        let pa = q - a;
        let ba = b - a;
        let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1.0e-6), 0.0, 1.0);
        d = d + w * (length(pa - ba * h) - vol.p[F_CAP].w);
    }
    w = vec2<f32>(sel(F_TOR, 0u), sel(F_TOR, 1u));
    if any(w != vec2<f32>(0.0)) {
        let r = vol.p[F_TOR + 1u].xy;
        let major = 0.5 * (r.x + r.y);
        let minor = 0.5 * abs(r.y - r.x);
        d = d + w * (length(vec2<f32>(length(q.xz) - major, q.y)) - minor);
    }
    w = vec2<f32>(sel(F_SPIR, 0u), sel(F_SPIR, 1u));
    if any(w != vec2<f32>(0.0)) {
        let s = vol.p[F_SPIR + 1u];
        let r = length(q.xz) + 1.0e-3;
        let arm = sin(atan2(q.z, q.x) * s.x - log(r) * s.y);
        d = d + w * ((1.0 - arm) * s.z * r + abs(q.y) * s.w);
    }
    w = vec2<f32>(sel(F_NOISE, 0u), sel(F_NOISE, 1u));
    if any(w != vec2<f32>(0.0)) {
        let c = vol.p[F_NOISE + 1u].xy;
        d = d + w * vnoise(q * c.x * 0.1) * c.y;
    }
    w = vec2<f32>(sel(F_FBM, 0u), sel(F_FBM, 1u));
    if any(w != vec2<f32>(0.0)) {
        d = d + w * fbm(q * vol.p[F_FBM].w / max(vol.p[P_SCALE].w, 1.0));
    }
    let sn1 = vol.p[P_SN1];
    let sn2 = vol.p[P_SN2];
    w = vec2<f32>(sel(F_SNOISE, 0u), sel(F_SNOISE, 1u));
    if any(w != vec2<f32>(0.0)) {
        let c = vol.p[F_SNOISE + 1u].xyz;
        let pq = q.zxy * c.x + vec3<f32>(c.y);
        d.x = d.x + w.x * spiral_c(pq, sn1.x, sn1.z, sn2.x, sn2.z) * c.z;
        d.y = d.y + w.y * spiral_c(pq, sn1.y, sn1.w, sn2.y, sn2.w) * c.z;
    }
    w = vec2<f32>(sel(F_IQ, 0u), sel(F_IQ, 1u));
    if any(w != vec2<f32>(0.0)) {
        let c = vol.p[F_IQ + 1u];
        let pq = q * c.x + vec3<f32>(c.y);
        d.x = d.x + w.x * spiral_c(pq, sn1.x, sn1.z, sn2.x, c.w) * c.z;
        d.y = d.y + w.y * spiral_c(pq, sn1.y, sn1.w, sn2.y, c.w) * c.z;
    }
    w = vec2<f32>(sel(F_S3D, 0u), sel(F_S3D, 1u));
    if any(w != vec2<f32>(0.0)) {
        let a = vol.p[P_S3D1];
        let b = vol.p[P_S3D2];
        d.x = d.x + w.x * spiral_3d(q, a.x, a.z, b.x, b.z) * vol.p[F_S3D].w;
        d.y = d.y + w.y * spiral_3d(q, a.y, a.w, b.y, b.w) * vol.p[F_S3D].w;
    }
    return d;
}

// Density of dust (x) and gas (y): matter lies in sheets around the zero level of each volume's field
// (as in Duke's nebula shaders), `height` thick.
fn density(q: vec3<f32>) -> vec2<f32> {
    let on = vec2<f32>(vol.p[P_DUST].x, vol.p[P_GAS].y);
    let h = vec2<f32>(vol.p[P_DUST].y, vol.p[P_GAS].z);
    let m = abs(fields(q)) * 0.5 + vec2<f32>(0.07);
    var dens = max(h - m, vec2<f32>(0.0)) / max(h, vec2<f32>(1.0e-3)) * on * exp(vol.p[P_THICK].zw);
    // Fade out over the outer fifth of the bounding shape, so matter cut by it has no hard edge.
    dens = dens * smoothstep(1.0, 0.8, edge(q));
    // No matter inside the star's bubble.
    let er = vol.p[P_STAR].w;
    if vol.p[P_AMB].w > 0.5 && er > 0.0 {
        dens = dens * smoothstep(er * 0.8, er, length(q - vol.p[P_STAR].xyz));
    }
    return dens;
}

// Position relative to the bounding shape: 0 at the centre, 1 on its surface.
fn edge(q: vec3<f32>) -> f32 {
    let s = vol.p[P_SHAPE];
    if s.x > 0.5 && s.x < 1.5 {
        return length(q) / max(s.y, 1.0e-3);
    }
    if s.x > 1.5 {
        return max(length(q.xz) / max(s.y, 1.0e-3), abs(q.y) / max(s.z, 1.0e-3));
    }
    let r = abs(q) / max(s.yzw, vec3<f32>(1.0e-3));
    return max(max(r.x, r.y), r.z);
}

fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * c, 1.0e-4), 1.5));
}

// Emission colour of the gas at internal position q (unit: Internal_scale).
fn emission(q: vec3<f32>) -> vec3<f32> {
    // Radius measured in units of the bounding shape.
    let i_s = max(vol.p[P_SHAPE].y, 1.0);
    let u = q / i_s;
    var c = vec3<f32>(0.0);
    for (var i = 0u; i < 3u; i = i + 1u) {
        let col = vol.p[P_EM + i * 3u];
        if col.w < 0.5 {
            continue;
        }
        let dir = vol.p[P_EM + i * 3u + 1u];
        let c0 = vol.p[P_EM + i * 3u + 2u];
        // `direction` weights the axes of the distance; floors keep a zero axis or a tiny falloff from
        // collapsing the glow into a sliver.
        let r = length((u - c0.xyz / i_s) * max(abs(dir.xyz), vec3<f32>(0.25)));
        let wgt = vol.p[P_EM3][i] * exp(-max(r - dir.w, 0.0) / max(c0.w, 0.15));
        c = c + col.rgb * wgt;
    }
    return c;
}

// Ray against the bounding shape (internal units): (t0, t1), t1 < t0 on a miss.
fn bounds(o: vec3<f32>, d: vec3<f32>) -> vec2<f32> {
    let s = vol.p[P_SHAPE];
    if s.x > 0.5 && s.x < 1.5 {
        let r = s.y;
        let b = dot(o, d);
        let c = dot(o, o) - r * r;
        let disc = b * b - c;
        if disc < 0.0 {
            return vec2<f32>(1.0, 0.0);
        }
        let k = sqrt(disc);
        return vec2<f32>(-b - k, -b + k);
    }
    let ext = select(s.yzw, vec3<f32>(s.y, s.z, s.y), s.x > 1.5);
    let inv = 1.0 / select(d, vec3<f32>(1.0e-6), abs(d) < vec3<f32>(1.0e-6));
    let ta = (-ext - o) * inv;
    let tb = (ext - o) * inv;
    let tmin = min(ta, tb);
    let tmax = max(ta, tb);
    return vec2<f32>(max(max(tmin.x, tmin.y), tmin.z), min(min(tmax.x, tmax.y), tmax.z));
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let rot = mat3x3<f32>(vol.rot0.xyz, vol.rot1.xyz, vol.rot2.xyz);
    let sc = vol.p[P_SCALE];
    let k = sc.w / max(sc.xyz, vec3<f32>(1.0e-3));
    // Camera and ray in internal units.
    let o = rot * (-vol.center_rel.xyz / vol.center_rel.w) * k;
    let d = normalize(rot * normalize(in.ray) * k);
    let tb = bounds(o, d);
    let t0 = max(tb.x, 0.0);
    let t1 = tb.y;
    if t1 <= t0 {
        discard;
    }
    let mp = vol.p[P_MARCH];
    let steps = clamp(i32(mp.x), 16, 400);
    let dt = (t1 - t0) / f32(steps);
    // Per-pixel jitter of the first sample hides the step pattern.
    let jitter = fract(sin(dot(in.clip.xy, vec2<f32>(12.9898, 78.233))) * 43758.547);

    let dust = vol.p[P_DUST];
    let dust2 = vol.p[P_DUST2];
    let gas = vol.p[P_GAS];
    let gas2 = vol.p[P_GAS2];
    let light = vol.p[P_LIGHT];
    let amb = vol.p[P_AMB];
    let star = vol.p[P_STAR];
    let starr = vol.p[P_STARR];
    let bloom = vol.p[P_BLOOM];
    // Scale from the script's densities to optical depth per internal unit.
    let k_dust = dust2.x * 0.02;
    let k_gas = gas2.x;

    var col = vec3<f32>(0.0);
    var tr = vec3<f32>(1.0);
    var td = 0.0;
    let tbreak = vol.p[P_TBREAK].xyz;
    for (var i = 0; i < steps; i = i + 1) {
        let t = t0 + (f32(i) + jitter) * dt;
        let q = o + d * t;
        let dens = density(q);
        let to_star = star.xyz - q;
        let ld = max(length(to_star), 1.0e-3);
        var lit = amb.rgb;
        if amb.w > 0.5 {
            let c = dot(d, to_star / ld);
            let phase = mix(hg(c, dust2.z), hg(c, -abs(dust2.w)), gas.x) * 12.566371;
            let fall = select(1.0 / (1.0 + ld * ld * 0.01), 1.0 / (1.0 + (ld / light.w) * (ld / light.w)), light.w > 0.0);
            lit = lit + light.rgb * phase * fall;
            if starr.x > 0.5 {
                col = col + tr * light.rgb / (ld * ld * max(starr.y, 1.0e-3)) / max(starr.z, 1.0e-3) * dt;
            }
            if starr.w > 0.5 {
                col = col + tr * light.rgb / exp(ld * ld * ld * bloom.x) / max(bloom.y, 1.0e-3) * dt;
            }
        }
        if dens.x + dens.y > 0.0 {
            let sd = dens.x * k_dust;
            let sg = dens.y * k_gas;
            let scat = (sd * dust.w * dust2.y + sg * gas.w) * lit;
            let em = sg * gas2.y * emission(q);
            col = col + tr * (scat + em) * dt;
            tr = tr * exp(-vec3<f32>(sd * dust.z + sg * gas.w) * dt);
            td = td + (dens.x + dens.y) * dt;
            if (mp.y > 0.0 && td > mp.y * 10.0) || all(tr < max(tbreak, vec3<f32>(0.003))) {
                break;
            }
        }
    }
    // Premultiplied blending: the light adds, the dust's opacity (faded in with the model) covers the sky.
    let opacity = (1.0 - dot(tr, vec3<f32>(1.0 / 3.0))) * vol.p[P_TBREAK].w;
    return vec4<f32>(col * mp.w * TUNE, opacity);
}
