// Aurora: curtains of rays around a magnetic pole plus a faint glowing band at
// their base (what an oval looks like from straight above). Additive.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::globals;

struct Aurora {
    // xyz: unit direction to the star; w: SE's AuroraBright.
    sun_dir: vec4<f32>,
    // rgb: lower-edge colour, w: brightness
    bottom: vec4<f32>,
    // rgb: multiplier for the upper edge
    top: vec4<f32>,
    // x: flash frequency, y: drift speed, z: seed
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> aurora: Aurora;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // x: position along the ring (0..1), y: height in the curtain (0..1) or across the band (-1..1)
    @location(2) uv: vec2<f32>,
    // x: ring fraction, y: 0 curtain / 1 base band
    @location(3) uv1: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) up: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) uv1: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let world_from_local = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.up = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.uv = v.uv;
    o.uv1 = v.uv1;
    return o;
}

fn hash11(n: f32) -> f32 {
    return fract(sin(n * 127.1) * 43758.5453);
}

// Smooth periodic-free 1D value noise in [0, 1].
fn noise1(x: f32) -> f32 {
    let i = floor(x);
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(hash11(i), hash11(i + 1.0), u);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let up = normalize(in.up);
    let l = normalize(aurora.sun_dir.xyz);
    // Only on the night side (fading through twilight).
    let night = 1.0 - smoothstep(-0.2, 0.08, dot(up, l));
    if night <= 0.0 {
        discard;
    }
    let seed = aurora.params.z * 97.0 + in.uv1.x * 13.0;
    let t = globals.time * (0.15 + 0.05 * aurora.params.y);
    let s = in.uv.x;
    // Large folds drifting along the oval, and occasional brightenings.
    let folds = smoothstep(0.25, 0.85, noise1(s * 18.0 - t * 0.6 + seed) * 0.7 + noise1(s * 47.0 + t + seed) * 0.3);
    let flash = 1.0 + 0.35 * sin(globals.time * (0.3 + aurora.params.x * 0.02) + seed) * noise1(s * 9.0 + seed);

    var col: vec3<f32>;
    if in.uv1.y < 0.5 {
        let v = clamp(in.uv.y, 0.0, 1.0);
        // Sharp lower edge, slow fade upwards.
        let vprof = smoothstep(0.0, 0.05, v) * exp(-v * 2.2);
        // Fine vertical rays shimmering along the curtain.
        let rays = 0.3 + 0.7 * pow(noise1(s * 900.0 + t * 4.0 + seed), 2.0) * noise1(s * 260.0 - t * 2.0 + seed);
        let tint = mix(aurora.bottom.rgb, aurora.bottom.rgb * aurora.top.rgb, smoothstep(0.1, 0.8, v));
        col = tint * vprof * rays * folds * 2.0;
    } else {
        let a = in.uv.y;
        col = aurora.bottom.rgb * exp(-a * a * 4.0) * (0.25 + 0.75 * folds) * 0.5;
    }
    let i = aurora.bottom.w * aurora.sun_dir.w * night * flash;
    return vec4<f32>(col * i, 0.0);
}
