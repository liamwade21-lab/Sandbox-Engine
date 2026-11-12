//! The small corner readout while he's on (spec F8): speed, Mach in air, height, gravity (no heat bar since 2026-11-12, the user's). Drawn after
//! `UiSet::Draw`, so hiding the panels has nothing to do with it.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use se_core::ephemeris::Ephemeris;
use se_core::units::SPEED_OF_LIGHT;

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::hud::HudState;
use crate::i18n::I18n;
use crate::mods::ModControl;
use crate::universe::SimTime;

use super::Hero;

/// What it shows, worked out with his motion each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Readout {
    pub speed: f64,
    /// `None` out of the air.
    pub mach: Option<f64>,
    /// Over the nearest ground, `None` far from everything.
    pub height: Option<f64>,
    pub gravity_g: f64,
}

/// m/s, km/s, then multiples of c. (`format_speed` turns to ly/s at 1e4 c, which says less here.)
pub fn format_speed(v: f64) -> String {
    let v = v.abs();
    if v < 999.5 {
        format!("{v:.0} m/s")
    } else if v < 0.01 * SPEED_OF_LIGHT {
        format!("{:.2} km/s", v / 1e3)
    } else {
        let c = v / SPEED_OF_LIGHT;
        if c < 100.0 {
            format!("{c:.2} c")
        } else if c < 1e6 {
            format!("{c:.0} c")
        } else {
            format!("{c:.2e} c")
        }
    }
}

pub fn format_height(h: f64) -> String {
    let a = h.abs();
    if a < 1e3 {
        format!("{h:.0} m")
    } else if a < 1e6 {
        format!("{:.1} km", h / 1e3)
    } else {
        format!("{:.0} km", h / 1e3)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    mut contexts: EguiContexts,
    hero: Res<Hero>,
    hud: Res<HudState>,
    i18n: Res<I18n>,
    control: Res<ModControl>,
    cam: Res<FlyCamera>,
    cams: Query<&Camera, With<MainCamera>>,
    (data, sim): (Res<SeData>, Res<SimTime>),
) -> Result {
    let ctx = contexts.ctx_mut()?;
    // Under the main menu nothing of his shows.
    if control.main_menu {
        return Ok(());
    }
    // A camera effect, not a panel: with the panels hidden too.
    speed_lines(ctx, &hero);
    if hud.hidden {
        return Ok(());
    }
    let r = &hero.readout;
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(6.0);
    // In the corner; in the menus under the Ships button, which comes back with the panels.
    let y = if control.panels_hidden() { 10.0 } else { 62.0 };
    egui::Area::new(egui::Id::new("hero_readout")).anchor(egui::Align2::RIGHT_TOP, [-10.0, y]).interactable(false).show(ctx, |ui| {
        frame.show(ui, |ui| {
            egui::Grid::new("hero_readout_grid").num_columns(2).spacing([8.0, 1.0]).show(ui, |ui| {
                let mut row = |name: &str, value: String| {
                    ui.label(i18n.tr(name));
                    ui.label(value);
                    ui.end_row();
                };
                row("Speed", format_speed(r.speed));
                if let Some(m) = r.mach {
                    row("Mach", format!("{m:.2}"));
                }
                if let Some(h) = r.height {
                    row("Height", format_height(h));
                }
                row("Gravity", format!("{:.2} g", r.gravity_g));
            });
            ui.small(i18n.tr("Tab: menus"));
        });
    });
    if control.panels_hidden() {
        guide(ctx, &hero);
        if let Some(vp) = cams.single().ok().and_then(|c| c.logical_viewport_size()) {
            targets(ctx, &hero, &cam, vp, &data, sim.jd);
        }
    }
    Ok(())
}

/// The lock-on's marks (`lock.rs`): a thin ring on the selection while the left button selects, gold corners on what he's locked on,
/// with its name, the distance to the stop and the trip's time from here (or an arrow at the screen's edge pointing round to it).
fn targets(ctx: &egui::Context, hero: &Hero, cam: &FlyCamera, vp: Vec2, data: &SeData, jd: f64) {
    let picking = super::lock::picks(hero);
    let show_sel = cam.selected.filter(|s| picking && Some(*s) != hero.lock);
    if hero.lock.is_none() && show_sel.is_none() {
        return;
    }
    let screen = ctx.content_rect();
    let s = (screen.height() / 540.0).max(1.0);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("hero_lock")));
    let view = crate::overlay::View::new(cam, vp);
    let eph = Ephemeris::new(&data.catalog);
    let dir_of = |o| view.local(eph.position(o, jd).delta_m(cam.pos).normalize_or_zero());
    let text = |at: egui::Pos2, align: egui::Align2, t: String, colour: egui::Color32| {
        let font = egui::FontId::proportional(7.0 * s);
        painter.text(at + egui::vec2(s, s), align, &t, font.clone(), egui::Color32::from_black_alpha(160));
        painter.text(at, align, t, font, colour);
    };
    let gold = egui::Color32::from_rgb(255, 211, 107);
    if let Some(sel) = show_sel {
        if let Some(p) = view.project(dir_of(sel)) {
            painter.circle_stroke(p, 9.0 * s, egui::Stroke::new(1.2 * s, egui::Color32::from_white_alpha(200)));
            text(p + egui::vec2(12.0 * s, -4.0 * s), egui::Align2::LEFT_CENTER, data.catalog.display_name(sel).to_string(), egui::Color32::from_white_alpha(220));
        }
    }
    let Some(lock) = hero.lock else { return };
    let mut line = data.catalog.display_name(lock).to_string();
    if let Some((left, eta)) = hero.lock_left {
        line += &format!("  {}", super::lock::format_distance(left));
        if let Some(t) = eta {
            line += &format!("  {}", super::lock::format_time(t));
        }
    }
    let d = dir_of(lock);
    match view.project(d).filter(|p| screen.shrink(4.0 * s).contains(*p)) {
        Some(p) => {
            let (r, l) = (11.0 * s, 5.0 * s);
            let stroke = egui::Stroke::new(1.6 * s, gold);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = p + egui::vec2(sx * r, sy * r);
                painter.line_segment([corner, corner - egui::vec2(sx * l, 0.0)], stroke);
                painter.line_segment([corner, corner - egui::vec2(0.0, sy * l)], stroke);
            }
            text(p + egui::vec2(0.0, r + 3.0 * s), egui::Align2::CENTER_TOP, line, gold);
        }
        None => {
            // Off the screen or behind: an arrow on a ring round the middle, the way to turn.
            let way = egui::vec2(d.x as f32, -d.y as f32);
            let way = if way.length() < 1e-6 { egui::vec2(0.0, -1.0) } else { way.normalized() };
            let at = screen.center() + way * screen.height() * 0.26;
            let tip = at + way * 8.0 * s;
            let side = egui::vec2(-way.y, way.x) * 5.0 * s;
            painter.add(egui::Shape::convex_polygon(vec![tip, at + side, at - side], gold, egui::Stroke::NONE));
            text(at - way * 6.0 * s, egui::Align2::CENTER_CENTER, line, gold);
        }
    }
}

