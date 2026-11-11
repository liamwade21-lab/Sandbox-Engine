// The sandbox's props and the weapon in the player's hands (sandbox/light.rs `PropMaterial`): a textured mesh lit as the
// ground is (ground_light.wesl `lit_surface`), from the light worked out at the player's place this frame (hero_light.wesl
// writes it before the main pass: daylight, cloud shadow, night, haze), as the hero's crater rims take his. Nothing in a
// material changes a frame: the light is in the two storage buffers.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import super::ground_light::{PointLight, lit_surface};
import super::hero_frame::{HeroFrame, HeroLight};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> frame: HeroFrame;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> prop_light: HeroLight;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var diff_smp: sampler;
// rgb the colour over the texture's; a the alpha below which a pixel is cut out (0: none are).
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var<uniform> tint: vec4<f32>;
// x: light of its own, as a share of full daylight on white (a muzzle flash, a beam, a screen); y: 1 when the mesh is seen
// from both sides (its back faces are lit as its front).
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var<uniform> glow: vec4<f32>;

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
    let texel = textureSample(diff_tex, diff_smp, in.uv);
    if tint.a > 0.0 && texel.a < tint.a {
        discard;
    }
    var n = normalize(in.world_normal);
    if glow.y > 0.5 && !front {
        n = -n;
    }
    let albedo = texel.rgb * tint.rgb;
    let pl = PointLight(prop_light.sun.rgb, prop_light.sky.rgb, prop_light.flat_e.rgb, prop_light.haze_t.rgb, prop_light.haze_a.rgb);
    var c = lit_surface(pl, normalize(frame.light.sun_dir.xyz), albedo, n, normalize(frame.up.xyz), 0.0, 0.0);
    // (Its own light, in the ground's scale: the Sun's light on a white ground facing it, times the share asked for.)
    if glow.x > 0.0 {
        let day = max(max(prop_light.sun.r, prop_light.sun.g), max(prop_light.sun.b, 1.0e-6));
        c += albedo * (glow.x * max(day, glow.z) / 3.14159);
    }
    return vec4<f32>(c, 1.0);
}
