//! The Star browser's search (SE's Star browser with its "Filter settings", Shift+F3): scans the
//! stars within a radius, catalog and procedural, generating their planetary systems as a visit
//! would, and lists the systems with the planets and moons that pass the filter. As in SE the
//! scan stops at 10,000 systems, nearest first; a row's last column counts the matches, its
//! tooltip lists them, and clicking it selects them in turn. Searching for stars (black holes,
//! neutron stars, a spectral class...) skips the planets and lists the nearest matching stars.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use bevy_egui::egui;
use se_core::catalog::{BodyTag, Catalog, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::star::{LumClass, SpectralClass, StarKind};

use crate::camera::{FlyCamera, start_travel};
use crate::data::SeData;
use crate::i18n::I18n;
use crate::procstars::{ProcStarField, ProcStarInfo, ScanLevels, StarScan};
use crate::system::{materialize_star, proc_system_input, procedural_star_record};

/// SE's limit on the scanned systems.
pub const MAX_SYSTEMS: usize = 10_000;
const LY_PER_PC: f64 = 3.261_563_777;
const EARTH_RADIUS_KM: f64 = 6371.0;

const CLASSES: [&str; 9] = ["any", "Terra", "Aquaria", "Ferria", "Carbonia", "Selena", "Neptune", "Jupiter", "Chthonia"];
const TEMPS: [&str; 8] = ["any", "frigid", "cold", "cool", "temperate", "warm", "hot", "torrid"];
const STAR_TYPES: [&str; 13] =
    ["any", "Black hole", "Neutron star", "White dwarf", "Brown dwarf", "Giant", "O", "B", "A", "F", "G", "K", "M"];
const OBJECTS: [&str; 4] = ["Planet or moon", "Planet only", "Moon only", "Star"];

/// Does a star's class match entry `t` of [`STAR_TYPES`]?
fn star_type_ok(t: usize, c: &SpectralClass) -> bool {
    match t {
        0 => true,
        1 => c.kind == StarKind::BlackHole,
        2 => c.kind == StarKind::NeutronStar,
        3 => c.kind == StarKind::WhiteDwarf,
        4 => c.kind == StarKind::BrownDwarf,
        5 => c.kind == StarKind::Normal && c.lum_class <= LumClass::III,
        _ => c.kind == StarKind::Normal && c.letter == ['O', 'B', 'A', 'F', 'G', 'K', 'M'][t - 6],
    }
}

/// What a scan collects: systems with their planets, or the stars of one type.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ScanKind {
    Systems,
    Stars(usize),
}

/// A star of a listed system.
#[derive(Clone, Debug)]
struct StarFacts {
    name: String,
    class: SpectralClass,
}

/// What the filter knows of a planet or moon.
#[derive(Clone, Debug)]
struct BodyFacts {
    name: String,
    moon: bool,
    /// Bulk class ("Terra", ...).
    class: String,
    /// Descriptive class ("Temperate mesobaric marine terra with life") or the bulk class.
    desc: String,
    /// SE's temperature word, if known.
    temp: Option<&'static str>,
    radius_km: f64,
    atmosphere: bool,
    ocean: bool,
    /// Life class, type and biome.
    life: Option<(String, String, String)>,
}

#[derive(Clone, Debug)]
enum StarSource {
    Catalog,
    Procedural(ProcStarInfo),
}

#[derive(Clone, Debug)]
struct SystemRow {
    name: String,
    pos_pc: DVec3,
    dist_pc: f64,
    spectral: String,
    source: StarSource,
    bodies: Vec<BodyFacts>,
    stars: Vec<StarFacts>,
}

/// A running or finished scan, shared with its worker.
#[derive(Default)]
struct Scan {
    rows: Mutex<Vec<SystemRow>>,
    total: AtomicUsize,
    scanned: AtomicUsize,
    cancel: AtomicBool,
    done: AtomicBool,
}

