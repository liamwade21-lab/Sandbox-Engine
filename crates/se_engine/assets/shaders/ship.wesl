// Spacecraft hull and catalog model material: SE's ship.glsl, with the ShipUniforms that
// FUN_14029f200 / FUN_14029f670 fill from the .sml material (parsed by FUN_1402a1fb0).
// Up to 4 stars light the surface with a Lambert / Hapke mix (AmbientParams.w), each dimmed by
// its eclipse shadow; a tangent-space normal map with a detail normal map at two scales, a
// detail diffuse map at two scales, a specular map with a normalised Blinn highlight, an
// emissive map and the thermal glow of a temperature map (through SE's Planck table) are added,
// then the ambient level times the diffuse colour. The camera sits at the render-space origin
// (floating origin), so the world position is the eye vector.
//
// The diffuse, specular and emissive maps are sampled through sRGB views (linear colour); the
// normal, detail and temperature maps are raw.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world, mesh_tangent_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

// Shader variant switches (the defines of ship.glsl), in `flags.x`.
const F_SPEC: u32 = 1u;     // SPEC: a SpecMap
const F_ASPEC: u32 = 2u;    // ASPEC: the diffuse alpha is the specular power (DiffMapAlpha "Spec")
const F_ATRNS: u32 = 4u;    // ATRNS: alpha-tested cut-out (DiffMapAlpha "Transp")
const F_EMIS: u32 = 8u;     // EMIS: an EmisMap
const F_DETDIF: u32 = 16u;  // DETDIF: a DetDiffMap
const F_DETNRM: u32 = 32u;  // DETNRM: a DetBumpMap / DetNormMap
const F_THERM: u32 = 64u;   // THERM: a TempMap on a design with a RadTemp

const NORM_ONE: f32 = 1.0 - 1.0 / 255.0;

struct Hull {
    // xyz: unit direction towards each light (SE's MAX_LIGHTS 4); w: 1 for a light in use
    light_dir: array<vec4<f32>, 4>,
    // rgb: light colour * intensity * eclipse shadow (LightColor, ECL)
    light_color: array<vec4<f32>, 4>,
    // x: AmbientParams.x (ambient level); y: AmbientParams.w (Lambert to Hapke lerp);
    // z: Params.w (brightness); w: LightParams.z (specular power factor of the lights)
    ambient: vec4<f32>,
    // HapkeParams: spot bright, 1 / spot width, coherent-backscatter bright, 1 / its width
    hapke: vec4<f32>,
    // ThermalParams: x RadTemp (kK), y ThermalEmissionShift, z brightness
    thermal: vec4<f32>,
    // DiffColorScale, DiffColorConst (rgb)
    diff_scale: vec4<f32>,
    diff_const: vec4<f32>,
    // EmisColor * EmisBright (rgb)
    emis: vec4<f32>,
    // SpecColorScale: rgb SpecColor, a SpecPower
    spec: vec4<f32>,
    // DetailScale: DetDiffScale, DetDiffMag, DetBumpScale, DetBumpMag
    detail: vec4<f32>,
    // MapCorrections: DiffGamma, SpecGamma, EmisGamma, 1 for a DirectX normal map
    corrections: vec4<f32>,
    flags: vec4<u32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> hull: Hull;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diff_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var norm_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var norm_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var spec_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var spec_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var emis_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var emis_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var det_diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var det_diff_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var det_norm_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(12) var det_norm_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(13) var temp_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(14) var temp_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(15) var planck_tex: texture_2d<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) world_tangent: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_pos = world.xyz;
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.world_tangent = mesh_tangent_local_to_world(m, v.tangent, v.instance_index);
    o.uv = v.uv;
    return o;
}

fn has(f: u32) -> bool {
    return (hull.flags.x & f) != 0u;
}

// MapCorrections: 1 keeps the map, a positive value is a power. A negative one asks for
// sRGB2linear, which the sRGB views have already done.
fn correct(c: vec3<f32>, g: f32) -> vec3<f32> {
    if g > 0.0 && g != 1.0 {
        return pow(max(c, vec3<f32>(0.0)), vec3<f32>(g));
    }
    return c;
}

// texture_mix.glh BlendNormals, NORMALS_BLEND_METHOD 3 (Unreal development network).
fn blend_normals(n1: vec3<f32>, n2: vec3<f32>, t: f32) -> vec3<f32> {
    return normalize(vec3<f32>(mix(n1.xy, n2.xy, t), n1.z));
}

// ship.glsl Hapke(): the hot spot correction.
fn hapke(mu: f32) -> f32 {
    let t = sqrt((1.0 - mu) / (1.0 + mu));
    return 1.0 + hull.hapke.x / (1.0 + t * hull.hapke.y) + hull.hapke.z / (1.0 + t * hull.hapke.w);
}

// SE's PlanckFunction 1D texture (4096 texels), linearly filtered by hand.
fn planck(u: f32) -> vec3<f32> {
    let n = f32(textureDimensions(planck_tex).x);
    let x = clamp(u, 0.0, 1.0) * n - 0.5;
    let i = clamp(floor(x), 0.0, n - 1.0);
    let f = clamp(x - i, 0.0, 1.0);
    let a = textureLoad(planck_tex, vec2<i32>(i32(i), 0), 0).rgb;
    let b = textureLoad(planck_tex, vec2<i32>(i32(min(i + 1.0, n - 1.0)), 0), 0).rgb;
    return mix(a, b, f);
}

