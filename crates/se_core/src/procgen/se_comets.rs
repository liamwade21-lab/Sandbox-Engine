//! SE's comets: the comets of a procedural star (FUN_1404fc9b0, run by CreateProcedural
//! FUN_1404f7c40 for every star FUN_1404f9210 gave a comet count), the tail values SE fills in
//! for every comet (FUN_140486280) and the particles of a tail (FUN_1404858f0).
//!
//! - A single star that is not a brown dwarf gets 100-199 comets (drawn in
//!   [`super::se_layout::layout`]). With `R` the inner edge of the planet zone (FUN_14050a890),
//!   perihelia are a truncated normal about `U(5, 10) R` (sigma a third of the way down to
//!   `R / 4`, cut at `R / 4` and as far above), aphelia one about 1.2 times the sphere of
//!   influence `S` (sigma 0.8 S / 3, cut to 0.4-2 S). `e = (Q - q) / (Q + q)` (at most 0.9999),
//!   `a = q / (1 - e)`; inclination, node, argument of perihelion and mean anomaly are uniform in
//!   +-180 degrees. No comets when the perihelion range reaches past 0.4 S.
//! - The comet body (FUN_140502710 on the comet's own seeds): mass `10^U(-20, -8.6)` Jupiter
//!   masses, density `tn(500, 100, 200, 3000)` kg/m^3 for the radius (FUN_1404c8990), albedo
//!   `tn(0.04, 0.01, 0.01, 0.1)` (FUN_1404ca7e0), rotation `tn(2, 1.5, 0.1, 20)` h, then the tail
//!   values of [`tail_defaults`]. Names are the star's name plus `C` and the number (FUN_1404fc9b0
//!   appends 0x43 and `%d`).
//!
//! The draws of the composition, 0x1210 and 0x1214 (FUN_1401aa500, FUN_14050a5f0, FUN_14050a520)
//! are not decoded and so do not advance the streams, and the orbit stream is the port's: SE
//! draws the orbits on whatever its thread's generator holds after the planets.

use crate::surface_gen::{erf, inv_norm_cdf, SeRng};

use super::se_layout::{inner_limit_au, SeStar};

const INV_2_32: f64 = 2.328_306_436_538_696_3e-10;
const PI: f64 = std::f64::consts::PI;
const TAU: f64 = std::f64::consts::TAU;
/// 1 km in parsecs and 0.1 AU in parsecs (DAT_1406d4038, _DAT_1406d4298).
const PC_PER_KM: f64 = 3.240_779_289_444_365e-14;
const TENTH_AU_PC: f64 = 4.848_136_883_338_228e-7;
/// 0.1 AU in km (_DAT_1406d5590).
const TENTH_AU_KM: f64 = 14_959_787.292_918_198;
/// Jupiter / Earth mass (DAT_1406d5948) and the Earth's mass in kg (DAT_1406d5ce0).
const JUPITER_ME: f32 = 317.828_4;
const EARTH_KG: f32 = 5.972_365e24;
/// Earth / Sun mass ratio and (Sun + Earth) / Sun (_DAT_1406d4330, DAT_1406d4b10).
const EARTH_SUN: f64 = 3.003_489_382_535_917_7e-6;
const SUN_EARTH: f64 = 1.000_003_003_489_382_5;

/// A comet's tail as SE keeps it (body record +0x1b9c..+0x1bec, CometTail block): `MaxLength`
/// (km), `Particles`, `GasToDust` (the gas share of the particles), the brightness and colour of
/// both kinds, and the body's `AbsMagn` and `SlopeParam` (the tail length goes as `r^-SlopeParam`).
#[derive(Debug, Clone, PartialEq)]
pub struct CometTail {
    pub max_length_km: f32,
    pub particles: u32,
    pub gas_to_dust: f32,
    pub gas_bright: f32,
    pub dust_bright: f32,
    pub gas_color: [f32; 3],
    pub dust_color: [f32; 3],
    pub abs_magn: f32,
    pub slope_param: f32,
}

/// What a catalog gives of [`CometTail`] (`None` = not given). `MaxLength` is in AU as written;
/// `Bright` sets both brightnesses before `GasBright` / `DustBright` (FUN_14046c0f0).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CometTailGiven {
    pub max_length_au: Option<f64>,
    pub particles: Option<i64>,
    pub gas_to_dust: Option<f64>,
    pub gas_bright: Option<f64>,
    pub dust_bright: Option<f64>,
    pub gas_color: Option<[f64; 3]>,
    pub dust_color: Option<[f64; 3]>,
    pub abs_magn: Option<f64>,
    pub slope_param: Option<f64>,
}

