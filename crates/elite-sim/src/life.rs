//! The canopy and the life support (step 2.3, the spec's F6).
//!
//! Rules (grade C, Elite's exe, studied for this step):
//! - The life support's update (`0x142EC3180`) keeps two clocks: how long the canopy has been breached and how long the module
//!   has been without power, each 0 while its cause is absent. Once either reaches its grace (the component's data: +0x18 for
//!   the breach, +0x14 for the power) the emergency oxygen runs down a second a second. At or under zero (`<= 0`), with the canopy
//!   breached or the module unpowered, the commander dies (`0x142ECF1C0`: here the ship is lost). While neither, the exe writes
//!   `max(full, oxygen + rate x dt)` (both the module's data): refilled at once; held at full here, since that `max` could
//!   pass the full time.
//! - The canopy is breached while its own health is at or under 0 (`0x1430C6130`, the cockpit's "Canopy Breached" state).
//! - The HUD's alarm (`0x142D59590`, `SetLifeSupportAlarm`): critical at 30 s or less left, otherwise on while the oxygen
//!   runs (the 30.0 at `0x144DE168C`).
//!
//! Numbers: the full time is 300 s (the stock 1E life support's five minutes, the spec's F6, and its module record: B). The
//! two graces are the LifeSupportComponent template's (B, N1: the prototype `lifesupportmodule`, chosen by its name, since the
//! module item names none). Stand-in (E): the canopy's health and how a hit reaches it weren't found, so the canopy breaks
//! once the hull is under a tenth.

/// The emergency oxygen's full time (s, B).
pub const OXYGEN_S: f64 = 300.0;
/// The alarm turns critical at this much oxygen left or less (s, C: `0x142D59590`).
pub const CRITICAL_S: f64 = 30.0;
/// The graces before the emergency oxygen starts (s, B): the canopy breached (template +0x18, against the breach clock
/// +0x244), the module unpowered (+0x14, against the clock +0x240).
pub const GRACE_BREACHED_S: f64 = 6.5;
pub const GRACE_UNPOWERED_S: f64 = 0.4;

/// Stand-ins (E).
pub mod stand_in {
    /// The hull's share under which the canopy is broken (the canopy's own health is not known; not flown: it needs damage).
    pub const CANOPY_HULL_SHARE: f64 = 0.10;
}

/// The HUD's life support alarm (`0x142D59590`'s three states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alarm {
    Off,
    On,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Life {
    /// The canopy is whole.
    pub canopy: bool,
    /// The emergency oxygen left (s).
    pub oxygen: f64,
    /// The emergency oxygen is running down.
    emergency: bool,
    breached_for: f64,
    unpowered_for: f64,
}

impl Default for Life {
    fn default() -> Self {
        Life { canopy: true, oxygen: OXYGEN_S, emergency: false, breached_for: 0.0, unpowered_for: 0.0 }
    }
}

impl Life {
    /// The hull's share now: under the stand-in's share the canopy breaks (it stays broken until the restock mends it).
    pub fn hull(&mut self, hull_share: f64) {
        if hull_share < stand_in::CANOPY_HULL_SHARE {
            self.canopy = false;
        }
    }

    /// Whether the emergency oxygen is running down (as of the last tick).
    pub fn running(&self) -> bool {
        self.emergency
    }

    /// The HUD's alarm.
    pub fn alarm(&self) -> Alarm {
        match (self.emergency, self.oxygen <= CRITICAL_S) {
            (false, _) => Alarm::Off,
            (true, false) => Alarm::On,
            (true, true) => Alarm::Critical,
        }
    }

