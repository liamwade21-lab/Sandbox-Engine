//! The Viltrumite powers mod (docs/spec-mods-viltrumite-powers.md): a playable Omni-Man. The menu, the registry, the
//! files' checks and the keys' snapshot (`ModInput`) are `crate::mods`.
//!
//! So far (step 8): he flies from hovering to far past light speed, falls and orbits under real gravity, lands, walks, runs
//! and jumps, and the camera follows him; the air booms and heats him; he punches, punches the ground and claps, and hard landings
//! leave craters and dust. `hero.rs` is his state machine and each frame's move, `frames.rs` where he is, `ground.rs` what he stands
//! on and where his feet go, `camera.rs` the chase camera, `air.rs` the one source of air and the boom's states, `sound.rs` his sounds
//! by that air, `fx.rs` what the air does round him and the passes that draw it and the impacts, `impact.rs` the punches, the clap,
//! the crater and dust pools and the shake, `anim.rs` the sampler, `cape.rs` the cape's springs, `draw.rs` how he and his shadow are
//! drawn and lit, `readout.rs` the corner readout, `breakable.rs` the buildings (so far the probe that lists those round him:
//! step 9's first part), `tune.rs` every number to tune.

pub mod air;
pub mod anim;
pub mod breakable;
pub mod camera;
pub mod controls;
pub mod cape;
pub mod draw;
pub mod frames;
pub mod fx;
pub mod ground;
pub mod hero;
pub mod impact;
pub mod readout;
pub mod solid;
pub mod sound;
pub mod tune;

use std::io::Write as _;

use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::upos::UPos;

use crate::camera::{FlyCamera, Follow, TravelKind};
use crate::data::SeData;
use crate::mods::{HeroAssets, ModControl, ModInput};
use crate::system::SystemBody;
use crate::universe::{SimTime, UniverseSet};

use air::{Boom, Flow};
use anim::Sampler;
use draw::HeroDraws;
use frames::Frame;
use ground::Soles;
use hero::{Director, Impact, State};
use readout::Readout;
use sound::Cues;

pub struct ViltrumitePlugin;

impl Plugin for ViltrumitePlugin {
    fn build(&self, app: &mut App) {
        draw::build(app);
        fx::build(app);
        app.add_plugins(MaterialPlugin::<impact::RimMaterial>::default());
        app.init_resource::<Hero>()
            .init_resource::<DevKeys>()
            .init_resource::<breakable::ProbeDev>()
            .init_resource::<solid::Solids>()
            .init_resource::<breakable::BrokenSet>()
            .init_resource::<sound::Voices>()
            // Test runs press real keys into the frame's input, before the mod's snapshot and every other reader.
            .add_systems(PreUpdate, dev_keys.run_if(dev_keys_held).after(bevy::input::InputSystems).before(crate::mods::snapshot_input))
            // First enable: `HeroAssets` arrives from the Mods menu's first enable, and these make the rest.
            .add_systems(Update, draw::spawn_hero.run_if(first_enable))
            .add_systems(Update, draw::sync_visibility.run_if(resource_exists::<HeroDraws>))
            // Off: the camera handed back the frame it happens, before a script's first command.
            .add_systems(Update, hand_back.run_if(to_hand_back).after(crate::mods::apply_request).before(crate::scripting::run_script))
            // On closing: before the session is saved.
            .add_systems(Last, hand_back.run_if(to_hand_back).after(crate::mods::off_on_exit).before(crate::session::save_on_exit))
            .add_systems(
                PostUpdate,
                // With the camera's own motion (as a ship's), before its transform is set. Nothing here runs while the
                // mod is off.
                hero::hero_move.run_if(hero_shown).in_set(UniverseSet::Camera).after(crate::camera::travel_and_move).before(crate::camera::sync_camera_transform),
            )
            .add_systems(
                PostUpdate,
                // After everything that moves the camera or sets the near plane (`keep_camera_above_ground`), before
                // the projection turns into this frame's matrix.
                draw::update_hero
                    .run_if(hero_shown)
                    .after(UniverseSet::Camera)
                    .after(crate::terrain::update_node_transforms)
                    .before(UniverseSet::FloatingOrigin)
                    .before(bevy::camera::CameraUpdateSystems)
                    // (The capture views copy the near plane he sets, in the same frame.)
                    .before(crate::capture::follow_main),
            )
            // The buildings' probe: the capsule from where he is now, before the scatter turns it into this frame's.
            .add_systems(PostUpdate, breakable::probe_ask.run_if(hero_shown).after(hero::hero_move).before(crate::scatter::probe::ask_probe))
            // Step 10: the broken buildings into the scatter's frame, after his move and the scatter's own update.
            .add_systems(PostUpdate, breakable::update_broken.run_if(breakable::broken_running).after(hero::hero_move).after(crate::scatter::update_scatter))
            // What the air does round him, from the camera as the frame draws it.
            .add_systems(PostUpdate, fx::update_fx.run_if(hero_shown).after(draw::update_hero).before(UniverseSet::FloatingOrigin))
            // The impacts' pools (made once he is), then the records written once. They run on after the mod is turned off while
            // anything is in the pools (the aftermath, F3), with the light the rims and the bowls take worked out at the nearest crater.
            .add_systems(Update, impact::spawn_impacts.run_if(impacts_to_make))
            .add_systems(
                PostUpdate,
                (impact::update_impacts, draw::aftermath_light, fx::upload)
                    .chain()
                    .run_if(impacts_running)
                    .after(fx::update_fx)
                    .after(draw::update_hero)
                    .after(crate::terrain::update_node_transforms)
                    .before(UniverseSet::FloatingOrigin),
            )
            // F7's shake: after the camera's transform is set from it, before the view's matrices.
            .add_systems(
                PostUpdate,
                impact::apply_shake
                    .run_if(impacts_running)
                    .after(impact::update_impacts)
                    .after(crate::camera::sync_camera_transform)
                    .before(bevy::camera::CameraUpdateSystems)
                    .before(bevy::transform::TransformSystems::Propagate)
                    .before(crate::capture::follow_main),
            )
            // His sounds, after his move. While he's off this is the one check, once the last voice has faded.
            .add_systems(PostUpdate, sound::play_sounds.run_if(sound::sound_running).after(hero::hero_move))
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                readout::draw.run_if(hero_shown).after(crate::skin::UiSet::Draw).before(crate::skin::UiSet::Backdrop),
            );
    }
}

