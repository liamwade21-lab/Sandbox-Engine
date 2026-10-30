// Spacecraft hull material: Lambert diffuse from the system's star, a small ambient
// term, an emissive map (windows, lights) and optional alpha cut-out (frame lattices).
// Textures are sRGB, so sampling gives linear colour. Two-sided: the normal is
// flipped on back faces.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Ship {
    // xyz: unit direction towards the star; w: star intensity
    sun_dir: vec4<f32>,
    // rgb: star colour
    sun_color: vec4<f32>,
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
    let l = normalize(ship.sun_dir.xyz);
    let ndl = max(dot(n, l), 0.0);
    let albedo = tex.rgb * ship.tint.rgb;
    let sun = ship.sun_color.rgb * ship.sun_dir.w * ndl;
    // Faint hemispheric fill so the night side is not pure black.
    let fill = ship.tint.w * (0.6 + 0.4 * (0.5 + 0.5 * dot(n, l)));
    let emissive = textureSample(emis_tex, emis_smp, in.uv).rgb * ship.emis.rgb;
    return vec4<f32>(albedo * (sun + vec3<f32>(fill)) + emissive, 1.0);
}
