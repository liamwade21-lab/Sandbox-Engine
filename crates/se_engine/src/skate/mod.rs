//! The Skate 3 physics mod (the user's 2026-11-12): play as Skate 3's default skater on his board, on any ground the sandbox
//! stands on. Built on the Skate 3 Rust Engine (SK8-ENGINE/skate-3-rust-engine, GPL-3.0): its engine-free calculations
//! (`crates/skate-core`: the push, the brake, the steering's tilt, the jump) and, for making the mod's files, its readers
//! (`crates/skate-data`, `convert.rs`).
//!
//! - `assets/mods/skate3` holds the skater and his board (`skater.s3m`, `tex/`), Skate 3's clips for him (`clips.s3c`), its
//!   tuning values (`tuning.cfg`) and its sounds (`sfx/`): made by `convert.rs` from the user's own copy of Skate 3.
//! - The mod plays inside the sandbox's first person (`sandbox/mod.rs`): the same ground field, buildings, trees, props and
//!   gravity, the skater's tick (`Skater::tick`) in place of the walker's move, and a chase camera behind him. The sandbox
//!   holds his state (`Sandbox::skate`) while the mod is on.
//! - `input.rs` the controls (a gamepad or the keyboard and mouse), `physics.rs` the board, `anim.rs` what he does and the
//!   clips that show it, `draw.rs` drawing him (skinned, lit as the props are), `sound.rs` Skate 3's sounds.

pub mod anim;
pub mod clips;
pub mod draw;
pub mod input;
pub mod physics;
pub mod sound;

#[cfg(test)]
mod convert;

use std::sync::Arc;

use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::*;
use bevy_egui::egui;
use se_core::skate3::{self, Model, Tuning};

use crate::mods::ModControl;
use crate::sandbox::world::SandboxWorld;
use crate::sandbox::Sandbox;
use anim::{ClipIndex, Controller, Seen};
use input::{Flick, MouseStick, Pad, Trick};
use physics::{Board, Drive, Phase, Tune};

/// The mod's files, read and checked at its first enable (kept until quit).
#[derive(Resource, Clone)]
pub struct SkateAssets {
    pub model: Arc<Model>,
    pub clips: Arc<ClipIndex>,
    pub tune: Arc<Tune>,
    /// Each draw's texture as RGBA8 (width, height, pixels), by its file.
    pub textures: Arc<Vec<(String, u32, u32, Vec<u8>)>>,
    pub sounds: Arc<sound::SoundBank>,
}

/// Reads the mod's files through `read` (the mods menu's checked reader: a refusal names the file and why).
pub fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>, needs: &[String]) -> Result<SkateAssets, (String, String)> {
    let model = skate3::parse_model(&read("skater.s3m").map_err(|e| ("skater.s3m".to_string(), e))?).map_err(|e| ("skater.s3m".to_string(), e.to_string()))?;
    let clips = skate3::parse_clips(&read("clips.s3c").map_err(|e| ("clips.s3c".to_string(), e))?).map_err(|e| ("clips.s3c".to_string(), e.to_string()))?;
    skate3::check_pair(&model, &clips).map_err(|e| ("clips.s3c".to_string(), e.to_string()))?;
    for (name, _) in clips::CLIPS {
        if clips.get(name).is_none() {
            return Err(("clips.s3c".to_string(), format!("clip {name} is missing")));
        }
    }
    let text = String::from_utf8(read("tuning.cfg").map_err(|e| ("tuning.cfg".to_string(), e))?).map_err(|_| ("tuning.cfg".to_string(), "not text".to_string()))?;
    let tuning = Tuning::parse(&text).map_err(|e| ("tuning.cfg".to_string(), e.to_string()))?;
    let tune = Tune::read(&tuning).map_err(|e| ("tuning.cfg".to_string(), e))?;
    let mut textures = Vec::new();
    for d in &model.draws {
        if d.texture.is_empty() || textures.iter().any(|(n, ..): &(String, u32, u32, Vec<u8>)| *n == d.texture) {
            continue;
        }
        let bytes = read(&d.texture).map_err(|e| (d.texture.clone(), e))?;
        let img = image::load_from_memory(&bytes).map_err(|e| (d.texture.clone(), e.to_string()))?.to_rgba8();
        textures.push((d.texture.clone(), img.width(), img.height(), img.into_raw()));
    }
    let sounds = sound::SoundBank::load(read, needs).map_err(|(f, e)| (f, e))?;
    Ok(SkateAssets { model: Arc::new(model), clips: Arc::new(ClipIndex::new(clips)), tune: Arc::new(tune), textures: Arc::new(textures), sounds: Arc::new(sounds) })
}

