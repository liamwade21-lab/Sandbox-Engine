//! Attitude holds and the autopilot (SE's ShipPrograde .. ShipHorRot, ShipRotateTo, ShipFlyTo,
//! ShipWarpTo and ShipGoTo).
//!
//! SE turns the ship to an orientation with its turn engines (`TurnEngines`, rad/s^2): it plans a
//! bang-bang turn about the axis of the quaternion difference (FUN_140520e00: accelerate for
//! `sqrt(angle / TurnEngines)`, then brake as long), tracks the target with a saturated controller
//! once there (FUN_1405211e0), plans again when the target moves more than 5 degrees away and
//! counts the turn done below 0.1 degree (`_DAT_1406d47d8`, DAT_1406d45b0 in FUN_140537060). While a program turns the ship the
//! player's turn commands are ignored. [`steer`] follows the same profile as a rate command: turn
//! at `sqrt(2 a angle)` (the speed a bang-bang turn has at that angle), never faster than the
//! port's rate limit.
//!
//! The holds point the nose along the velocity relative to the reference body (prograde), against
//! it, away from or towards the body (radial, antiradial) or along the orbit normal `r x v`
//! (normal up / down); Horizon levels the ship to the local horizon keeping its heading.
//!
//! The programs fly to the target in SE's stages ("Preparing jump/fly vector", "Correcting
//! jump/fly vector", "Accelerating", "Turning for braking", "Braking"); the guidance (a braking
//! profile `v = sqrt(2 a d)` on the line to the target, warp limited so a frame never covers more
//! than half the remaining distance) is the port's: SE's fly program was not decoded.

use bevy::math::{DQuat, DVec3};

use crate::camera::look_rotation;

/// A turn is complete below this error (rad, DAT_1406d45b0, 0.1 degree).
pub const TURN_DONE: f64 = 0.001_745_329_251_994_329_6;
/// The autopilot stops this many target radii from the centre (the camera's Go to distance).
pub const STOP_RADII: f64 = 4.0;
/// Fly rather than warp ("Fly or warp to target") when flying there takes less than this (s).
pub const GOTO_FLY_TIME: f64 = 300.0;
/// Error (rad) under which a program counts as pointed at its target and lights the engines.
pub const AIM_TOLERANCE: f64 = 0.0175;
/// Velocity direction error (rad) beyond which a program corrects its vector first.
pub const CORRECT_TOLERANCE: f64 = 0.035;

/// SE's orientation programs (ShipPrograde .. ShipNormalDown on 5-0, ShipHorRot on End).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attitude {
    #[default]
    Free,
    Prograde,
    Retrograde,
    Radial,
    Antiradial,
    NormalUp,
    NormalDown,
    Horizon,
}

impl Attitude {
    /// SE's interface label.
    pub fn label(self) -> &'static str {
        match self {
            Attitude::Free => "Free",
            Attitude::Prograde => "Prograde",
            Attitude::Retrograde => "Retrograde",
            Attitude::Radial => "Radial",
            Attitude::Antiradial => "Antiradial",
            Attitude::NormalUp => "Normal Up",
            Attitude::NormalDown => "Normal Down",
            Attitude::Horizon => "Horizon",
        }
    }

    pub fn parse(s: &str) -> Option<Attitude> {
        Some(match s.to_ascii_lowercase().replace([' ', '_'], "").as_str() {
            "free" | "off" => Attitude::Free,
            "prograde" => Attitude::Prograde,
            "retrograde" => Attitude::Retrograde,
            "radial" => Attitude::Radial,
            "antiradial" => Attitude::Antiradial,
            "normalup" | "normal" => Attitude::NormalUp,
            "normaldown" | "antinormal" => Attitude::NormalDown,
            "horizon" | "horrot" => Attitude::Horizon,
            _ => return None,
        })
    }
}

