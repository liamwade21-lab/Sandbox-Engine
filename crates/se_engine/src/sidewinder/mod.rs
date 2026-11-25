//! The Faulcon DeLacy Sidewinder mod (`docs/spec-mods-sidewinder.md`): Elite Dangerous's starter ship, spawned from the Q
//! menu's Mods tab where the player looks, parked on its gear, a solid thing to walk round; boarded with E and flown by Elite's
//! rules (`crates/elite-sim`) in the steps that follow.
//!
//! - The parked ship's truth is its pose in its body's frame (`Parked`); everything else reads it. Near the player, with
//!   nobody in it, the ship is one prop of the sandbox's world (ARCHITECTURE.md A11: the physics gun moves it, Z undoes it)
//!   and `keep_in_world` writes `Parked` from the prop every frame; seated, the flight writes `Parked` and the ship stands in
//!   the world as solid pieces while landed. The prop is made again in a world made again (another body, or the world
//!   dropped); a rebase moves it with the other props. The solid pieces are made again after a rebase (it takes them all out).
//! - Spawning is its own request (`ShipRequest`), not "the mod on": a parked ship stays when another mod is turned on
//!   (the spec's edge cases). Boarding (E, `seat.rs`) is what turns the mod on.
//! - Its files are read on a worker thread at the first spawn (`assets::load`), checked as every mod's are
//!   (`mods::load_sidewinder`): Elite's own hull, gear, cockpit and chairs, posed by Elite's rigs (ARCHITECTURE.md A6).
//! - `parts.rs` says which pose the moving parts have; `draw.rs` draws every rig node's model with `ShipMaterial`
//!   (`material.rs`), lit by the sandbox's light records as the props are.
//! - `mouse_widget.rs` is Elite's mouse widget in the seat (the dot and the arrow of the mouse's virtual stick), measured
//!   from the recordings; `hud.rs` draws it.
//! - `convert` (tests only) makes the mod's files from Elite's files.

pub mod assets;
pub mod beam;
pub mod chair;
pub mod draw;
mod exposure;
pub mod facts;
pub mod hyper;
pub mod lights;
mod machines;
pub mod material;
mod mouse_widget;
pub mod panels;
pub mod parts;
pub mod hud;
pub mod hud_anchor;
pub mod pilot;
pub mod seat;
pub mod shadow;
pub mod solids;
mod tunnel;
mod sound;
pub mod ui;

#[cfg(test)]
mod convert;

pub use assets::{load, ShipShape, SidewinderAssets};
pub use pilot::MouseSetting;

use bevy::math::{DQuat, DVec3, Mat3, Vec3};
use bevy::prelude::*;
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use se_core::catalog::ObjectRef;

use crate::mods::{ModControl, ModRegistry};
use crate::sandbox::props::Props;
use crate::sandbox::world::{PropShape, SandboxWorld, SolidHandle, SolidShape, Surface};
use crate::sandbox::Sandbox;

/// The mod's folder name and code.
pub const CODE: &str = "sidewinder";
/// The spawn rule (the spec's F1): the ship's middle at least this far ahead, at the first clear place within the farthest.
const SPAWN_NEAREST_M: f32 = 18.0;
const SPAWN_FARTHEST_M: f32 = 60.0;
const SPAWN_STEP_M: f32 = 2.0;
/// The most the ground may rise between the gear's feet for a place to count as level enough to set the ship down on.
const SPAWN_MOST_SLOPE_M: f32 = 1.5;
/// A spawn asked for while the player isn't on foot yet (the Q menu turned another mod off) waits this long at most.
const SPAWN_WAIT_S: f32 = 10.0;
/// It is in the sandbox's world (a prop, or its solid pieces while seated) while it is this near the world's anchor.
const SOLID_REACH_M: f64 = 1500.0;

/// What the Mods row, a test command or the game asks of the ship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShipRequest {
    /// A fresh ship where the player looks (an old one goes).
    Spawn,
    /// Fuel, hull, modules, canopy and shields back to full (the spec's D5: free, while landed).
    Restock,
    /// The ship goes.
    Remove,
}

/// The ship where it stands: its pose in its body's frame, and what it is in the sandbox's world while near (a prop with
/// nobody in it, solid pieces while seated and landed).
pub struct Parked {
    /// Which ship this is: raised once for each new ship (a spawn, one replacing the last too), never by the ship moving
    /// (carried, thrown, flown, boarded or left). What must tell a new ship from the same one reads this and nothing else.
    pub spawn: u32,
    pub obj: ObjectRef,
    /// The model's origin (between the gear's feet, on the ground), body frame, metres.
    pub pos: DVec3,
    /// Model space to the body's frame.
    pub rot: DQuat,
    solids: Vec<SolidHandle>,
    /// The world's `solids_cleared` count the pieces were added under: when it changes they are gone (a rebase).
    solids_epoch: Option<u32>,
    /// Its prop: the world's `id` and the prop's (a world made again holds no prop of the old one's).
    prop: Option<(u32, u32)>,
    /// The next prop is set down on its feet (`set_down_on_feet`): he has sat in it since the last.
    set_down: bool,
    /// Its prop has come down onto its feet since it last left the ground or was held, and the seconds it has rested on all
    /// three so far (`kept_awake`).
    down: bool,
    on_feet: f32,
}

impl Parked {
    /// Its prop, if it is in this world.
    fn prop_in(&self, world: &SandboxWorld) -> Option<u32> {
        self.prop.filter(|&(w, id)| w == world.id && world.prop(id).is_some()).map(|(_, id)| id)
    }

    /// Its own velocity (body frame, m/s): its prop's in the sandbox's world, at rest without one (`Pilot::boarding`).
    pub fn velocity(&self, world: Option<&SandboxWorld>) -> DVec3 {
        world.and_then(|w| Some(w.frame.axes * w.motion(self.prop_in(w)?)?.1.as_dvec3())).unwrap_or(DVec3::ZERO)
    }
}

#[derive(Resource, Default)]
pub struct Sidewinder {
    pub parked: Option<Parked>,
    loading: Option<Task<Result<assets::Loaded, crate::mods::FileFault>>>,
    /// A spawn waiting for the files or for the player to be on foot, and for how long it has waited.
    want_spawn: Option<f32>,
    /// The ships spawned so far (the last one's `Parked::spawn`).
    spawns: u32,
}

/// The model's axes in the sandbox's local frame (x east, y north, z up) for a ship whose nose points along `nose` (taken
/// across `up`) and whose up is `up`: model X right, Y up, -Z the nose.
fn model_axes(nose: Vec3, up: Vec3) -> Mat3 {
    let f = (nose - up * nose.dot(up)).normalize_or(up.any_orthonormal_vector());
    let right = f.cross(up);
    Mat3::from_cols(right, up, -f)
}

/// The up of the ground under the feet: the normal of the plane through the three ground points (`Vec3::Z` with fewer).
fn feet_up(ground: &[Vec3]) -> Vec3 {
    let [a, b, c] = ground else { return Vec3::Z };
    let n = (*b - *a).cross(*c - *a).normalize_or(Vec3::Z);
    if n.z < 0.0 { -n } else { n }
}

/// The middle of the feet (model space).
fn feet_mid(shape: &ShipShape) -> Vec3 {
    shape.feet.iter().copied().sum::<Vec3>() / shape.feet.len().max(1) as f32
}

