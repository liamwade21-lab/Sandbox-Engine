// Planetary rings, lit as SE's rings.glsl / rings_common.glh do on its flat (2D) ring: the radial
// texture's lit row (FrontMap, premultiplied by its alpha) scatters sunlight with a Hapke x Lambert
// phase function and an opposition spot, its back row (BackMap) forward-scatters with a Mie lobe
// (g = -0.95, so dusty rings brighten backlit); both become optical densities in a slab whose
// density falls off as exp(-3 |height|), marched in 8 steps along the view with SE's self-shadow
// towards the Sun (dimming the face turned from it). Each star is shadowed by the planet (an eclipse
// of its disc by the planet's, flattened as the planet) and by the moons' eclipses; the planet lights
// its rings too (SE's host light). A radial detail noise (RingDetailNoiseScale) breaks up close
// views. Premultiplied output.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import super::eclipse::{EclipseLight, eclipse_shadow, eclipse_value};

struct Ring {
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    /// x inner radius / planet radius, y outer / planet radius, z Opacity, w Brightness.
    geom: vec4<f32>,
    /// x FrontBright, y BackBright, z has texture, w PlanetShadow.
    params: vec4<f32>,
    /// SE's DensityParams: Density (front, back), Opacity, SelfShadow.
    density: vec4<f32>,
    /// x SpotBright, y 1 / SpotWidth, z Hapke, w SpotBrightCB.
    hapke: vec4<f32>,
    /// x 1 / SpotWidthCB, y detail noise frequency, z SE's Oblateness.g (1 / the polar semi-axis),
    /// w the eclipse intensity (AmbientColor.a).
    hapke2: vec4<f32>,
    /// The planet's own light on its rings (HostLightIndex): rgb at the rings' outer edge, w 1 when on.
    host: vec4<f32>,
    /// The further stars (SE's rings shaders loop over NLights): xyz direction, w 1 when used;
    /// rgb their light, intensity included.
    light_dir: array<vec4<f32>, 3>,
    light_color: array<vec4<f32>, 3>,
    /// Each light's eclipse casters (the main star first).
    eclipse: array<EclipseLight, 4>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> ring: Ring;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var ring_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var ring_smp: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) centre: vec3<f32>,
    @location(4) planet_radius: f32,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    o.world_pos = world.xyz;
    o.uv = v.uv;
    o.normal = normalize((m * vec4<f32>(0.0, 1.0, 0.0, 0.0)).xyz);
    o.centre = (m * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    // The mesh is in planet radii, so the model scale is the planet radius.
    o.planet_radius = length((m * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz);
    return o;
}

const TO_GRAY = vec3<f32>(0.299, 0.587, 0.114);
const PI: f32 = 3.14159265359;

// SE's RingsLambertBRDF: mu is the cosine of the phase angle.
fn lambert(mu: f32) -> f32 {
    return (mu + 1.0) * 0.5;
}

// SE's RingsHapkeBRDF: the shadow-hiding and coherent-backscatter opposition spots (French et al.
// 2007), t = tan(phase / 2).
fn hapke(mu: f32) -> f32 {
    let t = sqrt((1.0 - mu) / (1.0 + mu));
    let s = 1.0 + ring.hapke.x / (1.0 + t * ring.hapke.y) + ring.hapke.w / (1.0 + t * ring.hapke2.x);
    return mix(1.0, s * 0.5, ring.hapke.z);
}

// SE's RingsMieBRDF (Cornette-Shanks, normalised to 1 straight forward for g = -0.95).
fn mie(mu: f32, g: f32) -> f32 {
    let k = 1.5 / 806.202 * ((1.0 - g * g) / (2.0 + g * g));
    return k * (1.0 + mu * mu) * pow(1.0 + g * g - 2.0 * g * mu, -1.5);
}

// SE's slab, in units of its half-thickness (RingsSize; values assumed): the march starts on the
// viewer's face and runs to the far cap of a bounding cylinder `CAP` high; light paths leave the
// cylinder within `SIDE` sideways.
const CAP: f32 = 1.0;
const SIDE: f32 = 50.0;
const STEPS: i32 = 8;
// (1 - exp(-3)) / 3: the falloff's integral over a half-thickness.
const FALLOFF_INTEGRAL: f32 = 0.31674;

fn hash1(i: f32) -> f32 {
    return fract(sin(i * 127.1 + 311.7) * 43758.5453);
}

fn noise1(x: f32) -> f32 {
    let i = floor(x);
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(hash1(i), hash1(i + 1.0), u);
}

// SE's Fbm_texture along the radius (its noise texture at a fixed row): 4 octaves, lacunarity
// 2.218, gain 0.671, times 0.85.
fn detail_noise(x: f32) -> f32 {
    var sum = 0.0;
    var p = x;
    var amp = 1.0;
    for (var i = 0; i < 4; i++) {
        sum += amp * noise1(p);
        p *= 2.218281828459;
        amp *= 0.671415927;
    }
    return sum * 0.85 / 2.4;
}

// The light `sun` from direction `l` (already shadowed) the slab scatters towards the viewer, before
// `Brightness`, and in w the slab's transparency along the view: SE's march with the Hapke and Mie
// phase functions and self-shadowing. `transp_out`: the transparency per row (front, back, opacity).
fn ring_scatter(in: VertexOut, l: vec3<f32>, sun: vec3<f32>, front: vec4<f32>, back: vec4<f32>, od0: vec4<f32>, transp_out: ptr<function, vec3<f32>>) -> vec4<f32> {
    let v = normalize(-in.world_pos);
    let n = normalize(in.normal);
    let nv = dot(n, v);
    let nl = dot(n, l);
    let mu = clamp(dot(l, v), -0.999999, 0.999999);

    let lit = sun * front.rgb * (ring.params.x * hapke(mu) * lambert(mu));
    let dusty = sun * back.rgb * (ring.params.y * mie(mu, -0.95));
    // Heights on the viewer's side are positive; the march runs from +1 down to -CAP.
    let ey = max(abs(nv), 1e-4);
    let ly = nl * sign(nv);
    let step_s = (1.0 + CAP) / ey / f32(STEPS);
    var col = vec3<f32>(0.0);
    var transp = vec3<f32>(1.0);
    for (var i = 0; i < STEPS; i++) {
        let h = 1.0 - (f32(i) + 0.5) * step_s * ey;
        let od = od0 * exp(-3.0 * abs(h));
        // SE's volumetricShadowAnalythic: to the cylinder's cap (or side) towards the Sun, with the
        // falloff at the path's middle.
        let to_cap = (sign(ly) * CAP - h) / select(ly, 1e-4, abs(ly) < 1e-4);
        let dd = min(max(to_cap, 0.0), SIDE / max(sqrt(max(1.0 - ly * ly, 0.0)), 1e-4));
        let self_shadow = exp(-od.w * exp(-3.0 * abs(h + 0.5 * dd * ly)) * dd);
        let ext = exp(-step_s * od.xyz);
        col += (lit * (1.0 - ext.x) * transp.x + dusty * (1.0 - ext.y) * transp.y) * self_shadow;
        transp *= ext;
    }
    *transp_out = transp;
    return vec4<f32>(col, transp.z);
}

// What reaches the ring point of light `k` (0 the main star) from direction `l`: the planet's shadow
// as an eclipse of the light by the planet's disc (rings.glsl: EclipseValue of the light's and the
// planet's angular radii, the light's direction stretched by SE's Oblateness for a flattened planet,
// times the catalog's PlanetShadow), then the moons' and other bodies' eclipses (ECL,
// EclipseShadowAuto x AmbientColor.a), and none edge-on.
fn ring_shadow(in: VertexOut, k: i32, l: vec3<f32>) -> f32 {
    let e = ring.eclipse[k];
    let oc = in.centre - in.world_pos;
    let inv = inverseSqrt(max(dot(oc, oc), 1e-30));
    let pixel_dir = oc * inv;
    let n = normalize(in.normal);
    let ly = dot(l, n);
    let obl = ring.hapke2.z;
    let cos_a = dot(l, pixel_dir) / sqrt(max(1.0 + (obl * obl - 1.0) * ly * ly, 1e-12));
    let caster = asin(clamp(in.planet_radius * inv, 0.0, 1.0));
    let planet = clamp(1.0 - eclipse_value(max(e.info.x, 1e-7), caster, acos(clamp(cos_a, 0.0, 1.0))), 0.0, 1.0);
    var shadow = mix(1.0, planet, ring.params.w);
    if e.info.y > 0.5 {
        let p = (in.world_pos - in.centre) / in.planet_radius;
        shadow *= 1.0 - ring.hapke2.w * eclipse_shadow(e, p, l);
    }
    return shadow * smoothstep(0.0, 0.01, abs(ly));
}

// SE's host light (HostLightIndex, rings.glsl): the planet lighting its rings, falling off as the
// inverse square from the outer edge (HapkeParams2.z = outer / inner radius), by the planet's phase
// seen from the ring point (a Lambert sphere's, Chi the angle at the planet between the main star and
// the point) and its disc's size; Mie with g = -0.7, no self-shadow. Without self-shadow the march
// sums to `1 - transparency` per row (`transp`, the main march's).
fn host_scatter(in: VertexOut, l0: vec3<f32>, front: vec4<f32>, back: vec4<f32>, transp: vec3<f32>) -> vec3<f32> {
    let oc = in.centre - in.world_pos;
    let dist = max(length(oc), 1e-30);
    let pixel_dir = oc / dist;
    let dist_corr = ring.geom.y * in.planet_radius / dist;
    let cos_chi = clamp(-dot(pixel_dir, l0), -0.999999, 0.999999);
    let chi = acos(cos_chi);
    let ang = asin(clamp(in.planet_radius / dist, 0.0, 0.999999));
    let shine = dist_corr * dist_corr * ((1.0 - chi / PI) * cos_chi + sqrt(1.0 - cos_chi * cos_chi) / PI) * clamp(PI - ang - chi, 0.0, 1.0);
    let mu = clamp(dot(pixel_dir, normalize(-in.world_pos)), -0.999999, 0.999999);
    let lit = front.rgb * (ring.params.x * hapke(mu) * lambert(mu)) * (1.0 - transp.x);
    let dusty = back.rgb * (ring.params.y * mie(mu, -0.7)) * (1.0 - transp.y);
    return ring.host.rgb * shine * (lit + dusty);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let l = normalize(ring.sun_dir.xyz);
    let sun = ring.sun_color.rgb * ring.sun_dir.w;
    let v = normalize(-in.world_pos);
    let n = normalize(in.normal);
    let r = in.uv.x;

    var front = vec4<f32>(0.8, 0.75, 0.65, 0.6);
    var back = vec4<f32>(0.4, 0.37, 0.32, 0.6);
    if ring.params.z > 0.5 {
        front = textureSample(ring_tex, ring_smp, vec2<f32>(r, 0.75));
        back = textureSample(ring_tex, ring_smp, vec2<f32>(r, 0.25));
    }
    front = vec4<f32>(front.rgb * front.a, front.a);

    // Radial detail where its finest octave is resolved (SE fades it by distance; assumed here).
    let rad = r * ring.hapke2.y;
    let detail = 1.0 - smoothstep(0.3, 1.0, fwidth(rad * 4.9));
    if detail > 0.0 {
        let m = detail_noise(rad);
        let e_ring = smoothstep(0.0, 0.01, front.a);
        back = vec4<f32>(back.rgb * mix(1.0, m, 0.5 * detail * e_ring), back.a);
        front *= mix(1.0, m, 0.5 * detail);
    }

    // Optical densities (SE's ColorToOpticalDensity over a normal path of 2 half-thicknesses):
    // x front scattering, y back scattering, z opacity, w self-shadow.
    let d = clamp(1.0 - vec4<f32>(dot(front.rgb, TO_GRAY), dot(back.rgb, TO_GRAY), front.a, front.a) * ring.density, vec4<f32>(0.01), vec4<f32>(1.0));
    let od0 = -log(d) / (2.0 * FALLOFF_INTEGRAL);
    var transp = vec3<f32>(1.0);
    var transp_x = vec3<f32>(1.0);
    let lit = ring_scatter(in, l, sun * ring_shadow(in, 0, l), front, back, od0, &transp);
    var col = lit.rgb;
    for (var i = 0; i < 3; i++) {
        if ring.light_dir[i].w < 0.5 {
            break;
        }
        let lx = normalize(ring.light_dir[i].xyz);
        col += ring_scatter(in, lx, ring.light_color[i].rgb * ring_shadow(in, i + 1, lx), front, back, od0, &transp_x).rgb;
    }
    if ring.host.w > 0.5 {
        col += host_scatter(in, l, front, back, transp);
    }
    col *= ring.geom.w;
    let alpha = 1.0 - lit.w;
    let nv = dot(n, v);

    // Soft inner and outer edges, and none edge-on.
    let edge = clamp(min(r, 1.0 - r) / max(fwidth(r * 3.0), 1e-6), 0.0, 1.0) * smoothstep(0.0, 0.01, abs(nv));
    return vec4<f32>(col, alpha) * edge;
}
