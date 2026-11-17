//! What a deployed landing gear and a deployed cargo scoop change in flight (task N3).
//!
//! Rules (grade C, `re/elite/notes/sidewinder_orbital_cruise.md`):
//! - The landing gear module (`0x14307F8A0`) hands the flight controller a speed multiplier A every frame (controller
//!   virtual +0x168, `0x142E78C10`: controller +0x678): the performance record's +0x3EC while the ship's control state says
//!   the gear is down (its byte +0xBB, the one the frame shift drive's refusal reads), 1 otherwise.
//! - The cargo bay (`0x142A69A40`) hands it a second one, B (virtual +0x170, `0x142E780D0`: +0x680): the record's +0x3E8
//!   while the scoop's hatch is in its open state (3), 1 otherwise.
//! - The block refresh (`0x142E86E20`, step 4) multiplies both blocks' (current and full power) top speed and boost speed
//!   (`0x141B2DA50`; nothing else: not the accelerations, not the turn rates) by `min(A, B)` times the other factors (1 here).
//!   So gear and scoop together slow the ship as much as either alone. The blue zone reads the speed over the lowered top speed
//!   (`flight::step`), so the turn rates follow it. That is the branch taken in normal flight.
//! - In supercruise (the refresh's handle +0x480 active) the same product `min(A, B) x m` (m the cruise multiplier +0x670)
//!   goes instead into accel mode 2's k (`0x141B2D8F0`) of the current block alone, not through `0x141B2DA50`: the top
//!   speed, the forward and lateral accelerations and the reverse one's first term take it, the full-power block and the
//!   zero-throttle speed do not (`supercruise::cruise_block`, `Speed::step_with`, `step_held_with`, which take
//!   `speed_factor` as their `factor`).
//! - The boost is allowed with either deployed (`0x142E4A820`: its only refusals are the engines off and too little energy)
//!   and fires to the lowered boost speed. The hardpoints change no speed (nothing of theirs is in `0x142E86E20`); the drive's
//!   refusals are `drive.rs`'s.
//!
//! Numbers (B): the Sidewinder's performance record +0x3E8 and +0x3EC, 0.5 each (`re/elite/work/numbers/records/
//! item_sidewinder.txt`, block (114,13520)).
//!
//! Not found: what retracts the gear and the scoop by themselves (`0x14307F8A0` and `0x142DC2830` clear them on conditions
//! of objects not identified); whether the hatch's open state is reached at once or after its travel.

use crate::data::record;

/// The speed multiplier `min(A, B)` with the gear down and/or the cargo scoop out.
pub fn speed_factor(gear_down: bool, scoop_out: bool) -> f64 {
    let a = if gear_down { record::GEAR_SPEED } else { 1.0 };
    let b = if scoop_out { record::SCOOP_SPEED } else { 1.0 };
    a.min(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{self, THRUSTERS_2E};
    use crate::flight::{step, Input, Motion};

    /// Either one halves the top and boost speeds; both together no more than one; nothing else of the block changes.
    #[test]
    fn gear_and_scoop_halve_the_speeds() {
        assert_eq!((speed_factor(false, false), speed_factor(true, false), speed_factor(false, true), speed_factor(true, true)), (1.0, 0.5, 0.5, 0.5));
        let (free, free_top) = data::sidewinder_block(4.0, THRUSTERS_2E.opt_mass);
        let (gear, gear_top) = data::sidewinder_block_with(4.0, THRUSTERS_2E.opt_mass, speed_factor(true, false));
        assert_eq!((gear.top_speed, gear.boost_speed, gear_top), (0.5 * free.top_speed, 0.5 * free.boost_speed, 0.5 * free_top));
        assert_eq!((gear.accel_fwd, gear.accel_rev, gear.accel_side, gear.turn), (free.accel_fwd, free.accel_rev, free.accel_side, free.turn));
    }

    /// Full throttle with the gear down settles at half the top speed (110 m/s at 4 pips); the boost fires to half the boost
    /// speed.
    #[test]
    fn full_throttle_with_the_gear_down() {
        let (b, top) = data::sidewinder_block_with(4.0, THRUSTERS_2E.opt_mass, speed_factor(true, false));
        let (dt, full) = (1.0 / 60.0, Input { forward: 1.0, flight_assist: true, ..Input::default() });
        let mut m = Motion::default();
        for _ in 0..1200 {
            step(&b, top, false, &full, glam::DVec3::ZERO, &mut m, dt);
        }
        assert!((m.vel.length() - 0.5 * data::SIDEWINDER.speed).abs() < 0.5, "{}", m.vel.length());
        for _ in 0..120 {
            step(&b, top, true, &full, glam::DVec3::ZERO, &mut m, dt);
        }
        assert!(m.vel.length() <= 0.5 * data::SIDEWINDER.boost + 1e-6 && m.vel.length() > 0.5 * data::SIDEWINDER.speed + 20.0, "{}", m.vel.length());
    }
}
