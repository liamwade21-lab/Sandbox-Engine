//! Flying the Sidewinder in normal space (step 1.3): `elite_sim::flight` stepped at a fixed tick from Elite's keys, the ship
//! moving in its body's turning frame (as Elite's ships near a planet hover over a spot), the body's gravity as the outside
//! pull the flight assist holds against, the ground under it from the terrain's loaded heights, a plain touchdown with the
//! gear down by Elite's touchdown rule; the power distributor (pips, capacitors) and boost (step 2.1, `elite_sim::power`,
//! `elite_sim::boost`).
//!
//! - The tick is the physics world's 60 Hz, at most its 4 sub-steps a frame (`elite_sim::flight::PHYSICS_HZ`,
//!   `MAX_SUBSTEPS`: the game's record), the rest dropped (the spec's edge cases), on real seconds (the game clock runs at
//!   real time while seated). Life support runs on the frame's whole real seconds, outside that cap.
//! - The ship meets the ground's loaded heights and, near it, the buildings and trees the scatter's probe lists (`solids.rs`,
//!   ours: F). Its systems (heat, shields, fuel, the lasers), the frame shift drive, the glide, the air and the panels are
//!   below; what is a stand-in and what is measured is in `elite_sim` (each number labelled) and the spec's Build log.

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{DQuat, DVec3, Vec2};
use bevy::prelude::*;
use elite_sim::boost::{Boost, BoostSpec, Denied};
use elite_sim::flight::{self, Input, Motion};
use elite_sim::fuel::{Fuel, FuelWarning};
use elite_sim::heat::{Heat, HeatSpec};
use elite_sim::shields::{Hull, Shields};
use elite_sim::drive::{Drive, Engage, ShipState};
use elite_sim::glide::{self, Glide};
use elite_sim::landing;
use elite_sim::supercruise::{self, Well};
use elite_sim::weapons::Weapons;
use se_core::catalog::{BodyTag, Catalog, ObjectRef};
use se_core::upos::UPos;
use elite_sim::power::{Distributor, ENG, SYS, WEP};

use crate::data::SeData;
use crate::mods::ModControl;
use crate::sandbox::{Mode, Sandbox};
use crate::system::SystemBody;
use crate::terrain::Terrain;
use crate::universe::SimTime;

use super::seat::{action_pressed, Seat};
use super::{Sidewinder, SidewinderAssets};

/// The simulation's tick: the physics world's rate (B, `elite_sim::flight::PHYSICS_HZ`); at most
/// `elite_sim::flight::MAX_SUBSTEPS` of them a frame.
pub const TICK: f64 = 1.0 / flight::PHYSICS_HZ;
/// Supercruise's pose: where the ship is in the universe, its turn (model space to universe axes) and its speed along the nose.
#[derive(Clone, Copy, Debug)]
pub struct Cruise {
    pub pos: UPos,
    pub rot: DQuat,
    pub speed: f64,
    /// The wells' cap at the last tick (the HUD's gauge).
    pub cap: f64,
    /// The body the ship goes with (the nearest by its surface) and where it was last frame: the ship is carried by its real
    /// motion each frame, so supercruise's speed is relative to it (F: the frame Elite measures supercruise's speed in was not
    /// found in the exe; going with the nearest body keeps a ship leaving Earth from being swept 30 km/s along its orbit).
    pub carrier: Option<(ObjectRef, UPos)>,
}

/// The air the ship flies through (step 6.3, ours): the body it was read for, its model, and the hull's triangles' area
/// vectors (model space) the drag's outline is measured from.
#[derive(Default)]
pub struct AirState {
    obj: Option<ObjectRef>,
    model: Option<crate::viltrumite::air::AirModel>,
    areas: Vec<glam::DVec3>,
}

/// The planet HUD's readings (step 4.3): near a body's ground in normal flight or the glide.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlanetHud {
    /// Over the ground under the ship (m), its vertical speed (m/s, up positive), the heading (degrees from the body's north,
    /// clockwise), the nose's pitch over the horizon and the roll (degrees, right wing down positive).
    pub alt_m: f64,
    pub vspeed: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
}

/// The ship's one locked destination (U1, A9; grade E: how Elite plays, not traced in the exe): a body of this system
/// (`Pilot::nav_body`) or another system's star, hyperspace's target (`Pilot::target`), never both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    Body(ObjectRef),
    System(ObjectRef),
}

impl Dest {
    pub fn obj(self) -> ObjectRef {
        match self {
            Dest::Body(o) | Dest::System(o) => o,
        }
    }
}

/// The locked destination as the HUD shows it this frame: written by `fly` once a frame (from where the frame's ticks left
/// the ship), read by the HUD and nothing else (A9).
#[derive(Clone, Debug, PartialEq)]
pub struct DestMark {
    pub obj: ObjectRef,
    /// The unit direction to it in ship space (x right, y up, -z the nose).
    pub dir: Vec3,
    /// Its distance (m): for the body whose frame the ship is in, the height over its loaded ground at any height
    /// (`own_body_distance`); for another body of this system, to its mean radius; for another system, to the star.
    pub dist_m: f64,
    /// The distance is shown in the altimeter's form: it is the height over the ground and the planet HUD shows it too.
    pub over_ground: bool,
    /// Its angular radius (radians; 0 for another system's star, a right angle from inside a body's radius).
    pub ang_radius: f32,
    /// Its display name, made when the lock changes (kept from frame to frame while the object is the same).
    pub name: String,
    /// Another system's star.
    pub system: bool,
}

impl DestMark {
    /// Its distance as the HUD shows it: as the altimeter does while it is the height over the ground (so the two read the
    /// same), else as the panels do.
    pub fn distance_text(&self) -> String {
        if self.over_ground {
            super::panels::altitude_text(self.dist_m)
        } else {
            super::panels::distance_text(self.dist_m)
        }
    }
}

/// Whether the planet HUD shows `alt` m over the loaded ground (out of supercruise, near a body that isn't a star).
fn planet_hud_shows(alt: f64, star: bool, cruise: bool) -> bool {
    !cruise && !star && alt < PLANET_HUD_M
}

/// The distance of a destination that is the body whose frame the ship is in, `alt` m over its loaded ground: that height
/// at any height and in supercruise (never under 0: under a giant's cloud tops it reads 0), so it runs on without a jump
/// through the planet HUD's height; only its text's form changes there (the altimeter's while the planet HUD shows it).
fn own_body_distance(alt: f64, star: bool, cruise: bool) -> (f64, bool) {
    (alt.max(0.0), planet_hud_shows(alt, star, cruise))
}

/// The mark of a destination `to` away (m, the universe's axes) with radius `radius_m` (0 for another system's star,
/// `system`), seen from a ship turned `rot_u` (its axes to the universe's). `last` is last frame's mark: its name is kept
/// while the object is the same, else `name` makes it.
pub fn dest_mark(last: Option<DestMark>, obj: ObjectRef, to: DVec3, radius_m: f64, rot_u: DQuat, system: bool, name: impl FnOnce() -> String) -> DestMark {
    let d = to.length();
    let ang_radius = if system {
        0.0
    } else if d <= radius_m {
        std::f32::consts::FRAC_PI_2
    } else {
        (radius_m / d).asin() as f32
    };
    let name = match last {
        Some(m) if m.obj == obj => m.name,
        _ => name(),
    };
    DestMark {
        obj,
        dir: (rot_u.inverse() * to).normalize_or(DVec3::NEG_Z).as_vec3(),
        dist_m: if system { d } else { (d - radius_m).max(0.0) },
        over_ground: false,
        ang_radius,
        name,
        system,
    }
}

/// What a destination is called in the messages and the log ("Target: NAME", "Destination: NAME", or none locked).
pub fn dest_text(catalog: &Catalog, dest: Option<Dest>) -> String {
    match dest {
        Some(Dest::System(t)) => format!("Target: {}", catalog.display_name(t)),
        Some(Dest::Body(o)) => format!("Destination: {}", catalog.display_name(o)),
        None => "Destination cleared".to_string(),
    }
}

/// The planet HUD shows within this height of a solid body's ground (ours: Elite shows it in orbital cruise and below, whose
/// start wasn't found: A60).
pub const PLANET_HUD_M: f64 = 100_000.0;

/// Under this height over the ground the ship asks the probe for the buildings and trees ahead (m: the tallest buildings
/// stand about 1.6 km).
const TOWN_ASK_M: f64 = 2500.0;

/// What a SpaceEngine object is, for Elite's supercruise rules (the wells and the escape cone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// A star, or a barycentre (a star record with no body of its own).
    Star { barycenter: bool },
    Body(BodyTag),
    /// Anything else (galaxies, clusters, nebulae: never a system's member).
    Other,
}

fn class_of(cat: &Catalog, obj: ObjectRef) -> Class {
    match obj {
        ObjectRef::Star(i) => Class::Star { barycenter: cat.stars.get(i as usize).is_some_and(|s| s.barycenter) },
        ObjectRef::Body(i) => cat.bodies.get(i as usize).map_or(Class::Other, |b| Class::Body(b.tag)),
        _ => Class::Other,
    }
}

/// A body's supercruise well by Elite's kinds (`elite_sim::supercruise::Well`): a star is a star (kind 1); SpaceEngine's
/// planets, moons and dwarf planets follow the planet rule (Elite's kinds 2 and 7: the study proves the rule, not which
/// bodies; the decision of task N1b-2). Asteroids, comets, spacecraft and barycentres have no well and no escape cone of their
/// own (`Well::Other`): Elite has no such free bodies, so this part is ours (grade F); it also keeps a 5 km asteroid from a
/// planet's 20 AU well.
fn well_of(class: Class, radius: f64, mass: impl FnOnce() -> f64) -> Well {
    match class {
        Class::Star { barycenter: false } => Well::Star { radius },
        Class::Body(BodyTag::Planet | BodyTag::DwarfPlanet | BodyTag::Moon | BodyTag::DwarfMoon) => Well::Planet { radius, mass: mass() },
        Class::Star { barycenter: true } | Class::Body(BodyTag::Asteroid | BodyTag::Comet | BodyTag::Spacecraft) | Class::Other => Well::Other { radius },
    }
}

/// The escape cone leaving a body for supercruise (degrees from straight away from it): `Fp[0x34]` from a planet-rule body
/// (kind 2), `Fp[0x30]` from a star (`elite_sim::drive::escape_deg`); none from a body without a well (ours, F, as
/// `well_of`): the whole sphere.
fn escape_cone_deg(well: Well) -> f64 {
    match well {
        Well::Planet { .. } => elite_sim::drive::escape_deg(true),
        Well::Star { .. } => elite_sim::drive::escape_deg(false),
        Well::Other { .. } => 180.0,
    }
}

/// Too close to a body's surface in supercruise: an emergency drop (E: A50 found the drop's effects but not its triggers),
/// within this share of its radius or this many metres, whichever is more; and the shields' hit it costs (E).
const EMERGENCY_SHARE: f64 = 0.02;
const EMERGENCY_M: f64 = 2000.0;
const EMERGENCY_HIT: f64 = 40.0;


/// A crash's damage (MJ, through the shield then the hull) from the speed into the ground: Elite's collision rule was not
/// studied (A21's list), so this is a stand-in (E): nothing under `CRASH_FREE_MPS`, then half the mass (t) times the excess
/// speed squared over 1,000 (a 50 m/s belly landing breaks the stock shield; at about 100 m/s the ship is lost).
const CRASH_FREE_MPS: f64 = 10.0;
fn crash_damage(mass_t: f64, speed: f64) -> f64 {
    0.5 * mass_t * (speed - CRASH_FREE_MPS).max(0.0).powi(2) / 1000.0
}

/// The user's own Elite's mouse sensitivity (their `Custom.4.2.binds`), the value every measuring session was flown with.
pub const DEFAULT_MOUSE_SENSITIVITY: f32 = 0.2725;
/// The Settings window's slider's range (ours, F; U3, ARCHITECTURE.md A9); a saved value is clamped to it.
pub const MOUSE_SENSITIVITY_RANGE: std::ops::RangeInclusive<f32> = 0.05..=2.0;

/// The mouse's sensitivity (U3, ARCHITECTURE.md A9): the player's own setting, as in Elite (the preset file says 1.0, the
/// player's bindings file holds theirs). The default is the user's own Elite's 0.2725, their decision of 2026-11-23 ("Yes,
/// use 0.27"). Always there, the mod on or off; `settings.rs` saves it, and the mod reads nothing of the settings.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct MouseSetting {
    pub sensitivity: f32,
}

impl Default for MouseSetting {
    fn default() -> Self {
        Self { sensitivity: DEFAULT_MOUSE_SENSITIVITY }
    }
}

/// The mouse counts that move the virtual stick from the middle to full at this sensitivity (`MOUSE_FULL_COUNTS` is 1.0's;
/// a sensitivity that isn't finite counts as the default).
pub fn full_counts(sensitivity: f32) -> f32 {
    let s = if sensitivity.is_finite() { sensitivity } else { DEFAULT_MOUSE_SENSITIVITY };
    elite_sim::data::measured::MOUSE_FULL_COUNTS as f32 / s.max(0.01)
}

/// What the seat's keys asked for this frame (read by `seat::take_keys` before it takes the presses from everyone else).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PilotKeys {
    pub gear: bool,
    pub flight_assist: bool,
    pub throttle_zero: bool,
    pub throttle_up: bool,
    pub throttle_down: bool,
    pub yaw: f64,
    pub lateral: f64,
    pub vertical: f64,
    /// Mouse counts this frame (x right, y down).
    pub mouse: Vec2,
    /// The pad's sticks bound to roll and pitch (`ControlPad.binds`: the left stick), after its dead zone: they replace the
    /// mouse's virtual stick while deflected (step 7.1).
    pub pad_roll: f64,
    pub pad_pitch: f64,
    /// The distributor's keys (`IncreaseEnginesPower` and the rest) and boost (`UseBoostJuice`), pressed this frame.
    pub pip_eng: bool,
    pub pip_sys: bool,
    pub pip_wep: bool,
    pub pip_reset: bool,
    pub boost: bool,
    /// The lasers: the trigger held (`PrimaryFire`) and the hardpoints' key pressed (`DeployHardpointToggle`).
    pub fire: bool,
    pub hardpoints: bool,
    /// The frame shift drive's key (`HyperSuperCombination`, J), and the target ahead (`SelectTarget`, T).
    pub jump: bool,
    pub select_target: bool,
    /// The side panels' keys (step 4.2).
    pub panel: super::panels::PanelKeys,
    /// Silent running (`ToggleButtonUpInput`, Delete), the cargo scoop (`ToggleCargoScoop`, Home), the lights
    /// (`ShipSpotLightToggle`, Insert).
    pub silent: bool,
    pub scoop: bool,
    pub lights: bool,
    /// Elite's UI buttons pressed and released this frame (`ui_edges`), for Elite's own panels (`SE_SW_UI=1`: the focus
    /// machine, `ui::focus`).
    pub ui: UiEdges,
    /// The scanner's range keys pressed this frame (`RadarIncreaseRange`, `RadarDecreaseRange`: PageUp, PageDown in the
    /// shipped binds), for Elite's scanner (`SE_SW_UI=1`: `ui::focus` hands them to `ui::scanner::RadarKeys`). Flight keys
    /// in Elite: a focused panel does not quiet them (no UI button shares their keys).
    pub radar_up: bool,
    pub radar_down: bool,
    /// The FSS's enter and quit keys pressed this frame (`ExplorationFSSEnter`, `ExplorationFSSQuit`: ' and Backspace or ' in
    /// the shipped binds), for Elite's FSS screen (`SE_SW_UI=1`: `ui::screens`).
    pub fss_enter: bool,
    pub fss_quit: bool,
}

/// Elite's UI buttons the cockpit's panels take (`re/elite/notes/ui_rendering.md` 4.1 and 4.2: the focus keys, `UIFocus`, the
/// modal set's and the comms pages'), in the order a frame's edges reach the focus machine: `UIFocus` first, so a direction
/// pressed in the frame `UIFocus` goes down finds `Select UI Mode`.
pub const UI_BUTTONS: [&str; 17] = [
    "UIFocus", "FocusLeftPanel", "FocusRightPanel", "FocusCommsPanel", "QuickCommsPanel", "FocusRadarPanel", "UI_Up", "UI_Down", "UI_Left", "UI_Right",
    "UI_Select", "UI_Back", "UI_Toggle", "CycleNextPanel", "CyclePreviousPanel", "CycleNextPage", "CyclePreviousPage",
];

