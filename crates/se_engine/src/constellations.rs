//! Constellations: SE's lines, IAU boundaries and names from `catalogs/stars/Constellations.cfg`
//! (Catalogs.pak; loaded by FUN_140261780, one block parsed by FUN_140262d40). Each block has
//! `Zodiacal`, `LabelPos (RA h, Dec deg)`, `Lines { "star" "star" ... }` and `Boundary { (RA h, Dec
//! deg) ... }`; a line's stars are looked up by name in the star catalog (the part before a `/`).
//!
//! As SE draws them (OverlayFilters `ShowConstellations` and its `ShowConstel*` switches):
//! - lines (FUN_1402603f0) join the stars where they are, so from another system they take other
//!   shapes; they fade out between `ConstelLinesFadeStart` and `ConstelLinesFadeEnd` (4000-10000 pc)
//!   from the Sun;
//! - boundaries (FUN_14025fd90) lie at infinity (`ConstelBordersAtInf`), else on a sphere of
//!   `ConstelBordersRadius` (10 pc) round the Sun, and fade out between `ConstelBordersFadeStart` and
//!   `ConstelBordersFadeEnd` (200-800 pc);
//! - names (FUN_14025f7b0) sit at `LabelPos` at infinity, centred, fading as the boundaries: the
//!   full name in the interface language (`FullConstellationNames`, the IAU names are its locale
//!   keys) or the abbreviation;
//!
//! in the skin's `ConstelLinesColor`, `ConstelBoundariesColor` and `ConstelLabelsColor` with SE's
//! prim_line.glsl Gaussian lines (`LinesWidth`). The fade of each part is the distance fade times the
//! master and the part's switch (squared smooth switches), times the constellation's own switch.
//! Not ported: constellation art (`ShowConstelDrawings`: SE 0.991 ships no art), the exposure
//! factor `ExposureCalibration / Exposure` SE puts on the line colours (the overlay is drawn after
//! tone mapping here).

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{Catalog, ObjectRef};
use se_core::coords::radec_to_universe;

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::overlay::{Occluders, OverlaySettings, PrimLines, SkyConfig, SkyFade, SkyPath, View, ViewCone, overlay_layer};
use crate::system::SystemBody;

/// The constellations in Constellations.cfg's order: its abbreviation (Serpens in two parts, as the
/// file has it) and the IAU name, which is the key of its translation in the `*-gui.cfg` locales
/// (two with a `#nominative` context).
pub const IAU: [(&str, &str); 89] = [
    ("And", "Andromeda"),
    ("Ant", "Antlia"),
    ("Aps", "Apus"),
    ("Aqr", "Aquarius"),
    ("Aql", "Aquila"),
    ("Ara", "Ara"),
    ("Ari", "Aries"),
    ("Aur", "Auriga"),
    ("Boo", "Bootes"),
    ("Cae", "Caelum"),
    ("Cam", "Camelopardalis#nominative"),
    ("Cnc", "Cancer"),
    ("CVn", "Canes Venatici"),
    ("CMa", "Canis Major"),
    ("CMi", "Canis Minor"),
    ("Cap", "Capricornus"),
    ("Car", "Carina"),
    ("Cas", "Cassiopeia"),
    ("Cen", "Centaurus"),
    ("Cep", "Cepheus"),
    ("Cet", "Cetus"),
    ("Cha", "Chamaeleon"),
    ("Cir", "Circinus"),
    ("Col", "Columba"),
    ("Com", "Coma Berenices"),
    ("CrA", "Corona Australis"),
    ("CrB", "Corona Borealis"),
    ("Crv", "Corvus"),
    ("Crt", "Crater"),
    ("Cru", "Crux"),
    ("Cyg", "Cygnus"),
    ("Del", "Delphinus"),
    ("Dor", "Dorado"),
    ("Dra", "Draco"),
    ("Equ", "Equuleus"),
    ("Eri", "Eridanus"),
    ("For", "Fornax"),
    ("Gem", "Gemini"),
    ("Gru", "Grus"),
    ("Her", "Hercules"),
    ("Hor", "Horologium"),
    ("Hya", "Hydra"),
    ("Hyi", "Hydrus"),
    ("Ind", "Indus"),
    ("Lac", "Lacerta"),
    ("Leo", "Leo"),
    ("LMi", "Leo Minor"),
    ("Lep", "Lepus"),
    ("Lib", "Libra"),
    ("Lup", "Lupus"),
    ("Lyn", "Lynx"),
    ("Lyr", "Lyra"),
    ("Men", "Mensa"),
    ("Mic", "Microscopium"),
    ("Mon", "Monoceros"),
    ("Mus", "Musca"),
    ("Nor", "Norma"),
    ("Oct", "Octans"),
    ("Oph", "Ophiuchus"),
    ("Ori", "Orion"),
    ("Pav", "Pavo"),
    ("Peg", "Pegasus"),
    ("Per", "Perseus"),
    ("Phe", "Phoenix"),
    ("Pic", "Pictor"),
    ("Psc", "Pisces"),
    ("PsA", "Piscis Austrinus"),
    ("Pup", "Puppis#nominative"),
    ("Pyx", "Pyxis"),
    ("Ret", "Reticulum"),
    ("Sge", "Sagitta"),
    ("Sgr", "Sagittarius"),
    ("Sco", "Scorpius"),
    ("Scl", "Sculptor"),
    ("Sct", "Scutum"),
    ("SerCap", "Serpens Caput"),
    ("SerCau", "Serpens Cauda"),
    ("Sex", "Sextans"),
    ("Tau", "Taurus"),
    ("Tel", "Telescopium"),
    ("Tri", "Triangulum"),
    ("TrA", "Triangulum Australe"),
    ("Tuc", "Tucana"),
    ("UMa", "Ursa Major"),
    ("UMi", "Ursa Minor"),
    ("Vel", "Vela"),
    ("Vir", "Virgo"),
    ("Vol", "Volans"),
    ("Vul", "Vulpecula"),
];

