//! The feeders together, and the check of every message and field name sent against the export's `messages/hud.json`
//! (ignored: it reads the export, out of git).

use super::{Hud, Msg, Outbox};
use crate::facts::{Channels, Deploy, Destination, Hardpoint, Landing, LifeAlarm, Planet, ShieldState};
use crate::{UiFacts, Value};

/// Facts that make every feeder send what it can: seated and powered, flying near a planet with a destination locked,
/// armed, charging, cooling, silent, the life support alarm on.
fn full() -> UiFacts {
    let mut f = UiFacts::default();
    f.status.seated = true;
    f.status.powered = true;
    f.status.gear_down = true;
    f.speed = crate::facts::Speed {
        supercruise: false,
        speed_m_s: 80.0,
        forward_m_s: 80.0,
        top_m_s: 160.0,
        full_top_m_s: 220.0,
        engines_on: true,
        demand: 0.5,
        flight_assist: false,
    };
    f.heat.prop = 0.4;
    f.fuel = crate::facts::Fuel {
        main_t: 2.0,
        main_capacity_t: 2.0,
        reservoir_t: 0.3,
        reservoir_capacity_t: 0.3,
        burn_t_per_s: 1.0e-4,
        jump_cost_t: Some(0.5),
    };
    f.damage.shield.strength = 1.0;
    f.damage.shield.state = ShieldState::Active;
    f.damage.hull = 1.0;
    f.power.pips = Channels { wep: 0.5, eng: 0.5, sys: 0.5 };
    f.power.capacitors = Channels { wep: 1.0, eng: 1.0, sys: 1.0 };
    f.power.keys.eng = true;
    f.power.keys.sys = true;
    f.power.keys.wep = true;
    f.power.keys.reset = true;
    f.signature.silent = true;
    f.destination = Some(Destination { kind: 2, name: "Moon".into(), distance_m: 2.0e6, dir: [0.0, 0.0, 1.0] });
    f.drive.charge = Some(0.5);
    f.drive.cooldown = Some(0.5);
    let laser = Hardpoint {
        module_item_id: 1,
        name: "Laser".into(),
        weapon_type: 1,
        weapon_mode: 1,
        deploy_s: 1.0,
        retract_s: 1.0,
        deploy: Deploy::Deployed,
        module_on: true,
        powered: true,
        trigger: 1,
    };
    f.weapons = crate::facts::Weapons { armed: true, firegroup: Some(0), hardpoints: Some(vec![laser.clone(), laser]) };
    f.alerts.life_support = LifeAlarm::On;
    f.alerts.oxygen_s = 100.0;
    f.clock.time_of_day_s = Some(43_200.0);
    f.planet = Some(Planet {
        radius_m: 1.0e6,
        centre_distance_m: 1.0e6 + 50.0,
        inner_height_m: None,
        heading_deg: 0.0,
        pitch_deg: 0.0,
        roll_deg: 0.0,
        landing: Some(Landing {
            roll_deg: 0.0,
            pitch_deg: 0.0,
            axis_dot_normal: 1.0,
            speed_m_s: 1.0,
            speed_limit_m_s: 15.0,
            suitable_terrain: Some(true),
            landed: false,
        }),
    });
    f
}

/// Every message the feeders send over a few frames that go through every change: the full facts, then the lock, the
/// charge, the alarms, the weapons and the silent running gone, then unpowered.
fn every_message() -> Vec<Msg> {
    let mut hud = Hud::default();
    let mut out = Outbox::default();
    let mut f = full();
    hud.feed(&f, 1.0 / 60.0, &mut out);
    f.destination = None;
    f.drive.charge = None;
    f.signature.silent = false;
    f.weapons.armed = false;
    f.alerts.life_support = LifeAlarm::Off;
    hud.feed(&f, 1.0 / 60.0, &mut out);
    f.status.powered = false;
    hud.feed(&f, 1.0 / 60.0, &mut out);
    out.msgs
}

