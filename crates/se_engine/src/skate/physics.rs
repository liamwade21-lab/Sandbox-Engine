//! The skater's physics on the sandbox's ground (its local frame: metres, x east, y north, z up), built on Skate 3's own
//! calculations from the Skate 3 Rust Engine's `skate-core` and Skate 3's tuning values (`tuning.cfg`, `VALUES` and `CURVES`):
//!
//! - **On the ground**: four wheel rays find the ground's height and slope under the board; the board keeps to it (ride height
//!   from the wheels' radius and the trucks), and leaves it when the ground falls away faster than its path (a ramp's lip, a
//!   kerb). Its speed along the nose changes by the slope's pull (capped at Skate 3's `MaxGravityAcceleration`), the pushes
//!   (`riding::push::calculate_acceleration`: at most `MaxPushDVStart`..`MaxPushDVEnd` a stroke, nothing past
//!   `MaxPushableSpeed`), the foot brake (`riding::braking::calculate_braking`, its force over the board's mass), the
//!   surface's rolling friction (`FrictionVsSpeedNew`: none under 8 m/s) and, after `NoInputTime` without input,
//!   `FrictionVsSpeed_NoInput`. Steering is Skate 3's deck tilt (`riding::steering::calculate_tilt`: the stick through
//!   `SteeringVsInput`, the speed through `SteeringVsSpeed`, damped), turned into a turn by the trucks' geometry (their axis at
//!   `TruckRotationAxisAngle`, the wheelbase between them), the sideways grip holding the turn to what the wheels can take.
//!   Riding backwards (fakie) is allowed. A powerslide turns the board across its path and scrubs speed.
//! - **The pop**: Skate 3's `air::ground_jump::calculate` with its jump tables: the body rises to between `JumpMinHeight` and
//!   `JumpMaxHeight` by how long the crouch was held, less at speed (`MinHeightVsSpeed`, `MaxHeightVsSpeed`), along the
//!   ground's normal (`VerticalResponse`).
//! - **In the air**: the planet's gravity; the left stick spins him. Landing with the trick not caught, the board across the
//!   path, or too hard is a bail; so is running into a wall or a tree at speed.

use bevy::math::{Mat3, Quat, Vec2, Vec3};
use se_core::skate3::Tuning;
use skate_core::air::ground_jump::{self, GroundJumpInput, GroundJumpMode, GroundJumpSettings};
use skate_core::math::Vector3;
use skate_core::point_graph::PointGraph;
use skate_core::riding::{braking, push, steering};

use crate::sandbox::world::{Met, SandboxWorld};

/// Skate 3's tuning values the mod reads (`copy_skate3` writes them into `tuning.cfg` as `class/key/field`).
#[cfg_attr(not(test), allow(dead_code))]
pub const VALUES: &[(&str, &str, &str)] = &[
    ("physics_push", "default", "MaxPushableSpeed"),
    ("physics_mode", "normal", "MaxPushDVStart"),
    ("physics_mode", "normal", "MaxPushDVEnd"),
    ("physics_mode", "normal", "JumpMinHeight"),
    ("physics_mode", "normal", "JumpMaxHeight"),
    ("physics_mode", "normal", "Hash_BE3F74F978D777E5"),
    ("physics_brakes", "default", "FootBrakeForce"),
    ("physics_brakes", "default", "TailBrakeForce"),
    ("physics_brakes", "default", "SlowSpeed"),
    ("physicswheels", "default", "WheelRadius"),
    ("physicswheels", "default", "WheelMass"),
    ("physicstrucks", "default", "TruckMass"),
    ("physicsdeck", "default", "DeckMass"),
    ("physics_world", "default", "SkateboardMassFactor"),
    ("physicstrucks", "default", "TruckRotationAxisAngle"),
    ("physicstrucks", "default", "TruckZPosFront"),
    ("physicstrucks", "default", "TruckZPosBack"),
    ("physics_steering", "default", "HardTurnIncrease"),
    ("physics_steering", "default", "Damping"),
    ("physics_steering", "default", "SteeringVsSpeedMaxSpeed"),
    ("physics_steering", "default", "PushScalarIncriment"),
    ("physics_steering", "default", "PushScalarDecriment"),
    ("physics_steering", "default", "PushScalarMin"),
    ("physics_steering", "default", "GeneralScalar"),
    ("physics_steering", "default", "SteeringScalar_TightTrucks"),
    ("physics_steering", "default", "SteeringTiltBlending"),
    ("physics_manual", "default", "Tilt_TiltScalar"),
    ("physics_jump", "default", "SpeedResponseMaxSpeed"),
    ("physics_jump", "default", "MinScalar"),
    ("physics_jump", "default", "JumpYBonusMax"),
    ("physics_jump", "default", "JumpAdjustZFactor"),
    ("physics_jump", "default", "JumpAdjustXFactor"),
    ("physics_jump", "default", "AbsoluteMinHeight"),
    ("physics_jump", "default", "HippyJumpMinHeight"),
    ("physics_jump", "default", "HippyJumpMaxHeight"),
    ("physics_friction", "default", "NoInputTime"),
    ("physics_surfaces", "smooth", "Friction_MaxSpeedChange"),
    ("physics_surfaces", "smooth", "BrakingScalar"),
    ("physics_surfaces", "smooth", "SideFriction_Scalar"),
    ("physics_speed_conservation", "default", "Gravity"),
    ("physics_speed_conservation", "default", "MaxGravityAcceleration"),
    ("physics_speed_conservation", "default", "GeneralAmount"),
    ("physics_speed_conservation", "default", "NegativeGeneralAmount"),
];

