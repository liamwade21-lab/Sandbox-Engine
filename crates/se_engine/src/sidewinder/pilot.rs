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
    /// tab (a body of this system) with its HUD line.
    pub modules: [bool; 10],
    pub nav_body: Option<ObjectRef>,
    pub nav_note: String,
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
    /// The mouse's virtual stick (x roll, y pitch), -1..1: it stays where the mouse left it (Elite's default: no decay).
    pub stick: Vec2,
    accum: f64,
    pub ticks: u64,
    /// A test's controls held for some seconds (`sw stick`, `sw thrust`), and a test's boost press (`sw boost`).
    pub dev: Option<(Input, f32)>,
    pub dev_boost: bool,
    /// A test's trigger held for some seconds (`sw fire SECS`), and a test's press of the drive's key (`sw jump`).
    pub dev_fire: f32,
    pub dev_jump: bool,
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
            stick: Vec2::ZERO,
            accum: 0.0,
            ticks: 0,
            dev: None,
            dev_boost: false,
            dev_fire: 0.0,
            dev_jump: false,
            log: false,
            controls: Input::default(),
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

    /// The HUD's heat percentage.
    pub fn heat_percent(&self) -> f64 {
        self.heat.hud_percent(&self.heat_spec)
    }

    /// Sitting down in the parked ship: the flight starts afresh, the ship's systems (fuel, shields, hull, heat, pips and
    /// capacitors) are kept.
    pub fn boarding(&self) -> Pilot {
        // (A parked ship is landed on its gear: its command is down, whatever it was when he left.)
        Pilot { fuel: self.fuel, shields: self.shields, hull: self.hull, heat: self.heat.clone(), dist: self.dist.clone(), modules: self.modules, life: self.life, gear_down: true, landed: true, ..Pilot::default() }
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

#[allow(clippy::too_many_arguments)]
pub fn read_keys(b: &se_core::elite_asset::Binds, pad_binds: &se_core::elite_asset::Binds, pad: Option<&Gamepad>, keys: &ButtonInput<KeyCode>, buttons: &ButtonInput<MouseButton>, motion: &AccumulatedMouseMotion, real: bool, panel_open: bool) -> PilotKeys {
    let mouse_held = |action: &str| b.get(action).is_some_and(|a| a.keys().filter(|k| k.device == "Mouse").filter_map(|k| mouse_button(&k.key)).any(|m| buttons.pressed(m)));
    let pad_btn = |action: &str, just: bool| -> bool {
        let (Some(g), Some(a)) = (pad, pad_binds.get(action)) else { return false };
        a.keys().filter(|k| is_pad(&k.device)).filter_map(|k| pad_button(&k.key)).any(|bt| if just { g.just_pressed(bt) } else { g.pressed(bt) })
    };
    let held = |a: &str| action_pressed(b, keys, a, false) || pad_btn(a, false);
    let press = |a: &str| action_pressed(b, keys, a, true) || pad_btn(a, true);
    let axis = |plus: &str, minus: &str| held(plus) as i8 as f64 - held(minus) as i8 as f64;
    let pv = |action: &str| pad.map_or(0.0, |g| pad_value(pad_binds, g, action));
    let panel = super::panels::PanelKeys {
        left: press("FocusLeftPanel"),
        right: press("FocusRightPanel"),
        up: press("UI_Up"),
        down: press("UI_Down"),
        change_left: press("UI_Left"),
        change_right: press("UI_Right"),
        select: press("UI_Select"),
        back: press("UI_Back"),
        next_tab: press("CycleNextPanel"),
        prev_tab: press("CyclePreviousPanel"),
    };
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
    };
    // A focused panel takes its keys from the flight, as in Elite (W S A D Q E and the arrows); the mouse still flies.
    if panel_open {
        PilotKeys { throttle_up: false, throttle_down: false, yaw: 0.0, lateral: 0.0, pip_eng: false, pip_sys: false, pip_wep: false, pip_reset: false, boost: k.boost && !k.panel.back, ..k }
    } else {
        k
    }
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
    (mut ask, probe, lib, mut town, parts): (ResMut<crate::scatter::probe::ProbeAsk>, Res<crate::scatter::probe::BuildingProbe>, Option<Res<crate::scatter::library::ScatterLibrary>>, Local<(Option<u32>, Vec<super::solids::Obb>)>, Res<super::parts::Parts>),
) {
    // The gear's real state, as the gear machines left it (`parts::update`, last frame).
    let gear = parts.gear();
    if pilot.put {
        pilot.put = false;
    }
    let (Some(assets), true) = (assets, seat.seated && control.pilot && sb.mode == Mode::Pilot) else { return };
    // In the tunnel the old system's bodies go: `hyper::tunnel` holds the ship and the camera until the new one is entered.
    life_support(&mut pilot, time.delta_secs_f64(), &mut msg);
    if pilot.jump.is_some() {
        return;
    }
    let Some(p) = ship.parked.as_mut() else { return };
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
    let Some((b, terrain)) = bodies.iter().find(|(b, _)| b.obj == p.obj) else { return };
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
    // The side panels (step 4.2): their keys and head look; what they ask for goes in as the keys would.
    panels.update(&keys.panel, seat.head_look.then_some(seat.look.x));
    let mut keys = *keys;
    for a in std::mem::take(&mut panels.actions) {
        use super::panels::{PanelAction, MODULES, M_FSD};
        match a {
            PanelAction::Target(t) => {
                pilot.target = Some(t);
                msg.show(format!("Target: {}", data.catalog.display_name(t)));
            }
            PanelAction::NavBody(o) => {
                pilot.nav_body = if pilot.nav_body == Some(o) { None } else { Some(o) };
                msg.show(match pilot.nav_body {
                    Some(o) => format!("Destination: {}", data.catalog.display_name(o)),
                    None => "Destination cleared".to_string(),
                });
            }
            PanelAction::FlightAssist => keys.flight_assist = true,
            PanelAction::Gear => keys.gear = true,
            PanelAction::Hardpoints => keys.hardpoints = true,
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
    // The mouse's virtual stick: counts move it, it stays (no decay in Elite's default file); its dead zone is the file's.
    let setting = |name: &str, or: f32| assets.keys.value(name).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(or);
    let full = elite_sim::data::measured::MOUSE_FULL_COUNTS as f32 / setting("MouseSensitivity", 1.0).max(0.01);
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
    // The gear down or the cargo scoop out halves the top and boost speeds (`elite_sim::deployables`, C; the record's 0.5, B).
    let speed_k = elite_sim::deployables::speed_factor(pilot.gear_down, pilot.scoop);
    let (block, full_top) = elite_sim::data::sidewinder_block_with(pilot.dist.pips(ENG), pilot.mass_t(), speed_k);
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
        if let Some(t) = pilot.target.or(pilot.nav_body) {
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
    // Hyperspace's target (step 5.3): T takes the star nearest the nose within the jump's range (ours: Elite targets
    // systems from its Navigation panel and galaxy map, step 4.2), or clears it; then its angle from the nose, or why it can't
    // be jumped to.
    let here = data.catalog.system_root(b.obj);
    if keys.select_target {
        let range = elite_sim::fsd::jump_range(&elite_sim::data::FSD_2E, pilot.mass_t(), pilot.fuel.main);
        pilot.target = super::hyper::star_ahead(&data, sim.jd, ship_u, ship_rot_u * DVec3::NEG_Z, range, here);
        msg.show(match pilot.target {
            Some(t) => format!("Target: {}", data.catalog.display_name(t)),
            None => "No target".to_string(),
        });
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
    pilot.nav_note = match pilot.nav_body.and_then(|o| bodies.iter().find(|(sb, _)| sb.obj == o)) {
        Some((sb, _)) => format!("{}  {}", data.catalog.display_name(sb.obj), super::panels::distance_text((sb.pos.delta_m(ship_u).length() - sb.radius_m).max(0.0))),
        None => String::new(),
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
    while pilot.accum >= TICK && n < flight::MAX_SUBSTEPS {
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
                let r0 = p.pos.length().max(1.0);
                let g_ship = p.rot.inverse() * (-(p.pos / r0) * gm / (r0 * r0));
                let hover = -DVec3::new(g_ship.x, g_ship.y, -g_ship.z);
                pilot.motion = Motion { rot: to_sim_q(p.rot), last_acc: to_sim_v(hover), ..Motion::default() };
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
        let height = |p: &super::Parked, pt: Vec3| -> f64 {
            let w = p.pos + p.rot * pt.as_dvec3();
            w.length() - (crate::ship::ground_on(b.radius_m, b.shape, solid, w).0 + floor_m)
        };
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
    if pilot.accum > TICK {
        pilot.accum = 0.0;
    }
    pilot.dev_fire = (pilot.dev_fire - dt as f32).max(0.0);
    if jump_now {
        let from = match pilot.cruise {
            Some(c) => c.pos,
            None => b.pos.offset_m(b.orient * p.pos),
        };
        match (pilot.jump_fuel.take(), pilot.target) {
            (Some((t, worked_out)), Some(target)) if t == target => {
                let fuel = worked_out.min(pilot.fuel.main);
                pilot.fuel.main = (pilot.fuel.main - fuel).max(0.0);
                msg.show(format!("Hyperspace: {}", data.catalog.display_name(t)));
                info!("sidewinder: jump to {} for {fuel:.6} t of fuel", data.catalog.display_name(t));
                let rot_u = pilot.cruise.map_or((b.orient * p.rot).normalize(), |c| c.rot);
                super::hyper::begin(&data, sim.jd, pilot, from, rot_u, b.obj, fuel, t);
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
    let alt = p.pos.length() - crate::ship::ground_on(b.radius_m, b.shape, solid, p.pos).0;
    if !b.star && alt < PLANET_HUD_M {
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

    /// While landed the gear's command stays (down); flying, the key toggles it; boarding a parked ship gives the command
    /// down whatever it was.
    #[test]
    fn the_gear_is_refused_while_landed() {
        assert!(gear_toggle(true, true).is_err());
        assert_eq!((gear_toggle(false, true), gear_toggle(false, false)), (Ok(false), Ok(true)));
        let mut pilot = Pilot::default();
        pilot.gear_down = false;
        pilot.landed = false;
        let boarded = pilot.boarding();
        assert!(boarded.gear_down && boarded.landed);
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
}
