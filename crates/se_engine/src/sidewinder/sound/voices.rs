//! The mixer's voices as sound. The ship has one output on the audio device: [`ShipOutput`], an `AudioPlayer` entity
//! spawned when the first voice starts and despawned when the sound stops, whose [`OutputSource`] mixes every built voice
//! on the audio thread at the output rate (two channels, each voice through its matrix of gains: Wwise's pan rules, T5d-7).
//! The game thread hands it voices and writes each voice's controls in place ([`VoiceCtl`]: level,
//! speed, filter values, EQ bands, its emitter's place, a seek, a stop); nothing else crosses between the threads.
//!
//! A voice's recording is played by [`VoiceSource`]: cut at the media table's length and looped by starting the decoder
//! again, so a loop joins sample for sample; a recording with a loop region plays its intro once and then loops the region
//! alone (a region up to `LOOP_CACHE_S` is decoded once when the voice is built and replayed from memory, so that join is
//! exact too and the audio thread never grows a buffer); start offsets, loop counts and seeks as the mixer says.
//!
//! **Speed:** the source resamples the recording itself (linear between its frames, the speed ramped in 8 steps of 128
//! frames). (T5d: rodio 0.22's `SampleRateConverter`, which the player used before, works out `from * position` in `u32`
//! on the rates' reduced ratio, so a voice at rate x speed of 89,478 Hz or more whose rate shares no factor with the
//! output's overflows it about 0.93 s in, and its catch-up loop then runs ~2^32 times on the audio thread: the drop's 2 s
//! loop `15d19ba9` at a pitch near 2.0 stopped the game's whole output for 138-146 s in three runs out of three.
//! `rodios_converter_hangs_on_a_fast_coprime_rate_and_ours_does_not` shows both.)
//!
//! **Filters and effects:** each voice runs Wwise's low-pass and high-pass sections as Elite's own copy of the sound engine
//! does (`se_core::elite_sound::filter`: its cutoff of a 0-100 value, the Butterworth sections, the value ramped from the
//! last one to the new in 8 steps of 128 frames, bypassed at 0.1 and under, its memory cleared when it engages), at the
//! output rate after the resampling; then the effects on its way to the master, node's and buses' in signal order
//! ([`VoiceUpdate::fx`]: a linear effect on a bus gives the bus's mix what it gives each voice): a Parametric EQ's bands,
//! each section designed as the exe designs it (`filter::eq_coefs`) when its values change, its memory kept (the plug-in
//! keeps it), a band that is off passing the signal; a Guitar Distortion (`filter::Distortion`, its parameters taken once
//! a buffer of 1024 frames and ramped over it). The level comes last, as Wwise applies a voice's volume after its effects.
//! A sample at the chain's end beyond `filter::CHAIN_BOUND`, or not finite, is the guard's (below).
//!
//! **Blocks (T5d-4):** a voice is made a block of `BLOCK` frames (one ramp step) at a time and the output mixes a block at
//! a time: the controls are taken at the block's start (where the ramp's step took them before), the recording is
//! resampled into the block, then each filter and effect runs over the whole block, its state held across it
//! (`Section::run_block`, `Distortion::block`), the level last. Each sample gets the arithmetic it got frame by frame, in
//! the same order (`blocks_sound_as_frames_did` renders a supercruise entry's voices both ways); a band that is off and a
//! filter at its bypass value are skipped, as before. A Guitar Distortion is never skipped, even at a wet share of 0: its
//! DC blocker and level act on the dry signal and its wet path's bands keep their memory. A mono voice stays one channel
//! until it is mixed.
//!
//! **Virtual voices** are not built: a voice is built when the mixer reports it heard (`VoiceUpdate::start`, from where its
//! virtual behaviour says) and dropped when it goes virtual; a virtual voice costs the audio thread nothing.
//!
//! **Starts:** the voices a frame starts are built (decoder set-up, the skip to the start offset, the loop region's
//! decode: most of a start's cost) on worker threads, one batch a frame, without holding up the game thread
//! ([`Voices::start_queued`]); a batch is handed to the output whole in the frame its last voice is ready
//! ([`Voices::hand_over`]), and the output starts all of it at one output frame (a block's first), so the layers of one
//! event stay aligned sample for sample (`a_batch_starts_at_one_sample`). The delay is logged with `sw sound log on`.
//! Batches go in the order they were asked for. The voices a mixer step stops end at the frame its batch of starts begins
//! (with no starts in the step, at once), so a set handing over to another leaves no hole (`a_stop_meets_its_steps_starts`).
//!
//! **The guard:** a sample of the last stage that is not finite, or beyond the bound, becomes 0, its voice's filters start
//! from rest, and its block is counted (one log line a voice, `Voices::tell_guard`); an update whose level, speed or filter value is not finite is
//! dropped (the last value stays) and logged once a voice; a build that panics loses that voice only. `sw sound log on`
//! logs the audio thread's heartbeat once a second (`Voices::meter`: frames made, the busiest voices, frames skipped).
//!
//! **Where a voice sits (T5d-5):** a voice the mixer hands a placement (`VoiceUpdate::place`: its emitter placed by the game,
//! its sound 3D on the emitter's position) gets its emitter's place from the listener each frame (`VoiceCtl::place`, a
//! sequence lock written from the update, as its other controls are written); at each block's start it works out the
//! distance's gain, low-pass and high-pass shares, spread and pan (`se_core::elite_sound::place`), adds the filter shares
//! to the mixer's values before their ramp, and the output mixes it through the pan matrix (the distance's gain in it),
//! each gain ramped frame by frame from the last block's over the block (`mix_through`).
//!
//! **How every voice reaches the output (T5d-7):** one matrix of gains a voice (`[input channel][output channel]`), by
//! Wwise's own pan rules as Elite's exe holds them (`se_core::elite_sound::place`: the 2D or 3D panner into its route's
//! stereo or 7.1.4 layout, a 7.1.4 route down-mixed by Elite's table, all folded into the one matrix). A voice that is not
//! placed plays through the matrix the mixer hands it (`VoiceUpdate::matrix`, a voice at the listener) for its life; a
//! placed one through the matrix its place gives. The recording's own gain (`akd_gain_2`, `VoiceUpdate::media_gain`)
//! multiplies its samples as they are resampled, before the filters and effects (a node's Guitar Distortion is not
//! linear).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use bevy::audio::{AudioPlayer, AudioSource, ChannelCount, Decodable, PlaybackSettings, SampleRate, Source, Volume};
use bevy::prelude::*;
use se_core::elite_sound::data::EqFilter;
use se_core::elite_sound::filter::{self, Coefs, Distortion, Section};
use se_core::elite_sound::{DistortionNow, EqNow, FxNow, PlaceNow, Placement, VoiceUpdate, OUTPUT_RATE};

use super::load::Clip;

/// The most a voice's level is set to (a guard: the mixer's gain is at most a few times unity).
const MOST_VOLUME: f32 = 4.0;
/// No seek asked.
const NO_SEEK: u64 = u64::MAX;
/// A filter value's ramp (Elite's sound engine: `DAT_145ee0a5c` steps of `DAT_145ee0a54` frames); the level ramps over
/// one step.
const RAMP_STEPS: u8 = 8;
const RAMP_FRAMES: u32 = 128;
/// The frames a voice makes at a time and the output mixes at a time: one ramp step, so every control value holds across
/// a block (the level's own ramp goes frame by frame inside it).
const BLOCK: usize = RAMP_FRAMES as usize;
/// Starts in one frame up to this many are built on one worker thread; more are shared among several.
const BUILD_ALONE: usize = 4;
/// A loop region up to this long (s) is decoded when the voice is built and replayed from memory.
const LOOP_CACHE_S: f64 = 3.0;
/// The most effects a voice runs (the shipped sounds have at most 13: `every_sounds_chain_fits`), and the words each takes
/// in its table.
pub const MAX_STAGES: usize = 16;
const STAGE_WORDS: usize = 32;
/// The output's room for voices, made when it is made (the Init bank's voice cap is 256): a voice beyond it is counted
/// and not played, never a buffer grown on the audio thread.
const MAX_ACTIVE: usize = 512;

/// What the audio thread has done, shared by every voice: frames made (a heartbeat: written every `RAMP_FRAMES` frames)
/// and blocks in which the last stage found a sample not finite or beyond the bound (the guard acts a block at a time).
#[derive(Debug, Default)]
pub struct Meter {
    made: AtomicU64,
    bad_blocks: AtomicU64,
    /// Frames decoded and dropped to reach a seek or a loop's restart on the audio thread, and the longest such skip
    /// (microseconds).
    skipped: AtomicU64,
    longest_skip_us: AtomicU64,
    /// Voices the output had no room for.
    refused: AtomicU64,
    /// The master limiter's threshold as an output sample (`f32` bits: the threshold times the limiter's output gain and
    /// the game's volume, as the mix carries both; infinite until the game sets it), and the frames whose mix passed it:
    /// a frame counted is one where Elite's limiter would have acted (the mod builds its output gain only).
    limit: AtomicU32,
    over_limit: AtomicU64,
    /// Voices or decoders the audio thread had to drop itself (the game thread's hands were full or busy).
    dropped_here: AtomicU64,
}

impl Meter {
    fn new() -> Meter {
        Meter { limit: AtomicU32::new(f32::INFINITY.to_bits()), ..Default::default() }
    }
}

/// A sequence lock over words the game thread writes and the audio thread reads at a ramp step: the writer makes the count
/// odd, writes, makes it even; a reader that finds it odd, or changed by the end, keeps what it had (it never waits).
#[derive(Debug, Default)]
struct SeqLock(AtomicU32);

impl SeqLock {
    fn write(&self, f: impl FnOnce()) {
        self.0.fetch_add(1, Ordering::Relaxed);
        fence(Ordering::Release);
        f();
        self.0.fetch_add(1, Ordering::Release);
    }

    /// What `f` loads, if the words changed since `seen` and were read whole (the loads, then an acquire fence, then the
    /// count again: a write that overlapped shows as a changed count).
    fn read<T>(&self, seen: &mut u32, f: impl FnOnce() -> T) -> Option<T> {
        let s0 = self.0.load(Ordering::Acquire);
        if s0 == *seen || s0 & 1 == 1 {
            return None;
        }
        let v = f();
        fence(Ordering::Acquire);
        (self.0.load(Ordering::Relaxed) == s0).then(|| {
            *seen = s0;
            v
        })
    }
}

/// A voice's effects, under a sequence lock. Each stage is `STAGE_WORDS` words: its kind (0 off, 1 EQ, 2 distortion), then
/// an EQ's three bands or a distortion's six (four words each: on and the filter, `bit 8` on and the low byte the filter's
/// index or 0xff; frequency; gain; Q), then a distortion's type, drive, tone, rectification, level and wet share.
#[derive(Debug)]
struct FxTable {
    seq: SeqLock,
    n: AtomicU32,
    stages: [[AtomicU32; STAGE_WORDS]; MAX_STAGES],
}

impl Default for FxTable {
    fn default() -> Self {
        FxTable { seq: SeqLock::default(), n: AtomicU32::new(0), stages: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))) }
    }
}

/// A placed voice's emitter from the listener (metres, the listener's axes: x right, y up, -z ahead), as `f32` bits under
/// a sequence lock: written by the game each frame from the mixer's update, read by the voice at each block's start.
#[derive(Debug, Default)]
struct PlaceWords {
    seq: SeqLock,
    xyz: [AtomicU32; 3],
}

impl PlaceWords {
    /// A place not finite is dropped (the last one stays).
    fn write(&self, p: [f32; 3]) {
        if p.iter().all(|v| v.is_finite()) {
            self.seq.write(|| {
                for (a, v) in self.xyz.iter().zip(p) {
                    a.store(v.to_bits(), Ordering::Relaxed);
                }
            });
        }
    }

    fn read(&self, seen: &mut u32) -> Option<[f32; 3]> {
        self.seq.read(seen, || self.xyz.each_ref().map(|a| f32::from_bits(a.load(Ordering::Relaxed))))
    }
}

/// A band as four words.
fn band_words(b: &EqNow) -> [u32; 4] {
    [u32::from(b.on) << 8 | b.filter.map_or(0xff, |f| f as u32), b.frequency.to_bits(), b.gain_db.to_bits(), b.q.to_bits()]
}

fn band_of(w: &[u32]) -> EqNow {
    EqNow { on: w[0] & 0x100 != 0, filter: EqFilter::from_index(i64::from(w[0] & 0xff)), frequency: f32::from_bits(w[1]), gain_db: f32::from_bits(w[2]), q: f32::from_bits(w[3]) }
}

/// A stage as its words.
fn stage_words(fx: &FxNow) -> [u32; STAGE_WORDS] {
    let mut w = [0u32; STAGE_WORDS];
    match fx {
        FxNow::Off => {}
        FxNow::Eq(bands) => {
            w[0] = 1;
            for (k, b) in bands.iter().enumerate() {
                w[1 + 4 * k..5 + 4 * k].copy_from_slice(&band_words(b));
            }
        }
        FxNow::Distortion(d) => {
            w[0] = 2;
            for (k, b) in d.pre.iter().chain(&d.post).enumerate() {
                w[1 + 4 * k..5 + 4 * k].copy_from_slice(&band_words(b));
            }
            w[25..31].copy_from_slice(&[d.kind, d.drive.to_bits(), d.tone.to_bits(), d.rectification.to_bits(), d.level.to_bits(), d.wet.to_bits()]);
        }
    }
    w
}

fn stage_of(w: &[u32; STAGE_WORDS]) -> FxNow {
    let band = |k: usize| band_of(&w[1 + 4 * k..5 + 4 * k]);
    match w[0] {
        1 => FxNow::Eq(std::array::from_fn(band)),
        2 => FxNow::Distortion(DistortionNow {
            pre: std::array::from_fn(band),
            post: std::array::from_fn(|k| band(k + 3)),
            kind: w[25],
            drive: f32::from_bits(w[26]),
            tone: f32::from_bits(w[27]),
            rectification: f32::from_bits(w[28]),
            level: f32::from_bits(w[29]),
            wet: f32::from_bits(w[30]),
        }),
        _ => FxNow::Off,
    }
}

impl FxTable {
    fn write(&self, fx: &[FxNow]) {
        let n = fx.len().min(MAX_STAGES);
        self.seq.write(|| {
            for (slot, f) in self.stages.iter().zip(&fx[..n]) {
                for (a, v) in slot.iter().zip(stage_words(f)) {
                    a.store(v, Ordering::Relaxed);
                }
            }
            self.n.store(n as u32, Ordering::Relaxed);
        });
    }

    /// The stages into `out` (their count returned) if the table changed since `seen` and was read whole.
    fn read(&self, seen: &mut u32, out: &mut [FxNow; MAX_STAGES]) -> Option<usize> {
        self.seq.read(seen, || {
            let n = (self.n.load(Ordering::Relaxed) as usize).min(MAX_STAGES);
            for (o, slot) in out.iter_mut().zip(&self.stages).take(n) {
                *o = stage_of(&slot.each_ref().map(|a| a.load(Ordering::Relaxed)));
            }
            n
        })
    }
}

/// What a voice's source reads from the game while it plays.
pub struct VoiceCtl {
    /// A frame to jump to (`NO_SEEK`: none).
    seek: AtomicU64,
    /// The mixer's 0-100 low-pass and high-pass values, the playback speed and the level (the mixer's gain times the
    /// game's volume), as `f32` bits.
    lowpass: AtomicU32,
    highpass: AtomicU32,
    speed: AtomicU32,
    level: AtomicU32,
    /// Stopped: the output drops it.
    stop: AtomicBool,
    /// Stopped together with a batch of starts (its sequence number; 0 none): the output drops it at the block where that
    /// batch begins, so a stop and the starts of one mixer step meet at one sample (`Voices::start_queued`).
    stop_with: AtomicU64,
    fx: FxTable,
    /// Where it sits (`VoiceUpdate::place`, fixed for the voice's life; None: it plays through `matrix`) and its emitter's
    /// place now.
    placement: Option<Arc<Placement>>,
    place: PlaceWords,
    /// The stereo gains of a voice that is not placed (`VoiceUpdate::matrix`) and the recording's own gain
    /// (`VoiceUpdate::media_gain`), both fixed for the voice's life.
    matrix: [[f32; 2]; 2],
    media_gain: f32,
    /// Blocks of this voice in which the guard found a sample not finite or beyond the bound, and the low-pass, high-pass
    /// and speed in force at the first (`f32` bits), for the one line the game thread logs.
    bad_blocks: AtomicU32,
    first_bad: [AtomicU32; 3],
    /// Frames this voice has made (as `Meter::made`).
    made: AtomicU64,
    meter: Arc<Meter>,
    /// A long loop's restart (its region not kept in memory): a decoder already at the loop's start, made off the audio
    /// thread, which the source takes at the restart (`try_lock`: it never waits), asking for the next with `wants_spare`;
    /// and the decoder it put down, for the game thread to drop.
    spare: Mutex<Option<Decoder>>,
    wants_spare: AtomicBool,
    retired: Mutex<Option<Decoder>>,
}

/// A recording's decoder.
type Decoder = <AudioSource as Decodable>::Decoder;

impl VoiceCtl {
    fn idle(meter: Arc<Meter>) -> VoiceCtl {
        VoiceCtl {
            seek: AtomicU64::new(NO_SEEK),
            lowpass: AtomicU32::new(0),
            highpass: AtomicU32::new(0),
            speed: AtomicU32::new(1f32.to_bits()),
            level: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            stop_with: AtomicU64::new(0),
            fx: FxTable::default(),
            placement: None,
            place: PlaceWords::default(),
            matrix: [[0.0; 2]; 2],
            media_gain: 1.0,
            bad_blocks: AtomicU32::new(0),
            first_bad: Default::default(),
            made: AtomicU64::new(0),
            meter,
            spare: Mutex::new(None),
            wants_spare: AtomicBool::new(false),
            retired: Mutex::new(None),
        }
    }

    fn new(u: &VoiceUpdate, level: f32, meter: Arc<Meter>) -> VoiceCtl {
        let mut c = VoiceCtl::idle(meter);
        c.write(u, level);
        c.fx.write(&u.fx);
        c.placement = u.place.clone();
        if let Some(p) = u.position {
            c.place.write(p);
        }
        // (Values not finite play nothing: a matrix of zeros, a gain of 1.)
        c.matrix = if u.matrix.iter().flatten().all(|g| g.is_finite()) { u.matrix } else { [[0.0; 2]; 2] };
        c.media_gain = if u.media_gain.is_finite() { u.media_gain } else { 1.0 };
        c
    }

    /// The update's filter values, speed and level; a value that is not finite is dropped (the last one stays). False if
    /// one was dropped.
    fn write(&self, u: &VoiceUpdate, level: f32) -> bool {
        let mut all = true;
        for (a, v) in [(&self.lowpass, u.lowpass), (&self.highpass, u.highpass), (&self.speed, speed(u)), (&self.level, volume(u, level).unwrap_or(f32::NAN))] {
            if v.is_finite() {
                a.store(v.to_bits(), Ordering::Relaxed);
            } else {
                all = false;
            }
        }
        all
    }
}

