//! On-screen interface (egui): time controls, navigation info, search, object
//! information and render settings.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::ObjectRef;
use se_core::coords::universe_to_radec;
use se_core::ephemeris::Ephemeris;
use se_core::units::{METERS_PER_PC, format_distance, format_speed};

use crate::camera::{FlyCamera, start_center, start_travel};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::i18n::I18n;
use crate::stars::StarSettings;
use crate::universe::SimTime;

#[derive(Resource, Default)]
pub struct HudState {
    pub search: String,
    pub show_places: bool,
    pub places_filter: String,
    pub place_name: String,
    pub show_settings: bool,
    pub show_help: bool,
    pub hidden: bool,
    pub show_wiki: bool,
    /// SE's Find object dialog (F3).
    pub show_find: bool,
    /// The date being typed on the navigation bar (SE: click the date, Enter applies).
    pub date_edit: Option<String>,
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudState>()
            .add_systems(Startup, open_from_env)
            .add_systems(EguiPrimaryContextPass, draw_hud.in_set(crate::skin::UiSet::Draw))
            .add_systems(Update, hud_keys);
    }
}

/// Env `SE_UI_OPEN` (testing): windows to open at start, e.g. `settings,graphics`.
fn open_from_env(mut hud: ResMut<HudState>, mut gfx: ResMut<crate::graphics::GraphicsUi>) {
    let Some(list) = std::env::var_os("SE_UI_OPEN") else { return };
    for w in list.to_string_lossy().split(',') {
        match w.trim() {
            "settings" => hud.show_settings = true,
            "find" => hud.show_find = true,
            "places" => hud.show_places = true,
            "wiki" => hud.show_wiki = true,
            "help" => hud.show_help = true,
            "graphics" => gfx.open = true,
            _ => {}
        }
    }
}

fn hud_keys(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut hud: ResMut<HudState>,
    mut sim: ResMut<SimTime>,
    mut timing: ResMut<crate::universe::TimingMode>,
    mut egui: EguiContexts,
    mut gfx: ResMut<crate::graphics::GraphicsUi>,
    (mut msg, i18n, shots, mut shot_no): (ResMut<crate::messages::Messages>, Res<I18n>, Res<crate::screenshot::ScreenshotSettings>, ResMut<crate::screenshot::ScreenshotNumber>),
) {
    if egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if keys.just_pressed(KeyCode::F11) && !shift {
        // SE's format and numbering, in <Pictures>/SpaceEngine-rs/ (screenshot.rs); Shift+F11 is the
        // high-resolution screenshot tool.
        crate::screenshot::take(&mut commands, &shots, &mut shot_no);
    }
    // SE's keys (its default binds): Ctrl+` the interface, F3 Find object (Shift+F3 is the Star
    // browser's search), F4 Settings, Ctrl+F4 graphics, F7 Locations, I the wiki. F1 (SE's universe
    // map, not ported) opens the port's list of controls.
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    if keys.just_pressed(KeyCode::F1) {
        hud.show_help = !hud.show_help;
    }
    if keys.just_pressed(KeyCode::Backquote) && ctrl {
        hud.hidden = !hud.hidden;
    }
    if keys.just_pressed(KeyCode::F3) && !shift && !ctrl {
        hud.show_find = !hud.show_find;
    }
    if keys.just_pressed(KeyCode::F4) && !shift {
        if ctrl {
            gfx.open = !gfx.open;
        } else {
            hud.show_settings = !hud.show_settings;
        }
    }
    if keys.just_pressed(KeyCode::F7) {
        hud.show_places = !hud.show_places;
    }
    if keys.just_pressed(KeyCode::KeyI) {
        hud.show_wiki = !hud.show_wiki;
    }
    if keys.just_pressed(KeyCode::Space) {
        sim.paused = !sim.paused;
        msg.show(i18n.tr(if sim.paused { "Time paused" } else { "Time resumed" }));
    }
    let rate = sim.rate;
    // L / K while paused also resume time (SE's tutorial: "The time will resume automatically").
    if keys.just_pressed(KeyCode::KeyL) && !shift && !ctrl {
        sim.rate = rate_step(sim.rate, true);
        sim.paused = false;
    }
    if keys.just_pressed(KeyCode::KeyK) && !shift && !ctrl {
        sim.rate = rate_step(sim.rate, false);
        sim.paused = false;
    }
    if keys.just_pressed(KeyCode::KeyJ) && !shift && !ctrl {
        sim.rate = -sim.rate;
    }
    if keys.just_pressed(KeyCode::Backslash) && ctrl {
        // SE's CurrentTime: the system clock's date, the rate untouched.
        sim.jd = SimTime::default().jd;
        msg.show(i18n.tr("Current time"));
    } else if keys.just_pressed(KeyCode::Backslash) && shift {
        // SE's TimingMode (Shift+Backslash): real time or fixed frame time (universe.rs).
        timing.real_time = !timing.real_time;
        msg.show(i18n.tr(if timing.real_time { "Real time mode" } else { "Frame time mode" }));
    } else if keys.just_pressed(KeyCode::Backslash) && !shift {
        sim.rate = 1.0;
        msg.show(i18n.tr("Time reset"));
    } else if sim.rate != rate {
        msg.show(format!("{}: x{}", i18n.tr("Time scale"), format_rate(sim.rate)));
    }
}

/// A date typed as SE writes it: "YYYY.MM.DD HH:MM[:SS]" (also with '-' or '/' in the date, time optional).
fn parse_date(text: &str) -> Option<f64> {
    let mut parts = text.trim().splitn(2, [' ', 'T']);
    let date: Vec<&str> = parts.next()?.split(['.', '-', '/']).collect();
    let time: Vec<&str> = parts.next().map_or(Vec::new(), |t| t.trim().split(':').collect());
    if date.len() != 3 || time.len() > 3 {
        return None;
    }
    let year: i64 = date[0].parse().ok()?;
    let (month, day): (u32, u32) = (date[1].parse().ok()?, date[2].parse().ok()?);
    let num = |i: usize| time.get(i).map_or(Some(0.0), |v| v.parse::<f64>().ok());
    let (h, m, sec) = (num(0)?, num(1)?, num(2)?);
    let ok = (1..=12).contains(&month) && (1..=31).contains(&day) && (0.0..24.0).contains(&h) && (0.0..60.0).contains(&m) && (0.0..61.0).contains(&sec);
    ok.then(|| se_core::time::DateTime::new(year, month, day, h as u32, m as u32, sec).to_jd())
}

/// SE's time-rate steps (L / K and the navigation buttons): 1, 3, 10, 30, 100, ... and 0.3, 0.1,
/// 0.03, ... (the tutorial reaches 10x with two presses of L). Keeps the sign (J reverses time).
pub(crate) fn rate_step(rate: f64, up: bool) -> f64 {
    let (sign, mag) = (if rate < 0.0 { -1.0 } else { 1.0 }, rate.abs().max(1e-30));
    // Steps are 10^(k/2) rounded to 1 and 3: k even -> 10^(k/2), k odd -> 3 x 10^((k-1)/2).
    let value = |k: i32| if k.rem_euclid(2) == 0 { 10f64.powi(k.div_euclid(2)) } else { 3.0 * 10f64.powi(k.div_euclid(2)) };
    // The step at or below the current rate, then one step up or down from there.
    let mut k = (2.0 * mag.log10()).floor() as i32;
    while value(k + 1) <= mag * 1.0001 {
        k += 1;
    }
    while value(k) > mag * 1.0001 {
        k -= 1;
    }
    let on_step = (value(k) - mag).abs() <= mag * 1e-4;
    let next = if up { k + 1 } else if on_step { k - 1 } else { k };
    sign * value(next).clamp(1e-12, 1e15)
}

