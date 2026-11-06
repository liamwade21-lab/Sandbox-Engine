//! Plants on bodies whose ground is drawn from a colour map (Earth): a square patch of the map around
//! the camera, read from its deepest tiles on a worker thread, its colour with the land (not the
//! water mask) in alpha. The cull pass classifies it at each tree into forest, grass and dry steppe
//! (`vegetation.wesl` `veg_classes`, the same as the ground does with the colour it draws, so woods
//! end where the ground's canopy does; [`classify`] is its reference, tested here), and the trees
//! take the colour where they stand. The patch is made again once the camera is a quarter of its
//! half-width from its centre. Earth's map is about 1.2 km a texel (its level 5): the finer edges of
//! woods come from a noise and the ground's slope and shore.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::planet_tiles::{CubeFace, TileKey, TileLayer};
use se_core::vfs::Vfs;

/// Texels a side of a patch.
pub const PATCH_SIZE: u32 = 512;
/// A patch's half-width on the ground (m).
const PATCH_HALF_M: f64 = 300_000.0;

/// A patch's frame around its centre `c` (unit, body frame): texel (x, y) lies towards
/// `c + (u e1 + v e2) tan_a`, u and v from -1 to 1 across it (v down). As the clouds' detail patches.
pub fn frame_axes(c: DVec3) -> (DVec3, DVec3) {
    let a = if c.y.abs() > 0.99 { DVec3::X } else { DVec3::Y };
    let e1 = a.cross(c).normalize();
    (e1, c.cross(e1))
}

/// Forest, grass and dry steppe shares (0..1) of a colour-map texel (sRGB), and whether it is water:
/// the reference for `vegetation.wesl` `veg_classes` (keep them the same).
#[cfg_attr(not(test), allow(dead_code))]
/// From Earth's map at the plan's test places: forests (Black Forest, Congo, Siberia's taiga) are
/// dark and strongly green, excess green (2g - r - b over the sum) 0.56-0.66; farmland (the Paris
/// basin, the corn belt) green but redder, 0.39-0.43; tundra 0.24; dry savanna and deserts about 0,
/// deserts also bright; ice white.
pub fn classify(rgb: [u8; 3], water: f32) -> [f32; 3] {
    let [r, g, b] = rgb.map(|c| c as f32);
    let sum = (r + g + b).max(1.0);
    let exg = (2.0 * g - r - b) / sum;
    let bright = r.max(g).max(b);
    let s = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let land = 1.0 - s(0.3, 0.7, water);
    let forest = s(0.42, 0.58, exg) * (1.0 - s(45.0, 75.0, bright));
    let grass = s(0.06, 0.3, exg) * (1.0 - forest) * (1.0 - s(150.0, 210.0, bright));
    let steppe = s(-0.08, 0.04, exg) * (1.0 - s(0.15, 0.35, exg)) * (1.0 - s(130.0, 185.0, bright));
    [forest * land, grass * land, steppe * land]
}

/// A patch being made or made.
pub struct EarthVeg {
    /// The patch's centre (body frame, unit) and the tangent of its half-angle.
    pub centre: DVec3,
    pub tan_a: f64,
    /// Bumped for each new patch.
    pub id: u32,
    /// The map's colour (sRGB) with the land in alpha.
    pub color: Option<Handle<Image>>,
    /// The lights map's brightness (the towns' density for the cities: scatter.wesl), when the body has one.
    pub lights: Option<Handle<Image>>,
    /// The same bytes for the CPU, with the patch's centre and tan (the Viltrumite mod's hero: is he in a town?).
    lights_cpu: Option<(DVec3, f64, Vec<u8>)>,
    task: Option<Task<(DVec3, f64, Vec<u8>, Option<Vec<u8>>)>>,
    /// A finished patch waiting a few frames for its images to reach the GPU (until then the old one
    /// stays bound; a missing one would leave no plants for a frame).
    pending: Option<(DVec3, f64, Handle<Image>, Option<Handle<Image>>, u32)>,
}

impl Default for EarthVeg {
    fn default() -> Self {
        EarthVeg { centre: DVec3::Y, tan_a: 0.0, id: 0, color: None, lights: None, lights_cpu: None, task: None, pending: None }
    }
}

