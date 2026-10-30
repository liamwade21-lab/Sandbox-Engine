//! SE's flight HUD (seSpacecraftHUD) and ship panel.
//!
//! * HUD modes Off / Horizontal / Orbital / Warp / Docking HUD: ShipOffHUD .. ShipDockingHUD,
//!   Ctrl+1 .. Ctrl+5 in SE's key table (FUN_1405ba8f0). Each draws 3D pointers over the view in
//!   the skin's colours (`ShipHUDColor`, `Ship*PointerColor`): the nose (ShipNoseVector), the
//!   reference body and the velocity relative to it (ShipRefBodyVector, ShipRefBodyVelVector), the
//!   target and the velocity relative to it (ShipTargetVector, ShipTargetVelVector), the delta-v
//!   that points the motion at the target (ShipTargetDeltaVVector, ShipWarpDeltaVVector) and the
//!   boosted velocity (ShipBoostVelVector). Which pointers each mode shows was not decoded: the
//!   port gives the Horizontal HUD a pitch ladder over the reference body's horizon, the Orbital
//!   HUD the reference body's pointers, the Warp HUD the boost and warp pointers, the Docking HUD
//!   the target's.
//! * The ship panel (main-def.cfg `LockShipPanel`) with SE's rows: Altitude / Elevation, Ship /
//!   Body / Rel / Surf / Air velocity, 1st / 2nd cosmic vel, Grav / Aero / Total accel, Thrust,
//!   g Force, Tidal force, Angular velocity / accel, the engine groups, Orbital parameters with
//!   Stable / Unstable orbit, Planet radius and Atmo height, and in warp Boost factor, Physical /
//!   Effective velocity, Efficiency and Arrival after.

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use se_core::units::{format_distance, format_speed};

use super::autopilot::Attitude;
use super::{Flight, Ship};
use crate::camera::FlyCamera;

/// SE's standard gravity for the g Force row (m/s^2).
pub const G0: f64 = 9.80665;

/// seSpacecraftHUD's modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudMode {
    Off,
    Horizontal,
    #[default]
    Orbital,
    Warp,
    Docking,
}

impl HudMode {
    pub const ALL: [HudMode; 5] = [HudMode::Off, HudMode::Horizontal, HudMode::Orbital, HudMode::Warp, HudMode::Docking];

    /// SE's names ("Off HUD" ..).
    pub fn label(self) -> &'static str {
        match self {
            HudMode::Off => "Off HUD",
            HudMode::Horizontal => "Horizontal HUD",
            HudMode::Orbital => "Orbital HUD",
            HudMode::Warp => "Warp HUD",
            HudMode::Docking => "Docking HUD",
        }
    }

    /// The script name (`flight hud off|horizontal|orbital|warp|docking`).
    pub fn parse(s: &str) -> Option<HudMode> {
        Some(match s.to_ascii_lowercase().as_str() {
            "off" => HudMode::Off,
            "horizontal" | "horizon" => HudMode::Horizontal,
            "orbital" => HudMode::Orbital,
            "warp" => HudMode::Warp,
            "docking" => HudMode::Docking,
            _ => return None,
        })
    }

    /// Ctrl+1 .. Ctrl+5 (ShipOffHUD .. ShipDockingHUD).
    pub fn from_key(k: KeyCode) -> Option<HudMode> {
        let keys = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5];
        keys.iter().position(|d| *d == k).map(|i| HudMode::ALL[i])
    }
}

/// Orbital parameters around the reference body.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orbit {
    /// Periapsis and apoapsis radii (m; apoapsis infinite on an escape path).
    pub periapsis: f64,
    pub apoapsis: f64,
    pub eccentricity: f64,
    /// Inclination to the body's equator (rad).
    pub inclination: f64,
    /// Period (s; infinite on an escape path).
    pub period: f64,
}

