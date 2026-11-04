// Cloud shell patch, lit as SpaceEngine lights a cloud layer (planet.glsl, ACLOUDS): the star's
// light reddened through the atmosphere and cut by the planet's shadow at the cloud's height, the
// sky's irradiance (times SkyLight), SE's day ambient, a Hapke mix and subsurface scattering, then
// the haze between the camera and the cloud. Alpha-blended (straight colour; SE's is premultiplied).

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import super::eclipse::{EclipseLight, eclipse_light};
import super::atmosphere_common::{AtmParams, ATM_AMBIENT_SCALE, atm_transmittance_sun, atm_irradiance, atm_apply_aerial};
import super::cloud_cover::{se_terra_clouds, proc_clouds, se_cut, weather_cover};

const PI: f32 = 3.14159265358979;

// The node's lighting (terrain.rs PatchLight, as in planet.wesl): one record per node in the
// material's light page, at `clouds.slot`.
struct PatchLight {
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    // Each light's casters: [0] the brightest star, then the extra lights in order.
    eclipse: array<EclipseLight, 4>,
    // The further stars: xyz direction, w 1 when used; rgb their light, intensity included.
    light_dir: array<vec4<f32>, 3>,
    light_color: array<vec4<f32>, 3>,
    // The ground's (unused here); levels x SE's AmbientLighting.
    glow_atmo: vec4<f32>,
    levels: vec4<f32>,
    // x: atmosphere enabled.
    flags: vec4<f32>,
};

// The fixed part of the material; the lighting is in `lit`.
struct Clouds {
    rect: vec4<f32>,
    // x opacity, y relief of the cloud tops (m), z 1: the map is the colour of an opaque layer
    params: vec4<f32>,
    // xyz: planet centre (node-local), w: planet radius
    planet: vec4<f32>,
    // Procedural clouds (no texture): x coverage, y base frequency, z octaves, w enabled
    proc: vec4<f32>,
    // x seed, y the layer's index, z latitude belt strength, w the body's seed fraction
    proc2: vec4<f32>,
    // SE's terra clouds: 0 (cloudsFreq, cloudsOctaves, stripeZones, stripeTwist), 1 (cycloneMagn,
    // cycloneFreq, cycloneDensity, cycloneOctaves), 2 (Randomize, tidal lock +-1); se[0].x 0: off.
    se: array<vec4<f32>, 3>,
    // rgb: albedo (ModulateColor x ModulateBright)
    color: vec4<f32>,
    // x DayAmbient, y SubsurfScatBright, z SubsurfScatPower, w Hapke
    look: vec4<f32>,
    // x: (in `lit`) atmosphere enabled, y: sun irradiance scale for the tables, z: the haze relative
    // to y, w unused
    atm_flags: vec4<f32>,
    atm: AtmParams,
    // The node's record in `lit`.
    slot: u32,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> clouds: Clouds;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var clouds_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var clouds_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var inscatter_tex: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var atm_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var transmittance_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var irradiance_tex: texture_2d<f32>;
// The light page holding the node's PatchLight (at `clouds.slot`).
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var<storage, read> lit: array<PatchLight>;

// The volumetric clouds' record (volumetric_clouds.rs `CloudShadow`, as planet.wesl's). For the body
// they draw: key.w how much the weather shapes the shells, sun.w the shells' share of the clouds (the
// far cross-fade), layer_b[i] the layer's turn (xy) and its index + 1 (w).
struct CloudShadow {
    key: vec4<f32>,
    sun: vec4<f32>,
    layer_a: array<vec4<f32>, 4>,
    layer_b: array<vec4<f32>, 4>,
    map_c: vec4<f32>,
    map_e1: vec4<f32>,
    map_e2: vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var cloud_weather: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var cloud_weather_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var<storage, read> cloud_sh: CloudShadow;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) up: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // xyz planet centre (render space), w planet radius (render units)
    @location(3) @interpolate(flat) centre: vec4<f32>,
    // Direction from the planet centre in the body frame (procedural clouds).
    @location(4) body_dir: vec3<f32>,
    // The direction to the star in the layer's frame (the substellar point of locked worlds).
    @location(5) @interpolate(flat) sun_body: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let world_from_local = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_pos = world.xyz;
    o.up = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.uv = v.uv;
    let centre = (world_from_local * vec4<f32>(clouds.planet.xyz, 1.0)).xyz;
    o.centre = vec4<f32>(centre, clouds.planet.w * length(world_from_local[0].xyz));
    // Node meshes are in the body frame, offset from the planet centre.
    o.body_dir = v.position - clouds.planet.xyz;
    let s = lit[clouds.slot].sun_dir.xyz;
    o.sun_body = normalize(vec3<f32>(dot(normalize(world_from_local[0].xyz), s), dot(normalize(world_from_local[1].xyz), s), dot(normalize(world_from_local[2].xyz), s)) + vec3<f32>(1e-12));
    return o;
}

