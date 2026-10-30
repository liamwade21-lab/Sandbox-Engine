// Stellar corona: the far half of an additive shell `extent` star radii wide. Each pixel takes
// the brightness at its view ray's closest approach to the star, so the limb ring sits on the
// sphere's silhouette at any distance and the glow surrounds the camera when it is close (a flat
// billboard showed hard-edged sheets there). The star sphere's depth hides it inside the disc.

import bevy_pbr::render::mesh_functions::get_world_from_local;
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Corona {
    // rgb colour, w intensity at the limb
    color: vec4<f32>,
    // x: extent (star radii), y: seed, z: streamer strength
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> corona: Corona;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    // xyz: star centre (camera-relative), w: star radius.
    @location(1) @interpolate(flat) star: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let centre = m[3].xyz;
    let radius = length(m[0].xyz);
    let wp = centre + normalize(v.position) * radius * corona.params.x;
    var o: VertexOut;
    o.clip = position_world_to_clip(wp);
    o.world_position = wp;
    o.star = vec4<f32>(centre, radius);
    return o;
}

fn hash31(p: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 0x8da6b343u) ^ (bitcast<u32>(p.y) * 0xd8163841u) ^ (bitcast<u32>(p.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967295.0);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let b = vec3<i32>(i);
    let x0 = mix(mix(hash31(b), hash31(b + vec3<i32>(1, 0, 0)), u.x),
                 mix(hash31(b + vec3<i32>(0, 1, 0)), hash31(b + vec3<i32>(1, 1, 0)), u.x), u.y);
    let x1 = mix(mix(hash31(b + vec3<i32>(0, 0, 1)), hash31(b + vec3<i32>(1, 0, 1)), u.x),
                 mix(hash31(b + vec3<i32>(0, 1, 1)), hash31(b + vec3<i32>(1, 1, 1)), u.x), u.y);
    return mix(x0, x1, u.z);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let ext = corona.params.x;
    // The camera is at the origin: closest approach of the view ray (ahead of the camera).
    let d = normalize(in.world_position);
    let offset = d * max(dot(in.star.xyz, d), 0.0) - in.star.xyz;
    let r = length(offset) / in.star.w;
    let x = max(r, 1.0);
    // Steep inner (K) corona plus a broad faint outer (F) component.
    let profile = 0.85 * pow(x, -4.0) + 0.15 * pow(x, -1.8);
    // Radial streamers: noise over direction only, so they stretch outwards.
    let seed = vec3<f32>(corona.params.y * 37.0);
    let dir = normalize(offset + vec3<f32>(1e-20));
    let s = vnoise(dir * 5.0 + seed) * 0.65 + vnoise(dir * 13.0 + seed) * 0.35;
    let streak = mix(1.0, 0.25 + 1.6 * s * s, corona.params.z * smoothstep(1.0, 1.6, r));
    let fade = 1.0 - smoothstep(0.5 * ext, ext, r);
    let limb = smoothstep(0.97, 1.01, r);
    let i = corona.color.w * profile * streak * fade * limb;
    // Additive (premultiplied with alpha 0).
    return vec4<f32>(corona.color.rgb * i, 0.0);
}
