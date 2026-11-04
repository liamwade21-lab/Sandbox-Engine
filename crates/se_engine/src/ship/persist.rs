//! Ships kept between sessions, as SE's `spacecraft.cfg` (seSpacecraftsList::SaveSpacecraftsConfig
//! FUN_1401f5510 writes one block per ship with FUN_1405339e0; seSpacecraftList::ReadShipsConfig FUN_1401f56c0
//! reads them back, skipping a block without `ShipID`). The port writes `%APPDATA%/SpaceEngine-rs-v2/spacecraft.cfg`
//! on exit with SE's keys (`ShipID`, `RefBodyID`, `Model`, `UnivPos`, `Orientation`, `Velocity`,
//! `AngVelocity`, `WarpBoost`), the position in the port's fixed-point universe coordinates (as `save.cfg`),
//! velocities in m/s and rad/s, plus `Mine`, `Piloted` and, for a landed ship, `LandedPos`, `LandedRot` and
//! `LandedLevel` ([`SavedLanding`]); it rebuilds them on the next start, and flies again the one that was flown. Scripted runs and `--start` neither read nor write it, like the session.
//!
//! Not kept: SE's `SystemID`, `TargetID` / `TargetShipID`, fuel, propellant and heat (the port does not model
//! them), docking (`DockedShipID` / `DockedPort`), `HUDMode`, `PhysMode` and the autopilot programme. An idle
//! ship parks again at the body nearest its saved place (`park_ships`).

use std::sync::Arc;

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use se_core::places::{format_fixed, parse_fixed};
use se_core::ship::ShipDesign;
use se_core::upos::UPos;

use super::{Flight, Landed, Ship, spawn_built, take_control};
use crate::camera::FlyCamera;
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::ship_assets::HullDefaults;
use crate::universe::{UOrientation, UPosition};

fn ships_path() -> std::path::PathBuf {
    test_file().unwrap_or_else(|| crate::settings::user_file("spacecraft.cfg"))
}

/// `SE_SHIPS_FILE` (testing): the file to use instead, read and written in scripted runs too.
fn test_file() -> Option<std::path::PathBuf> {
    std::env::var_os("SE_SHIPS_FILE").map(std::path::PathBuf::from)
}

/// One saved ship.
#[derive(Debug, Clone, PartialEq)]
struct SavedShip {
    name: String,
    id: u32,
    ref_body: String,
    model: String,
    pos: [i128; 3],
    /// (w, x, y, z)
    rot: [f64; 4],
    vel: [f64; 3],
    ang_vel: [f64; 3],
    warp: f64,
    mine: bool,
    piloted: bool,
    landed: Option<SavedLanding>,
}

/// Where a landed ship stands on its `RefBodyID` (the port's `LandedPos`, `LandedRot` and
/// `LandedLevel`): it lands there again once the body exists, rather than falling onto whatever
/// coarse ground has loaded by then.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SavedLanding {
    /// Body axes (m).
    pos: [f64; 3],
    /// (w, x, y, z), body axes.
    rot: [f64; 4],
    /// [`super::Landed::level`].
    level: Option<u8>,
}

impl SavedShip {
    fn to_cfg(&self) -> String {
        let [px, py, pz] = self.pos.map(format_fixed);
        let [w, x, y, z] = self.rot;
        let [vx, vy, vz] = self.vel;
        let [ax, ay, az] = self.ang_vel;
        let landed = self.landed.map_or(String::new(), |l| {
            let ([lx, ly, lz], [lw, qx, qy, qz]) = (l.pos, l.rot);
            let level = l.level.map_or(-1, i32::from);
            format!("\tLandedPos   ({lx:?} {ly:?} {lz:?})\n\tLandedRot   ({lw:?} {qx:?} {qy:?} {qz:?})\n\tLandedLevel  {level}\n")
        });
        format!(
            "Spacecraft\t\"{}\"\n{{\n\tShipID       {}\n\tRefBodyID   \"{}\"\n\tModel       \"{}\"\n\tUnivPos     ({px} {py} {pz})\n\tOrientation ({w:?} {x:?} {y:?} {z:?})\n\tVelocity    ({vx:?} {vy:?} {vz:?})\n\tAngVelocity ({ax:?} {ay:?} {az:?})\n\tWarpBoost    {:?}\n\tMine         {}\n\tPiloted      {}\n{landed}}}\n\n",
            self.name, self.id, self.ref_body, self.model, self.warp, self.mine, self.piloted
        )
    }

