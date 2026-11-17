//! The pilot chair's animation in the game (ARCHITECTURE.md A6): Elite's own motion graph and clips
//! (`se_core::elite_chair::Chair`, the reviewed rules) stepped once a frame by the frame's real seconds with what the ship's
//! facts say (`facts::ShipFacts`, nothing else), its pose held in `ChairPose` for `draw` to skin `cockpit_chair` with.
//! Nothing here draws. The proofs are `re/elite/notes/sidewinder_chair.md` ("Inputs", "Buffets") and `agent_motiongraph.md`
//! (the variables' table, MG 2.6); the game's feeding of each variable is the notes' section "What the game feeds".
//!
//! What the chair is fed (`ChairInputs`), from `ShipFacts`:
//! - `seated`: the seat (`Seat::seated`): the chair template writes `Mode_Ship` when the pilot sits down and `Empty_Ship`
//!   when he gets up (FUN_142e78130, FUN_142e780f0).
//! - `throttle`: Elite's `ForwardReverseThrust`, the output of the throttle step of `processing_flight` (chair template
//!   +0x18): the flight's forward demand (`Pilot::controls().forward`, the same value `elite_sim::flight` reads).
//! - `roll`, `pitch`, `yaw`: Elite's `Roll`, `Pitch`, `Yaw` controls (template +0x38, +0x30, +0x40; FUN_142aa6b40) are the
//!   live control values, i.e. the demands after Elite's own shaping (the mouse's dead zone and linearity, 0x1408F2210, the
//!   pad's dead zone): the flight's demands (`Pilot::controls()`), which are exactly those. Their signs: posing the clips
//!   (`sidewinder_chair.md` "Inputs"; this module's tests) shows Roll +1 is the stick to the left, Pitch +1 the stick
//!   forward, and Yaw +1 the stick's grip turned to the right (`chairmale_rightstick_yaw` at phase 1). The sign conventions
//!   of Elite's controls themselves (the `processing_flight` chain) were not decoded (MG "Still uncertain"), so each is
//!   taken as the physical stick: rolling left is the stick to the left (the flight's roll, right wing up +, as is), nose
//!   down is the stick forward (minus the flight's pitch, nose up +), yawing right is the grip turned right (minus the
//!   flight's yaw, nose left +). Elite's invert settings (template +0x60..+0x70) are off (`Inverted Value="0"` on
//!   the three axes in both bindings the mod ships).
//! - `fire`: Elite's `PrimaryFire`, raw (template +0x28): the primary trigger's key or button held, or a test's `sw fire`.
//! - `velocity_change`: the cockpit point's change of velocity (v + w x r, FUN_1430692a0) in the ship's axes
//!   (`ShipFacts::cockpit_dv`, the change since the last frame), in Elite's axes through `ShipSpace::direction` (the mirror
//!   in z is its own inverse).
//!
//! The outside chair (`ext_cockpit_chair`, drawn when nobody is seated) stays in its bind pose: its skeleton is not the
//! chair's (75 bones, 73 of them at other indices, four of its own: `the_outside_chair_is_rigid`), so `Chair::new` refuses
//! the clips on it (they are matched by name and index). Its vertices hang one bone each on five bones, four of them its
//! own mesh bones under joints the clips move (the throttle lever, the stick's ball, both consoles' bases), so by name a
//! graph could move them; whether Elite runs one on the outside chair was not traced (its item `Ext_Cockpit_Chair`,
//! prototype `LowPolyCockpitChair`, names no motion graph in the notes). Seen only with the seat empty, its graph would
//! be `Empty_Ship`'s base loop, which moves only the button cover visibly (`the_empty_chair_rests_in_its_bind_pose`), and
//! no vertex of the outside chair hangs on its button cover.

use std::sync::Arc;
use std::time::Instant;

use bevy::math::{DQuat, DVec3, Mat4};
use bevy::prelude::*;
use se_core::catalog::ObjectRef;
use se_core::elite_asset::parse_edm;
use se_core::elite_chair::{Chair, ChairError, ChairInputs, ChairSkeleton};
use se_core::elite_ship::{parse_chair_clips, parse_chair_graph, ChairClips, ChairGraph, ShipDataError};

