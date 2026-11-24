//! The focus machine on a small machine written here in the shipped file's shape (no Elite data in the crate), and, ignored,
//! on the shipped `ui/focus.json` itself: every transition of `ui_rendering.md` section 4.2's table, `UIFocus` held and
//! released, the head look of section 4.3.

use super::*;
use serde_json::json;

fn modal() -> Json {
    json!({"type": "ControlInputScheme", "control_sets": ["Bindings_InterfaceModal", "UIEscBack"]})
}

fn flight_sets() -> Json {
    json!({"type": "ControlInputScheme", "control_sets": ["Bindings_Flight", "Bindings_Interface", "Bindings_UiFocus"]})
}

fn inputs(pairs: &[(&str, &str)], exits: &[&str], exit_target: Option<&str>) -> Json {
    json!({"type": "CockpitFocusInputActivity", "inputs": pairs, "exit_inputs": exits, "exit_target": exit_target})
}

fn head(component: Option<&str>, joint: Option<&str>, fov: f64, second: f64) -> Json {
    json!({"type": "HeadlookTargetBoneActivity", "panel_component": component, "bone": joint, "fov_channel_deg": fov, "fov_channel_deg_alt": second})
}

fn show() -> Json {
    json!({"type": "CockpitShowPanelActivity"})
}

fn focus_panel() -> Json {
    json!({"type": "CockpitFocusPanelActivity"})
}

/// A machine with the lone ship's states, written in the data's shape (the transitions follow the notes' 4.2).
fn sample() -> Vec<u8> {
    let flight_inputs = inputs(
        &[
            ("FocusLeftPanel", "External Panel"),
            ("FocusRightPanel", "Internal Panel"),
            ("FocusRadarPanel", "RedirectRoleSwitch"),
            ("QuickCommsPanel", "Comms Panel Mini"),
            ("FocusCommsPanel", "Comms Panel Full"),
        ],
        &["UIFocus"],
        Some("Select UI Mode"),
    );
    let maps = inputs(&[("GalaxyMapOpen", "Galaxy Map"), ("SystemMapOpen", "System Map")], &[], None);
    let comms_inputs = inputs(
        &[
            ("FocusLeftPanel", "External Panel"),
            ("FocusRightPanel", "Internal Panel"),
            ("FocusRadarPanel", "RedirectRoleSwitch"),
            ("FocusCommsPanel", "Back From"),
        ],
        &["UIFocus", "FocusCommsPanel", "UI_Back"],
        Some("Select UI Mode"),
    );
    let states = json!({
        "Idle": {"activities": [], "transitions": [{"target": "CockpitTransitionState"}]},
        "CockpitTransitionState": {"activities": [flight_sets(), show()], "transitions": []},
        "In Flight": {"activities": [flight_sets(), flight_inputs.clone(), show(), focus_panel(), maps], "transitions": []},
        "Back From": {"activities": [flight_sets(), flight_inputs, show()], "transitions": []},
        "Launching": {"activities": [show()], "transitions": []},
        "Select UI Mode": {"activities": [flight_sets(), modal(),
            inputs(&[("UI_Left", "External Panel"), ("UI_Right", "Internal Panel"), ("UI_Down", "RedirectRoleSwitch"), ("UI_Up", "Comms Panel Full")], &[], None),
            inputs(&[("UIFocus", "Back From")], &[], None), show(), head(None, None, 10.0, 0.0)], "transitions": []},
        "External Panel": {"activities": [modal(), flight_sets(),
            inputs(&[("FocusRightPanel", "Internal Panel"), ("FocusRadarPanel", "RedirectRoleSwitch"), ("FocusCommsPanel", "Comms Panel Full")], &["UIFocus", "FocusLeftPanel", "UI_Back"], Some("Back From")),
            head(Some("ExternalDataPanel"), Some("LeftPanel_Focus"), -17.0, -13.0), focus_panel(), show()], "transitions": []},
        "Internal Panel": {"activities": [flight_sets(),
            inputs(&[("FocusLeftPanel", "External Panel"), ("FocusRadarPanel", "RedirectRoleSwitch"), ("FocusCommsPanel", "Comms Panel Full")], &["UIFocus", "FocusRightPanel", "UI_Back"], Some("Select UI Mode")),
            modal(), head(Some("InternalDataPanel"), Some("RightPanel_Focus"), -17.0, -13.0), focus_panel(), show()], "transitions": []},
        "Comms Panel Full": {"activities": [flight_sets(), modal(), {"type": "ControlInputScheme", "control_sets": ["Bindings_InterfaceModal_Pagination"]},
            show(), focus_panel(), comms_inputs.clone(), head(Some("CommsPanel"), Some("CommsPanel_Focus"), -8.0, -5.0)], "transitions": []},
        "Comms Panel Mini": {"activities": [flight_sets(), modal(), show(), focus_panel(), comms_inputs], "transitions": []},
        "RedirectRoleSwitch": {"activities": [flight_sets(), modal(), show(), {"type": "RedirectRolePanelActivity"}], "transitions": []},
        "Role Switch": {"activities": [modal(), flight_sets(),
            inputs(&[("FocusLeftPanel", "External Panel"), ("FocusCommsPanel", "Comms Panel Full"), ("FocusRightPanel", "Internal Panel")], &["UIFocus", "FocusRadarPanel", "UI_Back"], Some("Back From")),
            head(Some("RoleSwitchPanel"), Some("CockpitChair_Bone_Rig!RoleSwitch_Panel"), -17.0, -17.0), focus_panel(), show()], "transitions": []},
        "Radar Panel": {"activities": [flight_sets(), modal(), show(), focus_panel(), head(None, Some("RadarPanel_Focus"), -20.0, -20.0)], "transitions": []},
        "Galaxy Map": {"activities": [], "transitions": []},
        "System Map": {"activities": [], "transitions": []}
    });
    serde_json::to_vec(&json!({"initial": "Idle", "states": states})).unwrap()
}

