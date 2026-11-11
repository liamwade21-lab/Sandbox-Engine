//! The Q menu: Garry's Mod's spawn menu, open only while Q is held. Three tabs: Spawnlists (the game's own lists; a click on a
//! prop spawns it where the player looks), Weapons (a click takes it in hand) and Mods (a click plays as that mod: the port's
//! own mods, through `mods::ModControl`, as the Mods window's rows do).
//!
//! The props' pictures are made here, a few at a time on worker threads (`icon.rs`), the first time a prop is on screen.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy_egui::{EguiContexts, egui};

use crate::mods::{ModControl, ModRegistry, grey_reason};

use super::assets::{GmodAssets, load_prop, prop_name};
use super::audio::Cue;
use super::icon::{IconMesh, IconTexture, render_icon};
use super::weapons::{Arsenal, WEAPONS};
use super::{Mode, Request, Sandbox};

/// A picture's side in texels, and on screen in points.
const ICON_PX: u32 = 96;
const ICON_SIZE: f32 = 72.0;
const ICON_GAP: f32 = 4.0;
/// Pictures being made at once.
const ICON_JOBS: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Spawnlists,
    Weapons,
    Mods,
}

enum Icon {
    Making,
    Ready(egui::TextureHandle),
    Failed,
}

#[derive(Resource, Default)]
pub struct Menu {
    tab: Tab,
    list: usize,
    icons: HashMap<String, Icon>,
    jobs: Vec<Task<(String, Option<Vec<u8>>)>>,
    /// What the pointer was over last frame (the rollover sound plays once on entering).
    hovered: Option<String>,
    /// A test's choice of tab.
    pub dev_tab: Option<String>,
}

/// Garry's Mod's Derma look, near enough: grey frames, a white tree, a dark sheet of icons, blue for what is chosen.
mod look {
    use bevy_egui::egui::Color32;
    pub const FRAME: Color32 = Color32::from_rgb(108, 111, 114);
    pub const FRAME_EDGE: Color32 = Color32::from_rgb(40, 42, 46);
    pub const TAB: Color32 = Color32::from_rgb(84, 87, 91);
    pub const TAB_ON: Color32 = Color32::from_rgb(150, 153, 157);
    pub const TREE: Color32 = Color32::from_rgb(244, 244, 244);
    pub const TREE_TEXT: Color32 = Color32::from_rgb(40, 40, 40);
    pub const SHEET: Color32 = Color32::from_rgb(58, 60, 64);
    pub const TILE: Color32 = Color32::from_rgb(74, 77, 82);
    pub const BLUE: Color32 = Color32::from_rgb(40, 140, 250);
    pub const TEXT: Color32 = Color32::from_rgb(240, 240, 240);
    pub const DIM: Color32 = Color32::from_rgb(190, 192, 196);
    pub const WARN: Color32 = Color32::from_rgb(255, 190, 90);
}

/// A prop's picture: its meshes with a small mip of their textures, lit brightly (Half-Life 2's textures are dark).
fn make_icon(install: Option<Arc<se_core::gmod::GmodInstall>>, path: &str) -> Option<Vec<u8>> {
    let prop = load_prop(install.as_deref(), path).ok()?;
    let textures: Vec<Option<(u32, u32, Vec<u8>)>> = prop
        .meshes
        .iter()
        .map(|m| {
            let t = m.material.texture.as_ref()?;
            // The mip nearest twice the picture's size.
            let mip = (0..t.mips.len()).rev().find(|i| (t.width >> i).max(1) >= ICON_PX).unwrap_or(0);
            Some(((t.width >> mip).max(1), (t.height >> mip).max(1), t.rgba(mip)))
        })
        .collect();
    let meshes: Vec<IconMesh> = prop
        .meshes
        .iter()
        .zip(&textures)
        .map(|(m, t)| IconMesh {
            positions: &m.positions,
            normals: &m.normals,
            uvs: &m.uvs,
            indices: &m.indices,
            texture: t.as_ref().map(|(w, h, rgba)| IconTexture { width: *w, height: *h, rgba }),
            tint: m.material.tint.map(|c| c * 1.7),
            alpha_cut: m.material.alpha_cut,
            two_sided: m.material.two_sided,
        })
        .collect();
    Some(render_icon(&meshes, ICON_PX))
}

