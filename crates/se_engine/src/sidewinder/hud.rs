//! The seat's HUD (step 4.1's fallback in its look: flat shapes and text; Elite draws its HUD as holograms). Since T4e-4 it
//! hangs in the cockpit: each group below is drawn in the old flat layout's coordinates (shares of the screen's height from
//! its middle, read off the user's own Elite screenshot at 3440 x 1440, 2026-11-14, the exit picture: E) into a `Sketch`,
//! which `hud_anchor` puts on the group's anchor, a rectangle in the ship's space at Elite's place, through the seat's
//! camera this frame: it moves with the cockpit when the head turns. Colours sampled from that picture: the HUD's orange
//! (213, 110, 0) and the shields' cyan (0, 200, 195).
//!
//! What it shows: the radar's ring under the view, heat on its upper left, the speed on its upper right with the throttle
//! gauge's arc (the fill the speed, the marker the throttle, its blue band the blue zone), the ship's hologram ring with the
//! shields' and hull's percentages, the three pip bars (the pips as notches, the capacitor as the fill), fuel and the four
//! indicators (mass locked, landing gear, cargo scoop, lights), the notes (silent running, the safe disengage), the emergency
//! oxygen's clock, the locked destination (U1: a marker in the view at its direction, through the seat's camera, and the
//! compass on its decoded place left of the radar), Elite's mouse widget (U2, `mouse_widget.rs`), and the keys at the bottom (the mod's help, not Elite's HUD: still flat on the screen, as the hyperspace
//! tunnel's name is; the tunnel itself is in the scene, `tunnel.rs`). The fade from black after boarding is
//! `seat::draw_prompt`'s.

use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::*;
use bevy_egui::egui;
use elite_sim::life::Alarm;

use crate::mods::ModControl;

use super::hud_anchor::{self as anchor, SeatView, Sketch};
use super::pilot::{Dest, DestMark, Pilot};
use super::seat::Seat;
use super::SidewinderAssets;

const ORANGE: egui::Color32 = egui::Color32::from_rgb(230, 118, 0);
const DIM: egui::Color32 = egui::Color32::from_rgb(110, 55, 0);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0, 200, 195);
const BLUE_ZONE: egui::Color32 = egui::Color32::from_rgb(40, 140, 220);

/// The speed's text: in supercruise (`cruise`, its speed) in c from a tenth of light's, Mm/s from 1,000 km/s, else km/s;
/// in normal flight the flight's `speed` in m/s.
fn speed_text(cruise: Option<f64>, speed: f64) -> String {
    match cruise {
        Some(v) if v >= 0.1 * elite_sim::supercruise::C => format!("{:.2} c", v / elite_sim::supercruise::C),
        Some(v) if v >= 1.0e6 => format!("{:.1} Mm/s", v / 1.0e6),
        Some(v) => format!("{:.1} km/s", v / 1000.0),
        None => format!("{:.0}", speed),
    }
}

/// Where the speed's text stands on the throttle's anchor, in the flat layout, its alignment there and its size. The place
/// is ours (E): its left end above the radar's ring on the right, so that the longest text (supercruise's "1000.0 km/s")
/// clears the ring, the hologram's ring, the gauge, the drive's charge bar and its label and the first two notes; at the
/// anchor's measured figure (0.174, 0.308, centred: Elite's figure is short) that text ran into the ring.
const SPEED_AT: (f32, f32) = (0.11, 0.258);
const SPEED_ALIGN: egui::Align2 = egui::Align2::LEFT_CENTER;
const SPEED_FONT: f32 = 0.024;

/// The throttle gauge's foot and top, in the flat layout on the throttle's anchor.
const GAUGE: [(f32, f32); 2] = [(0.194, 0.458), (0.236, 0.333)];

// The in-view marker of the destination (U1). Its look is ours (F: Elite's own reticle art is not decoded): four arcs of a
// ring round the destination's middle, gaps at the diagonals, the name above right and the distance under it. Sizes are
// shares of the screen's height.

