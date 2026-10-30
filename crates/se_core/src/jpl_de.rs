//! Reader for JPL planetary ephemerides in the standard binary format
//! (SpaceEngine ships DE436 as `ephemerides/linux_p1550p2650.436`).
//!
//! The file is a sequence of fixed-size records of `ncoeff` doubles: record 0 is
//! the header (titles, constant names, time span, `IPT` pointers), record 1 the
//! constant values, and each following record holds Chebyshev coefficients for
//! one time interval (32 days for DE436). Positions are in km, ICRF/equatorial,
//! relative to the Solar System barycentre (the Moon is geocentric).

use glam::DVec3;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeError {
    #[error("ephemeris file too small or truncated")]
    Truncated,
    #[error("unsupported or corrupt ephemeris header: {0}")]
    BadHeader(String),
}

/// Items of the ephemeris (indices into `IPT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeBody {
    Mercury = 0,
    Venus = 1,
    /// Earth-Moon barycentre.
    Emb = 2,
    Mars = 3,
    Jupiter = 4,
    Saturn = 5,
    Uranus = 6,
    Neptune = 7,
    Pluto = 8,
    /// Moon relative to the Earth (geocentric).
    MoonGeocentric = 9,
    Sun = 10,
}

pub struct JplEphemeris {
    pub title: String,
    pub start_jd: f64,
    pub end_jd: f64,
    pub interval_days: f64,
    pub au_km: f64,
    /// Earth/Moon mass ratio.
    pub emrat: f64,
    ncoeff: usize,
    ipt: [[usize; 3]; 12],
    /// The lunar libration angles' coefficients (offset, count, sub-intervals; 0 count: absent).
    lpt: [usize; 3],
    data: Vec<u8>,
}

impl std::fmt::Debug for JplEphemeris {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JplEphemeris")
            .field("title", &self.title)
            .field("start_jd", &self.start_jd)
            .field("end_jd", &self.end_jd)
            .finish()
    }
}

