//! The Faulcon DeLacy Sidewinder mod (`docs/spec-mods-sidewinder.md`): Elite Dangerous's starter ship, spawned from the Q
//! menu's Mods tab where the player looks, parked on its gear, a solid thing to walk round; boarded with E and flown by Elite's
//! rules (`crates/elite-sim`) in the steps that follow.
//!
//! - The parked ship's truth is its pose in its body's frame (`Parked`): its solid pieces in the sandbox's world and its
//!   meshes are made from that, and made again after the sandbox moves its frame (a rebase takes every solid piece out) or
//!   when the player comes back near it.
//! - Spawning is its own request (`ShipRequest`), not "the mod on": a parked ship stays when another mod is turned on
//!   (the spec's edge cases). Boarding is what will turn the mod on (step 1.3).
//! - Its files are read on a worker thread at the first spawn (`load`), checked as every mod's are (`mods::load_sidewinder`).
//! - `draw.rs` draws it as the props are drawn (`PropMaterial`, the sandbox's light records), one mesh per draw.
//! - `convert.rs` (tests only) makes the mod's files from the user's Elite install.

pub mod draw;
pub mod hud;
pub mod pilot;
pub mod seat;

#[cfg(test)]
mod convert;

use std::sync::Arc;

use bevy::math::{DQuat, DVec3, Mat3, Quat, Vec3};
use bevy::prelude::*;
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use se_core::catalog::ObjectRef;
use se_core::elite_asset::{parse_edm, Binds, Edm};

use crate::mods::{ModControl, ModRegistry};
use crate::sandbox::world::{SolidHandle, SolidShape, Surface};
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

/// The mod's files, read and checked at its first spawn (kept until quit).
#[derive(Resource, Clone)]
pub struct SidewinderAssets {
    pub ship: Arc<Edm>,
    pub cockpit: Arc<Edm>,
    /// Elite's default keyboard and mouse controls and its default pad's (copies of its `.binds` files).
    pub keys: Arc<Binds>,
    pub pad: Arc<Binds>,
    /// The ship's solid pieces and its box, at rest with the gear down (model space).
    pub shape: Arc<ShipShape>,
    /// The pilot's eyes in model space (the cockpit's part `eye`).
    pub eye: [f32; 3],
}

/// The parked ship's solid shape: convex pieces (one per draw), its box, and its feet.
#[derive(Clone, Debug)]
pub struct ShipShape {
    pub hulls: Vec<Vec<Vec3>>,
    pub lo: Vec3,
    pub hi: Vec3,
    /// The lowest point of each landing gear leg (where it stands).
    pub feet: Vec<Vec3>,
    /// The hull's lowest point in each cell of a 3 x 3 grid over its plan (what touches the ground with the gear up or the
    /// ship tilted).
    pub belly: Vec<Vec3>,
}

/// Each part's transform in model space at rest (its parents' composed), as a matrix.
pub fn rest_transforms(m: &Edm) -> Vec<Mat4> {
    let mut out: Vec<Mat4> = Vec::with_capacity(m.parts.len());
    for p in &m.parts {
        let local = Mat4::from_rotation_translation(Quat::from_array(p.rotation).normalize(), Vec3::from_array(p.translation));
        out.push(if p.parent >= 0 { out[p.parent as usize] * local } else { local });
    }
    out
}

/// Each part's transform with animation `name` at `t01` of its length (parts it doesn't move stay at rest).
pub fn posed_transforms(m: &Edm, name: &str, t01: f32) -> Vec<Mat4> {
    let anim = m.animations.iter().find(|a| a.name == name);
    let mut out: Vec<Mat4> = Vec::with_capacity(m.parts.len());
    for (i, p) in m.parts.iter().enumerate() {
        let (mut tr, mut rot) = (Vec3::from_array(p.translation), Quat::from_array(p.rotation).normalize());
        if let Some((t, secs)) = anim.and_then(|a| a.tracks.iter().find(|t| t.part as usize == i && !t.keys.is_empty()).map(|t| (t, a.seconds))) {
            let time = t01.clamp(0.0, 1.0) * secs;
            let k = t.keys.iter().rposition(|k| k.time <= time).unwrap_or(0);
            let (a, b) = (&t.keys[k], &t.keys[(k + 1).min(t.keys.len() - 1)]);
            let f = if b.time > a.time { ((time - a.time) / (b.time - a.time)).clamp(0.0, 1.0) } else { 0.0 };
            tr = Vec3::from_array(a.translation).lerp(Vec3::from_array(b.translation), f);
            rot = Quat::from_array(a.rotation).normalize().slerp(Quat::from_array(b.rotation).normalize(), f);
        }
        let local = Mat4::from_rotation_translation(rot, tr);
        out.push(if p.parent >= 0 { out[p.parent as usize] * local } else { local });
    }
    out
}

fn ship_shape(m: &Edm) -> ShipShape {
    let parts = rest_transforms(m);
    let mut hulls = Vec::new();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut feet = Vec::new();
    let mut gear = Vec::new();
    for d in &m.draws {
        let t = parts[d.part as usize];
        let pts: Vec<Vec3> = d.vertices.iter().map(|v| t.transform_point3(Vec3::from_array(m.position_m(v)))).collect();
        for p in &pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let is_gear = m.parts[d.part as usize].name.starts_with("gear");
        if is_gear {
            if let Some(f) = pts.iter().copied().min_by(|a, b| a.y.total_cmp(&b.y)) {
                feet.push(Vec3::new(t.w_axis.x, f.y, t.w_axis.z));
            }
        }
        gear.push(is_gear);
        hulls.push(pts);
    }
    // The belly: the lowest hull point (not the gear's) in each cell of a 3 x 3 grid over the plan.
    let mut cells: [Option<Vec3>; 9] = [None; 9];
    let size = (hi - lo).max(Vec3::splat(1e-3));
    for (pts, _) in hulls.iter().zip(&gear).filter(|(_, g)| !**g) {
        for p in pts {
            let i = (((p.x - lo.x) / size.x * 3.0) as usize).min(2) + 3 * (((p.z - lo.z) / size.z * 3.0) as usize).min(2);
            if cells[i].is_none_or(|c| p.y < c.y) {
                cells[i] = Some(*p);
            }
        }
    }
    let belly = cells.into_iter().flatten().collect();
    ShipShape { hulls, lo, hi, feet, belly }
}

