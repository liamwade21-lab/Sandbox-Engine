//! Which cockpit panel has the keys (ARCHITECTURE.md A10 "Keys and focus (UI5)"): Elite's `cockpitui` state machine run as
//! data. The machine is the shipped `ui/focus.json` (the export's `cockpitui_state_machine.json`, unchanged), read by
//! `Focus::from_json`; in go Elite's own button names (the mod's `.binds` and the fixed `ui` set), out come what the game
//! must do (`FocusOut`: the panels' life-cycle messages and the UI buttons routed to the focused movie) and what the state
//! says (`FocusState`: the focused panel, its head-look joint and field-of-view channel, the live control sets).
//!
//! Sources (`re/elite/notes/ui_rendering.md` section 4, `ui_messages_panels.md` part 1 7.B.5 and 7.D):
//! - The transitions are each state's `CockpitFocusInputActivity` records, in order: an input of `inputs` goes to its
//!   state, an `exit_inputs` one to `exit_target` (A: the data). Within a record the `inputs` are looked at before the
//!   `exit_inputs`, and the first record that names the button wins (the code's own rule, B). It decides one case:
//!   `Comms Panel Full` and `Mini` name `FocusCommsPanel` both as an input (to `Back From`) and as an exit (to
//!   `Select UI Mode`); taking the input, the comms key closes the comms panel as the other panels' own keys close theirs
//!   (and the left and role panels' exits go to `Back From` too), which is why the input is preferred.
//! - The panels are shown in every state that runs `CockpitShowPanelActivity` and hidden in the others (B: its Start and
//!   End clear and set the panel hide mask's bit 5, 7.D section 6).
//! - Which panel is focused is `CockpitFocusPanelActivity`'s per-state data, five numbers (external, internal, comms, role
//!   switch, commander: 0 untouched, 1 shown, 2 focused; the comms 2 carries a variant, 2 full and 1 mini). Those numbers
//!   are NOT in the shipped file (the converter's builder kept only the activity's name), so they are `FOCUS_LEVELS`
//!   below, copied from the notes' table (A: `re/elite/work/ui/render/cockpitui_csm.json`), and used only in the states
//!   whose data has the activity.
//! - The head look is `HeadlookTargetBoneActivity`'s data: its panel component, its joint and its field-of-view channel,
//!   normal and second (A, 4.3).
//! - `UIFocus` is held (the user's binds: `UIFocusMode = Bindings_FocusModeHold`; `Focus::hold`). The rule built (B: the
//!   binds' hold mode read against the data; C: in session ui1 a panel's own key closed it and the view came back):
//!   a "hold state" is one whose data lists `UIFocus` among its `inputs` (in the shipped machine, `Select UI Mode`, whose
//!   `UIFocus` input leads to `Back From`). A hold state is kept only while `UIFocus` is down: entered by any path while it
//!   is up (a right or comms panel's exits go there), the state's own `UIFocus` input is taken at once; `UIFocus`'s release
//!   in a hold state takes that input however the state was entered (held, a direction then 4 or `UI_Back` comes back to
//!   `Select UI Mode` with the key still down: the release leaves it). Held, a direction opens a panel and the release
//!   leaves it open (a panel's state is no hold state); released alone, `Select UI Mode` goes to `Back From`. A press while
//!   a panel is focused leaves the panel as the data says and its release does nothing more. `Back From` is the flight state the panels come
//!   back to: its inputs are `In Flight`'s and neither has a focused panel, the modal set or a head look (B: the state
//!   machine leaves it for `In Flight` through `CockpitTransitionState` by game logic not decoded). In toggle mode the
//!   release is never an input and nothing is left at once.
//! - Who sends `Open`, `Show`, `Hide`, `FocusOn`, `FocusOff` and `ReceivingButtons` to the side panels was never found (E,
//!   7.D sections 3 and 8: no function sends them by name; `Open` is the generic movie class's, A). This machine is the one
//!   place that sends a panel's life cycle (stand-in, B: 7.B.5.3; C for the sounds and fades: in session ui1 Elite played
//!   a panel's opening sound every time it was focused, keys 1, 3 and 4, +0.11 s, and its close was mostly done by
//!   0.3 s, which is the movies' own reaction to `Show`, its `showBegin` and show effect, and to `Hide`, its fade and
//!   `hide`). The first messages it ever sends (and the first after `reload`) are `Open` to the four. The comms panel is
//!   shown while the state shows the panels (its bar is in the cockpit in flight); the left, right and role panels are
//!   shown only while they have the keys (`wanted`). A change sends, in this order: the buttons the panel losing the keys
//!   still holds released, its `FocusOff` and `ReceivingButtons(false)`; `Hide` to each panel no longer wanted; `Show` to
//!   each panel newly wanted (with `FocusOff` if it has no keys); `FocusOn` and `ReceivingButtons(true)` to the panel
//!   gaining the keys (the comms panel's `FocusOn` takes its level: 2 full, 1 mini, `Content.focusOnMovie(level)`, which
//!   ignores a level not above the current one, so a change of level is a `FocusOff` and a new `FocusOn`). Why that order
//!   is in `changes`. A hidden panel stays on screen (`FocusState::on_screen`) until its `HideComplete` comes back.
//! - `RedirectRoleSwitch` goes on at once to `Role Switch` (stand-in, B: its `RedirectRolePanelActivity` chooses between
//!   `Role Switch` and `Radar Panel`, rule not decoded; the stock Sidewinder has the role panel: session ui1, key 3).
//! - A UI button that no transition takes, pressed while a panel is focused and its control set is live, goes to that
//!   panel's movie as `OnInputButton(name, "buttonDown" | "buttonUp")` (A: `ui_rendering.md` 4.1; the movie does its own key
//!   repeat, so only the press and the release are sent; a release goes only to the movie that got the press). A button a
//!   transition takes is not sent to the movie (ours, E: whether Elite also hands it to the movie was not decoded).
//! - With a pop-up open on the focused panel, `UI_Back` is the pop-up's (A: `InfoPopup.handleInput` and
//!   `ModuleManagementPopup.handleInput` take `InputButton.CANCEL` themselves: the location pop-up presses its Back, the
//!   module pop-up closes; C: in session ui1 BACK closed the pop-up and the panel stayed open): it goes to the movie and the
//!   machine does not take it. The machine learns of a pop-up from the movies' own events (`ExternalOnPopup`,
//!   `InternalOnPopup`, `InternalOnModulePopup`: `Focus::hear`, the actions `panels::on_out` makes of them).
//! - Only the states a lone ship reaches are driven: `Idle`, the flight states and the four panels' states. The maps'
//!   inputs and every state the data names that is not in `REACHED` are ignored (the maps are cut: the user's scope).

