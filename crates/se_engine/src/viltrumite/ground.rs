//! The ground under him (spec F6): one height for walking, the feet and the shadow, its normal, walking on it, and where
//! his drawn feet go.
//!
//! His capsule's middle is `Scene::stand_m` over `support_height`, followed exactly as he walks. What's drawn goes by
//! [`drawn_lift`]: the clip's own soles on that height (`Clip::ground_z`), a finer node's change eased out (`Hero.ease`), and
//! the root dropped to the lower foot's ground. Two-bone IK later goes in [`post_pose`], after the pose and before the
//! palette; it's the one place that knows both.

use bevy::math::{Affine3A, DVec3, Mat3A, Vec3, Vec3A};
use se_core::catalog::ObjectRef;
use se_core::hero_asset::{ClipSet, HeroModel};

use crate::ship::{BodyInfo, ground_on};
use crate::volumetric_clouds::giant_floor_m;

use super::Hero;
use super::frames::{Frame, Ground, Scene, find};
use super::tune;

/// What he'd come down on at a point of the body's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Footing {
    /// He can stand on it.
    Land,
    /// The sea's surface: he never goes under, and hovers.
    Sea,
    /// A gas giant, a star, a black hole, a model body: a stop, nothing to stand on.
    Nothing,
}

/// The height he and the camera stop at (F6's one `support_height`), at a point of the body's frame: `ground_on`, which
/// clamps to the sea, but a star's or a sphere's surface is known without loading, and a giant has its own floor, the
/// camera's, only where its decks are drawn (`decks`: Edge cases). A star's stop is also the camera's distance further
/// out (`camera_m`), as the camera's kept outside it. From step 9, a building slice's top when his feet are within 0.7 m
/// of it goes here too.
pub fn support_height(b: &BodyInfo, local: DVec3, decks: Option<ObjectRef>, camera_m: f64) -> Ground {
    let Some(t) = b.terrain else {
        // (Never inside the plain sphere `keep_camera_above_ground` keeps the camera out of: pushed out, the camera would
        // pull him to it, as if something else had moved it.)
        let k = (local / b.shape).try_normalize().map_or(1.0, |d| (d * b.shape).length());
        let extra = if matches!(b.obj, ObjectRef::Star(_)) { camera_m } else { 0.0 };
        return Ground { radial: b.radius * (1.0 + tune::SPHERE_STOP_SHARE) * k.max(1.0) + extra, level: Some(0) };
    };
    if t.proc.kind.is_giant() && decks == Some(b.obj) {
        if let Some(floor) = giant_floor_m(t) {
            let k = (local / b.shape).try_normalize().map_or(1.0, |d| (d * b.shape).length());
            return Ground { radial: (b.radius + floor) * k + tune::GIANT_STOP_M, level: Some(0) };
        }
    }
    let (radial, level) = ground_on(b.radius, b.shape, Some(t), local);
    Ground { radial, level }
}

/// What's there to stand on.
pub fn footing(b: &BodyInfo, local: DVec3) -> Footing {
    let Some(t) = b.terrain else { return Footing::Nothing };
    if t.proc.kind.is_giant() {
        return Footing::Nothing;
    }
    if t.has_ocean {
        let dir = (local / b.shape).try_normalize().unwrap_or(DVec3::Y);
        if t.ground_height(dir).is_some_and(|h| h < 0.0) {
            return Footing::Sea;
        }
    }
    Footing::Land
}

/// Two directions along the ground at `up`.
fn tangents(up: DVec3) -> (DVec3, DVec3) {
    let e1 = up.any_orthonormal_vector();
    (e1, up.cross(e1))
}

/// Three points of the ground round `local`, a triangle of side `span` (m), `None` where any isn't loaded.
pub fn ground_triangle(ground: impl Fn(DVec3) -> Ground, local: DVec3, span: f64) -> Option<[DVec3; 3]> {
    let up = local.normalize_or(DVec3::Y);
    let (e1, e2) = tangents(up);
    let r = span / 3f64.sqrt();
    let mut out = [DVec3::ZERO; 3];
    for (k, q) in out.iter_mut().enumerate() {
        let a = k as f64 * std::f64::consts::TAU / 3.0;
        let at = local + (e1 * a.cos() + e2 * a.sin()) * r;
        let g = ground(at);
        g.level?;
        *q = at.normalize_or(up) * g.radial;
    }
    Some(out)
}

