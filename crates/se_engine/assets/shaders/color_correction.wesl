// SE's final colour correction (`color_correction.rs`, hsl.glh `ColorCorrentionLDR`), on the
// tone-mapped linear image as SE's hdr_final.glsl applies it.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct ColorCorrection {
    // x: brightness - 1, y: contrast, z: saturation - 1 (vibrance), w: 1 / gamma
    bcsg: vec4<f32>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> cc: ColorCorrection;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(scene, scene_sampler, in.uv);
    // Contrast and brightness.
    var c = mix(vec3<f32>(0.5), src.rgb, cc.bcsg.y) + cc.bcsg.xxx;
    // Vibrance: extrapolate from luma by 1 + (S - 1) (1 - the pixel's saturation).
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sat = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
    c = mix(vec3<f32>(luma), c, 1.0 + cc.bcsg.z * (1.0 - sat));
    // Gamma.
    c = pow(max(c, vec3<f32>(1e-6)), cc.bcsg.www);
    return vec4<f32>(c, src.a);
}
