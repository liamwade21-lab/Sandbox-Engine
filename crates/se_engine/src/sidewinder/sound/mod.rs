//! The Sidewinder's sounds (ARCHITECTURE.md A7): Elite's own Wwise data (`sounds.json`) played by the pure mixer
//! (`se_core::elite_sound::Mixer`) from the ship's real state, only while seated.
//!
//! - `load.rs`: at the first spawn (the files are read only once `SidewinderAssets` exists: the spec's C20), on worker
//!   threads, `sounds.json` and every recording, each checked before anything can play it. The ship flies silent until then.
//! - `triggers.rs`: `Pilot`'s, `Seat`'s state to Elite's posts, states and game parameters, rule by rule
//!   (`triggers.json`'s ids).
//! - `voices.rs`: the mixer's voice updates as sound: one output source mixing every heard voice, built off the game
//!   thread.
//!
//! Getting out, losing the ship or the mod going off stops every voice at once and starts the mixer afresh.
//!
//! **Not posted (nothing in the flight drives them; nothing stands in for them):** the `OPEN_EVENTS`; mass lock
//! (`Cockpit_MassLockEngage` / `Disengage`: the flight has no mass lock), `supercruise_emergencystop` (the flight keeps no
//! record of an emergency drop), the fuel scoop, power plant and reboot cues (not modelled), the cargo scoop's acquired
//! and warning cues (no cargo), the canopy's crack stages 01-04 and its voice reports (which lines report kinds 0 and 1 is
//! not closed), the heat sink and malfunction lines (no heat sink, no malfunctions), `UnderAttack`, `EnginesOn_<suffix>`
//! (no event of the banks), the supercruise speed and SCO parameters (not decompiled).
//!
//! **The engine's inputs** are Elite's own rules in Elite's units (T5d-6, `triggers.rs` `Producer`), from the flight's
//! numbers in `ShipFacts::motion` (the flight owns them: nothing of the flight is worked out here).
//!
//! **Where each sound sits (T5d-5).** The listener is the pilot's head: the seat's eye (`SidewinderAssets::eye`) turned by
//! the head this frame (`seat::head_turn` of `Seat::head`, what the camera is written with). The state machines' cues come
//! from `Parts::events()` (`cues`, after `parts::update`): each is posted on an emitter of its own for its machine and
//! bones (`ShipSound::cue_emitter`: the two rear legs are two emitters), placed every frame at its bones as posed now
//! (`ShipRig::bone_frames` with the parts' values; `MODEL_ORIGIN` at its rig node's origin, `Parts::matrix`) from the
//! head, in the head's axes; their voices are panned and attenuated by the output (`voices.rs`,
//! `se_core::elite_sound::place`). Placed: the three legs' (at `ModelOrigin`, read as each leg model's origin, E). The
//! hull's cues name bones no shipped model has (`Ship_Origin`, `HardpointDoorLeft`, `HardpointDoorRight`): their emitters
//! are the listener's own. The machines' audio states (`AudioState`) are set from the same list. Every other emitter is
//! the listener's own and plays centred: the loops' and reporters' emitters
//! (`ship.json`'s `Internal_MainDrive_Left`, `Cockpit_Back_Centre`, `Internal_Cockpit_Origin`, `Alarms` ... 21 names:
//! `triggers::Emitter`'s `Drive`, `Cockpit`, `CockpitOrigin`, `Ship`, `Voice`) are Elite's audio emitter components named
//! by the entity's template, and no bone, locator or rig node of the shipped models carries those names (the hull's 156
//! bones and the cockpit's 148 checked in T5c; the rigs' 84 nodes in T5d-5), so the data gives them no place. Not built:
//! `KeepAudioEmitterComponentDisabled` (a gear leg's emitter disabled while it rests: what disabling does to its voices
//! was not traced) and the `Audio*` activities' starts and stops (logged with `sw sound log on`).
//!
//! **Filters and effects:** each voice's low-pass and high-pass run as Elite's own sound engine runs them, then the
//! Parametric EQs and Guitar Distortions on its way to the master, node's and buses' (`voices.rs`,
//! `se_core::elite_sound::filter`, decompiled from the exe); the master limiter's output level is in every voice's gain,
//! and the output counts the frames its mix passes the limiter's threshold (logged with `sw sound log on` and `sw sound
//! state`: 0 means Elite's limiter would never have acted). The other effects (compressors, the limiting itself, the
//! master's plug-in 0x00BA0003, meters' ballistics, the DTS headphone plug-ins, Tremolo, Flanger) are not built:
//! `se_core::elite_sound`'s docs list them.
//!
//! **The player's context:** player_location is outside_station and OnFootLocation NotOnFoot from the start (no station
//! near, never on foot: `triggers.rs`); POV is never set (no setter in the exe).