impl EarthVeg {
    /// The lights map's brightness (0..1) at body-frame direction `dir`, if it lies inside the patch.
    pub fn lights_at(&self, dir: DVec3) -> Option<f32> {
        let (c, tan_a, data) = self.lights_cpu.as_ref()?;
        let k = dir.dot(*c);
        if k <= 0.0 {
            return None;
        }
        let (e1, e2) = frame_axes(*c);
        let p = dir / k;
        let (u, v) = (p.dot(e1) / tan_a, p.dot(e2) / tan_a);
        if u.abs() >= 1.0 || v.abs() >= 1.0 {
            return None;
        }
        let n = PATCH_SIZE as f64;
        let (x, y) = (((u + 1.0) * 0.5 * n) as usize, ((v + 1.0) * 0.5 * n) as usize);
        data.get(y * PATCH_SIZE as usize + x).map(|&b| b as f32 / 255.0)
    }

    /// Starts a new patch around `cam_dir` (body frame, unit) when the camera has left the middle of
    /// the current one; takes a finished one.
    pub fn update(&mut self, vfs: &Arc<Vfs>, color: &TileLayer, water: Option<&TileLayer>, lights: Option<&TileLayer>, radius: f64, cam_dir: DVec3, images: &mut Assets<Image>) {
        if let Some((c, tan_a, image, lit, frames)) = self.pending.take() {
            if frames > 0 && self.color.is_some() {
                self.pending = Some((c, tan_a, image, lit, frames - 1));
            } else {
                (self.centre, self.tan_a, self.color, self.lights) = (c, tan_a, Some(image), lit);
                self.id += 1;
            }
            return;
        }
        if let Some(task) = self.task.as_mut() {
            if let Some((c, tan_a, col, lit)) = check_ready(task) {
                self.task = None;
                self.lights_cpu = lit.as_ref().map(|l| (c, tan_a, l.clone()));
                let lit = lit.map(|l| images.add(patch_image_format(l, TextureFormat::R8Unorm)));
                self.pending = Some((c, tan_a, images.add(patch_image(col)), lit, 3));
            }
            return;
        }
        let half = (PATCH_HALF_M / radius).min(0.5);
        let away = self.centre.dot(cam_dir).clamp(-1.0, 1.0).acos();
        if self.color.is_some() && away < half * 0.25 {
            return;
        }
        let (vfs, color, water, lights) = (vfs.clone(), color.clone(), water.cloned(), lights.cloned());
        let tan_a = half.tan();
        self.task = Some(AsyncComputeTaskPool::get().spawn(async move {
            let col = build_patch(&vfs, &color, water.as_ref(), cam_dir, tan_a);
            let lit = lights.map(|l| build_lights(&vfs, &l, cam_dir, tan_a));
            (cam_dir, tan_a, col, lit)
        }));
    }
}

fn patch_image(data: Vec<u8>) -> Image {
    patch_image_format(data, TextureFormat::Rgba8UnormSrgb)
}

fn patch_image_format(data: Vec<u8>, format: TextureFormat) -> Image {
    let mut image = Image::new(
        Extent3d { width: PATCH_SIZE, height: PATCH_SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The decoded tiles of one patch build.
struct Tiles<'a> {
    vfs: &'a Vfs,
    layer: &'a TileLayer,
    level: u8,
    cache: HashMap<TileKey, Option<image::RgbaImage>>,
}

impl Tiles<'_> {
    /// The layer's colour at body-frame direction `d` (bilinear inside the tile's inner 252 texels).
    fn sample(&mut self, d: DVec3) -> Option<[f32; 4]> {
        let (face, u, v) = CubeFace::from_dir(d);
        let n = (1u32 << self.level) as f64;
        let (col, row) = (((u * n).floor() as u32).min(n as u32 - 1), ((v * n).floor() as u32).min(n as u32 - 1));
        let key = TileKey { face, level: self.level, row, col };
        let (vfs, layer) = (self.vfs, self.layer);
        let img = self
            .cache
            .entry(key)
            .or_insert_with(|| vfs.read(&layer.tile_path(key)).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8()))
            .as_ref()?;
        let (w, h) = img.dimensions();
        let k = w as f64 / 256.0;
        let (lu, lv) = (u * n - col as f64, v * n - row as f64);
        let (x, y) = ((2.0 + lu * 252.0) * k - 0.5, (2.0 + lv * 252.0) * k - 0.5);
        let (x0, y0) = (x.floor().clamp(0.0, w as f64 - 1.0), y.floor().clamp(0.0, h as f64 - 1.0));
        let (x1, y1) = ((x0 + 1.0).min(w as f64 - 1.0), (y0 + 1.0).min(h as f64 - 1.0));
        let (fx, fy) = ((x - x0).clamp(0.0, 1.0) as f32, (y - y0).clamp(0.0, 1.0) as f32);
        let p = |x: f64, y: f64| img.get_pixel(x as u32, y as u32).0.map(|c| c as f32);
        let (a, b, c, e) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bot = c[i] + (e[i] - c[i]) * fx;
            out[i] = top + (bot - top) * fy;
        }
        Some(out)
    }
}

