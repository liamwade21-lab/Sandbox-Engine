//! The Skate 3 physics mod (the user's 2026-11-12): play as Skate 3's default skater on his board, on any ground the sandbox
//! stands on. Since 2026-11-13 (the user: "the physics need to be byte for byte ported") it runs the Skate 3 Rust Engine's own
//! simulation (SK8-ENGINE/skate-3-rust-engine, GPL-3.0), copied unchanged into `crates/skate-sim`: its controller code, Skate
//! 3's gesture recognisers and action and motion graphs, the board and the skeleton's physics, the tricks, grinds, bails, the
//! walking off the board, the stock camera and the scoring, a tick at a time at its own 60 Hz. `crates/skate-sim/UPSTREAM.md`
//! says what was copied; its `oracle_trace` test proves it ticks bit for bit as upstream does.
//!
//! - `assets/mods/skate3` holds the skater and his board as the sandbox draws them (`skater.s3m`, `tex/`), the retail files the
//!   simulation reads (`private/`: Skate 3's tuning collections, animation banks, state graphs, gesture patterns, camera
//!   graph, trick names) and Skate 3's sounds (`sfx/`), all made by `convert.rs` from the user's own copy of Skate 3.
//! - The simulation has its own ground (`Skater::origin`: east, north, up where he got on, Y up in its own axes), never moved by
//!   the sandbox's 24 m rebase. Its collision is the sandbox's ground field and solid pieces as triangles
//!   (`SandboxWorld::static_triangles`), remade off this thread whenever those change (`statics_rev`).
//! - `input.rs` the controls (a gamepad passed through; the keyboard and mouse made into a controller), `draw.rs` drawing him
//!   (skinned, lit as the props are) from the simulation's pose, `sound.rs` Skate 3's sounds from its events.

pub mod draw;
pub mod input;
pub mod sound;

#[cfg(test)]
mod convert;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::input::gamepad::{AxisSettings, ButtonAxisSettings, Gamepad, GamepadButton, GamepadSettings};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{DVec3, Mat3, Vec2, Vec3};
use bevy::prelude::*;
use bevy_egui::egui;
use se_core::skate3::{self, Model};
use skate_sim::{Hud, PoseFrame, PreparedWorld, RawPad, Sim};

use crate::mods::ModControl;
use crate::sandbox::world::{LocalFrame, SandboxWorld};
use crate::sandbox::Sandbox;
use input::{buttons, Controls, Device, Flick};

/// Skate 3's difficulty (the engine's `physics_mode`): the Rust port's default, what the user plays.
const DIFFICULTY: &str = "easy";
/// At most this many of its ticks a frame (a long frame drops the rest, as the sandbox's own ticks do).
const MOST_TICKS: u32 = 6;
/// The least time between two remakes of its collision (trees come and go round him every few frames as he rolls).
const REBUILD_GAP: std::time::Duration = std::time::Duration::from_millis(300);

/// The mod's files, read and checked at its first enable (kept until quit).
#[derive(Resource, Clone)]
pub struct SkateAssets {
    pub model: Arc<Model>,
    /// Each draw's texture as RGBA8 (width, height, pixels), by its file.
    pub textures: Arc<Vec<(String, u32, u32, Vec<u8>)>>,
    pub sounds: Arc<sound::SoundBank>,
    /// The folder holding `private/`, which the simulation reads its retail files from.
    pub root: PathBuf,
}

/// Reads the mod's files through `read` (the mods menu's checked reader: a refusal names the file and why); `root` is the
/// mod's folder that holds `private/`.
pub fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>, needs: &[String], root: PathBuf) -> Result<SkateAssets, (String, String)> {
    let model = skate3::parse_model(&read("skater.s3m").map_err(|e| ("skater.s3m".to_string(), e))?).map_err(|e| ("skater.s3m".to_string(), e.to_string()))?;
    let mut textures = Vec::new();
    for d in &model.draws {
        if d.texture.is_empty() || textures.iter().any(|(n, ..): &(String, u32, u32, Vec<u8>)| *n == d.texture) {
            continue;
        }
        let bytes = read(&d.texture).map_err(|e| (d.texture.clone(), e))?;
        let img = image::load_from_memory(&bytes).map_err(|e| (d.texture.clone(), e.to_string()))?.to_rgba8();
        textures.push((d.texture.clone(), img.width(), img.height(), img.into_raw()));
    }
    let sounds = sound::SoundBank::load(read, needs)?;
    Ok(SkateAssets { model: Arc::new(model), textures: Arc::new(textures), sounds: Arc::new(sounds), root })
}