#[test]
fn hud_feeds_life_first_and_data_only_seated() {
    let mut hud = Hud::default();
    let mut out = Outbox::default();
    let mut f = full();
    f.status.seated = false;
    hud.feed(&f, 0.0, &mut out);
    assert!(out.msgs.iter().all(|m| m.name == "Open" || m.name == "Hide"), "not seated: only the life cycle");
    let mut out = Outbox::default();
    f.status.seated = true;
    hud.feed(&f, 0.0, &mut out);
    assert!(out.msgs[..6].iter().all(|m| m.name == "Show"));
    assert!(out.msgs.len() > 30, "the data follows");
    let names: Vec<&str> = every_message().iter().map(|m| m.name).collect();
    for n in ["ClearTargetLocation", "RemoveIndicator", "SetCompassVisible", "Hide"] {
        assert!(names.contains(&n), "{n} over the changes");
    }
    hud.reset();
    let mut again = Outbox::default();
    hud.feed(&f, 0.0, &mut again);
    assert!(again.msgs.iter().filter(|m| m.name == "Open").count() == 6, "reset opens again");
}

/// Leaving the seat is not a reset: what went while nobody sat in it is cleared by its normal message on the next feed.
#[test]
fn clears_after_the_seat_was_left_without_reset() {
    let mut hud = Hud::default();
    let mut f = full();
    hud.feed(&f, 0.0, &mut Outbox::default());
    f.status.seated = false;
    hud.feed(&f, 0.0, &mut Outbox::default());
    f.destination = None;
    f.weapons.armed = false;
    f.drive.charge = None;
    hud.feed(&f, 0.0, &mut Outbox::default());
    f.status.seated = true;
    let mut out = Outbox::default();
    hud.feed(&f, 0.0, &mut out);
    let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
    assert!(names.contains(&"ClearTargetLocation"), "{names:?}");
    assert!(names.contains(&"RemoveIndicator"), "{names:?}");
    let off = out.msgs.iter().find(|m| m.name == "SetCentralWarning").expect("the warning cleared");
    assert_eq!(off.args[0], Value::from(0));
    assert!(!names.contains(&"Open"), "no reset: the movies are not opened again");
    assert!(!names.contains(&"ConfigureCockpit"), "no reset: the cockpit is not set up again");
}

/// A NaN or infinite fact is no new value: nothing is re-sent for it, frame after frame, and the next good value is.
#[test]
fn non_finite_facts_are_no_new_value() {
    let mut hud = Hud::default();
    let mut f = full();
    hud.feed(&f, 0.0, &mut Outbox::default());
    for _ in 0..300 {
        hud.feed(&f, 0.0, &mut Outbox::default());
    }
    let mut bad = f.clone();
    bad.heat.prop = f64::NAN;
    bad.speed.speed_m_s = f64::INFINITY;
    bad.speed.top_m_s = f64::NAN;
    bad.fuel.burn_t_per_s = f64::NAN;
    bad.fuel.main_t = f64::NEG_INFINITY;
    bad.damage.hull = f64::NAN;
    bad.power.capacitors.eng = f64::NAN;
    bad.alerts.oxygen_s = f64::NAN;
    bad.clock.time_of_day_s = Some(f64::NAN);
    bad.planet.as_mut().unwrap().centre_distance_m = f64::NAN;
    if let Some(d) = &mut bad.destination {
        d.distance_m = f64::NAN;
        d.dir = [f64::NAN, 0.0, 1.0];
    }
    for _ in 0..3 {
        let mut out = Outbox::default();
        hud.feed(&bad, 0.0, &mut out);
        let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
        // Only the messages sent every frame whatever changed: the compass, the altimeter and the landing guide.
        for n in &names {
            assert!(
                ["SetCompassData", "UpdateAltitude", "UpdateHeadingGauges", "UpdateLandingGuide", "SetLifeSupportFailureIn"]
                    .contains(n)
                    || n.starts_with("Indicate"),
                "{n} sent for a non-finite input"
            );
        }
        assert!(!names.contains(&"SetLifeSupportFailureIn"), "the oxygen kept");
        let mut values = Vec::new();
        out.msgs.iter().for_each(|m| m.args.iter().for_each(|a| numbers(a, &mut values)));
        assert!(values.iter().all(|v| v.is_finite()), "no non-finite number reaches a movie");
    }
    f.heat.prop = 0.9;
    let mut out = Outbox::default();
    hud.feed(&f, 0.0, &mut out);
    assert!(out.msgs.iter().any(|m| m.name == "SetHeat"), "the next good value is sent");
}

