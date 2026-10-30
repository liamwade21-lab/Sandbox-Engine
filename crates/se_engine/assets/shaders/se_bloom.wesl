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
    // Pseudo lens flare (hdr_combine.glsl): x GhostsBright, y HaloBright, z DirtBright, w on.
    flare_bright: vec4<f32>,
    // x GhostsBlur, y HaloBlur, z DirtDispersal (bloom levels), w viewport scale.
    flare_blur: vec4<f32>,
    // x GhostsDispersal, y GhostsCount, z HaloRadius, w Chromaticism.
    flare_shape: vec4<f32>,
    // x width / height, y 1 with a dirt texture, z 1 with a rays texture, w the ghosts' and halo's
    // gain to SE's brightness scale (lensflare.rs STAR_FRAME_GAIN).
    lens: vec4<f32>,
    // LensRaysMatr by columns.
    rays: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> bloom: SeBloom;
// combine_flares only.
@group(0) @binding(3) var dirt_tex: texture_2d<f32>;
@group(0) @binding(4) var rays_tex: texture_2d<f32>;

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

// hdr_combine.glsl BLOOM: BloomBright x the normalised weighted sum of the levels.
fn bloom_sum(uv: vec2<f32>) -> vec3<f32> {
    let w = array<f32, 6>(0.50, 0.60, 0.60, 0.45, 0.35, 0.23);
    var c = vec3<f32>(0.0);
    for (var k = 0; k < 6; k = k + 1) {
        c += textureSampleLevel(src, src_sampler, uv, f32(k)).rgb * w[k];
    }
    let bn = 1.0 / (0.5 + 0.6 + 0.6 + 0.45 + 0.35 + 0.23);
    return c * bn * bloom.params.x;
}

// Added to the frame (the pipeline blends One + One).
@fragment
fn combine(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(bloom_sum(in.uv), 0.0);
}

// hdr_combine.glsl textureCroma: the chain at `lod` with the channels pulled apart along `dir`. The
// port's suns are dimmer than SE's, so the samples are raised by the flare gain and limited to
// BloomDynamicRange, as SE's own bloom is where it is made.
fn croma(uv: vec2<f32>, lod: f32, dir: vec2<f32>, dist: vec3<f32>) -> vec3<f32> {
    let c = vec3<f32>(
        textureSampleLevel(src, src_sampler, uv + dir * dist.r, lod).r,
        textureSampleLevel(src, src_sampler, uv + dir * dist.g, lod).g,
        textureSampleLevel(src, src_sampler, uv + dir * dist.b, lod).b,
    ) * bloom.lens.w;
    return c * clamp(bloom.params.w / (dot(c, TO_GRAY) + 1.0e-10), 0.0, 1.0);
}

fn safe_normalize(v: vec2<f32>) -> vec2<f32> {
    let l = length(v);
    return select(vec2<f32>(0.0), v / l, l > 1.0e-12);
}

// hdr_combine.glsl BLOOM + FLARES (John Chapman's pseudo lens flare): ghosts of the bright parts of
// the bloom mirrored through the screen centre and a halo, both multiplied by the starburst
// (`LensRays`, x3) plus the lens dirt; the frame is also mixed towards a blurred bloom level by the
// dirt times DirtBright. Returns the added light in rgb and the dirt mix in alpha (the pipeline
// blends One + OneMinusSrcAlpha, so a grey dirt level stands for SE's per-channel mix).
@fragment
fn combine_flares(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let fb = bloom.flare_bright;
    let blur = bloom.flare_blur;
    let fs = bloom.flare_shape;
    let aspect = vec2<f32>(bloom.lens.x, 1.0);
    // The frame's texel (the chain's first level is half its size).
    let texel = 0.5 / vec2<f32>(textureDimensions(src));
    let dist = vec3<f32>(-texel.x * fs.w, 0.0, texel.y * fs.w);

    // Ghosts.
    let tc = vec2<f32>(1.0) - uv;
    let ghost_vec = (uv - vec2<f32>(0.5)) * fs.x;
    let ghost_dir = safe_normalize(ghost_vec);
    var ghost = vec3<f32>(0.0);
    let n = i32(fs.y);
    for (var i = 0; i < n; i = i + 1) {
        let fi = f32(i);
        let size = fi * fs.x;
        let refl = select(1.0, -1.0, size > 1.0);
        let fade = abs(1.0 - size);
        let offset = fract(tc + ghost_vec * fi);
        let color = croma(offset, blur.x * blur.w, refl * ghost_dir, dist);
        let luma = dot(color, TO_GRAY);
        let w0 = clamp(length(vec2<f32>(0.5) - offset) * 1.41421356, 0.0, 1.0);
        let w = pow(1.0 - w0, 10.0) * smoothstep(0.0, 5.5, luma) * fade * fade;
        ghost += color * w;
    }
    ghost *= fb.x;

    // Halo.
    var halo_vec = (uv - vec2<f32>(0.5)) * aspect;
    halo_vec = halo_vec - safe_normalize(halo_vec) * fs.z;
    let hw = pow(1.0 - clamp(length(halo_vec), 0.0, 1.0), 5.0) * fb.y;
    halo_vec = vec2<f32>(0.5) - halo_vec;
    let halo = croma(halo_vec, blur.y * blur.w, -2.0 * safe_normalize(halo_vec), dist) * hw * 0.33;

    // Lens dirt and the starburst.
    let dirt = textureSampleLevel(dirt_tex, src_sampler, uv, 0.0).rgb * bloom.lens.y;
    let rays_m = mat2x2<f32>(bloom.rays.xy, bloom.rays.zw);
    let tc_rays = rays_m * (uv - vec2<f32>(0.5)) * 0.5 + vec2<f32>(0.5);
    let rays = textureSampleLevel(rays_tex, src_sampler, tc_rays, 0.0).rgb * 3.0 * bloom.lens.z;
    let flare = (ghost + halo) * (rays + dirt);

    let dirt_mix = dirt * fb.z;
    let dirt_mask = textureSampleLevel(src, src_sampler, uv, blur.z * blur.w).rgb;
    return vec4<f32>(bloom_sum(uv) + flare + dirt_mask * dirt_mix, dot(dirt_mix, TO_GRAY));
}

// The blurred frame behind the interface's windows (gui.glsl BLUR_BACKGROUND, `BackTex`), tone-mapped
// as SE does (1 - exp(-exposure x c)); skin.rs adds BackgroundBrightness of it to every window.
@fragment
fn backdrop(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src, src_sampler, in.uv, 0.0).rgb;
    return vec4<f32>(1.0 - exp(-c), 1.0);
}