/// Something heard that a tick did (`sound.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkateEvent {
    Pop,
    /// A flip trick's board leaves the feet: the whoosh.
    Flip,
    /// Landed, falling this fast (m/s).
    Land(f32),
    Bail,
    Push,
}

/// Words on screen for a moment (a landed trick, a bail).
#[derive(Clone, Debug)]
pub struct Caption {
    pub text: String,
    pub sub: String,
    pub left: f32,
    pub bail: bool,
}

/// The skater while the mod is on (held by the sandbox: `Sandbox::skate`).
pub struct Skater {
    sim: Sim,
    /// The simulation's ground: east, north, up where he got on. Its native axes are x east, y up, z south (`native`).
    origin: LocalFrame,
    /// How far the sandbox's ground has moved up under him since (finer terrain arriving moves it by up to metres): the
    /// simulation's world is made that much lower and he is drawn that much higher, so its ground stays where he stands on it
    /// and he keeps his speed. `lift` goes with the collision now in, `next_lift` with the next one made.
    lift: f64,
    next_lift: f64,
    pub ctl: Controls,
    /// Seconds of the simulation's clock not yet ticked.
    accum: f32,
    /// The sandbox's statics count the simulation's collision was made from, and the one being made off this thread.
    built: u32,
    building: Option<(u32, f64, std::thread::JoinHandle<Result<PreparedWorld, String>>, std::time::Instant)>,
    /// How many times its collision was remade (for `skate state`), and when the last remake started.
    pub worlds: u32,
    last_build: std::time::Instant,
    /// What the ticks did since the frame's sounds took them.
    pub events: Vec<SkateEvent>,
    /// The wheels rolling, and how fast (the rolling sound).
    pub rolling: Option<f32>,
    /// The foot brake, a powerslide (their loops).
    pub braking: bool,
    pub sliding: bool,
    pub log: bool,
    /// The score as the engine's scoring has it now, and the moments to show.
    pub hud: Hud,
    pub caption: Option<Caption>,
    seen_trick: u32,
    seen_landing: u32,
    seen_bail: u32,
    fall_speed: f32,
    was_state: String,
    /// The simulation failed (a non-finite pose, as upstream stops on): the mod goes off.
    pub failed: Option<String>,
}

/// The simulation's native axes from its ground's east, north, up: x east, y up, z south.
fn native(l: DVec3) -> [f32; 3] {
    [l.x as f32, l.z as f32, -l.y as f32]
}

fn from_native(n: [f32; 3]) -> DVec3 {
    DVec3::new(n[0] as f64, -n[2] as f64, n[1] as f64)
}

/// The sandbox's ground field and solid pieces in the simulation's native axes, `lift` metres lower.
fn triangles(origin: &LocalFrame, lift: f64, world: &SandboxWorld) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let (v, t) = world.static_triangles();
    let v = v.into_iter().map(|p| native(origin.from_body(world.frame.to_body(p.as_dvec3())) - DVec3::Z * lift)).collect();
    (v, t)
}

/// The turn from native axes to east, north, up.
fn native_axes() -> Mat3 {
    Mat3::from_cols(Vec3::X, Vec3::Z, -Vec3::Y)
}

