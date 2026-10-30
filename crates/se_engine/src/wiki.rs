//! Wiki window: the localized description of the selected object, read from
//! SpaceEngine's database files, or a generated summary when there is no entry.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{Catalog, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::locale::{WikiEntry, WikiIndex, wiki_index};
use se_core::units::format_distance;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::hud::{HudState, object_kind};
use crate::i18n::I18n;
use crate::universe::SimTime;

/// Wiki entries for the language `lang`. Rebuilt when the UI language changes.
#[derive(Resource, Default)]
pub struct Wiki {
    pub index: WikiIndex,
    lang: String,
}

pub struct WikiPlugin;

impl Plugin for WikiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_index.after(crate::i18n::init_i18n))
            .add_systems(EguiPrimaryContextPass, draw_wiki.in_set(crate::skin::UiSet::Draw));
    }
}

fn build_index(mut commands: Commands, data: Res<SeData>, i18n: Res<I18n>, mut hud: ResMut<HudState>) {
    let t = std::time::Instant::now();
    let index = wiki_index(&data.vfs, i18n.lang());
    info!("wiki ({}): {} entries in {:.0?}", i18n.lang(), index.len(), t.elapsed());
    commands.insert_resource(Wiki { index, lang: i18n.lang().to_string() });
    // Debug/screenshot aid: start with the window open.
    if std::env::var("SE_WIKI_OPEN").is_ok_and(|v| v == "1") {
        hud.show_wiki = true;
    }
}

fn catalog_names(cat: &Catalog, r: ObjectRef) -> &[String] {
    match r {
        ObjectRef::Star(i) => &cat.stars[i as usize].names,
        ObjectRef::Body(i) => &cat.bodies[i as usize].names,
        ObjectRef::Galaxy(i) => &cat.galaxies[i as usize].names,
        ObjectRef::Cluster(i) => &cat.clusters[i as usize].names,
        ObjectRef::Nebula(i) => &cat.nebulae[i as usize].names,
    }
}

/// Wiki entry whose `Name` matches any of the object's catalog names (case-insensitive).
fn lookup<'a>(index: &'a WikiIndex, cat: &Catalog, r: ObjectRef) -> Option<&'a WikiEntry> {
    catalog_names(cat, r).iter().find_map(|n| index.get(&n.to_lowercase()))
}

/// Name of the object's parent body, star system or galaxy, if it has one.
fn parent_name(cat: &Catalog, r: ObjectRef) -> Option<String> {
    match r {
        ObjectRef::Star(i) => cat.stars[i as usize].parent.clone(),
        ObjectRef::Body(i) => Some(cat.bodies[i as usize].parent.clone()),
        ObjectRef::Galaxy(_) => None,
        ObjectRef::Cluster(i) => Some(cat.clusters[i as usize].galaxy.clone()),
        ObjectRef::Nebula(i) => Some(cat.nebulae[i as usize].galaxy.clone()),
    }
    .filter(|s| !s.is_empty())
}

fn draw_wiki(
    mut contexts: EguiContexts,
    mut hud: ResMut<HudState>,
    cam: Res<FlyCamera>,
    sim: Res<SimTime>,
    data: Res<SeData>,
    mut wiki: ResMut<Wiki>,
    i18n: Res<I18n>,
) -> Result {
    if !hud.show_wiki {
        return Ok(());
    }
    if wiki.lang != i18n.lang() {
        wiki.index = wiki_index(&data.vfs, i18n.lang());
        wiki.lang = i18n.lang().to_string();
    }
    let ctx = contexts.ctx_mut()?;
    let cat = &data.catalog;
    let eph = Ephemeris::new(cat);
    let mut open = true;
    egui::Window::new(crate::skin::title(ctx, &i18n.tr("Wiki")))
        .open(&mut open)
        .default_pos([330.0, 40.0])
        .default_width(420.0)
        .show(ctx, |ui| {
            let Some(sel) = cam.selected else {
                ui.label(i18n.tr("Select an object to read about it."));
                return;
            };
            match lookup(&wiki.index, cat, sel) {
                Some(entry) => {
                    ui.heading(entry.loc_name.as_str());
                    ui.weak(object_kind(&data, sel, &i18n, None));
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| match &entry.description {
                        Some(d) => {
                            ui.label(d.as_str());
                        }
                        None => {
                            ui.weak(i18n.tr("No description."));
                        }
                    });
                }
                None => {
                    ui.heading(cat.display_name(sel));
                    ui.label(object_kind(&data, sel, &i18n, None));
                    ui.weak(i18n.tr("No wiki entry for this object. Summary:"));
                    ui.separator();
                    let pos = eph.position(sel, sim.jd);
                    let d = pos.delta_m(cam.pos);
                    ui.label(format!("{}: {}", i18n.tr("distance"), format_distance(d.length())));
                    let r = eph.radius_m(sel);
                    if r > 0.0 {
                        ui.label(format!("{}: {}", i18n.tr("radius"), format_distance(r)));
                    }
                    if let Some(parent) = parent_name(cat, sel) {
                        ui.label(format!("{}: {parent}", i18n.tr("parent")));
                    }
                }
            }
        });
    hud.show_wiki = open;
    Ok(())
}