use crate::feed::{Movie, Msg};
use crate::panels::PanelAction;
use crate::Value;
use serde_json::Value as Json;
use std::collections::HashMap;

/// The four cockpit panels, in the focus data's order (external, internal, comms, role switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Left,
    Right,
    Comms,
    Role,
}

impl Panel {
    pub const ALL: [Panel; 4] = [Panel::Left, Panel::Right, Panel::Comms, Panel::Role];

    /// The panel's movie.
    pub fn movie(self) -> Movie {
        match self {
            Panel::Left => Movie::CockpitExternalPanel,
            Panel::Right => Movie::CockpitInternalPanel2,
            Panel::Comms => Movie::CockpitChatPanel,
            Panel::Role => Movie::RolePanel3,
        }
    }
}

/// `CockpitFocusPanelActivity`'s numbers per state (A: notes part 1 7.B.5.1's table, from `cockpitui_csm.json`): the
/// external, internal, comms and role-switch panels' levels (0 untouched, 1 shown, 2 focused) and the comms panel's variant
/// (2 full, 1 mini). The commander panel's fifth number is left out (no such panel in the mod). States not listed have
/// zeros (the notes: In Flight, Docked, Radar Panel...).
const FOCUS_LEVELS: [(&str, [u8; 4], u8); 5] = [
    ("External Panel", [2, 1, 1, 1], 0),
    ("Internal Panel", [1, 2, 1, 1], 0),
    ("Comms Panel Full", [1, 1, 2, 1], 2),
    ("Comms Panel Mini", [1, 1, 2, 1], 1),
    ("Role Switch", [1, 1, 1, 2], 0),
];

