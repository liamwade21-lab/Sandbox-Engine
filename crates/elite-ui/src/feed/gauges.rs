//! The lower HUD, `cockpitgaugepanel` (`re/elite/notes/ui_messages_hud.md`, its section; the exe's cockpit HUD controller
//! `FUN_142d2a7c0` and its per-frame senders). What is sent here: the cockpit's set-up, heat, the speed gauge (normal
//! flight), shields and hull, the distributor, the signature's silent clip, the compass, the locked destination's panel,
//! fuel and the status plates.
//!
//! Not sent, and why: the speed gauge in supercruise (that branch's bar and demand were not decoded, E);
//! `SetHyperspaceThrottle` (its text was not decoded, E); `SetCruiseInfo` / `ClearCruiseInfo` (the distance bar's scale
//! function and limits were not decoded, E); `SetLocationInfo` / `SetLocationPanelVisibility` (no "where am I" data in
//! the mod; the visibility's sender not found, E); the target, docking, multicrew, buffs, mining, scoop canister and
//! proving-ground messages (not needed by a lone flight: the notes' section 2).

use super::{changed, moved, Movie, Outbox};
use crate::{UiFacts, Value};

const MOVIE: Movie = Movie::CockpitGaugePanel;

/// `SetHeat` is heatProp over 3.5, with 1.5 / 3.5 (100 % on the label, the damage zone) and 2.5 / 3.5 (the bar's last
/// segment) (A: `FUN_142d6d3e0`, DAT_1451daf1c 3.5, DAT_145324d78, DAT_145324d7c).
const HEAT_SCALE: f64 = 3.5;
const HEAT_WARN: f64 = 1.5 / HEAT_SCALE;
const HEAT_HULL: f64 = 2.5 / HEAT_SCALE;
/// Heat is re-sent once heatProp moved by more than this from what was sent (A: DAT_1451ab540).
const HEAT_STEP: f64 = 0.002;
/// The blue band's centre is half the top-speed mark in normal flight (B: `FUN_142d58ac0`, DAT_144de28b8 0.5).
const OPTIMAL_SHARE: f64 = 0.5;
/// The bar shows reverse when the velocity along the nose is at or under this (m/s; B: DAT_145324cc0 -0.05).
const REVERSE_AT: f64 = -0.05;
/// The signature is re-sent once a channel moved by more than this (B: `FUN_142d640c0`, DAT_144e299a4 0.01).
const SIGNATURE_STEP: f64 = 0.01;
/// Fuel usage: the burn rate in t/s times 3600 (A: DAT_144e34100), smoothed by 0.07 of the gap a frame (A:
/// DAT_145188438), re-sent once the smoothed value moved by more than 0.005 (A: `FUN_142d67440`).
const HOUR_S: f64 = 3600.0;
const USAGE_SMOOTHING: f64 = 0.07;
const USAGE_STEP: f64 = 0.005;
/// The status plates' labels, pushed in this order (B: `FUN_142d81990`).
const MASS_LOCK_LABEL: &str = "$GAUGE_PANEL_mass_lock_warning;";
const GEAR_LABEL: &str = "$GAUGE_PANEL_landing_gear;";
const SCOOP_LABEL: &str = "$GAUGE_PANEL_cargo_scoop;";
/// A station's status "None" (A: the movies' `LocationType.eStationStatus_none`).
const STATION_STATUS_NONE: i32 = 0;

/// The speed gauge's values as last sent (the exe compares speed, max, optimal, the flag and desired: B).
#[derive(Clone, Debug, PartialEq)]
struct Throttle {
    speed_prop: f64,
    speed_m_s: f64,
    max: f64,
    optimal: f64,
    desired_visible: bool,
    desired: f64,
}

