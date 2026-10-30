//! Navigation overlay: orbit lines, object labels and markers, and click-to-select. What is drawn
//! follows SE's overlay filters (main-def.cfg `ShowOrbits*`, `LabelsMode`, `ShowLabels*`,
//! `ShowMarkers*`), in the interface skin's colours (`Info*Color`, `BodySelectedOrbitColor`).
//! Also the switches, keys, fades and line drawing shared by the sky overlays
//! ([`crate::constellations`], [`crate::grids`]).

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{BodyTag, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::proc_galaxy::ProcGalaxy;
use se_core::units::METERS_PER_PC;

use crate::camera::{CommandQueue, FlyCamera, MainCamera};
use crate::clusters::ClusterField;
use crate::data::SeData;
use crate::galaxies::{GalaxySettings, GalaxyState, far_mag_limits, profile_type};
use crate::procgalaxies::{ProcGalaxyField, promote_galaxy};
use crate::procstars::{ProcStarField, ProcStarInfo};
use crate::stars::StarSettings;
use crate::system::{ActiveSystem, SystemBody, materialize_star, procedural_star_record};
use crate::universe::{ScaledSpace, SimTime, UniverseSet};

/// SE's object kinds in the overlay filters and the skin's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkerKind {
    Ship,
    Planet,
    Moon,
    DwarfPlanet,
    DwarfMoon,
    Asteroid,
    Comet,
    /// A star of the system around the camera.
    Sun,
    Barycenter,
    /// A catalog star seen from outside its system.
    Star,
    Cluster,
    Nebula,
    Galaxy,
}

impl MarkerKind {
    pub const ALL: [Self; 13] = [
        Self::Ship,
        Self::Planet,
        Self::Moon,
        Self::DwarfPlanet,
        Self::DwarfMoon,
        Self::Asteroid,
        Self::Comet,
        Self::Sun,
        Self::Barycenter,
        Self::Star,
        Self::Cluster,
        Self::Nebula,
        Self::Galaxy,
    ];

    /// A catalog object's kind (stars are [`MarkerKind::Star`] here; the overlay draws the system's
    /// own stars as [`MarkerKind::Sun`]).
    pub fn of(data: &SeData, obj: ObjectRef) -> Self {
        match obj {
            ObjectRef::Star(i) if data.catalog.stars[i as usize].barycenter => Self::Barycenter,
            ObjectRef::Star(_) => Self::Star,
            ObjectRef::Body(i) => match data.catalog.bodies[i as usize].tag {
                BodyTag::Planet => Self::Planet,
                BodyTag::DwarfPlanet => Self::DwarfPlanet,
                BodyTag::Moon => Self::Moon,
                BodyTag::DwarfMoon => Self::DwarfMoon,
                BodyTag::Asteroid => Self::Asteroid,
                BodyTag::Comet => Self::Comet,
                BodyTag::Spacecraft => Self::Ship,
            },
            ObjectRef::Galaxy(_) => Self::Galaxy,
            ObjectRef::Cluster(_) => Self::Cluster,
            ObjectRef::Nebula(_) => Self::Nebula,
        }
    }

    /// The filter keys' name (`ShowLabels<name>`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Ship => "Ships",
            Self::Planet => "Planets",
            Self::Moon => "Moons",
            Self::DwarfPlanet => "DwarfPlanets",
            Self::DwarfMoon => "DwarfMoons",
            Self::Asteroid => "Asteroids",
            Self::Comet => "Comets",
            Self::Sun => "Suns",
            Self::Barycenter => "Barycenters",
            Self::Star => "Stars",
            Self::Cluster => "Clusters",
            Self::Nebula => "Nebulae",
            Self::Galaxy => "Galaxies",
        }
    }

    /// The skin's colour key (`Info<key>Color`).
    pub fn skin_key(self) -> &'static str {
        match self {
            Self::Ship => "Spacecraft",
            Self::Planet => "Planet",
            Self::Moon => "Moon",
            Self::DwarfPlanet => "DwarfPlanet",
            Self::DwarfMoon => "DwarfMoon",
            Self::Asteroid => "Asteroid",
            Self::Comet => "Comet",
            Self::Sun => "Sun",
            Self::Barycenter => "StarBary",
            Self::Star => "Star",
            Self::Cluster => "Cluster",
            Self::Nebula => "Nebula",
            Self::Galaxy => "Galaxy",
        }
    }

    /// SE's "major bodies", whose orbits are `OrbitMajorThickness` wide; the minor ones (dwarf planets
    /// and moons, asteroids, comets) get `OrbitMinorThickness`. Assumed from the settings' names
    /// "Thickness (major bodies)" and "Thickness (minor bodies)".
    fn major(self) -> bool {
        matches!(self, Self::Ship | Self::Planet | Self::Moon | Self::Sun | Self::Barycenter)
    }

    /// Labelled in the brief mode even when small on screen.
    fn prominent(self) -> bool {
        matches!(self, Self::Planet | Self::DwarfPlanet | Self::Moon | Self::Sun | Self::Barycenter)
    }
}

/// A set of [`MarkerKind`]s: one overlay filter's per-kind switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindSet(u16);

impl KindSet {
    const fn of(kinds: &[MarkerKind]) -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < kinds.len() {
            bits |= 1 << kinds[i] as u16;
            i += 1;
        }
        Self(bits)
    }

    pub fn has(self, k: MarkerKind) -> bool {
        self.0 & (1 << k as u16) != 0
    }

    pub fn set(&mut self, k: MarkerKind, on: bool) {
        if on {
            self.0 |= 1 << k as u16;
        } else {
            self.0 &= !(1 << k as u16);
        }
    }
}

/// A per-kind filter by SE's key name (`showorbitsplanets`, `showlabelsstars`, ..., lower case).
pub fn kind_filter<'a>(settings: &'a mut OverlaySettings, key: &str) -> Option<(&'a mut KindSet, MarkerKind)> {
    let (set, rest) = if let Some(r) = key.strip_prefix("showorbits") {
        (&mut settings.orbit_kinds, r)
    } else if let Some(r) = key.strip_prefix("showlabels") {
        (&mut settings.label_kinds, r)
    } else {
        (&mut settings.marker_kinds, key.strip_prefix("showmarkers")?)
    };
    let kind = MarkerKind::ALL.into_iter().find(|k| k.key().eq_ignore_ascii_case(rest))?;
    Some((set, kind))
}

#[derive(Resource, Debug, Clone)]
pub struct OverlaySettings {
    /// SE's `ShowOrbits` (key O), and per kind `ShowOrbits<Kinds>`.
    pub orbits: bool,
    pub orbit_kinds: KindSet,
    /// SE's `LabelsMode` (key ' cycles it): 0 off, 1 brief, 2 detail; per kind `ShowLabels<Kinds>`.
    pub labels_mode: u8,
    pub label_kinds: KindSet,
    /// SE's `ShowMarkers` (Ctrl+'), per kind `ShowMarkers<Kinds>`.
    pub markers: bool,
    pub marker_kinds: KindSet,
    /// Stars brighter than this apparent magnitude get labels.
    pub star_label_mag: f32,
    /// SE's "Selection pointer" (Filter objects, key ;).
    pub selection_pointer: bool,
    /// The frame-rate readout in the top bar (SE's `FPS`).
    pub show_fps: bool,
    /// The selection's full info table (SE's `TextMode` 2), else the brief one (1).
    pub info_full: bool,
    /// Constellations and coordinate grids.
    pub sky: SkySwitches,
}

/// SE's sky overlay switches: OverlayFilters `ShowConstellations` (master), `ShowConstelLines`,
/// `ShowConstelBoundaries`, `ShowConstelLabels`, `ShowConstelDrawings`, `ShowGrids` (master of the
/// celestial grids), `ShowGridEquatorial/Ecliptic/Galactic/Geographic`, Interface
/// `FullConstellationNames` and `ConstelBordersAtInf`; and the per-constellation switches behind the
/// script variables `ConstLines<Abbr>`, `ConstBoundary<Abbr>`, `ConstLabel<Abbr>`, `ConstLinesAll`,
/// `ConstLinesZodiac`, ... (FUN_140263e60, FUN_140260970).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkySwitches {
    pub constellations: bool,
    pub constel_lines: bool,
    pub constel_boundaries: bool,
    pub constel_labels: bool,
    /// SE 0.991 ships no constellation art, so this draws nothing.
    pub constel_drawings: bool,
    pub full_names: bool,
    pub borders_at_inf: bool,
    pub grids: bool,
    pub grid_equatorial: bool,
    pub grid_ecliptic: bool,
    pub grid_galactic: bool,
    pub grid_geographic: bool,
    /// Per constellation (bit = index in [`crate::constellations::IAU`]): lines, boundaries, labels.
    pub per_constel: [u128; 3],
    /// The zodiacal constellations (Constellations.cfg `Zodiacal`), set when the file is loaded.
    pub zodiac: u128,
}

/// The per-constellation switches' script variable prefixes, in [`SkySwitches::per_constel`] order.
const PER_CONSTEL_VARS: [&str; 3] = ["constlines", "constboundary", "constlabel"];

