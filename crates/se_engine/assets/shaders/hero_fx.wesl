// The Viltrumite mod's effects in the air (viltrumite/fx.rs, spec F9): the plasma shell, the sonic cone, the rings, the trail. One
// additive pass after the clouds and before the solids' upscale, drawn into the solids' corner of the main texture, with no depth
// attachment and no vertex buffers: every vertex comes from `fx`, one storage buffer written in place, every draw is one instance range
// (`FX_*`). `clip.z = 0.5 w`: nothing is cut by the near plane, which he pulls in as it is; what's behind the camera still is.
//
// They read the scene's depth for a soft edge against the ground and against him, and the clouds' own buffers: a point beyond the
// clouds' distance on its ray is dimmed by their transmittance. (Drawn in the main pass, every cloud on the ray would dim them, even
// clouds behind them.) Fallback textures stand in where the clouds didn't draw: distance and transmittance 1, which dims nothing.
//
// Light is in the ground's scale, as his own (hero.wesl): the cone and the rings take `hero_light`, his own light of this frame.
//
// Step 8: its dust is a new kind of draw here (`FX_DUST`), and the craters a pass of their own before the clouds; their records wait in
// `FxRecords`.

import bevy_render::view::View;
import super::hero_frame::HeroLight;

const PI: f32 = 3.14159265358979;

const SHELL_RINGS: u32 = 16;
const AROUND: u32 = 32;
const CONE_BANDS: u32 = 6;

// Which draw an instance is: viltrumite/fx.rs `FX_*`.
const FX_SHELL: u32 = 0;
const FX_CONE: u32 = 1;
const FX_TRAIL: u32 = 2;
const FX_RING: u32 = 3;

struct FxRing {
    // xyz its middle from the camera, w its radius (m).
    centre: vec4<f32>,
    // xyz its plane's normal, w how far the band reaches either side (m).
    normal: vec4<f32>,
    // rgb its own light on top of what it scatters (night), w its strength (0: none).
    tint: vec4<f32>,
}

struct FxPoint {
    // xyz from the camera, w half its width (m), negative where no segment goes from here to the next point.
    at: vec4<f32>,
    // rgb its light, a how far along its life (0 new).
    light: vec4<f32>,
}

struct FxRecords {
    // xyz his middle from the camera, w the clock (s, wrapped).
    him: vec4<f32>,
    // xyz the way he's going through the air, w his speed through it (m/s).
    along: vec4<f32>,
    // rgb the shell's light, w 0 when there's none.
    shell: vec4<f32>,
    // x its radius, y its length, z its nose ahead of his middle (m).
    shell_size: vec4<f32>,
    // x the half-angle (radians), y the slant's length (m), z how strong, w his nose ahead of his middle (m).
    cone: vec4<f32>,
    // The far cloud buffers: x the resolved texels' size in pixels, y the trace's; the near pass's: z, w.
    cloud_px: vec4<f32>,
    // xyz towards the star (his own light's).
    sun: vec4<f32>,
    rings: array<FxRing, 4>,
    trail: array<FxPoint, 256>,
    // Step 8.
    craters: array<array<vec4<f32>, 2>, 32>,
    dust: array<array<vec4<f32>, 2>, 64>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> fx: FxRecords;
@group(0) @binding(2) var<storage, read> hero_light: HeroLight;
@if(MSAA) @group(0) @binding(3) var scene_depth: texture_depth_multisampled_2d;
@if(NO_MSAA) @group(0) @binding(3) var scene_depth: texture_depth_2d;
@group(0) @binding(4) var far_clouds: texture_2d<f32>;
@group(0) @binding(5) var far_dist: texture_2d<f32>;
@group(0) @binding(6) var near_clouds: texture_2d<f32>;
@group(0) @binding(7) var near_dist: texture_2d<f32>;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // From the camera (m).
    @location(0) rel: vec3<f32>,
    // Across and along, each draw's own.
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) light: vec4<f32>,
    @location(4) @interpolate(flat) kind: u32,
    @location(5) @interpolate(flat) ring: u32,
}

fn to_clip(rel: vec3<f32>) -> vec4<f32> {
    var c = view.clip_from_world * vec4<f32>(rel, 1.0);
    c.z = 0.5 * c.w;
    return c;
}

// A quad's corner of the six vertices of `vi`: x, y each 0 or 1.
fn corner(vi: u32) -> vec2<f32> {
    let c = vi % 6u;
    return vec2<f32>(select(0.0, 1.0, c == 1u || c == 4u || c == 5u), select(0.0, 1.0, c == 2u || c == 3u || c == 5u));
}

// Two axes across `a`.
fn across(a: vec3<f32>) -> array<vec3<f32>, 2> {
    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(a.y) > 0.9);
    let e1 = normalize(cross(a, helper));
    return array<vec3<f32>, 2>(e1, cross(a, e1));
}

