//! The sandbox player's movement: Half-Life 2's, as Garry's Mod runs it. This is Valve's `gamemovement.cpp` (Source SDK
//! 2013, `CGameMovement`: `FullWalkMove`, `WalkMove`, `AirMove`, `Friction`, `Accelerate`, `AirAccelerate`, `TryPlayerMove`,
//! `StepMove`, `StayOnGround`, `CategorizePosition`, `CheckJumpButton`, `Duck`) rewritten, not copied, with the numbers Garry's Mod's
//! sandbox gamemode sets (`player_sandbox.lua`, `player_default.lua`) and its server's convars.
//!
//! Everything here is in Source's units (1 unit = 0.75 inch, `UNIT_M` metres) and axes (x east, y north, z up), in the sandbox's
//! local frame, so the numbers read as they do in the SDK. It knows nothing of planets: the world is a [`MoveWorld`] that answers
//! hull traces, and gravity is a number handed in each tick (the planet's own, `Vars::gravity`).

use bevy::math::Vec3;

/// One Source unit in metres: Valve's world scale, 16 units to the foot. Props keep their real sizes at it (an oil drum is 0.86 m).
pub const UNIT_M: f32 = 0.01905;

/// The server's tick: Garry's Mod's default, 66.67 a second (`-tickrate 66`). Movement and the props' physics step by it whatever
/// the frame rate, as in the game (air strafing and friction depend on the step).
pub const TICK: f32 = 0.015;

/// `VEC_HULL_MIN/MAX`, `VEC_DUCK_HULL_MAX`, `VEC_VIEW`, `VEC_DUCK_VIEW` (shareddefs / hl2 gamerules): a box 32 wide, 72 tall
/// (36 ducked), the eyes 64 up (28 ducked).
pub const HULL_HALF: f32 = 16.0;
pub const HULL_HEIGHT: f32 = 72.0;
pub const HULL_HEIGHT_DUCK: f32 = 36.0;
pub const VIEW_Z: f32 = 64.0;
pub const VIEW_Z_DUCK: f32 = 28.0;

/// A slope's normal must point at least this much up to be stood on (`CategorizePosition`: `plane.normal[2] < 0.7` is not ground).
pub const WALKABLE_Z: f32 = 0.7;
/// Rising faster than this he is not on the ground whatever is under him (`NON_JUMP_VELOCITY`).
const NON_JUMP_VELOCITY: f32 = 140.0;
/// `AirAccelerate`'s cap on the wish speed (`GetAirSpeedCap`): what makes air strafing what it is.
const AIR_SPEED_CAP: f32 = 30.0;
/// `DIST_EPSILON`: how far a trace stops short of what it hits.
pub const DIST_EPSILON: f32 = 0.031_25;
const MAX_CLIP_PLANES: usize = 5;
/// `PLAYER_FALL_PUNCH_THRESHOLD` (Half-Life 2's): landing faster than this knocks the view (`CheckFalling`).
pub const FALL_PUNCH_SPEED: f32 = 303.0;
/// `PLAYER_MAX_SAFE_FALL_SPEED` (Half-Life 2's): the speed of a fall that starts to hurt.
pub const SAFE_FALL_SPEED: f32 = 526.5;

/// The convars and the player class's numbers.
#[derive(Clone, Copy, Debug)]
pub struct Vars {
    /// The planet's own gravity at the player, units/s^2 (Source's `sv_gravity` is 600: 11.4 m/s^2).
    pub gravity: f32,
    /// `sv_accelerate` (10), `sv_airaccelerate` (10), `sv_friction` (8 in Garry's Mod, 4 in Half-Life 2), `sv_stopspeed`
    /// (10 in Garry's Mod, 100 in Half-Life 2), `sv_stepsize` (18), `sv_bounce` (0).
    pub accelerate: f32,
    pub air_accelerate: f32,
    pub friction: f32,
    pub stop_speed: f32,
    pub step_size: f32,
    /// `player_sandbox.lua`: WalkSpeed 200, RunSpeed 400, SlowWalkSpeed 100 (Alt), DuckSpeed and UnDuckSpeed 0.1 s; from
    /// `player_default.lua`: CrouchedWalkSpeed 0.3 (a share of the walk), JumpPower 200.
    pub walk_speed: f32,
    pub run_speed: f32,
    pub slow_speed: f32,
    pub crouch_share: f32,
    pub jump_power: f32,
    pub duck_time: f32,
    pub unduck_time: f32,
}

