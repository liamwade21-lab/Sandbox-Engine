// Alcubierre warp bubble: SE's einstein.glsl with WARP (ALCUBIERRE_METRIC), see ship/warp.rs.
//
// Two passes (the compute pass binds only the uniform), as SE's impostor ("COMPUTE + STORE", then "SAMPLE + RENDER"):
//  * `compute` traces each view ray through the Alcubierre metric of the bubble around the ship
//    (gr_geodesic.glh TraceGeodesic: Hamiltonian geodesic integration, numerical metric
//    gradient) at a reduced resolution and stores the bent direction (view space) and the
//    ray's final momentum, SE's passParam;
//  * `render` / `render_ms` sample that map and do SE's Skybox() for WARP: the frame where the
//    bent ray lands on screen, the environment map beyond the screen and where the bent ray lands
//    on geometry, the "geodesic redshift" by passParam, and the ship itself (unbent pixels with
//    depth) kept, bent with the rest when the camera is outside the bubble.
//
// Units: the ray space of SE's shader, where the bubble wall is at R = 1; the motion is along +x.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::accretion_disk::{linearstep, sstep};

struct Warp {
    // tan(fov/2) horizontally / vertically, environment map valid (1), unused.
    view: vec4<f32>,
    // Columns of the view -> world (environment) rotation.
    sky0: vec4<f32>,
    sky1: vec4<f32>,
    sky2: vec4<f32>,
    // Columns of the view -> bubble rotation (bubble x: direction of motion).
    bub0: vec4<f32>,
    bub1: vec4<f32>,
    bub2: vec4<f32>,
    // Camera position in ray units; w: EyeDist * invGravRadius, half its length.
    eye: vec4<f32>,
    // MetricParams warpV, bubbleSig; warpStep (Params.y); render sphere radius in ray units.
    metric: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var lin: sampler;
@group(0) @binding(2) var<uniform> warp: Warp;
@group(0) @binding(3) var warp_map: texture_2d<f32>;
@group(0) @binding(4) var env_px: texture_2d<f32>;
@group(0) @binding(5) var env_nx: texture_2d<f32>;
@group(0) @binding(6) var env_py: texture_2d<f32>;
@group(0) @binding(7) var env_ny: texture_2d<f32>;
@group(0) @binding(8) var env_pz: texture_2d<f32>;
@group(0) @binding(9) var env_nz: texture_2d<f32>;
@group(0) @binding(10) var depth: texture_depth_2d;
@group(0) @binding(11) var depth_ms: texture_depth_multisampled_2d;

// gr_geodesic.glh: max_scale, the numerical derivative eps, 256 steps.
const MAX_DISTANCE: f32 = 100.0;
const EPS: f32 = 0.02;
const MAX_STEPS: i32 = 256;

fn tanh_safe(x: f32) -> f32 {
    return tanh(clamp(x, -4.0, 4.0));
}

// AlcubierreR: distance from the bubble centre, which moves along x at warpV.
fn alcubierre_r(m: vec4<f32>, q: vec4<f32>) -> f32 {
    let x = m.x * q.x;
    return sqrt((q.y - x) * (q.y - x) + q.z * q.z + q.w * q.w);
}

// MetricBase().x: the shift g_xt = -warpV f(r), f the bubble's top-hat of thickness bubbleSig.
fn shift(m: vec4<f32>, q: vec4<f32>) -> f32 {
    let r = alcubierre_r(m, q);
    let s = m.y;
    let f = 0.5 * (tanh_safe(s * (r + 1.0)) - tanh_safe(s * (r - 1.0))) / tanh_safe(s);
    return -m.x * f;
}

// L(qt, G(q)) with G = diag(-1, 0, 1, 1) + K K^T, K = (g_xt, 1, 0, 0).
fn lagrangian(k: f32, v: vec4<f32>) -> f32 {
    return (k * k - 1.0) * v.x * v.x + 2.0 * k * v.x * v.y + v.y * v.y + v.z * v.z + v.w * v.w;
}

// G(q) v.
fn lower(k: f32, v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>((k * k - 1.0) * v.x + k * v.y, k * v.x + v.y, v.z, v.w);
}

// Ginv(q) p: diag(0, 1, 1, 1) - K' K'^T, K' = (1, -g_xt, 0, 0).
fn raise(k: f32, p: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(-p.x + k * p.y, k * p.x + (1.0 - k * k) * p.y, p.z, p.w);
}

struct Traced {
    dir: vec3<f32>,
    // SE's passParam: the length of the final momentum (1 for an unbent ray).
    pass_param: f32,
}

// TraceGeodesic for ALCUBIERRE_METRIC: `ro`, `rd` in ray units, bubble frame.
fn trace(m: vec4<f32>, ro_in: vec3<f32>, rd_in: vec3<f32>) -> Traced {
    var res: Traced;
    res.dir = rd_in;
    res.pass_param = 1.0;
    var ro = ro_in;
    let rd = rd_in;
    // SphereIntersection(ro, rd, max_distance).
    let b = dot(ro, rd);
    let c = dot(ro, ro) - MAX_DISTANCE * MAX_DISTANCE;
    let h = b * b - c;
    if h < 0.0 {
        return res;
    }
    var t0 = vec2<f32>(-sqrt(h) - b, sqrt(h) - b);
    if t0.x < 0.0 && t0.y < 0.0 {
        return res;
    }
    if t0.x >= 0.0 {
        ro = ro + rd * t0.x;
    }
    // CameraToGeodesic: start near the bubble, time 1 - |ro|.
    let start = length(ro);
    ro = ro + rd * max(start - 5.0, 0.0);
    var q = vec4<f32>(1.0 - length(ro), ro);
    var qt = 2.0 * vec4<f32>(1.0, rd);
    var p = normalize(lower(shift(m, q), qt));
    var minl = 1.0e10;
    var dist = 1.0e10;
    var dir = rd;
    let k_step = m.z * 0.5 + 0.4;
    for (var i = 0; i < MAX_STEPS; i = i + 1) {
        // GetTimeStep.
        let r = alcubierre_r(m, q);
        let dt = k_step * mix(0.01, 20.0, abs(r - 1.0) / MAX_DISTANCE);
        // HamiltonianGradient: qt = Ginv p, then dL/dq by forward differences.
        let k0 = shift(m, q);
        qt = raise(k0, p);
        let h0 = lagrangian(k0, qt);
        let delta = EPS * max(length(q.yzw), 1.0e-4);
        let grad = vec4<f32>(
            lagrangian(shift(m, q + vec4<f32>(delta, 0.0, 0.0, 0.0)), qt),
            lagrangian(shift(m, q + vec4<f32>(0.0, delta, 0.0, 0.0)), qt),
            lagrangian(shift(m, q + vec4<f32>(0.0, 0.0, delta, 0.0)), qt),
            lagrangian(shift(m, q + vec4<f32>(0.0, 0.0, 0.0, delta)), qt)) - vec4<f32>(h0);
        p = p + grad / delta * dt;
        q = q + 2.0 * qt * dt;
        // GeodesicToCamera.
        dir = normalize(qt.yzw);
        dist = length(q.yzw);
        minl = min(minl, dist);
        if dist >= MAX_DISTANCE {
            break;
        }
    }
    // Avoid a sharp transition.
    res.dir = normalize(mix(rd_in, dir, sstep(MAX_DISTANCE, 0.0, minl)));
    res.pass_param = length(p);
    return res;
}

fn view_dir(uv: vec2<f32>, tan_xy: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return normalize(vec3<f32>(ndc.x * tan_xy.x, ndc.y * tan_xy.y, -1.0));
}

fn bubble_rot(w: Warp) -> mat3x3<f32> {
    return mat3x3<f32>(w.bub0.xyz, w.bub1.xyz, w.bub2.xyz);
}

// Alpha of SE's impostor billboard (SamplePrecomputed): the render sphere's disc, edges faded.
fn impostor_alpha(w: Warp, d_b: vec3<f32>) -> f32 {
    let e = length(w.eye.xyz);
    let rs = w.metric.w;
    if e <= rs {
        return 1.0;
    }
    let to_c = -w.eye.xyz / e;
    let cosp = dot(d_b, to_c);
    if cosp <= 0.0 {
        return 0.0;
    }
    let tan_p = sqrt(max(1.0 - cosp * cosp, 0.0)) / cosp;
    let tan_s = rs / sqrt(e * e - rs * rs);
    let r = tan_p / tan_s;
    return 1.0 - smoothstep(0.95, 0.99, r * r);
}

@fragment
fn compute(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let w = warp;
    let d0 = view_dir(in.uv, w.view.xy);
    let rot = bubble_rot(w);
    let d_b = rot * d0;
    if impostor_alpha(w, d_b) <= 0.0 {
        return vec4<f32>(d0, 1.0);
    }
    let t = trace(w.metric, w.eye.xyz, d_b);
    return vec4<f32>(transpose(rot) * t.dir, t.pass_param);
}

// Environment map lookup in direction `v` (world axes), as `sample_env` in blackhole_lens.wesl.
fn face_axes(f: i32) -> array<vec3<f32>, 2> {
    switch f {
        case 0: { return array<vec3<f32>, 2>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0)); }
        case 1: { return array<vec3<f32>, 2>(vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0)); }
        case 2: { return array<vec3<f32>, 2>(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, -1.0)); }
        case 3: { return array<vec3<f32>, 2>(vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0)); }
        case 4: { return array<vec3<f32>, 2>(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0)); }
        default: { return array<vec3<f32>, 2>(vec3<f32>(0.0, 0.0, -1.0), vec3<f32>(0.0, 1.0, 0.0)); }
    }
}

