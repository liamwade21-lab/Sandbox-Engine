//! Shields and hull (step 2.3).
//!
//! Rules (grade C, A21, `docs/elite-flight-model.md`): the shield's strength is the generator's mass curve times the ship's
//! base shield (`0x14387EB40`); it regenerates only while powered, under its maximum and once the delay since the last hit has
//! passed, at a rate blended between two numbers by the SYS channel's share (0.5 without a distributor), the delay restarted by
//! any hit with damage (`0x14387A960`, `0x143877C10`); a broken shield waits its own delay (16 s without a record) and comes back
//! once it is over a share of its maximum; it collapses at zero (or with no power); a hit at least as big as the shield takes
//! it to zero and the rest, scaled by what was left over, goes on to the hull (inferred). The hull's strength is the ship's base
//! armour times the bulkheads' multiplier (C form).
//!
//! Numbers: the base shield and armour are coriolis's (B); the generator's mass curve and its single regeneration figures are
//! coriolis's (B: `regen`, `brokenRegen`, taken as the rate at either end of the SYS blend until measured); the delay while up,
//! the share it returns at, and the energy each MJ costs are data not in the exe: stand-ins until a measurement (a shield hit:
//! the user's call, the backlog).

use crate::data::{self, ShieldGenerator};
use crate::flight::mass_curve;

/// Stand-ins (E) until measured.
pub mod stand_in {
    /// The regeneration delay while the shield is up (s), the broken one's (the exe's default when the record has none: 16 s,
    /// C), and the share of the maximum a broken shield comes back at.
    pub const DELAY_UP_S: f64 = 2.0;
    pub const DELAY_BROKEN_S: f64 = 16.0;
    pub const RETURN_SHARE: f64 = 0.5;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shields {
    /// Strength now and at most (MJ).
    pub strength: f64,
    pub max: f64,
    pub up: bool,
    since_hit: f64,
    pub regen_up: f64,
    pub regen_broken: f64,
}

/// The hull (its strength now and at most).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    pub strength: f64,
    pub max: f64,
}

impl Shields {
    /// The stock Sidewinder's shield at mass `mass_t`, full and up.
    pub fn sidewinder(mass_t: f64) -> Shields {
        let g: &ShieldGenerator = &data::SHIELD_2E;
        let k = mass_curve(mass_t, [g.min_mass, g.opt_mass, g.max_mass], [g.min_mul, g.opt_mul, g.max_mul]);
        let max = k * data::SIDEWINDER.base_shield;
        Shields { strength: max, max, up: true, since_hit: f64::MAX, regen_up: g.regen, regen_broken: g.broken_regen }
    }

    pub fn percent(&self) -> f64 {
        100.0 * self.strength / self.max.max(1e-9)
    }

    /// A tick: regeneration while powered (`sys_share` 0..1), the delay since the last hit, collapse and return.
    pub fn tick(&mut self, dt: f64, powered: bool, _sys_share: f64) {
        // (f64::MAX plus a tick stays f64::MAX: never hit.)
        self.since_hit += dt;
        if !powered {
            self.strength = 0.0;
            self.up = false;
            return;
        }
        let delay = if self.up { stand_in::DELAY_UP_S } else { stand_in::DELAY_BROKEN_S };
        if self.strength < self.max && self.since_hit >= delay {
            let rate = if self.up { self.regen_up } else { self.regen_broken };
            self.strength = (self.strength + rate * dt).min(self.max);
        }
        if !self.up && self.strength > self.max * stand_in::RETURN_SHARE {
            self.up = true;
        }
        if self.strength <= f64::EPSILON {
            self.up = false;
        }
    }

    /// A hit of `damage` (after resistances): what goes on to the hull.
    pub fn hit(&mut self, damage: f64) -> f64 {
        if damage <= 0.0 {
            return 0.0;
        }
        self.since_hit = 0.0;
        if !self.up {
            return damage;
        }
        if damage >= self.strength {
            let rest = damage - self.strength;
            self.strength = 0.0;
            self.up = false;
            rest
        } else {
            self.strength -= damage;
            0.0
        }
    }

    pub fn refill(&mut self) {
        self.strength = self.max;
        self.up = true;
        self.since_hit = f64::MAX;
    }
}

impl Hull {
    /// The stock Sidewinder's hull (base armour times the lightweight bulkheads' multiplier, B).
    pub fn sidewinder() -> Hull {
        let max = data::SIDEWINDER.base_armour * (1.0 + data::BULKHEADS_HULL_BOOST);
        Hull { strength: max, max }
    }

    pub fn percent(&self) -> f64 {
        100.0 * self.strength / self.max.max(1e-9)
    }

    /// Damage to the hull: true when it reaches zero (the ship is destroyed).
    pub fn damage(&mut self, d: f64) -> bool {
        self.strength = (self.strength - d.max(0.0)).max(0.0);
        self.strength <= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hit under the shield takes from it alone; a bigger one breaks it and the rest reaches the hull; a broken shield
    /// waits its delay, regenerates and comes back over half; the hull's zero destroys the ship.
    #[test]
    fn hits_breaks_and_regeneration() {
        let m = data::JOURNAL_UNLADEN_MASS + 2.3;
        let mut s = Shields::sidewinder(m);
        assert!(s.max > 0.0 && s.up);
        assert_eq!(s.hit(1.0), 0.0);
        let rest = s.hit(s.strength + 5.0);
        assert!((rest - 5.0).abs() < 1e-9 && !s.up);
        for _ in 0..(15 * 60) {
            s.tick(1.0 / 60.0, true, 0.5);
        }
        assert_eq!(s.strength, 0.0, "still waiting the broken delay");
        for _ in 0..(120 * 60) {
            s.tick(1.0 / 60.0, true, 0.5);
        }
        assert!(s.up && s.strength == s.max);
        let mut h = Hull::sidewinder();
        assert!(!h.damage(h.max * 0.5));
        assert!(h.damage(h.max));
    }
}