impl Default for Vars {
    fn default() -> Self {
        Vars {
            gravity: 600.0,
            accelerate: 10.0,
            air_accelerate: 10.0,
            friction: 8.0,
            stop_speed: 10.0,
            step_size: 18.0,
            walk_speed: 200.0,
            run_speed: 400.0,
            slow_speed: 100.0,
            crouch_share: 0.3,
            jump_power: 200.0,
            duck_time: 0.1,
            unduck_time: 0.1,
        }
    }
}

/// The keys of one tick (a `CUserCmd`): `forward` and `side` are -1..1, the view's yaw in radians (0 looks east, turning left
/// grows it, as Source's).
#[derive(Clone, Copy, Debug, Default)]
pub struct Cmd {
    pub forward: f32,
    pub side: f32,
    pub yaw: f32,
    pub jump: bool,
    pub duck: bool,
    pub sprint: bool,
    pub walk: bool,
}

/// What a hull trace hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// The ground or anything else that never moves.
    World,
    /// A prop (its id in the sandbox's world) and its velocity where it was hit, units/s.
    Prop(u32, Vec3),
}

/// A hull trace's answer (`trace_t`).
#[derive(Clone, Copy, Debug)]
pub struct Trace {
    /// How much of the way was free, 0..1.
    pub fraction: f32,
    /// Where the hull's origin (the middle of its bottom) ends.
    pub end: Vec3,
    /// The surface's normal where it stopped (zero when nothing was hit).
    pub normal: Vec3,
    /// The hull began inside something.
    pub start_solid: bool,
    pub hit: Option<Hit>,
}

impl Trace {
    pub fn clear(end: Vec3) -> Self {
        Trace { fraction: 1.0, end, normal: Vec3::ZERO, start_solid: false, hit: None }
    }
}

/// The world the player moves in.
pub trait MoveWorld {
    /// Sweep the player's box (its origin the middle of its bottom, standing or ducked) from `start` to `end`.
    fn trace(&self, start: Vec3, end: Vec3, ducked: bool) -> Trace;
    /// The player walked or fell into a prop: push it (`velocity` his, units/s, `normal` the surface's).
    fn touch(&mut self, _prop: u32, _at: Vec3, _velocity: Vec3, _normal: Vec3) {}
}

/// What happened in a tick, for the sounds and the view.
#[derive(Clone, Copy, Debug, Default)]
pub struct Events {
    pub jumped: bool,
    /// Landed this tick, at this speed down (units/s).
    pub landed: Option<f32>,
}

/// The player (`CBasePlayer`'s movement state).
#[derive(Clone, Debug)]
pub struct Player {
    /// The middle of the bottom of his box.
    pub origin: Vec3,
    pub velocity: Vec3,
    /// What he stands on (`GetGroundEntity`), and its surface's normal.
    pub ground: Option<Hit>,
    pub ground_normal: Vec3,
    /// The box is the ducked one (`FL_DUCKING`).
    pub ducked: bool,
    /// On the way down or up (`m_bDucking`), and how far: 0 standing, 1 ducked (the view's height follows it).
    pub ducking: bool,
    pub duck_amount: f32,
    /// Jump was down last tick (`IN_JUMP` in `m_nOldButtons`): holding it doesn't hop.
    old_jump: bool,
    /// `m_surfaceFriction`: 1, or 0.25 just after leaving the ground upwards on a slope.
    surface_friction: f32,
    /// How fast he was falling (`m_flFallVelocity`).
    pub fall_velocity: f32,
    /// The steps' height smoothed out of the view (`m_outStepHeight`): what the eyes still have to rise.
    pub step_smooth: f32,
}

impl Player {
    pub fn at(origin: Vec3) -> Self {
        Player {
            origin,
            velocity: Vec3::ZERO,
            ground: None,
            ground_normal: Vec3::Z,
            ducked: false,
            ducking: false,
            duck_amount: 0.0,
            old_jump: false,
            surface_friction: 1.0,
            fall_velocity: 0.0,
            step_smooth: 0.0,
        }
    }

    pub fn on_ground(&self) -> bool {
        self.ground.is_some()
    }

