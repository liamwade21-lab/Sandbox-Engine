//! Runs SpaceEngine `.se` scripts (see `docs/formats/se_script.md`).
//!
//! `se_core::se_script` parses a script into a flat command list; [`ScriptRunner`]
//! executes it over frames: instant commands run back to back within a frame, while
//! `Wait`, `WaitTrigger`, `WaitMessage` and `WaitVar` block the script until their
//! condition holds. `Goto` flights, `Orbit`, `Fly`, `Turn`, `FOV` and `Interpolate` run as
//! effects alongside the script. Camera moves reuse the free-flight camera
//! ([`crate::camera::FlyCamera`]) and its `Follow` binding, selection is
//! `FlyCamera::selected` (or a script waypoint, [`crate::waypoints`]), time commands drive
//! [`SimTime`], settings go through [`crate::script_vars`], the tutorial interface hooks
//! through [`crate::script_gui`] and music through [`crate::music::ScriptMusic`].
//!
//! `User*Control` locks the user's movement and rotation ([`crate::camera::InputLocks`]) and
//! time controls (the time keys are swallowed and other changes undone) until the script
//! frees them or ends. When a script ends SE resets its interface, waypoints and locks
//! (FUN_14054d240, FUN_140221b10), and so does the port.
//!
//! Commands the port has no equivalent for are logged once and skipped. Triggers that wait
//! for GUI actions the port does not have (the universe map and system chart controls,
//! panel pins, clicks inside dialogs) give up after a few seconds so tutorials keep
//! flowing; the "Skip step" button ends any wait early.
//!
//! The Scripts window (HUD) lists the scripts shipped in `data/scripts` and runs one.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{BodyTag, Catalog, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::sc::Value;
use se_core::se_script::{self, Command, ControlLimits, GotoArgs, InputKind, LockMode, Location, MarkupLine, MultiMode, Op, Script, Trigger, VarWait};
use se_core::places::PC_M;
use se_core::upos::UPos;

use crate::camera::{Follow, FlyCamera, TravelKind, clamp_speed, look_rotation};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::hud::HudState;
use crate::i18n::I18n;
use crate::music::{MusicRequest, ScriptMusic};
use crate::script_gui::{ActiveHighlight, Dialogs, ScriptGui};
use crate::script_vars::Settings;
use crate::universe::SimTime;
use crate::waypoints::Waypoints;

/// Triggers on GUI actions the port lacks give up after this many seconds.
const UNSUPPORTED_WAIT_S: f64 = 4.0;
/// With `--script-auto`, any wait for the user ends after this many seconds.
const AUTO_TRIGGER_S: f64 = 8.0;
const AUTO_MESSAGE_S: f64 = 4.0;
/// `WaitTrigger "LoadingComplete"`: the port streams content as it goes, so just
/// give it a moment.
const LOADING_SETTLE_S: f64 = 1.0;
const MAX_OPS_PER_FRAME: usize = 4000;
const MAX_CALL_DEPTH: usize = 8;
const DEFAULT_GOTO_S: f64 = 4.0;
const DEFAULT_FOV: f32 = 60.0;
/// The engine version scripts check with `CheckVersion` (SE 0.991).
const ENGINE_VERSION: u32 = 991;
/// `Control|Move` matches a `Direction` when every component of the movement's direction is
/// within 1% of it (FUN_140275dd0: |d - n| <= 0.01 |d + n|), plus rounding.
const DIRECTION_TOLERANCE: f64 = 0.01;
/// `Control|SetTime`: a date change bigger than the clock's own step by this much (days, 1 min).
const SET_TIME_JUMP_D: f64 = 1.0 / 1440.0;
/// The navigation toolbar's date as SE's `GUI|LeftClick` names it (tut-03).
const DATE_WIDGET: &str = "Navigation toolbar|Date";
/// The time keys a script's `UserTimeControl "Disabled"` swallows (Space, L, K, J, \).
const TIME_KEYS: [KeyCode; 5] = [KeyCode::Space, KeyCode::KeyL, KeyCode::KeyK, KeyCode::KeyJ, KeyCode::Backslash];

pub struct ScriptingPlugin;

impl Plugin for ScriptingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((crate::waypoints::WaypointsPlugin, crate::script_gui::ScriptGuiPlugin))
            .init_resource::<ScriptRunner>()
            .init_resource::<ScriptUi>()
            .add_systems(Startup, queue_cli_script)
            .add_systems(PreUpdate, swallow_time_keys.after(bevy::input::InputSystems))
            // Idle, the runner takes no part in the frame (it borrows many settings).
            .add_systems(Update, run_script.run_if(|r: Res<ScriptRunner>| r.running() || r.pending.is_some() || r.stop))
            .add_systems(EguiPrimaryContextPass, (draw_captions, draw_scripts_window).in_set(crate::skin::UiSet::Draw));
    }
}

/// While a script disables the user's time controls, their keys never reach the HUD.
fn swallow_time_keys(runner: Res<ScriptRunner>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    if runner.time_guard.as_ref().is_some_and(|g| g.locked) {
        for k in TIME_KEYS {
            keys.reset(k);
        }
    }
}

/// The scripts window toggled by the HUD's "Scripts" button.
#[derive(Resource, Default)]
pub struct ScriptUi {
    pub show: bool,
    entries: Option<Vec<(String, Vec<String>)>>,
}

/// Message box text (`ShowMessage` / `WaitMessage`).
#[derive(Debug, Clone)]
pub struct Caption {
    pub lines: Vec<MarkupLine>,
    /// Waiting for the "Ok" button (`WaitMessage`).
    pub ack: bool,
}

struct Frame {
    name: String,
    script: Arc<Script>,
    pc: usize,
    loops: Vec<LoopState>,
}

struct LoopState {
    remaining: Option<u32>,
}

/// Camera and time state when a trigger started waiting; triggers compare against it.
#[derive(Clone)]
struct Snapshot {
    rot: DQuat,
    fov: f32,
    speed: f64,
    rate: f64,
    paused: bool,
    jd: f64,
    selected: Option<ObjectRef>,
    /// The selected waypoint's name.
    selected_wp: Option<String>,
    travel: bool,
    following: bool,
    rotating: bool,
    track: bool,
}

struct TrigState {
    trig: Trigger,
    snap: Snapshot,
    done: bool,
    /// `Control|Move`: the user's movement since the trigger started (m), as SE sums it.
    moved: DVec3,
    /// `GUI|Open` / `GUI|Close`: the dialog has been seen closed / open since the start.
    seen_closed: bool,
    seen_open: bool,
    /// `Control|SetTime`: the date last frame.
    prev_jd: f64,
}

enum Blocked {
    Seconds(f64),
    Triggers { mode: MultiMode, items: Vec<TrigState>, elapsed: f64, max_wait: Option<f64> },
    /// `then_stop`: the script ends after the Ok (`CheckVersion` for another version).
    Message { elapsed: f64, then_stop: bool },
    Var { name: String, wait: VarWait, elapsed: f64 },
}

/// A `Fly` or `Turn`: a speed along or about a camera axis, eased to its target.
#[derive(Clone, Copy)]
struct Motion {
    /// Port camera axes (x right, y up, z back).
    axis: DVec3,
    cur: f64,
    target: f64,
    /// Change per second.
    rate: f64,
}

impl Motion {
    fn new(axis: DVec3, from: f64, target: f64, fade: f64) -> Self {
        let rate = if fade > 0.0 { (target - from).abs() / fade } else { f64::INFINITY };
        Self { axis, cur: from, target, rate }
    }

    /// Advances the speed; false once it has stopped.
    fn step(&mut self, dt: f64) -> bool {
        let d = self.target - self.cur;
        self.cur = if d.abs() <= self.rate * dt { self.target } else { self.cur + d.signum() * self.rate * dt };
        !(self.cur == 0.0 && self.target == 0.0)
    }
}

/// A script's `UserTimeControl`: `locked` ("Disabled") holds the rate and pause state the script
/// last set; "Limited" keeps the user's changes inside `TimeRange` / `TimeRateRange`.
#[derive(Clone)]
struct TimeGuard {
    locked: bool,
    rate: f64,
    paused: bool,
    limits: ControlLimits,
}

/// A script's `UserMoveControl "Limited"`.
#[derive(Clone)]
struct MoveLimit {
    center: Option<ObjectRef>,
    limits: ControlLimits,
}

/// A `Goto` flight, expressed in the target body's frame so it follows the body.
struct Flight {
    target: ObjectRef,
    start_local: DVec3,
    start_rot: DQuat,
    end_local: DVec3,
    end_rot: DQuat,
    elapsed: f64,
    duration: f64,
    /// Bind with SyncRot on arrival (a Goto to a latitude and longitude), else Follow.
    sync: bool,
    /// `AccelTime`/`DriftTime`/`DecelTime`/`AccelPower`/`DecelPower`; without them the flight
    /// eases in and out (smoothstep).
    profile: Option<Profile>,
}

/// A Goto's flight profile (FUN_140563f30 reads the keys; the flight time is their sum, as its
/// `local_1ac + local_1a8 + local_1a4`): progress grows as a power `pa` of time while speeding up,
/// evenly while drifting, and as a power `pd` of the time left while braking, with the speed
/// continuous between the phases. The curve shapes are the port's (SE's integrator was not
/// traced); a power of 1 is a constant speed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Profile {
    accel: f64,
    drift: f64,
    decel: f64,
    pa: f64,
    pd: f64,
}

impl Profile {
    /// SE's defaults for unset keys (FUN_140563f30: 1 s to speed up, none drifting, 1 s braking);
    /// the powers default to 2.
    fn from_args(a: &GotoArgs) -> Option<Self> {
        let any = a.accel_time.is_some() || a.drift_time.is_some() || a.decel_time.is_some() || a.accel_power.is_some() || a.decel_power.is_some();
        any.then(|| Self {
            accel: a.accel_time.unwrap_or(1.0).max(0.0),
            drift: a.drift_time.unwrap_or(0.0).max(0.0),
            decel: a.decel_time.unwrap_or(1.0).max(0.0),
            pa: a.accel_power.unwrap_or(2.0).max(1e-3),
            pd: a.decel_power.unwrap_or(2.0).max(1e-3),
        })
    }

    fn total(&self) -> f64 {
        self.accel + self.drift + self.decel
    }