/// The mod is on, its files read, and nothing of him made yet.
fn first_enable(control: Res<ModControl>, assets: Option<Res<HeroAssets>>, draws: Option<Res<HeroDraws>>) -> bool {
    control.hero && assets.is_some() && draws.is_none()
}

/// The mod is on and he is made.
fn hero_shown(control: Res<ModControl>, assets: Option<Res<HeroAssets>>, draws: Option<Res<HeroDraws>>) -> bool {
    control.hero && assets.is_some() && draws.is_some()
}

/// He's made and the impacts' pools aren't yet.
fn impacts_to_make(draws: Option<Res<HeroDraws>>, impacts: Option<Res<impact::Impacts>>, input: Option<Res<fx::FxInput>>) -> bool {
    draws.is_some() && input.is_some() && impacts.is_none()
}

/// The impacts' systems: while he's on, or while the pools still hold something after he's been turned off (C22).
fn impacts_running(control: Res<ModControl>, impacts: Option<Res<impact::Impacts>>, draws: Option<Res<HeroDraws>>) -> bool {
    impacts.is_some() && draws.is_some() && (control.hero || control.aftermath)
}

fn to_hand_back(control: Res<ModControl>, hero: Res<Hero>) -> bool {
    !control.hero && hero.placed
}

/// Which light he takes (`hero light free` forces the CPU's, to compare them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LightPath {
    #[default]
    Auto,
    Free,
}

