//! Wwise's voice low-pass and high-pass filters as Elite's own copy of the sound engine does them (the engine is linked into
//! `EliteDangerous64.exe`, build 2026.10.23.334925; decompiled offline into `re/elite/ghidra/dec/`):
//!
//! - `FUN_144ad61c0` turns a 0-100 low-pass value into a cutoff: under 30 a line from 20,000 Hz (0) to 7,000 Hz (30),
//!   from 30 on `16.797443 * 2^((100 - v) * 1042939.9375 / 2^23)` worked out with the exe's own fast power of two (a
//!   float's bits from an integer, then a quadratic on the mantissa: reproduced here bit for bit in `f32`); the result is
//!   capped at 0.45 x the output sample rate. `FUN_144ad6190` is the high-pass's: the same function of `100 - v`.
//! - `FUN_144abec00` (low-pass) and `FUN_144abd730` (high-pass) run a second-order Butterworth section (the `sqrt 2` of
//!   `DAT_1450cb4ac`) with the bilinear transform's `tan(pi fc / fs)`, its value ramped from the last buffer's to this one's
//!   in 8 steps (`DAT_145ee0a5c`) of 128 frames (`DAT_145ee0a54`).
//! - `FUN_144ac2bf0`: a filter whose value is at most 0.1 (`DAT_144dee294`) is bypassed.
//! - The Parametric EQ's bands ([`eq_coefs`]): `FUN_144a0f510` maps a band's filter to `FUN_144a05110`'s design type and
//!   that function designs the section (RBJ's cookbook shapes with the exe's own constants and fast power of ten).
//! - The Guitar Distortion ([`Distortion`], plug-in 0x007e0003: registration record `0x145ee00f8`, create `0x144a040a0`,
//!   `Init` `0x144a04540`, `Execute` `0x144a04230`; its parameters `SetParam` `0x144a04b60`, `SetParamsBlock` `0x144a04f50`):
//!   the input is kept as the dry signal; the wet one goes through three pre-EQ bands, the distortion curve, the
//!   rectifier and three post-EQ bands; the two are mixed by the wet share, both times the output level; a DC blocker ends
//!   it. Each stage is described where it is built.

/// At most this, a filter value leaves the signal alone (`FUN_144ac2bf0`).
pub const BYPASS: f32 = 0.1;
/// The cap on a cutoff, as a share of the output sample rate (`DAT_145154fec`).
pub const MOST_SHARE: f32 = 0.45;
/// Under this a section's output is 0 (about -300 dB: far under anything heard, far over the subnormal floats).
const FLUSH: f64 = 1e-15;
/// The guard at the end of a voice's chain: a sample beyond this (before the voice's level) means the chain ran away, and
/// its memory starts again from rest. 80 dB over full scale: the shipped data's largest chain gain stays far under it
/// (`every_sounds_chain_fits` in the game's voices), so no legitimate boost can reach it.
pub const CHAIN_BOUND: f32 = 1.0e4;
/// Elite's sound engine buffer (frames): an effect takes its parameters once a buffer and ramps from the last buffer's
/// values over it (the voice filters' 8 steps of 128 frames, `DAT_145ee0a5c` x `DAT_145ee0a54`, are one buffer).
pub const BUFFER_FRAMES: u32 = 1024;

/// `FUN_144ad61c0`: a 0-100 low-pass value's cutoff (Hz) at an output sample rate.
pub fn lowpass_cutoff(v: f32, sample_rate: f32) -> f32 {
    let f = if v >= 30.0 {
        let bits = ((100.0 - v) * 1_042_939.937_5_f32 + 1_065_353_216.0_f32) as i64 as u32;
        let m = f32::from_bits((bits & 0x007f_ffff) + 0x3f80_0000);
        let e = f32::from_bits(bits & 0xff80_0000);
        ((m * 0.325_189_77 + 0.020_805_772) * m + 0.653_043_45) * e * 16.797_443
    } else {
        (30.0 - v) * 433.333_34 + 7000.0
    };
    f.min(sample_rate * MOST_SHARE)
}

/// `FUN_144ad6190`: a 0-100 high-pass value's cutoff (Hz).
pub fn highpass_cutoff(v: f32, sample_rate: f32) -> f32 {
    lowpass_cutoff(100.0 - v, sample_rate)
}

/// A filter value's cutoff, or None where Wwise bypasses the filter.
pub fn cutoff(v: f32, highpass: bool, sample_rate: f32) -> Option<f32> {
    if !(v > BYPASS) {
        return None;
    }
    Some(if highpass { highpass_cutoff(v, sample_rate) } else { lowpass_cutoff(v, sample_rate) })
}

/// One second-order section's coefficients, normalised: `y = b0 x + b1 x1 + b2 x2 - a1 y1 - a2 y2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

/// The Butterworth sections of `FUN_144abec00` (low-pass: `c = 1 / tan(pi fc / fs)`, `b0 = 1 / (1 + sqrt2 c + c^2)`,
/// `b1 = 2 b0`, `a1 = (2 - 2 c^2) b0`, `a2 = (1 - sqrt2 c + c^2) b0`) and `FUN_144abd730` (high-pass: `k = tan(pi fc / fs)`,
/// `b0 = 1 / (k^2 + sqrt2 k + 1)`, `b1 = -2 b0`, `a1 = 2 (k^2 - 1) b0`, `a2 = (k^2 - sqrt2 k + 1) b0`).
///
/// Worked out in `f64` (the exe's are `f32`): a cutoff of a few hertz puts the poles within a thousandth of the unit circle,
/// where `f32` rounding of `c^2` leaves little margin. None for a cutoff or rate that is not a finite positive number; the
/// cutoff is held within 1 Hz .. 0.45 of the rate.
pub fn coefs(cutoff_hz: f32, highpass: bool, sample_rate: f32) -> Option<Coefs> {
    if !(cutoff_hz.is_finite() && cutoff_hz > 0.0 && sample_rate.is_finite() && sample_rate >= 1.0) {
        return None;
    }
    let (fs, fc) = (f64::from(sample_rate), f64::from(cutoff_hz));
    let k = (std::f64::consts::PI / fs * fc.clamp(1.0, fs * f64::from(MOST_SHARE))).tan();
    let r2 = std::f64::consts::SQRT_2;
    Some(if highpass {
        let b0 = 1.0 / (k * k + r2 * k + 1.0);
        Coefs { b0, b1: -2.0 * b0, b2: b0, a1: 2.0 * (k * k - 1.0) * b0, a2: (k * k - r2 * k + 1.0) * b0 }
    } else {
        let c = 1.0 / k;
        let b0 = 1.0 / (1.0 + r2 * c + c * c);
        Coefs { b0, b1: 2.0 * b0, b2: b0, a1: (2.0 - 2.0 * c * c) * b0, a2: (1.0 - r2 * c + c * c) * b0 }
    })
}

