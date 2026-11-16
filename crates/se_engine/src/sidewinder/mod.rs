//! The Faulcon DeLacy Sidewinder mod (`docs/spec-mods-sidewinder.md`): Elite Dangerous's starter ship, spawned from the Q
//! menu's Mods tab where the player looks, parked on its gear, a solid thing to walk round; boarded with E and flown by Elite's
//! rules (`crates/elite-sim`) in the steps that follow.
//!
//! - The parked ship's truth is its pose in its body's frame (`Parked`): its solid pieces in the sandbox's world and its
//!   meshes are made from that, and made again after the sandbox moves its frame (a rebase takes every solid piece out) or
//!   when the player comes back near it.
//! - Spawning is its own request (`ShipRequest`), not "the mod on": a parked ship stays when another mod is turned on
//!   (the spec's edge cases). Boarding (E, `seat.rs`) is what turns the mod on.
//! - Its files are read on a worker thread at the first spawn (`assets::load`), checked as every mod's are
//!   (`mods::load_sidewinder`): Elite's own hull, gear, cockpit and chairs, posed by Elite's rigs (ARCHITECTURE.md A6).
//! - `parts.rs` says which pose the moving parts have; `draw.rs` draws every rig node's model with `ShipMaterial`
//!   (`material.rs`), lit by the sandbox's light records as the props are.
//! - `convert` (tests only) makes the mod's files from Elite's files.

pub mod assets;
pub mod draw;
pub mod hyper;
pub mod material;
pub mod panels;
pub mod parts;
pub mod hud;
pub mod pilot;
pub mod seat;
pub mod solids;
mod sound;

#[cfg(test)]
mod convert;

pub use assets::{load, ShipShape, SidewinderAssets};

use bevy::math::{DQuat, DVec3, Mat3, Vec3};
use bevy::prelude::*;
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use se_core::catalog::ObjectRef;

use crate::mods::{ModControl, ModRegistry};
use crate::sandbox::world::{SandboxWorld, SolidHandle, SolidShape, Surface};
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
/// Its solid pieces are kept in the sandbox's world while it is this near the world's anchor.
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

/// The ship where it stands: its pose in its body's frame, and its solid pieces in the sandbox's world while near.
pub struct Parked {
    pub obj: ObjectRef,
    /// The model's origin (between the gear's feet, on the ground), body frame, metres.
    pub pos: DVec3,
    /// Model space to the body's frame.
    pub rot: DQuat,
    solids: Vec<SolidHandle>,
    /// The world's `solids_cleared` count the pieces were added under: when it changes they are gone (a rebase).
    solids_epoch: Option<u32>,
}

#[derive(Resource, Default)]
pub struct Sidewinder {
    pub parked: Option<Parked>,
    loading: Option<Task<Result<assets::Loaded, crate::mods::FileFault>>>,
    /// A spawn waiting for the files or for the player to be on foot, and for how long it has waited.
    want_spawn: Option<f32>,
}

/// The model's axes in the sandbox's local frame (x east, y north, z up) for a ship whose nose points along `nose`
/// (horizontal): model X right, Y up, -Z the nose.
fn model_axes(nose: Vec3) -> Mat3 {
    let f = Vec3::new(nose.x, nose.y, 0.0).normalize_or(Vec3::X);
    let right = f.cross(Vec3::Z);
    Mat3::from_cols(right, Vec3::Z, -f)
}