/// The states a lone ship reaches (the brief's list; the notes' 4.2): every other target is ignored.
const REACHED: [&str; 13] = [
    "Idle",
    "CockpitTransitionState",
    "In Flight",
    "Back From",
    "Select UI Mode",
    "External Panel",
    "Internal Panel",
    "Comms Panel Full",
    "Comms Panel Mini",
    "RedirectRoleSwitch",
    "Role Switch",
    "Radar Panel",
    "Launching",
];

/// The flight state the cockpit settles in once someone sits (the CSM's own path: `Idle` -> `CockpitTransitionState` ->
/// `In Flight`, its transitions' targets).
const FLIGHT: &str = "In Flight";
const IDLE: &str = "Idle";
/// The role panel's redirect and where it goes (stand-in, see the module's notes).
const REDIRECT: &str = "RedirectRoleSwitch";
const ROLE_SWITCH: &str = "Role Switch";

/// The control set that makes the UI buttons the panel's (A: `ui_rendering.md` 4.2, `controls.json`
/// `bindings_interfacemodal`) and its members; the comms states' pagination set and its members.
const MODAL_SET: &str = "Bindings_InterfaceModal";
const MODAL_BUTTONS: [&str; 9] =
    ["UI_Up", "UI_Down", "UI_Left", "UI_Right", "UI_Select", "CyclePreviousPanel", "CycleNextPanel", "UI_Back", "UI_Toggle"];
const PAGINATION_SET: &str = "Bindings_InterfaceModal_Pagination";
const PAGINATION_BUTTONS: [&str; 2] = ["CyclePreviousPage", "CycleNextPage"];

/// The button as a movie knows it (A: `EliteInputButton.as`; the binds' cycle names gain `UI_`).
fn movie_button(button: &str) -> Option<&'static str> {
    Some(match button {
        "UI_Up" => "UI_Up",
        "UI_Down" => "UI_Down",
        "UI_Left" => "UI_Left",
        "UI_Right" => "UI_Right",
        "UI_Select" => "UI_Select",
        "UI_Back" => "UI_Back",
        "UI_Toggle" => "UI_Toggle",
        "CyclePreviousPanel" => "UI_CyclePreviousPanel",
        "CycleNextPanel" => "UI_CycleNextPanel",
        "CyclePreviousPage" => "UI_CyclePreviousPage",
        "CycleNextPage" => "UI_CycleNextPage",
        _ => return None,
    })
}

/// The held focus button (the binds' `UIFocus`).
const UI_FOCUS: &str = "UIFocus";

/// One `CockpitFocusInputActivity`: inputs to states, and the exit inputs to the exit state.
#[derive(Clone, Debug, Default, PartialEq)]
struct InputActivity {
    inputs: Vec<(String, String)>,
    exits: Vec<String>,
    exit_target: Option<String>,
}

/// The head look a state asks for (`HeadlookTargetBoneActivity`, A).
#[derive(Clone, Debug, PartialEq)]
pub struct HeadLook {
    /// The panel component the head aims at (`ExternalDataPanel`...), if any.
    pub panel_component: Option<String>,
    /// The cockpit's joint the head aims at (`LeftPanel_Focus`...), if any (`Select UI Mode` has none: it only widens).
    pub joint: Option<String>,
    /// The field-of-view channel, degrees: normal and second (the activity's two pairs; which one Elite uses depends on a
    /// camera setting, 4.3).
    pub fov_deg: f64,
    pub fov_deg_second: f64,
}

/// One state of the machine, as the data gives it.
#[derive(Clone, Debug, Default)]
struct State {
    inputs: Vec<InputActivity>,
    control_sets: Vec<String>,
    shows_panels: bool,
    focus_panel: bool,
    head: Option<HeadLook>,
}

/// What the current state says.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusState {
    /// The machine's state's name.
    pub state: String,
    /// The panels are shown (`CockpitShowPanelActivity`).
    pub shown: bool,
    /// The panel that has the keys, if any.
    pub focused: Option<Panel>,
    /// The comms panel's focus level when it has the keys: 2 full, 1 mini.
    pub comms_level: Option<i32>,
    /// The head look the state asks for, if any.
    pub head: Option<HeadLook>,
    /// The control sets live in the state (`ControlInputScheme`, names only: their flags were not decoded).
    pub control_sets: Vec<String>,
    /// The panels' modal set is live: the UI buttons are the panel's, so the flight keys that share their keys are quiet.
    pub quiet_flight: bool,
    /// The panels on screen, in `Panel::ALL`'s order: their movies are drawn, stepped and fed. The comms panel while the
    /// panels are shown, the focused panel, and a panel that was sent `Hide` until its `HideComplete` (or `HidingPanel`)
    /// comes back through `Focus::hear`, at most `HIDE_LIMIT_S` (`Focus::step`).
    pub on_screen: Vec<Panel>,
}

