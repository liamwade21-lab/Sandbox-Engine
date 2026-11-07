//! The lock-on (the user, 2026-11-07: "when i am flying supersonic, instead of left click punching, it will be to select object,
//! then when you press G ... it will lock on to the planet and while you are holding W to go forward the viltrumite will speed up
//! superfast or slower depending on how far the object is away from him (he cannot select things what will take him 5 minutes of
//! irl time to travel to) and slow down so he can smoothly arrive to planets").
//!
//! In supersonic fast flight the left button selects what's under the cross: the nearest marker within `LOCK_PICK_PX` (the system's
//! bodies always have one), else the port's own pick of the sky (stars, galaxies), handed over as a scripted click. G locks on to the
//! selection (any state; G again lets go), and W then flies him there: the view swings round to it, the speed is the distance left to
//! the stop over `LOCK_TAU_S` (so it falls by e every `LOCK_TAU_S` and he eases in), at most `LOCK_TOP_MS`, climbing by doubling every
//! `LOCK_DOUBLE_S`. He stops `LOCK_ARRIVE_RADII` from its middle, in a hover. Nothing whose trip would take over `LOCK_REACH_S` can be
//! selected this way or locked on. F7's caps still hold (in air, near other bodies): this only ever asks for less.

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::upos::UPos;

use crate::camera::{CommandQueue, FlyCamera, TravelKind};
use crate::data::SeData;
use crate::messages::Messages;
use crate::mods::ModInput;
use crate::overlay::ScreenMarkers;
use crate::universe::SimTime;

use super::hero::slerp_dir;
use super::{Hero, controls, tune};

/// Frames a pick handed to the port is waited for.
const PICK_WAIT_FRAMES: u8 = 4;

/// The left button selects (not punches): supersonic fast flight, not diving. (By last frame's flow, as the readout's prompt: the
/// button's reader here and the punches in `hero_move` must agree.)
pub fn picks(hero: &Hero) -> bool {
    hero.fast && !hero.dive && controls::supersonic(&hero.flow, hero.flow.speed)
}

/// Where he stops, from the middle: `LOCK_ARRIVE_RADII` of a planet's or a moon's radius, more of a star's (closer, its glare whites
/// out the view), and at least `LOCK_ARRIVE_MIN_M` over the surface.
pub fn arrive_m(obj: ObjectRef, radius: f64) -> f64 {
    let radii = match obj {
        ObjectRef::Body(_) => tune::LOCK_ARRIVE_RADII.0,
        ObjectRef::Star(_) => tune::LOCK_ARRIVE_RADII.1,
        _ => tune::LOCK_ARRIVE_RADII.2,
    };
    (radius * radii).max(radius + tune::LOCK_ARRIVE_MIN_M)
}

/// Where a target is now, and how far from its middle he stops.
pub fn stop_of(eph: &Ephemeris, obj: ObjectRef, jd: f64) -> (UPos, f64) {
    (eph.position(obj, jd), arrive_m(obj, eph.radius_m(obj)))
}

/// Close enough to the stop to call it arrived.
pub fn done_m(arrive: f64) -> f64 {
    (arrive * tune::LOCK_DONE_SHARE).max(1.0)
}

/// The speed the lock asks for with `remaining` metres to the stop.
pub fn lock_speed(remaining: f64) -> f64 {
    (remaining.max(0.0) / tune::LOCK_TAU_S).min(tune::LOCK_TOP_MS)
}

/// The speed one step on: never over what the lock asks for or `cap` (braked down to it with F7's time constant); with `go` climbing
/// towards it, doubling every `LOCK_DOUBLE_S` (at least `LOCK_ACCEL_MIN` m/s²); without, held.
pub fn step_speed(speed: f64, remaining: f64, go: bool, cap: f64, dt: f64) -> f64 {
    let want = lock_speed(remaining).min(cap);
    if speed > want {
        return want + (speed - want) * (-dt / tune::CAP_BRAKE_TAU_S).exp();
    }
    if !go {
        return speed;
    }
    (speed * 2f64.powf(dt / tune::LOCK_DOUBLE_S)).max(speed + tune::LOCK_ACCEL_MIN * dt).min(want)
}