/// Keplerian orbit of relative position `r`, velocity `v` around `mu` (m^3/s^2), inclination to
/// the plane normal to `axis`.
pub fn orbit_from_state(r: DVec3, v: DVec3, mu: f64, axis: DVec3) -> Option<Orbit> {
    let rl = r.length();
    if mu <= 0.0 || rl <= 0.0 {
        return None;
    }
    let h = r.cross(v);
    let e_vec = v.cross(h) / mu - r / rl;
    let e = e_vec.length();
    let p = h.length_squared() / mu;
    let periapsis = p / (1.0 + e);
    let energy = v.length_squared() * 0.5 - mu / rl;
    let (apoapsis, period) = if e < 1.0 && energy < 0.0 {
        let a = -mu / (2.0 * energy);
        (a * (1.0 + e), std::f64::consts::TAU * (a * a * a / mu).sqrt())
    } else {
        (f64::INFINITY, f64::INFINITY)
    };
    let inclination = if h.length() > 0.0 { h.normalize().dot(axis.normalize_or(DVec3::Y)).clamp(-1.0, 1.0).acos() } else { 0.0 };
    Some(Orbit { periapsis, apoapsis, eccentricity: e, inclination, period })
}

/// Stable orbit: bound, and the periapsis clears the surface and the atmosphere.
pub fn orbit_stable(o: &Orbit, radius: f64, atmo_height: f64) -> bool {
    o.eccentricity < 1.0 && o.periapsis > radius + atmo_height.max(0.0)
}

/// The velocity change that turns `rel_vel` straight at `to_target` keeping its speed (the
/// delta-v pointers).
pub fn delta_v_to(to_target: DVec3, rel_vel: DVec3) -> DVec3 {
    to_target.normalize_or_zero() * rel_vel.length() - rel_vel
}

/// SE's panel numbers, filled by the physics.
#[derive(Clone, Debug, Default)]
pub struct PanelInfo {
    pub valid: bool,
    pub elevation: f64,
    pub ship_speed: f64,
    pub body_speed: f64,
    pub rel_speed: f64,
    pub surf_speed: f64,
    pub circular: f64,
    pub escape: f64,
    pub grav: f64,
    pub thrust: f64,
    pub total: f64,
    pub g_force: f64,
    pub tidal: f64,
    pub ang_vel: f64,
    pub ang_accel: f64,
    pub orbit: Option<Orbit>,
    pub stable: bool,
    pub planet_radius: f64,
    pub atmo_height: Option<f64>,
    /// Current and commanded boost factors (linear).
    pub boost: f64,
    pub boost_cmd: f64,
    pub arrival: Option<f64>,
    /// Pointer directions (universe axes): nose, local vertical, to the reference body, velocity
    /// relative to it, to the target and relative to it, the boosted velocity.
    pub nose: DVec3,
    pub up: DVec3,
    pub to_ref: DVec3,
    pub ref_vel: DVec3,
    pub to_target: Option<DVec3>,
    pub target_vel: DVec3,
    pub warp_vel: DVec3,
}

impl PanelInfo {
    /// Rates from the reference body's state: `r` from its centre, `body_v` its velocity,
    /// `surf_v` the velocity of its surface under the ship, `gm`, `axis` its rotation axis.
    #[allow(clippy::too_many_arguments)]
    pub fn body_rows(&mut self, r: DVec3, vel: DVec3, body_v: DVec3, surf_v: DVec3, gm: f64, axis: DVec3, radius: f64, atmo: Option<f64>, length: f64) {
        let rl = r.length().max(1.0);
        self.valid = true;
        self.elevation = rl - radius;
        self.ship_speed = vel.length();
        self.body_speed = body_v.length();
        self.rel_speed = (vel - body_v).length();
        self.surf_speed = (vel - surf_v).length();
        self.circular = (gm / rl).sqrt();
        self.escape = (2.0 * gm / rl).sqrt();
        self.tidal = 2.0 * gm * length / (rl * rl * rl);
        self.orbit = orbit_from_state(r, vel - body_v, gm, axis);
        self.stable = self.orbit.is_some_and(|o| orbit_stable(&o, radius, atmo.unwrap_or(0.0)));
        self.planet_radius = radius;
        self.atmo_height = atmo;
        self.up = r / rl;
        self.to_ref = -r / rl;
    }
}

