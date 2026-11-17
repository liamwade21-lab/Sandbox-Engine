//! The Wwise Tone Generator (source plug-in 0x00660002) as Elite's own copy of it plays (`EliteDangerous64.exe`, build
//! 2026.10.23.334925, decompiled offline into `re/elite/ghidra/dec/`; T5d-8 in `re/elite/notes/sidewinder_sound_events.md`):
//! registration record 0x145ee2fe8 (create 0x144ae5580, parameters 0x144ae56d0), the plug-in's `Init` 0x144ae57c0,
//! `Reset` 0x144ae5e30, `Execute` 0x144ae5700; its parameters' `SetParamsBlock` 0x144ae61e0 and `SetParam` 0x144ae6060.
//! The data's four (sources `11681f29`, `37399b95`, `12cbc351`, `28952f3b`) use the sine (`FUN_144ae7d60`) and the white
//! noise (`FUN_144ae7f90`), each in its envelope or fixed mode; the other waveforms (triangle, square, saw tooth: four
//! times oversampled and filtered; pink noise) and the frequency sweep are not built, and a block that needs one is
//! refused ([`ToneParams::from_block`]).
//!
//! Per frame, as the exe works it: the envelope's stage moves on when its frame count reaches the stage's length (one
//! check a frame), the envelope adds its stage's slope; the gain (`10^(dB/20)`, ramped by 10 / rate a frame towards a new
//! value, constant here: no curve drives it); the sine reads the 512-entry table ([`sine_table`]) between two entries at
//! the phase before it moves on (`(next - this) frac + this`), the noise is a 32-bit linear congruence shared by every
//! noise voice (`DAT_145ee0fd0 = DAT_145ee0fd0 x 0xbb38435 + 0x3619636b`, read as a signed integer times 2^-31); the
//! output is `gain x wave x envelope` (sine) or `envelope x noise x gain x 2^-31`. At the period's end (the stages' frames
//! summed) the envelope starts again. A play is `loops x` that period (an endless loop for a loop count of 0).

use std::sync::OnceLock;

/// The table's entries (`DAT_145ee0fe0`) and the shift of its index.
const TABLE: usize = 512;
const TABLE_MASK: u32 = (TABLE - 1) as u32;
/// The lowest frequency the sine plays (`DAT_144deaa54`), the highest (`DAT_145155018`, or half the rate).
const MIN_FREQ: f32 = f32::from_bits(0x3a83_126f);
const MAX_FREQ: f32 = 20000.0;
/// The fixed mode's fades (s, `DAT_144dec660`) when it plays once.
const FADE_S: f32 = f32::from_bits(0x3ba3_d70a);
/// The noise's step (`0xbb38435`, `0x3619636b`), its first value in the exe's image (`DAT_145ee0fd0`) and its scale
/// (`DAT_1450cb494`, 2^-31).
pub const NOISE_MUL: u32 = 0x0bb3_8435;
pub const NOISE_ADD: u32 = 0x3619_636b;
pub const NOISE_SEED: u32 = 0x0000_56ce;
const NOISE_SCALE: f32 = f32::from_bits(0x3000_0000);
const DB20: f32 = f32::from_bits(0x3d4c_cccd);
/// The gain's ramp per frame: 10 / rate (`Reset`: `DAT_144de1680` / the rate).
const RAMP_PER_S: f32 = 10.0;

/// The waveforms and modes the data uses (`eGenType`, `eGenMode`).
pub const SINE: u32 = 0;
pub const WHITE_NOISE: u32 = 4;
pub const MODE_FIXED: u32 = 0;
pub const MODE_ENVELOPE: u32 = 1;

/// The sine's table (`DAT_145ee0fe0`): `f32(10^(-0.1/20) sin(2 pi i / 512))` worked out in `f64` (the 512 values are the
/// exe's bit for bit: `re/elite/work/sound/t5d8/table.py`).
pub fn sine_table() -> &'static [f32; TABLE] {
    static T: OnceLock<[f32; TABLE]> = OnceLock::new();
    T.get_or_init(|| {
        let a = 10f64.powf(-0.1 / 20.0);
        std::array::from_fn(|i| (a * (std::f64::consts::TAU * i as f64 / TABLE as f64).sin()) as f32)
    })
}

