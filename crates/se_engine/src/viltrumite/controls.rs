//! The controls as in the user's own Viltrumite mod (the user, 2026-11-07: "i want the controls to be how it is in
//! C:\Users\liamw\Desktop\Viltrumite"; its `Flight.cs`, `Stacks.cs`, `Combat.cs`): Space taps (take off, drop) and charges (a
//! launch, a boost stack past the barrier), W twice for fast flight and S twice back to a hover, W and S its speed, Ctrl a dive,
//! Shift and W twice a super run, the left button a combo and a charged punch. Pure logic here; `hero.rs` drives it.

use bevy::math::DVec3;

use super::air::Flow;
use super::hero::{ramp_speed, ramp_time, slerp_dir};
use super::tune;

/// Space held: a tap if let go before `TAP_S`, else a charge filling over `full_s`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpaceHold {
    /// When it went down (his clock), while it's down.
    pub down_at: Option<f64>,
    pub charging: bool,
    /// 0..1.
    pub charge: f64,
}

/// What Space did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpaceEvent {
    Idle,
    /// Held past a tap: the charge begins.
    Started,
    Charging,
    /// Let go before `TAP_S`.
    Tap,
    /// Let go while charging, at this charge.
    Release(f64),
}

impl SpaceHold {
    pub fn step(&mut self, pressed: bool, held: bool, now: f64, dt: f64, full_s: f64) -> SpaceEvent {
        if pressed {
            *self = SpaceHold { down_at: Some(now), charging: false, charge: 0.0 };
        }
        let Some(at) = self.down_at else { return SpaceEvent::Idle };
        if held {
            if now - at < tune::TAP_S {
                return SpaceEvent::Idle;
            }
            let started = !self.charging;
            self.charging = true;
            self.charge = (self.charge + dt / full_s.max(1e-3)).min(1.0);
            return if started { SpaceEvent::Started } else { SpaceEvent::Charging };
        }
        let was = *self;
        *self = SpaceHold::default();
        if was.charging { SpaceEvent::Release(was.charge) } else { SpaceEvent::Tap }
    }

    /// Whatever it was doing, nothing.
    pub fn cancel(&mut self) {
        *self = SpaceHold::default();
    }
}

/// A press that comes within `DOUBLE_TAP_S` of the last: the second of a double tap (and the next press starts afresh).
pub fn double_tap(last: &mut f64, pressed: bool, now: f64) -> bool {
    if !pressed {
        return false;
    }
    let twice = now - *last <= tune::DOUBLE_TAP_S;
    *last = if twice { f64::NEG_INFINITY } else { now };
    twice
}

/// The speed of sound here, or the reference one out of the air (the launch's top and the stacks go by it everywhere).
pub fn sound_speed(flow: &Flow) -> f64 {
    if flow.in_air() { flow.air.sound_speed } else { tune::SOUND_REF_MS }
}

/// Past the barrier for the controls: Mach 1 in air, or the reference speed of sound out of it.
pub fn supersonic(flow: &Flow, speed: f64) -> bool {
    if flow.in_air() { flow.mach >= 1.0 } else { speed >= tune::SOUND_REF_MS }
}

/// A charged launch from `from` m/s: towards `LAUNCH_TOP_MACH` times the speed of sound by the charge (a full one breaks the barrier).
pub fn launch_speed(from: f64, charge: f64, sound: f64) -> f64 {
    let from = from.max(tune::FAST_MIN_MS);
    let top = (tune::LAUNCH_TOP_MACH * sound).max(from);
    from + charge.clamp(0.0, 1.0) * (top - from)
}

/// Fast flight's share of its range at `speed` (0 at `FAST_MIN_MS`, 1 at the launch's top): the camera's widening, the loops.
pub fn speed_share(speed: f64, sound: f64) -> f64 {
    let top = tune::LAUNCH_TOP_MACH * sound;
    ((speed.max(1.0) / tune::FAST_MIN_MS).ln() / (top / tune::FAST_MIN_MS).ln()).clamp(0.0, 1.0)
}

