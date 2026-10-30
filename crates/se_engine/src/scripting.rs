//! Runs SpaceEngine `.se` scripts (see `docs/formats/se_script.md`).
//!
//! `se_core::se_script` parses a script into a flat command list; [`ScriptRunner`]
//! executes it over frames: instant commands run back to back within a frame, while
//! `Wait`, `WaitTrigger`, `WaitMessage` and `WaitVar` block the script until their
//! condition holds. `Goto` flights, `Orbit`, `FOV` and `Interpolate` run as effects
//! alongside the script. Camera moves reuse the free-flight camera
//! ([`crate::camera::FlyCamera`]) and its `Follow` binding, selection is
//! `FlyCamera::selected`, and time commands drive [`SimTime`].
//!
//! Commands the port has no equivalent for (tutorial widgets, dialogs, sounds, input
//! locks, waypoints) are logged once and skipped. Triggers that wait for GUI actions
//! the port does not have give up after a few seconds so tutorials keep flowing;
//! the "Skip step" button ends any wait early.
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
use se_core::se_script::{self, Command, GotoArgs, Location, MarkupLine, MultiMode, Op, Script, Trigger, VarWait};
use se_core::places::PC_M;
use se_core::upos::UPos;

use crate::camera::{Follow, FlyCamera, clamp_speed, look_rotation};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::hud::HudState;
use crate::i18n::I18n;
use crate::overlay::OverlaySettings;
use crate::stars::StarSettings;
use crate::universe::SimTime;

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

pub struct ScriptingPlugin;

impl Plugin for ScriptingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScriptRunner>()
            .init_resource::<ScriptUi>()
            .add_systems(Startup, queue_cli_script)
            .add_systems(Update, run_script)
            .add_systems(EguiPrimaryContextPass, (draw_captions, draw_scripts_window).in_set(crate::skin::UiSet::Draw));
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
    pos: UPos,
    rot: DQuat,
    fov: f32,
    speed: f64,
    rate: f64,
    paused: bool,
    jd: f64,
    selected: Option<ObjectRef>,
    travel: bool,
    following: bool,
    rotating: bool,
}

struct TrigState {
    trig: Trigger,
    snap: Snapshot,
    done: bool,
}

enum Blocked {
    Seconds(f64),
    Triggers { mode: MultiMode, items: Vec<TrigState>, elapsed: f64, max_wait: Option<f64> },
    Message { elapsed: f64 },
    Var { name: String, wait: VarWait, elapsed: f64 },
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
    fov_anim: Option<Anim>,
    interps: Vec<Interp>,
    pub caption: Option<Caption>,
    print: Option<(String, Option<f64>)>,
    logged: HashSet<String>,
    unsupported_vars: Vec<String>,
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
        self.fov_anim = None;
        self.interps.clear();
        self.caption = None;
        self.print = None;
        self.saved.clear();
        self.status.clear();
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

struct Cx<'a> {
    cam: &'a mut FlyCamera,
    sim: &'a mut SimTime,
    data: &'a SeData,
    overlay: &'a mut OverlaySettings,
    stars: &'a mut StarSettings,
    auto: bool,
    aspect: f64,
    quit: bool,
}

impl<'a> Cx<'a> {
    fn cat(&self) -> &'a Catalog {
        let data: &'a SeData = self.data;
        &data.catalog
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
    mut overlay: ResMut<OverlaySettings>,
    mut stars: ResMut<StarSettings>,
    mut hud: ResMut<HudState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(path) = runner.pending.take() {
        runner.reset();
        runner.stop = false;
        if let Some(frame) = load_frame(&data, &path) {
            runner.name = frame.name.clone();
            // The main-menu background scene is a bare scene: hide the interface.
            if frame.name.to_ascii_lowercase().contains("scripts/menu/") && !hud.hidden {
                hud.hidden = true;
                runner.hid_hud = true;
            }
            runner.frames.push(frame);
        }
    }
    if runner.stop {
        runner.stop = false;
        runner.reset();
        if runner.hid_hud {
            hud.hidden = false;
            runner.hid_hud = false;
        }
        info!("script {} stopped", runner.name);
        return;
    }
    if runner.frames.is_empty() {
        return;
    }
    let aspect = windows.single().map(|w| (w.width() / w.height().max(1.0)) as f64).unwrap_or(16.0 / 9.0);
    let mut cx = Cx {
        cam: &mut cam,
        sim: &mut sim,
        data: &data,
        overlay: &mut overlay,
        stars: &mut stars,
        auto: cfg.script_auto,
        aspect,
        quit: false,
    };
    runner.step(&mut cx, time.delta_secs_f64());
    if cx.quit {
        exit.write(AppExit::Success);
    }
}