/// `10^(gain_db / 40)` as the exe works it out for the EQ's shelves and peaks (`FUN_144a05110`): `x = gain * 0.025`, a
/// float's bits from `x * 2^23 log2(10) + 127 * 2^23`, then the same quadratic on the mantissa as the low-pass's fast power
/// of two. None below `x = -37` (the exe leaves the factor at 0 there; no band of the data comes near: gains are -24..24).
fn eq_amplitude(gain_db: f32) -> Option<f32> {
    fast_pow10(gain_db * 0.025)
}

/// The exe's fast `10^x` (`DAT_1457db464` = 2^23 log2 10, `DAT_1457db468` = 127 x 2^23, the quadratic `DAT_1457db450`,
/// `DAT_1457db44c`, `DAT_1457db454` on the mantissa); None below `x = -37` (`DAT_1457db46c`), where the exe gives 0.
fn fast_pow10(x: f32) -> Option<f32> {
    if !(x >= -37.0) {
        return None;
    }
    let bits = (x * 27_866_352.0_f32 + 1_065_353_216.0_f32) as i64 as u32;
    Some(fast_mantissa(bits))
}

/// The fast power's last stage: the float of `bits`' exponent times the quadratic on its mantissa.
#[inline]
fn fast_mantissa(bits: u32) -> f32 {
    let m = f32::from_bits((bits & 0x007f_ffff) + 0x3f80_0000);
    let e = f32::from_bits(bits & 0xff80_0000);
    ((m * 0.325_189_77 + 0.020_805_772) * m + 0.653_043_45) * e
}

/// A Parametric EQ band's section, as Elite's copy of the plug-in designs it: `FUN_144a0f510` maps the band's filter to the
/// design function's type and `FUN_144a05110` designs it, in `f32`, at the output rate, the frequency capped at 0.45 of it:
/// - low-pass and high-pass: the voice filters' Butterworth sections (`tan(pi f / fs)`, `sqrt 2`), gain and Q unused;
/// - band-pass (constant 0 dB peak) and notch: `alpha = sin(w) / (2 Q)`, `w = 2 pi f / fs`;
/// - peaking: `A = 10^(gain / 40)`, `b = 1 +- alpha A, -2 cos w`, `a = 1 +- alpha / A, -2 cos w`;
/// - low and high shelves: slope 1, Q unused: `2 sqrt(A) alpha = sin(w) sqrt(A) sqrt 2`,
///   `b0 = A((A+1) -+ (A-1) cos w + s)`, `b1 = +-2A((A-1) -+ (A+1) cos w)`, `b2 = A((A+1) -+ (A-1) cos w - s)`,
///   `a0 = (A+1) +- (A-1) cos w + s`, `a1 = -+2((A-1) +- (A+1) cos w)`, `a2 = (A+1) +- (A-1) cos w - s`;
///
/// each divided by `a0` (the exe stores `-a1`, `-a2` for its direct form; [`Coefs`] keeps the signs of `y = b0 x + ... - a1
/// y1 - a2 y2`). None where a value is not a finite number (a frequency of 0 or less, a Q of 0) or the gain is out of the
/// exe's reach: the band is then left out.
pub fn eq_coefs(filter: super::data::EqFilter, frequency: f32, gain_db: f32, q: f32, sample_rate: f32) -> Option<Coefs> {
    use super::data::EqFilter as F;
    if !(frequency > 0.0 && sample_rate >= 1.0 && frequency.is_finite() && gain_db.is_finite() && q.is_finite()) {
        return None;
    }
    let f = frequency.min(sample_rate * MOST_SHARE);
    let r2 = 1.414_213_5_f32;
    let w = f * 6.283_185_5 / sample_rate;
    let (b0, b1, b2, a0, a1, a2) = match filter {
        F::LowPass | F::HiPass => {
            let k = (f * std::f32::consts::PI / sample_rate).tan();
            let k = if filter == F::LowPass { 1.0 / k } else { k };
            let n = 1.0 / (k * k + 1.0 + k * r2);
            let a2 = (k * k + 1.0 - k * r2) * n;
            if filter == F::LowPass {
                (n, n + n, n, 1.0, (1.0 - k * k) * (n + n), a2)
            } else {
                (n, n * -2.0, n, 1.0, (1.0 - k * k) * (n * -2.0), a2)
            }
        }
        F::BandPass | F::Notch => {
            let (s, c) = w.sin_cos();
            let alpha = s * (0.5 / q);
            if filter == F::BandPass {
                (alpha, 0.0, -alpha, alpha + 1.0, c * -2.0, 1.0 - alpha)
            } else {
                (1.0, c * -2.0, 1.0, alpha + 1.0, c * -2.0, 1.0 - alpha)
            }
        }
        F::PeakingEq => {
            let a = eq_amplitude(gain_db)?;
            let (s, c) = w.sin_cos();
            let alpha = s * (0.5 / q);
            (alpha * a + 1.0, c * -2.0, 1.0 - alpha * a, alpha / a + 1.0, c * -2.0, 1.0 - alpha / a)
        }
        F::LowShelf | F::HiShelf => {
            let a = eq_amplitude(gain_db)?;
            let (s, c) = w.sin_cos();
            let shelf = s * a.sqrt() * r2;
            let (am1, ap1) = (a - 1.0, a + 1.0);
            if filter == F::LowShelf {
                (
                    (shelf + (ap1 - am1 * c)) * a,
                    (a + a) * (am1 - ap1 * c),
                    ((ap1 - am1 * c) - shelf) * a,
                    shelf + (am1 * c + ap1),
                    am1 * -2.0 - (ap1 * c + ap1 * c),
                    (am1 * c + ap1) - shelf,
                )
            } else {
                (
                    ((am1 * c + ap1) + shelf) * a,
                    (ap1 * c + am1) * a * -2.0,
                    ((am1 * c + ap1) - shelf) * a,
                    (ap1 - am1 * c) + shelf,
                    (am1 - ap1 * c) + (am1 - ap1 * c),
                    (ap1 - am1 * c) - shelf,
                )
            }
        }
    };
    let inv = 1.0 / a0;
    let c = Coefs { b0: f64::from(inv * b0), b1: f64::from(inv * b1), b2: f64::from(inv * b2), a1: f64::from(inv * a1), a2: f64::from(inv * a2) };
    [c.b0, c.b1, c.b2, c.a1, c.a2].iter().all(|v| v.is_finite()).then_some(c)
}

