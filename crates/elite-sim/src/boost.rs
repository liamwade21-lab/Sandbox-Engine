//! Boost (step 2.1): its states and its energy (grade C: `0x142E4A820`, A10 and A20, `docs/elite-flight-model.md`).
//!
//! A press, only while idle: the engines off refuse it (`$INFO_boost_denied_engine_off;`), else the boost's cost is drawn from
//! the ENG capacitor all or nothing (refused with `$INFO_boost_denied_not_enough_energy;`, the capacitor untouched). Then
//! warm-up (field 0 s), firing (field 1 s: the flight's cap is the boost speed, and the forward demand is pushed to full over
//! block 0x10 s), cool-down (field 2 s), idle. While firing it adds heat at field 6 a second (label "Engines"); no energy is
//! drawn while it fires.
//!
//! Numbers: the cost is coriolis's `boostEnergy` (B). The three times are data not yet measured: stand-ins (E), the coriolis
//! `boostInt` (B: the interval between boosts) split evenly between firing and cooling, until step 0.9's boost runs (s05,
//! s06) measure them. Field 6 (the heat) is step 2.2's.

use crate::power::{Distributor, ENG};

/// Stand-ins (E) until measured: the warm-up, and the firing's share of the boost interval.
pub mod stand_in {
    pub const WARM_S: f64 = 0.0;
    pub const FIRE_SHARE: f64 = 0.5;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Idle,
    /// Seconds into each.
    Warm(f64),
    Firing(f64),
    Cool(f64),
}

/// Why a press didn't boost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    EngineOff,
    NotEnoughEnergy,
    /// Not idle (warming, firing or cooling): the press does nothing, with no message.
    Busy,
}

/// The boost's timings and cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoostSpec {
    pub warm: f64,
    pub fire: f64,
    pub cool: f64,
    /// MJ from the ENG capacitor.
    pub cost: f64,
    /// Block 0x10: seconds for the forward demand to reach full while firing (0: at once).
    pub ramp: f64,
}

impl BoostSpec {
    /// The stock Sidewinder's (the cost B, the times stand-ins).
    pub fn sidewinder(ramp: f64) -> BoostSpec {
        let s = &crate::data::SIDEWINDER;
        let fire = s.boost_interval * stand_in::FIRE_SHARE;
        BoostSpec { warm: stand_in::WARM_S, fire, cool: s.boost_interval - fire, cost: s.boost_energy, ramp }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boost {
    pub state: State,
}

impl Default for Boost {
    fn default() -> Self {
        Boost { state: State::Idle }
    }
}

impl Boost {
    /// The boost key's press.
    pub fn press(&mut self, spec: &BoostSpec, dist: &mut Distributor, engine_on: bool) -> Result<(), Denied> {
        if self.state != State::Idle {
            return Err(Denied::Busy);
        }
        if !engine_on {
            return Err(Denied::EngineOff);
        }
        if !dist.draw(ENG, spec.cost) {
            return Err(Denied::NotEnoughEnergy);
        }
        self.state = if spec.warm > 0.0 { State::Warm(0.0) } else { State::Firing(0.0) };
        Ok(())
    }

    /// A tick: the states' clocks.
    pub fn tick(&mut self, spec: &BoostSpec, dt: f64) {
        self.state = match self.state {
            State::Idle => State::Idle,
            State::Warm(t) if t + dt >= spec.warm => State::Firing(t + dt - spec.warm),
            State::Warm(t) => State::Warm(t + dt),
            State::Firing(t) if t + dt >= spec.fire => State::Cool(t + dt - spec.fire),
            State::Firing(t) => State::Firing(t + dt),
            State::Cool(t) if t + dt >= spec.cool => State::Idle,
            State::Cool(t) => State::Cool(t + dt),
        };
    }

    pub fn firing(&self) -> bool {
        matches!(self.state, State::Firing(_))
    }

    /// The forward demand while firing: pushed towards full over the ramp's seconds.
    pub fn forward(&self, spec: &BoostSpec, demand: f64) -> f64 {
        match self.state {
            State::Firing(t) => {
                let push = if spec.ramp > 0.0 { (t / spec.ramp).clamp(0.0, 1.0) } else { 1.0 };
                demand.max(push)
            }
            _ => demand,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data;

    /// A press draws the cost once and runs warm, fire, cool, idle; a second press while busy and one with too little
    /// energy are refused, the capacitor untouched.
    #[test]
    fn boost_cycle_and_refusals() {
        let spec = BoostSpec::sidewinder(0.0);
        let mut d = Distributor::new(&data::DISTRIBUTOR_1E);
        let mut b = Boost::default();
        let full = d.charge[ENG];
        assert_eq!(b.press(&spec, &mut d, true), Ok(()));
        assert!((d.charge[ENG] - (full - spec.cost)).abs() < 1e-12);
        assert!(b.firing());
        assert_eq!(b.forward(&spec, 0.0), 1.0);
        assert_eq!(b.press(&spec, &mut d, true), Err(Denied::Busy));
        let mut t = 0.0;
        while b.state != State::Idle {
            b.tick(&spec, 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        assert!((t - (spec.warm + spec.fire + spec.cool)).abs() < 0.02);
        // Too little left (1 MJ of 8): refused, untouched.
        let left = d.charge[ENG];
        assert_eq!(b.press(&spec, &mut d, true), Err(Denied::NotEnoughEnergy));
        assert_eq!(d.charge[ENG], left);
        assert_eq!(b.press(&spec, &mut d, false), Err(Denied::EngineOff));
    }
}
