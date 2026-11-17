// Hyperspace's tunnel in the scene (sidewinder/tunnel.rs `TunnelMaterial`; ARCHITECTURE.md A6, T4e-3): a sphere round the camera,
// turned with the ship, drawn in the main pass, opaque, at a depth just behind the seat's slice (`TunnelGpu::look` x), so the
// cockpit, its chair, the canopy's glass and the HUD stay in front of it and nothing of the universe shows through it.
//
// Its look is the stand-in the HUD drew flat until T4e-3 (ours, F: Elite's tunnel isn't built), by the same rule, in the old
// layout's units (screen heights from the view's middle, y down) on the plane of the ship's nose at the configured seat view
// (`seat::seat_vertical_fov`): with the head straight ahead it lands on the screen where the flat one did; turned, it stays with
// the ship. On a dark blue (4, 6, 16): 160 streaks, each along a fixed direction out from the nose, its place along it running
// outwards at 0.6 a second from its own phase (squared: slow in the middle, fast at the edge), longer, wider and more opaque
// further out; a faint disc on the nose. Composited as the HUD's painter did (each shape's colour times its alpha and coverage
// in sRGB, then over in linear light), giving the colour the screen showed.
//
// That colour reaches the screen whatever the exposure: the engine's colour correction and tone curve (color_correction.wesl)
// are undone for it (its `final_image` exactly, with the camera's own values; the tone curve only for SE's default AgX, the
// inverse of `tone_agx` below, which must change with it), and the result is divided by the exposure the frame is drawn with:
// 2^(the camera's colour grading exposure) times 1 + the bloom's gain and, in photo mode 2, 2^-(the automatic exposure's adapted compensation), read from
// the ship's own copy of it as its lamps read it (material.rs `ShipScene`, sidewinder_lit.wesl `adapted`).

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::view;

struct TunnelGpu {
    // x 1 while a jump shows it (0: the warm-up's draw, no pixel), y the jump's seconds, z the old layout's reach (half the
    // screen's diagonal in heights), w 2 tan(half the configured vertical field of view).
    clock: vec4<f32>,
    // x the shell's depth (reverse z), y the tone shader (`color_correction::TONE_SHADERS` index; 1 the default AgX), z the
    // bloom's gain (bloom.rs `SeBloom`: BloomBright, 0 with the bloom off).
    look: vec4<f32>,
    // The camera's colour correction (`ColorCorrection::bcsg`: x brightness - 1, y contrast, z saturation - 1, w 1 / gamma).
    bcsg: vec4<f32>,
};

// The head of `ShipScene`'s record (sidewinder_lit.wesl `ShipScene`): its exposure inputs, then x 1 when the adapted
// compensation applies (photo mode 2).
struct ShipSceneHead {
    inputs: vec4<f32>,
    adapt_on: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> tunnel: TunnelGpu;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> ship_scene: ShipSceneHead;
// The automatic exposure's state as the previous frame left it (x the adapted compensation in stops, positive darker).
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<storage, read> ship_adapt: vec4<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The direction from the eye in the ship's space (x right, y up, -z the nose).
    @location(0) dir: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    var o: VertexOut;
    o.dir = v.position;
    if tunnel.clock.x < 0.5 {
        // (The warm-up: every vertex outside the depth range, so the pipeline is built and nothing is drawn.)
        o.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return o;
    }
    let world = mesh_position_local_to_world(get_world_from_local(v.instance_index), vec4<f32>(v.position, 1.0));
    o.clip = position_world_to_clip(world.xyz);
    o.clip.z = tunnel.look.x * o.clip.w;
    return o;
}

fn linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// One shape over what is under it, as the HUD's painter laid it: its sRGB colour `c` times `a` (alpha x coverage), in linear light.
fn over(under: vec3<f32>, c: vec3<f32>, a: f32) -> vec3<f32> {
    return linear(c * a) + under * (1.0 - a);
}

// The old flat tunnel's colour on the screen (linear) at `s` (screen heights from the middle, y down), `px` one pixel in heights.
fn stand_in(s: vec2<f32>, px: f32) -> vec3<f32> {
    let seconds = tunnel.clock.y;
    let most = tunnel.clock.z;
    var c = linear(vec3<f32>(4.0, 6.0, 16.0) / 255.0);
    let streak = vec3<f32>(150.0, 190.0, 255.0) / 255.0;
    for (var i = 0u; i < 160u; i++) {
        let k = f32((i * 2654435761u) >> 8u) / 16777216.0;
        let a = k * 6.2831855;
        let phase = f32(((i * 40503u) >> 4u) % 1000u) / 1000.0;
        let f = pow(fract(seconds * 0.6 + phase), 2.0);
        let dir = vec2<f32>(cos(a), sin(a));
        let r0 = f * most;
        let r1 = (f * 1.25 + 0.02) * most;
        // (Distances in pixels: along the streak past its ends, and across it.)
        let along = dot(s, dir);
        let past = max(r0 - along, along - r1) / px;
        let across = abs(s.x * dir.y - s.y * dir.x) / px;
        let width = (0.001 + 0.004 * f) / px;
        // (The painter's feathering: a pixel's fade across each edge and end; a line under a pixel wide is a pixel wide, fainter.)
        let cover = clamp(0.5 * max(width, 1.0) + 0.5 - across, 0.0, 1.0) * clamp(0.5 - past, 0.0, 1.0) * min(width, 1.0);
        if cover > 0.0 {
            c = over(c, streak, floor(40.0 + 215.0 * f) / 255.0 * cover);
        }
    }
    let disc = clamp(0.04 / px + 0.5 - length(s) / px, 0.0, 1.0);
    return over(c, vec3<f32>(200.0, 220.0, 255.0) / 255.0, 90.0 / 255.0 * disc);
}

