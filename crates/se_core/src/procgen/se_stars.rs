//! SE's physical parameters of generated stars: mass from luminosity (FUN_1401dd9f0), remnant
//! radii (FUN_1401d5fe0), and the age, metallicity and C/O its star generator draws
//! (FUN_1401d63b0 for field stars, FUN_1401d6c60 for cluster members and the stars about a
//! supermassive black hole). The engine feeds these to the planet generator ([`super::StarParams`]).

use crate::surface_gen::SeRng;

/// SE's cap on ages and lifetimes (Gyr) and its floor on ages.
pub const MAX_AGE_GYR: f64 = 12.87;
const MIN_AGE_GYR: f64 = 1e-4;

/// Mass (solar) of a star of bolometric luminosity `l_bol` (solar), luminosity code `lum_code`
/// ([`crate::climate::star_class_bits`]) and spectral type `letter` (O = 0 .. M = 6, L 7, T 8,
/// Y 9), FUN_1401dd9f0 for stars that are not remnants: the inverse of SE's mass-luminosity law,
/// for luminosity codes below 9 (class bits below 0x4800) clamped to 0.0767-180 for O-M,
/// 0.012-0.0767 for L and T, 0.003-0.03 for Y.
pub fn mass_from_lum(l_bol: f64, lum_code: u32, letter: u32) -> f64 {
    let l = l_bol.max(1e-12);
    let m = if l > 36_109.844 {
        l / 1800.0
    } else if l > 16.0 {
        l.powf(0.285_714_3) - 0.208_271_3 + l * 5.767_715e-6
    } else if l > 0.031_490_8 {
        l.powf(0.25)
    } else if l > 0.001_152_73 {
        (l / 0.23).powf(0.434_782_6)
    } else {
        (l / 1237.633).powf(0.165_813_7)
    };
    match letter {
        _ if lum_code >= 9 => m,
        0..=6 => m.clamp(0.0767, 180.0),
        7 | 8 => m.clamp(0.012, 0.0767),
        _ => m.clamp(0.003, 0.03),
    }
}

/// A solitary remnant's mass (solar), as SE draws it (FUN_1401d63b0, then FUN_1401d5fe0): white
/// dwarf `tn(1, 0.5, 0.56, 1.44)`, neutron star `tn(2.22, 0.39, 1.44, 3)`, black hole
/// `tn(8, 3, 3, 100)`; `None` for other stars.
pub fn remnant_mass(r: &mut SeRng, kind: crate::star::StarKind) -> Option<f64> {
    use crate::star::StarKind;
    let (mean, sigma, lo, hi) = match kind {
        StarKind::WhiteDwarf => (1.0, 0.5, 0.56, 1.44),
        StarKind::NeutronStar => (2.22, 0.39, 1.44, 3.0),
        StarKind::BlackHole => (8.0, 3.0, 3.0, 100.0),
        _ => return None,
    };
    Some(r.tnormal(mean, sigma, lo, hi) as f64)
}

/// A remnant's radius (km) for its mass (FUN_1401d5fe0); `None` for other stars.
pub fn remnant_radius_km(kind: crate::star::StarKind, mass_sol: f64) -> Option<f64> {
    use crate::star::StarKind;
    let r_sun = crate::units::SOLAR_RADIUS_KM;
    match kind {
        StarKind::WhiteDwarf => Some(white_dwarf_radius_sol(mass_sol) * r_sun),
        StarKind::NeutronStar => Some(neutron_star_radius_km(mass_sol)),
        StarKind::BlackHole => Some(black_hole_radius_sol(mass_sol) * r_sun),
        _ => None,
    }
}

/// A white dwarf's mass as SE reads it from its class (1.44 down to 0.56 Msun); one made from a
/// star of mass `m` (FUN_1401d5fe0) keeps `(m - 1.44) / -0.88` of that span.
pub fn white_dwarf_mass(m: f64) -> f64 {
    1.44 - ((m - 1.44) / -0.88).clamp(0.0, 1.0) * 0.88
}

/// White dwarf radius (solar radii), FUN_1401d5fe0: `0.0125 M^-1/3`.
pub fn white_dwarf_radius_sol(mass_sol: f64) -> f64 {
    0.0125 * mass_sol.max(1e-3).powf(-1.0 / 3.0)
}

/// Neutron star radius (km), FUN_1401d5fe0: `cbrt(1703.7 M)`.
pub fn neutron_star_radius_km(mass_sol: f64) -> f64 {
    (1703.7 * mass_sol).cbrt()
}

/// Black hole radius (solar radii), FUN_1401d5fe0: `4.245e-6 M` (2.953 km per solar mass).
pub fn black_hole_radius_sol(mass_sol: f64) -> f64 {
    4.245_005e-6 * mass_sol
}

/// What SE's star generator draws besides the star's class and light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarDraw {
    pub age_gyr: f64,
    /// [Fe/H] (dex).
    pub feh: f64,
    pub c_to_o: f64,
}