/// The ground's normal at `local`, from three samples `NORMAL_SPAN_M` apart (body frame); straight up where one isn't loaded.
pub fn normal_at(s: &Scene, b: &BodyInfo, local: DVec3) -> DVec3 {
    let up = local.normalize_or(DVec3::Y);
    let Some([p0, p1, p2]) = ground_triangle(|p| (s.ground)(b, p), local, tune::NORMAL_SPAN_M) else { return up };
    let n = (p1 - p0).cross(p2 - p0).normalize_or(up);
    if n.dot(up) < 0.0 { -n } else { n }
}

/// What a walking step's keys say.
#[derive(Clone, Copy, Debug, Default)]
pub struct WalkIn {
    /// Where he's asked to go, body frame (zero: stop).
    pub wish: DVec3,
    /// How fast (m/s).
    pub speed: f64,
    /// Gravity here (m/s2).
    pub gravity: f64,
}

/// What a walking step came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WalkEvent {
    #[default]
    Walked,
    /// The ground fell away: he's falling, at the velocity he had.
    Ledge,
    /// He stepped off the shore. (He's left on the sea's surface, and his velocity is gone.)
    Sea,
    /// Nothing is loaded under him: held in the air.
    Held,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WalkOut {
    pub event: WalkEvent,
    /// A finer node moved the ground: what's to be added to the visual offset, so what's drawn doesn't jump (m).
    pub ease: f64,
    pub slope_deg: f64,
    /// On ground too steep to stand on.
    pub sliding: bool,
    /// Couldn't go on uphill.
    pub blocked: bool,
}

/// The slope at a point (degrees), and which way is downhill along the ground.
fn slope_there(n: DVec3, up: DVec3) -> (f64, DVec3) {
    let cos = n.dot(up).clamp(-1.0, 1.0);
    (cos.acos().to_degrees(), (-up + n * cos).normalize_or_zero())
}

