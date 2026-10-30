//! Spacecraft and flight mode.
//!
//! The "Ships" window lists the designs in `models/spacecraft`; "Fly" (or the `ship NAME`
//! script command) spawns one where the camera is and enters flight mode. A ship is an
//! entity with a [`UPosition`] and [`UOrientation`] like every other body, with its merged
//! hull meshes and engine plumes as children (see `ship_assets.rs`). Physics run in f64 in
//! the universe frame:
//!
//! * engines are accelerations in m/s^2 (`MainEngines` etc., decoded from the executable),
//!   `TurnEngines` is an angular acceleration in rad/s^2;
//! * gravity is summed over every body of the active system;
//! * the warp drive multiplies the displacement by `10^boost`, with the boost limited by
//!   the altitude above the nearest body (the executable's "cross the altitude in no less
//!   than two frames" rule), so warp speed grows with distance from massive bodies;
//! * ground contact uses the terrain height field.
//!
//! * in an atmosphere the design's aerodynamic model adds drag, lift and stabilising moments
//!   (`se_core::ship::aero`, `air.rs`), when the Physics key `Aero` is on;
//! * attitude holds and the autopilot turn the ship and drive its engines (`autopilot.rs`).
//!
//! While flight mode is on, the free camera ([`FlyCamera`]) is driven by [`ship_camera`]
//! instead of the user: `camera.rs` checks [`FlightHold`] and leaves the camera alone.
//!
//! Keys are SE's ship bindings (key table FUN_1405ba8f0): 5-0 Prograde .. Normal Down, End
//! Horizon, Num5 Kill rotation, U Flight assist, Z Stop engines, Alt+Z Stop warp drive, Shift+Z
//! Sync velocity, H Hold altitude, T Select target, P Select reference body, C Rotate to target,
//! Shift+G Fly to target, Alt+G Warp to target, G Fly or warp to target, Alt+= / Alt+- (and
//! PageUp / PageDown) warp drive +/-.

mod air;
mod autopilot;
mod engine_sound;

use std::sync::Arc;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::math::{DQuat, DVec2, DVec3};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::ship::aero::{AeroInput, aero_step};
use se_core::ship::{EffectGroup, ShipDesign, list_designs};
use se_core::units::{G, METERS_PER_AU, SPEED_OF_LIGHT, format_distance, format_speed};
use se_core::upos::UPos;

use self::air::AirCache;
use self::autopilot::{Attitude, Autopilot, Program, ProgramInput, RefMode};
use self::engine_sound::{EngineSoundCache, start_engine_sounds, update_engine_sounds};
use crate::camera::{FlightHold, FlyCamera, MainCamera, look_rotation};
use crate::data::SeData;
use crate::ship_assets::{
    LoadedShip, PlumeMaterial, PlumeSpec, PlumeUniform, ShipSurfaceMaterial, ShipUniform, build_ship, flat_texture,
    plume_mesh, plume_rotation,
};
use crate::system::SystemBody;
use crate::terrain::Terrain;
use crate::universe::{SimTime, UOrientation, UPosition, UniverseSet};

/// Largest angular rate (rad/s) per axis. The executable clamps its angular state to
/// +-0.03 in an unidentified unit; this value is an approximation that keeps big ships
/// steerable.
const MAX_TURN_RATE: f64 = 0.1;
/// Mouse turning: rad/s added per pixel, per rad/s^2 of `TurnEngines`.
const MOUSE_TURN_GAIN: f64 = 0.05;
/// Throttle slew (full scale per second) when W/S are held.
const THROTTLE_RATE: f64 = 0.5;
/// Engine spool-up (full scale per second); the executable's ramp factor is 2.0.
const ENGINE_SPOOL: f64 = 2.0;
/// Warp boost ramp, log10 units per second (the executable's default).
const WARP_RATE: f64 = 4.0;
/// Impact speed (m/s) from which a touchdown counts as a crash.
const CRASH_SPEED: f64 = 25.0;
/// Density (kg/m^3) assumed for bodies with no catalog mass.
const DEFAULT_DENSITY: f64 = 3000.0;

// ------------------------------------------------------------------------------------
// State
// ------------------------------------------------------------------------------------

/// Header-parsed designs available to fly.
#[derive(Resource, Default)]
pub struct ShipLibrary {
    pub designs: Vec<Arc<ShipDesign>>,
}

/// State of the Ships window.
#[derive(Resource, Default)]
pub struct ShipUi {
    pub show: bool,
    pub selected: usize,
    pub filter: String,
}

/// Script and UI requests (`ship NAME`, `throttle X`, ...).
#[derive(Message, Clone, Debug)]
pub struct ShipCommand {
    pub verb: String,
    pub arg: String,
}

/// Player input gathered in `Update`, consumed by the physics in `PostUpdate`.
#[derive(Clone, Copy, Default, Debug)]
pub struct ShipInput {
    /// RCS translation, each axis -1..1 (x right, y up).
    pub rcs: DVec3,
    /// Torque command per local axis (x pitch up, y yaw left, z roll left), -1..1.
    pub turn: DVec3,
    /// Mouse turning request for this frame (pixels).
    pub mouse: DVec2,
    pub boost_turn: bool,
    pub brake: bool,
}

/// Aerodynamic numbers of the last frame (SE's ship panel: Mach, Dynamic pressure, Air velocity,
/// Attack angle, Sideslip angle, Aero accel).
#[derive(Clone, Copy, Debug, Default)]
pub struct AeroInfo {
    pub active: bool,
    pub air_speed: f64,
    pub mach: f64,
    pub q: f64,
    pub alpha: f64,
    pub beta: f64,
    pub accel: f64,
    pub density: f64,
    pub sound_speed: f64,
}

/// Numbers shown on the flight HUD, written by the physics.
#[derive(Clone, Debug, Default)]
pub struct FlightInfo {
    pub speed_rel: f64,
    pub speed_eff: f64,
    pub nearest: String,
    pub altitude: f64,
    pub g: f64,
    pub notice: String,
    pub notice_until: f64,
    pub aero: AeroInfo,
    /// Target name and distance from its centre (m).
    pub target: String,
    pub target_distance: f64,
}

#[derive(Resource)]
pub struct Flight {
    /// Flight mode: the camera follows the ship and the keys fly it.
    pub active: bool,
    pub ship: Option<Entity>,
    /// Commanded main-engine throttle, -1 (full retro) to 1 (full main).
    pub throttle: f64,
    /// Whether the warp drive is engaged, and the requested boost (log10).
    pub warp_on: bool,
    pub warp_cmd: f64,
    /// Angular damping (flight assist).
    pub assist: bool,
    pub gravity: bool,
    pub collisions: bool,
    /// Index into the chase presets; one past the last is the free orbit camera.
    pub cam_mode: usize,
    /// Chase camera distance factor.
    pub zoom: f64,
    pub orbit_yaw: f64,
    pub orbit_pitch: f64,
    pub orbit_dist: f64,
    pub input: ShipInput,
    /// Held RCS / turn commands set by the `shiprcs` / `shipturn` script commands.
    pub script_rcs: DVec3,
    pub script_turn: DVec3,
    pub info: FlightInfo,
    /// Orientation hold (5-0, End).
    pub attitude: Attitude,
    /// Running autopilot program and its target (ShipSelectTarget, T).
    pub autopilot: Option<Autopilot>,
    pub target: Option<ObjectRef>,
    /// Reference body mode (P) and the body picked for [`RefMode::Selected`].
    pub ref_mode: RefMode,
    pub ref_body: Option<ObjectRef>,
    /// Kill rotation (Num5), Sync velocity (Shift+Z) and Hold altitude (H) programs.
    pub kill_rotation: bool,
    pub sync_velocity: bool,
    pub hold_altitude: Option<f64>,
    /// The Physics key `Aero` (main-def.cfg: true).
    pub aero: bool,
}

impl Default for Flight {
    fn default() -> Self {
        Self {
            active: false,
            ship: None,
            throttle: 0.0,
            warp_on: false,
            warp_cmd: 0.0,
            assist: true,
            gravity: true,
            collisions: true,
            cam_mode: 0,
            zoom: 1.0,
            orbit_yaw: 0.6,
            orbit_pitch: 0.3,
            orbit_dist: 100.0,
            input: ShipInput::default(),
            script_rcs: DVec3::ZERO,
            script_turn: DVec3::ZERO,
            info: FlightInfo::default(),
            attitude: Attitude::Free,
            autopilot: None,
            target: None,
            ref_mode: RefMode::Nearest,
            ref_body: None,
            kill_rotation: false,
            sync_velocity: false,
            hold_altitude: None,
            aero: true,
        }
    }
}

/// Physical state of the player's ship.
#[derive(Component)]
pub struct Ship {
    pub design: Arc<ShipDesign>,
    /// Velocity in the universe frame (m/s).
    pub vel: DVec3,
    /// Angular velocity in the ship frame (rad/s).
    pub ang_vel: DVec3,
    /// Spooled main-engine level, -1..1.
    pub engine: f64,
    /// Current warp boost (log10 multiplier).
    pub warp: f64,
    /// Distance from the pivot to the hull bottom (m).
    pub clearance: f64,
    pub radius: f64,
    pub landed: Option<Landed>,
}

/// The ship rests on a body and turns with it.
#[derive(Clone, Copy)]
pub struct Landed {
    pub body: Entity,
    pub local_pos: DVec3,
    pub local_rot: DQuat,
}

#[derive(Component)]
struct ShipLoading(Task<Option<LoadedShip>>);

struct PlumeInstance {
    entity: Entity,
    material: Handle<PlumeMaterial>,
    spec: PlumeSpec,
    base_scale: Vec3,
    visible: bool,
}

#[derive(Component, Default)]
struct ShipVisuals {
    hull: Vec<Handle<ShipSurfaceMaterial>>,
    plumes: Vec<PlumeInstance>,
}

#[derive(Resource)]
struct PlumeMesh(Handle<Mesh>);

pub struct ShipPlugin;

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((MaterialPlugin::<ShipSurfaceMaterial>::default(), MaterialPlugin::<PlumeMaterial>::default()))
            .init_resource::<Flight>()
            .insert_resource(ShipUi { show: std::env::var_os("SE_SHIPS_WINDOW").is_some(), ..default() })
            .init_resource::<ShipLibrary>()
            .init_resource::<AirCache>()
            .init_resource::<EngineSoundCache>()
            .init_resource::<FlightKeys>()
            .add_message::<ShipCommand>()
            .add_systems(Startup, (load_library, make_plume_mesh))
            .add_systems(PreUpdate, take_flight_keys.after(bevy::input::InputSystems))
            .add_systems(
                Update,
                (ship_commands, ship_input, finish_ship_loading, sync_hold, start_engine_sounds).chain(),
            )
            .add_systems(
                PostUpdate,
                // Plumes and sounds after the physics, which adds the programs' RCS and turn commands.
                (ship_physics, ship_camera, update_ship_visuals, update_engine_sounds)
                    .chain()
                    .in_set(UniverseSet::Camera)
                    .after(crate::camera::travel_and_move)
                    .before(crate::camera::sync_camera_transform),
            )
            .add_systems(
                PostUpdate,
                ship_near_plane.in_set(UniverseSet::Camera).after(crate::camera::sync_camera_transform),
            )
            .add_systems(EguiPrimaryContextPass, (draw_ship_window, draw_flight_hud).in_set(crate::skin::UiSet::Draw));
    }
}

