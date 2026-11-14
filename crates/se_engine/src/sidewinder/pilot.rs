//! Flying the Sidewinder in normal space (step 1.3): `elite_sim::flight` stepped at a fixed tick from Elite's keys, the ship
//! moving in its body's turning frame (as Elite's ships near a planet hover over a spot), the body's gravity as the outside
//! pull the flight assist holds against, the ground under it from the terrain's loaded heights, a plain touchdown with the
//! gear down by Elite's touchdown rule; the power distributor (pips, capacitors) and boost (step 2.1, `elite_sim::power`,
//! `elite_sim::boost`).
//!
//! - The tick is 60 Hz (the spec's Q8: Elite's own rate was not found in its exe), at most 6 a frame, the rest dropped (the
//!   spec's edge cases), on real seconds (the game clock runs at real time while seated).
//! - Not yet: building contact and damage (step 2.3), liftoff rules and slopes (2.4), heat (2.2). The ship's own numbers
//!   are partly stand-ins (`elite_sim::data`, `power::stand_in`, `boost::stand_in`), until measured.

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
use elite_sim::supercruise::{self, Well};
use elite_sim::weapons::Weapons;
use se_core::catalog::ObjectRef;
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

/// The simulation's tick (the spec's Q8 fallback) and the most ticks a frame.
pub const TICK: f64 = 1.0 / 60.0;
const MOST_TICKS: u32 = 6;
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

/// Supercruise starts at its floor (the community's 30 km/s, `supercruise::stand_in`).
const CRUISE_START: f64 = supercruise::stand_in::FLOOR_PLANET;
/// Too close to a body's surface in supercruise: an emergency drop (E: A50 found the drop's effects but not its triggers),
/// within this share of its radius or this many metres, whichever is more; and the shields' hit it costs (E).
const EMERGENCY_SHARE: f64 = 0.02;
const EMERGENCY_M: f64 = 2000.0;
const EMERGENCY_HIT: f64 = 40.0;

/// The guns' places in model space (m): under the nose either side of the middle (E: Elite's hardpoints are on its in-game
/// hull, which is not among the open files; placed on the stand-in hull until the hull is recreated).
pub const GUNS: [[f32; 3]; 2] = [[-1.5, 2.0, -6.0], [1.5, 2.0, -6.0]];

/// A crash's damage (MJ, through the shield then the hull) from the speed into the ground: Elite's collision rule was not
/// studied (A21's list), so this is a stand-in (E): nothing under `CRASH_FREE_MPS`, then half the mass (t) times the excess
/// speed squared over 1,000 (a 50 m/s belly landing breaks the stock shield; at about 100 m/s the ship is lost).
const CRASH_FREE_MPS: f64 = 10.0;
fn crash_damage(mass_t: f64, speed: f64) -> f64 {
    0.5 * mass_t * (speed - CRASH_FREE_MPS).max(0.0).powi(2) / 1000.0
}

/// Touching the ground with the gear down slower than this lands it; faster it is stopped there (the landing component's
/// speed limit, P10 in A21's notes, is data: a stand-in until step 2.4 measures it).
const TOUCHDOWN_MPS: f64 = 10.0;
/// Elite's touchdown rule (0x1430ECE00, A21, grade C): the ship's down within this many degrees of gravity's (the landing
/// record's +0x38; 60 without a record: the record is data, so 60 until measured) and its landing points' rays agreeing
/// within this many metres.
const TOUCHDOWN_TILT_DEG: f64 = 60.0;
const TOUCHDOWN_SPREAD_M: f64 = 0.5;

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
    /// A test's placing of the ship over a body (`sw put NAME ALT_KM`), and the air's density where it is (kg/m^3).
    pub dev_put: Option<(ObjectRef, f64)>,
    pub air_density: f64,
    /// The modules' switches (the right panel's Modules tab, `panels::MODULES`), and the destination picked on the Navigation
    /// tab (a body of this system) with its HUD line.
    pub modules: [bool; 10],
    pub nav_body: Option<ObjectRef>,
    pub nav_note: String,
    pub planet: Option<PlanetHud>,
    /// The HUD's line for the target (its name, distance and the jump's fuel, or why not).
    pub target_note: String,
    /// The stars' heat this frame (a share of the capacity a second), for the HUD.
    pub star_heat: f64,
    /// A test's turn to face the target (`sw face`).
    pub dev_face: bool,
    pub gear_down: bool,
    /// The gear's animation, 0 down .. 1 up.
    pub gear_t: f32,
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
            dev_put: None,
            air_density: 0.0,
            modules: [true; 10],
            nav_body: None,
            nav_note: String::new(),
            planet: None,
            target_note: String::new(),
            star_heat: 0.0,
            dev_face: false,
            gear_down: true,
            gear_t: 0.0,
            landed: true,
            stick: Vec2::ZERO,
            accum: 0.0,
            ticks: 0,
            dev: None,
            dev_boost: false,
            dev_fire: 0.0,
            dev_jump: false,
            log: false,
        }
    }
}

