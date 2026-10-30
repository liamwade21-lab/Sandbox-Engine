// SE's lens flare modulation (lens_flare.glsl FrameTex / BloomTex / Params2), bloom.rs's probe pass:
// for each sun, the frame and the bloom at its centre, limited and gamma-corrected, into one texel
// that the flare sprites (`lens_flare.wesl`) are scaled by.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

const TO_GRAY: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);

struct Probe {
    // Per sun: xy screen position (uv), z bloom level, w frame (0) to bloom (1) mix.
    sources: array<vec4<f32>, 4>,
    // x: the frame's brightness limit (BloomDynamicRange, as hdr_down.glsl limits the bloom),
    // y: the port-to-SE brightness gain of the samples (`lensflare.rs` STAR_FRAME_GAIN).
    params: vec4<f32>,
};

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var probe_sampler: sampler;
@group(0) @binding(3) var<uniform> probe_u: Probe;

@fragment
fn probe(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let s = probe_u.sources[min(u32(in.position.x), 3u)];
    let gain = probe_u.params.y;
    // SE's bloom is limited to BloomDynamicRange where it is made; the gained one is limited here.
    var from_bloom = textureSampleLevel(bloom, probe_sampler, s.xy, s.z).rgb * gain;
    from_bloom *= clamp(probe_u.params.x / (dot(from_bloom, TO_GRAY) + 1.0e-10), 0.0, 1.0);
    var from_frame = clamp(textureSampleLevel(frame, probe_sampler, s.xy, 0.0).rgb * gain, vec3<f32>(0.0), vec3<f32>(100.0));
    from_frame *= clamp(probe_u.params.x / (dot(from_frame, TO_GRAY) + 1.0e-10), 0.0, 1.0);
    // Small suns read the frame, large ones the bloom; the modulation is limited and gamma-corrected.
    var m = mix(from_frame, from_bloom, s.w);
    m *= smoothstep(3.0, 10.0, dot(m, TO_GRAY));
    return vec4<f32>(pow(m, vec3<f32>(1.0 / 2.2)), 1.0);
}
