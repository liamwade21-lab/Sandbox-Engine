//! Celestial reference frames.
//!
//! Universe frame (see [`crate::upos`]): X = equatorial RA 6h, Y = north celestial
//! pole, Z = vernal equinox. Equivalent to J2000 equatorial axes permuted as
//! `(x_eq, y_eq, z_eq) -> (y_eq, z_eq, x_eq)`. This was derived from the Milky Way
//! quaternion in `LocalGroup.sc`, whose disk normal must point at the north
//! galactic pole (RA 192.86, Dec +27.13).

use glam::{DQuat, DVec3};

/// Obliquity of the ecliptic at J2000, degrees.
pub const OBLIQUITY_J2000_DEG: f64 = 23.439_291_1;

/// Unit vector in the universe frame for equatorial RA (hours) / Dec (degrees).
pub fn radec_to_universe(ra_hours: f64, dec_deg: f64) -> DVec3 {
    let a = (ra_hours * 15.0).to_radians();
    let d = dec_deg.to_radians();
    eq_to_universe(DVec3::new(d.cos() * a.cos(), d.cos() * a.sin(), d.sin()))
}

/// Unit vector in the universe frame for RA/Dec both in degrees.
pub fn radec_deg_to_universe(ra_deg: f64, dec_deg: f64) -> DVec3 {
    radec_to_universe(ra_deg / 15.0, dec_deg)
}

/// Equatorial J2000 cartesian (x = RA 0h, z = NCP) to universe frame.
pub fn eq_to_universe(v: DVec3) -> DVec3 {
    DVec3::new(v.y, v.z, v.x)
}

pub fn universe_to_eq(v: DVec3) -> DVec3 {
    DVec3::new(v.z, v.x, v.y)
}

/// Inverse of [`radec_to_universe`]: (RA hours in [0,24), Dec degrees).
pub fn universe_to_radec(v: DVec3) -> (f64, f64) {
    let e = universe_to_eq(v).normalize_or_zero();
    let ra = e.y.atan2(e.x).to_degrees().rem_euclid(360.0) / 15.0;
    let dec = e.z.clamp(-1.0, 1.0).asin().to_degrees();
    (ra, dec)
}

/// Ecliptic J2000 cartesian (x = vernal equinox, z = ecliptic north) to universe frame.
pub fn ecliptic_to_universe(v: DVec3) -> DVec3 {
    let e = OBLIQUITY_J2000_DEG.to_radians();
    let (s, c) = e.sin_cos();
    let eq = DVec3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c);
    eq_to_universe(eq)
}

/// Rotation taking ecliptic-frame vectors to the universe frame.
pub fn ecliptic_to_universe_quat() -> DQuat {
    let x = ecliptic_to_universe(DVec3::X);
    let y = ecliptic_to_universe(DVec3::Y);
    let z = ecliptic_to_universe(DVec3::Z);
    DQuat::from_mat3(&glam::DMat3::from_cols(x, y, z))
}

/// Rotation taking equatorial-frame vectors to the universe frame.
pub fn eq_to_universe_quat() -> DQuat {
    DQuat::from_mat3(&glam::DMat3::from_cols(
        eq_to_universe(DVec3::X),
        eq_to_universe(DVec3::Y),
        eq_to_universe(DVec3::Z),
    ))
}

/// Galactic north pole and centre directions (J2000) in universe frame.
pub fn galactic_north_pole() -> DVec3 {
    radec_deg_to_universe(192.859_48, 27.128_25)
}

pub fn galactic_center_dir() -> DVec3 {
    radec_deg_to_universe(266.404_99, -28.936_17)
}

/// Rotation taking galactic cartesian (x = GC, z = NGP) to the universe frame.
pub fn galactic_to_universe_quat() -> DQuat {
    let z = galactic_north_pole();
    let x = (galactic_center_dir() - z * galactic_center_dir().dot(z)).normalize();
    let y = z.cross(x);
    DQuat::from_mat3(&glam::DMat3::from_cols(x, y, z))
}

/// Galactic longitude/latitude (degrees) of a universe-frame direction.
pub fn universe_to_galactic(v: DVec3) -> (f64, f64) {
    let g = galactic_to_universe_quat().inverse() * v.normalize_or_zero();
    let l = g.y.atan2(g.x).to_degrees().rem_euclid(360.0);
    let b = g.z.clamp(-1.0, 1.0).asin().to_degrees();
    (l, b)
}

/// SE galaxy orientation: catalogs store `Quat (w x y z)` that maps universe to
/// model space; the galaxy disk normal is model -Z. Returns model->universe.
pub fn se_galaxy_quat_to_model_rotation(wxyz: [f64; 4]) -> DQuat {
    DQuat::from_xyzw(wxyz[1], wxyz[2], wxyz[3], wxyz[0]).normalize().inverse()
}

/// SE `Axis (angle_deg x y z)` alternative orientation form.
pub fn se_axis_angle_to_model_rotation(a: [f64; 4]) -> DQuat {
    let axis = DVec3::new(a[1], a[2], a[3]).normalize_or(DVec3::Y);
    DQuat::from_axis_angle(axis, a[0].to_radians()).inverse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_axes() {
        assert!((radec_to_universe(0.0, 0.0) - DVec3::Z).length() < 1e-12);
        assert!((radec_to_universe(6.0, 0.0) - DVec3::X).length() < 1e-12);
        assert!((radec_to_universe(0.0, 90.0) - DVec3::Y).length() < 1e-12);
        let (ra, dec) = universe_to_radec(radec_to_universe(17.76, -28.93));
        assert!((ra - 17.76).abs() < 1e-9 && (dec + 28.93).abs() < 1e-9);
    }

    #[test]
    fn milky_way_quat_points_disk_normal_to_ngp() {
        let q = se_galaxy_quat_to_model_rotation([-0.9252796, 0.2015153, 0.1678004, -0.27403]);
        let normal = q * DVec3::NEG_Z;
        assert!(normal.dot(galactic_north_pole()) > 0.9999, "{normal:?}");
    }

    #[test]
    fn galactic_coords() {
        // The J2000 definitions of the centre and pole are orthogonal to ~1e-5 deg.
        let (l, b) = universe_to_galactic(galactic_center_dir());
        assert!(l.min(360.0 - l) < 1e-3 && b.abs() < 1e-3, "l={l} b={b}");
        let (_, b) = universe_to_galactic(galactic_north_pole());
        assert!((b - 90.0).abs() < 1e-6);
    }
}
