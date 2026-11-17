//! The frame shift drive's supercruise (step 5.2): the gravity wells' slowdown, the speed cap, the speed, the drop.
//!
//! Rules (grade C, A50, A60, `docs/elite-flight-model.md`):
//! - Each body gives a slowdown `s` (`0x142E96C30`), the ship takes the largest (`0x142EA13E0`):
//!   - a star (kind 1): `d' = d - 1.1 R`, `x = clamp(d' / 12 AU, 0, 1)`,
//!     `s = min(max((1 - x)^2, 1 - ln(d/AU x P04 + P00) / ln(10000 P04 + P00)), 1 - P0C / Vsc)`;
//!   - kinds 2 and 7 (the planet rule; which bodies those kinds are is open): `W = (M^0.1 + 1) x 20 AU` (M a value of the
//!     body's: its mass is inferred, the unit open), `x = clamp(d' / W, 0, 1)`, `s = min((1 - x)^2, 1 - P08 / Vsc)`;
//!   - any other kind: none.
//! - The cap: `V = Vsc m`, `m = 1 - (s + (1 - max(P40, F) / Vsc - s) w)`, and the speed at zero throttle `P10 + (F - P10) w`
//!   (`0x142EA9810`, `0x142E780C0`): w is orbital cruise's blend, the ship's ShipPlanetProximityComponent's (`orbital.rs`);
//!   at w = 0 the cap is `Vsc (1 - s)` and the zero-throttle speed P10. In that component's mode 1 with a target the cap
//!   also takes `heading_factor` (up to x2.5 with a ship axis square to the target). The same frame sets the full-throttle
//!   brake's flag to `w <= 0.01` (`ORBITAL_BRAKE_W`).
//! - The speed: flight assist on's rule along the nose (`0x142E6E960`) with the supercruise block in accel mode 2
//!   (`0x141B2D8F0`, `Block::mode2`, k = m): the forward target `c + (V - c) x throttle`, the time constant, the limits (braking
//!   a full throttle at a quarter, `0x142E6AF90`), the jerk limit and the speed guard (never past the larger of the speed and
//!   the cap: over a falling cap it brakes, harder the further over). `Speed::step_held`, what `pilot.rs` calls, also holds the speed
//!   to the cap (E; its doc says why).
//! - The safe drop at a target (`0x142E9A080`): `Dmin < distance < Dmax`, the relative speed under `Vdrop`, the nose within
//!   P24; the window widens with the target's freedom `x = clamp((Vt/c - 1) / (Vsc/c - 1), 0, 1)^0.25`, Vt the speed the wells
//!   allow at the target: `Dmin = P18 + (10 P28 - P18) x` and alike for Dmax (P1C, P2C) and Vdrop (P20, P30).
//! - Orbital cruise's safe stop (`0x142CAE4A0`): a sphere of +0x1D0 swept along the tick's path meets a planet; without a
//!   target or at z + 1,000 or more an emergency stop; else by S's mode (`orbital::Mode`): nothing in mode 0; in mode 1 an
//!   emergency stop over `2 (z + (f - z) w)`; in mode 2 the glide under f, else an emergency stop (`safe_stop`).
//!
//! Numbers (B): the ship item's supercruise floats P (performance record +0x1BC) and supercruise block (+0xF0,
//! `data::record::CRUISE`), `re/elite/notes/sidewinder_numbers.md`. Not known: M's unit (`stand_in`), the orbital zone's numbers
//! (`orbital.rs`), which bodies are kinds 2 and 7 (N1's S1 and G1 runs, `re/elite/measure/BACKLOG.md`).

use crate::data::record;
use crate::flight::{FULL_FORWARD, FULL_FORWARD_BRAKE};
use crate::orbital::Mode;

/// The speed of light, m/s.
pub const C: f64 = 299_792_458.0;
/// The astronomical unit the exe uses, m (0x145E8D8B0: 1.49598e11).
pub const AU: f64 = 1.49598e11;