fn numbers(v: &Value, out: &mut Vec<f64>) {
    match v {
        Value::Num(n) => out.push(*n),
        Value::Array(items) => items.iter().for_each(|i| numbers(i, out)),
        Value::Object(fields) => fields.iter().for_each(|(_, i)| numbers(i, out)),
        _ => {}
    }
}

/// Every object field name in a value, nested objects and arrays included.
fn field_names(v: &Value, names: &mut Vec<String>) {
    match v {
        Value::Object(fields) => {
            for (k, inner) in fields {
                names.push(k.clone());
                field_names(inner, names);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| field_names(i, names)),
        _ => {}
    }
}

/// Every `$KEY` the messages' texts name (the token's key, before ':' or ';').
fn text_keys(v: &Value, keys: &mut Vec<String>) {
    match v {
        Value::Str(s) => {
            for piece in s.split('$').skip(1) {
                let key: String = piece.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if !key.is_empty() {
                    keys.push(key.to_lowercase());
                }
            }
        }
        Value::Object(fields) => fields.iter().for_each(|(_, i)| text_keys(i, keys)),
        Value::Array(items) => items.iter().for_each(|i| text_keys(i, keys)),
        _ => {}
    }
}

/// Whether `name` stands in `text` as a whole word, after `lead` ("" for any).
fn has_word(text: &str, lead: &str, name: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let needle = format!("{lead}{name}");
    text.match_indices(needle.as_str()).any(|(at, _)| {
        let before = lead.is_empty() && text[..at].chars().next_back().is_some_and(ident);
        let after = text[at + needle.len()..].chars().next().is_some_and(ident);
        !before && !after
    })
}

/// The decompiled ActionScript of a movie and the shared classes, as one text.
fn movie_code(dir: &str, movie: &str) -> String {
    fn walk(path: &std::path::Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "as") {
                out.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    let mut out = String::new();
    walk(&std::path::Path::new(dir).join("code").join(movie), &mut out);
    walk(&std::path::Path::new(dir).join("code/_shared"), &mut out);
    out
}

/// Every message name sent is one its movie listens for, with the export's number of arguments; every field name sent
/// is named in that message's entry or read by the movie's own code (`.name`: the notes' entries list some objects only
/// in part, the code is the truth); every text key sent is in the string table (`ELITE_UI_EXPORT`, default the export in
/// `re/`).
#[test]
#[ignore = "reads Elite's message notes and string table from the export (out of git)"]
fn every_name_sent_is_in_the_export() {
    let dir = std::env::var("ELITE_UI_EXPORT")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/elite/frea_export/sidewinder_ui/decoded").into());
    let read = |path: &str| -> serde_json::Value {
        let bytes = std::fs::read(format!("{dir}/{path}")).unwrap_or_else(|e| panic!("{path} in the export: {e}"));
        serde_json::from_slice(&bytes).expect("JSON")
    };
    let hud = read("messages/hud.json");
    let strings = read("strings/en.json");
    let mut problems = Vec::new();
    let mut code: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for msg in every_message() {
        let movie = &hud["movies"][msg.movie.name()];
        if movie.is_null() {
            problems.push(format!("{}: no such movie", msg.movie.name()));
            continue;
        }
        if movie["listens"].get(msg.name).is_none() {
            problems.push(format!("{}: does not listen for {}", msg.movie.name(), msg.name));
        }
        let entry = &movie["messages"][msg.name];
        if let Some(args) = entry["args"].as_array()
            && args.len() != msg.args.len()
        {
            problems.push(format!("{} {}: {} arguments sent, {} in the notes", msg.movie.name(), msg.name, msg.args.len(), args.len()));
        }
        let text = entry.to_string();
        let mut names = Vec::new();
        msg.args.iter().for_each(|a| field_names(a, &mut names));
        for name in names {
            let code = code.entry(msg.movie.name()).or_insert_with(|| movie_code(&dir, msg.movie.name()));
            if !has_word(&text, "", &name) && !has_word(code, ".", &name) {
                problems.push(format!("{} {}: field {name} is neither in the notes' entry nor read by the movie", msg.movie.name(), msg.name));
            }
        }
        let mut keys = Vec::new();
        msg.args.iter().for_each(|a| text_keys(a, &mut keys));
        for key in keys {
            if strings.get(&key).is_none() {
                problems.push(format!("{} {}: text key {key} is not in the string table", msg.movie.name(), msg.name));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
