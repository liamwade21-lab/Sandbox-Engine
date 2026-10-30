// SE's lens flares (lens_flare.glsl), drawn by bloom.rs after the bloom chain is made, before the
// combine: a probe pass (`lens_flare_probe.wesl`) reads, for each sun, the frame and the bloom at
// its centre into one texel (FrameTex / BloomTex, `Params2`), and the sprites (quads already placed
// on the screen by `lensflare.rs`) are scaled by it, so terrain, clouds, rings or ships over the sun
// hide its flare and the atmosphere's reddening tints it.

@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(0) @binding(2) var probe_tex: texture_2d<f32>;

struct SpriteIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    // rgb: element colour x brightness, w: the sun's probe texel
    @location(2) color: vec4<f32>,
};

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) @interpolate(flat) source: u32,
};

@vertex
fn sprite_vertex(v: SpriteIn) -> SpriteOut {
    var o: SpriteOut;
    o.clip = vec4<f32>(v.position, 0.0, 1.0);
    o.uv = v.uv;
    o.color = v.color.rgb;
    o.source = u32(v.color.w + 0.5);
    return o;
}

@fragment
fn sprite_fragment(in: SpriteOut) -> @location(0) vec4<f32> {
    let modulate = textureLoad(probe_tex, vec2<i32>(i32(in.source), 0), 0).rgb;
    let texel = textureSample(atlas, atlas_sampler, in.uv).rgb;
    // Added to the HDR frame (the pipeline blends One + One).
    return vec4<f32>(texel * max(in.color * modulate, vec3<f32>(0.0)), 0.0);
}