    /// Progress 0..1 at the fraction `t` of the flight (stretched over the flight's duration).
    fn progress(&self, t: f64) -> f64 {
        let total = self.total();
        if total <= 0.0 {
            return 1.0;
        }
        let (a, d, c) = (self.accel / total, self.drift / total, self.decel / total);
        // Drift speed v: v (a / pa + d + c / pd) = 1.
        let v = 1.0 / (a / self.pa + d + c / self.pd);
        let t = t.clamp(0.0, 1.0);
        let s = if t < a {
            v * a / self.pa * (t / a).powf(self.pa)
        } else if t <= a + d {
            v * a / self.pa + v * (t - a)
        } else if c > 0.0 {
            1.0 - v * c / self.pd * ((1.0 - t) / c).powf(self.pd)
        } else {
            1.0
        };
        s.clamp(0.0, 1.0)
    }
}

struct Anim {
    from: f64,
    to: f64,
    elapsed: f64,
    duration: f64,
}

struct Interp {
    name: String,
    anim: Anim,
}

/// Executes one script (plus the scripts it `Run`s).
#[derive(Resource, Default)]
pub struct ScriptRunner {
    frames: Vec<Frame>,
    blocked: Option<Blocked>,
    building: Option<(MultiMode, Vec<TrigState>)>,
    /// Name of the top-level script, for the UI.
    pub name: String,
    /// One-line description of what the script is doing now.
    pub status: String,
    vars: HashMap<String, Value>,
    saved: Vec<HashMap<String, Value>>,
    flight: Option<Flight>,
    orbit: Option<(DVec3, f64)>,
    fly: Option<Motion>,
    turn: Option<Motion>,
    time_guard: Option<TimeGuard>,
    move_limit: Option<MoveLimit>,
    fov_anim: Option<Anim>,
    interps: Vec<Interp>,
    pub caption: Option<Caption>,
    print: Option<(String, Option<f64>)>,
    logged: HashSet<String>,
    unsupported_vars: Vec<String>,
    /// Settings' values before the script first changed them, for `Reset`.
    defaults: HashMap<String, f64>,
    /// The saved script variables (`LessonPassed`) were read.
    loaded_persistent: bool,
    hid_hud: bool,
    cur_line: u32,
    // UI -> runner requests
    pending: Option<String>,
    stop: bool,
    ack: bool,
    skip: bool,
}

impl ScriptRunner {
    pub fn running(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Ask the runner to start `path` (a VFS path like `scripts/menu/Scene1.se`,
    /// `tutorials/tut-00-interface.se`, or a file on disk) on the next frame.
    pub fn request(&mut self, path: &str) {
        self.pending = Some(path.to_string());
    }

    fn reset(&mut self) {
        self.frames.clear();
        self.blocked = None;
        self.building = None;
        self.flight = None;
        self.orbit = None;
        self.fly = None;
        self.turn = None;
        self.time_guard = None;
        self.move_limit = None;
        self.fov_anim = None;
        self.interps.clear();
        self.caption = None;
        self.print = None;
        self.saved.clear();
        self.status.clear();
    }

    /// What SE undoes when a script ends or is stopped (FUN_14054d240, FUN_140221b10): input
    /// locks, the tutorial interface state and the waypoints.
    fn end_effects(&mut self, cam: &mut FlyCamera, gui: &mut ScriptGui, wps: &mut Waypoints) {
        self.reset();
        cam.locks = Default::default();
        gui.reset();
        wps.clear();
    }

    fn warn_once(&mut self, key: &str, msg: String) {
        if self.logged.insert(key.to_string()) {
            warn!("script {}: {msg}", self.name);
        }
    }

    fn flush_unsupported_vars(&mut self) {
        if !self.unsupported_vars.is_empty() {
            let n = self.unsupported_vars.len();
            let first: Vec<&str> = self.unsupported_vars.iter().take(6).map(String::as_str).collect();
            info!("script {}: {n} settings have no equivalent in the port and were ignored (e.g. {})", self.name, first.join(", "));
            self.unsupported_vars.clear();
        }
    }
}

// ---- loading ----------------------------------------------------------------------

/// Reads a script by VFS path (with or without the `scripts/` prefix) or file path.
fn read_script(data: &SeData, path: &str) -> Option<(String, String)> {
    let p = path.replace('\\', "/");
    if std::path::Path::new(&p).is_file() {
        let bytes = std::fs::read(&p).ok()?;
        return Some((p, se_core::vfs::decode_text(&bytes)));
    }
    let stripped = p.strip_prefix("data/").unwrap_or(&p).to_string();
    let candidates = [
        stripped.clone(),
        format!("scripts/{stripped}"),
        format!("scripts/menu/{stripped}"),
        format!("scripts/tutorials/{stripped}"),
    ];
    for c in candidates {
        if data.vfs.exists(&c) {
            if let Ok(text) = data.vfs.read_string(&c) {
                return Some((c, text));
            }
        }
    }
    None
}

fn load_frame(data: &SeData, path: &str) -> Option<Frame> {
    let Some((name, text)) = read_script(data, path) else {
        error!("script not found: {path}");
        return None;
    };
    let script = se_script::parse(&text);
    for w in script.warnings.iter().take(10) {
        warn!("script {name}: {w}");
    }
    info!("script {name}: {} commands", script.commands.len());
    Some(Frame { name, script: Arc::new(script), pc: 0, loops: Vec::new() })
}

fn queue_cli_script(cfg: Res<EngineConfig>, mut runner: ResMut<ScriptRunner>, mut ui: ResMut<ScriptUi>) {
    // `SE_SCRIPTS_WINDOW=1` opens the Scripts window at startup (for screenshots).
    ui.show = std::env::var_os("SE_SCRIPTS_WINDOW").is_some();
    if let Some(p) = &cfg.script {
        runner.request(p);
    }
}

// ---- execution context -------------------------------------------------------------

struct Cx<'a, 'w, 'd> {
    cam: &'a mut FlyCamera,
    sim: &'a mut SimTime,
    data: &'a SeData,
    vars: &'a mut Settings<'w>,
    dialogs: &'a mut Dialogs<'d>,
    music: &'a mut ScriptMusic,
    /// `--script-auto`: waits for the user end after this many seconds (`SE_SCRIPT_AUTO_S`, else 8).
    auto: Option<f64>,
    aspect: f64,
    quit: bool,
    /// The script ended (finished or `CheckVersion` stopped it): undo its effects.
    ended: bool,
}

impl<'a> Cx<'a, '_, '_> {
    fn cat(&self) -> &'a Catalog {
        let data: &'a SeData = self.data;
        &data.catalog
    }

    fn wps(&self) -> &Waypoints {
        &self.vars.waypoints
    }
}

#[allow(clippy::too_many_arguments)]
fn run_script(
    time: Res<Time>,
    data: Res<SeData>,
    cfg: Res<EngineConfig>,
    mut runner: ResMut<ScriptRunner>,
    mut cam: ResMut<FlyCamera>,
    mut sim: ResMut<SimTime>,
    mut vars: Settings,
    mut dialogs: Dialogs,
    mut music: ResMut<ScriptMusic>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(path) = runner.pending.take() {
        runner.end_effects(&mut cam, &mut dialogs.gui, &mut vars.waypoints);
        runner.stop = false;
        if !runner.loaded_persistent {
            runner.loaded_persistent = true;
            for (k, v) in crate::script_vars::load_persistent() {
                runner.vars.insert(k, Value::Num(v));
            }
        }
        if let Some(frame) = load_frame(&data, &path) {
            runner.name = frame.name.clone();
            // The main-menu background scene is a bare scene: hide the interface.
            if frame.name.to_ascii_lowercase().contains("scripts/menu/") && !dialogs.hud.hidden {
                dialogs.hud.hidden = true;
                runner.hid_hud = true;
            }
            runner.frames.push(frame);
        }
    }
    if runner.frames.is_empty() && !runner.stop {
        return;
    }
    let aspect = windows.single().map(|w| (w.width() / w.height().max(1.0)) as f64).unwrap_or(16.0 / 9.0);
    let mut cx = Cx {
        cam: &mut cam,
        sim: &mut sim,
        data: &data,
        vars: &mut vars,
        dialogs: &mut dialogs,
        music: &mut music,
        auto: cfg.script_auto.then(|| std::env::var("SE_SCRIPT_AUTO_S").ok().and_then(|v| v.parse().ok()).unwrap_or(AUTO_TRIGGER_S)),
        aspect,
        quit: false,
        ended: false,
    };
    if runner.stop {
        runner.stop = false;
        // The port's choice: a script stopped by hand gives back the settings its first SaveVars
        // kept, as its RestoreVars would have (the port saves settings as they change).
        if let Some(saved) = runner.saved.first().cloned() {
            for (k, v) in saved {
                runner.set_var(&mut cx, &k, v);
            }
        }
        runner.end_effects(cx.cam, &mut cx.dialogs.gui, &mut cx.vars.waypoints);
        if runner.hid_hud {
            cx.dialogs.hud.hidden = false;
            runner.hid_hud = false;
        }
        info!("script {} stopped", runner.name);
        return;
    }
    runner.step(&mut cx, time.delta_secs_f64());
    if cx.ended {
        runner.frames.clear();
        let name = std::mem::take(&mut runner.name);
        let status = std::mem::take(&mut runner.status);
        runner.end_effects(cx.cam, &mut cx.dialogs.gui, &mut cx.vars.waypoints);
        (runner.name, runner.status) = (name, status);
    }
    if cx.quit {
        exit.write(AppExit::Success);
    }
}

impl ScriptRunner {
    fn step(&mut self, cx: &mut Cx, dt: f64) {
        self.guard_user_input(cx, dt);
        self.step_inner(cx, dt);
        // What the script itself set this frame is what the time lock holds.
        if let Some(g) = &mut self.time_guard {
            g.rate = cx.sim.rate;
            g.paused = cx.sim.paused;
        }
    }

    fn step_inner(&mut self, cx: &mut Cx, mut dt: f64) {
        if self.blocked.is_none() {
            // A click on "Skip step" / "Ok" with nothing to skip must not leak into the next wait.
            self.skip = false;
            self.ack = false;
        }
        self.update_effects(cx, dt);
        for _ in 0..MAX_OPS_PER_FRAME {
            if self.blocked.is_some() {
                if !self.poll_blocked(cx, dt) {
                    return;
                }
                self.blocked = None;
                dt = 0.0;
                if cx.ended {
                    return;
                }
            }
            let Some(fr) = self.frames.last_mut() else { return };
            if fr.pc >= fr.script.commands.len() {
                let done = self.frames.pop().unwrap();
                self.flush_unsupported_vars();
                if self.frames.is_empty() {
                    info!("script {} finished", done.name);
                    self.status = "finished".into();
                    self.caption = None;
                    self.print = None;
                    cx.ended = true;
                    return;
                }
                continue;
            }
            let script = fr.script.clone();
            let idx = fr.pc;
            fr.pc += 1;
            self.exec(cx, &script.commands[idx]);
        }
        self.status = "running (command limit per frame reached)".into();
    }

