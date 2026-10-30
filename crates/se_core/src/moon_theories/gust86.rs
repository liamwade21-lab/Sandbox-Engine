//! GUST86: analytic theory of the five major Uranian satellites
//! (Laskar and Jacobson 1987, A&A 188, 212), as implemented in `seOrbitGUST86`.
//!
//! Per satellite the executable holds three trigonometric series whose phases are
//! integer combinations (signed bytes) of the mean-longitude (`N`) and
//! periapsis-line (`E`) arguments of all five satellites, the five inclination
//! arguments `I` with one amplitude each for `(q, p)`, and a block of mean
//! elements `[dN/dt, dE/dt, dI/dt, N0, E0, I0, n0]`. The time argument is days
//! from JD 2444239.5.

use super::{elliptic_position, mat_mul, rot_y, rot_z, row_apply, frame_g, ExeImage, Mat3, TWO_PI};

/// Epoch of the time argument (JD, 1980 Jan 1.0).
const EPOCH_JD: f64 = 2_444_239.5;
/// First satellite object (`seOrbitGUST86`, 0x100 apart).
const OBJ0: u64 = 0x1_4079_b880;
const OBJ_STRIDE: u64 = 0x100;
const VTABLE: u64 = 0x1_4068_9128;
/// Euler angles of the Uranus-equator -> internal frame chain.
const ANGLE_ADDRS: [u64; 3] = [0x1_406d_5ee8, 0x1_406d_4c30, 0x1_406d_5ec0];

pub const BODIES: [&str; 5] = ["miranda", "ariel", "umbriel", "titania", "oberon"];

struct Term {
    mult: [i8; 10],
    amp: f64,
}

struct Sat {
    gm: f64,
    /// `[fqn, fqe, fqi, phn, phe, phi, n0]`
    par: [f64; 7],
    n_series: Vec<Term>,
    lam_series: Vec<Term>,
    z_series: Vec<Term>,
    zeta_amp: [f64; 5],
}

#[derive(Debug)]
pub struct Gust86 {
    sats: Vec<Sat>,
    matrix: Mat3,
}

impl std::fmt::Debug for Sat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sat(gm {:e})", self.gm)
    }
}

fn read_terms(exe: &ExeImage, ptr: u64, n: usize) -> Option<Vec<Term>> {
    (0..n as u64)
        .map(|i| {
            let a = ptr + 24 * i;
            let b = exe.slice(a, 10)?;
            let mut mult = [0i8; 10];
            for (m, v) in mult.iter_mut().zip(b) {
                *m = *v as i8;
            }
            Some(Term { mult, amp: exe.f64(a + 16)? })
        })
        .collect()
}

impl Gust86 {
    pub fn load(exe: &ExeImage) -> Option<Self> {
        let mut sats = Vec::new();
        for k in 0..5u64 {
            let o = OBJ0 + OBJ_STRIDE * k;
            if exe.u64(o)? != VTABLE {
                return None;
            }
            let gm = exe.f64(o + 0x70)?;
            let series = exe.u64(o + 0x78)?;
            let zeta = exe.u64(o + 0x80)?;
            let par_p = exe.u64(o + 0x88)?;
            let mut s = Vec::new();
            for i in 0..3u64 {
                let (p, n) = exe.vec_header(series + 16 * i)?;
                s.push(read_terms(exe, p, n)?);
            }
            let zeta_p = exe.u64(zeta)?;
            let mut zeta_amp = [0.0; 5];
            zeta_amp.copy_from_slice(&exe.f64s(zeta_p, 5)?);
            let mut par = [0.0; 7];
            par.copy_from_slice(&exe.f64s(par_p, 7)?);
            let z = s.pop()?;
            let l = s.pop()?;
            let n = s.pop()?;
            sats.push(Sat { gm, par, n_series: n, lam_series: l, z_series: z, zeta_amp });
        }
        // Sentinel: Miranda's mean motion is 4.44519055 rad/day.
        if (sats[0].par[0] - 4.445_190_55).abs() > 1e-9 {
            return None;
        }
        let [a, b, g] = [exe.f64(ANGLE_ADDRS[0])?, exe.f64(ANGLE_ADDRS[1])?, exe.f64(ANGLE_ADDRS[2])?];
        let m = mat_mul(&mat_mul(&mat_mul(&rot_y(g), &rot_z(b)), &rot_y(a)), &frame_g());
        Some(Self { sats, matrix: m })
    }

    /// Position relative to Uranus in the internal frame, AU.
    pub fn position(&self, body: &str, jd: f64) -> Option<[f64; 3]> {
        let k = BODIES.iter().position(|b| *b == body)?;
        let t = jd - EPOCH_JD;
        let mut arg_n = [0.0; 5];
        let mut arg_e = [0.0; 5];
        let mut arg_i = [0.0; 5];
        for (i, s) in self.sats.iter().enumerate() {
            let p = &s.par;
            arg_n[i] = (t * p[0] + p[3]) % TWO_PI;
            arg_e[i] = (t * p[1] + p[4]) % TWO_PI;
            arg_i[i] = (t * p[2] + p[5]) % TWO_PI;
        }
        let phase = |m: &[i8; 10]| -> f64 {
            let mut x = 0.0;
            for i in 0..5 {
                if m[i] != 0 {
                    x += m[i] as f64 * arg_n[i];
                }
                if m[5 + i] != 0 {
                    x += m[5 + i] as f64 * arg_e[i];
                }
            }
            x
        };
        let s = &self.sats[k];
        let n = s.n_series.iter().map(|tm| phase(&tm.mult).cos() * tm.amp).sum::<f64>() + s.par[6];
        let lam = s.par[0] * t + s.par[3] + s.lam_series.iter().map(|tm| phase(&tm.mult).sin() * tm.amp).sum::<f64>();
        let (mut kk, mut hh) = (0.0, 0.0);
        for tm in &s.z_series {
            let ph = phase(&tm.mult);
            kk += ph.cos() * tm.amp;
            hh += ph.sin() * tm.amp;
        }
        let (mut qq, mut pp) = (0.0, 0.0);
        for (i, amp) in s.zeta_amp.iter().enumerate() {
            qq += arg_i[i].cos() * amp;
            pp += arg_i[i].sin() * amp;
        }
        let a = (s.gm / (n * n)).cbrt();
        let p = elliptic_position(a, [lam, kk, hh, qq, pp]);
        // The executable negates the third component before the frame matrix.
        Some(row_apply([p[0], p[1], -p[2]], &self.matrix))
    }
}
