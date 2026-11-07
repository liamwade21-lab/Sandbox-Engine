//! Free-flight camera with SpaceEngine-style controls.
//!
//! The camera's true position is a [`UPos`]; the Bevy camera entity always sits
//! at the render origin and only carries the orientation (floating origin).
//!
//! Controls (SpaceEngine's default binds): W/S (Up/Down) forward/back, A/D (Left/Right) strafe,
//! R/F (Numpad 1/0) up/down, Q/E (Numpad 7/9) roll, Numpad 4/6/8/2 turn, Numpad 5 stops turning,
//! Z stops moving. Left-drag looks around, right-drag orbits the followed (or selected) object,
//! left+right drag zooms towards it, Shift+left drag changes the FOV, middle click zooms to
//! `ZoomFOV` and back, PgUp/PgDn zoom while held, Home resets the zoom. 1/2/3 switch the movement
//! mode (Free, Spacecraft, Aircraft: [`MoveMode`]), 4 flies the current spacecraft. The mouse wheel or +/- change the
//! assigned velocity (Ctrl: next velocity preset). G goes to the selection (Ctrl+G to its centre,
//! Shift+G lands on it), C centres it, End levels the view to its horizon, T tracks it, Ctrl+Numpad
//! 5/4/6/8/2 look back/left/right/up/down. Shift+D / Shift+F / Shift+R: free, follow or rotate with
//! the selection (SE's BindMode); 5 toggles the auto horizon. Backspace selects the previous
//! object, Shift+H cycles the home address.

use std::collections::VecDeque;

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::render::view::ColorGrading;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::EguiContexts;
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::units::{METERS_PER_AU, METERS_PER_PC};
use se_core::upos::UPos;

use crate::config::EngineConfig;
use crate::data::SeData;
use crate::universe::{ScaledSpace, SimTime, UniverseSet};

/// The observer.
#[derive(Resource, Debug, Clone)]
pub struct FlyCamera {
    pub pos: UPos,
    pub rot: DQuat,
    /// Current velocity in m/s (universe frame).
    pub velocity: DVec3,
    /// Assigned velocity (m/s): the speed of keyboard flight, changed with the wheel.
    pub speed: f64,
    pub fov_deg: f32,
    pub mouse_sensitivity: f64,
    /// Selected object.
    pub selected: Option<ObjectRef>,
    /// Active "go to" flight.
    pub travel: Option<Travel>,
    /// Object whose motion (and, close in, rotation) the camera follows.
    pub follow: Option<Follow>,
    /// Distance to the nearest surface (m), for auto speed and the HUD.
    pub nearest_surface: f64,
    /// Distance to the edge of the nearest galaxy we are outside of (m), part of `nearest_surface`.
    pub nearest_galaxy: f64,
    pub nearest_name: String,
    /// Depth under an ocean's surface (m), 0 above it.
    pub depth_m: f64,
    /// Dev script `agl M`: keep the camera this far above the ground under it (m).
    pub hold_agl: Option<f64>,
    /// How far `keep_camera_above_ground` pushed it out of the ground this frame (m, universe axes): the Viltrumite mod's hero
    /// mustn't take that for a jump of the camera by something else.
    pub floor_push: DVec3,
    /// Dev script `shore [M]`: move to the nearest coast, M metres inland, facing the sea.
    pub find_shore: Option<f64>,
    /// SE's `AutoHorizon` (key 5): see [`horizon_up`].
    pub auto_horizon: bool,
    /// Whether the auto horizon held the view this frame (for the HUD).
    pub horizon_active: bool,
    /// SE's Track object (T): keep the view turned to the selection.
    pub track: bool,
    /// Keyboard turning (rad/s about the camera's x, y, z), ramped like the movement keys.
    pub turn_vel: DVec3,
    /// FOV being zoomed to over `ZoomTime`.
    pub fov_target: Option<f32>,
    /// Earlier selections, for Backspace (SE's SelectPrevObject).
    pub history: Vec<ObjectRef>,
    /// Script input locks (`UserMoveControl`, `UserRotationControl`).
    pub locks: InputLocks,
    /// SE's movement mode (keys 1-3).
    pub move_mode: MoveMode,
    /// Spacecraft / Aircraft mode: the movement keys' thrust (camera frame, each axis -1..1),
    /// ramped in over MoveKeyAccel and out over MoveKeyDecel.
    pub thrust: DVec3,
    /// FOV animation (SE's camera +0x1468 / +0x1470 / +0x1478 / +0x1480, FUN_14056ad80): from,
    /// to and the time it has run.
    pub zoom_anim: Option<ZoomAnim>,
    /// Counts the travels started (Go to, Center, Land, Horizon) and the binds made with
    /// [`Self::bind`] (Shift+F / Shift+R / Shift+D, the toolbar), so a script trigger can tell a new
    /// one from one already under way, or a binding already there, when it began waiting.
    pub travel_serial: u32,
    pub bind_serial: u32,
}

/// One FOV animation over `ZoomTime`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomAnim {
    pub from: f32,
    pub to: f32,
    pub elapsed: f64,
}

/// SE's script input locks (FUN_140552c30 sets DAT_1419d5ae6, FUN_140552900 DAT_1419d5ae5): while
/// `movement` is set the user cannot move the camera (movement keys, mouse orbit and zoom, Go to,
/// Land, Follow / Rotate with / Free binding: FUN_140595be0, FUN_1405986f0, FUN_140597720), and
/// while `rotation` is set not turn it (mouse look, turn keys, Center, Horizon, Track). Script
/// commands still move it. `UserMoveControl "Limited"` sets the same flag (FUN_140552c30:414) with
/// limits instead of a full stop: the keys and mouse still move the camera inside them, but Go to,
/// Land and binding are refused (`limited`). The scripting runner sets them and clears them when a
/// script ends (FUN_140221b10).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InputLocks {
    pub movement: bool,
    pub rotation: bool,
    pub limited: bool,
}

impl InputLocks {
    /// SE's movement flag: Go to, Land, binding and a script's Goto / Follow / Free are refused.
    pub fn travel(&self) -> bool {
        self.movement || self.limited
    }
}

/// SE's camera movement mode (`MoveMode`, camera +0x140c, DAT_1419d5adc; keys 1-3 in the key table
/// FUN_1405ba8f0, MoveModeFree..MoveModeAircraft; script `MoveMode`). SE's camera update FUN_140576090
/// only differs in how the movement keys act: in Free mode they set the velocity (the camera stops
/// when they are let go), in Spacecraft and Aircraft mode they give an acceleration and the velocity
/// stays (inertia). The Spacecraft velocity is kept in the universe frame, the Aircraft one in the
/// camera's (FUN_14057a740 turns it between the two on a switch), so an aircraft's flight turns
/// with its nose. Turning, roll and the auto horizon are the same in all three. `MoveMode 4` (key 4,
/// FUN_1405b9420) is piloting a spacecraft: the port's ship flight (`ship.rs`); SE then sets Free
/// mode (FUN_1401f6aa0).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MoveMode {
    #[default]
    Free,
    Spacecraft,
    Aircraft,
}

impl MoveMode {
    pub const ALL: [MoveMode; 3] = [MoveMode::Free, MoveMode::Spacecraft, MoveMode::Aircraft];

    /// SE's number (save.cfg and script `MoveMode`).
    pub fn number(self) -> u8 {
        match self {
            MoveMode::Free => 1,
            MoveMode::Spacecraft => 2,
            MoveMode::Aircraft => 3,
        }
    }

    pub fn from_number(n: u8) -> Option<MoveMode> {
        MoveMode::ALL.into_iter().find(|m| m.number() == n)
    }

    /// The splash on a switch (eng-gui "Free mode" / "Spacecraft mode" / "Aircraft mode").
    pub fn label(self) -> &'static str {
        match self {
            MoveMode::Free => "Free mode",
            MoveMode::Spacecraft => "Spacecraft mode",
            MoveMode::Aircraft => "Aircraft mode",
        }
    }

    /// Whether the assigned value is an acceleration (the toolbar's "Acceleration", FUN_1405b94a0).
    pub fn inertial(self) -> bool {
        self != MoveMode::Free
    }

    /// What a velocity becomes as this mode's assigned value: SE's Go to and the console's velocity
    /// commands set a tenth of it as the acceleration (DAT_1406d47e8 0.1, FUN_1405c9c80, FUN_140561a40).
    pub fn assigned(self, velocity: f64) -> f64 {
        if self.inertial() { velocity / MODE_SPEED_FACTOR } else { velocity }
    }
}

/// SE's factor between the assigned velocity and the acceleration of the inertial modes
/// (DAT_1406d4ec0 = 10, FUN_14057a740): entering Spacecraft or Aircraft mode divides it, leaving
/// multiplies it back.
pub const MODE_SPEED_FACTOR: f64 = 10.0;

/// An assigned acceleration as SE's toolbar writes it (m/s², km/s², ...).
pub fn format_acceleration(a: f64) -> String {
    let s = se_core::units::format_speed(a);
    match s.strip_suffix("/s") {
        Some(head) => format!("{head}/s²"),
        None => format!("{s}/s"),
    }
}

impl Default for FlyCamera {
    fn default() -> Self {
        Self {
            pos: UPos::ZERO,
            rot: DQuat::IDENTITY,
            velocity: DVec3::ZERO,
            speed: 100.0e3,
            fov_deg: DEFAULT_FOV_DEG,
            mouse_sensitivity: 0.0025,
            selected: None,
            travel: None,
            follow: None,
            nearest_surface: 1.0e12,
            nearest_galaxy: f64::INFINITY,
            nearest_name: String::new(),
            depth_m: 0.0,
            hold_agl: None,
            floor_push: DVec3::ZERO,
            find_shore: None,
            auto_horizon: true,
            horizon_active: false,
            track: false,
            turn_vel: DVec3::ZERO,
            fov_target: None,
            history: Vec::new(),
            locks: InputLocks::default(),
            move_mode: MoveMode::Free,
            thrust: DVec3::ZERO,
            zoom_anim: None,
            travel_serial: 0,
            bind_serial: 0,
        }
    }
}

impl FlyCamera {
    /// SE's binding (BindMode 0 free, 1 follow, 2 rotate with): 0 when the camera is not bound.
    pub fn bind_mode(&self) -> u8 {
        match self.follow {
            None => 0,
            Some(f) if f.sync => 2,
            Some(_) => 1,
        }
    }

    /// Bind the camera to `target` (SE's FollowObject / SyncRotObject), or free it (`None`). Refused
    /// while a script locks movement (FUN_1405986f0).
    pub fn bind(&mut self, target: Option<ObjectRef>, sync: bool, eph: &Ephemeris, jd: f64) {
        if self.locks.travel() {
            return;
        }
        self.follow = target.map(|t| Follow::new(t, sync, eph, jd));
        self.bind_serial = self.bind_serial.wrapping_add(1);
    }

    /// Switch the movement mode (FUN_14057a740): leaving Spacecraft or Aircraft mode multiplies the
    /// assigned value by [`MODE_SPEED_FACTOR`] back into a velocity, entering one divides it into an
    /// acceleration. The velocity carries over (the port keeps it in the universe frame in every
    /// mode, so SE's turn between the Spacecraft and Aircraft frames needs nothing here).
    pub fn set_move_mode(&mut self, mode: MoveMode) {
        if mode == self.move_mode {
            return;
        }
        if self.move_mode.inertial() {
            self.speed *= MODE_SPEED_FACTOR;
        }
        if mode.inertial() {
            self.speed /= MODE_SPEED_FACTOR;
        }
        self.speed = clamp_speed(self.speed);
        self.thrust = DVec3::ZERO;
        self.move_mode = mode;
    }

    /// Set the assigned value from a velocity (Go to's arrival speed): a tenth of it as the
    /// acceleration in the inertial modes (FUN_1405c9c80).
    pub fn assign_velocity(&mut self, v: f64) {
        self.speed = clamp_speed(self.move_mode.assigned(v));
    }
}

/// Camera attachment to a moving body. Each frame the camera is re-expressed in
/// the target's previous frame and re-applied to its current one, so any user
/// movement made during the frame is preserved.
#[derive(Debug, Clone, Copy)]
pub struct Follow {
    pub target: ObjectRef,
    pub last_pos: UPos,
    pub last_rot: DQuat,
    /// Whether the last frame used the body's rotation.
    pub locked: bool,
    /// SE's "Rotate with object" (SyncRot, BindMode 2): turn with the body, not only move with it
    /// ("Follow object", BindMode 1). SE never switches between the two by itself.
    pub sync: bool,
}