fn names(out: &[FocusOut]) -> Vec<(Movie, &'static str)> {
    out.iter().map(|o| (o.msg().movie, o.msg().name)).collect()
}

fn flying(bytes: &[u8]) -> Focus {
    let mut f = Focus::from_json(bytes).expect("the machine");
    f.seat(true);
    f
}

/// Every transition of the notes' 4.2 table, from `In Flight` and between the focused states.
fn check_transitions(bytes: &[u8]) {
    let go = |from: &str, presses: &[&str]| -> String {
        let mut f = flying(bytes);
        f.current = from.to_string();
        for p in presses {
            f.input(p, true);
            f.input(p, false);
        }
        f.state().state
    };
    for flight in ["In Flight", "Back From"] {
        assert_eq!(go(flight, &["FocusLeftPanel"]), "External Panel");
        assert_eq!(go(flight, &["FocusRightPanel"]), "Internal Panel");
        assert_eq!(go(flight, &["FocusRadarPanel"]), "Role Switch", "through RedirectRoleSwitch");
        assert_eq!(go(flight, &["FocusCommsPanel"]), "Comms Panel Full");
        assert_eq!(go(flight, &["QuickCommsPanel"]), "Comms Panel Mini");
        assert_eq!(go(flight, &["GalaxyMapOpen"]), flight, "the maps are cut");
    }
    assert_eq!(go("External Panel", &["FocusRightPanel"]), "Internal Panel");
    assert_eq!(go("External Panel", &["FocusRadarPanel"]), "Role Switch");
    assert_eq!(go("External Panel", &["FocusCommsPanel"]), "Comms Panel Full");
    assert_eq!(go("External Panel", &["FocusLeftPanel"]), "Back From");
    assert_eq!(go("External Panel", &["UI_Back"]), "Back From");
    assert_eq!(go("Internal Panel", &["FocusLeftPanel"]), "External Panel");
    // The right and comms panels' exits go to `Select UI Mode`, which is left at once with `UIFocus` up.
    assert_eq!(go("Internal Panel", &["FocusRightPanel"]), "Back From");
    assert_eq!(go("Internal Panel", &["UI_Back"]), "Back From");
    assert_eq!(go("Comms Panel Full", &["FocusLeftPanel"]), "External Panel");
    assert_eq!(go("Comms Panel Full", &["FocusRightPanel"]), "Internal Panel");
    assert_eq!(go("Comms Panel Full", &["FocusRadarPanel"]), "Role Switch");
    assert_eq!(go("Comms Panel Full", &["FocusCommsPanel"]), "Back From");
    assert_eq!(go("Comms Panel Mini", &["FocusCommsPanel"]), "Back From");
    assert_eq!(go("Comms Panel Mini", &["UI_Back"]), "Back From");
    assert_eq!(go("Comms Panel Full", &["UI_Back"]), "Back From");
    assert_eq!(go("Role Switch", &["FocusLeftPanel"]), "External Panel");
    assert_eq!(go("Role Switch", &["FocusRadarPanel"]), "Back From");
    assert_eq!(go("Role Switch", &["UI_Back"]), "Back From");
    // Select UI Mode: the four directions.
    for (key, to) in [("UI_Left", "External Panel"), ("UI_Right", "Internal Panel"), ("UI_Down", "Role Switch"), ("UI_Up", "Comms Panel Full")] {
        assert_eq!(go("Select UI Mode", &[key]), to);
    }
}