    /// The eyes' height over the origin, on the way between standing and ducked.
    pub fn view_z(&self) -> f32 {
        // SimpleSpline, as `SetDuckedEyeOffset` eases it.
        let t = self.duck_amount.clamp(0.0, 1.0);
        let s = 3.0 * t * t - 2.0 * t * t * t;
        VIEW_Z + (VIEW_Z_DUCK - VIEW_Z) * s
    }

    /// The speed his keys ask for (`m_flMaxSpeed`, set by the player class each tick, then `HandleDuckingSpeedCrop`).
    pub fn max_speed(&self, cmd: &Cmd, v: &Vars) -> f32 {
        let base = if cmd.sprint && !self.ducked {
            v.run_speed
        } else if cmd.walk {
            v.slow_speed
        } else {
            v.walk_speed
        };
        if self.ducked && self.on_ground() { v.walk_speed * v.crouch_share } else { base }
    }
}

/// `ClipVelocity`: slide `v` along a surface.
fn clip_velocity(v: Vec3, normal: Vec3, overbounce: f32) -> Vec3 {
    let mut out = v - normal * (v.dot(normal) * overbounce);
    // Once more, so rounding never leaves it moving into the plane.
    let adjust = out.dot(normal);
    if adjust < 0.0 {
        out -= normal * adjust;
    }
    out
}

/// One tick of `dt` seconds (`CGameMovement::PlayerMove` for a walking player).
pub fn player_move(p: &mut Player, cmd: &Cmd, v: &Vars, world: &mut dyn MoveWorld, dt: f32) -> Events {
    let mut ev = Events::default();
    // Shot upwards (an explosion, a prop under him): off the ground at once.
    if p.velocity.z > 250.0 {
        p.ground = None;
    }
    let was_on_ground = p.on_ground();
    duck(p, cmd, v, world, dt);
    let max_speed = p.max_speed(cmd, v);

    // FullWalkMove.
    start_gravity(p, v, dt);
    if cmd.jump {
        if check_jump(p, v, dt) {
            ev.jumped = true;
        }
    } else {
        p.old_jump = false;
    }
    if p.on_ground() {
        p.velocity.z = 0.0;
        friction(p, v, dt);
    }
    let (sin, cos) = cmd.yaw.sin_cos();
    let forward = Vec3::new(cos, sin, 0.0);
    let right = Vec3::new(sin, -cos, 0.0);
    let wish_vel = forward * (cmd.forward * max_speed) + right * (cmd.side * max_speed);
    let mut wish_speed = wish_vel.length();
    let wish_dir = if wish_speed > 0.0 { wish_vel / wish_speed } else { Vec3::ZERO };
    wish_speed = wish_speed.min(max_speed);
    if p.on_ground() {
        walk_move(p, v, world, wish_dir, wish_speed, dt);
    } else {
        air_move(p, v, world, wish_dir, wish_speed, dt);
    }
    categorize_position(p, world);
    finish_gravity(p, v, dt);
    if p.on_ground() {
        p.velocity.z = 0.0;
    }
    // CheckFalling.
    if p.on_ground() && !was_on_ground && !ev.jumped {
        ev.landed = Some(p.fall_velocity);
    }
    if p.on_ground() {
        p.fall_velocity = 0.0;
    } else {
        p.fall_velocity = -p.velocity.z;
    }
    // The eyes rise over a step instead of jumping (`CBasePlayer::SmoothViewOnStairs`' share of it).
    p.step_smooth *= (-dt * 14.0).exp();
    if p.step_smooth.abs() < 0.01 {
        p.step_smooth = 0.0;
    }
    ev
}

fn start_gravity(p: &mut Player, v: &Vars, dt: f32) {
    p.velocity.z -= v.gravity * 0.5 * dt;
}

fn finish_gravity(p: &mut Player, v: &Vars, dt: f32) {
    p.velocity.z -= v.gravity * 0.5 * dt;
}

