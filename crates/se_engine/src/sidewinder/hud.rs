//! The seat's HUD (step 4.1's fallback: a flat overlay; Elite draws its HUD as holograms in the cockpit, which step 4.1's
//! first part replaces this with). Laid out from the user's own Elite screenshot at 3440 x 1440 (2026-11-14, the exit
//! picture): every place below is a share of the screen's height from its middle, read off that picture (E: the look
//! recreated, the numbers the simulation's). Colours sampled from it: the HUD's orange (213, 110, 0) and the shields' cyan
//! (0, 200, 195).
//!
//! What it shows: the radar's ring under the view, heat on its upper left, the speed on its upper right with the throttle
//! gauge's arc (the fill the speed, the marker the throttle, its blue band the blue zone), the ship's hologram ring with the
//! shields' and hull's percentages, the three pip bars (the pips as notches, the capacitor as the fill), fuel and the three
//! indicators (mass locked, landing gear, cargo scoop), and the keys at the bottom.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::mods::ModControl;

use super::pilot::Pilot;
use super::seat::Seat;

const ORANGE: egui::Color32 = egui::Color32::from_rgb(230, 118, 0);
const DIM: egui::Color32 = egui::Color32::from_rgb(110, 55, 0);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0, 200, 195);
const BLUE_ZONE: egui::Color32 = egui::Color32::from_rgb(40, 140, 220);

/// A point `x`, `y` screen heights from the middle (y down).
fn at(r: egui::Rect, x: f32, y: f32) -> egui::Pos2 {
    let h = r.height();
    egui::pos2(r.center().x + x * h, r.center().y + y * h)
}

