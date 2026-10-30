// SE's reduced-resolution solids (`render_scale.rs`): the main pass draws into the top-left
// `SolidResolution` part of the main texture; `fxaa` (fxaa.glsl, `SolidFXAA`) or a copy moves it into the
// low-resolution scratch texture, and `upscale` (upscale_bicubic.glsl with bicubic_common.glh's
// `textureUpscaled`, `SolidUpscaleMode`) draws it over the whole main texture.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct SolidUpscale {
    // x `SolidResolution`, y `SolidTargetRes`, z sharpness (`SolidSharpness`), w on
    params: vec4<f32>,
    // x METHOD (0 nearest, 1 bilinear, 2 bicubic, 3 lanczos), y SHARP (0 none, 1 simple, 2 CAS), z FXAA
    mode: vec4<u32>,
    // xy the solids' size, zw the full size (pixels)
    size: vec4<f32>,
};

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var<uniform> up: SolidUpscale;

// ---------------------------------------------------------------------------------------------------
// bicubic_common.glh, with LIMIT and SPLINE 0 (upscale_bicubic.glsl's settings).

fn spline_cubic(v: f32) -> vec4<f32> {
    let n = vec4<f32>(1.0, 2.0, 3.0, 4.0) - v;
    let s = n * n * n;
    var w: vec4<f32>;
    w.x = s.x;
    w.y = s.y - 4.0 * s.x;
    w.z = s.z - 4.0 * s.y + 6.0 * s.x;
    w.w = 6.0 - w.x - w.y - w.z;
    return w;
}

// Reinhard to LDR and back around the CAS (`br` 0.05).
const BR: f32 = 0.05;
fn to_ldr(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(BR * c.rgb / (1.0 + BR * c.rgb), c.a);
}
fn to_hdr(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb / (BR * (1.0 - c.rgb)), c.a);
}

fn texture_lim(uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(tex, tex_sampler, uv, 0.0);
    return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(65000.0)), c.a);
}

fn texture_nearest(uv: vec2<f32>, res: vec4<f32>, off: vec2<f32>) -> vec4<f32> {
    let p = vec2<i32>(uv * res.xy + off);
    return textureLoad(tex, clamp(p, vec2<i32>(0), vec2<i32>(res.xy) - 1), 0);
}

fn texture_bicubic(uv_in: vec2<f32>, res: vec4<f32>, off: vec2<f32>) -> vec4<f32> {
    var uv = uv_in * res.xy + off - vec2<f32>(0.5);
    let uvf = fract(uv);
    uv = uv - uvf;
    let xcubic = spline_cubic(uvf.x);
    let ycubic = spline_cubic(uvf.y);
    let c = uv.xxyy + vec4<f32>(-0.5, 1.5, -0.5, 1.5);
    let s = vec4<f32>(xcubic.xz + xcubic.yw, ycubic.xz + ycubic.yw);
    let offset = (c + vec4<f32>(xcubic.yw, ycubic.yw) / s) * res.zzww;
    let sx = s.x / (s.x + s.y);
    let sy = s.z / (s.z + s.w);
    let s0 = texture_lim(offset.xz);
    let s1 = texture_lim(offset.yz);
    let s2 = texture_lim(offset.xw);
    let s3 = texture_lim(offset.yw);
    return mix(mix(s3, s2, sx), mix(s1, s0, sx), sy);
}

