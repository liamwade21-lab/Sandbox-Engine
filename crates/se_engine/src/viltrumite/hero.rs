//! His state machine (spec F4) and each frame's move: flight, X, falling, walking, jumps and landings, what the frames
//! do, the camera and the clips. The ground itself is `ground.rs`.

use std::cell::Cell;

use bevy::math::{DQuat, DVec2, DVec3};
use bevy::prelude::*;
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::hero_asset::ClipSet;
use se_core::units::G;
use se_core::upos::UPos;

use crate::camera::{FlyCamera, Follow, TravelKind};
use crate::data::SeData;
use crate::mods::{HeroAssets, ModInput};
use crate::ship::air::AirCache;
use crate::ship::{BodyInfo, body_mass, gravity_at, velocity_at};
use crate::system::SystemBody;
use crate::terrain::Terrain;
use crate::universe::SimTime;
use crate::volumetric_clouds::VolumetricClouds;

use super::air::{self, AirModel, Flow};
use super::anim::{Layer, Sampler};
use super::camera;
use super::cape::CapeIn;
use super::frames::{Frame, Ground, Motion, Outcome, Scene, height_over, to_local};
use super::ground::{self, Footing, Landing, WalkEvent, WalkIn};
use super::readout::Readout;
use super::tune;
use super::{Hero, facing};

/// What he's doing (F4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Hover,
    Flight,
    Falling,
    /// On his feet: standing, walking, running or sprinting, by his speed (the gaits are the clips').
    Grounded,
    /// Going up from a jump: gravity, no drag.
    Jump,
    /// The short stop.
    Land,
    /// The hard one. (The crater and dust are step 8's: `Hero.impact`.)
    HardLanding,
}

impl State {
    fn motion(self) -> Motion {
        match self {
            State::Hover | State::Grounded | State::Land | State::HardLanding => Motion::Held,
            State::Flight => Motion::Flown,
            State::Falling | State::Jump => Motion::Ballistic,
        }
    }

    /// On the ground, moved by `ground::walk_step`.
    pub fn on_foot(self) -> bool {
        matches!(self, State::Grounded | State::Land | State::HardLanding)
    }
}

/// A landing's event, for the effects (step 8: the crater and dust, the splash; step 6's sounds). Only ever overwritten:
/// nothing builds up while nothing reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Impact {
    /// `speed` m/s onto the ground, at this point of the body's frame.
    Hard { obj: ObjectRef, local: DVec3, speed: f64 },
    /// Into the sea.
    Splash { obj: ObjectRef, local: DVec3, speed: f64 },
}

/// The flight tiers (F3's clips), by speed with hysteresis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tier {
    #[default]
    Hover,
    Cruise,
    Fast,
    Hyper,
}

/// The tier at `speed`, coming from `tier`.
pub fn tier_at(tier: Tier, speed: f64) -> Tier {
    let (cruise_up, fast_up) = (tune::CRUISE_RISE_MS, tune::FAST_RISE_MS);
    let (cruise_down, fast_down) = (tune::CRUISE_FALL_MS, tune::FAST_FALL_MS);
    if speed < tune::HOVER_UNDER_MS {
        return Tier::Hover;
    }
    match tier {
        Tier::Hover | Tier::Cruise => {
            if speed >= fast_up {
                Tier::Hyper
            } else if speed >= cruise_up {
                Tier::Fast
            } else {
                Tier::Cruise
            }
        }
        Tier::Fast => {
            if speed >= fast_up {
                Tier::Hyper
            } else if speed < cruise_down {
                Tier::Cruise
            } else {
                Tier::Fast
            }
        }
        Tier::Hyper => {
            if speed >= fast_down {
                Tier::Hyper
            } else if speed < cruise_down {
                Tier::Cruise
            } else {
                Tier::Fast
            }
        }
    }
}

// ---- Flight -------------------------------------------------------------------------------------------------------

/// Speed on the Shift ramp after `t` seconds held, logarithmic between the table's points, doubling beyond them.
pub fn ramp_speed(t: f64) -> f64 {
    let table = &tune::RAMP;
    if t <= table[0].0 {
        return table[0].1;
    }
    for w in table.windows(2) {
        if t <= w[1].0 {
            let k = (t - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 * (w[1].1 / w[0].1).powf(k);
        }
    }
    let (t_end, v_end) = table[table.len() - 1];
    (v_end * 2f64.powf((t - t_end) / tune::SPACE_DOUBLE_S)).min(tune::GUARD_MS)
}

/// The Shift time that gives `speed`: pressing Shift again goes on from the speed he has.
pub fn ramp_time(speed: f64) -> f64 {
    let table = &tune::RAMP;
    if speed <= table[0].1 {
        return table[0].0;
    }
    for w in table.windows(2) {
        if speed <= w[1].1 {
            return w[0].0 + (w[1].0 - w[0].0) * (speed / w[0].1).ln() / (w[1].1 / w[0].1).ln();
        }
    }
    let (t_end, v_end) = table[table.len() - 1];
    t_end + tune::SPACE_DOUBLE_S * (speed / v_end).log2()
}

/// What the keys ask of his flight.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlightIn {
    /// Where the move keys point (unit, universe axes), zero with none.
    pub wish: DVec3,
    /// Shift.
    pub boost: bool,
    /// Shift pressed this frame.
    pub boost_edge: bool,
    /// W alone holds his speed.
    pub hold: bool,
}

impl FlightIn {
    pub fn any(&self) -> bool {
        self.boost || self.wish != DVec3::ZERO
    }
}

/// The velocity one step on (frame-relative, universe axes), and the ramp's time. `aim`: forward. `cap`: the most he
/// may fly here.
pub fn flight_step(v: DVec3, ramp_t: &mut f64, i: &FlightIn, aim: DVec3, cap: f64, dt: f64) -> DVec3 {
    let speed = v.length();
    let dir_now = v.try_normalize();
    let toward = if i.wish != DVec3::ZERO {
        i.wish
    } else if i.boost {
        aim
    } else {
        dir_now.unwrap_or(aim)
    };
    let mut target = if i.boost {
        if i.boost_edge {
            *ramp_t = ramp_time(speed);
        }
        *ramp_t += dt;
        // Never further along the ramp than the cap lets him go, or leaving the air would be a jump.
        *ramp_t = ramp_t.min(ramp_time(cap));
        ramp_speed(*ramp_t + tune::RAMP_TAU_S)
    } else if i.hold {
        speed.max(tune::FLY_MS)
    } else if i.wish != DVec3::ZERO {
        tune::FLY_MS
    } else {
        0.0
    };
    target = target.min(cap);
    let brake = 0.5f64.powf(dt / tune::BRAKE_HALF_S);
    let cos = dir_now.map_or(1.0, |d| d.dot(toward));
    // Turned right round at speed: he stops first.
    let reversing = speed > tune::FLY_MS * tune::SLOW_FLIGHT_FACTOR && cos < -0.5;
    let new_speed = if speed > cap {
        cap + (speed - cap) * (-dt / tune::CAP_BRAKE_TAU_S).exp()
    } else if reversing {
        speed * brake
    } else if target > speed {
        speed + (target - speed) * (1.0 - (-dt / tune::RAMP_TAU_S).exp())
    } else {
        (speed * brake).max(target)
    };
    let new_speed = new_speed.min(tune::GUARD_MS);
    if speed <= tune::FLY_MS * tune::SLOW_FLIGHT_FACTOR {
        // Slow: straight to what's asked.
        let want = toward * target.max(if i.any() { new_speed } else { 0.0 });
        let a = 1.0 - (-dt / tune::SLOW_FLIGHT_TAU_S).exp();
        let nv = v + (want - v) * a;
        // (On the ramp the speed goes by the ramp, not by this blend.)
        return if i.boost && nv.length() < new_speed { nv.normalize_or(toward) * new_speed } else { nv };
    }
    let dir = match dir_now {
        Some(d) if !reversing => slerp_dir(d, toward, 1.0 - (-dt / tune::STEER_TAU_S).exp()),
        Some(d) => d,
        None => toward,
    };
    dir * new_speed
}

/// `a` turned towards `b` by share `k` of the angle between them, keeping unit length (opposite: about any axis).
pub fn slerp_dir(a: DVec3, b: DVec3, k: f64) -> DVec3 {
    let angle = a.angle_between(b);
    if angle < 1e-9 {
        return b;
    }
    let axis = a.cross(b).try_normalize().unwrap_or_else(|| a.any_orthonormal_vector());
    (DQuat::from_axis_angle(axis, angle * k) * a).normalize()
}

/// The cap at this height over the ground, in this air, at this distance from the nearest surface.
pub fn cap_at(air: Option<(&AirModel, f64)>, nearest_surface: f64) -> f64 {
    if air.is_some_and(|(a, h)| h < a.top_m()) {
        return tune::CAP_AIR_MS;
    }
    (tune::CAP_SPACE_PER_S * nearest_surface).max(tune::CAP_AIR_MS).min(tune::GUARD_MS)
}

// ---- The clips ----------------------------------------------------------------------------------------------------

/// What the base layer plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pose {
    #[default]
    Idle,
    ArmsCrossed,
    Cruise,
    Fast,
    Hyper,
    HyperUp,
    /// The take-off's crouch, then `ToFast`.
    Crouch,
    ToFast,
    /// Coming out of speed.
    Break,
    ToIdle,
    Fall,
    FallLoop,
    /// On his feet, Mixamo's clips.
    GroundIdle,
    TurnLeft,
    TurnRight,
    Walk,
    Run,
    Sprint,
    /// Off the ground, then rising, then coming down.
    JumpStart,
    JumpLoop,
    JumpDown,
    Land,
    LandHard,
}

/// The clips he plays, found once.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clips {
    pose: [Option<u16>; 23],
    /// Forward, back, left, right, up, down.
    idle_lean: [Option<u16>; 6],
    /// Left, right, up, down.
    fast_lean: [Option<u16>; 4],
    hyper_lean: [Option<u16>; 4],
}