const CFG_PATH: &str = "catalogs/stars/Constellations.cfg";

/// SE's colours when the skin has none (FUN_14036a560).
const LINES_COLOR: [f32; 4] = [0.4, 0.4, 0.4, 1.0];
const BOUNDARIES_COLOR: [f32; 4] = [0.0, 0.0, 0.5, 1.0];
const LABELS_COLOR: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// One `Constellation` block of Constellations.cfg.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConstellationDef {
    pub abbr: String,
    pub zodiacal: bool,
    /// `LabelPos`: (RA hours, Dec degrees).
    pub label_pos: (f64, f64),
    /// `Lines`: pairs of star names.
    pub lines: Vec<(String, String)>,
    /// `Boundary`: (RA hours, Dec degrees) round the constellation.
    pub boundary: Vec<(f64, f64)>,
}

/// The numbers of a `( a, b )` row.
fn pair(s: &str) -> Option<(f64, f64)> {
    let mut it = s.trim_matches(|c: char| c == '(' || c == ')' || c.is_whitespace()).split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).map(|t| t.parse::<f64>().ok());
    Some((it.next()??, it.next()??))
}

/// Constellations.cfg's blocks, in file order.
pub fn parse_constellations(text: &str) -> Vec<ConstellationDef> {
    #[derive(PartialEq, Clone, Copy)]
    enum Part {
        None,
        Lines,
        Boundary,
    }
    let (mut out, mut part, mut pending) = (Vec::<ConstellationDef>::new(), Part::None, Part::None);
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(k, r)| (k, r.trim()));
        match (part, key) {
            (_, "{") => part = pending,
            (_, "}") => {
                part = Part::None;
                pending = Part::None;
            }
            (Part::Lines, _) => {
                let names: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
                if let Some(c) = out.last_mut().filter(|_| names.len() >= 2) {
                    c.lines.push((names[0].to_string(), names[1].to_string()));
                }
            }
            (Part::Boundary, _) => {
                if let (Some(c), Some(p)) = (out.last_mut(), pair(line)) {
                    c.boundary.push(p);
                }
            }
            (_, "Constellation") => out.push(ConstellationDef { abbr: rest.trim_matches('"').to_string(), ..default() }),
            (_, "Zodiacal") => {
                if let Some(c) = out.last_mut() {
                    c.zodiacal = rest.eq_ignore_ascii_case("true");
                }
            }
            (_, "LabelPos") => {
                if let (Some(c), Some(p)) = (out.last_mut(), pair(rest)) {
                    c.label_pos = p;
                }
            }
            (_, "Lines") => pending = Part::Lines,
            (_, "Boundary") => pending = Part::Boundary,
            _ => {}
        }
    }
    out
}

