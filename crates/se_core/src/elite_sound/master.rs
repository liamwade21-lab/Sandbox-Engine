//! The master bus's last plug-in, 0x00BA0003 (share set `4cc84cba`, the master bus `e2b7bc37`'s slot 3), as Elite's own
//! copy of it runs (`EliteDangerous64.exe`, build 2026.10.23.334925, decompiled offline into `re/elite/ghidra/dec/`; the
//! study T5f and the port T5d-8 in `re/elite/notes/sidewinder_sound_events.md`). It runs on the summed stereo output, after
//! every voice: it is not linear, so it can never be folded into the voices.
//!
//! - **Frame** (`Execute` 0x144af7f10 -> 0x144af8030 -> the engine 0x144af8720 -> 0x144afa510): Wwise's buffer cut into
//!   calls of at most 256 frames ([`CALL_FRAMES`]), interleaved; the engine is always initialised at 48,000 Hz
//!   (`FUN_144afa140` is handed 48000 and 256 whatever the device: [`ENGINE_RATE`]); the modules in the order of table
//!   0x145ee8420: the equaliser, the compressor, the channel gains, the limiter, each skipped when its block byte is 0
//!   (0x144af9670 makes the bypass mask, 0x144afa510 skips). The first call after a reset initialises every module (memory
//!   cleared, the compressor's and limiter's gains at 1) and takes the parameters ("new parameters": the compressor's power
//!   starts at the first input frame's square, the limiter's peak at its absolute value).
//! - **Equaliser** (`SetParams` 0x144afb110, design 0x144afe5b0 / 0x144afed20, run 0x144afaf30 / 0x144afe590, convert
//!   0x144afabf0): up to six biquads in a row ([`Biquad`]), each run two frames at a time by the two-channel branch of
//!   0x144afe590 (the frame pair's matrix of 0x144afe5b0, sums in the exe's order: [`Biquad::two`]). A band that designs to
//!   the identity is skipped (0x144afab70).
//! - **Compressor** (`SetParams` 0x144afc580, run 0x144afc270, a band 0x144b00300, convert 0x144afb470): the bands in
//!   parallel, each filtered from the module's input by the same biquads (Q 0.707, `DAT_1457ea8f0`: band 0 low-pass at the
//!   first crossover twice; a middle band low-pass at its upper crossover twice then high-pass at its lower twice; the last
//!   high-pass at its lower twice), compressed per channel, then summed in band order (0x144aff260).
//! - **Channel gains** (0x144afe450, run 0x144afe400 -> 0x144b02bd0): each channel times `10^(dB/20)` of its speaker (FL,
//!   FR on a stereo output: 0 dB in the data, a gain of exactly 1).
//! - **Limiter** (`SetParams` 0x144afdbc0, run 0x144afdaa0 -> 0x144afd080, convert 0x144afd5c0; mode 2, the only one
//!   built: the data's "Soft Limiter"): a 128-frame look-ahead, the peak over it, an exponential attack and a release
//!   coefficient, the make-up gain, a ceiling at threshold x make-up.
//!
//! The arithmetic is `f32` throughout, in the exe's order (each sum as its instructions add). The C library's `sinf`/`cosf`
//! (`FUN_144ca62d0`), `powf` (`FUN_1448b2470`) and `expf` (`FUN_1448c0530`) are taken as correctly rounded (worked out in
//! `f64` and rounded once: grade E, their results are used only when parameters are set). Ours (not Elite's): a sample in
//! that is not finite is 0 and one beyond [`INPUT_BOUND`] is held there (the voices' guards make neither reachable), and a
//! block whose output is not finite (never seen) is silenced and the chain started again.

use super::data::MasterSuite;

/// The engine's sample rate: `FUN_144af8720` initialises it with 48,000 Hz whatever the device.
pub const ENGINE_RATE: f32 = 48000.0;
/// Frames of one engine call (0x144af8030 cuts Wwise's buffer into calls of at most 256 frames).
pub const CALL_FRAMES: usize = 256;
/// Ours: the largest magnitude a sample in may have (beyond it is held there).
pub const INPUT_BOUND: f32 = 1.0e6;

const ONE: f32 = 1.0;
const HALF: f32 = 0.5;
/// `DAT_144def63c`, `DAT_144deaa60`, `DAT_1450cb4ac`, `DAT_1450bd22c`, `DAT_144de1654`.
const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
const MINUS_TWO: f32 = -2.0;
const SQRT2: f32 = f32::from_bits(0x3fb5_04f3);
const DB40: f32 = f32::from_bits(0x3ccc_cccd);
const DB20: f32 = f32::from_bits(0x3d4c_cccd);
/// A band's Q outside these is the identity (`DAT_144dee294`, `DAT_1450d2860`).
const Q_MIN: f32 = f32::from_bits(0x3dcc_cccd);
const Q_MAX: f32 = 12.0;
/// The crossovers' Q (`DAT_1457ea8f0`).
const CROSSOVER_Q: f32 = f32::from_bits(0x3f34_fdf4);
/// The time constants' numerator (`DAT_14538aae8`, -2.2) and the power smoother's (`DAT_1458004d0`, -440.00003).
const TIME_K: f32 = f32::from_bits(0xc00c_cccd);
const POWER_K: f32 = f32::from_bits(0xc3dc_0001);
/// The compressor's fast log and power of two (vectors at 0x145800620..0x1458006e0; masks copied at start-up from
/// 0x1458003c0..0x1458003e0 and 0x144def770).
const LOG_A: f32 = f32::from_bits(0xbeaa_aaab);
const LOG_B: f32 = 2.0;
const LOG_C: f32 = f32::from_bits(0xbfd5_5555);
const LOG10_2: f32 = f32::from_bits(0x3e9a_209b);
const TEN: f32 = 10.0;
const FLOOR_DB: f32 = -100.0;
const LOG2_10: f32 = f32::from_bits(0x4054_9a78);
const EXP_A: f32 = 0.25;
const EXP_B: f32 = 0.75;
/// Ours: a band counts as reducing a frame only when its gain is more than 0.01 dB under 1 (10^(-0.01/20)). The release
/// itself stops about 1.3e-4 short of 1 in `f32`, as Elite's does (`g (1 - rel) + rel` rounds back to the same value), so
/// "under 1" counted nearly every frame after the first reduction; the sound does not depend on this.
pub const REDUCED_GAIN: f32 = 0.998_849_4;
/// Ours (as the voices' filters, `super::filter`): a biquad's output and the compressor's power under this are 0, so a
/// tail decays to an exact 0 instead of through the subnormal floats, whose arithmetic is many times slower (in silence
/// 95,846 of 96,000 output samples were subnormal and the chain cost 3.9 ms a second of audio instead of 2.4). About
/// -300 dB: no sample of a signal in the normal range is touched. Whether Wwise flushes them was not read.
const FLUSH: f32 = 1e-15;

