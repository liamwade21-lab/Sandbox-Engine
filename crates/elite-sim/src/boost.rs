//! Boost (step 2.1): its states and its energy (grade C: `0x142E4A820`, A10 and A20, `docs/elite-flight-model.md`).
//!
//! A press, only while idle: the engines off refuse it (`$INFO_boost_denied_engine_off;`), else the boost's cost is drawn from
//! the ENG capacitor all or nothing (refused with `$INFO_boost_denied_not_enough_energy;`, the capacitor untouched). Then
//! warm-up (field 0 s), firing (field 1 s: the flight's cap is the boost speed, and the forward demand is pushed to full over
//! block 0x10 s), cool-down (field 2 s), idle. While firing it adds heat at field 6 a second (label "Engines"); no energy is
//! drawn while it fires.
//!
//! While firing the flight step also multiplies every upper acceleration limit by field 4 and takes field 5 as flight assist's
//! D (`flight::step`, the block's `boost_accel`, `boost_fa_d`).
//!
//! Numbers (B, the ship item's boost record, performance record +0x24C, `data::record`): warm-up 1 s, firing 2 s, cool-down
//! 1 s (together coriolis's `boostInt` of 4 s), 10 heat a second while firing; the cost 7 MJ (field 3, coriolis's
//! `boostEnergy`). Session 2's boost runs (s05, s06) check them against the game (`tests/traces.rs`).

use crate::data::{record, SIDEWINDER};
use crate::power::{Distributor, ENG};

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
    /// Field 6: heat a second while firing.
    pub heat: f64,
}

impl BoostSpec {
    /// The stock Sidewinder's (B, the boost record).
    pub fn sidewinder(ramp: f64) -> BoostSpec {
        BoostSpec {
            warm: record::BOOST_WARM_S,
            fire: record::BOOST_FIRE_S,
            cool: record::BOOST_COOL_S,
            cost: SIDEWINDER.boost_energy,
            ramp,
            heat: record::BOOST_HEAT_PER_S,
        }
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

    /// The heat the boost adds now, a second (field 6 while firing, label "Engines": 0x142E4A820, 0x143813740).
    pub fn heat_rate(&self, spec: &BoostSpec) -> f64 {
        if self.firing() { spec.heat } else { 0.0 }
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
        assert_eq!(b.state, State::Warm(0.0));
        assert_eq!(b.press(&spec, &mut d, true), Err(Denied::Busy));
        // The record's three times: 1 s warming (no push, no heat), 2 s firing (the push to full, 10 heat a second), 1 s
        // cooling.
        let dt = 1.0 / 60.0;
        let (mut t, mut fired, mut heat) = (0.0, 0.0, 0.0);
        while b.state != State::Idle {
            if matches!(b.state, State::Warm(_)) {
                assert_eq!(b.forward(&spec, 0.0), 0.0);
            }
            if b.firing() {
                assert_eq!(b.forward(&spec, 0.0), 1.0);
                fired += dt;
            }
            heat += b.heat_rate(&spec) * dt;
            b.tick(&spec, dt);
            t += dt;
        }
        assert_eq!((spec.warm, spec.fire, spec.cool, spec.heat), (1.0, 2.0, 1.0, 10.0));
        assert!((t - 4.0).abs() < 0.02 && (fired - 2.0).abs() < 0.02 && (heat - 20.0).abs() < 0.2, "{t} {fired} {heat}");
        // Too little left (1 MJ of 8): refused, untouched.
        let left = d.charge[ENG];
        assert_eq!(b.press(&spec, &mut d, true), Err(Denied::NotEnoughEnergy));
        assert_eq!(d.charge[ENG], left);
        assert_eq!(b.press(&spec, &mut d, false), Err(Denied::EngineOff));
    }

    /// While firing the stock block's upper limits are four times the record's (from rest: 4 x 45 m/s^2 x the mass curve);
    /// not firing, the plain 45. (The jerk limit rests while firing; not firing it would hold the first tick to 25 x dt, so the
    /// second case starts with its memory already past 45, where only the acceleration limit can hold it.)
    #[test]
    fn firing_quadruples_the_acceleration() {
        use crate::flight::{step, Input, Motion};
        let (block, full_top) = data::sidewinder_block(4.0, data::THRUSTERS_2E.opt_mass);
        let full = Input { forward: 1.0, flight_assist: true, ..Input::default() };
        let dt = 1.0 / 60.0;
        let mut m = Motion::default();
        step(&block, full_top, true, &full, glam::DVec3::ZERO, &mut m, dt);
        assert!((m.vel.length() / dt - 180.0).abs() < 1e-6, "{}", m.vel.length() / dt);
        let mut m = Motion { last_acc: glam::DVec3::new(0.0, 0.0, 100.0), ..Motion::default() };
        step(&block, full_top, false, &full, glam::DVec3::ZERO, &mut m, dt);
        assert!((m.vel.length() / dt - 45.0).abs() < 1e-6, "{}", m.vel.length() / dt);
    }
}
