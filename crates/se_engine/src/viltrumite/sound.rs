//! His sounds (spec F10): four loops whose levels go by the air, and one-shots, at most 8 at a time; where there's no air
//! all of it is silent (D12, the user's 2026-11-12: fading out over `SHOT_AIR_PA`). Non-spatial, times `SoundSettings`, voices as `ship/engine_sound.rs`. Everything goes by real seconds
//! (F5): paused and at x1000 he sounds the same.
//!
//! The loops are spawned when he's turned on, paused and silent, and stay for as long as he's on: never a voice a frame.
//! Turned off, every voice fades over `SOUND_FADE_S` and is despawned (removing `AudioSink` alone restarts a sound), and
//! then nothing here runs. Losing the focus fades and pauses them, and they go on from there.
//!
//! q, Mach and the temperature come from `air.rs` alone (`Hero::flow`, `Hero::heat_k`). Whoever knows a thing happened
//! calls its function on `Hero::sound` (`Cues`), once; `play_sounds` plays what was asked for that frame. With no audio
//! device a voice is an entity with no `AudioSink`: nothing plays, and it goes when its time's up.
//!
//! The source mod's `wind_loop`, `superWind_start`, `wind_end` and `wind_attack_*` are its super breath, a non-goal: never
//! used for flight, and not in the copy (the user, 2026-11-07; `no_breath_sounds` fails if one comes back).

use std::io::Write as _;
use std::sync::Arc;

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::mods::ModControl;
use crate::sound::SoundSettings;

use super::air::Flow;
use super::hero::State;
use super::{Hero, tune};

macro_rules! sounds {
    ($($name:ident $file:literal,)*) => {
        /// The mod's sound files (`sfx/NAME.ogg`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Sfx { $($name,)* }
        const ALL: &[Sfx] = &[$(Sfx::$name,)*];
        const FILES: &[&str] = &[$($file,)*];
    };
}

sounds! {
    FastLoop "fastFlight_loop",
    FasterLoop "fasterFlight_loop",
    HumLoop "sentry_flightfx_loop",
    ChargeLoop "flightCharge_loop",
    SupersonicStart "supersonicStart",
    SonicBoom "sonic_boom_v2",
    SupersonicEnd "supersonicEnd",
    FastStart "fastFlight_start",
    FastFadeoff "fastFlight_fadeoff",
    FasterFadeoff "fasterFlight_fadeoff",
    Launch "superSonic_launch_1",
    JumpSmall "basic_move_sound_short_1",
    JumpExp "jump_exp",
    FlightEnd "sentry_flightfx_loop_end",
    ChargeStart "flightCharge_start",
    MeleeStart1 "melee_start_1",
    MeleeStart3 "melee_start_3",
    Punch1 "punch_1",
    Punch2 "punch_2",
    Punch3 "punch_3",
    HitBuilding "hPunch_1",
    ExplodeGround "explode_ground1",
    Smash "sentry_tk_Smash_hit1",
    Clap1 "thunderclap1",
    Clap2 "thunderclap2",
    Clap3 "thunderclap3",
    Shockwave "sentry_tk_shockwave1",
    PunchCharged1 "bPunch_1",
    PunchCharged2 "bPunch_2",
    HitBuilding2 "hPunch_2",
    MeleeStart2 "melee_start_2",
    MeleeStart4 "melee_start_4",
}

impl Sfx {
    fn file(self) -> &'static str {
        FILES[self as usize]
    }

    /// The least time between two of this sound (s): a tapped key doesn't pile up long ones.
    fn gap_s(self) -> f64 {
        match self {
            Sfx::Launch | Sfx::FastStart | Sfx::FastFadeoff | Sfx::FasterFadeoff | Sfx::FlightEnd => tune::WHOOSH_GAP_S,
            _ => tune::SHOT_GAP_S,
        }
    }
}

/// The loops, in the order of `loop_levels`.
const LOOPS: [Sfx; 4] = [Sfx::FastLoop, Sfx::FasterLoop, Sfx::HumLoop, Sfx::ChargeLoop];

/// 0 to 1 as `x` goes from the first to the second, straight.
fn ramp(x: f64, (from, to): (f64, f64)) -> f32 {
    (((x - from) / (to - from)).clamp(0.0, 1.0)) as f32
}

/// How much air the sounds count here, every loop's and one-shot's share of its volume: 0 in vacuum (D12: silent), 1 in air,
/// logarithmically in the pressure between `SHOT_AIR_PA`.
pub fn air_share(pressure_pa: f64) -> f32 {
    let log_p = if pressure_pa > 0.0 { pressure_pa.log10() } else { f64::NEG_INFINITY };
    ramp(log_p, (tune::SHOT_AIR_PA.0.log10(), tune::SHOT_AIR_PA.1.log10()))
}

/// `fastFlight_loop`'s pitch by Mach: low at low speed, where it stands in for the air going past.
fn fast_pitch(mach: f64) -> f32 {
    let log_m = if mach > 0.0 { mach.log10() } else { f64::NEG_INFINITY };
    let k = ramp(log_m, (tune::FAST_PITCH_MACH.0.log10(), tune::FAST_PITCH_MACH.1.log10()));
    tune::FAST_PITCH.0 + (tune::FAST_PITCH.1 - tune::FAST_PITCH.0) * k
}

/// What the loops go by this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub flow: Flow,
    /// The low-passed temperature (K).
    pub heat_k: f64,
    /// Hovering or flying: not falling, not on his feet. (Only the hum minds, and only in air: `loop_levels`.)
    pub flying: bool,
    /// A charge runs: the right button's clap, Space's launch or stack, the left button's punch.
    pub charging: bool,
    /// How full it is (0..1).
    pub charge: f32,
    /// The user's mod's loops: fast flight (`fastFlight_loop` by its speed under the barrier, `fasterFlight_loop` past it, more
    /// and higher by the stacks). `share`: fast flight's speed in its range (`controls::speed_share`).
    pub fast: bool,
    pub supersonic: bool,
    pub stacks: u32,
    pub share: f32,
}