impl Default for SkySwitches {
    /// main-def.cfg: `ShowConstellations false` with lines, boundaries and names on and drawings off,
    /// `ShowGrids false` with `ShowGridEquatorial true`, the others off; `FullConstellationNames true`,
    /// `ConstelBordersAtInf true`. Every constellation's own switches start on.
    fn default() -> Self {
        let all = (1u128 << crate::constellations::IAU.len()) - 1;
        Self {
            constellations: false,
            constel_lines: true,
            constel_boundaries: true,
            constel_labels: true,
            constel_drawings: false,
            full_names: true,
            borders_at_inf: true,
            grids: false,
            grid_equatorial: true,
            grid_ecliptic: false,
            grid_galactic: false,
            grid_geographic: false,
            per_constel: [all; 3],
            zodiac: 0,
        }
    }
}

impl SkySwitches {
    /// SE's keys of the switches, as saved in the settings.
    pub const KEYS: [&'static str; 12] = [
        "ShowConstellations",
        "ShowConstelLines",
        "ShowConstelBoundaries",
        "ShowConstelLabels",
        "ShowConstelDrawings",
        "FullConstellationNames",
        "ConstelBordersAtInf",
        "ShowGrids",
        "ShowGridEquatorial",
        "ShowGridEcliptic",
        "ShowGridGalactic",
        "ShowGridGeographic",
    ];

    /// A switch by its key (any case).
    pub fn switch_mut(&mut self, key: &str) -> Option<&mut bool> {
        Some(match key.to_ascii_lowercase().as_str() {
            "showconstellations" => &mut self.constellations,
            "showconstellines" => &mut self.constel_lines,
            "showconstelboundaries" => &mut self.constel_boundaries,
            "showconstellabels" => &mut self.constel_labels,
            "showconsteldrawings" => &mut self.constel_drawings,
            "fullconstellationnames" => &mut self.full_names,
            "constelbordersatinf" => &mut self.borders_at_inf,
            "showgrids" => &mut self.grids,
            "showgridequatorial" => &mut self.grid_equatorial,
            "showgridecliptic" => &mut self.grid_ecliptic,
            "showgridgalactic" => &mut self.grid_galactic,
            "showgridgeographic" => &mut self.grid_geographic,
            _ => return None,
        })
    }

    pub fn switch(&self, key: &str) -> Option<bool> {
        let mut s = *self;
        s.switch_mut(key).map(|v| *v)
    }

    /// The switches in [`SkySwitches::KEYS`] order (what the settings save).
    pub fn saved(&self) -> [bool; 12] {
        Self::KEYS.map(|k| self.switch(k).unwrap_or(false))
    }

    /// Whether constellation `i`'s lines (0), boundary (1) or label (2) are switched on.
    pub fn constel(&self, part: usize, i: usize) -> bool {
        self.per_constel[part] & (1 << i) != 0
    }

    /// A script variable of these switches (any case): the switch keys, and `ConstLines<Abbr>`,
    /// `ConstLinesAll`, `ConstLinesZodiac` and their `ConstBoundary*`, `ConstLabel*` twins.
    pub fn var(&self, key: &str) -> Option<bool> {
        if let Some(v) = self.switch(key) {
            return Some(v);
        }
        let (part, mask) = self.constel_var(key)?;
        Some(self.per_constel[part] & mask == mask)
    }

    /// Sets a script variable (see [`SkySwitches::var`]); false when it is not one of these. Setting
    /// `ConstLinesAll` switches every constellation's lines, `ConstLinesZodiac` the zodiacal ones'.
    pub fn set_var(&mut self, key: &str, on: bool) -> bool {
        if let Some(v) = self.switch_mut(key) {
            *v = on;
            return true;
        }
        let Some((part, mask)) = self.constel_var(key) else { return false };
        if on {
            self.per_constel[part] |= mask;
        } else {
            self.per_constel[part] &= !mask;
        }
        true
    }

    /// (part, constellations) of a `ConstLines*` / `ConstBoundary*` / `ConstLabel*` variable.
    fn constel_var(&self, key: &str) -> Option<(usize, u128)> {
        let k = key.to_ascii_lowercase();
        let (part, rest) = PER_CONSTEL_VARS.iter().enumerate().find_map(|(p, pre)| Some((p, k.strip_prefix(pre)?)))?;
        let mask = match rest {
            "all" => (1u128 << crate::constellations::IAU.len()) - 1,
            "zodiac" => self.zodiac,
            abbr => 1 << crate::constellations::IAU.iter().position(|(a, _)| a.eq_ignore_ascii_case(abbr))?,
        };
        Some((part, mask))
    }
}

impl Default for OverlaySettings {
    /// SE's defaults (main-def.cfg): no orbits (`ShowOrbits false`), labels (`LabelsMode 0`), markers
    /// (`ShowMarkers false`) or frame rate (`FPS false`), and its per-kind switches. For scripted runs
    /// (which skip the saved settings): env `SE_SHOW_FPS=1` shows the frame rate anyway, and
    /// `SE_OVERLAY` switches overlays on, e.g. `orbits,labels=2,markers,ShowLabelsGalaxies`, and the
    /// sky's (`constellations`, `grids`, `ShowGridEcliptic`, `ShowConstelBoundaries=0`, ...).
    fn default() -> Self {
        use MarkerKind::*;
        let mut s = Self {
            orbits: false,
            orbit_kinds: KindSet::of(&[Ship, Planet, Moon, DwarfPlanet, Sun, Barycenter]),
            labels_mode: 0,
            label_kinds: KindSet::of(&[Ship, Planet, Moon, DwarfPlanet, DwarfMoon, Asteroid, Comet, Sun, Barycenter]),
            markers: false,
            marker_kinds: KindSet::of(&MarkerKind::ALL),
            star_label_mag: 1.5,
            selection_pointer: true,
            show_fps: std::env::var_os("SE_SHOW_FPS").is_some(),
            info_full: true,
            sky: SkySwitches::default(),
        };
        for w in std::env::var("SE_OVERLAY").unwrap_or_default().split(',') {
            match w.trim() {
                "orbits" => s.orbits = true,
                "markers" => s.markers = true,
                "labels" => s.labels_mode = 1,
                "labels=2" => s.labels_mode = 2,
                "constellations" => s.sky.constellations = true,
                "grids" => s.sky.grids = true,
                key => {
                    let (key, on) = match key.split_once('=') {
                        Some((k, v)) => (k, v != "0" && !v.eq_ignore_ascii_case("false")),
                        None => (key, true),
                    };
                    if let Some((set, kind)) = kind_filter(&mut s, &key.to_ascii_lowercase()) {
                        set.set(kind, on);
                    } else {
                        s.sky.set_var(key, on);
                    }
                }
            }
        }
        s
    }
}

/// Screen-space markers computed this frame (also used for picking).
#[derive(Resource, Default)]
pub struct ScreenMarkers {
    pub items: Vec<Marker>,
}

#[derive(Clone, Debug)]
pub struct Marker {
    pub obj: ObjectRef,
    pub screen: Vec2,
    pub label: String,
    pub priority: f32,
    pub kind: MarkerKind,
    /// A minor body that looks tiny: labelled in the detail mode only.
    pub small: bool,
}

#[derive(Resource, Default)]
struct ClickState {
    press_pos: Option<Vec2>,
    moved: f32,
}

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OverlaySettings>()
            .init_resource::<ScreenMarkers>()
            .init_resource::<ClickState>()
            .init_resource::<SkyFade>()
            .add_systems(Startup, load_sky_config)
            .add_systems(PostUpdate, compute_markers.after(UniverseSet::FloatingOrigin).before(TransformSystems::Propagate))
            .add_systems(
                EguiPrimaryContextPass,
                (draw_orbits, draw_labels, draw_selection_pointer).chain().in_set(crate::skin::UiSet::Draw),
            )
            .add_systems(Update, (overlay_keys, pick_on_click, fade_sky.after(overlay_keys)));
    }
}

/// SE's main-def.cfg values for the sky overlays (Interface `ConstelBordersRadius`,
/// `ConstelBordersFadeStart/End`, `ConstelLinesFadeStart/End`, `OverlayFadeIn/Out`; Display
/// `LinesWidth`), read from the install.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct SkyConfig {
    /// The boundaries' sphere around the Sun when not at infinity, parsecs.
    pub borders_radius_pc: f64,
    /// Boundaries and names fade out between these distances from the Sun, parsecs.
    pub borders_fade: (f64, f64),
    /// Lines fade out between these distances from the Sun, parsecs.
    pub lines_fade: (f64, f64),
    /// A switch's fade in and out, seconds (seSmoothSwitch).
    pub fade_in_s: f32,
    pub fade_out_s: f32,
    /// prim_line.glsl's `Width.x`: the lines' Gaussian `exp(-r^2 / LinesWidth)`, r in pixels.
    pub lines_width: f32,
}

impl Default for SkyConfig {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self {
            borders_radius_pc: 10.0,
            borders_fade: (200.0, 800.0),
            lines_fade: (4000.0, 10000.0),
            fade_in_s: 0.2,
            fade_out_s: 0.35,
            lines_width: 1.5,
        }
    }
}

fn load_sky_config(mut commands: Commands, cfg: Res<crate::config::EngineConfig>) {
    let mut c = SkyConfig::default();
    if let Ok(text) = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")) {
        let doc = se_core::sc::parse(&text);
        let f = |key: &str| doc.find_deep(key).and_then(|i| i.f64());
        c.borders_radius_pc = f("ConstelBordersRadius").unwrap_or(c.borders_radius_pc);
        c.borders_fade = (f("ConstelBordersFadeStart").unwrap_or(c.borders_fade.0), f("ConstelBordersFadeEnd").unwrap_or(c.borders_fade.1));
        c.lines_fade = (f("ConstelLinesFadeStart").unwrap_or(c.lines_fade.0), f("ConstelLinesFadeEnd").unwrap_or(c.lines_fade.1));
        c.fade_in_s = f("OverlayFadeIn").map_or(c.fade_in_s, |v| v as f32);
        c.fade_out_s = f("OverlayFadeOut").map_or(c.fade_out_s, |v| v as f32);
        c.lines_width = f("LinesWidth").filter(|v| *v > 0.0).map_or(c.lines_width, |v| v as f32);
    }
    commands.insert_resource(c);
}

