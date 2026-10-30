// Planet terrain patch: textured with SpaceEngine cube-map tiles, lit through
// the body's precomputed atmosphere when it has one.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::globals;
import super::atmosphere_common::{AtmParams, ATM_AMBIENT_SCALE, atm_surface_irradiance, atm_apply_aerial, atm_transmittance_sun, atm_sky_radiance};
import super::eclipse::{Eclipse, eclipse_light};

struct Planet {
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    planet: vec4<f32>,
    rect_color: vec4<f32>,
    rect_lights: vec4<f32>,
    rect_water: vec4<f32>,
    rect_clouds: vec4<f32>,
    base_color: vec4<f32>,
    params: vec4<f32>,
    ocean_sea: vec4<f32>,
    ocean_shelf: vec4<f32>,
    // x: atmosphere enabled, y: sun irradiance scale for the tables (SE hdrPlanetSurfaceBright),
    // z: the scattered light (haze, the sky seen in water) relative to y: Bright x hdrAtmoScatterBright / y
    atm_flags: vec4<f32>,
    proc_pal0: vec4<f32>,
    proc_pal1: vec4<f32>,
    proc_pal2: vec4<f32>,
    proc_pal3: vec4<f32>,
    // x: surface kind id, y: relief (m), z: temperature (K), w: has life
    proc_params: vec4<f32>,
    // x: radius (m), y: seed fraction, z: procedural colour enabled
    proc_params2: vec4<f32>,
    atm: AtmParams,
    eclipse: Eclipse,
    // Own rings: x inner, y outer radius (planet radii), z opacity, w 0 none / 0.5 untextured / 1 textured.
    ring: vec4<f32>,
    // xyz: ring plane normal (render orientation).
    ring_normal: vec4<f32>,
    // xy: node corner on its cube face (m, wrapped), z: node edge (m); for soil textures.
    // w: split distance in node edges, for the geomorph (0: none).
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
    // z ThermalEmissionShift (kK).
    thermal2: vec4<f32>,
    // The further stars (SE lights a body by up to 4, planet.glsl's NLights loop): xyz direction,
    // w 1 when used; rgb their light, intensity included.
    light_dir: array<vec4<f32>, 3>,
    light_color: array<vec4<f32>, 3>,
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
    // per soil: x BiasRough, y ScaleRough, z BiasHeight, w ScaleHeight
    soil_bias: array<vec4<f32>, 32>,
    // per soil: xyz its Color in HSV (SE's rgb2hsl of the stored colour), w its Slope
    soil_color: array<vec4<f32>, 32>,
    // x biomes on, y soils on, z vegetation on, w number of soils
    flags: vec4<f32>,
    // SE climate parameters (surface_tex::climate_uniform); climate[4].w 0 when unused.
    climate: array<vec4<f32>, 9>,
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
@group(constants::MATERIAL_BIND_GROUP) @binding(15) var<uniform> biomes: Biomes;
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
};