/// Where the ship goes for a spawn (the spec's F1): along the view, its middle 18 to 60 m ahead, level enough, clear of the
/// ground, buildings and props; nose towards the player. The model's origin and its axes in the local frame.
fn spawn_place(sb: &Sandbox, shape: &ShipShape) -> Result<(Vec3, Mat3), &'static str> {
    let world = sb.world.as_ref().ok_or("not on the ground")?;
    let grid = world.terrain().ok_or("the ground isn't ready")?;
    let ahead = Vec3::new(sb.forward.x, sb.forward.y, 0.0).normalize_or(Vec3::new(sb.yaw.cos(), sb.yaw.sin(), 0.0));
    let axes = model_axes(-ahead);
    let centre = (shape.lo + shape.hi) * 0.5;
    let mut d = SPAWN_NEAREST_M;
    while d <= SPAWN_FARTHEST_M {
        // The model's middle (its box's, on the plan) this far ahead.
        let mid = Vec3::new(sb.eye.x, sb.eye.y, 0.0) + ahead * d;
        let origin = mid - axes * Vec3::new(centre.x, 0.0, centre.z);
        let heights: Option<Vec<f32>> = shape.feet.iter().map(|f| {
            let p = origin + axes * *f;
            grid.height(p.x, p.y)
        }).collect();
        d += SPAWN_STEP_M;
        let Some(h) = heights else { continue };
        let (top, low) = h.iter().fold((f32::MIN, f32::MAX), |(a, b), &x| (a.max(x), b.min(x)));
        if top - low > SPAWN_MOST_SLOPE_M {
            continue;
        }
        let at = Vec3::new(origin.x, origin.y, top);
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

/// Takes the ship's solid pieces out of the world if they are still in it.
fn remove_solids(sb: &mut Sandbox, p: &mut Parked) {
    if let (Some(world), Some(epoch)) = (sb.world.as_mut(), p.solids_epoch.take()) {
        if world.solids_cleared == epoch && world.frame.obj == p.obj {
            for h in p.solids.drain(..) {
                world.remove_solid(h);
            }
            world.wake_props();
            world.refresh();
        }
    }
    p.solids.clear();
}

/// Puts the ship's solid pieces in the sandbox's world where it stands (and refreshes the world's query tree).
fn add_solids(world: &mut SandboxWorld, p: &mut Parked, shape: &ShipShape) {
    let at = world.frame.from_body(p.pos).as_vec3();
    let rot = (world.frame.rotation().inverse() * p.rot).normalize().as_quat();
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
        Some(ShipRequest::Remove) => {
            if let Some(mut p) = ship.parked.take() {
                remove_solids(&mut sb, &mut p);
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
            }
            return;
        }
        ship.want_spawn = None;
        // The ship being replaced mustn't stand in the new one's way: its pieces out first (`remove_solids` refreshes the
        // world's query tree), and back in if there is no room.
        let had_solids = ship.parked.as_ref().is_some_and(|p| p.solids_epoch.is_some());
        if let Some(old) = ship.parked.as_mut() {
            remove_solids(&mut sb, old);
        }
        match spawn_place(&sb, &a.shape) {
            Ok((at, axes)) => {
                let world = sb.world.as_ref().unwrap();
                let pos = world.frame.to_body(at.as_dvec3());
                let rot = (world.frame.rotation() * DQuat::from_mat3(&axes.as_dmat3())).normalize();
                info!("sidewinder: spawned at {:.1} {:.1} {:.1} m in the sandbox's frame, {:.0} m away", at.x, at.y, at.z, (at - sb.eye).truncate().length());
                ship.parked = Some(Parked { obj: world.frame.obj, pos, rot, solids: Vec::new(), solids_epoch: None });
                if let Some(pl) = pilot.as_mut() {
                    pl.fresh_ship();
                }
                sb.cues.push(crate::sandbox::audio::Cue::ui("ui/buttonclickrelease.wav"));
            }
            Err(why) => {
                if let (true, Some(old), Some(world)) = (had_solids, ship.parked.as_mut(), sb.world.as_mut()) {
                    add_solids(world, old, &a.shape);
                }
                msg.show(why.to_string());
                info!("sidewinder: no spawn: {why}");
            }
        }
    }
    control.ship_parked = ship.parked.is_some();
}

