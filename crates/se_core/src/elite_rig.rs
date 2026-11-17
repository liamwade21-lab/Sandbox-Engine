//! Elite Dangerous's ship animation rules as pure code (ARCHITECTURE.md A5): posing a kinematic rig, evaluating its
//! Bezier curves, and running its code state machines. The reference is the export's `kinpose.py`
//! (`re/elite/frea_export/sidewinder/decoded/animation/`), the proofs `re/elite/notes/sidewinder_animation.md` and
//! `agent_csm.md`. The data is `ship.json` (`crate::elite_ship`). Nothing here draws, plays or reads a file: the
//! caller supplies the bones and acts on the events.
//!
//! Conventions:
//! - Elite's model space: +X starboard, +Y up, +Z nose (left-handed), metres; time in seconds; angles as the data
//!   stores them. Nothing is converted to the mod's space here.
//! - Matrices are glam's `DMat4` with column vectors. Elite stores row vectors (row-major, rows 0-2 the local X, Y, Z
//!   axes, row 3 the translation, a point mapping as p' = p.M); a matrix here is Elite's transposed, converted once by
//!   `elite_matrix`: its columns `x_axis`, `y_axis`, `z_axis` are the node's local axes and `w_axis` its position, and
//!   Elite's product `A.B` is `B * A` here.
//! - Arithmetic is f64, as the reference's (Elite itself runs in float32).
//! - Values (rig motions' values, generic parameters, the published dominant-curve variables) live in one `Values`
//!   table that the caller keeps for the ship; a value never set is 0.
//!
//! The machines (`Runner`, one per machine): a request names a target; each `step` takes one transition of the cheapest
//! route (Dijkstra over `cost`), leaving at once (`wait_for_activities` 0) or once the state's running activities have
//! finished or completed a cycle (1). Curves (by `counts_as_running`), `WaitSomeGameTime` and waiting door requests run;
//! every other type keeps the framework's default IsRunning, false (agent_csm.md sections 1, 2, 9a).
//! - Door requests couple the gear legs to the hull's doors (`Event::DoorRequest`: the caller requests the door's
//!   machine). `DoorClientRequest` has its own IsFinished (0x1443aa5d0): with `door[1]` set it is unfinished while the
//!   door client says its door is busy, so the leg's DoorsOpen -> Lowering and DoorsClosed -> Up (both waiting) wait for
//!   the doors (sidewinder_animation.md, "Who requests the machines' states"). The runner knows nothing of other
//!   machines: `step_with` takes the caller's answer for "is this door busy"; `step` means no door is ever busy.
//! - A step longer than a frame (`MAX_FRAME_S`) runs as frames, so it gives the events a run of frames would, in that
//!   order; a step changes state at most as many times as the machine has states (then `Event::Fault`).
//! - Not traced in the exe, chosen and marked here: entering a state with `SetTargetState X` makes X the machine's
//!   target as a request would (the cheapest route, `wait_for_activities` honoured), and only when no route leads from
//!   that state to X does the machine move straight there. It is the one rule that fits all six in the stock machines:
//!   `Dying` must pass through `Exploding` (its explosion and its 0.15 s wait) to `Dead`, `Reboot` routes back to the
//!   lights, and the `JumpTo*` states have no transitions at all. A sound window still open when its state is left posts its end event
//!   then (two windows run past their curve's end: gear_front LowerShip's Stow_Motor03, gear_right Rising's
//!   Stow_Door_Hinge); a window that ended before a resumed clock's start is skipped, one the start falls inside opens
//!   at the first step; the lights carry their off and on levels, their intensity animator (fades, blinks) is not run.

use std::collections::HashMap;

use glam::DMat4;

use crate::elite_ship::{Activity, CurveActivity, KnownActivity, Light, Machine, Motion, MotionKind, RigNode, State};

/// Every named value a ship's machines write and its rigs read.
pub type Values = HashMap<String, f64>;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RigError {
    #[error("rig node {node}: no bone {bone} in model {model}")]
    NoBone { node: String, model: String, bone: String },
    #[error("no rig node {0}")]
    NoNode(String),
    #[error("machine {machine}: no state {state}")]
    NoState { machine: String, state: String },
    #[error("machine {machine}: no route from {from} to {to}")]
    NoRoute { machine: String, from: String, to: String },
    #[error("machine {machine}: {why}")]
    BadMachine { machine: String, why: String },
}

/// Values within this of 0 move nothing.
const EPS: f64 = 0.001;
/// `publishes_dominant`'s threshold: the first curve's ends differ by this share (DAT_144de1654).
const DOMINANT_EPS: f64 = 0.05;
/// The longest frame a step runs as one: a frame at 60 Hz. Longer steps are split into equal frames.
pub const MAX_FRAME_S: f64 = 1.0 / 60.0;
/// The most frames one step runs (18 minutes at 60 Hz); a longer step takes longer frames.
pub const MAX_FRAMES: usize = 1 << 16;

/// Elite's stored matrix (16 numbers, row-major, translation in row 3) as a matrix here.
pub fn elite_matrix(rows: &[f64; 16]) -> DMat4 {
    DMat4::from_cols_array(rows)
}

/// Bind matrices by (model, bone), both matched case-insensitively as the engine hashes names in lower case.
#[derive(Clone, Debug, Default)]
pub struct BoneTable(HashMap<(String, String), DMat4>);

impl BoneTable {
    pub fn insert(&mut self, model: &str, bone: &str, bind: DMat4) {
        self.0.insert((model.to_lowercase(), bone.to_lowercase()), bind);
    }

    pub fn get(&self, model: &str, bone: &str) -> Option<DMat4> {
        self.0.get(&(model.to_lowercase(), bone.to_lowercase())).copied()
    }
}

/// One motion acting on a node's matrix (`kinpose.apply_motion`).
pub fn apply_motion(m: DMat4, motion: &Motion, v: f64) -> DMat4 {
    if v.abs() <= EPS {
        return m;
    }
    let mut m = m;
    match motion.kind {
        MotionKind::RotateDirect | MotionKind::RotateDriven => {
            let ang = if motion.kind == MotionKind::RotateDirect {
                v * motion.sign()
            } else {
                (if v >= 0.0 { v * motion.b } else { -v * motion.a }) * std::f64::consts::PI / 180.0
            };
            let (c, s) = (ang.cos(), ang.sin());
            let (r0, r1, r2) = (m.x_axis, m.y_axis, m.z_axis);
            match motion.axis {
                0 => {
                    m.x_axis = r0 * c + r1 * s;
                    m.y_axis = r1 * c - r0 * s;
                }
                1 => {
                    m.x_axis = r0 * c - r2 * s;
                    m.z_axis = r0 * s + r2 * c;
                }
                _ => {
                    m.y_axis = r1 * c + r2 * s;
                    m.z_axis = r2 * c - r1 * s;
                }
            }
        }
        MotionKind::TranslateDirect | MotionKind::TranslateDriven => {
            let d = if motion.kind == MotionKind::TranslateDirect {
                v * motion.sign()
            } else {
                v.abs() * if v >= 0.0 { motion.b } else { motion.a }
            };
            let row = match motion.axis {
                0 => m.z_axis,
                1 => m.y_axis,
                _ => m.x_axis,
            };
            m.w_axis += row * d;
        }
    }
    m
}

/// A posed node: its world matrix in the space of the root's `base`.
#[derive(Clone, Debug)]
pub struct NodePose<'a> {
    pub node: &'a RigNode,
    pub world: DMat4,
}

/// Poses a rig (`kinpose.pose`): every node, parents first. A node's matrix starts as its attach bone's bind in its
/// parent's model times the parent's world matrix (the root, or a node without a bone or a parent model, at the
/// parent's), then its motions act in list order with `values`.
pub fn pose<'a>(root: &'a RigNode, bones: &BoneTable, values: &Values, base: DMat4) -> Result<Vec<NodePose<'a>>, RigError> {
    let mut out = Vec::new();
    walk(root, base, None, bones, values, &mut out)?;
    Ok(out)
}

fn walk<'a>(
    node: &'a RigNode,
    parent_world: DMat4,
    parent_model: Option<&str>,
    bones: &BoneTable,
    values: &Values,
    out: &mut Vec<NodePose<'a>>,
) -> Result<(), RigError> {
    let mut m = match (&node.attach_bone, parent_model) {
        (Some(bone), Some(model)) => {
            let bind = bones.get(model, bone).ok_or_else(|| RigError::NoBone {
                node: node.name.clone(),
                model: model.to_string(),
                bone: bone.clone(),
            })?;
            parent_world * bind
        }
        _ => parent_world,
    };
    for motion in &node.motions {
        m = apply_motion(m, motion, values.get(&motion.value).copied().unwrap_or(0.0));
    }
    out.push(NodePose { node, world: m });
    for c in &node.children {
        walk(c, m, node.model.as_deref(), bones, values, out)?;
    }
    Ok(())
}

/// Where a rig hangs from another's posed node: that node's world times the bind of `bone` in its model (the ship
/// item's `LandingGearRoot!AttachLandingGearLeft`). The base to pose the hung rig with.
pub fn attach_base(poses: &[NodePose], bones: &BoneTable, node: &str, bone: &str) -> Result<DMat4, RigError> {
    let p = poses.iter().find(|p| p.node.name.eq_ignore_ascii_case(node)).ok_or_else(|| RigError::NoNode(node.to_string()))?;
    let model = p.node.model.as_deref().unwrap_or("");
    let bind = bones.get(model, bone).ok_or_else(|| RigError::NoBone {
        node: node.to_string(),
        model: model.to_string(),
        bone: bone.to_string(),
    })?;
    Ok(p.world * bind)
}

