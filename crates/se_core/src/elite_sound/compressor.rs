//! The Wwise Compressor (plug-in 0x006c0003) as Elite's own copy of it runs (`EliteDangerous64.exe`, build
//! 2026.10.23.334925, decompiled offline into `re/elite/ghidra/dec/`; the port T5d-9 in
//! `re/elite/notes/sidewinder_sound_events.md`), for the one bus the output mixes as a 12-channel sub-mix
//! ([`super::SUBMIX_BUS`]). Every other bus's compressor stays unbuilt (none would act at the ship's levels).
//!
//! - **Registration** (record 0x145edfe58: type 3, company 0, id 0x6c, create 0x1449f4680, the parameters' create
//!   0x1449f5100): the effect's vtable 0x1457db400 (slot 2 Reset 0x1449f4fe0, slot 6 Init 0x1449f47d0, slot 7 Execute
//!   0x1449f46c0), the parameters' 0x1457db478 (slot 1 `SetParam` 0x1449f5180, slot 5 `SetParamsBlock` 0x1449f5300).
//! - **Init** (0x1449f47d0): the rate from the format; the channels counted, all of them less the LFE when Process LFE is
//!   off and the layout has one; the attack and release coefficients `expf(-2.2 / (max(t, 0.001) x rate))`, the
//!   detector's `expf(-43.0663223 / rate)`; the output gain kept as the last buffer's. Channel Link on (with more than one
//!   channel) takes the linked routine 0x1449f4c00; the unlinked one (0x1449f4930) is not built: the sub-mix's compressor
//!   links. Process LFE and Channel Link are read here only. Reset (0x1449f4fe0) zeroes the gain state and the power.
//! - **Execute** (0x1449f46c0, once a Wwise buffer: 1024 frames in Elite, [`BUFFER_FRAMES`]): a coefficient worked out
//!   again when its time changed; the linked routine with the threshold and `1 / ratio - 1`; then the output gain ramped
//!   over the buffer from the last buffer's to this one's (0x1449f4310: four lanes from `g, g + d, g + 2d, g + 3d`, each
//!   moving by `4d`, `d = (new - last) / 1024`; with the two the same a plain multiply, skipped at 1: 0x1449f4540).
//! - **The linked routine** (0x1449f4c00; the order of every operation read from its disassembly, `re/elite/work/sound/
//!   t5d9/dis_9f4c00.txt`), each frame: the squares of the counted channels summed in channel order, times `1 / n`, plus
//!   1e-25 (the mean square over the bus's channels: one detector for all of them); the power `p = (p - ms) c + ms`; its
//!   fast level in dB, `11.5811863 / (m + 1)^3 x (m^3 - 1) + (e - 127) x 3.0103` (m and e the mantissa and exponent of
//!   p's bits), less the threshold, at least 0 (the static curve: a hard knee); the gain state `g = (g - over) k + over`, k
//!   the attack's coefficient when `over - g >= 0`, else the release's; then `x = g (1 / ratio - 1) 0.05`, the gain 0 for
//!   `x < -37`, else the fast power of ten of `x` (the bits of `x x 2^23 log2(10) + 127 x 2^23`, truncated: the mantissa's
//!   `(m 0.325189769 + 0.0208057724) m + 0.653043449` times the exponent's bits); every counted channel times the gain.
//!
//! The fast power gives 0.99903899 for a gain of 0 dB ([`rest_gain`]): the compressor with nothing over its threshold
//! lowers its bus by 0.0084 dB. That is Elite's own maths, kept. `expf` (`FUN_1448c0530`) and `powf` (`FUN_1448b2470`) are
//! taken as correctly rounded (worked out in `f64`, rounded once: E, as in [`super::master`]); they are used only when
//! parameters change. Ours (not Elite's): a sample in that is not finite is 0 and one beyond [`INPUT_BOUND`] is held there
//! (the voices' guards make neither reachable), and a block whose output is not finite (never seen) is silenced and the
//! state reset, counted.

use super::data;

