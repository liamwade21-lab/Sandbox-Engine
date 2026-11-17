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

/// At most this, a filter value leaves the signal alone (`FUN_144ac2bf0`).
pub const BYPASS: f32 = 0.1;
/// The cap on a cutoff, as a share of the output sample rate (`DAT_145154fec`).
pub const MOST_SHARE: f32 = 0.45;
/// Under this a section's output is 0 (about -300 dB: far under anything heard, far over the subnormal floats).
const FLUSH: f64 = 1e-15;
/// Over this a section has run away (`Section::run`).
const RUNAWAY: f64 = 16.0;

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

/// One channel's section state (direct form I).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Section {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Section {
    pub fn run(&mut self, c: &Coefs, x: f32) -> f32 {
        let x = if x.is_finite() { f64::from(x) } else { 0.0 };
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        // An output far beyond anything a Butterworth section makes of a full-scale input (its overshoot is a few
        // percent) means it ran away: it starts again rather than hand the player a value that would wreck the mix.
        if !(y.abs() <= RUNAWAY) {
            *self = Section::default();
            return 0.0;
        }
        // A decaying output is flushed to 0 long before it reaches the subnormal floats, whose arithmetic is many times
        // slower.
        let y = if y.abs() < FLUSH { 0.0 } else { y };
        *self = Section { x1: x, x2: self.x1, y1: y, y2: self.y1 };
        y as f32
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