/// Speed lines past the barrier in air (the user, 2026-11-07; their mod's `Fx.SpeedLines`, drawn on the screen here): faint streaks
/// running out from the middle, clear of him, as strong as `Hero::buffet`, more, longer and thicker with each stack. Under the panels.
fn speed_lines(ctx: &egui::Context, hero: &Hero) {
    let k = hero.buffet.min(1.0) as f32;
    if k < 0.02 {
        return;
    }
    let screen = ctx.content_rect();
    let c = screen.center();
    let reach = 0.5 * screen.size().length();
    let s = (screen.height() / 540.0).max(1.0);
    let stacks = hero.stacks.min(super::tune::STACKS_MAX) as f32;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("hero_speed_lines")));
    let t = hero.clock as f32;
    let n = 16 + 6 * stacks as usize;
    for i in 0..n {
        let fi = i as f32;
        // Each streak runs out over 1/RATE s, then comes back somewhere else.
        let run = t * 2.2 + fi * 0.618;
        let (lap, p) = (run.floor(), run.fract());
        let a = fi * 2.399 + lap * 1.31;
        let dir = egui::vec2(a.cos(), a.sin());
        let r = reach * (0.34 + 0.62 * p);
        let len = reach * (0.05 + 0.15 * k) * (1.0 + 0.3 * stacks);
        let alpha = (k * 0.4 * (std::f32::consts::PI * p).sin()).clamp(0.0, 1.0);
        painter.line_segment([c + dir * r, c + dir * (r + len)], egui::Stroke::new((1.0 + 0.25 * stacks) * s, egui::Color32::from_white_alpha((alpha * 255.0) as u8)));
    }
}

