//! SE's procedural planetary-system architecture for a single star (seSolarSystem::
//! CreateProcedural, FUN_1404f7c40).
//!
//! - FUN_1404f9210 gives the star 5-14 planet slots and a sphere of influence of
//!   `tn(40, 8, 10, 1000) * M/M_sun` AU.
//! - FUN_140500700 places planets outward from the inner limit (FUN_14050a890 / FUN_14050a9f0).
//!   Each step multiplies the distance by U(1.2, 2.7), or by U(1.7, 2.2) with an eccentricity
//!   around 0.2 for 7% of slots. A planet hotter than 3200 K is dropped without using a slot.
//! - FUN_140501f60 / FUN_140504e10 draw the planet's type and log-normal mass (gas giant 12%,
//!   ice giant 25%, else terra; capped by the disk mass, 1.2% of the star's). Its composition
//!   and radius then come from [`climate::procedural_composition`] /
//!   [`climate::procedural_radius_km`]. A body of at most 2000 km is a dwarf planet with an
//!   asteroid belt (FUN_1404fd2a0).
//! - The layout stream finally draws each planet's number of moons.
//! - A single star that is not a brown dwarf also gets 100-199 comets ([`super::se_comets`]).
//!
//! The star's seeds are the port's, not SE's (the port's stars are not SE's octree stars), so a
//! system follows SE's rules without matching SE's system for the same star.

use crate::climate;
use crate::surface_gen::SeRng;

const INV_2_32: f64 = 2.328_306_436_538_696_3e-10;
const JUPITER_ME: f64 = 317.828_399_658_203_1;
/// Earth / Sun mass ratio as SE stores it.
const EARTH_SUN: f64 = 3.003_489_382_535_917_7e-6;
const KM_PER_AU: f64 = 149_597_870.7;
const DEG: f64 = std::f64::consts::PI / 180.0;
const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;
/// ProbHotGasGiant + ProbHotIceGiant + ProbHotTerra (main-def.cfg SolarSystemGenerator).
const PROB_HOT: f64 = 0.0049 + 0.0532 + 0.0042;
const PROB_GAS_GIANT: f32 = 0.12;
const PROB_ICE_GIANT: f32 = 0.25;
const PROB_COLD_TERRA: f32 = 0.1;
const PROB_RINGS_DEBRIS: f32 = 0.03;
/// Asteroids per system at most (FUN_140501f60).
pub(super) const MAX_ASTEROIDS: u32 = 1000;

/// The star as the layout reads it. `lum_code`/`letter` are SE's class-word fields
/// ([`climate::star_class_bits`]). A barycentre's host carries its stars' total mass and light
/// and its primary's class.
#[derive(Debug, Clone)]
pub(super) struct SeStar {
    pub mass_sol: f64,
    pub radius_km: f64,
    pub teff: f64,
    pub lum_sol: f64,
    pub lum_code: u32,
    pub letter: u32,
    /// The star's C/O (record +0x1614) and metallicity (+0x1610), which each planet scales by
    /// U(0.8, 1.2).
    pub c_to_o: f32,
    pub met: f32,
}

impl SeStar {
    pub(super) fn brown_dwarf(&self) -> bool {
        self.lum_code == 7 && self.letter >= 7
    }
    pub(super) fn mass_earth(&self) -> f64 {
        self.mass_sol / EARTH_SUN
    }
}

#[derive(Debug, Clone)]
pub(super) struct SePlanet {
    pub a_au: f64,
    pub ecc: f64,
    /// Inclination and node (degrees) about the system's plane; argument of periapsis and mean
    /// anomaly (degrees).
    pub inc_deg: f64,
    pub node_deg: f64,
    pub argp_deg: f64,
    pub ma_deg: f64,
    pub mass_me: f64,
    pub radius_km: f64,
    pub comp: [f32; 6],
    /// SE bulk class ([`crate::surface_gen::bulk`]).
    pub class: i32,
    pub dwarf: bool,
    pub moons: u32,
    pub asteroids: u32,
    /// Bodies of a "debris" ring (ProbRingsDebris), counted in `moons` as SE does.
    pub debris: u32,
    /// SE's record seeds of the planet (its streams A and B, FUN_140500700).
    pub seeds: (u32, u32),
    /// Its number among the system's planets (SE's planet counter).
    pub n: u32,
}