#[derive(Resource)]
pub struct Hero {
    pub frame: Frame,
    pub state: State,
    /// His body's turn in the universe's axes, port axes (-Z forward, +Y up).
    pub rot: DQuat,
    /// His position and rotation in the universe, as of the last time they were worked out.
    pub world: (UPos, DQuat),
    /// Real seconds, whatever the game clock does (T8).
    pub clock: f64,
    pub sampler: Box<Sampler>,
    pub director: Director,
    pub light_path: LightPath,
    /// The camera's distance behind him (m), the wheel's.
    pub dist: f64,
    /// How much further back the speed has it, eased.
    pub zoom: f64,
    /// Seconds on the Shift ramp.
    pub ramp_t: f64,
    pub was_boost: bool,
    pub readout: Readout,
    /// Where he last put the camera: anything else moved it if it's now far from there.
    pub cam_written: UPos,
    /// What carries him, as a binding of the camera as he last wrote it: a Go To starting goes with it for its first
    /// frame (`travel_and_move`), as the camera's own binding would have.
    pub carrier: Option<Follow>,
    /// A Go To or a landing carried him last frame.
    pub carried: bool,
    /// Placed since the last frame by something else: lift him if that's under the ground (F2).
    spawned: bool,
    /// Placed since the mod was turned on.
    pub placed: bool,
    /// `hero trace`: a test's log of every frame.
    trace: Option<Trace>,
    /// `hero orbit K`: a test puts him in orbit next frame, at K x the circular speed.
    dev_orbit: Option<f64>,
    /// Real seconds in this landing.
    pub state_t: f64,
    /// Seconds Shift has been held on his feet: a sprint after a while.
    pub shift_t: f64,
    /// Falling from a jump or off a ledge, not from flight.
    pub short_fall: bool,
    /// The last landing's event, for the effects (step 8).
    pub impact: Option<Impact>,
    /// A finer node moved the ground: his and the camera's drawn place are this far from his real place, along his up (m).
    pub ease: f64,
    /// The slope under him (degrees), and whether it's too steep to stand on.
    pub slope_deg: f64,
    pub sliding: bool,
    /// His sole points, once.
    pub soles: Option<Soles>,
    /// His drawn root over the capsule's middle by the clip (m), eased, and how far it's dropped to the lower foot's ground.
    pub clip_lift: f64,
    pub root_drop: f64,
    /// Last frame's velocity over the ground, and what the cloth feels, smoothed.
    pub last_vel: DVec3,
    pub felt: DVec3,
    /// `hero feet`: a test's log of his soles against the ground.
    feet: Option<Trace>,
    /// `hero soil off`: the ground keeps its stones round him, to compare.
    pub soil_off: bool,
    /// `hero shadow off`: his shadow isn't drawn, to measure it.
    pub shadow_off: bool,
    /// He came down on the sea with keys held: they count again once let go.
    pub keys_up_first: bool,
    /// Ctrl took him to a stop with nothing to stand on (a giant's floor, a star): he stays there until it's let go. (One key,
    /// not `keys_up_first`: dived there with Ctrl and W, letting go of Ctrl alone flies on along W.)
    pub ctrl_up_first: bool,
    /// While a Go To or a landing carries him (he has no velocity of his own then): the camera last frame, in that body's frame.
    pub carried_from: Option<(ObjectRef, DVec3)>,
    /// `hero hold on`: his body keeps facing where it does, so a test's camera can go round him.
    pub hold_body: bool,
    /// What he's flying through this frame, from `air.rs`: the sounds, the readout and the boom go by it.
    pub flow: Flow,
    /// F9's temperature, low-passed (K).
    pub heat_k: f64,
    /// The way he's going through the air (unit), whether his own flight or a Go To: last frame's when he isn't going anywhere.
    pub air_dir: DVec3,
    /// F9's barrier.
    pub boom: Boom,
    /// The effects' state between frames.
    pub fx: fx::FxState,
    /// `hero cruise`: a test holds this speed and height, the way he was going, whatever the view does.
    pub cruise: Option<hero::Cruise>,
    /// `hero boom`: a test's log of the air and the barrier.
    boom_log: Option<Trace>,
    /// The sounds asked for this frame: one function per trigger, `play_sounds` plays them.
    pub sound: Cues,
    /// What the buttons are doing (step 8), its layer, the clips it plays (found once), and which punch is next.
    pub action: impact::Action,
    pub act_layer: impact::ActLayer,
    pub act_clips: Option<impact::ActClips>,
    pub punches: u8,
    /// This frame's impacts, for the pools (`impact::update_impacts` drains them).
    pub blasts: impact::Blasts,
    /// F7: how far out the camera may be with the buildings across the line of sight (m): pulled in at once, let out eased.
    pub cam_pull: f64,
    /// What the buildings did to him (`hero solid stats`, `log`).
    pub solid: solid::SolidWatch,
    /// The building the last punch hit (step 10 breaks it), taken by whoever acts on it.
    pub punched: Option<solid::Key>,
    /// This frame's broken buildings (step 10), taken by `breakable::update_broken`.
    pub breaks: Vec<breakable::Broken>,
    /// The controls as in the user's Viltrumite mod (`controls.rs`): fast flight, the dive, Space's tap or charge, the boost stacks,
    /// the double taps' last presses (his clock), the super run, what's left of the take-off's rise (s).
    pub fast: bool,
    pub dive: bool,
    pub space: controls::SpaceHold,
    pub stacks: u32,
    pub tap_w: f64,
    pub tap_s: f64,
    pub super_run: bool,
    pub rise_t: f64,
    /// The camera's shake and kick (0..~1, dying out), and how much wider the view is (degrees, eased): `impact::apply_shake`.
    pub shake: f64,
    pub kick: f64,
    pub fov_add: f64,
    /// Past the barrier in air: the steady buffet's strength (0..~1.4, eased; `controls::buffet`), which the speed lines follow too.
    pub buffet: f64,
    /// The left button: when it went down (his clock), the combo's step and its last punch, the full charge's cue given.
    pub punch_down: Option<f64>,
    pub combo: u32,
    pub last_punch: f64,
    pub punch_ready: bool,
}