/// A time rate as the navigation bar writes it: whole numbers plainly, fractions with their digits.
fn format_rate(rate: f64) -> String {
    if rate.abs() >= 1.0 { format!("{rate:.0}") } else { format!("{rate}") }
}

/// The object's type as SE names it (`se_core::classname`): "Yellow dwarf (G2V)", "Planet (Temperate
/// oceanic terra with life)" from SE's classes when the body's surface has been set up (`classes`:
/// the classes and whether it has life), "Spiral galaxy (Sb)", "Globular star cluster".
pub(crate) fn object_kind(data: &SeData, r: ObjectRef, i18n: &I18n, classes: Option<(se_core::surface_gen::Classes, bool)>) -> String {
    use se_core::catalog::BodyTag;
    let c = &data.catalog;
    let cap = |s: &str| {
        let mut ch = s.chars();
        ch.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + ch.as_str())
    };
    match r {
        ObjectRef::Star(i) => {
            let s = &c.stars[i as usize];
            if s.barycenter {
                // A binary planet's barycentre (SE's moon generator) is no star system.
                let kind = if se_core::climate::planet_barycentre(c, i) { "Barycenter" } else { "Star system" };
                return i18n.tr(kind).to_string();
            }
            let mass = (s.mass_kg > 0.0).then(|| s.mass_kg / se_core::units::SOLAR_MASS_KG);
            let kind = i18n.tr(se_core::classname::star_type(&s.class, mass));
            if s.class.raw.is_empty() { kind.to_string() } else { format!("{kind} ({})", s.class.raw) }
        }
        ObjectRef::Body(i) => {
            let b = &c.bodies[i as usize];
            let tag = i18n.tr(match b.tag {
                BodyTag::Planet => "Planet",
                BodyTag::DwarfPlanet => "Dwarf planet",
                BodyTag::Moon => "Moon",
                BodyTag::DwarfMoon => "Dwarf moon",
                BodyTag::Asteroid => "Asteroid",
                BodyTag::Comet => "Comet",
                BodyTag::Spacecraft => "Spacecraft",
            });
            let class = classes
                .map(|(cl, life)| i18n.planet_class(&cl, life))
                .filter(|s| !s.is_empty())
                .or_else(|| b.class.clone());
            match class {
                Some(cl) => format!("{tag} ({cl})"),
                None => tag.to_string(),
            }
        }
        ObjectRef::Galaxy(i) => {
            let k = &c.galaxies[i as usize].kind;
            let t = if k.starts_with('E') {
                "elliptical#galaxy_type"
            } else if k.starts_with("S0") || k.starts_with("SB0") {
                "lenticular#galaxy_type"
            } else if k.starts_with("SB") {
                "barred spiral#galaxy_type"
            } else if k.starts_with('S') {
                "spiral#galaxy_type"
            } else {
                "irregular#galaxy_type"
            };
            let word = i18n.tr(t);
            format!("{} {} ({k})", cap(word.split('#').next().unwrap_or(word)), i18n.tr("galaxy"))
        }
        ObjectRef::Cluster(i) => {
            // SE has two words: a galactic nucleus (`Kern`) cluster is a globular one (02-012).
            use se_core::catalog::ClusterType;
            let open = matches!(ClusterType::parse(&c.clusters[i as usize].kind), ClusterType::Open | ClusterType::Part);
            let t = if open { "open#star_cluster_type" } else { "globular#star_cluster_type" };
            let word = i18n.tr(t);
            format!("{} {}", cap(word.split('#').next().unwrap_or(word)), i18n.tr("star cluster"))
        }
        ObjectRef::Nebula(i) => {
            let k = c.nebulae[i as usize].kind.to_ascii_lowercase();
            let t = if k.starts_with('p') {
                "planetary#nebula_type"
            } else if k.starts_with('s') {
                return cap(i18n.tr("supernova remnant#nebula_type").split('#').next().unwrap_or("supernova remnant"));
            } else {
                "diffuse#nebula_type"
            };
            let word = i18n.tr(t);
            format!("{} {}", cap(word.split('#').next().unwrap_or(word)), i18n.tr("nebula"))
        }
    }
}

/// SE's overlay filters: orbits, labels (off / brief / detail) and markers, each switched per kind.
pub(crate) fn overlay_filters(ui: &mut egui::Ui, overlay: &mut crate::overlay::OverlaySettings, i18n: &I18n) {
    use crate::overlay::MarkerKind as K;
    ui.checkbox(&mut overlay.orbits, i18n.tr("orbits"));
    let modes = ["Off", "brief", "detail"];
    egui::ComboBox::from_label(i18n.tr("labels")).selected_text(i18n.tr(modes[overlay.labels_mode.min(2) as usize])).show_ui(ui, |ui| {
        for (v, name) in modes.iter().enumerate() {
            ui.selectable_value(&mut overlay.labels_mode, v as u8, i18n.tr(name));
        }
    });
    ui.checkbox(&mut overlay.markers, i18n.tr("markers"));
    ui.checkbox(&mut overlay.sky.constellations, i18n.tr("Constellations"));
    ui.indent("constellation_switches", |ui| constellation_switches(ui, &mut overlay.sky, i18n));
    ui.checkbox(&mut overlay.sky.grids, i18n.tr("Celestial grids"));
    ui.indent("grid_switches", |ui| grid_switches(ui, &mut overlay.sky, i18n));
    ui.collapsing(i18n.tr("Filter objects"), |ui| {
        egui::Grid::new("overlay_kinds").striped(true).show(ui, |ui| {
            ui.label("");
            ui.label(i18n.tr("Orbits"));
            ui.label(i18n.tr("Labels"));
            ui.label(i18n.tr("Markers"));
            ui.end_row();
            for k in K::ALL {
                let name = match k {
                    K::Ship => "Spacecraft",
                    K::Planet => "Planets",
                    K::Moon => "Moons",
                    K::DwarfPlanet => "Dwarf planets",
                    K::DwarfMoon => "Dwarf moons",
                    K::Asteroid => "Asteroids",
                    K::Comet => "Comets",
                    K::Sun => "Suns",
                    K::Barycenter => "Barycenters",
                    K::Star => "Stars",
                    K::Cluster => "Clusters",
                    K::Nebula => "Nebulae",
                    K::Galaxy => "Galaxies",
                };
                ui.label(i18n.tr(name));
                let check = |ui: &mut egui::Ui, set: &mut crate::overlay::KindSet| {
                    let mut on = set.has(k);
                    if ui.checkbox(&mut on, "").changed() {
                        set.set(k, on);
                    }
                };
                // Only bodies have orbits.
                if matches!(k, K::Star | K::Cluster | K::Nebula | K::Galaxy) {
                    ui.label("");
                } else {
                    check(ui, &mut overlay.orbit_kinds);
                }
                check(ui, &mut overlay.label_kinds);
                check(ui, &mut overlay.marker_kinds);
                ui.end_row();
            }
        });
    });
}