// Camera height above the sphere of radius planet.w (m). Exact to centimetres: worked out from
// the camera in node-local metres (small near the camera) rather than from planet-scale floats.
// The node centre lies on that sphere.
fn camera_local(world_from_local: mat4x4<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(world_from_local[0].xyz, world_from_local[1].xyz, world_from_local[2].xyz);
    return (transpose(m) * -world_from_local[3].xyz) / dot(m[0], m[0]);
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
    let split_m = planet.soil_tile.z * planet.soil_tile.w;
    if split_m > 0.0 {
        let d = length(v.position - camera_local(world_from_local));
        morph = clamp((2.0 * split_m - d) / (0.6 * split_m), 0.0, 1.0);
    }
    let local = v.position + v.morph.xyz * (1.0 - morph);
    let world = mesh_position_local_to_world(world_from_local, vec4<f32>(local, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_pos = world.xyz;
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.uv = v.uv;
    let centre = (world_from_local * vec4<f32>(planet.planet.xyz, 1.0)).xyz;
    o.up = normalize(world.xyz - centre);
    o.height = v.height + v.morph.w * (1.0 - morph);
    o.axis_x = normalize(world_from_local[0].xyz);
    o.axis_y = normalize(world_from_local[1].xyz);
    o.axis_z = normalize(world_from_local[2].xyz);
    o.centre = vec4<f32>(centre, planet.planet.w * length(world_from_local[0].xyz));
    o.body_dir = v.body_dir;
    o.cam_alt = camera_altitude(world_from_local);
    o.tangent_u = normalize((world_from_local * vec4<f32>(planet.water.xyz, 0.0)).xyz);
    return o;
}

// ---------------------------------------------------------------------------
// Procedural surface colour (bodies without texture maps).

fn hash31(p: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 0x8da6b343u) ^ (bitcast<u32>(p.y) * 0xd8163841u) ^ (bitcast<u32>(p.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967295.0);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let b = vec3<i32>(i);
    let a000 = hash31(b);
    let a100 = hash31(b + vec3<i32>(1, 0, 0));
    let a010 = hash31(b + vec3<i32>(0, 1, 0));
    let a110 = hash31(b + vec3<i32>(1, 1, 0));
    let a001 = hash31(b + vec3<i32>(0, 0, 1));
    let a101 = hash31(b + vec3<i32>(1, 0, 1));
    let a011 = hash31(b + vec3<i32>(0, 1, 1));
    let a111 = hash31(b + vec3<i32>(1, 1, 1));
    let x0 = mix(mix(a000, a100, u.x), mix(a010, a110, u.x), u.y);
    let x1 = mix(mix(a001, a101, u.x), mix(a011, a111, u.x), u.y);
    return mix(x0, x1, u.z) * 2.0 - 1.0;
}

fn fbm3(p: vec3<f32>, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var o = 0; o < octaves; o = o + 1) {
        sum += amp * vnoise(q);
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
        amp *= 0.5;
    }
    return sum;
}

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

// ---------------------------------------------------------------------------
// SE's gas giant clouds (tg_common.glh HeightMapCloudsGasGiant, TurbulenceGasGiant,
// CycloneNoiseGasGiant; tg_gasgiant_color.glsl), with SE's hash Perlin noise, Fbm (lacunarity
// 2.218, H 0.5) and spherical Fibonacci cells as in tg_height.wesl. The point is the unit
// direction in the body frame (y the pole).

fn gg_hash(gridcell_in: vec3<f32>) -> array<vec4<f32>, 6> {
    const OFFSET = vec2<f32>(50.0, 161.0);
    const DOMAIN: f32 = 69.0;
    const SOMELARGEFLOATS = vec3<f32>(635.298681, 682.357502, 668.926525);
    const ZINC = vec3<f32>(48.500388, 65.294118, 63.934599);
    var gridcell = gridcell_in;
    gridcell = gridcell - floor(gridcell * (1.0 / DOMAIN)) * DOMAIN;
    let inc1 = select(gridcell + vec3<f32>(1.0), vec3<f32>(0.0), gridcell > vec3<f32>(DOMAIN - 1.5));
    var P = vec4<f32>(gridcell.xy, inc1.xy) + OFFSET.xyxy;
    P *= P;
    P = P.xzxz * P.yyww;
    let lowz = vec4<f32>(1.0) / (SOMELARGEFLOATS.xyzx + vec2<f32>(gridcell.z, inc1.z).xxxy * ZINC.xyzx);
    let highz = vec2<f32>(1.0) / (SOMELARGEFLOATS.yz + inc1.zz * ZINC.yz);
    return array<vec4<f32>, 6>(fract(P * lowz.xxxx), fract(P * lowz.yyyy), fract(P * lowz.zzzz), fract(P * lowz.wwww), fract(P * highz.xxxx), fract(P * highz.yyyy));
}

fn gg_noise(p: vec3<f32>) -> f32 {
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf1 = Pf - 1.0;
    let h = gg_hash(Pi);
    let C = vec4<f32>(0.49999);
    let gx0 = h[0] - C;
    let gy0 = h[1] - C;
    let gz0 = h[2] - C;
    let gx1 = h[3] - C;
    let gy1 = h[4] - C;
    let gz1 = h[5] - C;
    let r0 = inverseSqrt(gx0 * gx0 + gy0 * gy0 + gz0 * gz0) * (vec2<f32>(Pf.x, Pf1.x).xyxy * gx0 + vec2<f32>(Pf.y, Pf1.y).xxyy * gy0 + Pf.zzzz * gz0);
    let r1 = inverseSqrt(gx1 * gx1 + gy1 * gy1 + gz1 * gz1) * (vec2<f32>(Pf.x, Pf1.x).xyxy * gx1 + vec2<f32>(Pf.y, Pf1.y).xxyy * gy1 + Pf1.zzzz * gz1);
    let b = Pf * Pf * Pf * (Pf * (Pf * 6.0 - 15.0) + 10.0);
    let m0 = mix(r0, r1, b.z);
    let m1 = mix(m0.xy, m0.zw, b.y);
    return mix(m1.x, m1.y, b.x) * 1.1547005383792515;
}

fn gg_fbm(point: vec3<f32>, octaves: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    let gain = pow(2.218281828459, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        sum += gg_noise(pt) * ampl;
        ampl *= gain;
        pt *= 2.218281828459;
    }
    return sum;
}

fn gg_hash1(p: f32) -> f32 {
    return fract(sin(p) * 158.5453123);
}

fn gg_rotate(angle: f32, axis: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    let t = 1.0 - c;
    let m = mat3x3<f32>(
        t * axis.x * axis.x + c, t * axis.x * axis.y - s * axis.z, t * axis.x * axis.z + s * axis.y,
        t * axis.x * axis.y + s * axis.z, t * axis.y * axis.y + c, t * axis.y * axis.z - s * axis.x,
        t * axis.x * axis.z - s * axis.y, t * axis.y * axis.z + s * axis.x, t * axis.z * axis.z + c);
    return v * m;
}

// SE's inverseSF: the nearest of n spherical Fibonacci points (cell index, distance) and the point.
fn gg_isf(p: vec3<f32>, n: f32, nearest: ptr<function, vec3<f32>>) -> vec2<f32> {
    const phi: f32 = 1.61803398875;
    let m = 1.0 - 1.0 / n;
    let fi = min(atan2(p.y, p.x), PI);
    let cosTheta = p.z;
    let k = max(2.0, floor(log(n * PI * sqrt(5.0) * (1.0 - cosTheta * cosTheta)) / log(phi + 1.0)));
    let Fk = pow(phi, k) / sqrt(5.0);
    let F = vec2<f32>(floor(Fk + 0.5), floor(Fk * phi + 0.5));
    let ka = 2.0 * F / n;
    let kb = 2.0 * PI * (fract((F + 1.0) * phi) - (phi - 1.0));
    let det = ka.y * kb.x - ka.x * kb.y;
    let iB = mat2x2<f32>(ka.y / det, -ka.x / det, kb.y / det, -kb.x / det);
    let c = floor(iB * vec2<f32>(fi, cosTheta - m));
    var d = 8.0;
    var j = 0.0;
    for (var s = 0; s < 4; s++) {
        let uv = vec2<f32>(f32(s - 2 * (s / 2)), f32(s / 2));
        let i = dot(F, uv + c);
        let fs = 2.0 * PI * fract(i * phi);
        let ct = m - 2.0 * i / n;
        // Past the pole SE's sqrt is NaN and the candidate never wins.
        if abs(ct) > 1.0 {
            continue;
        }
        let st = sqrt(1.0 - ct * ct);
        let q = vec3<f32>(cos(fs) * st, sin(fs) * st, ct);
        let rr = q - p;
        let d2 = dot(rr, rr);
        if d2 < d {
            *nearest = q;
            d = d2;
            j = i;
        }
    }
    return vec2<f32>(j, sqrt(d));
}

fn gg_turbulence(point_in: vec3<f32>) -> vec3<f32> {
    let twist = biomes.giant[0].w;
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    var strength = 5.5;
    var freq = 800.0 * 0.7;
    var size = 15.0 * 0.7;
    let dens = 0.8;
    for (var i = 0; i < 5; i++) {
        let cell = gg_isf(point, freq, &center);
        let rnd = gg_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.5 * dens - rnd);
            let dist = clamp(1.0 - r, 0.0, 1.0);
            let dist2 = clamp(0.5 - r, 0.0, 1.0);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.25);
            twisted = gg_rotate(dir * twist * sign(center.y) * fi, center, point);
        }
        freq = min(freq * 2.0, 1600.0);
        size = min(size * 1.2, 30.0);
        strength = strength * 1.5;
        point = twisted;
    }
    return twisted;
}

fn gg_cyclones(point_in: vec3<f32>, offset: ptr<function, f32>) -> vec3<f32> {
    let cy = biomes.giant[2];
    let cy2 = biomes.giant[3];
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    // Equatorial oval cyclones.
    var offs = 0.6;
    var squeeze = 2.27;
    var strength = 2.5;
    var freq = cy.y * 50.0;
    var dens = cy.z * 0.02;
    var size = 5.0;
    for (var i = 0; f32(i) < cy.w; i++) {
        let cell = gg_isf(vec3<f32>(point.x, point.y * squeeze, point.z), freq, &center);
        let rnd = gg_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.7 * dens - rnd);
            let dist = clamp(1.0 - r, 0.0, 1.0);
            let dist2 = clamp(0.5 - r, 0.0, 1.0);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.5);
            twisted = gg_rotate(cy.x * dir * sign(center.y + 0.001) * fi, center, point);
            *offset += offs * fi * dir;
        }
        freq = min(freq * 2.0, 6400.0);
        dens = min(dens * 3.5, 0.3);
        size = min(size * 1.5, 15.0);
        offs = offs * 0.85;
        squeeze = max(squeeze - 0.3, 1.0);
        strength *= 1.3;
        point = twisted;
    }
    // Polar round cyclones.
    strength = 7.125;
    freq = cy2.y * 800.0;
    dens = 0.5;
    size = 8.0;
    offs = cy2.x * smoothstep(cy2.z - 0.1, cy2.z + 0.1, abs(point.y));
    for (var i = 0; f32(i) < cy2.w; i++) {
        let cell = gg_isf(point, freq, &center);
        let rnd = gg_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.7 * dens - rnd);
            let dist = clamp(1.0 - r, 0.0, 1.0);
            let dist2 = clamp(0.5 - r, 0.0, 1.0);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.5);
            twisted = gg_rotate(cy.x * dir * sign(center.y + 0.001) * fi, center, point);
            *offset += offs * fi * dir;
        }
        freq = min(freq * 1.8, 6400.0);
        size = min(size * 1.2, 15.0);
        offs = offs * 0.95;
        strength *= 1.3;
        point = twisted;
    }
    return twisted;
}