use super::assets::ShipSpace;
use super::facts::ShipFacts;
use super::{Sidewinder, SidewinderAssets};

const CLIPS_FILE: &str = "chair/clips.json";
const CLIPS_BIN: &str = "chair/clips.bin";
const GRAPH_FILE: &str = "chair/graph.json";
/// The bones `sw chair` reports (the chair model's names): the throttle lever (`chairmale_leftstickmove` moves it and its
/// slider), the stick's grip by its trigger's pivot (`chairmale_rightstickmove` tilts the stick under it,
/// `chairmale_rightstick_yaw` turns it) and the seat (the buffets turn the frame under it).
const REPORTED: [(&str, &str); 3] = [("throttle lever", "BN_LHAccelLever_joint"), ("stick's grip", "BN_JoyStickTrigger01_joint"), ("seat", "BN_MainChair_joint")];

/// The chair's files, read and checked at the first spawn (`load`): its clips, its motion graph and its skeleton.
pub struct ChairData {
    skeleton: ChairSkeleton,
    clips: Arc<ChairClips>,
    graph: Arc<ChairGraph>,
}

impl ChairData {
    /// A chair at the graph's first state (nobody has sat in it yet).
    fn chair(&self) -> Result<Chair, ChairError> {
        Chair::new(&self.skeleton, Arc::clone(&self.clips), Arc::clone(&self.graph))
    }
}

/// A refusal of `Chair::new` as the file it names, and why.
fn fault(e: ChairError, model_file: &str) -> (String, String) {
    match e {
        ChairError::Data(ShipDataError::Bad { file, why }) => (file.to_string(), why),
        ChairError::Skeleton(why) => (model_file.to_string(), why),
        ChairError::Clips(why) => (CLIPS_FILE.to_string(), why),
        ChairError::Graph(why) => (GRAPH_FILE.to_string(), why),
    }
}

/// Reads the chair's clips, graph and the skeleton of its model (`model_file`, the seat's chair) through the mods' checked
/// reader, and makes a chair of them once, so whatever `Chair::new` refuses greys the Mods row now, naming the file.
pub(super) fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>, model_file: &str) -> Result<Arc<ChairData>, (String, String)> {
    let bytes = |f: &str| read(f).map_err(|e| (f.to_string(), e));
    let data_fault = |e: ShipDataError| fault(ChairError::Data(e), model_file);
    let edm = parse_edm(&bytes(model_file)?).map_err(|e| (model_file.to_string(), e.to_string()))?;
    let skeleton = ChairSkeleton::from_edm(&edm).map_err(|e| fault(e, model_file))?;
    let clips = parse_chair_clips(&bytes(CLIPS_FILE)?, &bytes(CLIPS_BIN)?).map_err(data_fault)?;
    let graph = parse_chair_graph(&bytes(GRAPH_FILE)?).map_err(data_fault)?;
    let data = ChairData { skeleton, clips: Arc::new(clips), graph: Arc::new(graph) };
    data.chair().map_err(|e| fault(e, model_file))?;
    Ok(Arc::new(data))
}

/// What the chair is fed this frame, from the ship's facts (the module's notes give each one's source).
fn inputs(facts: &ShipFacts, space: &ShipSpace) -> ChairInputs {
    let dv = space.direction(facts.cockpit_dv.as_dvec3());
    ChairInputs {
        seated: facts.seated,
        throttle: facts.throttle as f32,
        roll: facts.roll as f32,
        pitch: -facts.pitch as f32,
        yaw: -facts.yaw as f32,
        fire: facts.fire,
        velocity_change: dv.to_array(),
    }
}

/// The step's cost (real time on this thread), for `sw chair`.
#[derive(Clone, Copy, Debug, Default)]
struct Cost {
    steps: u64,
    total_s: f64,
    most_s: f64,
    last_s: f64,
}