impl Follow {
    pub fn new(target: ObjectRef, sync: bool, eph: &Ephemeris, jd: f64) -> Self {
        let last_rot = if sync { eph.orientation(target, jd) } else { DQuat::IDENTITY };
        Self { target, last_pos: eph.position(target, jd), last_rot, locked: sync, sync }
    }
}

/// What a [`Travel`] does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TravelKind {
    /// Go to (G): fly to `end_distance` (0: `GOTO_RADII`) from the target, bound to it (rotating
    /// with a body, following anything else).
    Goto,
    /// Center (C): turn to the target.
    Center,
    /// Land on object (Shift+G): descend to the ground under the camera, rotating with the body,
    /// and level the view.
    Land,
    /// Horizon object (End): level the view to the target's horizon.
    Horizon,
}

#[derive(Debug, Clone)]
pub struct Travel {
    pub target: ObjectRef,
    pub kind: TravelKind,
    /// Camera offset from the target when the flight began (set on its first frame), so
    /// the flight moves with the target instead of starting from a fixed point in space.
    pub start_off: Option<DVec3>,
    pub start_rot: DQuat,
    /// Final distance from the target's centre (m).
    pub end_distance: f64,
    pub elapsed: f64,
    pub duration: f64,
    /// Set the assigned velocity from the arrival distance when the flight starts (Go to from
    /// the keyboard or a panel; script `goto` commands keep the velocity, as SE's console does).
    pub set_speed: bool,
}

#[derive(Component)]
pub struct MainCamera;

/// Set while a spacecraft is flown (`ship.rs`): the camera pose then comes from the ship
/// and `travel_and_move` and the free-flight controls leave it alone.
#[derive(Resource, Default)]
pub(crate) struct FlightHold(pub bool);

fn not_in_flight(hold: Res<FlightHold>) -> bool {
    !hold.0
}

/// Scripted commands (from `--run`), executed one per frame (or after waits).
#[derive(Resource, Default)]
pub struct CommandQueue {
    pub queue: VecDeque<String>,
    pub wait_frames: u32,
    /// `click X Y` (logical pixels): a left click there, handled by the overlay's picking.
    pub click: Option<Vec2>,
}

#[derive(Resource, Default)]
struct ScreenshotState {
    frames: u32,
    taken: bool,
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        let cmds = app.world().resource::<EngineConfig>().commands.clone();
        app.init_resource::<FlyCamera>()
            .init_resource::<FlightHold>()
            .init_resource::<ScreenshotState>()
            .insert_resource(CommandQueue {
                queue: cmds.into(),
                wait_frames: 0,
                click: None,
            })
            .add_systems(Startup, (spawn_camera, place_at_start))
            .add_systems(Update, (run_commands, move_mode_keys, input_controls.run_if(not_in_flight), screenshot_and_exit))
            .add_systems(
                PostUpdate,
                (update_nearest, travel_and_move, sync_camera_transform)
                    .chain()
                    .in_set(UniverseSet::Camera),
            );
    }
}

fn spawn_camera(mut commands: Commands) {
    // SE's bloom (bloom.rs), set from `BloomSettings` (env SE_NO_BLOOM turns it off).
    let bloom = crate::bloom::SeBloom::default();
    // SE's tone shaders (main-def.cfg `ToneMappingShader`) and final colour correction run in
    // color_correction.rs; Bevy's pass only applies the exposure (exposure.rs), linearly.
    let (tonemapping, grading) = (Tonemapping::Linear, ColorGrading::default());
    commands.spawn((
        // The volumetric clouds read the depth buffer (volumetric_clouds.rs).
        Camera3d {
            depth_texture_usages: (bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
                | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING)
                .into(),
            ..default()
        },
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Hdr,
        tonemapping,
        grading,
        bloom,
        Projection::Perspective(PerspectiveProjection {
            fov: 60f32.to_radians(),
            near: 0.05,
            ..default()
        }),
        Transform::IDENTITY,
        // The port has no Bevy lights: no light clusters to build for this view (main.rs turns
        // GPU clustering off).
        bevy::light::cluster::ClusterConfig::None,
        // Every ground patch has its own material, so indirect multi-draw batches nothing but
        // costs ~3-4 ms of per-patch bookkeeping a frame. CPU frustum culling still runs.
        bevy::render::view::NoIndirectDrawing,
        MainCamera,
    ));
}

/// Place the camera near the configured start object.
fn place_at_start(cfg: Res<EngineConfig>, data: Res<SeData>, time: Res<SimTime>, mut cam: ResMut<FlyCamera>) {
    let cat = &data.catalog;
    let Some(r) = cat.find(&cfg.start_at).or_else(|| cat.find("Earth")) else {
        return;
    };
    let eph = Ephemeris::new(cat);
    let target = eph.position(r, time.jd);
    let radius = eph.radius_m(r).max(1.0);
    // Start on the day side: between the target and its star, slightly above the plane.
    let sun = cat.find("Sun").map(|s| eph.position(s, time.jd)).unwrap_or(UPos::ZERO);
    let to_sun = sun.delta_m(target).normalize_or(DVec3::Z);
    let offset_dir = (to_sun + DVec3::Y * 0.3).normalize();
    cam.pos = target.offset_m(offset_dir * radius * 4.0);
    cam.rot = look_rotation(-offset_dir, DVec3::Y);
    cam.selected = Some(r);
}

/// SpaceEngine's `VelocityPresets` (config/main-def.cfg), m/s: Ctrl+wheel steps through them.
const VELOCITY_PRESETS: [f64; 26] = [
    0.1,
    1.0,
    10.0,
    100.0,
    1.0e3,
    1.0e4,
    1.0e5,
    1.0e6,
    1.0e7,
    LIGHT_M_S,
    10.0 * LIGHT_M_S,
    100.0 * LIGHT_M_S,
    METERS_PER_AU,
    10.0 * METERS_PER_AU,
    100.0 * METERS_PER_AU,
    1000.0 * METERS_PER_AU,
    0.1 * METERS_PER_PC,
    METERS_PER_PC,
    10.0 * METERS_PER_PC,
    100.0 * METERS_PER_PC,
    1.0e3 * METERS_PER_PC,
    1.0e4 * METERS_PER_PC,
    1.0e5 * METERS_PER_PC,
    1.0e6 * METERS_PER_PC,
    1.0e7 * METERS_PER_PC,
    1.0e8 * METERS_PER_PC,
];
const LIGHT_M_S: f64 = 299_792_458.0;
/// Velocity factor per wheel notch or +/- press: four notches per decade.
const WHEEL_STEP: f64 = 1.778_279_410_038_923;
/// SpaceEngine `MoveKeyAccel` / `MoveKeyDecel` (s): time to reach / lose full speed.
const MOVE_ACCEL_S: f64 = 0.2;
const MOVE_DECEL_S: f64 = 0.5;
/// SpaceEngine `DefaultFOV` (Home), `ZoomFOV` (middle click), `MinFOV` / `MaxFOV` and `ZoomTime`.
pub const DEFAULT_FOV_DEG: f32 = 45.0;
const ZOOM_FOV_DEG: f32 = 20.0;
pub const MIN_FOV_DEG: f32 = 0.000_277_78;
pub const MAX_FOV_DEG: f32 = 120.0;
const ZOOM_TIME_S: f64 = 0.25;
/// SpaceEngine `TurnKeyAccel` / `TurnKeyDecel` (s), and the port's keyboard turn rates (rad/s at
/// the default FOV; SE's "Turn by keys speed" / "Roll by keys speed" defaults are not known).
const TURN_ACCEL_S: f64 = 0.2;
const TURN_DECEL_S: f64 = 0.5;
const TURN_KEY_RATE: f64 = 1.0;
const ROLL_KEY_RATE: f64 = 1.5;
/// Land on object ends this high over the ground or a sea (m; assumed, SE's Go to stops at 1.5 m).
const LAND_ALTITUDE_M: f64 = 2.0;
/// SE's auto horizon holds the view below max(this x radius, the atmosphere's height)
/// (FUN_140576090: body radius x 0.005 against its atmosphere height).
pub(crate) const HORIZON_RADII: f64 = 0.005;
/// SE's pitch limit in horizon mode (FUN_140576090: +-pi/2, the yaw taken over at 1.5698).
const HORIZON_PITCH_MAX: f64 = 1.5697963267948967;

/// Next (`up`) or previous velocity preset from `v`.
fn velocity_preset_step(v: f64, up: bool) -> f64 {
    let n = VELOCITY_PRESETS.len();
    if up {
        VELOCITY_PRESETS.iter().copied().find(|p| *p > v * 1.0001).unwrap_or(VELOCITY_PRESETS[n - 1])
    } else {
        VELOCITY_PRESETS.iter().rev().copied().find(|p| *p < v * 0.9999).unwrap_or(VELOCITY_PRESETS[0])
    }
}

/// Go to arrives this many radii from the target's centre (SE's tutorials: "4 radii is a default goto
/// distance"; 25,513 km from Earth).
const GOTO_RADII: f64 = 4.0;

/// Go to sets the assigned velocity to the arrival distance (from the target centre) divided by
/// this many seconds: SpaceEngine arrives 25,513 km from Earth's centre at 2,551 km/s, and at the
/// Sun and at Sirius B the same way.
const GOTO_SPEED_DIVISOR: f64 = 10.0;

/// SE's Go to timing (main-def.cfg): `GotoSpeed` 10, the distance falling tenfold a second, so a
/// flight lasts log10(start / arrival distance) seconds plus a second to speed up and brake, from
/// `CenterTime` up to `LongGotoTime` 20 s; G again during a flight is the "fast Go to" of
/// `FastGotoTime` 3 s; Center turns in `CenterTime` 1.5 s.
pub(crate) const GOTO_SPEED: f64 = 10.0;
const LONG_GOTO_TIME: f64 = 20.0;
const FAST_GOTO_TIME: f64 = 3.0;
const CENTER_TIME: f64 = 1.5;

/// A Go to's duration (s) from `start_d` to `end_d` (m).
fn goto_time(start_d: f64, end_d: f64) -> f64 {
    ((start_d / end_d.max(1e-9)).max(1.0).ln() / GOTO_SPEED.ln() + 1.0).clamp(CENTER_TIME, LONG_GOTO_TIME)
}

/// Clamp an assigned velocity to the preset range.
pub fn clamp_speed(v: f64) -> f64 {
    v.clamp(VELOCITY_PRESETS[0], VELOCITY_PRESETS[VELOCITY_PRESETS.len() - 1])
}

