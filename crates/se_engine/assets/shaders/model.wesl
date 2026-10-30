// Catalog model meshes (asteroids, comets) as SE's ship.glsl draws them: the diffuse map times
// DiffColorScale, two scales of detail diffuse, the tangent-space normal map blended with two scales
// of detail normals, Lambert mixed with the simplified Hapke BRDF (opposition surge), a Blinn-Phong
// highlight from SpecColor / SpecPower, and the ambient level. Colour maps are read as stored (SE's
// DiffGamma 1), linear.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Model {
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    // rgb DiffColorScale, w ambient
    base_color: vec4<f32>,
    // The further stars: xyz direction, w 1 when used; rgb their light, intensity included.
    light_dir: array<vec4<f32>, 3>,
    light_color: array<vec4<f32>, 3>,
    // rgb SpecColor, w SpecPower (0 none)
    spec: vec4<f32>,
    // SE's DetailScale: detail diffuse tiling and mix, detail bump tiling and mix
    detail: vec4<f32>,
    // SE's HapkeParams
    hapke: vec4<f32>,
    // x Hapke-to-Lambert mix, y 1 with a normal map
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> model: Model;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var model_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var model_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var norm_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var norm_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var det_diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var det_diff_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var det_norm_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var det_norm_smp: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.uv = v.uv;
    o.world_pos = world.xyz;
    return o;
}

// A tangent-space normal from a map's xy (z rebuilt, as SE's FromBC5).
fn map_normal(t: vec4<f32>) -> vec3<f32> {
    let xy = t.xy * 2.0 - 1.0;
    return vec3<f32>(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

// SE's BlendNormals, NORMALS_BLEND_METHOD 3.
fn blend_normals(n1: vec3<f32>, n2: vec3<f32>, t: f32) -> vec3<f32> {
    return normalize(vec3<f32>(mix(n1.xy, n2.xy, t), n1.z));
}

// SE's Hapke hot spot term at the phase cosine `mu`.
fn hapke(mu: f32) -> f32 {
    let t = sqrt((1.0 - mu) / (1.0 + mu));
    return 1.0 + model.hapke.x / (1.0 + t * model.hapke.y) + model.hapke.z / (1.0 + t * model.hapke.w);
}

// The star's light reaching the surface of normal `n` from `l` seen along `eye` (from the camera),
// Lambert mixed with the simplified Hapke BRDF, plus its highlight weight (rgb, w).
fn lit(n: vec3<f32>, l: vec3<f32>, eye: vec3<f32>, cos_o: f32) -> vec2<f32> {
    let cos_i = clamp(dot(n, l), 0.0, 1.0);
    let mu = clamp(-dot(l, eye), -0.999, 1.0);
    let lit_factor = mix(cos_i, cos_i / (cos_i + cos_o) * hapke(mu), model.params.x);
    var spec = 0.0;
    if model.spec.w > 0.0 {
        let h = normalize(l - eye);
        let p = max(model.spec.w, 1.0);
        spec = (p * 0.039788735 + 0.31830989) * pow(clamp(dot(h, n), 0.0, 1.0), p);
    }
    return vec2<f32>(lit_factor, spec * lit_factor);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let n0 = normalize(in.world_normal);
    // The normal map and the two scales of detail normals, blended as SE does.
    var nt = map_normal(textureSample(norm_tex, norm_smp, in.uv));
    if model.detail.w > 0.0 {
        let d0 = map_normal(textureSample(det_norm_tex, det_norm_smp, in.uv * model.detail.z));
        let d1 = map_normal(textureSample(det_norm_tex, det_norm_smp, in.uv * model.detail.z * 4.0));
        nt = blend_normals(nt, blend_normals(d0, d1, 0.5), model.detail.w);
    } else {
        nt = normalize(nt);
    }
    // SE's tangent frame (its meshes' stored tangents): the tangent along +u, the binormal
    // w cross(tangent, normal) along -v. Rebuilt per pixel from the UV gradients rather than stored
    // (16 more bytes a vertex on 67P's 12 million).
    let dp1 = dpdx(in.world_pos);
    let dp2 = dpdy(in.world_pos);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let p2 = cross(dp2, n0);
    let p1 = cross(n0, dp1);
    let du = p2 * duv1.x + p1 * duv2.x;
    let dv = p2 * duv1.y + p1 * duv2.y;
    let t = normalize(du - n0 * dot(du, n0) + vec3<f32>(1e-20));
    let b = -normalize(dv - n0 * dot(dv, n0) + vec3<f32>(1e-20));
    let n = normalize(t * nt.x + b * nt.y + n0 * nt.z);

    var albedo = textureSample(model_tex, model_smp, in.uv).rgb * model.base_color.rgb;
    if model.detail.y > 0.0 {
        let c0 = mix(vec3<f32>(0.5), textureSample(det_diff_tex, det_diff_smp, in.uv * model.detail.x).rgb, model.detail.y);
        let c1 = mix(vec3<f32>(0.5), textureSample(det_diff_tex, det_diff_smp, in.uv * model.detail.x * 4.0).rgb, model.detail.y);
        albedo *= c0 * c1 * (4.0 + 6.0 * model.detail.y * model.detail.y);
    }

    let eye = normalize(in.world_pos);
    let cos_o = clamp(-dot(n, eye), 0.001, 1.0);
    let l = normalize(model.sun_dir.xyz);
    let s0 = lit(n, l, eye, cos_o);
    var light = model.sun_color.rgb * model.sun_dir.w * s0.x;
    var spec = model.sun_color.rgb * model.sun_dir.w * s0.y;
    for (var i = 0; i < 3; i++) {
        if model.light_dir[i].w < 0.5 {
            break;
        }
        let si = lit(n, normalize(model.light_dir[i].xyz), eye, cos_o);
        light += model.light_color[i].rgb * si.x;
        spec += model.light_color[i].rgb * si.y;
    }
    // SE's highlight takes the diffuse colour as its specular colour (no SpecMap).
    let col = albedo * (light + model.base_color.w) + model.spec.rgb * albedo * spec;
    return vec4<f32>(col, 1.0);
}