/// `UIFocus` held: the press goes to `Select UI Mode`, released alone it goes to `Back From`; held, a direction opens a
/// panel and the release keeps it open; a tap inside a panel leaves the panel and its release does nothing more. In toggle
/// mode the release does nothing and `Select UI Mode` stays.
fn check_hold(bytes: &[u8]) {
    let mut f = flying(bytes);
    f.input("UIFocus", true);
    assert_eq!(f.state().state, "Select UI Mode");
    assert_eq!(f.state().focused, None);
    f.input("UIFocus", false);
    assert_eq!(f.state().state, "Back From", "released alone");
    f.input("UIFocus", true);
    f.input("UI_Left", true);
    f.input("UI_Left", false);
    assert_eq!(f.state().state, "External Panel");
    assert_eq!(f.state().focused, Some(Panel::Left));
    f.input("UIFocus", false);
    assert_eq!(f.state().state, "External Panel", "the release keeps the panel the direction opened");
    f.input("UIFocus", true);
    assert_eq!(f.state().state, "Back From", "a press inside a panel leaves it");
    f.input("UIFocus", false);
    assert_eq!(f.state().state, "Back From", "and its release does nothing more");
    // Held, a direction opens a panel, its exit comes back to `Select UI Mode` with the key still down: the release
    // leaves it (the right panel's 4 and UI_Back, the comms panel's UI_Back).
    for (direction, exit) in [("UI_Right", "FocusRightPanel"), ("UI_Right", "UI_Back"), ("UI_Up", "UI_Back")] {
        f.input("UIFocus", true);
        f.input(direction, true);
        f.input(direction, false);
        assert!(f.state().focused.is_some(), "{direction} opens a panel");
        f.input(exit, true);
        f.input(exit, false);
        assert_eq!(f.state().state, "Select UI Mode", "{direction}, {exit}: back with UIFocus down");
        f.input("UIFocus", false);
        assert_flying(&f, &format!("held, {direction}, {exit}, released"));
    }
    f.hold = false;
    f.input("UIFocus", true);
    f.input("UIFocus", false);
    assert_eq!(f.state().state, "Select UI Mode", "toggle mode: the release is no input, nothing is left at once");
    f.input("UIFocus", true);
    assert_eq!(f.state().state, "Back From");
}

/// The state is a flight state: no panel has the keys, the flight keys are live, the head looks ahead.
fn assert_flying(f: &Focus, why: &str) {
    let s = f.state();
    assert!(["In Flight", "Back From"].contains(&s.state.as_str()), "{why}: in {}", s.state);
    assert_eq!(s.focused, None, "{why}");
    assert!(!s.quiet_flight, "{why}: the flight keys live");
    assert_eq!(s.head, None, "{why}: the head look back");
}

