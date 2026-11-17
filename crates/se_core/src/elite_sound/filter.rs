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

/// `10^(gain_db / 40)` as the exe works it out for the EQ's shelves and peaks (`FUN_144a05110`): `x = gain * 0.025`, a
/// float's bits from `x * 2^23 log2(10) + 127 * 2^23`, then the same quadratic on the mantissa as the low-pass's fast power
/// of two. None below `x = -37` (the exe leaves the factor at 0 there; no band of the data comes near: gains are -24..24).
fn eq_amplitude(gain_db: f32) -> Option<f32> {
    let x = gain_db * 0.025;
    if !(x >= -37.0) {
        return None;
    }
    let bits = (x * 27_866_352.0_f32 + 1_065_353_216.0_f32) as i64 as u32;
    let m = f32::from_bits((bits & 0x007f_ffff) + 0x3f80_0000);
    let e = f32::from_bits(bits & 0xff80_0000);
    Some(((m * 0.325_189_77 + 0.020_805_772) * m + 0.653_043_45) * e)
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