/// A low-pass or high-pass section's state on one voice: its value moving to the latest in `RAMP_STEPS` steps. A section
/// going from bypassed to running starts from rest (Elite's sound engine clears a filter's memory when its bypass changes:
/// `FUN_144ac2bf0`'s +0xe flag). It runs at the output rate, after the speed's resampling, as Wwise's voice filters do.
struct Filter {
    highpass: bool,
    now: f32,
    from: f32,
    to: f32,
    step: u8,
    coefs: Option<Coefs>,
    channels: Vec<Section>,
}

impl Filter {
    fn new(highpass: bool, value: f32, channels: u16) -> Filter {
        let value = finite_value(value);
        let mut f = Filter { highpass, now: value, from: value, to: value, step: RAMP_STEPS, coefs: None, channels: vec![Section::default(); usize::from(channels.max(1))] };
        f.design();
        f
    }

    /// The section for the value now (None: bypassed, the value and its target both at most 0.1).
    fn design(&mut self) {
        let was = self.coefs.is_some();
        self.coefs = filter::cutoff(self.now.max(self.to), self.highpass, OUTPUT_RATE).and_then(|_| {
            let hz = filter::cutoff(self.now, self.highpass, OUTPUT_RATE).unwrap_or(if self.highpass { 1.0 } else { OUTPUT_RATE * filter::MOST_SHARE });
            filter::coefs(hz, self.highpass, OUTPUT_RATE)
        });
        if !was && self.coefs.is_some() {
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.channels.iter_mut().for_each(|s| *s = Section::default());
    }

    /// One ramp step: a new target starts a ramp from where it is.
    fn tick(&mut self, target: f32) {
        let target = finite_value(target);
        if target != self.to {
            (self.from, self.to) = (self.now, target);
            self.step = 0;
        }
        if self.step < RAMP_STEPS {
            self.step += 1;
            self.now = self.from + (self.to - self.from) * f32::from(self.step) / f32::from(RAMP_STEPS);
            self.design();
        }
    }

    /// The first `n` frames of a block (the channels apart, `BLOCK` apart) in place; a bypassed filter leaves them.
    fn run_block(&mut self, buf: &mut [f32], n: usize) {
        let Some(co) = &self.coefs else { return };
        for (s, chan) in self.channels.iter_mut().zip(buf.chunks_mut(BLOCK)) {
            s.run_block(co, &mut chan[..n]);
        }
    }
}

/// One EQ band on a voice: its values as last read, its section (None: off, or values the design leaves out) and one
/// memory a channel, kept across changes as the plug-in keeps it.
struct EqStage {
    now: EqNow,
    coefs: Option<Coefs>,
    channels: Vec<Section>,
}

impl EqStage {
    fn set(&mut self, b: EqNow) {
        if b == self.now {
            return;
        }
        self.now = b;
        self.coefs = match (b.on, b.filter) {
            (true, Some(f)) => filter::eq_coefs(f, b.frequency, b.gain_db, b.q, OUTPUT_RATE),
            _ => None,
        };
    }
}

/// One effect on a voice: what it runs now, a Parametric EQ's three bands and a Guitar Distortion, both made when the voice
/// is built (so a change of kind allocates nothing); a change of kind starts the new one from rest.
struct Stage {
    now: FxNow,
    bands: [EqStage; 3],
    dist: Distortion,
}

impl Stage {
    fn new(channels: usize) -> Stage {
        Stage {
            now: FxNow::Off,
            bands: std::array::from_fn(|_| EqStage { now: EqNow::OFF, coefs: None, channels: vec![Section::default(); channels] }),
            dist: Distortion::new(channels, OUTPUT_RATE),
        }
    }

    fn set(&mut self, fx: FxNow) {
        if fx == self.now {
            return;
        }
        if std::mem::discriminant(&fx) != std::mem::discriminant(&self.now) {
            self.clear();
        }
        self.now = fx;
        match fx {
            FxNow::Eq(bands) => {
                for (stage, b) in self.bands.iter_mut().zip(bands) {
                    stage.set(b);
                }
            }
            FxNow::Distortion(d) => self.dist.set(&d),
            FxNow::Off => {}
        }
    }

    fn clear(&mut self) {
        self.bands.iter_mut().for_each(|b| b.channels.iter_mut().for_each(|c| *c = Section::default()));
        self.dist.reset();
    }

    /// The first `n` frames of a block of `channels` channels (each `BLOCK` apart) in place: each band that has a section
    /// over the frames (a band that is off is skipped, as the plug-in skips it), or the distortion over them.
    fn run_block(&mut self, buf: &mut [f32], channels: usize, n: usize) {
        match self.now {
            FxNow::Eq(_) => {
                for b in &mut self.bands {
                    let Some(co) = &b.coefs else { continue };
                    for (s, chan) in b.channels.iter_mut().zip(buf.chunks_mut(BLOCK)).take(channels) {
                        s.run_block(co, &mut chan[..n]);
                    }
                }
            }
            FxNow::Distortion(_) => self.dist.block(buf, channels, BLOCK, n),
            FxNow::Off => {}
        }
    }
}

/// A voice's speed moving to the latest in `RAMP_STEPS` steps, as its filter values do.
struct SpeedRamp {
    now: f32,
    from: f32,
    to: f32,
    step: u8,
}

impl SpeedRamp {
    fn tick(&mut self, target: f32) {
        let target = finite_speed(target);
        if target != self.to {
            (self.from, self.to) = (self.now, target);
            self.step = 0;
        }
        if self.step < RAMP_STEPS {
            self.step += 1;
            self.now = self.from + (self.to - self.from) * f32::from(self.step) / f32::from(RAMP_STEPS);
        }
    }
}

/// A placed voice's distance and pan (`VoiceCtl::placement`, `se_core::elite_sound::place`): the place last read and what
/// it gives (the distance's filter shares, added to the voice's values at each block's start), and the pan matrix (the
/// distance's gain in it) of the last block and of this one, which the output ramps between over the block.
struct Placed {
    data: Arc<Placement>,
    seen: u32,
    now: PlaceNow,
    from: [[f32; 2]; 2],
}

impl Placed {
    fn new(data: Arc<Placement>, ctl: &VoiceCtl, channels: usize) -> Placed {
        let mut seen = u32::MAX;
        let now = data.at(ctl.place.read(&mut seen).unwrap_or([0.0; 3]), channels);
        Placed { data, seen, now, from: now.matrix }
    }

    /// A block's start: the place read if it changed, the last block's matrix kept to ramp from.
    fn step(&mut self, ctl: &VoiceCtl, channels: usize) {
        self.from = self.now.matrix;
        if let Some(rel) = ctl.place.read(&mut self.seen) {
            self.now = self.data.at(rel, channels);
        }
    }
}

/// A voice's low-pass and high-pass values now: the mixer's, plus a placed voice's distance shares (the sections clamp the
/// sum to 0-100, as the mixer clamps an unplaced voice's).
fn filter_targets(ctl: &VoiceCtl, placed: Option<&Placed>) -> [f32; 2] {
    let [lp, hp] = [&ctl.lowpass, &ctl.highpass].map(|a| f32::from_bits(a.load(Ordering::Relaxed)));
    match placed {
        Some(p) => [lp + p.now.lowpass, hp + p.now.highpass],
        None => [lp, hp],
    }
}

/// A filter value as the sections take it: one that is not a number is 0 (bypassed).
fn finite_value(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 100.0) } else { 0.0 }
}

/// A speed as the resampler takes it (`speed`'s range; one that is not a number is 1).
fn finite_speed(s: f32) -> f32 {
    if s.is_finite() { s.clamp(0.25, 4.0) } else { 1.0 }
}

/// The samples of a voice, at the output rate: the clip from `start_frame`, cut at `clip.frames`, again and again while
/// plays are left, resampled by its speed (linear between the recording's frames), then through its two filters and its
/// effects. Everything it holds is made when it is built, on a worker thread. On the audio thread it allocates only what
/// the Vorbis decoder does per packet (the decoding library's own buffers, a packet's worth) and, for a seek backwards
/// (a Seek action), the decoder made again; a long loop's restart takes a decoder made off the thread (`VoiceCtl::spare`)
/// and a source that ends is handed back to the game thread to be dropped (`OutputSource`).
pub struct VoiceSource {
    clip: Arc<Clip>,
    ctl: Arc<VoiceCtl>,
    dec: Decoder,
    /// The next frame of the recording to hand out, and the decoder's own position (they differ while the loop region
    /// plays from memory).
    frame: u64,
    dec_at: u64,
    /// The loop region's samples (decoded when built, for a region up to `LOOP_CACHE_S`), and whether it plays from them
    /// now.
    looped: Vec<f32>,
    from_cache: bool,
    plays_left: Option<u32>,
    /// Where a play ends and where the next starts (the loop region, or the whole recording).
    end: u64,
    restart: u64,
    /// The two recording frames the output lies between, and where between (0 to 1).
    cur: Vec<f32>,
    nxt: Vec<f32>,
    phase: f64,
    /// The block made last (`BLOCK` frames a channel, the channels one after another: a mono voice stays one channel until
    /// it is mixed), its frames, and the sample the iterator hands out next.
    buf: Vec<f32>,
    made: usize,
    pos: usize,
    speed: SpeedRamp,
    filters: [Filter; 2],
    /// The effects (room for `MAX_STAGES`, `stages_n` of them in use), the table's sequence last read and a scratch copy.
    stages: Vec<Stage>,
    stages_n: usize,
    fx_seen: u32,
    fx_read: Box<[FxNow; MAX_STAGES]>,
    /// The level now and where its ramp goes (over one step).
    level: f32,
    level_to: f32,
    level_step: f32,
    /// Where it sits (None: centred).
    placed: Option<Placed>,
    /// The last play is over: nothing more to hand out.
    over: bool,
    /// Built and handed over: skips from here on are the audio thread's (`Meter::skipped`).
    built: bool,
}

impl VoiceSource {
    fn new(clip: Arc<Clip>, ctl: Arc<VoiceCtl>, start_frame: u64, plays: Option<u32>, region: Option<(u64, u64)>) -> VoiceSource {
        let dec = clip.source.decoder();
        // (A region that starts past the clip, or ends before it starts, is no region: the whole recording loops.)
        let (restart, end) = region.map(|(a, b)| (a, b.min(clip.frames))).filter(|(a, b)| a < b).unwrap_or((0, clip.frames));
        let ch = usize::from(clip.channels.max(1));
        let speed = finite_speed(f32::from_bits(ctl.speed.load(Ordering::Relaxed)));
        let level = f32::from_bits(ctl.level.load(Ordering::Relaxed));
        let level = if level.is_finite() { level } else { 0.0 };
        let placed = ctl.placement.clone().map(|p| Placed::new(p, &ctl, ch));
        let [lp, hp] = filter_targets(&ctl, placed.as_ref());
        let filters = [Filter::new(false, lp, clip.channels), Filter::new(true, hp, clip.channels)];
        let stages = (0..MAX_STAGES).map(|_| Stage::new(ch)).collect();
        let mut s = VoiceSource {
            clip,
            ctl,
            dec,
            frame: 0,
            dec_at: 0,
            looped: Vec::new(),
            from_cache: false,
            plays_left: plays,
            end,
            restart,
            cur: vec![0.0; ch],
            nxt: vec![0.0; ch],
            phase: 0.0,
            buf: vec![0.0; ch * BLOCK],
            made: 0,
            pos: 0,
            speed: SpeedRamp { now: speed, from: speed, to: speed, step: RAMP_STEPS },
            filters,
            stages,
            stages_n: 0,
            fx_seen: u32::MAX,
            fx_read: Box::new([FxNow::Off; MAX_STAGES]),
            level,
            level_to: level,
            level_step: 0.0,
            placed,
            over: false,
            built: false,
        };
        s.read_fx();
        s.cache_loop();
        s.skip_to(start_frame.min(s.clip.frames.saturating_sub(1)));
        if s.long_loop() {
            let spare = spare_decoder(&s.clip, s.restart);
            if let Ok(mut slot) = s.ctl.spare.lock() {
                *slot = Some(spare);
            }
        }
        let mut first = std::mem::take(&mut s.cur);
        s.over = !s.read_frame(&mut first);
        s.cur = first;
        let mut second = std::mem::take(&mut s.nxt);
        if !s.over && !s.read_frame(&mut second) {
            second.iter_mut().for_each(|x| *x = 0.0);
        }
        s.nxt = second;
        s.built = true;
        s
    }

    /// The loop region decoded into memory, for a voice that loops a region up to `LOOP_CACHE_S` (done when it is built,
    /// on the worker thread). The decoder is left at the region's end.
    fn cache_loop(&mut self) {
        let looping = self.plays_left != Some(1);
        let len = self.end - self.restart;
        if !looping || len as f64 > f64::from(self.clip.rate) * LOOP_CACHE_S {
            return;
        }
        let ch = usize::from(self.clip.channels.max(1));
        let mut buf = Vec::with_capacity(len as usize * ch);
        self.skip_to(self.restart);
        'frames: while self.dec_at < self.end {
            for _ in 0..ch {
                match self.dec.next() {
                    Some(x) => buf.push(x),
                    None => break 'frames,
                }
            }
            self.dec_at += 1;
        }
        if buf.len() == len as usize * ch {
            self.looped = buf;
        }
        self.frame = self.dec_at;
    }

    /// It loops from the decoder, its region not kept in memory: each restart takes a spare decoder.
    fn long_loop(&self) -> bool {
        self.plays_left != Some(1) && self.looped.is_empty()
    }

    /// A long loop's restart: the spare decoder taken if it is there (the one put down goes to the game thread to drop, or
    /// is dropped here if its slot is still full); else the decoder made again here (`skip_to`, counted as a skip).
    fn restart_loop(&mut self) {
        let spare = self.ctl.spare.try_lock().ok().and_then(|mut s| s.take());
        let Some(dec) = spare else {
            self.skip_to(self.restart);
            return;
        };
        let old = std::mem::replace(&mut self.dec, dec);
        match self.ctl.retired.try_lock() {
            Ok(mut slot) if slot.is_none() => *slot = Some(old),
            _ => {
                self.ctl.meter.dropped_here.fetch_add(1, Ordering::Relaxed);
                drop(old);
            }
        }
        self.dec_at = self.restart;
        self.frame = self.restart;
        self.from_cache = false;
        self.ctl.wants_spare.store(true, Ordering::Release);
    }

    /// The decoder back to the clip's start and on to `frame` (decoded and dropped: exact to the sample).
    fn skip_to(&mut self, frame: u64) {
        let started = std::time::Instant::now();
        let from = self.dec_at;
        if frame < self.dec_at {
            self.dec = self.clip.source.decoder();
            self.dec_at = 0;
        }
        let ch = u64::from(self.clip.channels);
        'frames: while self.dec_at < frame {
            for _ in 0..ch {
                if self.dec.next().is_none() {
                    self.dec_at = self.clip.frames;
                    break 'frames;
                }
            }
            self.dec_at += 1;
        }
        self.frame = self.dec_at;
        self.from_cache = false;
        if !self.built {
            return;
        }
        let m = &self.ctl.meter;
        m.skipped.fetch_add(self.dec_at.saturating_sub(from.min(self.dec_at)), Ordering::Relaxed);
        m.longest_skip_us.fetch_max(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    }

    /// The recording's next frame into `buf` (a seek asked first; at a play's end the loop's start, or false once the
    /// last play is over). A decoder that ends inside a frame leaves the rest silent and ends the play.
    fn read_frame(&mut self, buf: &mut [f32]) -> bool {
        let seek = self.ctl.seek.swap(NO_SEEK, Ordering::Relaxed);
        if seek != NO_SEEK {
            self.skip_to(seek.min(self.clip.frames.saturating_sub(1)));
        }
        // The end of a play: the last one plays the recording out, the others go back to the loop's start.
        let last = matches!(self.plays_left, Some(n) if n <= 1);
        if self.frame >= if last { self.clip.frames } else { self.end } {
            match self.plays_left {
                Some(n) if n <= 1 => return false,
                Some(n) => self.plays_left = Some(n - 1),
                None => {}
            }
            if !self.looped.is_empty() {
                self.frame = self.restart;
                self.from_cache = true;
            } else {
                self.restart_loop();
                if self.frame >= self.clip.frames {
                    // (The loop's start lies past what the decoder holds: nothing left to loop.)
                    return false;
                }
            }
        }
        if self.from_cache || (!self.looped.is_empty() && (self.restart..self.end).contains(&self.frame) && self.frame != self.dec_at) {
            let i = (self.frame.saturating_sub(self.restart) as usize).saturating_mul(buf.len());
            if let Some(s) = self.looped.get(i..i + buf.len()).filter(|_| self.frame >= self.restart && self.frame < self.end) {
                buf.copy_from_slice(s);
                self.frame += 1;
                self.from_cache = true;
                return true;
            }
            // (Past the region, the last play's tail: the decoder goes on from where the frame is.)
            if self.dec_at != self.frame {
                self.skip_to(self.frame);
            }
            self.from_cache = false;
        }
        let mut ended = false;
        for x in buf.iter_mut() {
            *x = if ended { 0.0 } else { self.dec.next().unwrap_or_else(|| {
                ended = true;
                0.0
            }) };
        }
        self.frame = if ended { self.clip.frames } else { self.frame + 1 };
        self.dec_at = self.frame;
        true
    }

    /// The effects' table read if it changed: each stage given its values (a band's section redesigned where its values
    /// changed, a distortion's taken at its next buffer).
    fn read_fx(&mut self) {
        if let Some(n) = self.ctl.fx.read(&mut self.fx_seen, &mut self.fx_read) {
            for (stage, f) in self.stages.iter_mut().zip(self.fx_read.iter()).take(n) {
                stage.set(*f);
            }
            self.stages_n = n;
        }
    }

    /// A ramp step's controls taken (at each block's start: the speed, the place, the filter values and the effects step
    /// on, the level's ramp to the latest set over the block).
    fn step_controls(&mut self) {
        self.speed.tick(f32::from_bits(self.ctl.speed.load(Ordering::Relaxed)));
        if let Some(p) = &mut self.placed {
            p.step(&self.ctl, self.cur.len());
        }
        for (f, t) in self.filters.iter_mut().zip(filter_targets(&self.ctl, self.placed.as_ref())) {
            f.tick(t);
        }
        self.read_fx();
        let to = f32::from_bits(self.ctl.level.load(Ordering::Relaxed));
        if to.is_finite() {
            self.level = self.level_to;
            self.level_to = to.clamp(0.0, MOST_VOLUME);
            self.level_step = (self.level_to - self.level) / RAMP_FRAMES as f32;
        }
    }

    /// The recording's frame pair moved on until the output's position lies between them (the end of the last play sets
    /// `over`; the frame then made is the last).
    fn advance(&mut self) {
        while self.phase >= 1.0 {
            std::mem::swap(&mut self.cur, &mut self.nxt);
            let mut next = std::mem::take(&mut self.nxt);
            if !self.read_frame(&mut next) {
                self.over = true;
                next.iter_mut().for_each(|x| *x = 0.0);
            }
            self.nxt = next;
            self.phase -= 1.0;
            if self.over {
                self.phase = 0.0;
                break;
            }
        }
    }

    /// The next block into `self.buf`: its frame count, `BLOCK` while the voice plays, fewer in the block where it ends,
    /// then 0. One block is one ramp step: the controls are taken at its start, the recording is resampled into it (linear
    /// between its frames), then each filter and each effect runs over the whole block in turn, the level last, frame by
    /// frame (each sample gets the arithmetic the frame-at-a-time chain gave it, in the same order, so the same bits:
    /// `blocks_sound_as_frames_did`). The guard acts at the block's end: a bad sample is 0 and the chain starts again from
    /// rest for the next block.
    fn make_block(&mut self) -> usize {
        if self.over {
            return 0;
        }
        self.step_controls();
        let ch = self.cur.len();
        let inc = f64::from(self.clip.rate) / f64::from(OUTPUT_RATE) * f64::from(self.speed.now);
        let gain = self.ctl.media_gain;
        let mut n = 0;
        while n < BLOCK && !self.over {
            self.advance();
            let k = self.phase as f32;
            for (c, (a, b)) in self.cur.iter().zip(&self.nxt).enumerate() {
                self.buf[c * BLOCK + n] = (a + (b - a) * k) * gain;
            }
            self.phase += inc;
            n += 1;
        }
        for f in &mut self.filters {
            f.run_block(&mut self.buf, n);
        }
        for stage in &mut self.stages[..self.stages_n] {
            stage.run_block(&mut self.buf, ch, n);
        }
        let mut bad = false;
        for i in 0..n {
            self.level += self.level_step;
            for c in 0..ch {
                let y = &mut self.buf[c * BLOCK + i];
                *y = if y.is_finite() && y.abs() <= filter::CHAIN_BOUND { *y * self.level } else {
                    bad = true;
                    0.0
                };
            }
        }
        if bad {
            self.not_finite();
        }
        self.ctl.meter.made.fetch_add(n as u64, Ordering::Relaxed);
        self.ctl.made.fetch_add(n as u64, Ordering::Relaxed);
        n
    }

    /// The guard of the last stage: a block with a sample that is not finite or beyond `filter::CHAIN_BOUND` has it as 0,
    /// the voice's filters and effects start again from rest, and it is counted (the values in force at the first are kept
    /// for the game thread's log line).
    #[cold]
    fn not_finite(&mut self) {
        self.filters.iter_mut().for_each(Filter::clear);
        self.stages.iter_mut().for_each(Stage::clear);
        self.ctl.meter.bad_blocks.fetch_add(1, Ordering::Relaxed);
        if self.ctl.bad_blocks.fetch_add(1, Ordering::Relaxed) == 0 {
            let now = [self.filters[0].now, self.filters[1].now, self.speed.now];
            for (a, v) in self.ctl.first_bad.iter().zip(now) {
                a.store(v.to_bits(), Ordering::Relaxed);
            }
        }
    }
}