/// The filter (SE's "Filter settings"; the choices are indices into the lists shown).
#[derive(Clone, PartialEq, Default)]
struct Filter {
    /// Index into [`OBJECTS`].
    object: usize,
    /// Index into [`STAR_TYPES`] (object "Star").
    star: usize,
    class: usize,
    temp: usize,
    /// 0 any, 1 non-arid (with an ocean), 2 arid.
    water: usize,
    /// 0 any, 1 with, 2 without.
    atmosphere: usize,
    /// 0 any, 1 any life, 2 organic, 3 exotic, 4 lifeless.
    life: usize,
    /// 0 any, 1 unicellular, 2 multicellular.
    complexity: usize,
    /// 0 any, 1 marine, 2 terrestrial.
    biome: usize,
    /// 0 all, 1 catalog systems only, 2 procedural only.
    systems: usize,
    /// Radius limits in Earth radii (0: none).
    radius_min: f64,
    radius_max: f64,
}

impl Filter {
    fn is_default(&self) -> bool {
        *self == Filter::default()
    }

    fn kind(&self) -> ScanKind {
        if self.object == 3 { ScanKind::Stars(self.star) } else { ScanKind::Systems }
    }

    /// Indices of a row's matching stars (star search) or planets and moons.
    fn row_matches(&self, r: &SystemRow) -> Vec<usize> {
        if !self.system_ok(r) {
            Vec::new()
        } else if self.object == 3 {
            (0..r.stars.len()).filter(|&k| star_type_ok(self.star, &r.stars[k].class)).collect()
        } else {
            (0..r.bodies.len()).filter(|&k| self.body_ok(&r.bodies[k])).collect()
        }
    }

    fn system_ok(&self, r: &SystemRow) -> bool {
        match self.systems {
            1 => matches!(r.source, StarSource::Catalog),
            2 => matches!(r.source, StarSource::Procedural(_)),
            _ => true,
        }
    }

    fn body_ok(&self, b: &BodyFacts) -> bool {
        let pick = |i: usize, ok: &dyn Fn(usize) -> bool| i == 0 || ok(i);
        let life = b.life.as_ref();
        let has = |s: &Option<&(String, String, String)>, f: &dyn Fn(&(String, String, String)) -> bool| s.is_some_and(f);
        let re = b.radius_km / EARTH_RADIUS_KM;
        pick(self.object, &|i| (i == 2) == b.moon)
            && pick(self.class, &|i| b.class.eq_ignore_ascii_case(CLASSES[i]))
            && pick(self.temp, &|i| b.temp == Some(TEMPS[i]))
            && pick(self.water, &|i| (i == 1) == b.ocean)
            && pick(self.atmosphere, &|i| (i == 1) == b.atmosphere)
            && pick(self.life, &|i| match i {
                1 => life.is_some(),
                2 => has(&life, &|l| l.0.eq_ignore_ascii_case("organic")),
                3 => has(&life, &|l| l.0.eq_ignore_ascii_case("exotic")),
                _ => life.is_none(),
            })
            && pick(self.complexity, &|i| {
                has(&life, &|l| l.1.to_ascii_lowercase().starts_with(if i == 1 { "uni" } else { "multi" }))
            })
            && pick(self.biome, &|i| has(&life, &|l| l.2.to_ascii_lowercase().contains(if i == 1 { "marine" } else { "terrestrial" })))
            && (self.radius_min <= 0.0 || re >= self.radius_min)
            && (self.radius_max <= 0.0 || re <= self.radius_max)
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum Column {
    #[default]
    Distance,
    Name,
    Class,
    Bodies,
    Matches,
}

/// The search window's state.
#[derive(Resource)]
pub struct SearchState {
    radius_ly: f64,
    filter: Filter,
    /// The filter being edited in the settings (applied with Ok).
    editing: Option<Filter>,
    hide_empty: bool,
    sort: Column,
    descending: bool,
    scan: Option<Arc<Scan>>,
    /// What the last scan collected, and when it started.
    scan_kind: ScanKind,
    started: Option<std::time::Instant>,
    rows: Vec<SystemRow>,
    /// Indices into each row's bodies that pass the filter.
    matches: Vec<Vec<usize>>,
    /// Row order as shown.
    order: Vec<usize>,
    dirty: bool,
    /// Per row, the next match a click selects.
    cycle: Vec<usize>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            radius_ly: 50.0,
            filter: Filter::default(),
            editing: None,
            hide_empty: false,
            sort: Column::Distance,
            descending: false,
            scan: None,
            scan_kind: ScanKind::Systems,
            started: None,
            rows: Vec::new(),
            matches: Vec::new(),
            order: Vec::new(),
            dirty: false,
            cycle: Vec::new(),
        }
    }
}

