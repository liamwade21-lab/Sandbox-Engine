//! Procedural galaxies of the universe (`seGalNodeCache::CreateUniverseBlock`, decompiled).
//!
//! The universe is a cube of half-size [`ROOT_HALF_PC`] (5 Gpc) centred on the origin,
//! subdivided into an octree of [`LEVELS`] levels. A node generates up to 16384 candidate
//! galaxies from a node-seeded LCG; candidates are accepted by a cosmic-web density (domain-warped
//! Voronoi `F2 - F1`, so galaxies pile up on cell walls and edges) and by an apparent-magnitude cut
//! (procedural galaxies are only those fainter than the catalog limit, `GalaxyMaxAppMagn`).
//! Every level holds one tenth of the probability mass of a truncated Gaussian of absolute
//! magnitudes (mean -22, sigma 5, +-2 sigma), so coarse nodes hold the brightest galaxies.
//!
//! Naming: `RG 0-<level>-<node>-<index>`; `node` is the base-8 path from the root (child =
//! `parent * 8 + octant`), `index` counts accepted galaxies in the node. See
//! `docs/formats/procedural_deepsky.md` for what was decoded and what is approximated.

use std::sync::OnceLock;

use glam::DVec3;

/// Half-size of the root node, parsecs (decoded: node centres of the saved places `RG 0-9-...`).
pub const ROOT_HALF_PC: f64 = 5.0e9;
/// Number of octree levels (0..=9).
pub const LEVELS: u8 = 10;
/// Candidates per node (`min(100000, 0x4000)`).
pub const CANDIDATES: usize = 16384;
/// `GalaxyMaxAppMagn`: galaxies brighter than this from the Sun are left to the catalogs.
pub const MAX_APP_MAG: f32 = 12.1;

const INV32: f32 = 1.0 / 4_294_967_296.0;
const INV32_D: f64 = 1.0 / 4_294_967_296.0;
const LCG_A: u32 = 1_664_525;
const LCG_C: u32 = 1_013_904_223;

/// Octant -> child offset signs (table at `0x1407f4be0`): bit 2 = -x, bit 1 = -y, bit 0 = -z.
pub const OCTANT_DIRS: [[f64; 3]; 8] = [
    [1.0, 1.0, 1.0],
    [1.0, 1.0, -1.0],
    [1.0, -1.0, 1.0],
    [1.0, -1.0, -1.0],
    [-1.0, 1.0, 1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [-1.0, -1.0, -1.0],
];

/// Number of sprite models per Hubble type index 9..=16 in the shipped `.pst` files
/// (`S0 Sa Sb Sc SBa SBb SBc Irr`), used for the model draw when the caller has no catalog.
pub const DEFAULT_MODEL_POOLS: [u32; 17] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 13, 26, 11, 4, 25, 42, 11];

#[inline]
fn lcg(s: &mut u32) -> u32 {
    *s = s.wrapping_mul(LCG_A).wrapping_add(LCG_C);
    *s
}

// ---------------------------------------------------------------------------------------------
// Octree addressing
// ---------------------------------------------------------------------------------------------

/// Address of a node of the galaxy octree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GalNodeKey {
    pub level: u8,
    /// Base-8 path from the root (`0` for the root itself).
    pub id: u64,
}

impl GalNodeKey {
    pub const ROOT: GalNodeKey = GalNodeKey { level: 0, id: 0 };

    pub fn child(self, octant: u8) -> Self {
        GalNodeKey { level: self.level + 1, id: self.id * 8 + octant as u64 }
    }

    pub fn parent(self) -> Option<Self> {
        (self.level > 0).then(|| GalNodeKey { level: self.level - 1, id: self.id / 8 })
    }

    /// Half-size of the node cube, parsecs.
    pub fn half(self) -> f64 {
        ROOT_HALF_PC / (1u64 << self.level) as f64
    }

    /// Octant digits from the root down (length = level).
    pub fn digits(self) -> Vec<u8> {
        let mut d = Vec::with_capacity(self.level as usize);
        let mut n = self.id;
        for _ in 0..self.level {
            d.push((n % 8) as u8);
            n /= 8;
        }
        d.reverse();
        d
    }

    /// Node centre, parsecs (universe frame).
    pub fn center(self) -> DVec3 {
        let mut c = DVec3::ZERO;
        let mut h = ROOT_HALF_PC;
        for o in self.digits() {
            h *= 0.5;
            let d = OCTANT_DIRS[o as usize];
            c += DVec3::new(d[0], d[1], d[2]) * h;
        }
        c
    }

    /// The node of `level` containing `p` (points outside the cube are clamped to the border).
    pub fn containing(level: u8, p: DVec3) -> Self {
        let mut key = GalNodeKey::ROOT;
        let mut c = DVec3::ZERO;
        let mut h = ROOT_HALF_PC;
        for _ in 0..level {
            let o = (p.x < c.x) as u8 * 4 + (p.y < c.y) as u8 * 2 + (p.z < c.z) as u8;
            h *= 0.5;
            let d = OCTANT_DIRS[o as usize];
            c += DVec3::new(d[0], d[1], d[2]) * h;
            key = key.child(o);
        }
        key
    }