/// Something heard or seen that a tick did (`sound.rs`, the trick's name on screen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkateEvent {
    /// The pop, with its strength (0..1).
    Pop(f32),
    /// A flip trick's board leaves the feet: the whoosh.
    Flip,
    Land(f32),
    Bail,
    Push,
    HitWall(f32),
    Trick(Trick, bool),
}

/// The skater while the mod is on (held by the sandbox: `Sandbox::skate`).
pub struct Skater {
    pub board: Board,
    pub ctl: Controller,
    pub tune: Arc<Tune>,
    pub pad: Pad,
    flick: Flick,
    pub mouse: MouseStick,
    /// The board a tick ago, drawn between the two.
    pub prev_pos: Vec3,
    pub prev_rot: Quat,
    /// The chase camera's direction (radians from east) and where it is (local frame).
    cam_yaw: f32,
    cam_eye: Option<Vec3>,
    /// What the ticks did since the frame's sounds took them.
    pub events: Vec<SkateEvent>,
    /// The words on screen and how long they stay (a trick's name).
    pub caption: Option<(String, f32)>,
    /// The wheels rolled this frame, and how fast (the rolling sound).
    pub rolling: Option<f32>,
    /// The foot brake or a powerslide is on (their loops).
    pub braking: bool,
    pub sliding: bool,
    pub log: bool,
}

impl Skater {
    pub fn new(assets: &SkateAssets, pos: Vec3, heading: f32) -> Skater {
        let board = Board::at(pos, heading);
        let q = Quat::from_mat3(&board.rotation(0.0));
        Skater {
            board,
            ctl: Controller::new(assets.clips.clone()),
            tune: assets.tune.clone(),
            pad: Pad::default(),
            flick: Flick::default(),
            mouse: MouseStick::default(),
            prev_pos: pos,
            prev_rot: q,
            cam_yaw: heading,
            cam_eye: None,
            events: Vec::new(),
            caption: None,
            rolling: None,
            braking: false,
            sliding: false,
            log: false,
        }
    }