    fn jump(&mut self, pc: usize) {
        if let Some(fr) = self.frames.last_mut() {
            fr.pc = pc;
        }
    }

    fn exec(&mut self, cx: &mut Cx, cmd: &Command) {
        self.cur_line = cmd.line;
        let eph_jd = cx.sim.jd;
        match &cmd.op {
            // FUN_140565510: a script for another version stops behind a message box.
            Op::CheckVersion(v) => {
                if *v != ENGINE_VERSION {
                    let text = format!("This script was designed for a different version of SpaceEngine ({v}, this is {ENGINE_VERSION})");
                    warn!("script {}: {text}", self.name);
                    self.caption = Some(Caption { lines: se_script::markup_lines(&text), ack: true });
                    self.ack = false;
                    self.blocked = Some(Blocked::Message { elapsed: 0.0, then_stop: true });
                }
            }
            Op::Run(p) => {
                if self.frames.len() >= MAX_CALL_DEPTH {
                    self.warn_once("run-depth", format!("Run \"{p}\" nested too deeply (line {})", cmd.line));
                } else if let Some(f) = load_frame(cx.data, p) {
                    self.frames.push(f);
                }
            }
            Op::Quit => {
                cx.quit = true;
                self.frames.clear();
            }
            Op::Restart => {
                self.frames.truncate(1);
                if let Some(f) = self.frames.first_mut() {
                    f.pc = 0;
                    f.loops.clear();
                }
            }
            // A waypoint of that name first, then the catalog.
            Op::Select(name) => {
                if let Some(w) = cx.wps().find(name).map(|w| w.name.clone()) {
                    cx.cam.selected = None;
                    cx.vars.waypoints.selected = Some(w);
                } else {
                    match resolve_object(cx.cat(), name) {
                        Some(r) => {
                            cx.cam.selected = Some(r);
                            cx.vars.waypoints.selected = None;
                        }
                        None => self.warn_once(&format!("sel:{name}"), format!("object not found: \"{name}\" (line {})", cmd.line)),
                    }
                }
            }
            Op::Unselect => {
                cx.cam.selected = None;
                cx.vars.waypoints.selected = None;
            }
            Op::Center { .. } => {
                // FUN_140563cf0 refuses while the user's rotation is locked.
                if cx.cam.locks.rotation {
                    self.warn_once("center-locked", format!("camera rotation is limited, ignoring Center command (line {})", cmd.line));
                } else if let Some(pos) = selection_pos(cx) {
                    let d = pos.delta_m(cx.cam.pos);
                    cx.cam.rot = look_rotation(d, cx.cam.rot * DVec3::Y);
                }
            }
            // FUN_1405986f0 / FUN_140598810 / FUN_140597720 do nothing while movement is locked.
            Op::Follow | Op::SyncRot if !cx.cam.locks.movement => {
                if let Some(sel) = cx.cam.selected {
                    let eph = Ephemeris::new(cx.cat());
                    let sync = matches!(cmd.op, Op::SyncRot);
                    cx.cam.travel = None;
                    cx.cam.follow = Some(Follow::new(sel, sync, &eph, eph_jd));
                }
            }
            Op::Free if !cx.cam.locks.movement => cx.cam.follow = None,
            Op::Follow | Op::SyncRot | Op::Free => {}
            Op::GotoLocation(loc) => self.goto_location(cx, loc),
            // FUN_140563f30: "Camera movement is limited, ignoring Goto command".
            Op::Goto(_) if cx.cam.locks.movement => {
                warn!("script {}, line {}: camera movement is limited, ignoring Goto command", self.name, cmd.line);
            }
            Op::Goto(args) => self.start_goto(cx, args, cmd.line),
            Op::Fov { fov, time } => {
                let to = fov.unwrap_or(DEFAULT_FOV as f64);
                match time {
                    Some(t) if *t > 0.0 => {
                        self.fov_anim = Some(Anim { from: cx.cam.fov_deg as f64, to, elapsed: 0.0, duration: *t })
                    }
                    _ => {
                        self.fov_anim = None;
                        cx.cam.fov_deg = to as f32;
                    }
                }
            }
            Op::Orbit { axis, speed } => self.orbit = Some((DVec3::from_array(*axis), *speed)),
            Op::StopOrbit => self.orbit = None,
            Op::StopMotion => {
                cx.cam.velocity = DVec3::ZERO;
                cx.cam.travel = None;
                cx.cam.turn_vel = DVec3::ZERO;
                self.flight = None;
                self.fly = None;
                self.turn = None;
            }
            // `Horizon`: level the view to the selection's horizon (SE's Horizon object, End).
            Op::Horizon => {
                if let Some(sel) = cx.cam.selected {
                    crate::camera::start_horizon(cx.cam, sel);
                }
            }
            // SE's camera axes are x left, y down, z ahead: the port's with every axis reversed.
            Op::Fly { axis, speed, fade } => {
                let dir = -DVec3::from_array(*axis).normalize_or(DVec3::Z);
                let from = self.fly.map_or(0.0, |f| f.cur);
                self.fly = Some(Motion::new(dir, from, *speed, *fade));
            }
            Op::StopFly { fade } => {
                if let Some(f) = &mut self.fly {
                    *f = Motion::new(f.axis, f.cur, 0.0, *fade);
                }
            }
            // `Turn` about the axis as `Orbit` takes it (camera axes).
            Op::Turn { axis, speed, fade } => {
                let a = DVec3::from_array(*axis).normalize_or(DVec3::Y);
                let from = self.turn.map_or(0.0, |t| t.cur);
                self.turn = Some(Motion::new(a, from, *speed, *fade));
            }
            Op::StopTurn { fade } => {
                if let Some(t) = &mut self.turn {
                    *t = Motion::new(t.axis, t.cur, 0.0, *fade);
                }
            }
            Op::StopRotation => {
                cx.cam.turn_vel = DVec3::ZERO;
                self.turn = None;
            }
            Op::Speed(v) => cx.cam.speed = clamp_speed(*v),
            Op::MoveMode(m) => self.warn_once("movemode", format!("MoveMode {m} has no counterpart in the port, skipping (line {})", cmd.line)),
            Op::UserControl { kind, mode, limits } => self.user_control(cx, *kind, *mode, limits),
            Op::GotoUrl(u) => self.warn_once("gotourl", format!("GotoURL \"{u}\": se:// locations are not supported, skipping (line {})", cmd.line)),
            Op::HighlightWidget(h) => cx.dialogs.gui.highlight = ActiveHighlight::new(h),
            Op::SetWidgetStyle { widget, pos, pos_pix, visible } => {
                if widget.eq_ignore_ascii_case("Message box") {
                    if pos.is_some() || pos_pix.is_some() {
                        cx.dialogs.gui.message_pos = *pos;
                        cx.dialogs.gui.message_pos_pix = *pos_pix;
                    }
                    if *visible == Some(false) {
                        self.caption = None;
                    }
                } else {
                    self.warn_once(&format!("style:{widget}"), format!("SetWidgetStyle \"{widget}\": only the message box can be styled, skipping (line {})", cmd.line));
                }
            }
            Op::ShowDialog { dialog, tab } => {
                if !cx.dialogs.set_open(dialog, true, tab.as_deref()) {
                    self.warn_once(&format!("dlg:{dialog}"), format!("dialog \"{dialog}\" has no counterpart in the port (line {})", cmd.line));
                }
            }
            Op::HideDialog(dialog) => {
                cx.dialogs.set_open(dialog, false, None);
            }
            Op::HideAllDialogs => cx.dialogs.hide_all(),
            Op::HideAllToolbars => cx.dialogs.gui.toolbars = [true; 4],
            // SE's main menu is not ported; its tutorial list is the Scripts window.
            Op::OpenMenu(m) => {
                if m.to_ascii_lowercase().contains("tutorial") {
                    cx.dialogs.scripts.show = true;
                } else {
                    self.warn_once(&format!("menu:{m}"), format!("menu \"{m}\" has no counterpart in the port (line {})", cmd.line));
                }
            }
            Op::CloseMenu => {}
            Op::ScreenFade { out, time, color } => cx.dialogs.gui.start_fade(*out, *time, *color),
            Op::PlayMusic { file, crossfade } => {
                let mix = crossfade.map_or(crate::music::MIX_SCRIPT_S, |c| c as f32);
                cx.music.requests.push(MusicRequest::Play { file: file.clone(), mix });
            }
            Op::PauseMusic { crossfade } => cx.music.requests.push(MusicRequest::Pause { mix: crossfade.map_or(crate::music::MIX_SCRIPT_S, |c| c as f32) }),
            Op::ResumeMusic { crossfade } => cx.music.requests.push(MusicRequest::Resume { mix: crossfade.map_or(crate::music::MIX_SCRIPT_S, |c| c as f32) }),
            Op::Get(name) => match self.get_var(cx, name) {
                Some(v) => info!("script {}: {name} = {v}", self.name),
                None => info!("script {}: {name} is not set", self.name),
            },
            Op::Reset(name) => {
                let k = name.to_ascii_lowercase();
                match self.defaults.get(&k).copied() {
                    Some(v) => self.set_var(cx, name, Value::Num(v)),
                    None => {
                        self.vars.remove(&k);
                    }
                }
            }
            Op::TimeScale(x) => cx.sim.rate = *x,
            Op::SetDate(jd) => cx.sim.jd = *jd,
            Op::StopTime => cx.sim.paused = true,
            Op::StartTime => cx.sim.paused = false,
            Op::Wait(s) => {
                if *s > 0.0 {
                    self.blocked = Some(Blocked::Seconds(*s));
                    self.status = format!("Wait {s}");
                }
            }
            Op::BeginMultiTrigger(mode) => self.building = Some((*mode, Vec::new())),
            Op::WaitTrigger(t) => {
                if let Some(name) = &t.object {
                    if find_target(cx, name).is_none() && !t.kind.to_ascii_lowercase().starts_with("gui|") {
                        self.warn_once(
                            &format!("trigobj:{name}"),
                            format!("trigger object \"{name}\" is neither in the catalog nor a waypoint, it times out (line {})", cmd.line),
                        );
                    }
                }
                let snap = snapshot(cx);
                let state = TrigState {
                    trig: (**t).clone(),
                    done: false,
                    moved: DVec3::ZERO,
                    seen_closed: false,
                    seen_open: false,
                    prev_jd: snap.jd,
                    snap,
                };
                match &mut self.building {
                    Some((_, items)) => items.push(state),
                    None => self.block_on_triggers(MultiMode::And, vec![state]),
                }
            }
            Op::EndMultiTrigger => {
                if let Some((mode, items)) = self.building.take() {
                    if !items.is_empty() {
                        self.block_on_triggers(mode, items);
                    }
                }
            }
            Op::ShowMessage(t) => self.caption = Some(Caption { lines: se_script::markup_lines(t), ack: false }),
            Op::WaitMessage(t) => {
                self.caption = Some(Caption { lines: se_script::markup_lines(t), ack: true });
                self.ack = false;
                self.blocked = Some(Blocked::Message { elapsed: 0.0, then_stop: false });
                self.status = "waiting for Ok".into();
            }
            Op::HideMessage => self.caption = None,
            Op::Print { text, time } => self.print = Some((text.clone(), *time)),
            Op::HidePrint => self.print = None,
            Op::Log(t) => info!("script {}: {t}", self.name),
            Op::Set { name, value, .. } => self.set_var(cx, name, value.clone()),
            Op::WaitVar { name, wait } => {
                if !self.var_satisfied(cx, name, wait) {
                    if !self.is_live_var(cx, name) {
                        self.warn_once(&format!("wv:{name}"), format!("cannot observe \"{name}\" in the port, WaitVar times out (line {})", cmd.line));
                    }
                    self.blocked = Some(Blocked::Var { name: name.clone(), wait: wait.clone(), elapsed: 0.0 });
                    self.status = format!("waiting for {name}");
                }
            }
            Op::SaveVars => {
                let snap = self.snapshot_vars(cx);
                self.saved.push(snap);
            }
            Op::RestoreVars => {
                if let Some(vars) = self.saved.pop() {
                    for (k, v) in vars {
                        self.set_var(cx, &k, v);
                    }
                }
            }
            Op::Interpolate { name, to, time } => {
                let from = self.get_var(cx, name).unwrap_or(0.0);
                self.interps.retain(|i| !i.name.eq_ignore_ascii_case(name));
                if *time > 0.0 {
                    self.interps.push(Interp { name: name.clone(), anim: Anim { from, to: *to, elapsed: 0.0, duration: *time } });
                } else {
                    self.set_var(cx, name, Value::Num(*to));
                }
            }
            Op::If { cond, on_false } | Op::Elif { cond, on_false } => {
                let lhs = self.get_var(cx, &cond.var).unwrap_or(0.0);
                if !cond.eval(lhs) {
                    self.jump(*on_false);
                }
            }
            Op::Else | Op::EndIf => {}
            Op::Jump(t) => self.jump(*t),
            Op::Loop { count, end } => {
                if *count == Some(0) {
                    self.jump(*end + 1);
                } else if let Some(fr) = self.frames.last_mut() {
                    fr.loops.push(LoopState { remaining: *count });
                }
            }
            Op::EndLoop { start } => {
                let mut again = false;
                if let Some(fr) = self.frames.last_mut() {
                    if let Some(l) = fr.loops.last_mut() {
                        match &mut l.remaining {
                            Some(n) => {
                                *n = n.saturating_sub(1);
                                again = *n > 0;
                            }
                            None => again = true,
                        }
                    }
                    if !again {
                        fr.loops.pop();
                    }
                }
                if again {
                    self.jump(*start + 1);
                }
            }
            Op::Break { end } => {
                if let Some(fr) = self.frames.last_mut() {
                    fr.loops.pop();
                }
                self.jump(*end + 1);
            }
            Op::Waypoint(w) => {
                let cat = cx.cat();
                if let Err(e) = cx.vars.waypoints.define(w, cat) {
                    self.warn_once(&format!("wp:{}", w.name), format!("{e} (line {})", cmd.line));
                }
            }
            Op::DeleteWaypoint(n) => {
                if !cx.vars.waypoints.delete(n) {
                    self.warn_once(&format!("wpdel:{n}"), format!("DeleteWaypoint: no waypoint \"{n}\" (line {})", cmd.line));
                }
            }
            Op::ClearWaypoints => cx.vars.waypoints.clear(),
            Op::Track => cx.cam.track = cx.cam.selected.is_some(),
            Op::Untrack => cx.cam.track = false,
            // The port has no menu or game modes: the planetarium is all there is.
            Op::Other(item) if matches!(item.key.to_ascii_lowercase().as_str(), "enterplanetarium" | "continuesession") => {}
            Op::Other(item) => {
                let key = item.key.to_ascii_lowercase();
                self.warn_once(&key, format!("command \"{}\" is not supported, skipping (first at line {})", item.key, cmd.line));
            }
        }
    }

