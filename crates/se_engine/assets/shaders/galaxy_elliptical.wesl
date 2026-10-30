// Elliptical galaxies (`ellipticals.rs`): SE's `galaxy_bulge.glsl` with `ColorCorrentionHDR` of hsl.glh
// (decrypted, re/se_shaders). The back faces of the galaxy's ellipsoid, projected to infinity like the other
// galaxy layers, look up the pre-integrated density table by the ray's offset from the centre and its half
// length inside the (unit-sphere) model.

import bevy_pbr::render::mesh_view_bindings::view;

struct BulgeParams {
    // xyz: galaxy centre relative to the camera (pc), w: ellipsoid radius (pc)
    center_rel: vec4<f32>,
    // columns of the universe -> galaxy rotation
    rot0: vec4<f32>,
    rot1: vec4<f32>,
    rot2: vec4<f32>,
    // xyz: the ellipsoid's axes in radii
    axes: vec4<f32>,
    // SE's Color and ColorEffects (brightness - 1, contrast, saturation - 1, 1 / gamma)
    color: vec4<f32>,
    effects: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> u: BulgeParams;
// SE's PrecomputedDensityFunc (galaxy_bulge.tif, 512 x 64).
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var table: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var table_samp: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // SE's vPosition: the vertex on the model's unit sphere.
    @location(0) local: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let rot = mat3x3<f32>(u.rot0.xyz, u.rot1.xyz, u.rot2.xyz);
    let rel = u.center_rel.xyz + transpose(rot) * (v.position * u.axes.xyz) * u.center_rel.w;
    var clip = view.clip_from_world * vec4<f32>(rel, 0.0);
    clip.z = 0.0;
    var o: VertexOut;
    o.clip = clip;
    o.local = v.position;
    return o;
}

fn color_correction_hdr(c_in: vec3<f32>, bcsg: vec4<f32>) -> vec3<f32> {
    // Reinhard to LDR, contrast and brightness, vibrance, gamma, back to HDR.
    var c = c_in / (1.0 + c_in);
    c = mix(vec3<f32>(0.5), c, bcsg.y) + bcsg.xxx;
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sat = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
    c = mix(vec3<f32>(luma), c, 1.0 + bcsg.z * (1.0 - sat));
    c = mix(vec3<f32>(luma), c, 1.0 + bcsg.z * (1.0 - sat * sign(bcsg.z)));
    c = pow(max(c, vec3<f32>(1.0e-6)), bcsg.www);
    c = c / (1.0 - c);
    return max(c, vec3<f32>(0.0));
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    // SE's EyePos is minus the eye in model space (the unit sphere of the ellipsoid).
    let rot = mat3x3<f32>(u.rot0.xyz, u.rot1.xyz, u.rot2.xyz);
    let eye = (rot * (-u.center_rel.xyz)) / (u.center_rel.w * u.axes.xyz);
    let vert = normalize(in.local);
    let ray = vert - eye;                          // from the eye to the pixel's back-face point
    let rlen = length(ray) + 1.0e-4;
    var depth = dot(vert, ray / rlen);             // half length of the chord along the ray
    let r = sqrt(max(1.0 - depth * depth, 0.0));   // the ray's offset from the centre
    let offset = pow(0.5 * r, 0.25);               // the table's parametrisation
    depth = min(depth, 0.5 * rlen);                // inside: half of what lies ahead
    let density = textureSampleLevel(table, table_samp, vec2<f32>(offset, depth), 0.0).r;
    let c = u.color * density * 12.0;
    return vec4<f32>(color_correction_hdr(c.rgb, u.effects) * 10.0, c.a);
}