/// A value under [`FLUSH`] (in magnitude) made 0.
#[inline]
fn flush(v: f32) -> f32 {
    #[cfg(test)]
    if tests::NO_FLUSH.with(std::cell::Cell::get) {
        return v;
    }
    if v.abs() < FLUSH { 0.0 } else { v }
}

/// The compressor works its level out every this many frames of a call (`test bl, 0x1f`).
const LEVEL_EVERY: u64 = 32;
/// The limiter's look-ahead (frames), attack coefficient (`0x3d3b91cb`, fixed in mode 2) and the margin taken off its
/// target (`DAT_1454cf628`, 0.0026).
pub const LOOKAHEAD: usize = 128;
const LIMIT_ATTACK: f32 = f32::from_bits(0x3d3b_91cb);
const LIMIT_MARGIN: f32 = f32::from_bits(0x3b2a_64c3);
/// The convert routines' bounds: the equaliser's frequency 10..24000 Hz (`DAT_145800444`), gain -20..20 dB, Q 0.1..12;
/// the compressor's threshold -60..0, ratio 1..30, attack 0..0.5 s, release 0..5 s, make-up -18..18 dB, crossovers
/// 10..24000 Hz; the channel gains -100..12 dB (`DAT_145153b18`); the limiter's threshold -30..0 dB, release 0..5 s.
const FREQ: (f32, f32) = (10.0, 24000.0);

fn f32_of(v: f64) -> f32 {
    v as f32
}

/// The convert routines' clamp (`FUN_144aff240`): under `lo`, or not a number, is `lo`; else at most `hi`.
fn held(v: f32, lo: f32, hi: f32) -> f32 {
    if v >= lo { v.min(hi) } else { lo }
}

/// `powf` / `expf` / `sinf, cosf` as correctly rounded `f32` results (grade E, the module's notes).
fn powf(b: f32, e: f32) -> f32 {
    f64::from(b).powf(f64::from(e)) as f32
}

fn expf(x: f32) -> f32 {
    f64::from(x).exp() as f32
}

fn sincosf(x: f32) -> (f32, f32) {
    let (s, c) = f64::from(x).sin_cos();
    (s as f32, c as f32)
}

/// `1 - expf(-2.2 / (rate x t))`, or 1 for a time of 0 (the compressor's and limiter's attack and release).
fn time_coef(t: f32) -> f32 {
    if t != 0.0 { ONE - expf(TIME_K / (ENGINE_RATE * t)) } else { ONE }
}

/// One biquad as the engine keeps it (`FUN_144afe5b0`): `b0 b1 b2`, the feedback `a1 = -a1/a0`, `a2 = -a2/a0` (stored with
/// the sign for `y = b0 x + b1 x1 + b2 x2 + a1 y1 + a2 y2`), and the frame pair's matrix: `c1 = b0 a1 + b1`,
/// `c2 = b1 a1 + b2`, `c3 = a1 b2`, `c4 = a1 a1 + a2`, `c5 = a1 a2` (its two-channel block of 24 floats).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
    c1: f32,
    c2: f32,
    c3: f32,
    c4: f32,
    c5: f32,
}

/// One channel's memory of a biquad: the last two inputs and outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Mem {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// What a band of type 0, or one outside the design's range, is (0x144afe5b0's first branch).
    pub const IDENTITY: Biquad = Biquad::of(ONE, 0.0, 0.0, ONE, 0.0, 0.0);

    /// The section from `FUN_144afed20`'s raw coefficients (`a0 a1 a2 b0 b1 b2`), as 0x144afe5b0 stores it: each times
    /// `1 / a0`, the feedback negated, then the pair matrix.
    const fn of(a0: f32, a1: f32, a2: f32, b0: f32, b1: f32, b2: f32) -> Biquad {
        let inv = ONE / a0;
        let (b0, b1, b2, a1, a2) = (b0 * inv, b1 * inv, b2 * inv, -(a1 * inv), -(a2 * inv));
        Biquad { b0, b1, b2, a1, a2, c1: b0 * a1 + b1, c2: b1 * a1 + b2, c3: a1 * b2, c4: a1 * a1 + a2, c5: a1 * a2 }
    }

    /// The engine's design (0x144afe5b0, `FUN_144afed20`) of the types the data uses: 1 low-pass, 2 high-pass, 3 peaking,
    /// 4 high shelf; the identity for type 0, a frequency at or beyond half the engine's rate or 0 and under, a Q outside
    /// 0.1..12. `w = (f / 48000) x 2 pi`, `alpha = (0.5 / Q) sin w`; low-pass `b = ((1-c)/2, 1-c, (1-c)/2)`, high-pass
    /// `b = ((1+c)/2, -(1+c), (1+c)/2)`, both `a = (1+alpha, -2c, 1-alpha)`; peaking `A = sqrt(powf(10, g x 0.05))`,
    /// `b = (1 + alpha A, -2c, 1 - alpha A)`, `a = (1 + alpha/A, -2c, 1 - alpha/A)` (alpha alone for A = 0); high shelf
    /// (Q unused) `A = powf(10, g x 0.025)`, `t = sqrt(A) sqrt2 sin w`, `b0 = ((A-1)c + (A+1) + t) A`, `b1 = ((A+1)c + (A-1))
    /// A (-2)`, `b2 = ((A-1)c + (A+1) - t) A`, `a0 = (A+1) - (A-1)c + t`, `a1 = (A-1) - (A+1)c` doubled, `a2 = (A+1) - (A-1)c
    /// - t`. None for another type (not built: the data uses none).
    pub fn design(kind: u32, frequency: f32, q: f32, gain_db: f32) -> Option<Biquad> {
        let ratio = frequency / ENGINE_RATE;
        if kind == 0 || ratio <= 0.0 || HALF <= ratio || q < Q_MIN || Q_MAX < q {
            return matches!(kind, 0..=4).then_some(Biquad::IDENTITY);
        }
        let (s, c) = sincosf(ratio * TWO_PI);
        let alpha = HALF / q * s;
        let (a0, a1, a2, b0, b1, b2) = match kind {
            1 => {
                let b1 = ONE - c;
                let b0 = b1 * HALF;
                (alpha + ONE, c * MINUS_TWO, ONE - alpha, b0, b1, b0)
            }
            2 => {
                let t = c + ONE;
                let b0 = t * HALF;
                (alpha + ONE, c * MINUS_TWO, ONE - alpha, b0, -t, b0)
            }
            3 => {
                let a = powf(TEN, gain_db * DB20).sqrt();
                let r = if a != 0.0 { alpha / a } else { alpha };
                let m = a * alpha;
                (r + ONE, c * MINUS_TWO, ONE - r, m + ONE, c * MINUS_TWO, ONE - m)
            }
            4 => {
                let a = powf(TEN, gain_db * DB40);
                let (am1, ap1) = (a - ONE, a + ONE);
                let t = a.sqrt() * SQRT2 * s;
                let up = am1 * c + ap1;
                let down = ap1 - am1 * c;
                let sh = am1 - ap1 * c;
                ((down + t), sh + sh, down - t, (up + t) * a, (ap1 * c + am1) * a * MINUS_TWO, (up - t) * a)
            }
            _ => return None,
        };
        Some(Biquad::of(a0, a1, a2, b0, b1, b2))
    }

    /// Whether the band does anything (0x144afab70: anything but `b = 1, 0, 0` and no feedback).
    fn active(&self) -> bool {
        !(self.b0 == ONE && self.b1 == 0.0 && self.b2 == 0.0 && self.a1 == 0.0 && self.a2 == 0.0)
    }

    /// Two frames of one channel, as 0x144afe590's two-channel branch adds them (its lanes 0 and 1):
    /// `y0 = x0 b0 + x0 0 + x1' b1 + x2' b2 + y1' a1 + y2' a2`, `y1 = x0 c1 + x1 b0 + x1' c2 + x2' c3 + y1' c4 + y2' c5`; each
    /// under [`FLUSH`] made 0 (ours).
    #[inline]
    fn two(&self, m: &mut Mem, x0: f32, x1: f32) -> (f32, f32) {
        let y0 = flush(x0 * self.b0 + x0 * 0.0 + m.x1 * self.b1 + m.x2 * self.b2 + m.y1 * self.a1 + m.y2 * self.a2);
        let y1 = flush(x0 * self.c1 + x1 * self.b0 + m.x1 * self.c2 + m.x2 * self.c3 + m.y1 * self.c4 + m.y2 * self.c5);
        *m = Mem { x1, x2: x0, y1, y2: y0 };
        (y0, y1)
    }

    /// One channel of `frames` frames (`stride` 2, starting at `ch`) in place, a pair at a time. (An odd frame at the end,
    /// which the output's blocks never leave, takes the pair's first formula: ours.)
    fn run(&self, m: &mut Mem, buf: &mut [f32], ch: usize, frames: usize) {
        let mut i = 0;
        while i + 1 < frames {
            let (y0, y1) = self.two(m, buf[2 * i + ch], buf[2 * i + 2 + ch]);
            buf[2 * i + ch] = y0;
            buf[2 * i + 2 + ch] = y1;
            i += 2;
        }
        if i < frames {
            let x0 = buf[2 * i + ch];
            let y0 = flush(x0 * self.b0 + x0 * 0.0 + m.x1 * self.b1 + m.x2 * self.b2 + m.y1 * self.a1 + m.y2 * self.a2);
            *m = Mem { x1: x0, x2: m.x1, y1: y0, y2: m.y1 };
            buf[2 * i + ch] = y0;
        }
    }
}