    fn block_on_triggers(&mut self, mode: MultiMode, items: Vec<TrigState>) {
        let max_wait = items.iter().filter_map(|i| i.trig.max_wait_time).fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.min(b))));
        let kinds: Vec<String> = items.iter().map(|i| i.trig.kind.clone()).collect();
        for it in &items {
            if trigger_class(&it.trig) == Class::Unsupported {
                let kind = it.trig.kind.clone();
                self.warn_once(
                    &format!("trig:{kind}"),
                    format!("trigger \"{kind}\" has no equivalent in the port, it times out after {UNSUPPORTED_WAIT_S} s (line {})", self.cur_line),
                );
            }
        }
        self.status = format!("waiting for {}", kinds.join(" + "));
        self.blocked = Some(Blocked::Triggers { mode, items, elapsed: 0.0, max_wait });
    }

    /// True when the block has ended.
    fn poll_blocked(&mut self, cx: &mut Cx, dt: f64) -> bool {
        let skip = std::mem::take(&mut self.skip);
        let ack = std::mem::take(&mut self.ack);
        let mut blocked = self.blocked.take().unwrap();
        let done = match &mut blocked {
            Blocked::Seconds(left) => {
                *left -= dt;
                *left <= 0.0 || skip
            }
            Blocked::Message { elapsed, then_stop } => {
                *elapsed += dt;
                if ack || skip || cx.auto.is_some_and(|a| *elapsed >= a * AUTO_MESSAGE_S / AUTO_TRIGGER_S) {
                    if let Some(c) = &mut self.caption {
                        c.ack = false;
                    }
                    if *then_stop {
                        info!("script {} stopped: made for another version", self.name);
                        cx.ended = true;
                    }
                    true
                } else {
                    false
                }
            }
            Blocked::Var { name, wait, elapsed } => {
                *elapsed += dt;
                let done = skip
                    || self.var_satisfied(cx, name, wait)
                    || (!self.is_live_var(cx, name) && *elapsed >= UNSUPPORTED_WAIT_S)
                    || cx.auto.is_some_and(|a| *elapsed >= a);
                if done {
                    cx.dialogs.gui.wait_ended();
                }
                done
            }
            Blocked::Triggers { mode, items, elapsed, max_wait } => {
                *elapsed += dt;
                let el = *elapsed;
                for it in items.iter_mut() {
                    if it.done {
                        continue;
                    }
                    let check = check_trigger(it, cx, el, dt);
                    // ActivationDelay: the trigger counts only after that long.
                    let active = it.trig.activation_delay.is_none_or(|d| el >= d);
                    it.done = active
                        && match check {
                            Check::Yes => true,
                            Check::No => false,
                            Check::Unsupported => el >= UNSUPPORTED_WAIT_S,
                        };
                }
                let ok = match mode {
                    MultiMode::And => items.iter().all(|i| i.done),
                    MultiMode::Or => items.iter().any(|i| i.done),
                };
                let done = ok || skip || max_wait.is_some_and(|m| el >= m) || cx.auto.is_some_and(|a| el >= a);
                if done {
                    cx.dialogs.gui.wait_ended();
                    if cx.auto.is_some() {
                        // Test runs: which waits the port could watch.
                        let kinds: Vec<String> = items
                            .iter()
                            .map(|i| format!("{}{}", i.trig.kind, if trigger_class(&i.trig) == Class::Unsupported { " (unsupported)" } else { "" }))
                            .collect();
                        info!("script wait line {}: {} -> {} after {el:.1} s", self.cur_line, kinds.join(" + "), if ok { "met" } else { "ended" });
                    }
                }
                done
            }
        };
        if !done {
            self.blocked = Some(blocked);
        }
        done
    }

    // ---- variables ----

    fn get_var(&self, cx: &Cx, name: &str) -> Option<f64> {
        cx.vars.get(cx.cam, cx.sim, name).or_else(|| self.vars.get(&name.to_ascii_lowercase()).and_then(Value::as_f64))
    }

    /// Whether `name` is one of the port's settings (so a `WaitVar` on it can be met).
    fn is_live_var(&self, cx: &Cx, name: &str) -> bool {
        cx.vars.get(cx.cam, cx.sim, name).is_some()
    }

    fn set_var(&mut self, cx: &mut Cx, name: &str, value: Value) {
        let k = name.to_ascii_lowercase();
        if let Some(v) = value.as_f64() {
            let before = cx.vars.get(cx.cam, cx.sim, &k);
            if cx.vars.set(cx.cam, cx.sim, &k, v) {
                if let Some(b) = before {
                    self.defaults.entry(k).or_insert(b);
                }
                return;
            }
        }
        let persistent = crate::script_vars::PERSISTENT.contains(&k.as_str());
        self.store_var(name, k.clone(), value);
        if persistent {
            let saved: Vec<(String, f64)> =
                crate::script_vars::PERSISTENT.iter().filter_map(|p| self.vars.get(*p).and_then(Value::as_f64).map(|v| (p.to_string(), v))).collect();
            crate::script_vars::save_persistent(&saved);
        }
    }

    /// A variable the port does not act on, kept for scripts to read back.
    fn store_var(&mut self, name: &str, k: String, value: Value) {
        if !self.vars.contains_key(&k) && !IGNORED_VARS.contains(&k.as_str()) {
            self.unsupported_vars.push(name.to_string());
        }
        self.vars.insert(k, value);
    }

    fn snapshot_vars(&self, cx: &Cx) -> HashMap<String, Value> {
        let mut m = self.vars.clone();
        for (k, v) in cx.vars.snapshot(cx.cam, cx.sim) {
            m.insert(k, Value::Num(v));
        }
        m
    }

    // ---- input locks ----

    /// `UserMoveControl` / `UserRotationControl` / `UserTimeControl` / `UserStereobaseControl`.
    fn user_control(&mut self, cx: &mut Cx, kind: InputKind, mode: LockMode, limits: &ControlLimits) {
        match kind {
            InputKind::Move => {
                cx.cam.locks.movement = mode == LockMode::Disabled;
                if mode == LockMode::Disabled {
                    cx.cam.velocity = DVec3::ZERO;
                    cx.cam.travel = None;
                }
                self.move_limit = (mode == LockMode::Limited).then(|| MoveLimit {
                    center: limits.center.as_deref().and_then(|c| resolve_object(cx.cat(), c)),
                    limits: limits.clone(),
                });
            }
            InputKind::Rotation => {
                cx.cam.locks.rotation = mode == LockMode::Disabled;
                if mode == LockMode::Disabled {
                    cx.cam.turn_vel = DVec3::ZERO;
                }
            }
            InputKind::Time => {
                self.time_guard = (mode != LockMode::Free).then(|| TimeGuard {
                    locked: mode == LockMode::Disabled,
                    rate: cx.sim.rate,
                    paused: cx.sim.paused,
                    limits: limits.clone(),
                });
                // "Limited" puts the rate inside its range at once (FUN_140552330).
                if let (LockMode::Limited, Some((a, b))) = (mode, limits.rate_range) {
                    cx.sim.rate = cx.sim.rate.clamp(a, b);
                }
            }
            InputKind::Stereobase => {
                self.warn_once("stereobase", "UserStereobaseControl: the port has no stereoscopic view, ignored".into());
            }
        }
    }

    /// Undoes what the user changed against the script's locks.
    fn guard_user_input(&mut self, cx: &mut Cx, _dt: f64) {
        if let Some(g) = &self.time_guard {
            if g.locked {
                cx.sim.rate = g.rate;
                cx.sim.paused = g.paused;
            } else {
                if let Some((a, b)) = g.limits.rate_range {
                    cx.sim.rate = cx.sim.rate.clamp(a, b);
                }
                if let Some((a, b)) = g.limits.time_range {
                    cx.sim.jd = cx.sim.jd.clamp(a, b);
                }
            }
        }
        if let Some(m) = &self.move_limit {
            if let Some(kms) = m.limits.speed_kms {
                cx.cam.speed = cx.cam.speed.min(clamp_speed(kms * 1000.0));
            }
            if let Some(c) = m.center {
                let eph = Ephemeris::new(cx.cat());
                let p = eph.position(c, cx.sim.jd);
                let max = m.limits.dist_km.map(|k| k * 1000.0).or(m.limits.dist_rad.map(|r| r * eph.radius_m(c)));
                let off = cx.cam.pos.delta_m(p);
                if let Some(max) = max.filter(|m| off.length() > *m) {
                    cx.cam.pos = p.offset_m(off.normalize_or(DVec3::Z) * max);
                    cx.cam.velocity = DVec3::ZERO;
                }
            }
        }
    }

    fn var_satisfied(&self, cx: &Cx, name: &str, wait: &VarWait) -> bool {
        let Some(v) = self.get_var(cx, name) else { return false };
        match wait {
            VarWait::Range(a, b) => v >= *a && v <= *b,
            VarWait::Equals(x) => match x.as_f64() {
                Some(t) => (v - t).abs() < 1e-6,
                None => false,
            },
        }
    }

    // ---- continuous effects ----

    fn update_effects(&mut self, cx: &mut Cx, dt: f64) {
        if let Some((_, t)) = &mut self.print {
            if let Some(left) = t {
                *left -= dt;
                if *left <= 0.0 {
                    self.print = None;
                }
            }
        }
        if let Some(a) = &mut self.fov_anim {
            a.elapsed += dt;
            let s = smooth((a.elapsed / a.duration).clamp(0.0, 1.0));
            cx.cam.fov_deg = (a.from + (a.to - a.from) * s) as f32;
            if a.elapsed >= a.duration {
                self.fov_anim = None;
            }
        }
        let mut finished = Vec::new();
        let mut sets = Vec::new();
        for (i, it) in self.interps.iter_mut().enumerate() {
            it.anim.elapsed += dt;
            let t = (it.anim.elapsed / it.anim.duration).clamp(0.0, 1.0);
            sets.push((it.name.clone(), it.anim.from + (it.anim.to - it.anim.from) * t));
            if t >= 1.0 {
                finished.push(i);
            }
        }
        for (n, v) in sets {
            self.set_var(cx, &n, Value::Num(v));
        }
        for i in finished.into_iter().rev() {
            self.interps.remove(i);
        }
        self.update_flight(cx, dt);
        if self.flight.is_none() {
            self.update_orbit(cx, dt);
        }
        // `Fly`: along a camera axis; the follow binding carries the camera on with its body.
        if let Some(f) = &mut self.fly {
            let alive = f.step(dt);
            cx.cam.pos = cx.cam.pos.offset_m(cx.cam.rot * f.axis * (f.cur * dt));
            if !alive {
                self.fly = None;
            }
        }
        // `Turn`: about a camera axis, in degrees per second.
        if let Some(t) = &mut self.turn {
            let alive = t.step(dt);
            cx.cam.rot = (cx.cam.rot * DQuat::from_axis_angle(t.axis, t.cur.to_radians() * dt)).normalize();
            if !alive {
                self.turn = None;
            }
        }
    }

    fn update_orbit(&mut self, cx: &mut Cx, dt: f64) {
        let (Some((axis_cam, speed)), Some(sel)) = (self.orbit, cx.cam.selected) else { return };
        let eph = Ephemeris::new(cx.cat());
        let center = match cx.cam.follow {
            Some(f) if f.target == sel => f.last_pos,
            _ => eph.position(sel, cx.sim.jd),
        };
        let axis = (cx.cam.rot * axis_cam).normalize_or(DVec3::Y);
        let q = DQuat::from_axis_angle(axis, speed.to_radians() * dt);
        let off = cx.cam.pos.delta_m(center);
        cx.cam.pos = center.offset_m(q * off);
        cx.cam.rot = (q * cx.cam.rot).normalize();
    }

    // ---- camera commands ----

    fn goto_location(&mut self, cx: &mut Cx, loc: &Location) {
        let jd = loc.jd.unwrap_or(cx.sim.jd);
        cx.sim.jd = jd;
        let cat = &cx.data.catalog;
        let eph = Ephemeris::new(cat);
        // Positions are relative to the Sun's centre.
        let sun = sun_position(cat, jd);
        cx.cam.pos = sun.offset_m(DVec3::from_array(loc.pos_m));
        cx.cam.rot = DQuat::from_xyzw(loc.rot[0], loc.rot[1], loc.rot[2], loc.rot[3]).normalize();
        cx.cam.velocity = DVec3::ZERO;
        cx.cam.travel = None;
        cx.cam.follow = None;
        self.flight = None;
        // Saved assigned velocity, in parsecs per second.
        if loc.vel > 0.0 {
            cx.cam.speed = clamp_speed(loc.vel * se_core::units::METERS_PER_PC);
        }
        let body = resolve_object(cat, &loc.body);
        if body.is_none() {
            let key = format!("loc:{}", loc.body);
            self.warn_once(&key, format!("GotoLocation body \"{}\" is not in the catalog", loc.body));
        }
        cx.cam.selected = body;
        if let Some(b) = body {
            let bp = eph.position(b, jd);
            let radius = eph.radius_m(b).max(1.0);
            let d = bp.delta_m(cx.cam.pos).length();
            // Far-away systems (the main-menu scene is ~22 kpc out) are placed in the
            // catalog from rounded RA/Dec/distance, so their position is off by far more
            // than the camera's distance from the body in the saved location, and the
            // saved camera offset is lost. Re-anchor: frame the body together with its
            // parent (the menu moon and its ringed planet) from above the orbital plane,
            // or just the body from a few radii when it has no parent.
            if bp.to_meters().length() > 100.0 * PC_M && d > 1.0e4 * radius.max(1.0e5) {
                let parent = resolve_object(cat, &loc.parent).filter(|p| *p != b);
                if let Some(p) = parent {
                    let sep = eph.position(p, jd).delta_m(bp);
                    let mid = bp.offset_m(sep * 0.5);
                    let toward = sep.normalize_or(DVec3::X);
                    let pole = (eph.orientation(p, jd) * DVec3::Y).normalize_or(DVec3::Y);
                    let side = toward.cross(pole).normalize_or(DVec3::Z);
                    let dir = (pole * 0.85 + side * 0.3 - toward * 0.1).normalize();
                    cx.cam.pos = mid.offset_m(dir * sep.length() * 1.1);
                    cx.cam.rot = look_rotation(-dir, pole);
                } else {
                    let fwd = cx.cam.rot * DVec3::NEG_Z;
                    cx.cam.pos = bp.offset_m(-fwd * (radius * 3.0));
                }
                info!(
                    "script GotoLocation: catalog position of \"{}\" is {:.3e} m from the saved camera; re-anchored the camera on it",
                    loc.body, d
                );
            }
        }
        if let (Some(b), mode @ 1..) = (body, loc.bind_mode) {
            cx.cam.follow = Some(Follow::new(b, mode >= 2, &eph, jd));
        }
    }

    fn start_goto(&mut self, cx: &mut Cx, a: &GotoArgs, line: u32) {
        if let Some(name) = &a.object {
            match resolve_object(cx.cat(), name) {
                Some(r) => cx.cam.selected = Some(r),
                None => self.warn_once(&format!("sel:{name}"), format!("Goto object not found: \"{name}\" (line {line})")),
            }
        }
        let Some(target) = cx.cam.selected else {
            self.warn_once("goto-none", format!("Goto with nothing selected, skipping (line {line})"));
            return;
        };
        let jd = cx.sim.jd;
        let eph = Ephemeris::new(cx.cat());
        let radius = eph.radius_m(target).max(1.0);
        let q = eph.orientation(target, jd);
        let center = eph.position(target, jd);
        let start_local = q.inverse() * cx.cam.pos.delta_m(center);
        let start_rot = q.inverse() * cx.cam.rot;
        // Direction of the destination from the body centre, in the body frame.
        let dir = match (a.lat, a.lon) {
            (None, None) => start_local.normalize_or(DVec3::Z),
            (lat, lon) => {
                let cur = start_local.normalize_or(DVec3::Z);
                let cur_lat = cur.y.clamp(-1.0, 1.0).asin();
                let cur_lon = (-cur.z).atan2(cur.x);
                let (lat, lon) = (lat.map_or(cur_lat, f64::to_radians), lon.map_or(cur_lon, f64::to_radians));
                DVec3::new(lat.cos() * lon.cos(), lat.sin(), -lat.cos() * lon.sin())
            }
        };
        let dist = match (a.height_km, a.dist_km, a.dist_rad) {
            (Some(h), _, _) => radius + h * 1000.0,
            (_, Some(d), _) => d * 1000.0,
            (_, _, Some(r)) => r * radius,
            _ => start_local.length(),
        }
        .max(radius * 1.0001);
        let end_local = dir * dist;
        let oriented = a.yaw.is_some() || a.pitch.is_some() || a.roll.is_some();
        // FUN_140563f30: "Camera rotation is limited, ignoring Center option in Goto command".
        let center = a.center && !cx.cam.locks.rotation;
        if a.center && cx.cam.locks.rotation {
            warn!("script {}, line {line}: camera rotation is limited, ignoring Center option in Goto command", self.name);
        }
        let end_rot = if cx.cam.locks.rotation && !oriented {
            start_rot
        } else if oriented && !center {
            horizon_rotation(dir, a.yaw.unwrap_or(0.0), a.pitch.unwrap_or(0.0), a.roll.unwrap_or(0.0))
        } else {
            look_rotation(-end_local, DVec3::Y)
        };
        cx.cam.travel = None;
        cx.cam.follow = None;
        cx.cam.velocity = DVec3::ZERO;
        let profile = Profile::from_args(a);
        let duration = a.time.or(profile.map(|p| p.total())).unwrap_or(DEFAULT_GOTO_S).max(0.0);
        // SE binds a Goto to a point on the surface with SyncRot, any other with Follow (FUN_140563f30).
        let sync = a.lat.is_some() || a.lon.is_some();
        self.flight = Some(Flight { target, start_local, start_rot, end_local, end_rot, elapsed: 0.0, duration, sync, profile });
        // Zero-time flights land immediately, so following commands see the result.
        self.update_flight(cx, 0.0);
    }

    fn update_flight(&mut self, cx: &mut Cx, dt: f64) {
        let Some(f) = &mut self.flight else { return };
        f.elapsed += dt;
        let t = if f.duration <= 0.0 { 1.0 } else { (f.elapsed / f.duration).clamp(0.0, 1.0) };
        let s = match f.profile {
            Some(p) => p.progress(t),
            None => smooth(t),
        };
        let jd = cx.sim.jd;
        let eph = Ephemeris::new(&cx.data.catalog);
        let q = eph.orientation(f.target, jd);
        let center = eph.position(f.target, jd);
        let (r0, r1) = (f.start_local.length().max(1.0), f.end_local.length().max(1.0));
        let r = (r0.ln() + (r1.ln() - r0.ln()) * s).exp();
        let (d0, d1) = (f.start_local.normalize_or(DVec3::Z), f.end_local.normalize_or(DVec3::Z));
        let arc = DQuat::from_rotation_arc(d0, d1);
        let dir = DQuat::IDENTITY.slerp(arc, s) * d0;
        cx.cam.pos = center.offset_m(q * (dir * r));
        cx.cam.rot = (q * f.start_rot.slerp(f.end_rot, s)).normalize();
        if t >= 1.0 {
            cx.cam.follow = Some(Follow {
                target: f.target,
                last_pos: center,
                last_rot: if f.sync { q } else { DQuat::IDENTITY },
                locked: f.sync,
                sync: f.sync,
            });
            self.flight = None;
        }
    }
}