    /// One tick: the controls, what he does, the board.
    pub fn tick(&mut self, world: &mut SandboxWorld, gravity: f32, dt: f32) {
        self.prev_pos = self.board.pos;
        self.prev_rot = Quat::from_mat3(&self.board.rotation(self.board.slide));
        let pad = self.pad;
        // (One-shots act on the frame's first tick.)
        self.pad.trick = None;
        self.pad.off = false;
        let flick = self.flick.step(&pad, dt);
        let slide_side = if pad.slide && pad.left.x.abs() > 0.4 { pad.left.x.signum() } else { 0.0 };
        let seen = Seen {
            phase: self.board.phase,
            speed: self.board.speed(),
            fakie: self.board.fakie,
            push: pad.push,
            brake: pad.brake,
            slide_side,
            crouch: flick.crouch,
            pop: flick.pop.map(|t| (t, flick.crouched_for)),
        };
        let asked = self.ctl.before(&seen, dt);
        let hips = anim::hips_height(self.ctl.anim.pose(None), &[]);
        let drive = Drive { steer: pad.left.x, push: asked.push, pushing: asked.pushing, brake: asked.brake, slide: asked.slide, pop: asked.pop, hips_m: hips, any: pad.any };
        let happened = self.board.step(&self.tune, world, &drive, gravity, dt);
        if happened.popped {
            self.events.push(SkateEvent::Pop(drive.pop.unwrap_or(0.5)));
            if asked.popped_trick.is_some_and(|t| t != Trick::Ollie) {
                self.events.push(SkateEvent::Flip);
            }
        }
        if asked.push {
            self.events.push(SkateEvent::Push);
        }
        if let Some(v) = happened.hit_wall {
            self.events.push(SkateEvent::HitWall(v));
        }
        let after = self.ctl.after(self.board.phase, happened.landed, happened.bailed, self.board.bail_time, self.board.speed());
        if after.bail_board {
            let mut ignore = physics::Happened::default();
            self.board.bail(&mut ignore);
        }
        if after.bailed {
            self.events.push(SkateEvent::Bail);
            if let Some((t, false)) = self.ctl.last_trick {
                self.events.push(SkateEvent::Trick(t, false));
            }
        }
        if let Some(down) = happened.landed.filter(|_| after.landed) {
            self.events.push(SkateEvent::Land(down));
        }
        if let Some(t) = after.landed_trick {
            self.events.push(SkateEvent::Trick(t, true));
            self.caption = Some((t.name().to_string(), 1.6));
        }
        if after.recover {
            self.board.recover();
        }
        self.rolling = happened.rolling.then_some(self.board.speed());
        self.braking = asked.brake && happened.rolling && self.board.speed() > 0.3;
        self.sliding = self.board.slide.abs() > 30.0 && happened.rolling;
        if self.log && (happened.popped || happened.landed.is_some() || after.bailed) {
            info!(
                "skate: {:?} at {:.1} m/s ({:.1} up), {:?}{}",
                self.ctl.state,
                self.board.speed(),
                self.board.vel.z,
                self.board.phase,
                happened.landed.map(|d| format!(", landed at {d:.1} m/s down")).unwrap_or_default()
            );
        }
    }

    /// The sandbox's frame moved to a new anchor (it follows him every `REBASE_M`): everything of his into the new frame.
    pub fn rebase(&mut self, shift: &dyn Fn(Vec3) -> Vec3, turn: bevy::math::Mat3) {
        let yaw_of = |h: f32| {
            let f = turn * Vec3::new(h.cos(), h.sin(), 0.0);
            f.y.atan2(f.x)
        };
        self.board.pos = shift(self.board.pos);
        self.prev_pos = shift(self.prev_pos);
        self.board.vel = turn * self.board.vel;
        self.board.up = (turn * self.board.up).normalize_or(Vec3::Z);
        self.board.heading = yaw_of(self.board.heading);
        self.prev_rot = Quat::from_mat3(&turn) * self.prev_rot;
        self.cam_yaw = yaw_of(self.cam_yaw);
        self.cam_eye = self.cam_eye.map(|e| shift(e));
    }

    /// The ground under him was replaced and he keeps his height over it (as the sandbox's walker does).
    pub fn lift(&mut self, dz: f32) {
        self.board.pos.z += dz;
        self.prev_pos.z += dz;
        if let Some(e) = self.cam_eye.as_mut() {
            e.z += dz;
        }
    }

    /// The board between the last two ticks: its deck's middle and its turn (local frame).
    pub fn drawn(&self, k: f32) -> (Vec3, Quat) {
        let now = Quat::from_mat3(&self.board.rotation(self.board.slide));
        (self.prev_pos.lerp(self.board.pos, k), self.prev_rot.slerp(now, k))
    }

