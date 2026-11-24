//! The game's side of Elite's cockpit focus machine (ARCHITECTURE.md A10 "Keys and focus (UI5)", task UI5b): `UiFocus` owns
//! `elite_ui::focus::Focus`, made from the shipped `ui/focus.json` (read with the UI's files: `load::PanelFiles`) once a
//! ship's UI is made, and runs it.
//!
//! - In: the seat's Elite UI buttons pressed and released this frame (`PilotKeys::ui`, from the shipped binds and the fixed
//!   `ui` set: `pilot::ui_edges`; a button pressed and released within one frame is pressed this frame and released the
//!   next), the focus key's LEVEL every seated frame (held only while it is down: a frame whose keys are not the pilot's,
//!   a menu up or the window not in front, counts as up, so a release lost meanwhile cannot leave the machine in
//!   `Select UI Mode`), the tests' presses (`sw ui key`), the panels' pop-ups and finished hides heard from the
//!   movies (`panels::UiPanels::heard`), the frame's seconds (`Focus::step`: a hide whose `HideComplete` is lost ends), and
//!   the seat: `Focus::seat(true)` every seated frame, `Focus::seat(false)` every frame out of the seat (each does nothing
//!   once settled), watched here so the seat never reads the UI.
//! - Out: the panels' life cycle and their buttons as movie messages (`UiMessages`), in the order the machine gives them;
//!   which panel has the keys (`panels::Panels::open`: the readers the stand-in panels had keep working, the head's aim
//!   `seat::panel_head`, the flight keys quieted, the GUI lamps), whether the flight keys the UI shares are quiet
//!   (`Panels::quiet`, the machine's `quiet_flight`), and which panel movies run and are drawn (`UiWanted`: the machine's
//!   `on_screen`, so a panel fades out by its own `Hide` and its opening sound plays on every `Show`).
//! - Also the scanner's range keys (`PilotKeys::radar_up`, `radar_down`) into `scanner::RadarKeys` each seated frame: the
//!   seat's keys reach the UI only through here.
//! - A panel movie's first load is lazy (`movies.rs`: when first on screen), so what the machine sends a panel movie that is
//!   not ready is dropped, and the frame after it becomes ready `Focus::reload_movie` gives its `Open` and what brings it to
//!   the current state. The machine is made afresh with each ship's UI (its movies are new).
//! - Who sends the panels' life cycle in Elite was not found (E, `elite_ui::focus`'s notes): this is the one place.

use bevy::prelude::*;
use elite_ui::feed::Movie;
use elite_ui::focus::{Focus, FocusOut, FocusState, Panel};

use super::super::panels::{Panels, Side};
use super::super::pilot::{PilotKeys, UI_BUTTONS};
use super::super::seat::Seat;
use super::load::PanelFiles;
use super::panels::UiPanels;
use super::place::UiDraw;
use super::scanner::RadarKeys;
use super::{seated, UiMessages, UiMovieState, UiSet, UiWanted};
use crate::mods::ModControl;

/// A test's tap (`sw ui key BUTTON tap`) is released this long after its press (s, real; as the runtime's proof presses keys:
/// `crates/elite-gfx/tests/panels_feed.rs`).
const TAP_S: f64 = 0.1;

/// The scanner's range actions a test may press with `sw ui key` (into `RadarKeys` as the seat's PageUp and PageDown do:
/// the test runs' key presser has no Page keys).
const RADAR_BUTTONS: [&str; 2] = ["RadarIncreaseRange", "RadarDecreaseRange"];

/// The comms panel's pages' control set's name in the focus machine's states (`ui/focus.json`, `ui/controls.json`).
const PAGINATION_SET_NAME: &str = "Bindings_InterfaceModal_Pagination";

