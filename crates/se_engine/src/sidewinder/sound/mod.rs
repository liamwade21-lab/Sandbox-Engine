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
//! (no event of the banks), the supercruise speed and SCO parameters (not decompiled), and the gear's and doors' state
//! machine cues (`ShipSound::post_cue`, for the animation task). Left at the banks' defaults, their setters found but not
//! what they read (`triggers.rs`, `unset_engine_syncs`): `AudioEngineClass`, `engine_mass_modifier`,
//! `spaceship_speed_stationary_self`.
//!
//! **Emitters have no positions, so every voice plays centred:** the loops' emitters (`ship.json`'s `Internal_MainDrive_Left`,
//! `Cockpit_Back_Centre`, `Internal_Cockpit_Origin` ... 21 names) are Elite's audio emitter components, named by the
//! entity's template (not at rest); no bone or locator of the shipped models carries those names (the hull's 156 bones and
//! the cockpit's 148 checked, T5c), so where each hangs is not proven. 112 of the shipped events play 3D-positioned
//! recordings (positioning Emitter, spatialised) and all sound centred until their emitters are found. The state
//! machines' cues name their bones (`ShipSound::post_cue` gives the mixer their places, for its distance curves; the
//! player still plays them centred).
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
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use se_core::catalog::ObjectRef;
use se_core::elite_sound::{curve, wwise_id, Mixer, Output, SoundBank, VoiceUpdate};

use crate::mods::ModControl;
use crate::sound::SoundSettings;

use super::parts::Parts;
use super::pilot::{Pilot, PilotKeys, TICK};
use super::seat::Seat;
use super::{Sidewinder, SidewinderAssets};
use load::{Clip, Loaded, Rumble};
use triggers::{Call, Emitter, Flight, ShipView, Triggers};
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
    /// The flight's velocity (its rest frame) and tick count when last read, and the acceleration worked out from them.
    last_vel: Option<(u64, DVec3)>,
    acc: DVec3,
    /// Motion Generator voices: their age (s) and source.
    rumbling: HashMap<u32, (f64, se_core::elite_sound::Id)>,
    rumble_on: bool,
    /// What each voice is now, for `sw sound state`; the most voices at once.
    now: BTreeMap<u32, (String, VoiceUpdate)>,
    most: usize,
    /// Unknown events and unbuilt outputs already logged.
    warned: HashSet<String>,
    pub log: bool,
    /// When the audio thread's heartbeat was last logged, and its frame count then.
    beat: (f64, u64),
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
    app.add_systems(
        PostUpdate,
        (
            note_presses.before(super::hyper::tunnel),
            drive.after(super::seat::seat_camera).before(crate::camera::sync_camera_transform),
        )
            .run_if(resource_exists::<ShipSound>)
            .in_set(crate::universe::UniverseSet::Camera),
    );
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
            last_vel: None,
            acc: DVec3::ZERO,
            rumbling: HashMap::new(),
            rumble_on: false,
            now: BTreeMap::new(),
            most: 0,
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
                Call::State(g, s) => {
                    if self.log {
                        info!("sidewinder sound: state {g} = {s}");
                    }
                    self.mixer.set_state(g, s);
                }
                Call::Gp(n, v) => self.mixer.set_game_parameter(n, *v, None),
            }
        }
    }

    /// Posts a state machine's sound cue (`<cue>_b_<suffix>` or `_e`) at a set of bones: the emitter is placed at their
    /// middle from the pilot's eye in the ship's space. For the animation task (the gear's and doors' cues of `ship.json`).
    #[allow(dead_code)]
    pub fn post_cue(&mut self, cue: &str, suffix: &str, begin: bool, bones: &[usize], parts: &Parts, eye: [f32; 3]) {
        if !self.active || bones.is_empty() {
            return;
        }
        let mid = bones.iter().map(|b| parts.matrix(*b).w_axis.truncate()).sum::<Vec3>() / bones.len() as f32;
        let e = Emitter::Cue(wwise_id(cue).0 | 0x8000_0000);
        self.mixer.set_emitter(e.id(), (mid - Vec3::from_array(eye)).to_array());
        self.call(&[Call::Post(format!("{cue}_{}_{suffix}", if begin { "b" } else { "e" }), e)]);
    }

    /// Every voice stopped, the mixer made afresh, the triggers back to before the audio starts.
    fn stop(&mut self, commands: &mut Commands, pads: &Query<Entity, With<Gamepad>>, rumble: &mut MessageWriter<GamepadRumbleRequest>) {
        let mut calls = Vec::new();
        self.triggers.teardown(&mut calls);
        self.call(&calls);
        self.voices.stop_all(commands);
        self.seeds += 1;
        self.mixer = Mixer::new(self.bank.clone(), self.seeds);
        self.active = false;
        self.last_vel = None;
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

/// The ship's state as the triggers read it (the feeders: what fills Elite's audio component is not known, so each field
/// is read from the flight as `triggers.rs` says).
fn view(pilot: &Pilot, presses: &Presses, dt: f64, acc: DVec3) -> ShipView {
    use elite_sim::boost::State;
    use elite_sim::power::{ENG, SYS, WEP};
    let rot = DQuat::from_array(pilot.motion.rot.to_array());
    let vel = rot.inverse() * DVec3::from_array(pilot.motion.vel.to_array());
    let acc = rot.inverse() * acc;
    // (The gear down or the scoop out halve the top speed, as the flight's own block has it: `pilot.rs`.)
    let speed_k = elite_sim::deployables::speed_factor(pilot.gear_down, pilot.scoop);
    let (block, _) = elite_sim::data::sidewinder_block_with(pilot.dist.pips(ENG), pilot.mass_t(), speed_k);
    let flight = Flight {
        vel: vel.to_array(),
        acc: acc.to_array(),
        omega: pilot.motion.omega.to_array(),
        top_speed: block.top_speed,
        accel_fwd: block.accel_fwd,
        back_share: block.back_share,
        throttle: pilot.throttle,
    };
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
        motion: triggers::motion_in(&flight),
        pips: [pilot.dist.pips(SYS), pilot.dist.pips(ENG), pilot.dist.pips(WEP)],
    }
}