impl Skater {
    /// On his board at `feet` (the sandbox frame, metres) facing `yaw` (radians from east, toward north).
    pub fn new(assets: &SkateAssets, world: &SandboxWorld, feet: Vec3, yaw: f32) -> Result<Skater, String> {
        let started = std::time::Instant::now();
        let mut sim = Sim::load(&assets.root, DIFFICULTY)?;
        let origin = LocalFrame { obj: world.frame.obj, anchor: world.frame.to_body(feet.as_dvec3()), axes: world.frame.axes };
        let (v, t) = triangles(&origin, 0.0, world);
        sim.set_world(&v, &t)?;
        // (0 faces the simulation's +z, south; turning toward +x, east.)
        let heading = yaw.cos().atan2(-yaw.sin());
        sim.spawn([0.0, 0.0, 0.0], heading)?;
        info!("skate: on the board ({} triangles round him, loaded in {} ms)", t.len(), started.elapsed().as_millis());
        Ok(Skater {
            sim,
            origin,
            lift: 0.0,
            next_lift: 0.0,
            ctl: Controls::default(),
            accum: 0.0,
            built: world.statics_rev,
            building: None,
            worlds: 0,
            last_build: std::time::Instant::now(),
            events: Vec::new(),
            rolling: None,
            braking: false,
            sliding: false,
            log: false,
            hud: Hud::default(),
            caption: None,
            seen_trick: 0,
            seen_landing: 0,
            seen_bail: 0,
            fall_speed: 0.0,
            was_state: String::new(),
            failed: None,
        })
    }

    /// A native point into the sandbox's frame (metres).
    pub fn to_frame(&self, world: &SandboxWorld, n: [f32; 3]) -> Vec3 {
        world.frame.from_body(self.origin.to_body(from_native(n) + DVec3::Z * self.lift)).as_vec3()
    }

    /// The sandbox's ground under him moved up by `dz` metres (new terrain): the next collision made follows it.
    pub fn ground_moved(&mut self, dz: f64) {
        if dz.is_finite() {
            self.next_lift += dz;
            if self.log && dz.abs() > 0.01 {
                info!("skate: the ground under him moved {dz:+.2} m");
            }
        }
    }

    /// A native direction into the sandbox's frame.
    pub fn dir_to_frame(&self, world: &SandboxWorld, n: [f32; 3]) -> Vec3 {
        let turn = (world.frame.axes.transpose() * self.origin.axes).as_mat3();
        turn * from_native(n).as_vec3()
    }

    /// The turn from native axes to the sandbox frame's.
    pub fn native_to_frame(&self, world: &SandboxWorld) -> Mat3 {
        (world.frame.axes.transpose() * self.origin.axes).as_mat3() * native_axes()
    }

    /// The frame's share of the simulation: its collision brought up to the sandbox's, then its ticks with the controls.
    pub fn frame(&mut self, world: &SandboxWorld, dt: f32) {
        if self.failed.is_some() {
            return;
        }
        // A new collision, made off this thread, put in once made.
        if let Some((rev, lift, job, started)) = self.building.take() {
            if job.is_finished() {
                match job.join() {
                    Ok(Ok(prepared)) => match self.sim.install_world(prepared) {
                        Ok(()) => {
                            self.built = rev;
                            self.lift = lift;
                            self.worlds += 1;
                            if self.log {
                                info!("skate: new ground in {} ms", started.elapsed().as_millis());
                            }
                        }
                        Err(e) => warn!("skate: the new ground was refused: {e}"),
                    },
                    Ok(Err(e)) => warn!("skate: the new ground couldn't be made: {e}"),
                    Err(_) => warn!("skate: making the new ground failed"),
                }
            } else {
                self.building = Some((rev, lift, job, started));
            }
        }
        // (A ground move under him can't wait: he's drawn off the ground until it's in.)
        let urgent = (self.next_lift - self.lift).abs() > 0.01;
        if self.building.is_none() && world.statics_rev != self.built && (urgent || self.last_build.elapsed() >= REBUILD_GAP) {
            self.last_build = std::time::Instant::now();
            let lift = self.next_lift;
            let (v, t) = triangles(&self.origin, lift, world);
            let material = self.sim.floor_material();
            let rev = world.statics_rev;
            self.building = Some((rev, lift, std::thread::spawn(move || skate_sim::prepare_world(&v, &t, material)), std::time::Instant::now()));
        }
        // The controller as this frame has it (upstream polls it once a drawn frame), then its own ticks at its own period
        // (60 Hz; slower while its camera asks for slow motion). A key's flick plays a sample a tick.
        self.sim.sample(&self.ctl.pad);
        self.accum += dt;
        let mut ran = 0;
        loop {
            let period = self.sim.period().as_secs_f32();
            if self.accum < period || ran >= MOST_TICKS {
                if ran >= MOST_TICKS {
                    self.accum = 0.0;
                }
                break;
            }
            self.accum -= period;
            ran += 1;
            if let Some(pad) = self.ctl.tick_pad() {
                self.sim.sample(&pad);
            }
            if let Err(e) = self.sim.tick() {
                warn!("skate: the simulation stopped: {e}");
                self.failed = Some(e);
                return;
            }
            self.take_events();
        }
    }