/// Skate 3's curves the mod reads.
#[cfg_attr(not(test), allow(dead_code))]
pub const CURVES: &[(&str, &str, &str)] = &[
    ("physics_steering", "default", "SteeringVsSpeed"),
    ("physics_steering", "default", "SteeringVsInput"),
    ("physics_surfaces", "smooth", "FrictionVsSpeedNew"),
    ("physics_friction", "default", "FrictionVsSpeed_NoInput"),
    ("physics_jump", "default", "VerticalResponse"),
    ("physics_jump", "default", "JumpYScalarVsGroundNormalY"),
    ("physics_jump", "default", "JumpSpeedScalarVsAngle"),
    ("physics_jump", "default", "MinHeightVsSpeed"),
    ("physics_jump", "default", "MaxHeightVsSpeed"),
];

/// Skate 3's values, read once from `tuning.cfg`.
pub struct Tune {
    pub max_push_speed: f32,
    pub push_dv: [f32; 2],
    pub brake: braking::BrakeSettings,
    pub board_mass: f32,
    pub wheel_radius: f32,
    pub truck_axis: f32,
    pub steering: steering::SteeringSettings,
    pub jump: GroundJumpSettings,
    pub jump_mode: GroundJumpMode,
    pub friction: PointGraph<8>,
    pub no_input_friction: PointGraph<8>,
    pub no_input_time: f32,
    pub max_gravity_accel: f32,
}

fn graph8(t: &Tuning, k: &str) -> Result<PointGraph<8>, String> {
    let (x, y) = t.curve::<8>(k).map_err(|e| e.to_string())?;
    Ok(PointGraph { x, y })
}