/// Ctrl+1 .. Ctrl+5: the HUD mode.
pub fn hud_keys(keys: &ButtonInput<KeyCode>, flight: &mut Flight) {
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let other = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::AltLeft, KeyCode::AltRight]);
    if !ctrl || other {
        return;
    }
    for k in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5] {
        if keys.just_pressed(k) {
            if let Some(m) = HudMode::from_key(k) {
                flight.hud = m;
            }
        }
    }
}

fn color(text: &crate::skin::SkinText, key: &str, default: [f32; 4]) -> egui::Color32 {
    let c = text.rgba(key, default);
    egui::Color32::from_rgba_unmultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8)
}

/// Projects universe directions to screen points.
struct Projector {
    inv_rot: bevy::math::DQuat,
    tan: (f64, f64),
    rect: egui::Rect,
}

impl Projector {
    fn point(&self, dir: DVec3) -> Option<egui::Pos2> {
        let v = self.inv_rot * dir;
        if v.z >= -1e-6 {
            return None;
        }
        let (x, y) = (v.x / -v.z / self.tan.0, v.y / -v.z / self.tan.1);
        if x.abs() > 1.2 || y.abs() > 1.2 {
            return None;
        }
        let c = self.rect.center();
        Some(egui::pos2(c.x + (x as f32) * self.rect.width() * 0.5, c.y - (y as f32) * self.rect.height() * 0.5))
    }
}

/// Pointer shapes.
#[derive(Clone, Copy)]
enum Mark {
    Nose,
    Pro,
    Retro,
    Body,
    Target,
    DeltaV,
    Boost,
}

fn draw_mark(p: &egui::Painter, at: egui::Pos2, mark: Mark, c: egui::Color32) {
    let s = egui::Stroke::new(1.5, c);
    let r = 9.0;
    let v = |x: f32, y: f32| at + egui::vec2(x, y);
    match mark {
        Mark::Nose => {
            p.line_segment([v(-18.0, 0.0), v(-8.0, 0.0)], s);
            p.line_segment([v(-8.0, 0.0), v(-4.0, 6.0)], s);
            p.line_segment([v(-4.0, 6.0), v(0.0, 0.0)], s);
            p.line_segment([v(0.0, 0.0), v(4.0, 6.0)], s);
            p.line_segment([v(4.0, 6.0), v(8.0, 0.0)], s);
            p.line_segment([v(8.0, 0.0), v(18.0, 0.0)], s);
        }
        Mark::Pro => {
            p.circle_stroke(at, r, s);
            p.line_segment([v(-r, 0.0), v(-r - 7.0, 0.0)], s);
            p.line_segment([v(r, 0.0), v(r + 7.0, 0.0)], s);
            p.line_segment([v(0.0, -r), v(0.0, -r - 7.0)], s);
        }
        Mark::Retro => {
            p.circle_stroke(at, r, s);
            let d = r * 0.7;
            p.line_segment([v(-d, -d), v(d, d)], s);
            p.line_segment([v(-d, d), v(d, -d)], s);
        }
        Mark::Body => {
            let pts = vec![v(0.0, -r - 2.0), v(r + 2.0, 0.0), v(0.0, r + 2.0), v(-r - 2.0, 0.0), v(0.0, -r - 2.0)];
            p.add(egui::Shape::line(pts, s));
        }
        Mark::Target => {
            p.rect_stroke(egui::Rect::from_center_size(at, egui::vec2(2.0 * r, 2.0 * r)), 0.0, s, egui::StrokeKind::Middle);
            p.circle_filled(at, 1.5, c);
        }
        Mark::DeltaV => {
            let pts = vec![v(0.0, -r), v(r, r * 0.7), v(-r, r * 0.7), v(0.0, -r)];
            p.add(egui::Shape::line(pts, s));
        }
        Mark::Boost => {
            p.circle_stroke(at, r, s);
            p.circle_stroke(at, r * 0.45, s);
        }
    }
}

