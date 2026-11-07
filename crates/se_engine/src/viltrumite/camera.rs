//! The chase camera (spec F7). `aim` is `FlyCamera::rot`; the camera's place comes from his exact `UPos` with no
//! spring, so he can't jitter against it at any speed.

use bevy::math::{DQuat, DVec2, DVec3};

use crate::camera::{HORIZON_RADII, horizon_turn, look_rotation};
use crate::ship::BodyInfo;

use super::frames::{Ground, height_over};
use super::tune;

/// From his middle to the camera, in the universe's axes: above his head and behind him.
pub fn offset(aim: DQuat, dist: f64) -> DVec3 {
    aim * DVec3::new(0.0, tune::CAM_UP_M, dist)
}

/// How much further back the camera sits at this speed.
pub fn speed_zoom(speed: f64) -> f64 {
    if speed > 30.0 { tune::CAM_SPEED_ZOOM * (speed / 30.0).log10() } else { 0.0 }
}

/// The speed's part of the camera's distance one step on, eased: only the distance, his place stays exact.
pub fn ease_zoom(zoom: f64, speed: f64, dt: f64) -> f64 {
    zoom + (speed_zoom(speed) - zoom) * (1.0 - (-dt / tune::CAM_ZOOM_TAU_S).exp())
}

/// The body's local vertical at a point of its frame (the ellipsoid's normal), universe axes.
pub fn vertical(b: &BodyInfo, local: DVec3) -> DVec3 {
    (b.orient * (b.from_shape(local) / b.shape)).normalize_or(DVec3::Y)
}

/// The height (over the reference ellipsoid) below which the view holds the horizon: SE's rule, as `horizon_up`.
pub fn horizon_height(b: &BodyInfo) -> f64 {
    let atm = b.terrain.and_then(|t| t.atmosphere.as_ref()).map_or(0.0, |a| a.top_radius_m(b.radius) - b.radius);
    (b.radius * HORIZON_RADII).max(atm)
}

/// His own vertical where the view holds the horizon there (`cam.follow` is off, so `horizon_up` can't say).
pub fn horizon_up(b: &BodyInfo, local: DVec3) -> Option<DVec3> {
    (height_over(b, local) < horizon_height(b)).then(|| vertical(b, local))
}

/// The aim after a mouse move of `d` (radians): about his own up below the horizon's height, freely above it.
pub fn turn_aim(aim: DQuat, d: DVec2, up: Option<DVec3>) -> DQuat {
    match up {
        Some(up) => horizon_turn(aim, up, -d.x, -d.y),
        None => (aim * DQuat::from_axis_angle(DVec3::Y, -d.x) * DQuat::from_axis_angle(DVec3::X, -d.y)).normalize(),
    }
}

/// Takes the roll out of the aim, gently, where the view holds the horizon: above it the view may have been rolled.
pub fn level_roll(aim: DQuat, up: DVec3, dt: f64) -> DQuat {
    let f = aim * DVec3::NEG_Z;
    if f.cross(up).length() < 1e-3 {
        return aim;
    }
    let q = look_rotation(f, up).inverse() * aim;
    // (A quaternion and its negative are the same turn, and either can be `aim`'s or `look_rotation`'s: the short way
    // round, or a level view reads as a roll of a whole turn and rolls it.)
    let q = if q.w < 0.0 { -q } else { q };
    let roll = 2.0 * q.z.atan2(q.w);
    if roll.abs() < 1e-5 {
        return aim;
    }
    (aim * DQuat::from_rotation_z(-roll * (1.0 - (-dt / tune::ROLL_LEVEL_TAU_S).exp()))).normalize()
}

