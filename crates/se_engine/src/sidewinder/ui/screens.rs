//! Elite's Full Spectrum System scanner screen in the game (ARCHITECTURE.md A10, task UI7): `UiScreens` owns the FSS's
//! feeder (`elite_ui::feed::Fss`) and the screen's open state.
//!
//! - Keys: the FSS's enter and quit keys from the shipped binds (`PilotKeys::fss_enter`, `fss_quit`) and the tests'
//!   `sw ui fss open|close`, by Elite's rule (`elite_ui::screen_facts::may_open`: supercruise, the throttle at zero;
//!   `next_open`). A refused enter shows Elite's own refusal text through the mod's messages (`REFUSE_*`; where Elite
//!   itself shows it was not decoded).
//! - While open: the movie is wanted (`UiWanted`) and drawn as a screen overlay by `place::draw_screens`, as `notifications`
//!   is (the layout gives `fsshud` the screen kind). Closed, the movie stays wanted until it says `UIClosed`.
//! - While it covers the view (`UiScreens::covers`: open, and until the movie's `UIClosed` after a close, so the cockpit
//!   comes back in the frame the screen goes): nothing of the ship is drawn (`draw::place`, and with it Elite's HUD pieces,
//!   plates, side panels, scanner and holograms, which show only with the seat's set) nor the stand-in HUD and the mouse
//!   widget (`hud::draw_hud`); the head is straight ahead (head look's turn and switch kept for after), every ship action's
//!   key is quiet and the mouse and the pad's sticks do not steer (`Seat::screen`, read by `seat_camera`, `take_keys` into
//!   `pilot::ship_input`'s screen case, and `pilot::fly`). The ship's sounds go on. Out of the seat the screen is closed at once.
//! - The legend's keys: the shipped binds go to the host's text table once (`bind_keys`, from `load.rs`), so a
//!   `$BINDING_input_*` token is written as the bound key's name (`elite_ui::Text`'s `binding` type, a stand-in, E).
//! - The facts (`feed`, read when the screen opens and when the system or its bodies change while open): the system's name (`ActiveSystem::name`) and its bodies (the `SystemBody` entities: `fly`'s
//!   Navigation rows are made only while the left panel has the keys), each planet or moon by Elite's planet class
//!   (`hologram::elite_class` and `Life::of`, the mapping the target hologram uses). The movie's sounds go through the
//!   movies' path (`movies.rs`, the `FssHUD` table). The exe's own `SystemScan_*` posts are not made: the notes name the
//!   functions that post them, never their triggers.
//! - Not built (Elite's game draws these itself, or they are scanning gameplay, the user's scope): the FSS camera and its
//!   view of the sky, the signal blobs and the signal lines round the aim, zooming onto a body and its view, the system's
//!   orbital-plane outline, the discovery scan (the honk) and the discovery state, the help pop-up (`ExplorationFSSShowHelp`),
//!   the tuning keys (`ExplorationFSSRadioTuningX_*`: the rate the exe moves the tuning by is not decoded, so the tuning
//!   is only a test's, `sw ui fss tune`).
//! - Only while Elite's UI is made (`UiDraw`; with `SE_SW_UI=0` the module's plugin is not added).

use bevy::prelude::*;
use elite_gfx::Out;
use elite_ui::feed::{Fss, Movie, Outbox};
use elite_ui::screen_facts::{may_open, next_open, BodyKind, FssBody, FssFacts};
use elite_ui::text::{Bound, Chord};
use se_core::catalog::{BodyTag, ObjectRef};

use super::super::pilot::{Pilot, PilotKeys};
use super::super::seat::Seat;
use super::hologram::{elite_class, Life};
use super::place::UiDraw;
use super::{seated, UiMessages, UiMovieState, UiReplies, UiSet, UiWanted};
use crate::mods::ModControl;
use crate::data::SeData;
use crate::system::{ActiveSystem, SystemBody};

const MOVIE: Movie = Movie::FssHud;

/// Elite's two refusals the mod has a cause for, by their keys in the shipped string table and their English text there
/// (`ui/strings/en.json`: a test checks them), shown through the mod's messages (the pilot's messages, the comms log).
const REFUSE_SUPERCRUISE: (&str, &str) = ("info_fss_enter_supercruise", "Enter supercruise to use FSS mode");
const REFUSE_THROTTLE: (&str, &str) = ("info_fss_throttle_warning", "Throttle down to use FSS mode");
/// Elite's message when the FSS closes itself for speed (`info_fss_warning_speed`; B: its name and text say it, the exe's
/// test was not traced), the mod's automatic close in supercruise being the throttle off zero.
const CLOSE_SPEED: (&str, &str) = ("info_fss_warning_speed", "Too fast, closing FSS");