/// The Horizontal HUD's pitch ladder: lines every 10 degrees of pitch over the reference body's
/// horizon, around the nose's heading.
fn draw_ladder(p: &egui::Painter, proj: &Projector, nose: DVec3, up: DVec3, c: egui::Color32) {
    let h = (nose - up * nose.dot(up)).normalize_or_zero();
    if h == DVec3::ZERO {
        return;
    }
    let right = h.cross(up);
    for k in -9..=9 {
        let a = (k as f64 * 10.0).to_radians();
        let d = h * a.cos() + up * a.sin();
        let (Some(c0), Some(c1)) = (proj.point(d), proj.point(d + right * 0.02)) else { continue };
        let dir = (c1 - c0).normalized();
        let half = if k == 0 { 160.0 } else { 60.0 };
        let gap = if k == 0 { 0.0 } else { 18.0 };
        let s = egui::Stroke::new(if k == 0 { 1.5 } else { 1.0 }, c);
        if k >= 0 {
            p.line_segment([c0 - dir * half, c0 - dir * gap], s);
            p.line_segment([c0 + dir * gap, c0 + dir * half], s);
        } else {
            // Below the horizon dashed.
            for i in 0..4 {
                let t0 = gap + (half - gap) * i as f32 / 4.0;
                let t1 = t0 + (half - gap) / 8.0;
                p.line_segment([c0 - dir * t0, c0 - dir * t1], s);
                p.line_segment([c0 + dir * t0, c0 + dir * t1], s);
            }
        }
        if k != 0 {
            p.text(c0 + dir * (half + 4.0), egui::Align2::LEFT_CENTER, format!("{}", k * 10), egui::FontId::monospace(11.0), c);
        }
    }
}

/// The 3D pointers of the current HUD mode.
pub fn draw_hud_pointers(
    mut contexts: EguiContexts,
    flight: Res<Flight>,
    cam: Res<FlyCamera>,
    hud: Res<crate::hud::HudState>,
    ships: Query<&Ship>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !flight.active || flight.remote || hud.hidden || flight.hud == HudMode::Off {
        return Ok(());
    }
    let Some(ship) = flight.ship.and_then(|e| ships.get(e).ok()) else { return Ok(()) };
    let info = &flight.info.panel;
    if !info.valid {
        return Ok(());
    }
    let rect = ctx.content_rect();
    let tan_y = (cam.fov_deg.to_radians() as f64 * 0.5).tan();
    let proj = Projector { inv_rot: cam.rot.inverse(), tan: (tan_y * rect.width() as f64 / rect.height() as f64, tan_y), rect };
    let text = crate::skin::text(ctx);
    let hud_c = color(&text, "ShipHUDColor", [0.3, 1.0, 0.3, 1.0]);
    let ref_c = color(&text, "ShipRefBodyPointerColor", [1.0, 1.0, 0.5, 1.0]);
    let tgt_c = color(&text, "ShipTargetPointerColor", [1.0, 0.2, 0.2, 1.0]);
    let vel_c = color(&text, "ShipVelocityPointerColor", [0.5, 1.0, 1.0, 1.0]);
    let tdv_c = color(&text, "ShipTargetDeltaVPointerColor", [0.4, 0.4, 1.0, 1.0]);
    let wdv_c = color(&text, "ShipWarpDeltaVPointerColor", [0.4, 0.4, 1.0, 1.0]);
    let boost_c = color(&text, "ShipBoostVelPointerColor", [0.8, 0.1, 1.0, 1.0]);
    let painter = ctx.layer_painter(egui::LayerId::background());
    let put = |dir: DVec3, mark: Mark, c: egui::Color32| {
        if dir.length_squared() > 0.0 {
            if let Some(at) = proj.point(dir.normalize()) {
                draw_mark(&painter, at, mark, c);
            }
        }
    };
    let min_v = 1e-3;
    match flight.hud {
        HudMode::Off => {}
        HudMode::Horizontal => {
            draw_ladder(&painter, &proj, info.nose, info.up, hud_c);
            if info.ref_vel.length() > min_v {
                put(info.ref_vel, Mark::Pro, vel_c);
            }
        }
        HudMode::Orbital => {
            put(info.to_ref, Mark::Body, ref_c);
            put(-info.to_ref, Mark::Body, ref_c);
            if info.ref_vel.length() > min_v {
                put(info.ref_vel, Mark::Pro, vel_c);
                put(-info.ref_vel, Mark::Retro, vel_c);
            }
        }
        HudMode::Warp => {
            if ship.warp > 0.0 && info.warp_vel.length() > min_v {
                put(info.warp_vel, Mark::Boost, boost_c);
            }
            if let Some(t) = info.to_target {
                put(t, Mark::Target, tgt_c);
                if info.warp_vel.length() > min_v {
                    put(delta_v_to(t, info.warp_vel), Mark::DeltaV, wdv_c);
                }
            }
        }
        HudMode::Docking => {
            if let Some(t) = info.to_target {
                put(t, Mark::Target, tgt_c);
                if info.target_vel.length() > min_v {
                    put(info.target_vel, Mark::Pro, vel_c);
                    put(-info.target_vel, Mark::Retro, vel_c);
                    put(delta_v_to(t, info.target_vel), Mark::DeltaV, tdv_c);
                }
            }
        }
    }
    if flight.hud != HudMode::Off {
        put(info.nose, Mark::Nose, hud_c);
    }
    Ok(())
}

