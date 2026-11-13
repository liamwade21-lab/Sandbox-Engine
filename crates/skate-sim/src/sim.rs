//! The facade over the copied simulation: the only new logic of this crate (with `sim_bridge.rs`).
//!
//! One `Sim::step` is exactly what upstream runs per fixed tick, in upstream's order
//! (`input::publish_actions` -> `physics::controls::sample` -> `physics::advance` ->
//! `presentation::capture`), minus the Bevy schedule around it.
use crate::{
    camera::CameraRuntime,
    custom_difficulty,
    difficulty::Difficulty,
    graph_runtime::StockGraphs,
    input::{ControllerInput, platform::{DeviceError, DevicePacket}},
    physics::{GamePhysics, PlayerControls, SkaterRuntime, sim_bridge},
    presentation::Snapshot,
};
use bevy::math::{Mat3, Mat4, Quat, Vec3, Vec4};
use bevy::transform::components::Transform;
use skate_core::{
    input::xbox::XboxState,
    math::Vector3,
    physics::{
        board::BodyId,
        board_world::{
            BoardWorld, WorldTriangle,
            query_metadata::{Bounds, QueryMesh, QueryMetadata, QueryPool},
        },
        collision::TriangleFeature,
        drive_frames::RetailAffineTransform,
        phase::PhysicsEvent,
    },
};
use std::{collections::HashMap, path::Path, time::Duration};

/// Raw controller state, as the XInput/SDL backends hand it to the simulation (before `xbox::convert`).
///
/// Sticks run -1..1 with **+y up** (pushing the right stick down = -1, as XInput reports it; the SDL
/// backend inverts SDL's positive-down y to this before it reaches the game). Triggers 0..1. `buttons`
/// are XInput `wButtons` bits, see [`buttons`]. Values are quantised exactly like a real pad
/// (`i16` sticks, `u8` triggers) so the game's own dead-zone and curve maths see what a pad gives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawPad {
    pub buttons: u16,
    pub triggers: [f32; 2],
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
}

/// XInput `wButtons` bits (`input/platform.rs` `BUTTON_NAMES`).
pub mod buttons {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_STICK: u16 = 0x0040;
    pub const RIGHT_STICK: u16 = 0x0080;
    pub const LEFT_SHOULDER: u16 = 0x0100;
    pub const RIGHT_SHOULDER: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

impl RawPad {
    /// From the integer values a device reports (XInput units, +y up).
    pub fn from_xinput(buttons: u16, triggers: [u8; 2], left: [i16; 2], right: [i16; 2]) -> Self {
        let stick = |v: [i16; 2]| v.map(|v| if v < 0 { f32::from(v) / 32768.0 } else { f32::from(v) / 32767.0 });
        Self {
            buttons,
            triggers: triggers.map(|t| f32::from(t) / 255.0),
            left_stick: stick(left),
            right_stick: stick(right),
        }
    }

    /// The XInput-shaped state the simulation consumes.
    pub fn xbox_state(&self) -> XboxState {
        let axis = |v: f32| -> i16 {
            let v = if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
            if v < 0.0 { (v * 32768.0).round() as i16 } else { (v * 32767.0).round() as i16 }
        };
        let trigger = |v: f32| -> u8 { (if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 } * 255.0).round() as u8 };
        XboxState {
            buttons: self.buttons,
            triggers: self.triggers.map(trigger),
            left: self.left_stick.map(axis),
            right: self.right_stick.map(axis),
        }
    }
}

/// The stock skeleton's bone order and hierarchy (the order of every bone array this crate returns).
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub bone_names: Vec<String>,
    /// Parent bone index per bone, -1 for the root.
    pub parents: Vec<i32>,
    /// Bone indices of the board's trucks and wheels in `bone_names`:
    /// front truck, back truck, front-left, front-right, back-left, back-right wheel.
    pub board_bones: [usize; 6],
}

/// 4x4 column-major matrix, `[column][row]` (glam's `to_cols_array_2d`): columns 0..2 are the basis
/// vectors (right, up, at/forward), column 3 the translation.
pub type Mat4Cols = [[f32; 4]; 4];

/// A camera pose in the sim's Y-up metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraOut {
    pub position: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    /// Where the camera looks (the engine's "At" axis).
    pub forward: [f32; 3],
    /// Bevy-convention rotation (looks along -Z), as `presentation::camera_transform` builds it. xyzw.
    pub rotation: [f32; 4],
    /// Vertical field of view in degrees.
    pub fov_y_degrees: f32,
    /// True when the camera cut this tick (do not interpolate from the previous frame).
    pub cut: bool,
}