/// The sky switches' smooth values (SE's seSmoothSwitch, value at +0x14): each moves linearly to 1
/// over `OverlayFadeIn` when switched on and to 0 over `OverlayFadeOut` when off; SE draws with
/// the value squared.
#[derive(Resource, Debug, Clone)]
pub struct SkyFade {
    pub constellations: f32,
    pub constel: [f32; 3],
    pub grids: f32,
    /// Equatorial, ecliptic, galactic, geographic.
    pub grid: [f32; 4],
    /// Per constellation: lines, boundaries, labels.
    pub per_constel: [Vec<f32>; 3],
}

impl Default for SkyFade {
    fn default() -> Self {
        let n = crate::constellations::IAU.len();
        Self { constellations: 0.0, constel: [0.0; 3], grids: 0.0, grid: [0.0; 4], per_constel: [vec![1.0; n], vec![1.0; n], vec![1.0; n]] }
    }
}

/// One step of a smooth switch towards `on`.
pub(crate) fn fade_step(v: f32, on: bool, dt: f32, cfg: &SkyConfig) -> f32 {
    if on { (v + dt / cfg.fade_in_s.max(1e-3)).min(1.0) } else { (v - dt / cfg.fade_out_s.max(1e-3)).max(0.0) }
}

fn fade_sky(time: Res<Time>, settings: Res<OverlaySettings>, cfg: Option<Res<SkyConfig>>, mut fade: ResMut<SkyFade>) {
    let cfg = cfg.as_deref().copied().unwrap_or_default();
    let (dt, s) = (time.delta_secs(), &settings.sky);
    let f = &mut *fade;
    f.constellations = fade_step(f.constellations, s.constellations, dt, &cfg);
    for (v, on) in f.constel.iter_mut().zip([s.constel_lines, s.constel_boundaries, s.constel_labels]) {
        *v = fade_step(*v, on, dt, &cfg);
    }
    f.grids = fade_step(f.grids, s.grids, dt, &cfg);
    for (v, on) in f.grid.iter_mut().zip([s.grid_equatorial, s.grid_ecliptic, s.grid_galactic, s.grid_geographic]) {
        *v = fade_step(*v, on, dt, &cfg);
    }
    for (part, values) in f.per_constel.iter_mut().enumerate() {
        for (i, v) in values.iter_mut().enumerate() {
            *v = fade_step(*v, s.constel(part, i), dt, &cfg);
        }
    }
}

/// SE's overlay keys: O orbits, ' labels (off, brief, detail), Ctrl+' markers, ; the selection
/// pointer; X constellations, Shift+X their lines, Ctrl+X boundaries, Alt+X names, Ctrl+; the
/// celestial grids, Alt+; the geographic grid (key table FUN_1405ba8f0: ToggleConstellations,
/// ToggleConstLines/Boundaries/Labels, ToggleCelGrids, ToggleGeoGrids), also while a ship is piloted.
fn overlay_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<OverlaySettings>,
    mut egui: EguiContexts,
    mut msg: ResMut<crate::messages::Messages>,
    i18n: Res<crate::i18n::I18n>,
) {
    if egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false) {
        return;
    }
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let mods = ctrl || shift || alt;
    let sky = &mut settings.sky;
    let x = match (shift, ctrl, alt) {
        (false, false, false) => Some((&mut sky.constellations, "Constellations")),
        (true, false, false) => Some((&mut sky.constel_lines, "Constellation lines")),
        (false, true, false) => Some((&mut sky.constel_boundaries, "Constellation boundaries")),
        (false, false, true) => Some((&mut sky.constel_labels, "Constellation names")),
        _ => None,
    };
    if let (Some((switch, name)), true) = (x, keys.just_pressed(KeyCode::KeyX)) {
        *switch = !*switch;
        msg.switch(&i18n, name, *switch);
    }
    let sky = &mut settings.sky;
    if keys.just_pressed(KeyCode::Semicolon) && !shift && ctrl != alt {
        let (switch, name) = if ctrl { (&mut sky.grids, "Celestial grids") } else { (&mut sky.grid_geographic, "Geographic grid") };
        *switch = !*switch;
        msg.switch(&i18n, name, *switch);
    }
    if keys.just_pressed(KeyCode::KeyO) && !mods {
        settings.orbits = !settings.orbits;
        msg.switch(&i18n, "Orbits", settings.orbits);
    }
    if keys.just_pressed(KeyCode::Quote) {
        if ctrl {
            settings.markers = !settings.markers;
            msg.switch(&i18n, "Markers", settings.markers);
        } else if !mods {
            settings.labels_mode = (settings.labels_mode + 1) % 3;
            match settings.labels_mode {
                0 => msg.switch(&i18n, "Labels", false),
                m => msg.show(format!("{}: {}", i18n.tr("Labels"), i18n.tr(if m == 1 { "brief" } else { "detail" }))),
            }
        }
    }
    if keys.just_pressed(KeyCode::Semicolon) && !mods {
        settings.selection_pointer = !settings.selection_pointer;
        msg.switch(&i18n, "Selection pointer", settings.selection_pointer);
    }
}

/// The overlay's kind for a body of the system around the camera.
fn body_kind(data: &SeData, obj: ObjectRef) -> MarkerKind {
    match MarkerKind::of(data, obj) {
        MarkerKind::Star => MarkerKind::Sun,
        k => k,
    }
}

/// A barycentre's first body (Earth-Moon's Earth), if bodies orbit it.
fn bary_body(data: &SeData, obj: ObjectRef) -> Option<ObjectRef> {
    let cat = &data.catalog;
    cat.children_of(cat.display_name(obj)).iter().copied().find(|c| matches!(c, ObjectRef::Body(_)))
}

/// The skin's colour key for an object: a barycentre's after what orbits it (`InfoPlanBaryColor`,
/// `InfoAsterBaryColor`, else `InfoStarBaryColor`).
fn color_key(data: &SeData, obj: ObjectRef, kind: MarkerKind) -> &'static str {
    if kind != MarkerKind::Barycenter {
        return kind.skin_key();
    }
    match bary_body(data, obj).map(|b| MarkerKind::of(data, b)) {
        Some(MarkerKind::Planet | MarkerKind::DwarfPlanet | MarkerKind::Moon) => "PlanBary",
        Some(_) => "AsterBary",
        None => "StarBary",
    }
}

/// The overlay's layer, below the windows: the sky's grids and constellations, orbit lines, then
/// labels and markers.
pub(crate) fn overlay_layer() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Background, egui::Id::new("overlay"))
}

/// The camera's pinhole projection in logical pixels.
pub(crate) struct View {
    inv: DQuat,
    pub focal: f64,
    centre: (f64, f64),
}

impl View {
    pub fn new(cam: &FlyCamera, vp: Vec2) -> Self {
        let focal = vp.y as f64 * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();
        Self { inv: cam.rot.inverse(), focal, centre: (vp.x as f64 * 0.5, vp.y as f64 * 0.5) }
    }

    /// Camera space: x right, y up, -z ahead.
    pub fn local(&self, p: DVec3) -> DVec3 {
        self.inv * p
    }
    /// A point ahead of the camera (camera space) on the screen, if it is in front.
    pub fn project(&self, v: DVec3) -> Option<egui::Pos2> {
        (-v.z > 1e-6 * v.length()).then(|| self.screen(v))
    }

    pub fn screen(&self, v: DVec3) -> egui::Pos2 {
        let d = (-v.z).max(1e-30);
        let x = (self.centre.0 + self.focal * v.x / d).clamp(-1e7, 1e7);
        let y = (self.centre.1 - self.focal * v.y / d).clamp(-1e7, 1e7);
        egui::pos2(x as f32, y as f32)
    }
}