/// Reads the mod's files through `read` (the mods menu's checked reader: a refusal names the file and why).
pub fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<SidewinderAssets, (String, String)> {
    let edm = |f: &str| -> Result<Edm, (String, String)> { parse_edm(&read(f).map_err(|e| (f.to_string(), e))?).map_err(|e| (f.to_string(), e.to_string())) };
    let binds = |f: &str| -> Result<Binds, (String, String)> {
        let b = read(f).map_err(|e| (f.to_string(), e))?;
        Binds::parse(&String::from_utf8_lossy(&b)).map_err(|e| (f.to_string(), e.to_string()))
    };
    let ship = edm("ship.edm")?;
    let shape = ship_shape(&ship);
    if shape.feet.is_empty() {
        return Err(("ship.edm".to_string(), "the ship has no landing gear to stand on".to_string()));
    }
    let cockpit = edm("cockpit.edm")?;
    let eye = cockpit.parts.iter().find(|p| p.name == "eye").map(|p| p.translation).ok_or_else(|| ("cockpit.edm".to_string(), "no eye point".to_string()))?;
    Ok(SidewinderAssets {
        eye,
        cockpit: Arc::new(cockpit),
        keys: Arc::new(binds("binds/KeyboardMouseOnly.binds")?),
        pad: Arc::new(binds("binds/ControlPad.binds")?),
        shape: Arc::new(shape),
        ship: Arc::new(ship),
    })
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
    loading: Option<Task<Result<SidewinderAssets, crate::mods::FileFault>>>,
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
                Ok(a) => {
                    info!("sidewinder: files read ({} draws, {} parts; {} key bindings)", a.ship.draws.len(), a.ship.parts.len(), a.keys.actions.len());
                    commands.insert_resource(a);
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
        match spawn_place(&sb, &a.shape) {
            Ok((at, axes)) => {
                if let Some(mut old) = ship.parked.take() {
                    remove_solids(&mut sb, &mut old);
                }
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
        let at = world.frame.from_body(p.pos).as_vec3();
        let rot = (world.frame.rotation().inverse() * p.rot).normalize().as_quat();
        for pts in &assets.shape.hulls {
            if let Some(h) = world.add_solid(&SolidShape::Hull(pts.clone()), at, rot, Surface::Metal) {
                p.solids.push(h);
            }
        }
        p.solids_epoch = Some(world.solids_cleared);
        world.wake_props();
        world.refresh();
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
            .add_systems(
                PostUpdate,
                (pilot::fly, seat::seat_camera)
                    .chain()
                    .run_if(resource_exists::<SidewinderAssets>)
                    .in_set(crate::universe::UniverseSet::Camera)
                    .after(crate::sandbox::sandbox_step)
                    .before(crate::viltrumite::hero::hero_move)
                    .before(crate::camera::sync_camera_transform),
            )
            .add_systems(bevy_egui::EguiPrimaryContextPass, (seat::draw_prompt, hud::draw_hud).run_if(resource_exists::<SidewinderAssets>).in_set(crate::skin::UiSet::Draw));
        // (The spec's C20: never spawned, nothing of the mod runs. Its systems wait for a request, then for its files.)
        app.add_systems(Update, handle_requests.run_if(asked))
            // After the sandbox's frame (its world may have been rebased this frame), before its props are drawn.
            .add_systems(PostUpdate, keep_solid.run_if(resource_exists::<SidewinderAssets>).after(crate::sandbox::sandbox_step));
    }
}

/// `sw spawn|remove|restock|state`: test commands (`--run "sw ..."`).
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
                    "sidewinder: seated; {}, {:.2} m/s, throttle {:.2}, flight assist {}, gear {} ({:.2}), {} ticks; pips (half) WEP/ENG/SYS {:?}, capacitors {:.2}/{:.2}/{:.2} MJ, boost {:?}; drive {:?}, supercruise {}, glide {:?}",
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
                    pl.cruise.map_or("no".to_string(), |c| format!("{:.0} km/s", c.speed / 1000.0)),
                    pl.glide
                );
                if let Some(p) = ship.parked.as_ref() {
                    let nose = p.rot * bevy::math::DVec3::NEG_Z;
                    let down = -p.pos.normalize_or(bevy::math::DVec3::Y);
                    info!("sidewinder: {:.3} km from {:?}'s centre, the nose {:.1} degrees over the horizon", p.pos.length() / 1000.0, p.obj, elite_sim::glide::pitch_deg(nose.to_array(), down.to_array()));
                }
            }
            None
        }
        _ => {
            warn!("sw: spawn | remove | restock | state | board | leave | throttle X | thrust LAT VERT FWD SECS | stick PITCH ROLL YAW SECS | fa on|off | gear | log on|off | pip eng|sys|wep|reset | boost");
            None
        }
    };
    if let Some(r) = req {
        world.resource_mut::<ModControl>().ship_request = Some(r);
    }
}