/// F10's table: each loop's volume (0..1, before `SoundSettings`) and pitch. The hum is for flying. Every loop takes the air's share:
/// none where there's no air (D12), whatever q or the heat say (q and a skin's heat can be high in air too thin to carry sound).
pub fn loop_levels(r: &Reading) -> [(f32, f32); 4] {
    let log_q = r.flow.log_q();
    let air = air_share(r.flow.air.pressure_pa);
    let hums = if r.flying { 1.0 } else { 0.0 };
    let lerp = |(a, b): (f32, f32), k: f32| a + (b - a) * k.clamp(0.0, 1.0);
    // The user's mod: the fast loop through fast flight, the roar past the barrier, in air (the user, 2026-11-07: "there is no wind
    // sound effect as he is going very fast thru the atmosphere").
    let stacks = r.stacks.min(tune::STACKS_MAX) as f32;
    let fast_mode = if r.fast && !r.supersonic { lerp(tune::FAST_LOOP_LEVEL, r.share) } else { 0.0 };
    let roar = if r.fast && r.supersonic { (lerp(tune::ROAR_LEVEL, r.share) + tune::ROAR_STACK * stacks).min(1.0) } else { 0.0 };
    [
        (ramp(log_q, tune::FAST_LOG_Q).max(fast_mode) * air, fast_pitch(r.flow.mach)),
        (ramp(r.heat_k, tune::FASTER_HEAT_K).max(roar) * air, 1.0 + tune::ROAR_STACK * stacks),
        (tune::HUM_VOLUME * hums * air * (1.0 - ramp(log_q, tune::HUM_LOG_Q)), tune::HUM_PITCH),
        // (As the user's mod: 0.45 swelling to 0.95 as it fills.)
        (if r.charging { air * (0.45 + 0.5 * r.charge.clamp(0.0, 1.0)) } else { 0.0 }, 1.0),
    ]
}

/// One sound asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cue {
    sfx: Sfx,
    gain: f32,
    pitch: f32,
}

/// What's asked to sound this frame: one function per trigger of F10's table. Each is meant to be called once, the frame it
/// happens; the same sound twice in a frame is one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cues {
    asked: [Option<Cue>; tune::SHOTS_MAX],
    /// Which swing and which punch come next (no two alike running).
    swings: u8,
    blows: u8,
}

impl Cues {
    fn ask(&mut self, sfx: Sfx, gain: f32, pitch: f32) {
        for slot in &mut self.asked {
            match slot {
                Some(c) if c.sfx == sfx => {
                    c.gain = c.gain.max(gain);
                    return;
                }
                Some(_) => {}
                None => {
                    *slot = Some(Cue { sfx, gain, pitch });
                    return;
                }
            }
        }
    }

    /// `jump_exp`, a little quieter: a tap of Space takes him off the ground (the user's mod's take-off).
    pub fn take_off_ground(&mut self) {
        self.ask(Sfx::JumpExp, 0.8, 1.0);
    }

    /// The charged punch lands: `bPunch_1`, `bPunch_2` in turn, and the smash.
    pub fn punch_charged(&mut self) {
        self.blows = self.blows.wrapping_add(1);
        self.ask(if self.blows % 2 == 0 { Sfx::PunchCharged1 } else { Sfx::PunchCharged2 }, 1.0, 1.0);
        self.ask(Sfx::Smash, 0.8, 1.0);
    }

    /// The punch's wind-up begins: one of `melee_start_1` to `4`.
    pub fn windup(&mut self) {
        self.swings = self.swings.wrapping_add(1);
        self.ask([Sfx::MeleeStart1, Sfx::MeleeStart2, Sfx::MeleeStart3, Sfx::MeleeStart4][self.swings as usize % 4], 0.8, 1.0);
    }

    /// The punch's charge is full: `flightCharge_start`, pitched up (the user's mod).
    pub fn punch_ready(&mut self) {
        self.ask(Sfx::ChargeStart, 1.0, 1.5);
    }

    /// `superSonic_launch_1`: a charged launch.
    pub fn launch(&mut self) {
        self.ask(Sfx::Launch, 1.0, 1.0);
    }

    /// `fastFlight_start`: the boost starts.
    pub fn boost_start(&mut self) {
        self.ask(Sfx::FastStart, 1.0, 1.0);
    }

    /// `fastFlight_fadeoff`: the boost is let go.
    pub fn boost_end(&mut self) {
        self.ask(Sfx::FastFadeoff, 1.0, 1.0);
    }

    /// `fasterFlight_fadeoff`: leaving Hyper.
    pub fn leave_hyper(&mut self) {
        self.ask(Sfx::FasterFadeoff, 1.0, 1.0);
    }

    /// `basic_move_sound_short_1`, or `jump_exp` from a sprint.
    pub fn jump(&mut self, sprinting: bool) {
        self.ask(if sprinting { Sfx::JumpExp } else { Sfx::JumpSmall }, 1.0, 1.0);
    }

    /// `sentry_flightfx_loop_end`: X stops the flight.
    pub fn stop_flying(&mut self) {
        self.ask(Sfx::FlightEnd, 1.0, 1.0);
    }

    /// `flightCharge_start`: the right button goes down. (`play_sounds` calls it, and the loop and letting go are its own.)
    pub fn charge_start(&mut self) {
        self.ask(Sfx::ChargeStart, 1.0, 1.0);
    }

    /// `explode_ground1`: a hard landing.
    pub fn hard_landing(&mut self) {
        self.ask(Sfx::ExplodeGround, 1.0, 1.0);
    }

    /// `explode_ground1` low: he comes down on the sea at `speed` m/s (stepping off the shore is hardly anything).
    pub fn splash(&mut self, speed: f64) {
        let gain = tune::SPLASH_GAIN.0 + (tune::SPLASH_GAIN.1 - tune::SPLASH_GAIN.0) * ramp(speed, tune::SPLASH_SPEED_MS);
        self.ask(Sfx::ExplodeGround, gain, tune::SPLASH_PITCH);
    }

    /// Step 7: `supersonicStart` and `sonic_boom_v2`, Sub to Super, loud by `log10 q` (`q` in Pa).
    pub fn boom(&mut self, q: f64) {
        let log_q = if q > 0.0 { q.log10() } else { f64::NEG_INFINITY };
        let gain = tune::BOOM_GAIN.0 + (tune::BOOM_GAIN.1 - tune::BOOM_GAIN.0) * ramp(log_q, tune::BOOM_LOG_Q);
        self.ask(Sfx::SupersonicStart, gain, 1.0);
        self.ask(Sfx::SonicBoom, gain, 1.0);
    }

    /// Step 7: `supersonicEnd`, Super to Sub.
    pub fn supersonic_end(&mut self) {
        self.ask(Sfx::SupersonicEnd, 1.0, 1.0);
    }

    /// Step 8: `melee_start_1`, `melee_start_3` in turn: a punch's swing.
    pub fn punch_swing(&mut self) {
        self.swings = self.swings.wrapping_add(1);
        self.ask([Sfx::MeleeStart1, Sfx::MeleeStart2, Sfx::MeleeStart3, Sfx::MeleeStart4][self.swings as usize % 4], 1.0, 1.0);
    }

    /// Step 8: `punch_1`, `punch_2`, `punch_3` in turn: a punch lands.
    pub fn punch(&mut self) {
        self.blows = self.blows.wrapping_add(1);
        self.ask([Sfx::Punch1, Sfx::Punch2, Sfx::Punch3][self.blows as usize % 3], 1.0, 1.0);
    }

    /// Steps 8, 10: `hPunch_1`, a punch hits a building.
    pub fn punch_hits_building(&mut self) {
        self.blows = self.blows.wrapping_add(1);
        self.ask(if self.blows % 2 == 0 { Sfx::HitBuilding } else { Sfx::HitBuilding2 }, 1.0, 1.0);
    }