mod load;
mod triggers;
mod voices;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use bevy::audio::AddAudioSource;
use bevy::input::gamepad::{Gamepad, GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use se_core::catalog::ObjectRef;
use se_core::elite_rig::Event as MachineEvent;
use se_core::elite_sound::{curve, Mixer, Output, SoundBank, VoiceUpdate};

use crate::mods::ModControl;
use crate::sound::SoundSettings;

use super::assets::ShipRig;
use super::facts::ShipFacts;
use super::parts::{Parts, ShipEvent};
use super::pilot::{Pilot, PilotKeys};
use super::seat::{head_turn, Seat};
use super::{Sidewinder, SidewinderAssets};
use load::{Clip, Loaded, Rumble};
use triggers::{Call, Emitter, ShipView, Triggers};
use voices::{Event, ShipOutput, Voices};

/// Wwise's 0 dB (a mixer gain of 1) as Bevy's volume: unity at the game's default sound volume (`SoundSettings::volume`,
/// 0.3, which multiplies it), so the mixer's gain reaches the speakers as it is. A principle, not a fit (the orchestrator's
/// decision, T5d); the matched-filter fits of the cues' own recordings in the user's Elite recordings confirm it
/// (`re/elite/work/sound/t5d/mfit.py`, the notes' level table). (T5c's factor of 3.039 was fitted by power over
/// `s08_fa_off.wav`, which also holds Elite's spoken line: it counted speech.)
const LEVEL: f32 = 1.0 / 0.3;
/// A frame longer than this is logged (`sw sound log on`).
const LONG_FRAME_S: f64 = 0.05;
/// A rumble request lasts this long and is renewed every frame while a Motion Generator voice plays.
const RUMBLE_S: f32 = 0.1;

/// The reading of the files, from the first spawn.
#[derive(Resource, Default)]
struct SoundLoad {
    started: bool,
    task: Option<Task<Result<Loaded, (String, String)>>>,
}

/// The ship's sounds once read: the mixer, the triggers' memory, the voices.
#[derive(Resource)]
pub struct ShipSound {
    bank: Arc<SoundBank>,
    clips: HashMap<se_core::elite_sound::Id, Arc<Clip>>,
    rumbles: HashMap<se_core::elite_sound::Id, Rumble>,
    mixer: Mixer,
    seeds: u64,
    triggers: Triggers,
    voices: Voices,
    /// Playing (seated): the audio objects exist.
    active: bool,
    /// The parked pose when the audio last stopped: a different one at the next boarding is a new ship.
    left_at: Option<(ObjectRef, DVec3)>,
    /// Motion Generator voices: their age (s) and source.
    rumbling: HashMap<u32, (f64, se_core::elite_sound::Id)>,
    rumble_on: bool,
    /// What each voice is now, for `sw sound state`; the most voices at once.
    now: BTreeMap<u32, (String, VoiceUpdate)>,
    most: usize,
    /// The state machines' cue emitters by the mixer's id (`cue_emitter`: one per machine and bone set), each placed every
    /// frame from the head.
    cue_emitters: BTreeMap<u32, CueEmitter>,
    /// The parts' pose (`Parts::revision`) the cue emitters' places were worked out for.
    cue_rev: Option<u64>,
    /// Unknown events and unbuilt outputs already logged.
    warned: HashSet<String>,
    pub log: bool,
    /// When the audio thread's heartbeat was last logged, and its frame count then.
    beat: (f64, u64),
}

/// A state machine's cue emitter: its machine (`ship.machines`' index) and bones, the rig nodes they are on
/// (`ShipRig::cue_nodes`), each bone as `Node!Bone` (`ShipRig::bone_frames`' form) and where they are now in the ship's
/// space (their middle; None: the data gives them no place, so the emitter is the listener's own).
struct CueEmitter {
    machine: usize,
    bones: Vec<String>,
    nodes: Vec<usize>,
    names: Vec<String>,
    at: Option<Vec3>,
}

/// The cue bone that names its model's origin: the gear legs' cues are all at `ModelOrigin`, which is no bone of the leg
/// models (nor of any shipped model): read by its name as the leg's own model's origin, its rig's root node (where
/// `ShipRig::cue_nodes` puts a bone no model has), grade E. The hull's cue bones (`Ship_Origin`, the name of the hull's
/// audio emitter of its loops, a string of the exe; `HardpointDoorLeft`, `HardpointDoorRight`) are no bone either and
/// name no model: those emitters are the listener's own.
const MODEL_ORIGIN: &str = "ModelOrigin";

/// The first cue emitter's id (`Emitter::Cue`): after the listener's own (`triggers::Emitter`'s 0x100..0x302).
const FIRST_CUE_EMITTER: u32 = 0x1000;

/// The listener: the pilot's head in the ship's space (the seat's eye, turned by the head this frame).
struct Listener {
    eye: Vec3,
    turn: Quat,
}

impl Listener {
    fn of(assets: &SidewinderAssets, seat: &Seat) -> Listener {
        Listener { eye: Vec3::from_array(assets.eye), turn: head_turn(seat.head.turn) }
    }

    /// A point of the ship's space from the head, in the head's axes (x right, y up, -z ahead).
    fn from(&self, p: Vec3) -> [f32; 3] {
        (self.turn.inverse() * (p - self.eye)).to_array()
    }
}

/// The middle of some points (the origin for none).
fn middle(points: &[Vec3]) -> Vec3 {
    points.iter().sum::<Vec3>() / points.len().max(1) as f32
}

/// The drive's and boost's keys of this frame, read before the flight takes the test commands' presses.
#[derive(Resource, Default)]
struct Presses {
    jump: bool,
    boost: bool,
}

pub fn plugin(app: &mut App) {
    if !crate::sandbox::enabled() {
        return;
    }
    app.add_audio_source::<ShipOutput>().init_resource::<SoundLoad>().init_resource::<Presses>();
    // (The spec's C20: nothing before a spawn's files are read.)
    app.add_systems(Update, load_sounds.run_if(resource_exists::<SidewinderAssets>).run_if(|l: Res<SoundLoad>| l.task.is_some() || !l.started));
    app.add_systems(PostUpdate, note_presses.before(super::hyper::tunnel).run_if(resource_exists::<ShipSound>).in_set(crate::universe::UniverseSet::Camera));
    // (After the machines ran this frame: their cues and the parts' places are this frame's.)
    app.add_systems(PostUpdate, (cues, drive).chain().after(super::parts::update).run_if(resource_exists::<ShipSound>).run_if(resource_exists::<SidewinderAssets>));
}

/// Starts reading at the first spawn; takes the result when ready.
fn load_sounds(mut commands: Commands, mut load: ResMut<SoundLoad>) {
    if !load.started {
        load.started = true;
        info!("sidewinder sound: reading sounds.json and its recordings");
        load.task = Some(AsyncComputeTaskPool::get().spawn(async move { load::load() }));
        return;
    }
    let Some(task) = load.task.as_mut() else { return };
    let Some(done) = futures::check_ready(task) else { return };
    load.task = None;
    match done {
        Ok(l) => {
            info!(
                "sidewinder sound: {} events, {} recordings checked in {:.2} s ({} refused), {} rumbles",
                l.bank.file.events.len(),
                l.clips.len(),
                l.seconds,
                l.refused.len(),
                l.rumbles.len()
            );
            for (file, why) in &l.refused {
                warn!("sidewinder sound: {file} refused: {why} (it stays silent)");
            }
            commands.insert_resource(ShipSound::new(l));
        }
        Err((file, why)) => warn!("sidewinder sound: {file}: {why} (the ship stays silent)"),
    }
}

impl ShipSound {
    fn new(l: Loaded) -> ShipSound {
        let mixer = Mixer::new(l.bank.clone(), 1);
        for (effect, modulator, param) in l.bank.effects_not_exact() {
            info!("sidewinder sound: effect {effect} is not exact: its modulator {modulator} (on its parameter {param}) is not built, so that parameter keeps its value without it");
        }
        ShipSound {
            bank: l.bank,
            clips: l.clips,
            rumbles: l.rumbles,
            mixer,
            seeds: 1,
            triggers: Triggers::default(),
            voices: Voices::default(),
            active: false,
            left_at: None,
            rumbling: HashMap::new(),
            rumble_on: false,
            now: BTreeMap::new(),
            most: 0,
            cue_emitters: BTreeMap::new(),
            cue_rev: None,
            warned: HashSet::new(),
            log: false,
            beat: (0.0, 0),
        }
    }

    /// Hands the triggers' calls to the mixer.
    fn call(&mut self, calls: &[Call]) {
        for c in calls {
            match c {
                Call::Post(name, e) => {
                    if self.log {
                        info!("sidewinder sound: post {name} on {e:?} at {:.3} s", self.mixer.time());
                    }
                    if self.mixer.post(name, e.id()).is_err() && self.warned.insert(name.clone()) {
                        warn!("sidewinder sound: {name} is no event of sounds.json");
                    }
                }
                Call::State(g, s) => self.set_state(g, s),
                Call::Gp(n, v) => self.mixer.set_game_parameter(n, *v, None),
            }
        }
    }

    fn set_state(&mut self, group: &str, state: &str) {
        if self.log {
            info!("sidewinder sound: state {group} = {state}");
        }
        self.mixer.set_state(group, state);
    }

    /// The emitter of machine `machine`'s cues at `bones` (made at its first cue, placed from then on every frame): one per
    /// machine and bone set, so the same cue of two legs plays at two places.
    fn cue_emitter(&mut self, rig: &ShipRig, machine: usize, bones: &[String], nodes: Vec<usize>) -> u32 {
        if let Some((id, _)) = self.cue_emitters.iter().find(|(_, e)| e.machine == machine && e.bones == bones) {
            return *id;
        }
        let id = FIRST_CUE_EMITTER + self.cue_emitters.len() as u32;
        let names = bones.iter().zip(&nodes).filter_map(|(b, n)| rig.nodes.get(*n).map(|node| format!("{}!{b}", node.name))).collect();
        self.cue_emitters.insert(id, CueEmitter { machine, bones: bones.to_vec(), nodes, names, at: None });
        self.cue_rev = None;
        id
    }

    /// Every cue emitter placed from the head now: its bones' places (`ShipRig::bone_frames` with the parts' values, worked
    /// out again whenever the parts' pose changed or an emitter was made; `MODEL_ORIGIN` at its node's origin), in the head's
    /// axes. An emitter none of whose bones the data places is left unplaced: the listener's own.
    fn place_cue_emitters(&mut self, rig: &ShipRig, parts: &Parts, listener: &Listener) {
        if self.cue_rev != Some(parts.revision()) {
            self.cue_rev = Some(parts.revision());
            let names: Vec<String> = self.cue_emitters.values().flat_map(|e| e.names.iter().cloned()).collect();
            let frames = rig.bone_frames(parts.values(), &names).unwrap_or_default();
            let mut k = 0;
            for e in self.cue_emitters.values_mut() {
                let frames = frames.get(k..k + e.names.len()).unwrap_or_default();
                k += e.names.len();
                let found: Vec<Vec3> = e
                    .bones
                    .iter()
                    .zip(&e.nodes)
                    .zip(frames)
                    .filter_map(|((b, n), f)| f.map(|f| f.0).or_else(|| (b == MODEL_ORIGIN).then(|| parts.matrix(*n).w_axis.truncate())))
                    .collect();
                e.at = (!found.is_empty()).then(|| middle(&found));
            }
        }
        for (id, e) in &self.cue_emitters {
            if let Some(at) = e.at {
                self.mixer.set_emitter(*id, listener.from(at));
            }
        }
    }

    /// One of this frame's machine events (`Parts::events`): a cue's begin, end or single event posted on its emitter
    /// (none when no bank has it: it plays nothing, as in Elite), an audio state set, an audio activity logged.
    fn machine_event(&mut self, e: &ShipEvent, assets: &SidewinderAssets, parts: &Parts, listener: &Listener) {
        let rig = &assets.rig;
        match &e.event {
            MachineEvent::SoundBegin { event, bones, .. } | MachineEvent::SoundEnd { event, bones, .. } | MachineEvent::SoundSingle { event, bones, .. } => {
                let Some(event) = event else { return };
                let nodes = rig.cue_nodes(e.machine, bones);
                if nodes.is_empty() {
                    if self.warned.insert(format!("cue nodes {} {bones:?}", e.machine)) {
                        warn!("sidewinder sound: no rig node holds machine {}'s bones {bones:?}: {event} is not posted", e.machine);
                    }
                    return;
                }
                let id = self.cue_emitter(rig, e.machine, bones, nodes);
                self.place_cue_emitters(rig, parts, listener);
                if self.log {
                    let m = rig.ship.machines.get(e.machine);
                    let at = self.cue_emitters[&id].at.map(|p| listener.from(p)).map_or("the listener's own (the data gives its bones no place)".to_string(), |[x, y, z]| {
                        format!("({x:.2}, {y:.2}, {z:.2}) m from the head ({:.2} m)", Vec3::new(x, y, z).length())
                    });
                    info!("sidewinder sound: cue {event} of machine {} ({} {}) at {bones:?} on emitter {id:#x}, {at}", e.machine, m.map_or("?", |m| m.name.as_str()), m.map_or("?", |m| m.owner.as_str()));
                }
                self.call(&[Call::Post(event.clone(), Emitter::Cue(id))]);
            }
            MachineEvent::AudioState { group, state } => self.set_state(group, state),
            MachineEvent::Started { kind, .. } | MachineEvent::Stopped { kind, .. } => {
                if self.log {
                    info!("sidewinder sound: machine {} {}: {kind} (not built)", e.machine, if matches!(e.event, MachineEvent::Started { .. }) { "starts" } else { "stops" });
                }
            }
            _ => {}
        }
    }

    /// Every voice stopped, the mixer made afresh, the triggers back to before the audio starts.
    fn stop(&mut self, commands: &mut Commands, pads: &Query<Entity, With<Gamepad>>, rumble: &mut MessageWriter<GamepadRumbleRequest>) {
        let mut calls = Vec::new();
        self.triggers.teardown(&mut calls);
        self.call(&calls);
        self.voices.stop_all(commands);
        self.seeds += 1;
        self.mixer = Mixer::new(self.bank.clone(), self.seeds);
        self.cue_emitters.clear();
        self.cue_rev = None;
        self.active = false;
        self.rumbling.clear();
        self.now.clear();
        if std::mem::take(&mut self.rumble_on) {
            for pad in pads {
                rumble.write(GamepadRumbleRequest::Stop { gamepad: pad });
            }
        }
        info!("sidewinder sound: stopped");
    }
}

/// The test commands' and the keys' presses of the drive and boost, before `pilot::fly` takes the commands'.
fn note_presses(pilot: Res<Pilot>, keys: Res<PilotKeys>, mut p: ResMut<Presses>) {
    *p = Presses { jump: keys.jump || pilot.dev_jump, boost: keys.boost || pilot.dev_boost };
}

/// The ship's state as the triggers read it: the pilot's state, and the flight's numbers for the engine's inputs as the
/// facts carry them (`ShipFacts::motion`, the flight's demands).
fn view(pilot: &Pilot, presses: &Presses, dt: f64, facts: &ShipFacts) -> ShipView {
    use elite_sim::boost::State;
    use elite_sim::power::{ENG, SYS, WEP};
    let boost = match pilot.boost.state {
        State::Idle => 0,
        State::Warm(_) => 1,
        State::Firing(_) => 2,
        State::Cool(_) => 3,
    };
    ShipView {
        dt,
        mode: if pilot.jump.is_some() { 3 } else if pilot.cruise.is_some() { 2 } else { 1 },
        engines_on: pilot.powered,
        boost,
        boost_refused: presses.boost && boost == 0 && !pilot.landed && pilot.cruise.is_none() && pilot.glide.is_none(),
        flight_assist: pilot.flight_assist,
        gear_down: pilot.gear_down,
        scoop: pilot.scoop,
        lights: pilot.lights,
        silent: pilot.silent,
        canopy_breached: !pilot.life.canopy,
        oxygen_mask: pilot.life.running(),
        heat: pilot.heat.fraction(&pilot.heat_spec),
        hull: pilot.hull.strength / pilot.hull.max.max(1e-9),
        shields_up: pilot.shields.up,
        shields_powered: elite_sim::silent::shields_powered(pilot.powered && pilot.modules[super::panels::M_SHIELDS], pilot.silent),
        drive: pilot.drive,
        jump_pressed: presses.jump,
        safe_drop: pilot.safe_drop,
        glide: pilot.glide.is_some(),
        motion: facts.motion,
        throttle: facts.throttle,
        pips: [pilot.dist.pips(SYS), pilot.dist.pips(ENG), pilot.dist.pips(WEP)],
    }
}

/// The sound plays: seated in the parked ship, the mod's pilot on, the ship not lost.
fn playing(seat: &Seat, control: &ModControl, pilot: &Pilot, ship: &Sidewinder) -> bool {
    seat.seated && control.pilot && ship.parked.is_some() && !pilot.destroyed
}

/// This frame's machine events (`Parts::events`, after `parts::update`) while the sound plays: the cues posted on their
/// emitters, the audio states set (`ShipSound::machine_event`).
fn cues(mut snd: ResMut<ShipSound>, parts: Res<Parts>, assets: Res<SidewinderAssets>, seat: Res<Seat>, control: Res<ModControl>, pilot: Res<Pilot>, ship: Res<Sidewinder>) {
    if parts.events().is_empty() || !playing(&seat, &control, &pilot, &ship) {
        return;
    }
    let listener = Listener::of(&assets, &seat);
    for e in parts.events() {
        snd.machine_event(e, &assets, &parts, &listener);
    }
}

/// Once a frame after the seat, the flight and the machines: the triggers, the cue emitters' places, the mixer, the voices.
#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut snd: ResMut<ShipSound>,
    presses: Res<Presses>,
    seat: Res<Seat>,
    control: Res<ModControl>,
    pilot: Res<Pilot>,
    ship: Res<Sidewinder>,
    (parts, assets, facts): (Res<Parts>, Res<SidewinderAssets>, Res<ShipFacts>),
    settings: Res<SoundSettings>,
    mut outputs: ResMut<Assets<ShipOutput>>,
    pads: Query<Entity, With<Gamepad>>,
    mut rumble: MessageWriter<GamepadRumbleRequest>,
) {
    let snd = &mut *snd;
    snd.voices.service(&mut commands);
    let parked = ship.parked.as_ref().map(|p| (p.obj, p.pos));
    if !playing(&seat, &control, &pilot, &ship) {
        if snd.active {
            snd.left_at = parked;
            snd.stop(&mut commands, &pads, &mut rumble);
        }
        return;
    }
    if !snd.active {
        snd.active = true;
        if snd.left_at != parked {
            snd.triggers.new_ship();
        }
        info!("sidewinder sound: started");
    }
    let dt = time.delta_secs_f64().min(0.25);
    let v = view(&pilot, &presses, dt, &facts);
    let mut calls = Vec::new();
    snd.triggers.step(&v, &mut calls);
    snd.call(&calls);
    snd.place_cue_emitters(&assets.rig, &parts, &Listener::of(&assets, &seat));
    let level = LEVEL * settings.volume;
    let ShipSound { mixer, voices, clips, rumbles, rumbling, now, log, warned, .. } = snd;
    let log = *log;
    let stepped = std::time::Instant::now();
    let updates = mixer.step(dt as f32);
    let mut motors = [0.0f32; 2];
    for u in updates {
        match u.output {
            Output::Audio { source } => {
                let clip = clips.get(&source);
                if log {
                    if let Some(to) = u.seek_to {
                        info!("sidewinder sound: voice {} seeks to {to:.3} s", u.voice);
                    }
                }
                voices.apply(u, clip, level, &mut |e| if log {
                    match e {
                        Event::Started(u, file) => info!(
                            "sidewinder sound: voice {} starts {file} on emitter {:#x}: gain {:.4} ({:.1} dB), pitch {:.3}, low-pass {:.0}, high-pass {:.0}{}{}{}",
                            u.voice,
                            u.emitter,
                            u.gain,
                            u.volume_db,
                            u.pitch,
                            u.lowpass,
                            u.highpass,
                            u.start.map_or(String::new(), |s| if s.looping { format!(", looping {:?}", s.loop_count) } else { String::new() }),
                            if u.virtual_voice { ", virtual" } else { "" },
                            placed_text(u, clip)
                        ),
                        Event::Stopped(u) => info!("sidewinder sound: voice {} stops", u.voice),
                        Event::Virtual(u) => info!("sidewinder sound: voice {} goes virtual (dropped until heard)", u.voice),
                        Event::Silent(u) => info!("sidewinder sound: voice {} has no recording (source {source})", u.voice),
                    }
                });
                if u.stop {
                    now.remove(&u.voice);
                } else if let Some(c) = clip {
                    match now.get_mut(&u.voice) {
                        Some(e) => e.1.clone_from(u),
                        None => {
                            now.insert(u.voice, (c.file.clone(), u.clone()));
                        }
                    }
                }
            }
            Output::Rumble { source } => {
                if u.stop {
                    rumbling.remove(&u.voice);
                    continue;
                }
                let age = rumbling.entry(u.voice).or_insert((0.0, source));
                if let Some(r) = rumbles.get(&source).filter(|r| age.0 <= r.length_s) {
                    for (m, curve_pts) in motors.iter_mut().zip(&r.motors) {
                        *m += (curve::find(curve_pts, age.0).abs() as f32 * u.gain).clamp(0.0, 1.0);
                    }
                }
                age.0 += dt;
            }
            Output::Synth { source } => {
                if warned.insert(format!("synth {source}")) {
                    info!("sidewinder sound: a Tone Generator voice (source {source}) is not built: silent");
                }
            }
        }
    }
    snd.voices.set_limit(snd.mixer.master_limiter(), level);
    let built = std::time::Instant::now();
    let queued = snd.voices.queued();
    snd.voices.start_queued(&mut commands, &mut outputs);
    let handed = snd.voices.hand_over();
    snd.voices.tell_guard();
    if snd.log {
        for (n, waited) in &handed {
            info!("sidewinder sound: {n} starts handed to the output together, {:.1} ms after they were asked for", waited * 1000.0);
        }
    }
    // The audio thread's heartbeat, once a second (`sw sound log on`): frames its voices made, blocks the guard acted on.
    let t = time.elapsed_secs_f64();
    if snd.log && t - snd.beat.0 >= 1.0 {
        let (made, bad, skipped, longest_us, refused, top) = snd.voices.meter();
        let (over, dropped) = snd.voices.over_limit();
        let top: Vec<String> = top.iter().map(|(id, f, n, s)| format!("{id} {f} {n} at {s:.2}")).collect();
        info!(
            "sidewinder sound: audio thread made {} frames in {:.2} s over {} voices ({bad} blocks with a sample not finite, {refused} voices refused, {over} frames over the master limiter's threshold, {dropped} drops on the audio thread so far; {skipped} frames skipped, the longest skip {:.1} ms; most: {})",
            made - snd.beat.1,
            t - snd.beat.0,
            snd.voices.count(),
            longest_us as f64 / 1000.0,
            top.join(", ")
        );
        snd.beat = (t, made);
    }
    if snd.log && (queued > 0 || dt > LONG_FRAME_S) {
        info!(
            "sidewinder sound: frame {:.1} ms (the last), mixer {:.2} ms, {queued} starts sent to be built in {:.2} ms",
            dt * 1000.0,
            (built - stepped).as_secs_f64() * 1000.0,
            built.elapsed().as_secs_f64() * 1000.0
        );
    }
    snd.most = snd.most.max(snd.voices.count());
    // The pad's motors (`carried_07`: channel 0 the left, low-frequency motor; 1 the right), renewed each frame.
    let on = motors.iter().any(|m| *m > 0.0);
    if on || snd.rumble_on {
        for pad in &pads {
            rumble.write(GamepadRumbleRequest::Stop { gamepad: pad });
            if on {
                let intensity = GamepadRumbleIntensity { strong_motor: motors[0].min(1.0), weak_motor: motors[1].min(1.0) };
                rumble.write(GamepadRumbleRequest::Add { duration: std::time::Duration::from_secs_f32(RUMBLE_S), intensity, gamepad: pad });
            }
        }
        snd.rumble_on = on;
    }
}