/// Fast flight one step on (frame-relative velocity, universe axes): the heading turns towards the look at `steer` (1/s); W climbs the
/// speed ramp (300 m/s in a second, 30 km/s in six, then doubling: `RAMP`), S halves it every `S_BRAKE_HALF_S`, neither holds it;
/// never past `cap`. `w_edge`: W pressed this frame (the ramp goes on from the speed he has).
pub fn fast_step(v: DVec3, ramp_t: &mut f64, aim: DVec3, w: bool, w_edge: bool, s: bool, steer: f64, cap: f64, dt: f64) -> DVec3 {
    let speed = v.length();
    let dir = slerp_dir(v.try_normalize().unwrap_or(aim), aim, 1.0 - (-steer * dt).exp());
    let mut new_speed = speed;
    if w && !s {
        if w_edge {
            *ramp_t = ramp_time(speed);
        }
        *ramp_t = (*ramp_t + dt).min(ramp_time(cap));
        let target = ramp_speed(*ramp_t + tune::RAMP_TAU_S).min(cap);
        if target > speed {
            new_speed = speed + (target - speed) * (1.0 - (-dt / tune::RAMP_TAU_S).exp());
        }
    }
    if s {
        new_speed *= 0.5f64.powf(dt / tune::S_BRAKE_HALF_S);
    }
    if new_speed > cap {
        new_speed = cap + (new_speed - cap) * (-dt / tune::CAP_BRAKE_TAU_S).exp();
    }
    dir * new_speed.min(tune::GUARD_MS)
}

/// The dive one step on: turning straight down (`down`, unit) at `DIVE_TURN`, gaining `DIVE_ACCEL`, never past `cap`.
pub fn dive_step(v: DVec3, down: DVec3, cap: f64, dt: f64) -> DVec3 {
    let speed = v.length();
    let dir = slerp_dir(v.try_normalize().unwrap_or(down), down, 1.0 - (-tune::DIVE_TURN * dt).exp());
    dir * if speed < cap { (speed + tune::DIVE_ACCEL * dt).min(cap) } else { speed }
}

/// A ground launch's heading: the look, at least `LAUNCH_MIN_UP_DEG` over the level (`up` the local vertical, unit).
pub fn launch_dir(aim: DVec3, up: DVec3) -> DVec3 {
    let flat = (aim - up * aim.dot(up)).try_normalize();
    let min = tune::LAUNCH_MIN_UP_DEG.to_radians();
    let pitch = aim.dot(up).clamp(-1.0, 1.0).asin();
    match flat {
        Some(f) if pitch < min => (f * min.cos() + up * min.sin()).normalize(),
        Some(_) => aim,
        None => up,
    }
}

/// The punch's charge (0..1): held from `PUNCH_CHARGE_S` (0) to `PUNCH_FULL_S` (1).
pub fn punch_charge(held: f64) -> f64 {
    ((held - tune::PUNCH_CHARGE_S) / (tune::PUNCH_FULL_S - tune::PUNCH_CHARGE_S)).clamp(0.0, 1.0)
}