    /// One tick's events into the sounds and the words on screen.
    fn take_events(&mut self) {
        let e = self.sim.events().clone();
        if e.popped {
            self.events.push(SkateEvent::Pop);
        }
        if e.airborne {
            self.fall_speed = (-e.board_velocity[1]).max(0.0);
        }
        if e.landed {
            self.events.push(SkateEvent::Land(self.fall_speed));
        }
        if e.wiped_out {
            self.events.push(SkateEvent::Bail);
        }
        if e.pushed {
            self.events.push(SkateEvent::Push);
        }
        let hud = self.sim.hud();
        if hud.trick_seq != self.seen_trick {
            self.seen_trick = hud.trick_seq;
            if e.airborne && !hud.trick_id.is_empty() && !hud.trick_id.to_ascii_uppercase().contains("OLLIE") {
                self.events.push(SkateEvent::Flip);
            }
        }
        if hud.landing_seq != self.seen_landing {
            self.seen_landing = hud.landing_seq;
            if !hud.landed_trick_text.is_empty() {
                let sub = if hud.landed_sketchy { "Sketchy" } else if hud.landed_clean { "Clean" } else { "" };
                self.caption = Some(Caption { text: hud.landed_trick_text.clone(), sub: sub.to_string(), left: 2.0, bail: false });
            }
        }
        if hud.bail_seq != self.seen_bail {
            self.seen_bail = hud.bail_seq;
            self.caption = Some(Caption { text: "Bail".to_string(), sub: String::new(), left: 1.6, bail: true });
        }
        if self.log && e.state != self.was_state {
            info!("skate: {} -> {} at {:.2} m/s ({})", self.was_state, e.state, e.ground_speed, hud.trick_text);
        }
        self.was_state = e.state.clone();
        self.hud = hud;
        let on_wheels = !e.airborne && e.wheels_touching > 0;
        self.rolling = (on_wheels && e.ground_speed > 0.15).then_some(e.ground_speed);
        self.sliding = on_wheels && e.state == "SlideGround";
        self.braking = on_wheels && e.brake_contact == 1 && e.ground_speed > 0.3;
    }

    /// The deck's middle and velocity in the sandbox's frame (metres).
    pub fn board(&self, world: &SandboxWorld) -> (Vec3, Vec3) {
        let (p, v) = self.sim.board();
        (self.to_frame(world, p), self.dir_to_frame(world, v))
    }

    /// Where he is (his trajectory, the root of his pose: under him on the board or off it) in the sandbox's frame, and the
    /// board's velocity: what the sandbox's walker follows (its ground field, the solids round him, the props' box).
    pub fn rider(&self, world: &SandboxWorld) -> (Vec3, Vec3) {
        let (deck, vel) = self.board(world);
        let at = self.sim.pose().map(|p| self.to_frame(world, [p.current.root[3][0], p.current.root[3][1], p.current.root[3][2]]));
        (at.unwrap_or(deck), vel)
    }

    /// His pose between the last two ticks (`None` before the first).
    pub fn pose(&self) -> Option<PoseFrame> {
        let k = (self.accum / self.sim.period().as_secs_f32()).clamp(0.0, 1.0);
        self.sim.pose().map(|p| p.blend(k, self.sim.skeleton()))
    }

    /// The stock camera between the last two ticks, in the sandbox's frame: its eye, forward and up.
    pub fn camera(&self, world: &SandboxWorld) -> Option<(Vec3, Vec3, Vec3)> {
        let c = self.pose()?.camera;
        Some((self.to_frame(world, c.position), self.dir_to_frame(world, c.forward), self.dir_to_frame(world, c.up)))
    }

    pub fn set_aspect_ratio(&mut self, aspect: f32) {
        self.sim.set_aspect_ratio(aspect);
    }

    pub fn state_name(&self) -> String {
        self.sim.state_name()
    }
}