/// A placed voice's place for the log: where its emitter is from the head, the distance's gain and filter shares and the
/// pan's gains on each channel (empty for a centred voice).
fn placed_text(u: &VoiceUpdate, clip: Option<&Arc<Clip>>) -> String {
    let (Some(p), Some(at)) = (&u.place, u.position) else { return String::new() };
    let now = p.at(at, clip.map_or(1, |c| usize::from(c.channels)));
    let [x, y, z] = at;
    format!(
        ", placed at ({x:.2}, {y:.2}, {z:.2}) m: distance {:+.1} dB, low-pass {:+.1}, high-pass {:+.1}, left / right {:.3} / {:.3}{}",
        curve::gain_to_db(f64::from(now.gain)),
        now.lowpass,
        now.highpass,
        now.matrix[0][0],
        now.matrix[0][1],
        if p.pan { "" } else { " (not panned)" }
    )
}

/// The game parameters `sw sound state` lists.
const LISTED: &[&str] = &[
    "spaceship_speed_self",
    "spaceship_speed_physical_self",
    "spaceship_speed_relative_self",
    "spaceship_accel_self",
    "spaceship_accel_relative_smooth_self",
    "spaceship_accel_directional_self",
    "spaceship_target_speed_relative_self",
    "spaceship_speed_stationary_self",
    "spaceship_pitch_self",
    "spaceship_yaw_self",
    "spaceship_roll_self",
    "spaceship_speed_rot_self",
    "spaceship_speed_rot_self_smooth",
    "spaceship_outOfControlYaw_self",
    "engine_mass_modifier",
    "spaceship_heat_self",
    "hyperdrive_charge_self",
    "hyperdrive_countdown_self",
    "supercruise_charge_self",
    "GUI_FSDCooldownProgess",
    "PlayerInsideAndEmitterAttached",
];