/// SE's constellation switches (Filters toolbar "Constellations": lines, boundaries, names, art) and
/// the Interface setting `FullConstellationNames`.
fn constellation_switches(ui: &mut egui::Ui, sky: &mut crate::overlay::SkySwitches, i18n: &I18n) {
    ui.checkbox(&mut sky.constel_lines, i18n.tr("Constellation lines"));
    ui.checkbox(&mut sky.constel_boundaries, i18n.tr("Constellation boundaries"));
    ui.checkbox(&mut sky.constel_labels, i18n.tr("Constellation names"));
    // SE 0.991 has the switch but ships no constellation art.
    ui.add_enabled(false, egui::Checkbox::new(&mut sky.constel_drawings, i18n.tr("Constellation art")));
    ui.checkbox(&mut sky.full_names, i18n.tr("Full constellation names"));
}

/// SE's grid switches (Filters toolbar "Celestial grids").
fn grid_switches(ui: &mut egui::Ui, sky: &mut crate::overlay::SkySwitches, i18n: &I18n) {
    ui.checkbox(&mut sky.grid_equatorial, i18n.tr("Equatorial grid"));
    ui.checkbox(&mut sky.grid_ecliptic, i18n.tr("Ecliptic grid"));
    ui.checkbox(&mut sky.grid_galactic, i18n.tr("Galactic grid"));
    ui.checkbox(&mut sky.grid_geographic, i18n.tr("Geographic grid"));
}

/// Whether the pointer is over the interface (egui's windows, toolbars and the info table), so a
/// click there does not also pick the object behind it.
/// A body's `Life` blocks for SE's life rows ("Life forms", "Life biomes", "Life origin"): the class and
/// type ("Organic multicellular"), the biomes ("marine, terrestrial") and the origin, `Panspermia true`
/// or else "Abiogenesis" (SE's flags 0x4000 / 0x8000, FUN_1404eb190). The rows' wording is assumed
/// from SE's strings; several blocks (organic and exotic life) are joined with "; ".
fn life_lines(block: &se_core::sc::Block, i18n: &I18n) -> Option<(String, String, String)> {
    let (mut forms, mut biomes, mut origins) = (Vec::new(), Vec::new(), Vec::new());
    for l in block.all("Life").filter_map(|i| i.block()) {
        let class = l.str("Class").unwrap_or("Organic");
        let kind = l.str("Type").map(|t| i18n.tr(&t.to_ascii_lowercase()).to_string());
        forms.push(match kind {
            Some(k) => format!("{} {k}", i18n.tr(class)),
            None => i18n.tr(class).to_string(),
        });
        if let Some(b) = l.str("Biome") {
            biomes.push(b.split('/').map(|x| i18n.tr(&x.trim().to_ascii_lowercase()).to_string()).collect::<Vec<_>>().join(", "));
        }
        origins.push(i18n.tr(if l.bool("Panspermia").unwrap_or(false) { "Panspermia" } else { "Abiogenesis" }).to_string());
    }
    if forms.is_empty() {
        return None;
    }
    origins.dedup();
    Some((forms.join("; "), biomes.join("; "), origins.join("; ")))
}

/// The mass of a catalog object (a barycentre's: its members', not the star default it carries).
fn object_mass(cat: &se_core::catalog::Catalog, r: ObjectRef) -> f64 {
    match r {
        ObjectRef::Body(p) => cat.bodies[p as usize].mass_kg,
        ObjectRef::Star(p) => {
            let s = &cat.stars[p as usize];
            if !s.barycenter {
                return s.mass_kg;
            }
            let name = s.names.first().map_or("", String::as_str);
            cat.children_of(name).iter().map(|&c| if c == r { 0.0 } else { object_mass(cat, c) }).sum()
        }
        _ => 0.0,
    }
}

/// SE's Hill and influence sphere radii of a body (FUN_14050f090): from its orbit and its parent's
/// mass; the members of a binary (a barycentre parent) take their barycentre's (SE copies the
/// parent's for its binary companion; that every member does here is assumed).
fn sphere_radii(cat: &se_core::catalog::Catalog, b: &se_core::catalog::BodyRecord) -> Option<(f64, f64)> {
    let o = b.orbit.as_ref()?;
    let parent = cat.find(&b.parent)?;
    if let ObjectRef::Star(p) = parent
        && cat.stars[p as usize].barycenter
    {
        let bary = &cat.stars[p as usize];
        let bo = bary.orbit.as_ref()?;
        let host = cat.find(bary.parent.as_deref()?)?;
        return se_core::orbit::hill_influence_radii(bo.semi_major_axis, bo.eccentricity, object_mass(cat, parent), object_mass(cat, host));
    }
    se_core::orbit::hill_influence_radii(o.semi_major_axis, o.eccentricity, b.mass_kg, object_mass(cat, parent))
}

/// SE's "Orbital resonances" (FUN_140514760): every sibling on an elliptic orbit of a kind SE
/// compares (FUN_1404e95b0: planets and dwarf planets together, moons with moons, asteroids with
/// asteroids, comets with asteroids and comets; the kind mapping is assumed) whose period makes a
/// resonance with this body's ([`se_core::orbit::resonance`]), as "name (p:q)".
fn orbital_resonances(cat: &se_core::catalog::Catalog, i: usize) -> String {
    use se_core::catalog::BodyTag as T;
    let b = &cat.bodies[i];
    let Some(o) = b.orbit.as_ref().filter(|o| o.eccentricity < 1.0 && o.period > 0.0) else { return String::new() };
    let group = |t: T| match t {
        T::Planet | T::DwarfPlanet => 0,
        T::Moon | T::DwarfMoon => 1,
        T::Asteroid => 2,
        T::Comet => 3,
        T::Spacecraft => 4,
    };
    let comparable = |t: T| match (group(b.tag), group(t)) {
        (3, g) => g == 2 || g == 3,
        (4, _) | (_, 4) => false,
        (a, g) => a == g,
    };
    let mut out = Vec::new();
    for &r in cat.children_of(&b.parent) {
        let ObjectRef::Body(j) = r else { continue };
        if j as usize == i {
            continue;
        }
        let s = &cat.bodies[j as usize];
        let Some(so) = s.orbit.as_ref().filter(|so| so.eccentricity < 1.0 && so.period > 0.0) else { continue };
        if !comparable(s.tag) {
            continue;
        }
        if let Some((p, q)) = se_core::orbit::resonance(so.period / o.period) {
            out.push(format!("{} ({p}:{q})", cat.display_name(r).replace('|', "/")));
        }
    }
    out.join(", ")
}

/// What an object orbiting the selection is counted as in the info panel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChildGroup {
    Planets,
    DwarfPlanets,
    Moons,
    Stars,
}

fn child_group(cat: &se_core::catalog::Catalog, c: ObjectRef) -> Option<ChildGroup> {
    use se_core::catalog::BodyTag;
    match c {
        ObjectRef::Star(i) if !cat.stars[i as usize].barycenter => Some(ChildGroup::Stars),
        ObjectRef::Body(i) => match cat.bodies[i as usize].tag {
            BodyTag::Planet => Some(ChildGroup::Planets),
            BodyTag::DwarfPlanet => Some(ChildGroup::DwarfPlanets),
            BodyTag::Moon | BodyTag::DwarfMoon => Some(ChildGroup::Moons),
            _ => None,
        },
        _ => None,
    }
}

