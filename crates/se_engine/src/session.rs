//! SE's planetarium state (save/save-def.cfg `Planetarium`): `MoveMode`, `BindMode`, `RefBody`, the
//! position, `Orientation`, `Velocity`, `TimeScale` and `Time`/`Date`, saved on exit and restored on the
//! next start ("Continue"). The port writes `%APPDATA%/SpaceEngine-rs/save.cfg` in that syntax, with the
//! position as the port's own fixed-point universe coordinates (`AbsPosition`, exact) and the time as
//! SE's Julian Date * 2^80 hex (`places.rs`).
//!
//! Not ported: SE's first start from save-def.cfg (Saturn, Follow mode; its `RelPosition` encoding is not
//! decoded), so with no save the port starts by Earth as before. Scripted runs and `--start` neither read
//! nor write the save. `MoveMode` is the camera's movement mode (1 Free, 2 Spacecraft, 3 Aircraft) or 4 while a
//! spacecraft is flown (FUN_1401be6b0); SE then restores the piloted ship by its `Spacecraft` id
//! (FUN_1401beaa0). Here a saved 4 comes back as Free mode and `ship/persist.rs` flies the ship marked
//! `Piloted` in spacecraft.cfg again.

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use se_core::places::{fixed_to_f64, format_date, format_fixed, parse_fixed};
use se_core::upos::UPos;

use crate::camera::FlyCamera;
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::universe::SimTime;

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        // After bevy_window turns the closed window into AppExit (in Last too).
        app.add_systems(PostStartup, restore_session).add_systems(Last, save_on_exit.after(bevy::window::ExitSystems));
    }
}

fn save_path() -> std::path::PathBuf {
    crate::settings::user_file("save.cfg")
}

/// The saved state.
#[derive(Debug, Clone, PartialEq)]
struct Session {
    /// SE's `MoveMode` 1-4.
    move_mode: u8,
    bind_mode: u8,
    ref_body: String,
    pos: [i128; 3],
    /// (w, x, y, z), as SE's places.
    rot: [f64; 4],
    /// Assigned speed, parsecs per second (SE's `Velocity`; in Spacecraft and Aircraft mode SE writes
    /// the assigned acceleration times 100, FUN_1401be6b0, and reads it back times 0.01, FUN_1401beaa0).
    speed_pc: f64,
    rate: f64,
    jd: f64,
}

const TIME_SCALE: f64 = (1u128 << 80) as f64;

impl Session {
    fn to_cfg(&self) -> String {
        let [w, x, y, z] = self.rot;
        let [px, py, pz] = self.pos.map(format_fixed);
        format!(
            "// SpaceEngine-rs planetarium state\nLastMode    0\nPlanetarium\n{{\n\tMoveMode    {}\n\tBindMode    {}\n\tRefBody     \"{}\"\n\tAbsPosition ({px} {py} {pz})\n\tOrientation ({w:?} {x:?} {y:?} {z:?})\n\tVelocity    {:?}\n\tTimeScale   {:?}\n\tTime        {}\n\tDate        \"{}\"\n}}\n",
            self.move_mode,
            self.bind_mode,
            self.ref_body,
            self.speed_pc,
            self.rate,
            format_fixed((self.jd * TIME_SCALE) as i128),
            format_date(self.jd)
        )
    }

    /// Reads the `Planetarium` block; `None` when the position, orientation or time is missing.
    fn parse(text: &str) -> Option<Session> {
        let mut s = Session { move_mode: 1, bind_mode: 0, ref_body: String::new(), pos: [0; 3], rot: [1.0, 0.0, 0.0, 0.0], speed_pc: 0.0, rate: 1.0, jd: 0.0 };
        let (mut has_pos, mut has_rot, mut has_time) = (false, false, false);
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            let Some((key, rest)) = line.split_once(char::is_whitespace) else { continue };
            let rest = rest.trim();
            let list = || rest.trim_matches(|c| c == '(' || c == ')').split([' ', ',', '\t']).filter(|t| !t.is_empty()).collect::<Vec<_>>();
            match key {
                "MoveMode" => s.move_mode = rest.parse::<u8>().ok().filter(|m| (1..=4).contains(m)).unwrap_or(1),
                "BindMode" => s.bind_mode = rest.parse::<u8>().unwrap_or(0).min(2),
                "RefBody" => s.ref_body = rest.trim_matches('"').to_string(),
                "AbsPosition" => {
                    let v: Vec<i128> = list().into_iter().filter_map(parse_fixed).collect();
                    if let [x, y, z] = v[..] {
                        s.pos = [x, y, z];
                        has_pos = true;
                    }
                }
                "Orientation" => {
                    let v: Vec<f64> = list().into_iter().filter_map(|t| t.parse().ok()).collect();
                    if let [w, x, y, z] = v[..] {
                        s.rot = [w, x, y, z];
                        has_rot = true;
                    }
                }
                "Velocity" => s.speed_pc = rest.parse().unwrap_or(0.0),
                "TimeScale" => s.rate = rest.parse().unwrap_or(1.0),
                "Time" => {
                    if let Some(t) = parse_fixed(rest) {
                        s.jd = fixed_to_f64(t);
                        has_time = true;
                    }
                }
                _ => {}
            }
        }
        (has_pos && has_rot && has_time).then_some(s)
    }
}

