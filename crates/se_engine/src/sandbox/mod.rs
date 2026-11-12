//! The sandbox (the user's 2026-11-11 feature): the game starts on Earth's ground in first person, with Garry's Mod's
//! movement and controls, weapons and sounds; holding Q opens a spawn menu (props, weapons, the mods to play as); V switches
//! the port's own free flight on and off.
//!
//! `movement.rs` is Half-Life 2's player movement, `world.rs` the props' physics in a local frame on the planet (gravity is the
//! planet's own), `assets.rs` what is read from the Garry's Mod install (models, textures, sounds: nothing of it is copied into
//! the project), `props.rs` and `light.rs` how props and the weapon are drawn and lit like the ground, `weapons.rs` the weapons
//! and the HUD, `menu.rs` the Q menu, `audio.rs` the sounds. This file is the player's state, his keys, each frame's ticks and the
//! camera.
//!
//! Three modes (`Mode`): `Walk` (he owns the camera and the keys), `Fly` (the port's free flight exactly as it was; V) and `Hero`
//! (a mod plays: the Viltrumite hero owns everything, the sandbox only keeps its props going and its Q menu).

pub mod assets;
pub mod audio;
pub mod icon;
pub mod light;
pub mod menu;
pub mod movement;
pub mod props;
pub mod solids;
pub mod soundscript;
pub mod viewmodel;
pub mod weapons;
pub mod world;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, PrimaryEguiContext};
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::units::G;

use crate::camera::{FlyCamera, Follow};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::mods::ModControl;
use crate::scatter::library::ScatterLibrary;
use crate::scatter::probe::{BuildingProbe, ProbeAsk, ProbeBuilding, ProbeCapsule};
use crate::ship::{body_mass, ground_on};
use crate::system::SystemBody;
use crate::terrain::Terrain;
use crate::universe::{SimTime, UniverseSet};

use movement::{Cmd, Player, Punch, Vars, HULL_HALF, HULL_HEIGHT, HULL_HEIGHT_DUCK, TICK, UNIT_M};
use world::{Impact, LocalFrame, Met, SandboxWorld, TerrainGrid};

/// Garry's Mod's `fov_desired` is 75 degrees across a 4:3 screen; Source keeps the height of the view whatever the screen's
/// shape, so that is 59.84 degrees up and down, which is what the port's field of view is.
const WALK_FOV_DEG: f32 = 59.84;
/// `sensitivity 3` x `m_yaw 0.022`: degrees a mouse count (Garry's Mod's `config_default.cfg`).
const MOUSE_DEG: f32 = 3.0 * 0.022;
/// The ground field: samples a side and their spacing. 60 m each way round the anchor.
const GRID_N: usize = 161;
const GRID_CELL: f32 = 0.75;
/// Rows of it sampled a frame while a new one is made (about 2,600 heights: half a millisecond).
const GRID_ROWS_A_FRAME: usize = 16;
/// The frame is moved to the player when he is this far from its anchor (and never further than a small share of the body's
/// radius: on an asteroid "up" turns quickly).
const REBASE_M: f64 = 24.0;
const REBASE_RADII: f64 = 0.002;
/// How far the gap the hull keeps from what it meets is (Source's `DIST_EPSILON` is 0.03 units; rapier's casts are less exact).
const SKIN: f32 = 0.1;
/// First person is offered within this many radii of a body's surface.
const WALK_REACH_RADII: f64 = 3.0;
/// The lowest the eyes may be over the CPU's ground before the camera's clamp lifts them (`FlyCamera::eye_floor_m`).
const EYE_FLOOR_M: f64 = 0.12;
/// Source's `sv_maxvelocity`: nothing moves faster on any axis (a fall's terminal speed: 67 m/s).
const MAX_VELOCITY: f32 = 3500.0;

/// Where the game may start, picked by where the Sun is highest at launch: open, level land round the world (latitude and
/// longitude in degrees, east positive, and the way he faces, degrees from north).
const SPOTS: [(&str, f64, f64, f32); 8] = [
    ("Kansas", 38.52, -98.70, 200.0),
    ("the Pampas", -34.10, -61.20, 20.0),
    ("Salisbury Plain", 51.22, -1.92, 160.0),
    ("the Serengeti", -2.45, 34.85, 100.0),
    ("the Kazakh steppe", 49.60, 69.80, 180.0),
    ("the Mongolian steppe", 47.60, 106.30, 180.0),
    ("New South Wales", -31.90, 146.20, 0.0),
    ("the Canterbury Plains", -43.70, 171.90, 300.0),
];

/// `SE_SANDBOX`: `0` leaves the sandbox out (the port as it was), `1` starts on Earth in a scripted test run too.
fn env_switch() -> Option<bool> {
    static SWITCH: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var("SE_SANDBOX").ok().map(|v| v.trim() != "0"))
}

/// The sandbox exists at all (V, Q).
pub fn enabled() -> bool {
    env_switch() != Some(false)
}

/// This launch starts on Earth's ground in first person: any launch the user makes. Test runs (`--run`, `--screenshot`,
/// `--script`) and `--start NAME` keep the port's own start unless `SE_SANDBOX=1`.
pub fn starts_on_earth(cfg: &EngineConfig) -> bool {
    match env_switch() {
        Some(false) => false,
        Some(true) => true,
        None => !crate::settings::scripted(cfg) && !cfg.start_given,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The port's free flight, as it always was.
    Fly,
    /// First person.
    Walk,
    /// A mod plays (the Viltrumite hero).
    Hero,
}

/// Where he's to be put down once the ground there is known.
#[derive(Clone, Copy, Debug)]
struct Pending {
    obj: ObjectRef,
    /// Body frame, metres from the centre: his eyes if `eyes`, else a point over the spot (dropped to the ground).
    point: DVec3,
    eyes: bool,
    /// Degrees from north, when the view isn't the camera's.
    heading: Option<f32>,
    /// Where the camera waits meanwhile, and its turn: in the body's frame, so it stays over the spot.
    cam_point: DVec3,
    cam_rot: DQuat,
}

/// A new ground field being sampled, a few rows a frame.
struct Build {
    frame: LocalFrame,
    heights: Vec<f32>,
    row: usize,
    /// The time its sampling has taken so far.
    spent: std::time::Duration,
}

/// What a tick did that is heard or seen (`audio.rs`, the view).
#[derive(Clone, Copy, Debug)]
pub enum Event {
    Step { left: bool, speed: f32, ducked: bool },
    Jump,
    /// Landed at this speed down, units/s.
    Land(f32),
    PropImpact(Impact),
}

/// What the menu, the keys and the test commands ask of the props (`props::handle_requests`).
#[derive(Clone, Debug)]
pub enum Request {
    /// A prop by its model's path, where he looks.
    Spawn(String),
    /// The last prop spawned goes (Z).
    Undo,
    Clear,
}

/// The keys of this frame (as `mods::ModInput` is the hero's): read once, before everything else.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct WalkInput {
    pub forward: f32,
    pub side: f32,
    pub jump: bool,
    pub duck: bool,
    pub sprint: bool,
    pub walk: bool,
    pub attack: bool,
    pub attack_pressed: bool,
    pub attack2: bool,
    pub attack2_pressed: bool,
    pub use_pressed: bool,
    pub use_held: bool,
    pub reload_pressed: bool,
    /// R is down.
    pub reload: bool,
    pub undo_pressed: bool,
    pub flashlight_pressed: bool,
    /// 1..=9 pressed.
    pub slot: Option<u8>,
    pub wheel: f32,
    /// Counts the mouse moved.
    pub mouse: Vec2,
    /// V was pressed: the free flight's switch.
    pub toggle_flight: bool,
}

