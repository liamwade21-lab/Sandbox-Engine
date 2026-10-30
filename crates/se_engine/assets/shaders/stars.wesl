// Point-like stars drawn as screen-space sprites at infinity: SpaceEngine's star sprite
// (star_sprite.glsl). Each star is a quad of 4 vertices sharing a position (parsecs, relative to the
// mesh origin) and data (linear RGB + absolute magnitude). The vertex shader turns the apparent
// magnitude into SE's pixel brightness b (StarBrightMin at the magnitude limit, x 10^0.4 per
// magnitude, x the overbright; compressed above 100), fades the point in over b = StarBrightMin ..
// +1/20, greys faint points (PointDesaturation) and draws a Gaussian of 2 sigma^2 = PointScale^2
// px^2 (at 1080 rows) out to where it falls to 1/500 of the cutoff. Bright points are only a few
// pixels across, with SE's flare wing around them (flareBright x 2 sigma^2 / (flareDecay r^2 + 1), the
// sprite then reaching where the wing falls to the cutoff); their glare comes from the bloom (bloom.rs).

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_world_from_local;

struct StarParams {
    // xyz: camera position relative to the mesh origin, parsecs.
    cam_pos: vec4<f32>,
    // x: magnitude limit, y: overbright factor, z: Gaussian 2 sigma^2 (px^2), w: cutoff brightness
    params: vec4<f32>,
    // x: 1 / PointDesaturation, y: StarColorShift (not for planets), z: colour saturation,
    // w: magnitude offset added to every star (used for reflected-light bodies)
    params2: vec4<f32>,
    // w = 0: stars at infinity, camera offset from cam_pos.
    // w = 1: one sprite drawn at render-space position xyz (system bodies).
    // w = 2: stars at infinity, mesh translation = mesh origin minus camera, in pc
    //        (procedural star octree nodes share one material this way).
    // w = 3: batched sprites, each at its vertex position in render space with its
    //        apparent magnitude in data.w (system bodies, one mesh for all).
    anchor: vec4<f32>,
    // Motion blur (star fields at infinity): x = trail from the camera's motion, y = from its
    // rotation too, z = trail scale (exposure / frame time), w = longest trail (px).
    blur: vec4<f32>,
    // xyz: camera displacement since the previous frame (pc, world axes).
    blur_delta: vec4<f32>,
    // The previous frame's view rotation.
    prev_view: mat4x4<f32>,
    // rgb: colour multiplier (a sun of the system: its unhidden part times the atmospheres' transmittance).
    tint: vec4<f32>,
    // The active system draws its own stars: hide the point at xyz (relative to the camera, pc)
    // within w pc; w = 0 hides nothing.
    hide: vec4<f32>,
    // SE's Param3: x flareBright, y flareDecay (stars.rs flare_params).
    flare: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> star: StarParams;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) data: vec4<f32>,
    @location(2) corner: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // ln(500 b): the Gaussian's exponent at the sprite's edge.
    @location(2) edge: f32,
    // Half the blur trail, in sprite radii.
    @location(3) @interpolate(flat) trail: f32,
    // The flare wing at r^2 (sprite radii): x / (y r^2 + 1).
    @location(4) @interpolate(flat) flare: vec2<f32>,
};

const LOG10_2: f32 = 0.30102999566;
const TO_GRAY: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);