/// The gauge panel's feeder.
#[derive(Clone, Debug, Default)]
pub struct Gauges {
    configured: bool,
    heat: Option<f64>,
    throttle: Option<Throttle>,
    damage: Option<(f64, i32, f64)>,
    power: Option<(crate::facts::Channels, crate::facts::Channels)>,
    signature: Option<(f64, f64, bool)>,
    compass: Option<bool>,
    compass_target: Option<(i32, String)>,
    target: Option<(i32, String, String)>,
    fuel: Option<[f64; 5]>,
    usage_sent: Option<f64>,
    /// The smoothed fuel usage (t/h): the ship's, so `reset` keeps it.
    usage: f64,
    status: Option<(bool, bool, bool, bool, f64)>,
}

impl Gauges {
    pub fn feed(&mut self, facts: &UiFacts, _dt: f64, out: &mut Outbox) {
        self.configure(out);
        self.heat(facts, out);
        self.throttle(facts, out);
        self.damage(facts, out);
        self.power(facts, out);
        self.signature(facts, out);
        self.compass(facts, out);
        self.target(facts, out);
        self.fuel(facts, out);
        self.status(facts, out);
    }

    /// The movie has just been loaded: everything is sent again (not for boarding: see `feed`).
    pub fn reset(&mut self) {
        *self = Gauges { usage: self.usage, ..Gauges::default() };
    }

    /// The cockpit's set-up (B: `FUN_142d03b10` / `FUN_142d03880`): the normal cockpit (type 0) with the fuel bar shown,
    /// then the lone clean ship's fixed state (the coordinator's decision of 2026-11-23, traced in the movie's `Content`),
    /// so nothing authored shows where the mod has nothing to say:
    /// - `SetLocalLegalStatus(0, false, false, " ")` (`setLegalStatus`, 4 parameters, the last optional): status 0 puts
    ///   both legal plates on their "disabled" frame, false hides the icon, and the text is written only when it is not
    ///   empty, so one space blanks the frame's authored text. Elite's own text for a clean ship was not decoded (its
    ///   sender is E): stand-in.
    /// - `SetLocationPanelVisibility(false)` (`LocationPanel.setVisible`, 1 parameter): the location panel is hidden.
    ///   Elite fills it with the place's name, faction, system, government and economy; the mod has no factions,
    ///   governments or economies and the fill rule was not decoded, so it is hidden rather than filled: a stated
    ///   difference from Elite.
    /// - `SetVesselStatusVisible(false)` (`VesselStatus.SetVisible`, 1 parameter): the fighter or crew vessel's status
    ///   is hidden, the lone ship's state.
    fn configure(&mut self, out: &mut Outbox) {
        if !self.configured {
            out.send(MOVIE, "ConfigureCockpit", vec![0.into()]);
            out.send(MOVIE, "ShowFuelLevels", vec![]);
            out.send(MOVIE, "SetLocalLegalStatus", vec![0.into(), false.into(), false.into(), " ".into()]);
            out.send(MOVIE, "SetLocationPanelVisibility", vec![false.into()]);
            out.send(MOVIE, "SetVesselStatusVisible", vec![false.into()]);
            self.configured = true;
        }
    }

    fn heat(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let prop = facts.heat.prop;
        if moved(&mut self.heat, prop, HEAT_STEP) {
            out.send(MOVIE, "SetHeat", vec![(prop / HEAT_SCALE).into(), HEAT_WARN.into(), HEAT_HULL.into()]);
        }
    }