impl Clips {
    fn find(set: &ClipSet) -> Clips {
        let one = |name: &str| {
            let i = set.find(name).map(|i| i as u16);
            if i.is_none() {
                warn!("viltrumite: no clip '{name}'");
            }
            i
        };
        Clips {
            pose: [
                one("bf_idle"),
                one("bf_idle_arms_crossed"),
                one("bf_idle_combat2"),
                one("bf_fast_botharmsback"),
                one("bf_fast_rightarmpointingahead"),
                one("bf_hyper_up_rightarmahead"),
                one("charge_flight_stopped"),
                one("bf_idle_to_fast"),
                one("bf_break"),
                one("bf_fast_to_idle"),
                one("fall"),
                one("fall_loop"),
                one("idle"),
                one("turn_left"),
                one("turn_right"),
                one("walk"),
                one("run"),
                one("sprint"),
                one("male@jump_fall_start_stopped"),
                one("male@jump_fall_loop"),
                one("male@jump_fall_down_loop"),
                one("male@jump_fall_land_hard"),
                one("land_punch"),
            ],
            idle_lean: [one("bf_idle_fwd_add"), one("bf_idle_bwd_add"), one("bf_idle_left_add"), one("bf_idle_right_add"), one("bf_idle_up_add"), one("bf_idle_down_add")],
            fast_lean: [one("bf_fast_botharmsback_left"), one("bf_fast_botharmsback_right"), one("bf_fast_botharmsback_up"), one("bf_fast_botharmsback_down")],
            hyper_lean: [one("bf_fast_left_add"), one("bf_fast_right_add"), one("bf_fast_up_add"), one("bf_fast_down_add")],
        }
    }

    fn of(&self, pose: Pose) -> Option<u16> {
        self.pose[pose as usize]
    }
}

/// The base layer and its one-shots, the leans. Works on the sampler's layers each frame; everything else is the sampler's.
#[derive(Clone, Copy, Debug, Default)]
pub struct Director {
    clips: Option<Clips>,
    pub pose: Pose,
    pub tier: Tier,
    /// Seconds still in Hover.
    still: f32,
    /// Right, up, forward, each -1..1, smoothed.
    lean: [f32; 3],
    /// A dev command plays one clip alone (`hero clip`).
    pub pinned: bool,
    started: bool,
    /// A turn clip has ended: his body takes over this much of the root's turn (radians about his up, + to his left).
    turned: Option<f64>,
}

/// One frame's facts for the clips.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cue {
    pub state: State,
    pub speed: f64,
    /// Degrees above level of his flight.
    pub climb_deg: f64,
    /// Slow and not going forward: the idle pose with its leans says it better than the cruise pose.
    pub sideways: bool,
    /// Right, up, forward, each -1..1: how the velocity or the steering lies in his axes.
    pub lean: [f32; 3],
    /// Move keys or Shift held.
    pub keys: bool,
    /// Shift pressed.
    pub boost_edge: bool,
    /// Falling from a jump or off a ledge, not from flight: the short fall's clip.
    pub short_fall: bool,
    /// On his feet: on what's too steep to stand on.
    pub sliding: bool,
    /// On his feet: W, A, S or D held.
    pub walking: bool,
    /// On his feet: how far the aim is off where he faces (degrees, + to his left).
    pub off_aim_deg: f64,
}

impl Director {
    /// What the base layer should play for this frame's facts, one-shots aside.
    fn wanted(&self, c: &Cue) -> Pose {
        match c.state {
            State::Falling | State::Jump => Pose::Fall,
            State::Grounded | State::Land | State::HardLanding => Pose::GroundIdle,
            State::Hover => {
                if self.still > tune::ARMS_CROSSED_AFTER_S {
                    Pose::ArmsCrossed
                } else {
                    Pose::Idle
                }
            }
            State::Flight => match self.tier {
                Tier::Hover => Pose::Idle,
                Tier::Cruise if c.sideways => Pose::Idle,
                Tier::Cruise => Pose::Cruise,
                Tier::Fast => Pose::Fast,
                Tier::Hyper => {
                    let was_up = self.pose == Pose::HyperUp;
                    let (rise, fall) = tune::HYPER_UP_DEG;
                    if c.climb_deg > if was_up { fall } else { rise } { Pose::HyperUp } else { Pose::Hyper }
                }
            },
        }
    }

    fn go(&mut self, s: &mut Sampler, pose: Pose, fade: f32) {
        self.go_at(s, pose, 0.0, 1.0, fade);
    }

    /// `pose` from `time` into its clip, at `rate`.
    fn go_at(&mut self, s: &mut Sampler, pose: Pose, time: f32, rate: f32, fade: f32) {
        let Some(clips) = self.clips else { return };
        self.pose = pose;
        s.crossfade(fade);
        s.layers[0] = Layer { clip: clips.of(pose), time, weight: 1.0, rate, ..Layer::default() };
    }

    /// A turn clip is playing: his body holds still for it, its root does the turning.
    pub fn turning(&self) -> bool {
        matches!(self.pose, Pose::TurnLeft | Pose::TurnRight)
    }

    /// A turn clip ended since last asked: how much his body takes over.
    pub fn take_turned(&mut self) -> Option<f64> {
        self.turned.take()
    }

    /// The gait for this speed, coming from the one he has.
    fn gait(&self, speed: f64) -> Pose {
        let moving = matches!(self.pose, Pose::Walk | Pose::Run | Pose::Sprint);
        let idle = if moving { tune::GAIT_IDLE_MS.1 } else { tune::GAIT_IDLE_MS.0 };
        let run = if matches!(self.pose, Pose::Run | Pose::Sprint) { tune::GAIT_RUN_MS.1 } else { tune::GAIT_RUN_MS.0 };
        let sprint = if self.pose == Pose::Sprint { tune::GAIT_SPRINT_MS.1 } else { tune::GAIT_SPRINT_MS.0 };
        if speed < idle {
            Pose::GroundIdle
        } else if speed >= sprint {
            Pose::Sprint
        } else if speed >= run {
            Pose::Run
        } else {
            Pose::Walk
        }
    }

    /// On his feet, and in a jump or a short fall: the clips of F4's table.
    fn on_foot(&mut self, c: &Cue, s: &mut Sampler, set: &ClipSet, clips: &Clips) {
        let clip_of = |p: Pose| clips.of(p).and_then(|i| set.clips.get(i as usize));
        let event = |p: Pose, name: &str| clip_of(p).and_then(|k| set.events_of(k).iter().find(|e| e.name == name)).map(|e| e.time);
        let time = s.layers[0].time;
        let end = s.layers[0].clip.and_then(|i| set.clips.get(i as usize)).map_or(0.0, |k| k.duration);
        match c.state {
            State::Land => {
                if self.pose != Pose::Land {
                    self.go_at(s, Pose::Land, tune::LAND_CLIP_FROM_S, tune::LAND_CLIP_RATE, tune::FADE_LAND_S);
                }
            }
            State::HardLanding => {
                if self.pose != Pose::LandHard {
                    // From its contact: the ground has just been reached.
                    self.go_at(s, Pose::LandHard, event(Pose::LandHard, "contact").unwrap_or(0.27), 1.0, tune::FADE_LAND_S);
                }
            }
            State::Jump => match self.pose {
                Pose::JumpStart => {
                    if time >= tune::JUMP_CLIP_END_S {
                        self.go(s, Pose::JumpLoop, tune::FADE_S);
                    }
                }
                Pose::JumpLoop => {}
                _ => self.go_at(s, Pose::JumpStart, tune::JUMP_CLIP_FROM_S, tune::JUMP_CLIP_RATE, tune::FADE_GAIT_S),
            },
            State::Falling => {
                if self.pose != Pose::JumpDown {
                    self.go(s, Pose::JumpDown, tune::FADE_S);
                }
            }
            _ if c.sliding => {
                if self.pose != Pose::JumpDown {
                    self.go(s, Pose::JumpDown, tune::FADE_S);
                }
            }
            _ if self.turning() => {
                if c.walking {
                    let want = self.gait(c.speed.max(tune::GAIT_IDLE_MS.0));
                    let at = event(want, "foot_l").unwrap_or(0.0);
                    self.go_at(s, want, at, 1.0, tune::FADE_GAIT_S);
                } else if time >= end {
                    // His body takes the root's turn over, and the pose as it stands goes the other way: no jump.
                    let deg = if self.pose == Pose::TurnLeft { tune::TURN_CLIP_DEG } else { -tune::TURN_CLIP_DEG };
                    self.turned = Some(deg.to_radians());
                    s.rebase_yaw(deg.to_radians() as f32);
                    self.go(s, Pose::GroundIdle, tune::FADE_S);
                }
            }
            _ => {
                let want = self.gait(c.speed);
                let walked = matches!(self.pose, Pose::Walk | Pose::Run | Pose::Sprint);
                if want != self.pose {
                    match want {
                        Pose::GroundIdle => {
                            let fade = if matches!(self.pose, Pose::Land | Pose::LandHard) { tune::FADE_UP_S } else { tune::FADE_S };
                            self.go(s, Pose::GroundIdle, fade);
                        }
                        _ => {
                            // Starting: on left-foot contact. Changing gait: at the same place in the stride.
                            let at = event(want, "foot_l").unwrap_or(0.0);
                            let at = if walked && end > 0.0 {
                                let was = event(self.pose, "foot_l").unwrap_or(0.0);
                                let share = ((time - was) / end).rem_euclid(1.0);
                                at + share * clip_of(want).map_or(0.0, |k| k.duration)
                            } else {
                                at
                            };
                            let fade = if matches!(self.pose, Pose::Land | Pose::LandHard) { tune::FADE_UP_S } else { tune::FADE_GAIT_S };
                            self.go_at(s, want, at, 1.0, fade);
                        }
                    }
                } else if want == Pose::GroundIdle && !c.walking && c.off_aim_deg.abs() > tune::TURN_CLIP_OVER_DEG {
                    let turn = if c.off_aim_deg > 0.0 { Pose::TurnLeft } else { Pose::TurnRight };
                    self.go(s, turn, tune::FADE_GAIT_S);
                }
                // A gait's clock by his speed over its stride, in the rate the spec allows.
                if matches!(self.pose, Pose::Walk | Pose::Run | Pose::Sprint) {
                    let stride = clip_of(self.pose).map_or(0.0, |k| k.stride_speed as f64);
                    if stride > 0.0 {
                        let (lo, hi) = tune::STRIDE_RATE;
                        s.layers[0].rate = (c.speed / stride).clamp(lo, hi) as f32;
                    }
                }
            }
        }
    }