fn load_library(data: Res<SeData>, mut lib: ResMut<ShipLibrary>) {
    let mut designs = Vec::new();
    for path in list_designs(&data.vfs) {
        match ShipDesign::load_header(&data.vfs, &path) {
            Some(d) => designs.push(Arc::new(d)),
            None => warn!("cannot read ship design {path}"),
        }
    }
    designs.sort_by_key(|d| d.name.to_ascii_lowercase());
    info!("{} ship designs", designs.len());
    lib.designs = designs;
}

fn make_plume_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(PlumeMesh(meshes.add(plume_mesh())));
}

// ------------------------------------------------------------------------------------
// Commands, input
// ------------------------------------------------------------------------------------

fn find_design(lib: &ShipLibrary, name: &str) -> Option<Arc<ShipDesign>> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() {
        return None;
    }
    let exact = lib
        .designs
        .iter()
        .find(|d| d.id.to_ascii_lowercase() == n || d.name.to_ascii_lowercase() == n);
    exact
        .or_else(|| lib.designs.iter().find(|d| d.id.to_ascii_lowercase().contains(&n) || d.name.to_ascii_lowercase().contains(&n)))
        .cloned()
}

/// Velocity of a body in the universe frame (m per simulated second).
fn body_velocity(eph: &Ephemeris, obj: ObjectRef, sim: &SimTime) -> DVec3 {
    if sim.paused {
        return DVec3::ZERO;
    }
    let h = 0.5 / 86_400.0;
    eph.position(obj, sim.jd + h).delta_m(eph.position(obj, sim.jd - h))
}

/// Velocity of the frame a ship at `pos` is "at rest" in: the body's orbital velocity, plus
/// its rotation (so the surface is stationary) up to one radius of altitude, faded out by two;
/// the orbital part fades out between 50 and 100 radii (far from every body the universe frame
/// is used: the planets' orbital speeds mean nothing there). The fades keep the frame continuous,
/// so the warp, which multiplies the velocity relative to it, never turns a switch into a jump.
fn reference_velocity(eph: &Ephemeris, obj: ObjectRef, body_pos: UPos, radius: f64, pos: UPos, sim: &SimTime) -> DVec3 {
    let rel = pos.delta_m(body_pos);
    let (_, fade, spin) = frame_terms(rel.length() - radius, radius);
    if fade <= 0.0 {
        return DVec3::ZERO;
    }
    let spin_v = if spin > 0.0 { body_spin(eph, obj, sim).cross(rel) * spin } else { DVec3::ZERO };
    (body_velocity(eph, obj, sim) + spin_v) * fade
}

/// A body's share of the blended frame at altitude `alt` over it (radius `r`): its weight
/// (inverse square altitude), the fade of its orbital velocity (1 below 50 radii, 0 beyond 100)
/// and of its rotation (1 below one radius, 0 beyond two).
fn frame_terms(alt: f64, r: f64) -> (f64, f64, f64) {
    let r = r.max(1.0);
    let w = 1.0 / alt.max(1e-3 * r).powi(2);
    let fade = ((100.0 * r - alt) / (50.0 * r)).clamp(0.0, 1.0);
    let spin = ((2.0 * r - alt) / r).clamp(0.0, 1.0);
    (w, fade, spin)
}

/// The frame the warp drive multiplies velocities against when the player warps: every body's
/// [`reference_velocity`] weighted by inverse square altitude, so it changes smoothly where the
/// nearest body changes (`parts`: altitude, radius, frame velocity of each body).
fn blend_frames(parts: impl IntoIterator<Item = (f64, f64, DVec3)>) -> DVec3 {
    let (mut sum, mut wsum) = (DVec3::ZERO, 0.0);
    for (alt, r, v) in parts {
        let (w, ..) = frame_terms(alt, r);
        sum += v * w;
        wsum += w;
    }
    if wsum > 0.0 { sum / wsum } else { DVec3::ZERO }
}

/// Angular velocity of a body (rad per simulated second, universe frame).
fn body_spin(eph: &Ephemeris, obj: ObjectRef, sim: &SimTime) -> DVec3 {
    if sim.paused {
        return DVec3::ZERO;
    }
    let h = 0.5 / 86_400.0;
    let dq = eph.orientation(obj, sim.jd + h) * eph.orientation(obj, sim.jd - h).inverse();
    let (axis, angle) = dq.to_axis_angle();
    if angle.is_finite() { axis * angle } else { DVec3::ZERO }
}

#[allow(clippy::too_many_arguments)]
fn ship_commands(
    mut reader: MessageReader<ShipCommand>,
    mut commands: Commands,
    data: Res<SeData>,
    lib: Res<ShipLibrary>,
    sim: Res<SimTime>,
    mut flight: ResMut<Flight>,
    mut cam: ResMut<FlyCamera>,
    time: Res<Time>,
    bodies: Query<&SystemBody>,
    ships: Query<(Entity, &UPosition, &Ship)>,
) {
    for msg in reader.read() {
        let arg = msg.arg.trim();
        match msg.verb.as_str() {
            "ship" => {
                let design = if arg.is_empty() {
                    lib.designs.first().cloned()
                } else {
                    find_design(&lib, arg)
                };
                let Some(design) = design else {
                    warn!("ship: no design matching '{arg}'");
                    continue;
                };
                for (e, ..) in &ships {
                    commands.entity(e).despawn();
                }
                spawn_ship(&mut commands, &data, &sim, &mut flight, &mut cam, &bodies, design);
            }
            "throttle" => {
                if let Ok(v) = arg.parse::<f64>() {
                    flight.throttle = v.clamp(-1.0, 1.0);
                }
            }
            "warp" => {
                if let Ok(v) = arg.parse::<f64>() {
                    flight.warp_cmd = v.max(0.0);
                    flight.warp_on = v > 0.0;
                } else {
                    flight.warp_on = !matches!(arg, "off" | "0");
                }
            }
            "shipcam" => {
                if let Ok(v) = arg.parse::<usize>() {
                    flight.cam_mode = v;
                }
            }
            "shiporbit" => {
                // shiporbit YAW PITCH [DIST_M]: the orbit camera, angles in degrees.
                let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                if v.len() >= 2 {
                    flight.orbit_yaw = v[0].to_radians();
                    flight.orbit_pitch = v[1].to_radians();
                    if let Some(d) = v.get(2) {
                        flight.orbit_dist = *d;
                    }
                    flight.cam_mode = usize::MAX; // any index past the chase presets is the orbit view
                }
            }
            "shiprcs" | "shipturn" => {
                let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                let c = DVec3::new(
                    v.first().copied().unwrap_or(0.0),
                    v.get(1).copied().unwrap_or(0.0),
                    v.get(2).copied().unwrap_or(0.0),
                )
                .clamp(DVec3::splat(-1.0), DVec3::ONE);
                if msg.verb == "shiprcs" {
                    flight.script_rcs = c;
                } else {
                    flight.script_turn = c;
                }
            }
            "flight" => {
                // `flight on|off`, or a flight program: `flight target NAME|selected`,
                // `flight ap flyto|warpto|goto|rotateto|off`, `flight hold MODE`,
                // `flight ref nearest|influence|NAME`, `flight aero on|off`, `flight sync`,
                // `flight killrot`, `flight holdalt`.
                let (sub, rest) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
                let on = !matches!(rest, "off" | "0" | "false");
                let find = |n: &str| if n.is_empty() || n == "selected" { cam.selected } else { data.catalog.find(n) };
                match sub {
                    "target" => flight.target = find(rest),
                    "ap" => {
                        let program = match rest {
                            "flyto" => Some(Program::FlyTo),
                            "warpto" => Some(Program::WarpTo),
                            "rotateto" => Some(Program::RotateTo),
                            "goto" => {
                                let eph = Ephemeris::new(&data.catalog);
                                let d = match (flight.target, flight.ship.and_then(|e| ships.get(e).ok())) {
                                    (Some(t), Some((_, p, ship))) => (eph.position(t, sim.jd).delta_m(p.0).length(), ship.design.clone()),
                                    _ => continue,
                                };
                                Some(autopilot::choose_goto(d.1.can_warp(), d.0, d.1.main_engines))
                            }
                            _ => None,
                        };
                        start_program(&mut flight, program, time.elapsed_secs_f64());
                    }
                    "hold" => {
                        if let Some(a) = Attitude::parse(rest) {
                            flight.attitude = a;
                            flight.autopilot = None;
                        }
                    }
                    "ref" => {
                        flight.ref_mode = match rest {
                            "nearest" => RefMode::Nearest,
                            "influence" => RefMode::Influence,
                            n => {
                                flight.ref_body = find(n);
                                RefMode::Selected
                            }
                        }
                    }
                    "aero" => flight.aero = on,
                    "sync" => flight.sync_velocity = on,
                    "killrot" => flight.kill_rotation = on,
                    "holdalt" => flight.hold_altitude = if on { Some(flight.info.altitude) } else { None },
                    _ => flight.active = flight.ship.is_some() && !matches!(arg, "off" | "0" | "false"),
                }
            }
            "unship" => {
                for (e, ..) in &ships {
                    commands.entity(e).despawn();
                }
                flight.ship = None;
                flight.active = false;
            }
            _ => {}
        }
    }
}

