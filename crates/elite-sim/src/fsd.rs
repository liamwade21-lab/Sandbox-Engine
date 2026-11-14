//! The frame shift drive's jump: fuel per jump and range.
//!
//! **Fuel per jump** (grade B for the constants, the formula's shape the community's, E7): `fuel_mul x (distance x mass /
//! optimal mass) ^ fuel_power` tonnes, where mass is the ship's mass at the jump (unladen + cargo + fuel aboard). A jump
//! needing more than the drive's `max_fuel` is out of range. The spec's C16 asks for this to match a Sidewinder jump in the
//! user's journal: the journal of 2026-11-14 holds jumps of the user's other ship only (an engineered drive with a Guardian
//! booster), so that check waits for a Sidewinder `FSDJump` (step 5.1).
//!
//! **Range** (grade B, checked): `optimal mass / mass x (fuel / fuel_mul) ^ (1 / fuel_power)` light years for `fuel` up to
//! `max_fuel`. Elite's own `MaxJumpRange` in the journal's `Loadout` is this with the mass taken as unladen + `max_fuel`
//! (no cargo, only one jump's fuel aboard): 8.149506 ly for the user's stock ship, matched to all seven digits.

use crate::data::Fsd;

/// Fuel in tonnes for a jump of `distance_ly` at `mass_t`, or `None` past the drive's limit.
pub fn jump_fuel(fsd: &Fsd, distance_ly: f64, mass_t: f64) -> Option<f64> {
    let f = fsd.fuel_mul * (distance_ly * mass_t / fsd.opt_mass).powf(fsd.fuel_power);
    (f <= fsd.max_fuel).then_some(f)
}

/// The farthest jump at `mass_t` with `fuel_t` available for it (at most the drive's `max_fuel` counts).
pub fn jump_range(fsd: &Fsd, mass_t: f64, fuel_t: f64) -> f64 {
    let f = fuel_t.min(fsd.max_fuel).max(0.0);
    fsd.opt_mass / mass_t * (f / fsd.fuel_mul).powf(1.0 / fsd.fuel_power)
}

/// The journal's `MaxJumpRange`: the range with only one full jump's fuel aboard and no cargo.
pub fn max_jump_range(fsd: &Fsd, unladen_t: f64) -> f64 {
    jump_range(fsd, unladen_t + fsd.max_fuel, fsd.max_fuel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{FSD_2E, JOURNAL_MAX_JUMP_LY, JOURNAL_UNLADEN_MASS};

    /// B against B: the drive's numbers give the journal's `MaxJumpRange` to its last digit.
    #[test]
    fn max_jump_range_is_the_journals() {
        let r = max_jump_range(&FSD_2E, JOURNAL_UNLADEN_MASS);
        assert!((r - JOURNAL_MAX_JUMP_LY).abs() < 5e-7, "{r}");
    }

    #[test]
    fn fuel_and_range_agree() {
        let m = JOURNAL_UNLADEN_MASS + 2.0 + 0.3;
        let r = jump_range(&FSD_2E, m, 2.0);
        let f = jump_fuel(&FSD_2E, r, m).unwrap();
        assert!((f - FSD_2E.max_fuel).abs() < 1e-12, "{f}");
        assert!(jump_fuel(&FSD_2E, r * 1.001, m).is_none());
        assert!(jump_fuel(&FSD_2E, 4.0, m).unwrap() < 0.2);
    }
}
