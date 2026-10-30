// SE's tone mapping and final colour correction (`color_correction.rs`): hdr_tone.glsl with the chosen
// hdrt_*_tone.glh `ToneMapping`, then hdr_final.glsl (hsl.glh `ColorCorrentionLDR`, `DITHERING`).
// Two passes as SE's: `fragment` (hdr_tone) tone-maps into the scratch texture, `final_image` (hdr_final,
// with its `SHARP` textureFastCAS) corrects it back into the main one.
// Bevy's tone mapping pass runs before this one as `Tonemapping::Linear`, so the input is the HDR frame with the
// exposure applied; the output is linear (the swapchain encodes sRGB), as the port's earlier AgX look.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct ColorCorrection {
    // x: brightness - 1, y: contrast, z: saturation - 1 (vibrance), w: 1 / gamma
    bcsg: vec4<f32>,
    // x: tone shader (`TONE_SHADERS` index), y: dithering, z: auto exposure on
    mode: vec4<u32>,
    // x: `Sharpness` (0 no sharpening)
    sharp: vec4<f32>,
};

// The auto exposure state (`auto_exposure.wesl`): x the compensation in stops (positive darker).
struct AutoState {
    comp: vec4<f32>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> cc: ColorCorrection;
@group(0) @binding(3) var<storage, read> auto_state: AutoState;

fn srgb_encode(c: f32) -> f32 {
    if c > 0.0031308 {
        return 1.055 * pow(c, 0.41667) - 0.055;
    }
    return c * 12.92;
}

fn srgb_encode3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(srgb_encode(c.r), srgb_encode(c.g), srgb_encode(c.b));
}

fn rec709_encode(c: f32) -> f32 {
    if c < 0.018 {
        return c * 4.5;
    }
    return 1.099 * pow(c, 0.45) - 0.099;
}

