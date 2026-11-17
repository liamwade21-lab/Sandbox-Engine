//! The ship's state machines running, and the pose of its moving parts (ARCHITECTURE.md A6): one `elite_rig::Runner` per
//! machine (`ship.machines`' order) over the rigs' values, asked by the ship's facts (`facts::ShipFacts`) as Elite asks them
//! (`machines.rs`), stepped in real seconds, their events acted on; every rig node's matrix in the ship's space for `draw`,
//! every machine's state for `lights`, the canopy's crack amount for the glass (`Parts::crack`), the gear's real state and
//! its feet for the flight, every cue and audio state for the sound (`Parts::events`). Nothing of the lights or the sound is
//! read here.
//!
//! Each frame: (a) a machine is asked for a state only when what is wanted of it changes (`ask`; Elite's code asks on a
//! change too: the gear manager when its request byte changes, FUN_14307f8a0, the power distributor when its state does);
//! (b) the activities the runner leaves to the caller that this module runs step (the heat vents'
//! `GenericParamDriveKinematicActivity`, `elite_rig::param_drive_step`; the cockpit's `CockpitCracksActivity`, `elite_rig::Cracks`, fed
//! `ShipFacts::damage`); (c) every machine steps, the gear doors and every
//! other machine before the legs, in frames of at most `elite_rig::MAX_FRAME_S`; a leg waiting on its doors asks whether
//! the doors' machine is busy (`elite_rig::door_busy`: not in its own target state, FUN_142d393d0); (d) the events are acted
//! on after each of those frames: a leg's door request reaches its doors' machine at once, in the same frame (Elite's request
//! sets the door's target synchronously); (e) the parts are posed only if a value may have changed. A new ship (a spawn, a
//! parked ship nobody sits in put somewhere else, or a test's `sw put`) is settled: made afresh and run forward silently
//! until every machine rests.

use std::collections::HashSet;

use bevy::math::{DQuat, DVec3, Mat4};
use bevy::prelude::*;
use se_core::catalog::ObjectRef;
use se_core::elite_rig::{self, Cracks, Event, Runner, Values, CRACKS_NONE, MAX_FRAMES, MAX_FRAME_S};
use se_core::elite_ship::Activity;
use serde_json::{Map, Value};

use super::assets::{ShipRig, ShipShape};
use super::facts::ShipFacts;
use super::machines;
use super::{Sidewinder, SidewinderAssets};

/// Settling a new ship: steps of this many seconds, at most `SETTLE_STEPS` of them (the longest route, doors then legs,
/// is under 10 s).
const SETTLE_STEP_S: f64 = 1.0;
const SETTLE_STEPS: usize = 120;

/// The gear's real state: each leg's `SetLandingGearState` (0 up, 1 doors open, 2 down, 3 rising, 4 raised), as the leg
/// component keeps it (+0x1a8, written by 0x1430d3100).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gear {
    legs: Vec<u32>,
}

impl Gear {
    /// Each leg's state, in the legs' machines' order.
    pub fn legs(&self) -> &[u32] {
        &self.legs
    }

    /// Every leg deployed (`elite_rig::gear_deployed`, the leg's own test).
    pub fn deployed(&self) -> bool {
        !self.legs.is_empty() && self.legs.iter().all(|&s| elite_rig::gear_deployed(s))
    }

    /// Any leg moving (`elite_rig::gear_moving`).
    pub fn moving(&self) -> bool {
        self.legs.iter().any(|&s| elite_rig::gear_moving(s))
    }

    /// Every leg down (2).
    pub fn locked_down(&self) -> bool {
        !self.legs.is_empty() && self.legs.iter().all(|&s| s == 2)
    }

    /// Every leg up (0).
    pub fn stowed(&self) -> bool {
        !self.legs.is_empty() && self.legs.iter().all(|&s| s == 0)
    }
}

/// What the machines did this frame for the sound (`Parts::events`): every sound cue (`SoundBegin`, `SoundEnd`,
/// `SoundSingle`: the cue, its event's name and its bones), every `AudioState`, and the audio activities' starts and stops
/// (`Started` / `Stopped` of `Audio*`, `KeepAudio*`, `ShipVoiceReporter`), each with its machine's index in `ship.machines`
/// (a gear leg's cues by its own leg's machine: one emitter per leg).
#[derive(Clone, Debug, PartialEq)]
pub struct ShipEvent {
    pub machine: usize,
    pub event: Event,
}

/// A running `GenericParamDriveKinematicActivity` (re/elite/work/csm/logic/notes.md): its output moves toward the source's
/// place in its range at most `rate` a second (`elite_rig::param_drive_target`, `param_drive_step`), written to the rig value
/// and the parameter.
#[derive(Clone, Debug)]
struct Drive {
    machine: usize,
    index: usize,
    names: Vec<String>,
    source: String,
    range: (f64, f64),
    rate: f64,
    out: f64,
    target: f64,
}

impl Drive {
    /// At Start (0x144336a80): the output starts at the rig value's, else the parameter's, value.
    fn start(machine: usize, index: usize, f: &Map<String, Value>, values: &Values) -> Option<Drive> {
        let s = |k: &str| f.get(k).and_then(Value::as_str).map(str::to_string);
        let x = |k: &str| f.get(k).and_then(Value::as_f64);
        let names: Vec<String> = [s("kinematic_value"), s("output_param")].into_iter().flatten().collect();
        let out = names.iter().find_map(|n| values.get(n).copied()).unwrap_or(0.0);
        Some(Drive { machine, index, names, source: s("source_param")?, range: (x("range_start")?, x("range_end")?), rate: x("rate")?, out, target: out })
    }