impl Default for Hero {
    fn default() -> Self {
        Hero {
            frame: Frame::at(UPos::ZERO),
            state: State::Hover,
            rot: DQuat::IDENTITY,
            world: (UPos::ZERO, DQuat::IDENTITY),
            clock: 0.0,
            sampler: Box::default(),
            director: Director::default(),
            light_path: LightPath::Auto,
            dist: tune::CAM_DIST_M,
            zoom: 0.0,
            ramp_t: 0.0,
            was_boost: false,
            readout: Readout::default(),
            cam_written: UPos::ZERO,
            carrier: None,
            carried: false,
            spawned: false,
            placed: false,
            trace: None,
            dev_orbit: None,
            state_t: 0.0,
            shift_t: 0.0,
            short_fall: false,
            impact: None,
            ease: 0.0,
            slope_deg: 0.0,
            sliding: false,
            soles: None,
            clip_lift: 0.0,
            root_drop: 0.0,
            last_vel: DVec3::ZERO,
            felt: DVec3::ZERO,
            feet: None,
            soil_off: false,
            shadow_off: false,
            keys_up_first: false,
            ctrl_up_first: false,
            carried_from: None,
            hold_body: false,
            flow: Flow::default(),
            heat_k: 0.0,
            air_dir: DVec3::NEG_Z,
            boom: Boom::default(),
            fx: fx::FxState::default(),
            cruise: None,
            boom_log: None,
            sound: Cues::default(),
            action: impact::Action::None,
            act_layer: impact::ActLayer::default(),
            act_clips: None,
            punches: 0,
            blasts: impact::Blasts::default(),
            cam_pull: f64::MAX,
            solid: solid::SolidWatch::default(),
            punched: None,
            breaks: Vec::new(),
            fast: false,
            dive: false,
            space: controls::SpaceHold::default(),
            stacks: 0,
            tap_w: f64::NEG_INFINITY,
            tap_s: f64::NEG_INFINITY,
            super_run: false,
            rise_t: 0.0,
            shake: 0.0,
            kick: 0.0,
            fov_add: 0.0,
            buffet: 0.0,
            punch_down: None,
            combo: 0,
            last_punch: f64::NEG_INFINITY,
            punch_ready: false,
        }
    }
}

/// A rotation with -Z towards `look` and +Y as near `up` as can be.
impl Hero {
    /// A kick to the camera (the user's mod's `ChaseCamera.Kick`): it shakes by 0.8 of it and the view widens for a moment.
    pub fn kick_camera(&mut self, amount: f64) {
        self.kick = self.kick.max(amount);
        self.shake = self.shake.max(0.8 * amount);
    }
}

fn facing(look: DVec3, up: DVec3) -> DQuat {
    let z = -look.normalize_or(DVec3::NEG_Z);
    let x = up.cross(z).try_normalize().unwrap_or_else(|| z.any_orthonormal_vector());
    DQuat::from_mat3(&DMat3::from_cols(x, z.cross(x), z))
}

/// F3: the camera stays where it is, still; within 2 radii of a body it's bound to it, or the planet leaves
/// (`travel_and_move`, camera.rs). The cursor comes back. The craters and dust stay and drain (the aftermath: `impact.rs`).
fn hand_back(
    mut hero: ResMut<Hero>,
    mut cam: ResMut<FlyCamera>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    bodies: Query<&SystemBody>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut soil: ResMut<crate::volumetric_clouds::HeroSoil>,
    (impacts, mut rims): (Option<Res<impact::Impacts>>, Query<&mut Visibility, With<impact::RimPart>>),
    (mut probe, mut probe_dev): (ResMut<crate::scatter::probe::BuildingProbe>, ResMut<breakable::ProbeDev>),
) {
    hero.placed = false;
    breakable::probe_off(&mut probe, &mut probe_dev);
    *soil = default();
    if let Some(im) = impacts {
        im.hide_warm_rim(&mut rims);
    }
    hero.state = State::Hover;
    hero.ease = 0.0;
    hero.impact = None;
    hero.director.pinned = false;
    hero.trace = None;
    hero.feet = None;
    hero.flow = Flow::default();
    hero.heat_k = 0.0;
    hero.boom = Boom::default();
    hero.fx = fx::FxState::default();
    hero.cruise = None;
    hero.boom_log = None;
    impact::reset(&mut hero);
    cam.velocity = DVec3::ZERO;
    cam.travel = None;
    cam.follow = None;
    let eph = Ephemeris::new(&data.catalog);
    let near = bodies.iter().filter(|b| b.radius_m > 0.0).map(|b| (b, b.pos.delta_m(cam.pos).length() / b.radius_m - 1.0)).min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((b, _)) = near.filter(|n| n.1 < tune::OFF_BIND_RADII) {
        cam.follow = Some(Follow::new(b.obj, matches!(b.obj, ObjectRef::Body(_)), &eph, sim.jd));
    } else if let Some(obj) = hero.frame.body() {
        // Further out, what carried him: moving with it, no longer turning.
        cam.follow = Some(Follow::new(obj, false, &eph, sim.jd));
    }
    for mut c in &mut cursor {
        if c.grab_mode != CursorGrabMode::None {
            c.grab_mode = CursorGrabMode::None;
        }
        if !c.visible {
            c.visible = true;
        }
    }
}