    /// Normal flight's branch of `FUN_142d58ac0` (B): max = the pips' top over the full-power top ([0x750] / [0x81c]),
    /// optimal = 0.5 max, desired = the demand x max, shown with flight assist on, the bar = |v| / the full-power top
    /// (negative in reverse), the text `$UNIT:#value=<|v|>;`. The three messages go together, when any value changed.
    /// With the engine unpowered ([0x910] 0) the exe's quotient has no value; 0 is sent (stand-in). A full-power top
    /// that is not above 0 sends 0 marks (ours: no division by 0).
    fn throttle(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let s = &facts.speed;
        if s.supercruise {
            return;
        }
        let max = if s.full_top_m_s > 0.0 { s.top_m_s / s.full_top_m_s } else { 0.0 };
        let sign = if s.forward_m_s <= REVERSE_AT { -1.0 } else { 1.0 };
        let speed_prop = if s.engines_on && s.full_top_m_s > 0.0 { sign * s.speed_m_s / s.full_top_m_s } else { 0.0 };
        let now = Throttle {
            speed_prop,
            speed_m_s: s.speed_m_s,
            max,
            optimal: max * OPTIMAL_SHARE,
            desired_visible: s.flight_assist,
            desired: s.demand * max,
        };
        if !changed(&mut self.throttle, now.clone()) {
            return;
        }
        let forward = now.desired >= 0.0;
        let throttle = Value::object([
            ("speedProp", Value::from(now.speed_prop)),
            ("speedString", format!("$UNIT:#value={};", now.speed_m_s).into()),
            ("optimal", now.optimal.into()),
            ("desiredVisible", now.desired_visible.into()),
            ("desired", now.desired.into()),
        ]);
        out.send(MOVIE, "SetThrottle", vec![throttle]);
        out.send(MOVIE, "SetThrottleMaxSpeed", vec![now.max.into(), forward.into()]);
        out.send(MOVIE, "SetThrottleOptimal", vec![now.optimal.into(), forward.into()]);
    }

    /// `SetDamage` (A for the fields, B for the sender `FUN_142d5d1e0`): the Sidewinder's one shield zone as [strength,
    /// state], the plain sub-state 0 (the module is not a scout's); the hull with type 0 (no caustic or Thargoid damage)
    /// and pulse rate 0 (caustic only). Sent when any of them changed.
    fn damage(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let d = &facts.damage;
        let now = (d.shield.strength, d.shield.state.code(), d.hull);
        if !changed(&mut self.damage, now) {
            return;
        }
        let shields = Value::object([
            ("shieldsPercents", Value::array([Value::from(now.0), now.1.into()])),
            ("shieldSubState", 0.into()),
        ]);
        let hull = Value::object([("hullPercent", Value::from(now.2)), ("hullType", 0.into()), ("pulseRate", 0.into())]);
        out.send(MOVIE, "SetDamage", vec![shields, hull]);
    }

    /// `SetPowerLevels(wep, eng, sys)` (the capacitors' charge / capacity) and `SetPowerAssignment` (each channel's pips /
    /// most, extra pips 0), sent together when any value changed (B: `FUN_142d7e520`, one cache for both); the pip keys'
    /// flashes on every press (B: the same sender).
    fn power(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let p = &facts.power;
        if changed(&mut self.power, (p.capacitors, p.pips)) {
            let c = p.capacitors;
            out.send(MOVIE, "SetPowerLevels", vec![c.wep.into(), c.eng.into(), c.sys.into()]);
            let assignment = Value::object([
                ("distEng", Value::from(p.pips.eng)),
                ("engExtraPip", 0.into()),
                ("distWep", p.pips.wep.into()),
                ("wepExtraPip", 0.into()),
                ("distSys", p.pips.sys.into()),
                ("sysExtraPip", 0.into()),
            ]);
            out.send(MOVIE, "SetPowerAssignment", vec![assignment]);
        }
        let keys = p.keys;
        for (pressed, name) in [
            (keys.eng, "IndicateEnginePowerInput"),
            (keys.sys, "IndicateSystemPowerInput"),
            (keys.wep, "IndicateWeaponPowerInput"),
            (keys.reset, "IndicateResetPowerInput"),
        ] {
            if pressed {
                out.send(MOVIE, name, vec![]);
            }
        }
    }

    /// `SetSignature(a, b, silent)` (B: `FUN_142d640c0`): re-sent once a channel moved by more than 0.01 or silent running
    /// changed. The channels have no source in the mod (E), so without them it carries only silent running: (0, 0, silent)
    /// on the first feed and whenever silent running changes, so the bar shows empty (stand-in: the mod has no signature)
    /// and the silent clip shows while silent (A: the movie hides the bar then). Unsent, the movie keeps its authored
    /// placeholder, the silent clip, whatever the ship does (seen in the proof's pictures).
    fn signature(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let sig = &facts.signature;
        let now = match sig.channels {
            Some([a, b]) => {
                let moved = self.signature.is_none_or(|(la, lb, ls)| {
                    (a - la).abs() > SIGNATURE_STEP || (b - lb).abs() > SIGNATURE_STEP || ls != sig.silent
                });
                if !moved {
                    return;
                }
                (a, b, sig.silent)
            }
            None => {
                if self.signature.is_some_and(|(_, _, s)| s == sig.silent) {
                    return;
                }
                (0.0, 0.0, sig.silent)
            }
        };
        self.signature = Some(now);
        out.send(MOVIE, "SetSignature", vec![now.0.into(), now.1.into(), now.2.into()]);
    }