/// `CheckJumpButton`: Garry's Mod jumps at the player's JumpPower whatever the gravity (Half-Life 2 alone: 160).
fn check_jump(p: &mut Player, v: &Vars, dt: f32) -> bool {
    if !p.on_ground() {
        // In the air: holding it must not jump on landing.
        p.old_jump = true;
        return false;
    }
    if p.old_jump {
        return false;
    }
    // Not while the box is changing size.
    if p.ducking && p.ducked {
        return false;
    }
    // What he stood on goes with him (a prop moving up).
    if let Some(Hit::Prop(_, pv)) = p.ground {
        p.velocity.x += pv.x;
        p.velocity.y += pv.y;
    }
    p.ground = None;
    if p.ducking || p.ducked {
        p.velocity.z = v.jump_power;
    } else {
        p.velocity.z += v.jump_power;
    }
    finish_gravity(p, v, dt);
    p.old_jump = true;
    true
}

/// `Friction`: the ground's drag, stronger below `sv_stopspeed`.
fn friction(p: &mut Player, v: &Vars, dt: f32) {
    let speed = p.velocity.length();
    if speed < 0.1 {
        return;
    }
    let control = speed.max(v.stop_speed);
    let drop = control * v.friction * p.surface_friction * dt;
    let new_speed = (speed - drop).max(0.0);
    if new_speed != speed {
        p.velocity *= new_speed / speed;
    }
}

/// `Accelerate`.
fn accelerate(p: &mut Player, wish_dir: Vec3, wish_speed: f32, accel: f32, dt: f32) {
    let add = wish_speed - p.velocity.dot(wish_dir);
    if add <= 0.0 {
        return;
    }
    let speed = (accel * dt * wish_speed * p.surface_friction).min(add);
    p.velocity += wish_dir * speed;
}

/// `AirAccelerate`: the same with the wish speed capped at 30, but the gain worked out from the uncapped one.
fn air_accelerate(p: &mut Player, wish_dir: Vec3, wish_speed: f32, accel: f32, dt: f32) {
    let add = wish_speed.min(AIR_SPEED_CAP) - p.velocity.dot(wish_dir);
    if add <= 0.0 {
        return;
    }
    let speed = (accel * wish_speed * dt * p.surface_friction).min(add);
    p.velocity += wish_dir * speed;
}

/// What he stands on carries him (`m_vecBaseVelocity`).
fn base_velocity(p: &Player) -> Vec3 {
    match p.ground {
        Some(Hit::Prop(_, pv)) => Vec3::new(pv.x, pv.y, 0.0),
        _ => Vec3::ZERO,
    }
}

fn walk_move(p: &mut Player, v: &Vars, world: &mut dyn MoveWorld, wish_dir: Vec3, wish_speed: f32, dt: f32) {
    p.velocity.z = 0.0;
    accelerate(p, wish_dir, wish_speed, v.accelerate, dt);
    p.velocity.z = 0.0;
    let base = base_velocity(p);
    p.velocity += base;
    if p.velocity.length() < 1.0 {
        p.velocity = Vec3::ZERO;
        return;
    }
    // First just try going there.
    let mut dest = p.origin + p.velocity * dt;
    dest.z = p.origin.z;
    let first = world.trace(p.origin, dest, p.ducked);
    if first.fraction == 1.0 {
        p.origin = first.end;
        p.velocity -= base;
        stay_on_ground(p, v, world);
        return;
    }
    step_move(p, v, world, dt);
    p.velocity -= base;
    stay_on_ground(p, v, world);
}

fn air_move(p: &mut Player, v: &Vars, world: &mut dyn MoveWorld, wish_dir: Vec3, wish_speed: f32, dt: f32) {
    air_accelerate(p, wish_dir, wish_speed, v.air_accelerate, dt);
    try_player_move(p, world, dt);
}

/// `StepMove`: slide along the ground, and slide a step higher, and keep whichever got further.
fn step_move(p: &mut Player, v: &Vars, world: &mut dyn MoveWorld, dt: f32) {
    let (start, start_vel) = (p.origin, p.velocity);
    try_player_move(p, world, dt);
    let (down_pos, down_vel) = (p.origin, p.velocity);
    p.origin = start;
    p.velocity = start_vel;
    // Up a step.
    let up = world.trace(p.origin, p.origin + Vec3::Z * (v.step_size + DIST_EPSILON), p.ducked);
    if !up.start_solid {
        p.origin = up.end;
    }
    try_player_move(p, world, dt);
    // And down again.
    let down = world.trace(p.origin, p.origin - Vec3::Z * (v.step_size + DIST_EPSILON), p.ducked);
    if down.normal.z < WALKABLE_Z {
        // Nothing to stand on up there: the first try stands.
        p.origin = down_pos;
        p.velocity = down_vel;
        return;
    }
    if !down.start_solid {
        p.origin = down.end;
    }
    let flat = |a: Vec3| (a.x - start.x).powi(2) + (a.y - start.y).powi(2);
    if flat(down_pos) > flat(p.origin) {
        p.origin = down_pos;
        p.velocity = down_vel;
    } else {
        p.velocity.z = down_vel.z;
    }
    let step = p.origin.z - start.z;
    if step > 0.0 {
        p.step_smooth += step;
    }
}

