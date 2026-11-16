//! Hyperspace's rules (step 5.3) that aren't the drive's states (`drive.rs`) or the jump's fuel (`fsd.rs`): where the ship
//! arrives, whether a body hides the target, and the countdown's heat.
//!
//! Grades: the arrival's distance is not found in the exe (A50: "Hyperspace arrival position ... not found"): a stand-in
//! (E), 5 radii of the star out on the line from where the ship jumped from, facing it (the star about 23 degrees across).
//! The target hidden behind a body refuses the jump in Elite (its HUD's "target obscured"); the exe's test was not traced:
//! ours (F) is the straight line to the target passing within a body's radius. The charge's and the countdown's heat have the
//! exe's form (`5.1-fsd.md`: heat += (D[0x0C] + D[0x10] x max(0, t - D[0x04])) x dt while charging, D[0x0C] x dt departing: C)
//! with the FSD module record's numbers (B, N1: 10 heat a second, no growth past the charge). The tunnel's length is measured
//! from the user's journals (D).

use glam::DVec3;

/// The tunnel's length (s, D): the user's journals give 18 to 19 s from `StartJump` (written at the countdown's start) to
/// `FSDJump` over 6 jumps (2026-11-14, the user's other ship: the tunnel is taken to be the same for every ship, E), less the
/// exe's 5 s countdown; whole-second timestamps, so +-1 s. The game's tunnel lasts this long, or until the system has loaded.
pub const TUNNEL_S: f64 = 13.5;

/// `D[0x0C]`: the heat a second while the jump charges and counts down; `D[0x10]`: its growth a second for each second a
/// complete charge is held past its time (heat units, the capacity 140; B).
pub const DEPARTING_HEAT_PER_S: f64 = 10.0;
pub const OVERCHARGE_HEAT_PER_S: f64 = 0.0;

/// Stand-ins (E) until measured, and the two heats as `pilot.rs` reads them.
pub mod stand_in {
    /// The arrival's distance from the star, in its radii (not found in the exe; N1's J1 looks at a past arrival video).
    pub const ARRIVAL_RADII: f64 = 5.0;
    /// `super::DEPARTING_HEAT_PER_S` and `OVERCHARGE_HEAT_PER_S` (B) as shares of the stock Sidewinder's heat capacity, at the
    /// path `pilot.rs` reads them from.
    pub const DEPARTING_HEAT_RATE: f64 = super::DEPARTING_HEAT_PER_S / crate::data::SIDEWINDER.heat_capacity;
    pub const OVERCHARGE_HEAT_RATE: f64 = super::OVERCHARGE_HEAT_PER_S / crate::data::SIDEWINDER.heat_capacity;
}

/// Whether a body hides the target: `to_target` and each body's centre relative to the ship (m), with its radius.
pub fn obscured(to_target: DVec3, bodies: &[(DVec3, f64)]) -> bool {
    let dist = to_target.length();
    if dist <= 0.0 {
        return false;
    }
    let dir = to_target / dist;
    bodies.iter().any(|&(c, r)| {
        let along = c.dot(dir);
        along > 0.0 && along < dist && (c - dir * along).length() < r
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FSD record's heat at charge: 10 a second of the capacity's 140, nothing more past the charge.
    #[test]
    fn the_charge_heats_ten_a_second() {
        assert!((stand_in::DEPARTING_HEAT_RATE * 140.0 - 10.0).abs() < 1e-12 && stand_in::OVERCHARGE_HEAT_RATE == 0.0);
    }

    #[test]
    fn a_body_on_the_line_hides_the_target() {
        let target = DVec3::new(0.0, 0.0, -4e16);
        assert!(obscured(target, &[(DVec3::new(0.0, 1e6, -1e7), 6.4e6)]));
        assert!(!obscured(target, &[(DVec3::new(0.0, 1e7, -1e7), 6.4e6)]));
        // Behind the ship, or beyond the target: no.
        assert!(!obscured(target, &[(DVec3::new(0.0, 0.0, 1e7), 6.4e6)]));
        assert!(!obscured(target, &[(DVec3::new(0.0, 0.0, -5e16), 6.4e6)]));
    }
}