/// The seat's keyboard and mouse binds in the mod's folder.
pub(super) use super::super::assets::KEYS_FILE;

/// The system's discovery scan counts as done (stand-in, E): Elite's screen shows the bodies' signals only after the
/// discovery scan (the honk); the mod has no discovery scan (the user's scope), so the screen opens ACTIVE, the bodies on
/// the spectrum as undiscovered signals.
const DISCOVERY_SCAN_DONE: bool = true;

/// What a test asked (`sw ui fss`), taken on the next seated frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ask {
    Open,
    Close,
    Tune(f64),
}

/// The FSS screen: its feeder, whether it is open or closing, the tuning, the tests' asks, the facts last fed.
#[derive(Resource, Default)]
pub struct UiScreens {
    fss: Fss,
    open: bool,
    /// Closed, and the movie has not said `UIClosed` yet (it stays wanted meanwhile, so it takes its `Close`).
    closing: bool,
    tuning: Option<f64>,
    asks: Vec<Ask>,
    /// The facts last fed, and whether their bodies were read since the screen last opened.
    facts: FssFacts,
    built: bool,
}

impl UiScreens {
    /// The screen covers the view: open, or closed and its movie not yet hidden (`UIClosed` not heard): the cockpit, its HUD
    /// and the mouse's steering wait for this, so the hand-over happens in one frame either way.
    pub(crate) fn covers(&self) -> bool {
        self.open || self.closing
    }
}

/// The shipped binds' keys into the host's text table, for every movie's `$BINDING_input_*` tokens (`elite_ui::Text::bind`).
pub(super) fn bind_keys(text: &mut elite_ui::Text, binds: &[u8]) -> Result<(), String> {
    let binds = std::str::from_utf8(binds).map_err(|e| format!("{KEYS_FILE}: {e}"))?;
    let binds = se_core::elite_asset::Binds::parse(binds).map_err(|e| format!("{KEYS_FILE}: {e:?}"))?;
    let chord = |k: &se_core::elite_asset::KeyBind| Chord { key: k.key.clone(), modifiers: k.modifiers.iter().map(|(_, key)| key.clone()).collect() };
    for (action, b) in &binds.actions {
        text.bind(action, Bound { primary: b.primary.as_ref().map(chord), secondary: b.secondary.as_ref().map(chord), axis: b.axis.as_ref().map(chord) });
    }
    Ok(())
}

/// A system body as the FSS reads it: a star, or a planet, moon or dwarf by Elite's planet class; None for the rest
/// (asteroids, comets, spacecraft, a body not in the catalog).
fn fss_body(data: &SeData, obj: ObjectRef, star: bool) -> Option<FssBody> {
    match obj {
        ObjectRef::Star(i) if star => Some(FssBody { id: u64::from(i), kind: BodyKind::Star }),
        ObjectRef::Body(i) => {
            let b = data.catalog.bodies.get(i as usize)?;
            matches!(b.tag, BodyTag::Planet | BodyTag::DwarfPlanet | BodyTag::Moon | BodyTag::DwarfMoon)
                .then(|| FssBody { id: u64::from(i), kind: BodyKind::Planet(elite_class(b.class.as_deref(), Life::of(&b.block, b.civilization().is_some()))) })
        }
        _ => None,
    }
}

/// Each seated frame, before the flight (`pilot::fly`) and the seat's camera: the keys and the tests' asks by Elite's rule
/// (a refusal or an automatic close says Elite's own message), the movie wanted, and what covers the view (`Seat::screen`),
/// so the flight, the seat's camera, the focus machine, the cockpit's draw and the HUD all see this frame's state. The
/// seat's key reading (`seat::take_keys`, PreUpdate) is the one reader a frame behind: the keys it reads decide this.
fn decide(
    keys: Res<PilotKeys>,
    pilot: Res<Pilot>,
    (movies, replies): (Res<UiMovieState>, Res<UiReplies>),
    mut wanted: ResMut<UiWanted>,
    (mut seat, mut msg): (ResMut<Seat>, ResMut<crate::messages::Messages>),
    mut ui: ResMut<UiScreens>,
) {
    let ui = &mut *ui;
    let mut enter = keys.fss_enter;
    let mut quit = keys.fss_quit;
    for ask in ui.asks.drain(..) {
        match ask {
            Ask::Open => enter = true,
            Ask::Close => quit = true,
            Ask::Tune(f) => ui.tuning = Some(f.clamp(0.0, 1.0)),
        }
    }
    let supercruise = pilot.cruise.is_some();
    let allowed = may_open(supercruise, pilot.throttle);
    if enter && !ui.open && !allowed {
        let (key, text) = if supercruise { REFUSE_THROTTLE } else { REFUSE_SUPERCRUISE };
        info!("sidewinder ui: the FSS refused (${key};: supercruise {supercruise}, throttle {:.2})", pilot.throttle);
        msg.show(text);
    }
    let open = next_open(ui.open, enter, quit, allowed);
    if open != ui.open {
        info!("sidewinder ui: the FSS {}", if open { "opened" } else { "closed" });
        if !open && !quit && supercruise {
            // (Closed by the rule while still in supercruise: the throttle off zero, Elite's "too fast". Out of supercruise
            // Elite's strings name no cause the mod has: no message.)
            let (key, text) = CLOSE_SPEED;
            info!("sidewinder ui: the FSS closed by the rule (${key};: throttle {:.2})", pilot.throttle);
            msg.show(text);
        }
        ui.closing = !open;
        ui.open = open;
        ui.built = false;
    }
    if ui.closing && (!movies.ready.contains(&MOVIE) || replies.outs.iter().any(|(m, o)| *m == MOVIE && matches!(o, Out::Closed))) {
        ui.closing = false;
    }
    wanted.movies.retain(|m| *m != MOVIE);
    if ui.covers() {
        wanted.movies.push(MOVIE);
    }
    seat.screen = ui.covers();
}