/// The values a state sets at once (its `SetKinematicValueMultiple` activities).
pub fn state_pose(state: &State) -> Values {
    let mut v = Values::new();
    for a in &state.activities {
        if let Activity::Known(KnownActivity::SetKinematicValueMultiple { values }) = a {
            v.extend(values.iter().map(|(k, x)| (k.clone(), *x)));
        }
    }
    v
}

// ------------------------------------------------------------------------------------------------ curves

/// A key: time s, value, in slope, in length, out slope, out length.
pub type Key = [f64; 6];

/// The control points of the segment k0 -> k1 (`kinpose._segment`; the exe's 0x14431be40, 0x140929960).
fn segment(k0: &Key, k1: &Key) -> [(f64, f64); 4] {
    let [t0, v0, _, _, mut os, mut ol] = *k0;
    let [t1, v1, mut is, mut il, _, _] = *k1;
    let lin = (v1 - v0) / (t1 - t0);
    let straight = |slope: f64, length: f64| length.abs() < 0.01 || (lin != 0.0 && (slope / lin - 1.0).abs() < 0.01);
    if straight(os, ol) && straight(is, il) {
        (os, ol, is, il) = (0.0, 0.0, 0.0, 0.0);
    }
    let x1 = ol / (os * os + 1.0).sqrt() + t0;
    let y1 = (x1 - t0) * os + v0;
    let x2 = -il / (is * is + 1.0).sqrt() + t1;
    let y2 = (x2 - t1) * is + v1;
    [(t0, v0), (x1, y1), (x2, y2), (t1, v1)]
}

fn bezier(p0: f64, p1: f64, p2: f64, p3: f64, u: f64) -> f64 {
    (1.0 - u).powf(3.0) * p0 + 3.0 * (1.0 - u).powf(2.0) * u * p1 + 3.0 * (1.0 - u) * u * u * p2 + u.powf(3.0) * p3
}

/// u in [0, 1] where x(u) = t: bisection, as the reference (the exe solves the cubic; x increases over a segment).
fn solve_u(x: [f64; 4], t: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let u = (lo + hi) / 2.0;
        if bezier(x[0], x[1], x[2], x[3], u) < t {
            lo = u;
        } else {
            hi = u;
        }
    }
    (lo + hi) / 2.0
}

/// A curve's value at time t (`kinpose.eval_curve`): before the first key the first value, after the last the last
/// (one key: its value always; no keys: 0).
pub fn eval_curve(keys: &[Key], t: f64) -> f64 {
    let (Some(&first), Some(&last)) = (keys.first(), keys.last()) else { return 0.0 };
    if t <= first[0] {
        return first[1];
    }
    if t >= last[0] {
        return last[1];
    }
    let i = (0..keys.len() - 1).rev().find(|&j| keys[j][0] <= t).unwrap_or(0);
    let p = segment(&keys[i], &keys[i + 1]);
    if t <= p[0].0 {
        return p[0].1;
    }
    let u = solve_u([p[0].0, p[1].0, p[2].0, p[3].0], t);
    bezier(p[0].1, p[1].1, p[2].1, p[3].1, u)
}

/// The time at which a curve first passes through v (`kinpose.time_for_value`: the start of a clock whose
/// `start_at_time_zero` is clear). A value outside the curve's range starts at the nearest extreme key. Fewer than two
/// keys: 0.
pub fn time_for_value(keys: &[Key], v: f64) -> f64 {
    if keys.len() < 2 {
        return 0.0;
    }
    const N: usize = 200;
    for i in 0..keys.len() - 1 {
        let p = segment(&keys[i], &keys[i + 1]);
        let (x0, x3) = (p[0].0, p[3].0);
        let (mut prev_t, mut prev_y) = (x0, p[0].1);
        for j in 1..=N {
            let t = x0 + (x3 - x0) * j as f64 / N as f64;
            let y = eval_curve(keys, t);
            if (prev_y - v) * (y - v) <= 0.0 {
                let (mut lo, mut hi) = (prev_t, t);
                for _ in 0..50 {
                    let mid = (lo + hi) / 2.0;
                    if (eval_curve(keys, lo) - v) * (eval_curve(keys, mid) - v) <= 0.0 {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return (lo + hi) / 2.0;
            }
            (prev_t, prev_y) = (t, y);
        }
    }
    // the first key holding the highest / lowest value, as Python's max / min pick it
    let hi_k = keys.iter().fold(keys[0], |a, k| if k[1] > a[1] { *k } else { a });
    let lo_k = keys.iter().fold(keys[0], |a, k| if k[1] < a[1] { *k } else { a });
    if (v - hi_k[1]).abs() < (v - lo_k[1]).abs() {
        hi_k[0]
    } else {
        lo_k[0]
    }
}

/// Whether a curve activity (start_at_time_zero clear, publish_dominant_value set) publishes its
/// `<owner>_<value>_DominantCurveValue`: when its first curve's first and last values differ by a relative 0.05 (no
/// keys: false).
pub fn publishes_dominant(first_curve_keys: &[Key]) -> bool {
    let (Some(k0), Some(kn)) = (first_curve_keys.first(), first_curve_keys.last()) else { return false };
    let (v0, vn) = (k0[1], kn[1]);
    let m = v0.abs().max(vn.abs());
    m != 0.0 && (v0 / m - vn / m).abs() >= DOMINANT_EPS
}

/// The dominant curve the resume rule solves: the first curve, or when published a straight ramp over its whole span
/// (no keys: none).
pub fn dominant_keys(first_curve_keys: &[Key], publish: bool) -> Vec<Key> {
    let (Some(k0), Some(kn), true) = (first_curve_keys.first(), first_curve_keys.last(), publish) else {
        return first_curve_keys.to_vec();
    };
    vec![[k0[0], k0[1], 0.0, 0.0, 0.0, 0.0], [kn[0], kn[1], 0.0, 0.0, 0.0, 0.0]]
}

/// Where a curve activity's clock starts: 0 with `start_at_time_zero`, else where the dominant curve passes
/// `current_value` (the published variable's value, or the first curve's driven value when nothing is published).
pub fn resume_time(first_curve_keys: &[Key], current_value: f64, start_at_time_zero: bool, publish: bool) -> f64 {
    if start_at_time_zero {
        return 0.0;
    }
    time_for_value(&dominant_keys(first_curve_keys, publish), current_value)
}

/// A value during a curve activity's blend-in: (1 - w) x its value at Start + w x the curve's first value,
/// w = clamp(timer / blend, 0, 1).
pub fn blend_in(start_value: f64, curve_value: f64, timer_s: f64, blend_s: f64) -> f64 {
    let w = if blend_s <= 0.0 { 0.0 } else { (timer_s / blend_s).clamp(0.0, 1.0) };
    (1.0 - w) * start_value + w * curve_value
}

// ------------------------------------------------------------------------------------------------ state machines

/// What a machine did, for the caller to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The machine entered a state.
    Entered(String),
    /// A sound cue's window opened: post `event` (None: no bank has the cue's event, so silence).
    SoundBegin { cue: String, event: Option<String>, bones: Vec<String> },
    /// A sound cue's window closed (or its state was left while it was open).
    SoundEnd { cue: String, event: Option<String>, bones: Vec<String> },
    /// A flag-0 cue's one event, at its window's start.
    SoundSingle { cue: String, event: Option<String>, bones: Vec<String> },
    /// `SetLandingGearState`: 0 up, 1 doors open, 2 down, 3 rising.
    GearState(u32),
    /// `DoorClientRequest`: the caller asks the doors' machine to open or close.
    DoorRequest { open: bool, door: [u16; 2] },
    /// A light of the entered state comes on (its off and on levels are in it).
    LightOn(Light),
    /// A light of the left state goes out.
    LightOff(Light),
    /// `AudioState`: set the Wwise state.
    AudioState { group: String, state: String },
    /// An activity type the rules don't run started (index into the state's activities).
    Started { kind: String, index: usize },
    /// ... and stopped.
    Stopped { kind: String, index: usize },
    /// The machine stopped: more state changes in one step than it has states (a loop in damaged data).
    Fault(String),
}

/// A window of a sound cue: waiting, open, or done.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Window {
    Waiting,
    Open,
    Done,
}

#[derive(Clone, Debug)]
struct CurveRun {
    time: f64,
    /// Loops completed (the cycle counter a waiting transition watches).
    cycles: u32,
    /// The blend-in's timer.
    blend: f64,
    /// Each curve's value at Start.
    start: Vec<f64>,
    /// The published ramp: its variable and keys.
    ramp: Option<(String, Vec<Key>)>,
    windows: Vec<Vec<Window>>,
}

#[derive(Clone, Debug)]
enum Run {
    Curve(CurveRun),
    Wait(f64),
    /// A `DoorClientRequest`: its `door` value (0 open or close, 1 whether to wait for the door).
    Door([u16; 2]),
    Idle,
}

/// The door answer of `Runner::step`: no door is ever busy.
fn no_door_busy(_: [u16; 2]) -> bool {
    false
}

/// Runs one state machine: requests name a target state; `step` advances it.
#[derive(Clone, Debug)]
pub struct Runner {
    machine: Machine,
    current: usize,
    target: Option<usize>,
    /// One per activity of the current state.
    runs: Vec<Run>,
    /// The next state on the route and the (activity, cycle count) pairs it still waits for.
    waiting: Option<(usize, Vec<(usize, u32)>)>,
}

fn curve_name(c: &crate::elite_ship::Curve) -> &str {
    c.value.as_deref().or(c.param.as_deref()).unwrap_or("")
}

fn write(values: &mut Values, c: &crate::elite_ship::Curve, v: f64) {
    for name in [&c.value, &c.param].into_iter().flatten() {
        values.insert(name.clone(), v);
    }
}