/// A body to select once its star is in the catalog (a procedural system is generated first).
#[derive(Resource, Default)]
pub struct PendingSelect(Option<(SystemRow, String, bool)>);

pub struct StarSearchPlugin;

impl Plugin for StarSearchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SearchState>().init_resource::<PendingSelect>().add_systems(Update, apply_pending);
    }
}

fn temp_word(t: &str) -> Option<&'static str> {
    TEMPS[1..].iter().copied().find(|w| w.eq_ignore_ascii_case(t))
}

/// Facts of a catalog system's planets and moons; temperatures as the Selection panel has them.
fn catalog_bodies(cat: &Catalog, eph: &Ephemeris, root: u32, jd: f64) -> Vec<BodyFacts> {
    let mut out = Vec::new();
    let lit = se_core::climate::system_stars(cat, root);
    for m in cat.system_members(&cat.stars[root as usize].names[0]) {
        let ObjectRef::Body(i) = m else { continue };
        let b = &cat.bodies[i as usize];
        let moon = match b.tag {
            BodyTag::Planet | BodyTag::DwarfPlanet => false,
            BodyTag::Moon | BodyTag::DwarfMoon => true,
            _ => continue,
        };
        let light = se_core::climate::star_light_from(eph, cat, m, jd, &lit);
        let completion = cat
            .climate
            .as_ref()
            .and_then(|t| se_core::climate::complete(t, &se_core::climate::catalog_body_in(cat, i as usize, &light)?));
        let model = se_core::climate::display::catalog_model(cat, i as usize, &light, completion.as_ref());
        let t = model.panel(&se_core::climate::display::catalog_view(eph, cat, i as usize, jd)).average;
        let life = se_core::body_params::LifeParams::all_from_body(&b.block).into_iter().next().map(|l| {
            (l.class.unwrap_or_default(), l.kind.unwrap_or_default(), l.biome.unwrap_or_default())
        });
        let class = b.class.clone().unwrap_or_default();
        out.push(BodyFacts {
            name: b.names.first().cloned().unwrap_or_default(),
            moon,
            desc: class.clone(),
            class,
            temp: (t.is_finite() && t > 0.0).then(|| se_core::procgen::temperature_class(t as f64)),
            radius_km: b.radius_km,
            atmosphere: b.block.sub("Atmosphere").is_some(),
            ocean: b.block.sub("Ocean").is_some(),
            life,
        });
    }
    out
}

/// The stars of a catalog system (its root and every star below it).
fn catalog_stars(cat: &Catalog, root: u32) -> Vec<StarFacts> {
    let r = &cat.stars[root as usize];
    std::iter::once(ObjectRef::Star(root))
        .chain(cat.system_members(&r.names[0]))
        .filter_map(|m| match m {
            ObjectRef::Star(i) if !cat.stars[i as usize].barycenter => {
                let s = &cat.stars[i as usize];
                Some(StarFacts { name: s.names[0].clone(), class: s.class.clone() })
            }
            _ => None,
        })
        .collect()
}