/// Which body the ship's velocity, altitude and holds refer to (ShipSelectRefBody, P).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefMode {
    /// The body with the nearest surface.
    #[default]
    Nearest,
    /// The body whose gravity is strongest at the ship.
    Influence,
    /// The body the player picked (the selection when P was pressed).
    Selected,
}

impl RefMode {
    pub fn label(self) -> &'static str {
        match self {
            RefMode::Nearest => "Nearest body",
            RefMode::Influence => "Influence body",
            RefMode::Selected => "Reference body",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    RotateTo,
    FlyTo,
    WarpTo,
}

impl Program {
    pub fn label(self) -> &'static str {
        match self {
            Program::RotateTo => "Rotate to target",
            Program::FlyTo => "Fly to target",
            Program::WarpTo => "Warp to target",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Orienting,
    PreparingJump,
    CorrectingJump,
    PreparingFly,
    CorrectingFly,
    Accelerating,
    TurningForBraking,
    Braking,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Orienting => "Orientation",
            Stage::PreparingJump => "Preparing jump vector",
            Stage::CorrectingJump => "Correcting jump vector",
            Stage::PreparingFly => "Preparing fly vector",
            Stage::CorrectingFly => "Correcting fly vector",
            Stage::Accelerating => "Accelerating",
            Stage::TurningForBraking => "Turning for braking",
            Stage::Braking => "Braking",
        }
    }
}

/// A running program.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Autopilot {
    pub program: Program,
    pub stage: Stage,
}

impl Autopilot {
    pub fn new(program: Program) -> Autopilot {
        let stage = match program {
            Program::RotateTo => Stage::Orienting,
            Program::FlyTo => Stage::PreparingFly,
            Program::WarpTo => Stage::PreparingJump,
        };
        Autopilot { program, stage }
    }
}

/// "Fly or warp to target": warp when the ship has a drive and flying would take long.
pub fn choose_goto(can_warp: bool, distance_m: f64, main_accel: f64) -> Program {
    let fly_time = 2.0 * (distance_m.max(0.0) / main_accel.max(1e-6)).sqrt();
    if can_warp && fly_time > GOTO_FLY_TIME { Program::WarpTo } else { Program::FlyTo }
}

/// The nose (-Z) direction of `q`.
pub fn nose(q: DQuat) -> DVec3 {
    q * DVec3::NEG_Z
}

/// `cur` turned the shortest way so that its nose points along `dir`.
pub fn point_nose(cur: DQuat, dir: DVec3) -> Option<DQuat> {
    let d = dir.try_normalize()?;
    Some((DQuat::from_rotation_arc(nose(cur), d) * cur).normalize())
}

/// Orientation an attitude hold wants: `rel_pos` is the ship relative to the reference body's
/// centre, `rel_vel` its velocity relative to the body's frame (universe frame, metres).
pub fn attitude_rotation(mode: Attitude, cur: DQuat, rel_pos: DVec3, rel_vel: DVec3) -> Option<DQuat> {
    let r = rel_pos.try_normalize()?;
    let dir = match mode {
        Attitude::Free => return None,
        Attitude::Prograde => rel_vel,
        Attitude::Retrograde => -rel_vel,
        Attitude::Radial => r,
        Attitude::Antiradial => -r,
        Attitude::NormalUp => rel_pos.cross(rel_vel),
        Attitude::NormalDown => -rel_pos.cross(rel_vel),
        Attitude::Horizon => {
            // Keep the heading, level the wings: nose along the horizon, up the local vertical.
            let f = nose(cur);
            let mut h = f - r * f.dot(r);
            if h.length() < 0.05 {
                let u = cur * DVec3::Y * -f.dot(r).signum();
                h = u - r * u.dot(r);
            }
            return Some(look_rotation(h.try_normalize()?, r));
        }
    };
    point_nose(cur, dir)
}