/// The compressor's fast `10 log10(p)` (0x144b00300): -100 for a power under 0, else the mantissa `m` (1..2) and the
/// exponent `e` of its bits, `((m x -1/3 + 2) m - 5/3 + e) x log10(2) x 10`.
pub fn fast_db(p: f32) -> f32 {
    if p < 0.0 {
        return FLOOR_DB;
    }
    let b = p.to_bits();
    let m = f32::from_bits(b & 0x007f_ffff | 0x3f80_0000);
    let e = (((b >> 23) & 0xff) as i32 - 127) as f32;
    ((m * LOG_A + LOG_B) * m + LOG_C + e) * LOG10_2 * TEN
}

/// The compressor's fast `10^(g/20)` (0x144b00300): `x = g x 0.05 x log2(10)`, `i` its integer part (`cvttps2dq`: toward 0,
/// 0x80000000 out of range), `f = x - i`, the bits of `(f x 0.25 + 0.75) f + 1` plus `i << 23` (`paddd`).
pub fn fast_gain(g: f32) -> f32 {
    let x = g * DB20 * LOG2_10;
    let i = if x.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&x) { i32::MIN } else { x as i32 };
    let f = x - i as f32;
    let poly = (f * EXP_A + EXP_B) * f + ONE;
    f32::from_bits(poly.to_bits().wrapping_add((i as u32).wrapping_shl(23)))
}

/// One band of the compressor (0x144afc580's per band and channel fields, 0x144b00300's loop).
#[derive(Clone, Copy, Debug)]
struct CompBand {
    /// Its crossover filters, in order, and their memory a channel.
    filters: [Biquad; 4],
    filters_n: usize,
    mem: [[Mem; 2]; 4],
    /// Bypassed (0x144afc270 skips the compression; the band is still summed).
    off: bool,
    threshold: f32,
    slope: f32,
    knee: f32,
    knee_start: f32,
    att: f32,
    one_minus_att: f32,
    rel: f32,
    one_minus_rel: f32,
    makeup: f32,
    power_c: f32,
    one_minus_power_c: f32,
    /// Per channel: the smoothed power, the gain and the gain it moves to (worked out every 32nd frame).
    power: [f32; 2],
    gain: [f32; 2],
    target: [f32; 2],
}

impl CompBand {
    /// The static gain (dB) the band moves to at a level `l` (dB): 0 under the threshold (or the knee's start), else the
    /// ratio's slope over it, the knee's quadratic inside the knee.
    fn gain_db(&self, l: f32) -> f32 {
        let d = l - self.knee_start;
        let q = d / self.knee;
        if self.knee <= d || self.knee == 0.0 {
            if self.threshold < l { (self.threshold - l) * self.slope } else { 0.0 }
        } else if self.knee_start < l {
            ((ONE - q * (HALF * self.slope)) * d + self.knee_start) - l
        } else {
            0.0
        }
    }

    /// The gain's step towards its target (the attack's coefficient going down, the release's going up).
    #[inline]
    fn smooth(&self, g: f32, t: f32) -> f32 {
        if t < g { t * self.att + g * self.one_minus_att } else { g * self.one_minus_rel + t * self.rel }
    }

    /// The band's compression of its filtered signal in place (stereo, interleaved), from frame number `start` (the level
    /// is worked out on frames whose number is a multiple of 32: the frame within the engine's call). `reduced[i]` is set
    /// where its gain is under [`REDUCED_GAIN`] on either channel; `least` is its least gain met. The power under
    /// [`FLUSH`] is 0 (ours).
    fn compress(&mut self, buf: &mut [f32], frames: usize, start: u64, reduced: &mut [bool], least: &mut f32) {
        for (i, r) in reduced.iter_mut().enumerate().take(frames) {
            let level = (start + i as u64) % LEVEL_EVERY == 0;
            for ch in 0..2 {
                let x = buf[2 * i + ch];
                self.power[ch] = flush(x * x * self.power_c + self.power[ch] * self.one_minus_power_c);
                if level {
                    self.target[ch] = fast_gain(self.gain_db(fast_db(self.power[ch])));
                }
                self.gain[ch] = self.smooth(self.gain[ch], self.target[ch]);
                *r |= self.gain[ch] < REDUCED_GAIN;
                *least = least.min(self.gain[ch]);
                buf[2 * i + ch] = x * (self.gain[ch] * self.makeup);
            }
        }
    }
}