fn shell_vertex(vi: u32) -> VertexOut {
    let q = vi / 6u;
    let k = corner(vi);
    let u = (f32(q % AROUND) + k.x) / f32(AROUND) * 2.0 * PI;
    let s = (f32(q / AROUND) + k.y) / f32(SHELL_RINGS);
    let a = fx.along.xyz;
    let e = across(a);
    let out_dir = e[0] * cos(u) + e[1] * sin(u);
    let radius = fx.shell_size.x;
    let length_m = fx.shell_size.y;
    // A paraboloid round him, its nose ahead of him.
    let rel = fx.him.xyz + a * (fx.shell_size.z - length_m * s * s) + out_dir * radius * s;
    var o: VertexOut;
    o.clip = to_clip(rel);
    o.rel = rel;
    o.uv = vec2<f32>(u, s);
    o.normal = normalize(a * radius + out_dir * 2.0 * length_m * s);
    o.light = fx.shell;
    o.kind = FX_SHELL;
    return o;
}

fn cone_vertex(vi: u32) -> VertexOut {
    let q = vi / 6u;
    let k = corner(vi);
    let u = (f32(q % AROUND) + k.x) / f32(AROUND) * 2.0 * PI;
    let s = (f32(q / AROUND) + k.y) / f32(CONE_BANDS);
    let a = fx.along.xyz;
    let e = across(a);
    let out_dir = e[0] * cos(u) + e[1] * sin(u);
    let t = fx.cone.y * s;
    let angle = fx.cone.x;
    let rel = fx.him.xyz + a * fx.cone.w + (out_dir * sin(angle) - a * cos(angle)) * t;
    var o: VertexOut;
    o.clip = to_clip(rel);
    o.rel = rel;
    o.uv = vec2<f32>(u, s);
    o.normal = a * sin(angle) + out_dir * cos(angle);
    o.light = vec4<f32>(fx.cone.z);
    o.kind = FX_CONE;
    return o;
}

fn ring_vertex(vi: u32, index: u32) -> VertexOut {
    let r = fx.rings[index];
    let k = corner(vi) * 2.0 - 1.0;
    let e = across(r.normal.xyz);
    let reach = r.centre.w + 3.0 * r.normal.w;
    let rel = r.centre.xyz + (e[0] * k.x + e[1] * k.y) * reach;
    var o: VertexOut;
    o.clip = to_clip(rel);
    o.rel = rel;
    o.uv = k * reach;
    o.normal = r.normal.xyz;
    o.light = r.tint;
    o.kind = FX_RING;
    o.ring = index;
    return o;
}

fn trail_vertex(vi: u32) -> VertexOut {
    let seg = vi / 6u;
    let k = corner(vi);
    let i = seg + u32(k.y);
    let p = fx.trail[i];
    // The ribbon faces the camera, along the path at this point.
    let before = fx.trail[seg].at.xyz;
    let after = fx.trail[seg + 1u].at.xyz;
    let side = normalize(cross(after - before + vec3<f32>(1e-3, 0.0, 0.0), p.at.xyz));
    let rel = p.at.xyz + side * abs(p.at.w) * (k.x * 2.0 - 1.0);
    var o: VertexOut;
    // (No segment here: a zero-area triangle.)
    o.clip = select(to_clip(rel), vec4<f32>(0.0, 0.0, 0.0, 1.0), fx.trail[seg].at.w < 0.0);
    o.rel = rel;
    o.uv = vec2<f32>(k.x * 2.0 - 1.0, p.light.a);
    o.light = vec4<f32>(p.light.rgb, 1.0);
    o.kind = FX_TRAIL;
    return o;
}