/// The focus machine and what feeds it.
#[derive(Resource, Default)]
pub struct UiFocus {
    /// None until a ship's UI is made (or if its file failed: the panels then stay closed and the log says why).
    focus: Option<Focus>,
    /// Tests' presses waiting: (button, pressed, due at real seconds), taken in order.
    test_keys: Vec<(&'static str, bool, f64)>,
    /// The pop-ups the panels' movies said they show (`ExternalOnPopup` and the others: the last state each sent), for
    /// `sw ui focus`.
    popups: Vec<(Movie, i32)>,
    /// The machine's state last logged.
    logged: Option<String>,
    /// The focus key's level as the machine was last told it, and a test's hold of it (`sw ui key UIFocus down`).
    focus_held: bool,
    test_focus_held: bool,
    /// Buttons pressed and released within one frame: their release, given the next frame.
    pending_up: Vec<&'static str>,
    /// Elite's FSS screen covered the view last frame (`Seat::screen`).
    covered: bool,
}

/// The focus key (`UIFocus`): the machine is told its level, not its edges.
const UI_FOCUS: &str = "UIFocus";

impl UiFocus {
    /// What the machine's state says now (None before it is made).
    pub fn state(&self) -> Option<FocusState> {
        self.focus.as_ref().map(Focus::state)
    }
}

/// The stand-in carrier's side for a focused panel.
pub fn side_of(panel: Panel) -> Side {
    match panel {
        Panel::Left => Side::Left,
        Panel::Right => Side::Right,
        Panel::Role => Side::Role,
        Panel::Comms => Side::Comms,
    }
}

/// The view's field of view's channel in a state (degrees): its head look's first pair (`HeadLook::fov_deg`, A), 0 for a
/// state without one (the comms bar's, the flight's: no head look in Elite's data is no change).
pub fn view_channel(state: &FocusState) -> f32 {
    state.head.as_ref().map_or(0.0, |h| h.fov_deg as f32)
}

/// A focused panel's own key (its state's exit input: to `Back From`, or `Select UI Mode` and on to it with the focus key up).
fn own_key(panel: Panel) -> &'static str {
    match panel {
        Panel::Left => "FocusLeftPanel",
        Panel::Right => "FocusRightPanel",
        Panel::Comms => "FocusCommsPanel",
        Panel::Role => "FocusRadarPanel",
    }
}

/// The panel movies that run and are drawn: the machine's panels on screen.
pub fn running(state: &FocusState) -> Vec<Movie> {
    state.on_screen.iter().map(|p| p.movie()).collect()
}

/// The machine's messages to the movies, but those to a panel movie not ready (`reload_movie` brings it up when it is).
fn push(messages: &mut UiMessages, ready: &[Movie], outs: Vec<FocusOut>) {
    messages.msgs.extend(outs.iter().map(FocusOut::msg).filter(|m| ready.contains(&m.movie)));
}

/// A ship's UI has just been made: a fresh machine (in `Idle`; its first messages open the four movies).
fn make(files: Option<Res<PanelFiles>>, mut ui: ResMut<UiFocus>) {
    ui.popups.clear();
    ui.logged = None;
    let made = files.ok_or_else(|| "its files are not there".to_string()).and_then(|f| Focus::from_json(&f.focus));
    ui.focus = match made {
        Ok(f) => Some(f),
        Err(e) => {
            warn!("sidewinder ui: the focus machine is not made ({e}): Elite's panels stay closed");
            None
        }
    };
}

/// Out of the seat: the machine back to `Idle` (its panels hidden), its hides timed. (`seat::free_seat` closes the carrier.)
fn leave(time: Res<Time<Real>>, seat: Res<Seat>, control: Res<ModControl>, movies: Res<UiMovieState>, mut ui: ResMut<UiFocus>, mut messages: ResMut<UiMessages>) {
    if seated(&seat, &control) {
        return;
    }
    ui.focus_held = false;
    ui.test_focus_held = false;
    ui.pending_up.clear();
    ui.covered = false;
    let Some(focus) = ui.focus.as_mut() else { return };
    focus.step(time.delta_secs_f64());
    let outs = focus.seat(false);
    if !outs.is_empty() {
        info!("sidewinder ui: out of the seat: the focus machine is in {}", focus.state().state);
        push(&mut messages, &movies.ready, outs);
    }
}