/// `TryPlayerMove`: move by the velocity, sliding along up to four surfaces.
fn try_player_move(p: &mut Player, world: &mut dyn MoveWorld, dt: f32) {
    let primal = p.velocity;
    let mut original = p.velocity;
    let mut planes: [Vec3; MAX_CLIP_PLANES] = [Vec3::ZERO; MAX_CLIP_PLANES];
    let mut count = 0;
    let mut all_fraction = 0.0;
    let mut time_left = dt;
    for _ in 0..4 {
        if p.velocity == Vec3::ZERO {
            break;
        }
        let end = p.origin + p.velocity * time_left;
        let t = world.trace(p.origin, end, p.ducked);
        all_fraction += t.fraction;
        if t.start_solid && t.fraction == 0.0 {
            // Inside something: stopped (the sandbox's unstick lifts him out).
            p.velocity = Vec3::ZERO;
            return;
        }
        if t.fraction > 0.0 {
            p.origin = t.end;
            original = p.velocity;
            count = 0;
        }
        if t.fraction == 1.0 {
            break;
        }
        if let Some(Hit::Prop(id, _)) = t.hit {
            world.touch(id, t.end, p.velocity, t.normal);
        }
        time_left -= time_left * t.fraction;
        if count >= MAX_CLIP_PLANES {
            p.velocity = Vec3::ZERO;
            break;
        }
        planes[count] = t.normal;
        count += 1;
        if count == 1 && !p.on_ground() {
            // In the air the first surface only turns him (floors and walls alike: `sv_bounce` is 0).
            let nv = clip_velocity(original, planes[0], 1.0);
            p.velocity = nv;
            original = nv;
        } else {
            let mut i = 0;
            while i < count {
                p.velocity = clip_velocity(original, planes[i], 1.0);
                if (0..count).all(|j| j == i || p.velocity.dot(planes[j]) >= 0.0) {
                    break;
                }
                i += 1;
            }
            if i == count {
                // Along the crease of two surfaces, or stopped in a corner of more.
                if count != 2 {
                    p.velocity = Vec3::ZERO;
                    break;
                }
                let dir = planes[0].cross(planes[1]).normalize_or_zero();
                p.velocity = dir * dir.dot(p.velocity);
            }
            // Turned right back: stop, or he shivers in sloping corners.
            if p.velocity.dot(primal) <= 0.0 {
                p.velocity = Vec3::ZERO;
                break;
            }
        }
    }
    if all_fraction == 0.0 {
        p.velocity = Vec3::ZERO;
    }
}

/// `StayOnGround`: keep the feet on the ground going down a slope or a step, instead of hopping off it.
fn stay_on_ground(p: &mut Player, v: &Vars, world: &mut dyn MoveWorld) {
    let up = world.trace(p.origin, p.origin + Vec3::Z * 2.0, p.ducked);
    let start = up.end;
    let end = p.origin - Vec3::Z * v.step_size;
    let t = world.trace(start, end, p.ducked);
    if t.fraction > 0.0 && t.fraction < 1.0 && !t.start_solid && t.normal.z >= WALKABLE_Z {
        let before = p.origin.z;
        p.origin = t.end;
        // (Down a step the eyes follow smoothly too.)
        p.step_smooth += p.origin.z - before;
    }
}