/// Fast flight one step on, locked (frame-relative velocity, universe axes): the heading turns towards `aim` at `steer` (1/s); W
/// (`go`) speeds him up by `step_speed`, turning back towards it he slows as he turns; S halves the speed as in fast flight.
#[allow(clippy::too_many_arguments)]
pub fn fly(v: DVec3, aim: DVec3, go: bool, brake: bool, steer: f64, cap: f64, remaining: f64, dt: f64) -> DVec3 {
    let speed = v.length();
    let now = v.try_normalize().unwrap_or(aim);
    let dir = slerp_dir(now, aim, 1.0 - (-steer * dt).exp());
    let mut new_speed = step_speed(speed, remaining, go && !brake, cap, dt);
    if go && now.dot(aim) < 0.0 {
        new_speed = new_speed.min(speed * 0.5f64.powf(dt / tune::BRAKE_HALF_S));
    }
    if brake {
        new_speed *= 0.5f64.powf(dt / tune::S_BRAKE_HALF_S);
    }
    dir * new_speed.min(tune::GUARD_MS)
}

/// How long the trip takes from `distance` (to the target's middle) at `speed`, W held all the way (without F7's caps or the target's
/// motion: an estimate). `None` past `LOCK_REACH_S`.
pub fn trip_time(distance: f64, arrive: f64, speed: f64) -> Option<f64> {
    let done = done_m(arrive);
    let ease_from = tune::LOCK_TOP_MS * tune::LOCK_TAU_S;
    let dt = 1.0 / 30.0;
    let (mut left, mut v, mut t) = (distance - arrive, speed.max(0.0), 0.0);
    while left > done {
        if t > tune::LOCK_REACH_S {
            return None;
        }
        // At the top: straight on to where it eases in.
        if v >= tune::LOCK_TOP_MS * 0.999 && left > ease_from * 1.001 {
            t += (left - ease_from) / v;
            left = ease_from;
            continue;
        }
        v = step_speed(v, left, true, f64::INFINITY, dt);
        left -= v * dt;
        t += dt;
    }
    (t <= tune::LOCK_REACH_S).then_some(t)
}

/// The way to a locked target from `pos`.
#[derive(Clone, Copy, Debug)]
pub struct Way {
    /// Where to fly (unit, universe axes): straight at it, or over the horizon of the body he's near while that's in the way.
    pub heading: DVec3,
    /// To its middle, and to the stop.
    pub distance: f64,
    pub remaining: f64,
    pub arrive: f64,
}

/// `arrive`: from `stop_of`. `near`: the body he's near (its middle, its radius with its air), unless that's the target.
pub fn way(target: UPos, arrive: f64, pos: UPos, near: Option<(UPos, f64)>) -> Way {
    let to = target.delta_m(pos);
    let distance = to.length();
    let straight = to.try_normalize().unwrap_or(DVec3::Z);
    let mut heading = straight;
    if let Some((c, r)) = near {
        let to_c = c.delta_m(pos);
        let along = to_c.dot(straight);
        // The line passes through the body (and its air, a little margin) before it gets there: over its horizon first.
        if along > 0.0 && along < distance && (to_c - straight * along).length() < r * 1.02 {
            let up = (-to_c).try_normalize().unwrap_or(DVec3::Z);
            let flat = (straight - up * straight.dot(up)).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
            let a = tune::LOCK_CLIMB_DEG.to_radians();
            heading = (flat * a.cos() + up * a.sin()).normalize();
        }
    }
    Way { heading, distance, remaining: distance - arrive, arrive }
}

/// How far, in what reads: m and km, AU, light years.
pub fn format_distance(m: f64) -> String {
    let m = m.max(0.0);
    if m < 1.0e9 {
        super::readout::format_height(m)
    } else if m < 1.0e15 {
        format!("{:.2} AU", m / 1.495_978_707e11)
    } else {
        format!("{:.2} ly", m / 9.460_730_472_580_8e15)
    }
}

/// "12 s", "3 min 20 s".
pub fn format_time(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s < 60 { format!("{s} s") } else { format!("{} min {:02} s", s / 60, s % 60) }
}

/// A selection the port's sky pick is making for him: the one before it, frames left to wait.
#[derive(Clone, Copy, Debug)]
pub struct PickWait {
    pub before: Option<ObjectRef>,
    pub frames: u8,
}

