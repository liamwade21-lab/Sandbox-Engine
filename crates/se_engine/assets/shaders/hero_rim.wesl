// The Viltrumite mod's crater rims (viltrumite/impact.rs `RimMaterial`): `nibsuper_ground_break1`, broken ground, half-sunk round a
// crater. Lit as the ground is (ground_light.wesl `lit_surface`), from his own light of this frame (hero_light.wesl writes it before
// the main pass: daylight, cloud shadow, night, haze), as the trees and the buildings take theirs: a rim is within 2 km of the camera,
// near him. Nothing in its material changes a frame.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world, mesh_tangent_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import super::ground_light::{PointLight, lit_surface};
import super::hero_frame::{HeroFrame, HeroLight};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> frame: HeroFrame;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> hero_light: HeroLight;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var diff_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var norm_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var norm_smp: sampler;
// rgb the rock's colour over its texture's.
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var<uniform> tint: vec4<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world_tangent: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.world_tangent = mesh_tangent_local_to_world(m, v.tangent, v.instance_index);
    o.uv = v.uv;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let n0 = normalize(in.world_normal);
    let t = normalize(in.world_tangent.xyz - n0 * dot(in.world_tangent.xyz, n0));
    let b = cross(n0, t) * in.world_tangent.w;
    let bump = textureSample(norm_tex, norm_smp, in.uv).xyz * 2.0 - 1.0;
    let n = normalize(t * bump.x + b * bump.y + n0 * max(bump.z, 0.2));
    let albedo = textureSample(diff_tex, diff_smp, in.uv).rgb * tint.rgb;
    let pl = PointLight(hero_light.sun.rgb, hero_light.sky.rgb, hero_light.flat_e.rgb, hero_light.haze_t.rgb, hero_light.haze_a.rgb);
    let c = lit_surface(pl, normalize(frame.light.sun_dir.xyz), albedo, n, normalize(frame.up.xyz), 0.0, 0.0);
    return vec4<f32>(c, 1.0);
}
