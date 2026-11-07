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
// Impacts (step 8, viltrumite/impact.rs): the dust is a draw of its own here (`dust_vertex`, `dust_fragment`), blended over the scene,
// sprites from the smoke flipbook; the craters a pass of their own before the clouds (`decal_vertex`, `decal_fragment`), which multiplies
// what the main pass drew, so they take the ground's own light, night and cloud shadow, and follow whatever ground the depth says.

import bevy_render::view::View;
import super::hero_frame::HeroLight;

const PI: f32 = 3.14159265358979;

const SHELL_RINGS: u32 = 16;
const AROUND: u32 = 32;
const CONE_BANDS: u32 = 6;
// A trail segment seen within about 9 degrees of end-on is dimmed (the sine of that angle): from a chase camera 20 degrees off his line
// none of it is, looking back along the trail its many ribbons don't pile up at the point it runs to.
const END_ON: f32 = 0.15;

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

// A crater (viltrumite/fx.rs `FxCraterGpu`).
struct FxCrater {
    // xyz its middle on the ground as drawn now, from the camera, w its radius (m).
    at: vec4<f32>,
    // xyz the ground's normal there, w its seed (0..1).
    up: vec4<f32>,
    // x how far in (0..1), y the bowl's depth, z the lip's height, w how far out it reaches (all in radii).
    look: vec4<f32>,
    // xyz the sea's up there, w how high its middle stands over the sea (m; 1e9 on a body with none).
    sea: vec4<f32>,
}

// A burst of dust (viltrumite/fx.rs `FxDustGpu`).
struct FxDust {
    // xyz where it rose from, from the camera, w its age (s).
    at: vec4<f32>,
    // xyz up there, w its size (m).
    up: vec4<f32>,
    // rgb its colour (albedo), w its seed.
    look: vec4<f32>,
    // x its life (s), y gravity (m/s2), z the air's hold on it (s; 0 in vacuum), w its style: 0 a plume, 1 a ring along the ground,
    // 2 spray.
    motion: vec4<f32>,
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
    // His body as capsules round his bones, which the craters leave alone: x how many of `parts` (0: he isn't drawn).
    body: vec4<f32>,
    // The craters' burnt ground: x at the middle, y at the edge; z how high over the sea a point must stand to be ground (m).
    decal: vec4<f32>,
    // The capsules (viltrumite/impact.rs `BodyParts`): two points from the camera each (m), the first's w its radius.
    parts: array<vec4<f32>, 48>,
    rings: array<FxRing, 4>,
    trail: array<FxPoint, 256>,
    craters: array<FxCrater, 32>,
    dust: array<FxDust, 64>,
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
@group(0) @binding(8) var sprite: texture_2d<f32>;
@group(0) @binding(9) var sprite_smp: sampler;

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
    // Seen end-on, the ribbons of a straight trail all cover the point it runs to, and their light adds up there, far past F9's 6: each
    // fades as it turns end-on (full from END_ON's sine of the angle between it and the eye's ray on).
    let end_on = length(cross(normalize(after - before + vec3<f32>(1e-3, 0.0, 0.0)), normalize(p.at.xyz)));
    var o: VertexOut;
    // (No segment here: a zero-area triangle.)
    o.clip = select(to_clip(rel), vec4<f32>(0.0, 0.0, 0.0, 1.0), fx.trail[seg].at.w < 0.0);
    o.rel = rel;
    o.uv = vec2<f32>(k.x * 2.0 - 1.0, p.light.a);
    o.light = vec4<f32>(p.light.rgb * smoothstep(0.0, END_ON, end_on), 1.0);
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
    // (Clamped: interpolated, s can pass 1 by a rounding, and pow of a negative is NaN, which the additive blend would keep.)
    let thin = max(1.0 - s, 0.0);
    let lit = in.light.rgb * thin * thin * seen * tongues * 2.0;
    return limited(lit, 15.0) * smoothstep(0.3, 1.2, 1.0 / in.clip.w);
}

fn cone_light(in: VertexOut) -> vec3<f32> {
    let s = in.uv.y;
    // A cloud with a sharp front, thinning out behind.
    let wisps = noise2(vec2<f32>(in.uv.x * 24.0 / (2.0 * PI), s * 3.0 - fx.him.w * 2.0), 24.0);
    let k = pow(max(1.0 - s, 0.0), 1.5) * smoothstep(0.03, 0.16, s) * (0.35 + 1.3 * wisps);
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

// ---- The craters ----------------------------------------------------------------------------------------------------------------------

struct DecalOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
}