fn restore_session(cfg: Res<EngineConfig>, data: Res<SeData>, mut cam: ResMut<FlyCamera>, mut sim: ResMut<SimTime>) {
    if crate::settings::scripted(&cfg) || cfg.start_given {
        return;
    }
    let Some(s) = std::fs::read_to_string(save_path()).ok().as_deref().and_then(Session::parse) else { return };
    let cat = &data.catalog;
    sim.jd = s.jd;
    sim.rate = s.rate;
    cam.pos = UPos { x: s.pos[0], y: s.pos[1], z: s.pos[2] };
    let [w, x, y, z] = s.rot;
    cam.rot = DQuat::from_xyzw(x, y, z, w).normalize();
    cam.velocity = DVec3::ZERO;
    cam.travel = None;
    cam.follow = None;
    let mode = crate::camera::MoveMode::from_number(s.move_mode).unwrap_or_else(|| {
        warn!("the saved spacecraft cannot be flown again; continuing in Free mode");
        crate::camera::MoveMode::Free
    });
    cam.set_move_mode(mode);
    let scale = if mode.inertial() { SAVE_ACCEL_SCALE.recip() } else { 1.0 };
    cam.speed = crate::camera::clamp_speed(s.speed_pc * scale * se_core::units::METERS_PER_PC);
    cam.selected = cat.find(&s.ref_body);
    if let (Some(r), 1..) = (cam.selected, s.bind_mode) {
        cam.bind(Some(r), s.bind_mode == 2, &se_core::ephemeris::Ephemeris::new(cat), s.jd);
    }
    info!("restored the session from {}", save_path().display());
}

/// SE's save.cfg `Velocity` in the inertial modes: the assigned acceleration times this
/// (DAT_1406d5100; read back times DAT_1406d4698 0.01).
const SAVE_ACCEL_SCALE: f64 = 100.0;

pub(crate) fn save_on_exit(
    mut exits: MessageReader<AppExit>,
    cfg: Res<EngineConfig>,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    sim: Res<SimTime>,
    flight: Option<Res<crate::ship::Flight>>,
) {
    if exits.read().count() == 0 || crate::settings::scripted(&cfg) || cfg.start_given {
        return;
    }
    let cat = &data.catalog;
    let rot = cam.rot.normalize();
    let piloting = flight.is_some_and(|f| f.active && f.ship.is_some());
    // SE's MoveMode: 4 while a spacecraft is flown (FUN_1401be6b0), which comes back as Free mode.
    let move_mode = if piloting { 4 } else { cam.move_mode.number() };
    // The Velocity scale goes with the mode written: x100 for 2 and 3, else the camera's assigned
    // velocity (an acceleration back into one; flying a ship sets Free mode, so this is a guard).
    let speed = match (move_mode, cam.move_mode.inertial()) {
        (2 | 3, _) => cam.speed * SAVE_ACCEL_SCALE,
        (_, true) => cam.speed * crate::camera::MODE_SPEED_FACTOR,
        _ => cam.speed,
    };
    let s = Session {
        move_mode,
        bind_mode: cam.bind_mode(),
        // SE's RefBody: the body the camera is bound to, else the selection.
        ref_body: cam.follow.map(|f| f.target).or(cam.selected).map(|r| cat.display_name(r).to_string()).unwrap_or_default(),
        pos: [cam.pos.x, cam.pos.y, cam.pos.z],
        rot: [rot.w, rot.x, rot.y, rot.z],
        speed_pc: speed / se_core::units::METERS_PER_PC,
        rate: sim.rate,
        jd: sim.jd,
    };
    let path = save_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match crate::settings::write_atomic(&path, &s.to_cfg()) {
        Ok(()) => info!("saved the session to {}", path.display()),
        Err(e) => warn!("cannot save the session to {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trips() {
        let s = Session {
            move_mode: 3,
            bind_mode: 2,
            ref_body: "Saturn".into(),
            pos: [-123_456_789_012_345_678_901_234_567, 42, 1 << 100],
            rot: [0.5, -0.5, 0.5, 0.5],
            speed_pc: 7.812611668711611e-10,
            rate: -1000.0,
            jd: 2_461_340.123_456_789,
        };
        let back = Session::parse(&s.to_cfg()).unwrap();
        assert_eq!(back, s);
        assert!(s.to_cfg().contains("Date        \"2026."));
        assert_eq!(Session::parse("Planetarium\n{\n\tBindMode 1\n}\n"), None);
        assert!(s.to_cfg().contains("MoveMode    3\n"));
        // A bad MoveMode falls back to Free mode.
        let bad = s.to_cfg().replace("MoveMode    3", "MoveMode    7");
        assert_eq!(Session::parse(&bad).unwrap().move_mode, 1);
    }
}