/// The ground's floor for the camera: `local` (body frame) lifted so `keep_camera_above_ground`'s 1.5 m push never fires,
/// or `None` if it's clear (or nothing is loaded there).
pub fn floored(ground: Ground, local: DVec3) -> Option<DVec3> {
    ground.level?;
    let min = ground.radial + tune::CAM_FLOOR_M;
    let r = local.length();
    (r < min && r > 0.0).then(|| local * (min / r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_is_behind_and_above_him() {
        let aim = DQuat::from_rotation_y(0.7);
        let o = offset(aim, 6.0);
        // Behind: along +Z of the view, up: along +Y of the view.
        assert!((aim.inverse() * o - DVec3::new(0.0, tune::CAM_UP_M, 6.0)).length() < 1e-9);
    }

    #[test]
    fn the_speed_moves_it_back_a_little() {
        assert_eq!(speed_zoom(20.0), 0.0);
        assert!(speed_zoom(30.0).abs() < 1e-9);
        // At 1000 c: 8 m further.
        let d = speed_zoom(3e11);
        assert!((d - 8.0).abs() < 0.1, "{d}");
    }

    /// A sudden stop: the camera comes in over a moment, not in a frame.
    #[test]
    fn a_sudden_stop_does_not_jump_the_camera() {
        let dt = 1.0 / 60.0;
        let z0 = speed_zoom(30_000.0);
        let z1 = ease_zoom(z0, 0.0, dt);
        assert!(z0 - z1 < 0.2 && z1 < z0, "{z0} to {z1}");
        let mut z = z0;
        for _ in 0..180 {
            z = ease_zoom(z, 0.0, dt);
        }
        assert!(z < 0.01, "{z}");
    }

    /// Below the horizon's height a sideways move never tilts the horizon, and pitch stops short of straight up.
    #[test]
    fn the_horizon_stays_level_with_his_own_up() {
        let up = DVec3::new(0.2, 0.9, 0.1).normalize();
        let mut aim = look_rotation(DVec3::X, up);
        for _ in 0..500 {
            aim = turn_aim(aim, DVec2::new(0.1, -0.03), Some(up));
            let right = aim * DVec3::X;
            assert!(right.dot(up).abs() < 1e-6, "{}", right.dot(up));
        }
        assert!((aim * DVec3::NEG_Z).dot(up) < 1.0 - 1e-7);
    }

    #[test]
    fn free_aim_turns_about_the_view() {
        let aim = DQuat::IDENTITY;
        let got = turn_aim(aim, DVec2::new(0.0, 0.1), None);
        // The mouse moved down: he looks down.
        assert!((got * DVec3::NEG_Z).y < 0.0);
    }

    #[test]
    fn roll_goes_out_slowly() {
        let up = DVec3::Y;
        let mut aim = look_rotation(DVec3::NEG_Z, up) * DQuat::from_rotation_z(0.5);
        for _ in 0..600 {
            aim = level_roll(aim, up, 1.0 / 60.0);
        }
        assert!((aim * DVec3::X).dot(up).abs() < 1e-3);
        // Heading unchanged.
        assert!((aim * DVec3::NEG_Z - DVec3::NEG_Z).length() < 1e-6);
    }

    /// A level view whose quaternion has the other sign is still level: no barrel roll. (`look_rotation` isn't continuous
    /// in its sign, and a view carried round by the body's turn changes sign against it.)
    #[test]
    fn a_level_view_of_either_sign_stays_put() {
        let up = DVec3::new(0.2, 0.9, 0.1).normalize();
        let level = look_rotation(DVec3::X, up);
        for aim in [level, -level] {
            let got = level_roll(aim, up, 1.0 / 60.0);
            assert!((got * DVec3::X - level * DVec3::X).length() < 1e-9, "{got}");
            assert!((got * DVec3::Y - level * DVec3::Y).length() < 1e-9, "{got}");
        }
        // And a little roll of either sign goes out the short way.
        for aim in [level * DQuat::from_rotation_z(0.2), -(level * DQuat::from_rotation_z(0.2))] {
            let got = level_roll(aim, up, 1.0 / 60.0);
            let left = (got * DVec3::X).angle_between(level * DVec3::X);
            assert!(left < 0.2 && left > 0.19, "{left}");
        }
    }

    #[test]
    fn the_floor_lifts_only_under_it() {
        let g = Ground { radial: 1000.0, level: Some(4) };
        assert_eq!(floored(g, DVec3::X * 1005.0), None);
        assert_eq!(floored(g, DVec3::X * 1000.5), Some(DVec3::X * 1002.0));
        assert_eq!(floored(Ground { radial: 1000.0, level: None }, DVec3::X * 900.0), None);
    }
}