#[derive(Resource)]
pub struct Sandbox {
    pub mode: Mode,
    /// The mode a mod took over from, to go back to.
    pub(crate) before_hero: Mode,
    pending: Option<Pending>,
    pub world: Option<SandboxWorld>,
    pub player: Player,
    /// On the ground field: he moves. Until then he's held where he was put.
    pub placed: bool,
    /// The view, Source's angles in radians: yaw from east, turning left; pitch up.
    pub yaw: f32,
    pub pitch: f32,
    pub punch: Punch,
    pub vars: Vars,
    /// Seconds of real time not yet ticked.
    accumulator: f32,
    /// The player's origin a tick ago and now, and the eyes' height: drawn between the two.
    prev_origin: Vec3,
    prev_view_z: f32,
    build: Option<Build>,
    /// Ticks until the ground field is next checked against the terrain.
    check_in: u32,
    /// The Q menu is held open.
    pub menu: bool,
    /// Units walked since the last footstep, and which foot is next.
    step_dist: f32,
    step_left: bool,
    /// What the last frame's ticks did.
    pub events: Vec<Event>,
    /// The ticks ever run (the tests' clock).
    pub ticks: u64,
    /// The eyes this frame in the local frame (metres) and the view's forward and up there: what the weapon and the menu use.
    pub eye: Vec3,
    pub forward: Vec3,
    /// Gravity at the player, m/s^2 (the planet's own).
    pub gravity: f32,
    /// A dev command's view angles for the next frame (degrees).
    dev_look: Option<(f32, f32)>,
    log_state: bool,
    /// A test holds the Q menu open.
    dev_menu: bool,
    cursor: CursorState,
    /// What was asked of the props this frame, and the sounds to play (`audio::play_cues`).
    pub requests: Vec<Request>,
    pub cues: Vec<audio::Cue>,
    pub rng: audio::Rng,
    /// What he stood on a tick ago (a prop's surface; `None` the ground): a jump's sound is its.
    pub last_ground: Option<String>,
    /// The ground's own surface property, for his steps and what strikes it (`audio::ground_name`).
    pub ground_material: &'static str,
    /// The buildings and trees made solid round him (`solids.rs`).
    pub solids: solids::Solids,
    /// The sound that goes on while a gun holds a prop: its file and volume.
    pub loop_sound: Option<(&'static str, f32)>,
    /// The prop a gun holds, or his hands (E: `by_hand`).
    pub held: Option<u32>,
    pub by_hand: bool,
    /// The mouse turns the held prop, not the view (the physics gun's E): this frame's counts are in `spin`.
    pub rotating: bool,
    pub spin: Vec2,
}

#[derive(Default)]
struct CursorState {
    focused: Option<bool>,
    /// Frames left in which the mouse doesn't count: the cursor is on its way back to locked.
    regain: u8,
    menu: bool,
    walking: bool,
}

impl Default for Sandbox {
    fn default() -> Self {
        Sandbox {
            mode: Mode::Fly,
            before_hero: Mode::Fly,
            pending: None,
            world: None,
            player: Player::at(Vec3::ZERO),
            placed: false,
            yaw: 0.0,
            pitch: 0.0,
            punch: Punch::default(),
            vars: Vars::default(),
            accumulator: 0.0,
            prev_origin: Vec3::ZERO,
            prev_view_z: movement::VIEW_Z,
            build: None,
            check_in: 0,
            menu: false,
            step_dist: 0.0,
            step_left: true,
            events: Vec::new(),
            ticks: 0,
            eye: Vec3::ZERO,
            forward: Vec3::X,
            gravity: 9.81,
            dev_look: None,
            log_state: false,
            dev_menu: false,
            cursor: CursorState::default(),
            requests: Vec::new(),
            cues: Vec::new(),
            rng: audio::Rng(0x5EED_1234),
            last_ground: None,
            ground_material: "grass",
            solids: solids::Solids::default(),
            loop_sound: None,
            held: None,
            by_hand: false,
            rotating: false,
            spin: Vec2::ZERO,
        }
    }
}

impl Sandbox {
    pub fn walking(&self) -> bool {
        self.mode == Mode::Walk
    }

    /// How far this frame is between the last two ticks, 0..1: what moves by ticks is drawn there.
    pub fn tick_fraction(&self) -> f32 {
        (self.accumulator / TICK).clamp(0.0, 1.0)
    }

    /// The view's forward in the local frame (x east, y north, z up).
    fn view_forward(&self, pitch_extra_deg: f32, yaw_extra_deg: f32) -> Vec3 {
        let (p, y) = (self.pitch + pitch_extra_deg.to_radians(), self.yaw + yaw_extra_deg.to_radians());
        Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
    }
}

pub struct SandboxPlugin;

impl Plugin for SandboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sandbox>().init_resource::<WalkInput>().init_resource::<assets::GmodAssets>().init_resource::<props::Props>().init_resource::<menu::Menu>().init_resource::<weapons::Arsenal>();
        if !enabled() {
            return;
        }
        light::build(app);
        viewmodel::build(app);
        app.add_systems(Startup, (find_gmod, audio::spawn_ears))
            .add_systems(Update, (gmod_found.run_if(resource_exists::<GmodSearch>), props::handle_requests))
            .add_systems(bevy_egui::EguiPrimaryContextPass, (weapons::draw_hud, menu::draw_menu).chain().in_set(crate::skin::UiSet::Draw))
            .add_systems(PostUpdate, (weapons::weapon_frame, audio::play_cues).chain().after(sandbox_step))
            .add_systems(Update, audio::log_voices)
            .add_systems(
                PostUpdate,
                // Where things are drawn this frame: after the camera is where the frame draws from, placed from it as the
                // floating origin places what it moves.
                (props::place_props, weapons::place_weapon)
                    .after(UniverseSet::Camera)
                    .after(weapons::weapon_frame)
                    .after(crate::terrain::update_node_transforms)
                    .before(UniverseSet::FloatingOrigin)
                    .before(bevy::camera::CameraUpdateSystems),
            )
            .add_systems(PostStartup, start_on_earth.after(crate::session::restore_session))
            // His keys and the cursor: after the frame's input (and a test's pressed keys), before the mod's snapshot (which
            // must see the Q menu) and before everything else reads the keys.
            .add_systems(
                PreUpdate,
                (
                    read_input.after(bevy::input::InputSystems).after(crate::viltrumite::dev_keys).before(crate::mods::snapshot_input),
                    strip_egui_input.after(bevy_egui::EguiPreUpdateSet::ProcessInput).before(bevy_egui::EguiPreUpdateSet::BeginPass),
                ),
            )
            .add_systems(
                PostUpdate,
                // With the camera's own motion, as a ship's and the hero's, before its transform is set.
                sandbox_step
                    .in_set(UniverseSet::Camera)
                    .after(crate::camera::travel_and_move)
                    // (Before the hero's: the frame a mod takes over, he's placed from the camera, which this has just put
                    // at the player's eyes for this frame's place of the body.)
                    .before(crate::viltrumite::hero::hero_move)
                    .before(crate::camera::sync_camera_transform),
            )
            .add_systems(
                PostUpdate,
                // The first-person near plane, after everything that sets it.
                near_plane.after(crate::terrain::keep_camera_above_ground).after(crate::camera::sync_camera_transform).before(bevy::camera::CameraUpdateSystems),
            );
    }
}

/// The search for Garry's Mod's files, on a worker thread.
#[derive(Resource)]
struct GmodSearch(bevy::tasks::Task<assets::Opened>);

