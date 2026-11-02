// Cloud shell patch, lit as SpaceEngine lights a cloud layer (planet.glsl, ACLOUDS): the star's
// light reddened through the atmosphere and cut by the planet's shadow at the cloud's height, the
// sky's irradiance (times SkyLight), SE's day ambient, a Hapke mix and subsurface scattering, then
// the haze between the camera and the cloud. Alpha-blended (straight colour; SE's is premultiplied).

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import super::eclipse::{EclipseLight, eclipse_light};
import super::atmosphere_common::{AtmParams, ATM_AMBIENT_SCALE, atm_transmittance_sun, atm_irradiance, atm_apply_aerial};

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
    // The ground's (unused here).
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
    // x seed, y unused, z latitude belt strength
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
    let x0 = mix(mix(hash31(b), hash31(b + vec3<i32>(1, 0, 0)), u.x),
                 mix(hash31(b + vec3<i32>(0, 1, 0)), hash31(b + vec3<i32>(1, 1, 0)), u.x), u.y);
    let x1 = mix(mix(hash31(b + vec3<i32>(0, 0, 1)), hash31(b + vec3<i32>(1, 0, 1)), u.x),
                 mix(hash31(b + vec3<i32>(0, 1, 1)), hash31(b + vec3<i32>(1, 1, 1)), u.x), u.y);
    return mix(x0, x1, u.z);
}

// fBm in [0, 1] whose finest octaves fade once smaller than a pixel (`fw` = pixel
// footprint in the same units as `p`).
fn fbm_aa(p: vec3<f32>, octaves: i32, fw: f32) -> f32 {
    var s = 0.0;
    var norm = 0.0;
    var amp = 0.5;
    var q = p;
    var f = 1.0;
    for (var i = 0; i < octaves; i++) {
        let w = clamp(1.0 - fw * f * 1.5, 0.0, 1.0);
        s += amp * mix(0.5, vnoise(q), w);
        norm += amp;
        amp *= 0.58;
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
        f *= 2.03;
    }
    return s / norm;
}

// ---------------------------------------------------------------------------
// SE's terra clouds (tg_common.glh HeightMapCloudsTerra, CycloneNoiseTerra, TurbulenceTerra,
// FbmClouds; tg_clouds_color.glsl), with SE's hash Perlin noise and spherical Fibonacci cells as in
// planet.wesl's gas giants. The point is the unit direction in the layer's frame (y the pole).