/// A Tone Generator's parameter block (69 bytes) as `SetParamsBlock` 0x144ae61e0 copies it, every value as stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneParams {
    pub gain_db: f32,
    pub start_freq: f32,
    pub stop_freq: f32,
    pub start_rand: [f32; 2],
    pub sweep: bool,
    pub sweep_kind: u32,
    pub stop_rand: [f32; 2],
    pub waveform: u32,
    pub mode: u32,
    pub fixed_s: f32,
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain_s: f32,
    pub sustain_db: f32,
    pub release_s: f32,
    pub channel_mask: u32,
}

impl ToneParams {
    /// The block from its hex (`sounds.json`'s `params_hex`): f32 gain at 0, start frequency 4, stop 8, start's random range
    /// 0x0c / 0x10, u8 sweep 0x14, u32 sweep kind 0x15, stop's random range 0x19 / 0x1d, u32 waveform 0x21, u32 mode 0x25,
    /// f32 fixed duration 0x29, attack 0x2d, decay 0x31, sustain 0x35, sustain level (dB) 0x39, release 0x3d, u32 channel
    /// mask 0x41. An error for a block of another length, or one needing what is not built (a waveform other than the sine
    /// and the white noise, a sweep, a mode other than the two, more than one channel).
    pub fn from_block(hex: &str) -> Result<ToneParams, String> {
        let b: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(hex.get(2 * i..2 * i + 2).unwrap_or("zz"), 16)).collect::<Result<_, _>>().map_err(|_| "its block is not hex".to_string())?;
        if b.len() != 0x45 {
            return Err(format!("a block of {} bytes, the Tone Generator's is 69", b.len()));
        }
        let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let p = ToneParams {
            gain_db: f(0),
            start_freq: f(4),
            stop_freq: f(8),
            start_rand: [f(0x0c), f(0x10)],
            sweep: b[0x14] != 0,
            sweep_kind: u(0x15),
            stop_rand: [f(0x19), f(0x1d)],
            waveform: u(0x21),
            mode: u(0x25),
            fixed_s: f(0x29),
            attack_s: f(0x2d),
            decay_s: f(0x31),
            sustain_s: f(0x35),
            sustain_db: f(0x39),
            release_s: f(0x3d),
            channel_mask: u(0x41),
        };
        if p.waveform != SINE && p.waveform != WHITE_NOISE {
            return Err(format!("waveform {} is not built (the sine and the white noise are)", p.waveform));
        }
        if p.waveform == SINE && (p.sweep || p.start_rand[0] != p.start_rand[1]) {
            return Err("a sweep or a random start frequency is not built".into());
        }
        if p.mode > MODE_ENVELOPE {
            return Err(format!("mode {} is not built", p.mode));
        }
        if p.channel_mask.count_ones() != 1 {
            return Err(format!("channel mask {:#x}: only one channel is built", p.channel_mask));
        }
        Ok(p)
    }

    /// The stages' lengths (frames: attack, decay, sustain, release as `Init` makes them, each `(int)(rate x s)`), their
    /// slopes and the envelope's start, and the period (the lengths summed before the zero lengths are made 1).
    fn stages(&self, rate: u32, loops: u16) -> ([u32; 4], [f32; 4], f32, u32) {
        let frames = |s: f32| (rate as f32 * s) as i64 as i32 as u32;
        if self.mode == MODE_ENVELOPE {
            let mut len = [frames(self.attack_s), frames(self.decay_s), frames(self.sustain_s), frames(self.release_s)];
            let period = len[0].wrapping_add(len[3]).wrapping_add(len[1]).wrapping_add(len[2]);
            for k in [0, 1, 3] {
                len[k] = len[k].max(1);
            }
            let s = f64::from(10f32).powf(f64::from(self.sustain_db * DB20)) as f32;
            let slopes = [1.0 / len[0] as f32, (-1.0 / len[1] as f32) * (1.0 - s), 0.0, (-1.0 / len[3] as f32) * s];
            (len, slopes, 0.0, period)
        } else if loops == 1 {
            let fade = rate as f32 * FADE_S;
            let n = fade as i64 as i32 as u32;
            let len = [n, 0, frames(self.fixed_s), n];
            (len, [1.0 / n as f32, 0.0, 0.0, -1.0 / n as f32], 0.0, len.iter().fold(0u32, |a, l| a.wrapping_add(*l)))
        } else {
            let len = [0, 0, frames(self.fixed_s), 0];
            (len, [0.0; 4], 1.0, len[2])
        }
    }

    /// A play's frames at `rate` for a loop count (`Init`'s +0x18 = loops x the period), None for an endless loop.
    pub fn play_frames(&self, rate: u32, loops: u16) -> Option<u64> {
        (loops != 0).then(|| u64::from(loops) * u64::from(self.stages(rate, loops).3))
    }

    /// The period's frames at `rate` (`Init`'s +0x1c), for a voice that plays once or not (the fixed mode fades in and out
    /// only when it plays once).
    pub fn period_frames(&self, rate: u32, once: bool) -> u32 {
        self.stages(rate, if once { 1 } else { 0 }).3
    }
}