/// The sub-mix's channels, in Wwise's pipeline order: FL FR FC BL BR SL SR HFL HFR HBL HBR (the mask's order, as
/// [`super::place::DOWN_MIX_L`] lists them), then the LFE (E: Wwise keeps the LFE last; the LFE carries nothing here, and
/// a 0 added anywhere in the detector's sum leaves it as it is).
pub const CHANNELS: usize = 12;
pub const LFE: usize = 11;
/// Frames of a Wwise buffer in Elite (T5d-3: every plug-in ramps over 1024 frames): Execute takes its parameters once a
/// buffer.
pub const BUFFER_FRAMES: u32 = 1024;
/// The rate the plug-in is initialised with (Elite's sound engine at 48 kHz, T5g section 0).
pub const RATE: f32 = 48000.0;
/// Ours: the largest magnitude a sample in may have (beyond it is held there).
pub const INPUT_BOUND: f32 = 1.0e6;

const ONE: f32 = 1.0;
/// `DAT_14538aae8` (-2.2), the shortest time (`DAT_144deaa54`, 0.001 s), the detector's numerator (`DAT_1457db470`).
const TIME_K: f32 = f32::from_bits(0xc00c_cccd);
const MIN_TIME: f32 = f32::from_bits(0x3a83_126f);
const DETECT_K: f32 = f32::from_bits(0xc22c_43ea);
/// Added to the mean square (`DAT_1457db448`, 1e-25).
const FLOOR: f32 = f32::from_bits(0x15f7_9688);
/// The fast level's constants (`DAT_1457db460`, `DAT_1457db45c` = 10 log10(2), the exponent's bias `DAT_144e299b8`).
const LOG_K: f32 = f32::from_bits(0x4139_4c8a);
const DB_PER_OCTAVE: f32 = f32::from_bits(0x4040_a8c1);
const EXP_BIAS: f32 = 127.0;
/// dB to a power of ten (`DAT_144de1654`, 0.05).
const DB20: f32 = f32::from_bits(0x3d4c_cccd);
/// The fast power's floor (`DAT_1457db46c`, -37), scale (`DAT_1457db464`, 2^23 log2(10)) and bias (`DAT_1457db468`,
/// 127 x 2^23), its polynomial (`DAT_1457db450`, `DAT_1457db44c`, `DAT_1457db454`).
const POW_FLOOR: f32 = -37.0;
const POW_SCALE: f32 = f32::from_bits(0x4bd4_9a78);
const POW_BIAS: f32 = f32::from_bits(0x4e7e_0000);
const POW_A: f32 = f32::from_bits(0x3ea6_7f46);
const POW_B: f32 = f32::from_bits(0x3caa_70de);
const POW_C: f32 = f32::from_bits(0x3f27_2ddb);
/// The output ramp's lanes (`DAT_144ded004`, 4).
const LANES: f32 = 4.0;
/// `SetParam`'s clamps: the threshold -96.3..0 dB (`DAT_1457db3f8`), attack and release 0..2 s (`DAT_144dec664`), the
/// output level -24..24 dB (`DAT_1457db4a8`, `DAT_145220f4c`).
pub const THRESHOLD_RANGE: (f64, f64) = (-96.3, 0.0);
pub const TIME_RANGE: (f64, f64) = (0.0, 2.0);
pub const OUTPUT_RANGE: (f64, f64) = (-24.0, 24.0);

/// `expf` as correctly rounded (E, the module's notes).
fn expf(x: f32) -> f32 {
    f64::from(x).exp() as f32
}

/// `powf(10, db x 0.05)`, correctly rounded (`SetParamsBlock` and `SetParam` make the output level a gain so).
pub fn output_gain(db: f32) -> f32 {
    f64::from(10.0f32).powf(f64::from(db * DB20)) as f32
}

/// `cvttss2si` into 64 bits: truncated toward 0; not a number or out of range gives 0x8000000000000000.
fn cvttss2si64(v: f32) -> i64 {
    if v.is_nan() || !(-9.223_372e18..9.223_372e18).contains(&v) { i64::MIN } else { v as i64 }
}