impl Runner {
    /// A runner in the machine's initial state, its activities started. The machine is checked first
    /// (`Machine::check`, what `parse_ship` refuses), so one made otherwise is an `Err`, never a panic.
    pub fn new(machine: &Machine, values: &mut Values) -> Result<(Runner, Vec<Event>), RigError> {
        machine.check().map_err(|why| RigError::BadMachine { machine: machine.name.clone(), why })?;
        let initial = machine.state_index(&machine.initial).ok_or_else(|| RigError::NoState {
            machine: machine.name.clone(),
            state: machine.initial.clone(),
        })?;
        let mut r = Runner { machine: machine.clone(), current: initial, target: None, runs: Vec::new(), waiting: None };
        let mut events = Vec::new();
        if !r.change(initial, values, &mut events, &mut 0) {
            let why = events.iter().rev().find_map(|e| if let Event::Fault(w) = e { Some(w.clone()) } else { None }).unwrap_or_default();
            return Err(RigError::BadMachine { machine: machine.name.clone(), why });
        }
        Ok((r, events))
    }

    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// The current state's name.
    pub fn state(&self) -> &str {
        &self.machine.states[self.current].name
    }

    /// Asks for a state: the machine walks the cheapest route of transitions to it. A state no route reaches from the
    /// current one is an `Err`, and the machine goes on as it was.
    pub fn request(&mut self, target: &str) -> Result<(), RigError> {
        let t = self.index(target)?;
        if !self.machine.reaches(self.current, t) {
            return Err(RigError::NoRoute { machine: self.machine.name.clone(), from: self.state().to_string(), to: target.to_string() });
        }
        self.target = Some(t);
        Ok(())
    }

    /// Whether any activity of the current state is running (as a waiting transition sees it), no door being busy.
    pub fn busy(&self) -> bool {
        self.busy_with(&no_door_busy)
    }

    /// `busy` with the caller's answer for the doors (`step_with`).
    pub fn busy_with(&self, door_busy: &dyn Fn([u16; 2]) -> bool) -> bool {
        (0..self.runs.len()).any(|i| self.running(i, door_busy))
    }

    /// The state the machine is walking to (a request's or a `SetTargetState`'s), None before any.
    pub fn target(&self) -> Option<&str> {
        self.target.map(|t| self.machine.states[t].name.as_str())
    }

    /// Where the current state's curve activity `index` stands on its clock, seconds (None: not a running curve).
    pub fn curve_time(&self, index: usize) -> Option<f64> {
        match self.runs.get(index) {
            Some(Run::Curve(c)) => Some(c.time),
            _ => None,
        }
    }

    fn index(&self, state: &str) -> Result<usize, RigError> {
        self.machine.state_index(state).ok_or_else(|| RigError::NoState { machine: self.machine.name.clone(), state: state.to_string() })
    }

    /// Advances the clocks by dt seconds (a negative or NaN dt is 0), writes the driven values and takes the
    /// transitions of the route when they may leave. A step longer than `MAX_FRAME_S` is taken as equal frames no longer
    /// than it (at most `MAX_FRAMES`), so its events come out as a run of frames would give them, in time order. No more
    /// state changes in one step than the machine has states: past that it stops, clears its target and says so with
    /// `Event::Fault`.
    pub fn step(&mut self, dt: f64, values: &mut Values) -> Vec<Event> {
        self.step_with(dt, values, &no_door_busy)
    }

    /// `step` with the caller's answer for the doors: a `DoorClientRequest` whose `door[1]` is set counts as running while
    /// `door_busy(door)` says its door is busy (its IsFinished, 0x1443aa5d0: `door[1] == 0 || !DoorClient.busy()`), so a
    /// waiting transition out of its state waits for the door. The machine knows nothing of other machines: the answer is
    /// the caller's whole part.
    pub fn step_with(&mut self, dt: f64, values: &mut Values, door_busy: &dyn Fn([u16; 2]) -> bool) -> Vec<Event> {
        let dt = if dt > 0.0 { dt } else { 0.0 };
        let frames = ((dt / MAX_FRAME_S).ceil() as usize).clamp(1, MAX_FRAMES);
        let h = dt / frames as f64;
        let (mut events, mut changes) = (Vec::new(), 0);
        for k in 0..frames {
            if !self.frame(h, values, &mut events, &mut changes, door_busy) || (k + 1 < frames && self.settled()) {
                break;
            }
        }
        events
    }

    /// One frame: the clocks, then the route's next transition when it may leave. False after a fault.
    fn frame(&mut self, dt: f64, values: &mut Values, events: &mut Vec<Event>, changes: &mut usize, door_busy: &dyn Fn([u16; 2]) -> bool) -> bool {
        let state = self.current;
        for i in 0..self.runs.len() {
            let act = &self.machine.states[state].activities[i];
            match (&mut self.runs[i], act) {
                (Run::Curve(run), Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c))) => step_curve(run, c, dt, values, events),
                (Run::Wait(left), _) => *left -= dt,
                _ => {}
            }
        }
        let Some((next, wait)) = self.target.filter(|&t| t != self.current).and_then(|t| self.next_hop(t)) else {
            return true;
        };
        if self.waiting.as_ref().map(|w| w.0) != Some(next) {
            let list = if wait { (0..self.runs.len()).filter(|&i| self.running(i, door_busy)).map(|i| (i, self.cycles(i))).collect() } else { Vec::new() };
            self.waiting = Some((next, list));
        }
        let pending: Vec<(usize, u32)> = self.waiting.as_ref().map(|w| w.1.clone()).unwrap_or_default();
        let still: Vec<(usize, u32)> = pending.into_iter().filter(|&(i, c)| self.running(i, door_busy) && self.cycles(i) <= c).collect();
        if still.is_empty() {
            return self.change(next, values, events, changes);
        }
        if let Some(w) = &mut self.waiting {
            w.1 = still;
        }
        true
    }

    /// Whether further frames would change nothing: no transition to take, every wait over, every free-running curve
    /// past its end with its windows done (a driven curve's clock stands with its input).
    fn settled(&self) -> bool {
        if self.target.is_some_and(|t| t != self.current && self.next_hop(t).is_some()) {
            return false;
        }
        let acts = &self.machine.states[self.current].activities;
        self.runs.iter().zip(acts).all(|(run, a)| match (run, a) {
            (Run::Wait(left), _) => *left <= 0.0,
            (Run::Curve(r), Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c))) => {
                let o = &c.options;
                o.driven_input.is_some()
                    || (!o.looping && r.time >= c.duration_s && r.blend >= o.blend_in_s && r.windows.iter().flatten().all(|w| *w == Window::Done))
            }
            _ => true,
        })
    }

    /// The first state of the cheapest route to `target` and whether its transition waits (Dijkstra over the costs,
    /// FUN_140a1fa20).
    fn next_hop(&self, target: usize) -> Option<(usize, bool)> {
        let n = self.machine.states.len();
        let mut dist = vec![u64::MAX; n];
        let mut first: Vec<Option<(usize, bool)>> = vec![None; n];
        let mut done = vec![false; n];
        dist[self.current] = 0;
        while let Some(u) = (0..n).filter(|&i| !done[i] && dist[i] != u64::MAX).min_by_key(|&i| dist[i]) {
            done[u] = true;
            if u == target {
                break;
            }
            for t in &self.machine.states[u].transitions {
                let Some(v) = self.machine.state_index(&t.target) else { continue };
                let d = dist[u] + t.cost as u64;
                if d < dist[v] {
                    dist[v] = d;
                    first[v] = if u == self.current { Some((v, t.wait_for_activities)) } else { first[u] };
                }
            }
        }
        first[target]
    }

    fn running(&self, i: usize, door_busy: &dyn Fn([u16; 2]) -> bool) -> bool {
        let act = &self.machine.states[self.current].activities[i];
        match (&self.runs[i], act) {
            (Run::Door(door), _) => door[1] != 0 && door_busy(*door),
            (Run::Curve(run), Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c))) => {
                let o = &c.options;
                if o.driven_input.is_some() {
                    o.count_when_full
                } else {
                    o.counts_as_running && (o.looping || run.time < c.duration_s || run.blend < o.blend_in_s)
                }
            }
            (Run::Wait(left), _) => *left > 0.0,
            _ => false,
        }
    }

    fn cycles(&self, i: usize) -> u32 {
        match &self.runs[i] {
            Run::Curve(run) => run.cycles,
            _ => 0,
        }
    }

    /// Leaves the current state for `state`, then follows `SetTargetState` jumps, one state at a time (never by
    /// recursion). False, with `Event::Fault`, once this step's state changes outnumber the machine's states.
    fn change(&mut self, state: usize, values: &mut Values, events: &mut Vec<Event>, changes: &mut usize) -> bool {
        let mut next = Some(state);
        while let Some(s) = next {
            *changes += 1;
            if *changes > self.machine.states.len() {
                self.target = None;
                events.push(Event::Fault(format!(
                    "machine {}: more than {} state changes in one step (at {}): stopped",
                    self.machine.name,
                    self.machine.states.len(),
                    self.state()
                )));
                return false;
            }
            self.leave(events);
            next = self.enter(s, values, events);
        }
        true
    }

    /// Enters a state and starts its activities. A `SetTargetState` makes its target the machine's, as a request would;
    /// when no route leads there the state to move to at once is returned.
    fn enter(&mut self, state: usize, values: &mut Values, events: &mut Vec<Event>) -> Option<usize> {
        self.current = state;
        self.waiting = None;
        events.push(Event::Entered(self.machine.states[state].name.clone()));
        let mut runs = Vec::new();
        for (i, a) in self.machine.states[state].activities.iter().enumerate() {
            runs.push(match a {
                Activity::Known(KnownActivity::SetKinematicValueMultiple { values: v }) => {
                    values.extend(v.iter().map(|(k, x)| (k.clone(), *x)));
                    Run::Idle
                }
                Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) => Run::Curve(start_curve(c, &self.machine.owner, values)),
                Activity::Known(KnownActivity::SetLandingGearState { state }) => {
                    events.push(Event::GearState(*state));
                    Run::Idle
                }
                Activity::Known(KnownActivity::DoorClientRequest { door }) => {
                    events.push(Event::DoorRequest { open: door[0] != 0, door: *door });
                    Run::Door(*door)
                }
                Activity::Known(KnownActivity::SetTargetState { target }) => {
                    if let Some(t) = self.machine.state_index(target) {
                        self.target = Some(t);
                    }
                    Run::Idle
                }
                Activity::Known(KnownActivity::WaitSomeGameTime { seconds }) => Run::Wait(*seconds),
                Activity::Known(KnownActivity::PointLightActivity(l) | KnownActivity::SpotLightActivity(l)) => {
                    events.push(Event::LightOn(l.clone()));
                    Run::Idle
                }
                Activity::Known(KnownActivity::AudioState { fields }) => {
                    events.push(Event::AudioState { group: fields.state_group.clone(), state: fields.state.clone() });
                    Run::Idle
                }
                Activity::Other(o) => {
                    events.push(Event::Started { kind: o.kind.clone(), index: i });
                    Run::Idle
                }
            });
        }
        self.runs = runs;
        self.machine.jump_of(state)
    }

    fn leave(&mut self, events: &mut Vec<Event>) {
        let state = &self.machine.states[self.current];
        for (i, (run, a)) in self.runs.iter().zip(&state.activities).enumerate() {
            match (run, a) {
                (Run::Curve(run), Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c))) => {
                    for (cue, ws) in c.sound_cues.iter().zip(&run.windows) {
                        if ws.contains(&Window::Open) {
                            events.push(Event::SoundEnd { cue: cue.cue.clone(), event: cue.events.end.clone(), bones: cue.bones.clone() });
                        }
                    }
                }
                (_, Activity::Known(KnownActivity::PointLightActivity(l) | KnownActivity::SpotLightActivity(l))) => events.push(Event::LightOff(l.clone())),
                (_, Activity::Other(o)) => events.push(Event::Stopped { kind: o.kind.clone(), index: i }),
                _ => {}
            }
        }
        self.runs.clear();
        self.waiting = None;
    }
}

