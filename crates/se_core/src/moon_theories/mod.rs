//! SpaceEngine's analytic ephemeris theories for planetary moons.
//!
//! The catalog names a theory per moon (`Orbit { AnalyticModel "Io-L1" ... }`) and
//! SpaceEngine evaluates it instead of the plain Keplerian elements. Each theory is
//! a C++ class in the executable whose coefficient tables live in `.data`. This
//! module re-implements the evaluation (found by decompiling the executable, see
//! `docs/formats/moon_theories.md`) and loads the tables from the user's install
//! at runtime ([`exe::ExeImage`]), so no table is stored in the repository.
//!
//! Theories (catalog `AnalyticModel` suffix):
//! - `GUST86`: Miranda, Ariel, Umbriel, Titania, Oberon (Laskar and Jacobson 1987)
//! - `L1`: Io, Europa, Ganymede, Callisto (Lainey et al.)
//! - `TASS17`: Mimas to Iapetus (Vienne and Duriez, TASS 1.7)
//! - `NOE-4`: Phobos, Deimos
//! - `HTC20`: Helene, Calypso, Telesto
//! - `Noyelles`: Janus, Epimetheus
//! - `Emelyanov`: Triton
//!
//! Every theory yields planet-centred elements `(a or n, lambda, k, h, q, p)`, solves
//! Kepler's equation and rotates the result into SpaceEngine's internal ecliptic
//! frame. [`MoonTheories::offset_m`] returns the universe-frame offset in meters.

use glam::DVec3;
use std::path::Path;

pub mod exe;
mod gust86;
mod htc20;
mod l1;
mod noe4;
mod nep;
mod noyelles;
mod tass;

pub use exe::ExeImage;

use crate::coords::ecliptic_to_universe;
use crate::units::METERS_PER_AU;

/// Row-major 3x3 matrix, laid out exactly as SpaceEngine stores its frame matrices.
pub(crate) type Mat3 = [f64; 9];

/// `a * b` in SpaceEngine's row-major convention.
pub(crate) fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut o = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            o[3 * r + c] = a[3 * r] * b[c] + a[3 * r + 1] * b[3 + c] + a[3 * r + 2] * b[6 + c];
        }
    }
    o
}

/// Row vector times matrix: `out_j = sum_i p_i m[3i + j]`.
pub(crate) fn row_apply(p: [f64; 3], m: &Mat3) -> [f64; 3] {
    let mut o = [0.0; 3];
    for (j, v) in o.iter_mut().enumerate() {
        *v = p[0] * m[j] + p[1] * m[3 + j] + p[2] * m[6 + j];
    }
    o
}

/// Rotation about the (internal) Y axis, SpaceEngine's matrix layout.
pub(crate) fn rot_y(a: f64) -> Mat3 {
    let (s, c) = a.sin_cos();
    [c, 0.0, s, 0.0, 1.0, 0.0, -s, 0.0, c]
}
pub(crate) fn rot_z(a: f64) -> Mat3 {
    let (s, c) = a.sin_cos();
    [c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0]
}
pub(crate) fn rot_x(a: f64) -> Mat3 {
    let (s, c) = a.sin_cos();
    [1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c]
}

/// SpaceEngine's global frame matrix at `DAT_140854f90`: transpose of the
/// matrix built from Euler angles `(-obliquity, pi/2)`; it carries the
/// equatorial-based theory frames into the internal ecliptic frame.
pub(crate) fn frame_g() -> Mat3 {
    let t1 = -OBLIQUITY_RAD;
    let t2 = std::f64::consts::FRAC_PI_2;
    let (s1, c1) = t1.sin_cos();
    let (s2, c2) = t2.sin_cos();
    let b = [c2 * c1, -s1 * c2, s2, s1, c1, 0.0, -s2 * c1, s1 * s2, c2];
    [b[0], b[3], b[6], b[1], b[4], b[7], b[2], b[5], b[8]]
}

/// J2000 mean obliquity of the ecliptic used by SpaceEngine, radians.
pub(crate) const OBLIQUITY_RAD: f64 = 0.409_092_804_028_403_4;
pub(crate) const TWO_PI: f64 = std::f64::consts::TAU;

/// Position (same unit as `a`) from the elements `[lambda, k, h, q, p]`.
///
/// Laskar's non-singular elements: `k + ih = e exp(i varpi)`,
/// `q + ip = sin(i/2) exp(i Omega)`. The result is in the frame whose
/// Z axis is the orbit-plane reference normal, already permuted into
/// SpaceEngine's `(x, normal, y)` axis order.
pub(crate) fn elliptic_position(a: f64, el: [f64; 5]) -> [f64; 3] {
    let [lam, k, h, qq, pp] = el;
    let lam = lam % TWO_PI;
    let mut f = (lam - lam.sin() * k) + lam.cos() * h;
    for _ in 0..10 {
        let (sf, cf) = f.sin_cos();
        let dl = ((sf * k + (lam - f)) - cf * h) / ((1.0 - cf * k) - sf * h);
        f += dl;
        if dl.abs() <= 1e-14 {
            break;
        }
    }
    let (sf, cf) = f.sin_cos();
    let phi = (1.0 - k * k - h * h).max(0.0).sqrt();
    let psi = 1.0 / (phi + 1.0);
    let t = (cf * h - sf * k) * psi;
    let y1 = ((sf - h) + t * k) * a;
    let x1 = ((cf - k) - t * h) * a;
    let rt = (1.0 - qq * qq - pp * pp).max(0.0).sqrt();
    let two_pq = 2.0 * pp * qq;
    [
        two_pq * y1 + x1 * (1.0 - 2.0 * pp * pp),
        (y1 * qq - x1 * pp) * 2.0 * rt,
        -x1 * two_pq - (1.0 - 2.0 * qq * qq) * y1,
    ]
}