    /// Sets the layers for this frame.
    pub fn update(&mut self, c: &Cue, s: &mut Sampler, set: &ClipSet, dt: f32) {
        if self.pinned {
            return;
        }
        let clips = *self.clips.get_or_insert_with(|| Clips::find(set));
        if !self.started {
            self.started = true;
            self.pose = Pose::Idle;
            s.play(clips.of(Pose::Idle).unwrap_or(0), 0.0);
        }
        self.tier = tier_at(self.tier, c.speed);
        self.still = if c.state == State::Hover { self.still + dt } else { 0.0 };
        if c.state.on_foot() || c.state == State::Jump || (c.state == State::Falling && c.short_fall) {
            self.on_foot(c, s, set, &clips);
            self.set_leans(c, s, set, &clips, dt);
            return;
        }
        let want = self.wanted(c);
        let time = s.layers[0].time;
        let end = s.layers[0].clip.and_then(|i| set.clips.get(i as usize)).map_or(0.0, |k| k.duration);
        let upright = matches!(self.pose, Pose::Idle | Pose::ArmsCrossed | Pose::Cruise);
        let speedy = matches!(self.pose, Pose::Fast | Pose::Hyper | Pose::HyperUp);
        match self.pose {
            _ if c.state == State::Falling && !matches!(self.pose, Pose::Fall | Pose::FallLoop) => self.go(s, Pose::Fall, tune::FADE_S),
            // `fall`'s last frame is `fall_loop`'s first: no crossfade.
            Pose::Fall if c.state == State::Falling => {
                if time >= end {
                    self.pose = Pose::FallLoop;
                    s.layers[0] = Layer { clip: clips.of(Pose::FallLoop), time: 0.0, weight: 1.0, ..Layer::default() };
                }
            }
            Pose::FallLoop if c.state == State::Falling => {}
            Pose::Crouch => {
                if c.state == State::Hover || !c.keys {
                    self.go(s, want, tune::FADE_S);
                } else if time >= tune::TAKE_OFF_CROUCH_S {
                    self.go(s, Pose::ToFast, tune::FADE_BREAK_S);
                }
            }
            Pose::ToFast => {
                if time >= end || c.state == State::Hover {
                    self.go(s, want, tune::FADE_S);
                }
            }
            Pose::ToIdle => {
                if time >= end || c.keys && c.speed >= tune::CRUISE_RISE_MS {
                    self.go(s, want, tune::FADE_S);
                }
            }
            Pose::Break => {
                if c.state == State::Hover || c.keys || c.speed < tune::HOVER_UNDER_MS {
                    self.go(s, want, tune::FADE_S);
                }
            }
            _ if upright && c.boost_edge && c.state == State::Flight => self.go(s, Pose::Crouch, tune::FADE_BREAK_S),
            _ if speedy && !c.keys && c.state == State::Flight && c.speed >= tune::FAST_FALL_MS => self.go(s, Pose::Break, tune::FADE_BREAK_S),
            _ if speedy && matches!(want, Pose::Cruise | Pose::Idle | Pose::ArmsCrossed) => self.go(s, Pose::ToIdle, tune::FADE_S),
            pose if pose != want => {
                let fade = if matches!(want, Pose::Hyper | Pose::HyperUp) && speedy { tune::FADE_HYPER_S } else { tune::FADE_S };
                self.go(s, want, fade);
            }
            _ => {}
        }
        self.set_leans(c, s, set, &clips, dt);
    }

    fn set_leans(&mut self, c: &Cue, s: &mut Sampler, set: &ClipSet, clips: &Clips, dt: f32) {
        let k = 1.0 - (-dt / tune::LEAN_TAU_S).exp();
        for (l, t) in self.lean.iter_mut().zip(c.lean) {
            *l += (t.clamp(-1.0, 1.0) - *l) * k;
        }
        let [right, up, fwd] = self.lean;
        // One-way ramps: time is how far he leans, the weight goes to 0 with the lean (`hero_asset`'s comment).
        let lean_layer = |clip: Option<u16>, amount: f32| -> Layer {
            let a = amount.clamp(0.0, 1.0);
            let d = clip.and_then(|i| set.clips.get(i as usize)).map_or(0.0, |k| k.duration);
            Layer { clip, time: a * d, weight: (a / tune::LEAN_FULL_WEIGHT_AT).min(1.0), ..Layer::default() }
        };
        let side = |pair: [Option<u16>; 2], v: f32| lean_layer(if v >= 0.0 { pair[0] } else { pair[1] }, v.abs());
        let none = Layer::default();
        let (a, b, d) = match self.pose {
            Pose::Idle | Pose::ArmsCrossed => {
                let g = clips.idle_lean;
                (side([g[0], g[1]], fwd), side([g[3], g[2]], right), side([g[4], g[5]], up))
            }
            Pose::Fast => {
                let g = clips.fast_lean;
                (side([g[1], g[0]], right), side([g[2], g[3]], up), none)
            }
            Pose::Hyper => {
                let g = clips.hyper_lean;
                (side([g[1], g[0]], right), side([g[2], g[3]], up), none)
            }
            _ => (none, none, none),
        };
        s.layers[1] = a;
        s.layers[2] = b;
        s.layers[3] = d;
    }
}

// ---- A frame ------------------------------------------------------------------------------------------------------

/// A body's spin (rad per second of the game's clock) over a minute, so the date's rounding (0.04 % of a day's spin over
/// the second `spin_at` uses) stays out of his velocity; a body that turns faster than that can tell gets `spin_at`.
fn smooth_spin(eph: &Ephemeris, obj: ObjectRef, jd: f64) -> DVec3 {
    // (A quaternion and its negative are the same turn, and the orientation's sign can flip between two dates: the
    // scaled axis must come from the one with w over 0, or a tiny turn reads as nearly a whole one for that frame.)
    let turn = |h: f64| {
        let dq = (eph.orientation(obj, jd + h) * eph.orientation(obj, jd - h).inverse()).normalize();
        (if dq.w < 0.0 { -dq } else { dq }).to_scaled_axis()
    };
    let w = turn(30.0 / 86_400.0);
    if w.length() < 1.0 && w.is_finite() { w / 60.0 } else { turn(0.5 / 86_400.0) }
}

/// A body's velocity over a minute, for the same reason.
fn smooth_velocity(eph: &Ephemeris, obj: ObjectRef, jd: f64) -> DVec3 {
    let h = 30.0 / 86_400.0;
    let v = eph.position(obj, jd + h).delta_m(eph.position(obj, jd - h)) / 60.0;
    if v.is_finite() { v } else { velocity_at(eph, obj, jd) }
}

/// The height the air (F9) and the readout count from: over the ground, but a giant's over its radius (`over_radius`,
/// its 1 atm is there), whatever floor the camera has under it: that floor is 10 of its decks' scale heights down, and
/// only when the clouds are on.
fn air_height(giant: bool, over_radius: f64, local: DVec3, ground: Ground) -> f64 {
    if giant { over_radius } else { local.length() - ground.radial }
}

/// The height the air counts from, at a point of the body's frame.
fn air_height_on(b: &BodyInfo, local: DVec3, ground: Ground) -> f64 {
    air_height(b.terrain.is_some_and(|t| t.proc.kind.is_giant()), height_over(b, local), local, ground)
}

/// F2: at the camera's spot, ahead of it, in Hover. The camera doesn't move: he appears where it looks.
fn turn_on(hero: &mut Hero, cam: &mut FlyCamera) {
    hero.placed = true;
    hero.state = State::Hover;
    hero.ramp_t = 0.0;
    hero.was_boost = false;
    hero.dist = tune::SPAWN_AHEAD_M;
    hero.zoom = 0.0;
    let aim = cam.rot;
    hero.frame = Frame::at(cam.pos.offset_m(-camera::offset(aim, hero.dist)));
    hero.rot = aim;
    hero.spawned = true;
    hero.cam_written = cam.pos;
    hero.carried = false;
    hero.ease = 0.0;
    hero.clip_lift = 0.0;
    hero.root_drop = 0.0;
    hero.shift_t = 0.0;
    hero.state_t = 0.0;
    hero.short_fall = false;
    hero.sliding = false;
    hero.keys_up_first = false;
    hero.ctrl_up_first = false;
    hero.carried_from = None;
    hero.impact = None;
    hero.last_vel = DVec3::ZERO;
    hero.felt = DVec3::ZERO;
    hero.flow = Flow::default();
    hero.heat_k = 0.0;
    // (Turned off at speed it still says Hyper, and the first frame would sound like leaving it.)
    hero.director.tier = Tier::Hover;
    cam.travel = None;
    cam.velocity = DVec3::ZERO;
    cam.thrust = DVec3::ZERO;
    cam.turn_vel = DVec3::ZERO;
}

/// He's where the camera says, from a Go To or a jump.
fn follow_camera(hero: &mut Hero, cam: &FlyCamera) {
    let dist = hero.dist + hero.zoom;
    hero.frame = Frame::at(cam.pos.offset_m(-camera::offset(cam.rot, dist)));
    hero.state = State::Hover;
    hero.spawned = true;
}

/// He's over the sea: 1.5 m up, and hovering. To walk on, fly to land.
fn hover_over_sea(hero: &mut Hero, s: &Scene) {
    if let Frame::Attached { obj, local_pos, local_vel, .. } = &mut hero.frame {
        if let Some(b) = s.bodies.iter().find(|b| b.obj == *obj) {
            let g = (s.ground)(b, *local_pos);
            let to = g.radial + s.stand_m + tune::SEA_HOVER_M;
            hero.ease += local_pos.length() - to;
            *local_pos = local_pos.normalize_or(DVec3::Y) * to;
            *local_vel = DVec3::ZERO;
        }
    }
    hero.state = State::Hover;
    hero.keys_up_first = true;
}

