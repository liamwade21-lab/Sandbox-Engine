//! The mixer's voices as sound. The ship has one output on the audio device: [`ShipOutput`], an `AudioPlayer` entity
//! spawned when the first voice starts and despawned when the sound stops, whose [`OutputSource`] mixes every built voice
//! on the audio thread at the output rate (two channels, a mono voice on both at full level, as the player had it). The
//! game thread hands it voices and writes each voice's controls in place ([`VoiceCtl`]: level, speed, filter values, EQ
//! bands, a seek, a stop); nothing else crosses between the threads.
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
//! output rate after the resampling; then the Parametric EQ bands on its way to the master, node's and buses' in signal
//! order ([`VoiceUpdate::eq`]: a linear effect on a bus gives the bus's mix what it gives each voice), each band's section
//! designed as the exe designs it (`filter::eq_coefs`) when its values change, its memory kept (the plug-in keeps it). A
//! band that is off passes the signal.
//!
//! **Virtual voices** are not built: a voice is built when the mixer reports it heard (`VoiceUpdate::start`, from where its
//! virtual behaviour says) and dropped when it goes virtual; a virtual voice costs the audio thread nothing.
//!
//! **Starts:** the voices a frame starts are built (decoder set-up, the skip to the start offset, the loop region's
//! decode: most of a start's cost) on worker threads, one batch a frame, without holding up the game thread
//! ([`Voices::start_queued`]); a batch is handed to the output whole in the frame its last voice is ready
//! ([`Voices::hand_over`]), and the output starts all of it at one output frame, so the layers of one event stay aligned
//! sample for sample (`a_batch_starts_at_one_sample`). The delay is logged with `sw sound log on`. Batches go in the order
//! they were asked for.
//!
//! **The guard:** a sample of the last stage that is not finite becomes 0, its voice's filters start from rest, and it is
//! counted (one log line a voice, `Voices::tell_guard`); an update whose level, speed or filter value is not finite is
//! dropped (the last value stays) and logged once a voice; a build that panics loses that voice only. `sw sound log on`
//! logs the audio thread's heartbeat once a second (`Voices::meter`: frames made, the busiest voices, frames skipped).
//!
//! Every voice plays centred: where each emitter sits is a later task (the mixer's distances already act on its gains).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use bevy::audio::{AudioPlayer, AudioSource, ChannelCount, Decodable, PlaybackSettings, SampleRate, Source, Volume};
use bevy::prelude::*;
use se_core::elite_sound::data::EqFilter;
use se_core::elite_sound::filter::{self, Coefs, Section};
use se_core::elite_sound::{EqNow, VoiceUpdate, OUTPUT_RATE};

use super::load::Clip;

/// The most a voice's level is set to (a guard: the mixer's gain is at most a few times unity).
const MOST_VOLUME: f32 = 4.0;
/// No seek asked.
const NO_SEEK: u64 = u64::MAX;
/// A filter value's ramp (Elite's sound engine: `DAT_145ee0a5c` steps of `DAT_145ee0a54` frames); the level ramps over
/// one step.
const RAMP_STEPS: u8 = 8;
const RAMP_FRAMES: u32 = 128;
/// Starts in one frame up to this many are built on one worker thread; more are shared among several.
const BUILD_ALONE: usize = 4;
/// A loop region up to this long (s) is decoded when the voice is built and replayed from memory.
const LOOP_CACHE_S: f64 = 3.0;
/// The most EQ bands a voice runs: 16 stages of 3 (the shipped sounds have at most 12 stages: `every_sounds_eq_fits`).
pub const MAX_EQ_BANDS: usize = 48;
/// The output's room for voices, made when it is made (the Init bank's voice cap is 256): a voice beyond it is counted
/// and not played, never a buffer grown on the audio thread.
const MAX_ACTIVE: usize = 512;