/// One Tone Generator voice (mono). Made with everything it needs; [`ToneGen::next`] allocates nothing.
#[derive(Clone, Debug)]
pub struct ToneGen {
    noise: bool,
    /// The sine's phase (an index into the table and the fraction past it) and its step.
    index: u32,
    frac: f32,
    inc: f32,
    /// The envelope: the stage, frames in it, the value; the stages' lengths and slopes; the start value at each period.
    stage: usize,
    count: u32,
    env: f32,
    len: [u32; 4],
    slope: [f32; 4],
    env_start: f32,
    /// Frames into the period, and the period.
    at: u32,
    period: u32,
    gain: f32,
    /// Frames left in the play (None: endless).
    left: Option<u64>,
}

impl ToneGen {
    /// The voice as `Init` then `Reset` leave it (`Reset` is what gives the gain its value and the ramp its step: without
    /// it nothing would sound), at the output rate, for its sound's loop count (0 endless, as Wwise's).
    pub fn new(p: &ToneParams, rate: u32, loops: u16) -> ToneGen {
        let (len, slope, env_start, period) = p.stages(rate, loops);
        let freq = p.start_freq + p.start_rand[0];
        let top = (rate as f32 * 0.5).min(MAX_FREQ);
        let f = if MIN_FREQ <= freq { if top <= freq { top - 1.0 } else { freq } } else { MIN_FREQ };
        ToneGen {
            noise: p.waveform == WHITE_NOISE,
            index: 0,
            frac: 0.0,
            inc: f * TABLE as f32 / rate as f32,
            stage: 0,
            count: 0,
            env: env_start,
            len,
            slope,
            env_start,
            at: 0,
            period,
            gain: f64::from(10f32).powf(f64::from(p.gain_db * DB20)) as f32,
            left: p.play_frames(rate, loops),
        }
    }

    /// The ramp of a gain change (10 / rate a frame): constant here, so the gain is its value from the first frame.
    pub fn ramp_per_frame(rate: u32) -> f32 {
        RAMP_PER_S / rate as f32
    }