/// He's come down: F4's three landings where there's land, the sea's hover, a stop on anything else. He stops where he hit it, not
/// where a flight's slide went on to.
fn touch_down(hero: &mut Hero, s: &Scene, into: f64, out: &Outcome) {
    let Frame::Attached { obj, local_pos, .. } = &mut hero.frame else {
        hero.state = State::Hover;
        return;
    };
    let obj = *obj;
    *local_pos = out.at;
    let local_pos = out.at;
    let Some(b) = s.bodies.iter().find(|b| b.obj == obj) else { return };
    hero.state_t = 0.0;
    match (s.footing)(b, local_pos) {
        Footing::Land => match ground::landing_of(into, out.speed) {
            Landing::Soft => {
                hero.state = State::Grounded;
                hero.frame.set_local_velocity(out.along.clamp_length_max(tune::SPRINT_MS));
            }
            Landing::Short => {
                hero.state = State::Land;
                hero.frame.set_local_velocity(DVec3::ZERO);
            }
            Landing::Hard => {
                hero.state = State::HardLanding;
                hero.frame.set_local_velocity(DVec3::ZERO);
                hero.impact = Some(Impact::Hard { obj, local: local_pos, speed: out.speed });
                hero.sound.hard_landing();
            }
        },
        Footing::Sea => {
            hero.impact = Some(Impact::Splash { obj, local: local_pos, speed: out.speed });
            hero.sound.splash(out.speed);
            hover_over_sea(hero, s);
        }
        Footing::Nothing => {
            hero.state = State::Hover;
            hero.frame.set_local_velocity(DVec3::ZERO);
            // With Ctrl still held every frame would be a take-off and this stop, and each take-off a sound.
            hero.ctrl_up_first = true;
        }
    }
}

/// F4's Hyper: a tier of flight. (`Director::tier` also follows a fall's speed, and a fall isn't Hyper: no flight ends when
/// the air slows it.)
fn flying_hyper(state: State, tier: Tier) -> bool {
    state == State::Flight && tier == Tier::Hyper
}

/// F10's "leaving Hyper", from last frame's `flying_hyper`: slowing, X, Z, a Go To, the sea's stop. On his feet he's crashed, and that
/// has its own sound.
fn leaves_hyper(was_hyper: bool, state: State, tier: Tier) -> bool {
    was_hyper && !flying_hyper(state, tier) && !state.on_foot()
}

