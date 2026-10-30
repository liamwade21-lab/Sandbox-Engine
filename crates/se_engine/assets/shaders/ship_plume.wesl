// Engine exhaust: SE's ship_engines.glsl. Each engine effect is a box ([-1, 1]^3 in its own
// frame, the nozzle on the z = +1 face, the exhaust towards -z); the fragment ray-marches the
// view ray through the box (up to 128 steps of ScaleStep.w) summing the density of the engine's
// type (the ship_*_engine.glh files), times the colour. From outside the box its front faces are
// drawn (the hull in front hides the exhaust, the hull behind it does not); with the camera inside,
// its back faces, the march starting at the camera. Output alpha is 0 so that AlphaMode::Add
// (premultiplied) adds colour only.

import bevy_pbr::render::mesh_functions::{get_world_from_local, get_local_from_world, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Plume {
    // rgb: SE's Color uniform (effect colour times level and brightness)
    color: vec4<f32>,
    // x: engine type (0 Fusion, 1 Chemical, 2 Ion, 3 Ion2, 4 Aerospike, 5 Sphere, 6 Square,
    // 7 SuperDraco_1atm); y: time (s, EyePosTime.w); z: integration step (ScaleStep.w)
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> plume: Plume;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The box-space vertex position and eye (the camera sits at the render-space origin).
    @location(0) local_pos: vec3<f32>,
    @location(1) eye: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.local_pos = v.position;
    let inv = get_local_from_world(v.instance_index);
    o.eye = (inv * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    return o;
}

// Hash based Perlin noise (ship_engines.glsl).
fn hash(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453123);
}

fn noise(x: vec3<f32>) -> f32 {
    let p = floor(x);
    var f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    let n = p.x + p.y * 157.0 + 113.0 * p.z;
    return mix(
        mix(mix(hash(n), hash(n + 1.0), f.x), mix(hash(n + 157.0), hash(n + 158.0), f.x), f.y),
        mix(mix(hash(n + 113.0), hash(n + 114.0), f.x), mix(hash(n + 270.0), hash(n + 271.0), f.x), f.y),
        f.z,
    );
}

// GLSL smoothstep, also with the edges reversed as the .glh files use it.
fn ss(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Noise modulating the exhaust core, `xy_scale` the noise frequency across the jet.
fn core_noise(p: vec3<f32>, l: f32, xy_scale: f32, t: f32) -> f32 {
    var n = p;
    n = vec3<f32>(n.xy * xy_scale, n.z + t);
    return noise(n) * sqrt((l - 0.25) * 1.333);
}

// ship_Fusion_engine.glh
fn fusion(p: vec3<f32>, t: f32) -> f32 {
    let r2 = dot(p.xy, p.xy);
    let l = clamp(0.5 - 0.5 * p.z, 0.0, 1.0);
    var nz = 1.0;
    if r2 * l < 0.04 && l > 0.25 {
        nz -= core_noise(p, l, 6.7, t);
    }
    let core = exp(-8.0 * r2) * exp(-8.0 * l) * l * (1.0 - ss(0.01, 0.04, r2 * l));
    let envelope = exp(-4.0 * r2) * exp(-4.0 * l) * ss(0.0, 0.49, r2);
    return (core * 40.0 * nz + envelope) * ss(1.0, 0.75, l);
}

// ship_Chemical_engine.glh
fn chemical(p: vec3<f32>, t: f32) -> f32 {
    let r2 = 1.0 / dot(p.xy, p.xy);
    let l = clamp(0.5 - 0.5 * p.z, 0.0, 1.0);
    var nz = 2.0;
    if l > 0.25 {
        nz -= core_noise(p, l, 6.7, t);
    }
    var core = 2.0 - 2.0 * pow(l, 0.25);
    core = 0.6 * ss(0.5, 3.0, r2 * (l + 0.05)) * core * core * core;
    return core * nz;
}

// ship_Ion_engine.glh (`ion2`: ship_Ion2_engine.glh, whose envelope has no hole)
fn ion(p: vec3<f32>, t: f32, ion2: bool) -> f32 {
    let r2 = dot(p.xy, p.xy);
    let l = clamp(0.5 - 0.5 * p.z, 0.0, 1.0);
    var nz = 1.0;
    if l > 0.25 {
        nz -= core_noise(p, l, 6.7, t);
    }
    // Ion2's smoothstep(0.0, 0.0, r2 * 0.01) is 1 off the axis.
    let hole = select(ss(0.0, 0.49, r2), 1.0, ion2);
    let core = exp(-4.0 * r2) * exp(-4.0 * l) * hole;
    return core * nz * ss(1.0, 0.75, l);
}

// ship_Aerospike_engine.glh: a 15 x 2 grid of jets.
fn aerospike(p: vec3<f32>, t: f32) -> f32 {
    let jets = vec2<f32>(15.0, 2.0);
    var pp = p * vec3<f32>(0.5, 0.5, -0.5) + vec3<f32>(0.5);
    pp = vec3<f32>(abs(fract(pp.xy * jets) * 2.0 - vec2<f32>(1.0)), pp.z);
    let r2 = dot(pp.xy, pp.xy);
    var nz = 1.0;
    if pp.z > 0.25 {
        let n = vec3<f32>(p.xy * 16.7, p.z + t);
        nz = max(0.0, 1.0 - noise(n) * sqrt((pp.z - 0.25) * 2.333));
    }
    let core = exp(-5.6 * pp.z) * exp(-4.0 * r2) * (1.0 - pp.z) * nz;
    let envelope = exp(-4.0 * pp.z);
    return (core + envelope * 0.03) * ss(1.0, 0.75, pp.z);
}

// ship_Sphere_engine.glh (its clamp(-z, z, -z) is -z).
fn sphere(p: vec3<f32>) -> f32 {
    let r2 = dot(p.xy, p.xy);
    let l = -p.z;
    let l2 = dot(p.yz, p.yz);
    let v2 = dot(p.xz, p.xz);
    let core = exp(-4.0 * r2) * exp(-4.0 * l2) * exp(-4.0 * v2);
    return core * ss(1.0, 0.5, l);
}

// ship_Square_engine.glh
fn square(p: vec3<f32>, t: f32) -> f32 {
    let l = clamp(0.5 - 0.5 * p.z, 0.0, 1.0);
    var nz = 1.0;
    if l > 0.25 {
        nz -= core_noise(p, l, 6.7, t);
    }
    let core = 0.5 * exp(-4.0 * l);
    return core * nz * ss(1.0, 0.75, l);
}

// ship_SuperDraco_1atm_engine.glh
fn superdraco(p: vec3<f32>, t: f32) -> f32 {
    let r2 = 1.0 / dot(p.xy, p.xy);
    let l = 0.5 - 0.5 * p.z;
    let hi = step(0.5, l);
    var n = p;
    n.z *= 0.005 * t * (1.0 - hi) + 0.02 * t * hi;
    n = vec3<f32>(n.xy + t, n.z);
    let nn = noise(n);
    let l1 = 0.5 - 0.5 * sin(p.z * 8.0 * (4.0 - p.z) + 0.4 * nn);
    let core = 0.5 * ss(0.5, 3.0, (r2 - 4.0) * (l1 * (1.0 - l1) + 0.05))
        * pow(mix(2.0, 0.0, sqrt(sqrt(l1 * (1.0 - l1) + 0.01) - 0.01)), 3.0);
    let mask = 0.5 * ss(0.0, 2.0, r2 * 0.5 * (0.35 - l + 0.05)) * l;
    let envelope = 1.5 * ss(0.8, 3.0, (r2 + 0.8 * nn) * (l + 0.05))
        * clamp(pow(mix(-8.0, 2.0, sqrt(sqrt(1.0 - l + 0.5 * nn + 0.01) - 0.01)), 3.0), 0.0, 2.0);
    let mask1 = 1.5 * ss(0.0, 3.0, l + 0.05) * (1.0 - hi) + 1.5 * ss(0.0, 3.0, 1.0 - l + 0.05) * hi;
    return 2.0 * core * mask + envelope * mask1;
}

fn density(kind: i32, p: vec3<f32>, t: f32) -> f32 {
    switch kind {
        case 0: { return fusion(p, t); }
        case 1: { return chemical(p, t); }
        case 2: { return ion(p, t, false); }
        case 3: { return ion(p, t, true); }
        case 4: { return aerospike(p, t); }
        case 5: { return sphere(p); }
        case 6: { return square(p, t); }
        default: { return superdraco(p, t); }
    }
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let ro = in.eye;
    let rd = normalize(in.local_pos - in.eye);
    // RayBoxIntersect against [-1, 1]^3.
    let safe = select(rd, vec3<f32>(1e-8), abs(rd) < vec3<f32>(1e-8));
    let m = 1.0 / safe;
    let n = m * ro;
    let k = abs(m);
    let t1 = -n - k;
    let t2 = -n + k;
    let near = max(max(t1.x, t1.y), t1.z);
    let far = min(min(t2.x, t2.y), t2.z);
    var intensity = 0.0;
    if near <= far && far >= 0.0 {
        let kind = i32(plume.params.x + 0.5);
        let time = plume.params.y;
        let step_len = plume.params.z;
        var t = max(near, 0.0);
        for (var i = 0; i < 128 && t < far; i++) {
            intensity += density(kind, ro + t * rd, time) * step_len;
            t += step_len;
        }
    }
    return vec4<f32>(max(intensity, 0.0) * plume.color.rgb, 0.0);
}