/// Facts of a procedural star's system, generated as on arrival: its bodies and its stars (SE's
/// components of a multiple system, else the star itself).
fn procedural_system(info: &ProcStarInfo, pos_pc: DVec3, tables: Option<&Arc<se_core::climate::ClimateTables>>) -> (Vec<BodyFacts>, Vec<StarFacts>) {
    let mut rec = procedural_star_record(info, pos_pc);
    let (params, seed) = proc_system_input(&rec);
    let g = se_core::procgen::generate_system_for_search(&params, seed);
    // SE's life, as on arrival: its life generator on the completed system (system.rs `generate`).
    let mut lives = std::collections::HashMap::new();
    if let Some(t) = tables {
        rec.barycenter = g.root_mass_sol.is_some();
        let mut doc = se_core::sc::parse(&g.sc);
        se_core::procgen::apply_life_to_doc(&[rec], t.clone(), g.root_mass_sol, &mut doc, se_core::time::J2000);
        for it in &doc.items {
            if let (Some(name), Some(l)) = (it.name(), it.block().and_then(|b| b.sub("Life"))) {
                let s = |k: &str| l.str(k).unwrap_or_default().to_string();
                lives.insert(name.to_string(), (s("Class"), s("Type"), s("Biome")));
            }
        }
    }
    let stars = if g.stars.is_empty() {
        vec![StarFacts { name: info.name.clone(), class: SpectralClass::parse(&info.class) }]
    } else {
        g.stars.iter().map(|(n, c)| StarFacts { name: n.clone(), class: SpectralClass::parse(c) }).collect()
    };
    let bodies = g
        .bodies
        .into_iter()
        .filter_map(|b| {
            let moon = match b.tag {
                "Planet" | "DwarfPlanet" => false,
                "Moon" | "DwarfMoon" => true,
                _ => return None,
            };
            let life = lives.remove(&b.name);
            Some(BodyFacts {
                temp: b.class_desc.split_whitespace().next().and_then(temp_word),
                moon,
                class: b.class,
                desc: if life.is_some() { format!("{} with life", b.class_desc) } else { b.class_desc },
                name: b.name,
                radius_km: b.radius_km,
                atmosphere: b.has_atmosphere,
                ocean: b.has_ocean,
                life,
            })
        })
        .collect();
    (bodies, stars)
}