/// Supercruise's top speed: the supercruise block's 0x00, 6e11 m/s held in a 32-bit float (2001.384 c; B).
pub const VSC: f64 = record::CRUISE.top_speed;
/// P00, P04: the star rule's two log constants (B).
pub const STAR_P00: f64 = 1.04;
pub const STAR_P04: f64 = 0.03;
/// P08, P0C: the cap's floors near a planet (kinds 2, 7) and near a star (B). Not the zero-throttle speed (`MIN_SPEED`).
pub const FLOOR_PLANET: f64 = 200_000.0;
pub const FLOOR_STAR: f64 = 1.0e8;
/// P10: the speed at zero throttle, controller +0x688 (`0x142E780C0`, B): supercruise's slowest, and its speed on entry.
pub const MIN_SPEED: f64 = 30_000.0;
/// P14 (= record +0x1D0): the size of the sphere orbital cruise's safe stop sweeps along the path, m (B; whether it is
/// measured from the body's surface or another radius is open).
pub const SWEPT_M: f64 = 8000.0;
/// P24: the safe drop's cone round the target, degrees (B).
pub const DROP_ANGLE_DEG: f64 = 30.0;
/// P34 (z, record +0x1F0) and P38 (f, +0x1F4): the safe stop's speeds (B): an emergency at z + 1,000 or more whatever the
/// mode; in mode 2 a glide under f.
pub const SAFE_STOP_Z: f64 = 200_000.0;
pub const SAFE_STOP_F: f64 = 5050.0;
/// P3C (F) and P40: orbital cruise's approach speed, where its blend w takes the cap and the zero-throttle speed (B).
pub const APPROACH_SPEED: f64 = 2500.0;
pub const APPROACH_CAP: f64 = 2500.0;
/// P44, P48 (degrees) and P4C: the cap's heading factor in orbital cruise's mode 1 (B): `1 + (P4C - 1) g`, g 1 within P44 of
/// square to the target, 0 from P48.
pub const HEADING_FULL_DEG: f64 = 2.0;
pub const HEADING_NONE_DEG: f64 = 6.0;
pub const HEADING_BOOST: f64 = 2.5;
/// The full-throttle brake (`flight::FULL_FORWARD_BRAKE`) holds in supercruise only while w is at most this
/// (`0x142EA9810` hands the controller's flag +0x5E2 `w <= 0.01`, the float at 0x144E299A4, through its virtual +0x180).
pub const ORBITAL_BRAKE_W: f64 = 0.01;
/// The planet rule's width unit: the AU (C: 0x142E96C30 multiplies by 0x145E8D8B0 = 1.49598e11 and 0x145E8D8A0 = 1.0, then by
/// 20.0 at 0x145E895BC). With M in Earth masses every planet's well is tens of AU wide (`a_planets_well_slows_supercruise`).
pub const PLANET_WIDTH_UNIT_M: f64 = AU;

/// The safe drop's window: the nearest and farthest distance (m) and the most relative speed (m/s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropWindow {
    pub dmin: f64,
    pub dmax: f64,
    pub vdrop: f64,
}

/// P18, P1C, P20: the window at a target deep in a well (x = 0, B).
pub const DROP_WELL: DropWindow = DropWindow { dmin: 20_000.0, dmax: 1.0e6, vdrop: 1.0e6 };
/// P28, P2C, P30 x 10 (the exe's factor): the window at a free target (x = 1, B).
pub const DROP_FREE: DropWindow = DropWindow { dmin: 0.0, dmax: 3.0e7, vdrop: 3.0e7 };

/// What is not known yet, and the old functions' numbers kept until what they need is built.
pub mod stand_in {
    /// The planet rule's M: the body's mass in Earth masses (an assumption: the unit is open; N1's S1 run fits it against a
    /// moon's journal `MassEM`).
    pub const PLANET_MASS_UNIT_KG: f64 = 5.972e24;
    /// The floor `cap`, `slowdown` and `too_fast`'s callers have used near planets since step 5.2: 30 km/s, which is in truth
    /// P10, the zero-throttle speed (`super::MIN_SPEED`, B; `pilot.rs` starts supercruise at that name). The
    /// record's floor near planets is 200 km/s (`super::FLOOR_PLANET`), but without orbital cruise's blend w no ship would then
    /// slow under the safe stop's f (5,050 m/s) and every planet approach would end in an emergency stop. w's rule is found
    /// (`crate::orbital`) but its zone's seven numbers are not among the records read: the old functions keep this until they
    /// are (`cruise_cap`, `min_speed`, `safe_stop` and `orbital::Zone` take them).
    pub const FLOOR_PLANET: f64 = super::MIN_SPEED;
    /// The old functions' safe stop threshold: twice their floor (E), for the same reason.
    pub const SAFE_STOP_SPEED: f64 = 2.0 * FLOOR_PLANET;
}

