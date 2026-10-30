// Comet coma and tails: additive HDR glow. Geometry is a unit mesh scaled by the
// entity (uniform MeterScale) and oriented so local +Y points anti-sunward and
// local +X points in the direction the dust lags behind the orbit.
// Output alpha is 0 so that AlphaMode::Add (premultiplied) adds colour only.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Comet {
    // rgb tint, w = intensity (already scaled by distance and camera fade)
    color: vec4<f32>,
    // x: 0 = coma (sphere), 1 = tail (tube along uv.x = 0..1 from nucleus to tip)
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> comet: Comet;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_pos = world.xyz;
    o.normal = normalize((m * vec4<f32>(v.normal, 0.0)).xyz);
    o.uv = v.uv;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    // The camera sits at the render-space origin (floating origin).
    let v = normalize(-in.world_pos);
    let n = normalize(in.normal);
    let facing = abs(dot(n, v));

    var a: f32;
    if comet.params.x < 0.5 {
        // Coma: bright core, soft limb.
        a = pow(facing, 2.0);
    } else {
        // Tail: fade in at the nucleus, fade out towards the tip, soft silhouette.
        let s = in.uv.x;
        let axial = smoothstep(0.0, 0.08, s) * pow(1.0 - s, 1.3);
        a = pow(facing, 1.5) * axial;
    }
    return vec4<f32>(comet.color.rgb * comet.color.w * a, 0.0);
}