fn run_scan(cat: Arc<Catalog>, stars: Option<StarScan>, center: DVec3, radius_pc: f64, jd: f64, kind: ScanKind, scan: Arc<Scan>) {
    if let ScanKind::Stars(t) = kind {
        return scan_stars(cat, stars, center, radius_pc, t, scan);
    }
    // Catalog systems: the roots of their trees (procedural stars that became catalog entries
    // come again from the field).
    let mut cands: Vec<(f64, Option<u32>, Option<(DVec3, ProcStarInfo)>)> = cat
        .stars
        .iter()
        .enumerate()
        .filter(|(_, s)| s.has_position && s.parent.is_none() && s.source != "procedural")
        .map(|(i, s)| (s.pos_pc.distance(center), Some(i as u32), None))
        .filter(|c| c.0 <= radius_pc)
        .collect();
    if let Some(st) = stars.as_ref() {
        let near = st.stars_within(center, radius_pc, MAX_SYSTEMS, ScanLevels::All, &|_| true);
        cands.extend(near.into_iter().map(|(p, info)| (p.distance(center), None, Some((p, info)))));
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    cands.truncate(MAX_SYSTEMS);
    scan.total.store(cands.len(), Ordering::Relaxed);
    let push = |row: SystemRow| {
        if let Ok(mut v) = scan.rows.lock() {
            v.push(row);
        }
        scan.scanned.fetch_add(1, Ordering::Relaxed);
    };
    // Catalog systems first, so the catalog is released early (new procedural stars wait for it).
    {
        let eph = Ephemeris::new(&cat);
        for (d, i, _) in cands.iter().filter(|c| c.1.is_some()) {
            if scan.cancel.load(Ordering::Relaxed) {
                return;
            }
            let i = i.unwrap_or(0);
            let s = &cat.stars[i as usize];
            push(SystemRow {
                name: s.names[0].clone(),
                pos_pc: s.pos_pc,
                dist_pc: *d,
                spectral: s.class.raw.clone(),
                source: StarSource::Catalog,
                bodies: catalog_bodies(&cat, &eph, i, jd),
                stars: catalog_stars(&cat, i),
            });
        }
    }
    let tables = cat.climate.clone();
    drop(cat);
    for (d, _, p) in cands.into_iter() {
        let Some((pos, info)) = p else { continue };
        if scan.cancel.load(Ordering::Relaxed) {
            return;
        }
        let (bodies, stars) = procedural_system(&info, pos, tables.as_ref());
        push(SystemRow {
            name: info.name.clone(),
            pos_pc: pos,
            dist_pc: d,
            spectral: info.class.clone(),
            bodies,
            stars,
            source: StarSource::Procedural(info),
        });
    }
    scan.done.store(true, Ordering::Relaxed);
}

/// A star search: one row per matching star, catalog and procedural, the nearest
/// [`MAX_SYSTEMS`]; no planetary systems are generated.
fn scan_stars(cat: Arc<Catalog>, stars: Option<StarScan>, center: DVec3, radius_pc: f64, t: usize, scan: Arc<Scan>) {
    let started = std::time::Instant::now();
    let mut rows: Vec<SystemRow> = cat
        .stars
        .iter()
        .filter(|s| s.has_position && !s.barycenter && s.source != "procedural" && star_type_ok(t, &s.class))
        .map(|s| (s, s.pos_pc.distance(center)))
        .filter(|(_, d)| *d <= radius_pc)
        .map(|(s, d)| SystemRow {
            name: s.names[0].clone(),
            pos_pc: s.pos_pc,
            dist_pc: d,
            spectral: s.class.raw.clone(),
            source: StarSource::Catalog,
            bodies: Vec::new(),
            stars: vec![StarFacts { name: s.names[0].clone(), class: s.class.clone() }],
        })
        .collect();
    drop(cat);
    if let Some(st) = stars.as_ref() {
        let keep = |s: &se_core::star_octree::ProcStar| match t {
            1 => s.kind == StarKind::BlackHole,
            2 => s.kind == StarKind::NeutronStar,
            3 => s.kind == StarKind::WhiteDwarf,
            _ => star_type_ok(t, &SpectralClass::parse(&s.class)),
        };
        // Only the levels whose magnitudes can hold the type (the field's dead stars have a level
        // of their own; giants and O-A stars are bright, so their nodes are few and large).
        let inf = f64::INFINITY;
        let levels = match t {
            1 | 2 => ScanLevels::Remnants,
            3 => ScanLevels::Mags(10.0, inf),
            4 => ScanLevels::Mags(inf, inf),
            5 => ScanLevels::Mags(-inf, 1.0),
            6 => ScanLevels::Mags(-inf, -2.5),
            7 => ScanLevels::Mags(-inf, 2.0),
            8 => ScanLevels::Mags(0.0, 3.5),
            9 => ScanLevels::Mags(1.5, 5.5),
            10 => ScanLevels::Mags(-inf, 7.0),
            11 => ScanLevels::Mags(-inf, 10.5),
            _ => ScanLevels::All,
        };
        rows.extend(st.stars_within(center, radius_pc, MAX_SYSTEMS, levels, &keep).into_iter().map(|(p, info)| SystemRow {
            name: info.name.clone(),
            pos_pc: p,
            dist_pc: p.distance(center),
            spectral: info.class.clone(),
            bodies: Vec::new(),
            stars: vec![StarFacts { name: info.name.clone(), class: SpectralClass::parse(&info.class) }],
            source: StarSource::Procedural(info),
        }));
    }
    rows.sort_by(|a, b| a.dist_pc.total_cmp(&b.dist_pc));
    rows.truncate(MAX_SYSTEMS);
    info!(
        "star search ({}) within {radius_pc:.0} pc: {} found in {:.1} s, nearest {:?}",
        STAR_TYPES[t],
        rows.len(),
        started.elapsed().as_secs_f64(),
        rows.first().map(|r| (&r.name, r.dist_pc))
    );
    scan.total.store(rows.len(), Ordering::Relaxed);
    scan.scanned.store(rows.len(), Ordering::Relaxed);
    if let Ok(mut v) = scan.rows.lock() {
        *v = rows;
    }
    scan.done.store(true, Ordering::Relaxed);
}

impl SearchState {
    /// Dev command `search LY [life|bh]`: search within LY light years, for life or black holes.
    pub fn dev_search(&mut self, data: &SeData, cam: &FlyCamera, field: Option<&ProcStarField>, jd: f64, radius_ly: f64, what: &str) {
        self.radius_ly = radius_ly;
        self.filter = Filter::default();
        match what {
            "life" => self.filter.life = 1,
            "bh" => (self.filter.object, self.filter.star) = (3, 1),
            _ => {}
        }
        self.hide_empty = what == "life";
        self.start(data, cam, field, jd);
    }

    fn start(&mut self, data: &SeData, cam: &FlyCamera, field: Option<&ProcStarField>, jd: f64) {
        if let Some(s) = self.scan.take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
        let scan = Arc::new(Scan::default());
        self.scan = Some(scan.clone());
        // The table drawn after this call still indexes the rows through `order`.
        self.rows.clear();
        self.matches.clear();
        self.cycle.clear();
        self.order.clear();
        self.dirty = true;
        let cat = data.catalog.clone();
        let stars = field.filter(|f| f.enabled).map(|f| f.scan());
        let center = cam.pos.to_parsecs();
        let radius_pc = self.radius_ly.max(0.1) / LY_PER_PC;
        let kind = self.filter.kind();
        self.scan_kind = kind;
        self.started = Some(std::time::Instant::now());
        AsyncComputeTaskPool::get().spawn(async move { run_scan(cat, stars, center, radius_pc, jd, kind, scan) }).detach();
    }

    /// Take the worker's new rows; re-filter and re-sort when needed.
    fn update(&mut self) {
        if let Some(scan) = &self.scan {
            if let Ok(mut v) = scan.rows.lock() {
                if !v.is_empty() {
                    let f = &self.filter;
                    for r in v.drain(..) {
                        self.matches.push(f.row_matches(&r));
                        self.cycle.push(0);
                        self.rows.push(r);
                    }
                    self.dirty = true;
                }
            }
        }
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let filtering = !self.filter.is_default();
        self.order = (0..self.rows.len()).filter(|&i| !(self.hide_empty && filtering && self.matches[i].is_empty())).collect();
        let (rows, matches) = (&self.rows, &self.matches);
        let key = |i: &usize| -> (f64, String) {
            let r = &rows[*i];
            match self.sort {
                Column::Distance => (r.dist_pc, String::new()),
                Column::Name => (0.0, r.name.clone()),
                Column::Class => (0.0, r.spectral.clone()),
                Column::Bodies => (r.bodies.len() as f64, String::new()),
                Column::Matches => (matches[*i].len() as f64, String::new()),
            }
        };
        self.order.sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            let o = ka.0.total_cmp(&kb.0).then_with(|| ka.1.cmp(&kb.1));
            if self.descending { o.reverse() } else { o }
        });
    }

    fn refilter(&mut self) {
        let f = &self.filter;
        self.matches = self.rows.iter().map(|r| f.row_matches(r)).collect();
        self.dirty = true;
    }
}