    /// Step 8: `sentry_tk_Smash_hit1`, a ground punch.
    pub fn ground_punch(&mut self) {
        self.ask(Sfx::Smash, 1.0, 1.0);
    }

    /// Step 10: `sentry_tk_Smash_hit1`, a building breaks.
    pub fn building_breaks(&mut self) {
        self.ask(Sfx::Smash, 1.0, 1.0);
    }

    /// Step 8: `thunderclap1` to `3` by the charge (0..1).
    pub fn thunderclap(&mut self, charge: f32) {
        let sfx = if charge < 1.0 / 3.0 {
            Sfx::Clap1
        } else if charge < 2.0 / 3.0 {
            Sfx::Clap2
        } else {
            Sfx::Clap3
        };
        self.ask(sfx, 1.0, 1.0);
    }

    /// Step 8: `sentry_tk_shockwave1`, under a thunderclap on the ground.
    pub fn shockwave(&mut self) {
        self.ask(Sfx::Shockwave, 1.0, 1.0);
    }

    fn clear(&mut self) {
        self.asked = [None; tune::SHOTS_MAX];
    }
}

/// Every voice of the mod has this, so a test can count them.
#[derive(Component)]
pub struct HeroVoice;

/// A loop while the mod's voices exist.
#[derive(Debug)]
struct LoopVoice {
    entity: Entity,
    sfx: Sfx,
    /// Where it's fading to its level from, and its pitch.
    level: f32,
    pitch: f32,
    /// What the sink was last told, so an unchanged value isn't sent again.
    sent_volume: f32,
    sent_pitch: f32,
}

/// A one-shot that's playing.
#[derive(Debug)]
struct Shot {
    entity: Entity,
    sfx: Sfx,
    /// Its own loudness, before the air.
    gain: f32,
    /// The air's share now (D12): it follows the air while it plays, as the loops do, so one that began in air goes silent if
    /// he leaves it.
    air: f32,
    /// 1, going to 0 once it's cut: `flightCharge_start` when the button's let go.
    fade: f32,
    cut: bool,
    /// Seconds it has been playing: `SHOT_MAX_S` ends it with no `AudioSink` to say so.
    played: f32,
    sent_volume: f32,
}

/// The mod's voices.
#[derive(Resource, Default)]
pub struct Voices {
    /// By `Sfx`, once: first on. `None`: refused by the guard or missing, so that sound stays silent.
    handles: Vec<Option<Handle<AudioSource>>>,
    loops: Vec<LoopVoice>,
    shots: [Option<Shot>; tune::SHOTS_MAX],
    /// The whole mod's fade, for turning off and the focus.
    master: f32,
    charging: bool,
    clock: f64,
    /// When each sound last started and how loud it was asked for, and how many times since the mod's voices were made.
    last_fired: Vec<(f64, f32)>,
    fired: Vec<u32>,
    /// `hero sound trace`: a test's log of every frame.
    trace: Option<std::io::BufWriter<std::fs::File>>,
}

impl Voices {
    fn alive(&self) -> bool {
        !self.loops.is_empty() || self.shots.iter().any(Option::is_some)
    }

    fn shots_alive(&self) -> usize {
        self.shots.iter().flatten().count()
    }
}

/// F10's guard: bevy_audio panics on anything but Ogg Vorbis. (Each file has also been decoded once by the Mods menu's first
/// enable, `mods::check_files`: a bad one greys the row, so it doesn't get here.)
fn load_sounds(assets: &mut Assets<AudioSource>) -> Vec<Option<Handle<AudioSource>>> {
    let handles: Vec<_> = FILES
        .iter()
        .map(|name| {
            let rel = format!("sfx/{name}.ogg");
            let bytes = crate::mods::mod_file("viltrumite", &rel).and_then(|p| std::fs::read(p).ok());
            match bytes {
                Some(b) if crate::music::is_ogg_vorbis(&b) => Some(assets.add(AudioSource { bytes: Arc::from(b) })),
                _ => {
                    warn!("viltrumite: sound {rel} is missing or isn't Ogg Vorbis: it stays silent");
                    None
                }
            }
        })
        .collect();
    info!("viltrumite: {} of {} sounds ready", handles.iter().flatten().count(), FILES.len());
    handles
}

pub(super) fn sound_running(control: Res<ModControl>, voices: Res<Voices>) -> bool {
    control.hero || voices.alive()
}

