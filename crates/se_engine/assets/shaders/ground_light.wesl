// Lighting shared by the ground (planet.wesl) and the objects that stand on it (scatter_draw.wesl:
// trees now, buildings later): the light record, the clouds' shadow, the terminator and the haze.
// A tree, a building and the ground under them must take the same light, or forests and towns show
// as wrongly lit patches from altitude. Like the other shared modules this one has no bindings: the
// textures and records are passed in.

import super::atmosphere_common::{AtmParams, atm_apply_aerial};
import super::eclipse::EclipseLight;
import super::cloud_cover::{weather_cover, weather_shaped, climb_dist};

// A body's lighting (terrain.rs PatchLight): one record per ground node in the material's light page,
// and the body's own copy for the objects on it. Clouds.wesl has the same struct.
struct PatchLight {
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    // Each light's casters: [0] the brightest star, then the extra lights in order.
    eclipse: array<EclipseLight, 4>,
    // The further stars (SE lights a body by up to 4, planet.glsl's NLights loop): xyz direction,
    // w 1 when used; rgb their light, intensity included.
    light_dir: array<vec4<f32>, 3>,
    light_color: array<vec4<f32>, 3>,
    // SE's GlowColorAtmo (terrain.rs glow_color_atmo), in the units of `light_color`; 0 none. w the 3D
    // street objects' range (m; 0 none: planet.wesl `street_near`).
    glow_atmo: vec4<f32>,
    // Planet's relit components: x base_color.w (ambient), y params.x (glow brightness), z thermal2.w
    // (self-emission scale), w ice.w (water display level).
    levels: vec4<f32>,
    // x atm_flags.x (atmosphere enabled), y soil_tile.w (geomorph split distance).
    flags: vec4<f32>,
};

// The volumetric clouds' shadow record (volumetric_clouds.rs CloudShadow).
struct CloudShadow {
    // x: the body's radius (m), y: its seed fraction (as `planet.proc_params2.xy`), z: 1 on.
    key: vec4<f32>,
    // xyz: towards the star in the body frame.
    sun: vec4<f32>,
    // Per layer: x bottom, y top (m above the radius), z opacity (0: none), w how cumuliform.
    layer_a: array<vec4<f32>, 4>,
    // Per layer: x, y cos and sin of the layer's turn about the pole.
    layer_b: array<vec4<f32>, 4>,
    // The shadow map around the camera: xyz its centre in the body's frame on the reference sphere
    // (m, the ellipsoid undone), w its half-width (m); its two axes.
    map_c: vec4<f32>,
    map_e1: vec4<f32>,
    map_e2: vec4<f32>,
};

// The light left by the volumetric clouds over the body-frame direction `bdir` at `height` m on a
// body of radius `r` (m) whose cloud key (radius, seed fraction) is `key`: near the camera their
// shadow map (traced through the clouds themselves), farther out each layer's cover where the star's
// ray crosses the layer's cloud mass, as the map's mean. `cover` and `weather`, `pattern` share
// `cover_smp`.
fn cloud_shadow_at(
    sh: CloudShadow,
    cover: texture_cube<f32>,
    cover_smp: sampler,
    map: texture_2d<f32>,
    map_smp: sampler,
    weather: texture_cube<f32>,
    pattern: texture_cube<f32>,
    r: f32,
    key: vec2<f32>,
    bdir: vec3<f32>,
    height: f32,
) -> f32 {
    if sh.key.z < 0.5 || abs(sh.key.x - key.x) > 0.5 || abs(sh.key.y - key.y) > 1e-4 {
        return 1.0;
    }
    let l = sh.sun.xyz;
    // (Past the terminator the ground still takes some of the star's light, so the shadow goes on there as
    // for a grazing star: cut off at the terminator, the clouds' shadows once ended in a line.)
    let mu = max(dot(bdir, l), 0.0);
    var t = 1.0;
    for (var i = 0; i < 4; i++) {
        let a = sh.layer_a[i];
        if a.z <= 0.0 {
            continue;
        }
        let mid = mix(a.x, a.y, 0.25 + 0.25 * (1.0 - a.w));
        let q = normalize(bdir * (r + height) + l * climb_dist(r + height, mu, max(mid - height, 0.0)));
        let cs = sh.layer_b[i].xy;
        let d = vec3<f32>(cs.x * q.x - cs.y * q.z, q.y, cs.y * q.x + cs.x * q.z);
        let w = textureSampleLevel(weather, cover_smp, q, 0.0);
        let add = weather_shaped(w.g * 2.0 - 1.0, textureSampleLevel(pattern, cover_smp, q, 2.0).r);
        let c = weather_cover(textureSampleLevel(cover, cover_smp, d, 2.0)[i], w.r * 2.0, add, a.z);
        // As the shadow map near the camera gives it (volumetric_clouds.wesl `shadow_gen`, a quarter of
        // the light through thick cloud): solid clouds over their share of the area, a veil along the
        // star's slant path (the map's shadows were once far fainter: a square of sunlit ground around
        // the camera).
        let solid = a.w * smoothstep(0.02, 0.25, c);
        let veil = 0.25 + 0.75 * pow(max(1.0 - 0.995 * c, 0.0), 1.0 / max(mu, 0.08));
        t *= mix(veil, 1.0 - 0.75 * c, solid);
    }
    let rel = bdir * r - sh.map_c.xyz;
    let uv = vec2<f32>(dot(rel, sh.map_e1.xyz), dot(rel, sh.map_e2.xyz)) / sh.map_c.w * 0.5 + 0.5;
    let edge = max(abs(uv.x - 0.5), abs(uv.y - 0.5)) * 2.0;
    if edge < 1.0 {
        // The map's own shapes at the cover's mean darkness: the map over its mean across 10 km (its last
        // mip), times the cover's shadow. (A patchy cloud lets more light through than an even one of the
        // same depth, so the map alone was lighter than the cover's shadow: a lighter square round the
        // camera.)
        let m = textureSampleLevel(map, map_smp, uv, 0.0).r;
        let mean = textureSampleLevel(map, map_smp, uv, 6.0).r;
        t = mix(min(t * m / max(mean, 0.25), 1.0), t, smoothstep(0.75, 1.0, edge));
    }
    return t;
}