    /// One frame with the source's value: true if the output moved.
    fn step(&mut self, v: f64, dt: f64, values: &mut Values) -> bool {
        self.target = elite_rig::param_drive_target(v, self.range);
        let out = elite_rig::param_drive_step(self.out, self.target, self.rate, dt);
        let moved = out != self.out;
        self.out = out;
        for n in &self.names {
            values.insert(n.clone(), self.out);
        }
        moved
    }
}

/// The machines of one ship.
struct Run {
    runners: Vec<Runner>,
    /// The order they step in: every machine but the legs, then the legs (on their doors).
    order: Vec<usize>,
    /// The state each machine was last asked for (`Parts::ask`: asked again only when what is wanted changes).
    asked: Vec<Option<String>>,
    /// The state each machine's events are in (its last `Entered`).
    entered: Vec<String>,
    /// Each leg's doors' machine; each door machine's last request from its leg (open).
    door_of: Vec<Option<usize>>,
    door_open: Vec<Option<bool>>,
    /// The legs' machines (the gear's order) and each one's gear rig (`ShipRig::gear_of_leg`: its foot in `ShipShape`).
    legs: Vec<usize>,
    leg_gear: Vec<Option<usize>>,
    drives: Vec<Drive>,
    /// The running cracks activity: (machine, activity), its state.
    cracks: Option<(usize, usize, Cracks)>,
    /// Nodes a `KModelState` with `restore_on_stop` changed, to give back at its Stop: (machine, activity, node, shown).
    saved: Vec<(usize, usize, usize, bool)>,
    /// The parked ship these machines are for (its place, to tell a new ship).
    ship: (ObjectRef, DVec3, DQuat),
}

impl Run {
    /// Whether machine `i`'s door requests find their doors busy (only a leg has doors).
    fn doors_busy(&self, i: usize) -> bool {
        self.door_of[i].is_some_and(|d| elite_rig::door_busy(&self.runners[d]))
    }
}

#[derive(Resource, Default)]
pub struct Parts {
    /// A test's requests over the table's (`sw parts ask`, `sw lights state`), by machine name.
    pub dev_states: Vec<(String, String)>,
    /// `sw parts log on`: each state change and event, with the machines' clock.
    pub log: bool,
    /// The canopy's crack amount (the cockpit render object's +0x48, gCockpitGlassCrackAmount): the cracks activity's level,
    /// kept when it stops; 0 for a new ship (the glass effects object is made with it 0).
    crack: f32,
    values: Values,
    run: Option<Run>,
    gear: Gear,
    /// Each rig node's matrix in the ship's space (`ShipRig::nodes`' order).
    matrices: Vec<Mat4>,
    /// Each rig node shown (the `KModelState` activities').
    shown: Vec<bool>,
    shown_revision: u64,
    /// Every machine's state now, in `ship.machines`' order (empty with no ship).
    states: Vec<String>,
    /// Counts the changes of the states and of the pose.
    revision: u64,
    events: Vec<ShipEvent>,
    /// Real seconds the machines have run.
    clock: f64,
    warned: HashSet<String>,
}

impl Parts {
    /// A node's matrix now (identity before the first pose).
    pub fn matrix(&self, node: usize) -> Mat4 {
        self.matrices.get(node).copied().unwrap_or(Mat4::IDENTITY)
    }

    /// The rigs' values now.
    pub fn values(&self) -> &Values {
        &self.values
    }

    /// Every machine's state now, in `ship.machines`' order.
    pub fn states(&self) -> &[String] {
        &self.states
    }

    /// The gear's real state.
    pub fn gear(&self) -> &Gear {
        &self.gear
    }

    /// Each leg's foot contact point in the ship's space at its pose now, for every leg that has its physics body
    /// (`elite_rig::gear_has_body`: gear state 1 to 4): the point of the leg's foot that `shape.feet` holds at rest, carried
    /// by its node's matrix this frame (`ShipShape::foot_nodes`). With every leg down (2) these are `shape.feet`.
    pub fn feet(&self, shape: &ShipShape) -> Vec<Vec3> {
        let Some(run) = self.run.as_ref() else { return Vec::new() };
        run.leg_gear
            .iter()
            .zip(&self.gear.legs)
            .filter(|(_, state)| elite_rig::gear_has_body(**state))
            .filter_map(|(g, _)| shape.foot_nodes.get((*g)?))
            .map(|&(node, local)| self.matrix(node).transform_point3(local))
            .collect()
    }

    /// Is the node shown (a `KModelState` may hide it)?
    pub fn shown(&self, node: usize) -> bool {
        self.shown.get(node).copied().unwrap_or(true)
    }

    pub fn shown_revision(&self) -> u64 {
        self.shown_revision
    }

    /// The canopy's crack amount now (`crack`), for the glass.
    pub fn crack(&self) -> f32 {
        self.crack
    }


