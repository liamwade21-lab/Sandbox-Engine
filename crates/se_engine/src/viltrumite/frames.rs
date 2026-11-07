//! Where he is (spec F5). All on the real clock (`dt.min(0.1)`), whatever the game's does, so he plays the same at
//! x1000 and paused.
//!
//! - **`Attached`**: below the attach height. Position and velocity in the body's own turning frame, so the ground
//!   can't wobble against him however the date rounds (`sim.jd`'s f64 steps move a planet by metres). Flying, he
//!   simply moves in that frame. Falling, he follows real physics there: each step is worked out in the inertial
//!   frame and turned back, which is the Coriolis and centrifugal terms with no approximation at any spin.
//! - **`Free`**: above it. The same idea with the frame the ship's physics rests in (ship.rs `frame_terms`): he goes
//!   with the nearest body's motion out to 50 radii (faded out by 100), its turn within one radius of height (by two),
//!   beyond that with nothing. **`vel` is relative to that frame.** The carry uses the body's real change this frame, not
//!   its velocity, so a date's rounding or a date jump moves him with it, and hovering (`vel` 0) holds still whatever the
//!   clock does. While falling the frame only turns by what the game clock did beyond his own clock, as ship.rs's carry
//!   does: the part of the turn on his own clock is physics, the ground going round under him. Then `vel` is inertial,
//!   relative to the body's motion.
//!
//! The handover changes no number but the frame's name: `Attached` and `Free` at the same height with the same turn are
//! the same frame, and it's exact while falling too because both are the same physics. Nothing else can make him jump.
//!
//! Pure over [`Scene`], so tests drive it with bodies of their own.

use bevy::math::{DQuat, DVec3};
use se_core::catalog::ObjectRef;
use se_core::upos::UPos;

use crate::ship::{BodyInfo, frame_terms, gravity_at};

use super::ground::Footing;
use super::tune;

/// The body a `Free` hero goes with this frame.
#[derive(Clone, Copy, Debug)]
pub struct Carrier {
    pub obj: ObjectRef,
    pub last_pos: UPos,
    pub last_orient: DQuat,
    /// From the body's centre, universe axes (m). What's real: `pos` is worked out from it, so a hover's rounding to
    /// `UPos`'s millimetre never builds up.
    pub rel: DVec3,
    /// How much of the body's motion he goes with.
    pub fade: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Frame {
    Attached {
        obj: ObjectRef,
        /// From the body's centre, in its frame (m).
        local_pos: DVec3,
        /// In its frame (m/s): relative to the ground and the air.
        local_vel: DVec3,
        /// The fineness of the ground under him, `None` where nothing is loaded.
        level: Option<u8>,
        last_orient: DQuat,
    },
    Free {
        pos: UPos,
        /// Relative to the frame he's carried in, universe axes (m/s). Falling: relative to the body's motion alone.
        vel: DVec3,
        carrier: Option<Carrier>,
    },
}

/// How he moves this step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// Still in the frame.
    Held,
    /// At `vel`, no gravity or drag.
    Flown,
    /// Gravity and drag.
    Ballistic,
}

/// The ground under a point: radial distance from the body's centre, and the node's level. `None`: nothing is loaded
/// there (a sphere without terrain says `Some(0)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    pub radial: f64,
    pub level: Option<u8>,
}

/// What the step works on: this frame's bodies and how to ask about them.
pub struct Scene<'a> {
    pub bodies: &'a [BodyInfo<'a>],
    /// A body's spin (rad/s of the game's clock, universe axes), without the date's rounding.
    pub spin: &'a dyn Fn(ObjectRef) -> DVec3,
    /// A body's velocity (m/s, universe axes), without the date's rounding.
    pub vel: &'a dyn Fn(ObjectRef) -> DVec3,
    /// The ground at a point of the body's frame.
    pub ground: &'a dyn Fn(&BodyInfo<'_>, DVec3) -> Ground,
    /// What's there to stand on at a point of the body's frame.
    pub footing: &'a dyn Fn(&BodyInfo<'_>, DVec3) -> Footing,
    /// The drag per metre (`0.5 rho CdA/m`) at a point of the body's frame.
    pub drag: &'a dyn Fn(&BodyInfo<'_>, DVec3) -> f64,
    /// His middle stays this far over the ground.
    pub stand_m: f64,
}

/// What a step ran into.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    /// He fell onto the ground: his speed towards it (m/s).
    pub contact: Option<f64>,
    /// He flew into the ground, and slid: his speed towards it (m/s). What Ctrl turns into a landing.
    pub flown_into: Option<f64>,
    /// With either: his speed over the ground, and his velocity along it (body frame), as they were.
    pub speed: f64,
    pub along: DVec3,
}

pub(super) fn find<'b, 'a>(s: &'b Scene<'a>, obj: ObjectRef) -> Option<&'b BodyInfo<'a>> {
    s.bodies.iter().find(|b| b.obj == obj)
}

/// Height over the body's reference ellipsoid (m), in its sphere frame.
pub fn height_over(b: &BodyInfo, local: DVec3) -> f64 {
    (local / b.shape).length() - b.radius
}

/// A point in the universe in the body's frame.
pub fn to_local(b: &BodyInfo, pos: UPos) -> DVec3 {
    b.orient.inverse() * pos.delta_m(b.pos)
}

/// The highest its ground can be, over the radius (m).
fn relief_top(b: &BodyInfo) -> f64 {
    b.terrain.map_or(0.0, |t| {
        let (lo, span) = t.ground_range_m();
        (lo + span).max(0.0) as f64
    })
}

/// The attach height over the body's highest ground (m).
fn attach_height(b: &BodyInfo) -> f64 {
    (b.radius * tune::ATTACH_RADII).min(tune::ATTACH_MAX_M)
}

/// The sphere he attaches inside (radius in the sphere frame, m).
pub fn attach_radius(b: &BodyInfo) -> f64 {
    b.radius + relief_top(b) + attach_height(b)
}

fn leave_radius(b: &BodyInfo) -> f64 {
    b.radius + relief_top(b) + attach_height(b) * tune::LEAVE_FACTOR
}

fn solid(b: &BodyInfo) -> bool {
    b.radius > 0.0
}

