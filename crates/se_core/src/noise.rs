//! Deterministic 3D noise for procedural terrain (gradient noise, fBm, ridged
//! multifractal, crater fields). Pure functions of position and seed.

use glam::DVec3;

#[inline]
fn hash3(x: i64, y: i64, z: i64, seed: u64) -> u64 {
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    h ^= (x as u64).wrapping_mul(0x8CB9_2BA7_2F3D_8DD7);
    h = h.rotate_left(31).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = h.rotate_left(29).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^ (h >> 32)
}

/// Pseudo-random unit-ish gradient for a lattice point.
#[inline]
fn grad(x: i64, y: i64, z: i64, seed: u64) -> DVec3 {
    let h = hash3(x, y, z, seed);
    // 12 edge gradients of a cube, as in improved Perlin noise.
    match h % 12 {
        0 => DVec3::new(1.0, 1.0, 0.0),
        1 => DVec3::new(-1.0, 1.0, 0.0),
        2 => DVec3::new(1.0, -1.0, 0.0),
        3 => DVec3::new(-1.0, -1.0, 0.0),
        4 => DVec3::new(1.0, 0.0, 1.0),
        5 => DVec3::new(-1.0, 0.0, 1.0),
        6 => DVec3::new(1.0, 0.0, -1.0),
        7 => DVec3::new(-1.0, 0.0, -1.0),
        8 => DVec3::new(0.0, 1.0, 1.0),
        9 => DVec3::new(0.0, -1.0, 1.0),
        10 => DVec3::new(0.0, 1.0, -1.0),
        _ => DVec3::new(0.0, -1.0, -1.0),
    }
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Gradient noise in roughly [-1, 1].
pub fn gradient(p: DVec3, seed: u64) -> f64 {
    let i = p.floor();
    let f = p - i;
    let (ix, iy, iz) = (i.x as i64, i.y as i64, i.z as i64);
    let u = DVec3::new(fade(f.x), fade(f.y), fade(f.z));
    let mut corners = [0.0; 8];
    for (k, c) in corners.iter_mut().enumerate() {
        let (dx, dy, dz) = ((k & 1) as i64, ((k >> 1) & 1) as i64, ((k >> 2) & 1) as i64);
        let g = grad(ix + dx, iy + dy, iz + dz, seed);
        *c = g.dot(f - DVec3::new(dx as f64, dy as f64, dz as f64));
    }
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(corners[0], corners[1], u.x);
    let x10 = lerp(corners[2], corners[3], u.x);
    let x01 = lerp(corners[4], corners[5], u.x);
    let x11 = lerp(corners[6], corners[7], u.x);
    let y0 = lerp(x00, x10, u.y);
    let y1 = lerp(x01, x11, u.y);
    lerp(y0, y1, u.z) * 1.1
}

/// Fractal Brownian motion: sum of octaves, normalised to about [-1, 1].
pub fn fbm(p: DVec3, seed: u64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * gradient(p * freq, seed.wrapping_add(o as u64 * 0x632B_E59B));
        norm += amp;
        amp *= gain;
        freq *= lacunarity;
    }
    sum / norm.max(1e-12)
}

/// Ridged multifractal (sharp mountain ranges), in about [0, 1].
pub fn ridged(p: DVec3, seed: u64, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut freq, mut weight, mut norm) = (0.0, 1.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        let n = 1.0 - gradient(p * freq, seed.wrapping_add(o as u64 * 0x2545_F491)).abs();
        let n = n * n * weight;
        weight = (n * 2.0).clamp(0.0, 1.0);
        sum += n * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.1;
    }
    sum / norm.max(1e-12)
}

/// Crater field on the unit sphere: sum of crater profiles (bowl + raised rim)
/// over several size classes. Returns relative height in about [-1, 0.3].
pub fn craters(dir: DVec3, seed: u64, density: f64, levels: u32) -> f64 {
    craters_from(dir, seed, density, levels, 0.35)
}