/// One step of the turn towards `target`: the new angular velocity (ship frame, rad/s) from `w`,
/// changing each axis by at most `accel * dt`, and the remaining error (rad).
pub fn steer(cur: DQuat, target: DQuat, w: DVec3, accel: f64, max_rate: f64, dt: f64) -> (DVec3, f64) {
    let mut err = cur.inverse() * target;
    if err.w < 0.0 {
        err = -err;
    }
    let (axis, angle) = err.to_axis_angle();
    let angle = if angle.is_finite() { angle } else { 0.0 };
    // A bang-bang turn passes `angle` from the stop at sqrt(2 a angle); near the target the
    // command falls off linearly so discrete steps do not chatter.
    let tau = (4.0 * dt).max(0.25);
    let rate = (2.0 * accel * angle).sqrt().min(angle / tau).min(max_rate);
    let want = if angle > 1e-9 { axis * rate } else { DVec3::ZERO };
    let step = accel * dt;
    let dw = (want - w).clamp(DVec3::splat(-step), DVec3::splat(step));
    ((w + dw).clamp(DVec3::splat(-max_rate), DVec3::splat(max_rate)), angle)
}

/// Time (s) to turn a ship round (180 degrees) with angular acceleration `accel` and rate limit
/// `max_rate`: a bang-bang turn, with a coast at the limit when it is reached.
pub fn flip_time(accel: f64, max_rate: f64) -> f64 {
    let a = accel.max(1e-9);
    let pi = std::f64::consts::PI;
    if max_rate * max_rate / a >= pi { 2.0 * (pi / a).sqrt() } else { pi / max_rate + max_rate / a }
}

/// What a program needs to know each frame.
#[derive(Clone, Copy, Debug)]
pub struct ProgramInput {
    pub orient: DQuat,
    /// Target centre relative to the ship (m) and the ship's velocity relative to it (m/s).
    pub to_target: DVec3,
    pub rel_vel: DVec3,
    /// Gravitational acceleration at the ship (m/s^2).
    pub gravity: DVec3,
    /// Distance from the target's centre at which the program stops (m).
    pub stop_distance: f64,
    /// Engine accelerations (m/s^2): main, retro (0 without retro engines), RCS sideways, RCS up.
    pub main: f64,
    pub retro: f64,
    pub corr: f64,
    pub up: f64,
    pub can_warp: bool,
    /// Time (s) the ship needs to turn round (for braking with the main engines).
    pub flip_time: f64,
    /// Current warp boost (log10) and the ship time step (s).
    pub warp: f64,
    pub dt: f64,
}

/// What a program asks of the ship this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProgramOutput {
    pub attitude: Option<DQuat>,
    pub throttle: f64,
    /// RCS translation command, ship axes (x right, y up), -1..1.
    pub rcs: DVec3,
    pub warp_on: bool,
    /// Upper limit of the warp boost (log10) this frame.
    pub warp_limit: f64,
    /// The program reached its target (the ship is then matched to the target's velocity).
    pub done: bool,
}

/// RCS command (ship axes) that cancels the sideways velocity `v_lat` in about half a second and
/// holds against the sideways part `g_lat` of gravity (both universe frame).
fn rcs_cancel(i: &ProgramInput, v_lat: DVec3, g_lat: DVec3) -> DVec3 {
    let want = i.orient.inverse() * (-v_lat * 2.0 - g_lat);
    let up = if want.y > 0.0 { i.up } else { i.corr };
    DVec3::new((want.x / i.corr.max(1e-6)).clamp(-1.0, 1.0), (want.y / up.max(1e-6)).clamp(-1.0, 1.0), 0.0)
}