/// One completed tick's render snapshot (`presentation::Snapshot`).
#[derive(Clone, Debug)]
pub struct PoseFrame {
    /// The skater's root: animation space -> world (`animated_skeleton.roots.animation_to_world`).
    pub root: Mat4Cols,
    /// Every bone's matrix in animation (model) space, in `Skeleton::bone_names` order, native axes.
    /// World matrix of bone i = `root * bones[i]`.
    pub bones: Vec<Mat4Cols>,
    pub camera: CameraOut,
}

/// The last two completed ticks, to blend at the host frame's overstep alpha like upstream `present`.
#[derive(Clone, Debug)]
pub struct Pose {
    pub previous: PoseFrame,
    pub current: PoseFrame,
    /// True when `previous` was reset to `current` (teleport, camera cut, tick-rate change).
    pub cut: bool,
}

impl Pose {
    /// `animation::present`: root and camera are blended as transforms (lerp / slerp, fov lerp), each bone
    /// as its LOCAL transform against its parent (so limbs turn instead of shearing), then recomposed
    /// into animation-space matrices.
    pub fn blend(&self, alpha: f32, skeleton: &Skeleton) -> PoseFrame {
        let alpha = alpha.clamp(0.0, 1.0);
        let m = |c: &Mat4Cols| Mat4::from_cols_array_2d(c);
        let root = crate::presentation::blend(
            Transform::from_matrix(m(&self.previous.root)),
            Transform::from_matrix(m(&self.current.root)),
            alpha,
        )
        .to_matrix();
        let mut globals: Vec<Mat4> = Vec::with_capacity(self.current.bones.len());
        for (i, current) in self.current.bones.iter().enumerate() {
            let local = |bones: &Vec<Mat4Cols>| -> Option<Transform> {
                let global = m(bones.get(i)?);
                let parent = skeleton.parents.get(i).copied().unwrap_or(-1);
                Some(Transform::from_matrix(if parent >= 0 {
                    m(bones.get(parent as usize)?).inverse() * global
                } else {
                    global
                }))
            };
            let local = match (local(&self.previous.bones), local(&self.current.bones)) {
                (Some(a), Some(b)) => crate::presentation::blend(a, b, alpha).to_matrix(),
                _ => m(current),
            };
            let parent = skeleton.parents.get(i).copied().unwrap_or(-1);
            let global = if parent >= 0 && (parent as usize) < globals.len() { globals[parent as usize] * local } else { local };
            globals.push(global);
        }
        let camera = {
            let a = &self.previous.camera;
            let b = &self.current.camera;
            let t = crate::presentation::blend(camera_transform(a), camera_transform(b), alpha);
            let mut out = camera_out(t, a.fov_y_degrees + (b.fov_y_degrees - a.fov_y_degrees) * alpha);
            out.cut = b.cut;
            out
        };
        PoseFrame { root: root.to_cols_array_2d(), bones: globals.iter().map(|g| g.to_cols_array_2d()).collect(), camera }
    }
}

fn camera_transform(c: &CameraOut) -> Transform {
    Transform { translation: Vec3::from_array(c.position), rotation: Quat::from_array(c.rotation), ..Default::default() }
}

fn camera_out(t: Transform, fov_y_degrees: f32) -> CameraOut {
    CameraOut {
        position: t.translation.to_array(),
        right: (t.rotation * Vec3::X).to_array(),
        up: (t.rotation * Vec3::Y).to_array(),
        forward: (t.rotation * Vec3::NEG_Z).to_array(),
        rotation: t.rotation.to_array(),
        fov_y_degrees,
        cut: false,
    }
}

/// What upstream's scoring HUD shows (the `scoring_runtime` accessors `hud_input` reads, plus the landing results).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    /// Points of the current trick sequence (the big number).
    pub sequence_score: f32,
    pub sequence_active: bool,
    /// Banked line score.
    pub line_score: f32,
    pub multiplier: f32,
    /// Seconds left on the line timer, and the timer's capacity.
    pub line_time: f32,
    pub line_capacity: f32,
    /// The trick label as the HUD receives it, e.g. `ID_TRICK_KICKFLIP 360` (language-table ids).
    pub trick_id: String,
    /// The same label as the HUD draws it (language table, else a readable fallback).
    pub trick_text: String,
    /// Increments each time the trick shown changes.
    pub trick_seq: u32,
    pub clean: bool,
    pub sketchy: bool,
    /// Stance flags as the HUD binds them (regular/switch, nollie/fakie style slots).
    pub stance: [bool; 4],
    pub new_trick: bool,
    pub modified_trick: bool,
    pub close_tricks: bool,
    /// Landing result: counter (increments per landed trick), the landed trick's full label and base id.
    pub landing_seq: u32,
    pub landed_trick_id: String,
    pub landed_trick_text: String,
    pub landed_base_id: String,
    pub landed_spin_degrees: i32,
    pub landed_clean: bool,
    pub landed_sketchy: bool,
    /// Counter of bails.
    pub bail_seq: u32,
}

