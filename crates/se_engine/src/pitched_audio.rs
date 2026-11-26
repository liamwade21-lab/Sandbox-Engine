//! Sounds played at a speed (a pitch) that the port applies itself, so rodio only ever sees speed 1.0 and a standard rate.
//!
//! Why: rodio 0.22.2's resampler (`conversions/sample_rate.rs`) multiplies two `u32`s, `from * next_output_span_pos_in_chunk`.
//! When Windows plays at 176.4 or 192 kHz and a sound's rate has no common factor with the device's (a wave's rate times a
//! random pitch, cut to a whole number: about one pitch in four), that wraps, and a loop runs about four billion times inside
//! the audio thread: every sound freezes for seconds per bad sound, nothing is logged, and swing after swing of the crowbar
//! the freezes stack up until the sound never comes back. At the standard rates (11025, 22050, 44100, 48000) and speed 1.0
//! the numbers stay small at every device rate (the tests go over the grid).
//!
//! So a voice whose pitch is not 1, or that changes, plays a `PitchedSound`: its source steps through the wave's samples
//! (decoded once, `Wave`) at the speed with linear interpolation between neighbouring frames, channel by channel, and reports
//! the wave's own channel count and rate (44100 for a wave of any other rate, the conversion folded into its step) whatever
//! the speed. A looping one loops inside the source, the interpolation wrapping across the seam (rodio's `repeat_infinite`,
//! Bevy's `PlaybackMode::Loop`, buffers the first pass's output for ever: a changing speed would freeze and the memory grow).
//! Play a looping one with `PlaybackMode::Once` (it never ends, so it lasts until its entity goes, as a `Loop` does).
//!
//! Never pass a speed to rodio (`with_speed`, `AudioSink::set_speed`, `PlaybackSettings { speed }`): a test fails if one
//! comes back. The speed of a playing voice is its `Speed` handle, read by its source once an output frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bevy::audio::{AddAudioSource, AudioSource, ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::*;

pub struct PitchedAudioPlugin;

impl Plugin for PitchedAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<PitchedSound>();
    }
}

/// The rates rodio is given: with these its resampler's numbers stay small at every device rate.
pub const STANDARD_RATES: [u32; 4] = [11_025, 22_050, 44_100, 48_000];

/// What a wave of any other rate is reported at.
const OTHER_RATE: u32 = 44_100;

/// The speeds a voice can have (a value outside is held at the nearest; one not finite is ignored). Above 0, or a voice
/// that should end never would.
const SPEED_RANGE: (f32, f32) = (1.0 / 64.0, 64.0);

/// A sound's samples, decoded once and shared by every voice that plays it.
#[derive(Clone, Debug)]
pub struct Wave {
    /// Channels interleaved.
    samples: Arc<[Sample]>,
    channels: u16,
    rate: u32,
}

impl Wave {
    pub fn new(channels: u16, rate: u32, samples: Vec<Sample>) -> Wave {
        let channels = channels.max(1);
        let mut samples = samples;
        // (Whole frames only.)
        samples.truncate(samples.len() / usize::from(channels) * usize::from(channels));
        Wave { samples: samples.into(), channels, rate: rate.max(1) }
    }

    /// A file Bevy can play (Ogg Vorbis, WAV) decoded once by Bevy's own decoder; `None` if it can't be decoded (Bevy's
    /// decoder panics on a bad file: caught here).
    pub fn decode(source: &AudioSource) -> Option<Wave> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let decoder = source.decoder();
            let (channels, rate) = (decoder.channels().get(), decoder.sample_rate().get());
            Wave::new(channels, rate, decoder.collect())
        }))
        .ok()
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    /// The memory its samples take.
    pub fn bytes(&self) -> usize {
        std::mem::size_of_val(&*self.samples)
    }

    /// The rate its source reports to rodio.
    pub fn reported_rate(&self) -> u32 {
        if STANDARD_RATES.contains(&self.rate) { self.rate } else { OTHER_RATE }
    }
}

/// A playing voice's speed: the game writes it, the voice's source reads it once an output frame.
#[derive(Clone, Debug)]
pub struct Speed(Arc<AtomicU32>);

impl Speed {
    pub fn new(speed: f32) -> Speed {
        let s = Speed(Arc::new(AtomicU32::new(1f32.to_bits())));
        s.set(speed);
        s
    }