/// What the audio thread has done, shared by every voice: frames made (a heartbeat: written every `RAMP_FRAMES` frames)
/// and samples the last stage found not finite.
#[derive(Debug, Default)]
pub struct Meter {
    made: AtomicU64,
    bad: AtomicU64,
    /// Frames decoded and dropped to reach a seek or a loop's restart on the audio thread, and the longest such skip
    /// (microseconds).
    skipped: AtomicU64,
    longest_skip_us: AtomicU64,
    /// Voices the output had no room for.
    refused: AtomicU64,
}

/// A voice's EQ bands, written by the game thread and read by the audio thread at each ramp step under a sequence lock:
/// the writer makes `seq` odd, writes, makes it even; a reader that finds it odd, or changed by the end, keeps what it had.
/// Each band is four words: on and the filter (`bit 8` on, the low byte the filter's index or 0xff), frequency, gain, Q.
#[derive(Debug)]
struct EqTable {
    seq: AtomicU32,
    n: AtomicU32,
    bands: [[AtomicU32; 4]; MAX_EQ_BANDS],
}

impl Default for EqTable {
    fn default() -> Self {
        EqTable { seq: AtomicU32::new(0), n: AtomicU32::new(0), bands: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))) }
    }
}

impl EqTable {
    fn write(&self, bands: &[EqNow]) {
        let n = bands.len().min(MAX_EQ_BANDS);
        self.seq.fetch_add(1, Ordering::AcqRel);
        for (slot, b) in self.bands.iter().zip(&bands[..n]) {
            let kind = u32::from(b.on) << 8 | b.filter.map_or(0xff, |f| f as u32);
            for (a, v) in slot.iter().zip([kind, b.frequency.to_bits(), b.gain_db.to_bits(), b.q.to_bits()]) {
                a.store(v, Ordering::Relaxed);
            }
        }
        self.n.store(n as u32, Ordering::Relaxed);
        self.seq.fetch_add(1, Ordering::AcqRel);
    }

    /// The bands into `out` (their count returned) if the table changed since `seen` and was read whole.
    fn read(&self, seen: &mut u32, out: &mut [EqNow; MAX_EQ_BANDS]) -> Option<usize> {
        let s0 = self.seq.load(Ordering::Acquire);
        if s0 == *seen || s0 & 1 == 1 {
            return None;
        }
        let n = (self.n.load(Ordering::Relaxed) as usize).min(MAX_EQ_BANDS);
        for (o, slot) in out.iter_mut().zip(&self.bands).take(n) {
            let [kind, f, g, q] = slot.each_ref().map(|a| a.load(Ordering::Relaxed));
            *o = EqNow {
                on: kind & 0x100 != 0,
                filter: EqFilter::from_index(i64::from(kind & 0xff)),
                frequency: f32::from_bits(f),
                gain_db: f32::from_bits(g),
                q: f32::from_bits(q),
            };
        }
        (self.seq.load(Ordering::Acquire) == s0).then(|| {
            *seen = s0;
            n
        })
    }
}

/// What a voice's source reads from the game while it plays.
#[derive(Debug)]
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
    eq: EqTable,
    /// Samples of this voice found not finite, and the low-pass, high-pass and speed in force at the first (`f32` bits),
    /// for the one line the game thread logs.
    bad: AtomicU32,
    first_bad: [AtomicU32; 3],
    /// Frames this voice has made (as `Meter::made`).
    made: AtomicU64,
    meter: Arc<Meter>,
}

impl VoiceCtl {
    fn idle(meter: Arc<Meter>) -> VoiceCtl {
        VoiceCtl {
            seek: AtomicU64::new(NO_SEEK),
            lowpass: AtomicU32::new(0),
            highpass: AtomicU32::new(0),
            speed: AtomicU32::new(1f32.to_bits()),
            level: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            eq: EqTable::default(),
            bad: AtomicU32::new(0),
            first_bad: Default::default(),
            made: AtomicU64::new(0),
            meter,
        }
    }