/// Every way into `Select UI Mode` without `UIFocus` held ends in flight, and a `UIFocus` tap afterwards too; each panel's
/// own key opens it and closes it.
fn check_no_trap(bytes: &[u8]) {
    let tap = |f: &mut Focus, k: &str| {
        f.input(k, true);
        f.input(k, false);
    };
    for (open, close) in [
        ("FocusRightPanel", "FocusRightPanel"),
        ("FocusRightPanel", "UI_Back"),
        ("FocusCommsPanel", "UI_Back"),
        ("QuickCommsPanel", "UI_Back"),
        ("FocusLeftPanel", "UIFocus"),
        ("FocusRadarPanel", "UIFocus"),
        ("FocusRightPanel", "UIFocus"),
        ("FocusCommsPanel", "UIFocus"),
    ] {
        let mut f = flying(bytes);
        tap(&mut f, open);
        assert!(f.state().focused.is_some(), "{open} opens a panel");
        tap(&mut f, close);
        assert_flying(&f, &format!("{open} then {close}"));
        tap(&mut f, "UIFocus");
        assert_flying(&f, &format!("{open} then {close}, then a UIFocus tap"));
    }
    for (key, panel) in [
        ("FocusLeftPanel", Panel::Left),
        ("FocusRightPanel", Panel::Right),
        ("FocusRadarPanel", Panel::Role),
        ("FocusCommsPanel", Panel::Comms),
    ] {
        let mut f = flying(bytes);
        tap(&mut f, key);
        assert_eq!(f.state().focused, Some(panel), "{key} opens");
        tap(&mut f, key);
        assert_flying(&f, &format!("{key} twice"));
    }
}

/// The head look of the notes' 4.3 table.
fn check_head(bytes: &[u8]) {
    let mut f = flying(bytes);
    let look = |f: &Focus| f.state().head.map(|h| (h.panel_component, h.joint, h.fov_deg, h.fov_deg_second));
    let s = |v: &str| Some(v.to_string());
    for (state, want) in [
        ("External Panel", (s("ExternalDataPanel"), s("LeftPanel_Focus"), -17.0, -13.0)),
        ("Internal Panel", (s("InternalDataPanel"), s("RightPanel_Focus"), -17.0, -13.0)),
        ("Comms Panel Full", (s("CommsPanel"), s("CommsPanel_Focus"), -8.0, -5.0)),
        ("Radar Panel", (None, s("RadarPanel_Focus"), -20.0, -20.0)),
        ("Role Switch", (s("RoleSwitchPanel"), s("CockpitChair_Bone_Rig!RoleSwitch_Panel"), -17.0, -17.0)),
        ("Select UI Mode", (None, None, 10.0, 0.0)),
    ] {
        f.current = state.to_string();
        assert_eq!(look(&f), Some(want), "{state}");
    }
    f.current = "In Flight".to_string();
    assert_eq!(look(&f), None);
}

#[test]
fn transitions_on_the_sample() {
    check_transitions(&sample());
}

#[test]
fn hold_and_release_on_the_sample() {
    check_hold(&sample());
}

#[test]
fn head_look_on_the_sample() {
    check_head(&sample());
}

#[test]
fn no_trap_on_the_sample() {
    check_no_trap(&sample());
}

/// With a pop-up open on the focused panel, `UI_Back` is the pop-up's: it goes to the movie and the panel stays focused;
/// once the movie says the pop-up closed, `UI_Back` leaves the panel again.
#[test]
fn ui_back_closes_a_popup_first() {
    let mut f = flying(&sample());
    f.input("FocusLeftPanel", true);
    f.hear(&[PanelAction::Popup { movie: Movie::CockpitExternalPanel, state: 2 }]);
    let out = f.input("UI_Back", true);
    assert_eq!(out, vec![FocusOut::Button { movie: Movie::CockpitExternalPanel, name: "UI_Back", down: true }]);
    assert_eq!(f.state().state, "External Panel");
    assert_eq!(f.input("UI_Back", false).len(), 1, "its release too");
    f.hear(&[PanelAction::Popup { movie: Movie::CockpitExternalPanel, state: 0 }]);
    f.input("UI_Back", true);
    assert_eq!(f.state().state, "Back From");
}