/// The frame's controls into the skater's controller: the gamepad if one is being used, else the keyboard and mouse. (Before
/// the sandbox's `read_input`, which takes the key presses from everything else after.)
#[allow(clippy::too_many_arguments)]
pub fn read_pad(
    mut sb: ResMut<Sandbox>,
    control: Res<ModControl>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    pads: Query<(Entity, &Gamepad)>,
    time: Res<Time<Real>>,
    cfg: Res<crate::config::EngineConfig>,
    mut dev: ResMut<DevPad>,
) {
    let menus = control.menus_open() || control.main_menu;
    let real = crate::mods::real_window(&cfg);
    let dt = time.delta_secs().min(0.1);
    let Some(sk) = sb.skate.as_mut() else { return };
    let ctl = &mut sk.ctl;
    let mut pad = RawPad::default();
    if !menus {
        let held = |k: KeyCode| keys.pressed(k);
        let axis = |plus: &[KeyCode], minus: &[KeyCode]| (keys.any_pressed(plus.iter().copied()) as i8 - keys.any_pressed(minus.iter().copied()) as i8) as f32;
        // The left stick: A/D (and the arrows) steer, the arrows up/down lean (manuals).
        pad.left_stick = [axis(&[KeyCode::KeyD, KeyCode::ArrowRight], &[KeyCode::KeyA, KeyCode::ArrowLeft]), axis(&[KeyCode::ArrowUp], &[KeyCode::ArrowDown])];
        let mut b = 0u16;
        let mut set = |on: bool, bit: u16| {
            if on {
                b |= bit;
            }
        };
        set(held(KeyCode::KeyW), buttons::A);
        set(held(KeyCode::KeyS) || keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]), buttons::B);
        set(held(KeyCode::KeyR), buttons::X);
        set(held(KeyCode::KeyF), buttons::Y);
        set(held(KeyCode::KeyZ), buttons::LB);
        set(held(KeyCode::KeyC), buttons::RB);
        set(held(KeyCode::Tab), buttons::BACK);
        pad.buttons = b;
        pad.triggers = [mouse_buttons.pressed(MouseButton::Left) as u8 as f32, mouse_buttons.pressed(MouseButton::Right) as u8 as f32];
        // The mouse is the right stick; Space held is the stick held down, let go a flick up (an ollie).
        let counts = if real { motion.delta } else { Vec2::ZERO };
        let r = ctl.mouse.step(counts, dt);
        pad.right_stick = [r.x, r.y];
        let space = held(KeyCode::Space);
        if space {
            pad.right_stick = [0.0, -1.0];
        } else if ctl.space_held && ctl.queue.is_empty() {
            ctl.queue.extend([[0.0, 1.0]; 5]);
            ctl.queue.push_back([0.0, 0.0]);
        }
        ctl.space_held = space;
        const FLICK_KEYS: [(KeyCode, Flick); 8] = [
            (KeyCode::Digit1, Flick::Kickflip),
            (KeyCode::Digit2, Flick::Heelflip),
            (KeyCode::Digit3, Flick::FsShuvit),
            (KeyCode::Digit4, Flick::Shuvit),
            (KeyCode::Digit5, Flick::VarialKickflip),
            (KeyCode::Digit6, Flick::VarialHeelflip),
            (KeyCode::Digit7, Flick::Hardflip),
            (KeyCode::Digit8, Flick::InwardHeelflip),
        ];
        if let Some((_, f)) = FLICK_KEYS.iter().find(|(k, _)| keys.just_pressed(*k)) {
            ctl.play(*f);
        }
        // The device last used counts: a gamepad from the moment one is touched, the keyboard and mouse from a key, a mouse
        // button or the mouse moving. A gamepad's sticks then pass through as they are (no gate: Skate 3's recognisers see
        // every small move, and its own dead zones apply; Bevy's are off, `raw_pads`).
        if keys.get_just_pressed().next().is_some() || mouse_buttons.get_just_pressed().next().is_some() || counts.length() > 2.0 {
            ctl.device = Device::Keyboard;
        }
        for (e, g) in &pads {
            let touched = g.left_stick().length() > 0.3 || g.right_stick().length() > 0.3 || g.get_just_pressed().next().is_some();
            if touched {
                ctl.device = Device::Pad(e);
            }
        }
        if let Device::Pad(e) = ctl.device {
            if let Ok((_, g)) = pads.get(e) {
                let (l, r) = (g.left_stick(), g.right_stick());
                let mut b = 0u16;
                for (button, bit) in [
                    (GamepadButton::South, buttons::A),
                    (GamepadButton::East, buttons::B),
                    (GamepadButton::West, buttons::X),
                    (GamepadButton::North, buttons::Y),
                    (GamepadButton::LeftTrigger, buttons::LB),
                    (GamepadButton::RightTrigger, buttons::RB),
                    (GamepadButton::Start, buttons::START),
                    (GamepadButton::Select, buttons::BACK),
                    (GamepadButton::LeftThumb, buttons::LEFT_STICK),
                    (GamepadButton::RightThumb, buttons::RIGHT_STICK),
                    (GamepadButton::DPadUp, buttons::DPAD_UP),
                    (GamepadButton::DPadDown, buttons::DPAD_DOWN),
                    (GamepadButton::DPadLeft, buttons::DPAD_LEFT),
                    (GamepadButton::DPadRight, buttons::DPAD_RIGHT),
                ] {
                    if g.pressed(button) {
                        b |= bit;
                    }
                }
                pad = RawPad {
                    buttons: b,
                    triggers: [g.get(GamepadButton::LeftTrigger2).unwrap_or(0.0), g.get(GamepadButton::RightTrigger2).unwrap_or(0.0)],
                    left_stick: [l.x, l.y],
                    right_stick: [r.x, r.y],
                };
                // (The keyboard's flicks and Space don't play over a gamepad.)
                ctl.queue.clear();
            } else {
                ctl.device = Device::Keyboard;
            }
        }
    }
    // A test's controller (`skate pad ...`) stands in for both while it lasts.
    if dev.left > 0.0 {
        dev.left -= dt;
        pad = dev.pad;
    }
    if let Some(f) = dev.flick.take() {
        ctl.play(f);
    }
    ctl.pad = pad;
}

