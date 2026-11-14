//! Stars (step 5.4): the heat a star gives by distance, and its exclusion zone.
//!
//! Rules (A50, `docs/elite-flight-model.md`): per star, with `x = d / R - 1` (d from its centre, R its radius), the heat each
//! second is `P18 x clamp((x - P20) / (P1C - P20), 0, 1)` (times `P24` unless a query says not), "Star Proximity" (C, the
//! form, `0x142C83F60`); a supercruise exclusion zone registers its owner with a radius times 1.05 (C, the registration only;
//! that the radius is the star's own is inferred, and what the zone does was not traced: here, heading into it in supercruise
//! is an emergency drop, as the emergency drop's own rule says, E).
//!
//! Numbers: `P18`, `P1C`, `P20`, `P24` are data (D, not found): stand-ins (E): no heat from two radii out (x = 1), the full rate
//! at the zone's edge (x = 0.05), a full rate of 15 % of the heat capacity a second, `P24` 1.

/// The exe's factor on the exclusion zone's radius (C).
pub const EXCLUSION_FACTOR: f64 = 1.05;

/// Stand-ins (E) until measured.
pub mod stand_in {
    /// `P18`: the full rate, as a share of the heat capacity a second.
    pub const FULL_RATE: f64 = 0.15;
    /// `P20` and `P1C`: the x (`d / R - 1`) where the rate is zero and where it is full.
    pub const ZERO_X: f64 = 1.0;
    pub const FULL_X: f64 = 0.05;
    /// `P24`.
    pub const SCALE: f64 = 1.0;
}

/// A star's heat a second, as a share of the heat capacity, at `d` from its centre (`r` its radius, both m).
pub fn heat_rate(d: f64, r: f64) -> f64 {
    use stand_in::*;
    if r <= 0.0 {
        return 0.0;
    }
    let x = d / r - 1.0;
    FULL_RATE * ((x - ZERO_X) / (FULL_X - ZERO_X)).clamp(0.0, 1.0) * SCALE
}

/// Inside the star's exclusion zone.
pub fn excluded(d: f64, r: f64) -> bool {
    d < EXCLUSION_FACTOR * r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_heat_grows_inwards_and_the_zone() {
        let r = 7.0e8;
        assert_eq!(heat_rate(3.0 * r, r), 0.0);
        assert_eq!(heat_rate(2.0 * r, r), 0.0);
        assert!((heat_rate(1.05 * r, r) - stand_in::FULL_RATE).abs() < 1e-12);
        assert!((heat_rate(1.0 * r, r) - stand_in::FULL_RATE).abs() < 1e-12);
        let (a, b) = (heat_rate(1.8 * r, r), heat_rate(1.3 * r, r));
        assert!(a > 0.0 && b > a && b < stand_in::FULL_RATE);
        assert!(excluded(1.04 * r, r) && !excluded(1.06 * r, r));
    }
}