/// One frame on foot. Stays in the body's frame, so the ground can't wobble against him (F5).
pub fn walk_step(frame: &mut Frame, s: &Scene, i: &WalkIn, dt: f64) -> WalkOut {
    let mut out = WalkOut::default();
    let Frame::Attached { obj, local_pos, local_vel, level, .. } = frame else {
        out.event = WalkEvent::Ledge;
        return out;
    };
    let Some(b) = find(s, *obj) else {
        out.event = WalkEvent::Ledge;
        return out;
    };
    let p0 = *local_pos;
    let here = (s.ground)(b, p0);
    // F2: nothing loaded: held where he is, never the ellipsoid.
    if here.level.is_none() {
        *local_vel = DVec3::ZERO;
        *level = None;
        out.event = WalkEvent::Held;
        return out;
    }
    let up = p0.normalize_or(DVec3::Y);
    // He stood on the ground as it was: if it's moved under him it's a finer node, whatever the distance.
    let stand0 = here.radial + s.stand_m;
    out.ease = p0.length() - stand0;
    let n = normal_at(s, b, p0);
    let (slope_deg, downhill) = slope_there(n, up);
    out.slope_deg = slope_deg;
    out.sliding = slope_deg > tune::SLOPE_WALK_DEG;

    let wish = (i.wish - n * i.wish.dot(n)).normalize_or_zero();
    let target = wish * i.speed;
    let mut v = *local_vel - n * local_vel.dot(n);
    let reach = tune::WALK_ACCEL * dt;
    if out.sliding {
        // Too steep: he goes down it whatever the keys, and can only steer across. (No friction on the way down: `reach` works
        // on what isn't downhill.)
        let down = v.dot(downhill).max(0.0);
        let across = v - downhill * v.dot(downhill);
        let want = target - downhill * target.dot(downhill);
        let across = across + (want - across).clamp_length_max(reach);
        let down = (down + i.gravity * slope_deg.to_radians().sin() * tune::SLIDE_SHARE * dt).min(tune::SLIDE_MAX_MS);
        v = across + downhill * down;
    } else {
        v += (target - v).clamp_length_max(reach);
    }

    // Uphill onto what's too steep to walk: that part of the step goes (walk to 50 degrees; above, it's a slide or a wall).
    let mut p1 = p0 + v * dt;
    let mut g1 = (s.ground)(b, p1);
    if g1.radial > here.radial {
        let up1 = p1.normalize_or(up);
        let (slope1, down1) = slope_there(normal_at(s, b, p1), up1);
        let climb = v.dot(-down1);
        if slope1 > tune::SLOPE_WALK_DEG && climb > 0.0 {
            out.blocked = true;
            v += down1 * climb;
            p1 = p0 + v * dt;
            g1 = (s.ground)(b, p1);
            // Along the contour is all that's left; if even that climbs too fast, he stays put.
            let limit = v.length() * dt * tune::SLOPE_WALK_DEG.to_radians().tan();
            if g1.radial - here.radial > limit.max(1e-3) {
                p1 = p0;
                g1 = here;
                v = DVec3::ZERO;
            }
        }
    }
    if g1.level.is_none() {
        // The edge of what's loaded: he waits there.
        *local_vel = DVec3::ZERO;
        out.event = WalkEvent::Held;
        return out;
    }
    if (s.footing)(b, p1) == Footing::Sea {
        *local_pos = p1.normalize_or(up) * (g1.radial + s.stand_m);
        *local_vel = DVec3::ZERO;
        *level = g1.level;
        out.event = WalkEvent::Sea;
        return out;
    }
    let drop = here.radial - g1.radial;
    if g1.level == here.level && drop > tune::LEDGE_DROP_M + v.length() * dt * tune::SLOPE_WALK_DEG.to_radians().tan() {
        // Off a ledge: from where his feet were, with what speed he had.
        *local_pos = p1.normalize_or(up) * (here.radial + s.stand_m);
        *local_vel = v;
        out.event = WalkEvent::Ledge;
        return out;
    }
    if g1.level != here.level {
        // Onto a neighbour of another level: its triangles meet this one's only at the skirts. What the step along this
        // slope didn't lead to is eased too.
        out.ease += here.radial + (p1.length() - p0.length()) - g1.radial;
    }
    *local_pos = p1.normalize_or(up) * (g1.radial + s.stand_m);
    *local_vel = v;
    *level = g1.level;
    out
}

/// The three kinds of touchdown (Numbers to tune). `into`: his speed into the ground; `speed`: over it. A fast glancing touch is
/// still a crash: at that speed the ground doesn't care for the angle. A slow one is how much of it went into the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    Soft,
    Short,
    Hard,
}

pub fn landing_of(into: f64, speed: f64) -> Landing {
    let impact = if speed >= tune::HARD_LANDING_MS { speed } else { into };
    if impact < tune::SOFT_LANDING_MS {
        Landing::Soft
    } else if impact < tune::HARD_LANDING_MS {
        Landing::Short
    } else {
        Landing::Hard
    }
}

// ---- His feet ------------------------------------------------------------------------------------------------------

/// How many points of each sole are followed.
const SOLE_POINTS: usize = 8;

/// A few points under each foot, each rigid with its foot or toe bone, found once from his mesh: the lowest in his bind
/// pose (feet flat), at the heel, the ball and either side of it. (The mesh's vertices go once it's on the GPU.)
#[derive(Clone, Copy, Debug)]
pub struct Soles {
    sides: [[(u16, Vec3); SOLE_POINTS]; 2],
}