/// One channel's section state (direct form I).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Section {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Section {
    #[inline]
    pub fn run(&mut self, c: &Coefs, x: f32) -> f32 {
        let x = if x.is_finite() { f64::from(x) } else { 0.0 };
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        // Only a value that is not a number restarts the section here; a boost of any size passes (the chain's own end
        // has the bound, `CHAIN_BOUND`).
        if !y.is_finite() {
            *self = Section::default();
            return 0.0;
        }
        // A decaying output is flushed to 0 long before it reaches the subnormal floats, whose arithmetic is many times
        // slower.
        let y = if y.abs() < FLUSH { 0.0 } else { y };
        *self = Section { x1: x, x2: self.x1, y1: y, y2: self.y1 };
        y as f32
    }

    /// [`Section::run`] on each sample of a block in place, its state held locally across the block: the same arithmetic
    /// sample for sample, so the same output bits.
    #[inline]
    pub fn run_block(&mut self, c: &Coefs, xs: &mut [f32]) {
        let mut s = *self;
        for x in xs {
            *x = s.run(c, *x);
        }
        *self = s;
    }
}

/// The Guitar Distortion's curves (the `Execute`'s dispatch `FUN_144a068a0` on the type; 0 runs none).
pub const DISTORTION_NONE: u32 = 0;
/// `FUN_144a07950`: the soft clip `2a` under a third, `(3 - (2 - 3a)^2) / 3` under two thirds, 1 above, of `a = |g x|`,
/// with the input's sign (0 and below negative).
pub const DISTORTION_OVERDRIVE: u32 = 1;
/// `FUN_144a07400`: `1 - e^-a` by the exe's fast exponential (0 past `a = 37`), the input's sign turned over (a positive
/// input gives a negative output).
pub const DISTORTION_HEAVY: u32 = 2;

/// The Overdrive curve of one already gained sample (`FUN_144a07950`; thirds as the exe's `DAT_1457dc0fc` 0.33333331 and
/// `DAT_1457dc100` 0.66666663).
#[inline]
pub fn overdrive(gx: f32) -> f32 {
    const THIRD: f32 = 0.333_333_31;
    const TWO_THIRDS: f32 = 0.666_666_63;
    let a = gx.abs();
    let mut y = 1.0;
    if a <= TWO_THIRDS {
        let t = 2.0 - a * 3.0;
        y = (3.0 - t * t) * THIRD;
    }
    if a <= THIRD {
        y = a + a;
    }
    if gx <= 0.0 { -y } else { y }
}

/// The Heavy curve of one already gained sample (`FUN_144a07400`: `e^-a` from the float bits `127 x 2^23 - a x 2^23 / ln
/// 2`, `DAT_1457dc10c`, through the same quadratic; the comparison with `-0.0`, `DAT_144ded02c`, puts 0 with the positives).
#[inline]
pub fn heavy(gx: f32) -> f32 {
    let a = gx.abs();
    let e = if a <= 37.0 { fast_mantissa((1_065_353_216.0_f32 - a * 12_102_203.0_f32) as i64 as u32) } else { 0.0 };
    if -0.0 <= gx { e - 1.0 } else { 1.0 - e }
}

/// The values a Guitar Distortion ramps over a buffer: the gain before the curve and after it, the rectifier's floor and
/// fold, and the wet and dry gains (the output level in both).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct DistortionGains {
    pre: f32,
    post: f32,
    floor: f32,
    fold: f32,
    wet: f32,
    dry: f32,
}