/// The nearest body whose motion counts where he is, keeping the current one unless another is clearly nearer.
fn pick_carrier<'b, 'a>(s: &'b Scene<'a>, pos: UPos, current: Option<ObjectRef>) -> Option<&'b BodyInfo<'a>> {
    let alt = |b: &BodyInfo| b.pos.delta_m(pos).length() - b.radius;
    let best = s.bodies.iter().filter(|b| solid(b) && frame_terms(alt(b), b.radius).1 > 0.0).min_by(|a, b| alt(a).total_cmp(&alt(b)))?;
    if let Some(c) = current.and_then(|c| find(s, c)).filter(|c| solid(c) && frame_terms(alt(c), c.radius).1 > 0.0) {
        if alt(c).max(0.0) < alt(best).max(0.0) * tune::CARRIER_HYSTERESIS {
            return Some(c);
        }
    }
    Some(best)
}

/// The earliest point of a segment (start `p0`, moving by `d`) inside any body's attach sphere: its share of the
/// segment and the body.
fn first_attach<'b, 'a>(s: &'b Scene<'a>, p0: UPos, d: DVec3) -> Option<(f64, &'b BodyInfo<'a>)> {
    let mut best: Option<(f64, &'b BodyInfo<'a>)> = None;
    let len = d.length();
    for b in s.bodies.iter().filter(|b| solid(b)) {
        let ra = attach_radius(b);
        let rel = p0.delta_m(b.pos);
        // Nowhere near (a shape is at most about 2 radii).
        if rel.length() - len > ra * b.shape.max_element().max(1.0) {
            continue;
        }
        // In the sphere frame the ellipsoid is a sphere.
        let inv = b.orient.inverse();
        let x0 = (inv * rel) / b.shape;
        let dx = (inv * d) / b.shape;
        let c = x0.length_squared() - ra * ra;
        let t = if c <= 0.0 {
            0.0
        } else {
            let (a, h) = (dx.length_squared(), x0.dot(dx));
            let disc = h * h - a * c;
            if a <= 0.0 || h >= 0.0 || disc < 0.0 {
                continue;
            }
            let t = (-h - disc.sqrt()) / a;
            if t > 1.0 {
                continue;
            }
            t
        };
        if best.is_none_or(|(tb, _)| t < tb) {
            best = Some((t, b));
        }
    }
    best
}

/// Where his middle may go down to along `p`, and the ground's level. Where nothing is loaded he holds his height if
/// he's already under where ground could be (F2: never the ellipsoid, oceans' seabeds and burials follow).
fn floor_at(s: &Scene, b: &BodyInfo, p: DVec3, held: f64) -> (f64, Option<u8>) {
    let g = (s.ground)(b, p);
    match g.level {
        Some(_) => (g.radial + s.stand_m, g.level),
        None => {
            let top = g.radial * (b.radius + relief_top(b)) / b.radius.max(1.0) + tune::UNLOADED_MARGIN_M;
            (top.min(held), None)
        }
    }
}

/// The first point after `p0`, on the way to `p1`, under the floor, by sphere tracing. (He may start on it: flying
/// along the ground he does, every frame.) The steps run out on a long path slanting past the body, each only a share of
/// the height: then the path's lowest point says whether it goes in, and bisection finds where. Judged by its end alone
/// he came out on the far side.
fn trace_ground(s: &Scene, b: &BodyInfo, p0: DVec3, p1: DVec3) -> Option<DVec3> {
    let held = p0.length();
    let seg = p1 - p0;
    let len = seg.length();
    let over = |p: DVec3| p.length() - floor_at(s, b, p, held).0;
    let mut alt = over(p0);
    let mut t = 0.0;
    for _ in 0..tune::TRACE_STEPS {
        t += (alt * tune::TRACE_SHARE).max(tune::TRACE_MIN_M);
        if t >= len {
            return (over(p1) <= 0.0).then_some(p1);
        }
        let p = p0 + seg * (t / len);
        alt = over(p);
        if alt <= 0.0 {
            return Some(p);
        }
    }
    let dir = seg / len;
    let lowest = (-p0.dot(dir)).clamp(t, len);
    if over(p0 + dir * lowest) > 0.0 {
        return None;
    }
    let (mut above, mut under) = (t, lowest);
    for _ in 0..tune::TRACE_BISECT {
        let mid = 0.5 * (above + under);
        if over(p0 + dir * mid) <= 0.0 {
            under = mid;
        } else {
            above = mid;
        }
    }
    Some(p0 + dir * under)
}

impl Frame {
    /// Free at a point of the universe, still.
    pub fn at(pos: UPos) -> Frame {
        Frame::Free { pos, vel: DVec3::ZERO, carrier: None }
    }

    /// Where he was left in `Free`, or `last`, where he last was: before this frame's bodies are known.
    pub fn world_or(&self, last: UPos) -> UPos {
        match self {
            Frame::Attached { .. } => last,
            Frame::Free { pos, .. } => *pos,
        }
    }

    /// Where he is in the universe. (`last`: where he was, if his body has gone.)
    pub fn world(&self, s: &Scene, last: UPos) -> UPos {
        match self {
            Frame::Attached { obj, local_pos, .. } => find(s, *obj).map_or(last, |b| b.pos.offset_m(b.orient * *local_pos)),
            Frame::Free { pos, .. } => *pos,
        }
    }

    pub fn body(&self) -> Option<ObjectRef> {
        match self {
            Frame::Attached { obj, .. } => Some(*obj),
            Frame::Free { carrier, .. } => carrier.map(|c| c.obj),
        }
    }

    pub fn attached(&self) -> bool {
        matches!(self, Frame::Attached { .. })
    }