impl Soles {
    pub fn of(model: &HeroModel) -> Option<Soles> {
        let bone = |name: &str| model.bones.iter().position(|b| b.name == name);
        let mut sides = [[(0u16, Vec3::ZERO); SOLE_POINTS]; 2];
        for (side, name) in sides.iter_mut().zip(["L", "R"]) {
            let (foot, toe) = (bone(&format!("SKEL_{name}_Foot"))?, bone(&format!("SKEL_{name}_Toe0"))?);
            for (pair, b) in side.chunks_mut(SOLE_POINTS / 2).zip([foot, toe]) {
                let mut pts: Vec<Vec3> = Vec::new();
                for d in model.draws.iter().filter(|d| !d.cutout() && !d.cape()) {
                    let vs = &model.vertices[d.first_vertex as usize..(d.first_vertex + d.vertex_count) as usize];
                    // Mostly on this bone.
                    pts.extend(vs.iter().filter(|v| v.joints.iter().zip(v.weights).any(|(&j, w)| j as usize == b && w >= 128)).map(|v| Vec3::from(v.position)));
                }
                let low = pts.iter().map(|p| p.z).fold(f32::MAX, f32::min);
                pts.retain(|p| p.z < low + 0.012);
                let by = |key: fn(&Vec3) -> f32, sign: f32| pts.iter().copied().max_by(|a, c| (key(a) * sign).total_cmp(&(key(c) * sign)));
                let picks = [by(|p| p.x, 1.0)?, by(|p| p.x, -1.0)?, by(|p| p.y, 1.0)?, by(|p| p.y, -1.0)?];
                let inv = Affine3A::from_mat4(model.bones[b].inverse_bind);
                for (slot, p) in pair.iter_mut().zip(picks) {
                    *slot = (b as u16, Vec3::from(inv.transform_point3a(Vec3A::from(p))));
                }
            }
        }
        Some(Soles { sides })
    }

    /// Each foot's lowest point in model space (left, right).
    pub fn lowest(&self, world: impl Fn(usize) -> Affine3A) -> [Vec3; 2] {
        self.sides.map(|side| {
            side.iter().map(|&(b, p)| Vec3::from(world(b as usize).transform_point3a(Vec3A::from(p)))).min_by(|a, c| a.z.total_cmp(&c.z)).unwrap_or(Vec3::ZERO)
        })
    }
}

/// What the feet came to in a frame, for `hero feet` and the root's drop.
#[derive(Clone, Copy, Debug, Default)]
pub struct Feet {
    /// Left and right sole over the ground under it, as drawn (m).
    pub gap: [f64; 2],
    /// The lower foot's ground under the ground at his middle (m).
    pub lower: f64,
}

/// Where the soles are against the ground, given the model's turn into the body's frame and how far his root is drawn over
/// the capsule's middle (along `up`).
pub fn feet_on(s: &Soles, world: impl Fn(usize) -> Affine3A, to_local: Mat3A, middle: DVec3, up_lift: f64, ground: impl Fn(DVec3) -> Ground) -> Feet {
    let up = middle.normalize_or(DVec3::Y);
    let mid = ground(middle).radial;
    let mut f = Feet::default();
    for (k, p) in s.lowest(world).into_iter().enumerate() {
        let d = Vec3::from(to_local * Vec3A::from(p)).as_dvec3();
        let at = middle + up * up_lift + d;
        let g = ground(at).radial;
        f.gap[k] = at.length() - g;
        f.lower = f.lower.max(mid - g);
    }
    f
}