/// The colour texels (sRGB, the land in alpha: 0 on water).
fn build_patch(vfs: &Vfs, color: &TileLayer, water: Option<&TileLayer>, c: DVec3, tan_a: f64) -> Vec<u8> {
    let started = std::time::Instant::now();
    let n = PATCH_SIZE as usize;
    let (e1, e2) = frame_axes(c);
    let mut col_tiles = Tiles { vfs, layer: color, level: color.max_level, cache: HashMap::new() };
    let mut water_tiles = water.map(|w| Tiles { vfs, layer: w, level: w.max_level, cache: HashMap::new() });
    let mut col = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let u = (x as f64 + 0.5) / n as f64 * 2.0 - 1.0;
            let v = (y as f64 + 0.5) / n as f64 * 2.0 - 1.0;
            let d = (c + (e1 * u + e2 * v) * tan_a).normalize();
            let Some(rgb) = col_tiles.sample(d) else { continue };
            let wet = water_tiles.as_mut().and_then(|t| t.sample(d)).map_or(0.0, |w| w[0] / 255.0);
            let rgb8 = [rgb[0], rgb[1], rgb[2]].map(|v| v.round().clamp(0.0, 255.0) as u8);
            let land = ((1.0 - wet) * 255.0).round().clamp(0.0, 255.0) as u8;
            let at = (y * n + x) * 4;
            col[at..at + 4].copy_from_slice(&[rgb8[0], rgb8[1], rgb8[2], land]);
        }
    }
    info!(
        "scatter: plants' map patch made ({} tiles, {:.0} ms)",
        col_tiles.cache.len(),
        started.elapsed().as_secs_f64() * 1e3
    );
    col
}

/// The lights map's brightness over the patch (its brightest channel, a byte a texel): the towns'
/// density for the cities (scatter.wesl's buildings, as the ground's from its lights tiles).
fn build_lights(vfs: &Vfs, lights: &TileLayer, c: DVec3, tan_a: f64) -> Vec<u8> {
    let n = PATCH_SIZE as usize;
    let (e1, e2) = frame_axes(c);
    let mut tiles = Tiles { vfs, layer: lights, level: lights.max_level, cache: HashMap::new() };
    let mut out = vec![0u8; n * n];
    for y in 0..n {
        for x in 0..n {
            let u = (x as f64 + 0.5) / n as f64 * 2.0 - 1.0;
            let v = (y as f64 + 0.5) / n as f64 * 2.0 - 1.0;
            let d = (c + (e1 * u + e2 * v) * tan_a).normalize();
            if let Some(l) = tiles.sample(d) {
                out[y * n + x] = l[0].max(l[1]).max(l[2]).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earth_colours() {
        // Earth's map at the plan's test places (scatter/earth.rs notes).
        let forest = |rgb| classify(rgb, 0.0)[0];
        let grass = |rgb| classify(rgb, 0.0)[1];
        assert!(forest([24, 39, 12]) > 0.8, "Black Forest");
        assert!(forest([25, 42, 9]) > 0.8, "taiga");
        assert!(forest([39, 45, 13]) < 0.2 && grass([39, 45, 13]) > 0.5, "Paris fields");
        assert!(forest([52, 57, 29]) < 0.05 && grass([52, 57, 29]) > 0.5, "tundra");
        assert!(classify([200, 168, 119], 0.0).iter().all(|&v| v < 0.05), "Sahara");
        assert!(classify([251, 253, 252], 0.0).iter().all(|&v| v < 0.01), "ice");
        assert!(classify([90, 67, 43], 0.0)[2] > 0.3, "dry savanna: steppe");
        assert!(classify([24, 39, 12], 1.0).iter().all(|&v| v < 0.01), "water");
    }
}