/// A voice alone as a source of its own (its samples interleaved, at the output rate): what the tests pull.
impl Iterator for VoiceSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let ch = self.cur.len().max(1);
        if self.pos >= self.made * ch {
            self.made = self.make_block();
            self.pos = 0;
            if self.made == 0 {
                return None;
            }
        }
        let (i, c) = (self.pos / ch, self.pos % ch);
        self.pos += 1;
        Some(self.buf[c * BLOCK + i])
    }
}

impl Source for VoiceSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.clip.channels.max(1)).unwrap_or(ChannelCount::MIN)
    }

    /// Always the output rate: the speed is applied here, so the player converts nothing for it.
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(OUTPUT_RATE as u32).unwrap_or(SampleRate::MIN)
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// A voice ready for the output: its source and its controls.
pub struct Ready {
    src: VoiceSource,
    ctl: Arc<VoiceCtl>,
}

/// What the output hands back to the game thread to drop: voices that ended (a decoder and up to a megabyte of loop
/// region each) and the batches' emptied lists. The room is made with it; the game thread empties it every frame.
#[derive(Default)]
pub struct Returned {
    voices: Vec<Ready>,
    batches: Vec<Vec<Ready>>,
}

impl Returned {
    fn with_room() -> Returned {
        Returned { voices: Vec::with_capacity(MAX_ACTIVE), batches: Vec::with_capacity(MAX_ACTIVE) }
    }
}

/// A batch of starts as the output receives it: its sequence number (from 1, in the order asked for) and its voices.
type BatchMsg = (u64, Vec<Ready>);

/// The ship's output: every built voice mixed at the output rate into two channels, a block (`BLOCK` frames) at a time.
/// Batches of voices arrive whole through `rx` and start together at a block's first frame, and the voices stopped with
/// them (`VoiceCtl::stop_with`) end there. On the audio thread it allocates nothing of its own (its lists' room is made
/// with it), never blocks (`try_recv`, `try_lock`), and drops nothing it can hand back: what ends waits in `held` and goes
/// to `returned` when its lock is free (a full list drops the rest here, counted in `Meter::dropped_here`). The channel
/// itself frees a block of its own every 31 batches received (the standard library's list channel).
pub struct OutputSource {
    rx: Receiver<BatchMsg>,
    active: Vec<Ready>,
    held: Returned,
    returned: Arc<Mutex<Returned>>,
    /// The block being handed out, its frames' two channels interleaved, and the next sample of it.
    mix: [f32; 2 * BLOCK],
    pos: usize,
    /// The newest batch begun.
    begun: u64,
    meter: Arc<Meter>,
}

impl OutputSource {
    fn new(rx: Receiver<BatchMsg>, meter: Arc<Meter>, returned: Arc<Mutex<Returned>>) -> OutputSource {
        OutputSource { rx, active: Vec::with_capacity(MAX_ACTIVE), held: Returned::with_room(), returned, mix: [0.0; 2 * BLOCK], pos: 2 * BLOCK, begun: 0, meter }
    }

    /// A voice that ended, or a spent batch list, kept to hand back (dropped here only if the room is full).
    fn hold_voice(&mut self, r: Ready) {
        if self.held.voices.len() < self.held.voices.capacity() {
            self.held.voices.push(r);
        } else {
            self.meter.dropped_here.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn hold_batch(&mut self, b: Vec<Ready>) {
        if self.held.batches.len() < self.held.batches.capacity() {
            self.held.batches.push(b);
        } else {
            self.meter.dropped_here.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// What is held handed to the game thread, as much as its room takes, if its lock is free now.
    fn hand_back(&mut self) {
        if self.held.voices.is_empty() && self.held.batches.is_empty() {
            return;
        }
        let Ok(mut r) = self.returned.try_lock() else { return };
        while r.voices.len() < r.voices.capacity() {
            let Some(v) = self.held.voices.pop() else { break };
            r.voices.push(v);
        }
        while r.batches.len() < r.batches.capacity() {
            let Some(b) = self.held.batches.pop() else { break };
            r.batches.push(b);
        }
    }

    /// One output block: the batches that arrived join first (all of a batch at its first frame), then every voice adds its
    /// block through its matrix (`mix_through`); a voice stopped (at once, or with a
    /// batch now begun) or over is held to be handed back. Each frame whose mix passes the limiter's threshold is counted.
    fn mix_block(&mut self) {
        while let Ok((seq, mut batch)) = self.rx.try_recv() {
            self.begun = self.begun.max(seq);
            while let Some(r) = batch.pop() {
                if r.ctl.stop.load(Ordering::Relaxed) {
                    self.hold_voice(r);
                } else if self.active.len() < MAX_ACTIVE {
                    self.active.push(r);
                } else {
                    self.meter.refused.fetch_add(1, Ordering::Relaxed);
                    self.hold_voice(r);
                }
            }
            self.hold_batch(batch);
        }
        self.mix.fill(0.0);
        let mut i = 0;
        while i < self.active.len() {
            let r = &mut self.active[i];
            let with = r.ctl.stop_with.load(Ordering::Relaxed);
            let stopped = r.ctl.stop.load(Ordering::Relaxed) || (with != 0 && with <= self.begun);
            let n = if stopped { 0 } else { r.src.make_block() };
            let (b, ch) = (&r.src.buf, r.src.cur.len());
            let (from, to) = r.src.placed.as_ref().map_or((r.ctl.matrix, r.ctl.matrix), |p| (p.from, p.now.matrix));
            mix_through(&mut self.mix, b, ch, n, from, to);
            if n < BLOCK {
                let done = self.active.swap_remove(i);
                self.hold_voice(done);
                continue;
            }
            i += 1;
        }
        let limit = f32::from_bits(self.meter.limit.load(Ordering::Relaxed));
        let over = self.mix.chunks_exact(2).filter(|f| f[0].abs() > limit || f[1].abs() > limit).count();
        if over > 0 {
            self.meter.over_limit.fetch_add(over as u64, Ordering::Relaxed);
        }
        self.pos = 0;
        self.hand_back();
    }
}

/// A voice's first `n` frames (`ch` channels, `BLOCK` apart in `b`) added to the stereo mix through its matrix (`[input
/// channel][output channel]`), each gain ramped frame by frame from the last block's matrix to this one's over a ramp step,
/// so a moving emitter or a turning head never steps a gain (a voice not placed has the same matrix at both ends: the ramp
/// adds exactly 0, the same bits as a fixed gain).
fn mix_through(mix: &mut [f32; 2 * BLOCK], b: &[f32], ch: usize, n: usize, from: [[f32; 2]; 2], to: [[f32; 2]; 2]) {
    let d = |c: usize, o: usize| (to[c][o] - from[c][o]) / RAMP_FRAMES as f32;
    let (d00, d01, d10, d11) = (d(0, 0), d(0, 1), d(1, 0), d(1, 1));
    for (i, f) in mix.chunks_exact_mut(2).take(n).enumerate() {
        let k = (i + 1) as f32;
        let l = b[i];
        f[0] += l * (from[0][0] + d00 * k);
        f[1] += l * (from[0][1] + d01 * k);
        if ch > 1 {
            let r = b[BLOCK + i];
            f[0] += r * (from[1][0] + d10 * k);
            f[1] += r * (from[1][1] + d11 * k);
        }
    }
}

impl Iterator for OutputSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos == self.mix.len() {
            self.mix_block();
        }
        let s = self.mix[self.pos];
        self.pos += 1;
        Some(s)
    }
}

impl Source for OutputSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap_or(ChannelCount::MIN)
    }

    /// The output rate: every voice is resampled to it by its own source, so the player converts nothing.
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(OUTPUT_RATE as u32).unwrap_or(SampleRate::MIN)
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// The ship's output as a Bevy audio asset: the source made with it, taken by the first `decoder` call.
#[derive(Asset, TypePath, Clone)]
pub struct ShipOutput {
    ready: Arc<Mutex<Option<OutputSource>>>,
}

impl Decodable for ShipOutput {
    type Decoder = OutputSource;

    fn decoder(&self) -> OutputSource {
        self.ready.lock().ok().and_then(|mut r| r.take()).unwrap_or_else(|| OutputSource::new(channel().1, Arc::new(Meter::new()), Arc::default()))
    }
}

/// Where a voice's start puts it: the frame, how many plays (None: for ever) and the looped region.
fn start_of(u: &VoiceUpdate, rate: u32) -> Option<(u64, Option<u32>, Option<(u64, u64)>)> {
    let start = u.start?;
    let plays = if start.looping { start.loop_count } else { Some(1) };
    let frame = (f64::from(start.offset.max(0.0)) * f64::from(rate)) as u64;
    Some((frame, plays, if start.looping { start.loop_region } else { None }))
}

/// Where a voice's loop restarts (frames): its region's start, as `VoiceSource::new` takes the region.
fn loop_restart(region: Option<(u64, u64)>, frames: u64) -> u64 {
    region.map(|(a, b)| (a, b.min(frames))).filter(|(a, b)| a < b).map_or(0, |(a, _)| a)
}

/// The output's entity.
#[derive(Component)]
pub struct ShipVoice;

/// The voices built or being built (the mixer's id to their controls and recording), this frame's starts, the batches on
/// worker threads and the output.
pub struct Voices {
    playing: HashMap<u32, Playing>,
    queued: Vec<Queued>,
    /// Voices stopped in this mixer step (or gone virtual, or replaced): they end with this step's starts if it has any,
    /// else at once (`start_queued`).
    stopping: Vec<Arc<VoiceCtl>>,
    /// Voices stopped with a batch not yet handed to the output: its sequence number and when (`settle_stops`). They stay
    /// here until the batch goes (the output then ends them where it begins), so a stop of everything meanwhile reaches
    /// them too (`stop_all`); one whose batch is not handed over within `STOP_WAIT_S` stops at once (`expire_stops`).
    waiting: Vec<(u64, Arc<VoiceCtl>, std::time::Instant)>,
    building: VecDeque<Batch>,
    /// The next batch's sequence number.
    next_batch: u64,
    output: Option<(Entity, Sender<BatchMsg>)>,
    meter: Arc<Meter>,
    /// What the output hands back, taken here every frame by swapping in `spare` (emptied, its room kept: nothing is
    /// allocated for it frame to frame).
    returned: Arc<Mutex<Returned>>,
    spare: Returned,
    /// Voices whose long loop asked for a spare decoder, being made on worker threads.
    spares: Vec<std::thread::JoinHandle<()>>,
    /// The output stopped: its entity, despawned `RETIRE_S` later (the output hands back what ends every frame, so by then
    /// its voices are back here to be dropped).
    retiring: Option<(Entity, std::time::Instant)>,
}

impl Default for Voices {
    fn default() -> Self {
        Voices {
            playing: HashMap::new(),
            queued: Vec::new(),
            stopping: Vec::new(),
            waiting: Vec::new(),
            building: VecDeque::new(),
            next_batch: 1,
            output: None,
            meter: Arc::new(Meter::new()),
            returned: Arc::new(Mutex::new(Returned::with_room())),
            spare: Returned::with_room(),
            spares: Vec::new(),
            retiring: None,
        }
    }
}

/// How long a stopped output is kept for its voices to come back before it is despawned.
const RETIRE_S: f64 = 0.5;
/// The longest a stop waits for the batch it is to meet (ours: about ten times the slowest hand-over measured, 28 ms in
/// T5d-4's acceptance run); past it the voice stops at once, so a build that never returns holds no stop for ever.
const STOP_WAIT_S: f64 = 0.25;

struct Playing {
    ctl: Arc<VoiceCtl>,
    clip: Arc<Clip>,
    /// The effects last written (the table is written only when they change).
    fx: Vec<FxNow>,
    /// The guard's lines already logged for this voice: an update not finite, a sample not finite.
    told_update: bool,
    told_sample: bool,
    /// The voice's frame count at the last `meter`.
    made_seen: u64,
    /// Where its loop restarts (frames), for a spare decoder.
    restart: u64,
}

/// A start waiting for `start_queued`.
struct Queued {
    voice: u32,
    clip: Arc<Clip>,
    ctl: Arc<VoiceCtl>,
    start_frame: u64,
    plays: Option<u32>,
    region: Option<(u64, u64)>,
}

/// One frame's starts on worker threads: its sequence number, the threads, when asked, and how many.
struct Batch {
    seq: u64,
    parts: Vec<std::thread::JoinHandle<Vec<Option<Ready>>>>,
    asked: std::time::Instant,
    count: usize,
}

/// A voice's speed (a pitch that is not finite gives NaN: the callers drop it).
fn speed(u: &VoiceUpdate) -> f32 {
    if u.pitch.is_finite() { u.pitch.clamp(0.25, 4.0) } else { f32::NAN }
}

/// A voice's level: the mixer's gain (silent while virtual) times the game's volume; None if it is not finite.
fn volume(u: &VoiceUpdate, level: f32) -> Option<f32> {
    if u.virtual_voice {
        return Some(0.0);
    }
    let v = u.gain * level;
    v.is_finite().then(|| v.clamp(0.0, MOST_VOLUME))
}

/// A decoder at `frame` of the clip (decoded and dropped up to it), made off the audio thread.
fn spare_decoder(clip: &Clip, frame: u64) -> Decoder {
    let mut dec = clip.source.decoder();
    let samples = frame * u64::from(clip.channels.max(1));
    for _ in 0..samples {
        if dec.next().is_none() {
            break;
        }
    }
    dec
}

/// Builds one voice; a build that panics gives None (that voice is lost, the others of its batch go on).
fn build(q: &Queued) -> Option<Ready> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Ready { src: VoiceSource::new(q.clip.clone(), q.ctl.clone(), q.start_frame, q.plays, q.region), ctl: q.ctl.clone() })).ok()
}

