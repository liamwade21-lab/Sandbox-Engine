//! Keplerian orbits as described by `Orbit { ... }` blocks.
//!
//! Catalog keys: `SemiMajorAxis` (AU) or `PericenterDist` (AU), `Period` (years),
//! `Eccentricity`, `Inclination`, `AscendingNode`, `ArgOfPericenter`/`ArgOfPericen`,
//! `LongOfPericen`, `MeanAnomaly`/`MeanLongitude`, `Epoch` (JD), `RefPlane`
//! ("Ecliptic", "Equator", "Extrinsic", ...). Angles in degrees. `Separation` (AU) is the
//! distance between the two members of a binary; the catalog splits it by mass
//! (`Catalog::resolve_separations`).

use crate::sc::Block;
use crate::units::{G, METERS_PER_AU, SECONDS_PER_DAY, SECONDS_PER_YEAR};
use glam::{DQuat, DVec3};
use std::f64::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefPlane {
    /// J2000 ecliptic (SE's default for Solar System bodies).
    #[default]
    Ecliptic,
    /// Earth's J2000 equator.
    Equator,
    /// The parent body's equator (`RefPlane "Equator"` on moons in SE refers to the
    /// parent's equator; resolved by the caller).
    ParentEquator,
    /// Plane perpendicular to the line of sight from Earth (used for binary stars).
    Extrinsic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeplerOrbit {
    /// Semi-major axis in meters (negative is not used; hyperbolic uses `e > 1`).
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    /// Radians.
    pub inclination: f64,
    pub ascending_node: f64,
    pub arg_of_periapsis: f64,
    /// Mean anomaly at epoch, radians.
    pub mean_anomaly: f64,
    /// Julian date of epoch.
    pub epoch: f64,
    /// Seconds; if zero it is derived from the parent mass.
    pub period: f64,
    pub ref_plane: RefPlane,
    /// Name of an analytic theory (e.g. "Earth-EMB-DE"); informational.
    pub analytic_model: Option<String>,
    /// `Separation` in meters until the catalog has split it between the binary's members
    /// (then 0 and `semi_major_axis` holds this member's share).
    pub separation: f64,
    /// Read from a `BinaryOrbit` block: both members carry the same elements, so the second
    /// one is put on the far side.
    pub binary: bool,
}

impl Default for KeplerOrbit {
    fn default() -> Self {
        Self {
            semi_major_axis: 0.0,
            eccentricity: 0.0,
            inclination: 0.0,
            ascending_node: 0.0,
            arg_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            epoch: crate::time::J2000,
            period: 0.0,
            ref_plane: RefPlane::Ecliptic,
            analytic_model: None,
            separation: 0.0,
            binary: false,
        }
    }
}

impl KeplerOrbit {
    /// Parse an `Orbit` block. Returns `None` if it has no usable size.
    pub fn from_block(b: &Block) -> Option<Self> {
        let mut o = KeplerOrbit {
            eccentricity: b.f64("Eccentricity").unwrap_or(0.0).max(0.0),
            inclination: b.f64("Inclination").unwrap_or(0.0).to_radians(),
            ascending_node: b.f64_any(&["AscendingNode", "PositionAngle"]).unwrap_or(0.0).to_radians(),
            epoch: b.f64("Epoch").unwrap_or(crate::time::J2000),
            // `PeriodDays` is the unit-suffixed alias of `Period` (years); `MeanMotion` (deg/day) gives
            // it too, and is how SE's catalogs time hyperbolic orbits ('Oumuamua).
            period: b
                .f64("Period")
                .map(|y| y * SECONDS_PER_YEAR)
                .or_else(|| b.f64("PeriodDays").map(|d| d * SECONDS_PER_DAY))
                .or_else(|| b.f64("MeanMotion").filter(|&n| n > 0.0).map(|n| 360.0 / n * SECONDS_PER_DAY))
                .unwrap_or(0.0),
            analytic_model: b.str("AnalyticModel").map(str::to_string),
            separation: b.f64("Separation").map_or(0.0, |a| a * METERS_PER_AU),
            ..Default::default()
        };
        o.ref_plane = match b.str("RefPlane").map(|s| s.to_ascii_lowercase()) {
            Some(s) if s == "equator" => RefPlane::ParentEquator,
            Some(s) if s == "extrinsic" => RefPlane::Extrinsic,
            Some(s) if s == "earthequator" || s == "eqtrue" => RefPlane::Equator,
            _ => RefPlane::Ecliptic,
        };
        let e = o.eccentricity;
        // SE writes a hyperbolic orbit's semi-major axis negative; here it is kept positive.
        o.semi_major_axis = if let Some(a) = b.f64("SemiMajorAxis") {
            a.abs() * METERS_PER_AU
        } else if let Some(a) = b.f64("SemiMajorAxisKm") {
            a.abs() * 1000.0
        } else if let Some(q) = b.f64("PericenterDistKm") {
            let q = q * 1000.0;
            if (e - 1.0).abs() < 1e-9 { q } else { q / (1.0 - e).abs() }
        } else if let Some(q) = b.f64("PericenterDist") {
            if (e - 1.0).abs() < 1e-9 {
                q * METERS_PER_AU // parabolic: treat q as scale
            } else {
                q * METERS_PER_AU / (1.0 - e).abs()
            }
        } else {
            0.0
        };
        // Exoplanet catalogs often give only a period; the semi-major axis is then
        // derived from the parent mass in `resolve_period`.
        if o.semi_major_axis <= 0.0 && o.period <= 0.0 && o.separation <= 0.0 && o.analytic_model.is_none() {
            return None;
        }
        // As SE's loader (FUN_1401cc2e0): the longitudes of pericentre and mean longitude default
        // to 0, so a missing argument of pericentre is `-node` and a missing mean anomaly
        // `-(node + w)`, with the catalog's node (0 if absent; a node generated later by the
        // system step leaves both alone). Proxima b (`ArgOfPericen 290` only) thus starts at
        // M = 70 deg at J2000, which SE's positions confirm.
        let node = o.ascending_node;
        o.arg_of_periapsis = if let Some(w) = b.f64_any(&["ArgOfPericenter", "ArgOfPericen", "ArgOfPeriapsis"]) {
            w.to_radians()
        } else {
            b.f64_any(&["LongOfPericen", "LongOfPericenter"]).unwrap_or(0.0).to_radians() - node
        };
        o.mean_anomaly = if let Some(m) = b.f64("MeanAnomaly") {
            m.to_radians()
        } else if let (None, Some(tp)) = (b.f64("MeanLongitude"), b.f64("PericenterEpoch")) {
            // Time of pericenter passage: M = n (epoch - tp)
            let n = if o.period > 0.0 { TAU / o.period } else { 0.0 };
            n * (o.epoch - tp) * SECONDS_PER_DAY
        } else {
            b.f64("MeanLongitude").unwrap_or(0.0).to_radians() - node - o.arg_of_periapsis
        };
        Some(o)
    }

    /// Fill in the period from Kepler's third law if the catalog didn't give one.
    pub fn resolve_period(&mut self, parent_mass_kg: f64) {
        if parent_mass_kg <= 0.0 {
            return;
        }
        let mu = G * parent_mass_kg;
        if self.period <= 0.0 && self.semi_major_axis > 0.0 {
            // Parabolic: `a` holds q and Barker's equation runs on n = sqrt(mu / (2 q^3)).
            let k = if (self.eccentricity - 1.0).abs() < 1e-9 { 2.0 } else { 1.0 };
            self.period = TAU * (k * self.semi_major_axis.powi(3) / mu).sqrt();
        } else if self.semi_major_axis <= 0.0 && self.period > 0.0 {
            // Kepler's third law: a^3 = mu P^2 / (4 pi^2).
            self.semi_major_axis = (mu * self.period * self.period / (TAU * TAU)).cbrt();
        }
    }

    pub fn mean_motion(&self) -> f64 {
        if self.period > 0.0 { TAU / self.period } else { 0.0 }
    }

    /// Orientation of the orbital plane (perifocal -> reference plane), with the
    /// reference plane's normal along +Z.
    pub fn plane_rotation(&self) -> DQuat {
        DQuat::from_rotation_z(self.ascending_node)
            * DQuat::from_rotation_x(self.inclination)
            * DQuat::from_rotation_z(self.arg_of_periapsis)
    }

    /// Position relative to the focus, in the reference-plane frame (meters).
    pub fn position_at(&self, jd: f64) -> DVec3 {
        let dt = (jd - self.epoch) * SECONDS_PER_DAY;
        let m = self.mean_anomaly + self.mean_motion() * dt;
        let a = self.semi_major_axis;
        let e = self.eccentricity;
        let p = if e < 1.0 {
            let ea = solve_kepler_elliptic(m, e);
            DVec3::new(a * (ea.cos() - e), a * (1.0 - e * e).sqrt() * ea.sin(), 0.0)
        } else if e > 1.0 {
            let h = solve_kepler_hyperbolic(m, e);
            DVec3::new(a * (e - h.cosh()), a * (e * e - 1.0).sqrt() * h.sinh(), 0.0)
        } else {
            // Parabolic (Barker's equation), `a` holds the pericenter distance.
            let q = a;
            let w = 1.5 * m;
            let y = (w + (w * w + 1.0).sqrt()).cbrt();
            let d = y - 1.0 / y;
            let nu = 2.0 * d.atan();
            let r = q * (1.0 + d * d);
            DVec3::new(r * nu.cos(), r * nu.sin(), 0.0)
        };
        self.plane_rotation() * p
    }

    /// Sample the full ellipse as a polyline in the reference-plane frame.
    pub fn sample_path(&self, segments: usize) -> Vec<DVec3> {
        let rot = self.plane_rotation();
        let a = self.semi_major_axis;
        let e = self.eccentricity;
        if e >= 1.0 {
            return Vec::new();
        }
        let b = a * (1.0 - e * e).sqrt();
        (0..=segments)
            .map(|i| {
                let ea = TAU * i as f64 / segments as f64;
                rot * DVec3::new(a * (ea.cos() - e), b * ea.sin(), 0.0)
            })
            .collect()
    }
}

/// SE's Hill and influence sphere radii (FUN_14050f090, the info panel's "Hill / influence sphere
/// radius"): `a (1 - e) cbrt(m / 3M)` and the Laplace sphere `a (m / M)^0.4` (the Hill radius when that
/// is not positive), in `a`'s unit. None without both masses.
pub fn hill_influence_radii(a: f64, e: f64, m: f64, parent_m: f64) -> Option<(f64, f64)> {
    if m <= 0.0 || parent_m <= 0.0 || a <= 0.0 {
        return None;
    }
    let q = m / parent_m;
    let hill = (q / 3.0).cbrt() * a * (1.0 - e);
    let influence = q.powf(0.4) * a;
    Some((hill, if influence > 0.0 { influence } else { hill }))
}

/// SE's "Roche limit for satellites" (the info panel, FUN_1403a8350): the rigid-body limit for
/// satellites as dense as the Moon, `2190.056 km x (M / 0.0123 Earth masses)^(1/3)` (1.26 lunar
/// radii around the Moon), in metres.
pub fn roche_limit_for_satellites(mass_kg: f64) -> f64 {
    let m_earth = mass_kg / crate::units::EARTH_MASS_KG;
    2_190.056 * 1000.0 * (m_earth / 0.0123).cbrt()
}

/// SE's orbital resonance of a period ratio (FUN_140442050): for denominators 100, 300, 500, 700
/// and 900, the five numerators around `ratio x d`, reduced; the first `p:q` with both under 11
/// within 1% (`|ratio - p/q| <= 0.01 |ratio + p/q|`). Single precision, as SE.
pub fn resonance(ratio: f64) -> Option<(u32, u32)> {
    let r = ratio as f32;
    for d in [100i32, 300, 500, 700, 900] {
        let n0 = (d as f32 * r).round() as i32;
        if n0 < 1 {
            break;
        }
        for k in -2..=2 {
            let n = n0 + k;
            let (mut a, mut b) = (n, d);
            while b != 0 {
                (a, b) = (b, a % b);
            }
            let g = a.max(1);
            let (p, q) = (n / g, d / g);
            if p < 11 && q < 11 && q > 0 {
                let f = p as f32 / q as f32;
                if (r - f).abs() <= (f + r).abs() * 0.01 {
                    return Some((p.max(0) as u32, q as u32));
                }
            }
        }
    }
    None
}

/// Solve `M = E - e sin E` for E.
pub fn solve_kepler_elliptic(m: f64, e: f64) -> f64 {
    let m = (m + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
    let mut ea = if e < 0.8 { m } else { std::f64::consts::PI.copysign(m) };
    for _ in 0..50 {
        let f = ea - e * ea.sin() - m;
        let fp = 1.0 - e * ea.cos();
        let d = f / fp;
        ea -= d;
        if d.abs() < 1e-14 {
            break;
        }
    }
    ea
}

/// Solve `M = e sinh H - H` for H.
pub fn solve_kepler_hyperbolic(m: f64, e: f64) -> f64 {
    let mut h = (2.0 * m.abs() / e + 1.8).ln().copysign(m);
    for _ in 0..100 {
        let f = e * h.sinh() - h - m;
        let fp = e * h.cosh() - 1.0;
        let d = f / fp;
        h -= d;
        if d.abs() < 1e-13 {
            break;
        }
    }
    h
}

#[cfg(test)]
mod tests {
    #[test]
    fn resonances_follow_se() {
        // Pluto and Neptune (247.9 / 164.8 years), Io and Europa, none for an irrational ratio.
        assert_eq!(resonance(164.8 / 247.9), Some((2, 3)));
        assert_eq!(resonance(3.551 / 1.769), Some((2, 1)));
        assert_eq!(resonance(std::f64::consts::PI), None);
        assert_eq!(resonance(0.001), None);
    }

    #[test]
    fn spheres_and_roche_limit_follow_se() {
        // The Earth around the Sun: Hill sphere about 1.5 million km, influence sphere about 0.92.
        let (hill, infl) = hill_influence_radii(1.496e11, 0.0167, 5.972e24, 1.989e30).unwrap();
        assert!((hill / 1e9 - 1.47).abs() < 0.02, "{hill}");
        assert!((infl / 1e9 - 0.925).abs() < 0.01, "{infl}");
        assert_eq!(hill_influence_radii(1.0, 0.0, 0.0, 1.0), None);
        // Earth: 9,480 km (1.26 Earth radii x (5.51 / 3.34)^(1/3)).
        assert!((roche_limit_for_satellites(crate::units::EARTH_MASS_KG) / 1000.0 - 9_484.0).abs() < 10.0);
    }

    use super::*;
    use crate::sc;

    #[test]
    fn kepler_solver() {
        for &e in &[0.0, 0.1, 0.5, 0.9, 0.99] {
            for i in 0..20 {
                let m = -3.0 + i as f64 * 0.3;
                let ea = solve_kepler_elliptic(m, e);
                let back = ea - e * ea.sin();
                let mm = (m + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
                assert!((back - mm).abs() < 1e-10, "e={e} m={m}");
            }
        }
        let h = solve_kepler_hyperbolic(5.0, 1.5);
        assert!((1.5 * h.sinh() - h - 5.0).abs() < 1e-9);
    }

    #[test]
    fn circular_orbit_radius() {
        let doc = sc::parse("Orbit { SemiMajorAxis 1.0 Period 1.0 Eccentricity 0 Inclination 0 }");
        let o = KeplerOrbit::from_block(doc.sub("Orbit").unwrap()).unwrap();
        for k in 0..8 {
            let p = o.position_at(crate::time::J2000 + k as f64 * 40.0);
            assert!((p.length() / METERS_PER_AU - 1.0).abs() < 1e-12);
        }
        // Quarter period moves 90 degrees.
        let p0 = o.position_at(crate::time::J2000);
        let p1 = o.position_at(crate::time::J2000 + 365.25 / 4.0);
        assert!(p0.normalize().dot(p1.normalize()).abs() < 1e-9);
    }

    #[test]
    fn unit_suffixed_keys() {
        let doc = sc::parse("Orbit { SemiMajorAxisKm 329115.316 PeriodDays 1.09648307 Eccentricity 0 }");
        let o = KeplerOrbit::from_block(doc.sub("Orbit").unwrap()).unwrap();
        assert!((o.semi_major_axis - 329_115_316.0).abs() < 1.0);
        assert!((o.period - 1.09648307 * SECONDS_PER_DAY).abs() < 1e-6);
    }

    #[test]
    fn hyperbolic_orbit_with_negative_axis_and_mean_motion() {
        // 'Oumuamua as SE's catalog has it.
        let doc = sc::parse(
            "Orbit { Epoch 2458059.5 MeanMotion 0.6802 SemiMajorAxis -1.2798 Eccentricity 1.19951 \
             Inclination 122.69 AscendingNode 24.599 ArgOfPericenter 241.70 MeanAnomaly 36.425 }",
        );
        let o = KeplerOrbit::from_block(doc.sub("Orbit").unwrap()).unwrap();
        assert!((o.semi_major_axis / METERS_PER_AU - 1.2798).abs() < 1e-9);
        assert!((o.mean_motion().to_degrees() * SECONDS_PER_DAY - 0.6802).abs() < 1e-9);
        // Outbound at the epoch, about 1.5 AU from the Sun, and farther a year on.
        let r0 = o.position_at(2458059.5).length() / METERS_PER_AU;
        let r1 = o.position_at(2458059.5 + 365.0).length() / METERS_PER_AU;
        assert!(r0 > 1.0 && r0 < 2.0 && r1 > r0 + 2.0, "{r0} {r1}");
    }

    #[test]
    fn missing_angles_default_to_zero_longitudes() {
        let deg = |o: &KeplerOrbit| (o.arg_of_periapsis.to_degrees(), o.mean_anomaly.to_degrees());
        // Proxima b: only the argument of pericentre.
        let doc = sc::parse("Orbit { PeriodDays 11.18418 SemiMajorAxis 0.04857 ArgOfPericen 290 }");
        let (w, m) = deg(&KeplerOrbit::from_block(doc.sub("Orbit").unwrap()).unwrap());
        assert!((w - 290.0).abs() < 1e-9 && (m + 290.0).abs() < 1e-9, "{w} {m}");
        // Proxima c: only the node.
        let doc = sc::parse("Orbit { PeriodDays 1928 SemiMajorAxis 1.489 AscendingNode 135 }");
        let (w, m) = deg(&KeplerOrbit::from_block(doc.sub("Orbit").unwrap()).unwrap());
        assert!((w + 135.0).abs() < 1e-9 && m.abs() < 1e-9, "{w} {m}");
    }

    #[test]
    fn axis_from_period() {
        let doc = sc::parse("Orbit { Period 1.0 }");
        let mut o = KeplerOrbit::from_block(doc.sub("Orbit").unwrap()).unwrap();
        o.resolve_period(crate::units::SOLAR_MASS_KG);
        assert!((o.semi_major_axis / METERS_PER_AU - 1.0).abs() < 2e-3);
    }

    #[test]
    fn period_from_mass() {
        let mut o = KeplerOrbit {
            semi_major_axis: METERS_PER_AU,
            ..Default::default()
        };
        o.resolve_period(crate::units::SOLAR_MASS_KG);
        assert!((o.period / SECONDS_PER_YEAR - 1.0).abs() < 1e-3);
    }

    #[test]
    fn parabolic_orbit_moves_at_escape_speed() {
        let doc = sc::parse("Orbit { Eccentricity 1 PericenterDist 0.5 }");
        let mut o = KeplerOrbit::from_block(doc.sub("Orbit").unwrap()).unwrap();
        let m = crate::units::SOLAR_MASS_KG;
        o.resolve_period(m);
        for days in [0.0, 30.0, 400.0] {
            let jd = crate::time::J2000 + days;
            let h = 1e-3;
            let r = o.position_at(jd);
            let v = (o.position_at(jd + h) - o.position_at(jd - h)) / (2.0 * h * SECONDS_PER_DAY);
            let escape = (2.0 * G * m / r.length()).sqrt();
            assert!((v.length() / escape - 1.0).abs() < 1e-5, "{days}: {}", v.length() / escape);
        }
    }
}