// instance_index picks the draw.
@vertex
fn vertex(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VertexOut {
    if ii == FX_SHELL {
        return shell_vertex(vi);
    }
    if ii == FX_CONE {
        return cone_vertex(vi);
    }
    if ii == FX_TRAIL {
        return trail_vertex(vi);
    }
    return ring_vertex(vi, ii - FX_RING);
}

fn hash12(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

// Value noise, x wrapping at `wrap`.
fn noise2(p: vec2<f32>, wrap: f32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let w = f * f * (3.0 - 2.0 * f);
    let x0 = i.x % wrap;
    let x1 = (i.x + 1.0) % wrap;
    let lo = mix(hash12(vec2<f32>(x0, i.y)), hash12(vec2<f32>(x1, i.y)), w.x);
    let hi = mix(hash12(vec2<f32>(x0, i.y + 1.0)), hash12(vec2<f32>(x1, i.y + 1.0)), w.x);
    return mix(lo, hi, w.y);
}

// A colour's brightest channel held to `most`, keeping the colour.
fn limited(c: vec3<f32>, most: f32) -> vec3<f32> {
    return c * min(most / max(max(c.r, max(c.g, c.b)), 1e-6), 1.0);
}

// How much of the point's light gets past the scene: a soft edge against what's in front of it. Reversed depth, infinite far plane.
fn soft_depth(pix: vec2<i32>, w: f32, soft_m: f32) -> f32 {
    let d = textureLoad(scene_depth, pix, 0);
    if d <= 0.0 {
        return 1.0;
    }
    return clamp((view.clip_from_view[3][2] / d - w) / soft_m, 0.0, 1.0);
}

fn dim_by(clouds: texture_2d<f32>, dist: texture_2d<f32>, px: vec2<f32>, pos: vec2<f32>, d: f32) -> f32 {
    let last_d = vec2<i32>(textureDimensions(dist)) - 1;
    let cloud_d = textureLoad(dist, min(vec2<i32>(pos / px.y), last_d), 0).r;
    let last_c = vec2<i32>(textureDimensions(clouds)) - 1;
    let tr = textureLoad(clouds, min(vec2<i32>(pos / px.x), last_c), 0).a;
    return mix(1.0, clamp(tr, 0.0, 1.0), smoothstep(0.9 * cloud_d, 1.1 * cloud_d, d));
}

// What the clouds leave of a point this far along its ray: the near pass's, then the far pass's.
fn cloud_dim(pos: vec2<f32>, d: f32) -> f32 {
    return dim_by(far_clouds, far_dist, fx.cloud_px.xy, pos, d) * dim_by(near_clouds, near_dist, fx.cloud_px.zw, pos, d);
}

// A thin cloud of droplets in his own light: from the star whichever side it's on, more of it looking towards the star.
fn droplets_light(rel: vec3<f32>) -> vec3<f32> {
    let towards = max(dot(normalize(rel), fx.sun.xyz), 0.0);
    let phase = 0.6 + 2.0 * pow(towards, 6.0);
    return (hero_light.sun.rgb * phase + hero_light.sky.rgb + hero_light.flat_e.rgb) / PI;
}

// Glowing gas, or a thin sheet of droplets: seen at a slant the eye looks through more of it, as far as its thickness goes.
fn slant(in: VertexOut) -> f32 {
    return 1.0 / (abs(dot(normalize(in.normal), normalize(in.rel))) + 0.35);
}

fn shell_light(in: VertexOut) -> vec3<f32> {
    let s = in.uv.y;
    // Gas, no surface: it thins out towards the edge of what's seen, so no line shows there.
    let seen = smoothstep(0.0, 0.55, abs(dot(normalize(in.normal), normalize(in.rel))));
    // Hottest at the nose, flames going back along him, faster than the eye follows.
    let around = in.uv.x * 30.0 / (2.0 * PI);
    let flames = noise2(vec2<f32>(around, s * s * 2.0 - fx.him.w * 5.0), 30.0) * noise2(vec2<f32>(around * 2.0 + 3.0, s * s * 3.5 - fx.him.w * 7.0), 60.0);
    let tongues = mix(1.0, 0.45 + 2.2 * flames, smoothstep(0.05, 0.5, s));
    let lit = in.light.rgb * pow(1.0 - s, 2.0) * seen * tongues * 2.0;
    return limited(lit, 15.0) * smoothstep(0.3, 1.2, 1.0 / in.clip.w);
}

fn cone_light(in: VertexOut) -> vec3<f32> {
    let s = in.uv.y;
    // A cloud with a sharp front, thinning out behind.
    let wisps = noise2(vec2<f32>(in.uv.x * 24.0 / (2.0 * PI), s * 3.0 - fx.him.w * 2.0), 24.0);
    let k = pow(1.0 - s, 1.5) * smoothstep(0.03, 0.16, s) * (0.35 + 1.3 * wisps);
    let through = 1.0 / (abs(dot(normalize(in.normal), normalize(in.rel))) + 0.6);
    return limited(droplets_light(in.rel) * in.light.x * k * through, 3.0) * smoothstep(0.5, 2.0, 1.0 / in.clip.w);
}

fn ring_light(in: VertexOut) -> vec3<f32> {
    let r = fx.rings[in.ring];
    let angle = atan2(in.uv.y, in.uv.x);
    let frayed = noise2(vec2<f32>((angle + PI) * 24.0 / (2.0 * PI), r.centre.w * 0.08), 24.0);
    let off = (length(in.uv) - r.centre.w) / (r.normal.w * (0.45 + 0.55 * frayed));
    let lit = (droplets_light(in.rel) + in.light.rgb) * in.light.w * exp(-off * off) * slant(in);
    return limited(lit, 3.0) * smoothstep(1.0, 4.0, 1.0 / in.clip.w);
}

fn trail_light(in: VertexOut) -> vec3<f32> {
    let u = in.uv.x;
    let lit = in.light.rgb * exp(-u * u * 3.0);
    return limited(lit, 6.0) * smoothstep(1.5, 10.0, 1.0 / in.clip.w);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    var c: vec3<f32>;
    var soft: f32;
    if in.kind == FX_SHELL {
        c = shell_light(in);
        soft = 0.25;
    } else if in.kind == FX_CONE {
        c = cone_light(in);
        soft = 1.0;
    } else if in.kind == FX_RING {
        c = ring_light(in);
        soft = 1.0;
    } else {
        c = trail_light(in);
        soft = 2.0;
    }
    let w = 1.0 / in.clip.w;
    let k = soft_depth(vec2<i32>(in.clip.xy), w, soft) * cloud_dim(in.clip.xy, length(in.rel));
    return vec4<f32>(c * k, 0.0);
}