/// The ship set down on its feet: its up the ground's under them (`ground`, local frame, the feet's order), its nose along
/// `nose` across it, the feet's middle (`feet_mid`, model space) on the ground's. Its origin and axes.
fn stand_on(ground: &[Vec3], nose: Vec3, feet_mid: Vec3) -> (Vec3, Mat3) {
    let axes = model_axes(nose, feet_up(ground));
    (ground.iter().copied().sum::<Vec3>() / ground.len().max(1) as f32 - axes * feet_mid, axes)
}

/// Where the ship goes for a spawn (the spec's F1): along the view, its middle 18 to 60 m ahead, level enough, clear of the
/// ground, buildings and props; nose towards the player; set down on its three feet (its up the ground's under them), so it
/// stands where it is put and nothing drops or rocks. The model's origin and its axes in the local frame.
fn spawn_place(sb: &Sandbox, shape: &ShipShape) -> Result<(Vec3, Mat3), &'static str> {
    let world = sb.world.as_ref().ok_or("not on the ground")?;
    let grid = world.terrain().ok_or("the ground isn't ready")?;
    let ahead = Vec3::new(sb.forward.x, sb.forward.y, 0.0).normalize_or(Vec3::new(sb.yaw.cos(), sb.yaw.sin(), 0.0));
    let level = model_axes(-ahead, Vec3::Z);
    let centre = (shape.lo + shape.hi) * 0.5;
    let mut d = SPAWN_NEAREST_M;
    while d <= SPAWN_FARTHEST_M {
        // The model's middle (its box's, on the plan) this far ahead.
        let mid = Vec3::new(sb.eye.x, sb.eye.y, 0.0) + ahead * d;
        let origin = mid - level * Vec3::new(centre.x, 0.0, centre.z);
        let ground: Option<Vec<Vec3>> = shape.feet.iter().map(|f| {
            let p = origin + level * *f;
            grid.height(p.x, p.y).map(|h| Vec3::new(p.x, p.y, h))
        }).collect();
        d += SPAWN_STEP_M;
        let Some(ground) = ground else { continue };
        let (top, low) = ground.iter().fold((f32::MIN, f32::MAX), |(a, b), g| (a.max(g.z), b.min(g.z)));
        if top - low > SPAWN_MOST_SLOPE_M {
            continue;
        }
        let (at, axes) = stand_on(&ground, -ahead, feet_mid(shape));
        // The hull's box (above the gear) must be clear: an axis-aligned box round the turned one.
        let lo = Vec3::new(shape.lo.x, shape.feet.iter().map(|f| f.y).fold(f32::MIN, f32::max) + 0.3, shape.lo.z);
        let corners = (0..8).map(|i| axes * Vec3::new(if i & 1 == 0 { lo.x } else { shape.hi.x }, if i & 2 == 0 { lo.y } else { shape.hi.y }, if i & 4 == 0 { lo.z } else { shape.hi.z }));
        let (cmin, cmax) = corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), c| (a.min(c), b.max(c)));
        if world.box_blocked(at + (cmin + cmax) * 0.5, (cmax - cmin) * 0.5) {
            continue;
        }
        return Ok((at, axes));
    }
    Err("There is no room for the ship here")
}

/// Where the ship stands in the world's frame: its origin and its turn from model space.
fn local_pose(world: &SandboxWorld, p: &Parked) -> (Vec3, Quat) {
    (world.frame.from_body(p.pos).as_vec3(), (world.frame.rotation().inverse() * p.rot).normalize().as_quat())
}

/// Takes the ship's solid pieces out of the world if they are still in it (the caller refreshes the world's query tree).
fn drop_solids(world: &mut SandboxWorld, p: &mut Parked) {
    if p.solids_epoch.take() == Some(world.solids_cleared) {
        for h in p.solids.drain(..) {
            world.remove_solid(h);
        }
    }
    p.solids.clear();
}

/// Takes the ship's prop out of the world if it is in it, and a gun holding it lets go (the caller refreshes the world's
/// query tree).
fn drop_prop(world: &mut SandboxWorld, held: &mut Option<u32>, p: &mut Parked) {
    if let Some(id) = p.prop_in(world) {
        world.remove(id);
        if *held == Some(id) {
            *held = None;
        }
    }
    p.prop = None;
}

/// Takes the ship out of the sandbox's world, whatever it is there (its prop or its solid pieces): it goes, or another
/// takes its place.
fn take_out(sb: &mut Sandbox, p: &mut Parked) {
    match sb.world.as_mut() {
        Some(world) => {
            drop_prop(world, &mut sb.held, p);
            drop_solids(world, p);
            world.wake_props();
            world.refresh();
        }
        None => {
            p.solids.clear();
            p.solids_epoch = None;
            p.prop = None;
        }
    }
}

/// The ship as one prop of the world where `Parked` has it, at rest, of the flight's own mass (`Pilot::mass_t`).
fn add_prop(world: &mut SandboxWorld, p: &mut Parked, shape: &ShipShape, mass_kg: f32) {
    // (A new prop comes down onto its feet before it may sleep: `kept_awake`.)
    p.down = false;
    p.on_feet = 0.0;
    let (at, rot) = local_pose(world, p);
    p.prop = world.spawn(&PropShape::Hulls(shape.hulls.clone()), at, rot, mass_kg, Surface::Metal).map(|id| (world.id, id));
    match p.prop {
        Some((_, id)) => info!("sidewinder: a prop of the sandbox's world (prop {id}, {mass_kg:.0} kg) at {:.2} {:.2} {:.2} m", at.x, at.y, at.z),
        None => warn!("sidewinder: its pieces make no prop"),
    }
}

/// `Parked` from its prop's pose this frame, between the prop's last two ticks as the props are drawn (`props::place_props`).
fn follow_prop(world: &SandboxWorld, id: u32, t: f32, p: &mut Parked) {
    let Some(pose) = world.prop(id).map(|prop| prop.between(t)) else { return };
    p.pos = world.frame.to_body(pose.pos.as_dvec3());
    p.rot = (world.frame.rotation() * pose.rot.as_dquat()).normalize();
}

/// Puts the ship's solid pieces in the sandbox's world where it stands (and refreshes the world's query tree).
fn add_solids(world: &mut SandboxWorld, p: &mut Parked, shape: &ShipShape) {
    let (at, rot) = local_pose(world, p);
    for pts in &shape.hulls {
        if let Some(h) = world.add_solid(&SolidShape::Hull(pts.clone()), at, rot, Surface::Metal) {
            p.solids.push(h);
        }
    }
    p.solids_epoch = Some(world.solids_cleared);
    world.wake_props();
    world.refresh();
}

