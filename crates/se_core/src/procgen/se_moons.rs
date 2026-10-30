//! SE's moon masses (FUN_140501990) and binary planets (SE's moon generator, FUN_1404fdc90).
//!
//! - **Moon masses**: a moon of SE's type Moon around a planet of more than 7 Earth masses weighs
//!   `M/M_J 10^N(-1.7, 1)` Earth masses times the zone factor below; with ProbMassiveMoonGasGiant
//!   (0.05) `10 M/M_J 10^N(-1.7, 1)` instead (a very massive moon), and with ProbBinaryGasGiant
//!   (0.001) `tn(0.2, 0.3, 0.01, 1) M` (a binary planet). Around smaller planets it weighs
//!   `0.3 M 10^tn(-1.7, 1, -5.2, -0.5)` times the zone factor. SE draws a ProbBinaryTerra (0.008) or
//!   ProbMassiveMoonTerra (0.1) mass for planets up to 6 Earth masses (1 for dry icy ones) but
//!   then overwrites it, so those two only cost a draw. Small moons (SE's dwarf moons) weigh
//!   `10^tn(-7.5, 2.5, -11, -5.2)` Earth masses.
//! - **Zone factor**: `max(exp(-0.01 ((a/R - N(20, 2)) 1.5644 U)^2), 1e-5)`, with the Gaussian and
//!   `U` drawn once per planet: moons near about 20 planet radii are the heavy ones.
//! - **Binary planets**: when the heaviest moon has at least 1/50 of its planet's mass the two
//!   orbit their barycentre, which takes the planet's place about the star with their total mass.
//!   Up to a mass ratio of 20 the barycentre takes the planet's name and the pair are `a` and `b`;
//!   beyond, the moon is `<planet>.1` and the barycentre `<planet>-<n>.1` (SE's place "An Island
//!   Terra": body `... 6`, parent `... 6-6.1`). With ProbBinaryPlanetMoons (0.4) the other moons
//!   whose periapsis is beyond three times the pair's separation stay, orbiting the barycentre;
//!   the rest are removed. Each component's rings end before `2.8` Hill radii of the other.

use crate::surface_gen::SeRng;

/// main-def.cfg `SolarSystemGenerator` (exe globals 0x14075f908 .. 0x14075f8f8).
const PROB_MASSIVE_MOON_GAS_GIANT: f32 = 0.05;
const PROB_MASSIVE_MOON_TERRA: f32 = 0.1;
const PROB_BINARY_GAS_GIANT: f32 = 0.001;
const PROB_BINARY_TERRA: f32 = 0.008;
pub(super) const PROB_BINARY_PLANET_MOONS: f32 = 0.4;
const JUPITER_ME: f32 = 317.828_4;

/// The per-planet draws behind the zone factor: where moons are heaviest (planet radii) and the
/// zone's width scale.
#[derive(Debug, Clone, Copy)]
pub(super) struct MoonZone {
    peak_radii: f32,
    width: f32,
}

pub(super) fn zone(r: &mut SeRng) -> MoonZone {
    let width = r.uniform(1.0);
    MoonZone { peak_radii: r.gauss(20.0, 2.0), width }
}

/// The mass factor of a moon `a_radii` planet radii out (the moon generator's argument to
/// FUN_140501990).
pub(super) fn zone_factor(z: &MoonZone, a_radii: f32) -> f32 {
    let x = ((a_radii - z.peak_radii) * (z.width * 1.5644)) as f64;
    ((x * x * -0.01).exp() as f32).max(1e-5)
}

