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
            // `PeriodDays` is the unit-suffixed alias of `Period` (years).
            period: b.f64("Period").map(|y| y * SECONDS_PER_YEAR).or_else(|| b.f64("PeriodDays").map(|d| d * SECONDS_PER_DAY)).unwrap_or(0.0),
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
        o.semi_major_axis = if let Some(a) = b.f64("SemiMajorAxis") {
            a * METERS_PER_AU
        } else if let Some(a) = b.f64("SemiMajorAxisKm") {
            a * 1000.0
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
            self.period = TAU * (self.semi_major_axis.powi(3) / mu).sqrt();
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
}