    /// This frame's events for the sound (`ShipEvent`; the sound's `cues` reads them after `update`).
    pub fn events(&self) -> &[ShipEvent] {
        &self.events
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn warn_once(&mut self, key: String, text: impl FnOnce() -> String) {
        if self.warned.insert(key) {
            warn!("sidewinder parts: {}", text());
        }
    }

    /// The machines made afresh for a new ship and run forward silently until every one rests. Their first requests (the
    /// facts' wanted states, asked once here) are ours: Elite's init asks the gear legs by owner conditions not traced
    /// (FUN_1430bfdc0) and makes `CockpitState` in its state by the canopy (FUN_1430c6130).
    fn make(&mut self, rig: &ShipRig, ship: (ObjectRef, DVec3, DQuat), facts: &ShipFacts) {
        let n = rig.ship.machines.len();
        let mut values = Values::new();
        let mut runners = Vec::with_capacity(n);
        let mut first = Vec::with_capacity(n);
        for m in &rig.ship.machines {
            match Runner::new(m, &mut values) {
                Ok((r, ev)) => {
                    runners.push(r);
                    first.push(ev);
                }
                // (`parse_ship` checks every machine: a refusal here would be a bug. The parts stay as they were.)
                Err(e) => return self.warn_once(format!("make {}", m.name), || format!("the machines can't run: {e}")),
            }
        }
        let legs: Vec<usize> = (0..n).filter(|&i| rig.is_leg(i)).collect();
        let mut door_of = vec![None; n];
        for &l in &legs {
            door_of[l] = machines::door_of_leg(rig, l);
        }
        let order = (0..n).filter(|i| !legs.contains(i)).chain(legs.iter().copied()).collect();
        let entered = runners.iter().map(|r| r.state().to_string()).collect();
        let leg_gear = legs.iter().map(|&l| rig.gear_of_leg(l)).collect();
        self.run = Some(Run { runners, order, asked: vec![None; n], entered, door_of, door_open: vec![None; n], legs: legs.clone(), leg_gear, drives: Vec::new(), cracks: None, saved: Vec::new(), ship });
        self.values = values;
        self.crack = 0.0;
        self.gear = Gear { legs: vec![0; legs.len()] };
        self.shown = vec![true; rig.nodes.len()];
        self.shown_revision += 1;
        for (i, ev) in first.into_iter().enumerate() {
            self.act(rig, facts, i, ev, true);
        }
        for _ in 0..SETTLE_STEPS {
            self.frame(rig, facts, SETTLE_STEP_S, true);
            if self.settled() {
                break;
            }
        }
        info!("sidewinder parts: {n} machines made and settled, the gear {:?}", self.gear.legs);
    }

    /// Every machine at rest: at its target, nothing running (a leg's doors counted), every drive at its target.
    fn settled(&self) -> bool {
        let Some(run) = self.run.as_ref() else { return true };
        run.drives.iter().all(|d| d.out == d.target)
            && run.runners.iter().enumerate().all(|(i, r)| {
                let busy = run.doors_busy(i);
                r.target().is_none_or(|t| t == r.state()) && !r.busy_with(&|_| busy)
            })
    }

    /// What is wanted of machine `i` now: a test's state, else the table's (`machines::wanted`), else (a gear door) its
    /// leg's last request.
    fn wanted<'a>(&'a self, rig: &'a ShipRig, i: usize, facts: &ShipFacts) -> Option<&'a str> {
        let run = self.run.as_ref()?;
        let m = rig.ship.machines.get(i)?;
        let dev = self.dev_states.iter().find(|(name, s)| name.eq_ignore_ascii_case(&m.name) && m.state(s).is_some()).map(|(_, s)| s.as_str());
        let door = || run.door_open[i].zip(machines::door_states(rig, i)).map(|(open, (o, c))| if open { o } else { c });
        dev.or_else(|| machines::wanted(rig, i, facts)).or_else(door)
    }

    /// Asks machine `i` for what is wanted of it, if that changed since it was last asked: true if it was asked.
    fn ask(&mut self, rig: &ShipRig, i: usize, facts: &ShipFacts) -> bool {
        let Some(want) = self.wanted(rig, i, facts).map(str::to_string) else { return false };
        let Some(run) = self.run.as_mut() else { return false };
        if run.asked[i].as_deref() == Some(want.as_str()) {
            return false;
        }
        let asked = run.runners[i].request(&want);
        run.asked[i] = Some(want.clone());
        match asked {
            Ok(()) => true,
            Err(e) => {
                if self.warned.insert(format!("ask {i} {want}")) {
                    info!("sidewinder parts: {e}");
                }
                false
            }
        }
    }

    /// One frame of `dt` real seconds (the module's (a) to (d)); true if a value may have changed. Quiet: a settling's
    /// (no log, no events for the sound).
    fn frame(&mut self, rig: &ShipRig, facts: &ShipFacts, dt: f64, quiet: bool) -> bool {
        if self.run.is_none() {
            return false;
        }
        let mut moved = false;
        // (a) The requests, on a change.
        for i in 0..rig.ship.machines.len() {
            moved |= self.ask(rig, i, facts);
        }
        // (b) The drives.
        let mut unfed = Vec::new();
        if let Some(run) = self.run.as_mut() {
            for d in &mut run.drives {
                match facts.param(&d.source) {
                    Some(v) => moved |= d.step(v, dt, &mut self.values),
                    None => unfed.push(d.source.clone()),
                }
            }
        }
        for s in unfed {
            self.warn_once(format!("drive {s}"), || format!("nothing feeds the drive's parameter {s}: it stands still"));
        }
        // (b) The cracks.
        let damage = facts.damage;
        if let Some((machine, _, cracks)) = self.run.as_mut().and_then(|r| r.cracks.as_mut()) {
            let (machine, group) = (*machine, cracks.group.clone());
            let states = cracks.step(damage, dt as f32);
            self.crack = cracks.level;
            self.post_cracks(machine, &group, states, quiet);
        }
        // (c), (d) The machines in frames, the doors and every other before the legs, each frame's events acted on.
        let frames = ((dt / MAX_FRAME_S).ceil() as usize).clamp(1, MAX_FRAMES);
        let h = dt / frames as f64;
        for k in 0..frames {
            let Some(run) = self.run.as_mut() else { return moved };
            let mut all = Vec::with_capacity(run.order.len());
            for o in 0..run.order.len() {
                let i = run.order[o];
                let busy = run.doors_busy(i);
                let answer = |_: [u16; 2]| busy;
                moved |= run.runners[i].busy_with(&answer);
                let ev = run.runners[i].step_with(h, &mut self.values, &answer);
                moved |= !ev.is_empty();
                all.push((i, ev));
            }
            for (i, ev) in all {
                self.act(rig, facts, i, ev, quiet);
            }
            if k + 1 < frames && self.settled() {
                break;
            }
        }
        moved
    }

    /// Acts on machine `i`'s events (quiet: a settling's, no log or events for the sound).
    fn act(&mut self, rig: &ShipRig, facts: &ShipFacts, i: usize, events: Vec<Event>, quiet: bool) {
        let name = &rig.ship.machines[i].name;
        for e in events {
            if self.log && !quiet {
                info!("sidewinder parts: {:.3} s {name} ({i}): {e:?}", self.clock);
            }
            let Some(run) = self.run.as_mut() else { return };
            match &e {
                Event::Entered(s) => run.entered[i] = s.clone(),
                Event::GearState(g) => {
                    if let Some(k) = run.legs.iter().position(|&l| l == i) {
                        self.gear.legs[k] = *g;
                    }
                }
                // The doors are asked at once (Elite's request sets their target synchronously) and take the transition they
                // may take at once in this frame: a step of no time, no clock moving (ours: which of the two machines Elite
                // updates first in a frame was not traced; this way the doors' clock starts with the leg's state).
                Event::DoorRequest { open, .. } => {
                    if let Some(d) = run.door_of[i] {
                        run.door_open[d] = Some(*open);
                        if self.ask(rig, d, facts) {
                            if let Some(run) = self.run.as_mut() {
                                let ev = run.runners[d].step_with(0.0, &mut self.values, &|_| false);
                                self.act(rig, facts, d, ev, quiet);
                            }
                        }
                    }
                }
                Event::SoundBegin { .. } | Event::SoundEnd { .. } | Event::SoundSingle { .. } | Event::AudioState { .. } if !quiet => {
                    self.events.push(ShipEvent { machine: i, event: e.clone() });
                }
                Event::Started { kind, index } | Event::Stopped { kind, index } => {
                    let start = matches!(e, Event::Started { .. });
                    let fields = match rig.ship.machines[i].state(&run.entered[i]).and_then(|s| s.activities.get(*index)) {
                        Some(Activity::Other(o)) => o.fields.clone(),
                        _ => Map::new(),
                    };
                    match kind.as_str() {
                        "KModelState" => self.model_state(rig, i, *index, &fields, start),
                        "GenericParamDriveKinematicActivity" if start => match Drive::start(i, *index, &fields, &self.values) {
                            Some(d) => run.drives.push(d),
                            None => self.warn_once(format!("drive {i}"), || format!("{name}: a drive without its fields")),
                        },
                        "GenericParamDriveKinematicActivity" => run.drives.retain(|d| (d.machine, d.index) != (i, *index)),
                        "CockpitCracksActivity" if start => match Cracks::new(&fields) {
                            Some(mut c) => {
                                let (group, states) = (c.group.clone(), c.start(facts.damage));
                                self.crack = c.level;
                                run.cracks = Some((i, *index, c));
                                self.post_cracks(i, &group, states, quiet);
                            }
                            None => self.warn_once(format!("cracks {i}"), || format!("{name}: a cracks activity without its fields")),
                        },
                        // (Stop, FUN_1434de0e0: the Wwise state back to none; the level stays where it was.)
                        "CockpitCracksActivity" => {
                            if let Some((_, _, c)) = run.cracks.take_if(|(m, a, _)| (*m, *a) == (i, *index)) {
                                self.post_cracks(i, &c.group, vec![CRACKS_NONE.to_string()], quiet);
                            }
                        }
                        k if k.starts_with("Audio") || k.starts_with("KeepAudio") || k == "ShipVoiceReporter" => {
                            if !quiet {
                                self.events.push(ShipEvent { machine: i, event: e.clone() });
                            }
                        }
                        k => {
                            if self.warned.insert(format!("kind {k}")) {
                                let first = rig.ship.machines[i].states.iter().find(|s| s.activities.iter().any(|a| a.type_name() == k)).map_or("", |s| s.name.as_str());
                                info!("sidewinder parts: {k} activities are not run (first: {name}, {first})");
                            }
                        }
                    }
                }
                // (A machine stopped by a fault has no target: as a door it is never busy again, so no leg waits on it.)
                Event::Fault(why) => {
                    let why = why.clone();
                    self.warn_once(format!("fault {i}"), || why);
                }
                _ => {}
            }
        }
    }

    /// The Wwise states the cracks activity of machine `i` set, as the sound's events (quiet: a settling's, none).
    fn post_cracks(&mut self, i: usize, group: &str, states: Vec<String>, quiet: bool) {
        if quiet {
            return;
        }
        for state in states {
            if self.log {
                info!("sidewinder parts: {:.3} s the cracks set {group} to {state}, the crack amount {:.2}", self.clock, self.crack);
            }
            self.events.push(ShipEvent { machine: i, event: Event::AudioState { group: group.to_string(), state } });
        }
    }

    /// A `KModelState` (re/elite/work/csm/logic/notes.md): at Start its nodes are shown or hidden; at Stop given back only
    /// with `restore_on_stop`.
    fn model_state(&mut self, rig: &ShipRig, i: usize, index: usize, f: &Map<String, Value>, start: bool) {
        let Some(run) = self.run.as_mut() else { return };
        let before = self.shown.clone();
        if start {
            let visible = f.get("visible").and_then(Value::as_u64).unwrap_or(0) != 0;
            let restore = f.get("restore_on_stop").and_then(Value::as_u64).unwrap_or(0) != 0;
            let names: Vec<&str> = f.get("node_names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            for node in names.iter().filter_map(|n| rig.machine_node(i, n)) {
                if restore {
                    run.saved.push((i, index, node, self.shown[node]));
                }
                self.shown[node] = visible;
            }
        } else {
            for &(_, _, node, was) in run.saved.iter().filter(|s| (s.0, s.1) == (i, index)) {
                self.shown[node] = was;
            }
            run.saved.retain(|s| (s.0, s.1) != (i, index));
        }
        if self.shown != before {
            self.shown_revision += 1;
        }
    }

    /// Every node posed with the values now (only when a value may have changed).
    fn repose(&mut self, rig: &ShipRig) {
        match rig.pose(&self.values) {
            Ok(m) => {
                self.matrices = m;
                self.revision += 1;
            }
            Err(e) => self.warn_once("pose".to_string(), || format!("the parts can't be posed: {e}")),
        }
    }

    /// `sw parts`: every machine's state, target and whether it is busy; the gear; the drives.
    fn describe(&self, rig: &ShipRig) {
        let Some(run) = self.run.as_ref() else { return info!("sidewinder parts: no machines (no ship)") };
        for (i, (m, r)) in rig.ship.machines.iter().zip(&run.runners).enumerate() {
            let doors = run.doors_busy(i);
            let busy = r.busy_with(&|_| doors);
            info!("sidewinder parts: {i:2} {} ({}): {}{}{}", m.name, m.file, r.state(), r.target().filter(|t| *t != r.state()).map(|t| format!(" -> {t}")).unwrap_or_default(), if busy { ", busy" } else { "" });
        }
        let g = self.gear();
        info!("sidewinder parts: the gear {:?} (deployed {}, moving {}, locked down {}, stowed {}) at {:.3} s", g.legs(), g.deployed(), g.moving(), g.locked_down(), g.stowed(), self.clock);
        for d in &run.drives {
            info!("sidewinder parts: drive {} of {}: {:.3} -> {:.3}", d.names.join("/"), d.source, d.out, d.target);
        }
        info!("sidewinder parts: the crack amount {:.3} (the cracks activity {})", self.crack, run.cracks.as_ref().map_or("stopped".to_string(), |(_, _, c)| format!("at stage {}, clock {:.2} s", c.index, c.clock)));
    }
}

/// Runs the machines each frame and poses the parts when a value changed.
pub(super) fn update(assets: Res<SidewinderAssets>, facts: Res<ShipFacts>, ship: Res<Sidewinder>, time: Res<Time<Real>>, mut parts: ResMut<Parts>) {
    let rig = &assets.rig;
    let parts = &mut *parts;
    parts.events.clear();
    let Some(p) = ship.parked.as_ref() else {
        if parts.run.take().is_some() {
            parts.states.clear();
            parts.revision += 1;
        }
        return;
    };
    let at = (p.obj, p.pos, p.rot);
    let new_ship = facts.settle || parts.run.as_ref().is_none_or(|r| !facts.seated && r.ship != at);
    let moved = if new_ship {
        parts.make(rig, at, &facts);
        true
    } else {
        let dt = time.delta_secs_f64();
        parts.clock += dt;
        if let Some(r) = parts.run.as_mut() {
            r.ship = at;
        }
        parts.frame(rig, &facts, dt, false)
    };
    if let Some(run) = parts.run.as_ref() {
        if parts.states.len() != run.runners.len() || parts.states.iter().zip(&run.runners).any(|(s, r)| s != r.state()) {
            parts.states = run.runners.iter().map(|r| r.state().to_string()).collect();
            parts.revision += 1;
        }
    }
    if moved {
        parts.repose(rig);
    }
}

/// `sw parts`, `sw parts log on|off`, `sw parts ask MACHINE STATE|auto`.
pub fn dev_command(world: &mut World, args: &str) {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => {
            let rig = world.get_resource::<SidewinderAssets>().map(|a| a.rig.clone());
            match rig {
                Some(rig) => world.resource::<Parts>().describe(&rig),
                None => info!("sidewinder parts: the ship's files aren't read"),
            }
        }
        ["log", w] => world.resource_mut::<Parts>().log = *w != "off",
        ["ask", machine, "auto"] => world.resource_mut::<Parts>().dev_states.retain(|(m, _)| !m.eq_ignore_ascii_case(machine)),
        ["ask", machine, rest @ ..] if !rest.is_empty() => {
            let mut parts = world.resource_mut::<Parts>();
            parts.dev_states.retain(|(m, _)| !m.eq_ignore_ascii_case(machine));
            parts.dev_states.push((machine.to_string(), rest.join(" ")));
        }
        _ => warn!("sw parts [log on|off] [ask MACHINE STATE|auto]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> super::super::assets::SidewinderAssets {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        super::super::assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets
    }

    fn rig() -> std::sync::Arc<ShipRig> {
        files().rig
    }

    const SHIP: (ObjectRef, DVec3, DQuat) = (ObjectRef::Star(0), DVec3::ZERO, DQuat::IDENTITY);

    /// A parked, powered ship with a cool radiator running (heat 0.3).
    fn facts(gear_down: bool) -> ShipFacts {
        let dissipation = elite_sim::heat::visual_dissipation(0.3, true);
        ShipFacts { powered: true, landed: true, gear_down, canopy_intact: true, heat: 0.3, radiator: true, dissipation, ..Default::default() }
    }

    /// The machines run at `dt` a frame: each machine's state entries with their times, the events, every frame's values.
    struct Sim {
        parts: Parts,
        dt: f64,
        t: f64,
        entries: Vec<(f64, usize, String)>,
        events: Vec<ShipEvent>,
        frames: Vec<Values>,
    }

    impl Sim {
        fn new(rig: &ShipRig, gear_down: bool, dt: f64) -> Sim {
            let mut parts = Parts::default();
            parts.make(rig, SHIP, &facts(gear_down));
            parts.repose(rig);
            Sim { parts, dt, t: 0.0, entries: Vec::new(), events: Vec::new(), frames: Vec::new() }
        }

        fn state(&self, i: usize) -> &str {
            self.parts.run.as_ref().unwrap().runners[i].state()
        }

        /// Frames with the gear commanded `down` for `secs`.
        fn run(&mut self, rig: &ShipRig, down: bool, secs: f64) {
            let n = rig.ship.machines.len();
            for _ in 0..(secs / self.dt).round() as usize {
                let before: Vec<String> = (0..n).map(|i| self.state(i).to_string()).collect();
                self.t += self.dt;
                if self.parts.frame(rig, &facts(down), self.dt, false) {
                    self.parts.repose(rig);
                }
                self.events.append(&mut self.parts.events);
                for (i, b) in before.iter().enumerate() {
                    if self.state(i) != b {
                        self.entries.push((self.t, i, self.state(i).to_string()));
                    }
                }
                self.frames.push(self.parts.values.clone());
            }
        }

        /// When machine `i` last entered `state`.
        fn at(&self, i: usize, state: &str) -> f64 {
            self.entries.iter().rev().find(|(_, m, s)| *m == i && s == state).map(|e| e.0).unwrap_or_else(|| panic!("machine {i} never entered {state}"))
        }

        /// The largest change of any value between two frames.
        fn largest_step(&self) -> f64 {
            self.frames.windows(2).flat_map(|w| w[1].iter().map(|(k, v)| (v - w[0].get(k).copied().unwrap_or(*v)).abs())).fold(0.0, f64::max)
        }

        fn cue(&self, name: &str) -> bool {
            self.events.iter().any(|e| matches!(&e.event, Event::SoundBegin { cue, .. } | Event::SoundSingle { cue, .. } if cue == name))
        }
    }

    fn side(rig: &ShipRig, leg: usize) -> &'static str {
        ["Front", "Left", "Right"].into_iter().find(|s| rig.ship.machines[leg].owner.ends_with(s)).unwrap()
    }