/// Model space (Z up, +Y forward) to the port's (Y up, -Z forward): `draw.rs` `model_to_port` as a turn.
fn model_to_port_q() -> Quat {
    Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

/// One frame of him: every bit of his motion and the free camera's place, in `UniverseSet::Camera`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hero_move(
    time: Res<Time<Real>>,
    sim: Res<SimTime>,
    data: Res<SeData>,
    input: Res<ModInput>,
    mut hero: ResMut<Hero>,
    mut cam: ResMut<FlyCamera>,
    mut air_cache: ResMut<AirCache>,
    clouds: Res<VolumetricClouds>,
    assets: Res<HeroAssets>,
    q: Query<(&SystemBody, Option<&Terrain>)>,
) {
    let hero = &mut *hero;
    let dt = time.delta_secs_f64().min(0.1);
    hero.clock += dt;
    let eph = Ephemeris::new(&data.catalog);
    let bodies: Vec<BodyInfo> = q
        .iter()
        .map(|(b, terrain)| BodyInfo { obj: b.obj, pos: b.pos, orient: b.orient, radius: b.radius_m, shape: b.shape, gm: G * body_mass(&data, b.obj, b.radius_m), terrain })
        .collect();
    let decks = clouds.active.and_then(|e| q.get(e).ok()).map(|(b, _)| b.obj);
    let camera_m = hero.dist + hero.zoom;
    let ground = |b: &BodyInfo, local: DVec3| ground::support_height(b, local, decks, camera_m);
    let footing = |b: &BodyInfo, local: DVec3| ground::footing(b, local);
    let spin = |obj: ObjectRef| smooth_spin(&eph, obj, sim.jd);
    let vel = |obj: ObjectRef| smooth_velocity(&eph, obj, sim.jd);

    if !hero.placed {
        turn_on(hero, &mut cam);
    }
    // (After that: turned off in Hyper it still says so. Before all below that changes the state: X, Z, a Go To.)
    let was_hyper = flying_hyper(hero.state, hero.director.tier);
    // F8: every frame, not once: the Track and bind buttons and a Go To set them. `hold_agl`, a dev command's, would
    // fight his height.
    cam.follow = None;
    cam.track = false;
    cam.hold_agl = None;
    cam.velocity = DVec3::ZERO;

    // A Go To or a landing carries him; WASD cancels it (F8). Shift, Ctrl and Space don't: Shift+G and Ctrl+G are held
    // down to start one, so it would cancel itself in its first frame.
    let moving_keys = input.forward != 0.0 || input.right != 0.0;
    if moving_keys && cam.travel.as_ref().is_some_and(|t| matches!(t.kind, TravelKind::Goto | TravelKind::Land)) {
        cam.travel = None;
    }
    // A Go To into a body's centre (Ctrl+G) ends at its ground: he can't go in, and the spot ahead of a camera near the
    // centre swings right round the body.
    let mut at_ground = false;
    let into_body = cam.travel.as_ref().filter(|t| t.kind == TravelKind::Goto && t.end_distance > 0.0).and_then(|t| Some((t.end_distance, bodies.iter().find(|b| b.obj == t.target && b.radius > 0.0)?)));
    if let Some((end, b)) = into_body {
        let local = to_local(b, cam.pos);
        let g = ground(b, local);
        // (Only a flight that ends under the ground: a plain G from the ground starts with the camera on its floor.)
        if g.level.is_some() && end < g.radial && local.length() < g.radial + tune::CAM_FLOOR_M {
            cam.pos = b.pos.offset_m(b.orient * (local.normalize_or(DVec3::Y) * (g.radial + tune::CAM_FLOOR_M)));
            cam.travel = None;
            at_ground = true;
        }
    }
    let travelling = cam.travel.as_ref().is_some_and(|t| matches!(t.kind, TravelKind::Goto | TravelKind::Land));
    // A jump of the camera by something else (Places, a dev command): he goes with it. The last frame of a Go To too. (What
    // `keep_camera_above_ground` pushed it by isn't that: a finer node's ground rising past the camera's floor.)
    hero.cam_written = hero.cam_written.offset_m(cam.floor_push);
    if travelling || at_ground || hero.carried || cam.pos.delta_m(hero.cam_written).length() > tune::TELEPORT_M {
        follow_camera(hero, &cam);
    }
    hero.carried = travelling;
    let carried_cam = travelling.then_some(cam.pos);

    // The nearest body by its surface: his up, his air, the readout. (Air is worked out once a frame, for it alone.)
    let pos0 = hero.frame.world_or(hero.world.0);
    let near_of = |pos: UPos| {
        bodies.iter().filter(|b| b.radius > 0.0).map(|b| (b, height_over(b, to_local(b, pos)))).min_by(|a, b| a.1.total_cmp(&b.1))
    };
    let air_model = near_of(pos0).filter(|(b, _)| !matches!(b.obj, ObjectRef::Star(_))).and_then(|(b, _)| Some((b.obj, AirModel::of(&mut air_cache, &data, &eph, sim.jd, b.obj, b.radius, b.terrain)?)));
    // F4: no drag rising from a jump.
    let no_drag = Cell::new(false);
    let drag = |b: &BodyInfo, local: DVec3| match &air_model {
        Some((obj, a)) if *obj == b.obj && !no_drag.get() => a.drag(air_height_on(b, local, ground(b, local))),
        _ => 0.0,
    };
    let scene = Scene { bodies: &bodies, spin: &spin, vel: &vel, ground: &ground, footing: &footing, drag: &drag, stand_m: tune::STAND_M };

    // The keys that change how he moves, before the frame's carry: falling, it and the step belong together (`Frame::carry`).
    if input.z && !travelling && !input.shift && !input.ctrl && !input.alt {
        // Z stops him, whatever he's doing (Go To's stop is input_controls'). On his feet he stays on them.
        if !hero.state.on_foot() {
            hero.state = State::Hover;
        }
        hero.frame.set_velocity(&scene, DVec3::ZERO);
    } else if input.x_pressed && !travelling && matches!(hero.state, State::Hover | State::Flight) && !input.alt {
        hero.frame.to_ballistic(&scene);
        hero.state = State::Falling;
        hero.short_fall = false;
        hero.sound.stop_flying();
    } else if input.space_pressed && matches!(hero.state, State::Falling | State::Jump) {
        // F4: he holds still where he is, at once. (Space still held, he flies up from there.) That's the double tap too:
        // the first press jumped, this is the second.
        hero.frame.set_velocity(&scene, DVec3::ZERO);
        hero.state = State::Hover;
    } else if input.space_pressed && hero.state == State::Grounded && !travelling {
        hero.sound.jump(hero.frame.velocity(&scene).length() >= tune::GAIT_SPRINT_MS.1);
        hero.frame.kick_up(tune::JUMP_MS);
        hero.state = State::Jump;
        hero.short_fall = true;
    }

    // A test's orbit: at K x the circular speed round the nearest body, the way he looks.
    if let Some(k) = hero.dev_orbit.take() {
        let pos = hero.frame.world_or(hero.world.0);
        if let Some((b, _)) = near_of(pos) {
            let rel = pos.delta_m(b.pos);
            let f = cam.rot * DVec3::NEG_Z;
            let along = (f - rel * (f.dot(rel) / rel.length_squared())).normalize_or_zero();
            let v = along * k * (b.gm / rel.length()).sqrt();
            hero.frame.to_ballistic(&scene);
            hero.state = State::Falling;
            hero.frame = match hero.frame {
                Frame::Attached { obj, local_pos, level, last_orient, .. } if obj == b.obj => {
                    let w = b.orient.inverse() * spin(b.obj);
                    Frame::Attached { obj, local_pos, local_vel: b.orient.inverse() * v - w.cross(local_pos), level, last_orient }
                }
                // (`carry` picks its carrier next and takes that body's motion off a fall's velocity: here it never had it.)
                _ => Frame::Free { pos, vel: v + vel(b.obj), carrier: None },
            };
        }
    }

    // What the bodies did since last frame. The view and his body turn with the frame.
    let motion = hero.state.motion();
    let turn = hero.frame.carry(&scene, dt, motion, hero.world.0);
    cam.rot = (turn * cam.rot).normalize();
    hero.rot = (turn * hero.rot).normalize();
    // The cape (below) feels what he did against this, not what the game clock turned: it runs on real seconds, whatever the
    // clock does (F5).
    let was_rot = hero.rot;
    hero.last_vel = turn * hero.last_vel;
    hero.felt = turn * hero.felt;
    // What he stood on is gone (its system unloaded).
    if hero.state.on_foot() && !hero.frame.attached() {
        hero.state = State::Hover;
    }

    // Spawned under the ground: lifted (F2). Ground not loaded: `advance` holds him in the air.
    if std::mem::take(&mut hero.spawned) {
        let pos = hero.frame.world(&scene, hero.world.0);
        if let Some((b, _)) = near_of(pos) {
            let local = to_local(b, pos);
            let g = ground(b, local);
            if g.level.is_some() && local.length() < g.radial + tune::STAND_M {
                let lifted = b.pos.offset_m(b.orient * (local.normalize_or(DVec3::Y) * (g.radial + tune::SPAWN_LIFT_M)));
                hero.frame = Frame::at(lifted);
                hero.frame.carry(&scene, dt, Motion::Held, lifted);
            }
        }
    }

    // Where he is, for the up and the height.
    let pos = hero.frame.world(&scene, hero.world.0);
    let near = near_of(pos).map(|(b, _)| (b, to_local(b, pos)));
    let up_here = near.and_then(|(b, local)| camera::horizon_up(b, local));
    let horizon = up_here.filter(|_| cam.auto_horizon);

    // The look.
    if input.mouse != Vec2::ZERO && cam.travel.is_none() && !cam.locks.rotation {
        let d = input.mouse.as_dvec2() * cam.mouse_sensitivity * (cam.fov_deg as f64 / 60.0);
        cam.rot = camera::turn_aim(cam.rot, DVec2::new(d.x, d.y), horizon);
    }
    if let Some(up) = horizon.filter(|_| cam.travel.is_none()) {
        cam.rot = camera::level_roll(cam.rot, up, dt);
    }
    if input.wheel != 0.0 {
        hero.dist = (hero.dist * tune::CAM_WHEEL.powf(input.wheel as f64)).clamp(tune::CAM_MIN_M, tune::CAM_MAX_M);
    }

    // The keys.
    let mut walking = false;
    // He walked off something this frame.
    let mut fall_starts = false;
    let aim = cam.rot;
    let (fwd, right, aim_up) = (aim * DVec3::NEG_Z, aim * DVec3::X, aim * DVec3::Y);
    let lift = up_here.unwrap_or(aim_up);
    let wish = (fwd * input.forward as f64 + right * input.right as f64 + lift * input.up as f64).normalize_or_zero();
    // (A Go To carries him: Shift and Ctrl held from Shift+G and Ctrl+G, or Space, fly nothing meanwhile.)
    let mut flight_in = if travelling { FlightIn::default() } else { FlightIn { wish, boost: input.shift, boost_edge: input.shift && !hero.was_boost, hold: input.forward > 0.0 } };
    // Off the shore with the keys down: he hovers there until they're let go, or running into the sea launches him.
    if hero.keys_up_first {
        if flight_in.any() {
            flight_in = FlightIn::default();
        } else {
            hero.keys_up_first = false;
        }
    }
    if hero.ctrl_up_first {
        if input.ctrl && !travelling {
            flight_in = FlightIn::default();
        } else {
            hero.ctrl_up_first = false;
        }
    }
    let boost_released = hero.was_boost && !input.shift;
    hero.was_boost = input.shift;
    let take_off = hero.state == State::Hover && flight_in.any();
    if take_off {
        hero.state = State::Flight;
        hero.ramp_t = 0.0;
        hero.sound.take_off();
        if flight_in.boost {
            hero.sound.launch();
        }
    }
    if hero.state == State::Flight && !travelling {
        if flight_in.boost_edge || (take_off && flight_in.boost) {
            hero.sound.boost_start();
        }
        if boost_released {
            hero.sound.boost_end();
        }
    }
    if hero.state.on_foot() {
        hero.state_t += dt;
        // The keys, along the ground. (Looking straight down, the view's up says which way forward is.)
        let keys = hero.state == State::Grounded && !travelling;
        let up = near.map_or(lift, |(b, local)| b.orient * local.normalize_or(DVec3::Y));
        let flat = fwd - up * fwd.dot(up);
        let view_up = aim_up * -fwd.dot(up).signum();
        let ahead = if flat.length() < 0.05 { (view_up - up * view_up.dot(up)).normalize_or(right) } else { flat.normalize() };
        let side = ahead.cross(up);
        let wish = if keys { (ahead * input.forward as f64 + side * input.right as f64).normalize_or_zero() } else { DVec3::ZERO };
        walking = wish != DVec3::ZERO;
        hero.shift_t = if input.shift { hero.shift_t + dt } else { 0.0 };
        let speed = if !input.shift {
            tune::WALK_MS
        } else if hero.shift_t >= tune::SPRINT_AFTER_S {
            tune::SPRINT_MS
        } else {
            tune::RUN_MS
        };
        if let Some((b, local)) = near {
            let a_frame = gravity_at(&bodies, b.pos);
            let i = WalkIn { wish: b.orient.inverse() * wish, speed, gravity: (gravity_at(&bodies, b.pos.offset_m(b.orient * local)) - a_frame).length() };
            let out = ground::walk_step(&mut hero.frame, &scene, &i, dt);
            hero.ease += out.ease;
            hero.slope_deg = out.slope_deg;
            hero.sliding = out.sliding;
            match out.event {
                WalkEvent::Ledge => {
                    hero.state = State::Falling;
                    hero.short_fall = true;
                    fall_starts = true;
                }
                WalkEvent::Sea => {
                    if let Frame::Attached { obj, local_pos, .. } = hero.frame {
                        hero.impact = Some(Impact::Splash { obj, local: local_pos, speed: out.speed });
                    }
                    hero.sound.splash(out.speed);
                    hover_over_sea(hero, &scene);
                }
                WalkEvent::Walked | WalkEvent::Held => {}
            }
        }
        if hero.state == State::Land && hero.state_t >= tune::LAND_S || hero.state == State::HardLanding && hero.state_t >= tune::HARD_LANDING_S {
            hero.state = State::Grounded;
        }
    } else if hero.state == State::Flight {
        let height = near.map_or(f64::INFINITY, |(b, local)| air_height_on(b, local, ground(b, local)));
        // The nearest surface from where he is against this frame's bodies. (`cam.nearest_surface` compares last frame's
        // camera with them: at x1000 hundreds of km out, whichever side of a moon he's on.)
        let surface = near.map_or(f64::INFINITY, |(b, local)| height_over(b, local).max(0.0)).min(cam.nearest_galaxy);
        let cap = cap_at(air_model.as_ref().map(|(_, a)| (a, height)), surface);
        let v = hero.frame.velocity(&scene);
        let nv = flight_step(v, &mut hero.ramp_t, &flight_in, fwd, cap, dt);
        if !flight_in.any() && nv.length() < tune::HOVER_UNDER_MS {
            hero.frame.set_velocity(&scene, DVec3::ZERO);
            hero.state = State::Hover;
        } else {
            hero.frame.set_velocity(&scene, nv);
        }
    }
    // (The frame he walks off something, the fall waits for the next one: this frame's `carry` was on foot, and a ballistic step
    // after it would throw him by the ground's turn, 7.75 m at Earth's equator.)
    if !hero.state.on_foot() && !fall_starts {
        let motion = hero.state.motion();
        no_drag.set(hero.state == State::Jump);
        let out = hero.frame.advance(&scene, dt, motion);
        no_drag.set(false);
        // F4: falling he lands. Flying, with Ctrl or by hitting it hard enough.
        let flown = out.flown_into.filter(|&into| {
            let footing = hero.frame.body().and_then(|o| bodies.iter().find(|b| b.obj == o)).map_or(Footing::Nothing, |b| (scene.footing)(b, out.at));
            hero.state == State::Flight && ground::flight_lands(into, input.ctrl, footing)
        });
        if let Some(into) = out.contact.or(flown) {
            touch_down(hero, &scene, into, &out);
        }
    }
    hero.ease = (hero.ease * (-dt / tune::GROUND_EASE_TAU_S).exp()).clamp(-tune::GROUND_EASE_MAX_M, tune::GROUND_EASE_MAX_M);

    // Where that left him.
    let pos = hero.frame.world(&scene, hero.world.0);
    let near = near_of(pos).map(|(b, _)| (b, to_local(b, pos)));
    let height = near.map(|(b, local)| air_height_on(b, local, ground(b, local)));
    let v = match hero.state {
        State::Falling | State::Jump => hero.frame.over_ground(&scene),
        _ => hero.frame.velocity(&scene),
    };
    let speed = v.length();
    // Past the top of the jump.
    if hero.state == State::Jump && near.is_none_or(|(b, local)| v.dot(b.orient * local.normalize_or(DVec3::Y)) <= 0.0) {
        hero.state = State::Falling;
        hero.short_fall = true;
    }

    // His body: along the aim when slow, along his velocity when fast (F4).
    let up_body = near.and_then(|(b, local)| camera::horizon_up(b, local)).unwrap_or(aim_up);
    let level = |look: DVec3, up: DVec3| {
        let flat = look - up * look.dot(up);
        // Looking straight along his up: the view's own up tells his heading.
        if flat.length() < 0.05 { facing(aim_up * -look.dot(up).signum(), up) } else { facing(flat, up) }
    };
    let dir = v.try_normalize().unwrap_or(fwd);
    let climb_deg = dir.dot(up_body).clamp(-1.0, 1.0).asin().to_degrees();
    let along = speed > tune::BODY_ALONG_VELOCITY_MS && hero.state == State::Flight;
    // On his feet: towards where he's going, or the aim when he stands; held for a turn clip and a landing.
    let holds = hero.director.turning() || hero.hold_body;
    let landing = matches!(hero.state, State::Land | State::HardLanding);
    let stands = hero.state == State::Grounded && !walking && speed < tune::GAIT_IDLE_MS.0;
    let body_tau = if landing {
        tune::LAND_UPRIGHT_TAU_S
    } else if stands {
        tune::STAND_TURN_TAU_S
    } else {
        tune::BODY_TAU_S
    };
    let target = if landing {
        // Down from a head-first dive he's upright at once, still facing where he was going.
        level(hero.rot * DVec3::NEG_Z, up_body)
    } else if holds {
        hero.rot
    } else if hero.state == State::Grounded && !stands {
        level(dir, up_body)
    } else if hero.director.pose == Pose::HyperUp {
        // Arm up: his head leads, and his back is to the camera.
        let flat = fwd - dir * fwd.dot(dir);
        facing(if flat.length() < 0.05 { -up_body } else { flat }, dir)
    } else if along {
        // (Straight along his up: the view's.)
        facing(dir, if dir.dot(up_body).abs() > 0.95 { aim_up } else { up_body })
    } else {
        level(fwd, up_body)
    };
    hero.rot = hero.rot.slerp(target, 1.0 - (-dt / body_tau).exp()).normalize();

    // The clips.
    let body_inv = hero.rot.inverse();
    let in_body = body_inv * v;
    let steer = body_inv * flight_in.wish.try_normalize().unwrap_or(if flight_in.boost { fwd } else { dir });
    let slow = speed <= tune::BODY_ALONG_VELOCITY_MS;
    let lean = if slow {
        [(in_body.x / tune::FLY_MS) as f32, (in_body.y / tune::FLY_MS) as f32, (-in_body.z / tune::FLY_MS) as f32]
    } else {
        [(steer.x * 2.0) as f32, (steer.y * 2.0) as f32, 0.0]
    };
    let forwardish = dir.dot(fwd) > 0.7;
    // On his feet: how far the aim is off where he faces.
    let facing_now = hero.rot * DVec3::NEG_Z;
    let aim_flat = fwd - up_body * fwd.dot(up_body);
    let off_aim_deg = if hero.state == State::Grounded && aim_flat.length() > 0.05 {
        facing_now.cross(aim_flat).dot(up_body).atan2(facing_now.dot(aim_flat)).to_degrees()
    } else {
        0.0
    };
    let air_moves = !hero.state.on_foot() && hero.state != State::Jump && !(hero.state == State::Falling && hero.short_fall);
    let cue = Cue {
        state: hero.state,
        speed,
        climb_deg,
        sideways: slow && !forwardish,
        lean: if air_moves { lean } else { [0.0; 3] },
        keys: flight_in.any(),
        boost_edge: take_off && flight_in.boost,
        short_fall: hero.short_fall,
        sliding: hero.sliding && hero.state == State::Grounded,
        walking,
        off_aim_deg: if hero.hold_body { 0.0 } else { off_aim_deg },
    };
    let hero_ref = &mut *hero;
    hero_ref.director.update(&cue, &mut hero_ref.sampler, &assets.clips, dt as f32);
    if leaves_hyper(was_hyper, hero_ref.state, hero_ref.director.tier) {
        hero_ref.sound.leave_hyper();
    }
    if let Some(yaw) = hero.director.take_turned() {
        hero.rot = (hero.rot * DQuat::from_rotation_y(yaw)).normalize();
    }

    // The cape's frame (`cape.rs`): what the cloth feels, in his model's axes. Clamped here while it's still f64: at 1000 c
    // what changes in a frame doesn't fit the cloth's numbers.
    if dt > 1e-4 {
        let a_frame = hero.frame.body().and_then(|o| bodies.iter().find(|b| b.obj == o)).map_or(DVec3::ZERO, |b| gravity_at(&bodies, b.pos));
        let felt = gravity_at(&bodies, pos) - a_frame - (v - hero.last_vel) / dt;
        let felt = if felt.is_finite() { felt.clamp_length_max(tune::CAPE_ACCEL_MAX as f64 * 2.0) } else { DVec3::ZERO };
        hero.felt += (felt - hero.felt) * (1.0 - (-dt / 0.05).exp());
        let air = air_model.as_ref().filter(|(o, _)| near.is_some_and(|(b, _)| b.obj == *o)).zip(height).map_or(0.0, |((_, a), h)| a.at(h).density);
        let to_model = (hero.rot.as_quat() * model_to_port_q()).inverse();
        let turn = to_model * (was_rot.as_quat() * model_to_port_q());
        hero.sampler.cape_in = CapeIn {
            dt: dt as f32,
            accel: to_model * hero.felt.as_vec3(),
            wind: to_model * (-v).clamp_length_max(tune::CAPE_WIND_MAX_MS as f64).as_vec3(),
            air: (air as f32 / tune::CAPE_AIR_FULL).clamp(0.0, 1.0),
            turn: turn.normalize(),
        };
        hero.last_vel = v;
    }

    // The camera: exact, from his UPos (and the ground's new node, eased, as he's drawn). A floor over the ground of whatever it's in.
    hero.zoom = camera::ease_zoom(hero.zoom, speed, dt);
    let back = camera::offset(cam.rot, hero.dist + hero.zoom);
    let eased = near.map_or(DVec3::ZERO, |(b, local)| b.orient * local.normalize_or(DVec3::Y) * hero.ease);
    let mut cam_pos = pos.offset_m(back + eased);
    for b in &bodies {
        let rel = cam_pos.delta_m(b.pos);
        // (As far out as `keep_camera_above_ground` looks, whose push he'd take for something else moving the camera.)
        let top = b.terrain.map_or(b.radius, |t| t.top_radius().max(b.radius));
        if rel.length() > top * b.shape.max_element().max(1.0) * 1.2 {
            continue;
        }
        let local = b.orient.inverse() * rel;
        if let Some(lifted) = camera::floored(ground(b, local), local) {
            cam_pos = b.pos.offset_m(b.orient * lifted);
        }
    }
    cam.pos = cam_pos;
    hero.cam_written = cam_pos;
    hero.world = (pos, hero.rot);
    hero.carrier = hero.frame.body().and_then(|o| bodies.iter().find(|b| b.obj == o)).map(|b| {
        let locked = hero.frame.turn_share(&scene) >= 0.5;
        Follow { target: b.obj, last_pos: b.pos, last_rot: if locked { b.orient } else { DQuat::IDENTITY }, locked, sync: locked }
    });

    // The readout.
    let a_frame = hero.frame.body().and_then(|o| bodies.iter().find(|b| b.obj == o)).map_or(DVec3::ZERO, |b| gravity_at(&bodies, b.pos));
    // A Go To or a landing carries him at rest in his own frame (`follow_camera`): through the air he goes as fast as the camera
    // goes over the ground. (The camera, not him: he swings round it as the view turns.)
    let carried = carried_cam.zip(near).map(|(p, (b, _))| (b.obj, to_local(b, p)));
    let through_air = match (carried, hero.carried_from) {
        (Some((obj, now)), Some((from_obj, from))) if obj == from_obj => (now - from).length() / time.delta_secs_f64().max(1e-6),
        (Some(_), _) => 0.0,
        (None, _) => speed,
    };
    hero.carried_from = carried;
    hero.flow = air_model.as_ref().filter(|(o, _)| near.is_some_and(|(b, _)| b.obj == *o)).zip(height).map_or(Flow::vacuum(through_air), |((_, a), h)| a.flow(h, through_air));
    hero.heat_k = air::ease_heat(hero.heat_k, hero.flow.skin_k, dt);
    // The readout's height is his soles', where there's anything to stand on: nothing when he stands.
    let soles = near.filter(|(b, local)| (scene.footing)(b, *local) != Footing::Nothing).map_or(0.0, |_| tune::STAND_M);
    hero.readout = Readout {
        speed: through_air,
        mach: hero.flow.in_air().then_some(hero.flow.mach),
        height: height.map(|h| (h - soles).max(if soles > 0.0 { 0.0 } else { h })),
        gravity_g: (gravity_at(&bodies, pos) - a_frame).length() / 9.806_65,
        heat: 0.0,
    };

    // A test's trace.
    if let Some(trace) = hero.trace.as_mut() {
        let body = trace.body.and_then(|o| bodies.iter().find(|b| b.obj == o)).or(near.map(|n| n.0));
        let local = body.map_or(DVec3::ZERO, |b| to_local(b, pos));
        let seen = cam.rot.inverse() * pos.delta_m(cam.pos);
        // How far the view is rolled off his horizon.
        let roll = up_body.dot(cam.rot * DVec3::X).clamp(-1.0, 1.0).asin().to_degrees();
        trace.write(&format!(
            "{:.7},{:.9},{},{},{:?},{:?},{:.5},{:.5},{:.5},{:.4},{:.4},{:.4},{:.4},{:.4},{:.5},{:.5},{:.5},{:.4}\n",
            hero.clock,
            sim.jd,
            sim.rate,
            hero.frame.attached() as u8,
            hero.state,
            hero.director.pose,
            local.x,
            local.y,
            local.z,
            height.unwrap_or(0.0),
            speed,
            v.x,
            v.y,
            v.z,
            seen.x,
            seen.y,
            seen.z,
            roll,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flies by a test's keys and says the speed at the given times.
    fn fly(i: FlightIn, from: f64, cap: f64, times: &[f64]) -> Vec<f64> {
        let dt = 1.0 / 60.0;
        let (mut v, mut ramp) = (DVec3::NEG_Z * from, 0.0);
        let mut out = Vec::new();
        let mut t = 0.0;
        let mut edge = i.boost;
        for &want in times {
            while t < want - dt / 2.0 {
                let j = FlightIn { boost_edge: edge, ..i };
                edge = false;
                v = flight_step(v, &mut ramp, &j, DVec3::NEG_Z, cap, dt);
                t += dt;
            }
            out.push(v.length());
        }
        out
    }

    fn near(v: f64, want: f64, share: f64) -> bool {
        (v / want - 1.0).abs() < share
    }

    /// The spec's table, within 25 %: 300 m/s at 1 s, 3 km/s at 3 s, 30 km/s at 6 s.
    #[test]
    fn the_shift_ramp_is_what_the_spec_says() {
        let i = FlightIn { boost: true, ..default() };
        let got = fly(i, 0.0, tune::GUARD_MS, &[1.0, 3.0, 6.0]);
        eprintln!("ramp: {got:?}");
        assert!(near(got[0], 300.0, 0.25), "{got:?}");
        assert!(near(got[1], 3_000.0, 0.25), "{got:?}");
        assert!(near(got[2], 30_000.0, 0.25), "{got:?}");
    }

    /// In space: x2 per second beyond the ramp. c about 13 s after it, 1000 c about 23 s after it.
    #[test]
    fn the_space_ramp_doubles() {
        let i = FlightIn { boost: true, ..default() };
        let got = fly(i, 0.0, tune::GUARD_MS, &[6.0, 6.0 + 13.3, 6.0 + 23.3]);
        eprintln!("space ramp: {got:?}");
        assert!(near(got[1], 3.0e8, 0.3), "{got:?}");
        assert!(near(got[2], 3.0e11, 0.3), "{got:?}");
    }

    #[test]
    fn the_cap_in_air_stops_the_ramp_at_30_km_s() {
        let i = FlightIn { boost: true, ..default() };
        let got = fly(i, 0.0, tune::CAP_AIR_MS, &[6.0, 10.0, 20.0]);
        assert!(got[1] <= tune::CAP_AIR_MS * 1.001 && got[2] <= tune::CAP_AIR_MS * 1.001, "{got:?}");
        assert!(got[2] > tune::CAP_AIR_MS * 0.99, "{got:?}");
    }

    #[test]
    fn over_a_cap_he_brakes_with_its_time_constant() {
        let got = fly(FlightIn { boost: true, ..default() }, 1e8, 1e7, &[tune::CAP_BRAKE_TAU_S, 0.4, 2.0]);
        let e = (1e8 - 1e7) * (-1.0f64).exp() + 1e7;
        assert!(near(got[0], e, 0.05), "{got:?} against {e}");
        assert!(near(got[2], 1e7, 0.01), "{got:?}");
    }

    /// The cap in space eases him in: flying flat out at a body from far off, he arrives at the cap in air's speed, not
    /// at many times the speed of light. (With the spec's brake of 0.2 s he reached the ground at 2 c.)
    #[test]
    fn the_cap_in_space_eases_him_in() {
        let dt = 1.0 / 60.0;
        let (mut v, mut ramp, mut d) = (DVec3::NEG_Z * 1.0e9, 0.0, 3.0e9f64);
        let i = FlightIn { boost: true, ..default() };
        let mut most = 0.0f64;
        while d > 0.0 {
            v = flight_step(v, &mut ramp, &i, DVec3::NEG_Z, cap_at(None, d), dt);
            d -= v.length() * dt;
            // Never far over the cap where he is, once he's been braked to it.
            if d < 1.0e8 {
                most = most.max(v.length() / cap_at(None, d.max(0.0)));
            }
        }
        assert!(v.length() < tune::CAP_AIR_MS * 1.5, "hit at {} m/s", v.length());
        assert!(most < 2.0, "{most} x the cap");
    }

    #[test]
    fn leaving_the_air_at_full_shift_is_not_a_jump() {
        // A long stretch at the air's cap, then the cap lifts.
        let dt = 1.0 / 60.0;
        let (mut v, mut ramp) = (DVec3::NEG_Z, 0.0);
        let i = FlightIn { boost: true, ..default() };
        for k in 0..1200 {
            let j = FlightIn { boost_edge: k == 0, ..i };
            v = flight_step(v, &mut ramp, &j, DVec3::NEG_Z, tune::CAP_AIR_MS, dt);
        }
        let before = v.length();
        v = flight_step(v, &mut ramp, &i, DVec3::NEG_Z, tune::GUARD_MS, dt);
        assert!(v.length() < before * 1.05, "{} after {before}", v.length());
    }

    #[test]
    fn w_alone_holds_speed_and_no_keys_halve_it() {
        let i = FlightIn { hold: true, wish: DVec3::NEG_Z, ..default() };
        let got = fly(i, 3000.0, tune::GUARD_MS, &[1.0, 5.0]);
        assert!(near(got[0], 3000.0, 1e-3) && near(got[1], 3000.0, 1e-3), "{got:?}");
        let got = fly(FlightIn::default(), 3000.0, tune::GUARD_MS, &[0.15, 0.3]);
        assert!(near(got[0], 1500.0, 0.02), "{got:?}");
        assert!(near(got[1], 750.0, 0.02), "{got:?}");
    }

    #[test]
    fn wasd_alone_is_the_fly_speed() {
        let i = FlightIn { wish: DVec3::NEG_Z, hold: true, ..default() };
        let got = fly(i, 0.0, tune::GUARD_MS, &[1.0, 3.0]);
        assert!(near(got[1], tune::FLY_MS, 0.01), "{got:?}");
        // A sideways key at speed comes down to it.
        let i = FlightIn { wish: DVec3::X, ..default() };
        let got = fly(i, 3000.0, tune::GUARD_MS, &[3.0]);
        assert!(near(got[0], tune::FLY_MS, 0.05), "{got:?}");
    }

    /// S at speed stops him first, then goes back.
    #[test]
    fn s_at_speed_stops_then_reverses() {
        let dt = 1.0 / 60.0;
        let (mut v, mut ramp) = (DVec3::NEG_Z * 5000.0, 0.0);
        let i = FlightIn { wish: DVec3::Z, ..default() };
        let mut crossed = false;
        for _ in 0..600 {
            let nv = flight_step(v, &mut ramp, &i, DVec3::NEG_Z, tune::GUARD_MS, dt);
            // Never more than he had, and no jump through zero faster than the brake.
            assert!(nv.length() <= v.length().max(tune::FLY_MS) + 1e-6);
            if nv.z > 0.0 {
                crossed = true;
            }
            v = nv;
        }
        assert!(crossed && near(v.z, tune::FLY_MS, 0.05), "{v}");
    }

    #[test]
    fn steering_at_speed_keeps_it() {
        let dt = 1.0 / 60.0;
        let (mut v, mut ramp) = (DVec3::NEG_Z * 3000.0, 0.0);
        let i = FlightIn { wish: DVec3::new(0.0, 0.7071, -0.7071), hold: true, ..default() };
        for _ in 0..120 {
            v = flight_step(v, &mut ramp, &i, DVec3::NEG_Z, tune::GUARD_MS, dt);
        }
        assert!(near(v.length(), 3000.0, 1e-3));
        assert!(v.normalize().dot(i.wish) > 0.999, "{v}");
    }

    #[test]
    fn ramp_time_is_the_inverse_of_ramp_speed() {
        for t in [0.0, 0.3, 1.0, 2.0, 3.0, 5.0, 6.0, 7.5, 20.0, 40.0] {
            let got = ramp_time(ramp_speed(t));
            assert!((got - t).abs() < 1e-6, "{t}: {got}");
        }
        // Pressing Shift at 3 km/s goes on from there.
        assert!((ramp_time(3_000.0) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn the_caps_are_as_the_spec_says() {
        let air = AirModel::from_params(se_core::ship::aero::AtmoParams { p0_atm: 1.0, t0_k: 288.0, molar_mass: 29.0, gravity: 9.81, adiabat: 0.25, height_m: 0.0 });
        assert_eq!(cap_at(Some((&air, 1000.0)), 1e12), tune::CAP_AIR_MS);
        // Above the air: 3 x the distance.
        assert!((cap_at(Some((&air, 1e7)), 1e8) / 3e8 - 1.0).abs() < 1e-9);
        // c at 100,000 km.
        assert!((cap_at(None, 1.0e8) / se_core::units::SPEED_OF_LIGHT - 1.0).abs() < 0.01);
        // 1500 c at 1 AU.
        assert!((cap_at(None, se_core::units::METERS_PER_AU) / se_core::units::SPEED_OF_LIGHT / 1500.0 - 1.0).abs() < 0.01);
        // Close by it's the floor.
        assert_eq!(cap_at(None, 100.0), tune::CAP_AIR_MS);
        assert_eq!(cap_at(None, f64::INFINITY), tune::GUARD_MS);
    }

    /// A star's stop is never inside the plain sphere the camera is kept out of, at a flattened body's pole too.
    #[test]
    fn a_flattened_spheres_stop_is_outside_the_cameras_sphere() {
        let r = 7.0e8;
        let b = BodyInfo { obj: ObjectRef::Body(0), pos: UPos::ZERO, orient: DQuat::IDENTITY, radius: r, shape: DVec3::new(1.0, 0.9, 1.0), gm: 0.0, terrain: None };
        for dir in [DVec3::Y, DVec3::X, DVec3::new(1.0, 1.0, 0.0).normalize()] {
            let g = ground::support_height(&b, dir * r, None, 0.0);
            assert!(g.radial >= r * (1.0 + tune::SPHERE_STOP_SHARE) * 0.999_999_99, "{dir}: {}", g.radial - r);
        }
    }

    /// A giant's air stands on its radius whatever floor the camera has under it: F9's 1 atm at the radius, and the same
    /// with the clouds off. Elsewhere it's the ground.
    #[test]
    fn a_giants_air_counts_from_its_radius() {
        let r = 7.0e7;
        let local = DVec3::X * r;
        // The decks' floor, 200 km down, and the radius itself: the same air.
        for radial in [r - 200e3 + 50.0, r] {
            assert_eq!(air_height(true, 0.0, local, Ground { radial, level: Some(0) }), 0.0);
        }
        let air = AirModel::from_params(se_core::ship::aero::AtmoParams { p0_atm: 1.0, t0_k: 165.0, molar_mass: 2.3, gravity: 24.8, adiabat: 0.25, height_m: 0.0 });
        let at = air.at(air_height(true, 0.0, local, Ground { radial: r - 200e3, level: Some(0) }));
        assert!((at.pressure_pa - 101_325.0).abs() < 1.0, "{at:?}");
        let terminal = (24.8 / air.drag(0.0)).sqrt();
        assert!((150.0..240.0).contains(&terminal), "{terminal}");
        // Land: over the ground, whatever the radius says.
        assert_eq!(air_height(false, 5000.0, local, Ground { radial: r - 1000.0, level: Some(3) }), 1000.0);
    }

    /// F10's "leaving Hyper" goes by F4's tier of flight, never by a fall's speed.
    #[test]
    fn only_a_flight_leaves_hyper() {
        // Slowing in flight; X; Z, a Go To and the sea's stop.
        assert!(leaves_hyper(true, State::Flight, Tier::Fast));
        assert!(leaves_hyper(true, State::Falling, Tier::Hyper));
        assert!(leaves_hyper(true, State::Hover, Tier::Hover));
        // Still in it; a crash, which has its own sound.
        assert!(!leaves_hyper(true, State::Flight, Tier::Hyper));
        assert!(!leaves_hyper(true, State::HardLanding, Tier::Hover));
        // A fall: at an orbit's speed it's no flight, the air slowing it ends none, and Space stopping it neither.
        assert!(!flying_hyper(State::Falling, Tier::Hyper));
        assert!(!leaves_hyper(false, State::Falling, Tier::Fast));
        assert!(!leaves_hyper(false, State::Hover, Tier::Hover));
    }

    #[test]
    fn tiers_rise_and_fall_with_hysteresis() {
        let mut t = Tier::Hover;
        let mut at = |v: f64| {
            t = tier_at(t, v);
            t
        };
        assert_eq!(at(2.0), Tier::Hover);
        assert_eq!(at(5.0), Tier::Cruise);
        assert_eq!(at(119.0), Tier::Cruise);
        assert_eq!(at(120.0), Tier::Fast);
        // Falling, it holds down to 100.
        assert_eq!(at(101.0), Tier::Fast);
        assert_eq!(at(99.0), Tier::Cruise);
        assert_eq!(at(1500.0), Tier::Hyper);
        assert_eq!(at(1201.0), Tier::Hyper);
        assert_eq!(at(1199.0), Tier::Fast);
        assert_eq!(at(2.9), Tier::Hover);
    }

    fn real_clips() -> Option<ClipSet> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/mods/viltrumite");
        se_core::hero_asset::parse_clips(&std::fs::read(root.join("clips.sevc")).ok()?).ok()
    }

    /// The whole flight, by the real clips: the poses follow the tiers, the take-off and the stop play their one-shots, a
    /// lean's time is how far he leans and its weight goes to 0 with it.
    #[test]
    fn the_clips_follow_the_flight() {
        let Some(set) = real_clips() else { return };
        let mut d = Director::default();
        let mut s = Box::<Sampler>::default();
        let dt = 1.0 / 60.0;
        let step = |c: Cue, secs: f32, d: &mut Director, s: &mut Sampler| {
            let mut cue = c;
            for _ in 0..(secs / dt) as usize {
                d.update(&cue, s, &set, dt);
                s.advance(&set, dt);
                cue.boost_edge = false;
            }
        };
        let hover = Cue { state: State::Hover, ..default() };
        step(hover, 1.0, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Idle);
        step(hover, 4.0, &mut d, &mut s);
        assert_eq!(d.pose, Pose::ArmsCrossed);
        let boost = Cue { state: State::Flight, speed: 100.0, keys: true, boost_edge: true, ..default() };
        step(boost, 0.2, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Crouch);
        step(Cue { speed: 200.0, ..boost }, 0.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::ToFast);
        step(Cue { speed: 400.0, ..boost }, 1.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Fast);
        step(Cue { speed: 5000.0, ..boost }, 0.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Hyper);
        step(Cue { speed: 5000.0, climb_deg: 60.0, ..boost }, 0.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::HyperUp);
        step(Cue { speed: 5000.0, climb_deg: 46.0, ..boost }, 0.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::HyperUp, "hysteresis");
        step(Cue { speed: 5000.0, climb_deg: 30.0, ..boost }, 0.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Hyper);
        // Let go: the break, then still.
        step(Cue { speed: 2000.0, keys: false, ..boost }, 0.2, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Break);
        step(hover, 0.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Idle);
        // Slowing with keys from Fast: to idle, then what's wanted.
        d.pose = Pose::Fast;
        let cruise = Cue { state: State::Flight, speed: 20.0, keys: true, ..default() };
        step(cruise, 0.2, &mut d, &mut s);
        assert_eq!(d.pose, Pose::ToIdle);
        step(cruise, 1.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Cruise);
        // X.
        let fall = Cue { state: State::Falling, speed: 20.0, ..default() };
        step(fall, 0.2, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Fall);
        step(fall, 6.0, &mut d, &mut s);
        assert_eq!(d.pose, Pose::FallLoop);
        step(fall, 20.0, &mut d, &mut s);
        assert_eq!(d.pose, Pose::FallLoop, "the loop stays");
    }

    /// F4's table on foot, by the real clips: standing, the gaits by speed at their stride's rate, the jump's clips, the landings.
    #[test]
    fn the_clips_on_foot_follow_his_speed() {
        let Some(set) = real_clips() else { return };
        let mut d = Director::default();
        let mut s = Box::<Sampler>::default();
        let dt = 1.0 / 60.0;
        let step = |c: Cue, secs: f32, d: &mut Director, s: &mut Sampler| {
            for _ in 0..(secs / dt) as usize {
                d.update(&c, s, &set, dt);
                s.advance(&set, dt);
            }
        };
        let stand = Cue { state: State::Grounded, ..default() };
        step(stand, 0.5, &mut d, &mut s);
        assert_eq!(d.pose, Pose::GroundIdle);
        let name = |s: &Sampler| set.clips[s.layers[0].clip.unwrap() as usize].name.clone();
        assert_eq!(name(&s), "idle");
        // Starting: on left-foot contact.
        let walk = Cue { speed: tune::WALK_MS, walking: true, ..stand };
        d.update(&walk, &mut s, &set, dt);
        assert_eq!(d.pose, Pose::Walk);
        let walk_clip = &set.clips[s.layers[0].clip.unwrap() as usize];
        let foot = set.events_of(walk_clip).iter().find(|e| e.name == "foot_l").unwrap().time;
        assert!((s.layers[0].time - foot).abs() < 1e-3, "{}", s.layers[0].time);
        // The rate is the speed over the stride, within the spec's limits.
        let rate = (tune::WALK_MS as f32 / walk_clip.stride_speed).clamp(0.7, 1.5);
        assert!((s.layers[0].rate - rate).abs() < 1e-3, "{}", s.layers[0].rate);
        step(Cue { speed: tune::RUN_MS, ..walk }, 0.2, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Run);
        step(Cue { speed: tune::SPRINT_MS, ..walk }, 0.2, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Sprint);
        // Hugely over the stride the rate stops at its limit.
        step(Cue { speed: 40.0, ..walk }, 0.1, &mut d, &mut s);
        assert_eq!(s.layers[0].rate, 1.5);
        step(stand, 0.2, &mut d, &mut s);
        assert_eq!(d.pose, Pose::GroundIdle);
        // Off the ground.
        step(Cue { state: State::Jump, short_fall: true, speed: 28.0, ..default() }, 0.1, &mut d, &mut s);
        assert_eq!(d.pose, Pose::JumpStart);
        assert!(name(&s).contains("jump_fall_start_stopped"));
        step(Cue { state: State::Jump, short_fall: true, speed: 20.0, ..default() }, 1.0, &mut d, &mut s);
        assert_eq!(d.pose, Pose::JumpLoop);
        step(Cue { state: State::Falling, short_fall: true, speed: 5.0, ..default() }, 0.1, &mut d, &mut s);
        assert_eq!(d.pose, Pose::JumpDown);
        // Falling from flight is the other clip.
        step(Cue { state: State::Falling, speed: 5.0, ..default() }, 0.1, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Fall);
        step(Cue { state: State::Land, ..default() }, 0.1, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Land);
        step(Cue { state: State::HardLanding, ..default() }, 0.1, &mut d, &mut s);
        assert_eq!(d.pose, Pose::LandHard);
        assert!(s.layers[0].time >= 0.27, "from its contact: {}", s.layers[0].time);
        step(stand, 0.1, &mut d, &mut s);
        assert_eq!(d.pose, Pose::GroundIdle);
        step(Cue { state: State::Hover, ..default() }, 0.1, &mut d, &mut s);
        assert_eq!(d.pose, Pose::Idle);
    }

    /// Over 70 degrees off the aim he plays a turn clip, his body holding still; when it ends his body takes the root's turn over
    /// with no jump in where he faces, and has gone the right way.
    #[test]
    fn a_turn_clip_turns_him_without_a_jump() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/mods/viltrumite");
        let (Ok(mb), Some(set)) = (std::fs::read(root.join("omniman.sevm")), real_clips()) else { return };
        let model = se_core::hero_asset::parse_model(&mb).unwrap();
        let spine = model.bones.iter().position(|b| b.name == "SKEL_Spine3").unwrap();
        for (off, sign) in [(120.0f64, 1.0f64), (-120.0, -1.0)] {
            let mut d = Director::default();
            let mut s = Box::<Sampler>::default();
            let dt = 1.0f32 / 60.0;
            let stand = Cue { state: State::Grounded, ..default() };
            for _ in 0..30 {
                d.update(&stand, &mut s, &set, dt);
                s.advance(&set, dt);
                s.sample(&model, &set);
            }
            // His chest's way in the model is Y, his body's turn about model Z is `body`.
            let mut body = 0.0f64;
            let heading = |s: &Sampler, body: f64| {
                let f = s.world(spine).matrix3 * bevy::math::Vec3A::Y;
                f.x.atan2(f.y) as f64 * -1.0 + body
            };
            let start = heading(&s, body);
            let turned = Cue { off_aim_deg: off, ..stand };
            d.update(&turned, &mut s, &set, dt);
            assert_eq!(d.pose, if sign > 0.0 { Pose::TurnLeft } else { Pose::TurnRight });
            assert!(d.turning());
            let mut last = heading(&s, body);
            let mut most: f64 = 0.0;
            for _ in 0..120 {
                d.update(&turned, &mut s, &set, dt);
                if let Some(yaw) = d.take_turned() {
                    body += yaw;
                }
                s.advance(&set, dt);
                s.sample(&model, &set);
                let h = heading(&s, body);
                most = most.max((h - last).abs());
                last = h;
                if !d.turning() {
                    break;
                }
            }
            assert_eq!(d.pose, Pose::GroundIdle);
            assert!(most < 0.2, "a jump of {most} radians a frame");
            let turned_by = (last - start).to_degrees();
            assert!((turned_by * sign - 90.0).abs() < 25.0, "{off}: turned {turned_by}");
        }
    }

    #[test]
    fn a_lean_is_sampled_by_how_far_never_by_the_clock() {
        let Some(set) = real_clips() else { return };
        let mut d = Director::default();
        let mut s = Box::<Sampler>::default();
        let dt = 1.0 / 60.0;
        let lean = Cue { state: State::Flight, speed: 15.0, keys: true, sideways: true, lean: [-0.5, 0.0, 0.0], ..default() };
        for _ in 0..240 {
            d.update(&lean, &mut s, &set, dt);
            s.advance(&set, dt);
        }
        let layer = s.layers[2];
        let clip = &set.clips[layer.clip.unwrap() as usize];
        assert!(clip.name.ends_with("left_add") && clip.lean(), "{}", clip.name);
        assert!((layer.time / clip.duration - 0.5).abs() < 0.02, "{}", layer.time);
        assert_eq!(layer.weight, 1.0);
        // The other way, the other clip.
        let other = Cue { lean: [0.5, 0.0, 0.0], ..lean };
        for _ in 0..240 {
            d.update(&other, &mut s, &set, dt);
        }
        let clip = &set.clips[s.layers[2].clip.unwrap() as usize];
        assert!(clip.name.ends_with("right_add"), "{}", clip.name);
        // No lean: no weight.
        let straight = Cue { lean: [0.0; 3], ..lean };
        for _ in 0..600 {
            d.update(&straight, &mut s, &set, dt);
        }
        assert!(s.layers[2].weight < 0.01, "{}", s.layers[2].weight);
    }
}
