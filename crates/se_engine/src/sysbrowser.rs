//! SE's solar system browser (F2 at the selection's level, Ctrl+F2 at the camera's system, Alt+F2 at
//! the system's root; main-def.cfg `SolarSystemBrowser`): a grid of `ThumbnailSize` thumbnails of a
//! level's stars, planets and moons, lit from `ViewAngle` 35 degrees off the view, sized on a
//! logarithmic scale (`LogScaleBase` 3, `RealSize` false) or by their true sizes, with dwarf planets
//! (`ShowDwarfPlanets`) and without dwarf moons (`ShowDwarfMoons`). A click selects, a double click
//! opens the object's satellites (or goes to it when it has none); Up goes back a level.
//!
//! SE renders each thumbnail with its 3D renderer; here they are shaded spheres in the body's
//! colour (atmosphere rim, rings, glowing stars), so surfaces and clouds are not shown.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{BodyTag, ObjectRef};

use crate::camera::{FlyCamera, start_center, start_travel};
use crate::data::SeData;
use crate::i18n::I18n;

const THUMBNAIL: f32 = 120.0;
const VIEW_ANGLE_DEG: f32 = 35.0;
const LOG_BASE: f64 = 3.0;

pub struct SysBrowserPlugin;

impl Plugin for SysBrowserPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SysBrowser { show_dwarf_planets: true, ..default() })
            .add_systems(Startup, |mut st: ResMut<SysBrowser>| {
                // For screenshots of the window: SE_SYSBROWSER=NAME opens it at that object's level.
                st.open_at = std::env::var("SE_SYSBROWSER").ok();
            })
            .add_systems(Update, browser_keys)
            .add_systems(EguiPrimaryContextPass, draw.in_set(crate::skin::UiSet::Draw));
    }
}

#[derive(Resource, Default)]
pub struct SysBrowser {
    pub open: bool,
    /// The object whose satellites are shown.
    level: Option<ObjectRef>,
    /// Levels to go back to.
    back: Vec<ObjectRef>,
    show_dwarf_planets: bool,
    show_dwarf_moons: bool,
    real_size: bool,
    open_at: Option<String>,
}

fn browser_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut st: ResMut<SysBrowser>,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    active: Res<crate::system::ActiveSystem>,
    mut egui: EguiContexts,
) {
    if let Some(name) = st.open_at.take() {
        if let Some(r) = data.catalog.find(&name) {
            st.open = true;
            st.level = Some(r);
        }
    }
    if egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false) || !keys.just_pressed(KeyCode::F2) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if shift {
        return;
    }
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let cat = &data.catalog;
    let root = cat.find(&active.name);
    // Selection level: the selection's parent (its own satellites when it has some).
    let level = if alt {
        root
    } else if ctrl {
        cam.follow.map(|f| f.target).or(root)
    } else {
        cam.selected.and_then(|s| level_of(cat, s).or(Some(s))).or(root)
    };
    if st.open && st.level == level {
        st.open = false;
    } else {
        st.open = true;
        st.level = level;
        st.back.clear();
    }
}

fn name_of(cat: &se_core::catalog::Catalog, r: ObjectRef) -> String {
    cat.display_name(r).to_string()
}

fn parent_of(cat: &se_core::catalog::Catalog, r: ObjectRef) -> Option<ObjectRef> {
    let p = match r {
        ObjectRef::Body(i) => cat.bodies[i as usize].parent.clone(),
        ObjectRef::Star(i) => cat.stars[i as usize].parent.clone().unwrap_or_default(),
        _ => return None,
    };
    if p.is_empty() { None } else { cat.find(&p) }
}

/// The level an object is shown in: its parent, where a barycentre of planets stands for its
/// primary (the Moon is in Earth's level, Earth in the Sun's).
fn level_of(cat: &se_core::catalog::Catalog, r: ObjectRef) -> Option<ObjectRef> {
    let p = parent_of(cat, r)?;
    if !is_bary(cat, p) {
        return Some(p);
    }
    match bary_primary(cat, p) {
        Some(primary) if primary != r => Some(primary),
        _ => level_of(cat, p).or(Some(p)),
    }
}

