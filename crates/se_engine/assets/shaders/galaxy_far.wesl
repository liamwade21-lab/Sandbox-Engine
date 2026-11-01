// Far-field galaxy sprites: every catalog galaxy as one elliptical, analytically
// shaded billboard at infinity (like the starfield).
//
// Per vertex: position (pc, Sun-centred, plus the entity translation), data = (linear RGB, absolute magnitude),
// shape = (disk normal xyz, radius pc), corner = (corner xy, galaxy id, profile type).
// Apparent magnitude, angular size and the fade towards the 3D model are computed here.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_world_from_local;

struct FarParams {
    // xyz: camera position in the mesh frame (pc).
    cam_pos: vec4<f32>,
    // x: brightness, y: min sprite radius (px), z: reference focal length (px), w: reference magnitude
    params: vec4<f32>,
    // x: cull when peak intensity falls below this, y: colour saturation, z: intergalactic-vision gain,
    // w: intensity it may raise a sprite to
    params2: vec4<f32>,
    // profile normalisations (integral of the profile over the unit disc) for types 0..3
    norm: vec4<f32>,
    // x: galaxy id (-1 unused), y: model fade (0 = far sprite only, 1 = 3D model only)
    resident: array<vec4<f32>, 16>,
    // Motion blur, as in stars.wesl.
    blur: vec4<f32>,
    blur_delta: vec4<f32>,
    prev_view: mat4x4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> gal: FarParams;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) data: vec4<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) corner: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) ptype: f32,
    // Half the blur trail, in sprite radii.
    @location(3) @interpolate(flat) trail: f32,
};

const LOG10_2: f32 = 0.30102999566;
const LOG2_10: f32 = 3.32192809489;

fn hidden() -> VertexOut {
    var o: VertexOut;
    o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    o.color = vec3<f32>(0.0);
    o.uv = vec2<f32>(0.0);
    o.ptype = 0.0;
    o.trail = 0.0;
    return o;
}

// Screen trail (px) of a point at world offset `rel` (pc), as in stars.wesl.
fn blur_trail(rel: vec3<f32>, clip: vec4<f32>) -> vec2<f32> {
    if gal.blur.x < 0.5 || clip.w <= 0.0 {
        return vec2<f32>(0.0);
    }
    let pdir = normalize(rel + gal.blur_delta.xyz);
    var pv = view.view_from_world * vec4<f32>(pdir, 0.0);
    if gal.blur.y > 0.5 {
        pv = gal.prev_view * vec4<f32>(pdir, 0.0);
    }
    let pc = view.clip_from_view * pv;
    if pc.w <= 0.0 {
        return vec2<f32>(0.0);
    }
    var t = (clip.xy / clip.w - pc.xy / pc.w) * view.viewport.zw * 0.5 * gal.blur.z;
    let l = length(t);
    if l > gal.blur.w {
        t = t * (gal.blur.w / l);
    }
    return t;
}

// Radial surface-brightness profiles, x = r / R in [0, 1]. Must match `profile` in galaxies.rs.
// ptype 3 (quasars) and 4 (star clusters' particles) share the point-like profile.
fn profile(ptype: f32, x: f32) -> f32 {
    let w = max(1.0 - x * x, 0.0);
    let win = w * w;
    if ptype < 0.5 {
        // de Vaucouleurs (ellipticals), effective radius 0.25 R, softened core.
        let xs = sqrt(x * x + 1.0e-4);
        return exp(-7.67 * (pow(xs / 0.25, 0.25) - 1.0)) * win;
    } else if ptype < 1.5 {
        // exponential disc + compact bulge
        return (exp(-3.2 * x) + 2.5 * exp(-60.0 * x * x)) * win;
    } else if ptype < 2.5 {
        // irregular: broad and flat
        return exp(-2.5 * x * x) * win;
    }
    // point-like (quasars)
    return exp(-18.0 * x * x) * win;
}

fn norm_for(ptype: f32) -> f32 {
    if ptype < 0.5 { return gal.norm.x; }
    if ptype < 1.5 { return gal.norm.y; }
    if ptype < 2.5 { return gal.norm.z; }
    return gal.norm.w;
}

// SE's reddening distance (DAT_14075eb8c = 4.227e9 pc, about c / H0).
const REDDEN_PC: f32 = 4.227e9;

