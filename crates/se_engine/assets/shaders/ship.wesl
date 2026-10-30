// Spacecraft hull material (SE's ship.glsl): Lambert diffuse from up to 4 lights, the ambient
// level, an emissive map (windows, lights) and optional alpha cut-out (frame lattices).
// Textures are sRGB, so sampling gives linear colour. Two-sided: the normal is
// flipped on back faces.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Ship {
    // xyz: unit direction towards each light; w: 1 for a light in use
    light_dir: array<vec4<f32>, 4>,
    // rgb: light colour * intensity * eclipse shadow
    light_color: array<vec4<f32>, 4>,
    // rgb: diffuse tint; w: ambient level
    tint: vec4<f32>,
    // rgb: emissive colour * brightness; w: alpha cut-off (negative = opaque)
    emis: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> ship: Ship;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diff_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var emis_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var emis_smp: sampler;

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
fn fragment(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let tex = textureSample(diff_tex, diff_smp, in.uv);
    if ship.emis.w >= 0.0 && tex.a < ship.emis.w {
        discard;
    }
    var n = normalize(in.world_normal);
    if !front {
        n = -n;
    }
    let albedo = tex.rgb * ship.tint.rgb;
    // FragColor = (AmbientParams.x * diffColor + sum of LightColor[i] * cosI * diffColor).
    var lit = vec3<f32>(ship.tint.w);
    for (var i = 0; i < 4; i++) {
        if ship.light_dir[i].w > 0.5 {
            lit += ship.light_color[i].rgb * max(dot(n, normalize(ship.light_dir[i].xyz)), 0.0);
        }
    }
    let emissive = textureSample(emis_tex, emis_smp, in.uv).rgb * ship.emis.rgb;
    return vec4<f32>(albedo * lit + emissive, 1.0);
}