/// The cue of machine `machine` whose begin, end or single event is `event`, as that machine's event (a test's way to play
/// one leg's cue alone: nothing but the machines' own cue events, which are all `SHIP_EVENTS`).
fn machine_cue(rig: &ShipRig, machine: usize, event: &str) -> Option<ShipEvent> {
    use se_core::elite_ship::{Activity, KnownActivity};
    let m = rig.ship.machines.get(machine)?;
    let cue = m.states.iter().flat_map(|s| &s.activities).find_map(|a| match a {
        Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) => {
            c.sound_cues.iter().find(|q| [&q.events.begin, &q.events.end, &q.events.single].iter().any(|e| e.as_deref() == Some(event)))
        }
        _ => None,
    })?;
    let (name, ev, bones) = (cue.cue.clone(), Some(event.to_string()), cue.bones.clone());
    let event = if cue.events.end.as_deref() == Some(event) {
        MachineEvent::SoundEnd { cue: name, event: ev, bones }
    } else if cue.events.begin.as_deref() == Some(event) {
        MachineEvent::SoundBegin { cue: name, event: ev, bones }
    } else {
        MachineEvent::SoundSingle { cue: name, event: ev, bones }
    };
    Some(ShipEvent { machine, event })
}

/// `sw sound log on|off|state|emitters|cue MACHINE EVENT` (`cue`: one of that machine's own cue events posted on its
/// emitter as the machine would, for a test of one leg alone).
pub fn dev_command(world: &mut World, arg: &str) {
    if !world.contains_resource::<ShipSound>() {
        info!("sidewinder sound: not read yet (it is read at the first spawn)");
        return;
    }
    let words: Vec<&str> = arg.split_whitespace().collect();
    if let ["cue", machine, event] = words.as_slice() {
        let (machine, event) = (machine.parse::<usize>().ok(), event.to_string());
        world.resource_scope(|world, mut snd: Mut<ShipSound>| {
            let (Some(assets), Some(parts), Some(seat)) = (world.get_resource::<SidewinderAssets>(), world.get_resource::<Parts>(), world.get_resource::<Seat>()) else { return };
            match machine.and_then(|m| machine_cue(&assets.rig, m, &event)) {
                Some(e) => snd.machine_event(&e, assets, parts, &Listener::of(assets, seat)),
                None => warn!("sw sound cue: {event} is no cue event of machine {machine:?}"),
            }
        });
        return;
    }
    let mut snd = world.resource_mut::<ShipSound>();
    match words.as_slice() {
        ["log", w] => {
            snd.log = *w != "off";
            info!("sidewinder sound: log {}", if snd.log { "on" } else { "off" });
        }
        ["state"] => {
            let gps: Vec<String> = LISTED.iter().map(|n| format!("{n} {:.3}", snd.mixer.game_parameter(n, None))).collect();
            let (over, dropped) = snd.voices.over_limit();
            let limiter = snd.mixer.master_limiter().map_or("none".to_string(), |l| format!("threshold {:.2} dB, output {:+.2} dB", l.threshold_db, l.output_db));
            info!(
                "sidewinder sound: {}, {} voices (most {}), mixer at {:.2} s; the master limiter {limiter}: {over} frames over its threshold, {dropped} drops on the audio thread; {}",
                if snd.active { "playing" } else { "not playing" },
                snd.voices.count(),
                snd.most,
                snd.mixer.time(),
                gps.join(", ")
            );
            for (id, (file, u)) in &snd.now {
                info!(
                    "sidewinder sound:   voice {id} {file}: gain {:.4} ({:.1} dB, fade {:.3}), pitch {:.3}, low-pass {:.0}, high-pass {:.0}{}",
                    u.gain,
                    u.volume_db,
                    u.fade,
                    u.pitch,
                    u.lowpass,
                    u.highpass,
                    if u.virtual_voice { ", virtual" } else { "" }
                );
            }
        }
        ["emitters"] => {
            for (id, e) in &snd.cue_emitters {
                let at = snd.mixer.emitter(*id).map_or("not placed".to_string(), |[x, y, z]| format!("({x:.2}, {y:.2}, {z:.2}) m from the head"));
                info!("sidewinder sound: emitter {id:#x}: machine {} at {:?} (rig nodes {:?}), {at}", e.machine, e.bones, e.nodes);
            }
            info!("sidewinder sound: every other emitter (the drive loops 0x100-0x107, the cockpit positions 0x200-0x208, the cockpit's origin 0x300, the voice 0x302) is the listener's own: centred");
        }
        _ => warn!("sw sound: log on|off | state | emitters | cue MACHINE EVENT"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> Arc<SoundBank> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/sounds.json");
        Arc::new(se_core::elite_sound::parse_sounds(&std::fs::read(path).unwrap()).unwrap())
    }

    /// Every event the triggers can post is in the shipped file, and the normal-space start plays the loops.
    #[test]
    #[ignore]
    fn the_triggers_events_are_shipped_and_the_loops_play() {
        let bank = shipped();
        let mut m = Mixer::new(bank.clone(), 1);
        let mut t = Triggers::default();
        let v = ShipView::at_rest();
        let mut missing = Vec::new();
        let mut frames = |t: &mut Triggers, m: &mut Mixer, v: &ShipView, n: usize| {
            let mut audible = 0;
            for _ in 0..n {
                let mut calls = Vec::new();
                t.step(v, &mut calls);
                for c in &calls {
                    match c {
                        Call::Post(name, e) => {
                            if m.post(name, e.id()).is_err() {
                                missing.push(name.clone());
                            }
                        }
                        Call::State(g, s) => m.set_state(g, s),
                        Call::Gp(name, x) => m.set_game_parameter(name, *x, None),
                    }
                }
                audible = m.step(v.dt as f32).iter().filter(|u| !u.stop && !u.virtual_voice && u.gain > 1e-4).count();
            }
            audible
        };
        let rest = frames(&mut t, &mut m, &v, 300);
        // The drives' bus meter (28bc85e1) and the cockpit beds' switch on it (On at -50.34 and under).
        let meter = m.game_parameter_id(se_core::elite_sound::Id(0x1e5b_a700), None);
        for u in m.step(0.0).iter().filter(|u| matches!(u.output, Output::Audio { .. })) {
            eprintln!("  at rest: voice {} {:?} emitter {:#x} {:.1} dB fade {:.2}{}", u.voice, u.output, u.emitter, u.volume_db, u.fade, if u.virtual_voice { " virtual" } else { "" });
        }
        let mut moving = v;
        moving.motion = super::super::facts::SoundMotion { omega: [0.2, 0.1, 0.3], physics_dt: 1.0 / 60.0, ..triggers::stock(2.0, [0.0, 0.0, -150.0]) };
        moving.throttle = 0.7;
        let flying = frames(&mut t, &mut m, &moving, 300);
        drop(frames);
        assert!(missing.is_empty(), "{missing:?}");
        let meter_flying = m.game_parameter_id(se_core::elite_sound::Id(0x1e5b_a700), None);
        eprintln!("audible voices: {rest} at rest, {flying} at 150 m/s; the drives' meter {meter:.1} dB at rest, {meter_flying:.1} flying");
        assert!(flying > 0);
    }

    /// T5e's offline check run on the code as built (T5d-6): the flight itself (`elite_sim::flight::step`, a tick a frame at
    /// 60 frames a second, assist on, the gear up, the stock mass) flown to full throttle on 0, 2 and 4 ENG pips, its numbers
    /// handed over as `ShipFacts` does (`SoundMotion::new`), the triggers and the shipped data's mixer: the drive layer
    /// 02ad84c3's pitch and level on each plateau, then on the ENG 2 plateau a full pitch, yaw and roll and a half-rate pitch
    /// held until the spin settles: the turning layer 12567705's level. Prints its table (`--nocapture`; T5d-6's numbers:
    /// `re/elite/work/sound/t5d6/offline.txt`).
    #[test]
    #[ignore]
    fn t5e_offline_check_on_the_built_code() {
        use elite_sim::data::{sidewinder_block, FUEL_TANK_T, JOURNAL_UNLADEN_MASS, SIDEWINDER};
        use elite_sim::flight::{step, Input, Motion};
        use se_core::elite_sound::Id;
        const DRIVE: Id = Id(0x02ad_84c3);
        const TURN: Id = Id(0x1256_7705);
        let bank = shipped();
        let mass = JOURNAL_UNLADEN_MASS + FUEL_TANK_T + SIDEWINDER.reserve_fuel;
        let dt = 1.0 / 60.0;
        // (voice, dB, cents) of a source among this step's updates.
        let find = |u: &[VoiceUpdate], id: Id| {
            u.iter().find(|u| matches!(u.output, Output::Audio { source } if source == id) && !u.stop).map(|u| (20.0 * f64::from(u.gain).max(1e-12).log10(), u.pitch_cents, u.virtual_voice))
        };
        let show = |what: &str, u: &[VoiceUpdate]| {
            let f = |x: Option<(f64, f32, bool)>| x.map_or("not playing".to_string(), |(db, c, virt)| format!("{db:6.1} dB {c:6.0} c{}", if virt { " virtual" } else { "" }));
            eprintln!("{what:<34} drive 02ad84c3 {}   turning 12567705 {}", f(find(u, DRIVE)), f(find(u, TURN)));
        };
        let full = Input { forward: 1.0, flight_assist: true, ..Input::default() };
        // From a fresh start: 25 s at full throttle, then `then` for 4 s; the mixer and the last step's voices.
        let run = |eng: f64, then: Input| {
            let (block, full_top) = sidewinder_block(eng, mass);
            let mut m = Mixer::new(bank.clone(), 1);
            let mut t = Triggers::default();
            let mut motion = Motion::default();
            let mut v = ShipView { gear_down: false, pips: [2.0, eng, 4.0 - eng], ..ShipView::at_rest() };
            let mut last = Vec::new();
            for (secs, input) in [(25.0, full), (4.0, then)] {
                for _ in 0..(secs / dt) as usize {
                    step(&block, full_top, false, &input, glam::DVec3::ZERO, &mut motion, dt);
                    let vel = (motion.rot.inverse() * motion.vel).to_array();
                    v.motion = super::super::facts::SoundMotion::new(&block, full_top, vel, motion.omega.to_array(), 1, mass);
                    v.throttle = input.forward;
                    let mut calls = Vec::new();
                    t.step(&v, &mut calls);
                    for c in &calls {
                        match c {
                            Call::Post(name, e) => {
                                let _ = m.post(name, e.id());
                            }
                            Call::State(g, s) => m.set_state(g, s),
                            Call::Gp(name, x) => m.set_game_parameter(name, *x, None),
                        }
                    }
                    last = m.step(dt as f32).to_vec();
                }
            }
            eprintln!("  {:.1} m/s (top {:.1}, full-power {:.1}), speed_relative {:.1}", motion.vel.length(), block.top_speed, full_top, m.game_parameter("spaceship_speed_relative_self", None));
            (m, last)
        };
        for (eng, label) in [(2.0, "s01 plateau, ENG 2"), (4.0, "s03 plateau, ENG 4"), (0.0, "s04 plateau, ENG 0")] {
            show(label, &run(eng, full).1);
        }
        for (name, input) in [
            ("s01 + full pitch", Input { pitch: 1.0, ..full }),
            ("s01 + full yaw", Input { yaw: 1.0, ..full }),
            ("s01 + full roll", Input { roll: 1.0, ..full }),
            // (At the top speed the blue zone holds pitch to 0.35 of its raw 39 deg/s: a stick of 0.175 asks half of that.)
            ("s01 + half-rate pitch", Input { pitch: 0.175, ..full }),
        ] {
            let (m, u) = run(2.0, input);
            let shares = ["spaceship_pitch_self", "spaceship_yaw_self", "spaceship_roll_self", "spaceship_speed_rot_self"].map(|n| m.game_parameter(n, None));
            show(&format!("{name} ({:.2} {:.2} {:.2}, rot {:.2})", shares[0], shares[1], shares[2], shares[3]), &u);
        }
    }

    /// The one effect of the shipped data that is not exact for want of modulators: the ship voice's Guitar Distortion
    /// `d1b95c6e` (node `1d2c221f`), its wet share (parameter 65) the LFO modulator `2bf46665`'s curve.
    #[test]
    fn the_only_effect_not_exact_is_the_voice_distortion() {
        use se_core::elite_sound::Id;
        assert_eq!(shipped().effects_not_exact(), vec![(Id(0xd1b9_5c6e), Id(0x2bf4_6665), se_core::elite_sound::GD_PARAM_WET)]);
    }

    /// Nothing the triggers post is an `OPEN_EVENTS` event, and everything they post is a `SHIP_EVENTS` one: the triggers
    /// run through every state the flight can give them (each cockpit switch both ways, the boost's stages and a refusal,
    /// heat and hull falling, the shields, the canopy and the mask, the engines, a glide, the drive's supercruise and jump
    /// with their drops, cancels and refusals), and every name posted is checked against the shipped file's two lists.
    #[test]
    fn nothing_open_is_ever_posted() {
        use elite_sim::drive::Drive;
        let bank = shipped();
        let mut t = Triggers::default();
        let mut posted = HashSet::new();
        let mut run = |t: &mut Triggers, v: &ShipView, frames: usize| {
            for _ in 0..frames {
                let mut calls = Vec::new();
                t.step(v, &mut calls);
                for c in calls {
                    if let Call::Post(n, _) = c {
                        posted.insert(n);
                    }
                }
            }
        };
        let mut v = ShipView::at_rest();
        run(&mut t, &v, 5);
        for flip in 0..2 {
            let on = flip == 0;
            v.flight_assist = !on;
            v.gear_down = !on;
            v.lights = on;
            v.scoop = on;
            v.silent = on;
            v.glide = on;
            v.engines_on = !on;
            run(&mut t, &v, 240);
        }
        for stage in [1u8, 2, 3, 0] {
            v.boost = stage;
            run(&mut t, &v, 30);
        }
        v.boost_refused = true;
        run(&mut t, &v, 1);
        v.boost_refused = false;
        run(&mut t, &v, 2);
        for h in [0.5, 1.25, 1.6, 0.3] {
            v.heat = h;
            run(&mut t, &v, 10);
        }
        for h in [0.9, 0.5, 0.2] {
            v.hull = h;
            run(&mut t, &v, 2);
        }
        for (up, powered) in [(false, true), (true, true), (false, false), (true, true)] {
            v.shields_up = up;
            v.shields_powered = powered;
            run(&mut t, &v, 2);
        }
        v.canopy_breached = true;
        v.oxygen_mask = true;
        run(&mut t, &v, 2);
        v.canopy_breached = false;
        v.oxygen_mask = false;
        v.jump_pressed = true;
        run(&mut t, &v, 1);
        v.jump_pressed = false;
        let mut drive = |t: &mut Triggers, v: &mut ShipView, mode: u8, d: Drive, frames: usize| {
            v.mode = mode;
            v.drive = d;
            run(t, v, frames);
        };
        // A supercruise charge cancelled, one that goes in, the drop with both messages, the cooldown.
        drive(&mut t, &mut v, 1, Drive::Charging(1.0), 60);
        v.jump_pressed = true;
        drive(&mut t, &mut v, 1, Drive::Ready, 1);
        v.jump_pressed = false;
        for left in (0..5).rev() {
            drive(&mut t, &mut v, 1, Drive::Charging(f64::from(left)), 60);
        }
        drive(&mut t, &mut v, 2, Drive::Supercruise, 60);
        for safe in [Some(false), Some(true), None] {
            v.safe_drop = safe;
            drive(&mut t, &mut v, 2, Drive::Supercruise, 5);
        }
        drive(&mut t, &mut v, 1, Drive::Arriving(4.0), 60);
        drive(&mut t, &mut v, 1, Drive::Cooldown(4.0), 60);
        drive(&mut t, &mut v, 1, Drive::Ready, 5);
        // A jump: cancelled once, then charged through the countdown, the tunnel, arriving in supercruise, the drop.
        drive(&mut t, &mut v, 1, Drive::JumpCharging(3.0), 30);
        drive(&mut t, &mut v, 1, Drive::Ready, 5);
        for left in (0..15).rev() {
            drive(&mut t, &mut v, 1, Drive::JumpCharging(f64::from(left) + 0.5), 60);
        }
        for left in (0..5).rev() {
            drive(&mut t, &mut v, 1, Drive::Departing(f64::from(left) + 0.5), 60);
        }
        drive(&mut t, &mut v, 3, Drive::Tunnel, 120);
        drive(&mut t, &mut v, 2, Drive::Arrived(5.0), 120);
        drive(&mut t, &mut v, 2, Drive::Supercruise, 60);
        drive(&mut t, &mut v, 1, Drive::Arriving(4.0), 60);
        drive(&mut t, &mut v, 1, Drive::Ready, 5);
        let mut calls = Vec::new();
        t.teardown(&mut calls);
        posted.extend(calls.into_iter().filter_map(|c| if let Call::Post(n, _) = c { Some(n) } else { None }));
        let open: HashSet<se_core::elite_sound::Id> = bank.file.open_events.iter().map(|n| se_core::elite_sound::event_id(n)).collect();
        let ship: HashSet<se_core::elite_sound::Id> = bank.file.ship_events.iter().map(|n| se_core::elite_sound::event_id(n)).collect();
        let mut names: Vec<&String> = posted.iter().collect();
        names.sort();
        let opened: Vec<&&String> = names.iter().filter(|n| open.contains(&se_core::elite_sound::event_id(n))).collect();
        let unlisted: Vec<&&String> = names.iter().filter(|n| !ship.contains(&se_core::elite_sound::event_id(n))).collect();
        assert!(opened.is_empty(), "OPEN_EVENTS posted: {opened:?}");
        assert!(unlisted.is_empty(), "posted but not in SHIP_EVENTS: {unlisted:?}");
        assert!(names.len() > 100, "the sweep posts {} events", names.len());
    }

    /// The gear motors' attenuation (`258639ce`, on 38 of the cues' sounds) read at three distances against the shipped
    /// data: its VolumeDry curve (dB scaling: linear gain - 1) is 0 dB at 0 m, `20 log10(1 - 0.67647)` dB at its middle
    /// point 49.5238 m, silence (the -96.3 dB floor) from 300 m; its LowPassFilter 0, 21 at 50.4762 m, 73 at 300 m; its
    /// Spread 50 at 0 m, 0 at 300 m.
    #[test]
    fn an_attenuation_read_at_three_distances() {
        use se_core::elite_sound::{Id, Object, Placement};
        let bank = shipped();
        let Some(Object::Attenuation(att)) = bank.file.objects.get(&Id(0x2586_39ce)) else { panic!("no attenuation 258639ce") };
        let p = Placement::new(true, Some(att)).unwrap();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(close(p.volume_db(0.0), 0.0));
        assert!(close(p.volume_db(49.5238), 20.0 * (1.0f64 - 0.67647).log10()), "{}", p.volume_db(49.5238));
        assert!(close(p.volume_db(300.0), se_core::elite_sound::curve::DB_FLOOR));
        assert!(close(p.lowpass_at(0.0), 0.0) && close(p.lowpass_at(50.4762), 21.0) && close(p.lowpass_at(300.0), 73.0));
        assert!(close(p.spread_at(0.0), 50.0) && close(p.spread_at(300.0), 0.0));
        assert!(p.highpass.is_none());
        // Within the ship (the legs are 3 to 8 m from the head): what the distance costs, falling with it.
        let mut last = 0.0;
        for d in [3.0, 8.0] {
            eprintln!("258639ce at {d} m: {:+.2} dB, low-pass {:+.2}, spread {:.1}", p.volume_db(d), p.lowpass_at(d), p.spread_at(d));
            assert!(p.volume_db(d) < last);
            last = p.volume_db(d);
        }
    }

    /// A gear motor's cue on a placed emitter: its voices carry their placement (panned, the attenuation's curves) and their
    /// gain leaves the distance out (the output applies it), so the same cue 3 m and 30 m away has the same gain; the cue on
    /// an emitter the game never placed is not placed and plays as before.
    #[test]
    fn a_cue_on_a_placed_emitter_is_placed() {
        let bank = shipped();
        let starts = |at: Option<[f32; 3]>| {
            let mut m = Mixer::new(bank.clone(), 1);
            m.set_game_parameter("PlayerInsideAndEmitterAttached", triggers::INSIDE, None);
            if let Some(p) = at {
                m.set_emitter(0x1000, p);
            }
            m.post("SideWinder_Rear_Deploy_Motor01_b_landinggear", 0x1000).unwrap();
            let mut got = Vec::new();
            for _ in 0..20 {
                got.extend(m.step(0.02).iter().filter(|u| u.start.is_some() && !u.virtual_voice).cloned());
            }
            got
        };
        let (near, far, centred) = (starts(Some([-3.0, 0.0, 0.0])), starts(Some([-30.0, 0.0, 0.0])), starts(None));
        assert!(!near.is_empty() && near.len() == far.len() && near.len() == centred.len(), "{} {} {}", near.len(), far.len(), centred.len());
        for ((n, f), c) in near.iter().zip(&far).zip(&centred) {
            let p = n.place.as_ref().expect("placed");
            assert!(p.pan && p.volume.is_some(), "{p:?}");
            assert!((n.gain - f.gain).abs() < 1e-6, "the distance is not in the gain: {} against {}", n.gain, f.gain);
            assert!(p.volume_db(30.0) < p.volume_db(3.0));
            assert!(c.place.is_none() && c.position.is_none());
        }
    }

    /// How many shipped events start a voice with a non-zero low-pass or high-pass.
    #[test]
    #[ignore]
    fn count_events_with_filters() {
        let bank = shipped();
        let mut n = 0;
        for name in &bank.file.ship_events {
            let mut m = Mixer::new(bank.clone(), 1);
            m.post(name, 1).unwrap();
            let mut filtered = false;
            for _ in 0..30 {
                filtered |= m.step(0.05).iter().any(|u| u.lowpass > 0.0 || u.highpass > 0.0);
            }
            n += filtered as usize;
        }
        eprintln!("{n} of {} shipped events start a voice with a filter value over 0", bank.file.ship_events.len());
    }
}