/// A constellation ready to draw.
pub struct Constellation {
    /// Index in [`IAU`].
    pub iau: usize,
    pub label_dir: DVec3,
    /// The lines' stars, parsecs from the Sun.
    pub lines: Vec<(DVec3, DVec3)>,
    /// Directions round the boundary, closed (the first again at the end).
    pub boundary: SkyPath,
}

#[derive(Resource, Default)]
pub struct Constellations(pub Vec<Constellation>);

/// A line's star in the catalog, by the part of its name before a `/` (as FUN_140262d40).
fn star_pos(cat: &Catalog, name: &str) -> Option<DVec3> {
    match cat.find(name.split('/').next()?.trim())? {
        ObjectRef::Star(i) => cat.stars.get(i as usize).filter(|s| s.has_position).map(|s| s.pos_pc),
        _ => None,
    }
}

/// Resolves the parsed blocks against the star catalog; unknown abbreviations and stars are left
/// out (SE logs them).
pub fn build(defs: &[ConstellationDef], cat: &Catalog) -> (Vec<Constellation>, usize) {
    let mut missing = 0;
    let list = defs
        .iter()
        .filter_map(|d| {
            let iau = IAU.iter().position(|(a, _)| *a == d.abbr)?;
            let lines = d
                .lines
                .iter()
                .filter_map(|(a, b)| {
                    let p = star_pos(cat, a).zip(star_pos(cat, b));
                    missing += p.is_none() as usize;
                    p
                })
                .collect();
            let mut boundary: Vec<DVec3> = d.boundary.iter().map(|&(ra, dec)| radec_to_universe(ra, dec)).collect();
            if let Some(&first) = boundary.first() {
                boundary.push(first);
            }
            Some(Constellation { iau, label_dir: radec_to_universe(d.label_pos.0, d.label_pos.1), lines, boundary: SkyPath::new(boundary) })
        })
        .collect();
    (list, missing)
}

/// The zodiacal constellations as a [`crate::overlay::SkySwitches::zodiac`] mask.
pub fn zodiac_mask(defs: &[ConstellationDef]) -> u128 {
    defs.iter().filter(|d| d.zodiacal).filter_map(|d| IAU.iter().position(|(a, _)| *a == d.abbr)).fold(0, |m, i| m | 1 << i)
}

/// A constellation's name as SE labels it: the IAU name in the interface language, else the
/// abbreviation (one `Ser` for both parts of Serpens).
pub fn label(i18n: &crate::i18n::I18n, iau: usize, full: bool) -> String {
    let (abbr, name) = IAU[iau];
    if !full {
        return if abbr.starts_with("Ser") { "Ser".into() } else { abbr.into() };
    }
    let t = i18n.tr(name);
    t.split('#').next().unwrap_or(t).to_string()
}

/// 1 up to `start`, 0 from `end` (SE's `(3 - 2t) t^2` of `(d - end) / (start - end)`).
pub fn distance_fade(d: f64, (start, end): (f64, f64)) -> f32 {
    let t = ((d - end) / (start - end)).clamp(0.0, 1.0) as f32;
    (3.0 - 2.0 * t) * t * t
}

pub struct ConstellationsPlugin;

impl Plugin for ConstellationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Constellations>()
            .init_resource::<ReloadConstellations>()
            .add_systems(Startup, load_constellations)
            .add_systems(Update, load_constellations.run_if(|r: Res<ReloadConstellations>| r.0))
            .add_systems(
            EguiPrimaryContextPass,
            draw_constellations.in_set(crate::skin::UiSet::Draw).after(crate::grids::draw_grids).before(crate::overlay::draw_orbits),
        );
    }
}

/// Set by the script command `ReloadConstellations` (FUN_140551c40): the constellations are read again.
#[derive(Resource, Default)]
pub struct ReloadConstellations(pub bool);