/// The loaded theories.
#[derive(Debug)]
pub struct MoonTheories {
    gust86: Option<gust86::Gust86>,
    l1: Option<l1::L1>,
    tass: Option<tass::Tass>,
    noe4: Option<noe4::Noe4>,
    nep: Option<nep::Nep>,
    htc20: Option<htc20::Htc20>,
    noyelles: Option<noyelles::Noyelles>,
    l1_chebyshev: bool,
}

impl MoonTheories {
    /// Load every theory from `SpaceEngine.exe`; a theory that fails its sanity
    /// checks is left out (those moons fall back to their Keplerian orbit).
    pub fn from_exe(exe: &ExeImage) -> Self {
        Self { gust86: gust86::Gust86::load(exe), l1: l1::L1::load(exe), tass: tass::Tass::load(exe), noe4: noe4::Noe4::load(exe), nep: nep::Nep::load(exe), htc20: htc20::Htc20::load(exe), noyelles: noyelles::Noyelles::load(exe), l1_chebyshev: true }
    }

    /// Enable or disable SpaceEngine's Chebyshev correction of the L1 elements
    /// (default on, as in the executable). The correction shifts the Galilean
    /// moons' orbital radii by up to about 1% (3000 to 13000 km); without it L1
    /// agrees with JPL Horizons to a few hundred km.
    pub fn with_l1_chebyshev(mut self, on: bool) -> Self {
        self.l1_chebyshev = on;
        self
    }

    /// Load from `<install>/system/SpaceEngine.exe`.
    pub fn from_install(install: &Path) -> Result<Self, String> {
        let exe = ExeImage::load(&install.join("system").join("SpaceEngine.exe"))?;
        Ok(Self::from_exe(&exe))
    }

    /// Whether `model` (a catalog `AnalyticModel` string) names a loaded theory.
    pub fn supports(&self, model: &str) -> bool {
        let lower = model.to_ascii_lowercase();
        let Some((body, theory)) = lower.split_once('-') else { return false };
        match theory {
            "gust86" => self.gust86.is_some() && gust86::BODIES.contains(&body),
            "l1" => self.l1.is_some() && l1::BODIES.contains(&body),
            "tass17" => self.tass.is_some() && tass::BODIES.contains(&body),
            "noe-4" => self.noe4.is_some() && noe4::BODIES.contains(&body),
            "htc20" => self.htc20.is_some() && htc20::BODIES.contains(&body),
            "noyelles" => self.noyelles.is_some() && noyelles::BODIES.contains(&body),
            "emelyanov" => self.nep.is_some() && body == "triton",
            _ => false,
        }
    }

    /// Moon position relative to its parent planet in SpaceEngine's internal frame,
    /// astronomical units. `jd` is the Julian date (TDB). `None` when the theory is
    /// not loaded or `jd` is outside its validity range.
    pub fn internal_au(&self, model: &str, jd: f64) -> Option<[f64; 3]> {
        let lower = model.to_ascii_lowercase();
        let (body, theory) = lower.split_once('-')?;
        let within = |r: (f64, f64)| (r.0..=r.1).contains(&jd);
        match theory {
            "gust86" => self.gust86.as_ref()?.position(body, jd),
            "l1" if within(l1::VALID_JD) => self.l1.as_ref()?.position(body, jd, self.l1_chebyshev),
            "tass17" => self.tass.as_ref()?.position(body, jd),
            "noe-4" if within(noe4::VALID_JD) => self.noe4.as_ref()?.position(body, jd),
            "htc20" => self.htc20.as_ref()?.position(body, jd),
            "noyelles" if within(noyelles::VALID_JD) => self.noyelles.as_ref()?.position(body, jd),
            "emelyanov" => self.nep.as_ref()?.position(body, jd),
            _ => None,
        }
    }

    /// Moon position relative to its parent planet, universe-frame meters.
    pub fn offset_m(&self, model: &str, jd: f64) -> Option<DVec3> {
        let [x, y, z] = self.internal_au(model, jd)?;
        // Internal axes: X = ecliptic X, Y = ecliptic north, Z = -ecliptic Y.
        Some(ecliptic_to_universe(DVec3::new(x, -z, y)) * METERS_PER_AU)
    }
}

#[cfg(test)]
mod tests;