// `se_core::star::blackbody_rgb` at 500 K x 2000^(i/32); w = g + b - r, which rises with the
// temperature, to find a colour's temperature again.
var<private> BLACKBODY: array<vec4<f32>, 33> = array<vec4<f32>, 33>(
    vec4<f32>(1.00000, 0.00000, 0.00000, -1.00000),
    vec4<f32>(1.00000, 0.00000, 0.00000, -1.00000),
    vec4<f32>(1.00000, 0.00000, 0.00000, -1.00000),
    vec4<f32>(1.00000, 0.03272, 0.00000, -0.96728),
    vec4<f32>(1.00000, 0.09502, 0.00000, -0.90498),
    vec4<f32>(1.00000, 0.17929, 0.00000, -0.82071),
    vec4<f32>(1.00000, 0.28494, 0.01515, -0.69991),
    vec4<f32>(1.00000, 0.40971, 0.08912, -0.50118),
    vec4<f32>(1.00000, 0.54887, 0.22579, -0.22534),
    vec4<f32>(1.00000, 0.69495, 0.43820, 0.13315),
    vec4<f32>(1.00000, 0.83833, 0.72438, 0.56271),
    vec4<f32>(0.94004, 0.91117, 1.00000, 0.97113),
    vec4<f32>(0.70276, 0.75963, 1.00000, 1.05687),
    vec4<f32>(0.56558, 0.66219, 1.00000, 1.09662),
    vec4<f32>(0.48173, 0.59756, 1.00000, 1.11583),
    vec4<f32>(0.42807, 0.55358, 1.00000, 1.12551),
    vec4<f32>(0.39239, 0.52299, 1.00000, 1.13060),
    vec4<f32>(0.36789, 0.50128, 1.00000, 1.13339),
    vec4<f32>(0.35061, 0.48560, 1.00000, 1.13499),
    vec4<f32>(0.33816, 0.47409, 1.00000, 1.13594),
    vec4<f32>(0.32901, 0.46554, 1.00000, 1.13653),
    vec4<f32>(0.32220, 0.45911, 1.00000, 1.13690),
    vec4<f32>(0.31707, 0.45423, 1.00000, 1.13715),
    vec4<f32>(0.31317, 0.45049, 1.00000, 1.13732),
    vec4<f32>(0.31018, 0.44762, 1.00000, 1.13744),
    vec4<f32>(0.30787, 0.44540, 1.00000, 1.13753),
    vec4<f32>(0.30609, 0.44368, 1.00000, 1.13759),
    vec4<f32>(0.30470, 0.44233, 1.00000, 1.13764),
    vec4<f32>(0.30362, 0.44128, 1.00000, 1.13767),
    vec4<f32>(0.30277, 0.44046, 1.00000, 1.13770),
    vec4<f32>(0.30211, 0.43982, 1.00000, 1.13771),
    vec4<f32>(0.30159, 0.43932, 1.00000, 1.13773),
    vec4<f32>(0.30118, 0.43892, 1.00000, 1.13774)
);

// SE's StarColorShift: the star colour table built at the temperature x 10^-shift ("+" red, "-" blue;
// FUN_1401a60d0). The baked colour's temperature is found in the blackbody table, shifted and looked up
// again (32 table steps span a factor 2000).
fn color_shift(c: vec3<f32>, shift: f32) -> vec3<f32> {
    let q = c.g + c.b - c.r;
    var i = 0;
    loop {
        if i >= 31 || BLACKBODY[i + 1].w > q {
            break;
        }
        i = i + 1;
    }
    let f = clamp((q - BLACKBODY[i].w) / max(BLACKBODY[i + 1].w - BLACKBODY[i].w, 1e-6), 0.0, 1.0);
    let x = clamp(f32(i) + f - shift * 32.0 * 0.30293575, 0.0, 32.0);
    let j = min(i32(floor(x)), 31);
    return mix(BLACKBODY[j].rgb, BLACKBODY[j + 1].rgb, x - f32(j));
}
const LOG2_10: f32 = 3.32192809489;

fn hidden() -> VertexOut {
    var o: VertexOut;
    o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    o.color = vec3<f32>(0.0);
    o.uv = vec2<f32>(0.0);
    o.edge = 0.0;
    o.trail = 0.0;
    o.flare = vec2<f32>(0.0);
    return o;
}