/// Keys of Elite's fixed `ui` control set (not rebindable; `ui_rendering.md` 4.1, DirectInput scancodes) that the shipped binds
/// do not already give: CyclePreviousPage 26 (`[`) and CycleNextPage 27 (`]`). Left out (ours, E: Elite's rule between the
/// fixed set and the binds was not decoded): Select on Enter (the binds' QuickCommsPanel: in flight one press would open the
/// mini comms panel and select in it), CycleNextPanel on Tab (the binds' boost), Back on Esc (the port's menu, which the seat
/// leaves alone), and the codes the notes could not name (121 to 126, 106). The rest (W S A D, Space, E, Q) are the binds' own.
const UI_FIXED: [(&str, KeyCode); 2] = [("CyclePreviousPage", KeyCode::BracketLeft), ("CycleNextPage", KeyCode::BracketRight)];

/// A frame's presses (`down`) and releases (`up`) of Elite's UI buttons, and which are down now (`held`), one bit each in
/// `UI_BUTTONS`' order. A frame whose keys are not the pilot's (`seat::take_keys`' `free`: a menu up, the window not in
/// front) gives none, so nothing is held then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiEdges {
    pub down: u32,
    pub up: u32,
    pub held: u32,
}

impl UiEdges {
    /// Whether a button (of `UI_BUTTONS`) is down this frame.
    pub fn is_held(self, button: &str) -> bool {
        UI_BUTTONS.iter().position(|b| *b == button).is_some_and(|i| self.held & (1 << i) != 0)
    }

    /// The edges in order, (button, pressed): a button pressed and released within the frame gives both, the press first.
    pub fn iter(self) -> impl Iterator<Item = (&'static str, bool)> {
        UI_BUTTONS.iter().enumerate().flat_map(move |(i, &name)| {
            let bit = 1u32 << i;
            [(self.down & bit != 0).then_some((name, true)), (self.up & bit != 0).then_some((name, false))].into_iter().flatten()
        })
    }
}

/// One button's edge from its keys' states this frame (`now`: one of them is down; `before`: one was down last frame;
/// `tapped`: one went down this frame): (pressed, released). A press and release within one frame is both.
pub fn ui_edge(now: bool, before: bool, tapped: bool) -> (bool, bool) {
    match (now, before) {
        (true, false) => (true, false),
        (false, true) => (false, true),
        (false, false) => (tapped, tapped),
        (true, true) => (false, false),
    }
}

/// The frame's edges of Elite's UI buttons from every key the shipped binds give each (keyboard keys without modifiers, as
/// `action_pressed` reads them, mouse buttons, the pad's buttons) and the fixed set's (`UI_FIXED`).
fn ui_edges(b: &se_core::elite_asset::Binds, pad_binds: &se_core::elite_asset::Binds, pad: Option<&Gamepad>, keys: &ButtonInput<KeyCode>, buttons: &ButtonInput<MouseButton>) -> UiEdges {
    let mut edges = UiEdges::default();
    for (i, &name) in UI_BUTTONS.iter().enumerate() {
        // Each key: (down now, pressed this frame, released this frame).
        let mut states: Vec<(bool, bool, bool)> = Vec::new();
        for k in b.get(name).into_iter().flat_map(|a| a.keys()) {
            match k.device.as_str() {
                "Keyboard" if k.modifiers.is_empty() => states.extend(super::seat::key_code(&k.key).map(|c| (keys.pressed(c), keys.just_pressed(c), keys.just_released(c)))),
                "Mouse" => states.extend(mouse_button(&k.key).map(|m| (buttons.pressed(m), buttons.just_pressed(m), buttons.just_released(m)))),
                _ => {}
            }
        }
        if let (Some(g), Some(a)) = (pad, pad_binds.get(name)) {
            states.extend(a.keys().filter(|k| is_pad(&k.device)).filter_map(|k| pad_button(&k.key)).map(|bt| (g.pressed(bt), g.just_pressed(bt), g.just_released(bt))));
        }
        states.extend(UI_FIXED.iter().filter(|(n, _)| *n == name).map(|&(_, c)| (keys.pressed(c), keys.just_pressed(c), keys.just_released(c))));
        let now = states.iter().any(|s| s.0);
        // (Down last frame: down now and not pressed this frame, or released this frame after a press before it; a key
        // pressed and released within the frame was not down before it.)
        let before = states.iter().any(|s| (s.0 || s.2) && !s.1);
        let (down, up) = ui_edge(now, before, states.iter().any(|s| s.1));
        edges.down |= u32::from(down) << i;
        edges.up |= u32::from(up) << i;
        edges.held |= u32::from(now) << i;
    }
    edges
}

/// The ship's switches a test can press (`sw silent|scoop|lights`): the same as their keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Silent,
    Scoop,
    Lights,
    /// The gear's key (`sw gear`): refused while landed as the key is.
    Gear,
}

/// The flight's state while seated.
#[derive(Resource)]
pub struct Pilot {
    pub motion: Motion,
    pub throttle: f64,
    pub flight_assist: bool,
    /// The power distributor: pips and capacitors (step 2.1).
    pub dist: Distributor,
    pub boost: Boost,
    /// Heat (step 2.2).
    pub heat: Heat,
    pub heat_spec: HeatSpec,
    /// For the HUD: the full block's top speed, this block's top as a share of it, the blue zone's band (speed shares where
    /// the turn curve is within 2 % of its best).
    pub full_top: f64,
    pub top_share: f64,
    pub blue_zone: (f32, f32),
    /// The ship's systems (steps 2.3, 2.5): they belong to the ship, so boarding keeps them (`Pilot::boarding`) and a new
    /// spawn or the restock makes them full (`Pilot::fresh_ship`, `restock`).
    pub fuel: Fuel,
    pub shields: Shields,
    pub hull: Hull,
    /// The power plant runs (fuel aboard).
    pub powered: bool,
    /// The hull reached zero: `seat_camera` loses the ship and sets him down.
    pub destroyed: bool,
    /// The lasers (step 2.6), and each beam's length this frame (m: to the ground or the lasers' range).
    pub weapons: Weapons,
    pub beam_len: [f32; 2],
    /// The frame shift drive (step 5.2) and, in supercruise, the ship's own pose in the universe (the parked pose follows it
    /// in the nearest body's frame every frame).
    pub drive: Drive,
    pub cruise: Option<Cruise>,
    /// The glide into a planet's lower air (step 6.2), from supercruise.
    pub glide: Option<Glide>,
    /// Hyperspace (step 5.3): the target star, the jump in progress (its tunnel), a test's jump (`sw hyper NAME`).
    pub target: Option<ObjectRef>,
    pub jump: Option<super::hyper::Jump>,
    pub dev_hyper: Option<ObjectRef>,
    pub dev_hyper_fail: bool,
    /// A test's placing of the ship over a body (`sw put NAME ALT_KM`), and the air's density where it is (kg/m^3).
    pub dev_put: Option<(ObjectRef, f64)>,
    /// `sw trace FILE`: a row a tick in the measured traces' columns (`elite_sim::trace`), and when it started (ticks).
    pub trace: Option<(std::io::BufWriter<std::fs::File>, u64)>,
    /// The jump's target and fuel as worked out when its charge started (A50: taken at departure, the least of it and the fuel
    /// aboard).
    pub jump_fuel: Option<(ObjectRef, f64)>,
    pub air_density: f64,
    /// The modules' switches (the right panel's Modules tab, `panels::MODULES`), and the destination picked on the Navigation
    /// tab (a body of this system) with its HUD line. `nav_body` and `target` are the one destination (`Dest`): change them
    /// through `lock` and `unlock`, which keep at most one of them set.
    pub modules: [bool; 10],
    pub nav_body: Option<ObjectRef>,
    pub nav_note: String,
    /// The locked destination's mark for the HUD this frame (`fly`; None with nothing locked, the body not found, or in the
    /// tunnel).
    pub dest: Option<DestMark>,
    pub planet: Option<PlanetHud>,
    /// Silent running, the cargo scoop deployed, the lights on; the canopy and the emergency oxygen (`elite_sim::life`); the
    /// destination's safe disengage (in supercruise with a destination: ready or not).
    pub silent: bool,
    pub scoop: bool,
    pub lights: bool,
    pub life: elite_sim::life::Life,
    pub safe_drop: Option<bool>,
    /// A test's pitch over the horizon to turn to, keeping the heading (`sw aim DEG`), and a test's press of a switch's key
    /// (`sw silent|scoop|lights|gear`).
    pub dev_aim: Option<f64>,
    pub dev_switch: Option<Switch>,
    /// The HUD's line for the target (its name, distance and the jump's fuel, or why not).
    pub target_note: String,
    /// The stars' heat this frame (a share of the capacity a second), for the HUD.
    pub star_heat: f64,
    /// A test's turn to face the target (`sw face`).
    pub dev_face: bool,
    /// The gear's command (L toggles it); its real state is the gear machines' (`parts::Parts::gear`).
    pub gear_down: bool,
    /// A test's placing (`sw put`) happened this frame (it sets the gear up): the gear's machines are settled to it, not
    /// animated (`facts::ShipFacts::settle`). Cleared at the start of every frame's flight.
    pub put: bool,
    pub landed: bool,
    /// Boarded and not yet flown: the flight's first frame decides whether the ship stands landed or flies from the pose it
    /// was left in (`stands_landed`), at this velocity (the prop's at the boarding, body frame, m/s). Meanwhile it counts as
    /// landed but stands in the sandbox's world as nothing (`keep_in_world`).
    pub boarded: Option<DVec3>,
    /// The mouse's virtual stick (x roll, y pitch), -1..1: it stays where the mouse left it (Elite's default: no decay).
    pub stick: Vec2,
    accum: f64,
    pub ticks: u64,
    /// The cockpit point's change of velocity tick by tick in this frame's flight (`record_tick`), for the pilot chair's
    /// buffets (`facts::ShipFacts::ticks`).
    pub tick_dv: TickChanges,
    /// The cockpit point's velocity after the last tick (the flight's rest frame, m/s), None when there is none to compare the
    /// next tick with: a new ship or a boarding (`Pilot::default`), and whenever the motion isn't the flight's.
    tick_cockpit: Option<DVec3>,
    /// A test's controls held for some seconds (`sw stick`, `sw thrust`), and a test's boost press (`sw boost`).
    pub dev: Option<(Input, f32)>,
    pub dev_boost: bool,
    /// A test's trigger held for some seconds (`sw fire SECS`), and a test's press of the drive's key (`sw jump`).
    pub dev_fire: f32,
    pub dev_jump: bool,
    /// A test's mouse counts (`sw mouse DX DY`: x right, y down, as real counts), added to the next flown frame's counts.
    pub dev_mouse: Vec2,
    pub log: bool,
    /// The demands the flight read this frame (`controls`).
    controls: Input,
}

impl Default for Pilot {
    fn default() -> Self {
        Pilot {
            motion: Motion::default(),
            throttle: 0.0,
            flight_assist: true,
            dist: Distributor::new(&elite_sim::data::DISTRIBUTOR_1E),
            boost: Boost::default(),
            heat: Heat::new(&HeatSpec::sidewinder()),
            heat_spec: HeatSpec::sidewinder(),
            full_top: 1.0,
            top_share: 1.0,
            blue_zone: (0.0, 1.0),
            fuel: Fuel::sidewinder(),
            shields: Shields::sidewinder(elite_sim::data::JOURNAL_UNLADEN_MASS + elite_sim::data::FUEL_TANK_T + elite_sim::data::SIDEWINDER.reserve_fuel),
            hull: Hull::sidewinder(),
            powered: true,
            destroyed: false,
            weapons: Weapons::default(),
            beam_len: [0.0; 2],
            drive: Drive::Ready,
            cruise: None,
            glide: None,
            target: None,
            jump: None,
            dev_hyper: None,
            dev_hyper_fail: false,
            dev_put: None,
            trace: None,
            jump_fuel: None,
            air_density: 0.0,
            modules: [true; 10],
            nav_body: None,
            nav_note: String::new(),
            dest: None,
            planet: None,
            silent: false,
            scoop: false,
            lights: false,
            life: elite_sim::life::Life::default(),
            safe_drop: None,
            dev_aim: None,
            dev_switch: None,
            target_note: String::new(),
            star_heat: 0.0,
            dev_face: false,
            gear_down: true,
            put: false,
            landed: true,
            boarded: None,
            stick: Vec2::ZERO,
            accum: 0.0,
            ticks: 0,
            dev: None,
            dev_boost: false,
            dev_fire: 0.0,
            dev_jump: false,
            dev_mouse: Vec2::ZERO,
            log: false,
            controls: Input::default(),
            tick_dv: TickChanges::default(),
            tick_cockpit: None,
        }
    }
}

/// The most ticks one frame's flight runs (`flight::MAX_SUBSTEPS`).
const MAX_TICKS: usize = flight::MAX_SUBSTEPS as usize;

/// The cockpit point's change of velocity of each tick of one frame's flight, in order (ship space's axes, m/s): a fixed
/// list, so nothing is allocated a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TickChanges {
    /// The frame whose flight recorded them (`FrameCount`): they are that frame's only.
    pub frame: u32,
    len: usize,
    dv: [Vec3; MAX_TICKS],
}

impl TickChanges {
    pub fn as_slice(&self) -> &[Vec3] {
        &self.dv[..self.len]
    }

    /// Emptied for `frame`'s flight.
    fn start(&mut self, frame: u32) {
        self.frame = frame;
        self.len = 0;
    }

    /// (A frame runs at most `MAX_TICKS` ticks, so the list never overflows.)
    fn push(&mut self, dv: Vec3) {
        if let Some(slot) = self.dv.get_mut(self.len) {
            *slot = dv;
            self.len += 1;
        }
    }
}

impl Pilot {
    /// After a tick: the cockpit point's (`cockpit`, ship space) velocity, v + w x r (Elite's cockpit-point velocity,
    /// `sidewinder_chair.md` "Buffets"), against the last tick's, turned into the ship's axes, recorded in `tick_dv`. Zero,
    /// with nothing to compare the next tick with, while the motion isn't the flight's: in supercruise (whose speed and turn
    /// live in `Cruise`), in the tunnel and on a test's placing (which resets it). The glide's motion is the flight's.
    fn record_tick(&mut self, cockpit: Vec3) {
        let live = self.cruise.is_none() && self.jump.is_none() && !self.put;
        // (`vel` in the rest frame, `rot` ship axes to it, `omega` in ship axes; elite_sim's glam, so through arrays.)
        let rot = DQuat::from_array(self.motion.rot.to_array());
        let now = live.then(|| DVec3::from_array(self.motion.vel.to_array()) + rot * DVec3::from_array(self.motion.omega.to_array()).cross(cockpit.as_dvec3()));
        let dv = match (now, self.tick_cockpit) {
            (Some(now), Some(last)) => (rot.inverse() * (now - last)).as_vec3(),
            _ => Vec3::ZERO,
        };
        self.tick_dv.push(dv);
        self.tick_cockpit = now;
    }

    /// The flight didn't run this frame (not seated, the tunnel, no ship): its next tick has nothing to compare with.
    fn forget_ticks(&mut self) {
        if self.tick_cockpit.is_some() {
            self.tick_cockpit = None;
        }
    }
}

impl Pilot {
    /// The demands the flight read this frame (the keys, mouse or pad after Elite's dead zone and linearity, or a test's
    /// `sw stick` / `sw thrust`): pitch nose up +, yaw nose left +, roll right wing up +, forward the throttle. Kept from the
    /// last frame the flight ran (seated, in Pilot mode); a boarding starts them at rest.
    pub fn controls(&self) -> Input {
        self.controls
    }

    /// The locked destination (`Dest`), if any.
    pub fn locked(&self) -> Option<Dest> {
        match (self.nav_body, self.target) {
            (Some(o), _) => Some(Dest::Body(o)),
            (None, Some(t)) => Some(Dest::System(t)),
            (None, None) => None,
        }
    }

    /// Every way of picking a destination goes through here (the Navigation tab, T, `sw dest`, `sw target`): `pick` becomes
    /// the ship's one destination and the other kind is cleared; with `again_unlocks` (the panel's pick) picking the one
    /// already locked unlocks it instead. Returns the destination now. (A jump's charge is cancelled by `fly` when the
    /// target changes, as before.)
    pub fn lock(&mut self, pick: Dest, again_unlocks: bool) -> Option<Dest> {
        if again_unlocks && self.locked() == Some(pick) {
            self.nav_body = None;
            self.target = None;
        } else {
            (self.nav_body, self.target) = match pick {
                Dest::Body(o) => (Some(o), None),
                Dest::System(t) => (None, Some(t)),
            };
        }
        self.locked()
    }

    /// Unlocks the destination if it is of the kind given (`system`: another system's star; else a body of this one): T
    /// finding no star ahead clears the star target only, as `sw target none` does; `sw dest none` the body.
    pub fn unlock(&mut self, system: bool) {
        if system {
            self.target = None;
        } else {
            self.nav_body = None;
        }
    }

    /// The HUD's heat percentage.
    pub fn heat_percent(&self) -> f64 {
        self.heat.hud_percent(&self.heat_spec)
    }