// ---- Dev commands -------------------------------------------------------------------------------------------------

/// `hero keys`: real keys held for a while. They go into `ButtonInput`, so every reader sees what a keyboard would give.
#[derive(Resource, Default)]
pub struct DevKeys {
    keys: Vec<KeyCode>,
    buttons: Vec<MouseButton>,
    /// Real seconds left.
    left: f64,
    pressed: bool,
    /// Keys to let go next frame (the ones a new `hero keys` replaced).
    release: Vec<KeyCode>,
    release_buttons: Vec<MouseButton>,
}

fn dev_keys_held(dev: Res<DevKeys>) -> bool {
    !dev.keys.is_empty() || !dev.buttons.is_empty() || !dev.release.is_empty() || !dev.release_buttons.is_empty()
}

fn dev_keys(mut dev: ResMut<DevKeys>, mut keys: ResMut<ButtonInput<KeyCode>>, mut buttons: ResMut<ButtonInput<MouseButton>>, time: Res<Time<Real>>) {
    let dev = &mut *dev;
    for k in dev.release.drain(..) {
        keys.release(k);
    }
    for b in dev.release_buttons.drain(..) {
        buttons.release(b);
    }
    if dev.keys.is_empty() && dev.buttons.is_empty() {
        return;
    }
    if !dev.pressed {
        dev.pressed = true;
        for k in &dev.keys {
            keys.press(*k);
        }
        for b in &dev.buttons {
            buttons.press(*b);
        }
        return;
    }
    dev.left -= time.delta_secs_f64();
    if dev.left <= 0.0 {
        dev.pressed = false;
        dev.release.append(&mut dev.keys);
        dev.release_buttons.append(&mut dev.buttons);
    }
}

/// A key's name in `hero keys`.
fn key_named(name: &str) -> Option<KeyCode> {
    Some(match name {
        "w" => KeyCode::KeyW,
        "a" => KeyCode::KeyA,
        "s" => KeyCode::KeyS,
        "d" => KeyCode::KeyD,
        "x" => KeyCode::KeyX,
        "z" => KeyCode::KeyZ,
        "g" => KeyCode::KeyG,
        "r" => KeyCode::KeyR,
        "f" => KeyCode::KeyF,
        "q" => KeyCode::KeyQ,
        "e" => KeyCode::KeyE,
        "c" => KeyCode::KeyC,
        "t" => KeyCode::KeyT,
        "up" => KeyCode::ArrowUp,
        "down" => KeyCode::ArrowDown,
        "left" => KeyCode::ArrowLeft,
        "right" => KeyCode::ArrowRight,
        "4" => KeyCode::Digit4,
        "tab" => KeyCode::Tab,
        "f3" => KeyCode::F3,
        "alt" => KeyCode::AltLeft,
        "space" => KeyCode::Space,
        "shift" => KeyCode::ShiftLeft,
        "ctrl" => KeyCode::ControlLeft,
        _ => return None,
    })
}