/// Places `design` so that its first chase camera coincides with the free camera, at rest
/// relative to the nearest body, and enters flight mode.
fn spawn_ship(
    commands: &mut Commands,
    data: &SeData,
    sim: &SimTime,
    flight: &mut Flight,
    cam: &mut FlyCamera,
    bodies: &Query<&SystemBody>,
    design: Arc<ShipDesign>,
) {
    let full = match ShipDesign::load(&data.vfs, &design.path) {
        Some(d) => Arc::new(d),
        None => {
            warn!("ship: cannot load {}", design.path);
            return;
        }
    };
    let preset = full.default_camera();
    let local_cam = preset.pos;
    let orient = (cam.rot * preset.rot.inverse()).normalize();
    let pos = cam.pos.offset_m(-(orient * local_cam));
    let eph = Ephemeris::new(&data.catalog);
    let vel = bodies
        .iter()
        .min_by(|a, b| {
            let da = a.pos.delta_m(pos).length() - a.radius_m;
            let db = b.pos.delta_m(pos).length() - b.radius_m;
            da.total_cmp(&db)
        })
        .map(|b| reference_velocity(&eph, b.obj, b.pos, b.radius_m, pos, sim))
        .unwrap_or(DVec3::ZERO);

    let vfs = data.vfs.clone();
    let d2 = full.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move { build_ship(&vfs, &d2) });
    let id = commands
        .spawn((
            Ship {
                design: full.clone(),
                vel,
                ang_vel: DVec3::ZERO,
                engine: 0.0,
                warp: 0.0,
                clearance: full.length_m * 0.1,
                radius: full.length_m * 0.5,
                landed: None,
            },
            ShipLoading(task),
            ShipVisuals::default(),
            UPosition(pos),
            UOrientation(orient),
            Transform::IDENTITY,
            Visibility::Visible,
        ))
        .id();
    flight.ship = Some(id);
    flight.active = true;
    flight.throttle = 0.0;
    flight.warp_on = false;
    flight.warp_cmd = full.warp_boost_log;
    flight.cam_mode = 0;
    flight.zoom = 1.0;
    flight.orbit_dist = full.length_m * 1.3;
    flight.input = ShipInput::default();
    flight.autopilot = None;
    flight.attitude = Attitude::Free;
    flight.kill_rotation = false;
    flight.sync_velocity = false;
    flight.hold_altitude = None;
    cam.travel = None;
    cam.follow = None;
    cam.velocity = DVec3::ZERO;
    info!("flying {} ({} modules)", full.name, full.modules.len());
}