/// Each seated frame in the UI's feed: the facts (the system's bodies read only when the screen opens and when the system
/// or its bodies change while it is open: nothing while it is closed), the feeder's messages (to the movie once it is
/// ready; everything again when it has just become so).
fn feed(
    (data, system): (Res<SeData>, Res<ActiveSystem>),
    bodies: Query<&SystemBody>,
    added: Query<(), Added<SystemBody>>,
    mut removed: RemovedComponents<SystemBody>,
    movies: Res<UiMovieState>,
    mut messages: ResMut<UiMessages>,
    mut ui: ResMut<UiScreens>,
) {
    let ui = &mut *ui;
    let bodies_changed = !added.is_empty() | (removed.read().count() > 0);
    if ui.open && (!ui.built || bodies_changed || ui.facts.system != system.name) {
        ui.facts.system = system.name.clone(); // `ActiveSystem::name`
        ui.facts.bodies = bodies.iter().filter_map(|b| fss_body(&data, b.obj, b.star)).collect(); // by their catalog class
        ui.built = true;
    }
    ui.facts.open = ui.open;
    ui.facts.discovery_scan_done = DISCOVERY_SCAN_DONE; // stand-in: the mod has no discovery scan
    ui.facts.tuning = ui.tuning; // a test's only (`sw ui fss tune`)
    if movies.fresh.contains(&MOVIE) {
        ui.fss.reset();
    }
    let mut out = Outbox::default();
    ui.fss.feed(&ui.facts, &mut out);
    if movies.ready.contains(&MOVIE) {
        messages.msgs.extend(out.msgs);
    }
}

/// Out of the seat (the ship lost or left): the screen closed at once, its `Close` sent (the movies step on a moment after
/// the seat is left), nothing covering any more.
fn leave(seat: Res<Seat>, control: Res<ModControl>, movies: Res<UiMovieState>, mut wanted: ResMut<UiWanted>, mut messages: ResMut<UiMessages>, mut ui: ResMut<UiScreens>) {
    if seated(&seat, &control) || !ui.covers() {
        return;
    }
    info!("sidewinder ui: the FSS closed: out of the seat");
    ui.open = false;
    ui.closing = false;
    ui.facts.open = false;
    let mut out = Outbox::default();
    let ui = &mut *ui;
    ui.fss.feed(&ui.facts, &mut out);
    if movies.ready.contains(&MOVIE) {
        messages.msgs.extend(out.msgs);
    }
    wanted.movies.retain(|m| *m != MOVIE);
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<UiScreens>()
        .add_systems(PostUpdate, decide.before(super::super::pilot::fly).run_if(resource_exists::<UiDraw>).run_if(super::seated_now))
        .add_systems(PostUpdate, feed.in_set(UiSet::Feed).run_if(resource_exists::<UiDraw>))
        .add_systems(PostUpdate, leave.before(UiSet::Feed).after(super::super::facts::update).run_if(resource_exists::<UiDraw>));
}

/// `sw ui fss open|close|state|tune F`: the enter and quit keys as the seat gives them (by Elite's rule, the next seated
/// frame), the screen's state, or the tuning set to F (0..1; the tuning keys' rate is not decoded).
pub fn dev_command(world: &mut World, args: &str) {
    let mut words = args.split_whitespace();
    let verb = words.next().unwrap_or("state").to_ascii_lowercase();
    let ask = match verb.as_str() {
        "open" => Ask::Open,
        "close" => Ask::Close,
        "tune" => match words.next().and_then(|w| w.parse::<f64>().ok()).filter(|f| f.is_finite()) {
            Some(f) => Ask::Tune(f),
            None => {
                warn!("sw ui fss tune: give the tuning, 0 to 1; got `{args}`");
                return;
            }
        },
        "state" => {
            state(world);
            return;
        }
        _ => {
            warn!("sw ui fss: open | close | state | tune F; got `{args}`");
            return;
        }
    };
    info!("sidewinder ui: fss {ask:?} (a test)");
    world.resource_mut::<UiScreens>().asks.push(ask);
}

