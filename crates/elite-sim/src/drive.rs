//! The frame shift drive's states for supercruise (step 5.2): charging, refusals, supercruise, the drop and the cooldowns.
//!
//! Rules (grade C, A50, `docs/elite-flight-model.md`): the refusals come in the exe's order (`0x142C71440`: disabled,
//! malfunction, cooldown, disrupted, mass lock, hardpoints, landing gear, cargo scoop, ...): the ones that can happen here are
//! the cooldown, the hardpoints, the landing gear and the cargo scoop (and being landed); at the charge's end the drive needs the nose pointed
//! away from a nearby body (`requires_alignment_escape`, within an angle of the escape direction); after leaving supercruise
//! the drive is `Arriving` for 5 s, then cools down for the module's own time (`0x142CB6D80`).
//!
//! Numbers: the charge time and the cooldown after a drop are data (the FSD's), not in the exe: stand-ins from the spec's
//! starting hints (E: charge about 15 s, cooldown about 10 s after a drop: 5 s Arriving and 5 s more), until session 5
//! measures them; the escape angle and the distance it applies within are stand-ins too (E).
//!
//! Hyperspace (step 5.3; A50): with a target, the drive's key charges a jump instead (from normal space or supercruise); the
//! charge's time is data (`D[0x04]`: a stand-in, E); at the charge's end the conditions (`5.1-fsd.md`: `requires_alignment`,
//! the nose within `Fp[0x30]` degrees of the destination, C) hold the charge, complete, until they're met; then the countdown,
//! `Departing`, a ticker from 0 to 1 at dt / 5 (C: about 5 s); a refusal that appears meanwhile, or pressing the key again,
//! cancels it; at its end the jump (`Engage::Jump`: the game runs the tunnel); after the jump Arriving 1 s (C), then the jump's
//! cooldown (`Fp[0x18]`, data: a stand-in, E). Supercruise's escape rule (`requires_alignment_escape`) is a condition at the
//! charge's end too, with the same `Fp[0x30]`, and holds its charge the same way (C; the hold is inferred from the HUD's code
//! for the failing condition and the charge's heat growing past the charge's time).

/// Stand-ins (E) until measured.
pub mod stand_in {
    /// The charge to supercruise (s), the cooldown after Arriving (s).
    pub const CHARGE_S: f64 = 15.0;
    pub const COOLDOWN_S: f64 = 5.0;
    /// Near a body (within this many of its radii from its centre) the nose must be within `ALIGN_DEG` of straight away.
    pub const ESCAPE_RADII: f64 = 3.0;
    /// `Fp[0x30]`: the alignment the charge's end needs, degrees (the exe reads the one number for the destination's and the
    /// escape's: C; its value is data).
    pub const ALIGN_DEG: f64 = 60.0;
    /// Hyperspace: the charge (s), the cooldown after a jump's Arriving (s).
    pub const JUMP_CHARGE_S: f64 = 15.0;
    pub const JUMP_COOLDOWN_S: f64 = 10.0;
}

/// The exe's Arriving time after leaving supercruise (C).
pub const ARRIVING_S: f64 = 5.0;
/// The exe's countdown (the Departing ticker's dt / 5) and Arriving after a jump (C).
pub const DEPARTING_S: f64 = 5.0;
pub const ARRIVING_AFTER_JUMP_S: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drive {
    Ready,
    /// Seconds charged.
    Charging(f64),
    Supercruise,
    /// Seconds left of Arriving, then of the cooldown.
    Arriving(f64),
    Cooldown(f64),
    /// Hyperspace: seconds charged, then the countdown's ticker (0 to 1), then the tunnel (the game ends it).
    JumpCharging(f64),
    Departing(f64),
    Tunnel,
    /// In supercruise after a jump: seconds left of Arriving and the jump's cooldown.
    Arrived(f64),
}

/// What a tick started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engage {
    Nothing,
    Supercruise,
    Jump,
}

/// Why the drive won't charge (in the exe's order), or why a finished charge didn't engage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Cooldown,
    Hardpoints,
    LandingGear,
    CargoScoop,
    Landed,
    /// The nose isn't pointed away from the nearby body (`requires_alignment_escape`).
    Escape,
    /// Hyperspace: the target needs more fuel than the drive's most for a jump, more than is aboard, is hidden behind a body,
    /// is the system the ship is in, or the nose wasn't on it at the countdown's end.
    OutOfRange,
    NoFuel,
    Obscured,
    SameSystem,
    Align,
}

