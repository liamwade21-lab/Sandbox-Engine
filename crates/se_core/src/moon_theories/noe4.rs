//! NOE-4-2007: Mars satellites Phobos and Deimos (`seOrbitNOE4`).
//!
//! SpaceEngine evaluates a mean ellipse: constant `(a, k, h, q, p)` and a mean
//! longitude with a quadratic term, `lambda = lambda0 + (n + rate * s) * s`, where
//! `s` is days from J2000 plus 6491.5. (The executable also walks four trigonometric
//! series for these elements but overwrites the results with the constants, so
//! they have no effect and are not reproduced.) The orbit frame follows the IAU
//! Mars pole, which moves linearly in time.

use super::{elliptic_position, frame_g, mat_mul, rot_y, rot_z, row_apply, ExeImage};

const EPOCH_JD: f64 = 2_451_545.0;
const OBJ0: u64 = 0x1_407a_10c0;
const OBJ_STRIDE: u64 = 0x110;
const VTABLE: u64 = 0x1_4068_92e0;
/// Mean-element epoch offset in days (`DAT_1406d5398`).
const ELEMENT_SHIFT: f64 = 6_491.5;
/// IAU Mars pole (`DAT_1406d51e0` etc.): RA 317.68143 - 0.1061 T, Dec 52.8865 - 0.0609 T,
/// T in Julian centuries from J2000.
const POLE_RA: f64 = 317.681_43;
const POLE_RA_RATE: f64 = 0.1061;
const POLE_DEC: f64 = 52.886_5;
const POLE_DEC_RATE: f64 = 0.0609;

/// Validity range (JD, years -3000 to 7000).
pub const VALID_JD: (f64, f64) = (625_308.0, 4_277_758.0);

pub const BODIES: [&str; 2] = ["phobos", "deimos"];

struct Sat {
    /// Mean motion, rad/day.
    n: f64,
    /// Secular acceleration of the mean motion.
    rate: f64,
    /// `[a (AU), lambda0, k, h, q, p]`
    consts: [f64; 6],
}

#[derive(Debug)]
pub struct Noe4 {
    sats: Vec<Sat>,
}

impl std::fmt::Debug for Sat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sat(n {})", self.n)
    }
}

impl Noe4 {
    pub fn load(exe: &ExeImage) -> Option<Self> {
        let mut sats = Vec::new();
        for k in 0..2u64 {
            let o = OBJ0 + OBJ_STRIDE * k;
            if exe.u64(o)? != VTABLE {
                return None;
            }
            let mut consts = [0.0; 6];
            consts.copy_from_slice(&exe.f64s(exe.u64(o + 0x88)?, 6)?);
            sats.push(Sat { n: exe.f64(o + 0x78)?, rate: exe.f64(o + 0x80)?, consts });
        }
        // Sentinel: Phobos' mean motion is 19.7020556283139 rad/day.
        if (sats[0].n - 19.702_055_628_313_9).abs() > 1e-9 {
            return None;
        }
        Some(Self { sats })
    }

    /// Position relative to Mars in the internal frame, AU.
    pub fn position(&self, body: &str, jd: f64) -> Option<[f64; 3]> {
        let k = BODIES.iter().position(|b| *b == body)?;
        let s = &self.sats[k];
        let t = jd - EPOCH_JD;
        let a1 = std::f64::consts::FRAC_PI_2 - (POLE_RA - t * POLE_RA_RATE / 36_525.0).to_radians();
        let a2 = std::f64::consts::FRAC_PI_2 - (POLE_DEC - t * POLE_DEC_RATE / 36_525.0).to_radians();
        let m = mat_mul(
            &mat_mul(&mat_mul(&rot_y(-std::f64::consts::FRAC_PI_2), &rot_z(a2)), &rot_y(a1)),
            &frame_g(),
        );
        let ts = t + ELEMENT_SHIFT;
        let c = &s.consts;
        let lam = (ts * s.rate + s.n) * ts + c[1];
        let pos = elliptic_position(c[0], [lam, c[2], c[3], c[4], c[5]]);
        Some(row_apply(pos, &m))
    }
}
