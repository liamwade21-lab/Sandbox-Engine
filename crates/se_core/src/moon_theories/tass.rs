//! TASS 1.7: analytic theory of the eight main Saturnian satellites
//! (Vienne and Duriez 1995, A&A 297, 588), as implemented in `seOrbitTASS`.
//!
//! The theory gives, for each satellite, the mean motion `n`, mean longitude
//! `lambda`, `z = k + ih` and `zeta = q + ip` as sums of cosine/sine terms. A term
//! group carries seven signed multipliers of the "argument" series (the
//! perturbed mean longitudes of Mimas..Titan and Iapetus, `sum amp sin(f t + ph)`)
//! plus a list of `(amp, phase, freq)` terms. Hyperion (index 7) uses the same
//! structure without an argument series. The time argument is days from JD 2444240.

use super::{elliptic_position, mat_mul, rot_x, rot_y, row_apply, ExeImage, Mat3};

const EPOCH_JD: f64 = 2_444_240.0;
const OBJ0: u64 = 0x1_407a_c3c0;
const OBJ_STRIDE: u64 = 0x120;
const VTABLE: u64 = 0x1_4068_9588;
const ANGLE_ADDRS: [u64; 2] = [0x1_406d_5ef0, 0x1_406d_5e50];

pub const BODIES: [&str; 8] =
    ["mimas", "enceladus", "tethys", "dione", "rhea", "titan", "hyperion", "iapetus"];
/// Objects (indices into `BODIES`) providing the seven argument series.
const ARG_SATS: [usize; 7] = [0, 1, 2, 3, 4, 5, 7];

#[derive(Clone, Copy)]
struct Term {
    amp: f64,
    phase: f64,
    freq: f64,
}

struct Group {
    mult: [i8; 7],
    terms: Vec<Term>,
}

struct Sat {
    gm: f64,
    n0: f64,
    consts: [f64; 6],
    /// Index of the argument series added to the mean longitude (7 = none).
    arg_index: i32,
    /// n, lambda, (k, h), (q, p) series.
    series: [Vec<Group>; 4],
}

#[derive(Debug)]
pub struct Tass {
    sats: Vec<Sat>,
    matrix: Mat3,
}

impl std::fmt::Debug for Sat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sat(gm {:e})", self.gm)
    }
}

fn read_groups(exe: &ExeImage, ptr: u64, n: usize) -> Option<Vec<Group>> {
    (0..n as u64)
        .map(|i| {
            let a = ptr + 24 * i;
            let b = exe.slice(a, 7)?;
            let mut mult = [0i8; 7];
            for (m, v) in mult.iter_mut().zip(b) {
                *m = *v as i8;
            }
            let (tp, tn) = (exe.u64(a + 8)?, exe.i32(a + 16)?);
            if !(0..100_000).contains(&tn) {
                return None;
            }
            let terms = (0..tn as u64)
                .map(|j| {
                    let t = tp + 24 * j;
                    Some(Term { amp: exe.f64(t)?, phase: exe.f64(t + 8)?, freq: exe.f64(t + 16)? })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Group { mult, terms })
        })
        .collect()
}

impl Tass {
    pub fn load(exe: &ExeImage) -> Option<Self> {
        let mut sats = Vec::new();
        for k in 0..8u64 {
            let o = OBJ0 + OBJ_STRIDE * k;
            if exe.u64(o)? != VTABLE {
                return None;
            }
            let mut consts = [0.0; 6];
            consts.copy_from_slice(&exe.f64s(exe.u64(o + 0x80)?, 6)?);
            let s = exe.u64(o + 0x88)?;
            let mut series: [Vec<Group>; 4] = Default::default();
            for (i, v) in series.iter_mut().enumerate() {
                let (p, n) = exe.vec_header(s + 16 * i as u64)?;
                *v = read_groups(exe, p, n)?;
            }
            sats.push(Sat {
                gm: exe.f64(o + 0x70)?,
                n0: exe.f64(o + 0x78)?,
                consts,
                arg_index: exe.i32(o + 0x90)?,
                series,
            });
        }
        // Sentinel: Mimas' mean motion is 6.667061728781588 rad/day.
        if (sats[0].n0 - 6.667_061_728_781_588).abs() > 1e-9 {
            return None;
        }
        let [a, b] = [exe.f64(ANGLE_ADDRS[0])?, exe.f64(ANGLE_ADDRS[1])?];
        Some(Self { sats, matrix: mat_mul(&rot_x(b), &rot_y(a)) })
    }

    /// Position relative to Saturn in the internal frame, AU.
    pub fn position(&self, body: &str, jd: f64) -> Option<[f64; 3]> {
        let k = BODIES.iter().position(|b| *b == body)?;
        let t = jd - EPOCH_JD;
        // Argument series: sum amp * sin(freq t + phase) of each object's first
        // lambda group.
        let mut arg = [0.0; 7];
        for (slot, &si) in arg.iter_mut().zip(ARG_SATS.iter()) {
            let g = self.sats[si].series[1].first()?;
            *slot = g.terms.iter().rev().fold(0.0, |acc, tm| acc + (t * tm.freq + tm.phase).sin() * tm.amp);
        }
        let s = &self.sats[k];
        let base = |m: &[i8; 7]| -> f64 {
            let mut x = 0.0;
            for (i, &mi) in m.iter().enumerate() {
                if mi != 0 {
                    x += mi as f64 * arg[i];
                }
            }
            x
        };
        let mut n = s.consts[0];
        for g in s.series[0].iter().rev() {
            let b = base(&g.mult);
            for tm in g.terms.iter().rev() {
                n += (t * tm.freq + tm.phase + b).cos() * tm.amp;
            }
        }
        let n = (n + 1.0) * s.n0;
        let mut lam = s.consts[1];
        let start = if s.arg_index != 7 {
            lam += arg[s.arg_index as usize];
            1
        } else {
            0
        };
        for g in s.series[1][start.min(s.series[1].len())..].iter().rev() {
            let b = base(&g.mult);
            for tm in g.terms.iter().rev() {
                lam += (t * tm.freq + tm.phase + b).sin() * tm.amp;
            }
        }
        lam += t * s.n0;
        let mut z = [s.consts[2], s.consts[3]];
        let mut zeta = [s.consts[4], s.consts[5]];
        for (acc, series) in [(&mut z, &s.series[2]), (&mut zeta, &s.series[3])] {
            for g in series.iter().rev() {
                let b = base(&g.mult);
                for tm in g.terms.iter().rev() {
                    let ph = t * tm.freq + tm.phase + b;
                    acc[0] += ph.cos() * tm.amp;
                    acc[1] += ph.sin() * tm.amp;
                }
            }
        }
        let a = (s.gm / (n * n)).cbrt();
        let pos = elliptic_position(a, [lam, z[0], z[1], zeta[0], zeta[1]]);
        Some(row_apply(pos, &self.matrix))
    }
}
