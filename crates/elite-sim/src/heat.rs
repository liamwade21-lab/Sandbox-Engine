//! Heat (step 2.2): the ship's heat store, what adds to it, the radiator that takes it away, the warnings and the damage.
//!
//! Rules (grade C, A20, `docs/elite-flight-model.md`): the capacity is the ship's (record +0xF0) and heat starts at 0.4 of it
//! (`0x143813D20`); an addition may be capped to a share of the capacity, and heat never goes under 0 (`0x143813740`); the
//! radiator takes `k x (min + (max - min) x s^2)` a second, `s = clamp((f - f0) / (f1 - f0), 0, 1)` of the heat fraction
//! `f = heat / capacity`, nothing while unpowered or under the two switches that turn it off (silent running is one: inferred)
//! (`0x144399D90`, which also gives the radiator's two visual values: `visual_dissipation`); the voice line `$HeatWarning100;` and the journal's `HeatWarning` when `f` rises through 1.5, at most every
//! 5 s; heat damage over the ship's threshold `t0`, at a rate rising to a ceiling by `f`, dealt every so many seconds, with
//! `$HeatWarningDamage;` when `f` rises through `t0`, at most every 5 s (`0x142DC1940`).
//!
//! The HUD's percentage (grade A, UI4c): `f / 1.5 x 100` (`hud_percent`): the exe sends the gauge `f / 3.5` with its marks
//! at 1.5 / 3.5 and 2.5 / 3.5 (FUN_142d6d3e0) and the movie prints `int(arg1 / arg2 x 100 + 0.5)` percent
//! (`re/elite/notes/ui_messages_hud.md`, "Resolved question: the heat percentage"). So 100 % is where `$HeatWarning100;`
//! fires, the bar's second mark (167 %) is where the heat damage starts and its full scale (233 %) where it reaches its most.
//!
//! Numbers: the capacity is the ship's heat record's (B: item data +0x20, +0xF0, 140; coriolis agrees). The radiator's are
//! `ext_radiator_standard`'s record (B, N1: +0x08 the most, 18.15; +0x18, +0x1C the curve's ends, 0 and 1; that the radiator
//! component's +0x1B0 is this record is inferred, its writer 0x1443A6BA0 taking it from a module-data object). The damage's
//! threshold, the fraction of its most and its interval are the heat record's +0xF4, +0xF8, +0x104 (B: 2.5, 3.5, 2 s). Open
//! (`stand_in`): what the radiator's least is, the damage rates' unit, every load's size but the idle's fit.

/// `ext_radiator_standard`'s R+0x08: the radiator's rate at the top of its curve (heat a second, B with the inferred link).
pub const DISSIPATE_MAX: f64 = 18.15;
/// R+0x18, R+0x1C: the heat fractions where the radiator's curve starts and reaches its top (B with the inferred link).
pub const DISSIPATE_F0: f64 = 0.0;
pub const DISSIPATE_F1: f64 = 1.0;
/// The heat record's +0x104: heat damage is dealt every this many seconds (B).
pub const DAMAGE_EVERY_S: f64 = 2.0;
/// The heat record's +0xF4 and +0xF8: the heat fraction where heat damage starts and where its rate reaches its most (B; the
/// exe compares heat / capacity with them, `0x142DC1940`, C). On the HUD 167 % and 233 %, the gauge's second mark and full
/// scale (`f / 1.5`, A).
pub const DAMAGE_F0: f64 = 2.5;
pub const DAMAGE_F1: f64 = 3.5;
/// The heat fraction the HUD shows as 100 % (A: the gauge's message `f / 3.5` against its mark 1.5 / 3.5, FUN_142d6d3e0, and
/// the movie's `int(arg1 / arg2 x 100 + 0.5)`; the same number as `WARNING_FRACTION`, from another constant of the exe).
pub const HUD_FULL_FRACTION: f64 = 1.5;