/// A body's pull on supercruise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Well {
    /// A star (kind 1): its radius, m.
    Star { radius: f64 },
    /// A body of kind 2 or 7 (the planet rule): its radius (m) and mass (kg).
    Planet { radius: f64, mass: f64 },
    /// A body of any other kind: no well (`0x142E96C30` gives it none). Its radius (m).
    Other { radius: f64 },
}

/// The exe's slowdown with the given floors near planets and stars.
fn slowdown_floored(w: Well, d: f64, floor_planet: f64, floor_star: f64) -> f64 {
    match w {
        Well::Star { radius } => {
            let dd = (d - 1.1 * radius).max(0.0);
            let x = (dd / (12.0 * AU)).clamp(0.0, 1.0);
            let log = 1.0 - ((d / AU) * STAR_P04 + STAR_P00).max(1e-12).ln() / (10_000.0 * STAR_P04 + STAR_P00).ln();
            ((1.0 - x).powi(2)).max(log).min(1.0 - floor_star / VSC).max(0.0)
        }
        Well::Planet { radius, mass } => {
            let dd = (d - 1.1 * radius).max(0.0);
            let m = mass / stand_in::PLANET_MASS_UNIT_KG;
            let width = (m.max(0.0).powf(0.1) + 1.0) * 20.0 * PLANET_WIDTH_UNIT_M;
            let x = (dd / width).clamp(0.0, 1.0);
            ((1.0 - x).powi(2)).min(1.0 - floor_planet / VSC).max(0.0)
        }
        Well::Other { .. } => 0.0,
    }
}

/// The slowdown `s` (0..1) of one body at distance `d` (m, from its centre), with the record's floors (B).
pub fn well_slowdown(w: Well, d: f64) -> f64 {
    slowdown_floored(w, d, FLOOR_PLANET, FLOOR_STAR)
}

/// The slowdown as the mod has used it: the star's floor the record's, a planet's `stand_in::FLOOR_PLANET` (30 km/s) until
/// orbital cruise's blend is built.
pub fn slowdown(w: Well, d: f64) -> f64 {
    slowdown_floored(w, d, stand_in::FLOOR_PLANET, FLOOR_STAR)
}

/// Supercruise's cap with the bodies round the ship (`(well, distance)`) as the mod has used it: `Vsc (1 - max s)` with
/// `slowdown`'s floors.
pub fn cap(bodies: &[(Well, f64)]) -> f64 {
    let s = bodies.iter().map(|(w, d)| slowdown(*w, *d)).fold(0.0, f64::max);
    VSC * (1.0 - s)
}

/// The exe's cap (`0x142EA9810`): `Vsc m`, `m = 1 - (s + (1 - max(P40, F) / Vsc - s) w)`, s the wells' largest with the record's
/// floors, w orbital cruise's blend (0..1; its source is open: 0 outside orbital cruise).
pub fn cruise_cap(bodies: &[(Well, f64)], w: f64) -> f64 {
    let s = bodies.iter().map(|(b, d)| well_slowdown(*b, *d)).fold(0.0, f64::max);
    let w = w.clamp(0.0, 1.0);
    VSC * (1.0 - (s + (1.0 - APPROACH_CAP.max(APPROACH_SPEED) / VSC - s) * w))
}

/// The speed at zero throttle (controller +0x688, `0x142E780C0`): `P10 + (F - P10) w`.
pub fn min_speed(w: f64) -> f64 {
    MIN_SPEED + (APPROACH_SPEED - MIN_SPEED) * w.clamp(0.0, 1.0)
}