fn se_hash(gridcell_in: vec3<f32>) -> array<vec4<f32>, 6> {
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

fn se_noise(p: vec3<f32>) -> f32 {
    let Pi = floor(p);
    let Pf = p - Pi;
    let Pf1 = Pf - 1.0;
    let h = se_hash(Pi);
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

fn se_hash1(p: f32) -> f32 {
    return fract(sin(p) * 158.5453123);
}

fn se_rotate(angle: f32, axis: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
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
fn se_isf(p: vec3<f32>, n: f32, nearest: ptr<function, vec3<f32>>) -> vec2<f32> {
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

// FbmClouds / FbmClouds3D: amplitude x 0.333 and frequency x pi per octave.
fn se_fbm_clouds(point: vec3<f32>, octaves: f32) -> f32 {
    var pt = point;
    var sum = 0.0;
    var ampl = 1.0;
    for (var i = 0; f32(i) < octaves; i++) {
        sum += se_noise(pt) * ampl;
        ampl *= 0.333;
        pt *= 3.1416;
    }
    return sum;
}

fn se_fbm_clouds3(point: vec3<f32>, octaves: f32) -> vec3<f32> {
    var pt = point;
    var sum = vec3<f32>(0.0);
    var ampl = 1.0;
    for (var i = 0; f32(i) < octaves; i++) {
        sum += vec3<f32>(se_noise(pt), se_noise(pt + vec3<f32>(3.33, 5.71, 1.96)), se_noise(pt + vec3<f32>(7.77, 2.65, 4.37))) * ampl;
        ampl *= 0.333;
        pt *= 3.1416;
    }
    return sum;
}

struct CycloneOut {
    p: vec3<f32>,
    weight: f32,
    coverage: f32,
};

// CycloneNoiseTerra: cyclones in random spherical Fibonacci cells, octave by octave (twice the
// frequency and density each), twisting the point around the cell centre.
fn se_cyclones(point_in: vec3<f32>, coverage_in: f32) -> CycloneOut {
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    var weight = 1.0;
    var coverage = coverage_in;
    let mag = -clouds.se[2].w * clouds.se[1].x;
    var freq = clouds.se[1].y * 50.0;
    var dens = clouds.se[1].z * 0.02;
    var size = 8.0;
    for (var i = 0; f32(i) < clouds.se[1].w; i++) {
        let cell = se_isf(point, freq, &center);
        let rnd = se_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dist = 1.0 - r;
            let fi = mix(log(r), dist * dist * dist, r);
            twisted = se_rotate(mag * sign(center.y + 0.001) * fi, center, point);
            let w = saturate(1.0 - r * 10.0);
            weight = min(weight, 1.0 - w * w);
            coverage = mix(coverage, 1.0, dist);
        }
        freq *= 2.0;
        dens *= 2.0;
        size *= 2.0;
        point = twisted;
    }
    var o: CycloneOut;
    o.p = twisted;
    o.weight = saturate(weight);
    o.coverage = coverage;
    return o;
}

// TurbulenceTerra: two octaves of small vortices.
fn se_turbulence(point_in: vec3<f32>) -> vec3<f32> {
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
    var strength = 5.5;
    var freq = 20.0 * 0.7;
    var size = 4.0 * 0.7;
    let dens = 0.3;
    for (var i = 0; i < 2; i++) {
        let cell = se_isf(point, freq, &center);
        let rnd = se_hash1(cell.x);
        let r = size * cell.y;
        if rnd < dens && r < 1.0 {
            let dir = sign(0.5 * dens - rnd);
            let dist = saturate(1.0 - r);
            let dist2 = saturate(0.5 - r);
            let fi = pow(dist, strength) * (exp(-6.0 * dist2) + 0.25);
            twisted = se_rotate(dir * 15.0 * sign(center.y + 0.001) * fi, center, point);
        }
        freq = min(freq * 2.0, 1600.0);
        size = min(size * 1.2, 30.0);
        strength *= 1.5;
        point = twisted;
    }
    return twisted;
}

// HeightMapCloudsTerra, then tg_clouds_color's `saturate(h^2 3.5 + h)` opacity. On tidally locked
// worlds one giant cyclone sits at the substellar point `sun` (SE's cloud frame puts it on y; here
// the star's direction in the layer's frame) and the night side stays clear. `fp`: the pixel's
// footprint (radians), finer noise octaves dropped.
fn se_terra_clouds(point: vec3<f32>, coverage_in: f32, fp: f32, sun: vec3<f32>) -> f32 {
    let se0 = clouds.se[0];
    let zones = cos(point.y * se0.z);
    let ang = zones * se0.w;
    var twisted = point;
    var coverage = coverage_in;
    var weight = 1.0;
    if clouds.se[2].w > 0.0 {
        let center = sun;
        let r = length(center - point);
        let mag = -clouds.se[2].w * clouds.se[1].x;
        if r < 1.0 {
            let dist = 1.0 - r;
            let fi = mix(log(r), dist * dist * dist, r);
            twisted = se_rotate(mag * fi, center, point);
            weight = saturate(r * 40.0 - 0.05);
            weight = weight * weight;
            coverage = mix(coverage, 1.0, dist);
        }
        weight *= smoothstep(-0.2, 0.0, dot(point, sun));
    } else {
        let c = se_cyclones(point, coverage);
        twisted = c.p;
        weight = c.weight;
        coverage = c.coverage;
    }
    twisted = se_turbulence(twisted);
    let sa = sin(ang);
    let ca = cos(ang);
    twisted = vec3<f32>(ca * twisted.x - sa * twisted.z, twisted.y, sa * twisted.x + ca * twisted.z);
    twisted = twisted * se0.x + clouds.se[2].xyz;
    let p = twisted * se0.x * 6.37;
    // Octaves down to about a tenth of a pixel (frequency x pi each): SE bakes every octave into its
    // tiles, and with only the coarse ones left, thresholded Perlin lobes show as a checkerboard.
    let base = se0.x * se0.x * 6.37;
    let oct = clamp(floor(log2(0.5 / max(base * fp, 1e-30)) / log2(3.1416)) + 3.0, 1.0, se0.y);
    let q = p + se_fbm_clouds3(p, oct);
    let r = p + se_fbm_clouds3(q, oct);
    let f = se_fbm_clouds(r, oct) * 0.7 + coverage - 0.3;
    let h = saturate(f) * weight;
    return saturate(h * h * 3.5 + h);
}

// Cloud cover of a procedural planet at direction `d` in the layer's frame (which turns with the
// layer's wind).
fn proc_clouds(d: vec3<f32>) -> f32 {
    let p = d;
    let freq = 7.0 * max(clouds.proc.y, 0.1);
    // Weather systems are stretched along the parallels.
    let ps = vec3<f32>(p.x, p.y * 1.7, p.z);
    let seed = vec3<f32>(clouds.proc2.x * 71.0, clouds.proc2.x * 13.0, clouds.proc2.x * 37.0);
    let fw = length(fwidth(p)) * freq;
    let warp = vec3<f32>(
        fbm_aa(ps * freq * 0.5 + seed, 4, fw * 0.5),
        fbm_aa(ps * freq * 0.5 + seed + vec3<f32>(5.2), 4, fw * 0.5),
        fbm_aa(ps * freq * 0.5 + seed + vec3<f32>(9.1), 4, fw * 0.5)) - vec3<f32>(0.5);
    let n = fbm_aa(ps * freq + warp * 2.2 + seed, i32(clamp(clouds.proc.z + 2.0, 4.0, 9.0)), fw);
    // Cloudier near the equator and at mid latitudes, clearer in the subtropics.
    let belts = 1.0 + clouds.proc2.z * (0.5 * cos(d.y * 9.42) - 0.1);
    let cov = clamp(clouds.proc.x * belts, 0.0, 1.0);
    let t = 0.5 + (0.5 - cov) * 0.45;
    let cover = smoothstep(t - 0.03, t + 0.1, n) * (0.55 + 0.45 * smoothstep(t, t + 0.2, n));
    // Full coverage closes the layer (Titan's haze, Coverage 1).
    return mix(cover, 1.0, smoothstep(0.9, 1.0, clouds.proc.x));
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
            density = se_terra_clouds(cdir, clouds.proc.x, cfp, in.sun_body);
        } else {
            density = proc_clouds(cdir);
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
    // SE drops texels under 5% and rescales the rest (`cloudsFade`).
    if density < 0.05 {
        discard;
    }
    // SE fades the layer out within 1 km of the camera (its planet units, planet.glsl).
    let km_per_unit = clouds.planet.w / max(in.centre.w, 1e-30) * 0.001;
    let near = smoothstep(0.0, 1.0, length(in.world_pos) * km_per_unit);
    let a = clamp((density - 0.05) / 0.95 * clouds.params.x, 0.0, 1.0) * near;

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
    return vec4<f32>(col, a);
}