// The AgX tone curve's polynomial (color_correction.wesl `tone_agx`).
fn agx_curve(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn inverse3(m: mat3x3<f32>) -> mat3x3<f32> {
    let a = m[0];
    let b = m[1];
    let c = m[2];
    let r0 = cross(b, c);
    let r1 = cross(c, a);
    let r2 = cross(a, b);
    return transpose(mat3x3<f32>(r0, r1, r2)) * (1.0 / dot(a, r0));
}

// What `tone_agx(v, false)` turns into `out` (linear, 0..1): its matrices and log range as there, its polynomial (rising over
// 0..1) solved by halving.
fn agx_inverse(out: vec3<f32>) -> vec3<f32> {
    let agx_mat = mat3x3<f32>(
        0.842479062253094, 0.0423282422610123, 0.0423756549057051,
        0.0784335999999992, 0.878468636469772, 0.0784336,
        0.0792237451477643, 0.0791661274605434, 0.879142973793104,
    );
    let agx_inv = mat3x3<f32>(
        1.19687900512017, -0.0528968517574562, -0.0529716355144438,
        -0.0980208811401368, 1.15190312990417, -0.0980434501171241,
        -0.0990297440797205, -0.0989611768448433, 1.15107367264116,
    );
    let min_ev = -12.47393;
    let max_ev = log2(16.2917);
    let want = inverse3(agx_inv) * pow(max(out, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    var lo = vec3<f32>(0.0);
    var hi = vec3<f32>(1.0);
    for (var i = 0; i < 20; i++) {
        let mid = 0.5 * (lo + hi);
        let up = agx_curve(mid) < want;
        lo = select(lo, mid, up);
        hi = select(mid, hi, up);
    }
    let ev = 0.5 * (lo + hi) * (max_ev - min_ev) + min_ev;
    return max(inverse3(agx_mat) * exp2(ev) / 0.75, vec3<f32>(0.0));
}

// What the frame must hold (before the exposure) for the screen to show `out` (linear): color_correction.wesl's `final_image`
// undone (gamma, vibrance, contrast and brightness; its dithering and sharpening left), then its tone curve (AgX only).
fn undo_grading(out: vec3<f32>) -> vec3<f32> {
    let g = tunnel.bcsg;
    var c = pow(max(out, vec3<f32>(1e-6)), vec3<f32>(1.0 / g.w));
    // (The vibrance keeps the luma and scales the saturation s by 1 + z (1 - s): solved for s.)
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sat = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
    var s = sat;
    if abs(g.z) > 1e-4 {
        s = ((1.0 + g.z) - sqrt(max((1.0 + g.z) * (1.0 + g.z) - 4.0 * g.z * sat, 0.0))) / (2.0 * g.z);
    }
    c = luma + (c - luma) / (1.0 + g.z * (1.0 - s));
    c = (c - g.x - 0.5) / g.y + 0.5;
    if u32(tunnel.look.y) == 1u {
        return agx_inverse(c);
    }
    return max(c, vec3<f32>(0.0));
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let d = normalize(in.dir);
    // (On the plane of the nose, in the old layout's screen heights; a pixel from this pixel's own footprint, taken before the
    // branch: derivatives need every pixel of the quad. Behind the nose's plane, the dark blue alone.)
    let ahead = -d.z;
    let s = vec2<f32>(d.x, -d.y) / (max(ahead, 1e-3) * tunnel.clock.w);
    let px = max(0.5 * (length(dpdx(s)) + length(dpdy(s))), 1e-6);
    var out = linear(vec3<f32>(4.0, 6.0, 16.0) / 255.0);
    if ahead > 1e-3 {
        out = stand_in(s, px);
    }
    let adapted = ship_scene.adapt_on.x * ship_adapt.x;
    let exposure = exp2(view.color_grading.exposure - adapted);
    // (SE's bloom adds its gain times the blurred frame, threshold 0: an even field comes out 1 + gain times as bright, so that
    // is taken off. Round a streak its glow still spreads: no picture-wide pass can be undone here.)
    return vec4<f32>(undo_grading(out) / (exposure * (1.0 + tunnel.look.z)), 1.0);
}
