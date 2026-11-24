//! The side panels' feeders together (`Panels`), the focus machine's messages with them, and the check (ignored: it reads
//! the export, out of git) that every message, argument count, field, DataStore property and text key sent exists in
//! `messages/panels.json`, the movies' own code or the full string table.

use super::{Msg, Outbox, Panels, Store};
use crate::facts::location_type as kind;
use crate::focus::{FocusState, Panel};
use crate::panel_facts::{FireGroups, FireWeapon, FlightPhase, Functions, Location, LogLine, Module, PanelFacts, Power, Ship};
use crate::panels::{on_out, Function, PanelAction};
use crate::Value;

/// A lone Sidewinder's facts with every page's data (the numbers are this test's).
pub fn sample() -> PanelFacts {
    let place = |id, name: &str, kind, distance_m, parent| Location { id, name: name.into(), kind, distance_m, parent, ..Location::default() };
    let module = |id, item: &str, kind, size, rating, power_mw, switchable| Module {
        id,
        item: item.into(),
        kind,
        size,
        rating,
        power_mw,
        on: true,
        switchable,
        health: 1.0,
        priority: None,
        weapon_mode: None,
    };
    let mut laser = module(9, "Hpt_PulseLaser_Gimbal_Small", 7, 1, 'G', 0.39, true);
    laser.weapon_mode = Some(1);
    let mut laser2 = laser.clone();
    laser2.id = 10;
    let mut neighbour = place(20, "Neighbour", kind::STAR, 4.0e16, None);
    neighbour.other_system = true;
    let mut planet = place(3, "Planet 2", kind::PLANET, 6.8e10, Some(2));
    planet.locked = true;
    let mut star = place(2, "Star", kind::STAR, 8.7e10, None);
    star.current = true;
    PanelFacts {
        system: "Test System".into(),
        locations: vec![star, planet, place(4, "Moon 2 A", kind::PLANET, 6.79e10, Some(3)), neighbour],
        commander: Some("Tester".into()),
        ship: Ship { model: "sidewinder".into(), name: None, ident: Some("SW-01".into()) },
        modules: vec![
            module(1, "Int_PowerPlant_Size2_Class1", 10, 2, 'E', 0.0, false),
            module(2, "Int_Engine_Size2_Class1", 22, 2, 'E', 2.0, true),
            module(3, "Int_Hyperdrive_Size2_Class1", 9, 2, 'E', 0.16, true),
            module(4, "Int_LifeSupport_Size1_Class1", 23, 1, 'E', 0.32, true),
            module(5, "Int_PowerDistributor_Size1_Class1", 20, 1, 'E', 0.32, false),
            module(6, "Int_Sensors_Size1_Class1", 6, 1, 'E', 0.16, true),
            module(7, "Int_FuelTank_Size1_Class3", 11, 1, 'C', 0.0, false),
            module(8, "Int_ShieldGenerator_Size2_Class1", 4, 2, 'E', 0.9, true),
            laser,
            laser2,
        ],
        power: Power { capacity_mw: 6.4, available_mw: 6.4 },
        fire_groups: FireGroups {
            count: 1,
            active: 0,
            weapons: vec![
                FireWeapon { module: 9, slot: 0, trigger: 0, weapon_type: 0, bindings: vec![1] },
                FireWeapon { module: 10, slot: 1, trigger: 0, weapon_type: 0, bindings: vec![1] },
            ],
        },
        cargo_capacity_t: Some(4.0),
        functions: Functions { flight_assist: true, ..Functions::default() },
        flight: FlightPhase::Normal,
        leave_allowed: false,
        comms: vec![LogLine { id: 1, text: "Landing gear deployed".into(), time_of_day_s: 50_000.0 }],
        time_of_day_s: Some(50_000.0),
    }
}

fn shown() -> FocusState {
    FocusState {
        state: "In Flight".into(),
        shown: true,
        focused: None,
        comms_level: None,
        head: None,
        control_sets: Vec::new(),
        quiet_flight: false,
        on_screen: Panel::ALL.to_vec(),
    }
}