    /// Takes him along with what the bodies did since last frame. Returns the turn the view should go with. `motion`
    /// is this frame's, `last` where he was.
    ///
    /// Falling, in both frames, only what the game clock did beyond his own counts: the turn on his own clock is what
    /// the step's inertial motion lets the ground do under him. Change a state before this, not after: the turn back here and
    /// the inertial step in `advance` cancel on the ground only in the same frame.
    pub fn carry(&mut self, s: &Scene, dt: f64, motion: Motion, last: UPos) -> DQuat {
        match self {
            Frame::Attached { obj, local_pos, local_vel, last_orient, .. } => match find(s, *obj) {
                Some(b) => {
                    let turn = (b.orient * last_orient.inverse()).normalize();
                    *last_orient = b.orient;
                    if motion != Motion::Ballistic {
                        return turn;
                    }
                    let w = (s.spin)(b.obj);
                    let back = DQuat::from_scaled_axis(b.orient.inverse() * w * -dt);
                    *local_pos = back * *local_pos;
                    *local_vel = back * *local_vel;
                    (turn * DQuat::from_scaled_axis(w * -dt)).normalize()
                }
                // Its system has gone: he goes on from where he was.
                None => {
                    let vel = *last_orient * *local_vel;
                    *self = Frame::Free { pos: last, vel, carrier: None };
                    DQuat::IDENTITY
                }
            },
            Frame::Free { pos, vel, carrier } => {
                let mut view = DQuat::IDENTITY;
                match carrier.as_mut().and_then(|c| Some((find(s, c.obj)?, c))) {
                    Some((b, c)) => {
                        let turn = (b.orient * c.last_orient.inverse()).normalize();
                        let dp = b.pos.delta_m(c.last_pos);
                        let (_, fade, share) = frame_terms(c.rel.length() - b.radius, b.radius);
                        let r = match motion {
                            Motion::Ballistic => (turn * DQuat::from_scaled_axis((s.spin)(b.obj) * -dt)).normalize(),
                            _ => turn,
                        };
                        let r = if share >= 1.0 { r } else { DQuat::IDENTITY.slerp(r, share).normalize() };
                        c.rel = r * c.rel + dp * (fade - 1.0);
                        *vel = r * *vel;
                        if motion == Motion::Ballistic && fade != c.fade {
                            *vel += (s.vel)(b.obj) * (c.fade - fade);
                        }
                        c.fade = fade;
                        c.last_pos = b.pos;
                        c.last_orient = b.orient;
                        *pos = b.pos.offset_m(c.rel);
                        view = r;
                    }
                    None => *carrier = None,
                }
                // Who carries him from here on. (After the carry: a date's jump moves a body far from where he was, and
                // he should have gone with it.)
                let want = pick_carrier(s, *pos, carrier.map(|c| c.obj));
                if want.map(|b| b.obj) != carrier.map(|c| c.obj) {
                    let alt = want.map_or(0.0, |b| b.pos.delta_m(*pos).length() - b.radius);
                    let fade = want.map_or(0.0, |b| frame_terms(alt, b.radius).1);
                    if motion == Motion::Ballistic {
                        let old = carrier.map_or(DVec3::ZERO, |c| (s.vel)(c.obj) * c.fade);
                        let new = want.map_or(DVec3::ZERO, |b| (s.vel)(b.obj) * fade);
                        *vel += old - new;
                    }
                    *carrier = want.map(|b| Carrier { obj: b.obj, last_pos: b.pos, last_orient: b.orient, rel: pos.delta_m(b.pos), fade });
                }
                view
            }
        }
    }

    /// Moves him over `dt`: the frame change happens here, and swept collision. `Attached` and `Free` both end it on
    /// the ground or above it, never under.
    pub fn advance(&mut self, s: &Scene, dt: f64, motion: Motion) -> Outcome {
        if let Frame::Free { pos, vel, carrier } = self {
            let p0 = *pos;
            let v0 = if motion == Motion::Held { DVec3::ZERO } else { *vel };
            let frame_body = carrier.and_then(|c| find(s, c.obj));
            let fade = carrier.map_or(0.0, |c| c.fade);
            let (d, mut v1) = match motion {
                Motion::Held => (DVec3::ZERO, DVec3::ZERO),
                Motion::Flown => (v0 * dt, v0),
                Motion::Ballistic => {
                    let a_frame = frame_body.map_or(DVec3::ZERO, |b| gravity_at(s.bodies, b.pos) * fade);
                    let half = v0 + (gravity_at(s.bodies, p0) - a_frame) * (dt * 0.5);
                    let d = half * dt;
                    let v1 = half + (gravity_at(s.bodies, p0.offset_m(d)) - a_frame) * (dt * 0.5);
                    (d, v1)
                }
            };
            if motion == Motion::Ballistic {
                if let Some(b) = frame_body {
                    let local = to_local(b, p0.offset_m(d));
                    let k = (s.drag)(b, local);
                    if k > 0.0 {
                        let rel = p0.offset_m(d).delta_m(b.pos);
                        let w = (s.spin)(b.obj) * frame_terms(rel.length() - b.radius, b.radius).2;
                        let air = v1 - w.cross(rel);
                        v1 = air / (1.0 + k * air.length() * dt) + w.cross(rel);
                    }
                }
            }
            match first_attach(s, p0, d) {
                Some((t, b)) => {
                    let at = p0.offset_m(d * t);
                    let local_pos = to_local(b, at);
                    // As it was at the cut: the rest of the step is yet to come.
                    let v1 = v0 + (v1 - v0) * t;
                    let local_vel = match motion {
                        Motion::Held => DVec3::ZERO,
                        Motion::Flown => b.orient.inverse() * v1,
                        Motion::Ballistic => {
                            // Relative to the new body's motion: without its velocity when it's what carried him,
                            // so the date's rounding stays out.
                            let same = frame_body.is_some_and(|c| c.obj == b.obj);
                            let rel = if same && fade >= 1.0 {
                                v1
                            } else {
                                v1 + frame_body.map_or(DVec3::ZERO, |c| (s.vel)(c.obj) * fade) - (s.vel)(b.obj)
                            };
                            let w = b.orient.inverse() * (s.spin)(b.obj);
                            b.orient.inverse() * rel - w.cross(local_pos)
                        }
                    };
                    let rest = dt * (1.0 - t);
                    *self = Frame::Attached { obj: b.obj, local_pos, local_vel, level: None, last_orient: b.orient };
                    return self.advance_attached(s, rest, motion);
                }
                None => {
                    *vel = v1;
                    match carrier {
                        Some(c) => {
                            c.rel += d;
                            *pos = frame_body.map_or_else(|| p0.offset_m(d), |b| b.pos.offset_m(c.rel));
                        }
                        None => *pos = p0.offset_m(d),
                    }
                }
            }
            Outcome::default()
        } else {
            self.advance_attached(s, dt, motion)
        }
    }

