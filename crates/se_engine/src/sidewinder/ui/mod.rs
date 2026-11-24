//! Elite's ship UI in the game (ARCHITECTURE.md A10, task UI3): Elite's own UI movies run by the vendored Ruffle
//! (`elite_gfx`), drawn into textures and shown on Elite's own panel models in the cockpit.
//!
//! - On unless `SE_SW_UI=0` (`enabled`, read once at start); with it 0 nothing of this module but `UiLive` is added to the
//!   app and the stand-in HUD and side panels (`hud.rs`, `panels.rs`) draw. They are also the fallback: every guard between
//!   the two reads `UiLive` (the UI made and running), so if the UI cannot be made or turns itself off (`place::turn_off`,
//!   `sw ui off`) the stand-ins take over at once.
//! - The interface the other tasks use: `UiMessages` (the feeders push, `UiSet::Movies` sends), `UiReplies` (what the movies
//!   sent back this frame), `UiMovieState` (loaded and ready), `UiWanted` (which movies run).
//! - Files, one job each: `load.rs` (the UI's files, on a worker thread), `host.rs` (the movies' questions answered),
//!   `movies.rs` (the movies running: load, send, step, draw, drain), `material.rs` (the materials' values and flags),
//!   `place.rs` (the pieces made, placed and shown; the screen movies' overlay), `pass.rs` (the UI's own pass that draws
//!   the pieces after the engine's colour passes, at the view's full size), `plates.rs` (the dark plates' states).
//!   `facts.rs` and `feed.rs` are task UI4b's (the HUD's data from the game).
//! - When: the UI's files are read once the first spawn's are and the ship's materials exist (`ShipScene`), and its pieces
//!   made then, so the pass's pipelines are built before the first boarding. The movies
//!   load only while seated, one a frame, and step and draw only while seated, and step for a grace after (`UiGrace`); they
//!   stay loaded when the pilot gets up (`ModControl::pilot` is true exactly while seated, so "the mod off" is getting up),
//!   and go with the ship.
//! - Not drawn, so the stand-in HUD's own readouts for them stay with `SE_SW_UI=1` (`hud.rs`): the target sights' cells and
//!   the orbital HUD (they run and take their messages and sounds, but where and how big Elite draws them is neither decoded
//!   nor shown in any picture of session ui1), and the multicrew gauges (`place.rs` `MULTICREW_PANEL`: Elite shows no
//!   second set of the gauges beside the scanner on the Sidewinder's single seat).
//! - Order (PostUpdate, after the flight and `facts::update`): `UiSet::Feed`, `UiSet::Movies` before the cockpit is placed
//!   (`draw::place`), `UiSet::Place` right after it (it hangs the pieces on the frame that placement made, for the pass).

mod facts;
mod feed;
mod focus;
pub(super) mod hologram;
mod host;
mod load;
mod material;
mod movies;
mod panels;
pub(super) mod pass;
mod place;
mod plates;
pub(super) mod scanner;
pub(super) mod screens;

use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};
use elite_ui::feed::Movie;

use crate::mods::ModControl;

use super::material::ShipScene;
use super::seat::Seat;
use super::{Sidewinder, SidewinderAssets};

/// What the movies are sent, in order (the feed systems push, as `elite_ui::feed::Outbox` gives it; `movies` drains every
/// frame after them, before the step): the DataStore writes first (`elite_gfx::Movie::store`), then the messages, each list
/// in its own order.
#[derive(Resource, Default)]
pub struct UiMessages {
    pub msgs: Vec<elite_ui::feed::Msg>,
    pub stores: Vec<elite_ui::feed::Store>,
}

/// What the movies sent back this frame (filled by `movies`; cleared at the start of the next frame's step).
#[derive(Resource, Default)]
pub struct UiReplies {
    pub outs: Vec<(elite_ui::feed::Movie, elite_gfx::Out)>,
}

/// Which movies are loaded and ready, and which became ready THIS frame (the feeders reset on those).
#[derive(Resource, Default)]
pub struct UiMovieState {
    pub ready: Vec<elite_ui::feed::Movie>,
    pub fresh: Vec<elite_ui::feed::Movie>,
}

/// Which movies should be running (set by whoever knows: the HUD's set while seated, a side panel while open).
#[derive(Resource, Default)]
pub struct UiWanted {
    pub movies: Vec<elite_ui::feed::Movie>,
}

/// The UI's systems in their order: the feeders, then the movies (send, step, draw, drain), then the quads' placing.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiSet {
    Feed,
    Movies,
    Place,
}