/// After the pose is sampled, before the palette: how far along his up his drawn root goes from the capsule's middle (m).
/// The clip's soles on the ground under him, a finer node's change eased out, and (F6) the root down to the lower foot's
/// ground, at most `ROOT_DROP_MAX_M`, smoothed. Two-bone IK would go here: it has the pose and the ground.
pub fn post_pose(hero: &mut Hero, clips: &ClipSet, body: Option<&(BodyInfo, DVec3)>, dt: f64) -> f64 {
    let on_foot = hero.state.on_foot();
    let ground_z = hero.sampler.layers[0].clip.and_then(|i| clips.clips.get(i as usize)).map(|k| k.ground_z as f64);
    let target = match ground_z.filter(|_| on_foot) {
        // (`ground_z` is where the clip's lowest sole gets to: his root's that far over the ground.)
        Some(z) => -z - tune::STAND_M,
        None => 0.0,
    };
    hero.clip_lift += (target - hero.clip_lift) * (1.0 - (-dt / tune::CLIP_GROUND_TAU_S).exp());
    let lift = hero.clip_lift + hero.ease - hero.root_drop;
    let sampler = &hero.sampler;
    let feet = match (body, &hero.soles) {
        (Some((b, local)), Some(soles)) if on_foot || hero.feet.is_some() => {
            let to_local = Mat3A::from_quat((b.orient.inverse() * hero.rot).as_quat()) * Mat3A::from_cols(Vec3A::X, Vec3A::new(0.0, 0.0, -1.0), Vec3A::Y);
            Some(feet_on(soles, |i| sampler.world(i), to_local, *local, lift, |p| support_height(b, p, None, 0.0)))
        }
        _ => None,
    };
    let want = feet.filter(|_| on_foot).map_or(0.0, |f| f.lower.clamp(0.0, tune::ROOT_DROP_MAX_M));
    let was = hero.root_drop;
    hero.root_drop += (want - hero.root_drop) * (1.0 - (-dt / tune::ROOT_DROP_TAU_S).exp());
    let lift = hero.clip_lift + hero.ease - hero.root_drop;
    if let (Some(f), Some(trace)) = (feet, hero.feet.as_mut()) {
        let (level, height) = match (hero.frame, body) {
            (Frame::Attached { level, local_pos, .. }, Some((b, _))) => (level.map_or(-1, |l| l as i32), local_pos.length() - support_height(b, local_pos, None, 0.0).radial),
            _ => (-1, 0.0),
        };
        let moved = hero.root_drop - was;
        let l0 = sampler_time(&hero.sampler);
        trace.write(&format!(
            "{:.4},{:?},{:?},{:.3},{:.3},{},{:.1},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}\n",
            hero.clock,
            hero.state,
            hero.director.pose,
            l0,
            hero.readout.speed,
            level,
            hero.slope_deg,
            f.gap[0] - moved,
            f.gap[1] - moved,
            f.lower,
            hero.root_drop,
            hero.clip_lift,
            hero.ease,
            height,
        ));
    }
    lift
}