/// The Sun's centre in the universe frame. Planet positions come from JPL DE436 relative
/// to the Solar System barycentre, which sits up to ~1.5e9 m away from the Sun's centre.
fn sun_position(cat: &Catalog, jd: f64) -> UPos {
    if let Some(km) = cat.de.as_ref().and_then(|de| de.position_for_model("Sun-DE", jd)) {
        return UPos::from_meters(se_core::coords::eq_to_universe(km * 1000.0));
    }
    cat.find("Sun").map(|s| Ephemeris::new(cat).position(s, jd)).unwrap_or(UPos::ZERO)
}

/// Variables that only affect SpaceEngine's GUI feedback, or the saved tutorial progress;
/// setting them is not worth a report.
const IGNORED_VARS: &[&str] = &["silentvarchange", "lessonpassed"];

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn snapshot(cx: &Cx) -> Snapshot {
    Snapshot {
        selected_wp: cx.wps().selected.clone(),
        track: cx.cam.track,
        rot: cx.cam.rot,
        fov: cx.cam.fov_deg,
        speed: cx.cam.speed,
        rate: cx.sim.rate,
        paused: cx.sim.paused,
        jd: cx.sim.jd,
        selected: cx.cam.selected,
        travel: cx.cam.travel.is_some(),
        following: cx.cam.follow.is_some(),
        rotating: cx.cam.bind_mode() == 2,
    }
}