/// The pictures that finished since last frame become textures; the ones asked for start, a few at a time.
fn pump_icons(menu: &mut Menu, ctx: &egui::Context, assets: &GmodAssets, wanted: &[String]) {
    let mut i = 0;
    while i < menu.jobs.len() {
        match check_ready(&mut menu.jobs[i]) {
            Some((path, rgba)) => {
                let icon = match rgba {
                    Some(rgba) if rgba.len() == (ICON_PX * ICON_PX * 4) as usize => {
                        let image = egui::ColorImage::from_rgba_unmultiplied([ICON_PX as usize, ICON_PX as usize], &rgba);
                        Icon::Ready(ctx.load_texture(format!("sandbox icon {path}"), image, egui::TextureOptions::LINEAR))
                    }
                    _ => Icon::Failed,
                };
                menu.icons.insert(path, icon);
                drop(menu.jobs.swap_remove(i));
            }
            None => i += 1,
        }
    }
    for path in wanted {
        if menu.jobs.len() >= ICON_JOBS {
            break;
        }
        if menu.icons.contains_key(path) {
            continue;
        }
        menu.icons.insert(path.clone(), Icon::Making);
        let (install, path) = (assets.install.clone(), path.clone());
        menu.jobs.push(AsyncComputeTaskPool::get().spawn(async move {
            let icon = make_icon(install, &path);
            (path, icon)
        }));
    }
}