impl CometTailGiven {
    /// From a comet's catalog block (its `CometTail` sub-block plus `AbsMagn` / `SlopeParam`).
    pub fn from_body_block(b: &crate::sc::Block) -> Self {
        let mut g = CometTailGiven { abs_magn: b.f64("AbsMagn"), slope_param: b.f64("SlopeParam"), ..Default::default() };
        if let Some(t) = b.sub("CometTail") {
            let bright = t.f64("Bright");
            g.max_length_au = t.f64("MaxLength");
            g.particles = t.f64("Particles").map(|p| p as i64);
            g.gas_to_dust = t.f64("GasToDust");
            g.gas_bright = t.f64("GasBright").or(bright);
            g.dust_bright = t.f64("DustBright").or(bright);
            g.gas_color = t.vec3("GasColor");
            g.dust_color = t.vec3("DustColor");
        }
        g
    }
}

/// SE's tail values for a comet of `mass_me` Earth masses and eccentricity `ecc`, keeping what
/// the catalog gives (FUN_140486280):
/// - `MaxLength` `(clamp(log10 M + 15, 0, 8) * 0.12375 + 0.01) * 0.1` AU;
/// - `Particles` 1000 + 2000 `clamp((MaxLength / 0.1 AU - 0.01) / 0.99, 0, 1)` (also when the
///   catalog gives fewer than 1);
/// - `SlopeParam` `tn(4, 2, 2, 20)` and `AbsMagn` `tn(8, 4, -2, 20)` (stream b);
/// - `GasToDust` 0.25; with `g = U(0, 0.5)` (+0.5 on an open orbit) and `k = U(0.2, 1) / 20`,
///   `GasBright` `g k` and `DustBright` `(1 - g) k` (stream a, drawn either way);
/// - `GasColor` (0.04 0.12 0.2); `DustColor` white, 5% (0.47 1 0.77).
pub fn tail_defaults(r: &mut SeRng, given: &CometTailGiven, mass_me: f64, ecc: f64) -> CometTail {
    let max_length_km = match given.max_length_au {
        // FUN_14046c0f0 turns AU into km.
        Some(au) => (au as f32 as f64 * 149_597_870.7) as f32,
        None => {
            let f = (mass_me as f32).log10() + 15.0;
            let f = if f >= 0.0 { f.min(8.0) } else { 0.0 };
            ((f * 0.125 * 0.99 + 0.01) as f64 * TENTH_AU_KM) as f32
        }
    };
    let particles = match given.particles {
        Some(p) if p >= 1 => p as u32,
        _ => {
            let f = (((max_length_km as f64 * PC_PER_KM) / TENTH_AU_PC - 0.01) / 0.99) as f32;
            let n = if f >= 0.0 {
                if f <= 1.0 { f * 2000.0 + 1000.0 } else { 3000.0 }
            } else {
                1000.0
            };
            n as u32
        }
    };
    let slope_param = given.slope_param.map_or_else(|| r.tnormal(4.0, 2.0, 2.0, 20.0), |v| v as f32);
    let abs_magn = given.abs_magn.map_or_else(|| r.tnormal(8.0, 4.0, -2.0, 20.0), |v| v as f32);
    let gas_to_dust = given.gas_to_dust.map_or(0.25, |v| v as f32);
    let mut g = r.uniform(0.5);
    if ecc >= 1.0 {
        g += 0.5;
    }
    let k = (r.uniform(0.8) + 0.2) / 20.0;
    let gas_bright = given.gas_bright.map_or(k * g, |v| v as f32);
    let dust_bright = given.dust_bright.map_or(((1.0 - g as f64) * k as f64) as f32, |v| v as f32);
    let gas_color = given.gas_color.map_or([0.04, 0.12, 0.2], |c| c.map(|x| x as f32));
    let dust_color = match given.dust_color {
        Some(c) => c.map(|x| x as f32),
        None if r.uniform(1.0) >= 0.95 => [0.47, 1.0, 0.77],
        None => [1.0, 1.0, 1.0],
    };
    CometTail { max_length_km, particles, gas_to_dust, gas_bright, dust_bright, gas_color, dust_color, abs_magn, slope_param }
}