/// The line of keys at the bottom of the screen, as the user's mod has it ("Key:Action|...").
#[allow(clippy::too_many_arguments)]
pub fn prompt(state: super::hero::State, fast: bool, dive: bool, super_on: bool, stacks: u32, charging: bool, super_run: bool) -> String {
    use super::hero::State;
    let off = "|Tab:Menus";
    match state {
        _ if charging => "Release Space:Launch".to_string(),
        State::Grounded | State::Land | State::HardLanding if super_run => "Shift+W:Super run|Space:Take off".to_string() + off,
        State::Grounded | State::Land | State::HardLanding => {
            "Space:Fly (hold: launch)|Ctrl+Space:Jump|Shift+W:Run|Shift+2xW:Super run|LMB:Punch (hold: charge)|RMB:Thunderclap".to_string() + off
        }
        State::Flight if dive => "Release Ctrl:Pull up".to_string(),
        State::Flight if fast && super_on => {
            let stack = if stacks >= tune::STACKS_MAX { "|Max stacks".to_string() } else { format!("|Hold Space (full):Stack {}/{}", stacks + 1, tune::STACKS_MAX) };
            format!("W/S:Speed|Ctrl:Dive|2xS:Hover{stack}|LMB:Punch{off}")
        }
        State::Flight if fast => "W/S:Speed|Ctrl:Dive|2xS:Hover|Hold Space:Boost|LMB:Punch".to_string() + off,
        State::Hover | State::Flight => "WASD:Move|2xW:Fast|Hold Space:Boost|Space:Drop|Ctrl:Down|LMB:Punch|RMB:Thunderclap".to_string() + off,
        State::Falling | State::Jump => "Space:Hover".to_string() + off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_and_a_charge() {
        let mut h = SpaceHold::default();
        let dt = 1.0 / 60.0;
        assert_eq!(h.step(true, true, 0.0, dt, 1.4), SpaceEvent::Idle);
        assert_eq!(h.step(false, false, 0.1, dt, 1.4), SpaceEvent::Tap);
        // Held: a charge from TAP_S, full after `full_s` more, let go.
        h.step(true, true, 1.0, dt, 1.4);
        let mut t = 1.0;
        let mut started = 0;
        while t < 1.0 + tune::TAP_S + 1.5 {
            t += dt;
            if h.step(false, true, t, dt, 1.4) == SpaceEvent::Started {
                started += 1;
            }
        }
        assert_eq!(started, 1);
        assert_eq!(h.step(false, false, t + dt, dt, 1.4), SpaceEvent::Release(1.0));
        assert_eq!(h.step(false, false, t + 2.0 * dt, dt, 1.4), SpaceEvent::Idle);
    }

    #[test]
    fn a_double_tap_is_two_presses_close_together() {
        let mut last = f64::NEG_INFINITY;
        assert!(!double_tap(&mut last, true, 1.0));
        assert!(double_tap(&mut last, true, 1.2));
        // A third right after starts again.
        assert!(!double_tap(&mut last, true, 1.3));
        assert!(!double_tap(&mut last, true, 2.0));
    }

    #[test]
    fn a_full_launch_breaks_the_barrier() {
        let sound = 340.0;
        assert!(launch_speed(0.0, 1.0, sound) > sound);
        assert_eq!(launch_speed(0.0, 0.0, sound), tune::FAST_MIN_MS);
        // From faster than the top, no loss.
        assert_eq!(launch_speed(2000.0, 1.0, sound), 2000.0);
    }

    #[test]
    fn w_climbs_s_brakes_none_holds() {
        let dt = 1.0 / 60.0;
        let aim = DVec3::X;
        let mut t = 0.0;
        let mut v = DVec3::X * 100.0;
        for _ in 0..60 {
            v = fast_step(v, &mut t, aim, false, false, false, 3.2, 30_000.0, dt);
        }
        assert!((v.length() - 100.0).abs() < 1e-6, "{v}");
        let mut v2 = v;
        for i in 0..60 {
            v2 = fast_step(v2, &mut t, aim, true, i == 0, false, 3.2, 30_000.0, dt);
        }
        assert!(v2.length() > 250.0, "{}", v2.length());
        let mut v3 = v2;
        for _ in 0..60 {
            v3 = fast_step(v3, &mut t, aim, false, false, true, 3.2, 30_000.0, dt);
        }
        assert!(v3.length() < v2.length() * 0.2, "{}", v3.length());
    }

    #[test]
    fn a_ground_launch_goes_up_at_least_fifteen_degrees() {
        let up = DVec3::Y;
        let d = launch_dir(DVec3::X, up);
        assert!((d.dot(up).asin().to_degrees() - tune::LAUNCH_MIN_UP_DEG).abs() < 1e-9);
        let steep = DVec3::new(1.0, 2.0, 0.0).normalize();
        assert_eq!(launch_dir(steep, up), steep);
    }

    #[test]
    fn the_dive_turns_down_and_speeds_up() {
        let mut v = DVec3::X * 100.0;
        for _ in 0..120 {
            v = dive_step(v, DVec3::NEG_Y, 30_000.0, 1.0 / 60.0);
        }
        assert!(v.normalize().dot(DVec3::NEG_Y) > 0.99 && v.length() > 155.0, "{v}");
    }
}
