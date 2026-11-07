//! The small corner readout while he's on (spec F8): speed, Mach in air, height, gravity, heat. Drawn after
//! `UiSet::Draw`, so hiding the panels has nothing to do with it.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use se_core::units::SPEED_OF_LIGHT;

use crate::hud::HudState;
use crate::i18n::I18n;
use crate::mods::ModControl;

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
    /// The bar, 0..1, and the look's colour at that temperature.
    pub heat: f32,
    pub heat_colour: [f32; 3],
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

pub(super) fn draw(mut contexts: EguiContexts, hero: Res<Hero>, hud: Res<HudState>, i18n: Res<I18n>, control: Res<ModControl>) -> Result {
    if hud.hidden {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
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
                ui.label(i18n.tr("Heat"));
                let [cr, cg, cb] = r.heat_colour.map(|c| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8);
                ui.add(egui::ProgressBar::new(r.heat).desired_width(70.0).desired_height(8.0).fill(egui::Color32::from_rgb(cr, cg, cb)));
                ui.end_row();
            });
            ui.small(i18n.tr("Tab: menus"));
        });
    });
    if control.panels_hidden() {
        guide(ctx, &hero);
    }
    Ok(())
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
    let line = controls::prompt(hero.state, hero.fast, hero.dive, super_on, hero.stacks, hero.space.charging, hero.super_run);
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
