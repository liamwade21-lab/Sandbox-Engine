//! HTC 2.0: Saturnian co-orbital satellites Helene, Telesto and Calypso
//! (`seOrbitHTC20`).
//!
//! The theory is a direct Fourier series for the Saturn-centred Cartesian
//! position `(x, y, z)` (J2000 ecliptic, AU) in four arguments
//! `a_k = f_k t + phi_k`. Each term has six amplitudes (three for the position,
//! three for the velocity, which the port does not need), a sine/cosine flag and
//! four signed multipliers. The executable stores the components in its internal
//! frame order `(x, z, -y)`. The time argument is days from J2000.

use super::ExeImage;

const EPOCH_JD: f64 = 2_451_545.0;
const OBJ0: u64 = 0x1_4076_3870;
const OBJ_STRIDE: u64 = 0xd0;
const VTABLE: u64 = 0x1_4068_02d8;

/// Memory order of the objects (not the order they are registered in).
pub const BODIES: [&str; 3] = ["helene", "telesto", "calypso"];

struct Term {
    amp: [f64; 3],
    sine: bool,
    mult: [i8; 4],
}

struct Sat {
    freq: [f64; 4],
    phase: [f64; 4],
    terms: Vec<Term>,
}

#[derive(Debug)]
pub struct Htc20 {
    sats: Vec<Sat>,
}

impl std::fmt::Debug for Sat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sat({} terms)", self.terms.len())
    }
}

impl Htc20 {
    pub fn load(exe: &ExeImage) -> Option<Self> {
        let mut sats = Vec::new();
        for k in 0..3u64 {
            let o = OBJ0 + OBJ_STRIDE * k;
            if exe.u64(o)? != VTABLE {
                return None;
            }
            let par = exe.f64s(exe.u64(o + 0x78)?, 8)?;
            let tp = exe.u64(o + 0x80)?;
            let n = exe.i32(o + 0x88)?;
            if !(0..100_000).contains(&n) {
                return None;
            }
            let terms = (0..n as u64)
                .map(|i| {
                    let a = tp + 56 * i;
                    let b = exe.slice(a + 48, 5)?;
                    // Amplitudes [0], [2], [1] feed internal x, y and -z.
                    Some(Term {
                        amp: [exe.f64(a)?, exe.f64(a + 8)?, exe.f64(a + 16)?],
                        sine: b[0] != 0,
                        mult: [b[1] as i8, b[2] as i8, b[3] as i8, b[4] as i8],
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            sats.push(Sat {
                freq: [par[0], par[1], par[2], par[3]],
                phase: [par[4], par[5], par[6], par[7]],
                terms,
            });
        }
        // Sentinel: first Helene frequency is 2.29427177 rad/day.
        if (sats[0].freq[0] - 2.294_271_77).abs() > 1e-9 {
            return None;
        }
        Some(Self { sats })
    }

    /// Position relative to Saturn in the internal frame, AU.
    pub fn position(&self, body: &str, jd: f64) -> Option<[f64; 3]> {
        let k = BODIES.iter().position(|b| *b == body)?;
        let s = &self.sats[k];
        let t = jd - EPOCH_JD;
        let a: Vec<f64> = (0..4).map(|i| t * s.freq[i] + s.phase[i]).collect();
        let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
        for tm in &s.terms {
            let arg = (tm.mult[1] as f64 * a[1] + tm.mult[0] as f64 * a[0]) + tm.mult[2] as f64 * a[2]
                + tm.mult[3] as f64 * a[3];
            let v = if tm.sine { arg.sin() } else { arg.cos() };
            x += v * tm.amp[0];
            y += v * tm.amp[2];
            z -= v * tm.amp[1];
        }
        Some([x, y, z])
    }
}