fn ship_input(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut flight: ResMut<Flight>,
    ships: Query<(&Ship, &UPosition)>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut egui: EguiContexts,
    mut taken: ResMut<FlightKeys>,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    sim: Res<SimTime>,
) {
    let taken = std::mem::take(&mut *taken);
    let (egui_pointer, egui_keys) = egui
        .ctx_mut()
        .map(|c| (c.egui_wants_pointer_input() || crate::hud::pointer_over_ui(c), c.egui_wants_keyboard_input()))
        .unwrap_or((false, false));
    let dt = time.delta_secs_f64();

    if !egui_keys {
        if keys.just_pressed(KeyCode::Escape) && flight.active {
            flight.active = false;
            flight.throttle = 0.0;
            stop_programs(&mut flight);
        } else if keys.just_pressed(KeyCode::Enter) && flight.ship.is_some() {
            flight.active = !flight.active;
            if !flight.active {
                flight.throttle = 0.0;
            }
        } else if keys.just_pressed(KeyCode::KeyF) && flight.ship.is_some() && !flight.active {
            // F only enters: in flight it is SE's ShipMoveDown.
            flight.active = true;
        }
    }
    flight.input = ShipInput::default();
    let Some((ship, ship_pos)) = flight.ship.and_then(|e| ships.get(e).ok()) else {
        if flight.active {
            flight.active = false;
        }
        return;
    };
    if !flight.active {
        return;
    }

    // Mouse: turning in chase views, orbiting in the free view.
    let n_chase = ship.design.chase_cameras().len();
    let orbit_view = flight.cam_mode >= n_chase;
    let dragging = !egui_pointer && (buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Left));
    if dragging {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
        if orbit_view {
            flight.orbit_yaw -= motion.delta.x as f64 * 0.005;
            flight.orbit_pitch = (flight.orbit_pitch + motion.delta.y as f64 * 0.005).clamp(-1.5, 1.5);
        } else {
            flight.input.mouse = motion.delta.as_dvec2();
        }
    } else if cursor.grab_mode != CursorGrabMode::None {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
    if !egui_pointer && scroll.delta.y != 0.0 {
        let f = 0.9f64.powf(scroll.delta.y as f64);
        if orbit_view {
            flight.orbit_dist = (flight.orbit_dist * f).clamp(ship.design.length_m * 0.2, ship.design.length_m * 20.0);
        } else {
            flight.zoom = (flight.zoom * f).clamp(0.2, 10.0);
        }
    }
    if egui_keys {
        return;
    }

    // Throttle (W/S); Z is SE's Stop engines (ShipStopEngines), X the port's cut.
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let plain = !shift && !alt && !ctrl;
    let now = time.elapsed_secs_f64();
    let mut dthr = 0.0;
    if keys.pressed(KeyCode::KeyW) {
        dthr += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        dthr -= 1.0;
    }
    flight.throttle = (flight.throttle + dthr * THROTTLE_RATE * dt).clamp(-1.0, 1.0);
    if keys.just_pressed(KeyCode::KeyX) {
        flight.throttle = 0.0;
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        if alt {
            stop_warp_drive(&mut flight);
        } else if shift {
            // ShipSyncVelocity.
            flight.sync_velocity = !flight.sync_velocity;
            flight.autopilot = None;
        } else {
            // ShipStopEngines: also ends a running program (and the jump it was making).
            flight.throttle = 0.0;
            if flight.autopilot.take().is_some_and(|a| a.program == Program::WarpTo) {
                flight.warp_on = false;
            }
            flight.hold_altitude = None;
        }
    }

    // RCS translation.
    let axis = |pos: KeyCode, neg: KeyCode| keys.pressed(pos) as i32 as f64 - keys.pressed(neg) as i32 as f64;
    flight.input.rcs = DVec3::new(axis(KeyCode::KeyD, KeyCode::KeyA), axis(KeyCode::KeyR, KeyCode::KeyF), 0.0);
    // Rotation: arrows pitch/yaw, Q/E roll.
    flight.input.turn = DVec3::new(
        axis(KeyCode::ArrowUp, KeyCode::ArrowDown),
        axis(KeyCode::ArrowLeft, KeyCode::ArrowRight),
        axis(KeyCode::KeyQ, KeyCode::KeyE),
    );
    flight.input.rcs = (flight.input.rcs + flight.script_rcs).clamp(DVec3::splat(-1.0), DVec3::ONE);
    flight.input.turn = (flight.input.turn + flight.script_turn).clamp(DVec3::splat(-1.0), DVec3::ONE);
    flight.input.boost_turn = shift;
    flight.input.brake = keys.pressed(KeyCode::Backspace);
    if flight.input.brake {
        // The port's brake: it takes over from any program.
        stop_programs(&mut flight);
        flight.throttle = 0.0;
    }

    if keys.just_pressed(KeyCode::KeyU) {
        // ShipFlightAssist.
        flight.assist = !flight.assist;
    }
    if keys.just_pressed(KeyCode::Numpad5) && !ctrl {
        // ShipStopRotation: drop the holds and brake the rotation.
        flight.attitude = Attitude::Free;
        flight.kill_rotation = true;
        if flight.autopilot.is_some_and(|a| a.program == Program::RotateTo) {
            flight.autopilot = None;
        }
    }
    // Orientation holds, 5-0 and End (pressing the active one again releases it).
    let holds = [
        (KeyCode::Digit5, Attitude::Prograde),
        (KeyCode::Digit6, Attitude::Retrograde),
        (KeyCode::Digit7, Attitude::Radial),
        (KeyCode::Digit8, Attitude::Antiradial),
        (KeyCode::Digit9, Attitude::NormalUp),
        (KeyCode::Digit0, Attitude::NormalDown),
        (KeyCode::End, Attitude::Horizon),
    ];
    for (k, a) in holds {
        let pressed = if k == KeyCode::Digit0 { taken.digit0 } else { keys.just_pressed(k) };
        if pressed && plain {
            flight.attitude = if flight.attitude == a { Attitude::Free } else { a };
            flight.kill_rotation = false;
            if flight.autopilot.is_some_and(|a| a.program == Program::RotateTo) {
                flight.autopilot = None;
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyH) && plain {
        // ShipHoldAlt.
        flight.hold_altitude = match flight.hold_altitude {
            Some(_) => None,
            None => flight.info.altitude.is_finite().then_some(flight.info.altitude),
        };
    }
    // Target and reference body: the camera's selection.
    if keys.just_pressed(KeyCode::KeyT) && plain {
        flight.target = cam.selected;
        let name = cam.selected.map_or("none".to_string(), |o| data.catalog.display_name(o).to_string());
        notify(&mut flight, format!("Target: {name}"), now);
    }
    if taken.p {
        flight.ref_mode = match flight.ref_mode {
            RefMode::Nearest => RefMode::Influence,
            RefMode::Influence if cam.selected.is_some() => RefMode::Selected,
            _ => RefMode::Nearest,
        };
        if flight.ref_mode == RefMode::Selected {
            flight.ref_body = cam.selected;
        }
        let label = flight.ref_mode.label();
        notify(&mut flight, format!("Reference body: {label}"), now);
    }
    // Autopilot: C rotate to, Shift+G fly to, Alt+G warp to, G fly or warp.
    let program = if keys.just_pressed(KeyCode::KeyC) && plain {
        Some(Program::RotateTo)
    } else if keys.just_pressed(KeyCode::KeyG) && !ctrl {
        Some(if shift {
            Program::FlyTo
        } else if alt {
            Program::WarpTo
        } else {
            let eph = Ephemeris::new(&data.catalog);
            let d = flight.target.map_or(0.0, |t| eph.position(t, sim.jd).delta_m(ship_pos.0).length());
            autopilot::choose_goto(ship.design.can_warp(), d, ship.design.main_engines)
        })
    } else {
        None
    };
    if let Some(p) = program {
        let running = flight.autopilot.is_some_and(|a| a.program == p);
        start_program(&mut flight, if running { None } else { Some(p) }, now);
    }
    if keys.just_pressed(KeyCode::KeyV) && plain {
        flight.cam_mode = if flight.cam_mode >= n_chase { 0 } else { flight.cam_mode + 1 };
        flight.zoom = 1.0;
    }
    // Warp drive +/- (SE's ShipWarpDrivePlus / Minus, Alt+= / Alt+-; also PageUp / PageDown):
    // + engages the drive at its last level, then raises it; - lowers it and stops it at 0.
    let wmax = ship.design.warp_boost_log;
    if keys.just_pressed(KeyCode::PageUp) || (alt && keys.just_pressed(KeyCode::Equal)) {
        if !ship.design.can_warp() {
            notify(&mut flight, "No warp drive".to_string(), now);
        } else if !flight.warp_on {
            flight.warp_on = true;
            if flight.warp_cmd < 1.0 {
                flight.warp_cmd = wmax;
            }
        } else {
            flight.warp_cmd = (flight.warp_cmd + 1.0).min(wmax);
        }
    }
    if keys.just_pressed(KeyCode::PageDown) || (alt && keys.just_pressed(KeyCode::Minus)) {
        flight.warp_cmd = (flight.warp_cmd - 1.0).max(0.0);
        if flight.warp_cmd < 1.0 {
            flight.warp_on = false;
        }
    }
    if flight.input.brake {
        flight.warp_on = false;
    }
}

/// Keys SE uses in flight that other parts of the port bind globally (0 toggles the chart in
/// `browser.rs`, P resets the magnitude limit in `stars.rs`): taken from the input while flying.
#[derive(Resource, Default)]
struct FlightKeys {
    digit0: bool,
    p: bool,
}

fn take_flight_keys(hold: Res<FlightHold>, mut keys: ResMut<ButtonInput<KeyCode>>, mut taken: ResMut<FlightKeys>, mut egui: EguiContexts) {
    if !hold.0 || egui.ctx_mut().is_ok_and(|c| c.egui_wants_keyboard_input()) {
        return;
    }
    let mods = [KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::AltLeft, KeyCode::AltRight, KeyCode::ControlLeft, KeyCode::ControlRight];
    if !keys.any_pressed(mods) {
        taken.digit0 |= keys.clear_just_pressed(KeyCode::Digit0);
        taken.p |= keys.clear_just_pressed(KeyCode::KeyP);
    }
}

/// ShipStopWarpDrive (Alt+Z): the drive off, and a Warp to target program with it (it would
/// engage the drive again).
fn stop_warp_drive(flight: &mut Flight) {
    flight.warp_on = false;
    if flight.autopilot.is_some_and(|a| a.program == Program::WarpTo) {
        flight.autopilot = None;
        flight.throttle = 0.0;
    }
}

/// Ends the autopilot and every hold (leaving the pilot's seat, braking): nothing keeps driving
/// the engines or the warp drive it engaged.
fn stop_programs(flight: &mut Flight) {
    if let Some(a) = flight.autopilot.take() {
        if a.program != Program::RotateTo {
            flight.throttle = 0.0;
        }
        if a.program == Program::WarpTo {
            flight.warp_on = false;
        }
    }
    flight.attitude = Attitude::Free;
    flight.kill_rotation = false;
    flight.sync_velocity = false;
    flight.hold_altitude = None;
}

fn notify(flight: &mut Flight, text: String, now: f64) {
    flight.info.notice = text;
    flight.info.notice_until = now + 3.0;
}

/// Starts an autopilot program at the target (`None` stops the running one); the holds give way.
fn start_program(flight: &mut Flight, program: Option<Program>, now: f64) {
    match program {
        Some(_) if flight.target.is_none() => notify(flight, "Select a target first (T)".to_string(), now),
        Some(p) => {
            flight.autopilot = Some(Autopilot::new(p));
            flight.attitude = Attitude::Free;
            flight.kill_rotation = false;
            flight.sync_velocity = false;
            notify(flight, format!("Autopilot: {}", p.label()), now);
        }
        None => {
            if let Some(a) = flight.autopilot.take() {
                if a.program == Program::WarpTo {
                    flight.warp_on = false;
                }
                notify(flight, "Autopilot off".to_string(), now);
            }
        }
    }
}

fn sync_hold(flight: Res<Flight>, mut hold: ResMut<FlightHold>) {
    if hold.0 != flight.active {
        hold.0 = flight.active;
    }
}

// ------------------------------------------------------------------------------------
// Loading
// ------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn finish_ship_loading(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut hull_mats: ResMut<Assets<ShipSurfaceMaterial>>,
    mut plume_mats: ResMut<Assets<PlumeMaterial>>,
    plume_mesh: Option<Res<PlumeMesh>>,
    mut loading: Query<(Entity, &mut ShipLoading, &mut Ship, &mut ShipVisuals)>,
    mut flight: ResMut<Flight>,
) {
    let Some(plume_mesh) = plume_mesh else { return };
    for (entity, mut task, mut ship, mut visuals) in &mut loading {
        let Some(result) = check_ready(&mut task.0) else { continue };
        commands.entity(entity).remove::<ShipLoading>();
        let Some(loaded) = result else {
            warn!("failed to build ship {}", ship.design.name);
            if flight.ship == Some(entity) {
                flight.active = false;
                flight.ship = None;
            }
            commands.entity(entity).despawn();
            continue;
        };
        ship.clearance = loaded.clearance;
        ship.radius = loaded.radius;
        let white = images.add(flat_texture([255, 255, 255, 255]));
        let black = images.add(flat_texture([0, 0, 0, 255]));
        let n_groups = loaded.groups.len();
        for g in loaded.groups {
            let diffuse = g.diffuse.map(|i| images.add(i)).unwrap_or_else(|| white.clone());
            let emissive = g.emissive.map(|i| images.add(i)).unwrap_or_else(|| black.clone());
            let mat = hull_mats.add(ShipSurfaceMaterial {
                u: ShipUniform {
                    light_dir: [Vec4::new(0.0, 1.0, 0.0, 1.0), Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
                    light_color: [Vec4::ONE, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
                    tint: Vec4::new(g.tint[0], g.tint[1], g.tint[2], 0.0),
                    emis: Vec4::new(g.emis[0], g.emis[1], g.emis[2], g.cutoff),
                },
                diffuse,
                emissive,
            });
            visuals.hull.push(mat.clone());
            commands.spawn((
                ChildOf(entity),
                Mesh3d(meshes.add(g.mesh)),
                MeshMaterial3d(mat),
                Transform::IDENTITY,
                Visibility::Inherited,
                NoFrustumCulling,
            ));
        }
        for (i, spec) in loaded.plumes.iter().enumerate() {
            let rot = plume_rotation(spec.dir, spec.x_axis);
            let base_scale = Vec3::new(spec.size.x * 0.5, spec.size.y * 0.5, spec.size.z);
            let intensity = spec.brightness
                * match spec.kind.as_str() {
                    "Fusion" | "Chemical" => 3.0,
                    "Aerospike" => 2.5,
                    "Hyper" => 2.0,
                    _ => 1.6,
                };
            let material = plume_mats.add(PlumeMaterial {
                u: PlumeUniform {
                    color: Vec4::new(spec.color[0] * 3.0, spec.color[1] * 3.0, spec.color[2] * 3.0, intensity),
                    params: Vec4::new(0.0, 0.0, i as f32, 0.0),
                },
            });
            let e = commands
                .spawn((
                    ChildOf(entity),
                    Mesh3d(plume_mesh.0.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform { translation: spec.pos, rotation: rot, scale: base_scale * 1e-3 },
                    Visibility::Hidden,
                    NoFrustumCulling,
                ))
                .id();
            visuals.plumes.push(PlumeInstance { entity: e, material, spec: spec.clone(), base_scale, visible: false });
        }
        info!(
            "ship {} built: {} meshes, {} vertices, {} plumes, {} modules unresolved",
            ship.design.name,
            n_groups,
            loaded.vertices,
            visuals.plumes.len(),
            loaded.modules_missing
        );
    }
}

// ------------------------------------------------------------------------------------
// Physics
// ------------------------------------------------------------------------------------

struct BodyInfo<'a> {
    entity: Entity,
    obj: ObjectRef,
    pos: UPos,
    orient: DQuat,
    radius: f64,
    gm: f64,
    terrain: Option<&'a Terrain>,
}

fn body_mass(data: &SeData, obj: ObjectRef, radius: f64) -> f64 {
    let m = match obj {
        ObjectRef::Body(i) => data.catalog.bodies.get(i as usize).map(|b| b.mass_kg).unwrap_or(0.0),
        ObjectRef::Star(i) => data.catalog.stars.get(i as usize).map(|s| s.mass_kg).unwrap_or(0.0),
        _ => 0.0,
    };
    if m > 0.0 { m } else { 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3) * DEFAULT_DENSITY }
}

/// Gravitational acceleration at `pos` summed over `bodies` (m/s^2).
fn gravity_at(bodies: &[BodyInfo], pos: UPos) -> DVec3 {
    let mut a = DVec3::ZERO;
    for b in bodies {
        let r = b.pos.delta_m(pos);
        let d = r.length();
        if d <= 0.0 {
            continue;
        }
        // Inside the body the field falls off linearly (uniform density).
        let d_eff = d.max(b.radius);
        a += r * (b.gm / (d_eff * d_eff * d_eff));
    }
    a
}

#[allow(clippy::too_many_arguments)]
fn ship_physics(
    time: Res<Time>,
    sim: Res<SimTime>,
    data: Res<SeData>,
    mut flight: ResMut<Flight>,
    mut air: ResMut<AirCache>,
    body_q: Query<(Entity, &SystemBody, Option<&Terrain>)>,
    mut ships: Query<(&mut Ship, &mut UPosition, &mut UOrientation)>,
) {
    let Some(entity) = flight.ship else { return };
    let Ok((mut ship, mut upos, mut urot)) = ships.get_mut(entity) else { return };
    let dt = time.delta_secs_f64().min(0.1);
    if dt <= 0.0 {
        return;
    }
    // Ship time follows simulated time (up to 10x), or real time while the clock is paused.
    let ship_dt = if sim.paused { dt } else { dt * sim.rate.abs().clamp(0.01, 10.0) };
    let design = ship.design.clone();
    let eph = Ephemeris::new(&data.catalog);
    let now = time.elapsed_secs_f64();

    let infos: Vec<BodyInfo> = body_q
        .iter()
        .map(|(entity, b, terrain)| BodyInfo {
            entity,
            obj: b.obj,
            pos: b.pos,
            orient: b.orient,
            radius: b.radius_m,
            gm: G * body_mass(&data, b.obj, b.radius_m),
            terrain,
        })
        .collect();
    let (nearest, _, _, altitude) = nearest_state(&infos, &eph, &sim, &data, upos.0);
    let refb = reference_body(&infos, flight.ref_mode, flight.ref_body, upos.0).or(nearest);
    let ref_vel = refb.map_or(DVec3::ZERO, |b| reference_velocity(&eph, b.obj, b.pos, b.radius, upos.0, &sim));

    // Engine accelerations.
    let retro_a = if design.retro_engines > 0.0 {
        design.retro_engines
    } else if design.corr_engines > 0.0 {
        design.corr_engines
    } else {
        design.main_engines * 0.2
    };
    let corr_a = if design.corr_engines > 0.0 { design.corr_engines } else { design.main_engines * 0.05 };
    let up_a = if design.hover_engines > 0.0 { design.hover_engines } else { corr_a };

    // --- Autopilot and attitude holds ---------------------------------------------------------
    let mut input = flight.input;
    let mut attitude: Option<DQuat> = None;
    let mut ap_out = autopilot::ProgramOutput::default();
    let target = flight.target.map(|t| {
        let b = infos.iter().find(|b| b.obj == t);
        let pos = b.map_or_else(|| eph.position(t, sim.jd), |b| b.pos);
        let radius = b.map_or_else(|| eph.radius_m(t), |b| b.radius);
        (t, pos.delta_m(upos.0), ship.vel - body_velocity(&eph, t, &sim), radius)
    });
    flight.info.target = flight.target.map_or(String::new(), |t| data.catalog.display_name(t).to_string());
    flight.info.target_distance = target.map_or(f64::NAN, |t| t.1.length());
    if !flight.active {
        // Programs run only while the ship is piloted (the Ships window's Leave, `flight off`).
        stop_programs(&mut flight);
    }
    match (flight.autopilot, target) {
        (Some(mut ap), Some((_, to_target, rel_vel, radius))) => {
            let i = ProgramInput {
                orient: urot.0,
                to_target,
                rel_vel,
                gravity: if flight.gravity { gravity_at(&infos, upos.0) } else { DVec3::ZERO },
                stop_distance: (radius * autopilot::STOP_RADII).max(design.length_m * 20.0),
                main: design.main_engines,
                retro: design.retro_engines.max(0.0),
                corr: corr_a,
                up: up_a,
                can_warp: design.can_warp(),
                flip_time: autopilot::flip_time(design.turn_engines, MAX_TURN_RATE),
                warp: ship.warp,
                dt: ship_dt,
            };
            let before = ap.stage;
            ap_out = autopilot::run_program(&mut ap, &i);
            attitude = ap_out.attitude;
            if ap.stage != before {
                info!("autopilot {}: {} at {} from the target", ap.program.label(), ap.stage.label(), format_distance(to_target.length()));
            }
            if ap_out.done {
                info!("autopilot {} done at {} from the target", ap.program.label(), format_distance(to_target.length()));
                flight.autopilot = None;
                flight.warp_on = false;
                if ap.program != Program::RotateTo {
                    // Arrived: match the frame of the body there.
                    flight.sync_velocity = true;
                    flight.throttle = 0.0;
                }
                notify(&mut flight, format!("{}: done", ap.program.label()), now);
            } else {
                flight.autopilot = Some(ap);
                if ap.program != Program::RotateTo {
                    flight.throttle = ap_out.throttle;
                    flight.warp_on = ap_out.warp_on;
                    if ap_out.warp_on && flight.warp_cmd < 1.0 {
                        flight.warp_cmd = design.warp_boost_log;
                    }
                    input.rcs = (input.rcs + ap_out.rcs).clamp(DVec3::splat(-1.0), DVec3::ONE);
                }
            }
        }
        (Some(_), None) => {
            flight.autopilot = None;
            flight.warp_on = false;
        }
        _ => {}
    }
    if attitude.is_none() && flight.attitude != Attitude::Free {
        if let Some(b) = refb {
            attitude = autopilot::attitude_rotation(flight.attitude, urot.0, upos.0.delta_m(b.pos), ship.vel - ref_vel);
        }
    }

    // --- Air ---------------------------------------------------------------------------------
    // The nearest body whose atmosphere the ship is in (SE: planet with air, altitude under its
    // height, the Physics key Aero).
    let aero_body = nearest.filter(|_| flight.aero && ship.landed.is_none()).and_then(|b| {
        let top = b.terrain.and_then(|t| t.atmosphere.as_ref()).map(|a| a.top_radius_m(b.radius) - b.radius);
        let p = air.get(&data, &eph, sim.jd, b.obj, b.radius, top)?;
        (altitude < p.height_m).then(|| {
            let ground = upos.0.delta_m(b.pos).length() - b.radius - altitude;
            (b, p, body_velocity(&eph, b.obj, &sim), body_spin(&eph, b.obj, &sim), ground)
        })
    });
    let aero_at = |pos: UPos, vel: DVec3, rot: DQuat, w: DVec3| {
        let (b, p, bv, spin, ground) = aero_body?;
        let rel = pos.delta_m(b.pos);
        let st = p.state(rel.length() - b.radius - ground);
        let v_air = rot.inverse() * (vel - bv - spin.cross(rel));
        let o = aero_step(&AeroInput {
            model: design.aero,
            length_m: design.length_m,
            mass_kg: design.mass_kg,
            v_air,
            w_se: -w,
            density: st.density,
            sound_speed: st.sound_speed,
            controls: DVec3::ZERO,
        });
        Some((o, st, v_air.length()))
    };
    let aero0 = aero_at(upos.0, ship.vel, urot.0, ship.ang_vel);

    // --- Rotation ------------------------------------------------------------------------
    let alpha = design.turn_engines.max(1e-6) * if input.boost_turn { 10.0 } else { 1.0 };
    let cmd = input.turn;
    if ship.landed.is_none() {
        let mut w = ship.ang_vel;
        if let Some((o, ..)) = aero0 {
            // SE's angular state is the negated physical rate.
            w -= o.ang_accel_se * ship_dt;
        }
        if let Some(t) = attitude {
            // A program turns the ship; the player's turn commands are ignored meanwhile.
            let w0 = w;
            let (nw, _) = autopilot::steer(urot.0, t, w, design.turn_engines.max(1e-6), MAX_TURN_RATE, ship_dt);
            w = nw;
            input.turn = ((nw - w0) / (design.turn_engines.max(1e-6) * ship_dt)).clamp(DVec3::splat(-1.0), DVec3::ONE);
        } else if flight.kill_rotation {
            // ShipStopRotation (FUN_140539700): brake the rotation at the turn engines' rate.
            let step = alpha * ship_dt;
            w = if w.length() <= step { DVec3::ZERO } else { w - w.normalize() * step };
            input.turn = DVec3::from_array(w.to_array().map(|x| if x.abs() > 1e-9 { -x.signum() } else { 0.0 }));
            if w == DVec3::ZERO {
                flight.kill_rotation = false;
            }
        } else {
            // Mouse movement is an impulse of angular velocity (pitch up when moving up).
            w.x += -input.mouse.y * MOUSE_TURN_GAIN * design.turn_engines * if input.boost_turn { 10.0 } else { 1.0 };
            w.y += -input.mouse.x * MOUSE_TURN_GAIN * design.turn_engines * if input.boost_turn { 10.0 } else { 1.0 };
            for k in 0..3 {
                let c = cmd[k];
                if c != 0.0 {
                    w[k] += c * alpha * ship_dt;
                } else if flight.assist || input.brake {
                    let step = alpha * ship_dt;
                    w[k] = if w[k].abs() <= step { 0.0 } else { w[k] - step * w[k].signum() };
                }
            }
        }
        for k in 0..3 {
            w[k] = w[k].clamp(-MAX_TURN_RATE, MAX_TURN_RATE);
        }
        ship.ang_vel = w;
        if w != DVec3::ZERO {
            urot.0 = (urot.0 * DQuat::from_scaled_axis(w * ship_dt)).normalize();
        }
    } else {
        ship.ang_vel = DVec3::ZERO;
    }

    // --- Hold altitude (H) ---------------------------------------------------------------------
    if let (Some(h0), Some(b)) = (flight.hold_altitude, refb) {
        // Vertical speed towards the held altitude, gravity compensated, with the hover engines
        // or RCS (and the main engines when the nose points up or down). Altitude, vertical and
        // velocity frame all of the reference body, as the held altitude (the HUD's).
        let up = upos.0.delta_m(b.pos).normalize_or(DVec3::Y);
        let vz = (ship.vel - ref_vel).dot(up);
        let vz_want = hold_climb_rate(h0, body_altitude(b, upos.0));
        let g_up = if flight.gravity { gravity_at(&infos, upos.0).dot(up) } else { 0.0 };
        let a_up = (vz_want - vz) - g_up;
        let l = urot.0.inverse() * (up * a_up);
        input.rcs.y = (input.rcs.y + l.y / if l.y > 0.0 { up_a } else { corr_a }).clamp(-1.0, 1.0);
        if (urot.0 * DVec3::NEG_Z).dot(up).abs() > 0.7 {
            flight.throttle = if l.z <= 0.0 { (-l.z / design.main_engines.max(1e-6)).min(1.0) } else { -(l.z / retro_a).min(1.0) };
        }
    }

    // --- Thrust ----------------------------------------------------------------------------
    let target_thr = flight.throttle.clamp(-1.0, 1.0);
    let step = ENGINE_SPOOL * ship_dt;
    ship.engine += (target_thr - ship.engine).clamp(-step, step);
    let main_a = if ship.engine >= 0.0 { ship.engine * design.main_engines } else { ship.engine * retro_a };
    let a_local = DVec3::new(
        input.rcs.x * corr_a,
        if input.rcs.y > 0.0 { input.rcs.y * up_a } else { input.rcs.y * corr_a },
        -main_a,
    );
    let mut a_thrust = urot.0 * a_local;

    // --- Pinned to the ground -----------------------------------------------------------
    let mut pos = upos.0;
    let mut vel = ship.vel;
    let mut skip_integration = false;
    if let Some(l) = ship.landed {
        match infos.iter().find(|b| b.entity == l.body) {
            Some(b) => {
                // The ground under the ship may be refined (or fall away) as terrain loads.
                let mut local = l.local_pos;
                let d = local.length();
                if d > 0.0 {
                    let ground = b.terrain.map_or(0.0, |t| t.surface_height(local / d));
                    let min_r = b.radius + ground + ship.clearance;
                    if d < min_r {
                        local *= min_r / d;
                    } else if d > min_r + 2.0 && flight.collisions {
                        ship.landed = None;
                    }
                }
                if let Some(l) = ship.landed.as_mut() {
                    l.local_pos = local;
                }
                pos = b.pos.offset_m(b.orient * local);
                urot.0 = (b.orient * l.local_rot).normalize();
                let rel = pos.delta_m(b.pos);
                vel = body_velocity(&eph, b.obj, &sim) + body_spin(&eph, b.obj, &sim).cross(rel);
                let up = rel.normalize_or(DVec3::Y);
                let g_up = if flight.gravity { gravity_at(&infos, pos).dot(up) } else { 0.0 };
                a_thrust = urot.0 * a_local;
                if a_thrust.dot(up) > -g_up.min(0.0) + 0.5 && a_thrust.dot(up) > 0.0 {
                    ship.landed = None;
                } else {
                    skip_integration = ship.landed.is_some();
                }
            }
            None => ship.landed = None,
        }
    }

    // --- Warp boost ---------------------------------------------------------------------------
    // The warp multiplies the velocity relative to the reference frame, or to the target while
    // the autopilot warps there.
    let warp_frame = match (flight.autopilot, target) {
        (Some(a), Some((t, ..))) if a.program == Program::WarpTo => body_velocity(&eph, t, &sim),
        _ => blend_frames(infos.iter().map(|b| {
            let alt = upos.0.delta_m(b.pos).length() - b.radius;
            // Only bodies within 100 radii have a frame velocity (and are worth an ephemeris lookup).
            let v = if frame_terms(alt, b.radius).1 > 0.0 {
                reference_velocity(&eph, b.obj, b.pos, b.radius, upos.0, &sim)
            } else {
                DVec3::ZERO
            };
            (alt, b.radius, v)
        })),
    };
    let speed = (vel - ref_vel).length().max((vel - warp_frame).length());
    let wmax = design.warp_boost_log;
    let mut warp_target = if flight.warp_on && design.can_warp() { flight.warp_cmd.min(wmax) } else { 0.0 };
    if flight.autopilot.is_some_and(|a| a.program == Program::WarpTo) {
        warp_target = warp_target.min(ap_out.warp_limit);
    }
    if warp_target > 0.0 && altitude.is_finite() {
        // The executable keeps the warp from crossing the remaining altitude in under two frames.
        let radius = nearest.map(|b| b.radius).unwrap_or(1.0);
        if altitude < 0.01 * radius {
            warp_target = 0.0;
        } else if speed > 1e-6 {
            let t_cross = altitude / (speed * 10f64.powf(warp_target));
            if t_cross < 2.0 * ship_dt {
                warp_target = (altitude / (speed * 2.0 * ship_dt)).log10().clamp(0.0, warp_target);
            }
        }
    }
    let step = WARP_RATE * ship_dt;
    ship.warp += (warp_target - ship.warp).clamp(-step, step);
    if ship.warp < 1e-3 && warp_target == 0.0 {
        ship.warp = 0.0;
    }
    let mut boost = 10f64.powf(ship.warp);
    if boost > 1.0 && altitude.is_finite() && speed > 1e-9 {
        // Never step more than half the remaining altitude, however far the boost still has to ramp down.
        boost = boost.min((altitude.max(0.0) / (2.0 * speed * ship_dt)).max(1.0));
    }
    if flight.autopilot.is_some_and(|a| a.program == Program::WarpTo) {
        // The same for the distance left to the autopilot's stop point.
        boost = boost.min(10f64.powf(ap_out.warp_limit.max(0.0)));
    }

    // --- Integration -----------------------------------------------------------------------------
    let sync = flight.sync_velocity || input.brake;
    let mut aero_accel = DVec3::ZERO;
    if !skip_integration {
        let n = ((ship_dt / 0.05).ceil() as usize).clamp(1, 32);
        let h = ship_dt / n as f64;
        for _ in 0..n {
            let mut a = a_thrust;
            if flight.gravity {
                a += gravity_at(&infos, pos);
            }
            if boost <= 1.0 {
                if let Some((o, _, v_air)) = aero_at(pos, vel, urot.0, ship.ang_vel) {
                    // Drag never reverses the air velocity within a step.
                    let mut aa = urot.0 * o.accel;
                    if aa.length() * h > 0.5 * v_air {
                        aa *= 0.5 * v_air / (aa.length() * h);
                    }
                    aero_accel = aa;
                    a += aa;
                }
            }
            if sync {
                let vr = vel - ref_vel;
                let s = vr.length();
                let cap = retro_a.max(corr_a).max(design.main_engines * 0.5);
                if s > 0.0 {
                    let want = (s / h).min(cap);
                    a -= vr / s * want;
                }
            }
            vel += a * h;
            if ship.warp == 0.0 && vel.length() > SPEED_OF_LIGHT {
                vel *= SPEED_OF_LIGHT / vel.length();
            }
            pos = pos.offset_m((warp_frame + (vel - warp_frame) * boost) * h);
        }
    }
    if flight.sync_velocity && (vel - ref_vel).length() < 0.05 {
        flight.sync_velocity = false;
    }

    // --- Ground contact -----------------------------------------------------------------------------
    let mut notice: Option<String> = None;
    if flight.collisions && ship.landed.is_none() {
        for b in &infos {
            let rel = pos.delta_m(b.pos);
            let d = rel.length();
            let top = b.terrain.map_or(b.radius, |t| t.top_radius().max(b.radius));
            if d > top * 1.2 + 20_000.0 + ship.clearance || d <= 0.0 {
                continue;
            }
            let ground = ground_at(b, rel);
            let min_r = b.radius + ground + ship.clearance;
            if d >= min_r {
                continue;
            }
            let up = rel / d;
            pos = b.pos.offset_m(up * min_r);
            let rel = up * min_r;
            let surf_vel = body_velocity(&eph, b.obj, &sim) + body_spin(&eph, b.obj, &sim).cross(rel);
            let mut v_rel = vel - surf_vel;
            let vn = v_rel.dot(up);
            if vn < 0.0 {
                if -vn >= CRASH_SPEED {
                    notice = Some(format!("CRASH: impact at {}", format_speed(-vn)));
                }
                v_rel -= up * vn;
            }
            v_rel *= (-3.0 * ship_dt).exp();
            let lifting = a_thrust.dot(up) > 0.5;
            vel = surf_vel + v_rel;
            if v_rel.length() < 0.5 && !lifting {
                vel = surf_vel;
                ship.landed = Some(Landed {
                    body: b.entity,
                    local_pos: b.orient.inverse() * rel,
                    local_rot: b.orient.inverse() * urot.0,
                });
                notice = notice.or(Some("Landed".to_string()));
            }
            break;
        }
    }

    ship.vel = vel;
    upos.0 = pos;
    flight.input.rcs = input.rcs;
    flight.input.turn = input.turn;

    // --- HUD numbers -------------------------------------------------------------------------------
    // Recomputed at the final position: the bodies have moved since the start of the frame.
    let (_, _, near_name, altitude_end) = nearest_state(&infos, &eph, &sim, &data, pos);
    let refb_end = reference_body(&infos, flight.ref_mode, flight.ref_body, pos).or(nearest);
    let ref_vel_end = refb_end.map_or(DVec3::ZERO, |b| reference_velocity(&eph, b.obj, b.pos, b.radius, pos, &sim));
    let speed_rel = (vel - ref_vel_end).length();
    let g_vec = gravity_at(&infos, pos);
    flight.info.speed_rel = speed_rel;
    flight.info.speed_eff = speed_rel * boost;
    flight.info.nearest = refb_end.map_or(near_name, |b| data.catalog.display_name(b.obj).to_string());
    flight.info.altitude = refb_end.map_or(altitude_end, |b| body_altitude(b, pos));
    flight.info.g = g_vec.length();
    flight.info.aero = match aero0 {
        Some((o, st, v_air)) => AeroInfo {
            active: true,
            air_speed: v_air,
            mach: o.mach,
            q: o.q,
            alpha: o.alpha,
            beta: o.beta,
            accel: aero_accel.length(),
            density: st.density,
            sound_speed: st.sound_speed,
        },
        None => AeroInfo::default(),
    };
    if let Some(n) = notice {
        flight.info.notice = n;
        flight.info.notice_until = now + 4.0;
    } else if ship.landed.is_some() && flight.info.notice_until < now {
        flight.info.notice = "Landed".to_string();
        flight.info.notice_until = now + 0.5;
    }
}

/// The reference body for `mode` (SE's ShipSelectRefBody): the nearest surface, the strongest
/// gravity, or the picked body when it is in the system.
fn reference_body<'a>(infos: &'a [BodyInfo<'a>], mode: RefMode, picked: Option<ObjectRef>, pos: UPos) -> Option<&'a BodyInfo<'a>> {
    match mode {
        RefMode::Nearest => None,
        RefMode::Influence => infos.iter().max_by(|a, b| {
            let g = |x: &BodyInfo| x.gm / pos.delta_m(x.pos).length_squared().max(1.0);
            g(a).total_cmp(&g(b))
        }),
        RefMode::Selected => picked.and_then(|o| infos.iter().find(|b| b.obj == o)),
    }
}

/// The body with the nearest surface to `pos`, the velocity of the frame the ship is at rest
/// in there, its name, and the altitude above its (terrain) surface.
fn nearest_state<'a>(
    infos: &'a [BodyInfo<'a>],
    eph: &Ephemeris,
    sim: &SimTime,
    data: &SeData,
    pos: UPos,
) -> (Option<&'a BodyInfo<'a>>, DVec3, String, f64) {
    let nearest = infos
        .iter()
        .min_by(|a, b| (a.pos.delta_m(pos).length() - a.radius).total_cmp(&(b.pos.delta_m(pos).length() - b.radius)));
    match nearest {
        Some(b) => {
            let rel = pos.delta_m(b.pos);
            let altitude = rel.length() - b.radius - ground_at(b, rel);
            (
                nearest,
                reference_velocity(eph, b.obj, b.pos, b.radius, pos, sim),
                data.catalog.display_name(b.obj).to_string(),
                altitude,
            )
        }
        None => (None, DVec3::ZERO, String::new(), f64::INFINITY),
    }
}

