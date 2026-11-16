//! Stars (step 5.4): the heat a star gives by distance, and its exclusion zone.
//!
//! Rules (A50, `docs/elite-flight-model.md`): per star, with `x = d / R - 1` (d from its centre, R its radius), the heat each
//! second is `Q18 x clamp((x - Q20) / (Q1C - Q20), 0, 1)` (times `Q24` unless a query says not), "Star Proximity" (C, the
//! form, `0x142C83F60`); a supercruise exclusion zone registers its owner with a radius times 1.05 (C, the registration only;
//! that the radius is the star's own is inferred, and what the zone does was not traced: here, heading into it in supercruise
//! is an emergency drop, as the emergency drop's own rule says, E).
//!
//! Numbers (B, N1): the ship prototype's ShipBodyProximityEffects template Q (`re/elite/notes/sidewinder_numbers.md`; that
//! 0x142C83F60 reads this component's +0x50 is inferred from its class name beside "Star Proximity"): the full rate 18 heat a
//! second for the Sidewinder (Q18), none from two radii out (Q20 = 1), the full rate from 1 % of a radius over the surface
//! (Q1C = 0.01). Q24 holds 0.5; what turns it off (+0xF8's vt+0x18: supercruise? a module?) is open, so the scale stays 1 here
//! (`stand_in::SCALE`).

/// The exe's factor on the exclusion zone's radius (C).
pub const EXCLUSION_FACTOR: f64 = 1.05;

/// Q18: the full rate, heat a second (B).
pub const FULL_RATE: f64 = 18.0;
/// Q20 and Q1C: the x (`d / R - 1`) where the rate is zero and where it is full (B).
pub const ZERO_X: f64 = 1.0;
pub const FULL_X: f64 = 0.01;

/// What is open.
pub mod stand_in {
    /// The scale on the rate: the record's Q24 is 0.5, applied unless a query (+0xF8's vt+0x18) says not, and which states
    /// those are was not found: 1 (no scale) as the mod has had it, until a measured approach to a star (N1's S1 run passes
    /// the star: the HUD's heat against the distance) shows which applies.
    pub const SCALE: f64 = 1.0;
}

/// A star's heat a second, as a share of the stock Sidewinder's heat capacity, at `d` from its centre (`r` its radius, both m).
pub fn heat_rate(d: f64, r: f64) -> f64 {
    if r <= 0.0 {
        return 0.0;
    }
    let x = d / r - 1.0;
    FULL_RATE / crate::data::SIDEWINDER.heat_capacity * ((x - ZERO_X) / (FULL_X - ZERO_X)).clamp(0.0, 1.0) * stand_in::SCALE
}

/// Inside the star's exclusion zone.
pub fn excluded(d: f64, r: f64) -> bool {
    d < EXCLUSION_FACTOR * r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// None from two radii out, the full 18 heat a second (of 140) from 1 % of a radius over the surface, in proportion
    /// between; the zone at 1.05 radii.
    #[test]
    fn star_heat_grows_inwards_and_the_zone() {
        let r = 7.0e8;
        let full = 18.0 / 140.0;
        assert_eq!(heat_rate(3.0 * r, r), 0.0);
        assert_eq!(heat_rate(2.0 * r, r), 0.0);
        assert!((heat_rate(1.01 * r, r) - full).abs() < 1e-12);
        assert!((heat_rate(1.0 * r, r) - full).abs() < 1e-12);
        assert!((heat_rate(1.505 * r, r) - full * 0.5).abs() < 1e-12);
        let (a, b) = (heat_rate(1.8 * r, r), heat_rate(1.3 * r, r));
        assert!(a > 0.0 && b > a && b < full);
        assert!(excluded(1.04 * r, r) && !excluded(1.06 * r, r));
    }
}
