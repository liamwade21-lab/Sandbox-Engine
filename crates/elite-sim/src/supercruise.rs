//! The frame shift drive's supercruise (step 5.2): the gravity wells' slowdown, the speed cap, the drop.
//!
//! Rules (grade C, A50, `docs/elite-flight-model.md`):
//! - Each body gives a slowdown `s` (`0x142E96C30`), the ship takes the largest (`0x142EA13E0`):
//!   - a star (kind 1): `d' = d - 1.1 R`, `x = clamp(d' / 12 AU, 0, 1)`,
//!     `s = min(max((1 - x)^2, 1 - ln(d/AU x P04 + P00) / ln(10000 P04 + P00)), 1 - P0C / Vsc)`;
//!   - a planet (kinds 2, 7): `W = (M^0.1 + 1) x 20 AU` (M a value of the body's: its mass is inferred, the unit open),
//!     `x = clamp(d' / W, 0, 1)`, `s = min((1 - x)^2, 1 - P08 / Vsc)`.
//! - The cap: `V = Vsc (1 - s)` plainly (`0x142EA9810`); the throttle and the accelerations by flight assist on's rule with the
//!   supercruise block, braking harder while over the cap (accel mode 2, `0x141B2D8F0`).
//! - The safe drop at a target (`0x142E9A080`): `Dmin < distance < Dmax`, the relative speed under `Vdrop`, the nose within an
//!   angle; all three widen with the target's speed (`x = clamp((Vt/c - 1) / (Vsc/c - 1), 0, 1)^0.25`).
//!
//! Numbers: every `P` (the ship record's supercruise floats), `Vsc`, the drop's distances, speed and angle, M's unit and the
//! supercruise block's accelerations are data, not in the exe (D): stand-ins in `stand_in`, each to be measured (supercruise
//! runs in the user's Elite: the HUD's speed against the distance to a body, the drop's window).

/// The speed of light, m/s.
pub const C: f64 = 299_792_458.0;
/// The astronomical unit the exe uses, m (A50: 1.49598e11).
pub const AU: f64 = 1.49598e11;

/// Stand-ins (E) for the data the rules read, until measured. The values are the community's well-known figures where there
/// is one (named), else the least-assuming choice; none is fitted.
pub mod stand_in {
    use super::C;
    /// Supercruise's top speed (the community's 2001 c) and its floors near a planet and a star (the community's 30 km/s
    /// minimum for both).
    pub const VSC: f64 = 2001.0 * C;
    pub const FLOOR_PLANET: f64 = 30_000.0;
    pub const FLOOR_STAR: f64 = 30_000.0;
    /// The star rule's two log constants (P00, P04): 1 and 1 (the log term `1 - ln(d/AU + 1) / ln(10001)` is then 1 at the
    /// star, 0.92 at 1 AU and 0 at 10,000 AU).
    pub const STAR_P00: f64 = 1.0;
    pub const STAR_P04: f64 = 1.0;
    /// The planet rule's M: the body's mass in Earth masses (an assumption: the unit is open).
    pub const PLANET_MASS_UNIT_KG: f64 = 5.972e24;
    /// The planet rule's width unit: A50 read its constant as 20 AU, but the unit is not shown in the code; in AU an Earth's
    /// well would be 40 AU wide, against every supercruise approach (planets slow a ship within tens of light seconds), so
    /// light seconds stand in until session 5 measures an approach (E).
    pub const PLANET_WIDTH_UNIT_M: f64 = 299_792_458.0;
    /// The drop: under 1,000 km and under 1,000 km/s (the community's figures), the nose within 30 degrees (a stand-in).
    pub const DROP_DMIN_M: f64 = 0.0;
    pub const DROP_DMAX_M: f64 = 1.0e6;
    pub const DROP_SPEED: f64 = 1.0e6;
    pub const DROP_ANGLE_DEG: f64 = 30.0;
    /// The supercruise block's forward acceleration and braking, as a share of the cap a second (stand-ins).
    pub const ACCEL_SHARE: f64 = 0.1;
    pub const BRAKE_SHARE: f64 = 0.2;
    /// The safe stop's speed threshold (A60's `f`, data): twice the floor, so a ship the wells held near the floor glides and
    /// one coming in much faster is stopped (E).
    pub const SAFE_STOP_SPEED: f64 = 2.0 * FLOOR_PLANET;
}

/// A body's pull on supercruise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Well {
    /// A star: its radius, m.
    Star { radius: f64 },
    /// A planet or moon: its radius (m) and mass (kg).
    Planet { radius: f64, mass: f64 },
}

/// The slowdown `s` (0..1) of one body at distance `d` (m, from its centre).
pub fn slowdown(w: Well, d: f64) -> f64 {
    use stand_in::*;
    match w {
        Well::Star { radius } => {
            let dd = (d - 1.1 * radius).max(0.0);
            let x = (dd / (12.0 * AU)).clamp(0.0, 1.0);
            let log = 1.0 - ((d / AU) * STAR_P04 + STAR_P00).max(1e-12).ln() / (10_000.0 * STAR_P04 + STAR_P00).ln();
            ((1.0 - x).powi(2)).max(log).min(1.0 - FLOOR_STAR / VSC).max(0.0)
        }
        Well::Planet { radius, mass } => {
            let dd = (d - 1.1 * radius).max(0.0);
            let m = mass / PLANET_MASS_UNIT_KG;
            let width = (m.max(0.0).powf(0.1) + 1.0) * 20.0 * PLANET_WIDTH_UNIT_M;
            let x = (dd / width).clamp(0.0, 1.0);
            ((1.0 - x).powi(2)).min(1.0 - FLOOR_PLANET / VSC).max(0.0)
        }
    }
}