/// Altitude of `pos` over the (terrain) surface of `b`.
fn body_altitude(b: &BodyInfo, pos: UPos) -> f64 {
    let rel = pos.delta_m(b.pos);
    rel.length() - b.radius - ground_at(b, rel)
}

/// Vertical speed (m/s) Hold altitude asks for at altitude `h` to come back to `h0`.
fn hold_climb_rate(h0: f64, h: f64) -> f64 {
    ((h0 - h) * 0.5).clamp(-100.0, 100.0)
}

/// Height of the rendered surface of `b` at the given position relative to its centre.
fn ground_at(b: &BodyInfo, rel: DVec3) -> f64 {
    let d = rel.length();
    if d <= 0.0 {
        return 0.0;
    }
    b.terrain.map_or(0.0, |t| t.surface_height((b.orient.inverse() * rel) / d))
}

// ------------------------------------------------------------------------------------
// Camera
// ------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn ship_camera(
    flight: Res<Flight>,
    ships: Query<(&Ship, &UPosition, &UOrientation)>,
    bodies: Query<(&SystemBody, &Terrain)>,
    mut cam: ResMut<FlyCamera>,
) {
    if !flight.active {
        return;
    }
    let Some((ship, pos, rot)) = flight.ship.and_then(|e| ships.get(e).ok()) else { return };
    let design = &ship.design;
    let chase = design.chase_cameras();
    let mode = flight.cam_mode.min(chase.len());
    let (cp, cr) = if mode < chase.len() {
        let c = &chase[mode];
        let local = c.pos * flight.zoom;
        (pos.0.offset_m(rot.0 * local), (rot.0 * c.rot).normalize())
    } else {
        let dir = DQuat::from_rotation_y(flight.orbit_yaw) * DQuat::from_rotation_x(-flight.orbit_pitch) * DVec3::Z;
        let world_dir = rot.0 * dir;
        let p = pos.0.offset_m(world_dir * flight.orbit_dist);
        (p, look_rotation(-world_dir, rot.0 * DVec3::Y))
    };
    cam.pos = cp;
    cam.rot = cr;
    cam.velocity = DVec3::ZERO;
    cam.travel = None;
    cam.follow = None;
    // Keep the camera out of the terrain.
    for (body, terrain) in &bodies {
        let rel = cam.pos.delta_m(body.pos);
        let d = rel.length();
        if d > terrain.top_radius().max(body.radius_m) * 1.2 || d <= 0.0 {
            continue;
        }
        let ground = terrain.surface_height((body.orient.inverse() * rel) / d);
        let min_r = body.radius_m + ground + 2.0;
        if d < min_r {
            cam.pos = body.pos.offset_m(rel / d * min_r);
        }
    }
}

