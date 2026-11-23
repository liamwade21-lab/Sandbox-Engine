//! The top panel, `cockpitinfopanel` (`re/elite/notes/ui_messages_hud.md`, its section; the alarm state machine
//! `FUN_142d59590`). What is sent: flight assist off, the drive's charge bar, the life support alarm and its countdown,
//! the clock.
//!
//! Not sent, and why: `AddMessage` (each message's priority and object fields were not decoded, E); the proximity and
//! impact alarms and the danger marker (the mod has no ship alarm component); the countdown, red and blue central
//! warnings (which drive phase sends which, and their labels, were not decoded, E); `SetSystemInfo` (no jump destination
//! data); `SetHeadlightState` (which of the icon's three frames is which was not read, E); `SetClockVisible` (the option's
//! default was not found, E); `SetShipName` (the ship has no name); the scanner, mining and test messages (not a lone
//! flight's).

use super::{changed, Movie, Outbox};
use crate::facts::LifeAlarm;
use crate::{UiFacts, Value};

const MOVIE: Movie = Movie::CockpitInfoPanel;

/// The central warning's types (A: the movie's `HyperspaceTimer`): off, and the charge bar.
const WARNING_OFF: i32 = 0;
const WARNING_CHARGE_BAR: i32 = 1;

/// The info panel's feeder.
#[derive(Clone, Debug)]
pub struct Info {
    flight: Option<i32>,
    /// The central warning as the movie last had it: off when it was just loaded (its handler reacts only to a change,
    /// and turning it off plays the warning's end sound, so the off state is not sent to a fresh movie).
    warning: Option<(i32, Option<f64>)>,
    life: Option<i32>,
    oxygen: Option<f64>,
    clock: Option<u32>,
}

impl Default for Info {
    fn default() -> Self {
        Info { flight: None, warning: Some((WARNING_OFF, None)), life: None, oxygen: None, clock: None }
    }
}

impl Info {
    pub fn feed(&mut self, facts: &UiFacts, _dt: f64, out: &mut Outbox) {
        // `SetFlightAlert(state)`: 1 while flight assist is off, on change (B: `FUN_142d59590`, cache +0x17c).
        let flight = i32::from(!facts.speed.flight_assist);
        if changed(&mut self.flight, flight) {
            out.send(MOVIE, "SetFlightAlert", vec![flight.into()]);
        }

        // `SetCentralWarning(type, value, label)`: the charge bar (type 1, value 0..1) while the frame shift drive charges,
        // off (type 0) after (B: the notes' "the charging FSD bar"; types and values A, the movie). The label the exe sends
        // was not decoded (E): sent empty (stand-in). Sent when the type or the value changes.
        let warning = match facts.drive.charge {
            Some(share) => (WARNING_CHARGE_BAR, Some(share)),
            None => (WARNING_OFF, None),
        };
        if changed(&mut self.warning, warning) {
            out.send(MOVIE, "SetCentralWarning", vec![warning.0.into(), warning.1.into(), "".into()]);
        }

        // `SetLifeSupportAlarm(state)` on change, and `SetLifeSupportFailureIn(seconds)` while it is on, when the oxygen
        // left changes (B: `FUN_142d59590`, caches; the mod's alarm is that function's rule, `elite_sim::life`).
        let alarm = match facts.alerts.life_support {
            LifeAlarm::Off => 0,
            LifeAlarm::On => 1,
            LifeAlarm::Critical => 2,
        };
        if changed(&mut self.life, alarm) {
            out.send(MOVIE, "SetLifeSupportAlarm", vec![alarm.into()]);
        }
        if alarm == 0 {
            self.oxygen = None;
        } else if changed(&mut self.oxygen, facts.alerts.oxygen_s) {
            out.send(MOVIE, "SetLifeSupportFailureIn", vec![facts.alerts.oxygen_s.into()]);
        }

        // `SetTime(seconds since midnight)` each time the clock's whole second changes (B: "every second").
        if let Some(t) = facts.clock.time_of_day_s {
            let second = t.max(0.0).floor() as u32;
            if changed(&mut self.clock, second) {
                out.send(MOVIE, "SetTime", vec![Value::from(second)]);
            }
        }
    }

    /// The movie has just been loaded: everything is sent again (not for boarding: see `feed`).
    pub fn reset(&mut self) {
        *self = Info::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(info: &mut Info, facts: &UiFacts) -> Vec<&'static str> {
        let mut out = Outbox::default();
        info.feed(facts, 0.0, &mut out);
        out.msgs.iter().map(|m| m.name).collect()
    }

    #[test]
    fn alarms_on_change() {
        let mut info = Info::default();
        let mut facts = UiFacts::default();
        facts.speed.flight_assist = true;
        assert_eq!(names(&mut info, &facts), ["SetFlightAlert", "SetLifeSupportAlarm"], "the warning stays off unsent");
        assert!(names(&mut info, &facts).is_empty());
        facts.speed.flight_assist = false;
        let mut out = Outbox::default();
        info.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs[0].args, vec![Value::from(1)]);
    }

    #[test]
    fn charge_bar() {
        let mut info = Info::default();
        let mut facts = UiFacts::default();
        names(&mut info, &facts);
        facts.drive.charge = Some(0.25);
        let mut out = Outbox::default();
        info.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs[0].name, "SetCentralWarning");
        assert_eq!(out.msgs[0].args, vec![Value::from(1), 0.25.into(), "".into()]);
        facts.drive.charge = None;
        let mut out = Outbox::default();
        info.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs[0].args, vec![Value::from(0), Value::Null, "".into()]);
    }

    #[test]
    fn life_support_and_clock() {
        let mut info = Info::default();
        let mut facts = UiFacts::default();
        facts.alerts.life_support = LifeAlarm::On;
        facts.alerts.oxygen_s = 120.5;
        facts.clock.time_of_day_s = Some(3725.4);
        let mut out = Outbox::default();
        info.feed(&facts, 0.0, &mut out);
        let sent: Vec<(&str, Vec<Value>)> = out.msgs.iter().map(|m| (m.name, m.args.clone())).collect();
        assert!(sent.contains(&("SetLifeSupportAlarm", vec![Value::from(1)])));
        assert!(sent.contains(&("SetLifeSupportFailureIn", vec![Value::from(120.5)])));
        assert!(sent.contains(&("SetTime", vec![Value::from(3725u32)])));
        facts.clock.time_of_day_s = Some(3725.9);
        assert!(names(&mut info, &facts).is_empty(), "the same second");
        facts.clock.time_of_day_s = Some(3726.0);
        facts.alerts.oxygen_s = 119.5;
        assert_eq!(names(&mut info, &facts), ["SetLifeSupportFailureIn", "SetTime"]);
        facts.alerts.life_support = LifeAlarm::Off;
        assert_eq!(names(&mut info, &facts), ["SetLifeSupportAlarm"], "no countdown once off");
    }
}
