// Atmosphere sky shell: a sphere (mesh radius ~1, back faces only) scaled to the top of the atmosphere.
// Geometry is derived from the shell's own world transform: centre = translation, top radius = scale,
// so the shell works in "scaled space" where translation and scale are both multiplied by k.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_clip};
import super::atmosphere_common::{AtmParams, SkyRadiance, atm_sky_radiance, ATM_PI};

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
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: SkyUniform;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var inscatter_tex: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atm_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var transmittance_tex: texture_2d<f32>;

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

struct SkyLook {
    // The sky's light along the ray (and the star's disc).
    col: vec3<f32>,
    // Transmittance from the camera to space along the ray, 0 where the ray hits the ground.
    transmittance: vec3<f32>,
};

fn sky_look(in: VertexOut) -> SkyLook {
    let rt = sky.atm.radii.y;
    let cam = in.cam_local * rt;
    let sr: SkyRadiance = atm_sky_radiance(sky.atm, inscatter_tex, transmittance_tex, atm_sampler, cam, normalize(in.local_pos - in.cam_local), in.sun_local);
    // The disc in world directions (round whatever the shell's shape).
    let v = normalize(in.world_pos - view.world_position);
    let s = normalize(sky.sun_dir.xyz);
    let e_sun = sky.sun_irradiance.rgb * sky.sun_irradiance.w;
    var col = sr.radiance * e_sun;
    // The further stars' sky light.
    var extra = array<vec4<f32>, 3>(in.extra0, in.extra1, in.extra2);
    for (var i = 0; i < 3; i++) {
        if extra[i].w < 0.5 {
            break;
        }
        let sx = atm_sky_radiance(sky.atm, inscatter_tex, transmittance_tex, atm_sampler, cam, normalize(in.local_pos - in.cam_local), extra[i].xyz);
        col += sx.radiance * sky.light_irradiance[i].rgb * sky.sun_irradiance.w;
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
// (SE's `atmo_transm.glsl`), so the Moon shows in the daytime sky and stars redden at the horizon.
@fragment
fn extinction(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(sky_look(in).transmittance, 1.0);
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