    /// The next frame, None once the play is over; `seed` is the noise's state, shared by every noise voice.
    pub fn next(&mut self, seed: &mut u32) -> Option<f32> {
        if let Some(n) = &mut self.left {
            if *n == 0 {
                return None;
            }
            *n -= 1;
        }
        let (this, next, frac) = if self.noise {
            *seed = seed.wrapping_mul(NOISE_MUL).wrapping_add(NOISE_ADD);
            (0.0, 0.0, 0.0)
        } else {
            let t = sine_table();
            let i = self.index & TABLE_MASK;
            let (this, next, frac) = (t[i as usize], t[((i + 1) & TABLE_MASK) as usize], self.frac);
            let whole = self.inc as i64 as i32;
            let mut index = self.index.wrapping_add(whole as u32);
            let f = self.frac + (self.inc - whole as u32 as f32);
            self.frac = f;
            if 1.0 < f {
                index = index.wrapping_add(1);
                self.frac = f - 1.0;
            }
            self.index = index & TABLE_MASK;
            (this, next, frac)
        };
        let stage = self.stage.min(3);
        let (stage, count) = if self.len[stage] <= self.count { ((stage + 1).min(3), 0) } else { (stage, self.count) };
        self.stage = stage;
        self.env += self.slope[stage];
        self.count = count + 1;
        let env = self.env;
        self.at += 1;
        if self.period <= self.at {
            self.at = 0;
            self.stage = 0;
            self.count = 0;
            self.env = self.env_start;
        }
        Some(if self.noise {
            env * (*seed as i32 as f32) * self.gain * NOISE_SCALE
        } else {
            self.gain * ((next - this) * frac + this) * env
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four shipped sources' blocks (`sounds.json`'s `source_plugins`, from `media/plugins.json`).
    const SINE_6K: &str = "000090c10080bb4500007a440000000000000000000000000000000000000000000000000001000000000000410000c03f0000000000008040000000000000404004000000";
    const NOISE_A: &str = "000010c100007a4400007a4400000000000000000000000000000000000000000004000000010000000000003f00000000000000000000803f000000000000404004000000";
    const NOISE_B: &str = "000010c100007a4400007a4400000000000000000000000000000000000000000004000000010000000000803f000000000000000000004040000000000000003f04000000";
    const NOISE_FIX: &str = "0000c0c000007a4400007a4400000000000000000000000000000000000000000004000000000000000000803f00000000000000000000803f000040c10000000004000000";

    fn play(p: &ToneParams, loops: u16, frames: usize) -> Vec<f32> {
        let mut g = ToneGen::new(p, 48000, loops);
        let mut seed = NOISE_SEED;
        (0..frames).map_while(|_| g.next(&mut seed)).collect()
    }

    /// The blocks read as the audio export's decoder names them (`media/plugins.json`).
    #[test]
    fn the_blocks_read_as_the_export_names_them() {
        let s = ToneParams::from_block(SINE_6K).unwrap();
        assert_eq!((s.gain_db, s.start_freq, s.stop_freq, s.waveform, s.mode, s.channel_mask), (-18.0, 6000.0, 1000.0, SINE, MODE_ENVELOPE, 4));
        assert_eq!((s.fixed_s, s.attack_s, s.decay_s, s.sustain_s, s.sustain_db, s.release_s), (8.0, 1.5, 0.0, 4.0, 0.0, 3.0));
        let f = ToneParams::from_block(NOISE_FIX).unwrap();
        assert_eq!((f.gain_db, f.waveform, f.mode, f.fixed_s, f.sustain_db), (-6.0, WHITE_NOISE, MODE_FIXED, 1.0, -12.0));
        assert!(ToneParams::from_block(&SINE_6K[..20]).is_err());
        let mut tri = SINE_6K.to_string();
        tri.replace_range(0x21 * 2..0x21 * 2 + 2, "01");
        assert!(ToneParams::from_block(&tri).unwrap_err().contains("waveform 1"));
    }

    /// Each one's length: the envelope's stages summed (`Init`), the fixed mode's period, an endless loop none.
    #[test]
    fn each_plays_its_length() {
        let p = |h| ToneParams::from_block(h).unwrap();
        assert_eq!(p(SINE_6K).play_frames(48000, 1), Some(408_000));
        assert_eq!(p(NOISE_A).play_frames(48000, 1), Some(192_000));
        assert_eq!(p(NOISE_B).play_frames(48000, 1), Some(168_000));
        assert_eq!(p(NOISE_FIX).play_frames(48000, 0), None);
        assert_eq!(p(NOISE_FIX).period_frames(48000, false), 48_000);
        assert_eq!(p(NOISE_FIX).play_frames(48000, 1), Some(240 + 48_000 + 240));
        assert_eq!(play(&p(NOISE_B), 1, 400_000).len(), 168_000);
        assert_eq!(play(&p(NOISE_FIX), 0, 200_000).len(), 200_000);
    }

    /// The sine: 6 kHz, its peak at full envelope the gain (-18 dB) times the table's 10^(-0.1/20), the attack a straight
    /// line over its 1.5 s, the release one down to 0 over its 3 s.
    #[test]
    fn the_sine_has_its_frequency_level_and_envelope() {
        let p = ToneParams::from_block(SINE_6K).unwrap();
        let y = play(&p, 1, 500_000);
        let peak = |a: usize, b: usize| y[a..b].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let full = 10f32.powf(-18.0 / 20.0) * 10f32.powf(-0.1 / 20.0);
        assert!((peak(100_000, 200_000) - full).abs() < 1e-4 * full, "{} against {full}", peak(100_000, 200_000));
        assert!((peak(35_000, 37_000) / full - 0.5).abs() < 0.02, "half way up the attack: {}", peak(35_000, 37_000) / full);
        assert!((peak(336_000, 336_100) / full - 0.5).abs() < 0.02, "half way down the release");
        // (Near 0, not at it: the envelope adds its slope 144,000 times in f32, as the exe does.)
        assert!(peak(407_900, 408_000) < 0.02 * full, "{}", peak(407_900, 408_000) / full);
        let crossings = y[100_000..148_000].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((5999..=6001).contains(&crossings), "{crossings} cycles in a second");
    }

    /// The noise: the exe's congruence from its first value, white and uniform: its RMS the gain over sqrt 3, its sustain
    /// and release as the envelope says (a 0 attack is one frame long).
    #[test]
    fn the_noise_is_the_exes_and_has_its_level() {
        let p = ToneParams::from_block(NOISE_A).unwrap();
        let mut g = ToneGen::new(&p, 48000, 1);
        let mut seed = NOISE_SEED;
        let first = g.next(&mut seed).unwrap();
        let s1 = NOISE_SEED.wrapping_mul(0x0bb3_8435).wrapping_add(0x3619_636b);
        assert_eq!(seed, s1);
        let gain = f64::from(10f32).powf(f64::from(-9.0 * DB20)) as f32;
        assert_eq!(first, 1.0 * (s1 as i32 as f32) * gain * NOISE_SCALE);
        let y = play(&p, 1, 192_000);
        let rms = (y[1000..48_000].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / 47_000.0).sqrt();
        let want = f64::from(10f32.powf(-0.45)) / 3f64.sqrt();
        assert!((rms / want - 1.0).abs() < 0.02, "{rms} against {want}");
        let tail = (y[191_000..].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / 1000.0).sqrt();
        assert!(tail < 0.01 * want, "the release ends at 0");
        // The fixed mode looping for ever: the envelope at 1 throughout, its sustain level not used.
        let f = play(&ToneParams::from_block(NOISE_FIX).unwrap(), 0, 100_000);
        let rms = (f[60_000..].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / 40_000.0).sqrt();
        assert!((rms / (f64::from(10f32.powf(-0.3)) / 3f64.sqrt()) - 1.0).abs() < 0.02, "{rms}");
    }

    /// The table: the exe's values at a few places (bits read from `DAT_145ee0fe0`).
    #[test]
    fn the_table_is_the_exes() {
        let t = sine_table();
        assert_eq!(t[0], 0.0);
        assert_eq!(t[1].to_bits(), 0.012131066992878914f32.to_bits());
        assert_eq!(t[128].to_bits(), 0.9885531067848206f32.to_bits());
        assert_eq!(t[511], -t[1]);
    }
}