/// SE's orbit trail (main-def.cfg `OrbitTrailsLength`) and line widths (`OrbitMajorThickness`,
/// `OrbitMinorThickness`, pixels).
const ORBIT_TRAILS: f32 = 0.72;
const ORBIT_MAJOR_PX: f32 = 5.02377;
const ORBIT_MINOR_PX: f32 = 3.16979;
const ORBIT_POINTS: usize = 256;
/// Across an orbit line (fractions of its half-width, twice the thickness): SE's `exp(-27 u^2)`.
const ORBIT_BAND: [(f32, f32); 7] = [(-0.32, 0.0), (-0.2, 0.34), (-0.1, 0.76), (0.0, 1.0), (0.1, 0.76), (0.2, 0.34), (0.32, 0.0)];

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// SE's orbit lines (orbit.glsl): a screen-space band, brightest along the path, colour x2, and a
/// trail: opacity `1 - OrbitTrailsLength (1 - f^2)`, f the fraction of the orbit from just ahead of
/// the body, so full just behind the body and dimmest just ahead of it. The path runs through the
/// body's exact position. Every member of the system around the camera has one, barycentres too
/// (Earth's path round the Sun is the Earth-Moon barycentre's). Bodies' discs hide the lines behind
/// them; an orbit fades in as it grows past a few pixels on screen (assumed).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_orbits(
    mut contexts: EguiContexts,
    settings: Res<OverlaySettings>,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    sim: Res<SimTime>,
    active: Res<ActiveSystem>,
    bodies: Query<&SystemBody>,
    cams: Query<&Camera, With<MainCamera>>,
    mut members: Local<(Option<u32>, Vec<ObjectRef>)>,
    ships: Query<(Entity, &crate::ship::Ship, &crate::universe::UPosition)>,
    flight: Res<crate::ship::Flight>,
) -> Result {
    if !settings.orbits {
        return Ok(());
    }
    let Ok(camera) = cams.single() else { return Ok(()) };
    let Some(vp) = camera.logical_viewport_size() else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let skin = crate::skin::text(ctx);
    let px = 1.0 / ctx.pixels_per_point();
    let view = View::new(&cam, vp);
    let eph = Ephemeris::new(&data.catalog);
    let spheres: Vec<(DVec3, f64)> =
        bodies.iter().filter(|b| b.px_radius > 1.0 && b.radius_m > 0.0).map(|b| (b.pos.delta_m(cam.pos), b.radius_m)).collect();
    let hidden = |p: DVec3| {
        let d = p.length();
        let dir = p / d.max(1e-30);
        spheres.iter().any(|(c, r)| {
            let b = c.dot(dir);
            let disc = b * b - (c.length_squared() - r * r);
            disc > 0.0 && b - disc.sqrt() > 0.0 && b - disc.sqrt() < d
        })
    };
    if members.0 != active.root {
        *members = (active.root, data.catalog.system_members(&active.name));
    }
    let mut mesh = egui::Mesh::default();
    for &obj in &members.1 {
        // A barycentre of bodies counts as its first body (the Earth-Moon one's path is a planet's,
        // a binary asteroid's an asteroid's); one of stars as a barycentre.
        let kind = match body_kind(&data, obj) {
            MarkerKind::Barycenter => bary_body(&data, obj).map_or(MarkerKind::Barycenter, |b| MarkerKind::of(&data, b)),
            k => k,
        };
        let selected = cam.selected == Some(obj);
        if !selected && !settings.orbit_kinds.has(kind) {
            continue;
        }
        let Some((parent, pts)) = eph.orbit_path(obj, sim.jd, ORBIT_POINTS) else { continue };
        let parent_pos = eph.position(parent, sim.jd);
        let base = parent_pos.delta_m(cam.pos);
        let size = pts.iter().map(|p| p.length()).fold(0.0, f64::max);
        let vis = smoothstep(2.0, 12.0, (view.focal * size / base.length().max(1.0)) as f32);
        if vis <= 0.0 {
            continue;
        }
        // The body on its path: the nearest point of the sampled ellipse, as a fraction of the orbit.
        let rel = eph.position(obj, sim.jd).delta_m(parent_pos);
        let n = pts.len() - 1;
        let (seg, t) = (0..n)
            .map(|i| {
                let (a, d) = (pts[i], pts[i + 1] - pts[i]);
                let t = ((rel - a).dot(d) / d.length_squared().max(1e-30)).clamp(0.0, 1.0);
                (i, t, (a + d * t).distance_squared(rel))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map_or((0, 0.0), |(i, t, _)| (i, t));
        let at = (seg as f64 + t) / n as f64;
        // The body is off the catalog's mean ellipse when its position comes from a theory (the
        // Moon's) or the ephemeris: bend the path into it over a quarter of the orbit either side.
        let off = rel - (pts[seg] + (pts[seg + 1] - pts[seg]) * t);
        let bend = |f: f64| off * (1.0 - (f.min(1.0 - f) / 0.25).min(1.0)).powi(2);
        // From just ahead of the body round to just behind it: (offset from the camera, f).
        let mut path: Vec<(DVec3, f32)> = Vec::with_capacity(n + 2);
        path.push((base + rel, 0.0));
        for k in 1..=n {
            let i = (seg + k) % n;
            let f = (i as f64 / n as f64 - at).rem_euclid(1.0);
            path.push((base + pts[i] + bend(f), f as f32));
        }
        path.push((base + rel, 1.0));
        let color = if selected { skin.selected_orbit } else { skin.kind(color_key(&data, obj, body_kind(&data, obj))) };
        let width = if kind.major() || selected { ORBIT_MAJOR_PX } else { ORBIT_MINOR_PX } * px;
        let alpha = |p: DVec3, f: f32| if hidden(p) { 0.0 } else { (1.0 - ORBIT_TRAILS * (1.0 - f * f)) * vis };
        let points: Vec<(DVec3, f32)> = path.iter().map(|&(p, f)| (view.local(p), alpha(p, f))).collect();
        for run in clip_runs(&points) {
            let screen: Vec<(egui::Pos2, f32)> = run.iter().map(|&(v, a)| (view.screen(v), a)).collect();
            add_band(&mut mesh, &screen, color, 2.0 * width);
        }
    }
    // SE's ship trajectories (ShowOrbitsShips, "View ship trajectory"): each flying ship's conic about
    // the body pulling it hardest, the flown one in the selected-orbit colour (ShipSelectedOrbitColor;
    // its value is not in the skin, so BodySelectedOrbitColor's is used).
    if settings.orbit_kinds.has(MarkerKind::Ship) {
        let pulls: Vec<(se_core::upos::UPos, f64, f64)> = bodies
            .iter()
            .filter(|b| b.radius_m > 0.0)
            .map(|b| (b.pos, se_core::units::G * crate::ship::body_mass(&data, b.obj, b.radius_m), b.radius_m))
            .collect();
        for (entity, ship, pos) in &ships {
            if ship.landed.is_some() || ship.parked.is_some() {
                continue;
            }
            let Some(&(bpos, mu, radius)) = pulls.iter().max_by(|a, b| {
                let g = |p: &(se_core::upos::UPos, f64, f64)| p.1 / pos.0.delta_m(p.0).length_squared().max(1.0);
                g(a).total_cmp(&g(b))
            }) else {
                continue;
            };
            let Some(obj) = bodies.iter().find(|b| b.pos == bpos).map(|b| b.obj) else { continue };
            // The body's own velocity, from its ephemeris a second either side.
            let dt = 1.0 / 86_400.0;
            let body_vel = eph.position(obj, sim.jd + dt).delta_m(eph.position(obj, sim.jd - dt)) / 2.0;
            let (r, v) = (pos.0.delta_m(bpos), ship.vel - body_vel);
            let Some(conic) = ship_conic(r, v, mu, radius, ORBIT_POINTS) else { continue };
            let base = bpos.delta_m(cam.pos);
            let flown = flight.ship == Some(entity);
            let color = if flown { skin.selected_orbit } else { skin.kind(MarkerKind::Ship.skin_key()) };
            let width = if flown { ORBIT_MAJOR_PX } else { ORBIT_MINOR_PX } * px;
            let points: Vec<(DVec3, f32)> = conic
                .iter()
                .map(|&(p, f)| {
                    let at = base + p;
                    (view.local(at), if hidden(at) { 0.0 } else { 1.0 - ORBIT_TRAILS * (1.0 - f * f) })
                })
                .collect();
            for run in clip_runs(&points) {
                let screen: Vec<(egui::Pos2, f32)> = run.iter().map(|&(v, a)| (view.screen(v), a)).collect();
                add_band(&mut mesh, &screen, color, 2.0 * width);
            }
        }
    }
    if !mesh.is_empty() {
        ctx.layer_painter(overlay_layer()).add(egui::Shape::mesh(mesh));
    }
    Ok(())
}

/// A ship's conic about a body of gravitational parameter `mu` and radius `radius` from its relative
/// position `r` and velocity `v`: points relative to the body with the trail fraction `f` (0 just ahead
/// of the ship, 1 just behind it, as [`draw_orbits`]). A bound orbit closes; an escape path runs out
/// to 20 times its periapsis (or the body's radius) on either side of the ship.
pub(crate) fn ship_conic(r: DVec3, v: DVec3, mu: f64, radius: f64, n: usize) -> Option<Vec<(DVec3, f32)>> {
    let rl = r.length();
    let h = r.cross(v);
    if mu <= 0.0 || rl <= 0.0 || h.length_squared() <= 0.0 {
        return None;
    }
    let e_vec = v.cross(h) / mu - r / rl;
    let e = e_vec.length();
    let p = h.length_squared() / mu;
    let pa = if e > 1e-9 { e_vec / e } else { r / rl };
    let qa = h.normalize().cross(pa);
    let at = |th: f64| pa * th.cos() + qa * th.sin();
    let nu = r.dot(qa).atan2(r.dot(pa));
    let mut pts = Vec::with_capacity(n + 1);
    if e < 1.0 {
        for k in 0..=n {
            let th = nu + std::f64::consts::TAU * k as f64 / n as f64;
            pts.push((at(th) * (p / (1.0 + e * th.cos())), k as f32 / n as f32));
        }
    } else {
        // Out to r_max on the incoming and outgoing legs.
        let r_max = 20.0 * (p / (1.0 + e)).max(radius);
        let th_max = ((p / r_max - 1.0) / e).clamp(-1.0, 1.0).acos();
        let (lo, hi) = (-th_max, th_max);
        if nu <= lo || nu >= hi {
            return None;
        }
        // From the ship on to the outgoing end, then back from the incoming end to the ship.
        let ahead = ((n as f64) * (hi - nu) / (hi - lo)).ceil().max(1.0) as usize;
        for k in 0..=ahead {
            let th = nu + (hi - nu) * k as f64 / ahead as f64;
            pts.push((at(th) * (p / (1.0 + e * th.cos())), 0.5 * k as f32 / ahead as f32));
        }
        let behind = n.saturating_sub(ahead).max(1);
        for k in 0..=behind {
            let th = lo + (nu - lo) * k as f64 / behind as f64;
            pts.push((at(th) * (p / (1.0 + e * th.cos())), 0.5 + 0.5 * k as f32 / behind as f32));
        }
    }
    Some(pts)
}

/// Splits a camera-space polyline (with an alpha per point) into its runs in front of the camera,
/// cut where they come within 0.6 degrees of the camera's plane (projected within 100 focal lengths
/// of the centre).
pub(crate) fn clip_runs(points: &[(DVec3, f32)]) -> Vec<Vec<(DVec3, f32)>> {
    const NEAR: f64 = 1e-2;
    let ahead = |v: DVec3| -v.z > NEAR * v.length();
    let mut runs = Vec::new();
    let mut run: Vec<(DVec3, f32)> = Vec::new();
    for (k, &(v, a)) in points.iter().enumerate() {
        if k > 0 {
            let (u, b) = points[k - 1];
            if ahead(u) != ahead(v) {
                let eps = NEAR * u.length().max(v.length());
                let (du, dv) = (-u.z, -v.z);
                let t = if du != dv { ((du - eps) / (du - dv)).clamp(0.0, 1.0) } else { 0.0 };
                run.push((u + (v - u) * t, b + (a - b) * t as f32));
                if ahead(u) {
                    runs.push(std::mem::take(&mut run));
                }
            }
        }
        if ahead(v) {
            run.push((v, a));
        }
    }
    runs.push(run);
    runs.retain(|r| r.len() > 1);
    runs
}

/// A band along a screen polyline, [`ORBIT_BAND`] across `half` pixels either side; the colour
/// doubled as in SE and premultiplied by each point's alpha.
fn add_band(mesh: &mut egui::Mesh, pts: &[(egui::Pos2, f32)], color: egui::Color32, half: f32) {
    let rows = ORBIT_BAND.len() as u32;
    let [r, g, b, a] = color.to_srgba_unmultiplied().map(|c| c as f32 / 255.0);
    let mut normal = egui::vec2(0.0, 1.0);
    for j in 0..pts.len() {
        let d = pts[(j + 1).min(pts.len() - 1)].0 - pts[j.saturating_sub(1)].0;
        if d.length_sq() > 1e-6 {
            let d = d.normalized();
            normal = egui::vec2(-d.y, d.x);
        }
        let base = mesh.vertices.len() as u32;
        let (p, alpha) = pts[j];
        for &(u, fall) in &ORBIT_BAND {
            let k = alpha * a * fall;
            let c = |v: f32| ((v * 2.0).min(1.0) * k * 255.0).round() as u8;
            let col = egui::Color32::from_rgba_premultiplied(c(r), c(g), c(b), (k * 255.0).round() as u8);
            mesh.colored_vertex(p + normal * (u * half), col);
        }
        if j > 0 {
            let prev = base - rows;
            for c in 0..rows - 1 {
                mesh.add_triangle(prev + c, prev + c + 1, base + c);
                mesh.add_triangle(prev + c + 1, base + c + 1, base + c);
            }
        }
    }
}

/// The cone round the view direction holding the screen (with a margin), for culling the sky's
/// paths before projecting them.
pub(crate) struct ViewCone {
    fwd: DVec3,
    half: f64,
}

impl ViewCone {
    pub fn new(cam: &FlyCamera, view: &View) -> Self {
        let corner = (view.centre.0 * view.centre.0 + view.centre.1 * view.centre.1).sqrt();
        Self { fwd: cam.rot * DVec3::NEG_Z, half: (corner / view.focal).atan() + 0.03 }
    }

    /// Whether a cone of half-angle `r` round `axis` (unit) can reach the screen.
    pub fn sees(&self, axis: DVec3, r: f64) -> bool {
        let lim = self.half + r;
        lim >= std::f64::consts::PI || self.fwd.dot(axis) > lim.cos()
    }
}

/// A path of directions on the sky (universe frame), with the cone round every few segments so
/// that the parts out of view can be skipped.
pub(crate) struct SkyPath {
    pub pts: Vec<DVec3>,
    /// Per piece of [`SkyPath::PIECE`] segments: the cone's axis and half-angle.
    pieces: Vec<(DVec3, f64)>,
}

impl SkyPath {
    const PIECE: usize = 8;

    pub fn new(pts: Vec<DVec3>) -> Self {
        let pieces = (0..pts.len().saturating_sub(1))
            .step_by(Self::PIECE)
            .map(|s| {
                let p = &pts[s..=(s + Self::PIECE).min(pts.len() - 1)];
                let axis = p.iter().fold(DVec3::ZERO, |a, v| a + v.normalize_or_zero()).normalize_or(DVec3::Z);
                (axis, p.iter().map(|v| axis.angle_between(*v)).fold(0.0, f64::max))
            })
            .collect();
        Self { pts, pieces }
    }

    /// The point ranges of the path that may show (consecutive pieces joined).
    pub fn visible(&self, cone: &ViewCone) -> Vec<std::ops::Range<usize>> {
        let mut out: Vec<std::ops::Range<usize>> = Vec::new();
        for (k, &(axis, r)) in self.pieces.iter().enumerate() {
            if !cone.sees(axis, r) {
                continue;
            }
            let (s, e) = (k * Self::PIECE, ((k + 1) * Self::PIECE + 1).min(self.pts.len()));
            match out.last_mut() {
                Some(last) if last.end == s + 1 => last.end = e,
                _ => out.push(s..e),
            }
        }
        out
    }
}

/// SE's prim_line.glsl lines (constellations, grids): across a line the Gaussian
/// `g = exp(-r^2 / LinesWidth)` (r in physical pixels, out to where it falls to 1/256), blended
/// `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` with colour `bright g rgb` and alpha `g a`; in egui's
/// premultiplied terms, colour `bright g rgb clamp(g a)` over alpha `clamp(g a)`.
pub(crate) struct PrimLines {
    pub mesh: egui::Mesh,
    /// Across the line: (offset in logical pixels, g).
    rows: [(f32, f32); 5],
    /// The viewport with a margin; pieces wholly off one side of it are left out.
    clip: egui::Rect,
    /// SE's premultiplied line path (`glBlendFunc(ONE, ONE_MINUS_SRC_ALPHA)`, as for the
    /// geographic grid, FUN_1404c7460): colour `bright g rgb` over alpha `bright g a`.
    pub premultiplied: bool,
}

impl PrimLines {
    pub fn new(lines_width: f32, pixels_per_point: f32, vp: Vec2) -> Self {
        let w = lines_width.max(0.05);
        let r_max = (256f32.ln() * w).sqrt();
        let r_mid = 0.4 * r_max;
        let g = (-r_mid * r_mid / w).exp();
        let px = 1.0 / pixels_per_point.max(0.1);
        let rows = [(-r_max * px, 0.0), (-r_mid * px, g), (0.0, 1.0), (r_mid * px, g), (r_max * px, 0.0)];
        let clip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(vp.x, vp.y)).expand(8.0);
        Self { mesh: egui::Mesh::default(), rows, clip, premultiplied: false }
    }

    /// A camera-space polyline with an alpha per point (0 where it is hidden), in `rgb` x `bright`.
    pub fn polyline(&mut self, view: &View, pts: &[(DVec3, f32)], rgb: [f32; 3], bright: f32) {
        let c = self.clip;
        let off = |a: egui::Pos2, b: egui::Pos2| {
            (a.x < c.min.x && b.x < c.min.x) || (a.x > c.max.x && b.x > c.max.x) || (a.y < c.min.y && b.y < c.min.y) || (a.y > c.max.y && b.y > c.max.y)
        };
        for run in clip_runs(pts) {
            let screen: Vec<(egui::Pos2, f32)> = run.iter().map(|&(v, a)| (view.screen(v), a)).collect();
            let mut piece: Vec<(egui::Pos2, f32)> = Vec::new();
            for w in screen.windows(2) {
                if (w[0].1 <= 0.0 && w[1].1 <= 0.0) || off(w[0].0, w[1].0) {
                    self.band(&piece, rgb, bright);
                    piece.clear();
                    continue;
                }
                if piece.is_empty() {
                    piece.push(w[0]);
                }
                piece.push(w[1]);
            }
            self.band(&piece, rgb, bright);
        }
    }

    fn band(&mut self, pts: &[(egui::Pos2, f32)], rgb: [f32; 3], bright: f32) {
        if pts.len() < 2 {
            return;
        }
        let rows = self.rows.len() as u32;
        let mesh = &mut self.mesh;
        let mut normal = egui::vec2(0.0, 1.0);
        for j in 0..pts.len() {
            let d = pts[(j + 1).min(pts.len() - 1)].0 - pts[j.saturating_sub(1)].0;
            if d.length_sq() > 1e-6 {
                let d = d.normalized();
                normal = egui::vec2(-d.y, d.x);
            }
            let base = mesh.vertices.len() as u32;
            let (p, a) = pts[j];
            for &(u, g) in &self.rows {
                let (alpha, k) = if self.premultiplied { ((bright * g * a).clamp(0.0, 1.0), bright * g) } else { ((g * a).clamp(0.0, 1.0), bright * g * (g * a).clamp(0.0, 1.0)) };
                let ch = |v: f32| ((k * v).clamp(0.0, 1.0) * 255.0).round() as u8;
                let col = egui::Color32::from_rgba_premultiplied(ch(rgb[0]), ch(rgb[1]), ch(rgb[2]), (alpha * 255.0).round() as u8);
                mesh.colored_vertex(p + normal * u, col);
            }
            if j > 0 {
                let prev = base - rows;
                for r in 0..rows - 1 {
                    mesh.add_triangle(prev + r, prev + r + 1, base + r);
                    mesh.add_triangle(prev + r + 1, base + r + 1, base + r);
                }
            }
        }
    }
}

/// The system's resolved bodies (discs over a pixel), which hide what is behind them: each as its
/// centre relative to the camera and the map of camera-relative vectors into the space where its
/// ellipsoid is the unit sphere (an oblate Earth's surface at mid latitudes lies well inside its
/// equatorial sphere).
pub(crate) struct Occluders(Vec<Occluder>);

struct Occluder {
    centre: DVec3,
    inv: DQuat,
    scale: DVec3,
    /// The direction to the centre and the cosine of the cone round it holding the ellipsoid (-2
    /// from inside its bounding sphere): rays outside it miss.
    axis: DVec3,
    cone_cos: f64,
}

impl Occluders {
    pub fn new<'a>(bodies: impl Iterator<Item = &'a SystemBody>, cam: &FlyCamera) -> Self {
        let list = bodies
            .filter(|b| b.px_radius > 1.0 && b.radius_m > 0.0)
            .map(|b| {
                let centre = b.pos.delta_m(cam.pos);
                let (dist, bound) = (centre.length(), b.radius_m * b.shape.max_element());
                let cone_cos = if dist > bound { (1.0 - (bound / dist).powi(2)).sqrt() } else { -2.0 };
                Occluder { centre, inv: b.orient.inverse(), scale: 1.0 / (b.shape * b.radius_m), axis: centre / dist.max(1e-30), cone_cos }
            })
            .collect();
        Self(list)
    }

    /// Whether the ray from the camera to `p` (camera-relative, metres) meets a body before `p`.
    /// From inside an ellipsoid (the ground under sea level), what points below the camera's own
    /// level is hidden.
    pub fn hides(&self, p: DVec3) -> bool {
        let d = p.length();
        let dir = p / d.max(1e-30);
        self.0.iter().any(|o| {
            if dir.dot(o.axis) < o.cone_cos {
                return false;
            }
            let (ad, ac) = (o.inv * dir * o.scale, o.inv * o.centre * o.scale);
            let (a, b, cc) = (ad.length_squared(), ad.dot(ac), ac.length_squared() - 1.0);
            if cc < 0.0 {
                return b > 0.0;
            }
            let disc = b * b - a * cc;
            let t = (b - disc.max(0.0).sqrt()) / a;
            disc > 0.0 && t > 0.0 && t < d
        })
    }

    /// Whether a direction from the camera (any length) meets a body.
    pub fn hides_dir(&self, dir: DVec3) -> bool {
        !self.0.is_empty() && self.hides(dir.normalize_or(DVec3::Z) * 1e30)
    }
}