    /// The node of `level` at grid cell `cell` (0 .. 2^level per axis, +x/+y/+z last).
    pub fn from_cell(level: u8, cell: [u32; 3]) -> Self {
        let mut key = GalNodeKey::ROOT;
        for l in (0..level).rev() {
            let bit = |c: u32| (c >> l) & 1;
            // A set octant bit means the negative half, i.e. a zero grid bit.
            let o = (bit(cell[0]) == 0) as u8 * 4 + (bit(cell[1]) == 0) as u8 * 2 + (bit(cell[2]) == 0) as u8;
            key = key.child(o);
        }
        key
    }

    /// Nodes of `level` whose cube lies within `reach` parsecs of `p` (a box test), nearest first.
    pub fn nodes_around(level: u8, p: DVec3, reach: f64) -> Vec<GalNodeKey> {
        let n = 1u64 << level;
        let size = 2.0 * ROOT_HALF_PC / n as f64;
        let range = |c: f64| -> (u32, u32) {
            let lo = ((c - reach + ROOT_HALF_PC) / size).floor().clamp(0.0, (n - 1) as f64);
            let hi = ((c + reach + ROOT_HALF_PC) / size).floor().clamp(0.0, (n - 1) as f64);
            (lo as u32, hi as u32)
        };
        let (rx, ry, rz) = (range(p.x), range(p.y), range(p.z));
        let mut out = Vec::new();
        for x in rx.0..=rx.1 {
            for y in ry.0..=ry.1 {
                for z in rz.0..=rz.1 {
                    out.push(GalNodeKey::from_cell(level, [x, y, z]));
                }
            }
        }
        out.sort_by(|a, b| a.distance_to(p).total_cmp(&b.distance_to(p)));
        out
    }

    /// Distance from `p` to the node cube (0 inside).
    pub fn distance_to(self, p: DVec3) -> f64 {
        let gap = ((p - self.center()).abs() - DVec3::splat(self.half())).max(DVec3::ZERO);
        gap.length()
    }

    /// RNG seed of the node (`CreateUniverseBlock`, 32-bit wrap-around arithmetic).
    pub fn seed(self) -> u32 {
        (self.id as u32 as i32)
            .wrapping_mul(0xcb0)
            .wrapping_add(0) // `a` field of the node, always 0 for the universe octree
            .wrapping_add((self.level as i32).wrapping_mul(0x19a2)) as u32
    }
}

/// Procedural galaxy name `RG 0-<level>-<node>-<index>`.
pub fn galaxy_name(key: GalNodeKey, index: u32) -> String {
    format!("RG 0-{}-{}-{}", key.level, key.id, index)
}

/// Parses `RG a-level-node-index` (case-insensitive, `a` must be 0).
pub fn parse_galaxy_name(s: &str) -> Option<(GalNodeKey, u32)> {
    let s = s.trim();
    let rest = s.get(..3).filter(|p| p.eq_ignore_ascii_case("RG ")).map(|_| &s[3..])?;
    let mut it = rest.trim().split('-');
    let a: i64 = it.next()?.trim().parse().ok()?;
    let level: u8 = it.next()?.trim().parse().ok()?;
    let id: u64 = it.next()?.trim().parse().ok()?;
    let idx: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || a != 0 || level >= LEVELS || id >= 8u64.pow(level as u32) {
        return None;
    }
    Some((GalNodeKey { level, id }, idx))
}

// ---------------------------------------------------------------------------------------------
// Numerical helpers (erf, normal quantile)
// ---------------------------------------------------------------------------------------------

/// Error function (W. J. Cody rational approximations, ~1e-16).
pub fn erf(x: f64) -> f64 {
    let ax = x.abs();
    let r = if ax < 0.5 {
        const P: [f64; 5] = [3.161123743870565596947, 1.138641541510501556495e2, 3.774852376853020208137e2, 3.209377589138469472562e3, 1.857777061846031526730e-1];
        const Q: [f64; 4] = [2.360129095234412093499e1, 2.440246379344441733056e2, 1.282616526077372275645e3, 2.844236833439170622273e3];
        let y = ax * ax;
        let mut num = P[4] * y;
        let mut den = y;
        for i in 0..3 {
            num = (num + P[i]) * y;
            den = (den + Q[i]) * y;
        }
        ax * (num + P[3]) / (den + Q[3])
    } else {
        1.0 - erfc_pos(ax)
    };
    if x < 0.0 { -r } else { r }
}

