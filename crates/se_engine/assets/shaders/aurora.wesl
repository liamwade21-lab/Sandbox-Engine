// Auroras: SpaceEngine's sprite particles (aurora.glsl). Each particle is a quad of four vertices
// sharing a point of an oval ring on the unit sphere (the oval's frame, its axis +y) and SE's vertex
// bytes (data: phase high, phase low, 0, corner). The vertex shader displaces the ring by Fbm
// noise, stretches the sprite up from the base (the top of the atmosphere) by the aurora's height
// times a noise, colours its foot BottomColor and its top a noisy TopColor, and widens it across in
// view space; the fragment shader draws SE's elongated gaussian, fading to the top. No day/night
// mask: SE has none. Output alpha is 0 so that AlphaMode::Add (premultiplied) adds colour only.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_world_from_local;

struct Aurora {
    // The oval's frame in the body frame (columns x, y = its axis, z); w of frame_x: the base
    // radius (m), the top of the atmosphere.
    frame_x: vec4<f32>,
    frame_y: vec4<f32>,
    frame_z: vec4<f32>,
    // SE's ScaleTime: sprite width (base radii), sprite length (Height / base), the ring's
    // displacement gain, the flash phase.
    scale_time: vec4<f32>,
    // SE's TopColor and BottomColor, times the aurora's brightness.
    top: vec4<f32>,
    bottom: vec4<f32>,
    // xy SE's SunVecClip.xy (the pattern's drift with the star); z 1 for AuroraQuality's DETAIL
    // shader (hash noise in the vertex shader, texture detail in the fragment shader), 0 TEX_NOISE.
    sun_vec: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> aurora: Aurora;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var noise_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var noise_smp: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) data: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // xy sprite corner, z share of the sprite's length that is its streak, w ScaleFade.z
    @location(0) tex: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) noise_point: vec2<f32>,
};

const LAC_GAIN: vec3<f32> = vec3<f32>(2.218281828459, 2.218281828459, 0.671415927);

// TEX_NOISE: the noise texture.
fn fbm_tex(point: vec2<f32>, octaves: i32) -> f32 {
    var sum = 0.0;
    var pa = vec3<f32>(point, 1.0);
    for (var i = 0; i < octaves; i++) {
        sum += pa.z * (textureSampleLevel(noise_tex, noise_smp, pa.xy, 0.0).r - 0.5);
        pa *= LAC_GAIN;
    }
    return sum;
}

fn fbm4_tex(point: vec2<f32>, octaves: i32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    var pa = vec3<f32>(point, 1.0);
    for (var i = 0; i < octaves; i++) {
        sum += pa.z * (textureSampleLevel(noise_tex, noise_smp, pa.xy, 0.0) - 0.5);
        pa *= LAC_GAIN;
    }
    return sum;
}

// DETAIL: SE's hash value noise.
fn hash(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453);
}

fn noise(x: vec2<f32>) -> f32 {
    let p = floor(x);
    var f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    let n = p.x + p.y * 57.0;
    return mix(mix(hash(n), hash(n + 1.0), f.x), mix(hash(n + 57.0), hash(n + 58.0), f.x), f.y);
}

fn noise4(x: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(noise(x), noise(x + vec2<f32>(0.272, -0.375)), noise(x + vec2<f32>(-0.464, 0.674)), noise(x + vec2<f32>(0.548, -0.145)));
}

fn fbm4_hash(point: vec2<f32>, octaves: i32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    var pa = vec3<f32>(point * 256.0, 1.0);
    for (var i = 0; i < octaves; i++) {
        sum += pa.z * (noise4(pa.xy) - 0.5);
        pa *= LAC_GAIN;
    }
    return sum;
}

fn fbm4(point: vec2<f32>, octaves: i32) -> vec4<f32> {
    if aurora.sun_vec.z > 0.5 {
        return fbm4_hash(point, octaves);
    }
    return fbm4_tex(point, octaves);
}