    pub fn set(&self, speed: f32) {
        if speed.is_finite() {
            self.0.store(speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// One voice's sound: a wave, the speed it plays at and whether it loops. Made for each voice (it is small: the samples are
/// shared), so each has its own speed; it goes when the voice's entity does.
#[derive(Asset, TypePath, Clone, Debug)]
pub struct PitchedSound {
    wave: Wave,
    speed: Speed,
    looping: bool,
}

impl PitchedSound {
    /// Played once from the start, at a fixed speed (it lasts the wave's length over the speed).
    pub fn once(wave: &Wave, speed: f32) -> PitchedSound {
        PitchedSound { wave: wave.clone(), speed: Speed::new(speed), looping: false }
    }

    /// Looped for ever at the speed the handle says (play it with `PlaybackMode::Once`).
    pub fn looping(wave: &Wave, speed: &Speed) -> PitchedSound {
        PitchedSound { wave: wave.clone(), speed: speed.clone(), looping: true }
    }
}

impl Decodable for PitchedSound {
    type Decoder = PitchedSource;

    fn decoder(&self) -> PitchedSource {
        PitchedSource::new(self.wave.clone(), self.speed.clone(), self.looping)
    }
}

/// The source rodio pulls: the wave stepped through at the speed, interleaved samples out.
pub struct PitchedSource {
    wave: Wave,
    speed: Speed,
    looping: bool,
    /// The wave's frames to one reported frame at speed 1 (1 at a standard rate).
    ratio: f64,
    /// Where it is: a whole frame and the fraction past it (kept apart, so speed 1 is exact and hours of looping don't drift).
    frame: usize,
    frac: f64,
    /// The channel the next sample is of.
    chan: u16,
}

impl PitchedSource {
    fn new(wave: Wave, speed: Speed, looping: bool) -> PitchedSource {
        let ratio = f64::from(wave.rate) / f64::from(wave.reported_rate());
        PitchedSource { wave, speed, looping, ratio, frame: 0, frac: 0.0, chan: 0 }
    }
}

impl Iterator for PitchedSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let frames = self.wave.frames();
        let ch = usize::from(self.wave.channels);
        // (Ends only on a frame's boundary: `frame` moves only after a frame's last channel.)
        if self.frame >= frames {
            return None;
        }
        let c = usize::from(self.chan);
        let a = self.wave.samples[self.frame * ch + c];
        let out = if self.frac == 0.0 {
            a
        } else {
            let next = self.frame + 1;
            // (Past the end: the first frame of a loop, silence after a sound that ends.)
            let b = if next < frames {
                self.wave.samples[next * ch + c]
            } else if self.looping {
                self.wave.samples[c]
            } else {
                0.0
            };
            a + (b - a) * self.frac as Sample
        };
        self.chan += 1;
        if usize::from(self.chan) == ch {
            self.chan = 0;
            let p = self.frac + f64::from(self.speed.get()) * self.ratio;
            let whole = p.floor();
            self.frac = p - whole;
            self.frame += whole as usize;
            if self.looping {
                self.frame %= frames;
            }
        }
        Some(out)
    }
}

impl Source for PitchedSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.wave.channels).unwrap_or(ChannelCount::MIN)
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.wave.reported_rate()).unwrap_or(SampleRate::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        // (The speed can change while it plays.)
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use rodio::{Player, SpatialPlayer, mixer};

    use super::*;

    /// The device rates Windows can be set to that the tests play at.
    const DEVICE_RATES: [u32; 5] = [44_100, 48_000, 96_000, 176_400, 192_000];

    /// The pitch that made the crowbar's hit freeze the sound at 192 kHz (44100 x it = 48511, prime to 192000).
    const BAD_PITCH: f32 = 1.100_034;

    /// cpal's blocks: 10 ms at the device's rate, stereo.
    fn block_samples(device_rate: u32) -> usize {
        (device_rate / 100) as usize * 2
    }

    fn sine(channels: u16, rate: u32, seconds: f32, hz: &[f32]) -> Wave {
        let frames = (seconds * rate as f32) as usize;
        let mut s = Vec::with_capacity(frames * usize::from(channels));
        for i in 0..frames {
            for c in 0..usize::from(channels) {
                let f = hz[c % hz.len()];
                s.push((std::f64::consts::TAU * f64::from(f) * i as f64 / f64::from(rate)).sin() as f32 * 0.5);
            }
        }
        Wave::new(channels, rate, s)
    }

    /// The crowbar's hit from the pack (as the sandbox reads it: 16-bit PCM made a WAV, decoded by Bevy's decoder), or a
    /// mono 44.1 kHz sound of its shape without the pack.
    fn crowbar() -> (AudioSource, String) {
        let path = "weapons/crowbar/crowbar_impact1.wav";
        if let Some(Ok(pack)) = crate::sandbox::assets::open_pack()
            && let Ok(pcm) = pack.sound(path)
        {
            let desc = format!("{path} from the pack ({} Hz, {} channel(s), {:.2} s)", pcm.rate, pcm.channels, pcm.seconds());
            return (AudioSource { bytes: Arc::from(pcm.to_wav()) }, desc);
        }
        let w = sine(1, 44_100, 0.65, &[700.0]);
        let samples = w.samples.iter().map(|s| (s * 30_000.0) as i16).collect();
        let pcm = se_core::gmod::Pcm { rate: 44_100, channels: 1, samples, loop_start: None };
        (AudioSource { bytes: Arc::from(pcm.to_wav()) }, "a synthetic 0.65 s mono 44.1 kHz sound".to_string())
    }

    /// What a pull of the mixer gave: the seconds of work until the player was empty, the stereo frames that came out (to
    /// the last one that isn't silence) and the loudest sample.
    struct Played {
        secs: f64,
        frames: usize,
        peak: f32,
    }

    impl Played {
        /// The frames a sound of `seconds` (its length over its speed) should give at the device's rate, its source reporting
        /// `rodio_rate`, and not silence. Within a block over; under by at most rodio's
        /// own loss, its queue's last span of a sound (512 samples) cut short (Bevy's own decoder loses the same:
        /// `the_same_length_as_bevys_own_path`), plus a block.
        fn check(&self, seconds: f64, rodio_rate: f64, device_rate: u32, what: &str) {
            let want = seconds * f64::from(device_rate);
            let block = f64::from(device_rate / 100);
            let span = RODIO_SPAN_FRAMES / rodio_rate * f64::from(device_rate);
            let got = self.frames as f64;
            assert!(got <= want + block && got >= want - span - block, "{what}: {} frames came out, {want:.0} wanted", self.frames);
            assert!(self.peak > 0.01, "{what}: silence came out (peak {})", self.peak);
        }
    }

    /// The most of a sound's end rodio's queue drops, in frames of the source it is given (its `threshold`, 512 samples: a
    /// mono source's 512 frames, the most).
    const RODIO_SPAN_FRAMES: f64 = 512.0;

    /// Pulls a mixer at `device_rate` the way cpal does (blocks of 10 ms) until the player has played everything, on its own
    /// thread; `None` if it was still going after `guard`.
    fn time_playback<F>(device_rate: u32, guard: Duration, start: F) -> Option<Played>
    where
        F: FnOnce(&rodio::mixer::Mixer) -> Box<dyn Fn() -> bool> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mix, mut out) = mixer::mixer(ChannelCount::new(2).unwrap(), SampleRate::new(device_rate).unwrap());
            let empty = start(&mix);
            // Samples pulled, the first and last that aren't silence (counted from 1), the loudest.
            let (mut n, mut first, mut last, mut peak) = (0usize, 0usize, 0usize, 0.0f32);
            let mut pull = || {
                for _ in 0..block_samples(device_rate) {
                    let s = out.next().unwrap_or(0.0);
                    n += 1;
                    if s != 0.0 {
                        first = if first == 0 { n } else { first };
                        last = n;
                    }
                    peak = peak.max(s.abs());
                }
            };
            let t = Instant::now();
            // (A player's queue plays silence once empty, so the mixer never ends: stop when the player says it's done, or
            // after a minute of sound, which no sound here lasts.)
            for _ in 0..6000 {
                pull();
                if empty() {
                    break;
                }
            }
            let secs = t.elapsed().as_secs_f64();
            // The player is empty once its source has ended, while the converters still hold its last span: 100 ms more
            // brings it out. The sound's length is from its first sample that isn't silence to its last.
            for _ in 0..10 {
                pull();
            }
            let frames = if first == 0 { 0 } else { (last - first + 1).div_ceil(2) };
            let _ = tx.send(Played { secs, frames, peak });
        });
        rx.recv_timeout(guard).ok()
    }

    /// The new path: a `PitchedSound` at `speed` through a `Player`, as Bevy plays a voice.
    fn time_pitched(wave: &Wave, speed: f32, device_rate: u32, spatial: bool, guard: Duration) -> Option<Played> {
        let sound = PitchedSound::once(wave, speed);
        time_playback(device_rate, guard, move |mix| {
            if spatial {
                let p = SpatialPlayer::connect_new(mix, [0.3, 0.0, 0.4], [0.1, 0.0, 0.0], [-0.1, 0.0, 0.0]);
                p.append(sound.decoder());
                Box::new(move || p.empty())
            } else {
                let p = Player::connect_new(mix);
                p.append(sound.decoder());
                Box::new(move || p.empty())
            }
        })
    }

    /// The most a sound's work may take, as a share of its length, with every core busy (a hang is many times its length).
    const MOST_WORK: f64 = 0.25;

    /// The user's proof: the crowbar's hit at the bad pitch through rodio's own Player on a 192 kHz mixer, pulled in cpal's
    /// blocks, takes a fraction of a second of work.
    #[test]
    fn the_worst_case_plays_at_once() {
        let (source, desc) = crowbar();
        let wave = Wave::decode(&source).unwrap();
        let length = wave.frames() as f64 / f64::from(wave.rate) / f64::from(BAD_PITCH);
        for spatial in [false, true] {
            let p = time_pitched(&wave, BAD_PITCH, 192_000, spatial, Duration::from_secs(60)).expect("still playing after a minute");
            println!(
                "{desc} at speed {BAD_PITCH}, 192 kHz, spatial {spatial}: {:.4} s of work for {length:.3} s of sound; {} frames pulled, {:.0} wanted, peak {:.3}",
                p.secs,
                p.frames,
                length * 192_000.0,
                p.peak
            );
            assert!(p.secs < 0.25, "{desc}: {:.3} s of work for {length:.3} s of sound", p.secs);
            p.check(length, f64::from(wave.reported_rate()), 192_000, &desc);
        }
    }

    /// The bug the source avoids, kept to show it: the same sound through rodio's own speed (`Player::set_speed`, as
    /// `PlaybackSettings::with_speed` does). It passes when the hang shows (seconds of work for 0.6 s of sound); the guard
    /// gives up after 60 s.
    #[test]
    #[ignore = "documents rodio 0.22.2's hang: about 7 s of a CPU"]
    fn rodios_own_speed_hangs_at_192_khz() {
        let (source, desc) = crowbar();
        let took = time_playback(192_000, Duration::from_secs(60), move |mix| {
            let p = Player::connect_new(mix);
            Player::set_speed(&p, BAD_PITCH);
            p.append(source.decoder());
            Box::new(move || p.empty())
        });
        let took = took.map(|p| p.secs);
        println!("{desc} through rodio's speed {BAD_PITCH} at 192 kHz: {took:?} s of work");
        assert!(took.is_none_or(|t| t > 0.5), "rodio's own speed didn't hang: {took:?}");
    }

    /// The fixed source at every standard rate, mono and stereo, at every device rate, over a spread of speeds and 200
    /// random pitches as the scripts give them: no second of sound takes more than a small share of a second of work. (Run
    /// on every core.)
    #[test]
    fn every_rate_and_speed_plays_at_once() {
        let mut speeds = vec![0.5, 0.9, 1.0, BAD_PITCH, 1.2, 2.0];
        let mut rng = crate::sandbox::audio::Rng(2026);
        speeds.extend((0..200).map(|_| rng.range(0.9, 1.2)));
        let mut cells = Vec::new();
        for rate in STANDARD_RATES {
            for channels in [1u16, 2] {
                let wave = sine(channels, rate, 1.0, &[440.0, 660.0]);
                for device in DEVICE_RATES {
                    for &s in &speeds {
                        cells.push((wave.clone(), s, device));
                    }
                }
            }
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let next = std::sync::atomic::AtomicUsize::new(0);
        let worst = std::sync::Mutex::new((0.0f64, String::new()));
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((wave, s, device)) = cells.get(i) else { break };
                        let what = format!("{} Hz x{} at speed {s} on {device} Hz", wave.rate, wave.channels);
                        // (A bad cell is given up on after 2 s: a hang is seconds a sound.)
                        let played = time_pitched(wave, *s, *device, false, Duration::from_secs(2));
                        let length = wave.frames() as f64 / f64::from(wave.rate) / f64::from(*s);
                        if let Some(p) = &played {
                            p.check(length, f64::from(wave.reported_rate()), *device, &what);
                        }
                        let took = played.map_or(f64::INFINITY, |p| p.secs);
                        let share = took / length;
                        let mut w = worst.lock().unwrap();
                        if share > w.0 {
                            *w = (share, format!("{what}: {took:.4} s for {length:.3} s"));
                        }
                    }
                });
            }
        });
        let (share, cell) = worst.into_inner().unwrap();
        println!("{} cells; the slowest: {cell} ({:.2} % of its length)", cells.len(), share * 100.0);
        assert!(share < MOST_WORK, "too slow: {cell}");
    }

    /// Two of SE's music tracks are 96 kHz, played as they are (`music.rs`, speed 1, through rodio): that rate at speed 1
    /// keeps the resampler's numbers small at every device rate too.
    #[test]
    fn se_musics_96_khz_plays_at_once() {
        let w = sine(2, 96_000, 1.0, &[440.0, 660.0]);
        for device in DEVICE_RATES {
            let samples = w.samples.to_vec();
            let played = time_playback(device, Duration::from_secs(60), move |mix| {
                let p = Player::connect_new(mix);
                p.append(rodio::buffer::SamplesBuffer::new(ChannelCount::new(2).unwrap(), SampleRate::new(96_000).unwrap(), samples));
                Box::new(move || p.empty())
            })
            .expect("still playing after a minute");
            assert!(played.secs < MOST_WORK, "96 kHz on {device} Hz: {:.3} s for 1 s", played.secs);
            played.check(1.0, 96_000.0, device, &format!("96 kHz on {device} Hz"));
        }
    }

    /// At speed 1 a sound comes out of rodio as long as through Bevy's own `AudioSource` path, which the voices used before
    /// (both lose the same end of the last span of rodio's queue: see `Played::check`).
    #[test]
    fn the_same_length_as_bevys_own_path() {
        for rate in STANDARD_RATES {
            for channels in [1u16, 2] {
                let w = sine(channels, rate, 1.0, &[440.0, 660.0]);
                let samples: Vec<i16> = w.samples.iter().map(|s| (s * 30_000.0) as i16).collect();
                let wav = se_core::gmod::Pcm { rate, channels, samples, loop_start: None }.to_wav();
                for device in DEVICE_RATES {
                    let src = AudioSource { bytes: Arc::from(wav.clone()) };
                    let bevys = time_playback(device, Duration::from_secs(10), move |mix| {
                        let p = Player::connect_new(mix);
                        p.append(src.decoder());
                        Box::new(move || p.empty())
                    })
                    .unwrap();
                    let ours = time_pitched(&w, 1.0, device, false, Duration::from_secs(10)).unwrap();
                    let what = format!("{rate} Hz x{channels} on {device} Hz");
                    assert_eq!(ours.frames, bevys.frames, "{what}");
                    ours.check(1.0, f64::from(rate), device, &what);
                }
            }
        }
    }

    fn pull_all(source: PitchedSource) -> Vec<Sample> {
        source.collect()
    }

    /// Crossings of zero upward on one channel.
    fn rises(samples: &[Sample], channels: usize, c: usize) -> usize {
        let ch: Vec<Sample> = samples.iter().skip(c).step_by(channels).copied().collect();
        ch.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn a_speed_raises_the_pitch_and_shortens_the_sound() {
        for channels in [1u16, 2] {
            let hz = [441.0, 300.0];
            let wave = sine(channels, 44_100, 1.0, &hz);
            for s in [0.5f32, 0.9, BAD_PITCH, 1.2, 2.0] {
                let out = pull_all(PitchedSound::once(&wave, s).decoder());
                let ch = usize::from(channels);
                assert_eq!(out.len() % ch, 0);
                let frames = out.len() / ch;
                let want = (wave.frames() as f64 / f64::from(s)).ceil() as usize;
                assert!(frames.abs_diff(want) <= 1, "{channels} ch at {s}: {frames} frames, {want} wanted");
                let seconds = frames as f64 / 44_100.0;
                for c in 0..ch {
                    let heard = rises(&out, ch, c) as f64 / seconds;
                    let want = f64::from(hz[c]) * f64::from(s);
                    assert!((heard / want - 1.0).abs() < 0.01, "{channels} ch, channel {c} at {s}: {heard:.1} Hz, {want:.1} wanted");
                }
            }
        }
    }

    #[test]
    fn at_speed_one_the_samples_are_the_waves_own() {
        for channels in [1u16, 2] {
            let wave = sine(channels, 22_050, 0.3, &[123.0, 456.0]);
            let out = pull_all(PitchedSound::once(&wave, 1.0).decoder());
            assert_eq!(out.as_slice(), &*wave.samples);
            let s = PitchedSound::once(&wave, 1.0).decoder();
            assert_eq!((s.channels().get(), s.sample_rate().get()), (channels, 22_050));
        }
    }

    /// A wave of a rate that isn't standard is reported at 44100 and keeps its pitch.
    #[test]
    fn another_rate_is_converted_by_the_step() {
        let wave = sine(1, 32_000, 1.0, &[400.0]);
        for s in [1.0f32, 1.5] {
            let src = PitchedSound::once(&wave, s).decoder();
            assert_eq!(src.sample_rate().get(), 44_100);
            let out = pull_all(src);
            let seconds = out.len() as f64 / 44_100.0;
            assert!((seconds - 1.0 / f64::from(s)).abs() < 1e-3, "{seconds} s at {s}");
            let heard = rises(&out, 1, 0) as f64 / seconds;
            assert!((heard / (400.0 * f64::from(s)) - 1.0).abs() < 0.01, "{heard} Hz at {s}");
        }
    }

    /// A loop goes round with no jump at its seam larger than the wave's own steps (its seam counted as one of them).
    #[test]
    fn a_loop_wraps_without_a_click() {
        for channels in [1u16, 2] {
            // 100 whole periods, so the seam is as smooth as the rest.
            let wave = sine(channels, 44_100, 1.0, &[441.0, 882.0]);
            let ch = usize::from(channels);
            let frames = wave.frames();
            for s in [0.9f32, 1.0, BAD_PITCH, 1.7] {
                for c in 0..ch {
                    let own = (0..frames).map(|i| (wave.samples[((i + 1) % frames) * ch + c] - wave.samples[i * ch + c]).abs()).fold(0.0, f32::max);
                    let out: Vec<Sample> = PitchedSound::looping(&wave, &Speed::new(s)).decoder().take(frames * ch * 3).collect();
                    assert_eq!(out.len(), frames * ch * 3, "a loop never ends");
                    let chan: Vec<Sample> = out.iter().skip(c).step_by(ch).copied().collect();
                    let most = chan.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
                    assert!(most <= own * s.max(1.0) * 1.001, "{channels} ch, channel {c} at {s}: a jump of {most}, the wave's own {own}");
                }
            }
        }
    }

    /// The speed written while it plays takes effect from the next frame out.
    #[test]
    fn a_speed_changed_while_playing_follows_at_once() {
        let ramp: Vec<Sample> = (0..10_000).map(|i| i as Sample).collect();
        let wave = Wave::new(1, 44_100, ramp);
        let speed = Speed::new(1.0);
        let mut src = PitchedSound::looping(&wave, &speed).decoder();
        let first: Vec<Sample> = src.by_ref().take(100).collect();
        assert_eq!(first, (0..100).map(|i| i as Sample).collect::<Vec<_>>());
        speed.set(2.0);
        let then: Vec<Sample> = src.by_ref().take(4).collect();
        assert_eq!(then, vec![100.0, 102.0, 104.0, 106.0]);
        speed.set(0.5);
        let slow: Vec<Sample> = src.take(3).collect();
        assert_eq!(slow, vec![108.0, 108.5, 109.0]);
    }

    /// A sound played once ends (so a `Despawn` voice goes, in its length over the speed); a loop never empties its player.
    #[test]
    fn once_ends_and_a_loop_goes_on_in_a_player() {
        let wave = sine(1, 44_100, 0.2, &[440.0]);
        let (mix, mut out) = mixer::mixer(ChannelCount::new(2).unwrap(), SampleRate::new(48_000).unwrap());
        let once = Player::connect_new(&mix);
        once.append(PitchedSound::once(&wave, 2.0).decoder());
        let looped = Player::connect_new(&mix);
        looped.append(PitchedSound::looping(&wave, &Speed::new(2.0)).decoder());
        // 0.1 s and a little more at 48 kHz, stereo.
        let mut pulled = 0;
        while !once.empty() && pulled < 48_000 * 2 {
            out.next();
            pulled += 1;
        }
        let secs = pulled as f64 / 96_000.0;
        assert!((secs - 0.1).abs() < 0.01, "the sound at speed 2 lasted {secs} s");
        for _ in 0..96_000 {
            out.next();
        }
        assert!(!looped.empty(), "the loop ended");
    }

    #[test]
    fn a_speed_out_of_reason_is_held() {
        let s = Speed::new(f32::NAN);
        assert_eq!(s.get(), 1.0);
        s.set(0.0);
        assert_eq!(s.get(), SPEED_RANGE.0);
        s.set(f32::INFINITY);
        assert_eq!(s.get(), SPEED_RANGE.0);
        s.set(1e9);
        assert_eq!(s.get(), SPEED_RANGE.1);
    }

    /// What `speed_uses` looks for, joined at run time so this file's own text never matches.
    fn needle(parts: &[&str]) -> String {
        parts.concat()
    }

    /// Where a file's code hands rodio a speed, as (line, what): a call of `set_speed(` or `with_speed(` in any form (a method,
    /// `Player::set_speed(&p, x)`, `AudioSink::set_speed(..)`); in a file that names `PlaybackSettings`, a field written
    /// (`.speed =`), a field named (`speed:`), and a `speed` in a `PlaybackSettings { .. }` literal up to its own closing brace
    /// (braces inside counted). Comment lines are skipped (they may name what is forbidden).
    fn speed_uses(source: &str) -> Vec<(usize, &'static str)> {
        let code: Vec<&str> = source.lines().map(|l| if l.trim_start().starts_with("//") { "" } else { l }).collect();
        let text = code.join("\n");
        let (set, with, settings) = (needle(&["set", "_speed("]), needle(&["with", "_speed("]), needle(&["Playback", "Settings"]));
        let names_settings = text.contains(&settings);
        let written = |line: &str| {
            let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            t.contains(".speed=") && !t.contains(".speed==")
        };
        let named = |line: &str| line.match_indices("speed:").any(|(i, _)| !line[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_'));
        let mut found = Vec::new();
        for (n, line) in code.iter().enumerate() {
            if line.contains(&set) || line.contains(&with) {
                found.push((n + 1, "a set_speed/with_speed call"));
            }
            if names_settings && written(line) {
                found.push((n + 1, "a speed field written"));
            }
            if names_settings && named(line) {
                found.push((n + 1, "a speed field named"));
            }
        }
        // Each literal: from the settings' name, past white space, a `{`, to the brace that closes it.
        let mut from = 0;
        while let Some(i) = text[from..].find(&settings) {
            let after = from + i + settings.len();
            from = after;
            let rest = &text[after..];
            let open = rest.len() - rest.trim_start().len();
            if !rest[open..].starts_with('{') {
                continue;
            }
            let mut depth = 0usize;
            let mut end = rest.len();
            for (k, c) in rest[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if rest[open..end].split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "speed") {
                found.push((text[..after].lines().count(), "a speed in a settings literal"));
            }
        }
        found
    }

    /// The uses the guard lets be: (file under `src`, what, text on the line, why). Only the `speed:` rule is let be in whole
    /// files, where their literals and field writes are still checked.
    const ALLOWED: &[(&str, &str, &str, &str)] = &[
        ("pitched_audio.rs", "a set_speed/with_speed call", "Player::set_speed(&p, BAD_PITCH)", "the ignored test that shows rodio's own speed hanging"),
        ("sidewinder/sound/voices.rs", "a speed field named", "", "its voices apply the speed themselves (VoiceCtl, SpeedRamp) and report 48 kHz"),
        ("skate/sound.rs", "a speed field named", "speed: Speed,", "the loop voice's pitched_audio::Speed handle"),
        ("viltrumite/sound.rs", "a speed field named", "speed: Speed,", "the loop voice's pitched_audio::Speed handle"),
        ("viltrumite/sound.rs", "a speed field named", "fn splash(&mut self, speed: f64)", "a cue's argument: how fast he hit the sea"),
    ];

    /// The guard sees each way of handing rodio a speed, and not the port's own handle.
    #[test]
    fn the_guard_sees_every_form() {
        let settings = needle(&["Playback", "Settings"]);
        let cases = [
            (needle(&["sink.set", "_speed(1.2);"]), "a set_speed/with_speed call"),
            (needle(&["Player::set", "_speed(&p, x);"]), "a set_speed/with_speed call"),
            (needle(&["AudioSink::set", "_speed(&mut sink, x);"]), "a set_speed/with_speed call"),
            (format!("let a = {settings}::ONCE.{};", needle(&["with", "_speed(2.0)"])), "a set_speed/with_speed call"),
            (format!("let mut s = {settings}::ONCE;\ns.speed = 1.1;"), "a speed field written"),
            (format!("let s = {settings} {{ mode: m,\n speed: 1.1, ..default() }};"), "a speed field named"),
            (format!("let s = {settings} {{ volume: v(|| {{ 1.0 }}), speed, ..default() }};"), "a speed in a settings literal"),
        ];
        for (code, what) in &cases {
            assert!(speed_uses(code).iter().any(|(_, w)| w == what), "missed ({what}): {code}");
        }
        // Not a hand-over: the port's own handle, a comment, a file that never names the settings, settings without a speed.
        assert!(speed_uses("voice.speed.set(voice.pitch);").is_empty());
        assert!(speed_uses(&format!("// {}", needle(&["sink.set", "_speed(1.2);"]))).is_empty());
        assert!(speed_uses("cam.speed = 3.0;\nlet c = Cue { speed: 1.0 };").is_empty());
        assert!(speed_uses(&format!("let s = {settings}::ONCE.with_volume(v);\nlet t = {settings} {{ mode: m, ..default() }};")).is_empty());
    }

    /// Guard: nothing in the engine hands rodio a speed again (see this module's docs: rodio's resampler hangs the audio
    /// thread at 176.4 and 192 kHz).
    #[test]
    fn no_speed_reaches_rodio() {
        let mut found = Vec::new();
        let mut used = vec![false; ALLOWED.len()];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") {
                    continue;
                }
                let rel: String = p.strip_prefix(&root).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                let source = std::fs::read_to_string(&p).unwrap();
                let lines: Vec<&str> = source.lines().collect();
                for (n, what) in speed_uses(&source) {
                    let line = lines.get(n - 1).map_or("", |l| l.trim());
                    match ALLOWED.iter().position(|(file, w, text, _)| rel == *file && what == *w && line.contains(text)) {
                        Some(a) => used[a] = true,
                        None => found.push(format!("{rel}:{n}: {what}: {line}")),
                    }
                }
            }
        }
        // (An entry nothing needs any more goes, so the list stays short.)
        for (a, (file, what, text, _)) in ALLOWED.iter().enumerate() {
            if !used[a] {
                found.push(format!("ALLOWED has an entry nothing needs: {file}, {what}, {text:?}"));
            }
        }
        assert!(
            found.is_empty(),
            "a speed handed to rodio: its resampler hangs the audio thread at 176.4/192 kHz with many pitches. Play the sound as a \
             pitched_audio::PitchedSound (pitched_audio.rs) and set its Speed instead:\n{}",
            found.join("\n")
        );
    }

    /// The rate and channels of every sound the sandbox, the Viltrumite and Skate 3 mods and SE's own sounds play (the
    /// report's table). Reads the pack, the mods' folders and SE's install where present.
    #[test]
    #[ignore = "a table for the report: reads the pack and SE's install"]
    fn print_sound_rates() {
        use std::collections::BTreeMap;
        fn add(table: &mut BTreeMap<(String, u32, u16), (usize, String)>, group: &str, rate: u32, channels: u16, name: &str) {
            let e = table.entry((group.to_string(), rate, channels)).or_insert((0, name.to_string()));
            e.0 += 1;
        }
        let mut table = BTreeMap::new();
        if let Some(Ok(pack)) = crate::sandbox::assets::open_pack() {
            for path in pack.list("sound/", ".wav") {
                match pack.sound(&path) {
                    Ok(p) => add(&mut table, "gmod pack", p.rate, p.channels, &path),
                    Err(e) => println!("{path}: {e}"),
                }
            }
        }
        let mods = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods");
        for m in ["viltrumite", "skate3"] {
            for e in std::fs::read_dir(mods.join(m).join("sfx")).unwrap().flatten() {
                let bytes = std::fs::read(e.path()).unwrap();
                let name = e.file_name().to_string_lossy().into_owned();
                match Wave::decode(&AudioSource { bytes: Arc::from(bytes) }) {
                    Some(w) => add(&mut table, m, w.rate, w.channels, &name),
                    None => println!("{m}/{name}: can't decode"),
                }
            }
        }
        if let Ok(vfs) = se_core::vfs::Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")) {
            for dir in ["sounds", "music"] {
                for path in vfs.list(dir) {
                    let lower = path.to_ascii_lowercase();
                    if !(lower.ends_with(".ogg") || lower.ends_with(".wav")) {
                        continue;
                    }
                    let Ok(bytes) = vfs.read(&path) else { continue };
                    match Wave::decode(&AudioSource { bytes: Arc::from(bytes) }) {
                        Some(w) => add(&mut table, &format!("SE {dir}"), w.rate, w.channels, &path),
                        None => println!("{path}: can't decode"),
                    }
                }
            }
        }
        for ((group, rate, channels), (n, example)) in &table {
            println!("{group:12} {rate:6} Hz {channels} ch {n:5} files, e.g. {example}");
        }
    }
}