/// The Mods row's, the keys' and the test commands' asks; the files read on a worker thread at the first spawn.
#[allow(clippy::too_many_arguments)]
fn handle_requests(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut ship: ResMut<Sidewinder>,
    mut sb: ResMut<Sandbox>,
    mut control: ResMut<ModControl>,
    mut reg: ResMut<ModRegistry>,
    assets: Option<Res<SidewinderAssets>>,
    mut msg: ResMut<crate::messages::Messages>,
    mut pilot: Option<ResMut<pilot::Pilot>>,
    mut seat: ResMut<seat::Seat>,
    mut props: ResMut<Props>,
) {
    let ship = &mut *ship;
    // The files, once read.
    if let Some(task) = ship.loading.as_mut() {
        if let Some(done) = futures::check_ready(task) {
            ship.loading = None;
            match done {
                Ok(assets::Loaded { assets: a, build }) => {
                    let r = &a.rig;
                    let draws: usize = r.nodes.iter().filter_map(|n| n.model).map(|m| r.models[m].draws.len()).sum();
                    info!(
                        "sidewinder: files read ({} rig nodes, {} models, {draws} draws, {} textures; rest height {:.3} m, {} solid pieces, feet {:?}; {} key bindings)",
                        r.nodes.len(),
                        r.models.len(),
                        build.images.len(),
                        r.space.lift,
                        a.shape.hulls.len(),
                        a.shape.feet,
                        a.keys.actions.len()
                    );
                    commands.insert_resource(a);
                    commands.insert_resource(build);
                }
                Err((grey, why)) => {
                    warn!("sidewinder: {grey}: {why}");
                    reg.mark(CODE, grey);
                    msg.show("The Sidewinder's files are missing or damaged".to_string());
                    ship.want_spawn = None;
                }
            }
            return; // (the resource arrives next frame)
        }
    }
    match control.ship_request.take() {
        Some(ShipRequest::Spawn) => {
            ship.want_spawn = Some(0.0);
            if !(sb.walking() && sb.placed && sb.world.is_some()) {
                info!("sidewinder: the spawn waits for the player to be on foot (at most {SPAWN_WAIT_S} s)");
            }
            if assets.is_none() && ship.loading.is_none() {
                let needs = reg.mods.iter().find(|m| m.code == CODE).map(|m| m.needs.clone());
                match needs {
                    Some(needs) => {
                        info!("sidewinder: reading its files");
                        ship.loading = Some(AsyncComputeTaskPool::get().spawn(async move { crate::mods::load_sidewinder(CODE, &needs) }));
                    }
                    None => {
                        msg.show("The Sidewinder mod isn't installed".to_string());
                        ship.want_spawn = None;
                    }
                }
            }
        }
        // (Seated, the seat is released first and takes the ship: `seat::seat_camera`, later this frame.)
        Some(ShipRequest::Remove) if seat.seated => seat.request_remove(),
        Some(ShipRequest::Remove) => {
            if let Some(mut p) = ship.parked.take() {
                take_out(&mut sb, &mut p);
                info!("sidewinder: removed");
            }
        }
        Some(ShipRequest::Restock) => {
            if ship.parked.is_some() {
                if let Some(pl) = pilot.as_mut() {
                    pl.restock();
                }
                msg.show("Refuelled, repaired and restocked".to_string());
                info!("sidewinder: restocked");
            }
        }
        None => {}
    }
    if let (Some(waited), Some(a)) = (ship.want_spawn, assets.as_ref()) {
        if !(sb.walking() && sb.placed && sb.world.is_some()) {
            let waited = waited + time.delta_secs();
            ship.want_spawn = (waited < SPAWN_WAIT_S).then_some(waited);
            if ship.want_spawn.is_none() {
                msg.show("Spawn the Sidewinder while on foot".to_string());
                info!("sidewinder: no spawn: the player isn't on foot");
            }
            return;
        }
        ship.want_spawn = None;
        // The ship being replaced mustn't stand in the new one's way: out of the world first (`take_out` refreshes the
        // world's query tree). If there is no room it stays, and `keep_in_world` puts it back this frame.
        if let Some(old) = ship.parked.as_mut() {
            take_out(&mut sb, old);
        }
        match spawn_place(&sb, &a.shape) {
            Ok((at, axes)) => {
                let world = sb.world.as_ref().unwrap();
                let pos = world.frame.to_body(at.as_dvec3());
                let rot = (world.frame.rotation() * DQuat::from_mat3(&axes.as_dmat3())).normalize();
                info!("sidewinder: spawned at {:.1} {:.1} {:.1} m in the sandbox's frame, {:.0} m away", at.x, at.y, at.z, (at - sb.eye).truncate().length());
                if ship.parked.is_some() {
                    ui::new_ship(&mut commands);
                }
                ship.spawns += 1;
                ship.parked = Some(Parked { spawn: ship.spawns, obj: world.frame.obj, pos, rot, solids: Vec::new(), solids_epoch: None, prop: None, set_down: false, down: false, on_feet: 0.0 });
                if let Some(pl) = pilot.as_mut() {
                    pl.fresh_ship();
                }
                // Z takes it back as it does a prop (the ship it replaced leaves the list).
                props.ship_spawned();
                sb.cues.push(crate::sandbox::audio::Cue::ui("ui/buttonclickrelease.wav"));
            }
            Err(why) => {
                msg.show(why.to_string());
                info!("sidewinder: no spawn: {why}");
            }
        }
    }
    control.ship_parked = ship.parked.is_some();
}

/// The parked ship in the sandbox's world while it is near (ARCHITECTURE.md A11). Nobody in it: one prop, and `Parked`
/// follows the prop every frame (the only writer of it then). Seated and landed: its solid pieces (the flight writes
/// `Parked`); flying, nothing (it moves). Boarding takes the prop out where `Parked` last had it, getting out puts it back
/// there at rest. What is missing is put back where `Parked` says: the solid pieces after a rebase took them out, the prop in
/// a world made again (a rebase moves it with the other props).
fn keep_in_world(mut ship: ResMut<Sidewinder>, mut sb: ResMut<Sandbox>, assets: Option<Res<SidewinderAssets>>, seat: Res<seat::Seat>, pilot: Res<pilot::Pilot>, time: Res<Time<Real>>) {
    let (Some(p), Some(assets)) = (ship.parked.as_mut(), assets) else { return };
    let sb = &mut *sb;
    let t = sb.tick_fraction();
    let Some(world) = sb.world.as_mut() else {
        p.solids.clear();
        p.solids_epoch = None;
        p.prop = None;
        return;
    };
    // (Near by the pose `Parked` had: after a set-down across the planet the prop stands where f32 can't hold its place, and it
    // is taken out unread.)
    let near = world.frame.obj == p.obj && world.frame.from_body(p.pos).length() < SOLID_REACH_M;
    let prop = p.prop_in(world);
    if let (Some(id), true, false) = (prop, near, seat.seated) {
        follow_prop(world, id, t, p);
    }
    let want_prop = near && !seat.seated;
    // (Seated, the next prop is the one he gets out to: set down on its feet, as a spawn sets it down.)
    if seat.seated {
        p.set_down = true;
    }
    // (Boarded and not yet flown, it may be hanging in the air: no pieces until the flight says it stands landed.)
    let want_solids = near && seat.seated && pilot.landed && pilot.boarded.is_none();
    if p.solids_epoch != Some(world.solids_cleared) {
        p.solids.clear();
        p.solids_epoch = None;
    }
    let mut taken = false;
    match (want_solids, p.solids_epoch.is_some()) {
        (true, false) => add_solids(world, p, &assets.shape),
        (false, true) => {
            drop_solids(world, p);
            taken = true;
        }
        _ => {}
    }
    match (want_prop, prop) {
        (true, None) => {
            if std::mem::take(&mut p.set_down) {
                if let Some((pos, rot)) = set_down_on_feet(world, p, &assets.shape) {
                    (p.pos, p.rot) = (pos, rot);
                }
            }
            add_prop(world, p, &assets.shape, (pilot.mass_t() * 1000.0) as f32);
        }
        (false, Some(_)) => {
            drop_prop(world, &mut sb.held, p);
            taken = true;
        }
        _ => {}
    }
    if taken {
        // (What leant on it falls.)
        world.wake_props();
        world.refresh();
    }
    if let (true, Some(id)) = (want_prop, p.prop_in(world)) {
        if let Some(f) = footing(world, id, &assets.shape) {
            let awake = kept_awake(&mut p.down, &mut p.on_feet, &f, assets.shape.feet.len(), sb.held == Some(id), time.delta_secs());
            world.set_keep_awake(id, awake);
        }
    }
}