fn is_bary(cat: &se_core::catalog::Catalog, r: ObjectRef) -> bool {
    matches!(r, ObjectRef::Star(i) if cat.stars[i as usize].barycenter)
}

fn kids_of(cat: &se_core::catalog::Catalog, r: ObjectRef) -> &[ObjectRef] {
    cat_name(cat, r).map_or(&[][..], |n| cat.children_of(&n))
}

/// A barycentre of planets and moons shows as its largest member (Earth for Earth-Moon), whose
/// satellites then include the other members; one of stars shows all of them.
fn bary_primary(cat: &se_core::catalog::Catalog, bary: ObjectRef) -> Option<ObjectRef> {
    let kids = kids_of(cat, bary);
    if kids.iter().any(|k| matches!(k, ObjectRef::Star(_))) {
        return None;
    }
    kids.iter().copied().max_by(|a, b| radius_m(cat, *a).total_cmp(&radius_m(cat, *b)))
}

/// The orbit an object is placed by: a barycentre member's is its barycentre's.
fn orbit_a(cat: &se_core::catalog::Catalog, r: ObjectRef) -> f64 {
    let own = |r: ObjectRef| match r {
        ObjectRef::Body(i) => cat.bodies[i as usize].orbit.as_ref().map_or(0.0, |o| o.semi_major_axis),
        ObjectRef::Star(i) => cat.stars[i as usize].orbit.as_ref().map_or(0.0, |o| o.semi_major_axis),
        _ => 0.0,
    };
    match parent_of(cat, r) {
        Some(p) if is_bary(cat, p) && bary_primary(cat, p).is_some() => own(p),
        _ => own(r),
    }
}

/// A level's members: barycentres as their primary (or their stars), the other members of the
/// level's own barycentre as its satellites, dwarf planets and moons filtered as the settings say,
/// asteroids and comets only where nothing else orbits, ordered by orbit size (stars first).
fn members(st: &SysBrowser, cat: &se_core::catalog::Catalog, level: ObjectRef) -> Vec<ObjectRef> {
    let mut kids: Vec<ObjectRef> = kids_of(cat, level).to_vec();
    if let Some(p) = parent_of(cat, level).filter(|p| is_bary(cat, *p) && bary_primary(cat, *p) == Some(level)) {
        kids.extend(kids_of(cat, p).iter().copied().filter(|k| *k != level));
    }
    let minor_only = kids.iter().all(|k| matches!(*k, ObjectRef::Body(i) if matches!(cat.bodies[i as usize].tag, BodyTag::Asteroid | BodyTag::Comet | BodyTag::Spacecraft)));
    let mut out = Vec::new();
    let mut stack = kids;
    while let Some(k) = stack.pop() {
        if is_bary(cat, k) {
            match bary_primary(cat, k) {
                Some(p) => stack.push(p),
                None => stack.extend(kids_of(cat, k).iter().copied()),
            }
            continue;
        }
        let keep = match k {
            ObjectRef::Body(i) => match cat.bodies[i as usize].tag {
                BodyTag::DwarfPlanet => st.show_dwarf_planets,
                BodyTag::DwarfMoon => st.show_dwarf_moons,
                BodyTag::Spacecraft => false,
                BodyTag::Asteroid | BodyTag::Comet => minor_only,
                _ => true,
            },
            _ => true,
        };
        if keep {
            out.push(k);
        }
    }
    out.sort_by(|x, y| orbit_a(cat, *x).total_cmp(&orbit_a(cat, *y)));
    out.dedup();
    out
}

fn cat_name(cat: &se_core::catalog::Catalog, r: ObjectRef) -> Option<String> {
    match r {
        ObjectRef::Body(i) => cat.bodies[i as usize].names.first().cloned(),
        ObjectRef::Star(i) => cat.stars[i as usize].names.first().cloned(),
        _ => None,
    }
}

fn radius_m(cat: &se_core::catalog::Catalog, r: ObjectRef) -> f64 {
    se_core::ephemeris::Ephemeris::new(cat).radius_m(r).max(1.0)
}

