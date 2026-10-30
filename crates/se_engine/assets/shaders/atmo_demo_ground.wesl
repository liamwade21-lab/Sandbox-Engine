// Demo-only planet surface: an analytic ray-traced sphere (full-screen triangle, writes depth) with a
// procedural albedo, lit through `atm_ground_atmosphere`. Used by src/bin/atmo_demo.rs to exercise the
// ground-side API of atmosphere_common.wesl. Centre = model translation, radius = model scale.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_world_from_local;
import super::atmosphere_common::{AtmParams, atm_ground_atmosphere};

struct GroundUniform {
    atm: AtmParams,
    sun_dir: vec4<f32>,
    sun_irradiance: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> ground: GroundUniform;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var inscatter_tex: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atm_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var transmittance_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var irradiance_tex: texture_2d<f32>;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
    @location(1) @interpolate(flat) center_radius: vec4<f32>,
};

@vertex
fn vertex(@builtin(instance_index) instance_index: u32, @location(0) position: vec3<f32>) -> VertexOut {
    // The mesh is one oversized triangle whose positions are the NDC corners (-1,-1), (3,-1), (-1,3).
    let m = get_world_from_local(instance_index);
    let p = position.xy;
    var o: VertexOut;
    o.clip = vec4<f32>(p, 0.99, 1.0);
    o.ndc = p;
    o.center_radius = vec4<f32>(m[3].xyz, length(m[0].xyz));
    return o;
}

fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
                mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
                mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

fn albedo_at(n: vec3<f32>) -> vec3<f32> {
    // Continents from low-frequency noise, texture detail at several scales (cell sizes ~ 800, 100, 12 km).
    let land = vnoise(n * 7.0 + vec3<f32>(3.1, 1.7, 0.4)) * 0.6 + vnoise(n * 23.0) * 0.3 + vnoise(n * 61.0) * 0.1;
    let detail = vnoise(n * 8000.0) * 0.5 + vnoise(n * 500000.0) * 0.5;
    let grid = step(0.5, fract(n.x * 400.0)) + step(0.5, fract(n.z * 400.0));
    var col = vec3<f32>(0.04, 0.07, 0.18); // ocean
    if land > 0.42 {
        let dry = smoothstep(0.5, 0.75, vnoise(n * 11.0 + 5.0));
        col = mix(vec3<f32>(0.10, 0.20, 0.06), vec3<f32>(0.32, 0.26, 0.15), dry) * (0.7 + 0.6 * detail);
        col = col * (0.9 + 0.1 * (grid - 1.0));
    }
    let snow = smoothstep(0.9, 0.97, abs(n.x));
    return mix(col, vec3<f32>(0.8), snow);
}

struct FragOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOut) -> FragOut {
    let wn = view.world_from_clip * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(wn.xyz / wn.w - view.world_position);
    let radius = in.center_radius.w;
    let oc = view.world_position - in.center_radius.xyz;
    let r0 = length(oc);
    let b = dot(oc, dir);
    let c = (r0 - radius) * (r0 + radius);
    let disc = b * b - c;
    if disc < 0.0 || b >= 0.0 {
        discard;
    }
    // Numerically stable nearest root: t = c / (-b + sqrt(disc)).
    let t = c / (-b + sqrt(disc));
    let pos_w = view.world_position + dir * t;
    let to_table = ground.atm.radii.x / radius; // km per world unit (planet radius <-> Rg)
    let cam_km = oc * to_table;
    let pos_km = cam_km + dir * (t * to_table);
    let n = normalize(pos_km);
    let s = normalize(ground.sun_dir.xyz);
    let e_sun = ground.sun_irradiance.rgb * ground.sun_irradiance.w;
    let col = atm_ground_atmosphere(ground.atm, inscatter_tex, transmittance_tex, irradiance_tex, atm_sampler,
                                    albedo_at(n), n, s, e_sun, 1.0, cam_km, pos_km);
    let clip = view.clip_from_world * vec4<f32>(pos_w, 1.0);
    var o: FragOut;
    o.color = vec4<f32>(col, 1.0);
    o.depth = clip.z / clip.w;
    return o;
}