/// `erfc(x)` for `x >= 0.46875` (Cody), also valid above 0.
fn erfc_pos(x: f64) -> f64 {
    if x < 4.0 {
        const C: [f64; 9] = [5.64188496988670089e-1, 8.88314979438837594, 6.61191906371416295e1, 2.98635138197400131e2, 8.81952221241769090e2, 1.71204761263407058e3, 2.05107837782607147e3, 1.23033935479799725e3, 2.15311535474403846e-8];
        const D: [f64; 8] = [1.57449261107098347e1, 1.17693950891312499e2, 5.37181101862009858e2, 1.62138957456669019e3, 3.29079923573345963e3, 4.36261909014324716e3, 3.43936767414372164e3, 1.23033935480374942e3];
        let mut num = C[8] * x;
        let mut den = x;
        for i in 0..7 {
            num = (num + C[i]) * x;
            den = (den + D[i]) * x;
        }
        let r = (num + C[7]) / (den + D[7]);
        let ysq = (x * 16.0).trunc() / 16.0;
        let del = (x - ysq) * (x + ysq);
        (-ysq * ysq).exp() * (-del).exp() * r
    } else {
        const P: [f64; 6] = [3.05326634961232344e-1, 3.60344899949804439e-1, 1.25781726111229246e-1, 1.60837851487422766e-2, 6.58749161529837803e-4, 1.63153871373020978e-2];
        const Q: [f64; 5] = [2.56852019228982242, 1.87295284992346725, 5.27905102951428412e-1, 6.05183413124413191e-2, 2.33520497626869185e-3];
        let y = 1.0 / (x * x);
        let mut num = P[5] * y;
        let mut den = y;
        for i in 0..4 {
            num = (num + P[i]) * y;
            den = (den + Q[i]) * y;
        }
        let mut r = y * (num + P[4]) / (den + Q[4]);
        r = (0.5641895835477562869480795 - r) / x;
        let ysq = (x * 16.0).trunc() / 16.0;
        let del = (x - ysq) * (x + ysq);
        (-ysq * ysq).exp() * (-del).exp() * r
    }
}

/// Standard normal quantile (Wichura's AS241, ~1e-16).
pub fn ndtri(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    let q = p - 0.5;
    if q.abs() <= 0.425 {
        let r = 0.180625 - q * q;
        return q
            * (((((((2509.0809287301226727 * r + 33430.575583588128105) * r + 67265.770927008700853) * r + 45921.953931549871457) * r + 13731.693765509461125) * r + 1971.5909503065514427) * r + 133.14166789178437745) * r
                + 3.387132872796366608)
            / (((((((5226.495278852545925 * r + 28729.085735721942674) * r + 39307.89580009271061) * r + 21213.794301586595867) * r + 5394.1960214247511077) * r + 687.1870074920579083) * r + 42.313330701600911252) * r
                + 1.0);
    }
    let mut r = if q < 0.0 { p } else { 1.0 - p };
    r = (-r.ln()).sqrt();
    let val = if r <= 5.0 {
        r -= 1.6;
        (((((((7.7454501427834140764e-4 * r + 0.0227238449892691845833) * r + 0.24178072517745061177) * r + 1.27045825245236838258) * r + 3.64784832476320460504) * r + 5.7694972214606914055) * r + 4.6303378461565452959) * r
            + 1.42343711074968357734)
            / (((((((1.05075007164441684324e-9 * r + 5.475938084995344946e-4) * r + 0.0151986665636164571966) * r + 0.14810397642748007459) * r + 0.68976733498510000455) * r + 1.6763848301838038494) * r + 2.05319162663775882187) * r
                + 1.0)
    } else {
        r -= 5.0;
        (((((((2.01033439929228813265e-7 * r + 2.71155556874348757815e-5) * r + 0.0012426609473880784386) * r + 0.026532189526576123093) * r + 0.29656057182850489123) * r + 1.7848265399172913358) * r + 5.4637849111641143699) * r
            + 6.6579046435011037772)
            / (((((((2.04426310338993978564e-15 * r + 1.4215117583164458887e-7) * r + 1.8463183175100546818e-5) * r + 7.868691311456132591e-4) * r + 0.0148753612908506148525) * r + 0.13692988092273580531) * r + 0.59983220655588793769) * r
                + 1.0)
    };
    if q < 0.0 { -val } else { val }
}

/// Truncated normal draw of `FUN_140443fd0(mean, sigma, min, max)` using one draw from `s`.
pub fn truncated_normal(s: &mut u32, mean: f32, sigma: f32, min: f32, max: f32) -> f32 {
    let u = lcg(s) as f64 * INV32_D;
    let sd = 2.0f32.sqrt() * sigma;
    let lo = erf(((min - mean) / sd) as f64) as f32 as f64;
    let hi = erf(((max - mean) / sd) as f64) as f32 as f64;
    let d = hi - lo;
    if d == 0.0 {
        return (min as f64 + (max - min) as f64 * u) as f32;
    }
    if u == 0.0 {
        return max;
    }
    let p = (d * u + lo + 1.0) * 0.5;
    let z = (ndtri(p) / 2.0f64.sqrt()) as f32;
    z * sd + mean
}

/// Absolute magnitude -> luminosity, solar units (`10^((4.83 - M) * 0.4)`, float arithmetic).
pub fn lum_from_mag(m: f32) -> f32 {
    10f32.powf((4.83f32 - m) * 0.4)
}

/// Apparent magnitude of luminosity `lum` (solar) at `dist_pc` (`FUN_1401d8300`).
pub fn app_mag(lum: f32, dist_pc: f32) -> f32 {
    if lum <= 0.0 {
        return 1.0e38;
    }
    let d = dist_pc.abs();
    ((d.log10() as f64 * 5.0 - 5.0 + 4.83f32 as f64) - (lum.log10() / 0.4) as f64) as f32
}