/// Elite's UI instead of the stand-in HUD and side panels: on unless `SE_SW_UI=0`. The one place the switch is read (once,
/// at start). It only says whether the UI is added to the app; whether it is made and running now is `UiLive`.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| switch_on(std::env::var("SE_SW_UI").ok().as_deref()))
}

/// The switch's value: unset, or anything but 0, is on.
fn switch_on(value: Option<&str>) -> bool {
    value.is_none_or(|v| v.trim() != "0")
}

/// Elite's UI is made and running this frame (its pieces made, not turned off): what every guard between it and the stand-in
/// HUD and side panels reads (`hud.rs`, `pilot.rs`; the focus machine, the FSS and the panels' head look follow it through
/// `place::UiDraw` and `Panels`). False with the switch off, before a spawn's UI is made, once the ship has gone and after
/// the UI turned itself off.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiLive(pub bool);

/// Every movie of the scope (A10).
const ALL: [Movie; 12] = [
    Movie::CockpitGaugePanel,
    Movie::CockpitHudRing,
    Movie::TargetSights,
    Movie::CockpitInfoPanel,
    Movie::OrbitalHud,
    Movie::Notifications,
    Movie::CockpitExternalPanel,
    Movie::CockpitInternalPanel2,
    Movie::RolePanel3,
    Movie::CockpitChatPanel,
    Movie::FssHud,
    Movie::SaaHud,
];

/// The flight HUD's movies: wanted from the first boarding (the feeders' set, `elite_ui::feed::Hud`).
pub const HUD: [Movie; 6] = [Movie::CockpitGaugePanel, Movie::CockpitHudRing, Movie::TargetSights, Movie::CockpitInfoPanel, Movie::OrbitalHud, Movie::Notifications];

/// The movie of that file name (any case).
pub fn movie_named(name: &str) -> Option<Movie> {
    ALL.into_iter().find(|m| m.name().eq_ignore_ascii_case(name))
}

/// A movie's interface: the name of its `Items/UIMovie/<Interface>` record, whose table its `PlaySound` names are looked up
/// in (`ui/sound_tables.json`'s keys; `re/elite/notes/ui_sounds.md` section 1, step 8: the record holds the interface's
/// name and its movie's). For the panel movies it is the cockpit component's movie name (`layout.json`
/// `component.movie`); a test checks every one against the shipped tables.
pub fn interface(movie: Movie) -> &'static str {
    match movie {
        Movie::CockpitGaugePanel => "CockpitGaugePanel",
        Movie::CockpitHudRing => "CockpitHUDRing",
        Movie::TargetSights => "TargetSights",
        Movie::CockpitInfoPanel => "CockpitInfoPanel",
        Movie::OrbitalHud => "OrbitalHud",
        Movie::Notifications => "Notifications",
        Movie::CockpitExternalPanel => "CockpitExternalPanel",
        Movie::CockpitInternalPanel2 => "CockpitInternalPanel2",
        Movie::RolePanel3 => "RolePanel3",
        Movie::CockpitChatPanel => "CockpitChatPanel",
        Movie::FssHud => "FssHUD",
        Movie::SaaHud => "SaaHUD",
    }
}

/// A movie's trace that is an error: an ActionScript error, a `Break`, a message with no listener (the bridge's manager
/// traces "no listener for message NAME"), a tween's failed completion (as the runtime's proofs read them).
pub fn is_error(text: &str) -> bool {
    text.starts_with("Break:")
        || text.contains("Error #")
        || text.contains("no listener for message")
        || ["TypeError", "ReferenceError", "ArgumentError", "RangeError", "VerifyError", "SecurityError"].iter().any(|e| text.contains(e))
        || text.starts_with("tween onComplete:")
}

/// The pilot sits in the seat and the mod is on.
fn seated(seat: &Seat, control: &ModControl) -> bool {
    seat.seated && control.pilot
}

fn seated_now(seat: Res<Seat>, control: Res<ModControl>) -> bool {
    seated(&seat, &control)
}

/// The movies' grace after the pilot leaves the seat: for `elite_ui::focus::HIDE_LIMIT_S` of real time (Elite's own limit
/// on a panel's hide) they go on stepping and draining, drawing nothing and sent nothing, so what their own timers hold
/// (the role panel's second `OnDisembarkPressed` 0.15 s after the first, a hide's fade) runs out while he is out, not at his
/// next boarding. A ship lost or removed ends it at once. Kept by `track_grace` each frame before the movies.
#[derive(Resource, Default)]
pub struct UiGrace {
    /// Seated this frame.
    pub seated: bool,
    /// The real time the pilot left the seat, while the grace runs.
    out_at: Option<f64>,
}