/// One frame of his sounds, after his move. Runs only while he's on or something is still fading.
#[allow(clippy::too_many_arguments)]
pub(super) fn play_sounds(
    mut commands: Commands,
    time: Res<Time<Real>>,
    settings: Res<SoundSettings>,
    control: Res<ModControl>,
    mut hero: ResMut<Hero>,
    mut voices: ResMut<Voices>,
    mut assets: ResMut<Assets<AudioSource>>,
    mut sinks: Query<&mut AudioSink>,
) {
    let voices = &mut *voices;
    // (What a frame of this costs, only while a test traces it.)
    let started = voices.trace.is_some().then(std::time::Instant::now);
    let dt = time.delta_secs().min(0.1);
    voices.clock = time.elapsed_secs_f64();
    let on = control.hero && hero.placed;
    // (Not while he's still loading and can't be seen.)
    let audible = on && !control.unfocused && !control.loading;

    if on && voices.loops.is_empty() {
        if voices.handles.is_empty() {
            voices.handles = load_sounds(&mut assets);
            voices.last_fired = vec![(f64::NEG_INFINITY, 0.0); FILES.len()];
        }
        voices.fired = vec![0; FILES.len()];
        // Each from its own pitch, or the hum's first moment is a slide down from 1.
        let at_rest = loop_levels(&Reading::default());
        for (i, sfx) in LOOPS.into_iter().enumerate() {
            let Some(handle) = voices.handles[sfx as usize].clone() else { continue };
            let pitch = at_rest[i].1;
            let entity = commands.spawn((AudioPlayer(handle), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)).with_speed(pitch).paused(), HeroVoice)).id();
            voices.loops.push(LoopVoice { entity, sfx, level: 0.0, pitch, sent_volume: 0.0, sent_pitch: pitch });
        }
    }

    let step = dt / tune::SOUND_FADE_S;
    voices.master += ((audible as u8 as f32) - voices.master).clamp(-step, step);

    // The charge itself, not the button: held over a ground punch, a clap or a Go To, no charge runs and none is heard.
    let clap = matches!(hero.action, super::impact::Action::Charge { .. });
    let punch = matches!(hero.action, super::impact::Action::WindUp { .. });
    let charging = audible && (clap || hero.space.charging || punch);
    let was_charging = std::mem::replace(&mut voices.charging, charging);
    if charging && !was_charging {
        hero.sound.charge_start();
    }

    // The loops. Turned off they keep the level they had while the master takes them down.
    let charge = if hero.space.charging {
        hero.space.charge as f32
    } else if let super::impact::Action::WindUp { held, .. } = hero.action {
        super::controls::punch_charge(held as f64) as f32
    } else {
        1.0
    };
    let speed = hero.flow.speed;
    let reading = Reading {
        flow: hero.flow,
        heat_k: hero.heat_k,
        flying: matches!(hero.state, State::Hover | State::Flight),
        charging,
        charge,
        // (A dive roars too, as in the user's mod.)
        fast: hero.fast || hero.dive,
        supersonic: super::controls::supersonic(&hero.flow, speed),
        stacks: hero.stacks,
        share: super::controls::speed_share(speed, super::controls::sound_speed(&hero.flow)) as f32,
    };
    let levels = loop_levels(&reading);
    let pitch_k = 1.0 - (-dt / tune::PITCH_TAU_S).exp();
    for voice in &mut voices.loops {
        let i = LOOPS.iter().position(|l| *l == voice.sfx).unwrap_or(0);
        if on {
            let (level, pitch) = levels[i];
            voice.level += (level - voice.level).clamp(-step, step);
            voice.pitch += (pitch - voice.pitch) * pitch_k;
        }
        let volume = (voice.level * voices.master * settings.volume * tune::SOUND_GAIN * tune::LOOP_GAIN[i]).min(tune::LOOP_MOST[i]);
        let Ok(mut sink) = sinks.get_mut(voice.entity) else { continue };
        if volume <= tune::SOUND_SILENT {
            if !sink.is_paused() {
                sink.pause();
            }
            continue;
        }
        if (volume - voice.sent_volume).abs() > tune::SOUND_SILENT {
            sink.set_volume(Volume::Linear(volume));
            voice.sent_volume = volume;
        }
        if (voice.pitch - voice.sent_pitch).abs() > 1e-3 {
            sink.set_speed(voice.pitch);
            voice.sent_pitch = voice.pitch;
        }
        if sink.is_paused() {
            sink.play();
        }
    }

    // What's asked for this frame.
    if audible && settings.volume > 0.0 {
        let share = air_share(hero.flow.air.pressure_pa);
        for cue in hero.sound.asked.into_iter().flatten() {
            start_shot(&mut commands, voices, cue, share, settings.volume * tune::SOUND_GAIN, control.dev);
        }
    }
    hero.sound.clear();

    // The one-shots playing.
    let (master, volume) = (voices.master, settings.volume * tune::SOUND_GAIN);
    let air_now = air_share(hero.flow.air.pressure_pa);
    for slot in &mut voices.shots {
        let Some(shot) = slot else { continue };
        shot.air += (air_now - shot.air).clamp(-step, step);
        if master > 0.0 {
            shot.played += dt;
        }
        if shot.sfx == Sfx::ChargeStart && !charging {
            shot.cut = true;
        }
        if shot.cut {
            shot.fade = (shot.fade - step).max(0.0);
        }
        let done = match sinks.get_mut(shot.entity) {
            Ok(mut sink) => {
                let want = shot.gain * shot.air * shot.fade * master * volume;
                if (want - shot.sent_volume).abs() > tune::SOUND_SILENT {
                    sink.set_volume(Volume::Linear(want));
                    shot.sent_volume = want;
                }
                // Paused only by the focus: it goes on from there.
                if master <= 0.0 && !sink.is_paused() {
                    sink.pause();
                } else if master > 0.0 && sink.is_paused() {
                    sink.play();
                }
                sink.empty() || shot.fade <= 0.0
            }
            // No audio device: nothing says it's over.
            Err(_) => shot.played > tune::SHOT_MAX_S || shot.fade <= 0.0,
        };
        if done {
            commands.entity(shot.entity).despawn();
            *slot = None;
        }
    }

    let shots = voices.shots_alive();
    if let Some(out) = voices.trace.as_mut() {
        let f = &reading.flow;
        let _ = write!(out, "{:.4},{:?},{:.4},{:.5},{:.4},{:.1},{:.1},{:.2},{},{},{:.4}", voices.clock, hero.state, f.speed, f.q, f.mach, f.skin_k, reading.heat_k, f.air.pressure_pa, reading.flying as u8, charging as u8, voices.master);
        for (i, want) in levels.iter().enumerate() {
            let voice = voices.loops.iter().find(|v| v.sfx == LOOPS[i]);
            let (level, pitch) = voice.map_or((0.0, 0.0), |v| (v.level, v.pitch));
            let _ = write!(out, ",{:.4},{:.4},{:.4}", want.0, level, pitch);
        }
        let _ = writeln!(
            out,
            ",{},{shots},{},{},{},{},{:.4},{:.4}",
            voices.loops.len(),
            started.map_or(0, |t| t.elapsed().as_micros()),
            reading.fast as u8,
            reading.supersonic as u8,
            reading.stacks,
            reading.share,
            reading.charge
        );
    }

    // Off, and faded: nothing's left, and this doesn't run again.
    if !on && voices.master <= 0.0 {
        for voice in voices.loops.drain(..) {
            commands.entity(voice.entity).despawn();
        }
        for slot in &mut voices.shots {
            if let Some(shot) = slot.take() {
                commands.entity(shot.entity).despawn();
            }
        }
        voices.charging = false;
    }
}

/// Starts a one-shot, in a free slot or the one that's played longest: at most `SHOTS_MAX`, and the newest sound counts.
fn start_shot(commands: &mut Commands, voices: &mut Voices, cue: Cue, share: f32, volume: f32, dev: bool) {
    let i = cue.sfx as usize;
    // (A louder one still counts: W then Shift within the gap is a quiet take-off, then the boost's own.)
    let (when, how_loud) = voices.last_fired[i];
    if voices.clock - when < cue.sfx.gap_s() && cue.gain <= how_loud {
        return;
    }
    let Some(handle) = voices.handles[i].clone() else { return };
    voices.last_fired[i] = (voices.clock, cue.gain);
    voices.fired[i] += 1;
    if dev {
        info!("hero sound: {} fired, gain {:.3}, pitch {:.2}", cue.sfx.file(), cue.gain * share, cue.pitch);
    }
    // No air, no sound (D12): counted as fired, but no voice, so it takes no one's slot.
    if cue.gain * share <= tune::SOUND_SILENT {
        return;
    }
    let slot = voices.shots.iter().position(Option::is_none).unwrap_or_else(|| {
        let oldest = voices.shots.iter().enumerate().max_by(|a, b| {
            let played = |s: &Option<Shot>| s.as_ref().map_or(0.0, |s| s.played);
            played(a.1).total_cmp(&played(b.1))
        });
        oldest.map_or(0, |(i, _)| i)
    });
    if let Some(old) = voices.shots[slot].take() {
        commands.entity(old.entity).despawn();
    }
    let sent = cue.gain * share * voices.master * volume;
    let entity = commands.spawn((AudioPlayer(handle), PlaybackSettings::ONCE.with_volume(Volume::Linear(sent)).with_speed(cue.pitch), HeroVoice)).id();
    voices.shots[slot] = Some(Shot { entity, sfx: cue.sfx, gain: cue.gain, air: share, fade: 1.0, cut: false, played: 0.0, sent_volume: sent });
}