/// The limiter in mode 2 (0x144afdbc0, 0x144afd080).
#[derive(Clone, Debug)]
struct Limiter {
    threshold: f32,
    makeup: f32,
    rel: f32,
    /// Per channel: the peak over the look-ahead, the gain; the look-ahead's samples.
    peak: [f32; 2],
    gain: [f32; 2],
    delay: [[f32; 2]; LOOKAHEAD],
}

impl Limiter {
    /// One frame (both channels), its look-ahead slot `slot`. True if a channel's target was under 1 (the peak in its
    /// window over the threshold).
    fn frame(&mut self, f: &mut [f32], slot: usize) -> bool {
        let mut over = false;
        let mut target = [ONE; 2];
        for (ch, t) in target.iter_mut().enumerate() {
            let a = f[ch].abs();
            let m = self.peak[ch];
            *t = if m > a {
                if m == self.delay[slot][ch].abs() {
                    // The peak leaves the window: the largest of the other slots and the new sample.
                    let mut p = 0.0f32;
                    for k in (slot + 1..LOOKAHEAD).chain(0..slot) {
                        let d = self.delay[k][ch].abs();
                        if p < d {
                            p = d;
                        }
                    }
                    if p < a {
                        p = a;
                    }
                    self.peak[ch] = p;
                }
                let t = self.threshold / self.peak[ch] - LIMIT_MARGIN;
                if t <= ONE { t } else { ONE }
            } else {
                self.peak[ch] = a;
                let t = self.threshold / a - LIMIT_MARGIN;
                if t < ONE || t.is_nan() { t } else { ONE }
            };
            over |= *t < ONE;
        }
        let ceiling = self.threshold * self.makeup;
        for (ch, t) in target.into_iter().enumerate() {
            let g = self.gain[ch];
            let coef = if t < g { Some(LIMIT_ATTACK) } else if g < t { Some(self.rel) } else { None };
            if let Some(k) = coef {
                self.gain[ch] = (ONE - k) * g + k * t;
            }
            let d = self.delay[slot][ch];
            self.delay[slot][ch] = f[ch];
            let y = d * self.gain[ch] * self.makeup;
            f[ch] = if y.abs() > ceiling { (ceiling / y).abs() * y } else { y };
        }
        over
    }
}

/// What the chain has done since it was made: frames (stereo) through it, frames where a compressor band's gain was more
/// than 0.01 dB under 1 ([`REDUCED_GAIN`]), frames where the limiter's target was under 1, blocks silenced by the guard,
/// and each compressor band's least gain (1 while it never went under).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterCounts {
    pub frames: u64,
    pub compressed: u64,
    pub limited: u64,
    pub guarded: u64,
    pub least_gain: [f32; 4],
}

impl Default for MasterCounts {
    fn default() -> MasterCounts {
        MasterCounts { frames: 0, compressed: 0, limited: 0, guarded: 0, least_gain: [ONE; 4] }
    }
}

impl MasterCounts {
    /// Each band's largest reduction (dB under 1; 0 if none).
    pub fn most_db(&self) -> [f32; 4] {
        self.least_gain.map(|g| if g > 0.0 { -20.0 * g.min(ONE).log10() } else { f32::INFINITY })
    }
}

/// The whole plug-in on a stereo output ([`MasterChain::process`]). Everything is made by [`MasterChain::new`]; processing
/// allocates nothing.
#[derive(Clone, Debug)]
pub struct MasterChain {
    modules_on: [bool; 4],
    eq: [Biquad; 6],
    eq_n: usize,
    eq_mem: [[Mem; 2]; 6],
    bands: [CompBand; 4],
    bands_n: usize,
    gains: [f32; 2],
    limiter: Limiter,
    /// The first call after a reset takes the parameters (the compressor's power and the limiter's peak from its first
    /// frame).
    fresh: bool,
    /// Frames processed since the reset (the compressor's 32-frame cadence and the limiter's slot).
    clock: u64,
    /// The compressor's input and one band's signal (one call's frames).
    input: [f32; 2 * CALL_FRAMES],
    band: [f32; 2 * CALL_FRAMES],
    reduced: [bool; CALL_FRAMES],
    counts: MasterCounts,
}

