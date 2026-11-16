//! Heat (step 2.2): the ship's heat store, what adds to it, the radiator that takes it away, the warnings and the damage.
//!
//! Rules (grade C, A20, `docs/elite-flight-model.md`): the capacity is the ship's (record +0xF0) and heat starts at 0.4 of it
//! (`0x143813D20`); an addition may be capped to a share of the capacity, and heat never goes under 0 (`0x143813740`); the
//! radiator takes `k x (min + (max - min) x s^2)` a second, `s = clamp((f - f0) / (f1 - f0), 0, 1)` of the heat fraction
//! `f = heat / capacity`, nothing while unpowered or under the two switches that turn it off (silent running is one: inferred)
//! (`0x144399D90`); the voice line `$HeatWarning100;` and the journal's `HeatWarning` when `f` rises through 1.5, at most every
//! 5 s; heat damage over the ship's threshold `t0`, at a rate rising to a ceiling by `f`, dealt every so many seconds, with
//! `$HeatWarningDamage;` when `f` rises through `t0`, at most every 5 s (`0x142DC1940`).
//!
//! Numbers: the capacity is the ship's heat record's (B: item data +0x20, +0xF0, 140; coriolis agrees). The radiator's are
//! `ext_radiator_standard`'s record (B, N1: +0x08 the most, 18.15; +0x18, +0x1C the curve's ends, 0 and 1; that the radiator
//! component's +0x1B0 is this record is inferred, its writer 0x1443A6BA0 taking it from a module-data object). The damage's
//! interval is the heat record's +0x104 (B, 2 s). Open (`stand_in`): what the radiator's least is, what the damage's threshold
//! and rates mean against the HUD's percentage, every load's size; and what the HUD's heat percentage is (`hud_percent` is
//! `f x 100` until N1's H1 run reads it).

/// `ext_radiator_standard`'s R+0x08: the radiator's rate at the top of its curve (heat a second, B with the inferred link).
pub const DISSIPATE_MAX: f64 = 18.15;
/// R+0x18, R+0x1C: the heat fractions where the radiator's curve starts and reaches its top (B with the inferred link).
pub const DISSIPATE_F0: f64 = 0.0;
pub const DISSIPATE_F1: f64 = 1.0;
/// The heat record's +0x104: heat damage is dealt every this many seconds (B).
pub const DAMAGE_EVERY_S: f64 = 2.0;

/// What is open, each kept as the mod has had it until a measurement says what the record's number means.
pub mod stand_in {
    /// The radiator's rate at the bottom of its curve (heat a second): EDSY's `heatdismin` 1.18, which is the record's R+0x00,
    /// a field the rate code (0x144399D90) doesn't read; the field it reads, R+0x04, is 0. Something else may read 1.18 (a
    /// passive loss): kept until N1's H1 run sees whether a cool idle ship still sheds heat.
    pub const DISSIPATE_MIN: f64 = 1.18;
    /// The radiator's multiplier `k` (starts at 1 in the exe's tick; its other values not found).
    pub const DISSIPATE_K: f64 = 1.0;
    /// Heat damage: the threshold fraction, the fraction where the rate reaches its ceiling, the rates (a share of the hull a
    /// second here). The heat record holds 2.5, 3.5, 0.5 and 1.5 (+0xF4..+0x100), but 2.5 as heat / capacity disagrees with the
    /// game's known damage above 100 % unless the HUD's percentage is not heat / capacity, and the rates' unit is not known:
    /// kept until N1's H1 run reads the HUD's scale.
    pub const DAMAGE_F0: f64 = 1.0;
    pub const DAMAGE_F1: f64 = 1.5;
    pub const DAMAGE_RATE_LO: f64 = 0.0;
    pub const DAMAGE_RATE_HI: f64 = 0.01;
    /// The heat the ship makes idling (MJ a second: the power plant and whatever else runs), the one number that holds session
    /// 2's idle at its measured 36 % (s09, 60 s at rest, 2/2/2 pips; near the system's star, so the star's heat is in it) with
    /// the radiator's numbers above and the HUD's percentage taken as heat / capacity: a stand-in fitted to one measurement
    /// (E) until N1's H1 and F1 runs separate the loads.
    pub const IDLE_LOAD: f64 = 3.379;
}

/// The exe's constants (C).
pub const START_SHARE: f64 = 0.4;
pub const WARNING_FRACTION: f64 = 1.5;
pub const WARNING_EVERY_S: f64 = 5.0;

/// The radiator and damage numbers of one ship.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeatSpec {
    pub capacity: f64,
    pub dis_min: f64,
    pub dis_max: f64,
    pub dis_f0: f64,
    pub dis_f1: f64,
    pub dis_k: f64,
    pub dmg_f0: f64,
    pub dmg_f1: f64,
    pub dmg_lo: f64,
    pub dmg_hi: f64,
    pub dmg_every: f64,
}

impl HeatSpec {
    pub fn sidewinder() -> HeatSpec {
        HeatSpec {
            capacity: crate::data::SIDEWINDER.heat_capacity,
            dis_min: stand_in::DISSIPATE_MIN,
            dis_max: DISSIPATE_MAX,
            dis_f0: DISSIPATE_F0,
            dis_f1: DISSIPATE_F1,
            dis_k: stand_in::DISSIPATE_K,
            dmg_f0: stand_in::DAMAGE_F0,
            dmg_f1: stand_in::DAMAGE_F1,
            dmg_lo: stand_in::DAMAGE_RATE_LO,
            dmg_hi: stand_in::DAMAGE_RATE_HI,
            dmg_every: DAMAGE_EVERY_S,
        }
    }
}

