// Comet tails: SpaceEngine's particle sprites (comet_tail.glsl). Each particle is a quad of four
// vertices sharing a random direction (position) and SE's vertex bytes (data: phase high, phase
// low, kind 0 gas / 1-255 dust, corner). The mesh is in tail lengths, the universe axes, at the
// nucleus. The vertex shader moves the particle through its cycle: gas pushed straight away from
// the star by the solar wind, dust by light pressure and left behind along the orbit (CometVel),
// both spreading faster away from perihelion (scale_time.z); the sprite is stretched along its
// motion in view space. The fragment shader draws a soft capsule, dimmer as 1 / (r^2 + 0.05)
// with r the distance from the star in AU, and lights dust with a Henyey-Greenstein phase
// (g = -0.3). Output alpha is 0 so that AlphaMode::Add (premultiplied) adds colour only.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_world_from_local;

struct CometTail {
    // x tail length (km), y particle cycle phase, z 1 - (q / r)^0.25, w sprite scale
    scale_time: vec4<f32>,
    // rgb colour times one particle's brightness
    gas_color: vec4<f32>,
    dust_color: vec4<f32>,
    // xyz velocity about the star (km/s)
    comet_vel: vec4<f32>,
    // xyz comet - camera (AU)
    eye_pos: vec4<f32>,
    // xyz comet - star (AU), w light-pressure factor
    sun_pos: vec4<f32>,
    // rgb star light, a 1
    sun_col: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> comet: CometTail;

const KM2AU: f32 = 1.0 / 149598000.0;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) data: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Fragment position relative to the nucleus (AU).
    @location(0) frag_pos: vec3<f32>,
    // xy sprite corner, z share of the sprite's length that is its streak, w 0 gas / 1 dust
    @location(1) tex: vec4<f32>,
    @location(2) color: vec4<f32>,
    // Three sprite radii (AU), the distance from the eye below which a sprite fades.
    @location(3) sprite_size: f32,
};

fn mod2(x: f32) -> f32 {
    return x - 2.0 * floor(x * 0.5);
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let st = comet.scale_time;
    var tex = vec4<f32>(mod2(floor(v.data.a * 4.0)), mod2(floor(v.data.a * 2.0)), 0.0, 0.0);
    let pos_vec = normalize(v.position);
    let sun_vec = normalize(comet.sun_pos.xyz);
    let t = fract(st.y + v.data.r + v.data.g / 256.0);
    let fade = (1.0 - pow(t, 0.25)) * clamp(t * 50.0, 0.0, 1.0) * clamp(1.0 - st.z, 0.0, 1.0) / st.w;

    var color: vec4<f32>;
    var pos: vec3<f32>;
    var sprite_size: f32;
    var sprite_dir: vec3<f32>;
    if v.data.b == 0.0 {
        // Gas: interaction with the solar wind.
        color = vec4<f32>(comet.gas_color.rgb, 0.0);
        tex.w = 0.0;
        let vel = 0.03 * (1.0 + 13.0 * st.z) * pos_vec;
        let wvel = -14.0 * (1.001 - st.z) * sun_vec;
        let dv = dot(vel, sun_vec) * sun_vec - wvel;
        let acc = 0.005 * dot(dv, dv) * sun_vec;
        pos = pos_vec * 0.001 + vel * t + acc * t * t;
        let scale = 1.0 + 10.0 * t + 10.0 * st.z;
        sprite_size = scale * 0.0025 * st.w;
        let sprite_length = 0.0001 + 0.1 * pow(clamp(t * 2.0, 0.0, 1.0), 4.0);
        sprite_dir = sun_vec * sprite_length;
    } else {
        // Dust: interaction with sunlight (light pressure).
        color = vec4<f32>(comet.dust_color.rgb, 0.0);
        tex.w = 1.0;
        let acc = 0.5 * (comet.sun_pos.w * sun_vec - 0.02 * v.data.b * comet.comet_vel.xyz);
        let vel = 0.03 * (1.0 + 13.0 * st.z) * pos_vec - 0.0003 * comet.comet_vel.xyz + 0.8 * v.data.b * sun_vec;
        pos = pos_vec * 0.001 + vel * t + acc * t * t;
        let scale = 0.5 + 1.0 / (t + 1.0) + 10.0 * st.z;
        sprite_size = scale * 0.0075 * st.w;
        let sprite_length = 0.0001 + 0.1 * pow(clamp(t * 2.0, 0.0, 1.0), 1.5);
        sprite_dir = 0.5 * (1.5 * sun_vec + normalize(acc) * 0.67) * sprite_length;
    }

    // SE's sprite size is in km, its view space too: here the view space is in render units, so
    // the size scales by the tail length in render units.
    let m = get_world_from_local(v.instance_index);
    let size_v = sprite_size * length(m[0].xyz);
    let view_n = view.view_from_world * (m * vec4<f32>(pos, 1.0));
    let view_o = view.view_from_world * (m * vec4<f32>(pos + sprite_dir, 1.0));
    var view_pos = view_n;
    var frag = pos;
    if tex.x != 0.0 {
        view_pos = view_o;
        frag = pos + sprite_dir;
    }

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
    o.frag_pos = frag * st.x * KM2AU;
    o.tex = tex;
    o.color = color * fade;
    o.sprite_size = 3.0 * sprite_size * st.x * KM2AU;
    return o;
}

// Henyey-Greenstein phase function.
fn hg(cosa: f32) -> f32 {
    let g = -0.3;
    let x = 1.0 + g * g - 2.0 * g * cosa;
    return 0.25 * (1.0 - g * g) * inverseSqrt(x * x * x);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let eye_rel = comet.eye_pos.xyz + in.frag_pos;
    let sun_rel = comet.sun_pos.xyz + in.frag_pos;
    let eye_dist = length(eye_rel);
    let sun_dist = length(sun_rel);

    var tc = in.tex.xy * 2.0 - 1.0;
    let s = sign(tc.x);
    tc.x = s * min(s * (tc.x - s * in.tex.z) / (in.tex.z - 1.0), 0.0);
    let r = length(tc);
    var bright = exp(-1.5 * r) * clamp(1.0 - r, 0.0, 1.0);
    bright *= clamp(eye_dist / in.sprite_size, 0.0, 1.0) * comet.sun_col.a / (sun_dist * sun_dist + 0.05);
    bright = pow(bright, 0.35);

    // Gas sprites: just their glow.
    var c = in.color * bright;
    // Dust sprites: sunlight scattered with the Henyey-Greenstein phase function.
    if in.tex.w != 0.0 {
        let cosa = dot(eye_rel, sun_rel) / (eye_dist * sun_dist);
        c *= comet.sun_col * (bright / comet.sun_col.a * hg(cosa));
    }
    return vec4<f32>(c.rgb, 0.0);
}