    /// Sitting down in the parked ship: the flight starts afresh, the ship's systems (fuel, shields, hull, heat, pips and
    /// capacitors) are kept, and so is its locked destination (grade E: how Elite plays; a new ship, `fresh_ship`, has none).
    /// `vel` is the parked ship's own velocity (body frame, m/s: the prop's, if it was moving): whether it stands landed or
    /// flies from there is the flight's first frame's to decide (`boarded`).
    pub fn boarding(&self, vel: DVec3) -> Pilot {
        // (A parked ship's gear command is down, whatever it was when he left.)
        Pilot {
            boarded: Some(vel),
            fuel: self.fuel,
            shields: self.shields,
            hull: self.hull,
            heat: self.heat.clone(),
            dist: self.dist.clone(),
            modules: self.modules,
            life: self.life,
            nav_body: self.nav_body,
            target: self.target,
            gear_down: true,
            landed: true,
            ..Pilot::default()
        }
    }

    /// A new ship: everything full.
    pub fn fresh_ship(&mut self) {
        *self = Pilot::default();
    }

    /// The restock (the spec's D5): fuel, shields and hull full, heat back to its start.
    pub fn restock(&mut self) {
        self.fuel.refill();
        self.shields.refill();
        self.hull = Hull::sidewinder();
        self.heat = Heat::new(&self.heat_spec);
        self.powered = true;
        self.life = elite_sim::life::Life::default();
    }

    /// The ship's mass now: unladen (the journal's) and the fuel aboard.
    pub fn mass_t(&self) -> f64 {
        elite_sim::data::JOURNAL_UNLADEN_MASS + self.fuel.total()
    }

    /// The gear down or the cargo scoop out halves the top and boost speeds (`elite_sim::deployables`, C; the record's 0.5, B).
    pub fn speed_k(&self) -> f64 {
        elite_sim::deployables::speed_factor(self.gear_down, self.scoop)
    }

    /// The current block and the full-power block's top speed as the flight flies by them: the ENG pips' blend, the mass and
    /// `speed_k` (`elite_sim::data::sidewinder_block_with`).
    pub fn block(&self) -> (elite_sim::flight::Block, f64) {
        elite_sim::data::sidewinder_block_with(self.dist.pips(ENG), self.mass_t(), self.speed_k())
    }
}

/// The speed shares where a blue-zone curve is within 2 % of its best (the HUD's blue band).
fn blue_band(c: &elite_sim::flight::Curve) -> (f32, f32) {
    let best = (0..=100).map(|i| c.at(i as f64 / 100.0)).fold(f64::MIN, f64::max);
    let inside: Vec<f64> = (0..=100).map(|i| i as f64 / 100.0).filter(|x| c.at(*x) >= 0.98 * best).collect();
    (inside.first().copied().unwrap_or(0.0) as f32, inside.last().copied().unwrap_or(1.0) as f32)
}

// elite-sim's glam and Bevy's may be different versions: through arrays.
fn to_sim_v(v: DVec3) -> glam::DVec3 {
    glam::DVec3::from_array(v.to_array())
}
fn from_sim_v(v: glam::DVec3) -> DVec3 {
    DVec3::from_array(v.to_array())
}
fn to_sim_q(q: DQuat) -> glam::DQuat {
    glam::DQuat::from_array(q.to_array())
}
fn from_sim_q(q: glam::DQuat) -> DQuat {
    DQuat::from_array(q.to_array())
}

/// Elite's name for a pad button (`ControlPad.binds`, an Xbox 360 pad) as Bevy's.
pub fn pad_button(name: &str) -> Option<GamepadButton> {
    use GamepadButton as B;
    Some(match name {
        "Pad_A" => B::South,
        "Pad_B" => B::East,
        "Pad_X" => B::West,
        "Pad_Y" => B::North,
        "Pad_LBumper" => B::LeftTrigger,
        "Pad_RBumper" => B::RightTrigger,
        "Pad_LTrigger" => B::LeftTrigger2,
        "Pad_RTrigger" => B::RightTrigger2,
        "Pad_Back" => B::Select,
        "Pad_Start" => B::Start,
        "Pad_LThumb" => B::LeftThumb,
        "Pad_RThumb" => B::RightThumb,
        "Pad_DPadUp" => B::DPadUp,
        "Pad_DPadDown" => B::DPadDown,
        "Pad_DPadLeft" => B::DPadLeft,
        "Pad_DPadRight" => B::DPadRight,
        _ => return None,
    })
}

/// Elite's name for a pad stick axis as Bevy's.
pub fn pad_axis(name: &str) -> Option<GamepadAxis> {
    Some(match name {
        "Pad_LStickX" => GamepadAxis::LeftStickX,
        "Pad_LStickY" => GamepadAxis::LeftStickY,
        "Pad_RStickX" => GamepadAxis::RightStickX,
        "Pad_RStickY" => GamepadAxis::RightStickY,
        _ => return None,
    })
}

fn is_pad(device: &str) -> bool {
    !matches!(device, "Keyboard" | "Mouse" | "{NoDevice}" | "")
}

/// An axis action's value from the pad: its file's binding, inverted flag and dead zone (the dead zone rescaled as the mouse's
/// is: the exe's axis rule was not looked for, a stand-in).
fn pad_value(b: &se_core::elite_asset::Binds, g: &Gamepad, action: &str) -> f64 {
    let Some(a) = b.get(action) else { return 0.0 };
    let Some(k) = a.axis.as_ref().filter(|k| is_pad(&k.device)) else { return 0.0 };
    let Some(axis) = pad_axis(&k.key) else { return 0.0 };
    let v = g.get(axis).unwrap_or(0.0) as f64 * if a.inverted == Some(true) { -1.0 } else { 1.0 };
    let dz = a.deadzone.unwrap_or(0.0).clamp(0.0, 0.95);
    if v.abs() <= dz { 0.0 } else { ((v.abs() - dz) / (1.0 - dz)).copysign(v) }
}

/// Reads the seat's keys by Elite's own bindings: the keyboard's (`KeyboardMouseOnly.binds`) and, if a pad is there, the pad's
/// (`ControlPad.binds`) (`seat::take_keys` calls this before it takes the presses).
/// Elite's name for a mouse button as Bevy's.
fn mouse_button(name: &str) -> Option<MouseButton> {
    Some(match name {
        "Mouse_1" => MouseButton::Left,
        "Mouse_2" => MouseButton::Right,
        "Mouse_3" => MouseButton::Middle,
        _ => return None,
    })
}

/// Elite's control set a focused panel adds (`ui/controls.json` `bindings_interfacemodal`, A: a test checks it against the
/// shipped file; its on-foot button, `HumanoidToggleMissionHelpPanelButton`, left out) and the one the comms panel adds for its
/// pages (`bindings_interfacemodal_pagination`).
pub const MODAL_SET: [&str; 9] = ["UI_Up", "UI_Down", "UI_Left", "UI_Right", "UI_Select", "CyclePreviousPanel", "CycleNextPanel", "UI_Back", "UI_Toggle"];
pub const PAGINATION_SET: [&str; 2] = ["CyclePreviousPage", "CycleNextPage"];

/// The frame's input as the ship's actions see it: the keys bound (in the shipped binds, and the fixed set's `UI_FIXED`) to
/// a button of the UI's live control sets (`sets`: the modal set while a panel has the keys, the pages in the comms panel)
/// are quiet for every ship action, whatever it is (in the shipped binds: W S A D the throttle and yaw, Q E the lateral
/// thrust, the arrows the pips, Delete silent running, Home the cargo scoop, Insert the lights, Z flight assist, the right
/// button secondary fire); keys the UI does not use go on flying the ship. The one rule for the stand-in panels and Elite's.
/// Elite's notes (`re/elite/notes/ui_rendering.md` 4.2; `ui_messages_panels.md` 12.3, the states' activities) say a focused
/// panel adds `Bindings_InterfaceModal` (the comms states `_Pagination` too) to the live control sets, with the sets' members
/// (A); how a live set's keys block another set's was not decoded (the sets' flags, E): that the UI's keys are the UI's alone
/// while those sets are live is our reading of it. Returned: the keys, the mouse buttons, and the pad's buttons taken.
fn ship_input(b: &se_core::elite_asset::Binds, pad_binds: &se_core::elite_asset::Binds, keys: &ButtonInput<KeyCode>, buttons: &ButtonInput<MouseButton>, sets: super::panels::UiSets) -> (ButtonInput<KeyCode>, ButtonInput<MouseButton>, Vec<GamepadButton>) {
    let (mut keys, mut buttons, mut pad) = (keys.clone(), buttons.clone(), Vec::new());
    // Elite's FSS screen is an input mode of its own (B: Elite's observed behaviour, the ship cannot be flown from it; the
    // per-set flags of its input schemes were not decoded): every ship action is quiet; its own keys are read unmasked.
    if sets.screen {
        keys.reset_all();
        buttons.reset_all();
        let all = pad_binds.actions.values().flat_map(|a| a.keys()).filter(|k| is_pad(&k.device)).filter_map(|k| pad_button(&k.key));
        return (keys, buttons, all.collect());
    }
    let taken = MODAL_SET.iter().filter(|_| sets.modal).chain(PAGINATION_SET.iter().filter(|_| sets.pages));
    for &action in taken {
        for k in b.get(action).into_iter().flat_map(|a| a.keys()) {
            match k.device.as_str() {
                "Keyboard" => super::seat::key_code(&k.key).into_iter().for_each(|c| keys.reset(c)),
                "Mouse" => mouse_button(&k.key).into_iter().for_each(|m| buttons.reset(m)),
                _ => {}
            }
        }
        pad.extend(pad_binds.get(action).into_iter().flat_map(|a| a.keys()).filter(|k| is_pad(&k.device)).filter_map(|k| pad_button(&k.key)));
        UI_FIXED.iter().filter(|(n, _)| *n == action).for_each(|&(_, c)| keys.reset(c));
    }
    (keys, buttons, pad)
}

/// The seat's keys this frame: the UI's buttons from the input as it is (`ui_edges`, the stand-in panels' keys, the FSS's
/// keys), the ship's actions from it without the keys the UI's live control sets take (`ship_input`, `sets`).
#[allow(clippy::too_many_arguments)]
pub fn read_keys(b: &se_core::elite_asset::Binds, pad_binds: &se_core::elite_asset::Binds, pad: Option<&Gamepad>, keys: &ButtonInput<KeyCode>, buttons: &ButtonInput<MouseButton>, motion: &AccumulatedMouseMotion, real: bool, sets: super::panels::UiSets) -> PilotKeys {
    let ui_press = |a: &str| action_pressed(b, keys, a, true) || pad.zip(pad_binds.get(a)).is_some_and(|(g, a)| a.keys().filter(|k| is_pad(&k.device)).filter_map(|k| pad_button(&k.key)).any(|bt| g.just_pressed(bt)));
    let panel = super::panels::PanelKeys {
        left: ui_press("FocusLeftPanel"),
        right: ui_press("FocusRightPanel"),
        up: ui_press("UI_Up"),
        down: ui_press("UI_Down"),
        change_left: ui_press("UI_Left"),
        change_right: ui_press("UI_Right"),
        select: ui_press("UI_Select"),
        back: ui_press("UI_Back"),
        next_tab: ui_press("CycleNextPanel"),
        prev_tab: ui_press("CyclePreviousPanel"),
    };
    let (fss_enter, fss_quit) = (ui_press("ExplorationFSSEnter"), ui_press("ExplorationFSSQuit"));
    let ui = ui_edges(b, pad_binds, pad, keys, buttons);
    let (keys, buttons, pad_taken) = ship_input(b, pad_binds, keys, buttons, sets);
    let (keys, buttons) = (&keys, &buttons);
    let mouse_held = |action: &str| b.get(action).is_some_and(|a| a.keys().filter(|k| k.device == "Mouse").filter_map(|k| mouse_button(&k.key)).any(|m| buttons.pressed(m)));
    let pad_btn = |action: &str, just: bool| -> bool {
        let (Some(g), Some(a)) = (pad, pad_binds.get(action)) else { return false };
        a.keys().filter(|k| is_pad(&k.device)).filter_map(|k| pad_button(&k.key)).filter(|bt| !pad_taken.contains(bt)).any(|bt| if just { g.just_pressed(bt) } else { g.pressed(bt) })
    };
    let held = |a: &str| action_pressed(b, keys, a, false) || pad_btn(a, false);
    let press = |a: &str| action_pressed(b, keys, a, true) || pad_btn(a, true);
    let axis = |plus: &str, minus: &str| held(plus) as i8 as f64 - held(minus) as i8 as f64;
    let pv = |action: &str| pad.map_or(0.0, |g| pad_value(pad_binds, g, action));
    let k = PilotKeys {
        gear: press("LandingGearToggle"),
        flight_assist: press("ToggleFlightAssist"),
        throttle_zero: press("SetSpeedZero"),
        throttle_up: held("ForwardKey"),
        throttle_down: held("BackwardKey"),
        // (Yaw on the pad's axis: Elite's yaw axis is positive to the right, the simulation's to the left.)
        yaw: (axis("YawLeftButton", "YawRightButton") - pv("YawAxisRaw")).clamp(-1.0, 1.0),
        lateral: (axis("RightThrustButton", "LeftThrustButton") + pv("LateralThrustRaw")).clamp(-1.0, 1.0),
        vertical: (axis("UpThrustButton", "DownThrustButton") + pv("VerticalThrustRaw")).clamp(-1.0, 1.0),
        mouse: if real { motion.delta } else { Vec2::ZERO },
        // (Roll on the pad: stick right rolls right, a negative roll about the back axis.)
        pad_roll: -pv("RollAxisRaw"),
        pad_pitch: pv("PitchAxisRaw"),
        pip_eng: press("IncreaseEnginesPower"),
        pip_sys: press("IncreaseSystemsPower"),
        pip_wep: press("IncreaseWeaponsPower"),
        pip_reset: press("ResetPowerDistribution"),
        boost: press("UseBoostJuice"),
        fire: real && (mouse_held("PrimaryFire") || pad.is_some_and(|_| pad_btn("PrimaryFire", false))),
        hardpoints: press("DeployHardpointToggle"),
        jump: press("HyperSuperCombination"),
        select_target: press("SelectTarget"),
        panel,
        silent: press("ToggleButtonUpInput"),
        scoop: press("ToggleCargoScoop"),
        lights: press("ShipSpotLightToggle"),
        ui,
        radar_up: press("RadarIncreaseRange"),
        radar_down: press("RadarDecreaseRange"),
        fss_enter,
        fss_quit,
    };
    k
}

/// The canopy and the emergency oxygen (step 2.3's F6: `elite_sim::life`), once a frame on the frame's whole real seconds:
/// outside the flight's capped ticks (exact at any frame rate: the rule is linear in time) and before the tunnel's early
/// return (life support runs whatever the ship is doing). The module needs its switch and the power plant. Out of oxygen the
/// ship is lost through `destroyed`, which `seat_camera`'s one way out of the seat (`release`) takes wherever the ship is,
/// the tunnel too.
fn life_support(pilot: &mut Pilot, real_dt: f64, msg: &mut crate::messages::Messages) {
    pilot.life.hull(pilot.hull.percent() / 100.0);
    let was_running = pilot.life.running();
    let lost = pilot.life.tick(real_dt, pilot.powered && pilot.modules[super::panels::M_LIFE_SUPPORT]);
    if pilot.life.running() != was_running {
        info!("sidewinder: emergency oxygen {} ({:.1} s left, canopy {})", if was_running { "off" } else { "running" }, pilot.life.oxygen, if pilot.life.canopy { "whole" } else { "breached" });
    }
    if lost && !pilot.destroyed {
        msg.show("Oxygen depleted".to_string());
        info!("sidewinder: the emergency oxygen ran out{}", if pilot.jump.is_some() { " in hyperspace" } else { "" });
        pilot.destroyed = true;
    }
}

/// The Navigation tab's pick (`Pilot::lock`: the locked one again unlocks it), shown and logged.
fn panel_pick(pilot: &mut Pilot, pick: Dest, catalog: &Catalog, msg: &mut crate::messages::Messages) {
    let text = dest_text(catalog, pilot.lock(pick, true));
    info!("sidewinder: the panel's pick: {text}");
    msg.show(text);
}