// ---------------------------------------------------------------------------------------------
// Level table
// ---------------------------------------------------------------------------------------------

/// Absolute-magnitude bounds of the galaxies of one level.
#[derive(Debug, Clone, Copy)]
pub struct LevelBand {
    pub mag_lo: f32,
    pub mag_hi: f32,
    /// Luminosity of `mag_lo` (the brightest galaxy the level can hold), solar.
    pub lum_max: f32,
}

/// The 10 bands of `FUN_1401bda90`: the +-2 sigma span of N(-22, 5) cut into 10 equal-probability
/// slices (the quantile chain is rounded to `f32` at every step, like the exe).
pub fn level_bands() -> &'static [LevelBand; LEVELS as usize] {
    static T: OnceLock<[LevelBand; LEVELS as usize]> = OnceLock::new();
    T.get_or_init(|| {
        let a = (erf(std::f64::consts::SQRT_2) + 1.0) * 0.5;
        let b = (erf(-std::f64::consts::SQRT_2) + 1.0) * 0.5;
        let step = (a - b) / 10.0;
        let mag = |q: f64| ((ndtri(q) / 2.0f64.sqrt()) * 7.0710678118654755 - 22.0) as f32;
        let mut q = (b as f32) as f64;
        let mut mags = [0f32; LEVELS as usize + 1];
        mags[0] = mag(q);
        for m in mags.iter_mut().skip(1) {
            q = ((q + step) as f32) as f64;
            *m = mag(q);
        }
        let mut out = [LevelBand { mag_lo: 0.0, mag_hi: 0.0, lum_max: 0.0 }; LEVELS as usize];
        for (k, band) in out.iter_mut().enumerate() {
            *band = LevelBand { mag_lo: mags[k], mag_hi: mags[k + 1], lum_max: lum_from_mag(mags[k]) };
        }
        out
    })
}

// ---------------------------------------------------------------------------------------------
// Noise tables (built once, deterministic)
// ---------------------------------------------------------------------------------------------

const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57,
    177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143,
    54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255,
    82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224,
    232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204, 176, 115, 121, 50,
    45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
];

/// The universe's cosmic-web tables.
pub struct CosmicWeb {
    /// Value-noise lattice (`T[k] = 2 * PERM[k & 255] / 255 - 1`, 513 entries).
    val: Vec<f64>,
    /// Voronoi feature points of a 64^3 periodic grid, `[x, y, z]` per cell.
    points: Vec<[f64; 3]>,
}

/// State of the C `rand` stream when the Voronoi table is generated (decoded: the stream starts at
/// 0 and the float noise table `FUN_140069040` consumes `3 * 64^3` draws first).
pub const WEB_SEED: u32 = 0;

impl CosmicWeb {
    fn build(seed: u32) -> Self {
        let mut s = seed;
        for _ in 0..3 * 64 * 64 * 64 {
            lcg(&mut s);
        }
        let mut points = Vec::with_capacity(64 * 64 * 64);
        for _ in 0..64 * 64 * 64 {
            let x = lcg(&mut s) as f64 * INV32_D;
            let y = lcg(&mut s) as f64 * INV32_D;
            let z = lcg(&mut s) as f64 * INV32_D;
            points.push([x, y, z]);
        }
        let val = (0..513).map(|k| (2.0 * PERM[k & 255] as f64) / 255.0 - 1.0).collect();
        CosmicWeb { val, points }
    }

    /// Value noise in [-1, 1] (`FUN_140061200`).
    fn noise(&self, p: [f64; 3]) -> f64 {
        let fl = |v: f64| -> i32 { if v < 0.0 { (v as i32) - 1 } else { v as i32 } };
        let (xi, yi, zi) = (fl(p[0]), fl(p[1]), fl(p[2]));
        let (x, y, z) = (p[0] - xi as f64, p[1] - yi as f64, p[2] - zi as f64);
        let perm = |i: usize| PERM[i & 255] as usize;
        let (xm, ym, zm) = ((xi & 255) as usize, (yi & 255) as usize, (zi & 255) as usize);
        let a = perm(xm) + ym;
        let b = perm(xm + 1) + ym;
        let aa = perm(a) + zm;
        let ab = perm(a + 1) + zm;
        let ba = perm(b) + zm;
        let bb = perm(b + 1) + zm;
        let sm = |t: f64| (3.0 - 2.0 * t) * t * t;
        let (sx, sy, sz) = (sm(x), sm(y), sm(z));
        let t = &self.val;
        let lerp = |a: f64, b: f64, w: f64| (b - a) * w + a;
        let x0 = lerp(t[aa], t[ba], sx);
        let x0z = lerp(t[aa + 1], t[ba + 1], sx);
        let x1 = lerp(t[ab], t[bb], sx);
        let x1z = lerp(t[ab + 1], t[bb + 1], sx);
        let y0 = lerp(x0, x1, sy);
        let y0z = lerp(x0z, x1z, sy);
        lerp(y0, y0z, sz)
    }

