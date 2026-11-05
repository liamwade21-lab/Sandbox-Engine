// What the trees and the ground share about plants (no bindings): the forest a colour map shows
// (Earth's, scatter/earth.rs `classify`), the noise that sharpens its 1.2 km texels into woods with
// edges and clearings, and the crowns the ground draws past the trees' range (the canopy). The
// scatter's cull pass places trees by `map_forest`; the ground (planet.wesl) darkens the forest floor
// under them and draws the canopy beyond them from the same values, so the woods keep their shapes
// as the trees come and go.

fn veg_hash(p: vec3<i32>) -> f32 {
    var h = (u32(p.x) * 0x8da6b343u) ^ (u32(p.y) * 0xd8163841u) ^ (u32(p.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967295.0);
}

// Value noise (0..1) at `p`.
fn veg_vnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let x00 = mix(veg_hash(i), veg_hash(i + vec3<i32>(1, 0, 0)), w.x);
    let x10 = mix(veg_hash(i + vec3<i32>(0, 1, 0)), veg_hash(i + vec3<i32>(1, 1, 0)), w.x);
    let x01 = mix(veg_hash(i + vec3<i32>(0, 0, 1)), veg_hash(i + vec3<i32>(1, 0, 1)), w.x);
    let x11 = mix(veg_hash(i + vec3<i32>(0, 1, 1)), veg_hash(i + vec3<i32>(1, 1, 1)), w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}

// A linear colour as sRGB (0..255), for `veg_classes`.
fn lin_to_srgb255(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308)) * 255.0;
}

// Forest, grassland and dry steppe shares (0..1) of a colour-map colour (sRGB, 0..255), as
// scatter/earth.rs `classify` (keep them the same). The ground and the cull pass both classify the
// filtered colour (not filtered classes), so woods end in the same places for both.
fn veg_classes(rgb: vec3<f32>) -> vec3<f32> {
    let sum = max(rgb.r + rgb.g + rgb.b, 1.0);
    let exg = (2.0 * rgb.g - rgb.r - rgb.b) / sum;
    let bright = max(rgb.r, max(rgb.g, rgb.b));
    let forest = smoothstep(0.42, 0.58, exg) * (1.0 - smoothstep(45.0, 75.0, bright));
    let grass = smoothstep(0.06, 0.3, exg) * (1.0 - forest) * (1.0 - smoothstep(150.0, 210.0, bright));
    let steppe = smoothstep(-0.08, 0.04, exg) * (1.0 - smoothstep(0.15, 0.35, exg)) * (1.0 - smoothstep(130.0, 185.0, bright));
    return vec3<f32>(forest, grass, steppe);
}

// The noise (0..1, 100 m - 1.5 km) that shapes the woods at body-frame direction `dir` on a body of
// `radius` (m). The scatter works it out once per tree at placement.
fn map_noise(dir: vec3<f32>, radius: f32) -> f32 {
    let p = dir * (radius / 900.0);
    return veg_vnoise(p) * 0.5 + veg_vnoise(p * 2.7) * 0.3 + veg_vnoise(p * 7.3) * 0.2;
}

// The forest's share from the map's forest class `forest` and the woods' noise `n` there: the
// map's 1.2 km texels sharpened into woods with edges and clearings; none on steep ground (`slope`
// 1 - cos of its tilt).
fn map_forest(forest: f32, n: f32, slope: f32) -> f32 {
    return smoothstep(0.38, 0.62, forest + (n - 0.5) * 1.3) * (1.0 - smoothstep(0.18, 0.45, slope));
}

// The crowns seen from afar (the ground past the trees' range): a factor on the ground's colour at
// body-frame direction `dir`, crowns lit on the side towards the star (`sun`, body frame) with dark
// gaps between them, about 1 on average so the forest keeps its colour. `size` is the crowns'
// spacing (m).
fn canopy_shade(dir: vec3<f32>, radius: f32, sun: vec3<f32>, size: f32) -> f32 {
    let q = dir * (radius / size);
    let st = sun - dir * dot(sun, dir);
    let h = veg_vnoise(q);
    let hs = veg_vnoise(q + st * 0.35);
    // Groups of trees a few crowns across, lighter and darker.
    let group = veg_vnoise(q * 0.27 + vec3<f32>(17.3, 5.1, 9.7));
    let crown = smoothstep(0.32, 0.58, h);
    let lit = clamp(0.5 + (h - hs) * 3.0, 0.0, 1.0);
    return mix(0.2, 1.0, crown) * mix(0.55, 1.5, lit) * mix(0.75, 1.25, group) * 1.6;
}