/// `sw ui fss state`: open or closing, Elite's rule now, the facts last fed, the movie's state.
fn state(world: &mut World) {
    let pilot = world.resource::<Pilot>();
    let allowed = may_open(pilot.cruise.is_some(), pilot.throttle);
    let (supercruise, throttle) = (pilot.cruise.is_some(), pilot.throttle);
    // (The ship's nose in the universe's axes in supercruise, and the mouse's stick: whether the mouse steers.)
    let (stick, nose) = (pilot.stick, pilot.cruise.map(|c| c.rot * bevy::math::DVec3::NEG_Z));
    let covers = world.resource::<UiScreens>().covers();
    let head = world.resource::<Seat>().head.turn;
    let movies = world.resource::<UiMovieState>();
    let ready = movies.ready.contains(&MOVIE);
    let wanted = world.resource::<UiWanted>().movies.contains(&MOVIE);
    let ui = world.resource::<UiScreens>();
    let f = &ui.facts;
    let planets = f.bodies.iter().filter(|b| matches!(b.kind, BodyKind::Planet(_))).count();
    info!(
        "sidewinder ui: fss open {}, closing {}, covers {covers}, may open {allowed} (supercruise {supercruise}, throttle {throttle:.2}); stick {stick:?}, nose {nose:?}, head {head:?}; system {:?}, {} bodies ({planets} planets and moons, classes {:?}), discovery scan {}, tuning {:?}; movie wanted {wanted}, ready {ready}",
        ui.open,
        ui.closing,
        f.system,
        f.bodies.len(),
        f.bodies.iter().filter_map(|b| if let BodyKind::Planet(c) = b.kind { Some(c) } else { None }).collect::<Vec<_>>(),
        f.discovery_scan_done,
        f.tuning
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// While the screen covers every kind of ship action is dead (the seat's keys read with `UiSets::screen`), and alive again
    /// without it; the FSS's own quit key acts either way.
    #[test]
    fn the_screen_quiets_every_ship_action() {
        use super::super::super::panels::UiSets;
        use super::super::super::pilot::read_keys;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/binds");
        let read = |f: &str| se_core::elite_asset::Binds::parse(&std::fs::read_to_string(dir.join(f)).unwrap()).unwrap();
        let (binds, pad) = (read("KeyboardMouseOnly.binds"), read("ControlPad.binds"));
        let motion = bevy::input::mouse::AccumulatedMouseMotion::default();
        let actions: [(&str, fn(&PilotKeys) -> bool); 13] = [
            ("ForwardKey", |k| k.throttle_up),
            ("SetSpeedZero", |k| k.throttle_zero),
            ("UpThrustButton", |k| k.vertical != 0.0),
            ("UseBoostJuice", |k| k.boost),
            ("HyperSuperCombination", |k| k.jump),
            ("LandingGearToggle", |k| k.gear),
            ("ShipSpotLightToggle", |k| k.lights),
            ("IncreaseEnginesPower", |k| k.pip_eng),
            ("DeployHardpointToggle", |k| k.hardpoints),
            ("SelectTarget", |k| k.select_target),
            ("RadarIncreaseRange", |k| k.radar_up),
            ("ToggleFlightAssist", |k| k.flight_assist),
            ("ExplorationFSSQuit", |k| k.fss_quit),
        ];
        for (action, acted) in actions {
            let key = binds.key(action, "Keyboard").and_then(super::super::super::seat::key_code).unwrap_or_else(|| panic!("{action}: no key"));
            let mut keys = ButtonInput::<KeyCode>::default();
            keys.press(key);
            let buttons = ButtonInput::<MouseButton>::default();
            let at = |screen: bool| acted(&read_keys(&binds, &pad, None, &keys, &buttons, &motion, true, UiSets { screen, ..UiSets::default() }));
            assert!(at(false), "{action} acts without the screen");
            assert_eq!(at(true), action == "ExplorationFSSQuit", "{action} with the screen up");
        }
    }

    /// The refusals' and the automatic close's texts are the shipped string table's own.
    #[test]
    fn the_refusals_are_elites_own_text() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui/strings/en.json");
        let table: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for (key, text) in [REFUSE_SUPERCRUISE, REFUSE_THROTTLE, CLOSE_SPEED] {
            assert_eq!(table[key].as_str(), Some(text), "{key}");
        }
    }
}