/// A curve activity's Start (FUN_144336e40): the published ramp and where the clock starts.
fn start_curve(c: &CurveActivity, owner: &str, values: &mut Values) -> CurveRun {
    let o = &c.options;
    let start: Vec<f64> = c.curves.iter().map(|k| values.get(curve_name(k)).copied().unwrap_or(0.0)).collect();
    let mut ramp = None;
    let mut time = 0.0;
    if let (Some(first), true) = (c.curves.first().filter(|f| !f.keys.is_empty()), o.driven_input.is_none() && !o.start_at_time_zero) {
        let publish = o.publish_dominant_value && publishes_dominant(&first.keys);
        let current = if publish {
            // get-or-create: made with the first key's value, never overwritten by a later Start
            let var = format!("{owner}_{}_DominantCurveValue", curve_name(first));
            let v = *values.entry(var.clone()).or_insert(first.keys[0][1]);
            ramp = Some((var, dominant_keys(&first.keys, true)));
            v
        } else {
            start.first().copied().unwrap_or(0.0)
        };
        // with a blend-in the clock runs from 0 once the blend is done (`kinpose.blend_in`)
        if o.blend_in_s <= 0.0 {
            time = resume_time(&first.keys, current, false, publish);
        }
    }
    let windows = c
        .sound_cues
        .iter()
        .map(|q| q.windows_s.iter().map(|w| if w[1] < time { Window::Done } else { Window::Waiting }).collect())
        .collect();
    CurveRun { time, cycles: 0, blend: 0.0, start, ramp, windows }
}

/// A curve activity's frame: the blend, else the clock (free-running: rate x dt; driven: its input x duration), then
/// every curve's value, the ramp and the sound windows.
fn step_curve(run: &mut CurveRun, c: &CurveActivity, dt: f64, values: &mut Values, events: &mut Vec<Event>) {
    let o = &c.options;
    let get = |values: &Values, name: &Option<String>| name.as_ref().map(|n| values.get(n).copied().unwrap_or(0.0));
    if let Some(input) = get(values, &o.driven_input) {
        run.time = input.clamp(0.0, 1.0) * c.duration_s;
    } else if run.blend < o.blend_in_s {
        run.blend += dt;
        for (k, s) in c.curves.iter().zip(&run.start) {
            write(values, k, blend_in(*s, k.keys.first().map_or(*s, |k0| k0[1]), run.blend, o.blend_in_s));
        }
        return;
    } else {
        run.time += get(values, &o.rate_input).unwrap_or(1.0) * dt;
        if o.looping && run.time >= c.duration_s {
            run.time = 0.0;
            run.cycles += 1;
            run.windows.iter_mut().flatten().for_each(|w| *w = Window::Waiting);
        }
    }
    let weight = get(values, &o.weight_input);
    for (k, s) in c.curves.iter().zip(&run.start) {
        let v = eval_curve(&k.keys, run.time);
        write(values, k, weight.map_or(v, |w| (v - s) * w + s));
    }
    if let Some((var, keys)) = &run.ramp {
        values.insert(var.clone(), eval_curve(keys, run.time));
    }
    for (cue, ws) in c.sound_cues.iter().zip(run.windows.iter_mut()) {
        for (w, state) in cue.windows_s.iter().zip(ws.iter_mut()) {
            if *state == Window::Waiting && run.time >= w[0] {
                let (cue_name, bones) = (cue.cue.clone(), cue.bones.clone());
                if cue.flag == 1 {
                    events.push(Event::SoundBegin { cue: cue_name, event: cue.events.begin.clone(), bones });
                    *state = Window::Open;
                } else {
                    events.push(Event::SoundSingle { cue: cue_name, event: cue.events.single.clone(), bones });
                    *state = Window::Done;
                }
            }
            if *state == Window::Open && run.time >= w[1] {
                events.push(Event::SoundEnd { cue: cue.cue.clone(), event: cue.events.end.clone(), bones: cue.bones.clone() });
                *state = Window::Done;
            }
        }
    }
}

// ------------------------------------------------------------------------------------- the gear, the doors, the drive

/// The gear leg's "deployed" test (the leg component's vtable +0x30, 0x1430c94e0) on its `SetLandingGearState` value
/// (0 up, 1 doors open, 2 down, 3 rising, 4 raised): down or raised.
pub fn gear_deployed(state: u32) -> bool {
    matches!(state, 2 | 4)
}

/// The gear leg's "moving" test (vtable +0x38, 0x1430c9500): doors open or rising.
pub fn gear_moving(state: u32) -> bool {
    matches!(state, 1 | 3)
}

/// Whether the leg has its physics body: the landing gear manager's update (FUN_14307f8a0) adds it at states 1 and 3 (and
/// while the raised stance, 4, is on), keeps it at 2 and removes it at 0, so a leg touches the ground from doors-open on.
pub fn gear_has_body(state: u32) -> bool {
    matches!(state, 1..=4)
}

/// A door's busy test, as the door client asks the door manager (slot +0x50, 0x142e68ab0, which is FUN_142d393d0 on the
/// door's machine): busy while the machine is not in its own target state (FUN_140a22430 the target, FUN_140a21680 "is in
/// state"). A machine with no target (never asked, or stopped by a fault) is never busy.
pub fn door_busy(door: &Runner) -> bool {
    door.target().is_some_and(|t| t != door.state())
}