/// The longest a hiding panel stays on screen without its `HideComplete`: twice the movies' fade (A: `Content.hide`
/// tweens alpha to 0 in `PANEL_SHOW_TIME`, 0.5 s, then says `HideComplete`), so a lost event cannot keep it.
pub const HIDE_LIMIT_S: f64 = 1.0;

/// What the game must do after an input: a message to a panel's movie (`Show`, `Hide`, `FocusOn`, `FocusOff`,
/// `ReceivingButtons`), or a UI button for the focused movie (`OnInputButton`).
#[derive(Clone, Debug, PartialEq)]
pub enum FocusOut {
    Send(Msg),
    Button { movie: Movie, name: &'static str, down: bool },
}

impl FocusOut {
    /// As the message the movie gets.
    pub fn msg(&self) -> Msg {
        match self {
            FocusOut::Send(m) => m.clone(),
            FocusOut::Button { movie, name, down } => Msg {
                movie: *movie,
                name: "OnInputButton",
                args: vec![Value::from(*name), Value::from(if *down { "buttonDown" } else { "buttonUp" })],
            },
        }
    }
}

/// The machine.
#[derive(Clone, Debug)]
pub struct Focus {
    states: HashMap<String, State>,
    current: String,
    /// `UIFocus` works in hold mode (the user's binds; false: toggle mode).
    pub hold: bool,
    /// `UIFocus` is down now.
    focus_down: bool,
    /// The four movies have been sent `Open` (since they were loaded).
    opened: bool,
    /// The movies with a pop-up open (from their own events).
    popups: Vec<Movie>,
    /// The UI buttons pressed to a movie and not yet released to it.
    pressed: Vec<(Movie, &'static str)>,
    /// The panels sent `Hide` whose `HideComplete` has not come back, with the seconds since (`step`).
    hiding: Vec<(Panel, f64)>,
}

impl Focus {
    fn empty() -> Focus {
        Focus {
            states: HashMap::new(),
            current: String::new(),
            hold: true,
            focus_down: false,
            opened: false,
            popups: Vec::new(),
            pressed: Vec::new(),
            hiding: Vec::new(),
        }
    }

    /// The machine from the shipped `focus.json` (`{"initial", "states": {NAME: {"activities", "transitions"}}}`). It
    /// starts in the data's initial state (`Idle`).
    pub fn from_json(bytes: &[u8]) -> Result<Focus, String> {
        let raw: Json = serde_json::from_slice(bytes).map_err(|e| format!("the focus machine is not JSON: {e}"))?;
        let initial = raw["initial"].as_str().ok_or("the focus machine has no initial state")?.to_string();
        let list = raw["states"].as_object().ok_or("the focus machine has no states")?;
        let mut states = HashMap::new();
        for (name, s) in list {
            let mut state = State::default();
            for a in s["activities"].as_array().ok_or_else(|| format!("state {name} has no activities"))? {
                match a["type"].as_str() {
                    Some("ControlInputScheme") => {
                        for set in a["control_sets"].as_array().into_iter().flatten().filter_map(Json::as_str) {
                            if !state.control_sets.iter().any(|s| s == set) {
                                state.control_sets.push(set.to_string());
                            }
                        }
                    }
                    Some("CockpitFocusInputActivity") => state.inputs.push(input_activity(a)),
                    Some("CockpitShowPanelActivity") => state.shows_panels = true,
                    Some("CockpitFocusPanelActivity") => state.focus_panel = true,
                    Some("HeadlookTargetBoneActivity") => {
                        state.head = Some(HeadLook {
                            panel_component: a["panel_component"].as_str().map(str::to_string),
                            joint: a["bone"].as_str().map(str::to_string),
                            fov_deg: a["fov_channel_deg"].as_f64().unwrap_or(0.0),
                            fov_deg_second: a["fov_channel_deg_alt"].as_f64().unwrap_or(0.0),
                        })
                    }
                    _ => {}
                }
            }
            states.insert(name.clone(), state);
        }
        for needed in REACHED {
            if !states.contains_key(needed) {
                return Err(format!("the focus machine has no state {needed}"));
            }
        }
        if !states.contains_key(&initial) {
            return Err(format!("the focus machine's initial state {initial} is not one of its states"));
        }
        Ok(Focus { states, current: initial, ..Focus::empty() })
    }