/// What audio would react to this tick (`game_audio::skate_events::observe`'s inputs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Events {
    pub tick: u64,
    /// `PhysicsEvent::StateChanged` of the tick, as `(from, to)` state names.
    pub state_changes: Vec<(String, String)>,
    /// The skater left an air state for a non-air, non-bail one this tick (derived from the state change:
    /// upstream only emits `StateChanged` events, `Landing`/`Wipeout` are never produced).
    pub landed: bool,
    /// A bail (category 300, `WipeoutGround`) began this tick.
    pub wiped_out: bool,
    /// Ground -> air with a jump in progress (an ollie/pop), derived from the state change and jump strength.
    pub popped: bool,
    /// A push plant began this tick (edge of the animation's push contact, as `observe` logs `AUDIO_EVENT push`).
    pub pushed: bool,
    /// The animation's push contact this tick (level: true on every tick the foot is planted).
    pub push_contact: bool,
    /// Brake foot went down / came up (edges: the animation repeats the events every tick while braking).
    pub brake_down: bool,
    pub brake_up: bool,
    /// The animation's brake event this tick: +1 foot down, -1 foot up, 0 none (level, repeats).
    pub brake_contact: i32,
    /// Largest footstep strength this tick.
    pub footstep: f32,
    pub grind_started: bool,
    pub grind_stopped: bool,
    // Levels, as observe publishes them.
    pub state: String,
    pub ground_speed: f32,
    pub airborne: bool,
    pub grinding: bool,
    /// Audio surface id under the board (`physics::board_surface`).
    pub surface: u32,
    /// Wheels with a ground line hit this tick (0..4).
    pub wheels_touching: u32,
    pub board_position: [f32; 3],
    pub board_velocity: [f32; 3],
}

/// A completed tick's render snapshot plus the exact (undecomposed) root matrix.
#[derive(Clone)]
struct Frame {
    snapshot: Snapshot,
    root: Mat4Cols,
}

pub struct Sim {
    physics: GamePhysics,
    skater: SkaterRuntime,
    controls: PlayerControls,
    graphs: StockGraphs,
    camera: CameraRuntime,
    input: ControllerInput,
    skeleton: Skeleton,
    language: Option<crate::apt_text::TextAssets>,
    history: Option<(Frame, Frame)>,
    history_generation: u64,
    history_period: Duration,
    history_cut: bool,
    events: Events,
    was_grinding: bool,
    was_push: bool,
    last_brake: i32,
}

fn vec3(v: Vector3) -> [f32; 3] { [v.x, v.y, v.z] }