/// Camera orientation for a spot on a body: `up` is the local vertical (body frame),
/// yaw is the heading from north towards east, pitch the elevation (-90 = straight
/// down) and roll the bank, all in degrees.
fn horizon_rotation(up: DVec3, yaw: f64, pitch: f64, roll: f64) -> DQuat {
    let north = (DVec3::Y - up * up.y).normalize_or(DVec3::X);
    let east = north.cross(up);
    let (yaw, pitch, roll) = (yaw.to_radians(), pitch.to_radians(), roll.to_radians());
    let heading = north * yaw.cos() + east * yaw.sin();
    let fwd = (heading * pitch.cos() + up * pitch.sin()).normalize();
    let cam_up = up * pitch.cos() - heading * pitch.sin();
    let right = fwd.cross(cam_up).normalize();
    let cam_up = right.cross(fwd);
    // Roll about the view axis.
    let r = DQuat::from_axis_angle(fwd, -roll);
    let (right, cam_up) = (r * right, r * cam_up);
    DQuat::from_mat3(&bevy::math::DMat3::from_cols(right, cam_up, -fwd)).normalize()
}

/// SpaceEngine object paths look like `Earth-Moon|Earth`; resolve by the full name,
/// then by the last path component, then earlier ones.
pub(crate) fn resolve_object(cat: &Catalog, name: &str) -> Option<ObjectRef> {
    if let Some(r) = cat.find(name) {
        return Some(r);
    }
    name.rsplit('|').find_map(|part| cat.find(part.trim()))
}

/// What a script names: a catalog object or one of its waypoints.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Object(ObjectRef),
    /// Position and radius (m).
    Waypoint(UPos, f64),
}

/// Waypoints first (scripts name them explicitly), then the catalog.
fn find_target(cx: &Cx, name: &str) -> Option<Target> {
    if let Some(w) = cx.wps().find(name) {
        let (p, r) = Waypoints::place(w, &Ephemeris::new(cx.cat()), cx.sim.jd);
        return Some(Target::Waypoint(p, r));
    }
    resolve_object(cx.cat(), name).map(Target::Object)
}

impl Target {
    /// Position, radius (m) and the body's orientation (for latitudes).
    fn place(self, eph: &Ephemeris, jd: f64) -> (UPos, f64, DQuat) {
        match self {
            Target::Object(r) => (eph.position(r, jd), eph.radius_m(r).max(1.0), eph.orientation(r, jd)),
            Target::Waypoint(p, r) => (p, r, DQuat::IDENTITY),
        }
    }
}

/// Where the selection is: the catalog object or the selected waypoint.
fn selection_pos(cx: &Cx) -> Option<UPos> {
    let eph = Ephemeris::new(cx.cat());
    match (cx.cam.selected, cx.wps().selected()) {
        (Some(s), _) => Some(eph.position(s, cx.sim.jd)),
        (None, Some(w)) => Some(Waypoints::place(w, &eph, cx.sim.jd).0),
        _ => None,
    }
}

// ---- triggers ----------------------------------------------------------------------

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Class {
    Supported,
    Unsupported,
}

enum Check {
    Yes,
    No,
    Unsupported,
}

/// The trigger kinds the port can watch (exe strings `Control|Rotate` .. `Input|Custom`). The
/// universe map and system chart controls, panel pins, clicks inside dialogs and input
/// bindings have no counterpart and time out; of the clicks, the navigation toolbar's date is
/// the port's too.
fn trigger_class(t: &Trigger) -> Class {
    let date_click = t.dialog.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(DATE_WIDGET));
    match t.kind.to_ascii_lowercase().as_str() {
        "gui|leftclick" if date_click => Class::Supported,
        "loadingcomplete" | "screenshotcomplete" | "entermenu" | "enterplanetarium" | "entersingleplayer" | "time"
        | "object|select" | "object|approach" | "object|see" | "control|goto" | "control|gotocenter" | "control|move"
        | "control|rotate" | "control|orbit" | "control|changespeed" | "control|settimerate" | "control|pausetime"
        | "control|resumetime" | "control|reversetime" | "control|settime" | "control|setcurrenttime"
        | "control|changefov" | "control|resetfov" | "control|center" | "control|follow" | "control|syncrot"
        | "control|free" | "control|track" | "control|untrack" | "control|horizon" | "control|land"
        | "control|stopmove" | "control|stoprotate" | "gui|open" | "gui|close"
        | "gui|selectobject" => Class::Supported,
        _ => Class::Unsupported,
    }
}

/// `Control|Move`'s direction test (FUN_140275dd0): the movement direction `n` (SE camera axes)
/// matches `d` when |d_i - n_i| <= 0.01 |d_i + n_i| on every axis (plus rounding).
fn direction_matches(d: [f64; 3], n: DVec3) -> bool {
    let n = n.to_array();
    (0..3).all(|i| (d[i] - n[i]).abs() <= DIRECTION_TOLERANCE * (d[i] + n[i]).abs() + 1e-6)
}

/// Camera longitude and latitude over a body (degrees, the port's body frame as `Goto`'s).
fn lon_lat(local: DVec3) -> (f64, f64) {
    let d = local.normalize_or(DVec3::Z);
    ((-d.z).atan2(d.x).to_degrees(), d.y.clamp(-1.0, 1.0).asin().to_degrees())
}