/// The ship is usually far closer to the camera than the nearest body is to the surface;
/// pull the near plane in so the hull is not clipped.
fn ship_near_plane(
    flight: Res<Flight>,
    ships: Query<(&Ship, &UPosition)>,
    cam: Res<FlyCamera>,
    mut proj: Query<&mut Projection, With<MainCamera>>,
) {
    if !flight.active {
        return;
    }
    let Some((ship, pos)) = flight.ship.and_then(|e| ships.get(e).ok()) else { return };
    let d = pos.0.delta_m(cam.pos).length();
    let near = (d * 0.01).clamp(0.02, 50.0).min((ship.radius * 0.02).max(0.05)) as f32;
    for mut p in &mut proj {
        if let Projection::Perspective(p) = p.as_mut() {
            p.near = p.near.min(near.max(0.02));
        }
    }
}

// ------------------------------------------------------------------------------------
// Lighting and plumes
// ------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn update_ship_visuals(
    time: Res<Time>,
    data: Res<SeData>,
    flight: Res<Flight>,
    fade: Res<crate::system::LightingFade>,
    ambient: Res<crate::system::AmbientLighting>,
    star_settings: Res<crate::stars::StarSettings>,
    ships: Query<(&Ship, &UPosition, &UOrientation, &ShipVisuals)>,
    bodies: Query<(&SystemBody, Option<&crate::terrain::Terrain>)>,
    mut hull_mats: ResMut<Assets<ShipSurfaceMaterial>>,
    mut plume_mats: ResMut<Assets<PlumeMaterial>>,
    mut tfs: Query<(&mut Transform, &mut Visibility)>,
) {
    let cat = &data.catalog;
    for (ship, pos, _rot, vis) in &ships {
        // SE lights a ship like a body (FUN_1404bf5c0, called for ships with kind 7 from
        // FUN_1401f2b10): its up to MAX_LIGHTS stars of the most flux, faded and normalised to the
        // brightest, each dimmed by the bodies eclipsing it (ship.glsl ECL).
        let mut stars: Vec<(f64, &SystemBody, Vec3)> = Vec::new();
        for (b, _) in bodies.iter().filter(|b| b.0.star) {
            let ObjectRef::Star(i) = b.obj else { continue };
            let s = &cat.stars[i as usize];
            let d = b.pos.delta_m(pos.0).length().max(1.0);
            let flux = se_core::star::lum_from_abs_mag(s.abs_mag) / (d / METERS_PER_AU).powi(2);
            let c = Vec3::from_array(crate::stars::star_render_color(s.class.kind, s.teff));
            stars.push((flux, b, crate::system::illum_color(c, s.teff as f32, &star_settings)));
        }
        stars.sort_by(|a, b| b.0.total_cmp(&a.0));
        stars.truncate(4);
        let k = fade.planet_intensities(&stars.iter().map(|s| (s.0, true)).collect::<Vec<_>>());
        let (mut dirs, mut colors) = ([Vec4::ZERO; 4], [Vec4::ZERO; 4]);
        for (i, ((_, b, c), k)) in stars.iter().zip(&k).enumerate() {
            let shadow = crate::system::sun_sprite_tint(b, pos.0, &bodies);
            dirs[i] = b.pos.delta_m(pos.0).normalize_or(DVec3::Y).as_vec3().extend(1.0);
            colors[i] = (*c * *k * shadow).extend(1.0);
        }
        for h in &vis.hull {
            if let Some(mut m) = hull_mats.get_mut(h) {
                (m.u.light_dir, m.u.light_color, m.u.tint.w) = (dirs, colors, ambient.0);
            }
        }

        // Plume levels from the commands.
        let t = time.elapsed_secs();
        for p in &vis.plumes {
            let level = plume_level(&p.spec, ship, &flight.input);
            let show = level > 0.02;
            if let Ok((mut tf, mut v)) = tfs.get_mut(p.entity) {
                let want = if show { Visibility::Inherited } else { Visibility::Hidden };
                if *v != want {
                    *v = want;
                }
                if show {
                    let l = level.powf(0.8);
                    tf.scale = Vec3::new(p.base_scale.x * (0.5 + 0.5 * l), p.base_scale.y * (0.5 + 0.5 * l), p.base_scale.z * l);
                }
            }
            if show || p.visible {
                if let Some(mut m) = plume_mats.get_mut(&p.material) {
                    m.u.params.x = level;
                    m.u.params.y = t;
                }
            }
        }
    }
}