    /// What the current state says.
    pub fn state(&self) -> FocusState {
        let s = &self.states[&self.current];
        let levels = if s.focus_panel {
            FOCUS_LEVELS.iter().find(|(n, _, _)| *n == self.current).map(|(_, l, v)| (*l, *v))
        } else {
            None
        };
        let focused = levels.and_then(|(l, _)| Panel::ALL.iter().zip(l).find(|(_, lv)| *lv == 2).map(|(p, _)| *p));
        let comms_level = levels.filter(|_| focused == Some(Panel::Comms)).map(|(_, v)| i32::from(v));
        let on_screen = Panel::ALL
            .into_iter()
            .filter(|p| wanted(s.shows_panels, focused, *p) || self.hiding.iter().any(|(h, _)| h == p))
            .collect();
        FocusState {
            state: self.current.clone(),
            shown: s.shows_panels,
            focused,
            comms_level,
            head: s.head.clone(),
            control_sets: s.control_sets.clone(),
            quiet_flight: s.control_sets.iter().any(|c| c == MODAL_SET),
            on_screen,
        }
    }

    /// Someone sits in the seat or leaves it. Seated, the cockpit settles in `In Flight` by the machine's own path (`Idle`
    /// -> `CockpitTransitionState` -> `In Flight`, its transitions' targets; the transition state only passes, so only the
    /// end counts); leaving goes back to `Idle` from wherever it is. Nothing happens if it is already so.
    pub fn seat(&mut self, seated: bool) -> Vec<FocusOut> {
        let settled = if seated { self.current != IDLE } else { self.current == IDLE };
        if settled {
            return Vec::new();
        }
        let before = self.state();
        self.enter(if seated { FLIGHT } else { IDLE });
        let out = self.changes(&before);
        self.deliver(out)
    }

    /// The four movies have just been loaded again: they are opened and brought to the current state (shown, focused).
    pub fn reload(&mut self) -> Vec<FocusOut> {
        self.opened = false;
        self.popups.clear();
        self.pressed.clear();
        self.hiding.clear();
        let out = self.changes(&self.blank());
        self.deliver(out)
    }

    /// One panel's movie has just been loaded (the game loads each the first time it is wanted, so what was sent to it
    /// before is lost): `Open` to it and the messages that bring it to the current state (its `Show`, its focus). Nothing
    /// before the machine's first messages (they open all four).
    pub fn reload_movie(&mut self, movie: Movie) -> Vec<FocusOut> {
        if !self.opened {
            return Vec::new();
        }
        self.popups.retain(|m| *m != movie);
        self.pressed.retain(|(m, _)| *m != movie);
        self.hiding.retain(|(p, _)| p.movie() != movie);
        let open = FocusOut::Send(Msg { movie, name: "Open", args: vec![] });
        let now = self.state();
        std::iter::once(open).chain(changes(&self.blank(), &now).into_iter().filter(|o| o.msg().movie == movie)).collect()
    }

    /// The current state with nothing on screen and nothing focused (the movies as loaded).
    fn blank(&self) -> FocusState {
        FocusState { shown: false, focused: None, comms_level: None, on_screen: Vec::new(), ..self.state() }
    }

    /// What the movies sent back (`panels::on_out`'s actions): the pop-ups they open and close, and the hides they
    /// finish.
    pub fn hear(&mut self, actions: &[PanelAction]) {
        for action in actions {
            match *action {
                PanelAction::Popup { movie, state } => {
                    self.popups.retain(|m| *m != movie);
                    if state != 0 {
                        self.popups.push(movie);
                    }
                }
                PanelAction::HideComplete(movie) => self.hiding.retain(|(p, _)| p.movie() != movie),
                _ => {}
            }
        }
    }

