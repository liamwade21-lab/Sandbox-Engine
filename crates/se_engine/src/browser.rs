//! SpaceEngine's Star browser (nearby stars, sortable, and the search for planets by
//! parameters, `star_search.rs`; SE's Shift+F3) and System chart (the active planetary system drawn
//! to a logarithmic scale; SE's chart mode keys, F8 or 0).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{BodyTag, ObjectRef};
use se_core::units::{METERS_PER_PC, format_distance};

use crate::camera::{FlyCamera, start_travel};
use crate::data::SeData;
use crate::i18n::I18n;
use crate::system::ActiveSystem;

/// Stars listed by the browser.
const MAX_STARS: usize = 300;

pub struct BrowserPlugin;

impl Plugin for BrowserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BrowserState>()
            .add_systems(Startup, |mut st: ResMut<BrowserState>| {
                // For screenshots of the windows.
                if let Some(v) = std::env::var_os("SE_BROWSER_OPEN") {
                    st.show_stars = true;
                    st.show_chart = v != "search";
                    st.search_tab = v == "search";
                }
            })
            .add_systems(Update, browser_keys)
            .add_systems(EguiPrimaryContextPass, (draw_star_browser, draw_system_chart).in_set(crate::skin::UiSet::Draw));
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum SortBy {
    #[default]
    Distance,
    Brightness,
    Name,
}

#[derive(Resource, Default)]
pub(crate) struct BrowserState {
    pub(crate) show_stars: bool,
    show_chart: bool,
    /// The search tab is shown (else the nearby stars).
    pub(crate) search_tab: bool,
    sort: SortBy,
    /// Nearest stars: (star index, distance pc, apparent magnitude), refreshed periodically.
    stars: Vec<(u32, f64, f64)>,
    refreshed_at: f64,
}

fn browser_keys(keys: Res<ButtonInput<KeyCode>>, mut st: ResMut<BrowserState>, mut egui: EguiContexts) {
    if egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if shift && keys.just_pressed(KeyCode::F3) {
        st.show_stars = !st.show_stars;
    }
    // SE's chart mode: F8 or 0.
    if keys.any_just_pressed([KeyCode::F8, KeyCode::Digit0]) {
        st.show_chart = !st.show_chart;
    }
}

fn refresh_stars(st: &mut BrowserState, data: &SeData, cam: &FlyCamera) {
    let cam_pc = cam.pos.to_parsecs();
    let mut v: Vec<(u32, f64, f64)> = data
        .catalog
        .stars
        .iter()
        .enumerate()
        .filter(|(_, s)| s.has_position && !s.barycenter)
        .map(|(i, s)| {
            let d = s.pos_pc.distance(cam_pc).max(1e-9);
            (i as u32, d, s.abs_mag + 5.0 * (d.log10() - 1.0))
        })
        .collect();
    let n = MAX_STARS.min(v.len());
    if n > 0 {
        v.select_nth_unstable_by(n - 1, |a, b| a.1.total_cmp(&b.1));
        v.truncate(n);
    }
    st.stars = v;
}