// Opaque materials: no discard in this entry point, so the depth test stays early (hull meshes
// have many overlapping layers).
@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, textureSample(diff_tex, diff_smp, in.uv));
}

// ATRNS materials (DiffMapAlpha "Transp"): alpha test at 0.5.
@fragment
fn fragment_cutout(in: VertexOut) -> @location(0) vec4<f32> {
    let diff = textureSample(diff_tex, diff_smp, in.uv);
    if diff.a < 0.5 {
        discard;
    }
    return shade(in, diff);
}

// `diff_map`: the diffuse colour in rgb, transparency or specular power in alpha.
fn shade(in: VertexOut, diff_map: vec4<f32>) -> vec4<f32> {
    var diff = vec4<f32>(correct(diff_map.rgb, hull.corrections.x) * hull.diff_scale.rgb + hull.diff_const.rgb, diff_map.a);

    // ship.glsl flips the normal of back faces only under ALPHA_TRANSP, which SE never defines;
    // back faces are culled unless the material is TwoSided.
    let normal = normalize(in.world_normal);

    // Normal map (and detail normal map) in tangent space, expanded to [-1, 1].
    var bump = textureSample(norm_tex, norm_smp, in.uv).xyz;
    if hull.corrections.w > 0.5 {
        bump.y = 1.0 - bump.y;
    }
    var nv = 2.0 * bump - NORM_ONE;
    if has(F_DETNRM) {
        let d0 = 2.0 * textureSample(det_norm_tex, det_norm_smp, in.uv * hull.detail.z).xyz - NORM_ONE;
        let d1 = 2.0 * textureSample(det_norm_tex, det_norm_smp, in.uv * hull.detail.z * 4.0).xyz - NORM_ONE;
        nv = blend_normals(nv, blend_normals(d0, d1, 0.5), hull.detail.w);
    } else {
        nv = normalize(nv);
    }
    var n = normal;
    if dot(in.world_tangent.xyz, in.world_tangent.xyz) > 1e-12 {
        let t = normalize(in.world_tangent.xyz);
        let b = in.world_tangent.w * cross(t, normal);
        n = normalize(mat3x3<f32>(t, b, normal) * nv);
    }

    let eye = normalize(in.world_pos);
    let cos_o = clamp(-dot(n, eye), 0.001, 1.0);

    // Detail diffuse colour at two scales.
    if has(F_DETDIF) {
        let k = hull.detail.y;
        let c0 = mix(vec4<f32>(0.5), textureSample(det_diff_tex, det_diff_smp, in.uv * hull.detail.x), k);
        let c1 = mix(vec4<f32>(0.5), textureSample(det_diff_tex, det_diff_smp, in.uv * hull.detail.x * 4.0), k);
        diff = diff * c0 * c1 * (4.0 + 6.0 * k * k);
    }

    // Specular colour in rgb and power in alpha.
    let specular = has(F_SPEC) || has(F_ASPEC);
    var spec_c = vec4<f32>(0.0);
    if has(F_SPEC) {
        spec_c = textureSample(spec_tex, spec_smp, in.uv);
        if hull.corrections.y != 1.0 {
            // ship.glsl raises the specular map to the diffuse gamma.
            spec_c = vec4<f32>(correct(spec_c.rgb, select(1.0, hull.corrections.x, hull.corrections.y > 0.0)), spec_c.a);
        }
        if has(F_ASPEC) {
            spec_c += diff;
        }
    } else if has(F_ASPEC) {
        spec_c = diff;
    }
    spec_c.a *= hull.spec.a;

    // Lambert / Hapke lighting and the specular highlights of each light.
    var lit = vec3<f32>(0.0);
    var spec_lit = vec3<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        if hull.light_dir[i].w < 0.5 {
            continue;
        }
        let l = normalize(hull.light_dir[i].xyz);
        let cos_i = clamp(dot(n, l), 0.0, 1.0);
        let mu = clamp(-dot(l, eye), -0.999, 1.0);
        let lf = mix(cos_i, cos_i / (cos_i + cos_o) * hapke(mu), hull.ambient.y);
        let sun = hull.light_color[i].rgb * lf;
        lit += sun;
        if specular {
            let h = normalize(l - eye);
            let p = max(spec_c.a * hull.ambient.w, 1.0);
            spec_lit += (p * 0.039788735 + 0.31830989) * pow(clamp(dot(h, n), 0.0, 1.0), p) * sun;
        }
    }
    var col = lit * diff.rgb;
    if specular {
        col += hull.spec.rgb * spec_c.rgb * spec_lit;
    }
    if has(F_EMIS) {
        col += hull.emis.rgb * correct(textureSample(emis_tex, emis_smp, in.uv).rgb, hull.corrections.z);
    }
    if has(F_THERM) {
        let temp = textureSample(temp_tex, temp_smp, in.uv).r * hull.thermal.x;
        col += planck(log(temp + hull.thermal.y) * 0.188 + 0.1316) * hull.thermal.z * 500.0;
    }
    var result = (hull.ambient.x * diff.rgb + col) * hull.ambient.z;
    // Limit the brightness while preserving colour.
    let luma = max(result.r, max(result.g, result.b));
    result *= clamp(65000.0 / (luma + 1.0e-10), 0.0, 1.0);
    return vec4<f32>(result, 1.0);
}