    /// The game's frame (`dt` seconds): a hiding panel whose `HideComplete` has not come within `HIDE_LIMIT_S` leaves
    /// the screen anyway.
    pub fn step(&mut self, dt: f64) {
        for (_, t) in &mut self.hiding {
            *t += dt.max(0.0);
        }
        self.hiding.retain(|(_, t)| *t < HIDE_LIMIT_S);
    }

    /// A button pressed (`down`) or released, by Elite's name (`FocusLeftPanel`, `UI_Select`, `CycleNextPanel`...).
    pub fn input(&mut self, button: &str, down: bool) -> Vec<FocusOut> {
        let out = if button == UI_FOCUS { self.ui_focus(down) } else { self.button(button, down) };
        self.deliver(out)
    }

    /// `UIFocus` (the hold rule in the module's notes).
    fn ui_focus(&mut self, down: bool) -> Vec<FocusOut> {
        self.focus_down = down;
        let takes = down || (self.hold && self.is_hold_state());
        let Some(target) = takes.then(|| self.transition(UI_FOCUS)).flatten() else { return Vec::new() };
        let before = self.state();
        self.enter(&target);
        self.settle();
        self.changes(&before)
    }

    /// Any other button: a transition, the pop-up's `UI_Back`, or a UI button for the focused movie.
    fn button(&mut self, button: &str, down: bool) -> Vec<FocusOut> {
        let now = self.state();
        let focused = now.focused.map(Panel::movie);
        let popup_back = button == "UI_Back" && focused.is_some_and(|m| self.popups.contains(&m));
        if down
            && !popup_back
            && let Some(target) = self.transition(button)
        {
            self.enter(&target);
            self.settle();
            return self.changes(&now);
        }
        let Some(movie) = focused else { return Vec::new() };
        let live = |set: &str| now.control_sets.iter().any(|c| c == set);
        let routed = (MODAL_BUTTONS.contains(&button) && live(MODAL_SET))
            || (PAGINATION_BUTTONS.contains(&button) && live(PAGINATION_SET));
        let Some(name) = movie_button(button).filter(|_| routed) else { return Vec::new() };
        let held = self.pressed.iter().position(|p| *p == (movie, name));
        match (down, held) {
            (true, None) => self.pressed.push((movie, name)),
            (true, Some(_)) => {}
            (false, Some(at)) => {
                self.pressed.remove(at);
            }
            // A release whose press went elsewhere (or nowhere) is not sent.
            (false, None) => return Vec::new(),
        }
        vec![FocusOut::Button { movie, name, down }]
    }

    /// Goes to a state (a redirect goes on at once).
    fn enter(&mut self, target: &str) {
        self.current = if target == REDIRECT { ROLE_SWITCH.to_string() } else { target.to_string() };
    }

    /// The data lists `UIFocus` among the current state's inputs.
    fn is_hold_state(&self) -> bool {
        self.states[&self.current].inputs.iter().any(|a| a.inputs.iter().any(|(i, _)| i == UI_FOCUS))
    }

    /// In hold mode a hold state entered with `UIFocus` up is left at once by its own `UIFocus` input.
    fn settle(&mut self) {
        for _ in 0..REACHED.len() {
            if !self.hold || self.focus_down || !self.is_hold_state() {
                return;
            }
            let Some(target) = self.transition(UI_FOCUS) else { return };
            self.enter(&target);
        }
    }

    /// The state an input leads to from here, if it leads anywhere this mod drives.
    fn transition(&self, button: &str) -> Option<String> {
        let s = &self.states[&self.current];
        let target = s.inputs.iter().find_map(|a| {
            a.inputs.iter().find(|(i, _)| i == button).map(|(_, t)| Some(t.clone())).or_else(|| {
                a.exits.iter().any(|e| e == button).then(|| a.exit_target.clone())
            })
        })??;
        REACHED.contains(&target.as_str()).then_some(target)
    }