impl Tune {
    pub fn read(t: &Tuning) -> Result<Tune, String> {
        let v = |k: &str| t.value(k).map_err(|e| e.to_string());
        let (vx, vy) = t.curve::<16>("physics_jump/default/VerticalResponse").map_err(|e| e.to_string())?;
        let mass_factor = v("physics_world/default/SkateboardMassFactor")?;
        Ok(Tune {
            max_push_speed: v("physics_push/default/MaxPushableSpeed")?,
            push_dv: [v("physics_mode/normal/MaxPushDVStart")?, v("physics_mode/normal/MaxPushDVEnd")?],
            brake: braking::BrakeSettings {
                input_force: v("physics_brakes/default/FootBrakeForce")?,
                override_force: v("physics_brakes/default/TailBrakeForce")?,
                minimum_speed: v("physics_brakes/default/SlowSpeed")?,
            },
            board_mass: (v("physicsdeck/default/DeckMass")? + 2.0 * v("physicstrucks/default/TruckMass")? + 4.0 * v("physicswheels/default/WheelMass")?) * mass_factor,
            wheel_radius: v("physicswheels/default/WheelRadius")?,
            truck_axis: v("physicstrucks/default/TruckRotationAxisAngle")?.to_radians(),
            steering: steering::SteeringSettings {
                hard_turn_increase: v("physics_steering/default/HardTurnIncrease")?,
                damping: v("physics_steering/default/Damping")?,
                speed_graph_max_speed: v("physics_steering/default/SteeringVsSpeedMaxSpeed")?,
                push_scalar_increment: v("physics_steering/default/PushScalarIncriment")?,
                push_scalar_decrement: v("physics_steering/default/PushScalarDecriment")?,
                push_scalar_min: v("physics_steering/default/PushScalarMin")?,
                manual_scalar: v("physics_manual/default/Tilt_TiltScalar")?,
                general_scalar: v("physics_steering/default/GeneralScalar")?,
                tight_trucks_scalar: v("physics_steering/default/SteeringScalar_TightTrucks")?,
                speed_graph: graph8(t, "physics_steering/default/SteeringVsSpeed")?,
                input_graph: graph8(t, "physics_steering/default/SteeringVsInput")?,
                tilt_blending: v("physics_steering/default/SteeringTiltBlending")?,
            },
            jump: GroundJumpSettings {
                vertical_response: PointGraph { x: vx, y: vy },
                y_scalar_vs_normal_y: graph8(t, "physics_jump/default/JumpYScalarVsGroundNormalY")?,
                speed_scalar_vs_angle: graph8(t, "physics_jump/default/JumpSpeedScalarVsAngle")?,
                minimum_height_vs_speed: graph8(t, "physics_jump/default/MinHeightVsSpeed")?,
                maximum_height_vs_speed: graph8(t, "physics_jump/default/MaxHeightVsSpeed")?,
                speed_response_max_speed: v("physics_jump/default/SpeedResponseMaxSpeed")?,
                minimum_scalar: v("physics_jump/default/MinScalar")?,
                maximum_y_bonus: v("physics_jump/default/JumpYBonusMax")?,
                adjust_z_factor: v("physics_jump/default/JumpAdjustZFactor")?,
                adjust_x_factor: v("physics_jump/default/JumpAdjustXFactor")?,
                absolute_minimum_height: v("physics_jump/default/AbsoluteMinHeight")?,
                hippy_minimum_height: v("physics_jump/default/HippyJumpMinHeight")?,
                hippy_maximum_height: v("physics_jump/default/HippyJumpMaxHeight")?,
            },
            jump_mode: GroundJumpMode {
                minimum_height_64: v("physics_mode/normal/Hash_BE3F74F978D777E5")?,
                minimum_height_68: v("physics_mode/normal/JumpMinHeight")?,
                maximum_height: v("physics_mode/normal/JumpMaxHeight")?,
            },
            friction: graph8(t, "physics_surfaces/smooth/FrictionVsSpeedNew")?,
            no_input_friction: graph8(t, "physics_friction/default/FrictionVsSpeed_NoInput")?,
            no_input_time: v("physics_friction/default/NoInputTime")?,
            max_gravity_accel: v("physics_speed_conservation/default/MaxGravityAcceleration")?,
        })
    }
}

/// The wheels' places on the board: half the wheelbase along the nose, half the track across (Skate 3's rig: the wheel bones).
pub const WHEEL_HALF_BASE: f32 = 0.24;
pub const WHEEL_HALF_TRACK: f32 = 0.10;
/// The deck's middle over the wheels' centres (the rig: the wheel bones 5 cm under `SKATEBOARD_ROOT`).
pub const DECK_OVER_AXLE: f32 = 0.05;
/// The skater and board as a box for walls and props: half sizes and its middle over the deck.
const BODY_HALF: Vec3 = Vec3::new(0.25, 0.25, 0.8);
const BODY_UP: f32 = 0.85;
/// The board leaves the ground when it would be this far over it after a tick on its own path.
const LEAVE_M: f32 = 0.06;
/// Sideways grip: the most sideways acceleration the wheels hold, m/s^2.
const GRIP: f32 = 9.0;
/// Hitting a wall faster than this (m/s into it) is a bail; landing faster than this downwards too.
const WALL_BAIL: f32 = 3.5;
const LAND_BAIL: f32 = 11.0;
/// A powerslide's scrub, m/s^2, and the board's turn across its path.
const SLIDE_DECEL: f32 = 5.5;
pub const SLIDE_ANGLE: f32 = 80.0;
/// Turning on the spot (a kickturn) below this speed, radians a second at full stick.
const KICKTURN_SPEED: f32 = 0.6;
const KICKTURN_RATE: f32 = 3.2;
/// Spinning in the air at full stick, radians a second.
const AIR_SPIN: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ground,
    Air,
    /// Off the board: tumbling to a stop, then up again.
    Bail,
}

