// Lighting shared by the ground (planet.wesl) and what stands on it (scatter_draw.wesl: trees and
// buildings; hero.wesl: the Viltrumite mod's hero): the light record, the clouds' shadow, the terminator,
// the haze and the normal-free light at a point. Everything on a body must take the same light, or
// forests and towns show as wrongly lit patches from altitude. Like the other shared modules this one
// has no bindings: the textures and records are passed in.

import super::atmosphere_common::{AtmParams, ATM_AMBIENT_SCALE, ATM_PI, atm_transmittance_sun, atm_irradiance, atm_apply_aerial};
import super::eclipse::{EclipseLight, eclipse_light};
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
    // The Viltrumite mod's hero: where the ground holds still (planet.wesl), a line from the camera (m). `hero` its start, his soles'
    // place on the ground, w how much it holds (0 with the mod off); `hero_end` its end, where his shadow ends.
    hero: vec4<f32>,
    hero_end: vec4<f32>,
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

// The light at a point that does not depend on the surface's normal: worked out per object or vertex
// (the atmosphere tables, the clouds' shadow and the haze barely change across a tree), so the pixels
// only add the normal's part (`lit_surface`).
struct PointLight {
    // The brightest star's direct light at the point: shadowed (eclipses, clouds), through the
    // atmosphere, in the ground's scale.
    sun: vec3<f32>,
    // Its sky light (times 0.5 + 0.5 n.up in the pixel).
    sky: vec3<f32>,
    // Day ambient, the further stars (taken on level ground) and SE's AmbientLighting.
    flat_e: vec3<f32>,
    // The haze between the point and the camera: transmittance and inscattered light.
    haze_t: vec3<f32>,
    haze_a: vec3<f32>,
}

// What `point_light_at` reads of a body besides its atmosphere and light record: `m0`..`m2` its axes
// scaled by the ellipsoid (render frame), `centre` xyz from the camera (m), w its radius (m), `atm_flags`
// as the ground's, `key` x, y its cloud key (radius m, seed fraction), z 1 with atmosphere tables bound.
struct LightBody {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    centre: vec4<f32>,
    atm_flags: vec4<f32>,
    key: vec4<f32>,
}

// One star's light on level ground at the point, as lit_ground gives the ground: (direct through
// the atmosphere, sky light) for irradiance `e`.
fn star_terms(
    atm: AtmParams,
    tex_t: texture_2d<f32>,
    tex_e: texture_2d<f32>,
    smp: sampler,
    radius: f32,
    up: vec3<f32>,
    pos_km: vec3<f32>,
    height: f32,
    l: vec3<f32>,
    e: vec3<f32>,
    has_atm: bool,
) -> array<vec3<f32>, 2> {
    if has_atm {
        let r = length(pos_km);
        let mu_s = dot(pos_km / r, l);
        let t_sun = atm_transmittance_sun(atm, tex_t, smp, r, mu_s);
        let e_sky = atm_irradiance(atm, tex_e, smp, r, mu_s) * (atm.look.w * ATM_PI * ATM_AMBIENT_SCALE);
        return array<vec3<f32>, 2>(e * t_sun, e * e_sky);
    }
    return array<vec3<f32>, 2>(e * horizon_shadow_at(radius, height, dot(up, l)), vec3<f32>(0.0));
}

