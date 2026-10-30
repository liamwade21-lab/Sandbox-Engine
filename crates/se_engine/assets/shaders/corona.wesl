// Stellar corona, SE's corona_plane_proc.glsl (drawn by FUN_1405481c0): three mutually
// perpendicular planes through the star, fixed in its body frame and `Radius` wide, each showing
// fbm rays added to the frame. A plane fades where it is seen edge-on and where it passes the
// camera; the star's depth hides the parts behind it. SE also draws the parts in front of the disc,
// but its photosphere outshines them about a thousandfold; the port's disc is far dimmer
// (system.rs STAR_SURFACE_BRIGHT), so the planes are cut where the line of sight meets the star's
// ellipsoid instead, which leaves the disc as SE shows it.

import bevy_pbr::render::mesh_functions::get_world_from_local;
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Corona {
    // rgb: SE's `Color` (the star colour at unit luma times the corona brightness), w unused
    color: vec4<f32>,
    // SE's `Bright`: (Gamma, Brightness, RayBrightness, SpotBrightness)
    bright: vec4<f32>,
    // SE's `Shape`: (RayDensity, RayCurvature, Scale, SinFreq)
    shape: vec4<f32>,
    // xyz: `EyePos.w` of the three planes (normals z, y, x in the body frame), w: the star's radius
    // over the corona's
    phase: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> corona: Corona;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // xyz: the plane's normal (world), w: the corona radius (world units, the longest semi-axis)
    @location(2) @interpolate(flat) plane: vec4<f32>,
    @location(3) @interpolate(flat) phase: f32,
    // The pixel and the camera in the planes' frame (body axes, corona radii along each), where the
    // star's ellipsoid is the sphere of radius `phase.w`.
    @location(4) local: vec3<f32>,
    @location(5) @interpolate(flat) cam_local: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let wp = (m * vec4<f32>(v.position, 1.0)).xyz;
    var o: VertexOut;
    o.clip = position_world_to_clip(wp);
    o.world_position = wp;
    o.uv = v.uv;
    // The planes are scaled by the star's ellipsoid along the body axes; a normal along one of them
    // stays along it.
    let radius = max(length(m[0].xyz), max(length(m[1].xyz), length(m[2].xyz)));
    o.plane = vec4<f32>(normalize((m * vec4<f32>(v.normal, 0.0)).xyz), radius);
    let n = abs(v.normal);
    o.phase = select(select(corona.phase.z, corona.phase.y, n.y > 0.5), corona.phase.x, n.z > 0.5);
    // The camera (the world origin) in the planes' frame: the columns are orthogonal.
    let t = -m[3].xyz;
    o.cam_local = vec3<f32>(dot(m[0].xyz, t) / dot(m[0].xyz, m[0].xyz), dot(m[1].xyz, t) / dot(m[1].xyz, m[1].xyz),
        dot(m[2].xyz, t) / dot(m[2].xyz, m[2].xyz));
    o.local = v.position;
    return o;
}

fn hash(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453);
}

fn noise(x: vec2<f32>) -> f32 {
    let p = floor(x);
    var f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    let n = p.x + p.y * 57.0;
    return mix(mix(hash(n), hash(n + 1.0), f.x), mix(hash(n + 57.0), hash(n + 58.0), f.x), f.y);
}

const M2: mat2x2<f32> = mat2x2<f32>(1.6, 1.2, -1.2, 1.6);

fn fbm(p0: vec2<f32>) -> f32 {
    var z = 0.5;
    var r = 0.0;
    var p = p0;
    for (var i = 1; i < 6; i = i + 1) {
        r = r + (sin(noise(p) * corona.shape.w) * 0.5 + 0.5) * z;
        z = z * 0.5;
        p = p * M2;
    }
    return r;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let uv0 = in.uv * 2.0 - 1.0;
    let r0 = length(uv0);
    // The camera is at the origin: `ray` runs from it to the pixel, in corona radii.
    let dist = length(in.world_position) / in.plane.w;
    let cos_view = abs(dot(in.world_position, in.plane.xyz)) / max(length(in.world_position), 1e-30);
    var fade = clamp(dist - 0.05, 0.0, 1.0) * clamp(1.0 - r0, 0.0, 1.0) * clamp(cos_view - 0.1, 0.0, 1.0);
    // Cut over the star's disc, a little inside its limb so the sphere mesh's facets leave no gap.
    let d = normalize(in.local - in.cam_local);
    let along = -dot(in.cam_local, d);
    let k = corona.phase.w;
    if along > 0.0 {
        fade *= smoothstep(k * 0.975, k * 0.99, length(in.cam_local + d * along));
    }
    if fade <= 0.0 {
        return vec4<f32>(0.0);
    }
    let shape = corona.shape;
    let bright = corona.bright;
    let uv = uv0 * shape.y * 0.05;
    let uvn = normalize(uv + vec2<f32>(1e-12, 0.0));
    let r = length(uv);
    let x = dot(uvn, vec2<f32>(0.5, 0.0)) - in.phase;
    let y = dot(uvn, vec2<f32>(0.0, 0.5)) - in.phase;
    var val = fbm(shape.z * vec2<f32>(r + y * shape.x, r + x * shape.x - y));
    val = sqrt(smoothstep(bright.x, bright.x + bright.z, val));
    let color = mix(vec3<f32>(1.0 - val), vec3<f32>(1.0), bright.w - 2000.0 * r / (shape.y * bright.y));
    // SE's gamma correction, then the colour; additive (premultiplied with alpha 0).
    let c = pow(fade * max(color, vec3<f32>(0.0)), vec3<f32>(2.2)) * corona.color.rgb;
    return vec4<f32>(c, 0.0);
}