/// One generated comet: orbit about its star (AU, degrees, SE's frame before the port's tilt),
/// body and tail.
#[derive(Debug, Clone)]
pub struct SeComet {
    pub a_au: f64,
    pub ecc: f64,
    pub inc_deg: f64,
    pub node_deg: f64,
    pub argp_deg: f64,
    pub ma_deg: f64,
    pub period_days: f64,
    pub mass_me: f64,
    pub radius_km: f64,
    pub albedo: f64,
    pub rot_h: f64,
    pub obliq_deg: f64,
    pub eq_node_deg: f64,
    pub tail: CometTail,
}

/// SE's double-precision truncated normal on stream b (FUN_140441b80).
fn tnormal_d(r: &mut SeRng, mean: f64, sigma: f64, lo: f64, hi: f64) -> f64 {
    r.b = r.b.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
    let u = r.b as f64 * INV_2_32;
    let k = std::f64::consts::SQRT_2 * sigma;
    let a = erf((lo - mean) / k);
    let d = erf((hi - mean) / k) - a;
    if d == 0.0 {
        return (hi - lo) * u + lo;
    }
    inv_norm_cdf((d * u + a + 1.0) * 0.5) / std::f64::consts::SQRT_2 * k + mean
}

/// A uniform angle in [-pi, pi) on stream a, in doubles.
fn angle(r: &mut SeRng) -> f64 {
    r.a = r.a.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
    r.a as f64 * TAU * INV_2_32 - PI
}

/// The `count` comets of single star `s` with sphere of influence `soi_au` (FUN_1404fc9b0);
/// `seed` is SE's system seed (generator +0x170).
pub(super) fn comets(s: &SeStar, seed: u32, count: u32, soi_au: f64) -> Vec<SeComet> {
    if count == 0 {
        return Vec::new();
    }
    let r_in = inner_limit_au(s, false) as f32 as f64;
    let inner = r_in * 0.25;
    let mut orb = SeRng::new(seed.wrapping_mul(0x3d1).wrapping_add(0x2b07), seed.wrapping_mul(0x1f5).wrapping_add(0x0a3f));
    let outer = (orb.uniform(5.0) as f64 + 5.0) * r_in;
    let w1 = (outer - inner) / 3.0;
    let hi1 = w1 * 3.0 + outer;
    let soi = soi_au as f32 as f64;
    let (s12, s2) = (soi * 1.2, soi + soi);
    let w2 = (s2 - s12) / 3.0;
    let lo2 = s12 - w2 * 3.0;
    if hi1 > lo2 {
        return Vec::new();
    }
    let m_star = s.mass_sol / EARTH_SUN;
    (0..count)
        .map(|n| {
            let q = tnormal_d(&mut orb, outer, w1, inner, hi1);
            let big_q = tnormal_d(&mut orb, s12, w2, lo2, s2);
            let x = (big_q - q) / (big_q + q);
            let ecc = if x >= 0.0 { x.min(0.9999) } else { 0.0 };
            let a_au = q / (1.0 - ecc);
            let inc = angle(&mut orb);
            let node = angle(&mut orb);
            let argp = angle(&mut orb);
            let ma = angle(&mut orb);

            // FUN_140502710 on the comet's seeds.
            let mut r = SeRng::new(n.wrapping_mul(0x21ad).wrapping_add(seed.wrapping_mul(0x2244)), seed.wrapping_mul(0xc49).wrapping_add(n.wrapping_mul(0x1938)));
            let _c_to_o = (1.2 - r.uniform(0.4)) * s.c_to_o;
            let _met = (1.2 - r.uniform(0.4)) * s.met;
            let albedo = r.tnormal(0.04, 0.01, 0.01, 0.1);
            let mass_me = 10f32.powf(r.uniform(11.4) - 20.0) * JUPITER_ME;
            let rho = r.tnormal(500.0, 100.0, 200.0, 3000.0);
            let radius_km = (mass_me * EARTH_KG / (rho * 4.188_79)).powf(1.0 / 3.0) * 0.001;
            let eq_node = r.uniform(360.0);
            let obliq = r.uniform(360.0);
            let rot_h = if radius_km < 200.0 { r.tnormal(2.0, 1.5, 0.1, 20.0) } else { 10.0 };
            let tail = tail_defaults(&mut r, &CometTailGiven::default(), mass_me as f64, ecc);
            let period_days = (a_au * a_au * a_au * SUN_EARTH / (m_star + mass_me as f64) / EARTH_SUN).sqrt() * 365.242_189_85;
            SeComet {
                a_au,
                ecc,
                inc_deg: inc.to_degrees(),
                node_deg: node.to_degrees(),
                argp_deg: argp.to_degrees(),
                ma_deg: ma.to_degrees(),
                period_days,
                mass_me: mass_me as f64,
                radius_km: radius_km as f64,
                albedo: albedo as f64,
                rot_h: rot_h as f64,
                obliq_deg: obliq as f64,
                eq_node_deg: eq_node as f64,
                tail,
            }
        })
        .collect()
}