/// What a thumbnail shows: base colour, emissive, atmosphere tint, rings.
struct Look {
    color: [f32; 3],
    glow: bool,
    atmosphere: Option<[f32; 3]>,
    rings: bool,
}

fn look(cat: &se_core::catalog::Catalog, r: ObjectRef) -> Look {
    match r {
        ObjectRef::Star(i) => {
            let s = &cat.stars[i as usize];
            Look { color: se_core::star::blackbody_rgb(s.teff.max(1000.0)), glow: true, atmosphere: None, rings: false }
        }
        ObjectRef::Body(i) => {
            let b = &cat.bodies[i as usize];
            let c = b.color.unwrap_or([0.6, 0.58, 0.55]);
            let m = c[0].max(c[1]).max(c[2]).max(1e-6);
            let color = [(c[0] / m) as f32 * 0.85, (c[1] / m) as f32 * 0.85, (c[2] / m) as f32 * 0.85];
            let atmosphere = b.block.sub("Atmosphere").map(|_| [0.55, 0.7, 1.0]);
            Look { color, glow: false, atmosphere, rings: b.block.sub("Rings").is_some() }
        }
        _ => Look { color: [0.7; 3], glow: false, atmosphere: None, rings: false },
    }
}

/// A shaded sphere of radius `r` at `c`, lit from `VIEW_ANGLE_DEG` off the view.
fn paint_sphere(p: &egui::Painter, c: egui::Pos2, r: f32, lk: &Look) {
    const RINGS: usize = 14;
    const SEGS: usize = 40;
    let a = VIEW_ANGLE_DEG.to_radians();
    let light = Vec3::new(-a.sin(), 0.25, a.cos()).normalize();
    let shade = |x: f32, y: f32| {
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        let n = Vec3::new(x, -y, z);
        let i = if lk.glow { 0.75 + 0.25 * z } else { n.dot(light).max(0.0) * 0.95 + 0.03 };
        let col = lk.color.map(|v| (v * i).clamp(0.0, 1.0));
        let to8 = |v: f32| (v.powf(1.0 / 2.2) * 255.0) as u8;
        egui::Color32::from_rgb(to8(col[0]), to8(col[1]), to8(col[2]))
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, shade(0.0, 0.0));
    for ri in 1..=RINGS {
        let rr = ri as f32 / RINGS as f32;
        for s in 0..SEGS {
            let t = s as f32 / SEGS as f32 * std::f32::consts::TAU;
            let (x, y) = (rr * t.cos(), rr * t.sin());
            mesh.colored_vertex(c + egui::vec2(x, y) * r, shade(x * 0.999, y * 0.999));
        }
    }
    let idx = |ri: usize, s: usize| if ri == 0 { 0 } else { (1 + (ri - 1) * SEGS + s % SEGS) as u32 };
    for s in 0..SEGS {
        mesh.add_triangle(0, idx(1, s), idx(1, s + 1));
    }
    for ri in 1..RINGS {
        for s in 0..SEGS {
            mesh.add_triangle(idx(ri, s), idx(ri + 1, s), idx(ri + 1, s + 1));
            mesh.add_triangle(idx(ri, s), idx(ri + 1, s + 1), idx(ri, s + 1));
        }
    }
    p.add(mesh);
    if let Some(t) = lk.atmosphere {
        let col = egui::Color32::from_rgba_unmultiplied((t[0] * 255.0) as u8, (t[1] * 255.0) as u8, (t[2] * 255.0) as u8, 90);
        p.circle_stroke(c, r + 1.5, egui::Stroke::new(2.5, col));
    }
    if lk.glow {
        let to8 = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
        let col = egui::Color32::from_rgba_unmultiplied(to8(lk.color[0]), to8(lk.color[1]), to8(lk.color[2]), 60);
        p.circle_stroke(c, r * 1.1, egui::Stroke::new(r * 0.12, col));
    }
    if lk.rings {
        let pts: Vec<egui::Pos2> = (0..=64).map(|i| {
            let t = i as f32 / 64.0 * std::f32::consts::TAU;
            c + egui::vec2(t.cos() * r * 2.1, t.sin() * r * 0.45)
        }).collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(r * 0.12, egui::Color32::from_rgba_unmultiplied(210, 195, 170, 150))));
    }
}

