// SpaceEngine's bloom (hdr_down.glsl, hdr_gauss_blur.glsl, hdr_combine.glsl), run on the HDR frame
// before tone mapping (bloom.rs):
//   1. downsample the frame into a 6-level chain of 2x2 averages (level 0 at half resolution); the
//      first pass clamps the frame to FrameDynamicRange and the bloom source's luma to
//      BloomDynamicRange,
//   2. blur every level with SE's 9-tap Gaussian (bilinear taps), horizontally then vertically,
//      subtracting BloomThreshold in the horizontal pass,
//   3. add BloomBright x the normalised weighted sum of the six levels to the frame (SE's "sonic
//      bloom" weights 0.50, 0.60, 0.60, 0.45, 0.35, 0.23).

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct SeBloom {
    // x: BloomBright, y: BloomThreshold, z: FrameDynamicRange, w: BloomDynamicRange.
    params: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> bloom: SeBloom;

const TO_GRAY: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);

// hdr_down.glsl: the average of four bilinear taps one source texel apart.
fn down4(uv: vec2<f32>, first: bool) -> vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    var c = vec4<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let o = vec2<f32>(f32(-(i & 1)), f32(-(i >> 1u)));
        var s = textureSampleLevel(src, src_sampler, uv + o * t, 0.0);
        if first {
            s = clamp(s, vec4<f32>(0.0), vec4<f32>(bloom.params.z));
        }
        c += max(s, vec4<f32>(0.0));
    }
    return 0.25 * c;
}

@fragment
fn down_first(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var c = down4(in.uv, true);
    // Limit the bloom source's brightness.
    let luma = dot(c.rgb, TO_GRAY);
    c = vec4<f32>(c.rgb * clamp(bloom.params.w / (luma + 1.0e-10), 0.0, 1.0), 1.0);
    return c;
}

@fragment
fn down(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(down4(in.uv, false).rgb, 1.0);
}

// hdr_gauss_blur.glsl KERNEL 2: 9 taps as 5 bilinear fetches, normalised to keep the energy.
fn gauss(uv: vec2<f32>, dir: vec2<f32>, threshold: f32) -> vec4<f32> {
    let step = dir / vec2<f32>(textureDimensions(src));
    let w = array<f32, 3>(0.2270270270, 0.3162162162, 0.0702702703);
    let o = array<f32, 3>(0.0, 1.3846153846, 3.2307692308);
    var c = max(textureSampleLevel(src, src_sampler, uv, 0.0).rgb - vec3<f32>(threshold), vec3<f32>(0.0)) * w[0];
    for (var i = 1; i < 3; i = i + 1) {
        let a = textureSampleLevel(src, src_sampler, uv + o[i] * step, 0.0).rgb;
        let b = textureSampleLevel(src, src_sampler, uv - o[i] * step, 0.0).rgb;
        c += w[i] * (max(a - vec3<f32>(threshold), vec3<f32>(0.0)) + max(b - vec3<f32>(threshold), vec3<f32>(0.0)));
    }
    return vec4<f32>(c, 1.0);
}

@fragment
fn blur_h(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return gauss(in.uv, vec2<f32>(1.0, 0.0), bloom.params.y);
}

@fragment
fn blur_v(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return gauss(in.uv, vec2<f32>(0.0, 1.0), 0.0);
}

// hdr_combine.glsl BLOOM: added to the frame (the pipeline blends One + One).
@fragment
fn combine(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let w = array<f32, 6>(0.50, 0.60, 0.60, 0.45, 0.35, 0.23);
    var c = vec3<f32>(0.0);
    for (var k = 0; k < 6; k = k + 1) {
        c += textureSampleLevel(src, src_sampler, in.uv, f32(k)).rgb * w[k];
    }
    let bn = 1.0 / (0.5 + 0.6 + 0.6 + 0.45 + 0.35 + 0.23);
    return vec4<f32>(c * bn * bloom.params.x, 0.0);
}

// The blurred frame behind the interface's windows (gui.glsl BLUR_BACKGROUND, `BackTex`), tone-mapped
// as SE does (1 - exp(-exposure x c)); skin.rs adds BackgroundBrightness of it to every window.
@fragment
fn backdrop(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src, src_sampler, in.uv, 0.0).rgb;
    return vec4<f32>(1.0 - exp(-c), 1.0);
}