// hdrt_ACES_Fitted_tone.glh (BakingLab's fit).
fn tone_aces(c: vec3<f32>) -> vec3<f32> {
    let bias = 0.35;
    let m_in = mat3x3<f32>(
        0.59719 * bias, 0.07600 * bias, 0.02840 * bias,
        0.35458 * bias, 0.90834 * bias, 0.13383 * bias,
        0.04823 * bias, 0.01566 * bias, 0.83777 * bias,
    );
    let m_out = mat3x3<f32>(
        1.60475, -0.10208, -0.00327,
        -0.53108, 1.10813, -0.07276,
        -0.07367, -0.00605, 1.07602,
    );
    var x = m_in * c;
    let a = x * (x + 0.0245786) - 0.000090537;
    let b = x * (0.983729 * x + 0.4329510) + 0.238081;
    x = m_out * (a / b);
    return pow(max(x, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

// hdrt_AgX_tone.glh / hdrt_AgX_Punchy_tone.glh (iolite's minimal AgX, white 16.2917).
fn tone_agx(v: vec3<f32>, punchy: bool) -> vec3<f32> {
    let agx_mat = mat3x3<f32>(
        0.842479062253094, 0.0423282422610123, 0.0423756549057051,
        0.0784335999999992, 0.878468636469772, 0.0784336,
        0.0792237451477643, 0.0791661274605434, 0.879142973793104,
    );
    let agx_inv = mat3x3<f32>(
        1.19687900512017, -0.0528968517574562, -0.0529716355144438,
        -0.0980208811401368, 1.15190312990417, -0.0980434501171241,
        -0.0990297440797205, -0.0989611768448433, 1.15107367264116,
    );
    let min_ev = -12.47393;
    let max_ev = log2(16.2917);
    var x = agx_mat * (max(v, vec3<f32>(0.0)) * 0.75);
    x = clamp(log2(max(x, vec3<f32>(1e-10))), vec3<f32>(min_ev), vec3<f32>(max_ev));
    x = (x - min_ev) / (max_ev - min_ev);
    let x2 = x * x;
    let x4 = x2 * x2;
    x = 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
    if punchy {
        let luma = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
        x = pow(max(x, vec3<f32>(0.0)), vec3<f32>(1.35));
        x = luma + 1.4 * (x - luma);
    }
    x = max(agx_inv * x, vec3<f32>(0.0));
    return pow(x, vec3<f32>(2.2));
}

// SE's `ToneMapping` of each hdrt_*_tone.glh, by `TONE_SHADERS` index.
fn tone_map(c: vec3<f32>, op: u32) -> vec3<f32> {
    switch op {
        case 0u: { return tone_aces(c); }
        case 2u: { return tone_agx(c * 1.67, true); }
        case 3u: {
            // Bruneton
            let c0 = pow(c * 0.38317, vec3<f32>(1.0 / 2.2));
            let c1 = 1.0 - exp(-c);
            return pow(mix(c0, c1, step(vec3<f32>(1.413), c)), vec3<f32>(1.5));
        }
        case 4u: { return 1.0 - exp(-c * 0.85); }
        case 5u: {
            // Filmic (Hejl-Burgess-Dawson)
            let x = max(vec3<f32>(0.0), c * 0.2 - 0.004);
            return (x * (x * 6.2 + 0.5)) / (x * (x * 6.2 + 1.7) + 0.06);
        }
        case 6u: {
            // Filmic_Hejl (white point 1)
            let vh = vec4<f32>(c * 0.15, 1.0);
            let va = 1.425 * vh + 0.05;
            let vf = (vh * va + 0.004) / (vh * (va + 0.55) + 0.0491) - 0.0821;
            return srgb_encode3(vf.rgb / vf.w);
        }
        case 7u: {
            let x = c * 0.25;
            return vec3<f32>(rec709_encode(x.r), rec709_encode(x.g), rec709_encode(x.b));
        }
        case 8u: { return c / (1.0 + c); }
        case 9u: { return 1.163 * (c / (1.0 + c)); }
        case 10u: { return srgb_encode3(c * 0.25); }
        default: { return tone_agx(c, false); }
    }
}

// Bevy's screen-space dither (static per pixel, so a still view gives identical frames), one 8-bit step.
fn dither(pos: vec2<f32>) -> vec3<f32> {
    var d = vec3<f32>(dot(vec2<f32>(171.0, 231.0), pos));
    d = fract(d / vec3<f32>(103.0, 71.0, 97.0));
    return (d - 0.5) / 255.0;
}

// hdr_tone.glsl: the tone-mapped frame, into the scratch texture.
@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(scene, scene_sampler, in.uv);
    var hdr = clamp(src.rgb, vec3<f32>(0.0), vec3<f32>(65000.0));
    if cc.mode.z != 0u {
        hdr = hdr * exp2(-auto_state.comp.x);
    }
    return vec4<f32>(tone_map(hdr, cc.mode.x), src.a);
}

// A texel of the tone-mapped frame, clamped to the edge (SE's textureOffset).
fn tap(p: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    return textureLoad(scene, clamp(p, vec2<i32>(0), size - 1), 0);
}

// hdr_final.glsl `textureFastCAS` (AMD FidelityFX CAS, SE's `SHARP` variant): a cross of five texels,
// the sharpening shaped by the green channel's distance to 0 and 1, `sharpness` 0..1 (`LimParams.w`).
fn fast_cas(p: vec2<i32>, size: vec2<i32>, sharpness: f32) -> vec4<f32> {
    let col = tap(p, size);
    let n = tap(p + vec2<i32>(0, -1), size);
    let e = tap(p + vec2<i32>(1, 0), size);
    let s = tap(p + vec2<i32>(0, 1), size);
    let w = tap(p + vec2<i32>(-1, 0), size);
    var max_g = max(max(max(col.y, n.y), max(e.y, s.y)), w.y);
    let min_g = min(min(min(col.y, n.y), min(e.y, s.y)), w.y);
    let colw = n + e + s + w;
    let d_min_g = min_g;
    let d_max_g = 1.0 - max_g;
    max_g = max(0.0, max_g);
    var a = select(d_min_g / max_g, d_max_g / max_g, d_max_g < d_min_g);
    a = sqrt(max(0.0, a)) * mix(-0.125, -0.2, sharpness);
    return (col + colw * a) / (1.0 + 4.0 * a);
}

// hdr_final.glsl: the sharpening (when `Sharpness` > 0) and the colour correction of the tone-mapped
// scratch texture, back into the main texture.
@fragment
fn final_image(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(scene));
    let p = vec2<i32>(in.position.xy);
    var src: vec4<f32>;
    if cc.sharp.x > 0.0 {
        src = fast_cas(p, size, cc.sharp.x);
    } else {
        src = tap(p, size);
    }
    var c = src.rgb;
    // Contrast and brightness.
    c = mix(vec3<f32>(0.5), c, cc.bcsg.y) + cc.bcsg.xxx;
    // Vibrance: extrapolate from luma by 1 + (S - 1) (1 - the pixel's saturation).
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sat = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
    c = mix(vec3<f32>(luma), c, 1.0 + cc.bcsg.z * (1.0 - sat));
    // Gamma.
    c = max(pow(max(c, vec3<f32>(1e-6)), cc.bcsg.www), vec3<f32>(0.0));
    if cc.mode.y != 0u {
        // SE dithers the 8-bit output; the swapchain encodes sRGB, so dither in that space.
        c = pow(max(pow(c, vec3<f32>(1.0 / 2.2)) + dither(in.position.xy), vec3<f32>(0.0)), vec3<f32>(2.2));
    }
    return vec4<f32>(c, src.a);
}