/// `CategorizePosition`: is there ground under the feet?
fn categorize_position(p: &mut Player, world: &mut dyn MoveWorld) {
    p.surface_friction = 1.0;
    let prop_z = match p.ground {
        Some(Hit::Prop(_, pv)) => pv.z,
        _ => 0.0,
    };
    if p.velocity.z - prop_z > NON_JUMP_VELOCITY {
        p.ground = None;
        return;
    }
    let t = world.trace(p.origin, p.origin - Vec3::Z * 2.0, p.ducked);
    match t.hit {
        Some(hit) if t.normal.z >= WALKABLE_Z => {
            let landing = p.ground.is_none();
            p.ground = Some(hit);
            p.ground_normal = t.normal;
            if !t.start_solid && t.fraction > 0.0 && t.fraction < 1.0 {
                p.origin = t.end;
            }
            if landing {
                // (`SetGroundEntity`: what he lands on stops his fall.)
                p.velocity.z = 0.0;
            }
        }
        _ => {
            p.ground = None;
            if p.velocity.z > 0.0 {
                p.surface_friction = 0.25;
            }
        }
    }
}

/// `Duck`: Ctrl. On the ground the view goes down over `duck_time` and the box shrinks at the end; in the air the box shrinks at
/// once and the feet come up (which is what gets him onto a crate); standing up needs room.
fn duck(p: &mut Player, cmd: &Cmd, v: &Vars, world: &mut dyn MoveWorld, dt: f32) {
    let lift = HULL_HEIGHT - HULL_HEIGHT_DUCK;
    if cmd.duck {
        if !p.ducked {
            if p.on_ground() {
                p.ducking = true;
                p.duck_amount += dt / v.duck_time.max(0.001);
                if p.duck_amount >= 1.0 {
                    p.duck_amount = 1.0;
                    p.ducked = true;
                    p.ducking = false;
                }
            } else {
                // In the air: at once, the feet pulled up under him.
                p.ducked = true;
                p.ducking = false;
                p.duck_amount = 1.0;
                p.origin.z += lift;
            }
        } else if p.duck_amount < 1.0 {
            p.duck_amount = (p.duck_amount + dt / v.duck_time.max(0.001)).min(1.0);
            p.ducking = p.duck_amount < 1.0;
        }
        return;
    }
    if !p.ducked && p.duck_amount <= 0.0 {
        p.ducking = false;
        return;
    }
    if p.ducked {
        // Room to stand? On the ground the box grows up; in the air the feet go back down.
        let from = if p.on_ground() { p.origin } else { p.origin - Vec3::Z * lift };
        let room = if p.on_ground() {
            let t = world.trace(from, from, false);
            !t.start_solid
        } else {
            let t = world.trace(p.origin, from, true);
            t.fraction == 1.0 && !world.trace(from, from, false).start_solid
        };
        if !room {
            return;
        }
        p.origin = from;
        p.ducked = false;
        p.ducking = true;
    }
    p.duck_amount -= dt / v.unduck_time.max(0.001);
    if p.duck_amount <= 0.0 {
        p.duck_amount = 0.0;
        p.ducking = false;
    }
}

/// `DecayPunchAngle` (`CBasePlayer`): the view's kick is a damped spring. Angles in degrees: x pitch (down is +), y yaw, z roll.
#[derive(Clone, Copy, Debug, Default)]
pub struct Punch {
    pub angle: Vec3,
    pub velocity: Vec3,
}

impl Punch {
    const DAMPING: f32 = 9.0;
    const SPRING: f32 = 65.0;

    /// `ViewPunch`.
    pub fn kick(&mut self, angle: Vec3) {
        self.velocity += angle * 20.0;
    }

    /// `ViewPunchReset`: the kick goes at once, unless (with a tolerance) it is already more than that.
    pub fn reset(&mut self, tolerance: f32) {
        if tolerance != 0.0 && self.velocity.length_squared() + self.angle.length_squared() > tolerance * tolerance {
            return;
        }
        *self = Punch::default();
    }

    /// `UTIL_ClipPunchAngleOffset`: a kick cut so the view's whole punch stays within `clip` (degrees) each way.
    pub fn clipped(&self, kick: Vec3, clip: Vec3) -> Vec3 {
        (kick + self.angle).clamp(-clip, clip) - self.angle
    }