#[derive(Debug, Clone)]
pub(super) struct SeAsteroid {
    pub a_au: f64,
    pub ecc: f64,
    pub inc_deg: f64,
    pub node_deg: f64,
    pub argp_deg: f64,
    pub ma_deg: f64,
}

pub(super) struct SeSystem {
    pub planets: Vec<SePlanet>,
    pub asteroids: Vec<SeAsteroid>,
    /// A single star's comets ([`super::se_comets`]); none for a host of a multiple system.
    pub comets: Vec<super::se_comets::SeComet>,
}

pub(super) fn ua(r: &mut SeRng) -> f64 {
    r.a = r.a.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
    r.a as f64 * INV_2_32
}

/// Sum-of-12 Gaussian on stream B, in doubles (FUN_140441d10).
pub(super) fn gauss_d(r: &mut SeRng, mean: f64, sigma: f64) -> f64 {
    let mut s = 0.0f64;
    for _ in 0..12 {
        r.b = r.b.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        s += r.b as f64;
    }
    (s * INV_2_32 - 6.0) * sigma + mean
}

/// SE's `if lo <= x { min(x, hi) } else { lo }`-style clamp of a drawn count.
fn count(x: f32, lo: f32, hi: f32) -> u32 {
    (if x >= lo { x.min(hi) } else { lo }) as u32
}

/// Inner edge of the planet zone (AU), FUN_14050a9f0: the largest of 1.05 stellar radii, the
/// distance where a planet's equilibrium temperature falls to 800 K (3200 K with hot planets
/// allowed) and `4 * 15545 km * cbrt(2 M/M_earth)`.
pub(super) fn inner_limit_au(s: &SeStar, hot: bool) -> f64 {
    let t_lim = if hot { 3200.0 } else { 800.0 };
    let t_star = match s.lum_code {
        0..=10 => s.teff,
        11 => 5800.0,
        _ => 1e8,
    };
    let ratio = (t_lim / t_star).min(1.0);
    let by_temp = s.radius_km / (2.0 * ratio * ratio);
    let by_size = s.radius_km * 1.05;
    let by_mass = (2.0 * s.mass_earth()).cbrt() * 15_545.24 * 4.0;
    by_temp.max(by_size).max(by_mass) / KM_PER_AU
}

/// A planet drawn on its own streams `(pa, pb)` as SE's planet generator draws one (FUN_140501f60):
/// C/O and metallicity `U(0.8, 1.2)` times the system's, type by `ProbGasGiant` / `ProbIceGiant`, mass
/// `10^tn(...)` Jupiter masses, then composition, class and radius at equilibrium temperature `t_eq`.
/// SE's rogue planets (FUN_1404fc280) are drawn so, without a disk cap.
pub(super) fn lone_planet(pa: u32, pb: u32, t_eq: f32, c_to_o: f32, met: f32) -> (f64, f64, [f32; 6], i32) {
    let mut pr = SeRng::new(pa, pb);
    let c_to_o = (1.2 - pr.uniform(0.4)) * c_to_o;
    let met = (1.2 - pr.uniform(0.4)) * met;
    let u = pr.uniform(1.0);
    let (ty, mean, sig, lo, hi) = if u < PROB_GAS_GIANT {
        (3u8, 0.5, 0.5, -1.0, 1.12f32)
    } else if u < PROB_GAS_GIANT + PROB_ICE_GIANT {
        (2, -1.0, 0.5, 0.003_146_352f32.log10(), -0.5)
    } else {
        (1, -2.66, 1.0, -6.2, -1.2)
    };
    let mass_me = 10f64.powf(pr.tnormal(mean, sig, lo, hi) as f64) * JUPITER_ME;
    let cold = pr.uniform(1.0) < PROB_COLD_TERRA;
    let (comp, class) = climate::procedural_composition(&mut pr, ty, mass_me as f32, t_eq, cold, c_to_o, met);
    let radius_km = climate::procedural_radius_km(&comp, mass_me as f32, t_eq) as f64;
    (mass_me, radius_km, comp, class)
}