    fn legs(rig: &ShipRig) -> Vec<usize> {
        (0..rig.ship.machines.len()).filter(|&i| rig.is_leg(i)).collect()
    }

    /// The gear's whole cycle through the runners at 30, 60 and 120 frames a second, each time within a frame of the data's:
    /// the doors open (2.50 / 2.73 / 2.63 s) from the frame the leg asks them, then the legs lower (5.33 s); the legs rise
    /// (4.23 / 4.63 / 5.03 s), then the doors close (2.37 / 2.50 / 2.50 s) and only then is the leg up; the gear's real state
    /// follows; every cue comes in `Parts::events` with its own leg's machine.
    #[test]
    fn the_gear_cycle_in_its_times() {
        let rig = rig();
        for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 120.0] {
            let mut s = Sim::new(&rig, false, dt);
            assert!(s.parts.gear().stowed(), "{:?}", s.parts.gear());
            s.run(&rig, true, 0.5);
            assert!(s.parts.gear().moving() && !s.parts.gear().deployed());
            s.run(&rig, true, 12.0);
            assert!(s.parts.gear().locked_down() && s.parts.gear().deployed(), "{:?}", s.parts.gear());
            s.run(&rig, false, 12.0);
            assert!(s.parts.gear().stowed(), "{:?}", s.parts.gear());
            for leg in legs(&rig) {
                let door = machines::door_of_leg(&rig, leg).unwrap();
                let (open, close, rise) = match side(&rig, leg) {
                    "Front" => (2.5, 2.3666666, 5.0333333),
                    "Left" => (2.7333333, 2.5, 4.6333332),
                    _ => (2.6333334, 2.5, 4.2333333),
                };
                let fps = 1.0 / dt;
                let within = |a: f64, b: f64, what: &str| assert!((a - b).abs() <= dt + 1e-6, "{} at {fps:.0} fps: {what} {a:.4} s, the data's {b:.4}", side(&rig, leg));
                assert_eq!(s.at(door, "OpenDoors"), s.at(leg, "DoorsOpen"), "the doors are asked in the leg's frame");
                within(s.at(door, "DoorsOpen") - s.at(door, "OpenDoors"), open, "doors open");
                within(s.at(leg, "Lowering") - s.at(leg, "DoorsOpen"), open, "held for the doors");
                within(s.at(leg, "Down") - s.at(leg, "Lowering"), 5.3333335, "lowering");
                within(s.at(leg, "DoorsClosed") - s.at(leg, "Rising"), rise, "rising");
                assert_eq!(s.at(door, "CloseDoors"), s.at(leg, "DoorsClosed"), "the doors are asked in the leg's frame");
                within(s.at(door, "DoorsClosed") - s.at(door, "CloseDoors"), close, "doors close");
                within(s.at(leg, "Up") - s.at(leg, "DoorsClosed"), close, "held for the doors");
                assert!(s.at(leg, "Up") >= s.at(door, "DoorsClosed"));
                assert!(s.events.iter().any(|e| e.machine == leg && matches!(e.event, Event::SoundBegin { .. })), "{}: no cue of its own", side(&rig, leg));
            }
            // The cues came with the motion: the legs' and the doors' sounds, ends with them.
            assert!(s.cue("Sidewinder_Deploy_Motor01") && s.cue("DoorClose_Rear"), "{} events", s.events.len());
            assert!(s.events.iter().any(|e| matches!(e.event, Event::SoundEnd { .. })));
        }
    }

    /// The command reversed half way down (Lowering -> Rising leaves at once): the gear ends stowed by the legs' own route;
    /// each leg's dominant ramp (`<owner>_<value>_DominantCurveValue`, the resume rule's) goes on from where it stood; no
    /// value jumps by more than the whole cycle's own steps except in the reversal's frame, where Rising's other curves
    /// start at the time the dominant ramp gives them. That jump is Elite's own: every gear curve's blend-in time is 0 in
    /// the data, so nothing eases it, and Elite's resume rule (0x144333720) solves only the dominant (first) curve for the
    /// time to start at.
    #[test]
    fn a_reversed_cycle_ends_stowed() {
        let rig = rig();
        let legs = legs(&rig);
        let mut whole = Sim::new(&rig, false, 1.0 / 60.0);
        whole.run(&rig, true, 12.0);
        whole.run(&rig, false, 12.0);
        let most = whole.largest_step();
        let mut s = Sim::new(&rig, false, 1.0 / 60.0);
        while s.state(legs[0]) != "Lowering" {
            s.run(&rig, true, s.dt);
        }
        s.run(&rig, true, 2.6);
        let reversal = s.frames.len();
        s.run(&rig, false, 15.0);
        assert!(s.parts.gear().stowed(), "{:?}", s.parts.gear());
        for &leg in &legs {
            let names: Vec<&str> = s.entries.iter().filter(|e| e.1 == leg).map(|e| e.2.as_str()).collect();
            assert_eq!(names, ["DoorsOpen", "Lowering", "Rising", "DoorsClosed", "Up"], "{}", side(&rig, leg));
        }
        let step = |w: &[Values], k: &str| (w[1][k] - w[0].get(k).copied().unwrap_or(w[1][k])).abs();
        for (f, w) in s.frames.windows(2).enumerate() {
            // (The request is taken in the reversal's first frame; Rising's curves write their values from the next.)
            let at_reversal = f == reversal;
            for k in w[1].keys() {
                let limit = if k.ends_with("_DominantCurveValue") || !at_reversal { most * 1.5 + 1e-9 } else { f64::INFINITY };
                assert!(step(w, k) <= limit, "frame {f}: {k} moved {} (the whole cycle's most {most})", step(w, k));
            }
        }
    }

    /// A spawn is settled: standing on its gear, its doors open, with no event and nothing moving after.
    #[test]
    fn a_spawn_is_settled_silently() {
        let rig = rig();
        let mut s = Sim::new(&rig, true, 1.0 / 60.0);
        assert!(s.parts.gear().locked_down(), "{:?}", s.parts.gear());
        assert!(s.parts.events().is_empty());
        for i in legs(&rig) {
            assert_eq!(s.state(i), "Down");
            assert_eq!(s.state(machines::door_of_leg(&rig, i).unwrap()), "DoorsOpen");
        }
        assert!(!s.parts.frame(&rig, &facts(true), s.dt, false), "something moved after the settling");
        assert!(s.parts.events().is_empty());
        s.run(&rig, true, 1.0);
        assert!(s.events.is_empty());
    }

    /// A machine is asked only when what is wanted of it changes: a test's request holds against the table every frame
    /// after, and its machine's own way on is left alone; `auto` gives the table's back.
    #[test]
    fn requests_are_made_on_a_change() {
        let rig = rig();
        let doors = machines::door_of_leg(&rig, legs(&rig)[0]).unwrap();
        let name = rig.ship.machines[doors].name.clone();
        let mut s = Sim::new(&rig, true, 1.0 / 60.0);
        s.parts.dev_states.push((name.clone(), "DoorsClosed".to_string()));
        s.run(&rig, true, 4.0);
        assert_eq!(s.state(doors), "DoorsClosed");
        let asked = s.entries.iter().filter(|e| e.1 == doors).count();
        s.run(&rig, true, 2.0);
        assert_eq!(s.entries.iter().filter(|e| e.1 == doors).count(), asked, "asked again with nothing changed");
        s.parts.dev_states.clear();
        s.run(&rig, true, 4.0);
        assert_eq!(s.state(doors), "DoorsOpen", "auto: the leg's last request back");
    }

    /// A door held by a test (`sw parts ask`) can't hang its leg: the leg waits for the door's own target, so it goes on
    /// once the door stands in the state the test asked for.
    #[test]
    fn a_held_door_does_not_hang_its_leg() {
        let rig = rig();
        let leg = legs(&rig)[0];
        let doors = machines::door_of_leg(&rig, leg).unwrap();
        let mut s = Sim::new(&rig, false, 1.0 / 60.0);
        s.parts.dev_states.push((rig.ship.machines[doors].name.clone(), "DoorsClosed".to_string()));
        s.run(&rig, true, 9.0);
        assert_eq!((s.state(doors), s.state(leg)), ("DoorsClosed", "Down"));
    }

    /// With every leg down the feet are the shape's (within 1 mm); stowed, no leg has its body and there are none; moving,
    /// each leg's foot is where its pose puts it, between the two.
    #[test]
    fn the_feet_follow_the_legs() {
        let a = files();
        let (rig, shape) = (&a.rig, &a.shape);
        let s = Sim::new(rig, true, 1.0 / 60.0);
        let feet = s.parts.feet(shape);
        assert_eq!(feet.len(), shape.feet.len());
        for (f, rest) in feet.iter().zip(&shape.feet) {
            assert!(f.distance(*rest) < 1e-3, "{f} against {rest}");
        }
        let mut up = Sim::new(rig, false, 1.0 / 60.0);
        assert!(up.parts.feet(shape).is_empty());
        up.run(rig, true, 3.5);
        let moving = up.parts.feet(shape);
        assert_eq!(moving.len(), shape.feet.len());
        assert!(moving.iter().zip(&shape.feet).all(|(m, rest)| m.y > rest.y + 0.05), "{moving:?}");
    }

    /// A breached canopy hides the windscreen and shows its broken twin (their `KModelState`s), whole: Normal shows the
    /// windscreen alone; the breach goes through Breaching (5 s) to Canopy Breached, both showing the broken one alone.
    #[test]
    fn the_windscreen_goes_with_the_canopy() {
        let rig = rig();
        let node = |name: &str| rig.nodes.iter().position(|n| n.name == name).unwrap();
        let (screen, broken) = (node("Cockpit_Windscreen"), node("Cockpit_Windscreen_Broken"));
        assert!(rig.nodes[broken].model.is_some(), "the broken windscreen is drawn");
        let mut s = Sim::new(&rig, true, 1.0 / 60.0);
        assert!(s.parts.shown(screen) && !s.parts.shown(broken));
        let state = rig.ship.machines.iter().position(|m| m.name == "CockpitState").unwrap();
        let breached = ShipFacts { canopy_intact: false, ..facts(true) };
        for _ in 0..60 {
            s.parts.frame(&rig, &breached, s.dt, false);
        }
        assert_eq!(s.state(state), "Breaching");
        assert!(!s.parts.shown(screen) && s.parts.shown(broken));
        for _ in 0..360 {
            s.parts.frame(&rig, &breached, s.dt, false);
        }
        assert_eq!(s.state(state), "Canopy Breached");
        assert!(!s.parts.shown(screen) && s.parts.shown(broken));
        let mut parts = Parts::default();
        parts.make(&rig, SHIP, &breached);
        assert!(!parts.shown(screen) && parts.shown(broken), "a ship made breached");
    }

    /// The cracks in the game (their rule is `elite_rig::Cracks`, tested there): the ship's own cracks activity (four stages,
    /// group `CockpitCrack`) runs in Normal; the facts' damage cracks the canopy; the breach stops the activity and the amount
    /// stays whatever the damage after, and the glass reads it (`Parts::crack`).
    #[test]
    fn the_cracks_follow_the_damage() {
        let rig = rig();
        let fields = rig.ship.machines.iter().flat_map(|m| &m.states).flat_map(|s| &s.activities).find_map(|a| match a {
            Activity::Other(o) if o.kind == "CockpitCracksActivity" => Some(o.fields.clone()),
            _ => None,
        });
        let c = Cracks::new(&fields.unwrap()).unwrap();
        assert_eq!((c.stages.len(), c.group.as_str()), (4, "CockpitCrack"));
        let dt = 1.0 / 60.0;
        // In the game: a test's damage cracks the canopy; the breach stops the activity and the amount stays.
        let mut s = Sim::new(&rig, true, dt);
        let cracked = ShipFacts { damage: 0.65, ..facts(true) };
        for _ in 0..240 {
            s.parts.frame(&rig, &cracked, s.dt, false);
        }
        assert_eq!(s.parts.crack(), 3.67);
        let breached = ShipFacts { canopy_intact: false, ..cracked };
        for _ in 0..30 {
            s.parts.frame(&rig, &breached, s.dt, false);
        }
        assert!(s.parts.run.as_ref().unwrap().cracks.is_none(), "stopped by the breach");
        assert!(s.parts.events.iter().any(|e| matches!(&e.event, Event::AudioState { group, state } if group == "CockpitCrack" && state == "None")));
        let whole_again = ShipFacts { damage: 0.0, ..breached };
        for _ in 0..30 {
            s.parts.frame(&rig, &whole_again, s.dt, false);
        }
        assert_eq!(s.parts.crack(), 3.67, "kept after the breach, whatever the damage");
    }
}
