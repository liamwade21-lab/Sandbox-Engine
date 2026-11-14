//! The frame shift drive's states for supercruise (step 5.2): charging, refusals, supercruise, the drop and the cooldowns.
//!
//! Rules (grade C, A50, `docs/elite-flight-model.md`): the refusals come in the exe's order (`0x142C71440`: disabled,
//! malfunction, cooldown, disrupted, mass lock, hardpoints, landing gear, cargo scoop, ...): the ones that can happen here are
//! the cooldown, the hardpoints and the landing gear (and being landed); at the charge's end the drive needs the nose pointed
//! away from a nearby body (`requires_alignment_escape`, within an angle of the escape direction); after leaving supercruise
//! the drive is `Arriving` for 5 s, then cools down for the module's own time (`0x142CB6D80`).
//!
//! Numbers: the charge time and the cooldown after a drop are data (the FSD's), not in the exe: stand-ins from the spec's
//! starting hints (E: charge about 15 s, cooldown about 10 s after a drop: 5 s Arriving and 5 s more), until session 5
//! measures them; the escape angle and the distance it applies within are stand-ins too (E).

/// Stand-ins (E) until measured.
pub mod stand_in {
    /// The charge to supercruise (s), the cooldown after Arriving (s).
    pub const CHARGE_S: f64 = 15.0;
    pub const COOLDOWN_S: f64 = 5.0;
    /// Near a body (within this many of its radii from its centre) the nose must be within this many degrees of straight away.
    pub const ESCAPE_RADII: f64 = 3.0;
    pub const ESCAPE_DEG: f64 = 60.0;
}

/// The exe's Arriving time after leaving supercruise (C).
pub const ARRIVING_S: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drive {
    Ready,
    /// Seconds charged.
    Charging(f64),
    Supercruise,
    /// Seconds left of Arriving, then of the cooldown.
    Arriving(f64),
    Cooldown(f64),
}

/// Why the drive won't charge (in the exe's order), or why a finished charge didn't engage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Cooldown,
    Hardpoints,
    LandingGear,
    Landed,
    /// The nose isn't pointed away from the nearby body (`requires_alignment_escape`).
    Escape,
}

impl Refusal {
    /// The HUD's message (our words: Elite's own text is in its localisation, not looked at).
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Cooldown => "Frame shift drive cooling down",
            Refusal::Hardpoints => "Frame shift drive: hardpoints deployed",
            Refusal::LandingGear => "Frame shift drive: landing gear deployed",
            Refusal::Landed => "Frame shift drive: landed",
            Refusal::Escape => "Frame shift drive: point away from the body",
        }
    }
}

/// What the ship is doing, for the refusals.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShipState {
    pub hardpoints: bool,
    pub gear_down: bool,
    pub landed: bool,
    /// The angle (degrees) between the nose and straight away from the nearest body, and the distance from its centre in its
    /// radii.
    pub escape_angle: f64,
    pub radii: f64,
}

impl Drive {
    fn blocked(self, s: &ShipState) -> Option<Refusal> {
        if matches!(self, Drive::Arriving(_) | Drive::Cooldown(_)) {
            return Some(Refusal::Cooldown);
        }
        if s.hardpoints {
            return Some(Refusal::Hardpoints);
        }
        if s.gear_down {
            return Some(Refusal::LandingGear);
        }
        if s.landed {
            return Some(Refusal::Landed);
        }
        None
    }

    /// The drive's key (J): starts a charge when ready, cancels a charge, asks for a drop in supercruise (`Ok(true)`).
    pub fn press(&mut self, s: &ShipState) -> Result<bool, Refusal> {
        match *self {
            Drive::Ready => {
                if let Some(r) = self.blocked(s) {
                    return Err(r);
                }
                *self = Drive::Charging(0.0);
                Ok(false)
            }
            Drive::Charging(_) => {
                *self = Drive::Ready;
                Ok(false)
            }
            Drive::Supercruise => Ok(true),
            Drive::Arriving(_) | Drive::Cooldown(_) => Err(Refusal::Cooldown),
        }
    }

    /// A tick: charging (a refusal that appears cancels it; at its end the escape rule), Arriving and the cooldown. Returns
    /// `Ok(true)` the tick supercruise begins.
    pub fn tick(&mut self, dt: f64, s: &ShipState) -> Result<bool, Refusal> {
        match *self {
            Drive::Charging(t) => {
                if let Some(r) = self.blocked(s) {
                    *self = Drive::Ready;
                    return Err(r);
                }
                if t + dt < stand_in::CHARGE_S {
                    *self = Drive::Charging(t + dt);
                    return Ok(false);
                }
                if s.radii < stand_in::ESCAPE_RADII && s.escape_angle > stand_in::ESCAPE_DEG {
                    *self = Drive::Ready;
                    return Err(Refusal::Escape);
                }
                *self = Drive::Supercruise;
                Ok(true)
            }
            Drive::Arriving(t) => {
                *self = if t > dt { Drive::Arriving(t - dt) } else { Drive::Cooldown(stand_in::COOLDOWN_S) };
                Ok(false)
            }
            Drive::Cooldown(t) => {
                *self = if t > dt { Drive::Cooldown(t - dt) } else { Drive::Ready };
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// Leaving supercruise (a drop): Arriving, then the cooldown.
    pub fn drop_out(&mut self) {
        *self = Drive::Arriving(ARRIVING_S);
    }

    /// The charge's share done (the HUD's bar).
    pub fn charge_share(&self) -> Option<f64> {
        match *self {
            Drive::Charging(t) => Some((t / stand_in::CHARGE_S).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free() -> ShipState {
        ShipState { radii: 10.0, ..ShipState::default() }
    }

    /// Refused with the gear down or the hardpoints out; a charge engages after its time; the drop leads to Arriving and the
    /// cooldown, which refuse a new charge until they're over; near a body pointed at it, the charge ends without engaging.
    #[test]
    fn charge_engage_drop_and_cooldown() {
        let mut d = Drive::Ready;
        assert_eq!(d.press(&ShipState { gear_down: true, ..free() }), Err(Refusal::LandingGear));
        assert_eq!(d.press(&ShipState { hardpoints: true, ..free() }), Err(Refusal::Hardpoints));
        assert_eq!(d.press(&free()), Ok(false));
        let mut engaged = false;
        for _ in 0..(16 * 60) {
            if d.tick(1.0 / 60.0, &free()) == Ok(true) {
                engaged = true;
            }
        }
        assert!(engaged && d == Drive::Supercruise);
        assert_eq!(d.press(&free()), Ok(true));
        d.drop_out();
        assert_eq!(d.press(&free()), Err(Refusal::Cooldown));
        for _ in 0..(11 * 60) {
            let _ = d.tick(1.0 / 60.0, &free());
        }
        assert_eq!(d, Drive::Ready);
        // Pointed at a body from close by: the charge ends refused.
        assert_eq!(d.press(&free()), Ok(false));
        let near = ShipState { radii: 1.01, escape_angle: 170.0, ..ShipState::default() };
        let mut last = Ok(false);
        for _ in 0..(16 * 60) {
            last = d.tick(1.0 / 60.0, &near);
            if last.is_err() {
                break;
            }
        }
        assert_eq!(last, Err(Refusal::Escape));
        // Gear lowered while charging: cancelled.
        assert_eq!(d.press(&free()), Ok(false));
        assert_eq!(d.tick(0.1, &ShipState { gear_down: true, ..free() }), Err(Refusal::LandingGear));
        assert_eq!(d, Drive::Ready);
    }
}
