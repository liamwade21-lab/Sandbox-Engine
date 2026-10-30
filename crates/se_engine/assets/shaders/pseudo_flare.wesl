// Pseudo lens flare (`lensflare.rs`, SE's `PseudoFlarePreset`): John Chapman's screen-space flare. The
// bright parts of the frame are mirrored through the screen centre into a row of ghosts and a halo,
// with chromatic distortion and the lens colour gradient, and added to the HDR frame.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Pseudo {
    // x: ghost count, y: ghost dispersal, z: ghost brightness, w: ghost blur
    ghosts: vec4<f32>,
    // x: halo radius, y: halo brightness, z: halo blur, w: chromatic distortion
    halo: vec4<f32>,
    // x: overall brightness, y: width / height, z: brightness cap of the source
    params: vec4<f32>,
    lens_color: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> pf: Pseudo;

// The flare source: the frame, capped like SE's bloom source.
fn source(uv: vec2<f32>) -> vec3<f32> {
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return vec3<f32>(0.0);
    }
    return min(textureSampleLevel(scene, scene_sampler, uv, 0.0).rgb, vec3<f32>(pf.params.z));
}

// Blurred source with the colour channels pulled apart along `dir` (chromatic distortion).
fn distorted(uv: vec2<f32>, dir: vec2<f32>, blur: f32) -> vec3<f32> {
    let k = dir * pf.halo.w * 0.004;
    let r = blur * 0.004;
    let offs = array<vec2<f32>, 5>(vec2(0.0), vec2(r, 0.0), vec2(-r, 0.0), vec2(0.0, r), vec2(0.0, -r));
    var c = vec3<f32>(0.0);
    for (var i = 0; i < 5; i = i + 1) {
        let p = uv + offs[i];
        c = c + vec3<f32>(source(p - k).r, source(p).g, source(p + k).b);
    }
    return c / 5.0;
}

fn lens_color(x: f32) -> vec3<f32> {
    let f = clamp(x, 0.0, 1.0) * 15.0;
    let i = u32(floor(f));
    let j = min(i + 1u, 15u);
    return mix(pf.lens_color[i].rgb, pf.lens_color[j].rgb, fract(f));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let base = textureSampleLevel(scene, scene_sampler, in.uv, 0.0);
    // Mirrored coordinates: a bright spot at uv shows ghosts on the far side of the centre.
    let tc = vec2<f32>(1.0) - in.uv;
    let ghost_vec = (vec2<f32>(0.5) - tc) * pf.ghosts.y;
    let dir = normalize(ghost_vec + vec2<f32>(1.0e-6));
    let norm = length(vec2<f32>(0.5));

    var ghosts = vec3<f32>(0.0);
    let n = i32(pf.ghosts.x);
    for (var i = 0; i < n; i = i + 1) {
        let off = fract(tc + ghost_vec * f32(i));
        let w = pow(1.0 - length(vec2<f32>(0.5) - off) / norm, 10.0);
        ghosts = ghosts + distorted(off, dir, pf.ghosts.w) * w;
    }
    ghosts = ghosts * lens_color(length(vec2<f32>(0.5) - tc) / norm);

    // Halo: a ring of radius `HaloRadius` (aspect corrected so it stays round).
    let hv = normalize(ghost_vec * vec2<f32>(pf.params.y, 1.0) + vec2<f32>(1.0e-6)) * vec2<f32>(1.0 / pf.params.y, 1.0)
        * pf.halo.x * 0.5;
    let hw = pow(1.0 - length(vec2<f32>(0.5) - fract(tc + hv)) / norm, 5.0);
    let halo = distorted(tc + hv, dir, pf.halo.z) * hw;

    let flare = (ghosts * pf.ghosts.z + halo * pf.halo.y) * pf.params.x;
    return vec4<f32>(base.rgb + flare, base.a);
}