// SE's galaxy Reddening (FUN_140299fe0, multiplied into galaxy_model.glsl / galaxy_plane.glsl):
// (1 - x, 1 - 2x, 1 - 4x), x = d / 4.227e9 pc. It dims as well as reddens: blue is gone at 1 Gpc,
// green at 2 Gpc. Must match `reddening` in galaxies.rs.
fn redshift_tint(d_pc: f32) -> vec3<f32> {
    let x = d_pc / REDDEN_PC;
    return clamp(vec3<f32>(1.0) - x * vec3<f32>(1.0, 2.0, 4.0), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Intergalactic vision: the gain in full for unresolved sprites, none for galaxies big on
// screen, and never above the cap. Must match `vision` in galaxies.rs.
fn vision(inten: f32, gain: f32, px: f32, cap: f32) -> f32 {
    let resolved = smoothstep(3.0, 40.0, px);
    let boosted = inten * pow(max(gain, 1.0), 1.0 - resolved);
    return max(inten, min(boosted, cap));
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    // The entity translation is an extra offset of the vertex positions (streamed procedural nodes keep
    // their vertices relative to the node centre and set translation = centre - camera); zero for the catalogs.
    let world_from_local = get_world_from_local(v.instance_index);
    let rel = v.position + world_from_local[3].xyz - gal.cam_pos.xyz;
    let d = max(length(rel), 1e-9);
    let radius_pc = v.shape.w;
    // Camera inside the galaxy: the 3D model (or nothing) takes over.
    if d < radius_pc * 1.02 {
        return hidden();
    }
    var mfade = 0.0;
    for (var i = 0u; i < 16u; i = i + 1u) {
        if abs(gal.resident[i].x - v.corner.z) < 0.5 {
            mfade = gal.resident[i].y;
        }
    }
    if mfade > 0.999 {
        return hidden();
    }

    let focal = view.clip_from_view[1][1] * view.viewport.w * 0.5;
    let dir = rel / d;
    // Angular radius in pixels (exact for a sphere seen from outside).
    let a_px = focal * radius_pc / sqrt(max(d * d - radius_pc * radius_pc, 1e-12));
    let n = v.shape.xyz;
    // Only discs have a meaningful axis; ellipticals/irregulars/points use a zero vector.
    var cosi = 1.0;
    if length(n) > 0.5 {
        cosi = abs(dot(n, dir));
    }
    let min_px = gal.params.y;
    var a_eff = max(a_px, min_px);
    var b_eff = max(a_px * cosi, min_px);
    // Star clusters (ptype 4): SE's star-like particle (star_point.glsl GALAXY path), a point of fixed
    // size whose light follows the flux, faded out as the cluster's radius grows from 1 px to
    // 100 px (x height/1080) on screen while its member stars take over (clusters.rs, from 8 px).
    let particle = v.corner.w > 3.5;
    var particle_fade = 1.0;
    if particle {
        a_eff = min_px;
        b_eff = min_px;
        particle_fade = 1.0 - smoothstep(1.0, 100.0 * view.viewport.w / 1080.0, a_px);
    }

    // Apparent magnitude and flux relative to the reference magnitude.
    let m = v.data.w + 5.0 * (log2(d) * LOG10_2 - 1.0);
    let flux = exp2(0.4 * (gal.params.w - m) * LOG2_10);
    // Surface brightness is independent of distance and of the field of view.
    let sb = (focal / gal.params.z) * (focal / gal.params.z);
    let norm = norm_for(v.corner.w);
    // A new block of procedural galaxies brightens over SE's StarFadeDuration (cam_pos.w: 1 -> 0).
    let gain = gal.params.x * (1.0 - gal.cam_pos.w);
    let inten = vision(gain * flux * sb / (norm * a_eff * b_eff), gal.params2.z, a_px, gal.params2.w) * particle_fade;
    if inten < gal.params2.x {
        return hidden();
    }

    let wdir = normalize((world_from_local * vec4<f32>(dir, 0.0)).xyz);
    let wn = normalize((world_from_local * vec4<f32>(n, 0.0)).xyz);
    var clip = view.clip_from_world * vec4<f32>(wdir, 0.0);
    if clip.w <= 0.0 {
        return hidden();
    }

    let margin = a_eff * 2.0 * 1.5 / view.viewport.zw;
    let ndc = clip.xy / clip.w;
    if abs(ndc.x) > 1.0 + margin.x || abs(ndc.y) > 1.0 + margin.y {
        return hidden();
    }

    // Screen-space axes of the projected disc: the minor axis follows the projected normal.
    let nperp = wn - wdir * dot(wn, wdir);
    let nv = (view.view_from_world * vec4<f32>(nperp, 0.0)).xy;
    var minor = vec2<f32>(0.0, 1.0);
    if length(nv) > 1e-5 {
        minor = normalize(nv);
    }
    var major = vec2<f32>(-minor.y, minor.x);
    var off = v.corner.x * a_eff * major + v.corner.y * b_eff * minor;
    // Motion blur of galaxies seen as points: a round sprite stretched along its trail.
    var half = 0.0;
    if a_eff < 8.0 {
        let trail = blur_trail(wdir * d, clip);
        let tl = length(trail);
        if tl > 0.5 {
            major = trail / tl;
            half = tl * 0.5 / a_eff;
            off = (major * v.corner.x * (1.0 + half) + vec2<f32>(-major.y, major.x) * v.corner.y) * a_eff - trail * 0.5;
        }
    }
    clip = vec4<f32>(clip.xy + off * 2.0 / view.viewport.zw * clip.w, 0.0, clip.w);

    let lum = dot(v.data.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let col = mix(vec3<f32>(lum), v.data.rgb, gal.params2.y) * redshift_tint(d);

    var o: VertexOut;
    o.clip = clip;
    o.color = col * inten * (1.0 - mfade) / (1.0 + 1.27 * half);
    o.uv = vec2<f32>(v.corner.x * (1.0 + half), v.corner.y);
    o.ptype = v.corner.w;
    o.trail = half;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let along = max(abs(in.uv.x) - in.trail, 0.0);
    let x = length(vec2<f32>(along, in.uv.y));
    if x > 1.0 {
        discard;
    }
    // Premultiplied blending in Bevy: alpha 0 makes AlphaMode::Add purely additive.
    return vec4<f32>(in.color * profile(in.ptype, x), 0.0);
}