impl DistortionGains {
    /// As the exe works them out of the parameters:
    /// - `FUN_144a07dd0`: the drive's gain `4 x 25^(drive / 100) - 3` (`powf`, `DAT_144de1688` 25, `DAT_144ded004` 4,
    ///   `DAT_144deaa5c` 3); the Overdrive takes it times `tone / 300 + 2 / 3` (`DAT_1457dc0f8`, `DAT_1457dc100`), the
    ///   Heavy alone; after the curve both multiply by the fast `10^(-drive / 100)` (`DAT_1452477b4` -0.01);
    /// - `FUN_144a08110`: the rectifier's floor `-(50 - min(r, 50)) / 50` and fold `(r - 50) / 50` (`DAT_144de1690` 50,
    ///   `DAT_144e299a8` 0.02, `DAT_1457dc118` -0.02);
    /// - the `Execute`'s mix (`FUN_1449fadc0`): the wet signal times `level x wet / 100`, the dry times `level x (100 -
    ///   wet) / 100` (`DAT_144dec67c` 100, `DAT_144e299a4` 0.01).
    fn of(d: &super::DistortionNow) -> DistortionGains {
        let drive_gain = 25.0f32.powf(d.drive * 0.01) * 4.0 - 3.0;
        let pre = match d.kind {
            DISTORTION_OVERDRIVE => (d.tone * 0.003_333_333_2 + 0.666_666_63) * drive_gain,
            DISTORTION_HEAVY => drive_gain,
            _ => 1.0,
        };
        let dry_share = (100.0 - d.wet) * 0.01;
        DistortionGains {
            pre,
            post: fast_pow10(d.drive * -0.01).unwrap_or(0.0),
            floor: (50.0 - d.rectification.min(50.0)) * -0.02,
            fold: (d.rectification - 50.0) * 0.02,
            wet: d.level * (1.0 - dry_share),
            dry: d.level * dry_share,
        }
    }

    /// `k / n` of the way from `self` to `to` (the exe's ramps: `from + k (to - from) / n`).
    fn at(&self, to: &DistortionGains, t: f32) -> DistortionGains {
        let l = |a: f32, b: f32| a + (b - a) * t;
        DistortionGains { pre: l(self.pre, to.pre), post: l(self.post, to.post), floor: l(self.floor, to.floor), fold: l(self.fold, to.fold), wet: l(self.wet, to.wet), dry: l(self.dry, to.dry) }
    }
}

/// One EQ band of the Guitar Distortion: its values, its section (None: off) and a memory a channel.
#[derive(Clone, Debug)]
struct DistortionBand {
    now: super::EqNow,
    coefs: Option<Coefs>,
    channels: Vec<Section>,
}

impl DistortionBand {
    fn set(&mut self, b: super::EqNow, sample_rate: f32) {
        if b != self.now {
            self.now = b;
            self.coefs = match (b.on, b.filter) {
                (true, Some(f)) => eq_coefs(f, b.frequency, b.gain_db, b.q, sample_rate),
                _ => None,
            };
        }
    }

    fn run(&mut self, chan: usize, x: f32) -> f32 {
        match (&self.coefs, self.channels.get_mut(chan)) {
            (Some(c), Some(s)) => s.run(c, x),
            _ => x,
        }
    }
}

/// A Guitar Distortion on one voice, as Elite's copy of the plug-in runs it (`Execute` `0x144a04230`). Each buffer of
/// [`BUFFER_FRAMES`] it takes the latest parameters ([`Distortion::set`]): the bands whose values changed are designed
/// again (`FUN_144a04730`: `FUN_144a05110` with the band's type as its design type, [`super::data::EqFilter::from_design`];
/// their memory is kept), and every gain ramps from the last buffer's to this one's over the buffer. Per frame and channel:
/// the dry input is kept; the wet signal goes through the pre-EQ bands (`FUN_144a05ba0` on +0x20), the curve with its gains
/// (`FUN_144a068a0`), the rectifier (`FUN_144a07ef0`: under 50 a floor, `max(x, floor)`; over 50 the floor 0 and the
/// negative half folded up by `fold`; at 0 none) and the post-EQ bands (+0x30); the two are mixed (`FUN_1449fadc0`) and
/// the result goes through the DC blocker `y = x - x1 + R y1`, `R = 1 - 251.327 / fs` (`FUN_144a08170`, its `R` set in
/// `Init` from `DAT_1457dc0b8`). The first buffer starts at its own values (`Init` sets the last buffer's to them).
/// Nothing here allocates once it is made.
#[derive(Clone, Debug)]
pub struct Distortion {
    pre: [DistortionBand; 3],
    post: [DistortionBand; 3],
    target: Option<super::DistortionNow>,
    kind: u32,
    rect_mode: u8,
    from: DistortionGains,
    to: DistortionGains,
    started: bool,
    frame: u32,
    dc: Vec<(f32, f32)>,
    r: f32,
    sample_rate: f32,
}

impl Distortion {
    pub fn new(channels: usize, sample_rate: f32) -> Distortion {
        let band = DistortionBand { now: super::EqNow::OFF, coefs: None, channels: vec![Section::default(); channels.max(1)] };
        Distortion {
            pre: std::array::from_fn(|_| band.clone()),
            post: std::array::from_fn(|_| band.clone()),
            target: None,
            kind: DISTORTION_NONE,
            rect_mode: 0,
            from: DistortionGains::default(),
            to: DistortionGains::default(),
            started: false,
            frame: 0,
            dc: vec![(0.0, 0.0); channels.max(1)],
            r: 1.0 - 251.327_42 / sample_rate,
            sample_rate,
        }
    }

    /// The parameters the next buffer takes.
    pub fn set(&mut self, d: &super::DistortionNow) {
        self.target = Some(*d);
    }

    /// Every memory to rest; the next buffer starts at its own values, as a new instance does.
    pub fn reset(&mut self) {
        for b in self.pre.iter_mut().chain(self.post.iter_mut()) {
            b.channels.iter_mut().for_each(|s| *s = Section::default());
        }
        self.dc.iter_mut().for_each(|d| *d = (0.0, 0.0));
        self.started = false;
        self.frame = 0;
    }

    /// The start of a buffer: the parameters taken, the bands designed, the ramps set.
    fn begin_buffer(&mut self) {
        let Some(d) = self.target else { return };
        for (band, b) in self.pre.iter_mut().zip(d.pre).chain(self.post.iter_mut().zip(d.post)) {
            band.set(b, self.sample_rate);
        }
        self.kind = d.kind;
        // (`FUN_144a08110`: 0 is none; over 50 the fold; a value that is not a number reads as 0, as `ucomiss` gives.)
        let r = d.rectification;
        self.rect_mode = if r == 0.0 || r.is_nan() { 0 } else if r > 50.0 { 2 } else { 1 };
        let g = DistortionGains::of(&d);
        self.from = if self.started { self.to } else { g };
        self.to = g;
        self.started = true;
    }