fn in_range(v: f64, r: (f64, f64)) -> bool {
    v >= r.0.min(r.1) && v <= r.0.max(r.1)
}

/// Whether an object is of a trigger's `Type`. SE also filters by class (`"Planet|Temperate|Terra"`);
/// the port checks the first part only.
fn type_matches(cat: &Catalog, r: ObjectRef, ty: &str) -> bool {
    let base = ty.split('|').next().unwrap_or(ty).trim().to_ascii_lowercase();
    match (r, base.as_str()) {
        (_, "selection") => true,
        (ObjectRef::Body(i), t) => {
            let tag = cat.bodies[i as usize].tag;
            matches!(
                (tag, t),
                (BodyTag::Planet, "planet")
                    | (BodyTag::DwarfPlanet, "dwarfplanet")
                    | (BodyTag::Moon, "moon")
                    | (BodyTag::DwarfMoon, "dwarfmoon")
                    | (BodyTag::Asteroid, "asteroid")
                    | (BodyTag::Comet, "comet")
                    | (BodyTag::Spacecraft, "spacecraft")
            )
        }
        (ObjectRef::Star(_), "star" | "sun") => true,
        (ObjectRef::Galaxy(_), "galaxy") => true,
        (ObjectRef::Cluster(_), "cluster") => true,
        (ObjectRef::Nebula(_), "nebula") => true,
        _ => false,
    }
}

/// Yaw/pitch/roll (degrees) of the camera relative to its orientation at `snap`.
fn rotation_since(snap: &Snapshot, rot: DQuat) -> (f64, f64, f64) {
    let rel = snap.rot.inverse() * rot;
    let (y, p, r) = rel.to_euler(EulerRot::YXZ);
    (y.to_degrees(), p.to_degrees(), r.to_degrees())
}

/// True when `v` has left the symmetric-ish range `r` (rotation triggers fire once the
/// turn exceeds the given limits).
fn outside(v: f64, r: (f64, f64)) -> bool {
    v < r.0.min(r.1) || v > r.0.max(r.1)
}

fn check_trigger(it: &mut TrigState, cx: &Cx, elapsed: f64, dt: f64) -> Check {
    let yes = |b: bool| if b { Check::Yes } else { Check::No };
    let cat = cx.cat();
    let eph = Ephemeris::new(cat);
    let jd = cx.sim.jd;
    let cam = &*cx.cam;
    let (t, snap) = (&it.trig, &it.snap);
    let prev_jd = std::mem::replace(&mut it.prev_jd, jd);
    // The user's own movement this frame (velocity, so a followed body's motion does not count).
    it.moved += cam.velocity * dt;
    match t.kind.to_ascii_lowercase().as_str() {
        "loadingcomplete" => yes(elapsed >= LOADING_SETTLE_S),
        "screenshotcomplete" | "entermenu" | "enterplanetarium" | "entersingleplayer" => Check::Yes,
        // FUN_140276ca0: Mode CROSSED / EARLIER / LATER against `Time`, INRANGE / OUTRANGE
        // against `TimeRange`.
        "time" => {
            let mode = t.mode.as_deref().unwrap_or("later").to_ascii_lowercase();
            match (mode.as_str(), t.time_range, t.time_jd) {
                ("inrange", Some(r), _) => yes(in_range(jd, r)),
                ("outrange", Some(r), _) => yes(!in_range(jd, r)),
                (_, _, Some(target)) => yes(match mode.as_str() {
                    "earlier" => jd <= target,
                    "crossed" => (snap.jd < target) != (jd < target),
                    _ => jd >= target,
                }),
                _ => Check::Yes,
            }
        }
        "object|select" => {
            let changed = snap.selected != cam.selected || snap.selected_wp != cx.wps().selected;
            match (&t.object, &t.ty) {
                (Some(name), _) => match find_target(cx, name) {
                    Some(Target::Object(want)) => yes(cam.selected == Some(want) && changed),
                    Some(Target::Waypoint(..)) => {
                        let sel = cx.wps().selected().map(|w| w.name.as_str());
                        yes(changed && sel.is_some_and(|s| cx.wps().find(name).is_some_and(|w| w.name == s)))
                    }
                    None => Check::Unsupported,
                },
                (None, Some(ty)) => yes(changed && cam.selected.is_some_and(|s| type_matches(cat, s, ty))),
                (None, None) => yes(changed && (cam.selected.is_some() || cx.wps().selected.is_some())),
            }
        }
        "object|approach" => {
            let target = match (&t.object, &t.ty) {
                (Some(name), _) => match find_target(cx, name) {
                    Some(r) => Some(r),
                    None => return Check::Unsupported,
                },
                (None, Some(ty)) if ty.eq_ignore_ascii_case("selection") => cam.selected.map(Target::Object),
                (None, Some(ty)) => {
                    let near = cat.find(&cam.nearest_name);
                    near.filter(|r| type_matches(cat, *r, ty)).map(Target::Object)
                }
                (None, None) => cam.selected.map(Target::Object),
            };
            let Some(target) = target else { return Check::No };
            let (pos, radius, q) = target.place(&eph, jd);
            let off = cam.pos.delta_m(pos);
            let d = off.length();
            // LonLatRange (lon0, lon1, lat0, lat1): where over the body the camera is.
            if let Some([lo0, lo1, la0, la1]) = t.lonlat_range {
                let (lon, lat) = lon_lat(q.inverse() * off);
                if !in_range(lon, (lo0, lo1)) || !in_range(lat, (la0, la1)) {
                    return Check::No;
                }
            }
            if let Some(r) = t.range_rad {
                return yes(in_range(d / radius, r));
            }
            if let Some(r) = t.range_km {
                return yes(in_range((d - radius).max(0.0) / 1000.0, r));
            }
            if let Some(r) = t.range {
                // parsecs
                return yes(in_range(d / PC_M, r));
            }
            yes(d / radius < 10.0)
        }
        "object|see" => {
            let target = match &t.object {
                Some(n) => match find_target(cx, n) {
                    Some(r) => Some(r),
                    None => return Check::Unsupported,
                },
                None => cam.selected.map(Target::Object),
            };
            let Some(target) = target else { return Check::No };
            let local = cam.rot.inverse() * target.place(&eph, jd).0.delta_m(cam.pos);
            if local.z >= 0.0 {
                return Check::No;
            }
            let tan_half = ((cam.fov_deg as f64).to_radians() * 0.5).tan();
            let x = (local.x / -local.z / (tan_half * cx.aspect) + 1.0) * 0.5;
            let y = (1.0 - local.y / -local.z / tan_half) * 0.5;
            match t.rect {
                Some([x0, x1, y0, y1]) => yes(in_range(x, (x0, x1)) && in_range(y, (y0, y1))),
                None => yes((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)),
            }
        }
        // A Go to started (to `Object`, or to an object of `Type`); GotoCenter is Ctrl+G's.
        "control|goto" | "control|gotocenter" => {
            let Some(tr) = cam.travel.as_ref().filter(|tr| tr.kind == TravelKind::Goto && !snap.travel) else { return Check::No };
            let center_ok = !t.kind.eq_ignore_ascii_case("control|gotocenter") || (tr.end_distance > 0.0 && tr.end_distance <= 10.0);
            let object_ok = match &t.object {
                Some(n) => resolve_object(cat, n) == Some(tr.target),
                None => true,
            };
            let type_ok = t.ty.as_ref().is_none_or(|ty| type_matches(cat, tr.target, ty));
            yes(center_ok && object_ok && type_ok)
        }
        // FUN_140275dd0: the user's movement summed since the start, within RangeKm (or Range in
        // parsecs), while moving along Direction (SE's camera axes: x left, y down, z ahead).
        "control|move" => {
            let v = cam.velocity;
            if v.length_squared() == 0.0 {
                return Check::No;
            }
            if let Some(d) = t.direction {
                let n = -(cam.rot.inverse() * v).normalize();
                if !direction_matches(d, n) {
                    return Check::No;
                }
            }
            let d_m = it.moved.length();
            match (t.range_km, t.range) {
                (Some(r), _) => yes(d_m / 1000.0 >= r.0.min(r.1) && d_m / 1000.0 < r.0.max(r.1)),
                (None, Some(r)) => yes(d_m / PC_M >= r.0.min(r.1) && d_m / PC_M < r.0.max(r.1)),
                (None, None) => Check::Yes,
            }
        }
        "control|rotate" | "control|orbit" => {
            let (y, p, r) = rotation_since(snap, cam.rot);
            let mut tests = Vec::new();
            if let Some(rg) = t.yaw_range {
                tests.push(outside(y, rg));
            }
            if let Some(rg) = t.pitch_range {
                tests.push(outside(p, rg));
            }
            if let Some(rg) = t.roll_range {
                tests.push(outside(r, rg));
            }
            if tests.is_empty() {
                tests.push(y.abs() > 5.0 || p.abs() > 5.0);
            }
            let or = t.mode.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("or"));
            yes(if or { tests.iter().any(|b| *b) } else { tests.iter().all(|b| *b) })
        }
        "control|changespeed" => {
            let kms = cam.speed / 1000.0;
            if let Some(r) = t.range_km {
                yes(in_range(kms, r))
            } else if let Some(r) = t.range {
                // parsecs per second
                yes(in_range(kms * 1000.0 / se_core::units::METERS_PER_PC, r))
            } else {
                yes(cam.speed != snap.speed)
            }
        }
        "control|settimerate" => match t.range {
            Some(r) => yes(in_range(cx.sim.rate, r)),
            None => yes(cx.sim.rate != snap.rate),
        },
        "control|pausetime" => yes(cx.sim.paused && !snap.paused),
        "control|resumetime" => yes(!cx.sim.paused && snap.paused),
        "control|reversetime" => yes(cx.sim.rate * snap.rate < 0.0),
        // The date set by hand: a jump the clock's own step does not explain.
        "control|settime" => {
            let step = if cx.sim.paused { 0.0 } else { cx.sim.rate * dt / 86400.0 };
            yes(((jd - prev_jd) - step).abs() > SET_TIME_JUMP_D)
        }
        // Ctrl+\: the date jumps to the system clock's.
        "control|setcurrenttime" => {
            let now = SimTime::default().jd;
            yes((jd - now).abs() < SET_TIME_JUMP_D && (snap.jd - now).abs() >= SET_TIME_JUMP_D)
        }
        "control|changefov" => match t.range {
            Some(r) => yes(in_range(cam.fov_deg as f64, r)),
            None => yes((cam.fov_deg - snap.fov).abs() > 0.5),
        },
        // Home: back to the default field of view.
        "control|resetfov" => {
            let def = crate::camera::DEFAULT_FOV_DEG;
            yes((cam.fov_target == Some(def) || (cam.fov_deg - def).abs() < 0.01) && (snap.fov - def).abs() >= 0.01)
        }
        "control|center" => {
            if cam.travel.as_ref().is_some_and(|tr| tr.kind == TravelKind::Center) && !snap.travel {
                return Check::Yes;
            }
            let Some(sel) = cam.selected else { return Check::No };
            let to = eph.position(sel, jd).delta_m(cam.pos);
            let fwd = cam.rot * DVec3::NEG_Z;
            yes(fwd.angle_between(to) < 3f64.to_radians())
        }
        "control|horizon" => yes(cam.travel.as_ref().is_some_and(|tr| tr.kind == TravelKind::Horizon) && !snap.travel),
        "control|follow" => yes(cam.follow.is_some() && !snap.following),
        "control|syncrot" => yes(cam.bind_mode() == 2 && !snap.rotating),
        "control|free" => yes(cam.follow.is_none() && snap.following),
        "control|track" => yes(cam.track && !snap.track),
        "control|untrack" => yes(!cam.track && snap.track),
        // Land on object (Shift+G) started, or the camera is down.
        "control|land" => {
            let landing = cam.travel.as_ref().filter(|tr| tr.kind == TravelKind::Land);
            let object_ok = |r: ObjectRef| t.object.as_ref().is_none_or(|n| resolve_object(cat, n) == Some(r));
            yes(landing.is_some_and(|tr| object_ok(tr.target)) || cam.nearest_surface < 100.0)
        }
        "control|stopmove" => yes(cam.velocity.length() < 1e-3 && cam.travel.is_none()),
        "control|stoprotate" => yes(cam.turn_vel == DVec3::ZERO),
        // The port's windows by SE's names: opened (or closed) since the trigger started.
        "gui|open" | "gui|close" => {
            let Some(open) = t.dialog.as_deref().and_then(|d| cx.dialogs.is_open(d)) else { return Check::Unsupported };
            it.seen_closed |= !open;
            it.seen_open |= open;
            if t.kind.eq_ignore_ascii_case("gui|open") { yes(open && it.seen_closed) } else { yes(!open && it.seen_open) }
        }
        // A click on the navigation toolbar's date opens its field.
        "gui|leftclick" if t.dialog.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(DATE_WIDGET)) => {
            yes(cx.dialogs.hud.date_edit.is_some())
        }
        // An object picked in the dialog: the selection changed while it is open.
        "gui|selectobject" => {
            let Some(open) = t.dialog.as_deref().and_then(|d| cx.dialogs.is_open(d)) else { return Check::Unsupported };
            let changed = cam.selected.is_some() && cam.selected != snap.selected;
            let object_ok = t.object.as_ref().is_none_or(|n| resolve_object(cat, n) == cam.selected);
            yes(open && changed && object_ok)
        }
        _ => Check::Unsupported,
    }
}