/// Every message and DataStore write the feeders make over a visit of every page, the pop-ups and their buttons.
pub fn everything() -> (Vec<Msg>, Vec<Store>) {
    let mut panels = Panels::default();
    let mut facts = sample();
    let mut out = Outbox::default();
    let focus = shown();
    let frame = |panels: &mut Panels, facts: &PanelFacts, out: &mut Outbox| panels.feed(facts, &focus, 0.5, out);
    frame(&mut panels, &facts, &mut out);
    let left = crate::feed::Movie::CockpitExternalPanel;
    let right = crate::feed::Movie::CockpitInternalPanel2;
    let pages = [(left, 3), (left, 1), (right, 1), (right, 2), (right, 3), (right, 4), (right, 6)];
    for (movie, page) in pages {
        panels.hear(&[PanelAction::Page { movie, page }], &facts);
        frame(&mut panels, &facts, &mut out);
    }
    panels.hear(&[PanelAction::LocationSelected(3), PanelAction::ModuleSelected(8), PanelAction::TabsReady], &facts);
    frame(&mut panels, &facts, &mut out);
    panels.hear(&[PanelAction::LocationButton(1), PanelAction::ModuleOption(0)], &facts);
    facts.functions.silent_running = true;
    frame(&mut panels, &facts, &mut out);
    for _ in 0..4 {
        frame(&mut panels, &facts, &mut out);
    }
    (out.msgs, out.stores)
}

#[test]
fn panels_feed_while_shown_and_resolve_buttons() {
    let mut panels = Panels::default();
    let facts = sample();
    let hidden = FocusState { shown: false, on_screen: Vec::new(), ..shown() };
    let mut out = Outbox::default();
    panels.feed(&facts, &hidden, 0.1, &mut out);
    assert!(out.msgs.is_empty() && out.stores.is_empty(), "nothing while hidden (the life cycle is the focus machine's)");
    let mut out = Outbox::default();
    panels.feed(&facts, &shown(), 0.1, &mut out);
    let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
    for n in ["SetPanelsMode", "SetDestinationData", "SetTabData", "InitDataStore", "PushMessageToLog"] {
        assert!(names.contains(&n), "{n} on the first shown feed");
    }
    assert!(!names.contains(&"Open"), "the life cycle is the focus machine's");
    // The pop-ups: the location's Unlock unlocks, the module pop-up's switch switches the shield generator off.
    let resolved = panels.hear(&[PanelAction::LocationSelected(3), PanelAction::ModuleSelected(8)], &facts);
    assert!(resolved.is_empty());
    panels.feed(&facts, &shown(), 0.1, &mut Outbox::default());
    let resolved = panels.hear(
        &[
            PanelAction::LocationButton(1),
            PanelAction::ModuleOption(0),
            PanelAction::Toggle(Function::LandingGear),
            PanelAction::ShowComplete(crate::feed::Movie::RolePanel3),
        ],
        &facts,
    );
    assert_eq!(
        resolved,
        [PanelAction::Unlock, PanelAction::SetModule { id: 8, on: false }, PanelAction::Toggle(Function::LandingGear)]
    );
    // From the movies' own words to the ship's actions.
    let acts = on_out(crate::feed::Movie::CockpitInternalPanel2, "ComboOptionSelected", &[Value::from(12)]);
    assert_eq!(panels.hear(&acts, &facts), [PanelAction::Toggle(Function::CargoScoop)]);
    panels.reset();
    let mut out = Outbox::default();
    panels.feed(&facts, &shown(), 0.1, &mut out);
    assert!(out.msgs.iter().any(|m| m.name == "SetTabData"), "everything again after a reload");
}