// The normal-free light at `world` (render frame, the camera at the origin), as the ground there
// takes it (planet.wesl lit_ground and finish). `smp` samples the atmosphere tables; the clouds' textures
// are as `cloud_shadow_at`'s.
fn point_light_at(
    b: LightBody,
    atm: AtmParams,
    light: PatchLight,
    tex_s: texture_3d<f32>,
    tex_t: texture_2d<f32>,
    tex_e: texture_2d<f32>,
    smp: sampler,
    sh: CloudShadow,
    cover: texture_cube<f32>,
    cover_smp: sampler,
    map: texture_2d<f32>,
    map_smp: sampler,
    weather: texture_cube<f32>,
    pattern: texture_cube<f32>,
    world: vec3<f32>,
) -> PointLight {
    let centre = b.centre.xyz;
    let radius = b.centre.w;
    let rel = world - centre;
    // The body's axes and the ellipsoid's scale on them: its sphere frame, as the ground's table_local.
    let ax = normalize(b.m0.xyz);
    let ay = normalize(b.m1.xyz);
    let az = normalize(b.m2.xyz);
    let alen = vec3<f32>(length(b.m0.xyz), length(b.m1.xyz), length(b.m2.xyz));
    let local = vec3<f32>(dot(rel, ax) / alen.x, dot(rel, ay) / alen.y, dot(rel, az) / alen.z);
    let bdir = normalize(local);
    let height = length(local) - radius;
    let up = normalize(rel);
    let p_rel = rel / radius;
    let l = normalize(light.sun_dir.xyz);
    let has_atm = b.atm_flags.x > 0.5 && b.key.z > 0.5;
    let pos_km = up * atm.radii.x * (1.0 + max(height, 0.0) / radius);
    // At the height the ground gives its cloud shadow (lit_ground: the radial distance in the render
    // frame, the ellipsoid's), not the sphere frame's: on a flattened planet they differ by up to
    // flattening x radius.
    let clouds = cloud_shadow_at(sh, cover, cover_smp, map, map_smp, weather, pattern, radius, b.key.xy, bdir, length(rel) - radius);
    let shadow = eclipse_light(light.eclipse[0], p_rel, l) * clouds;
    let e_sun = light.sun_color.rgb * light.sun_dir.w * shadow * b.atm_flags.y;
    var o: PointLight;
    let t0 = star_terms(atm, tex_t, tex_e, smp, radius, up, pos_km, height, l, e_sun, has_atm);
    o.sun = t0[0];
    o.sky = t0[1];
    // SE's day ambient (DayAmbient) on the day side, and its AmbientLighting.
    o.flat_e = e_sun * (ATM_PI * ATM_AMBIENT_SCALE * b.atm_flags.w * max(dot(up, l), 0.0)) + vec3<f32>(light.levels.x * b.atm_flags.y);
    // The further stars (no cloud shadow, as the ground's), on level ground.
    for (var i = 0; i < 3; i++) {
        if light.light_dir[i].w < 0.5 {
            break;
        }
        let lx = normalize(light.light_dir[i].xyz);
        let e_x = light.light_color[i].rgb * eclipse_light(light.eclipse[i + 1], p_rel, lx) * b.atm_flags.y;
        let tx = star_terms(atm, tex_t, tex_e, smp, radius, up, pos_km, height, lx, e_x, has_atm);
        o.flat_e += tx[0] * max(dot(up, lx), 0.0) + tx[1] + e_x * (ATM_PI * ATM_AMBIENT_SCALE * b.atm_flags.w * max(dot(up, lx), 0.0));
    }
    o.haze_t = vec3<f32>(1.0);
    o.haze_a = vec3<f32>(0.0);
    if has_atm {
        let to_tables = atm.radii.x / max(radius, 1e-30);
        let c = -centre;
        let cam_km = vec3<f32>(dot(c, ax) / alen.x, dot(c, ay) / alen.y, dot(c, az) / alen.z) * to_tables;
        var view_km = local * to_tables;
        let r_ground = atm.radii.x * (1.0 + height / max(radius, 1.0));
        view_km *= max(r_ground / max(length(view_km), 1e-6), 1.0);
        o.haze_a = ground_haze(atm, tex_s, tex_t, smp, vec3<f32>(0.0), cam_km, view_km, ax, ay, az, l, e_sun, light, b.atm_flags);
        o.haze_t = atm_apply_aerial(atm, tex_s, tex_t, smp, vec3<f32>(1.0), cam_km, view_km, l, vec3<f32>(0.0));
    }
    return o;
}

// A surface's colour from its point's light, albedo and normal `n`: `l` towards the brightest star, `wrap`
// softens its term (leaves), `trans` adds the light through them.
fn lit_surface(pl: PointLight, l: vec3<f32>, albedo: vec3<f32>, n: vec3<f32>, up: vec3<f32>, wrap: f32, trans: f32) -> vec3<f32> {
    let direct = max((dot(n, l) + wrap) / (1.0 + wrap), 0.0) + trans;
    let e = pl.sun * direct + pl.sky * (0.5 + 0.5 * dot(n, up)) + pl.flat_e;
    return albedo / ATM_PI * e * pl.haze_t + pl.haze_a;
}