    /// 3-octave vector fBm used as a domain warp (`FUN_140062490`: gain 0.6, lacunarity 2.218282).
    fn warp(&self, p: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0; 3];
        let mut amp = 1.0;
        let mut q = p;
        for _ in 0..3 {
            let n0 = self.noise(q);
            let n1 = self.noise([q[0] + 3.33, q[1] + 5.71, q[2] + 1.96]);
            let n2 = self.noise([q[0] + 7.77, q[1] + 2.65, q[2] + 4.37]);
            out[0] += n0 * amp;
            out[1] += n1 * amp;
            out[2] += n2 * amp;
            amp *= 0.6;
            q = [q[0] * 2.218282, q[1] * 2.218282, q[2] * 2.218282];
        }
        out
    }

    /// Squared distances to the nearest and second-nearest feature point (`FUN_1400621d0`).
    fn voronoi(&self, p: [f64; 3]) -> (f64, f64) {
        let fl = |v: f64| -> i32 { if v < 0.0 { (v as i32) - 1 } else { v as i32 } };
        let (xi, yi, zi) = (fl(p[0]), fl(p[1]), fl(p[2]));
        let (fx, fy, fz) = (p[0] - xi as f64, p[1] - yi as f64, p[2] - zi as f64);
        let (mut f1, mut f2) = (f64::MAX, f64::MAX);
        for dz in -1..=1i32 {
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    let cx = ((xi + dx) & 63) as usize;
                    let cy = ((yi + dy) & 63) as usize;
                    let cz = ((zi + dz) & 63) as usize;
                    let pt = &self.points[(cy + cz * 64) * 64 + cx];
                    let ex = dx as f64 + pt[0] - fx;
                    let ey = dy as f64 + pt[1] - fy;
                    let ez = dz as f64 + pt[2] - fz;
                    let d = ex * ex + ey * ey + ez * ez;
                    if d < f1 {
                        f2 = f1;
                        f1 = d;
                    } else if d < f2 {
                        f2 = d;
                    }
                }
            }
        }
        (f1, f2)
    }

    /// Acceptance probability of a galaxy candidate at universe position `p` (parsecs).
    pub fn density(&self, p: DVec3) -> f64 {
        let s = [p.x * 2.0e-8, p.y * 2.0e-8, p.z * 2.0e-8];
        let w = self.warp([s[0] * 2.3179, s[1] * 2.3179, s[2] * 2.3179]);
        let q = [w[0] * 0.15 + s[0], w[1] * 0.15 + s[1], w[2] * 0.15 + s[2]];
        let (f1, f2) = self.voronoi(q);
        (1.0 - (f2.sqrt() - f1.sqrt()).abs()).powf(20.0)
    }
}

/// The shared cosmic-web tables (built on first use, about 10 ms).
pub fn cosmic_web() -> &'static CosmicWeb {
    static W: OnceLock<CosmicWeb> = OnceLock::new();
    W.get_or_init(|| CosmicWeb::build(WEB_SEED))
}

// ---------------------------------------------------------------------------------------------
// Galaxy records
// ---------------------------------------------------------------------------------------------

/// A procedural galaxy (`seGalaxy` record of the exe, in decoded units).
#[derive(Debug, Clone, PartialEq)]
pub struct ProcGalaxy {
    pub node: GalNodeKey,
    /// Index among the accepted galaxies of the node (the last number of the RG name).
    pub index: u32,
    /// Universe position, parsecs.
    pub pos_pc: DVec3,
    pub abs_mag: f32,
    /// Luminosity, solar units.
    pub lum: f32,
    /// Hubble type index 1..=16 (E0..E7, S0, Sa, Sb, Sc, SBa, SBb, SBc, Irr).
    pub hubble: u8,
    /// A very luminous galaxy (`L > 1e12`) drawn as a compact one (5 %); radius uses `L * 1e-3`.
    pub compact: bool,
    /// Radius, parsecs (`sqrt(L / surface-luminosity)`, at most 50 000).
    pub radius_pc: f32,
    /// Random orientation, `(w, x, y, z)` (uniform in the 4-cube, normalised).
    pub quat: [f32; 4],
    /// Raw draw for the sprite-model pool pick (`pool[type][roll % len]`), types 9..=16 only.
    pub model_roll: Option<u32>,
    /// Apparent magnitude seen from the Sun (always above [`MAX_APP_MAG`]).
    pub app_mag: f32,
}

impl ProcGalaxy {
    pub fn name(&self) -> String {
        galaxy_name(self.node, self.index)
    }