// ---- UI ----------------------------------------------------------------------------

fn draw_captions(
    mut contexts: EguiContexts,
    mut runner: ResMut<ScriptRunner>,
    i18n: Res<I18n>,
    gui: Res<ScriptGui>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if let Some(cap) = runner.caption.clone() {
        let blocked_on_trigger = matches!(runner.blocked, Some(Blocked::Triggers { .. } | Blocked::Var { .. }));
        let mut ack = false;
        let mut skip = false;
        // SetWidgetStyle "Message box" { Pos (x, y) }: the box's top-left corner at that fraction
        // of the screen (PosPix: pixels); else bottom centre.
        let screen = ctx.content_rect();
        let placed = gui
            .message_pos
            .map(|[x, y]| egui::pos2(screen.width() * x as f32, screen.height() * y as f32))
            .or(gui.message_pos_pix.map(|[x, y]| egui::pos2(x as f32, y as f32)));
        let max_width = match placed {
            // Inside the screen, with the frame's margins.
            Some(p) => (screen.right() - p.x - 48.0).clamp(240.0, 680.0),
            None => 680.0,
        };
        let area = egui::Area::new(egui::Id::new("script_caption"));
        let area = match placed {
            Some(p) => area.fixed_pos(p),
            None => area.anchor(egui::Align2::CENTER_BOTTOM, [0.0, -36.0]),
        };
        let shown = area.show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_black_alpha(205))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 110, 150)))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_max_width(max_width);
                        for line in &cap.lines {
                            let text = egui::RichText::new(&line.text).color(egui::Color32::from_gray(235));
                            ui.label(if line.big { text.size(26.0).strong() } else { text.size(15.0) });
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if cap.ack && ui.button(i18n.tr("Ok")).clicked() {
                                ack = true;
                            }
                            if blocked_on_trigger && ui.small_button("Skip step").clicked() {
                                skip = true;
                            }
                        });
                    });
            });
        crate::script_gui::mark_named(ctx, "Message box", shown.response.rect);
        if cap.ack && !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            ack = true;
        }
        runner.ack |= ack;
        runner.skip |= skip;
    }
    if let Some((text, _)) = runner.print.clone() {
        egui::Area::new(egui::Id::new("script_print"))
            .anchor(egui::Align2::CENTER_TOP, [0.0, 70.0])
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(text).size(20.0).color(egui::Color32::WHITE).background_color(egui::Color32::from_black_alpha(150)));
            });
    }
    Ok(())
}

fn scan_scripts(data: &SeData) -> Vec<(String, Vec<String>)> {
    let mut groups = Vec::new();
    for (title, dir) in [("Menu", "scripts/menu"), ("Tutorials", "scripts/tutorials"), ("Other", "scripts")] {
        let mut files = data.vfs.list_ext(dir, "se");
        files.sort();
        if !files.is_empty() {
            groups.push((title.to_string(), files));
        }
    }
    groups
}

fn draw_scripts_window(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<ScriptUi>,
    mut runner: ResMut<ScriptRunner>,
    data: Res<SeData>,
    hud: Res<HudState>,
    i18n: Res<I18n>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !ui_state.show || hud.hidden {
        return Ok(());
    }
    if ui_state.entries.is_none() {
        ui_state.entries = Some(scan_scripts(&data));
    }
    let mut open = true;
    let mut start: Option<String> = None;
    let mut stop = false;
    let mut skip = false;
    let entries = ui_state.entries.clone().unwrap_or_default();
    egui::Window::new(crate::skin::title(ctx, &i18n.tr("Scripts"))).open(&mut open).default_width(300.0).default_pos([320.0, 40.0]).show(ctx, |ui| {
        if runner.running() {
            ui.horizontal(|ui| {
                ui.strong(short_name(&runner.name));
                if ui.button(i18n.tr("Stop")).clicked() {
                    stop = true;
                }
                if ui.button(i18n.tr("Skip step")).clicked() {
                    skip = true;
                }
            });
            ui.label(format!("line {}: {}", runner.cur_line, if runner.status.is_empty() { "running" } else { &runner.status }));
            ui.separator();
        } else if !runner.name.is_empty() {
            ui.weak(format!("{}: {}", short_name(&runner.name), runner.status));
            ui.separator();
        }
        if entries.is_empty() {
            ui.label(i18n.tr("No scripts found in data/scripts."));
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (title, files) in &entries {
                ui.label(egui::RichText::new(title).strong());
                for f in files {
                    if ui.link(short_name(f)).on_hover_text(f).clicked() {
                        start = Some(f.clone());
                    }
                }
                ui.add_space(4.0);
            }
        });
    });
    ui_state.show = open;
    if let Some(f) = start {
        runner.request(&f);
    }
    runner.stop |= stop;
    runner.skip |= skip;
    Ok(())
}

fn short_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_direction_needs_the_same_axis() {
        // W flies along SE's +z (ahead); a slight sideways drift fails, as in FUN_140275dd0.
        assert!(direction_matches([0.0, 0.0, 1.0], DVec3::new(0.0, 0.0, 1.0)));
        assert!(direction_matches([0.0, 0.0, 1.0], DVec3::new(1e-9, 0.0, 1.0)));
        assert!(!direction_matches([0.0, 0.0, 1.0], DVec3::new(0.1, 0.0, 0.995)));
        assert!(!direction_matches([0.0, 0.0, -1.0], DVec3::new(0.0, 0.0, 1.0)));
        // The port's camera moving forward (-z) is SE's +z.
        let v = DVec3::NEG_Z * 500.0;
        assert!(direction_matches([0.0, 0.0, 1.0], -(DQuat::IDENTITY.inverse() * v).normalize()));
    }

    #[test]
    fn flight_profile() {
        let a = GotoArgs { accel_time: Some(1.0), decel_time: Some(1.0), drift_time: Some(2.0), ..Default::default() };
        let p = Profile::from_args(&a).unwrap();
        assert_eq!(p.total(), 4.0);
        assert_eq!((p.progress(0.0), p.progress(1.0)), (0.0, 1.0));
        // Symmetric: half way at the middle; monotonic.
        assert!((p.progress(0.5) - 0.5).abs() < 1e-12);
        let mut last = 0.0;
        for i in 1..=100 {
            let s = p.progress(i as f64 / 100.0);
            assert!(s >= last);
            last = s;
        }
        // Cruising at constant speed: equal steps in the drift.
        let (s1, s2, s3) = (p.progress(0.4), p.progress(0.5), p.progress(0.6));
        assert!(((s2 - s1) - (s3 - s2)).abs() < 1e-12);
        assert!(Profile::from_args(&GotoArgs::default()).is_none());
    }

    #[test]
    fn fly_speed_eases() {
        let mut m = Motion::new(DVec3::NEG_Z, 0.0, 100.0, 2.0);
        assert!(m.step(1.0));
        assert!((m.cur - 50.0).abs() < 1e-9);
        m.step(5.0);
        assert_eq!(m.cur, 100.0);
        let mut stop = Motion::new(m.axis, m.cur, 0.0, 0.0);
        assert!(!stop.step(0.016));
    }

    #[test]
    fn lon_lat_over_a_body() {
        let d = se_core::se_script::polar_dir(30.0, 70.0);
        let (lon, lat) = lon_lat(DVec3::from_array(d) * 5.0);
        assert!((lon - 30.0).abs() < 1e-9 && (lat - 70.0).abs() < 1e-9);
    }
}
