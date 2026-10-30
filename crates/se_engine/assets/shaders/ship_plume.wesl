// Engine exhaust plume: an additive teardrop mesh along local +Z (uv.x = 0 at the nozzle,
// 1 at the tip, uv.y = angle / 2pi). Brightness follows the view-facing term so the
// silhouette is soft, with a hot white core near the nozzle and a slight flicker.
// Output alpha is 0 so that AlphaMode::Add (premultiplied) adds colour only.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Plume {
    // rgb colour, w intensity
    color: vec4<f32>,
    // x: level 0..1, y: time (s), z: seed, w: unused
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> plume: Plume;

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
    let view = normalize(-in.world_pos);
    let n = normalize(in.normal);
    let facing = abs(dot(n, view));
    let s = in.uv.x;
    let level = plume.params.x;
    let t = plume.params.y;
    let flicker = 0.88 + 0.12 * sin(t * 37.0 + plume.params.z * 11.0 + s * 9.0 + in.uv.y * 6.2831853 * 3.0);
    let axial = smoothstep(0.0, 0.04, s) * pow(max(1.0 - s, 0.0), 1.25);
    let body = pow(facing, 1.6) * axial * flicker;
    let core = exp(-s * 5.0) * pow(facing, 3.0);
    let col = mix(plume.color.rgb, vec3<f32>(1.0), clamp(core * 0.8, 0.0, 1.0));
    let i = plume.color.w * level * (body + 1.5 * core);
    return vec4<f32>(col * i, 0.0);
}