pub fn draw_hud(mut contexts: bevy_egui::EguiContexts, seat: Res<Seat>, pilot: Res<Pilot>, control: Res<ModControl>, panels: Res<super::panels::Panels>) -> Result {
    if !seat.seated || !control.pilot || control.main_menu {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let r = ctx.content_rect();
    let h = r.height();
    // Hyperspace's tunnel: over everything while the destination loads (ours: Elite's tunnel isn't built).
    if let Some(j) = pilot.jump {
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("sidewinder tunnel")));
        p.rect_filled(r, 0.0, egui::Color32::from_rgb(4, 6, 16));
        let c = r.center();
        let most = 0.5 * r.width().hypot(h);
        for i in 0..160u32 {
            let k = (i.wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32;
            let a = k * std::f32::consts::TAU;
            let phase = ((i.wrapping_mul(40_503) >> 4) % 1000) as f32 / 1000.0;
            let f = ((j.seconds as f32 * 0.6 + phase) % 1.0).powi(2);
            let dir = egui::vec2(a.cos(), a.sin());
            let (r0, r1) = (f * most, (f * 1.25 + 0.02) * most);
            let col = egui::Color32::from_rgba_unmultiplied(150, 190, 255, (40.0 + 215.0 * f) as u8);
            p.add(egui::Shape::line_segment([c + dir * r0, c + dir * r1], egui::Stroke::new((0.001 + 0.004 * f) * h, col)));
        }
        p.circle_filled(c, 0.04 * h, egui::Color32::from_rgba_unmultiplied(200, 220, 255, 90));
        p.text(egui::pos2(c.x, r.bottom() - 0.12 * h), egui::Align2::CENTER_CENTER, "HYPERSPACE", egui::FontId::monospace(0.026 * h), ORANGE);
        return Ok(());
    }
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("sidewinder hud")));
    let font = |s: f32| egui::FontId::monospace(s * h);
    let stroke = |c: egui::Color32| egui::Stroke::new((0.0018 * h).max(1.0), c);

    // The radar's ring (an ellipse under the view; the sensors switched off take it away).
    let radar = at(r, 0.0, 0.354);
    let rings: &[(f32, f32)] = if pilot.modules[super::panels::M_SENSORS] { &[(1.0, 0.2014), (0.66, 0.13), (0.33, 0.065)] } else { &[] };
    for &(k, a) in rings {
        let pts: Vec<egui::Pos2> = (0..=64).map(|i| {
            let t = i as f32 / 64.0 * std::f32::consts::TAU;
            egui::pos2(radar.x + a * h * t.cos(), radar.y + 0.083 * h * k * t.sin())
        }).collect();
        p.add(egui::Shape::line(pts, stroke(if k == 1.0 { ORANGE } else { DIM })));
    }

    // Heat, upper left of the radar.
    let heat = pilot.heat_percent();
    p.text(at(r, -0.125, 0.292), egui::Align2::CENTER_CENTER, format!("{:03.0}%", heat), font(0.022), if heat >= 100.0 { egui::Color32::RED } else { ORANGE });

    // The speed, upper right, and the throttle gauge's arc to its right: the fill the speed (of the full block's top), the
    // marker the throttle, the blue band the blue zone (the throttle shares where the turn curve is at its best: the
    // block's curve peak `p`).
    let speed = pilot.motion.vel.length();
    let shown = match pilot.cruise {
        Some(c) if c.speed >= 0.1 * elite_sim::supercruise::C => format!("{:.2} c", c.speed / elite_sim::supercruise::C),
        Some(c) if c.speed >= 1.0e6 => format!("{:.1} Mm/s", c.speed / 1.0e6),
        Some(c) => format!("{:.1} km/s", c.speed / 1000.0),
        None => format!("{:.0}", speed),
    };
    p.text(at(r, 0.174, 0.308), egui::Align2::CENTER_CENTER, shown, font(0.024), ORANGE);
    // The drive's charge, as a bar over the radar.
    if let Some(share) = pilot.drive.charge_share() {
        let (a, b) = (at(r, -0.12, 0.25), at(r, 0.12, 0.25));
        p.add(egui::Shape::line_segment([a, b], egui::Stroke::new(0.005 * h, DIM)));
        p.add(egui::Shape::line_segment([a, a + (b - a) * share as f32], egui::Stroke::new(0.005 * h, ORANGE)));
        p.text(at(r, 0.0, 0.258), egui::Align2::CENTER_TOP, "FRAME SHIFT DRIVE CHARGING", font(0.012), ORANGE);
    }
    // A complete charge held by its condition (A50's codes at the charge's end).
    if let Some((why, _)) = pilot.drive.held() {
        p.text(at(r, 0.0, 0.29), egui::Align2::CENTER_CENTER, if why == elite_sim::drive::Refusal::Align { "ALIGN WITH THE DESTINATION" } else { "POINT AWAY FROM THE BODY" }, font(0.013), ORANGE);
    }
    // The jump's countdown (A50's Departing: 5 s) in the middle of the view.
    if let Some(left) = pilot.drive.countdown() {
        p.text(at(r, 0.0, -0.1), egui::Align2::CENTER_CENTER, format!("{}", left.ceil().max(1.0) as u32), font(0.06), ORANGE);
    }
    // The target and the destination, under the radar.
    if pilot.target.is_some() {
        p.text(at(r, 0.0, 0.46), egui::Align2::CENTER_CENTER, format!("TARGET  {}", pilot.target_note), font(0.013), ORANGE);
    }
    if pilot.nav_body.is_some() {
        p.text(at(r, 0.0, 0.478), egui::Align2::CENTER_CENTER, format!("DESTINATION  {}", pilot.nav_note), font(0.013), ORANGE);
    }
    // The planet HUD (step 4.3, ours in its look: Elite's is a hologram): the altimeter and vertical speed to the right, the
    // heading over the view, the pitch and roll beside it; landed, the way out.
    if let Some(ph) = pilot.planet {
        let alt = if ph.alt_m.abs() >= 10_000.0 { format!("{:.1} KM", ph.alt_m / 1000.0) } else { format!("{:.0} M", ph.alt_m) };
        p.text(at(r, 0.30, 0.02), egui::Align2::LEFT_CENTER, format!("ALT {alt}"), font(0.016), ORANGE);
        p.text(at(r, 0.30, 0.045), egui::Align2::LEFT_CENTER, format!("V {:+.0} M/S", ph.vspeed), font(0.013), if ph.vspeed < -5.0 && ph.alt_m < 200.0 { egui::Color32::RED } else { ORANGE });
        p.text(at(r, 0.0, -0.40), egui::Align2::CENTER_CENTER, format!("{:03.0}", ph.heading_deg), font(0.016), ORANGE);
        p.text(at(r, -0.30, 0.02), egui::Align2::RIGHT_CENTER, format!("PITCH {:+.0}", ph.pitch_deg), font(0.013), ORANGE);
        p.text(at(r, -0.30, 0.045), egui::Align2::RIGHT_CENTER, format!("ROLL {:+.0}", ph.roll_deg), font(0.013), ORANGE);
    }
    if pilot.landed {
        p.text(at(r, 0.0, 0.40), egui::Align2::CENTER_CENTER, "DISEMBARK  (B)", font(0.014), ORANGE);
    }
    // The side panels (step 4.2).
    if panels.open.is_some() {
        let mass = pilot.mass_t();
        let fsd = &elite_sim::data::FSD_2E;
        let lines = super::panels::StatusLines {
            home: vec![
                "FAULCON DELACY SIDEWINDER MK I".to_string(),
                format!("Hull {:.0} %   Shields {:.0} %", pilot.hull.percent(), pilot.shields.percent()),
                format!("Fuel {:.2} t   Heat {:.0} %", pilot.fuel.total(), pilot.heat_percent()),
                format!("Target: {}", if pilot.target.is_some() { pilot.target_note.as_str() } else { "none" }),
                format!("Destination: {}", if pilot.nav_body.is_some() { pilot.nav_note.as_str() } else { "none" }),
            ],
            fire: vec![
                "FIRE GROUP A".to_string(),
                "  Primary: Pulse laser 1F x2".to_string(),
                "  Secondary: none".to_string(),
            ],
            status: vec![
                format!("Fuel  main {:.2} t, reservoir {:.2} t", pilot.fuel.main, pilot.fuel.reservoir),
                format!("Mass  {:.1} t", mass),
                format!("Jump range  {:.2} LY now, {:.2} LY at most", elite_sim::fsd::jump_range(fsd, mass, pilot.fuel.main), elite_sim::fsd::max_jump_range(fsd, elite_sim::data::JOURNAL_UNLADEN_MASS)),
                format!("Heat  {:.0} %", pilot.heat_percent()),
                format!("Hull  {:.0} %   Shields {:.0} %", pilot.hull.percent(), pilot.shields.percent()),
                format!("Pips  SYS {} ENG {} WEP {}", pilot.dist.half[elite_sim::power::SYS] as f32 / 2.0, pilot.dist.half[elite_sim::power::ENG] as f32 / 2.0, pilot.dist.half[elite_sim::power::WEP] as f32 / 2.0),
                format!("Drive  {:?}", pilot.drive),
            ],
        };
        let ship = [pilot.flight_assist, pilot.gear_down, pilot.weapons.deployed];
        super::panels::draw(&p, r, &panels, &pilot.modules, ship, &lines, ORANGE, DIM);
    }
    let (x0, y0, x1, y1) = (0.194f32, 0.458f32, 0.236f32, 0.333f32);
    let along = |f: f32| at(r, x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
    p.add(egui::Shape::line_segment([along(0.0), along(1.0)], stroke(DIM)));
    let full_top = pilot.full_top.max(1.0);
    // (In supercruise the gauge's fill is the speed against the wells' cap.)
    let fill = match pilot.cruise {
        Some(c) => (c.speed / c.cap.max(1.0)).clamp(0.0, 1.0) as f32,
        None => (speed / full_top).clamp(0.0, 1.0) as f32,
    };
    p.add(egui::Shape::line_segment([along(0.0), along(fill)], egui::Stroke::new(0.006 * h, ORANGE)));
    let (bz0, bz1) = pilot.blue_zone;
    p.add(egui::Shape::line_segment([along(bz0), along(bz1)], egui::Stroke::new(0.003 * h, BLUE_ZONE)));
    let th = pilot.throttle as f32;
    let mark = along((th.abs() * pilot.top_share as f32).clamp(0.0, 1.0));
    p.add(egui::Shape::line_segment([mark - egui::vec2(0.012 * h, 0.0), mark + egui::vec2(0.012 * h, 0.0)], stroke(if th < 0.0 { egui::Color32::RED } else { egui::Color32::WHITE })));

    // The ship's hologram ring: shields (cyan) and hull.
    let ring = at(r, 0.385, 0.303);
    let pts: Vec<egui::Pos2> = (0..=64).map(|i| {
        let t = i as f32 / 64.0 * std::f32::consts::TAU;
        egui::pos2(ring.x + 0.108 * h * t.cos(), ring.y + 0.055 * h * t.sin())
    }).collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(0.004 * h, if pilot.shields.up { CYAN } else { DIM })));
    p.text(at(r, 0.401, 0.367), egui::Align2::CENTER_CENTER, format!("{:.0}%", pilot.shields.percent()), font(0.016), CYAN);
    p.text(at(r, 0.317, 0.424), egui::Align2::CENTER_CENTER, format!("{:.0}%", pilot.hull.percent()), font(0.016), ORANGE);

    // The pips: three bars (SYS, ENG, WEP), the capacitor's charge as the fill and the pips as notches beside.
    let d = &pilot.dist;
    for (i, &(c, label, lx)) in [(elite_sim::power::SYS, "SYS", 0.553f32), (elite_sim::power::ENG, "ENG", 0.597), (elite_sim::power::WEP, "WEP", 0.642)].iter().enumerate() {
        let (top, bottom) = (0.3125f32, 0.382f32);
        let half_w = 0.0083f32;
        let charge = (d.charge[c] / d.caps[c].capacity.max(1e-9)).clamp(0.0, 1.0) as f32;
        let frame = egui::Rect::from_min_max(at(r, lx - half_w, top), at(r, lx + half_w, bottom));
        p.rect_stroke(frame, 0.0, stroke(DIM), egui::StrokeKind::Inside);
        let fill = egui::Rect::from_min_max(at(r, lx - half_w, bottom - (bottom - top) * charge), at(r, lx + half_w, bottom));
        p.rect_filled(fill, 0.0, ORANGE);
        for k in 0..d.half[c] {
            let y = bottom - (bottom - top) * (k as f32 + 0.5) / elite_sim::power::MOST_HALF as f32;
            p.circle_filled(at(r, lx + half_w + 0.006, y), 0.0025 * h, ORANGE);
        }
        p.text(at(r, lx, 0.398 + 0.004 * i as f32), egui::Align2::CENTER_TOP, label, font(0.011), ORANGE);
    }

    // Fuel and the indicators.
    p.text(at(r, 0.708, 0.343), egui::Align2::CENTER_CENTER, "FUEL", font(0.012), ORANGE);
    p.text(at(r, 0.83, 0.343), egui::Align2::CENTER_CENTER, format!("{:.2} t", pilot.fuel.main), font(0.012), ORANGE);
    let fuel_share = (pilot.fuel.main / pilot.fuel.main_cap.max(1e-9)).clamp(0.0, 1.0) as f32;
    p.add(egui::Shape::line_segment([at(r, 0.683, 0.364), at(r, 0.875, 0.364)], egui::Stroke::new(0.004 * h, DIM)));
    p.add(egui::Shape::line_segment([at(r, 0.683, 0.364), at(r, 0.683 + 0.192 * fuel_share, 0.364)], egui::Stroke::new(0.004 * h, ORANGE)));
    for (k, (label, on)) in [("MASS LOCKED", false), ("LANDING GEAR", pilot.gear_down), ("CARGO SCOOP", false)].iter().enumerate() {
        let y = 0.392 + 0.02 * k as f32;
        let b = egui::Rect::from_center_size(at(r, 0.694, y), egui::vec2(0.009 * h, 0.009 * h));
        if *on {
            p.rect_filled(b, 0.0, ORANGE);
        } else {
            p.rect_stroke(b, 0.0, stroke(DIM), egui::StrokeKind::Inside);
        }
        p.text(at(r, 0.705, y), egui::Align2::LEFT_CENTER, *label, font(0.012), if *on { ORANGE } else { DIM });
    }

    // Messages of the moment: flight assist off, boosting, landed.
    let mut notes = Vec::new();
    if !pilot.flight_assist {
        notes.push("FLIGHT ASSIST OFF");
    }
    if pilot.boost.firing() {
        notes.push("BOOST");
    }
    if pilot.landed {
        notes.push("LANDED");
    }
    if pilot.cruise.is_some() {
        notes.push("SUPERCRUISE");
    }
    if pilot.glide.is_some() {
        notes.push("GLIDE");
    }
    if pilot.star_heat > 0.0 {
        notes.push("STAR PROXIMITY");
    }
    for (k, n) in notes.iter().enumerate() {
        p.text(at(r, 0.0, 0.22 + 0.025 * k as f32), egui::Align2::CENTER_CENTER, *n, font(0.016), ORANGE);
    }
    let keys = "Mouse pitch/roll  A D yaw  W S X throttle  Q E R F thrust  Tab boost  Arrows pips  J supercruise/jump  T target ahead  1 4 panels  U hardpoints  Z assist  L gear  Middle button head look  B out (landed)";
    p.text(egui::pos2(r.center().x, r.bottom() - 0.01 * h), egui::Align2::CENTER_BOTTOM, keys, egui::FontId::proportional(0.0095 * h), ORANGE.gamma_multiply(0.7));
    Ok(())
}