fn project(camera: &Camera, cam_tf: &GlobalTransform, render_pos: Vec3) -> Option<Vec2> {
    // Behind the camera?
    let fwd = cam_tf.forward();
    if render_pos.dot(*fwd) <= 0.0 {
        return None;
    }
    camera.world_to_viewport(cam_tf, render_pos).ok()
}

/// Deep-sky objects get labels when brighter than this (apparent magnitude) or bigger (radians).
const DSO_LABEL_MAG: f64 = 9.0;
const DSO_LABEL_SIZE: f64 = 0.01;
/// SE's `MarkersSize` (pixels).
const MARKER_PX: f32 = 7.0;

fn compute_markers(
    settings: Res<OverlaySettings>,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    space: Res<ScaledSpace>,
    bodies: Query<&SystemBody>,
    cams: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut markers: ResMut<ScreenMarkers>,
    mut sky_cache: Local<(u32, Vec<(ObjectRef, f32)>)>,
    // 02-014: SE's ShowCatClusters / ShowProcClusters also hide the clusters' labels and markers.
    (cluster_filter, proc_settings): (Res<crate::clusters::ClusterSettings>, Res<crate::procgalaxies::ProcSettings>),
) {
    markers.items.clear();
    let Ok((camera, _)) = cams.single() else { return };
    // The camera transform is propagated later in the frame; build it from the source of truth.
    let cam_tf = GlobalTransform::from(Transform::from_rotation(cam.rot.as_quat()));
    let cat = &data.catalog;
    // Resolved bodies hide markers behind their discs: (screen centre, px radius, distance).
    let mut discs: Vec<(Vec2, f32, f64)> = Vec::new();
    for b in &bodies {
        if b.px_radius > 1.5 {
            let delta = b.pos.delta_m(cam.pos);
            if let Some(s) = project(camera, &cam_tf, space.map(delta).0) {
                discs.push((s, b.px_radius.min(1e6) as f32, delta.length()));
            }
        }
    }
    let occluded = |s: Vec2, dist: f64| discs.iter().any(|(c, r, d)| *d < dist * 0.999 && c.distance(s) < *r);
    // Rays from the camera blocked by a nearby body's sphere (e.g. the ground we stand on).
    let spheres: Vec<(DVec3, f64)> = bodies
        .iter()
        .filter(|b| b.radius_m > 0.0 && b.pos.delta_m(cam.pos).length() < b.radius_m * 50.0)
        .map(|b| (b.pos.delta_m(cam.pos), b.radius_m))
        .collect();
    let ray_blocked = |dir: DVec3, max_dist: f64| {
        spheres.iter().any(|(c, r)| {
            let b = c.dot(dir);
            let disc = b * b - (c.length_squared() - r * r);
            disc > 0.0 && b - disc.sqrt() > 0.0 && b - disc.sqrt() < max_dist
        })
    };
    for b in &bodies {
        let kind = body_kind(&data, b.obj);
        let delta = b.pos.delta_m(cam.pos);
        let Some(screen) = project(camera, &cam_tf, space.map(delta).0) else { continue };
        let dist = delta.length().max(1.0);
        if occluded(screen, dist) || ray_blocked(delta / dist, dist - b.radius_m) {
            continue;
        }
        // Rough visual importance: angular size, boosted for planets/stars.
        let ang = (b.radius_m / dist) as f32;
        let priority = ang.log10()
            + match kind {
                MarkerKind::Sun => 4.0,
                MarkerKind::Planet | MarkerKind::DwarfPlanet => 3.0,
                MarkerKind::Moon => 1.5,
                _ => 0.0,
            };
        markers.items.push(Marker {
            obj: b.obj,
            screen,
            label: cat.display_name(b.obj).to_string(),
            priority,
            kind,
            small: !kind.prominent() && ang < 1e-4,
        });
    }
    // Stars and deep-sky objects, for the kinds labelled or marked; the list is refreshed a few
    // times per second.
    let wanted = |k: MarkerKind| {
        (settings.labels_mode > 0 && settings.label_kinds.has(k)) || (settings.markers && settings.marker_kinds.has(k))
    };
    sky_cache.0 += 1;
    if sky_cache.0 % 15 == 1 {
        let cam_pc = cam.pos.to_parsecs();
        sky_cache.1.clear();
        if wanted(MarkerKind::Star) {
            for (i, s) in cat.stars.iter().enumerate() {
                if s.barycenter || !s.has_position || s.hidden() {
                    continue;
                }
                let d = s.pos_pc.distance(cam_pc).max(1e-9);
                let m = se_core::star::app_mag_from_abs(s.abs_mag, d) as f32;
                if m < settings.star_label_mag && d > 1e-3 {
                    sky_cache.1.push((ObjectRef::Star(i as u32), m));
                }
            }
        }
        let mut dso = |obj: ObjectRef, pos_pc: DVec3, radius_pc: f64, abs_mag: f64| {
            let d = pos_pc.distance(cam_pc);
            if d <= radius_pc {
                return;
            }
            let m = se_core::star::app_mag_from_abs(abs_mag, d);
            if m < DSO_LABEL_MAG || radius_pc / d > DSO_LABEL_SIZE {
                sky_cache.1.push((obj, m as f32));
            }
        };
        if wanted(MarkerKind::Galaxy) {
            for (i, g) in cat.galaxies.iter().enumerate() {
                dso(ObjectRef::Galaxy(i as u32), g.pos_pc, g.radius_pc, g.abs_mag);
            }
        }
        if wanted(MarkerKind::Cluster) {
            for (i, c) in cat.clusters.iter().enumerate().filter(|(_, c)| cluster_filter.shows(c, proc_settings.clusters)) {
                dso(ObjectRef::Cluster(i as u32), c.pos_pc, c.radius_pc, c.abs_mag);
            }
        }
        if wanted(MarkerKind::Nebula) {
            for (i, n) in cat.nebulae.iter().enumerate() {
                dso(ObjectRef::Nebula(i as u32), n.pos_pc, n.radius_pc, n.abs_mag);
            }
        }
    }
    for &(obj, m) in &sky_cache.1 {
        let pos_pc = match obj {
            ObjectRef::Star(i) => cat.stars[i as usize].pos_pc,
            ObjectRef::Galaxy(i) => cat.galaxies[i as usize].pos_pc,
            ObjectRef::Cluster(i) => cat.clusters[i as usize].pos_pc,
            ObjectRef::Nebula(i) => cat.nebulae[i as usize].pos_pc,
            ObjectRef::Body(_) => continue,
        };
        let delta = (pos_pc * METERS_PER_PC) - cam.pos.to_meters();
        let dir = delta.normalize_or(DVec3::Z).as_vec3() * 1.0e6;
        let Some(screen) = project(camera, &cam_tf, dir) else { continue };
        if occluded(screen, f64::INFINITY) || ray_blocked(delta.normalize_or(DVec3::Z), f64::INFINITY) {
            continue;
        }
        // Prefer a star's proper name (its first alias without digits) for the label.
        let label = match obj {
            ObjectRef::Star(i) => {
                let s = &cat.stars[i as usize];
                s.names.iter().find(|n| !n.chars().any(|c| c.is_ascii_digit())).unwrap_or(&s.names[0]).clone()
            }
            _ => cat.display_name(obj).to_string(),
        };
        markers.items.push(Marker { obj, screen, label, priority: -m, kind: MarkerKind::of(&data, obj), small: false });
    }
    markers
        .items
        .sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap_or(std::cmp::Ordering::Equal));
}