/// Rotation whose forward (-Z) points along `dir`.
pub fn look_rotation(dir: DVec3, up: DVec3) -> DQuat {
    let f = dir.normalize_or(DVec3::NEG_Z);
    let up = if f.cross(up).length_squared() < 1e-10 { DVec3::Z } else { up };
    let r = f.cross(up).normalize();
    let u = r.cross(f);
    DQuat::from_mat3(&bevy::math::DMat3::from_cols(r, u, -f))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn input_controls(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    mut cam: ResMut<FlyCamera>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut egui: EguiContexts,
    mut preset_notches: Local<f32>,
    bodies: Query<(&crate::system::SystemBody, Option<&crate::terrain::Terrain>)>,
    (mut msg, i18n, mut last_sel): (ResMut<crate::messages::Messages>, Res<crate::i18n::I18n>, Local<Option<ObjectRef>>),
    // The camera's rotation when the Aircraft velocity was last turned, and that frame's number.
    (mut last_rot, frames): (Local<Option<(DQuat, u32)>>, Res<bevy::diagnostic::FrameCount>),
    control: Res<crate::mods::ModControl>,
) {
    // The Viltrumite mod is on: he owns the mouse, the wheel and the keys that steer or bind the free camera (the
    // spec's F8 table, row by row, gates each below). The rest stays as it is: C, End, T, G, Z, the FOV, the speed.
    let hero = control.hero;
    let (egui_pointer, egui_keys) = egui
        .ctx_mut()
        .map(|c| (c.egui_wants_pointer_input() || crate::hud::pointer_over_ui(c), c.egui_wants_keyboard_input()))
        .unwrap_or((false, false));
    let dt = time.delta_secs_f64();
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let plain = !shift && !ctrl && !alt;
    // Selection history (Backspace goes back through it).
    if cam.selected != *last_sel {
        if let Some(prev) = *last_sel {
            cam.history.retain(|h| *h != prev);
            cam.history.push(prev);
            if cam.history.len() > 64 {
                cam.history.remove(0);
            }
        }
        *last_sel = cam.selected;
    }
    let horizon = horizon_up(&cam, &bodies);
    cam.horizon_active = horizon.is_some();
    // Turn by yaw about the view's up (or the local vertical in horizon mode) and pitch about its right.
    let turn = |rot: DQuat, yaw: f64, pitch: f64| match horizon {
        Some(up) => horizon_turn(rot, up, yaw, pitch),
        None => (rot * DQuat::from_axis_angle(DVec3::Y, yaw) * DQuat::from_axis_angle(DVec3::X, pitch)).normalize(),
    };

    // Mouse: left-drag looks, right-drag orbits, both zoom, Shift+left changes the FOV.
    let (left, right) = (buttons.pressed(MouseButton::Left), buttons.pressed(MouseButton::Right));
    if hero {
        // (The cursor is the mod's too: `mods::snapshot_input`.)
    } else if !egui_pointer && (left || right) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
        let d = motion.delta.as_dvec2() * cam.mouse_sensitivity * (cam.fov_deg as f64 / 60.0);
        let pivot = cam.follow.map(|f| f.target).or(cam.selected);
        let locks = cam.locks;
        let look = (left && !right) || pivot.is_none();
        if d != bevy::math::DVec2::ZERO {
            if left && !right && shift {
                let fov = cam.fov_deg as f64 * (motion.delta.y as f64 * 0.005).exp();
                cam.fov_deg = fov.clamp(MIN_FOV_DEG as f64, MAX_FOV_DEG as f64) as f32;
                cam.fov_target = None;
            } else if look {
                // A script's UserRotationControl "Disabled" stops looking around.
                if !locks.rotation {
                    cam.rot = turn(cam.rot, -d.x, -d.y);
                    cam.travel = None;
                }
            } else if let (false, Some(p)) = (locks.movement, pivot) {
                // Orbiting and zooming move the camera: not while movement is locked.
                let eph = Ephemeris::new(&data.catalog);
                let center = eph.position(p, sim.jd);
                let off = cam.pos.delta_m(center);
                if right && !left {
                    // Orbit: turn the camera and its offset from the object together.
                    let q = DQuat::from_axis_angle(cam.rot * DVec3::Y, -d.x)
                        * DQuat::from_axis_angle(cam.rot * DVec3::X, -d.y);
                    cam.pos = center.offset_m(q * off);
                    cam.rot = (q * cam.rot).normalize();
                } else {
                    // Zoom: scale the distance to the object (drag up to approach).
                    let r = eph.radius_m(p).max(1.0);
                    let f = (motion.delta.y as f64 * 0.01).exp();
                    let solid = matches!(p, ObjectRef::Body(_) | ObjectRef::Star(_));
                    let dist = zoom_distance(off.length(), f, r, solid);
                    cam.pos = center.offset_m(off.normalize_or(DVec3::Z) * dist);
                }
                cam.travel = None;
            }
        }
    } else if cursor.grab_mode != CursorGrabMode::None {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
    // SE's middle click zooms to ZoomFOV, and back to the default FOV.
    if !hero && !egui_pointer && buttons.just_pressed(MouseButton::Middle) {
        let to = if (cam.fov_target.unwrap_or(cam.fov_deg) - ZOOM_FOV_DEG).abs() < 0.01 { DEFAULT_FOV_DEG } else { ZOOM_FOV_DEG };
        cam.fov_target = Some(to);
    }
    zoom_step(&mut cam, dt);

    // Assigned velocity: the wheel scales it smoothly (high-resolution wheels send
    // fractions of a notch), Ctrl+wheel steps through the presets.
    if !hero && !egui_pointer && scroll.delta.y != 0.0 {
        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 100.0,
        };
        if ctrl {
            *preset_notches += notches;
            while preset_notches.abs() >= 1.0 {
                let up = *preset_notches > 0.0;
                cam.speed = velocity_preset_step(cam.speed, up);
                *preset_notches -= preset_notches.signum();
            }
        } else {
            *preset_notches = 0.0;
            cam.speed = clamp_speed(cam.speed * WHEEL_STEP.powf(notches as f64));
        }
    }
    // Typing in a text field: the keys count as let go, so flight and turning ramp down as they
    // would without them instead of carrying on.
    let no_keys = ButtonInput::<KeyCode>::default();
    let keys: &ButtonInput<KeyCode> = if egui_keys { &no_keys } else { &keys };
    let plus = keys.just_pressed(KeyCode::NumpadAdd) || keys.just_pressed(KeyCode::Equal);
    let minus = keys.just_pressed(KeyCode::NumpadSubtract) || keys.just_pressed(KeyCode::Minus);
    if plus || minus {
        cam.speed = if ctrl {
            velocity_preset_step(cam.speed, plus)
        } else {
            clamp_speed(cam.speed * if plus { WHEEL_STEP } else { 1.0 / WHEEL_STEP })
        };
    }
    // Zoom: SE's ZoomIn / ZoomOut (PgUp / PgDn) narrow or widen the FOV while held, but not while a
    // zoom animation runs; ResetZoom (Home) animates back to the default FOV over ZoomTime.
    let zoom_dir = keys.pressed(KeyCode::PageDown) as i32 - keys.pressed(KeyCode::PageUp) as i32;
    if zoom_dir != 0 && cam.fov_target.is_none() {
        cam.fov_deg = zoom_key_step(cam.fov_deg, zoom_dir, dt);
    }
    if keys.just_pressed(KeyCode::Home) && plain {
        cam.fov_target = Some(DEFAULT_FOV_DEG);
    }

    // Keyboard turning and roll, ramped in over TurnKeyAccel and out over TurnKeyDecel.
    let held = |a: KeyCode, b: KeyCode| (keys.pressed(a) || keys.pressed(b)) as i32 as f64;
    let num = |k: KeyCode| (keys.pressed(k) && !ctrl) as i32 as f64;
    let fov_k = (cam.fov_deg as f64 / DEFAULT_FOV_DEG as f64).min(1.0);
    let locks = cam.locks;
    let turn_in = if locks.rotation || hero {
        DVec3::ZERO
    } else {
        DVec3::new(
            (num(KeyCode::Numpad8) - num(KeyCode::Numpad2)) * TURN_KEY_RATE * fov_k,
            (num(KeyCode::Numpad4) - num(KeyCode::Numpad6)) * TURN_KEY_RATE * fov_k,
            (held(KeyCode::KeyQ, KeyCode::Numpad7) - held(KeyCode::KeyE, KeyCode::Numpad9)) * ROLL_KEY_RATE,
        )
    };
    let tau = if turn_in != DVec3::ZERO { TURN_ACCEL_S } else { TURN_DECEL_S } / 3.0;
    cam.turn_vel = cam.turn_vel + (turn_in - cam.turn_vel) * (1.0 - (-dt / tau).exp());
    if turn_in == DVec3::ZERO && cam.turn_vel.length() < 1e-3 {
        cam.turn_vel = DVec3::ZERO;
    }
    if keys.just_pressed(KeyCode::Numpad5) && !ctrl {
        cam.turn_vel = DVec3::ZERO;
    }
    if cam.turn_vel != DVec3::ZERO {
        let w = cam.turn_vel * dt;
        let rot = turn(cam.rot, w.y, w.x);
        cam.rot = (rot * DQuat::from_axis_angle(DVec3::Z, w.z)).normalize();
        cam.travel = None;
    }
    // Look back / left / right / up / down (Ctrl+Numpad 5/4/6/8/2).
    if ctrl && !locks.rotation && !hero {
        let look = [
            (KeyCode::Numpad5, DQuat::from_rotation_y(std::f64::consts::PI)),
            (KeyCode::Numpad4, DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2)),
            (KeyCode::Numpad6, DQuat::from_rotation_y(-std::f64::consts::FRAC_PI_2)),
            (KeyCode::Numpad8, DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2)),
            (KeyCode::Numpad2, DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2)),
        ];
        for (k, q) in look {
            if keys.just_pressed(k) {
                cam.rot = (cam.rot * q).normalize();
                cam.travel = None;
            }
        }
    }

    // Movement; Shift, Ctrl and Alt with these letters are other commands.
    let mut dir = DVec3::ZERO;
    let mv = |a: KeyCode, b: KeyCode| ((plain && keys.pressed(a)) || keys.pressed(b)) as i32 as f64;
    dir.z += mv(KeyCode::KeyS, KeyCode::ArrowDown) - mv(KeyCode::KeyW, KeyCode::ArrowUp);
    dir.x += mv(KeyCode::KeyD, KeyCode::ArrowRight) - mv(KeyCode::KeyA, KeyCode::ArrowLeft);
    dir.y += mv(KeyCode::KeyR, KeyCode::Numpad1) - mv(KeyCode::KeyF, KeyCode::Numpad0);
    if locks.movement || hero {
        dir = DVec3::ZERO;
    }
    if keys.just_pressed(KeyCode::KeyZ) && plain {
        cam.velocity = DVec3::ZERO;
        cam.travel = None;
    }
    let speed = cam.speed;
    if dir != DVec3::ZERO {
        cam.travel = None;
    }
    // Exponential approach: ~95% of the change within MoveKeyAccel / MoveKeyDecel.
    let tau = if dir != DVec3::ZERO { MOVE_ACCEL_S } else { MOVE_DECEL_S } / 3.0;
    let a = 1.0 - (-dt / tau).exp();
    if cam.move_mode.inertial() {
        // Spacecraft / Aircraft: the keys accelerate the camera by the assigned acceleration and the
        // velocity stays when they are let go (FUN_140576090 adds dt * the input to it each frame).
        cam.thrust = cam.thrust + (dir.normalize_or_zero() - cam.thrust) * a;
        if dir == DVec3::ZERO && cam.thrust.length() < 1e-3 {
            cam.thrust = DVec3::ZERO;
        }
        let accel = cam.rot * cam.thrust * speed;
        cam.velocity += accel * dt;
    } else {
        let target_vel = cam.rot * dir.normalize_or_zero() * speed;
        cam.velocity = cam.velocity + (target_vel - cam.velocity) * a;
        if cam.velocity.length() < speed * 1e-4 && dir == DVec3::ZERO {
            cam.velocity = DVec3::ZERO;
        }
    }
    // Aircraft: SE keeps the velocity in the camera frame, so it turns as the camera turns.
    // Only from the previous frame: after frames this system skipped (flying a ship) the
    // rotation meanwhile is not applied at once.
    let frame = frames.0;
    if let (MoveMode::Aircraft, Some((prev, f))) = (cam.move_mode, *last_rot) {
        if f.wrapping_add(1) == frame {
            cam.velocity = aircraft_turn(prev, cam.rot, cam.velocity);
        }
    }
    *last_rot = Some((cam.rot, frame));

    if keys.just_pressed(KeyCode::KeyG) && !alt {
        if let Some(t) = cam.selected {
            if shift && !ctrl {
                start_land(&mut cam, t);
            } else if ctrl && !shift {
                // Go to object's centre (into a galaxy, a nebula, a barycentre).
                start_travel(&mut cam, t, Some(GOTO_CENTER_M));
            } else if !shift && !hero {
                // (While the Viltrumite mod is on, plain G is his lock-on: `viltrumite/lock.rs`.)
                // G again during a Go to to the same object: the fast Go to, from where the camera is.
                let fast = cam.travel.as_ref().is_some_and(|tr| tr.target == t && tr.kind == TravelKind::Goto);
                start_travel(&mut cam, t, None);
                if let (true, Some(tr)) = (fast, cam.travel.as_mut()) {
                    tr.duration = FAST_GOTO_TIME;
                }
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyC) && plain {
        if let Some(t) = cam.selected {
            start_center(&mut cam, t);
        }
    }
    if keys.just_pressed(KeyCode::End) && plain && !locks.rotation {
        if let Some(t) = cam.selected.or(cam.follow.map(|f| f.target)) {
            start_horizon(&mut cam, t);
        }
    }
    // (Not while he's on: the camera is his, and tracking is forced off every frame.)
    if keys.just_pressed(KeyCode::KeyT) && plain && !hero && !locks.rotation && (cam.track || cam.selected.is_some()) {
        cam.track = !cam.track;
        msg.show(format!("{}: {}", i18n.tr("Track object"), i18n.tr(if cam.track { "enabled" } else { "disabled" })));
    }
    // Binding: Shift+D free, Shift+F follow, Shift+R rotate with the selection.
    if shift && !ctrl && !alt {
        let eph = Ephemeris::new(&data.catalog);
        if keys.just_pressed(KeyCode::KeyD) && cam.follow.is_some() && !locks.travel() {
            cam.bind(None, false, &eph, sim.jd);
            msg.show(i18n.tr("Free binding to object"));
        }
        for (k, sync, text) in [(KeyCode::KeyF, false, "Follow object"), (KeyCode::KeyR, true, "Rotate with object")] {
            if let (false, true, Some(t), false) = (hero, keys.just_pressed(k), cam.selected, locks.travel()) {
                cam.bind(Some(t), sync, &eph, sim.jd);
                msg.show(format!("{}: {}", i18n.tr(text), data.catalog.display_name(t)));
            }
        }
        // Shift+H: the home address in turn (main-def.cfg HomeWorld, HomeStar, HomeSystem, HomeGalaxy).
        if keys.just_pressed(KeyCode::KeyH) {
            let home: Vec<ObjectRef> = HOME_ADDRESS.iter().filter_map(|n| data.catalog.find(n)).collect();
            if !home.is_empty() {
                let next = cam.selected.and_then(|s| home.iter().position(|h| *h == s)).map_or(0, |i| (i + 1) % home.len());
                cam.selected = Some(home[next]);
            }
        }
    }
    if keys.just_pressed(KeyCode::Digit5) && plain {
        cam.auto_horizon = !cam.auto_horizon;
        msg.show(format!("{}: {}", i18n.tr("Auto horizon on planets"), i18n.tr(if cam.auto_horizon { "enabled" } else { "disabled" })));
    }
    // Backspace: the previous selection.
    if keys.just_pressed(KeyCode::Backspace) && plain {
        if let Some(prev) = cam.history.pop() {
            cam.selected = Some(prev);
            *last_sel = Some(prev);
        }
    }
}

/// The distance after a two-button zoom by `f` from `d`: never into a solid body or star (radius
/// `r`), nor into a galaxy, cluster or nebula from outside it; inside one the zoom is free (down to
/// 1 m) instead of jumping out to its edge.
fn zoom_distance(d: f64, f: f64, r: f64, solid: bool) -> f64 {
    let floor = if solid || d > r { r * 1.0001 } else { 1.0 };
    (d * f).max(floor)
}

/// An Aircraft-mode velocity after the camera turned from `prev` to `rot`: the same in the camera's
/// frame (SE keeps it there, FUN_140576090 mode 3).
pub fn aircraft_turn(prev: DQuat, rot: DQuat, v: DVec3) -> DVec3 {
    rot * (prev.inverse() * v)
}

/// SE's movement mode keys (key table FUN_1405ba8f0, MoveModeFree..MoveModeGame on 1-4): 1-3 set
/// the Free, Spacecraft or Aircraft mode with its splash (FUN_1405c7f80, FUN_1405c8050,
/// FUN_1405c80f0), leaving a spacecraft being flown first (they clear the piloted ship); 4 flies the
/// current spacecraft (FUN_1405b9420 -> FUN_1401f6aa0), which needs one (the Ships window), and SE
/// then is in Free mode. Ctrl+1-5 are the flight HUD modes (ship/hud.rs) and 5-0 the flight
/// attitudes and the auto horizon, so only plain 1-4 are taken here.
pub(crate) fn move_mode_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut cam: ResMut<FlyCamera>,
    flight: Option<ResMut<crate::ship::Flight>>,
    mut egui: EguiContexts,
    (mut msg, i18n, mut was_flying): (ResMut<crate::messages::Messages>, Res<crate::i18n::I18n>, Local<bool>),
) {
    let mut flight = flight;
    let piloting = flight.as_ref().is_some_and(|f| f.active);
    // SE flies a spacecraft in Free mode (FUN_1401f6aa0 sets it), however the flight began: key 4,
    // Enter, F or the Ships window.
    if piloting && !*was_flying {
        cam.set_move_mode(MoveMode::Free);
    }
    *was_flying = piloting;
    let typing = egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false);
    let mods = [KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft, KeyCode::AltRight];
    if typing || keys.any_pressed(mods) {
        return;
    }
    for (k, mode) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3].into_iter().zip(MoveMode::ALL) {
        if !keys.just_pressed(k) || (mode == cam.move_mode && !piloting) {
            continue;
        }
        if let Some(f) = flight.as_mut().filter(|f| f.active) {
            f.active = false;
            f.remote = false;
            f.stop_engines();
        }
        cam.set_move_mode(mode);
        msg.show(i18n.tr(mode.label()));
    }
    if keys.just_pressed(KeyCode::Digit4) {
        if let Some(f) = flight.as_mut().filter(|f| f.ship.is_some() && !f.active) {
            f.active = true;
            f.remote = false;
            cam.set_move_mode(MoveMode::Free);
        }
    }
}