    /// The messages from `before` to now: the life cycle and focus messages, the losing panel's held buttons released
    /// first.
    fn changes(&mut self, before: &FocusState) -> Vec<FocusOut> {
        let after = self.state();
        let mut out = Vec::new();
        let kept = after.focused.filter(|_| after.shown && after.comms_level == before.comms_level);
        if let Some(lost) = before.focused.filter(|p| Some(*p) != kept) {
            let movie = lost.movie();
            for (_, name) in self.pressed.iter().filter(|(m, _)| *m == movie) {
                out.push(FocusOut::Button { movie, name, down: false });
            }
            self.pressed.retain(|(m, _)| *m != movie);
        }
        out.extend(changes(before, &after));
        for o in &out {
            let FocusOut::Send(m) = o else { continue };
            let Some(panel) = Panel::ALL.into_iter().find(|p| p.movie() == m.movie) else { continue };
            match m.name {
                "Hide" => {
                    self.hiding.retain(|(p, _)| *p != panel);
                    self.hiding.push((panel, 0.0));
                }
                // A `Show` during the fade stops it (`Content.show` removes the tweens): no `HideComplete` will come.
                "Show" => self.hiding.retain(|(p, _)| *p != panel),
                _ => {}
            }
        }
        out
    }

    /// The first messages since the movies were loaded open them first.
    fn deliver(&mut self, out: Vec<FocusOut>) -> Vec<FocusOut> {
        if self.opened || out.is_empty() {
            return out;
        }
        self.opened = true;
        let open = Panel::ALL.iter().map(|p| FocusOut::Send(Msg { movie: p.movie(), name: "Open", args: vec![] }));
        open.chain(out).collect()
    }
}

fn input_activity(a: &Json) -> InputActivity {
    let names = |v: &Json| v.as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_string).collect::<Vec<_>>();
    InputActivity {
        inputs: a["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string())))
            .collect(),
        exits: names(&a["exit_inputs"]),
        exit_target: a["exit_target"].as_str().map(str::to_string),
    }
}

/// The messages a change of state means (the stand-in rule in the module's notes).
fn changes(before: &FocusState, after: &FocusState) -> Vec<FocusOut> {
    let mut out = Vec::new();
    let send = |out: &mut Vec<FocusOut>, panel: Panel, name: &'static str, args: Vec<Value>| {
        out.push(FocusOut::Send(Msg { movie: panel.movie(), name, args }));
    };
    let focus_on = |out: &mut Vec<FocusOut>, panel: Panel, level: Option<i32>| {
        let args = if panel == Panel::Comms { vec![Value::from(level.unwrap_or(2))] } else { vec![] };
        send(out, panel, "FocusOn", args);
        send(out, panel, "ReceivingButtons", vec![true.into()]);
    };
    let focus_off = |out: &mut Vec<FocusOut>, panel: Panel| {
        send(out, panel, "FocusOff", vec![]);
        send(out, panel, "ReceivingButtons", vec![false.into()]);
    };
    let focus_of = |s: &FocusState| s.focused.filter(|_| s.shown);
    let (was, now) = (focus_of(before), focus_of(after));
    let moved = was != now || before.comms_level != after.comms_level;
    // The order (the movies' code, A): the panel losing the keys is switched off before its `Hide` (`Content.hide` leaves
    // its input on until the fade ends: no key may reach it meanwhile); a panel coming on is shown before its `FocusOn`
    // (`Content.show` restarts the panel: visible, alpha 1, the show effect, its opening sound; the role panel's `show`
    // does not enable its input, so the `FocusOn` that does comes after); a shown panel without the keys is switched off
    // (the left and right panels' `show` enables their input).
    if let Some(p) = was.filter(|_| moved) {
        focus_off(&mut out, p);
    }
    for p in Panel::ALL {
        if wanted(before.shown, before.focused, p) && !wanted(after.shown, after.focused, p) {
            send(&mut out, p, "Hide", vec![]);
        }
    }
    for p in Panel::ALL {
        if !wanted(before.shown, before.focused, p) && wanted(after.shown, after.focused, p) {
            send(&mut out, p, "Show", vec![]);
            if now != Some(p) {
                send(&mut out, p, "FocusOff", vec![]);
            }
        }
    }
    if let Some(p) = now.filter(|_| moved) {
        focus_on(&mut out, p, after.comms_level);
    }
    out
}

/// A panel is wanted on screen (sent `Show`, not `Hide`): the comms panel while the panels are shown (its bar is in the
/// cockpit in flight; its focus only changes its level), another panel while it has the keys.
fn wanted(shown: bool, focused: Option<Panel>, panel: Panel) -> bool {
    shown && (panel == Panel::Comms || focused == Some(panel))
}

#[cfg(test)]
mod tests;
