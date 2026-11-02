//! Body orientation models.
//!
//! Body-fixed frame (SpaceEngine's planet frame, verified against Earth's cube
//! map tiles): +X = prime meridian (longitude 0), +Y = north pole, +Z = longitude
//! 90 W (so -Z = 90 E). Relative to the IAU body frame (x = meridian, y = 90 E,
//! z = pole) this is `(x, y, z)_se = (x, z, -y)_iau`.

use crate::coords::{ecliptic_to_universe_quat, eq_to_universe_quat};
use crate::sc::Block;
use crate::time::J2000;
use glam::{DMat3, DQuat, DVec3};

/// Rotation from the SE body frame to IAU body axes (columns = SE axes in IAU).
fn body_to_iau() -> DQuat {
    DQuat::from_mat3(&DMat3::from_cols(DVec3::X, DVec3::Z, DVec3::NEG_Y))
}

/// Orientation (body frame -> universe frame) from the 3-1-3 Euler angles of the body's axes against
/// the ICRF (radians): the IAU model's 90 deg + RA, 90 deg - Dec and W, or DE's lunar librations.
pub fn from_icrf_euler(phi: f64, theta: f64, psi: f64) -> DQuat {
    let q = DQuat::from_rotation_z(phi) * DQuat::from_rotation_x(theta) * DQuat::from_rotation_z(psi);
    (eq_to_universe_quat() * q * body_to_iau()).normalize()
}

/// How SE turns a body in place of its rotation model (worked out by `Ephemeris::orientation`,
/// which knows the positions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RotationLock {
    #[default]
    None,
    /// `RotationModel "Moon-DE"`: the Moon's librations from the JPL ephemeris.
    MoonDe,
    /// `TidalLocked true`: longitude 0 faces the parent, the pole along the orbit's normal.
    Tidal,
}