/// SE's home address (main-def.cfg Controls, cycled with Shift+H), from the world outwards.
const HOME_ADDRESS: [&str; 4] = ["Earth", "Sun", "Solar System", "Milky Way"];

/// Go to object's centre stops this far from it (m).
const GOTO_CENTER_M: f64 = 1.0;

/// SE's ZoomIn / ZoomOut step (FUN_1405b8ba0 / FUN_1405b8b10 -> FUN_14056b0c0): the FOV times
/// 1 / 1.02 or 1.02 (DAT_1406d4ab0, DAT_1406d4b30) each frame the key is held, within MinFOV..MaxFOV.
/// SE steps once a frame; the port steps at that rate for 60 frames a second, whatever its frame rate.
const ZOOM_KEY_FACTOR: f64 = 1.02;
const ZOOM_KEY_FPS: f64 = 60.0;

/// The FOV after holding a zoom key for `dt` seconds (`dir` +1 widens, -1 narrows).
pub fn zoom_key_step(fov: f32, dir: i32, dt: f64) -> f32 {
    let f = fov as f64 * ZOOM_KEY_FACTOR.powf(dir as f64 * dt * ZOOM_KEY_FPS);
    f.clamp(MIN_FOV_DEG as f64, MAX_FOV_DEG as f64) as f32
}

/// SE's FOV animation (FUN_14056ad80): over `ZoomTime` the FOV goes from `from` to `to` as
/// from + (to - from) * (t / ZoomTime)^p, p = 0.5 when narrowing (fast at first) and 2 when
/// widening (DAT_1406d4990, DAT_1406d4c60).
pub fn zoom_anim_fov(a: &ZoomAnim) -> f32 {
    let t = (a.elapsed / ZOOM_TIME_S).clamp(0.0, 1.0);
    let p = if a.from < a.to { 2.0 } else { 0.5 };
    (a.from as f64 + (a.to as f64 - a.from as f64) * t.powf(p)) as f32
}

/// Animate to `fov_target` (set by the middle button, Home and scripts) over `ZoomTime`; a new
/// target starts a new animation from the current FOV.
fn zoom_step(cam: &mut FlyCamera, dt: f64) {
    let Some(to) = cam.fov_target else {
        cam.zoom_anim = None;
        return;
    };
    let mut a = match cam.zoom_anim {
        Some(a) if a.to == to => a,
        _ => ZoomAnim { from: cam.fov_deg, to, elapsed: 0.0 },
    };
    a.elapsed += dt;
    cam.fov_deg = zoom_anim_fov(&a);
    if a.elapsed >= ZOOM_TIME_S {
        cam.fov_deg = to;
        cam.fov_target = None;
        cam.zoom_anim = None;
    } else {
        cam.zoom_anim = Some(a);
    }
}

/// The local vertical (universe frame) while SE's auto horizon holds the view: AutoHorizon on, the
/// camera rotating with a body (SyncRot), and lower than max(`HORIZON_RADII` radii, the atmosphere's
/// height) over its ellipsoid (FUN_140576090). The vertical is the ellipsoid's normal.
pub fn horizon_up(cam: &FlyCamera, bodies: &Query<(&crate::system::SystemBody, Option<&crate::terrain::Terrain>)>) -> Option<DVec3> {
    if !cam.auto_horizon {
        return None;
    }
    let f = cam.follow.filter(|f| f.sync)?;
    let (b, t) = bodies.iter().find(|(b, _)| b.obj == f.target && !b.star)?;
    let rel = b.orient.inverse() * cam.pos.delta_m(b.pos);
    let dir = b.from_shape(rel).normalize_or(DVec3::Y);
    let alt = rel.length() - b.radius_m * b.to_shape(dir).length();
    let atm = t.and_then(|t| t.atmosphere.as_ref()).map_or(0.0, |a| a.top_radius_m(b.radius_m) - b.radius_m);
    (alt < (b.radius_m * HORIZON_RADII).max(atm)).then(|| (b.orient * (dir / b.shape)).normalize())
}

/// SE's horizon mode turn: the view's yaw about the local vertical `up`, pitch towards it (stopping
/// short of straight up or down) and roll are kept as angles, so turning never tilts the horizon.
pub fn horizon_turn(rot: DQuat, up: DVec3, yaw: f64, pitch: f64) -> DQuat {
    let f = rot * DVec3::NEG_Z;
    let sin_e = f.dot(up).clamp(-1.0, 1.0);
    // Heading along the ground; looking straight down (or up) it is the view's up (or down).
    let flat = f - up * sin_e;
    let (heading, roll) = if flat.length() > 1e-4 {
        let level = look_rotation(f, up);
        let q = level.inverse() * rot;
        (flat.normalize(), 2.0 * q.z.atan2(q.w))
    } else {
        let u = rot * DVec3::Y * -sin_e.signum();
        ((u - up * u.dot(up)).normalize_or(up.any_orthonormal_vector()), 0.0)
    };
    let e = (sin_e.asin() + pitch).clamp(-HORIZON_PITCH_MAX, HORIZON_PITCH_MAX);
    let h = DQuat::from_axis_angle(up, yaw) * heading;
    let f2 = h * e.cos() + up * e.sin();
    (look_rotation(f2, up) * DQuat::from_rotation_z(roll)).normalize()
}

/// The level view along `rot`'s heading (or its up when looking straight down) at local vertical `up`.
fn level_rotation(rot: DQuat, up: DVec3) -> DQuat {
    let f = rot * DVec3::NEG_Z;
    let mut h = f - up * f.dot(up);
    if h.length() < 0.1 {
        let u = rot * DVec3::Y * -f.dot(up).signum();
        h = u - up * u.dot(up);
    }
    look_rotation(h.normalize_or(up.any_orthonormal_vector()), up)
}

/// SE's Center: turn to the target in `CenterTime`. Handled in `travel_and_move` as a travel that
/// only rotates; the camera stays bound to the object it follows.
pub fn start_center(cam: &mut FlyCamera, target: ObjectRef) {
    // SE's Center is refused while a script locks rotation (FUN_140563cf0).
    if cam.locks.rotation {
        return;
    }
    let start_rot = cam.rot;
    cam.travel = Some(Travel { target, kind: TravelKind::Center, start_off: None, start_rot, end_distance: -1.0, elapsed: 0.0, duration: CENTER_TIME, set_speed: false });
    cam.travel_serial = cam.travel_serial.wrapping_add(1);
}

/// SE's Horizon object (End): level the view to `target`'s horizon in `CenterTime`.
pub fn start_horizon(cam: &mut FlyCamera, target: ObjectRef) {
    let start_rot = cam.rot;
    cam.travel = Some(Travel { target, kind: TravelKind::Horizon, start_off: None, start_rot, end_distance: -1.0, elapsed: 0.0, duration: CENTER_TIME, set_speed: false });
    cam.travel_serial = cam.travel_serial.wrapping_add(1);
}

/// SE's Land on object (Shift+G): planets, moons, asteroids and comets only.
pub fn start_land(cam: &mut FlyCamera, target: ObjectRef) {
    if !matches!(target, ObjectRef::Body(_)) || cam.locks.travel() {
        return;
    }
    cam.velocity = DVec3::ZERO;
    let start_rot = cam.rot;
    cam.travel = Some(Travel { target, kind: TravelKind::Land, start_off: None, start_rot, end_distance: LAND_ALTITUDE_M, elapsed: 0.0, duration: -1.0, set_speed: true });
    cam.travel_serial = cam.travel_serial.wrapping_add(1);
}