/// How far along the destination's direction the marker's point is put before projecting (m; any distance past the
/// cockpit gives the same place: only the direction counts).
const MARK_FAR_M: f32 = 1000.0;
/// The ring's least radius (ours, F).
const MARK_MIN_RADIUS: f32 = 0.016;
/// The ring's margin outside the body's disc (ours, F).
const MARK_MARGIN: f32 = 0.006;
/// The ring's greatest radius (ours, F): a ring round the view's middle then stays clear of the HUD's lower parts (the
/// messages start 0.22 under the middle). A bigger disc gets the least ring on its middle instead (`mark_radius`).
const MARK_MAX_RADIUS: f32 = 0.2;
/// Each arc's length (radians; ours, F): four of them, centred right, down, left and up.
const MARK_ARC: f32 = 0.9;
/// The name's and the distance's sizes, and their gap from the ring's right edge (ours, F).
const MARK_FONT: f32 = 0.013;
const MARK_DIST_FONT: f32 = 0.011;
const MARK_TEXT_GAP: f32 = 0.006;

/// The compass's dot's radius, a share of the screen's height in the compass's layout (ours, F: big enough that the hollow
/// dot reads as hollow inside the HUD's stroke).
const COMPASS_DOT: f32 = 0.0075;

/// The marker's ring radius (px) for a disc of `disc` px on a screen `h` px high (ours, F): the disc plus the margin, at least
/// the least radius; when that is over the greatest (a body filling the view), or the disc can't be measured (infinite or
/// NaN), the least radius on the body's middle.
fn mark_radius(disc: f32, h: f32) -> f32 {
    let r = (disc + MARK_MARGIN * h).max(MARK_MIN_RADIUS * h);
    if r <= MARK_MAX_RADIUS * h { r } else { MARK_MIN_RADIUS * h }
}

/// Where the in-view marker goes on the `screen`: the destination's middle and its disc's apparent radius there (px), the
/// direction from the eye (`eye`, ship space) projected through the seat's camera as the cockpit is drawn; None when it is
/// behind the eye or off the screen. (A disc too big to project, its edge behind the eye, has an infinite radius.)
fn mark_place(view: &SeatView, eye: Vec3, screen: egui::Rect, m: &DestMark) -> Option<(egui::Pos2, f32)> {
    let far = |d: Vec3| view.project(eye + d * MARK_FAR_M);
    let at = far(m.dir).filter(|p| screen.contains(*p))?;
    // The disc's edge: the direction turned by the angular radius about an axis square to it.
    let edge = Quat::from_axis_angle(m.dir.any_orthonormal_vector(), m.ang_radius) * m.dir;
    let radius = far(edge).map_or(f32::INFINITY, |e| e.distance(at));
    Some((at, radius))
}

/// The compass's dot (grade E: no recording shows Elite's compass or its in-view marker, since nothing was locked in any of
/// them, `re/elite/notes/sidewinder_hud_marks.md` C and R; with nothing locked the recordings show no compass, which is why it
/// is drawn only while something is locked): the destination's direction `dir` (ship space) seen down the nose, orthographic: its x and y as shares of the
/// ring's radius (x right, y up), so dead ahead and dead astern are both the middle and a right angle off the nose is on the
/// ring; `true` with the destination ahead (drawn filled), `false` behind (hollow).
pub fn compass_dot(dir: Vec3) -> (Vec2, bool) {
    let d = dir.normalize_or(Vec3::NEG_Z);
    (Vec2::new(d.x, d.y), d.z <= 0.0)
}

/// A point `x`, `y` screen heights from the middle (y down).
fn at(r: egui::Rect, x: f32, y: f32) -> egui::Pos2 {
    let h = r.height();
    egui::pos2(r.center().x + x * h, r.center().y + y * h)
}