/// `hero keys KEYS SECS` | `aim Y P` | `state` | `probe ...` (`breakable::dev_command`) | `solid ...` (`solid::dev_command`) | `trace FILE [BODY] | off` | `feet FILE | off` | `boom FILE | off` | `cruise V [M] | off` | `air` | `dist M` | `soil on|off` | `hold on|off` | `shadow on|off` | `orbit K` | `clip NAME|auto` | `light auto|free` | `crater [R]` | `dust [SIZE]` | `splash [SIZE]` | `impacts` | `sound ...` (`sound::dev_command`):
/// test runs have no keyboard or mouse. `keys`: KEYS joined with + (`w+shift`; also `lmb`, `rmb`), held SECS real seconds (0:
/// for a frame); `off` lets go. `aim`: turn Y degrees about his up, and pitch P degrees over his level (his horizon near a body,
/// the view's own in space). `trace`: one line a frame to FILE, his place in BODY's frame (else the nearest's). `orbit`: falling
/// from here at K x the circular speed round the nearest body, the way he looks. `cruise`: flying at V m/s over the body's ground,
/// level, the way he looks at first (M metres over the ground, else where he is), whatever the view does after that, so a test's view
/// can go round him. `boom`: one line a frame to FILE: the air, the heat, the barrier's state and each event. `air`: what the air of the
/// nearest body is, and F9's table for it.
pub fn dev_command(world: &mut World, arg: &str) {
    let (verb, rest) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
    match verb.to_ascii_lowercase().as_str() {
        "keys" => {
            let (names, secs) = rest.rsplit_once(' ').map_or((rest, 0.0), |(a, b)| (a, b.trim().parse().unwrap_or(0.0)));
            let mut dev = world.resource_mut::<DevKeys>();
            let dev = &mut *dev;
            dev.release.append(&mut dev.keys);
            dev.release_buttons.append(&mut dev.buttons);
            dev.pressed = false;
            dev.left = secs;
            for name in names.to_ascii_lowercase().split('+').filter(|n| !n.is_empty() && *n != "off") {
                match (name, key_named(name)) {
                    ("lmb", _) => dev.buttons.push(MouseButton::Left),
                    ("rmb", _) => dev.buttons.push(MouseButton::Right),
                    (_, Some(k)) => dev.keys.push(k),
                    _ => warn!("hero keys: no key '{name}'"),
                }
            }
        }
        "aim" => {
            let v: Vec<f64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            let [yaw, pitch] = v[..] else {
                warn!("hero aim: wants Y P, got '{rest}'");
                return;
            };
            let _ = world.run_system_cached_with(aim, (yaw.to_radians(), pitch.to_radians()));
        }
        "state" => {
            let _ = world.run_system_cached(log_state);
        }
        "sound" => sound::dev_command(world, rest),
        "probe" => breakable::dev_command(world, rest),
        "solid" => solid::dev_command(world, rest),
        "break" => breakable::dev_break(world, rest),
        "orbit" => world.resource_mut::<Hero>().dev_orbit = Some(rest.parse().unwrap_or(1.0)),
        "cruise" => {
            let v: Vec<f64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            let mut hero = world.resource_mut::<Hero>();
            // (Already cruising: the same way, and the same height unless given.)
            let was = hero.cruise;
            hero.cruise = v.first().map(|&speed| hero::Cruise { speed, height_m: v.get(1).copied().or(was.and_then(|c| c.height_m)), going: was.and_then(|c| c.going) });
        }
        "air" => {
            let _ = world.run_system_cached(log_air);
        }
        "boom" => {
            let mut hero = world.resource_mut::<Hero>();
            hero.boom_log = if rest.eq_ignore_ascii_case("off") {
                None
            } else {
                match std::fs::File::create(rest) {
                    Ok(f) => {
                        let mut w = std::io::BufWriter::new(f);
                        let _ = writeln!(w, "clock,state,height,speed,mach,pressure,density,q,skin_k,heat_k,boom,event,carried,glow,shell,trail,cone");
                        Some(Trace { out: w, body: None })
                    }
                    Err(e) => {
                        warn!("hero boom: {rest}: {e}");
                        None
                    }
                }
            };
        }
        "dist" => world.resource_mut::<Hero>().dist = rest.parse().unwrap_or(tune::CAM_DIST_M),
        "soil" => world.resource_mut::<Hero>().soil_off = rest.eq_ignore_ascii_case("off"),
        "hold" => world.resource_mut::<Hero>().hold_body = rest.eq_ignore_ascii_case("on"),
        "shadow" => world.resource_mut::<Hero>().shadow_off = rest.eq_ignore_ascii_case("off"),
        "trace" => {
            let (file, body) = rest.split_once(' ').map_or((rest, ""), |(a, b)| (a, b.trim()));
            let body = if body.is_empty() { None } else { world.resource::<SeData>().catalog.find(body) };
            let mut hero = world.resource_mut::<Hero>();
            hero.trace = if file.eq_ignore_ascii_case("off") {
                None
            } else {
                match std::fs::File::create(file) {
                    Ok(f) => {
                        let mut w = std::io::BufWriter::new(f);
                        let _ = writeln!(w, "clock,jd,rate,attached,state,pose,x,y,z,height,speed,vx,vy,vz,seen_x,seen_y,seen_z,roll");
                        Some(Trace { out: w, body })
                    }
                    Err(e) => {
                        warn!("hero trace: {file}: {e}");
                        None
                    }
                }
            };
        }
        "feet" => {
            let mut hero = world.resource_mut::<Hero>();
            hero.feet = if rest.eq_ignore_ascii_case("off") {
                None
            } else {
                match std::fs::File::create(rest) {
                    Ok(f) => {
                        let mut w = std::io::BufWriter::new(f);
                        let _ = writeln!(w, "clock,state,pose,time,speed,level,slope,gap_l,gap_r,lower,root_drop,clip_lift,ease,height");
                        Some(Trace { out: w, body: None })
                    }
                    Err(e) => {
                        warn!("hero feet: {rest}: {e}");
                        None
                    }
                }
            };
        }
        "clip" => {
            let name = rest.to_string();
            // (No clips until the mod's first enable, or if its files were refused.)
            let done = world.try_resource_scope(|world, assets: Mut<HeroAssets>| {
                let mut hero = world.resource_mut::<Hero>();
                if name.eq_ignore_ascii_case("auto") {
                    hero.director = Director::default();
                    return;
                }
                match assets.clips.find(&name) {
                    Some(c) => {
                        hero.director.pinned = true;
                        hero.sampler.play(c as u16, 0.0);
                    }
                    None => warn!("viltrumite: no clip '{name}'"),
                }
            });
            if done.is_none() {
                warn!("hero clip: the mod's files aren't loaded (turn it on first)");
            }
        }
        "light" => world.resource_mut::<Hero>().light_path = if rest.eq_ignore_ascii_case("free") { LightPath::Free } else { LightPath::Auto },
        "crater" | "dust" | "splash" => {
            // A test's impact under him: a crater of R m with its dust, or dust (or the sea's spray) of SIZE m, as an impact would leave.
            let size: f64 = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(if verb == "crater" { 10.0 } else { 3.0 });
            let mut hero = world.resource_mut::<Hero>();
            let Some(obj) = hero.frame.body() else {
                warn!("hero {verb}: he's on no body");
                return;
            };
            let Frame::Attached { local_pos, .. } = hero.frame else {
                warn!("hero {verb}: he's too far from the ground");
                return;
            };
            let kind = match verb {
                "crater" => impact::BlastKind::Crater(size),
                "splash" => impact::BlastKind::Dust(size, impact::Style::Spray),
                _ => impact::BlastKind::Dust(size, impact::Style::Plume),
            };
            hero.blasts.push(impact::Blast { obj, local: local_pos, kind, shake_m: size });
        }
        "impacts" => {
            if let Some(mut i) = world.get_resource_mut::<impact::Impacts>() {
                i.log = true;
            } else {
                info!("hero impacts: none made (the mod hasn't been turned on)");
            }
        }
        _ => warn!("hero: unknown command '{arg}'"),
    }
}

