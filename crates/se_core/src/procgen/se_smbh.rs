//! SE's stars about a supermassive black hole (seSolarSystem::CreateSupermassiveBlackHoleSystem,
//! FUN_1404fb950), which a black hole of 400 solar masses or more gets instead of planets
//! (FUN_1404f7c40, catalog and procedural alike).
//!
//! - **Count**: `int(3 log10 M)` stars, at most 50 (19 for Sgr A*'s 4.31e6 solar masses).
//! - **Orbits**: periapsis beyond the larger of the hole's radius + 1 AU and
//!   `0.0017085 cbrt(2 M / 14.536)` AU (M in Earth masses), apoapsis within 500 times that.
//!   Mean anomaly, argument of periapsis, node and inclination uniform in 0-360 degrees,
//!   `e = tn(0.5, 0.5, 0, 0.89)`, semi-major axis a whole number of AU above its least value;
//!   the period counts the star's mass.
//! - **Stars** (FUN_1401d6c60, as for cluster members): born together `tn(50, 20, 1, 200)` Myr
//!   ago; each draws `M_V = N(3, 4)`, a main-sequence star of that light, unless it has outlived
//!   its lifetime: then a giant of `M_V = N(1, 1)`, or, older still, a white dwarf of
//!   `M_V = N(14, 0.83)`. [Fe/H], C/O and age follow [`super::se_stars::group_star`].
//!
//! Assumed: the main-sequence temperature for a magnitude comes from the port's relation
//! ([`crate::star::main_sequence_abs_mag`]) and a giant's from the port's giants (3500-5200 K),
//! as SE's class tables (FUN_1401da080) are not decoded; SE's Wolf-Rayet and carbon-star
//! conversions are not applied; the orbit draws follow the hole's seeds, their place in SE's
//! stream is not replicated.

use super::se_stars::{self, StarDraw};
use crate::star::{self, StarKind};
use crate::surface_gen::SeRng;

const EARTH_SUN: f64 = 3.003_489_382_535_917_7e-6;
const DEG_PER_TURN: f64 = 360.0;

#[derive(Debug, Clone)]
pub(super) struct SmbhStar {
    pub class: String,
    pub mass_sol: f64,
    pub lum_sol: f64,
    pub teff: f64,
    pub radius_km: f64,
    pub draw: StarDraw,
    pub a_au: f64,
    pub ecc: f64,
    pub inc_deg: f64,
    pub node_deg: f64,
    pub argp_deg: f64,
    pub ma_deg: f64,
    pub period_days: f64,
}

/// Whether SE gives the star this generator.
pub(super) fn applies(kind: StarKind, mass_sol: f64) -> bool {
    kind == StarKind::BlackHole && mass_sol >= 400.0
}

/// Number of stars about a hole of `mass_sol` solar masses.
pub(super) fn star_count(mass_sol: f64) -> u32 {
    ((mass_sol.max(1.0).log10() * 3.0) as i32).clamp(0, 50) as u32
}

/// The stars about a hole of `mass_sol` solar masses and `radius_km`; `seed` stands in for SE's
/// system seed.
pub(super) fn stars(mass_sol: f64, radius_km: f64, seed: u32) -> Vec<SmbhStar> {
    let m_e = mass_sol / EARTH_SUN;
    let inner = (radius_km * 3.240_779_289_444_365e-14 / 4.848_136_811_095_36e-6 + 1.0).max((2.0 * m_e / 14.536).cbrt() * 0.001_708_513_6);
    let outer = inner * 500.0;
    let mut r = SeRng::new(seed.wrapping_mul(0x3917).wrapping_add(0xcdf5f), seed.wrapping_mul(0xad45).wrapping_add(0x856f2));
    let age = r.tnormal(50.0, 20.0, 1.0, 200.0) as f64 * 0.001;
    (0..star_count(mass_sol))
        .map(|k| {
            let ma_deg = r.uniform(1.0) as f64 * DEG_PER_TURN;
            let argp_deg = r.uniform(1.0) as f64 * DEG_PER_TURN;
            let node_deg = r.uniform(1.0) as f64 * DEG_PER_TURN;
            let inc_deg = r.uniform(1.0) as f64 * DEG_PER_TURN;
            let ecc = r.tnormal(0.5, 0.5, 0.0, 0.89) as f64;
            let a_min = inner / (1.0 - ecc);
            let a_au = a_min + r.int((outer / (1.0 + ecc) - a_min).max(0.0) as u32) as f64;
            let mut sr = SeRng::new(seed.wrapping_mul(0x71_e981).wrapping_add(k.wrapping_mul(0x7a_ee1d)), seed.wrapping_mul(0x45_de81).wrapping_add(k.wrapping_mul(0x15_23da)));
            let mut s = group_member(&mut sr, age);
            s.period_days = (a_au.powi(3) * 1.000_003 / (s.mass_sol + mass_sol)).sqrt() * 365.242_189_85;
            SmbhStar { a_au, ecc, inc_deg, node_deg, argp_deg, ma_deg, ..s }
        })
        .collect()
}