/// Labels (right of the object, SE's `LabelsPosPlanet`/`LabelsPosDSO` 2) and markers (circles
/// `MarkersSize` across), in the skin's colour for each kind. The brief mode leaves out minor bodies
/// that look tiny, the detail mode labels them too (assumed: what SE's two modes differ in is
/// unverified). Labels that would overlap a brighter object's are left out.
fn draw_labels(
    mut contexts: EguiContexts,
    data: Res<SeData>,
    settings: Res<OverlaySettings>,
    markers: Res<ScreenMarkers>,
    cam: Res<FlyCamera>,
) -> Result {
    if settings.labels_mode == 0 && !settings.markers {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let skin = crate::skin::text(ctx);
    let painter = ctx.layer_painter(overlay_layer());
    let mut placed: Vec<egui::Rect> = Vec::new();
    let font = egui::FontId::proportional(13.0);
    for m in markers.items.iter().take(400) {
        let selected = cam.selected == Some(m.obj);
        let label =
            settings.labels_mode > 0 && settings.label_kinds.has(m.kind) && (settings.labels_mode == 2 || !m.small || selected);
        let marker = settings.markers && settings.marker_kinds.has(m.kind) && !(selected && settings.selection_pointer);
        if !label && !marker {
            continue;
        }
        let p = egui::pos2(m.screen.x, m.screen.y);
        let color = skin.kind(color_key(&data, m.obj, m.kind));
        if marker && m.kind == MarkerKind::Cluster {
            // 02-013: SE marks star clusters with diamonds ("green diamonds highlight star clusters",
            // the tutorial's locale text), in the skin's cluster colour.
            let h = MARKER_PX * 0.5;
            let pts = vec![p + egui::vec2(0.0, -h), p + egui::vec2(h, 0.0), p + egui::vec2(0.0, h), p + egui::vec2(-h, 0.0)];
            painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.0, color)));
        } else if marker {
            painter.circle_stroke(p, MARKER_PX * 0.5, egui::Stroke::new(1.0, color));
        }
        if label {
            let galley = painter.layout_no_wrap(m.label.clone(), font.clone(), color);
            let rect = egui::Rect::from_min_size(p + egui::vec2(MARKER_PX * 0.5 + 4.0, -galley.size().y * 0.5), galley.size());
            if !selected && placed.iter().any(|r| r.expand(2.0).intersects(rect)) {
                continue;
            }
            placed.push(rect);
            painter.galley(rect.min, galley, color);
        }
    }
    Ok(())
}