fn load_constellations(
    data: Res<SeData>,
    mut cons: ResMut<Constellations>,
    mut settings: ResMut<OverlaySettings>,
    mut reload: ResMut<ReloadConstellations>,
) {
    reload.0 = false;
    let text = match data.vfs.read_string(CFG_PATH) {
        Ok(t) => t,
        Err(e) => {
            warn!("constellations: {e}");
            return;
        }
    };
    let defs = parse_constellations(&text);
    let (list, missing) = build(&defs, &data.catalog);
    settings.sky.zodiac = zodiac_mask(&defs);
    info!("{} constellations loaded ({} lines, {missing} with unknown stars)", list.len(), list.iter().map(|c| c.lines.len()).sum::<usize>());
    cons.0 = list;
}

fn rgb(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

/// SE's line samples are the stars themselves (GL lines); here a line is cut into ~1 degree steps
/// so that the parts behind a body (the ground) can be left out.
const STEP_RAD: f64 = 0.0175;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_constellations(
    mut contexts: EguiContexts,
    settings: Res<OverlaySettings>,
    fade: Res<SkyFade>,
    cfg: Option<Res<SkyConfig>>,
    cons: Res<Constellations>,
    cam: Res<FlyCamera>,
    cams: Query<&Camera, With<MainCamera>>,
    bodies: Query<&SystemBody>,
    i18n: Res<crate::i18n::I18n>,
) -> Result {
    let master = fade.constellations * fade.constellations;
    if master <= 0.0 || cons.0.is_empty() {
        return Ok(());
    }
    let Ok(camera) = cams.single() else { return Ok(()) };
    let Some(vp) = camera.logical_viewport_size() else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let skin = crate::skin::text(ctx);
    let cfg = cfg.as_deref().copied().unwrap_or_default();
    let view = View::new(&cam, vp);
    let occ = Occluders::new(bodies.iter(), &cam);
    // Camera space, and 0 where a body hides the point (points behind the camera are cut anyway).
    let local = |v: DVec3, a: f32| {
        let l = view.local(v);
        (l, if l.z < 0.0 && occ.hides_dir(v) { 0.0 } else { a })
    };
    let cam_pc = cam.pos.to_parsecs();
    let d_sun = cam_pc.length();
    let sq = |v: f32| v * v;
    let lines_f = distance_fade(d_sun, cfg.lines_fade) * sq(fade.constel[0]) * master;
    let borders_f = distance_fade(d_sun, cfg.borders_fade) * master;
    let (bound_f, label_f) = (borders_f * sq(fade.constel[1]), borders_f * sq(fade.constel[2]));
    let mut mesh = PrimLines::new(cfg.lines_width, ctx.pixels_per_point(), vp);
    let mut pts: Vec<(DVec3, f32)> = Vec::new();
    let cone = ViewCone::new(&cam, &view);
    if lines_f > 0.0 {
        // Width.z = the fade F, Color.a = F times the constellation's switch: alpha g F^2 s^2 and
        // colour F g rgb (FUN_1402603f0; the boundaries likewise, FUN_14025fd90).
        let color = rgb(skin.rgba("ConstelLinesColor", LINES_COLOR));
        for c in &cons.0 {
            let a = lines_f * lines_f * sq(fade.per_constel[0][c.iau]);
            if a <= 0.0 {
                continue;
            }
            for &(p, q) in &c.lines {
                let (p, q) = (p - cam_pc, q - cam_pc);
                let ang = p.angle_between(q);
                // The line's directions run along the arc between its ends.
                let axis = (p.normalize_or_zero() + q.normalize_or_zero()).normalize_or(p.normalize_or_zero());
                if !cone.sees(axis, 0.5 * ang) {
                    continue;
                }
                let n = ((ang / STEP_RAD).ceil() as usize).clamp(1, 180);
                pts.clear();
                pts.extend((0..=n).map(|k| local(p.lerp(q, k as f64 / n as f64), a)));
                mesh.polyline(&view, &pts, color, lines_f);
            }
        }
    }
    if bound_f > 0.0 {
        let color = rgb(skin.rgba("ConstelBoundariesColor", BOUNDARIES_COLOR));
        let r = cfg.borders_radius_pc;
        for c in &cons.0 {
            let a = bound_f * bound_f * sq(fade.per_constel[1][c.iau]);
            if a <= 0.0 {
                continue;
            }
            if settings.sky.borders_at_inf {
                for range in c.boundary.visible(&cone) {
                    pts.clear();
                    pts.extend(c.boundary.pts[range].iter().map(|&dir| local(dir, a)));
                    mesh.polyline(&view, &pts, color, bound_f);
                }
            } else {
                pts.clear();
                pts.extend(c.boundary.pts.iter().map(|&dir| local(dir * r - cam_pc, a)));
                mesh.polyline(&view, &pts, color, bound_f);
            }
        }
    }
    let painter = ctx.layer_painter(overlay_layer());
    if !mesh.mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh.mesh));
    }
    if label_f > 0.0 {
        let [r, g, b, a] = skin.rgba("ConstelLabelsColor", LABELS_COLOR).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let base = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
        let font = egui::FontId::proportional(13.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(vp.x, vp.y));
        for c in &cons.0 {
            let f = label_f * sq(fade.per_constel[2][c.iau]);
            let Some(p) = view.project(view.local(c.label_dir)).filter(|p| f > 0.0 && screen.expand(100.0).contains(*p)) else { continue };
            if occ.hides_dir(c.label_dir) {
                continue;
            }
            painter.text(p, egui::Align2::CENTER_CENTER, label(&i18n, c.iau, settings.sky.full_names), font.clone(), base.gamma_multiply(f));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Constellation	"And"
{
	Zodiacal false

	LabelPos (1.395, 42.96)

	Lines
	{
		"GAM And"	"BET And"
		"KSI Ser"	"ETA Oph"	// moved from Oph
	}

	Boundary
	{
		( 23.5357132, +35.1897736 ) // AND  I
		(  0.0229885, -48.6960831 ) // AND  O
	}
}

Constellation	"Ari"
{
	Zodiacal true
	LabelPos (2.6, 20.5)
	Lines
	{
		"ALF Ari"	"BET Ari"
	}
	Boundary
	{
		( 1.0, 10.0 )
	}
}
"#;

    #[test]
    fn parses_blocks() {
        let defs = parse_constellations(SAMPLE);
        assert_eq!(defs.len(), 2);
        let and = &defs[0];
        assert_eq!(and.abbr, "And");
        assert!(!and.zodiacal && defs[1].zodiacal);
        assert_eq!(and.label_pos, (1.395, 42.96));
        assert_eq!(and.lines, vec![("GAM And".into(), "BET And".into()), ("KSI Ser".into(), "ETA Oph".into())]);
        assert_eq!(and.boundary, vec![(23.5357132, 35.1897736), (0.0229885, -48.6960831)]);
        assert_eq!(zodiac_mask(&defs), 1 << 6);
    }

    #[test]
    fn iau_table_matches_the_file_order() {
        assert_eq!(IAU.len(), 89);
        let abbrs: std::collections::HashSet<_> = IAU.iter().map(|(a, _)| a.to_ascii_lowercase()).collect();
        assert_eq!(abbrs.len(), 89);
        assert_eq!(IAU[6].0, "Ari");
        assert_eq!(IAU[59], ("Ori", "Orion"));
    }

    #[test]
    fn fades_with_distance() {
        assert_eq!(distance_fade(10.0, (200.0, 800.0)), 1.0);
        assert_eq!(distance_fade(900.0, (200.0, 800.0)), 0.0);
        assert!((distance_fade(500.0, (200.0, 800.0)) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn script_vars_switch_constellations() {
        let mut s = crate::overlay::SkySwitches { zodiac: 1 << 6, ..Default::default() };
        assert!(s.set_var("ConstLinesAll", false));
        assert!(!s.constel(0, 0) && s.constel(1, 0));
        assert!(s.set_var("constlineszodiac", true));
        assert!(s.constel(0, 6) && !s.constel(0, 5));
        assert!(s.set_var("ConstLabelOri", false));
        assert_eq!(s.var("ConstLabelOri"), Some(false));
        assert!(s.set_var("ShowConstellations", true) && s.constellations);
        assert!(!s.set_var("ShowSomethingElse", true));
    }
}