    /// Every `Spacecraft` block with a `ShipID`, a `Model` and a position.
    fn parse_all(text: &str) -> Vec<SavedShip> {
        let mut out = Vec::new();
        let mut cur: Option<(SavedShip, bool, bool)> = None;
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("Spacecraft") {
                let name = rest.trim().trim_matches('"').to_string();
                let blank = SavedShip {
                    name,
                    id: 0,
                    ref_body: String::new(),
                    model: String::new(),
                    pos: [0; 3],
                    rot: [1.0, 0.0, 0.0, 0.0],
                    vel: [0.0; 3],
                    ang_vel: [0.0; 3],
                    warp: 0.0,
                    mine: true,
                    piloted: false,
                    landed: None,
                };
                cur = Some((blank, false, false));
                continue;
            }
            if line == "}" {
                if let Some((s, has_id, has_pos)) = cur.take() {
                    if has_id && has_pos && !s.model.is_empty() {
                        out.push(s);
                    } else if !has_id {
                        warn!("spacecraft \"{}\" has no ShipID", s.name);
                    }
                }
                continue;
            }
            let Some((s, has_id, has_pos)) = cur.as_mut() else { continue };
            let Some((key, rest)) = line.split_once(char::is_whitespace) else { continue };
            let rest = rest.trim();
            let list = || rest.trim_matches(|c| c == '(' || c == ')').split([' ', ',', '\t']).filter(|t| !t.is_empty()).collect::<Vec<_>>();
            let floats = || list().into_iter().filter_map(|t| t.parse::<f64>().ok()).collect::<Vec<_>>();
            match key {
                "ShipID" => {
                    if let Ok(v) = rest.parse() {
                        s.id = v;
                        *has_id = true;
                    }
                }
                "RefBodyID" => s.ref_body = rest.trim_matches('"').to_string(),
                "Model" => s.model = rest.trim_matches('"').to_string(),
                "UnivPos" => {
                    if let [x, y, z] = list().into_iter().filter_map(parse_fixed).collect::<Vec<_>>()[..] {
                        s.pos = [x, y, z];
                        *has_pos = true;
                    }
                }
                "Orientation" => {
                    if let [w, x, y, z] = floats()[..] {
                        s.rot = [w, x, y, z];
                    }
                }
                "Velocity" => {
                    if let [x, y, z] = floats()[..] {
                        s.vel = [x, y, z];
                    }
                }
                "AngVelocity" => {
                    if let [x, y, z] = floats()[..] {
                        s.ang_vel = [x, y, z];
                    }
                }
                "WarpBoost" => s.warp = rest.parse().unwrap_or(0.0),
                "Mine" => s.mine = rest != "false",
                "Piloted" => s.piloted = rest == "true",
                "LandedPos" | "LandedRot" | "LandedLevel" => {
                    let l = s.landed.get_or_insert(SavedLanding { pos: [0.0; 3], rot: [1.0, 0.0, 0.0, 0.0], level: None });
                    match (key, &floats()[..]) {
                        ("LandedPos", &[x, y, z]) => l.pos = [x, y, z],
                        ("LandedRot", &[w, x, y, z]) => l.rot = [w, x, y, z],
                        ("LandedLevel", _) => l.level = rest.parse::<u8>().ok(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        out
    }
}

pub(super) fn save_ships_on_exit(
    mut exits: MessageReader<AppExit>,
    cfg: Res<EngineConfig>,
    data: Res<SeData>,
    flight: Res<Flight>,
    ships: Query<(Entity, &Ship, &UPosition, &UOrientation)>,
) {
    if exits.read().count() == 0 || (test_file().is_none() && (crate::settings::scripted(&cfg) || cfg.start_given)) {
        return;
    }
    let cat = &data.catalog;
    let mut text = String::from("// SpaceEngine-rs spacecraft\n\n");
    for (e, ship, pos, rot) in &ships {
        // SE's RefBodyID: the body the ship rests on or holds station at.
        let anchor = ship.landed.map(|l| l.body).or(ship.parked.map(|p| p.body));
        let ref_body = anchor.map(|b| cat.display_name(b).to_string()).unwrap_or_default();
        let q = rot.0.normalize();
        let s = SavedShip {
            name: if ship.nickname.is_empty() { ship.design.name.clone() } else { ship.nickname.clone() },
            id: ship.serial,
            ref_body,
            model: ship.design.path.clone(),
            pos: [pos.0.x, pos.0.y, pos.0.z],
            rot: [q.w, q.x, q.y, q.z],
            vel: ship.vel.to_array(),
            ang_vel: ship.ang_vel.to_array(),
            warp: ship.warp,
            mine: ship.mine,
            piloted: flight.active && flight.ship == Some(e),
            landed: ship.landed.map(|l| {
                let q = l.local_rot.normalize();
                SavedLanding { pos: l.local_pos.to_array(), rot: [q.w, q.x, q.y, q.z], level: l.level }
            }),
        };
        text.push_str(&s.to_cfg());
    }
    let path = ships_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match crate::settings::write_atomic(&path, &text) {
        Ok(()) => info!("saved {} spacecraft to {}", ships.iter().count(), path.display()),
        Err(e) => warn!("cannot save the spacecraft to {}: {e}", path.display()),
    }
}

/// Restored idle ships and the name of the body they were parked at: an idle ship parks at the nearest
/// body that exists, so it parks again once that one does (bodies appear as their system is made). The
/// name is looked up until it is found: a procedural star or a generated planet is in the catalog
/// only once its system has been made, after the ships are restored. A ship saved landed (piloted
/// or not) lands on it again where it stood.
#[derive(Resource, Default)]
pub(super) struct PendingRefs(Vec<(Entity, String, f64, Option<SavedLanding>)>);

/// Seconds a restored ship waits for its body.
const REF_WAIT_S: f64 = 30.0;

pub(super) fn settle_restored(
    time: Res<Time>,
    data: Res<SeData>,
    mut pending: ResMut<PendingRefs>,
    bodies: Query<&crate::system::SystemBody>,
    mut ships: Query<&mut Ship>,
) {
    let now = time.elapsed_secs_f64();
    pending.0.retain(|(e, name, t0, landing)| {
        let (e, t0) = (*e, *t0);
        if let Some(r) = data.catalog.find(name)
            && bodies.iter().any(|b| b.obj == r)
        {
            if let Ok(mut ship) = ships.get_mut(e) {
                ship.parked = None;
                if let Some(l) = landing {
                    let [w, x, y, z] = l.rot;
                    ship.landed = Some(Landed {
                        body: r,
                        local_pos: DVec3::from(l.pos),
                        local_rot: DQuat::from_xyzw(x, y, z, w).normalize(),
                        level: l.level,
                    });
                }
            }
            return false;
        }
        now - t0 < REF_WAIT_S
    });
}

/// Rebuilds the saved ships once the hull textures exist (first frames).
#[allow(clippy::too_many_arguments)]
pub(super) fn restore_ships(
    mut done: Local<bool>,
    time: Res<Time>,
    mut pending: ResMut<PendingRefs>,
    mut commands: Commands,
    cfg: Res<EngineConfig>,
    data: Res<SeData>,
    hull: Option<Res<HullDefaults>>,
    mut flight: ResMut<Flight>,
    mut cam: ResMut<FlyCamera>,
) {
    if *done {
        return;
    }
    let Some(hull) = hull else { return };
    *done = true;
    if test_file().is_none() && (crate::settings::scripted(&cfg) || cfg.start_given) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(ships_path()) else { return };
    let saved = SavedShip::parse_all(&text);
    for s in &saved {
        let Some(design) = ShipDesign::load(&data.vfs, &s.model) else {
            warn!("spacecraft \"{}\": cannot load its model {}", s.name, s.model);
            continue;
        };
        let design = Arc::new(design);
        let [w, x, y, z] = s.rot;
        let pos = UPos { x: s.pos[0], y: s.pos[1], z: s.pos[2] };
        let orient = DQuat::from_xyzw(x, y, z, w).normalize();
        let e = spawn_built(&mut commands, &data, &mut flight, design.clone(), (hull.thermal_shift, hull.model_bright), pos, orient, DVec3::from(s.vel), s.mine);
        let nickname = if s.name == design.name { String::new() } else { s.name.clone() };
        let (id, ang_vel, warp) = (s.id, DVec3::from(s.ang_vel), s.warp);
        commands.entity(e).queue(move |mut ent: EntityWorldMut| {
            if let Some(mut ship) = ent.get_mut::<Ship>() {
                (ship.serial, ship.nickname, ship.ang_vel, ship.warp) = (id, nickname, ang_vel, warp);
            }
        });
        flight.next_serial = flight.next_serial.max(id);
        let landing = s.landed.filter(|l| DVec3::from(l.pos) != DVec3::ZERO);
        if s.piloted {
            take_control(&mut flight, &mut cam, e, &design, false);
        }
        if !s.ref_body.is_empty() && (!s.piloted || landing.is_some()) {
            pending.0.push((e, s.ref_body.clone(), time.elapsed_secs_f64(), landing));
        }
    }
    if !saved.is_empty() {
        info!("restored {} spacecraft from {}", saved.len(), ships_path().display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacecraft_round_trip() {
        let s = SavedShip {
            name: "Red Dwarf".into(),
            id: 7,
            ref_body: "Moon".into(),
            model: "ships/Fighter.sss".into(),
            pos: [-123_456_789_012_345_678_901, 42, 1 << 90],
            rot: [0.5, -0.5, 0.5, 0.5],
            vel: [1.5, -2.25e3, 3.0e-7],
            ang_vel: [0.0, 0.1, -0.2],
            warp: 2.5,
            mine: false,
            piloted: true,
            landed: None,
        };
        // Landed ones keep where they stand on the body, and how fine the ground there was.
        let on = |level| SavedShip { landed: Some(SavedLanding { pos: [1.0e6, -2.5, 1.737e6], rot: [0.0, 1.0, 0.0, 0.0], level }), ..s.clone() };
        let (a, b) = (on(Some(14)), on(None));
        let text = format!("// header\n\n{}{}{}{}", s.to_cfg(), a.to_cfg(), b.to_cfg(), "Spacecraft \"no id\"\n{\n\tModel \"x\"\n}\n");
        assert_eq!(SavedShip::parse_all(&text), vec![s, a, b]);
    }
}