/// Go to `target` (refused while a script locks movement).
pub fn start_travel(cam: &mut FlyCamera, target: ObjectRef, distance: Option<f64>) {
    if cam.locks.travel() {
        return;
    }
    cam.velocity = DVec3::ZERO;
    cam.travel = Some(Travel {
        target,
        kind: TravelKind::Goto,
        start_off: None,
        start_rot: cam.rot,
        end_distance: distance.unwrap_or(0.0),
        elapsed: 0.0,
        // Set from the distance on the flight's first frame (`goto_time`).
        duration: -1.0,
        set_speed: true,
    });
    cam.travel_serial = cam.travel_serial.wrapping_add(1);
}

/// Find the nearest surface among the active system's bodies and the galaxies.
fn update_nearest(
    data: Res<SeData>,
    mut cam: ResMut<FlyCamera>,
    mut space: ResMut<ScaledSpace>,
    bodies: Query<&crate::system::SystemBody>,
    clouds: Res<crate::volumetric_clouds::VolumetricClouds>,
) {
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    let mut best = f64::INFINITY;
    let mut best_name = String::new();
    let mut best_center = f64::INFINITY;
    let mut best_radius = 0.0;
    for b in &bodies {
        let c = b.pos.delta_m(cam.pos).length();
        let surf = (c - b.radius_m).max(0.0);
        if surf < best {
            best = surf;
            best_center = c;
            best_radius = b.radius_m;
            best_name = cat.display_name(b.obj).to_string();
        }
    }
    // Galaxies we are outside of: distance to their disk extent.
    let mut nearest_galaxy = f64::INFINITY;
    for g in &cat.galaxies {
        let d = (g.pos_pc.distance(cam_pc) - g.radius_pc) * se_core::units::METERS_PER_PC;
        if d > 0.0 {
            nearest_galaxy = nearest_galaxy.min(d);
        }
        if d > 0.0 && d < best {
            best = d;
            best_center = d;
            best_radius = 0.0;
            best_name = g.names[0].clone();
        }
    }
    cam.nearest_surface = best.max(1.0);
    cam.nearest_galaxy = nearest_galaxy;
    cam.nearest_name = best_name;
    // Keep the nearest body (and its neighbourhood) in linear space, and the volumetric clouds' body
    // (it may lie beyond a nearer moon).
    let clouds_reach = clouds.active.and_then(|e| bodies.get(e).ok()).map_or(0.0, |b| b.pos.delta_m(cam.pos).length() + b.radius_m * 3.0);
    space.linear_radius = (best_center + best_radius * 3.0).max(clouds_reach).clamp(1.0e5, 1.0e13);
}

pub fn travel_and_move(
    time: Res<Time>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    mut cam: ResMut<FlyCamera>,
    hold: Res<FlightHold>,
    bodies: Query<(&crate::system::SystemBody, Option<&crate::terrain::Terrain>)>,
    control: Res<crate::mods::ModControl>,
    hero: Res<crate::viltrumite::Hero>,
) {
    // A ship's camera, or the Viltrumite mod's, unless a Go To or the like is running: that carries him. (Until he's placed
    // it goes on as it's bound, so he appears at its spot against this frame's bodies, not one frame of their motion off.)
    let his = control.hero && hero.placed;
    if hold.0 || (his && cam.travel.is_none()) {
        return;
    }
    let dt = time.delta_secs_f64();
    let eph = Ephemeris::new(&data.catalog);
    // A Go To starting while he's on: what carries him has moved since he last wrote the camera. With it first, or the
    // flight starts from one frame of its motion off.
    if his && cam.travel.as_ref().is_some_and(|t| t.start_off.is_none() && matches!(t.kind, TravelKind::Goto | TravelKind::Land)) {
        cam.follow = hero.carrier;
    }
    // Carry the camera with the object it is bound to, also while a Go to or Center
    // flight runs: planets move at tens of km/s, which would otherwise sweep their
    // surface into a camera near them.
    if let Some(mut f) = cam.follow {
        let target_pos = eph.position(f.target, sim.jd);
        let radius = eph.radius_m(f.target);
        // Express the camera in last frame's target frame...
        let local_off = f.last_rot.inverse() * cam.pos.delta_m(f.last_pos);
        let local_rot = f.last_rot.inverse() * cam.rot;
        // ...turning with the body in SE's SyncRot binding, otherwise only moving with it.
        let locked = f.sync && radius > 0.0;
        let rot = if locked { eph.orientation(f.target, sim.jd) } else { DQuat::IDENTITY };
        let (local_off, local_rot) = if locked != f.locked {
            // Switching modes: re-derive the local frame from world values.
            (rot.inverse() * (f.last_rot * local_off), rot.inverse() * (f.last_rot * local_rot))
        } else {
            (local_off, local_rot)
        };
        cam.pos = target_pos.offset_m(rot * local_off);
        cam.rot = (rot * local_rot).normalize();
        f.last_pos = target_pos;
        f.last_rot = rot;
        f.locked = locked;
        cam.follow = Some(f);
    }
    if let Some(mut tr) = cam.travel.take() {
        tr.elapsed += dt;
        let target_pos = eph.position(tr.target, sim.jd);
        let radius = eph.radius_m(tr.target).max(100.0);
        if tr.kind == TravelKind::Land {
            land_step(&mut cam, &mut tr, &eph, sim.jd, &bodies);
        } else if tr.kind == TravelKind::Horizon {
            let t = (tr.elapsed / tr.duration).clamp(0.0, 1.0);
            let up = cam.pos.delta_m(target_pos).normalize_or(DVec3::Y);
            cam.rot = tr.start_rot.slerp(level_rotation(tr.start_rot, up), t * t * (3.0 - 2.0 * t)).normalize();
            if t < 1.0 {
                cam.travel = Some(tr);
            }
            return;
        }
        if tr.kind == TravelKind::Land {
            if tr.elapsed < tr.duration {
                cam.travel = Some(tr);
            }
            return;
        }
        if tr.duration < 0.0 {
            let end_d = if tr.end_distance > 0.0 { tr.end_distance } else { radius * GOTO_RADII };
            tr.duration = goto_time(target_pos.delta_m(cam.pos).length(), end_d);
        }
        let t = (tr.elapsed / tr.duration).clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        let to_target = target_pos.delta_m(cam.pos);
        // Orientation: turn towards the target during the first part of the flight.
        let look = look_rotation(to_target, cam.rot * DVec3::Y);
        let rt = (s * 2.0).min(1.0);
        cam.rot = tr.start_rot.slerp(look, rt).normalize();
        if tr.kind == TravelKind::Goto {
            let end_d = if tr.end_distance > 0.0 { tr.end_distance } else { radius * GOTO_RADII };
            if tr.start_off.is_none() && tr.set_speed {
                // As in SpaceEngine, Go to sets the assigned velocity to a tenth of the arrival
                // distance per second, so flying on afterwards approaches the target instead of
                // rushing past it at the speed used to get there.
                cam.assign_velocity(end_d / GOTO_SPEED_DIVISOR);
            }
            let start_off = *tr.start_off.get_or_insert_with(|| cam.pos.delta_m(target_pos));
            // From the real distance, also from inside the arrival distance (no jump out on the
            // first frame).
            let start_d = start_off.length().max(1.0);
            // Interpolate the distance logarithmically so huge jumps stay smooth.
            let d = (start_d.ln() + (end_d.ln() - start_d.ln()) * s).exp();
            let dir = start_off.normalize_or(DVec3::Z);
            cam.pos = target_pos.offset_m(dir * d);
            // Go to binds the camera to the target at once, as in SpaceEngine, so a flight
            // cut short (by moving or looking around) still moves with it. SE's binding is
            // Follow ("Following Moon", its tutorial); the port rotates with planets, moons and
            // the other bodies (the user's choice), so their surface holds still under the camera.
            // (Bound already: the next frame's follow step switches the frame.)
            let sync = matches!(tr.target, ObjectRef::Body(_));
            match cam.follow.as_mut() {
                Some(f) if f.target == tr.target => f.sync = sync,
                _ => cam.follow = Some(Follow::new(tr.target, sync, &eph, sim.jd)),
            }
        }
        if t < 1.0 {
            cam.travel = Some(tr);
        }
    } else {
        if cam.velocity != DVec3::ZERO {
            let v = cam.velocity;
            cam.pos = cam.pos.offset_m(v * dt);
        }
        // Track object: keep the view on the selection.
        if cam.track {
            match cam.selected {
                Some(sel) => {
                    let to = eph.position(sel, sim.jd).delta_m(cam.pos);
                    if to.length_squared() > 0.0 {
                        cam.rot = look_rotation(to, cam.rot * DVec3::Y);
                    }
                }
                None => cam.track = false,
            }
        }
    }
}

/// One frame of Land on object: the camera drops (logarithmically in altitude) to `end_distance`
/// over the ground (or sea) under where it started, rotating with the body from the start, and
/// turns to the level view along its heading (its up when it looked straight down).
fn land_step(cam: &mut FlyCamera, tr: &mut Travel, eph: &Ephemeris, jd: f64, bodies: &Query<(&crate::system::SystemBody, Option<&crate::terrain::Terrain>)>) {
    let pos = eph.position(tr.target, jd);
    let q = eph.orientation(tr.target, jd);
    let radius = eph.radius_m(tr.target).max(1.0);
    let body = bodies.iter().find(|(b, _)| b.obj == tr.target);
    let shape = body.map_or(DVec3::ONE, |(b, _)| b.shape);
    let start = *tr.start_off.get_or_insert_with(|| {
        tr.start_rot = q.inverse() * cam.rot;
        q.inverse() * cam.pos.delta_m(pos)
    });
    let dir = (start / shape).normalize_or(DVec3::Y);
    let k = (dir * shape).length();
    let ground = body.and_then(|(_, t)| t).map_or(0.0, |t| t.surface_height(dir));
    let alt0 = (start.length() - radius * k).max(tr.end_distance * 1.0001);
    if tr.duration < 0.0 {
        tr.duration = goto_time(alt0, tr.end_distance);
        if tr.set_speed {
            // Walking pace on arrival (assumed).
            cam.assign_velocity(10.0);
        }
    }
    let t = (tr.elapsed / tr.duration).clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    // The altitude over the reference ellipsoid falls to the ground's height plus the landing height.
    let alt1 = ground.max(0.0) * k + tr.end_distance;
    let alt = (alt0.ln() + (alt1.max(1e-3).ln() - alt0.ln()) * s).exp() + ground.min(0.0) * k * s;
    let up = (dir / shape).normalize();
    let end_rot = level_rotation(tr.start_rot, up);
    cam.pos = pos.offset_m(q * (start.normalize() * (radius * k + alt)));
    cam.rot = (q * tr.start_rot.slerp(end_rot, s)).normalize();
    cam.follow = Some(Follow { target: tr.target, last_pos: pos, last_rot: q, locked: true, sync: true });
}

/// Near clipping plane for a camera `surface_m` above the nearest surface: far out for
/// depth precision, but always well in front of that surface (down to a millimetre when
/// crossing a sea surface).
pub fn near_plane(surface_m: f64) -> f32 {
    (surface_m * 1e-3).clamp((surface_m * 0.5).clamp(0.001, 0.02), 1000.0) as f32
}

pub fn sync_camera_transform(
    cam: Res<FlyCamera>,
    mut q: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    for (mut tf, mut proj) in &mut q {
        tf.translation = Vec3::ZERO;
        tf.rotation = cam.rot.as_quat();
        if let Projection::Perspective(p) = proj.as_mut() {
            p.fov = cam.fov_deg.to_radians();
            p.near = near_plane(cam.nearest_surface);
        }
    }
}