/// Only the panels on screen are fed (nothing piles up for a movie not loaded); a panel coming on screen sends its state;
/// `reset_movie` makes that one feeder send everything again while the others (the comms log above all) carry on.
#[test]
fn only_panels_on_screen_are_fed_and_one_movie_resets() {
    let left = crate::feed::Movie::CockpitExternalPanel;
    let chat = crate::feed::Movie::CockpitChatPanel;
    let mut panels = Panels::default();
    let facts = sample();
    let flying = FocusState { on_screen: vec![Panel::Comms], ..shown() };
    let mut out = Outbox::default();
    for _ in 0..40 {
        panels.feed(&facts, &flying, 0.1, &mut out);
    }
    assert!(out.msgs.iter().all(|m| m.movie == chat), "only the comms panel in flight");
    assert!(out.stores.is_empty(), "the role panel's DataStore waits until it is on screen");
    let with_left = FocusState { on_screen: vec![Panel::Left, Panel::Comms], ..shown() };
    let mut out = Outbox::default();
    panels.feed(&facts, &with_left, 0.1, &mut out);
    let to_left: Vec<&str> = out.msgs.iter().filter(|m| m.movie == left).map(|m| m.name).collect();
    assert_eq!(to_left, ["SetPanelsMode", "SetDestinationData"], "the left panel's state when it comes on");
    let mut out = Outbox::default();
    panels.feed(&facts, &with_left, 0.1, &mut out);
    assert!(out.msgs.iter().all(|m| m.movie != left), "then only changes");
    panels.reset_movie(left);
    panels.reset_movie(crate::feed::Movie::CockpitHudRing);
    let mut out = Outbox::default();
    panels.feed(&facts, &with_left, 0.1, &mut out);
    let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
    assert!(names.contains(&"SetPanelsMode") && names.contains(&"SetDestinationData"), "the left panel again: {names:?}");
    assert!(!names.contains(&"PushMultipleMessagesToLog"), "the comms log is not put back for another movie");
    panels.reset_movie(chat);
    let mut out = Outbox::default();
    panels.feed(&facts, &with_left, 0.1, &mut out);
    let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
    assert!(names.contains(&"PushMultipleMessagesToLog"), "a real reload of the comms movie puts its log back");
    assert!(!names.contains(&"SetPanelsMode"), "the left panel carries on");
}

#[test]
fn the_focus_machine_names_the_panels_movies() {
    assert_eq!(Panel::Left.movie().name(), "cockpitexternalpanel");
    assert_eq!(Panel::Right.movie().name(), "cockpitinternalpanel2");
    assert_eq!(Panel::Comms.movie().name(), "cockpitchatpanel");
    assert_eq!(Panel::Role.movie().name(), "rolepanel3");
}

#[test]
fn non_finite_facts_send_nothing_new() {
    let mut panels = Panels::default();
    let mut facts = sample();
    panels.feed(&facts, &shown(), 0.1, &mut Outbox::default());
    for _ in 0..10 {
        panels.feed(&facts, &shown(), 0.1, &mut Outbox::default());
    }
    facts.locations.push(Location { id: 99, name: "Nowhere".into(), distance_m: f64::NAN, ..Location::default() });
    facts.power.available_mw = f64::INFINITY;
    let mut out = Outbox::default();
    panels.feed(&facts, &shown(), 0.1, &mut out);
    assert!(out.msgs.iter().all(|m| m.name != "SetDestinationData"), "a place with no distance is no change");
}

// ---- the check against the export (ignored) ----

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

/// Every `$KEY` a value's texts name (before ':' or ';').
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

/// Whether `name` stands in `text` as a whole word right after `lead`.
fn has_word(text: &str, lead: &str, name: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let needle = format!("{lead}{name}");
    text.match_indices(needle.as_str()).any(|(at, _)| {
        let before = lead.is_empty() && text[..at].chars().next_back().is_some_and(ident);
        let after = text[at + needle.len()..].chars().next().is_some_and(ident);
        !before && !after
    })
}

/// A movie's decompiled code and the shared classes, as one text.
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