// How far a crater's box reaches either side of its ground (m): what's much higher (trees, a cliff behind) is left alone.
fn decal_tall(c: FxCrater) -> f32 {
    return 1.5 + 0.25 * c.at.w;
}

// The rectangle a crater's box covers on the screen (the whole screen when the camera is in it or beside it), each pixel once.
@vertex
fn decal_vertex(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> DecalOut {
    let c = fx.craters[ii];
    let n = normalize(c.up.xyz);
    let e = across(n);
    let reach = c.at.w * c.look.w;
    let tall = decal_tall(c);
    var lo = vec2<f32>(1e9);
    var hi = vec2<f32>(-1e9);
    var behind = 0u;
    for (var j = 0u; j < 8u; j++) {
        let k = vec3<f32>(select(-1.0, 1.0, (j & 1u) != 0u), select(-1.0, 1.0, (j & 2u) != 0u), select(-1.0, 1.0, (j & 4u) != 0u));
        let q = view.clip_from_world * vec4<f32>(c.at.xyz + (e[0] * k.x + e[1] * k.y) * reach + n * k.z * tall, 1.0);
        if q.w <= 1e-3 {
            behind += 1u;
        } else {
            lo = min(lo, q.xy / q.w);
            hi = max(hi, q.xy / q.w);
        }
    }
    if behind == 8u {
        lo = vec2<f32>(2.0);
        hi = vec2<f32>(2.0);
    } else if behind > 0u {
        lo = vec2<f32>(-1.0);
        hi = vec2<f32>(1.0);
    }
    lo = clamp(lo, vec2<f32>(-1.0), vec2<f32>(1.0));
    hi = clamp(hi, vec2<f32>(-1.0), vec2<f32>(1.0));
    var o: DecalOut;
    o.clip = vec4<f32>(mix(lo, hi, corner(vi)), 0.5, 1.0);
    o.index = ii;
    return o;
}

// Is this point on him? (He's drawn over the ground: the craters leave him alone.) Capsules round his bones, as he's posed this frame:
// the ground punch's fist, an arm on the ground, his back flying low over a crater.
fn in_him(p: vec3<f32>) -> bool {
    let n = min(u32(fx.body.x), 24u);
    if n == 0u || length(p - fx.him.xyz) > 3.0 {
        return false;
    }
    for (var i = 0u; i < n; i++) {
        let a = fx.parts[2u * i];
        let ab = fx.parts[2u * i + 1u].xyz - a.xyz;
        let t = clamp(dot(p - a.xyz, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
        if length(p - a.xyz - ab * t) < a.w {
            return true;
        }
    }
    return false;
}

// The light on a surface facing `n` on ground facing `up`, in his light: `lit_surface`'s terms without the albedo.
fn light_on(n: vec3<f32>, up: vec3<f32>) -> vec3<f32> {
    return hero_light.sun.rgb * max(dot(n, fx.sun.xyz), 0.0) + hero_light.sky.rgb * (0.5 + 0.5 * dot(n, up)) + hero_light.flat_e.rgb;
}

// What a crater multiplies the scene by here: the burnt ground and the thrown-out rays, and the bowl's and the lip's walls as the
// ratio of their light to the level ground's (so the ground's own light, night and cloud shadow stay in it).
@fragment
fn decal_fragment(in: DecalOut) -> @location(0) vec4<f32> {
    let none = vec4<f32>(1.0);
    let c = fx.craters[in.index];
    let d = textureLoad(scene_depth, vec2<i32>(in.clip.xy), 0);
    // The point the pixel sees, from the camera. (Through `main_pass_viewport`: under the render scale the pixels fill a corner.)
    let vp = view.main_pass_viewport;
    let ndc = vec2<f32>((in.clip.x - vp.x) / vp.z * 2.0 - 1.0, 1.0 - (in.clip.y - vp.y) / vp.w * 2.0);
    let wp = view.world_from_clip * vec4<f32>(ndc, max(d, 1e-12), 1.0);
    let p = wp.xyz / wp.w;
    // Which way what it sees faces, from its neighbours (before any return: derivatives need every pixel of the quad).
    let face = cross(dpdx(p), dpdy(p));
    if d <= 0.0 || in_him(p) {
        return none;
    }
    let n = normalize(c.up.xyz);
    // Only what lies like the ground: a wall, a car's side or a rock's face standing up is left alone.
    let lying = smoothstep(0.55, 0.8, abs(dot(face, n)) / max(length(face), 1e-12));
    let rel = p - c.at.xyz;
    // Not the sea: its flat surface would pass for ground, and the crater's burn and rays would lie on the water.
    if c.sea.w + dot(rel, c.sea.xyz) < fx.decal.z {
        return none;
    }
    let h = dot(rel, n);
    let flat = rel - n * h;
    let radius = c.at.w;
    let e = across(n);
    let around = (atan2(dot(flat, e[1]), dot(flat, e[0])) + PI) / (2.0 * PI);
    let seed = c.up.w * 97.0;
    // A ragged edge.
    let ragged = 1.0 + 0.16 * (noise2(vec2<f32>(around * 11.0, seed), 11.0) - 0.5) + 0.07 * (noise2(vec2<f32>(around * 37.0, seed + 7.0), 37.0) - 0.5);
    let r = length(flat) / (radius * ragged);
    let reach = c.look.w;
    let height_k = 1.0 - smoothstep(0.6 + 0.04 * radius, decal_tall(c), abs(h));
    if r >= reach || height_k <= 0.0 {
        return none;
    }
    // Burnt in the bowl, lighter on the lip, the thrown-out ground beyond it reaching out in rays, each ending raggedly.
    let grain = noise2(vec2<f32>(around * 64.0, r * 9.0 + seed), 64.0);
    let inner = mix(fx.decal.x, fx.decal.y, r * r) * (0.85 + 0.3 * grain);
    let rays = smoothstep(0.3, 0.85, noise2(vec2<f32>(around * 23.0, seed + 3.0), 23.0) * 0.75 + 0.25 * grain);
    let clumps = noise2(vec2<f32>(around * 57.0, r * 4.0 + seed + 11.0), 57.0);
    let ends = min(1.12 + (reach - 1.12) * (0.2 + 0.8 * rays) * (0.6 + 0.7 * clumps), reach * 0.97);
    let out_k = 1.0 - smoothstep(1.0, ends, r);
    let outer = mix(1.0, mix(0.8, 0.6, rays) * (0.9 + 0.2 * clumps), pow(out_k, 1.2));
    let lip_w = 0.12;
    // (A square, not `pow`: inside the lip its base is negative, and `pow` of a negative is undefined, NaN on some drivers.)
    let lip_x = (r - 1.02) / lip_w;
    let lip_band = exp(-lip_x * lip_x);
    let albedo = mix(mix(inner, outer, smoothstep(0.92, 1.1, r)), 0.9, lip_band * 0.3);
    // The walls: the ground's normal turned by the slope, the bowl's rising outward (its depth in radii), then the lip's.
    let slope = select(0.0, 2.0 * c.look.y * r, r < 1.0) - c.look.z * 2.0 * (r - 1.02) / (lip_w * lip_w) * lip_band;
    let out_dir = flat / max(length(flat), 1e-4);
    let wall = normalize(n - out_dir * slope);
    let ratio = min(light_on(wall, n) / max(light_on(n, n), vec3<f32>(1e-6)), vec3<f32>(2.0));
    let k = clamp(c.look.x, 0.0, 1.0) * height_k * lying;
    return vec4<f32>(mix(vec3<f32>(1.0), albedo * ratio, k), 1.0);
}

// ---- The dust -------------------------------------------------------------------------------------------------------------------------

// The lit ground on a ring round where a burst rose (`dust_tint`), and the seed of the burst it's for (another seed: not yet seen).
@group(0) @binding(10) var<storage, read> dust_tints: array<vec4<f32>, 64>;
// The same for the compute pass, and the scene it reads.
@group(0) @binding(11) var scene_colour: texture_2d<f32>;
@group(0) @binding(12) var<storage, read_write> dust_tint_out: array<vec4<f32>, 64>;

// A point's pixel in the solid's corner of the target, and whether the scene there is the point itself (nothing nearer in front).
fn seen_pixel(p: vec3<f32>) -> vec3<i32> {
    let q = view.clip_from_world * vec4<f32>(p, 1.0);
    if q.w <= 0.1 {
        return vec3<i32>(0, 0, 0);
    }
    let ndc = q.xy / q.w;
    if any(abs(ndc) >= vec2<f32>(0.995)) {
        return vec3<i32>(0, 0, 0);
    }
    let vp = view.main_pass_viewport;
    let pix = vec2<i32>(vec2<f32>(vp.x + (ndc.x * 0.5 + 0.5) * vp.z, vp.y + (0.5 - ndc.y * 0.5) * vp.w));
    let d = textureLoad(scene_depth, pix, 0);
    if d <= 0.0 {
        return vec3<i32>(0, 0, 0);
    }
    let scene_w = view.clip_from_view[3][2] / d;
    return vec3<i32>(pix, select(0, 1, abs(scene_w - q.w) < 0.08 * q.w + 0.5));
}

// Each burst's colour: the lit ground on a ring round where it rose, from the scene as the main pass drew it (before the craters, the
// clouds and the dust). A new burst takes it at once, then it follows slowly; off the screen it keeps what it had.
@compute @workgroup_size(64)
fn dust_tint(@builtin(local_invocation_index) i: u32) {
    let b = fx.dust[i];
    let seed = b.look.w;
    let had = dust_tint_out[i];
    if b.up.w <= 0.0 {
        return;
    }
    let n = normalize(b.up.xyz);
    let e = across(n);
    let ring = max(b.up.w, 2.0) * 1.4;
    var sum = vec3<f32>(0.0);
    var count = 0.0;
    for (var k = 0u; k < 12u; k++) {
        let a = f32(k) / 12.0 * 2.0 * PI;
        let s = seen_pixel(b.at.xyz + (e[0] * cos(a) + e[1] * sin(a)) * ring);
        if s.z == 1 {
            sum += textureLoad(scene_colour, s.xy, 0).rgb;
            count += 1.0;
        }
    }
    if count < 1.0 {
        return;
    }
    let now = sum / count;
    dust_tint_out[i] = vec4<f32>(select(now, mix(had.rgb, now, 0.15), had.w == seed), seed);
}

struct DustOut {
    @builtin(position) clip: vec4<f32>,
    // From the camera (m).
    @location(0) rel: vec3<f32>,
    // In the flipbook.
    @location(1) uv: vec2<f32>,
    // rgb its light, a how much of it there is.
    @location(2) colour: vec4<f32>,
    // The puff's radius (m).
    @location(3) radius: f32,
}

// One quad of a burst: thrown from where it rose by its style, the air holding it back (a plume grows, rises a little and thins out) or
// none (it flies as thrown and falls back), its own frame of the flipbook. Stateless: all of it from the burst's age and the quad's hash.
@vertex
fn dust_vertex(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> DustOut {
    let b = fx.dust[ii];
    let q = f32(vi / 6u);
    let seed = b.look.w * 61.0 + q * 1.37;
    let h1 = hash12(vec2<f32>(seed, 1.0));
    let h2 = hash12(vec2<f32>(seed, 2.0));
    let h3 = hash12(vec2<f32>(seed, 3.0));
    let h4 = hash12(vec2<f32>(seed, 4.0));
    let h5 = hash12(vec2<f32>(seed, 5.0));
    let h6 = hash12(vec2<f32>(seed, 6.0));
    let n = normalize(b.up.xyz);
    let e = across(n);
    let age = b.at.w;
    let size = b.up.w;
    let life = max(b.motion.x, 0.1);
    let g = b.motion.y;
    let hold = b.motion.z;
    let style = u32(b.motion.w + 0.5);
    // How it's thrown. A plume: most of it a surge along the ground, the rest a column; its own life a share of the burst's.
    var el = mix(0.03, 0.3, h1);
    var speed = size * mix(0.7, 1.9, h3);
    var puff = size * mix(0.28, 0.5, h4);
    var drop = false;
    if style == 0u && h6 < 0.35 {
        el = mix(0.7, 1.45, h1);
        speed = size * mix(0.5, 1.5, h3);
        puff = size * mix(0.2, 0.36, h4);
    } else if style == 1u {
        el = mix(0.02, 0.18, h1);
        speed = size * mix(0.6, 1.3, h3);
        puff = size * mix(0.2, 0.36, h4);
    } else if style == 2u {
        if h6 < 0.55 {
            drop = true;
            el = mix(1.0, 1.52, h1);
            speed = size * mix(1.0, 2.6, h3);
            puff = size * mix(0.07, 0.15, h4);
        } else {
            el = mix(0.2, 1.2, h1);
            speed = size * mix(0.4, 1.2, h3);
            puff = size * mix(0.25, 0.5, h4);
        }
    }
    let own_life = life * mix(0.55, 1.0, h5);
    let az = h2 * 2.0 * PI;
    let dir = n * sin(el) + (e[0] * cos(az) + e[1] * sin(az)) * cos(el);
    var off: vec3<f32>;
    var a: f32;
    var frame: f32;
    // Lit from above: the cloud's top lighter than its core near the ground.
    var shade = 1.0;
    if hold > 0.0 && !drop {
        // In air: held back, rising a little, growing as it thins out.
        let k = min(age / own_life, 1.0);
        off = dir * speed * hold * (1.0 - exp(-age / hold)) + n * size * 0.2 * (1.0 - exp(-age / 4.0));
        puff *= 0.45 + 0.9 * sqrt(k);
        // (A ring of dust along the ground is thin: it's seen through.)
        a = smoothstep(0.0, 0.1, age) * pow(max(1.0 - k, 0.0), 1.2) * select(1.0, 0.4, style == 1u);
        frame = mix(9.0, 47.9, k);
        shade = 0.65 + 0.6 * clamp(dot(off, n) / max(size, 0.5), 0.0, 1.0);
    } else {
        // As thrown, falling back; gone where it lands.
        off = dir * speed * age - n * 0.5 * g * age * age;
        let height = dot(off, n);
        off -= n * min(height, 0.0);
        a = smoothstep(0.0, 0.05, age) * (1.0 - smoothstep(0.75, 1.0, age / life)) * (1.0 - smoothstep(0.0, puff * 0.5 + 0.1, -height));
        frame = 14.0 + floor(h4 * 10.0);
    }
    // The quad, facing the camera, turned by its hash.
    let k = corner(vi) * 2.0 - 1.0;
    let spin = h5 * 2.0 * PI + age * (h3 - 0.5) * 0.6;
    let kr = vec2<f32>(k.x * cos(spin) - k.y * sin(spin), k.x * sin(spin) + k.y * cos(spin));
    let right = view.world_from_view[0].xyz;
    let up_v = view.world_from_view[1].xyz;
    let rel = b.at.xyz + off + (right * kr.x + up_v * kr.y) * puff;
    let f = floor(frame);
    let cell = vec2<f32>(f % 7.0, floor(f / 7.0));
    var o: DustOut;
    o.clip = to_clip(rel);
    o.rel = rel;
    o.uv = (cell + k * vec2<f32>(0.5, -0.5) + 0.5) / 7.0;
    // The colour of the lit ground it rose from, a little lighter (fine dust scatters more), once the scene has been read there; else, and
    // for spray, its own colour in his light, the sunlit side brighter, through the haze to him.
    let lit = hero_light.sun.rgb * (0.55 + 0.35 * max(dot(n, fx.sun.xyz), 0.0)) + hero_light.sky.rgb + hero_light.flat_e.rgb;
    var c = b.look.rgb / PI * lit * hero_light.haze_t.rgb + hero_light.haze_a.rgb;
    let tint = dust_tints[ii];
    if style != 2u && tint.w == b.look.w {
        // Fine and dry, it's paler than the ground it rose from.
        let grey = dot(tint.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        c = mix(tint.rgb, vec3<f32>(grey), 0.35) * 1.6;
    }
    c *= shade;
    o.colour = vec4<f32>(c, a);
    o.radius = puff;
    return o;
}

@fragment
fn dust_fragment(in: DustOut) -> @location(0) vec4<f32> {
    let tex = textureSample(sprite, sprite_smp, in.uv);
    let w = 1.0 / in.clip.w;
    let k = soft_depth(vec2<i32>(in.clip.xy), w, max(in.radius * 0.6, 0.3)) * cloud_dim(in.clip.xy, length(in.rel));
    let a = clamp(tex.a * in.colour.a * 2.0, 0.0, 1.0) * k;
    let c = in.colour.rgb * (0.7 + 0.5 * tex.r);
    return vec4<f32>(c * a, a);
}