/// The parked ship's solid pieces in the sandbox's world: added when it is near and they aren't there (first, or after a
/// rebase took them out), taken out when it goes far or the world changes body.
fn keep_solid(mut ship: ResMut<Sidewinder>, mut sb: ResMut<Sandbox>, assets: Option<Res<SidewinderAssets>>, seat: Res<seat::Seat>, pilot: Res<pilot::Pilot>) {
    let (Some(p), Some(assets)) = (ship.parked.as_mut(), assets) else { return };
    // (Flying, it is no solid piece of the sandbox's world: it moves.)
    let landed = !seat.seated || pilot.landed;
    let Some(world) = sb.world.as_mut() else {
        p.solids.clear();
        p.solids_epoch = None;
        return;
    };
    let here = landed && world.frame.obj == p.obj && world.frame.from_body(p.pos).length() < SOLID_REACH_M;
    let present = p.solids_epoch == Some(world.solids_cleared);
    if !present {
        p.solids.clear();
        p.solids_epoch = None;
    }
    if here && !present {
        add_solids(world, p, &assets.shape);
    } else if !here && present {
        for h in p.solids.drain(..) {
            world.remove_solid(h);
        }
        p.solids_epoch = None;
        world.refresh();
    }
}

/// Something for `handle_requests` to do: a request, the files being read or a spawn waiting.
fn asked(control: Res<ModControl>, ship: Res<Sidewinder>) -> bool {
    control.ship_request.is_some() || ship.loading.is_some() || ship.want_spawn.is_some()
}

pub struct SidewinderPlugin;

