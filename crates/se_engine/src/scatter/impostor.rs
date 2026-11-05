//! Impostors: each model drawn from `AZIMUTHS` directions at `ELEVATIONS` heights into an atlas,
//! on the CPU at load (a worker thread), as Megaton Rainfall bakes its trees' (its exe; the method
//! is ours). Colour (linear albedo x the material's tint, stored sRGB) and object-space normals,
//! both with the cut-out's coverage in alpha. Rendered at twice the size and averaged, the colour
//! spread into the clear texels (no dark fringes in the mips), and the mips' alpha rescaled so a
//! far tree keeps the cover its leaves have up close.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Vec3;
use bevy::prelude::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};

/// Directions around the model and heights above its horizon (views per model = their product).
pub const AZIMUTHS: u32 = 8;
pub const ELEVATIONS: u32 = 3;
pub const VIEWS: u32 = AZIMUTHS * ELEVATIONS;
/// The views' elevation angles (degrees): `scatter_draw.wesl` picks between them.
pub const ELEVATION_DEG: [f32; ELEVATIONS as usize] = [0.0, 35.0, 70.0];
/// Texels a side of a view.
pub const SIZE: u32 = 128;
/// Rendered at this many times the size and averaged.
const SUPER: u32 = 2;

/// A decoded texture (RGBA8, sRGB colour).
#[derive(Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

pub struct BakeSub {
    pub first_index: u32,
    pub index_count: u32,
    pub texture: Texture,
    pub tint: [f32; 4],
    pub leaves: bool,
}

pub struct BakeModel {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub subs: Vec<BakeSub>,
    /// The quad's half size and centre height (m), as `ModelGpu`.
    pub half: f32,
    pub centre_y: f32,
    pub crown_y: f32,
    pub crown_r: f32,
}

pub struct Baked {
    pub layers: u32,
    /// Per layer, its mip chain, RGBA8.
    pub color: Vec<Vec<Vec<u8>>>,
    pub normal: Vec<Vec<Vec<u8>>>,
    /// Per model: the mean colour of its views (linear, covered texels) and their mean coverage.
    pub means: Vec<[f32; 4]>,
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// The direction from the model towards the camera of view `v` (azimuth-major within an elevation:
/// `v = elevation * AZIMUTHS + azimuth`). Azimuth 0 looks from +z; azimuths turn towards +x.
pub fn view_dir(v: u32) -> Vec3 {
    let az = (v % AZIMUTHS) as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
    let el = ELEVATION_DEG[(v / AZIMUTHS) as usize].to_radians();
    Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos())
}

/// Bakes every model's views, the models spread over the machine's cores.
pub fn bake(models: &[BakeModel]) -> Baked {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let mut per_model: Vec<Vec<(Vec<Vec<u8>>, Vec<Vec<u8>>)>> = Vec::with_capacity(models.len());
    std::thread::scope(|scope| {
        let chunk = models.len().div_ceil(threads).max(1);
        let handles: Vec<_> = models
            .chunks(chunk)
            .map(|ms| scope.spawn(move || ms.iter().map(|m| (0..VIEWS).map(|v| bake_view(m, view_dir(v))).collect::<Vec<_>>()).collect::<Vec<_>>()))
            .collect();
        for h in handles {
            per_model.extend(h.join().expect("impostor bake thread"));
        }
    });
    let (mut color, mut normal, mut means) = (Vec::new(), Vec::new(), Vec::new());
    for views in per_model {
        let (mut sum, mut cover, mut texels) = ([0.0f64; 3], 0.0f64, 0.0f64);
        for (c, n) in views {
            for p in c[0].chunks_exact(4) {
                let a = p[3] as f64 / 255.0;
                for i in 0..3 {
                    sum[i] += srgb_to_linear(p[i]) as f64 * a;
                }
                cover += a;
                texels += 1.0;
            }
            color.push(c);
            normal.push(n);
        }
        let k = 1.0 / cover.max(1e-9);
        means.push([(sum[0] * k) as f32, (sum[1] * k) as f32, (sum[2] * k) as f32, (cover / texels.max(1.0)) as f32]);
    }
    Baked { layers: color.len() as u32, color, normal, means }
}