    fn new(u: &VoiceUpdate, level: f32, meter: Arc<Meter>) -> VoiceCtl {
        let c = VoiceCtl::idle(meter);
        c.write(u, level);
        c.eq.write(&u.eq);
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

    fn run(&mut self, chan: usize, x: f32) -> f32 {
        match (&self.coefs, self.channels.get_mut(chan)) {
            (Some(c), Some(s)) => s.run(c, x),
            _ => x,
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
/// EQ bands. Everything it holds is made when it is built: on the audio thread it allocates nothing.
pub struct VoiceSource {
    clip: Arc<Clip>,
    ctl: Arc<VoiceCtl>,
    dec: <AudioSource as Decodable>::Decoder,
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
    /// The output frame made last, and the channel `next` hands out next.
    out: Vec<f32>,
    out_chan: usize,
    speed: SpeedRamp,
    filters: [Filter; 2],
    /// The EQ bands (room for `MAX_EQ_BANDS`, `eq_n` of them in use), the table's sequence last read and a scratch copy.
    eq: Vec<EqStage>,
    eq_n: usize,
    eq_seen: u32,
    eq_read: Box<[EqNow; MAX_EQ_BANDS]>,
    /// Output frames since the last ramp step.
    since_step: u32,
    /// The level now and where its ramp goes (over one step).
    level: f32,
    level_to: f32,
    level_step: f32,
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
        let filters = [
            Filter::new(false, f32::from_bits(ctl.lowpass.load(Ordering::Relaxed)), clip.channels),
            Filter::new(true, f32::from_bits(ctl.highpass.load(Ordering::Relaxed)), clip.channels),
        ];
        let eq = (0..MAX_EQ_BANDS).map(|_| EqStage { now: EqNow::OFF, coefs: None, channels: vec![Section::default(); ch] }).collect();
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
            out: vec![0.0; ch],
            out_chan: 0,
            speed: SpeedRamp { now: speed, from: speed, to: speed, step: RAMP_STEPS },
            filters,
            eq,
            eq_n: 0,
            eq_seen: u32::MAX,
            eq_read: Box::new([EqNow::OFF; MAX_EQ_BANDS]),
            since_step: 0,
            level,
            level_to: level,
            level_step: 0.0,
            over: false,
            built: false,
        };
        s.read_eq();
        s.cache_loop();
        s.skip_to(start_frame.min(s.clip.frames.saturating_sub(1)));
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
                self.skip_to(self.restart);
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

    /// The EQ table read if it changed: each band's section redesigned where its values changed.
    fn read_eq(&mut self) {
        if let Some(n) = self.ctl.eq.read(&mut self.eq_seen, &mut self.eq_read) {
            for (stage, b) in self.eq.iter_mut().zip(self.eq_read.iter()).take(n) {
                stage.set(*b);
            }
            self.eq_n = n;
        }
    }

    /// The next output frame into `self.out`, or false once the voice is over.
    fn make_frame(&mut self) -> bool {
        if self.over {
            return false;
        }
        if self.since_step == 0 {
            self.speed.tick(f32::from_bits(self.ctl.speed.load(Ordering::Relaxed)));
            let targets = [self.ctl.lowpass.load(Ordering::Relaxed), self.ctl.highpass.load(Ordering::Relaxed)];
            for (f, t) in self.filters.iter_mut().zip(targets) {
                f.tick(f32::from_bits(t));
            }
            self.read_eq();
            let to = f32::from_bits(self.ctl.level.load(Ordering::Relaxed));
            if to.is_finite() {
                self.level = self.level_to;
                self.level_to = to.clamp(0.0, MOST_VOLUME);
                self.level_step = (self.level_to - self.level) / RAMP_FRAMES as f32;
            }
        }
        self.since_step = (self.since_step + 1) % RAMP_FRAMES;
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
        let k = self.phase as f32;
        self.level += self.level_step;
        let mut bad = false;
        for c in 0..self.out.len() {
            let x = self.cur[c] + (self.nxt[c] - self.cur[c]) * k;
            let low = self.filters[0].run(c, x);
            let mut y = self.filters[1].run(c, low);
            for stage in &mut self.eq[..self.eq_n] {
                if let (Some(co), Some(s)) = (&stage.coefs, stage.channels.get_mut(c)) {
                    y = s.run(co, y);
                }
            }
            self.out[c] = if y.is_finite() { y * self.level } else {
                bad = true;
                0.0
            };
        }
        if bad {
            self.not_finite();
        }
        self.phase += f64::from(self.clip.rate) / f64::from(OUTPUT_RATE) * f64::from(self.speed.now);
        if self.since_step == 0 {
            self.ctl.meter.made.fetch_add(u64::from(RAMP_FRAMES), Ordering::Relaxed);
            self.ctl.made.fetch_add(u64::from(RAMP_FRAMES), Ordering::Relaxed);
        }
        true
    }

    /// The guard of the last stage: a frame with a sample that is not finite has it as 0, the voice's filters and EQ start
    /// again from rest, and it is counted (the values in force at the first are kept for the game thread's log line).
    #[cold]
    fn not_finite(&mut self) {
        self.filters.iter_mut().for_each(Filter::clear);
        self.eq.iter_mut().for_each(|s| s.channels.iter_mut().for_each(|c| *c = Section::default()));
        self.ctl.meter.bad.fetch_add(1, Ordering::Relaxed);
        if self.ctl.bad.fetch_add(1, Ordering::Relaxed) == 0 {
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
        if self.out_chan == 0 && !self.make_frame() {
            return None;
        }
        let s = self.out.get(self.out_chan).copied().unwrap_or(0.0);
        self.out_chan = (self.out_chan + 1) % self.out.len().max(1);
        Some(s)
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

/// The ship's output: every built voice mixed at the output rate into two channels. Batches of voices arrive whole
/// through `rx` and start at one output frame. On the audio thread it allocates nothing (its voice list's room is made
/// with it) and never blocks (`try_recv`).
pub struct OutputSource {
    rx: Receiver<Vec<Ready>>,
    active: Vec<Ready>,
    frame: [f32; 2],
    chan: usize,
    meter: Arc<Meter>,
}

impl OutputSource {
    fn new(rx: Receiver<Vec<Ready>>, meter: Arc<Meter>) -> OutputSource {
        OutputSource { rx, active: Vec::with_capacity(MAX_ACTIVE), frame: [0.0; 2], chan: 0, meter }
    }

    /// One output frame: the batches that arrived join first (all of a batch at this frame), then every voice adds its
    /// frame; a voice stopped or over is dropped.
    fn mix(&mut self) {
        while let Ok(batch) = self.rx.try_recv() {
            for r in batch {
                if r.ctl.stop.load(Ordering::Relaxed) {
                    continue;
                }
                if self.active.len() < MAX_ACTIVE {
                    self.active.push(r);
                } else {
                    self.meter.refused.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        let mut f = [0.0f32; 2];
        self.active.retain_mut(|r| {
            if r.ctl.stop.load(Ordering::Relaxed) || !r.src.make_frame() {
                return false;
            }
            let o = &r.src.out;
            f[0] += o[0];
            f[1] += o.get(1).copied().unwrap_or(o[0]);
            true
        });
        self.frame = f;
    }
}

impl Iterator for OutputSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.chan == 0 {
            self.mix();
        }
        let s = self.frame[self.chan];
        self.chan ^= 1;
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
        self.ready.lock().ok().and_then(|mut r| r.take()).unwrap_or_else(|| OutputSource::new(channel().1, Arc::default()))
    }
}

/// Where a voice's start puts it: the frame, how many plays (None: for ever) and the looped region.
fn start_of(u: &VoiceUpdate, rate: u32) -> Option<(u64, Option<u32>, Option<(u64, u64)>)> {
    let start = u.start?;
    let plays = if start.looping { start.loop_count } else { Some(1) };
    let frame = (f64::from(start.offset.max(0.0)) * f64::from(rate)) as u64;
    Some((frame, plays, if start.looping { start.loop_region } else { None }))
}

/// The output's entity.
#[derive(Component)]
pub struct ShipVoice;

/// The voices built or being built (the mixer's id to their controls and recording), this frame's starts, the batches on
/// worker threads and the output.
#[derive(Default)]
pub struct Voices {
    playing: HashMap<u32, Playing>,
    queued: Vec<Queued>,
    building: VecDeque<Batch>,
    output: Option<(Entity, Sender<Vec<Ready>>)>,
    meter: Arc<Meter>,
}

struct Playing {
    ctl: Arc<VoiceCtl>,
    clip: Arc<Clip>,
    /// The EQ bands last written (the table is written only when they change).
    eq: Vec<EqNow>,
    /// The guard's lines already logged for this voice: an update not finite, a sample not finite.
    told_update: bool,
    told_sample: bool,
    /// The voice's frame count at the last `meter`.
    made_seen: u64,
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

/// One frame's starts on worker threads: the threads, when asked, and how many.
struct Batch {
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

    /// Every voice stopped at once (the output despawned with all it holds; batches still building are dropped when done).
    pub fn stop_all(&mut self, commands: &mut Commands) {
        if let Some((e, _)) = self.output.take() {
            commands.entity(e).try_despawn();
        }
        self.playing.clear();
        self.queued.clear();
        self.building.clear();
    }

    /// One mixer update: queue its voice's start, update it in place, drop it when it goes virtual or stop it. `level`
    /// maps the mixer's gain to the output's level.
    pub fn apply<'a>(&mut self, u: &'a VoiceUpdate, clip: Option<&'a Arc<Clip>>, level: f32, log: &mut impl FnMut(Event<'a>)) {
        if u.stop || u.virtual_voice {
            if let Some(p) = self.playing.remove(&u.voice) {
                p.ctl.stop.store(true, Ordering::Relaxed);
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
            if let Some(old) = self.playing.insert(u.voice, Playing { ctl: ctl.clone(), clip: c.clone(), eq: u.eq.clone(), told_update: false, told_sample: false, made_seen: 0 }) {
                old.ctl.stop.store(true, Ordering::Relaxed);
            }
            self.queued.retain(|q| q.voice != u.voice);
            self.queued.push(Queued { voice: u.voice, clip: c.clone(), ctl, start_frame, plays, region });
            return;
        }
        let Some(p) = self.playing.get_mut(&u.voice) else { return };
        if let Some(to) = u.seek_to.filter(|t| t.is_finite()) {
            p.ctl.seek.store((f64::from(to.max(0.0)) * f64::from(p.clip.rate)) as u64, Ordering::Relaxed);
        }
        if p.eq != u.eq {
            p.ctl.eq.write(&u.eq);
            p.eq.clone_from(&u.eq);
        }
        if !p.ctl.write(u, level) && !std::mem::replace(&mut p.told_update, true) {
            warn!(
                "sidewinder sound: voice {} {}: an update not finite (gain {}, pitch {}, low-pass {}, high-pass {}) dropped, the last values kept",
                u.voice, p.clip.file, u.gain, u.pitch, u.lowpass, u.highpass
            );
        }
    }

    /// The frame's queued starts sent to worker threads as one batch (shared among a few threads when there are more than
    /// `BUILD_ALONE`); the game thread does not wait for them. The output is spawned with the first start.
    pub fn start_queued(&mut self, commands: &mut Commands, outputs: &mut Assets<ShipOutput>) {
        if self.queued.is_empty() {
            return;
        }
        if self.output.is_none() {
            let (tx, rx) = channel();
            let h = outputs.add(ShipOutput { ready: Arc::new(Mutex::new(Some(OutputSource::new(rx, self.meter.clone())))) });
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
        self.building.push_back(Batch { parts, asked: std::time::Instant::now(), count });
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
            if let Some((_, tx)) = &self.output {
                let _ = tx.send(ready);
            }
            done.push((b.count, b.asked.elapsed().as_secs_f64()));
        }
        done
    }

    /// Frames the audio thread has made and samples it found not finite, since the voices began; frames skipped and the
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
            m.bad.load(Ordering::Relaxed),
            m.skipped.swap(0, Ordering::Relaxed),
            m.longest_skip_us.swap(0, Ordering::Relaxed),
            m.refused.load(Ordering::Relaxed),
            top,
        )
    }

    /// The guard's one line a voice: the first sample of it the last stage found not finite.
    pub fn tell_guard(&mut self) {
        for (id, p) in &mut self.playing {
            let n = p.ctl.bad.load(Ordering::Relaxed);
            if n > 0 && !std::mem::replace(&mut p.told_sample, true) {
                let [lp, hp, sp] = p.ctl.first_bad.each_ref().map(|a| f32::from_bits(a.load(Ordering::Relaxed)));
                warn!("sidewinder sound: voice {id} {}: a sample not finite replaced by 0 and its filters reset (speed {sp:.3}, low-pass {lp:.1}, high-pass {hp:.1}; {n} so far)", p.clip.file);
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

    /// A voice's controls at level 1, nothing else set.
    fn ctl() -> Arc<VoiceCtl> {
        let c = VoiceCtl::idle(Arc::default());
        c.level.store(1f32.to_bits(), Ordering::Relaxed);
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

    use super::super::triggers::{motion_in, Call, Flight, ShipView, Triggers};
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
        sources: HashMap<u32, (VoiceSource, Arc<VoiceCtl>, f64)>,
        meter: Arc<Meter>,
        worst: f32,
        samples: u64,
        simulated: f64,
        cpu: std::time::Duration,
        most: usize,
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
                sources: HashMap::new(),
                meter: Arc::default(),
                worst: 0.0,
                samples: 0,
                simulated: 0.0,
                cpu: std::time::Duration::ZERO,
                most: 0,
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
                    continue;
                }
                if u.start.is_some() {
                    self.playing.insert(u.voice, (u.emitter, source));
                    self.built.insert(u.voice);
                    self.builds += 1;
                    *self.started.entry(source).or_default() += 1;
                    self.starts.push((self.simulated, u.clone()));
                    if self.audio {
                        let (frame, plays, region) = start_of(&u, clip.rate).unwrap();
                        let ctl = Arc::new(VoiceCtl::new(&u, 1.0, self.meter.clone()));
                        self.sources.insert(u.voice, (VoiceSource::new(clip, ctl.clone(), frame, plays, region), ctl, 0.0));
                    }
                    continue;
                }
                self.playing.insert(u.voice, (u.emitter, source));
                if let Some((_, ctl, _)) = self.sources.get(&u.voice) {
                    if let Some(to) = u.seek_to.filter(|t| t.is_finite()) {
                        ctl.seek.store((f64::from(to.max(0.0)) * f64::from(clip.rate)) as u64, Ordering::Relaxed);
                    }
                    ctl.write(&u, 1.0);
                    ctl.eq.write(&u.eq);
                }
            }
            self.most = self.most.max(self.playing.len());
            if self.audio {
                let t = std::time::Instant::now();
                let mut ended = Vec::new();
                for (id, (src, _, carry)) in &mut self.sources {
                    let want = f64::from(OUTPUT_RATE) * v.dt + *carry;
                    let frames = want.floor();
                    *carry = want - frames;
                    for _ in 0..frames as u64 * u64::from(src.clip.channels) {
                        match src.next() {
                            Some(s) => {
                                assert!(s.is_finite() && s.abs() <= 4.0, "voice {id} {}: {s}", src.clip.file);
                                self.worst = self.worst.max(s.abs());
                                self.samples += 1;
                            }
                            None => {
                                ended.push(*id);
                                break;
                            }
                        }
                    }
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
            v.motion = motion_in(&Flight { vel: [0.0, 0.0, -100.0], acc: [0.0; 3], omega: [0.0; 3], top_speed: 220.0, accel_fwd: 40.0, back_share: 0.5, throttle: 1.0 });
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
        assert_eq!(r.meter.bad.load(Ordering::Relaxed), 0);
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
        const GROWING: [(u32, Id); 2] = [(0x100, Id(0x1223_e7e9)), (0x205, Id(0x06a6_2f20))];
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
            eq: Vec::new(),
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

    /// A mono 16-bit WAV clip of the given samples at 48 kHz.
    fn wav(samples: &[i16]) -> Clip {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
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
        Clip { source, file: "click.wav".into(), frames: samples.len() as u64, channels: 1, rate: 48000 }
    }

    /// Two voices built apart (as two worker threads build them) and handed over as one batch while the output is already
    /// playing start at the same output frame: a click of +0.25 in one and +0.5 in the other add up to 0.75 in a single
    /// frame, and nothing of either comes before it.
    #[test]
    fn a_batch_starts_at_one_sample() {
        let (tx, rx) = channel();
        let mut out = OutputSource::new(rx, Arc::default());
        let early: Vec<f32> = out.by_ref().take(2 * 333).collect();
        assert!(early.iter().all(|s| *s == 0.0), "silent with no voice");
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
        tx.send(batch).unwrap();
        let after: Vec<f32> = out.by_ref().take(2 * 20).collect();
        let left: Vec<f32> = after.iter().step_by(2).copied().collect();
        assert_eq!(left[0], 0.75, "both clicks in the batch's first frame: {left:?}");
        assert!(left[1..].iter().all(|s| *s == 0.0), "{left:?}");
        assert_eq!(after[1], 0.75, "a mono voice on both channels at full level");
    }

    /// A voice's EQ bands run after its filters, each the exe's section: a voice with one band (a peak of -6 dB at 1 kHz)
    /// gives what that section gives the recording; a band that is off, or a table read half-written, changes nothing.
    #[test]
    fn a_voices_eq_bands_shape_it() {
        let tone: Vec<i16> = (0..9600).map(|i| ((std::f64::consts::TAU * 1000.0 * f64::from(i) / 48000.0).sin() * 16000.0) as i16).collect();
        let clip = Arc::new(wav(&tone));
        let peak = EqNow { on: true, filter: Some(EqFilter::PeakingEq), frequency: 1000.0, gain_db: -6.0, q: 1.0 };
        let c = ctl();
        c.eq.write(&[EqNow::OFF, peak]);
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
        off.eq.write(&[EqNow { on: false, ..peak }]);
        let untouched: Vec<f32> = VoiceSource::new(clip, off, 0, Some(1), None).collect();
        assert_eq!(untouched, plain);
        // (A reader that meets the table mid-write keeps what it had.)
        let t = EqTable::default();
        t.write(&[peak]);
        let mut seen = 0;
        let mut out = [EqNow::OFF; MAX_EQ_BANDS];
        assert_eq!(t.read(&mut seen, &mut out), Some(1));
        assert_eq!(t.read(&mut seen, &mut out), None, "unchanged");
        t.seq.fetch_add(1, Ordering::AcqRel);
        assert_eq!(t.read(&mut seen, &mut out), None, "being written");
    }

    /// Every shipped sound's EQ bands fit a voice's table.
    #[test]
    fn every_sounds_eq_fits() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/sounds.json");
        let bank = se_core::elite_sound::parse_sounds(&std::fs::read(path).unwrap()).unwrap();
        let most = bank.file.objects.keys().map(|id| bank.fx_stages(*id).len()).max().unwrap_or(0);
        eprintln!("the most EQ stages on a shipped sound's way: {most}");
        assert!(most * 3 <= MAX_EQ_BANDS, "{most} stages");
    }

    /// A long session (the review's finding): supercruise round trips with a boost after each, through the real triggers
    /// and mixer, and the voices built as the game builds them (heard ones only). After 1, 50 and 500 trips: the objects
    /// playing, the voices built, the starts of a boost, and the mixer's step time (median over the last trip's frames).
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
                eprintln!(
                    "after {trip} trips: {} objects playing, {} voices ({} built), a boost built {boost_builds} voices with {boost_voices} voices in all; frame (triggers + mixer) median {ms:.3} ms",
                    r.mixer.playing_objects().len(),
                    r.playing.len(),
                    r.built.len()
                );
            }
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
