//! Janus and Epimetheus (`seOrbitNoy`, catalog name `Noyelles`).
//!
//! Series for the co-orbital pair in five arguments `A_k = f_k tau + phi_k`
//! (`tau` in Julian years from J2000): the semi-major axis (km), the mean
//! longitude, `(k, h)` and `(q, p)`. Each term has up to three signed multipliers
//! of the arguments and an amplitude. The time argument is days from J2000.

use super::{elliptic_position, frame_g, mat_mul, rot_y, rot_z, row_apply, ExeImage, Mat3};

const EPOCH_JD: f64 = 2_451_545.0;
const OBJ0: u64 = 0x1_4076_4100;
const OBJ_STRIDE: u64 = 0x130;
const VTABLE: u64 = 0x1_4068_0300;
/// Y rotation, Z rotation (the third is a fixed -pi/2 about Y).
const ANGLE_ADDRS: [u64; 3] = [0x1_406d_4a60, 0x1_406d_4810, 0x1_406d_5ec0];

/// Validity range (JD, years 1950 to 2050); outside it SpaceEngine continues the
/// orbit as a Keplerian one.
pub const VALID_JD: (f64, f64) = (2_433_283.0, 2_469_808.0);

pub const BODIES: [&str; 2] = ["janus", "epimetheus"];

#[derive(Clone, Copy)]
struct Term {
    mult: [i8; 3],
    amp: f64,
}

struct Sat {
    /// `f[0..5]`, `phi[0..5]`
    par: [f64; 10],
    /// Offset added to the mean longitude.
    lon0: f64,
    a: Vec<Term>,
    lam: Vec<Term>,
    z: Vec<Term>,
    zeta: Vec<Term>,
}

#[derive(Debug)]
pub struct Noyelles {
    sats: Vec<Sat>,
    matrix: Mat3,
}

impl std::fmt::Debug for Sat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sat({} terms)", self.a.len() + self.lam.len() + self.z.len() + self.zeta.len())
    }
}

fn read_terms(exe: &ExeImage, ptr: u64, n: i32) -> Option<Vec<Term>> {
    if !(0..100_000).contains(&n) {
        return None;
    }
    (0..n as u64)
        .map(|i| {
            let a = ptr + 24 * i;
            let b = exe.slice(a, 3)?;
            Some(Term { mult: [b[0] as i8, b[1] as i8, b[2] as i8], amp: exe.f64(a + 8)? })
        })
        .collect()
}

impl Noyelles {
    pub fn load(exe: &ExeImage) -> Option<Self> {
        let mut sats = Vec::new();
        for k in 0..2u64 {
            let o = OBJ0 + OBJ_STRIDE * k;
            if exe.u64(o)? != VTABLE {
                return None;
            }
            let mut par = [0.0; 10];
            par.copy_from_slice(&exe.f64s(exe.u64(o + 0x78)?, 10)?);
            sats.push(Sat {
                par,
                lon0: exe.f64(o + 0xb0)?,
                a: read_terms(exe, exe.u64(o + 0x80)?, exe.i32(o + 0xa0)?)?,
                lam: read_terms(exe, exe.u64(o + 0x88)?, exe.i32(o + 0xa4)?)?,
                z: read_terms(exe, exe.u64(o + 0x90)?, exe.i32(o + 0xa8)?)?,
                zeta: read_terms(exe, exe.u64(o + 0x98)?, exe.i32(o + 0xac)?)?,
            });
        }
        // Sentinel: first frequency is 3304.0143278 rad/year.
        if (sats[0].par[0] - 3304.014_327_8).abs() > 1e-6 {
            return None;
        }
        let [a, b, g] = [exe.f64(ANGLE_ADDRS[0])?, exe.f64(ANGLE_ADDRS[1])?, exe.f64(ANGLE_ADDRS[2])?];
        let m = mat_mul(&mat_mul(&mat_mul(&rot_y(g), &rot_z(b)), &rot_y(a)), &frame_g());
        Some(Self { sats, matrix: m })
    }

    /// Position relative to Saturn in the internal frame, AU.
    pub fn position(&self, body: &str, jd: f64) -> Option<[f64; 3]> {
        let k = BODIES.iter().position(|b| *b == body)?;
        let s = &self.sats[k];
        let tau = (jd - EPOCH_JD) / 365.25;
        let p = &s.par;
        let arg: Vec<f64> = (0..5).map(|i| tau * p[i] + p[5 + i]).collect();
        // Semi-major axis: element 0 only supplies the constant term.
        let mut a = 0.0;
        for tm in s.a.iter().skip(1).rev() {
            let ph = tm.mult[1] as f64 * arg[1] + tm.mult[0] as f64 * arg[0] + tm.mult[2] as f64 * arg[2];
            a += ph.cos() * tm.amp;
        }
        a += s.a.first()?.amp;
        let mut lam = 0.0;
        for tm in s.lam.iter().rev() {
            lam += (tm.mult[2] as f64 * arg[4] + tm.mult[1] as f64 * arg[1]).sin() * tm.amp;
        }
        let lam = arg[0] + s.lon0 + lam;
        let (mut kk, mut hh) = (0.0, 0.0);
        for tm in s.z.iter().rev() {
            let ph = tm.mult[1] as f64 * arg[1] + tm.mult[0] as f64 * arg[0] + tm.mult[2] as f64 * arg[2];
            kk += ph.cos() * tm.amp;
            hh += ph.sin() * tm.amp;
        }
        let (mut qq, mut pp) = (0.0, 0.0);
        for tm in s.zeta.iter().rev() {
            let ph = tm.mult[2] as f64 * arg[4] + tm.mult[1] as f64 * arg[3];
            qq += ph.cos() * tm.amp;
            pp += ph.sin() * tm.amp;
        }
        let pos = elliptic_position(a / 149_597_870.7, [lam, kk, hh, qq, pp]);
        Some(row_apply(pos, &self.matrix))
    }
}
