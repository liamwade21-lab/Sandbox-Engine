//! Tiling 3D noise volumes for the port's volumetric clouds (se_engine `volumetric_clouds.rs`), after
//! Schneider's "Real-time volumetric cloudscapes" (GPU Pro 7) and Hillaire's tileable volume noise:
//!
//! - the shape volume ([`SHAPE_SIZE`]^3 RGBA8): R Perlin-Worley (Perlin fBm lifted by Worley fBm),
//!   G, B and A Worley fBm at 4, 8 and 16 cells across the tile;
//! - the detail volume ([`DETAIL_SIZE`]^3 RGBA8): R, G and B Worley fBm at 2, 4 and 8 cells, A 1.
//!
//! Every noise wraps with the tile, so the volumes repeat seamlessly. Pure and deterministic; the
//! engine builds them once on worker threads.

/// Texels along each side of the shape volume.
pub const SHAPE_SIZE: usize = 128;
/// Texels along each side of the detail volume.
pub const DETAIL_SIZE: usize = 32;

fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / (1u32 << 24) as f32
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Ken Perlin's gradient noise (about -1..1) at `p` (in cells), repeating every `period` cells.
fn perlin(p: [f32; 3], period: i32, seed: u32) -> f32 {
    let i = p.map(|v| v.floor() as i32);
    let f = [p[0] - i[0] as f32, p[1] - i[1] as f32, p[2] - i[2] as f32];
    let grad = |dx: i32, dy: i32, dz: i32| {
        let h = hash((i[0] + dx).rem_euclid(period), (i[1] + dy).rem_euclid(period), (i[2] + dz).rem_euclid(period), seed) % 12;
        let (x, y, z) = (f[0] - dx as f32, f[1] - dy as f32, f[2] - dz as f32);
        match h {
            0 => x + y,
            1 => -x + y,
            2 => x - y,
            3 => -x - y,
            4 => x + z,
            5 => -x + z,
            6 => x - z,
            7 => -x - z,
            8 => y + z,
            9 => -y + z,
            10 => y - z,
            _ => -y - z,
        }
    };
    let u = f.map(fade);
    let x00 = lerp(grad(0, 0, 0), grad(1, 0, 0), u[0]);
    let x10 = lerp(grad(0, 1, 0), grad(1, 1, 0), u[0]);
    let x01 = lerp(grad(0, 0, 1), grad(1, 0, 1), u[0]);
    let x11 = lerp(grad(0, 1, 1), grad(1, 1, 1), u[0]);
    lerp(lerp(x00, x10, u[1]), lerp(x01, x11, u[1]), u[2])
}

/// Perlin fBm over the tile (`t` in 0..1), `cells` across at the first octave, mapped to 0..1.
fn perlin_fbm(t: [f32; 3], cells: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    for o in 0..octaves {
        let c = cells << o;
        sum += amp * perlin(t.map(|v| v * c as f32), c, seed.wrapping_add(o));
        norm += amp;
        amp *= 0.5;
    }
    (sum / norm * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// Inverted Worley noise over the tile (`t` in 0..1) with `cells` cells across: 1 at a feature point,
/// falling to 0 one cell away.
fn worley(t: [f32; 3], cells: i32, seed: u32) -> f32 {
    let p = t.map(|v| v * cells as f32);
    let i = p.map(|v| v.floor() as i32);
    let mut best = f32::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = [i[0] + dx, i[1] + dy, i[2] + dz];
                let h = hash(c[0].rem_euclid(cells), c[1].rem_euclid(cells), c[2].rem_euclid(cells), seed);
                let fp = [
                    c[0] as f32 + unit(h),
                    c[1] as f32 + unit(hash(h as i32, 1, 0, seed)),
                    c[2] as f32 + unit(hash(h as i32, 2, 0, seed)),
                ];
                let d2 = (fp[0] - p[0]).powi(2) + (fp[1] - p[1]).powi(2) + (fp[2] - p[2]).powi(2);
                best = best.min(d2);
            }
        }
    }
    1.0 - best.sqrt().min(1.0)
}

