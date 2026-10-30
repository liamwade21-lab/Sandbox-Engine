// A sprite-model galaxy's front texture (`FrontTexture`, `<name>.png`) as a flat disc over its
// sprites, as SE's galaxy_plane.glsl draws it: faded where the eye is within a model unit of the
// pixel and when the disc is seen edge-on, the colour raised to 1 / DSOGamma (ColorCorrentionLDR with
// the default brightness, contrast and saturation), premultiplied: the texture's light adds, its
// alpha (the dust lanes) hides the sprites behind.
//
// The corners are drawn at infinity like the sprites (w = 0), from their offsets to the camera in
// parsecs, so the disc keeps its true perspective at any distance.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_world_from_local;

struct PlaneParams {
    // xyz: galaxy centre relative to the camera (pc), w: model fade (0..1)
    center_rel: vec4<f32>,
    // The disc's half-axes (pc, universe frame): the texture's columns and its rows downwards.
    axis_x: vec4<f32>,
    axis_y: vec4<f32>,
    // x: brightness, y: 1 / gamma, z: parsecs per model unit
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> plane: PlaneParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var front: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var front_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) rel: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let rel = plane.center_rel.xyz + plane.axis_x.xyz * v.position.x + plane.axis_y.xyz * v.position.y;
    let world_from_local = get_world_from_local(v.instance_index);
    let dir = (world_from_local * vec4<f32>(rel, 0.0)).xyz;
    var o: VertexOut;
    o.clip = view.clip_from_world * vec4<f32>(dir, 0.0);
    o.uv = v.uv;
    o.rel = rel;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let dist = max(length(in.rel), 1e-9);
    let n = normalize(cross(plane.axis_x.xyz, plane.axis_y.xyz));
    let fade = smoothstep(0.0, 1.0, dist / plane.params.z)
        * smoothstep(0.0, 1.0, abs(dot(in.rel, n)) / dist * 2.5 - 0.2)
        * plane.center_rel.w;
    let tex = textureSample(front, front_sampler, in.uv);
    let rgb = pow(max(tex.rgb, vec3<f32>(1e-6)), vec3<f32>(plane.params.y)) * plane.params.x * fade;
    return vec4<f32>(rgb, clamp(tex.a * fade, 0.0, 1.0));
}
