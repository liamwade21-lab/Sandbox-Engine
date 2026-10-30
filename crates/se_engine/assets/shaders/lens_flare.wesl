// Lens flare sprites (`lensflare.rs`): quads already placed on the screen by the CPU (NDC xy), drawn
// in front of everything and added to the HDR frame. Colour = atlas texel * vertex colour.

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var atlas: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var atlas_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    var o: VertexOut;
    // Reverse-Z: depth 1 is the near plane, so the sprite passes the depth test everywhere.
    o.clip = vec4<f32>(v.position.xy, 1.0, 1.0);
    o.uv = v.uv;
    o.color = v.color.rgb;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv).rgb;
    // Premultiplied blending in Bevy: alpha 0 makes AlphaMode::Add purely additive.
    return vec4<f32>(texel * in.color, 0.0);
}
