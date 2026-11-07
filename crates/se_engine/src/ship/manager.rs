//! SE's ships manager (seShipsManager) in the Ships window: the "My ships" / "Other ships" tabs
//! (SMTab) list every ship in the world (seSpacecraftArray) with Take control (of spacecraft),
//! Control remotely, Teleport here, Destroy, Destroy all and a Nickname; the Build menu
//! (seShipsManagerBuildDialog) lists the designs by class (Astronaut, Probe, Shuttle, Starship,
//! Planetship, Space station, Building, Planetary base) with Build and Build fleet.
//!
//! The actions are `ship` script commands ([`Action`]), so scripts can do the same. Not ported:
//! SE's limit of piloting to its simulator mode (the port has no game modes) and its fleet size
//! and formation (not decoded; [`FLEET_SIZE`] ships in a V, [`fleet_offset`]).

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use se_core::units::format_distance;

use super::{Flight, Ship, ShipCommand, ShipLibrary, ShipUi};
use crate::camera::FlyCamera;
use crate::universe::UPosition;

/// Ships a Build fleet builds (assumed).
pub const FLEET_SIZE: usize = 5;

/// SE's build classes (the Build menu's tabs).
pub const CLASSES: [&str; 8] = ["Astronaut", "Probe", "Shuttle", "Starship", "Planetship", "Space station", "Building", "Planetary base"];

/// The manager's actions, `ship ACTION ...`; `Fly` is a bare `ship NAME` (build and take control).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Fly,
    Build,
    Fleet,
    Other,
    Take,
    Remote,
    Teleport,
    Destroy,
    DestroyAll,
    Nick,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        Some(match s {
            "fly" => Action::Fly,
            "build" => Action::Build,
            "fleet" => Action::Fleet,
            "other" => Action::Other,
            "take" => Action::Take,
            "remote" => Action::Remote,
            "teleport" => Action::Teleport,
            "destroy" => Action::Destroy,
            "destroyall" => Action::DestroyAll,
            "nick" => Action::Nick,
            _ => return None,
        })
    }
}

/// Camera-space offset of the `i`-th ship of a fleet built ahead of the camera (`length`: the
/// design's length): the first three lengths ahead, the others behind it to the left and right.
pub fn fleet_offset(i: usize, length: f64) -> DVec3 {
    let row = i.div_ceil(2) as f64;
    let side = if i == 0 { 0.0 } else if i % 2 == 1 { -1.0 } else { 1.0 };
    DVec3::new(side * row * length * 1.5, 0.0, -(3.0 + row * 1.2) * length)
}

/// The ship a script names: its number, else its nickname, else its design (id or name).
pub fn find_ship<'a>(ships: impl Iterator<Item = (Entity, &'a Ship)>, who: &str) -> Option<Entity> {
    let who = who.trim().trim_start_matches('#');
    let list: Vec<(Entity, &Ship)> = ships.collect();
    if let Ok(n) = who.parse::<u32>() {
        return list.iter().find(|(_, s)| s.serial == n).map(|(e, _)| *e);
    }
    let w = who.to_ascii_lowercase();
    list.iter()
        .find(|(_, s)| !s.nickname.is_empty() && s.nickname.to_ascii_lowercase() == w)
        .or_else(|| list.iter().find(|(_, s)| s.design.id.to_ascii_lowercase() == w || s.design.name.to_ascii_lowercase() == w))
        .map(|(e, _)| *e)
}

/// The class tab a design is listed under (SE's classes, else its own).
pub fn class_of(class: &str) -> String {
    CLASSES.iter().find(|c| c.eq_ignore_ascii_case(class.trim())).map_or_else(|| class.trim().to_string(), |c| c.to_string())
}

/// The manager's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Mine,
    Others,
}

