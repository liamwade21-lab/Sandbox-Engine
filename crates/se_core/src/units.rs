//! Physical constants and unit conversions used by catalogs.
//!
//! Catalog conventions (as used by SpaceEngine): distances of stars and galaxies in
//! parsecs, galaxy radii in parsecs, planet radii in km, semi-major axes in AU,
//! periods in years, masses in Earth or Solar masses, RA in hours, Dec in degrees.

pub const METERS_PER_KM: f64 = 1_000.0;
pub const METERS_PER_AU: f64 = 1.495_978_707e11;
pub const METERS_PER_LY: f64 = 9.460_730_472_580_8e15;
pub const METERS_PER_PC: f64 = 3.085_677_581_491_367e16;
pub const SECONDS_PER_DAY: f64 = 86_400.0;
pub const DAYS_PER_JULIAN_YEAR: f64 = 365.25;
pub const SECONDS_PER_YEAR: f64 = SECONDS_PER_DAY * DAYS_PER_JULIAN_YEAR;
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;
pub const G: f64 = 6.674_30e-11;

pub const SOLAR_MASS_KG: f64 = 1.988_47e30;
pub const EARTH_MASS_KG: f64 = 5.972_19e24;
pub const JUPITER_MASS_KG: f64 = 1.898_13e27;
pub const SOLAR_RADIUS_KM: f64 = 695_700.0;
pub const EARTH_RADIUS_KM: f64 = 6_378.14;
pub const JUPITER_RADIUS_KM: f64 = 71_492.0;
pub const SOLAR_LUMINOSITY_W: f64 = 3.828e26;
pub const SOLAR_TEFF: f64 = 5_778.0;
/// Absolute visual magnitude of the Sun.
pub const SOLAR_ABS_MAG_V: f64 = 4.83;
/// Absolute bolometric magnitude of the Sun (IAU 2015 B2).
pub const SOLAR_ABS_MAG_BOL: f64 = 4.74;
pub const STEFAN_BOLTZMANN: f64 = 5.670_374_419e-8;

/// Human readable distance in the units SE's HUD uses (m, km, AU, ly, kpc, Mpc...).
pub fn format_distance(meters: f64) -> String {
    let m = meters.abs();
    if m < 1e3 {
        format!("{meters:.1} m")
    } else if m < 1e7 {
        format!("{:.3} km", meters / METERS_PER_KM)
    } else if m < 0.1 * METERS_PER_AU {
        format!("{:.0} km", meters / METERS_PER_KM)
    } else if m < 0.1 * METERS_PER_LY {
        format!("{:.4} AU", meters / METERS_PER_AU)
    } else if m < 1e3 * METERS_PER_LY {
        format!("{:.3} ly", meters / METERS_PER_LY)
    } else if m < 1e6 * METERS_PER_PC {
        format!("{:.3} kpc", meters / (1e3 * METERS_PER_PC))
    } else if m < 1e9 * METERS_PER_PC {
        format!("{:.3} Mpc", meters / (1e6 * METERS_PER_PC))
    } else {
        format!("{:.3} Gpc", meters / (1e9 * METERS_PER_PC))
    }
}

/// Human readable speed (m/s, km/s, c, ly/s, pc/s...).
pub fn format_speed(mps: f64) -> String {
    let v = mps.abs();
    if v < 1e3 {
        format!("{mps:.1} m/s")
    } else if v < 0.01 * SPEED_OF_LIGHT {
        format!("{:.3} km/s", mps / 1e3)
    } else if v < 1e4 * SPEED_OF_LIGHT {
        format!("{:.3} c", mps / SPEED_OF_LIGHT)
    } else if v < 1e3 * METERS_PER_PC {
        format!("{:.3} ly/s", mps / METERS_PER_LY)
    } else if v < 1e6 * METERS_PER_PC {
        format!("{:.3} kpc/s", mps / (1e3 * METERS_PER_PC))
    } else {
        format!("{:.3} Mpc/s", mps / (1e6 * METERS_PER_PC))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(format_distance(12.0), "12.0 m");
        assert_eq!(format_distance(METERS_PER_AU), "1.0000 AU");
        assert_eq!(format_distance(4.24 * METERS_PER_LY), "4.240 ly");
        assert_eq!(format_speed(SPEED_OF_LIGHT), "1.000 c");
    }
}