/// Every message sent is one its movie listens for (its `api.json`), with no more arguments than the notes' entry gives;
/// every field is named in the movie's notes (`panels.json`) or read by the movie's code (`.name`); every DataStore property is
/// in the notes or a string the movie's code names (`"name"`); every text key is in the full string table (the shipped `ui/strings/en.json`, or
/// `ELITE_UI_STRINGS`).
#[test]
#[ignore = "reads Elite's message notes, the movies' code and the string table (out of git or Elite's data)"]
fn every_name_sent_is_in_the_export() {
    let dir = std::env::var("ELITE_UI_EXPORT")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/elite/frea_export/sidewinder_ui/decoded").into());
    let strings_path = std::env::var("ELITE_UI_STRINGS").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../se_engine/assets/mods/sidewinder/ui/strings/en.json").into()
    });
    let read = |path: &str| -> serde_json::Value {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_slice(&bytes).expect("JSON")
    };
    let notes = read(&format!("{dir}/messages/panels.json"));
    let strings = read(&strings_path);
    let mut code: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut problems = Vec::new();
    let (mut msgs, stores) = everything();
    // The focus machine's messages too (the comms panel's FocusOn takes its level).
    msgs.push(Msg { movie: Panel::Comms.movie(), name: "FocusOn", args: vec![Value::from(2)] });
    msgs.push(Msg { movie: Panel::Left.movie(), name: "ReceivingButtons", args: vec![Value::from(true)] });
    msgs.push(Msg { movie: Panel::Left.movie(), name: "OnInputButton", args: vec!["UI_Down".into(), "buttonDown".into()] });
    for msg in &msgs {
        let movie = msg.movie.name();
        let api = read(&format!("{dir}/movies/{movie}/api.json"));
        if api["listens"].get(msg.name).is_none() {
            problems.push(format!("{movie}: does not listen for {}", msg.name));
        }
        let entry = &notes[movie]["messages"][msg.name];
        if let Some(args) = entry["args"].as_array()
            && msg.args.len() > args.len()
        {
            problems.push(format!("{movie} {}: {} arguments sent, {} in the notes", msg.name, msg.args.len(), args.len()));
        }
        // The movie's whole entry in the notes (its messages, pop-ups, builders' key lists), as the brief allows.
        let text = notes[movie].to_string();
        let mut names = Vec::new();
        msg.args.iter().for_each(|a| field_names(a, &mut names));
        let code = code.entry(movie).or_insert_with(|| movie_code(&dir, movie));
        for name in names {
            if !has_word(&text, "", &name) && !has_word(code, ".", &name) {
                problems.push(format!("{movie} {}: field {name} is neither in the notes' entry nor read by the movie", msg.name));
            }
        }
        let mut keys = Vec::new();
        msg.args.iter().for_each(|a| text_keys(a, &mut keys));
        for key in keys {
            if strings.get(&key).is_none() {
                problems.push(format!("{movie} {}: text key {key} is not in the string table", msg.name));
            }
        }
    }
    for store in &stores {
        let movie = store.movie.name();
        let code = code.entry(movie).or_insert_with(|| movie_code(&dir, movie));
        let in_notes = has_word(&notes[movie].to_string(), "", &store.property);
        if !code.contains(&format!("\"{}\"", store.property)) && !in_notes {
            problems.push(format!("{movie}: DataStore property {} is neither in the notes nor named by the movie's code", store.property));
        }
        let mut keys = Vec::new();
        text_keys(&store.value, &mut keys);
        let mut names = Vec::new();
        field_names(&store.value, &mut names);
        for name in names {
            if !has_word(code, ".", &name) && !code.contains(&format!("\"{name}\"")) {
                problems.push(format!("{movie} {}: field {name} is not read by the movie's code", store.property));
            }
        }
        for key in keys {
            if strings.get(&key).is_none() {
                problems.push(format!("{movie} {}: text key {key} is not in the string table", store.property));
            }
        }
    }
    problems.sort();
    problems.dedup();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