/// What a tick asks of the board (from `anim.rs`'s controller).
#[derive(Clone, Copy, Debug, Default)]
pub struct Drive {
    /// Steering, -1 left .. 1 right.
    pub steer: f32,
    /// A push's stroke lands this tick.
    pub push: bool,
    /// Pushing (Skate 3 holds the steering back while it is).
    pub pushing: bool,
    pub brake: bool,
    /// Powerslide: -1 or 1 the side, 0 none.
    pub slide: f32,
    /// Pop this tick, with this strength (0..1).
    pub pop: Option<f32>,
    /// The hips' height over the deck now (the jump's "current height" of the body).
    pub hips_m: f32,
    /// Any input this frame.
    pub any: bool,
}

/// What a tick did, for the animation and the sounds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Happened {
    pub popped: bool,
    /// Landed, at this speed into the ground (m/s).
    pub landed: Option<f32>,
    pub bailed: bool,
    /// The wheels roll on the ground this tick.
    pub rolling: bool,
    /// Hit something standing, at this speed into it.
    pub hit_wall: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Board {
    /// The deck's middle (the skater's origin), local frame, metres.
    pub pos: Vec3,
    pub vel: Vec3,
    /// The nose's direction over the ground, radians from east turning north.
    pub heading: f32,
    /// The deck's up.
    pub up: Vec3,
    pub phase: Phase,
    pub air_time: f32,
    pub bail_time: f32,
    /// Skate 3's steering state: the damped stick, the push scalar, the deck's tilt (radians).
    damped: f32,
    push_scalar: f32,
    pub tilt: f32,
    /// Seconds since the last input.
    idle: f32,
    /// The powerslide's turn of the board across its path now, degrees (signed).
    pub slide: f32,
    /// Riding backwards.
    pub fakie: bool,
}

impl Board {
    pub fn at(pos: Vec3, heading: f32) -> Board {
        Board { pos, vel: Vec3::ZERO, heading, up: Vec3::Z, phase: Phase::Ground, air_time: 0.0, bail_time: 0.0, damped: 0.0, push_scalar: 1.0, tilt: 0.0, idle: 0.0, slide: 0.0, fakie: false }
    }

    pub fn nose(&self) -> Vec3 {
        Vec3::new(self.heading.cos(), self.heading.sin(), 0.0)
    }

    /// The nose along the deck (on its slope).
    pub fn forward(&self) -> Vec3 {
        let f = self.nose();
        (f - self.up * f.dot(self.up)).normalize_or(f)
    }

    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    /// The board's turn in the local frame: its columns are Skate 3's model axes (x the board's left, y up, z the nose).
    pub fn rotation(&self, extra_yaw_deg: f32) -> Mat3 {
        let up = self.up.normalize_or(Vec3::Z);
        let h = self.heading + extra_yaw_deg.to_radians();
        let f0 = Vec3::new(h.cos(), h.sin(), 0.0);
        let fwd = (f0 - up * f0.dot(up)).normalize_or(f0);
        let left = up.cross(fwd).normalize_or(Vec3::Y);
        Mat3::from_cols(left, up, fwd)
    }

    pub fn ride_height(t: &Tune) -> f32 {
        t.wheel_radius + DECK_OVER_AXLE
    }