    fn advance_attached(&mut self, s: &Scene, dt: f64, motion: Motion) -> Outcome {
        let Frame::Attached { obj, local_pos, local_vel, level, .. } = self else { return Outcome::default() };
        let Some(b) = find(s, *obj) else { return Outcome::default() };
        let p0 = *local_pos;
        // Where the step starts against the ground: `carry` turned a falling hero back by the ground's turn over the step.
        let mut from = p0;
        let (p1, mut v1) = match motion {
            Motion::Held => (p0, DVec3::ZERO),
            Motion::Flown => (p0 + *local_vel * dt, *local_vel),
            Motion::Ballistic => {
                // In the inertial frame, about the axes at the start of the step (`carry` turned him back for it):
                // Coriolis and centrifugal exactly, at any spin.
                let w = b.orient.inverse() * (s.spin)(b.obj);
                from = DQuat::from_scaled_axis(w * dt) * p0;
                let a_frame = gravity_at(s.bodies, b.pos);
                let accel = |p: DVec3| b.orient.inverse() * (gravity_at(s.bodies, b.pos.offset_m(b.orient * p)) - a_frame);
                let half = *local_vel + w.cross(p0) + accel(p0) * (dt * 0.5);
                let p1 = p0 + half * dt;
                let v_out = half + accel(p1) * (dt * 0.5);
                (p1, v_out - w.cross(p1))
            }
        };
        if motion == Motion::Ballistic {
            let k = (s.drag)(b, p1);
            if k > 0.0 {
                v1 /= 1.0 + k * v1.length() * dt;
            }
        }
        let mut out = Outcome::default();
        let mut p = p1;
        if let Some(hit) = trace_ground(s, b, from, p1) {
            let held = p0.length();
            let (floor, loaded) = floor_at(s, b, hit, held);
            let up = hit.normalize_or(DVec3::Y);
            p = up * floor;
            let into = -v1.dot(up);
            match motion {
                // Along the ground, the whole step, less what goes into it. (Stopped at the hit he crawled: on any rising
                // ground the trace's first metre is under the floor, every frame.)
                Motion::Flown => {
                    if into > 0.0 {
                        out.flown_into = Some(into);
                        out.speed = v1.length();
                        v1 += up * into;
                        out.along = v1;
                    }
                    let rest = p1 - hit;
                    let along = hit + rest - up * rest.dot(up).min(0.0);
                    p = along.normalize_or(up) * along.length().max(floor_at(s, b, along, held).0);
                }
                Motion::Held => {}
                // Nothing loaded: held in the air, no landing, and the fall goes on when there's ground (F2).
                Motion::Ballistic if loaded.is_none() => v1 = DVec3::ZERO,
                Motion::Ballistic => {
                    out.contact = Some(into.max(0.0));
                    out.speed = v1.length();
                    out.along = v1 + up * into;
                    v1 = DVec3::ZERO;
                }
            }
        }
        *level = (s.ground)(b, p).level;
        *local_pos = p;
        *local_vel = v1;
        if (p / b.shape).length() > leave_radius(b) {
            let rel = b.orient * p;
            let vel = match motion {
                Motion::Ballistic => {
                    let w = b.orient.inverse() * (s.spin)(b.obj);
                    b.orient * (v1 + w.cross(p))
                }
                _ => b.orient * v1,
            };
            let (_, fade, _) = frame_terms(rel.length() - b.radius, b.radius);
            *self = Frame::Free {
                pos: b.pos.offset_m(rel),
                vel,
                carrier: Some(Carrier { obj: b.obj, last_pos: b.pos, last_orient: b.orient, rel, fade }),
            };
        }
        out
    }

    /// Velocity relative to the frame, universe axes: what flight steers. (Falling in `Free`, relative to the body's
    /// motion alone: see [`Frame::over_ground`].)
    pub fn velocity(&self, s: &Scene) -> DVec3 {
        match self {
            Frame::Attached { obj, local_vel, last_orient, .. } => find(s, *obj).map_or(*last_orient, |b| b.orient) * *local_vel,
            Frame::Free { vel, .. } => *vel,
        }
    }

    pub fn set_velocity(&mut self, s: &Scene, v: DVec3) {
        match self {
            Frame::Attached { obj, local_vel, last_orient, .. } => *local_vel = find(s, *obj).map_or(*last_orient, |b| b.orient).inverse() * v,
            Frame::Free { vel, .. } => *vel = v,
        }
    }

    /// His velocity along the body's frame, in that frame: on the ground, what walking steers.
    pub fn set_local_velocity(&mut self, v: DVec3) {
        if let Frame::Attached { local_vel, .. } = self {
            *local_vel = v;
        }
    }

    /// The jump's fixed impulse: straight up from where he stands.
    pub fn kick_up(&mut self, speed: f64) {
        if let Frame::Attached { local_pos, local_vel, .. } = self {
            *local_vel += local_pos.normalize_or(DVec3::Y) * speed;
        }
    }

    /// The share of its body's turn the frame goes with.
    pub fn turn_share(&self, s: &Scene) -> f64 {
        match self {
            Frame::Attached { .. } => 1.0,
            Frame::Free { carrier: Some(c), .. } => find(s, c.obj).map_or(0.0, |b| frame_terms(c.rel.length() - b.radius, b.radius).2),
            Frame::Free { .. } => 0.0,
        }
    }

    /// The velocity the `Free` frame's turn gives his position.
    fn turning(&self, s: &Scene) -> DVec3 {
        let Frame::Free { carrier: Some(c), .. } = self else { return DVec3::ZERO };
        ((s.spin)(c.obj) * self.turn_share(s)).cross(c.rel)
    }

    /// Velocity relative to the ground (or what carries him), universe axes, while falling.
    pub fn over_ground(&self, s: &Scene) -> DVec3 {
        self.velocity(s) - self.turning(s)
    }