/// Bilinear sample of a texture at (u, v) (repeating), linear rgb and alpha.
fn sample(t: &Texture, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (t.width as i64, t.height as i64);
    let x = u * w as f32 - 0.5;
    let y = v * h as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: i64, yi: i64| {
        let (xi, yi) = (xi.rem_euclid(w), yi.rem_euclid(h));
        let o = ((yi * w + xi) * 4) as usize;
        let p = &t.rgba[o..o + 4];
        [srgb_to_linear(p[0]), srgb_to_linear(p[1]), srgb_to_linear(p[2]), p[3] as f32 / 255.0]
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let mut out = [0.0; 4];
    for k in 0..4 {
        out[k] = (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy;
    }
    out
}

/// One view: (colour mips, normal mips).
fn bake_view(m: &BakeModel, dir: Vec3) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let n = (SIZE * SUPER) as usize;
    let right = if dir.y.abs() > 0.999 { Vec3::X } else { Vec3::Y.cross(dir).normalize() };
    let up = dir.cross(right);
    let centre = Vec3::new(0.0, m.centre_y, 0.0);
    let crown = Vec3::new(0.0, m.crown_y, 0.0);
    let mut depth = vec![f32::NEG_INFINITY; n * n];
    let mut col = vec![[0.0f32; 3]; n * n];
    let mut nor = vec![Vec3::ZERO; n * n];
    let project = |p: [f32; 3]| {
        let q = Vec3::from_array(p) - centre;
        let x = q.dot(right) / m.half;
        let y = q.dot(up) / m.half;
        ((x * 0.5 + 0.5) * n as f32, (0.5 - y * 0.5) * n as f32, q.dot(dir))
    };
    for s in &m.subs {
        let idx = &m.indices[s.first_index as usize..(s.first_index + s.index_count) as usize];
        for tri in idx.chunks_exact(3) {
            let ids = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            let p = ids.map(|i| project(m.positions[i]));
            let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
            if area.abs() < 1e-9 {
                continue;
            }
            let x0 = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
            let x1 = (p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil() as usize).min(n);
            let y0 = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
            let y1 = (p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil() as usize).min(n);
            for py in y0..y1 {
                for px in x0..x1 {
                    let (sx, sy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let e = |a: (f32, f32, f32), b: (f32, f32, f32)| (b.0 - a.0) * (sy - a.1) - (sx - a.0) * (b.1 - a.1);
                    let w0 = e(p[1], p[2]) / area;
                    let w1 = e(p[2], p[0]) / area;
                    let w2 = e(p[0], p[1]) / area;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * p[0].2 + w1 * p[1].2 + w2 * p[2].2;
                    let k = py * n + px;
                    if z <= depth[k] {
                        continue;
                    }
                    let uv = |i: usize| m.uvs.get(ids[i]).copied().unwrap_or([0.0; 2]);
                    let (u, v) = (
                        w0 * uv(0)[0] + w1 * uv(1)[0] + w2 * uv(2)[0],
                        w0 * uv(0)[1] + w1 * uv(1)[1] + w2 * uv(2)[1],
                    );
                    let t = sample(&s.texture, u, v);
                    if s.leaves && t[3] < 0.5 {
                        continue;
                    }
                    depth[k] = z;
                    col[k] = [t[0] * s.tint[0], t[1] * s.tint[1], t[2] * s.tint[2]];
                    let vn = |i: usize| Vec3::from_array(m.normals.get(ids[i]).copied().unwrap_or([0.0, 1.0, 0.0]));
                    let mut nn = (vn(0) * w0 + vn(1) * w1 + vn(2) * w2).normalize_or(Vec3::Y);
                    if s.leaves {
                        // The crown shades as a ball (as `scatter_draw.wesl` leaves do).
                        let pos = Vec3::from_array(m.positions[ids[0]]) * w0
                            + Vec3::from_array(m.positions[ids[1]]) * w1
                            + Vec3::from_array(m.positions[ids[2]]) * w2;
                        let ball = ((pos - crown) / m.crown_r.max(0.1)).normalize_or(Vec3::Y);
                        if nn.dot(dir) < 0.0 {
                            nn = -nn;
                        }
                        nn = (nn * 0.3 + ball * 0.7).normalize_or(Vec3::Y);
                    }
                    nor[k] = nn;
                }
            }
        }
    }
    // Average the super-samples: coverage in alpha, the covered texels' mean colour.
    let s = SIZE as usize;
    let mut c0 = vec![[0.0f32; 4]; s * s];
    let mut n0 = vec![[0.0f32; 4]; s * s];
    for y in 0..s {
        for x in 0..s {
            let mut acc = [0.0f32; 3];
            let mut nacc = Vec3::ZERO;
            let mut cov = 0.0;
            for dy in 0..SUPER as usize {
                for dx in 0..SUPER as usize {
                    let k = (y * SUPER as usize + dy) * n + x * SUPER as usize + dx;
                    if depth[k] > f32::NEG_INFINITY {
                        for c in 0..3 {
                            acc[c] += col[k][c];
                        }
                        nacc += nor[k];
                        cov += 1.0;
                    }
                }
            }
            let a = cov / (SUPER * SUPER) as f32;
            if cov > 0.0 {
                let nn = nacc.normalize_or(Vec3::Y);
                c0[y * s + x] = [acc[0] / cov, acc[1] / cov, acc[2] / cov, a];
                n0[y * s + x] = [nn.x, nn.y, nn.z, a];
            }
        }
    }
    dilate(&mut c0, s);
    dilate(&mut n0, s);
    let cmips = mips(c0, s);
    let nmips = mips(n0, s);
    let to_color = |l: &Vec<[f32; 4]>| l.iter().flat_map(|p| [linear_to_srgb(p[0]), linear_to_srgb(p[1]), linear_to_srgb(p[2]), (p[3] * 255.0).round() as u8]).collect::<Vec<u8>>();
    let to_normal = |l: &Vec<[f32; 4]>| {
        l.iter()
            .flat_map(|p| {
                let v = Vec3::new(p[0], p[1], p[2]).normalize_or(Vec3::Y);
                [((v.x * 0.5 + 0.5) * 255.0).round() as u8, ((v.y * 0.5 + 0.5) * 255.0).round() as u8, ((v.z * 0.5 + 0.5) * 255.0).round() as u8, (p[3] * 255.0).round() as u8]
            })
            .collect::<Vec<u8>>()
    };
    (cmips.iter().map(to_color).collect(), nmips.iter().map(to_normal).collect())
}

/// Spreads the covered texels' rgb into the clear ones (their alpha stays 0), so filtering and
/// mips do not blend in black at the cut-out's edges.
fn dilate(img: &mut [[f32; 4]], s: usize) {
    let mut have: Vec<bool> = img.iter().map(|p| p[3] > 0.0).collect();
    for _ in 0..s {
        let mut changed = false;
        let prev = img.to_vec();
        let prev_have = have.clone();
        for y in 0..s {
            for x in 0..s {
                let k = y * s + x;
                if prev_have[k] {
                    continue;
                }
                let mut acc = [0.0f32; 3];
                let mut c = 0.0;
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= s as i64 || ny >= s as i64 {
                        continue;
                    }
                    let j = ny as usize * s + nx as usize;
                    if prev_have[j] {
                        for q in 0..3 {
                            acc[q] += prev[j][q];
                        }
                        c += 1.0;
                    }
                }
                if c > 0.0 {
                    img[k] = [acc[0] / c, acc[1] / c, acc[2] / c, 0.0];
                    have[k] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// The share of texels an alpha test at 0.5 keeps.
fn coverage(l: &[[f32; 4]], scale: f32) -> f32 {
    l.iter().filter(|p| p[3] * scale >= 0.5).count() as f32 / l.len().max(1) as f32
}

/// The mip chain: alpha-weighted colour, and alpha rescaled to keep the top level's coverage.
fn mips(top: Vec<[f32; 4]>, s: usize) -> Vec<Vec<[f32; 4]>> {
    let target = coverage(&top, 1.0);
    let mut out = vec![top];
    let mut size = s;
    while size > 1 {
        let prev = out.last().unwrap();
        let h = size / 2;
        let mut l = vec![[0.0f32; 4]; h * h];
        for y in 0..h {
            for x in 0..h {
                let mut acc = [0.0f32; 4];
                let mut wsum = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = prev[(2 * y + dy) * size + 2 * x + dx];
                    let w = p[3].max(1e-4);
                    for q in 0..3 {
                        acc[q] += p[q] * w;
                    }
                    wsum += w;
                    acc[3] += p[3] * 0.25;
                }
                l[y * h + x] = [acc[0] / wsum, acc[1] / wsum, acc[2] / wsum, acc[3]];
            }
        }
        // Coverage-preserving alpha (the scale that keeps the share an alpha test keeps).
        if target > 0.0 {
            let (mut lo, mut hi) = (0.25f32, 8.0f32);
            for _ in 0..16 {
                let mid = (lo + hi) * 0.5;
                if coverage(&l, mid) < target { lo = mid } else { hi = mid }
            }
            for p in &mut l {
                p[3] = (p[3] * hi).min(1.0);
            }
        }
        out.push(l);
        size = h;
    }
    out
}

/// The baked atlas as two Bevy array images (colour sRGB, normals linear).
pub fn images(b: &Baked) -> (Image, Image) {
    let make = |layers: &Vec<Vec<Vec<u8>>>, format: TextureFormat| {
        let mips = layers.first().map_or(1, |l| l.len()) as u32;
        let data: Vec<u8> = layers.iter().flat_map(|l| l.iter().flatten().copied()).collect();
        let mut image = Image::new_uninit(
            Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: b.layers },
            TextureDimension::D2,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.data = Some(data);
        image.texture_descriptor.mip_level_count = mips;
        image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..Default::default() });
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..Default::default()
        });
        image
    };
    (make(&b.color, TextureFormat::Rgba8UnormSrgb), make(&b.normal, TextureFormat::Rgba8Unorm))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square card facing +z, textured opaque green: seen from the front it covers its share of
    /// the view, and the mips keep that coverage.
    #[test]
    fn card_coverage() {
        let tex = Texture { width: 1, height: 1, rgba: Arc::new(vec![0, 255, 0, 255]) };
        let m = BakeModel {
            positions: vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 2.0, 0.0], [1.0, 2.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0; 2]; 4],
            indices: vec![0, 1, 2, 2, 1, 3],
            subs: vec![BakeSub { first_index: 0, index_count: 6, texture: tex, tint: [1.0; 4], leaves: false }],
            half: 2.0,
            centre_y: 1.0,
            crown_y: 1.0,
            crown_r: 1.0,
        };
        let (c, n) = bake_view(&m, view_dir(0));
        assert_eq!(c.len(), 8);
        let top = &c[0];
        let covered = top.chunks_exact(4).filter(|p| p[3] >= 128).count() as f32 / (SIZE * SIZE) as f32;
        // The card is half the quad wide and half its height: a quarter of the view.
        assert!((covered - 0.25).abs() < 0.02, "{covered}");
        // Green, and the normal faces the camera (+z: blue channel high).
        let k = ((SIZE / 2) * SIZE + SIZE / 2) as usize * 4;
        assert!(top[k + 1] > 200 && top[k] < 30);
        assert!(n[0][k + 2] > 240);
        let mid = &c[3];
        let cov3 = mid.chunks_exact(4).filter(|p| p[3] >= 128).count() as f32 / mid.len() as f32 * 4.0;
        assert!((cov3 - 0.25).abs() < 0.06, "{cov3}");
    }
}