fn texture_lanczos(uv: vec2<f32>, res: vec4<f32>, off: vec2<f32>) -> vec4<f32> {
    let src_pos = uv * res.xy + off;
    let src_centre = floor(src_pos - 0.5) + 0.5;
    let fxy = src_pos - src_centre;
    let f = vec4<f32>(fxy, 1.0 - fxy);
    // Piecewise approximation of the Lanczos kernel.
    let l2_w0_o3 = ((1.5672 * f - 2.6445) * f + 0.0837) * f + 0.9976;
    let l2_w1_o3 = ((-0.7389 * f + 1.3652) * f - 0.6295) * f - 0.0004;
    let w1_2 = l2_w0_o3;
    let w12 = w1_2.xy + w1_2.zw;
    var wedge = l2_w1_o3 * w12.yxyx;
    let tc12 = res.zw * (src_centre + w1_2.zw / w12);
    let tc0 = res.zw * (src_centre - 1.0);
    let tc3 = res.zw * (src_centre + 2.0);
    let sum = wedge.x + wedge.y + wedge.z + wedge.w + w12.x * w12.y;
    wedge = wedge / sum;
    let col = texture_lim(vec2<f32>(tc12.x, tc0.y)) * wedge.y
        + texture_lim(vec2<f32>(tc0.x, tc12.y)) * wedge.x
        + texture_lim(tc12) * (w12.x * w12.y)
        + texture_lim(vec2<f32>(tc3.x, tc12.y)) * wedge.z
        + texture_lim(vec2<f32>(tc12.x, tc3.y)) * wedge.w;
    return max(col, vec4<f32>(0.0));
}

fn sample_norm(uv: vec2<f32>, res: vec4<f32>, off: vec2<f32>) -> vec4<f32> {
    switch up.mode.x {
        case 0u: { return texture_nearest(uv, res, off); }
        case 1u: { return textureSampleLevel(tex, tex_sampler, uv + off * res.zw, 0.0); }
        case 2u: { return texture_bicubic(uv, res, off); }
        default: { return texture_lanczos(uv, res, off); }
    }
}

// Classic sharpness filter (`sampleSharp`).
fn sample_sharp(uv: vec2<f32>, res: vec4<f32>, sharpness: f32) -> vec4<f32> {
    let c = sample_norm(uv, res, vec2<f32>(0.0, 0.0));
    let sides = sample_norm(uv, res, vec2<f32>(1.0, 1.0)) + sample_norm(uv, res, vec2<f32>(-1.0, -1.0))
        + sample_norm(uv, res, vec2<f32>(1.0, -1.0)) + sample_norm(uv, res, vec2<f32>(-1.0, 1.0));
    return max(c + (c - 0.25 * sides) * sharpness, vec4<f32>(0.0));
}

// AMD FidelityFX CAS, the fast variant (`sampleFastCAS`), in Reinhard LDR.
fn sample_fast_cas(uv: vec2<f32>, res: vec4<f32>, sharpness: f32) -> vec4<f32> {
    var col = to_ldr(sample_norm(uv, res, vec2<f32>(0.0, 0.0)));
    let n = to_ldr(sample_norm(uv, res, vec2<f32>(0.0, -1.0)));
    let e = to_ldr(sample_norm(uv, res, vec2<f32>(1.0, 0.0)));
    let s = to_ldr(sample_norm(uv, res, vec2<f32>(0.0, 1.0)));
    let w = to_ldr(sample_norm(uv, res, vec2<f32>(-1.0, 0.0)));
    var max_g = max(max(max(col.y, n.y), max(e.y, s.y)), w.y);
    let min_g = min(min(min(col.y, n.y), min(e.y, s.y)), w.y);
    let colw = n + e + s + w;
    let d_min_g = min_g;
    let d_max_g = 1.0 - max_g;
    max_g = max(0.0, max_g);
    var a = select(d_min_g / max_g, d_max_g / max_g, d_max_g < d_min_g);
    a = sqrt(max(0.0, a)) * mix(-0.125, -0.2, sharpness);
    col = max((col + colw * a) / (1.0 + 4.0 * a), vec4<f32>(0.0));
    return to_hdr(col);
}

// upscale_bicubic.glsl: `textureUpscaled`, clamped to the colour limit.
@fragment
fn upscale(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(tex));
    let res = vec4<f32>(size, 1.0 / size);
    var c: vec4<f32>;
    switch up.mode.y {
        case 0u: { c = sample_norm(in.uv, res, vec2<f32>(0.0)); }
        case 1u: { c = sample_sharp(in.uv, res, up.params.z); }
        default: { c = sample_fast_cas(in.uv, res, up.params.z); }
    }
    return clamp(c, vec4<f32>(0.0), vec4<f32>(65000.0));
}