fn combo(ui: &mut egui::Ui, i18n: &I18n, label: &str, value: &mut usize, items: &[&str]) {
    ui.label(i18n.tr(label));
    egui::ComboBox::from_id_salt(label).selected_text(i18n.tr(items[*value])).show_ui(ui, |ui| {
        for (k, it) in items.iter().enumerate() {
            ui.selectable_value(value, k, i18n.tr(it));
        }
    });
    ui.end_row();
}

/// The search tab of the Star browser.
#[allow(clippy::too_many_arguments)]
pub fn search_ui(
    ui: &mut egui::Ui,
    st: &mut SearchState,
    data: &SeData,
    cam: &mut FlyCamera,
    field: Option<&ProcStarField>,
    jd: f64,
    i18n: &I18n,
    pending: &mut PendingSelect,
) {
    st.update();
    ui.horizontal(|ui| {
        ui.label(i18n.tr("Search radius"));
        ui.add(egui::DragValue::new(&mut st.radius_ly).range(1.0..=1000.0).speed(1.0).suffix(" ly"));
        if ui.button(i18n.tr("Search")).clicked() {
            st.start(data, cam, field, jd);
        }
        if ui.button(i18n.tr("Filter settings")).clicked() {
            st.editing = Some(st.filter.clone());
        }
        if ui.checkbox(&mut st.hide_empty, i18n.tr("Filter")).changed() {
            st.dirty = true;
        }
    });
    if let Some(scan) = &st.scan {
        let found = st.matches.iter().filter(|m| !m.is_empty()).count();
        let (n, total) = (scan.scanned.load(Ordering::Relaxed), scan.total.load(Ordering::Relaxed));
        let busy = match st.started {
            Some(t) if !scan.done.load(Ordering::Relaxed) => format!(" ... {:.0} s", t.elapsed().as_secs_f64()),
            _ => String::new(),
        };
        let found = if st.filter.is_default() { String::new() } else { format!("{found} / ") };
        ui.label(format!("{found}{n} / {total}{busy}"));
        if st.filter.kind() != st.scan_kind {
            ui.label(i18n.tr("Press Search to scan for this object type"));
        }
    }
    filter_window(ui.ctx(), st, i18n);
    ui.separator();
    let header = |ui: &mut egui::Ui, st: &mut SearchState, col: Column, text: &str| {
        let mark = if st.sort == col { if st.descending { " v" } else { " ^" } } else { "" };
        if ui.button(format!("{}{mark}", i18n.tr(text))).clicked() {
            st.descending = if st.sort == col { !st.descending } else { false };
            st.sort = col;
            st.dirty = true;
        }
    };
    egui::Grid::new("search_head").num_columns(5).min_col_width(70.0).show(ui, |ui| {
        header(ui, st, Column::Name, "Name");
        header(ui, st, Column::Distance, "Distance");
        header(ui, st, Column::Class, "Class");
        header(ui, st, Column::Bodies, "Planets");
        header(ui, st, Column::Matches, "Filter");
        ui.end_row();
    });
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let n = st.order.len();
    // (row, match to select): applied after the table, which borrows the rows.
    let mut clicked: Option<(usize, Option<usize>)> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, n, |ui, range| {
        egui::Grid::new("search_rows").striped(true).num_columns(5).min_col_width(70.0).show(ui, |ui| {
            for k in range {
                let i = st.order[k];
                let r = &st.rows[i];
                let selected_here = cam.selected.is_some_and(|s| data.catalog.display_name(s) == r.name);
                if ui.selectable_label(selected_here, r.name.as_str()).clicked() {
                    clicked = Some((i, None));
                }
                ui.label(format!("{:.2} ly", r.dist_pc * LY_PER_PC));
                ui.label(r.spectral.as_str());
                if st.scan_kind == ScanKind::Systems {
                    ui.label(r.bodies.iter().filter(|b| !b.moon).count().to_string());
                } else {
                    ui.label("-");
                }
                let m = &st.matches[i];
                let resp = ui.selectable_label(false, m.len().to_string());
                if !m.is_empty() {
                    let resp = resp.on_hover_ui(|ui| {
                        for &k in m.iter().take(20) {
                            let (name, desc) = match st.filter.object {
                                3 => (&r.stars[k].name, &r.stars[k].class.raw),
                                _ => (&r.bodies[k].name, &r.bodies[k].desc),
                            };
                            ui.label(format!("{name} ({desc})"));
                        }
                    });
                    if resp.clicked() {
                        clicked = Some((i, Some(st.cycle[i] % m.len())));
                    }
                }
                ui.end_row();
            }
        });
    });
    match clicked {
        Some((i, None)) => pending.0 = Some((st.rows[i].clone(), st.rows[i].name.clone(), false)),
        Some((i, Some(c))) => {
            st.cycle[i] = c + 1;
            let k = st.matches[i][c];
            let body = if st.filter.object == 3 { st.rows[i].stars[k].name.clone() } else { st.rows[i].bodies[k].name.clone() };
            pending.0 = Some((st.rows[i].clone(), body, false));
        }
        None => {}
    }
    ui.separator();
    if ui.button(i18n.tr("Go to")).clicked() {
        if let Some(t) = cam.selected {
            start_travel(cam, t, None);
        }
    }
}