    /// One frame (a sample a channel) in place.
    pub fn frame(&mut self, x: &mut [f32]) {
        if self.frame == 0 {
            self.begin_buffer();
        }
        let g = self.from.at(&self.to, self.frame as f32 / BUFFER_FRAMES as f32);
        self.frame = (self.frame + 1) % BUFFER_FRAMES;
        if !self.started {
            return;
        }
        for (c, s) in x.iter_mut().enumerate() {
            let dry = *s;
            let mut w = dry;
            for b in &mut self.pre {
                w = b.run(c, w);
            }
            w = match self.kind {
                DISTORTION_OVERDRIVE => overdrive(g.pre * w) * g.post,
                DISTORTION_HEAVY => heavy(g.pre * w) * g.post,
                _ => w,
            };
            if self.rect_mode != 0 {
                let floored = if w <= g.floor { g.floor } else { w };
                w = if self.rect_mode == 2 && w <= 0.0 { floored - w * g.fold } else { floored };
            }
            for b in &mut self.post {
                w = b.run(c, w);
            }
            let y = g.wet * w + g.dry * dry;
            let Some(dc) = self.dc.get_mut(c) else { continue };
            let out = y - dc.0 + self.r * dc.1;
            let out = if out.is_finite() { if out.abs() < FLUSH as f32 { 0.0 } else { out } } else { 0.0 };
            *dc = (if y.is_finite() { y } else { 0.0 }, out);
            *s = out;
        }
    }