/// Main-sequence temperature of absolute magnitude `mv` (inverting the port's relation).
fn ms_teff(mv: f64) -> f64 {
    let (mut lo, mut hi) = (2300.0f64, 50_000.0f64);
    for _ in 0..40 {
        let mid = (lo * hi).sqrt();
        if star::main_sequence_abs_mag(mid) > mv {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo * hi).sqrt()
}

fn letter_of(teff: f64) -> u32 {
    [30000.0, 10000.0, 7500.0, 6000.0, 5200.0, 3700.0, 2400.0, 1300.0, 550.0].iter().take_while(|&&lim| teff < lim).count() as u32
}

fn lum_bol(mv: f64, teff: f64) -> f64 {
    star::lum_from_abs_mag(mv) * 10f64.powf(-0.4 * star::bolometric_correction(teff))
}

/// One star of a group of age `age_gyr` (FUN_1401d6c60).
fn group_member(r: &mut SeRng, age_gyr: f64) -> SmbhStar {
    let draw = se_stars::group_star(r, age_gyr);
    let lifetime = |m: f64, lum_code: u32| crate::climate::star_lifetime_gyr(m as f32, lum_code) as f64;
    let blank = |class: String, mass_sol, lum_sol, teff, radius_km| SmbhStar {
        class,
        mass_sol,
        lum_sol,
        teff,
        radius_km,
        draw,
        a_au: 0.0,
        ecc: 0.0,
        inc_deg: 0.0,
        node_deg: 0.0,
        argp_deg: 0.0,
        ma_deg: 0.0,
        period_days: 0.0,
    };
    let mv = r.gauss(3.0, 4.0) as f64;
    let t = ms_teff(mv);
    let l = lum_bol(mv, t);
    let m = se_stars::mass_from_lum(l, 7, letter_of(t));
    if lifetime(m, 7) > draw.age_gyr {
        let r_km = star::radius_from_lum_teff(l, t) * crate::units::SOLAR_RADIUS_KM;
        return blank(format!("{}V", crate::star_octree::ms_class_name(t)), m, l, t, r_km);
    }
    let mv = r.gauss(1.0, 1.0) as f64;
    let t = 3500.0 + r.uniform(1700.0) as f64;
    let l = lum_bol(mv, t);
    let m = se_stars::mass_from_lum(l, 5, letter_of(t));
    if lifetime(m, 5) > draw.age_gyr {
        let letter = if t < 3900.0 { "M" } else if t < 5200.0 { "K" } else { "G" };
        let r_km = star::radius_from_lum_teff(l, t) * crate::units::SOLAR_RADIUS_KM;
        return blank(format!("{letter}{} III", r.int(10)), m, l, t, r_km);
    }
    let mv = r.gauss(14.0, 0.83) as f64;
    let m = se_stars::white_dwarf_mass(se_stars::mass_from_lum(lum_bol(mv, 6000.0), 7, 4));
    let t = 5000.0 + (r.uniform(1.0) as f64).powi(2) * 25_000.0;
    let r_km = se_stars::white_dwarf_radius_sol(m) * crate::units::SOLAR_RADIUS_KM;
    let l = (r_km / crate::units::SOLAR_RADIUS_KM).powi(2) * (t / 5772.0).powi(4);
    blank(format!("DA{:.0}", (50_400.0 / t).clamp(1.0, 9.0)), m, l, t, r_km)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_a_star_gets_19_stars() {
        assert_eq!(star_count(4.31e6), 19);
        assert_eq!(star_count(400.0), 7);
        assert_eq!(star_count(1e30), 50);
        let s = stars(4.31e6, 1.27e7, 12345);
        assert_eq!(s.len(), 19);
        for x in &s {
            assert!(x.a_au * (1.0 - x.ecc) >= 9.9 && x.a_au * (1.0 + x.ecc) <= 4980.0, "{} {}", x.a_au, x.ecc);
            assert!(x.period_days > 0.0 && x.mass_sol > 0.0 && x.radius_km > 0.0);
            assert!(x.draw.age_gyr < 0.25);
        }
        // Young: mostly main-sequence stars (M_V above about 0 outlives 50 Myr).
        let all: Vec<SmbhStar> = (0..20u32).flat_map(|k| stars(4.31e6, 1.27e7, k * 7919 + 3)).collect();
        let ms = all.iter().filter(|x| x.class.ends_with('V')).count() as f64 / all.len() as f64;
        assert!((0.6..0.9).contains(&ms), "main-sequence share {ms}");
    }
}