/// What the ship as a prop stands on, by the physics' own contacts: whether it is upright (within the touchdown's own
/// tilt), on how many of its feet it rests (each contact counted for the foot nearest it on the plan), and whether it
/// touches the ground at all.
struct Footing {
    upright: bool,
    feet: usize,
    touching: bool,
}

fn footing(world: &SandboxWorld, id: u32, shape: &ShipShape) -> Option<Footing> {
    let prop = world.prop(id)?;
    let tilt = (prop.now.rot * Vec3::Y).z.clamp(-1.0, 1.0).acos().to_degrees();
    let contacts = world.ground_contacts(id);
    let mut resting = vec![false; shape.feet.len()];
    for c in &contacts {
        let plan = |f: &Vec3| (f.x - c.x).powi(2) + (f.z - c.z).powi(2);
        if let Some(i) = (0..shape.feet.len()).min_by(|a, b| plan(&shape.feet[*a]).total_cmp(&plan(&shape.feet[*b]))) {
            resting[i] = true;
        }
    }
    Some(Footing { upright: f64::from(tilt) <= elite_sim::landing::TOUCHDOWN_TILT_DEG, feet: resting.iter().filter(|r| **r).count(), touching: !contacts.is_empty() })
}

/// Whether the ship's prop is kept from sleeping (`SandboxWorld::set_keep_awake`): only while it is coming down onto its
/// feet, from when it leaves the ground or a gun takes it until it has rested on all three for as long as the physics
/// wants a body still before it sleeps (`RigidBodyActivation::default_time_until_sleep`: a bounce that touches all three
/// for a moment is not resting), or come to rest on its side or back. `down` is that state and `on_feet` the seconds on
/// all three so far, kept between frames (`Parked`); `dt` the seconds since the last call. Then it sleeps by the physics'
/// own rule as any prop does: a foot rocking off its contact for a moment doesn't keep it awake again (awake on uneven
/// ground it creeps downhill).
fn kept_awake(down: &mut bool, on_feet: &mut f32, f: &Footing, feet: usize, held: bool, dt: f32) -> bool {
    if held || !f.touching {
        *down = false;
        *on_feet = 0.0;
    } else if !f.upright {
        *down = true;
    } else if f.feet == feet {
        *on_feet += dt;
        *down |= *on_feet >= rapier3d::prelude::RigidBodyActivation::default_time_until_sleep();
    } else {
        *on_feet = 0.0;
    }
    !*down
}

/// The ship set down where it stands on the sandbox's ground field under its feet, its heading kept (`stand_on`): getting
/// out puts the prop down as a spawn does, not at the flight's pose (level to the radial), which would drop and tip it on a
/// slope. None where the field doesn't reach under every foot.
fn set_down_on_feet(world: &SandboxWorld, p: &Parked, shape: &ShipShape) -> Option<(DVec3, DQuat)> {
    let grid = world.terrain()?;
    let (at, rot) = local_pose(world, p);
    let ground: Option<Vec<Vec3>> = shape.feet.iter().map(|f| {
        let q = at + rot * *f;
        grid.height(q.x, q.y).map(|h| Vec3::new(q.x, q.y, h))
    }).collect();
    let (at, axes) = stand_on(&ground?, rot * Vec3::NEG_Z, feet_mid(shape));
    Some((world.frame.to_body(at.as_dvec3()), (world.frame.rotation() * DQuat::from_mat3(&axes.as_dmat3())).normalize()))
}

/// Something for `handle_requests` to do: a request, the files being read or a spawn waiting.
fn asked(control: Res<ModControl>, ship: Res<Sidewinder>) -> bool {
    control.ship_request.is_some() || ship.loading.is_some() || ship.want_spawn.is_some()
}

pub struct SidewinderPlugin;

impl Plugin for SidewinderPlugin {
    fn build(&self, app: &mut App) {
        // (The mouse's sensitivity is a setting: there whether the mod or the sandbox is on or not, as `settings.rs` saves it.)
        app.init_resource::<Sidewinder>().init_resource::<MouseSetting>();
        if !crate::sandbox::enabled() {
            return;
        }
        draw::build(app);
        beam::build(app);
        shadow::build(app);
        tunnel::build(app);
        app.init_resource::<seat::Seat>()
            .init_resource::<seat::BoardPrompt>()
            // E at the ship and the seat's keys: after the frame's input, before the sandbox's reader takes E as "use".
            .add_systems(
                PreUpdate,
                (seat::board, seat::take_keys).chain().run_if(resource_exists::<SidewinderAssets>).after(bevy::input::InputSystems).after(crate::viltrumite::dev_keys).before(crate::sandbox::read_input),
            )
            // The seat writes the camera with the camera's own motion, after the sandbox's frame (which leaves first person the
            // frame he boards), before the hero's and before the camera's transform is set.
            .init_resource::<pilot::Pilot>()
            .init_resource::<pilot::PilotKeys>()
            .init_resource::<panels::Panels>()
            .add_systems(
                PostUpdate,
                (hyper::tunnel, pilot::fly, seat::seat_camera)
                    .chain()
                    .run_if(resource_exists::<SidewinderAssets>)
                    .in_set(crate::universe::UniverseSet::Camera)
                    .after(crate::sandbox::sandbox_step)
                    .before(crate::viltrumite::hero::hero_move)
                    .before(crate::camera::sync_camera_transform),
            )
            // Elite's field of view in the seat's view: inside the set that makes the view's matrices, so after everything
            // that writes the projection before it (the flight's, the sandbox's first-person view), and before the capture
            // views copy it.
            .add_systems(
                PostUpdate,
                seat::seat_fov
                    .run_if(resource_exists::<SidewinderAssets>)
                    .in_set(bevy::camera::CameraUpdateSystems)
                    .before(bevy::render::camera::camera_system)
                    .before(crate::capture::follow_main),
            )
            .add_systems(Update, seat::fade_clock.run_if(resource_exists::<SidewinderAssets>))
            .add_systems(bevy_egui::EguiPrimaryContextPass, (seat::draw_prompt, hud::draw_hud).run_if(resource_exists::<SidewinderAssets>).in_set(crate::skin::UiSet::Draw))
            // The HUD draws with the head's turn the seat wrote the camera with this frame: egui's pass (which runs the HUD)
            // after `seat_camera`.
            .configure_sets(PostUpdate, bevy_egui::EguiPostUpdateSet::EndPass.after(seat::seat_camera));
        // (The spec's C20: never spawned, nothing of the mod runs. Its systems wait for a request, then for its files.)
        // (After the sandbox's requests: its undo and Clean up ask for the ship's removal, done the same frame.)
        app.add_systems(Update, handle_requests.run_if(asked).after(crate::sandbox::props::handle_requests))
            // After the sandbox's frame (its ticks; its world may have been rebased) and after the seat's (boarding and getting
            // out are settled, the flight has written `Parked`), before anything reads `Parked` (`draw.rs`'s chain is after it).
            .add_systems(PostUpdate, keep_in_world.run_if(resource_exists::<SidewinderAssets>).after(crate::sandbox::sandbox_step).after(seat::seat_camera));
        sound::plugin(app);
        // Elite's own UI movies in the cockpit (on unless `SE_SW_UI=0`; with it 0 the plugin adds only `ui::UiLive`).
        ui::plugin(app);
        // The pilot chair's pose: from this frame's facts, for the parked ship.
        app.init_resource::<chair::ChairPose>().add_systems(PostUpdate, chair::update.run_if(resource_exists::<SidewinderAssets>).after(facts::update));
    }
}