/// A test's per-frame log.
pub struct Trace {
    out: std::io::BufWriter<std::fs::File>,
    body: Option<ObjectRef>,
}

impl Trace {
    fn write(&mut self, line: &str) {
        let _ = self.out.write_all(line.as_bytes());
    }
}

fn aim(In((yaw, pitch)): In<(f64, f64)>, mut cam: ResMut<FlyCamera>, bodies: Query<&SystemBody>) {
    let up = bodies
        .iter()
        .filter(|b| b.radius_m > 0.0)
        .map(|b| (b, cam.pos.delta_m(b.pos)))
        .min_by(|a, b| (a.1.length() - a.0.radius_m).total_cmp(&(b.1.length() - b.0.radius_m)))
        .filter(|(b, rel)| rel.length() < b.radius_m * 1.5)
        .map_or(cam.rot * DVec3::Y, |(_, rel)| rel.normalize_or(DVec3::Y));
    let f = cam.rot * DVec3::NEG_Z;
    let flat = (f - up * f.dot(up)).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
    let heading = DQuat::from_axis_angle(up, yaw) * flat;
    cam.rot = crate::camera::look_rotation(heading * pitch.cos() + up * pitch.sin(), up);
}

/// `hero air`: the nearest body's air as `air.rs` has it, with F9's stages worked out for it at the ground and 60 km, and where 150 Pa is.
fn log_air(hero: Res<Hero>, data: Res<SeData>, sim: Res<SimTime>, mut cache: ResMut<crate::ship::air::AirCache>, bodies: Query<(&SystemBody, Option<&crate::terrain::Terrain>)>) {
    let eph = Ephemeris::new(&data.catalog);
    let Some((b, t)) = bodies.iter().filter(|(b, _)| b.radius_m > 0.0).min_by(|a, b| (a.0.pos.delta_m(hero.world.0).length() - a.0.radius_m).total_cmp(&(b.0.pos.delta_m(hero.world.0).length() - b.0.radius_m))) else { return };
    let name = data.catalog.display_name(b.obj).to_string();
    let top = t.and_then(|t| t.atmosphere.as_ref()).map(|a| a.top_radius_m(b.radius_m) - b.radius_m);
    let own = cache.get(&data, &eph, sim.jd, b.obj, b.radius_m, top);
    let Some(air) = air::AirModel::of(&mut cache, &data, &eph, sim.jd, b.obj, b.radius_m, t) else {
        info!("hero air: {name}: no air, shell top {top:?}");
        return;
    };
    let stages = [800.0, 2000.0, 3500.0, 6000.0, 10000.0];
    let speed_for = |h: f64, want: f64| (1..=600).map(|i| 100.0 * i as f64).find(|&v| air.flow(h, v).skin_k >= want);
    let table = |h: f64| stages.map(|t| speed_for(h, t).map_or(-1.0, |v| v / 1e3));
    let boom_top = (0..2_000_000).map(|i| i as f64 * 100.0).find(|&h| air.at(h).pressure_pa < tune::BOOM_AIR_PA.0).unwrap_or(0.0) / 1e3;
    info!(
        "hero air: {name}: {} params {:?}, shell top {top:?}, air top {:.1} km, ground: {:?}, 150 Pa at {boom_top:.1} km; km/s to reach {stages:?} K: ground {:?}, 60 km {:?}",
        if own.is_some() { "catalog's" } else { "fallback's" },
        air.params,
        air.top_m() / 1e3,
        air.at(0.0),
        table(0.0),
        table(60e3),
    );
}

