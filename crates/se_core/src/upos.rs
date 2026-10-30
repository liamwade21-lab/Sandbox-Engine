//! Universe-scale positions.
//!
//! A [`UPos`] is a fixed-point position with 1/1024 m resolution stored in `i128`
//! per axis. That spans ~1.7e35 m, far beyond the observable universe (~4.4e26 m),
//! while differences between nearby positions stay exact. Rendering converts the
//! difference to the camera into `f64` and then to camera-relative `f32`.
//!
//! The frame is SpaceEngine's universe frame: right-handed, Y towards the north
//! celestial pole, Z towards the vernal equinox (RA 0h), X towards RA 6h; the
//! origin is the Solar System barycentre.

use glam::DVec3;
use std::ops::{Add, AddAssign, Sub, SubAssign};

/// Fixed-point units per meter.
pub const UNITS_PER_METER: f64 = 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UPos {
    pub x: i128,
    pub y: i128,
    pub z: i128,
}

impl UPos {
    pub const ZERO: UPos = UPos { x: 0, y: 0, z: 0 };

    pub fn from_meters(v: DVec3) -> Self {
        UPos {
            x: (v.x * UNITS_PER_METER).round() as i128,
            y: (v.y * UNITS_PER_METER).round() as i128,
            z: (v.z * UNITS_PER_METER).round() as i128,
        }
    }

    pub fn from_parsecs(v: DVec3) -> Self {
        Self::from_meters(v * crate::units::METERS_PER_PC)
    }

    /// Absolute position in meters (loses precision far from the origin).
    pub fn to_meters(self) -> DVec3 {
        DVec3::new(
            self.x as f64 / UNITS_PER_METER,
            self.y as f64 / UNITS_PER_METER,
            self.z as f64 / UNITS_PER_METER,
        )
    }

    pub fn to_parsecs(self) -> DVec3 {
        self.to_meters() / crate::units::METERS_PER_PC
    }

    /// `self - origin` in meters, exact up to f64 rounding of the result.
    pub fn delta_m(self, origin: UPos) -> DVec3 {
        (self - origin).to_meters()
    }

    pub fn offset_m(self, d: DVec3) -> UPos {
        self + UPos::from_meters(d)
    }

    pub fn distance_m(self, other: UPos) -> f64 {
        self.delta_m(other).length()
    }
}

impl Add for UPos {
    type Output = UPos;
    fn add(self, o: UPos) -> UPos {
        UPos {
            x: self.x + o.x,
            y: self.y + o.y,
            z: self.z + o.z,
        }
    }
}

impl Sub for UPos {
    type Output = UPos;
    fn sub(self, o: UPos) -> UPos {
        UPos {
            x: self.x - o.x,
            y: self.y - o.y,
            z: self.z - o.z,
        }
    }
}

impl AddAssign for UPos {
    fn add_assign(&mut self, o: UPos) {
        *self = *self + o;
    }
}

impl SubAssign for UPos {
    fn sub_assign(&mut self, o: UPos) {
        *self = *self - o;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::METERS_PER_PC;

    #[test]
    fn far_positions_keep_meter_precision() {
        // A body 1 Gpc away, and a camera 1.5 m from it.
        let far = UPos::from_parsecs(DVec3::new(1e9, -3e8, 2e8));
        let cam = far.offset_m(DVec3::new(1.5, 0.0, 0.0));
        let d = cam.delta_m(far);
        assert!((d.x - 1.5).abs() < 1e-3, "{d:?}");
        assert!(far.to_parsecs().x - 1e9 < 1.0);
        assert!(METERS_PER_PC * 1e9 * UNITS_PER_METER < i128::MAX as f64);
    }
}