    pub fn decay(&mut self, dt: f32) {
        if self.angle.length_squared() > 0.001 || self.velocity.length_squared() > 0.001 {
            self.angle += self.velocity * dt;
            self.velocity *= (1.0 - Self::DAMPING * dt).max(0.0);
            self.velocity -= self.angle * (Self::SPRING * dt).clamp(0.0, 2.0);
            self.angle = self.angle.clamp(Vec3::new(-89.0, -179.0, -89.0), Vec3::new(89.0, 179.0, 89.0));
        } else {
            *self = Punch::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground at z = 0 and, from x = 200 on, a platform `step` units high.
    struct Flat {
        step: f32,
    }

    impl Flat {
        fn floor_at(&self, x: f32) -> f32 {
            if x + HULL_HALF > 200.0 { self.step } else { 0.0 }
        }
    }

    impl MoveWorld for Flat {
        fn trace(&self, start: Vec3, end: Vec3, _ducked: bool) -> Trace {
            // Sampled along the way: fine enough for the tests' speeds.
            if start.z < self.floor_at(start.x) - 0.001 {
                return Trace { fraction: 0.0, end: start, normal: Vec3::Z, start_solid: true, hit: Some(Hit::World) };
            }
            let n = ((end - start).length() / 0.05).ceil().max(1.0) as usize;
            let mut last = 0.0;
            for i in 1..=n {
                let f = i as f32 / n as f32;
                let q = start.lerp(end, f);
                let floor = self.floor_at(q.x);
                if q.z < floor {
                    let before = start.lerp(end, last);
                    // The platform's side or a floor.
                    let wall = before.z < floor && self.floor_at(before.x) < floor;
                    let normal = if wall { -Vec3::X } else { Vec3::Z };
                    return Trace { fraction: last, end: before, normal, start_solid: false, hit: Some(Hit::World) };
                }
                last = f;
            }
            Trace::clear(end)
        }
    }

    fn run(p: &mut Player, cmd: &Cmd, v: &Vars, w: &mut Flat, secs: f32) -> Vec<Events> {
        (0..(secs / TICK).round() as usize).map(|_| player_move(p, cmd, v, w, TICK)).collect()
    }

    #[test]
    fn walks_runs_and_stops_at_garrys_mod_speeds() {
        let v = Vars::default();
        let mut w = Flat { step: 0.0 };
        let mut p = Player::at(Vec3::new(-5000.0, 0.0, 0.0));
        run(&mut p, &Cmd::default(), &v, &mut w, 0.1);
        assert!(p.on_ground());
        run(&mut p, &Cmd { forward: 1.0, ..Cmd::default() }, &v, &mut w, 2.0);
        assert!((p.velocity.length() - 200.0).abs() < 1.0, "walk {}", p.velocity.length());
        run(&mut p, &Cmd { forward: 1.0, sprint: true, ..Cmd::default() }, &v, &mut w, 2.0);
        assert!((p.velocity.length() - 400.0).abs() < 1.0, "run {}", p.velocity.length());
        run(&mut p, &Cmd { forward: 1.0, walk: true, ..Cmd::default() }, &v, &mut w, 2.0);
        assert!((p.velocity.length() - 100.0).abs() < 1.0, "slow {}", p.velocity.length());
        run(&mut p, &Cmd { forward: 1.0, duck: true, ..Cmd::default() }, &v, &mut w, 2.0);
        assert!((p.velocity.length() - 60.0).abs() < 1.0, "ducked {}", p.velocity.length());
        assert!(p.ducked && (p.view_z() - VIEW_Z_DUCK).abs() < 0.01);
        // Friction 8: from a run he stops in well under a second.
        run(&mut p, &Cmd { forward: 1.0, sprint: true, ..Cmd::default() }, &v, &mut w, 2.0);
        run(&mut p, &Cmd::default(), &v, &mut w, 0.7);
        assert!(p.velocity.length() < 1.0, "stop {}", p.velocity.length());
    }

    #[test]
    fn a_jump_is_as_high_as_its_power_and_gravity_say() {
        let v = Vars::default();
        let mut w = Flat { step: 0.0 };
        let mut p = Player::at(Vec3::new(-5000.0, 0.0, 0.0));
        run(&mut p, &Cmd::default(), &v, &mut w, 0.1);
        let mut top: f32 = 0.0;
        let mut ticks = 0;
        let jump = Cmd { jump: true, ..Cmd::default() };
        let first = player_move(&mut p, &jump, &v, &mut w, TICK);
        assert!(first.jumped);
        while !p.on_ground() && ticks < 400 {
            // Held: no second jump on landing.
            let ev = player_move(&mut p, &jump, &v, &mut w, TICK);
            assert!(!ev.jumped);
            top = top.max(p.origin.z);
            ticks += 1;
        }
        // v^2 / 2g = 200^2 / 1200 = 33.3 units and 0.667 s in the air, less the jump tick's own gravity (Source takes its
        // half steps before and after the jump is added: 1.5 units at 66 ticks a second).
        assert!((top - 32.0).abs() < 1.0, "top {top}");
        assert!(((ticks as f32 + 1.0) * TICK - 0.65).abs() < 0.05, "air {}", ticks as f32 * TICK);
        // A weaker planet: higher and longer.
        let moon = Vars { gravity: 100.0, ..v };
        let mut p = Player::at(Vec3::new(-5000.0, 0.0, 0.0));
        run(&mut p, &Cmd::default(), &moon, &mut w, 0.1);
        player_move(&mut p, &Cmd::default(), &moon, &mut w, TICK);
        player_move(&mut p, &jump, &moon, &mut w, TICK);
        let mut top: f32 = 0.0;
        for _ in 0..2000 {
            player_move(&mut p, &Cmd::default(), &moon, &mut w, TICK);
            top = top.max(p.origin.z);
        }
        assert!((top - 198.0).abs() < 3.0, "moon top {top}");
        assert!(p.on_ground());
    }

    #[test]
    fn steps_up_18_units_and_not_more() {
        let v = Vars::default();
        let fwd = Cmd { forward: 1.0, ..Cmd::default() };
        let mut w = Flat { step: 17.0 };
        let mut p = Player::at(Vec3::new(100.0, 0.0, 0.0));
        run(&mut p, &fwd, &v, &mut w, 1.5);
        assert!(p.origin.x > 250.0 && (p.origin.z - 17.0).abs() < 0.1, "17: {:?}", p.origin);
        let mut w = Flat { step: 20.0 };
        let mut p = Player::at(Vec3::new(100.0, 0.0, 0.0));
        run(&mut p, &fwd, &v, &mut w, 1.5);
        assert!(p.origin.x < 200.0 && p.origin.z.abs() < 0.1, "20: {:?}", p.origin);
        // A jump clears it.
        let mut p = Player::at(Vec3::new(100.0, 0.0, 0.0));
        run(&mut p, &fwd, &v, &mut w, 0.2);
        run(&mut p, &Cmd { forward: 1.0, jump: true, ..Cmd::default() }, &v, &mut w, 1.5);
        assert!(p.origin.x > 250.0 && (p.origin.z - 20.0).abs() < 0.1, "jumped: {:?}", p.origin);
    }

    #[test]
    fn air_strafing_gains_speed() {
        // Turning with the strafe key held gains speed in the air: the 30-unit cap only limits the speed along the wish direction.
        let v = Vars { gravity: 0.0, ..Vars::default() };
        let mut w = Flat { step: 0.0 };
        let mut p = Player::at(Vec3::new(-5000.0, 0.0, 500.0));
        p.velocity = Vec3::new(300.0, 0.0, 0.0);
        let mut yaw = 0.0_f32;
        for _ in 0..66 {
            yaw += 0.02;
            player_move(&mut p, &Cmd { side: -1.0, yaw, ..Cmd::default() }, &v, &mut w, TICK);
        }
        assert!(p.velocity.length() > 320.0, "{}", p.velocity.length());
        // Straight ahead with W alone: nothing gained.
        let mut p = Player::at(Vec3::new(-5000.0, 0.0, 500.0));
        p.velocity = Vec3::new(300.0, 0.0, 0.0);
        for _ in 0..66 {
            player_move(&mut p, &Cmd { forward: 1.0, ..Cmd::default() }, &v, &mut w, TICK);
        }
        assert!((p.velocity.length() - 300.0).abs() < 0.5);
    }

    #[test]
    fn the_punch_comes_back_to_rest() {
        let mut punch = Punch::default();
        punch.kick(Vec3::new(-2.0, 1.0, 0.0));
        let mut peak: f32 = 0.0;
        for _ in 0..200 {
            punch.decay(TICK);
            peak = peak.max(punch.angle.length());
        }
        assert!(peak > 1.0 && peak < 12.0, "peak {peak}");
        assert!(punch.angle.length() < 0.05);
    }
}
