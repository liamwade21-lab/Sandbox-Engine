//! Hyperspace (step 5.3): the jump between SpaceEngine's stars, and its tunnel.
//!
//! A jump holds the ship's universe pose (the arrival point by the target star, facing it) while the port's own system loader
//! (`system.rs` `select_active_system`, which follows the camera's nearest star and builds its system in the background) swaps
//! the systems: the camera waits at the arrival point, and the tunnel ends once the target's system is the active one and its
//! star's body is spawned (G4), and not before the tunnel's measured length. Then the ship's parked pose is re-parented to that star and it goes on in supercruise.
//! Until then `fly` and `seat_camera` stand aside (the old system's bodies are gone, and with them the frame the ship was in).

use bevy::math::{DQuat, DVec3, Vec3};
use bevy::prelude::*;
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::upos::UPos;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::mods::ModControl;
use crate::sandbox::{Mode, Sandbox};
use crate::system::{ActiveSystem, SystemBody};
use crate::universe::SimTime;

use super::pilot::{Cruise, Pilot};
use super::seat::Seat;
use super::{Sidewinder, SidewinderAssets};

/// A jump in progress: the tunnel.
#[derive(Clone, Copy, Debug)]
pub struct Jump {
    pub target: ObjectRef,
    /// The target's system's root (what `ActiveSystem::root` reads once it is entered).
    pub root: u32,
    pub arrive: UPos,
    /// The ship's turn at the arrival (facing the star), model space to universe axes.
    pub rot: DQuat,
    pub seconds: f64,
    pub frames: u32,
}

/// A target's state: `Ok((the angle between the nose and it in degrees, the jump's fuel in tonnes))`, or why it can't be
/// jumped to (the fuel needed past the drive's most, or more than the main tank holds; a body of this system in the way; the
/// system the ship is in). `here` is the current system's root, `bodies` its bodies' centres from the ship and radii.
#[allow(clippy::too_many_arguments)]
pub fn check(data: &SeData, jd: f64, target: ObjectRef, from: UPos, nose: DVec3, mass_t: f64, main_t: f64, here: Option<u32>, bodies: &[(DVec3, f64)]) -> Result<(f64, f64), elite_sim::drive::Refusal> {
    use elite_sim::drive::Refusal;
    if data.catalog.system_root(target) == here {
        return Err(Refusal::SameSystem);
    }
    let to = Ephemeris::new(&data.catalog).position(target, jd).delta_m(from);
    let ly = to.length() / se_core::units::METERS_PER_LY;
    let fuel = elite_sim::fsd::jump_fuel(&elite_sim::data::FSD_2E, ly, mass_t).ok_or(Refusal::OutOfRange)?;
    if fuel > main_t {
        return Err(Refusal::NoFuel);
    }
    if elite_sim::hyperspace::obscured(glam::DVec3::from_array(to.to_array()), &bodies.iter().map(|(c, r)| (glam::DVec3::from_array(c.to_array()), *r)).collect::<Vec<_>>()) {
        return Err(Refusal::Obscured);
    }
    Ok((nose.angle_between(to).to_degrees(), fuel))
}

/// The catalog stars within `range_ly`, not of the system `here` (the Navigation tab's systems).
pub fn stars_within(data: &SeData, jd: f64, from: UPos, range_ly: f64, here: Option<u32>) -> Vec<ObjectRef> {
    let cat = &data.catalog;
    let eph = Ephemeris::new(cat);
    let from_pc = from.to_parsecs();
    let reach_pc = range_ly * se_core::units::METERS_PER_LY / se_core::units::METERS_PER_PC + 0.01;
    (0..cat.stars.len())
        .filter(|&i| {
            let s = &cat.stars[i];
            s.has_position && !s.barycenter && !s.hidden() && s.pos_pc.distance_squared(from_pc) < reach_pc * reach_pc
        })
        .map(|i| ObjectRef::Star(i as u32))
        .filter(|&t| cat.system_root(t) != here && eph.position(t, jd).delta_m(from).length() / se_core::units::METERS_PER_LY <= range_ly)
        .collect()
}