/// What a tick's heat did, for the voice, the journal and the hull.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeatEvents {
    /// `$HeatWarning100;` / `HeatWarning`.
    pub warning: bool,
    /// `$HeatWarningDamage;` / `HeatDamage`.
    pub damage_warning: bool,
    /// Hull damage dealt this tick (a share of the hull).
    pub damage: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Heat {
    pub heat: f64,
    last_f: f64,
    since_warning: f64,
    since_damage_warning: f64,
    damage_acc: f64,
    damage_clock: f64,
}

impl Heat {
    pub fn new(spec: &HeatSpec) -> Heat {
        let heat = START_SHARE * spec.capacity;
        Heat { heat, last_f: START_SHARE, since_warning: f64::MAX, since_damage_warning: f64::MAX, damage_acc: 0.0, damage_clock: 0.0 }
    }

    pub fn fraction(&self, spec: &HeatSpec) -> f64 {
        self.heat / spec.capacity.max(1e-9)
    }

    /// The HUD's percentage (open: `f x 100` until measured).
    pub fn hud_percent(&self, spec: &HeatSpec) -> f64 {
        self.fraction(spec) * 100.0
    }

    /// Adds `q` MJ (a load x dt); with `cap` the result is held to that share of the capacity (`0x143813740`).
    pub fn add(&mut self, spec: &HeatSpec, q: f64, cap: Option<f64>) {
        let q = match cap {
            Some(c) => q.min((c * spec.capacity - self.heat).max(0.0)),
            None => q,
        };
        self.heat = (self.heat + q).max(0.0);
    }

    /// A tick: the radiator (`powered`, not `silent`), then the warnings and the damage.
    pub fn tick(&mut self, spec: &HeatSpec, dt: f64, powered: bool, silent: bool) -> HeatEvents {
        let f = self.fraction(spec);
        if powered && !silent {
            let s = ((f - spec.dis_f0) / (spec.dis_f1 - spec.dis_f0).max(1e-9)).clamp(0.0, 1.0);
            let rate = spec.dis_k * (spec.dis_min + (spec.dis_max - spec.dis_min) * s * s);
            self.heat = (self.heat - rate * dt).max(0.0);
        }
        let f = self.fraction(spec);
        let mut ev = HeatEvents::default();
        self.since_warning += dt;
        self.since_damage_warning += dt;
        if self.last_f < WARNING_FRACTION && f >= WARNING_FRACTION && self.since_warning >= WARNING_EVERY_S {
            ev.warning = true;
            self.since_warning = 0.0;
        }
        if f > spec.dmg_f0 {
            if self.last_f <= spec.dmg_f0 && self.since_damage_warning >= WARNING_EVERY_S {
                ev.damage_warning = true;
                self.since_damage_warning = 0.0;
            }
            let x = ((f - spec.dmg_f0) / (spec.dmg_f1 - spec.dmg_f0).max(1e-9)).clamp(0.0, 1.0);
            self.damage_acc += (spec.dmg_lo + (spec.dmg_hi.max(spec.dmg_lo) - spec.dmg_lo) * x) * dt;
            self.damage_clock += dt;
            if self.damage_clock >= spec.dmg_every {
                ev.damage = self.damage_acc;
                self.damage_acc = 0.0;
                self.damage_clock = 0.0;
            }
        } else {
            self.damage_clock = 0.0;
        }
        self.last_f = f;
        ev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Heat starts at 0.4 of the capacity, the radiator brings it down and never under 0, an addition's cap holds.
    #[test]
    fn heat_store_and_radiator() {
        let spec = HeatSpec::sidewinder();
        let mut h = Heat::new(&spec);
        assert!((h.fraction(&spec) - 0.4).abs() < 1e-12);
        let before = h.heat;
        h.tick(&spec, 1.0, true, false);
        assert!(h.heat < before);
        let silent = h.heat;
        h.tick(&spec, 1.0, true, true);
        assert_eq!(h.heat, silent);
        for _ in 0..10_000 {
            h.tick(&spec, 1.0, true, false);
        }
        assert_eq!(h.heat, 0.0);
        h.add(&spec, 1000.0, Some(0.5));
        assert!((h.fraction(&spec) - 0.5).abs() < 1e-12);
    }

    /// The warning fires once as the fraction rises through 1.5; heat damage over the threshold is dealt on its interval.
    #[test]
    fn warnings_and_damage() {
        let spec = HeatSpec::sidewinder();
        let mut h = Heat::new(&spec);
        h.add(&spec, 1.49 * spec.capacity - h.heat, None);
        assert!(!h.tick(&spec, 0.0, false, false).warning);
        h.add(&spec, 0.02 * spec.capacity, None);
        let ev = h.tick(&spec, 0.0, false, false);
        assert!(ev.warning);
        assert!(!h.tick(&spec, 0.0, false, false).warning);
        // The damage is dealt every 2 s (the record's +0x104): nothing in the first 1.9 s, then all of it at once.
        let mut dealt = Vec::new();
        for i in 0..150 {
            let d = h.tick(&spec, 1.0 / 60.0, false, false).damage;
            if d > 0.0 {
                dealt.push(i);
            }
        }
        assert_eq!(spec.dmg_every, 2.0);
        assert!(dealt.len() == 1 && (119..=120).contains(&dealt[0]), "{dealt:?}");
    }
}