/// A field star's draws (FUN_1401d63b0, population 1). `x` is its distance from the galaxy's
/// centre in galaxy radii: [Fe/H] `N(0.1 - 0.4503 x, 0.07)` and C/O `N(1 - 0.6755 x, 0.1699)` for
/// x up to 1 (-0.3503 and 0.3245 beyond). The age is `U` times the star's lifetime (at most 12.87
/// Gyr), or for one star in ten `N(0, 0.01)` Gyr (a newborn), at least 1e-4 Gyr. SE then evolves a
/// star older than its lifetime into a giant or remnant; the engine's stars come with their class,
/// so a giant or remnant here keeps the age the draw gave its progenitor's lifetime.
pub fn field_star(r: &mut SeRng, x: f64, lifetime_gyr: f64) -> StarDraw {
    let x = x.abs();
    let (feh_mean, c_mean) = if x <= 1.0 { (0.1 - x * 0.450_331_6, 1.0 - x * 0.675_497_4) } else { (-0.350_331_6, 0.324_502_6) };
    let feh = r.gauss(feh_mean as f32, 0.07) as f64;
    let c_to_o = r.gauss(c_mean as f32, 0.169_864_4) as f64;
    let age = if r.uniform(1.0) < 0.1 {
        r.gauss(0.0, 0.01) as f64
    } else {
        r.uniform(lifetime_gyr.min(MAX_AGE_GYR) as f32) as f64
    };
    StarDraw { age_gyr: age.clamp(MIN_AGE_GYR, MAX_AGE_GYR), feh, c_to_o }
}

/// A remnant SE makes without a progenitor's draws (FUN_1401d5fe0 with them unset): metallicity
/// parameter `N(1.5, 0.25)`, C/O `N(1.5, 0.1699)` (so its planets come out dry and carbon-rich),
/// age `U` times 12.87 Gyr.
pub fn remnant_star(r: &mut SeRng) -> StarDraw {
    let feh = r.gauss(1.5, 0.25) as f64;
    let c_to_o = r.gauss(1.5, 0.169_864_4) as f64;
    let age = (r.uniform(MAX_AGE_GYR as f32) as f64).max(MIN_AGE_GYR);
    StarDraw { age_gyr: age, feh, c_to_o }
}

/// A star born with a group of age `age_gyr` (a cluster, or the stars about a supermassive black
/// hole; FUN_1401d6c60): [Fe/H] `N(-0.1252, 0.07)`, C/O `N(0.6623, 0.1699)`, age `+ N(0, 0.01)`.
pub fn group_star(r: &mut SeRng, age_gyr: f64) -> StarDraw {
    let feh = r.gauss(-0.125_165_8, 0.07) as f64;
    let c_to_o = r.gauss(0.662_251_3, 0.169_864_4) as f64;
    let age = (r.gauss(0.0, 0.01) as f64 + age_gyr).clamp(MIN_AGE_GYR, MAX_AGE_GYR);
    StarDraw { age_gyr: age, feh, c_to_o }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mass_law_inverts_ses_luminosities() {
        assert!((mass_from_lum(1.0, 7, 4) - 1.0).abs() < 1e-9);
        // L = 1.4 M^3.5 + 0.00303 (55 - M) at 10 Msun is 4428; the inverse gives ~ 10.
        let m = mass_from_lum(4428.0, 7, 1);
        assert!((m - 10.0).abs() < 1.0, "{m}");
        // Red giants of 60 Lsun come out about 3 Msun, as in SE.
        assert!((mass_from_lum(60.0, 5, 5) - 3.0).abs() < 0.2);
        assert_eq!(mass_from_lum(1e-6, 7, 6), 0.0767);
        // White dwarfs and Wolf-Rayet stars (codes 9, 10) are not clamped.
        assert!(mass_from_lum(1e-6, 9, 6) < 0.05);
        assert_eq!(mass_from_lum(1e-9, 7, 8), 0.012);
        assert!((mass_from_lum(1e-6, 7, 8) - 0.0311).abs() < 1e-3);
    }

    #[test]
    fn remnants() {
        assert!((white_dwarf_mass(0.0767) - 0.56).abs() < 1e-9);
        assert!((white_dwarf_mass(1.0) - 1.0).abs() < 1e-9);
        assert!((white_dwarf_radius_sol(0.6) * 695_700.0 - 10_300.0).abs() < 100.0);
        assert!((neutron_star_radius_km(1.4) - 13.36).abs() < 0.05);
        assert!((black_hole_radius_sol(1.0) * 695_700.0 - 2.953).abs() < 0.01);
    }

    #[test]
    fn field_draws() {
        let (mut young, mut feh, n) = (0, 0.0, 4000);
        for i in 0..n {
            let mut r = SeRng::new(i * 7 + 1, i * 13 + 5);
            let d = field_star(&mut r, 0.667, 10.0);
            assert!((MIN_AGE_GYR..=MAX_AGE_GYR).contains(&d.age_gyr));
            young += (d.age_gyr < 0.05) as usize;
            feh += d.feh;
        }
        let fy = young as f64 / n as f64;
        assert!((0.08..0.14).contains(&fy), "young {fy}");
        // The solar neighbourhood (2/3 of the galaxy's radius): [Fe/H] about -0.2, C/O 0.55.
        assert!((feh / n as f64 + 0.2).abs() < 0.01);
        let g = group_star(&mut SeRng::new(3, 4), 0.05);
        assert!((g.age_gyr - 0.05).abs() < 0.05 && g.feh < 0.2);
    }
}