/// The pilot chair's pose now: Elite's motion graph running for the parked ship, `None` without one.
#[derive(Resource, Default)]
pub struct ChairPose {
    /// `sw chair log on`: each state change, cross-fade and clip starting or ending, with the run's time.
    pub log: bool,
    chair: Option<Chair>,
    /// Each reported bone's index in the skeleton (`REPORTED`'s order).
    reported: Vec<Option<usize>>,
    /// The palette last frame, to tell whether the pose changed.
    last: Vec<Mat4>,
    revision: u64,
    cost: Cost,
    /// For the log: the state, the cross-fade and the clips playing after the last step.
    seen: (String, Option<(u32, u32)>, Vec<String>),
    /// The parked ship the chair is for (its body and place): another one nobody sits in (a new spawn) gets a new chair.
    ship: Option<(ObjectRef, DVec3, DQuat)>,
}

/// What `draw` reads. (The drawing's next task skins the chair with these; until then only the tests read them.)
#[allow(dead_code)]
impl ChairPose {
    /// The skinning palette, one matrix per bone of the chair model (`models/cockpit_chair.edm`'s bones, in its order, as
    /// `ChairSkeleton::from_edm` reads them): posed world x inverse bind, in the chair model's space (Elite's axes), so a
    /// vertex of that model skins as `palette[bone] * v` (then the chair's node matrix as today). Empty without a ship.
    pub fn skin_matrices(&self) -> &[Mat4] {
        self.chair.as_ref().map_or(&[], |c| c.skin_matrices())
    }

    /// Each bone's posed world matrix in the chair model's space (Elite's axes). Empty without a ship.
    pub fn bone_matrices(&self) -> &[Mat4] {
        self.chair.as_ref().map_or(&[], |c| c.bone_matrices())
    }

    /// Counts the changes of the pose (a matrix of the palette changed, or the chair came or went).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The graph's state (`Mode_Ship`, `Empty_Ship`, ...; the one being entered during a cross-fade), "" without a ship.
    pub fn state(&self) -> &str {
        self.chair.as_ref().map_or("", |c| c.state())
    }
}