/// The spawn menu, while Q is held.
#[allow(clippy::too_many_arguments)]
pub fn draw_menu(
    mut contexts: EguiContexts,
    mut sb: ResMut<Sandbox>,
    mut menu: ResMut<Menu>,
    assets: Res<GmodAssets>,
    mut arsenal: ResMut<Arsenal>,
    reg: Res<ModRegistry>,
    mut control: ResMut<ModControl>,
    flight: Res<crate::ship::Flight>,
    runner: Res<crate::scripting::ScriptRunner>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let menu = &mut *menu;
    if let Some(name) = menu.dev_tab.take() {
        menu.tab = match name.as_str() {
            "weapons" => Tab::Weapons,
            "mods" => Tab::Mods,
            _ => Tab::Spawnlists,
        };
    }
    // (Pictures still being made finish whether or not the menu is up.)
    if !sb.menu {
        pump_icons(menu, ctx, &assets, &[]);
        menu.hovered = None;
        return Ok(());
    }
    let screen = ctx.content_rect();
    let size = egui::vec2((screen.width() * 0.86).min(1500.0), (screen.height() * 0.84).min(900.0));
    let rect = egui::Rect::from_center_size(screen.center(), size);
    let mut wanted: Vec<String> = Vec::new();
    let mut hovered: Option<String> = None;
    egui::Area::new(egui::Id::new("sandbox_spawn_menu")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        ui.set_min_size(size);
        ui.set_max_size(size);
        ui.painter().rect_filled(rect, 6.0, look::FRAME);
        ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(1.5, look::FRAME_EDGE), egui::StrokeKind::Inside);
        let inner = rect.shrink(8.0);
        // The tabs.
        let mut x = inner.left();
        for (tab, name) in [(Tab::Spawnlists, "Spawnlists"), (Tab::Weapons, "Weapons"), (Tab::Mods, "Mods")] {
            let galley = ui.painter().layout_no_wrap(name.to_string(), egui::FontId::proportional(15.0), look::TEXT);
            let r = egui::Rect::from_min_size(egui::pos2(x, inner.top()), egui::vec2(galley.size().x + 26.0, 26.0));
            let resp = ui.interact(r, egui::Id::new(("sandbox_tab", name)), egui::Sense::click());
            let on = menu.tab == tab;
            let fill = if on { look::TAB_ON } else if resp.hovered() { look::TAB.gamma_multiply(1.25) } else { look::TAB };
            ui.painter().rect_filled(r, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, fill);
            ui.painter().galley(r.center() - galley.size() * 0.5, galley, look::TEXT);
            if resp.clicked() && !on {
                menu.tab = tab;
                sb.cues.push(Cue::ui("garrysmod/ui_click.wav"));
            }
            x = r.right() + 3.0;
        }
        // (What it is, and the keys, at the right of the tabs.)
        let hint = match sb.mode {
            Mode::Hero => "Release Q to close   ·   playing a mod: its own keys apply   ·   V: free flight",
            _ => "Release Q to close   ·   Z: undo   ·   V: free flight",
        };
        ui.painter().text(egui::pos2(inner.right() - 4.0, inner.top() + 13.0), egui::Align2::RIGHT_CENTER, hint, egui::FontId::proportional(13.0), look::DIM);
        let body = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.top() + 26.0), inner.max);
        ui.painter().rect_filled(body, 0.0, look::TAB_ON);
        let body = body.shrink(6.0);
        match menu.tab {
            Tab::Spawnlists => {
                // Left: the lists. Right: the chosen list's props.
                let tree = egui::Rect::from_min_max(body.min, egui::pos2(body.left() + 230.0, body.bottom()));
                let sheet = egui::Rect::from_min_max(egui::pos2(tree.right() + 6.0, body.top()), body.max);
                ui.painter().rect_filled(tree, 3.0, look::TREE);
                ui.painter().rect_filled(sheet, 3.0, look::SHEET);
                let mut picked = None;
                ui.scope_builder(egui::UiBuilder::new().max_rect(tree.shrink(4.0)), |ui| {
                    egui::ScrollArea::vertical().id_salt("sandbox_lists").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for (i, list) in assets.lists.iter().enumerate() {
                            let on = i == menu.list;
                            let text = format!("{}{}  ({})", "    ".repeat(list.depth), list.name, list.models.len());
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click());
                            if on || resp.hovered() {
                                ui.painter().rect_filled(r, 2.0, if on { look::BLUE } else { look::BLUE.gamma_multiply(0.25) });
                            }
                            ui.painter().text(r.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(13.5), if on { egui::Color32::WHITE } else { look::TREE_TEXT });
                            if resp.clicked() {
                                picked = Some(i);
                            }
                        }
                    });
                });
                if let Some(i) = picked.filter(|i| *i != menu.list) {
                    menu.list = i;
                    sb.cues.push(Cue::ui("garrysmod/ui_click.wav"));
                }
                let models: &[String] = assets.lists.get(menu.list).map_or(&[], |l| &l.models);
                ui.scope_builder(egui::UiBuilder::new().max_rect(sheet.shrink(6.0)), |ui| {
                    if !assets.looked {
                        ui.colored_label(look::TEXT, "Looking for Garry's Mod's files...");
                        return;
                    }
                    let cell = ICON_SIZE + ICON_GAP;
                    let cols = ((ui.available_width() + ICON_GAP) / cell).floor().max(1.0) as usize;
                    let rows = models.len().div_ceil(cols);
                    egui::ScrollArea::vertical().id_salt(("sandbox_sheet", menu.list)).auto_shrink([false, false]).show_rows(ui, ICON_SIZE, rows, |ui, range| {
                        ui.spacing_mut().item_spacing = egui::vec2(ICON_GAP, ICON_GAP);
                        for row in range {
                            ui.horizontal(|ui| {
                                for path in models.iter().skip(row * cols).take(cols) {
                                    let (r, resp) = ui.allocate_exact_size(egui::vec2(ICON_SIZE, ICON_SIZE), egui::Sense::click());
                                    ui.painter().rect_filled(r, 3.0, look::TILE);
                                    match menu.icons.get(path) {
                                        Some(Icon::Ready(tex)) => {
                                            ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                                        }
                                        Some(Icon::Failed) => {
                                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(26.0), look::DIM);
                                        }
                                        Some(Icon::Making) => {}
                                        None => wanted.push(path.clone()),
                                    }
                                    if resp.hovered() {
                                        ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(2.0, look::BLUE), egui::StrokeKind::Inside);
                                        hovered = Some(path.clone());
                                    }
                                    let resp = resp.on_hover_text(prop_name(path));
                                    if resp.clicked() {
                                        sb.requests.push(Request::Spawn(path.clone()));
                                    }
                                }
                            });
                        }
                    });
                });
            }
            Tab::Weapons => {
                ui.painter().rect_filled(body, 3.0, look::SHEET);
                ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink(10.0)), |ui| {
                    ui.colored_label(look::DIM, "Click a weapon to take it in hand.  1-4 and the mouse wheel pick one too.");
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                        for (i, w) in WEAPONS.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(190.0, 86.0), egui::Sense::click());
                            let on = arsenal.current == i;
                            ui.painter().rect_filled(r, 4.0, if on { look::BLUE.gamma_multiply(0.55) } else { look::TILE });
                            if resp.hovered() {
                                ui.painter().rect_stroke(r, 4.0, egui::Stroke::new(2.0, look::BLUE), egui::StrokeKind::Inside);
                                hovered = Some(w.name.to_string());
                            }
                            ui.painter().text(r.left_top() + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, w.name, egui::FontId::proportional(17.0), look::TEXT);
                            ui.painter().text(r.left_top() + egui::vec2(10.0, 32.0), egui::Align2::LEFT_TOP, format!("Slot {}", w.slot), egui::FontId::proportional(12.5), look::DIM);
                            ui.painter().text(r.left_top() + egui::vec2(10.0, 50.0), egui::Align2::LEFT_TOP, w.hint, egui::FontId::proportional(12.0), look::DIM);
                            if resp.clicked() {
                                arsenal.take(i);
                                sb.cues.push(Cue::ui("ui/buttonclickrelease.wav"));
                            }
                        }
                    });
                });
            }
            Tab::Mods => {
                ui.painter().rect_filled(body, 3.0, look::SHEET);
                ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink(10.0)), |ui| {
                    ui.colored_label(look::DIM, "Click who to play as.");
                    ui.add_space(8.0);
                    let row = |ui: &mut egui::Ui, title: &str, state: &str, text: &str, warn: Option<String>, on: bool, enabled: bool| -> bool {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(760.0), 64.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
                        ui.painter().rect_filled(r, 4.0, if on { look::BLUE.gamma_multiply(0.55) } else { look::TILE });
                        if enabled && resp.hovered() {
                            ui.painter().rect_stroke(r, 4.0, egui::Stroke::new(2.0, look::BLUE), egui::StrokeKind::Inside);
                        }
                        let ink = if enabled { look::TEXT } else { look::DIM.gamma_multiply(0.7) };
                        ui.painter().text(r.left_top() + egui::vec2(12.0, 8.0), egui::Align2::LEFT_TOP, title, egui::FontId::proportional(18.0), ink);
                        ui.painter().text(r.right_top() + egui::vec2(-12.0, 10.0), egui::Align2::RIGHT_TOP, state, egui::FontId::proportional(14.0), if on { egui::Color32::WHITE } else { look::DIM });
                        match warn {
                            Some(w) => ui.painter().text(r.left_top() + egui::vec2(12.0, 36.0), egui::Align2::LEFT_TOP, w, egui::FontId::proportional(13.0), look::WARN),
                            None => ui.painter().text(r.left_top() + egui::vec2(12.0, 36.0), egui::Align2::LEFT_TOP, text, egui::FontId::proportional(13.0), look::DIM),
                        };
                        ui.add_space(6.0);
                        resp.clicked()
                    };
                    let none_on = control.active.is_none();
                    if row(ui, "Yourself", if none_on { "Playing" } else { "" }, "First person, on foot: Garry's Mod's movement and weapons.", None, none_on, true) && !none_on {
                        control.request_off();
                        sb.before_hero = Mode::Walk;
                        sb.cues.push(Cue::ui("ui/buttonclickrelease.wav"));
                    }
                    if reg.mods.is_empty() {
                        ui.colored_label(look::DIM, "No mods found.");
                    }
                    for m in &reg.mods {
                        let on = control.active.as_deref() == Some(m.id.as_str());
                        let grey = if on { None } else { grey_reason(m, flight.active, runner.starting()) };
                        let state = match (on, control.loading) {
                            (true, true) => "Loading",
                            (true, false) => "Playing",
                            _ => "",
                        };
                        if row(ui, &m.name, state, &m.description, grey.as_ref().map(|g| g.to_string()), on, grey.is_none()) {
                            if on {
                                control.request_off();
                                sb.before_hero = Mode::Walk;
                            } else {
                                control.request_on(&m.id);
                            }
                            sb.cues.push(Cue::ui("ui/buttonclickrelease.wav"));
                            info!("sandbox: the Q menu's Mods tab: {} {}", m.name, if on { "off" } else { "on" });
                        }
                    }
                });
            }
        }
    });
    if hovered.is_some() && hovered != menu.hovered {
        sb.cues.push(Cue::new("garrysmod/ui_hover.wav", 0.25, 1.0));
    }
    menu.hovered = hovered;
    pump_icons(menu, ctx, &assets, &wanted);
    Ok(())
}