/// `sw spawn|remove|restock|state|pose ...`: test commands (`--run "sw ..."`).
pub fn dev_command(world: &mut World, arg: &str) {
    if !world.contains_resource::<seat::Seat>() {
        warn!("sw: the sandbox is off (SE_SANDBOX=0): the Sidewinder mod plays in it");
        return;
    }
    let verb = arg.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let req = match verb.as_str() {
        "spawn" => Some(ShipRequest::Spawn),
        "remove" => Some(ShipRequest::Remove),
        "restock" => Some(ShipRequest::Restock),
        "board" => {
            let id = world.resource::<ModRegistry>().mods.iter().find(|m| m.code == CODE).map(|m| m.id.clone());
            if let (Some(id), true) = (id, world.resource::<Sidewinder>().parked.is_some()) {
                let reg = world.remove_resource::<ModRegistry>().unwrap();
                world.resource_mut::<ModControl>().board(&reg, &id);
                world.insert_resource(reg);
                let (keys, buttons) = (world.resource::<ButtonInput<KeyCode>>().clone(), world.resource::<ButtonInput<MouseButton>>().clone());
                let seat = seat::Seat::boarded(&mut world.resource_mut::<crate::universe::SimTime>(), &keys, &buttons);
                *world.resource_mut::<seat::Seat>() = seat;
                let vel = world.resource::<Sidewinder>().parked.as_ref().map_or(DVec3::ZERO, |p| p.velocity(world.resource::<Sandbox>().world.as_ref()));
                let fresh = world.resource::<pilot::Pilot>().boarding(vel);
                *world.resource_mut::<pilot::Pilot>() = fresh;
                info!("sidewinder: boarded (a test)");
            } else {
                warn!("sw board: no ship parked");
            }
            None
        }
        "leave" => {
            world.resource_mut::<seat::Seat>().request_leave();
            None
        }
        // `sw look YAW PITCH` (degrees, yaw to the left, pitch up): the head turned so, for pictures; `sw look off` gives it back.
        // Anything else changes nothing and says how to use it.
        "look" => {
            let words: Vec<&str> = arg.split_whitespace().skip(1).collect();
            let nums: Option<Vec<f32>> = words.iter().map(|w| w.parse().ok()).collect();
            let look = match (&words[..], nums.as_deref()) {
                // (A turn that isn't finite is refused here, before anything is logged: the seat's own rule.)
                (_, Some(&[yaw, pitch])) => seat::test_turn(Vec2::new(yaw.to_radians(), pitch.to_radians())).map(Some),
                (&[w], _) if w.eq_ignore_ascii_case("off") => Some(None),
                _ => None,
            };
            match look {
                Some(look) => {
                    info!("sidewinder: the head {}", look.map_or("is the pilot's again".to_string(), |l| format!("turned {:.2} {:.2} degrees (a test)", l.x.to_degrees(), l.y.to_degrees())));
                    world.resource_mut::<seat::Seat>().test_look = look;
                }
                None => warn!("sw look: use `sw look YAW PITCH` (finite degrees, yaw to the left, pitch up) or `sw look off`; got `{}`", words.join(" ")),
            }
            None
        }
        "pip" => {
            let word = arg.split_whitespace().nth(1).unwrap_or("").to_ascii_lowercase();
            let mut pl = world.resource_mut::<pilot::Pilot>();
            match word.as_str() {
                "eng" => pl.dist.increase(elite_sim::power::ENG),
                "sys" => pl.dist.increase(elite_sim::power::SYS),
                "wep" => pl.dist.increase(elite_sim::power::WEP),
                _ => pl.dist.reset(),
            }
            None
        }
        "boost" => {
            world.resource_mut::<pilot::Pilot>().dev_boost = true;
            None
        }
        // `sw hardpoints`, `sw fire SECS`: the hardpoints' toggle and the trigger held.
        "jump" => {
            world.resource_mut::<pilot::Pilot>().dev_jump = true;
            None
        }
        // `sw put NAME ALT_KM`: the ship over a body of this system, level, in thruster flight (a test's placing).
        "put" => {
            let words: Vec<&str> = arg.split_whitespace().skip(1).collect();
            let (name, alt) = match words.split_last() {
                Some((a, rest)) if a.parse::<f64>().is_ok() => (rest.join(" "), a.parse::<f64>().unwrap_or(1.0) * 1000.0),
                _ => (words.join(" "), 1000.0),
            };
            let found = world.resource::<crate::data::SeData>().catalog.find(&name);
            if found.is_none() {
                info!("sidewinder: no body named {name:?}");
            }
            world.resource_mut::<pilot::Pilot>().dev_put = found.map(|f| (f, alt));
            None
        }
        // `sw trace FILE|off`: a row a tick of the flight in the measured traces' columns.
        "trace" => {
            // (The rest of the line is the path: it may hold spaces, as the project's folder does.)
            let path = arg.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
            let path = if path.is_empty() { "off".to_string() } else { path };
            let mut pl = world.resource_mut::<pilot::Pilot>();
            pl.trace = if path == "off" {
                None
            } else {
                match std::fs::File::create(&path) {
                    Ok(f) => {
                        use std::io::Write;
                        let mut w = std::io::BufWriter::new(f);
                        let _ = writeln!(w, "t,speed,heat,key_w,key_s,key_x,key_tab,key_q,key_e,key_r,key_f,key_a,key_d,key_z,key_l,key_up,key_down,key_left,key_right,mouse_x,mouse_y,pip_sys,pip_eng,pip_wep,fa_off,fuel");
                        info!("sidewinder: tracing to {path}");
                        Some((w, pl.ticks))
                    }
                    Err(e) => {
                        info!("sidewinder: can't write {path}: {e}");
                        None
                    }
                }
            };
            None
        }
        // `sw dest NAME|none|state`: the destination (a body of this system, as the Navigation tab picks, through
        // `Pilot::lock`: it clears a star target; set, never unlocked by naming it again); `state` logs the locked destination
        // and the HUD's mark of it. `sw aim DEG`: the nose DEG over the horizon, the heading kept (a test's stick).
        "dest" => {
            let name = arg.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
            if name == "state" {
                let pl = world.resource::<pilot::Pilot>();
                let mark = pl.dest.as_ref().map_or("none".to_string(), |m| {
                    format!("{:?} dir ({:.5}, {:.5}, {:.5}), {:.0} m, angular radius {:.5} rad", m.name, m.dir.x, m.dir.y, m.dir.z, m.dist_m, m.ang_radius)
                });
                info!("sidewinder: locked {:?}, the mark {mark}", pl.locked());
                return;
            }
            let found = world.resource::<crate::data::SeData>().catalog.find(&name);
            if found.is_none() && name != "none" {
                info!("sidewinder: no body named {name:?}");
            }
            let mut pl = world.resource_mut::<pilot::Pilot>();
            let now = match found {
                Some(o) => pl.lock(pilot::Dest::Body(o), false),
                None => {
                    pl.unlock(false);
                    pl.locked()
                }
            };
            info!("sidewinder: sw dest: {}", pilot::dest_text(&world.resource::<crate::data::SeData>().catalog, now));
            None
        }
        // `sw mouse DX DY`: a test's mouse counts (x right, y down, as real counts and the measuring sessions' `keys.py
        // mouse`), through the same line of `fly` as the real ones. At the default sensitivity (the user's 0.2725, `MouseSetting`)
        // full stick is 1,180 counts and one step of the recordings is `sw mouse 0 -100`.
        "mouse" => {
            let nums: Option<Vec<f32>> = arg.split_whitespace().skip(1).map(|w| w.parse::<f32>().ok().filter(|x| x.is_finite())).collect();
            match nums.as_deref() {
                Some(&[dx, dy]) => {
                    world.resource_mut::<pilot::Pilot>().dev_mouse += Vec2::new(dx, dy);
                    info!("sidewinder: mouse {dx} {dy} counts (a test)");
                }
                _ => warn!("sw mouse: use `sw mouse DX DY` (finite counts, x right, y down); got `{arg}`"),
            }
            None
        }
        // `sw sens X|state`: the mouse's sensitivity for this run (U3; not saved: test runs skip the settings), and the stick.
        "sens" => {
            let word = arg.split_whitespace().nth(1).unwrap_or("");
            if word != "state" {
                match word.parse::<f32>().ok().filter(|s| s.is_finite() && *s > 0.0) {
                    Some(s) => world.resource_mut::<MouseSetting>().sensitivity = s,
                    None => {
                        warn!("sw sens: use `sw sens X` (a finite sensitivity over 0) or `sw sens state`; got `{arg}`");
                        return;
                    }
                }
            }
            let s = world.resource::<MouseSetting>().sensitivity;
            let stick = world.resource::<pilot::Pilot>().stick;
            info!("sidewinder: mouse sensitivity {s} (full stick at {:.1} counts), the stick {:.5} {:.5}", pilot::full_counts(s), stick.x, stick.y);
            None
        }
        "aim" => {
            let deg = arg.split_whitespace().nth(1).and_then(|w| w.parse::<f64>().ok()).unwrap_or(0.0);
            world.resource_mut::<pilot::Pilot>().dev_aim = Some(deg);
            None
        }
        // `sw silent|scoop`: the switch's key pressed (Delete, Home); `sw lights switch` presses the lights' (Insert). `sw canopy
        // [SECS]`: the canopy broken, with SECS of emergency oxygen left if given (a short clock for a test).
        "silent" | "scoop" => {
            world.resource_mut::<pilot::Pilot>().dev_switch = Some(if verb == "silent" { pilot::Switch::Silent } else { pilot::Switch::Scoop });
            None
        }
        // `sw lights [heat F|auto] [head on|off|auto] [gui off|auto] [cockpit X|auto] [state MACHINE STATE|auto] [switch]`:
        // alone, logs every light.
        "lights" => {
            let rest = arg.split_once(' ').map_or("", |(_, rest)| rest);
            // (`sw lights state MACHINE STATE|auto` is the parts' `ask`.)
            match rest.trim_start().strip_prefix("state ") {
                Some(ask) => parts::dev_command(world, &format!("ask {ask}")),
                None => lights::dev_command(world, rest),
            }
            None
        }
        // `sw canopy damage F|auto`: a test's ship damage (0..1) for the cockpit's cracks (`ShipFacts::dev_damage`).
        "canopy" if arg.split_whitespace().nth(1) == Some("damage") => {
            let word = arg.split_whitespace().nth(2).unwrap_or("auto");
            let damage = word.parse::<f32>().ok().filter(|d| d.is_finite());
            info!("sidewinder: the ship's damage for the cracks {}", damage.map_or("is the ship's again".to_string(), |d| format!("{d} (a test)")));
            world.resource_mut::<facts::ShipFacts>().dev_damage = damage;
            None
        }
        "canopy" => {
            let word = arg.split_whitespace().nth(1);
            let secs = word.map(|w| w.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0));
            if let Some(None) = secs {
                warn!("sw canopy: {:?} isn't a finite number of seconds at or above 0", word.unwrap_or(""));
                return;
            }
            let mut pl = world.resource_mut::<pilot::Pilot>();
            pl.life.canopy = false;
            if let Some(Some(s)) = secs {
                pl.life.oxygen = s;
            }
            info!("sidewinder: the canopy is broken (a test), {:.1} s of oxygen", pl.life.oxygen);
            None
        }
        // `sw face`: turns the ship to face its target (a test's aim; the real one is the stick).
        "face" => {
            world.resource_mut::<pilot::Pilot>().dev_face = true;
            None
        }
        // `sw target NAME|none` sets hyperspace's target (through `Pilot::lock`: it clears a body destination; `none` or a name
        // that is no star clears the target only); `sw hyper NAME` jumps there at once (the tunnel without the charge).
        "target" | "hyper" => {
            let mut words: Vec<&str> = arg.split_whitespace().skip(1).collect();
            let fail = verb == "hyper" && words.last() == Some(&"fail");
            if fail {
                words.pop();
            }
            let name = words.join(" ");
            let found = {
                let data = world.resource::<crate::data::SeData>();
                let f = data.catalog.find(&name);
                let star = f.and_then(|f| hyper::as_star(data, f));
                info!("sidewinder: {name:?} is {f:?}{}, its star {:?}", match f {
                    Some(se_core::catalog::ObjectRef::Star(i)) if data.catalog.stars[i as usize].barycenter => " (a barycentre)",
                    _ => "",
                }, star.map(|s| data.catalog.display_name(s)));
                star
            };
            if found.is_none() && name != "none" {
                info!("sidewinder: no star named {name:?}");
            }
            let mut pl = world.resource_mut::<pilot::Pilot>();
            if verb == "target" {
                match found {
                    Some(t) => {
                        pl.lock(pilot::Dest::System(t), false);
                    }
                    None => pl.unlock(true),
                }
            } else {
                pl.dev_hyper = found;
                pl.dev_hyper_fail = fail;
            }
            None
        }
        // `sw pose up|down|auto`: the gear commanded up or down whatever the pilot's (the machines move it; pictures from the free
        // camera). `sw parts [log on|off] [ask MACHINE STATE|auto]`: the state machines.
        "parts" => {
            parts::dev_command(world, arg.split_once(' ').map_or("", |(_, rest)| rest));
            None
        }
        // `sw chair [log on|off]`: the pilot chair's state, clips, variables and bones; its changes logged.
        "chair" => {
            chair::dev_command(world, arg.split_once(' ').map_or("", |(_, rest)| rest));
            None
        }
        "pose" => {
            let word = arg.split_whitespace().nth(1).unwrap_or("auto").to_ascii_lowercase();
            world.resource_mut::<facts::ShipFacts>().dev_gear = match word.as_str() {
                "up" => Some(false),
                "down" => Some(true),
                _ => None,
            };
            None
        }
        "sound" => {
            sound::dev_command(world, arg.split_once(' ').map_or("", |(_, rest)| rest));
            None
        }
        // `sw ui state|send MOVIE NAME JSON_ARGS|want MOVIE on|off|plate NAME on|off`: Elite's UI movies (`SE_SW_UI=1`).
        "ui" => {
            ui::dev_command(world, arg.split_once(' ').map_or("", |(_, rest)| rest));
            None
        }
        "hardpoints" => {
            let mut pl = world.resource_mut::<pilot::Pilot>();
            pl.weapons.deployed = !pl.weapons.deployed;
            None
        }
        "fire" => {
            let secs = arg.split_whitespace().nth(1).and_then(|w| w.parse::<f32>().ok()).unwrap_or(1.0);
            world.resource_mut::<pilot::Pilot>().dev_fire = secs;
            None
        }
        "throttle" | "thrust" | "stick" | "fa" | "gear" | "log" => {
            let nums: Vec<f64> = arg.split_whitespace().skip(1).filter_map(|w| w.parse().ok()).collect();
            let word = arg.split_whitespace().nth(1).unwrap_or("").to_ascii_lowercase();
            let g = |i: usize| nums.get(i).copied().unwrap_or(0.0);
            let mut pl = world.resource_mut::<pilot::Pilot>();
            match verb.as_str() {
                "throttle" => pl.throttle = g(0).clamp(-1.0, 1.0),
                "fa" => pl.flight_assist = word != "off",
                // (The key's press: refused while landed, as the key is.)
                "gear" => pl.dev_switch = Some(pilot::Switch::Gear),
                "log" => pl.log = word != "off",
                // `sw thrust LAT VERT FWD SECS`, `sw stick PITCH ROLL YAW SECS`: demands held for the seconds given.
                "thrust" => pl.dev = Some((elite_sim::flight::Input { lateral: g(0), vertical: g(1), forward: g(2), ..default() }, g(3) as f32)),
                _ => pl.dev = Some((elite_sim::flight::Input { pitch: g(0), roll: g(1), yaw: g(2), ..default() }, g(3) as f32)),
            }
            None
        }
        // `sw feet`: where the parked ship stands: each foot's height over the sandbox's ground field (what the prop rests on)
        // and over the loaded ground (what the flight reads), and every solid piece whose lowest point is within 0.3 m of the
        // field.
        "feet" => {
            let mut q = world.query::<(&crate::system::SystemBody, Option<&crate::terrain::Terrain>)>();
            let (ship, sb, assets) = (world.resource::<Sidewinder>(), world.resource::<Sandbox>(), world.get_resource::<SidewinderAssets>());
            let (Some(p), Some(w), Some(a)) = (ship.parked.as_ref(), sb.world.as_ref(), assets) else {
                info!("sidewinder: sw feet: no parked ship in the sandbox's world");
                return;
            };
            let Some((b, t)) = q.iter(world).find(|(b, _)| b.obj == p.obj) else { return };
            let t = t.filter(|t| !t.proc.kind.is_giant());
            let (at, rot) = local_pose(w, p);
            let field = |m: Vec3| {
                let l = at + rot * m;
                w.terrain().and_then(|g| g.height(l.x, l.y)).map_or(f32::NAN, |h| l.z - h)
            };
            let loaded = |m: Vec3| {
                let q = p.pos + p.rot * m.as_dvec3();
                q.length() - crate::ship::ground_on(b.radius_m, b.shape, t, q).0
            };
            let up = p.pos.normalize();
            info!(
                "sidewinder: feet: tilt {:.3} degrees to the radial, {:.3} to the frame's up; the prop {}",
                (p.rot * DVec3::Y).dot(up).clamp(-1.0, 1.0).acos().to_degrees(),
                (rot * Vec3::Y).z.clamp(-1.0, 1.0).acos().to_degrees(),
                p.prop_in(w).map_or("none".to_string(), |id| format!(
                    "{}, {} ground contacts, {}",
                    if w.sleeping(id) { "asleep" } else { "awake" },
                    w.ground_contacts(id).len(),
                    footing(w, id, &a.shape).map_or(String::new(), |f| format!("upright {}, on {} feet, kept awake {}", f.upright, f.feet, !p.down))
                ))
            );
            for (i, f) in a.shape.feet.iter().enumerate() {
                info!("sidewinder: foot {i} at {:.2} {:.2} {:.2} (model): {:.3} m over the field, {:.3} m over the loaded ground", f.x, f.y, f.z, field(*f), loaded(*f));
            }
            for (i, h) in a.shape.hulls.iter().enumerate() {
                let low = h.iter().copied().min_by(|x, y| field(*x).total_cmp(&field(*y)));
                if let Some(low) = low.filter(|l| field(*l) < 0.3) {
                    info!("sidewinder: piece {i}: lowest {:.3} m over the field ({:.3} over the loaded ground) at {:.2} {:.2} {:.2} (model)", field(low), loaded(low), low.x, low.y, low.z);
                }
            }
            None
        }
        "state" => {
            let ship = world.resource::<Sidewinder>();
            let sb = world.resource::<Sandbox>();
            match (ship.parked.as_ref(), sb.world.as_ref()) {
                (Some(p), Some(w)) if w.frame.obj == p.obj => {
                    let at = w.frame.from_body(p.pos);
                    // (Its tilt: the model's up against the frame's.)
                    let tilt = (w.frame.rotation().inverse() * p.rot * DVec3::Y).z.clamp(-1.0, 1.0).acos().to_degrees();
                    let nose = w.frame.rotation().inverse() * p.rot * DVec3::NEG_Z;
                    let what = match p.prop_in(w).and_then(|id| w.prop(id)) {
                        Some(prop) => {
                            let v = w.motion(prop.id).map_or(Vec3::ZERO, |m| m.1);
                            format!("prop {} ({:.0} kg, {:.3} m/s{}{})", prop.id, prop.mass, v.length(), if prop.frozen { ", frozen" } else { "" }, if prop.held { ", held" } else { "" })
                        }
                        None => format!("{} solid pieces in", if p.solids_epoch == Some(w.solids_cleared) { p.solids.len() } else { 0 }),
                    };
                    info!(
                        "sidewinder: parked at {:.4} {:.4} {:.4} m in the sandbox's frame ({:.1} m from the eyes; body frame {:.4} {:.4} {:.4}), tilted {:.3} degrees, the nose at yaw {:.2}; {what}",
                        at.x, at.y, at.z, (at.as_vec3() - sb.eye).length(), p.pos.x, p.pos.y, p.pos.z, tilt, nose.y.atan2(nose.x).to_degrees()
                    );
                }
                (Some(_), _) => info!("sidewinder: parked on another body"),
                _ => info!("sidewinder: none (files read: {})", world.contains_resource::<SidewinderAssets>()),
            }
            let (seat, pl) = (world.resource::<seat::Seat>(), world.resource::<pilot::Pilot>());
            if seat.seated {
                let v = pl.motion.vel;
                info!(
                    "sidewinder: seated; {}, {:.2} m/s, throttle {:.2}, flight assist {}, gear {} ({:?}), {} ticks; pips (half) WEP/ENG/SYS {:?}, capacitors {:.2}/{:.2}/{:.2} MJ, boost {:?}; drive {:?}, supercruise {}, glide {:?}, target {:?}, fuel {:.6} t main, {:.6} t reservoir; modules {:?}, destination {:?}, planet HUD {:?}, held {:?}, silent {}, scoop {}, lights {}, life {:?}, safe drop {:?}",
                    if pl.landed { "landed" } else { "flying" },
                    v.length(),
                    pl.throttle,
                    pl.flight_assist,
                    if pl.gear_down { "down" } else { "up" },
                    world.resource::<parts::Parts>().gear().legs(),
                    pl.ticks,
                    pl.dist.half,
                    pl.dist.charge[0],
                    pl.dist.charge[1],
                    pl.dist.charge[2],
                    pl.boost.state,
                    pl.drive,
                    pl.cruise.map_or("no".to_string(), |c| format!("{:.0} km/s (the wells' cap {:.0} km/s)", c.speed / 1000.0, c.cap / 1000.0)),
                    pl.glide,
                    pl.target,
                    pl.fuel.main,
                    pl.fuel.reservoir,
                    pl.modules,
                    pl.nav_body,
                    pl.planet,
                    pl.drive.held(),
                    pl.silent,
                    pl.scoop,
                    pl.lights,
                    pl.life,
                    pl.safe_drop
                );
                if let Some(p) = ship.parked.as_ref() {
                    let nose = p.rot * bevy::math::DVec3::NEG_Z;
                    let down = -p.pos.normalize_or(bevy::math::DVec3::Y);
                    let w = pl.motion.omega;
                    info!("sidewinder: {:.3} km from {:?}'s centre, the nose {:.1} degrees over the horizon; turning {:.1} {:.1} {:.1} deg/s (pitch, yaw, roll); heat {:.1} %, stars' heat {:.4} a second, shields {:.0} % ({}), hull {:.0} %, air {:.4} kg/m3", p.pos.length() / 1000.0, p.obj, elite_sim::glide::pitch_deg(nose.to_array(), down.to_array()), w.x.to_degrees(), w.y.to_degrees(), w.z.to_degrees(), pl.heat_percent(), pl.star_heat, pl.shields.percent(), if pl.shields.up { "up" } else { "down" }, pl.hull.percent(), pl.air_density);
                }
            }
            None
        }
        _ => {
            warn!("sw: spawn | remove | restock | state | feet | board | leave | throttle X | thrust LAT VERT FWD SECS | stick PITCH ROLL YAW SECS | fa on|off | gear | log on|off | pip eng|sys|wep|reset | boost | hardpoints | fire SECS | jump | target NAME|none | hyper NAME [fail] | face | put NAME ALT_KM | trace FILE|off | dest NAME|none | aim DEG | silent | scoop | lights [heat F|auto] [head on|off|auto] [state MACHINE STATE|auto] [switch] | canopy [SECS] | pose up|down|auto");
            None
        }
    };
    if let Some(r) = req {
        world.resource_mut::<ModControl>().ship_request = Some(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::world::{LocalFrame, TerrainGrid};

    /// A world of `rise_e`, `rise_n` (m a metre, east and north) and the ship's pieces.
    fn sloped(rise_e: f32, rise_n: f32) -> (SandboxWorld, TerrainGrid, std::sync::Arc<ShipShape>) {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        let shape = assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets.shape;
        let frame = LocalFrame::at(ObjectRef::Body(0), DVec3::ONE, DVec3::new(0.0, 0.0, 6.371e6));
        let mut w = SandboxWorld::new(frame, crate::sandbox::movement::TICK);
        let (n, cell) = (161, 0.75);
        let half = (n - 1) as f32 * cell * 0.5;
        let heights = (0..n * n).map(|i| ((i % n) as f32 * cell - half) * rise_e + ((i / n) as f32 * cell - half) * rise_n).collect();
        w.set_terrain(TerrainGrid { n, cell, heights });
        let grid = w.terrain().unwrap().clone();
        (w, grid, shape)
    }

    /// Steps the world `secs` with the ship's rule for sleeping (`kept_awake`) as `keep_in_world` applies it; the pose
    /// after `early` seconds and at the end.
    fn settle(w: &mut SandboxWorld, id: u32, shape: &ShipShape, early: f32, secs: f32) -> (crate::sandbox::world::Pose, crate::sandbox::world::Pose) {
        let mut impacts = Vec::new();
        let mut first = w.prop(id).unwrap().now;
        let (mut down, mut on_feet) = (false, 0.0);
        for k in 0..(secs / crate::sandbox::movement::TICK) as usize {
            let f = footing(w, id, shape).unwrap();
            let awake = kept_awake(&mut down, &mut on_feet, &f, shape.feet.len(), false, crate::sandbox::movement::TICK);
            w.set_keep_awake(id, awake);
            w.step(&mut impacts);
            if k == (early / crate::sandbox::movement::TICK) as usize {
                first = w.prop(id).unwrap().now;
            }
        }
        (first, w.prop(id).unwrap().now)
    }

    fn feet_over(grid: &TerrainGrid, shape: &ShipShape, p: crate::sandbox::world::Pose) -> Vec<f32> {
        shape.feet.iter().map(|f| { let q = p.pos + p.rot * *f; q.z - grid.height(q.x, q.y).unwrap() }).collect()
    }

    /// The parked ship as a prop stands on its three feet where a spawn sets it down (`stand_on`), on flat ground and on
    /// slopes as steep as the spawn allows across and along it, and then holds still: the pose after 5 s and after 120 s agree
    /// to the millimetre and it sleeps (ARCHITECTURE.md A11; the review's 0.47 m foot was a prop asleep part-way through
    /// settling, its 0.2 m in 52 s a prop never let sleep).
    #[test]
    fn the_ship_stands_on_its_three_feet_and_holds_still() {
        // (Rise east, rise north, m a metre: the spawn's 1.5 m between the feet is about 1:6 along the ship, 1:7 across it.)
        for (rise_e, rise_n) in [(0.0f32, 0.0f32), (0.0, 0.16), (0.0, -0.16), (0.14, 0.0), (0.1, 0.1)] {
            let (mut w, grid, shape) = sloped(rise_e, rise_n);
            let level = model_axes(Vec3::Y, Vec3::Z);
            let ground: Vec<Vec3> = shape.feet.iter().map(|f| { let p = level * *f; Vec3::new(p.x, p.y, grid.height(p.x, p.y).unwrap()) }).collect();
            let (at, axes) = stand_on(&ground, Vec3::Y, feet_mid(&shape));
            let id = w.spawn(&PropShape::Hulls(shape.hulls.clone()), at, Quat::from_mat3(&axes), 45200.0, Surface::Metal).unwrap();
            let (early, end) = settle(&mut w, id, &shape, 5.0, 120.0);
            let feet = feet_over(&grid, &shape, end);
            assert!(feet.iter().all(|h| h.abs() < 0.02), "slope {rise_e} {rise_n}: the feet {feet:?} m over the ground");
            assert!((end.pos - early.pos).length() < 0.001, "slope {rise_e} {rise_n}: crept {} m from 5 s to 120 s", (end.pos - early.pos).length());
            assert!(w.sleeping(id), "slope {rise_e} {rise_n}: not asleep at 120 s");
        }
    }

    /// Dropped level onto a 1:8 slope across it (one rear foot and the front over the ground: as a thrown ship lands), it
    /// comes down onto all three feet, then sleeps and holds still; it is not put to sleep on two.
    #[test]
    fn a_ship_coming_down_onto_its_feet_is_not_put_to_sleep_on_two() {
        let (mut w, grid, shape) = sloped(0.125, 0.0);
        let level = model_axes(Vec3::Y, Vec3::Z);
        let top = shape.feet.iter().map(|f| { let p = level * *f; grid.height(p.x, p.y).unwrap() }).fold(f32::MIN, f32::max);
        let id = w.spawn(&PropShape::Hulls(shape.hulls.clone()), Vec3::new(0.0, 0.0, top), Quat::from_mat3(&level), 45200.0, Surface::Metal).unwrap();
        let (early, end) = settle(&mut w, id, &shape, 60.0, 120.0);
        let feet = feet_over(&grid, &shape, end);
        assert!(feet.iter().all(|h| h.abs() < 0.02), "the feet {feet:?} m over the ground");
        assert!((end.pos - early.pos).length() < 0.001 && w.sleeping(id), "crept {} m from 60 s to 120 s", (end.pos - early.pos).length());
    }
}