impl Pilot {
    /// The HUD's heat percentage.
    pub fn heat_percent(&self) -> f64 {
        self.heat.hud_percent(&self.heat_spec)
    }

    /// Sitting down in the parked ship: the flight starts afresh, the ship's systems (fuel, shields, hull, heat, pips and
    /// capacitors) are kept.
    pub fn boarding(&self) -> Pilot {
        Pilot { fuel: self.fuel, shields: self.shields, hull: self.hull, heat: self.heat.clone(), dist: self.dist.clone(), ..Pilot::default() }
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
    };
    // A focused panel takes its keys from the flight, as in Elite (W S A D Q E and the arrows); the mouse still flies.
    if panel_open {
        PilotKeys { throttle_up: false, throttle_down: false, yaw: 0.0, lateral: 0.0, pip_eng: false, pip_sys: false, pip_wep: false, pip_reset: false, ..k }
    } else {
        k
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
) {
    let (Some(assets), true) = (assets, seat.seated && control.pilot && sb.mode == Mode::Pilot) else { return };
    // In the tunnel the old system's bodies go: `hyper::tunnel` holds the ship and the camera until the new one is entered.
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
            pilot.gear_t = 1.0;
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
        let parts = super::rest_transforms(&assets.ship);
        for d in &assets.ship.draws {
            let t = parts[d.part as usize];
            let at = |i: u16| glam::DVec3::from_array(t.transform_point3(Vec3::from_array(assets.ship.position_m(&d.vertices[i as usize]))).as_dvec3().to_array());
            for tri in d.indices.chunks_exact(3) {
                air.areas.push(elite_sim::air::area_vector(at(tri[0]), at(tri[1]), at(tri[2])));
            }
        }
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
    if keys.gear {
        pilot.gear_down = !pilot.gear_down;
        msg.show(if pilot.gear_down { "Landing gear deployed" } else { "Landing gear retracted" }.to_string());
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
    // The gear's animation (3 s, the converter's `gear_up`).
    let gear_s = assets.ship.animations.iter().find(|a| a.name == "gear_up").map_or(3.0, |a| a.seconds).max(0.1);
    let towards = if pilot.gear_down { 0.0 } else { 1.0 };
    pilot.gear_t += (towards - pilot.gear_t).clamp(-(dt as f32) / gear_s, dt as f32 / gear_s);

    let gm = se_core::units::G * crate::ship::body_mass(&data, b.obj, b.radius_m);
    let (block, full_top) = elite_sim::data::sidewinder_block(pilot.dist.pips(ENG), pilot.mass_t());
    let spec = BoostSpec::sidewinder(block.boost_ramp);
    pilot.full_top = full_top;
    pilot.top_share = block.top_speed / full_top.max(1e-9);
    pilot.blue_zone = blue_band(&block.curve_turn);
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
    // Hyperspace's target (step 5.3): T takes the star nearest the nose within the jump's range (ours: Elite targets
    // systems from its Navigation panel and galaxy map, step 4.2), or clears it; then its angle from the nose, or why it can't
    // be jumped to.
    if std::mem::take(&mut pilot.dev_face) {
        if let Some(t) = pilot.target {
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
        gear_down: pilot.gear_down || pilot.gear_t < 0.999,
        landed: pilot.landed,
        escape_angle: nose_body.dot(p.pos.normalize_or(DVec3::Y)).clamp(-1.0, 1.0).acos().to_degrees(),
        radii: p.pos.length() / b.radius_m.max(1.0),
        cruise: pilot.cruise.is_some(),
        target: target_state.map(|s| s.map(|(angle, _)| angle)),
    };
    let mut drop_now = false;
    let jump = keys.jump || std::mem::take(&mut pilot.dev_jump);
    if jump && !pilot.modules[super::panels::M_FSD] {
        msg.show("Frame shift drive: switched off".to_string());
    } else if jump && pilot.glide.is_some() {
        msg.show("Frame shift drive: not during a glide".to_string());
    } else if jump {
        match pilot.drive.press(&fsd) {
            Ok(true) => drop_now = true,
            Ok(false) => msg.show(match pilot.drive {
                Drive::Charging(_) => "Frame shift drive charging".to_string(),
                Drive::JumpCharging(_) => format!("Frame shift drive charging: {}", pilot.target.map_or("", |t| data.catalog.display_name(t))),
                _ => "Frame shift drive charge cancelled".to_string(),
            }),
            Err(r) => msg.show(r.message().to_string()),
        }
    }
    // The bodies' wells this frame (their places move slowly against a tick).
    let wells: Vec<(Well, UPos, f64, ObjectRef, DQuat)> = if pilot.cruise.is_some() || matches!(pilot.drive, Drive::Charging(_)) {
        bodies
            .iter()
            .map(|(sb, _)| {
                let w = if sb.star { Well::Star { radius: sb.radius_m } } else { Well::Planet { radius: sb.radius_m, mass: crate::ship::body_mass(&data, sb.obj, sb.radius_m) } };
                (w, sb.pos, sb.radius_m, sb.obj, sb.orient)
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut emergency = false;
    let mut jump_now = false;
    let mut glide_into: Option<ObjectRef> = None;
    // The bodies a glide can go into (solid ground: a terrain, not a star or a gas giant) and a point's height over their loaded
    // ground (the point in the body's frame).
    // (A gas giant's glide goes by its radius: F4, ours. It has no exclusion zone either.)
    let glidable: Vec<(ObjectRef, f64, DVec3, Option<&Terrain>)> = if pilot.cruise.is_some() {
        bodies.iter().filter_map(|(sb, t)| t.filter(|_| !sb.star).map(|t| (sb.obj, sb.radius_m, sb.shape, (!t.proc.kind.is_giant()).then_some(t)))).collect()
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
    while pilot.accum >= TICK && n < MOST_TICKS {
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
        pilot.boost.tick(&spec, TICK);
        let sys_share = pilot.dist.share(SYS);
        pilot.shields.tick(TICK, powered && pilot.modules[super::panels::M_SHIELDS], sys_share);
        // Heat: the idle load (a stand-in fitted to session 2's idle), then the radiator (step 2.2).
        let hs = pilot.heat_spec;
        if powered {
            pilot.heat.add(&hs, elite_sim::heat::stand_in::IDLE_LOAD * TICK, None);
        }
        let ev = pilot.heat.tick(&hs, TICK, powered, false);
        // The lasers (step 2.6): both guns of the fire group, WEP's energy and heat a shot.
        let guns_on = super::panels::M_LASERS.iter().any(|&m| pilot.modules[m]);
        let trigger = (keys.fire || pilot.dev_fire > 0.0) && powered && guns_on && pilot.cruise.is_none() && pilot.glide.is_none();
        pilot.weapons.tick(TICK, trigger, &mut pilot.dist, &mut pilot.heat, &hs);
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
        // The countdown's heat (A50: data, a stand-in).
        if matches!(pilot.drive, Drive::Departing(_)) {
            pilot.heat.add(&hs, elite_sim::hyperspace::stand_in::DEPARTING_HEAT_RATE * hs.capacity * TICK, None);
        }
        match pilot.drive.tick(TICK, &fsd) {
            Ok(Engage::Jump) => {
                jump_now = true;
                break;
            }
            Ok(Engage::Supercruise) => {
                pilot.cruise = Some(Cruise { pos: b.pos.offset_m(b.orient * p.pos), rot: (b.orient * p.rot).normalize(), speed: CRUISE_START, cap: CRUISE_START, carrier: Some((b.obj, b.pos)) });
                msg.show("Supercruise".to_string());
                info!("sidewinder: supercruise");
            }
            Err(r) => msg.show(r.message().to_string()),
            Ok(Engage::Nothing) => {}
        }
        if let Some(c) = pilot.cruise.as_mut() {
            // Supercruise: the turn by the normal block with the speed curve read at the throttle (the supercruise block's
            // turn rates are data, not measured: E; "best turning at half throttle" is the spec's hint), the speed towards the
            // throttle's share of the wells' cap (A50), along the nose; no gravity, no ground.
            let throttle = pilot.throttle.max(0.0);
            let mut m = Motion { rot: to_sim_q(c.rot), omega: pilot.motion.omega, last_alpha: pilot.motion.last_alpha, ..Motion::default() };
            m.vel = m.rot * glam::DVec3::new(0.0, 0.0, -throttle * block.top_speed);
            let rin = Input { pitch: input.pitch, yaw: input.yaw, roll: input.roll, flight_assist: true, ..Input::default() };
            flight::step(&block, full_top, false, &rin, glam::DVec3::ZERO, &mut m, TICK);
            c.rot = from_sim_q(m.rot).normalize();
            pilot.motion.omega = m.omega;
            pilot.motion.last_alpha = m.last_alpha;
            let near: Vec<(Well, f64)> = wells.iter().map(|(w, at, _, _, _)| (*w, c.pos.delta_m(*at).length())).collect();
            let cap = supercruise::cap(&near);
            c.cap = cap;
            c.speed = supercruise::step_speed(c.speed, throttle, cap, TICK);
            c.pos = c.pos.offset_m(c.rot * DVec3::NEG_Z * c.speed * TICK);
            // (Only heading into it: leaving a body from its surface is how supercruise starts.) A body with solid ground:
            // heading down within the glide's entry height over its loaded ground (the spec's hint), the glide (step 6.2); a
            // star or a gas giant: within its zone, an emergency drop.
            let heading = c.rot * DVec3::NEG_Z;
            if let Some((_, _, _, obj, _)) = wells.iter().find(|(_, at, _, obj, orient)| {
                heading.dot(at.delta_m(c.pos)) > 0.0 && height_over(*obj, orient.inverse() * c.pos.delta_m(*at)).is_some_and(|h| h < glide::stand_in::ENTRY_M)
            }) {
                glide_into = Some(*obj);
                break;
            }
            // (A star's zone is its exclusion zone, 1.05 of its radius: step 5.4.)
            if wells.iter().any(|(w, at, r, obj, _)| {
                let to = at.delta_m(c.pos);
                let inside = match w {
                    Well::Star { .. } => elite_sim::stars::excluded(to.length(), *r),
                    _ => !glidable.iter().any(|g| g.0 == *obj) && to.length() - r < (r * EMERGENCY_SHARE).max(EMERGENCY_M),
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
                from_sim_v(elite_sim::air::drag_acceleration(to_sim_v(vel), density, area, pilot.mass_t() * 1000.0))
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
        // The ground under the ship: its contact points' heights over the loaded ground (the gear's feet while it is down,
        // the belly's always), each straight down the radial.
        let gear_out = pilot.gear_t < 0.05;
        let height = |p: &super::Parked, pt: Vec3| -> f64 {
            let w = p.pos + p.rot * pt.as_dvec3();
            w.length() - (crate::ship::ground_on(b.radius_m, b.shape, solid, w).0 + floor_m)
        };
        let feet: Vec<f64> = if gear_out { assets.shape.feet.iter().map(|f| height(p, *f)).collect() } else { Vec::new() };
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
            // Elite's touchdown (0x1430ECE00, A21): the ship's down within 60 degrees of gravity's, its landing points' rays
            // agreeing within 0.5 m; the speed's limit is the landing component's data (a stand-in here).
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
                let low = assets.shape.feet.iter().map(|f| height(p, *f)).fold(f64::MAX, f64::min);
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
        match (pilot.target, target_state) {
            (Some(t), Some(Ok((_, fuel)))) => {
                pilot.fuel.main = (pilot.fuel.main - fuel).max(0.0);
                msg.show(format!("Hyperspace: {}", data.catalog.display_name(t)));
                info!("sidewinder: jump to {} for {fuel:.6} t of fuel", data.catalog.display_name(t));
                super::hyper::begin(&data, sim.jd, pilot, from, t);
            }
            _ => pilot.drive = if pilot.cruise.is_some() { Drive::Supercruise } else { Drive::Ready },
        }
        return;
    }
    // The planet HUD's readings (step 4.3): near a solid body's ground (a giant's radius), out of supercruise.
    pilot.planet = None;
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
                msg.show("Emergency drop".to_string());
                info!("sidewinder: emergency drop");
            } else {
                msg.show("Supercruise exit".to_string());
                info!("sidewinder: dropped from supercruise at {:.0} km/s", c.speed / 1000.0);
            }
        }
        return;
    }
    // Each beam runs along the nose from its gun to the ground or the lasers' range (a ray stepped against the loaded heights).
    for (i, gun) in GUNS.iter().enumerate() {
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
            roll_deg: (-right.dot(up)).clamp(-1.0, 1.0).asin().to_degrees(),
        });
    }
    if pilot.log && n > 0 {
        let v = from_sim_v(pilot.motion.vel);
        let (ground, _) = crate::ship::ground_on(b.radius_m, b.shape, solid, p.pos);
        info!("sidewinder: {:.1} m/s, {:.1} m over the ground (its origin), throttle {:.2}, FA {}, gear {}", v.length(), p.pos.length() - ground, pilot.throttle, pilot.flight_assist, pilot.gear_down);
    }
}

/// The larger in size of two stick values (the pad's and the mouse's).
fn larger(a: f64, b: f64) -> f64 {
    if a.abs() >= b.abs() { a } else { b }
}

/// Elite's touchdown rule (0x1430ECE00, A21): None when the ship may touch down, else why not. `spread` is how far apart the
/// landing points' heights over the ground are.
fn touchdown(gear_out: bool, tilt_deg: f64, spread_m: f64, speed: f64) -> Option<&'static str> {
    if !gear_out {
        Some("the landing gear isn't down")
    } else if tilt_deg > TOUCHDOWN_TILT_DEG {
        Some("tilted too far")
    } else if spread_m > TOUCHDOWN_SPREAD_M {
        Some("the ground under the feet is too uneven")
    } else if speed > TOUCHDOWN_MPS {
        Some("too fast")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Elite's touchdown rule: gear down, tilt at most 60 degrees, the feet within 0.5 m of each other, slow enough.
    #[test]
    fn touchdown_is_elites_rule() {
        assert_eq!(touchdown(true, 0.0, 0.0, 1.0), None);
        assert_eq!(touchdown(true, 60.0, 0.5, 1.0), None);
        assert!(touchdown(false, 0.0, 0.0, 1.0).is_some());
        assert!(touchdown(true, 60.5, 0.0, 1.0).is_some());
        assert!(touchdown(true, 0.0, 0.51, 1.0).is_some());
        assert!(touchdown(true, 0.0, 0.0, TOUCHDOWN_MPS + 0.1).is_some());
        assert_eq!(larger(0.2, -0.5), -0.5);
        assert_eq!(larger(-0.7, 0.5), -0.7);
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