/// The user's own Viltrumite mod's aids (its `Hud.cs`): a cross in the middle, the charge bar under it (gold when full; a punch's shows a
/// tick where it becomes a charged one), and the keys for what he's doing in a line near the bottom.
fn guide(ctx: &egui::Context, hero: &Hero) {
    use super::controls;
    use super::impact::Action;
    let screen = ctx.content_rect();
    let s = (screen.height() / 540.0).max(1.0);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("hero_guide")));
    let c = screen.center();
    let cross = |half: f32, thick: f32, colour: egui::Color32| {
        painter.rect_filled(egui::Rect::from_center_size(c, egui::vec2(half * 2.0, thick)), 0.0, colour);
        painter.rect_filled(egui::Rect::from_center_size(c, egui::vec2(thick, half * 2.0)), 0.0, colour);
    };
    cross(7.0 * s, 3.0 * s, egui::Color32::from_black_alpha(115));
    cross(6.0 * s, 1.4 * s, egui::Color32::from_white_alpha(230));
    let (charge, tick) = if hero.space.charging {
        (Some(hero.space.charge as f32), None)
    } else if let Action::WindUp { held, .. } = hero.action {
        let full = super::tune::PUNCH_FULL_S as f32;
        (Some((held / full).min(1.0)), Some(super::tune::PUNCH_CHARGE_S as f32 / full))
    } else {
        (None, None)
    };
    if let Some(k) = charge {
        let (w, h) = (60.0 * s, 3.0 * s);
        let at = egui::pos2(c.x - w * 0.5, c.y + 14.0 * s);
        painter.rect_filled(egui::Rect::from_min_size(at - egui::vec2(s, s), egui::vec2(w + 2.0 * s, h + 2.0 * s)), 0.0, egui::Color32::from_black_alpha(128));
        let fill = if k >= 1.0 { egui::Color32::from_rgba_unmultiplied(255, 217, 89, 242) } else { egui::Color32::from_white_alpha(230) };
        painter.rect_filled(egui::Rect::from_min_size(at, egui::vec2(w * k.clamp(0.0, 1.0), h)), 0.0, fill);
        if let Some(t) = tick {
            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(at.x + w * t - 0.5 * s, at.y - 2.0 * s), egui::vec2(s, h + 4.0 * s)), 0.0, egui::Color32::from_rgba_unmultiplied(255, 217, 89, 230));
        }
    }
    let speed = hero.flow.speed;
    let super_on = hero.fast && controls::supersonic(&hero.flow, speed);
    let line = controls::prompt(hero.state, hero.fast, hero.dive, super_on, hero.stacks, hero.space.charging, hero.super_run, hero.lock.is_some());
    let size = (screen.height() / 72.0).clamp(11.0, 28.0);
    egui::Area::new(egui::Id::new("hero_guide_line"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -screen.height() * 0.12])
        .interactable(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new().fill(egui::Color32::from_black_alpha(107)).inner_margin(egui::Margin::symmetric((size * 0.9) as i8, (size * 0.35) as i8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = size * 0.3;
                    for (i, item) in line.split('|').enumerate() {
                        if i > 0 {
                            ui.add_space(size * 1.2);
                        }
                        let (key, what) = item.split_once(':').unwrap_or((item, ""));
                        ui.label(egui::RichText::new(key).size(size).color(egui::Color32::from_rgb(255, 211, 107)));
                        ui.label(egui::RichText::new(what).size(size).color(egui::Color32::from_rgb(237, 237, 230)));
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_read_as_the_spec_says() {
        assert_eq!(format_speed(0.0), "0 m/s");
        assert_eq!(format_speed(20.0), "20 m/s");
        assert_eq!(format_speed(999.0), "999 m/s");
        assert_eq!(format_speed(1500.0), "1.50 km/s");
        assert_eq!(format_speed(30_000.0), "30.00 km/s");
        // 1 % of c is where km/s stops saying much.
        assert_eq!(format_speed(3.0e6), "0.01 c");
        assert_eq!(format_speed(SPEED_OF_LIGHT), "1.00 c");
        assert_eq!(format_speed(1000.0 * SPEED_OF_LIGHT), "1000 c");
        // Where `format_speed` in se_core would say ly/s.
        assert_eq!(format_speed(1e5 * SPEED_OF_LIGHT), "100000 c");
        assert_eq!(format_speed(1e7 * SPEED_OF_LIGHT), "1.00e7 c");
    }

    #[test]
    fn heights_read_in_the_right_units() {
        assert_eq!(format_height(1.5), "2 m");
        assert_eq!(format_height(999.0), "999 m");
        assert_eq!(format_height(1500.0), "1.5 km");
        assert_eq!(format_height(350_000.0), "350.0 km");
        assert_eq!(format_height(3.0e7), "30000 km");
    }
}
