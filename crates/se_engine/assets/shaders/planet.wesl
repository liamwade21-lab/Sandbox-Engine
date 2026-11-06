// Planet terrain patch: textured with SpaceEngine cube-map tiles, lit through
// the body's precomputed atmosphere when it has one.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::globals;
import super::atmosphere_common::{AtmParams, ATM_AMBIENT_SCALE, atm_surface_irradiance, atm_apply_aerial, atm_transmittance_sun};
import super::eclipse::{EclipseLight, eclipse_light};
import super::cloud_cover::{weather_cover, weather_shaped, climb_dist};
import super::gas_giant::{GiantParams, gas_giant_albedo, gg_fbm, gg_hash, gg_hash1, gg_isf, gg_noise, gg_rotate};
// The light record, the clouds' shadow, the terminator and the haze, shared with the objects on the
// ground (scatter_draw.wesl). PatchLight: the node's lighting (terrain.rs), one record per node in the
// material's light page, at `planet.slot`.
import super::ground_light::{PatchLight, CloudShadow, cloud_shadow_at, horizon_shadow_at, ground_haze};
// The biome weights and their noises, shared with the objects placed on the ground (the scatter's cull
// pass). The wrappers below (`terra_noise`, `se_terra_snow`, `se_terra_weights`, `biome_weights`, ...)
// pass them this material's values.
import super::vegetation::{lin_to_srgb255, veg_classes, map_noise, map_forest, canopy_shade};
import super::settlement::{settlements, st_fields, st_points};
import super::city::{city_pos, city_ground, village_ground, CityGround, ct_dome_mask};
import super::surface_class::{TerraMix, hash31, vnoise, fbm3, ter_oct, ter_fbm, ter_fbm3d, terra_noise_at, terra_climate_noise_at, snow_vary_at, se_terra_snow_at, se_terra_climate_at, plants_ptf_at, se_terra_weights_at, designed_noise_at, biome_weights_at};

// The fixed part of the material. The components marked "slot" are read from `lit`.
struct Planet {
    planet: vec4<f32>,
    rect_color: vec4<f32>,
    rect_lights: vec4<f32>,
    rect_water: vec4<f32>,
    rect_clouds: vec4<f32>,
    // rgb fallback albedo; w (slot) ambient.
    base_color: vec4<f32>,
    // x (slot) glow brightness, y water specular, z cloud opacity, w water roughness.
    params: vec4<f32>,
    // rgb SE's WaterSurfColor (linear), w 1 when the body has water.
    ocean_sea: vec4<f32>,
    // SE's WaterFogColor: the water's absorption, rgb and opacity, per km.
    water_fog: vec4<f32>,
    // x: (slot) atmosphere enabled, y: sun irradiance scale for the tables (SE hdrPlanetSurfaceBright),
    // z: the scattered light (haze, the sky seen in water) relative to y: Bright x hdrAtmoScatterBright / y
    atm_flags: vec4<f32>,
    proc_pal0: vec4<f32>,
    proc_pal1: vec4<f32>,
    proc_pal2: vec4<f32>,
    proc_pal3: vec4<f32>,
    // x: surface kind id, y: relief (m), z: temperature (K), w: has life
    proc_params: vec4<f32>,
    // x: radius (m), y: seed fraction, z: procedural colour enabled, w: SE's volatiles class
    proc_params2: vec4<f32>,
    atm: AtmParams,
    // Own rings: x inner, y outer radius (planet radii), z opacity, w 0 none / 0.5 untextured / 1 textured.
    ring: vec4<f32>,
    // xyz: ring plane normal (render orientation).
    ring_normal: vec4<f32>,
    // xy: node corner on its cube face (m, wrapped), z: node edge (m); for soil textures.
    // w (slot): split distance in node edges, for the geomorph (0: none).
    soil_tile: vec4<f32>,
    // xyz: the node's +u axis (body frame), for the waves; w: 1 on the water surface material.
    water: vec4<f32>,
    // rgb: SE's ModulateColor x ModulateBright on the colour map; w: Hapke (0 Lambert .. 1 Hapke BRDF).
    modulate: vec4<f32>,
    // SE's HapkeParams: SpotBright, 1 / SpotWidth, SpotBrightCB, 1 / SpotWidthCB.
    hapke: vec4<f32>,
    // SE's thermal emission: x base temperature (kK), y lava coverage, z subsolar sun heating
    // (kK^4), w 1 for giants.
    thermal: vec4<f32>,
    // x star surface radiance per its irradiance here (1 / (pi (R*/d)^2)), y star temperature (K),
    // z ThermalEmissionShift (kK), w (slot) self-emission scale (EnableSelfEmission x SelfEmissionBright).
    thermal2: vec4<f32>,
    // SE's GlowColor: rgb the glow map's tint, w the mode (1 Night, 0 Permanent).
    glow: vec4<f32>,
    // SE's ice glint: x SpecBrightIce, y RoughnessIce, z 1 when the colour map's alpha is the ice mask;
    // w (slot) SE's WaterSurfColor.a, the water display (ShowWater) level.
    ice: vec4<f32>,
    // A generated planet's civilization (settlement.wesl): x level (0 none), y culture seed, z its
    // lights' brightness, w its liking for the coast; its lights' colour; its towns' colour by day.
    civ: vec4<f32>,
    civ_light: vec4<f32>,
    civ_urban: vec4<f32>,
    // The city layout's frame (city.wesl): the node corner's 4,096 m tile of face metres and the cube
    // face; the true metres per face metre along u and v, the culture's grid turn.
    city_tile: vec4<i32>,
    city_frame: vec4<f32>,
    // x the ground's lowest point (m above the radius), y its height range (m)
    city_land: vec4<f32>,
    // The node's record in `lit`.
    slot: u32,
    // The body's biome tints, sample layers and soils (`terrain.rs` `PlanetUniform::biome`).
    biomes: Biomes,
};

// Biome texturing of the body (see surface_tex.rs). Slots: 0 bottom, 1 shelf, 2 beach,
// 3 lowland, 4 upland, 5 rock, 6 snow, 7 ice, 8 lava, 9 steppe, 10 grass, 11 forest.
struct Biomes {
    // rgb tint (linear)
    color: array<vec4<f32>, 12>,
    // x array layer (-1 none), y period (m), z strength, w share of the texture's own colour
    large: array<vec4<f32>, 12>,
    small: array<vec4<f32>, 12>,
    // soils: x detail layer (-1 none), y period (m), z modulation layer, w period (m)
    soil: array<vec4<f32>, 32>,
    // per soil: the modulation layer's share in x colour, y normals, z roughness, w height
    soil_mod: array<vec4<f32>, 32>,
    // per soil: x BiasRough, y ScaleRough, z and w the detail and Modul layers' rotation flags
    soil_bias: array<vec4<f32>, 32>,
    // per soil: xyz its Color in HSV (SE's rgb2hsl of the stored colour), w its Slope
    soil_color: array<vec4<f32>, 32>,
    // per soil: x ScaleHeight, y BiasHeight, z the displacement (m) of a unit of height (0: none)
    soil_height: array<vec4<f32>, 32>,
    // x biomes on, y soils on, z vegetation on, w number of soils
    flags: vec4<f32>,
    // SE climate parameters (surface_tex::climate_uniform); climate[4].w 0 when unused.
    climate: array<vec4<f32>, 12>,
    // xyz SE's ColorVary (linear), w 1 to apply it
    color_vary: vec4<f32>,
    // SE's gas giant clouds: the preset's CloudsPalette (gamma encoded) and parameters, see
    // `surface_tex::BiomeUniform::giant` (giant[1].y 1 when on).
    giant_palette: array<vec4<f32>, 8>,
    giant: array<vec4<f32>, 5>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> planet: Planet;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var color_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var color_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var lights_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var lights_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var water_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var water_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var clouds_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var clouds_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var inscatter_tex: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var atm_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var transmittance_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(12) var irradiance_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(13) var ring_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(14) var ring_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(16) var sample_tex: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(17) var sample_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(18) var soil_tex: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(19) var soil_smp: sampler;
// The node's normals (body frame, 0..1 encoded), finer than its mesh.
@group(constants::MATERIAL_BIND_GROUP) @binding(20) var normal_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(21) var normal_smp: sampler;
// The soils' normal (BC5 xy) and roughness / height / occlusion maps, layers as in soil_tex.
@group(constants::MATERIAL_BIND_GROUP) @binding(22) var soil_norm_tex: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(23) var soil_rhao_tex: texture_2d_array<f32>;
// The sample layers' relief: normal xy (rg, x along u), roughness (b), height (a).
@group(constants::MATERIAL_BIND_GROUP) @binding(24) var sample_relief_tex: texture_2d_array<f32>;
// SE's 64^3 Noise3DSampler (textureLoad, repeating), as the height generator reads it.
@group(constants::MATERIAL_BIND_GROUP) @binding(25) var noise_tex: texture_3d<f32>;
// The light page holding this node's PatchLight (at `planet.slot`).
@group(constants::MATERIAL_BIND_GROUP) @binding(26) var<storage, read> lit: array<PatchLight>;

// The volumetric clouds' shadow on the ground of the body they draw (volumetric_clouds.rs
// `CloudShadow`): their cover map (a cube, a channel per layer, each in its layer's frame) and the
// layers.
@group(constants::MATERIAL_BIND_GROUP) @binding(27) var cloud_cover: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(28) var cloud_cover_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(29) var<storage, read> cloud_sh: CloudShadow;
@group(constants::MATERIAL_BIND_GROUP) @binding(30) var cloud_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(31) var cloud_map_smp: sampler;
// The body's weather (volumetric_clouds weather_sim.wesl `sim_out`): R the cover multiplier / 2, G
// the cover offset x 0.5 + 0.5.
@group(constants::MATERIAL_BIND_GROUP) @binding(32) var cloud_weather: texture_cube<f32>;
// The weather's own cloud's pattern (volumetric_clouds.wesl `weather_pattern`), with the cover's sampler.
@group(constants::MATERIAL_BIND_GROUP) @binding(33) var cloud_pattern: texture_cube<f32>;

// The light left by the volumetric clouds over the body-frame direction `bdir` at `height` m: near
// the camera their shadow map (traced through the clouds themselves), farther out each layer's cover
// where the star's ray crosses the layer's cloud mass, as the map's mean.
fn cloud_shadow(bdir: vec3<f32>, height: f32) -> f32 {
    return cloud_shadow_at(cloud_sh, cloud_cover, cloud_cover_smp, cloud_map, cloud_map_smp, cloud_weather, cloud_pattern, planet.planet.w, planet.proc_params2.xy, bdir, height);
}

const PI: f32 = 3.14159265359;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) height: f32,
    @location(4) body_dir: vec3<f32>,
    // Offset to the parent node's surface (xyz, m) and its height difference (w).
    @location(5) morph: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // Direction from planet centre (world), for smooth terminator lighting.
    @location(3) up: vec3<f32>,
    @location(4) height: f32,
    // xyz planet centre (render space), w planet radius (render units).
    @location(5) @interpolate(flat) centre: vec4<f32>,
    @location(6) body_dir: vec3<f32>,
    // Camera height above sea level (m), negative under water.
    @location(7) @interpolate(flat) cam_alt: f32,
    // The node's +u axis (world), for the waves.
    @location(8) @interpolate(flat) tangent_u: vec3<f32>,
    // The node frame's axes in world space (unit), for the normal map.
    @location(9) @interpolate(flat) axis_x: vec3<f32>,
    @location(10) @interpolate(flat) axis_y: vec3<f32>,
    @location(11) @interpolate(flat) axis_z: vec3<f32>,
    // The node frame's scale along those axes (render units per node unit): the body's ellipsoid.
    @location(12) @interpolate(flat) axis_len: vec3<f32>,
    // The airless worlds' kilometre-scale colour noises (`selena_noise`), on nodes fine enough to
    // carry them at their vertices (terrain.rs `vertex_noise`).
    @if(SELENA_AT_VERTICES) @location(13) selena: vec3<f32>,
    // The same for SE's terra worlds (terrain.rs `vertex_noise`): xyz `terra_noise`, w with
    // `TERRA_CLIMATE_AT_VERTICES` `terra_climate_noise`. A world has one generator, so never both.
    @if(TERRA_AT_VERTICES) @location(13) terra: vec4<f32>,
    // With `TERRA_PARTIAL` (coarser nodes): `terra` holds the first `vertex_octaves` of each of
    // `terra_noise`'s sums, and this their shared warp (`terra_noise`'s `avd`).
    @if(TERRA_PARTIAL) @location(14) terra_avd: vec3<f32>,
    // How many octaves of each sum the vertices took (the same at all of a node's vertices; passed
    // on so the pixels never disagree with them by an octave at a rounding edge).
    @if(TERRA_PARTIAL) @location(15) @interpolate(flat) terra_k: vec3<f32>,
};

// Camera height above the sphere of radius planet.w (m). Exact to centimetres: worked out from
// the camera in node-local metres (small near the camera) rather than from planet-scale floats.
// The node centre lies on that sphere. The node's axes are the body's, scaled per axis by its
// ellipsoid (MeterScale), so the inverse divides each axis by its own squared scale.
fn camera_local(world_from_local: mat4x4<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(world_from_local[0].xyz, world_from_local[1].xyz, world_from_local[2].xyz);
    let s2 = vec3<f32>(dot(m[0], m[0]), dot(m[1], m[1]), dot(m[2], m[2]));
    return (transpose(m) * -world_from_local[3].xyz) / s2;
}