/// Executes `--run` commands: `goto NAME [dist_km]`, `select NAME`, `look NAME`,
/// `wait N`, `rate X`, `pause`, `date YYYY-MM-DD`, `speed N`, `fov DEG`,
/// `move x y z` (meters, camera frame), `fly KMS` (dev: keep flying forward at KMS km/s).
fn run_commands(
    mut q: ResMut<CommandQueue>,
    mut cam: ResMut<FlyCamera>,
    mut sim: ResMut<SimTime>,
    data: Res<SeData>,
    active: Res<crate::system::ActiveSystem>,
    places: Res<crate::data::Places>,
    mut ship_cmds: MessageWriter<crate::ship::ShipCommand>,
    mut fly: Local<f64>,
    procstars: Option<Res<crate::procstars::ProcStarField>>,
    mut commands: Commands,
    mut search: ResMut<crate::star_search::SearchState>,
    (mut stars, mut spin, mut orbit): (ResMut<crate::stars::StarSettings>, Local<f64>, Local<f64>),
    bodies: Query<(&crate::system::SystemBody, Option<&crate::terrain::Terrain>)>,
    (wps, mut wait_wp): (Option<Res<crate::waypoints::Waypoints>>, Local<usize>),
    (mut timing, mut switches): (ResMut<crate::universe::TimingMode>, ResMut<crate::switches::RenderSwitches>),
) {
    let active_root = active.root;
    // `waitwp N`: hold the queue until a script has defined N waypoints.
    if *wait_wp > 0 {
        if wps.as_ref().is_some_and(|w| w.list.len() >= *wait_wp) {
            *wait_wp = 0;
        } else {
            return;
        }
    }
    // `spin DEG`: dev turn about the view's up axis, DEG degrees per second at a nominal 200 frames/s.
    if *spin != 0.0 {
        let r = cam.rot * DQuat::from_rotation_y((*spin / 200.0).to_radians());
        cam.rot = r.normalize();
    }
    // `orbit DEG`: dev orbit about the selection (as the right mouse button drags it), DEG degrees per
    // second about the view's up axis at a nominal 200 frames/s.
    if let (true, Some(p)) = (*orbit != 0.0, cam.selected) {
        let center = Ephemeris::new(&data.catalog).position(p, sim.jd);
        let q = DQuat::from_axis_angle(cam.rot * DVec3::Y, (*orbit / 200.0).to_radians());
        let off = cam.pos.delta_m(center);
        cam.pos = center.offset_m(q * off);
        cam.rot = (q * cam.rot).normalize();
    }
    // `fly KMS`: dev flight along the view direction, KMS km/s at a nominal 200 frames/s (fixed step per frame,
    // so runs are repeatable). `fly 0` stops it.
    if *fly != 0.0 {
        let d = cam.rot * DVec3::new(0.0, 0.0, -*fly * 1000.0 / 200.0);
        cam.pos = cam.pos.offset_m(d);
    }
    if q.wait_frames > 0 {
        q.wait_frames -= 1;
        return;
    }
    if cam.travel.is_some() {
        return;
    }
    let Some(cmd) = q.queue.pop_front() else { return };
    info!("command: {cmd}");
    let mut parts = cmd.splitn(2, ' ');
    let verb = parts.next().unwrap_or("").to_ascii_lowercase();
    let arg = parts.next().unwrap_or("").trim().to_string();
    let cat = &data.catalog;
    let find = |s: &str| cat.find(s);
    match verb.as_str() {
        "goto" | "select" | "look" => {
            // Optional trailing distance in km: "goto Earth 20000"
            let (name, dist) = match arg.rsplit_once(' ') {
                Some((n, d)) if d.parse::<f64>().is_ok() && find(&arg).is_none() => {
                    (n.to_string(), d.parse::<f64>().ok().map(|k| k * 1000.0))
                }
                _ => (arg.clone(), None),
            };
            match find(&name) {
                Some(r) => {
                    cam.selected = Some(r);
                    match verb.as_str() {
                        "goto" => {
                            start_travel(&mut cam, r, dist);
                            if let Some(t) = cam.travel.as_mut() {
                                t.duration = 0.01;
                                t.set_speed = false;
                            }
                        }
                        "look" => {
                            let eph = Ephemeris::new(cat);
                            let d = eph.position(r, sim.jd).delta_m(cam.pos);
                            cam.rot = look_rotation(d, DVec3::Y);
                        }
                        _ => {}
                    }
                }
                None => warn!("command target not found: {name}"),
            }
        }
        // `bodypos NAME X Y Z [DX DY DZ]`: the camera at km X, Y (along the pole), Z in the body's own
        // frame, looking along DX DY DZ (body frame; default at the centre), the pole up.
        "bodypos" => {
            let words: Vec<&str> = arg.split_whitespace().collect();
            let nums: Vec<f64> = words.iter().rev().map_while(|w| w.parse().ok()).collect::<Vec<_>>().into_iter().rev().collect();
            let name = words[..words.len() - nums.len()].join(" ");
            match (find(&name), nums.len()) {
                (Some(r), 3 | 6) => {
                    let eph = Ephemeris::new(cat);
                    let q = eph.orientation(r, sim.jd);
                    let p = DVec3::new(nums[0], nums[1], nums[2]) * 1000.0;
                    let look = if nums.len() == 6 { DVec3::new(nums[3], nums[4], nums[5]) } else { -p };
                    cam.pos = eph.position(r, sim.jd).offset_m(q * p);
                    cam.rot = look_rotation(q * look, q * DVec3::Y);
                    cam.selected = Some(r);
                }
                _ => warn!("bodypos: expected NAME X Y Z [DX DY DZ], got {arg}"),
            }
        }
        "wait" => q.wait_frames = arg.parse().unwrap_or(1),
        "waitwp" => *wait_wp = arg.parse().unwrap_or(1),
        "rate" => sim.rate = arg.parse().unwrap_or(1.0),
        "pause" => sim.paused = true,
        "resume" => sim.paused = false,
        // SE's TimingMode: `timing frame` steps the clock a fixed 1/60 s a frame (test runs then
        // reach the same date whatever their frame rate), `timing real` goes back.
        // timing frame [FPS] | real: FPS is what a frame steps by (SE's `RealTimeFPS`, 60).
        "timing" => {
            let mut parts = arg.split_whitespace();
            timing.real_time = parts.next() != Some("frame");
            if let Some(fps) = parts.next().and_then(|f| f.parse::<f64>().ok()) {
                timing.fps = fps;
            }
        }
        // A render filter by its settings key, e.g. `switch ShowRain off` (dev: comparisons).
        "switch" => {
            if let Some((key, on)) = arg.split_once(' ') {
                switches.set_var(key.trim(), on.trim() != "off");
            }
        }
        "fly" => *fly = arg.parse().unwrap_or(0.0),
        "go" => {
            // go [KM]: Go to the selection (like G), arriving at once.
            if let Some(t) = cam.selected {
                start_travel(&mut cam, t, arg.parse::<f64>().ok().map(|k| k * 1000.0));
                if let Some(t) = cam.travel.as_mut() {
                    t.duration = 0.01;
                }
            }
        }
        "click" => {
            let v: Vec<f32> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if v.len() == 2 {
                q.click = Some(Vec2::new(v[0], v[1]));
            }
        }
        "speed" => cam.assign_velocity(arg.parse::<f64>().unwrap_or(100.0) * 1000.0),
        "fov" => cam.fov_deg = arg.parse().unwrap_or(DEFAULT_FOV_DEG),
        // zoomto DEG: animate the FOV there over ZoomTime, as the middle button and Home do.
        "zoomto" => cam.fov_target = arg.parse::<f32>().ok().map(|f| f.clamp(MIN_FOV_DEG, MAX_FOV_DEG)),
        // movemode N: SE's movement mode 1 Free, 2 Spacecraft, 3 Aircraft (keys 1-3).
        "movemode" => {
            if let Some(m) = arg.parse::<u8>().ok().and_then(MoveMode::from_number) {
                cam.set_move_mode(m);
            }
        }
        // points N: SE's StarPointMethod (1 sprites, 2 motion blur, 3 motion and rotation blur).
        "points" => stars.point_style = crate::stars::PointStyle::from_method(arg.parse().unwrap_or(2)),
        "colorshift" => stars.color_shift = arg.parse().unwrap_or(0.0),
        "maglimit" => {
            stars.mag_limit = arg.parse().unwrap_or(stars.mag_limit).clamp(crate::stars::MAG_LIMIT_MIN, crate::stars::MAG_LIMIT_MAX)
        }
        "spin" => *spin = arg.parse().unwrap_or(0.0),
        "orbit" => *orbit = arg.parse().unwrap_or(0.0),
        "date" => {
            let p: Vec<f64> = arg.split(['-', ' ', ':', 'T']).filter_map(|s| s.parse().ok()).collect();
            if p.len() >= 3 {
                let dt = se_core::time::DateTime::new(
                    p[0] as i64,
                    p[1] as u32,
                    p[2] as u32,
                    p.get(3).copied().unwrap_or(0.0) as u32,
                    p.get(4).copied().unwrap_or(0.0) as u32,
                    p.get(5).copied().unwrap_or(0.0),
                );
                sim.jd = dt.to_jd();
            }
        }
        // vel X Y Z: a push of the free-flight velocity (km/s, camera axes), as a held movement key
        // gives (script `Control|Move` triggers watch it).
        "vel" => {
            let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if v.len() == 3 {
                cam.velocity = cam.rot * DVec3::new(v[0], v[1], v[2]) * 1000.0;
            }
        }
        "move" => {
            let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if v.len() == 3 {
                let d = cam.rot * DVec3::new(v[0], v[1], v[2]);
                cam.pos = cam.pos.offset_m(d);
            }
        }
        "land" => {
            // land: SE's Land on object (Shift+G) on the selection.
            if let Some(t) = cam.selected {
                start_land(&mut cam, t);
            }
        }
        "bind" => {
            // bind 0|1|2: free, follow or rotate with the selection (Shift+D / F / R).
            let m: u8 = arg.trim().parse().unwrap_or(0);
            let t = cam.selected.filter(|_| m > 0);
            cam.bind(t, m == 2, &Ephemeris::new(&data.catalog), sim.jd);
        }
        "turn" | "roll" => {
            // turn YAW PITCH / roll DEG (degrees): a mouse-look turn (through the auto horizon when it
            // holds) or a roll.
            let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse::<f64>().ok()).map(f64::to_radians).collect();
            let (a, b) = (v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0));
            cam.rot = match (verb.as_str(), horizon_up(&cam, &bodies)) {
                ("roll", _) => cam.rot * DQuat::from_rotation_z(a),
                (_, Some(up)) => horizon_turn(cam.rot, up, a, b),
                _ => cam.rot * DQuat::from_rotation_y(a) * DQuat::from_rotation_x(b),
            }
            .normalize();
        }
        "station" => {
            // station N [KM]: look at a level 4 civilization's station N from KM (default 3) away,
            // on its sunlit side (orbital.rs; pause first: stations move along their orbits).
            let mut it = arg.split_whitespace();
            let n = it.next().and_then(|w| w.parse::<usize>().ok()).unwrap_or(0);
            let km = it.next().and_then(|w| w.parse::<f64>().ok()).unwrap_or(3.0);
            commands.insert_resource(crate::orbital::StationGoto(Some((n, km))));
        }
        "agl" => {
            // agl M: hold the camera M metres above the ground under it (`agl off` ends it).
            cam.hold_agl = arg.trim().parse().ok();
        }
        "shore" => {
            // shore [M]: the nearest coast in the loaded terrain, M m inland (300), facing the sea.
            cam.find_shore = Some(arg.trim().parse().unwrap_or(300.0));
        }
        "surface" => {
            // surface LAT LON ALT_KM [PITCH_DEG] [HEADING_DEG] on the selected body.
            let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if let (Some(target), true) = (cam.selected, v.len() >= 3) {
                let eph = Ephemeris::new(cat);
                let (lat, lon) = (v[0].to_radians(), v[1].to_radians());
                let pitch = v.get(3).copied().unwrap_or(-15.0).to_radians();
                let heading = v.get(4).copied().unwrap_or(0.0).to_radians();
                let radius = eph.radius_m(target);
                let q = eph.orientation(target, sim.jd);
                let tp = eph.position(target, sim.jd);
                // SE body frame: +X lon 0, +Y north, +Z lon 90 W.
                let up = DVec3::new(lat.cos() * lon.cos(), lat.sin(), -lat.cos() * lon.sin());
                let north = (DVec3::Y - up * up.y).normalize_or(DVec3::X);
                let east = north.cross(up);
                let fwd_h = north * heading.cos() + east * heading.sin();
                let fwd = (fwd_h * pitch.cos() + up * pitch.sin()).normalize();
                // On SE's ellipsoid: the ground point drawn for `up`, raised along it.
                let shape = match target {
                    ObjectRef::Body(i) => DVec3::from_array(cat.bodies[i as usize].shape()),
                    _ => DVec3::ONE,
                };
                cam.pos = tp.offset_m(q * (up * radius * shape + up * (v[2] * 1000.0)));
                cam.rot = look_rotation(q * fwd, q * up);
                cam.velocity = DVec3::ZERO;
                cam.follow = Some(Follow {
                    target,
                    last_pos: tp,
                    last_rot: q,
                    locked: true,
                    sync: true,
                });
            }
        }
        "town" => {
            // town [village|town|city|mega] [ALT_KM] [PITCH]: to the nearest settlement of that size
            // (the largest the civilization has by default) of the selected body's civilization
            // (`SE_CIV_TEST` too), looking at it from ALT_KM (default 2) at PITCH degrees (default
            // -35; at most -2, the camera back far enough to see the middle ahead). The land there is
            // not checked: the field may put it in the sea or on mountains (try the next size).
            let mut kind = None;
            let mut alt = 2.0;
            let mut pitch_deg = -35.0f64;
            let mut numbers = 0;
            for w in arg.split_whitespace() {
                match w {
                    "village" => kind = Some(0u8),
                    "town" => kind = Some(1),
                    "city" => kind = Some(2),
                    "mega" => kind = Some(3),
                    _ => {
                        if let Ok(v) = w.parse::<f64>() {
                            if numbers == 0 { alt = v } else { pitch_deg = v.min(-2.0) }
                            numbers += 1;
                        }
                    }
                }
            }
            if let Some(target @ ObjectRef::Body(i)) = cam.selected {
                let b = &cat.bodies[i as usize];
                // The body's civilization as its terrain has it (a level 4 civilization's colonies too),
                // else as the catalog and `SE_CIV_TEST` give it.
                let name = b.names.first().cloned().unwrap_or_default();
                let civ = bodies.iter().find(|(sb, _)| sb.obj == target).and_then(|(_, t)| t.and_then(|t| t.civ)).or_else(|| {
                    b.civilization()
                        .or(crate::system::civ_test().filter(|_| crate::scatter::land_plants(&b.block)))
                        .map(|l| (l, se_core::procgen::civilization::culture_seed(&name)))
                });
                if let Some((level, culture)) = civ {
                    let eph = Ephemeris::new(cat);
                    let radius = eph.radius_m(target);
                    let q = eph.orientation(target, sim.jd);
                    let tp = eph.position(target, sim.jd);
                    let dir = (q.inverse() * cam.pos.delta_m(tp)).normalize_or(DVec3::Y);
                    // The ground's seed: the culture seed's four digits through f32, as the shader takes them.
                    let frac = (culture % 10_000) as f32 / 10_000.0;
                    let seed = (frac * 10_000.0) as u32;
                    let kinds: Vec<u8> = kind.map_or_else(|| (0..=3).rev().collect(), |k| vec![k]);
                    let found = kinds.iter().find_map(|&k| {
                        se_core::procgen::civilization::nearest_settlement(dir, radius, level, seed, k, 3000.0).map(|t| (k, t))
                    });
                    if let Some((k, (d, r_km))) = found {
                        let (lat, lon) = (d.y.asin(), (-d.z).atan2(d.x));
                        info!("town: a {} of {:.1} km at {:.3} {:.3} on {name} (level {level})", ["village", "town", "city", "megalopolis"][k as usize], r_km, lat.to_degrees(), lon.to_degrees());
                        let up = DVec3::new(lat.cos() * lon.cos(), lat.sin(), -lat.cos() * lon.sin());
                        let north = (DVec3::Y - up * up.y).normalize_or(DVec3::X);
                        let pitch = pitch_deg.to_radians();
                        let fwd = (north * pitch.cos() + up * pitch.sin()).normalize();
                        let shape = DVec3::from_array(b.shape());
                        // Back from the middle by the view's run, so the town is ahead.
                        let back = alt * 1000.0 / (-pitch).tan();
                        let at = (up * radius - north * back).normalize();
                        cam.pos = tp.offset_m(q * (at * radius * shape + at * (alt * 1000.0)));
                        cam.rot = look_rotation(q * fwd, q * up);
                        cam.velocity = DVec3::ZERO;
                        cam.follow = Some(Follow { target, last_pos: tp, last_rot: q, locked: true, sync: true });
                    } else {
                        info!("town: no settlement within 3000 km on {name}");
                    }
                } else {
                    info!("town: {name} has no civilization");
                }
            }
        }
        "search" => {
            // search LY [life|bh]: the Star browser's search (shown with SE_BROWSER_OPEN=search).
            let mut it = arg.split_whitespace();
            let ly = it.next().and_then(|s| s.parse().ok()).unwrap_or(50.0);
            search.dev_search(&data, &cam, procstars.as_deref(), sim.jd, ly, it.next().unwrap_or(""));
        }
        "shot" => {
            // shot PATH: save a screenshot and carry on.
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(std::path::PathBuf::from(arg.clone())));
        }
        "rs" => {
            // rs NAME: jump next to a procedural star by its RS name (it then becomes the active system).
            match procstars.as_ref().and_then(|f| f.find_by_name(&arg)) {
                Some(p) => {
                    cam.pos = UPos::from_parsecs(p + DVec3::new(0.0, 0.0, 0.002));
                    cam.follow = None;
                    cam.travel = None;
                    cam.velocity = DVec3::ZERO;
                }
                None => warn!("procedural star not found: {arg}"),
            }
        }
        "at" => {
            // at X Y Z: place the camera at universe-frame parsecs from the Sun.
            let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if v.len() == 3 {
                cam.pos = UPos::from_parsecs(DVec3::new(v[0], v[1], v[2]));
                cam.follow = None;
                cam.travel = None;
                cam.velocity = DVec3::ZERO;
            }
        }
        "gal" => {
            // gal L B DIST: galactic longitude/latitude (deg) and distance (pc) from the Sun.
            let v: Vec<f64> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if v.len() == 3 {
                let (l, b) = (v[0].to_radians(), v[1].to_radians());
                let g = DVec3::new(b.cos() * l.cos(), b.cos() * l.sin(), b.sin());
                let u = se_core::coords::galactic_to_universe_quat() * g;
                cam.pos = UPos::from_parsecs(u * v[2]);
                cam.follow = None;
                cam.travel = None;
                cam.velocity = DVec3::ZERO;
            }
        }
        "child" => {
            // child N [dist_km]: go to the Nth body (1-based, by orbit size) of the active system.
            let mut it = arg.split_whitespace();
            let n: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
            let dist = it.next().and_then(|s| s.parse::<f64>().ok()).map(|k| k * 1000.0);
            if let Some(root) = active_root {
                let root_name = cat.stars[root as usize].names[0].clone();
                let mut kids: Vec<(f64, ObjectRef)> = cat
                    .system_members(&root_name)
                    .into_iter()
                    .filter_map(|r| match r {
                        ObjectRef::Body(i) => {
                            let b = &cat.bodies[i as usize];
                            let a = |o: Option<&se_core::orbit::KeplerOrbit>| o.map(|o| o.semi_major_axis).unwrap_or(0.0);
                            match cat.find(&b.parent) {
                                // A binary planet counts once, by its primary, at its barycentre's orbit.
                                Some(ObjectRef::Star(s)) if se_core::climate::planet_barycentre(cat, s) => {
                                    let heaviest = cat.children_of(&b.parent).iter().all(|c| match c {
                                        ObjectRef::Body(j) => cat.bodies[*j as usize].mass_kg <= b.mass_kg,
                                        _ => true,
                                    });
                                    heaviest.then(|| (a(cat.stars[s as usize].orbit.as_ref()), r))
                                }
                                Some(ObjectRef::Star(_)) => Some((a(b.orbit.as_ref()), r)),
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                    .collect();
                kids.sort_by(|a, b| a.0.total_cmp(&b.0));
                if let Some(&(_, r)) = kids.get(n.saturating_sub(1)) {
                    cam.selected = Some(r);
                    start_travel(&mut cam, r, dist);
                    if let Some(t) = cam.travel.as_mut() {
                        t.duration = 0.01;
                        t.set_speed = false;
                    }
                } else {
                    warn!("active system has only {} planets", kids.len());
                }
            }
        }
        "nearest" => {
            // Select and fly to the nearest star (catalog or materialized procedural).
            if let Some(root) = active_root {
                cam.selected = Some(ObjectRef::Star(root));
                start_travel(&mut cam, ObjectRef::Star(root), None);
                if let Some(t) = cam.travel.as_mut() {
                    t.duration = 0.01;
                    t.set_speed = false;
                }
            }
        }
        "place" => {
            // place NAME: jump to a saved SpaceEngine place (case-insensitive substring).
            let needle = arg.to_lowercase();
            if let Some(p) = places.0.iter().find(|p| p.name.to_lowercase().contains(&needle)) {
                cam.pos = UPos::from_meters(DVec3::from_array(p.pos_m));
                cam.rot = DQuat::from_xyzw(p.rot[0], p.rot[1], p.rot[2], p.rot[3]).normalize();
                cam.follow = None;
                cam.travel = None;
                cam.velocity = DVec3::ZERO;
                cam.selected = find(&p.body);
                sim.jd = p.jd;
                sim.paused = true;
            } else {
                warn!("no place matching {arg}");
            }
        }
        "galactic" => {
            // Jump to galactic-scale view: distance in parsecs from the Sun toward NGP.
            let d: f64 = arg.parse().unwrap_or(30_000.0);
            let mw = find("Milky Way");
            let center = mw
                .map(|r| Ephemeris::new(cat).position(r, sim.jd))
                .unwrap_or(UPos::ZERO);
            let ngp = se_core::coords::galactic_north_pole();
            let off = (ngp * 0.8 + se_core::coords::galactic_center_dir() * -0.6).normalize();
            cam.pos = center.offset_m(off * d * se_core::units::METERS_PER_PC);
            cam.rot = look_rotation(center.delta_m(cam.pos), ngp);
            cam.follow = None;
        }
        "ship" | "throttle" | "warp" | "flight" | "unship" | "shipcam" | "shiporbit" | "shiprcs" | "shipturn" => {
            // Spacecraft commands are handled by ship.rs.
            ship_cmds.write(crate::ship::ShipCommand { verb: verb.clone(), arg: arg.clone() });
        }
        // mods list | on NAME | off | window [off] | sound NAME: the Mods menu (mods.rs).
        "mods" => commands.queue(move |w: &mut World| crate::mods::dev_command(w, &arg)),
        // hero keys KEYS SECS | aim Y P | state | trace FILE [BODY] | clip NAME|auto | light auto|free: the Viltrumite
        // mod's hero (viltrumite::dev_command).
        "hero" => commands.queue(move |w: &mut World| crate::viltrumite::dev_command(w, &arg)),
        // uiclick X Y: a left click for egui at that point of the window (test runs have no mouse).
        "uiclick" => commands.queue(move |w: &mut World| crate::mods::dev_ui_click(w, &arg)),
        // script PATH: start a script, as the Scripts window does.
        "script" => commands.queue(move |w: &mut World| w.resource_mut::<crate::scripting::ScriptRunner>().request(&arg)),
        // `perf NAME` / `perf end`: frame-time segments for SE_PERF (perf.rs).
        // preset NAME: pick a graphics preset (Low ... Ultra), as the Graphics settings do; test runs
        // otherwise keep High, since they skip the user's settings.
        "preset" => commands.queue(move |w: &mut World| {
            if let Some(mut g) = w.get_resource_mut::<crate::graphics::GraphicsSettings>() {
                g.pick_preset(&arg);
            }
        }),
        // vclouds 0|1|2: the volumetric clouds off (SE's flat clouds), High or Ultra (comparisons).
        "vclouds" => commands.queue(move |w: &mut World| {
            if let Some(mut g) = w.get_resource_mut::<crate::graphics::GraphicsSettings>() {
                g.quality.volumetric_clouds = arg.trim().parse::<u32>().unwrap_or(2).min(2);
            }
        }),
        // levels V [C]: the Vegetation and Cities settings (0 Off, 1 Low, 2 High, 3 Ultra; C defaults to
        // V). Test runs use the High preset: `levels 3` measures what the user's Custom settings draw.
        "levels" => commands.queue(move |w: &mut World| {
            let mut it = arg.split_whitespace().filter_map(|a| a.parse::<u32>().ok());
            let v = it.next().unwrap_or(3).min(3);
            let c = it.next().unwrap_or(v).min(3);
            if let Some(mut g) = w.get_resource_mut::<crate::graphics::GraphicsSettings>() {
                g.quality.vegetation = v;
                g.quality.cities = c;
            }
        }),
        // scatter on|off|test: the GPU scatter (vegetation) on or off, or its test (Megaton Rainfall's
        // trees evenly on any planet's land).
        "scatter" => commands.queue(move |w: &mut World| {
            if let Some(mut s) = w.get_resource_mut::<crate::scatter::Scatter>() {
                let a = arg.trim();
                s.enabled = !matches!(a, "off" | "0");
                s.test = a == "test";
                s.reset_body();
            }
        }),
        "perf" => commands.queue(move |w: &mut World| {
            if let Some(mut p) = w.get_resource_mut::<crate::perf::PerfLog>() {
                p.mark(&arg);
            }
        }),
        // photo 0|1|2: SE's photo mode, HDR, Manual or Auto (exposure); `ambient X`: SE's
        // AmbientLighting (test runs skip the user's settings).
        "photo" => commands.queue(move |w: &mut World| {
            if let Some(mut p) = w.get_resource_mut::<crate::exposure::PhotoMode>() {
                *p = crate::exposure::PhotoMode::new(arg.trim().parse::<u8>().unwrap_or(0));
            }
        }),
        // (Exposure compensation in stops, for measuring dark pictures: a night's trees are 0 or 1 in a PNG.)
        "exposure" => commands.queue(move |w: &mut World| {
            if let Some(mut e) = w.get_resource_mut::<crate::exposure::ExposureComp>() {
                e.set(arg.trim().parse::<f32>().unwrap_or(0.0));
            }
        }),
        "ambient" => commands.queue(move |w: &mut World| {
            if let Some(mut a) = w.get_resource_mut::<crate::system::AmbientLighting>() {
                a.0 = arg.trim().parse::<f32>().unwrap_or(0.0).clamp(0.0, 1.0);
            }
        }),
        other => warn!("unknown command: {other}"),
    }
}

fn screenshot_and_exit(
    mut commands: Commands,
    cfg: Res<EngineConfig>,
    q: Res<CommandQueue>,
    mut state: ResMut<ScreenshotState>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = cfg.screenshot.clone() else { return };
    if !q.queue.is_empty() || q.wait_frames > 0 {
        return;
    }
    state.frames += 1;
    if !state.taken && state.frames >= cfg.screenshot_frames {
        state.taken = true;
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    } else if state.taken && (path.exists() || state.frames >= cfg.screenshot_frames + 600) {
        // Give the async readback a moment to finish writing.
        if state.frames >= cfg.screenshot_frames + 5 {
            exit.write(AppExit::Success);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use std::sync::Arc;
    use std::time::Duration;

    const DT: f64 = 0.01;

    /// A star and a planet on a fast orbit (about 300 km/s), with the camera 10 radii out.
    fn setup() -> (World, ObjectRef) {
        let mut cat = se_core::catalog::Catalog::new();
        cat.load_sc(
            r#"Star "Sun" { RA 0 Dec 0 Dist 0 Class "G2V" MassSol 1 }
Planet "P" { ParentBody "Sun" Radius 6000 Mass 1 Orbit { SemiMajorAxis 1 Period 0.1 } }"#,
            "test",
        );
        cat.finalize();
        let planet = cat.find("P").unwrap();
        let mut world = World::new();
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs_f64(DT));
        world.insert_resource(time);
        let sim = SimTime { jd: se_core::time::J2000, rate: 1.0, paused: false };
        let pos = Ephemeris::new(&cat).position(planet, sim.jd);
        let cam = FlyCamera { pos: pos.offset_m(DVec3::X * 6.0e7), selected: Some(planet), ..default() };
        world.insert_resource(sim);
        world.insert_resource(cam);
        world.insert_resource(FlightHold::default());
        world.insert_resource(crate::mods::ModControl::default());
        world.insert_resource(crate::viltrumite::Hero::default());
        world.insert_resource(SeData { vfs: Arc::new(se_core::vfs::Vfs::new()), catalog: Arc::new(cat) });
        (world, planet)
    }

    /// Run `n` frames of `travel_and_move`, advancing the simulation clock.
    fn step(world: &mut World, n: usize) {
        for _ in 0..n {
            world.resource_mut::<SimTime>().jd += DT / 86_400.0;
            world.run_system_once(travel_and_move).unwrap();
        }
    }

    fn offset(world: &World, planet: ObjectRef) -> DVec3 {
        let cat = &world.resource::<SeData>().catalog;
        let jd = world.resource::<SimTime>().jd;
        world.resource::<FlyCamera>().pos.delta_m(Ephemeris::new(cat).position(planet, jd))
    }

    #[test]
    fn interrupted_goto_stays_with_the_target() {
        let (mut world, planet) = setup();
        start_travel(&mut world.resource_mut::<FlyCamera>(), planet, None);
        step(&mut world, 100);
        // Moving or looking around cancels the flight halfway.
        world.resource_mut::<FlyCamera>().travel = None;
        let before = offset(&world, planet);
        step(&mut world, 300);
        // The planet moved ~900 km meanwhile; the camera kept its place relative to it.
        assert!((offset(&world, planet) - before).length() < 1.0, "{:?} -> {:?}", before, offset(&world, planet));
    }

    #[test]
    fn goto_from_inside_the_arrival_distance_does_not_jump() {
        let (mut world, planet) = setup();
        // 1.5 radii out; Go to arrives at 4 radii.
        let start = DVec3::X * 9.0e6;
        let pos = world.resource::<FlyCamera>().pos.offset_m(start - offset(&world, planet));
        world.resource_mut::<FlyCamera>().pos = pos;
        start_travel(&mut world.resource_mut::<FlyCamera>(), planet, None);
        step(&mut world, 1);
        let d1 = offset(&world, planet).length();
        assert!((d1 - 9.0e6).abs() < 1.0e5, "first frame at {d1}");
        step(&mut world, 500);
        assert!((offset(&world, planet).length() - 2.4e7).abs() < 1.0e3);
    }

    #[test]
    fn travels_and_binds_are_counted() {
        // A script's Control|Goto / Follow wait compares these with their values when it began, so
        // a flight under way or a binding already there does not hide a new one.
        let (world, planet) = setup();
        let cat = world.resource::<SeData>().catalog.clone();
        let eph = Ephemeris::new(&cat);
        let mut cam = world.resource::<FlyCamera>().clone();
        start_travel(&mut cam, planet, None);
        start_center(&mut cam, planet);
        assert_eq!(cam.travel_serial, 2);
        cam.bind(Some(planet), false, &eph, 0.0);
        cam.bind(Some(planet), false, &eph, 0.0);
        assert_eq!(cam.bind_serial, 2);
        // Locked: refused, not counted.
        cam.locks.movement = true;
        start_travel(&mut cam, planet, None);
        cam.bind(None, false, &eph, 0.0);
        assert_eq!((cam.travel_serial, cam.bind_serial), (2, 2));
    }

    #[test]
    fn zoom_floor_only_for_solid_bodies() {
        // Outside a planet the zoom stops at its surface.
        assert_eq!(zoom_distance(2.0e7, 0.01, 6.0e6, true), 6.0e6 * 1.0001);
        // Inside a galaxy any zoom stays inside instead of jumping to its edge.
        let r = 1.5e21;
        assert_eq!(zoom_distance(1.0e20, 0.5, r, false), 5.0e19);
        assert_eq!(zoom_distance(1.0e20, 1.25, r, false), 1.25e20);
        // From outside it, not into it.
        assert_eq!(zoom_distance(2.0e21, 0.5, r, false), r * 1.0001);
    }

    #[test]
    fn horizon_turns_keep_the_roll() {
        let up = DVec3::new(0.3, 0.9, -0.2).normalize();
        // A rolled view looking a little down.
        let rot = (look_rotation(DVec3::new(1.0, -0.2, 0.4), up) * DQuat::from_rotation_z(0.3)).normalize();
        let roll_of = |q: DQuat| {
            let level = look_rotation(q * DVec3::NEG_Z, up);
            let d = level.inverse() * q;
            2.0 * d.z.atan2(d.w)
        };
        let mut q = rot;
        for _ in 0..50 {
            q = horizon_turn(q, up, 0.13, 0.02);
        }
        assert!((roll_of(q) - 0.3).abs() < 1e-6, "{}", roll_of(q));
        // Pitch stops short of straight up.
        let q = horizon_turn(rot, up, 0.0, 3.0);
        assert!((q * DVec3::NEG_Z).dot(up) > 0.9999 && (q * DVec3::NEG_Z).dot(up) < 1.0);
    }

    #[test]
    fn goto_times() {
        // A short hop takes CenterTime, 1 AU to Earth's 4 radii about 4.8 s, the interstellar
        // flight to Vega about 8.5 s, from across the universe to a planet LongGotoTime.
        assert_eq!(goto_time(3.0e7, 2.55e7), CENTER_TIME);
        assert!((goto_time(1.496e11, 2.55e7) - 4.77).abs() < 0.01);
        assert!((goto_time(2.39e17, 7.84e9) - 8.48).abs() < 0.01);
        assert_eq!(goto_time(1.0e27, 1.0e7), LONG_GOTO_TIME);
    }

    #[test]
    fn center_keeps_the_binding() {
        let (mut world, planet) = setup();
        start_travel(&mut world.resource_mut::<FlyCamera>(), planet, None);
        step(&mut world, 500);
        let before = offset(&world, planet);
        world.resource_mut::<FlyCamera>().travel =
            Some(Travel { target: planet, kind: TravelKind::Center, start_off: None, start_rot: DQuat::IDENTITY, end_distance: -1.0, elapsed: 0.0, duration: 0.8, set_speed: false });
        step(&mut world, 200);
        assert!(world.resource::<FlyCamera>().follow.is_some());
        assert!((offset(&world, planet) - before).length() < 1.0);
    }

    #[test]
    fn move_modes_switch_like_se() {
        let mut cam = FlyCamera { speed: 1000.0, ..default() };
        // Entering an inertial mode makes a tenth of the velocity the acceleration, leaving it undoes that.
        cam.set_move_mode(MoveMode::Spacecraft);
        assert_eq!((cam.move_mode, cam.speed), (MoveMode::Spacecraft, 100.0));
        cam.set_move_mode(MoveMode::Aircraft);
        assert_eq!(cam.speed, 100.0);
        cam.set_move_mode(MoveMode::Free);
        assert_eq!(cam.speed, 1000.0);
        // Go to's arrival speed is a tenth as an acceleration.
        cam.set_move_mode(MoveMode::Spacecraft);
        cam.assign_velocity(2.5e6);
        assert_eq!(cam.speed, 2.5e5);
        assert_eq!(MoveMode::ALL.map(MoveMode::number), [1, 2, 3]);
        assert_eq!(MoveMode::from_number(4), None);
        assert_eq!(MoveMode::from_number(3), Some(MoveMode::Aircraft));
        assert_eq!(format_acceleration(9.8), "9.8 m/s²");
        assert_eq!(format_acceleration(2.0e4), "20.000 km/s²");
    }

    #[test]
    fn aircraft_velocity_turns_with_the_view() {
        let prev = look_rotation(DVec3::X, DVec3::Y);
        let rot = (prev * DQuat::from_rotation_y(0.5)).normalize();
        let v = prev * DVec3::new(0.0, 0.0, -100.0);
        let w = aircraft_turn(prev, rot, v);
        assert!((w - rot * DVec3::new(0.0, 0.0, -100.0)).length() < 1e-9);
    }

    #[test]
    fn zoom_like_se() {
        // Narrowing is fast at first (t^0.5), widening slow at first (t^2); both end at ZoomTime.
        let mut a = ZoomAnim { from: 45.0, to: 20.0, elapsed: ZOOM_TIME_S * 0.25 };
        assert!((zoom_anim_fov(&a) - 32.5).abs() < 1e-4);
        a.elapsed = ZOOM_TIME_S;
        assert_eq!(zoom_anim_fov(&a), 20.0);
        let b = ZoomAnim { from: 20.0, to: 45.0, elapsed: ZOOM_TIME_S * 0.5 };
        assert!((zoom_anim_fov(&b) - 26.25).abs() < 1e-4);
        let mut cam = FlyCamera { fov_target: Some(20.0), ..default() };
        for _ in 0..20 {
            zoom_step(&mut cam, 1.0 / 60.0);
        }
        assert_eq!((cam.fov_deg, cam.fov_target, cam.zoom_anim), (20.0, None, None));
        // The zoom keys: 1.02 a frame at 60 frames a second, within MinFOV..MaxFOV.
        assert!((zoom_key_step(45.0, 1, 1.0 / 60.0) - 45.9).abs() < 1e-4);
        assert!((zoom_key_step(45.0, -1, 1.0) - 45.0 / 1.02f32.powi(60)).abs() < 1e-3);
        assert_eq!(zoom_key_step(119.0, 1, 1.0), MAX_FOV_DEG);
        assert_eq!(zoom_key_step(MIN_FOV_DEG, -1, 1.0), MIN_FOV_DEG);
    }
}