fn find_gmod(mut commands: Commands) {
    commands.insert_resource(GmodSearch(bevy::tasks::AsyncComputeTaskPool::get().spawn(async { assets::open_install() })));
}

fn gmod_found(mut commands: Commands, mut search: ResMut<GmodSearch>, mut found: ResMut<assets::GmodAssets>) {
    let Some(opened) = bevy::tasks::futures::check_ready(&mut search.0) else { return };
    for line in &opened.lines {
        info!("sandbox: {line}");
    }
    found.install = opened.install;
    found.lists = opened.lists;
    found.scripts = opened.scripts;
    found.views = opened.views;
    found.looked = true;
    commands.remove_resource::<GmodSearch>();
}

/// The launch: on Earth's ground, where the Sun is highest among `SPOTS`.
fn start_on_earth(cfg: Res<EngineConfig>, data: Res<SeData>, time: Res<SimTime>, mut cam: ResMut<FlyCamera>, mut sb: ResMut<Sandbox>) {
    if !starts_on_earth(&cfg) {
        return;
    }
    let cat = &data.catalog;
    let (Some(earth), Some(sun)) = (cat.find("Earth"), cat.find("Sun")) else {
        warn!("sandbox: no Earth in the catalog: the port's own start stays");
        return;
    };
    let eph = Ephemeris::new(cat);
    let q = eph.orientation(earth, time.jd);
    let to_sun = q.inverse() * eph.position(sun, time.jd).delta_m(eph.position(earth, time.jd)).normalize_or(DVec3::X);
    let forced = std::env::var("SE_SANDBOX_SPOT").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|i| *i < SPOTS.len());
    let best = forced.unwrap_or_else(|| {
        let height = |i: usize| sphere_dir(SPOTS[i].1, SPOTS[i].2).dot(to_sun);
        (0..SPOTS.len()).max_by(|a, b| height(*a).total_cmp(&height(*b))).unwrap_or(0)
    });
    let (name, lat, lon, heading) = SPOTS[best];
    info!("sandbox: starting on Earth in {name} ({lat:.2}, {lon:.2}), the Sun {:.0} degrees up", sphere_dir(lat, lon).dot(to_sun).asin().to_degrees());
    cam.selected = Some(earth);
    place_on(&mut sb, &mut cam, &eph, time.jd, earth, DVec3::from_array(body_shape(cat, earth)), lat, lon, heading);
}

fn body_shape(cat: &se_core::catalog::Catalog, obj: ObjectRef) -> [f64; 3] {
    match obj {
        ObjectRef::Body(i) => cat.bodies[i as usize].shape(),
        _ => [1.0; 3],
    }
}

/// SE's body frame: +X at longitude 0, +Y north, +Z at 90 W.
fn sphere_dir(lat_deg: f64, lon_deg: f64) -> DVec3 {
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    DVec3::new(lat.cos() * lon.cos(), lat.sin(), -lat.cos() * lon.sin())
}

/// Put him on a body at a latitude and longitude, facing `heading` degrees from north: the camera goes there now (so the
/// ground there loads) and he's set down once it's known.
#[allow(clippy::too_many_arguments)]
fn place_on(sb: &mut Sandbox, cam: &mut FlyCamera, eph: &Ephemeris, jd: f64, obj: ObjectRef, shape: DVec3, lat: f64, lon: f64, heading: f32) {
    let radius = eph.radius_m(obj);
    let q = eph.orientation(obj, jd);
    let centre = eph.position(obj, jd);
    let dir = sphere_dir(lat, lon);
    // Well over any ground there can be: the eyes are dropped onto it.
    let point = dir * shape * radius + dir * 12_000.0;
    let frame = LocalFrame::at(obj, shape, dir * shape * radius);
    let fwd = frame.axes.y_axis * (heading.to_radians().cos() as f64) + frame.axes.x_axis * (heading.to_radians().sin() as f64);
    let cam_point = dir * shape * radius + dir * 2.0;
    let cam_rot = crate::camera::look_rotation(fwd, frame.axes.z_axis);
    cam.pos = centre.offset_m(q * cam_point);
    cam.rot = (q * cam_rot).normalize();
    cam.velocity = DVec3::ZERO;
    cam.travel = None;
    cam.follow = Some(Follow::new(obj, true, eph, jd));
    sb.pending = Some(Pending { obj, point, eyes: false, heading: Some(heading), cam_point, cam_rot });
    enter_walk(sb);
}

fn enter_walk(sb: &mut Sandbox) {
    sb.mode = Mode::Walk;
    sb.placed = false;
    sb.build = None;
    sb.accumulator = 0.0;
    sb.punch = Punch::default();
}

/// Nothing else moves the camera while he walks, and it is bound to the body as SE's "rotate with object" binds it, as of
/// this frame's place of the body: free flight or a mod taking it over carries it on from exactly here.
fn bind_camera(cam: &mut FlyCamera, b: &SystemBody) {
    cam.velocity = DVec3::ZERO;
    cam.travel = None;
    cam.track = false;
    cam.hold_agl = None;
    cam.follow = Some(Follow { target: b.obj, last_pos: b.pos, last_rot: b.orient, locked: true, sync: true });
}