/// Level (0..1) of an engine effect: its plume size and the loudness of its sound. Main and
/// retro follow the spooled engine, hover the upward RCS command, warp effects the boost, and a
/// thruster fires when its push helps the translation or turn command.
fn plume_level(s: &PlumeSpec, ship: &Ship, input: &ShipInput) -> f32 {
    let engine = ship.engine;
    let rcs = input.rcs;
    let wfrac = if ship.design.warp_boost_log > 0.0 { (ship.warp / ship.design.warp_boost_log) as f32 } else { 0.0 };
    match s.group {
        EffectGroup::Main => engine.max(0.0) as f32,
        EffectGroup::Retro => (-engine).max(0.0) as f32,
        EffectGroup::Hover => rcs.y.max(0.0) as f32,
        EffectGroup::Hyper => wfrac,
        EffectGroup::Thruster => {
            // The nozzle pushes the ship along -dir; it fires when that helps a command.
            let push = -s.dir.as_dvec3();
            let trans = push.dot(DVec3::new(rcs.x, rcs.y, 0.0).normalize_or(DVec3::ZERO)).max(0.0);
            let torque = s.pos.as_dvec3().cross(push);
            let rot_cmd = input.turn;
            let rot = if torque.length() > 1e-6 && rot_cmd.length() > 0.0 { torque.normalize().dot(rot_cmd.normalize()).max(0.0) } else { 0.0 };
            (trans + rot * 0.8) as f32
        }
        EffectGroup::Other => 0.0,
    }
    .clamp(0.0, 1.0)
}

// ------------------------------------------------------------------------------------
// Interface
// ------------------------------------------------------------------------------------

fn draw_ship_window(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<ShipUi>,
    lib: Res<ShipLibrary>,
    mut flight: ResMut<Flight>,
    hud: Res<crate::hud::HudState>,
    mut writer: MessageWriter<ShipCommand>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if hud.hidden {
        return Ok(());
    }
    egui::Area::new(egui::Id::new("ships_button"))
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 36.0])
        .show(ctx, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if ui.button(if flight.active { "Ships (flying)" } else { "Ships" }).clicked() {
                ui_state.show = !ui_state.show;
            }
        });
    if !ui_state.show {
        return Ok(());
    }
    let mut open = true;
    egui::Window::new(crate::skin::title(ctx, "Ships")).open(&mut open).default_width(340.0).default_pos([320.0, 60.0]).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.text_edit_singleline(&mut ui_state.filter);
        });
        let filter = ui_state.filter.to_ascii_lowercase();
        let mut chosen: Option<usize> = None;
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for (i, d) in lib.designs.iter().enumerate() {
                if !filter.is_empty() && !d.name.to_ascii_lowercase().contains(&filter) {
                    continue;
                }
                ui.horizontal(|ui| {
                    if ui.selectable_label(ui_state.selected == i, &d.name).clicked() {
                        chosen = Some(i);
                    }
                    ui.weak(format!("{}  {} m", d.class, d.length_m.round()));
                });
            }
        });
        if let Some(i) = chosen {
            ui_state.selected = i;
        }
        if let Some(d) = lib.designs.get(ui_state.selected) {
            ui.separator();
            ui.heading(&d.name);
            ui.label(format!("{} | {} | {}", d.class, d.faction, d.pack));
            ui.label(format!("length {} m, mass {:.3e} kg", d.length_m.round(), d.mass_kg));
            ui.label(format!(
                "main {:.1}  retro {:.1}  hover {:.1}  RCS {:.1} m/s²",
                d.main_engines, d.retro_engines, d.hover_engines, d.corr_engines
            ));
            ui.label(format!(
                "turn {:.4} rad/s²  warp up to 10^{:.0}{}",
                d.turn_engines,
                d.warp_boost_log,
                if d.can_warp() { "" } else { " (no warp drive)" }
            ));
            ui.label(format!("aerodynamic model: {}", d.aero.name()));
            ui.horizontal(|ui| {
                if ui.button("Fly").clicked() {
                    writer.write(ShipCommand { verb: "ship".into(), arg: d.id.clone() });
                }
                if flight.ship.is_some() {
                    if ui.button(if flight.active { "Leave (Esc)" } else { "Enter (F)" }).clicked() {
                        writer.write(ShipCommand {
                            verb: "flight".into(),
                            arg: if flight.active { "off".into() } else { "on".into() },
                        });
                    }
                    if ui.button("Remove ship").clicked() {
                        writer.write(ShipCommand { verb: "unship".into(), arg: String::new() });
                    }
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut flight.gravity, "Gravity");
            ui.checkbox(&mut flight.collisions, "Collisions");
            ui.checkbox(&mut flight.assist, "Flight assist");
            ui.checkbox(&mut flight.aero, "Aerodynamics");
        });
        ui.weak("W/S throttle, A/D/R/F RCS, arrows + Q/E turn, V camera, Alt+= / Alt+- warp, Esc leaves");
        ui.weak("5-0 / End holds, Num5 kill rotation, T target, P reference body, C / G / Shift+G / Alt+G autopilot");
    });
    if !open {
        ui_state.show = false;
    }
    Ok(())
}