impl MasterChain {
    /// The chain for a parameter block, its values held by the modules' convert routines; an error for what they refuse or
    /// the chain does not build (a channel link, a limiter mode other than 2, an equaliser type the data does not use).
    pub fn new(s: &MasterSuite) -> Result<MasterChain, String> {
        if !(1..=6).contains(&s.eq_bands_n) || s.eq.len() < s.eq_bands_n as usize {
            return Err(format!("the master's equaliser has {} bands (1-6 accepted)", s.eq_bands_n));
        }
        if !(1..=4).contains(&s.comp_bands_n) || s.comp.len() < s.comp_bands_n as usize {
            return Err(format!("the master's compressor has {} bands (1-4 accepted)", s.comp_bands_n));
        }
        if s.comp_link != 0 {
            return Err(format!("the master's compressor links its channels (mode {}): not built", s.comp_link));
        }
        if s.limiter_mode != 2 || s.limiter_link {
            return Err(format!("the master's limiter in mode {} (link {}): only mode 2 unlinked is built", s.limiter_mode, s.limiter_link));
        }
        let mut eq = [Biquad::IDENTITY; 6];
        for (k, b) in s.eq.iter().take(s.eq_bands_n as usize).enumerate() {
            // (0x144af9670: a band whose on byte is 0 has its type set to 0.)
            let kind = if b.on { b.kind } else { 0 };
            let (f, g, q) = (held(f32_of(b.frequency), FREQ.0, FREQ.1), held(f32_of(b.gain_db), -20.0, 20.0), held(f32_of(b.q), Q_MIN, Q_MAX));
            eq[k] = Biquad::design(kind, f, q, g).ok_or_else(|| format!("the master's equaliser band {k} has type {kind}: not built"))?;
        }
        let mut xo = [0.0f32; 3];
        for (k, c) in s.crossovers.iter().enumerate().take(s.comp_bands_n as usize - 1) {
            xo[k] = held(f32_of(*c), FREQ.0, FREQ.1);
            if k > 0 && xo[k] <= xo[k - 1] {
                return Err("the master's crossovers do not rise".into());
            }
        }
        let n = s.comp_bands_n as usize;
        let power_c = ONE - expf(POWER_K / ENGINE_RATE);
        let mut bands = [CompBand {
            filters: [Biquad::IDENTITY; 4],
            filters_n: 0,
            mem: Default::default(),
            off: false,
            threshold: -20.0,
            slope: 0.5,
            knee: 0.0,
            knee_start: -20.0,
            att: ONE,
            one_minus_att: 0.0,
            rel: ONE,
            one_minus_rel: 0.0,
            makeup: ONE,
            power_c,
            one_minus_power_c: ONE - power_c,
            power: [0.0; 2],
            gain: [ONE; 2],
            target: [ONE; 2],
        }; 4];
        for (k, (band, p)) in bands.iter_mut().zip(&s.comp).take(n).enumerate() {
            let lp = |f| Biquad::design(1, f, CROSSOVER_Q, 0.0).unwrap_or(Biquad::IDENTITY);
            let hp = |f| Biquad::design(2, f, CROSSOVER_Q, 0.0).unwrap_or(Biquad::IDENTITY);
            band.filters_n = match (n, k) {
                (1, _) => 0,
                (_, 0) => {
                    band.filters[..2].copy_from_slice(&[lp(xo[0]); 2]);
                    2
                }
                (_, k) if k == n - 1 => {
                    band.filters[..2].copy_from_slice(&[hp(xo[k - 1]); 2]);
                    2
                }
                (_, k) => {
                    band.filters = [lp(xo[k]), lp(xo[k]), hp(xo[k - 1]), hp(xo[k - 1])];
                    4
                }
            };
            band.off = !s.comp_bands_on[k];
            band.threshold = held(f32_of(p.threshold_db), -60.0, 0.0);
            let ratio = held(f32_of(p.ratio), ONE, 30.0);
            let (att, rel) = (held(f32_of(p.attack_s), 0.0, HALF), held(f32_of(p.release_s), 0.0, 5.0));
            let makeup = held(f32_of(p.makeup_db), -18.0, 18.0);
            let knee = held(f32_of(p.knee_db), 0.0, 20.0);
            let most = (band.threshold + band.threshold).abs();
            band.knee = if most < knee { most } else { knee };
            band.slope = ONE - ONE / ratio;
            band.att = time_coef(att);
            band.one_minus_att = ONE - band.att;
            band.rel = time_coef(rel);
            band.one_minus_rel = ONE - band.rel;
            band.makeup = powf(TEN, makeup * DB20);
            band.knee_start = band.threshold - band.knee * HALF;
        }
        let gains = [0, 1].map(|ch| powf(TEN, held(f32_of(s.channel_gains_db[ch]), -100.0, 12.0) * DB20));
        let threshold_db = held(f32_of(s.limiter_threshold_db), -30.0, 0.0);
        let makeup_db = held(f32_of(s.limiter_makeup_db), 0.0, -threshold_db);
        let limiter = Limiter {
            threshold: powf(TEN, threshold_db * DB20),
            makeup: powf(TEN, makeup_db * DB20),
            rel: time_coef(held(f32_of(s.limiter_release_s), 0.0, 5.0)),
            peak: [0.0; 2],
            gain: [ONE; 2],
            delay: [[0.0; 2]; LOOKAHEAD],
        };
        Ok(MasterChain {
            modules_on: s.modules_on,
            eq,
            eq_n: s.eq_bands_n as usize,
            eq_mem: Default::default(),
            bands,
            bands_n: n,
            gains,
            limiter,
            fresh: true,
            clock: 0,
            input: [0.0; 2 * CALL_FRAMES],
            band: [0.0; 2 * CALL_FRAMES],
            reduced: [false; CALL_FRAMES],
            counts: MasterCounts::default(),
        })
    }

    /// Back to its state when made (the engine's initialisation, `FUN_144afa140` and the modules' own: memory cleared, the
    /// compressor's and limiter's gains at 1, the limiter's peak 0), the next call taking the parameters. Allocates
    /// nothing; the counts are kept.
    pub fn reset(&mut self) {
        self.eq_mem = Default::default();
        for b in &mut self.bands {
            b.mem = Default::default();
            b.power = [0.0; 2];
            b.gain = [ONE; 2];
            b.target = [ONE; 2];
        }
        let l = &mut self.limiter;
        l.peak = [0.0; 2];
        l.gain = [ONE; 2];
        l.delay = [[0.0; 2]; LOOKAHEAD];
        self.fresh = true;
        self.clock = 0;
    }

    pub fn counts(&self) -> MasterCounts {
        self.counts
    }

    /// The chain's delay (frames): the limiter's look-ahead when it is on.
    pub fn latency(&self) -> usize {
        if self.modules_on[3] { LOOKAHEAD } else { 0 }
    }

    /// Stereo frames, interleaved, in place, an engine call (at most 256 frames) at a time. A block of the output (128
    /// frames) is half a call: the arithmetic is the same whatever the cut, but for the compressor's 32-frame cadence,
    /// which counts frames from the reset (a multiple of 32 at each cut keeps it Elite's).
    pub fn process(&mut self, buf: &mut [f32]) {
        for call in buf.chunks_mut(2 * CALL_FRAMES) {
            let frames = call.len() / 2;
            let call = &mut call[..2 * frames];
            for x in call.iter_mut() {
                *x = if x.is_finite() { x.clamp(-INPUT_BOUND, INPUT_BOUND) } else { 0.0 };
            }
            self.call(call, frames);
            if !call.iter().all(|x| x.is_finite()) {
                call.fill(0.0);
                self.reset();
                self.counts.guarded += 1;
            }
            self.clock += frames as u64;
            self.counts.frames += frames as u64;
        }
    }

    fn call(&mut self, buf: &mut [f32], frames: usize) {
        if self.modules_on[0] {
            for (b, mem) in self.eq.iter().zip(self.eq_mem.iter_mut()).take(self.eq_n) {
                if b.active() {
                    for (ch, m) in mem.iter_mut().enumerate() {
                        b.run(m, buf, ch, frames);
                    }
                }
            }
        }
        if self.modules_on[1] {
            self.compress(buf, frames);
            self.counts.compressed += self.reduced[..frames].iter().filter(|r| **r).count() as u64;
        }
        if self.modules_on[2] {
            for f in buf.chunks_exact_mut(2) {
                f[0] *= self.gains[0];
                f[1] *= self.gains[1];
            }
        }
        if self.modules_on[3] {
            if self.fresh {
                self.limiter.peak = [buf[0].abs(), buf[1].abs()];
            }
            let mut limited = 0;
            for (i, f) in buf.chunks_exact_mut(2).enumerate() {
                let slot = ((self.clock + i as u64) % LOOKAHEAD as u64) as usize;
                limited += u64::from(self.limiter.frame(f, slot));
            }
            self.counts.limited += limited;
        }
        self.fresh = false;
    }