fn fmt_time(s: f64) -> String {
    if !s.is_finite() {
        return "-".to_string();
    }
    let (d, h, m) = ((s / 86400.0).floor(), ((s % 86400.0) / 3600.0).floor(), ((s % 3600.0) / 60.0).floor());
    if d >= 365.25 * 1000.0 {
        format!("{:.3e} y", s / (365.25 * 86400.0))
    } else if d >= 1.0 {
        format!("{d:.0} d {h:02.0}:{m:02.0}")
    } else {
        format!("{h:02.0}:{m:02.0}:{:04.1}", s % 60.0)
    }
}

fn fmt_dist(m: f64) -> String {
    if m.is_finite() { format_distance(m) } else { "-".to_string() }
}

/// The ship panel (`LockShipPanel`) and the flight notices.
pub fn draw_flight_hud(mut contexts: EguiContexts, flight: Res<Flight>, time: Res<Time>, ships: Query<&Ship>, hud: Res<crate::hud::HudState>) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !flight.active || hud.hidden {
        return Ok(());
    }
    let Some(ship) = flight.ship.and_then(|e| ships.get(e).ok()) else { return Ok(()) };
    let d = &ship.design;
    let info = &flight.info;
    let pi = &info.panel;
    let n_chase = d.chase_cameras().len();
    let mode = flight.cam_mode.min(n_chase);
    let cam_name = if flight.remote {
        "remote".to_string()
    } else if mode < n_chase {
        format!("chase {}/{}", mode + 1, n_chase)
    } else {
        "orbit".to_string()
    };
    let text = crate::skin::text(ctx);
    let hud_c = color(&text, "ShipHUDColor", [0.3, 1.0, 0.3, 1.0]);
    let row = |ui: &mut egui::Ui, label: &str, value: String| {
        ui.label(egui::RichText::new(label).color(hud_c.gamma_multiply(0.8)));
        ui.label(egui::RichText::new(value).monospace().color(hud_c));
        ui.end_row();
    };
    let warping = ship.warp > 0.05 || flight.warp_on;
    egui::Area::new(egui::Id::new("flight_hud"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -36.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new().fill(egui::Color32::from_black_alpha(150)).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    let nick = if ship.nickname.is_empty() { String::new() } else { format!("\"{}\"  ", ship.nickname) };
                    ui.label(egui::RichText::new(format!("{nick}{}  ({})   {}", d.name, d.class, flight.hud.label())).strong().size(15.0).color(hud_c));
                });
                ui.horizontal_top(|ui| {
                    egui::Grid::new("ship_panel_a").num_columns(2).spacing([10.0, 1.0]).show(ui, |ui| {
                        row(ui, "Ref body", format!("{} ({})", info.nearest, flight.ref_mode.label()));
                        row(ui, "Altitude", fmt_dist(info.altitude));
                        if pi.valid {
                            row(ui, "Elevation", fmt_dist(pi.elevation));
                            row(ui, "Ship velocity", format_speed(pi.ship_speed));
                            row(ui, "Body velocity", format_speed(pi.body_speed));
                            row(ui, "Rel velocity", format_speed(pi.rel_speed));
                            row(ui, "Surf velocity", format_speed(pi.surf_speed));
                        }
                        if info.aero.active {
                            row(ui, "Air velocity", format_speed(info.aero.air_speed));
                        }
                        if pi.valid {
                            row(ui, "1st cosmic vel", format_speed(pi.circular));
                            row(ui, "2nd cosmic vel", format_speed(pi.escape));
                        }
                    });
                    ui.add_space(18.0);
                    egui::Grid::new("ship_panel_b").num_columns(2).spacing([10.0, 1.0]).show(ui, |ui| {
                        row(ui, "Grav accel", format!("{:.3} m/s²", pi.grav));
                        row(ui, "Aero accel", format!("{:.3} m/s²", info.aero.accel));
                        row(ui, "Thrust", format!("{:.3} m/s²", pi.thrust));
                        row(ui, "Total accel", format!("{:.3} m/s²", pi.total));
                        row(ui, "g Force", format!("{:.3} g", pi.g_force));
                        row(ui, "Tidal force", format!("{:.3e} m/s²", pi.tidal));
                        row(ui, "Angular velocity", format!("{:.3} °/s", pi.ang_vel.to_degrees()));
                        row(ui, "Angular accel", format!("{:.3} °/s²", pi.ang_accel.to_degrees()));
                        row(
                            ui,
                            "Main / Retro / Hover",
                            format!("{:.0}% / {:.0}% / {:.0}%", ship.main * 100.0, ship.retro * 100.0, ship.hover * 100.0),
                        );
                    });
                    ui.add_space(18.0);
                    egui::Grid::new("ship_panel_c").num_columns(2).spacing([10.0, 1.0]).show(ui, |ui| {
                        if warping && d.can_warp() {
                            row(ui, "Warp drive", format!("10^{:.1} (set {:.0}, max {:.0})", ship.warp, flight.warp_cmd, d.warp_boost_log));
                            row(ui, "Boost factor", format!("{:.3e}", pi.boost));
                            row(ui, "Max boost factor", format!("{:.3e}", 10f64.powf(d.warp_boost_log)));
                            row(ui, "Physical velocity", format_speed(info.speed_rel));
                            row(ui, "Effective velocity", format_speed(info.speed_eff));
                            row(ui, "Efficiency", format!("{:.1}%", 100.0 * (pi.boost / pi.boost_cmd.max(1.0)).min(1.0)));
                            if let Some(t) = pi.arrival {
                                row(ui, "Arrival after", fmt_time(t));
                            }
                        } else if let Some(o) = pi.orbit {
                            row(ui, "Orbital parameters", (if pi.stable { "Stable orbit" } else { "Unstable orbit" }).to_string());
                            row(ui, "Periapsis", fmt_dist(o.periapsis - pi.planet_radius));
                            row(ui, "Apoapsis", fmt_dist(o.apoapsis - pi.planet_radius));
                            row(ui, "Eccentricity", format!("{:.4}", o.eccentricity));
                            row(ui, "Inclination", format!("{:.2}°", o.inclination.to_degrees()));
                            row(ui, "Period", fmt_time(o.period));
                            row(ui, "Planet radius", fmt_dist(pi.planet_radius));
                            row(ui, "Atmo height", pi.atmo_height.map_or("-".to_string(), fmt_dist));
                        }
                        row(ui, "Velocity", format_speed(if ship.warp > 0.05 { info.speed_eff } else { info.speed_rel }));
                        if !info.target.is_empty() {
                            row(ui, "Target", info.target.clone());
                            row(ui, "Distance to target", fmt_dist(info.target_distance));
                        }
                    });
                });
                let a = &info.aero;
                if a.active {
                    ui.label(
                        egui::RichText::new(format!(
                            "Mach {:.2}  Dynamic pressure {:.2} kPa  Attack angle {:+.1}°  Sideslip angle {:+.1}°  Speed of sound {}  air density {:.4} kg/m³ ({})",
                            a.mach,
                            a.q * 1e-3,
                            a.alpha.to_degrees(),
                            a.beta.to_degrees(),
                            format_speed(a.sound_speed),
                            a.density,
                            d.aero.name()
                        ))
                        .monospace(),
                    );
                }
                let mut programs = Vec::new();
                if let Some(ap) = flight.autopilot {
                    programs.push(format!("Autopilot: {} - {}", ap.program.label(), ap.stage.label()));
                }
                if flight.attitude != Attitude::Free {
                    programs.push(format!("Orientation: {}", flight.attitude.label()));
                }
                if flight.kill_rotation {
                    programs.push("Kill rotation".to_string());
                }
                if flight.sync_velocity {
                    programs.push("Sync velocity".to_string());
                }
                if let Some(h) = flight.hold_altitude {
                    programs.push(format!("Hold altitude {}", format_distance(h)));
                }
                if !flight.assist {
                    programs.push("Flight assist off".to_string());
                }
                ui.vertical_centered(|ui| {
                    if !programs.is_empty() {
                        ui.label(egui::RichText::new(programs.join("   ")).color(egui::Color32::from_rgb(140, 220, 255)).monospace());
                    }
                    if info.notice_until > time.elapsed_secs_f64() && !info.notice.is_empty() {
                        ui.label(egui::RichText::new(&info.notice).color(egui::Color32::from_rgb(255, 200, 80)).strong());
                    }
                    ui.weak(format!(
                        "=/- main  Shift+=/- retro  Ctrl+=/- hover  Z stop  W/S/A/D/R/F thrusters  keypad/Q/E turn  Alt+=/- warp  Alt+Z stop warp  V camera [{cam_name}]  Ctrl+1-5 HUD  Esc leave"
                    ));
                    ui.weak("5-0 End holds  Num5 kill rot.  U assist  H hold alt.  Shift+Z sync vel.  T target  P ref body  C rotate  G go  Shift+G fly  Alt+G warp");
                });
            });
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_keys_follow_se_table() {
        assert_eq!(HudMode::from_key(KeyCode::Digit1), Some(HudMode::Off));
        assert_eq!(HudMode::from_key(KeyCode::Digit5), Some(HudMode::Docking));
        assert_eq!(HudMode::from_key(KeyCode::Digit6), None);
        for m in HudMode::ALL {
            let name = m.label().trim_end_matches(" HUD").to_ascii_lowercase();
            assert_eq!(HudMode::parse(&name), Some(m));
        }
    }

    #[test]
    fn circular_orbit_elements() {
        let mu: f64 = 3.986e14;
        let r: f64 = 7.0e6;
        let v = (mu / r).sqrt();
        let o = orbit_from_state(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, 0.0, -v), mu, DVec3::Y).unwrap();
        assert!(o.eccentricity < 1e-9);
        assert!((o.periapsis - r).abs() < 1e-3 && (o.apoapsis - r).abs() < 1e-3);
        assert!((o.period - std::f64::consts::TAU * (r * r * r / mu).sqrt()).abs() < 1e-6);
        // Equatorial (angular momentum along the axis).
        assert!(o.inclination < 1e-9);
        assert!(orbit_stable(&o, 6.371e6, 1.0e5));
        assert!(!orbit_stable(&o, 6.371e6, 7.0e5));
        // At escape speed the path is open.
        let e = orbit_from_state(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, (2.0 * mu / r).sqrt() * 1.01, 0.0), mu, DVec3::Y).unwrap();
        assert!(e.eccentricity > 1.0 && e.apoapsis.is_infinite() && !orbit_stable(&e, 1.0, 0.0));
        assert!((e.inclination - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn delta_v_turns_motion_to_target() {
        let v = DVec3::new(3.0, 4.0, 0.0);
        let dv = delta_v_to(DVec3::X * 10.0, v);
        assert!(((v + dv) - DVec3::X * 5.0).length() < 1e-12);
    }
}
