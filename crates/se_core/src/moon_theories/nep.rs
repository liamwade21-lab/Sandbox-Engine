//! Triton (`seOrbitNEP`, catalog name `Emelyanov`).
//!
//! A single-satellite model: a circular orbit of fixed radius whose inclination,
//! node and longitude get periodic corrections from seven terms in two Neptune
//! arguments, `N = 258.329018 + 0.00598182615 t` and `x = 200.788305 - Omega`
//! (degrees). `Omega = 72.89882654 + 0.001433819551 s` precesses linearly.
//! `s` is days from JD 2378520.5 (1800 Jan 1.5), `t` days from J2000.

use super::{frame_g, mat_mul, rot_y, rot_z, row_apply, ExeImage, Mat3};

const EPOCH_S: f64 = 2_378_520.5;
const EPOCH_T: f64 = 2_451_545.0;
const OBJ: u64 = 0x1_4079_fc00;
const VTABLE: u64 = 0x1_4068_92b8;
const ANGLE_ADDRS: [u64; 2] = [0x1_406d_4a48, 0x1_406d_5f18];
/// `DAT_1406d4620`, `DAT_1406d51b8`, `DAT_1406d5178` (degrees, degrees/day).
const N_RATE: f64 = 0.005_981_826_15;
const N_0: f64 = 258.329_018;
const X_0: f64 = 200.788_305;

struct Term {
    inc_amp: f64,
    lon_amp: f64,
    node_amp: f64,
    n_mult: i32,
    x_mult: i32,
}

#[derive(Debug)]
pub struct Nep {
    /// `[radius km, inclination0, longitude0, mean motion deg/day, node0, node rate]`
    c: [f64; 6],
    terms: Vec<Term>,
    matrix: Mat3,
}

impl std::fmt::Debug for Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Term({}, {})", self.n_mult, self.x_mult)
    }
}

impl Nep {
    pub fn load(exe: &ExeImage) -> Option<Self> {
        if exe.u64(OBJ)? != VTABLE {
            return None;
        }
        let mut c = [0.0; 6];
        c.copy_from_slice(&exe.f64s(exe.u64(OBJ + 0x88)?, 6)?);
        let (tp, tn) = exe.vec_header(exe.u64(OBJ + 0x90)?)?;
        let terms = (0..tn as u64)
            .map(|i| {
                let a = tp + 32 * i;
                Some(Term {
                    inc_amp: exe.f64(a)?,
                    lon_amp: exe.f64(a + 8)?,
                    node_amp: exe.f64(a + 16)?,
                    n_mult: exe.i32(a + 24)?,
                    x_mult: exe.i32(a + 28)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        // Sentinel: Triton's orbital radius is 354758.98 km.
        if (c[0] - 354_758.98).abs() > 1e-6 || terms.len() != 7 {
            return None;
        }
        let [a, b] = [exe.f64(ANGLE_ADDRS[0])?, exe.f64(ANGLE_ADDRS[1])?];
        Some(Self { c, terms, matrix: mat_mul(&mat_mul(&rot_z(b), &rot_y(a)), &frame_g()) })
    }

    /// Position relative to Neptune in the internal frame, AU.
    pub fn position(&self, body: &str, jd: f64) -> Option<[f64; 3]> {
        if body != "triton" {
            return None;
        }
        let s = jd - EPOCH_S;
        let t = jd - EPOCH_T;
        let c = &self.c;
        let d2r = std::f64::consts::PI / 180.0;
        let n_arg = t * N_RATE + N_0;
        let node0 = s * c[5] + c[4];
        let x_arg = X_0 - node0;
        let (mut inc, mut lon, mut node) = (0.0, 0.0, 0.0);
        for tm in self.terms.iter().rev() {
            let ph = (tm.n_mult as f64 * n_arg + tm.x_mult as f64 * x_arg) * d2r;
            let (sn, cs) = ph.sin_cos();
            inc += cs * tm.inc_amp;
            lon += sn * tm.lon_amp;
            node += sn * tm.node_amp;
        }
        let inc = (inc + c[1]) * d2r;
        let lon = (lon + s * c[3] + c[2]) * d2r;
        let node = (node + node0) * d2r;
        let r = c[0] / 149_597_870.7;
        let pos = [lon.cos() * r, lon.sin() * r, 0.0];
        let (sn, cn) = node.sin_cos();
        let (si, ci) = inc.sin_cos();
        let rot: Mat3 = [cn, sn, 0.0, -sn * ci, cn * ci, si, 0.0, 0.0, 0.0];
        Some(row_apply(pos, &mat_mul(&rot, &self.matrix)))
    }
}