fn draw_flight_hud(
    mut contexts: EguiContexts,
    flight: Res<Flight>,
    time: Res<Time>,
    ships: Query<&Ship>,
    hud: Res<crate::hud::HudState>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !flight.active || hud.hidden {
        return Ok(());
    }
    let Some(ship) = flight.ship.and_then(|e| ships.get(e).ok()) else { return Ok(()) };
    let d = &ship.design;
    let info = &flight.info;
    let n_chase = d.chase_cameras().len();
    let mode = flight.cam_mode.min(n_chase);
    let cam_name = if mode < n_chase { format!("chase {}/{}", mode + 1, n_chase) } else { "orbit".to_string() };
    egui::Area::new(egui::Id::new("flight_hud"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -14.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new().fill(egui::Color32::from_black_alpha(150)).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(format!("{}  ({})", d.name, d.class)).strong().size(16.0));
                    let speed = if ship.warp > 0.05 {
                        format!("{}  (warp {})", format_speed(info.speed_eff), format_speed(info.speed_rel))
                    } else {
                        format_speed(info.speed_rel)
                    };
                    ui.label(egui::RichText::new(format!("speed {speed}")).monospace());
                    let warp = if !d.can_warp() {
                        "no warp drive".to_string()
                    } else if flight.warp_on || ship.warp > 0.05 {
                        format!("warp ON 10^{:.1} (set {:.0}, max {:.0})", ship.warp, flight.warp_cmd, d.warp_boost_log)
                    } else {
                        format!("warp off (set {:.0}, max {:.0})", flight.warp_cmd, d.warp_boost_log)
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "throttle {:+.0}% (engine {:+.0}%)   {}",
                            flight.throttle * 100.0,
                            ship.engine * 100.0,
                            warp
                        ))
                        .monospace(),
                    );
                    let alt = if info.altitude.is_finite() { format_distance(info.altitude) } else { "-".to_string() };
                    ui.label(
                        egui::RichText::new(format!(
                            "{} ({}): altitude {}   g {:.3} m/s²",
                            info.nearest,
                            flight.ref_mode.label(),
                            alt,
                            info.g
                        ))
                        .monospace(),
                    );
                    if !info.target.is_empty() {
                        let dist = if info.target_distance.is_finite() { format_distance(info.target_distance) } else { "-".to_string() };
                        ui.label(egui::RichText::new(format!("target {}: distance {}", info.target, dist)).monospace());
                    }
                    let mut programs = Vec::new();
                    if let Some(ap) = flight.autopilot {
                        programs.push(format!("Autopilot: {} - {}", ap.program.label(), ap.stage.label()));
                    }
                    if flight.attitude != Attitude::Free {
                        programs.push(format!("Orientation: {}", flight.attitude.label()));
                    }
                    if flight.kill_rotation {
                        programs.push("Kill rotation".to_string());
                    }
                    if flight.sync_velocity {
                        programs.push("Sync velocity".to_string());
                    }
                    if let Some(h) = flight.hold_altitude {
                        programs.push(format!("Hold altitude {}", format_distance(h)));
                    }
                    if !programs.is_empty() {
                        ui.label(egui::RichText::new(programs.join("   ")).color(egui::Color32::from_rgb(140, 220, 255)).monospace());
                    }
                    let a = &info.aero;
                    if a.active {
                        ui.label(
                            egui::RichText::new(format!(
                                "air {}  Mach {:.2}  dyn. pressure {:.2} kPa  attack {:+.1}°  sideslip {:+.1}°  aero accel {:.2} m/s²",
                                format_speed(a.air_speed),
                                a.mach,
                                a.q * 1e-3,
                                a.alpha.to_degrees(),
                                a.beta.to_degrees(),
                                a.accel
                            ))
                            .monospace(),
                        );
                        ui.weak(format!(
                            "air density {:.4} kg/m³  speed of sound {}  model {}",
                            a.density,
                            format_speed(a.sound_speed),
                            d.aero.name()
                        ));
                    }
                    if info.notice_until > time.elapsed_secs_f64() && !info.notice.is_empty() {
                        ui.label(egui::RichText::new(&info.notice).color(egui::Color32::from_rgb(255, 200, 80)).strong());
                    }
                    ui.weak(format!(
                        "W/S throttle  Z stop  A/D/R/F RCS  arrows/Q/E turn  U assist {}  V camera [{}]  Alt+=/- warp  Alt+Z stop warp  Esc leave",
                        if flight.assist { "on" } else { "off" },
                        cam_name
                    ));
                    ui.weak("5-0 End holds  Num5 kill rot.  H hold alt.  Shift+Z sync vel.  T target  P ref body  C rotate  G go  Shift+G fly  Alt+G warp");
                });
            });
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_points_to_the_body_and_follows_inverse_square() {
        let mk = |x: f64| BodyInfo {
            entity: Entity::PLACEHOLDER,
            obj: ObjectRef::Body(0),
            pos: UPos::from_meters(DVec3::new(x, 0.0, 0.0)),
            orient: DQuat::IDENTITY,
            radius: 6.371e6,
            gm: G * 5.972e24,
            terrain: None,
        };
        let b = [mk(0.0)];
        let g1 = gravity_at(&b, UPos::from_meters(DVec3::new(6.371e6, 0.0, 0.0)));
        // Surface gravity of the Earth, pointing at the centre (-X).
        assert!((g1.x + 9.82).abs() < 0.05, "{g1:?}");
        let g2 = gravity_at(&b, UPos::from_meters(DVec3::new(2.0 * 6.371e6, 0.0, 0.0)));
        assert!((g2.x / g1.x - 0.25).abs() < 1e-6);
        // Inside: linear fall-off.
        let g3 = gravity_at(&b, UPos::from_meters(DVec3::new(0.5 * 6.371e6, 0.0, 0.0)));
        assert!((g3.x / g1.x - 0.5).abs() < 1e-6);
    }

    #[test]
    fn design_lookup_by_id_or_name() {
        let d = |id: &str, name: &str| Arc::new(ShipDesign { id: id.into(), name: name.into(), ..Default::default() });
        let lib = ShipLibrary { designs: vec![d("DragonFly", "Fire Dragonfly"), d("Shuttle", "Shuttle")] };
        assert_eq!(find_design(&lib, "dragonfly").unwrap().id, "DragonFly");
        assert_eq!(find_design(&lib, "Fire Dragonfly").unwrap().id, "DragonFly");
        assert_eq!(find_design(&lib, "shut").unwrap().id, "Shuttle");
        assert!(find_design(&lib, "nope").is_none());
        assert!(find_design(&lib, "").is_none());
    }
    #[test]
    fn warp_frame_is_continuous() {
        // Orbital frame in full below 50 radii, none beyond 100; rotation to one radius, gone by two.
        assert_eq!(frame_terms(40.0, 1.0).1, 1.0);
        assert!((frame_terms(75.0, 1.0).1 - 0.5).abs() < 1e-12);
        assert_eq!(frame_terms(100.0, 1.0).1, 0.0);
        assert_eq!(frame_terms(0.5, 1.0).2, 1.0);
        assert_eq!(frame_terms(2.0, 1.0).2, 0.0);
        // Crossing from one body to another 1000 km away the blended frame never jumps.
        let (va, vb, r, gap) = (DVec3::new(0.0, 1000.0, 0.0), DVec3::new(0.0, -1000.0, 0.0), 1000.0, 1.0e6);
        let mut last: Option<DVec3> = None;
        for k in 0..=995 {
            let x = 2000.0 + k as f64 * 1000.0;
            let v = blend_frames([(x - r, r, va), (gap - x - r, r, vb)]);
            if let Some(l) = last {
                assert!((v - l).length() < 20.0, "{k}: {l:?} -> {v:?}");
            }
            last = Some(v);
        }
        // Each body's frame rules close to it.
        assert!((blend_frames([(10.0, r, va), (1e5, r, vb)]) - va).length() < 1.0);
    }

    #[test]
    fn hold_altitude_measures_from_the_reference_body() {
        let b = BodyInfo {
            entity: Entity::PLACEHOLDER,
            obj: ObjectRef::Body(0),
            pos: UPos::from_meters(DVec3::new(1e7, 0.0, 0.0)),
            orient: DQuat::IDENTITY,
            radius: 6.371e6,
            gm: 0.0,
            terrain: None,
        };
        let p = UPos::from_meters(DVec3::new(1e7 + 6.371e6 + 5000.0, 0.0, 0.0));
        assert!((body_altitude(&b, p) - 5000.0).abs() < 1e-6);
        assert_eq!(hold_climb_rate(5000.0, body_altitude(&b, p)), 0.0);
        assert_eq!(hold_climb_rate(5000.0, 4990.0), 5.0);
        assert_eq!(hold_climb_rate(5000.0, 9000.0), -100.0);
    }

    #[test]
    fn leaving_the_seat_and_stopping_the_warp_end_the_programs() {
        let mut f = Flight { autopilot: Some(Autopilot::new(Program::WarpTo)), warp_on: true, throttle: 1.0, ..default() };
        stop_warp_drive(&mut f);
        assert!(!f.warp_on && f.autopilot.is_none() && f.throttle == 0.0);
        // Fly to target uses no warp: Alt+Z leaves it running.
        f.autopilot = Some(Autopilot::new(Program::FlyTo));
        stop_warp_drive(&mut f);
        assert!(f.autopilot.is_some());
        f.attitude = Attitude::Prograde;
        f.hold_altitude = Some(1000.0);
        f.sync_velocity = true;
        f.throttle = 1.0;
        stop_programs(&mut f);
        assert!(f.autopilot.is_none() && f.attitude == Attitude::Free && f.hold_altitude.is_none() && !f.sync_velocity);
        assert_eq!(f.throttle, 0.0);
    }

    #[test]
    fn thrusters_follow_the_turn_command_the_programs_write() {
        let ship = Ship {
            design: Arc::new(ShipDesign::default()),
            vel: DVec3::ZERO,
            ang_vel: DVec3::ZERO,
            engine: 0.0,
            warp: 0.0,
            clearance: 1.0,
            radius: 1.0,
            landed: None,
        };
        // A nose thruster blowing to +x pushes the nose left: it fires for a left yaw.
        let spec = PlumeSpec {
            group: EffectGroup::Thruster,
            kind: "Ion".into(),
            color: [1.0; 3],
            pos: Vec3::new(5.0, 0.0, -10.0),
            dir: Vec3::X,
            x_axis: Vec3::Y,
            size: Vec3::ONE,
            brightness: 1.0,
            sound: None,
            volume: 0.02,
        };
        let yaw = |y: f64| ShipInput { turn: DVec3::new(0.0, y, 0.0), ..default() };
        assert!((plume_level(&spec, &ship, &yaw(1.0)) - 0.8).abs() < 1e-6);
        assert_eq!(plume_level(&spec, &ship, &yaw(-1.0)), 0.0);
    }
}