    /// A tick, the module powered or not (switched on and the power plant running): true whenever the commander can't live,
    /// the oxygen at or under 0 with the canopy breached or the module unpowered (the exe's test, `0x142EC3180`).
    pub fn tick(&mut self, dt: f64, module_powered: bool) -> bool {
        let breached = !self.canopy;
        self.breached_for = if breached { self.breached_for + dt } else { 0.0 };
        self.unpowered_for = if module_powered { 0.0 } else { self.unpowered_for + dt };
        self.emergency = (breached && self.breached_for >= GRACE_BREACHED_S) || (!module_powered && self.unpowered_for >= GRACE_UNPOWERED_S);
        if self.emergency {
            self.oxygen = (self.oxygen - dt).max(0.0);
        } else {
            // (The exe writes `max(full, oxygen + rate x dt)`, which could pass the full time: held at full here.)
            self.oxygen = self.oxygen.max(OXYGEN_S);
        }
        self.oxygen <= 0.0 && (breached || !module_powered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    /// The ticks until the oxygen runs out (at most 400 s).
    fn until_out(l: &mut Life, powered: bool) -> f64 {
        let mut t = 0.0;
        while !l.tick(DT, powered) {
            t += DT;
            assert!(t < 400.0, "never ran out");
        }
        t + DT
    }

    /// Ticks for `secs` seconds.
    fn run(l: &mut Life, powered: bool, secs: f64) {
        for _ in 0..(secs / DT).round() as usize {
            l.tick(DT, powered);
        }
    }

    /// A hull under a tenth breaks the canopy; after the breach's 6.5 s grace the oxygen lasts its 300 s, a second a second,
    /// and its end is reported once.
    #[test]
    fn a_breach_runs_the_oxygen_down() {
        let mut l = Life::default();
        l.hull(0.5);
        assert!(l.canopy && !l.tick(DT, true) && !l.running() && l.oxygen == OXYGEN_S);
        l.hull(0.05);
        assert!(!l.canopy);
        let t = until_out(&mut l, true);
        assert!((t - GRACE_BREACHED_S - OXYGEN_S).abs() < 2.0 * DT, "{t}");
        assert!(l.oxygen == 0.0 && l.running());
    }

    /// The graces (the template's 6.5 s breached, 0.4 s unpowered): the oxygen is held full until one is over.
    #[test]
    fn the_graces() {
        assert_eq!((GRACE_BREACHED_S, GRACE_UNPOWERED_S), (6.5, 0.4));
        let mut l = Life { canopy: false, ..Life::default() };
        run(&mut l, true, 6.4);
        assert!(!l.running() && l.oxygen == OXYGEN_S);
        run(&mut l, true, 0.2);
        assert!(l.running());
        let mut m = Life::default();
        run(&mut m, false, 0.35);
        assert!(!m.running());
        run(&mut m, false, 0.1);
        assert!(m.running());
    }

    /// At or under 0 with a breach (or without power) the commander is lost on every tick, not only the one that crosses 0;
    /// a whole canopy with power refills instead.
    #[test]
    fn lost_at_zero() {
        let mut l = Life { canopy: false, ..Life::default() };
        run(&mut l, true, GRACE_BREACHED_S + 0.1);
        l.oxygen = 0.0;
        assert!(l.tick(DT, true) && l.tick(DT, true) && l.tick(0.0, true));
        let mut m = Life::default();
        run(&mut m, false, GRACE_UNPOWERED_S + 0.1);
        m.oxygen = 0.0;
        assert!(m.tick(DT, false));
        assert!(!m.tick(DT, true) && m.oxygen == OXYGEN_S);
    }

    /// The module without power runs it down too; with power back and the canopy whole it is full again at once (the exe's
    /// `max`); a broken canopy keeps it running whatever the power.
    #[test]
    fn power_and_the_refill() {
        let mut l = Life::default();
        run(&mut l, false, 10.0);
        assert!(l.running() && (l.oxygen - (OXYGEN_S - 10.0 + GRACE_UNPOWERED_S)).abs() < 0.05, "{}", l.oxygen);
        l.tick(DT, true);
        assert!(!l.running() && l.oxygen == OXYGEN_S);
        l.canopy = false;
        run(&mut l, true, 10.0);
        assert!(l.running() && (l.oxygen - (OXYGEN_S - 10.0 + GRACE_BREACHED_S)).abs() < 0.05, "{}", l.oxygen);
        assert_eq!(Life::default().alarm(), Alarm::Off);
    }

    /// The alarm: on while the oxygen runs, critical from 30 s left.
    #[test]
    fn the_alarm() {
        let mut l = Life { canopy: false, ..Life::default() };
        run(&mut l, true, GRACE_BREACHED_S + 0.1);
        assert_eq!(l.alarm(), Alarm::On);
        l.oxygen = CRITICAL_S + DT / 2.0;
        l.tick(DT, true);
        assert_eq!(l.alarm(), Alarm::Critical);
        assert!(until_out(&mut l, true) < CRITICAL_S + 2.0 * DT);
    }
}