/// What happened to a voice this frame, for the log.
pub enum Event<'a> {
    Started(&'a VoiceUpdate, &'a str),
    Stopped(&'a VoiceUpdate),
    /// Gone virtual: dropped until it is heard again.
    Virtual(&'a VoiceUpdate),
    /// No recording for its source (not shipped or refused).
    Silent(&'a VoiceUpdate),
}

impl Voices {
    pub fn count(&self) -> usize {
        self.playing.len()
    }

    /// Starts queued this frame.
    pub fn queued(&self) -> usize {
        self.queued.len()
    }

    /// Every voice stopped at once (the playing ones, this step's stopped ones and those waiting for their batch): each is
    /// told to stop, so the output hands it back to be dropped here, and the output is despawned `RETIRE_S` later
    /// (`service`); batches still building are dropped when done. A new start meanwhile makes a new output.
    pub fn stop_all(&mut self, commands: &mut Commands) {
        for ctl in self.playing.values().map(|p| &p.ctl).chain(&self.stopping).chain(self.waiting.iter().map(|w| &w.1)) {
            ctl.stop.store(true, Ordering::Relaxed);
        }
        self.stopping.clear();
        self.waiting.clear();
        if let Some((e, _)) = self.output.take() {
            if let Some((old, _)) = self.retiring.replace((e, std::time::Instant::now())) {
                commands.entity(old).try_despawn();
            }
        }
        self.playing.clear();
        self.queued.clear();
        self.building.clear();
    }

    /// Once a frame, playing or not: what the output handed back dropped here, the spare decoders long loops asked for
    /// made on worker threads (and the ones they put down dropped), a stopped output despawned once its voices are back,
    /// stops that waited too long for their batch acted on.
    pub fn service(&mut self, commands: &mut Commands) {
        self.expire_stops(std::time::Instant::now());
        if let Ok(mut r) = self.returned.lock() {
            std::mem::swap(&mut *r, &mut self.spare);
        }
        self.spare.voices.clear();
        self.spare.batches.clear();
        if let Some((e, at)) = self.retiring {
            if at.elapsed().as_secs_f64() > RETIRE_S {
                commands.entity(e).try_despawn();
                self.retiring = None;
            }
        }
        self.spares.retain(|t| !t.is_finished());
        for p in self.playing.values() {
            let retired = p.ctl.retired.try_lock().ok().and_then(|mut r| r.take());
            drop(retired);
            if p.ctl.wants_spare.swap(false, Ordering::AcqRel) {
                let (ctl, clip) = (p.ctl.clone(), p.clip.clone());
                let restart = p.restart;
                self.spares.push(std::thread::spawn(move || {
                    let dec = spare_decoder(&clip, restart);
                    if let Ok(mut s) = ctl.spare.lock() {
                        *s = Some(dec);
                    }
                }));
            }
        }
    }

    /// Frames the master limiter would have acted on so far, and the threshold the output compares with (an output sample:
    /// the limiter's threshold times its output gain and the game's volume).
    pub fn set_limit(&self, limiter: Option<se_core::elite_sound::LimiterNow>, level: f32) {
        let v = limiter.map_or(f32::INFINITY, |l| 10f32.powf((l.threshold_db + l.output_db) / 20.0) * level);
        self.meter.limit.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn over_limit(&self) -> (u64, u64) {
        (self.meter.over_limit.load(Ordering::Relaxed), self.meter.dropped_here.load(Ordering::Relaxed))
    }

    /// One mixer update: queue its voice's start, update it in place, drop it when it goes virtual or stop it (at this
    /// step's starts: `start_queued`). `level` maps the mixer's gain to the output's level.
    pub fn apply<'a>(&mut self, u: &'a VoiceUpdate, clip: Option<&'a Arc<Clip>>, level: f32, log: &mut impl FnMut(Event<'a>)) {
        if u.stop || u.virtual_voice {
            if let Some(p) = self.playing.remove(&u.voice) {
                self.stopping.push(p.ctl);
                log(if u.stop { Event::Stopped(u) } else { Event::Virtual(u) });
            }
            self.queued.retain(|q| q.voice != u.voice);
            return;
        }
        if u.start.is_some() {
            let Some((c, (start_frame, plays, region))) = clip.and_then(|c| Some((c, start_of(u, c.rate)?))) else {
                log(Event::Silent(u));
                return;
            };
            log(Event::Started(u, &c.file));
            let ctl = Arc::new(VoiceCtl::new(u, level, self.meter.clone()));
            if let Some(old) = self.playing.insert(u.voice, Playing { ctl: ctl.clone(), clip: c.clone(), fx: u.fx.clone(), told_update: false, told_sample: false, made_seen: 0, restart: loop_restart(region, c.frames) }) {
                self.stopping.push(old.ctl);
            }
            self.queued.retain(|q| q.voice != u.voice);
            self.queued.push(Queued { voice: u.voice, clip: c.clone(), ctl, start_frame, plays, region });
            return;
        }
        let Some(p) = self.playing.get_mut(&u.voice) else { return };
        if let (Some(_), Some(at)) = (&p.ctl.placement, u.position) {
            p.ctl.place.write(at);
        }
        if let Some(to) = u.seek_to.filter(|t| t.is_finite()) {
            p.ctl.seek.store((f64::from(to.max(0.0)) * f64::from(p.clip.rate)) as u64, Ordering::Relaxed);
        }
        if p.fx != u.fx {
            p.ctl.fx.write(&u.fx);
            p.fx.clone_from(&u.fx);
        }
        if !p.ctl.write(u, level) && !std::mem::replace(&mut p.told_update, true) {
            warn!(
                "sidewinder sound: voice {} {}: an update not finite (gain {}, pitch {}, low-pass {}, high-pass {}) dropped, the last values kept",
                u.voice, p.clip.file, u.gain, u.pitch, u.lowpass, u.highpass
            );
        }
    }

    /// The frame's queued starts sent to worker threads as one batch (shared among a few threads when there are more than
    /// `BUILD_ALONE`); the game thread does not wait for them. The output is spawned with the first start. The voices this
    /// mixer step stopped end where the batch begins (T5d-4: stopped at once, they left a hole of the batch's build time,
    /// 6 to 13 ms, at every supercruise entry); with no starts in the step they stop at once.
    pub fn start_queued(&mut self, commands: &mut Commands, outputs: &mut Assets<ShipOutput>) {
        if self.queued.is_empty() {
            self.settle_stops(None);
            return;
        }
        let seq = self.next_batch;
        self.next_batch += 1;
        self.settle_stops(Some(seq));
        if self.output.is_none() {
            let (tx, rx) = channel();
            let h = outputs.add(ShipOutput { ready: Arc::new(Mutex::new(Some(OutputSource::new(rx, self.meter.clone(), self.returned.clone())))) });
            let e = commands.spawn((ShipVoice, AudioPlayer(h), PlaybackSettings::ONCE.with_volume(Volume::Linear(1.0)))).id();
            self.output = Some((e, tx));
        }
        let jobs = std::mem::take(&mut self.queued);
        let count = jobs.len();
        let threads = if count <= BUILD_ALONE { 1 } else { std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 8) };
        let per = count.div_ceil(threads);
        let mut parts = Vec::new();
        let mut jobs = jobs.into_iter().peekable();
        while jobs.peek().is_some() {
            let part: Vec<Queued> = jobs.by_ref().take(per).collect();
            parts.push(std::thread::spawn(move || part.iter().map(build).collect()));
        }
        self.building.push_back(Batch { seq, parts, asked: std::time::Instant::now(), count });
    }

    /// This step's stopped voices told when to end: with the batch `with` (where it begins; kept in `waiting` until it is
    /// handed over), or at once.
    fn settle_stops(&mut self, with: Option<u64>) {
        let now = std::time::Instant::now();
        for ctl in self.stopping.drain(..) {
            match with {
                Some(seq) => {
                    ctl.stop_with.store(seq, Ordering::Relaxed);
                    self.waiting.push((seq, ctl, now));
                }
                None => ctl.stop.store(true, Ordering::Relaxed),
            }
        }
    }

    /// The stops that waited `STOP_WAIT_S` for their batch made at once (a build that never returns holds none for ever).
    fn expire_stops(&mut self, now: std::time::Instant) {
        self.waiting.retain(|(_, ctl, at)| {
            let late = now.saturating_duration_since(*at).as_secs_f64() >= STOP_WAIT_S;
            if late {
                ctl.stop.store(true, Ordering::Relaxed);
            }
            !late
        });
    }

    /// The batches whose voices are all built handed to the output, in the order they were asked for. Returns, for each
    /// batch handed over, its size and how long it waited (for the log).
    pub fn hand_over(&mut self) -> Vec<(usize, f64)> {
        let mut done = Vec::new();
        while self.building.front().is_some_and(|b| b.parts.iter().all(|t| t.is_finished())) {
            let Some(b) = self.building.pop_front() else { break };
            let mut ready = Vec::with_capacity(b.count);
            let mut lost = 0;
            for t in b.parts {
                match t.join() {
                    Ok(v) => {
                        lost += v.iter().filter(|r| r.is_none()).count();
                        ready.extend(v.into_iter().flatten());
                    }
                    Err(_) => lost += 1,
                }
            }
            if lost > 0 {
                warn!("sidewinder sound: {lost} of {} starts could not be built (they stay silent)", b.count);
            }
            let sent = self.output.as_ref().is_some_and(|(_, tx)| tx.send((b.seq, ready)).is_ok());
            // (The stops waiting for this batch are the output's now; with no output to take it they stop at once.)
            self.waiting.retain(|(seq, ctl, _)| {
                if *seq > b.seq {
                    return true;
                }
                if !sent {
                    ctl.stop.store(true, Ordering::Relaxed);
                }
                false
            });
            done.push((b.count, b.asked.elapsed().as_secs_f64()));
        }
        done
    }

    /// Frames the audio thread has made and blocks in which the guard acted, since the voices began; frames skipped and the
    /// longest skip (us) since the last call; voices refused for want of room so far; the voices that made the most frames
    /// since the last call (id, file, frames, speed).
    pub fn meter(&mut self) -> (u64, u64, u64, u64, u64, Vec<(u32, String, u64, f32)>) {
        let m = &self.meter;
        let mut top: Vec<(u32, String, u64, f32)> = self
            .playing
            .iter_mut()
            .map(|(id, p)| {
                let made = p.ctl.made.load(Ordering::Relaxed);
                let d = made - std::mem::replace(&mut p.made_seen, made);
                (*id, p.clip.file.clone(), d, f32::from_bits(p.ctl.speed.load(Ordering::Relaxed)))
            })
            .collect();
        top.sort_by_key(|t| std::cmp::Reverse(t.2));
        top.truncate(3);
        (
            m.made.load(Ordering::Relaxed),
            m.bad_blocks.load(Ordering::Relaxed),
            m.skipped.swap(0, Ordering::Relaxed),
            m.longest_skip_us.swap(0, Ordering::Relaxed),
            m.refused.load(Ordering::Relaxed),
            top,
        )
    }

    /// The guard's one line a voice: the first sample of it the last stage found not finite.
    pub fn tell_guard(&mut self) {
        for (id, p) in &mut self.playing {
            let n = p.ctl.bad_blocks.load(Ordering::Relaxed);
            if n > 0 && !std::mem::replace(&mut p.told_sample, true) {
                let [lp, hp, sp] = p.ctl.first_bad.each_ref().map(|a| f32::from_bits(a.load(Ordering::Relaxed)));
                warn!("sidewinder sound: voice {id} {}: a sample not finite replaced by 0 and its filters reset (speed {sp:.3}, low-pass {lp:.1}, high-pass {hp:.1}; {n} blocks so far)", p.clip.file);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono 16-bit WAV whose sample i is `i` (as a 16-bit value).
    fn ramp(frames: u16) -> Clip {
        let data: Vec<u8> = (0..frames).flat_map(|i| (i as i16).to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&[1, 0, 1, 0]);
        b.extend_from_slice(&48000u32.to_le_bytes());
        b.extend_from_slice(&96000u32.to_le_bytes());
        b.extend_from_slice(&[2, 0, 16, 0]);
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        let (source, _) = crate::mods::check_wav(b).unwrap();
        Clip { source, file: "ramp.wav".into(), frames: u64::from(frames), channels: 1, rate: 48000 }
    }

    /// A mono voice on both channels at full level (these tests' matrix: their samples are the voice's own).
    const MONO_BOTH: [[f32; 2]; 2] = [[1.0, 1.0], [0.0, 0.0]];

    /// A voice's controls at level 1 through `MONO_BOTH`, nothing else set.
    fn ctl() -> Arc<VoiceCtl> {
        let mut c = VoiceCtl::idle(Arc::default());
        c.level.store(1f32.to_bits(), Ordering::Relaxed);
        c.matrix = MONO_BOTH;
        Arc::new(c)
    }

    fn play(start: u64, plays: Option<u32>, region: Option<(u64, u64)>, take: usize) -> Vec<i32> {
        VoiceSource::new(Arc::new(ramp(10)), ctl(), start, plays, region).take(take).map(|s| (s * 32768.0).round() as i32).collect()
    }

    #[test]
    fn loops_join_sample_for_sample() {
        // The whole recording three times, nothing between the plays.
        assert_eq!(play(0, Some(3), None, 100), [(0..10).collect::<Vec<_>>(), (0..10).collect(), (0..10).collect()].concat());
        // From an offset, for ever.
        assert_eq!(play(7, None, None, 6), [7, 8, 9, 0, 1, 2]);
        // An intro, the region looped, the last play running out to the end.
        assert_eq!(play(0, Some(3), Some((3, 6)), 100), [0, 1, 2, 3, 4, 5, 3, 4, 5, 3, 4, 5, 6, 7, 8, 9]);
        // A region that starts past the clip is no region: the whole recording loops.
        assert_eq!(play(0, Some(2), Some((20, 30)), 100), [(0..10).collect::<Vec<_>>(), (0..10).collect()].concat());
    }

    /// The speed is the source's own (it hands the player the output rate, so nothing outside resamples it): a change
    /// while it plays ramps in over 8 steps of 128 frames, after which the recording goes by at the new speed. (Played by
    /// the player's speed, a voice kept the speed it started with: its converter is built once, the source giving no span
    /// lengths.)
    #[test]
    fn speed_changes_while_playing_move_the_pitch() {
        let c = ctl();
        let mut src = VoiceSource::new(Arc::new(ramp(20000)), c.clone(), 0, Some(1), None);
        let first: Vec<f32> = src.by_ref().take(1000).collect();
        assert!(first.windows(2).all(|w| ((w[1] - w[0]) * 32768.0 - 1.0).abs() < 1e-3), "speed 1: a frame a frame");
        c.speed.store(2f32.to_bits(), Ordering::Relaxed);
        let later: Vec<f32> = src.by_ref().take(3000).collect();
        let step = |w: &[f32]| (w[1] - w[0]) * 32768.0;
        assert!(later[2000..].windows(2).all(|w| (step(w) - 2.0).abs() < 1e-3), "speed 2: two frames a frame");
        c.speed.store(f32::NAN.to_bits(), Ordering::Relaxed);
        let nan: Vec<f32> = src.by_ref().take(2000).collect();
        assert!(nan[1500..].windows(2).all(|w| (step(w) - 1.0).abs() < 1e-3), "a speed that is not a number plays at 1");
    }

    /// A filter value of 0.1 or less leaves the samples untouched; a high-pass at 100 takes a ramp's slow rise nearly out
    /// once its value has ramped in (8 steps of 128 frames).
    #[test]
    fn filters_bypass_at_zero_and_cut_when_set() {
        let c = ctl();
        let clip = Arc::new(ramp(4000));
        let plain: Vec<f32> = VoiceSource::new(clip.clone(), c.clone(), 0, Some(1), None).collect();
        assert_eq!(plain.len(), 4000);
        assert!(plain.iter().enumerate().all(|(i, s)| (s * 32768.0).round() as usize == i));
        c.highpass.store(100f32.to_bits(), Ordering::Relaxed);
        let cut: Vec<f32> = VoiceSource::new(clip, c, 0, Some(1), None).collect();
        let late = cut[2000..].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(late < plain[2000] * 0.05, "{late}");
    }

    /// A voice whose filter values, speed and silence change every 800 samples (as the game writes them), random within
    /// their ranges and with the edge values, NaN among them: no panic, every sample finite and bounded, and fast.
    #[test]
    #[ignore]
    fn filters_survive_values_changing_while_playing() {
        let l = super::super::load::load().unwrap_or_else(|(f, e)| panic!("{f}: {e}"));
        let mut seed = 7u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        let edge = [0.0f32, 0.1, 0.10001, 30.0, 99.99, 100.0, f32::NAN];
        let t = std::time::Instant::now();
        let mut n = 0usize;
        for clip in l.clips.values().take(120) {
            let c = ctl();
            let mut src = VoiceSource::new(clip.clone(), c.clone(), 0, None, None);
            for _ in 0..200 {
                for (a, pick) in [(&c.lowpass, rnd()), (&c.highpass, rnd())] {
                    let v = if pick < 0.3 { edge[(rnd() * edge.len() as f32) as usize % edge.len()] } else { rnd() * 100.0 };
                    a.store(v.to_bits(), Ordering::Relaxed);
                }
                c.speed.store((0.25 + rnd() * 3.75).to_bits(), Ordering::Relaxed);
                for _ in 0..800 * usize::from(clip.channels) {
                    let s = src.next().unwrap();
                    assert!(s.is_finite() && s.abs() < 2.0, "{}: {s}", clip.file);
                    n += 1;
                }
            }
        }
        eprintln!("{n} samples in {:.2} s", t.elapsed().as_secs_f64());
    }

    /// Every shipped recording through both filters at their extremes and a slow speed (the drop's 0.23), looped twice:
    /// the samples stay finite and bounded, and a second of it takes far less than a second to make.
    #[test]
    #[ignore]
    fn every_recording_stays_finite_through_the_filters() {
        let l = super::super::load::load().unwrap_or_else(|(f, e)| panic!("{f}: {e}"));
        for (lp, hp, sp) in [(0.0f32, 0.0f32, 1.0f32), (100.0, 77.0, 0.232), (35.0, 0.0, 1.0), (0.0, 100.0, 4.0)] {
            let t0 = std::time::Instant::now();
            for clip in l.clips.values() {
                let c = ctl();
                c.lowpass.store(lp.to_bits(), Ordering::Relaxed);
                c.highpass.store(hp.to_bits(), Ordering::Relaxed);
                c.speed.store(sp.to_bits(), Ordering::Relaxed);
                let n = (clip.frames as usize * 2).min(clip.rate as usize) * usize::from(clip.channels);
                let t = std::time::Instant::now();
                let worst = VoiceSource::new(clip.clone(), c, 0, Some(2), None).take(n).fold(0.0f32, |m, s| if s.is_finite() { m.max(s.abs()) } else { f32::INFINITY });
                assert!(worst < 4.0, "{}: {worst} at LPF {lp} HPF {hp} speed {sp}", clip.file);
                assert!(t.elapsed().as_secs_f64() < 0.5, "{}: {:?} for {n} samples", clip.file, t.elapsed());
            }
            eprintln!("LPF {lp} HPF {hp} speed {sp}: {} recordings in {:.2} s", l.clips.len(), t0.elapsed().as_secs_f64());
        }
        // The drop's cooldown blips (the speed clamped to 0.25): values ramping in from 0 while they play.
        for f in ["sfx/13a829e5.ogg", "sfx/07ad946d.ogg", "sfx/01901a52.ogg"] {
            let clip = l.clips.values().find(|c| c.file == f).unwrap();
            let c = ctl();
            c.speed.store(0.25f32.to_bits(), Ordering::Relaxed);
            let mut src = VoiceSource::new(clip.clone(), c.clone(), 0, Some(1), None);
            c.lowpass.store(35f32.to_bits(), Ordering::Relaxed);
            c.highpass.store(77f32.to_bits(), Ordering::Relaxed);
            let worst = src.by_ref().fold(0.0f32, |m, s| if s.is_finite() { m.max(s.abs()) } else { f32::INFINITY });
            assert!(worst < 1.0, "{f}: {worst}");
        }
    }

    // ------------------------------------------------------------------ the audio thread offline

    use super::super::triggers::{stock, Call, ShipView, Triggers};
    use elite_sim::drive::{Drive, ARRIVING_S, CHARGE_S, COOLDOWN_S, JUMP_CHARGE_S};
    use se_core::elite_sound::{Id, Mixer, Output};
    use std::collections::BTreeMap;

    /// A scenario as the game runs it, offline: the triggers and the mixer a frame at a time as `drive` calls them, each
    /// update applied to its voice as `apply` does (the same `VoiceCtl::write`, seeks and stops), and, with `audio`, each
    /// live voice's `VoiceSource` pulled for the frame at the output rate (what the player takes from it).
    struct Run {
        mixer: Mixer,
        triggers: Triggers,
        clips: HashMap<Id, Arc<Clip>>,
        audio: bool,
        /// Voice -> (emitter, source) for every playing recording (heard or virtual), the ones built (heard) and how many
        /// builds there were.
        playing: HashMap<u32, (u32, Id)>,
        built: std::collections::HashSet<u32>,
        builds: usize,
        /// Starts by source, and every start with its time.
        started: HashMap<Id, usize>,
        starts: Vec<(f64, VoiceUpdate)>,
        /// Each built voice's start and its latest update (a benchmark's snapshot).
        last: HashMap<u32, (VoiceUpdate, VoiceUpdate)>,
        sources: HashMap<u32, (VoiceSource, Arc<VoiceCtl>, f64)>,
        meter: Arc<Meter>,
        worst: f32,
        samples: u64,
        simulated: f64,
        cpu: std::time::Duration,
        most: usize,
        /// The buses' compressors' inputs, when a check watches them (`no_bus_compressor_would_act`).
        taps: Option<Taps>,
    }

    /// The compressors on the shipped sounds' buses (not built: `se_core::elite_sound`'s docs) and their thresholds as the
    /// banks hold them at DynamicRange's default (T5d-2's `re/elite/work/sound/t5d2/bus_fx.json`: each `Wwise Compressor`
    /// in a bus slot the ship's events reach, its block's threshold plus its curve on parameter 0 at the default; bypassed
    /// slots left out: 13d02225's). The eight on 12-channel buses (25b1912f, 34b88e1e, 80c77eb0, 94f4d066, aa299971,
    /// b28d0893, b804e991, and 87de5a42, which no ship event reaches) read their bus's power averaged over 12 channels;
    /// here every one is read in stereo, on the safe side (T5g-2 section 5: 7.8 dB higher for a point source ahead).
    const COMPRESSORS: &[(u32, u32, f64)] = &[
        (0x180710c3, 3, -18.0),
        (0x25b1912f, 3, -20.0),
        (0x34b88e1e, 1, -44.6),
        (0x368d104e, 0, -25.0),
        (0x722bdfeb, 0, -17.0),
        (0x80c77eb0, 3, -17.0),
        (0x8ad0fcab, 3, -17.0),
        (0x8f2aa457, 3, -18.0),
        (0x94f4d066, 3, -15.0),
        (0x964c90e0, 3, -13.22),
        (0x987ecc5e, 3, -25.0),
        (0xa1542164, 3, -13.22),
        (0xa9952ab4, 3, -20.0),
        (0xaa299971, 3, -35.5),
        (0xb28d0893, 3, -17.0),
        (0xb7a6077a, 0, -17.0),
        (0xb804e991, 3, -17.0),
        (0xbfd4c387, 3, -12.0),
        (0xd4fc004a, 1, -17.0),
    ];

    /// What reaches each compressor: the voices on its bus's way summed in stereo through their matrices, each less the
    /// gain it takes after the compressor's slot (`Mixer::gains_after_slots`), and the loudest mean square over a frame
    /// (10 ms) so far, against the threshold. (The effects after the slot shape a voice's output as well as scale it:
    /// only their output levels are taken back out, so a band they cut reads low here.)
    struct Taps {
        sums: Vec<Vec<[f32; 2]>>,
        loudest: Vec<(f64, f64)>,
    }

    impl Taps {
        fn new() -> Taps {
            Taps { sums: vec![Vec::new(); COMPRESSORS.len()], loudest: vec![(f64::NEG_INFINITY, 0.0); COMPRESSORS.len()] }
        }

        /// A voice's frame (`ch` channels interleaved) added at each compressor on its way.
        fn add(&mut self, buf: &[f32], ch: usize, m: [[f32; 2]; 2], after: &[(Id, [f64; 4])]) {
            for (k, (bus, slot, _)) in COMPRESSORS.iter().enumerate() {
                let Some((_, g)) = after.iter().find(|(b, _)| b.0 == *bus) else { continue };
                let undo = 10f64.powf(-g[*slot as usize] / 20.0) as f32;
                let sum = &mut self.sums[k];
                let n = buf.len() / ch;
                if sum.len() < n {
                    sum.resize(n, [0.0; 2]);
                }
                for (f, s) in sum.iter_mut().zip(buf.chunks_exact(ch)) {
                    let r = if ch > 1 { s[1] } else { 0.0 };
                    f[0] += (s[0] * m[0][0] + r * m[1][0]) * undo;
                    f[1] += (s[0] * m[0][1] + r * m[1][1]) * undo;
                }
            }
        }

        /// The frame's sums read and cleared.
        fn end_frame(&mut self, at: f64) {
            for (sum, loudest) in self.sums.iter_mut().zip(&mut self.loudest) {
                if !sum.is_empty() {
                    let ms = sum.iter().map(|f| f64::from(f[0]).powi(2) + f64::from(f[1]).powi(2)).sum::<f64>() / (2 * sum.len()) as f64;
                    let db = 10.0 * ms.max(1e-30).log10();
                    if db > loudest.0 {
                        *loudest = (db, at);
                    }
                }
                sum.clear();
            }
        }
    }

    impl Run {
        fn new(audio: bool) -> Run {
            let l = super::super::load::load().unwrap_or_else(|(f, e)| panic!("{f}: {e}"));
            Run {
                mixer: Mixer::new(l.bank.clone(), 1),
                triggers: Triggers::default(),
                clips: l.clips,
                audio,
                playing: HashMap::new(),
                built: Default::default(),
                builds: 0,
                started: HashMap::new(),
                starts: Vec::new(),
                last: HashMap::new(),
                sources: HashMap::new(),
                meter: Arc::default(),
                worst: 0.0,
                samples: 0,
                simulated: 0.0,
                cpu: std::time::Duration::ZERO,
                most: 0,
                taps: None,
            }
        }

        fn frame(&mut self, v: &ShipView) {
            let mut calls = Vec::new();
            self.triggers.step(v, &mut calls);
            for c in &calls {
                match c {
                    Call::Post(n, e) => {
                        let _ = self.mixer.post(n, e.id());
                    }
                    Call::State(g, s) => self.mixer.set_state(g, s),
                    Call::Gp(n, x) => self.mixer.set_game_parameter(n, *x, None),
                }
            }
            for u in self.mixer.step(v.dt as f32).to_vec() {
                let finite = [u.gain, u.pitch, u.lowpass, u.highpass].iter().all(|x| x.is_finite());
                assert!(finite || u.stop, "an update not finite at {:.2} s: {u:?}", self.simulated);
                let Output::Audio { source } = u.output else { continue };
                let Some(clip) = self.clips.get(&source).cloned() else { continue };
                // (As `apply`: a voice gone virtual is dropped, built again from its next start.)
                if u.stop || u.virtual_voice {
                    if u.stop {
                        self.playing.remove(&u.voice);
                    } else {
                        self.playing.insert(u.voice, (u.emitter, source));
                    }
                    if self.built.remove(&u.voice) {
                        self.sources.remove(&u.voice);
                    }
                    self.last.remove(&u.voice);
                    continue;
                }
                if u.start.is_some() {
                    self.playing.insert(u.voice, (u.emitter, source));
                    self.built.insert(u.voice);
                    self.builds += 1;
                    *self.started.entry(source).or_default() += 1;
                    self.starts.push((self.simulated, u.clone()));
                    self.last.insert(u.voice, (u.clone(), u.clone()));
                    if self.audio {
                        let (frame, plays, region) = start_of(&u, clip.rate).unwrap();
                        let ctl = Arc::new(VoiceCtl::new(&u, 1.0, self.meter.clone()));
                        self.sources.insert(u.voice, (VoiceSource::new(clip, ctl.clone(), frame, plays, region), ctl, 0.0));
                    }
                    continue;
                }
                self.playing.insert(u.voice, (u.emitter, source));
                if let Some((_, now)) = self.last.get_mut(&u.voice) {
                    now.clone_from(&u);
                }
                if let Some((_, ctl, _)) = self.sources.get(&u.voice) {
                    if let Some(to) = u.seek_to.filter(|t| t.is_finite()) {
                        ctl.seek.store((f64::from(to.max(0.0)) * f64::from(clip.rate)) as u64, Ordering::Relaxed);
                    }
                    ctl.write(&u, 1.0);
                    ctl.fx.write(&u.fx);
                }
            }
            self.most = self.most.max(self.playing.len());
            if self.audio {
                let t = std::time::Instant::now();
                let mut ended = Vec::new();
                let mut tap = Vec::new();
                for (id, (src, ctl, carry)) in &mut self.sources {
                    let want = f64::from(OUTPUT_RATE) * v.dt + *carry;
                    let frames = want.floor();
                    *carry = want - frames;
                    tap.clear();
                    for _ in 0..frames as u64 * u64::from(src.clip.channels) {
                        match src.next() {
                            Some(s) => {
                                assert!(s.is_finite() && s.abs() <= 4.0, "voice {id} {}: {s}", src.clip.file);
                                self.worst = self.worst.max(s.abs());
                                self.samples += 1;
                                tap.push(s);
                            }
                            None => {
                                ended.push(*id);
                                break;
                            }
                        }
                    }
                    if let Some(t) = &mut self.taps {
                        t.add(&tap, usize::from(src.clip.channels.max(1)), ctl.matrix, &self.mixer.gains_after_slots(*id));
                    }
                }
                if let Some(t) = &mut self.taps {
                    t.end_frame(self.simulated);
                }
                for id in ended {
                    self.sources.remove(&id);
                }
                self.cpu += t.elapsed();
            }
            self.simulated += v.dt;
        }

        fn hold(&mut self, v: &ShipView, seconds: f64) {
            for _ in 0..(seconds / v.dt).round() as usize {
                self.frame(v);
            }
        }

        /// A drive state that changes over its seconds, a frame at a time.
        fn ramp(&mut self, v: &ShipView, seconds: f64, at: impl Fn(f64) -> Drive) {
            let mut w = v.clone();
            for i in 0..(seconds / v.dt).round() as usize {
                w.drive = at(i as f64 * v.dt);
                self.frame(&w);
            }
        }

        /// The playing recordings by emitter and source, sorted.
        fn set(&self) -> Vec<(u32, Id)> {
            let mut s: Vec<_> = self.playing.values().copied().collect();
            s.sort_by_key(|(e, i)| (*e, i.0));
            s
        }

        fn flying() -> ShipView {
            let mut v = ShipView::at_rest();
            v.dt = 0.01;
            v.gear_down = false;
            v.motion = stock(2.0, [0.0, 0.0, -100.0]);
            v
        }

        /// Charge, supercruise for `cruise` seconds, drop and wait out the drive's cooldown and `after` seconds more.
        fn supercruise_trip(&mut self, cruise: f64, after: f64) {
            let v = Run::flying();
            self.ramp(&v, CHARGE_S, Drive::Charging);
            let mut sc = v.clone();
            sc.mode = 2;
            sc.drive = Drive::Supercruise;
            self.hold(&sc, cruise);
            self.drop_out(after);
        }

        fn drop_out(&mut self, after: f64) {
            let v = Run::flying();
            self.ramp(&v, ARRIVING_S, |t| Drive::Arriving(ARRIVING_S - t));
            self.ramp(&v, COOLDOWN_S, |t| Drive::Cooldown(COOLDOWN_S - t));
            self.hold(&v, after);
        }

        /// A hyperspace jump from normal space: the charge, the tunnel, arriving in supercruise, then the drop.
        fn jump_trip(&mut self, after: f64) {
            let v = Run::flying();
            self.ramp(&v, JUMP_CHARGE_S, Drive::JumpCharging);
            let mut tunnel = v.clone();
            tunnel.mode = 3;
            tunnel.drive = Drive::Tunnel;
            self.hold(&tunnel, 6.0);
            let mut arrived = v.clone();
            arrived.mode = 2;
            self.ramp(&arrived, 10.0, |t| Drive::Arrived(10.0 - t));
            let mut sc = arrived;
            sc.drive = Drive::Supercruise;
            self.hold(&sc, 5.0);
            self.drop_out(after);
        }
    }

    /// The supercruise entry and drop the reviewer's recording went silent after (the cockpit's cooldown blips at speed
    /// 0.25 with low-pass 35 and high-pass 77, the drive's layers at 0.2-0.6, ~90 voices), run through the very sources
    /// the audio thread runs, by the parameters the mixer gives them frame by frame, at their real rates: no panic, every
    /// sample finite and within 4, and far less audio-thread time than the seconds simulated.
    #[test]
    #[ignore]
    fn a_drop_plays_through_the_audio_threads_sources() {
        let mut r = Run::new(true);
        r.hold(&Run::flying(), 5.0);
        r.supercruise_trip(20.0, 15.0);
        let share = r.cpu.as_secs_f64() / r.simulated;
        eprintln!(
            "{:.0} s simulated, {} samples, worst {:.3}, most {} voices, audio-thread share {:.3} ({:.2} s)",
            r.simulated,
            r.samples,
            r.worst,
            r.most,
            share,
            r.cpu.as_secs_f64()
        );
        assert_eq!(r.meter.bad_blocks.load(Ordering::Relaxed), 0);
        assert!(share < 0.25, "the sources alone take {share:.2} of real time");
    }

    /// Supercruise round trips and a hyperspace jump end with the same objects playing on the same emitters as the first
    /// round trip did (the transition handles' ends: `audio_mode_transitions`), and the voices never pile up. (By
    /// object, the Play actions' targets: the recordings under a random container change with its picks.)
    ///
    /// Two silent objects do grow, as Elite's own banks make them: `boost_origin_internal` (posted by
    /// `ShipOrigin_b_sidewinder_internal`, each time the drive loops start) plays the continuous switch container
    /// `1223e7e9` (on switch BOOST: Idle plays nothing) that no event of the banks stops, one more per return to normal
    /// space; and `Front_Right_SharedCockpit_e` PLAYS the container `06a62f20` (a switch with nothing on its default)
    /// where its `_b` does too and the other positions' `_e` stop theirs, two more per change of the shared set. Their
    /// voices are bounded by the playback limit above them (`0890e5f7`: 3) and the boost's warm-up is checked below.
    #[test]
    #[ignore]
    fn round_trips_leave_the_same_voices_playing() {
        let steady = |v: Vec<(u32, Id)>| -> Vec<(u32, Id)> { v.into_iter().filter(|x| !GROWING.contains(x)).collect() };
        let mut r = Run::new(false);
        r.hold(&Run::flying(), 5.0);
        let (before, before_voices) = (r.mixer.playing_objects(), r.set().len());
        r.supercruise_trip(10.0, 5.0);
        let (first, first_voices) = (steady(r.mixer.playing_objects()), r.set().len());
        let diff = |a: &[(u32, Id)], b: &[(u32, Id)]| -> Vec<(u32, Id)> { b.iter().filter(|x| !a.contains(x)).copied().collect() };
        eprintln!("before the first trip {} objects ({before_voices} voices), after it {} ({first_voices} voices): new {:?}", before.len(), first.len(), diff(&before, &first));
        let mut most = 0;
        for k in 2..=20 {
            r.supercruise_trip(10.0, 5.0);
            let now = steady(r.mixer.playing_objects());
            assert_eq!(now, first, "after supercruise trip {k}: new {:?}, gone {:?}", diff(&first, &now), diff(&now, &first));
            most = most.max(r.set().len());
        }
        r.jump_trip(5.0);
        let now = steady(r.mixer.playing_objects());
        assert_eq!(now, first, "after a hyperspace jump and the drop: new {:?}, gone {:?}", diff(&first, &now), diff(&now, &first));
        // (A random sequence's gaps make the count at an instant differ by a few; the pile-up was 10 to 88 in one trip.)
        assert!(most <= first_voices + 8, "voices pile up: {first_voices} after the first trip, {most} later");
        // The boost after 21 returns starts as many voices as after one (the grown containers add nothing heard).
        let boost_starts = |r: &mut Run| {
            let before: usize = r.started.values().sum();
            let mut v = Run::flying();
            v.boost = 1;
            r.hold(&v, 1.0);
            r.started.values().sum::<usize>() - before
        };
        let late = boost_starts(&mut r);
        let mut fresh = Run::new(false);
        fresh.hold(&Run::flying(), 5.0);
        fresh.supercruise_trip(10.0, 5.0);
        let early = boost_starts(&mut fresh);
        eprintln!("after 20 trips and a jump: {} voices (at most {most} between trips); the boost's warm-up starts {late} voices (after one trip {early})", r.set().len());
        assert!(late <= early + 3, "the boost grew: {early} voices after one trip, {late} after 21");
    }

    /// Every sample rate and channel count among the shipped recordings, 10 s each through a source whose speed (0.05 to
    /// 8, the player's range clamped), filter values (0 to 100, jumps and ramps) and silence change as fast as the game
    /// writes them, loop regions at their edges and starts at the end: no panic, every sample finite and within 4. And
    /// the values the game hands Bevy: volume and speed not finite are dropped, never passed on.
    #[test]
    #[ignore]
    fn sources_survive_any_values() {
        let l = super::super::load::load().unwrap_or_else(|(f, e)| panic!("{f}: {e}"));
        let mut kinds: BTreeMap<(u32, u16), Arc<Clip>> = BTreeMap::new();
        for c in l.clips.values() {
            kinds.entry((c.rate, c.channels)).or_insert_with(|| c.clone());
        }
        let mut seed = 11u64;
        let mut rnd = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        for ((rate, ch), clip) in &kinds {
            let n = clip.frames;
            for (start, region) in [(0, None), (n - 1, None), (0, Some((0, n))), (0, Some((n - 1, n))), (0, Some((n / 2, n / 2 + 1))), (n - 1, Some((0, 1)))] {
                let c = ctl();
                let mut src = VoiceSource::new(clip.clone(), c.clone(), start, None, region);
                let (mut lp, mut hp) = (0.0f32, 0.0f32);
                let mut made = 0u64;
                while made < u64::from(*rate) * 10 * u64::from(*ch) {
                    // A jump, a ramp step or an edge value, every 128 to 800 frames.
                    let pick = rnd();
                    lp = if pick < 0.2 {
                        [0.0, 0.1, 0.10001, 30.0, 100.0][(rnd() * 5.0) as usize % 5]
                    } else if pick < 0.6 {
                        (lp + (rnd() - 0.5) * 10.0).clamp(0.0, 100.0)
                    } else {
                        rnd() * 100.0
                    };
                    hp = if rnd() < 0.3 { rnd() * 100.0 } else { (hp + (rnd() - 0.5) * 10.0).clamp(0.0, 100.0) };
                    c.lowpass.store(lp.to_bits(), Ordering::Relaxed);
                    c.highpass.store(hp.to_bits(), Ordering::Relaxed);
                    c.speed.store(finite_speed(0.05 + rnd() * 7.95).to_bits(), Ordering::Relaxed);
                    for _ in 0..(128 + (rnd() * 672.0) as u64) * u64::from(*ch) {
                        let s = src.next().expect("a looping source never ends");
                        assert!(s.is_finite() && s.abs() <= 4.0, "{} ({rate} Hz, {ch} ch, start {start}, region {region:?}): {s}", clip.file);
                        made += 1;
                    }
                }
            }
            eprintln!("{rate} Hz {ch} ch ({}): 6 loop and start cases, 10 s each", clip.file);
        }
        // What `apply` hands Bevy and the sources.
        let update = |gain: f32, pitch: f32, lowpass: f32, highpass: f32| VoiceUpdate {
            voice: 1,
            output: Output::Audio { source: Id(1) },
            emitter: 0,
            position: None,
            place: None,
            matrix: MONO_BOTH,
            media_gain: 1.0,
            start: None,
            stop: false,
            seek_to: None,
            gain,
            volume_db: 0.0,
            fade: 1.0,
            pitch,
            pitch_cents: 0.0,
            lowpass,
            highpass,
            lowpass_hz: None,
            highpass_hz: None,
            virtual_voice: false,
            fx: Vec::new(),
        };
        let mut u = update(f32::NAN, f32::INFINITY, f32::NAN, 50.0);
        assert_eq!(volume(&u, 1.0), None);
        u.gain = f32::NEG_INFINITY;
        assert_eq!(volume(&u, 1.0), None);
        u.gain = -1.0;
        assert_eq!(volume(&u, 1.0), Some(0.0));
        assert!(!speed(&u).is_finite());
        let c = VoiceCtl::new(&update(1.0, 1.0, 0.0, 0.0), 1.0, Arc::default());
        assert!(!c.write(&u, 1.0), "a value not finite is reported");
        assert_eq!(f32::from_bits(c.lowpass.load(Ordering::Relaxed)), 0.0, "and dropped: the last one stays");
        assert_eq!(f32::from_bits(c.highpass.load(Ordering::Relaxed)), 50.0);
        assert_eq!(f32::from_bits(c.speed.load(Ordering::Relaxed)), 1.0);
    }

    /// A batch handed to the output as `hand_over` hands it.
    fn send_batch(tx: &Sender<BatchMsg>, batch: Vec<Ready>) {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        tx.send((SEQ.fetch_add(1, Ordering::Relaxed), batch)).unwrap();
    }

    /// A mono 16-bit WAV clip of the given samples at 48 kHz.
    fn wav(samples: &[i16]) -> Clip {
        wav_of(samples, 1)
    }

    /// A 16-bit WAV clip at 48 kHz of `channels` channels, the samples interleaved.
    fn wav_of(samples: &[i16], channels: u16) -> Clip {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&[1, 0]);
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&48000u32.to_le_bytes());
        b.extend_from_slice(&(96000 * u32::from(channels)).to_le_bytes());
        b.extend_from_slice(&(2 * channels).to_le_bytes());
        b.extend_from_slice(&[16, 0]);
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        let (source, _) = crate::mods::check_wav(b).unwrap();
        Clip { source, file: "click.wav".into(), frames: (samples.len() / usize::from(channels)) as u64, channels, rate: 48000 }
    }

    /// Two voices built apart (as two worker threads build them) and handed over as one batch while the output is already
    /// playing start at the same output frame, the next block's first: a click of +0.25 in one and +0.5 in the other add
    /// up to 0.75 in a single frame, and nothing of either comes before it.
    #[test]
    fn a_batch_starts_at_one_sample() {
        let (tx, rx) = channel();
        let mut out = OutputSource::new(rx, Arc::new(Meter::new()), Arc::default());
        let early: Vec<f32> = out.by_ref().take(2 * 333).collect();
        assert!(early.iter().all(|s| *s == 0.0), "silent with no voice");
        let wait = (BLOCK - 333 % BLOCK) % BLOCK;
        let early: Vec<f32> = out.by_ref().take(2 * wait).collect();
        assert!(early.iter().all(|s| *s == 0.0), "the rest of the block already mixed");
        let click = |v: i16| {
            let c = Arc::new(wav(&[v, 0, 0, 0, 0, 0, 0, 0]));
            let ctl = ctl();
            Ready { src: VoiceSource::new(c, ctl.clone(), 0, Some(1), None), ctl }
        };
        let batch = std::thread::scope(|s| {
            let a = s.spawn(|| click(8192));
            let b = s.spawn(|| click(16384));
            vec![a.join().unwrap(), b.join().unwrap()]
        });
        send_batch(&tx, batch);
        let after: Vec<f32> = out.by_ref().take(2 * 20).collect();
        let left: Vec<f32> = after.iter().step_by(2).copied().collect();
        assert_eq!(left[0], 0.75, "both clicks in the batch's first frame: {left:?}");
        assert!(left[1..].iter().all(|s| *s == 0.0), "{left:?}");
        assert_eq!(after[1], 0.75, "a mono voice on both channels at full level");
    }

    /// A stop posted in the same mixer step as starts ends its voice where that step's batch begins, however long the
    /// batch takes to build: a loop stopped and another started in one step (a supercruise entry's kind) leave no zero
    /// frame between them, the old one sounding up to the frame the new one starts. Stopped at once, as before T5d-4, the
    /// same pair leaves a hole as long as the build (shown too). On the game side a step's stops take its batch, or end at
    /// once when the step has no starts.
    #[test]
    fn a_stop_meets_its_steps_starts() {
        let dc = |v: i16| Arc::new(wav(&vec![v; 4800]));
        let looped = |clip: Arc<Clip>| {
            let ctl = ctl();
            Ready { src: VoiceSource::new(clip, ctl.clone(), 0, None, None), ctl }
        };
        for with_batch in [true, false] {
            let (tx, rx) = channel();
            let mut out = OutputSource::new(rx, Arc::new(Meter::new()), Arc::default());
            let a = looped(dc(8192));
            let a_ctl = a.ctl.clone();
            tx.send((1, vec![a])).unwrap();
            let mut got: Vec<f32> = out.by_ref().take(2 * 1000).collect();
            // The step: the old loop stops, the new one's start goes to be built as batch 2, ready 600 frames (12.5 ms) on.
            if with_batch {
                a_ctl.stop_with.store(2, Ordering::Relaxed);
            } else {
                a_ctl.stop.store(true, Ordering::Relaxed);
            }
            got.extend(out.by_ref().take(2 * 600));
            tx.send((2, vec![looped(dc(16384))])).unwrap();
            got.extend(out.by_ref().take(2 * 1000));
            let left: Vec<f32> = got.iter().step_by(2).copied().collect();
            let holes = left.iter().filter(|s| **s == 0.0).count();
            let first_new = left.iter().position(|s| *s == 0.5).unwrap();
            if with_batch {
                assert_eq!(holes, 0, "no frame between the stop and the starts is silent");
                assert!(left[..first_new].iter().all(|s| *s == 0.25) && left[first_new..].iter().all(|s| *s == 0.5), "the old voice up to the frame the new one starts, never both");
                assert_eq!(first_new % BLOCK, 0, "at a block's first frame");
            } else {
                assert!(holes >= 600 - BLOCK, "stopped at once: a hole of {holes} frames");
            }
            eprintln!("{}: {holes} silent frames, the new voice from frame {first_new}", if with_batch { "stopped with the batch" } else { "stopped at once" });
        }
        // The game side.
        let clip = dc(1);
        for with in [Some(5), None] {
            let (mut v, ctl) = stopped_voice(&clip);
            assert!(!ctl.stop.load(Ordering::Relaxed) && ctl.stop_with.load(Ordering::Relaxed) == 0, "nothing ends before the step's starts are known");
            v.settle_stops(with);
            assert_eq!((ctl.stop.load(Ordering::Relaxed), ctl.stop_with.load(Ordering::Relaxed)), (with.is_none(), with.unwrap_or(0)));
        }
    }

    /// Voice 7's start (a loop of source 1) or its stop, centred, at level 1.
    fn plain_update(stop: bool) -> VoiceUpdate {
        VoiceUpdate {
            voice: 7,
            output: Output::Audio { source: Id(1) },
            emitter: 0,
            position: None,
            place: None,
            matrix: MONO_BOTH,
            media_gain: 1.0,
            start: (!stop).then_some(se_core::elite_sound::VoiceStart { offset: 0.0, looping: true, loop_count: None, loop_region: None }),
            stop,
            seek_to: None,
            gain: 1.0,
            volume_db: 0.0,
            fade: 1.0,
            pitch: 1.0,
            pitch_cents: 0.0,
            lowpass: 0.0,
            highpass: 0.0,
            lowpass_hz: None,
            highpass_hz: None,
            virtual_voice: false,
            fx: Vec::new(),
        }
    }

    /// The game side with voice 7 started and then stopped in the next mixer step (its stop not yet settled), and its
    /// controls.
    fn stopped_voice(clip: &Arc<Clip>) -> (Voices, Arc<VoiceCtl>) {
        let mut v = Voices::default();
        v.apply(&plain_update(false), Some(clip), 1.0, &mut |_| {});
        let ctl = v.playing[&7].ctl.clone();
        v.queued.clear();
        v.apply(&plain_update(true), Some(clip), 1.0, &mut |_| {});
        (v, ctl)
    }

    /// The last review's finding: a voice stopped with a batch that is not handed over yet waits for it in `waiting`, so
    /// leaving the seat meanwhile (`stop_all`) stops it too (before, it was in neither list and sounded on until the output
    /// went, 0.5 s later); a batch that never finishes building holds the stop at most `STOP_WAIT_S`; a batch handed over
    /// with no output to take it stops its waiting voices at once.
    #[test]
    fn stops_waiting_for_a_batch_are_never_lost() {
        let clip = Arc::new(wav(&[1; 480]));
        // Leaving before the batch is handed over.
        let (mut v, ctl) = stopped_voice(&clip);
        v.settle_stops(Some(3));
        assert_eq!(v.waiting.len(), 1);
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let world = World::new();
        v.stop_all(&mut Commands::new(&mut queue, &world));
        assert!(ctl.stop.load(Ordering::Relaxed) && v.waiting.is_empty(), "stop_all reaches the voice waiting for its batch");
        // A build that never returns: the stop waits its bound, then acts.
        let (mut v, ctl) = stopped_voice(&clip);
        let (hold, never) = std::sync::mpsc::channel::<()>();
        v.building.push_back(Batch { seq: 3, parts: vec![std::thread::spawn(move || { let _ = never.recv(); Vec::new() })], asked: std::time::Instant::now(), count: 1 });
        v.settle_stops(Some(3));
        let t0 = std::time::Instant::now();
        assert!(v.hand_over().is_empty(), "the batch is still building");
        v.expire_stops(t0);
        assert!(!ctl.stop.load(Ordering::Relaxed), "within its bound the stop still waits for the batch");
        v.expire_stops(t0 + std::time::Duration::from_secs_f64(STOP_WAIT_S + 0.01));
        assert!(ctl.stop.load(Ordering::Relaxed) && v.waiting.is_empty(), "past it the voice stops at once");
        drop(hold);
        // A batch handed over with no output: its waiting voices stop at once.
        let (mut v, ctl) = stopped_voice(&clip);
        v.building.push_back(Batch { seq: 4, parts: vec![std::thread::spawn(Vec::new)], asked: std::time::Instant::now(), count: 0 });
        v.settle_stops(Some(4));
        while v.hand_over().is_empty() {
            std::thread::yield_now();
        }
        assert!(ctl.stop.load(Ordering::Relaxed) && v.waiting.is_empty());
    }

    /// A voice through `p`, of a steady level 1 (its matrix the one at the listener), placed at `at` when given.
    fn voice_through(clip: &Arc<Clip>, p: Placement, at: Option<[f32; 3]>, media_gain: f32) -> Ready {
        let mut c = VoiceCtl::idle(Arc::default());
        c.level.store(1f32.to_bits(), Ordering::Relaxed);
        c.matrix = p.listener_matrix(usize::from(clip.channels));
        c.media_gain = media_gain;
        if let Some(at) = at {
            c.placement = Some(Arc::new(p));
            c.place.write(at);
        }
        let ctl = Arc::new(c);
        Ready { src: VoiceSource::new(clip.clone(), ctl.clone(), 0, None, None), ctl }
    }

    /// The first `frames` frames the output makes of one voice.
    fn mixed(r: Ready, frames: usize) -> Vec<f32> {
        let (tx, rx) = channel();
        let o = OutputSource::new(rx, Arc::new(Meter::new()), Arc::default());
        send_batch(&tx, vec![r]);
        o.take(2 * frames).collect()
    }

    /// A placed voice of a steady level as the output mixes it, its emitter moved each block (as the game writes it each
    /// frame) from hard left to hard right round the front, on a stereo route (T5g's planar panner): fully left at -90
    /// degrees, 0.7071 / 0.7071 straight ahead, fully right at +90, and no gain ever steps by more than 1 % from one frame
    /// to the next (each is ramped over its block). Straight ahead, a placed mono voice gives exactly the samples a voice
    /// not positioned in space gives (2D, direct assignment: 0.7071 on both).
    #[test]
    fn a_moving_emitter_pans_smoothly() {
        use se_core::elite_sound::Route;
        let level = 0.25f32;
        let clip = Arc::new(wav(&vec![(level * 32768.0) as i16; 48000]));
        let spatial = || Placement::routed(Route::default(), true, None);
        let (tx, rx) = channel();
        let mut out = OutputSource::new(rx, Arc::new(Meter::new()), Arc::default());
        let r = voice_through(&clip, spatial(), Some([-3.0, 0.0, 0.0]), 1.0);
        let moving = r.ctl.clone();
        send_batch(&tx, vec![r]);
        let mut got: Vec<f32> = Vec::new();
        let blocks = 60;
        for k in 0..=blocks {
            // Round the front at 3 m: from -90 to +90 degrees.
            let a = (-90.0 + 180.0 * k as f32 / blocks as f32).to_radians();
            moving.place.write([3.0 * a.sin(), 0.0, -3.0 * a.cos()]);
            got.extend(out.by_ref().take(2 * BLOCK));
        }
        got.extend(out.by_ref().take(4 * BLOCK));
        let gains: Vec<[f32; 2]> = got.chunks_exact(2).map(|f| [f[0] / level, f[1] / level]).collect();
        let first = gains[BLOCK];
        let last = *gains.last().unwrap();
        assert!((first[0] - 1.0).abs() < 1e-3 && first[1].abs() < 1e-3, "hard left first: {first:?}");
        assert!(last[0].abs() < 1e-3 && (last[1] - 1.0).abs() < 1e-3, "hard right last: {last:?}");
        let mid = gains[gains.len() / 2 - BLOCK];
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((mid[0] - h).abs() < 0.1 && (mid[1] - h).abs() < 0.1, "near the middle: {mid:?}");
        let step = gains.windows(2).map(|w| (w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs())).fold(0.0f32, f32::max);
        assert!(step <= 0.01, "a gain stepped by {step} in one frame");
        // Straight ahead: the same bits as a voice not positioned in space.
        let bits = |v: Vec<f32>| v.into_iter().map(f32::to_bits).collect::<Vec<u32>>();
        let ahead = bits(mixed(voice_through(&clip, spatial(), Some([0.0, 0.0, -2.0]), 1.0), 4 * BLOCK));
        let flat = bits(mixed(voice_through(&clip, Placement::routed(Route::default(), false, None), None, 1.0), 4 * BLOCK));
        assert_eq!(ahead, flat);
        assert_eq!(f32::from_bits(flat[2 * BLOCK]), level * 0.70710677);
    }

    /// A stereo voice with a place but no spatialisation keeps both its channels (the T5d-5 review: a placed voice's matrix
    /// was a mono one whatever its channels, so a stereo one lost its right channel), on either route.
    #[test]
    fn a_placed_stereo_voice_keeps_both_channels() {
        use se_core::elite_sound::{Layout, Route};
        let (l, r) = (0.25f32, -0.5f32);
        let frames: Vec<i16> = (0..4800).flat_map(|_| [(l * 32768.0) as i16, (r * 32768.0) as i16]).collect();
        let clip = Arc::new(wav_of(&frames, 2));
        for layout in [Layout::Stereo, Layout::Height714] {
            let p = Placement::routed(Route { layout, ..Route::default() }, false, None);
            let got = mixed(voice_through(&clip, p, Some([-4.0, 0.0, -1.0]), 1.0), 4 * BLOCK);
            assert!(got.chunks_exact(2).all(|f| f[0] == l && f[1] == r), "{layout:?}: {:?}", &got[..4]);
        }
    }

    /// Only the frame rate changing (the game writing a still emitter's place every block, every fourth, or once) changes
    /// nothing the output makes: the same bits, on a 7.1.4 route.
    #[test]
    fn the_frame_rate_changes_nothing() {
        use se_core::elite_sound::{Layout, Route};
        let clip = Arc::new(wav(&(0..9600).map(|i| ((f64::from(i) * 0.05).sin() * 8000.0) as i16).collect::<Vec<_>>()));
        let at = [2.0, -1.0, -3.0];
        let runs: Vec<Vec<u32>> = [1, 4, 1000]
            .iter()
            .map(|every| {
                let r = voice_through(&clip, Placement::routed(Route { layout: Layout::Height714, ..Route::default() }, true, None), Some(at), 1.0);
                let ctl = r.ctl.clone();
                let (tx, rx) = channel();
                let mut o = OutputSource::new(rx, Arc::new(Meter::new()), Arc::default());
                send_batch(&tx, vec![r]);
                let mut got = Vec::new();
                for k in 0..32 {
                    if k % every == 0 {
                        ctl.place.write(at);
                    }
                    got.extend(o.by_ref().take(2 * BLOCK).map(f32::to_bits));
                }
                got
            })
            .collect();
        assert!(runs.windows(2).all(|w| w[0] == w[1]));
        assert!(runs[0].iter().any(|b| *b != 0));
    }

    /// The recording's own gain (`akd_gain_2`) multiplies the samples before the voice's effects: with a Guitar Distortion
    /// on the voice, 1.414 x the recording goes into the distortion, which is not 1.414 x what the distortion makes of it.
    #[test]
    fn the_recordings_gain_comes_before_the_effects() {
        let tone: Vec<i16> = (0..4800).map(|i| ((std::f64::consts::TAU * 440.0 * f64::from(i) / 48000.0).sin() * 12000.0) as i16).collect();
        let clip = Arc::new(wav(&tone));
        let d = DistortionNow { pre: [EqNow::OFF; 3], post: [EqNow::OFF; 3], kind: filter::DISTORTION_OVERDRIVE, drive: 60.0, tone: 50.0, rectification: 0.0, level: 1.0, wet: 100.0 };
        let g = std::f32::consts::SQRT_2;
        let c = ctl();
        let mut c = Arc::into_inner(c).unwrap();
        c.media_gain = g;
        c.fx.write(&[FxNow::Distortion(d)]);
        let got: Vec<f32> = VoiceSource::new(clip, Arc::new(c), 0, Some(1), None).collect();
        let run = |pre: f32, post: f32| -> Vec<f32> {
            let mut p = Distortion::new(1, OUTPUT_RATE);
            p.set(&d);
            tone.iter().map(|s| {
                let mut f = [f32::from(*s) / 32768.0 * pre];
                p.frame(&mut f);
                f[0] * post
            }).collect()
        };
        let (before, after) = (run(g, 1.0), run(1.0, g));
        assert!(got.iter().zip(&before).all(|(a, b)| (a - b).abs() < 1e-6), "the gain goes into the distortion");
        assert!(got.iter().zip(&after).any(|(a, b)| (a - b).abs() > 1e-3), "and the order matters");
    }

    /// The block path against the frame-at-a-time path it replaced (kept here only): a supercruise entry's heard voices
    /// with their chains (`supercruise_entry`: filters, EQ bands, distortions, mono and stereo, loops) rendered for 4 s of
    /// audio both ways and mixed in the same order give the same samples (the largest difference is printed: float noise
    /// at most, from a voice's removal reordering the sum a few frames apart; it is 0 while no voice ends).
    #[test]
    #[ignore]
    fn blocks_sound_as_frames_did() {
        // The old path: a frame through the chain, a sample a channel (the code before T5d-4).
        fn old_frame(s: &mut VoiceSource, since_step: &mut u32, out: &mut Vec<f32>) -> bool {
            if s.over {
                return false;
            }
            if *since_step == 0 {
                s.step_controls();
            }
            *since_step = (*since_step + 1) % RAMP_FRAMES;
            s.advance();
            let k = s.phase as f32;
            s.level += s.level_step;
            let mut work: Vec<f32> = s.cur.iter().zip(&s.nxt).map(|(a, b)| (a + (b - a) * k) * s.ctl.media_gain).collect();
            for (c, x) in work.iter_mut().enumerate() {
                for f in &mut s.filters {
                    if let (Some(co), Some(sec)) = (&f.coefs, f.channels.get_mut(c)) {
                        *x = sec.run(co, *x);
                    }
                }
            }
            for stage in &mut s.stages[..s.stages_n] {
                match stage.now {
                    FxNow::Eq(_) => {
                        for (c, y) in work.iter_mut().enumerate() {
                            for b in &mut stage.bands {
                                if let (Some(co), Some(sec)) = (&b.coefs, b.channels.get_mut(c)) {
                                    *y = sec.run(co, *y);
                                }
                            }
                        }
                    }
                    FxNow::Distortion(_) => stage.dist.frame(&mut work),
                    FxNow::Off => {}
                }
            }
            out.clear();
            out.extend(work.iter().map(|y| if y.is_finite() && y.abs() <= filter::CHAIN_BOUND { y * s.level } else { 0.0 }));
            s.phase += f64::from(s.clip.rate) / f64::from(OUTPUT_RATE) * f64::from(s.speed.now);
            true
        }
        let set = supercruise_entry(80);
        let meter = Arc::new(Meter::new());
        let frames = 48000 * 4;
        let mut old: Vec<(VoiceSource, u32, bool)> = set.iter().map(|(c, s, u)| (built(c, s, u, &meter).src, 0, true)).collect();
        let mut want = vec![0.0f32; 2 * frames];
        let mut o = Vec::new();
        for f in want.chunks_exact_mut(2) {
            for (s, since, live) in &mut old {
                if *live && old_frame(s, since, &mut o) {
                    // (Through the voice's matrix, as `mix_through` does it.)
                    let m = s.ctl.matrix;
                    f[0] += o[0] * m[0][0];
                    f[1] += o[0] * m[0][1];
                    if let Some(r) = o.get(1) {
                        f[0] += r * m[1][0];
                        f[1] += r * m[1][1];
                    }
                } else {
                    *live = false;
                }
            }
        }
        let (tx, rx) = channel();
        let mut out = OutputSource::new(rx, meter.clone(), Arc::default());
        // (The output pops a batch from its end: reversed, it keeps the set's order, as the old mix above has it.)
        send_batch(&tx, set.iter().rev().map(|(c, s, u)| built(c, s, u, &meter)).collect());
        let got: Vec<f32> = out.by_ref().take(2 * frames).collect();
        let worst = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        let same = got.iter().zip(&want).filter(|(a, b)| a.to_bits() == b.to_bits()).count();
        let peak = want.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        eprintln!("{} voices, {frames} frames: {same} of {} samples the same bits, the largest difference {worst:e} (the mix's peak {peak:.3})", set.len(), got.len());
        assert!(worst <= peak * 1e-6, "the block path differs by {worst}");
    }

    /// A voice's effects run after its filters, each the exe's: a voice with one EQ band (a peak of -6 dB at 1 kHz) gives
    /// what that section gives the recording; a band that is off, or a table read half-written, changes nothing; a Guitar
    /// Distortion stage gives what `filter::Distortion` gives, and its values cross the table whole.
    #[test]
    fn a_voices_effects_shape_it() {
        let tone: Vec<i16> = (0..9600).map(|i| ((std::f64::consts::TAU * 1000.0 * f64::from(i) / 48000.0).sin() * 16000.0) as i16).collect();
        let clip = Arc::new(wav(&tone));
        let peak = EqNow { on: true, filter: Some(EqFilter::PeakingEq), frequency: 1000.0, gain_db: -6.0, q: 1.0 };
        let c = ctl();
        c.fx.write(&[FxNow::Eq([EqNow::OFF, peak, EqNow::OFF])]);
        let shaped: Vec<f32> = VoiceSource::new(clip.clone(), c, 0, Some(1), None).collect();
        let co = filter::eq_coefs(EqFilter::PeakingEq, 1000.0, -6.0, 1.0, OUTPUT_RATE).unwrap();
        let mut sec = Section::default();
        let expected: Vec<f32> = tone.iter().map(|s| sec.run(&co, f32::from(*s) / 32768.0)).collect();
        assert_eq!(shaped.len(), expected.len());
        assert!(shaped.iter().zip(&expected).all(|(a, b)| (a - b).abs() < 1e-6), "the band is the exe's section");
        let rms = |v: &[f32]| (v[4800..].iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / 4800.0).sqrt();
        let plain: Vec<f32> = VoiceSource::new(clip.clone(), ctl(), 0, Some(1), None).collect();
        let db = 20.0 * (rms(&shaped) / rms(&plain)).log10();
        assert!((db + 6.0).abs() < 0.1, "-6 dB at the peak: {db:.2}");
        let off = ctl();
        off.fx.write(&[FxNow::Eq([EqNow { on: false, ..peak }; 3]), FxNow::Off]);
        let untouched: Vec<f32> = VoiceSource::new(clip.clone(), off, 0, Some(1), None).collect();
        assert_eq!(untouched, plain);
        // A distortion: the voice's stage is the filter's processor, sample for sample.
        let d = DistortionNow { pre: [peak, EqNow::OFF, EqNow::OFF], post: [EqNow::OFF; 3], kind: filter::DISTORTION_OVERDRIVE, drive: 30.0, tone: 40.0, rectification: 20.0, level: 0.8, wet: 48.0 };
        let dc = ctl();
        dc.fx.write(&[FxNow::Distortion(d)]);
        let distorted: Vec<f32> = VoiceSource::new(clip, dc, 0, Some(1), None).collect();
        let mut p = Distortion::new(1, OUTPUT_RATE);
        p.set(&d);
        let want: Vec<f32> = tone.iter().map(|s| {
            let mut f = [f32::from(*s) / 32768.0];
            p.frame(&mut f);
            f[0]
        }).collect();
        assert!(distorted.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-6), "the stage is the processor");
        // (A reader that meets the table mid-write keeps what it had; a distortion's values cross it whole.)
        let t = FxTable::default();
        t.write(&[FxNow::Eq([peak; 3]), FxNow::Distortion(d)]);
        let mut seen = 0;
        let mut out = [FxNow::Off; MAX_STAGES];
        assert_eq!(t.read(&mut seen, &mut out), Some(2));
        assert_eq!((out[0], out[1]), (FxNow::Eq([peak; 3]), FxNow::Distortion(d)));
        assert_eq!(t.read(&mut seen, &mut out), None, "unchanged");
        t.seq.0.fetch_add(1, Ordering::AcqRel);
        assert_eq!(t.read(&mut seen, &mut out), None, "being written");
    }

    /// Every shipped sound's effects fit a voice's table, and the largest gain its chain can give a full-scale input (the
    /// positive gains of every EQ band and output level on its way, at their clamps where a curve sets them; a distortion's
    /// level times the larger of its dry path and twice its wet path's bands; the filters' overshoot as 6 dB) stays far
    /// under the chain's guard (`filter::CHAIN_BOUND`), so the guard cannot mute a legitimate boost.
    #[test]
    fn every_sounds_chain_fits() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/sounds.json");
        let bank = se_core::elite_sound::parse_sounds(&std::fs::read(path).unwrap()).unwrap();
        let most = bank.file.objects.keys().map(|id| bank.fx_stages(*id).len()).max().unwrap_or(0);
        use se_core::elite_sound::data::{Accumulation, EqBand, Effect};
        // The most a parameter reaches: its block value, or its curves' most (an exclusive curve's, or the block value plus
        // each additive curve's most), held to the plug-in's clamp.
        let reach = |e: &Effect, p: u32, base: f64, hi: f64| {
            let curves: Vec<_> = e.rtpc.iter().filter(|r| r.param_id == p).collect();
            let top = |r: &&se_core::elite_sound::data::Rtpc| r.points.iter().map(|q| q.y).fold(f64::MIN, f64::max);
            let v = match curves.iter().find(|r| r.accumulation == Accumulation::Exclusive) {
                Some(r) => top(r),
                None => base + curves.iter().map(top).sum::<f64>(),
            };
            if curves.is_empty() { v } else { v.min(hi) }
        };
        let band_db = |e: &Effect, b: &EqBand, p: u32, hi: f64| {
            let shaping = matches!(b.filter, EqFilter::PeakingEq | EqFilter::LowShelf | EqFilter::HiShelf) || e.rtpc.iter().any(|r| r.param_id == p);
            let on = b.on || e.rtpc.iter().any(|r| r.param_id == p + 4);
            if on && shaping { reach(e, p + 1, b.gain_db, hi).max(0.0) } else { 0.0 }
        };
        let mut worst = (0.0f64, se_core::elite_sound::Id(0));
        for id in bank.file.objects.keys() {
            let mut db = 6.0;
            for st in bank.fx_stages(*id) {
                let Some(e) = bank.file.effects.get(&st.effect) else { continue };
                if let Some(eq) = &e.eq {
                    db += eq.bands.iter().enumerate().map(|(k, b)| band_db(e, b, 5 * k as u32, 24.0)).sum::<f64>();
                    db += reach(e, 15, eq.output_level_db, 24.0).max(0.0);
                }
                if let Some(d) = &e.distortion {
                    let at = |k: usize| ((k / 3) * 30 + (k % 3) * 10) as u32;
                    let pre: f64 = d.pre_eq.iter().enumerate().map(|(k, b)| band_db(e, b, at(k), 48.0)).sum();
                    let post: f64 = d.post_eq.iter().enumerate().map(|(k, b)| band_db(e, b, at(k + 3), 48.0)).sum();
                    let wet = 6.0 + post + if d.distortion == filter::DISTORTION_NONE { pre } else { 0.0 };
                    db += wet.max(0.0) + reach(e, 64, d.output_level_db, 24.0).max(0.0);
                }
                if let Some(l) = &e.limiter {
                    db += reach(e, 4, l.output_level_db, 24.0).max(0.0);
                }
            }
            if db > worst.0 {
                worst = (db, *id);
            }
        }
        let bound = 20.0 * f64::from(filter::CHAIN_BOUND).log10();
        eprintln!("the most effects on a shipped sound's way: {most}; the largest chain gain {:.1} dB ({}), the guard at {bound:.0} dB", worst.0, worst.1);
        assert!(most <= MAX_STAGES, "{most} stages");
        assert!(worst.0 + 12.0 < bound, "{} can reach {:.1} dB", worst.1, worst.0);
    }

    /// The Guitar Distortions the data puts on buses (the GUI bus 368d104e, the Voice bus d4fc004a) are not linear, so per
    /// voice they would be wrong where they distort: but their wet share is 0 at their game parameters' defaults, and
    /// nothing the ship does sets those (a flight with every trigger: the boost, flight assist, supercruise trips, a jump),
    /// so where the mod plays them each is its output level and DC blocker alone, linear, and per voice is exact.
    #[test]
    #[ignore]
    fn every_effect_on_a_bus_is_linear_where_it_acts() {
        let mut r = Run::new(false);
        let mut v = Run::flying();
        r.hold(&v, 3.0);
        v.flight_assist = false;
        r.hold(&v, 1.0);
        v.flight_assist = true;
        v.boost = 2;
        r.hold(&v, 2.0);
        v.boost = 0;
        r.hold(&v, 1.0);
        r.supercruise_trip(5.0, 3.0);
        r.jump_trip(3.0);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/sounds.json");
        let bank = se_core::elite_sound::parse_sounds(&std::fs::read(path).unwrap()).unwrap();
        let mut checked = 0;
        for (sound, _) in bank.file.objects.iter().filter(|(_, o)| matches!(o, se_core::elite_sound::Object::Sound(_))) {
            for st in bank.fx_stages(*sound).iter().filter(|st| st.on_bus) {
                let Some(e) = bank.file.effects.get(&st.effect).filter(|e| e.distortion.is_some()) else { continue };
                let d = e.distortion.as_ref().unwrap();
                let mut wet = d.wet_dry;
                for c in e.rtpc.iter().filter(|c| c.param_id == se_core::elite_sound::GD_PARAM_WET) {
                    let def = bank.file.game_syncs.game_parameters.get(&c.game_parameter.id).map_or(0.0, |g| g.default);
                    let now = f64::from(r.mixer.game_parameter_id(c.game_parameter.id, None));
                    assert!((now - def).abs() < 1e-6, "{}: game parameter {} was set ({now}, default {def})", st.effect, c.game_parameter.id);
                    wet += se_core::elite_sound::curve::eval(&c.points, c.scaling, def);
                }
                assert!(wet.abs() < 1e-9, "the distortion {} on bus {} has a wet share of {wet} where the mod plays it", st.effect, st.holder);
                checked += 1;
            }
        }
        eprintln!("{checked} sound ways through a bus's distortion, each at a wet share of 0");
        assert!(checked > 0);
    }

    /// The two silent objects Elite's own banks grow (`round_trips_leave_the_same_voices_playing`): by emitter and object.
    const GROWING: [(u32, Id); 2] = [(0x100, Id(0x1223_e7e9)), (0x205, Id(0x06a6_2f20))];

    /// A long session (the review's finding): supercruise round trips with a boost after each, through the real triggers
    /// and mixer, and the voices built as the game builds them (heard ones only). After 50 and 500 trips the voices playing
    /// (heard or virtual), the voices built (heard) and a boost's builds stay what they were after the first trip, within a
    /// few voices, and the frame's cost (the triggers and the mixer, median over 2 s) stays bounded; the silent objects the
    /// banks grow (`GROWING`, four a trip) are printed: they cost no voice.
    #[test]
    #[ignore]
    fn a_long_session_stays_bounded() {
        let mut r = Run::new(false);
        let mut v = Run::flying();
        v.dt = 1.0 / 60.0;
        r.hold(&v, 5.0);
        let step_ms = |r: &mut Run, v: &ShipView, secs: f64| {
            let mut t = Vec::new();
            for _ in 0..(secs / v.dt).round() as usize {
                let t0 = std::time::Instant::now();
                r.frame(v);
                t.push(t0.elapsed().as_secs_f64() * 1000.0);
            }
            t.sort_by(f64::total_cmp);
            t[t.len() / 2]
        };
        // After the first trip: (voices playing, voices built, a boost's builds, the frame's median ms).
        let mut first: Option<(usize, usize, usize, f64)> = None;
        for trip in 1..=500 {
            r.supercruise_trip(2.0, 1.0);
            let builds = r.builds;
            let mut b = Run::flying();
            b.dt = v.dt;
            b.boost = 2;
            r.hold(&b, 0.5);
            let boost_builds = r.builds - builds;
            let boost_voices = r.playing.len();
            b.boost = 0;
            r.hold(&b, 0.5);
            if [1, 50, 500].contains(&trip) {
                let ms = step_ms(&mut r, &v, 2.0);
                let objects = r.mixer.playing_objects();
                let idle = objects.iter().filter(|o| GROWING.contains(o)).count();
                eprintln!(
                    "after {trip} trips: {} objects playing ({idle} of them the banks' growing silent ones), {} voices ({} built), a boost built {boost_builds} voices with {boost_voices} voices in all; frame (triggers + mixer) median {ms:.3} ms",
                    objects.len(),
                    r.playing.len(),
                    r.built.len()
                );
                let now = (r.playing.len(), r.built.len(), boost_builds, ms);
                match first {
                    None => first = Some(now),
                    Some(f) => {
                        assert!(now.0 <= f.0 + 8 && now.1 <= f.1 + 8, "after {trip} trips {} voices playing, {} built (after the first {}, {})", now.0, now.1, f.0, f.1);
                        assert!(now.2 <= f.2 + 3, "a boost builds {} voices after {trip} trips, {} after the first", now.2, f.2);
                        // (The banks' idle objects cost the mixer's walk a little each: T5d-4 measured 0.42 ms after one
                        // trip and 0.83 ms after 500. Bounded here: at most three times the first trip's, and under a
                        // quarter of a 60 Hz frame.)
                        assert!(now.3 <= 3.0 * f.3 && now.3 < 4.0, "the frame takes {:.3} ms after {trip} trips, {:.3} after the first", now.3, f.3);
                    }
                }
            }
        }
    }

    /// Decision 6 of T5d-7: the buses' compressors stay unbuilt while none would act. A whole flight offline, every voice
    /// through its matrix and its recording's gain (at rest, full throttle, the boost through its states, flight assist
    /// off and on, the gear's cues all at once on an emitter at the listener, which is louder than their places, a
    /// supercruise trip and a hyperspace jump): each compressor's input, read in stereo (the safe side for the 12-channel
    /// ones), against its threshold. Prints the table; fails if one would act.
    #[test]
    #[ignore]
    fn no_bus_compressor_would_act() {
        let mut r = Run::new(true);
        r.taps = Some(Taps::new());
        let mut v = Run::flying();
        v.throttle = 0.0;
        r.hold(&v, 2.0);
        v.throttle = 1.0;
        r.hold(&v, 3.0);
        for (state, secs) in [(1, 0.6), (2, 2.0), (3, 1.0), (0, 0.5)] {
            v.boost = state;
            r.hold(&v, secs);
        }
        for fa in [false, true] {
            v.flight_assist = fa;
            r.hold(&v, 1.0);
        }
        for name in [
            "SideWinder_Rear_Deploy_Motor01_b_landinggear",
            "SideWinder_Rear_Deploy_Motor02_b_landinggear",
            "SideWinder_Rear_Deploy_Door_Hinge_b_landinggear",
            "Sidewinder_Deploy_Motor01_b_landinggear",
            "DoorOpen_Rear_b_sidewinder_external",
            "LandingGearLower_b_sidewinder",
        ] {
            let _ = r.mixer.post(name, 0x1000);
        }
        r.hold(&v, 3.0);
        r.supercruise_trip(5.0, 2.0);
        r.jump_trip(2.0);
        let t = r.taps.take().unwrap();
        let mut acting = Vec::new();
        eprintln!("bus      slot threshold  loudest input (10 ms mean square, stereo)  at");
        for ((bus, slot, threshold), (db, at)) in COMPRESSORS.iter().zip(&t.loudest) {
            eprintln!("{bus:08x} {slot}    {threshold:+7.2}    {db:+7.2} dB ({:+.2} under)  {at:.2} s", threshold - db);
            if db > threshold {
                acting.push(format!("{bus:08x}"));
            }
        }
        assert!(acting.is_empty(), "compressors that would act: {acting:?}");
    }

    /// T5d-7's table: what each voice's matrix (Wwise's pan rules, at the listener, as every emitter but the legs' is) and
    /// its recording's own gain change in level against the player before them (a mono voice at 1.0 on both channels, a
    /// stereo one 1:1, no recording gain), for every Sound the shipped events reach, by class (route, channels, the
    /// recording's gain, panner): mono exactly, stereo from the recording's own samples (the first 30 s: the L / R
    /// correlation counts), as T5g-2's `classes.py` did with the studies' models.
    #[test]
    #[ignore]
    fn level_change_by_class() {
        use se_core::elite_sound::{Layout, Object, Panner};
        let l = super::super::load::load().unwrap_or_else(|(f, e)| panic!("{f}: {e}"));
        let bank = l.bank.clone();
        let mut mixer = Mixer::new(bank.clone(), 1);
        let mut todo: Vec<Id> = bank.file.ship_events.iter().chain(&bank.file.open_events).filter_map(|n| bank.event(n)).flat_map(|e| e.actions.iter().copied()).collect();
        let mut seen = std::collections::HashSet::new();
        let mut sounds = Vec::new();
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            match bank.object(id) {
                Some(Object::Action(a)) if a.kind == se_core::elite_sound::data::ActionKind::Play => todo.push(a.target),
                Some(Object::Sound(s)) if bank.file.media.contains_key(&s.source_id) => sounds.push((id, s.source_id)),
                Some(Object::RanSeqCntr(c)) => todo.extend(&c.children),
                Some(Object::LayerCntr(c)) => todo.extend(&c.children),
                Some(Object::SwitchCntr(c)) => todo.extend(c.children.iter().chain(c.switches.iter().flat_map(|s| s.nodes.iter()))),
                _ => {}
            }
        }
        let mut classes: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for (id, src) in &sounds {
            let media = &bank.file.media[src];
            let p = mixer.placement_of(*id);
            let ch = media.channels as usize;
            let g = f64::from(media.akd_gain_2.map_or(1.0, |g| g as f32));
            let m = p.listener_matrix(ch).map(|r| r.map(|x| f64::from(x) * g));
            let change = if ch == 1 {
                10.0 * ((m[0][0].powi(2) + m[0][1].powi(2)) / 2.0).log10()
            } else {
                let Some(clip) = l.clips.get(src) else { continue };
                let (mut old, mut new) = (0.0f64, 0.0f64);
                let mut dec = clip.source.decoder();
                for _ in 0..(clip.frames as usize).min(48000 * 30) {
                    let (Some(a), Some(b)) = (dec.next(), dec.next()) else { break };
                    let (a, b) = (f64::from(a), f64::from(b));
                    old += a * a + b * b;
                    new += (a * m[0][0] + b * m[1][0]).powi(2) + (a * m[0][1] + b * m[1][1]).powi(2);
                }
                if old <= 0.0 {
                    continue;
                }
                10.0 * (new / old).log10()
            };
            let r = p.route;
            let kind = if p.pan { "3D" } else if r.panner == Panner::BalanceFade { "2D balance-fade" } else { "2D direct" };
            let key = format!("{:5} {:6} akd {:6} {}", if r.layout == Layout::Height714 { "7.1.4" } else { "2.0" }, if ch == 1 { "mono" } else { "stereo" }, if g > 1.2 { "1.414" } else { "1/none" }, kind);
            classes.entry(key).or_default().push(change);
        }
        eprintln!("{} Sound objects with recordings on the shipped events' paths", sounds.len());
        for (k, mut v) in classes {
            v.sort_by(f64::total_cmp);
            eprintln!("{k:40} n {:3}  median {:+6.2}  min {:+6.2}  max {:+6.2}", v.len(), v[v.len() / 2], v[0], v[v.len() - 1]);
        }
    }

    /// The cues the level table measures, as the mixer starts them from the state of the user's recordings (at rest,
    /// engines on, ENG 4 pips for the boost): each voice's recording, gain, pitch and filter values. For
    /// `re/elite/work/sound/t5d/mfit.py`.
    #[test]
    #[ignore]
    fn cue_levels() {
        let mut r = Run::new(false);
        let mut v = Run::flying();
        v.motion = Default::default();
        v.gear_down = false;
        r.hold(&v, 5.0);
        let cue = |r: &mut Run, name: &str, v: &ShipView, secs: f64| {
            let from = r.starts.len();
            r.hold(v, secs);
            for (t, u) in &r.starts[from..] {
                eprintln!("{name}: +{:.3} s source {:08x} gain {:.2} dB pitch {:.4} low-pass {:.1} high-pass {:.1}{}", t - r.simulated + secs, match u.output { Output::Audio { source } | Output::Rumble { source } | Output::Synth { source } => source.0 }, 20.0 * u.gain.max(1e-9).log10(), u.pitch, u.lowpass, u.highpass, if u.virtual_voice { " virtual" } else { "" });
            }
        };
        v.flight_assist = false;
        cue(&mut r, "flight assist off", &v, 3.0);
        v.flight_assist = true;
        cue(&mut r, "flight assist on", &v, 3.0);
        v.pips = [1.0, 4.0, 1.0];
        cue(&mut r, "pips", &v, 2.0);
        for (stage, secs) in [(1u8, 0.6), (2, 2.0), (3, 3.0), (0, 2.0)] {
            v.boost = stage;
            cue(&mut r, &format!("boost stage {stage}"), &v, secs);
        }
    }

    /// A loop too long to keep in memory restarts on the spare decoder made off the audio thread (no frame decoded and
    /// dropped there, the join still sample for sample, the old decoder put down for the game thread, a new spare asked);
    /// a voice that ends and a spent batch list go back to the game thread instead of being dropped on the audio thread;
    /// the frames over the limiter's threshold are counted.
    #[test]
    fn long_loops_restart_on_a_spare_and_what_ends_goes_back() {
        let n = (LOOP_CACHE_S * 48000.0) as usize + 4800;
        let samples: Vec<i16> = (0..n).map(|i| (i % 30000) as i16).collect();
        let clip = Arc::new(wav(&samples));
        let c = ctl();
        let mut src = VoiceSource::new(clip.clone(), c.clone(), 0, None, None);
        assert!(src.looped.is_empty() && c.spare.lock().unwrap().is_some(), "a long loop gets a spare when built");
        let got: Vec<i32> = src.by_ref().take(n + 10).map(|s| (s * 32768.0).round() as i32).collect();
        assert_eq!(got[n - 1], ((n - 1) % 30000) as i32);
        assert_eq!(&got[n..n + 10], &(0..10).collect::<Vec<i32>>()[..], "the restart joins sample for sample");
        assert_eq!(c.meter.skipped.load(Ordering::Relaxed), 0, "nothing decoded and dropped on the audio thread");
        assert!(c.spare.lock().unwrap().is_none() && c.retired.lock().unwrap().is_some() && c.wants_spare.load(Ordering::Relaxed));
        // What ends goes back.
        let (tx, rx) = channel();
        let returned = Arc::new(Mutex::new(Returned::with_room()));
        let meter = Arc::new(Meter::new());
        meter.limit.store(0.5f32.to_bits(), Ordering::Relaxed);
        let mut out = OutputSource::new(rx, meter.clone(), returned.clone());
        let click = {
            let ctl = ctl();
            Ready { src: VoiceSource::new(Arc::new(wav(&[30000, 0, 0, 0])), ctl.clone(), 0, Some(1), None), ctl }
        };
        send_batch(&tx, vec![click]);
        let _: Vec<f32> = out.by_ref().take(2 * 20).collect();
        let r = returned.lock().unwrap();
        assert_eq!((r.voices.len(), r.batches.len()), (1, 1), "the ended voice and its batch's list are handed back");
        assert_eq!(meter.over_limit.load(Ordering::Relaxed), 1, "one frame over the threshold");
        assert_eq!(meter.dropped_here.load(Ordering::Relaxed), 0);
    }

    /// The audio thread's cost: 50 voices (the longest shipped recordings, looping) with 12 EQ stages of three peaking bands
    /// each, mixed by the output for 10 s of audio; the CPU time per second of audio. Then the same with a Guitar Distortion
    /// stage added to each (the drive layers' 02382a72 at full wet share: its six bands, the Overdrive, the rectifier).
    #[test]
    #[ignore]
    fn fifty_voices_cost() {
        let l = super::super::load::load().unwrap_or_else(|(f, e)| panic!("{f}: {e}"));
        let mut clips: Vec<Arc<Clip>> = l.clips.values().cloned().collect();
        clips.sort_by_key(|c| (std::cmp::Reverse(c.frames * u64::from(c.channels)), c.file.clone()));
        let eqs: Vec<FxNow> = (0..12).map(|k| FxNow::Eq(std::array::from_fn(|i| EqNow { on: true, filter: Some(EqFilter::PeakingEq), frequency: 100.0 + 300.0 * (3 * k + i) as f32, gain_db: -3.0, q: 1.0 }))).collect();
        let band = |filter, gain_db, frequency| EqNow { on: true, filter: Some(filter), frequency, gain_db, q: 1.0 };
        let gd = FxNow::Distortion(DistortionNow {
            pre: [band(EqFilter::LowShelf, 12.5, 462.0), band(EqFilter::PeakingEq, -2.0, 5200.0), band(EqFilter::HiShelf, -2.0, 1500.0)],
            post: [band(EqFilter::PeakingEq, 3.0, 892.0), band(EqFilter::PeakingEq, -2.5, 5234.0), band(EqFilter::HiShelf, -3.0, 5522.0)],
            kind: filter::DISTORTION_OVERDRIVE,
            drive: 29.0,
            tone: 35.0,
            rectification: 29.0,
            level: 1.0,
            wet: 48.0,
        });
        let with_gd: Vec<FxNow> = std::iter::once(gd).chain(eqs.iter().copied()).collect();
        for (name, stages) in [("12 EQ stages each", &eqs), ("12 EQ stages and a Guitar Distortion each", &with_gd)] {
            let (tx, rx) = channel();
            let mut out = OutputSource::new(rx, Arc::new(Meter::new()), Arc::default());
            let batch: Vec<Ready> = clips.iter().take(50).map(|c| {
                let ctl = ctl();
                ctl.fx.write(stages);
                Ready { src: VoiceSource::new(c.clone(), ctl.clone(), 0, None, None), ctl }
            }).collect();
            send_batch(&tx, batch);
            let t = std::time::Instant::now();
            let n = out.by_ref().take(48000 * 2 * 10).filter(|s| s.is_finite()).count();
            let ms = t.elapsed().as_secs_f64() * 1000.0 / 10.0;
            eprintln!("50 voices, {name}: {ms:.1} ms of CPU per second of audio ({n} samples)");
        }
    }

    /// The heard voices of a real supercruise entry, as the game builds them: the triggers and the mixer through 5 s of
    /// flight, the charge and 0.5 s of supercruise; each built voice's recording, its start and its latest update (gain,
    /// pitch, filter values, effects), sorted by voice. Taken round to `n` (the set again from its start) for a benchmark.
    fn supercruise_entry(n: usize) -> Vec<(Arc<Clip>, VoiceUpdate, VoiceUpdate)> {
        let mut r = Run::new(false);
        let v = Run::flying();
        r.hold(&v, 5.0);
        r.ramp(&v, CHARGE_S, Drive::Charging);
        let mut sc = v.clone();
        sc.mode = 2;
        sc.drive = Drive::Supercruise;
        r.hold(&sc, 0.5);
        let mut ids: Vec<u32> = r.last.keys().copied().collect();
        ids.sort_unstable();
        let set: Vec<_> = ids
            .iter()
            .filter_map(|id| {
                let (start, now) = r.last[id].clone();
                let Output::Audio { source } = start.output else { return None };
                Some((r.clips.get(&source)?.clone(), start, now))
            })
            .collect();
        set.iter().cycle().take(n).cloned().collect()
    }

    /// One voice of a snapshot built as the game builds it: its controls from the latest update, its source from the start.
    fn built(clip: &Arc<Clip>, start: &VoiceUpdate, now: &VoiceUpdate, meter: &Arc<Meter>) -> Ready {
        let (frame, plays, region) = start_of(start, clip.rate).unwrap();
        let ctl = Arc::new(VoiceCtl::new(now, 1.0, meter.clone()));
        Ready { src: VoiceSource::new(clip.clone(), ctl.clone(), frame, plays, region), ctl }
    }

    /// The audio thread's cost on the ship's own chains: 80 voices of a supercruise entry (its heard voices taken round to
    /// 80: `supercruise_entry`) mixed by the output for 10 s of audio, in CPU time per second of audio; the stages the
    /// voices really run are counted (EQ bands with a section, distortions).
    #[test]
    #[ignore]
    fn supercruise_voices_cost() {
        let set = supercruise_entry(80);
        let distinct = set.iter().map(|(_, s, _)| s.voice).collect::<std::collections::HashSet<_>>().len();
        let bands: usize = set.iter().map(|(_, _, u)| u.fx.iter().map(|f| match f { FxNow::Eq(b) => b.iter().filter(|b| b.on).count(), _ => 0 }).sum::<usize>()).sum();
        let dists = set.iter().map(|(_, _, u)| u.fx.iter().filter(|f| matches!(f, FxNow::Distortion(_))).count()).sum::<usize>();
        let mono = set.iter().filter(|(c, _, _)| c.channels == 1).count();
        // (Then every voice placed, as a gear motor's cue is: panned, with its attenuation `258639ce`'s curves, its emitter
        // moving round the head every block: the positioning's cost on top.)
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/sounds.json");
        let bank = se_core::elite_sound::parse_sounds(&std::fs::read(path).unwrap()).unwrap();
        let Some(se_core::elite_sound::Object::Attenuation(att)) = bank.file.objects.get(&Id(0x2586_39ce)) else { panic!("no attenuation 258639ce") };
        let placement = Arc::new(Placement::new(true, Some(att)).unwrap());
        for placed in [false, true] {
            let meter = Arc::new(Meter::new());
            let (tx, rx) = channel();
            let mut out = OutputSource::new(rx, meter.clone(), Arc::default());
            let batch: Vec<Ready> = set
                .iter()
                .map(|(c, s, u)| {
                    let mut u = u.clone();
                    if placed {
                        u.place = Some(placement.clone());
                        u.position = Some([-3.0, -1.0, 2.0]);
                    }
                    built(c, s, &u, &meter)
                })
                .collect();
            let ctls: Vec<Arc<VoiceCtl>> = batch.iter().map(|r| r.ctl.clone()).collect();
            send_batch(&tx, batch);
            let t = std::time::Instant::now();
            let mut n = 0;
            for k in 0..48000 * 10 / BLOCK {
                if placed {
                    let a = k as f32 * 0.01;
                    ctls.iter().for_each(|c| c.place.write([4.0 * a.sin(), -1.0, -4.0 * a.cos()]));
                }
                n += out.by_ref().take(2 * BLOCK).filter(|s| s.is_finite()).count();
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / 10.0;
            eprintln!(
                "80 voices of a supercruise entry ({distinct} distinct, {mono} mono; {bands} EQ bands switched on, {dists} distortions){}: {ms:.1} ms of CPU per second of audio ({n} samples)",
                if placed { ", every voice placed and moving" } else { "" }
            );
        }
    }

    /// The dropout's cause, shown on the player's own sample rate converter (rodio 0.22's `SampleRateConverter`, reached
    /// here through `Source::mix`, which converts its second input to the first's rate as the player's mixer does): it
    /// reduces the rates' ratio to `from / to` and works out `from * position` in `u32`. A voice at rate x speed of at least
    /// 89,478 Hz whose rate shares no factor with 48,000 (here `15d19ba9`, the 2 s loop the drop starts at about 2.0, at
    /// 1190 cents: 95,447 Hz) overflows it after ~0.93 s of output, and its catch-up loop then runs ~2^32 times on the audio
    /// thread: the game's whole output stopped for 138-146 s (three runs). With the speed applied inside `VoiceSource`
    /// (the output rate handed over) the same voice passes straight through.
    #[test]
    #[ignore]
    fn rodios_converter_hangs_on_a_fast_coprime_rate_and_ours_does_not() {
        let l = super::super::load::load().unwrap_or_else(|(f, e)| panic!("{f}: {e}"));
        let clip = l.clips.values().find(|c| c.file == "sfx/15d19ba9.ogg").unwrap().clone();
        let carrier = l.clips.values().find(|c| c.rate == 48000).unwrap().clone();
        let speed = 2f32.powf(1190.0 / 1200.0);
        let pull = |name: &'static str, src: Box<dyn Source<Item = f32> + Send>| {
            let (tx, rx) = std::sync::mpsc::channel();
            let lead = VoiceSource::new(carrier.clone(), ctl(), 0, None, None);
            std::thread::spawn(move || {
                // Two seconds at 48 kHz stereo.
                let n = lead.mix(src).take(48000 * 2 * 2).count();
                let _ = tx.send(n);
            });
            let r = rx.recv_timeout(std::time::Duration::from_secs(20));
            eprintln!("{name}: {r:?}");
            r.is_ok()
        };
        // The player's way, as the game had it: the recording at its own rate with no span lengths (as the old source
        // gave none: the converter is never rebuilt), the speed outside.
        struct NoSpans<S>(S);
        impl<S: Iterator<Item = f32>> Iterator for NoSpans<S> {
            type Item = f32;
            fn next(&mut self) -> Option<f32> {
                self.0.next()
            }
        }
        impl<S: Source<Item = f32>> Source for NoSpans<S> {
            fn current_span_len(&self) -> Option<usize> {
                None
            }
            fn channels(&self) -> ChannelCount {
                self.0.channels()
            }
            fn sample_rate(&self) -> SampleRate {
                self.0.sample_rate()
            }
            fn total_duration(&self) -> Option<std::time::Duration> {
                None
            }
        }
        let theirs = pull("the player's speed", Box::new(NoSpans(clip.source.decoder()).speed(speed)));
        // Ours: the source resamples, the converter is a pass-through.
        let c = ctl();
        c.speed.store(speed.to_bits(), Ordering::Relaxed);
        let ours = pull("the source's own speed", Box::new(VoiceSource::new(clip.clone(), c, 0, None, None)));
        assert!(ours, "our source must pass through");
        assert!(!theirs, "rodio's converter was expected to hang (rodio fixed? then the note in voices.rs is history)");
    }
}