    /// The compass (B: `FUN_142d5cf70`, the mode rule of `FUN_142d16b70`): the mod's only thing to point at is the locked
    /// destination (mode 5, its flag clear), else none (mode 1); a destination whose direction is the zero vector has no
    /// compass. `SetCompassVisible(mode != 1)` when the mode changes; `SetCompassData({x, y, z, w}, highlight, unused)` every
    /// frame while shown, highlight false (mode 5) and the unused flag false, w 0 (the movie reads only x, y and z:
    /// stand-in); `HighlightCompass` when it becomes shown or the destination changes (B: "becomes visible or the target
    /// changes").
    fn compass(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let toward = facts.destination.as_ref().filter(|d| d.dir != [0.0; 3]);
        let became = changed(&mut self.compass, toward.is_some());
        if became {
            out.send(MOVIE, "SetCompassVisible", vec![toward.is_some().into()]);
        }
        let Some(d) = toward else {
            self.compass_target = None;
            return;
        };
        let [x, y, z] = d.dir;
        let dir = Value::object([("x", Value::from(x)), ("y", y.into()), ("z", z.into()), ("w", 0.into())]);
        out.send(MOVIE, "SetCompassData", vec![dir, false.into(), false.into()]);
        let retargeted = changed(&mut self.compass_target, (d.kind, d.name.clone()));
        if became || retargeted {
            out.send(MOVIE, "HighlightCompass", vec![]);
        }
    }

    /// The locked destination's panel (B: `FUN_142d717e0`): `SetTargetLocation(target)` when any of its values changed,
    /// `ClearTargetLocation` when the lock goes. The movie's `CruisePanel.updateDisplay` reads, on its normal-flight path,
    /// `detailOne`, `detailTwo`, `scanInProgress`, `type`, `stationStatus`, `locationName` and `distance` in that order and
    /// stops at the first that is undefined, so every one is sent: the mod has no detail lines, scan or station, so they get
    /// the movie's own empty state (empty texts, false, status 0 "None": A, `LocationType`). The distance text's form was not
    /// decoded (E); it is sent as `$DISTANCE:#value=<m>;`, the form of the exe's distance builder `FUN_144648d00` for the
    /// HUD's other distances (stand-in).
    fn target(&mut self, facts: &UiFacts, out: &mut Outbox) {
        match &facts.destination {
            Some(d) => {
                let now = (d.kind, d.name.clone(), format!("$DISTANCE:#value={};", d.distance_m));
                if changed(&mut self.target, now.clone()) {
                    let target = Value::object([
                        ("type", Value::from(now.0)),
                        ("locationName", now.1.into()),
                        ("distance", now.2.into()),
                        ("stationStatus", STATION_STATUS_NONE.into()),
                        ("detailOne", "".into()),
                        ("detailTwo", "".into()),
                        ("scanInProgress", false.into()),
                    ]);
                    out.send(MOVIE, "SetTargetLocation", vec![target]);
                }
            }
            None => {
                if self.target.take().is_some() {
                    out.send(MOVIE, "ClearTargetLocation", vec![]);
                }
            }
        }
    }