/// Name of a star's `n`-th (1-based) comet: SE appends `C` and the number to the star's name.
pub fn comet_name(star: &str, n: usize) -> String {
    format!("{star}C{n}")
}

/// SE's value-noise fBm (FUN_1400497e0 as set up by FUN_140048fc0(2, 0.6, 2.18282, 0.8)): two
/// octaves, lacunarity 2.18282, amplitude `lacunarity^(-0.6 i)`. SE's lattice values are a table
/// filled at start-up; these are hashed, uniform in [-1, 1].
fn value_fbm(p: [f32; 3]) -> f32 {
    fn lattice(x: i32, y: i32, z: i32) -> f32 {
        let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h as f32) * (2.0 / u32::MAX as f32) - 1.0
    }
    fn noise(p: [f32; 3]) -> f32 {
        let i = p.map(|c| c.floor() as i32);
        let f: [f32; 3] = std::array::from_fn(|k| p[k] - i[k] as f32);
        let s = f.map(|t| t * t * (3.0 - 2.0 * t));
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let v = |dx: i32, dy: i32, dz: i32| lattice(i[0] + dx, i[1] + dy, i[2] + dz);
        let x00 = lerp(v(0, 0, 0), v(1, 0, 0), s[0]);
        let x10 = lerp(v(0, 1, 0), v(1, 1, 0), s[0]);
        let x01 = lerp(v(0, 0, 1), v(1, 0, 1), s[0]);
        let x11 = lerp(v(0, 1, 1), v(1, 1, 1), s[0]);
        lerp(lerp(x00, x10, s[1]), lerp(x01, x11, s[1]), s[2])
    }
    const LACUNARITY: f32 = 2.182_82;
    let mut p = p;
    let mut sum = 0.0;
    for i in 0..2 {
        sum += LACUNARITY.powf(-0.6 * i as f32) * noise(p);
        p = p.map(|c| c * LACUNARITY);
    }
    sum
}

/// One particle of a tail (FUN_1404858f0): a random direction and SE's vertex bytes, phase high
/// and low and the kind (0 gas, 1-255 dust, the dust's response to light pressure); the four
/// corners of its sprite add the fourth byte (0, 0x40, 0x80, 0xc0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TailParticle {
    pub dir: [f32; 3],
    pub data: [u8; 3],
}