    /// The compressor over one call: each band from the module's input through its filters, compressed, the bands summed in
    /// order; `reduced` marks the frames where a band's gain was under [`REDUCED_GAIN`].
    fn compress(&mut self, buf: &mut [f32], frames: usize) {
        let n = 2 * frames;
        self.input[..n].copy_from_slice(&buf[..n]);
        if self.fresh {
            for b in &mut self.bands[..self.bands_n] {
                b.power = [buf[0] * buf[0], buf[1] * buf[1]];
            }
        }
        self.reduced.fill(false);
        for k in 0..self.bands_n {
            self.band[..n].copy_from_slice(&self.input[..n]);
            let band = &mut self.bands[k];
            for (f, mem) in band.filters.iter().zip(band.mem.iter_mut()).take(band.filters_n) {
                for (ch, m) in mem.iter_mut().enumerate() {
                    f.run(m, &mut self.band[..n], ch, frames);
                }
            }
            if !band.off {
                band.compress(&mut self.band[..n], frames, self.clock, &mut self.reduced, &mut self.counts.least_gain[k]);
            }
            if k == 0 {
                buf[..n].copy_from_slice(&self.band[..n]);
            } else {
                for (o, b) in buf[..n].iter_mut().zip(&self.band[..n]) {
                    *o += *b;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elite_sound::data::{MasterCompBand, MasterEqBand};

    /// Elite's block (share set `4cc84cba`, decoded by the converter as 0x144af9670 reads it).
    fn elite() -> MasterSuite {
        let band = |kind, frequency, gain_db, q| MasterEqBand { kind, frequency, gain_db, q, on: true };
        let comp = |t, r, a, rl, m, k| MasterCompBand { threshold_db: t, ratio: r, attack_s: a, release_s: rl, makeup_db: m, knee_db: k };
        MasterSuite {
            modules_on: [true; 4],
            eq_bands_n: 6,
            eq: vec![
                band(2, 50.0, -0.6134970188140869, 1.0199999809265137),
                band(3, 300.0, -1.7000000476837158, 1.5),
                band(3, 150.0, -1.25, 1.0),
                band(3, 600.0, 1.0, 1.0),
                band(3, 1500.0, -1.5, 0.800000011920929),
                band(4, 1500.0, 1.0, 1.0),
            ],
            comp_bands_n: 4,
            comp_link: 0,
            comp_link_amount: 0.0,
            comp_link_flag: false,
            comp_bands_on: [true; 4],
            crossovers: [141.66099548339844, 423.6636047363281, 3986.47900390625],
            comp: vec![
                comp(-18.0, 2.0, 0.04f32 as f64, 0.2f32 as f64, 1.0, 0.5),
                comp(-23.0, 1.5, 0.05f32 as f64, 0.14f32 as f64, 0.0, 0.0),
                comp(-14.0, 1.5, 0.01f32 as f64, 0.1f32 as f64, 1.0, 0.0),
                comp(-23.0, 1.5, 0.01f32 as f64, 0.1f32 as f64, 1.0, 0.0),
            ],
            channel_gains_db: [0.0, 0.0, 1.0, -3.0, -2.0, -2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            limiter_mode: 2,
            limiter_threshold_db: -14.0,
            limiter_attack_s: 0.0,
            limiter_release_s: 0.0,
            limiter_makeup_db: 14.0,
            limiter_link: false,
        }
    }

    fn only(module: usize) -> MasterChain {
        let mut s = elite();
        s.modules_on = [false; 4];
        s.modules_on[module] = true;
        MasterChain::new(&s).unwrap()
    }

    /// A biquad's gain (dB) at `hz` from its stored coefficients, worked out in `f64`.
    fn response_db(b: &Biquad, hz: f64) -> f64 {
        let w = std::f64::consts::TAU * hz / f64::from(ENGINE_RATE);
        let z1 = (w.cos(), -w.sin());
        let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
        let (b0, b1, b2, a1, a2) = (f64::from(b.b0), f64::from(b.b1), f64::from(b.b2), f64::from(b.a1), f64::from(b.a2));
        let num = (b0 + b1 * z1.0 + b2 * z2.0, b1 * z1.1 + b2 * z2.1);
        let den = (1.0 - a1 * z1.0 - a2 * z2.0, -a1 * z1.1 - a2 * z2.1);
        10.0 * ((num.0 * num.0 + num.1 * num.1) / (den.0 * den.0 + den.1 * den.1)).log10()
    }

    /// The steady gain (dB) of a sine at `hz` through a stereo processing step (left channel; the first second dropped).
    fn measured_db(hz: f64, amp: f32, mut step: impl FnMut(&mut [f32])) -> f64 {
        let n = 3 * ENGINE_RATE as usize;
        let mut buf: Vec<f32> = (0..n).flat_map(|i| {
            let x = amp * (std::f64::consts::TAU * hz * i as f64 / f64::from(ENGINE_RATE)).sin() as f32;
            [x, x]
        }).collect();
        for b in buf.chunks_mut(2 * 128) {
            step(b);
        }
        let skip = ENGINE_RATE as usize;
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for i in skip..n {
            a += f64::from(buf[2 * i]).powi(2);
            b += (f64::from(amp) * (std::f64::consts::TAU * hz * i as f64 / f64::from(ENGINE_RATE)).sin()).powi(2);
        }
        10.0 * (a / b).log10()
    }

    /// The equaliser's six bands at their centre frequencies and an octave either side: the measured gain of the pair form
    /// is the stored coefficients' response, and that is the band's own shape (a peak's gain at its centre, the high-pass's
    /// 20 log10 Q at its corner, the shelf's half gain at its frequency).
    #[test]
    fn equaliser_bands_respond_as_their_formulas() {
        let s = elite();
        for (k, band) in s.eq.iter().enumerate() {
            let b = Biquad::design(band.kind, band.frequency as f32, band.q as f32, band.gain_db as f32).unwrap();
            for hz in [band.frequency / 2.0, band.frequency, band.frequency * 2.0] {
                let mut m = [Mem::default(); 2];
                let got = measured_db(hz, 0.1, |buf| {
                    let frames = buf.len() / 2;
                    for (ch, m) in m.iter_mut().enumerate() {
                        b.run(m, buf, ch, frames);
                    }
                });
                // (Within 0.05 dB: the pair's second frame goes by the matrix's own f32 products, a hair off the stored
                // coefficients where the poles are near the unit circle, as in the exe.)
                let want = response_db(&b, hz);
                assert!((got - want).abs() < 0.05, "band {k} at {hz} Hz: {got:.4} dB measured, {want:.4} dB by its coefficients");
            }
            let at = response_db(&b, band.frequency);
            let shape = match band.kind {
                2 => 20.0 * band.q.log10(),
                3 => band.gain_db,
                _ => band.gain_db / 2.0,
            };
            assert!((at - shape).abs() < 0.02, "band {k}: {at:.4} dB at {} Hz, its shape says {shape:.4}", band.frequency);
        }
        // An octave under the high-pass: the second-order shape r^2 / sqrt((1 - r^2)^2 + (r / Q)^2) at r = 1/2.
        let hp = Biquad::design(2, 50.0, 1.02, 0.0).unwrap();
        let want = 20.0 * (0.25 / (0.5625f64 + (0.5 / 1.02f64).powi(2)).sqrt()).log10();
        assert!((response_db(&hp, 25.0) - want).abs() < 0.05, "{} against {want}", response_db(&hp, 25.0));
    }

    /// The frame pair's matrix computes the same filter as a plain direct form (to f32 rounding).
    #[test]
    fn the_pair_form_is_the_biquad() {
        let b = Biquad::design(3, 300.0, 1.5, -1.7).unwrap();
        let mut m = Mem::default();
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut buf: Vec<f32> = (0..4000).flat_map(|i| [((i * 7919) % 1000) as f32 / 1000.0 - 0.5, 0.0]).collect();
        let x: Vec<f32> = buf.iter().step_by(2).copied().collect();
        b.run(&mut m, &mut buf, 0, 4000);
        for (i, xi) in x.iter().enumerate() {
            let xi = f64::from(*xi);
            let y = f64::from(b.b0) * xi + f64::from(b.b1) * x1 + f64::from(b.b2) * x2 + f64::from(b.a1) * y1 + f64::from(b.a2) * y2;
            (x2, x1, y2, y1) = (x1, xi, y1, y);
            assert!((y - f64::from(buf[2 * i])).abs() < 1e-5, "frame {i}");
        }
    }

    thread_local! {
        /// The flush switched off on this thread (to compare with the chain as it was).
        pub(super) static NO_FLUSH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// The flush of tails (ours): a normal signal (a 1 kHz tone at -20 dB, -40 dB noise, 2 s, loud enough for the
    /// compressor and the limiter) gives the same output bits with it and without it; after a loud burst, silence in gives
    /// exact zeros out once the tails have died (within 1 s), and no subnormal sample on the way, where without the flush
    /// most of the silence comes out subnormal.
    #[test]
    fn tails_end_in_exact_zeros_and_normal_signals_are_untouched() {
        let run = |input: &[f32], no_flush: bool| {
            NO_FLUSH.with(|f| f.set(no_flush));
            let mut c = MasterChain::new(&elite()).unwrap();
            let mut b = input.to_vec();
            for part in b.chunks_mut(2 * 128) {
                c.process(part);
            }
            NO_FLUSH.with(|f| f.set(false));
            b
        };
        let signal: Vec<f32> = (0..2 * 96_000)
            .map(|i| {
                let t = (i / 2) as f32 / 48_000.0;
                0.1 * (std::f32::consts::TAU * 1000.0 * t).sin() + 0.01 * ((((i * 7919) % 2000) as f32 / 1000.0) - 1.0)
            })
            .collect();
        let (with, without) = (run(&signal, false), run(&signal, true));
        assert!(with.iter().zip(&without).all(|(a, b)| a.to_bits() == b.to_bits()), "a normal signal's output changed");
        let mut burst = signal[..2 * 24_000].to_vec();
        burst.extend(std::iter::repeat_n(0.0f32, 2 * 96_000));
        let (with, without) = (run(&burst, false), run(&burst, true));
        let sub = |v: &[f32]| v.iter().filter(|x| x.is_subnormal()).count();
        let tail = &with[2 * (24_000 + 48_000)..];
        eprintln!("silence after a burst: {} subnormal samples with the flush, {} without; the last second all 0: {}", sub(&with), sub(&without), tail.iter().all(|x| *x == 0.0));
        assert_eq!(sub(&with), 0);
        assert!(tail.iter().all(|x| x.to_bits() == 0), "silence in, exact zeros out after the tails");
    }

    /// Each band's static curve: under its threshold the band passes with its make-up; over it the ratio's slope, within
    /// the fast power's own error (up to 0.25 dB: its quadratic for 2^f), and exactly the exe's formula
    /// (`fast_gain(gain_db(fast_db(p)))` times the make-up) once the power has settled; band 0's knee bends it between.
    #[test]
    fn compressor_bands_follow_their_static_curve() {
        let c = only(1);
        for (k, band) in c.bands.iter().take(4).enumerate() {
            let mut b = *band;
            b.filters_n = 0;
            let (t, r) = (f64::from(b.threshold), 1.0 / (1.0 - f64::from(b.slope)));
            for l in [t - 10.0, t + 6.0, t + 12.0] {
                let x = 10f64.powf(l / 20.0) as f32;
                let mut buf = vec![x; 2 * 48000];
                let mut reduced = vec![false; 48000];
                b.compress(&mut buf, 48000, 0, &mut reduced, &mut 1.0);
                let out = 20.0 * f64::from(buf[2 * 47999]).abs().log10();
                let want = if l < t { l } else { t + (l - t) / r } + 20.0 * f64::from(b.makeup).log10();
                assert!((out - want).abs() < 0.3, "band {k} at {l} dB: {out:.3} dB out, the curve says {want:.3}");
                let exact = x * (fast_gain(b.gain_db(fast_db(b.power[0]))) * b.makeup);
                assert!((buf[2 * 47999] - exact).abs() <= 1e-4 * exact.abs(), "band {k} at {l} dB: {} against the formula's {exact}", buf[2 * 47999]);
                assert_eq!(reduced[47999], l > t, "band {k} at {l} dB");
            }
        }
        // Band 0's knee (0.5 dB): at its threshold the gain is the knee's quadratic, between 0 and the slope's.
        let b = c.bands[0];
        let mid = b.gain_db(b.threshold);
        assert!((mid + b.slope * b.knee / 8.0).abs() < 1e-5, "{mid}: the quadratic gives -s W / 8 at the threshold");
        assert_eq!(b.gain_db(b.threshold - 1.0), 0.0);
    }

    /// The fast log and power: within 0.05 dB and 0.3 dB of the exact ones (the quadratic for 2^f errs up to 0.25 dB).
    #[test]
    fn fast_log_and_power_are_close() {
        for db in [-90.0f32, -40.0, -23.0, -18.0, -6.0, 0.0, 6.0] {
            let p = 10f32.powf(db / 10.0);
            assert!((fast_db(p) - db).abs() < 0.05, "{db}: {}", fast_db(p));
            let g = fast_gain(db);
            assert!((20.0 * g.log10() - db).abs() < 0.3, "{db}: {g}");
        }
        assert_eq!(fast_gain(0.0), 1.0);
        assert_eq!(fast_db(-1.0), FLOOR_DB);
        assert!(fast_gain(-400.0).is_finite() && fast_gain(-400.0) >= 0.0);
    }

    /// The attack and release: the gain covers all but e^-2.2 (89 %) of its way in the band's time (2.2 time constants, the
    /// coefficient `1 - exp(-2.2 / (48000 t))`), each per frame.
    #[test]
    fn compressor_attack_and_release_take_their_times() {
        let c = only(1);
        let s = elite();
        for (b, p) in c.bands.iter().zip(&s.comp) {
            for (down, secs) in [(true, p.attack_s), (false, p.release_s)] {
                let (from, to) = if down { (1.0f32, 0.5f32) } else { (0.5, 1.0) };
                let mut g = from;
                let n = (secs as f32 * ENGINE_RATE).round() as usize;
                for _ in 0..n {
                    g = b.smooth(g, to);
                }
                let left = f64::from((g - to) / (from - to));
                assert!((left - (-2.2f64).exp()).abs() < 2e-3, "{secs} s ({}): {left} left", if down { "attack" } else { "release" });
            }
        }
    }

    /// The limiter: under its threshold a plain +14 dB (after its 128-frame delay), at and over it held to the ceiling
    /// (threshold x make-up, 0 dBFS) less its 0.0026 margin.
    #[test]
    fn limiter_below_at_and_over_its_threshold() {
        let thr = 10f32.powf(-0.7);
        let makeup = 10f32.powf(0.7);
        for (a, limited) in [(0.1f32, false), (thr, true), (0.25, true)] {
            let mut c = only(3);
            let mut buf = vec![a; 2 * 4800];
            c.process(&mut buf);
            assert!(buf[..2 * LOOKAHEAD].iter().all(|y| *y == 0.0), "the look-ahead's delay");
            let y = buf[2 * 4799];
            let want = if limited { (thr / a - LIMIT_MARGIN) * a * makeup } else { a * makeup };
            assert!((y - want).abs() < 1e-5, "{a}: {y} out, {want} wanted");
            assert_eq!(c.counts().limited > 0, limited, "{a}");
        }
    }

    /// Silence in, silence out (the whole plug-in).
    #[test]
    fn silence_stays_silent() {
        let mut c = MasterChain::new(&elite()).unwrap();
        let mut buf = vec![0.0f32; 2 * 48000];
        c.process(&mut buf);
        assert!(buf.iter().all(|y| *y == 0.0));
    }

    /// Full scale and beyond never comes out over the limiter's ceiling (threshold x make-up), and nothing that is not
    /// finite comes out of a sample that is not finite or huge.
    #[test]
    fn nothing_comes_out_over_the_ceiling_or_not_finite() {
        let ceiling = 10f32.powf(-0.7) * 10f32.powf(0.7);
        for amp in [1.0f32, 10.0, 1.0e30] {
            let mut c = MasterChain::new(&elite()).unwrap();
            let mut buf: Vec<f32> = (0..96000).flat_map(|i| {
                let x = if (i / 40) % 2 == 0 { amp } else { -amp };
                [x * (i as f32 * 0.01).sin(), x]
            }).collect();
            c.process(&mut buf);
            let most = buf.iter().fold(0.0f32, |m, y| m.max(y.abs()));
            assert!(most.is_finite() && most <= ceiling * (1.0 + 2.0 * f32::EPSILON), "{amp}: {most}");
            assert!(c.counts().limited > 0 && c.counts().compressed > 0);
        }
        let mut c = MasterChain::new(&elite()).unwrap();
        let mut buf: Vec<f32> = (0..4096).map(|i| [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5, 1.0e38][i % 5]).collect();
        c.process(&mut buf);
        assert!(buf.iter().all(|y| y.is_finite() && y.abs() <= ceiling * 1.000001));
        assert_eq!(c.counts().guarded, 0);
    }

    /// The channels are independent (link mode 0): a loud left leaves the quiet right as it is alone.
    #[test]
    fn the_channels_are_independent() {
        let mk = |left: f32| -> Vec<f32> { (0..48000).flat_map(|i| { let s = (i as f32 * 0.05).sin(); [left * s, 0.01 * s] }).collect() };
        let (mut a, mut b) = (mk(0.9), mk(0.0));
        MasterChain::new(&elite()).unwrap().process(&mut a);
        MasterChain::new(&elite()).unwrap().process(&mut b);
        assert!(a.iter().skip(1).step_by(2).zip(b.iter().skip(1).step_by(2)).all(|(x, y)| x == y));
    }

    /// At small levels the whole plug-in is about +14 dB (T5f's small-signal response: +14.1..14.6 dB from 631 to 1600 Hz).
    #[test]
    fn a_quiet_mix_comes_out_14_db_louder() {
        let mut c = MasterChain::new(&elite()).unwrap();
        let g = measured_db(1000.0, 10f32.powf(-50.0 / 20.0), |b| c.process(b));
        assert!((14.0..14.8).contains(&g), "{g:.2} dB at 1 kHz");
        assert_eq!(c.counts().compressed + c.counts().limited, 0);
    }

    /// The same output for blocks of 128 frames (the output's, always) and of 256 (whole engine calls): only those two cuts
    /// are tested (the compressor's 32-frame cadence follows the frame count, not the cut, and 32 divides both). A
    /// reset leaves no tail of what went before.
    #[test]
    fn blocks_of_128_and_256_give_the_same_output_and_a_reset_leaves_no_tail() {
        let input: Vec<f32> = (0..2 * 20480).map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0).collect();
        let run = |cut: usize| {
            let mut c = MasterChain::new(&elite()).unwrap();
            let mut b = input.clone();
            for part in b.chunks_mut(2 * cut) {
                c.process(part);
            }
            b
        };
        assert_eq!(run(128), run(256));
        let mut c = MasterChain::new(&elite()).unwrap();
        let mut loud = input.clone();
        c.process(&mut loud);
        c.reset();
        let mut quiet = vec![0.0f32; 2 * 4096];
        c.process(&mut quiet);
        assert!(quiet.iter().all(|y| *y == 0.0));
    }

    /// The cross-check against T5f's scratch model of the plug-in (`re/elite/work/t5f/bamaster`): raw stereo `f32` frames
    /// in (`XCHECK_IN`), the chain's output out (`XCHECK_OUT`); `re/elite/work/sound/t5d8/xcheck.py` compares the two.
    #[test]
    #[ignore]
    fn cross_check_file() {
        let (Ok(i), Ok(o)) = (std::env::var("XCHECK_IN"), std::env::var("XCHECK_OUT")) else { return };
        let b = std::fs::read(i).unwrap();
        let mut x: Vec<f32> = b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let mut c = MasterChain::new(&elite()).unwrap();
        for part in x.chunks_mut(2 * 128) {
            c.process(part);
        }
        std::fs::write(o, x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        eprintln!("{:?}", c.counts());
    }

    /// A block the plug-in refuses or the chain does not build is an error with a message, never a panic.
    #[test]
    fn what_is_not_built_is_refused() {
        let mut s = elite();
        s.comp_link = 1;
        assert!(MasterChain::new(&s).unwrap_err().contains("link"));
        let mut s = elite();
        s.limiter_mode = 0;
        assert!(MasterChain::new(&s).unwrap_err().contains("mode"));
        let mut s = elite();
        s.eq[1].kind = 9;
        assert!(MasterChain::new(&s).unwrap_err().contains("type 9"));
        let mut s = elite();
        s.crossovers = [500.0, 400.0, 3000.0];
        assert!(MasterChain::new(&s).is_err());
    }
}