/// What is open, each kept as the mod has had it until a measurement says what the record's number means.
pub mod stand_in {
    /// The radiator's rate at the bottom of its curve (heat a second): EDSY's `heatdismin` 1.18, which is the record's R+0x00,
    /// a field the rate code (0x144399D90) doesn't read; the field it reads, R+0x04, is 0. Something else may read 1.18 (a
    /// passive loss): kept until N1's H1 run sees whether a cool idle ship still sheds heat.
    pub const DISSIPATE_MIN: f64 = 1.18;
    /// The radiator's multiplier `k` (starts at 1 in the exe's tick; its other values not found).
    pub const DISSIPATE_K: f64 = 1.0;
    /// Heat damage's rates at `DAMAGE_F0` and at `DAMAGE_F1` (a share of the hull a second here). The heat record holds 0.5 and
    /// 1.5 (+0xFC, +0x100), but their unit is not known: the exe hands the summed damage to an object it holds (+0x98, virtual
    /// +0x30) that was not traced, so what 0.5 is (hull points, a share, module damage) is open: kept as the mod has had them.
    pub const DAMAGE_RATE_LO: f64 = 0.0;
    pub const DAMAGE_RATE_HI: f64 = 0.01;
    /// The heat the ship makes idling (MJ a second: the power plant and whatever else runs), the one number that holds session
    /// 2's idle at its measured 36 % (s09, 60 s at rest, 2/2/2 pips, 36 in all 3834 frames; near the system's star, so the
    /// star's heat is in it) with the radiator's numbers above. On the HUD's scale (`f / 1.5`, A) 36 % is f = 0.54, where the
    /// radiator takes 1.18 + (18.15 - 1.18) x 0.54^2 = 6.128 a second; the HUD's whole percent (35.5 to 36.5) bounds it to
    /// 5.99 to 6.27. A stand-in fitted to one measurement (E) until N1's H1 and F1 runs separate the loads. (It was 3.379,
    /// fitted with the percentage taken as heat / capacity, f = 0.36.)
    pub const IDLE_LOAD: f64 = 6.128;
}

/// The radiator's two visual values, which the ship's lights and its vents' flaps read as generic parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisualDissipation {
    /// `HeatRadiator_VisualActiveDissipation`.
    pub active: f64,
    /// `HeatRadiator_VisualPassiveDissipation`.
    pub passive: f64,
}