impl Refusal {
    /// The HUD's message (our words: Elite's own text is in its localisation, not looked at).
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Cooldown => "Frame shift drive cooling down",
            Refusal::Hardpoints => "Frame shift drive: hardpoints deployed",
            Refusal::LandingGear => "Frame shift drive: landing gear deployed",
            Refusal::CargoScoop => "Frame shift drive: cargo scoop deployed",
            Refusal::Landed => "Frame shift drive: landed",
            Refusal::Escape => "Frame shift drive: point away from the body",
            Refusal::OutOfRange => "Frame shift drive: target out of range",
            Refusal::NoFuel => "Frame shift drive: not enough fuel",
            Refusal::Obscured => "Frame shift drive: target obscured",
            Refusal::SameSystem => "Frame shift drive: already in that system",
            Refusal::Align => "Frame shift drive: align with the destination",
        }
    }
}

/// What the ship is doing, for the refusals.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShipState {
    pub hardpoints: bool,
    pub gear_down: bool,
    pub scoop: bool,
    pub landed: bool,
    /// The angle (degrees) between the nose and straight away from the nearest body, and the distance from its centre in its
    /// radii.
    pub escape_angle: f64,
    pub radii: f64,
    /// In supercruise.
    pub cruise: bool,
    /// A hyperspace target: `None` without one, else the angle (degrees) between the nose and it, or why it can't be jumped to.
    pub target: Option<Result<f64, Refusal>>,
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
        if s.scoop {
            return Some(Refusal::CargoScoop);
        }
        if s.landed {
            return Some(Refusal::Landed);
        }
        None
    }

    /// The drive's key (J): starts a charge when ready (a jump's with a target), cancels a charge, asks for a drop in
    /// supercruise without a target (`Ok(true)`) or charges a jump with one.
    pub fn press(&mut self, s: &ShipState) -> Result<bool, Refusal> {
        match *self {
            Drive::Ready => {
                if let Some(r) = self.blocked(s) {
                    return Err(r);
                }
                if let Some(t) = s.target {
                    t?;
                    *self = Drive::JumpCharging(0.0);
                } else {
                    *self = Drive::Charging(0.0);
                }
                Ok(false)
            }
            Drive::Charging(_) => {
                *self = Drive::Ready;
                Ok(false)
            }
            Drive::Supercruise => match s.target {
                Some(t) => {
                    t?;
                    *self = Drive::JumpCharging(0.0);
                    Ok(false)
                }
                None => Ok(true),
            },
            Drive::JumpCharging(_) | Drive::Departing(_) => {
                *self = if s.cruise { Drive::Supercruise } else { Drive::Ready };
                Ok(false)
            }
            Drive::Tunnel => Ok(false),
            Drive::Arriving(_) | Drive::Cooldown(_) | Drive::Arrived(_) => Err(Refusal::Cooldown),
        }
    }

    /// A jump's charge or countdown goes on only while nothing refuses it: back to where it started from otherwise.
    fn jump_held(&mut self, s: &ShipState) -> Result<f64, Option<Refusal>> {
        let back = if s.cruise { Drive::Supercruise } else { Drive::Ready };
        let why = if s.cruise { None } else { self.blocked(s) };
        let r = match (why, s.target) {
            (Some(r), _) => Err(Some(r)),
            (None, None) => Err(None),
            (None, Some(Err(r))) => Err(Some(r)),
            (None, Some(Ok(angle))) => Ok(angle),
        };
        if r.is_err() {
            *self = back;
        }
        r
    }

    /// A tick: charging (a refusal that appears cancels it; at its end the escape rule), a jump's charge and countdown (at
    /// its end the alignment), Arriving and the cooldowns. Returns what began this tick.
    pub fn tick(&mut self, dt: f64, s: &ShipState) -> Result<Engage, Refusal> {
        match *self {
            Drive::Charging(t) => {
                if let Some(r) = self.blocked(s) {
                    *self = Drive::Ready;
                    return Err(r);
                }
                // (Complete, it holds until the nose points away from a nearby body.)
                if t + dt < stand_in::CHARGE_S || (s.radii < stand_in::ESCAPE_RADII && s.escape_angle > stand_in::ALIGN_DEG) {
                    *self = Drive::Charging(t + dt);
                    return Ok(Engage::Nothing);
                }
                *self = Drive::Supercruise;
                Ok(Engage::Supercruise)
            }
            Drive::JumpCharging(t) => match self.jump_held(s) {
                Err(why) => why.map_or(Ok(Engage::Nothing), Err),
                Ok(angle) => {
                    // (Complete, it holds until the nose is on the destination.)
                    *self = if t + dt < stand_in::JUMP_CHARGE_S || angle > stand_in::ALIGN_DEG { Drive::JumpCharging(t + dt) } else { Drive::Departing(0.0) };
                    Ok(Engage::Nothing)
                }
            },
            Drive::Departing(x) => match self.jump_held(s) {
                Err(why) => why.map_or(Ok(Engage::Nothing), Err),
                Ok(_) => {
                    let x = x + dt / DEPARTING_S;
                    if x < 1.0 {
                        *self = Drive::Departing(x);
                        Ok(Engage::Nothing)
                    } else {
                        *self = Drive::Tunnel;
                        Ok(Engage::Jump)
                    }
                }
            },
            Drive::Arriving(t) => {
                *self = if t > dt { Drive::Arriving(t - dt) } else { Drive::Cooldown(stand_in::COOLDOWN_S) };
                Ok(Engage::Nothing)
            }
            Drive::Cooldown(t) => {
                *self = if t > dt { Drive::Cooldown(t - dt) } else { Drive::Ready };
                Ok(Engage::Nothing)
            }
            Drive::Arrived(t) => {
                *self = if t > dt { Drive::Arrived(t - dt) } else { Drive::Supercruise };
                Ok(Engage::Nothing)
            }
            _ => Ok(Engage::Nothing),
        }
    }

    /// The jump is over (the tunnel's end): in supercruise, Arriving and then the jump's cooldown.
    pub fn arrived() -> Drive {
        Drive::Arrived(ARRIVING_AFTER_JUMP_S + stand_in::JUMP_COOLDOWN_S)
    }

    /// A complete charge held by its condition at the charge's end (the HUD's message), with the seconds held.
    pub fn held(&self) -> Option<(Refusal, f64)> {
        match *self {
            Drive::Charging(t) if t >= stand_in::CHARGE_S => Some((Refusal::Escape, t - stand_in::CHARGE_S)),
            Drive::JumpCharging(t) if t >= stand_in::JUMP_CHARGE_S => Some((Refusal::Align, t - stand_in::JUMP_CHARGE_S)),
            _ => None,
        }
    }

    /// The countdown's seconds left.
    pub fn countdown(&self) -> Option<f64> {
        match *self {
            Drive::Departing(x) => Some((1.0 - x) * DEPARTING_S),
            _ => None,
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
            Drive::JumpCharging(t) => Some((t / stand_in::JUMP_CHARGE_S).clamp(0.0, 1.0)),
            Drive::Departing(_) => Some(1.0),
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

    /// The refusals in the exe's order (`0x142C71440`): hardpoints, then the landing gear, then the cargo scoop.
    #[test]
    fn refusals_in_the_exes_order() {
        let all = ShipState { hardpoints: true, gear_down: true, scoop: true, landed: true, ..free() };
        assert_eq!(Drive::Ready.press(&all), Err(Refusal::Hardpoints));
        assert_eq!(Drive::Ready.press(&ShipState { hardpoints: false, ..all }), Err(Refusal::LandingGear));
        assert_eq!(Drive::Ready.press(&ShipState { hardpoints: false, gear_down: false, ..all }), Err(Refusal::CargoScoop));
        assert_eq!(Refusal::CargoScoop.message(), "Frame shift drive: cargo scoop deployed");
    }

    /// Refused with the gear down or the hardpoints out; a charge engages after its time; the drop leads to Arriving and the
    /// cooldown, which refuse a new charge until they're over; near a body pointed at it, the charge ends without engaging.
    #[test]
    fn charge_engage_drop_and_cooldown() {
        let mut d = Drive::Ready;
        assert_eq!(d.press(&ShipState { gear_down: true, ..free() }), Err(Refusal::LandingGear));
        assert_eq!(d.press(&ShipState { hardpoints: true, ..free() }), Err(Refusal::Hardpoints));
        assert_eq!(d.press(&ShipState { scoop: true, ..free() }), Err(Refusal::CargoScoop));
        assert_eq!(d.press(&free()), Ok(false));
        // The cargo scoop deployed during the charge cancels it.
        assert_eq!(d.tick(1.0 / 60.0, &ShipState { scoop: true, ..free() }), Err(Refusal::CargoScoop));
        assert_eq!(d, Drive::Ready);
        assert_eq!(d.press(&free()), Ok(false));
        let mut engaged = false;
        for _ in 0..(16 * 60) {
            if d.tick(1.0 / 60.0, &free()) == Ok(Engage::Supercruise) {
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
        // Pointed at a body from close by: the complete charge holds until the nose points away.
        assert_eq!(d.press(&free()), Ok(false));
        let near = ShipState { radii: 1.01, escape_angle: 170.0, ..ShipState::default() };
        for _ in 0..(16 * 60) {
            assert_eq!(d.tick(1.0 / 60.0, &near), Ok(Engage::Nothing));
        }
        assert!(matches!(d.held(), Some((Refusal::Escape, s)) if s > 0.9));
        assert_eq!(d.tick(1.0 / 60.0, &ShipState { escape_angle: 20.0, ..near }), Ok(Engage::Supercruise));
        let mut d = Drive::Ready;
        // Gear lowered while charging: cancelled.
        assert_eq!(d.press(&free()), Ok(false));
        assert_eq!(d.tick(0.1, &ShipState { gear_down: true, ..free() }), Err(Refusal::LandingGear));
        assert_eq!(d, Drive::Ready);
    }

    /// With a target the key charges a jump (refused if the target is), then the 5 s countdown, the jump if aligned; in
    /// supercruise a cancel goes back to supercruise; after the jump Arriving and the cooldown refuse the key, then supercruise.
    #[test]
    fn jump_charge_countdown_and_arrival() {
        let dt = 1.0 / 60.0;
        let aimed = ShipState { target: Some(Ok(2.0)), ..free() };
        let mut d = Drive::Ready;
        assert_eq!(d.press(&ShipState { target: Some(Err(Refusal::OutOfRange)), ..free() }), Err(Refusal::OutOfRange));
        assert_eq!(d.press(&aimed), Ok(false));
        assert!(matches!(d, Drive::JumpCharging(_)));
        let (mut t, mut jumped, mut counted) = (0.0, false, false);
        while t < 30.0 && !jumped {
            jumped = d.tick(dt, &aimed) == Ok(Engage::Jump);
            counted |= d.countdown().is_some_and(|c| c > 4.9);
            t += dt;
        }
        assert!(jumped && counted && d == Drive::Tunnel);
        assert!((t - stand_in::JUMP_CHARGE_S - DEPARTING_S).abs() < 3.0 * dt, "{t}");
        // Not aligned at the charge's end: it holds there (no countdown) until aligned.
        let off = ShipState { target: Some(Ok(stand_in::ALIGN_DEG + 20.0)), cruise: true, ..free() };
        let mut d = Drive::Supercruise;
        assert_eq!(d.press(&off), Ok(false));
        for _ in 0..(25 * 60) {
            assert_eq!(d.tick(dt, &off), Ok(Engage::Nothing));
        }
        assert!(matches!(d.held(), Some((Refusal::Align, s)) if s > 9.9) && d.countdown().is_none());
        assert_eq!(d.tick(dt, &ShipState { target: Some(Ok(5.0)), ..off }), Ok(Engage::Nothing));
        assert!(d.countdown().is_some());
        // The key again cancels, back to supercruise.
        assert_eq!(d.press(&off), Ok(false));
        assert_eq!(d, Drive::Supercruise);
        // The target hidden cancels.
        assert_eq!(d.press(&off), Ok(false));
        assert_eq!(d.tick(dt, &ShipState { target: Some(Err(Refusal::Obscured)), cruise: true, ..free() }), Err(Refusal::Obscured));
        assert_eq!(d, Drive::Supercruise);
        // Without a target in supercruise the key is a drop.
        assert_eq!(d.press(&ShipState { cruise: true, ..free() }), Ok(true));
        let mut d = Drive::arrived();
        assert_eq!(d.press(&aimed), Err(Refusal::Cooldown));
        for _ in 0..((ARRIVING_AFTER_JUMP_S + stand_in::JUMP_COOLDOWN_S + 0.1) as usize * 60 + 60) {
            let _ = d.tick(dt, &aimed);
        }
        assert_eq!(d, Drive::Supercruise);
    }
}