    /// `SetFuelLevels(main, reserve, capacity, chunk, cost, usage)` (A: `FUN_142d67440`): main and reserve as level /
    /// capacity floored to 1 % (0 with no capacity), the main tank's capacity, the reservoir's capacity (one bar segment),
    /// the jump's fuel (0 with none), `$FUEL_USAGE_PER_HOUR:#value=<t/h>;` of the smoothed usage. Sent when any of the first
    /// five changed or the smoothed usage moved by more than 0.005 from what was sent.
    fn fuel(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let f = &facts.fuel;
        let share = |level: f64, capacity: f64| if capacity > 0.0 { (level / capacity * 100.0).floor() * 0.01 } else { 0.0 };
        self.usage += (f.burn_t_per_s * HOUR_S - self.usage) * USAGE_SMOOTHING;
        let levels = [
            share(f.main_t, f.main_capacity_t),
            share(f.reservoir_t, f.reservoir_capacity_t),
            f.main_capacity_t,
            f.reservoir_capacity_t,
            f.jump_cost_t.unwrap_or(0.0),
        ];
        let usage_moved = self.usage_sent.is_none_or(|u| (self.usage - u).abs() > USAGE_STEP);
        if self.fuel.as_ref() == Some(&levels) && !usage_moved {
            return;
        }
        self.fuel = Some(levels);
        self.usage_sent = Some(self.usage);
        let mut args: Vec<Value> = levels.iter().map(|&v| v.into()).collect();
        args.push(format!("$FUEL_USAGE_PER_HOUR:#value={};", self.usage).into());
        out.send(MOVIE, "SetFuelLevels", args);
    }