/// The radiator's visual values from the heat fraction `f` (FUN_144399d90, through FUN_144398f40): active clamp((f - f0) /
/// (1 - f0), 0, 1), passive clamp(f / f1, 0, 1), f0 and f1 the radiator record's +0x18 and +0x1C (`DISSIPATE_F0`,
/// `DISSIPATE_F1`); both 0 while the radiator is off (`radiator` false: unpowered or silent running). Its second condition
/// (vtable +0x70 of the component's +0x158) and the heat sink's term (a ship without one has none) are taken as passing.
pub fn visual_dissipation(f: f64, radiator: bool) -> VisualDissipation {
    let on = |x: f64| if radiator { x.clamp(0.0, 1.0) } else { 0.0 };
    VisualDissipation { active: on((f - DISSIPATE_F0) / (1.0 - DISSIPATE_F0)), passive: on(f / DISSIPATE_F1) }
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
            dmg_f0: DAMAGE_F0,
            dmg_f1: DAMAGE_F1,
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

    /// The HUD's percentage, `f / 1.5 x 100` (A, `HUD_FULL_FRACTION`; the movie rounds it to a whole percent).
    pub fn hud_percent(&self, spec: &HeatSpec) -> f64 {
        self.fraction(spec) / HUD_FULL_FRACTION * 100.0
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

    /// The radiator's visual values (FUN_144399d90): the active one over f0..1, the passive one over 0..f1, both clamped and
    /// both 0 with the radiator off.
    #[test]
    fn visual_dissipation_values() {
        let d = visual_dissipation(0.4, true);
        assert!((d.active - 0.4).abs() < 1e-12 && (d.passive - 0.4).abs() < 1e-12);
        assert_eq!(visual_dissipation(2.0, true), VisualDissipation { active: 1.0, passive: 1.0 });
        assert_eq!(visual_dissipation(-0.5, true), VisualDissipation { active: 0.0, passive: 0.0 });
        assert_eq!(visual_dissipation(0.9, false), VisualDissipation::default());
    }

    /// The warning fires once as the fraction rises through 1.5 (the HUD's 100 %); heat damage starts over the record's 2.5
    /// (the HUD's 167 %) and is dealt on its interval.
    #[test]
    fn warnings_and_damage() {
        let spec = HeatSpec::sidewinder();
        let mut h = Heat::new(&spec);
        h.add(&spec, 1.49 * spec.capacity - h.heat, None);
        assert!(!h.tick(&spec, 0.0, false, false).warning);
        h.add(&spec, 0.02 * spec.capacity, None);
        let ev = h.tick(&spec, 0.0, false, false);
        assert!(ev.warning && !ev.damage_warning);
        assert!((h.hud_percent(&spec) - 100.67).abs() < 0.01);
        assert!(!h.tick(&spec, 0.0, false, false).warning);
        // Under 2.5 no damage, however long.
        assert!((0..300).all(|_| h.tick(&spec, 1.0 / 60.0, false, false).damage == 0.0));
        h.add(&spec, 2.51 * spec.capacity - h.heat, None);
        assert!(h.tick(&spec, 0.0, false, false).damage_warning);
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

    /// The HUD's scale: 100 % at the warning's 1.5, the gauge's marks at 2.5 (167 %) and 3.5 (233 %).
    #[test]
    fn hud_percent_is_heat_over_one_and_a_half_capacities() {
        let spec = HeatSpec::sidewinder();
        let mut h = Heat::new(&spec);
        for (f, pct) in [(0.4, 26.667), (1.5, 100.0), (2.5, 166.667), (3.5, 233.333)] {
            h.heat = f * spec.capacity;
            assert!((h.hud_percent(&spec) - pct).abs() < 1e-3, "{f}: {}", h.hud_percent(&spec));
        }
    }

    /// A powered tick of the ship at rest, in `pilot.rs`'s order: the idle load and `extra` (heat a second), the radiator.
    fn idle_tick(h: &mut Heat, spec: &HeatSpec, extra: f64) {
        let dt = 1.0 / 60.0;
        h.add(spec, (stand_in::IDLE_LOAD + extra) * dt, None);
        h.tick(spec, dt, true, false);
    }

    /// The idle ship settles at session 2's measured 36 % (s09: 36 in every frame of 60 s at rest): within the HUD's whole
    /// percent, 35.5 to 36.5.
    #[test]
    fn the_idle_ship_reads_session_twos_36_percent() {
        let spec = HeatSpec::sidewinder();
        let mut h = Heat::new(&spec);
        for _ in 0..600 * 60 {
            idle_tick(&mut h, &spec, 0.0);
        }
        let pct = h.hud_percent(&spec);
        assert!((35.5..36.5).contains(&pct), "{pct}");
    }

    /// A boost from that idle (1 s warming, 2 s firing at the boost record's 10 a second) peaks at session 2's 44 % (s05 and
    /// s06: 36 or 37 before, 44 at most; with the percentage taken as heat / capacity it would read 49 %).
    #[test]
    fn a_boost_from_idle_peaks_at_session_twos_44_percent() {
        use crate::boost::{Boost, BoostSpec, State};
        let spec = HeatSpec::sidewinder();
        let bs = BoostSpec::sidewinder(0.0);
        let mut h = Heat::new(&spec);
        for _ in 0..600 * 60 {
            idle_tick(&mut h, &spec, 0.0);
        }
        let mut b = Boost { state: State::Warm(0.0) };
        let mut peak: f64 = 0.0;
        for _ in 0..10 * 60 {
            let extra = b.heat_rate(&bs);
            b.tick(&bs, 1.0 / 60.0);
            idle_tick(&mut h, &spec, extra);
            peak = peak.max(h.hud_percent(&spec));
        }
        assert_eq!((peak + 0.5) as i32, 44, "{peak}");
    }
}