impl Sim {
    /// `root` = the folder holding `private/stock/...`; `difficulty` one of easy|normal|hardcore.
    pub fn load(root: &Path, difficulty: &str) -> Result<Self, String> {
        let difficulty = Difficulty::parse(difficulty)?;
        if !matches!(difficulty, Difficulty::Easy | Difficulty::Normal | Difficulty::Hardcore) {
            return Err(format!("difficulty must be easy, normal or hardcore, not {difficulty:?}"));
        }
        let assets = skate_data::GameAssets::load(root).map_err(|e| e.to_string())?;
        let graphs = StockGraphs::load(root, &assets)?;
        // The map-less load builds upstream's test course; replace its world and rails with a plain host
        // floor before the skater is built (the skater's trajectory binds the grind provider at load).
        let mut physics = GamePhysics::load_with_difficulty(root, None, difficulty)?;
        sim_bridge::clear_grind_world(&mut physics)?;
        let floor = flat_floor(200.0, 0.0, sim_bridge::floor_material(&physics))?;
        sim_bridge::install_world(&mut physics, floor)?;
        let skater = SkaterRuntime::load(root, &graphs, &physics, difficulty.profile_key())?;
        let controls = PlayerControls::load(root)?;
        let camera = CameraRuntime::load(root)?;
        let frames = &skater.animation.evaluator.frames;
        let b = skater.skeleton_output.pose.board_bones;
        let skeleton = Skeleton {
            bone_names: frames.bone_names.clone(),
            parents: frames.parents.clone(),
            board_bones: b.all(),
        };
        // Trick display names: the HUD's language table (`private/hud/runtime/trickdisplay.json`), if present.
        let language = std::fs::read(root.join("private/hud/runtime/trickdisplay.json")).ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|json| serde_json::from_value::<std::collections::BTreeMap<String, String>>(json["language"].clone()).ok())
            .map(|language| crate::apt_text::TextAssets { language, ..Default::default() });
        let mut sim = Self {
            physics,
            skater,
            controls,
            graphs,
            camera,
            input: ControllerInput::default(),
            skeleton,
            language,
            history: None,
            history_generation: 0,
            history_period: Duration::ZERO,
            history_cut: false,
            events: Events::default(),
            was_grinding: false,
            was_push: false,
            last_brake: 0,
        };
        // Production applies the customiser's draft every frame (`customiser::preferences`); with no
        // customisation that is the empty profile: truck/wheel 0.7, default gestures, regular stance.
        sim.apply_profile(&serde_json::json!({}));
        // The skater is built at upstream's test-course spawn: stand it on the host floor until the host spawns it.
        sim.spawn([0.0, 0.0, 0.0], 0.0)?;
        // The teleport completes over the first two ticks (a second `spawn` is refused while one is pending).
        for _ in 0..2 {
            sim.step(&RawPad::default())?;
        }
        Ok(sim)
    }

    /// Replace the static collision world with host triangles, in the sim's native Y-up metres.
    ///
    /// Triangles are ONE-SIDED: the face whose normal `(b-a) x (c-a)` points up (+Y) is the rideable side,
    /// so wind them counter-clockwise seen from above. Built the way upstream's map loader builds a
    /// portable map's collision (`skate_world::collision_world`): 1 mm vertex welding, reversed-edge
    /// pairing for edge cosines and convexity bits, 64-triangle query ranges. Degenerate triangles are skipped.
    ///
    /// The triangles are first grouped by 16 m ground cell (a stable sort) so the 64-triangle query ranges bound
    /// small areas; use [`Sim::set_world_in_order`] to keep the host's order (contact tie-breaks follow it).
    /// The world holds no state between ticks (its contact buffer is cleared at every query), but the
    /// skater's histories (retained contacts, probes) describe the old world: when swapping the world under a
    /// riding skater, `spawn` afterwards (a teleport resets them), or swap in identical geometry.
    pub fn set_world(&mut self, vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> Result<(), String> {
        let world = collision_world_from_mesh(vertices, triangles, sim_bridge::floor_material(&self.physics), true)?;
        sim_bridge::install_world(&mut self.physics, world)
    }

    /// `set_world` with the triangles kept in the host's order (as upstream's loader keeps a map's order).
    pub fn set_world_in_order(&mut self, vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> Result<(), String> {
        let world = collision_world_from_mesh(vertices, triangles, sim_bridge::floor_material(&self.physics), false)?;
        sim_bridge::install_world(&mut self.physics, world)
    }

    /// Put the board and skater at `wheel_ground` (a point on the ground under the wheels) facing `heading`
    /// radians about +Y (0 faces +Z, positive turns toward +X), at rest. Uses upstream's own teleport
    /// (`SkaterRuntime::travel_to`, the path the mods' session teleport and the pause-menu travel take),
    /// which runs at the start of the next `step`; the board settles from the teleport's 0.1 m clearance.
    /// A spawn completes within two ticks; asking again before it has is refused with an `Err`.
    pub fn spawn(&mut self, wheel_ground: [f32; 3], heading: f32) -> Result<(), String> {
        if !wheel_ground.iter().all(|v| v.is_finite()) || !heading.is_finite() {
            return Err("spawn needs finite values".into());
        }
        let mut m = skate_core::physics::skeleton_animation_record::IDENTITY;
        let (s, c) = heading.sin_cos();
        m[0] = [c, 0.0, -s, 0.0];
        m[2] = [s, 0.0, c, 0.0];
        m[3] = [wheel_ground[0], wheel_ground[1], wheel_ground[2], 1.0];
        self.skater.travel_to(m)?;
        self.input.discard_gameplay();
        Ok(())
    }

    /// One fixed tick with this raw controller state: `sample` then `tick` (a drawn frame of exactly one tick, as the
    /// tests and upstream's `sample_raw_for_test` drive it).
    pub fn step(&mut self, pad: &RawPad) -> Result<(), String> {
        self.sample(pad);
        self.tick()
    }

    /// Upstream's `input::poll_controllers` (PreUpdate, once a drawn frame, whether or not a tick follows): the
    /// controller's state now, into its packet history (`ControllerInput::collect`).
    pub fn sample(&mut self, pad: &RawPad) {
        self.input.collect([
            Ok(DevicePacket { number: self.input.publications as u32 + 1, state: pad.xbox_state(), subtype: 1, kind: None }),
            Err(DeviceError::Disconnected),
            Err(DeviceError::Disconnected),
            Err(DeviceError::Disconnected),
        ]);
    }

    /// One fixed tick (upstream's FixedUpdate: `input::publish_actions`, `controls::sample`, `physics::advance`) on the
    /// controller as the frames since the last tick `sample`d it.
    pub fn tick(&mut self) -> Result<(), String> {
        if sim_bridge::failed(&self.physics) {
            return Err("the simulation failed on an earlier tick".into());
        }
        self.input.publish_actions();
        let tick_input = self.input.tick_input();
        // physics::controls::sample
        let mut map = tick_input.actions();
        self.controls
            .update_for_physics(&mut map, &self.physics, &self.skater, &self.camera)
            .map_err(|e| format!("Offboard controller publication: {e}"))?;
        self.controls.publish_gestures(
            self.physics.difficulty_index(),
            self.skater.player_input.physical.state.state_16,
        );
        // physics::advance
        let mut actions = tick_input.actions();
        let previous_state = self.skater.player_state.current();
        if let Err(message) = sim_bridge::advance(
            &mut self.physics,
            &mut self.skater,
            &mut self.controls,
            &self.graphs,
            &mut actions,
            tick_input.controller_available(),
            &mut self.camera,
        ) {
            sim_bridge::mark_failed(&mut self.physics);
            return Err(format!("{message}; state={:?}; tick={}", self.skater.player_state.current(), self.physics.ticks));
        }
        self.capture();
        self.collect_events(previous_state);
        Ok(())
    }

    /// `presentation::capture`.
    fn capture(&mut self) {
        let skater = &self.skater;
        if skater.pose_generation == self.history_generation && !self.camera.manual_cam.manual_cam_active() {
            return;
        }
        let Some(frame) = self.camera.presentation_frame() else { return };
        let period = self.physics.period();
        let [right, up, at] = frame.basis.columns.map(Vec3::from_array);
        let next_root = skater.animated_skeleton.roots.animation_to_world;
        let next = Snapshot {
            root: Transform::from_matrix(crate::animation::native_matrix(skater.animated_skeleton.roots.animation_to_world)),
            bones: skater.render_pose.iter().copied().map(crate::animation::native_matrix).collect(),
            camera: Transform {
                translation: Vec3::new(frame.position[0], frame.position[1], frame.position[2]),
                // Native At is forward; Bevy cameras look along local -Z.
                rotation: Quat::from_mat3(&Mat3::from_cols(-right, up, -at)).normalize(),
                ..Default::default()
            },
            fov: frame.field_of_view_degrees.to_radians(),
        };
        let reset = frame.discontinuity
            || skater.player_input.processed.flags_2472 & (1 << 10) != 0
            || self.history_period != period;
        self.history_cut = reset;
        let next = Frame { snapshot: next, root: next_root };
        if let Some((previous, current)) = self.history.as_mut() {
            if reset || current.snapshot.bones.len() != next.snapshot.bones.len() {
                *previous = next.clone();
            } else {
                std::mem::swap(previous, current);
            }
            *current = next;
        } else {
            self.history = Some((next.clone(), next));
        }
        self.history_generation = skater.pose_generation;
        self.history_period = period;
    }

    fn collect_events(&mut self, previous_state: skate_core::player::state::PhysicalStateId) {
        // `observe` takes the animation's audio latches every tick.
        let audio = std::mem::take(&mut self.skater.animation_input.audio);
        let state = self.skater.player_state.current();
        let deck = self.physics.board.bodies()[BodyId::Deck.index()].rates;
        let grinds = &self.skater.player_input.physical.grinds;
        let on_rail = (grinds.grinding_316 != 0 || state.is_grind()) && grinds.leaving_317 == 0;
        let mut events = Events { tick: self.physics.ticks, ..Default::default() };
        for event in sim_bridge::events(&self.physics) {
            match event {
                PhysicsEvent::StateChanged { from, to } => {
                    events.state_changes.push((format!("{from:?}"), format!("{to:?}")));
                    if from.category() == 100 && to.category() == 200 && self.skater.animation_input.extra.jump_strength > 0.0 {
                        events.popped = true;
                    }
                    if from.category() == 200 && to.category() != 200 && to.category() != 300 {
                        events.landed = true;
                    }
                    if to.category() == 300 && from.category() != 300 {
                        events.wiped_out = true;
                    }
                }
                // Never emitted upstream (only `StateChanged` is); handled for completeness.
                PhysicsEvent::Landing => events.landed = true,
                PhysicsEvent::Wipeout => events.wiped_out = true,
                PhysicsEvent::Contact { .. } => {}
            }
        }
        // `observe` edge-detects the push (`audio.push && !seen.push`) and logs the brake events on change:
        // the animation latches repeat every tick while the foot is planted / braking.
        let brake = if audio.brake_down { 1 } else if audio.brake_up { -1 } else { 0 };
        events.push_contact = audio.push;
        events.pushed = audio.push && !self.was_push;
        events.brake_contact = brake;
        events.brake_down = brake == 1 && self.last_brake != 1;
        events.brake_up = brake == -1 && self.last_brake != -1;
        self.was_push = audio.push;
        self.last_brake = brake;
        events.footstep = audio.footstep;
        events.grind_started = on_rail && !self.was_grinding;
        events.grind_stopped = !on_rail && self.was_grinding;
        events.state = format!("{state:?}");
        events.ground_speed = self.physics.riding.motion.ground_speed;
        events.airborne = state.category() == 200;
        events.grinding = on_rail;
        events.surface = crate::physics::board_surface(&self.physics);
        events.wheels_touching = self.physics.riding.wheel_lines.audio_surfaces.iter().filter(|&&s| s != 0).count() as u32;
        events.board_position = vec3(deck.position);
        events.board_velocity = vec3(deck.linear_velocity);
        self.was_grinding = on_rail;
        let _ = previous_state;
        self.events = events;
    }

    /// The clock's current tick period (60 Hz normally; the camera's slow-motion requests change it).
    pub fn period(&self) -> Duration { self.physics.period() }

    pub fn ticks(&self) -> u64 { self.physics.ticks }

    /// Bone names, parents and the board bones' indices.
    pub fn skeleton(&self) -> &Skeleton { &self.skeleton }

    /// The skater's bones and the camera for the last tick and the one before. `None` before the first tick.
    pub fn pose(&self) -> Option<Pose> {
        let (previous, current) = self.history.as_ref()?;
        let frame = |f: &Frame| PoseFrame {
            // The matrix as the simulation publishes it (upstream's snapshot decomposes it into a `Transform`).
            root: f.root,
            bones: f.snapshot.bones.iter().map(Mat4::to_cols_array_2d).collect(),
            camera: camera_out(f.snapshot.camera, f.snapshot.fov.to_degrees()),
        };
        let mut pose = Pose { previous: frame(previous), current: frame(current), cut: self.history_cut };
        pose.current.camera.cut = self.history_cut;
        Some(pose)
    }

    /// The stock camera's output of the last tick (`presentation_frame`), unblended.
    pub fn camera(&self) -> Option<CameraOut> {
        let (_, current) = self.history.as_ref()?;
        let mut out = camera_out(current.snapshot.camera, current.snapshot.fov.to_degrees());
        out.cut = self.history_cut;
        Some(out)
    }

    /// The stock camera graph's angle: `true` = High (graph type 1, upstream's default), `false` = Low (type 0).
    pub fn set_camera_high(&mut self, high: bool) { self.camera.set_camera_type(u32::from(high)); }

    /// Host window aspect ratio (width / height) for the camera's projection maths (default 16:9).
    pub fn set_aspect_ratio(&mut self, aspect: f32) { self.camera.set_aspect_ratio(aspect); }

    pub fn hud(&self) -> Hud {
        let s = &self.skater.scoring;
        let text = |label: &str| -> String {
            if label.is_empty() { String::new() } else { crate::apt_text::localize_trick(label, self.language.as_ref()) }
        };
        let capacity = s.data.line_capacity;
        Hud {
            sequence_score: s.sequence_score(),
            sequence_active: s.sequence_active(),
            line_score: s.line_score(),
            multiplier: s.multiplier(),
            line_time: s.line_time(),
            line_capacity: capacity,
            trick_id: s.trick_name().to_owned(),
            trick_text: text(s.trick_name()),
            trick_seq: s.trick_seq(),
            clean: s.clean(),
            sketchy: s.sketchy(),
            stance: s.stance(),
            new_trick: s.new_trick,
            modified_trick: s.modified_trick,
            close_tricks: s.close_tricks,
            landing_seq: s.landing_seq,
            landed_trick_id: s.landed_trick.clone(),
            landed_trick_text: text(&s.landed_trick),
            landed_base_id: s.landed_base.clone(),
            landed_spin_degrees: s.landed_spin_degrees,
            landed_clean: s.landed_clean,
            landed_sketchy: s.landed_sketchy,
            bail_seq: s.bail_seq,
        }
    }

    /// Events of the last tick that audio reacts to, plus the levels it reads.
    pub fn events(&self) -> &Events { &self.events }

    /// The current `PhysicalStateId` (for logs).
    pub fn state_name(&self) -> String { format!("{:?}", self.skater.player_state.current()) }

    /// Apply a customiser profile (`truck`, `wheel` 0..1; `gestures`; `stance`, `style`, `posture`),
    /// through the same function upstream's customiser UI uses.
    pub fn apply_profile(&mut self, profile: &serde_json::Value) {
        crate::customiser::apply_preferences(profile, &mut self.physics, &mut self.skater.animation);
    }

    /// The board's deck body (position, linear velocity), Y-up metres.
    pub fn board(&self) -> ([f32; 3], [f32; 3]) {
        let rates = self.physics.board.bodies()[BodyId::Deck.index()].rates;
        (vec3(rates.position), vec3(rates.linear_velocity))
    }

    /// The contact material `set_world` gives the host's triangles (for `prepare_world` off this thread).
    pub fn floor_material(&self) -> SurfaceMaterial { sim_bridge::floor_material(&self.physics) }

    /// Put in a world made by `prepare_world` (the same as `set_world`, without the work on this thread).
    pub fn install_world(&mut self, world: PreparedWorld) -> Result<(), String> { sim_bridge::install_world(&mut self.physics, world.0) }
}

/// The contact material of a host world's triangles.
pub type SurfaceMaterial = skate_core::physics::contact::RetailContactMaterial;

/// A collision world built from host triangles, as `Sim::set_world` builds it, ready for `Sim::install_world`.
pub struct PreparedWorld(BoardWorld);

/// `Sim::set_world`'s work, for another thread: the same triangles' rules, the material from `Sim::floor_material`.
pub fn prepare_world(vertices: &[[f32; 3]], triangles: &[[u32; 3]], material: SurfaceMaterial) -> Result<PreparedWorld, String> {
    collision_world_from_mesh(vertices, triangles, material, true).map(PreparedWorld)
}

/// A flat host floor `size` metres square at height `y`, upward facing.
fn flat_floor(size: f32, y: f32, material: skate_core::physics::contact::RetailContactMaterial) -> Result<BoardWorld, String> {
    let h = size * 0.5;
    let vertices = [[-h, y, -h], [h, y, -h], [h, y, h], [-h, y, h]];
    collision_world_from_mesh(&vertices, &[[0, 3, 2], [0, 2, 1]], material, false)
}

/// `skate_world::collision_world`'s portable-map path (no native edges, no retail archive) for host triangles.
pub(crate) fn collision_world_from_mesh(
    mesh_vertices: &[[f32; 3]],
    mesh_triangles: &[[u32; 3]],
    material: skate_core::physics::contact::RetailContactMaterial,
    group_by_cell: bool,
) -> Result<BoardWorld, String> {
    // Host triangles as 3 points each, dropping degenerate or non-finite ones.
    let mut corners: Vec<[[f32; 3]; 3]> = Vec::with_capacity(mesh_triangles.len());
    for tri in mesh_triangles {
        let mut points = [[0.0; 3]; 3];
        for (k, &index) in tri.iter().enumerate() {
            points[k] = *mesh_vertices.get(index as usize).ok_or("triangle index out of range")?;
        }
        if !points.iter().flatten().all(|v| v.is_finite()) {
            continue;
        }
        let [a, b, c] = points.map(Vec3::from_array);
        if (b - a).cross(c - a).try_normalize().is_none() {
            continue;
        }
        corners.push(points);
    }
    if corners.is_empty() {
        return Err("the world has no usable triangles".into());
    }
    // Group neighbours so the 64-triangle query ranges below bound small areas (host meshes are not
    // spatially ordered): a stable sort by 16 m cell. Triangle order only decides contact tie-breaks.
    if group_by_cell { corners.sort_by_key(|t| {
        let c = (Vec3::from_array(t[0]) + Vec3::from_array(t[1]) + Vec3::from_array(t[2])) / 3.0;
        ((c.x / 16.0).floor() as i64, (c.z / 16.0).floor() as i64)
    }); }
    // Match the reference RW mesh compiler's 1 mm vertex welding and reversed
    // edge pairing. Triangle diagonals are adjacency, never authored ledges.
    let mut welded = HashMap::<[i64; 3], usize>::new();
    let mut positions = Vec::<Vec3>::new();
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    for points in &corners {
        let ids = points.map(|p| {
            let inverse = 1.0 / f64::from(0.001_f32);
            let key = p.map(|v| (f64::from(v) * inverse).round() as i64);
            *welded.entry(key).or_insert_with(|| {
                let id = positions.len();
                positions.push(Vec3::from_array(p));
                id
            })
        });
        let [a, b, c] = points.map(Vec3::from_array);
        let normal = (b - a).cross(c - a).try_normalize().ok_or("Invalid collision triangle normal")?;
        vertices.push(ids);
        normals.push(normal);
    }
    let mut cosines = vec![[1.; 3]; vertices.len()];
    let mut flags = vec![TriangleFeature::ONE_SIDED | TriangleFeature::USE_EDGE_COSINES | 0xe0; vertices.len()];
    let mut open = HashMap::<(usize, usize), (usize, usize)>::new();
    for (i, ids) in vertices.iter().enumerate() {
        for edge in 0..3 {
            let (a, b) = (ids[edge], ids[(edge + 1) % 3]);
            if let Some((other, oe)) = open.remove(&(b, a)) {
                let cosine = normals[i].dot(normals[other]).clamp(-1., 1.);
                let orientation = (positions[b] - positions[a]).dot(normals[i].cross(normals[other]));
                // ExtendedEdgeCosine / MakeEdgeCode in rw_collision_mesh.cpp:
                // orientation >= -1e-6 is convex; flat edges have no convex bit.
                for (ti, e) in [(i, edge), (other, oe)] {
                    cosines[ti][e] = cosine;
                    if orientation <= -1.0e-6 || cosine >= 1. {
                        flags[ti] &= !(0x20 << e);
                    }
                }
            } else {
                open.entry((a, b)).or_insert((i, edge));
            }
        }
    }
    let mut adjacent = vec![Vec::new(); positions.len()];
    for (i, ids) in vertices.iter().enumerate() {
        for &v in ids {
            adjacent[v].push(i);
        }
    }
    for (v, faces) in adjacent.iter().enumerate() {
        let reference = normals[faces[0]];
        if faces.iter().all(|&i| (reference.dot(normals[i]) - 1.).abs() <= 0.01) {
            for &i in faces {
                for corner in 0..3 {
                    if vertices[i][corner] == v {
                        flags[i] |= 0x200 << corner;
                    }
                }
            }
        }
    }
    let mut triangles = Vec::with_capacity(vertices.len());
    // Host triangles carry no authored material table: surface 0 (material 0 / category 0), as the test
    // course does, and the physics settings' floor contact material.
    let packed_surfaces = vec![0u16; vertices.len()];
    for i in 0..vertices.len() {
        let points = vertices[i].map(|id| {
            let p = positions[id];
            Vector3::new(p.x, p.y, p.z)
        });
        triangles.push(
            WorldTriangle::from_vertices(points, material, 0, flags[i], cosines[i], 0.)
                .ok_or("Invalid collision volume")?,
        );
    }
    // Bound contiguous ranges once so the existing BVH can reject distant geometry (portable maps have no
    // native cluster hierarchy). Triangle order and mesh identity/filter values are kept as upstream.
    let mut meshes = Vec::new();
    for start in (0..triangles.len()).step_by(64) {
        let end = (start + 64).min(triangles.len());
        let bounds = Bounds::from_points(triangles[start..end].iter().flat_map(|t| t.triangle.vertices))
            .ok_or("collision bounds empty")?;
        meshes.push(QueryMesh {
            geometry: 0,
            rejection_flags: 0,
            triangle_range: start..end,
            local_to_world: RetailAffineTransform::IDENTITY,
            world_to_local: RetailAffineTransform::IDENTITY,
            local_bounds: bounds,
            matching_group: -1,
            pool: QueryPool::Ground,
        });
    }
    let metadata = QueryMetadata { packed_surfaces, meshes, static_edges: vec![], island_flags: 0 };
    BoardWorld::with_query_metadata(triangles, metadata).map_err(str::to_owned)
}