    /// Hubble type string (`E3`, `Sb`, `SBc`, `Irr`...).
    pub fn kind(&self) -> &'static str {
        hubble_name(self.hubble)
    }

    /// Visual radius: E galaxies are drawn `GalaxyEllipticalScale` (2.4) times larger than their
    /// luminosity radius.
    pub fn display_radius_pc(&self) -> f64 {
        if self.hubble <= 8 { self.radius_pc as f64 * 2.4 } else { self.radius_pc as f64 }
    }

    /// Absolute magnitude the galaxy is drawn with. SE's radius law (`sqrt(L / surface
    /// luminosity)`) gives every galaxy of a type the same surface brightness, but the radius
    /// stops at 50 kpc and compact galaxies take 1/1000 of their light: such galaxies (nearly all
    /// of levels 0-2, M -32 to -24.5) would be drawn up to thousands of times brighter per area
    /// than the rest, washed-out white discs. SE draws them like the others (its level-3 ones,
    /// M -24 at the cap, look dimmer than M 31), so their light is cut to what the law gives their
    /// radius. Irregulars draw their surface luminosity at random; 300 is assumed.
    pub fn vis_abs_mag(&self) -> f32 {
        let sigma = match self.hubble {
            1..=8 => (9 - self.hubble as i32) as f32 * 30.0,
            9..=15 => 30.0,
            _ => 300.0,
        };
        let lum = self.lum.min(sigma * self.radius_pc * self.radius_pc);
        if lum > 0.0 { 4.83 - 2.5 * lum.log10() } else { self.abs_mag }
    }

    /// Flattening of an E galaxy (`En` -> n / 10).
    pub fn ellipticity(&self) -> f32 {
        if (1..=8).contains(&self.hubble) { (self.hubble - 1) as f32 / 10.0 } else { 0.0 }
    }
}

pub fn hubble_name(t: u8) -> &'static str {
    const N: [&str; 17] = ["E0", "E0", "E1", "E2", "E3", "E4", "E5", "E6", "E7", "S0", "Sa", "Sb", "Sc", "SBa", "SBb", "SBc", "Irr"];
    N[(t as usize).min(16)]
}

/// Hubble type of a luminosity and a uniform draw (`FUN_1401bd930`): dwarfs (L < 1e9) are 40 %
/// ellipticals, 5 % lenticular-to-Sa and 55 % irregular; giants are 11 % E, 15 % S0, 71 % spirals
/// (barred ones 54 % / 54 % / 44 % of Sa/Sb/Sc) and 3 % Irr.
pub fn pick_hubble(lum: f32, u: f32) -> u8 {
    let trunc = |v: f32| v as i32;
    if lum < 1.0e9 {
        if u < 0.4 {
            return (1 - trunc((u / 0.4) * -8.0)) as u8;
        }
        if u < 0.45 {
            return (10 - trunc(((u - 0.45) / (0.45 - 0.4)) * -3.0)) as u8;
        }
    } else {
        if u < 0.11 {
            return (1 - trunc((u / 0.11) * -8.0)) as u8;
        }
        if u < 0.26 {
            return 9;
        }
        if u < 0.97 {
            let f = (u - 0.26) / (0.97 - 0.26);
            if f < 0.157 {
                return if f / 0.157 < 0.529 { 13 } else { 10 };
            }
            if f < 0.458 {
                return if f / 0.458 < 0.539 { 14 } else { 11 };
            }
            return if f / 1.0 < 0.443 { 15 } else { 12 };
        }
    }
    16
}

fn random_quat(s: &mut u32) -> [f32; 4] {
    let mut q = [0f32; 4];
    for v in q.iter_mut() {
        let u = lcg(s) as f32;
        *v = 1.0 - (u + u) * INV32;
    }
    let mut n = q[1] * q[1] + q[0] * q[0] + q[2] * q[2] + q[3] * q[3];
    if n == 0.0 {
        q[0] = 1.0;
        n = 1.0;
    }
    let k = 1.0 / n.sqrt();
    [q[0] * k, q[1] * k, q[2] * k, q[3] * k]
}

/// Radius divisor class of `FUN_140464db0` (surface luminosity, solar per pc^2).
fn radius_pc(lum: f32, hubble: u8, compact: bool, s: &mut u32) -> f32 {
    if lum <= 0.0 {
        return 0.0;
    }
    let l = if compact { lum * 0.001 } else { lum };
    let div = match hubble {
        1..=8 => (9 - hubble as i32) as f32 * 0.3 * 100.0,
        9..=15 => 30.0,
        16 => lcg(s) as f32 * 5.0 * INV32 * 300.0,
        _ => 1000.0,
    };
    (l / div).sqrt().min(50_000.0)
}

