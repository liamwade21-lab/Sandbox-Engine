// Brown dwarfs: SE's tg_browndwarf_glow.glsl evaluated per pixel. The surface temperature is
// `mix(surfTempB, GlowMapSun, erosion)` of the star's, surfTempB = mix(1, palette alpha, cloudsLayer)
// x (1 - 0.2 height) with cloudsLayer 0 on the body itself (the palette is for its cloud shells), the
// height SE's HeightMapCloudsGasGiant (zones, cyclones, turbulence, stripes; as planet.wesl's gas
// giants, with this body's parameters). Then the photosphere's limb darkening and thermal colour
// (star_surface.wesl), whose uniform, noise texture and vertex stage this material shares.

import super::star_surface::{star, glow_map_sun, VertexOut, GRAN_MEAN};
import super::gas_giant::{gg_noise, gg_fbm, gg_isf, gg_hash1, gg_rotate};

struct BrownDwarf {
    // As surface_tex::BiomeUniform::giant: (cloudsFreq, cloudsOctaves, stripeZones, stripeTwist),
    // (stripeFluct, 1, 0, 0), equatorial and polar cyclones, Randomize.
    clouds: array<vec4<f32>, 5>,
    // x erosion (the granulation's share), y the bands' mean height
    params: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> bd: BrownDwarf;

fn bd_turbulence(point_in: vec3<f32>) -> vec3<f32> {
    let twist = bd.clouds[0].w;
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

fn bd_cyclones(point_in: vec3<f32>, offset: ptr<function, f32>) -> vec3<f32> {
    let cy = bd.clouds[2];
    let cy2 = bd.clouds[3];
    var point = point_in;
    var twisted = point;
    var center = vec3<f32>(0.0);
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

// SE's HeightMapCloudsGasGiant with this body's parameters; the stripes' octaves finer than about
// a pixel (`fp`, radians) are left out.
fn bd_height(point: vec3<f32>, fp: f32) -> f32 {
    let g0 = bd.clouds[0];
    var twisted = point;
    var zp = bd.clouds[4].xyz * 7.1563;
    zp.y += twisted.y * g0.z * 0.5;
    let zones = gg_noise(zp) * 0.6 + 0.25;
    var offset = 0.0;
    if bd.clouds[2].w > 0.0 || bd.clouds[3].w > 0.0 {
        twisted = bd_cyclones(twisted, &offset);
    }
    twisted = bd_turbulence(twisted);
    let turb = gg_fbm(twisted * 0.2, g0.y);
    twisted = twisted * (0.00515 * g0.x) - turb * 0.0005;
    twisted.y *= 100.0 + turb;
    // The stripes vary 100 times faster across than along the bands.
    let fine = 0.00515 * g0.x * 100.0 * fp;
    let oct = clamp(floor(log2(0.5 / max(fine, 1e-30)) / log2(2.218281828459)) + 1.0, 1.0, g0.y);
    let height = bd.clouds[1].x * (gg_fbm(twisted, oct) * 0.7 + 0.5);
    return zones + height + offset;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let v = normalize(-in.world_position.xyz);
    let mu = clamp(dot(n, v), 0.0, 1.0);
    let point = normalize(in.local);
    let fp = max(length(fwidth(point)), 1e-9);

    let erosion = bd.params.x;
    var t = 1.0 - 0.2 * bd_height(point, fp);
    if erosion > 0.0 {
        t = mix(t, glow_map_sun(point, fp), erosion);
    }
    let ts = star.temp.x;
    var tk = t * ts * mix(0.86, 1.1, mix(1.0, mu, star.temp.y));
    tk = max(tk, 100.0);
    // A blackbody at tk relative to the disc centre's mean, as the photosphere's colour.
    let mean = mix(1.0 - 0.2 * bd.params.y, mix(star.gran2.y, star.gran2.z, GRAN_MEAN), erosion);
    let tc = ts * 1.1 * mean;
    let c = vec3<f32>(23587.0, 26160.0, 30942.0);
    let ratio = exp(c * (1.0 / tc - 1.0 / tk)) * (1.0 - exp(-c / tc)) / (1.0 - exp(-c / tk));
    let col = star.color.rgb * star.color.w * ratio;
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