/// The cap's factor in orbital cruise's mode 1 with a target (`0x142EA9810`): theta = |asin(a . u)| in degrees, u the unit
/// direction to the target, a the ship transform's row +0x20 (which axis is open: a = the nose makes theta the angle off
/// square to the target); `1 + (P4C - 1) (1 - clamp((theta - P44) / (P48 - P44), 0, 1))`. `axis_dot` is a . u.
pub fn heading_factor(axis_dot: f64) -> f64 {
    let theta = axis_dot.clamp(-1.0, 1.0).asin().abs().to_degrees();
    let g = 1.0 - ((theta - HEADING_FULL_DEG) / (HEADING_NONE_DEG - HEADING_FULL_DEG)).clamp(0.0, 1.0);
    1.0 + (HEADING_BOOST - 1.0) * g
}

/// Supercruise's speed along the nose and what its jerk limit remembers (the exe's +0x630).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Speed {
    pub v: f64,
    pub last_acc: f64,
}

impl Speed {
    /// A tick: flight assist on's rule along the nose with the supercruise block in accel mode 2 (see the module's doc),
    /// `throttle` 0..1, `cap` the cap now (`cruise_cap`), `w` orbital cruise's blend (the zero-throttle speed `min_speed(w)`;
    /// the full-throttle brake only while `w <= ORBITAL_BRAKE_W`).
    pub fn step(&mut self, throttle: f64, cap: f64, w: f64, dt: f64) {
        let throttle = throttle.clamp(0.0, 1.0);
        let k = (cap / VSC).max(0.0);
        let b = record::CRUISE.mode2(k, self.v);
        let v_cap = b.top_speed;
        let low = min_speed(w);
        let want = (low + (v_cap - low) * throttle).min(v_cap);
        let t = (b.fa_d * (4.0 / 3.0 * (self.v / v_cap.max(1e-9)).clamp(0.0, 1.0) - 1.0 / 3.0).max(0.0)).max(0.5);
        let brake = if throttle >= FULL_FORWARD && w <= ORBITAL_BRAKE_W { FULL_FORWARD_BRAKE * b.accel_rev } else { b.accel_rev };
        let mut a = ((want - self.v) / t.max(dt)).clamp(-brake, b.accel_fwd);
        if b.jerk_lin > 0.0 {
            let most = self.last_acc.abs() + b.jerk_lin * dt;
            a = a.clamp(-most, most);
            self.last_acc = a;
        }
        // The speed guard: never past the larger of the speed and the cap.
        let limit = self.v.max(v_cap);
        self.v = (self.v + a * dt).min(limit).max(0.0);
    }
}