/// Whether he can go to `obj` from where he is: its trip time, or why not.
fn reach(hero: &Hero, eph: &Ephemeris, jd: f64, obj: ObjectRef) -> Result<f64, String> {
    let (at, arrive) = stop_of(eph, obj, jd);
    let to = at.delta_m(hero.world.0).length();
    if to - arrive <= done_m(arrive) {
        return Err("already there".into());
    }
    trip_time(to, arrive, hero.flow.speed).ok_or_else(|| format!("over {} away", format_time(tune::LOCK_REACH_S)))
}

/// The left button in supersonic flight, the waits for the port's pick, G, and the notes `hero_move` leaves. Before `hero_move`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lock_keys(
    input: Res<ModInput>,
    mut hero: ResMut<Hero>,
    mut cam: ResMut<FlyCamera>,
    markers: Res<ScreenMarkers>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut script: ResMut<CommandQueue>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    mut msg: ResMut<Messages>,
) {
    let hero = &mut *hero;
    if let Some(note) = hero.note.take() {
        msg.show(note);
    }
    let eph = Ephemeris::new(&data.catalog);
    let name = |o: ObjectRef| data.catalog.display_name(o).to_string();
    let travelling = cam.travel.as_ref().is_some_and(|t| matches!(t.kind, TravelKind::Goto | TravelKind::Land));
    // The port's pick made a selection: too far, and it's undone.
    if let Some(mut wait) = hero.pick_wait.take() {
        if cam.selected != wait.before {
            if let Some(obj) = cam.selected {
                match reach(hero, &eph, sim.jd, obj) {
                    Ok(t) => msg.show(format!("{}: {} away. G: lock on", name(obj), format_time(t))),
                    Err(why) => {
                        info!("viltrumite: {} picked in the sky, too far: {why}", name(obj));
                        msg.show(format!("{} is too far: {why}", name(obj)));
                        cam.selected = wait.before;
                    }
                }
            }
        } else if wait.frames > 0 {
            wait.frames -= 1;
            hero.pick_wait = Some(wait);
        }
    }
    if input.left_pressed && picks(hero) && !travelling {
        let Ok(window) = windows.single() else { return };
        let centre = Vec2::new(window.width(), window.height()) * 0.5;
        let px = tune::LOCK_PICK_PX * (window.height() / 1080.0).max(0.5);
        let best = markers.items.iter().map(|m| (m.obj, m.screen.distance(centre))).filter(|(_, d)| *d < px).min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((obj, _)) => match reach(hero, &eph, sim.jd, obj) {
                Ok(t) => {
                    cam.selected = Some(obj);
                    msg.show(format!("{}: {} away. G: lock on", name(obj), format_time(t)));
                    info!("viltrumite: selected {} under the cross ({:.0} s away)", name(obj), t);
                }
                Err(why) => {
                    info!("viltrumite: {} under the cross, too far: {why}", name(obj));
                    msg.show(format!("{} is too far: {why}", name(obj)));
                }
            },
            None => {
                // Stars and galaxies: the port's own pick, through the ray of the cross.
                hero.pick_wait = Some(PickWait { before: cam.selected, frames: PICK_WAIT_FRAMES });
                script.click = Some(centre);
            }
        }
    }
    if input.g_pressed && !input.shift && !input.ctrl && !input.alt && !travelling {
        // G again (or with nothing else selected) lets go; with something else selected, locks on to that.
        let release = hero.lock.filter(|&l| cam.selected.is_none_or(|s| s == l));
        match (release, cam.selected) {
            (Some(l), _) => {
                hero.lock = None;
                msg.show(format!("Lock on {} released", name(l)));
            }
            (None, Some(obj)) => match reach(hero, &eph, sim.jd, obj) {
                Ok(t) => {
                    hero.lock = Some(obj);
                    msg.show(format!("Locked on {}: {}. Hold W to fly there", name(obj), format_time(t)));
                    info!("viltrumite: locked on {} ({:.0} s)", name(obj), t);
                }
                Err(why) => {
                    info!("viltrumite: can't lock on {}: {why}", name(obj));
                    msg.show(format!("Can't lock on {}: {why}", name(obj)));
                }
            },
            (None, None) => msg.show("Nothing selected: in supersonic flight the left button selects what's under the cross".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speed_is_the_distance_left_over_its_time_up_to_the_top() {
        assert!((lock_speed(6.0e6) - 6.0e6 / tune::LOCK_TAU_S).abs() < 1e-6);
        assert_eq!(lock_speed(1e30), tune::LOCK_TOP_MS);
        assert_eq!(lock_speed(-5.0), 0.0);
        // Climbing: doubling every LOCK_DOUBLE_S, never past what it asks for.
        let mut v = 1_000.0;
        for _ in 0..30 {
            v = step_speed(v, 1e20, true, f64::INFINITY, tune::LOCK_DOUBLE_S / 30.0);
        }
        assert!((v / 2_000.0 - 1.0).abs() < 1e-9, "{v}");
        assert!(step_speed(1e9, 3.0, true, f64::INFINITY, 0.01) < 1e9);
        // Let go of W: held, unless that's more than it asks for here.
        assert_eq!(step_speed(5_000.0, 1e20, false, f64::INFINITY, 0.1), 5_000.0);
        // F7's cap still holds.
        assert!(step_speed(1e6, 1e20, true, 30_000.0, 1.0) < 30_100.0);
    }

    #[test]
    fn he_eases_in_and_arrives_without_passing_it() {
        // Earth to the Moon, from the barrier's speed: about ten seconds, the last of them easing in.
        let (d, arrive) = (3.84e8, arrive_m(ObjectRef::Body(0), 1.737e6));
        let t = trip_time(d, arrive, 340.0).expect("the Moon is in reach");
        assert!((6.0..20.0).contains(&t), "{t}");
        let (mut left, mut v, mut most) = (d - arrive, 340.0, 0.0f64);
        let dt = 1.0 / 60.0;
        for _ in 0..(60.0 * t * 1.5) as usize {
            v = step_speed(v, left, true, f64::INFINITY, dt);
            left -= v * dt;
            most = most.max(v);
            assert!(left > 0.0, "passed the stop");
        }
        assert!(left <= done_m(arrive), "{left}");
        // Going faster than light on the way.
        assert!(most > 1.0e8, "{most}");
    }

    #[test]
    fn five_minutes_is_the_reach() {
        let ly = 9.4607e15;
        let star = arrive_m(ObjectRef::Star(0), 6.96e8);
        assert!(trip_time(4.37 * ly, star, 340.0).is_some_and(|t| t < 60.0), "Alpha Centauri");
        assert!(trip_time(80.0 * ly, star, 340.0).is_some(), "80 light years");
        assert!(trip_time(100.0 * ly, star, 340.0).is_none(), "100 light years");
        assert!(trip_time(2.5e6 * ly, arrive_m(ObjectRef::Galaxy(0), 1e21), 340.0).is_none(), "Andromeda");
        // Neptune from Earth, about 30 AU.
        assert!(trip_time(4.5e12, arrive_m(ObjectRef::Body(0), 2.46e7), 340.0).is_some_and(|t| t < 40.0));
    }

    #[test]
    fn a_body_in_the_way_is_climbed_over() {
        let r = 6.371e6;
        let c = UPos::ZERO;
        // Standing on top of it, the target straight through it: up over the horizon.
        let pos = c.offset_m(DVec3::Z * (r + 1_000.0));
        let target = c.offset_m(DVec3::NEG_Z * 4.0e8);
        let w = way(target, arrive_m(ObjectRef::Body(0), 1.7e6), pos, Some((c, r)));
        let climb = w.heading.dot(DVec3::Z).asin().to_degrees();
        assert!((climb - tune::LOCK_CLIMB_DEG).abs() < 1.0, "{climb}");
        // A target over the horizon: straight at it.
        let target = c.offset_m(DVec3::new(4.0e8, 0.0, 2.0e8));
        let w = way(target, arrive_m(ObjectRef::Body(0), 1.7e6), pos, Some((c, r)));
        assert!(w.heading.dot(target.delta_m(pos).normalize()) > 0.9999);
        assert!((w.remaining - (w.distance - arrive_m(ObjectRef::Body(0), 1.7e6))).abs() < 1e-6);
    }

    #[test]
    fn times_read_plainly() {
        assert_eq!(format_time(12.4), "12 s");
        assert_eq!(format_time(200.0), "3 min 20 s");
        assert_eq!(format_distance(3.2e6), "3200 km");
        assert_eq!(format_distance(4.5e12), "30.08 AU");
        assert_eq!(format_distance(4.13e16), "4.37 ly");
    }
}
