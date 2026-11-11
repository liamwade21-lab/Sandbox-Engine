//! A small CPU software rasterizer that draws spawn-menu thumbnails of textured models.
//! Self-contained: it never touches the GPU, so it can run on a worker thread.
use bevy::math::{Vec2, Vec3};

/// One mesh of the model, borrowed.
pub struct IconMesh<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub uvs: &'a [[f32; 2]],
    /// Triangle list, counter-clockwise seen from outside.
    pub indices: &'a [u32],
    pub texture: Option<IconTexture<'a>>,
    /// Multiplies the texture (or is the colour when there is none); linear light.
    pub tint: [f32; 3],
    /// Texels with alpha below this are holes (0: none).
    pub alpha_cut: f32,
    /// Don't cull back faces.
    pub two_sided: bool,
}

/// An RGBA8 texture (sRGB colours, straight alpha), `width * height * 4` bytes, row 0 at v = 0.
pub struct IconTexture<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

const SUPER: usize = 3; // supersampling per axis, box-filtered down at the end
const FOV_DEG: f32 = 25.0;

/// The view: Garry's Mod style, 35 degrees round to the left (+y), 20 above level.
struct Camera {
    eye: Vec3,
    fwd: Vec3,
    right: Vec3,
    up: Vec3,
    focal: f32, // 1 / tan(fov / 2)
}

impl Camera {
    /// None when there is nothing finite with a size to look at.
    fn new(meshes: &[IconMesh]) -> Option<Camera> {
        let ok = |p: &[f32; 3]| p.iter().all(|c| c.is_finite() && c.abs() < 1e18);
        let pts: Vec<Vec3> = meshes.iter().flat_map(|m| m.positions.iter().filter(|p| ok(p)).map(|p| Vec3::from(*p))).collect();
        let first = *pts.first()?;
        let (lo, hi) = pts.iter().fold((first, first), |(l, h), p| (l.min(*p), h.max(*p)));
        let centre = (lo + hi) * 0.5;
        let radius = pts.iter().map(|p| p.distance(centre)).fold(0.0, f32::max);
        if !(radius > 1e-9) {
            return None;
        }
        let (yaw, pitch) = (35f32.to_radians(), 20f32.to_radians());
        let dir = Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin());
        let half = FOV_DEG.to_radians() * 0.5;
        // The sphere's angular radius is 90 % of the half field of view.
        let dist = radius / (half * 0.9).sin();
        let fwd = -dir;
        let right = fwd.cross(Vec3::Z).normalize();
        let up = right.cross(fwd);
        Some(Camera { eye: centre + dir * dist, fwd, right, up, focal: 1.0 / half.tan() })
    }

    /// Pixel position (y down) and view depth of a point in a `size` x `size` picture.
    fn project(&self, p: Vec3, size: f32) -> Option<[f32; 3]> {
        let rel = p - self.eye;
        let z = rel.dot(self.fwd);
        if !(z > 1e-6) {
            return None;
        }
        let x = (rel.dot(self.right) / z * self.focal * 0.5 + 0.5) * size;
        let y = (0.5 - rel.dot(self.up) / z * self.focal * 0.5) * size;
        (x.is_finite() && y.is_finite()).then_some([x, y, z])
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Bilinear, repeating; returns linear rgb and straight alpha.
fn sample(tex: &IconTexture, lut: &[f32; 256], uv: Vec2) -> [f32; 4] {
    let (w, h) = (tex.width as i64, tex.height as i64);
    // Wrap in floats first so huge uvs can't overflow the integer casts.
    let u = uv.x - uv.x.floor();
    let v = uv.y - uv.y.floor();
    if !(u.is_finite() && v.is_finite()) {
        return [1.0; 4];
    }
    let (fx, fy) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let wrap = |i: f32, n: i64| (i as i64).rem_euclid(n);
    let (xa, xb) = (wrap(x0, w), wrap(x0 + 1.0, w));
    let (ya, yb) = (wrap(y0, h), wrap(y0 + 1.0, h));
    let texel = |x: i64, y: i64| {
        let i = ((y * w + x) * 4) as usize;
        [lut[tex.rgba[i] as usize], lut[tex.rgba[i + 1] as usize], lut[tex.rgba[i + 2] as usize], tex.rgba[i + 3] as f32 / 255.0]
    };
    let (a, b, c, d) = (texel(xa, ya), texel(xb, ya), texel(xa, yb), texel(xb, yb));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        out[k] = top + (bot - top) * ty;
    }
    out
}