/// SE's selection pointer (Filter objects > "Selection pointer", on by default): four tapered
/// cyan ticks turning around the selection, pointing at it, just outside its disc when it is
/// resolved; none once the disc fills much of the view; a selection off screen gets a hollow
/// triangle on the screen's edge pointing towards it. Compared with SE screenshots.
fn draw_selection_pointer(
    mut contexts: EguiContexts,
    settings: Res<OverlaySettings>,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    sim: Res<SimTime>,
    time: Res<Time>,
    cams: Query<&Camera, With<MainCamera>>,
) -> Result {
    let Some(sel) = cam.selected.filter(|_| settings.selection_pointer) else { return Ok(()) };
    let Ok(camera) = cams.single() else { return Ok(()) };
    let Some(vp) = camera.logical_viewport_size() else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let eph = Ephemeris::new(&data.catalog);
    let delta = eph.position(sel, sim.jd).delta_m(cam.pos);
    let dist = delta.length();
    if dist <= 0.0 {
        return Ok(());
    }
    let radius = eph.radius_m(sel);
    let focal = vp.y * 0.5 / (cam.fov_deg.to_radians() as f32 * 0.5).tan();
    let ang = if radius >= dist { std::f32::consts::FRAC_PI_2 } else { (radius / dist).asin() as f32 };
    let r_px = focal * ang.tan().min(1.0e4);
    if r_px > 0.45 * vp.min_element() {
        return Ok(());
    }
    let color = egui::Color32::from_rgb(80, 208, 216);
    let tip = egui::Color32::from_rgb(180, 245, 250);
    let painter = ctx.layer_painter(overlay_layer());
    let cam_tf = GlobalTransform::from(Transform::from_rotation(cam.rot.as_quat()));
    let dir = delta / dist;
    match project(camera, &cam_tf, dir.as_vec3() * 1.0e6) {
        Some(s) if s.x >= 0.0 && s.y >= 0.0 && s.x <= vp.x && s.y <= vp.y => {
            let c = egui::pos2(s.x, s.y);
            let r_in = (r_px + 4.0).max(7.0);
            let spin = time.elapsed_secs() * 0.8;
            for k in 0..4 {
                let a = spin + k as f32 * std::f32::consts::FRAC_PI_2;
                let u = egui::vec2(a.cos(), a.sin());
                let n = egui::vec2(-u.y, u.x);
                let (p_in, p_out) = (c + u * r_in, c + u * (r_in + 10.0));
                let pts = vec![p_in + n * 0.6, p_out + n * 2.2, p_out - n * 2.2, p_in - n * 0.6];
                painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
                painter.circle_filled(p_in + u * 1.2, 1.1, tip);
            }
        }
        _ => {
            // Direction on screen from the camera frame (x right, y up): at the edge, inset.
            let v = cam.rot.inverse() * dir;
            let d = egui::vec2(v.x as f32, -v.y as f32);
            let d = if d.length() < 1e-6 { egui::vec2(0.0, 1.0) } else { d.normalized() };
            let half = egui::vec2(vp.x, vp.y) * 0.5 - egui::vec2(24.0, 24.0);
            let t = (half.x / d.x.abs().max(1e-6)).min(half.y / d.y.abs().max(1e-6));
            let p = egui::pos2(vp.x * 0.5, vp.y * 0.5) + d * t;
            let n = egui::vec2(-d.y, d.x);
            let pts = vec![p + d * 9.0, p - d * 6.0 + n * 8.0, p - d * 6.0 - n * 8.0];
            painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.6, color)));
        }
    }
    Ok(())
}

/// Click radius around point-like objects, logical pixels.
const PICK_PX: f64 = 10.0;

/// What a click on empty sky hits besides the labelled markers.
enum Picked {
    Object(ObjectRef),
    /// A procedural star that is not in the catalog yet.
    Proc(ProcStarInfo, DVec3),
    /// A procedural galaxy that is not in the catalog yet.
    Galaxy(ProcGalaxy),
}