    /// `SetStatusIndicators` (B: `FUN_142d583f0` -> `FUN_142d81990`): the three plates in the exe's order (mass locked,
    /// landing gear, cargo scoop), each {label, enabled}, and the cooldown {visible, prop} (prop 0 while not cooling:
    /// stand-in, the value then was not decoded). Sent when a flag or the cooldown changed.
    fn status(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let st = &facts.status;
        let cooldown = facts.drive.cooldown;
        let now = (st.mass_locked, st.gear_down, st.cargo_scoop, cooldown.is_some(), cooldown.unwrap_or(0.0));
        if !changed(&mut self.status, now) {
            return;
        }
        let plate = |label: &str, enabled: bool| Value::object([("label", Value::from(label)), ("enabled", enabled.into())]);
        let indicators = Value::object([
            ("first", plate(MASS_LOCK_LABEL, now.0)),
            ("second", plate(GEAR_LABEL, now.1)),
            ("third", plate(SCOOP_LABEL, now.2)),
            ("cooldown", Value::object([("visible", Value::from(now.3)), ("prop", now.4.into())])),
        ]);
        out.send(MOVIE, "SetStatusIndicators", vec![indicators]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Channels, Destination, ShieldState};

    fn feed(g: &mut Gauges, facts: &UiFacts) -> Vec<super::super::Msg> {
        let mut out = Outbox::default();
        g.feed(facts, 1.0 / 60.0, &mut out);
        out.msgs
    }

    fn find<'a>(msgs: &'a [super::super::Msg], name: &str) -> Option<&'a super::super::Msg> {
        msgs.iter().find(|m| m.name == name)
    }

    /// The first argument of the named message this feed sends.
    fn first_arg(g: &mut Gauges, facts: &UiFacts, name: &str) -> Value {
        feed(g, facts).into_iter().find(|m| m.name == name).expect(name).args.remove(0)
    }

    fn num(v: &Value) -> f64 {
        v.as_f64().expect("a number")
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} against {b}");
    }

    /// Two ENG pips: the 160 m/s top of the 220 m/s full block (the notes' Sidewinder numbers).
    fn flying() -> UiFacts {
        let mut f = UiFacts::default();
        f.status.seated = true;
        f.speed = crate::facts::Speed {
            supercruise: false,
            speed_m_s: 160.86,
            forward_m_s: 160.86,
            top_m_s: 160.0,
            full_top_m_s: 220.0,
            engines_on: true,
            demand: 1.0,
            flight_assist: true,
        };
        f.heat.prop = 1.5;
        f
    }

    #[test]
    fn first_feed_configures_and_sends_all() {
        let mut g = Gauges::default();
        let msgs = feed(&mut g, &flying());
        let names: Vec<&str> = msgs.iter().map(|m| m.name).collect();
        assert_eq!(
            &names[..5],
            ["ConfigureCockpit", "ShowFuelLevels", "SetLocalLegalStatus", "SetLocationPanelVisibility", "SetVesselStatusVisible"]
        );
        assert_eq!(msgs[2].args, vec![Value::from(0), false.into(), false.into(), " ".into()]);
        for n in ["SetHeat", "SetThrottle", "SetThrottleMaxSpeed", "SetThrottleOptimal", "SetDamage", "SetPowerLevels", "SetPowerAssignment", "SetCompassVisible", "SetFuelLevels", "SetStatusIndicators"] {
            assert!(names.contains(&n), "{n} on the first feed");
        }
        let sig = find(&msgs, "SetSignature").expect("the empty bar on the first feed");
        assert_eq!(sig.args, vec![Value::from(0.0), 0.0.into(), false.into()]);
        assert!(feed(&mut g, &flying()).is_empty(), "nothing changed: nothing sent");
        g.reset();
        let again = feed(&mut g, &flying());
        assert_eq!(again.len(), msgs.len(), "everything again after reset");
    }

    #[test]
    fn heat_scale_and_threshold() {
        let mut g = Gauges::default();
        let msgs = feed(&mut g, &flying());
        let heat = find(&msgs, "SetHeat").unwrap();
        close(num(&heat.args[0]), 1.5 / 3.5);
        close(num(&heat.args[1]), 0.428_571_428_571);
        close(num(&heat.args[2]), 0.714_285_714_285);
        let mut f = flying();
        f.heat.prop = 1.5 + 0.0015;
        assert!(find(&feed(&mut g, &f), "SetHeat").is_none(), "under 0.002 from what was sent");
        f.heat.prop = 1.5 + 0.0025;
        assert!(find(&feed(&mut g, &f), "SetHeat").is_some(), "past 0.002");
    }

    #[test]
    fn throttle_at_two_engine_pips() {
        let mut g = Gauges::default();
        let msgs = feed(&mut g, &flying());
        let t = &find(&msgs, "SetThrottle").unwrap().args[0];
        let max = 160.0 / 220.0;
        close(num(t.get("speedProp").unwrap()), 160.86 / 220.0);
        assert_eq!(t.get("speedString").unwrap().as_str(), Some("$UNIT:#value=160.86;"));
        close(num(t.get("optimal").unwrap()), 0.5 * max);
        assert_eq!(t.get("desiredVisible").unwrap().as_bool(), Some(true));
        close(num(t.get("desired").unwrap()), max);
        let m = find(&msgs, "SetThrottleMaxSpeed").unwrap();
        close(num(&m.args[0]), max);
        assert_eq!(m.args[1].as_bool(), Some(true));
        let o = find(&msgs, "SetThrottleOptimal").unwrap();
        close(num(&o.args[0]), 0.5 * max);
        // Reverse at half demand: the bar goes negative, the marks mirror.
        let mut f = flying();
        f.speed.forward_m_s = -10.0;
        f.speed.speed_m_s = 10.0;
        f.speed.demand = -0.5;
        f.speed.flight_assist = false;
        let msgs = feed(&mut g, &f);
        let t = &find(&msgs, "SetThrottle").unwrap().args[0];
        close(num(t.get("speedProp").unwrap()), -10.0 / 220.0);
        close(num(t.get("desired").unwrap()), -0.5 * max);
        assert_eq!(t.get("desiredVisible").unwrap().as_bool(), Some(false));
        assert_eq!(find(&msgs, "SetThrottleMaxSpeed").unwrap().args[1].as_bool(), Some(false));
        // -0.05 m/s along the nose is still forward's edge: reverse at or under it.
        f.speed.forward_m_s = -0.04;
        let t = &first_arg(&mut g, &f, "SetThrottle");
        assert!(num(t.get("speedProp").unwrap()) > 0.0);
        // Supercruise: not sent.
        f.speed.supercruise = true;
        f.speed.speed_m_s = 1.0e6;
        assert!(find(&feed(&mut g, &f), "SetThrottle").is_none());
    }

    #[test]
    fn damage_and_power() {
        let mut g = Gauges::default();
        let mut f = flying();
        f.damage.shield.strength = 0.4;
        f.damage.shield.state = ShieldState::Recharging;
        f.damage.hull = 0.75;
        f.power.pips = Channels { wep: 4.0 / 8.0, eng: 6.0 / 8.0, sys: 2.0 / 8.0 };
        f.power.capacitors = Channels { wep: 1.0, eng: 0.5, sys: 0.25 };
        let msgs = feed(&mut g, &f);
        let d = find(&msgs, "SetDamage").unwrap();
        assert_eq!(d.args[0].get("shieldsPercents"), Some(&Value::array([Value::from(0.4), 2.into()])));
        assert_eq!(d.args[1].get("hullPercent").and_then(Value::as_f64), Some(0.75));
        let l = find(&msgs, "SetPowerLevels").unwrap();
        assert_eq!(l.args, vec![Value::from(1.0), 0.5.into(), 0.25.into()], "WEP, ENG, SYS");
        let a = &find(&msgs, "SetPowerAssignment").unwrap().args[0];
        assert_eq!(a.get("distEng").and_then(Value::as_f64), Some(0.75));
        assert_eq!(a.get("distSys").and_then(Value::as_f64), Some(0.25));
        f.power.keys.sys = true;
        let msgs = feed(&mut g, &f);
        assert_eq!(msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["IndicateSystemPowerInput"], "a press alone");
    }

    #[test]
    fn signature_silent_clip_without_channels() {
        let mut g = Gauges::default();
        let mut f = flying();
        assert!(find(&feed(&mut g, &f), "SetSignature").is_some(), "the empty bar first");
        assert!(find(&feed(&mut g, &f), "SetSignature").is_none());
        f.signature.silent = true;
        let s = find(&feed(&mut g, &f), "SetSignature").unwrap().clone();
        assert_eq!(s.args, vec![Value::from(0.0), 0.0.into(), true.into()]);
        assert!(find(&feed(&mut g, &f), "SetSignature").is_none());
        f.signature.silent = false;
        assert_eq!(find(&feed(&mut g, &f), "SetSignature").unwrap().args[2], false.into());
        assert!(find(&feed(&mut g, &f), "SetSignature").is_none(), "once");
        f.signature.channels = Some([0.3, 0.2]);
        assert!(find(&feed(&mut g, &f), "SetSignature").is_some());
        f.signature.channels = Some([0.305, 0.2]);
        assert!(find(&feed(&mut g, &f), "SetSignature").is_none(), "under 0.01");
        f.signature.channels = Some([0.32, 0.2]);
        assert!(find(&feed(&mut g, &f), "SetSignature").is_some());
    }

    #[test]
    fn compass_and_target_location() {
        let mut g = Gauges::default();
        let mut f = flying();
        feed(&mut g, &f);
        f.destination = Some(Destination { kind: 2, name: "Moon".into(), distance_m: 1.5e6, dir: [0.6, 0.0, 0.8] });
        let msgs = feed(&mut g, &f);
        let names: Vec<&str> = msgs.iter().map(|m| m.name).collect();
        assert_eq!(names, ["SetCompassVisible", "SetCompassData", "HighlightCompass", "SetTargetLocation"]);
        let dir = &msgs[1].args[0];
        assert_eq!(dir.get("x").and_then(Value::as_f64), Some(0.6));
        assert_eq!(dir.get("z").and_then(Value::as_f64), Some(0.8));
        let t = &msgs[3].args[0];
        assert_eq!(t.get("type").and_then(Value::as_f64), Some(2.0));
        assert_eq!(t.get("distance").and_then(Value::as_str), Some("$DISTANCE:#value=1500000;"));
        // Every field the movie's handler reads is there, the ones the mod has no data for in the movie's empty state.
        assert_eq!(t.get("stationStatus").and_then(Value::as_f64), Some(0.0));
        assert_eq!(t.get("detailOne").and_then(Value::as_str), Some(""));
        assert_eq!(t.get("detailTwo").and_then(Value::as_str), Some(""));
        assert_eq!(t.get("scanInProgress").and_then(Value::as_bool), Some(false));
        let msgs = feed(&mut g, &f);
        assert_eq!(msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetCompassData"], "the data every frame, the rest on change");
        // Another destination: highlighted again.
        f.destination = Some(Destination { kind: 1, name: "Sol".into(), distance_m: 1.5e11, dir: [0.0, 0.6, 0.8] });
        let msgs = feed(&mut g, &f);
        assert_eq!(msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetCompassData", "HighlightCompass", "SetTargetLocation"]);
        // A zero direction: no compass, the panel stays.
        f.destination.as_mut().unwrap().dir = [0.0; 3];
        let msgs = feed(&mut g, &f);
        assert_eq!(msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetCompassVisible"]);
        f.destination.as_mut().unwrap().dir = [0.0, 0.6, 0.8];
        let msgs = feed(&mut g, &f);
        assert_eq!(msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetCompassVisible", "SetCompassData", "HighlightCompass"]);
        f.destination = None;
        let msgs = feed(&mut g, &f);
        assert_eq!(msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetCompassVisible", "ClearTargetLocation"]);
    }

    #[test]
    fn fuel_floor_smoothing_and_threshold() {
        let mut g = Gauges::default();
        let mut f = flying();
        f.fuel = crate::facts::Fuel {
            main_t: 1.2345,
            main_capacity_t: 2.0,
            reservoir_t: 0.3,
            reservoir_capacity_t: 0.3,
            burn_t_per_s: 1.0e-4,
            jump_cost_t: Some(0.6),
        };
        let msgs = feed(&mut g, &f);
        let a = &find(&msgs, "SetFuelLevels").unwrap().args;
        close(num(&a[0]), 61.0 * 0.01);
        close(num(&a[1]), 100.0 * 0.01);
        assert_eq!((num(&a[2]), num(&a[3]), num(&a[4])), (2.0, 0.3, 0.6));
        let first = 1.0e-4 * 3600.0 * 0.07;
        assert_eq!(a[5].as_str(), Some(format!("$FUEL_USAGE_PER_HOUR:#value={first};").as_str()));
        // The next frame's smoothed step is 0.07 x (0.36 - 0.0252) = 0.0234: past 0.005, sent.
        assert!(find(&feed(&mut g, &f), "SetFuelLevels").is_some());
        // Settled near 0.36: small steps are not sent until they add up past 0.005.
        for _ in 0..200 {
            feed(&mut g, &f);
        }
        assert!(find(&feed(&mut g, &f), "SetFuelLevels").is_none());
        f.fuel.jump_cost_t = None;
        let msgs = feed(&mut g, &f);
        assert_eq!(num(&find(&msgs, "SetFuelLevels").unwrap().args[4]), 0.0, "no jump: cost 0");
    }

    #[test]
    fn status_plates_in_order() {
        let mut g = Gauges::default();
        let mut f = flying();
        f.status.gear_down = true;
        let s = &first_arg(&mut g, &f, "SetStatusIndicators");
        let label = |slot: &str| s.get(slot).and_then(|p| p.get("label")).and_then(Value::as_str).map(str::to_string);
        assert_eq!(label("first").as_deref(), Some("$GAUGE_PANEL_mass_lock_warning;"));
        assert_eq!(label("second").as_deref(), Some("$GAUGE_PANEL_landing_gear;"));
        assert_eq!(label("third").as_deref(), Some("$GAUGE_PANEL_cargo_scoop;"));
        assert_eq!(s.get("second").and_then(|p| p.get("enabled")), Some(&Value::Bool(true)));
        assert_eq!(s.get("cooldown").and_then(|c| c.get("visible")), Some(&Value::Bool(false)));
        f.drive.cooldown = Some(0.25);
        let s = &first_arg(&mut g, &f, "SetStatusIndicators");
        assert_eq!(s.get("cooldown").and_then(|c| c.get("prop")).and_then(Value::as_f64), Some(0.25));
        assert!(find(&feed(&mut g, &f), "SetStatusIndicators").is_none());
    }
}