/// Seated in Pilot mode: the frame's controls, the ticks, the ground. The camera follows from the ship's pose (`seat_camera`,
/// which runs after this).
#[allow(clippy::too_many_arguments)]
pub fn fly(
    time: Res<Time<Real>>,
    data: Res<SeData>,
    sb: Res<Sandbox>,
    seat: Res<Seat>,
    control: Res<ModControl>,
    keys: Res<PilotKeys>,
    assets: Option<Res<SidewinderAssets>>,
    mut ship: ResMut<Sidewinder>,
    mut pilot: ResMut<Pilot>,
    bodies: Query<(&SystemBody, Option<&Terrain>)>,
    mut msg: ResMut<crate::messages::Messages>,
    sim: Res<SimTime>,
    mut air_cache: ResMut<crate::ship::air::AirCache>,
    mut air: Local<AirState>,
    mut panels: ResMut<super::panels::Panels>,
    (mut ask, probe, lib, mut town, parts, frame, mouse, ui_live): (ResMut<crate::scatter::probe::ProbeAsk>, Res<crate::scatter::probe::BuildingProbe>, Option<Res<crate::scatter::library::ScatterLibrary>>, Local<(Option<u32>, Vec<super::solids::Obb>)>, Res<super::parts::Parts>, Res<bevy::diagnostic::FrameCount>, Res<MouseSetting>, Res<super::ui::UiLive>),
) {
    // The gear's real state, as the gear machines left it (`parts::update`, last frame).
    let gear = parts.gear();
    if pilot.put {
        pilot.put = false;
    }
    // (This frame's ticks' changes of the cockpit's velocity: none until a tick runs.)
    pilot.tick_dv.start(frame.0);
    let (Some(assets), true) = (assets, seat.seated && control.pilot && sb.mode == Mode::Pilot) else {
        pilot.forget_ticks();
        pilot.dest = None;
        return;
    };
    // In the tunnel the old system's bodies go: `hyper::tunnel` holds the ship and the camera until the new one is entered.
    life_support(&mut pilot, time.delta_secs_f64(), &mut msg);
    if pilot.jump.is_some() {
        pilot.forget_ticks();
        pilot.dest = None;
        return;
    }
    let Some(p) = ship.parked.as_mut() else {
        pilot.forget_ticks();
        pilot.dest = None;
        return;
    };
    // A test's placing (`sw put NAME ALT_KM`): over the body's day side, level, in thruster flight, the gear up.
    let put_now = pilot.dev_put.is_some();
    if let Some((obj, alt)) = pilot.dev_put.take() {
        if let Some((tb, tt)) = bodies.iter().find(|(sb, _)| sb.obj == obj) {
            let sun = bodies.iter().find(|(sb, _)| sb.star).map(|(sb, _)| sb.pos);
            let up = sun.map_or(DVec3::Y, |s| (tb.orient.inverse() * s.delta_m(tb.pos)).normalize_or(DVec3::Y));
            let solid_t = tt.filter(|t| !t.proc.kind.is_giant());
            let ground = crate::ship::ground_on(tb.radius_m, tb.shape, solid_t, up * tb.radius_m).0;
            p.obj = obj;
            p.pos = up * (ground + alt);
            let flat = up.any_orthonormal_vector();
            p.rot = DQuat::from_mat3(&bevy::math::DMat3::from_cols(flat.cross(up), up, -flat)).normalize();
            pilot.cruise = None;
            pilot.glide = None;
            pilot.landed = false;
            pilot.gear_down = false;
            pilot.put = true;
            pilot.motion = Motion { rot: to_sim_q(p.rot), ..Motion::default() };
            info!("sidewinder: put over {obj:?} at {:.1} km", alt / 1000.0);
        }
    }
    let Some((b, terrain)) = bodies.iter().find(|(b, _)| b.obj == p.obj) else {
        pilot.dest = None;
        return;
    };
    // Gas giants (step 6.3, ours: F4): no ground but the port's own floor, 10 scale heights under the radius, which stops the
    // ship (it can't land there); the glide, the beams and the air's heights go by the radius.
    let giant = terrain.is_some_and(|t| t.proc.kind.is_giant());
    let solid = if giant { None } else { terrain };
    let floor_m = if giant { terrain.and_then(crate::volumetric_clouds::giant_floor_m).unwrap_or(0.0) } else { 0.0 };
    // The buildings and trees near the ground (checker 4; ours, F: `solids.rs`): out of supercruise and the glide, under
    // `TOWN_ASK_M`, the probe is asked along the ship's next 1.5 s; its list (3-5 frames old, the body's frame) becomes boxes
    // once each time it changes.
    if pilot.cruise.is_none() && pilot.glide.is_none() && !giant && p.pos.length() - crate::ship::ground_on(b.radius_m, b.shape, solid, p.pos).0 < TOWN_ASK_M {
        let at = b.pos.offset_m(b.orient * p.pos);
        let ahead = b.orient * from_sim_v(pilot.motion.vel) * 1.5;
        ask.capsule = Some(crate::scatter::probe::ProbeCapsule { from: at, to: at.offset_m(ahead), margin_m: 30.0, both: false, trees_m: 20.0 });
    }
    let listed = probe.body.filter(|_| probe.arrived).and_then(|e| bodies.get(e).ok()).map(|(sb, _)| sb.obj) == Some(b.obj);
    match (&lib, listed) {
        (Some(lib), true) if town.0 != Some(probe.list_seq) => {
            town.0 = Some(probe.list_seq);
            town.1 = probe.list.iter().flat_map(|pb| super::solids::boxes_of(pb, lib)).collect();
        }
        (_, false) => {
            town.0 = None;
            town.1.clear();
        }
        _ => {}
    }
    if put_now && giant {
        info!("sidewinder: a gas giant: its floor {:.1} km under the radius", -floor_m / 1000.0);
    }
    // The air (ours): the body's model, read when the ship comes to a new body; the hull's area vectors once.
    if air.obj != Some(b.obj) {
        let eph = se_core::ephemeris::Ephemeris::new(&data.catalog);
        air.model = crate::viltrumite::air::AirModel::of(&mut air_cache, &data, &eph, sim.jd, b.obj, b.radius_m, terrain);
        air.obj = Some(b.obj);
    }
    if air.areas.is_empty() {
        let at = |p: Vec3| glam::DVec3::from_array(p.as_dvec3().to_array());
        air.areas.extend(assets.shape.drag_hull.iter().map(|t| elite_sim::air::area_vector(at(t[0]), at(t[1]), at(t[2]))));
    }
    let pilot = &mut *pilot;
    let dt = (time.delta_secs() as f64).min(0.25);
    // The side panels (step 4.2): their keys and head look; what they ask for goes in as the keys would. While Elite's UI is
    // made and running (`UiLive`) its focus machine drives `panels` and pushes the panels' asks (`ui::focus`, `ui::panels`).
    if !ui_live.0 {
        panels.update(&keys.panel, seat.head_look.then_some(seat.look.x));
    }
    let mut keys = *keys;
    for a in std::mem::take(&mut panels.actions) {
        use super::panels::{PanelAction, MODULES, M_FSD};
        match a {
            PanelAction::Target(t) => panel_pick(pilot, Dest::System(t), &data.catalog, &mut msg),
            PanelAction::NavBody(o) => panel_pick(pilot, Dest::Body(o), &data.catalog, &mut msg),
            PanelAction::Unlock => {
                pilot.unlock(false);
                pilot.unlock(true);
                let text = dest_text(&data.catalog, pilot.locked());
                info!("sidewinder: the panel's unlock: {text}");
                msg.show(text);
            }
            PanelAction::FlightAssist => keys.flight_assist = true,
            PanelAction::Gear => keys.gear = true,
            PanelAction::Hardpoints => keys.hardpoints = true,
            PanelAction::Jump => keys.jump = true,
            PanelAction::Lights => keys.lights = true,
            PanelAction::Silent => keys.silent = true,
            PanelAction::Scoop => keys.scoop = true,
            PanelAction::Module(i) if i == M_FSD && pilot.modules[i] && (pilot.cruise.is_some() || !matches!(pilot.drive, Drive::Ready | Drive::Arriving(_) | Drive::Cooldown(_))) => {
                msg.show("The frame shift drive is in use".to_string());
            }
            PanelAction::Module(i) => {
                pilot.modules[i] = !pilot.modules[i];
                msg.show(format!("{}: {}", MODULES[i].0, if pilot.modules[i] { "on" } else { "off" }));
            }
        }
    }
    // The controls of this frame.
    if matches!(pilot.dev_switch, Some(Switch::Gear)) {
        pilot.dev_switch = None;
        keys.gear = true;
    }
    if keys.gear {
        match gear_toggle(pilot.landed, pilot.gear_down) {
            Ok(down) => {
                pilot.gear_down = down;
                msg.show(if down { "Landing gear deployed" } else { "Landing gear retracted" }.to_string());
            }
            Err(why) => {
                msg.show(why.to_string());
                info!("sidewinder: gear refused: {why}");
            }
        }
    }
    if keys.flight_assist {
        pilot.flight_assist = !pilot.flight_assist;
        msg.show(if pilot.flight_assist { "Flight assist on" } else { "Flight assist off" }.to_string());
    }
    if keys.throttle_zero {
        pilot.throttle = 0.0;
    }
    // The distributor's keys (step 2.1).
    for (pressed, c) in [(keys.pip_eng, ENG), (keys.pip_sys, SYS), (keys.pip_wep, WEP)] {
        if pressed {
            pilot.dist.increase(c);
        }
    }
    if keys.pip_reset {
        pilot.dist.reset();
    }
    if keys.hardpoints && (pilot.cruise.is_some() || pilot.glide.is_some()) {
        msg.show(if pilot.glide.is_some() { "Hardpoints can't be deployed in a glide" } else { "Hardpoints can't be deployed in supercruise" }.to_string());
    } else if keys.hardpoints {
        pilot.weapons.deployed = !pilot.weapons.deployed;
        msg.show(if pilot.weapons.deployed { "Hardpoints deployed" } else { "Hardpoints retracted" }.to_string());
    }
    // Silent running (`elite_sim::silent`: the radiator off, C; the shields off, B), the cargo scoop (it refuses the drive in
    // the exe's order, C; refused in supercruise: ours, F), the lights (a switch and the HUD's sign; the beam is drawn
    // elsewhere). `Pilot`'s `silent`, `scoop` and `lights` are what the rest of the mod reads.
    match pilot.dev_switch.take() {
        Some(Switch::Silent) => keys.silent = true,
        Some(Switch::Scoop) => keys.scoop = true,
        Some(Switch::Lights) => keys.lights = true,
        Some(Switch::Gear) | None => {}
    }
    let mut said = |m: &str| {
        info!("sidewinder: {m}");
        msg.show(m.to_string());
    };
    if keys.silent {
        pilot.silent = !pilot.silent;
        said(if pilot.silent { "Silent running" } else { "Silent running off" });
    }
    if keys.scoop && pilot.cruise.is_some() && !pilot.scoop {
        said("Cargo scoop can't be deployed in supercruise");
    } else if keys.scoop {
        pilot.scoop = !pilot.scoop;
        said(if pilot.scoop { "Cargo scoop deployed" } else { "Cargo scoop retracted" });
    }
    if keys.lights {
        pilot.lights = !pilot.lights;
        said(if pilot.lights { "Lights on" } else { "Lights off" });
    }
    let rate = elite_sim::data::measured::THROTTLE_RATE * dt;
    pilot.throttle = (pilot.throttle + rate * (keys.throttle_up as i8 - keys.throttle_down as i8) as f64).clamp(-1.0, 1.0);
    // The mouse's virtual stick: counts move it, it stays (no decay in Elite's default file); its dead zone is the file's,
    // its sensitivity the player's setting (`MouseSetting`, U3: never the file's).
    // A test's counts (`sw mouse DX DY`) join the frame's real ones here.
    keys.mouse += std::mem::take(&mut pilot.dev_mouse);
    // (Elite's FSS screen takes the mouse and the sticks for its own view, `Seat::screen`: the ship flies on straight.)
    if seat.screen { (keys.mouse, keys.pad_roll, keys.pad_pitch, pilot.stick) = (Vec2::ZERO, 0.0, 0.0, Vec2::ZERO); }
    let setting = |name: &str, or: f32| assets.keys.value(name).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(or);
    let full = full_counts(mouse.sensitivity);
    pilot.stick = (pilot.stick + Vec2::new(keys.mouse.x, -keys.mouse.y) / full).clamp(Vec2::splat(-1.0), Vec2::splat(1.0));
    // The exe's rule (0x1408F2210, step 0.11): stick = sign(v) x ((|v| - dead zone) / (1 - dead zone)) ^ linearity.
    // (Without the setting in the file, the exe's defaults: dead zone 0.05, linearity 1: its table 0x145E77BD0.)
    let (dead, lin) = (setting("MouseDeadzone", 0.05), setting("MouseLinearity", 1.0));
    let dz = |v: f32| if v.abs() <= dead { 0.0 } else { ((v.abs() - dead) / (1.0 - dead)).powf(lin).copysign(v) as f64 };
    let mut input = Input {
        // (Mouse right rolls right: the right wing down, a negative roll about the back axis.) The pad's stick or the mouse's,
        // whichever is larger (the exe writes the mouse's value only when it is larger than the axis's other sources:
        // 0x1408F2210, step 0.11).
        roll: larger(keys.pad_roll, -dz(pilot.stick.x)),
        pitch: larger(keys.pad_pitch, dz(pilot.stick.y)),
        yaw: keys.yaw,
        lateral: keys.lateral,
        vertical: keys.vertical,
        forward: pilot.throttle,
        flight_assist: pilot.flight_assist,
    };
    if let Some((dev, left)) = pilot.dev.as_mut() {
        input = Input { flight_assist: pilot.flight_assist, forward: if dev.forward != 0.0 { dev.forward } else { pilot.throttle }, ..*dev };
        *left -= dt as f32;
        if *left <= 0.0 {
            pilot.dev = None;
        }
    }
    pilot.controls = input;

    let gm = se_core::units::G * crate::ship::body_mass(&data, b.obj, b.radius_m);
    // Boarded: landed only if it stands on its gear by the touchdown's own test; else it flies from the pose it was left in
    // (hanging frozen in the air, tumbling, tilted), its thrusters from hover thrust as a liftoff starts them, at the velocity
    // it had. Getting out stays the landed ship's alone: the pilot flies it level and lands it.
    if let Some(vel) = pilot.boarded.take() {
        let feet: Vec<f64> = parts.feet(&assets.shape).iter().map(|f| over_ground(b, solid, floor_m, p.pos, p.rot, *f)).collect();
        let tilt = (p.rot * DVec3::Y).dot(p.pos.normalize()).clamp(-1.0, 1.0).acos().to_degrees();
        pilot.landed = !giant && stands_landed(gear.deployed(), tilt, &feet, vel.length());
        if !pilot.landed {
            pilot.motion = Motion { rot: to_sim_q(p.rot), vel: to_sim_v(vel), last_acc: to_sim_v(hover_acc(p.pos, p.rot, gm)), ..Motion::default() };
        }
        info!(
            "sidewinder: boarded {}: {tilt:.1} degrees of tilt, the feet {:?} m over the ground, {:.2} m/s",
            if pilot.landed { "landed" } else { "flying (not standing on its gear)" },
            feet.iter().map(|f| (f * 100.0).round() / 100.0).collect::<Vec<_>>(),
            vel.length()
        );
    }
    let speed_k = pilot.speed_k();
    let (block, full_top) = pilot.block();
    let spec = BoostSpec::sidewinder(block.boost_ramp);
    pilot.full_top = full_top;
    pilot.top_share = block.top_speed / full_top.max(1e-9);
    let cruise_block = elite_sim::data::sidewinder_cruise_block();
    pilot.blue_zone = blue_band(if pilot.cruise.is_some() { &cruise_block.curve_turn } else { &block.curve_turn });
    // Boost: the press draws its cost from the ENG capacitor or is refused (the exe's messages, our words until step 3.3).
    // (Landed it does nothing: the flight doesn't run on the ground. Not from the exe: E.)
    let boost_pressed = keys.boost || std::mem::take(&mut pilot.dev_boost);
    if boost_pressed && !pilot.landed && pilot.cruise.is_none() && pilot.glide.is_none() {
        match pilot.boost.press(&spec, &mut pilot.dist, true) {
            Ok(()) => info!("sidewinder: boost"),
            Err(Denied::NotEnoughEnergy) => msg.show("Boost: not enough engine power".to_string()),
            Err(Denied::EngineOff) => msg.show("Boost: the engines are off".to_string()),
            Err(Denied::Busy) => {}
        }
    }
    // A test's turn (`sw aim DEG`): the nose `deg` over the body's horizon, the heading kept, the wings level.
    if let Some(deg) = pilot.dev_aim.take() {
        let up = p.pos.normalize_or(DVec3::Y);
        let nose = p.rot * DVec3::NEG_Z;
        let flat = (nose - up * nose.dot(up)).normalize_or(up.any_orthonormal_vector());
        let (s, c) = deg.to_radians().sin_cos();
        let fwd = flat * c + up * s;
        let right = fwd.cross(up).normalize_or(flat.cross(up));
        let rot = DQuat::from_mat3(&bevy::math::DMat3::from_cols(right, right.cross(fwd), -fwd)).normalize();
        p.rot = rot;
        if let Some(cr) = pilot.cruise.as_mut() {
            cr.rot = (b.orient * rot).normalize();
        }
        pilot.motion.omega = Default::default();
    }
    if std::mem::take(&mut pilot.dev_face) {
        if let Some(t) = pilot.locked().map(Dest::obj) {
            let from = pilot.cruise.map_or(b.pos.offset_m(b.orient * p.pos), |c| c.pos);
            let dir = se_core::ephemeris::Ephemeris::new(&data.catalog).position(t, sim.jd).delta_m(from).normalize_or(DVec3::NEG_Z);
            let rot_u = DQuat::from_rotation_arc(DVec3::NEG_Z, dir);
            match pilot.cruise.as_mut() {
                Some(c) => c.rot = rot_u,
                None => p.rot = (b.orient.inverse() * rot_u).normalize(),
            }
            pilot.motion.omega = Default::default();
        }
    }
    let (ship_u, ship_rot_u) = match pilot.cruise {
        Some(c) => (c.pos, c.rot),
        None => (b.pos.offset_m(b.orient * p.pos), (b.orient * p.rot).normalize()),
    };
    // Hyperspace's target (step 5.3): T locks the star nearest the nose within the jump's range (ours: Elite targets
    // systems from its Navigation panel and galaxy map, step 4.2), or, finding none, clears the star target only (a locked
    // body stays); then its angle from the nose, or why it can't be jumped to.
    let here = data.catalog.system_root(b.obj);
    if keys.select_target {
        let range = elite_sim::fsd::jump_range(&elite_sim::data::FSD_2E, pilot.mass_t(), pilot.fuel.main);
        match super::hyper::star_ahead(&data, sim.jd, ship_u, ship_rot_u * DVec3::NEG_Z, range, here) {
            Some(t) => msg.show(dest_text(&data.catalog, pilot.lock(Dest::System(t), false))),
            None => {
                pilot.unlock(true);
                msg.show("No target".to_string());
            }
        }
    }
    let others: Vec<(DVec3, f64)> = bodies.iter().map(|(sb, _)| (sb.pos.delta_m(ship_u), sb.radius_m)).collect();
    if panels.open == Some(super::panels::Side::Left) && (panels.nav.is_empty() || pilot.ticks % 60 == 0) {
        use se_core::catalog::{BodyTag, ObjectRef as O};
        let mut rows: Vec<super::panels::NavRow> = bodies
            .iter()
            .filter_map(|(sb, t)| {
                let kind = if sb.star {
                    "STAR"
                } else if t.is_some_and(|t| t.proc.kind.is_giant()) {
                    "GAS GIANT"
                } else {
                    match sb.obj {
                        O::Body(i) => match data.catalog.bodies[i as usize].tag {
                            BodyTag::Planet => "PLANET",
                            BodyTag::Moon | BodyTag::DwarfMoon => "MOON",
                            BodyTag::DwarfPlanet => "DWARF PLANET",
                            _ => return None,
                        },
                        _ => return None,
                    }
                };
                (sb.star || sb.radius_m >= 100_000.0).then(|| super::panels::NavRow { obj: sb.obj, name: data.catalog.display_name(sb.obj).to_string(), kind, dist_m: (sb.pos.delta_m(ship_u).length() - sb.radius_m).max(0.0), other_system: false })
            })
            .collect();
        rows.sort_by(|a, b2| a.dist_m.total_cmp(&b2.dist_m));
        rows.truncate(30);
        let range = elite_sim::fsd::jump_range(&elite_sim::data::FSD_2E, pilot.mass_t(), pilot.fuel.main);
        let eph = se_core::ephemeris::Ephemeris::new(&data.catalog);
        let mut stars: Vec<super::panels::NavRow> = super::hyper::stars_within(&data, sim.jd, ship_u, range, here)
            .into_iter()
            .map(|t| super::panels::NavRow { obj: t, name: data.catalog.display_name(t).to_string(), kind: "SYSTEM", dist_m: eph.position(t, sim.jd).delta_m(ship_u).length(), other_system: true })
            .collect();
        stars.sort_by(|a, b2| a.dist_m.total_cmp(&b2.dist_m));
        rows.extend(stars);
        panels.nav = rows;
    }
    // The locked destination's place and radius this frame (a body found among this system's, or another system's star by
    // the ephemeris): its mark and the HUD's line are made from it below, once the ticks have moved the ship.
    let dest_at = match pilot.locked() {
        Some(Dest::Body(o)) => bodies.iter().find(|(sb, _)| sb.obj == o).map(|(sb, _)| (o, sb.pos, sb.radius_m, false)),
        Some(Dest::System(t)) => Some((t, se_core::ephemeris::Ephemeris::new(&data.catalog).position(t, sim.jd), 0.0, true)),
        None => None,
    };
    // The stars' heat (step 5.4: A50's form, its numbers stand-ins), a share of the heat capacity a second.
    let star_heat: f64 = bodies.iter().filter(|(sb, _)| sb.star).map(|(sb, _)| elite_sim::stars::heat_rate(sb.pos.delta_m(ship_u).length(), sb.radius_m)).sum();
    pilot.star_heat = star_heat;
    if matches!(pilot.drive, Drive::JumpCharging(_) | Drive::Departing(_)) && pilot.jump_fuel.map(|j| j.0) != pilot.target {
        pilot.drive = if pilot.cruise.is_some() { Drive::Supercruise } else { Drive::Ready };
        msg.show("Frame shift drive charge cancelled: the target changed".to_string());
    }
    let target_state = pilot.target.map(|t| super::hyper::check(&data, sim.jd, t, ship_u, ship_rot_u * DVec3::NEG_Z, pilot.mass_t(), pilot.fuel.main, here, &others));
    pilot.target_note = match (pilot.target, target_state) {
        (Some(t), Some(s)) => {
            let ly = se_core::ephemeris::Ephemeris::new(&data.catalog).position(t, sim.jd).delta_m(ship_u).length() / se_core::units::METERS_PER_LY;
            match s {
                Ok((_, fuel)) => format!("{}  {:.2} ly  {:.2} t", data.catalog.display_name(t), ly, fuel),
                Err(r) => format!("{}  {:.2} ly  ({})", data.catalog.display_name(t), ly, r.message().trim_start_matches("Frame shift drive: ")),
            }
        }
        _ => String::new(),
    };
    // The frame shift drive (step 5.2): J charges (or cancels), and in supercruise asks for a drop; with a target, a jump's.
    let nose_body = p.rot * DVec3::NEG_Z;
    let fsd = ShipState {
        hardpoints: pilot.weapons.deployed,
        // The gear's request, not its legs: the drive's refusal (FUN_142c71440 near line 1620) reads FUN_142c99670's bit 2,
        // which is the control state's byte +0xBB, the request the speed rule reads too. So it charges while the legs rise.
        gear_down: pilot.gear_down,
        scoop: pilot.scoop,
        landed: pilot.landed,
        escape_angle: nose_body.dot(p.pos.normalize_or(DVec3::Y)).clamp(-1.0, 1.0).acos().to_degrees(),
        radii: p.pos.length() / b.radius_m.max(1.0),
        cruise: pilot.cruise.is_some(),
        target: target_state.map(|s| s.map(|(angle, _)| angle)),
    };
    // The escape cone from the body the ship is by (its well's kind: the cone doesn't read the mass).
    let escape_deg = escape_cone_deg(well_of(class_of(&data.catalog, b.obj), b.radius_m, || 0.0));
    let mut drop_now = false;
    let jump = keys.jump || std::mem::take(&mut pilot.dev_jump);
    if jump && !pilot.modules[super::panels::M_FSD] {
        msg.show("Frame shift drive: switched off".to_string());
    } else if jump && pilot.glide.is_some() {
        msg.show("Frame shift drive: not during a glide".to_string());
    } else if jump {
        match pilot.drive.press(&fsd) {
            Ok(true) => drop_now = true,
            Ok(false) => {
                if matches!(pilot.drive, Drive::JumpCharging(_)) {
                    pilot.jump_fuel = pilot.target.zip(target_state.and_then(|s| s.ok()).map(|(_, f)| f));
                }
                msg.show(match pilot.drive {
                    Drive::Charging(_) => "Frame shift drive charging".to_string(),
                    Drive::JumpCharging(_) => format!("Frame shift drive charging: {}", pilot.target.map_or("", |t| data.catalog.display_name(t))),
                    _ => "Frame shift drive charge cancelled".to_string(),
                });
            }
            Err(r) => {
                info!("sidewinder: {}", r.message());
                msg.show(r.message().to_string());
            }
        }
    }
    // The bodies' wells this frame (their places move slowly against a tick).
    let wells: Vec<(Well, UPos, f64, ObjectRef, DQuat)> = if pilot.cruise.is_some() || matches!(pilot.drive, Drive::Charging(_)) {
        bodies
            .iter()
            .map(|(sb, _)| (well_of(class_of(&data.catalog, sb.obj), sb.radius_m, || crate::ship::body_mass(&data, sb.obj, sb.radius_m)), sb.pos, sb.radius_m, sb.obj, sb.orient))
            .collect()
    } else {
        Vec::new()
    };
    let mut emergency = false;
    let mut jump_now = false;
    let mut too_fast_stop = false;
    let mut glide_into: Option<ObjectRef> = None;
    // The bodies a glide can go into (solid ground: a terrain, not a star or a gas giant) and a point's height over their loaded
    // ground (the point in the body's frame).
    // (A gas giant's glide goes by its radius: F4, ours; so does a body without a terrain. None has an exclusion zone: C14.)
    let glidable: Vec<(ObjectRef, f64, DVec3, Option<&Terrain>)> = if pilot.cruise.is_some() {
        bodies.iter().filter(|(sb, _)| !sb.star).map(|(sb, t)| (sb.obj, sb.radius_m, sb.shape, t.filter(|t| !t.proc.kind.is_giant()))).collect()
    } else {
        Vec::new()
    };
    let height_over = |obj: ObjectRef, local: DVec3| {
        glidable.iter().find(|g| g.0 == obj).map(|g| local.length() - crate::ship::ground_on(g.1, g.2, g.3, local).0)
    };
    // The glide's end tests, once a frame (A60: terrain within 10 km along the nose, the surface within 6,500 m below; a ray
    // stepped against the loaded heights, as the beams').
    let (glide_ahead, glide_below) = if pilot.glide.is_some() {
        let dir = p.rot * DVec3::NEG_Z;
        let mut ahead = f64::MAX;
        let mut s = 0.0;
        while s < glide::END_AHEAD_M {
            let at = p.pos + dir * s;
            if at.length() < crate::ship::ground_on(b.radius_m, b.shape, solid, at).0 {
                ahead = s;
                break;
            }
            s += 50.0;
        }
        (ahead, p.pos.length() - crate::ship::ground_on(b.radius_m, b.shape, solid, p.pos).0)
    } else {
        (f64::MAX, f64::MAX)
    };
    // Supercruise is carried by its body's real motion this frame (and takes the nearest body as its carrier from now on).
    if let Some(c) = pilot.cruise.as_mut() {
        if let Some((obj, last)) = c.carrier {
            if let Some((_, now, _, _, _)) = wells.iter().find(|w| w.3 == obj) {
                c.pos = c.pos.offset_m(now.delta_m(last));
            }
        }
        let nearest = wells.iter().min_by(|a, b2| (c.pos.delta_m(a.1).length() - a.2).total_cmp(&(c.pos.delta_m(b2.1).length() - b2.2)));
        c.carrier = nearest.map(|w| (w.3, w.1));
    }
    pilot.accum += dt;
    let mut n = 0;
    // (Each tick's change of the cockpit's velocity is recorded once it has ended: at the next tick's start, or after the
    // loop for the last; the loop's `continue` and `break` both pass one of the two.)
    let cockpit = Vec3::from_array(assets.eye);
    while pilot.accum >= TICK && n < flight::MAX_SUBSTEPS {
        if n > 0 {
            pilot.record_tick(cockpit);
        }
        pilot.accum -= TICK;
        n += 1;
        pilot.ticks += 1;
        // Fuel (step 2.5): the burn from the reservoir; dry, the plant gives nothing (no thrust, no recharge, no shields).
        let (share, warning) = pilot.fuel.tick(TICK);
        if let Some(w) = warning {
            msg.show(match w {
                FuelWarning::Quarter => "Fuel at 25 %",
                FuelWarning::Tenth => "Fuel at 10 %",
                FuelWarning::Twentieth => "Fuel at 5 %",
            }.to_string());
        }
        if pilot.powered && share < 1.0 {
            pilot.powered = false;
            msg.show("Out of fuel: the power plant has stopped".to_string());
        }
        let powered = pilot.powered;
        pilot.dist.recharge(TICK, powered);
        let sys_share = pilot.dist.share(SYS);
        pilot.shields.tick(TICK, elite_sim::silent::shields_powered(powered && pilot.modules[super::panels::M_SHIELDS], pilot.silent), sys_share);
        // Heat: the idle load (a stand-in fitted to session 2's idle) and the boost's while it fires (the boost record's
        // 10 a second, B), then the radiator (step 2.2).
        let hs = pilot.heat_spec;
        if powered {
            pilot.heat.add(&hs, elite_sim::heat::stand_in::IDLE_LOAD * TICK, None);
        }
        pilot.heat.add(&hs, pilot.boost.heat_rate(&spec) * TICK, None);
        pilot.boost.tick(&spec, TICK);
        let ev = pilot.heat.tick(&hs, TICK, powered, pilot.silent);
        // The lasers (step 2.6): both guns of the fire group, WEP's energy and heat a shot.
        let trigger = (keys.fire || pilot.dev_fire > 0.0) && powered && pilot.cruise.is_none() && pilot.glide.is_none();
        let guns = super::panels::M_LASERS.map(|m| trigger && pilot.modules[m]);
        pilot.weapons.tick_guns(TICK, guns, &mut pilot.dist, &mut pilot.heat, &hs);
        if ev.warning {
            msg.show("Heat warning".to_string());
        }
        if ev.damage_warning {
            msg.show("Heat damage".to_string());
        }
        if ev.damage > 0.0 && pilot.hull.damage(ev.damage * pilot.hull.max) {
            pilot.destroyed = true;
        }
        if star_heat > 0.0 {
            pilot.heat.add(&hs, star_heat * hs.capacity * TICK, None);
        }
        // The jump's charge's and countdown's heat (A50's form: D[0x0C] a second, more past the charge's time while held; the
        // numbers stand-ins).
        let charge_heat = match pilot.drive {
            Drive::JumpCharging(t) => elite_sim::hyperspace::stand_in::DEPARTING_HEAT_RATE + elite_sim::hyperspace::stand_in::OVERCHARGE_HEAT_RATE * (t - elite_sim::drive::stand_in::JUMP_CHARGE_S).max(0.0),
            Drive::Departing(_) => elite_sim::hyperspace::stand_in::DEPARTING_HEAT_RATE,
            _ => 0.0,
        };
        if charge_heat > 0.0 {
            pilot.heat.add(&hs, charge_heat * hs.capacity * TICK, None);
        }
        match pilot.drive.tick_escaping(TICK, &fsd, escape_deg) {
            Ok(Engage::Jump) => {
                jump_now = true;
                break;
            }
            Ok(Engage::Supercruise) => {
                // (At the zero-throttle speed, supercruise's entry speed: P10, B.)
                let start = supercruise::MIN_SPEED;
                pilot.cruise = Some(Cruise { pos: b.pos.offset_m(b.orient * p.pos), rot: (b.orient * p.rot).normalize(), speed: start, cap: start, carrier: Some((b.obj, b.pos)) });
                msg.show("Supercruise".to_string());
                info!("sidewinder: supercruise at {:.0} km/s (the escape cone {escape_deg:.0} degrees, the nose {:.1} from straight away)", start / 1000.0, fsd.escape_angle);
            }
            Err(r) => {
                info!("sidewinder: {}", r.message());
                msg.show(r.message().to_string());
            }
            Ok(Engage::Nothing) => {}
        }
        if let Some(c) = pilot.cruise.as_mut() {
            // Supercruise: the turn by the supercruise block (its turn rates, accelerations, jerks and curves: the record's, B),
            // in accel mode 2 so its top speed is the wells' cap and its speed curve reads the speed over the cap (the block's
            // x); the speed towards the throttle's share of the cap (A50), along the nose; no gravity, no ground.
            let throttle = pilot.throttle.max(0.0);
            let near: Vec<(Well, f64)> = wells.iter().map(|(w, at, _, _, _)| (*w, c.pos.delta_m(*at).length())).collect();
            let cap = supercruise::cap(&near);
            c.cap = cap;
            // (The gear's and the scoop's factor scales the cruise block as it does the normal one: `supercruise::cruise_block`.)
            let turning = supercruise::cruise_block(cap, speed_k, c.speed);
            let mut m = Motion { rot: to_sim_q(c.rot), omega: pilot.motion.omega, last_alpha: pilot.motion.last_alpha, ..Motion::default() };
            m.vel = m.rot * glam::DVec3::new(0.0, 0.0, -c.speed);
            let rin = Input { pitch: input.pitch, yaw: input.yaw, roll: input.roll, flight_assist: true, ..Input::default() };
            // (The second argument is the full-power block's top speed: in supercruise the record's Vsc.)
            flight::step(&turning, supercruise::VSC, false, &rin, glam::DVec3::ZERO, &mut m, TICK);
            c.rot = from_sim_q(m.rot).normalize();
            pilot.motion.omega = m.omega;
            pilot.motion.last_alpha = m.last_alpha;
            // (The jerk limit's memory, the flight controller's +0x630, kept in `Motion::last_acc` forward along z: every drop
            // and glide resets it; it carries only into supercruise and across a hyperspace jump.)
            let mut speed = supercruise::Speed { v: c.speed, last_acc: pilot.motion.last_acc.length() };
            speed.step_held_with(throttle, cap, speed_k, TICK);
            c.speed = speed.v;
            pilot.motion.last_acc = glam::DVec3::new(0.0, 0.0, speed.last_acc);
            let heading = c.rot * DVec3::NEG_Z;
            let step = heading * c.speed * TICK;
            // A60's swept test (checker 3: tested at one point a tick, a fast ship went from above the entry height to under
            // the ground in one tick): this tick's path against each solid body's entry shell (its radius and the entry height),
            // each star's exclusion zone and each other body's emergency zone. The first crossing stops the ship on it: the
            // glide, or an emergency stop if it closes too fast (TooFastNearPlanet) or the body isn't one to glide into.
            let mut hit: Option<(f64, ObjectRef, bool)> = None;
            for (w, at, r, obj, _) in &wells {
                // (A flattened body's shell from its smallest radius, so the per-tick test over the ground keeps the 25 km.)
                let flat = glidable.iter().find(|g| g.0 == *obj).map(|g| g.2.min_element());
                let glides = flat.is_some();
                let shell = match w {
                    _ if glides => r * flat.unwrap_or(1.0) + glide::stand_in::ENTRY_M,
                    _ => supercruise::exclusion_radius(*w).unwrap_or(r + (r * EMERGENCY_SHARE).max(EMERGENCY_M)),
                };
                if let Some(t) = supercruise::sweep(to_sim_v(c.pos.delta_m(*at)), to_sim_v(step), shell) {
                    if hit.is_none_or(|h| t < h.0) {
                        hit = Some((t, *obj, glides));
                    }
                }
            }
            let closing = |c: &Cruise, at: UPos| c.speed * heading.dot(at.delta_m(c.pos).normalize_or_zero());
            if let Some((t, obj, glides)) = hit {
                c.pos = c.pos.offset_m(step * t);
                let fast = wells.iter().find(|w| w.3 == obj).is_some_and(|w| supercruise::too_fast(closing(c, w.1)));
                if glides && !fast {
                    glide_into = Some(obj);
                } else {
                    emergency = true;
                    too_fast_stop = glides;
                }
                break;
            }
            c.pos = c.pos.offset_m(step);
            // (Only heading into it: leaving a body from its surface is how supercruise starts.) A body with solid ground:
            // heading down within the glide's entry height over its loaded ground (the spec's hint: a slow approach over ground
            // higher than the shell), the glide (step 6.2), unless closing too fast.
            if let Some((_, at, _, obj, _)) = wells.iter().find(|(_, at, _, obj, orient)| {
                heading.dot(at.delta_m(c.pos)) > 0.0 && height_over(*obj, orient.inverse() * c.pos.delta_m(*at)).is_some_and(|h| h < glide::stand_in::ENTRY_M)
            }) {
                if supercruise::too_fast(closing(c, *at)) {
                    emergency = true;
                    too_fast_stop = true;
                } else {
                    glide_into = Some(*obj);
                }
                break;
            }
            // (A star's zone is its exclusion zone, 1.05 of its radius: step 5.4.)
            if wells.iter().any(|(w, at, r, obj, _)| {
                let to = at.delta_m(c.pos);
                let inside = match supercruise::exclusion_radius(*w) {
                    Some(zone) => to.length() < zone,
                    None => !glidable.iter().any(|g| g.0 == *obj) && to.length() - r < (r * EMERGENCY_SHARE).max(EMERGENCY_M),
                };
                inside && heading.dot(to) > 0.0
            }) {
                emergency = true;
                break;
            }
            continue;
        }
        // The glide (A60): the velocity is its speed along the nose; the turn by the normal block with the speed curve read at
        // the throttle, as in supercruise (the glide's turn rates are not known: E); no thrusters, no gravity. The ground check
        // below still runs.
        let mut gliding = false;
        if let Some(g) = pilot.glide.as_mut() {
            let nose = p.rot * DVec3::NEG_Z;
            let down = -p.pos.normalize_or(DVec3::Y);
            let pitch = glide::pitch_deg(nose.to_array(), down.to_array());
            let k = g.tick(TICK, pitch, glide_ahead, glide_below);
            if let Some(e) = k.end {
                msg.show(e.message().to_string());
                info!("sidewinder: glide ended ({e:?}) at pitch {pitch:.0}, {glide_below:.0} m up");
            }
            if k.heat_share > 0.0 {
                pilot.heat.add(&hs, k.heat_share * hs.capacity, None);
            }
            match k.speed {
                Some(v) => {
                    let throttle = pilot.throttle.max(0.0);
                    pilot.motion.rot = to_sim_q(p.rot);
                    pilot.motion.vel = pilot.motion.rot * glam::DVec3::new(0.0, 0.0, -throttle * block.top_speed);
                    let rin = Input { pitch: input.pitch, yaw: input.yaw, roll: input.roll, flight_assist: true, ..Input::default() };
                    flight::step(&block, full_top, false, &rin, glam::DVec3::ZERO, &mut pilot.motion, TICK);
                    p.rot = from_sim_q(pilot.motion.rot).normalize();
                    let nose = p.rot * DVec3::NEG_Z;
                    pilot.motion.vel = to_sim_v(nose * v);
                    p.pos += nose * v * TICK;
                    gliding = true;
                }
                None => {
                    pilot.glide = None;
                    msg.show("Normal flight".to_string());
                    info!("sidewinder: glide over, normal flight");
                }
            }
        }
        if !gliding {
            if pilot.landed {
                // Lift off: up thrust (Elite lifts a landed ship on its vertical thrusters).
                if input.vertical <= 0.0 {
                    continue;
                }
                pilot.landed = false;
                // (The thrusters start from the thrust that holds the ship against gravity, so the jerk limit (block 0x28) ramps
                // from hovering: from zero the ship sank back for the 0.4 s the limit takes to reach 1 g. Elite doesn't fly a
                // landed ship, so how its liftoff starts is not known: E.)
                pilot.motion = Motion { rot: to_sim_q(p.rot), last_acc: to_sim_v(hover_acc(p.pos, p.rot, gm)), ..Motion::default() };
                msg.show("Liftoff".to_string());
                info!("sidewinder: liftoff");
            }
            let r = p.pos.length().max(1.0);
            let up = p.pos / r;
            // The air's drag (ours, F4): against the motion through the air (it turns with the body, as this frame does), the
            // outline along it, the density at this height over the ground (a giant's: over its radius, denser below it).
            let h = r - crate::ship::ground_on(b.radius_m, b.shape, solid, p.pos).0;
            let density = air.model.map_or(0.0, |m| m.at(h).density);
            let vel = from_sim_v(pilot.motion.vel);
            let drag = if density > 0.0 && vel.length() > 1e-6 {
                let along = p.rot.inverse() * vel.normalize();
                let area = elite_sim::air::outline_area(&air.areas, glam::DVec3::from_array(along.to_array()));
                from_sim_v(elite_sim::air::drag_in(elite_sim::air::Flight::Thrusters, to_sim_v(vel), density, area, pilot.mass_t() * 1000.0))
            } else {
                DVec3::ZERO
            };
            pilot.air_density = density;
            let outside = -up * gm / (r * r) + drag;
            pilot.motion.rot = to_sim_q(p.rot);
            let tick_input = if powered && pilot.modules[super::panels::M_THRUSTERS] { Input { forward: pilot.boost.forward(&spec, input.forward), ..input } } else { Input { flight_assist: false, ..Input::default() } };
            flight::step(&block, full_top, pilot.boost.firing(), &tick_input, to_sim_v(outside), &mut pilot.motion, TICK);
            p.rot = from_sim_q(pilot.motion.rot);
            p.pos += from_sim_v(pilot.motion.vel) * TICK;
        }
        // `sw trace FILE`: this tick as a measured trace's row (the keys from this frame's controls and the effective input).
        if let Some((w, t0)) = pilot.trace.as_mut() {
            use std::io::Write;
            let f = |b: bool| b as u8;
            let half = pilot.dist.half;
            let _ = writeln!(
                w,
                "{:.4},{:.3},{:.1},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.1},{:.1},{},{},{},{},{:.4}",
                (pilot.ticks - *t0) as f64 * TICK,
                from_sim_v(pilot.motion.vel).length(),
                pilot.heat.hud_percent(&hs),
                f(keys.throttle_up),
                f(keys.throttle_down),
                f(keys.throttle_zero),
                f(keys.boost),
                f(input.lateral < 0.0),
                f(input.lateral > 0.0),
                f(input.vertical > 0.0),
                f(input.vertical < 0.0),
                f(input.yaw > 0.0),
                f(input.yaw < 0.0),
                f(keys.flight_assist),
                f(keys.gear),
                f(keys.pip_eng),
                f(keys.pip_reset),
                f(keys.pip_sys),
                f(keys.pip_wep),
                keys.mouse.x,
                keys.mouse.y,
                half[SYS] as f32 / 2.0,
                half[ENG] as f32 / 2.0,
                half[WEP] as f32 / 2.0,
                f(!pilot.flight_assist),
                pilot.fuel.total()
            );
        }
        // The buildings and trees (ours, F): the ship's model box against their boxes, pushed out the shortest way, its speed
        // into them taken away; a hard enough hit damages it by the ground's rule.
        if !gliding && !town.1.is_empty() {
            let mid = ((assets.shape.lo + assets.shape.hi) * 0.5).as_dvec3();
            let half = ((assets.shape.hi - assets.shape.lo) * 0.5).as_dvec3();
            for bx in &town.1 {
                let ship_box = super::solids::Obb { c: p.pos + p.rot * mid, axes: [p.rot * DVec3::X, p.rot * DVec3::Y, p.rot * DVec3::Z], half };
                let Some((depth, n)) = super::solids::hit(&ship_box, bx) else { continue };
                p.pos += n * depth;
                let vel = from_sim_v(pilot.motion.vel);
                let into = -vel.dot(n);
                if into > 0.0 {
                    pilot.motion.vel = to_sim_v(vel + n * into);
                    let dmg = crash_damage(pilot.mass_t(), into);
                    if dmg > 0.0 {
                        let rest = pilot.shields.hit(dmg);
                        if pilot.hull.damage(rest) {
                            pilot.destroyed = true;
                        }
                    }
                    if into > 2.0 {
                        info!("sidewinder: hit a building or a tree at {into:.1} m/s: {dmg:.1} MJ, shields {:.0} %, hull {:.0} %", pilot.shields.percent(), pilot.hull.percent());
                    }
                }
            }
        }
        // The ground under the ship: its contact points' heights over the loaded ground (each leg's foot while the leg has its
        // body, the belly's always), each straight down the radial. Elite gives a leg its physics body from gear state 1 on
        // (FUN_14307f8a0), moving with it: the feet are where the legs' poses put them now (`Parts::feet`), so a ship coming
        // down on moving legs stands on them and rises as they extend. Touching down needs every leg deployed (the leg's own
        // test, state 2 or 4: 0x1430c94e0).
        let gear_out = gear.deployed();
        let height = |p: &super::Parked, pt: Vec3| over_ground(b, solid, floor_m, p.pos, p.rot, pt);
        let contact = parts.feet(&assets.shape);
        let feet: Vec<f64> = contact.iter().map(|f| height(p, *f)).collect();
        let lowest = assets.shape.belly.iter().map(|q| height(p, *q)).chain(feet.iter().copied()).fold(f64::MAX, f64::min);
        if lowest < 0.0 {
            if pilot.glide.take().is_some() {
                msg.show("Glide ended: the ground".to_string());
                info!("sidewinder: the glide met the ground");
            }
            if giant {
                // A gas giant's floor (F4): it stops the ship, which can't land there.
                let up = p.pos.normalize();
                let vel = from_sim_v(pilot.motion.vel);
                let down_speed = -vel.dot(up);
                p.pos -= up * lowest;
                if down_speed > 0.0 {
                    pilot.motion.vel = to_sim_v(vel + up * down_speed);
                    if pilot.log {
                        info!("sidewinder: stopped by the giant's floor at {down_speed:.1} m/s");
                    }
                }
                continue;
            }
            let up = p.pos.normalize();
            let vel = from_sim_v(pilot.motion.vel);
            let down_speed = -vel.dot(up);
            // Elite's touchdown (0x1430ECE00, A21, C): the ship's down within the landing record's 20 degrees of gravity's, at
            // most the landing component's 15 m/s (B), its landing points' rays agreeing within 0.5 m; that every leg must be
            // deployed is our reading (E: the exe's +0x200 state gate wasn't identified; "deployed" is the leg's own test).
            let tilt = (p.rot * DVec3::Y).dot(up).clamp(-1.0, 1.0).acos().to_degrees();
            let spread = feet.iter().copied().fold(f64::MIN, f64::max) - feet.iter().copied().fold(f64::MAX, f64::min);
            if let Some(why) = touchdown(gear_out && !feet.is_empty(), tilt, spread, vel.length()) {
                if pilot.log {
                    info!("sidewinder: on the ground, no touchdown: {why}");
                }
            } else {
                // Touchdown: level with the radial, the heading kept, standing on the highest ground under its feet.
                let fwd = p.rot * DVec3::NEG_Z;
                let flat = (fwd - up * fwd.dot(up)).normalize_or(up.any_orthonormal_vector());
                let right = flat.cross(up);
                p.rot = DQuat::from_mat3(&bevy::math::DMat3::from_cols(right, up, -flat)).normalize();
                let low = contact.iter().map(|f| height(p, *f)).fold(f64::MAX, f64::min);
                p.pos -= up * low;
                pilot.motion = Motion { rot: to_sim_q(p.rot), ..Motion::default() };
                pilot.landed = true;
                pilot.throttle = 0.0;
                msg.show("Touchdown".to_string());
                info!("sidewinder: touchdown at {:.1} m/s ({:.1} m/s down), {:.0} degrees of tilt, feet within {:.2} m", vel.length(), down_speed, tilt, spread);
                break;
            }
            // Stopped by the ground: out of it, and no speed into it; a hard enough impact damages it (step 2.3).
            let dmg = crash_damage(pilot.mass_t(), down_speed);
            if dmg > 0.0 {
                let rest = pilot.shields.hit(dmg);
                if pilot.hull.damage(rest) {
                    pilot.destroyed = true;
                }
                info!("sidewinder: hit the ground at {:.1} m/s: {:.1} MJ, shields {:.0} %, hull {:.0} %", down_speed, dmg, pilot.shields.percent(), pilot.hull.percent());
            }
            p.pos -= up * lowest;
            if down_speed > 0.0 {
                pilot.motion.vel = to_sim_v(vel + up * down_speed);
            }
        }
    }
    if n > 0 {
        pilot.record_tick(cockpit);
    }
    if pilot.accum > TICK {
        pilot.accum = 0.0;
    }
    pilot.dev_fire = (pilot.dev_fire - dt as f32).max(0.0);
    // The ship's pose in the universe as the ticks left it (the camera is written from it after this), and the HUD's mark of
    // the destination seen from there (A9), so the mark sits on the body as the cockpit sees it, turning or not.
    let (now_u, now_rot_u) = match pilot.cruise {
        Some(c) => (c.pos, c.rot),
        None => (b.pos.offset_m(b.orient * p.pos), (b.orient * p.rot).normalize()),
    };
    pilot.dest = dest_at.map(|(obj, at, radius, system)| dest_mark(pilot.dest.take(), obj, at.delta_m(now_u), radius, now_rot_u, system, || data.catalog.display_name(obj).to_string()));
    // The height over the loaded ground under the ship in the frame's body (in supercruise from the cruise's own pose, as the
    // parked pose is written below): the planet HUD's altitude, and the distance of a destination that is that body
    // (`own_body_distance`), so the marker, the DESTINATION line and ALT read the same number. (A star or a gas giant has no
    // solid terrain: its ground is its ellipsoid.)
    let ship_b = match pilot.cruise {
        Some(c) => b.orient.inverse() * c.pos.delta_m(b.pos),
        None => p.pos,
    };
    let alt = ship_b.length() - crate::ship::ground_on(b.radius_m, b.shape, solid, ship_b).0;
    let planet_hud = planet_hud_shows(alt, b.star, pilot.cruise.is_some());
    if let Some(m) = pilot.dest.as_mut().filter(|m| !m.system && m.obj == b.obj) {
        (m.dist_m, m.over_ground) = own_body_distance(alt, b.star, pilot.cruise.is_some());
    }
    pilot.nav_note = match &pilot.dest {
        Some(m) if !m.system => format!("{}  {}", m.name, m.distance_text()),
        _ => String::new(),
    };
    if jump_now {
        match (pilot.jump_fuel.take(), pilot.target) {
            (Some((t, worked_out)), Some(target)) if t == target => {
                let fuel = worked_out.min(pilot.fuel.main);
                pilot.fuel.main = (pilot.fuel.main - fuel).max(0.0);
                msg.show(format!("Hyperspace: {}", data.catalog.display_name(t)));
                info!("sidewinder: jump to {} for {fuel:.6} t of fuel", data.catalog.display_name(t));
                super::hyper::begin(&data, sim.jd, pilot, now_u, now_rot_u, b.obj, fuel, t);
                pilot.dest = None;
            }
            _ => pilot.drive = if pilot.cruise.is_some() { Drive::Supercruise } else { Drive::Ready },
        }
        return;
    }
    // The planet HUD's readings (step 4.3): near a solid body's ground (a giant's radius), out of supercruise.
    pilot.planet = None;
    pilot.safe_drop = None;
    // Supercruise's pose into the nearest body's frame (the parked pose is what the camera, the cockpit and the rest read);
    // a drop or an emergency drop back to normal flight there.
    if let Some(c) = pilot.cruise {
        let nearest = wells.iter().min_by(|a, b2| (c.pos.delta_m(a.1).length() - a.2).total_cmp(&(c.pos.delta_m(b2.1).length() - b2.2)));
        if let Some((_, at, _, obj, orient)) = nearest {
            p.obj = *obj;
            p.pos = orient.inverse() * c.pos.delta_m(*at);
            p.rot = (orient.inverse() * c.rot).normalize();
        }
        if let (Some(obj), false) = (glide_into, drop_now || emergency) {
            // Into the glide (A60, C): at 2,500 m/s along the nose, moved 7,500 m ahead, in the planet's frame.
            if let Some((_, at, _, _, orient)) = wells.iter().find(|w| w.3 == obj) {
                p.obj = obj;
                p.pos = orient.inverse() * c.pos.delta_m(*at);
                p.rot = (orient.inverse() * c.rot).normalize();
            }
            let nose = p.rot * DVec3::NEG_Z;
            let ahead = p.pos + nose * glide::HANDOVER_AHEAD_M;
            if height_over(obj, ahead).is_some_and(|h| h >= glide::END_BELOW_M) {
                p.pos = ahead;
                pilot.cruise = None;
                pilot.motion = Motion { rot: to_sim_q(p.rot), vel: to_sim_v(nose * glide::HANDOVER_SPEED), ..Motion::default() };
                pilot.glide = Some(Glide::default());
                pilot.drive.drop_out();
                msg.show("Glide".to_string());
                info!("sidewinder: glide into {obj:?} at pitch {:.0}, {:.0} m over the ground", glide::pitch_deg(nose.to_array(), (-p.pos.normalize_or(DVec3::Y)).to_array()), height_over(obj, p.pos).unwrap_or(0.0));
                return;
            }
            // The jump ahead would end under the glide's end height (high ground ahead): a plain drop here instead (F).
            msg.show("Glide refused: the ground ahead".to_string());
            info!("sidewinder: glide refused, the ground 7.5 km ahead");
            drop_now = true;
        }
        pilot.safe_drop = pilot.nav_body.and_then(|o| wells.iter().find(|w| w.3 == o)).map(|(_, at, r, _, _)| {
            let to = at.delta_m(c.pos);
            let angle = (c.rot * DVec3::NEG_Z).angle_between(to).to_degrees();
            supercruise::can_drop(to.length() - r, c.speed, angle)
        });
        if drop_now || emergency {
            pilot.cruise = None;
            let v = p.rot * DVec3::NEG_Z * c.speed.min(block.top_speed);
            pilot.motion = Motion { rot: to_sim_q(p.rot), vel: to_sim_v(v), ..Motion::default() };
            pilot.drive.drop_out();
            if emergency {
                let rest = pilot.shields.hit(EMERGENCY_HIT);
                if pilot.hull.damage(rest) {
                    pilot.destroyed = true;
                }
                msg.show(if too_fast_stop { "Emergency drop: too fast near the planet" } else { "Emergency drop" }.to_string());
                info!("sidewinder: emergency drop{} at {:.1} km/s by {:?}", if too_fast_stop { " (too fast near the planet)" } else { "" }, c.speed / 1000.0, p.obj);
            } else {
                msg.show("Supercruise exit".to_string());
                info!("sidewinder: dropped from supercruise at {:.0} km/s", c.speed / 1000.0);
            }
        }
        if pilot.log && n > 0 {
            info!("sidewinder: supercruise {:.0} m/s, the cap {:.0} m/s, throttle {:.2}, accel {:.4e} m/s^2, {:.0} km over {:?}'s radius", c.speed, c.cap, pilot.throttle, pilot.motion.last_acc.z, (c.pos.delta_m(b.pos).length() - b.radius_m) / 1000.0, b.obj);
        }
        return;
    }
    // Each beam runs along the nose from its gun to the ground or the lasers' range (a ray stepped against the loaded heights).
    // (The guns' mounts are the hull's hardpoint bones, `SidewinderAssets::guns`: the beam starts at the mount.)
    for (i, gun) in assets.guns.iter().enumerate() {
        let from = p.pos + p.rot * Vec3::from_array(*gun).as_dvec3();
        let dir = p.rot * DVec3::NEG_Z;
        let range = pilot.weapons.laser.range_m;
        let mut len = range;
        let mut s = 0.0;
        while s < range {
            let at = from + dir * s;
            let (ground, _) = crate::ship::ground_on(b.radius_m, b.shape, solid, at);
            if at.length() < ground {
                len = s;
                break;
            }
            s += 5.0_f64.max(s * 0.02);
        }
        pilot.beam_len[i] = if pilot.modules[super::panels::M_LASERS[i]] { len as f32 } else { 0.0 };
    }
    // (`alt` and `planet_hud` from above: the pose hasn't moved since, out of supercruise.)
    if planet_hud {
        let up = p.pos.normalize_or(DVec3::Y);
        let north = (DVec3::Y - up * up.y).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
        let east = north.cross(up);
        let nose = p.rot * DVec3::NEG_Z;
        let flat = nose - up * nose.dot(up);
        let right = p.rot * DVec3::X;
        pilot.planet = Some(PlanetHud {
            alt_m: alt,
            vspeed: from_sim_v(pilot.motion.vel).dot(up),
            heading_deg: flat.dot(east).atan2(flat.dot(north)).to_degrees().rem_euclid(360.0),
            pitch_deg: nose.dot(up).clamp(-1.0, 1.0).asin().to_degrees(),
            roll_deg: (-right.dot(up)).atan2((p.rot * DVec3::Y).dot(up)).to_degrees(),
        });
    }
    if pilot.log && n > 0 {
        let v = from_sim_v(pilot.motion.vel);
        let (ground, _) = crate::ship::ground_on(b.radius_m, b.shape, solid, p.pos);
        info!("sidewinder: {:.1} m/s, {:.3} m over the ground (its origin), throttle {:.2}, FA {}, gear {}, scoop {}, top {:.1} m/s, boost {}", v.length(), p.pos.length() - ground, pilot.throttle, pilot.flight_assist, pilot.gear_down, pilot.scoop, block.top_speed, pilot.boost.firing());
    }
}