/// Once a frame after the seat and the flight: the triggers, the mixer, the voices.
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
    settings: Res<SoundSettings>,
    mut outputs: ResMut<Assets<ShipOutput>>,
    pads: Query<Entity, With<Gamepad>>,
    mut rumble: MessageWriter<GamepadRumbleRequest>,
) {
    let snd = &mut *snd;
    snd.voices.service(&mut commands);
    let parked = ship.parked.as_ref().map(|p| (p.obj, p.pos));
    let playing = seat.seated && control.pilot && parked.is_some() && !pilot.destroyed;
    if !playing {
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
    // The acceleration from the flight's ticks (a frame may hold none or several).
    let vel = DVec3::from_array(pilot.motion.vel.to_array());
    match snd.last_vel {
        Some((ticks, last)) if pilot.ticks > ticks => snd.acc = (vel - last) / ((pilot.ticks - ticks) as f64 * TICK),
        Some(_) => {}
        None => snd.acc = DVec3::ZERO,
    }
    if snd.last_vel.is_none_or(|(t, _)| pilot.ticks != t) {
        snd.last_vel = Some((pilot.ticks, vel));
    }
    let v = view(&pilot, &presses, dt, snd.acc);
    let mut calls = Vec::new();
    snd.triggers.step(&v, &mut calls);
    snd.call(&calls);
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
                            "sidewinder sound: voice {} starts {file} on emitter {:#x}: gain {:.4} ({:.1} dB), pitch {:.3}, low-pass {:.0}, high-pass {:.0}{}{}",
                            u.voice,
                            u.emitter,
                            u.gain,
                            u.volume_db,
                            u.pitch,
                            u.lowpass,
                            u.highpass,
                            u.start.map_or(String::new(), |s| if s.looping { format!(", looping {:?}", s.loop_count) } else { String::new() }),
                            if u.virtual_voice { ", virtual" } else { "" }
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
    // The audio thread's heartbeat, once a second (`sw sound log on`): frames its voices made, samples it found not finite.
    let t = time.elapsed_secs_f64();
    if snd.log && t - snd.beat.0 >= 1.0 {
        let (made, bad, skipped, longest_us, refused, top) = snd.voices.meter();
        let (over, dropped) = snd.voices.over_limit();
        let top: Vec<String> = top.iter().map(|(id, f, n, s)| format!("{id} {f} {n} at {s:.2}")).collect();
        info!(
            "sidewinder sound: audio thread made {} frames in {:.2} s over {} voices ({bad} samples not finite, {refused} voices refused, {over} frames over the master limiter's threshold, {dropped} drops on the audio thread so far; {skipped} frames skipped, the longest skip {:.1} ms; most: {})",
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

/// The game parameters `sw sound state` lists.
const LISTED: &[&str] = &[
    "spaceship_speed_self",
    "spaceship_speed_physical_self",
    "spaceship_speed_relative_self",
    "spaceship_accel_self",
    "spaceship_accel_relative_smooth_self",
    "spaceship_accel_directional_self",
    "spaceship_target_speed_relative_self",
    "spaceship_pitch_self",
    "spaceship_yaw_self",
    "spaceship_roll_self",
    "spaceship_speed_rot_self",
    "spaceship_speed_rot_self_smooth",
    "spaceship_heat_self",
    "hyperdrive_charge_self",
    "hyperdrive_countdown_self",
    "supercruise_charge_self",
    "GUI_FSDCooldownProgess",
    "PlayerInsideAndEmitterAttached",
];

/// `sw sound log on|off|state`.
pub fn dev_command(world: &mut World, arg: &str) {
    let Some(mut snd) = world.get_resource_mut::<ShipSound>() else {
        info!("sidewinder sound: not read yet (it is read at the first spawn)");
        return;
    };
    let words: Vec<&str> = arg.split_whitespace().collect();
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
        _ => warn!("sw sound: log on|off | state"),
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
        moving.motion = triggers::motion_in(&Flight { vel: [0.0, 0.0, -150.0], acc: [0.0, 0.0, -20.0], omega: [0.2, 0.1, 0.3], top_speed: 220.0, accel_fwd: 40.0, back_share: 0.5, throttle: 0.7 });
        let flying = frames(&mut t, &mut m, &moving, 300);
        drop(frames);
        assert!(missing.is_empty(), "{missing:?}");
        let meter_flying = m.game_parameter_id(se_core::elite_sound::Id(0x1e5b_a700), None);
        eprintln!("audible voices: {rest} at rest, {flying} at 150 m/s; the drives' meter {meter:.1} dB at rest, {meter_flying:.1} flying");
        assert!(flying > 0);
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