// Screen trail (px) of a point at world offset `rel` (pc) from the camera, seen at `clip`: from
// where it was a frame ago to where it is, scaled to the exposure.
fn blur_trail(rel: vec3<f32>, clip: vec4<f32>) -> vec2<f32> {
    if star.blur.x < 0.5 || clip.w <= 0.0 {
        return vec2<f32>(0.0);
    }
    let pdir = normalize(rel + star.blur_delta.xyz);
    var pv = view.view_from_world * vec4<f32>(pdir, 0.0);
    if star.blur.y > 0.5 {
        pv = star.prev_view * vec4<f32>(pdir, 0.0);
    }
    let pc = view.clip_from_view * pv;
    if pc.w <= 0.0 {
        return vec2<f32>(0.0);
    }
    var t = (clip.xy / clip.w - pc.xy / pc.w) * view.viewport.zw * 0.5 * star.blur.z;
    let l = length(t);
    if l > star.blur.w {
        t = t * (star.blur.w / l);
    }
    return t;
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let world_from_local = get_world_from_local(v.instance_index);
    let batched = star.anchor.w > 2.5;
    var rel = v.position - star.cam_pos.xyz;
    if star.anchor.w > 1.5 {
        rel = v.position + world_from_local[3].xyz;
    }
    let d = max(length(rel), 1e-12);
    // Only the active system's own point is hidden; every other star stays drawn however close
    // (SE hides only the barycentre pseudo-star).
    if star.hide.w > 0.0 && !batched && length(rel - star.hide.xyz) < star.hide.w {
        return hidden();
    }

    // Apparent magnitude.
    var m = v.data.w + star.params2.w + 5.0 * (log2(d) * LOG10_2 - 1.0);
    if batched {
        m = v.data.w + star.params2.w;
    }
    // SE's pixel brightness, compressed above 100, and the fade-in above the cutoff (Param.w = 20).
    var b = star.params.w * star.params.y * exp2(0.4 * (star.params.x - m) * LOG2_10);
    if b > 100.0 {
        b = 31.6227766 * pow(b, 0.25);
    }
    let fade = clamp(20.0 * (b - star.params.w), 0.0, 1.0);
    if fade <= 0.0 {
        return hidden();
    }

    var dir = rel / d;
    if star.anchor.w < 1.5 {
        dir = normalize((world_from_local * vec4<f32>(dir, 0.0)).xyz);
    }
    var clip = view.clip_from_world * vec4<f32>(dir, 0.0);
    var depth = 0.0;
    if star.anchor.w > 0.5 && star.anchor.w < 1.5 {
        // Positioned sprite (system bodies): real depth so nearer objects occlude it.
        let c = view.clip_from_world * vec4<f32>(star.anchor.xyz, 1.0);
        clip = c;
        depth = c.z;
    }
    if batched {
        let c = view.clip_from_world * vec4<f32>(v.position + world_from_local[3].xyz, 1.0);
        clip = c;
        depth = c.z;
    }

    // The Gaussian out to where it falls to 1/500 of the cutoff, radius^2 = ln(500 b) 2 sigma^2, or the
    // flare wing to where it does, ((flareBright 500 b 2 sigma^2 - 1) / flareDecay) (SE's spriteSize).
    let gauss_r2 = log(b * 500.0) * star.params.z;
    let flare_r2 = (star.flare.x * 500.0 * b * star.params.z - 1.0) / max(star.flare.y, 1e-6);
    let r2_px = max(gauss_r2, flare_r2);
    let edge = r2_px / star.params.z;
    let radius_px = sqrt(r2_px);
    // Motion blur: the sprite becomes a capsule from its last position to this one.
    var trail = vec2<f32>(0.0);
    if star.anchor.w < 0.5 || (star.anchor.w > 1.5 && !batched) {
        trail = blur_trail(dir * d, clip);
    }
    let tl = length(trail);
    var axis = vec2<f32>(1.0, 0.0);
    if tl > 0.01 {
        axis = trail / tl;
    }
    let half = tl * 0.5 / radius_px;
    let off = (axis * v.corner.x * (1.0 + half) + vec2<f32>(-axis.y, axis.x) * v.corner.y) * radius_px - trail * 0.5;
    clip = vec4<f32>(clip.xy + off * 2.0 / view.viewport.zw * clip.w, depth, clip.w);

    // StarColorSaturation, then faint points turn grey (GrayLevel = (b / PointDesaturation)^3; not
    // for planets).
    var base = v.data.rgb;
    if !batched && star.params2.y != 0.0 {
        base = color_shift(base, star.params2.y);
    }
    var col = mix(vec3<f32>(dot(base, TO_GRAY)), base, star.params2.z);
    if !batched {
        col = mix(vec3<f32>(dot(col, TO_GRAY)), col, pow(clamp(b * star.params2.x, 0.0, 1.0), 3.0));
    }

    var o: VertexOut;
    o.clip = clip;
    // The light spreads along the trail.
    o.color = col * b * fade / (1.0 + 2.0 * half);
    o.uv = vec2<f32>(v.corner.x * (1.0 + half), v.corner.y);
    o.edge = edge;
    o.trail = half;
    o.flare = vec2<f32>(star.flare.x * star.params.z, star.flare.y * r2_px);
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    // Distance from the trail (a point without blur).
    let along = max(abs(in.uv.x) - in.trail, 0.0);
    let r2 = along * along + in.uv.y * in.uv.y;
    if r2 > 1.0 {
        discard;
    }
    // SE's Gaussian, exp(-r^2 / 2 sigma^2) with r^2 / 2 sigma^2 = r2 x edge, plus its flare wing
    // flareBright 2 sigma^2 / (flareDecay r^2 + 1), faded at the sprite's edge.
    let a = exp(-r2 * in.edge) + in.flare.x / (in.flare.y * r2 + 1.0) * smoothstep(1.0, 0.0, r2);
    // AlphaMode::Add is premultiplied blending in Bevy: alpha 0 makes it purely additive.
    return vec4<f32>(in.color * a * star.tint.rgb, 0.0);
}
