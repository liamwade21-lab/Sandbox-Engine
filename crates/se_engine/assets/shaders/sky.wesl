// Atmosphere sky shell: a sphere (mesh radius ~1, back faces only) scaled to the top of the atmosphere.
// Geometry is derived from the shell's own world transform: centre = translation, top radius = scale,
// so the shell works in "scaled space" where translation and scale are both multiplied by k.
// Each light is shadowed per fragment as SE's sky.glsl does it (RINGS, ECL): by the rings where the
// ray towards it crosses their plane (RingsShadow) and by the eclipse casters (EclipseShadowFar at the
// shell point, the top of the atmosphere, times the eclipse intensity), so a moon's shadow is a dark
// patch on the atmosphere and the horizon stays bright in totality.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_clip};
import super::atmosphere_common::{AtmParams, SkyRadiance, atm_sky_radiance, ATM_PI};
import super::eclipse::{EclipseLight, eclipse_shadow};

struct SkyUniform {
    atm: AtmParams,
    // xyz: unit direction towards the star (world/render space), w: star angular radius in radians (0 = no disc)
    sun_dir: vec4<f32>,
    // xyz: sun irradiance (scene radiance units), w: brightness multiplier (catalog `Bright`)
    sun_irradiance: vec4<f32>,
    // Body in front of the star during an eclipse: xyz direction, w angular radius (0 = none).
    occluder: vec4<f32>,
    // rgb: star irradiance for the disc, before eclipse dimming.
    disc_irradiance: vec4<f32>,
    // The further stars (SE's sky.glsl NLights loop): xyz direction, w 1 when used; rgb their
    // irradiance like `sun_irradiance`.
    light_dir: array<vec4<f32>, 3>,
    light_irradiance: array<vec4<f32>, 3>,
    // The rings (SE's RingsParams): x inner, y outer radius (planet radii), z PlanetShadow, w 1 textured,
    // 0.5 untextured, 0 no shadow.
    ring: vec4<f32>,
    // x planet radius / shell radius, y eclipse intensity (SE's AmbientColor.a), z SE's
    // AtmoDistanceLimit in shell radii (0 none), w 1 with AtmoExtinction.
    shadow: vec4<f32>,
    // Each light's eclipse casters, the main star first.
    eclipse: array<EclipseLight, 4>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: SkyUniform;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var inscatter_tex: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atm_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var transmittance_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var ring_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var ring_smp: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

// The scattering is worked out in the shell's own frame, where the atmosphere's top is the unit
// sphere: the shell turns with its body and is flattened like it (SE's ellipsoids), and the
// tables' spherical atmosphere then follows the body's shape.
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) local_pos: vec3<f32>,
    @location(2) @interpolate(flat) cam_local: vec3<f32>,
    // The star's direction, turned into the shell's frame.
    @location(3) @interpolate(flat) sun_local: vec3<f32>,
    // The further stars' directions likewise, w 1 when used.
    @location(4) @interpolate(flat) extra0: vec4<f32>,
    @location(5) @interpolate(flat) extra1: vec4<f32>,
    @location(6) @interpolate(flat) extra2: vec4<f32>,
    // The shell's axes times its scale (world), for the shadows.
    @location(7) @interpolate(flat) axis0: vec3<f32>,
    @location(8) @interpolate(flat) axis1: vec3<f32>,
    @location(9) @interpolate(flat) axis2: vec3<f32>,
};

// A direction `s` (world) in the frame of the shell with axes `c0`..`c2` (times its scale).
fn to_shell(c0: vec3<f32>, c1: vec3<f32>, c2: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(dot(c0, s) / length(c0), dot(c1, s) / length(c1), dot(c2, s) / length(c2)));
}

fn extra_local(c0: vec3<f32>, c1: vec3<f32>, c2: vec3<f32>, l: vec4<f32>) -> vec4<f32> {
    if l.w < 0.5 {
        return vec4<f32>(0.0, 1.0, 0.0, 0.0);
    }
    return vec4<f32>(to_shell(c0, c1, c2, normalize(l.xyz)), 1.0);
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    return shell_vertex(v);
}