fn draw_star_browser(
    mut contexts: EguiContexts,
    time: Res<Time>,
    data: Res<SeData>,
    mut cam: ResMut<FlyCamera>,
    mut st: ResMut<BrowserState>,
    i18n: Res<I18n>,
    mut search: ResMut<crate::star_search::SearchState>,
    mut pending: ResMut<crate::star_search::PendingSelect>,
    field: Option<Res<crate::procstars::ProcStarField>>,
    sim: Res<crate::universe::SimTime>,
) -> Result {
    if !st.show_stars {
        return Ok(());
    }
    let now = time.elapsed_secs_f64();
    if st.stars.is_empty() || now - st.refreshed_at > 1.0 {
        refresh_stars(&mut st, &data, &cam);
        st.refreshed_at = now;
    }
    let ctx = contexts.ctx_mut()?;
    let cat = &data.catalog;
    let mut open = true;
    let st = &mut *st;
    match st.sort {
        SortBy::Distance => st.stars.sort_by(|a, b| a.1.total_cmp(&b.1)),
        SortBy::Brightness => st.stars.sort_by(|a, b| a.2.total_cmp(&b.2)),
        SortBy::Name => st.stars.sort_by(|a, b| cat.stars[a.0 as usize].names[0].cmp(&cat.stars[b.0 as usize].names[0])),
    }
    egui::Window::new(crate::skin::title(ctx, &i18n.tr("Star browser")))
        .open(&mut open)
        .default_pos([300.0, 60.0])
        .default_size([520.0, 460.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut st.search_tab, false, i18n.tr("Nearby stars"));
                ui.selectable_value(&mut st.search_tab, true, i18n.tr("Search"));
            });
            ui.separator();
            if st.search_tab {
                crate::star_search::search_ui(ui, &mut search, &data, &mut cam, field.as_deref(), sim.jd, &i18n, &mut pending);
                return;
            }
            ui.horizontal(|ui| {
                ui.label(i18n.tr("Sort by"));
                ui.selectable_value(&mut st.sort, SortBy::Distance, i18n.tr("distance"));
                ui.selectable_value(&mut st.sort, SortBy::Brightness, i18n.tr("brightness"));
                ui.selectable_value(&mut st.sort, SortBy::Name, i18n.tr("Name"));
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("stars").striped(true).num_columns(5).show(ui, |ui| {
                    ui.strong(i18n.tr("Name"));
                    ui.strong(i18n.tr("Class"));
                    ui.strong(i18n.tr("Distance"));
                    ui.strong(i18n.tr("App. mag"));
                    ui.label("");
                    ui.end_row();
                    for &(i, d, m) in &st.stars {
                        let s = &cat.stars[i as usize];
                        let r = ObjectRef::Star(i);
                        if ui.selectable_label(cam.selected == Some(r), s.names[0].as_str()).clicked() {
                            cam.selected = Some(r);
                        }
                        ui.label(s.class.raw.as_str());
                        ui.label(format_distance(d * METERS_PER_PC));
                        ui.label(format!("{m:.2}"));
                        if ui.small_button("Go").clicked() {
                            cam.selected = Some(r);
                            start_travel(&mut cam, r, None);
                        }
                        ui.end_row();
                    }
                });
            });
        });
    if !open {
        st.show_stars = false;
    }
    Ok(())
}

/// One planet (or dwarf planet) with its moons, for the chart.
struct ChartPlanet {
    obj: ObjectRef,
    name: String,
    a_m: f64,
    radius_m: f64,
    color: egui::Color32,
    moons: Vec<(ObjectRef, String, f64)>,
}

fn body_color(c: Option<[f64; 3]>) -> egui::Color32 {
    let c = c.unwrap_or([0.75, 0.72, 0.68]);
    let m = c[0].max(c[1]).max(c[2]).max(1e-6);
    let f = |v: f64| ((v / m).powf(1.0 / 2.2) * 230.0) as u8;
    egui::Color32::from_rgb(f(c[0]), f(c[1]), f(c[2]))
}

fn chart_planets(data: &SeData, active: &ActiveSystem) -> Vec<ChartPlanet> {
    let cat = &data.catalog;
    let members = cat.system_members(&active.name);
    let mut planets: Vec<ChartPlanet> = Vec::new();
    for &m in &members {
        let ObjectRef::Body(i) = m else { continue };
        let b = &cat.bodies[i as usize];
        if !matches!(b.tag, BodyTag::Planet | BodyTag::DwarfPlanet) {
            continue;
        }
        // Dwarf planets only when they are of some size, as SpaceEngine's chart does.
        if b.tag == BodyTag::DwarfPlanet && b.radius_km < 400.0 {
            continue;
        }
        // A planet orbiting a barycentre (Earth in "Earth-Moon", Pluto in "Pluto-Charon")
        // is placed by the barycentre's orbit.
        let orbit = match cat.find(&b.parent) {
            Some(ObjectRef::Star(j)) if cat.stars[j as usize].barycenter && cat.stars[j as usize].orbit.is_some() => {
                cat.stars[j as usize].orbit.as_ref()
            }
            _ => b.orbit.as_ref(),
        };
        let Some(a) = orbit.map(|o| o.semi_major_axis).filter(|a| *a > 0.0) else { continue };
        let mut moons: Vec<(ObjectRef, String, f64)> = cat
            .children_of(&b.names[0])
            .iter()
            .filter_map(|&c| match c {
                ObjectRef::Body(j) => {
                    let mb = &cat.bodies[j as usize];
                    (mb.tag == BodyTag::Moon && mb.radius_km >= 100.0)
                        .then(|| (c, mb.names[0].clone(), mb.orbit.as_ref().map_or(0.0, |o| o.semi_major_axis)))
                }
                _ => None,
            })
            .collect();
        moons.sort_by(|x, y| x.2.total_cmp(&y.2));
        planets.push(ChartPlanet {
            obj: m,
            name: b.names[0].clone(),
            a_m: a,
            radius_m: b.radius_km * 1000.0,
            color: body_color(b.color),
            moons,
        });
    }
    planets.sort_by(|x, y| x.a_m.total_cmp(&y.a_m));
    planets
}