// ---- Dev commands ---------------------------------------------------------------------------------------------------

/// `hero sound` logs every loop's volume and pitch, as the sinks really have them, and what's going on with the air and the
/// voices. `hero sound trace FILE|off`: one line a frame. `hero sound fire NAME`: a one-shot by its file's name.
/// `hero sound cue WHAT [N]`: a trigger's function by name, for the ones no step fires yet.
pub fn dev_command(world: &mut World, arg: &str) {
    let (verb, rest) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
    match verb.to_ascii_lowercase().as_str() {
        "" => {
            let _ = world.run_system_cached(log_voices);
        }
        "trace" => {
            let mut voices = world.resource_mut::<Voices>();
            voices.trace = if rest.eq_ignore_ascii_case("off") {
                None
            } else {
                match std::fs::File::create(rest) {
                    Ok(f) => {
                        let mut w = std::io::BufWriter::new(f);
                        let mut head = "clock,state,speed,q,mach,skin_k,heat_k,pressure,flying,charging,master".to_string();
                        for l in LOOPS {
                            let n = l.file();
                            head += &format!(",{n}_want,{n}_level,{n}_pitch");
                        }
                        let _ = writeln!(w, "{head},loops,shots,us,fast,supersonic,stacks,share,charge");
                        Some(w)
                    }
                    Err(e) => {
                        warn!("hero sound trace: {rest}: {e}");
                        None
                    }
                }
            };
        }
        "fire" => match FILES.iter().position(|f| f.eq_ignore_ascii_case(rest)) {
            Some(i) => world.resource_mut::<Hero>().sound.ask(ALL[i], 1.0, 1.0),
            None => warn!("hero sound fire: no sound '{rest}'"),
        },
        "cue" => {
            let (what, n) = rest.split_once(' ').map_or((rest, 0.0), |(a, b)| (a, b.trim().parse().unwrap_or(0.0)));
            let mut hero = world.resource_mut::<Hero>();
            let cues = &mut hero.sound;
            match what.to_ascii_lowercase().as_str() {
                "boom" => cues.boom(n),
                "supersonic_end" => cues.supersonic_end(),
                "punch_swing" => cues.punch_swing(),
                "punch" => cues.punch(),
                "punch_hits_building" => cues.punch_hits_building(),
                "ground_punch" => cues.ground_punch(),
                "building_breaks" => cues.building_breaks(),
                "thunderclap" => cues.thunderclap(n as f32),
                "shockwave" => cues.shockwave(),
                "splash" => cues.splash(n),
                "hard_landing" => cues.hard_landing(),
                _ => warn!("hero sound cue: no trigger '{what}'"),
            }
        }
        _ => warn!("hero sound: unknown command '{arg}'"),
    }
}