/// Gamepads report their sticks and triggers as they are: no dead zone, live zone or change threshold of Bevy's (the
/// defaults would hide small moves and step the values), so the simulation's own controller code sees what XInput gives it.
/// (Only the Skate 3 mod reads gamepads.)
pub fn raw_pads(mut pads: Query<&mut GamepadSettings, Added<GamepadSettings>>) {
    for mut s in &mut pads {
        if let Ok(axis) = AxisSettings::new(-1.0, 0.0, 0.0, 1.0, 0.0) {
            s.default_axis_settings = axis;
        }
        s.default_button_axis_settings = ButtonAxisSettings { high: 1.0, low: 0.0, threshold: 0.0 };
    }
}

/// A test's controls: held for a number of seconds (`skate pad`), or a flick played (`skate flick`).
#[derive(Resource, Default)]
pub struct DevPad {
    pad: RawPad,
    left: f32,
    flick: Option<Flick>,
}

/// The mod turned on or off: the skater stands on his board where the walker was (once the sandbox stands on ground), or
/// the walker stands where he was.
pub fn sync(mut sb: ResMut<Sandbox>, mut control: ResMut<ModControl>, assets: Option<Res<SkateAssets>>, windows: Query<&Window>) {
    let sb = &mut *sb;
    let want = control.skate && assets.is_some();
    // Put somewhere else (a place, `sbx walk`): his board comes back once the walker stands there.
    if sb.skate.is_some() && (!sb.placed || !sb.walking()) && want {
        sb.skate = None;
        return;
    }
    if want && sb.skate.is_none() {
        if let (Some(assets), true, true) = (assets.as_ref(), sb.walking(), sb.placed) {
            let Some(world) = sb.world.as_ref() else { return };
            let feet = sb.player.origin * crate::sandbox::movement::UNIT_M;
            match Skater::new(assets, world, feet, sb.yaw) {
                Ok(mut sk) => {
                    if let Some(w) = windows.iter().next() {
                        sk.set_aspect_ratio(w.width() / w.height().max(1.0));
                    }
                    sb.skate = Some(sk);
                }
                Err(e) => {
                    warn!("skate: the simulation couldn't start: {e}");
                    control.request_off();
                }
            }
        }
    } else if !want && sb.skate.is_some() {
        if let Some(sk) = sb.skate.take() {
            // On foot where he was, facing the way his board faced.
            if let Some(world) = sb.world.as_ref() {
                let (pos, vel) = sk.rider(world);
                sb.player.origin = pos / crate::sandbox::movement::UNIT_M;
                sb.player.velocity = Vec3::ZERO;
                if vel.truncate().length() > 0.3 {
                    sb.yaw = vel.y.atan2(vel.x);
                }
            }
            info!("skate: off the board");
        }
    }
    if let Some(sk) = sb.skate.as_ref() {
        if sk.failed.is_some() {
            control.request_off();
        }
    }
}

