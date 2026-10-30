// The ship environment map's compute passes (`ship/envmap.rs`): the six camera faces copied into
// the cube's first level, its mip chain by 2x2 boxes (SE's MipmapsEnv), then ship_pbr.glsl's
// EnvMapSpec by pbr_spec_cubemap.glsl (one GGX roughness per level, 32 importance samples whose
// source level follows their solid angle) and EnvMapDiff by pbr_diff_cubemap.glsl (96 cosine
// samples). Cube layers follow the usual convention, in world axes. Each pass has its own binding
// numbers.

// Each face camera's forward and up (`FACES` in envmap.rs), indexed like the cube's layers.
const FWD: array<vec3<f32>, 6> = array<vec3<f32>, 6>(
    vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0),
    vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0));
const UP: array<vec3<f32>, 6> = array<vec3<f32>, 6>(
    vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, -1.0),
    vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 1.0, 0.0));

const PI: f32 = 3.14159265359;

// pbr_*_cubemap.glsl's GetViewVector: the direction of a layer's texel, txc in [-1, 1].
fn view_vector(txc: vec2<f32>, face: u32) -> vec3<f32> {
    var v = vec3<f32>(1.0, 0.0, 0.0);
    switch face {
        case 0u: { v = vec3<f32>(1.0, -txc.y, -txc.x); }
        case 1u: { v = vec3<f32>(-1.0, -txc.y, txc.x); }
        case 2u: { v = vec3<f32>(txc.x, 1.0, txc.y); }
        case 3u: { v = vec3<f32>(txc.x, -1.0, -txc.y); }
        case 4u: { v = vec3<f32>(txc.x, -txc.y, 1.0); }
        default: { v = vec3<f32>(-txc.x, -txc.y, -1.0); }
    }
    return normalize(v);
}

fn texel_dir(id: vec3<u32>, size: u32) -> vec3<f32> {
    let txc = (vec2<f32>(id.xy) + 0.5) / f32(size) * 2.0 - 1.0;
    return view_vector(txc, id.z);
}

// ---------------------------------------------------------------------------------------------
// The faces into the cube's first level.

@group(0) @binding(0) var face0: texture_2d<f32>;
@group(0) @binding(1) var face1: texture_2d<f32>;
@group(0) @binding(2) var face2: texture_2d<f32>;
@group(0) @binding(3) var face3: texture_2d<f32>;
@group(0) @binding(4) var face4: texture_2d<f32>;
@group(0) @binding(5) var face5: texture_2d<f32>;
@group(0) @binding(6) var copy_dst: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn copy_faces(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(copy_dst).x;
    if id.x >= size || id.y >= size {
        return;
    }
    // Where the texel's direction lands in its face camera's image (right = forward x up).
    let d = texel_dir(id, size);
    let fwd = FWD[id.z];
    let up = UP[id.z];
    let right = cross(fwd, up);
    let k = 1.0 / dot(d, fwd);
    let uv = vec2<f32>(dot(d, right) * k + 1.0, 1.0 - dot(d, up) * k) * 0.5;
    let p = vec2<i32>(clamp(uv * f32(size), vec2<f32>(0.0), vec2<f32>(f32(size) - 1.0)));
    var c = vec4<f32>(0.0);
    switch id.z {
        case 0u: { c = textureLoad(face0, p, 0); }
        case 1u: { c = textureLoad(face1, p, 0); }
        case 2u: { c = textureLoad(face2, p, 0); }
        case 3u: { c = textureLoad(face3, p, 0); }
        case 4u: { c = textureLoad(face4, p, 0); }
        default: { c = textureLoad(face5, p, 0); }
    }
    textureStore(copy_dst, vec2<i32>(id.xy), i32(id.z), vec4<f32>(c.rgb, 1.0));
}

// ---------------------------------------------------------------------------------------------
// One mip level from the one above.