/// The linked routine's fast `10 log10(p)` (0x1449f4c00): m and e the mantissa (1..2) and exponent of p's bits.
pub fn fast_db(p: f32) -> f32 {
    let b = p.to_bits();
    let m = f32::from_bits((b & 0x007f_ffff) + 0x3f80_0000);
    let e = ((b >> 23) & 0xff) as f32;
    let m1 = m + ONE;
    let k = LOG_K / (m1 * m1 * m1);
    let m3 = m * m * m;
    k * (m3 - ONE) + (e - EXP_BIAS) * DB_PER_OCTAVE
}

/// The linked routine's fast `10^x` (0x1449f4c00): 0 for `x` under -37 (or not a number).
pub fn fast_gain(x: f32) -> f32 {
    if !(x >= POW_FLOOR) {
        return 0.0;
    }
    let i = cvttss2si64(x * POW_SCALE + POW_BIAS) as u32;
    let m = f32::from_bits((i & 0x007f_ffff) + 0x3f80_0000);
    ((m * POW_A + POW_B) * m + POW_C) * f32::from_bits(i & 0xff80_0000)
}

/// The gain of a compressor with nothing over its threshold (the fast power at 0: 0.99903899, -0.0084 dB).
pub fn rest_gain() -> f32 {
    fast_gain(0.0)
}

/// An attack's or release's coefficient (Init 0x1449f47d0, Execute 0x1449f46c0): `expf(-2.2 / (t x rate))`, a time of
/// 0.001 s or under taken as 0.001.
pub fn time_coef(t: f32) -> f32 {
    expf(TIME_K / ((if t <= MIN_TIME { MIN_TIME } else { t }) * RATE))
}

/// The detector's coefficient (Init: `expf(-43.0663223 / rate)`, a time constant of 23.2 ms).
pub fn detect_coef() -> f32 {
    expf(DETECT_K / RATE)
}

/// A compressor's parameters as the plug-in holds them: the threshold (dB), ratio, attack and release (s), the output
/// level as a gain (`powf` of the dB), and the two read only at Init.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorNow {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_s: f32,
    pub release_s: f32,
    pub output_gain: f32,
    pub process_lfe: bool,
    pub channel_link: bool,
}

impl CompressorNow {
    /// The block's values as `SetParamsBlock` (0x1449f5300) takes them: each as stored, the output level made a gain.
    pub fn of_block(c: &data::Compressor) -> CompressorNow {
        CompressorNow {
            threshold_db: c.threshold_db as f32,
            ratio: c.ratio as f32,
            attack_s: c.attack_s as f32,
            release_s: c.release_s as f32,
            output_gain: output_gain(c.output_level_db as f32),
            process_lfe: c.process_lfe,
            channel_link: c.channel_link,
        }
    }
}

/// What a compressor has done since it was made: frames through it, frames whose gain was under its gain at rest (it
/// reduced its bus), the least gain met (its gain at rest while it never reduced), blocks its guard silenced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorCounts {
    pub frames: u64,
    pub reduced: u64,
    pub least_gain: f32,
    pub guarded: u64,
}

impl CompressorCounts {
    /// The most it reduced, in dB under its gain at rest (0 if it never did).
    pub fn most_db(&self) -> f32 {
        if self.least_gain > 0.0 { 20.0 * (rest_gain() / self.least_gain).log10() } else { f32::INFINITY }
    }
}

/// One Wwise Compressor on a 12-channel bus, linked, as Elite runs it (the module's notes). Allocates nothing; never
/// panics.
#[derive(Clone, Debug)]
pub struct Compressor {
    /// The parameters handed last (Execute reads them at a buffer's start).
    now: CompressorNow,
    /// Init's: the channels counted and `1 / n`, the detector's coefficient, the times the coefficients were made for.
    counted: usize,
    inv_n: f32,
    detect: f32,
    attack_t: f32,
    attack: f32,
    release_t: f32,
    release: f32,
    /// The last buffer's output gain (+0x18).
    last_out: f32,
    /// The state (Reset zeroes it): the gain state (dB over) and the power.
    gain_db: f32,
    power: f32,
    /// The frame within the Wwise buffer, and this buffer's threshold, slope and output gain (its ramp's lanes).
    pos: u32,
    threshold: f32,
    slope: f32,
    ramp: bool,
    out: f32,
    lanes: [f32; 4],
    lane_step: f32,
    rest: f32,
    counts: CompressorCounts,
}