// ---------------------------------------------------------------------------------------------------
// fxaa.glsl (FXAA 3.11 quality, FXAA_QUALITY__PRESET 12, HDR through Reinhard with exposure 1) on the
// low-resolution corner of the main texture; positions are in pixels.

const SUBPIX: f32 = 0.75;
const EDGE_THRESHOLD: f32 = 0.166;
const EDGE_THRESHOLD_MIN: f32 = 0.0833;

fn max3(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b));
}

// The low-resolution frame's size (its corner of the main texture).
fn frame() -> vec2<f32> {
    return up.size.xy;
}

fn fx_sample(p: vec2<f32>) -> vec4<f32> {
    let q = clamp(p, vec2<f32>(0.5), frame() - 0.5);
    let c = textureSampleLevel(tex, tex_sampler, q / vec2<f32>(textureDimensions(tex)), 0.0);
    return vec4<f32>(c.rgb / (max3(c.rgb) + 1.0), c.a);
}

fn fx_lum(p: vec2<f32>) -> f32 {
    return dot(fx_sample(p).rgb, vec3<f32>(0.299, 0.587, 0.114));
}

@fragment
fn fxaa(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var pos_m = in.position.xy;
    let rgby_m = fx_sample(pos_m);
    let luma_m = dot(rgby_m.rgb, vec3<f32>(0.299, 0.587, 0.114));
    var luma_s = fx_lum(pos_m + vec2<f32>(0.0, 1.0));
    let luma_e = fx_lum(pos_m + vec2<f32>(1.0, 0.0));
    var luma_n = fx_lum(pos_m + vec2<f32>(0.0, -1.0));
    let luma_w = fx_lum(pos_m + vec2<f32>(-1.0, 0.0));
    let range_max = max(max(luma_n, luma_w), max(luma_e, max(luma_s, luma_m)));
    let range_min = min(min(luma_n, luma_w), min(luma_e, min(luma_s, luma_m)));
    let range = range_max - range_min;
    var out = rgby_m;
    if range >= max(EDGE_THRESHOLD_MIN, range_max * EDGE_THRESHOLD) {
        let luma_nw = fx_lum(pos_m + vec2<f32>(-1.0, -1.0));
        let luma_se = fx_lum(pos_m + vec2<f32>(1.0, 1.0));
        let luma_ne = fx_lum(pos_m + vec2<f32>(1.0, -1.0));
        let luma_sw = fx_lum(pos_m + vec2<f32>(-1.0, 1.0));
        let luma_ns = luma_n + luma_s;
        let luma_we = luma_w + luma_e;
        let subpix_rcp_range = 1.0 / range;
        let subpix_nswe = luma_ns + luma_we;
        let edge_horz1 = -2.0 * luma_m + luma_ns;
        let edge_vert1 = -2.0 * luma_m + luma_we;
        let luma_nese = luma_ne + luma_se;
        let luma_nwne = luma_nw + luma_ne;
        let edge_horz2 = -2.0 * luma_e + luma_nese;
        let edge_vert2 = -2.0 * luma_n + luma_nwne;
        let luma_nwsw = luma_nw + luma_sw;
        let luma_swse = luma_sw + luma_se;
        let edge_horz4 = abs(edge_horz1) * 2.0 + abs(edge_horz2);
        let edge_vert4 = abs(edge_vert1) * 2.0 + abs(edge_vert2);
        let edge_horz3 = -2.0 * luma_w + luma_nwsw;
        let edge_vert3 = -2.0 * luma_s + luma_swse;
        let edge_horz = abs(edge_horz3) + edge_horz4;
        let edge_vert = abs(edge_vert3) + edge_vert4;
        let subpix_nwswnese = luma_nwsw + luma_nese;
        var length_sign = 1.0;
        let horz_span = edge_horz >= edge_vert;
        let subpix_a = subpix_nswe * 2.0 + subpix_nwswnese;
        if !horz_span {
            luma_n = luma_w;
            luma_s = luma_e;
        }
        let subpix_b = subpix_a * (1.0 / 12.0) - luma_m;
        let gradient_n = luma_n - luma_m;
        let gradient_s = luma_s - luma_m;
        var luma_nn = luma_n + luma_m;
        let luma_ss = luma_s + luma_m;
        let pair_n = abs(gradient_n) >= abs(gradient_s);
        let gradient = max(abs(gradient_n), abs(gradient_s));
        if pair_n {
            length_sign = -length_sign;
        }
        let subpix_c = clamp(abs(subpix_b) * subpix_rcp_range, 0.0, 1.0);
        var pos_b = pos_m;
        let off_np = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), horz_span);
        if !horz_span {
            pos_b.x += length_sign * 0.5;
        } else {
            pos_b.y += length_sign * 0.5;
        }
        // FXAA_QUALITY__PS 5: P0..P4 = 1, 1.5, 2, 4, 12.
        var pos_n = pos_b - off_np;
        var pos_p = pos_b + off_np;
        let subpix_d = -2.0 * subpix_c + 3.0;
        let subpix_e = subpix_c * subpix_c;
        if !pair_n {
            luma_nn = luma_ss;
        }
        let gradient_scaled = gradient / 4.0;
        let luma_mm = luma_m - luma_nn * 0.5;
        let subpix_f = subpix_d * subpix_e;
        let luma_mlt_zero = luma_mm < 0.0;
        var luma_end_n = fx_lum(pos_n) - luma_nn * 0.5;
        var luma_end_p = fx_lum(pos_p) - luma_nn * 0.5;
        var done_n = abs(luma_end_n) >= gradient_scaled;
        var done_p = abs(luma_end_p) >= gradient_scaled;
        let steps = array<f32, 4>(1.5, 2.0, 4.0, 12.0);
        for (var i = 0u; i < 4u; i++) {
            if !done_n {
                pos_n -= off_np * steps[i];
            }
            if !done_p {
                pos_p += off_np * steps[i];
            }
            if (done_n && done_p) || i == 3u {
                break;
            }
            if !done_n {
                luma_end_n = fx_lum(pos_n) - luma_nn * 0.5;
            }
            if !done_p {
                luma_end_p = fx_lum(pos_p) - luma_nn * 0.5;
            }
            done_n = abs(luma_end_n) >= gradient_scaled;
            done_p = abs(luma_end_p) >= gradient_scaled;
        }
        var dst_n = pos_m.x - pos_n.x;
        var dst_p = pos_p.x - pos_m.x;
        if !horz_span {
            dst_n = pos_m.y - pos_n.y;
            dst_p = pos_p.y - pos_m.y;
        }
        let good_span_n = (luma_end_n < 0.0) != luma_mlt_zero;
        let good_span_p = (luma_end_p < 0.0) != luma_mlt_zero;
        let span_length_rcp = 1.0 / (dst_p + dst_n);
        let direction_n = dst_n < dst_p;
        let dst = min(dst_n, dst_p);
        let good_span = select(good_span_p, good_span_n, direction_n);
        let subpix_g = subpix_f * subpix_f;
        let pixel_offset = dst * -span_length_rcp + 0.5;
        let subpix_h = subpix_g * SUBPIX;
        let pixel_offset_subpix = max(select(0.0, pixel_offset, good_span), subpix_h);
        if !horz_span {
            pos_m.x += pixel_offset_subpix * length_sign;
        } else {
            pos_m.y += pixel_offset_subpix * length_sign;
        }
        out = fx_sample(pos_m);
    }
    // Reverse Reinhard.
    return vec4<f32>(out.rgb / (1.0 - max3(out.rgb)), out.a);
}