    /// `frames` frames at once, the channels apart (channel `c`'s samples at `buf[c * stride..][..frames]`), in place:
    /// [`Distortion::frame`]'s arithmetic sample for sample (the same output bits), done a stage at a time over a run of
    /// frames (each band's section over the run, then the curve and the rectifier, then the mix and the DC blocker), runs
    /// cut at the buffer's edges and at `CHUNK` frames (the wet signal's room).
    pub fn block(&mut self, buf: &mut [f32], channels: usize, stride: usize, frames: usize) {
        const CHUNK: usize = 128;
        let mut wet = [0.0f32; CHUNK];
        let mut i = 0;
        while i < frames {
            if self.frame == 0 {
                self.begin_buffer();
            }
            let run = (frames - i).min((BUFFER_FRAMES - self.frame) as usize).min(CHUNK);
            let f0 = self.frame;
            self.frame = (self.frame + run as u32) % BUFFER_FRAMES;
            if self.started {
                let (from, to) = (self.from, self.to);
                let g = |j: usize| from.at(&to, (f0 + j as u32) as f32 / BUFFER_FRAMES as f32);
                for c in 0..channels {
                    let Some(dry) = buf.get_mut(c * stride + i..c * stride + i + run) else { continue };
                    let w = &mut wet[..run];
                    w.copy_from_slice(dry);
                    for b in &mut self.pre {
                        if let (Some(co), Some(s)) = (&b.coefs, b.channels.get_mut(c)) {
                            s.run_block(co, w);
                        }
                    }
                    match self.kind {
                        DISTORTION_OVERDRIVE => w.iter_mut().enumerate().for_each(|(j, x)| {
                            let g = g(j);
                            *x = overdrive(g.pre * *x) * g.post;
                        }),
                        DISTORTION_HEAVY => w.iter_mut().enumerate().for_each(|(j, x)| {
                            let g = g(j);
                            *x = heavy(g.pre * *x) * g.post;
                        }),
                        _ => {}
                    }
                    if self.rect_mode != 0 {
                        for (j, x) in w.iter_mut().enumerate() {
                            let g = g(j);
                            let floored = if *x <= g.floor { g.floor } else { *x };
                            *x = if self.rect_mode == 2 && *x <= 0.0 { floored - *x * g.fold } else { floored };
                        }
                    }
                    for b in &mut self.post {
                        if let (Some(co), Some(s)) = (&b.coefs, b.channels.get_mut(c)) {
                            s.run_block(co, w);
                        }
                    }
                    let Some(dc) = self.dc.get_mut(c) else { continue };
                    let mut d = *dc;
                    for (j, (s, x)) in dry.iter_mut().zip(w.iter()).enumerate() {
                        let g = g(j);
                        let y = g.wet * *x + g.dry * *s;
                        let out = y - d.0 + self.r * d.1;
                        let out = if out.is_finite() { if out.abs() < FLUSH as f32 { 0.0 } else { out } } else { 0.0 };
                        d = (if y.is_finite() { y } else { 0.0 }, out);
                        *s = out;
                    }
                    *dc = d;
                }
            }
            i += run;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoffs_are_the_exes() {
        // The two branches meet at 30 (7,000 Hz), 0 is 20,000 Hz, 100 is the constant itself.
        // (The exe's fast power of two reads 7,016 Hz at 30 where the line gives 7,000: kept as the exe has it.)
        assert!((lowpass_cutoff(30.0, 48000.0) - 7016.0).abs() < 1.0);
        assert_eq!(lowpass_cutoff(0.0, 48000.0), 20000.0);
        assert!((lowpass_cutoff(100.0, 48000.0) - 16.797).abs() < 0.05);
        // The fast power of two against the exact one: within a few tenths of a percent.
        for v in [35.0f32, 50.0, 64.0, 80.0, 99.0] {
            let exact = 16.797443 * 2f32.powf((100.0 - v) * 0.124_328_13);
            assert!((lowpass_cutoff(v, 48000.0) / exact - 1.0).abs() < 0.005, "{v}");
        }
        // The cap at 0.45 fs, the high-pass mirror, the bypass.
        assert_eq!(lowpass_cutoff(0.0, 32000.0), 14400.0);
        assert_eq!(highpass_cutoff(70.0, 48000.0), lowpass_cutoff(30.0, 48000.0));
        assert_eq!(cutoff(0.1, false, 48000.0), None);
        assert!(cutoff(0.2, true, 48000.0).is_some());
    }

    /// After the input stops, a low cutoff's ringing goes to an exact 0 and never through the subnormal floats.
    #[test]
    fn a_decaying_section_never_holds_subnormals() {
        for c in [coefs(17.0, true, 48000.0), coefs(20.0, false, 48000.0), coefs(21600.0, true, 48000.0)].map(Option::unwrap) {
            let mut s = Section::default();
            for i in 0..4800 {
                s.run(&c, if i % 2 == 0 { 0.5 } else { -0.5 });
            }
            for _ in 0..48000 * 20 {
                let y = s.run(&c, 0.0);
                assert!(y == 0.0 || y.is_normal(), "{y:e}");
            }
            assert_eq!(s, Section::default(), "rung down to nothing");
        }
    }

    /// The block forms give the per-sample forms' bits: a section over blocks of uneven lengths, and a stereo distortion
    /// over 3.5 buffers in blocks of 128 and of 100 frames whose parameters change between buffers (bands switched on and
    /// off, the curve, the rectifier's two modes, the wet share).
    #[test]
    fn blocks_give_the_samples_bits() {
        let input: Vec<f32> = (0..3584).map(|i| ((i as f32 * 0.037).sin() * 0.7 + (i as f32 * 0.31).cos() * 0.2) * if i % 997 == 0 { 3.0 } else { 1.0 }).collect();
        let c = eq_coefs(super::super::data::EqFilter::PeakingEq, 900.0, 7.0, 0.8, 48000.0).unwrap();
        let (mut a, mut b) = (Section::default(), Section::default());
        let one: Vec<f32> = input.iter().map(|x| a.run(&c, *x)).collect();
        let mut blk = input.clone();
        for part in blk.chunks_mut(77) {
            b.run_block(&c, part);
        }
        assert_eq!(one.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), blk.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
        let band = |on, filter, gain_db, frequency| super::super::EqNow { on, filter: Some(filter), frequency, gain_db, q: 1.0 };
        use super::super::data::EqFilter as F;
        let params = [
            super::super::DistortionNow { pre: [band(true, F::LowShelf, 12.5, 462.0), band(false, F::PeakingEq, -2.0, 5200.0), band(true, F::HiShelf, -2.0, 1500.0)], post: [band(true, F::PeakingEq, 3.0, 892.0); 3], kind: DISTORTION_OVERDRIVE, drive: 29.0, tone: 35.0, rectification: 29.0, level: 1.0, wet: 48.0 },
            super::super::DistortionNow { pre: [band(true, F::PeakingEq, 6.0, 300.0); 3], post: [band(false, F::HiShelf, -3.0, 5522.0); 3], kind: DISTORTION_HEAVY, drive: 70.0, tone: 0.0, rectification: 80.0, level: 0.7, wet: 100.0 },
            super::super::DistortionNow { pre: [band(false, F::PeakingEq, 0.0, 300.0); 3], post: [band(false, F::PeakingEq, 0.0, 300.0); 3], kind: DISTORTION_NONE, drive: 0.0, tone: 0.0, rectification: 0.0, level: 1.0, wet: 0.0 },
            super::super::DistortionNow { pre: [band(true, F::LowShelf, 3.0, 200.0); 3], post: [band(true, F::PeakingEq, -6.0, 2000.0); 3], kind: DISTORTION_OVERDRIVE, drive: 100.0, tone: 100.0, rectification: 50.0, level: 1.3, wet: 12.0 },
        ];
        for block in [128usize, 100] {
            let mut p = Distortion::new(2, 48000.0);
            let mut q = Distortion::new(2, 48000.0);
            let mut want = Vec::new();
            for (i, x) in input.iter().enumerate() {
                if i % BUFFER_FRAMES as usize == 0 {
                    p.set(&params[i / BUFFER_FRAMES as usize]);
                }
                let mut f = [*x, -*x * 0.5];
                p.frame(&mut f);
                want.extend(f);
            }
            let mut got = vec![0.0f32; input.len() * 2];
            let mut i = 0;
            while i < input.len() {
                let n = block.min(input.len() - i);
                if i % BUFFER_FRAMES as usize == 0 || (i / BUFFER_FRAMES as usize) != ((i + n - 1) / BUFFER_FRAMES as usize) {
                    // (A block that reaches a new buffer: the next buffer's values, as `frame` takes them at its start.)
                    q.set(&params[(i + n - 1) / BUFFER_FRAMES as usize]);
                }
                let mut planar: Vec<f32> = input[i..i + n].iter().copied().chain(input[i..i + n].iter().map(|x| -x * 0.5)).collect();
                q.block(&mut planar, 2, n, n);
                for j in 0..n {
                    got[2 * (i + j)] = planar[j];
                    got[2 * (i + j) + 1] = planar[n + j];
                }
                i += n;
            }
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&got), bits(&want), "blocks of {block}");
        }
    }

    /// A section's gain at a frequency (|H(e^jw)|).
    fn gain_at(c: &Coefs, hz: f64, fs: f64) -> f64 {
        let w = std::f64::consts::TAU * hz / fs;
        let (z1, z2) = ((w.cos(), -w.sin()), ((2.0 * w).cos(), -(2.0 * w).sin()));
        let num = (c.b0 + c.b1 * z1.0 + c.b2 * z2.0, c.b1 * z1.1 + c.b2 * z2.1);
        let den = (1.0 + c.a1 * z1.0 + c.a2 * z2.0, c.a1 * z1.1 + c.a2 * z2.1);
        (num.0.hypot(num.1)) / (den.0.hypot(den.1))
    }