impl ScriptRunner {
    fn step(&mut self, cx: &mut Cx, mut dt: f64) {
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
            Op::CheckVersion(_) => {}
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
            Op::Select(name) => match resolve_object(cx.cat(), name) {
                Some(r) => cx.cam.selected = Some(r),
                None => self.warn_once(&format!("sel:{name}"), format!("object not found: \"{name}\" (line {})", cmd.line)),
            },
            Op::Unselect => cx.cam.selected = None,
            Op::Center { .. } => {
                if let Some(sel) = cx.cam.selected {
                    let eph = Ephemeris::new(cx.cat());
                    let d = eph.position(sel, eph_jd).delta_m(cx.cam.pos);
                    cx.cam.rot = look_rotation(d, cx.cam.rot * DVec3::Y);
                }
            }
            Op::Follow | Op::SyncRot => {
                if let Some(sel) = cx.cam.selected {
                    let eph = Ephemeris::new(cx.cat());
                    let sync = matches!(cmd.op, Op::SyncRot);
                    cx.cam.travel = None;
                    cx.cam.follow = Some(Follow::new(sel, sync, &eph, eph_jd));
                }
            }
            Op::Free => cx.cam.follow = None,
            Op::GotoLocation(loc) => self.goto_location(cx, loc),
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
                self.flight = None;
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
                    if resolve_object(cx.cat(), name).is_none() {
                        self.warn_once(
                            &format!("trigobj:{name}"),
                            format!("trigger object \"{name}\" is not in the catalog (waypoints are not supported), it times out (line {})", cmd.line),
                        );
                    }
                }
                let state = TrigState { trig: (**t).clone(), snap: snapshot(cx), done: false };
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
                self.blocked = Some(Blocked::Message { elapsed: 0.0 });
                self.status = "waiting for Ok".into();
            }
            Op::HideMessage => self.caption = None,
            Op::Print { text, time } => self.print = Some((text.clone(), *time)),
            Op::HidePrint => self.print = None,
            Op::Log(t) => info!("script {}: {t}", self.name),
            Op::Set { name, value, .. } => self.set_var(cx, name, value.clone()),
            Op::WaitVar { name, wait } => {
                if !self.var_satisfied(cx, name, wait) {
                    if !is_live_var(name) {
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
            Op::Waypoint { .. } | Op::ClearWaypoints | Op::DeleteWaypoint(_) => {
                self.warn_once("waypoint", format!("waypoints are not supported, skipping (line {})", cmd.line))
            }
            Op::Track => cx.cam.track = cx.cam.selected.is_some(),
            Op::Untrack => cx.cam.track = false,
            // `Horizon`: level the view to the selection's horizon (SE's Horizon object, End).
            Op::Other(item) if item.key.eq_ignore_ascii_case("horizon") => {
                if let Some(sel) = cx.cam.selected {
                    crate::camera::start_horizon(cx.cam, sel);
                }
            }
            Op::Other(item) if item.key.eq_ignore_ascii_case("stoprotation") => cx.cam.turn_vel = DVec3::ZERO,
            // `SpeedKm X`: set the assigned velocity (km/s).
            Op::Other(item) if item.key.eq_ignore_ascii_case("speedkm") => {
                if let Some(v) = item.f64() {
                    cx.cam.speed = clamp_speed(v * 1000.0);
                }
            }
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
            if trigger_class(&it.trig.kind) == Class::Unsupported {
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
            Blocked::Message { elapsed } => {
                *elapsed += dt;
                if ack || skip || (cx.auto && *elapsed >= AUTO_MESSAGE_S) {
                    if let Some(c) = &mut self.caption {
                        c.ack = false;
                    }
                    true
                } else {
                    false
                }
            }
            Blocked::Var { name, wait, elapsed } => {
                *elapsed += dt;
                skip
                    || self.var_satisfied(cx, name, wait)
                    || (!is_live_var(name) && *elapsed >= UNSUPPORTED_WAIT_S)
                    || (cx.auto && *elapsed >= AUTO_TRIGGER_S)
            }
            Blocked::Triggers { mode, items, elapsed, max_wait } => {
                *elapsed += dt;
                let el = *elapsed;
                for it in items.iter_mut() {
                    if it.done {
                        continue;
                    }
                    it.done = match check_trigger(&it.trig, &it.snap, cx, el) {
                        Check::Yes => true,
                        Check::No => false,
                        Check::Unsupported => el >= UNSUPPORTED_WAIT_S,
                    };
                }
                let ok = match mode {
                    MultiMode::And => items.iter().all(|i| i.done),
                    MultiMode::Or => items.iter().any(|i| i.done),
                };
                ok || skip || max_wait.is_some_and(|m| el >= m) || (cx.auto && el >= AUTO_TRIGGER_S)
            }
        };
        if !done {
            self.blocked = Some(blocked);
        }
        done
    }

    // ---- variables ----

    fn get_var(&self, cx: &Cx, name: &str) -> Option<f64> {
        let k = name.to_ascii_lowercase();
        match k.as_str() {
            "showorbits" => Some(cx.overlay.orbits as u8 as f64),
            "labelsmode" => Some(cx.overlay.labels_mode as f64),
            "showmarkers" => Some(cx.overlay.markers as u8 as f64),
            "starmagnlimit" => Some(cx.stars.mag_limit as f64),
            "timescale" => Some(cx.sim.rate),
            "fov" => Some(cx.cam.fov_deg as f64),
            // ShowConstellations, ShowGrid*, ConstLines<Abbr>, ...
            _ => cx.overlay.sky.var(&k).map(|on| on as u8 as f64).or_else(|| self.vars.get(&k).and_then(Value::as_f64)),
        }
    }

    fn set_var(&mut self, cx: &mut Cx, name: &str, value: Value) {
        let k = name.to_ascii_lowercase();
        let n = value.as_f64();
        match (k.as_str(), n) {
            ("showorbits", Some(v)) => cx.overlay.orbits = v != 0.0,
            ("labelsmode", Some(v)) => cx.overlay.labels_mode = v.clamp(0.0, 2.0) as u8,
            ("showmarkers", Some(v)) => cx.overlay.markers = v != 0.0,
            ("starmagnlimit", Some(v)) => cx.stars.mag_limit = v as f32,
            ("timescale", Some(v)) => cx.sim.rate = v,
            ("fov", Some(v)) => cx.cam.fov_deg = v as f32,
            // The constellation and grid switches.
            (f, Some(v)) if cx.overlay.sky.set_var(f, v != 0.0) => {}
            // SE's per-kind filters: ShowOrbitsPlanets, ShowLabelsStars, ShowMarkersGalaxies, ...
            (f, Some(v)) if f.starts_with("show") => match crate::overlay::kind_filter(cx.overlay, f) {
                Some((set, kind)) => set.set(kind, v != 0.0),
                None => self.store_var(name, k.clone(), value),
            },
            _ => self.store_var(name, k.clone(), value),
        }
    }

    /// A variable the port does not act on, kept for scripts to read back.
    fn store_var(&mut self, name: &str, k: String, value: Value) {
        if !is_live_var(&k) && !self.vars.contains_key(&k) && !IGNORED_VARS.contains(&k.as_str()) {
            self.unsupported_vars.push(name.to_string());
        }
        self.vars.insert(k, value);
    }

    fn snapshot_vars(&self, cx: &Cx) -> HashMap<String, Value> {
        let mut m = self.vars.clone();
        for k in LIVE_VARS {
            if let Some(v) = self.get_var(cx, k) {
                m.insert((*k).to_string(), Value::Num(v));
            }
        }
        for k in crate::overlay::SkySwitches::KEYS {
            m.insert(k.to_ascii_lowercase(), Value::Num(cx.overlay.sky.var(k).unwrap_or(false) as u8 as f64));
        }
        m
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
        let end_rot = if oriented && !a.center {
            horizon_rotation(dir, a.yaw.unwrap_or(0.0), a.pitch.unwrap_or(0.0), a.roll.unwrap_or(0.0))
        } else {
            look_rotation(-end_local, DVec3::Y)
        };
        cx.cam.travel = None;
        cx.cam.follow = None;
        cx.cam.velocity = DVec3::ZERO;
        let duration = a.time.unwrap_or(DEFAULT_GOTO_S).max(0.0);
        // SE binds a Goto to a point on the surface with SyncRot, any other with Follow (FUN_140563f30).
        let sync = a.lat.is_some() || a.lon.is_some();
        self.flight = Some(Flight { target, start_local, start_rot, end_local, end_rot, elapsed: 0.0, duration, sync });
        // Zero-time flights land immediately, so following commands see the result.
        self.update_flight(cx, 0.0);
    }

    fn update_flight(&mut self, cx: &mut Cx, dt: f64) {
        let Some(f) = &mut self.flight else { return };
        f.elapsed += dt;
        let t = if f.duration <= 0.0 { 1.0 } else { (f.elapsed / f.duration).clamp(0.0, 1.0) };
        let s = smooth(t);
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

/// Variables mirrored by live port settings.
const LIVE_VARS: &[&str] = &["showorbits", "labelsmode", "starmagnlimit", "timescale", "fov"];

/// Variables that only affect SpaceEngine's GUI feedback; setting them is not worth a report.
const IGNORED_VARS: &[&str] = &["silentvarchange", "lessonpassed"];

fn is_live_var(name: &str) -> bool {
    LIVE_VARS.contains(&name.to_ascii_lowercase().as_str())
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn snapshot(cx: &Cx) -> Snapshot {
    Snapshot {
        pos: cx.cam.pos,
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
fn resolve_object(cat: &Catalog, name: &str) -> Option<ObjectRef> {
    if let Some(r) = cat.find(name) {
        return Some(r);
    }
    name.rsplit('|').find_map(|part| cat.find(part.trim()))
}

// ---- triggers ----------------------------------------------------------------------

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Supported,
    Unsupported,
}

enum Check {
    Yes,
    No,
    Unsupported,
}

fn trigger_class(kind: &str) -> Class {
    match kind.to_ascii_lowercase().as_str() {
        "loadingcomplete" | "screenshotcomplete" | "entermenu" | "enterplanetarium" | "entersingleplayer" | "time"
        | "object|select" | "object|approach" | "object|see" | "control|goto" | "control|move" | "control|rotate"
        | "control|orbit" | "control|changespeed" | "control|settimerate" | "control|pausetime"
        | "control|resumetime" | "control|reversetime" | "control|changefov" | "control|center" | "control|follow"
        | "control|land" | "control|stopmove" | "control|syncrot" => Class::Supported,
        _ => Class::Unsupported,
    }
}

fn in_range(v: f64, r: (f64, f64)) -> bool {
    v >= r.0.min(r.1) && v <= r.0.max(r.1)
}

fn type_matches(cat: &Catalog, r: ObjectRef, ty: &str) -> bool {
    match (r, ty.to_ascii_lowercase().as_str()) {
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

fn check_trigger(t: &Trigger, snap: &Snapshot, cx: &Cx, elapsed: f64) -> Check {
    let yes = |b: bool| if b { Check::Yes } else { Check::No };
    let cat = cx.cat();
    let eph = Ephemeris::new(cat);
    let jd = cx.sim.jd;
    let cam = &*cx.cam;
    match t.kind.to_ascii_lowercase().as_str() {
        "loadingcomplete" => yes(elapsed >= LOADING_SETTLE_S),
        "screenshotcomplete" | "entermenu" | "enterplanetarium" | "entersingleplayer" => Check::Yes,
        "time" => {
            let Some(target) = t.time_jd else { return Check::Yes };
            let mode = t.mode.as_deref().unwrap_or("later").to_ascii_lowercase();
            yes(match mode.as_str() {
                "earlier" => jd <= target,
                "crossed" => (snap.jd < target) != (jd < target),
                _ => jd >= target,
            })
        }
        "object|select" => match &t.object {
            Some(name) => match resolve_object(cat, name) {
                Some(want) => yes(cam.selected == Some(want) && snap.selected != cam.selected),
                None => Check::Unsupported,
            },
            None => yes(cam.selected.is_some() && snap.selected != cam.selected),
        },
        "object|approach" => {
            let obj = match (&t.object, &t.ty) {
                (Some(name), _) => match resolve_object(cat, name) {
                    Some(r) => Some(r),
                    None => return Check::Unsupported,
                },
                (None, Some(ty)) if ty.eq_ignore_ascii_case("selection") => cam.selected,
                (None, Some(ty)) => {
                    let near = cat.find(&cam.nearest_name);
                    near.filter(|r| type_matches(cat, *r, ty))
                }
                (None, None) => cam.selected,
            };
            let Some(obj) = obj else { return Check::No };
            let d = eph.position(obj, jd).delta_m(cam.pos).length();
            let radius = eph.radius_m(obj).max(1.0);
            if let Some(r) = t.range_rad {
                return yes(in_range(d / radius, r));
            }
            if let Some(r) = t.range_km {
                return yes(in_range((d - radius).max(0.0) / 1000.0, r));
            }
            yes(d / radius < 10.0)
        }
        "object|see" => {
            let obj = match &t.object {
                Some(n) => match resolve_object(cat, n) {
                    Some(r) => Some(r),
                    None => return Check::Unsupported,
                },
                None => cam.selected,
            };
            let Some(obj) = obj else { return Check::No };
            let local = cam.rot.inverse() * eph.position(obj, jd).delta_m(cam.pos);
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
        "control|goto" => {
            let started = cam.travel.is_some() && !snap.travel;
            let type_ok = match (&t.ty, cam.travel.as_ref()) {
                (Some(ty), Some(tr)) => type_matches(cat, tr.target, ty),
                _ => true,
            };
            yes(started && type_ok)
        }
        "control|move" => {
            let d_km = cam.pos.delta_m(snap.pos).length() / 1000.0;
            match t.range_km {
                Some(r) => yes(in_range(d_km, r)),
                None => yes(d_km > 1.0),
            }
        }
        "control|rotate" | "control|orbit" => {
            let (y, p, _) = rotation_since(snap, cam.rot);
            let mut tests = Vec::new();
            if let Some(r) = t.yaw_range {
                tests.push(outside(y, r));
            }
            if let Some(r) = t.pitch_range {
                tests.push(outside(p, r));
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
        "control|changefov" => yes((cam.fov_deg - snap.fov).abs() > 0.5),
        "control|center" => {
            let Some(sel) = cam.selected else { return Check::No };
            let to = eph.position(sel, jd).delta_m(cam.pos);
            let fwd = cam.rot * DVec3::NEG_Z;
            yes(fwd.angle_between(to) < 3f64.to_radians())
        }
        "control|follow" => yes(cam.follow.is_some() && !snap.following),
        "control|land" => yes(cam.nearest_surface < 100.0),
        "control|syncrot" => yes(cam.bind_mode() == 2 && !snap.rotating),
        "control|stopmove" => yes(cam.velocity.length() < 1e-3 && cam.travel.is_none()),
        _ => Check::Unsupported,
    }
}

// ---- UI ----------------------------------------------------------------------------

fn draw_captions(
    mut contexts: EguiContexts,
    mut runner: ResMut<ScriptRunner>,
    i18n: Res<I18n>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if let Some(cap) = runner.caption.clone() {
        let blocked_on_trigger = matches!(runner.blocked, Some(Blocked::Triggers { .. } | Blocked::Var { .. }));
        let mut ack = false;
        let mut skip = false;
        egui::Area::new(egui::Id::new("script_caption"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -36.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_black_alpha(205))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 110, 150)))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_max_width(680.0);
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