/// The catalog star nearest the nose (within `AHEAD_DEG`) within `range_ly`, not of the system `here`.
pub fn star_ahead(data: &SeData, jd: f64, from: UPos, nose: DVec3, range_ly: f64, here: Option<u32>) -> Option<ObjectRef> {
    const AHEAD_DEG: f64 = 15.0;
    let cat = &data.catalog;
    let eph = Ephemeris::new(cat);
    let from_pc = from.to_parsecs();
    let reach_pc = range_ly * se_core::units::METERS_PER_LY / se_core::units::METERS_PER_PC + 0.01;
    (0..cat.stars.len())
        .filter(|&i| {
            let s = &cat.stars[i];
            s.has_position && !s.barycenter && !s.hidden() && s.pos_pc.distance_squared(from_pc) < reach_pc * reach_pc
        })
        .map(|i| ObjectRef::Star(i as u32))
        .filter(|&t| cat.system_root(t) != here)
        .filter_map(|t| {
            let to = eph.position(t, jd).delta_m(from);
            let angle = nose.angle_between(to).to_degrees();
            (to.length() / se_core::units::METERS_PER_LY <= range_ly && angle <= AHEAD_DEG).then_some((t, angle))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| t)
}

/// A jump's target is a star: a barycentre (a multiple system's own name) stands for its brightest star.
pub fn as_star(data: &SeData, t: ObjectRef) -> Option<ObjectRef> {
    fn brightest(cat: &se_core::catalog::Catalog, name: &str, depth: u32) -> Option<(ObjectRef, f64)> {
        cat.children_of(name)
            .iter()
            .filter_map(|&c| match c {
                ObjectRef::Star(k) => {
                    let s = &cat.stars[k as usize];
                    if !s.barycenter {
                        Some((c, s.abs_mag))
                    } else if depth < 8 {
                        brightest(cat, &s.names[0], depth + 1)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
    let ObjectRef::Star(i) = t else { return None };
    let cat = &data.catalog;
    if !cat.stars[i as usize].barycenter {
        return Some(t);
    }
    // (The children are keyed by the parent's display name; failing that, every star of its system.)
    brightest(cat, cat.display_name(t), 0)
        .or_else(|| {
            let root = cat.system_root(t);
            (0..cat.stars.len())
                .filter(|&k| !cat.stars[k].barycenter && cat.system_root(ObjectRef::Star(k as u32)) == root)
                .map(|k| (ObjectRef::Star(k as u32), cat.stars[k].abs_mag))
                .min_by(|a, b| a.1.total_cmp(&b.1))
        })
        .map(|(c, _)| c)
}

/// The arrival point by a star, `radii` of its radius out along the line from `from`, and the turn facing it.
pub fn arrival(data: &SeData, jd: f64, from: UPos, target: ObjectRef, radii: f64) -> (UPos, DQuat) {
    let eph = Ephemeris::new(&data.catalog);
    let star = eph.position(target, jd);
    let r = eph.radius_m(target);
    let dir = star.delta_m(from).normalize_or(DVec3::NEG_Z);
    (star.offset_m(-dir * radii * r), DQuat::from_rotation_arc(DVec3::NEG_Z, dir))
}

/// The jump begins: the tunnel to `target` from the ship's universe place `from`.
pub fn begin(data: &SeData, jd: f64, pilot: &mut Pilot, from: UPos, target: ObjectRef) {
    let root = match target {
        ObjectRef::Star(i) => data.catalog.system_root(target).unwrap_or(i),
        _ => data.catalog.system_root(target).unwrap_or(0),
    };
    let (arrive, rot) = arrival(data, jd, from, target, elite_sim::hyperspace::stand_in::ARRIVAL_RADII);
    pilot.cruise = None;
    pilot.glide = None;
    pilot.jump = Some(Jump { target, root, arrive, rot, seconds: 0.0, frames: 0 });
    info!("sidewinder: hyperspace to {target:?} (root {root}), {:.3} ly", from.delta_m(arrive).length() / se_core::units::METERS_PER_LY);
}

/// The tunnel, each frame: the camera held at the arrival point (unbound: the body it was bound to goes with the old system)
/// until the target's system is entered; then the ship arrives in supercruise there.
#[allow(clippy::too_many_arguments)]
pub fn tunnel(
    time: Res<Time<Real>>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    active: Res<ActiveSystem>,
    seat: Res<Seat>,
    control: Res<ModControl>,
    sb: Res<Sandbox>,
    assets: Option<Res<SidewinderAssets>>,
    mut ship: ResMut<Sidewinder>,
    mut pilot: ResMut<Pilot>,
    mut cam: ResMut<FlyCamera>,
    bodies: Query<&SystemBody>,
    mut msg: ResMut<crate::messages::Messages>,
) {
    let (Some(assets), true) = (assets, seat.seated && control.pilot && sb.mode == Mode::Pilot) else { return };
    let Some(p) = ship.parked.as_mut() else { return };
    // A test's jump (`sw hyper NAME`): from where the ship is now.
    if let Some(target) = pilot.dev_hyper.take() {
        let from = match (pilot.cruise, bodies.iter().find(|b| b.obj == p.obj)) {
            (Some(c), _) => Some(c.pos),
            (None, Some(b)) => Some(b.pos.offset_m(b.orient * p.pos)),
            _ => None,
        };
        if let Some(from) = from {
            begin(&data, sim.jd, &mut pilot, from, target);
        }
    }
    let Some(j) = pilot.jump.as_mut() else { return };
    j.seconds += time.delta_secs_f64();
    j.frames += 1;
    let eye = Vec3::from_array(assets.eye).as_dvec3();
    cam.pos = j.arrive.offset_m(j.rot * eye);
    cam.rot = j.rot;
    cam.follow = None;
    cam.velocity = DVec3::ZERO;
    cam.travel = None;
    cam.track = false;
    cam.hold_agl = None;
    let star = bodies.iter().find(|b| b.obj == j.target);
    if j.frames % 60 == 0 {
        info!("sidewinder: tunnel {:.1} s, {} frames: the active system's root {:?} ({}), the target's body {}", j.seconds, j.frames, active.root, active.name, if star.is_some() { "spawned" } else { "not yet" });
    }
    // (At least the tunnel's measured length, D, and until the target's system is entered.)
    let (true, true, Some(star)) = (j.seconds >= elite_sim::hyperspace::TUNNEL_S, active.root == Some(j.root), star) else { return };
    let j = *j;
    p.obj = star.obj;
    p.pos = star.orient.inverse() * j.arrive.delta_m(star.pos);
    p.rot = (star.orient.inverse() * j.rot).normalize();
    let start = elite_sim::supercruise::stand_in::FLOOR_PLANET;
    pilot.cruise = Some(Cruise { pos: j.arrive, rot: j.rot, speed: start, cap: start, carrier: Some((star.obj, star.pos)) });
    pilot.drive = elite_sim::drive::Drive::arrived();
    pilot.jump = None;
    pilot.target = None;
    msg.show(format!("Arrived: {}", active.name));
    info!("sidewinder: arrived at {} after {} frames, {:.1} s", active.name, j.frames, j.seconds);
}