impl ChairPose {
    /// The chair made afresh for a new ship.
    fn make(&mut self, data: &ChairData) {
        match data.chair() {
            Ok(c) => {
                let bones = &data.skeleton.bones;
                self.reported = REPORTED.iter().map(|(_, name)| bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))).collect();
                self.last = c.skin_matrices().to_vec();
                self.seen = (c.state().to_string(), None, Vec::new());
                self.chair = Some(c);
                self.revision += 1;
            }
            // (Checked at load, so this would be a bug: the chair stays in its bind pose.)
            Err(e) => warn!("sidewinder chair: {e}"),
        }
    }

    /// One frame of `dt` real seconds with these inputs; the pose's revision moves only if a matrix changed.
    fn step(&mut self, dt: f32, inputs: &ChairInputs) {
        let Some(chair) = self.chair.as_mut() else { return };
        let t0 = Instant::now();
        chair.step(dt, inputs);
        let spent = t0.elapsed().as_secs_f64();
        let c = &mut self.cost;
        c.steps += 1;
        c.total_s += spent;
        c.most_s = c.most_s.max(spent);
        c.last_s = spent;
        if chair.skin_matrices() != self.last.as_slice() {
            self.last.clear();
            self.last.extend_from_slice(chair.skin_matrices());
            self.revision += 1;
        }
    }

    /// `sw chair log on`: what changed in this step.
    fn log_changes(&mut self, now_s: f64) {
        let Some(chair) = self.chair.as_ref() else { return };
        let (state, fade, clips) = &mut self.seen;
        if chair.state() != state {
            info!("sidewinder chair: {now_s:.3} s: state {state} -> {}", chair.state());
            *state = chair.state().to_string();
        }
        let f = chair.crossfade();
        let pair = f.map(|f| (f.from, f.to));
        if pair != *fade {
            match f {
                Some(f) => info!("sidewinder chair: {now_s:.3} s: cross-fade from state {} to {} over {:.3} s", f.from, f.to, f.duration_s),
                None => info!("sidewinder chair: {now_s:.3} s: cross-fade done"),
            }
            *fade = pair;
        }
        let playing: Vec<String> = chair.playing().into_iter().map(|p| p.clip).collect();
        for p in playing.iter().filter(|p| !clips.contains(p)) {
            info!("sidewinder chair: {now_s:.3} s: {p} starts");
        }
        for p in clips.iter().filter(|p| !playing.contains(p)) {
            info!("sidewinder chair: {now_s:.3} s: {p} ends");
        }
        *clips = playing;
    }

    /// `sw chair`: the state, the clips, the graph's variables, the reported bones and the step's cost.
    fn describe(&self, graph: &ChairGraph) {
        let Some(chair) = self.chair.as_ref() else {
            info!("sidewinder chair: no ship");
            return;
        };
        let fade = chair.crossfade().map_or_else(String::new, |f| format!(", cross-fade {} -> {} at {:.3} of {:.3} s", f.from, f.to, f.elapsed_s, f.duration_s));
        let clips: Vec<String> = chair.playing().iter().map(|p| format!("{} (weight {:.3}, phase {:.3})", p.clip, p.weight, p.phase)).collect();
        let t = &graph.templates;
        let names = [&t.throttle_variable, &t.joystick_weight_variable, &t.joystick_prop_variable, &t.joystick_yaw_variable, &t.trigger_variable, &t.buffet_intensity_variable, &t.buffet_speed_variable];
        let vars: Vec<String> = names.iter().map(|n| format!("{n} {}", chair.variable(n).map_or("-".to_string(), |v| format!("{v:.4}")))).collect();
        let bones: Vec<String> = REPORTED
            .iter()
            .zip(&self.reported)
            .map(|((what, _), i)| match i.and_then(|i| chair.bone_matrices().get(i)) {
                Some(m) => format!("{what} {:.4} {:.4} {:.4}", m.w_axis.x, m.w_axis.y, m.w_axis.z),
                None => format!("{what} absent"),
            })
            .collect();
        let c = &self.cost;
        info!(
            "sidewinder chair: state {}{fade}; playing: {}; variables: {}; bones (the chair model's space, m): {}; revision {}; step {:.4} ms (mean {:.4}, most {:.4} over {} steps)",
            chair.state(),
            clips.join(", "),
            vars.join(", "),
            bones.join(", "),
            self.revision,
            c.last_s * 1e3,
            c.total_s * 1e3 / c.steps.max(1) as f64,
            c.most_s * 1e3,
            c.steps
        );
    }
}

/// Steps the chair once a frame for the parked ship (after `facts::update`); made with a new ship (one nobody sits in that
/// is not where the last was: a spawn), dropped with it.
pub(super) fn update(assets: Res<SidewinderAssets>, facts: Res<ShipFacts>, ship: Res<Sidewinder>, time: Res<Time<Real>>, mut pose: ResMut<ChairPose>) {
    let pose = &mut *pose;
    let Some(p) = ship.parked.as_ref() else {
        pose.ship = None;
        if pose.chair.take().is_some() {
            pose.last.clear();
            pose.revision += 1;
        }
        return;
    };
    let at = (p.obj, p.pos, p.rot);
    if pose.chair.is_none() || (!facts.seated && pose.ship != Some(at)) {
        pose.make(&assets.chair);
    }
    pose.ship = Some(at);
    pose.step(time.delta_secs(), &inputs(&facts, &assets.rig.space));
    if pose.log {
        pose.log_changes(time.elapsed_secs_f64());
    }
}