fn shell_vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    var o: VertexOut;
    o.clip = mesh_position_local_to_clip(m, vec4<f32>(v.position, 1.0));
    o.world_pos = (m * vec4<f32>(v.position, 1.0)).xyz;
    o.local_pos = v.position;
    // The columns are the shell's axes times its scale (rotation and scale only, no shear).
    let c0 = m[0].xyz;
    let c1 = m[1].xyz;
    let c2 = m[2].xyz;
    let d = view.world_position - m[3].xyz;
    o.cam_local = vec3<f32>(dot(c0, d) / dot(c0, c0), dot(c1, d) / dot(c1, c1), dot(c2, d) / dot(c2, c2));
    o.sun_local = to_shell(c0, c1, c2, normalize(sky.sun_dir.xyz));
    o.extra0 = extra_local(c0, c1, c2, sky.light_dir[0]);
    o.extra1 = extra_local(c0, c1, c2, sky.light_dir[1]);
    o.extra2 = extra_local(c0, c1, c2, sky.light_dir[2]);
    o.axis0 = c0;
    o.axis1 = c1;
    o.axis2 = c2;
    return o;
}

// The daylight pass sits on the far plane: with reversed depth it is drawn only where nothing
// wrote depth (the stars, galaxies and nebulae), not over the bodies.
@vertex
fn vertex_far(v: Vertex) -> VertexOut {
    var o = shell_vertex(v);
    o.clip.z = 0.0;
    return o;
}

// SE's RingsShadow (rings_common.glh) on the sky: the light let through by the rings on the way from
// `ps` (body frame, shell radii, the shell's shape applied) towards `l` (body frame), 1 where the ray
// does not cross their plane.
fn ring_shadow(ps: vec3<f32>, l: vec3<f32>) -> f32 {
    if sky.ring.w < 0.25 {
        return 1.0;
    }
    let along = min(ps.y / select(l.y, 1e-12, abs(l.y) < 1e-12), 0.0);
    let proj = ps.xz - l.xz * along;
    let tex_u = (length(proj) / sky.shadow.x - sky.ring.x) / (sky.ring.y - sky.ring.x);
    let fw = fwidth(tex_u * 2.0);
    // Off the rings the smoothing is 0.
    if tex_u <= 0.0 || tex_u >= 1.0 {
        return 1.0;
    }
    let u = tex_u;
    let smoothing = clamp(min(u, 1.0 - u) / max(fw, 1e-12), 0.0, 1.0);
    var a = 0.6;
    if sky.ring.w > 0.75 {
        a = textureSampleLevel(ring_tex, ring_smp, vec2<f32>(u, 0.75), 0.0).a;
    }
    let density = clamp(sky.ring.z * a, 0.0, 1.0);
    let cos_phi = abs(l.y);
    let opacity = (1.0 - pow(1.0 - density, 1.0 / max(cos_phi, 0.333))) * smoothing * smoothstep(0.001, 0.003, cos_phi);
    return 1.0 - opacity;
}

// The share of light `k` (world direction `l`, body-frame `ll`) left at the shell point `pt` (shell
// frame, unit sphere): SE's `Shadow` of sky.glsl.
fn light_shadow(in: VertexOut, k: i32, pt: vec3<f32>, l: vec3<f32>, ll: vec3<f32>) -> f32 {
    let c0 = length(in.axis0);
    let shape = vec3<f32>(1.0, length(in.axis1) / c0, length(in.axis2) / c0);
    var s = ring_shadow(pt * shape, ll);
    let e = sky.eclipse[k];
    if e.info.y > 0.5 {
        // The shell point itself, at the top of the atmosphere (SE's FragPos at Radiuses.w, the shell's
        // radius, sky.glsl:186-188), in planet radii.
        let off = in.axis0 * pt.x + in.axis1 * pt.y + in.axis2 * pt.z;
        s *= 1.0 - sky.shadow.y * eclipse_shadow(e, off / (c0 * sky.shadow.x), l);
    }
    return s;
}

struct SkyLook {
    // The sky's light along the ray (and the star's disc).
    col: vec3<f32>,
    // Transmittance from the camera to space along the ray, 0 where the ray hits the ground.
    transmittance: vec3<f32>,
};