/// A button held when its panel loses the keys is released to it before the `FocusOff`; its later release goes nowhere.
#[test]
fn held_buttons_are_released_on_a_focus_change() {
    let mut f = flying(&sample());
    f.input("FocusLeftPanel", true);
    f.input("UI_Down", true);
    let out = f.input("FocusRightPanel", true);
    let n = names(&out);
    assert_eq!(n[0], (Movie::CockpitExternalPanel, "OnInputButton"));
    assert_eq!(out[0].msg().args, vec![Value::from("UI_Down"), "buttonUp".into()]);
    assert_eq!(n[1], (Movie::CockpitExternalPanel, "FocusOff"));
    assert!(f.input("UI_Down", false).is_empty(), "no release without its press");
    assert!(f.input("UI_Up", false).is_empty(), "never pressed: nothing");
}

/// The movies are opened before anything else, once; after a reload they are opened and brought to the state again.
#[test]
fn open_comes_first_and_a_reload_replays() {
    let mut f = Focus::from_json(&sample()).unwrap();
    let out = f.seat(true);
    let n = names(&out);
    assert!(n[..4].iter().all(|(_, m)| *m == "Open"), "{n:?}");
    assert_eq!(n[4].1, "Show");
    f.input("FocusLeftPanel", true);
    let n = names(&f.input("FocusRightPanel", true));
    assert!(!n.iter().any(|(_, m)| *m == "Open"), "opened once");
    let n = names(&f.reload());
    assert!(n[..4].iter().all(|(_, m)| *m == "Open"));
    let shown: Vec<Movie> = n.iter().filter(|(_, m)| *m == "Show").map(|(movie, _)| *movie).collect();
    assert_eq!(shown, [Movie::CockpitInternalPanel2, Movie::CockpitChatPanel], "the focused panel and the comms bar");
    assert!(n.contains(&(Movie::CockpitInternalPanel2, "FocusOn")), "the focused panel again");
    assert_eq!(f.state().on_screen, [Panel::Right, Panel::Comms], "a reload forgets the hiding left panel");
}

/// A movie loaded late gets `Open` and what brings it to the state: the focused panel its `Show` and focus, the comms
/// bar its `Show`, a panel not wanted only its `Open`; nothing before the machine's first messages.
#[test]
fn a_late_movie_is_brought_to_the_state() {
    let mut f = Focus::from_json(&sample()).unwrap();
    assert!(f.reload_movie(Movie::CockpitExternalPanel).is_empty(), "nothing opened yet");
    f.seat(true);
    f.input("FocusLeftPanel", true);
    let left = Movie::CockpitExternalPanel;
    assert_eq!(names(&f.reload_movie(left)), [(left, "Open"), (left, "Show"), (left, "FocusOn"), (left, "ReceivingButtons")]);
    let right = Movie::CockpitInternalPanel2;
    assert_eq!(names(&f.reload_movie(right)), [(right, "Open")]);
    let chat = Movie::CockpitChatPanel;
    assert_eq!(names(&f.reload_movie(chat)), [(chat, "Open"), (chat, "Show"), (chat, "FocusOff")]);
}