/// The larger in size of two stick values (the pad's and the mouse's).
fn larger(a: f64, b: f64) -> f64 {
    if a.abs() >= b.abs() { a } else { b }
}

/// Elite's touchdown rule (0x1430ECE00, A21; its numbers the landing component's and the landing record's,
/// `elite_sim::landing`): None when the ship may touch down, else why not. `spread` is how far apart the landing points'
/// heights over the ground are. It is tested at contact, where the height over the ground is 0 (within P14's 100 m).
/// The gear's key (and the panel's row): the new command, or why it is refused. While landed the command can't change (ours,
/// F: what Elite does there was not traced; without it the legs rose under a landed ship and left its hull on nothing).
fn gear_toggle(landed: bool, down: bool) -> Result<bool, &'static str> {
    if landed {
        Err("The landing gear can't be retracted while landed")
    } else {
        Ok(!down)
    }
}

/// Whether a ship boarded where it stands is landed (the review of ARCHITECTURE.md A11): the touchdown's own test on the
/// pose as it is (`touchdown`: the gear out, the tilt, the feet's spread, the speed) with its feet on the ground, the highest
/// of them within the touchdown's spread of it (`feet`: each foot's height over the loaded ground, m). Anything else (hanging
/// in the air, tilted, on its side, still moving fast) flies from that pose.
fn stands_landed(gear_out: bool, tilt_deg: f64, feet: &[f64], speed: f64) -> bool {
    let (low, high) = feet.iter().fold((f64::MAX, f64::MIN), |(l, h), &f| (l.min(f), h.max(f)));
    !feet.is_empty() && high <= landing::TOUCHDOWN_SPREAD_M && touchdown(gear_out, tilt_deg, high - low, speed).is_none()
}