    /// The EQ's designs do what each filter is for, by its response: the shelves reach their gain on their side and 0 dB on
    /// the other, a peak its gain at its frequency, the band-pass 0 dB at its frequency, the notch nothing there, the
    /// low-pass and high-pass are the voice filters' Butterworth sections. 0 dB shelves and peaks leave the signal alone.
    #[test]
    fn eq_bands_do_what_their_filters_are_for() {
        use super::super::data::EqFilter as F;
        let fs = 48000.0;
        let db = |g: f64| 20.0 * g.log10();
        let ls = eq_coefs(F::LowShelf, 150.0, 5.0, 1.0, 48000.0).unwrap();
        assert!((db(gain_at(&ls, 5.0, fs)) - 5.0).abs() < 0.05 && db(gain_at(&ls, 20000.0, fs)).abs() < 0.05);
        assert!((db(gain_at(&ls, 150.0, fs)) - 2.5).abs() < 0.1, "a shelf's middle is half its gain");
        let hs = eq_coefs(F::HiShelf, 7000.0, -6.0, 1.0, 48000.0).unwrap();
        assert!((db(gain_at(&hs, 21000.0, fs)) + 6.0).abs() < 0.3 && db(gain_at(&hs, 50.0, fs)).abs() < 0.05);
        let pk = eq_coefs(F::PeakingEq, 220.0, -3.5, 0.5, 48000.0).unwrap();
        assert!((db(gain_at(&pk, 220.0, fs)) + 3.5).abs() < 0.05, "{}", db(gain_at(&pk, 220.0, fs)));
        assert!(db(gain_at(&pk, 20000.0, fs)).abs() < 0.2, "{}", db(gain_at(&pk, 20000.0, fs)));
        let bp = eq_coefs(F::BandPass, 1000.0, 0.0, 2.0, 48000.0).unwrap();
        assert!(db(gain_at(&bp, 1000.0, fs)).abs() < 0.02 && db(gain_at(&bp, 100.0, fs)) < -20.0);
        let nt = eq_coefs(F::Notch, 1000.0, 0.0, 2.0, 48000.0).unwrap();
        assert!(gain_at(&nt, 1000.0, fs) < 1e-3 && db(gain_at(&nt, 100.0, fs)).abs() < 0.1);
        for (f, hp) in [(F::LowPass, false), (F::HiPass, true)] {
            let e = eq_coefs(f, 1000.0, 3.0, 4.0, 48000.0).unwrap();
            let v = coefs(1000.0, hp, 48000.0).unwrap();
            for (x, y) in [(e.b0, v.b0), (e.b1, v.b1), (e.b2, v.b2), (e.a1, v.a1), (e.a2, v.a2)] {
                assert!((x - y).abs() < 1e-5, "{f:?}: {x} against the voice filter's {y}");
            }
        }
        // At 0 dB the exe's fast power of ten gives A = 0.99904, not 1 (its quadratic at a mantissa of 1): such a band is
        // within 0.02 dB of flat, kept as the exe has it.
        assert!((eq_amplitude(0.0).unwrap() - 0.999_039).abs() < 1e-5);
        for f in [F::LowShelf, F::HiShelf, F::PeakingEq] {
            let flat = eq_coefs(f, 500.0, 0.0, 1.0, 48000.0).unwrap();
            for hz in [20.0, 500.0, 15000.0] {
                assert!(db(gain_at(&flat, hz, fs)).abs() < 0.02, "{f:?} at 0 dB");
            }
        }
        // The cap at 0.45 fs, and what is left out.
        assert_eq!(eq_coefs(F::PeakingEq, 30000.0, 6.0, 1.0, 48000.0), eq_coefs(F::PeakingEq, 21600.0, 6.0, 1.0, 48000.0));
        assert_eq!(eq_coefs(F::PeakingEq, 0.0, 6.0, 1.0, 48000.0), None);
        assert_eq!(eq_coefs(F::BandPass, 100.0, 0.0, 0.0, 48000.0), None);
        // The exe's fast power of ten against the exact one.
        for g in [-24.0f32, -3.5, 0.0, 5.0, 24.0] {
            let exact = 10f32.powf(g / 40.0);
            assert!((eq_amplitude(g).unwrap() / exact - 1.0).abs() < 0.005, "{g}");
        }
    }

    /// The Guitar Distortion's curves and gains, at values worked out by hand from the decompiled code.
    #[test]
    fn distortion_curves_and_gains_are_the_exes() {
        // Overdrive: 2a under a third, (3 - (2 - 3a)^2) / 3 under two thirds, 1 above; the sign kept, 0 negative.
        assert_eq!(overdrive(0.2), 0.4);
        assert!((overdrive(0.5) - 0.916_666_6).abs() < 1e-6);
        assert!((overdrive(-0.5) + 0.916_666_6).abs() < 1e-6);
        assert_eq!(overdrive(0.8), 1.0);
        assert_eq!(overdrive(-3.0), -1.0);
        assert!(overdrive(0.0) == 0.0 && overdrive(0.0).is_sign_negative());
        // Heavy: -(1 - e^-a) for a positive input, +(1 - e^-a) for a negative, by the fast exponential (its error is
        // within 0.4 % of e^-a).
        for a in [0.1f32, 1.0, 3.0] {
            let exact = 1.0 - (-a).exp();
            assert!((heavy(a) + exact).abs() < 0.004 * (-a).exp(), "{a}: {}", heavy(a));
            assert!((heavy(-a) - exact).abs() < 0.004 * (-a).exp());
        }
        assert_eq!(heavy(40.0), -1.0);
        assert_eq!(heavy(0.0), heavy(-0.0), "0 goes with the positives either way");
        // The gains: drive 0 and tone 35 give the Overdrive 35 / 300 + 2 / 3 before the curve and the fast 10^0 = 0.99904
        // after; drive 50 gives 4 x 25^0.5 - 3 = 17; the wet share 48 at level 1 splits 0.48 / 0.52.
        let d = |kind, drive, tone, rect, wet| crate::elite_sound::DistortionNow { pre: [super::super::EqNow::OFF; 3], post: [super::super::EqNow::OFF; 3], kind, drive, tone, rectification: rect, level: 1.0, wet };
        let g = DistortionGains::of(&d(DISTORTION_OVERDRIVE, 0.0, 35.0, 0.0, 48.0));
        assert!((g.pre - (35.0 / 300.0 + 2.0 / 3.0)).abs() < 1e-6 && (g.post - 0.999_04).abs() < 1e-5, "{g:?}");
        assert!((g.wet - 0.48).abs() < 1e-6 && (g.dry - 0.52).abs() < 1e-6);
        let g = DistortionGains::of(&d(DISTORTION_HEAVY, 50.0, 0.0, 75.0, 0.0));
        assert!((g.pre - 17.0).abs() < 1e-4 && (g.post - 10f32.powf(-0.5)).abs() < 0.003, "{g:?}");
        assert!((g.floor - 0.0).abs() < 1e-7 && (g.fold - 0.5).abs() < 1e-6 && g.wet == 0.0 && g.dry == 1.0);
        assert!((DistortionGains::of(&d(DISTORTION_OVERDRIVE, 0.0, 0.0, 10.0, 0.0)).floor + 0.8).abs() < 1e-6);
    }