fn filter_window(ctx: &egui::Context, st: &mut SearchState, i18n: &I18n) {
    let Some(f) = st.editing.as_mut() else { return };
    let mut close = None;
    egui::Window::new(crate::skin::title(ctx, &i18n.tr("Filter settings"))).collapsible(false).resizable(false).show(ctx, |ui| {
        egui::Grid::new("filter").num_columns(2).show(ui, |ui| {
            combo(ui, i18n, "Object to search", &mut f.object, &OBJECTS);
            combo(ui, i18n, "Systems", &mut f.systems, &["All systems", "Catalog systems only", "Procedural systems only"]);
            if f.object == 3 {
                combo(ui, i18n, "Star type", &mut f.star, &STAR_TYPES);
                return;
            }
            combo(ui, i18n, "Class", &mut f.class, &CLASSES);
            combo(ui, i18n, "Temperature", &mut f.temp, &TEMPS);
            combo(ui, i18n, "Water", &mut f.water, &["any", "non-arid", "arid"]);
            combo(ui, i18n, "Atmosphere", &mut f.atmosphere, &["any", "yes", "no"]);
            combo(ui, i18n, "Life", &mut f.life, &["any", "any life", "Organic", "Exotic", "no life"]);
            combo(ui, i18n, "Life type", &mut f.complexity, &["any", "Unicellular", "Multicellular"]);
            combo(ui, i18n, "Biome", &mut f.biome, &["any", "Marine", "Terrestrial"]);
            ui.label(i18n.tr("Radius (Earth radii)"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut f.radius_min).range(0.0..=100.0).speed(0.05));
                ui.label("-");
                ui.add(egui::DragValue::new(&mut f.radius_max).range(0.0..=100.0).speed(0.05));
            });
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui.button(i18n.tr("Ok")).clicked() {
                close = Some(true);
            }
            if ui.button(i18n.tr("Cancel")).clicked() {
                close = Some(false);
            }
            if ui.button(i18n.tr("Reset to default")).clicked() {
                *f = Filter::default();
            }
        });
    });
    match close {
        Some(true) => {
            st.filter = st.editing.take().unwrap_or_default();
            st.refilter();
        }
        Some(false) => st.editing = None,
        None => {}
    }
}

/// Select (and with `go`, travel to) a search result; a procedural system is generated into
/// the catalog first, in a frame where nothing else holds the catalog.
fn apply_pending(mut pending: ResMut<PendingSelect>, mut data: ResMut<SeData>, mut cam: ResMut<FlyCamera>) {
    let Some((row, body, go)) = pending.0.as_ref() else { return };
    if data.catalog.find(&row.name).is_none() {
        let StarSource::Procedural(info) = &row.source else {
            pending.0 = None;
            return;
        };
        let Some(cat) = Arc::get_mut(&mut data.catalog) else { return };
        materialize_star(cat, procedural_star_record(info, row.pos_pc));
    }
    if let Some(r) = data.catalog.find(body) {
        cam.selected = Some(r);
        if *go {
            start_travel(&mut cam, r, None);
        }
    }
    pending.0 = None;
}