/// The object under the cursor: the brightest visible star within [`PICK_PX`], else the brightest
/// galaxy drawn as a dot within it, else the smallest galaxy, cluster or nebula whose disc contains
/// the cursor (SpaceEngine selects any visible object this way, not only labelled ones).
/// `galaxy_mag_limits`: [`far_mag_limits`].
#[allow(clippy::too_many_arguments)]
fn pick_in_sky(
    ray: DVec3,
    rad_per_px: f64,
    cam_pc: DVec3,
    data: &SeData,
    mag_limit: f64,
    field: Option<&ProcStarField>,
    clusters: Option<&ClusterField>,
    galaxies: Option<&ProcGalaxyField>,
    galaxy_mag_limits: [f64; 4],
    cluster_shown: &dyn Fn(&se_core::catalog::ClusterRecord) -> bool,
) -> Option<Picked> {
    let cat = &data.catalog;
    let cos_max = (PICK_PX * rad_per_px).cos();
    // Is the direction to `p` within the click radius? Returns its distance (pc).
    let near_ray = |p: DVec3| {
        let d = p - cam_pc;
        let len = d.length();
        (len > 0.0 && d.dot(ray) > len * cos_max).then_some(len)
    };
    let app = |abs: f64, d: f64| se_core::star::app_mag_from_abs(abs, d.max(1e-9));
    let mut star: Option<(f64, Picked)> = None;
    let mut offer = |m: f64, pick: &dyn Fn() -> Picked| {
        if m < mag_limit && star.as_ref().is_none_or(|(bm, _)| m < *bm) {
            star = Some((m, pick()));
        }
    };
    for (i, s) in cat.stars.iter().enumerate() {
        if s.barycenter || !s.has_position || s.hidden() {
            continue;
        }
        if let Some(d) = near_ray(s.pos_pc) {
            offer(app(s.abs_mag, d), &|| Picked::Object(ObjectRef::Star(i as u32)));
        }
    }
    if let Some(field) = field {
        for (p, s, key) in field.loaded_stars() {
            if let Some(d) = near_ray(p) {
                offer(app(s.abs_mag as f64, d), &|| Picked::Proc(field.info(s, key), p));
            }
        }
    }
    if let Some(clusters) = clusters {
        for (p, m, id) in clusters.members(cat).filter(|(_, m, _)| !m.hidden) {
            if let Some(d) = near_ray(p) {
                offer(app(m.abs_mag as f64, d), &|| Picked::Proc(clusters.member_info(cat, id, m), p));
            }
        }
    }
    if let Some((_, pick)) = star {
        return Some(pick);
    }
    // Extended objects: hit inside their apparent disc (at least the point radius); the smallest
    // one wins. Galaxies under a pixel are drawn as dots of a minimum size: those whose sprite is
    // drawn at all are hit like stars and beat the discs, the brightest first. Returns
    // `(is a dot, apparent magnitude or angular radius)`.
    let point = PICK_PX * rad_per_px;
    let hit = |pos: DVec3, radius_pc: f64, galaxy: Option<(f64, u8)>| {
        let d = pos - cam_pc;
        let len = d.length();
        if len <= radius_pc || len == 0.0 {
            return None;
        }
        let ang_r = (radius_pc / len).asin();
        let off = (d.dot(ray) / len).clamp(-1.0, 1.0).acos();
        if ang_r > rad_per_px {
            return (off < ang_r.max(point)).then_some((false, ang_r));
        }
        let (abs_mag, ptype) = galaxy?;
        let m = app(abs_mag, len);
        (off < point && m <= galaxy_mag_limits[ptype as usize]).then_some((true, m))
    };
    let catalog_galaxies = cat.galaxies.iter().enumerate().map(|(i, g)| {
        (g.pos_pc, g.radius_pc, Some((g.vis_mag, profile_type(g))), Hit::Obj(ObjectRef::Galaxy(i as u32)))
    });
    // Every drawn procedural galaxy, not only the ones already big enough to be in the catalog.
    let proc_galaxies = galaxies.into_iter().flat_map(|f| f.unpromoted(cam_pc)).map(|g| {
        let ptype = match g.hubble {
            0..=8 => 0,
            9..=15 => 1,
            _ => 2,
        };
        (g.pos_pc, g.display_radius_pc(), Some((g.vis_abs_mag() as f64, ptype)), Hit::Gal(g))
    });
    let clusters = cat
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| cluster_shown(c))
        .map(|(i, c)| (c.pos_pc, c.radius_pc, None, Hit::Obj(ObjectRef::Cluster(i as u32))));
    let nebulae = cat.nebulae.iter().enumerate().map(|(i, n)| (n.pos_pc, n.radius_pc, None, Hit::Obj(ObjectRef::Nebula(i as u32))));
    let best = catalog_galaxies
        .chain(proc_galaxies)
        .chain(clusters)
        .chain(nebulae)
        .filter_map(|(p, r, g, h)| hit(p, r, g).map(|a| (a, h)))
        // Dots before discs, then the smaller key.
        .min_by(|a, b| b.0.0.cmp(&a.0.0).then(a.0.1.total_cmp(&b.0.1)));
    best.map(|(_, h)| match h {
        Hit::Obj(o) => Picked::Object(o),
        Hit::Gal(g) => Picked::Galaxy(g.clone()),
    })
}

/// An extended object under the cursor.
enum Hit<'a> {
    Obj(ObjectRef),
    Gal(&'a ProcGalaxy),
}

#[allow(clippy::too_many_arguments)]
fn pick_on_click(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    markers: Res<ScreenMarkers>,
    mut click: ResMut<ClickState>,
    mut cam: ResMut<FlyCamera>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    mut egui: EguiContexts,
    mut data: ResMut<SeData>,
    cameras: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    star_settings: Res<StarSettings>,
    (field, clusters, mut galaxies): (Option<Res<ProcStarField>>, Option<Res<ClusterField>>, Option<ResMut<ProcGalaxyField>>),
    (gal_settings, gal_state, cluster_filter, proc_settings): (
        Res<GalaxySettings>,
        Option<Res<GalaxyState>>,
        Res<crate::clusters::ClusterSettings>,
        Res<crate::procgalaxies::ProcSettings>,
    ),
    mut script: ResMut<CommandQueue>,
) {
    let over_ui = egui.ctx_mut().map(|c| crate::hud::pointer_over_ui(c)).unwrap_or(false);
    let Ok(window) = windows.single() else { return };
    if buttons.just_pressed(MouseButton::Left) && !over_ui {
        click.press_pos = window.cursor_position();
        click.moved = 0.0;
    }
    if buttons.pressed(MouseButton::Left) {
        click.moved += motion.delta.length();
    }
    // A short click, or a scripted `click X Y`.
    let mut at = script.click.take();
    if buttons.just_released(MouseButton::Left) {
        if let Some(pos) = click.press_pos.take() {
            if click.moved < 4.0 {
                at = Some(pos);
            }
        }
    }
    let Some(pos) = at else { return };
    let best = markers
        .items
        .iter()
        .map(|m| (m, m.screen.distance(pos)))
        .filter(|(_, d)| *d < 14.0)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    if let Some((m, _)) = best {
        cam.selected = Some(m.obj);
        return;
    }
    let Ok((camera, cam_tf)) = cameras.single() else { return };
    let Ok(ray) = camera.viewport_to_world(cam_tf, pos) else { return };
    let rad_per_px = (cam.fov_deg as f64).to_radians() / window.height().max(1.0) as f64;
    // Render space is the universe frame around the camera.
    let picked = pick_in_sky(
        ray.direction.as_dvec3(),
        rad_per_px,
        cam.pos.to_parsecs(),
        &data,
        star_settings.faint_limit() as f64,
        field.as_deref(),
        clusters.as_deref(),
        galaxies.as_deref(),
        far_mag_limits(
            &gal_settings,
            gal_state.map_or(1.0, |s| s.gain),
            window.resolution.physical_height() as f64 * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan(),
        ),
        &|c| cluster_filter.shows(c, proc_settings.clusters),
    );
    match picked {
        Some(Picked::Object(o)) => cam.selected = Some(o),
        Some(Picked::Proc(info, pos_pc)) => {
            // Make it a real star (with its planets) so Go to and the info panel work.
            let idx = match data.catalog.find(&info.name) {
                Some(ObjectRef::Star(i)) => i,
                _ => {
                    let rec = procedural_star_record(&info, pos_pc);
                    materialize_star(data.catalog_mut(), rec)
                }
            };
            info!("picked {}", data.catalog.stars[idx as usize].names[0]);
            cam.selected = Some(ObjectRef::Star(idx));
        }
        Some(Picked::Galaxy(g)) => {
            if let Some(field) = galaxies.as_deref_mut() {
                let idx = promote_galaxy(&mut data, field, &g);
                info!("picked {}", g.name());
                cam.selected = Some(ObjectRef::Galaxy(idx));
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_conics() {
        let mu: f64 = 3.986e14;
        // A circular orbit at 7000 km: every point on the circle, starting at the ship.
        let r = DVec3::new(7.0e6, 0.0, 0.0);
        let v = DVec3::new(0.0, 0.0, (mu / 7.0e6).sqrt());
        let c = ship_conic(r, v, mu, 6.371e6, 64).unwrap();
        assert!((c[0].0 - r).length() < 1.0 && c[0].1 == 0.0 && c[64].1 == 1.0);
        assert!(c.iter().all(|(p, _)| (p.length() - 7.0e6).abs() < 1.0));
        // An escape path: through the ship, within 20 periapses, fractions rising from it.
        let c = ship_conic(r, v * 1.6, mu, 6.371e6, 64).unwrap();
        assert!((c[0].0 - r).length() < 1.0);
        assert!(c.iter().all(|(p, _)| p.length() <= 20.0 * 7.0e6 * 1.001));
        assert!(c.windows(2).all(|w| w[1].1 >= w[0].1));
        // Falling straight down has no conic to draw.
        assert!(ship_conic(r, -r, mu, 6.371e6, 64).is_none());
    }

    #[test]
    fn lines_behind_the_camera_are_cut() {
        // Ahead (-z), behind, ahead again: two runs, each ending just short of the camera's plane.
        let pts = [(DVec3::new(-1.0, 0.0, -10.0), 1.0), (DVec3::new(0.0, 0.0, 10.0), 1.0), (DVec3::new(1.0, 0.0, -10.0), 1.0)];
        let runs = clip_runs(&pts);
        assert_eq!(runs.len(), 2);
        for run in &runs {
            assert_eq!(run.len(), 2);
            assert!(run.iter().all(|(v, _)| -v.z >= 0.99e-2 * v.length()));
        }
    }

    #[test]
    fn filters_by_se_key() {
        let mut s = OverlaySettings::default();
        assert!(!s.label_kinds.has(MarkerKind::Galaxy));
        let (set, kind) = kind_filter(&mut s, "showlabelsgalaxies").unwrap();
        set.set(kind, true);
        assert!(s.label_kinds.has(MarkerKind::Galaxy));
        assert!(s.orbit_kinds.has(MarkerKind::Planet) && !s.orbit_kinds.has(MarkerKind::Asteroid));
        assert!(kind_filter(&mut s, "showlabelsfoo").is_none());
    }
}