/// The planets and belts of a single star; `seed` stands in for SE's system seed.
pub(super) fn layout(s: &SeStar, seed: u32) -> SeSystem {
    // FUN_1404f9210: the star's own stream.
    let mut sr = SeRng::new(seed.wrapping_mul(0x2e3).wrapping_add(0x1486), seed.wrapping_mul(0x234).wrapping_add(0x1cdb));
    let slots = sr.int(10) + 5;
    // 100-199 comets, none around brown dwarfs (FUN_1404f9210 +0x1d40).
    let n_comets = if s.brown_dwarf() { 0 } else { sr.int(100) + 100 };
    let soi = sr.tnormal(40.0, 8.0, 10.0, 1000.0) as f64 * s.mass_sol;
    // SE caps it by the star's surroundings (10-1000 AU); a field star gets the 1000.
    let soi = soi.min(1000.0);
    let mut budget = MAX_ASTEROIDS;
    let mut sys = place(s, 0, slots, soi, &|hot| inner_limit_au(s, hot), seed, 0, &mut budget);
    sys.comets = super::se_comets::comets(s, seed, n_comets, soi);
    sys
}

/// FUN_140500700 for one host (a star or a barycentre) at SE's object index `idx`: `slots`
/// planet slots out to `soi` AU, the inner edge from `inner(hot)` (AU), planets numbered from
/// `n0` across the system (their seeds) and asteroids drawn from the system's `budget`.
#[allow(clippy::too_many_arguments)]
pub(super) fn place(s: &SeStar, idx: u32, slots: u32, soi: f64, inner: &dyn Fn(bool) -> f64, seed: u32, n0: u32, budget: &mut u32) -> SeSystem {
    let mut r = SeRng::new(seed.wrapping_mul(0x256).wrapping_add(idx.wrapping_mul(0x32d)), seed.wrapping_mul(0x570).wrapping_add(idx.wrapping_mul(0x162)));
    let m_e = s.mass_earth();
    let disk_me = if s.brown_dwarf() {
        (m_e * 1e-4).min(5.0)
    } else if s.lum_code == 11 && (s.letter == 12 || s.letter == 13) {
        r.tnormal(0.01, 0.005, 0.001, 0.02) as f64
    } else {
        m_e * 0.012
    };
    let disk = (disk_me / JUPITER_ME).log10() as f32;
    let hot = s.lum_code <= 5 || s.lum_code == 9 || ua(&mut r) <= PROB_HOT;
    let inner = inner(hot);
    let mut a = (ua(&mut r) * 0.1 + 1.0) * inner;

    let mut planets: Vec<SePlanet> = Vec::new();
    let (mut prev_inc, mut prev_apo) = (0.0f32, 0.0f64);
    let mut used = 0;
    while used < slots {
        if soi < a {
            break;
        }
        let u1 = ua(&mut r);
        let eccentric = u1 < 0.07;
        if prev_inc as f64 > PI / 4.0 {
            a *= ua(&mut r) * 0.3 + 2.2;
        }
        let u2 = ua(&mut r);
        let ecc = if eccentric {
            a *= u2 * 0.5 + 1.7;
            gauss_d(&mut r, 0.2, 0.1).abs()
        } else {
            a *= u2 * 1.5 + 1.2;
            gauss_d(&mut r, 0.0, 0.06).abs()
        };
        let mut sma = a;
        if a < inner {
            sma = inner / (1.0 - ecc);
        }
        // SE reads the previous apoapsis back from the orbit in parsecs, so this guard against
        // crossing orbits compares AU with parsecs and almost never acts.
        if sma < prev_apo * 1.01 {
            sma = prev_apo * 1.01 / (1.0 - ecc);
        }
        let period = (sma * sma * sma / s.mass_sol).sqrt() * 365.242_189_85;
        let ma = ua(&mut r) * TAU;
        let argp = ua(&mut r) * TAU;
        let mut node = ua(&mut r) * TAU - PI;
        let u3 = ua(&mut r);
        let sigma = (period / (sma * 4.848_136_811_095_36e-6 * TAU)).sqrt() * 0.0005;
        // SE adds 70-140 degrees for the eccentric slots of stars in dense surroundings
        // (generator +0xa4 above 0.5); a field star has none.
        let mut inc = (gauss_d(&mut r, 0.0, sigma) + 1.5 - u3 * 3.0) * DEG;
        let t_eq = climate::equilibrium_temps((s.lum_sol / (sma * sma)) as f32, 0.0, None).0 as f64;
        if t_eq > 3200.0 {
            continue;
        }

        // FUN_140501f60 on the planet's own stream.
        let n = n0 + planets.len() as u32;
        let pa = idx.wrapping_mul(0x5b71_04ac).wrapping_add(0xcdb6_e3e3).wrapping_sub(n.wrapping_mul(0x3249_1c1d)) ^ seed;
        let pb = (n + 1).wrapping_mul(0x18f4_b843).wrapping_sub(idx.wrapping_mul(0x4068_05d4)) ^ seed;
        let mut pr = SeRng::new(pa, pb);
        let c_to_o = (1.2 - pr.uniform(0.4)) * s.c_to_o;
        let met = (1.2 - pr.uniform(0.4)) * s.met;
        let u = pr.uniform(1.0);
        let (ty, mean, sig, lo, hi) = if u < PROB_GAS_GIANT {
            (3u8, 0.5, 0.5, -1.0, 1.12f32)
        } else if u < PROB_GAS_GIANT + PROB_ICE_GIANT {
            (2, -1.0, 0.5, 0.003_146_352f32.log10(), -0.5)
        } else {
            (1, -2.66, 1.0, -6.2, -1.2)
        };
        let mass_me = 10f64.powf(pr.tnormal(mean, sig, lo, hi.min(disk)) as f64) * JUPITER_ME;
        let cold = pr.uniform(1.0) < PROB_COLD_TERRA;
        let (comp, class) = climate::procedural_composition(&mut pr, ty, mass_me as f32, t_eq as f32, cold, c_to_o, met);
        let radius_km = climate::procedural_radius_km(&comp, mass_me as f32, t_eq as f32) as f64;
        let dwarf = radius_km <= 2000.0;
        let mut asteroids = 0;
        if dwarf {
            let k = ((sma * sma * sma).min(10.0) as u32 * 1000) / 3;
            asteroids = (pr.int(k) + 100).min(*budget);
            *budget -= asteroids;
        }
        prev_inc = inc as f32;
        prev_apo = sma * (1.0 + ecc) * 4.848_136_811_095_36e-6;

        // Moons, back on the layout stream (gas share of the composition: H + He).
        let gas = comp[0] + comp[1];
        let mut moons = if gas > 0.25 {
            count(r.gauss(60.0, 20.0), 5.0, 100.0)
        } else if gas > 0.0 && gas < 0.25 {
            count(r.gauss(20.0, 8.0), 0.0, 40.0)
        } else if !dwarf {
            count(r.gauss(10.0, 8.0), 0.0, 20.0)
        } else {
            let m = count(r.gauss(3.0, 2.0), 0.0, 10.0);
            let k = sma.sqrt().clamp(0.0, 1.0);
            inc += r.gauss(0.0, 8.0) as f64 * DEG * k;
            node += r.gauss(0.0, 2.5) as f64 * DEG * k;
            m
        };
        if s.brown_dwarf() {
            moons /= 3;
        }
        let mut debris = 0;
        if r.uniform(1.0) < PROB_RINGS_DEBRIS {
            let x = (r.gauss(300.0, 100.0) * (mass_me / JUPITER_ME).sqrt() as f32) as i32;
            if x > 10 {
                debris = x.min(1000) as u32;
                moons += debris;
            }
        }
        if moons > 0 {
            moons += 1;
        }
        planets.push(SePlanet {
            seeds: (pa, pb),
            n,
            a_au: sma,
            ecc,
            inc_deg: inc / DEG,
            node_deg: node / DEG,
            argp_deg: argp / DEG,
            ma_deg: ma / DEG,
            mass_me,
            radius_km,
            comp,
            class,
            dwarf,
            moons,
            asteroids,
            debris,
        });
        used += 1;
    }

    let mut asteroids = Vec::new();
    for i in 0..planets.len() {
        if planets[i].asteroids > 0 {
            belt(s, seed, i, n0 + i as u32, &planets, &mut asteroids);
        }
    }
    SeSystem { planets, asteroids, comets: Vec::new() }
}