/// Skate 3's score as its HUD gives it (drawn here with egui: the game's own HUD is a Flash movie): the trick being done
/// with its points and multiplier near the bottom, the line's score with its timer at the top right, and a landed trick's
/// or a bail's words for a moment.
pub fn draw_hud(mut contexts: bevy_egui::EguiContexts, mut sb: ResMut<Sandbox>, time: Res<Time<Real>>, control: Res<ModControl>) -> Result {
    let Some(sk) = sb.skate.as_mut() else { return Ok(()) };
    if control.main_menu {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("skate hud")));
    let white = |a: f32| egui::Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * a) as u8);
    let shadowed = |pos: egui::Pos2, align: egui::Align2, text: &str, size: f32, color: egui::Color32| {
        let font = egui::FontId::proportional(size);
        painter.text(pos + egui::vec2(2.0, 2.0), align, text, font.clone(), egui::Color32::from_black_alpha(color.a().saturating_sub(60)));
        painter.text(pos, align, text, font, color);
    };
    let h = &sk.hud;
    // The line: its banked score, the multiplier and the time left to keep it going.
    if h.line_score > 0.0 || h.sequence_active {
        let top_right = egui::pos2(rect.right() - 24.0, rect.top() + 20.0);
        shadowed(top_right, egui::Align2::RIGHT_TOP, &format!("{}", h.line_score.round() as i64), 34.0, white(1.0));
        if h.line_capacity > 0.0 && h.line_time > 0.0 {
            let w = 180.0;
            let bar = egui::Rect::from_min_size(egui::pos2(top_right.x - w, top_right.y + 44.0), egui::vec2(w, 5.0));
            painter.rect_filled(bar, 2.0, egui::Color32::from_black_alpha(120));
            let share = (h.line_time / h.line_capacity).clamp(0.0, 1.0);
            painter.rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(w * share, 5.0)), 2.0, egui::Color32::from_rgb(250, 200, 40));
        }
    }
    // The trick being done.
    if h.sequence_active && !h.trick_text.is_empty() {
        let base = egui::pos2(rect.center().x, rect.bottom() - rect.height() * 0.16);
        shadowed(base, egui::Align2::CENTER_BOTTOM, &h.trick_text, 30.0, white(1.0));
        let points = format!("{}  x{:.0}", h.sequence_score.round() as i64, h.multiplier.max(1.0));
        shadowed(base + egui::vec2(0.0, 6.0), egui::Align2::CENTER_TOP, &points, 24.0, egui::Color32::from_rgb(250, 200, 40));
    }
    // A landed trick or a bail.
    if let Some(c) = sk.caption.as_mut() {
        c.left -= time.delta_secs();
        if c.left <= 0.0 {
            sk.caption = None;
        } else {
            let a = (c.left / 0.4).clamp(0.0, 1.0);
            let pos = egui::pos2(rect.center().x, rect.top() + rect.height() * 0.22);
            let color = if c.bail { egui::Color32::from_rgba_unmultiplied(235, 70, 60, (255.0 * a) as u8) } else { white(a) };
            shadowed(pos, egui::Align2::CENTER_CENTER, &c.text, 40.0, color);
            if !c.sub.is_empty() {
                shadowed(pos + egui::vec2(0.0, 34.0), egui::Align2::CENTER_CENTER, &c.sub, 22.0, white(a * 0.85));
            }
        }
    }
    Ok(())
}

pub struct SkatePlugin;