impl Compressor {
    /// Init then Reset, on a 7.1.4 bus (it has an LFE). An unlinked compressor is not built.
    pub fn new(p: CompressorNow) -> Result<Compressor, String> {
        if !p.channel_link {
            return Err("its channels are not linked (the unlinked routine 0x1449f4930 is not built)".into());
        }
        let counted = if p.process_lfe { CHANNELS } else { CHANNELS - 1 };
        let rest = rest_gain();
        let mut c = Compressor {
            now: p,
            counted,
            inv_n: ONE / counted as f32,
            detect: detect_coef(),
            attack_t: p.attack_s,
            attack: time_coef(p.attack_s),
            release_t: p.release_s,
            release: time_coef(p.release_s),
            last_out: p.output_gain,
            gain_db: 0.0,
            power: 0.0,
            pos: 0,
            threshold: p.threshold_db,
            slope: 0.0,
            ramp: false,
            out: p.output_gain,
            lanes: [0.0; 4],
            lane_step: 0.0,
            rest,
            counts: CompressorCounts { frames: 0, reduced: 0, least_gain: rest, guarded: 0 },
        };
        c.reset();
        Ok(c)
    }

    /// Reset (0x1449f4fe0): the gain state and the power at 0, a new buffer next.
    pub fn reset(&mut self) {
        self.gain_db = 0.0;
        self.power = 0.0;
        self.pos = 0;
    }

    /// New parameters, taken at the next buffer's start (Process LFE and Channel Link are Init's: ignored here).
    pub fn set(&mut self, p: CompressorNow) {
        self.now = p;
    }

    pub fn counts(&self) -> CompressorCounts {
        self.counts
    }

    /// The detector's level now (dB, the fast log of its power).
    pub fn level_db(&self) -> f32 {
        fast_db(self.power)
    }

    /// The coefficients in force: the attack's, the release's, the detector's.
    pub fn coefficients(&self) -> (f32, f32, f32) {
        (self.attack, self.release, self.detect)
    }

    /// Execute's work at a buffer's start: the coefficients of a changed time, the threshold and slope, the output gain's
    /// ramp from the last buffer's.
    fn begin(&mut self) {
        let p = self.now;
        if p.attack_s != self.attack_t {
            self.attack_t = p.attack_s;
            self.attack = time_coef(p.attack_s);
        }
        if p.release_s != self.release_t {
            self.release_t = p.release_s;
            self.release = time_coef(p.release_s);
        }
        self.threshold = p.threshold_db;
        self.slope = ONE / p.ratio - ONE;
        let (from, to) = (self.last_out, p.output_gain);
        self.ramp = to != from;
        self.out = from;
        if self.ramp {
            let d = (to - from) / BUFFER_FRAMES as f32;
            let one = d + from;
            let two = one + d;
            self.lanes = [from, one, two, two + d];
            self.lane_step = d * LANES;
        }
        self.last_out = to;
    }