// SE's 256-texel tiles carry a 2-texel border shared with their neighbours (each repeats the
// other's 4 edge texels): the tile proper spans texels 2 to 254.
fn tile_uv(rect: vec4<f32>, uv: vec2<f32>) -> vec2<f32> {
    let t = rect.xy + uv * rect.z;
    return t * (252.0 / 256.0) + vec2<f32>(2.0 / 256.0);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let up = normalize(in.up);
    let l = normalize(lit[clouds.slot].sun_dir.xyz);
    let texel = textureSample(clouds_tex, clouds_smp, tile_uv(clouds.rect, in.uv));
    // SE's DiffMapAlpha: "None" makes the map the colour of an opaque layer (Venus); otherwise the
    // (greyscale) map is the opacity.
    let opaque = clouds.params.z > 0.5;
    var density = select(texel.r, 1.0, opaque);
    let cdir = normalize(in.body_dir);
    let cfp = max(length(fwidth(cdir)), 1e-9);
    if clouds.proc.w > 0.5 {
        if clouds.se[0].x > 0.0 {
            density = se_terra_clouds(cdir, clouds.proc.x, cfp, in.sun_body, clouds.se);
        } else {
            density = proc_clouds(cdir, length(fwidth(cdir)), clouds.proc, clouds.proc2);
        }
    }
    // The cloud tops' relief (SE draws the layer with its BumpMap, Earth's being the cloud map):
    // the normal tilted by the density's screen-space gradient, scaled to render units.
    let h = density * clouds.params.y * in.centre.w / max(clouds.planet.w, 1.0);
    let n0 = normalize(in.up);
    let dpx = dpdx(in.world_pos);
    let dpy = dpdy(in.world_pos);
    let r1 = cross(dpy, n0);
    let r2 = cross(n0, dpx);
    let det = dot(dpx, r1);
    let grad = sign(det) * (dpdx(h) * r1 + dpdy(h) * r2);
    let nb = normalize(abs(det) * n0 - grad);
    // The body the volumetric clouds draw: the weather shapes the cover as theirs (the same
    // `weather_cover`), and within their far cross-fade the shell gives way to them.
    var mul = 1.0;
    var add = 0.0;
    var share = 1.0;
    if cloud_sh.key.z > 0.5 && abs(cloud_sh.key.x - clouds.planet.w) < 0.5 && abs(cloud_sh.key.y - clouds.proc2.w) < 1e-4 {
        share = cloud_sh.sun.w;
        for (var i = 0; i < 4; i++) {
            let lb = cloud_sh.layer_b[i];
            if abs(lb.w - clouds.proc2.y - 1.0) < 0.5 {
                // The layer's frame back to the body's (the weather's).
                let q = vec3<f32>(lb.x * cdir.x + lb.y * cdir.z, cdir.y, lb.x * cdir.z - lb.y * cdir.x);
                let w = textureSampleLevel(cloud_weather, cloud_weather_smp, q, 0.0);
                mul = mix(1.0, w.r * 2.0, cloud_sh.key.w);
                add = (w.g * 2.0 - 1.0) * cloud_sh.key.w;
            }
        }
    }
    // SE drops texels under 5% and rescales the rest (`cloudsFade`).
    let cover = weather_cover(se_cut(density), mul, add, clouds.params.x);
    if cover <= 0.0 || share <= 0.0 {
        discard;
    }
    // SE fades the layer out within 1 km of the camera (its planet units, planet.glsl).
    let km_per_unit = clouds.planet.w / max(in.centre.w, 1e-30) * 0.001;
    let near = smoothstep(0.0, 1.0, length(in.world_pos) * km_per_unit);
    let a = cover * near;

    let rel = in.world_pos - in.centre.xyz;
    let shadow = eclipse_light(lit[clouds.slot].eclipse[0], rel / max(in.centre.w, 1e-30), l);
    // Star light above the atmosphere, in the tables' scale.
    let e_top = lit[clouds.slot].sun_color.rgb * lit[clouds.slot].sun_dir.w * shadow * clouds.atm_flags.y;
    let mu_s = dot(up, l);
    // The planet's shadow: lit until the star sets below the horizon at the cloud's height.
    let rr = clamp(in.centre.w / max(length(rel), 1e-30), 0.0, 1.0);
    let hor = clamp((sqrt(1.0 - rr * rr) + mu_s) * 500.0, 0.0, 1.0);
    var e_sun = e_top * hor;
    var sky = vec3<f32>(0.0);
    let to_table = clouds.atm.radii.x / max(in.centre.w, 1e-30);
    let pos_km = rel * to_table;
    let has_atm = lit[clouds.slot].flags.x > 0.5;
    if has_atm {
        let r = length(pos_km);
        e_sun *= atm_transmittance_sun(clouds.atm, transmittance_tex, atm_smp, r, mu_s);
        sky = atm_irradiance(clouds.atm, irradiance_tex, atm_smp, r, mu_s) * clouds.atm.look.w;
    }

    let v = normalize(-in.world_pos);
    let nl = max(dot(nb, l), 0.0);
    let nv = abs(dot(nb, v));
    // Hapke (Lommel-Seeliger) mixed into Lambert by the `Hapke` key.
    let hapke = mix(1.0, 1.0 / (nl + nv + 0.001), clouds.look.w);
    let albedo = clouds.color.rgb * select(vec3<f32>(1.0), texel.rgb, opaque);
    // SE's ambient (day ambient and sky light, not divided by pi, at the ambient's share of the
    // light), only where the cloud is dense.
    var col = albedo / PI * e_sun * nl * hapke + albedo * a * e_top * ATM_AMBIENT_SCALE * (max(mu_s, 0.0) * clouds.look.x + sky);
    // SE's AmbientLighting (its clouds' ambient starts from AmbientColor too), as on the ground.
    col += albedo * a * lit[clouds.slot].levels.x * clouds.atm_flags.y / PI;
    // Subsurface scattering, brightest towards the star (SubsurfScatBright, SubsurfScatPower).
    if clouds.look.y > 0.0 {
        let thickness = 1.0 - a;
        let h = normalize(l + v);
        let forward = exp2(-clamp(dot(v, h), 0.0, 1.0) * clouds.look.z - clouds.look.z);
        let back = clamp(nl * thickness + (1.0 - thickness), 0.0, 1.0) * 0.5;
        col += clouds.look.y * mix(back, 1.0, forward) / PI * e_sun;
    }
    // The further stars, lit the same way (SE's NLights loop).
    for (var i = 0; i < 3; i++) {
        if lit[clouds.slot].light_dir[i].w < 0.5 {
            break;
        }
        let lx = normalize(lit[clouds.slot].light_dir[i].xyz);
        let top_x = lit[clouds.slot].light_color[i].rgb * eclipse_light(lit[clouds.slot].eclipse[i + 1], rel / max(in.centre.w, 1e-30), lx) * clouds.atm_flags.y;
        let mu_x = dot(up, lx);
        var sun_x = top_x * clamp((sqrt(1.0 - rr * rr) + mu_x) * 500.0, 0.0, 1.0);
        var sky_x = vec3<f32>(0.0);
        if has_atm {
            let r = length(pos_km);
            sun_x *= atm_transmittance_sun(clouds.atm, transmittance_tex, atm_smp, r, mu_x);
            sky_x = atm_irradiance(clouds.atm, irradiance_tex, atm_smp, r, mu_x) * clouds.atm.look.w;
        }
        let nl_x = max(dot(nb, lx), 0.0);
        let hapke_x = mix(1.0, 1.0 / (nl_x + nv + 0.001), clouds.look.w);
        col += albedo / PI * sun_x * nl_x * hapke_x + albedo * a * top_x * ATM_AMBIENT_SCALE * (max(mu_x, 0.0) * clouds.look.x + sky_x);
        if clouds.look.y > 0.0 {
            let thickness = 1.0 - a;
            let forward = exp2(-clamp(dot(v, normalize(lx + v)), 0.0, 1.0) * clouds.look.z - clouds.look.z);
            let back = clamp(nl_x * thickness + (1.0 - thickness), 0.0, 1.0) * 0.5;
            col += clouds.look.y * mix(back, 1.0, forward) / PI * sun_x;
        }
    }
    // The haze between the camera and the cloud (the ground's own lies under it).
    if has_atm {
        col = atm_apply_aerial(clouds.atm, inscatter_tex, transmittance_tex, atm_smp, col, -in.centre.xyz * to_table, pos_km, l, e_top * clouds.atm_flags.z);
        for (var i = 0; i < 3; i++) {
            if lit[clouds.slot].light_dir[i].w < 0.5 {
                break;
            }
            let e_x = lit[clouds.slot].light_color[i].rgb * clouds.atm_flags.y * clouds.atm_flags.z;
            col += atm_apply_aerial(clouds.atm, inscatter_tex, transmittance_tex, atm_smp, vec3<f32>(0.0), -in.centre.xyz * to_table, pos_km, normalize(lit[clouds.slot].light_dir[i].xyz), e_x);
        }
    }
    return vec4<f32>(col, a * share);
}