/// `sw chair`, `sw chair log on|off`.
pub fn dev_command(world: &mut World, args: &str) {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => match world.get_resource::<SidewinderAssets>().map(|a| Arc::clone(&a.chair)) {
            Some(data) => world.resource::<ChairPose>().describe(&data.graph),
            None => info!("sidewinder chair: the ship's files aren't read"),
        },
        ["log", w] => world.resource_mut::<ChairPose>().log = *w != "off",
        _ => warn!("sw chair [log on|off]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::elite_chair::sample_clip;

    /// A bone's position in its ancestor's frame (both the skeleton's indices), from posed world matrices.
    fn relative(world: &[Mat4], ancestor: usize, bone: usize) -> DVec3 {
        world[ancestor].as_dmat4().inverse().transform_point3(world[bone].w_axis.truncate().as_dvec3())
    }

    fn read_mod(rel: &str) -> Result<Vec<u8>, String> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string())
    }

    const MODEL: &str = "models/cockpit_chair.edm";

    fn data() -> Arc<ChairData> {
        load(&read_mod, MODEL).unwrap_or_else(|(f, why)| panic!("{f}: {why}"))
    }

    fn bone(d: &ChairData, name: &str) -> usize {
        d.skeleton.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
    }

    fn parent(d: &ChairData, b: usize) -> usize {
        d.skeleton.bones[b].parent as usize
    }

    /// A pose for a parked ship, stepped `secs` at `hz` steps a second with the same facts each step.
    fn run(pose: &mut ChairPose, facts: &ShipFacts, secs: f32, hz: f32) {
        let space = ShipSpace { lift: 4.718 };
        for _ in 0..(secs * hz).round() as usize {
            pose.step(1.0 / hz, &inputs(facts, &space));
        }
    }

    fn fresh(d: &ChairData) -> ChairPose {
        let mut pose = ChairPose::default();
        pose.make(d);
        pose
    }

    fn seated() -> ShipFacts {
        ShipFacts { seated: true, ..ShipFacts::default() }
    }

    fn most_apart(a: &[Mat4], b: &[Mat4]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (*x - *y).abs().to_cols_array().into_iter().fold(0.0f32, f32::max)).fold(0.0, f32::max)
    }

    /// The chair loads from the mod's files; one damaged file is refused naming it (the Mods row greys), never a panic.
    #[test]
    fn the_chair_loads_and_a_damaged_file_is_refused() {
        let d = data();
        assert_eq!(d.skeleton.bones.len(), 142);
        for (broken, file) in [(CLIPS_FILE, CLIPS_FILE), (GRAPH_FILE, GRAPH_FILE), (CLIPS_BIN, CLIPS_BIN), (MODEL, MODEL)] {
            let read = |rel: &str| if rel == broken { read_mod(rel).map(|b| b[..b.len() / 2 + 1].to_vec()) } else { read_mod(rel) };
            match load(&read, MODEL) {
                Ok(_) => panic!("{broken} cut in half was accepted"),
                Err((f, why)) => assert_eq!(f, file, "{why}"),
            }
        }
        let missing = |rel: &str| if rel == GRAPH_FILE { Err("not found".to_string()) } else { read_mod(rel) };
        assert_eq!(load(&missing, MODEL).err().map(|e| e.0), Some(GRAPH_FILE.to_string()));
    }

    /// Unseated and at rest: a chair nobody has sat in yet (the graph's `Uninitialised`, no clip) has the bind pose's palette
    /// (identity within 1e-4), so nothing jumps when the drawing first skins with it. One he got up from (the empty seat's
    /// `Empty_Ship`) shows its base loop, which is the bind pose within 1e-3 on every bone but one: `chairmale_baseloop` holds
    /// the button cover (`BN_ButtonCover01_joint`, whose mesh bone has vertices) 144 degrees about its x axis from its bind,
    /// in both its frames (the clip's data). The revision counts only real changes.
    #[test]
    fn the_empty_chair_rests_in_its_bind_pose() {
        let d = data();
        let identity = vec![Mat4::IDENTITY; d.skeleton.bones.len()];
        let mut pose = fresh(&d);
        run(&mut pose, &ShipFacts::default(), 3.0, 60.0);
        assert_eq!((pose.state(), pose.skin_matrices().len()), ("Uninitialised", d.skeleton.bones.len()));
        let worst = most_apart(pose.skin_matrices(), &identity);
        assert!(worst < 1e-4, "never sat in: {worst}");
        run(&mut pose, &seated(), 3.0, 60.0);
        run(&mut pose, &ShipFacts::default(), 3.0, 60.0);
        assert_eq!(pose.state(), "Empty_Ship");
        let (cover_joint, cover) = (bone(&d, "BN_ButtonCover01_joint"), bone(&d, "ButtonCover01"));
        for (i, (m, b)) in pose.skin_matrices().iter().zip(&d.skeleton.bones).enumerate() {
            let apart = most_apart(&[*m], &[Mat4::IDENTITY]);
            if i == cover_joint || i == cover {
                let local = d.skeleton.bones[cover_joint].rotation;
                let turned = pose.bone_matrices()[cover_joint].to_scale_rotation_translation().1;
                let parent = pose.bone_matrices()[parent(&d, cover_joint)].to_scale_rotation_translation().1;
                let angle = bevy::math::Quat::from_array(local).angle_between(parent.inverse() * turned).to_degrees();
                assert!((angle - 144.23).abs() < 0.1, "the cover {angle}");
            } else {
                assert!(apart < 1e-3, "got up from: {} {apart}", b.name);
            }
        }
        let rev = pose.revision();
        let palette = pose.skin_matrices().to_vec();
        pose.step(0.0, &ChairInputs::default());
        assert_eq!(pose.skin_matrices(), palette.as_slice());
        assert_eq!(pose.revision(), rev, "a step that changed nothing counted");
        // (Without a ship: nothing to skin with.)
        assert!(ChairPose::default().skin_matrices().is_empty());
    }

    /// Full throttle forward and back move the lever (with its slider, both the clip's bones) by the clip's own amount: its
    /// place in the slider's parent's frame from the clip at phase 1 and 0 (the throttle's 10 taps full).
    #[test]
    fn the_throttle_moves_the_lever_by_the_clips_amount() {
        let d = data();
        let (lever, slider) = (bone(&d, "BN_LHAccelLever_joint"), bone(&d, "BN_LHAccelSlider_joint"));
        assert_eq!(parent(&d, lever), slider);
        let base = parent(&d, slider);
        let clip = d.clips.clip("chairmale_leftstickmove").unwrap();
        let channel = |name: &str| (clip.rot_channels.iter().position(|c| c.bone == name).unwrap(), clip.pos_channels.iter().position(|c| c.bone == name).unwrap());
        let ((sr, sp), (_, lp)) = (channel("BN_LHAccelSlider_joint"), channel("BN_LHAccelLever_joint"));
        let from_clip = |phase: f32| {
            let s = sample_clip(&d.clips, clip, phase, false).unwrap();
            let q = |r: [f32; 4]| bevy::math::DQuat::from_array(r.map(f64::from));
            let p = |v: [f32; 3]| DVec3::from_array(v.map(f64::from));
            p(s.positions[sp]) + q(s.rotations[sr]) * (p(s.positions[lp]) * d.skeleton.bones[lever].scale as f64)
        };
        let at = |throttle: f64| {
            let mut pose = fresh(&d);
            run(&mut pose, &ShipFacts { throttle, ..seated() }, 3.0, 60.0);
            assert_eq!(pose.state(), "Mode_Ship");
            relative(pose.bone_matrices(), base, lever)
        };
        let (forward, back) = (at(1.0), at(-1.0));
        let want = from_clip(1.0) - from_clip(0.0);
        assert!(want.length() > 0.01, "the clip moves the lever {want}");
        assert!((forward - back - want).length() < 1e-5, "moved {} for the clip's {want}", forward - back);
    }

    /// The stick follows the flight's demands the way a stick goes (the chair's space: +x the pilot's right, +y up, +z ahead):
    /// the nose down pushes the grip (its trigger's place) forward, the nose up back, a roll left to the left, a roll right
    /// to the right; a yaw left turns the grip's front (-y of the grip's bone, whose +z runs up the stick) to the left, a yaw
    /// right to the right; let go, it comes back.
    #[test]
    fn the_stick_follows_the_flights_demands() {
        let d = data();
        let (trigger, grip) = (bone(&d, "BN_JoyStickTrigger01_joint"), bone(&d, "BN_JoyStickBall_joint"));
        let settled = |facts: ShipFacts| {
            let mut pose = fresh(&d);
            run(&mut pose, &facts, 3.0, 60.0);
            pose.bone_matrices().to_vec()
        };
        let rest = settled(seated());
        let place = |m: &[Mat4]| m[trigger].w_axis.truncate();
        let front = |m: &[Mat4]| -m[grip].y_axis.truncate();
        let moved = |facts: ShipFacts| place(&settled(facts)) - place(&rest);
        let (down, up) = (moved(ShipFacts { pitch: -1.0, ..seated() }), moved(ShipFacts { pitch: 1.0, ..seated() }));
        let (left, right) = (moved(ShipFacts { roll: 1.0, ..seated() }), moved(ShipFacts { roll: -1.0, ..seated() }));
        assert!(down.z > 0.03 && up.z < -0.03, "pitch: {down} {up}");
        assert!(left.x < -0.03 && right.x > 0.03, "roll: {left} {right}");
        let (yaw_left, yaw_right) = (front(&settled(ShipFacts { yaw: 1.0, ..seated() })), front(&settled(ShipFacts { yaw: -1.0, ..seated() })));
        assert!(front(&rest).x.abs() < 1e-3 && yaw_left.x < -0.1 && yaw_right.x > 0.1, "yaw: {yaw_left} {yaw_right} at rest {}", front(&rest));
        let mut pose = fresh(&d);
        run(&mut pose, &ShipFacts { pitch: -1.0, roll: 1.0, yaw: 1.0, ..seated() }, 3.0, 60.0);
        run(&mut pose, &seated(), 1.0, 60.0);
        assert!(most_apart(pose.bone_matrices(), &rest) < 1e-5);
    }

    /// The trigger and a buffet change the pose, and it settles back to the seated rest pose.
    #[test]
    fn the_trigger_and_a_buffet_come_and_go() {
        let d = data();
        let trigger = bone(&d, "BN_JoyStickTrigger01_joint");
        let seat = bone(&d, "BN_MainChair_joint");
        let mut pose = fresh(&d);
        run(&mut pose, &seated(), 3.0, 60.0);
        let rest = pose.bone_matrices().to_vec();
        // The trigger: In, then Loop while held, Out when let go.
        run(&mut pose, &ShipFacts { fire: true, ..seated() }, 1.0, 60.0);
        let held = most_apart(&pose.bone_matrices()[trigger..=trigger], &rest[trigger..=trigger]);
        assert!(held > 1e-3, "the trigger held moved its bone {held}");
        run(&mut pose, &seated(), 2.0, 60.0);
        assert!(most_apart(&pose.bone_matrices()[trigger..=trigger], &rest[trigger..=trigger]) < 1e-5);
        // A buffet: a hard knock from the left (the cockpit's velocity changed by 60 m/s to the right in one frame).
        pose.step(1.0 / 60.0, &inputs(&ShipFacts { cockpit_dv: Vec3::new(60.0, 0.0, 0.0), ..seated() }, &ShipSpace { lift: 4.718 }));
        let mut most = 0.0f32;
        for _ in 0..30 {
            pose.step(1.0 / 60.0, &inputs(&seated(), &ShipSpace { lift: 4.718 }));
            most = most.max(most_apart(&pose.bone_matrices()[seat..=seat], &rest[seat..=seat]));
        }
        assert!(most > 1e-3, "the buffet moved the seat {most}");
        run(&mut pose, &seated(), 3.0, 60.0);
        assert!(most_apart(&pose.bone_matrices()[seat..=seat], &rest[seat..=seat]) < 1e-5);
    }

    /// 10 s of the same controls at 30, 60 and 120 steps a second end in the same pose.
    #[test]
    fn the_same_pose_at_any_frame_rate() {
        let d = data();
        let facts = ShipFacts { throttle: 0.6, pitch: 0.4, roll: -0.3, yaw: 0.5, fire: true, ..seated() };
        let poses: Vec<Vec<Mat4>> = [30.0, 60.0, 120.0]
            .into_iter()
            .map(|hz| {
                let mut pose = fresh(&d);
                run(&mut pose, &facts, 10.0, hz);
                pose.bone_matrices().to_vec()
            })
            .collect();
        for p in &poses[1..] {
            let apart = most_apart(&poses[0], p);
            assert!(apart < 1e-3, "{apart}");
        }
    }

    /// Non-finite controls and frame times don't poison the pose: it stays finite and comes back to rest.
    #[test]
    fn non_finite_inputs_do_not_poison_the_pose() {
        let d = data();
        let mut pose = fresh(&d);
        run(&mut pose, &seated(), 2.0, 60.0);
        let rest = pose.bone_matrices().to_vec();
        let bad = ShipFacts { throttle: f64::NAN, pitch: f64::INFINITY, roll: f64::NEG_INFINITY, yaw: f64::NAN, cockpit_dv: Vec3::new(f32::NAN, f32::INFINITY, 0.0), ..seated() };
        run(&mut pose, &bad, 1.0, 60.0);
        pose.step(f32::NAN, &inputs(&seated(), &ShipSpace { lift: 4.718 }));
        pose.step(f32::INFINITY, &inputs(&seated(), &ShipSpace { lift: 4.718 }));
        assert!(pose.skin_matrices().iter().all(|m| m.is_finite()));
        run(&mut pose, &seated(), 3.0, 60.0);
        assert!(pose.bone_matrices().iter().all(|m| m.is_finite()));
        assert!(most_apart(pose.bone_matrices(), &rest) < 1e-3);
    }

    /// The outside chair can't be posed by `Chair` with the clips: its skeleton is another (fewer bones, another order, some
    /// of its own); every vertex hangs on one bone, on five bones in all, and none on the button cover.
    #[test]
    fn the_outside_chair_is_rigid() {
        let edm = parse_edm(&read_mod("models/ext_cockpit_chair.edm").unwrap()).unwrap();
        let inside = data();
        let names: Vec<&str> = edm.parts.iter().map(|p| p.name.as_str()).collect();
        let own: Vec<&&str> = names.iter().filter(|n| !inside.skeleton.bones.iter().any(|b| b.name == **n)).collect();
        let reordered = names.iter().enumerate().filter(|(i, n)| inside.skeleton.bones.get(*i).is_none_or(|b| b.name != **n)).count();
        let skeleton = ChairSkeleton::from_edm(&edm).unwrap();
        let refused = Chair::new(&skeleton, Arc::clone(&inside.clips), Arc::clone(&inside.graph)).err();
        let mut used = std::collections::BTreeSet::new();
        let mut shared = 0usize;
        for v in edm.draws.iter().flat_map(|d| d.vertices.iter()).map(|v| v.decode()) {
            shared += usize::from(v.weights.iter().filter(|w| **w > 0.0).count() > 1);
            used.extend(v.bones.iter().zip(v.weights).filter(|(_, w)| *w > 0.0).map(|(b, _)| *b));
        }
        println!("the outside chair: {} bones ({reordered} out of the inside's order, its own {own:?}); the clips on it: {refused:?}; bones its vertices use {used:?}, {shared} vertices on more than one", names.len());
        assert!(names.len() < inside.skeleton.bones.len() && !own.is_empty() && reordered > 0);
        assert!(refused.is_some());
        assert_eq!(shared, 0);
        assert_eq!(used.len(), 5);
        let cover = names.iter().position(|n| n.to_ascii_lowercase().contains("buttoncover"));
        assert!(cover.is_none_or(|c| !used.contains(&(c as u8))), "a vertex on the button cover");
    }

    /// The inside chair's vertices each hang on one bone (what the drawing task skins with).
    #[test]
    fn the_inside_chairs_vertices_hang_on_one_bone() {
        let edm = parse_edm(&read_mod(MODEL).unwrap()).unwrap();
        let mut most = 0usize;
        let mut used = std::collections::BTreeSet::new();
        for v in edm.draws.iter().flat_map(|d| d.vertices.iter()).map(|v| v.decode()) {
            most = most.max(v.weights.iter().filter(|w| **w > 0.0).count());
            used.extend(v.bones.iter().zip(v.weights).filter(|(_, w)| *w > 0.0).map(|(b, _)| *b));
        }
        println!("the inside chair: at most {most} weights a vertex, {} bones used, the highest {:?}", used.len(), used.last());
        assert_eq!(most, 1);
    }
}