/// Generates the galaxies of one node. `pools[t]` is the number of sprite models for Hubble type
/// `t` (pass [`DEFAULT_MODEL_POOLS`] or the real catalog counts; it changes how many random draws
/// follow, so the same pools must be used everywhere for stable results).
pub fn generate_node(key: GalNodeKey, pools: &[u32; 17]) -> Vec<ProcGalaxy> {
    // SE has no galaxies at level 0 (every `RG 0-0-0-*` is "Object not found"; the band M -32 to
    // -27.9 stays empty), while levels 1-3 match it galaxy for galaxy.
    if key.level == 0 {
        return Vec::new();
    }
    let web = cosmic_web();
    let band = level_bands()[key.level as usize];
    let seed = key.seed();
    let (mut s0, mut s1) = (seed, seed);
    let center = key.center();
    let half = key.half();
    let two_h = (half + half) as f32;
    let mut out = Vec::new();
    for _ in 0..CANDIDATES {
        let mut off = [0f64; 3];
        for o in off.iter_mut() {
            let u = lcg(&mut s1);
            *o = (two_h * (u as f32) * INV32) as f64 - half;
        }
        let p = DVec3::new(center.x + off[0], center.y + off[1], center.z + off[2]);
        let dens = web.density(p);
        let u4 = lcg(&mut s1);
        if u4 as f64 * INV32_D > dens {
            continue;
        }
        // Accepted: magnitude (second stream), type, orientation, flags, radius, model draw.
        let m = truncated_normal(&mut s0, -22.0, 5.0, band.mag_lo, band.mag_hi);
        let lum = if m == -1.0e38 { 0.0 } else { lum_from_mag(m) };
        let u = lcg(&mut s1) as f32 * INV32;
        let hubble = pick_hubble(lum, u);
        let quat = random_quat(&mut s1);
        let mut compact = false;
        if lum > 999_999_995_904.0 && (lcg(&mut s1) as f32 * INV32) < 0.05 {
            compact = true;
        }
        let radius = radius_pc(lum, hubble, compact, &mut s1);
        let mut model_roll = None;
        if hubble >= 9 && pools[hubble as usize] != 0 {
            model_roll = Some(lcg(&mut s1));
        }
        let pos = DVec3::new(center.x + off[0] as f32 as f64, center.y + off[1] as f32 as f64, center.z + off[2] as f32 as f64);
        let dist = pos.length() as f32;
        let am = app_mag(lum, dist);
        if MAX_APP_MAG < am {
            out.push(ProcGalaxy {
                node: key,
                index: out.len() as u32,
                pos_pc: pos,
                abs_mag: m,
                lum,
                hubble,
                compact,
                radius_pc: radius,
                quat,
                model_roll,
                app_mag: am,
            });
        }
    }
    out
}

/// Generates only galaxy `index` of the node (convenience for name lookups).
pub fn galaxy_by_name(name: &str, pools: &[u32; 17]) -> Option<ProcGalaxy> {
    let (key, idx) = parse_galaxy_name(name)?;
    generate_node(key, pools).into_iter().nth(idx as usize)
}