/// The thrust that holds the ship against the body's gravity, as the flight's jerk memory takes it (`Motion::last_acc`, ship
/// space, z back): a liftoff and a ship boarded in the air start their thrusters from it, or the jerk limit lets the ship
/// sink for the 0.4 s it takes to reach 1 g.
fn hover_acc(pos: DVec3, rot: DQuat, gm: f64) -> DVec3 {
    let r0 = pos.length().max(1.0);
    let g_ship = rot.inverse() * (-(pos / r0) * gm / (r0 * r0));
    -DVec3::new(g_ship.x, g_ship.y, -g_ship.z)
}

/// A point of the ship (ship space) over the ground under it, straight down the radial (m): the loaded ground of a solid body
/// (`solid`), a giant's floor (`floor_m`) otherwise.
pub(super) fn over_ground(b: &SystemBody, solid: Option<&Terrain>, floor_m: f64, pos: DVec3, rot: DQuat, pt: Vec3) -> f64 {
    let w = pos + rot * pt.as_dvec3();
    w.length() - (crate::ship::ground_on(b.radius_m, b.shape, solid, w).0 + floor_m)
}

fn touchdown(gear_out: bool, tilt_deg: f64, spread_m: f64, speed: f64) -> Option<&'static str> {
    if !gear_out {
        Some("the landing gear isn't down")
    } else if tilt_deg > landing::TOUCHDOWN_TILT_DEG {
        Some("tilted too far")
    } else if spread_m > landing::TOUCHDOWN_SPREAD_M {
        Some("the ground under the feet is too uneven")
    } else if landing::allowed(0.0, speed, tilt_deg) {
        None
    } else {
        Some("too fast")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::facts::ShipFacts;

    /// U3: the user's 0.2725 by default; full stick at 1,180 counts there and at `MOUSE_FULL_COUNTS` at Elite's preset 1.0;
    /// no sensitivity gives an infinite or NaN scale.
    #[test]
    fn mouse_sensitivity_sets_full_stick() {
        assert_eq!(MouseSetting::default().sensitivity, 0.2725);
        assert!((full_counts(0.2725) - 1180.0).abs() < 0.5, "{}", full_counts(0.2725));
        assert_eq!(full_counts(1.0), elite_sim::data::measured::MOUSE_FULL_COUNTS as f32);
        for s in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(full_counts(s).is_finite() && full_counts(s) > 0.0, "{s}: {}", full_counts(s));
        }
    }

    /// One destination: a body clears a star target and a star a body; the panel's pick of the locked one unlocks it, the
    /// dev commands' (no `again_unlocks`) keeps it; T finding no star (`unlock(true)`) clears only a star target.
    #[test]
    fn one_destination() {
        let (earth, moon, rigil) = (ObjectRef::Body(1), ObjectRef::Body(2), ObjectRef::Star(7));
        let mut p = Pilot::default();
        assert_eq!(p.lock(Dest::Body(earth), true), Some(Dest::Body(earth)));
        assert_eq!(p.lock(Dest::System(rigil), true), Some(Dest::System(rigil)));
        assert_eq!((p.nav_body, p.target), (None, Some(rigil)), "the star cleared the body");
        assert_eq!(p.lock(Dest::Body(moon), true), Some(Dest::Body(moon)));
        assert_eq!((p.nav_body, p.target), (Some(moon), None), "the body cleared the star");
        assert_eq!(p.lock(Dest::Body(moon), false), Some(Dest::Body(moon)), "a dev command sets, never unlocks");
        assert_eq!(p.lock(Dest::Body(moon), true), None, "the panel's pick of the locked one unlocks it");
        assert_eq!((p.nav_body, p.target), (None, None));
        p.lock(Dest::Body(earth), true);
        p.unlock(true);
        assert_eq!(p.locked(), Some(Dest::Body(earth)), "T finding no star keeps a locked body");
        p.lock(Dest::System(rigil), false);
        p.unlock(false);
        assert_eq!(p.locked(), Some(Dest::System(rigil)), "`sw dest none` keeps a star target");
        p.unlock(true);
        assert_eq!(p.locked(), None);
        // The destination is the ship's: getting out and boarding again keeps it; a new ship has none.
        for d in [Dest::Body(moon), Dest::System(rigil)] {
            p.lock(d, false);
            let mut again = p.boarding(DVec3::ZERO);
            assert_eq!(again.locked(), Some(d));
            again.fresh_ship();
            assert_eq!(again.locked(), None);
        }
    }

    /// The mark: the direction in the ship's axes (a ship turned 90 degrees left sees a body along the universe's -X dead
    /// ahead), the distance to the surface, the angular radius; a star of another system has none; the name is kept while
    /// the object is the same and made anew when it changes.
    #[test]
    fn the_destinations_mark() {
        let rot = DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2);
        let m = dest_mark(None, ObjectRef::Body(2), DVec3::new(-2.0e6, 0.0, 0.0), 1.0e6, rot, false, || "Moon".to_string());
        assert!((m.dir - Vec3::NEG_Z).length() < 1e-6, "{}", m.dir);
        assert_eq!((m.dist_m, m.name.as_str(), m.system), (1.0e6, "Moon", false));
        assert!((m.ang_radius - std::f32::consts::FRAC_PI_6).abs() < 1e-6);
        // To its right in the ship (the universe's -Z for this turn: the nose's right is the universe's -Z), name kept.
        let m = dest_mark(Some(m), ObjectRef::Body(2), DVec3::new(0.0, 0.0, -5.0e6), 1.0e6, rot, false, || panic!("the name is kept"));
        assert!((m.dir - Vec3::X).length() < 1e-6, "{}", m.dir);
        let s = dest_mark(Some(m), ObjectRef::Star(7), DVec3::new(0.0, 4.0e16, 0.0), 0.0, rot, true, || "Rigil Kentaurus".to_string());
        assert_eq!((s.dist_m, s.ang_radius, s.name.as_str(), s.system), (4.0e16, 0.0, "Rigil Kentaurus", true));
        assert!((s.dir - Vec3::Y).length() < 1e-6);
        // The distance's text: the panels' form, or the altimeter's while it is the height over the ground.
        let mut g = dest_mark(None, ObjectRef::Body(2), DVec3::new(0.0, -1.0e6 - 4732.0, 0.0), 1.0e6, DQuat::IDENTITY, false, String::new);
        assert_eq!(g.distance_text(), "5 KM");
        g.over_ground = true;
        assert_eq!(g.distance_text(), "4732 M");
        // The frame's own body: the height over the ground runs on through the planet HUD's height (the same metres just
        // under and just over it), only the text's form changing; in supercruise and over a star the ordinary form; never
        // under 0.
        let (under, over) = (own_body_distance(PLANET_HUD_M - 0.5, false, false), own_body_distance(PLANET_HUD_M + 0.5, false, false));
        assert_eq!((under.0, under.1, over.0, over.1), (PLANET_HUD_M - 0.5, true, PLANET_HUD_M + 0.5, false));
        assert_eq!(own_body_distance(5_000.0, false, true), (5_000.0, false));
        assert_eq!(own_body_distance(5_000.0, true, false), (5_000.0, false));
        assert_eq!(own_body_distance(-30_000.0, false, false), (0.0, true));
        for (d, form) in [under, over] {
            g.dist_m = d;
            g.over_ground = form;
            assert!(g.distance_text().starts_with("100"), "{}", g.distance_text());
        }
        // Inside the body's radius it fills half the sky.
        assert_eq!(dest_mark(None, ObjectRef::Body(2), DVec3::new(0.0, -5.0e5, 0.0), 1.0e6, DQuat::IDENTITY, false, String::new).ang_radius, std::f32::consts::FRAC_PI_2);
    }

    /// The cockpit point's change of velocity tick by tick: the ship's own change turned into its axes, plus the spin carried
    /// to the cockpit; nothing on a new flight's first tick, in supercruise or on the tick after it; the facts take a frame's
    /// list only in that frame.
    #[test]
    fn the_cockpits_change_of_velocity_tick_by_tick() {
        let cockpit = Vec3::new(0.0, 6.7, -1.3);
        let mut pilot = Pilot::default();
        pilot.tick_dv.start(7);
        // (The ship turned 90 degrees left about its up axis: its nose, -Z in its axes, points along -X of the rest frame.)
        pilot.motion.rot = glam::DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2);
        pilot.record_tick(cockpit);
        // 3 m/s more along the rest frame's -X: straight ahead of the ship, its -Z.
        pilot.motion.vel = glam::DVec3::new(-3.0, 0.0, 0.0);
        pilot.record_tick(cockpit);
        // A spin of 1 rad/s about the ship's up axis (to the left) moves the cockpit (1.3 m ahead of the origin) to the left.
        pilot.motion.omega = glam::DVec3::new(0.0, 1.0, 0.0);
        pilot.record_tick(cockpit);
        let ticks = pilot.tick_dv.as_slice().to_vec();
        assert_eq!(ticks[0], Vec3::ZERO, "the first tick has nothing to compare with");
        assert!((ticks[1] - Vec3::new(0.0, 0.0, -3.0)).length() < 1e-5, "{}", ticks[1]);
        assert!((ticks[2] - Vec3::new(-1.3, 0.0, 0.0)).length() < 1e-5, "{}", ticks[2]);
        // The facts take the list in its frame only.
        let keys = PilotKeys::default();
        assert_eq!(ShipFacts::of(&pilot, &keys, true, 7, &ShipFacts::default()).ticks.as_slice(), ticks.as_slice());
        assert!(ShipFacts::of(&pilot, &keys, true, 8, &ShipFacts::default()).ticks.as_slice().is_empty());
        // Supercruise's motion isn't the flight's: nothing then, nor on the tick after.
        pilot.tick_dv.start(8);
        pilot.motion.vel = glam::DVec3::new(-500.0, 0.0, 0.0);
        pilot.cruise = Some(Cruise { pos: Default::default(), rot: DQuat::IDENTITY, speed: 3e4, cap: 3e4, carrier: None });
        pilot.record_tick(cockpit);
        pilot.cruise = None;
        pilot.record_tick(cockpit);
        pilot.motion.vel = glam::DVec3::new(-501.0, 0.0, 0.0);
        pilot.record_tick(cockpit);
        let after: Vec<Vec3> = pilot.tick_dv.as_slice().to_vec();
        assert_eq!(&after[..2], &[Vec3::ZERO; 2]);
        assert!(after[2].length() > 0.5, "live again: {}", after[2]);
        // (A frame never holds more ticks than the flight runs.)
        for _ in 0..10 {
            pilot.record_tick(cockpit);
        }
        assert_eq!(pilot.tick_dv.as_slice().len(), MAX_TICKS);
    }

    /// A ship lost at speed, then a new one spawned (and one boarded): the new flight's first tick gives no change of
    /// velocity, so the chair takes no knock from the lost ship's speed; a flight that didn't run (the tunnel, not seated)
    /// forgets its last velocity too.
    #[test]
    fn a_new_ship_takes_no_knock_from_the_last() {
        let cockpit = Vec3::new(0.0, 6.7, -1.3);
        let mut pilot = Pilot::default();
        pilot.motion.vel = glam::DVec3::new(0.0, 0.0, -400.0);
        pilot.record_tick(cockpit);
        pilot.record_tick(cockpit);
        pilot.destroyed = true;
        pilot.fresh_ship();
        pilot.tick_dv.start(1);
        pilot.record_tick(cockpit);
        assert_eq!(pilot.tick_dv.as_slice(), &[Vec3::ZERO]);
        pilot.motion.vel = glam::DVec3::new(0.0, 0.0, -400.0);
        pilot.record_tick(cockpit);
        let mut boarded = pilot.boarding(DVec3::ZERO);
        boarded.tick_dv.start(2);
        boarded.record_tick(cockpit);
        assert_eq!(boarded.tick_dv.as_slice(), &[Vec3::ZERO]);
        pilot.forget_ticks();
        pilot.tick_dv.start(3);
        pilot.motion.vel = glam::DVec3::ZERO;
        pilot.record_tick(cockpit);
        assert_eq!(pilot.tick_dv.as_slice(), &[Vec3::ZERO]);
    }

    /// While landed the gear's command stays (down); flying, the key toggles it; boarding a parked ship gives the command
    /// down whatever it was.
    #[test]
    fn the_gear_is_refused_while_landed() {
        assert!(gear_toggle(true, true).is_err());
        assert_eq!((gear_toggle(false, true), gear_toggle(false, false)), (Ok(false), Ok(true)));
        let mut pilot = Pilot::default();
        pilot.gear_down = false;
        pilot.landed = false;
        let boarded = pilot.boarding(DVec3::ZERO);
        assert!(boarded.gear_down && boarded.landed && boarded.boarded == Some(DVec3::ZERO));
    }

    /// Boarding decides landed or flying by the touchdown's own test on the pose as it is (the review of A11): on its gear on
    /// flat ground it stands landed; hanging 6 m up, tilted past the touchdown's 20 degrees, lying on its side, its gear not
    /// out or still moving fast, it flies.
    #[test]
    fn a_boarded_ship_is_landed_only_standing_on_its_gear() {
        let on_gear = [0.01, -0.005, 0.02];
        assert!(stands_landed(true, 1.2, &on_gear, 0.0));
        assert!(!stands_landed(true, 1.2, &on_gear.map(|h| h + 6.0), 0.0), "6 m up");
        assert!(!stands_landed(true, 25.0, &on_gear, 0.0), "tilted past the touchdown's limit");
        // On its side: one foot on the ground, the others metres up, the hull's up 90 degrees off.
        assert!(!stands_landed(true, 90.0, &[0.0, 4.5, 9.0], 0.0), "on its side");
        assert!(!stands_landed(true, 9.0, &[0.0, 0.3, 0.9], 0.0), "the feet too uneven");
        assert!(!stands_landed(false, 1.2, &on_gear, 0.0), "the gear not out");
        assert!(!stands_landed(true, 1.2, &on_gear, 35.0), "tumbling at 35 m/s");
        assert!(!stands_landed(true, 1.2, &[], 0.0), "no feet");
    }

    /// Elite's touchdown rule on the game's numbers: gear down, tilt at most 20 degrees, the feet within 0.5 m of each other,
    /// at most 15 m/s (12 m/s lands, 17 doesn't).
    #[test]
    fn touchdown_is_elites_rule() {
        assert_eq!(touchdown(true, 0.0, 0.0, 1.0), None);
        assert_eq!(touchdown(true, 20.0, 0.5, 12.0), None);
        assert_eq!(touchdown(true, 0.0, 0.0, 15.0), None);
        assert_eq!(touchdown(false, 0.0, 0.0, 1.0), Some("the landing gear isn't down"));
        assert_eq!(touchdown(true, 20.5, 0.0, 1.0), Some("tilted too far"));
        assert_eq!(touchdown(true, 0.0, 0.51, 1.0), Some("the ground under the feet is too uneven"));
        assert_eq!(touchdown(true, 0.0, 0.0, 17.0), Some("too fast"));
        assert_eq!(larger(0.2, -0.5), -0.5);
        assert_eq!(larger(-0.7, 0.5), -0.7);
    }

    /// Each SpaceEngine body class's supercruise well (the decision of task N1b-2): stars the star rule, planets, moons and
    /// dwarf planets the planet rule (kind 2), asteroids, comets, spacecraft and barycentres none.
    #[test]
    fn well_kind_of_each_body_class() {
        let (r, m) = (1.0e6, 2.0e22);
        assert_eq!(well_of(Class::Star { barycenter: false }, r, || m), Well::Star { radius: r });
        for tag in [BodyTag::Planet, BodyTag::DwarfPlanet, BodyTag::Moon, BodyTag::DwarfMoon] {
            assert_eq!(well_of(Class::Body(tag), r, || m), Well::Planet { radius: r, mass: m }, "{tag:?}");
        }
        for class in [Class::Star { barycenter: true }, Class::Body(BodyTag::Asteroid), Class::Body(BodyTag::Comet), Class::Body(BodyTag::Spacecraft), Class::Other] {
            assert_eq!(well_of(class, r, || panic!("no mass for a body without a well")), Well::Other { radius: r }, "{class:?}");
        }
        // A body without a well doesn't hold the cap down: near a 5 km asteroid it is the star's alone.
        let sun = Well::Star { radius: 6.957e8 };
        let asteroid = well_of(Class::Body(BodyTag::Asteroid), 5000.0, || 1.0e15);
        let au = supercruise::AU;
        assert_eq!(supercruise::cap(&[(sun, 2.0 * au), (asteroid, 10_000.0)]), supercruise::cap(&[(sun, 2.0 * au)]));
    }

    /// The escape cone by the body's kind: 50 degrees from a planet-rule body, 10 from a star, none from a body without a well.
    #[test]
    fn escape_cone_by_body_kind() {
        let planet = well_of(Class::Body(BodyTag::Moon), 1.7e6, || 7.3e22);
        assert_eq!(escape_cone_deg(planet), 50.0);
        assert_eq!(escape_cone_deg(well_of(Class::Star { barycenter: false }, 7.0e8, || 0.0)), 10.0);
        assert_eq!(escape_cone_deg(well_of(Class::Body(BodyTag::Comet), 5000.0, || 0.0)), 180.0);
        // A complete charge 1.01 radii out, the nose 30 degrees off straight away: engages by a planet, holds by a star.
        let near = ShipState { radii: 1.01, escape_angle: 30.0, ..ShipState::default() };
        let mut d = Drive::Charging(elite_sim::drive::CHARGE_S);
        assert_eq!(d.tick_escaping(TICK, &near, escape_cone_deg(planet)), Ok(Engage::Supercruise));
        let mut d = Drive::Charging(elite_sim::drive::CHARGE_S);
        assert_eq!(d.tick_escaping(TICK, &near, 10.0), Ok(Engage::Nothing));
        let mut d = Drive::Charging(elite_sim::drive::CHARGE_S);
        assert_eq!(d.tick_escaping(TICK, &ShipState { escape_angle: 179.0, ..near }, escape_cone_deg(Well::Other { radius: 5000.0 })), Ok(Engage::Supercruise));
    }

    fn shipped(name: &str) -> se_core::elite_asset::Binds {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/binds").join(name);
        se_core::elite_asset::Binds::parse(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    /// Step 7.1: the shipped pad file gives the spec's pad layout, and every pad name in it has a Bevy button or axis.
    #[test]
    fn pad_table_is_the_specs() {
        let b = shipped("ControlPad.binds");
        let axis = |a: &str| b.get(a).and_then(|x| x.axis.as_ref()).map(|k| k.key.clone()).unwrap_or_default();
        let button = |a: &str| b.get(a).map(|x| x.keys().filter(|k| is_pad(&k.device)).map(|k| k.key.clone()).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!((axis("RollAxisRaw"), axis("PitchAxisRaw")), ("Pad_LStickX".into(), "Pad_LStickY".into()));
        assert_eq!((axis("LateralThrustRaw"), axis("VerticalThrustRaw")), ("Pad_RStickX".into(), "Pad_RStickY".into()));
        for (action, key) in [
            ("ForwardKey", "Pad_RBumper"),
            ("BackwardKey", "Pad_LBumper"),
            ("UseBoostJuice", "Pad_B"),
            ("PrimaryFire", "Pad_RTrigger"),
            ("SecondaryFire", "Pad_LTrigger"),
            ("DeployHardpointToggle", "Pad_Back"),
            ("IncreaseEnginesPower", "Pad_DPadUp"),
            ("IncreaseWeaponsPower", "Pad_DPadRight"),
            ("IncreaseSystemsPower", "Pad_DPadLeft"),
            ("ResetPowerDistribution", "Pad_DPadDown"),
            ("UIFocus", "Pad_X"),
            ("SelectTarget", "Pad_Y"),
            ("HeadLookToggle", "Pad_RThumb"),
        ] {
            assert_eq!(button(action), vec![key.to_string()], "{action}");
        }
        for a in b.actions.values() {
            for k in a.keys().chain(a.axis.iter()).filter(|k| k.device == "XB360 Pad") {
                assert!(pad_button(&k.key).is_some() || pad_axis(&k.key).is_some() || k.key.starts_with("Pos_") || k.key.starts_with("Neg_") || k.key.starts_with("GamePad_"), "{}", k.key);
            }
        }
    }

    /// Every keyboard key Elite's default file binds has a Bevy key code (or the seat couldn't take it from the others).
    #[test]
    fn keyboard_keys_all_known() {
        let b = shipped("KeyboardMouseOnly.binds");
        let missing: Vec<String> = b.actions.values().flat_map(|a| a.keys()).filter(|k| k.device == "Keyboard").filter(|k| super::super::seat::key_code(&k.key).is_none()).map(|k| k.key.clone()).collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    /// Elite's UI buttons: a press and a release are each one edge, a press and release within one frame both (the press
    /// first), a key held on or a second key of the same button nothing; the edges come out in `UI_BUTTONS`' order, UIFocus
    /// first.
    #[test]
    fn ui_buttons_give_their_edges() {
        assert_eq!(ui_edge(true, false, true), (true, false));
        assert_eq!(ui_edge(false, true, false), (false, true));
        assert_eq!(ui_edge(false, false, true), (true, true));
        assert_eq!((ui_edge(true, true, false), ui_edge(true, true, true), ui_edge(false, false, false)), ((false, false), (false, false), (false, false)));
        let at = |b: &str| UI_BUTTONS.iter().position(|n| *n == b).unwrap();
        let edges = UiEdges { down: 1 << at("UI_Left") | 1 << at("UIFocus") | 1 << at("UI_Select"), up: 1 << at("UI_Select"), held: 1 << at("UIFocus") };
        assert_eq!(edges.iter().collect::<Vec<_>>(), [("UIFocus", true), ("UI_Left", true), ("UI_Select", true), ("UI_Select", false)]);
        assert!(edges.is_held("UIFocus") && !edges.is_held("UI_Left"));
        assert!(UI_BUTTONS.len() <= 32, "one bit each");
        // A key pressed and released within one frame: a press and a release (it was not down before the frame).
        let b = shipped("KeyboardMouseOnly.binds");
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::Space);
        keys.release(KeyCode::Space);
        let tap = ui_edges(&b, &b, None, &keys, &ButtonInput::default());
        assert_eq!(tap.iter().collect::<Vec<_>>(), [("UI_Select", true), ("UI_Select", false)]);
        assert!(!tap.is_held("UI_Select"));
    }

    /// The modal and page sets are the shipped `ui/controls.json`'s members (the modal set's on-foot button left out).
    #[test]
    fn the_ui_sets_are_the_shipped_ones() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui/controls.json");
        let c: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        let names = |set: &str| c[set].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let modal: Vec<String> = names("bindings_interfacemodal").into_iter().filter(|n| !n.starts_with("Humanoid")).collect();
        assert_eq!(modal, MODAL_SET.map(str::to_string));
        assert_eq!(names("bindings_interfacemodal_pagination"), PAGINATION_SET.map(str::to_string));
    }

    /// While a set of the UI is live its keys fly nothing (the binds' own answer), and keys it does not use fly on: with a
    /// panel open W, Q, the up arrow and Delete are the UI's; Z (the pages' key) still toggles flight assist outside the comms
    /// panel's pages, and not within them; R (vertical thrust) always flies.
    #[test]
    fn the_uis_keys_fly_nothing_while_its_set_is_live() {
        use super::super::panels::UiSets;
        let b = shipped("KeyboardMouseOnly.binds");
        let read = |held: &[KeyCode], sets: UiSets| {
            let mut keys = ButtonInput::<KeyCode>::default();
            for &k in held {
                keys.press(k);
            }
            read_keys(&b, &b, None, &keys, &ButtonInput::default(), &AccumulatedMouseMotion::default(), false, sets)
        };
        let all = [KeyCode::KeyW, KeyCode::KeyQ, KeyCode::ArrowUp, KeyCode::Delete, KeyCode::KeyZ, KeyCode::KeyR];
        let flying = read(&all, UiSets::default());
        assert!(flying.throttle_up && flying.lateral < 0.0 && flying.pip_eng && flying.silent && flying.flight_assist && flying.vertical > 0.0);
        let panel = read(&all, UiSets { modal: true, pages: false, screen: false });
        assert!(!panel.throttle_up && panel.lateral == 0.0 && !panel.pip_eng && !panel.silent, "the modal set's keys");
        assert!(panel.flight_assist && panel.vertical > 0.0, "keys the panel does not use");
        assert!(panel.panel.up && panel.ui.is_held("UI_Up"), "the UI still sees its keys");
        let comms = read(&all, UiSets { modal: true, pages: true, screen: false });
        assert!(!comms.flight_assist && comms.vertical > 0.0, "the pages' Z");
    }
}