/// SE's mass (Earth masses) of a Moon-type moon of a planet of `planet_me` Earth masses. `icy_dry`:
/// the planet has no hydrogen or helium and at least 10% ices.
pub(super) fn moon_mass(r: &mut SeRng, planet_me: f32, icy_dry: bool, factor: f32) -> f32 {
    let u = r.uniform(1.0);
    if planet_me > 7.0 {
        if u <= PROB_BINARY_GAS_GIANT {
            return r.tnormal(0.2, 0.3, 0.01, 1.0) * planet_me;
        }
        let x = 10f32.powf(r.gauss(-1.7, 1.0));
        return if u <= PROB_MASSIVE_MOON_GAS_GIANT { planet_me * 10.0 / JUPITER_ME * x } else { planet_me / JUPITER_ME * factor * x };
    }
    if planet_me <= if icy_dry { 1.0 } else { 6.0 } {
        // Drawn and then overwritten by SE.
        if u <= PROB_BINARY_TERRA {
            r.tnormal(0.5, 0.3, 0.01, 1.0);
        } else if u <= PROB_MASSIVE_MOON_TERRA {
            r.tnormal(0.02, 0.01, 0.001, 0.1);
        }
    }
    10f32.powf(r.tnormal(-1.7, 1.0, -5.2, -0.5)) * planet_me * 0.3 * factor
}

/// SE's mass (Earth masses) of a dwarf moon.
pub(super) fn small_moon_mass(r: &mut SeRng) -> f32 {
    10f32.powf(r.tnormal(-7.5, 2.5, -11.0, -5.2))
}

/// Whether a planet and its heaviest moon are a binary planet: `Some(true)` up to a mass ratio of
/// 20 (an `a`/`b` pair), `Some(false)` up to 50.
pub(super) fn binary(planet_me: f64, moon_me: f64) -> Option<bool> {
    let ratio = planet_me / moon_me;
    (ratio <= 50.0).then_some(ratio <= 20.0)
}

/// Outer edge (km) for the rings of a binary component of `self_me` Earth masses: the other's Hill
/// radius times 2.8 short of the periapsis separation.
pub(super) fn ring_limit_km(sep_km: f64, ecc: f64, self_me: f64, other_me: f64) -> f64 {
    (1.0 - ecc - (other_me / (self_me * 3.0)).cbrt() * 2.8) * sep_km
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_peaks_near_twenty_radii() {
        let z = MoonZone { peak_radii: 20.0, width: 1.0 };
        assert!((zone_factor(&z, 20.0) - 1.0).abs() < 1e-6);
        assert!(zone_factor(&z, 40.0) < 0.01);
        assert_eq!(zone_factor(&z, 400.0), 1e-5);
    }

    #[test]
    fn masses_and_binaries() {
        // Moons of an Earth at the zone's peak: a few percent weigh 1/50 of it (binary planets).
        let (mut bin, mut close, n) = (0, 0, 20_000u32);
        for i in 0..n {
            let mut r = SeRng::new(i.wrapping_mul(0x2545_f491), i ^ 0x9e37);
            let m = moon_mass(&mut r, 1.0, false, 1.0);
            assert!(m > 0.0 && m <= 0.3 * 10f32.powf(-0.5) + 1e-6, "{m}");
            if let Some(c) = binary(1.0, m as f64) {
                bin += 1;
                close += c as usize;
            }
        }
        let f = bin as f64 / n as f64;
        assert!((0.15..0.35).contains(&f), "binary share {f}");
        assert!(close > 0 && close < bin);
        // Jupiter: Galilean-sized moons at the peak, 5% very massive, 0.1% binary.
        let (mut massive, mut jbin) = (0, 0);
        for i in 0..n {
            let mut r = SeRng::new(i.wrapping_mul(0x2545_f491), i ^ 0x9e37);
            let m = moon_mass(&mut r, 317.8, false, 1.0);
            massive += (m > 3.0) as usize;
            jbin += binary(317.8, m as f64).is_some() as usize;
        }
        assert!((0.005..0.05).contains(&(massive as f64 / n as f64)), "massive {massive}");
        assert!(jbin > 0 && jbin < n as usize / 50, "giant binaries {jbin}");
        assert!(ring_limit_km(1e5, 0.0, 1.0, 0.1) > 0.0 && ring_limit_km(1e5, 0.0, 0.1, 1.0) < 0.0);
    }
}