impl RotationLock {
    pub fn from_body_block(b: &Block) -> Self {
        let model = b.get_last("RotationModel").and_then(|i| i.str().map(str::to_string));
        if model.is_some_and(|m| m.eq_ignore_ascii_case("Moon-DE")) {
            RotationLock::MoonDe
        } else if b.bool("TidalLocked") == Some(true) {
            RotationLock::Tidal
        } else {
            RotationLock::None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IauRotation {
    pub epoch: f64,
    pub pole_ra: f64,
    pub pole_ra_rate: f64,
    pub pole_dec: f64,
    pub pole_dec_rate: f64,
    pub prime_meridian: f64,
    pub rotation_rate: f64,
    /// (amplitude deg, phase deg, frequency deg/day, frequency rate deg/day^2) terms added to W.
    pub w_terms: Vec<[f64; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimpleRotation {
    /// Sidereal rotation period in hours (negative = retrograde). 0 = not rotating.
    pub period_hours: f64,
    pub obliquity: f64,
    pub eq_ascend_node: f64,
    pub offset: f64,
    pub precession_years: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RotationModel {
    Iau(IauRotation),
    Simple(SimpleRotation),
}

impl Default for RotationModel {
    fn default() -> Self {
        RotationModel::Simple(SimpleRotation {
            period_hours: 0.0,
            obliquity: 0.0,
            eq_ascend_node: 0.0,
            offset: 0.0,
            precession_years: 0.0,
        })
    }
}

impl RotationModel {
    /// Build from an object block (`RotationModel "IAU" {}` or the simple keys).
    pub fn from_body_block(b: &Block) -> Self {
        if let Some(item) = b.get_last("RotationModel") {
            if item.str().map(|s| s.eq_ignore_ascii_case("IAU")).unwrap_or(false) {
                if let Some(rb) = item.block() {
                    let mut w_terms = Vec::new();
                    if let Some(t) = rb.sub("PeriodicTermsDiurnal") {
                        let nums = t.anonymous_numbers();
                        // SE rows (SolarSys.sc): PoleRA amp, PoleDec amp, PrimeMer amp, phase, freq,
                        // freq_rate. Only the prime-meridian part is used.
                        for c in nums.chunks_exact(6) {
                            w_terms.push([c[2], c[3], c[4], c[5]]);
                        }
                    }
                    return RotationModel::Iau(IauRotation {
                        epoch: rb.f64("Epoch").unwrap_or(J2000),
                        pole_ra: rb.f64("PoleRA").unwrap_or(0.0),
                        pole_ra_rate: rb.f64("PoleRARate").unwrap_or(0.0),
                        pole_dec: rb.f64("PoleDec").unwrap_or(90.0),
                        pole_dec_rate: rb.f64("PoleDecRate").unwrap_or(0.0),
                        prime_meridian: rb.f64("PrimeMeridian").unwrap_or(0.0),
                        rotation_rate: rb.f64("RotationRate").unwrap_or(0.0),
                        w_terms,
                    });
                }
            }
        }
        RotationModel::Simple(SimpleRotation {
            period_hours: b.f64("RotationPeriod").unwrap_or(0.0),
            obliquity: b.f64("Obliquity").unwrap_or(0.0),
            eq_ascend_node: b.f64("EqAscendNode").unwrap_or(0.0),
            offset: b.f64("RotationOffset").unwrap_or(0.0),
            precession_years: b.f64("Precession").unwrap_or(0.0),
        })
    }

    /// Orientation (body frame -> universe frame) at Julian date `jd`.
    pub fn orientation(&self, jd: f64) -> DQuat {
        match self {
            RotationModel::Iau(r) => {
                let d = jd - r.epoch;
                let t = d / 36_525.0;
                let ra = (r.pole_ra + r.pole_ra_rate * t).to_radians();
                let dec = (r.pole_dec + r.pole_dec_rate * t).to_radians();
                let mut w = r.prime_meridian + r.rotation_rate * d;
                for term in &r.w_terms {
                    w += term[0] * (term[1] + term[2] * d + term[3] * d * d).to_radians().sin();
                }
                // Active rotation IAU body -> ICRF.
                let q_iau = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2 + ra)
                    * DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2 - dec)
                    * DQuat::from_rotation_z(w.rem_euclid(360.0).to_radians());
                (eq_to_universe_quat() * q_iau * body_to_iau()).normalize()
            }
            RotationModel::Simple(s) => {
                let days = jd - J2000;
                let spin = if s.period_hours != 0.0 {
                    s.offset + 360.0 * days * 24.0 / s.period_hours
                } else {
                    s.offset
                };
                // Pole tilted from the ecliptic normal by obliquity around the node line.
                let q_ecl = DQuat::from_rotation_z(s.eq_ascend_node.to_radians())
                    * DQuat::from_rotation_x(s.obliquity.to_radians())
                    * DQuat::from_rotation_z(spin.rem_euclid(360.0).to_radians());
                (ecliptic_to_universe_quat() * q_ecl * body_to_iau()).normalize()
            }
        }
    }

    /// Rotation period in seconds if the body rotates (absolute value).
    pub fn period_seconds(&self) -> Option<f64> {
        match self {
            RotationModel::Iau(r) if r.rotation_rate != 0.0 => Some(360.0 / r.rotation_rate.abs() * 86_400.0),
            RotationModel::Simple(s) if s.period_hours != 0.0 => Some(s.period_hours.abs() * 3600.0),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::radec_deg_to_universe;
    use crate::sc;

    #[test]
    fn iau_pole_matches_catalog() {
        let doc = sc::parse(
            "Planet \"Mars\" { RotationModel \"IAU\" { Epoch 2451545 PoleRA 317.68143 PoleDec 52.88650 PrimeMeridian 176.630 RotationRate 350.89198226 } }",
        );
        let m = RotationModel::from_body_block(doc.get("Planet").unwrap().block().unwrap());
        let q = m.orientation(J2000);
        let pole = q * DVec3::Y;
        let expect = radec_deg_to_universe(317.68143, 52.88650);
        assert!(pole.dot(expect) > 1.0 - 1e-12, "{pole:?} vs {expect:?}");
        // One sidereal day later the meridian is back where it was.
        let q2 = m.orientation(J2000 + 360.0 / 350.89198226);
        assert!((q * DVec3::X).dot(q2 * DVec3::X) > 1.0 - 1e-9);
        // Eastward rotation: after a quarter day the meridian moves towards -Z (90 E).
        let q3 = m.orientation(J2000 + 90.0 / 350.89198226);
        assert!((q3 * DVec3::X).dot(q * DVec3::NEG_Z) > 0.999);
    }

    #[test]
    fn diurnal_terms_read_six_column_rows() {
        // Mercury's rows in SolarSys.sc.
        let doc = sc::parse(
            "Planet \"Mercury\" { RotationModel \"IAU\" { Epoch 2451545 PrimeMeridian 329.5469 RotationRate 6.1385025\n\
             PeriodicTermsDiurnal {\n\
             0, 0, +0.00993822, 174.791086,  4.092335, 0,\n\
             0, 0, -0.00104581, 349.582171,  8.184670, 0,\n\
             } } }",
        );
        let m = RotationModel::from_body_block(doc.get("Planet").unwrap().block().unwrap());
        let RotationModel::Iau(r) = &m else { panic!() };
        assert_eq!(r.w_terms, vec![[0.00993822, 174.791086, 4.092335, 0.0], [-0.00104581, 349.582171, 8.18467, 0.0]]);
        // The terms move the meridian by hundredths of a degree, not hundreds.
        let plain = RotationModel::Iau(IauRotation { w_terms: Vec::new(), ..r.clone() });
        let jd = 2_460_000.5;
        let cos = (m.orientation(jd) * DVec3::X).dot(plain.orientation(jd) * DVec3::X);
        assert!(cos.acos().to_degrees() < 0.02, "{}", cos.acos().to_degrees());
    }

    #[test]
    fn simple_model_zero_obliquity_pole_is_ecliptic_north() {
        let doc = sc::parse("RotationPeriod 24\nObliquity 0\n");
        let m = RotationModel::from_body_block(&doc);
        let pole = m.orientation(J2000) * DVec3::Y;
        let ecl_north = crate::coords::ecliptic_to_universe(DVec3::Z);
        assert!(pole.dot(ecl_north) > 1.0 - 1e-12);
        assert!((m.period_seconds().unwrap() - 86_400.0).abs() < 1e-9);
    }
}