    /// `frames` frames of the bus in place, its channels `stride` apart in `buf` (channel c at `c x stride`, the
    /// [`CHANNELS`] in their order); a Wwise buffer's start falls every [`BUFFER_FRAMES`] frames, wherever the calls cut.
    pub fn process(&mut self, buf: &mut [f32], stride: usize, frames: usize) {
        let frames = frames.min(stride);
        if buf.len() < self.counted.saturating_sub(1) * stride + frames {
            return;
        }
        let n = self.counted;
        for i in 0..frames {
            if self.pos == 0 {
                self.begin();
            }
            let mut sum = 0.0f32;
            for c in 0..n {
                let x = &mut buf[c * stride + i];
                if !x.is_finite() {
                    *x = 0.0;
                } else if x.abs() > INPUT_BOUND {
                    *x = INPUT_BOUND.copysign(*x);
                }
                sum += *x * *x;
            }
            let ms = sum * self.inv_n + FLOOR;
            self.power = (self.power - ms) * self.detect + ms;
            let level = fast_db(self.power) - self.threshold;
            let over = if level > 0.0 { level } else { 0.0 };
            let k = if over - self.gain_db >= 0.0 { self.attack } else { self.release };
            self.gain_db = (self.gain_db - over) * k + over;
            let gain = fast_gain(self.gain_db * self.slope * DB20);
            let lane = (self.pos % 4) as usize;
            let out = if self.ramp { self.lanes[lane] } else { self.out };
            let scale = self.ramp || out != ONE;
            for c in 0..n {
                let x = &mut buf[c * stride + i];
                *x *= gain;
                if scale {
                    *x *= out;
                }
            }
            if self.ramp && lane == 3 {
                let s = self.lane_step;
                self.lanes.iter_mut().for_each(|l| *l += s);
            }
            self.counts.frames += 1;
            if gain < self.rest {
                self.counts.reduced += 1;
                self.counts.least_gain = self.counts.least_gain.min(gain);
            }
            self.pos += 1;
            if self.pos == BUFFER_FRAMES {
                self.pos = 0;
            }
        }
        let bad = (0..n).any(|c| buf[c * stride..c * stride + frames].iter().any(|x| !x.is_finite()));
        if bad {
            for c in 0..n {
                buf[c * stride..c * stride + frames].fill(0.0);
            }
            self.gain_db = 0.0;
            self.power = 0.0;
            self.counts.guarded += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRIDE: usize = 128;

    /// The data's sub-mix compressor (87c72d00 on aa299971): -35.5 dB, 20:1, 1 ms, 0.2 s, 0 dB, LFE and link on.
    fn bus() -> CompressorNow {
        CompressorNow { threshold_db: -35.5, ratio: 20.0, attack_s: 0.001, release_s: 0.2, output_gain: 1.0, process_lfe: true, channel_link: true }
    }

    /// `secs` of a steady sine of amplitude `a` on the channels `on`, block by block; each block's samples handed to `f`.
    fn run(c: &mut Compressor, a: f32, on: &[usize], secs: f32, mut f: impl FnMut(&[f32], &Compressor)) {
        let frames = (secs * RATE) as usize;
        let mut buf = vec![0.0f32; CHANNELS * STRIDE];
        let mut t = 0usize;
        while t < frames {
            buf.fill(0.0);
            for i in 0..STRIDE {
                let x = a * (std::f32::consts::TAU * 1000.0 * (t + i) as f32 / RATE).sin();
                for &ch in on {
                    buf[ch * STRIDE + i] = x;
                }
            }
            c.process(&mut buf, STRIDE, STRIDE);
            f(&buf, c);
            t += STRIDE;
        }
    }

    /// After 2 s more of the signal: the gain its last frame took, measured as the output over the input of a frame of
    /// that block (the same multiply as the routine's), and checked against the state's.
    fn gain_now(c: &mut Compressor, a: f32, on: &[usize]) -> f32 {
        let mut last = Vec::new();
        run(c, a, on, 2.0, |b, _| last = b.to_vec());
        let frames = (2.0 * RATE) as usize;
        let t0 = frames.div_ceil(STRIDE) * STRIDE - STRIDE;
        let (i, x) = (0..STRIDE).map(|i| (i, a * (std::f32::consts::TAU * 1000.0 * (t0 + i) as f32 / RATE).sin())).fold((0, 0.0f32), |m, v| if v.1.abs() > m.1.abs() { v } else { m });
        let state = fast_gain(c.gain_db * c.slope * DB20);
        let measured = last[on[0] * STRIDE + i] / x;
        assert!((measured / state - 1.0).abs() < 0.01, "measured {measured}, the state's {state}");
        state
    }

    #[test]
    fn the_fast_level_and_power_follow_the_exact_ones() {
        for db in [-120.0f64, -60.0, -35.5, -20.0, -3.0, 0.0, 6.0] {
            let p = 10f64.powf(db / 10.0) as f32;
            assert!((f64::from(fast_db(p)) - db).abs() < 0.03, "{db} dB reads {}", fast_db(p));
        }
        for db in [-40.0f64, -20.0, -9.5, -1.0, 0.0, 3.0] {
            let want = 10f64.powf(db / 20.0);
            assert!((f64::from(fast_gain((db * 0.05) as f32)) / want - 1.0).abs() < 0.003, "{db} dB gives {}", fast_gain((db * 0.05) as f32));
        }
        assert_eq!(rest_gain(), 0.325_189_77 + 0.020_805_772 + 0.653_043_45);
        assert!((20.0 * rest_gain().log10() + 0.0084).abs() < 0.0002);
        assert_eq!(fast_gain(-37.5), 0.0);
        assert_eq!(fast_gain(f32::NAN), 0.0);
    }

    #[test]
    fn the_static_curve_at_under_and_over_the_threshold() {
        let rms = |db: f32| 10f32.powf(db / 20.0) * std::f32::consts::SQRT_2;
        let all: Vec<usize> = (0..CHANNELS).collect();
        // Under: the gain at rest exactly (nothing over, the gain state at 0).
        let mut c = Compressor::new(bus()).unwrap();
        run(&mut c, rms(-45.0), &all, 1.0, |_, _| {});
        let g = gain_now(&mut c, rms(-45.0), &all);
        assert!((g / rest_gain() - 1.0).abs() < 1e-5, "under: {g}");
        assert_eq!(c.counts().reduced, 0);
        // At the threshold: next to nothing (the fast level's error, a hundredth of a dB at most, times 0.95).
        let mut c = Compressor::new(bus()).unwrap();
        let g = gain_now(&mut c, rms(-35.5), &all);
        assert!(20.0 * (rest_gain() / g).log10() < 0.05, "at: {g}");
        // Over by 10 dB at 20:1: 9.5 dB down (the sine's mean square ripples a little through the 23 ms detector).
        let mut c = Compressor::new(bus()).unwrap();
        run(&mut c, rms(-25.5), &all, 1.0, |_, _| {});
        let g = gain_now(&mut c, rms(-25.5), &all);
        let down = 20.0 * (rest_gain() / g).log10();
        eprintln!("10 dB over at 20:1: {down:.3} dB down (exact 9.5)");
        assert!((down - 9.5).abs() < 0.15, "over by 10 dB: {down:.3} dB down");
        assert!(c.counts().reduced > 0 && (c.counts().most_db() - 9.5).abs() < 0.3, "{:?}", c.counts());
    }

    #[test]
    fn attack_and_release_take_elites_times() {
        let mut c = Compressor::new(bus()).unwrap();
        let (a, r, d) = c.coefficients();
        assert_eq!(a, (-2.2f64 / 48.0).exp() as f32);
        assert_eq!(r, (-2.2f64 / 9600.0).exp() as f32);
        assert_eq!(d, (f64::from(f32::from_bits(0xc22c_43ea)) / 48000.0).exp() as f32);
        assert_eq!(time_coef(0.0), time_coef(0.001), "a time under 1 ms is 1 ms");
        // A change of time takes effect at the next buffer.
        c.set(CompressorNow { attack_s: 0.01, ..bus() });
        run(&mut c, 0.0, &[0], 0.01, |_, _| {});
        assert_eq!(c.coefficients().0, time_coef(0.01));
        // Release: 20 dB over, then silence: the reduction falls to a tenth in ln(10) / 2.2 x 0.2 s = 0.209 s plus the
        // detector's own fall (its power down 20 dB in 0.107 s), so between 0.21 and 0.33 s.
        let mut c = Compressor::new(bus()).unwrap();
        let all: Vec<usize> = (0..CHANNELS).collect();
        run(&mut c, 10f32.powf(-15.5 / 20.0) * 1.414, &all, 1.0, |_, _| {});
        let start = c.gain_db;
        let mut t = 0.0f32;
        let mut tenth = None;
        run(&mut c, 0.0, &all, 0.6, |_, c| {
            t += STRIDE as f32 / RATE;
            if tenth.is_none() && c.gain_db < start * 0.1 {
                tenth = Some(t);
            }
        });
        let tenth = tenth.unwrap();
        eprintln!("release: the reduction a tenth after {tenth:.3} s");
        assert!((0.2..0.34).contains(&tenth), "a tenth after {tenth:.3} s");
        // Attack: from silence to 20 dB over, the gain state follows the detector within a few ms (1 ms attack).
        let mut c = Compressor::new(bus()).unwrap();
        let mut lag = 0.0f32;
        run(&mut c, 10f32.powf(-15.5 / 20.0) * 1.414, &all, 0.2, |_, c| {
            let over = (fast_db(c.power) - c.threshold).max(0.0);
            lag = lag.max(over - c.gain_db);
        });
        eprintln!("attack: the gain state lags its input by {lag:.3} dB at most");
        assert!(lag < 1.0, "the gain state lags its input by {lag:.2} dB at most");
    }

    #[test]
    fn the_detector_averages_the_power_over_the_bus_channels() {
        let all: Vec<usize> = (0..CHANNELS).collect();
        let a = 0.1f32;
        let mut one = Compressor::new(bus()).unwrap();
        run(&mut one, a, &[0], 1.0, |_, _| {});
        let mut every = Compressor::new(bus()).unwrap();
        run(&mut every, a, &all, 1.0, |_, _| {});
        let d = every.level_db() - one.level_db();
        eprintln!("one channel of 12 reads {d:.3} dB lower than all 12");
        assert!((d - 10.79).abs() < 0.05, "one channel of 12 reads {d:.3} dB lower");
        // Without the LFE counted (Process LFE off): one of 11.
        let mut eleven = Compressor::new(CompressorNow { process_lfe: false, ..bus() }).unwrap();
        run(&mut eleven, a, &[0], 1.0, |_, _| {});
        assert!((eleven.level_db() - one.level_db() - 0.378).abs() < 0.05);
        assert!(Compressor::new(CompressorNow { channel_link: false, ..bus() }).is_err());
    }

    #[test]
    fn silence_stays_silence_and_bad_input_is_held() {
        let mut c = Compressor::new(bus()).unwrap();
        let mut buf = vec![0.0f32; CHANNELS * STRIDE];
        c.process(&mut buf, STRIDE, STRIDE);
        assert!(buf.iter().all(|x| *x == 0.0));
        buf[3] = f32::NAN;
        buf[STRIDE + 5] = f32::INFINITY;
        buf[2 * STRIDE + 7] = 1e30;
        c.process(&mut buf, STRIDE, STRIDE);
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() <= INPUT_BOUND));
        assert_eq!(buf[3], 0.0);
        assert_eq!(c.counts().guarded, 0);
        // A huge level is cut hard and stays finite; the state comes back down from it.
        run(&mut c, 1.0e5, &[0, 1], 0.1, |b, _| assert!(b.iter().all(|x| x.is_finite())));
        run(&mut c, 0.0, &[0], 3.0, |_, _| {});
        assert!(c.gain_db < 1e-3, "released: {}", c.gain_db);
    }

    #[test]
    fn the_output_gain_ramps_over_a_buffer() {
        let mut c = Compressor::new(bus()).unwrap();
        c.set(CompressorNow { output_gain: 2.0, ..bus() });
        let mut buf = vec![1e-4f32; CHANNELS * STRIDE];
        c.process(&mut buf, STRIDE, STRIDE);
        let g0 = buf[0] / 1e-4 / rest_gain();
        assert!((g0 - 1.0).abs() < 1e-5, "starts from the last gain: {g0}");
        for _ in 1..8 {
            buf.fill(1e-4);
            c.process(&mut buf, STRIDE, STRIDE);
        }
        let end = buf[STRIDE - 1] / 1e-4 / rest_gain();
        assert!((end - (2.0 - 1.0 / 1024.0)).abs() < 1e-4, "ends a step short of the new gain: {end}");
        buf.fill(1e-4);
        c.process(&mut buf, STRIDE, STRIDE);
        assert!((buf[0] / 1e-4 / rest_gain() - 2.0).abs() < 1e-5);
    }
}