@group(0) @binding(7) var down_src: texture_2d_array<f32>;
@group(0) @binding(8) var down_dst: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(down_dst).x;
    if id.x >= size || id.y >= size {
        return;
    }
    let p = vec2<i32>(id.xy) * 2;
    let l = i32(id.z);
    let c = textureLoad(down_src, p, l, 0) + textureLoad(down_src, p + vec2<i32>(1, 0), l, 0)
        + textureLoad(down_src, p + vec2<i32>(0, 1), l, 0) + textureLoad(down_src, p + vec2<i32>(1, 1), l, 0);
    textureStore(down_dst, vec2<i32>(id.xy), l, c * 0.25);
}

// ---------------------------------------------------------------------------------------------
// EnvMapSpec and EnvMapDiff. Params (PbrCubemapUniforms, FUN_1402ff440): the source's resolution,
// 6 res^2 / 4 pi (1 / a texel's solid angle), roughness^4, 1 / the source's exposure.

@group(0) @binding(9) var cube: texture_cube<f32>;
@group(0) @binding(10) var cube_smp: sampler;
@group(0) @binding(11) var filter_dst: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(12) var<uniform> params: vec4<f32>;

fn radical_inverse(b: u32) -> f32 {
    var bits = (b << 16u) | (b >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse(i));
}

fn tangent_frame(n: vec3<f32>) -> mat3x3<f32> {
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.999);
    let t = normalize(cross(up, n));
    return mat3x3<f32>(t, cross(n, t), n);
}

@compute @workgroup_size(8, 8, 1)
fn prefilter_spec(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(filter_dst).x;
    if id.x >= size || id.y >= size {
        return;
    }
    let n = texel_dir(id, size);
    let v = n;
    let r4 = params.z;
    let frame = tangent_frame(n);
    var color = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0u; i < 32u; i++) {
        let xi = hammersley(i, 32u);
        let phi = 2.0 * PI * xi.x;
        let cos_t = sqrt((1.0 - xi.y) / (1.0 + (r4 - 1.0) * xi.y));
        let sin_t = sqrt(1.0 - cos_t * cos_t);
        let h = normalize(frame * vec3<f32>(cos(phi) * sin_t, sin(phi) * sin_t, cos_t));
        let l = normalize(2.0 * dot(v, h) * h - v);
        let ndl = max(dot(n, l), 0.0);
        if ndl > 0.0 {
            let ndh = max(dot(n, h), 0.0);
            let den = ndh * ndh * (r4 - 1.0) + 1.0;
            let d = r4 / (PI * den * den);
            let hdv = max(dot(h, v), 0.0);
            let pdf = d * ndh / (4.0 * hdv) + 0.0001;
            let sa_sample = 1.0 / (32.0 * pdf + 0.0001);
            let mip = max(0.5 * log2(sa_sample * params.y) + 1.0, 0.0);
            color += textureSampleLevel(cube, cube_smp, l, mip).rgb * params.w * ndl;
            weight += ndl;
        }
    }
    textureStore(filter_dst, vec2<i32>(id.xy), i32(id.z), vec4<f32>(color / (params.w * weight), 1.0));
}

// SE passes the same Params to this pass, whose shader reads Params.y as the source level: 6 res^2 /
// 4 pi, past the last level, so the irradiance integrates the faces' averages.
@compute @workgroup_size(8, 8, 1)
fn irradiance(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(filter_dst).x;
    if id.x >= size || id.y >= size {
        return;
    }
    let n = texel_dir(id, size);
    let frame = tangent_frame(n);
    var color = vec3<f32>(0.0);
    var weight = 0.0;
    for (var s = 0u; s < 96u; s++) {
        let xi = hammersley(s, 96u);
        let phi = 2.0 * PI * xi.x;
        let cos_t = sqrt(xi.y);
        let sin_t = sqrt(1.0 - cos_t * cos_t);
        let l = frame * vec3<f32>(sin_t * cos(phi), sin_t * sin(phi), cos_t);
        let ndl = clamp(dot(l, n), 0.0, 1.0);
        color += textureSampleLevel(cube, cube_smp, l, params.y).rgb * params.w * ndl;
        weight += ndl;
    }
    textureStore(filter_dst, vec2<i32>(id.xy), i32(id.z), vec4<f32>(color / (params.w * weight), 1.0));
}