// The star's share above the horizon at `height` m on a body of radius `r` (m), for `mu` the cosine of
// its angle from the zenith.
fn horizon_shadow_at(r: f32, height: f32, mu: f32) -> f32 {
    let rr = clamp(1.0 / (1.0 + max(height, 0.0) / max(r, 1.0)), 0.0, 0.9999);
    return clamp((sqrt(1.0 - rr * rr) + mu) * 500.0, 0.0, 1.0);
}

// A world direction along the body's axes (the sky shell's `to_shell`).
fn axes_dir(ax: vec3<f32>, ay: vec3<f32>, az: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(dot(d, ax), dot(d, ay), dot(d, az)));
}

// Aerial perspective between the camera (`cam_km`) and a surface point (`view_km`), both in the
// atmosphere tables' frame: the brightest star's haze with its light `e_sun` (its cloud and eclipse
// shadow at the point included), the further stars' and a glowing atmosphere's. `ax`..`az` the body's
// unit axes (render frame), `l` towards the brightest star, `atm_flags` as the ground's.
fn ground_haze(
    p: AtmParams,
    tex_s: texture_3d<f32>,
    tex_t: texture_2d<f32>,
    smp: sampler,
    col_in: vec3<f32>,
    cam_km: vec3<f32>,
    view_km: vec3<f32>,
    ax: vec3<f32>,
    ay: vec3<f32>,
    az: vec3<f32>,
    l: vec3<f32>,
    e_sun: vec3<f32>,
    light: PatchLight,
    atm_flags: vec4<f32>,
) -> vec3<f32> {
    var col = atm_apply_aerial(p, tex_s, tex_t, smp, col_in, cam_km, view_km, axes_dir(ax, ay, az, l), e_sun * atm_flags.z);
    // The further stars' haze.
    for (var i = 0; i < 3; i++) {
        if light.light_dir[i].w < 0.5 {
            break;
        }
        let e_x = light.light_color[i].rgb * atm_flags.y * atm_flags.z;
        col += atm_apply_aerial(p, tex_s, tex_t, smp, vec3<f32>(0.0), cam_km, view_km, axes_dir(ax, ay, az, normalize(light.light_dir[i].xyz)), e_x);
    }
    // A glowing atmosphere over the ground (planet.glsl THERM): its inscatter as if lit from the
    // planet's centre times GlowColorAtmo.
    let g = light.glow_atmo.rgb;
    if g.r + g.g + g.b > 0.0 {
        col += atm_apply_aerial(p, tex_s, tex_t, smp, vec3<f32>(0.0), cam_km, view_km, vec3<f32>(0.0), g * atm_flags.y * atm_flags.z);
    }
    return col;
}