    /// The ground under the board: its point under the deck's middle and its normal, from the four wheels (at least three
    /// must find it within `reach` under their axles).
    fn ground(&self, world: &SandboxWorld, at: Vec3, t: &Tune, reach: f32) -> Option<(Vec3, Vec3)> {
        let f = self.nose();
        let left = Vec3::Z.cross(f);
        let lift = 0.5;
        let mut pts = [None; 4];
        for (k, (a, s)) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)].into_iter().enumerate() {
            let w = at + f * (a * WHEEL_HALF_BASE) + left * (s * WHEEL_HALF_TRACK);
            if let Some(hit) = world.ray(w + Vec3::Z * lift, -Vec3::Z, lift + Self::ride_height(t) + reach, None) {
                // A prop's top counts only when it's flat enough to ride on.
                if !matches!(hit.met, Met::Prop(_)) || hit.normal.z > 0.7 {
                    pts[k] = Some(hit.point);
                }
            }
        }
        let found: Vec<Vec3> = pts.iter().flatten().copied().collect();
        if found.len() < 3 {
            return None;
        }
        let avg = |a: Option<Vec3>, b: Option<Vec3>| match (a, b) {
            (Some(a), Some(b)) => Some((a + b) * 0.5),
            (Some(a), None) | (None, Some(a)) => Some(a),
            _ => None,
        };
        let (front, back, lft, rgt) = (avg(pts[0], pts[1]), avg(pts[2], pts[3]), avg(pts[0], pts[2]), avg(pts[1], pts[3]));
        let n = match (front, back, lft, rgt) {
            (Some(fr), Some(bk), Some(l), Some(r)) => (fr - bk).cross(l - r).normalize_or(Vec3::Z),
            _ => Vec3::Z,
        };
        let n = if n.z < 0.0 { -n } else { n };
        let mid = found.iter().copied().sum::<Vec3>() / found.len() as f32;
        // The point under the deck's middle on that plane.
        let under = at - n * (at - mid).dot(n);
        Some((under, n))
    }

    /// Where the board would stand on the ground at a point (its deck's middle), if there's ground there.
    pub fn stand_on(&mut self, world: &SandboxWorld, t: &Tune, at: Vec3) -> bool {
        match self.ground(world, at, t, 3.0) {
            Some((under, n)) => {
                self.pos = under + n * Self::ride_height(t);
                self.up = n;
                true
            }
            None => false,
        }
    }

    /// One tick.
    pub fn step(&mut self, t: &Tune, world: &mut SandboxWorld, d: &Drive, gravity: f32, dt: f32) -> Happened {
        let mut out = Happened::default();
        self.idle = if d.any { 0.0 } else { self.idle + dt };
        match self.phase {
            Phase::Ground => self.ground_step(t, world, d, gravity, dt, &mut out),
            Phase::Air => self.air_step(t, world, d, gravity, dt, &mut out),
            Phase::Bail => self.bail_step(t, world, gravity, dt),
        }
        out
    }

    fn ground_step(&mut self, t: &Tune, world: &mut SandboxWorld, d: &Drive, g: f32, dt: f32, out: &mut Happened) {
        let speed = self.vel.length();
        let fwd = self.forward();
        let signed = self.vel.dot(fwd);
        // Riding backwards only once he really rolls that way (a creep back down a slope while standing isn't fakie).
        self.fakie = signed < -0.4 || (self.fakie && signed < 0.0);
        let along = if self.fakie { -fwd } else { fwd };

        // Steering: Skate 3's deck tilt, then the trucks' turn.
        self.tilt = steering::calculate_tilt(
            &t.steering,
            steering::SteeringInput {
                turn: d.steer,
                hard_turn: 0.0,
                absolute_body_speed: speed,
                flipped_controls_scalar: 1.0,
                balance: 0.0,
                truck_tightness: 0.5,
                pushing: d.pushing,
            },
            Some(&mut self.push_scalar),
            Some(&mut self.damped),
        );
        let wheelbase = 2.0 * WHEEL_HALF_BASE;
        let curvature = 2.0 * self.tilt.tan() * t.truck_axis.sin() / wheelbase;
        let grip_rate = GRIP / speed.max(0.1);
        let mut yaw_rate = (-curvature * speed * if self.fakie { -1.0 } else { 1.0 }).clamp(-grip_rate, grip_rate);
        if speed < KICKTURN_SPEED {
            yaw_rate = -d.steer * KICKTURN_RATE;
        }
        // The powerslide: the board turns across its path and scrubs speed; the path keeps its line.
        let target_slide = if speed > 1.0 { d.slide * SLIDE_ANGLE } else { 0.0 };
        self.slide += (target_slide - self.slide) * (1.0 - (-dt * 10.0).exp());
        let sliding = self.slide.abs() > 30.0;
        let turn = if sliding { 0.0 } else { yaw_rate * dt };
        self.heading += turn;
        self.vel = Quat::from_axis_angle(self.up, turn) * self.vel;

        // Along the path: slope, push, brake, friction.
        let mut gain = (-Vec3::Z * g).dot(along).clamp(-t.max_gravity_accel, t.max_gravity_accel) * dt;
        if d.push && !self.fakie {
            let a = push::calculate_acceleration(
                push::PushInput {
                    flags_2468: 0x0200_0000,
                    flags_2472: 0,
                    target_speed: t.max_push_speed,
                    current_speed: speed,
                    absolute_body_speed: speed,
                    scale: 1.0,
                    delta_seconds: dt,
                    direction: Vector3::new(1.0, 0.0, 0.0),
                },
                push::PushLimits { maximum_pushable_speed: t.max_push_speed, low_speed_change: t.push_dv[0], high_speed_change: t.push_dv[1] },
            );
            gain += a.vector.x * dt;
        }
        let mut loss = t.friction.evaluate(speed);
        if self.idle > t.no_input_time {
            loss += t.no_input_friction.evaluate(speed);
        }
        if sliding {
            loss += SLIDE_DECEL;
        }
        if d.brake {
            let f = braking::calculate_braking(
                braking::BrakeInput { flags_2468: 0x4000_0000, input_2728: 1.0, signed_speed: speed, absolute_body_speed: speed, surface_factor: 1.0, direction: Vector3::new(1.0, 0.0, 0.0) },
                t.brake,
            );
            loss += -f.force_world.x / t.board_mass;
        }
        // Friction and the brake take speed away down to standing still; the slope can roll him backwards (fakie).
        let along_speed = self.vel.dot(along) + gain;
        let stop = loss * dt;
        let new_along = if along_speed > stop { along_speed - stop } else if along_speed < -stop { along_speed + stop } else { 0.0 };
        // Sideways: the wheels hold, except in a slide.
        let side_dir = self.up.cross(along).normalize_or(Vec3::Y);
        let hold = if sliding { 1.5 } else { 25.0 };
        let side = self.vel.dot(side_dir) * (-hold * dt).exp();
        self.vel = along * new_along + side_dir * side + self.up * self.vel.dot(self.up);

        // Pop.
        if let Some(strength) = d.pop {
            self.vel = self.jump_velocity(t, strength, d.hips_m, g);
            self.phase = Phase::Air;
            self.air_time = 0.0;
            out.popped = true;
            self.move_body(world, dt, out);
            return;
        }

        // Move, then keep to the ground or leave it.
        self.move_body(world, dt, out);
        if self.phase == Phase::Bail {
            return;
        }
        let ride = Self::ride_height(t);
        match self.ground(world, self.pos, t, 0.35) {
            Some((under, n)) => {
                let over = (self.pos - under).dot(n) - ride;
                if over > LEAVE_M && self.vel.dot(n) > -0.5 {
                    // The ground fell away under its path: off a lip or a kerb.
                    self.phase = Phase::Air;
                    self.air_time = 0.0;
                } else {
                    self.pos = under + n * ride;
                    self.up = self.up.lerp(n, 1.0 - (-dt * 20.0).exp()).normalize_or(n);
                    let vn = self.vel.dot(n);
                    self.vel -= n * vn;
                    out.rolling = speed > 0.2;
                }
            }
            None => {
                self.phase = Phase::Air;
                self.air_time = 0.0;
            }
        }
    }

    /// Skate 3's launch, in its frame (y up): our frame's x, z, -y.
    pub(crate) fn jump_velocity(&self, t: &Tune, strength: f32, hips_m: f32, g: f32) -> Vec3 {
        let to = |v: Vec3| [v.x, v.z, -v.y, 0.0];
        let from = |s: [f32; 4]| Vec3::new(s[0], -s[2], s[1]);
        let n = self.up.normalize_or(Vec3::Z);
        let f = self.forward();
        let r = ground_jump::calculate(
            GroundJumpInput {
                flags_2468: 0x0040_0000,
                flags_2480: 0,
                flags_2484: 0,
                flags_2488: 0,
                effective_forward: to(f),
                forward: to(f),
                current_velocity: to(self.vel),
                ground_reference_position: [0.0; 4],
                reference_up: [0.0, 1.0, 0.0, 0.0],
                filtered_ground_normal: to(n),
                animation_com_position: [0.0, hips_m, 0.0, 0.0],
                prepared_velocity: to(self.vel),
                jump_strength: strength.clamp(0.0, 1.0),
                jump_controls: [0.0, 0.0],
                gravity_y: -g,
                surface_speed: self.vel.length(),
            },
            t.jump_mode,
            &t.jump,
        );
        let v = from(r.velocity);
        if v.is_finite() { v } else { self.vel + n * 3.0 }
    }

    fn air_step(&mut self, t: &Tune, world: &mut SandboxWorld, d: &Drive, g: f32, dt: f32, out: &mut Happened) {
        self.air_time += dt;
        self.vel.z -= g * dt;
        self.heading += -d.steer * AIR_SPIN * dt;
        self.up = self.up.lerp(Vec3::Z, 1.0 - (-dt * 4.0).exp()).normalize_or(Vec3::Z);
        self.slide *= (-dt * 8.0).exp();
        self.move_body(world, dt, out);
        if self.phase == Phase::Bail || self.vel.z > 0.0 {
            return;
        }
        let ride = Self::ride_height(t);
        if let Some((under, n)) = self.ground(world, self.pos, t, 0.05) {
            if (self.pos - under).dot(n) - ride <= 0.02 {
                let down = -self.vel.dot(n);
                self.pos = under + n * ride;
                self.up = n;
                self.vel -= n * self.vel.dot(n);
                self.vel *= 0.95;
                self.phase = Phase::Ground;
                out.landed = Some(down);
                // Landing too hard, or with the board across the path: a bail. (Backwards is fine: fakie.)
                let path = Vec2::new(self.vel.x, self.vel.y);
                let across = path.length() > 1.5 && path.normalize().dot(Vec2::new(self.heading.cos(), self.heading.sin())).abs() < 0.55;
                if down > LAND_BAIL || across {
                    self.bail(out);
                }
            }
        }
    }

    fn bail_step(&mut self, t: &Tune, world: &mut SandboxWorld, g: f32, dt: f32) {
        self.bail_time += dt;
        let mut ignore = Happened::default();
        self.vel.z -= g * dt;
        self.move_body(world, dt, &mut ignore);
        self.phase = Phase::Bail;
        let ride = Self::ride_height(t);
        if let Some((under, n)) = self.ground(world, self.pos, t, 0.2) {
            if (self.pos - under).dot(n) - ride <= 0.02 {
                self.pos = under + n * ride;
                self.up = n;
                let vn = self.vel.dot(n).min(0.0);
                self.vel -= n * vn;
                // Tumbling along the ground stops him quickly.
                let s = self.vel.length();
                if s > 0.0 {
                    self.vel *= (s - 7.0 * dt).max(0.0) / s;
                }
            }
        }
    }

    pub fn bail(&mut self, out: &mut Happened) {
        self.phase = Phase::Bail;
        self.bail_time = 0.0;
        self.slide = 0.0;
        out.bailed = true;
    }

    /// Back on the board after a bail: standing still where he fell.
    pub fn recover(&mut self) {
        self.phase = Phase::Ground;
        self.vel = Vec3::ZERO;
        self.bail_time = 0.0;
        self.damped = 0.0;
        self.tilt = 0.0;
        self.fakie = false;
    }

    /// The move of a tick against walls, trees and props: stopped by what stands in the way (a bail if hit hard), props
    /// shoved.
    fn move_body(&mut self, world: &mut SandboxWorld, dt: f32, out: &mut Happened) {
        let step = self.vel * dt;
        if step.length_squared() < 1e-10 {
            return;
        }
        let centre = self.pos + Vec3::Z * BODY_UP;
        let Some(hit) = world.sweep_box(centre, centre + step, BODY_HALF) else {
            self.pos += step;
            return;
        };
        // The ground under the wheels is the ground step's business; only what stands up counts here.
        if hit.normal.z > 0.6 {
            self.pos += step;
            return;
        }
        let n = Vec3::new(hit.normal.x, hit.normal.y, 0.0).normalize_or(Vec3::X);
        let into = -self.vel.dot(n);
        if let Met::Prop(id) = hit.met {
            if let (Some((c, _)), Some(mass)) = (world.motion(id), world.prop(id).map(|p| p.mass)) {
                let share = (80.0 / (80.0 + mass)).min(1.0);
                world.impulse(id, -n * (into.max(0.0) * mass * share * 1.2), c);
                if mass < 40.0 {
                    // Light props are knocked aside.
                    self.vel *= 0.9;
                    self.pos += step;
                    return;
                }
            }
        }
        self.pos += step * (hit.fraction - 0.02).max(0.0);
        if into > 0.0 {
            self.vel += n * into;
            out.hit_wall = Some(into);
            if into > WALL_BAIL && self.phase != Phase::Bail {
                self.vel += n * into * 0.3;
                self.bail(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune() -> Tune {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/skate3/tuning.cfg");
        Tune::read(&Tuning::parse(&std::fs::read_to_string(path).unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn skate_3s_numbers_make_a_sensible_board() {
        let t = tune();
        assert!((t.board_mass - 11.56).abs() < 0.01, "{}", t.board_mass);
        // A push from standing gives at most MaxPushDVStart.
        let a = push::calculate_acceleration(
            push::PushInput { flags_2468: 0x0200_0000, flags_2472: 0, target_speed: t.max_push_speed, current_speed: 0.0, absolute_body_speed: 0.0, scale: 1.0, delta_seconds: 0.015, direction: Vector3::new(1.0, 0.0, 0.0) },
            push::PushLimits { maximum_pushable_speed: t.max_push_speed, low_speed_change: t.push_dv[0], high_speed_change: t.push_dv[1] },
        );
        assert!((a.vector.x * 0.015 - 0.6).abs() < 1e-4);
        // The ollie from flat ground at a jog lifts him by about half a metre to a metre, and keeps his speed.
        let b = Board { vel: Vec3::new(3.0, 0.0, 0.0), ..Board::at(Vec3::ZERO, 0.0) };
        for strength in [0.0, 1.0] {
            let v = b.jump_velocity(&t, strength, 0.75, 9.81);
            let rise = v.z * v.z / (2.0 * 9.81);
            assert!((0.3..1.2).contains(&rise), "strength {strength}: rise {rise} m, {v:?}");
            assert!((v.x - 3.0).abs() < 0.5, "keeps its speed: {v:?}");
            assert!(v.y.abs() < 0.3, "no sideways: {v:?}");
        }
        // Steering at walking pace is a tight turn, at speed a wide one.
        let tilt = |speed: f32| {
            let mut damped = 1.0;
            steering::calculate_tilt(&t.steering, steering::SteeringInput { turn: 1.0, hard_turn: 0.0, absolute_body_speed: speed, flipped_controls_scalar: 1.0, balance: 0.0, truck_tightness: 0.5, pushing: false }, None, Some(&mut damped))
        };
        let radius = |speed: f32| 2.0 * WHEEL_HALF_BASE / (2.0 * tilt(speed).tan() * t.truck_axis.sin());
        assert!(radius(2.0) < radius(12.0), "{} {}", radius(2.0), radius(12.0));
        assert!((0.5..6.0).contains(&radius(2.0)), "{}", radius(2.0));
    }

    /// The weakest pop (a quick flick) on flat ground stays up long enough for every trick to be caught.
    #[test]
    fn every_trick_is_caught_from_the_weakest_pop() {
        use super::super::anim::{self, POP_FLOOR};
        use super::super::input::Trick;
        let t = tune();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/skate3");
        let clips = se_core::skate3::parse_clips(&std::fs::read(dir.join("clips.s3c")).unwrap()).unwrap();
        let index = anim::ClipIndex::new(clips);
        let len = |n: &str| index.clips.get(n).unwrap().seconds();
        // The hips over the deck while rolling, from the riding clip.
        let mut pose = vec![[0.0; 10]; index.bones()];
        index.sample(index.find("R_IDLE_RIDE_N_0_CYC").unwrap(), 0.0, &mut pose);
        let hips = anim::hips_height(&pose, &[]);
        for speed in [2.0, 5.0, 8.5] {
            let b = Board { vel: Vec3::new(speed, 0.0, 0.0), ..Board::at(Vec3::ZERO, 0.0) };
            let v = b.jump_velocity(&t, POP_FLOOR, hips, 9.81);
            let air = 2.0 * v.z / 9.81;
            for trick in Trick::ALL {
                let caught = len("OLLIE_LOW_G") + anim::trick_clips(trick).map_or(0.0, |c| c.cyc_seconds + anim::caught_after(len(c.out)));
                println!("{trick:?} at {speed} m/s: caught at {caught:.2} s of {air:.2} s in the air");
                assert!(caught + 0.03 < air, "{trick:?} at {speed} m/s: caught at {caught:.2} s, in the air {air:.2} s");
            }
        }
    }
}
