// Catalog model meshes (asteroids, comets): Lambert diffuse from one directional
// star with a small flat ambient term. The texture is sRGB, so sampling gives linear.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Model {
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    base_color: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> model: Model;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var model_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var model_smp: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.uv = v.uv;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let l = normalize(model.sun_dir.xyz);
    let ndl = max(dot(n, l), 0.0);
    let albedo = textureSample(model_tex, model_smp, in.uv).rgb * model.base_color.rgb;
    let sun = model.sun_color.rgb * model.sun_dir.w * ndl;
    let ambient = model.base_color.w * albedo;
    return vec4<f32>(albedo * sun + ambient, 1.0);
}