fn camera_altitude(world_from_local: mat4x4<f32>) -> f32 {
    let x = camera_local(world_from_local);
    let r = planet.planet.w;
    let a = dot(normalize(-planet.planet.xyz), x);
    let b2 = max(dot(x, x) - a * a, 0.0);
    return (2.0 * r * a + a * a + b2) / (sqrt((r + a) * (r + a) + b2) + r);
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let world_from_local = get_world_from_local(v.instance_index);
    // Geomorph: on the parent's shape where the node has just split from it (twice the split
    // distance), its own from 1.4 times the split distance in.
    var morph = 1.0;
    let split_m = planet.soil_tile.z * lit[planet.slot].flags.y;
    if split_m > 0.0 {
        let d = length(v.position - camera_local(world_from_local));
        morph = clamp((2.0 * split_m - d) / (0.6 * split_m), 0.0, 1.0);
    }
    var local = v.position + v.morph.xyz * (1.0 - morph);
    var height = v.height + v.morph.w * (1.0 - morph);
    // SE's detail displacement ("3D stones", planet.glsl DET + DISP): within the soils' 2 km the
    // ground rises and falls by the soils' height maps. Not on the water surface.
    if planet.biomes.flags.y > 0.5 && planet.water.w < 0.5 {
        let scale = length(world_from_local[0].xyz);
        let w0 = (world_from_local * vec4<f32>(local, 1.0)).xyz;
        let fade = clamp(2.0 - length(w0) / max(scale, 1e-30) / 1000.0, 0.0, 1.0);
        if fade > 0.0 {
            let up_l = normalize(local - planet.planet.xyz);
            let slope = 1.0 - clamp(dot(normalize(v.normal), up_l), 0.0, 1.0);
            let l = normalize(lit[planet.slot].sun_dir.xyz);
            g_sun_body = normalize(vec3<f32>(
                dot(normalize(world_from_local[0].xyz), l),
                dot(normalize(world_from_local[1].xyz), l),
                dot(normalize(world_from_local[2].xyz), l)) + vec3<f32>(1e-12));
            let spacing = planet.soil_tile.z / 32.0;
            let ground = soil_ground_v(v.uv, normalize(v.body_dir), height, slope, spacing);
            let xy = planet.soil_tile.xy + v.uv * planet.soil_tile.z;
            // Read at the vertex spacing, the parent's where the node is still on its shape.
            let d = soil_disp_v(xy, spacing, 1.0 - morph, ground, slope) * fade;
            local += up_l * d;
            height += d;
        }
    }
    let world = mesh_position_local_to_world(world_from_local, vec4<f32>(local, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_pos = world.xyz;
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.uv = v.uv;
    let centre = (world_from_local * vec4<f32>(planet.planet.xyz, 1.0)).xyz;
    o.up = normalize(world.xyz - centre);
    o.height = height;
    o.axis_x = normalize(world_from_local[0].xyz);
    o.axis_y = normalize(world_from_local[1].xyz);
    o.axis_z = normalize(world_from_local[2].xyz);
    o.axis_len = vec3<f32>(length(world_from_local[0].xyz), length(world_from_local[1].xyz), length(world_from_local[2].xyz));
    o.centre = vec4<f32>(centre, planet.planet.w * length(world_from_local[0].xyz));
    o.body_dir = v.body_dir;
    // All octaves: on these nodes every pixel keeps them.
    @if(SELENA_AT_VERTICES) {
        o.selena = selena_noise(normalize(v.body_dir), 0.0);
    }
    @if(TERRA_AT_VERTICES) {
        @if(!TERRA_PARTIAL) {
            o.terra = vec4<f32>(terra_noise(normalize(v.body_dir), 0.0), 0.0);
        }
        @if(TERRA_PARTIAL) {
            let tp = terra_noise_coarse(normalize(v.body_dir));
            o.terra = vec4<f32>(tp.sums, 0.0);
            o.terra_avd = tp.avd;
            o.terra_k = tp.k;
        }
        @if(TERRA_CLIMATE_AT_VERTICES) {
            o.terra.w = terra_climate_noise(normalize(v.body_dir), 0.0);
        }
    }
    o.cam_alt = camera_altitude(world_from_local);
    o.tangent_u = normalize((world_from_local * vec4<f32>(planet.water.xyz, 0.0)).xyz);
    return o;
}

// ---------------------------------------------------------------------------
// Procedural surface colour (bodies without texture maps). `hash31`, `vnoise` and `fbm3` are in
// surface_class.wesl.

// fBm whose octaves fade out as their feature size (m) approaches the pixel
// footprint (m), preventing sub-pixel noise from aliasing into speckle.
fn fbm3_aa(p: vec3<f32>, octaves: i32, feature_m: f32, footprint_m: f32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    var size = feature_m;
    for (var o = 0; o < octaves; o = o + 1) {
        let w = 1.0 - smoothstep(0.25, 1.0, footprint_m / size);
        if w <= 0.0 {
            break;
        }
        sum += amp * w * vnoise(q);
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
        amp *= 0.5;
        size *= 0.5;
    }
    return sum;
}

fn proc_albedo(dir: vec3<f32>, height: f32, slope: f32, footprint_m: f32) -> vec3<f32> {
    let kind = planet.proc_params.x;
    let relief = max(planet.proc_params.y, 1.0);
    let temp = planet.proc_params.z;
    let radius = planet.proc_params2.x;
    let seed = vec3<f32>(planet.proc_params2.y * 917.0, planet.proc_params2.y * 331.0, planet.proc_params2.y * 613.0);
    let p0 = planet.proc_pal0.rgb;
    let p1 = planet.proc_pal1.rgb;
    let p2 = planet.proc_pal2.rgb;
    let p3 = planet.proc_pal3.rgb;
    if kind > 5.5 && kind < 7.5 {
        // Giants: latitude bands distorted by turbulence.
        let lat = dir.y;
        let turb = fbm3_aa(dir * 3.0 + seed, 6, radius / 3.0, footprint_m);
        let fine = fbm3_aa(dir * 18.0 + seed, 4, radius / 18.0, footprint_m);
        var freq = 21.0;
        if kind > 6.5 {
            freq = 7.0;
        }
        let band = sin((lat + turb * 0.06) * freq + seed.x);
        let band2 = sin((lat + turb * 0.04 + fine * 0.01) * freq * 2.3 + seed.y);
        var c = mix(p0, p2, band * 0.5 + 0.5);
        c = mix(c, p1, (band2 * 0.5 + 0.5) * 0.45);
        c = mix(c, p3, smoothstep(0.7, 0.97, abs(lat)));
        return c * (0.92 + 0.16 * fine);
    }
    let h = height / relief;
    let n_mid = fbm3_aa(dir * (radius / 40000.0) + seed, 5, 40000.0, footprint_m);
    let n_fine = fbm3_aa(dir * (radius / 1500.0) + seed, 6, 1500.0, footprint_m);
    var c = mix(p0, p1, smoothstep(-0.05, 0.35, h + n_mid * 0.2));
    c = mix(c, p2, smoothstep(0.35, 0.8, h + n_mid * 0.1));
    // Bare rock on steep slopes.
    c = mix(c, p2 * 0.8, smoothstep(0.2, 0.55, slope));
    // Ice caps and snow on cold worlds / high peaks.
    if temp < 300.0 {
        let cold = clamp((300.0 - temp) / 120.0, 0.0, 1.0);
        let polar = smoothstep(0.9 - 0.5 * cold, 0.97 - 0.4 * cold, abs(dir.y) + n_mid * 0.06);
        let peaks = smoothstep(0.7, 0.95, h + n_fine * 0.05) * (0.3 + 0.7 * cold);
        c = mix(c, p3, clamp(max(polar, peaks) * (1.0 - smoothstep(0.35, 0.6, slope) * 0.6), 0.0, 1.0));
    }
    return c * (0.85 + 0.3 * (n_fine * 0.5 + 0.5));
}

// SE's thermal emission (planet.glsl THERM). The temperature map (tg_*_glow): giants glow all over
// at surfTemperature x (1 - 0.2 height); rocky worlds where lava covers them, surfTemperature x
// (0.95 - 0.08 |fbm| + 0.08 |fbm|) x saturate(2 (lava 0.4 + 0.4 - 0.8 height)) x saturate((lava -
// 0.01) 25). Then the sun's heating, T = (heat n.l + T^4)^(1/4), and the colour of a blackbody at T:
// SE samples its Planck texture; here the radiance ratio to the star's surface at 610/550/465 nm,
// times the star's surface radiance as the port's irradiance scale implies (white balanced to the
// star, as the port's light is).
fn thermal_emission(dir: vec3<f32>, height: f32, nl: f32, e_sun: vec3<f32>) -> vec3<f32> {
    let t0 = planet.thermal.x;
    let h = clamp(height / max(planet.proc_params.y, 1.0), 0.0, 1.0);
    var t = 0.0;
    if planet.thermal.w > 0.5 {
        t = (1.0 - 0.2 * h) * t0;
    } else if planet.thermal.y > 0.01 {
        let lava = planet.thermal.y;
        let s = planet.proc_params2.y;
        let p = dir * 600.0 + vec3<f32>(s * 113.0, s * 71.0, s * 29.0);
        let dist = 10.0 * (fbm3(p * 0.2, 5) * 2.0 - 1.0);
        let glob = 0.95 - abs(fbm3((p + dist) * 0.01, 3) * 2.0 - 1.0) * 0.08;
        let vary = abs(fbm3(p + dist, 8) * 2.0 - 1.0);
        t = t0 * (glob + vary * 0.08) * clamp(2.0 * (lava * 0.4 + 0.4 - 0.8 * h), 0.0, 1.0) * clamp((lava - 0.01) * 25.0, 0.0, 1.0);
    }
    let t4 = planet.thermal.z * max(nl, 0.0) + t * t * t * t;
    let tk = (sqrt(sqrt(t4)) + planet.thermal2.z) * 1000.0;
    // Below 450 K nothing visible glows.
    if tk < 450.0 {
        return vec3<f32>(0.0);
    }
    let ts = planet.thermal2.y;
    let c = vec3<f32>(23587.0, 26160.0, 30942.0);
    let ratio = exp(c * (1.0 / ts - 1.0 / tk)) * (1.0 - exp(-c / ts)) / (1.0 - exp(-c / tk));
    return luma(e_sun) * planet.thermal2.x * ratio;
}

// ---------------------------------------------------------------------------
// Biome and soil detail textures.

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// SE's anti-tiling (`DetailTextureNoTiled`, TILING_FIX_MODE 4, tg_common.glh:611-649 and
// tg_detail_maps.glsl): a smooth noise, one random value a repeat, picks one of 8 zones, each the
// texture shifted and turned and, from zone 4, flipped; neighbouring zones blend. Zone axes:
// uv' = S R(a) uv, S swapping x and y when `flip`.
fn zone_xf(v: vec2<f32>, a: f32, flip: bool) -> vec2<f32> {
    let s = sin(a);
    let c = cos(a);
    let r = vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
    return select(r, r.yx, flip);
}

// A gradient or a normal's tilt in zone axes, back in the texture's own (the transpose).
fn zone_back(g: vec2<f32>, a: f32, flip: bool) -> vec2<f32> {
    let f = select(g, g.yx, flip);
    let s = sin(a);
    let c = cos(a);
    return vec2<f32>(c * f.x + s * f.y, -s * f.x + c * f.y);
}

// The biome sample layers' zones: shifted up to half a repeat, turned by the zone's number in radians.
fn zone_uv(uv: vec2<f32>, z: f32) -> vec2<f32> {
    return zone_xf(uv + sin(vec2<f32>(3.0, 7.0) * z) * 0.5, z, z >= 4.0);
}

fn zone_sample(layer: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, z: f32) -> vec4<f32> {
    return textureSampleGrad(sample_tex, sample_smp, zone_uv(uv, z), layer, zone_xf(dx, z, z >= 4.0), zone_xf(dy, z, z >= 4.0));
}

fn zone_relief(layer: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, z: f32) -> vec3<f32> {
    let r = textureSampleGrad(sample_relief_tex, sample_smp, zone_uv(uv, z), layer, zone_xf(dx, z, z >= 4.0), zone_xf(dy, z, z >= 4.0));
    return vec3<f32>(zone_back(relief_grad(r.xy), z, z >= 4.0), r.z);
}

// One biome sample layer in zone `z`, triplanar in body space (planes of weight 0 are not read).
fn triplanar(layer: i32, p: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>, w: vec3<f32>, z: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    if w.x > 0.0 {
        c += zone_sample(layer, p.yz, dx.yz, dy.yz, z).rgb * w.x;
    }
    if w.y > 0.0 {
        c += zone_sample(layer, p.zx, dx.zx, dy.zx, z).rgb * w.y;
    }
    if w.z > 0.0 {
        c += zone_sample(layer, p.xy, dx.xy, dy.xy, z).rgb * w.z;
    }
    return c;
}

// Height gradient (per texture unit) of an encoded relief normal.
fn relief_grad(xy: vec2<f32>) -> vec2<f32> {
    let n = xy * 2.0 - 1.0;
    return -n / sqrt(max(1.0 - dot(n, n), 0.04));
}

// A layer's relief in zone `z`, triplanar: the height gradient in body axes (per texture unit,
// which is per metre per metre of the layer's own scale) and the roughness.
fn triplanar_relief(layer: i32, p: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>, w: vec3<f32>, z: f32) -> vec4<f32> {
    var o = vec4<f32>(0.0);
    if w.x > 0.0 {
        let a = zone_relief(layer, p.yz, dx.yz, dy.yz, z);
        o += vec4<f32>(0.0, a.x, a.y, a.z) * w.x;
    }
    if w.y > 0.0 {
        let b = zone_relief(layer, p.zx, dx.zx, dy.zx, z);
        o += vec4<f32>(b.y, 0.0, b.x, b.z) * w.y;
    }
    if w.z > 0.0 {
        let c = zone_relief(layer, p.xy, dx.xy, dy.xy, z);
        o += vec4<f32>(c.x, c.y, 0.0, c.z) * w.z;
    }
    return o;
}

// Pixel footprint in body metres, set at the start of `fragment` (derivatives need uniform control flow).
var<private> g_footprint_m: f32 = 1.0;

// A sample layer at a point: colour factor around 1, relief gradient (body axes, m per m),
// roughness and the layer's strength.
struct SampleLook {
    col: vec3<f32>,
    grad: vec3<f32>,
    rough: f32,
    k: f32,
};

// A sample layer's texture coordinates at a point and their screen derivatives. Kept apart from
// the sampling so a caller can take the derivatives in uniform control flow and sample in a branch.
struct LayerPos {
    p: vec3<f32>,
    dx: vec3<f32>,
    dy: vec3<f32>,
};

fn layer_pos(l: vec4<f32>, pos_m: vec3<f32>) -> LayerPos {
    // A slow warp bends the texture grid so its repeats do not line up into a visible pattern.
    let p0 = pos_m / l.y;
    var o: LayerPos;
    o.p = p0 + vec3<f32>(vnoise(p0 * 0.07 + vec3<f32>(3.1)), vnoise(p0 * 0.07 + vec3<f32>(7.7)), vnoise(p0 * 0.07 + vec3<f32>(1.3))) * 1.2;
    o.dx = dpdx(o.p);
    o.dy = dpdy(o.p);
    return o;
}

// As a factor around 1: the texture divided by its mean (the 1x1 mip), so far away (where the
// mips average it) it fades to 1 and never shifts the albedo. `lp` is `layer_pos(l, ..)`.
fn sample_layer(l: vec4<f32>, lp: LayerPos, w: vec3<f32>) -> SampleLook {
    let layer = i32(max(l.x, 0.0));
    let p = lp.p;
    let dx = lp.dx;
    let dy = lp.dy;
    // A layer whose repeat covers only a few dozen pixels would show as a grid (SE's surfaces are
    // smooth from orbit): its contrast fades in as the repeat grows on screen.
    let on_screen = smoothstep(48.0, 384.0, l.y / max(g_footprint_m, 1.0e-3));
    let k = select(0.0, l.z, l.x >= 0.0) * on_screen;
    // Seen closer than its texels (512 a period), a layer shows its bilinear blobs: its relief
    // fades out from 2 to 8 screen pixels a texel and its colour to half (SE's ground is plain
    // between the soils' metres and the samples' scale, a few hundred metres up).
    let close = 1.0 - smoothstep(2.0, 8.0, l.y / 512.0 / max(g_footprint_m, 1.0e-4));
    var o: SampleLook;
    o.col = vec3<f32>(1.0);
    o.grad = vec3<f32>(0.0);
    o.rough = 1.0;
    o.k = k;
    // Nothing to read where the layer has faded out (its roughness counts by `k`).
    if k <= 0.0 {
        return o;
    }
    // Planes under 2% go (the rest renormalised, so nothing jumps): most points read one or two.
    let w2 = max(w - vec3<f32>(0.02), vec3<f32>(0.0));
    let tw = w2 / max(w2.x + w2.y + w2.z, 1e-6);
    let kz = 8.0 * clamp(vnoise(p) * 0.5 + 0.5, 0.0, 0.9999);
    let za = floor(kz);
    let fr = kz - za;
    // One zone where the blend has not begun or is done: its colour term moves it by 0.1 at most
    // for any but very different texels.
    var ca = vec3<f32>(0.0);
    var cb = vec3<f32>(0.0);
    if fr < 0.9 {
        ca = triplanar(layer, p, dx, dy, tw, za);
    }
    if fr >= 0.1 {
        cb = triplanar(layer, p, dx, dy, tw, za + 1.0);
    }
    var m = select(0.0, 1.0, fr >= 0.9);
    if fr >= 0.1 && fr < 0.9 {
        m = smoothstep(0.2, 0.8, fr - 0.1 * dot(ca - cb, vec3<f32>(1.0)));
    }
    let t = mix(ca, cb, m);
    let mean = textureSampleLevel(sample_tex, sample_smp, vec2<f32>(0.5), layer, 20.0).rgb;
    let n = t / max(mean, vec3<f32>(0.02));
    let f = mix(vec3<f32>(luma(n)), n, l.w);
    o.col = mix(vec3<f32>(1.0), f, k * mix(0.5, 1.0, close));
    // Up close the relief has faded and the soils' roughness takes over (where there are soils).
    if close > 0.0 || planet.biomes.flags.y < 0.5 {
        var ra = vec4<f32>(0.0);
        var rb = vec4<f32>(0.0);
        if m < 1.0 {
            ra = triplanar_relief(layer, p, dx, dy, tw, za);
        }
        if m > 0.0 {
            rb = triplanar_relief(layer, p, dx, dy, tw, za + 1.0);
        }
        o.grad = mix(ra.xyz, rb.xyz, m) * k * close;
        o.rough = mix(ra.w, rb.w, m);
    }
    return o;
}

// Relative weights of the 12 biome slots (designed worlds): surface_class.wesl `biome_weights_at`
// with this material's values. SE's terrain noises (`ter_oct`, `ter_fbm`, `ter_fbm3d`, `ter_rmf`) are
// there too.
fn biome_weights(h: f32, slope: f32, lat: f32, n: f32, moist: f32) -> array<f32, 12> {
    return biome_weights_at(h, slope, lat, n, moist, planet.proc_params, planet.ocean_sea.w > 0.5, planet.biomes.flags.z > 0.5);
}

// ---------------------------------------------------------------------------
// SE's airless colour noises (tg_common.glh), on SE's noise texture and hash Perlin as the height
// generator (tg_height.wesl) reads them, so the colour's craters, cracks and freckles fall on its.

fn se_noise_texel(c: vec3<f32>) -> vec4<f32> {
    let w = c - 64.0 * floor(c / 64.0);
    return textureLoad(noise_tex, vec3<i32>(w), 0);
}

// Cell3Noise2Color: the two nearest feature distances over the 3x3x3 cells, and the nearest one's
// random colour.
fn se_cell3_noise2_color(p: vec3<f32>, color: ptr<function, vec4<f32>>) -> vec2<f32> {
    let cell = floor(p);
    let offs = p - cell;
    var d1 = 1.0e38;
    var d2 = 1.0e38;
    var best = vec4<f32>(1.0);
    for (var z = -1; z < 2; z++) {
        for (var y = -1; y < 2; y++) {
            for (var x = -1; x < 2; x++) {
                let d = vec3<f32>(f32(x), f32(y), f32(z));
                let rnd = se_noise_texel(cell + d);
                let pos = rnd.xyz + d - offs;
                let dist = dot(pos, pos);
                if dist < d1 {
                    d2 = d1;
                    d1 = dist;
                    best = rnd;
                } else {
                    d2 = min(d2, dist);
                }
            }
        }
    }
    *color = best;
    return sqrt(vec2<f32>(d1, d2));
}

// Cell2Noise2: the same over the 2x2x2 cells around the nearest corner.
fn se_cell2_noise2(p: vec3<f32>) -> vec2<f32> {
    let cell = floor(p);
    let offs = p - cell - vec3<f32>(0.5);
    var d1 = 1.0e38;
    var d2 = 1.0e38;
    for (var z = -1; z < 1; z++) {
        for (var y = -1; y < 1; y++) {
            for (var x = -1; x < 1; x++) {
                let d = vec3<f32>(f32(x), f32(y), f32(z));
                let pos = se_noise_texel(cell + d).xyz + d - offs;
                let dist = dot(pos, pos);
                if dist < d1 {
                    d2 = d1;
                    d1 = dist;
                } else {
                    d2 = min(d2, dist);
                }
            }
        }
    }
    return sqrt(vec2<f32>(d1, d2));
}

// SE's NoiseDeriv (hash Perlin surflets): xyz the derivatives, w the noise.
fn se_noise_deriv(p: vec3<f32>) -> vec4<f32> {
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf1 = Pf - 1.0;
    let h = gg_hash(Pi);
    let C = vec4<f32>(0.49999);
    var gx0 = h[0] - C;
    var gy0 = h[1] - C;
    var gz0 = h[2] - C;
    let n0 = inverseSqrt(gx0 * gx0 + gy0 * gy0 + gz0 * gz0);
    gx0 *= n0;
    gy0 *= n0;
    gz0 *= n0;
    var gx1 = h[3] - C;
    var gy1 = h[4] - C;
    var gz1 = h[5] - C;
    let n1 = inverseSqrt(gx1 * gx1 + gy1 * gy1 + gz1 * gz1);
    gx1 *= n1;
    gy1 *= n1;
    gz1 *= n1;
    let px = vec2<f32>(Pf.x, Pf1.x).xyxy;
    let py = vec2<f32>(Pf.y, Pf1.y).xxyy;
    let r0 = px * gx0 + py * gy0 + Pf.zzzz * gz0;
    let r1 = px * gx1 + py * gy1 + Pf1.zzzz * gz1;
    let sq = Pf * Pf;
    let sq1 = Pf1 * Pf1;
    let lsq = vec2<f32>(sq.x, sq1.x).xyxy + vec2<f32>(sq.y, sq1.y).xxyy;
    let m0 = max(1.0 - (lsq + sq.zzzz), vec4<f32>(0.0));
    let m20 = m0 * m0;
    let m30 = m0 * m20;
    let m1 = max(1.0 - (lsq + sq1.zzzz), vec4<f32>(0.0));
    let m21 = m1 * m1;
    let m31 = m1 * m21;
    let t0 = -6.0 * m20 * r0;
    let t1 = -6.0 * m21 * r1;
    let d0 = vec3<f32>(dot(t0, px) + dot(m30, gx0), dot(t0, py) + dot(m30, gy0), dot(t0, Pf.zzzz) + dot(m30, gz0));
    let d1 = vec3<f32>(dot(t1, px) + dot(m31, gx1), dot(t1, py) + dot(m31, gy1), dot(t1, Pf1.zzzz) + dot(m31, gz1));
    return vec4<f32>(d0 + d1, dot(m30, r0) + dot(m31, r1)) * 2.3703703703703703;
}

// SE's JordanTurbulence as tg_selena_color calls it for the freckles: gains 0.8 / 0.5, warps
// 0.6 / 0.35, damps 1.0 / 0.8, damp scale 1, lacunarity 2.
fn se_jordan(point: vec3<f32>, octaves: f32) -> f32 {
    // One noise call per pass (the first octave's weights folded in): half the code of SE's form.
    var summ = 0.0;
    var warp = vec3<f32>(0.0);
    var damp = vec3<f32>(0.0);
    var gain_warp = 0.6;
    var gain_damp = 1.0;
    var amp = 0.8;
    var freq = 1.0;
    var damped = 1.0;
    for (var i = 0; f32(i) < max(octaves, 1.0); i++) {
        let nd = se_noise_deriv(point * freq + warp);
        let nd2 = nd * nd.w;
        summ += damped * nd2.w;
        warp += gain_warp * nd2.xyz;
        damp += gain_damp * nd2.xyz;
        gain_warp = 0.35;
        gain_damp = 0.8;
        if i > 0 {
            amp *= 0.5;
        }
        damped = select(amp * (1.0 - 1.0 / (1.0 + dot(damp, damp))), amp * 0.5, i == 0);
        freq *= 2.0;
    }
    return summ;
}

// SE's DistFbm: Fbm of noise displaced by its own NoiseVec3 x `dist`.
fn se_dist_fbm(point: vec3<f32>, dist: f32, octaves: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    let lac = 2.218281828459;
    let gain = pow(lac, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        let q = pt + 0.5;
        let w = vec3<f32>(gg_noise(q), gg_noise(q + vec3<f32>(3.33, 5.71, 1.96)), gg_noise(q + vec3<f32>(7.77, 2.65, 4.37)));
        sum += gg_noise(pt + w * dist) * ampl;
        ampl *= gain;
        pt *= lac;
    }
    return sum;
}

// SE's RayedCraterColorFunc (radRim 0.03, radOuter 0.8): the ejecta at `r` crater radii from the
// centre, `fi` the bearing (turns), `rnd` the crater's seed. `k` 0..1: how far the rays are resolved
// (thinner than a pixel they fade to their mean).
fn se_rayed_color_func(r: f32, fi: f32, rnd: f32, k: f32) -> f32 {
    let t = saturate((0.8 - r) / (0.8 - 0.03));
    let d4 = mix(0.08, pow(gg_noise(vec3<f32>(70.3 * fi, rnd, rnd)) * 0.5 + 0.5, 4.0), k);
    let fade = pow(t, 6.0) * 0.5 + smoothstep(0.03 * 1.1, 0.03 * 0.9, r) * 0.3;
    return fade * pow(saturate(1.0 - smoothstep(d4, d4 + 0.5, r)), 3.5);
}

// SE's RayedCraterColorNoise: the young craters' bright rays and ejecta, over the same spherical
// Fibonacci cells (and per-octave rotations) as the generator's RayedCraterNoise. Octaves whose
// craters are below about a pixel (`fp`) are left out.
fn se_rayed_crater_color(point_in: vec3<f32>, freq: f32, sqrt_density: f32, octaves: f32, fp: f32) -> f32 {
    let rnd = planet.biomes.climate[6].xyz;
    let lac = 2.218281828459;
    var point = point_in;
    let binormal = normalize(vec3<f32>(-point.z, 0.0, point.x));
    let rot = normalize(rnd);
    let shape_dist = 1.0 + 2.5 * 0.03 * ter_fbm(point * 419.54, ter_oct(3.0, 419.54, lac, fp), lac);
    let color_dist = 1.0 - 0.3 * ter_fbm(point * 315.16, ter_oct(3.0, 315.16, lac, fp), lac);
    var color = 0.0;
    var rad_factor = shape_dist / max(sqrt_density, 1e-3);
    var fib = 2.0 * freq;
    for (var i = 0; f32(i) < octaves; i++) {
        // The cells' spacing on the unit sphere, sqrt(4 pi / n), against the pixel.
        if sqrt(4.0 * PI / fib) < 2.0 * fp {
            break;
        }
        var centre = vec3<f32>(0.0);
        let cell = gg_isf(point, fib, &centre);
        let rad = gg_hash1(cell.x * 743.1) * 0.9 + 0.1;
        let fi = acos(clamp(dot(binormal, normalize(centre - point + vec3<f32>(1e-12))), -1.0, 1.0)) / (2.0 * PI);
        // A ray is 1/70 turn wide at its crater's radius, rad / rad_factor.
        let ray_w = 2.0 * PI * rad / rad_factor / 70.3;
        color += se_rayed_color_func(cell.y * rad_factor / rad, fi, 48.3 * dot(centre, rnd), saturate(ray_w / (2.0 * fp) - 0.5));
        if octaves > 1.0 {
            point = gg_rotate(2.0 * PI * gg_hash1(f32(i)), rot, point);
            fib *= 1.8;
            rad_factor *= sqrt(1.8);
        }
    }
    return min(color, 1.0) * color_dist;
}

// SE's CrackHeightFunc in its colour use (height 1): a dark-rimmed line where `r` < 1.
fn se_crack_func(last: f32, lastlast: f32, r: f32, p_in: vec3<f32>, noise_ok: bool) -> f32 {
    var p = p_in;
    p.x += 0.05 * r;
    let inner = smoothstep(0.0, 0.5, r);
    let outer = smoothstep(0.5, 1.0, r);
    let n = select(0.0, gg_noise(p * 625.7), noise_ok);
    let cracks = 0.4 * n * (1.0 - inner) + inner * (1.0 - outer);
    return mix(cracks, mix(last, lastlast, r), outer);
}

// SE's CrackColorNoise: the ice cracks' lines (the colour factor) and `mask`, 0 on them, which keeps
// the freckles off. SE's lines are 1/250 of a cell wide; thinner than a pixel they widen to it and
// fade by the share of it they cover.
fn se_crack_color(point_in: vec3<f32>, fp: f32, mask: ptr<function, f32>) -> f32 {
    let rnd = planet.biomes.climate[6].xyz;
    let lac = 2.218281828459;
    var scale = planet.biomes.climate[9].w;
    var point = (point_in + rnd) * scale;
    var new_land = 0.0;
    var last = 0.0;
    var lastlast = 0.0;
    *mask = 1.0;
    for (var i = 0; f32(i) < planet.biomes.climate[10].x; i++) {
        let w = max(1.0 / 250.0, fp * scale);
        let cover = (1.0 / 250.0) / w;
        let cell = se_cell2_noise2(point + 0.02 * ter_fbm3d(1.8 * point, ter_oct(6.0, 1.8 * scale, lac, fp)));
        let r = smoothstep(0.0, 1.0, abs(cell.y - cell.x) / w);
        lastlast = last;
        last = new_land;
        new_land = mix(last, se_crack_func(lastlast, last, r, point, 625.7 * scale * fp < 0.5), cover);
        point = point * 1.2 + rnd;
        scale *= 1.2;
        *mask *= mix(1.0, smoothstep(0.6, 1.0, r), cover);
    }
    return pow(saturate(1.0 - new_land), 2.0);
}

// The direction to the star in the body frame (set by `lit_ground`): the substellar point that SE's
// colour shaders put on +x for tidally locked worlds.
var<private> g_sun_body: vec3<f32> = vec3<f32>(1.0, 0.0, 0.0);

// The pixel's `VertexOut::selena` and `terra` (set by `fragment`).
@if(SELENA_AT_VERTICES) var<private> g_selena: vec3<f32>;
@if(TERRA_AT_VERTICES) var<private> g_terra: vec4<f32>;
@if(TERRA_PARTIAL) var<private> g_terra_avd: vec3<f32>;
@if(TERRA_PARTIAL) var<private> g_terra_k: vec3<f32>;

// `plants_coeff`, `blend_slot` and `TerraMix` (a point's slot weights and colour terms) are in
// surface_class.wesl.

// The airless worlds' colour noises that vary only over kilometres (tg_selena_color.glsl): x the
// latitude's noise, y the albedo variation's Fbm, z the young terrain's Fbm (0 without young terrain).
fn selena_noise(point: vec3<f32>, fp: f32) -> vec3<f32> {
    let rnd = planet.biomes.climate[6].xyz;
    let lac = 2.218281828459;
    let lat_noise = 0.15 * (ter_fbm(point * 0.7 + rnd, ter_oct(6.0, 0.7, lac, fp), lac) - 1.0);
    let avd = ter_fbm3d((point + rnd) * 0.07, ter_oct(3.0, 0.07, lac, fp)) * (1.5 + planet.biomes.climate[8].y);
    let vary = ter_fbm((point + avd) * 0.378, ter_oct(8.0, 0.378, lac, fp), lac);
    var young = 0.0;
    if planet.biomes.climate[7].w > 0.05 {
        let yd = ter_fbm3d((point - rnd) * 0.07, ter_oct(4.0, 0.07, lac, fp)) * 1.1;
        young = ter_fbm(point + yd, ter_oct(8.0, 1.0, lac, fp), lac);
    }
    return vec3<f32>(lat_noise, vary, young);
}

// SE's ColorMapSelena (tg_selena_color.glsl), airless worlds: the base surface by height bands, an
// albedo variation weakened on slopes and, on cratered worlds, outside younger patches, cut by the
// ice cracks' lines (CrackColorNoise) and the Europa-like freckles (JordanTurbulence in the hills
// biome); the leading/trailing hemisphere darkening (drivenDarkening), the young craters' bright rays
// (RayedCraterColorNoise) and the thin polar frost whitened. SE's asteroids (tg_asteroid_color.glsl)
// take the base surface by a noise instead of the height and darken flat ground (x (0.5 + slope)).
// The shield volcanoes' lava, which only steers SE's rock texture choice, is not ported. Cracks,
// freckles, rays and the asteroid look only on `procedural` worlds: a colour map has its own (the
// Moon's Tycho).
fn se_selena_mix(point: vec3<f32>, h: f32, slope: f32, fp: f32, procedural: bool) -> TerraMix {
    if planet.biomes.climate[7].z > 1.5 && procedural {
        let c0 = planet.biomes.climate[0];
        let rnd = planet.biomes.climate[6].xyz;
        let sea = c0.x;
        let lac = 2.218281828459;
        var o: TerraMix;
        o.rock_tint = vec3<f32>(1.0);
        var w: array<f32, 12>;
        let hn = se_dist_fbm(point * 3.7 + rnd, 1.5, ter_oct(5.0, 3.7, lac, fp));
        let ha = (hn - sea) / max(1.0 - sea, 1e-3) * 4.0 + 2.0;
        let a0 = clamp(i32(floor(ha)), 0, 5);
        w[a0] += 1.0 - fract(ha);
        w[clamp(a0 + 1, 0, 5)] += fract(ha);
        o.w = w;
        o.vary = 1.0;
        // SE's slope is 1 - cos^6 of the tilt (GetSurfaceHeightAndSlope), the port's 1 - cos.
        o.dark = 0.5 + 1.0 - pow(1.0 - slope, 6.0);
        o.white = 0.0;
        return o;
    }
    return se_selena_terms(point, h, slope, fp, procedural, selena_noise(point, fp));
}

// `se_selena_mix` past the asteroids, given `selena_noise` (`n`). The nodes that carry it at their
// vertices (`SELENA_AT_VERTICES`) pass the pixel's interpolated values: their cells are so fine
// against its features that the two differ by under a thousandth (below what 8-bit colour shows),
// and their pixels skip its 43 noises, most of the ground's cost on such worlds.
fn se_selena_terms(point: vec3<f32>, h: f32, slope: f32, fp: f32, procedural: bool, n: vec3<f32>) -> TerraMix {
    let c0 = planet.biomes.climate[0];
    let rnd = planet.biomes.climate[6].xyz;
    let sea = c0.x;
    let lac = 2.218281828459;
    var o: TerraMix;
    o.rock_tint = vec3<f32>(1.0);
    o.dark = 1.0;
    var w: array<f32, 12>;
    let lat_noise = n.x;
    var latitude: f32;
    if planet.biomes.climate[6].w <= 0.0 {
        latitude = saturate(abs(point.y) + lat_noise);
    } else {
        latitude = 1.0 - dot(point, g_sun_body) + lat_noise;
    }
    let hb = (h - sea) / max(1.0 - sea, 1e-3) * 4.0 + 2.0;
    let h0 = clamp(i32(floor(hb)), 0, 5);
    let h1 = clamp(h0 + 1, 0, 5);
    let dh = fract(hb);
    w[h0] += 1.0 - dh;
    w[h1] += dh;
    let slope_mod = 1.0 - slope;
    var vary = saturate(1.0 - n.y * slope_mod * slope_mod * 2.0);
    if planet.biomes.climate[7].w > 0.05 {
        var young = 1.0 - n.z;
        young = smoothstep(0.0, 1.0, young * young * young);
        vary = mix(0.0, vary, young);
    }
    var mask = 1.0;
    if planet.biomes.climate[10].x > 0.0 && procedural {
        vary *= se_crack_color(point, fp, &mask);
    }
    // The freckles, in the generator's hills biome (its Cell3Noise2Color domains by mainFreq).
    let c10 = planet.biomes.climate[10];
    let hills_freq = planet.biomes.climate[7].x;
    if c10.z > c10.y && procedural && ter_oct(10.0, hills_freq, 2.0, fp) > 0.0 {
        let p = point * c10.w + rnd;
        var col: vec4<f32>;
        let cell = se_cell3_noise2_color(p * 2.3 + 13.5 * ter_fbm3d(p * 0.06, ter_oct(6.0, 0.06 * c10.w, lac, fp)), &col);
        if col.r > c10.y && col.r < c10.z {
            let scale = saturate(2.0 * (pow(abs(cell.y - cell.x), 0.7) - 0.05));
            vary *= 1.0 - saturate(2.0 * mask * scale * se_jordan(point * hills_freq + rnd, ter_oct(10.0, hills_freq, 2.0, fp)));
        }
    }
    o.vary = vary;
    let dd = planet.biomes.climate[8].x;
    if dd != 0.0 {
        var z = -point.z * sign(dd) + 0.2 * ter_fbm(point * 1.63, ter_oct(3.0, 1.63, lac, fp), lac);
        z = saturate(1.0 - z);
        z *= z;
        o.dark = mix(1.0 - abs(dd), 1.0, z);
    }
    // The rayed craters (the generator's craterRayed density and octaves), then the frost.
    let c9 = planet.biomes.climate[9];
    let sd = planet.biomes.climate[7].w;
    var rays = 0.0;
    if sd * sd * c9.z > 0.05 * 0.05 && procedural {
        rays = se_rayed_crater_color(point, c9.x, sd * sqrt(c9.z), floor(c9.y * c9.z), fp);
    }
    o.white = 1.0 - (1.0 - rays) * (1.0 - 0.8 * saturate((latitude - c0.z + 0.3) * 2.0));
    o.w = w;
    return o;
}

// SE's ColorMapTerra (tg_terra_color.glsl) as slot weights: the climate from the latitude
// (Fbm-distorted; on tidally locked worlds from the substellar point, +x), the tropic band, the
// height temperature gradient, the snow line and ice caps, then the flatland climate distortion
// (ridged multifractals by hillsFreq / hillsMagn, colorDistFreq); the base surface by height bands
// from the beach at sea level to rock at the top (GetBaseSurface); steppe, forest and grass by their
// climate ranges where humid enough (ModifySurfaceByPlants); snow above climate 0.9; layered rock
// on slopes over 0.35. `h` is the height as a fraction of BumpHeight from the bottom (SE's height map
// value). Cracks and the volcano masks are not ported.
// SE's ColorMapTerra in three stages, so the vertex stage can run it (`se_terra_mix_v`): the
// fragment softens its two sharp transitions by their screen derivatives between them.
fn se_terra_mix(point: vec3<f32>, h: f32, slope: f32, fp: f32) -> TerraMix {
    return se_terra_mix_n(point, h, slope, fp, terra_noise(point, fp), terra_climate_noise(point, fp));
}

// `se_terra_mix` given `terra_noise` (`n`) and `terra_climate_noise` (`cv`). The nodes that carry
// them at their vertices (`TERRA_AT_VERTICES`, `TERRA_CLIMATE_AT_VERTICES`) pass the pixel's
// interpolated values: their cells are so fine against those noises' features that the colour
// differs by far less than 8-bit colour shows, and their pixels skip most of the ground's cost.
fn se_terra_mix_n(point: vec3<f32>, h: f32, slope: f32, fp: f32, n: vec3<f32>, cv: f32) -> TerraMix {
    let s = se_terra_snow(point, h, slope, fp, n.x);
    let c = se_terra_climate(slope, s, fwidth(s.y), cv);
    return se_terra_weights(point, h, slope, fp, c, fwidth(c.x), n);
}

// The vertex stage's: transitions as sharp as SE's.
fn se_terra_mix_v(point: vec3<f32>, h: f32, slope: f32, fp: f32) -> TerraMix {
    let n = terra_noise(point, fp);
    let s = se_terra_snow(point, h, slope, fp, n.x);
    let c = se_terra_climate(slope, s, 0.0, terra_climate_noise(point, fp));
    return se_terra_weights(point, h, slope, fp, c, 0.0, n);
}

// SE's ColorMapTerra noises that vary only over kilometres (surface_class.wesl `terra_noise_at`): x
// the latitude's noise, y the albedo variation, z on worlds with plants the humidity's Fbm (else 0).
fn terra_noise(point: vec3<f32>, fp: f32) -> vec3<f32> {
    return terra_noise_at(point, fp, planet.biomes.climate[6].xyz, planet.biomes.flags.z > 0.5);
}

// The finest frequency on the unit sphere whose features a node's vertices carry for its pixels to
// interpolate: cells (1.5 times their mean on the cube face) at most 1/16 of the feature, as
// terrain.rs `vertex_noise` decides it.
fn vertex_freq() -> f32 {
    return planet.proc_params2.x / max(16.0 * planet.soil_tile.z / 32.0 * 1.5, 1e-6);
}

// How many of a sum's octaves, from frequency `freq` by `lac`, the node's vertices carry.
fn vertex_octaves(octaves: f32, freq: f32, lac: f32) -> f32 {
    return clamp(floor(log2(vertex_freq() / freq) / log2(lac)) + 1.0, 0.0, octaves);
}

// Octaves `first` and on of `ter_fbm(point, octaves, lac)` (the same points and amplitudes as its loop).
fn ter_fbm_from(point: vec3<f32>, octaves: f32, lac: f32, first: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    let gain = pow(lac, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        if f32(i) >= first {
            sum += gg_noise(pt) * ampl;
        }
        ampl *= gain;
        pt *= lac;
    }
    return sum;
}

struct TerraCoarse {
    // The latitude's, the variation's and the humidity's Fbm over their first `vertex_octaves`.
    sums: vec3<f32>,
    avd: vec3<f32>,
    // Their octave counts.
    k: vec3<f32>,
};

// The vertex stage's share of `terra_noise` on nodes too coarse for all of it (`TERRA_PARTIAL`):
// each sum's octaves fine enough for the vertices, and the warp whole (`vertex_noise` asks it to fit).
fn terra_noise_coarse(point: vec3<f32>) -> TerraCoarse {
    let rnd = planet.biomes.climate[6].xyz;
    let lac = 2.218281828459;
    var o: TerraCoarse;
    o.k = vec3<f32>(vertex_octaves(6.0, 0.7, lac), vertex_octaves(8.0, 1.0, lac), vertex_octaves(8.0, 1.73, lac));
    o.avd = ter_fbm3d((point + rnd) * 0.07, 4.0) * 1.3;
    o.sums.x = ter_fbm(point * 0.7 + rnd, o.k.x, lac);
    o.sums.y = ter_fbm(point + o.avd, o.k.y, lac);
    o.sums.z = 0.0;
    if planet.biomes.flags.z > 0.5 {
        o.sums.z = ter_fbm((point + o.avd) * 1.73, o.k.z, lac);
    }
    return o;
}

// `terra_noise` with the vertices' share (`terra_noise_coarse`, interpolated: `v`, `avd_v`; `k` its
// octave counts) and the
// pixel's own finer octaves. A sum of which the pixel keeps fewer octaves than the vertices carry,
// or whose warp it cuts, is worked out whole here, as `terra_noise` does.
fn terra_noise_split(point: vec3<f32>, fp: f32, v: vec3<f32>, avd_v: vec3<f32>, k: vec3<f32>) -> vec3<f32> {
    let rnd = planet.biomes.climate[6].xyz;
    let lac = 2.218281828459;
    let n_lat = ter_oct(6.0, 0.7, lac, fp);
    let k_lat = k.x;
    var lat: f32;
    if n_lat >= k_lat {
        lat = v.x + ter_fbm_from(point * 0.7 + rnd, n_lat, lac, k_lat);
    } else {
        lat = ter_fbm(point * 0.7 + rnd, n_lat, lac);
    }
    let n_avd = ter_oct(4.0, 0.07, lac, fp);
    let whole_warp = n_avd >= 4.0;
    var avd = avd_v;
    if !whole_warp {
        avd = ter_fbm3d((point + rnd) * 0.07, n_avd) * 1.3;
    }
    let n_av = ter_oct(8.0, 1.0, lac, fp);
    let k_av = k.y;
    var avf: f32;
    if whole_warp && n_av >= k_av {
        avf = v.y + ter_fbm_from(point + avd, n_av, lac, k_av);
    } else {
        avf = ter_fbm(point + avd, n_av, lac);
    }
    var humidity = 0.0;
    if planet.biomes.flags.z > 0.5 {
        let n_h = ter_oct(8.0, 1.73, lac, fp);
        let k_h = k.z;
        if whole_warp && n_h >= k_h {
            humidity = v.z + ter_fbm_from((point + avd) * 1.73, n_h, lac, k_h);
        } else {
            humidity = ter_fbm((point + avd) * 1.73, n_h, lac);
        }
    }
    return vec3<f32>(0.15 * (lat - 1.0), 1.0 - 0.5 * avf, humidity);
}

// SE's flatland climate distortion (surface_class.wesl `terra_climate_noise_at`).
fn terra_climate_noise(point: vec3<f32>, fp: f32) -> f32 {
    return terra_climate_noise_at(point, fp, planet.biomes.climate[6].xyz, planet.biomes.climate[7], planet.biomes.climate[5].y);
}

// x the climate (before the snow), y the snow line, z the latitude. `lat_noise`: `terra_noise`'s x.
fn se_terra_snow(point: vec3<f32>, h: f32, slope: f32, fp: f32, lat_noise: f32) -> vec3<f32> {
    let vary = snow_vary_at(point, fp, planet.biomes.climate[6].xyz);
    return se_terra_snow_at(point, h, slope, lat_noise, vary, planet.biomes.climate[0], planet.biomes.climate[1], planet.biomes.climate[2], planet.biomes.climate[6].w > 0.0, g_sun_body);
}

// x the climate, y the ice cap. `ws`: the snow line's change over a pixel; `vary`:
// `terra_climate_noise`.
fn se_terra_climate(slope: f32, s: vec3<f32>, ws: f32, vary: f32) -> vec2<f32> {
    return se_terra_climate_at(slope, s, ws, vary, planet.biomes.climate[0], planet.biomes.climate[1]);
}

// The slots' weights. `c`: the climate and ice cap; `cw`: the climate's change over a pixel;
// `n`: `terra_noise`.
fn se_terra_weights(point: vec3<f32>, h: f32, slope: f32, fp: f32, c: vec2<f32>, cw: f32, n: vec3<f32>) -> TerraMix {
    let plants = planet.biomes.flags.z > 0.5;
    var ptf = 0.0;
    if plants {
        ptf = plants_ptf_at(point, fp);
    }
    return se_terra_weights_at(point, h, slope, fp, c, cw, n, ptf, planet.biomes.climate[0], planet.biomes.climate[2], planet.biomes.climate[3], planet.biomes.climate[4], plants, true);
}

// The ground's biome look: colour, relief gradient (body axes, m per m), roughness and how much
// the sample layers show (for the roughness).
struct BiomeLook {
    col: vec3<f32>,
    grad: vec3<f32>,
    rough: f32,
    k: f32,
    // SE's colour variation, darkening and whitening (TerraMix), for the caller (vary < 0: none).
    vary: f32,
    dark: f32,
    white: f32,
    // The forest slot's weight (the scatter's trees stand by it on SE's terra worlds), and how
    // fertile the ground is (steppe, grass, forest: settlements prefer it).
    forest: f32,
    fertile: f32,
};

// One biome slot: its large and small sample layers (colours multiplied, reliefs added), times
// its tint if `tinted`. `la`/`lb` are the layers' `layer_pos`.
fn biome_look(k: i32, la: LayerPos, lb: LayerPos, tw: vec3<f32>, tinted: bool) -> BiomeLook {
    let tint = select(vec3<f32>(1.0), planet.biomes.color[k].rgb, tinted);
    let a = sample_layer(planet.biomes.large[k], la, tw);
    let b = sample_layer(planet.biomes.small[k], lb, tw);
    var o: BiomeLook;
    o.col = tint * a.col * b.col;
    o.grad = a.grad + b.grad;
    o.k = max(a.k, b.k);
    o.rough = (a.rough * a.k + b.rough * b.k) / max(a.k + b.k, 1e-4);
    return o;
}

// The two strongest biomes at this point, blended: tint times detail, or (untinted) the
// detail alone as a factor around 1.
fn biome_albedo(dir: vec3<f32>, h: f32, slope: f32, tinted: bool, fp: f32) -> BiomeLook {
    let radius = planet.proc_params2.x;
    let seed = vec3<f32>(planet.proc_params2.y * 917.0, planet.proc_params2.y * 331.0, planet.proc_params2.y * 613.0);
    var w: array<f32, 12>;
    var rock_tint = vec3<f32>(1.0);
    let se = planet.biomes.climate[4].w > 0.5;
    var m: TerraMix;
    m.vary = -1.0;
    m.dark = 1.0;
    if se {
        // SE's height map value: sea level (or the dry worlds' reference) plus the height in
        // BumpHeights. SE's generator: 0 terra, 1 selena (airless), 2 asteroid (as selena here).
        let hv = planet.biomes.climate[5].z + h / max(planet.biomes.climate[4].z, 1.0);
        if planet.biomes.climate[7].z > 0.5 {
            // On nodes that carry them, the kilometre-scale noises come from the vertices.
            @if(SELENA_AT_VERTICES) {
                m = se_selena_terms(dir, hv, slope, fp, tinted, g_selena);
            }
            @if(!SELENA_AT_VERTICES) {
                m = se_selena_mix(dir, hv, slope, fp, tinted);
            }
        } else {
            @if(TERRA_AT_VERTICES) {
                var cv: f32;
                @if(TERRA_CLIMATE_AT_VERTICES) {
                    cv = g_terra.w;
                }
                @if(!TERRA_CLIMATE_AT_VERTICES) {
                    cv = terra_climate_noise(dir, fp);
                }
                var tn = g_terra.xyz;
                @if(TERRA_PARTIAL) {
                    tn = terra_noise_split(dir, fp, g_terra.xyz, g_terra_avd, g_terra_k);
                }
                m = se_terra_mix_n(dir, hv, slope, fp, tn, cv);
            }
            @if(!TERRA_AT_VERTICES) {
                m = se_terra_mix(dir, hv, slope, fp);
            }
        }
        w = m.w;
        rock_tint = m.rock_tint;
    } else {
        let dn = designed_noise_at(dir, radius, planet.proc_params2.y);
        w = biome_weights(h, slope, abs(dir.y), dn.x, dn.y);
    }
    var k1 = 3;
    var k2 = 4;
    var w1 = -1.0;
    var w2 = -1.0;
    for (var k = 0; k < 12; k = k + 1) {
        let v = w[k];
        if v > w1 {
            k2 = k1;
            w2 = w1;
            k1 = k;
            w1 = v;
        } else if v > w2 {
            k2 = k;
            w2 = v;
        }
    }
    let pos_m = dir * radius;
    var tw = pow(abs(dir), vec3<f32>(4.0));
    tw = tw / (tw.x + tw.y + tw.z);
    let t = clamp(max(w2, 0.0) / max(w1 + max(w2, 0.0), 1e-4), 0.0, 0.5);
    var b1 = biome_look(k1, layer_pos(planet.biomes.large[k1], pos_m), layer_pos(planet.biomes.small[k1], pos_m), tw, tinted);
    if k1 == 5 {
        b1.col *= rock_tint;
    }
    // The second biome is read only where it shows (t > 0; at t = 0 the mixes below give b1
    // exactly). Its derivatives are taken here, outside the per-pixel branch.
    let l2a = layer_pos(planet.biomes.large[k2], pos_m);
    let l2b = layer_pos(planet.biomes.small[k2], pos_m);
    var b2 = b1;
    if t > 0.0 {
        b2 = biome_look(k2, l2a, l2b, tw, tinted);
        if k2 == 5 {
            b2.col *= rock_tint;
        }
    }
    // Designed worlds: albedo patches a few hundred km across (SE's own albedoVary is applied with
    // its ColorVary by the caller; textured bodies keep their maps).
    let patches = select(1.0, 1.0 + 0.22 * fbm3(dir * (radius / 250000.0) + seed.yzx, 4), tinted && !se);
    var o: BiomeLook;
    o.col = mix(b1.col, b2.col, t) * patches;
    o.grad = mix(b1.grad, b2.grad, t);
    o.rough = mix(b1.rough, b2.rough, t);
    o.k = mix(b1.k, b2.k, t);
    o.vary = m.vary;
    o.dark = m.dark;
    o.white = m.white;
    o.forest = w[11];
    o.fertile = clamp(w[9] * 0.5 + w[10] + w[11] * 0.8, 0.0, 1.0);
    var total = 0.0;
    for (var k = 0; k < 12; k = k + 1) {
        total += max(w[k], 0.0);
    }
    return o;
}

// A soil at a point: its colour recoloured to the ground (linear), relief slope in its tangent
// plane (x along the node's metres x, y along y), roughness and occlusion.
struct SoilLook {
    col: vec3<f32>,
    slope: vec2<f32>,
    rough: f32,
    ao: f32,
};

fn soil_slope(rg: vec2<f32>) -> vec2<f32> {
    return 1.0 - rg * 2.0;
}

// SE's `rgb2hsl` (hsl.glh), which is hue, saturation and value (the largest channel).
fn rgb2hsv(rgb: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(rgb.bg, k.wz), vec4<f32>(rgb.gb, k.xy), step(rgb.b, rgb.g));
    let q = mix(vec4<f32>(p.xyw, rgb.r), vec4<f32>(rgb.r, p.yzx), step(p.x, rgb.r));
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return clamp(vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn hsv2rgb(hsv: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(hsv.xxx + k.xyz) * 6.0 - k.www);
    return hsv.z * mix(k.xxx, clamp(p - k.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), hsv.y);
}

// SE compares and recolours soils on the stored (sRGB-encoded) colours: the exact sRGB curve, as
// the textures were decoded with it (a plain 2.2 power desaturates dark map colours enough to swap
// Earth's grassland for cracked dirt).
fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}

// The soil coordinates repeat every 4096 m (`SOIL_WRAP_M`), and every soil period divides it.
const SOIL_WRAP_M: f32 = 4096.0;

struct SoilTexel {
    col: vec3<f32>,
    slope: vec2<f32>,
    rhao: vec3<f32>,
};

// A soil layer in zone `z` (SE's soils: shifted up to 1.5 repeats; quarter turns, and flips from
// zone 4, only for a layer with the rotation flag). `uv` in repeats, `dx`, `dy` its derivatives.
fn soil_texel(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, z: f32, rot: bool) -> SoilTexel {
    let a = select(0.0, z * 1.5707964, rot);
    let flip = rot && z >= 4.0;
    let p = zone_xf(uv + sin(vec2<f32>(3.0, 7.0) * z) * 1.5, a, flip);
    let gx = zone_xf(dx, a, flip);
    let gy = zone_xf(dy, a, flip);
    var t: SoilTexel;
    t.col = to_gamma(textureSampleGrad(soil_tex, soil_smp, p, l, gx, gy).rgb);
    t.slope = zone_back(soil_slope(textureSampleGrad(soil_norm_tex, soil_smp, p, l, gx, gy).rg), a, flip);
    t.rhao = textureSampleGrad(soil_rhao_tex, soil_smp, p, l, gx, gy).rgb;
    return t;
}

// The port's close-up octave: SE's layers are 1024 texels a repeat, so a camera metres up sees a
// coarse one magnified (a 10 m soil, 1 cm a texel, spans 9 screen pixels a texel at 1.5 m): it
// blurred into blobs on the way down and the ground seemed to turn into another texture. As a
// layer is magnified past a texel a pixel, the same layer at an eighth of its repeat fades in,
// modulating brightness, relief and occlusion around its mean, so the ground keeps its look and
// stays sharp down to the camera's 1.5 m.
const SOIL_OCTAVE: f32 = 8.0;
const SOIL_OCTAVE_STRENGTH: f32 = 0.55;

// A soil layer with SE's zones, and the close-up octave where it is magnified.
fn soil_layer(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, period_m: f32, rot: bool) -> SoilTexel {
    let n = max(i32(round(SOIL_WRAP_M / period_m)), 1);
    var o = soil_zoned(l, uv, dx, dy, n, f32(l * 3), rot);
    let texels = max(length(dx), length(dy)) * 1024.0;
    let k = (1.0 - smoothstep(1.0 / SOIL_OCTAVE, 1.0, texels)) * SOIL_OCTAVE_STRENGTH;
    if k > 0.005 {
        let f = soil_zoned(l, uv * SOIL_OCTAVE, dx * SOIL_OCTAVE, dy * SOIL_OCTAVE, n * i32(SOIL_OCTAVE), f32(l * 3) + 1.5, rot);
        let mean = to_gamma(textureSampleLevel(soil_tex, soil_smp, vec2<f32>(0.5), l, 20.0).rgb);
        o.col *= mix(1.0, luma(f.col) / max(luma(mean), 0.02), k);
        o.slope += f.slope * k;
        o.rhao.b *= mix(1.0, f.rhao.b, k);
    }
    return o;
}

// A soil layer with SE's zones: the noise picking them repeats with the soil coordinates (`n`
// repeats across the 4096 m wrap), so neighbouring nodes stay seamless.
fn soil_zoned(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, n: i32, seed: f32, rot: bool) -> SoilTexel {
    let kz = 8.0 * clamp(vnoise_tiled(vec3<f32>(uv, seed), vec2<i32>(n, n)).x * 0.5 + 0.5, 0.0, 0.9999);
    let za = floor(kz);
    let fr = kz - za;
    // One zone where the blend has not begun or is done (as in `sample_layer`).
    if fr < 0.1 {
        return soil_texel(l, uv, dx, dy, za, rot);
    }
    if fr >= 0.9 {
        return soil_texel(l, uv, dx, dy, za + 1.0, rot);
    }
    let a = soil_texel(l, uv, dx, dy, za, rot);
    let b = soil_texel(l, uv, dx, dy, za + 1.0, rot);
    let t = smoothstep(0.2, 0.8, fr - 0.1 * dot(a.col - b.col, vec3<f32>(1.0)));
    var o: SoilTexel;
    o.col = mix(a.col, b.col, t);
    o.slope = mix(a.slope, b.slope, t);
    o.rhao = mix(a.rhao, b.rhao, t);
    return o;
}

// One soil, as SE's `DetailTextureMulti` (tg_detail_maps.glsl) bakes it: the detail layer modulated
// by the Modul layer (`BlendModulate`, texture_mix.glh: colour and roughness by the Modul's
// brightness, normals and occlusion mixed, those two scaled by the layers' scale ratio), roughness
// `Bias + Scale r`, then recoloured in HSV so the texture's mean takes the ground's colour `surf`
// (HSV, gamma). No parallax: SE never reads the height channel. `xy`, `dx`, `dy`: node-local
// metres and their screen derivatives; `q`: how far the detail layer can be trusted.
fn soil_look(k: i32, xy: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, q: f32, surf: vec3<f32>) -> SoilLook {
    let s = planet.biomes.soil[k];
    let md = planet.biomes.soil_mod[k];
    let b = planet.biomes.soil_bias[k];
    // The port's own fades: a layer whose repeat spans only a few dozen pixels would still show
    // as a fine grid through the zones; it gives way to its mean.
    let fp = max(max(length(dx), length(dy)), 1e-6);
    let rm = smoothstep(32.0, 256.0, s.w / fp);
    let qd = q * smoothstep(12.0, 96.0, s.y / fp);
    let ratio = min(s.y / s.w, s.w / s.y);
    var det = hsv2rgb(surf);
    var det_avg = det;
    var det_slope = vec2<f32>(0.0);
    var det_rh = vec2<f32>(0.8, 1.0);
    if s.x >= 0.0 && q > 0.0 {
        let dl = i32(s.x);
        det_avg = to_gamma(textureSampleLevel(soil_tex, soil_smp, vec2<f32>(0.5), dl, 20.0).rgb);
        let d = soil_layer(dl, xy / s.y, dx / s.y, dy / s.y, s.y, b.z > 0.5);
        det = mix(det_avg, d.col, qd);
        det_slope = d.slope * qd;
        det_rh = mix(vec2<f32>(0.8, 1.0), d.rhao.rb, qd);
    }
    var lum = 1.0;
    var lum_avg = 1.0;
    var mod_slope = det_slope;
    var mod_ao = det_rh.y;
    if s.z >= 0.0 {
        let ml = i32(s.z);
        lum_avg = luma(to_gamma(textureSampleLevel(soil_tex, soil_smp, vec2<f32>(0.5), ml, 20.0).rgb));
        let m = soil_layer(ml, xy / s.w, dx / s.w, dy / s.w, s.w, b.w > 0.5);
        lum = mix(lum_avg, luma(m.col), rm);
        mod_slope = m.slope * rm;
        mod_ao = mix(1.0, m.rhao.b, rm);
    }
    var o: SoilLook;
    // Colour: (detail x Modul brightness) taken to the ground's colour. SE moves each texel in HSV
    // by the ground minus the texture's mean; a grey texel's hue is arbitrary, so the saturation
    // that brings in turned rock textures' greys green, gold and red, and the ground changed colour
    // as the soils resolved on the way down (orange from 200 m, gold at 20 m). Here the ground's
    // colour carries the texture's brightness, and half its colour departures around its mean: the
    // mean stays the ground's at every distance.
    let c = to_linear(det * mix(1.0, lum, md.x));
    let c_avg = to_linear(det_avg * mix(1.0, lum_avg, md.x));
    let lc = luma(c);
    let la = max(luma(c_avg), 1e-3);
    let g = to_linear(hsv2rgb(surf));
    let tint = c / max(lc, 1e-3) - c_avg / la;
    o.col = max((g + tint * luma(g) * 0.5) * min(lc / la, 4.0), vec3<f32>(0.0));
    o.slope = mix(det_slope, mod_slope, md.y * ratio);
    o.rough = clamp(det_rh.x * mix(1.0, lum, md.z) * b.y + b.x, 0.05, 1.0);
    o.ao = mix(det_rh.y, mod_ao, md.w * ratio);
    return o;
}

// The soils blended, with `n` turned by their relief: colour (linear) and occlusion (rgb, a), the
// normal (xyz) and roughness (w).
struct SoilOut {
    col: vec4<f32>,
    n: vec4<f32>,
};

// SE's `MAT_MIX_SOFTNESS`: soils whose weights differ by less than this blend.
const SOIL_MIX_SOFTNESS: f32 = 0.05;

// SE's `ComputeMaterialWeight` (tg_detail_maps.glsl, COLOR_METRICS 1): four times the slope gap
// squared plus the HSV gaps to the soil's Color (hue looped as SE does, only one way round), weight
// d^-1/2. `g`: the ground's colour in HSV (gamma); `slope6`: SE's slope, 1 - cos^6 of the tilt.
// The soil's HSV and Slope come in `soil_color` (surface_tex.rs).
fn soil_weight(k: i32, g: vec3<f32>, slope6: f32) -> f32 {
    let sc = planet.biomes.soil_color[k];
    let ds = slope6 - sc.w;
    var delta = g - sc.xyz;
    delta.x = min(delta.x, 1.0 - delta.x);
    let d = ds * ds * 4.0 + dot(abs(delta), vec3<f32>(1.0));
    return inverseSqrt(max(d, 1e-6));
}

// The soils at the fragment, as SE bakes them into its near tiles: every soil weighed against the
// ground's colour `ground` (linear) and slope (1 - cos), the heaviest winning with a soft band to
// the next (`BlendSmart2B`), recoloured to `surf` (linear).
// The soil coordinates (node-local metres) and the screen derivatives the soils need, taken in
// uniform control flow so the soils themselves can be skipped where they do not show.
struct SoilCoords {
    xy: vec2<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
    dp1: vec3<f32>,
    dp2: vec3<f32>,
};

fn soil_coords(in: VertexOut) -> SoilCoords {
    var c: SoilCoords;
    c.xy = planet.soil_tile.xy + in.uv * planet.soil_tile.z;
    c.dx = dpdx(c.xy);
    c.dy = dpdy(c.xy);
    c.dp1 = dpdx(in.world_pos);
    c.dp2 = dpdy(in.world_pos);
    return c;
}

fn soil_detail(sc: SoilCoords, n: vec3<f32>, slope: f32, ground: vec3<f32>, surf: vec3<f32>) -> SoilOut {
    let xy = sc.xy;
    let dx = sc.dx;
    let dy = sc.dy;
    // Tangent frame of the soil coordinates (cotangent frame from the screen derivatives).
    let dp1 = sc.dp1;
    let dp2 = sc.dp2;
    let dp2p = cross(dp2, n);
    let dp1p = cross(n, dp1);
    var tg = dp2p * dx.x + dp1p * dy.x;
    var bt = dp2p * dx.y + dp1p * dy.y;
    let inv = inverseSqrt(max(max(dot(tg, tg), dot(bt, bt)), 1e-30));
    tg *= inv;
    bt *= inv;
    let g = rgb2hsv(to_gamma(ground));
    let c = 1.0 - clamp(slope, 0.0, 1.0);
    let c2 = c * c;
    let slope6 = 1.0 - c2 * c2 * c2;
    var k1 = 0;
    var k2 = -1;
    var w1 = 0.0;
    var w2 = 0.0;
    for (var k = 0; k < i32(planet.biomes.flags.w); k++) {
        let w = soil_weight(k, g, slope6);
        if w > w1 {
            k2 = k1;
            w2 = w1;
            k1 = k;
            w1 = w;
        } else if w > w2 {
            k2 = k;
            w2 = w;
        }
    }
    let b2 = max(w2 - w1 + SOIL_MIX_SOFTNESS, 0.0);
    let t = select(0.0, b2 / (SOIL_MIX_SOFTNESS + b2), k2 >= 0 && k2 != k1);
    // The soil metres come from the node's uv: on a big node right by the camera (a coarse one
    // before its children load) they step in f32 units of about edge / 2^23, which scrambles the
    // fine layer (sparkle). It fades out until a pixel spans a few such steps: from 1.5 steps, full
    // at 4 (the derivatives then wobble by a quarter, a third of a mip). It used to need 16, which
    // took the soils off every node coarser than ~500 m under a camera 1.5 m up: while the finer
    // nodes loaded (or lagged a fast camera) the ground at its feet showed as from hundreds of
    // metres up, and looked out of reach.
    let q = smoothstep(1.5, 4.0, max(length(dx), length(dy)) / max(planet.soil_tile.z * 1.2e-7, 1e-12));
    let sh = rgb2hsv(to_gamma(surf));
    var o = soil_look(k1, xy, dx, dy, q, sh);
    if t > 0.004 {
        let o2 = soil_look(k2, xy, dx, dy, q, sh);
        o.col = mix(o.col, o2.col, t);
        o.slope = mix(o.slope, o2.slope, t);
        o.rough = mix(o.rough, o2.rough, t);
        o.ao = mix(o.ao, o2.ao, t);
    }
    var out: SoilOut;
    out.col = vec4<f32>(o.col, o.ao);
    out.n = vec4<f32>(normalize(n + tg * o.slope.x + bt * o.slope.y), o.rough);
    return out;
}

// ---------------------------------------------------------------------------
// The detail displacement, in the vertex stage.

// A soil layer's colour (gamma) and height in zone `z` (`soil_texel`, read at mip `lod`).
fn soil_texel_h(l: i32, uv: vec2<f32>, lod: f32, z: f32, rot: bool) -> vec4<f32> {
    let a = select(0.0, z * 1.5707964, rot);
    let flip = rot && z >= 4.0;
    let p = zone_xf(uv + sin(vec2<f32>(3.0, 7.0) * z) * 1.5, a, flip);
    let col = to_gamma(textureSampleLevel(soil_tex, soil_smp, p, l, lod).rgb);
    return vec4<f32>(col, textureSampleLevel(soil_rhao_tex, soil_smp, p, l, lod).g);
}

// A soil layer's height with SE's zones (`soil_layer`).
fn soil_layer_h(l: i32, uv: vec2<f32>, lod: f32, period_m: f32, rot: bool) -> f32 {
    let n = max(i32(round(SOIL_WRAP_M / period_m)), 1);
    let kz = 8.0 * clamp(vnoise_tiled(vec3<f32>(uv, f32(l * 3)), vec2<i32>(n, n)).x * 0.5 + 0.5, 0.0, 0.9999);
    let za = floor(kz);
    let fr = kz - za;
    if fr < 0.1 {
        return soil_texel_h(l, uv, lod, za, rot).w;
    }
    if fr >= 0.9 {
        return soil_texel_h(l, uv, lod, za + 1.0, rot).w;
    }
    let a = soil_texel_h(l, uv, lod, za, rot);
    let b = soil_texel_h(l, uv, lod, za + 1.0, rot);
    let t = smoothstep(0.2, 0.8, fr - 0.1 * dot(a.rgb - b.rgb, vec3<f32>(1.0)));
    return mix(a.w, b.w, t);
}

// One soil's displacement (m) at node metres `xy`: SE's detail height (tg_detail_maps.glsl: the
// detail layer's height mixed towards the Modul's by ModHeight times the layers' scale ratio, then
// x ScaleHeight) around the textures' means. The layers are read at the mip of the vertex spacing
// (1024 texels a period), `shift` mips coarser.
fn soil_disp(k: i32, xy: vec2<f32>, spacing: f32, shift: f32) -> f32 {
    if k < 0 || k >= i32(planet.biomes.flags.w) {
        return 0.0;
    }
    let hs = planet.biomes.soil_height[k];
    if hs.z <= 0.0 {
        return 0.0;
    }
    let s = planet.biomes.soil[k];
    let b = planet.biomes.soil_bias[k];
    var h = vec2<f32>(0.0);
    var hm = vec2<f32>(0.0);
    if s.x >= 0.0 {
        let l = i32(s.x);
        let lod = max(log2(max(spacing * 1024.0 / s.y, 1e-6)) + shift, 0.0);
        h = vec2<f32>(soil_layer_h(l, xy / s.y, lod, s.y, b.z > 0.5), textureSampleLevel(soil_rhao_tex, soil_smp, vec2<f32>(0.5), l, 20.0).g);
    }
    if s.z >= 0.0 {
        let l = i32(s.z);
        let lod = max(log2(max(spacing * 1024.0 / s.w, 1e-6)) + shift, 0.0);
        hm = vec2<f32>(soil_layer_h(l, xy / s.w, lod, s.w, b.w > 0.5), textureSampleLevel(soil_rhao_tex, soil_smp, vec2<f32>(0.5), l, 20.0).g);
        let a = select(1.0, planet.biomes.soil_mod[k].w * min(s.y / s.w, s.w / s.y), s.x >= 0.0);
        h = mix(h, hm, a);
    }
    return (h.x - h.y) * hs.x * hs.z;
}

// The displacement (m) of the soils the fragment shows here (`soil_detail`'s choice by the
// ground's colour and slope).
fn soil_disp_v(xy: vec2<f32>, spacing: f32, shift: f32, ground: vec3<f32>, slope: f32) -> f32 {
    let g = rgb2hsv(to_gamma(ground));
    let c = 1.0 - clamp(slope, 0.0, 1.0);
    let c2 = c * c;
    let slope6 = 1.0 - c2 * c2 * c2;
    var k1 = 0;
    var k2 = -1;
    var w1 = 0.0;
    var w2 = 0.0;
    for (var k = 0; k < i32(planet.biomes.flags.w); k++) {
        let w = soil_weight(k, g, slope6);
        if w > w1 {
            k2 = k1;
            w2 = w1;
            k1 = k;
            w1 = w;
        } else if w > w2 {
            k2 = k;
            w2 = w;
        }
    }
    let b2 = max(w2 - w1 + SOIL_MIX_SOFTNESS, 0.0);
    let t = select(0.0, b2 / (SOIL_MIX_SOFTNESS + b2), k2 >= 0 && k2 != k1);
    var d = soil_disp(k1, xy, spacing, shift);
    if t > 0.004 && k2 >= 0 {
        d = mix(d, soil_disp(k2, xy, spacing, shift), t);
    }
    // A stray NaN would drop the vertex and open a hole (tested on the bits: compilers may fold
    // `d == d` to true).
    return select(d, 0.0, (bitcast<u32>(d) & 0x7fffffffu) >= 0x7f800000u);
}

// The ground colour the soils are matched to (the fragment's `soil_ground`), from the colour map
// or, on procedural worlds, the biomes at their textures' means. `spacing`: the vertex spacing (m).
fn soil_ground_v(uv: vec2<f32>, dir: vec3<f32>, h: f32, slope: f32, spacing: f32) -> vec3<f32> {
    if planet.rect_color.w > 0.5 {
        let c = textureSampleLevel(color_tex, color_smp, tile_uv(planet.rect_color, uv), 0.0).rgb * planet.modulate.rgb;
        let steep = smoothstep(0.18, 0.45, slope);
        let rock = vec3<f32>(0.11, 0.10, 0.09) + c * 0.25;
        return mix(c, rock, steep * 0.85);
    }
    if planet.proc_params2.z < 0.5 {
        return planet.base_color.rgb;
    }
    if planet.biomes.flags.x < 0.5 || planet.proc_params.x >= 5.5 {
        return proc_albedo(dir, h, slope, spacing);
    }
    let radius = planet.proc_params2.x;
    let fp = spacing / max(radius, 1.0);
    let seed = vec3<f32>(planet.proc_params2.y * 917.0, planet.proc_params2.y * 331.0, planet.proc_params2.y * 613.0);
    let se = planet.biomes.climate[4].w > 0.5;
    var w: array<f32, 12>;
    var m: TerraMix;
    m.rock_tint = vec3<f32>(1.0);
    m.vary = -1.0;
    m.dark = 1.0;
    m.white = 0.0;
    if se {
        let hv = planet.biomes.climate[5].z + h / max(planet.biomes.climate[4].z, 1.0);
        if planet.biomes.climate[7].z > 0.5 {
            m = se_selena_mix(dir, hv, slope, fp, true);
        } else {
            m = se_terra_mix_v(dir, hv, slope, fp);
        }
        w = m.w;
    } else {
        let dn = designed_noise_at(dir, radius, planet.proc_params2.y);
        w = biome_weights(h, slope, abs(dir.y), dn.x, dn.y);
    }
    var k1 = 3;
    var k2 = 4;
    var w1 = -1.0;
    var w2 = -1.0;
    for (var k = 0; k < 12; k = k + 1) {
        let v = w[k];
        if v > w1 {
            k2 = k1;
            w2 = w1;
            k1 = k;
            w1 = v;
        } else if v > w2 {
            k2 = k;
            w2 = v;
        }
    }
    let t = clamp(max(w2, 0.0) / max(w1 + max(w2, 0.0), 1e-4), 0.0, 0.5);
    let c1 = planet.biomes.color[k1].rgb * select(vec3<f32>(1.0), m.rock_tint, k1 == 5);
    let c2 = planet.biomes.color[k2].rgb * select(vec3<f32>(1.0), m.rock_tint, k2 == 5);
    let patches = select(1.0, 1.0 + 0.22 * fbm3(dir * (radius / 250000.0) + seed.yzx, 4), !se);
    var b = mix(c1, c2, t) * patches;
    if planet.biomes.color_vary.w > 0.5 && m.vary >= 0.0 {
        b *= mix(planet.biomes.color_vary.xyz, vec3<f32>(1.0), m.vary);
    }
    if m.vary >= 0.0 {
        b = mix(b * m.dark, vec3<f32>(1.0), m.white);
    }
    return b;
}

// Map node uv to a texel-centred position inside the (ancestor) tile.
// SE's 256-texel tiles carry a 2-texel border shared with their neighbours (each repeats the
// other's 4 edge texels): the tile proper spans texels 2 to 254.
fn tile_uv(rect: vec4<f32>, uv: vec2<f32>) -> vec2<f32> {
    let t = rect.xy + uv * rect.z;
    return t * (252.0 / 256.0) + vec2<f32>(2.0 / 256.0);
}

// Shadow of the planet's own rings, SE's RingsShadow (rings_common.glh, planet.glsl:1059-1073): where
// the ray towards the star crosses the ring plane (the point's own spot when it does not, which lies
// inside the rings), the light the ring lets through on that slant path, `(1 - density) ^ (1 / cosPhi)`
// with the density `RingsParams.z x alpha` (ring.z: the ring's `PlanetShadow`), anti-aliased at the ring's
// edges by the derivatives and fading out at equinox. `p` is in planet radii from the centre.
fn ring_shadow(p: vec3<f32>, l: vec3<f32>) -> f32 {
    if planet.ring.w < 0.25 {
        return 1.0;
    }
    let nrm = normalize(planet.ring_normal.xyz);
    let ly = dot(l, nrm);
    let py = dot(p, nrm);
    let along = min(py / select(ly, 1e-12, abs(ly) < 1e-12), 0.0);
    let proj = p - l * along;
    let tex_u = (length(proj - nrm * dot(proj, nrm)) - planet.ring.x) / (planet.ring.y - planet.ring.x);
    let fw = fwidth(tex_u * 2.0);
    // Off the rings the smoothing is 0.
    if tex_u <= 0.0 || tex_u >= 1.0 {
        return 1.0;
    }
    let u = tex_u;
    let smoothing = clamp(min(u, 1.0 - u) / max(fw, 1e-12), 0.0, 1.0);
    var a = 0.6;
    if planet.ring.w > 0.75 {
        a = textureSampleLevel(ring_tex, ring_smp, vec2<f32>(u, 0.75), 0.0).a;
    }
    let density = clamp(planet.ring.z * a, 0.0, 1.0);
    let cos_phi = abs(ly);
    let opacity = (1.0 - pow(1.0 - density, 1.0 / max(cos_phi, 0.333))) * smoothing * smoothstep(0.001, 0.003, cos_phi);
    return 1.0 - opacity;
}

// ---------------------------------------------------------------------------
// Water. An ocean's surface is its own mesh at sea level (SE's ocean sphere), drawn from both
// sides with `water_fragment`; the ground mesh under it keeps the seabed.

// Wind waves over the node's cube-face metres (`soil_tile`), which wrap every 4096 m
// (`SOIL_WRAP_M`), so everything here repeats over 4096 m: a few long swells (deep-water speeds,
// wave numbers whole multiples of 2 pi / 4096 m) under a choppy sea of value noise stretched
// across the wind (the u axis) and drifting downwind. Waves finer than a pixel or two fade
// out (SE keeps them to about a pixel, as sparkles) and their slopes go into the glint's roughness instead; all of them together make up SE's
// water roughness (`total_var`, the slope variance along each axis). Returns the slope along u
// and v and the faded variance.
const WAVE_WRAP_M: f32 = 4096.0;

// Value noise over (x, y, t) repeating every `period` cells in x and y: the value (-1..1) and its
// slopes along x and y (per cell).
fn vnoise_tiled(p: vec3<f32>, period: vec2<i32>) -> vec3<f32> {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let du = 6.0 * f * (1.0 - f);
    let b = vec3<i32>(i);
    let x0 = ((b.x % period.x) + period.x) % period.x;
    let y0 = ((b.y % period.y) + period.y) % period.y;
    let x1 = (x0 + 1) % period.x;
    let y1 = (y0 + 1) % period.y;
    var v = vec2<f32>(0.0);
    var dx = vec2<f32>(0.0);
    var dy = vec2<f32>(0.0);
    for (var k = 0; k < 2; k++) {
        let z = b.z + k;
        let a = hash31(vec3<i32>(x0, y0, z));
        let bx = hash31(vec3<i32>(x1, y0, z)) - a;
        let by = hash31(vec3<i32>(x0, y1, z)) - a;
        let bxy = hash31(vec3<i32>(x1, y1, z)) - a - bx - by;
        v[k] = a + bx * u.x + by * u.y + bxy * u.x * u.y;
        dx[k] = (bx + bxy * u.y) * du.x;
        dy[k] = (by + bxy * u.x) * du.y;
    }
    return vec3<f32>(mix(v.x, v.y, u.z), mix(dx.x, dx.y, u.z), mix(dy.x, dy.y, u.z)) * 2.0 - vec3<f32>(1.0, 0.0, 0.0);
}

fn wave_slopes(xy: vec2<f32>, footprint_m: f32, t: f32, total_var: f32) -> vec3<f32> {
    var g = vec2<f32>(0.0);
    var faded = 0.0;
    // Swell, 12 to 27 m long: 12% of the slope variance.
    let s_swell = sqrt(2.0 * 0.12 * total_var / 3.0);
    for (var i = 0; i < 3; i++) {
        let fi = f32(i);
        let lambda = 12.0 * pow(1.5, fi);
        let ang = (fi - 1.0) * 0.35 + planet.proc_params2.y;
        let k = round(vec2<f32>(cos(ang), sin(ang)) * WAVE_WRAP_M / lambda) * (6.2831853 / WAVE_WRAP_M);
        let kl = max(length(k), 1e-3);
        let shown = 1.0 - smoothstep(0.2, 0.5, footprint_m / lambda);
        g += k * (s_swell / kl * cos(dot(k, xy) - sqrt(9.81 * kl) * t + fi * 2.1) * shown);
        faded += 0.5 * s_swell * s_swell * (1.0 - shown);
    }
    // Chop in cells of 2 m down to 12.5 cm (twice as long across the wind): the other 88%. The
    // noise's slopes have an rms of about 0.8 per cell.
    let s_chop = sqrt(0.88 * total_var / 5.0) / 0.8;
    for (var o = 0; o < 5; o++) {
        let c = 2.0 / pow(2.0, f32(o));
        let per = vec2<i32>(i32(WAVE_WRAP_M / c), i32(WAVE_WRAP_M / (2.0 * c)));
        let drift = sqrt(9.81 * c / 6.2831853) * t;
        let n = vnoise_tiled(vec3<f32>((xy.x - drift) / c, xy.y / (2.0 * c), t * 0.6 / sqrt(c) + f32(o) * 17.0), per);
        let shown = 1.0 - smoothstep(0.6, 1.6, footprint_m / (2.0 * c));
        g += vec2<f32>(n.y, 0.5 * n.z) * (s_chop * shown);
        faded += 0.88 * total_var / 5.0 * (1.0 - shown);
    }
    return vec3<f32>(g, faded);
}

// SE's WaterVisibilityHeight (config variable, default 10 km; terrain.rs `water_reach_m`).
const WATER_VISIBILITY_KM: f32 = 10.0;
// SE's hdrWaterSurfaceBright (main-def.cfg 1): the underwater fog's light (SurfParams5.y).
const WATER_SURFACE_BRIGHT: f32 = 1.0;

// SE's WaterParams.z (FUN_1404acdd0), per km: the 3D water is whole up to 1/z from the camera and
// gone at 2/z, the ground pass's flat sea taking over. From sea level 1/100 km to 1/1 km at
// WaterVisibilityHeight and above; under water 1/(2R); Oceanic and Superoceanic worlds (volatiles
// 4, 5) 0, the 3D water everywhere.
fn water_fade_z(cam_alt_m: f32) -> f32 {
    let volq = planet.proc_params2.w;
    if volq > 3.5 && volq < 5.5 {
        return 0.0;
    }
    let h = cam_alt_m / 1000.0;
    if h <= 0.0 {
        return 500.0 / max(planet.planet.w, 1.0);
    }
    return 1.0 / (100.0 - 99.0 * min(h / WATER_VISIBILITY_KM, 1.0));
}

// SE's WaterParams.w (FUN_1404ae4a0), the Fresnel opacity's gain that makes the sea opaque towards
// the horizon: 1.5; Oceanic worlds 1.5 to 3.5 by the camera's height over WaterVisibilityHeight;
// Superoceanic 4.5.
fn water_horizon_opacity(cam_alt_m: f32) -> f32 {
    let volq = planet.proc_params2.w;
    if volq > 4.5 && volq < 5.5 {
        return 4.5;
    }
    if volq > 3.5 && volq < 4.5 {
        return 1.5 + 2.0 * clamp(cam_alt_m / 1000.0 / WATER_VISIBILITY_KM, 0.0, 1.0);
    }
    return 1.5;
}

// SE's CookTorranceBRDF (terrain_pbr.glh) for a dielectric (F0 0.04, metallic 0, ao 1): x the
// diffuse term's (1 - F) / pi, y the specular term, with SE's specular occlusion (1 - (1 - n.v)^2).
fn se_cook_torrance(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, nv: f32, nl: f32, rough: f32) -> vec2<f32> {
    let r2 = rough * rough;
    let r4 = r2 * r2;
    let r1 = rough + 1.0;
    let rk = r1 * r1 * 0.125;
    let h = normalize(l + v + vec3<f32>(1e-7));
    let vh = clamp(dot(v, h), 0.0, 1.0);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let dd = nh * nh * (r4 - 1.0) + 1.0;
    let ndf = r4 / max(PI * dd * dd, 1e-10);
    let g = nv / (nv * (1.0 - rk) + rk) * (nl / (nl * (1.0 - rk) + rk));
    let f = 0.04 + 0.96 * pow(1.0 - vh, 5.0);
    let occ = (1.0 - nv) * (1.0 - nv);
    return vec2<f32>((1.0 - f) / PI, ndf * g * f / (4.0 * nv * nl + 0.001) * (1.0 - occ));
}

struct SeaLight {
    // Sum over the lights of their sea-level light on the normal times the diffuse term (before the
    // water's colour) and times the specular term (before SpecBrightWater).
    diff: vec3<f32>,
    spec: vec3<f32>,
};

// SE's sea lighting (planet.glsl:1032-1213, water.glsl:475-599): every light's Cook-Torrance terms on
// the surface normal `n` (RoughnessWater) times its light at sea level on it, in the surface's units
// (x hdrPlanetSurfaceBright, SurfParams5.x).
fn sea_light(s: Surf, n: vec3<f32>, v: vec3<f32>, rough: f32) -> SeaLight {
    var o: SeaLight;
    o.diff = vec3<f32>(0.0);
    o.spec = vec3<f32>(0.0);
    let nv = clamp(dot(n, v), 0.0, 1.0);
    for (var i = 0; i < 4; i++) {
        var l = normalize(lit[planet.slot].sun_dir.xyz);
        var sun = s.sun_at_ground;
        if i > 0 {
            if lit[planet.slot].light_dir[i - 1].w < 0.5 {
                break;
            }
            l = normalize(lit[planet.slot].light_dir[i - 1].xyz);
            sun = s.sun_x[i - 1];
        }
        let nl = clamp(dot(n, l), 0.0, 1.0);
        let ct = se_cook_torrance(n, v, l, nv, nl, rough);
        let e = sun * (planet.atm_flags.y * nl);
        o.diff += e * ct.x;
        o.spec += e * ct.y;
    }
    return o;
}

// SE's waterFogAccum (planet.glsl:959, 1157-1159, 1245) before the water's colour: AmbientColor plus
// each light's direct light on level ground and sky irradiance, at hdrWaterSurfaceBright.
fn water_fog_light(s: Surf) -> vec3<f32> {
    return (vec3<f32>(lit[planet.slot].levels.x) + s.e_direct / planet.atm_flags.y + (s.e_city - s.e_direct) / PI) * WATER_SURFACE_BRIGHT;
}

// SE's ambSeaAccum light (planet.glsl:1026, 1132-1144, 1213): AmbientColor plus each light's day
// ambient and sky irradiance (at hdrAtmoIllumBright 1), `amb_k` x AmbientColor (water.glsl tints
// the AmbientColor by the water's colour twice).
fn water_amb_light(s: Surf, amb_k: vec3<f32>) -> vec3<f32> {
    return lit[planet.slot].levels.x * amb_k + (s.e_level - s.e_direct - vec3<f32>(lit[planet.slot].levels.x * planet.atm_flags.y)) / PI;
}

// SE's water in the ground pass (planet.glsl:807-867, 1243-1275), over the lit ground `col`: the ground
// under water (the seabed, and from under water all of it) fades through the water's absorption
// (WaterFogColor) along the view's path under water plus the water's depth there into the lit
// water colour (the underwater fog); past the 3D water's reach (waterFade3D) the flat sea surface
// covers it, its diffuse colour and ambient by the Fresnel opacity (0.02 + WaterParams.w x 0.98 (1 -
// n.v)^4) and the fog's, its Cook-Torrance glint on top. No sky reflection, refraction or Snell's
// window: SE has none.
fn se_ground_water(in: VertexOut, s: Surf, col: vec3<f32>, up: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    // WaterSurfColor.a: the ShowWater animation, squared (DAT_1419fb564^2 in FUN_1404ae4a0).
    let a = lit[planet.slot].levels.w * lit[planet.slot].levels.w;
    let wm = clamp(s.water, 0.0, 1.0);
    // SurfParams1.w: the water mask is the body's water map (Earth's).
    let earth = select(0.0, 1.0, planet.rect_water.w > 0.5);
    let d = length(in.world_pos) * planet.planet.w / max(in.centre.w, 1e-30) / 1000.0;
    let h = in.height / 1000.0;
    let e = in.cam_alt / 1000.0;
    // The view's path under water (km), the ground taken as flat (planet.glsl:833-848).
    var uwd = 0.0;
    if e > 0.0 {
        if h < 0.0 {
            uwd = d * -h / max(e - h, 1e-9);
        }
    } else {
        uwd = select(d, d * -e / max(h - e, 1e-9), h > 0.0);
    }
    var depth = water_depth(in) / 1000.0;
    // No underwater fog where the water mask says land.
    if depth > 0.0 {
        uwd *= wm;
        depth *= wm;
    }
    // Earth's rivers and lakes above sea level: a fake depth by the mask.
    uwd += 0.07 * max(wm - 0.18, 0.0) * earth;
    let att = clamp(exp(-(uwd + depth) * a * planet.water_fog), vec4<f32>(0.0), vec4<f32>(1.0));
    let opacity = 1.0 - att.a;
    let fog = planet.ocean_sea.rgb * water_fog_light(s);
    let fog_mix = opacity * mix(1.0, wm, earth);
    var c = col * att.rgb * (1.0 - fog_mix) + fog * fog_mix;
    // The flat sea surface, faded in where the 3D water fades out (waterMaskAF).
    let af = wm * a * clamp(d * water_fade_z(in.cam_alt) - 1.0, 0.0, 1.0);
    if af > 0.0 {
        let fres = clamp(0.02 + water_horizon_opacity(in.cam_alt) * 0.98 * pow(1.0 - abs(dot(v, up)), 4.0), 0.0, 1.0);
        let op = clamp(opacity + fres, 0.0, 1.0) * af;
        let sl = sea_light(s, up, v, planet.params.w);
        let sea = planet.ocean_sea.rgb * (sl.diff + water_amb_light(s, vec3<f32>(1.0)));
        c = c * (1.0 - op) + sea * op + sl.spec * (planet.params.y * a * af);
    }
    return c;
}

// Cook-Torrance GGX for a dielectric (F0 0.04), times n.l.
fn ground_spec(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, a_in: f32) -> f32 {
    // Unlit side: nothing, and no 0/0 from the visibility term (a NaN blackens the whole pixel).
    let nl = dot(n, l);
    if nl <= 0.0 {
        return 0.0;
    }
    let a = max(a_in, 0.01);
    let nv = max(dot(n, v), 1e-3);
    let hv = normalize(v + l);
    let nh = max(dot(n, hv), 0.0);
    let a2 = max(a * a, 1e-4);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let ndf = a2 / (PI * dd * dd);
    let fh = 0.04 + 0.96 * pow(1.0 - max(dot(hv, v), 0.0), 5.0);
    let kg = a * 0.5;
    let vis = 1.0 / ((nl * (1.0 - kg) + kg) * (nv * (1.0 - kg) + kg));
    return ndf * fh * vis * 0.25 * nl;
}

struct Surf {
    col: vec3<f32>,
    // Sun light at sea level (after the atmosphere) and the irradiance of a level surface there.
    sun_at_ground: vec3<f32>,
    e_level: vec3<f32>,
    // Star light above the atmosphere (after eclipses), in the tables' scale.
    e_sun: vec3<f32>,
    // Water cover from the water map (or from the height on bodies without one).
    water: f32,
    // The light on level ground from the stars and their sky light, shadowed (SE's
    // cityLightingAccum), which switches the night lights.
    e_city: vec3<f32>,
    // Of `e_city`, the stars' direct light (SE's sunLightHorShadow x NdotLSC), for the water.
    e_direct: vec3<f32>,
    // The further stars' light at the ground (as `sun_at_ground`), for the sea's glint.
    sun_x: array<vec3<f32>, 3>,
    // A generated planet's civilization: its towns' density here (their lights by night).
    city: f32,
    // The street lamps' share of a town's night light here (1 off the towns and from afar).
    lamp: f32,
};

// SE's planetary sphere shadow (planet.glsl:1087-1095): a point `height` m up is lit until the star
// (at `mu` = cos of its zenith angle) sets below the point's own horizon, with a sharp edge, so
// peaks and crater rims catch the light past the terminator.
fn horizon_shadow(height: f32, mu: f32) -> f32 {
    return horizon_shadow_at(planet.planet.w, height, mu);
}

// The ground's colour lit at `pos_km` (no water, clouds or haze yet).
// The crowns' spacing the ground draws past the trees' range (m), and how much the forest floor
// darkens under the trees (the shade of their crowns).
const CANOPY_M: f32 = 14.0;
const FLOOR_DARK: f32 = 0.45;

fn lit_ground(in: VertexOut, n: vec3<f32>, up: vec3<f32>, l: vec3<f32>, pos_km: vec3<f32>) -> Surf {
    let has_atm = lit[planet.slot].flags.x > 0.5;
    let p_rel = (in.world_pos - in.centre.xyz) / max(in.centre.w, 1e-30);
    let bdir = normalize(in.body_dir);
    let clouds = cloud_shadow(bdir, (length(p_rel) - 1.0) * planet.planet.w);
    let shadow = eclipse_light(lit[planet.slot].eclipse[0], p_rel, l) * ring_shadow(p_rel, l) * clouds;
    let sun = lit[planet.slot].sun_color.rgb * lit[planet.slot].sun_dir.w * shadow;
    let e_sun = sun * planet.atm_flags.y;

    var albedo = planet.base_color.rgb;
    var emission = vec3<f32>(0.0);
    // The pixel's footprint on the unit sphere, for SE's terrain colour noise.
    let fp = max(length(fwidth(bdir)), 1e-9);
    g_sun_body = normalize(vec3<f32>(dot(in.axis_x, l), dot(in.axis_y, l), dot(in.axis_z, l)) + vec3<f32>(1e-12));
    let slope = 1.0 - clamp(dot(n, up), 0.0, 1.0);
    // Biome colour for procedural worlds, biome detail for textured ones.
    let textured = planet.rect_color.w > 0.5;
    var biome = vec3<f32>(1.0);
    // The sample layers' relief turns the normal (body axes to world through the node frame).
    var nb = n;
    var sample_rough = 1.0;
    var sample_k = 0.0;
    var forest = 0.0;
    var fertile = 0.0;
    if planet.biomes.flags.x > 0.5 {
        let b = biome_albedo(bdir, in.height, slope, !textured, fp);
        biome = b.col;
        forest = b.forest;
        fertile = b.fertile;
        // SE's ColorVary (tg_terra_color.glsl:82-158): the colour darkens towards it over
        // planet-wide patches, `mix(colorVary, 1, albedoVary)`, albedoVary = 1 - 0.5 Fbm of the
        // point distorted by 1.3 Fbm3D((point + Randomize) 0.07), cubed and smoothstepped.
        if planet.biomes.color_vary.w > 0.5 && b.vary >= 0.0 {
            biome *= mix(planet.biomes.color_vary.xyz, vec3<f32>(1.0), b.vary);
        }
        if b.vary >= 0.0 {
            biome = mix(biome * b.dark, vec3<f32>(1.0), b.white);
        }
        let g = in.axis_x * b.grad.x + in.axis_y * b.grad.y + in.axis_z * b.grad.z;
        nb = normalize(n - (g - n * dot(g, n)));
        sample_rough = b.rough;
        sample_k = b.k;
    }
    // SE's ice glint (AICE): SpecBrightIce times the colour map's alpha, the ice mask.
    var ice = 0.0;
    var map_rgb = vec3<f32>(0.0);
    if textured {
        // SE tints the colour map by the catalog's ModulateColor (Mars, Venus, Titan).
        let c = textureSample(color_tex, color_smp, tile_uv(planet.rect_color, in.uv));
        map_rgb = c.rgb;
        albedo = c.rgb * planet.modulate.rgb;
        ice = planet.ice.x * planet.ice.z * c.a;
        // Below the texture's resolution, steep faces shed their snow or soil: show rock.
        let steep = smoothstep(0.18, 0.45, slope);
        let rock = vec3<f32>(0.11, 0.10, 0.09) + albedo * 0.25;
        albedo = mix(albedo, rock, steep * 0.85);
    }
    // The soils are matched to the map's own colour: the biome textures' brightness below would
    // swap them in patches (stones over a sand sea).
    let soil_ground = albedo;
    if textured {
        // The colour map carries the hue; the biome textures add relief in brightness.
        albedo *= clamp(luma(biome), 0.0, 3.0);
    } else if planet.proc_params2.z > 0.5 {
        if planet.biomes.giant[1].y > 0.5 {
            albedo = gas_giant_albedo(bdir, GiantParams(planet.biomes.giant_palette, planet.biomes.giant));
        } else if planet.biomes.flags.x > 0.5 && planet.proc_params.x < 5.5 {
            albedo = biome;
        } else {
            albedo = proc_albedo(bdir, in.height, slope, g_footprint_m);
        }
    }
    // Thermal emission, by the unshadowed star (the heating is from its light at the surface).
    // SE's SelfEmissionBright scales it (SurfParams4.w), EnableSelfEmission off drops it.
    if (planet.thermal.x > 0.45 || planet.thermal.z > 0.04) && lit[planet.slot].levels.z > 0.0 {
        emission = thermal_emission(bdir, in.height, dot(up, l), lit[planet.slot].sun_color.rgb * lit[planet.slot].sun_dir.w * planet.atm_flags.y) * lit[planet.slot].levels.z;
    }
    // The soils, chosen by the map's colour and recoloured to the ground's, take over within 2 km
    // (SE's `detailFade = clamp(2 - distance_km, 0, 1)`, planet.glsl).
    var ns = nb;
    var rough = sample_rough;
    var fade = 0.0;
    if planet.biomes.flags.y > 0.5 {
        let sc = soil_coords(in);
        fade = clamp(2.0 - length(in.world_pos) * planet.planet.w / max(in.centre.w, 1e-30) / 1000.0, 0.0, 1.0);
        // Only where they show: past 2 km they would cost the most and add nothing.
        if fade > 0.0 {
            let so = soil_detail(sc, nb, slope, select(albedo, soil_ground, textured), albedo);
            albedo = mix(albedo, so.col.rgb, fade) * mix(1.0, so.col.a, fade);
            ns = normalize(mix(nb, so.n.xyz, fade));
            rough = mix(rough, so.n.w, fade);
        }
    }
    // The vegetation (scatter.rs; light record flags z the kind, w the trees' range): the forest
    // floor darkens under the trees, and from where they end the ground draws the forest's crowns
    // until they are too small to see (vegetation.wesl). The forest's share is the trees' own: SE's
    // forest weight on its terra worlds, the colour map's forest class sharpened by the woods' noise.
    let veg = lit[planet.slot].flags.z;
    if veg > 1.5 {
        let dist = length(in.world_pos) * planet.planet.w / max(in.centre.w, 1e-30);
        let far = lit[planet.slot].flags.w;
        // Over the last quarter of the trees' range they thin out and the canopy comes in (scatter.wesl).
        let canopy_k = (1.0 - smoothstep(0.25, 0.7, g_footprint_m / CANOPY_M)) * smoothstep(far * 0.75, far, dist);
        let near_k = 1.0 - smoothstep(far * 0.75, far, dist);
        if canopy_k > 0.0 || near_k > 0.0 {
            var f = forest;
            if veg > 2.5 {
                f = map_forest(veg_classes(lin_to_srgb255(map_rgb)).x, map_noise(bdir, planet.proc_params2.x), slope);
            }
            if f > 0.0 {
                var k = 1.0 - FLOOR_DARK * f * near_k;
                if canopy_k > 0.0 {
                    k *= mix(1.0, canopy_shade(bdir, planet.proc_params2.x, g_sun_body, CANOPY_M), f * canopy_k);
                }
                albedo *= k;
            }
        }
    }
    // A generated planet's civilization (settlement.wesl): its towns by day in their colour, the
    // fields' patchwork round them, where the land suits (low, flat, fertile, the coast as the
    // culture likes it); their lights by night (`finish`). Earth draws its own from its maps.
    var city = 0.0;
    // The street lamps' share of the town's night light (city.wesl CityGround::lamp), and the lamps'
    // heads and the cars' lights (lit by night: the star under the horizon).
    var lamp = 1.0;
    var city_emit = vec3<f32>(0.0);
    if planet.civ.x > 0.5 && !textured && planet.proc_params2.z > 0.5 {
        let sea = planet.ocean_sea.w > 0.5;
        let h = in.height;
        // The land's suitability without and with the pixel's own slope (the city's layout leaves
        // the slope out: bumpy ground would punch holes in its roads). Not the highlands: with a sea
        // up to a quarter of the ground's height range above it (more near the coast, as the culture
        // likes it), without one up to about half the range (the fertility, the coast and the
        // settlement field say where the towns cluster).
        let hr = (h - select(planet.city_land.x, 0.0, sea)) / max(planet.city_land.y, 100.0);
        var land = 1.0 - smoothstep(select(0.55, 0.25, sea), select(0.8, 0.5, sea), hr);
        if sea {
            land *= smoothstep(2.0, 20.0, h) * (1.0 + planet.civ.w * (1.0 - smoothstep(0.0, 0.03, hr)));
        }
        let fert = select(1.0, mix(0.3, 1.0, fertile), planet.biomes.flags.x > 0.5);
        land *= fert;
        let suit = land * (1.0 - smoothstep(0.04, 0.14, slope));
        if land > 0.0 {
            // The suitability says where settlements stand, not how dense they are: from about 0.3
            // the land takes them at full density (a people on poor land still fill its best land;
            // as a factor, towns x a world's typical 0.1-0.3 left its night side dark).
            let k = smoothstep(0.03, 0.3, suit);
            let st = settlements(bdir, planet.proc_params2.x, u32(planet.civ.x + 0.5), u32(planet.civ.y * 10000.0), g_footprint_m);
            let urban = clamp(st.x * k, 0.0, 1.0);
            // (No farmland on a colony without air.)
            let airless = planet.city_land.z > 0.5;
            let farm = clamp(st.y * k, 0.0, 1.0) * fert * (1.0 - urban) * select(1.0, 0.0, airless);
            albedo *= mix(vec3<f32>(1.0), st_fields(bdir, planet.proc_params2.x, u32(planet.civ.y * 10000.0), g_footprint_m), farm * 0.7);
            // The towns' streets, sidewalks and lots and the roads through the farmland (city.wesl);
            // villages too small for streets take the towns' colour.
            let kl = smoothstep(0.03, 0.3, land);
            // (`SE_CITY_TEST`: towns on all suitable land.)
            let cp = city_pos(planet.city_tile.xy, planet.soil_tile.xy + in.uv * planet.soil_tile.z);
            let metric = 0.5 * (planet.city_frame.x + planet.city_frame.y);
            // A colony without air: its towns under their domes alone (city.wesl `ct_dome_mask`).
            let domed = select(1.0, ct_dome_mask(cp, g_footprint_m / max(metric, 1e-3), u32(planet.civ.y * 10000.0) + 100u), airless);
            let town = clamp(max(st.x, planet.city_frame.w) * kl, 0.0, 1.0) * domed;
            albedo = mix(albedo, planet.civ_urban.rgb, urban * domed * 0.85 * (1.0 - smoothstep(0.06, 0.2, town)));
            let rural = smoothstep(0.1, 0.3, st.y * kl);
            var cg: CityGround;
            if planet.civ.x < 1.5 {
                // A primitive people's villages: dirt paths, huts, hearth fires.
                cg = village_ground(cp, metric, g_footprint_m, town, rural, u32(planet.civ.y * 10000.0) + 100u, albedo, globals.time);
            } else {
                cg = city_ground(cp, metric, g_footprint_m, town, rural, u32(planet.civ.y * 10000.0) + 100u, planet.city_frame.z, albedo, planet.civ_urban.rgb * 0.6, planet.civ_light.rgb / max(max(planet.civ_light.r, planet.civ_light.g), max(planet.civ_light.b, 1e-3)), globals.time);
            }
            albedo = cg.col;
            // Level 3 and up: near the ground the towers' windows give much of a town's light, the
            // streets less of it (their cultures are 1.6-2.2 times as bright from orbit).
            lamp = cg.lamp * select(1.0, mix(0.45, 1.0, smoothstep(30.0, 400.0, g_footprint_m)), planet.civ.x > 2.5);
            city_emit = cg.emit;
            // Their lights: the towns, and the countryside round them (farms, hamlets, roads).
            city = urban * domed + RURAL_LIGHT * farm * farm * st_points(bdir, planet.proc_params2.x, u32(planet.civ.y * 10000.0), g_footprint_m);
        }
    }
    // A textured body's civilization (Earth): its towns' layout where its lights map says they are.
    if planet.civ.x > 0.5 && textured && planet.rect_lights.w > 0.5 {
        let lt = textureSample(lights_tex, lights_smp, tile_uv(planet.rect_lights, in.uv)).rgb;
        // Not on the water (its mask; the lights bleed over rivers and bays).
        var dry = 1.0;
        if planet.rect_water.w > 0.5 {
            dry = 1.0 - smoothstep(0.2, 0.6, textureSample(water_tex, water_smp, tile_uv(planet.rect_water, in.uv)).r);
        }
        let lum = max(lt.r, max(lt.g, lt.b));
        let town = smoothstep(0.04, 0.5, lum) * dry;
        let rural = smoothstep(0.004, 0.03, lum) * dry;
        if rural > 0.0 {
            let cp = city_pos(planet.city_tile.xy, planet.soil_tile.xy + in.uv * planet.soil_tile.z);
            let metric = 0.5 * (planet.city_frame.x + planet.city_frame.y);
            let cg = city_ground(cp, metric, g_footprint_m, town, rural, u32(planet.civ.y * 10000.0) + 100u, 0.35, albedo, vec3<f32>(0.14, 0.135, 0.13), vec3<f32>(1.0, 0.68, 0.32), globals.time);
            albedo = cg.col;
            // Level 3 and up: near the ground the towers' windows give much of a town's light, the
            // streets less of it (their cultures are 1.6-2.2 times as bright from orbit).
            lamp = cg.lamp * select(1.0, mix(0.45, 1.0, smoothstep(30.0, 400.0, g_footprint_m)), planet.civ.x > 2.5);
            city_emit = cg.emit;
        }
    }
    // SE's "rings winter" (planet.glsl RINGS, light 0): on the hemisphere the rings shade, the
    // surface colour fades by tan(the sun's elevation over the ring plane) x the strength.
    if planet.ring_normal.w > 0.0 {
        let cos_phi = min(abs(g_sun_body.y), 0.9999);
        let lt = smoothstep(0.0, 0.6, abs(bdir.y)) * step(g_sun_body.y * bdir.y, 0.0);
        let st = clamp(cos_phi / sqrt(1.0 - cos_phi * cos_phi) * planet.ring_normal.w, 0.0, 1.0);
        albedo *= 1.0 - lt * st;
    }

    // Sun light reaching the surface (reddened through the atmosphere) and sky light.
    let terminator = horizon_shadow(in.height, dot(up, l));
    var s: Surf;
    s.e_sun = e_sun;
    s.sun_at_ground = sun * terminator;
    if has_atm {
        let e = atm_surface_irradiance(planet.atm, transmittance_tex, irradiance_tex, atm_smp, pos_km, ns, l, e_sun, 1.0);
        s.col = albedo / PI * e;
        let r = length(pos_km);
        s.sun_at_ground = sun * atm_transmittance_sun(planet.atm, transmittance_tex, atm_smp, r, dot(pos_km / r, l));
        s.e_level = atm_surface_irradiance(planet.atm, transmittance_tex, irradiance_tex, atm_smp, pos_km, up, l, e_sun, 1.0);
    } else {
        // Same radiometric scale as the atmosphere path (albedo / pi * irradiance).
        s.col = albedo / PI * max(dot(ns, l), 0.0) * terminator * sun * planet.atm_flags.y;
        s.e_level = max(dot(up, l), 0.0) * terminator * e_sun;
    }
    s.e_city = s.e_level;
    // The civilization's lights are on where the main star's light on level ground is under SE's
    // city-light cutoff (as `city_lights_on`), not counting the further lights: in SE's light scale
    // (`(F / F_max)^0.35`) a moon's or a close planet's shine lights the night side at about 1 % of
    // the star, ten times the cutoff, and every night with a moon up would be dark.
    let civ_cut = max(CITY_LIGHTS_CUTOFF * lit[planet.slot].sun_dir.w * planet.atm_flags.y, 1e-12);
    s.city = city * (1.0 - smoothstep(civ_cut * 0.2, civ_cut, dot(s.e_level, vec3<f32>(0.3, 0.59, 0.11))));
    s.lamp = lamp;
    s.e_direct = s.sun_at_ground * (planet.atm_flags.y * max(dot(up, l), 0.0));
    s.sun_x = array<vec3<f32>, 3>(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    // SE's Hapke BRDF (terrain_pbr.glh) on the direct light: Lommel-Seeliger with the opposition
    // surge (SpotBright over SpotWidth, and the coherent-backscatter spot), mixed in by `Hapke`.
    if planet.modulate.w > 0.0 {
        let v = normalize(-in.world_pos);
        let nl = max(dot(ns, l), 0.0);
        let nv = max(dot(ns, v), 0.0);
        let vl = clamp(dot(v, l), -0.999, 1.0);
        let t = sqrt((1.0 - vl) / (1.0 + vl));
        let k = 1.0 + planet.hapke.x / (1.0 + t * planet.hapke.y) + planet.hapke.z / (1.0 + t * planet.hapke.w);
        let h = mix(1.0, k / (nl + nv + 0.001), planet.modulate.w);
        s.col += albedo / PI * s.sun_at_ground * planet.atm_flags.y * nl * (h - 1.0);
    }
    // SE's day ambient (`DayAmbient`) on the day side, at the ambient's share of the light.
    let day = e_sun * (PI * ATM_AMBIENT_SCALE * planet.atm_flags.w * max(dot(up, l), 0.0));
    s.col += albedo / PI * day;
    s.e_level += day;
    // The further stars: their direct light (with the Hapke term), sky light and day ambient,
    // shadowed by the rings and eclipses on their own paths (planet.glsl:1032-1131).
    for (var i = 0; i < 3; i++) {
        if lit[planet.slot].light_dir[i].w < 0.5 {
            break;
        }
        let lx = normalize(lit[planet.slot].light_dir[i].xyz);
        let sun_x = lit[planet.slot].light_color[i].rgb * eclipse_light(lit[planet.slot].eclipse[i + 1], p_rel, lx) * ring_shadow(p_rel, lx);
        let e_x = sun_x * planet.atm_flags.y;
        let term_x = horizon_shadow(in.height, dot(up, lx));
        var ground_x = sun_x * term_x;
        var level_x: vec3<f32>;
        if has_atm {
            s.col += albedo / PI * atm_surface_irradiance(planet.atm, transmittance_tex, irradiance_tex, atm_smp, pos_km, ns, lx, e_x, 1.0);
            level_x = atm_surface_irradiance(planet.atm, transmittance_tex, irradiance_tex, atm_smp, pos_km, up, lx, e_x, 1.0);
            let r = length(pos_km);
            ground_x = sun_x * atm_transmittance_sun(planet.atm, transmittance_tex, atm_smp, r, dot(pos_km / r, lx));
        } else {
            s.col += albedo / PI * max(dot(ns, lx), 0.0) * term_x * e_x;
            level_x = max(dot(up, lx), 0.0) * term_x * e_x;
        }
        s.e_level += level_x;
        s.e_city += level_x;
        s.e_direct += ground_x * (planet.atm_flags.y * max(dot(up, lx), 0.0));
        s.sun_x[i] = ground_x;
        if planet.modulate.w > 0.0 {
            let v = normalize(-in.world_pos);
            let nl = max(dot(ns, lx), 0.0);
            let nv = max(dot(ns, v), 0.0);
            let vl = clamp(dot(v, lx), -0.999, 1.0);
            let t = sqrt((1.0 - vl) / (1.0 + vl));
            let k = 1.0 + planet.hapke.x / (1.0 + t * planet.hapke.y) + planet.hapke.z / (1.0 + t * planet.hapke.w);
            s.col += albedo / PI * ground_x * planet.atm_flags.y * nl * (mix(1.0, k / (nl + nv + 0.001), planet.modulate.w) - 1.0);
        }
        let day_x = e_x * (PI * ATM_AMBIENT_SCALE * planet.atm_flags.w * max(dot(up, lx), 0.0));
        s.col += albedo / PI * day_x;
        s.e_level += day_x;
    }
    // SE's AmbientLighting (the AmbientColor its light sums start from), in the light's units.
    s.col += albedo / PI * lit[planet.slot].levels.x * planet.atm_flags.y + emission;
    s.col += city_emit * ((1.0 - smoothstep(-0.12, 0.03, dot(up, l))) * lit[planet.slot].levels.z);
    s.e_level += vec3<f32>(lit[planet.slot].levels.x * planet.atm_flags.y);
    // The soils' sheen: GGX with their roughness (dielectric, F0 0.04), only where the
    // mesh's own surface faces the star.
    // Beyond the soils' 2 km, the sample layers' roughness where they show.
    let sheen = mix(sample_k, 1.0, fade);
    if sheen > 0.0 {
        let v = normalize(-in.world_pos);
        s.col += s.sun_at_ground * planet.atm_flags.y * ground_spec(ns, v, l, rough * rough) * smoothstep(0.0, 0.1, dot(n, l)) * sheen;
    }
    // The ice's Cook-Torrance glint (RoughnessIce) from every light, giving way to the soils' own
    // close by (SE's specTer goes to the detail textures' by detailFadeSpec). The rings' shadow uses
    // fwidth, so it is taken here in uniform control flow, not inside the per-pixel branch.
    var ring_x = array<f32, 3>(1.0, 1.0, 1.0);
    for (var i = 0; i < 3; i++) {
        if lit[planet.slot].light_dir[i].w < 0.5 {
            break;
        }
        ring_x[i] = ring_shadow(p_rel, normalize(lit[planet.slot].light_dir[i].xyz));
    }
    if ice * (1.0 - fade) > 0.0 {
        let v = normalize(-in.world_pos);
        let a = planet.ice.y * planet.ice.y;
        var glint = s.sun_at_ground * ground_spec(ns, v, l, a);
        for (var i = 0; i < 3; i++) {
            if lit[planet.slot].light_dir[i].w < 0.5 {
                break;
            }
            let lx = normalize(lit[planet.slot].light_dir[i].xyz);
            let sun_x = lit[planet.slot].light_color[i].rgb * eclipse_light(lit[planet.slot].eclipse[i + 1], p_rel, lx) * ring_x[i];
            glint += sun_x * horizon_shadow(in.height, dot(up, lx)) * ground_spec(ns, v, lx, a);
        }
        s.col += glint * planet.atm_flags.y * ice * (1.0 - fade) * smoothstep(0.0, 0.1, dot(n, l));
    }
    s.water = 0.0;
    if planet.ocean_sea.w > 0.5 {
        // SE's `step(height, seaLevel)` on the height tile, per pixel and anti-aliased over one:
        // the mesh's interpolated height would put water on dry beaches in triangles.
        let c = shore_code(in);
        s.water = clamp(0.5 - c / max(fwidth(c), 1e-4), 0.0, 1.0);
        if planet.rect_water.w > 0.5 {
            s.water = textureSample(water_tex, water_smp, tile_uv(planet.rect_water, in.uv)).r;
        }
    }
    return s;
}

// A civilization's lights at full density in the glow map's units: Earth's city cores (about 0.6
// in its lights tiles, times SE's 1.5 on the glow brightness).
const CIV_LIGHT: f32 = 0.9;
// The countryside's lights over farmland, as a share of a town's, times the farmland's share squared
// (brightest near the towns): from orbit Earth's populated land glows well beyond its towns, but
// unevenly (as the plain share, every land glowed alike).
const RURAL_LIGHT: f32 = 0.2;

// SE's city-light cutoff, per unit of the main light's intensity (SurfParams4.x = 0.001 with HDR,
// 0.05 without, times light 0's intensity at +0x58: FUN_1404ae4a0, FUN_1404bf5c0).
const CITY_LIGHTS_CUTOFF: f32 = 0.001;

// SE's city-light switch (planet.glsl:1336-1338): a Night glow map goes out as the light on level
// ground `e_city` (every light, planet shine included, after eclipses and ring shadows) rises from a
// fifth of the cutoff to it, so it lights up in eclipses, and a bright moon dims it as in SE. A
// Permanent one (Io's lava) always shines.
fn city_lights_on(e_city: vec3<f32>) -> f32 {
    let cut = max(CITY_LIGHTS_CUTOFF * lit[planet.slot].sun_dir.w * planet.atm_flags.y, 1e-12);
    let lum = dot(e_city, vec3<f32>(0.3, 0.59, 0.11));
    return 1.0 - smoothstep(cut * 0.2, cut, lum) * planet.glow.w;
}

// City lights at night and aerial perspective, for a camera above the sea. SE's surface pass casts
// no cloud shadows (planet.glsl's Shadow is rings x eclipse only; the clouds are their own shell
// drawn over the ground, which also covers the city lights).
fn finish(in: VertexOut, col_in: vec3<f32>, up: vec3<f32>, l: vec3<f32>, e_sun: vec3<f32>, e_city: vec3<f32>, city: f32, lamp: f32) -> vec3<f32> {
    var col = col_in;

    // A generated planet's civilization's lights (settlement.wesl): `city` is their density, already
    // switched by the night (`lit_ground`). Times the self-emission setting (`levels.z`), not the
    // glow map's brightness (`levels.y`: 0 on bodies without a glow map).
    if city > 0.0 {
        col += planet.civ_light.rgb * (planet.civ.z * CIV_LIGHT * city * lamp * lit[planet.slot].levels.z);
    }

    // The glow map (city lights, Io's lava) in the catalog's GlowColor, by SE's switch.
    if planet.rect_lights.w > 0.5 {
        let glow = textureSample(lights_tex, lights_smp, tile_uv(planet.rect_lights, in.uv)).rgb * planet.glow.rgb;
        col += glow * (city_lights_on(e_city) * lit[planet.slot].levels.y * lamp);
    }

    // Aerial perspective: haze and attenuation between the camera and the ground.
    if lit[planet.slot].flags.x > 0.5 {
        // In the tables' sphere: the body's axes with its ellipsoid undone, as the sky shell does,
        // so a flattened planet's ground lies on the tables' ground and not inside it near the
        // poles (which cut it to black and left only the haze).
        let cam_km = table_local(in, -in.centre.xyz);
        // Aerial perspective subtracts two inscatter lookups along the view ray; keep both
        // ends in the same (interpolated) geometry or the difference blows up.
        var view_km = table_local(in, in.world_pos - in.centre.xyz);
        // Between its corners a flat triangle sags inside the ground's sphere (0.4 km on a giant
        // seen from afar), which the haze took for under the ground: a darker cell per triangle.
        // Lift the point out to the ground's radius at its height (never down).
        let r_ground = planet.atm.radii.x * (1.0 + in.height / max(planet.planet.w, 1.0));
        view_km *= max(r_ground / max(length(view_km), 1e-6), 1.0);
        // The brightest star's, the further stars' and a glowing atmosphere's haze (shared with the
        // objects on the ground).
        col = ground_haze(planet.atm, inscatter_tex, transmittance_tex, atm_smp, col, cam_km, view_km, in.axis_x, in.axis_y, in.axis_z, l, e_sun, lit[planet.slot], planet.atm_flags);
    }
    return col;
}

// Wave normal at the fragment; w: slope variance of the waves too fine to show (for the glint).
fn wave_normal(in: VertexOut, up: vec3<f32>, total_var: f32) -> vec4<f32> {
    let xy = planet.soil_tile.xy + in.uv * planet.soil_tile.z;
    let w = wave_slopes(xy, g_footprint_m, globals.time, total_var);
    let tu = normalize(in.tangent_u - up * dot(in.tangent_u, up));
    let tv = cross(up, tu);
    return vec4<f32>(normalize(up - w.x * tu - w.y * tv), w.z);
}

// Smooth surface point (km, table frame) at height `h` (m) over the fragment's up vector.
// Interpolated positions sag below the sphere between vertices (tens of km on a coarse
// giant-planet patch), which the tables would read as lower altitude and shade as a grid.
// A world offset from the planet's centre in the atmosphere tables' frame (km): along the body's
// axes, each divided by the node frame's scale on it (the sky shell's `cam_local`), so the body's
// ellipsoid becomes the tables' sphere.
fn table_local(in: VertexOut, v: vec3<f32>) -> vec3<f32> {
    let l = vec3<f32>(dot(v, in.axis_x) / in.axis_len.x, dot(v, in.axis_y) / in.axis_len.y, dot(v, in.axis_z) / in.axis_len.z);
    return l * (planet.atm.radii.x / max(planet.planet.w, 1e-30));
}

// A world direction along the body's axes (the sky shell's `to_shell`).
fn body_axes(in: VertexOut, d: vec3<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(dot(d, in.axis_x), dot(d, in.axis_y), dot(d, in.axis_z)));
}

fn table_pos(up: vec3<f32>, h: f32) -> vec3<f32> {
    return up * planet.atm.radii.x * (1.0 + h / max(planet.planet.w, 1.0));
}

// The ground's shading normal from the node's normal map (texel centres span the node): the map
// holds the normal's departure from the smooth sphere's (body frame, square-root encoded, see
// terrain.rs `encode_normal_offset`), so a smooth disc shades from its exact direction. Normals
// stored whole quantised the sphere's own curvature into a grid near the terminator (04-014).
fn ground_normal(in: VertexOut) -> vec3<f32> {
    // Half the offset is stored (terrain.rs encode_offset), so slopes past 60 degrees fit.
    let e = textureSample(normal_tex, normal_smp, node_map_uv(in)).xyz * 2.0 - 1.0;
    let nb = normalize(normalize(in.body_dir) + 2.0 * e * abs(e));
    return normalize(in.axis_x * nb.x + in.axis_y * nb.y + in.axis_z * nb.z);
}

// The normal map's alpha: `tanh(h / SHORE_CODE_M)` of the ground's height at the pixel (terrain.rs).
const SHORE_CODE_M: f32 = 8.0;
fn shore_code(in: VertexOut) -> f32 {
    return textureSample(normal_tex, normal_smp, node_map_uv(in)).a * 2.0 - 1.0;
}

// Where the fragment reads the node's normal map (texel centres span the node), moved by up to
// half a texel with a noise as SE's `PlanetUVShatter` (Fbm2 x HALF_TEXEL) does: it hides the
// texel grid where the map is magnified.
fn node_map_uv(in: VertexOut) -> vec2<f32> {
    let res = f32(textureDimensions(normal_tex).x);
    let q = vec3<f32>(in.uv * (res - 1.0) * 0.5, 0.0);
    let jitter = vec2<f32>(fbm3(q, 3), fbm3(q + vec3<f32>(19.1, 7.3, 3.7), 3)) * 0.5;
    let t = clamp(in.uv * (res - 1.0) + jitter, vec2<f32>(0.0), vec2<f32>(res - 1.0));
    return (t + 0.5) / res;
}

// Depth of the water at the pixel (m): from the shore code near the coast (finer than the
// mesh), from the mesh's height further out where the code saturates.
fn water_depth(in: VertexOut) -> f32 {
    let c = shore_code(in);
    let h = select(in.height, SHORE_CODE_M * atanh(clamp(c, -0.999, 0.999)), abs(c) < 0.95);
    return max(-h, 0.0);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    @if(SELENA_AT_VERTICES) {
        g_selena = in.selena;
    }
    @if(TERRA_AT_VERTICES) {
        g_terra = in.terra;
    }
    @if(TERRA_PARTIAL) {
        g_terra_avd = in.terra_avd;
        g_terra_k = in.terra_k;
    }
    let n = ground_normal(in);
    let up = normalize(in.up);
    let l = normalize(lit[planet.slot].sun_dir.xyz);
    let v = normalize(-in.world_pos);
    g_footprint_m = length(fwidth(in.world_pos)) * planet.planet.w / max(in.centre.w, 1e-30);
    // With the water hidden (SE's ShowWater filter, faded by `ice.w`) the seabed is dry ground.
    let under = planet.ocean_sea.w > 0.5 && in.cam_alt < 0.0 && lit[planet.slot].levels.w > 0.5;
    let s = lit_ground(in, n, up, l, table_pos(up, max(in.height, 0.0)));
    var col = s.col;
    // SE's AWATER path: the underwater fog over the ground and, past the 3D water, the flat sea.
    if planet.ocean_sea.w > 0.5 {
        col = se_ground_water(in, s, col, up, v);
    }
    // Under water the view is all water: no aerial perspective or night lights over it.
    return vec4<f32>(select(finish(in, col, up, l, s.e_sun, s.e_city, s.city, s.lamp), col, under), 1.0);
}

// SE's 3D water (water.glsl): the sea surface near the camera, drawn over the ground pass with
// premultiplied alpha. Lit as the ground pass's flat sea (Cook-Torrance on the waves' normal, the
// water's colour, ambient), its opacity the Fresnel term (and from under water the absorption's
// fog), whole to 1 / WaterParams.z from the camera and gone at twice that.
@fragment
fn water_fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let up = normalize(in.up);
    let l = normalize(lit[planet.slot].sun_dir.xyz);
    // Within a centimetre or so of the surface, distant nodes can place the camera on the other
    // side of it than near ones do (f32 node offsets); mirror the view into the side each branch
    // expects so such a node still shades as grazing water rather than black.
    let v0 = normalize(-in.world_pos);
    let v = v0 - up * 2.0 * min(dot(v0, up), 0.0);
    g_footprint_m = length(fwidth(in.world_pos)) * planet.planet.w / max(in.centre.w, 1e-30);
    let pos_km = table_pos(up, 0.0);
    let s = lit_ground(in, up, up, l, pos_km);
    let d = length(in.world_pos) * planet.planet.w / max(in.centre.w, 1e-30) / 1000.0;
    // waterFade3D (water.glsl:348-355).
    let fade_x = d * water_fade_z(in.cam_alt) - 1.0;
    let fade = 1.0 - clamp(fade_x, 0.0, 1.0);
    // The waves are the port's own (SE's NoiseMap texture is generated by code not decoded), full to
    // 10 km and gone at 20 km as SE's wavesFade.
    let a0 = max(planet.params.w * planet.params.w, 0.002);
    let a_facet = min(a0, 0.04);
    let waves = clamp(2.0 - d * 0.1, 0.0, 1.0);
    let nw = normalize(mix(up, wave_normal(in, up, 0.5 * (a0 * a0 - a_facet * a_facet)).xyz, waves * waves));
    // WaterSurfColor.a: the ShowWater animation, squared.
    let a = lit[planet.slot].levels.w * lit[planet.slot].levels.w;
    let col = planet.ocean_sea.rgb;
    let horizon = water_horizon_opacity(in.cam_alt);

    // From above there is no fog (water.glsl:400-403). The ground pass drew the ground's haze under
    // the surface (`finish` of nothing), so only the surface's share of it is added.
    let sa = sea_light(s, nw, v, planet.params.w);
    let alpha_a = clamp(0.02 + horizon * 0.98 * pow(1.0 - abs(dot(v, nw)), 4.0), 0.0, 1.0) * a * fade;
    let surf = sa.spec * (planet.params.y * a * fade) + col * (sa.diff + water_amb_light(s, col)) * alpha_a;
    let above = vec4<f32>(finish(in, surf, up, l, s.e_sun, s.e_city, 0.0, 1.0) - finish(in, vec3<f32>(0.0), up, l, s.e_sun, s.e_city, 0.0, 1.0) * (1.0 - alpha_a), alpha_a);

    // From under water the whole path is in the water: the surface (lit from above, so no glint
    // towards the camera) fades through the absorption into the underwater fog, which also makes it
    // opaque with distance.
    let att = clamp(exp(-d * a * planet.water_fog), vec4<f32>(0.0), vec4<f32>(1.0));
    let op_w = 1.0 - att.a;
    let sb = sea_light(s, nw, v0, planet.params.w);
    let alpha_b = clamp(op_w + 0.02 + horizon * 0.98 * pow(1.0 - abs(dot(v0, nw)), 4.0), 0.0, 1.0) * a * fade;
    let diff_b = mix(col * sb.diff * att.rgb, col * water_fog_light(s), op_w);
    let rest_b = col * water_amb_light(s, col) * att.rgb * (1.0 - op_w);
    let spec_b = sb.spec * (planet.params.y * a) * att.rgb * (1.0 - op_w);
    let below = vec4<f32>(spec_b * fade + (diff_b + rest_b) * alpha_b, alpha_b);

    // Land (by the water map) shows through: the ground below sea level there is dry. The
    // flanges past the node's edges (uv outside 0..1) only close the cracks seen from below where
    // the fog has made the surface opaque; elsewhere they would blend a second layer of surface
    // over the neighbour.
    let flange = any(in.uv < vec2<f32>(-1e-4)) || any(in.uv > vec2<f32>(1.0 + 1e-4));
    let flange_off = in.cam_alt >= 0.0 || op_w < 0.95;
    if s.water < 0.5 || (flange && flange_off) || lit[planet.slot].levels.w <= 0.0 || fade_x > 1.0 {
        discard;
    }
    return select(above, below, in.cam_alt < 0.0);
}
