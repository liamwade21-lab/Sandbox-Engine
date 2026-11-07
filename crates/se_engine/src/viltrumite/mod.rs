//! The Viltrumite powers mod (docs/spec-mods-viltrumite-powers.md): a playable Omni-Man. The menu, the registry, the
//! files' checks and the keys' snapshot (`ModInput`) are `crate::mods`.
//!
//! So far (step 6): he flies from hovering to far past light speed, falls and orbits under real gravity, lands, walks, runs
//! and jumps, and the camera follows him. `hero.rs` is his state machine and each frame's move, `frames.rs` where he is,
//! `ground.rs` what he stands on and where his feet go, `camera.rs` the chase camera, `air.rs` the one source of air,
//! `sound.rs` his sounds by that air, `anim.rs` the sampler, `cape.rs` the cape's springs, `draw.rs` how he and his shadow
//! are drawn and lit, `readout.rs` the corner readout, `tune.rs` every number to tune.

pub mod air;
pub mod anim;
pub mod camera;
pub mod cape;
pub mod draw;
pub mod frames;
pub mod ground;
pub mod hero;
pub mod readout;
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

use air::Flow;
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
        app.init_resource::<Hero>()
            .init_resource::<DevKeys>()
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
    /// F9's temperature, low-passed (K). The heat's look is step 7's.
    pub heat_k: f64,
    /// The sounds asked for this frame: one function per trigger, `play_sounds` plays them.
    pub sound: Cues,
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
            sound: Cues::default(),
        }
    }
}

/// A rotation with -Z towards `look` and +Y as near `up` as can be.
fn facing(look: DVec3, up: DVec3) -> DQuat {
    let z = -look.normalize_or(DVec3::NEG_Z);
    let x = up.cross(z).try_normalize().unwrap_or_else(|| z.any_orthonormal_vector());
    DQuat::from_mat3(&DMat3::from_cols(x, z.cross(x), z))
}

/// F3: the camera stays where it is, still; within 2 radii of a body it's bound to it, or the planet leaves
/// (`travel_and_move`, camera.rs). The cursor comes back. The aftermath (craters, collapses draining) is steps 8 and 10's.
fn hand_back(
    mut hero: ResMut<Hero>,
    mut cam: ResMut<FlyCamera>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    bodies: Query<&SystemBody>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut soil: ResMut<crate::volumetric_clouds::HeroSoil>,
) {
    hero.placed = false;
    *soil = default();
    hero.state = State::Hover;
    hero.ease = 0.0;
    hero.impact = None;
    hero.director.pinned = false;
    hero.trace = None;
    hero.feet = None;
    hero.flow = Flow::default();
    hero.heat_k = 0.0;
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

/// `hero keys KEYS SECS` | `aim Y P` | `state` | `trace FILE [BODY] | off` | `feet FILE | off` | `dist M` | `soil on|off` | `hold on|off` | `shadow on|off` | `orbit K` | `clip NAME|auto` | `light auto|free` | `sound ...` (`sound::dev_command`):
/// test runs have no keyboard or mouse. `keys`: KEYS joined with + (`w+shift`; also `lmb`, `rmb`), held SECS real seconds (0:
/// for a frame); `off` lets go. `aim`: turn Y degrees about his up, and pitch P degrees over his level (his horizon near a body,
/// the view's own in space). `trace`: one line a frame to FILE, his place in BODY's frame (else the nearest's). `orbit`: falling
/// from here at K x the circular speed round the nearest body, the way he looks.
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
        "orbit" => world.resource_mut::<Hero>().dev_orbit = Some(rest.parse().unwrap_or(1.0)),
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
        Frame::Attached { local_pos, local_vel, level, .. } => format!("attached to {body}, level {level:?}, {:.3} m from its centre, {:.3} m/s", local_pos.length(), local_vel.length()),
        Frame::Free { vel, .. } => format!("free, carried by {body}, {:.3} m/s in the frame", vel.length()),
    };
    let on_foot = format!("slope {:.1} sliding {} ease {:.3} root drop {:.3} last impact {:?}", hero.slope_deg, hero.sliding, hero.ease, hero.root_drop, hero.impact);
    let r = &hero.readout;
    let d = &hero.director;
    let travel = cam.travel.as_ref().map(|t| matches!(t.kind, TravelKind::Goto | TravelKind::Land));
    let grab = cursor.single().map_or("no window".to_string(), |c| format!("{:?} visible {}", c.grab_mode, c.visible));
    let selected = cam.selected.map_or("nothing".to_string(), |o| data.catalog.display_name(o).to_string());
    info!(
        "hero: on {} loading {} pipelines {:?}, state {:?}, tier {:?}, pose {:?}, {frame}; {on_foot}; {near}; speed {} ({:.3} m/s), mach {:?}, height {:?}, gravity {:.3} g; {:.2} m from the camera; ramp {:.2} s; clock {:.2} s, sim rate {} paused {}; menus {} unfocused {}; cursor {grab}; input {:?} {:?} {:?} shift {} ctrl {}; aim forward {:?}; follow {} track {} travel {travel:?}; selected {selected}; street lamps {:?}",
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