    /// From flying (relative to the turning frame) to falling (inertial). `Attached` needs nothing: its velocity is
    /// relative to the turning frame in both.
    pub fn to_ballistic(&mut self, s: &Scene) {
        let t = self.turning(s);
        if let Frame::Free { vel, .. } = self {
            *vel += t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::units::G;

    const EARTH_R: f64 = 6.378e6;

    /// A round body of `radius` and `gm`, its ground at the radius, turning about its own Y.
    fn body(obj: u32, pos: UPos, orient: DQuat, radius: f64, gm: f64) -> BodyInfo<'static> {
        BodyInfo { obj: ObjectRef::Body(obj), pos, orient, radius, shape: DVec3::ONE, gm, terrain: None }
    }

    /// A test's world: the bodies, one spin and velocity, ground loaded at the radius.
    fn with_scene<R>(bodies: &[BodyInfo<'static>], w: DVec3, v: DVec3, drag: f64, f: impl FnOnce(&Scene) -> R) -> R {
        let (spin, vel) = (move |_| w, move |_| v);
        let ground = |b: &BodyInfo, _: DVec3| Ground { radial: b.radius, level: Some(3) };
        let dragf = move |_: &BodyInfo, _: DVec3| drag;
        let footing = |_: &BodyInfo, _: DVec3| Footing::Land;
        f(&Scene { bodies, spin: &spin, vel: &vel, ground: &ground, footing: &footing, drag: &dragf, stand_m: 1.0 })
    }

    /// The jump: 28 m/s up, gravity alone. Six times higher on the Moon, as gravity is a sixth. (Drag is the caller's: none rising.)
    #[test]
    fn a_jump_is_six_times_higher_on_the_moon() {
        let height = |radius: f64, gm: f64| {
            let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, radius, gm)];
            with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
                let start = DVec3::Y * (radius + s.stand_m);
                let mut f = Frame::Attached { obj: ObjectRef::Body(0), local_pos: start, local_vel: DVec3::ZERO, level: Some(3), last_orient: DQuat::IDENTITY };
                f.kick_up(tune::JUMP_MS);
                let mut most: f64 = 0.0;
                for _ in 0..60 * 60 {
                    f.carry(s, 1.0 / 60.0, Motion::Ballistic, UPos::ZERO);
                    let o = f.advance(s, 1.0 / 60.0, Motion::Ballistic);
                    most = most.max(local_of(&f, s, &bodies[0]).length() - radius - s.stand_m);
                    if o.contact.is_some() {
                        assert!((o.contact.unwrap() - tune::JUMP_MS).abs() < 0.5, "back at {:?}", o.contact);
                        break;
                    }
                }
                most
            })
        };
        let earth = height(EARTH_R, 3.986e14);
        let moon = height(1.7374e6, 4.905e12);
        assert!((earth - 40.1).abs() < 0.5, "Earth {earth}");
        assert!((moon - 242.0).abs() < 2.0, "Moon {moon}");
        assert!((moon / earth - 6.04).abs() < 0.1, "{}", moon / earth);
    }

    fn local_of(f: &Frame, s: &Scene, b: &BodyInfo) -> DVec3 {
        to_local(b, f.world(s, UPos::ZERO))
    }

    /// Flying up and down through the handover on a turning, moving body: in the body's frame he moves by exactly his
    /// velocity, at x1 and x1000 and backwards in time, whichever frame he's in.
    #[test]
    fn the_handover_makes_no_jump_flying() {
        let dt = 1.0 / 60.0;
        let w_real = 7.29e-5;
        for rate in [1.0, 1000.0, -300.0, 0.0] {
            let c0 = UPos::ZERO.offset_m(DVec3::new(1.5e11, 3e9, -4e10));
            let (mut t, mut pos, mut q) = (0.0, c0, DQuat::IDENTITY);
            let mk = |p, q| [body(0, p, q, EARTH_R, 0.0)];
            let bodies = mk(pos, q);
            let start = attach_radius(&bodies[0]) - 3000.0;
            let up = DVec3::new(0.6, 0.8, 0.0);
            let mut f = with_scene(&bodies, DVec3::Y * w_real, DVec3::X * 3e4, 0.0, |s| {
                let mut f = Frame::Free { pos: pos.offset_m(up * start), vel: DVec3::ZERO, carrier: None };
                f.carry(s, dt, Motion::Flown, UPos::ZERO);
                f.advance(s, dt, Motion::Flown);
                f
            });
            assert!(f.attached());
            let (mut was_attached, mut changes) = (true, 0);
            for k in 0..2000 {
                // Up at 4 km/s through the leaving height (63 km up), then back down through the attach height.
                let speed = if k < 1000 { 4000.0 } else { -4000.0 };
                t += dt * rate;
                pos = c0.offset_m(DVec3::X * 3e4 * t);
                q = DQuat::from_rotation_y(w_real * t);
                let bodies = mk(pos, q);
                let (before, after) = with_scene(&bodies, DVec3::Y * w_real, DVec3::X * 3e4, 0.0, |s| {
                    let b = &bodies[0];
                    f.carry(s, dt, Motion::Flown, UPos::ZERO);
                    let before = local_of(&f, s, b);
                    let along = before.normalize();
                    f.set_velocity(s, b.orient * (along * speed));
                    f.advance(s, dt, Motion::Flown);
                    (before, local_of(&f, s, b))
                });
                let moved = (after - before).length();
                assert!((moved - 4000.0 * dt).abs() < 5e-3, "rate {rate} step {k}: moved {moved}");
                if f.attached() != was_attached {
                    changes += 1;
                    was_attached = f.attached();
                }
            }
            assert_eq!(changes, 2, "rate {rate}");
        }
    }

    /// Falling through the handover: the same fall whichever frame carries it. Compared with one integration that stays
    /// in `Free`, from 1,000 km over a turning planet, at the moment he passes through both ways.
    #[test]
    fn the_handover_makes_no_jump_falling() {
        let dt = 1.0 / 60.0;
        let (w, gm) = (DVec3::Y * 7.29e-5, G * 5.97e24);
        let start = UPos::ZERO.offset_m(DVec3::new(1.0, 0.1, 0.2).normalize() * (EARTH_R + 400e3));
        let vel0 = DVec3::new(-1.0, 0.0, 0.5).normalize() * 300.0;
        // The body's frame at time t.
        let at = |t: f64| [body(0, UPos::ZERO, DQuat::from_rotation_y(7.29e-5 * t), EARTH_R, gm)];
        let bodies0 = at(0.0);
        let mut f = with_scene(&bodies0, w, DVec3::ZERO, 0.0, |s| {
            let mut f = Frame::Free { pos: start, vel: vel0, carrier: None };
            f.carry(s, dt, Motion::Ballistic, UPos::ZERO);
            f
        });
        // A plain inertial step for comparison.
        let (mut p, mut v) = (start.delta_m(UPos::ZERO), vel0);
        let accel = |p: DVec3| p * (-gm / p.length().powi(3));
        let (mut t, mut attached, mut changes) = (0.0, false, 0);
        for k in 0..12_000 {
            t += dt;
            let bodies = at(t);
            let got = with_scene(&bodies, w, DVec3::ZERO, 0.0, |s| {
                f.carry(s, dt, Motion::Ballistic, UPos::ZERO);
                let o = f.advance(s, dt, Motion::Ballistic);
                assert_eq!(o.contact, None, "step {k}");
                f.world(s, UPos::ZERO).delta_m(UPos::ZERO)
            });
            let half = v + accel(p) * (dt * 0.5);
            p += half * dt;
            v = half + accel(p) * (dt * 0.5);
            assert!((got - p).length() < 0.05, "step {k}: {got} against {p}");
            if f.attached() != attached {
                changes += 1;
                attached = f.attached();
            }
        }
        assert!(changes >= 1, "{changes} handovers");
    }

    /// Hovering stays put in the body's frame, whatever the date does: a jump of the date, rounding of the body's place,
    /// a clock at x1000 or paused.
    #[test]
    fn hovering_stays_put_in_any_frame_at_any_rate() {
        let dt = 1.0 / 60.0;
        for height in [1000.0, 200e3, 500e3, 3000e3] {
            let up = DVec3::new(0.0, 0.6, 0.8);
            let mut pos = UPos::ZERO.offset_m(DVec3::X * 1.5e11);
            let mut q = DQuat::IDENTITY;
            let bodies = [body(0, pos, q, EARTH_R, 0.0)];
            let local0 = up * (EARTH_R + height);
            let mut f = with_scene(&bodies, DVec3::Y * 7e-5, DVec3::X * 3e4, 0.0, |s| {
                let mut f = Frame::Free { pos: pos.offset_m(q * local0), vel: DVec3::ZERO, carrier: None };
                f.carry(s, dt, Motion::Held, UPos::ZERO);
                f.advance(s, dt, Motion::Held);
                f
            });
            for k in 0..6000 {
                // x1000: 16 s of the clock a frame. Now and then a date jump.
                let jump = if k % 1000 == 999 { 1.0e6 } else { 1000.0 * dt };
                q = (DQuat::from_rotation_y(7e-5 * jump) * q).normalize();
                pos = pos.offset_m(DVec3::new(3e4 * jump, 1.3 * jump, -2e3 * jump));
                let bodies = [body(0, pos, q, EARTH_R, 0.0)];
                with_scene(&bodies, DVec3::Y * 7e-5, DVec3::X * 3e4, 0.0, |s| {
                    f.carry(s, dt, Motion::Held, UPos::ZERO);
                    f.advance(s, dt, Motion::Held);
                    let got = local_of(&f, s, &bodies[0]);
                    assert!((got - local0).length() < 2e-3, "height {height} step {k}: off by {}", (got - local0).length());
                });
            }
        }
    }

    /// A fast spinner: flying up through the handover, the view turns exactly with the body.
    #[test]
    fn the_view_goes_with_the_frame() {
        let dt = 1.0 / 60.0;
        let w = 1e-3;
        let up = DVec3::Y;
        let mut q = DQuat::IDENTITY;
        let bodies = [body(0, UPos::ZERO, q, 1e6, 0.0)];
        let mut f = with_scene(&bodies, DVec3::Y * w, DVec3::ZERO, 0.0, |s| {
            let mut f = Frame::Free { pos: UPos::ZERO.offset_m(up * 1.1e6), vel: DVec3::ZERO, carrier: None };
            f.carry(s, dt, Motion::Held, UPos::ZERO);
            f.advance(s, dt, Motion::Held);
            f
        });
        let mut view = DQuat::IDENTITY;
        for _ in 0..200 {
            q = (DQuat::from_rotation_y(w * dt * 100.0) * q).normalize();
            let bodies = [body(0, UPos::ZERO, q, 1e6, 0.0)];
            with_scene(&bodies, DVec3::Y * w, DVec3::ZERO, 0.0, |s| {
                view = (f.carry(s, dt, Motion::Held, UPos::ZERO) * view).normalize();
                f.advance(s, dt, Motion::Held);
            });
            assert!((view * DVec3::X - q * DVec3::X).length() < 1e-9 && (view * DVec3::Y - q * DVec3::Y).length() < 1e-9, "{view} against {q}");
        }
    }

    /// He orbits: over ten periods of a low elliptical orbit the ellipse's axis and shape stay, in both frames.
    #[test]
    fn a_coast_is_a_stable_ellipse() {
        let dt = 1.0 / 60.0;
        // A dense little world, so a period is 30 s. Low enough to pass through the attach height.
        let (radius, gm) = (1e6, 4.0 * std::f64::consts::PI.powi(2) * (1.3e6f64).powi(3) / (30.0f64).powi(2));
        let w = DVec3::Y * 1e-3;
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, radius, gm)];
        let mut f = with_scene(&bodies, w, DVec3::ZERO, 0.0, |s| {
            let v = (gm / 1.3e6).sqrt() * 1.1;
            let mut f = Frame::Free { pos: UPos::ZERO.offset_m(DVec3::X * 1.3e6), vel: DVec3::Z * v, carrier: None };
            f.carry(s, dt, Motion::Ballistic, UPos::ZERO);
            f
        });
        let (mut lo, mut hi, mut attached_frames) = (f64::MAX, 0.0f64, 0);
        let mut q = DQuat::IDENTITY;
        let mut last_e: Option<f64> = None;
        for k in 0..18_000 {
            // The body turns at x1 under him.
            q = (DQuat::from_scaled_axis(w * dt) * q).normalize();
            let bodies = [body(0, UPos::ZERO, q, radius, gm)];
            let (r, e) = with_scene(&bodies, w, DVec3::ZERO, 0.0, |s| {
                f.carry(s, dt, Motion::Ballistic, UPos::ZERO);
                let o = f.advance(s, dt, Motion::Ballistic);
                assert_eq!(o.contact, None);
                let p = f.world(s, UPos::ZERO).delta_m(UPos::ZERO);
                // Inertial velocity in either frame.
                let v = match &f {
                    Frame::Attached { local_vel, local_pos, .. } => {
                        let wl = q.inverse() * w;
                        q * (*local_vel + wl.cross(*local_pos))
                    }
                    Frame::Free { vel, .. } => *vel,
                };
                (p.length(), v.length_squared() / 2.0 - gm / p.length())
            });
            lo = lo.min(r);
            hi = hi.max(r);
            attached_frames += f.attached() as u32;
            if let Some(e0) = last_e {
                assert!(((e - e0) / e0).abs() < 1e-3, "energy at {k}: {e} against {e0}");
            } else {
                last_e = Some(e);
            }
        }
        // Verlet's ellipse: the same ends, none of it spiralling.
        assert!(lo > 1.29e6 && hi < 2.02e6 && hi > 1.9e6, "{lo} .. {hi}");
        assert!(attached_frames > 100, "the orbit should pass through the attach height: {attached_frames}");
    }

    /// At any speed a body in the way stops him: no tunnelling, he ends above the ground, not past it.
    #[test]
    fn swept_collision_catches_him_at_any_speed() {
        let dt = 1.0 / 60.0;
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, EARTH_R, 0.0)];
        for speed in [3e3, 1e6, 3e8, 3e11, 3e15] {
            let start = DVec3::Z * (attach_radius(&bodies[0]) + speed * dt * 0.5);
            let mut f = Frame::Free { pos: UPos::ZERO.offset_m(start), vel: DVec3::NEG_Z * speed, carrier: None };
            with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
                for _ in 0..7000 {
                    f.carry(s, dt, Motion::Flown, UPos::ZERO);
                    f.advance(s, dt, Motion::Flown);
                    let r = f.world(s, UPos::ZERO).delta_m(UPos::ZERO);
                    assert!(r.z > 0.0 && r.length() >= EARTH_R, "speed {speed}: through the body at {r}");
                }
                assert!(f.attached(), "speed {speed}");
                let r = f.world(s, UPos::ZERO).delta_m(UPos::ZERO).length();
                assert!((r - EARTH_R - 1.0).abs() < 1e-2, "speed {speed}: stopped {} above the ground", r - EARTH_R);
            });
        }
    }

    /// Falling he hits the ground and the step says how fast; flying he slides along it.
    #[test]
    fn contact_is_reported_falling_and_slides_flying() {
        let dt = 1.0 / 60.0;
        let gm = G * 5.97e24;
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, EARTH_R, gm)];
        with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
            let mut f = Frame::Free { pos: UPos::ZERO.offset_m(DVec3::Y * (EARTH_R + 50.0)), vel: DVec3::ZERO, carrier: None };
            let mut hit = None;
            for _ in 0..1000 {
                f.carry(s, dt, Motion::Ballistic, UPos::ZERO);
                let o = f.advance(s, dt, Motion::Ballistic);
                if o.contact.is_some() {
                    hit = o.contact;
                    break;
                }
            }
            // sqrt(2 g h), Earth's g, 49 m.
            let want = (2.0 * 9.8 * 49.0f64).sqrt();
            assert!((hit.unwrap() - want).abs() < 1.0, "{hit:?} against {want}");
            let mut f = Frame::Attached { obj: ObjectRef::Body(0), local_pos: DVec3::Y * (EARTH_R + 1.0), local_vel: DVec3::new(100.0, -50.0, 0.0), level: None, last_orient: DQuat::IDENTITY };
            f.carry(s, dt, Motion::Flown, UPos::ZERO);
            let o = f.advance(s, dt, Motion::Flown);
            assert_eq!(o.contact, None);
            let Frame::Attached { local_pos, local_vel, .. } = f else { panic!() };
            assert!(local_pos.length() >= EARTH_R + 1.0 - 1e-6, "{}", local_pos.length() - EARTH_R);
            assert!(local_vel.y.abs() < 1.0 && local_vel.x > 99.0, "{local_vel}");
        });
    }

    /// A path that runs out of trace steps: slanting past the body at speed. It stops at the ground if it goes in (every
    /// frame's a chord of the body at these speeds), and goes straight on if it doesn't.
    #[test]
    fn a_fast_slanting_path_cannot_pass_through_the_body() {
        let dt = 1.0 / 60.0;
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, EARTH_R, 0.0)];
        with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
            for motion in [Motion::Flown, Motion::Ballistic] {
                for speed in [6e8, 3e9, 3e11] {
                    for off in [0.6, 0.69, 0.8, 0.95, 0.995] {
                        let side = DVec3::X * EARTH_R * off;
                        let start = side + DVec3::Z * (attach_radius(&bodies[0]) * 1.5);
                        let mut f = Frame::Free { pos: UPos::ZERO.offset_m(start), vel: DVec3::NEG_Z * speed, carrier: None };
                        f.carry(s, dt, motion, UPos::ZERO);
                        let o = f.advance(s, dt, motion);
                        let r = f.world(s, UPos::ZERO).delta_m(UPos::ZERO);
                        assert!(r.length() >= EARTH_R, "{motion:?} at {speed}, off {off}: under the ground at {r}");
                        if motion == Motion::Ballistic {
                            assert!(r.z > 0.0 && o.contact.is_some(), "at {speed}, off {off}: went on to {r}");
                        } else {
                            // Flying he goes on along the ground, off to the side he hit it: never straight on through.
                            assert!(r.x > side.x + 1.0, "at {speed}, off {off}: went on to {r}");
                        }
                    }
                    // Over the ground the whole way: nothing stops him.
                    let start = DVec3::X * EARTH_R * 1.01 + DVec3::Z * (attach_radius(&bodies[0]) * 1.5);
                    let mut f = Frame::Free { pos: UPos::ZERO.offset_m(start), vel: DVec3::NEG_Z * speed, carrier: None };
                    f.carry(s, dt, motion, UPos::ZERO);
                    let o = f.advance(s, dt, motion);
                    let r = f.world(s, UPos::ZERO).delta_m(UPos::ZERO);
                    let want = start.z - speed * dt;
                    assert!(o.contact.is_none() && (r.z - want).abs() < want.abs().max(1e6) * 1e-3, "{motion:?} at {speed}: stopped at {r}, not {want}");
                }
            }
        });
    }

    /// Flying into rising ground he slides along it at his whole speed; cut at the hit he crawled at 1 m a frame.
    #[test]
    fn flying_up_rising_ground_keeps_the_whole_step() {
        let dt = 1.0 / 60.0;
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, EARTH_R, 0.0)];
        // A slope of 1 in 10 along +X.
        let (spin, vel) = (|_| DVec3::ZERO, |_| DVec3::ZERO);
        let ground = |b: &BodyInfo, l: DVec3| Ground { radial: b.radius + l.normalize().x * b.radius * 0.1, level: Some(3) };
        let drag = |_: &BodyInfo, _: DVec3| 0.0;
        let footing = |_: &BodyInfo, _: DVec3| Footing::Land;
        let s = Scene { bodies: &bodies, spin: &spin, vel: &vel, ground: &ground, footing: &footing, drag: &drag, stand_m: 1.0 };
        let top = (s.ground)(&bodies[0], DVec3::Y).radial + 1.0;
        let mut f = Frame::Attached { obj: ObjectRef::Body(0), local_pos: DVec3::Y * top, local_vel: DVec3::X * 3000.0, level: None, last_orient: DQuat::IDENTITY };
        for _ in 0..60 {
            f.carry(&s, dt, Motion::Flown, UPos::ZERO);
            f.advance(&s, dt, Motion::Flown);
        }
        let Frame::Attached { local_pos, local_vel, .. } = f else { panic!() };
        assert!((local_pos.x - 3000.0).abs() < 30.0, "moved {}", local_pos.x);
        assert!((local_vel.length() - 3000.0).abs() < 1.0, "{local_vel}");
        assert!(local_pos.length() >= (s.ground)(&bodies[0], local_pos).radial + 1.0 - 1e-6);
    }

    /// X on the floor of a turning body: where he stood. (The path used to start from where `carry` turned him back to.)
    #[test]
    fn falling_from_the_floor_stays_where_he_stood() {
        let dt = 1.0 / 60.0;
        let w = DVec3::Y * 7.29e-5;
        let gm = G * 5.97e24;
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, EARTH_R, gm)];
        let start = DVec3::X * (EARTH_R + 1.0);
        with_scene(&bodies, w, DVec3::ZERO, 0.0, |s| {
            let mut f = Frame::Attached { obj: ObjectRef::Body(0), local_pos: start, local_vel: DVec3::ZERO, level: None, last_orient: DQuat::IDENTITY };
            f.carry(s, dt, Motion::Ballistic, UPos::ZERO);
            let o = f.advance(s, dt, Motion::Ballistic);
            assert!(o.contact.is_some());
            let Frame::Attached { local_pos, .. } = f else { panic!() };
            assert!((local_pos - start).length() < 1e-3, "moved {}", (local_pos - start).length());
        });
        // Falling and held over ground not yet loaded: no drift against the body either.
        let (spin, vel) = (move |_| w, |_| DVec3::ZERO);
        let ground = |b: &BodyInfo, _: DVec3| Ground { radial: b.radius, level: None };
        let drag = |_: &BodyInfo, _: DVec3| 0.0;
        let footing = |_: &BodyInfo, _: DVec3| Footing::Land;
        let s = Scene { bodies: &bodies, spin: &spin, vel: &vel, ground: &ground, footing: &footing, drag: &drag, stand_m: 1.0 };
        let start = DVec3::X * (EARTH_R + 50.0);
        let mut f = Frame::Attached { obj: ObjectRef::Body(0), local_pos: start, local_vel: DVec3::ZERO, level: None, last_orient: DQuat::IDENTITY };
        for _ in 0..60 {
            f.carry(&s, dt, Motion::Ballistic, UPos::ZERO);
            f.advance(&s, dt, Motion::Ballistic);
        }
        let Frame::Attached { local_pos, .. } = f else { panic!() };
        assert!((local_pos - start).length() < 1.0, "drifted {}", (local_pos - start).length());
    }

    /// The drag's implicit step: at any speed, no overshoot past zero; a fall reaches the terminal speed and stays.
    #[test]
    fn drag_settles_at_the_terminal_speed() {
        let dt = 1.0 / 60.0;
        let gm = G * 5.97e24;
        // 0.5 x 1.225 x 0.0078.
        let k = 0.5 * 1.225 * tune::DRAG_CDA_M;
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, EARTH_R, gm)];
        with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, k, |s| {
            let start = DVec3::Y * (EARTH_R + 100e3);
            for v0 in [0.0, -30_000.0] {
                let mut f = Frame::Attached { obj: ObjectRef::Body(0), local_pos: start, local_vel: DVec3::Y * v0, level: None, last_orient: DQuat::IDENTITY };
                for _ in 0..6000 {
                    f.carry(s, dt, Motion::Ballistic, UPos::ZERO);
                    f.advance(s, dt, Motion::Ballistic);
                }
                let Frame::Attached { local_vel, .. } = f else { panic!() };
                let v = local_vel.length();
                assert!((40.0..50.0).contains(&v), "from {v0}: {v}");
            }
        });
    }

    /// A date's jump leaves a free hero with the body, within 50 radii.
    #[test]
    fn a_date_jump_carries_him_with_the_body() {
        let dt = 1.0 / 60.0;
        let start = UPos::ZERO.offset_m(DVec3::X * 1.5e11);
        let local = DVec3::Y * EARTH_R * 5.0;
        let bodies = [body(0, start, DQuat::IDENTITY, EARTH_R, 0.0)];
        let mut f = with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
            let mut f = Frame::at(start.offset_m(local));
            f.carry(s, dt, Motion::Held, UPos::ZERO);
            f
        });
        let moved = UPos::ZERO.offset_m(DVec3::new(-1.0e11, 9e10, 0.0));
        let bodies = [body(0, moved, DQuat::from_rotation_y(2.0), EARTH_R, 0.0)];
        with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
            f.carry(s, dt, Motion::Held, UPos::ZERO);
            f.advance(s, dt, Motion::Held);
            let got = f.world(s, UPos::ZERO).delta_m(moved);
            assert!((got - local).length() < 1.0, "{got}");
        });
    }

    /// Past the frames (100 radii and beyond) hovering stays in the universe's frame.
    #[test]
    fn far_from_everything_he_stays_in_the_universe() {
        let dt = 1.0 / 60.0;
        let p = UPos::ZERO.offset_m(DVec3::X * EARTH_R * 200.0);
        let mut f = Frame::at(p);
        let bodies = [body(0, UPos::ZERO, DQuat::IDENTITY, EARTH_R, 0.0)];
        with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
            f.carry(s, dt, Motion::Held, UPos::ZERO);
            assert!(f.body().is_none());
        });
        let bodies = [body(0, UPos::ZERO.offset_m(DVec3::Y * 3e6), DQuat::IDENTITY, EARTH_R, 0.0)];
        with_scene(&bodies, DVec3::ZERO, DVec3::ZERO, 0.0, |s| {
            f.carry(s, dt, Motion::Held, UPos::ZERO);
            f.advance(s, dt, Motion::Held);
            assert_eq!(f.world(s, UPos::ZERO), p);
        });
    }
}