fn draw(
    mut contexts: EguiContexts,
    mut st: ResMut<SysBrowser>,
    mut cam: ResMut<FlyCamera>,
    data: Res<SeData>,
    i18n: Res<I18n>,
    hud: Res<crate::hud::HudState>,
) -> Result {
    if !st.open || hud.hidden {
        return Ok(());
    }
    let Some(level) = st.level else {
        st.open = false;
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let cat = &data.catalog;
    let list = members(&st, cat, level);
    let radii: Vec<f64> = list.iter().map(|r| radius_m(cat, *r)).collect();
    let (rmin, rmax) = radii.iter().fold((f64::INFINITY, 0.0f64), |(a, b), r| (a.min(*r), b.max(*r)));
    let mut open = true;
    let mut enter: Option<ObjectRef> = None;
    let mut up = false;
    let title = format!("{}: {}", i18n.tr("Solar system browser"), name_of(cat, level));
    egui::Window::new(crate::skin::title(ctx, &title)).id(egui::Id::new("sysbrowser")).open(&mut open).default_width(THUMBNAIL * 5.0 + 40.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.add_enabled(!st.back.is_empty() || level_of(cat, level).is_some(), egui::Button::new(i18n.tr("Up"))).clicked() {
                up = true;
            }
            ui.checkbox(&mut st.show_dwarf_planets, i18n.tr("Dwarf planets"));
            ui.checkbox(&mut st.show_dwarf_moons, i18n.tr("Dwarf moons"));
            ui.checkbox(&mut st.real_size, i18n.tr("Real size"));
            if let Some(sel) = cam.selected {
                if ui.button(i18n.tr("Center")).clicked() {
                    start_center(&mut cam, sel);
                }
                if ui.button(i18n.tr("Go to")).clicked() {
                    start_travel(&mut cam, sel, None);
                }
            }
        });
        ui.separator();
        if list.is_empty() {
            ui.label(i18n.tr("No satellites"));
            return;
        }
        egui::ScrollArea::vertical().max_height(THUMBNAIL * 3.6).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (r, rad) in list.iter().zip(&radii) {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(THUMBNAIL, THUMBNAIL + 18.0), egui::Sense::click());
                    let p = ui.painter_at(rect);
                    let selected = cam.selected == Some(*r);
                    if selected || resp.hovered() {
                        p.rect_filled(rect, 4.0, egui::Color32::from_white_alpha(if selected { 28 } else { 14 }));
                    }
                    // Log scale: each LogScaleBase-fold step down in radius loses an equal share.
                    let f = if st.real_size {
                        (rad / rmax).max(0.02)
                    } else {
                        let steps = (rmax / rad).max(1.0).ln() / LOG_BASE.ln();
                        let span = (rmax / rmin).max(1.0).ln() / LOG_BASE.ln();
                        1.0 - 0.75 * steps / span.max(1.0)
                    } as f32;
                    let lk = look(cat, *r);
                    let rr = THUMBNAIL * 0.5 * 0.8 * f * if lk.rings { 0.5 } else { 1.0 };
                    paint_sphere(&p, rect.center_top() + egui::vec2(0.0, THUMBNAIL * 0.5), rr.max(2.0), &lk);
                    p.text(rect.center_bottom() - egui::vec2(0.0, 2.0), egui::Align2::CENTER_BOTTOM, name_of(cat, *r), egui::FontId::proportional(12.0), ui.visuals().text_color());
                    if resp.clicked() {
                        cam.selected = Some(*r);
                    }
                    if resp.double_clicked() {
                        enter = Some(*r);
                    }
                }
            });
        });
    });
    if let Some(r) = enter {
        let has_kids = !members(&st, cat, r).is_empty();
        if has_kids {
            st.back.push(level);
            st.level = Some(r);
        } else {
            start_travel(&mut cam, r, None);
        }
    }
    if up {
        st.level = st.back.pop().or_else(|| level_of(cat, level));
    }
    if !open {
        st.open = false;
    }
    Ok(())
}