fn gg_height(point: vec3<f32>) -> f32 {
    let g0 = biomes.giant[0];
    let randomize = biomes.giant[4].xyz;
    var twisted = point;
    // Zones.
    var zp = randomize * 7.1563;
    zp.y += twisted.y * g0.z * 0.5;
    let zones = gg_noise(zp) * 0.6 + 0.25;
    var offset = 0.0;
    if biomes.giant[2].w > 0.0 {
        twisted = gg_cyclones(twisted, &offset);
    }
    twisted = gg_turbulence(twisted);
    // Stripes.
    let turb = gg_fbm(twisted * 0.2, g0.y);
    twisted = twisted * (0.00515 * g0.x) - turb * 0.0005;
    twisted.y *= 100.0 + turb;
    let height = biomes.giant[1].x * (gg_fbm(twisted, g0.y) * 0.7 + 0.5);
    return zones + height + offset;
}

// tg_gasgiant_color: the cloud height times the palette's colour at that height (SE's
// GetGasGiantCloudsColor over its 8 layers), then from gamma to linear (`colorGamma`).
fn gas_giant_albedo(dir: vec3<f32>) -> vec3<f32> {
    let h = gg_height(dir);
    let hs = 7.0 / 21.0;
    let y = clamp(h * hs, 0.5 * hs, 1.0 - 0.5 * hs);
    let f = (y - 0.5 * hs) / (1.0 - hs) * 7.0;
    let i0 = u32(clamp(floor(f), 0.0, 7.0));
    let i1 = min(i0 + 1u, 7u);
    let c = mix(biomes.giant_palette[i0].rgb, biomes.giant_palette[i1].rgb, fract(f));
    return pow(clamp(h * c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
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

// As a factor around 1: the texture divided by its mean (the 1x1 mip), so far away (where the
// mips average it) it fades to 1 and never shifts the albedo.
fn sample_layer(l: vec4<f32>, pos_m: vec3<f32>, w: vec3<f32>) -> SampleLook {
    let layer = i32(max(l.x, 0.0));
    // A slow warp bends the texture grid so its repeats do not line up into a visible pattern.
    let p0 = pos_m / l.y;
    let p = p0 + vec3<f32>(vnoise(p0 * 0.07 + vec3<f32>(3.1)), vnoise(p0 * 0.07 + vec3<f32>(7.7)), vnoise(p0 * 0.07 + vec3<f32>(1.3))) * 1.2;
    let dx = dpdx(p);
    let dy = dpdy(p);
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
    if close > 0.0 || biomes.flags.y < 0.5 {
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

// Relative weights of the 12 biome slots.
fn biome_weights(h: f32, slope: f32, lat: f32, n: f32, moist: f32) -> array<f32, 12> {
    var w: array<f32, 12>;
    let relief = max(planet.proc_params.y, 1.0);
    let temp = planet.proc_params.z;
    let hn = h / relief + n * 0.08;
    let sea = planet.ocean_sea.w > 0.5;
    let polar = smoothstep(0.75, 0.95, lat + n * 0.05);
    let cliff = smoothstep(0.3, 0.5, slope);
    var under = 0.0;
    if sea {
        under = 1.0 - smoothstep(-5.0, 0.0, h);
        let bottom = 1.0 - smoothstep(-0.5, -0.15, hn);
        w[0] = under * bottom;
        w[1] = under * (1.0 - bottom);
        let beach_h = max(relief * 0.01, 15.0);
        w[2] = (1.0 - under) * (1.0 - smoothstep(beach_h * 0.5, beach_h, h)) * (1.0 - cliff);
    }
    let land = (1.0 - under) * (1.0 - w[2]);
    let low = 1.0 - smoothstep(0.2, 0.4, hn);
    let high = smoothstep(0.55, 0.75, hn);
    let up = (1.0 - low) * (1.0 - high);
    // Snow line: high and polar ground on temperate worlds, everything on cold ones.
    let cold = clamp((270.0 - temp) / 60.0, 0.0, 1.0);
    let snow_line = mix(1.1, -0.3, cold) - polar * 0.7;
    let snow = smoothstep(snow_line, snow_line + 0.1, hn) * (1.0 - cliff * 0.7);
    // Ice: frozen worlds, and polar caps on cool ones.
    let ice = max(1.0 - smoothstep(130.0, 170.0, temp), polar * (1.0 - smoothstep(250.0, 290.0, temp))) * (1.0 - cliff);
    var lava = 0.0;
    if abs(planet.proc_params.x - 3.0) < 0.5 {
        lava = 1.0 - smoothstep(0.1, 0.3, hn);
    }
    let bare = land * (1.0 - snow) * (1.0 - ice) * (1.0 - lava);
    w[5] = bare * max(cliff, high);
    var veg = 0.0;
    if biomes.flags.z > 0.5 {
        veg = smoothstep(250.0, 270.0, temp) * (1.0 - smoothstep(310.0, 330.0, temp)) * (1.0 - polar);
    }
    let ground = bare * (1.0 - cliff);
    w[3] = ground * low * (1.0 - veg);
    w[4] = ground * up * (1.0 - veg * 0.5);
    let lv = ground * low * veg;
    w[9] = lv * (1.0 - smoothstep(0.3, 0.5, moist)) + ground * up * veg * 0.5;
    w[10] = lv * smoothstep(0.3, 0.5, moist) * (1.0 - smoothstep(0.55, 0.7, moist));
    w[11] = lv * smoothstep(0.55, 0.7, moist);
    w[6] = land * snow * (1.0 - ice);
    w[7] = land * ice;
    w[8] = land * lava * (1.0 - snow);
    return w;
}

// Weight of a vegetation band of the climate index (lo < 0: the body has no such vegetation).
fn climate_band(x: f32, lo: f32, hi: f32) -> f32 {
    if lo < 0.0 {
        return 0.0;
    }
    return smoothstep(lo - 0.03, lo + 0.03, x) * (1.0 - smoothstep(hi - 0.03, hi + 0.03, x));
}

// Biome weights from SE's generated climate parameters (surface_tex::climate_uniform), heights as
// fractions of BumpHeight. SE's terrain shader is encrypted, so this reading is designed: a climate
// index runs equator -> tropic band -> pole and rises with height (heightTempGrad); its steppe,
// grass and forest ranges place vegetation; snowLevel and icecapLatitude (2 = none) place snow and
// polar ice; beachWidth sets the beach.
fn se_biome_weights(h: f32, slope: f32, s0: f32, n: f32, moist: f32) -> array<f32, 12> {
    var w: array<f32, 12>;
    let c0 = biomes.climate[0];
    let c1 = biomes.climate[1];
    let c2 = biomes.climate[2];
    let c3 = biomes.climate[3];
    let c4 = biomes.climate[4];
    let bump = max(c4.z, 1.0);
    let sea = c0.x;
    // h is relative to the reference radius: sea level, or the mean height of dry worlds.
    let hn = biomes.climate[5].z + h / bump;
    let s = clamp(s0 + n * 0.04, 0.0, 1.0);
    let cliff = smoothstep(0.3, 0.5, slope);
    var under = 0.0;
    if planet.ocean_sea.w > 0.5 {
        under = 1.0 - smoothstep(-5.0, 0.0, h);
        let bottom = 1.0 - smoothstep(sea - 0.25, sea - 0.08, hn + n * 0.02);
        w[0] = under * bottom;
        w[1] = under * (1.0 - bottom);
        let beach_h = max(c2.w * bump, 5.0);
        w[2] = (1.0 - under) * (1.0 - smoothstep(beach_h * 0.5, beach_h, h)) * (1.0 - cliff);
    }
    let land = (1.0 - under) * (1.0 - w[2]);
    let tl = c1.w;
    let tw = c2.x;
    var cl = c1.y;
    if s < tl - tw {
        cl = mix(c1.z, c1.y, s / max(tl - tw, 1e-3));
    } else if s > tl + tw {
        cl = mix(c1.y, c1.x, (s - tl - tw) / max(1.0 - tl - tw, 1e-3));
    }
    cl = cl + c2.y * max(hn - sea, 0.0) + n * biomes.climate[5].x;
    let snow = smoothstep(c0.y, c0.y + 0.02, hn + n * 0.01) * (1.0 - cliff * 0.7);
    let ice = smoothstep(c0.z - 0.015, c0.z + 0.015, s) * (1.0 - cliff * 0.5);
    var lava = 0.0;
    if abs(planet.proc_params.x - 3.0) < 0.5 {
        lava = 1.0 - smoothstep(sea + 0.05, sea + 0.15, hn);
    }
    let bare = land * (1.0 - snow) * (1.0 - ice) * (1.0 - lava);
    let base = biomes.climate[5].w;
    let up = (hn - base) / max(1.0 - base, 0.05) + n * 0.1;
    let low = 1.0 - smoothstep(0.15, 0.35, up);
    let high = smoothstep(0.55, 0.8, up);
    w[5] = bare * max(cliff, high);
    let ground = bare * (1.0 - cliff) * (1.0 - high);
    var steppe = 0.0;
    var grass = 0.0;
    var forest = 0.0;
    if biomes.flags.z > 0.5 {
        let wet = clamp(c2.z * 2.0 * moist, 0.0, 1.0);
        steppe = climate_band(cl, c3.x, c3.y);
        grass = climate_band(cl, c3.z, c3.w) * mix(0.5, 1.0, wet);
        forest = climate_band(cl, c4.x, c4.y) * wet;
    }
    let veg = clamp(max(steppe, max(grass, forest)), 0.0, 1.0);
    w[3] = ground * low * (1.0 - veg);
    w[4] = ground * (1.0 - low) * (1.0 - veg);
    w[9] = ground * steppe;
    w[10] = ground * grass;
    w[11] = ground * forest;
    w[6] = land * snow * (1.0 - ice);
    w[7] = land * ice;
    w[8] = land * lava * (1.0 - snow);
    return w;
}

// SE's terrain noise for the colour map (tg_common.glh Fbm, Fbm3D and RidgedMultifractal over SE's
// hash Perlin): octaves finer than about a pixel (`fp`, radians on the unit sphere) are dropped,
// where SE's coarser tiles average them.
fn ter_oct(octaves: f32, freq: f32, lac: f32, fp: f32) -> f32 {
    return clamp(floor(log2(0.5 / max(freq * fp, 1e-30)) / log2(lac)) + 1.0, 0.0, octaves);
}

fn ter_fbm(point: vec3<f32>, octaves: f32, lac: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    let gain = pow(lac, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        sum += gg_noise(pt) * ampl;
        ampl *= gain;
        pt *= lac;
    }
    return sum;
}

fn ter_fbm3d(point: vec3<f32>, octaves: f32) -> vec3<f32> {
    var pt = point;
    var sum = vec3<f32>(0.0);
    var ampl = 1.0;
    let lac = 2.218281828459;
    let gain = pow(lac, -0.5);
    for (var i = 0; f32(i) < octaves; i++) {
        sum += vec3<f32>(gg_noise(pt), gg_noise(pt + vec3<f32>(3.33, 5.71, 1.96)), gg_noise(pt + vec3<f32>(7.77, 2.65, 4.37))) * ampl;
        ampl *= gain;
        pt *= lac;
    }
    return sum;
}

// RidgedMultifractal with SE's noiseOffset 0.8, noiseRidgeSmooth 0.0001 and noiseH 0.5.
fn ter_rmf(point: vec3<f32>, gain: f32, octaves: f32) -> f32 {
    var signal = 1.0;
    var sum = 0.0;
    var freq = 1.0;
    for (var i = 0; f32(i) < octaves; i++) {
        let weight = saturate(signal * gain);
        signal = gg_noise(point * freq);
        signal = 0.8 - sqrt(0.0001 + signal * signal);
        signal *= signal * weight;
        sum += signal * pow(freq, -0.5);
        freq *= 2.218281828459;
    }
    return sum;
}

// The direction to the star in the body frame (set by `lit_ground`): the substellar point that SE's
// colour shaders put on +x for tidally locked worlds.
var<private> g_sun_body: vec3<f32> = vec3<f32>(1.0, 0.0, 0.0);

// SE's PlantsBlendCoeff, its 0.025 ramps widened by `cw` (the climate's change across the pixel).
fn plants_coeff(lo: f32, hi: f32, climate: f32, fade: f32, trans_fractal: f32, cw: f32) -> f32 {
    let t = fade * smoothstep(lo - cw, lo + 0.025 + cw, climate) * (1.0 - smoothstep(hi - 0.025 - cw, hi + cw, climate));
    return saturate(t + trans_fractal * pow(abs(t - 0.5), 0.125));
}

// SE's sequential BlendMaterials as slot weights: the surface so far gives way to slot `k` by `t`.
fn blend_slot(w: ptr<function, array<f32, 12>>, k: i32, t: f32) {
    for (var j = 0; j < 12; j++) {
        (*w)[j] *= 1.0 - t;
    }
    (*w)[k] += t;
}

struct TerraMix {
    w: array<f32, 12>,
    // The cliff layers' tint of the rock slot (SE: cliff colour x (1 - (0, 0.5, 1) layers)).
    rock_tint: vec3<f32>,
    // The colour then goes `mix(colorVary, 1, vary)` x `dark`, then towards white by `white`.
    vary: f32,
    dark: f32,
    white: f32,
};

// SE's ColorMapSelena (tg_selena_color.glsl), airless worlds: the base surface by height bands, an
// albedo variation weakened on slopes and, on cratered worlds, outside younger patches; the
// leading/trailing hemisphere darkening (drivenDarkening) and ice caps whitened. Rayed craters,
// cracks and the hills' Jordan turbulence are not ported.
fn se_selena_mix(point: vec3<f32>, h: f32, slope: f32, fp: f32) -> TerraMix {
    let c0 = biomes.climate[0];
    let rnd = biomes.climate[6].xyz;
    let sea = c0.x;
    let lac = 2.218281828459;
    var o: TerraMix;
    o.rock_tint = vec3<f32>(1.0);
    o.dark = 1.0;
    var w: array<f32, 12>;
    let lat_noise = 0.15 * (ter_fbm(point * 0.7 + rnd, ter_oct(6.0, 0.7, lac, fp), lac) - 1.0);
    var latitude: f32;
    if biomes.climate[6].w <= 0.0 {
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
    let avd = ter_fbm3d((point + rnd) * 0.07, ter_oct(3.0, 0.07, lac, fp)) * (1.5 + biomes.climate[8].y);
    let slope_mod = 1.0 - slope;
    var vary = saturate(1.0 - ter_fbm((point + avd) * 0.378, ter_oct(8.0, 0.378, lac, fp), lac) * slope_mod * slope_mod * 2.0);
    if biomes.climate[7].w > 0.05 {
        let yd = ter_fbm3d((point - rnd) * 0.07, ter_oct(4.0, 0.07, lac, fp)) * 1.1;
        var young = 1.0 - ter_fbm(point + yd, ter_oct(8.0, 1.0, lac, fp), lac);
        young = smoothstep(0.0, 1.0, young * young * young);
        vary = mix(0.0, vary, young);
    }
    o.vary = vary;
    let dd = biomes.climate[8].x;
    if dd != 0.0 {
        var z = -point.z * sign(dd) + 0.2 * ter_fbm(point * 1.63, ter_oct(3.0, 1.63, lac, fp), lac);
        z = saturate(1.0 - z);
        z *= z;
        o.dark = mix(1.0 - abs(dd), 1.0, z);
    }
    o.white = 0.8 * saturate((latitude - c0.z + 0.3) * 2.0);
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
fn se_terra_mix(point: vec3<f32>, h: f32, slope: f32, fp: f32) -> TerraMix {
    let c0 = biomes.climate[0];
    let c1 = biomes.climate[1];
    let c2 = biomes.climate[2];
    let c3 = biomes.climate[3];
    let c4 = biomes.climate[4];
    let rnd = biomes.climate[6].xyz;
    let sea = c0.x;
    let snow_level = c0.y;
    let lat_ice = c0.z;
    let pole = c1.x;
    let tropic = c1.y;
    let equator = c1.z;
    let lat_tropic = c1.w;
    let tropic_width = c2.x;
    let lac = 2.218281828459;
    var o: TerraMix;
    o.rock_tint = vec3<f32>(1.0);
    var w: array<f32, 12>;
    o.dark = 1.0;

    var latitude: f32;
    var climate: f32;
    let lat_noise = 0.15 * (ter_fbm(point * 0.7 + rnd, ter_oct(6.0, 0.7, lac, fp), lac) - 1.0);
    if biomes.climate[6].w <= 0.0 {
        latitude = saturate(abs(point.y) + lat_noise);
        if latitude < lat_tropic - tropic_width {
            climate = mix(tropic, equator, saturate((lat_tropic - latitude - tropic_width) / lat_tropic));
        } else if latitude > lat_tropic + tropic_width {
            climate = mix(tropic, pole, saturate((latitude - lat_tropic - tropic_width) / (1.0 - lat_tropic)));
        } else {
            climate = tropic;
        }
    } else {
        latitude = 1.0 - dot(point, g_sun_body) + lat_noise;
        climate = mix(tropic, pole, saturate(latitude));
    }

    var vary = ter_fbm(point * 1700.0 + rnd, ter_oct(5.0, 1700.0, 3.5, fp), 3.5);
    let snow_line = h + 0.25 * vary * slope;
    let mont_height = saturate((h - sea) / (snow_level - sea));
    climate = min(climate + c2.y * mont_height, pole - 0.125);
    // SE's sharp transitions, softened where they would change within a pixel: the slope comes from
    // the node's normal map, coarser than SE's colour tiles, and would flip whole texels.
    climate = mix(climate, pole, saturate((snow_line - snow_level) * min(100.0, 0.5 / max(fwidth(snow_line), 1e-6))));
    let ice_cap = saturate((latitude / lat_ice - 1.0) * 50.0);
    climate = mix(climate, pole, ice_cap);

    let hills = biomes.climate[7];
    let pf = 0.0005 * hills.x / max(hills.y * hills.y, 1e-6);
    let pp = (point + rnd) * pf;
    let fr = 0.20 * (1.5 - ter_rmf(pp, 2.0, ter_oct(4.0, pf, lac, fp)))
        + 0.05 * (1.5 - ter_rmf(pp * 10.0, 2.0, ter_oct(4.0, pf * 10.0, lac, fp)))
        + 0.02 * (1.5 - ter_rmf(pp * 100.0, 2.0, ter_oct(4.0, pf * 100.0, lac, fp)));
    let df = biomes.climate[5].y * 0.005;
    var p = point * df + vec3<f32>(fr);
    p += ter_fbm3d(p * 0.38, ter_oct(4.0, df * 0.38, lac, fp)) * 1.2;
    vary = ter_fbm(p, ter_oct(4.0, df, lac, fp), lac) * 0.35 + 0.245;
    climate += vary * saturate(1.0 - 3.0 * slope) * saturate(1.0 - 1.333 * climate);
    let cw = fwidth(climate);

    // The base surface: beach at sea level, then lowland, upland and rock up to the top; the shelf
    // and bottom below.
    let hb = (h - sea) / max(1.0 - sea, 1e-3) * 4.0 + 2.0;
    let h0 = clamp(i32(floor(hb)), 0, 5);
    let h1 = clamp(h0 + 1, 0, 5);
    let dh = fract(hb);
    w[h0] += 1.0 - dh;
    w[h1] += dh;

    if biomes.flags.z > 0.5 {
        let ptf = abs(0.125 * ter_fbm(point * 3.0e5, ter_oct(2.0, 3.0e5, lac, fp), lac) + 0.125 * ter_fbm(point * 1.0e3, ter_oct(2.0, 1.0e3, lac, fp), lac));
        let pavd = ter_fbm3d((point + rnd) * 0.07, ter_oct(4.0, 0.07, lac, fp)) * 1.3;
        let humidity = ter_fbm((point + pavd) * 1.73, ter_oct(8.0, 1.73, lac, fp), lac) - 1.0 + c2.z * 2.0;
        let beach = c2.w;
        let fade = smoothstep(beach, beach * 2.0, h - sea) * (1.0 - smoothstep(0.65, 0.75, slope)) * smoothstep(-0.5, 0.5, humidity);
        var t = plants_coeff(c3.x, c3.y, climate, fade, ptf, cw);
        if t > 0.0 {
            blend_slot(&w, 9, t);
        }
        t = plants_coeff(c4.x, c4.y, climate, fade, ptf, cw);
        if t > 0.0 {
            blend_slot(&w, 11, t);
        }
        t = plants_coeff(c3.z, c3.w, climate, fade, ptf, cw);
        if t > 0.0 {
            blend_slot(&w, 10, t);
        }
    }
    if climate > 0.9 - cw {
        blend_slot(&w, 6, smoothstep(0.9 - cw, 0.92 + cw, climate));
    }
    if ice_cap == 0.0 && slope > 0.35 {
        let q = point * 378.3 + vec3<f32>(gg_noise(point * 967.7), gg_noise(point * 967.7 + vec3<f32>(3.33, 5.71, 1.96)), gg_noise(point * 967.7 + vec3<f32>(7.77, 2.65, 4.37))) * 10.0;
        let v = saturate(ter_fbm(q, ter_oct(5.0, 378.3, lac, fp), lac) * 0.7 + 0.5);
        let layers = ter_fbm(vec3<f32>(h * 168.4 + 0.17 * v, 0.43 * (q.x + q.y), 0.43 * (q.z - q.y)), ter_oct(4.0, 168.4 * 0.43 * 378.3, lac, fp), lac);
        let t = smoothstep(0.35, 0.65, slope);
        o.rock_tint = mix(vec3<f32>(1.0), vec3<f32>(1.0) - vec3<f32>(0.0, 0.5, 1.0) * layers, t / max(w[5] * (1.0 - t) + t, 1e-4));
        blend_slot(&w, 5, t);
    }
    // SE's albedoVary: 1 - 0.5 Fbm of the point distorted by 1.3 Fbm3D((point + Randomize) 0.07),
    // cubed and smoothstepped.
    let avd = ter_fbm3d((point + rnd) * 0.07, ter_oct(4.0, 0.07, lac, fp)) * 1.3;
    let av = 1.0 - 0.5 * ter_fbm(point + avd, ter_oct(8.0, 1.0, lac, fp), lac);
    o.vary = smoothstep(0.0, 1.0, av * av * av);
    o.w = w;
    return o;
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
};

// One biome slot: its large and small sample layers (colours multiplied, reliefs added), times
// its tint if `tinted`.
fn biome_look(k: i32, pos_m: vec3<f32>, tw: vec3<f32>, tinted: bool) -> BiomeLook {
    let tint = select(vec3<f32>(1.0), biomes.color[k].rgb, tinted);
    let a = sample_layer(biomes.large[k], pos_m, tw);
    let b = sample_layer(biomes.small[k], pos_m, tw);
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
    let se = biomes.climate[4].w > 0.5;
    var m: TerraMix;
    m.vary = -1.0;
    m.dark = 1.0;
    if se {
        // SE's height map value: sea level (or the dry worlds' reference) plus the height in
        // BumpHeights. SE's generator: 0 terra, 1 selena (airless), 2 asteroid (as selena here).
        let hv = biomes.climate[5].z + h / max(biomes.climate[4].z, 1.0);
        if biomes.climate[7].z > 0.5 {
            m = se_selena_mix(dir, hv, slope, fp);
        } else {
            m = se_terra_mix(dir, hv, slope, fp);
        }
        w = m.w;
        rock_tint = m.rock_tint;
    } else {
        let n = fbm3(dir * (radius / 30000.0) + seed, 4);
        let moist = fbm3(dir * (radius / 400000.0) + seed.zxy, 4) * 0.8 + 0.5;
        w = biome_weights(h, slope, abs(dir.y), n, moist);
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
    var b1 = biome_look(k1, pos_m, tw, tinted);
    var b2 = biome_look(k2, pos_m, tw, tinted);
    if k1 == 5 {
        b1.col *= rock_tint;
    }
    if k2 == 5 {
        b2.col *= rock_tint;
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

// A soil layer with SE's zones: the noise picking them repeats with the soil coordinates, so
// neighbouring nodes stay seamless.
fn soil_layer(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, period_m: f32, rot: bool) -> SoilTexel {
    let n = max(i32(round(SOIL_WRAP_M / period_m)), 1);
    let kz = 8.0 * clamp(vnoise_tiled(vec3<f32>(uv, f32(l * 3)), vec2<i32>(n, n)).x * 0.5 + 0.5, 0.0, 0.9999);
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
    let s = biomes.soil[k];
    let md = biomes.soil_mod[k];
    let b = biomes.soil_bias[k];
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
    // Colour: (detail x Modul brightness) moved in HSV by the ground minus the texture's mean.
    let c = det * mix(1.0, lum, md.x);
    let c_avg = det_avg * mix(1.0, lum_avg, md.x);
    var hsv = rgb2hsv(c) + surf - rgb2hsv(c_avg);
    hsv = vec3<f32>(fract(hsv.x), clamp(hsv.yz, vec2<f32>(0.0), vec2<f32>(1.0)));
    o.col = to_linear(hsv2rgb(hsv));
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
    let sc = biomes.soil_color[k];
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
    for (var k = 0; k < i32(biomes.flags.w); k++) {
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
    // fine layer (sparkle). It fades out until a pixel spans a few such steps.
    let q = smoothstep(4.0, 16.0, max(length(dx), length(dy)) / max(planet.soil_tile.z * 1.2e-7, 1e-12));
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

// How fast daylight fades with depth, per metre: blue sea water loses red within metres and keeps
// blue for tens; other seas by their colour.
fn water_dimming() -> vec3<f32> {
    let c = planet.ocean_sea.rgb / max(max(planet.ocean_sea.r, planet.ocean_sea.g), max(planet.ocean_sea.b, 1e-4));
    let k = 1.0 - c;
    return vec3<f32>(0.025) + 0.35 * k * k * k;
}

// Extinction of a line of sight through the water, per metre (absorption plus scattering: some
// 25 m of visibility in clear blue water).
fn water_extinction() -> vec3<f32> {
    return water_dimming() + vec3<f32>(0.04);
}

// Light the water scatters toward the eye at `depth` (m) looking along `d`: the sea colour lit by
// what reaches that depth, brighter looking up toward the surface.
fn water_glow(e_surf: vec3<f32>, depth: f32, d: vec3<f32>, up: vec3<f32>) -> vec3<f32> {
    let look_up = smoothstep(-0.4, 1.0, dot(d, up));
    return planet.ocean_sea.rgb / PI * e_surf * exp(-water_dimming() * max(depth, 0.0)) * mix(1.5, 8.0, look_up);
}

// Sky radiance seen from sea level at `pos_km` along `dir` (turned up to the horizon at least).
fn sky_seen(pos_km: vec3<f32>, dir: vec3<f32>, up: vec3<f32>, l: vec3<f32>, e_sun: vec3<f32>) -> vec3<f32> {
    let d = normalize(dir + up * max(0.01 - dot(dir, up), 0.0));
    return atm_sky_radiance(planet.atm, inscatter_tex, transmittance_tex, atm_smp, pos_km * 1.000001, d, l).radiance * e_sun;
}

// GGX glint of the star on water with normal `n` and roughness `a` (alpha).
fn water_glint(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, a_in: f32) -> f32 {
    let nl = dot(n, l);
    if nl <= 0.0 {
        return 0.0;
    }
    let a = max(a_in, 0.01);
    let nv = max(dot(n, v), 0.05);
    let hv = normalize(l + v);
    let nh = max(dot(n, hv), 0.0);
    let dd = nh * nh * (a * a - 1.0) + 1.0;
    let ndf = a * a / (PI * dd * dd);
    let fh = 0.02 + 0.98 * pow(1.0 - max(dot(hv, v), 0.0), 5.0);
    let kg = a * 0.5;
    let vis = 1.0 / ((nl * (1.0 - kg) + kg) * (nv * (1.0 - kg) + kg));
    return ndf * fh * vis * 0.25 * nl;
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
};

// SE's planetary sphere shadow (planet.glsl:1087-1095): a point `height` m up is lit until the star
// (at `mu` = cos of its zenith angle) sets below the point's own horizon, with a sharp edge, so
// peaks and crater rims catch the light past the terminator.
fn horizon_shadow(height: f32, mu: f32) -> f32 {
    let rr = clamp(1.0 / (1.0 + max(height, 0.0) / max(planet.planet.w, 1.0)), 0.0, 0.9999);
    return clamp((sqrt(1.0 - rr * rr) + mu) * 500.0, 0.0, 1.0);
}

// The ground's colour lit at `pos_km` (no water, clouds or haze yet).
fn lit_ground(in: VertexOut, n: vec3<f32>, up: vec3<f32>, l: vec3<f32>, pos_km: vec3<f32>) -> Surf {
    let has_atm = planet.atm_flags.x > 0.5;
    let p_rel = (in.world_pos - in.centre.xyz) / max(in.centre.w, 1e-30);
    let shadow = eclipse_light(planet.eclipse, p_rel, l) * ring_shadow(p_rel, l);
    let sun = planet.sun_color.rgb * planet.sun_dir.w * shadow;
    let e_sun = sun * planet.atm_flags.y;

    var albedo = planet.base_color.rgb;
    var emission = vec3<f32>(0.0);
    let bdir = normalize(in.body_dir);
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
    if biomes.flags.x > 0.5 {
        let b = biome_albedo(bdir, in.height, slope, !textured, fp);
        biome = b.col;
        // SE's ColorVary (tg_terra_color.glsl:82-158): the colour darkens towards it over
        // planet-wide patches, `mix(colorVary, 1, albedoVary)`, albedoVary = 1 - 0.5 Fbm of the
        // point distorted by 1.3 Fbm3D((point + Randomize) 0.07), cubed and smoothstepped.
        if biomes.color_vary.w > 0.5 && b.vary >= 0.0 {
            biome *= mix(biomes.color_vary.xyz, vec3<f32>(1.0), b.vary);
        }
        if b.vary >= 0.0 {
            biome = mix(biome * b.dark, vec3<f32>(1.0), b.white);
        }
        let g = in.axis_x * b.grad.x + in.axis_y * b.grad.y + in.axis_z * b.grad.z;
        nb = normalize(n - (g - n * dot(g, n)));
        sample_rough = b.rough;
        sample_k = b.k;
    }
    if textured {
        // SE tints the colour map by the catalog's ModulateColor (Mars, Venus, Titan).
        albedo = textureSample(color_tex, color_smp, tile_uv(planet.rect_color, in.uv)).rgb * planet.modulate.rgb;
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
        if biomes.giant[1].y > 0.5 {
            albedo = gas_giant_albedo(bdir);
        } else if biomes.flags.x > 0.5 && planet.proc_params.x < 5.5 {
            albedo = biome;
        } else {
            albedo = proc_albedo(bdir, in.height, slope, g_footprint_m);
        }
    }
    // Thermal emission, by the unshadowed star (the heating is from its light at the surface).
    if planet.thermal.x > 0.45 || planet.thermal.z > 0.04 {
        emission = thermal_emission(bdir, in.height, dot(up, l), planet.sun_color.rgb * planet.sun_dir.w * planet.atm_flags.y);
    }
    // The soils, chosen by the map's colour and recoloured to the ground's, take over within 2 km
    // (SE's `detailFade = clamp(2 - distance_km, 0, 1)`, planet.glsl).
    var ns = nb;
    var rough = sample_rough;
    var fade = 0.0;
    if biomes.flags.y > 0.5 {
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
        if planet.light_dir[i].w < 0.5 {
            break;
        }
        let lx = normalize(planet.light_dir[i].xyz);
        let sun_x = planet.light_color[i].rgb * eclipse_light(planet.eclipse, p_rel, lx) * ring_shadow(p_rel, lx);
        let e_x = sun_x * planet.atm_flags.y;
        let term_x = horizon_shadow(in.height, dot(up, lx));
        var ground_x = sun_x * term_x;
        if has_atm {
            s.col += albedo / PI * atm_surface_irradiance(planet.atm, transmittance_tex, irradiance_tex, atm_smp, pos_km, ns, lx, e_x, 1.0);
            s.e_level += atm_surface_irradiance(planet.atm, transmittance_tex, irradiance_tex, atm_smp, pos_km, up, lx, e_x, 1.0);
            let r = length(pos_km);
            ground_x = sun_x * atm_transmittance_sun(planet.atm, transmittance_tex, atm_smp, r, dot(pos_km / r, lx));
        } else {
            s.col += albedo / PI * max(dot(ns, lx), 0.0) * term_x * e_x;
            s.e_level += max(dot(up, lx), 0.0) * term_x * e_x;
        }
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
    s.col += albedo / PI * planet.base_color.w * planet.atm_flags.y + emission;
    s.e_level += vec3<f32>(planet.base_color.w * planet.atm_flags.y);
    // The soils' sheen: GGX with their roughness (dielectric, F0 0.04), only where the
    // mesh's own surface faces the star.
    // Beyond the soils' 2 km, the sample layers' roughness where they show.
    let sheen = mix(sample_k, 1.0, fade);
    if sheen > 0.0 {
        let v = normalize(-in.world_pos);
        s.col += s.sun_at_ground * planet.atm_flags.y * ground_spec(ns, v, l, rough * rough) * smoothstep(0.0, 0.1, dot(n, l)) * sheen;
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

// The seabed (`s.col`) `depth` m down seen from above, through shallow water fading into the deep
// colour along the light path (down to the bed and back up along `v`).
fn ocean_under(s: Surf, depth: f32, up: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let mu = max(dot(up, v), 0.05);
    let nl = max(dot(up, l), 0.0);
    let trans = exp(-max(depth, 0.0) * (1.0 + 1.0 / mu) / planet.ocean_shelf.w);
    var deep = planet.ocean_sea.rgb * nl * s.sun_at_ground;
    if planet.atm_flags.x > 0.5 {
        deep = planet.ocean_sea.rgb / PI * s.e_level;
    }
    return s.col * trans + deep * (1.0 - trans);
}

// The sea surface seen from above: the sky's reflection and the star's glint (rgb), GGX with SE's
// RoughnessWater (params.w) and SpecBrightWater (params.y) on the waves' normal `nw`, and the
// Fresnel reflectance (w); what lies below shows through the rest.
fn ocean_surface(s: Surf, pos_km: vec3<f32>, up: vec3<f32>, nw: vec3<f32>, v: vec3<f32>, l: vec3<f32>, a: f32) -> vec4<f32> {
    let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(nw, v), 0.0), 5.0);
    let spec_k = max(planet.params.y, 0.05);
    // The sky in the waves (a flat 8% of the sunlight without an atmosphere).
    var sky = s.sun_at_ground * 0.08;
    if planet.atm_flags.x > 0.5 {
        sky = sky_seen(pos_km, reflect(-v, nw), up, l, s.e_sun * planet.atm_flags.z);
    }
    // x10: the HDR gain the earlier glint was matched to SE with.
    return vec4<f32>(s.sun_at_ground * water_glint(nw, v, l, a) * spec_k * 10.0 + sky * fresnel, fresnel);
}

// Water seen from above with the seabed painted into it (lakes and shores the sea's own surface
// does not cover).
fn ocean_from_above(s: Surf, pos_km: vec3<f32>, depth: f32, up: vec3<f32>, nw: vec3<f32>, v: vec3<f32>, l: vec3<f32>, a: f32) -> vec3<f32> {
    let surf = ocean_surface(s, pos_km, up, nw, v, l, a);
    return ocean_under(s, depth, up, v, l) * (1.0 - surf.w) + surf.rgb;
}

// Cloud shadows, city lights at night and aerial perspective, for a camera above the sea.
fn finish(in: VertexOut, col_in: vec3<f32>, up: vec3<f32>, l: vec3<f32>, e_sun: vec3<f32>) -> vec3<f32> {
    var col = col_in;
    // Cloud shadows (the clouds themselves are a separate shell).
    var cloud = 0.0;
    if planet.rect_clouds.w > 0.5 {
        cloud = clamp(textureSample(clouds_tex, clouds_smp, tile_uv(planet.rect_clouds, in.uv)).r * planet.params.z, 0.0, 1.0);
        col *= 1.0 - 0.55 * cloud;
    }

    // City lights on the night side.
    if planet.rect_lights.w > 0.5 {
        let glow = textureSample(lights_tex, lights_smp, tile_uv(planet.rect_lights, in.uv)).rgb;
        let night = 1.0 - smoothstep(-0.15, 0.05, dot(up, l));
        // The lights map is a blurred density of city light (about a km per texel at
        // best); seen from close by it would light the ground evenly, so let it fade
        // in only once a pixel covers a good stretch of ground.
        let glow_fade = smoothstep(30.0, 600.0, g_footprint_m);
        col += glow * night * planet.params.x * glow_fade * (1.0 - clamp(cloud, 0.0, 1.0) * 0.7);
    }

    // Aerial perspective: haze and attenuation between the camera and the ground.
    if planet.atm_flags.x > 0.5 {
        let to_table = planet.atm.radii.x / max(in.centre.w, 1e-30);
        let cam_km = (-in.centre.xyz) * to_table;
        // Aerial perspective subtracts two inscatter lookups along the view ray; keep both
        // ends in the same (interpolated) geometry or the difference blows up.
        let view_km = (in.world_pos - in.centre.xyz) * to_table;
        col = atm_apply_aerial(planet.atm, inscatter_tex, transmittance_tex, atm_smp, col, cam_km, view_km, l, e_sun * planet.atm_flags.z);
        // The further stars' haze.
        for (var i = 0; i < 3; i++) {
            if planet.light_dir[i].w < 0.5 {
                break;
            }
            let e_x = planet.light_color[i].rgb * planet.atm_flags.y * planet.atm_flags.z;
            col += atm_apply_aerial(planet.atm, inscatter_tex, transmittance_tex, atm_smp, vec3<f32>(0.0), cam_km, view_km, normalize(planet.light_dir[i].xyz), e_x);
        }
    }
    return col;
}

// Under water everything is seen through the water: `col` fades over the path of `dist_m` (from
// `cam_depth` down to `end_depth`) into the water's glow. Most of that glow comes from the first
// visibility length, so it takes the light at the depth there (or the path's middle if shorter).
fn through_water(col: vec3<f32>, e_level: vec3<f32>, dist_m: f32, cam_depth: f32, end_depth: f32, d: vec3<f32>, up: vec3<f32>) -> vec3<f32> {
    let ext = water_extinction();
    let tr = exp(-ext * dist_m);
    let seen = min(0.5, 1.0 / (ext.b * max(dist_m, 1e-3)));
    return col * tr + water_glow(e_level, mix(cam_depth, end_depth, seen), d, up) * (1.0 - tr);
}

// Under water the eye adapts as the daylight above fades: up to twice as bright when the sun is
// low (SE's sea at sunset is about 1.7 times the port's otherwise).
fn underwater_exposure(e_level: vec3<f32>, e_sun: vec3<f32>) -> f32 {
    let w = vec3<f32>(0.2126, 0.7152, 0.0722);
    return clamp(sqrt(0.5 * dot(e_sun, w) / max(dot(e_level, w), 1e-12)), 1.0, 2.0);
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
fn table_pos(up: vec3<f32>, h: f32) -> vec3<f32> {
    return up * planet.atm.radii.x * (1.0 + h / max(planet.planet.w, 1.0));
}

// The ground's shading normal from the node's normal map (texel centres span the node).
fn ground_normal(in: VertexOut) -> vec3<f32> {
    let nb = textureSample(normal_tex, normal_smp, node_map_uv(in)).xyz * 2.0 - 1.0;
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
    let n = ground_normal(in);
    let up = normalize(in.up);
    let l = normalize(planet.sun_dir.xyz);
    let v = normalize(-in.world_pos);
    g_footprint_m = length(fwidth(in.world_pos)) * planet.planet.w / max(in.centre.w, 1e-30);
    // Texture lookups stay out of branches on the camera's depth (uniform control flow); the
    // results are picked at the end.
    let under = planet.ocean_sea.w > 0.5 && in.cam_alt < 0.0;
    let s = lit_ground(in, n, up, l, table_pos(up, max(in.height, 0.0)));
    var col = s.col;
    // Water above sea level (lakes, by the water map) and along the shore, with its surface. Under
    // the sea's own (see-through) surface only the water's depth: the seabed is drawn where it
    // is, so it no longer looks painted on the surface as the camera comes down to it.
    let wd = water_depth(in);
    if s.water > 0.0 {
        let a = max(planet.params.w * planet.params.w, 0.002);
        // Where the sea's mesh lies in front of the ground it draws the surface itself.
        let sea = in.height < 0.0 && s.water >= 0.5;
        let lake = ocean_from_above(s, table_pos(up, max(in.height, 0.0)), wd, up, up, v, l, a);
        col = mix(col, select(lake, ocean_under(s, wd, up, v, l), sea), clamp(s.water, 0.0, 1.0));
    }
    let above = finish(in, col, up, l, s.e_sun);

    // Under water: the seabed lit by the light that gets down to it, seen through the water.
    let depth = max(-in.height, 0.0);
    let bed = s.col * exp(-water_extinction() * depth * 1.3);
    let dist = length(in.world_pos) * planet.planet.w / max(in.centre.w, 1e-30);
    let below = through_water(bed, s.e_level, dist, -in.cam_alt, depth, -v, up) * underwater_exposure(s.e_level, s.e_sun);
    return vec4<f32>(select(above, below, under), 1.0);
}

@fragment
fn water_fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let up = normalize(in.up);
    let l = normalize(planet.sun_dir.xyz);
    // Within a centimetre or so of the surface, distant nodes can place the camera on the other
    // side of it than near ones do (f32 node offsets); mirror the view into the side each branch
    // expects so such a node still shades as grazing water rather than black.
    let v0 = normalize(-in.world_pos);
    let v = v0 - up * 2.0 * min(dot(v0, up), 0.0);
    g_footprint_m = length(fwidth(in.world_pos)) * planet.planet.w / max(in.centre.w, 1e-30);
    let pos_km = table_pos(up, 0.0);
    let s = lit_ground(in, up, up, l, pos_km);
    // The waves carry SE's roughness close by; a little stays on each facet.
    let a0 = max(planet.params.w * planet.params.w, 0.002);
    let a_facet = min(a0, 0.04);
    let nw = wave_normal(in, up, 0.5 * (a0 * a0 - a_facet * a_facet));
    let a = sqrt(a_facet * a_facet + 2.0 * nw.w);
    // From above the surface is see-through (premultiplied alpha): its reflection over the seabed
    // the ground pass drew, whose haze (`finish` of nothing) is already in that colour.
    let w = clamp(s.water, 0.0, 1.0);
    let surf = ocean_surface(s, pos_km, up, nw.xyz, v, l, a);
    let above = vec4<f32>((finish(in, surf.rgb, up, l, s.e_sun) - finish(in, vec3<f32>(0.0), up, l, s.e_sun) * (1.0 - surf.w)) * w, surf.w * w);

    // From below: the sky through Snell's window (the light refracted out), beyond it the water's
    // own depths reflected from the surface; then the water between the camera and the surface.
    let d = -v0 + up * 2.0 * max(dot(v0, up), 0.0);
    let n_down = -nw.xyz;
    let ci = clamp(dot(d, nw.xyz), 0.0, 1.0);
    let eta = 1.333;
    let st2 = eta * eta * (1.0 - ci * ci);
    let depth = -in.cam_alt;
    // Reflected from the surface: the water just below it, looking down.
    var col = water_glow(s.e_level, 0.0, reflect(d, n_down), up);
    if st2 < 1.0 {
        let ct = sqrt(1.0 - st2);
        let rs = (eta * ci - ct) / (eta * ci + ct);
        let rp = (eta * ct - ci) / (eta * ct + ci);
        let f = 0.5 * (rs * rs + rp * rp);
        let t = refract(d, n_down, eta);
        var sky = s.e_level / PI * (0.3 + 0.7 * max(dot(t, up), 0.0));
        if planet.atm_flags.x > 0.5 {
            sky = sky_seen(pos_km, t, up, l, s.e_sun * planet.atm_flags.z);
        }
        // The star itself, spread by the waves.
        sky += s.sun_at_ground * water_glint(t, t, l, max(a, 0.05)) * 4.0;
        col = sky * (1.0 - f) + col * f;
    }
    let dist = length(in.world_pos) * planet.planet.w / max(in.centre.w, 1e-30);
    let below = through_water(col, s.e_level, dist, depth, 0.0, d, up) * underwater_exposure(s.e_level, s.e_sun);
    // Land (by the water map) shows through: the ground below sea level there is dry. The
    // flanges past the node's edges (uv outside 0..1) only close the cracks seen from below;
    // from above they would blend a second layer of surface over the neighbour.
    let flange = any(in.uv < vec2<f32>(-1e-4)) || any(in.uv > vec2<f32>(1.0 + 1e-4));
    if s.water < 0.5 || (flange && in.cam_alt >= 0.0) {
        discard;
    }
    return select(above, vec4<f32>(below, 1.0), in.cam_alt < 0.0);
}
