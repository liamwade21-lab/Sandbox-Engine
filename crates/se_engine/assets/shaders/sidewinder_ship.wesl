// The Sidewinder's hull, gear, cockpit and chairs (sidewinder/material.rs `ShipMaterial`): Elite's models lit by the sandbox's
// light record at the player's place (hero_light.wesl writes it before the main pass), as the props are (sandbox_prop.wesl).
// Nothing in a material changes a frame: the light is in the two storage buffers.
//
// The fragment stage is two functions: `surface` (what the material is at the pixel: base colour, normal, roughness,
// metalness, emissive, ambient occlusion) and `light` (how it is lit). Today only the base colour is real and the lighting is
// the props'; Elite's material maths per shader family and its lit maths replace them (T4b).

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world, mesh_tangent_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import super::ground_light::{PointLight, lit_surface};
import super::hero_frame::{HeroFrame, HeroLight};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> frame: HeroFrame;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> ship_light: HeroLight;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var base_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var base_smp: sampler;

struct ShipParams {
    // rgb the base colour when it is one colour.
    colour: vec4<f32>,
    // x 1 when the base colour is the texture's.
    flags: vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var<uniform> params: ShipParams;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // xyz Elite's tangent, w the frame's sign (both already in the mod's axes).
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) uv2: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_tangent: vec4<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) uv2: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.world_tangent = mesh_tangent_local_to_world(m, v.tangent, v.instance_index);
    o.uv0 = v.uv0;
    o.uv1 = v.uv1;
    o.uv2 = v.uv2;
    return o;
}

// What the material is at a pixel.
struct Surface {
    base: vec3<f32>,
    normal: vec3<f32>,
    roughness: f32,
    metalness: f32,
    emissive: vec3<f32>,
    ao: f32,
};

fn surface(in: VertexOut) -> Surface {
    var s: Surface;
    s.base = params.colour.rgb;
    if params.flags.x > 0.5 {
        s.base = textureSample(base_tex, base_smp, in.uv0).rgb;
    }
    s.normal = normalize(in.world_normal);
    s.roughness = 1.0;
    s.metalness = 0.0;
    s.emissive = vec3<f32>(0.0);
    s.ao = 1.0;
    return s;
}

// How it is lit: the ground's light at the player's place (sandbox_prop.wesl's).
fn light(s: Surface) -> vec3<f32> {
    let sun_dir = normalize(frame.light.sun_dir.xyz);
    let up = normalize(frame.up.xyz);
    let pl = PointLight(ship_light.sun.rgb, ship_light.sky.rgb, ship_light.flat_e.rgb, ship_light.haze_t.rgb, ship_light.haze_a.rgb);
    var c = lit_surface(pl, sun_dir, s.base, s.normal, up, 0.0, 0.0);
    // By night the street lamps light it from above, as they do the road, the cars, the props and the hero.
    let night = 1.0 - smoothstep(-0.12, 0.03, dot(up, sun_dir));
    c += s.base * frame.town.rgb * night * (0.25 + 0.2 * max(dot(s.normal, up), 0.0));
    return c * s.ao + s.emissive;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(light(surface(in)), 1.0);
}