fn mod2(x: f32) -> f32 {
    return x - 2.0 * floor(x * 0.5);
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let st = aurora.scale_time;
    var tex = vec4<f32>(mod2(floor(v.data.a * 4.0)), mod2(floor(v.data.a * 2.0)), 0.0, 0.0);
    let pos_vec = normalize(v.position);
    let np = pos_vec.xz + v.data.xy + aurora.sun_vec.xy;
    let nois = fbm4(np * 0.002, 8);
    var pos = pos_vec;
    pos.x *= 1.0 + st.z * nois.a;
    pos.z *= 1.0 + st.z * nois.a;
    pos = normalize(pos);

    // ScaleFade: sprite width, sprite height, top colour interpolator, brightness.
    let sf = clamp(fbm4(np * 0.05, 6) * vec4<f32>(0.7, 0.7, 1.4, 0.7) + vec4<f32>(0.7), vec4<f32>(0.05, 0.05, 0.0, 0.0), vec4<f32>(2.0, 2.0, 1.0, 1.0));
    let sprite_width = st.x / sf.x * 1.5;
    let sprite_dir = pos_vec * (st.y * sf.y);

    var color: vec4<f32>;
    if tex.x == 0.0 {
        color = aurora.bottom;
    } else {
        let top = fbm4(np * 0.07 + vec2<f32>(0.0, st.w), 8) * vec4<f32>(0.5, 0.5, 0.5, 0.0) + vec4<f32>(0.5, 0.5, 0.5, 0.0);
        color = mix(aurora.bottom, top * aurora.top, sf.z * 3.0);
    }
    color *= fbm_tex(np * 0.0073 + vec2<f32>(0.75, 0.37 * st.w), 4) + 0.5;

    // The oval's unit sphere to the body frame (m) and to view space. SE's sprite width is in its
    // view units (km, as the base radius); here it is in base radii, scaled to render units.
    let m = get_world_from_local(v.instance_index);
    let base = aurora.frame_x.w;
    let frame = mat3x3<f32>(aurora.frame_x.xyz, aurora.frame_y.xyz, aurora.frame_z.xyz) * base;
    let size_v = sprite_width * base * length(m[0].xyz);
    let view_n = view.view_from_world * (m * vec4<f32>(frame * pos, 1.0));
    let view_o = view.view_from_world * (m * vec4<f32>(frame * (pos + sprite_dir), 1.0));
    var view_pos = select(view_o, view_n, tex.x == 0.0);

    let offset = 1.0 - tex.xy * 2.0;
    let ray = view_n.xyz - view_o.xyz;
    let ray_len = length(view_n.xy - view_o.xy);
    tex.z = ray_len / (ray_len + 2.0 * size_v);
    // Both normalised first: in render metres the cross product of the raw vectors overflows f32.
    let c = cross(normalize(ray), normalize(view_n.xyz));
    let tangent = select(vec3<f32>(1.0, 0.0, 0.0), normalize(c), dot(c, c) > 1e-20);
    let rot = mat2x2<f32>(tangent.y, -tangent.x, tangent.x, tangent.y);
    view_pos = vec4<f32>(view_pos.xy + (rot * offset) * size_v, view_pos.zw);

    var o: VertexOut;
    o.clip = view.clip_from_view * view_pos;
    o.tex = vec4<f32>(tex.xyz, sf.z);
    o.color = clamp(color * sf.a, vec4<f32>(0.0), vec4<f32>(1.0));
    o.noise_point = np;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    // Elongated gaussian "texture".
    var tc = 2.0 * in.tex.xy - 1.0;
    let z = in.tex.z;
    if tc.x < 0.0 {
        tc.x = max((tc.x + z) / (z - 1.0), 0.0);
    } else {
        tc.x = min((tc.x - z) / (z - 1.0), 0.0);
    }
    var bright = exp(-5.55 * dot(tc, tc));
    // Fade out the aurora's top.
    bright *= log(max(2.0 - in.tex.x, 1.0)) * 2.0;
    // DETAIL: one octave of texture noise.
    if aurora.sun_vec.z > 0.5 {
        let p = in.noise_point * 15.0 + vec2<f32>(tc.y * 0.03, 0.0) + vec2<f32>(0.0, 0.37) * aurora.scale_time.w;
        bright *= textureSample(noise_tex, noise_smp, p).r * 2.0 - 0.25;
    }
    return vec4<f32>((in.color * max(bright, 0.0)).rgb, 0.0);
}