/// [`craters`] with the cell size of the largest level (on the unit sphere) given.
pub fn craters_from(dir: DVec3, seed: u64, density: f64, levels: u32, cell0: f64) -> f64 {
    let mut h = 0.0;
    let mut cell = cell0;
    for level in 0..levels {
        let p = dir / cell;
        let i = p.floor();
        let (ix, iy, iz) = (i.x as i64, i.y as i64, i.z as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let (cx, cy, cz) = (ix + dx, iy + dy, iz + dz);
                    let hh = hash3(cx, cy, cz, seed ^ (level as u64 * 0xA24B_AED4));
                    let frac = |s: u32| ((hh >> s) & 0xFFFF) as f64 / 65535.0;
                    if frac(48) > density {
                        continue;
                    }
                    let centre = (DVec3::new(cx as f64 + frac(0), cy as f64 + frac(16), cz as f64 + frac(32)) * cell)
                        .normalize_or(DVec3::X);
                    // Only craters whose centre on the sphere stays in its cell: one pushed into
                    // another cell would reach past the cells searched and be cut off along a
                    // cell face, a straight cliff across the ground.
                    let home = (centre / cell).floor();
                    if (home.x as i64, home.y as i64, home.z as i64) != (cx, cy, cz) {
                        continue;
                    }
                    let radius = cell * (0.15 + 0.35 * frac(8).powi(2));
                    let d = (dir - centre).length() / radius;
                    if d < 1.6 {
                        // Bowl, rim and fading ejecta.
                        let bowl = d * d - 1.0;
                        let rim = 0.3 * (-(d - 1.0).powi(2) * 18.0).exp();
                        let profile = if d < 1.0 { bowl * 0.8 + rim } else { rim * (1.6 - d) / 0.6 };
                        h += profile * radius / cell * 0.5f64.powi(level as i32);
                    }
                }
            }
        }
        cell *= 0.5;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_range_and_determinism() {
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for i in 0..2000 {
            let p = DVec3::new(i as f64 * 0.137, i as f64 * 0.071 - 3.0, (i % 37) as f64 * 0.23);
            let n = gradient(p, 7);
            lo = lo.min(n);
            hi = hi.max(n);
            assert_eq!(n, gradient(p, 7));
        }
        assert!(lo > -1.3 && hi < 1.3 && hi - lo > 0.8, "{lo} {hi}");
        let f = fbm(DVec3::new(0.3, 0.2, 0.1), 1, 6, 2.0, 0.5);
        assert!(f.abs() <= 1.2);
        let r = ridged(DVec3::new(0.3, 0.2, 0.1), 1, 5);
        assert!((0.0..=1.0).contains(&r));
    }

    #[test]
    fn craters_are_continuous() {
        // A crater cut off at a cell face showed as a straight cliff across the ground: walk a
        // great circle in tiny steps and require no jumps.
        let (a, b) = (DVec3::new(0.3, 0.8, 0.1).normalize(), DVec3::new(-0.5, 0.2, 0.7).normalize());
        let axis = a.cross(b).normalize();
        let step = 2e-5;
        let mut prev = craters_from(a, 9, 0.6, 4, 0.05);
        let mut worst: f64 = 0.0;
        for i in 1..30_000 {
            let p = glam::DQuat::from_axis_angle(axis, i as f64 * step) * a;
            let h = craters_from(p, 9, 0.6, 4, 0.05);
            worst = worst.max((h - prev).abs());
            prev = h;
        }
        assert!(worst < 0.01, "height jump {worst}");
    }

    #[test]
    fn craters_are_bounded() {
        for i in 0..500 {
            let d = DVec3::new((i as f64).sin(), (i as f64 * 0.7).cos(), 0.3).normalize();
            let h = craters(d, 3, 0.5, 4);
            assert!(h > -3.0 && h < 1.0, "{h}");
        }
    }
}