fn f64_at(d: &[u8], o: usize) -> f64 {
    f64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

fn i32_at(d: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

impl JplEphemeris {
    pub fn parse(data: Vec<u8>) -> Result<Self, DeError> {
        if data.len() < 2856 {
            return Err(DeError::Truncated);
        }
        let title = String::from_utf8_lossy(&data[..84]).trim().to_string();
        let start_jd = f64_at(&data, 2652);
        let end_jd = f64_at(&data, 2660);
        let interval_days = f64_at(&data, 2668);
        let au_km = f64_at(&data, 2680);
        let emrat = f64_at(&data, 2688);
        if !(interval_days > 0.0 && end_jd > start_jd && au_km > 1e8) {
            return Err(DeError::BadHeader(format!("span {start_jd}..{end_jd} step {interval_days}")));
        }
        let mut ipt = [[0usize; 3]; 12];
        for (i, slot) in ipt.iter_mut().enumerate() {
            for (j, v) in slot.iter_mut().enumerate() {
                *v = i32_at(&data, 2696 + (i * 3 + j) * 4).max(0) as usize;
            }
        }
        let lpt = [i32_at(&data, 2844), i32_at(&data, 2848), i32_at(&data, 2852)];
        // Record length: the last coefficient referenced by any item.
        let mut ncoeff = 0usize;
        for (i, p) in ipt.iter().enumerate() {
            if p[0] == 0 || p[1] == 0 {
                continue; // item not present in this ephemeris
            }
            let comps = if i == 11 { 2 } else { 3 };
            ncoeff = ncoeff.max(p[0] + p[1] * p[2] * comps - 1);
        }
        if lpt[1] > 0 {
            ncoeff = ncoeff.max((lpt[0] + lpt[1] * lpt[2] * 3 - 1) as usize);
        }
        if ncoeff == 0 {
            return Err(DeError::BadHeader("no coefficients".into()));
        }
        let records = ((end_jd - start_jd) / interval_days).round() as usize;
        if data.len() < (records + 2) * ncoeff * 8 {
            return Err(DeError::Truncated);
        }
        Ok(Self {
            title,
            start_jd,
            end_jd,
            interval_days,
            au_km,
            emrat,
            ncoeff,
            ipt,
            lpt: lpt.map(|v| v.max(0) as usize),
            data,
        })
    }

    pub fn covers(&self, jd: f64) -> bool {
        jd >= self.start_jd && jd < self.end_jd
    }

    /// Position of an item in km (equatorial, barycentric except the Moon).
    pub fn position_km(&self, body: DeBody, jd: f64) -> Option<DVec3> {
        if !self.covers(jd) {
            return None;
        }
        let [off, n, nsub] = self.ipt[body as usize];
        self.chebyshev3(off, n, nsub, jd)
    }

    /// The Moon's mantle orientation at `jd`: DE's lunar libration angles (phi, theta, psi, radians),
    /// the 3-1-3 Euler rotation from the ICRF to the Moon's principal axes.
    pub fn lunar_euler(&self, jd: f64) -> Option<DVec3> {
        if !self.covers(jd) {
            return None;
        }
        let [off, n, nsub] = self.lpt;
        self.chebyshev3(off, n, nsub, jd)
    }

    /// A three-component item (coefficients from `off`, `n` per component, `nsub` sub-intervals).
    fn chebyshev3(&self, off: usize, n: usize, nsub: usize, jd: f64) -> Option<DVec3> {
        if n == 0 || nsub == 0 || off == 0 {
            return None;
        }
        let rec = ((jd - self.start_jd) / self.interval_days).floor() as usize;
        let base = (rec + 2) * self.ncoeff * 8;
        let rec_start = f64_at(&self.data, base);
        let sub_len = self.interval_days / nsub as f64;
        let dt = jd - rec_start;
        let sub = ((dt / sub_len).floor() as usize).min(nsub - 1);
        let t = 2.0 * (dt - sub as f64 * sub_len) / sub_len - 1.0;
        let mut out = [0.0f64; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let first = base + (off - 1 + (sub * 3 + c) * n) * 8;
            // Clenshaw-free direct Chebyshev sum (n is small).
            let (mut t0, mut t1) = (1.0, t);
            let mut sum = f64_at(&self.data, first) + f64_at(&self.data, first + 8) * t;
            for k in 2..n {
                let t2 = 2.0 * t * t1 - t0;
                sum += f64_at(&self.data, first + k * 8) * t2;
                t0 = t1;
                t1 = t2;
            }
            *o = sum;
        }
        Some(DVec3::from_array(out))
    }

    /// Barycentric positions of the Earth and the Moon (km).
    pub fn earth_moon_km(&self, jd: f64) -> Option<(DVec3, DVec3)> {
        let emb = self.position_km(DeBody::Emb, jd)?;
        let moon_geo = self.position_km(DeBody::MoonGeocentric, jd)?;
        let earth = emb - moon_geo / (1.0 + self.emrat);
        Some((earth, earth + moon_geo))
    }

    /// Barycentric position (km) for an SE analytic model name such as
    /// `"Mars-DE"`, `"Earth-EMB-DE"`, `"Moon-EMB-DE"`, `"EMB-DE"` or `"Sun-DE"`.
    pub fn position_for_model(&self, model: &str, jd: f64) -> Option<DVec3> {
        let m = model.to_ascii_lowercase();
        if !m.ends_with("-de") {
            return None;
        }
        let body = m.trim_end_matches("-de");
        match body {
            "earth-emb" | "earth" => self.earth_moon_km(jd).map(|(e, _)| e),
            "moon-emb" | "moon" => self.earth_moon_km(jd).map(|(_, m)| m),
            "emb" => self.position_km(DeBody::Emb, jd),
            "mercury" => self.position_km(DeBody::Mercury, jd),
            "venus" => self.position_km(DeBody::Venus, jd),
            "mars" => self.position_km(DeBody::Mars, jd),
            "jupiter" => self.position_km(DeBody::Jupiter, jd),
            "saturn" => self.position_km(DeBody::Saturn, jd),
            "uranus" => self.position_km(DeBody::Uranus, jd),
            "neptune" => self.position_km(DeBody::Neptune, jd),
            "pluto" => self.position_km(DeBody::Pluto, jd),
            "sun" => self.position_km(DeBody::Sun, jd),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny synthetic ephemeris: one item with constant coefficients.
    fn synthetic() -> JplEphemeris {
        // The header lives in record 0, so records must be at least ~2.9 KB.
        let ncoeff = 400usize;
        let mut d = vec![0u8; ncoeff * 8 * 4];
        let put_f = |d: &mut Vec<u8>, o: usize, v: f64| d[o..o + 8].copy_from_slice(&v.to_le_bytes());
        let put_i = |d: &mut Vec<u8>, o: usize, v: i32| d[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put_f(&mut d, 2652, 2_451_536.0);
        put_f(&mut d, 2660, 2_451_568.0);
        put_f(&mut d, 2668, 32.0);
        put_f(&mut d, 2680, 149_597_870.7);
        put_f(&mut d, 2688, 81.3);
        // Item 0 (Mercury): offset 3, 4 coefficients, 1 subinterval.
        put_i(&mut d, 2696, 3);
        put_i(&mut d, 2700, 4);
        put_i(&mut d, 2704, 1);
        // Make the record length 400 via a dummy pointer on item 11 (nutation).
        put_i(&mut d, 2696 + 33 * 4, 15);
        put_i(&mut d, 2696 + 34 * 4, 193);
        put_i(&mut d, 2696 + 35 * 4, 1);
        // Data record (record index 2): start/end JD then x,y,z coefficients.
        let base = 2 * ncoeff * 8;
        put_f(&mut d, base, 2_451_536.0);
        put_f(&mut d, base + 8, 2_451_568.0);
        // x = 1 + 2 t, y = 5, z = T2(t) = 2t^2 - 1
        let c = base + 16;
        put_f(&mut d, c, 1.0);
        put_f(&mut d, c + 8, 2.0);
        put_f(&mut d, c + 32, 5.0);
        put_f(&mut d, c + 64 + 16, 1.0);
        JplEphemeris::parse(d).unwrap()
    }

    #[test]
    fn chebyshev_evaluation() {
        let e = synthetic();
        assert_eq!(e.ncoeff, 400);
        // Middle of the interval: t = 0.
        let p = e.position_km(DeBody::Mercury, 2_451_552.0).unwrap();
        assert!((p.x - 1.0).abs() < 1e-12 && (p.y - 5.0).abs() < 1e-12 && (p.z + 1.0).abs() < 1e-12, "{p:?}");
        // End of interval: t -> 1.
        let p = e.position_km(DeBody::Mercury, 2_451_567.999_999).unwrap();
        assert!((p.x - 3.0).abs() < 1e-5 && (p.z - 1.0).abs() < 1e-5);
        assert!(e.position_km(DeBody::Mercury, 2_400_000.0).is_none());
        assert!(e.position_for_model("Mercury-DE", 2_451_552.0).is_some());
        assert!(e.position_for_model("Io-L1", 2_451_552.0).is_none());
    }
}