    /// The Guitar Distortion on a signal: with no wet share it is the output level and the DC blocker alone (`R = 1 -
    /// 251.327 / fs`); a rectification of 50 with the full wet share and no curve is a half-wave rectifier; a new level
    /// ramps in over one buffer, and the first buffer starts at its own values.
    #[test]
    fn a_distortion_runs_as_the_exe_does() {
        let fs = 48000.0;
        let d = |kind, rect, level: f32, wet| crate::elite_sound::DistortionNow { pre: [super::super::EqNow::OFF; 3], post: [super::super::EqNow::OFF; 3], kind, drive: 20.0, tone: 50.0, rectification: rect, level, wet };
        let input: Vec<f32> = (0..4096).map(|i| ((i as f32) * 0.05).sin() * 0.7 + 0.1).collect();
        let dc = |xs: &[f32], gain: f32| {
            let r = 1.0 - 251.327_42 / fs;
            let (mut x1, mut y1) = (0.0f32, 0.0f32);
            xs.iter().map(|x| {
                let y = x * gain - x1 + r * y1;
                (x1, y1) = (x * gain, y);
                y
            }).collect::<Vec<f32>>()
        };
        let run = |p: &crate::elite_sound::DistortionNow, xs: &[f32]| {
            let mut s = Distortion::new(1, fs);
            s.set(p);
            xs.iter().map(|x| {
                let mut f = [*x];
                s.frame(&mut f);
                f[0]
            }).collect::<Vec<f32>>()
        };
        let dry = run(&d(DISTORTION_OVERDRIVE, 30.0, 2.0, 0.0), &input);
        let want = dc(&input, 2.0);
        assert!(dry.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-5), "wet 0: the level and the DC blocker");
        let rect = run(&d(DISTORTION_NONE, 50.0, 1.0, 100.0), &input);
        let halved: Vec<f32> = input.iter().map(|x| x.max(0.0)).collect();
        let want = dc(&halved, 1.0);
        assert!(rect.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-5), "rectification 50: half-wave");
        // A level change asked during the first buffer: the second buffer ramps from 1 to 3, the third holds 3.
        let n = BUFFER_FRAMES as usize;
        let mut s = Distortion::new(1, fs);
        s.set(&d(DISTORTION_NONE, 0.0, 1.0, 0.0));
        let mut out = Vec::new();
        for (i, x) in input.iter().take(3 * n).enumerate() {
            if i == 10 {
                s.set(&d(DISTORTION_NONE, 0.0, 3.0, 0.0));
            }
            let mut f = [*x];
            s.frame(&mut f);
            out.push(f[0]);
        }
        let gained: Vec<f32> = input.iter().take(3 * n).enumerate().map(|(i, x)| x * if i < n { 1.0 } else if i < 2 * n { 1.0 + 2.0 * (i - n) as f32 / n as f32 } else { 3.0 }).collect();
        let want = dc(&gained, 1.0);
        assert!(out.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-4), "the level ramps over the next buffer");
    }

    #[test]
    fn sections_pass_and_stop_what_they_should() {
        // The output's RMS over the input's, once settled.
        let gain = |hz: f32, c: &Coefs| {
            let mut s = Section::default();
            let (mut a, mut b) = (0.0f64, 0.0f64);
            for i in 0..48000 {
                let x = (std::f64::consts::TAU * f64::from(hz) * f64::from(i) / 48000.0).sin() as f32;
                let y = s.run(c, x);
                if i >= 24000 {
                    a += f64::from(y * y);
                    b += f64::from(x * x);
                }
            }
            (a / b).sqrt() as f32
        };
        let lp = coefs(1000.0, false, 48000.0).unwrap();
        let hp = coefs(1000.0, true, 48000.0).unwrap();
        assert_eq!(coefs(f32::NAN, false, 48000.0), None);
        assert_eq!(coefs(0.0, true, 48000.0), None);
        // -3 dB at the cutoff (Butterworth), near 1 an octave away on the pass side, small two octaves on the stop side.
        assert!((gain(1000.0, &lp) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
        assert!((gain(1000.0, &hp) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
        assert!(gain(250.0, &lp) > 0.99 && gain(4000.0, &lp) < 0.07, "{} {}", gain(250.0, &lp), gain(4000.0, &lp));
        assert!(gain(4000.0, &hp) > 0.99 && gain(250.0, &hp) < 0.07, "{} {}", gain(4000.0, &hp), gain(250.0, &hp));
    }
}