/// The left, right and role panels are shown when they gain the keys and hidden when they lose them, every time (the
/// opening sound each time: session ui1); the comms bar stays shown. A hiding panel stays on screen until its
/// `HideComplete`, or `HIDE_LIMIT_S`; a `Show` during the fade ends the hiding.
#[test]
fn panels_show_and_hide_with_the_focus() {
    let mut f = flying(&sample());
    assert_eq!(f.state().on_screen, [Panel::Comms]);
    for (key, panel) in [("FocusLeftPanel", Panel::Left), ("FocusRightPanel", Panel::Right), ("FocusRadarPanel", Panel::Role)] {
        let m = panel.movie();
        for _ in 0..2 {
            let open = names(&f.input(key, true));
            assert_eq!(open, [(m, "Show"), (m, "FocusOn"), (m, "ReceivingButtons")], "{key} shows {panel:?}");
            let both: Vec<Panel> = Panel::ALL.into_iter().filter(|p| [panel, Panel::Comms].contains(p)).collect();
            assert_eq!(f.state().on_screen, both);
            let close = names(&f.input(key, true));
            assert_eq!(close, [(m, "FocusOff"), (m, "ReceivingButtons"), (m, "Hide")], "{key} again hides {panel:?}");
            assert_eq!(f.state().on_screen, both, "on screen while it fades");
            f.hear(&[PanelAction::HideComplete(m)]);
            assert_eq!(f.state().on_screen, [Panel::Comms], "gone once its hide is done");
        }
    }
    // The keys move from the left panel to the right: the left goes, the right comes, in that order.
    f.input("FocusLeftPanel", true);
    let (l, r) = (Movie::CockpitExternalPanel, Movie::CockpitInternalPanel2);
    assert_eq!(
        names(&f.input("FocusRightPanel", true)),
        [(l, "FocusOff"), (l, "ReceivingButtons"), (l, "Hide"), (r, "Show"), (r, "FocusOn"), (r, "ReceivingButtons")]
    );
    assert_eq!(f.state().on_screen, [Panel::Left, Panel::Right, Panel::Comms]);
    f.step(HIDE_LIMIT_S * 0.5);
    assert_eq!(f.state().on_screen, [Panel::Left, Panel::Right, Panel::Comms]);
    f.step(HIDE_LIMIT_S * 0.6);
    assert_eq!(f.state().on_screen, [Panel::Right, Panel::Comms], "a lost HideComplete keeps it no longer than the limit");
    // To the left panel (the right one fades), then to the right again before its HideComplete: shown again.
    f.input("FocusLeftPanel", true);
    f.input("FocusRightPanel", true);
    assert_eq!(f.state().on_screen, [Panel::Left, Panel::Right, Panel::Comms]);
    f.step(HIDE_LIMIT_S * 2.0);
    assert_eq!(f.state().on_screen, [Panel::Right, Panel::Comms], "the re-shown right panel is wanted, not hiding");
    // The comms panel: focus changes its level only.
    let c = Movie::CockpitChatPanel;
    f.input("FocusRightPanel", true);
    f.hear(&[PanelAction::HideComplete(r)]);
    let n = names(&f.input("FocusCommsPanel", true));
    assert_eq!(n, [(c, "FocusOn"), (c, "ReceivingButtons")]);
    let n = names(&f.input("FocusCommsPanel", true));
    assert_eq!(n, [(c, "FocusOff"), (c, "ReceivingButtons")], "no Hide for the comms bar");
    assert_eq!(f.state().on_screen, [Panel::Comms]);
    // Leaving the seat hides the comms bar (its `hide` is at once and says nothing back: it goes by the limit).
    let n = names(&f.seat(false));
    assert_eq!(n, [(c, "Hide")]);
    f.step(HIDE_LIMIT_S);
    assert!(f.state().on_screen.is_empty());
}

