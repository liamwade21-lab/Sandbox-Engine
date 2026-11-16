//! Touching down (the landing component's numbers; the rule is `pilot.rs`'s `touchdown`, which still holds its own constants
//! until a later task points it here).
//!
//! Rule (grade C, `0x1430ECE00`, A21): touchdown is allowed while the height h is `0 <= h <= P14`, the speed at most P10
//! (`|v|^2 <= P10^2`) and the ship's down within the landing record's tilt of gravity's; the approach flag is set under P1C; the
//! four landing points' rays must agree within 0.5 m (C, `.data`). What the +0x200 state gate is was not identified.
//!
//! Numbers (B, N1, `re/elite/notes/sidewinder_numbers.md`): the ship prototype's PlanetLandingModule template P (+0x10 15 m/s,
//! +0x14 100 m, +0x1C 150 m; +0x18 = 75 is read by nothing seen) and the ship item's landing record (item data +0x88, getter
//! 0x141B3B020: +0x38 the tilt, 20 degrees; the exe's 60 is only for a ship without a record).

/// P10: the fastest touchdown (m/s).
pub const TOUCHDOWN_MPS: f64 = 15.0;
/// P14: the highest the ship may be over the ground to touch down (m).
pub const TOUCHDOWN_HEIGHT_M: f64 = 100.0;
/// P1C: the approach flag's height (m).
pub const APPROACH_HEIGHT_M: f64 = 150.0;
/// The landing record's +0x38: the most the ship's down may lean from gravity's (degrees).
pub const TOUCHDOWN_TILT_DEG: f64 = 20.0;
/// The landing points' rays must agree within this (m, C).
pub const TOUCHDOWN_SPREAD_M: f64 = 0.5;

/// Whether touchdown is allowed: the height over the ground (m), the speed (m/s) and the tilt from gravity (degrees).
pub fn allowed(height_m: f64, speed: f64, tilt_deg: f64) -> bool {
    (0.0..=TOUCHDOWN_HEIGHT_M).contains(&height_m) && speed * speed <= TOUCHDOWN_MPS * TOUCHDOWN_MPS && tilt_deg <= TOUCHDOWN_TILT_DEG
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record's limits: 15 m/s, 100 m, 20 degrees; each alone refuses.
    #[test]
    fn touchdown_limits() {
        assert!(allowed(0.0, 15.0, 20.0) && allowed(100.0, 0.0, 0.0));
        assert!(!allowed(100.5, 1.0, 0.0), "too high");
        assert!(!allowed(-0.1, 1.0, 0.0), "under the ground");
        assert!(!allowed(10.0, 15.1, 0.0), "too fast");
        assert!(!allowed(10.0, 1.0, 20.5), "tilted");
    }
}