/// The frame's keys for the walking player, the cursor, V and the Q menu. Nothing else sees his keys' presses.
#[allow(clippy::too_many_arguments)]
fn read_input(
    mut input: ResMut<WalkInput>,
    mut sb: ResMut<Sandbox>,
    mut control: ResMut<ModControl>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut egui: EguiContexts,
    window: Single<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    cfg: Res<EngineConfig>,
    hold: Res<crate::camera::FlightHold>,
) {
    let (mut window, mut cursor) = window.into_inner();
    let sb = &mut *sb;
    let real = crate::mods::real_window(&cfg);
    let focused = !real || window.focused;
    let typing = egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    *input = WalkInput::default();
    // V: the free flight's switch and nothing else (not while a ship is flown: its keys are its own).
    if !typing && !hold.0 && keys.just_pressed(KeyCode::KeyV) && !(ctrl || alt) {
        input.toggle_flight = true;
        keys.clear_just_pressed(KeyCode::KeyV);
    }
    // Q held: the menu, walking or playing a mod. (In free flight Q stays the roll key.)
    let menu = (!typing && focused && sb.mode != Mode::Fly && keys.pressed(KeyCode::KeyQ)) || (sb.dev_menu && sb.mode != Mode::Fly);
    if sb.menu != menu {
        sb.menu = menu;
    }
    if control.spawn_menu != menu {
        control.spawn_menu = menu;
    }
    let walking = sb.mode == Mode::Walk;
    if control.walker != walking {
        control.walker = walking;
    }
    let st = &mut sb.cursor;
    if st.focused != Some(focused) {
        if focused {
            st.regain = 3;
        }
        st.focused = Some(focused);
    }
    if (st.menu && !menu) || (walking && !st.walking) {
        st.regain = 3;
    }
    st.menu = menu;
    let was_walking = std::mem::replace(&mut st.walking, walking);
    if !walking {
        if was_walking && real && sb.mode == Mode::Fly {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        }
        return;
    }
    // No text field keeps the keys while he walks with the cursor locked.
    if !menu {
        if let Ok(ctx) = egui.ctx_mut() {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }
    if !typing && focused {
        let held = |k: KeyCode| keys.pressed(k);
        let axis = |plus: KeyCode, minus: KeyCode| (held(plus) as i8 - held(minus) as i8) as f32;
        input.forward = axis(KeyCode::KeyW, KeyCode::KeyS);
        input.side = axis(KeyCode::KeyD, KeyCode::KeyA);
        input.jump = held(KeyCode::Space);
        input.duck = ctrl;
        input.sprint = shift;
        input.walk = alt;
        input.use_pressed = keys.just_pressed(KeyCode::KeyE);
        input.use_held = held(KeyCode::KeyE);
        input.reload_pressed = keys.just_pressed(KeyCode::KeyR);
        input.reload = held(KeyCode::KeyR);
        input.undo_pressed = keys.just_pressed(KeyCode::KeyZ);
        input.flashlight_pressed = keys.just_pressed(KeyCode::KeyF);
        const SLOTS: [KeyCode; 9] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9];
        input.slot = SLOTS.iter().position(|k| keys.just_pressed(*k)).map(|i| i as u8 + 1);
        if !menu && st.regain == 0 {
            input.attack = buttons.pressed(MouseButton::Left);
            input.attack_pressed = buttons.just_pressed(MouseButton::Left);
            input.attack2 = buttons.pressed(MouseButton::Right);
            input.attack2_pressed = buttons.just_pressed(MouseButton::Right);
            if real {
                input.mouse = motion.delta;
                input.wheel = match scroll.unit {
                    MouseScrollUnit::Line => scroll.delta.y,
                    MouseScrollUnit::Pixel => scroll.delta.y / 100.0,
                };
            } else {
                // (A test run: its buttons count, the real mouse's motion doesn't.)
            }
        }
    }
    // Nothing else sees these keys' presses (Space pauses the clock, 1-4 change the flight mode, G goes to the selection...).
    const HIS: [KeyCode; 23] = [
        KeyCode::Space,
        KeyCode::KeyW,
        KeyCode::KeyA,
        KeyCode::KeyS,
        KeyCode::KeyD,
        KeyCode::KeyQ,
        KeyCode::KeyE,
        KeyCode::KeyR,
        KeyCode::KeyF,
        KeyCode::KeyZ,
        KeyCode::KeyX,
        KeyCode::KeyC,
        KeyCode::KeyG,
        KeyCode::KeyT,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for k in HIS {
        keys.clear_just_pressed(k);
    }
    if !menu {
        for b in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            buttons.clear_just_pressed(b);
            buttons.clear_just_released(b);
        }
    }
    if real {
        // (winit's lock: through `None` at the window's middle, then `Locked`, as `mods::snapshot_input` does it.)
        let free = menu || !focused;
        let want = if free || st.regain == 3 { CursorGrabMode::None } else { CursorGrabMode::Locked };
        if !free && st.regain == 3 {
            let middle = Vec2::new(window.width(), window.height()) * 0.5;
            window.set_cursor_position(Some(middle));
        }
        if cursor.grab_mode != want {
            cursor.grab_mode = want;
        }
        if cursor.visible != free {
            cursor.visible = free;
        }
    }
    st.regain = st.regain.saturating_sub(1);
}

/// With the cursor locked a click must not press whatever egui has under it; with the Q menu open everything reaches it.
fn strip_egui_input(sb: Res<Sandbox>, mut q: Query<&mut bevy_egui::EguiInput, With<PrimaryEguiContext>>) {
    if !sb.walking() {
        return;
    }
    for mut input in &mut q {
        input.events.retain(|e| crate::mods::reaches_egui(e, sb.menu));
    }
}

/// The world's answer to the player's hull traces.
struct Tracer<'a> {
    world: &'a mut SandboxWorld,
}

impl Tracer<'_> {
    fn hull(ducked: bool) -> Vec3 {
        let h = if ducked { HULL_HEIGHT_DUCK } else { HULL_HEIGHT };
        Vec3::new(HULL_HALF, HULL_HALF, h * 0.5) * UNIT_M
    }
}

impl movement::MoveWorld for Tracer<'_> {
    fn trace(&self, start: Vec3, end: Vec3, ducked: bool) -> movement::Trace {
        let half = Self::hull(ducked);
        let centre = |p: Vec3| p * UNIT_M + Vec3::Z * half.z;
        if start == end {
            let blocked = self.world.box_blocked(centre(start), half - Vec3::splat(0.0005));
            return movement::Trace { fraction: if blocked { 0.0 } else { 1.0 }, end: start, normal: Vec3::ZERO, start_solid: blocked, hit: None };
        }
        let Some(hit) = self.world.sweep_box(centre(start), centre(end), half) else { return movement::Trace::clear(end) };
        let way = end - start;
        let len = way.length();
        // Stop a hair short of it, so the next move along it starts clear.
        let into = (-(way / len).dot(hit.normal)).max(0.05);
        let free = (len * hit.fraction - SKIN / into).max(0.0);
        let fraction = (free / len).min(1.0);
        let what = match hit.met {
            Met::Ground | Met::Solid(_) => movement::Hit::World,
            Met::Prop(id) => movement::Hit::Prop(id, self.world.velocity_at(id, hit.point) / UNIT_M),
        };
        movement::Trace { fraction, end: start + way * fraction, normal: hit.normal, start_solid: false, hit: Some(what) }
    }

    fn touch(&mut self, prop: u32, _at: Vec3, velocity: Vec3, normal: Vec3) {
        // Walking into a prop shoves it, as the player's physics shadow does in Source: up to a walking pace, and what is
        // heavier than he can push (`SetPushMassLimit` 350 kg) hardly at all.
        let Some((centre, v)) = self.world.motion(prop) else { return };
        let Some(mass) = self.world.prop(prop).map(|p| p.mass) else { return };
        let push = (-velocity.dot(normal) * UNIT_M).min(2.2);
        let along = -normal;
        let gain = push - v.dot(along);
        if gain > 0.0 {
            let share = (350.0 / mass).min(1.0);
            self.world.impulse(prop, along * (gain * mass * share * 0.6), centre);
        }
    }
}