    /// The chase camera: behind him along his path (his nose when slow), a little above, looking at his middle; never inside
    /// a wall or under the ground. Returns the eye and the point it looks at (local frame).
    pub fn chase(&mut self, world: &SandboxWorld, k: f32, dt: f32) -> (Vec3, Vec3) {
        let (pos, _) = self.drawn(k);
        let v = Vec2::new(self.board.vel.x, self.board.vel.y);
        let want = if v.length() > 1.2 && self.board.phase != Phase::Bail { v.y.atan2(v.x) } else { self.board.heading + if self.board.fakie { std::f32::consts::PI } else { 0.0 } };
        let mut d = (want - self.cam_yaw).rem_euclid(std::f32::consts::TAU);
        if d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        let rate = if self.board.phase == Phase::Air { 1.2 } else { 3.0 };
        self.cam_yaw += d * (1.0 - (-dt * rate).exp());
        let back = Vec3::new(self.cam_yaw.cos(), self.cam_yaw.sin(), 0.0);
        let target = pos + Vec3::Z * 1.0;
        let mut eye = target - back * 3.3 + Vec3::Z * 0.75;
        // Out of walls: as far back as the way is clear.
        let to = eye - target;
        if let Some(hit) = world.ray(target, to.normalize(), to.length() + 0.3, None) {
            eye = target + to.normalize() * (hit.dist - 0.3).max(0.3);
        }
        // Over the ground.
        if let Some(hit) = world.ray(eye + Vec3::Z * 3.0, -Vec3::Z, 6.0, None) {
            eye.z = eye.z.max(hit.point.z + 0.45);
        }
        // Smoothly, but never far behind.
        let eye = match self.cam_eye {
            Some(prev) if (prev - eye).length() < 6.0 => prev.lerp(eye, 1.0 - (-dt * 10.0).exp()),
            _ => eye,
        };
        self.cam_eye = Some(eye);
        (eye, target)
    }
}

/// The frame's controls into the skater's pad: the gamepad if it was used last, else the keyboard and mouse. (Before the
/// sandbox's `read_input`, which takes the key presses from everything else after.)
#[allow(clippy::too_many_arguments)]
pub fn read_pad(
    mut sb: ResMut<Sandbox>,
    control: Res<ModControl>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    pads: Query<&Gamepad>,
    time: Res<Time<Real>>,
    cfg: Res<crate::config::EngineConfig>,
    mut dev: ResMut<DevPad>,
) {
    let menus = control.menus_open() || control.main_menu;
    let real = crate::mods::real_window(&cfg);
    let dt = time.delta_secs().min(0.1);
    let Some(sk) = sb.skate.as_mut() else { return };
    let mut pad = Pad::default();
    if !menus {
        let held = |k: KeyCode| keys.pressed(k);
        let axis = |plus: KeyCode, minus: KeyCode| (held(plus) as i8 - held(minus) as i8) as f32;
        pad.left = Vec2::new(axis(KeyCode::KeyD, KeyCode::KeyA), axis(KeyCode::KeyW, KeyCode::KeyS));
        pad.push = held(KeyCode::KeyW);
        pad.brake = held(KeyCode::KeyS);
        pad.slide = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        pad.crouch_key = held(KeyCode::Space);
        pad.off = keys.just_pressed(KeyCode::KeyF);
        const TRICK_KEYS: [(KeyCode, Trick); 6] = [
            (KeyCode::Digit1, Trick::Kickflip),
            (KeyCode::Digit2, Trick::Heelflip),
            (KeyCode::Digit3, Trick::Shuvit),
            (KeyCode::Digit4, Trick::FsShuvit),
            (KeyCode::Digit5, Trick::Flip360),
            (KeyCode::Digit6, Trick::Laser),
        ];
        pad.trick = TRICK_KEYS.iter().find(|(k, _)| keys.just_pressed(*k)).map(|(_, t)| *t);
        let counts = if real { motion.delta } else { Vec2::ZERO };
        pad.right = sk.mouse.step(counts, dt);
        let _ = &buttons;
        pad.any = keys.get_pressed().next().is_some() || counts.length() > 0.5;
        // A gamepad that's being used takes over.
        for g in &pads {
            let l = g.left_stick();
            let r = g.right_stick();
            let used = l.length() > 0.2 || r.length() > 0.2 || g.get_pressed().next().is_some();
            if !used {
                continue;
            }
            pad.left = if l.length() > 0.15 { l } else { Vec2::ZERO };
            pad.right = r;
            pad.push = g.pressed(GamepadButton::South);
            pad.brake = g.pressed(GamepadButton::East) && l.x.abs() < 0.4;
            pad.slide = g.pressed(GamepadButton::East);
            pad.off = g.just_pressed(GamepadButton::North);
            pad.crouch_key = false;
            pad.any = true;
            break;
        }
    }
    // A test's pad (`skate pad ...`) stands in for both while it lasts.
    if dev.left > 0.0 {
        dev.left -= dt;
        pad = Pad { trick: pad.trick.or(dev.pad.trick), ..dev.pad };
        dev.pad.trick = None;
        pad.any = true;
    }
    if pad.off {
        info!("skate: off the board (F / Y)");
    }
    sk.pad = Pad { trick: pad.trick.or(sk.pad.trick), off: pad.off || sk.pad.off, ..pad };
}