/// Supercruise's cap with the bodies round the ship (`(well, distance)`): `Vsc (1 - max s)`.
pub fn cap(bodies: &[(Well, f64)]) -> f64 {
    let s = bodies.iter().map(|(w, d)| slowdown(*w, *d)).fold(0.0, f64::max);
    stand_in::VSC * (1.0 - s)
}

/// Supercruise's speed after a tick: towards `throttle x cap` (never under the floor: at zero throttle a ship in supercruise
/// slows to 30 km/s, not to a stop), accelerating at a share of the cap and braking harder while over it (the stand-in shares
/// until the supercruise block is measured).
pub fn step_speed(speed: f64, throttle: f64, cap: f64, dt: f64) -> f64 {
    let want = (throttle.clamp(0.0, 1.0) * cap).max(stand_in::FLOOR_PLANET.min(cap));
    if speed < want {
        (speed + stand_in::ACCEL_SHARE * cap.max(speed) * dt).min(want)
    } else {
        (speed - stand_in::BRAKE_SHARE * speed.max(cap) * dt).max(want)
    }
}

/// A60's swept test: the first point of this tick's path (`from`, the ship relative to a body's centre, then `step`) on the
/// sphere of `radius` round that centre, as the share of the step travelled (0 to 1), or `None`: no crossing this tick, moving
/// away, or already inside (leaving a body from its surface is how supercruise starts).
pub fn sweep(from: glam::DVec3, step: glam::DVec3, radius: f64) -> Option<f64> {
    let a = step.length_squared();
    let b = from.dot(step);
    let c = from.length_squared() - radius * radius;
    if a <= 0.0 || c <= 0.0 || b >= 0.0 {
        return None;
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    (0.0..=1.0).contains(&t).then_some(t)
}

/// A60's safe stop on a hit: the closing speed (m/s) at or over the threshold is an emergency stop (TooFastNearPlanet),
/// under it the drop to the glide (OrbitalCruiseSafeStop). The threshold is data (`+0x1F4`, not found): a stand-in.
pub fn too_fast(closing: f64) -> bool {
    closing >= stand_in::SAFE_STOP_SPEED
}

/// Whether a safe drop at the target is allowed: its distance (m), the relative speed (m/s) and the angle between the nose
/// and the target (degrees).
pub fn can_drop(distance: f64, speed: f64, angle_deg: f64) -> bool {
    use stand_in::*;
    distance > DROP_DMIN_M && distance < DROP_DMAX_M && speed < DROP_SPEED && angle_deg < DROP_ANGLE_DEG
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The swept test finds the shell this tick's path crosses, not one it starts inside or moves away from.
    #[test]
    fn the_sweep_finds_the_crossing() {
        use glam::DVec3;
        let from = DVec3::new(0.0, 2000.0, 0.0);
        assert!((sweep(from, DVec3::new(0.0, -1500.0, 0.0), 1000.0).unwrap() - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(sweep(from, DVec3::new(0.0, -500.0, 0.0), 1000.0), None);
        assert_eq!(sweep(from, DVec3::new(0.0, 500.0, 0.0), 1000.0), None);
        assert_eq!(sweep(DVec3::new(0.0, 900.0, 0.0), DVec3::new(0.0, -500.0, 0.0), 1000.0), None);
        // A fast pass straight through in one tick is found at its entry.
        assert!((sweep(from, DVec3::new(0.0, -5000.0, 0.0), 1000.0).unwrap() - 0.2).abs() < 1e-12);
        assert!(too_fast(stand_in::SAFE_STOP_SPEED) && !too_fast(stand_in::FLOOR_PLANET));
    }

    /// Far from everything the cap is supercruise's top speed; near a star it is slowed, never below the floor.
    #[test]
    fn wells_slow_the_cap() {
        let sun = Well::Star { radius: 6.957e8 };
        assert!((cap(&[(sun, 20_000.0 * AU)]) - stand_in::VSC).abs() / stand_in::VSC < 1e-9);
        let near = cap(&[(sun, 0.1 * AU)]);
        let far = cap(&[(sun, 5.0 * AU)]);
        assert!(near < far && far < stand_in::VSC);
        // (Vsc (1 - s) near the floor loses digits: 1e-3 m/s of 30 km/s.)
        assert!(cap(&[(sun, 6.957e8)]) >= stand_in::FLOOR_STAR - 1e-3);
        let earth = Well::Planet { radius: 6.371e6, mass: 5.972e24 };
        assert!(cap(&[(sun, 1.0 * AU), (earth, 1.0e7)]) <= cap(&[(sun, 1.0 * AU)]));
    }

    #[test]
    fn speed_follows_the_throttle_and_the_cap() {
        let mut v = 30_000.0;
        for _ in 0..600 {
            v = step_speed(v, 1.0, 1.0e8, 1.0 / 60.0);
        }
        assert!(v > 30_000.0 && v <= 1.0e8);
        let mut w = 2.0e8;
        for _ in 0..6000 {
            w = step_speed(w, 1.0, 1.0e8, 1.0 / 60.0);
        }
        assert!((w - 1.0e8).abs() < 1.0);
        assert!(can_drop(5.0e5, 5.0e5, 10.0) && !can_drop(2.0e6, 5.0e5, 10.0));
    }
}