/// Three octaves of Worley noise (`cells`, twice and four times as many), weighted 0.625, 0.25, 0.125.
fn worley_fbm(t: [f32; 3], cells: i32, seed: u32) -> f32 {
    worley(t, cells, seed) * 0.625 + worley(t, cells * 2, seed ^ 0x9e37) * 0.25 + worley(t, cells * 4, seed ^ 0x7f4a) * 0.125
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// An `n`^3 RGBA8 volume with `texel` giving each texel's four values, filled on all cores.
fn volume(n: usize, texel: impl Fn([f32; 3]) -> [f32; 4] + Sync) -> Vec<u8> {
    let mut out = vec![0u8; n * n * n * 4];
    let threads = std::thread::available_parallelism().map_or(4, |c| c.get()).clamp(1, n);
    let slab = n.div_ceil(threads) * n * n * 4;
    std::thread::scope(|s| {
        for (k, chunk) in out.chunks_mut(slab).enumerate() {
            let texel = &texel;
            s.spawn(move || {
                for (j, px) in chunk.chunks_exact_mut(4).enumerate() {
                    let idx = k * slab / 4 + j;
                    let (x, y, z) = (idx % n, idx / n % n, idx / (n * n));
                    let t = [x, y, z].map(|v| (v as f32 + 0.5) / n as f32);
                    px.copy_from_slice(&texel(t).map(byte));
                }
            });
        }
    });
    out
}

/// The shape volume, level 0 only.
pub fn shape_volume() -> Vec<u8> {
    volume(SHAPE_SIZE, |t| {
        let w = worley_fbm(t, 4, 11);
        let pw = w + perlin_fbm(t, 4, 4, 7) * (1.0 - w);
        [pw, w, worley_fbm(t, 8, 23), worley_fbm(t, 16, 37)]
    })
}

/// The detail volume, level 0 only.
pub fn detail_volume() -> Vec<u8> {
    volume(DETAIL_SIZE, |t| [worley_fbm(t, 2, 41), worley_fbm(t, 4, 53), worley_fbm(t, 8, 67), 1.0])
}

/// `level0` (an `n`^3 RGBA8 volume, `n` a power of two) followed by its mip levels (2x2x2 averages)
/// down to 1^3, and the number of levels.
pub fn with_mips(level0: Vec<u8>, n: usize) -> (Vec<u8>, u32) {
    let mut all = level0;
    let (mut start, mut m, mut levels) = (0, n, 1);
    while m > 1 {
        let h = m / 2;
        let mut next = Vec::with_capacity(h * h * h * 4);
        for z in 0..h {
            for y in 0..h {
                for x in 0..h {
                    for c in 0..4 {
                        let mut sum = 0u32;
                        for (dx, dy, dz) in [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 0, 1), (1, 0, 1), (0, 1, 1), (1, 1, 1)] {
                            sum += all[start + (((2 * z + dz) * m + 2 * y + dy) * m + 2 * x + dx) * 4 + c] as u32;
                        }
                        next.push(((sum + 4) / 8) as u8);
                    }
                }
            }
        }
        start = all.len();
        all.extend_from_slice(&next);
        m = h;
        levels += 1;
    }
    (all, levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noises_tile_and_stay_in_range() {
        for &p in &[[0.13f32, 0.71, 0.42], [0.99, 0.01, 0.5], [0.5, 0.5, 0.0]] {
            for k in 0..3 {
                let mut q = p;
                q[k] += 1.0;
                // One tile over: the same value.
                assert!((worley(p, 4, 3) - worley(q, 4, 3)).abs() < 1e-5);
                let (pc, qc) = (p.map(|v| v * 8.0), q.map(|v| v * 8.0));
                assert!((perlin(pc, 8, 5) - perlin(qc, 8, 5)).abs() < 1e-4);
            }
            let w = worley_fbm(p, 4, 9);
            let f = perlin_fbm(p, 4, 4, 9);
            assert!((0.0..=1.0).contains(&w) && (0.0..=1.0).contains(&f));
        }
    }

    /// The quantiles of the clouds' shape value (`volumetric_clouds.wesl` `layer_density`'s `base`, from
    /// the shape volume's channels) at mip levels 0 to 5, for the shader's table that spreads it evenly:
    /// `cargo test -p se_core print_shape_quantiles -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_shape_quantiles() {
        let (all, levels) = with_mips(shape_volume(), SHAPE_SIZE);
        let mut start = 0;
        for level in 0..levels.min(6) {
            let n = SHAPE_SIZE >> level;
            let mut base: Vec<f32> = all[start..start + n * n * n * 4]
                .chunks_exact(4)
                .map(|t| {
                    let [r, g, b, a] = [t[0], t[1], t[2], t[3]].map(|v| v as f32 / 255.0);
                    let fbm = g * 0.625 + b * 0.25 + a * 0.125;
                    ((r - (fbm - 1.0)) / (2.0 - fbm)).clamp(0.0, 1.0)
                })
                .collect();
            base.sort_by(f32::total_cmp);
            let q: Vec<String> = (0..=16).map(|k| format!("{:.3}", base[(k * (base.len() - 1)) / 16])).collect();
            println!("mip {level}: {}", q.join(", "));
            start += n * n * n * 4;
        }
    }

    #[test]
    fn detail_volume_is_deterministic_varied_and_mipped() {
        let a = detail_volume();
        assert_eq!(a.len(), DETAIL_SIZE.pow(3) * 4);
        assert_eq!(a, detail_volume());
        let r: Vec<u8> = a.chunks_exact(4).map(|t| t[0]).collect();
        let (lo, hi) = (r.iter().min().unwrap(), r.iter().max().unwrap());
        assert!(hi - lo > 100, "too flat: {lo}..{hi}");
        assert!(a.chunks_exact(4).all(|t| t[3] == 255));
        let (all, levels) = with_mips(a, DETAIL_SIZE);
        assert_eq!(levels, 6);
        assert_eq!(all.len(), (0..6).map(|l| (DETAIL_SIZE >> l).pow(3) * 4).sum::<usize>());
        // The last level is the mean of the whole volume.
        let mean = r.iter().map(|&v| v as f32).sum::<f32>() / r.len() as f32;
        assert!((all[all.len() - 4] as f32 - mean).abs() < 2.0);
    }
}