/// A test's controls: held for a number of seconds (`skate pad`).
#[derive(Resource, Default)]
pub struct DevPad {
    pad: Pad,
    left: f32,
}

/// The mod turned on or off: the skater stands on his board where the walker was (once the sandbox stands on ground), or
/// the walker stands where he was.
pub fn sync(mut sb: ResMut<Sandbox>, mut control: ResMut<ModControl>, assets: Option<Res<SkateAssets>>) {
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
            let heading = sb.yaw;
            let mut sk = Skater::new(assets, feet, heading);
            if !sk.board.stand_on(world, &sk.tune, feet + Vec3::new(heading.cos(), heading.sin(), 0.0) * 0.6) {
                sk.board.stand_on(world, &sk.tune, feet);
            }
            sk.prev_pos = sk.board.pos;
            info!("skate: on the board at {:.1} {:.1} {:.1}", sk.board.pos.x, sk.board.pos.y, sk.board.pos.z);
            sb.skate = Some(sk);
        }
    } else if !want && sb.skate.is_some() {
        if let Some(sk) = sb.skate.take() {
            // On foot where he was, facing the way he rode.
            sb.player.origin = sk.board.pos / crate::sandbox::movement::UNIT_M;
            sb.player.velocity = Vec3::ZERO;
            sb.yaw = sk.board.heading;
            info!("skate: off the board");
        }
    }
    if let Some(sk) = sb.skate.as_ref() {
        if sk.pad.off {
            control.request_off();
        }
    }
}

/// The trick's name on screen, Skate 3's way: big, for a moment, in the middle near the top.
pub fn draw_caption(mut contexts: bevy_egui::EguiContexts, mut sb: ResMut<Sandbox>, time: Res<Time<Real>>, control: Res<ModControl>) -> Result {
    let Some(sk) = sb.skate.as_mut() else { return Ok(()) };
    let Some((text, left)) = sk.caption.as_mut() else { return Ok(()) };
    *left -= time.delta_secs();
    if *left <= 0.0 || control.main_menu {
        sk.caption = None;
        return Ok(());
    }
    let alpha = (*left / 0.4).clamp(0.0, 1.0);
    let ctx = contexts.ctx_mut()?;
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("skate caption")));
    let pos = egui::pos2(rect.center().x, rect.top() + rect.height() * 0.22);
    let font = egui::FontId::proportional(40.0);
    let shadow = egui::Color32::from_black_alpha((180.0 * alpha) as u8);
    painter.text(pos + egui::vec2(2.0, 2.0), egui::Align2::CENTER_CENTER, text.as_str(), font.clone(), shadow);
    painter.text(pos, egui::Align2::CENTER_CENTER, text.as_str(), font, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * alpha) as u8));
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
            .add_systems(bevy_egui::EguiPrimaryContextPass, draw_caption.in_set(crate::skin::UiSet::Draw))
            .add_systems(PostUpdate, sound::play.after(crate::sandbox::sandbox_step));
    }
}