fn log_voices(
    mut voices: ResMut<Voices>,
    hero: Res<Hero>,
    control: Res<ModControl>,
    settings: Res<SoundSettings>,
    sinks: Query<&AudioSink>,
    mine: Query<(), With<HeroVoice>>,
    all: Query<(), With<AudioPlayer>>,
) {
    let mut loops = String::new();
    for voice in &voices.loops {
        let sink = sinks.get(voice.entity).map_or("no sink".to_string(), |s| format!("sink {:.4} speed {:.3} paused {} at {:.3} s", s.volume().to_linear(), s.speed(), s.is_paused(), s.position().as_secs_f32()));
        loops += &format!("{} level {:.3} pitch {:.3} {sink}; ", voice.sfx.file(), voice.level, voice.pitch);
    }
    let shots: Vec<&str> = voices.shots.iter().flatten().map(|s| s.sfx.file()).collect();
    let fired: Vec<String> = voices.fired.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| format!("{} x{n}", FILES[i])).collect();
    let (f, heat_k) = (&hero.flow, hero.heat_k);
    info!(
        "hero sound: on {} state {:?} master {:.3} setting {:.2}; air speed {:.2} m/s q {:.3} Pa mach {:.3} skin {:.0} K heat {:.0} K, p {:.2} Pa rho {:.5} T {:.1} K a {:.1} m/s, shot share {:.3}; {loops}shots {shots:?}; entities {} of the mod, {} audio in all; fired {fired:?}",
        control.hero,
        hero.state,
        voices.master,
        settings.volume,
        f.speed,
        f.q,
        f.mach,
        f.skin_k,
        heat_k,
        f.air.pressure_pa,
        f.air.density,
        f.air.temperature_k,
        f.air.sound_speed,
        air_share(f.air.pressure_pa),
        mine.iter().count(),
        all.iter().count(),
    );
    if let Some(t) = voices.trace.as_mut() {
        let _ = t.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::air::AirSample;
    use super::super::impact::Action;
    use super::*;

    fn flow(q: f64, mach: f64, pressure_pa: f64) -> Flow {
        Flow { q, mach, air: AirSample { pressure_pa, ..default() }, ..default() }
    }

    #[test]
    fn the_loops_follow_f10s_table() {
        let at = |q, mach, heat_k, flying, charging| loop_levels(&Reading { flow: flow(q, mach, 1e5), heat_k, flying, charging, charge: 1.0, ..default() });
        // Hovering in still air: the hum alone.
        let l = at(0.0, 0.0, 0.0, true, false);
        assert_eq!(l[0].0, 0.0);
        assert_eq!(l[1].0, 0.0);
        assert_eq!(l[2], (0.12, 0.6));
        assert_eq!(l[3].0, 0.0);
        // The fast loop from log q 1 to 4.5: it's the air going past at every speed.
        assert_eq!(at(10.0, 0.0, 0.0, true, false)[0].0, 0.0);
        assert!((at(10f64.powf(2.75), 0.0, 0.0, true, false)[0].0 - 0.5).abs() < 1e-5);
        assert_eq!(at(10f64.powf(4.5), 0.0, 0.0, true, false)[0].0, 1.0);
        // The faster loop by the temperature.
        assert_eq!(at(0.0, 0.0, 2500.0, true, false)[1].0, 0.0);
        assert!((at(0.0, 0.0, 4250.0, true, false)[1].0 - 0.5).abs() < 1e-5);
        assert_eq!(at(0.0, 0.0, 6000.0, true, false)[1].0, 1.0);
        // The hum fades from log q 0 to 2, and is only for flying.
        assert_eq!(at(1.0, 0.0, 0.0, true, false)[2].0, 0.12);
        assert!((at(10.0, 0.0, 0.0, true, false)[2].0 - 0.06).abs() < 1e-5);
        assert_eq!(at(100.0, 0.0, 0.0, true, false)[2].0, 0.0);
        // In air it's only for flying: falling or standing in it, no hum at any q.
        assert_eq!(at(0.0, 0.0, 0.0, false, false)[2].0, 0.0);
        // The charge while the button's held.
        assert!((at(0.0, 0.0, 0.0, false, true)[3].0 - 0.95).abs() < 1e-6);
    }

    /// D12 (the user's 2026-11-12): fully silent where there's no air, whatever he's doing and however fast.
    #[test]
    fn out_of_the_air_every_loop_is_silent() {
        let all = |flow: Flow, flying| {
            let r = Reading { flow, flying, heat_k: 9000.0, charging: true, charge: 1.0, fast: true, supersonic: true, stacks: 5, share: 1.0 };
            loop_levels(&r).map(|l| l.0)
        };
        for speed in [0.0, 25.0, 7_700.0, 3e11] {
            for flying in [true, false] {
                assert_eq!(all(Flow::vacuum(speed), flying), [0.0; 4], "at {speed} m/s, flying {flying}");
            }
        }
        // High q in air too thin to carry sound (a fast skim of the upper air): still nothing.
        assert_eq!(all(flow(1e5, 30.0, 1.0), true), [0.0; 4]);
        // In between it fades by the pressure alone, with no step on the way down.
        let hum = |p| loop_levels(&Reading { flow: flow(0.0, 0.0, p), flying: true, ..default() })[2].0;
        assert_eq!(hum(0.5), 0.0);
        assert!((hum(10.0) - 0.06).abs() < 1e-5, "{}", hum(10.0));
        assert_eq!(hum(100.0), 0.12);
        // Mars's ground and Earth's: as before.
        assert_eq!(hum(650.0), 0.12);
        assert_eq!(hum(101_325.0), 0.12);
        // Not flying, never a hum, in air or out of it.
        let still = |p| loop_levels(&Reading { flow: flow(0.0, 0.0, p), ..default() })[2].0;
        assert_eq!((still(0.0), still(10.0), still(101_325.0)), (0.0, 0.0, 0.0));
    }

    /// The same through `play_sounds`: the state comes from `Hero`, and the voice goes where it's told.
    #[test]
    fn the_hum_goes_by_the_air_in_every_state() {
        let mut a = app();
        turn(&mut a, true);
        let level = |a: &App| a.world().resource::<Voices>().loops.iter().find(|l| l.sfx == Sfx::HumLoop).unwrap().level;
        for state in [State::Hover, State::Flight, State::Falling, State::Jump, State::Grounded, State::Land, State::HardLanding] {
            a.world_mut().resource_mut::<Hero>().state = state;
            a.world_mut().resource_mut::<Hero>().flow = Flow::vacuum(1.0);
            for _ in 0..30 {
                frame(&mut a, 16);
            }
            assert_eq!(level(&a), 0.0, "{state:?} in vacuum");
            a.world_mut().resource_mut::<Hero>().flow.air.pressure_pa = 101_325.0;
            for _ in 0..30 {
                frame(&mut a, 16);
            }
            let want = if matches!(state, State::Hover | State::Flight) { 0.12 } else { 0.0 };
            assert_eq!(level(&a), want, "{state:?} in air");
        }
    }

    #[test]
    fn the_fast_loop_climbs_with_mach() {
        let at = |mach| loop_levels(&Reading { flow: flow(1e5, mach, 1e5), ..default() })[0].1;
        assert_eq!(at(0.0), tune::FAST_PITCH.0);
        // Low at 20 m/s, where it stands in for the air going past.
        assert!(at(0.06) < 0.65);
        assert!(at(1.0) > at(0.6));
        assert!(at(9.0) > at(1.0));
        assert_eq!(at(90.0), tune::FAST_PITCH.1);
    }

    #[test]
    fn in_vacuum_a_one_shot_is_silent() {
        assert_eq!(air_share(0.0), 0.0);
        assert_eq!(air_share(1.0), 0.0);
        assert_eq!(air_share(100.0), 1.0);
        assert_eq!(air_share(101_325.0), 1.0);
        // Mars's ground, about 650 Pa.
        assert_eq!(air_share(650.0), 1.0);
        let some = air_share(10.0);
        assert!((some - 0.5).abs() < 1e-6, "{some}");
    }

    /// D12: a one-shot that began in air goes silent while it plays if he leaves the air (a boom, then up into space).
    #[test]
    fn a_shot_begun_in_air_goes_silent_in_vacuum() {
        let mut a = app();
        turn(&mut a, true);
        frame(&mut a, 16);
        a.world_mut().resource_mut::<Hero>().sound.launch();
        frame(&mut a, 16);
        let air = |a: &App| a.world().resource::<Voices>().shots.iter().flatten().find(|s| s.sfx == Sfx::Launch).map(|s| s.air);
        assert_eq!(air(&a), Some(1.0));
        a.world_mut().resource_mut::<Hero>().flow = Flow::vacuum(3000.0);
        for _ in 0..20 {
            frame(&mut a, 16);
        }
        assert_eq!(air(&a), Some(0.0), "still playing, and silent");
    }

    #[test]
    fn a_cue_asked_twice_in_a_frame_is_one() {
        let mut c = Cues::default();
        c.take_off_ground();
        c.take_off_ground();
        c.launch();
        assert_eq!(c.asked.iter().flatten().count(), 2);
        // The same file asked again louder: one voice, the louder.
        c.jump(true);
        assert_eq!(c.asked.iter().flatten().count(), 2);
        assert_eq!(c.asked[0].unwrap().gain, 1.0);
        // Full: no panic.
        for _ in 0..40 {
            c.punch();
            c.punch_swing();
            c.thunderclap(0.0);
            c.thunderclap(0.5);
            c.thunderclap(1.0);
            c.shockwave();
            c.ground_punch();
        }
        assert_eq!(c.asked.iter().flatten().count(), tune::SHOTS_MAX);
    }

    #[test]
    fn swings_and_punches_take_turns_of_their_own() {
        let mut c = Cues::default();
        let mut seen = Vec::new();
        // Swing, punch, swing, punch, as step 8 will fire them.
        for _ in 0..6 {
            c.punch_swing();
            c.punch();
            seen.extend(c.asked.iter().flatten().map(|a| a.sfx));
            c.clear();
        }
        for sfx in [Sfx::MeleeStart1, Sfx::MeleeStart3, Sfx::Punch1, Sfx::Punch2, Sfx::Punch3] {
            assert!(seen.contains(&sfx), "{sfx:?} never comes");
        }
    }

    #[test]
    fn the_triggers_choose_their_sounds() {
        let one = |f: fn(&mut Cues)| {
            let mut c = Cues::default();
            f(&mut c);
            c.asked[0].unwrap()
        };
        let off = one(|c| c.take_off_ground());
        assert_eq!((off.sfx, off.gain), (Sfx::JumpExp, 0.8));
        assert_eq!(one(|c| c.stop_flying()).sfx, Sfx::FlightEnd);
        assert_eq!(one(|c| c.jump(false)).sfx, Sfx::JumpSmall);
        assert_eq!(one(|c| c.jump(true)).sfx, Sfx::JumpExp);
        assert_eq!(one(|c| c.thunderclap(0.1)).sfx, Sfx::Clap1);
        assert_eq!(one(|c| c.thunderclap(0.5)).sfx, Sfx::Clap2);
        assert_eq!(one(|c| c.thunderclap(1.0)).sfx, Sfx::Clap3);
        let hard = one(|c| c.hard_landing());
        assert_eq!((hard.sfx, hard.pitch, hard.gain), (Sfx::ExplodeGround, 1.0, 1.0));
        let sea = one(|c| c.splash(50.0));
        assert_eq!((sea.sfx, sea.pitch, sea.gain), (Sfx::ExplodeGround, 0.6, 1.0));
        assert!(one(|c| c.splash(1.8)).gain < 0.2);
        // The boom's loudness goes by log q: sea level over Mars.
        let boom = |q| {
            let mut c = Cues::default();
            c.boom(q);
            let (a, b) = (c.asked[0].unwrap(), c.asked[1].unwrap());
            assert_eq!((a.sfx, b.sfx), (Sfx::SupersonicStart, Sfx::SonicBoom));
            a.gain
        };
        assert_eq!(boom(1e5), 1.0);
        assert!(boom(60_000.0) > 0.99);
        assert!(boom(600.0) < 0.9 && boom(600.0) > 0.6);
    }

    /// The frame's world, with no `AudioPlugin`: as with no audio device, a voice gets no `AudioSink`.
    fn app() -> App {
        let mut app = App::new();
        app.insert_resource(Assets::<AudioSource>::default())
            .insert_resource(SoundSettings { volume: 0.3 })
            .init_resource::<ModControl>()
            .init_resource::<Hero>()
            .init_resource::<Voices>()
            .init_resource::<Time<Real>>()
            .add_systems(Update, play_sounds.run_if(sound_running));
        // In air, where everything sounds; the tests about vacuum say so (D12: silent there).
        app.world_mut().resource_mut::<Hero>().flow.air.pressure_pa = 101_325.0;
        app
    }

    fn frame(app: &mut App, ms: u64) {
        app.world_mut().resource_mut::<Time<Real>>().advance_by(Duration::from_millis(ms));
        app.update();
    }

    fn turn(app: &mut App, on: bool) {
        app.world_mut().resource_mut::<ModControl>().hero = on;
        app.world_mut().resource_mut::<Hero>().placed = on;
    }

    fn entities(app: &mut App) -> usize {
        app.world_mut().query_filtered::<Entity, With<HeroVoice>>().iter(app.world()).count()
    }

    #[test]
    fn nothing_exists_before_the_mod_is_on_and_nothing_stays_after() {
        let mut a = app();
        frame(&mut a, 16);
        assert_eq!(entities(&mut a), 0);
        for round in 0..50 {
            turn(&mut a, true);
            frame(&mut a, 16);
            frame(&mut a, 16);
            assert_eq!(entities(&mut a), 4, "round {round}: only the four loops, once");
            a.world_mut().resource_mut::<Hero>().sound.take_off_ground();
            frame(&mut a, 16);
            assert_eq!(entities(&mut a), 5);
            turn(&mut a, false);
            for _ in 0..60 {
                frame(&mut a, 16);
            }
            assert_eq!(entities(&mut a), 0, "round {round}: gone after the fade");
            assert!(!a.world().resource::<Voices>().alive());
        }
    }

    #[test]
    fn the_fade_takes_its_time_and_coming_back_reuses_the_voices() {
        let mut a = app();
        turn(&mut a, true);
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        assert_eq!(a.world().resource::<Voices>().master, 1.0);
        turn(&mut a, false);
        frame(&mut a, 50);
        let m = a.world().resource::<Voices>().master;
        assert!((m - 0.75).abs() < 1e-4, "{m}");
        assert_eq!(entities(&mut a), 4);
        // Back on before the fade ended: the same voices.
        let first = a.world().resource::<Voices>().loops[0].entity;
        turn(&mut a, true);
        frame(&mut a, 16);
        assert_eq!(entities(&mut a), 4);
        assert_eq!(a.world().resource::<Voices>().loops[0].entity, first);
    }

    #[test]
    fn a_loop_starts_at_its_own_pitch() {
        let mut a = app();
        turn(&mut a, true);
        frame(&mut a, 16);
        let v = a.world().resource::<Voices>();
        let pitch = |sfx: Sfx| v.loops.iter().find(|l| l.sfx == sfx).unwrap().pitch;
        assert_eq!(pitch(Sfx::HumLoop), tune::HUM_PITCH);
        assert_eq!(pitch(Sfx::FastLoop), tune::FAST_PITCH.0);
        assert_eq!(pitch(Sfx::FasterLoop), 1.0);
        assert_eq!(pitch(Sfx::ChargeLoop), 1.0);
    }

    #[test]
    fn one_shots_fire_once_and_never_pass_eight() {
        let mut a = app();
        turn(&mut a, true);
        frame(&mut a, 16);
        frame(&mut a, 16);
        // Asked only the frame it happens: once.
        a.world_mut().resource_mut::<Hero>().sound.jump(false);
        frame(&mut a, 16);
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        assert_eq!(a.world().resource::<Voices>().fired[Sfx::JumpSmall as usize], 1);
        // Asked every frame by a bug or a stuck key: the gap holds it down, whatever the frame rate.
        for _ in 0..10 {
            a.world_mut().resource_mut::<Hero>().sound.take_off_ground();
            frame(&mut a, 1);
        }
        assert_eq!(a.world().resource::<Voices>().fired[Sfx::JumpExp as usize], 1);
        // Another sound right after: fired once too.
        for _ in 0..3 {
            a.world_mut().resource_mut::<Hero>().sound.boost_start();
            frame(&mut a, 16);
        }
        assert_eq!(a.world().resource::<Voices>().fired[Sfx::FastStart as usize], 1);
        // Many at once: at most 8, the newest in.
        for sfx in ALL {
            a.world_mut().resource_mut::<Hero>().sound.ask(*sfx, 1.0, 1.0);
            frame(&mut a, 16);
            assert!(a.world().resource::<Voices>().shots_alive() <= tune::SHOTS_MAX);
            assert!(entities(&mut a) <= tune::SHOTS_MAX + LOOPS.len());
        }
        assert_eq!(a.world().resource::<Voices>().shots_alive(), tune::SHOTS_MAX);
        // With no sink to say they're over, their time ends them.
        for _ in 0..200 {
            frame(&mut a, 50);
        }
        assert_eq!(a.world().resource::<Voices>().shots_alive(), 0);
        assert_eq!(entities(&mut a), 4);
    }

    #[test]
    fn the_charge_starts_its_sound_and_letting_go_cuts_it() {
        let mut a = app();
        turn(&mut a, true);
        for _ in 0..20 {
            frame(&mut a, 16);
        }
        let charge = Action::Charge { clip: 0, t: 0.0, hold: 0.6, clap: 1.23, end: 2.0 };
        a.world_mut().resource_mut::<Hero>().flow = Flow::vacuum(0.0);
        a.world_mut().resource_mut::<Hero>().action = charge;
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        let v = a.world().resource::<Voices>();
        assert_eq!(v.fired[Sfx::ChargeStart as usize], 1, "held: once");
        // (In vacuum here: D12, silent, and the one-shot takes no voice.)
        assert_eq!(v.loops.iter().find(|l| l.sfx == Sfx::ChargeLoop).unwrap().level, 0.0);
        assert_eq!(v.shots_alive(), 0);
        // Into the air: the loop comes up; a new charge's start plays.
        a.world_mut().resource_mut::<Hero>().flow.air.pressure_pa = 101_325.0;
        a.world_mut().resource_mut::<Hero>().action = Action::None;
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        a.world_mut().resource_mut::<Hero>().action = charge;
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        let v = a.world().resource::<Voices>();
        assert!((v.loops.iter().find(|l| l.sfx == Sfx::ChargeLoop).unwrap().level - 0.95).abs() < 1e-6);
        assert_eq!(v.fired[Sfx::ChargeStart as usize], 2);
        assert_eq!(v.shots_alive(), 1);
        a.world_mut().resource_mut::<Hero>().action = Action::None;
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        let v = a.world().resource::<Voices>();
        assert_eq!(v.loops.iter().find(|l| l.sfx == Sfx::ChargeLoop).unwrap().level, 0.0);
        assert_eq!(v.shots_alive(), 0, "flightCharge_start goes with the charge");
    }

    #[test]
    fn losing_the_focus_fades_them_and_asks_nothing() {
        let mut a = app();
        turn(&mut a, true);
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        a.world_mut().resource_mut::<ModControl>().unfocused = true;
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        assert_eq!(a.world().resource::<Voices>().master, 0.0);
        assert_eq!(entities(&mut a), 4, "paused, not gone");
        a.world_mut().resource_mut::<Hero>().sound.take_off_ground();
        frame(&mut a, 16);
        assert_eq!(a.world().resource::<Voices>().fired[Sfx::FastStart as usize], 0);
        a.world_mut().resource_mut::<ModControl>().unfocused = false;
        for _ in 0..30 {
            frame(&mut a, 16);
        }
        assert_eq!(a.world().resource::<Voices>().master, 1.0);
    }

    #[test]
    fn with_the_setting_at_zero_nothing_fires() {
        let mut a = app();
        a.world_mut().resource_mut::<SoundSettings>().volume = 0.0;
        turn(&mut a, true);
        frame(&mut a, 16);
        frame(&mut a, 16);
        a.world_mut().resource_mut::<Hero>().sound.jump(false);
        frame(&mut a, 16);
        assert_eq!(a.world().resource::<Voices>().fired[Sfx::JumpSmall as usize], 0);
        assert_eq!(a.world().resource::<Voices>().shots_alive(), 0);
        assert_eq!(a.world().resource::<Hero>().sound, Cues::default());
    }

    #[test]
    fn real_seconds_only_a_hitch_counts_as_a_tenth() {
        let mut a = app();
        turn(&mut a, true);
        frame(&mut a, 2000);
        assert!((a.world().resource::<Voices>().master - 0.5).abs() < 1e-4);
    }

    #[test]
    fn every_sound_is_there_and_decodes() {
        // Every file of the mod's folder is in the list, and the other way round.
        let dir = crate::mods::mod_file("viltrumite", "sfx/fastFlight_loop.ogg").unwrap();
        let mut on_disk: Vec<String> = std::fs::read_dir(dir.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().trim_end_matches(".ogg").to_string()).collect();
        on_disk.sort();
        let mut listed: Vec<String> = FILES.iter().map(|f| f.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
        assert_eq!(ALL.len(), FILES.len());
        for (i, sfx) in ALL.iter().enumerate() {
            assert_eq!(*sfx as usize, i);
            let path = crate::mods::mod_file("viltrumite", &format!("sfx/{}.ogg", sfx.file())).unwrap();
            let (_, facts) = crate::mods::check_sound(std::fs::read(path).unwrap()).unwrap();
            println!("{:28} {} channels {} Hz {:6.2} s peak {:.2}", sfx.file(), facts.channels, facts.rate, facts.seconds, facts.peak);
            assert!(facts.seconds > 0.1 && facts.peak > 0.01, "{sfx:?} {facts:?}");
            assert!(facts.seconds / tune::SPLASH_PITCH < tune::SHOT_MAX_S || LOOPS.contains(sfx), "{sfx:?} outlives SHOT_MAX_S");
        }
    }

    /// The mod's own text: its `Need` lines.
    fn mod_cfg() -> String {
        std::fs::read_to_string(crate::mods::mod_file("viltrumite", "mod.cfg").unwrap()).unwrap()
    }

    /// Every sound played is checked on first enable (a damaged one greys the row): only what `Need` lists is.
    #[test]
    fn every_sound_is_a_need_of_the_mod() {
        let cfg = mod_cfg();
        for f in FILES {
            assert!(cfg.contains(&format!("Need \"sfx/{f}.ogg\"")), "{f} isn't checked on first enable");
        }
    }

    /// `wind_loop`, `superWind_start`, `wind_end`, `wind_attack_*` are the source mod's super breath (the user, 2026-11-07):
    /// a non-goal, and they don't sound like air going past. No file, no entry, no `Need`.
    #[test]
    fn no_breath_sounds() {
        let breath = |name: &str| name.to_ascii_lowercase().contains("wind");
        let dir = crate::mods::mod_file("viltrumite", "sfx/fastFlight_loop.ogg").unwrap();
        for e in std::fs::read_dir(dir.parent().unwrap()).unwrap() {
            let name = e.unwrap().file_name().to_string_lossy().to_string();
            assert!(!breath(&name), "{name} is in sfx/");
        }
        for f in FILES {
            assert!(!breath(f), "{f} is in the sound list");
        }
        for line in mod_cfg().lines().filter(|l| l.contains("sfx/")) {
            assert!(!breath(line), "{line} is in mod.cfg");
        }
    }

    /// `cargo test -p se_engine --release print_sound_levels -- --ignored --nocapture`: how loud each file really is, over its
    /// length (RMS of eight equal parts), since a test can't listen.
    #[test]
    #[ignore]
    fn print_sound_levels() {
        use bevy::audio::Decodable;
        for name in FILES {
            let path = crate::mods::mod_file("viltrumite", &format!("sfx/{name}.ogg")).unwrap();
            let (source, facts) = crate::mods::check_sound(std::fs::read(path).unwrap()).unwrap();
            let samples: Vec<f32> = source.decoder().collect();
            let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt();
            let parts: Vec<String> = samples.chunks(samples.len().div_ceil(8)).map(|c| format!("{:.3}", rms(c))).collect();
            println!("{name:28} {:5.2} s peak {:.2} rms {:.3} | {}", facts.seconds, facts.peak, rms(&samples), parts.join(" "));
        }
    }
}