/// Each seated frame: the movies' events heard, the hides timed, the panel movies just ready brought up, the seat, the
/// frame's buttons; then what the state says goes to the carrier and the wanted movies; the scanner's keys handed on.
#[allow(clippy::too_many_arguments)]
pub(super) fn step(
    time: Res<Time<Real>>,
    keys: Res<PilotKeys>,
    heard: Res<UiPanels>,
    movies: Res<UiMovieState>,
    mut ui: ResMut<UiFocus>,
    mut panels: ResMut<Panels>,
    mut messages: ResMut<UiMessages>,
    mut wanted: ResMut<UiWanted>,
    mut radar: ResMut<RadarKeys>,
    seat: Res<Seat>,
) {
    let now = time.elapsed_secs_f64();
    let ui = &mut *ui;
    radar.up = keys.radar_up;
    radar.down = keys.radar_down;
    let Some(focus) = ui.focus.as_mut() else { return };
    focus.hear(heard.heard());
    for a in heard.heard() {
        if let elite_ui::panels::PanelAction::Popup { movie, state } = *a {
            ui.popups.retain(|(m, _)| *m != movie);
            ui.popups.push((movie, state));
        }
    }
    focus.step(time.delta_secs_f64());
    let fresh = movies.fresh.iter().filter(|m| Panel::ALL.iter().any(|p| p.movie() == **m));
    let mut outs: Vec<FocusOut> = fresh.flat_map(|m| focus.reload_movie(*m)).collect();
    outs.extend(focus.seat(true));
    let due = ui.test_keys.iter().take_while(|(_, _, at)| *at <= now).count();
    let tests: Vec<(&'static str, bool, f64)> = ui.test_keys.drain(..due).collect();
    for &(button, down, _) in &tests {
        info!("sidewinder ui: key {button} {} (a test)", if down { "down" } else { "up" });
        if button == UI_FOCUS {
            ui.test_focus_held = down;
        }
    }
    // Elite's FSS screen covering the view (`Seat::screen`, last frame's: `ui::screens` runs after this): Elite's
    // `Exploration_FSS` state has only the FSS's bindings live and no `CockpitFocusInputActivity`, so the machine gets no UI
    // key and the focus key counts as up (as under a menu). Elite enters it through `CockpitTransitionState`, which shows the
    // panels and focuses none: the panel that had the keys loses them and hides. The machine here cannot enter that state,
    // so the panel's own key (its exit input) does it: `FocusOff`, `Hide` (its fade and closing sound), the machine in
    // `Back From`, the flight state the panels come back to, when the screen closes.
    let covered = seat.screen;
    let opened = covered && !ui.covered;
    ui.covered = covered;
    // The focus key's level first (a direction pressed in the frame it goes down finds `Select UI Mode`).
    let held = !covered && (keys.ui.is_held(UI_FOCUS) || ui.test_focus_held);
    if held != ui.focus_held {
        ui.focus_held = held;
        outs.extend(focus.input(UI_FOCUS, held));
    }
    if let Some(panel) = focus.state().focused.filter(|_| opened) {
        info!("sidewinder ui: the FSS covers the view: the {panel:?} panel left");
        outs.extend(focus.input(own_key(panel), true));
        outs.extend(focus.input(own_key(panel), false));
    }
    for button in std::mem::take(&mut ui.pending_up) {
        outs.extend(focus.input(button, false));
    }
    let edges: Vec<(&'static str, bool)> = if covered { Vec::new() } else { keys.ui.iter().filter(|(b, _)| *b != UI_FOCUS).collect() };
    for &(button, down) in &edges {
        if !down && edges.contains(&(button, true)) {
            ui.pending_up.push(button);
        } else {
            outs.extend(focus.input(button, down));
        }
    }
    for (button, down, _) in tests.into_iter().filter(|(b, _, _)| *b != UI_FOCUS) {
        match RADAR_BUTTONS.iter().position(|b| *b == button) {
            Some(0) => radar.up |= down,
            Some(_) => radar.down |= down,
            None if covered => {}
            None => outs.extend(focus.input(button, down)),
        }
    }
    push(&mut messages, &movies.ready, outs);
    let state = focus.state();
    if ui.logged.as_deref() != Some(state.state.as_str()) {
        info!(
            "sidewinder ui: focus {} (focused {:?}, on screen {:?}, quiet {}, head {:?})",
            state.state,
            state.focused,
            state.on_screen,
            state.quiet_flight,
            state.head.as_ref().and_then(|h| h.joint.as_deref())
        );
        ui.logged = Some(state.state.clone());
    }
    panels.open = state.focused.map(side_of);
    panels.quiet = state.quiet_flight;
    panels.pages = state.control_sets.iter().any(|s| s == PAGINATION_SET_NAME);
    panels.fov_add_deg = Some(view_channel(&state));
    panels.by_look = false;
    let run = running(&state);
    wanted.movies.retain(|m| !Panel::ALL.iter().any(|p| p.movie() == *m) || run.contains(m));
    for m in run {
        if !wanted.movies.contains(&m) {
            wanted.movies.push(m);
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<UiFocus>()
        .add_systems(PostUpdate, (make.run_if(resource_added::<UiDraw>), leave.run_if(resource_exists::<UiDraw>)).chain().before(UiSet::Feed).after(super::super::facts::update))
        .add_systems(PostUpdate, step.in_set(UiSet::Feed).run_if(resource_exists::<UiDraw>));
}

/// `sw ui key BUTTON down|up|tap`: an Elite UI button (`pilot::UI_BUTTONS`) into the focus machine as a key gives it, the
/// next seated frame (a tap releases it `TAP_S` later); or a scanner range action (`RADAR_BUTTONS`) into `RadarKeys`.
pub fn dev_key(world: &mut World, args: &str) {
    let mut words = args.split_whitespace();
    let (Some(name), Some(how)) = (words.next(), words.next()) else {
        warn!("sw ui key: use `sw ui key BUTTON down|up|tap`; got `{args}`");
        return;
    };
    let Some(button) = UI_BUTTONS.iter().chain(&RADAR_BUTTONS).copied().find(|b| b.eq_ignore_ascii_case(name)) else {
        warn!("sw ui key: {name} is not one of Elite's UI buttons: {UI_BUTTONS:?} {RADAR_BUTTONS:?}");
        return;
    };
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let presses: &[(bool, f64)] = match how.to_ascii_lowercase().as_str() {
        "down" => &[(true, 0.0)],
        "up" => &[(false, 0.0)],
        "tap" => &[(true, 0.0), (false, TAP_S)],
        _ => {
            warn!("sw ui key: down, up or tap; got `{how}`");
            return;
        }
    };
    let mut ui = world.resource_mut::<UiFocus>();
    for &(down, after) in presses {
        ui.test_keys.push((button, down, now + after));
    }
    ui.test_keys.sort_by(|a, b| a.2.total_cmp(&b.2));
}

/// `sw ui focus`: the machine's state, the panel movies wanted, the pop-ups the movies said they show.
pub fn dev_focus(world: &mut World) {
    let wanted: Vec<&str> = world.resource::<UiWanted>().movies.iter().filter(|m| Panel::ALL.iter().any(|p| p.movie() == **m)).map(|m| m.name()).collect();
    let ui = world.resource::<UiFocus>();
    match ui.state() {
        Some(s) => info!(
            "sidewinder ui: focus state {}, shown {}, focused {:?} (comms level {:?}), quiet flight {}, head {:?}, sets {:?}; panels wanted {wanted:?}; pop-ups {:?}; test keys waiting {}",
            s.state,
            s.shown,
            s.focused,
            s.comms_level,
            s.quiet_flight,
            s.head,
            s.control_sets,
            ui.popups.iter().filter(|(_, st)| *st != 0).map(|(m, st)| format!("{} {st}", m.name())).collect::<Vec<_>>(),
            ui.test_keys.len()
        ),
        None => info!("sidewinder ui: the focus machine is not made (no ship's UI yet, or its file failed)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Focus {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui/focus.json");
        Focus::from_json(&std::fs::read(path).unwrap()).unwrap()
    }

    /// Every button the seat reads is one the shipped binds bind or the fixed set gives, and every input the shipped machine's
    /// reached states name (but the cut maps') is one the seat reads.
    #[test]
    fn the_seat_reads_every_button_the_machine_takes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder");
        let binds = se_core::elite_asset::Binds::parse(&std::fs::read_to_string(dir.join("binds/KeyboardMouseOnly.binds")).unwrap()).unwrap();
        for b in UI_BUTTONS {
            let bound = binds.get(b).is_some_and(|a| a.keys().any(|k| !k.key.is_empty()));
            assert!(bound || b == "CyclePreviousPage" || b == "CycleNextPage", "{b} has no key in the shipped binds");
        }
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("ui/focus.json")).unwrap()).unwrap();
        for state in ["In Flight", "Back From", "Select UI Mode", "External Panel", "Internal Panel", "Comms Panel Full", "Comms Panel Mini", "Role Switch"] {
            for a in raw["states"][state]["activities"].as_array().unwrap().iter().filter(|a| a["type"] == "CockpitFocusInputActivity") {
                let inputs = a["inputs"].as_array().unwrap().iter().map(|p| p[0].as_str().unwrap());
                let exits = a["exit_inputs"].as_array().unwrap().iter().map(|e| e.as_str().unwrap());
                for input in inputs.chain(exits).filter(|i| !i.ends_with("MapOpen")) {
                    assert!(UI_BUTTONS.contains(&input), "{state}: {input} is not read by the seat");
                }
            }
        }
    }

    /// The carrier agrees with the machine: in every state the keys reach, a focused panel quiets the flight keys (so
    /// `Panels::open` or `Panels::quiet` is exactly the machine's `quiet_flight`), and the movies that run include the comms
    /// panel and the focused one (a panel losing the keys runs on while it fades: `on_screen`).
    #[test]
    fn the_carrier_follows_the_machine() {
        let mut f = machine();
        f.seat(true);
        let mut seen = Vec::new();
        // (Each panel opened by its key and closed by it again; then UIFocus held, a direction, the release.)
        let taps: [&[&str]; 6] = [
            &["FocusLeftPanel", "FocusLeftPanel"],
            &["FocusRightPanel", "FocusRightPanel"],
            &["FocusRadarPanel", "FocusRadarPanel"],
            &["FocusCommsPanel", "FocusCommsPanel"],
            &["QuickCommsPanel", "FocusCommsPanel"],
            &["UIFocus"],
        ];
        let mut presses: Vec<(&str, bool)> = taps.iter().flat_map(|t| t.iter().flat_map(|&b| [(b, true), (b, false)])).collect();
        presses.truncate(presses.len() - 1);
        presses.extend([("UI_Down", true), ("UI_Down", false), ("UIFocus", false), ("FocusRadarPanel", true), ("FocusRadarPanel", false)]);
        {
            for &(b, d) in &presses {
                f.input(b, d);
                let s = f.state();
                let open = s.focused.map(side_of);
                assert!(open.is_none() || s.quiet_flight, "{}: a focused panel with the flight keys live", s.state);
                let run = running(&s);
                assert!(run.contains(&Movie::CockpitChatPanel));
                assert!(s.focused.is_none_or(|p| run.contains(&p.movie())), "{}: the focused panel runs", s.state);
                seen.push((s.state.clone(), open, s.quiet_flight));
            }
        }
        let has = |state: &str, open: Option<Side>, quiet: bool| seen.iter().any(|s| s.0 == state && s.1 == open && s.2 == quiet);
        assert!(has("External Panel", Some(Side::Left), true));
        assert!(has("Internal Panel", Some(Side::Right), true));
        assert!(has("Role Switch", Some(Side::Role), true));
        assert!(has("Comms Panel Full", Some(Side::Comms), true));
        assert!(has("Comms Panel Mini", Some(Side::Comms), true));
        // UIFocus held alone: no panel has the keys, yet the flight keys the UI shares are quiet (the machine's word).
        assert!(has("Select UI Mode", None, true));
        assert!(has("Back From", None, false));
    }

    /// The view's channel is the state's head look's: the side and role panels -17, the full comms panel -8, `Select UI Mode`
    /// +10, the comms bar (no head look) and the flight 0. And each focused panel's own key leaves it with the focus key up
    /// (what the FSS screen's opening does).
    #[test]
    fn the_view_follows_the_states_head_look() {
        let mut f = machine();
        f.seat(true);
        let mut tap = |f: &mut Focus, b: &str| {
            f.input(b, true);
            f.input(b, false);
        };
        assert_eq!(view_channel(&f.state()), 0.0, "flight");
        for (key, state, deg) in [("FocusLeftPanel", "External Panel", -17.0), ("FocusRightPanel", "Internal Panel", -17.0), ("FocusRadarPanel", "Role Switch", -17.0), ("FocusCommsPanel", "Comms Panel Full", -8.0), ("QuickCommsPanel", "Comms Panel Mini", 0.0)] {
            tap(&mut f, key);
            let s = f.state();
            assert_eq!((s.state.as_str(), view_channel(&s)), (state, deg));
            tap(&mut f, own_key(s.focused.unwrap()));
            assert!(f.state().focused.is_none(), "{state}: its own key leaves it");
            assert_eq!(view_channel(&f.state()), 0.0);
        }
        f.input("UIFocus", true);
        assert_eq!((f.state().state.as_str(), view_channel(&f.state())), ("Select UI Mode", 10.0));
        f.input("UIFocus", false);
        assert_eq!(view_channel(&f.state()), 0.0);
    }

    /// The tests' button names are the seat's own: any case is taken, an unknown one is refused.
    #[test]
    fn a_test_key_is_one_of_the_buttons() {
        let mut world = World::new();
        world.init_resource::<Time<Real>>();
        world.init_resource::<UiFocus>();
        dev_key(&mut world, "focusleftpanel tap");
        dev_key(&mut world, "NoSuchButton down");
        let keys = &world.resource::<UiFocus>().test_keys;
        assert_eq!(keys.iter().map(|k| (k.0, k.1)).collect::<Vec<_>>(), [("FocusLeftPanel", true), ("FocusLeftPanel", false)]);
        assert!((keys[1].2 - keys[0].2 - TAP_S).abs() < 1e-9);
    }
}