/// `skate state|log on|off|pad KEYS SECS|trick NAME|tune`: test commands (`--run "skate ..."`).
pub fn dev_command(world: &mut World, arg: &str) {
    let mut parts = arg.split_whitespace();
    let verb = parts.next().unwrap_or("").to_ascii_lowercase();
    let rest: Vec<&str> = parts.collect();
    match verb.as_str() {
        "state" => {
            let sb = world.resource::<Sandbox>();
            match sb.skate.as_ref() {
                Some(sk) => info!(
                    "skate: {:?}, {:?}, {:.2} m/s, heading {:.0} deg, tilt {:.1} deg, slide {:.0} deg, fakie {}, clip {} ({:.2}), at {:.2} {:.2} {:.2}, up {:.2} {:.2} {:.2}",
                    sk.ctl.state,
                    sk.board.phase,
                    sk.board.speed(),
                    sk.board.heading.to_degrees().rem_euclid(360.0),
                    sk.board.tilt.to_degrees(),
                    sk.board.slide,
                    sk.board.fakie,
                    sk.ctl.anim.playing(),
                    sk.ctl.anim.progress(),
                    sk.board.pos.x,
                    sk.board.pos.y,
                    sk.board.pos.z,
                    sk.board.up.x,
                    sk.board.up.y,
                    sk.board.up.z
                ),
                None => info!("skate: not on the board (mod on: {})", world.resource::<ModControl>().skate),
            }
        }
        "log" => {
            let on = rest.first().copied() != Some("off");
            if let Some(sk) = world.resource_mut::<Sandbox>().skate.as_mut() {
                sk.log = on;
            }
        }
        "pad" => {
            // `skate pad push+left 2`: push, brake, slide, crouch, left, right, up, down (the left stick), rdown, rup,
            // rleft, rright (the right stick), held for the seconds given.
            let keys = rest.first().copied().unwrap_or("");
            let secs: f32 = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.5);
            let mut p = Pad::default();
            for k in keys.split('+') {
                match k {
                    "push" => p.push = true,
                    "brake" => p.brake = true,
                    "slide" => p.slide = true,
                    "crouch" => p.crouch_key = true,
                    "left" => p.left.x = -1.0,
                    "right" => p.left.x = 1.0,
                    "up" => p.left.y = 1.0,
                    "down" => p.left.y = -1.0,
                    "rdown" => p.right.y = -1.0,
                    "rup" => p.right.y = 1.0,
                    "rleft" => p.right.x = -1.0,
                    "rright" => p.right.x = 1.0,
                    "off" | "" => {}
                    other => warn!("skate pad: no key {other}"),
                }
            }
            let mut dev = world.resource_mut::<DevPad>();
            dev.pad = p;
            dev.left = secs;
        }
        "trick" => match rest.first().and_then(|n| Trick::by_name(&rest.join(" ")).or(Trick::by_name(n))) {
            Some(t) => {
                // The trick alone: whatever a `skate pad` held is let go.
                let mut dev = world.resource_mut::<DevPad>();
                dev.pad = Pad { trick: Some(t), ..Default::default() };
                dev.left = 0.05;
            }
            None => warn!("skate trick: which? (ollie, kickflip, heelflip, shuvit, fsshuvit, 360flip, laser)"),
        },
        _ => warn!("skate: state | log on|off | pad KEYS SECS | trick NAME"),
    }
}

/// The skater's state for the sandbox's own log line.
pub fn describe(sk: &Skater) -> String {
    format!("skating: {:?} {:?} at {:.1} m/s", sk.ctl.state, sk.board.phase, sk.board.speed())
}