/// The asteroids of dwarf planet `i` (FUN_1404fd2a0): between its neighbours' orbits (an
/// apoapsis plus 7 Hill radii inside, a periapsis less 2.8 outside; its own periapsis/apoapsis
/// where it has no neighbour), semi-major axes Gaussian about the dwarf planet's.
fn belt(s: &SeStar, seed: u32, i: usize, n: u32, planets: &[SePlanet], out: &mut Vec<SeAsteroid>) {
    let dp = &planets[i];
    let mut r = SeRng::new(seed.wrapping_mul(0x222).wrapping_add(n.wrapping_mul(0xb60)), seed.wrapping_mul(0x1ce).wrapping_add(n.wrapping_mul(0x676)));
    let m_e = s.mass_earth();
    let hill = |p: &SePlanet| (p.mass_me / (m_e * 3.0)).cbrt();
    let inner = match i.checked_sub(1).map(|j| &planets[j]) {
        Some(p) => (p.ecc + 1.0 + hill(p) * 7.0) * p.a_au,
        None => dp.a_au * (1.0 - dp.ecc),
    };
    let outer = match planets.get(i + 1) {
        Some(p) => (1.0 - p.ecc - hill(p) * 2.8) * p.a_au,
        None => dp.a_au * (1.0 + dp.ecc),
    };
    for _ in 0..dp.asteroids {
        let ecc = r.tnormal(0.1, 0.05, 0.0, 0.3) as f64;
        let (mut lo, mut hi) = (inner / (1.0 - ecc), outer / (ecc + 1.0));
        if hi < lo {
            std::mem::swap(&mut lo, &mut hi);
        }
        let x = gauss_d(&mut r, dp.a_au, (hi - lo) / 3.0);
        let a = x.clamp(lo, hi);
        let period = (a * a * a / s.mass_sol).sqrt() * 365.242_189_85;
        let ma = ua(&mut r) * TAU - PI;
        let argp = ua(&mut r) * TAU - PI;
        let node = ua(&mut r) * TAU - PI;
        let u = ua(&mut r);
        let inc = gauss_d(&mut r, 0.0, period / (a * 4.848_136_811_095_36e-6 * TAU) * 2e-7) + (2.5 - u * 5.0);
        out.push(SeAsteroid { a_au: a, ecc, inc_deg: inc, node_deg: node / DEG, argp_deg: argp / DEG, ma_deg: ma / DEG });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun() -> SeStar {
        SeStar { mass_sol: 1.0, radius_km: 695_700.0, teff: 5772.0, lum_sol: 1.0, lum_code: 7, letter: 4, c_to_o: 0.55, met: 1.0 }
    }

    #[test]
    #[ignore]
    fn print_sample() {
        for seed in 0..4u32 {
            let sys = layout(&sun(), seed * 7919 + 1);
            eprintln!("seed {seed}: {} planets, {} asteroids", sys.planets.len(), sys.asteroids.len());
            for p in &sys.planets {
                eprintln!(
                    "  a {:8.3} e {:.3} i {:6.2} M {:10.4} R {:8.0} {:9} dwarf {} moons {} ast {}",
                    p.a_au, p.ecc, p.inc_deg, p.mass_me, p.radius_km, crate::surface_gen::BULK_NAMES[p.class as usize], p.dwarf, p.moons, p.asteroids
                );
            }
        }
    }
}