pub fn draw_ship_window(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<ShipUi>,
    lib: Res<ShipLibrary>,
    mut flight: ResMut<Flight>,
    hud: Res<crate::hud::HudState>,
    cam: Res<FlyCamera>,
    ships: Query<(Entity, &Ship, &UPosition)>,
    mut writer: MessageWriter<ShipCommand>,
    control: Res<crate::mods::ModControl>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if hud.hidden {
        return Ok(());
    }
    // (With the panels: the Viltrumite mod hides them until Tab.)
    if !control.panels_hidden() {
        egui::Area::new(egui::Id::new("ships_button"))
            .anchor(egui::Align2::RIGHT_TOP, [-8.0, 36.0])
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if ui.button(if flight.active { "Ships (flying)" } else { "Ships" }).clicked() {
                    ui_state.show = !ui_state.show;
                }
            });
    }
    if !ui_state.show {
        return Ok(());
    }
    let mut send = |verb: &str, arg: String| {
        writer.write(ShipCommand { verb: verb.into(), arg });
    };
    let mut open = true;
    egui::Window::new(crate::skin::title(ctx, "Ships")).open(&mut open).default_width(380.0).default_pos([320.0, 60.0]).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut ui_state.tab, Tab::Mine, "My ships");
            ui.selectable_value(&mut ui_state.tab, Tab::Others, "Other ships");
            ui.separator();
            ui.toggle_value(&mut ui_state.build_menu, "Build menu");
        });
        // The ships in the world.
        let mine = ui_state.tab == Tab::Mine;
        let mut list: Vec<(Entity, &Ship, f64)> =
            ships.iter().filter(|(_, s, _)| s.mine == mine).map(|(e, s, p)| (e, s, p.0.delta_m(cam.pos).length())).collect();
        list.sort_by_key(|(_, s, _)| s.serial);
        if ui_state.picked.is_some_and(|p| !list.iter().any(|(e, ..)| *e == p)) {
            ui_state.picked = None;
        }
        egui::ScrollArea::vertical().id_salt("ship_list").max_height(150.0).show(ui, |ui| {
            if list.is_empty() {
                ui.weak(if mine { "No ships built: open the Build menu" } else { "No other ships" });
            }
            for (e, s, d) in &list {
                let flying = flight.ship == Some(*e);
                let name = if s.nickname.is_empty() { s.design.name.clone() } else { format!("{} ({})", s.nickname, s.design.name) };
                let label = format!("#{} {}{}", s.serial, name, if flying { "  [flying]" } else { "" });
                ui.horizontal(|ui| {
                    if ui.selectable_label(ui_state.picked == Some(*e), label).clicked() {
                        ui_state.picked = Some(*e);
                        ui_state.nick = s.nickname.clone();
                    }
                    ui.weak(format_distance(*d));
                });
            }
        });
        if let Some((e, s, _)) = ui_state.picked.and_then(|p| list.iter().find(|(e, ..)| *e == p)).copied() {
            let n = s.serial.to_string();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Take control").on_hover_text("Take control of spacecraft").clicked() {
                    send("ship", format!("take {n}"));
                }
                if ui.button("Control remotely").clicked() {
                    send("ship", format!("remote {n}"));
                }
                if ui.button("Teleport here").clicked() {
                    send("ship", format!("teleport {n}"));
                }
                if ui.button("Destroy").clicked() {
                    send("ship", format!("destroy {n}"));
                }
                if flight.ship == Some(e) {
                    if ui.button(if flight.active { "Leave (Esc)" } else { "Enter (F)" }).clicked() {
                        send("flight", if flight.active { "off".into() } else { "on".into() });
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Nickname");
                let r = ui.text_edit_singleline(&mut ui_state.nick);
                if r.lost_focus() && ui_state.nick != s.nickname {
                    let text = ui_state.nick.clone();
                    send("ship", format!("nick {n} {text}"));
                }
            });
        }
        if mine && !list.is_empty() && ui.button("Destroy all").clicked() {
            send("ship", "destroyall".to_string());
        }

        if ui_state.build_menu {
            ui.separator();
            // The Build menu: designs by class.
            let mut classes: Vec<String> = CLASSES.iter().map(|c| c.to_string()).collect();
            for d in &lib.designs {
                let c = class_of(&d.class);
                if !c.is_empty() && !classes.contains(&c) {
                    classes.push(c);
                }
            }
            ui.horizontal_wrapped(|ui| {
                for (i, c) in classes.iter().enumerate() {
                    let n = lib.designs.iter().filter(|d| class_of(&d.class) == *c).count();
                    ui.add_enabled_ui(n > 0, |ui| {
                        if ui.selectable_label(ui_state.class == i, format!("{c} ({n})")).clicked() {
                            ui_state.class = i;
                        }
                    });
                }
            });
            let class = classes.get(ui_state.class).cloned().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.text_edit_singleline(&mut ui_state.filter);
            });
            let filter = ui_state.filter.to_ascii_lowercase();
            let mut chosen: Option<usize> = None;
            egui::ScrollArea::vertical().id_salt("design_list").max_height(200.0).show(ui, |ui| {
                for (i, d) in lib.designs.iter().enumerate() {
                    if class_of(&d.class) != class || (!filter.is_empty() && !d.name.to_ascii_lowercase().contains(&filter)) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        if ui.selectable_label(ui_state.selected == i, &d.name).clicked() {
                            chosen = Some(i);
                        }
                        ui.weak(format!("{}  {} m", d.faction, d.length_m.round()));
                    });
                }
            });
            if let Some(i) = chosen {
                ui_state.selected = i;
            }
            if let Some(d) = lib.designs.get(ui_state.selected) {
                ui.separator();
                ui.heading(&d.name);
                ui.label(format!("{} | {} | {}", d.class, d.faction, d.pack));
                ui.label(format!("length {} m, mass {:.3e} kg", d.length_m.round(), d.mass_kg));
                ui.label(format!(
                    "main {:.1}  retro {:.1}  hover {:.1}  RCS {:.1} m/s²",
                    d.main_engines, d.retro_engines, d.hover_engines, d.corr_engines
                ));
                ui.label(format!(
                    "turn {:.4} rad/s²  warp up to 10^{:.0}{}",
                    d.turn_engines,
                    d.warp_boost_log,
                    if d.can_warp() { "" } else { " (no warp drive)" }
                ));
                ui.label(format!("aerodynamic model: {}", d.aero.name()));
                ui.horizontal(|ui| {
                    if ui.button("Build").clicked() {
                        send("ship", format!("build {}", d.id));
                    }
                    if ui.button("Build fleet").clicked() {
                        send("ship", format!("fleet {}", d.id));
                    }
                    if ui.button("Fly").on_hover_text("Build and take control").clicked() {
                        send("ship", d.id.clone());
                    }
                });
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut flight.gravity, "Gravity");
            ui.checkbox(&mut flight.collisions, "Collisions");
            ui.checkbox(&mut flight.assist, "Flight assist");
            ui.checkbox(&mut flight.aero, "Aerodynamics");
            // SE's cheats dialog: "Unlimited velocity" (FTL) and "Invulnerability" (God).
            ui.checkbox(&mut flight.ftl, "Unlimited velocity");
            ui.checkbox(&mut flight.god, "Invulnerability");
        });
    });
    if !open {
        ui_state.show = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn fleet_lines_up_in_a_v() {
        let o: Vec<DVec3> = (0..FLEET_SIZE).map(|i| fleet_offset(i, 10.0)).collect();
        assert_eq!(o[0], DVec3::new(0.0, 0.0, -30.0));
        assert!(o[1].x < 0.0 && o[2].x > 0.0 && o[1].z == o[2].z && o[1].z < o[0].z);
        assert!(o[3].z < o[1].z);
        // No two ships share a place.
        for i in 0..o.len() {
            for j in i + 1..o.len() {
                assert!(o[i].distance(o[j]) > 10.0);
            }
        }
    }

    #[test]
    fn ships_found_by_number_nickname_or_design() {
        let mk = |serial: u32, nick: &str, id: &str| Ship {
            design: Arc::new(se_core::ship::ShipDesign { id: id.into(), name: id.into(), ..Default::default() }),
            vel: DVec3::ZERO,
            ang_vel: DVec3::ZERO,
            main: 0.0,
            retro: 0.0,
            hover: 0.0,
            warp: 0.0,
            clearance: 1.0,
            radius: 1.0,
            landed: None,
            nickname: nick.into(),
            mine: true,
            serial,
            parked: None,
        };
        let ships = [(Entity::from_raw_u32(1).unwrap(), mk(1, "", "DragonFly")), (Entity::from_raw_u32(2).unwrap(), mk(2, "Rocinante", "Shuttle"))];
        let it = || ships.iter().map(|(e, s)| (*e, s));
        assert_eq!(find_ship(it(), "2"), Some(ships[1].0));
        assert_eq!(find_ship(it(), "#1"), Some(ships[0].0));
        assert_eq!(find_ship(it(), "rocinante"), Some(ships[1].0));
        assert_eq!(find_ship(it(), "dragonfly"), Some(ships[0].0));
        assert_eq!(find_ship(it(), "3"), None);
        assert_eq!(Action::parse("destroyall"), Some(Action::DestroyAll));
        assert_eq!(Action::parse("DragonFly"), None);
        assert_eq!(class_of("starship"), "Starship");
        assert_eq!(class_of("Fighter"), "Fighter");
    }
}