impl UiGrace {
    /// The movies step this frame: seated, or in the grace.
    fn stepping(&self) -> bool {
        self.seated || self.out_at.is_some()
    }
}

fn track_grace(seat: Res<Seat>, control: Res<ModControl>, ship: Res<Sidewinder>, time: Res<Time<Real>>, mut grace: ResMut<UiGrace>) {
    let seated = seated(&seat, &control);
    let now = time.elapsed_secs_f64();
    let out_at = match grace.out_at {
        _ if seated || ship.parked.is_none() => None,
        None if grace.seated => Some(now),
        Some(t) if now - t >= elite_ui::focus::HIDE_LIMIT_S => None,
        held => held,
    };
    if grace.seated != seated || grace.out_at != out_at {
        *grace = UiGrace { seated, out_at };
    }
}

fn stepping(grace: Res<UiGrace>) -> bool {
    grace.stepping()
}

/// A new ship replaced the parked one (`sidewinder::spawn`): the old ship's movies go and the new ship's are read and made
/// as at a first spawn (`load_due`), so they start fresh; the feeders resend everything when `UiDraw` comes back.
pub(super) fn new_ship(commands: &mut Commands) {
    if enabled() {
        commands.queue(place::renew);
    }
}

/// Not seated, and something the movies said or became on the last seated frame is still held.
fn stale(seat: Res<Seat>, control: Res<ModControl>, replies: Res<UiReplies>, state: Res<UiMovieState>) -> bool {
    !seated(&seat, &control) && (!replies.outs.is_empty() || !state.fresh.is_empty())
}

/// Nothing of the last seated frame's replies or newly ready movies waits for the next boarding. In the grace (`UiGrace`)
/// this runs after each step: the events that come out then reach no listener that runs only seated (the panels' `hear`,
/// the screens, the plates in `place`); their sounds have played in the step.
fn clear_stale(mut replies: ResMut<UiReplies>, mut state: ResMut<UiMovieState>) {
    replies.outs.clear();
    state.fresh.clear();
}

/// The UI's files are to be read: a ship's files read and its materials made, the ship there, nothing read or tried yet.
fn load_due(assets: Option<Res<SidewinderAssets>>, scene: Option<Res<ShipScene>>, ship: Res<Sidewinder>, draw: Option<Res<place::UiDraw>>, loading: Option<Res<place::UiLoading>>, off: Option<Res<place::UiOff>>) -> bool {
    assets.is_some() && scene.is_some() && ship.parked.is_some() && draw.is_none() && loading.is_none() && off.is_none()
}

/// The ship has gone and something of its UI is left.
fn ship_gone(ship: Res<Sidewinder>, draw: Option<Res<place::UiDraw>>, off: Option<Res<place::UiOff>>) -> bool {
    ship.parked.is_none() && (draw.is_some() || off.is_some())
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<UiLive>();
    if !enabled() {
        return;
    }
    let handoff = movies::Handoff::default();
    let built = pass::Built::default();
    app.init_resource::<UiMessages>()
        .init_resource::<UiReplies>()
        .init_resource::<UiGrace>()
        .init_resource::<UiMovieState>()
        .init_resource::<UiWanted>()
        .insert_resource(handoff.clone())
        .insert_resource(built.clone())
        .configure_sets(PostUpdate, (UiSet::Feed, UiSet::Movies).chain().after(super::facts::update).before(super::draw::place))
        .configure_sets(PostUpdate, UiSet::Feed.run_if(seated_now))
        .configure_sets(PostUpdate, UiSet::Place.after(super::draw::place).before(crate::universe::UniverseSet::FloatingOrigin).before(bevy::camera::CameraUpdateSystems))
        .add_systems(Update, (place::start_load.run_if(load_due), place::build.run_if(resource_exists::<place::UiLoading>), place::teardown.run_if(ship_gone), place::turn_off.run_if(place::failed)))
        .add_systems(
            PostUpdate,
            (track_grace, movies::step.run_if(resource_exists::<place::UiDraw>).run_if(stepping), clear_stale.run_if(stale)).chain().in_set(UiSet::Movies),
        )
        .add_systems(PostUpdate, place::place.in_set(UiSet::Place).run_if(resource_exists::<place::UiDraw>))
        .add_systems(bevy_egui::EguiPrimaryContextPass, place::draw_screens.run_if(resource_exists::<place::UiDraw>).before(crate::skin::UiSet::Draw));
    facts::plugin(app);
    feed::plugin(app);
    scanner::plugin(app);
    hologram::plugin(app);
    focus::plugin(app);
    panels::plugin(app);
    screens::plugin(app);
    pass::plugin(app, built);
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app.insert_resource(handoff).add_systems(Render, movies::hand_back.run_if(movies::handoff_busy).in_set(RenderSystems::Cleanup));
}