fn log_state(
    hero: Res<Hero>,
    control: Res<ModControl>,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    input: Res<ModInput>,
    pipes: Res<crate::pipeline_watch::GroundPipelines>,
    draws: Option<Res<HeroDraws>>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    bodies: Query<(&SystemBody, Option<&crate::terrain::Terrain>)>,
) {
    // What the frames work with, for the nearest body.
    let near = bodies
        .iter()
        .filter(|(b, _)| b.radius_m > 0.0)
        .min_by(|a, b| (a.0.pos.delta_m(hero.world.0).length() - a.0.radius_m).total_cmp(&(b.0.pos.delta_m(hero.world.0).length() - b.0.radius_m)))
        .map_or("no body near".to_string(), |(b, t)| {
            let info = crate::ship::BodyInfo { obj: b.obj, pos: b.pos, orient: b.orient, radius: b.radius_m, shape: b.shape, gm: 0.0, terrain: t };
            let local = frames::to_local(&info, hero.world.0);
            format!(
                "nearest {}: {:.3} m over its radius, in the sphere frame; attaches under {:.0} m, {:.0} m/s of ground under him; looking {:.1} degrees over its horizon",
                data.catalog.display_name(b.obj),
                frames::height_over(&info, local),
                frames::attach_radius(&info) - b.radius_m,
                crate::ship::spin_at(&Ephemeris::new(&data.catalog), b.obj, sim.jd).cross(b.orient * local).length(),
                (cam.rot * DVec3::NEG_Z).dot(camera::vertical(&info, local)).clamp(-1.0, 1.0).asin().to_degrees(),
            )
        });
    let body = hero.frame.body().map_or("none".to_string(), |o| data.catalog.display_name(o).to_string());
    let frame = match hero.frame {
        Frame::Attached { local_pos, local_vel, level, .. } => {
            format!("attached to {body}, level {level:?}, {:.3} m from its centre (at {:.3?}), {:.3} m/s", local_pos.length(), local_pos.to_array(), local_vel.length())
        }
        Frame::Free { vel, .. } => format!("free, carried by {body}, {:.3} m/s in the frame", vel.length()),
    };
    let on_foot = format!("slope {:.1} sliding {} ease {:.3} root drop {:.3} last impact {:?}", hero.slope_deg, hero.sliding, hero.ease, hero.root_drop, hero.impact);
    let r = &hero.readout;
    let d = &hero.director;
    let travel = cam.travel.as_ref().map(|t| matches!(t.kind, TravelKind::Goto | TravelKind::Land));
    let grab = cursor.single().map_or("no window".to_string(), |c| format!("{:?} visible {}", c.grab_mode, c.visible));
    let selected = cam.selected.map_or("nothing".to_string(), |o| data.catalog.display_name(o).to_string());
    info!(
        "hero: on {} loading {} pipelines {:?}, state {:?}, tier {:?}, pose {:?}, {frame}; {on_foot}; {near}; speed {} ({:.3} m/s), mach {:?}, height {:?}, gravity {:.3} g; {:.2} m from the camera; ramp {:.2} s; clock {:.2} s, sim rate {} paused {}; menus {} unfocused {}; cursor {grab}; input {:?} {:?} {:?} shift {} ctrl {}; aim forward {:?}; follow {} track {} travel {travel:?}; selected {selected}; street lamps {:?}; barrier {:?}, air {:.2} Pa, skin {:.0} K, heat {:.0} K",
        control.hero,
        control.loading,
        pipes.hero(),
        hero.state,
        d.tier,
        d.pose,
        readout::format_speed(r.speed),
        r.speed,
        r.mach,
        r.height,
        r.gravity_g,
        hero.world.0.delta_m(cam.pos).length(),
        hero.ramp_t,
        hero.clock,
        sim.rate,
        sim.paused,
        control.menus,
        control.unfocused,
        input.forward,
        input.right,
        input.up,
        input.shift,
        input.ctrl,
        cam.rot * DVec3::NEG_Z,
        cam.follow.is_some(),
        cam.track,
        draws.map_or([0.0; 3], |d| d.town()),
        hero.boom.state,
        hero.flow.air.pressure_pa,
        hero.flow.skin_k,
        hero.heat_k,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_have_the_names_a_test_uses() {
        for n in ["w", "a", "s", "d", "x", "z", "space", "shift", "ctrl", "tab"] {
            assert!(key_named(n).is_some(), "{n}");
        }
        assert_eq!(key_named("nope"), None);
    }

    #[test]
    fn facing_is_upright_and_looks_where_asked() {
        let q = facing(DVec3::X, DVec3::Y);
        assert!((q * DVec3::NEG_Z - DVec3::X).length() < 1e-9);
        assert!((q * DVec3::Y - DVec3::Y).length() < 1e-9);
    }
}