fn is_barycenter(cat: &se_core::catalog::Catalog, r: ObjectRef) -> bool {
    matches!(r, ObjectRef::Star(i) if cat.stars[i as usize].barycenter)
}

/// What orbits `sel`, seen through barycentres: the Sun's planets include Earth (under the
/// Earth-Moon barycentre, beside the Moon), not the barycentre or the 200 binary asteroids'
/// barycentres; a star system's stars are its component stars. A planet under a barycentre also
/// has that barycentre's moons (Earth the Moon, Pluto Charon and the rest).
fn orbiting(cat: &se_core::catalog::Catalog, sel: ObjectRef) -> Vec<ObjectRef> {
    fn walk(cat: &se_core::catalog::Catalog, of: ObjectRef, depth: u32, skip: ObjectRef, out: &mut Vec<ObjectRef>) {
        // A child can share its parent's name (planets/Stars.sc "Vega" under "Vega/ALF Lyr").
        for &c in cat.children_of(cat.display_name(of)) {
            if c == skip || c == of || out.contains(&c) {
                continue;
            }
            if is_barycenter(cat, c) {
                if depth < 4 {
                    walk(cat, c, depth + 1, skip, out);
                }
            } else {
                out.push(c);
            }
        }
    }
    let mut out = Vec::new();
    walk(cat, sel, 0, sel, &mut out);
    if let ObjectRef::Body(i) = sel {
        let parent = &cat.bodies[i as usize].parent;
        if let Some(p) = cat.find(parent).filter(|&p| is_barycenter(cat, p)) {
            let mut sib = Vec::new();
            walk(cat, p, 0, sel, &mut sib);
            for c in sib {
                if child_group(cat, c) == Some(ChildGroup::Moons) && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// The child lists of [`orbiting`] by group, in the panel's order (planets, dwarf planets, moons,
/// stars).
fn child_groups(cat: &se_core::catalog::Catalog, sel: ObjectRef) -> [Vec<ObjectRef>; 4] {
    let children = orbiting(cat, sel);
    [ChildGroup::Planets, ChildGroup::DwarfPlanets, ChildGroup::Moons, ChildGroup::Stars]
        .map(|group| children.iter().copied().filter(|&c| child_group(cat, c) == Some(group)).collect())
}

/// The stars of the selection's system the info panel reads, from one `system_members` walk kept
/// while the catalog stands: `star_light`'s list and the temperature view's.
struct SystemStars {
    root: u32,
    lit: Vec<u32>,
    view: Vec<u32>,
}

impl SystemStars {
    fn new(cat: &se_core::catalog::Catalog, root: u32) -> Self {
        let walk = se_core::climate::system_walk(cat, root);
        Self { root, lit: se_core::climate::system_stars_in(cat, &walk), view: se_core::climate::display::view_stars(cat, walk) }
    }
}

/// egui temp-data key of the info panel's clickable rects (its count headers and their lists).
const INFO_HITS: &str = "info_table_hits";

/// Whether the pointer is over the interface, so a click there does not also select an object.
/// The info panel sits on egui's background layer, which egui itself never counts as over the
/// interface: a click on its fold-out arrows also picked the star under them.
pub fn pointer_over_ui(ctx: &egui::Context) -> bool {
    ctx.is_pointer_over_egui()
        || ctx.pointer_latest_pos().is_some_and(|p| {
            ctx.data(|d| d.get_temp::<Vec<egui::Rect>>(egui::Id::new(INFO_HITS))).is_some_and(|rects| rects.iter().any(|r| r.contains(p)))
        })
}

fn draw_hud(
    mut contexts: EguiContexts,
    mut hud: ResMut<HudState>,
    mut cam: ResMut<FlyCamera>,
    mut sim: ResMut<SimTime>,
    mut stars: ResMut<StarSettings>,
    mut overlay: ResMut<crate::overlay::OverlaySettings>,
    mut places: ResMut<crate::data::Places>,
    cfg: Res<EngineConfig>,
    data: Res<SeData>,
    time: Res<Time>,
    mut script_ui: ResMut<crate::scripting::ScriptUi>,
    i18n: Res<I18n>,
    mut gfx_ui: ResMut<crate::graphics::GraphicsUi>,
    (mut thermal, mut resonances, mut sys_stars, mut groups): (
        Local<Option<(ObjectRef, se_core::climate::display::Model)>>,
        Local<Option<(ObjectRef, String)>>,
        Local<Option<SystemStars>>,
        Local<Option<(ObjectRef, [Vec<ObjectRef>; 4])>>,
    ),
    (mut browser, mut msg): (ResMut<crate::browser::BrowserState>, ResMut<crate::messages::Messages>),
    (terrains, clusters): (
        Query<(&crate::system::SystemBody, &crate::terrain::Terrain)>,
        Option<Res<crate::clusters::ClusterField>>,
    ),
) -> Result {
    // The panel's system and child lists hold while the catalog stands (the temperature model
    // stays as set up for the selection).
    if data.is_changed() {
        *sys_stars = None;
        *groups = None;
    }
    let ctx = contexts.ctx_mut()?;
    // The info panel puts back its clickable rects each frame it is drawn.
    ctx.data_mut(|d| d.remove_temp::<Vec<egui::Rect>>(egui::Id::new(INFO_HITS)));
    if hud.hidden {
        return Ok(());
    }
    let cat = &data.catalog;
    let eph = Ephemeris::new(cat);

    // SE's four toolbars (seMainToolbar, seFiltersToolbar, seNavigationToolbar, seCameraToolbar):
    // tools at the left (above navigation, clear of the info table), filters on the right, navigation
    // bottom left, camera bottom right. SE can
    // auto-hide them (`Lock*Panel`); here they stay pinned.
    let bar_frame = egui::Frame::window(&ctx.global_style()).inner_margin(4.0);
    egui::Area::new(egui::Id::new("toolbar_main")).anchor(egui::Align2::LEFT_BOTTOM, [4.0, -44.0]).show(ctx, |ui| {
        bar_frame.show(ui, |ui| {
            ui.vertical(|ui| {
                let tool = |ui: &mut egui::Ui, label: &str, hint: &str, on: bool| {
                    let r = ui.add(egui::Button::selectable(on, i18n.tr(label)).min_size(egui::vec2(72.0, 0.0))).on_hover_text(i18n.tr(hint));
                    crate::script_gui::mark(ui, "Main toolbar", label, &r); // script HighlightWidget (script_gui.rs)
                    r.clicked()
                };
                if tool(ui, "Find", "Find object (F3)", hud.show_find) {
                    hud.show_find = !hud.show_find;
                }
                if tool(ui, "Places", "Locations (F7)", hud.show_places) {
                    hud.show_places = !hud.show_places;
                }
                if tool(ui, "Scripts", "Scripts", script_ui.show) {
                    script_ui.show = !script_ui.show;
                }
                let search_open = browser.show_stars && browser.search_tab;
                if tool(ui, "Search", "Star browser search (Shift+F3)", search_open) {
                    browser.show_stars = !search_open;
                    browser.search_tab = true;
                }
                if tool(ui, "Wiki", "Wiki (I)", hud.show_wiki) {
                    hud.show_wiki = !hud.show_wiki;
                }
                if tool(ui, "Settings", "Settings (F4)", hud.show_settings) {
                    hud.show_settings = !hud.show_settings;
                }
                if tool(ui, "Graphics", "Graphics (Ctrl+F4)", gfx_ui.open) {
                    gfx_ui.open = !gfx_ui.open;
                }
                if tool(ui, "Help", "Help (F1)", hud.show_help) {
                    hud.show_help = !hud.show_help;
                }
            });
        });
    });
    egui::Area::new(egui::Id::new("toolbar_filters")).anchor(egui::Align2::RIGHT_CENTER, [-4.0, 0.0]).show(ctx, |ui| {
        bar_frame.show(ui, |ui| {
            ui.vertical(|ui| {
                let filter = |ui: &mut egui::Ui, on: &mut bool, label: &str| {
                    let r = ui.add(egui::Button::selectable(*on, i18n.tr(label)).min_size(egui::vec2(96.0, 0.0)));
                    crate::script_gui::mark(ui, "Filters toolbar", label, &r); // script HighlightWidget
                    if r.clicked() {
                        *on = !*on;
                    }
                };
                filter(ui, &mut overlay.orbits, "Orbits");
                let mut labels = overlay.labels_mode > 0;
                filter(ui, &mut labels, "Labels");
                if labels != (overlay.labels_mode > 0) {
                    overlay.labels_mode = labels as u8;
                }
                filter(ui, &mut overlay.markers, "Markers");
                // SE's constellations and grids buttons; their switches on a right click.
                let sky = |ui: &mut egui::Ui, on: &mut bool, label: &str| {
                    let r = ui.add(egui::Button::selectable(*on, i18n.tr(label)).min_size(egui::vec2(96.0, 0.0)));
                    crate::script_gui::mark(ui, "Filters toolbar", label, &r); // script HighlightWidget
                    if r.clicked() {
                        *on = !*on;
                    }
                    r
                };
                sky(ui, &mut overlay.sky.constellations, "Constellations")
                    .context_menu(|ui| constellation_switches(ui, &mut overlay.sky, &i18n));
                sky(ui, &mut overlay.sky.grids, "Celestial grids").context_menu(|ui| grid_switches(ui, &mut overlay.sky, &i18n));
                filter(ui, &mut overlay.selection_pointer, "Selection pointer");
                filter(ui, &mut overlay.info_full, "Full info");
                filter(ui, &mut overlay.show_fps, "FPS");
            });
        });
    });
    egui::Area::new(egui::Id::new("toolbar_navigation")).anchor(egui::Align2::LEFT_BOTTOM, [4.0, -4.0]).show(ctx, |ui| {
        bar_frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // The date; a click turns it into a field to type a date in (Enter applies, Esc cancels).
                match hud.date_edit.as_mut() {
                    Some(text) => {
                        let resp = ui.add(egui::TextEdit::singleline(text).font(egui::TextStyle::Monospace).desired_width(170.0));
                        if !resp.has_focus() && !resp.lost_focus() {
                            resp.request_focus();
                        }
                        if resp.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                match parse_date(text) {
                                    Some(jd) => sim.jd = jd,
                                    None => msg.show(i18n.tr("Invalid date")),
                                }
                            }
                            hud.date_edit = None;
                        }
                    }
                    None => {
                        let date = egui::Label::new(egui::RichText::new(sim.date().to_string()).monospace().strong()).sense(egui::Sense::click());
                        if crate::script_gui::tag("Navigation toolbar|Date", ui.add(date).on_hover_text(i18n.tr("Set date"))).clicked() {
                            hud.date_edit = Some(sim.date().to_string());
                        }
                    }
                }
                if crate::script_gui::tag("Navigation toolbar|Pause time", ui.button(if sim.paused { "▶" } else { "⏸" }).on_hover_text(i18n.tr("Pause (Space)"))).clicked() {
                    sim.paused = !sim.paused;
                }
                if crate::script_gui::tag("Navigation toolbar|Decelerate time speed", ui.button("<<").on_hover_text(i18n.tr("Slower (K)"))).clicked() {
                    sim.rate = rate_step(sim.rate, false);
                }
                if crate::script_gui::tag("Navigation toolbar|Accelerate time speed", ui.button(">>").on_hover_text(i18n.tr("Faster (L)"))).clicked() {
                    sim.rate = rate_step(sim.rate, true);
                }
                if crate::script_gui::tag("Navigation toolbar|Set normal time speed", ui.button(i18n.tr("Real"))).clicked() {
                    sim.rate = 1.0;
                }
                if crate::script_gui::tag("Navigation toolbar|Set current time", ui.button(i18n.tr("Now")).on_hover_text(i18n.tr("Current time (Ctrl+\\)"))).clicked() {
                    sim.jd = SimTime::default().jd;
                }
                ui.label(format!("×{:.3e}", sim.rate));
                if let Some(sel) = cam.selected {
                    ui.separator();
                    if crate::script_gui::tag("Navigation toolbar|Go to object", ui.button(i18n.tr("Go to")).on_hover_text(i18n.tr("Go to (G); again: fast Go to"))).clicked() {
                        start_travel(&mut cam, sel, None);
                    }
                    if crate::script_gui::tag("Navigation toolbar|Center object", ui.button(i18n.tr("Center")).on_hover_text(i18n.tr("Center (C)"))).clicked() {
                        start_center(&mut cam, sel);
                    }
                    if matches!(sel, ObjectRef::Body(_)) && crate::script_gui::tag("Navigation toolbar|Land on object", ui.button(i18n.tr("Land")).on_hover_text(i18n.tr("Land on object (Shift+G)"))).clicked() {
                        crate::camera::start_land(&mut cam, sel);
                    }
                    // SE's binding buttons: free, follow, rotate with the selection; and tracking.
                    let mode = if cam.follow.is_some_and(|f| f.target == sel) { cam.bind_mode() } else { 0 };
                    let eph = se_core::ephemeris::Ephemeris::new(cat);
                    for (m, label, tip) in [(0u8, "Free", "Free binding to object (Shift+D)"), (1, "Follow", "Follow object (Shift+F)"), (2, "Rotate", "Rotate with object (Shift+R)")] {
                        if crate::script_gui::tag(&format!("Navigation toolbar|{label}"), ui.add(egui::Button::selectable(mode == m, i18n.tr(label))).on_hover_text(i18n.tr(tip))).clicked() {
                            cam.bind((m > 0).then_some(sel), m == 2, &eph, sim.jd);
                        }
                    }
                    if ui.add(egui::Button::selectable(cam.track, i18n.tr("Track"))).on_hover_text(i18n.tr("Track object (T)")).clicked() {
                        cam.track = !cam.track;
                    }
                    if ui.button(i18n.tr("Clear")).on_hover_text(i18n.tr("Clear the selection")).clicked() {
                        cam.selected = None;
                    }
                }
                // What the camera is bound to (SE's "Following" / "Rotating" / "Tracking").
                if let Some(f) = cam.follow {
                    ui.separator();
                    let word = i18n.tr(if f.sync { "Rotating" } else { "Following" });
                    crate::script_gui::tag("Navigation toolbar|Following object", ui.label(format!("{word}: {}", cat.display_name(f.target))));
                }
                if cam.track {
                    ui.label(i18n.tr("Tracking"));
                }
            });
        });
    });
    egui::Area::new(egui::Id::new("toolbar_camera")).anchor(egui::Align2::RIGHT_BOTTOM, [-4.0, -4.0]).show(ctx, |ui| {
        bar_frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                if cam.depth_m > 0.0 {
                    ui.label(format!("{}: {} {}", cam.nearest_name, i18n.tr("depth"), format_distance(cam.depth_m)));
                } else {
                    ui.label(format!("{} ({})", cam.nearest_name, format_distance(cam.nearest_surface)));
                }
                ui.separator();
                // SE's toolbar shows the assigned acceleration in Spacecraft and Aircraft mode (FUN_1405b94a0).
                let v = ui.label(if cam.move_mode.inertial() {
                    format!("{} {}", i18n.tr("acceleration"), crate::camera::format_acceleration(cam.speed))
                } else {
                    format!("{} {}", i18n.tr("velocity"), format_speed(cam.speed))
                });
                crate::script_gui::tag("Navigation toolbar|Current velocity", v.clone());
                crate::script_gui::tag("Navigation toolbar|Assigned velocity", v);
                ui.separator();
                // Zooms in as far as a telescope would (`fov` script command down to arcseconds).
                let fov_speed = (cam.fov_deg * 0.01).max(1e-4) as f64;
                crate::script_gui::tag("Camera toolbar|FOV", ui.add(egui::DragValue::new(&mut cam.fov_deg).range(crate::camera::MIN_FOV_DEG..=crate::camera::MAX_FOV_DEG).speed(fov_speed).max_decimals(4).suffix("°")).on_hover_text(i18n.tr("field of view")));
                // SE's "Mag limit": the star limit raised by the zoom, as a telescope would.
                let mut shown = stars.shown_limit();
                let edit = egui::DragValue::new(&mut shown).speed(0.05).max_decimals(1).prefix("m ");
                if crate::script_gui::tag("Camera toolbar|Plan mag limit", ui.add(edit).on_hover_text(i18n.tr("Mag limit"))).changed() {
                    stars.set_shown_limit(shown);
                }
                if overlay.show_fps {
                    ui.separator();
                    ui.label(format!("{:.0} fps", 1.0 / time.delta_secs().max(1e-6)));
                }
            });
        });
    });

    // SE's Find object dialog (F3): Enter selects the first match and closes it.
    if hud.show_find {
        let mut open = true;
        let mut close = false;
        egui::Window::new(crate::skin::title(ctx, &i18n.tr("Find object"))).open(&mut open).default_pos([120.0, 60.0]).default_width(260.0).show(ctx, |ui| {
            let resp = ui.text_edit_singleline(&mut hud.search);
            if !resp.has_focus() && hud.search.is_empty() {
                resp.request_focus();
            }
            let results = cat.search(&hud.search, 12);
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some((_, r)) = results.first() {
                    cam.selected = Some(*r);
                    close = true;
                }
            }
            for (name, r) in &results {
                ui.horizontal(|ui| {
                    if ui.selectable_label(cam.selected == Some(*r), name).clicked() {
                        cam.selected = Some(*r);
                    }
                    ui.weak(object_kind(&data, *r, &i18n, None));
                });
            }
        });
        hud.show_find = open && !close;
    }

    // SE's info table: the selection's text in the upper-left corner, its name coloured by kind,
    // brief or full (`TextMode`), the diameter (SE's SelectionRadius false).
    if let Some(sel) = cam.selected {
        // The skin's colours: the name in its kind's `Info*Color`, the text in `ObjectInfoColor`.
        let skin = crate::skin::text(ctx);
        let title = skin.kind(crate::overlay::MarkerKind::of(&data, sel).skin_key());
        let full = overlay.info_full;
        // Plain text, below the windows (and without their blurred background), above the orbit lines
        // and labels.
        ctx.move_to_top(egui::LayerId::new(egui::Order::Background, egui::Id::new("info_table")));
        egui::Area::new(egui::Id::new("info_table")).order(egui::Order::Background).anchor(egui::Align2::LEFT_TOP, [10.0, 8.0]).interactable(true).show(ctx, |ui| {
            ui.style_mut().visuals.override_text_color = Some(skin.info);
            let line = |ui: &mut egui::Ui, key: &str, value: String| {
                ui.label(egui::RichText::new(format!("{}: {value}", i18n.tr(key))).size(13.0));
            };
            ui.label(egui::RichText::new(cat.display_name(sel)).family(crate::i18n::bold_family()).size(20.0).color(title));
            let classes = terrains.iter().find(|(b, _)| b.obj == sel).map(|(_, t)| (t.surf_gen.classes, t.proc.has_life));
            ui.label(egui::RichText::new(object_kind(&data, sel, &i18n, classes)).size(13.0));
            let pos = eph.position(sel, sim.jd);
            let d = pos.delta_m(cam.pos);
            let dist = d.length();
            line(ui, "Distance", format_distance(dist));
            let r = eph.radius_m(sel);
            if r > 0.0 {
                line(ui, "Diameter", format_distance(2.0 * r));
            }
            if !full {
                return;
            }
            if r > 0.0 && dist > r {
                line(ui, "Apparent size", format_angle(2.0 * (r / dist).asin()));
            }
            match sel {
                ObjectRef::Star(i) => {
                    let s = &cat.stars[i as usize];
                    if !s.barycenter && s.class.kind != se_core::star::StarKind::BlackHole {
                        let dpc = (dist / METERS_PER_PC).max(1e-12);
                        line(ui, "Apparent magnitude", format!("{:.2}", s.abs_mag + 5.0 * (dpc / 10.0).log10()));
                        line(ui, "Absolute magnitude", format!("{:.2}", s.abs_mag));
                        let lum = se_core::star::lum_from_abs_mag(s.abs_mag) * 10f64.powf(-0.4 * se_core::star::bolometric_correction(s.teff));
                        line(ui, "Luminosity", format!("{lum:.4} Sun"));
                        line(ui, "Temperature", format!("{:.0} K", s.teff));
                    }
                    if !s.barycenter {
                        line(ui, "Mass", format!("{:.3} Sun", s.mass_kg / se_core::units::SOLAR_MASS_KG));
                        if let Some(age) = s.block.f64("Age") {
                            line(ui, "Age", format!("{age:.3} billion years"));
                        }
                    }
                }
                ObjectRef::Body(i) => {
                    let b = &cat.bodies[i as usize];
                    line(ui, "Parent", b.parent.clone());
                    if b.mass_kg > 0.0 {
                        line(ui, "Mass", format!("{:.4} Earth", b.mass_kg / se_core::units::EARTH_MASS_KG));
                        if r > 0.0 {
                            line(ui, "Gravity", format!("{:.3} g", 6.674_30e-11 * b.mass_kg / (r * r) / 9.806_65));
                            line(ui, "Density", format!("{:.3} g/cm³", b.mass_kg / (4.0 / 3.0 * std::f64::consts::PI * r * r * r) / 1000.0));
                        }
                    }
                    line(ui, "Albedo", format!("{:.3}", b.albedo));
                    // The system's stars, walked once per system; a body outside any system has
                    // none, as star_light and catalog_view find.
                    let root = cat.system_root(sel);
                    if let Some(root) = root.filter(|&r| sys_stars.as_ref().is_none_or(|s| s.root != r)) {
                        *sys_stars = Some(SystemStars::new(cat, root));
                    }
                    let system = sys_stars.as_ref().filter(|_| root.is_some());
                    // The stars as seen from the body this frame, for the temperature and the day lines.
                    let view = system.map(|s| se_core::climate::display::catalog_view_from(&eph, cat, i as usize, sim.jd, &s.view)).unwrap_or_default();
                    // SE's panel temperature for the stars' current distances; the model is set up
                    // once per selection.
                    if thermal.as_ref().is_none_or(|(r, _)| *r != sel) {
                        let light = se_core::climate::star_light_from(&eph, cat, sel, sim.jd, system.map_or(&[][..], |s| &s.lit));
                        let completion = cat
                            .climate
                            .as_ref()
                            .and_then(|t| se_core::climate::complete(t, &se_core::climate::catalog_body_in(cat, i as usize, &light)?));
                        *thermal = Some((sel, se_core::climate::display::catalog_model(cat, i as usize, &light, completion.as_ref())));
                    }
                    if let Some((_, m)) = thermal.as_ref() {
                        let p = m.panel(&view);
                        line(ui, "Average temperature", format!("{:.1} °C (night {:.0} K, day {:.0} K)", p.average - 273.15, p.night, p.day));
                    }
                    if let Some(p) = b.block.sub("Atmosphere").and_then(|a| a.f64("Pressure")) {
                        line(ui, "Atmosphere pressure", format!("{p:.4} atm"));
                    }
                    if let Some(o) = &b.orbit {
                        line(ui, "Semimajor axis", format_distance(o.semi_major_axis));
                        line(ui, "Eccentricity", format!("{:.4}", o.eccentricity));
                        line(ui, "Orbital period", format!("{:.3} d", o.period / 86_400.0));
                        if resonances.as_ref().is_none_or(|(r, _)| *r != sel) {
                            *resonances = Some((sel, orbital_resonances(cat, i as usize)));
                        }
                        if let Some((_, text)) = resonances.as_ref().filter(|(_, t)| !t.is_empty()) {
                            line(ui, "Orbital resonances", text.clone());
                        }
                        if let Some((hill, influence)) = sphere_radii(cat, b) {
                            line(ui, "Hill / influence sphere radius", format!("{} / {}", format_distance(hill), format_distance(influence)));
                        }
                    }
                    if b.mass_kg > 0.0 {
                        line(ui, "Roche limit for satellites", format_distance(se_core::orbit::roche_limit_for_satellites(b.mass_kg)));
                    }
                    if let Some((forms, biomes, origin)) = life_lines(&b.block, &i18n) {
                        line(ui, "Life forms", forms);
                        if !biomes.is_empty() {
                            line(ui, "Life biomes", biomes);
                        }
                        line(ui, "Life origin", origin);
                    }
                    if let Some(p) = b.rotation.period_seconds() {
                        line(ui, "Rotation period", format!("{:.3} h", p / 3600.0));
                    }
                    // Camera context: the ground under the camera, and SE's day lines there.
                    let up = eph.orientation(sel, sim.jd).inverse() * (-d);
                    let n = up / up.length().max(1e-9);
                    let lat = n.y.clamp(-1.0, 1.0).asin();
                    if r > 0.0 && dist < r * 20.0 {
                        // Above the body's ellipsoid (Earth's is 21 km flatter at the poles).
                        let s = b.shape();
                        let surface = r / ((n.x / s[0]).powi(2) + (n.y / s[1]).powi(2) + (n.z / s[2]).powi(2)).sqrt();
                        line(ui, "Height above surface", format_distance(dist - surface));
                        line(ui, "Lat/Lon", format!("{:+.4}° {:+.4}°", lat.to_degrees(), (-n.z).atan2(n.x).to_degrees()));
                    }
                    if let Some(l) = se_core::climate::display::day_lines_from(&eph, cat, i as usize, sim.jd, lat, &view) {
                        let span = |days: f64| if days < 2.0 { format!("{:.3} h", days * 24.0) } else { format!("{days:.3} d") };
                        line(ui, "Solar day", span(l.solar_day));
                        line(
                            ui,
                            "Daylight hours at equator/current latitude",
                            format!("{} / {}", span(l.daylight_equator as f64 * l.solar_day), span(l.daylight_here as f64 * l.solar_day)),
                        );
                        line(ui, "Annual fraction of polar day at current latitude", format!("{:.1} %", l.polar_day_percent));
                    }
                }
                ObjectRef::Galaxy(i) => {
                    let g = &cat.galaxies[i as usize];
                    line(ui, "Absolute magnitude", format!("{:.2}", g.abs_mag));
                    let dpc = (dist / METERS_PER_PC).max(1e-12);
                    line(ui, "Apparent magnitude", format!("{:.2}", g.abs_mag + 5.0 * (dpc / 10.0).log10()));
                }
                // Star clusters (02-012): magnitudes, luminosity, age and the generated members' count
                // (SE's "Number of stars", "Fly inside to count stars" before they exist, FUN_14038a450).
                ObjectRef::Cluster(i) => {
                    let c = &cat.clusters[i as usize];
                    let dpc = (dist / METERS_PER_PC).max(1e-12);
                    line(ui, "Absolute magnitude", format!("{:.2}", c.abs_mag));
                    line(ui, "Apparent magnitude", format!("{:.2}", c.abs_mag + 5.0 * (dpc / 10.0).log10()));
                    let lum = se_core::star::lum_from_abs_mag(c.abs_mag);
                    line(ui, "Luminosity", if lum >= 1e4 { format!("{lum:.3e} Sun") } else { format!("{lum:.1} Sun") });
                    line(ui, "Age", format!("{:.3} billion years", c.age_gyr));
                    match clusters.as_ref().and_then(|f| f.member_count(i as usize)).filter(|n| *n > 0) {
                        Some(n) => line(ui, "Number of stars", n.to_string()),
                        None if !c.no_members => {
                            ui.label(egui::RichText::new(i18n.tr("Fly inside to count stars")).size(13.0));
                        }
                        None => {}
                    }
                }
                _ => {}
            }
            if pos.to_meters().length() > 1e14 {
                let (ra, dec) = universe_to_radec(pos.to_meters());
                line(ui, "RA/Dec", format!("{ra:.4}h {dec:+.4}° (from the Sun)"));
                line(ui, "Distance from the Sun", format!("{:.4} pc", pos.to_meters().length() / METERS_PER_PC));
            }
            // SE's counts of what orbits the selection ("Number of planets", "... moons", "... stars"),
            // each opening to the list. Asteroids, comets and spacecraft are not counted, as in SE.
            if groups.as_ref().is_none_or(|(r, _)| *r != sel) {
                *groups = Some((sel, child_groups(cat, sel)));
            }
            let lists = &groups.as_ref().expect("set above").1;
            let mut hits: Vec<egui::Rect> = Vec::new();
            for ((key, group), members) in [
                ("Number of planets", ChildGroup::Planets),
                ("Number of dwarf planets", ChildGroup::DwarfPlanets),
                ("Number of moons", ChildGroup::Moons),
                ("Number of stars", ChildGroup::Stars),
            ]
            .into_iter()
            .zip(lists)
            {
                // A planet counts its moons; a star or a star system its planets and stars (moons seen
                // through the Earth-Moon or Pluto-Charon barycentre are not the Sun's).
                if (group == ChildGroup::Moons) != matches!(sel, ObjectRef::Body(_)) {
                    continue;
                }
                if members.is_empty() {
                    continue;
                }
                let r = egui::CollapsingHeader::new(egui::RichText::new(format!("{}: {}", i18n.tr(key), members.len())).size(13.0))
                    .id_salt(key)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                            for &c in members.iter().take(200) {
                                if ui.link(cat.display_name(c)).clicked() {
                                    cam.selected = Some(c);
                                }
                            }
                        });
                    });
                hits.push(r.header_response.rect);
                hits.extend(r.body_response.map(|b| b.rect));
            }
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(INFO_HITS), hits));
        });
    }

    if hud.show_places {
        let mut open = true;
        let mut jump: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut save_now = false;
        let scripted = crate::settings::scripted(&cfg);
        egui::Window::new(crate::skin::title(ctx, &i18n.tr("Places"))).open(&mut open).default_width(300.0).show(ctx, |ui| {
            ui.text_edit_singleline(&mut hud.places_filter);
            ui.horizontal(|ui| {
                ui.label(i18n.tr("Name"));
                ui.text_edit_singleline(&mut hud.place_name);
                let save = ui.add_enabled(!scripted, egui::Button::new(i18n.tr("Save current place")));
                let save = save.on_disabled_hover_text(i18n.tr("Scripted runs do not write places"));
                save_now = save.clicked();
            });
            let filter = hud.places_filter.to_lowercase();
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (i, p) in places.0.iter().enumerate() {
                    if !filter.is_empty() && !p.name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        if ui.link(&p.name).on_hover_text(format!("{} — {}", p.body, p.date)).clicked() {
                            jump = Some(i);
                        }
                        if p.user {
                            ui.weak(i18n.tr("yours"));
                            if ui.small_button(i18n.tr("Delete")).clicked() {
                                delete = Some(i);
                            }
                        }
                    });
                }
            });
        });
        hud.show_places = open;
        if let Some(i) = jump {
            let p = &places.0[i];
            cam.pos = se_core::upos::UPos::from_meters(bevy::math::DVec3::from_array(p.pos_m));
            cam.rot = bevy::math::DQuat::from_xyzw(p.rot[0], p.rot[1], p.rot[2], p.rot[3]).normalize();
            cam.follow = None;
            cam.travel = None;
            cam.velocity = bevy::math::DVec3::ZERO;
            cam.selected = cat.find(&p.body);
            sim.jd = p.jd;
            // SE's places carry the assigned velocity (pc/s) and the binding to their body.
            if p.vel > 0.0 {
                cam.speed = crate::camera::clamp_speed(p.vel * se_core::units::METERS_PER_PC);
            }
            if let (Some(b), 1..) = (cam.selected, p.bind_mode) {
                cam.bind(Some(b), p.bind_mode >= 2, &eph, sim.jd);
            }
        }
        if save_now {
            let typed = hud.place_name.trim().to_string();
            let name = if typed.is_empty() { format!("Place {}", places.0.len() + 1) } else { typed };
            // The body is the one the camera is bound to, else the selection (SE's Body).
            let body = cam.follow.map(|f| f.target).or(cam.selected).map(|r| cat.display_name(r).to_string()).unwrap_or_default();
            let rot = cam.rot.normalize();
            let mut p = se_core::places::Place::new_user(
                &name,
                &body,
                sim.jd,
                cam.pos.to_meters().to_array(),
                [rot.x, rot.y, rot.z, rot.w],
            );
            p.vel = cam.speed / se_core::units::METERS_PER_PC;
            p.bind_mode = cam.bind_mode() as i32;
            places.0.push(p);
            match crate::data::write_user_places(&cfg, &places.0) {
                Ok(()) => info!("saved place '{name}' to {}", crate::data::user_places_file().display()),
                Err(e) => warn!("cannot save places: {e}"),
            }
            hud.place_name.clear();
        }
        if let Some(i) = delete {
            let removed = places.0.remove(i);
            if let Err(e) = crate::data::write_user_places(&cfg, &places.0) {
                warn!("cannot save places after deleting '{}': {e}", removed.name);
            }
        }
    }

    if hud.show_help {
        egui::Window::new(crate::skin::title(ctx, &i18n.tr("Controls"))).show(ctx, |ui| {
            ui.label("Mouse drag: look around    Wheel or +/-: speed (Ctrl: presets)    Middle click: zoom    PgUp/PgDn: zoom    Home: reset zoom");
            ui.label("W/S A/D R/F (arrows, Numpad 1/0): move    Q/E: roll    Numpad 4/6/8/2: turn    Numpad 5: stop turning    Z: stop");
            ui.label("G: go to selection (again: fast)    Ctrl+G: go to its centre    Shift+G: land    C: center    End: level to horizon    T: track");
            ui.label("Shift+D / Shift+F / Shift+R: free / follow / rotate with selection    5: auto horizon    Backspace: previous selection    Shift+H: home");
            ui.label("Space: pause time    K/L: time slower/faster    J: reverse    \\: real time    Ctrl+\\: current time    Click the date to type one");
            ui.label("Ctrl+`: hide interface    F3: find object    Shift+F3: star browser    F4: settings    F7: locations    F8: chart    I: wiki");
            ui.label("O: orbits    ': labels (brief, detail, off)    Ctrl+': markers    ;: selection pointer");
            ui.label("X: constellations (Shift/Ctrl/Alt+X: lines/boundaries/names)    Ctrl+;: celestial grids    Alt+;: geographic grid");
            ui.label("F11: screenshot (Pictures/SpaceEngine-rs)    Shift+F11: high-resolution screenshot    M: music on/off    N: next track");
        });
    }
    Ok(())
}

/// An angle (radians) as SE writes apparent sizes: degrees, arcminutes or arcseconds.
fn format_angle(a: f64) -> String {
    let deg = a.to_degrees();
    if deg >= 1.0 {
        format!("{deg:.3}°")
    } else if deg * 60.0 >= 1.0 {
        format!("{:.3}'", deg * 60.0)
    } else {
        format!("{:.3}\"", deg * 3600.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click on the info panel's fold-out rows is the interface's, not a pick of the star behind.
    #[test]
    fn info_panel_rows_count_as_interface() {
        let ctx = egui::Context::default();
        let at = |x: f32, y: f32| egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(x, y))], ..Default::default() };
        let row = egui::Rect::from_min_size(egui::pos2(10.0, 240.0), egui::vec2(180.0, 18.0));
        let _ = ctx.run_ui(at(20.0, 248.0), |ui| {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(INFO_HITS), vec![row]));
        });
        assert!(pointer_over_ui(&ctx));
        let _ = ctx.run_ui(at(600.0, 400.0), |_| {});
        assert!(!pointer_over_ui(&ctx));
    }
}