/// `sw ui state | facts | off | send MOVIE NAME JSON_ARGS | store MOVIE CONTEXT/PATH PROPERTY JSON_VALUE | want MOVIE on|off |
/// plate NAME on|off`: test commands (`off` turns the UI off as a failure would: the stand-ins take over for this ship) (in the JSON a semicolon is written as its escape, a backslash then `u003b`: a run's commands are split at
/// semicolons).
pub fn dev_command(world: &mut World, args: &str) {
    if !enabled() {
        warn!("sw ui: Elite's UI is switched off (SE_SW_UI=0)");
        return;
    }
    let mut words = args.split_whitespace();
    let verb = words.next().unwrap_or("state").to_ascii_lowercase();
    match verb.as_str() {
        "state" => state(world),
        "facts" => facts::log(world),
        "off" => place::off(world, "turned off by a test"),
        "key" => focus::dev_key(world, args.trim_start().split_once(char::is_whitespace).map_or("", |(_, rest)| rest)),
        "focus" => focus::dev_focus(world),
        "fss" => screens::dev_command(world, args.trim_start().split_once(char::is_whitespace).map_or("", |(_, rest)| rest)),
        "send" => {
            let (Some(movie), Some(name)) = (words.next().and_then(movie_named), words.next()) else {
                warn!("sw ui send: use `sw ui send MOVIE NAME JSON_ARGS`; got `{args}`");
                return;
            };
            let json = words.collect::<Vec<_>>().join(" ");
            let json = if json.is_empty() { "[]".to_string() } else { json };
            match serde_json::from_str::<elite_ui::Value>(&json) {
                Ok(elite_ui::Value::Array(values)) => {
                    // (A message's name is `'static` in the feeders' interface: a test's few names are leaked once each.)
                    let name: &'static str = Box::leak(name.to_string().into_boxed_str());
                    info!("sidewinder ui: send {} {name} {values:?} (a test)", movie.name());
                    world.resource_mut::<UiMessages>().msgs.push(elite_ui::feed::Msg { movie, name, args: values });
                }
                _ => warn!("sw ui send: the arguments must be a JSON array; got `{json}`"),
            }
        }
        "store" => {
            let (Some(movie), Some(path), Some(property)) = (words.next().and_then(movie_named), words.next(), words.next()) else {
                warn!("sw ui store: use `sw ui store MOVIE CONTEXT/PATH PROPERTY JSON_VALUE`; got `{args}`");
                return;
            };
            let json = words.collect::<Vec<_>>().join(" ");
            match serde_json::from_str::<elite_ui::Value>(&json) {
                Ok(value) => {
                    let context: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
                    info!("sidewinder ui: store {} {context:?} {property} {value:?} (a test)", movie.name());
                    world.resource_mut::<UiMessages>().stores.push(elite_ui::feed::Store { movie, context, property: property.to_string(), value });
                }
                Err(e) => warn!("sw ui store: the value is not JSON ({e}); got `{json}`"),
            }
        }
        "want" => {
            let (Some(movie), Some(on)) = (words.next().and_then(movie_named), words.next()) else {
                warn!("sw ui want: use `sw ui want MOVIE on|off`; got `{args}`");
                return;
            };
            let mut wanted = world.resource_mut::<UiWanted>();
            wanted.movies.retain(|m| *m != movie);
            if on.eq_ignore_ascii_case("on") {
                wanted.movies.push(movie);
            }
            info!("sidewinder ui: wanted {:?}", wanted.movies.iter().map(|m| m.name()).collect::<Vec<_>>());
        }
        "plate" => {
            let (Some(name), Some(on)) = (words.next(), words.next()) else {
                warn!("sw ui plate: use `sw ui plate NAME on|off`; got `{args}`");
                return;
            };
            match world.get_resource_mut::<place::UiDraw>().map(|mut d| d.plates.set(name, on.eq_ignore_ascii_case("on"))) {
                Some(true) => info!("sidewinder ui: plate {name} {on} (a test)"),
                Some(false) => warn!("sw ui plate: no plate named {name}"),
                None => warn!("sw ui plate: the UI isn't made"),
            }
        }
        _ => warn!("sw ui: state | facts | send MOVIE NAME JSON_ARGS | store MOVIE CONTEXT/PATH PROPERTY JSON_VALUE | want MOVIE on|off | plate NAME on|off"),
    }
}