fn sampler_time(s: &super::anim::Sampler) -> f32 {
    s.layers[0].time
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::upos::UPos;

    use bevy::math::DQuat;

    /// A body's ground as a function of the point.
    fn world<R>(radius: f64, ground: impl Fn(DVec3) -> Ground, foot: impl Fn(DVec3) -> Footing, f: impl FnOnce(&Scene, &BodyInfo) -> R) -> R {
        let bodies = [BodyInfo { obj: ObjectRef::Body(0), pos: UPos::ZERO, orient: DQuat::IDENTITY, radius, shape: DVec3::ONE, gm: 0.0, terrain: None }];
        let (spin, vel) = (|_| DVec3::ZERO, |_| DVec3::ZERO);
        let g = |_: &BodyInfo, p: DVec3| ground(p);
        let fo = |_: &BodyInfo, p: DVec3| foot(p);
        let d = |_: &BodyInfo, _: DVec3| 0.0;
        let s = Scene { bodies: &bodies, spin: &spin, vel: &vel, ground: &g, footing: &fo, drag: &d, stand_m: 1.0 };
        f(&s, &bodies[0])
    }

    const R: f64 = 1.0e6;

    fn at(x: f64, z: f64, h: f64) -> DVec3 {
        (DVec3::new(x, R, z)).normalize() * (R + h)
    }

    fn standing(pos: DVec3) -> Frame {
        Frame::Attached { obj: ObjectRef::Body(0), local_pos: pos, local_vel: DVec3::ZERO, level: Some(9), last_orient: DQuat::IDENTITY }
    }

    fn place(f: &Frame) -> (DVec3, DVec3) {
        let Frame::Attached { local_pos, local_vel, .. } = f else { panic!() };
        (*local_pos, *local_vel)
    }

    /// The ground at x along a slope of `deg` degrees rising with +x.
    fn ramp(deg: f64) -> impl Fn(DVec3) -> Ground {
        let tan = deg.to_radians().tan();
        move |p: DVec3| {
            let x = (p / p.y).x * R;
            Ground { radial: R + x.max(0.0) * tan, level: Some(9) }
        }
    }

    fn land(_: DVec3) -> Footing {
        Footing::Land
    }

    const DT: f64 = 1.0 / 60.0;

    #[test]
    fn he_stands_exactly_on_it_and_reaches_his_speed_at_the_acceleration_asked() {
        world(R, |_| Ground { radial: R, level: Some(9) }, land, |s, _| {
            let mut f = standing(at(0.0, 0.0, 1.0));
            let i = WalkIn { wish: DVec3::X, speed: 5.0, gravity: 9.8 };
            let mut t = 0.0;
            while t < 5.0 / tune::WALK_ACCEL - DT {
                walk_step(&mut f, s, &i, DT);
                t += DT;
            }
            let (_, v) = place(&f);
            assert!((v.length() - 4.9).abs() < 0.3, "{v}");
            for _ in 0..120 {
                let o = walk_step(&mut f, s, &i, DT);
                assert_eq!(o.event, WalkEvent::Walked);
                assert!(o.ease.abs() < 1e-6, "{}", o.ease);
                assert!((place(&f).0.length() - (R + 1.0)).abs() < 1e-6);
            }
            assert!((place(&f).1.length() - 5.0).abs() < 1e-6);
            // And comes to a stop.
            for _ in 0..60 {
                walk_step(&mut f, s, &WalkIn { speed: 5.0, ..WalkIn::default() }, DT);
            }
            assert!(place(&f).1.length() < 1e-6);
        });
    }

    #[test]
    fn he_walks_up_to_50_degrees() {
        for (deg, climbs) in [(30.0, true), (48.0, true), (53.0, false), (65.0, false), (80.0, false)] {
            world(R, ramp(deg), land, |s, _| {
                let mut f = standing(at(0.5, 0.0, 1.0));
                let i = WalkIn { wish: DVec3::X, speed: 5.0, gravity: 9.8 };
                let mut most: f64 = 0.0;
                for _ in 0..240 {
                    walk_step(&mut f, s, &i, DT);
                    most = most.max(place(&f).0.length() - R);
                }
                let climbed = most > 6.0;
                assert_eq!(climbed, climbs, "{deg} degrees: reached {most}");
            });
        }
    }

    /// On a slope too steep to stand on he goes down it, and across but not up with the keys.
    #[test]
    fn he_slides_down_what_is_too_steep() {
        world(R, ramp(60.0), land, |s, _| {
            let mut f = standing(at(40.0, 0.0, 40.0 * 60f64.to_radians().tan() + 1.0));
            let start = place(&f).0.length();
            let i = WalkIn { wish: DVec3::X, speed: 5.0, gravity: 9.8 };
            let mut o = WalkOut::default();
            for _ in 0..60 {
                o = walk_step(&mut f, s, &i, DT);
            }
            assert!(o.sliding, "{o:?}");
            assert!(place(&f).0.length() < start - 1.0, "from {start} to {}", place(&f).0.length());
        });
    }

    /// F6's rule: a real drop on one level is a fall; the same drop from a finer node arriving is only eased.
    #[test]
    fn a_ledge_is_a_fall_and_a_finer_node_is_not() {
        let cliff = |p: DVec3| {
            let x = (p / p.y).x * R;
            Ground { radial: if x > 5.0 { R - 3.0 } else { R }, level: Some(9) }
        };
        world(R, cliff, land, |s, _| {
            let mut f = standing(at(4.99, 0.0, 1.0));
            if let Frame::Attached { local_vel, .. } = &mut f {
                *local_vel = DVec3::X * 3.0;
            }
            let o = walk_step(&mut f, s, &WalkIn { wish: DVec3::X, speed: 3.0, gravity: 9.8 }, DT);
            assert_eq!(o.event, WalkEvent::Ledge);
            // His feet are where they were: the fall starts there.
            assert!((place(&f).0.length() - (R + 1.0)).abs() < 1e-3);
            // (The normal's samples reach over the edge, so the plane there is a steep one: less of it stays along it.)
            assert!(place(&f).1.length() > 0.5);
        });
        // The ground under him moves down 2 m while he stands still: eased, he stays on it.
        world(R, |_| Ground { radial: R - 2.0, level: Some(10) }, land, |s, _| {
            let mut f = standing(at(0.0, 0.0, 1.0));
            let o = walk_step(&mut f, s, &WalkIn::default(), DT);
            assert_eq!(o.event, WalkEvent::Walked);
            assert!((o.ease - 2.0).abs() < 1e-6, "{}", o.ease);
            assert!((place(&f).0.length() - (R - 1.0)).abs() < 1e-6);
        });
    }

    #[test]
    fn nothing_loaded_holds_him() {
        world(R, |_| Ground { radial: R - 500.0, level: None }, land, |s, _| {
            let mut f = standing(at(0.0, 0.0, 1.0));
            let before = place(&f).0;
            let o = walk_step(&mut f, s, &WalkIn { wish: DVec3::X, speed: 5.0, gravity: 9.8 }, DT);
            assert_eq!(o.event, WalkEvent::Held);
            assert_eq!(place(&f).0, before);
        });
    }

    #[test]
    fn stepping_into_the_sea_says_so() {
        let shore = |p: DVec3| if (p / p.y).x * R > 1.0 { Footing::Sea } else { Footing::Land };
        world(R, |_| Ground { radial: R, level: Some(9) }, shore, |s, _| {
            let mut f = standing(at(0.99, 0.0, 1.0));
            if let Frame::Attached { local_vel, .. } = &mut f {
                *local_vel = DVec3::X * 5.0;
            }
            let o = walk_step(&mut f, s, &WalkIn { wish: DVec3::X, speed: 5.0, gravity: 9.8 }, DT);
            assert_eq!(o.event, WalkEvent::Sea);
            assert_eq!(place(&f).1, DVec3::ZERO);
        });
    }

    #[test]
    fn the_normal_is_the_slopes() {
        world(R, ramp(30.0), land, |s, b| {
            let n = normal_at(s, b, at(20.0, 0.0, 12.0));
            let angle = n.dot(DVec3::Y).acos().to_degrees();
            assert!((angle - 30.0).abs() < 0.1, "{angle}");
            assert!(n.x < 0.0, "{n}");
        });
    }

    #[test]
    fn landings_go_by_the_table() {
        assert_eq!(landing_of(5.0, 5.0), Landing::Soft);
        assert_eq!(landing_of(11.9, 20.0), Landing::Soft);
        assert_eq!(landing_of(12.0, 20.0), Landing::Short);
        assert_eq!(landing_of(34.0, 34.0), Landing::Short);
        assert_eq!(landing_of(35.0, 35.0), Landing::Hard);
        // A fast glancing touch.
        assert_eq!(landing_of(2.0, 3000.0), Landing::Hard);
    }

    /// His real soles: they rest on a little under the middle, and a clip's `ground_z` says the same as the points do.
    #[test]
    fn his_soles_are_found_on_his_real_mesh() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/mods/viltrumite");
        let (Ok(mb), Ok(cb)) = (std::fs::read(root.join("omniman.sevm")), std::fs::read(root.join("clips.sevc"))) else { return };
        let m = se_core::hero_asset::parse_model(&mb).unwrap();
        let c = se_core::hero_asset::parse_clips(&cb).unwrap();
        let soles = Soles::of(&m).unwrap();
        let mut s = Box::<crate::viltrumite::anim::Sampler>::default();
        for name in ["idle", "walk", "run", "sprint"] {
            let i = c.find(name).unwrap();
            let clip = &c.clips[i];
            let mut lowest = f32::MAX;
            for k in 0..clip.frames {
                s.play(i as u16, k as f32 / c.fps);
                s.sample(&m, &c);
                let [l, r] = soles.lowest(|b| s.world(b));
                lowest = lowest.min(l.z).min(r.z);
            }
            assert!((lowest - clip.ground_z).abs() < 0.02, "{name}: {lowest} against {}", clip.ground_z);
        }
    }
}
