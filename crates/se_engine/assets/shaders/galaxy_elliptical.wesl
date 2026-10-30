// Volume of an elliptical galaxy (`ellipticals.rs`): the back faces of a sphere around the galaxy, drawn
// at infinity like the other galaxy layers, integrate the light of an ellipsoid along each view ray.
// Additive output, so it works from outside and from inside the galaxy.

import bevy_pbr::render::mesh_view_bindings::view;

struct VolumeParams {
    // xyz: galaxy centre relative to the camera (pc), w: radius (pc)
    center_rel: vec4<f32>,
    // columns of the universe -> galaxy rotation
    rot0: vec4<f32>,
    rot1: vec4<f32>,
    rot2: vec4<f32>,
    // rgb: colour * brightness per unit column density (radii), w: axis ratio
    color: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> vol: VolumeParams;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Point of the sphere relative to the camera (pc): interpolated, it gives the pixel's view ray.
    @location(0) ray: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let rel = vol.center_rel.xyz + v.position * vol.center_rel.w;
    var clip = view.clip_from_world * vec4<f32>(rel, 0.0);
    clip.z = 0.0;
    var o: VertexOut;
    o.clip = clip;
    o.ray = rel;
    return o;
}

// Deprojected de Vaucouleurs profile (Prugniel-Simien, n = 4) with the effective radius at a quarter of the
// radius and the far sprite's window. Must match `density` in ellipticals.rs.
fn density(m: f32) -> f32 {
    if m >= 1.0 {
        return 0.0;
    }
    let r = sqrt(m * m + 1.0e-4) / 0.25;
    let w = 1.0 - m * m;
    return pow(r, -0.851) * exp(-7.67 * (pow(r, 0.25) - 1.0)) * w * w;
}

const STEPS: i32 = 24;

// Integral of the density along s in [sa, sb] (s >= 0 measured from the ray's closest approach to the
// centre, where m^2 = b2 + (k s)^2). Log-spaced steps from the scale h: dense near the centre, where the
// profile is cuspy, and still covering the whole outer body.
fn side(sa: f32, sb: f32, b2: f32, k: f32, h: f32) -> f32 {
    let ua = log(1.0 + sa / h);
    let ub = log(1.0 + sb / h);
    let du = (ub - ua) / f32(STEPS);
    var acc = 0.0;
    for (var i = 0; i < STEPS; i = i + 1) {
        let e = exp(ua + (f32(i) + 0.5) * du);
        let s = h * (e - 1.0);
        acc = acc + density(sqrt(b2 + k * k * s * s)) * h * e;
    }
    return acc * du;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let rot = mat3x3<f32>(vol.rot0.xyz, vol.rot1.xyz, vol.rot2.xyz);
    // Camera and ray in the galaxy frame, in radii, with z stretched by 1 / q (the ellipsoid becomes the
    // unit sphere; t stays the true distance along the ray).
    let q = vol.color.w;
    let stretch = vec3<f32>(1.0, 1.0, 1.0 / q);
    let o = rot * (-vol.center_rel.xyz / vol.center_rel.w) * stretch;
    let d = rot * normalize(in.ray) * stretch;
    let k2 = dot(d, d);
    let k = sqrt(k2);
    let tc = -dot(o, d) / k2;
    let b2 = max(dot(o, o) - tc * tc * k2, 0.0);
    if b2 >= 1.0 {
        discard;
    }
    let half_len = sqrt((1.0 - b2) / k2);
    let t0 = max(tc - half_len, 0.0);
    let t1 = tc + half_len;
    if t1 <= t0 {
        discard;
    }
    let h = sqrt(b2 + 1.0e-4) / k;
    var sum = 0.0;
    if t0 < tc {
        sum = side(0.0, tc - t0, b2, k, h) + side(0.0, t1 - tc, b2, k, h);
    } else {
        sum = side(t0 - tc, t1 - tc, b2, k, h);
    }
    // Premultiplied blending in Bevy: alpha 0 makes AlphaMode::Add purely additive.
    return vec4<f32>(vol.color.rgb * sum, 0.0);
}