fn edge(a: [f32; 3], b: [f32; 3], px: f32, py: f32) -> f32 {
    (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])
}

/// A `size` x `size` RGBA8 picture (sRGB, straight alpha, transparent background), `size*size*4` bytes, row 0 at the top.
pub fn render_icon(meshes: &[IconMesh], size: u32) -> Vec<u8> {
    let size = size as usize;
    let mut out = vec![0u8; size * size * 4];
    let Some(cam) = Camera::new(meshes).filter(|_| size > 0 && size < 8192) else { return out };
    let s = size * SUPER;
    // Depth is 1/w (bigger is nearer); 0 means nothing drawn there.
    let mut depth = vec![0f32; s * s];
    let mut colour = vec![[0f32; 3]; s * s];
    let mut lut = [0f32; 256];
    for (i, l) in lut.iter_mut().enumerate() {
        *l = srgb_to_linear(i as f32 / 255.0);
    }
    // Key light from the upper left front of the camera, a faint fill from the other side.
    let key = (-cam.fwd * 0.5 + cam.up * 0.8 - cam.right * 0.5).normalize();
    let fill = (-cam.fwd * 0.5 + cam.right * 0.7).normalize();
    for m in meshes {
        let tex = m.texture.as_ref().filter(|t| t.width > 0 && t.height > 0 && t.rgba.len() >= t.width as usize * t.height as usize * 4);
        let use_uv = tex.is_some() && m.uvs.len() >= m.positions.len();
        let use_n = m.normals.len() >= m.positions.len();
        for t in m.indices.chunks_exact(3) {
            let ix = [t[0] as usize, t[1] as usize, t[2] as usize];
            if ix.iter().any(|&i| i >= m.positions.len()) {
                continue;
            }
            let pw = ix.map(|i| Vec3::from(m.positions[i]));
            let scr: Vec<Option<[f32; 3]>> = pw.iter().map(|p| cam.project(*p, s as f32)).collect();
            let (Some(a), Some(b), Some(c)) = (scr[0], scr[1], scr[2]) else { continue };
            let area = edge(a, b, c[0], c[1]);
            if !area.is_finite() || area.abs() < 1e-9 {
                continue;
            }
            // Counter-clockwise in the world is clockwise with y down: front faces have negative area.
            let back = area > 0.0;
            if back && !m.two_sided {
                continue;
            }
            let face_n = (pw[1] - pw[0]).cross(pw[2] - pw[0]);
            if !(face_n.length() > 0.0) || !face_n.is_finite() {
                continue;
            }
            let face_n = face_n.normalize();
            let vn: [Vec3; 3] = ix.map(|i| if use_n { Vec3::from(m.normals[i]) } else { Vec3::ZERO });
            let good_n = use_n && vn.iter().all(|n| n.is_finite() && n.length() > 1e-6);
            let flip = if back { -1.0 } else { 1.0 };
            let iw = [1.0 / a[2], 1.0 / b[2], 1.0 / c[2]];
            let uv: [Vec2; 3] = ix.map(|i| if use_uv { Vec2::from(m.uvs[i]) } else { Vec2::ZERO });
            let (x0, x1) = (a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize, (a[0].max(b[0]).max(c[0]).ceil().max(0.0) as usize).min(s - 1));
            let (y0, y1) = (a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize, (a[1].max(b[1]).max(c[1]).ceil().max(0.0) as usize).min(s - 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w = [edge(b, c, px, py) / area, edge(c, a, px, py) / area, edge(a, b, px, py) / area];
                    if w.iter().any(|&k| k < -1e-5) {
                        continue;
                    }
                    let inv = w[0] * iw[0] + w[1] * iw[1] + w[2] * iw[2];
                    let at = y * s + x;
                    if !(inv > depth[at]) {
                        continue;
                    }
                    // Perspective-correct weights.
                    let pc = [w[0] * iw[0] / inv, w[1] * iw[1] / inv, w[2] * iw[2] / inv];
                    let mut albedo = Vec3::from(m.tint);
                    if let Some(tx) = tex.filter(|_| use_uv) {
                        let st = sample(tx, &lut, uv[0] * pc[0] + uv[1] * pc[1] + uv[2] * pc[2]);
                        if st[3] < m.alpha_cut {
                            continue;
                        }
                        albedo *= Vec3::new(st[0], st[1], st[2]);
                    }
                    let n = if good_n { (vn[0].normalize() * pc[0] + vn[1].normalize() * pc[1] + vn[2].normalize() * pc[2]) * flip } else { face_n * flip };
                    let n = if n.length() > 1e-6 { n.normalize() } else { face_n * flip };
                    let lit = 0.35 + 0.75 * n.dot(key).max(0.0) + 0.15 * n.dot(fill).max(0.0);
                    depth[at] = inv;
                    colour[at] = [linear_to_srgb(albedo.x * lit), linear_to_srgb(albedo.y * lit), linear_to_srgb(albedo.z * lit)];
                }
            }
        }
    }
    // Box filter: the mean colour of the covered samples, alpha their share.
    let total = (SUPER * SUPER) as f32;
    for py in 0..size {
        for px in 0..size {
            let (mut sum, mut n) = ([0f32; 3], 0f32);
            for sy in 0..SUPER {
                for sx in 0..SUPER {
                    let at = (py * SUPER + sy) * s + px * SUPER + sx;
                    if depth[at] > 0.0 {
                        n += 1.0;
                        for k in 0..3 {
                            sum[k] += colour[at][k];
                        }
                    }
                }
            }
            if n > 0.0 {
                let o = (py * size + px) * 4;
                for k in 0..3 {
                    out[o + k] = (sum[k] / n * 255.0 + 0.5) as u8;
                }
                out[o + 3] = (n / total * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Geo { p: Vec<[f32; 3]>, n: Vec<[f32; 3]>, uv: Vec<[f32; 2]>, i: Vec<u32> }

    fn cube(flip: bool) -> Geo {
        let mut g = Geo { p: vec![], n: vec![], uv: vec![], i: vec![] };
        // (normal, u, v) with u x v = normal, so corners run counter-clockwise from outside.
        let faces = [([1., 0., 0.], [0., 1., 0.], [0., 0., 1.]), ([-1., 0., 0.], [0., 0., 1.], [0., 1., 0.]),
            ([0., 1., 0.], [0., 0., 1.], [1., 0., 0.]), ([0., -1., 0.], [1., 0., 0.], [0., 0., 1.]),
            ([0., 0., 1.], [1., 0., 0.], [0., 1., 0.]), ([0., 0., -1.], [0., 1., 0.], [1., 0., 0.])];
        for (n, u, v) in faces {
            let base = g.p.len() as u32;
            for (a, b) in [(-1.0f32, -1.0f32), (1., -1.), (1., 1.), (-1., 1.)] {
                g.p.push([0, 1, 2].map(|k| 0.5 * (n[k] + a * u[k] + b * v[k])));
                g.n.push(n);
                g.uv.push([(a + 1.) / 2., (b + 1.) / 2.]);
            }
            for t in [[0, 1, 2], [0, 2, 3]] {
                let t = if flip { [t[0], t[2], t[1]] } else { t };
                g.i.extend(t.map(|k| base + k));
            }
        }
        g
    }

    fn mesh<'a>(g: &'a Geo, tint: [f32; 3], two_sided: bool) -> IconMesh<'a> {
        IconMesh { positions: &g.p, normals: &g.n, uvs: &g.uv, indices: &g.i, texture: None, tint, alpha_cut: 0.0, two_sided }
    }

    fn px(img: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let o = (y * size + x) * 4;
        [img[o], img[o + 1], img[o + 2], img[o + 3]]
    }

    #[test]
    fn red_cube() {
        let g = cube(false);
        let img = render_icon(&[mesh(&g, [1.0, 0.0, 0.0], false)], 64);
        assert_eq!(img.len(), 64 * 64 * 4);
        let c = px(&img, 64, 32, 32);
        assert_eq!(c[3], 255);
        assert!(c[0] > 100 && c[1] < 30 && c[2] < 30, "{c:?}");
        for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63)] {
            assert_eq!(px(&img, 64, x, y)[3], 0);
        }
        let covered = img.chunks(4).filter(|p| p[3] > 127).count() as f32 / 4096.0;
        assert!((0.25..0.75).contains(&covered), "{covered}");
    }

    #[test]
    fn top_is_brightest() {
        let g = cube(false);
        let ms = [mesh(&g, [0.8; 3], false)];
        let size = 128usize;
        let img = render_icon(&ms, size as u32);
        let cam = Camera::new(&ms).unwrap();
        let bright = |n: Vec3| {
            let p = cam.project(n * 0.5, size as f32).unwrap();
            let (cx, cy) = (p[0] as usize, p[1] as usize);
            let mut sum = 0.0;
            for y in cy - 1..=cy + 1 {
                for x in cx - 1..=cx + 1 {
                    let q = px(&img, size, x, y);
                    assert_eq!(q[3], 255);
                    sum += q[0] as f32;
                }
            }
            sum / 9.0
        };
        let (top, front, left) = (bright(Vec3::Z), bright(Vec3::X), bright(Vec3::Y));
        assert!(top > front && top > left, "{top} {front} {left}");
    }

    #[test]
    fn winding() {
        // A closed cube flipped shows its far faces from inside, so test one quad (the cube's +z face).
        let g = cube(false);
        let one = |idx: &'static [u32], two| IconMesh { indices: idx, ..mesh(&g, [1.0; 3], two) };
        let clear = |img: &[u8]| img.chunks(4).all(|p| p[3] == 0);
        let front = [16u32, 17, 18, 16, 18, 19];
        let back = [16u32, 18, 17, 16, 19, 18];
        let front: &'static [u32] = Box::leak(Box::new(front));
        let back: &'static [u32] = Box::leak(Box::new(back));
        assert!(!clear(&render_icon(&[one(front, false)], 48)));
        assert!(clear(&render_icon(&[one(back, false)], 48)));
        assert!(!clear(&render_icon(&[one(back, true)], 48)));
    }

    #[test]
    fn checker_and_holes() {
        // A quad in the y-z plane, facing +x.
        let p = vec![[0., -1., -1.], [0., 1., -1.], [0., 1., 1.], [0., -1., 1.]];
        let n = vec![[1., 0., 0.]; 4];
        let uv = vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
        let i = vec![0, 1, 2, 0, 2, 3];
        let rgba = |a: u8| vec![255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255, 255, 0, 0, a];
        let (solid, holey) = (rgba(255), rgba(0));
        fn quad<'a>(p: &'a [[f32; 3]], n: &'a [[f32; 3]], uv: &'a [[f32; 2]], i: &'a [u32], data: &'a [u8], cut: f32) -> IconMesh<'a> {
            IconMesh { positions: p, normals: n, uvs: uv, indices: i, texture: Some(IconTexture { width: 2, height: 2, rgba: data }), tint: [1.0; 3], alpha_cut: cut, two_sided: true }
        }
        let a = render_icon(&[quad(&p, &n, &uv, &i, &solid, 0.5)], 96);
        assert!(a.chunks(4).any(|c| c[3] == 255 && c[0] as i32 > c[2] as i32 + 60));
        assert!(a.chunks(4).any(|c| c[3] == 255 && c[2] as i32 > c[0] as i32 + 60));
        let b = render_icon(&[quad(&p, &n, &uv, &i, &holey, 0.5)], 96);
        let hole = a.chunks(4).zip(b.chunks(4)).filter(|(s, h)| s[3] == 255 && h[3] == 0).count();
        assert!(hole > 20, "{hole}");
    }

    #[test]
    fn bad_input() {
        let clear = |img: Vec<u8>, size: usize| assert!(img.len() == size * size * 4 && img.iter().all(|&b| b == 0));
        clear(render_icon(&[], 16), 16);
        clear(render_icon(&[], 0), 0);
        let g = cube(false);
        let m = |p: &'static [[f32; 3]], i: &'static [u32]| IconMesh { positions: p, normals: &[], uvs: &[], indices: i, texture: None, tint: [1.0; 3], alpha_cut: 0.0, two_sided: true };
        clear(render_icon(&[m(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[0, 1, 9])], 16), 16);
        clear(render_icon(&[m(&[[f32::NAN; 3], [f32::NAN; 3], [f32::INFINITY; 3]], &[0, 1, 2])], 16), 16);
        clear(render_icon(&[m(&[[1., 1., 1.]; 3], &[0, 1, 2])], 16), 16);
        // Short normals and uvs are tolerated.
        let short = IconMesh { normals: &g.n[..3], uvs: &g.uv[..2], ..mesh(&g, [1.0; 3], false) };
        assert_eq!(render_icon(&[short], 16).len(), 16 * 16 * 4);
    }
}