/// Sitting down opens the four panels and shows the comms bar, none focused; a focus key shows and focuses one (and its
/// buttons go to it, down and up); another moves the focus; leaving the seat hides what is shown.
#[test]
fn life_cycle_messages() {
    let bytes = sample();
    let mut f = Focus::from_json(&bytes).unwrap();
    assert_eq!(f.state().state, "Idle");
    assert!(!f.state().shown);
    let out = f.seat(true);
    let shown: Vec<_> = names(&out);
    for p in Panel::ALL {
        assert!(shown.contains(&(p.movie(), "Open")), "{p:?} opened");
    }
    let c = Movie::CockpitChatPanel;
    assert_eq!(shown[4..], [(c, "Show"), (c, "FocusOff")], "only the comms bar is shown in flight");
    assert!(f.seat(true).is_empty(), "already seated");
    assert!(!f.state().quiet_flight, "in flight the flight keys are the ship's");
    let out = f.input("FocusLeftPanel", true);
    let l = Movie::CockpitExternalPanel;
    assert_eq!(names(&out), [(l, "Show"), (l, "FocusOn"), (l, "ReceivingButtons")]);
    assert_eq!(out[2].msg().args, vec![Value::from(true)]);
    assert!(f.input("FocusLeftPanel", false).is_empty(), "a focus key's release is nothing");
    assert!(f.state().quiet_flight);
    // A UI button goes to the focused movie, pressed and released; the cycle names gain `UI_`.
    assert_eq!(f.input("UI_Down", true), vec![FocusOut::Button { movie: Movie::CockpitExternalPanel, name: "UI_Down", down: true }]);
    assert_eq!(f.input("UI_Down", false)[0].msg().args, vec![Value::from("UI_Down"), "buttonUp".into()]);
    assert_eq!(f.input("CycleNextPanel", true), vec![FocusOut::Button { movie: Movie::CockpitExternalPanel, name: "UI_CycleNextPanel", down: true }]);
    assert_eq!(f.input("CycleNextPanel", false).len(), 1);
    assert!(f.input("CycleNextPage", true).is_empty(), "the pagination set is the comms panel's only");
    assert!(f.input("ThrottleUp", true).is_empty());
    // Moving the focus to the right panel.
    let out = f.input("FocusRightPanel", true);
    assert_eq!(
        names(&out),
        [
            (Movie::CockpitExternalPanel, "FocusOff"),
            (Movie::CockpitExternalPanel, "ReceivingButtons"),
            (Movie::CockpitExternalPanel, "Hide"),
            (Movie::CockpitInternalPanel2, "Show"),
            (Movie::CockpitInternalPanel2, "FocusOn"),
            (Movie::CockpitInternalPanel2, "ReceivingButtons"),
        ]
    );
    // The comms panel's FocusOn takes its level; mini to full is off and on again.
    let out = f.input("FocusCommsPanel", true);
    let on = out.iter().map(FocusOut::msg).find(|m| m.name == "FocusOn").unwrap();
    assert_eq!((on.movie, on.args.clone()), (Movie::CockpitChatPanel, vec![Value::from(2)]));
    assert!(f.input("CycleNextPage", true).len() == 1, "pagination in the comms panel");
    f.input("FocusCommsPanel", true);
    assert_eq!(f.state().state, "Back From");
    f.input("QuickCommsPanel", true);
    assert_eq!(f.state().comms_level, Some(1));
    // Leaving the seat: the focused panel let go, the comms bar (the only panel shown) hidden.
    let out = f.seat(false);
    let n = names(&out);
    assert_eq!(n, [(c, "FocusOff"), (c, "ReceivingButtons"), (c, "Hide")]);
    assert_eq!(f.state().state, "Idle");
}

#[test]
fn a_bad_machine_is_refused() {
    assert!(Focus::from_json(b"[]").is_err());
    assert!(Focus::from_json(br#"{"initial": "Idle", "states": {"Idle": {"activities": [], "transitions": []}}}"#).is_err());
}

fn shipped() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../se_engine/assets/mods/sidewinder/ui/focus.json");
    std::fs::read(path).expect("the shipped focus.json")
}

#[test]
#[ignore = "reads the mod's shipped ui/focus.json (Elite's data, a file)"]
fn transitions_hold_and_head_on_the_shipped_machine() {
    let bytes = shipped();
    check_transitions(&bytes);
    check_hold(&bytes);
    check_head(&bytes);
    check_no_trap(&bytes);
}

/// The modal sets' members named here are the shipped `controls.json`'s.
#[test]
#[ignore = "reads the mod's shipped ui/controls.json (Elite's data, a file)"]
fn the_modal_sets_are_the_shipped_ones() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../se_engine/assets/mods/sidewinder/ui/controls.json");
    let controls: Json = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let set = |name: &str| -> Vec<String> {
        controls[name].as_array().unwrap().iter().filter_map(|e| e["name"].as_str()).map(str::to_string).collect()
    };
    let modal = set("bindings_interfacemodal");
    for b in MODAL_BUTTONS {
        assert!(modal.iter().any(|m| m == b), "{b} in the modal set");
    }
    let pages = set("bindings_interfacemodal_pagination");
    assert_eq!(pages, PAGINATION_BUTTONS);
}