fn sky_look(in: VertexOut) -> SkyLook {
    let rt = sky.atm.radii.y;
    // SE's AtmoDistanceLimit: the scattering seen from no farther than that many radii.
    var cam_l = in.cam_local;
    let cl = length(cam_l);
    if sky.shadow.z > 0.0 && cl > sky.shadow.z {
        cam_l *= sky.shadow.z / cl;
    }
    let cam = cam_l * rt;
    let ray = normalize(in.local_pos - cam_l);
    // Where the shadows are taken: where the ray enters the shell from space (SE draws its near side
    // then), else the far shell point the fragment is on.
    var pt = in.local_pos;
    let b = dot(cam_l, ray);
    let c = dot(cam_l, cam_l) - 1.0;
    if c > 0.0 && b * b - c > 0.0 {
        pt = cam_l + ray * (-b - sqrt(b * b - c));
    }
    let sr: SkyRadiance = atm_sky_radiance(sky.atm, inscatter_tex, transmittance_tex, atm_sampler, cam, ray, in.sun_local);
    // The disc in world directions (round whatever the shell's shape).
    let v = normalize(in.world_pos - view.world_position);
    let s = normalize(sky.sun_dir.xyz);
    let e_sun = sky.sun_irradiance.rgb * sky.sun_irradiance.w;
    var col = sr.radiance * e_sun * light_shadow(in, 0, pt, s, in.sun_local);
    // The further stars' sky light.
    var extra = array<vec4<f32>, 3>(in.extra0, in.extra1, in.extra2);
    for (var i = 0; i < 3; i++) {
        if extra[i].w < 0.5 {
            break;
        }
        let sx = atm_sky_radiance(sky.atm, inscatter_tex, transmittance_tex, atm_sampler, cam, ray, extra[i].xyz);
        let sh = light_shadow(in, i + 1, pt, normalize(sky.light_dir[i].xyz), extra[i].xyz);
        col += sx.radiance * sky.light_irradiance[i].rgb * sky.sun_irradiance.w * sh;
    }

    // Optional sun disc, reddened by the atmosphere (only when the ray does not hit the ground).
    let rad = sky.sun_dir.w;
    if rad > 0.0 && sr.ground_dist < 0.0 {
        let nu = dot(v, s);
        let c = cos(rad);
        if nu > c {
            let solid_angle = ATM_PI * rad * rad;
            let edge = smoothstep(c, c + (1.0 - c) * 0.1, nu);
            // Hide the part of the disc covered by an eclipsing body.
            var mask = 1.0;
            if sky.occluder.w > 0.0 {
                let co = cos(sky.occluder.w);
                let w = (1.0 - co) * 0.03;
                mask = 1.0 - smoothstep(co - w, co + w, dot(v, normalize(sky.occluder.xyz)));
            }
            let e_disc = sky.disc_irradiance.rgb * sky.sun_irradiance.w;
            col = col + e_disc / solid_angle * sr.transmittance * edge * mask;
        }
    }
    var o: SkyLook;
    o.col = col;
    // Behind the ground the terrain owns the pixel.
    o.transmittance = select(vec3<f32>(0.0), sr.transmittance, sr.ground_dist < 0.0);
    return o;
}

// Pass 3: the sky's light, added (premultiplied blending with alpha 0).
@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(sky_look(in).col, 0.0);
}

// Pass 2 (multiplied into the frame): what lies behind the sky, dimmed and reddened per channel
// (SE's `atmo_transm.glsl`), so the Moon shows in the daytime sky and stars redden at the horizon;
// nothing without AtmoExtinction.
@fragment
fn extinction(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(mix(vec3<f32>(1.0), sky_look(in).transmittance, sky.shadow.w), 1.0);
}

// Pass 1 (multiplied into the frame, empty background only): a bright sky hides the stars,
// galaxies and nebulae (none by day; they come out at dusk and as the sky darkens on the way up).
// The scale is the sky's luminance at which they fade (tuned when the sky had 5x its SE light).
const DAYLIGHT_LUM: f32 = 0.004;

@fragment
fn daylight(in: VertexOut) -> @location(0) vec4<f32> {
    let lum = dot(sky_look(in).col, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(vec3<f32>(exp(-lum / DAYLIGHT_LUM)), 1.0);
}