fn draw_system_chart(
    mut contexts: EguiContexts,
    data: Res<SeData>,
    active: Res<ActiveSystem>,
    mut cam: ResMut<FlyCamera>,
    mut st: ResMut<BrowserState>,
    i18n: Res<I18n>,
) -> Result {
    if !st.show_chart {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let planets = chart_planets(&data, &active);
    let mut open = true;
    egui::Window::new(crate::skin::title(ctx, &format!("{}: {}", i18n.tr("System chart"), active.name)))
        .open(&mut open)
        .default_pos([60.0, 520.0])
        .default_size([980.0, 300.0])
        .show(ctx, |ui| {
            if planets.is_empty() {
                ui.label(i18n.tr("No planets known in this system."));
                return;
            }
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(400.0), 260.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            let (amin, amax) = (planets[0].a_m.log10(), planets.last().map_or(1.0, |p| p.a_m.log10()));
            let span = (amax - amin).max(0.5);
            let x_of = |a: f64| rect.left() + 40.0 + ((a.log10() - amin) / span) as f32 * (rect.width() - 80.0);
            let y0 = rect.top() + 50.0;
            painter.line_segment(
                [egui::pos2(rect.left() + 10.0, y0), egui::pos2(rect.right() - 10.0, y0)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
            );
            let r_max = planets.iter().map(|p| p.radius_m).fold(1.0, f64::max);
            for (n, p) in planets.iter().enumerate() {
                let x = x_of(p.a_m);
                // Sizes by the cube root of radius keep giants and small worlds both visible.
                let r = (4.0 + 18.0 * (p.radius_m / r_max).cbrt()) as f32;
                let selected = cam.selected == Some(p.obj);
                let centre = egui::pos2(x, y0);
                painter.circle_filled(centre, r, p.color);
                if selected {
                    painter.circle_stroke(centre, r + 3.0, egui::Stroke::new(1.5, egui::Color32::YELLOW));
                }
                // Alternate label heights so neighbours do not overlap.
                let lift = if n % 2 == 1 { 14.0 } else { 0.0 };
                painter.text(
                    egui::pos2(x, y0 - r - 6.0 - lift),
                    egui::Align2::CENTER_BOTTOM,
                    p.name.as_str(),
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_gray(210),
                );
                let hit = ui.interact(egui::Rect::from_center_size(centre, egui::vec2(2.0 * r, 2.0 * r)), ui.id().with(&p.name), egui::Sense::click());
                if hit.clicked() {
                    cam.selected = Some(p.obj);
                }
                if hit.double_clicked() {
                    start_travel(&mut cam, p.obj, None);
                }
                // Moons in a column below their planet.
                for (k, (mo, name, _)) in p.moons.iter().take(9).enumerate() {
                    let c = egui::pos2(x, y0 + r + 14.0 + k as f32 * 18.0);
                    let sel = cam.selected == Some(*mo);
                    painter.circle_filled(c, 4.0, egui::Color32::from_gray(if sel { 255 } else { 170 }));
                    painter.text(c + egui::vec2(7.0, 0.0), egui::Align2::LEFT_CENTER, name.as_str(), egui::FontId::proportional(10.0), egui::Color32::from_gray(160));
                    let mh = ui.interact(egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)), ui.id().with(name), egui::Sense::click());
                    if mh.clicked() {
                        cam.selected = Some(*mo);
                    }
                    if mh.double_clicked() {
                        start_travel(&mut cam, *mo, None);
                    }
                }
            }
            ui.weak(i18n.tr("Click to select, double-click to go. Orbit distances are to a logarithmic scale."));
        });
    if !open {
        st.show_chart = false;
    }
    Ok(())
}