/// The body a point is nearest the surface of, among those he could walk on: solid ground the port has heights for.
fn walkable_near<'a>(bodies: &'a Query<(&SystemBody, Option<&Terrain>)>, pos: se_core::upos::UPos) -> Option<(&'a SystemBody, &'a Terrain, f64)> {
    bodies
        .iter()
        .filter_map(|(b, t)| {
            let t = t?;
            if b.star || b.radius_m <= 0.0 || t.proc.kind.is_giant() {
                return None;
            }
            let h = b.from_shape(b.orient.inverse() * pos.delta_m(b.pos)).length() - b.radius_m;
            (h < b.radius_m * WALK_REACH_RADII).then_some((b, t, h))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

/// The ground's height in a frame at a point of it (metres over the frame's anchor's level); `None` where no ground is loaded.
fn ground_z(frame: &LocalFrame, b: &SystemBody, t: &Terrain, x: f64, y: f64) -> Option<f32> {
    let q = frame.to_body(DVec3::new(x, y, 0.0));
    let (r, level) = ground_on(b.radius_m, b.shape, Some(t), q);
    level?;
    Some(frame.from_body(q.normalize() * r).z as f32)
}

/// The point of the ground under (or over) a body-frame point; `None` where no ground is loaded.
fn ground_under(b: &SystemBody, t: &Terrain, point: DVec3) -> Option<DVec3> {
    let (r, level) = ground_on(b.radius_m, b.shape, Some(t), point);
    level?;
    Some(point.normalize() * r)
}

/// What stands round him: the probe's newest list of buildings and trees with the body it is on, the trees' trunks, and
/// the buildings the Viltrumite hero broke (`solids.rs`).
struct Near<'a> {
    list: Option<(&'a [ProbeBuilding], u32, ObjectRef)>,
    lib: &'a ScatterLibrary,
    broken: Option<&'a crate::viltrumite::breakable::BrokenSet>,
}

/// One frame of the sandbox: the modes' changes, the ground field, the ticks (his move, then the props'), and the camera.
#[allow(clippy::too_many_arguments)]
fn sandbox_step(
    time: Res<Time<Real>>,
    data: Res<SeData>,
    input: Res<WalkInput>,
    mut sb: ResMut<Sandbox>,
    mut cam: ResMut<FlyCamera>,
    mut control: ResMut<ModControl>,
    mut msg: ResMut<crate::messages::Messages>,
    (mut probe, mut ask, lib, broken): (ResMut<BuildingProbe>, ResMut<ProbeAsk>, Res<ScatterLibrary>, Option<Res<crate::viltrumite::breakable::BrokenSet>>),
    bodies: Query<(&SystemBody, Option<&Terrain>)>,
) {
    let sb = &mut *sb;
    sb.events.clear();

    // Leaving first person (for a mod, or for free flight) happens at the end of this frame: the camera is written once more
    // where his eyes are and bound to the body there, so whatever takes it carries it on from the right place. (The body moves
    // 500 m a frame round the Sun: a camera left where last frame had it is that far off.)
    let mut leave = None;
    if control.hero && sb.mode != Mode::Hero {
        sb.before_hero = sb.mode;
        if sb.mode == Mode::Walk {
            leave = Some(Mode::Hero);
        } else {
            sb.mode = Mode::Hero;
        }
    } else if !control.hero && sb.mode == Mode::Hero {
        sb.mode = Mode::Fly;
        if sb.before_hero == Mode::Walk {
            walk_from_camera(sb, &mut cam, &bodies, &mut msg);
        }
    }
    // V.
    if input.toggle_flight {
        match sb.mode {
            Mode::Walk if leave.is_none() => {
                leave = Some(Mode::Fly);
                msg.show("Free flight: on (V)".to_string());
                info!("sandbox: V: free flight on");
            }
            Mode::Walk => {}
            Mode::Fly => {
                if walk_from_camera(sb, &mut cam, &bodies, &mut msg) {
                    msg.show("Free flight: off (V)".to_string());
                    info!("sandbox: V: free flight off, on foot again");
                }
            }
            Mode::Hero => {
                // The mod's hero goes, and the free flight is what's left: V again walks.
                control.request_off();
                sb.before_hero = Mode::Fly;
                msg.show("Free flight: on (V)".to_string());
            }
        }
    }
    if sb.log_state && (sb.mode != Mode::Walk || sb.world.is_none()) {
        sb.log_state = false;
        info!(
            "sandbox: mode {:?}, pending {}, a mod on: {}, props {}; the camera {:.2} m from the nearest surface, bound {}",
            sb.mode,
            sb.pending.is_some(),
            control.hero,
            sb.world.as_ref().map_or(0, |w| w.prop_count()),
            cam.nearest_surface,
            cam.follow.is_some()
        );
    }
    let near = Near {
        list: probe.body.filter(|_| probe.arrived).and_then(|e| bodies.get(e).ok()).map(|(b, _)| (&probe.list[..], probe.list_seq, b.obj)),
        lib: &lib,
        broken: broken.as_deref(),
    };
    let lost = world_frame(sb, &mut cam, &data, &input, time.delta_secs().min(0.25), &bodies, &near);
    // The buildings and trees round him, for the frames to come (the list comes back 3-5 frames after it is asked for).
    if sb.mode == Mode::Walk && sb.placed && sb.pending.is_none() && sb.world.is_some() && leave.is_none() && !lost {
        ask.capsule = Some(ProbeCapsule { from: cam.pos, to: cam.pos, margin_m: solids::BUILDING_REACH_M, both: false, trees_m: solids::TREE_REACH_M });
    }
    if let Some(to) = leave.or(lost.then_some(Mode::Fly)).filter(|_| sb.mode == Mode::Walk) {
        sb.mode = to;
        sb.pending = None;
        if to == Mode::Fly {
            // (Nothing asks the probe now; a mod that takes over asks for itself. What is solid stays as it is.)
            probe.clear();
            // The free flight's assigned velocity is whatever it was left at (100 km/s at a fresh start): from standing on
            // the ground that is a kilometre a frame. A running pace to begin with, as SE sets on landing; the wheel changes it.
            cam.speed = cam.speed.min(20.0);
        }
        if let Some(w) = sb.world.as_mut() {
            w.park_player();
        }
    }
    if control.walker != (sb.mode == Mode::Walk) {
        control.walker = sb.mode == Mode::Walk;
    }
}

/// The frame's work on the world: where he's to be set down, the ground field, the ticks, the camera. `true` if he walks
/// and the body under him has gone (free flight takes over).
fn world_frame(sb: &mut Sandbox, cam: &mut FlyCamera, data: &SeData, input: &WalkInput, dt: f32, bodies: &Query<(&SystemBody, Option<&Terrain>)>, near: &Near) -> bool {
    // The body the world is on (or is to be on).
    let obj = sb.pending.map(|p| p.obj).or(sb.world.as_ref().map(|w| w.frame.obj));
    let found = obj.and_then(|o| bodies.iter().find(|(b, _)| b.obj == o)).and_then(|(b, t)| Some((b, t?)));
    let Some((b, terrain)) = found else {
        // Its system has gone (a trip to another star in free flight): the props go with it. (At the very start the
        // bodies aren't made yet: he waits.)
        if sb.world.is_some() && sb.mode != Mode::Walk {
            sb.world = None;
            sb.build = None;
        }
        return sb.mode == Mode::Walk && sb.pending.is_none() && sb.world.is_some();
    };
    let gm = G * body_mass(data, b.obj, b.radius_m);

    // Set down where he was asked to be, once there is ground there. Until then the camera waits at the spot, on the body.
    if let Some(p) = sb.pending {
        if sb.mode == Mode::Walk {
            cam.pos = b.pos.offset_m(b.orient * p.cam_point);
            cam.rot = (b.orient * p.cam_rot).normalize();
            bind_camera(cam, b);
        }
        if let Some(ground) = ground_under(b, terrain, p.point) {
            let eyes_up = (p.point.length() - ground.length()).max(0.0);
            let frame = LocalFrame::at(b.obj, b.shape, ground);
            match sb.world.as_mut().filter(|w| w.frame.obj == b.obj) {
                Some(w) => {
                    w.clear_terrain();
                    w.rebase(frame);
                }
                None => sb.world = Some(SandboxWorld::new(frame, TICK)),
            }
            sb.build = None;
            let feet = if p.eyes { (eyes_up as f32 / UNIT_M - movement::VIEW_Z).max(0.0) } else { 0.0 };
            sb.player = Player::at(Vec3::new(0.0, 0.0, feet + 1.0));
            sb.prev_origin = sb.player.origin;
            sb.prev_view_z = movement::VIEW_Z;
            match p.heading {
                Some(h) => {
                    // From north, clockwise, to Source's yaw from east, turning left.
                    sb.yaw = (90.0 - h).to_radians();
                    sb.pitch = 0.0;
                }
                None => {
                    let f = (frame.axes.transpose() * (p.cam_rot * DVec3::NEG_Z)).as_vec3();
                    sb.yaw = f.y.atan2(f.x);
                    sb.pitch = f.z.clamp(-1.0, 1.0).asin();
                }
            }
            sb.pending = None;
            sb.placed = false;
            let d = (ground / b.shape).normalize();
            let land = (ground / b.shape).length() - b.radius_m;
            info!(
                "sandbox: set down on {} at {:.4} {:.4}, {:.1} m over the ground, which is {land:.0} m over the body's radius{}",
                data.catalog.display_name(b.obj),
                d.y.asin().to_degrees(),
                (-d.z).atan2(d.x).to_degrees(),
                feet * UNIT_M,
                if terrain.has_ocean && land <= 0.0 { " (the sea)" } else { "" }
            );
        }
    }
    // (Out of `sb` while the frame works on both.)
    let Some(mut world) = sb.world.take() else { return false };
    step_world(sb, &mut world, b, terrain, gm, input, dt, cam, data, near);
    sb.world = Some(world);
    false
}

/// The rest of the frame, once there is a world: its ground field, the ticks and the camera.
#[allow(clippy::too_many_arguments)]
fn step_world(sb: &mut Sandbox, world: &mut SandboxWorld, b: &SystemBody, terrain: &Terrain, gm: f64, input: &WalkInput, dt: f32, cam: &mut FlyCamera, data: &SeData, near: &Near) {
    // The ground field: made when there is none, when he has walked off from the anchor, or when the terrain under it changed
    // (finer nodes arrive over the first half minute).
    let walking = sb.mode == Mode::Walk;
    let at = sb.player.origin.as_dvec3() * UNIT_M as f64;
    let reach = REBASE_M.min(b.radius_m * REBASE_RADII);
    if sb.build.is_none() && sb.pending.is_none() {
        let far = walking && sb.placed && at.truncate().length() > reach;
        let stale = world.terrain().is_some() && {
            sb.check_in = sb.check_in.saturating_sub(1);
            sb.check_in == 0 && {
                sb.check_in = 40;
                terrain_changed(world, b, terrain)
            }
        };
        if !world.has_ground() || far || stale {
            let frame = if far {
                // At the ground under him (his feet may be in the air).
                let here = world.frame.to_body(at);
                ground_under(b, terrain, here).map(|g| LocalFrame::at(b.obj, b.shape, g))
            } else {
                Some(world.frame)
            };
            if let Some(frame) = frame {
                sb.build = Some(Build { frame, heights: Vec::with_capacity(GRID_N * GRID_N), row: 0, spent: std::time::Duration::ZERO });
            }
        }
    }
    if let Some(build) = sb.build.as_mut() {
        let half = (GRID_N - 1) as f64 * GRID_CELL as f64 * 0.5;
        let mut failed = false;
        // (The first field of a place is made at once: he's waiting for it.)
        let rows = if world.has_ground() { GRID_ROWS_A_FRAME } else { GRID_N };
        let started = std::time::Instant::now();
        'rows: for _ in 0..rows {
            if build.row >= GRID_N {
                break;
            }
            let y = build.row as f64 * GRID_CELL as f64 - half;
            for ix in 0..GRID_N {
                let x = ix as f64 * GRID_CELL as f64 - half;
                match ground_z(&build.frame, b, terrain, x, y) {
                    Some(z) => build.heights.push(z),
                    None => {
                        failed = true;
                        break 'rows;
                    }
                }
            }
            build.row += 1;
        }
        build.spent += started.elapsed();
        if failed {
            sb.build = None;
        } else if build.row >= GRID_N {
            let build = sb.build.take().unwrap();
            if build.spent.as_millis() > 60 {
                // (About 26,000 heights: 15 ms as measured. Much more would be felt as he walks.)
                info!("sandbox: a ground field took {} ms to sample", build.spent.as_millis());
            }
            // He keeps his height over the ground through the change, as the props do (`replace_ground`).
            let over = world.terrain().and_then(|old| {
                let o = sb.player.origin * UNIT_M;
                Some(o.z - old.height(o.x, o.y)?)
            });
            if build.frame.anchor != world.frame.anchor {
                // The player goes with the frame.
                let from = world.frame;
                let turn = (build.frame.axes.transpose() * from.axes).as_mat3();
                let shift = |p: Vec3| (build.frame.from_body(from.to_body(p.as_dvec3() * UNIT_M as f64)) / UNIT_M as f64).as_vec3();
                sb.player.origin = shift(sb.player.origin);
                sb.prev_origin = shift(sb.prev_origin);
                sb.player.velocity = turn * sb.player.velocity;
            }
            world.replace_ground(build.frame, TerrainGrid { n: GRID_N, cell: GRID_CELL, heights: build.heights });
            world.lift_onto_ground();
            let o = sb.player.origin * UNIT_M;
            if let (Some(agl), Some(h)) = (over, world.terrain().and_then(|t| t.height(o.x, o.y))) {
                let dz = (h + agl - o.z) / UNIT_M;
                if dz.abs() > 0.1 {
                    sb.player.origin.z += dz;
                    sb.prev_origin.z += dz;
                }
            }
            if walking && !sb.placed {
                sb.placed = true;
                sb.accumulator = 0.0;
            }
        }
    }

    // The buildings and trees round him, as the probe last listed them: solid (while he walks; off foot they stay as they
    // were, so what leans on them goes on leaning).
    if walking && sb.pending.is_none() {
        let list = near.list.filter(|l| l.2 == world.frame.obj).map(|l| (l.0, l.1));
        let half = Tracer::hull(sb.player.ducked);
        let centre = sb.player.origin * UNIT_M + Vec3::Z * half.z;
        sb.solids.sync(world, list, near.lib, &|k| near.broken.is_some_and(|b| b.has(k)), sb.placed.then_some((centre, half)));
    }

    sb.ground_material = audio::ground_name(terrain.land_plants, sb.solids.buildings > 0);

    // Gravity: the planet's own, at where he is.
    let r = world.frame.to_body(at).length().max(b.radius_m * 0.5);
    sb.gravity = (gm / (r * r)) as f32;
    world.gravity = sb.gravity;
    sb.vars.gravity = sb.gravity / UNIT_M;

    // The view: the mouse turns it at once, whatever the ticks do.
    if walking {
        if let Some((yaw, pitch)) = sb.dev_look.take() {
            sb.yaw = yaw.to_radians();
            sb.pitch = pitch.to_radians();
        }
        if sb.rotating {
            sb.spin = input.mouse;
        } else {
            sb.spin = Vec2::ZERO;
            sb.yaw -= (input.mouse.x * MOUSE_DEG).to_radians();
            sb.pitch = (sb.pitch - (input.mouse.y * MOUSE_DEG).to_radians()).clamp(-89f32.to_radians(), 89f32.to_radians());
            sb.yaw = sb.yaw.rem_euclid(std::f32::consts::TAU);
        }
        if input.undo_pressed {
            sb.requests.push(Request::Undo);
        }
    }

    // The ticks.
    sb.accumulator += dt;
    let mut impacts = Vec::new();
    let mut ran = 0;
    while sb.accumulator >= TICK && ran < 8 {
        sb.accumulator -= TICK;
        ran += 1;
        sb.ticks += 1;
        if walking && sb.placed {
            let cmd = Cmd { forward: input.forward, side: input.side, yaw: sb.yaw, jump: input.jump, duck: input.duck, sprint: input.sprint, walk: input.walk };
            unstick(&mut sb.player, world);
            sb.prev_origin = sb.player.origin;
            sb.prev_view_z = sb.player.view_z() - sb.player.step_smooth;
            let was_ground = sb.player.on_ground();
            let ev = movement::player_move(&mut sb.player, &cmd, &sb.vars, &mut Tracer { world: &mut *world }, TICK);
            sb.player.velocity = sb.player.velocity.clamp(Vec3::splat(-MAX_VELOCITY), Vec3::splat(MAX_VELOCITY));
            sb.punch.decay(TICK);
            if ev.jumped {
                sb.events.push(Event::Jump);
            }
            if let Some(speed) = ev.landed {
                sb.events.push(Event::Land(speed));
                if speed > movement::FALL_PUNCH_SPEED {
                    // `PlayerRoughLandingEffects`: the view rolls.
                    sb.punch.angle.z = (speed * 0.013).min(8.0);
                }
            }
            footsteps(sb, was_ground);
            let h = if sb.player.ducked { HULL_HEIGHT_DUCK } else { HULL_HEIGHT };
            world.set_player(sb.player.origin * UNIT_M, HULL_HALF * UNIT_M, h * UNIT_M);
        }
        world.step(&mut impacts);
    }
    if ran == 8 {
        sb.accumulator = 0.0;
    }
    sb.events.extend(impacts.into_iter().map(Event::PropImpact));

    if !walking {
        return;
    }
    // The camera: his eyes, between the last two ticks.
    let k = (sb.accumulator / TICK).clamp(0.0, 1.0);
    let origin = if sb.placed { sb.prev_origin.lerp(sb.player.origin, k) } else { sb.player.origin };
    let view_z = sb.prev_view_z + (sb.player.view_z() - sb.player.step_smooth - sb.prev_view_z) * k;
    let eye = (origin + Vec3::Z * view_z) * UNIT_M;
    let fwd = sb.view_forward(-sb.punch.angle.x, sb.punch.angle.y);
    sb.eye = eye;
    sb.forward = sb.view_forward(0.0, 0.0);
    let frame = &world.frame;
    let to_world = b.orient * frame.rotation();
    cam.pos = b.pos.offset_m(b.orient * frame.to_body(eye.as_dvec3()));
    cam.rot = (crate::camera::look_rotation(to_world * fwd.as_dvec3(), to_world * DVec3::Z) * DQuat::from_rotation_z(-(sb.punch.angle.z as f64).to_radians())).normalize();
    bind_camera(cam, b);
    cam.eye_floor_m = Some(EYE_FLOOR_M);
    if sb.log_state {
        sb.log_state = false;
        log_state(sb, world, data, b);
    }
}

/// Through the ground's surface (new ground that rose under a corner of his box on a slope, a landing at a fall's full speed):
/// lifted clear. Inside it no trace would answer sensibly, and he would hang there gathering speed.
fn unstick(p: &mut Player, world: &SandboxWorld) {
    let half = Tracer::hull(p.ducked);
    let centre = p.origin * UNIT_M + Vec3::Z * half.z;
    let inner = half - Vec3::splat(0.003);
    if !world.box_in_ground(centre, inner) {
        return;
    }
    for i in 1..=80 {
        let up = i as f32 * 0.025;
        if !world.box_in_ground(centre + Vec3::Z * up, inner) {
            p.origin.z += up / UNIT_M;
            p.velocity.z = p.velocity.z.max(0.0);
            return;
        }
    }
}

/// V in free flight, or a mod handing the camera back: first person at the camera, if there's a world to stand on near it.
fn walk_from_camera(sb: &mut Sandbox, cam: &mut FlyCamera, bodies: &Query<(&SystemBody, Option<&Terrain>)>, msg: &mut crate::messages::Messages) -> bool {
    let Some((b, _, _)) = walkable_near(bodies, cam.pos) else {
        msg.show("No solid ground near enough to walk on: free flight stays on".to_string());
        return false;
    };
    let point = b.orient.inverse() * cam.pos.delta_m(b.pos);
    sb.pending = Some(Pending { obj: b.obj, point, eyes: true, heading: None, cam_point: point, cam_rot: b.orient.inverse() * cam.rot });
    enter_walk(sb);
    true
}

/// Has the terrain under the ground field changed since it was sampled? (Nine points of it against the terrain now.)
fn terrain_changed(world: &SandboxWorld, b: &SystemBody, t: &Terrain) -> bool {
    let Some(grid) = world.terrain() else { return false };
    let half = grid.half() * 0.8;
    for iy in -1..=1 {
        for ix in -1..=1 {
            let (x, y) = (ix as f32 * half, iy as f32 * half);
            let (Some(was), Some(now)) = (grid.height(x, y), ground_z(&world.frame, b, t, x as f64, y as f64)) else { continue };
            if (was - now).abs() > 0.03 {
                return true;
            }
        }
    }
    false
}

/// `UpdateStepSound` as Garry's Mod times it (`GM:PlayerStepSoundTime`): a step every so many milliseconds by the speed asked
/// for, only on the ground and moving.
fn footsteps(sb: &mut Sandbox, was_ground: bool) {
    let p = &sb.player;
    let speed = p.velocity.truncate().length();
    if !p.on_ground() || !was_ground {
        sb.step_dist = 0.0;
        return;
    }
    let (walk, _run) = if p.ducked { (60.0, 80.0) } else { (90.0, 220.0) };
    if speed < walk {
        sb.step_dist = 0.0;
        return;
    }
    // Garry's Mod: 400 ms at up to 100 units/s asked for, 350 to 300, 250 beyond, 50 more crouched.
    let ms = if speed <= 100.0 { 400.0 } else if speed <= 300.0 { 350.0 } else { 250.0 } + if p.ducked { 50.0 } else { 0.0 };
    sb.step_dist += TICK * 1000.0;
    if sb.step_dist >= ms {
        sb.step_dist = 0.0;
        sb.events.push(Event::Step { left: sb.step_left, speed, ducked: p.ducked });
        sb.step_left = !sb.step_left;
    }
}

/// The walking view's projection, after the camera's own is set from `FlyCamera`: Garry's Mod's field of view (the free
/// flight's `fov_deg` is left as the user has it: it is saved with the settings), and the near plane in for the weapon in
/// his hands, 20 to 60 cm from his eyes.
fn near_plane(sb: Res<Sandbox>, mut proj: Query<&mut Projection, With<crate::camera::MainCamera>>) {
    if !sb.walking() {
        return;
    }
    for mut p in &mut proj {
        if let Projection::Perspective(p) = p.as_mut() {
            p.fov = WALK_FOV_DEG.to_radians();
            p.near = p.near.min(0.02);
        }
    }
}

fn log_state(sb: &Sandbox, world: &SandboxWorld, data: &SeData, b: &SystemBody) {
    let p = &sb.player;
    let d = (world.frame.to_body(p.origin.as_dvec3() * UNIT_M as f64) / b.shape).normalize();
    let (lat, lon) = (d.y.asin().to_degrees(), (-d.z).atan2(d.x).to_degrees());
    let ground = world.terrain().and_then(|t| t.height(p.origin.x * UNIT_M, p.origin.y * UNIT_M));
    info!(
        "sandbox: mode {:?} placed {} on {} at {lat:.5} {lon:.5}; origin {:.1} {:.1} {:.1} u, {:.3} m over the field's ground; velocity {:.1} {:.1} {:.1} u/s (speed {:.1}); ground {:?}; ducked {}; yaw {:.1} pitch {:.1}; gravity {:.3} m/s2; props {}; solid: {} buildings, {} trees ({} pieces, {} waiting for him to step out); tick {}",
        sb.mode,
        sb.placed,
        data.catalog.display_name(b.obj),
        p.origin.x,
        p.origin.y,
        p.origin.z,
        ground.map_or(f32::NAN, |g| p.origin.z * UNIT_M - g),
        p.velocity.x,
        p.velocity.y,
        p.velocity.z,
        p.velocity.truncate().length(),
        p.ground.map(|g| match g {
            movement::Hit::World => "the ground".to_string(),
            movement::Hit::Prop(id, _) => format!("prop {id}"),
        }),
        p.ducked,
        sb.yaw.to_degrees(),
        sb.pitch.to_degrees(),
        sb.gravity,
        world.prop_count(),
        sb.solids.buildings,
        sb.solids.trees,
        world.solid_count(),
        sb.solids.waiting(),
        sb.ticks,
    );
}

/// `sbx walk [LAT LON [HEADING]]` | `fly` | `look YAW PITCH` | `state`: test runs have no keyboard or mouse. `walk` alone is V
/// from free flight (first person at the camera); with a place, he's put down there on the selected body. `look` sets the
/// view (degrees: yaw from east turning left, pitch up). `state` logs where he is and how he moves. Keys are pressed with
/// `hero keys KEYS SECS` (`w+shift`, `space`, `ctrl`, `v`, `q`, `1`..`6`, `lmb`, `rmb`).
pub fn dev_command(world: &mut World, arg: &str) {
    if !enabled() {
        warn!("sbx: the sandbox is off (SE_SANDBOX=0)");
        return;
    }
    let mut parts = arg.split_whitespace();
    let verb = parts.next().unwrap_or("").to_ascii_lowercase();
    let rest: Vec<&str> = parts.collect();
    let nums: Vec<f64> = rest.iter().filter_map(|s| s.parse().ok()).collect();
    match verb.as_str() {
        "walk" if nums.len() >= 2 => {
            world.resource_scope(|world, mut sb: Mut<Sandbox>| {
                world.resource_scope(|world, mut cam: Mut<FlyCamera>| {
                    let data = world.resource::<SeData>();
                    let jd = world.resource::<SimTime>().jd;
                    let Some(obj) = cam.selected else {
                        warn!("sbx walk: nothing is selected");
                        return;
                    };
                    let eph = Ephemeris::new(&data.catalog);
                    let shape = DVec3::from_array(body_shape(&data.catalog, obj));
                    place_on(&mut sb, &mut cam, &eph, jd, obj, shape, nums[0], nums[1], nums.get(2).copied().unwrap_or(0.0) as f32);
                });
            });
        }
        "walk" | "fly" => {
            let want = if verb == "walk" { Mode::Walk } else { Mode::Fly };
            if world.resource::<Sandbox>().mode != want {
                world.resource_mut::<WalkInput>().toggle_flight = true;
                // (Read by this frame's `sandbox_step`: `read_input` has already run.)
            }
        }
        "look" if nums.len() >= 2 => world.resource_mut::<Sandbox>().dev_look = Some((nums[0] as f32, nums[1] as f32)),
        "state" => world.resource_mut::<Sandbox>().log_state = true,
        // spawn NAME: a prop where he looks, by its model's path or the end of it (`oildrum001`).
        "spawn" => {
            let want = rest.first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
            let found = world.resource::<assets::GmodAssets>().lists.iter().flat_map(|l| l.models.iter()).find(|m| **m == want || assets::prop_name(m).eq_ignore_ascii_case(&want) || m.ends_with(&format!("/{want}.mdl"))).cloned();
            match found {
                Some(path) => world.resource_mut::<Sandbox>().requests.push(Request::Spawn(path)),
                None => warn!("sbx spawn: no prop named '{want}' in the spawn lists"),
            }
        }
        "undo" => world.resource_mut::<Sandbox>().requests.push(Request::Undo),
        "clear" => world.resource_mut::<Sandbox>().requests.push(Request::Clear),
        // menu on|off|spawnlists|weapons|mods: hold the Q menu open, or choose its tab.
        "menu" => match rest.first().copied().unwrap_or("on") {
            "on" => world.resource_mut::<Sandbox>().dev_menu = true,
            "off" => world.resource_mut::<Sandbox>().dev_menu = false,
            tab => world.resource_mut::<menu::Menu>().dev_tab = Some(tab.to_ascii_lowercase()),
        },
        // weapon N: take the Nth weapon of `weapons::WEAPONS` in hand (1: the crowbar).
        "weapon" if !nums.is_empty() => world.resource_mut::<weapons::Arsenal>().take((nums[0] as usize).saturating_sub(1)),
        // props: where every prop is and how it moves.
        "props" => {
            let sb = world.resource::<Sandbox>();
            let Some(w) = sb.world.as_ref() else {
                info!("sandbox: no world");
                return;
            };
            info!("sandbox: {} props, gravity {:.3} m/s2", w.prop_count(), w.gravity);
            let mut props: Vec<_> = w.props().collect();
            props.sort_by_key(|p| p.id);
            for p in props {
                let ground = w.terrain().and_then(|t| t.height(p.now.pos.x, p.now.pos.y));
                let v = w.motion(p.id).map_or(Vec3::ZERO, |m| m.1);
                info!(
                    "sandbox: prop {} ({:?}, {:.0} kg) at {:.2} {:.2} {:.2} m, {:.3} m over the ground, velocity {:.2} {:.2} {:.2} m/s{}{}",
                    p.id,
                    p.surface,
                    p.mass,
                    p.now.pos.x,
                    p.now.pos.y,
                    p.now.pos.z,
                    ground.map_or(f32::NAN, |g| p.now.pos.z - g),
                    v.x,
                    v.y,
                    v.z,
                    if p.frozen { ", frozen" } else { "" },
                    if p.held { ", held" } else { "" },
                );
            }
        }
        // solids: the nearest buildings and trees made solid, with the way to each (the yaw `sbx look` takes).
        "solids" => {
            let sb = world.resource::<Sandbox>();
            let at = sb.player.origin * UNIT_M;
            info!("sandbox: solid: {} buildings, {} trees; he is at {:.1} {:.1} {:.1} m", sb.solids.buildings, sb.solids.trees, at.x, at.y, at.z);
            for (tree, foot, size, waiting) in sb.solids.nearest(at, 8) {
                let d = foot - at;
                info!(
                    "sandbox: a {} {:.1} m off at yaw {:.1}, its foot {:.2} m over his feet, size {:.1} {:.1} {:.1}{}",
                    if tree { "tree" } else { "building" },
                    d.truncate().length(),
                    d.y.atan2(d.x).to_degrees(),
                    d.z,
                    size.x,
                    size.y,
                    size.z,
                    if waiting { " (waiting for him to step out)" } else { "" }
                );
            }
        }
        // arsenal: the weapon in hand and what its view model is playing.
        "arsenal" => info!("sandbox: in hand: {}", world.resource::<weapons::Arsenal>().describe()),
        _ => warn!("sbx: unknown command '{arg}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spot_is_a_place_on_earth() {
        for (name, lat, lon, heading) in SPOTS {
            assert!((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && (0.0..360.0).contains(&heading), "{name}");
            let d = sphere_dir(lat, lon);
            assert!((d.length() - 1.0).abs() < 1e-12);
            assert!((d.y.asin().to_degrees() - lat).abs() < 1e-9);
        }
    }

    #[test]
    fn the_view_looks_where_its_angles_say() {
        let mut sb = Sandbox::default();
        // Yaw 0 looks east, a quarter turn left looks north; pitch looks up.
        assert!((sb.view_forward(0.0, 0.0) - Vec3::X).length() < 1e-6);
        sb.yaw = std::f32::consts::FRAC_PI_2;
        assert!((sb.view_forward(0.0, 0.0) - Vec3::Y).length() < 1e-6);
        sb.pitch = 0.5;
        assert!(sb.view_forward(0.0, 0.0).z > 0.4);
    }
}
