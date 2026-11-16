//! Silent running (step 2.2): `ToggleButtonUpInput`, Delete in Elite's default file (a toggle: the file's `ToggleOn`).
//!
//! Rules:
//! - The radiator stops (grade C): its rate is 0 while byte +0xA5 of the ship object it holds is set (`0x144399D90`, A20),
//!   and the cockpit's button for `ToggleButtonUpInput` shows that same byte (`0x142D0CD20`), so the switch is this key's.
//!   `Heat::tick`'s `silent` is that switch.
//! - The shield generator loses its power and collapses as an unpowered one does, coming back by the broken shield's rule
//!   once the switch is off (`shields.rs`). B: Elite's own description of silent running; its link in the exe wasn't found.

/// The shield generator's power with silent running on or off.
pub fn shields_powered(generator_powered: bool, silent: bool) -> bool {
    generator_powered && !silent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heat::{Heat, HeatSpec};
    use crate::shields::{self, Shields};

    /// A second of the idle ship: its load, the radiator and the shields.
    fn second(h: &mut Heat, s: &mut Shields, spec: &HeatSpec, silent: bool) {
        for _ in 0..60 {
            h.add(spec, crate::heat::stand_in::IDLE_LOAD / 60.0, None);
            h.tick(spec, 1.0 / 60.0, true, silent);
            s.tick(1.0 / 60.0, shields_powered(true, silent), 0.5);
        }
    }

    /// On: the shields drop at once and the heat climbs on the idle load alone; off: the heat comes down and the shields
    /// rebuild after the broken shield's delay, counted from the power's return.
    #[test]
    fn silent_running_holds_the_heat_and_drops_the_shields() {
        let spec = HeatSpec::sidewinder();
        let mut h = Heat::new(&spec);
        let mut s = Shields::sidewinder(crate::data::JOURNAL_UNLADEN_MASS + 2.3);
        second(&mut h, &mut s, &spec, false);
        let (cool, full) = (h.fraction(&spec), s.strength);
        assert!(s.up && full > 0.0);
        second(&mut h, &mut s, &spec, true);
        assert!(!s.up && s.strength == 0.0);
        for _ in 0..19 {
            second(&mut h, &mut s, &spec, true);
        }
        let hot = h.fraction(&spec);
        assert!(hot > cool + 0.2, "{cool} -> {hot}");
        for _ in 0..(shields::stand_in::DELAY_BROKEN_S as usize - 1) {
            second(&mut h, &mut s, &spec, false);
        }
        assert!(h.fraction(&spec) < hot && s.strength == 0.0, "the broken shield's delay counts from the power's return");
        for _ in 0..120 {
            second(&mut h, &mut s, &spec, false);
        }
        assert!(s.up && s.strength == full);
    }
}