impl Speed {
    /// Supercruise's speed after a tick, as `pilot.rs` calls it: never over the wells' cap (a hard limit, E: kept from step
    /// 5.2, see below), then `Speed::step` at w = 0 (the zero-throttle speed P10) with this ship's memory of its last
    /// acceleration (the exe's +0x630: the jerk limit, 1e10 m/s^3, the supercruise block's).
    ///
    /// Why the hard limit stays: with the exe's own guard alone (`Speed::step` over a falling cap), a 75 % approach to Earth
    /// with these wells (1 AU width unit, M in Earth masses) overruns the cap from about 20 s out and reaches the glide's shell
    /// at 0.38 c, where the game holds such an approach (the community's "seven seconds at 75 %"): the wells' M unit and
    /// orbital cruise's zone numbers are not yet known (N1's S1 and G1 runs measure an approach).
    pub fn step_held(&mut self, throttle: f64, cap: f64, dt: f64) {
        self.v = self.v.min(cap);
        self.step(throttle, cap, 0.0, dt);
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

/// A body's supercruise exclusion zone (C14, the spec's F4): a star's is 1.05 of its radius (A50's registration, `stars.rs`);
/// planets and gas giants have none (they glide: A60's approach, and F4 for the giants).
pub fn exclusion_radius(w: Well) -> Option<f64> {
    match w {
        Well::Star { radius } => Some(crate::stars::EXCLUSION_FACTOR * radius),
        Well::Planet { .. } | Well::Other { .. } => None,
    }
}

/// A60's safe stop on a hit as the mod has used it: the closing speed (m/s) at or over `stand_in::SAFE_STOP_SPEED` is an
/// emergency stop (TooFastNearPlanet), under it the drop to the glide (OrbitalCruiseSafeStop).
pub fn too_fast(closing: f64) -> bool {
    closing >= stand_in::SAFE_STOP_SPEED
}

/// What orbital cruise's safe stop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafeStop {
    /// OrbitalCruiseSafeStop (exit kind 4): the glide.
    Glide,
    /// TooFastNearPlanet (exit kind 3).
    Emergency,
}

/// The exe's safe stop (`0x142CAE4A0`) when the swept sphere meets a free planet: without a target body, or closing at
/// z + 1,000 m/s or more, an emergency stop; else by S's mode: nothing (`None`) in mode 0; in mode 1 an emergency stop when
/// closing faster than `2 (z + (f - z) w)`, else nothing; in mode 2 the glide under f, else an emergency stop.
pub fn safe_stop(closing: f64, target: bool, mode: Mode, w: f64) -> Option<SafeStop> {
    if !target || closing >= SAFE_STOP_Z + 1000.0 {
        return Some(SafeStop::Emergency);
    }
    match mode {
        Mode::Free => None,
        Mode::Approach => (closing > 2.0 * (SAFE_STOP_Z + (SAFE_STOP_F - SAFE_STOP_Z) * w)).then_some(SafeStop::Emergency),
        Mode::Orbital => Some(if closing < SAFE_STOP_F { SafeStop::Glide } else { SafeStop::Emergency }),
    }
}

/// The target's freedom x (`0x142E9A080`) from the slowdown the wells give at the target: `x = clamp((Vt/c - 1) / (Vsc/c - 1),
/// 0, 1)^0.25`, Vt = (1 - s) Vsc.
pub fn freedom(target_slowdown: f64) -> f64 {
    let vt = (1.0 - target_slowdown) * VSC;
    ((vt / C - 1.0) / (VSC / C - 1.0)).clamp(0.0, 1.0).powf(0.25)
}

/// The safe drop's window at freedom x: each number from the well's to the free target's (`0x142E9A080`). A moving target's
/// widening (Dmax + 2 v_t, Vdrop + 0.1 v_t) is not built: no target here moves.
pub fn drop_window(x: f64) -> DropWindow {
    let x = x.clamp(0.0, 1.0);
    let l = |a: f64, b: f64| a + (b - a) * x;
    DropWindow { dmin: l(DROP_WELL.dmin, DROP_FREE.dmin), dmax: l(DROP_WELL.dmax, DROP_FREE.dmax), vdrop: l(DROP_WELL.vdrop, DROP_FREE.vdrop) }
}

/// Whether a safe drop at a target of freedom x is allowed: its distance (m), the relative speed (m/s) and the angle between
/// the nose and the target (degrees).
pub fn can_drop_at(distance: f64, speed: f64, angle_deg: f64, x: f64) -> bool {
    let w = drop_window(x);
    distance > w.dmin && distance < w.dmax && speed < w.vdrop && angle_deg < DROP_ANGLE_DEG
}

/// `can_drop_at` for a target deep in a well (x = 0), as `pilot.rs` calls it.
pub fn can_drop(distance: f64, speed: f64, angle_deg: f64) -> bool {
    can_drop_at(distance, speed, angle_deg, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C14: only stars have an exclusion zone.
    #[test]
    fn only_stars_have_an_exclusion_zone() {
        assert_eq!(exclusion_radius(Well::Star { radius: 7.0e8 }), Some(1.05 * 7.0e8));
        assert_eq!(exclusion_radius(Well::Planet { radius: 6.4e6, mass: 6e24 }), None);
        assert_eq!(exclusion_radius(Well::Planet { radius: 7.0e7, mass: 1.9e27 }), None);
        assert_eq!(exclusion_radius(Well::Other { radius: 1.0e5 }), None);
    }

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

    /// The exe's safe stop by S's mode: nothing in mode 0; in mode 1 an emergency over 2 (z + (f - z) w) (400 km/s at w = 0,
    /// 10.1 km/s at w = 1); in mode 2 a glide under f, an emergency at f or over; always an emergency without a target or at
    /// z + 1,000.
    #[test]
    fn the_safe_stop_by_mode() {
        use Mode::*;
        assert_eq!(safe_stop(SAFE_STOP_F - 1.0, true, Orbital, 1.0), Some(SafeStop::Glide));
        assert_eq!(safe_stop(SAFE_STOP_F, true, Orbital, 1.0), Some(SafeStop::Emergency));
        assert_eq!(safe_stop(150_000.0, true, Free, 0.0), None);
        assert_eq!(safe_stop(SAFE_STOP_Z + 1000.0, true, Free, 0.0), Some(SafeStop::Emergency));
        assert_eq!(safe_stop(1.0, false, Orbital, 1.0), Some(SafeStop::Emergency));
        assert_eq!(safe_stop(150_000.0, true, Approach, 0.0), None);
        assert_eq!(safe_stop(2.0 * SAFE_STOP_F + 1.0, true, Approach, 1.0), Some(SafeStop::Emergency));
        assert_eq!(safe_stop(2.0 * SAFE_STOP_F - 1.0, true, Approach, 1.0), None);
    }

    /// The heading factor: x2.5 within 2 degrees of square to the target, nothing from 6 degrees off it.
    #[test]
    fn the_heading_factor() {
        assert_eq!(heading_factor(0.0), HEADING_BOOST);
        assert!((heading_factor(4f64.to_radians().sin()) - 1.75).abs() < 1e-12);
        assert_eq!(heading_factor(1.0), 1.0);
        assert_eq!(heading_factor(-0.5), 1.0);
    }

    /// The full-throttle brake holds only outside orbital cruise: at w over 0.01 a full throttle over the cap brakes at the
    /// whole reverse acceleration, and the zero-throttle speed is min_speed(w).
    #[test]
    fn orbital_cruise_lifts_the_full_throttle_brake() {
        let (dt, cap) = (1.0 / 60.0, 1.0e-3 * VSC);
        let v = 2.0 * cap;
        let (mut out, mut inside) = (Speed { v, last_acc: f64::INFINITY }, Speed { v, last_acc: f64::INFINITY });
        out.step(1.0, cap, ORBITAL_BRAKE_W, dt);
        inside.step(1.0, cap, 0.5, dt);
        assert!(((v - inside.v) - 4.0 * (v - out.v)).abs() / (v - inside.v) < 1e-9);
        let mut s = Speed { v: 3000.0, last_acc: 0.0 };
        for _ in 0..1200 {
            s.step(0.0, APPROACH_CAP, 1.0, dt);
        }
        assert!((s.v - APPROACH_SPEED).abs() < 1.0, "{}", s.v);
    }

    /// Far from everything the cap is supercruise's top speed; near a star it is slowed, never below the star's floor (1e8
    /// m/s); a body of another kind adds nothing.
    #[test]
    fn wells_slow_the_cap() {
        assert!((VSC / C - 2001.384).abs() < 1e-3);
        let sun = Well::Star { radius: 6.957e8 };
        assert!((cap(&[(sun, 20_000.0 * AU)]) - VSC).abs() / VSC < 1e-9);
        let near = cap(&[(sun, 0.1 * AU)]);
        let far = cap(&[(sun, 5.0 * AU)]);
        assert!(near < far && far < VSC);
        assert!((cap(&[(sun, 6.957e8)]) - FLOOR_STAR).abs() < 1e-3 * FLOOR_STAR);
        let earth = Well::Planet { radius: 6.371e6, mass: 5.972e24 };
        assert!(cap(&[(sun, 1.0 * AU), (earth, 1.0e7)]) <= cap(&[(sun, 1.0 * AU)]));
        assert_eq!(cap(&[(sun, 1.0 * AU), (Well::Other { radius: 1.0e5 }, 1.0e6)]), cap(&[(sun, 1.0 * AU)]));
    }

    /// The record's floors (200 km/s by a planet, 1e8 m/s by a star) against the zero-throttle speed (30 km/s), and orbital
    /// cruise's blend taking both to 2,500 m/s.
    #[test]
    fn floors_and_the_zero_throttle_speed() {
        let earth = Well::Planet { radius: 6.371e6, mass: 5.972e24 };
        let sun = Well::Star { radius: 6.957e8 };
        assert!((cruise_cap(&[(earth, 1.05 * 6.371e6)], 0.0) - FLOOR_PLANET).abs() < 1e-3);
        assert!((cruise_cap(&[(sun, 6.957e8)], 0.0) - FLOOR_STAR).abs() < 1e-3 * FLOOR_STAR);
        assert!((cruise_cap(&[(earth, 1.05 * 6.371e6)], 1.0) - APPROACH_CAP).abs() < 1e-3);
        assert_eq!((min_speed(0.0), min_speed(1.0)), (MIN_SPEED, APPROACH_SPEED));
        // Throttle zero settles at the zero-throttle speed, not the floor; full throttle at the cap.
        let mut s = Speed { v: 150_000.0, last_acc: 0.0 };
        for _ in 0..1200 {
            s.step(0.0, FLOOR_PLANET, 0.0, 1.0 / 60.0);
        }
        assert!((s.v - MIN_SPEED).abs() < 1.0, "{}", s.v);
        for _ in 0..1200 {
            s.step(1.0, FLOOR_PLANET, 0.0, 1.0 / 60.0);
        }
        assert!((s.v - FLOOR_PLANET).abs() < 1.0, "{}", s.v);
    }

    /// Accel mode 2 in supercruise: from the zero-throttle speed far out the forward acceleration is the block's 4.5e10 m/s^2
    /// times k, reached under the jerk limit (1e10 m/s^3); over the cap the braking grows with the excess.
    #[test]
    fn supercruise_accelerates_by_mode_two() {
        let dt = 1.0 / 60.0;
        let mut s = Speed { v: MIN_SPEED, last_acc: 0.0 };
        s.step(1.0, VSC, 0.0, dt);
        assert!((s.v - MIN_SPEED - record::CRUISE.jerk_lin * dt * dt).abs() < 1.0, "{}", s.v);
        for _ in 0..600 {
            s.step(1.0, VSC, 0.0, dt);
        }
        assert!((s.last_acc - record::CRUISE.accel_fwd).abs() < 1.0, "{}", s.last_acc);
        // k = 1e-3: the forward acceleration 4.5e7 m/s^2 at the most.
        let cap = 1.0e-3 * VSC;
        let mut s = Speed { v: MIN_SPEED, last_acc: f64::INFINITY };
        s.step(0.5, cap, 0.0, dt);
        assert!(((s.v - MIN_SPEED) / dt - 1.0e-3 * record::CRUISE.accel_fwd).abs() < 1.0);
        // Twice over the cap at 75 % throttle: braking r (v/V0) sqrt(v/(V0 k)).
        let v = 2.0 * cap;
        let mut s = Speed { v, last_acc: f64::INFINITY };
        s.step(0.75, cap, 0.0, dt);
        let want = record::CRUISE.accel_rev * (v / VSC) * 2f64.sqrt();
        assert!(((v - s.v) / dt - want).abs() / want < 1e-9, "{} {want}", (v - s.v) / dt);
        // At full throttle a quarter of it.
        let mut s = Speed { v, last_acc: f64::INFINITY };
        s.step(1.0, cap, 0.0, dt);
        assert!(((v - s.v) / dt - 0.25 * want).abs() / want < 1e-9);
    }

    /// A planet's well (the 1 AU width unit): near the planet the cap grows in proportion to the distance (about 2 c at 10 light
    /// seconds from Earth), it is the floor inside 1.1 radii, far out it is much higher.
    #[test]
    fn a_planets_well_slows_supercruise() {
        let earth = Well::Planet { radius: 6.371e6, mass: 5.972e24 };
        let at = |d: f64| cap(&[(earth, d)]);
        let ls = C;
        assert!((at(10.0 * ls) / C - 2.0).abs() < 0.05, "{}", at(10.0 * ls) / C);
        assert!((at(20.0 * ls) / at(10.0 * ls) - 2.0).abs() < 0.01);
        assert!((at(1.05 * 6.371e6) - stand_in::FLOOR_PLANET).abs() < 1e-3);
        assert!(at(1.0 * AU) > 40.0 * at(10.0 * ls));
        // A heavier body's well is wider: slower at the same distance.
        let jupiter = Well::Planet { radius: 6.99e7, mass: 1.898e27 };
        assert!(cap(&[(jupiter, 100.0 * ls)]) < at(100.0 * ls));
    }

    /// Flying straight at Earth from 1 AU at 75 % and full throttle with `step_held` (the mod's cap, its hard limit), the
    /// closing speed at a shell 25 km over the ground is under the old functions' emergency threshold: the planet approach
    /// still glides; a ship coming in at 100 c is held to the cap the same tick.
    #[test]
    fn an_approach_arrives_slow() {
        let earth = Well::Planet { radius: 6.371e6, mass: 5.972e24 };
        let (dt, shell) = (1.0 / 60.0, 6.371e6 + 25_000.0);
        for throttle in [0.75, 1.0] {
            let (mut d, mut s, mut t) = (1.0 * AU, Speed { v: MIN_SPEED, last_acc: 0.0 }, 0.0);
            while d > shell && t < 3600.0 {
                s.step_held(throttle, cap(&[(earth, d)]), dt);
                d -= s.v * dt;
                t += dt;
            }
            assert!(d <= shell && !too_fast(s.v), "{throttle}: {t} s: {} m/s at {d} m", s.v);
        }
        let at = cap(&[(earth, 10.0 * C)]);
        let mut s = Speed { v: 100.0 * C, last_acc: 0.0 };
        s.step_held(1.0, at, dt);
        assert!(s.v <= at);
    }

    /// The jerk limit in the path the game uses: from 30 km/s at half throttle under a cap of 1,000 c, the first tick adds
    /// 1e10 x dt^2 (2.8e6 m/s; without the memory it added 3.75e8), each later one J dt^2 more than the one before, until the
    /// acceleration reaches mode 2's forward limit (after 2.25 s).
    #[test]
    fn step_held_keeps_the_jerk_limit() {
        let (dt, j) = (1.0 / 60.0, record::CRUISE.jerk_lin);
        let cap = 1000.0 * C;
        let mut s = Speed { v: MIN_SPEED, last_acc: 0.0 };
        s.step_held(0.5, cap, dt);
        assert!(((s.v - MIN_SPEED) - j * dt * dt).abs() < 1.0, "{}", s.v - MIN_SPEED);
        let was = s.v;
        s.step_held(0.5, cap, dt);
        assert!(((s.v - was) - 2.0 * j * dt * dt).abs() < 1.0, "{}", s.v - was);
        // Held 3 s, it has reached mode 2's forward acceleration (4.5e10 x k, k = cap / Vsc) and holds it.
        for _ in 0..178 {
            s.step_held(0.5, cap, dt);
        }
        assert!((s.last_acc - record::CRUISE.accel_fwd * cap / VSC).abs() < 1.0, "{}", s.last_acc);
    }

    /// The safe drop's window at a target in a well (x = 0: 20 km to 1,000 km, under 1,000 km/s) and a free one (x = 1: to
    /// 30,000 km, under 30,000 km/s); the nose within 30 degrees.
    #[test]
    fn the_safe_drop_window() {
        assert_eq!(drop_window(0.0), DROP_WELL);
        assert_eq!(drop_window(1.0), DROP_FREE);
        assert!(can_drop(5.0e5, 5.0e5, 10.0));
        assert!(!can_drop(2.0e6, 5.0e5, 10.0), "too far");
        assert!(!can_drop(5.0e5, 2.0e6, 10.0), "too fast");
        assert!(!can_drop(5.0e5, 5.0e5, 45.0), "not facing it");
        assert!(!can_drop(1.0e4, 5.0e5, 10.0), "inside the window's near edge");
        assert!(can_drop_at(1.0e4, 2.0e7, 10.0, 1.0) && can_drop_at(2.5e7, 1.0e6, 29.0, 1.0) && !can_drop_at(3.1e7, 1.0e6, 10.0, 1.0));
        // A target the wells slow to near the floor is in a well; one in empty space is free.
        assert_eq!(freedom(1.0 - FLOOR_PLANET / VSC), 0.0);
        assert!((freedom(0.0) - 1.0).abs() < 1e-12);
    }
}