impl Plugin for SidewinderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sidewinder>();
        if !crate::sandbox::enabled() {
            return;
        }
        draw::build(app);
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
            .add_systems(bevy_egui::EguiPrimaryContextPass, (seat::draw_prompt, hud::draw_hud).run_if(resource_exists::<SidewinderAssets>).in_set(crate::skin::UiSet::Draw));
        // (The spec's C20: never spawned, nothing of the mod runs. Its systems wait for a request, then for its files.)
        app.add_systems(Update, handle_requests.run_if(asked))
            // After the sandbox's frame (its world may have been rebased this frame), before its props are drawn.
            .add_systems(PostUpdate, keep_solid.run_if(resource_exists::<SidewinderAssets>).after(crate::sandbox::sandbox_step));
        sound::plugin(app);
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
                let seat = seat::Seat::boarded(&mut world.resource_mut::<crate::universe::SimTime>());
                *world.resource_mut::<seat::Seat>() = seat;
                let fresh = world.resource::<pilot::Pilot>().boarding();
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
        // `sw dest NAME|none`: the destination (a body of this system, as the Navigation tab picks); `sw aim DEG`: the nose DEG
        // over the horizon, the heading kept (a test's stick).
        "dest" => {
            let name = arg.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
            let found = world.resource::<crate::data::SeData>().catalog.find(&name);
            world.resource_mut::<pilot::Pilot>().nav_body = found;
            None
        }
        "aim" => {
            let deg = arg.split_whitespace().nth(1).and_then(|w| w.parse::<f64>().ok()).unwrap_or(0.0);
            world.resource_mut::<pilot::Pilot>().dev_aim = Some(deg);
            None
        }
        // `sw silent|scoop|lights`: the switch's key pressed (Delete, Home, Insert); `sw canopy [SECS]`: the canopy broken, with
        // SECS of emergency oxygen left if given (a short clock for a test).
        "silent" | "scoop" | "lights" => {
            world.resource_mut::<pilot::Pilot>().dev_switch = Some(match verb.as_str() {
                "silent" => pilot::Switch::Silent,
                "scoop" => pilot::Switch::Scoop,
                _ => pilot::Switch::Lights,
            });
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
        // `sw target NAME|none` sets hyperspace's target; `sw hyper NAME` jumps there at once (the tunnel without the charge).
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
                pl.target = found;
            } else {
                pl.dev_hyper = found;
                pl.dev_hyper_fail = fail;
            }
            None
        }
        // `sw pose up|down|auto`: the parts drawn with the gear up or down whatever the pilot's (pictures from the free camera).
        "pose" => {
            let word = arg.split_whitespace().nth(1).unwrap_or("auto").to_ascii_lowercase();
            world.resource_mut::<parts::Parts>().dev_gear = match word.as_str() {
                "up" => Some(false),
                "down" => Some(true),
                _ => None,
            };
            None
        }
        // `sw light test|off`: a point light and a spot over the hull (the lights buffer's test).
        "light" => {
            material::dev_light(world, arg.split_whitespace().nth(1).unwrap_or("off"));
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
                "gear" => pl.gear_down = !pl.gear_down,
                "log" => pl.log = word != "off",
                // `sw thrust LAT VERT FWD SECS`, `sw stick PITCH ROLL YAW SECS`: demands held for the seconds given.
                "thrust" => pl.dev = Some((elite_sim::flight::Input { lateral: g(0), vertical: g(1), forward: g(2), ..default() }, g(3) as f32)),
                _ => pl.dev = Some((elite_sim::flight::Input { pitch: g(0), roll: g(1), yaw: g(2), ..default() }, g(3) as f32)),
            }
            None
        }
        "state" => {
            let ship = world.resource::<Sidewinder>();
            let sb = world.resource::<Sandbox>();
            match (ship.parked.as_ref(), sb.world.as_ref()) {
                (Some(p), Some(w)) if w.frame.obj == p.obj => {
                    let at = w.frame.from_body(p.pos);
                    info!("sidewinder: parked at {:.2} {:.2} {:.2} m in the sandbox's frame ({:.1} m from the eyes), {} solid pieces in", at.x, at.y, at.z, (at.as_vec3() - sb.eye).length(), if p.solids_epoch == Some(w.solids_cleared) { p.solids.len() } else { 0 });
                }
                (Some(_), _) => info!("sidewinder: parked on another body"),
                _ => info!("sidewinder: none (files read: {})", world.contains_resource::<SidewinderAssets>()),
            }
            let (seat, pl) = (world.resource::<seat::Seat>(), world.resource::<pilot::Pilot>());
            if seat.seated {
                let v = pl.motion.vel;
                info!(
                    "sidewinder: seated; {}, {:.2} m/s, throttle {:.2}, flight assist {}, gear {} ({:.2}), {} ticks; pips (half) WEP/ENG/SYS {:?}, capacitors {:.2}/{:.2}/{:.2} MJ, boost {:?}; drive {:?}, supercruise {}, glide {:?}, target {:?}, fuel {:.6} t main, {:.6} t reservoir; modules {:?}, destination {:?}, planet HUD {:?}, held {:?}, silent {}, scoop {}, lights {}, life {:?}, safe drop {:?}",
                    if pl.landed { "landed" } else { "flying" },
                    v.length(),
                    pl.throttle,
                    pl.flight_assist,
                    if pl.gear_down { "down" } else { "up" },
                    pl.gear_t,
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
                    info!("sidewinder: {:.3} km from {:?}'s centre, the nose {:.1} degrees over the horizon; heat {:.1} %, stars' heat {:.4} a second, shields {:.0} % ({}), hull {:.0} %, air {:.4} kg/m3", p.pos.length() / 1000.0, p.obj, elite_sim::glide::pitch_deg(nose.to_array(), down.to_array()), pl.heat_percent(), pl.star_heat, pl.shields.percent(), if pl.shields.up { "up" } else { "down" }, pl.hull.percent(), pl.air_density);
                }
            }
            None
        }
        _ => {
            warn!("sw: spawn | remove | restock | state | board | leave | throttle X | thrust LAT VERT FWD SECS | stick PITCH ROLL YAW SECS | fa on|off | gear | log on|off | pip eng|sys|wep|reset | boost | hardpoints | fire SECS | jump | target NAME|none | hyper NAME [fail] | face | put NAME ALT_KM | trace FILE|off | dest NAME|none | aim DEG | silent | scoop | lights | canopy [SECS] | pose up|down|auto");
            None
        }
    };
    if let Some(r) = req {
        world.resource_mut::<ModControl>().ship_request = Some(r);
    }
}
