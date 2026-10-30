//! Time keeping: Julian dates (TT ~ UTC, the ~69 s difference is ignored as the
//! original does in casual use) and calendar conversions.

pub const J2000: f64 = 2_451_545.0;

/// Calendar date and time (proleptic Gregorian after 1582-10-15, Julian before).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: f64,
}

impl DateTime {
    pub fn new(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Julian date (Meeus, Astronomical Algorithms ch. 7).
    pub fn to_jd(&self) -> f64 {
        let (mut y, mut m) = (self.year, self.month as i64);
        if m <= 2 {
            y -= 1;
            m += 12;
        }
        let d = self.day as f64
            + (self.hour as f64 + (self.minute as f64 + self.second / 60.0) / 60.0) / 24.0;
        let gregorian = (self.year, self.month, self.day) >= (1582, 10, 15);
        let b = if gregorian {
            let a = y.div_euclid(100);
            2 - a + a.div_euclid(4)
        } else {
            0
        };
        (365.25 * (y as f64 + 4716.0)).floor() + (30.6001 * (m as f64 + 1.0)).floor() + d + b as f64 - 1524.5
    }

    pub fn from_jd(jd: f64) -> Self {
        let jd = jd + 0.5;
        let z = jd.floor();
        let f = jd - z;
        let a = if z < 2_299_161.0 {
            z
        } else {
            let alpha = ((z - 1_867_216.25) / 36_524.25).floor();
            z + 1.0 + alpha - (alpha / 4.0).floor()
        };
        let b = a + 1524.0;
        let c = ((b - 122.1) / 365.25).floor();
        let d = (365.25 * c).floor();
        let e = ((b - d) / 30.6001).floor();
        let day_f = b - d - (30.6001 * e).floor() + f;
        let month = if e < 14.0 { e - 1.0 } else { e - 13.0 } as u32;
        let year = if month > 2 { c - 4716.0 } else { c - 4715.0 } as i64;
        let day = day_f.floor() as u32;
        let mut secs = (day_f - day_f.floor()) * 86_400.0;
        // Avoid 59.9999999 artifacts.
        secs = (secs * 1000.0).round() / 1000.0;
        let hour = (secs / 3600.0).floor() as u32;
        let minute = ((secs - hour as f64 * 3600.0) / 60.0).floor() as u32;
        let second = secs - hour as f64 * 3600.0 - minute as f64 * 60.0;
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Current UTC time from the system clock.
    pub fn now_utc() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        Self::from_jd(unix_to_jd(secs))
    }
}

impl std::fmt::Display for DateTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}.{:02}.{:02} {:02}:{:02}:{:05.2}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

pub fn unix_to_jd(unix_seconds: f64) -> f64 {
    unix_seconds / 86_400.0 + 2_440_587.5
}

/// Julian centuries since J2000.
pub fn centuries_since_j2000(jd: f64) -> f64 {
    (jd - J2000) / 36_525.0
}

/// Greenwich mean sidereal time in degrees (IAU 1982), used for sky orientation.
pub fn gmst_degrees(jd: f64) -> f64 {
    let t = centuries_since_j2000(jd);
    let gmst = 280.460_618_37 + 360.985_647_366_29 * (jd - J2000) + 0.000_387_933 * t * t
        - t * t * t / 38_710_000.0;
    gmst.rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j2000_roundtrip() {
        let dt = DateTime::new(2000, 1, 1, 12, 0, 0.0);
        assert!((dt.to_jd() - J2000).abs() < 1e-9);
        let back = DateTime::from_jd(J2000);
        assert_eq!((back.year, back.month, back.day, back.hour), (2000, 1, 1, 12));
    }

    #[test]
    fn meeus_examples() {
        // Meeus example 7.a: 1957 Oct 4.81 = JD 2436116.31
        let jd = DateTime::new(1957, 10, 4, 19, 26, 24.0).to_jd();
        assert!((jd - 2_436_116.31).abs() < 1e-6);
        // Julian calendar: 333 Jan 27.5 = JD 1842713.0
        let jd = DateTime::new(333, 1, 27, 12, 0, 0.0).to_jd();
        assert!((jd - 1_842_713.0).abs() < 1e-6);
        let dt = DateTime::from_jd(1_842_713.0);
        assert_eq!((dt.year, dt.month, dt.day), (333, 1, 27));
    }
}