impl Plugin for SkatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DevPad>();
        if !crate::sandbox::enabled() {
            return;
        }
        draw::build(app);
        app.add_systems(PreUpdate, read_pad.after(bevy::input::InputSystems).before(crate::sandbox::read_input))
            .add_systems(PostUpdate, sync.before(crate::sandbox::sandbox_step))
            .add_systems(Update, raw_pads)
            .add_systems(bevy_egui::EguiPrimaryContextPass, draw_hud.in_set(crate::skin::UiSet::Draw))
            .add_systems(PostUpdate, sound::play.after(crate::sandbox::sandbox_step));
    }
}

/// `skate state|log on|off|pad KEYS SECS|flick NAME`: test commands (`--run "skate ..."`).
pub fn dev_command(world: &mut World, arg: &str) {
    let mut parts = arg.split_whitespace();
    let verb = parts.next().unwrap_or("").to_ascii_lowercase();
    let rest: Vec<&str> = parts.collect();
    match verb.as_str() {
        "state" => {
            let sb = world.resource::<Sandbox>();
            match (sb.skate.as_ref(), sb.world.as_ref()) {
                (Some(sk), Some(w)) => {
                    let (p, v) = sk.board(w);
                    let h = &sk.hud;
                    info!(
                        "skate: {}, {:.2} m/s, at {:.2} {:.2} {:.2}; trick {:?} ({}), sequence {:.0} x{:.1}, line {:.0}, landed {:?}; ground remade {} times{}",
                        sk.state_name(),
                        v.length(),
                        p.x,
                        p.y,
                        p.z,
                        h.trick_text,
                        h.trick_id,
                        h.sequence_score,
                        h.multiplier,
                        h.line_score,
                        h.landed_trick_text,
                        sk.worlds,
                        sk.failed.as_ref().map(|e| format!("; FAILED {e}")).unwrap_or_default()
                    );
                }
                _ => info!("skate: not on the board (mod on: {})", world.resource::<ModControl>().skate),
            }
        }
        "log" => {
            let on = rest.first().copied() != Some("off");
            if let Some(sk) = world.resource_mut::<Sandbox>().skate.as_mut() {
                sk.log = on;
            }
        }
        "pad" => {
            // `skate pad a+left 2`: a, b, x, y, lb, rb, lt, rt (buttons and triggers), left, right, up, down (the left stick),
            // rdown, rup, rleft, rright (the right stick), held for the seconds given.
            let keys = rest.first().copied().unwrap_or("");
            let secs: f32 = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.5);
            let mut p = RawPad::default();
            for k in keys.split('+') {
                match k {
                    "a" => p.buttons |= buttons::A,
                    "b" => p.buttons |= buttons::B,
                    "x" => p.buttons |= buttons::X,
                    "y" => p.buttons |= buttons::Y,
                    "lb" => p.buttons |= buttons::LB,
                    "rb" => p.buttons |= buttons::RB,
                    "lt" => p.triggers[0] = 1.0,
                    "rt" => p.triggers[1] = 1.0,
                    "left" => p.left_stick[0] = -1.0,
                    "right" => p.left_stick[0] = 1.0,
                    "up" => p.left_stick[1] = 1.0,
                    "down" => p.left_stick[1] = -1.0,
                    "rdown" => p.right_stick[1] = -1.0,
                    "rup" => p.right_stick[1] = 1.0,
                    "rleft" => p.right_stick[0] = -1.0,
                    "rright" => p.right_stick[0] = 1.0,
                    "off" | "" => {}
                    other => warn!("skate pad: no key {other}"),
                }
            }
            let mut dev = world.resource_mut::<DevPad>();
            dev.pad = p;
            dev.left = secs;
        }
        "flick" => match Flick::by_name(&rest.join(" ")) {
            Some(f) => world.resource_mut::<DevPad>().flick = Some(f),
            None => warn!("skate flick: which? ({})", Flick::ALL.map(|f| f.name()).join(", ")),
        },
        _ => warn!("skate: state | log on|off | pad KEYS SECS | flick NAME"),
    }
}

/// The skater's state for the sandbox's own log line.
pub fn describe(sk: &Skater) -> String {
    format!("skating: {} ({:.0} points in the line)", sk.state_name(), sk.hud.line_score)
}