/// Is a node worth generating when seen from `cam` with limiting magnitude `mag_limit`? Even the
/// brightest galaxy of the level, at the node's nearest point, must be above the limit.
pub fn node_visible(key: GalNodeKey, cam: DVec3, mag_limit: f32) -> bool {
    let d = key.distance_to(cam).max(1.0) as f32;
    app_mag(level_bands()[key.level as usize].lum_max, d) < mag_limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_se_lookups() {
        // Values from SE 0.991's info panel (Select "RG ...").
        assert!(galaxy_by_name("RG 0-0-0-1", &DEFAULT_MODEL_POOLS).is_none());
        for (name, kind, mag, lum) in [
            ("RG 0-1-0-0", "Sc", -26.34, 2.94e12),
            ("RG 0-2-0-1", "SBb", -24.94, 8.11e11),
            ("RG 0-3-0-769", "Sc", -24.03, 3.5177e11),
            ("RG 0-3-116-2668", "SBa", -24.35, 4.7088e11),
        ] {
            let g = galaxy_by_name(name, &DEFAULT_MODEL_POOLS).unwrap();
            assert_eq!(g.kind(), kind, "{name}");
            assert!((g.abs_mag - mag).abs() < 0.015, "{name} {}", g.abs_mag);
            assert!((g.lum / lum - 1.0).abs() < 0.01, "{name} {}", g.lum);
            // Drawn at the surface brightness of SE's radius law, not 10-100 times brighter.
            assert!(g.vis_abs_mag() > -23.0 && g.vis_abs_mag() < -22.0, "{name} {}", g.vis_abs_mag());
        }
    }

    #[test]
    fn containing_inverts_center() {
        for &(l, id) in &[(1u8, 3u64), (4, 1000), (9, 110_030_168)] {
            let k = GalNodeKey { level: l, id };
            assert_eq!(GalNodeKey::containing(l, k.center()), k);
            assert_eq!(k.parent().unwrap().child((id % 8) as u8), k);
        }
    }

    #[test]
    fn grid_cells_match_centers() {
        for level in [1u8, 3, 7] {
            for cell in [[0u32, 0, 0], [1, 0, 1], [(1 << level) - 1, 0, 3 % (1 << level)]] {
                let k = GalNodeKey::from_cell(level, cell);
                let c = k.center();
                let size = 2.0 * k.half();
                for a in 0..3 {
                    let got = ((c[a] + ROOT_HALF_PC) / size - 0.5).round() as u32;
                    assert_eq!(got, cell[a]);
                }
            }
        }
        let near = GalNodeKey::nodes_around(4, DVec3::new(1.0e9, -2.0e9, 3.0e8), 1.0e9);
        assert!(!near.is_empty());
        assert!(near.windows(2).all(|w| w[0].distance_to(DVec3::new(1.0e9, -2.0e9, 3.0e8)) <= w[1].distance_to(DVec3::new(1.0e9, -2.0e9, 3.0e8))));
        assert_eq!(near[0], GalNodeKey::containing(4, DVec3::new(1.0e9, -2.0e9, 3.0e8)));
    }

    #[test]
    fn names_round_trip() {
        let k = GalNodeKey { level: 9, id: 110_030_168 };
        let n = galaxy_name(k, 905);
        assert_eq!(n, "RG 0-9-110030168-905");
        assert_eq!(parse_galaxy_name(&n), Some((k, 905)));
        assert_eq!(parse_galaxy_name("rg 0-3-116-2668"), Some((GalNodeKey { level: 3, id: 116 }, 2668)));
        assert_eq!(parse_galaxy_name("RG 0-3-9999-1"), None);
        assert_eq!(parse_galaxy_name("RG 1-3-1-1"), None);
        assert_eq!(parse_galaxy_name("RS 0-3-1-1"), None);
    }

    #[test]
    fn level_bands_split_the_gaussian() {
        let b = level_bands();
        assert!((b[0].mag_lo + 32.0).abs() < 0.01, "{}", b[0].mag_lo);
        assert!((b[9].mag_hi + 12.0).abs() < 0.01, "{}", b[9].mag_hi);
        assert!((b[4].mag_hi + 22.0).abs() < 0.01);
        for k in 0..9 {
            assert_eq!(b[k].mag_hi, b[k + 1].mag_lo);
        }
    }

    #[test]
    fn special_functions() {
        assert!((erf(0.5) - 0.5204998778130465).abs() < 1e-14);
        assert!((erf(0.171) - 0.19108851021670556).abs() < 1e-14);
        assert!((erf(0.3) - 0.3286267594591274).abs() < 1e-14);
        assert!((erf(1.0) - 0.8427007929497149).abs() < 1e-14);
        assert!((erf(3.0) - 0.9999779095030014).abs() < 1e-14);
        assert!((erf(4.5) - 0.9999999998033839).abs() < 1e-14);
        assert!((erf(-2.0) + 0.9953222650189527).abs() < 1e-14);
        assert!((ndtri(0.975) - 1.959963984540054).abs() < 1e-12);
        assert!((ndtri(0.5)).abs() < 1e-15);
    }

    #[test]
    fn generation_is_deterministic_and_plausible() {
        let k = GalNodeKey { level: 6, id: 12345 };
        let a = generate_node(k, &DEFAULT_MODEL_POOLS);
        let b = generate_node(k, &DEFAULT_MODEL_POOLS);
        assert_eq!(a, b);
        assert!(!a.is_empty() && a.len() < CANDIDATES);
        let band = level_bands()[6];
        for (i, g) in a.iter().enumerate() {
            assert_eq!(g.index as usize, i);
            assert!(g.abs_mag >= band.mag_lo - 1e-3 && g.abs_mag <= band.mag_hi + 1e-3);
            assert!(g.app_mag > MAX_APP_MAG);
            assert!((1..=16).contains(&g.hubble));
            assert!((g.pos_pc - k.center()).abs().max_element() <= k.half() * 1.0001);
            assert_eq!(g.model_roll.is_some(), g.hubble >= 9);
        }
        assert_eq!(galaxy_by_name(&a[3].name(), &DEFAULT_MODEL_POOLS).unwrap(), a[3]);
    }

    #[test]
    fn types_follow_the_luminosity_split() {
        assert_eq!(pick_hubble(1e8, 0.0), 1);
        assert_eq!(pick_hubble(1e8, 0.5), 16);
        assert_eq!(pick_hubble(1e10, 0.2), 9);
        assert!((10..=15).contains(&pick_hubble(1e10, 0.6)));
        assert_eq!(pick_hubble(1e10, 0.99), 16);
    }

    #[test]
    fn web_density_is_a_probability_with_filaments() {
        let w = cosmic_web();
        let mut sum = 0.0;
        let n = 4000;
        for i in 0..n {
            let t = i as f64 / n as f64;
            let d = w.density(DVec3::new(t * 4.0e9 - 2.0e9, (t * 7.3).fract() * 3.0e9, (t * 3.1).fract() * -2.0e9));
            assert!((0.0..=1.0).contains(&d));
            sum += d;
        }
        let mean = sum / n as f64;
        assert!(mean > 0.005 && mean < 0.5, "mean density {mean}");
    }

    /// The decisive check: the galaxies named by two of SpaceEngine's own saved places sit right
    /// next to the places' camera positions (111 pc from a 60 pc dwarf, 18 kpc from a giant).
    #[test]
    fn matches_saved_places_of_the_exe() {
        let near = |lvl: u8, id: u64, idx: u32, cam: [f64; 3], max_d: f64, kind: &str| {
            let g = galaxy_by_name(&galaxy_name(GalNodeKey { level: lvl, id }, idx), &DEFAULT_MODEL_POOLS).unwrap();
            let d = (g.pos_pc - DVec3::from_array(cam)).length();
            assert!(d < max_d, "{} is {d} pc from its place", g.name());
            assert_eq!(g.kind(), kind);
            g
        };
        // "The Smallest Heart"
        let g = near(9, 110_030_168, 905, [-3.682_388_864_606_325e9, -1.760_958_967_765_144_5e9, 2.995_017_498_286_797e9], 500.0, "Irr");
        assert!((g.radius_pc - 60.0).abs() < 1.0);
        // parent galaxy of "Star Cluster Within A Super Dense Nebula"
        let g = near(3, 116, 2668, [0.832_489_535_329_029_6e9, 1.588_751_173_153_572e9, -0.552_580_100_512_168_8e9], 1.0e5, "SBa");
        assert_eq!(g.radius_pc, 50_000.0);
    }
}
