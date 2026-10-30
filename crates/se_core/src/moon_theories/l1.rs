//! L1: analytic theory of the Galilean satellites (Lainey, Vienne and Duriez),
//! as implemented in `seOrbitL1`.
//!
//! Per satellite the executable holds four series of `amp * cos(freq * t + phase)`
//! terms (semi-major axis, mean longitude, `(k, h)` and `(q, p)`), six constant
//! offsets, a linear mean-motion term and a small Chebyshev correction
//! (nine coefficients for each of five elements) that fades out beyond
//! about +-705 years from 1950. The time argument is days from JD 2433282.5.

use super::{elliptic_position, frame_g, mat_mul, rot_y, rot_z, row_apply, ExeImage, Mat3};

const EPOCH_JD: f64 = 2_433_282.5;
const OBJ0: u64 = 0x1_4079_f6a0;
const OBJ_STRIDE: u64 = 0x110;
const VTABLE: u64 = 0x1_4068_9290;
/// Euler angles (Y, Z, Y order of the exe's locals: inner, middle, outer).
const ANGLE_ADDRS: [u64; 3] = [0x1_406d_5f00, 0x1_406d_4960, 0x1_406d_4dc0];
/// Chebyshev window constants (`DAT_1406d5488`, `5348`, `5160`, `5f38`, `52e8`).
const CHEB_LIMIT: f64 = 257_501.25;
const CHEB_FADE: f64 = 3_652.5;
const CHEB_SCALE1: f64 = 182.625;
const CHEB_OFFSET: f64 = -7.005_831_604_495_938;
const CHEB_SCALE2: f64 = 1_632.449_445_585_216;

/// Validity range of the theory (JD, years 1550 to 2350); outside it SpaceEngine
/// switches to a Keplerian continuation.
pub const VALID_JD: (f64, f64) = (2_287_196.0, 2_579_380.0);

pub const BODIES: [&str; 4] = ["io", "europa", "ganymede", "callisto"];

#[derive(Clone, Copy)]
struct Term {
    amp: f64,
    phase: f64,
    freq: f64,
}

struct Sat {
    /// Mean-motion term added to the mean longitude (rad/day).
    rate: f64,
    series: [Vec<Term>; 4],
    cheb: Vec<f64>,
    consts: [f64; 6],
}

#[derive(Debug)]
pub struct L1 {
    sats: Vec<Sat>,
    matrix: Mat3,
}

impl std::fmt::Debug for Sat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sat(rate {})", self.rate)
    }
}

fn read_terms(exe: &ExeImage, ptr: u64, n: usize) -> Option<Vec<Term>> {
    (0..n as u64)
        .map(|i| {
            let a = ptr + 24 * i;
            Some(Term { amp: exe.f64(a)?, phase: exe.f64(a + 8)?, freq: exe.f64(a + 16)? })
        })
        .collect()
}

/// `sum amp * cos(freq t + phase)` summed from the last term to the first.
fn cos_sum(terms: &[Term], t: f64) -> f64 {
    terms.iter().rev().fold(0.0, |acc, tm| acc + (t * tm.freq + tm.phase).cos() * tm.amp)
}

fn sin_cos_sums(terms: &[Term], t: f64) -> (f64, f64) {
    let (mut s, mut c) = (0.0, 0.0);
    for tm in terms.iter().rev() {
        let (sn, cs) = (t * tm.freq + tm.phase).sin_cos();
        s += sn * tm.amp;
        c += cs * tm.amp;
    }
    (s, c)
}

impl L1 {
    pub fn load(exe: &ExeImage) -> Option<Self> {
        let mut sats = Vec::new();
        for k in 0..4u64 {
            let o = OBJ0 + OBJ_STRIDE * k;
            if exe.u64(o)? != VTABLE {
                return None;
            }
            let hdr = exe.u64(o + 0x80)?;
            let mut series: [Vec<Term>; 4] = Default::default();
            for (i, s) in series.iter_mut().enumerate() {
                let (p, n) = exe.vec_header(hdr + 16 * i as u64)?;
                *s = read_terms(exe, p, n)?;
            }
            let mut consts = [0.0; 6];
            consts.copy_from_slice(&exe.f64s(exe.u64(o + 0x90)?, 6)?);
            sats.push(Sat {
                rate: exe.f64(o + 0x78)?,
                series,
                cheb: exe.f64s(exe.u64(o + 0x88)?, 45)?,
                consts,
            });
        }
        // Sentinel: Io's mean motion is 3.5515522861824 rad/day.
        if (sats[0].rate - 3.551_552_286_182_4).abs() > 1e-9 {
            return None;
        }
        let [a, b, g] = [exe.f64(ANGLE_ADDRS[0])?, exe.f64(ANGLE_ADDRS[1])?, exe.f64(ANGLE_ADDRS[2])?];
        let m = mat_mul(&mat_mul(&mat_mul(&rot_y(g), &rot_z(b)), &rot_y(a)), &frame_g());
        Some(Self { sats, matrix: m })
    }

    /// Position relative to Jupiter in the internal frame, AU.
    pub fn position(&self, body: &str, jd: f64, chebyshev: bool) -> Option<[f64; 3]> {
        let k = BODIES.iter().position(|b| *b == body)?;
        let s = &self.sats[k];
        let t = jd - EPOCH_JD;
        let mut a = cos_sum(&s.series[0], t);
        let mut lam = cos_sum(&s.series[1], t);
        let (mut h, mut kk) = sin_cos_sums(&s.series[2], t);
        let (mut p, mut q) = sin_cos_sums(&s.series[3], t);
        let c = &s.consts;
        a += c[0];
        lam += c[1];
        kk += c[2];
        h += c[3];
        q += c[4];
        p += c[5];
        let taper = (CHEB_LIMIT - t.abs()) / CHEB_FADE;
        if chebyshev && taper > 0.0 {
            let taper = taper.min(1.0);
            let x = (t / CHEB_SCALE1 - CHEB_OFFSET) / CHEB_SCALE2;
            let mut tn = [0.0; 9];
            tn[0] = 1.0;
            tn[1] = x;
            for j in 2..9 {
                tn[j] = 2.0 * x * tn[j - 1] - tn[j - 2];
            }
            let corr = |e: usize| -> f64 { (0..9).map(|j| tn[j] * s.cheb[5 * j + e]).sum::<f64>() * taper };
            a += corr(0);
            lam += corr(1);
            kk += corr(2);
            h += corr(3);
            q += corr(4);
        }
        lam += t * s.rate;
        let pos = elliptic_position(a, [lam, kk, h, q, p]);
        Some(row_apply(pos, &self.matrix))
    }
}