/// The particles of a tail, seeded on stream a as SE seeds them (FUN_1404d4100 sets both streams
/// to the same value): directions uniform on the sphere (scaled by a uniform radius the shader
/// normalises away), the first `particles * GasToDust` gas, the dust's kind byte
/// `clamp((fbm + 0.7) / 1.4, 0, 1) * 254 + 1` along a line 5 noise units long.
pub fn tail_particles(seed: u32, particles: u32, gas_to_dust: f32) -> Vec<TailParticle> {
    let mut r = SeRng::new(seed, seed);
    let mut lcg = || {
        r.a = r.a.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        r.a
    };
    const INV: f32 = 2.328_306_4e-10;
    let mut x0 = lcg() as f32 * INV;
    let y0 = lcg() as f32 * INV;
    let z0 = lcg() as f32 * INV;
    let n_gas = (particles as f32 * gas_to_dust) as i32;
    let step = (5.0 / (particles as i32 - n_gas) as f64) as f32;
    (0..particles as i32)
        .map(|i| {
            let phi = lcg() as f32 * std::f32::consts::TAU * INV;
            let rad = lcg() as f32 * 0.999_999 * INV + 1e-6;
            let z = 1.0 - (lcg() as f32 * 2.0) * INV;
            let s = (1.0 - (z * z) as f64).sqrt() as f32 * rad;
            let u = lcg();
            let hi = ((u / 255) as u8).wrapping_add(u as u8);
            let lo = (lcg() % 255) as u8;
            let kind = if i < n_gas {
                0
            } else {
                let n = value_fbm([x0, y0, z0]);
                x0 += step;
                let b = ((n as f64 + 0.7) / 1.4) as f32;
                (if b >= 0.0 { b.min(1.0) } else { 0.0 } * 254.0) as u8 + 1
            };
            TailParticle { dir: [phi.cos() * s, phi.sin() * s, z * rad], data: [hi, lo, kind] }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun() -> SeStar {
        SeStar { mass_sol: 1.0, radius_km: 695_700.0, teff: 5772.0, lum_sol: 1.0, lum_code: 7, letter: 4, c_to_o: 0.55, met: 1.0 }
    }

    #[test]
    fn tail_defaults_follow_se() {
        let mut r = SeRng::new(1, 2);
        // Halley's mass gives 0.0575 AU and 3000 particles (more than 0.1 AU would also be 3000).
        let t = tail_defaults(&mut r, &CometTailGiven::default(), 3.681e-11, 0.97);
        let au = t.max_length_km as f64 / 149_597_870.7;
        assert!((au - (((3.681e-11f64).log10() + 15.0) * 0.12375 + 0.01) * 0.1).abs() < 1e-5, "{au}");
        assert_eq!(t.particles, (((au / 0.1 - 0.01) / 0.99) * 2000.0 + 1000.0) as u32);
        assert_eq!(t.gas_to_dust, 0.25);
        assert_eq!(t.gas_color, [0.04, 0.12, 0.2]);
        assert!((2.0..=20.0).contains(&t.slope_param) && (-2.0..=20.0).contains(&t.abs_magn));
        let k = t.gas_bright + t.dust_bright;
        assert!((0.01..=0.05).contains(&k), "{k}");
        // Tiny comets get the 0.001 AU floor and 1000 particles; given values stay.
        let given = CometTailGiven { max_length_au: Some(0.5), particles: Some(0), gas_bright: Some(0.5), ..Default::default() };
        let t = tail_defaults(&mut r, &given, 1e-20, 1.2);
        assert!((t.max_length_km - 0.5 * 149_597_870.7).abs() < 20.0);
        assert_eq!(t.particles, 3000);
        assert_eq!(t.gas_bright, 0.5);
        let t = tail_defaults(&mut r, &CometTailGiven::default(), 1e-20, 0.5);
        assert!((t.max_length_km as f64 / 149_597_870.7 - 0.001).abs() < 1e-7);
        assert_eq!(t.particles, 1000);
    }

    #[test]
    fn comets_of_a_sun() {
        let s = sun();
        let soi = 40.0;
        let cs = comets(&s, 12345, 150, soi);
        assert_eq!(cs.len(), 150);
        let r_in = inner_limit_au(&s, false);
        for c in &cs {
            let q = c.a_au * (1.0 - c.ecc);
            let big_q = c.a_au * (1.0 + c.ecc);
            assert!(q >= r_in * 0.25 - 1e-9 && q <= 20.0 * r_in, "q {q}");
            assert!(big_q >= 0.4 * soi - 1e-6 && big_q <= 2.0 * soi + 1e-6, "Q {big_q}");
            assert!(c.ecc > 0.5 && c.ecc <= 0.9999);
            assert!((-180.0..180.0).contains(&c.inc_deg) && (-180.0..180.0).contains(&c.ma_deg));
            // 10^(-20..-8.6) Jupiter masses; km-sized at comet densities.
            assert!(c.mass_me > 3e-18 && c.mass_me < 1e-6, "{}", c.mass_me);
            assert!(c.radius_km > 1e-4 && c.radius_km < 400.0, "{}", c.radius_km);
            assert!((0.01..=0.1).contains(&c.albedo));
            assert!(c.tail.particles >= 1000 && c.tail.particles <= 3000);
            let p_yr = (c.a_au.powi(3)).sqrt();
            assert!((c.period_days / 365.24 / p_yr - 1.0).abs() < 1e-3);
        }
        // Deterministic.
        assert_eq!(comets(&s, 12345, 150, soi)[17].a_au, cs[17].a_au);
        // A tiny sphere of influence leaves no room for comets.
        assert!(comets(&s, 1, 150, 2.0).is_empty());
        assert_eq!(comet_name("RS 1-2-3-4", 7), "RS 1-2-3-4C7");
    }

    #[test]
    fn tail_particles_split() {
        let p = tail_particles(77, 1000, 0.25);
        assert_eq!(p.len(), 1000);
        assert!(p[..250].iter().all(|q| q.data[2] == 0));
        assert!(p[250..].iter().all(|q| q.data[2] >= 1));
        let mean = p[250..].iter().map(|q| q.data[2] as f64).sum::<f64>() / 750.0;
        assert!((60.0..200.0).contains(&mean), "{mean}");
        for q in &p {
            let l = (q.dir[0] * q.dir[0] + q.dir[1] * q.dir[1] + q.dir[2] * q.dir[2]).sqrt();
            assert!(l <= 1.0001 && l > 0.0);
        }
        assert_eq!(tail_particles(77, 1000, 0.25), p);
    }
}