/// One frame of `ap` (stages advance in place). Fly to target flies the ship along the line to
/// the target with the closing speed under the braking profile `sqrt(2 a d)`; Warp to target does
/// the same with the drive engaged, its physical speed capped, and the boost limited so a frame
/// never covers more than half the remaining distance (the executable's rule for altitude).
pub fn run_program(ap: &mut Autopilot, i: &ProgramInput) -> ProgramOutput {
    let mut out = ProgramOutput::default();
    let d = i.to_target.length();
    let Some(dir) = i.to_target.try_normalize() else {
        out.done = true;
        return out;
    };
    let point = |d: DVec3| point_nose(i.orient, d);
    // How far the nose is from a direction (rad).
    let off_by = |d: DVec3| nose(i.orient).angle_between(d);
    if ap.program == Program::RotateTo {
        out.attitude = point(dir);
        out.done = off_by(dir) < TURN_DONE;
        return out;
    }
    let warp = ap.program == Program::WarpTo && i.can_warp;
    let remaining = (d - i.stop_distance).max(0.0);
    // While the program runs the warp multiplies the velocity relative to the target.
    let closing = i.rel_vel.dot(dir);
    let v_lat = i.rel_vel - dir * closing;
    let g_lat = i.gravity - dir * i.gravity.dot(dir);
    // Braking acceleration: retro engines along the nose, or the main engines turned round.
    let retro = i.retro > 0.0;
    let brake_a = if retro { i.retro } else { i.main };
    let a = 0.8 * brake_a;
    let profile = (2.0 * a * remaining).sqrt();
    // Before a turn round, also leave the distance covered while turning: v t + v^2 / 2a = d.
    let profile = if retro || matches!(ap.stage, Stage::Braking) {
        profile
    } else {
        let at = a * i.flip_time;
        (at * at + 2.0 * a * remaining).sqrt() - at
    };
    let cruise = (i.main * 2.0).clamp(10.0, 1000.0);
    let v_des = if warp { profile.min(cruise) } else { profile };
    let near = remaining <= i.stop_distance * 0.02 + 1.0 || (remaining < i.stop_distance * 0.25 && i.rel_vel.dot(dir).abs() < 0.5);
    if near && i.rel_vel.length() < 2.0 + 0.05 * v_des && i.warp < 0.05 {
        out.done = true;
        return out;
    }
    let (preparing, correcting) =
        if warp { (Stage::PreparingJump, Stage::CorrectingJump) } else { (Stage::PreparingFly, Stage::CorrectingFly) };
    let aim = dir * v_des.max(1.0) - v_lat;
    out.rcs = rcs_cancel(i, v_lat, g_lat);
    match ap.stage {
        Stage::PreparingJump | Stage::PreparingFly => {
            ap.stage = preparing;
            out.attitude = point(aim);
            if off_by(aim) < AIM_TOLERANCE {
                if v_lat.length() < (0.05 * v_des).max(2.0) {
                    ap.stage = Stage::Accelerating;
                } else {
                    // A large sideways drift: burn it off along the aim with the main engines.
                    out.throttle = (v_lat.length() / i.main.max(1e-6)).min(1.0);
                }
            }
        }
        Stage::Accelerating | Stage::CorrectingJump | Stage::CorrectingFly | Stage::Orienting => {
            out.attitude = point(aim);
            let off = off_by(aim) > CORRECT_TOLERANCE;
            ap.stage = if off { correcting } else { Stage::Accelerating };
            out.throttle = if !off && closing < v_des { 1.0 } else { 0.0 };
            if closing > v_des * 1.05 + 1.0 {
                ap.stage = if retro { Stage::Braking } else { Stage::TurningForBraking };
            }
        }
        Stage::TurningForBraking => {
            out.attitude = point(-dir);
            if off_by(-dir) < AIM_TOLERANCE {
                ap.stage = Stage::Braking;
            }
        }
        Stage::Braking => {
            // Keep the closing speed on the profile.
            let along = if retro { dir } else { -dir };
            out.attitude = point(along);
            let excess = closing - v_des;
            // Thrust only once pointed (the main engines turned round may still be turning).
            let t = if off_by(along) < CORRECT_TOLERANCE { (excess / (brake_a * 0.5)).clamp(0.0, 1.0) } else { 0.0 };
            out.throttle = if retro { -t } else { t };
            if excess < -(0.3 * v_des).max(10.0) && remaining > i.stop_distance * 0.25 {
                // Well under the profile with a long way still to go: speed up again.
                ap.stage = if retro { Stage::Accelerating } else { preparing };
            }
        }
    }
    if warp {
        // Jump only while the velocity points at the target: the drive multiplies all of it.
        let on_line = v_lat.length() <= closing * CORRECT_TOLERANCE;
        out.warp_on = ap.stage != preparing && ap.stage != correcting && closing > 0.5 && on_line;
        let speed = i.rel_vel.length();
        out.warp_limit = if speed > 1e-9 { (remaining / (2.0 * speed * i.dt)).max(1.0).log10() } else { 0.0 };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Angle (rad) between two orientations.
    fn angle_between(a: DQuat, b: DQuat) -> f64 {
        2.0 * a.dot(b).abs().min(1.0).acos()
    }

    #[test]
    fn holds_point_along_the_orbit_vectors() {
        let cur = DQuat::IDENTITY;
        let r = DVec3::new(7e6, 0.0, 0.0);
        let v = DVec3::new(0.0, 0.0, -7.5e3);
        let n = |m| nose(attitude_rotation(m, cur, r, v).unwrap());
        assert!((n(Attitude::Prograde) - DVec3::NEG_Z).length() < 1e-9);
        assert!((n(Attitude::Retrograde) - DVec3::Z).length() < 1e-9);
        assert!((n(Attitude::Radial) - DVec3::X).length() < 1e-9);
        assert!((n(Attitude::Antiradial) - DVec3::NEG_X).length() < 1e-9);
        // r x v = (7e6, 0, 0) x (0, 0, -7.5e3) = +Y.
        assert!((n(Attitude::NormalUp) - DVec3::Y).length() < 1e-9);
        assert!((n(Attitude::NormalDown) - DVec3::NEG_Y).length() < 1e-9);
        assert!(attitude_rotation(Attitude::Free, cur, r, v).is_none());
    }

    #[test]
    fn horizon_levels_the_wings_and_keeps_the_heading() {
        // Banked 30 degrees and pitched up 20, over a body below (-Y).
        let cur = DQuat::from_rotation_y(0.4) * DQuat::from_rotation_x(0.35) * DQuat::from_rotation_z(0.5);
        let q = attitude_rotation(Attitude::Horizon, cur, DVec3::Y * 1e6, DVec3::ZERO).unwrap();
        assert!(nose(q).y.abs() < 1e-9 && (q * DVec3::Y - DVec3::Y).length() < 1e-9);
        let h = |q: DQuat| {
            let f = nose(q);
            f.x.atan2(-f.z)
        };
        assert!((h(q) - h(cur)).abs() < 1e-9);
    }

    #[test]
    fn steering_converges_without_overshoot() {
        let target = DQuat::from_rotation_y(1.0);
        let mut q = DQuat::IDENTITY;
        let mut w = DVec3::ZERO;
        let (a, dt) = (0.05, 1.0 / 60.0);
        let mut max_err_after = 0.0f64;
        for k in 0..60 * 60 {
            let (nw, err) = steer(q, target, w, a, 0.1, dt);
            w = nw;
            q = (q * DQuat::from_scaled_axis(w * dt)).normalize();
            if k > 60 * 40 {
                max_err_after = max_err_after.max(err);
            }
        }
        assert!(angle_between(q, target) < TURN_DONE, "{}", angle_between(q, target));
        assert!(max_err_after < 0.01);
    }

    #[test]
    fn goto_warps_only_far_away() {
        assert_eq!(choose_goto(true, 1e12, 50.0), Program::WarpTo);
        assert_eq!(choose_goto(false, 1e12, 50.0), Program::FlyTo);
        assert_eq!(choose_goto(true, 1e5, 50.0), Program::FlyTo);
    }

    /// A point-mass simulation of "Fly to target" in free space.
    #[test]
    fn fly_to_stops_at_the_target() {
        let mut ap = Autopilot::new(Program::FlyTo);
        let target = DVec3::new(0.0, 0.0, -200_000.0);
        let (mut pos, mut vel) = (DVec3::ZERO, DVec3::new(30.0, 0.0, 0.0));
        let mut q = DQuat::from_rotation_y(1.2);
        let mut w = DVec3::ZERO;
        let dt = 0.05;
        let mut done = false;
        for _ in 0..200_000 {
            let i = ProgramInput {
                orient: q,
                to_target: target - pos,
                rel_vel: vel,
                gravity: DVec3::ZERO,
                stop_distance: 4000.0,
                main: 20.0,
                retro: 10.0,
                corr: 2.0,
                up: 2.0,
                can_warp: false,
                flip_time: 20.0,
                warp: 0.0,
                dt,
            };
            let o = run_program(&mut ap, &i);
            if o.done {
                done = true;
                break;
            }
            if let Some(t) = o.attitude {
                (w, _) = steer(q, t, w, 0.2, 0.3, dt);
            }
            q = (q * DQuat::from_scaled_axis(w * dt)).normalize();
            let a_main = if o.throttle >= 0.0 { o.throttle * 20.0 } else { o.throttle * 10.0 };
            let a = q * DVec3::new(o.rcs.x * 2.0, o.rcs.y * 2.0, -a_main);
            vel += a * dt;
            pos += vel * dt;
        }
        assert!(done, "never arrived: {pos:?} {vel:?} {ap:?}");
        let d = (target - pos).length();
        assert!(d > 3000.0 && d < 4500.0 && vel.length() < 5.0, "{d} {vel:?}");
    }
    /// "Warp to target" over the Earth-Moon distance with a ship without retro engines.
    #[test]
    fn warp_to_arrives_without_overshooting() {
        let mut ap = Autopilot::new(Program::WarpTo);
        let target = DVec3::new(3.8e8, 0.0, 0.0);
        let stop = 7.0e6;
        let (mut pos, mut vel) = (DVec3::ZERO, DVec3::ZERO);
        let (mut q, mut w, mut warp) = (DQuat::IDENTITY, DVec3::ZERO, 0.0f64);
        let dt = 1.0 / 60.0;
        let mut closest = f64::MAX;
        let mut done = false;
        for _ in 0..60 * 900 {
            let i = ProgramInput {
                orient: q,
                to_target: target - pos,
                rel_vel: vel,
                gravity: DVec3::ZERO,
                stop_distance: stop,
                main: 300.0,
                retro: 0.0,
                corr: 20.0,
                up: 20.0,
                can_warp: true,
                flip_time: flip_time(0.05, 0.1),
                warp,
                dt,
            };
            let o = run_program(&mut ap, &i);
            if o.done {
                done = true;
                break;
            }
            if let Some(t) = o.attitude {
                (w, _) = steer(q, t, w, 0.05, 0.1, dt);
            }
            q = (q * DQuat::from_scaled_axis(w * dt)).normalize();
            let want = if o.warp_on { o.warp_limit.min(15.0) } else { 0.0 };
            warp += (want - warp).clamp(-4.0 * dt, 4.0 * dt);
            let boost = 10f64.powf(warp).min(10f64.powf(o.warp_limit.max(0.0)));
            let a = q * DVec3::new(o.rcs.x * 20.0, o.rcs.y * 20.0, -o.throttle.max(0.0) * 300.0);
            vel += a * dt;
            pos += vel * boost * dt;
            closest = closest.min((target - pos).length());
        }
        assert!(done, "never arrived: {:?} {vel:?} {ap:?} warp {warp}", (target - pos).length());
        assert!(closest > stop * 0.9, "overshot to {closest}");
        assert!((target - pos).length() < stop * 1.3, "{}", (target - pos).length());
    }
}
