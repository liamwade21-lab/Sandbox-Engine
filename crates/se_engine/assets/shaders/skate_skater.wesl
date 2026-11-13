// The Skate 3 mod's skater and his board (skate/draw.rs `SkaterMaterial`): Skate 3's default skater, skinned by a bone palette of
// his own (one storage buffer written in place each frame), lit as the sandbox's props are (sandbox_prop.wesl: ground_light.wesl
// `lit_surface` with the light at the player's place). The meshes' transform is the identity: the palette takes a bind-pose
// vertex straight to the camera-relative render frame. (The weapon's shader, sandbox_viewmodel.wesl, without its narrower field
// of view and its nearest depth slice.)

import bevy_pbr::render::mesh_view_bindings::view;
import super::ground_light::{PointLight, lit_surface};
import super::hero_frame::{HeroFrame, HeroLight};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> frame: HeroFrame;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> prop_light: HeroLight;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var diff_smp: sampler;
// rgb the colour over the texture's; a the alpha below which a pixel is cut out (0: none are).
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var<uniform> tint: vec4<f32>;
// x: light of its own, as a share of full daylight on white; y: 1 when the mesh is seen from both sides; z the least that
// daylight counts as; w unused.
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var<uniform> glow: vec4<f32>;
// Per joint: bind-pose model space (Skate 3's metres) to the render frame.
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var<storage, read> palette: array<mat4x4<f32>>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let w = v.weights;
    let m = palette[v.joints.x] * w.x + palette[v.joints.y] * w.y + palette[v.joints.z] * w.z + palette[v.joints.w] * w.w;
    let world = (m * vec4<f32>(v.position, 1.0)).xyz;
    var o: VertexOut;
    o.clip = view.clip_from_world * vec4<f32>(world, 1.0);
    o.world_normal = (m * vec4<f32>(v.normal, 0.0)).xyz;
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
    // By night the street lamps light it from above, as they do the road, the cars and the hero (hero.wesl).
    let up = normalize(frame.up.xyz);
    let night = 1.0 - smoothstep(-0.12, 0.03, dot(up, normalize(frame.light.sun_dir.xyz)));
    c += albedo * frame.town.rgb * night * (0.25 + 0.2 * max(dot(n, up), 0.0));
    if glow.x > 0.0 {
        let day = max(max(prop_light.sun.r, prop_light.sun.g), max(prop_light.sun.b, 1.0e-6));
        c += albedo * (glow.x * max(day, glow.z) / 3.14159);
    }
    return vec4<f32>(c, 1.0);
}