fn sample_env(v: vec3<f32>) -> vec3<f32> {
    let a = abs(v);
    var f = 4;
    if a.x >= a.y && a.x >= a.z {
        f = select(1, 0, v.x > 0.0);
    } else if a.y >= a.z {
        f = select(3, 2, v.y > 0.0);
    } else {
        f = select(5, 4, v.z > 0.0);
    }
    let ax = face_axes(f);
    let rt = cross(ax[0], ax[1]);
    let k = 1.0 / dot(v, ax[0]);
    let uv = vec2<f32>(0.5 + 0.5 * dot(v, rt) * k, 0.5 - 0.5 * dot(v, ax[1]) * k);
    switch f {
        case 0: { return textureSampleLevel(env_px, lin, uv, 0.0).rgb; }
        case 1: { return textureSampleLevel(env_nx, lin, uv, 0.0).rgb; }
        case 2: { return textureSampleLevel(env_py, lin, uv, 0.0).rgb; }
        case 3: { return textureSampleLevel(env_ny, lin, uv, 0.0).rgb; }
        case 4: { return textureSampleLevel(env_pz, lin, uv, 0.0).rgb; }
        default: { return textureSampleLevel(env_nz, lin, uv, 0.0).rgb; }
    }
}

// colorTemperatureToRGB (einstein.glsl), valid from 1000 to 40000 K.
fn color_temperature(t: f32) -> vec3<f32> {
    var m0 = vec3<f32>(0.0, -2902.1955373783176, -8257.7997278925690);
    var m1 = vec3<f32>(0.0, 1669.5803561666639, 2575.2827530017594);
    var m2 = vec3<f32>(1.0, 1.3302673723350029, 1.8993753891711275);
    if t > 6500.0 {
        m0 = vec3<f32>(1745.0425298314172, 1216.6168361476490, -8257.7997278925690);
        m1 = vec3<f32>(-2666.3474220535695, -2173.1012343082230, 2575.2827530017594);
        m2 = vec3<f32>(0.55995389139931482, 0.70381203140554553, 1.8993753891711275);
    }
    let c = clamp(m0 / (vec3<f32>(clamp(t, 1000.0, 40000.0)) + m1) + m2, vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(c, vec3<f32>(1.0), sstep(1000.0, 0.0, t));
}

// Screen position of a view-space direction; `on` is false behind the camera.
struct Screen {
    uv: vec2<f32>,
    on: bool,
}

fn to_screen(w: Warp, d: vec3<f32>) -> Screen {
    var s: Screen;
    s.on = d.z < -1.0e-4;
    let z = max(-d.z, 1.0e-4);
    let ndc = vec2<f32>(d.x / z / w.view.x, d.y / z / w.view.y);
    s.uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return s;
}

struct Sampled {
    original: vec4<f32>,
    // Bent direction (view space) and passParam.
    dir: vec3<f32>,
    pass_param: f32,
    d0: vec3<f32>,
    alpha: f32,
    // Where the bent ray lands and where the ship's image is read (ShipUV).
    land: Screen,
    ship_uv: vec2<f32>,
}

// SamplePrecomputed and the screen positions Skybox() reads the frame and the depth at.
fn prepare(px: vec2<f32>, uv: vec2<f32>) -> Sampled {
    let w = warp;
    var s: Sampled;
    s.original = textureLoad(scene, vec2<i32>(px), 0);
    s.d0 = view_dir(uv, w.view.xy);
    s.alpha = impostor_alpha(w, bubble_rot(w) * s.d0);
    let wm = textureSampleLevel(warp_map, lin, uv, 0.0);
    s.dir = normalize(wm.xyz);
    s.pass_param = wm.w;
    s.land = to_screen(w, s.dir);
    // The ship's own image is bent too when the camera is outside the bubble wall.
    let warp_factor = smoothstep(0.8, 1.2, w.eye.w * 2.0);
    var land_uv = s.land.uv;
    if !s.land.on {
        land_uv = uv;
    }
    s.ship_uv = mix(uv, land_uv, warp_factor);
    return s;
}

// Skybox() for WARP with ENV_MAP. `geo_u`: the ship's image (ShipUV) is geometry; `geo_w`: the
// bent ray lands on geometry (SE: scene depth nearer than 1e25).
fn shade(s: Sampled, geo_u: bool, geo_w: bool, dims: vec2<f32>) -> vec4<f32> {
    let w = warp;
    if s.alpha <= 0.0 {
        return s.original;
    }
    let deflection = dot(s.dir - s.d0, s.dir - s.d0);
    let env_ok = w.view.z > 0.5;
    let ev = s.dir.x * w.sky0.xyz + s.dir.y * w.sky1.xyz + s.dir.z * w.sky2.xyz;
    var env = vec3<f32>(0.0);
    if env_ok {
        env = sample_env(ev);
    }
    // Frame where the bent ray lands, the environment map at and beyond the screen edges.
    var frame = vec3<f32>(0.0);
    var edge = 0.0;
    if s.land.on {
        let zone = mix(0.001, 0.15, linearstep(0.0, 0.01, deflection));
        let dd = (s.land.uv - clamp(s.land.uv, vec2<f32>(zone), vec2<f32>(1.0 - zone))) / zone;
        edge = clamp(1.0 - sqrt(dot(dd, dd)), 0.0, 1.0);
        if edge > 0.0 {
            frame = textureSampleLevel(scene, lin, clamp(s.land.uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
        }
    }
    var blend = edge;
    if !env_ok {
        // No environment map yet: the frame alone, faded out beyond the edges.
        blend = 1.0;
        frame = frame * edge;
    }
    var colour = mix(env, frame, blend);
    if geo_w && env_ok {
        colour = env;
    }
    // Geodesic redshift.
    let pp = max(s.pass_param, 0.1);
    colour = pow(pp, -3.0) * colour * pow(color_temperature(6700.0 * pow(pp, -0.15)) / color_temperature(6700.0), vec3<f32>(10.0));
    if geo_u {
        // The ship's image, transparent near the edges of the screen texture, but not when the
        // camera is inside the bubble.
        let inside = linearstep(0.5, 0.4, w.eye.w);
        let ship = textureSampleLevel(scene, lin, s.ship_uv, 0.0).rgb;
        colour = mix(colour, ship, mix(edge, 1.0, inside));
    }
    if any(colour != colour) {
        colour = vec3<f32>(0.0);
    }
    colour = min(colour, vec3<f32>(60000.0));
    return vec4<f32>(mix(s.original.rgb, colour, s.alpha), s.original.a);
}

fn texel(uv: vec2<f32>, dims: vec2<f32>) -> vec2<i32> {
    return clamp(vec2<i32>(uv * dims), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
}

// SceneDepth(uv, radius) > 0 in the port's reversed-Z depth: geometry. Radius 1 takes the
// farthest of the 3x3 texels, as SE's max of its forward depth.
fn geometry(uv: vec2<f32>, dims: vec2<f32>, radius: i32) -> bool {
    let c = texel(uv, dims);
    var d = 1.0;
    for (var j = -radius; j <= radius; j = j + 1) {
        for (var i = -radius; i <= radius; i = i + 1) {
            let p = clamp(c + vec2<i32>(i, j), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
            d = min(d, textureLoad(depth, p, 0));
        }
    }
    return d > 0.0;
}

fn geometry_ms(uv: vec2<f32>, dims: vec2<f32>, radius: i32) -> bool {
    let c = texel(uv, dims);
    var d = 1.0;
    for (var j = -radius; j <= radius; j = j + 1) {
        for (var i = -radius; i <= radius; i = i + 1) {
            let p = clamp(c + vec2<i32>(i, j), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
            d = min(d, textureLoad(depth_ms, p, 0));
        }
    }
    return d > 0.0;
}

@fragment
fn render(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(scene));
    let s = prepare(in.position.xy, in.uv);
    if s.alpha <= 0.0 {
        return s.original;
    }
    return shade(s, geometry(s.ship_uv, dims, 0), s.land.on && geometry(s.land.uv, dims, 1), dims);
}

@fragment
fn render_ms(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(scene));
    let s = prepare(in.position.xy, in.uv);
    if s.alpha <= 0.0 {
        return s.original;
    }
    return shade(s, geometry_ms(s.ship_uv, dims, 0), s.land.on && geometry_ms(s.land.uv, dims, 1), dims);
}

// Copies the shaded scratch texture (bound as `scene`) back into the main texture.
@fragment
fn blit(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return textureLoad(scene, vec2<i32>(in.position.xy), 0);
}