/// `sw ui state`: each movie wanted, loaded, ready, its cost, the messages sent, its traces; the plates shown.
fn state(world: &mut World) {
    let wanted = world.resource::<UiWanted>().movies.clone();
    let made = world.contains_resource::<place::UiDraw>();
    info!("sidewinder ui: made {made}, loading {}, off {}", world.contains_resource::<place::UiLoading>(), world.contains_resource::<place::UiOff>());
    let mut total = 0.0;
    if let Some(mut movies) = world.get_non_send_mut::<movies::UiMovies>() {
        let t = std::mem::take(&mut movies.step_times);
        info!(
            "sidewinder ui: the whole step over the last {} frames: median {:.1} ms, 99 % within {:.1} ms, most {:.2} ms",
            t.frames(),
            t.quantile(0.5),
            t.quantile(0.99),
            t.most_ms
        );
    }
    if let Some(movies) = world.get_non_send::<movies::UiMovies>() {
        for s in &movies.slots {
            let c = &s.cost;
            total += c.last_ms;
            info!(
                "sidewinder ui: {} wanted {} drawn {} loaded {} ready {}{}; last {:.3} ms, mean {:.3} ms over {} frames, {} draws, load {:.1} ms; {} messages sent ({} refused), {} traces, last {:?}",
                s.which.name(),
                wanted.contains(&s.which),
                s.drawn,
                s.loaded(),
                s.ready,
                s.failed.as_ref().map_or(String::new(), |e| format!(" (failed: {e})")),
                c.last_ms,
                c.total_ms / c.frames.max(1) as f64,
                c.frames,
                c.draws,
                c.load_ms,
                s.sent,
                s.refused,
                s.traces,
                s.last_traces
            );
        }
    }
    info!("sidewinder ui: the movies' last frame together {total:.3} ms");
    if let Some(draw) = world.get_resource::<place::UiDraw>() {
        let shown: Vec<&str> = draw.plates.plates.iter().filter(|p| p.shown()).map(|p| p.name.as_str()).collect();
        info!("sidewinder ui: plates shown {shown:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Elite's UI is on unless `SE_SW_UI=0`: unset is on, 0 off, 1 on.
    #[test]
    fn the_switch_is_on_unless_0() {
        assert!(switch_on(None));
        assert!(!switch_on(Some("0")));
        assert!(!switch_on(Some(" 0 ")));
        assert!(switch_on(Some("1")));
    }

    /// Every movie's interface is a table of the shipped `ui/sound_tables.json`, and every movie of the shipped layout is one
    /// of the scope's, under its own file name.
    #[test]
    fn every_movie_has_its_interface_and_name() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui");
        let tables: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("sound_tables.json")).unwrap()).unwrap();
        let interfaces = tables["interfaces"].as_object().unwrap();
        for m in ALL {
            assert!(interfaces.contains_key(interface(m)), "{} -> {} is not in sound_tables.json", m.name(), interface(m));
            assert_eq!(movie_named(m.name()), Some(m));
            assert_eq!(movie_named(&m.name().to_uppercase()), Some(m));
        }
        let layout = se_core::elite_ui_layout::parse_layout(&std::fs::read(dir.join("layout.json")).unwrap()).unwrap();
        for m in &layout.movies {
            let which = movie_named(&m.name).unwrap_or_else(|| panic!("{} is not in the scope", m.name));
            // (A panel movie's component names its interface.)
            if let Some(c) = &m.component {
                assert!(c.movie.eq_ignore_ascii_case(interface(which)), "{}: component {} against {}", m.name, c.movie, interface(which));
            }
        }
        for m in HUD {
            assert!(layout.movies.iter().any(|l| l.name == m.name()), "{} is not in the layout", m.name());
        }
    }

    #[test]
    fn errors_are_told_from_traces() {
        assert!(is_error("TypeError: Error #1009: Cannot access a property"));
        assert!(is_error("no listener for message Foo"));
        assert!(is_error("Break: x"));
        assert!(!is_error("hello from the movie"));
    }
}