pub fn draw_hud(mut contexts: bevy_egui::EguiContexts, seat: Res<Seat>, pilot: Res<Pilot>, control: Res<ModControl>, panels: Res<super::panels::Panels>, assets: Res<SidewinderAssets>, screens: Option<Res<super::ui::screens::UiScreens>>, live: Res<super::ui::UiLive>) -> Result {
    if !seat.seated || !control.pilot || control.main_menu || screens.is_some_and(|s| s.covers()) {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let r = ctx.content_rect();
    let h = r.height();
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("sidewinder hud")));
    // Hyperspace's tunnel is drawn in the scene behind the cockpit (`tunnel.rs`); its name stays flat on the screen, as the keys.
    if pilot.jump.is_some() {
        p.text(egui::pos2(r.center().x, r.bottom() - 0.12 * h), egui::Align2::CENTER_CENTER, "HYPERSPACE", egui::FontId::monospace(0.026 * h), ORANGE);
    }
    // The seat's camera as the cockpit is drawn with it this frame: the eye, the head's turn `seat_camera` wrote the camera
    // with (this pass runs after it: the mod's `plugin` orders them), Elite's field of view for the screen's aspect.
    let eye = Vec3::from_array(assets.eye);
    let view = SeatView::new(eye, seat.head, r);
    let font = |s: f32| egui::FontId::monospace(s * h);
    let stroke = |c: egui::Color32| egui::Stroke::new((0.0018 * h).max(1.0), c);
    // While Elite's UI is made and running (`UiLive`) the gauges its movies show are theirs (`ui`): the heat, the speed and
    // throttle, the target and destination lines, the compass, the shields and hull, the pips, the fuel and the indicators
    // are not drawn here. Otherwise (`SE_SW_UI=0`, or the UI could not be made or turned itself off) these stand-ins are.
    let movies = live.0;

    // The radar's ring (an ellipse under the view; the sensors switched off take it away).
    let mut g = Sketch::new(&p, r);
    let radar = at(r, 0.0, 0.354);
    let rings: &[(f32, f32)] = if pilot.modules[super::panels::M_SENSORS] && !movies { &[(1.0, 0.2014), (0.66, 0.13), (0.33, 0.065)] } else { &[] };
    for &(k, a) in rings {
        let pts: Vec<egui::Pos2> = (0..=64).map(|i| {
            let t = i as f32 / 64.0 * std::f32::consts::TAU;
            egui::pos2(radar.x + a * h * t.cos(), radar.y + 0.083 * h * k * t.sin())
        }).collect();
        g.add(egui::Shape::line(pts, stroke(if k == 1.0 { ORANGE } else { DIM })));
    }
    g.finish(&view, &anchor::RADAR, anchor::RADAR_BOX);

    // Heat, upper left of the radar.
    if !movies {
        let mut g = Sketch::new(&p, r);
        let heat = pilot.heat_percent();
        g.text(at(r, -0.125, 0.292), egui::Align2::CENTER_CENTER, format!("{:03.0}%", heat), font(0.022), if heat >= 100.0 { egui::Color32::RED } else { ORANGE });
        g.finish(&view, &anchor::HEAT, anchor::HEAT_BOX);

        // The speed, upper right, and the throttle gauge's arc to its right: the fill the speed (of the full block's top), the
        // marker the throttle, the blue band the blue zone (the throttle shares where the turn curve is at its best: the
        // block's curve peak `p`).
        let mut g = Sketch::new(&p, r);
        let speed = pilot.motion.vel.length();
        g.text(at(r, SPEED_AT.0, SPEED_AT.1), SPEED_ALIGN, speed_text(pilot.cruise.map(|c| c.speed), speed), font(SPEED_FONT), ORANGE);
        let [(x0, y0), (x1, y1)] = GAUGE;
        let along = |f: f32| at(r, x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
        g.line_segment([along(0.0), along(1.0)], stroke(DIM));
        let full_top = pilot.full_top.max(1.0);
        // (In supercruise the gauge's fill is the speed against the wells' cap.)
        let fill = match pilot.cruise {
            Some(c) => (c.speed / c.cap.max(1.0)).clamp(0.0, 1.0) as f32,
            None => (speed / full_top).clamp(0.0, 1.0) as f32,
        };
        g.line_segment([along(0.0), along(fill)], egui::Stroke::new(0.006 * h, ORANGE));
        let (bz0, bz1) = pilot.blue_zone;
        g.line_segment([along(bz0), along(bz1)], egui::Stroke::new(0.003 * h, BLUE_ZONE));
        let th = pilot.throttle as f32;
        let mark = along((th.abs() * pilot.top_share as f32).clamp(0.0, 1.0));
        g.line_segment([mark - egui::vec2(0.012 * h, 0.0), mark + egui::vec2(0.012 * h, 0.0)], stroke(if th < 0.0 { egui::Color32::RED } else { egui::Color32::WHITE }));
        g.finish(&view, &anchor::THROTTLE, anchor::THROTTLE_BOX);
    }

    // The messages under the view: the drive's charge (a bar over the radar), a complete charge held by its condition (A50's
    // codes at the charge's end), the way out when landed, and the notes of the moment.
    let mut g = Sketch::new(&p, r);
    if let Some(share) = pilot.drive.charge_share() {
        let (a, b) = (at(r, -0.12, 0.25), at(r, 0.12, 0.25));
        g.line_segment([a, b], egui::Stroke::new(0.005 * h, DIM));
        g.line_segment([a, a + (b - a) * share as f32], egui::Stroke::new(0.005 * h, ORANGE));
        g.text(at(r, 0.0, 0.258), egui::Align2::CENTER_TOP, "FRAME SHIFT DRIVE CHARGING", font(0.012), ORANGE);
    }
    if let Some((why, _)) = pilot.drive.held() {
        g.text(at(r, 0.0, 0.29), egui::Align2::CENTER_CENTER, if why == elite_sim::drive::Refusal::Align { "ALIGN WITH THE DESTINATION" } else { "POINT AWAY FROM THE BODY" }, font(0.013), ORANGE);
    }
    // (The mod's own hints, not Elite's: with Elite's UI the role panel is the way out; B still works.)
    if pilot.landed && !movies {
        g.text(at(r, 0.0, 0.40), egui::Align2::CENTER_CENTER, "DISEMBARK  (B)", font(0.014), ORANGE);
    }
    let mut notes = Vec::new();
    if !pilot.flight_assist {
        notes.push("FLIGHT ASSIST OFF");
    }
    if pilot.boost.firing() {
        notes.push("BOOST");
    }
    if pilot.landed && !movies {
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
    if pilot.silent {
        notes.push("SILENT RUNNING");
    }
    match pilot.safe_drop {
        Some(true) => notes.push("SAFE DISENGAGE READY"),
        Some(false) if pilot.cruise.is_some() => notes.push("DESTINATION: TOO FAST OR TOO FAR TO DISENGAGE"),
        _ => {}
    }
    for (k, n) in notes.iter().enumerate() {
        g.text(at(r, 0.0, 0.22 + 0.025 * k as f32), egui::Align2::CENTER_CENTER, *n, font(0.016), ORANGE);
    }
    g.finish(&view, &anchor::MESSAGES, anchor::MESSAGES_BOX);

    // The top of the view: the jump's countdown (A50's Departing: 5 s; its number the voice's, `drive::countdown_number`),
    // the canopy and the emergency oxygen (F6: the clock while it runs, red from 30 s left, the exe's critical alarm), and
    // the planet HUD's heading.
    let mut g = Sketch::new(&p, r);
    if let Some(n) = pilot.drive.countdown_number() {
        g.text(at(r, 0.0, -0.1), egui::Align2::CENTER_CENTER, format!("{n}"), font(0.06), ORANGE);
    }
    let alarm = pilot.life.alarm();
    if alarm != Alarm::Off {
        let s = pilot.life.oxygen.ceil() as u32;
        let why = if pilot.life.canopy { "LIFE SUPPORT OFFLINE" } else { "CANOPY BREACHED" };
        g.text(at(r, 0.0, -0.33), egui::Align2::CENTER_CENTER, format!("{why}  OXYGEN {}:{:02}", s / 60, s % 60), font(0.02), if alarm == Alarm::Critical { egui::Color32::RED } else { ORANGE });
    }
    if let Some(ph) = pilot.planet {
        g.text(at(r, 0.0, -0.40), egui::Align2::CENTER_CENTER, format!("{:03.0}", ph.heading_deg), font(0.016), ORANGE);
    }
    g.finish(&view, &anchor::TOP, anchor::TOP_BOX);

    // The target and the destination, on the target panel.
    if !movies {
        let mut g = Sketch::new(&p, r);
        if pilot.target.is_some() {
            g.text(at(r, 0.0, 0.46), egui::Align2::CENTER_CENTER, format!("TARGET  {}", pilot.target_note), font(0.013), ORANGE);
        }
        if pilot.nav_body.is_some() {
            g.text(at(r, 0.0, 0.478), egui::Align2::CENTER_CENTER, format!("DESTINATION  {}", pilot.nav_note), font(0.013), ORANGE);
        }
        g.finish(&view, &anchor::TARGET, anchor::TARGET_BOX);
    }

    // The locked destination (U1): the marker in the view at its real direction, and the compass left of the radar (shown
    // only while a destination is locked: E, as `compass_dot`).
    if let Some(m) = &pilot.dest {
        if let Some((mid, disc)) = mark_place(&view, eye, r, m) {
            let radius = mark_radius(disc, h);
            for k in 0..4 {
                let middle = k as f32 * std::f32::consts::FRAC_PI_2;
                let pts: Vec<egui::Pos2> = (0..=12)
                    .map(|i| {
                        let t = middle + MARK_ARC * (i as f32 / 12.0 - 0.5);
                        mid + radius * egui::vec2(t.cos(), t.sin())
                    })
                    .collect();
                p.add(egui::Shape::line(pts, stroke(ORANGE)));
            }
            // The name and the distance start past the ring's right edge, the name over the middle's height, the distance under.
            let text_at = mid + egui::vec2(radius + MARK_TEXT_GAP * h, 0.0);
            p.text(text_at, egui::Align2::LEFT_BOTTOM, &m.name, font(MARK_FONT), ORANGE);
            p.text(text_at, egui::Align2::LEFT_TOP, m.distance_text(), font(MARK_DIST_FONT), ORANGE);
        }
        if !movies {
            let mut g = Sketch::new(&p, r);
            let d = anchor::COMPASS_BOX;
            let (centre, ring) = (at(r, d.x, d.y), d.hw * h);
            let pts: Vec<egui::Pos2> = (0..=64)
                .map(|i| {
                    let t = i as f32 / 64.0 * std::f32::consts::TAU;
                    centre + ring * egui::vec2(t.cos(), t.sin())
                })
                .collect();
            g.add(egui::Shape::line(pts, stroke(ORANGE)));
            let (dot, ahead) = compass_dot(m.dir);
            let at_dot = centre + ring * egui::vec2(dot.x, -dot.y);
            if ahead {
                g.circle_filled(at_dot, COMPASS_DOT * h, ORANGE);
            } else {
                g.add(egui::Shape::circle_stroke(at_dot, COMPASS_DOT * h, stroke(ORANGE)));
            }
            g.finish(&view, &anchor::COMPASS, anchor::COMPASS_BOX);
        }
    }

    // Elite's mouse widget (U2): the dot on the nose's axis and the arrow off it along the mouse's virtual stick.
    if super::mouse_widget::shown(assets.keys.value("MouseGUI"), panels.open.is_some()) {
        super::mouse_widget::draw(&p, &view, eye, pilot.stick);
    }

    // The planet HUD (step 4.3, ours in its look: Elite's is a hologram): the altimeter and vertical speed on the right rail,
    // the pitch and roll on the left.
    if let Some(ph) = pilot.planet {
        let mut g = Sketch::new(&p, r);
        g.text(at(r, 0.30, 0.02), egui::Align2::LEFT_CENTER, format!("ALT {}", super::panels::altitude_text(ph.alt_m)), font(0.016), ORANGE);
        g.text(at(r, 0.30, 0.045), egui::Align2::LEFT_CENTER, format!("V {:+.0} M/S", ph.vspeed), font(0.013), if ph.vspeed < -5.0 && ph.alt_m < 200.0 { egui::Color32::RED } else { ORANGE });
        g.finish(&view, &anchor::ALTITUDE, anchor::ALTITUDE_BOX);
        let mut g = Sketch::new(&p, r);
        g.text(at(r, -0.30, 0.02), egui::Align2::RIGHT_CENTER, format!("PITCH {:+.0}", ph.pitch_deg), font(0.013), ORANGE);
        g.text(at(r, -0.30, 0.045), egui::Align2::RIGHT_CENTER, format!("ROLL {:+.0}", ph.roll_deg), font(0.013), ORANGE);
        g.finish(&view, &anchor::ATTITUDE, anchor::ATTITUDE_BOX);
    }

    // The ship's hologram ring: shields (cyan) and hull.
    if !movies {
        let mut g = Sketch::new(&p, r);
        let ring = at(r, 0.385, 0.303);
        let pts: Vec<egui::Pos2> = (0..=64).map(|i| {
            let t = i as f32 / 64.0 * std::f32::consts::TAU;
            egui::pos2(ring.x + 0.108 * h * t.cos(), ring.y + 0.055 * h * t.sin())
        }).collect();
        g.add(egui::Shape::line(pts, egui::Stroke::new(0.004 * h, if pilot.shields.up { CYAN } else { DIM })));
        g.text(at(r, 0.401, 0.367), egui::Align2::CENTER_CENTER, format!("{:.0}%", pilot.shields.percent()), font(0.016), CYAN);
        g.text(at(r, 0.317, 0.424), egui::Align2::CENTER_CENTER, format!("{:.0}%", pilot.hull.percent()), font(0.016), ORANGE);
        g.finish(&view, &anchor::HEALTH, anchor::HEALTH_BOX);

        // The pips: three bars (SYS, ENG, WEP), the capacitor's charge as the fill and the pips as notches beside.
        let mut g = Sketch::new(&p, r);
        let d = &pilot.dist;
        for (i, &(c, label, lx)) in [(elite_sim::power::SYS, "SYS", 0.553f32), (elite_sim::power::ENG, "ENG", 0.597), (elite_sim::power::WEP, "WEP", 0.642)].iter().enumerate() {
            let (top, bottom) = (0.3125f32, 0.382f32);
            let half_w = 0.0083f32;
            let charge = (d.charge[c] / d.caps[c].capacity.max(1e-9)).clamp(0.0, 1.0) as f32;
            let frame = egui::Rect::from_min_max(at(r, lx - half_w, top), at(r, lx + half_w, bottom));
            g.rect_stroke(frame, 0.0, stroke(DIM), egui::StrokeKind::Inside);
            let fill = egui::Rect::from_min_max(at(r, lx - half_w, bottom - (bottom - top) * charge), at(r, lx + half_w, bottom));
            g.rect_filled(fill, 0.0, ORANGE);
            for k in 0..d.half[c] {
                let y = bottom - (bottom - top) * (k as f32 + 0.5) / elite_sim::power::MOST_HALF as f32;
                g.circle_filled(at(r, lx + half_w + 0.006, y), 0.0025 * h, ORANGE);
            }
            g.text(at(r, lx, 0.398 + 0.004 * i as f32), egui::Align2::CENTER_TOP, label, font(0.011), ORANGE);
    }
    g.finish(&view, &anchor::PIPS, anchor::PIPS_BOX);

    // Fuel and the indicators.
    let mut g = Sketch::new(&p, r);
    g.text(at(r, 0.708, 0.343), egui::Align2::CENTER_CENTER, "FUEL", font(0.012), ORANGE);
    g.text(at(r, 0.83, 0.343), egui::Align2::CENTER_CENTER, format!("{:.2} t", pilot.fuel.main), font(0.012), ORANGE);
    let fuel_share = (pilot.fuel.main / pilot.fuel.main_cap.max(1e-9)).clamp(0.0, 1.0) as f32;
    g.line_segment([at(r, 0.683, 0.364), at(r, 0.875, 0.364)], egui::Stroke::new(0.004 * h, DIM));
    g.line_segment([at(r, 0.683, 0.364), at(r, 0.683 + 0.192 * fuel_share, 0.364)], egui::Stroke::new(0.004 * h, ORANGE));
    for (k, (label, on)) in [("MASS LOCKED", false), ("LANDING GEAR", pilot.gear_down), ("CARGO SCOOP", pilot.scoop), ("LIGHTS", pilot.lights)].iter().enumerate() {
        let y = 0.392 + 0.02 * k as f32;
        let b = egui::Rect::from_center_size(at(r, 0.694, y), egui::vec2(0.009 * h, 0.009 * h));
        if *on {
            g.rect_filled(b, 0.0, ORANGE);
        } else {
            g.rect_stroke(b, 0.0, stroke(DIM), egui::StrokeKind::Inside);
        }
        g.text(at(r, 0.705, y), egui::Align2::LEFT_CENTER, *label, font(0.012), if *on { ORANGE } else { DIM });
    }
    g.finish(&view, &anchor::FUEL, anchor::FUEL_BOX);
    }

    // The side panels (step 4.2), each on its own anchor beside the seat (with Elite's UI on, its own panels instead: `ui`).
    if panels.open.is_some() && !movies {
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
        super::panels::draw(&p, r, &view, &panels, &pilot.modules, ship, &lines, pilot.locked().map(Dest::obj), ORANGE, DIM);
    }

    if !movies {
        let keys = "Mouse pitch/roll  A D yaw  W S X throttle  Q E R F thrust  Tab boost  Arrows pips  J supercruise/jump  T target ahead  1 4 panels  U hardpoints  Z assist  L gear  Middle button head look  B out (off the ground the ship goes)";
        p.text(egui::pos2(r.center().x, r.bottom() - 0.01 * h), egui::Align2::CENTER_BOTTOM, keys, egui::FontId::proportional(0.0095 * h), ORANGE.gamma_multiply(0.7));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::seat::Head;
    use super::*;

    /// The eye: the cockpit's `!Camera` bone in the ship's space (`SidewinderAssets::eye` from the shipped files).
    const EYE: Vec3 = Vec3::new(0.0001, 6.7223, 1.3465);

    /// The speed's longest texts: km/s just under 1,000 km/s, Mm/s just under a tenth of light's, c at supercruise's top
    /// speed, and normal flight's m/s.
    fn longest() -> Vec<String> {
        let c = elite_sim::supercruise::C;
        vec![speed_text(Some(999_999.9), 0.0), speed_text(Some(0.1 * c - 1.0), 0.0), speed_text(Some(elite_sim::supercruise::VSC), 0.0), speed_text(None, 9_999.0)]
    }

    /// The text's size at `px` (egui's monospace, as the HUD lays it out).
    fn text_size(text: &str, px: f32) -> egui::Vec2 {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let size = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), egui::FontId::monospace(px), ORANGE).size());
        let _ = ctx.end_pass();
        size
    }

    /// Whether `p` is inside the closed polygon `poly` (crossings of a ray to the right).
    fn inside(p: egui::Pos2, poly: &[egui::Pos2]) -> bool {
        let mut odd = false;
        for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                odd = !odd;
            }
        }
        odd
    }

    /// An ellipse filling a group's design box (the radar's outer ring, the hologram's ring) as drawn on the screen.
    fn ring(view: &SeatView, r: egui::Rect, a: &anchor::Anchor, d: anchor::DesignBox) -> Vec<egui::Pos2> {
        (0..512)
            .map(|i| {
                let t = i as f32 / 512.0 * std::f32::consts::TAU;
                let p = at(r, d.x + d.hw * t.cos(), d.y + d.hh * t.sin());
                anchor::place(view, a, d, r, p).unwrap()
            })
            .collect()
    }

    /// The compass's dot: dead ahead the middle, filled; dead astern the middle, hollow; a right angle to the right on the
    /// ring's right edge, straight up on its top; behind and to the left inside the ring's left half, hollow.
    #[test]
    fn the_compass_dot() {
        assert_eq!(compass_dot(Vec3::NEG_Z), (Vec2::ZERO, true));
        assert_eq!(compass_dot(Vec3::Z * 3.0), (Vec2::ZERO, false));
        assert_eq!(compass_dot(Vec3::X), (Vec2::X, true));
        assert_eq!(compass_dot(Vec3::Y), (Vec2::Y, true));
        let (d, ahead) = compass_dot(Vec3::new(-1.0, 0.0, 1.0));
        assert!(!ahead && (d - Vec2::new(-std::f32::consts::FRAC_1_SQRT_2, 0.0)).length() < 1e-6, "{d}");
    }

    /// The ring's radius: the disc plus the margin, at least the least radius; a disc whose ring would pass the greatest
    /// radius, or one that can't be measured, gets the least ring on its middle.
    #[test]
    fn the_marker_ring_radius() {
        let h = 900.0;
        assert_eq!(mark_radius(1.0, h), MARK_MIN_RADIUS * h);
        assert!((mark_radius(100.0, h) - (100.0 + MARK_MARGIN * h)).abs() < 1e-3);
        assert!((mark_radius(MARK_MAX_RADIUS * h - MARK_MARGIN * h, h) - MARK_MAX_RADIUS * h).abs() < 1e-3);
        for disc in [MARK_MAX_RADIUS * h, 432.0, f32::INFINITY, f32::NAN] {
            assert_eq!(mark_radius(disc, h), MARK_MIN_RADIUS * h, "{disc}");
        }
    }

    fn mark(dir: Vec3, ang_radius: f32) -> DestMark {
        DestMark { obj: se_core::catalog::ObjectRef::Body(2), dir, dist_m: 1.0e6, over_ground: false, ang_radius, name: "Moon".into(), system: false }
    }

    /// The in-view marker: a destination dead ahead lands in the view's middle with a level head, at both sizes; with the
    /// head turned left it moves right on the screen (and with the head turned up, down); behind the eye, or turned off the
    /// screen, there is none. The disc's radius on the screen is the focal times tan of its angular radius dead ahead.
    #[test]
    fn the_marker_is_at_the_destinations_direction() {
        for (w, hh) in [(1600.0f32, 900.0f32), (3440.0, 1440.0)] {
            let r = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, hh));
            let level = SeatView::new(EYE, Head::default(), r);
            let (at, disc) = mark_place(&level, EYE, r, &mark(Vec3::NEG_Z, 0.01)).unwrap();
            assert!((at - r.center()).length() < 1e-2, "{at:?}");
            let focal = 0.5 * hh / (0.5 * super::super::seat::view_vertical_fov(w / hh, 0.0)).tan();
            assert!((disc - focal * 0.01f32.tan()).abs() < 0.05, "{disc} against {}", focal * 0.01f32.tan());
            let left = SeatView::new(EYE, Head::turned(Vec2::new(0.2, 0.0)), r);
            let (at_left, _) = mark_place(&left, EYE, r, &mark(Vec3::NEG_Z, 0.01)).unwrap();
            assert!(at_left.x > r.center().x + 10.0 && (at_left.y - r.center().y).abs() < 1.0, "{at_left:?}");
            let up = SeatView::new(EYE, Head::turned(Vec2::new(0.0, 0.2)), r);
            let (at_up, _) = mark_place(&up, EYE, r, &mark(Vec3::NEG_Z, 0.01)).unwrap();
            assert!(at_up.y > r.center().y + 10.0, "{at_up:?}");
            // A destination 10 degrees right of the nose is right of the middle by the focal times tan 10 degrees.
            let right = Vec3::new(10f32.to_radians().sin(), 0.0, -10f32.to_radians().cos());
            let (at_right, _) = mark_place(&level, EYE, r, &mark(right, 0.0)).unwrap();
            assert!((at_right.x - r.center().x - focal * 10f32.to_radians().tan()).abs() < 0.05, "{at_right:?}");
            assert!(mark_place(&level, EYE, r, &mark(Vec3::Z, 0.01)).is_none(), "behind the eye");
            assert!(mark_place(&level, EYE, r, &mark(Vec3::new(1.0, 0.0, -0.2).normalize(), 0.01)).is_none(), "off the screen");
        }
    }

    /// The speed's text, at its longest, stands clear of the radar's ring, the hologram's ring and the throttle's gauge at
    /// 1600 x 900 and at 3440 x 1440, seen straight ahead (by at least 0.3 % of the screen's height).
    #[test]
    fn the_speed_clears_the_radar() {
        for (w, hh) in [(1600.0f32, 900.0f32), (3440.0, 1440.0)] {
            let r = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, hh));
            let view = SeatView::new(EYE, Head::default(), r);
            let rings = [ring(&view, r, &anchor::RADAR, anchor::RADAR_BOX), ring(&view, r, &anchor::HEALTH, anchor::HEALTH_BOX)];
            let gauge: Vec<egui::Pos2> = (0..=64)
                .map(|i| {
                    let f = i as f32 / 64.0;
                    let [(x0, y0), (x1, y1)] = GAUGE;
                    anchor::place(&view, &anchor::THROTTLE, anchor::THROTTLE_BOX, r, at(r, x0 + (x1 - x0) * f, y0 + (y1 - y0) * f)).unwrap()
                })
                .collect();
            let margin = 0.003 * hh;
            for text in longest() {
                let flat = SPEED_ALIGN.anchor_size(at(r, SPEED_AT.0, SPEED_AT.1), text_size(&text, SPEED_FONT * hh));
                let quad = [flat.left_top(), flat.right_top(), flat.right_bottom(), flat.left_bottom()].map(|p| anchor::place(&view, &anchor::THROTTLE, anchor::THROTTLE_BOX, r, p).unwrap());
                let edge: Vec<egui::Pos2> = (0..4).flat_map(|k| (0..32).map(move |j| quad[k] + (quad[(k + 1) % 4] - quad[k]) * (j as f32 / 32.0))).collect();
                let near = |p: &egui::Pos2| inside(*p, &quad) || edge.iter().any(|e| e.distance(*p) < margin);
                let b = egui::Rect::from_points(&quad);
                for (k, ring) in rings.iter().enumerate() {
                    assert!(!edge.iter().any(|e| inside(*e, ring)) && !ring.iter().any(near), "{text:?} at {w}x{hh} meets ring {k}: {b:?}");
                }
                assert!(!gauge.iter().any(near), "{text:?} at {w}x{hh} meets the gauge: {b:?}");
            }
        }
    }
}