/// `GenericParamDriveKinematicActivity`'s target each frame (re/elite/work/csm/logic/notes.md, the per-frame job
/// 0x1442e1c20): the source parameter's place in its range [start, end], clamped to 0..1 (0 for an empty range).
pub fn param_drive_target(source: f64, range: (f64, f64)) -> f64 {
    let (a, b) = range;
    if b != a { ((source - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 }
}

/// ... and its output after a frame of `dt` seconds: toward the target at most `rate` a second (a rate of 0 or less: there
/// at once).
pub fn param_drive_step(out: f64, target: f64, rate: f64, dt: f64) -> f64 {
    let step = if rate > 0.0 { (target - out).clamp(-rate * dt, rate * dt) } else { target - out };
    out + step
}

// ---- The canopy's cracks (`CockpitCracksActivity`; the game runs it from `sidewinder/parts.rs`) ----

/// One stage of the cracks activity's data (re/elite/work/csm/logic/notes.md "CockpitCracksActivity").
#[derive(Clone, Debug, PartialEq)]
pub struct CrackStage {
    pub threshold: f32,
    pub level: f32,
    pub grow: f32,
    pub hold: f32,
    pub audio: Option<String>,
}

/// A running `CockpitCracksActivity`, to the letter in f32 (Start FUN_1434de9a0, the frame FUN_1434dd8b0, its stage pick
/// FUN_1434ddfc0, its reset FUN_1434de570; decompiles in re/elite/work/csm/dec/; its data is `CockpitState`'s Normal in `ship.json`): the stage index steps one at a time
/// towards the last stage whose threshold is below the damage, each stage's clock running up to its grow + hold time (the
/// damage above the stage's threshold) or down to 0; the level ramps from the previous stage's to this one's over its grow
/// time. It is the crack amount the cockpit's render object keeps (+0x48), written every frame the activity runs and left as
/// it is when it stops (Stop, FUN_1434de0e0, sets the Wwise state to none and writes no level). The activity's running-time sums (+0xec, +0xf0, +0xf4) and its second job (FUN_1434d9560: something
/// at the hard-coded thresholds 0.1, 0.2, 0.3) touch nothing drawn: not kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Cracks {
    pub stages: Vec<CrackStage>,
    /// The Wwise state group its stages set (`audio_state_group`).
    pub group: String,
    /// +0xe0: the stage now (-1 none).
    pub index: i32,
    /// +0xe8: the stage's clock.
    pub clock: f32,
    /// +0xc0: the level.
    pub level: f32,
    /// +0xb8: the threshold of the stage last stepped into (Start leaves it as it was: 0 for the object as made, taken).
    threshold: f32,
    /// +0xbc: the damage is above that threshold.
    rising: bool,
}

/// The cracks' Wwise state when no stage is entered (DAT_144e27250; the notes' "None").
pub const CRACKS_NONE: &str = "None";

impl Cracks {
    /// From the activity's fields; None when they aren't the cracks activity's.
    pub fn new(f: &serde_json::Map<String, serde_json::Value>) -> Option<Cracks> {
        let x = |s: &serde_json::Value, k: &str| s.get(k).and_then(serde_json::Value::as_f64).map(|v| v as f32);
        let stages = f
            .get("stages")?
            .as_array()?
            .iter()
            .map(|s| Some(CrackStage { threshold: x(s, "damage_threshold")?, level: x(s, "crack_level")?, grow: x(s, "grow_time")?, hold: x(s, "hold_time")?, audio: s.get("audio_state").and_then(serde_json::Value::as_str).map(str::to_string) }))
            .collect::<Option<Vec<_>>>()?;
        let group = f.get("audio_state_group").and_then(serde_json::Value::as_str)?.to_string();
        Some(Cracks { stages, group, index: -1, clock: 0.0, level: 0.0, threshold: 0.0, rising: true })
    }

    /// Stage `i`: below 0 the activity's own zero record (+0xc8), past the end the last.
    fn stage(&self, i: i32) -> CrackStage {
        let zero = CrackStage { threshold: 0.0, level: 0.0, grow: 0.0, hold: 0.0, audio: None };
        if i < 0 {
            zero
        } else {
            self.stages.get(i as usize).or(self.stages.last()).cloned().unwrap_or(zero)
        }
    }

    /// The last stage whose threshold is strictly below `damage` (-1 none).
    fn target(&self, damage: f32) -> i32 {
        self.stages.iter().rposition(|s| s.threshold < damage).map_or(-1, |i| i as i32)
    }

    /// FUN_1434ddfc0 with its flag 0: straight to the damage's stage, its clock full when rising, its level.
    fn pick(&mut self, damage: f32) {
        self.index = self.target(damage);
        let s = self.stage(self.index);
        self.clock = if self.rising { s.hold + s.grow } else { 0.0 };
        self.level = s.level;
    }

    /// Start (and the reset, FUN_1434de570): no stage, the Wwise state none, then the damage's stage. The Wwise states it
    /// sets.
    pub fn start(&mut self, damage: f32) -> Vec<String> {
        (self.index, self.clock, self.level, self.rising) = (-1, 0.0, 0.0, true);
        self.pick(damage);
        vec![CRACKS_NONE.to_string()]
    }

    /// One frame of `dt` s with `damage`: the Wwise states it sets.
    pub fn step(&mut self, damage: f32, dt: f32) -> Vec<String> {
        let target = self.target(damage);
        self.rising = self.threshold < damage;
        if damage <= 0.0 {
            return if self.level > 0.0 { self.start(damage) } else { Vec::new() };
        }
        let cur = self.stage(self.index);
        let span = cur.hold + cur.grow;
        let dir = if self.rising { 1 } else { -1 };
        let clock = dir as f32 * dt + self.clock;
        self.clock = if clock >= 0.0 { clock.min(span) } else { 0.0 };
        self.level = if cur.grow <= 0.0 {
            cur.level
        } else {
            let prev = self.stage(self.index - 1);
            self.clock.min(cur.grow) / cur.grow * (cur.level - prev.level) + prev.level
        };
        let at_end = if self.rising { self.clock == span } else { self.clock == 0.0 };
        if self.index == target || !at_end {
            return Vec::new();
        }
        self.index += dir;
        let next = self.stage(self.index);
        self.threshold = next.threshold;
        if self.rising {
            self.clock = 0.0;
            next.audio.into_iter().collect()
        } else {
            self.clock = next.hold + next.grow;
            vec![CRACKS_NONE.to_string()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elite_ship::{Machine, Rig};
    use serde_json::Value;

    /// A test data file (`tests/data/elite/`, zlib-compressed JSON written by `re/elite/tools/make_rig_vectors.py`).
    fn data(name: &str) -> Value {
        let path = format!("{}/tests/data/elite/{name}", env!("CARGO_MANIFEST_DIR"));
        let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let j = miniz_oxide::inflate::decompress_to_vec_zlib(&b).expect(name);
        serde_json::from_slice(&j).expect(name)
    }

    fn machines() -> Vec<Machine> {
        serde_json::from_value(data("machines.json.z")).expect("machines.json.z in ship.json's shape")
    }

    fn rows(v: &Value) -> [f64; 16] {
        let f: Vec<f64> = v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        f.try_into().unwrap()
    }

    fn values(v: &Value) -> Values {
        v.as_object().unwrap().iter().map(|(k, x)| (k.clone(), x.as_f64().unwrap())).collect()
    }

    /// The largest difference, and how many numbers were equal bit for bit.
    #[derive(Default)]
    struct Diff {
        max: f64,
        exact: usize,
        count: usize,
    }

    impl Diff {
        fn add(&mut self, a: f64, b: f64) {
            self.max = self.max.max((a - b).abs());
            self.exact += (a.to_bits() == b.to_bits()) as usize;
            self.count += 1;
        }
    }

    #[test]
    fn poses_match_kinpose() {
        let v = data("rig_vectors.json.z");
        let rigs: Vec<Rig> = serde_json::from_value(v["rigs"].clone()).unwrap();
        let mut bones = BoneTable::default();
        for b in v["bones"].as_array().unwrap() {
            bones.insert(b["model"].as_str().unwrap(), b["bone"].as_str().unwrap(), elite_matrix(&rows(&b["bind"])));
        }
        let hull = rigs.iter().find(|r| r.name == "Sidewinder").unwrap();
        let hull_rest = pose(&hull.root, &bones, &Values::new(), DMat4::IDENTITY).unwrap();
        let (mut d, mut base_d) = (Diff::default(), Diff::default());
        for case in v["poses"].as_array().unwrap() {
            let rig = rigs.iter().find(|r| r.name == case["rig"].as_str().unwrap()).unwrap();
            let base = elite_matrix(&rows(&case["base"]));
            if let Some(side) = ["Front", "Left", "Right"].iter().find(|s| rig.name.ends_with(*s)) {
                // the ship item hangs the gear rigs at LandingGearRoot!AttachLandingGear*
                let ours = attach_base(&hull_rest, &bones, "LandingGearRoot", &format!("!AttachLandingGear{side}")).unwrap();
                ours.to_cols_array().iter().zip(base.to_cols_array()).for_each(|(a, b)| base_d.add(*a, b));
            }
            let posed = pose(&rig.root, &bones, &values(&case["values"]), base).unwrap();
            let want = case["nodes"].as_object().unwrap();
            assert_eq!(posed.len(), want.len(), "{}", case["label"]);
            for p in &posed {
                let w = rows(&want[&p.node.name]);
                p.world.to_cols_array().iter().zip(w).for_each(|(a, b)| d.add(*a, b));
            }
        }
        println!("poses: {} numbers, max diff {:e}, {} bit-exact; gear bases max diff {:e}", d.count, d.max, d.exact, base_d.max);
        assert!(d.count > 1000 && d.max < 1e-5 && base_d.max < 1e-5);
    }

    fn curve_activity<'a>(m: &'a Machine, state: &str, index: usize) -> &'a CurveActivity {
        match &m.state(state).unwrap().activities[index] {
            Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) => c,
            other => panic!("{state}: {other:?}"),
        }
    }

    #[test]
    fn curves_match_kinpose() {
        let ms = machines();
        let v = data("curve_vectors.json.z");
        let at = |c: &Value| {
            let m = &ms[c["machine"].as_u64().unwrap() as usize];
            curve_activity(m, c["state"].as_str().unwrap(), c["activity"].as_u64().unwrap() as usize)
        };
        let mut d = Diff::default();
        for c in v["curves"].as_array().unwrap() {
            let keys = &at(c).curves[c["curve"].as_u64().unwrap() as usize].keys;
            for s in c["samples"].as_array().unwrap() {
                d.add(eval_curve(keys, s[0].as_f64().unwrap()), s[1].as_f64().unwrap());
            }
        }
        let mut t = Diff::default();
        for c in v["time_for_value"].as_array().unwrap() {
            t.add(time_for_value(&at(c).curves[0].keys, c["value"].as_f64().unwrap()), c["time"].as_f64().unwrap());
        }
        let mut r = Diff::default();
        for c in v["resume_time"].as_array().unwrap() {
            let zero = c.get("start_at_time_zero").and_then(Value::as_bool).unwrap_or(false);
            let got = resume_time(&at(c).curves[0].keys, c["value"].as_f64().unwrap(), zero, c["publish"].as_bool().unwrap());
            r.add(got, c["time"].as_f64().unwrap());
        }
        for c in v["dominant"].as_array().unwrap() {
            assert_eq!(publishes_dominant(&at(c).curves[0].keys), c["publishes"].as_bool().unwrap());
        }
        println!(
            "curves: {} samples max diff {:e} ({} bit-exact); time_for_value {} max {:e} ({} exact); resume_time {} max {:e} ({} exact)",
            d.count, d.max, d.exact, t.count, t.max, t.exact, r.count, r.max, r.exact
        );
        assert!(d.count > 3000 && d.max < 1e-5 && t.max < 1e-5 && r.max < 1e-5);
    }

    #[test]
    fn blend_in_weights_its_start() {
        assert_eq!(blend_in(2.0, 4.0, 0.0, 0.125), 2.0);
        assert_eq!(blend_in(2.0, 4.0, 0.0625, 0.125), 3.0);
        assert_eq!(blend_in(2.0, 4.0, 1.0, 0.125), 4.0);
        assert_eq!(blend_in(2.0, 4.0, 1.0, 0.0), 2.0);
    }

    // ---------------------------------------------------------------- the machines

    const DT: f64 = 1.0 / 240.0;

    struct Sim {
        runner: Runner,
        values: Values,
        t: f64,
        log: Vec<(f64, Event)>,
    }

    impl Sim {
        fn new(m: &Machine) -> Sim {
            let mut values = Values::new();
            let (runner, ev) = Runner::new(m, &mut values).unwrap();
            Sim { runner, values, t: 0.0, log: ev.into_iter().map(|e| (0.0, e)).collect() }
        }

        /// Steps until the machine rests in `state`.
        fn run_to(&mut self, state: &str) {
            self.runner.request(state).unwrap();
            let until = self.t + 30.0;
            while self.runner.state() != state || self.runner.busy() {
                self.step();
                assert!(self.t < until, "never reached {state}");
            }
        }

        fn step(&mut self) {
            self.t += DT;
            let ev = self.runner.step(DT, &mut self.values);
            self.log.extend(ev.into_iter().map(|e| (self.t, e)));
        }

        /// When the machine last entered `state`.
        fn entered(&self, state: &str) -> f64 {
            let e = Event::Entered(state.into());
            self.log.iter().rev().find(|(_, x)| *x == e).map(|(t, _)| *t).unwrap_or_else(|| panic!("never entered {state}"))
        }

        fn entries(&self) -> Vec<String> {
            self.log.iter().filter_map(|(_, e)| if let Event::Entered(s) = e { Some(s.clone()) } else { None }).collect()
        }

        /// Each gear state set, with the state that set it.
        fn gear_states(&self) -> Vec<(String, u32)> {
            let (mut out, mut state) = (Vec::new(), String::new());
            for (_, e) in &self.log {
                match e {
                    Event::Entered(s) => state = s.clone(),
                    Event::GearState(g) => out.push((state.clone(), *g)),
                    _ => {}
                }
            }
            out
        }
    }

    fn gear(ms: &[Machine], file: &str) -> Machine {
        ms.iter().find(|m| m.file == file && m.name == "LandingGearCSM").unwrap().clone()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= DT + 1e-9
    }

    #[test]
    fn gear_legs_lower_and_rise_in_their_times() {
        let ms = machines();
        for (file, rising) in [("gear_right", 4.2333333), ("gear_left", 4.6333332), ("gear_front", 5.0333333)] {
            let mut s = Sim::new(&gear(&ms, file));
            assert_eq!(s.runner.state(), "Up");
            s.run_to("Down");
            let lowering = s.entered("Down") - s.entered("Lowering");
            assert!(near(lowering, 5.3333335), "{file}: Lowering {lowering} s");
            s.run_to("Up");
            let rise = s.entered("DoorsClosed") - s.entered("Rising");
            assert!(near(rise, rising), "{file}: Rising {rise} s");
            assert_eq!(s.entries(), ["Up", "DoorsOpen", "Lowering", "Down", "Rising", "DoorsClosed", "Up"], "{file}");
            let want = [("Up", 0), ("DoorsOpen", 1), ("Down", 2), ("Rising", 3), ("Up", 0)].map(|(a, b)| (a.to_string(), b));
            assert_eq!(s.gear_states(), want, "{file}");
            let doors: Vec<bool> = s.log.iter().filter_map(|(_, e)| if let Event::DoorRequest { open, .. } = e { Some(*open) } else { None }).collect();
            assert_eq!(doors, [false, true, true, false, false], "{file}: Up, DoorsOpen, Down, DoorsClosed, Up");
        }
    }

    #[test]
    fn sound_cues_fire_at_their_windows() {
        let ms = machines();
        let mut checked = 0;
        for m in ms.iter().filter(|m| m.name == "LandingGearCSM") {
            let mut s = Sim::new(m);
            s.run_to("Down");
            s.run_to("Up");
            for (state, from) in [("Lowering", s.entered("Lowering")), ("Rising", s.entered("Rising"))] {
                let c = curve_activity(m, state, 0);
                for q in &c.sound_cues {
                    // a window running past the curve's end closes when the state is left (gear_right Rising's
                    // Stow_Door_Hinge, 4.4 s of 4.23)
                    let [w0, w1] = q.windows_s[0].map(|w| w.min(c.duration_s));
                    let find = |begin: bool| {
                        s.log
                            .iter()
                            .find(|(t, e)| {
                                *t >= from
                                    && match e {
                                        Event::SoundBegin { cue, .. } => begin && *cue == q.cue,
                                        Event::SoundEnd { cue, .. } => !begin && *cue == q.cue,
                                        _ => false,
                                    }
                            })
                            .unwrap_or_else(|| panic!("{} {state} {}", m.file, q.cue))
                    };
                    let (begin, end) = (find(true), find(false));
                    assert!(near(begin.0 - from, w0) && near(end.0 - from, w1), "{} {state} {}: {} {}", m.file, q.cue, begin.0 - from, end.0 - from);
                    let event = match &begin.1 {
                        Event::SoundBegin { event, .. } => event.clone().unwrap(),
                        _ => unreachable!(),
                    };
                    assert_eq!(event, format!("{}_b_landinggear", q.cue));
                    checked += 1;
                }
            }
        }
        // the front leg's Rising has no cues (Frontier's data)
        assert_eq!(checked, 15);
    }

    #[test]
    fn a_reversed_lowering_rises_from_where_it_stood() {
        let ms = machines();
        let m = gear(&ms, "gear_left");
        let mut s = Sim::new(&m);
        s.runner.request("Down").unwrap();
        while s.runner.state() != "Lowering" {
            s.step();
        }
        let lowering = curve_activity(&m, "Lowering", 0);
        while s.runner.curve_time(0).unwrap() < lowering.duration_s / 2.0 {
            s.step();
        }
        let var = format!("{}_{}_DominantCurveValue", m.owner, lowering.curves[0].value.as_deref().unwrap());
        s.runner.request("Up").unwrap();
        s.step();
        assert_eq!(s.runner.state(), "Rising", "Lowering -> Rising leaves at once");
        // where Lowering's clock left the variable in that step, before Rising started
        let stood = s.values[&var];
        let rising = curve_activity(&m, "Rising", 0);
        let start = s.runner.curve_time(0).unwrap();
        assert_eq!(start, resume_time(&rising.curves[0].keys, stood, false, true));
        // Rising's ramp is Lowering's backwards: it starts half-way through its own time, and the dominant value goes
        // on from where Lowering left it
        assert!((start / rising.duration_s - 0.5).abs() < 0.01, "{start}");
        s.step();
        assert!((s.values[&var] - stood).abs() < 0.01, "{} vs {stood}", s.values[&var]);
        s.run_to("Up");
        assert_eq!(s.entries(), ["Up", "DoorsOpen", "Lowering", "Rising", "DoorsClosed", "Up"]);
    }

    #[test]
    fn hull_doors_take_their_times() {
        let ms = machines();
        let hull = |name: &str| ms.iter().find(|m| m.file == "hull_sidewinder" && m.name == name).unwrap();
        for (name, open, close) in [
            ("LandingGearDoorsFront", 2.5, 2.3666666),
            ("LandingGearDoorsLeft", 2.7333333, 2.5),
            ("LandingGearDoorsRight", 2.6333334, 2.5),
        ] {
            let mut s = Sim::new(hull(name));
            s.run_to("DoorsOpen");
            assert!(near(s.entered("DoorsOpen") - s.entered("OpenDoors"), open), "{name} open");
            s.run_to("DoorsClosed");
            assert!(near(s.entered("DoorsClosed") - s.entered("CloseDoors"), close), "{name} close");
        }
        for name in ["LeftHardpointDoor", "RightHardpointDoor"] {
            let mut s = Sim::new(hull(name));
            s.run_to("DeployedState");
            assert!(near(s.entered("DeployedState") - s.entered("State_Deploying"), 1.0666667), "{name} deploy");
            s.run_to("IdleState");
            assert!(near(s.entered("IdleState") - s.entered("State_Recessing"), 2.5666666), "{name} recess");
        }
    }

    /// A gear leg and its doors as the game runs them: the doors step first, then the leg with "busy" = the doors' machine
    /// not in the state the leg's request asked for; the leg's door requests go to the doors' machine.
    struct Pair {
        leg: Sim,
        door: Sim,
    }

    impl Pair {
        fn new(leg: &Machine, door: &Machine) -> Pair {
            let mut p = Pair { leg: Sim::new(leg), door: Sim::new(door) };
            let first: Vec<Event> = p.leg.log.iter().map(|(_, e)| e.clone()).collect();
            p.ask_doors(&first);
            p
        }

        fn ask_doors(&mut self, events: &[Event]) {
            for e in events {
                if let Event::DoorRequest { open, .. } = e {
                    self.door.runner.request(if *open { "DoorsOpen" } else { "DoorsClosed" }).unwrap();
                }
            }
        }

        fn step(&mut self) {
            self.door.step();
            self.leg.t += DT;
            let door = self.door.runner.state().to_string();
            let busy = move |d: [u16; 2]| door != if d[0] != 0 { "DoorsOpen" } else { "DoorsClosed" };
            let ev = self.leg.runner.step_with(DT, &mut self.leg.values, &busy);
            self.ask_doors(&ev);
            let t = self.leg.t;
            self.leg.log.extend(ev.into_iter().map(|e| (t, e)));
        }

        fn run_to(&mut self, state: &str) {
            self.leg.runner.request(state).unwrap();
            let until = self.leg.t + 30.0;
            while self.leg.runner.state() != state || self.leg.runner.busy() {
                self.step();
                assert!(self.leg.t < until, "never reached {state}");
            }
        }
    }

    /// The door wait (0x1443aa5d0): a leg stays in DoorsOpen until its doors are open, then lowers (doors + 5.33 s in all);
    /// retracting, it reaches Up (gear state 0) only once its doors have closed.
    #[test]
    fn legs_wait_for_their_doors() {
        let ms = machines();
        let hull = |name: &str| ms.iter().find(|m| m.file == "hull_sidewinder" && m.name == name).unwrap().clone();
        for (file, doors, open, close) in [
            ("gear_front", "LandingGearDoorsFront", 2.5, 2.3666666),
            ("gear_left", "LandingGearDoorsLeft", 2.7333333, 2.5),
            ("gear_right", "LandingGearDoorsRight", 2.6333334, 2.5),
        ] {
            let mut p = Pair::new(&gear(&ms, file), &hull(doors));
            p.run_to("Down");
            let held = p.leg.entered("Lowering") - p.leg.entered("DoorsOpen");
            assert!((held - open).abs() <= 2.0 * DT + 1e-9, "{file}: held in DoorsOpen {held} s, doors {open} s");
            assert!(p.leg.entered("Lowering") >= p.door.entered("DoorsOpen"), "{file}: lowered before the doors were open");
            assert!(near(p.leg.entered("Down") - p.leg.entered("Lowering"), 5.3333335), "{file}");
            p.run_to("Up");
            assert!(p.leg.entered("Up") >= p.door.entered("DoorsClosed"), "{file}: up before the doors were closed");
            let held = p.leg.entered("Up") - p.leg.entered("DoorsClosed");
            assert!((held - close).abs() <= 2.0 * DT + 1e-9, "{file}: held in DoorsClosed {held} s, doors {close} s");
            assert_eq!(p.leg.gear_states().last(), Some(&("Up".to_string(), 0)), "{file}");
        }
    }

    /// `step` is `step_with` no door busy: a full cycle gives the same events at the same times either way.
    #[test]
    fn step_is_step_with_no_busy_door() {
        let ms = machines();
        let m = gear(&ms, "gear_left");
        let run = |with: bool| {
            let mut s = Sim::new(&m);
            s.runner.request("Down").unwrap();
            for k in 0..(12.0 / DT) as usize {
                if k == (7.0 / DT) as usize {
                    s.runner.request("Up").unwrap();
                }
                s.t += DT;
                let ev = if with { s.runner.step_with(DT, &mut s.values, &|_| false) } else { s.runner.step(DT, &mut s.values) };
                s.log.extend(ev.into_iter().map(|e| (s.t, e)));
            }
            (s.log, s.runner.state().to_string())
        };
        assert_eq!(run(true), run(false));
    }

    /// The pose a curve state hands over to: along its waiting transitions, past states that set none.
    fn next_pose(m: &Machine, state: &str) -> Option<Values> {
        let mut s = m.state(state)?;
        for _ in 0..m.states.len() {
            let t = s.transitions.iter().find(|t| t.wait_for_activities)?;
            s = m.state(&t.target)?;
            let p = state_pose(s);
            if !p.is_empty() {
                return Some(p);
            }
        }
        None
    }

    #[test]
    fn curves_end_on_the_next_states_pose() {
        let (mut checked, mut off) = (0, Vec::new());
        for m in machines() {
            for st in &m.states {
                for a in &st.activities {
                    let Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) = a else { continue };
                    let Some(pose) = next_pose(&m, &st.name) else { continue };
                    for k in &c.curves {
                        let Some(want) = k.value.as_ref().and_then(|n| pose.get(n)) else { continue };
                        checked += 1;
                        if (eval_curve(&k.keys, c.duration_s) - want).abs() > 1e-3 {
                            off.push(format!("{} {} {}", m.file, st.name, k.value.as_deref().unwrap()));
                        }
                    }
                }
            }
        }
        println!("{checked} curve ends checked against the next state's pose");
        // the one exception is Frontier's data: the front gear's TRight ends LowerShip at -0.223, Down sets 0
        assert_eq!(off, ["gear_front LowerShip Anim_LandingGearFront_TRight"], "{checked} checked");
        assert!(checked > 150, "{checked}");
    }

    #[test]
    fn unknown_states_are_refused() {
        let ms = machines();
        let (mut r, _) = Runner::new(&gear(&ms, "gear_left"), &mut Values::new()).unwrap();
        assert!(matches!(r.request("Sideways"), Err(RigError::NoState { .. })));
    }

    #[test]
    fn jump_states_move_the_machine() {
        let ms = machines();
        let mut s = Sim::new(&gear(&ms, "gear_left"));
        s.runner.request("JumpToShipRaised").unwrap();
        for _ in 0..10 {
            s.step();
        }
        assert_eq!(s.runner.state(), "ShipRaised");
        assert_eq!(s.entries(), ["Up", "JumpToShipRaised", "ShipRaised"]);
    }

    fn hull(ms: &[Machine], name: &str) -> Machine {
        ms.iter().find(|m| m.file == "hull_sidewinder" && m.name == name).unwrap().clone()
    }

    /// `Dying`'s `SetTargetState Dead` routes through `Exploding`: its explosion starts and its 0.15 s wait runs.
    #[test]
    fn dying_explodes_on_the_way_to_dead() {
        let ms = machines();
        let mut s = Sim::new(&hull(&ms, "OperationsMode"));
        s.runner.request("Dying").unwrap();
        for _ in 0..240 {
            s.step();
        }
        assert_eq!(s.entries(), ["Travel", "Dying", "Exploding", "Dead"]);
        let from = s.entered("Exploding");
        assert!(near(s.entered("Dead") - from, 0.15), "{}", s.entered("Dead") - from);
        let started = |kind: &str| s.log.iter().any(|(t, e)| *t == from && matches!(e, Event::Started { kind: k, .. } if k == kind));
        assert!(started("AudioShipExplosionActivity") && started("FireBoneThrownVFXActivity"));
    }

    /// A request for `Exploding` reaches it (through `Dying`, whose own target then takes the machine on to `Dead`).
    #[test]
    fn a_request_for_exploding_reaches_it() {
        let ms = machines();
        let mut s = Sim::new(&hull(&ms, "OperationsMode"));
        s.runner.request("Exploding").unwrap();
        for _ in 0..240 {
            s.step();
        }
        assert_eq!(s.entries(), ["Travel", "Dying", "Exploding", "Dead"]);
        // nothing leads out of Dead: a request it can't reach is refused and changes nothing
        assert!(matches!(s.runner.request("Travel"), Err(RigError::NoRoute { .. })));
        s.step();
        assert_eq!(s.runner.state(), "Dead");
    }

    #[test]
    fn jump_to_high_lands_on_high() {
        let ms = machines();
        let mut s = Sim::new(&hull(&ms, "LandingGearExtension"));
        s.runner.request("JumpToHigh").unwrap();
        s.step();
        assert_eq!(s.entries(), ["Low", "JumpToHigh", "High"]);
    }

    /// A small machine: states `A`, `B`, `C` with the given transitions (target, wait) and `SetTargetState`s.
    fn toy(states: &[(&str, &[&str], Option<&str>)]) -> Machine {
        use crate::elite_ship::{State, Transition};
        let mut m = gear(&machines(), "gear_left");
        m.initial = states[0].0.into();
        m.states = states
            .iter()
            .map(|(name, to, target)| State {
                name: name.to_string(),
                activities: target.iter().map(|t| Activity::Known(KnownActivity::SetTargetState { target: t.to_string() })).collect(),
                transitions: to.iter().map(|t| Transition { target: t.to_string(), cost: 2, wait_for_activities: false, input: 1, kind: 2 }).collect(),
            })
            .collect();
        m
    }

    /// `SetTargetState`s that route to each other ping-pong a state a frame: a step stops after as many changes as
    /// states, with a fault, instead of looping; jumps that loop are refused before anything runs.
    #[test]
    fn target_loops_stop_with_a_fault() {
        let m = toy(&[("A", &["B"], Some("B")), ("B", &["A"], Some("A"))]);
        let mut values = Values::new();
        let (mut r, _) = Runner::new(&m, &mut values).unwrap();
        let ev = r.step(1.0, &mut values);
        assert!(matches!(ev.last(), Some(Event::Fault(_))), "{ev:?}");
        assert_eq!(ev.iter().filter(|e| matches!(e, Event::Entered(_))).count(), 2);
        assert!(r.step(1.0, &mut values).is_empty(), "stopped: no target any more");
        let jumps = toy(&[("A", &[], Some("B")), ("B", &[], Some("A"))]);
        let e = Runner::new(&jumps, &mut Values::new()).unwrap_err();
        assert!(matches!(&e, RigError::BadMachine { why, .. } if why.contains("come back round")), "{e}");
    }

    /// A machine that didn't come through `parse_ship` is checked, never a panic.
    #[test]
    fn broken_machines_are_refused() {
        let ms = machines();
        let base = gear(&ms, "gear_left");
        let with_curve = |f: &dyn Fn(&mut CurveActivity)| {
            let mut m = base.clone();
            let lowering = m.states.iter_mut().find(|s| s.name == "Lowering").unwrap();
            if let Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) = &mut lowering.activities[0] {
                f(c);
            }
            m
        };
        for (m, what) in [
            (with_curve(&|c| c.curves.clear()), "without curves"),
            (with_curve(&|c| c.curves[0].keys.clear()), "has no keys"),
            (toy(&[("A", &["Z"], None)]), "a transition to Z"),
        ] {
            let e = Runner::new(&m, &mut Values::new()).unwrap_err().to_string();
            assert!(e.contains(what), "{e}");
        }
    }

    #[test]
    fn curve_functions_take_short_input() {
        assert_eq!(eval_curve(&[], 1.0), 0.0);
        let one = [[1.0, 2.5, 0.0, 0.0, 0.0, 0.0]];
        assert_eq!((eval_curve(&one, 0.0), eval_curve(&one, 1.0), eval_curve(&one, 9.0)), (2.5, 2.5, 2.5));
        assert_eq!((time_for_value(&[], 1.0), time_for_value(&one, 1.0)), (0.0, 0.0));
        assert!(!publishes_dominant(&[]));
        assert!(dominant_keys(&[], true).is_empty());
        assert_eq!(resume_time(&[], 1.0, false, true), 0.0);
    }

    #[test]
    fn a_negative_step_is_none() {
        let ms = machines();
        let mut s = Sim::new(&gear(&ms, "gear_left"));
        s.runner.request("Down").unwrap();
        while s.runner.state() != "Lowering" {
            s.step();
        }
        s.step();
        let t = s.runner.curve_time(0).unwrap();
        assert!(s.runner.step(-1.0, &mut s.values).is_empty());
        assert!(s.runner.step(f64::NAN, &mut s.values).is_empty());
        assert_eq!(s.runner.curve_time(0), Some(t));
    }

    /// One step of 10 s gives the events of 600 steps of 1/60 s, in the same order.
    #[test]
    fn a_long_step_is_a_run_of_frames() {
        let ms = machines();
        for (m, there, back) in [(gear(&ms, "gear_left"), "Down", "Up"), (hull(&ms, "LandingGearDoorsRight"), "DoorsOpen", "DoorsClosed")] {
            let run = |frames: usize, dt: f64| {
                let mut values = Values::new();
                let (mut r, mut ev) = Runner::new(&m, &mut values).unwrap();
                for target in [there, back] {
                    r.request(target).unwrap();
                    for _ in 0..frames {
                        ev.extend(r.step(dt, &mut values));
                    }
                    assert_eq!(r.state(), target);
                }
                ev
            };
            let long = run(1, 10.0);
            assert_eq!(long, run(600, 1.0 / 60.0), "{}", m.name);
            assert!(long.iter().filter(|e| matches!(e, Event::SoundBegin { .. } | Event::Entered(_))).count() >= 4);
        }
    }

    /// The leg's two tests and its body over the five gear states (0x1430c94e0, 0x1430c9500, FUN_14307f8a0).
    #[test]
    fn the_gear_tests() {
        let row = |s: u32| (gear_deployed(s), gear_moving(s), gear_has_body(s));
        assert_eq!((0..5).map(row).collect::<Vec<_>>(), [(false, false, false), (false, true, true), (true, false, true), (false, true, true), (true, false, true)]);
    }

    /// A door is busy from the request until its machine stands in the state asked for, whatever asked it; never asked or
    /// stopped by a fault, it is never busy (FUN_142d393d0).
    #[test]
    fn a_door_is_busy_until_it_reaches_its_target() {
        let ms = machines();
        let mut values = Values::new();
        let (mut door, _) = Runner::new(&hull(&ms, "LandingGearDoorsRight"), &mut values).unwrap();
        assert!(!door_busy(&door));
        door.request("DoorsOpen").unwrap();
        assert!(door_busy(&door));
        let mut t = 0.0;
        while door.state() != "DoorsOpen" && t < 10.0 {
            door.step(DT, &mut values);
            t += DT;
            assert_eq!(door_busy(&door), door.state() != "DoorsOpen");
        }
        assert!(!door_busy(&door) && near(t, 2.6333334 + DT));
        // Asked back mid-way, it is busy toward the new target.
        door.request("DoorsClosed").unwrap();
        assert!(door_busy(&door));
        let looped = toy(&[("A", &["B"], Some("B")), ("B", &["A"], Some("A"))]);
        let (mut r, _) = Runner::new(&looped, &mut values).unwrap();
        assert!(matches!(r.step(1.0, &mut values).last(), Some(Event::Fault(_))));
        assert!(!door_busy(&r), "a stopped machine is never busy");
    }

    /// The parameter drive: the source's place in its range, clamped, and the output moving toward it at its rate (the
    /// vents': 0.6..0.8 at 0.2 a second).
    #[test]
    fn the_parameter_drive() {
        assert!((param_drive_target(0.7, (0.6, 0.8)) - 0.5).abs() < 1e-12);
        assert_eq!((param_drive_target(0.5, (0.6, 0.8)), param_drive_target(0.9, (0.6, 0.8)), param_drive_target(0.9, (0.6, 0.6))), (0.0, 1.0, 0.0));
        assert!((param_drive_step(0.0, 1.0, 0.2, 0.5) - 0.1).abs() < 1e-12);
        assert_eq!(param_drive_step(0.95, 1.0, 0.2, 0.5), 1.0);
        assert!((param_drive_step(0.3, 0.1, 0.2, 0.5) - 0.2).abs() < 1e-12);
        assert!((param_drive_step(0.3, 0.9, 0.0, 0.01) - 0.9).abs() < 1e-12);
        let at = |fps: f64| (0..(fps * 3.0) as usize).fold(0.0, |o, _| param_drive_step(o, 1.0, 0.2, 1.0 / fps));
        assert!((at(30.0) - 0.6).abs() < 1e-9 && (at(120.0) - 0.6).abs() < 1e-9);
    }

    /// The cracks activity by FUN_1434dd8b0 on the Sidewinder's stages (`ship.json`, CockpitState Normal: 0.2, 0.4, 0.6,
    /// 0.9, levels 0.92, 2.27, 3.67, 10; the first three with no grow and a 1 s hold, the last a 2.5 s grow): damage 0
    /// keeps the canopy whole; at 0.95 the stages come one a second, each setting its Wwise state, the last ramping to 10
    /// over 2.5 s; damage falling back to 0.5 steps down one stage each time the stage's clock has run down to 0 (a lower
    /// stage's clock starts full, at its grow + hold), setting the state to none; damage 0 resets at once.
    #[test]
    fn cracks_follow_the_damage() {
        let stage = |t: f64, l: f64, g: f64, h: f64, a: &str| serde_json::json!({"damage_threshold": t, "crack_level": l, "grow_time": g, "hold_time": h, "audio_state": a});
        let fields = serde_json::json!({"audio_state_group": "CockpitCrack", "damage_param": "Damage", "stages": [
            stage(0.2, 0.92, 0.0, 1.0, "CockpitCrack_01"), stage(0.4, 2.27, 0.0, 1.0, "CockpitCrack_02"),
            stage(0.6, 3.67, 0.0, 1.0, "CockpitCrack_03"), stage(0.9, 10.0, 2.5, 0.0, "CockpitCrack_04")]});
        let mut c = Cracks::new(fields.as_object().unwrap()).unwrap();
        assert!(Cracks::new(serde_json::json!({"stages": []}).as_object().unwrap()).is_none(), "no group: not the cracks' fields");
        assert_eq!((c.stages.len(), c.group.as_str()), (4, "CockpitCrack"));
        assert_eq!(c.start(0.0), ["None"]);
        let dt = 1.0 / 60.0;
        let run = |c: &mut Cracks, damage: f32, secs: f32| -> (Vec<(f32, String)>, Vec<f32>) {
            let (mut states, mut levels) = (Vec::new(), Vec::new());
            for k in 0..(secs / dt).round() as usize {
                states.extend(c.step(damage, dt).into_iter().map(|s| (k as f32 * dt, s)));
                levels.push(c.level);
            }
            (states, levels)
        };
        let (states, levels) = run(&mut c, 0.0, 1.0);
        assert!(states.is_empty() && levels.iter().all(|&l| l == 0.0), "whole at no damage");
        let (states, levels) = run(&mut c, 0.95, 6.0);
        let names: Vec<&str> = states.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(names, ["CockpitCrack_01", "CockpitCrack_02", "CockpitCrack_03", "CockpitCrack_04"]);
        for (k, (t, _)) in states.iter().enumerate() {
            // (A stage steps on the frame its clock reaches the span: a frame late when the sum of the steps falls short.)
            assert!(t - k as f32 >= -dt && t - k as f32 <= (k + 1) as f32 * 1.5 * dt, "stage {} entered at {t} s", k + 1);
        }
        let at = |t: f32| levels[(t / dt) as usize];
        assert_eq!((at(0.5), at(1.5), at(2.5)), (0.92, 2.27, 3.67));
        assert!((at(3.0 + 1.25) - (3.67 + 0.5 * (10.0 - 3.67))).abs() < 0.2, "half way up the last grow: {}", at(4.25));
        assert!((levels.last().unwrap() - 10.0).abs() < 1e-5);
        let (states, _) = run(&mut c, 0.5, 6.0);
        assert_eq!(states.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["None", "None"], "down to the 0.4 stage");
        assert_eq!((c.index, c.level), (1, 2.27));
        let (states, levels) = run(&mut c, 0.0, dt);
        assert_eq!((states.len(), levels[0], c.index), (1, 0.0, -1), "reset at damage 0");
    }
}
