//! Planet-surface texturing presets ("biome presets") and soil presets.
//!
//! SpaceEngine ships 225 preset files in `textures/terrain/samples/*.cfg` that say which
//! sample textures colour each biome (Lowland, Upland, Snow, ...) of a planet, and 6
//! `textures/terrain/materials/*.pml` files describing the "soils" (detail/modulation
//! texture pairs) a preset may use. This module parses both and implements the
//! preset choice SpaceEngine makes for a planet (`FindPresetForPlanet`):
//!
//! 1. A preset whose `UseFor` names the body (`Parent|Body` or `Body`) wins.
//! 2. Otherwise presets whose class lines (`BulkClass`, `TempClass`, `AtmoClass`,
//!    `VolqClass`, `LifeOrganic`, `LifeExotic`, `SunTemp`) match the body survive.
//! 3. The survivors are picked by weighted choice with the body's `SurfStyle` in [0,1).
//! 4. If nothing survives, constraints are relaxed (sun temperature, temperature,
//!    volatiles and atmosphere, life, finally bulk class) and a warning is logged.
//!
//! The file format is described in `docs/formats/biome_presets.md`. Parsing uses
//! [`crate::sc`], which already handles `Layer "Large" { 64 1.000 1.000 "X" }` as an
//! anonymous row of values inside the block.

use std::collections::{BTreeSet, HashMap};

use crate::sc::{self, Block, Item, Value};
use crate::vfs::Vfs;

const SAMPLES_DIR: &str = "textures/terrain/samples";
const MATERIALS_DIR: &str = "textures/terrain/materials";

// ---------------------------------------------------------------------------------------
// Biome kinds
// ---------------------------------------------------------------------------------------

/// One surface biome slot of a preset, in SpaceEngine's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BiomeKind {
    Bottom,
    Shelf,
    Beach,
    Lowland,
    Upland,
    Rock,
    Snow,
    Ice,
    Lava,
    SteppeOrgU,
    ForestOrgU,
    GrassOrgU,
    SteppeOrgM,
    ForestOrgM,
    GrassOrgM,
    SteppeExoU,
    ForestExoU,
    GrassExoU,
    SteppeExoM,
    ForestExoM,
    GrassExoM,
}

impl BiomeKind {
    pub const ALL: [BiomeKind; 21] = [
        BiomeKind::Bottom,
        BiomeKind::Shelf,
        BiomeKind::Beach,
        BiomeKind::Lowland,
        BiomeKind::Upland,
        BiomeKind::Rock,
        BiomeKind::Snow,
        BiomeKind::Ice,
        BiomeKind::Lava,
        BiomeKind::SteppeOrgU,
        BiomeKind::ForestOrgU,
        BiomeKind::GrassOrgU,
        BiomeKind::SteppeOrgM,
        BiomeKind::ForestOrgM,
        BiomeKind::GrassOrgM,
        BiomeKind::SteppeExoU,
        BiomeKind::ForestExoU,
        BiomeKind::GrassExoU,
        BiomeKind::SteppeExoM,
        BiomeKind::ForestExoM,
        BiomeKind::GrassExoM,
    ];

    /// The name used in `Biome "Name"` blocks.
    pub fn name(self) -> &'static str {
        match self {
            BiomeKind::Bottom => "Bottom",
            BiomeKind::Shelf => "Shelf",
            BiomeKind::Beach => "Beach",
            BiomeKind::Lowland => "Lowland",
            BiomeKind::Upland => "Upland",
            BiomeKind::Rock => "Rock",
            BiomeKind::Snow => "Snow",
            BiomeKind::Ice => "Ice",
            BiomeKind::Lava => "Lava",
            BiomeKind::SteppeOrgU => "SteppeOrgU",
            BiomeKind::ForestOrgU => "ForestOrgU",
            BiomeKind::GrassOrgU => "GrassOrgU",
            BiomeKind::SteppeOrgM => "SteppeOrgM",
            BiomeKind::ForestOrgM => "ForestOrgM",
            BiomeKind::GrassOrgM => "GrassOrgM",
            BiomeKind::SteppeExoU => "SteppeExoU",
            BiomeKind::ForestExoU => "ForestExoU",
            BiomeKind::GrassExoU => "GrassExoU",
            BiomeKind::SteppeExoM => "SteppeExoM",
            BiomeKind::ForestExoM => "ForestExoM",
            BiomeKind::GrassExoM => "GrassExoM",
        }
    }

    /// Case-insensitive lookup by biome name.
    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s))
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------------------

/// A texture layer of a biome: `{ freq a b "Texture" }`.
#[derive(Clone, Debug)]
pub struct BiomeLayer {
    /// Tiling frequency.
    pub freq: f32,
    pub a: f32,
    pub b: f32,
    /// Sample texture name without extension, e.g. `Desert003_COL`.
    pub texture: String,
}

#[derive(Clone, Debug)]
pub struct Biome {
    pub color: [f32; 4],
    pub large: Option<BiomeLayer>,
    pub small: Option<BiomeLayer>,
}

#[derive(Clone, Debug)]
pub struct BiomePreset {
    /// File name, e.g. `terra_arid_default.cfg`.
    pub file: String,
    pub weight: f64,
    /// `UseFor` entries: `Parent|Body` or `Body`. Non-empty means the preset is pinned.
    pub use_for: Vec<String>,
    pub bulk: String,
    pub temp: String,
    pub atmo: String,
    pub volq: String,
    pub life_organic: String,
    pub life_exotic: String,
    pub color_vary: [f32; 3],
    /// `(collection, soil names)`; collections: Mineral, LifeOrganicUni, LifeOrganicMulti,
    /// LifeExoticUni, LifeExoticMulti.
    pub materials: Vec<(String, Vec<String>)>,
    /// All biomes present after `Default` inheritance, in [`BiomeKind::ALL`] order.
    pub biomes: Vec<(BiomeKind, Biome)>,
    /// Deep ocean colour (`OceanPalette/Deep`).
    pub ocean_palette: Option<[f32; 4]>,
    /// First cloud layer colour (`CloudsPalette/Layer0`).
    pub clouds_palette: Option<[f32; 4]>,

    // Extras beyond the minimal API.
    /// `OceanPalette/Shelf`.
    pub ocean_shelf: Option<[f32; 4]>,
    /// `OceanPalette/Absorption`.
    pub ocean_absorption: Option<[f32; 4]>,
    /// All `CloudsPalette/LayerN` colours in order (gas giants have 8).
    pub clouds_layers: Vec<[f32; 4]>,
    pub sun_class: String,
    /// `SunTemp (min max)` in K: host star effective temperature range.
    pub sun_temp: Option<[f64; 2]>,
    /// `SpecClass` (brown dwarfs: L, T, Y; carbon: C).
    pub spec_class: Option<String>,
}

impl BiomePreset {
    pub fn biome(&self, k: BiomeKind) -> Option<&Biome> {
        self.biomes.iter().find(|(bk, _)| *bk == k).map(|(_, b)| b)
    }

    /// Soil names of one materials collection (case-insensitive), if the preset has it.
    pub fn soils_of(&self, collection: &str) -> Option<&[String]> {
        self.materials
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(collection))
            .map(|(_, v)| v.as_slice())
    }
}

#[derive(Clone, Debug)]
pub struct SoilLayer {
    pub freq: f32,
    /// The second number: SE's rotation flag for the layer's anti-tiling lookups (`matScaleRot.zw`
    /// in tg_detail_maps.glsl), not a weight; the layer is used either way.
    pub rot: f32,
    pub texture: String,
}

#[derive(Clone, Debug)]
pub struct Soil {
    pub name: String,
    pub color: [f32; 4],
    pub slope: f32,
    pub mod_color: f32,
    pub mod_rough: f32,
    pub mod_norm: f32,
    pub mod_height: f32,
    pub bias_rough: f32,
    pub scale_rough: f32,
    pub bias_height: f32,
    pub scale_height: f32,
    pub detail: Option<SoilLayer>,
    pub modul: Option<SoilLayer>,
}

/// What the renderer knows about a body; the caller fills it.
#[derive(Clone, Debug, Default)]
pub struct BodyInfo {
    pub name: String,
    pub parent: String,
    /// The catalog `Class` string: "Terra", "Selena", "Ferria", "Aquaria", "Carbonia",
    /// "Glacia", "Jupiter", "Neptune", "Asteroid", ...
    pub class: String,
    pub temperature_k: f64,
    pub has_atmosphere: bool,
    pub has_ocean: bool,
    pub life_organic: bool,
    pub life_exotic: bool,
    pub multicellular: bool,
    pub surf_style: f64,
    /// SE's classes of the body ([`crate::surface_gen::Classes`]) when known; they replace the
    /// approximations from `class`, `temperature_k`, `has_atmosphere` and `has_ocean`.
    pub se: Option<crate::surface_gen::Classes>,
    /// Host-star temperature for `SunTemp` ([`crate::surface_gen::preset_sun_temp`]).
    pub sun_temp: Option<f64>,
}

/// Which constraints [`SurfacePresets::find_for_ex`] had to drop to find a preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relaxed {
    /// Pinned by `UseFor` or every constraint matched.
    Nothing,
    /// Host star temperature (`SunTemp`) ignored.
    SunTemp,
    /// ... and temperature class ignored.
    Temp,
    /// ... and volatiles/atmosphere classes ignored.
    VolqAtmo,
    /// ... and life classes ignored.
    Life,
    /// ... and bulk class ignored (any unpinned preset).
    Bulk,
}

pub struct SurfacePresets {
    pub presets: Vec<BiomePreset>,
    pub soils: Vec<Soil>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------------------
// Classification helpers
// ---------------------------------------------------------------------------------------

/// SE temperature class for a temperature in K.
///
/// Thresholds (upper bounds, exclusive): Frigid < 90, Cold < 170, Cool < 250,
/// Temperate < 330, Warm < 500, Hot < 1000, otherwise Torrid.
pub fn temp_class(k: f64) -> &'static str {
    if k < 90.0 {
        "Frigid"
    } else if k < 170.0 {
        "Cold"
    } else if k < 250.0 {
        "Cool"
    } else if k < 330.0 {
        "Temperate"
    } else if k < 500.0 {
        "Warm"
    } else if k < 1000.0 {
        "Hot"
    } else {
        "Torrid"
    }
}

/// Map a catalog class to SE's `BulkClass` (case-insensitive).
///
/// Terra, Selena -> Terra; Ferria, Chthonia -> Ferria; Carbonia -> Carbonia;
/// Aquaria, Glacia -> Aquaria; Asteroid, Comet, Small, Minor -> Asteroid;
/// Jupiter, GasGiant, Saturn -> Jupiter; Neptune, IceGiant, Uranus -> Neptune;
/// BrownDwarf -> BrownDwarf. Anything unknown falls back to Terra.
pub fn bulk_class(class: &str) -> &'static str {
    let c: String = class
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    match c.as_str() {
        "terra" | "selena" => "Terra",
        "ferria" | "chthonia" => "Ferria",
        "carbonia" => "Carbonia",
        "aquaria" | "glacia" => "Aquaria",
        "asteroid" | "comet" | "small" | "minor" | "minorbody" => "Asteroid",
        "jupiter" | "gasgiant" | "saturn" | "gas" => "Jupiter",
        "neptune" | "icegiant" | "uranus" | "ice" => "Neptune",
        "browndwarf" | "dwarf" => "BrownDwarf",
        _ => "Terra",
    }
}

/// Volatile class of a body: `Airless` (no atmosphere), `NonArid` (atmosphere and ocean),
/// otherwise `Arid`. A preset's `VolqClass` must equal this unless it is `Any`, so wet
/// (`NonArid`) presets are never given to airless or dry bodies until constraints relax.
fn body_volq(b: &BodyInfo) -> &'static str {
    if !b.has_atmosphere {
        "Airless"
    } else if b.has_ocean {
        "NonArid"
    } else {
        "Arid"
    }
}

fn is_any(s: &str) -> bool {
    s.is_empty() || s.eq_ignore_ascii_case("any") || s.eq_ignore_ascii_case("dontmatter")
}

/// Life requirement (`LifeOrganic` / `LifeExotic`) against a body's life state.
fn life_matches(req: &str, present: bool, multi: bool) -> bool {
    if is_any(req) {
        return true;
    }
    match req.to_ascii_lowercase().as_str() {
        "none" | "no" | "nolife" | "absent" => !present,
        "unormulti" | "unicellularormulticellular" | "any life" | "life" => present,
        "unicellular" | "uni" => present && !multi,
        "multicellular" | "multi" => present && multi,
        _ => true,
    }
}

/// Which constraint groups are active in a selection pass.
#[derive(Clone, Copy)]
struct Active {
    sun: bool,
    temp: bool,
    volq_atmo: bool,
    life: bool,
    bulk: bool,
}

/// SE's class test (FUN_140161470): `NonArid` is Lacustrine..Superoceanic, `NonAirless` any
/// atmosphere, `None` a wildcard bulk class, and `SunTemp` (lo, hi] holds the star temperature.
fn se_matches(p: &BiomePreset, c: crate::surface_gen::Classes, b: &BodyInfo, star_temp: Option<f64>, a: Active) -> bool {
    use crate::surface_gen::{ATMO_NAMES, BULK_NAMES, TEMP_NAMES, VOLQ_NAMES};
    let named = |req: &str, names: &[&str], i: i32| names.get(i as usize).is_some_and(|n| req.eq_ignore_ascii_case(n));
    if a.bulk && !(is_any(&p.bulk) || p.bulk.eq_ignore_ascii_case("none") || named(&p.bulk, &BULK_NAMES, c.bulk)) {
        return false;
    }
    if a.temp && !is_any(&p.temp) && !named(&p.temp, &TEMP_NAMES, c.temp) {
        return false;
    }
    if a.volq_atmo {
        let atmo = is_any(&p.atmo) || (p.atmo.eq_ignore_ascii_case("nonairless") && c.atmo != 0) || named(&p.atmo, &ATMO_NAMES, c.atmo);
        let volq = is_any(&p.volq) || (p.volq.eq_ignore_ascii_case("nonarid") && (2..=5).contains(&c.volq)) || named(&p.volq, &VOLQ_NAMES, c.volq);
        if !atmo || !volq {
            return false;
        }
    }
    if a.life && (!life_matches(&p.life_organic, b.life_organic, b.multicellular) || !life_matches(&p.life_exotic, b.life_exotic, b.multicellular)) {
        return false;
    }
    if a.sun {
        if let (Some(t), Some([lo, hi])) = (star_temp, p.sun_temp) {
            if t <= lo || t > hi {
                return false;
            }
        }
    }
    true
}

fn preset_matches(p: &BiomePreset, b: &BodyInfo, star_temp: Option<f64>, a: Active) -> bool {
    if let Some(c) = b.se {
        return se_matches(p, c, b, star_temp, a);
    }
    if a.bulk && !is_any(&p.bulk) && !p.bulk.eq_ignore_ascii_case(bulk_class(&b.class)) {
        return false;
    }
    if a.temp && !is_any(&p.temp) && !p.temp.eq_ignore_ascii_case(temp_class(b.temperature_k)) {
        return false;
    }
    if a.volq_atmo {
        if p.atmo.eq_ignore_ascii_case("airless") && b.has_atmosphere {
            return false;
        }
        if p.atmo.eq_ignore_ascii_case("nonairless") && !b.has_atmosphere {
            return false;
        }
        let v = body_volq(b);
        let ok = if is_any(&p.volq) {
            true
        } else {
            p.volq.eq_ignore_ascii_case(v)
        };
        if !ok {
            return false;
        }
    }
    if a.life {
        let organic_present = b.life_organic;
        let exotic_present = b.life_exotic;
        if !life_matches(&p.life_organic, organic_present, b.multicellular) {
            return false;
        }
        if !life_matches(&p.life_exotic, exotic_present, b.multicellular) {
            return false;
        }
    }
    if a.sun {
        if let (Some(t), Some([lo, hi])) = (star_temp, p.sun_temp) {
            if t < lo || t > hi {
                return false;
            }
        }
    }
    true
}

/// Weighted choice: walk the cumulative weights to `style * total`.
fn pick_weighted<'a>(cands: &[&'a BiomePreset], style: f64) -> Option<&'a BiomePreset> {
    if cands.is_empty() {
        return None;
    }
    let style = if style.is_finite() { style.clamp(0.0, 0.999_999_999) } else { 0.0 };
    let total: f64 = cands.iter().map(|p| p.weight.max(0.0)).sum();
    if total <= 0.0 {
        let i = ((style * cands.len() as f64) as usize).min(cands.len() - 1);
        return Some(cands[i]);
    }
    let target = style * total;
    let mut cum = 0.0;
    for p in cands {
        let w = p.weight.max(0.0);
        if w <= 0.0 {
            continue;
        }
        cum += w;
        if target < cum {
            return Some(p);
        }
    }
    cands.iter().rev().find(|p| p.weight > 0.0).copied()
}

/// Does a `UseFor` entry (`Parent|Body` or `Body`) name this body? Returns 2 for an exact
/// parent+body match, 1 for a body-name-only match, 0 for no match.
fn use_for_match(entry: &str, b: &BodyInfo) -> u8 {
    let (parent, body) = match entry.split_once('|') {
        Some((p, n)) => (Some(p.trim()), n.trim()),
        None => (None, entry.trim()),
    };
    let name_ok = b
        .name
        .split('/')
        .any(|n| n.trim().eq_ignore_ascii_case(body));
    if !name_ok {
        return 0;
    }
    match parent {
        None => 2,
        Some(p) if b.parent.split('/').any(|n| n.trim().eq_ignore_ascii_case(p)) => 2,
        // The catalog's parent may differ from SE's (e.g. a barycentre), so a matching
        // name alone still counts, with lower priority.
        Some(_) => 1,
    }
}

// ---------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------

/// A parsed preset file before `Default` inheritance: unspecified fields are `None`.
#[derive(Clone, Debug, Default)]
pub struct RawPreset {
    pub file: String,
    pub default: Option<String>,
    pub use_for: Vec<String>,
    pub weight: Option<f64>,
    pub bulk: Option<String>,
    pub temp: Option<String>,
    pub atmo: Option<String>,
    pub volq: Option<String>,
    pub sun_class: Option<String>,
    pub sun_temp: Option<[f64; 2]>,
    pub spec_class: Option<String>,
    pub life_organic: Option<String>,
    pub life_exotic: Option<String>,
    pub color_vary: Option<[f32; 3]>,
    pub materials: Vec<(String, Vec<String>)>,
    pub biomes: Vec<(BiomeKind, RawBiome)>,
    pub ocean: Option<OceanPalette>,
    pub clouds: Option<Vec<[f32; 4]>>,
}

#[derive(Clone, Debug, Default)]
pub struct RawBiome {
    pub color: Option<[f32; 4]>,
    pub large: Option<BiomeLayer>,
    pub small: Option<BiomeLayer>,
}

#[derive(Clone, Debug, Default)]
pub struct OceanPalette {
    pub deep: Option<[f32; 4]>,
    pub shelf: Option<[f32; 4]>,
    pub absorption: Option<[f32; 4]>,
}

fn nums_f32(item: &Item) -> Vec<f32> {
    item.nums().into_iter().map(|v| v as f32).collect()
}

fn color4(item: &Item) -> Option<[f32; 4]> {
    let n = nums_f32(item);
    match n.len() {
        0..=2 => None,
        3 => Some([n[0], n[1], n[2], 0.0]),
        _ => Some([n[0], n[1], n[2], n[3]]),
    }
}

/// All numbers and the first string of a block of bare values: `{ 64 1.000 1.000 "X" }`.
fn row_values(block: &Block) -> (Vec<f64>, Option<String>) {
    let mut nums = Vec::new();
    let mut text = None;
    for it in &block.items {
        for a in &it.args {
            match a {
                Value::Num(n) => nums.push(*n),
                Value::Str(s) | Value::Ident(s) => {
                    if text.is_none() {
                        text = Some(s.clone());
                    }
                }
                Value::Tuple(_) => {}
            }
        }
    }
    (nums, text)
}

fn parse_biome_layer(item: &Item, ctx: &str, warnings: &mut Vec<String>) -> Option<BiomeLayer> {
    let block = item.block.as_ref()?;
    let (n, tex) = row_values(block);
    match (n.as_slice(), tex) {
        ([f, a, b, ..], Some(texture)) => Some(BiomeLayer {
            freq: *f as f32,
            a: *a as f32,
            b: *b as f32,
            texture,
        }),
        _ => {
            warnings.push(format!("{ctx}: malformed Layer (line {})", item.line));
            None
        }
    }
}

fn str_arg(item: &Item) -> Option<String> {
    item.args.iter().find_map(|a| a.as_str().map(str::to_string))
}

fn palette_color(block: &Block, key: &str) -> Option<[f32; 4]> {
    block.get_last(key).and_then(color4)
}

/// Parse one preset file (no inheritance yet). Problems are appended to `warnings`.
pub fn parse_preset(file: &str, text: &str, warnings: &mut Vec<String>) -> RawPreset {
    let root = sc::parse(text);
    let mut p = RawPreset {
        file: file.to_string(),
        ..Default::default()
    };
    for it in &root.items {
        let key = it.key.to_ascii_lowercase();
        match key.as_str() {
            "default" => p.default = str_arg(it),
            "usefor" => {
                p.use_for
                    .extend(it.args.iter().filter_map(|a| a.as_str().map(|s| s.trim().to_string())));
            }
            "weight" => p.weight = it.f64(),
            "bulkclass" => p.bulk = str_arg(it),
            "tempclass" => p.temp = str_arg(it),
            "atmoclass" => p.atmo = str_arg(it),
            "volqclass" => p.volq = str_arg(it),
            "specclass" => p.spec_class = str_arg(it),
            "sunclass" => p.sun_class = str_arg(it),
            "suntemp" => {
                let n = it.nums();
                if n.len() >= 2 {
                    p.sun_temp = Some([n[0], n[1]]);
                } else {
                    warnings.push(format!("{file}: bad SunTemp (line {})", it.line));
                }
            }
            "lifeorganic" => p.life_organic = str_arg(it),
            "lifeexotic" => p.life_exotic = str_arg(it),
            "colorvary" => {
                let n = nums_f32(it);
                if n.len() >= 3 {
                    p.color_vary = Some([n[0], n[1], n[2]]);
                } else {
                    warnings.push(format!("{file}: bad ColorVary (line {})", it.line));
                }
            }
            "materials" => {
                let coll = str_arg(it).unwrap_or_default();
                let mut names = Vec::new();
                if let Some(b) = &it.block {
                    for sub in &b.items {
                        for a in &sub.args {
                            if let Value::Str(s) = a {
                                names.push(s.clone());
                            }
                        }
                    }
                }
                match p.materials.iter_mut().find(|(c, _)| *c == coll) {
                    Some((_, v)) => *v = names,
                    None => p.materials.push((coll, names)),
                }
            }
            "biome" => {
                let name = str_arg(it).unwrap_or_default();
                let Some(kind) = BiomeKind::from_name(&name) else {
                    warnings.push(format!("{file}: unknown biome \"{name}\" (line {})", it.line));
                    continue;
                };
                let mut rb = RawBiome::default();
                if let Some(b) = &it.block {
                    for sub in &b.items {
                        match sub.key.to_ascii_lowercase().as_str() {
                            "color" => rb.color = color4(sub),
                            "layer" => {
                                let ln = str_arg(sub).unwrap_or_default();
                                let ctx = format!("{file}: Biome {name}");
                                match ln.to_ascii_lowercase().as_str() {
                                    "large" => rb.large = parse_biome_layer(sub, &ctx, warnings),
                                    "small" => rb.small = parse_biome_layer(sub, &ctx, warnings),
                                    _ => warnings.push(format!("{ctx}: unknown layer \"{ln}\"")),
                                }
                            }
                            other => warnings.push(format!(
                                "{file}: Biome {name}: unknown key \"{other}\" (line {})",
                                sub.line
                            )),
                        }
                    }
                }
                match p.biomes.iter_mut().find(|(k, _)| *k == kind) {
                    Some((_, e)) => *e = rb,
                    None => p.biomes.push((kind, rb)),
                }
            }
            "oceanpalette" => {
                let mut o = OceanPalette::default();
                if let Some(b) = &it.block {
                    o.deep = palette_color(b, "Deep");
                    o.shelf = palette_color(b, "Shelf");
                    o.absorption = palette_color(b, "Absorption");
                } else if let Some(c) = color4(it) {
                    o.deep = Some(c);
                }
                p.ocean = Some(o);
            }
            "cloudspalette" => {
                let mut layers = Vec::new();
                if let Some(b) = &it.block {
                    for i in 0..16 {
                        match palette_color(b, &format!("Layer{i}")) {
                            Some(c) => layers.push(c),
                            None => break,
                        }
                    }
                } else if let Some(c) = color4(it) {
                    layers.push(c);
                }
                p.clouds = Some(layers);
            }
            "" => {}
            other => warnings.push(format!("{file}: unknown key \"{other}\" (line {})", it.line)),
        }
    }
    p
}

/// Parse a `.pml` soil file: `Soil "name" { Color (...) Slope .. Layer "Detail" { f w "tex" } }`.
pub fn parse_soils(file: &str, text: &str, warnings: &mut Vec<String>) -> Vec<Soil> {
    let root = sc::parse(text);
    let mut out = Vec::new();
    for it in &root.items {
        if !it.key.eq_ignore_ascii_case("soil") {
            if !it.key.is_empty() {
                warnings.push(format!("{file}: unknown key \"{}\" (line {})", it.key, it.line));
            }
            continue;
        }
        let (Some(name), Some(b)) = (it.name(), it.block.as_ref()) else {
            warnings.push(format!("{file}: malformed Soil (line {})", it.line));
            continue;
        };
        let f = |k: &str, d: f64| b.f64(k).unwrap_or(d) as f32;
        let layer = |which: &str| -> Option<SoilLayer> {
            let lb = b
                .all("Layer")
                .find(|l| l.name().is_some_and(|n| n.eq_ignore_ascii_case(which)))?
                .block
                .as_ref()?;
            let (n, tex) = row_values(lb);
            match (n.as_slice(), tex) {
                ([fr, w, ..], Some(texture)) => Some(SoilLayer {
                    freq: *fr as f32,
                    rot: *w as f32,
                    texture,
                }),
                _ => None,
            }
        };
        let color = b.get_last("Color").and_then(color4).unwrap_or([0.5, 0.5, 0.5, 0.0]);
        let detail = layer("Detail");
        let modul = layer("Modul");
        if detail.is_none() && modul.is_none() {
            warnings.push(format!("{file}: Soil \"{name}\" has no readable layers"));
        }
        out.push(Soil {
            name: name.to_string(),
            color,
            slope: f("Slope", 0.0),
            mod_color: f("ModColor", 1.0),
            mod_rough: f("ModRough", 1.0),
            mod_norm: f("ModNorm", 1.0),
            mod_height: f("ModHeight", 1.0),
            bias_rough: f("BiasRough", 0.0),
            scale_rough: f("ScaleRough", 1.0),
            bias_height: f("BiasHeight", 0.0),
            scale_height: f("ScaleHeight", 1.0),
            detail,
            modul,
        });
    }
    out
}

/// Read `Surface/SurfStyle` of a body block (a `Planet`/`Moon` item block). If the block
/// has no `Surface` sub-block, a direct `SurfStyle` key is accepted too.
pub fn surf_style(block: &Block) -> Option<f64> {
    block
        .sub("Surface")
        .and_then(|s| s.f64("SurfStyle"))
        .or_else(|| block.f64("SurfStyle"))
}

// ---------------------------------------------------------------------------------------
// Inheritance
// ---------------------------------------------------------------------------------------

fn resolve(
    raw: &RawPreset,
    by_file: &HashMap<String, usize>,
    all: &[RawPreset],
    depth: usize,
    warnings: &mut Vec<String>,
) -> BiomePreset {
    // Resolve the parent chain first (depth-limited to survive cycles).
    let parent = match raw.default.as_deref() {
        Some(d) if depth < 16 => match by_file.get(&d.to_ascii_lowercase()) {
            Some(&i) => Some(resolve(&all[i], by_file, all, depth + 1, warnings)),
            None => {
                warnings.push(format!("{}: Default \"{d}\" not found", raw.file));
                None
            }
        },
        Some(d) => {
            warnings.push(format!("{}: Default chain too deep at \"{d}\" (cycle?)", raw.file));
            None
        }
        None => None,
    };

    let s = |own: &Option<String>, f: fn(&BiomePreset) -> &String| -> String {
        own.clone()
            .or_else(|| parent.as_ref().map(|p| f(p).clone()))
            .unwrap_or_else(|| "Any".to_string())
    };

    // Materials: per-collection override over the parent's.
    let mut materials = parent.as_ref().map(|p| p.materials.clone()).unwrap_or_default();
    for (c, v) in &raw.materials {
        match materials.iter_mut().find(|(pc, _)| pc == c) {
            Some((_, pv)) => *pv = v.clone(),
            None => materials.push((c.clone(), v.clone())),
        }
    }

    // Biomes: field-wise override per biome.
    let mut slots: Vec<Option<Biome>> = vec![None; BiomeKind::ALL.len()];
    if let Some(p) = &parent {
        for (k, b) in &p.biomes {
            slots[k.index()] = Some(b.clone());
        }
    }
    for (k, rb) in &raw.biomes {
        let base = slots[k.index()].take();
        let color = rb
            .color
            .or(base.as_ref().map(|b| b.color))
            .unwrap_or([0.0; 4]);
        let large = rb.large.clone().or_else(|| base.as_ref().and_then(|b| b.large.clone()));
        let small = rb.small.clone().or_else(|| base.as_ref().and_then(|b| b.small.clone()));
        slots[k.index()] = Some(Biome { color, large, small });
    }
    let biomes: Vec<(BiomeKind, Biome)> = BiomeKind::ALL
        .iter()
        .zip(slots)
        .filter_map(|(k, b)| b.map(|b| (*k, b)))
        .collect();

    let ocean = match (&raw.ocean, &parent) {
        (Some(o), Some(p)) => (
            o.deep.or(p.ocean_palette),
            o.shelf.or(p.ocean_shelf),
            o.absorption.or(p.ocean_absorption),
        ),
        (Some(o), None) => (o.deep, o.shelf, o.absorption),
        (None, Some(p)) => (p.ocean_palette, p.ocean_shelf, p.ocean_absorption),
        (None, None) => (None, None, None),
    };
    let clouds_layers = raw
        .clouds
        .clone()
        .or_else(|| parent.as_ref().map(|p| p.clouds_layers.clone()))
        .unwrap_or_default();

    BiomePreset {
        file: raw.file.clone(),
        weight: raw.weight.or(parent.as_ref().map(|p| p.weight)).unwrap_or(1.0),
        use_for: raw.use_for.clone(),
        bulk: s(&raw.bulk, |p| &p.bulk),
        temp: s(&raw.temp, |p| &p.temp),
        atmo: s(&raw.atmo, |p| &p.atmo),
        volq: s(&raw.volq, |p| &p.volq),
        life_organic: s(&raw.life_organic, |p| &p.life_organic),
        life_exotic: s(&raw.life_exotic, |p| &p.life_exotic),
        color_vary: raw
            .color_vary
            .or(parent.as_ref().map(|p| p.color_vary))
            .unwrap_or([1.0; 3]),
        materials,
        biomes,
        ocean_palette: ocean.0,
        clouds_palette: clouds_layers.first().copied(),
        ocean_shelf: ocean.1,
        ocean_absorption: ocean.2,
        clouds_layers,
        sun_class: s(&raw.sun_class, |p| &p.sun_class),
        sun_temp: raw.sun_temp.or(parent.as_ref().and_then(|p| p.sun_temp)),
        spec_class: raw
            .spec_class
            .clone()
            .or_else(|| parent.as_ref().and_then(|p| p.spec_class.clone())),
    }
}

// ---------------------------------------------------------------------------------------
// SurfacePresets
// ---------------------------------------------------------------------------------------

impl SurfacePresets {
    /// Load every preset and soil file from the VFS. Never fails: problems become warnings.
    pub fn load(vfs: &Vfs) -> Self {
        let mut warnings = Vec::new();
        let mut preset_texts = Vec::new();
        for path in vfs.list_ext(SAMPLES_DIR, "cfg") {
            match vfs.read_string(&path) {
                Ok(t) => preset_texts.push((base_name(&path).to_string(), t)),
                Err(e) => warnings.push(format!("{path}: {e}")),
            }
        }
        let mut soil_texts = Vec::new();
        for path in vfs.list_ext(MATERIALS_DIR, "pml") {
            match vfs.read_string(&path) {
                Ok(t) => soil_texts.push((base_name(&path).to_string(), t)),
                Err(e) => warnings.push(format!("{path}: {e}")),
            }
        }
        let mut out = Self::from_texts(&preset_texts, &soil_texts);
        warnings.append(&mut out.warnings);
        out.warnings = warnings;
        out
    }

    /// Build from in-memory `(file name, text)` pairs (used by `load` and by tests).
    pub fn from_texts(preset_files: &[(String, String)], soil_files: &[(String, String)]) -> Self {
        let mut warnings = Vec::new();
        let raws: Vec<RawPreset> = preset_files
            .iter()
            .map(|(f, t)| parse_preset(f, t, &mut warnings))
            .collect();
        let by_file: HashMap<String, usize> = raws
            .iter()
            .enumerate()
            .map(|(i, r)| (r.file.to_ascii_lowercase(), i))
            .collect();
        let presets = raws
            .iter()
            .map(|r| resolve(r, &by_file, &raws, 0, &mut warnings))
            .collect();
        let mut soils = Vec::new();
        for (f, t) in soil_files {
            soils.extend(parse_soils(f, t, &mut warnings));
        }
        SurfacePresets {
            presets,
            soils,
            warnings,
        }
    }

    /// Preset for a body. See [`SurfacePresets::find_for_ex`].
    pub fn find_for(&self, body: &BodyInfo) -> Option<&BiomePreset> {
        self.find_for_ex(body, body.sun_temp).map(|(p, _)| p)
    }

    /// Preset for a body, also reporting which constraints had to be relaxed.
    /// `star_temp` is the host star's effective temperature in K (for `SunTemp`); `None`
    /// skips that check. Relaxation is logged with `log::warn!`.
    pub fn find_for_ex(&self, body: &BodyInfo, star_temp: Option<f64>) -> Option<(&BiomePreset, Relaxed)> {
        // 1. Pinned by UseFor (exact parent+body beats body-name-only).
        let mut best: Option<(&BiomePreset, u8)> = None;
        for p in &self.presets {
            let m = p.use_for.iter().map(|e| use_for_match(e, body)).max().unwrap_or(0);
            if m > 0 && best.is_none_or(|(_, bm)| m > bm) {
                best = Some((p, m));
            }
        }
        if let Some((p, _)) = best {
            return Some((p, Relaxed::Nothing));
        }

        // 2-4. Class match with progressively relaxed constraints.
        let passes = [
            (Relaxed::Nothing, Active { sun: true, temp: true, volq_atmo: true, life: true, bulk: true }),
            (Relaxed::SunTemp, Active { sun: false, temp: true, volq_atmo: true, life: true, bulk: true }),
            (Relaxed::Temp, Active { sun: false, temp: false, volq_atmo: true, life: true, bulk: true }),
            (Relaxed::VolqAtmo, Active { sun: false, temp: false, volq_atmo: false, life: true, bulk: true }),
            (Relaxed::Life, Active { sun: false, temp: false, volq_atmo: false, life: false, bulk: true }),
            (Relaxed::Bulk, Active { sun: false, temp: false, volq_atmo: false, life: false, bulk: false }),
        ];
        for (level, active) in passes {
            let cands: Vec<&BiomePreset> = self
                .presets
                .iter()
                .filter(|p| p.use_for.is_empty() && preset_matches(p, body, star_temp, active))
                .collect();
            if let Some(p) = pick_weighted(&cands, body.surf_style) {
                if level != Relaxed::Nothing {
                    log::warn!(
                        "biome preset for \"{}\" ({}, {:.0} K): relaxed constraints to {:?}, using {}",
                        body.name,
                        body.class,
                        body.temperature_k,
                        level,
                        p.file
                    );
                }
                return Some((p, level));
            }
        }
        log::warn!("no biome preset available for \"{}\"", body.name);
        None
    }

    /// First soil entry with this name (case-insensitive).
    pub fn soil(&self, name: &str) -> Option<&Soil> {
        self.soils.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Every distinct sample texture name referenced by any biome layer, sorted.
    pub fn sample_textures(&self) -> Vec<String> {
        let mut set = BTreeSet::new();
        for p in &self.presets {
            for (_, b) in &p.biomes {
                for l in [&b.large, &b.small].into_iter().flatten() {
                    set.insert(l.texture.clone());
                }
            }
        }
        set.into_iter().collect()
    }
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_CFG: &str = r#"
Weight     0.5
BulkClass "Terra"
TempClass "Any"
AtmoClass "NonAirless"
VolqClass "Arid"
SunClass  "Any"
SunTemp   (4900 6000)
LifeOrganic "DontMatter"
LifeExotic  "DontMatter"
ColorVary	(0.789 0.747 0.734)
Materials	"Mineral"
{
	"Gravel gray"
	"Snow 1"
}
Biome "Lowland"
{
	Color (0.590 0.519 0.491 0.000)
	Layer "Large"	{  64 1.000 0.500 "Desert002_COL" }
	Layer "Small"	{   4 1.000 1.000 "Upland002_COL" }
}
Biome "Upland" { Color (0.1 0.2 0.3 0.4) Layer "Large" { 8 1 1 "Rock001_COL" } }
OceanPalette
{
	Deep	(0.040 0.100 0.200 1.000)
	Shelf	(0.050 0.110 0.210 1.000)
	Absorption	(40.0 5.000 1.000 5.000)
}
"#;

    const CHILD_CFG: &str = r#"
Default   "default.cfg"
Weight     0.25
/* a comment */
TempClass "Warm"
Materials	"Mineral"
{
	"Mars red soil 1"
}
Biome "Lowland"
{
	Color (0.9, 0.8, 0.7, 0.0)
}
Biome "Snow" { Color (1 1 1 1) }
"#;

    fn preset_files() -> Vec<(String, String)> {
        vec![
            ("default.cfg".into(), DEFAULT_CFG.into()),
            ("child.cfg".into(), CHILD_CFG.into()),
            (
                "airless.cfg".into(),
                "Weight 1\nBulkClass \"Terra\"\nAtmoClass \"Airless\"\nVolqClass \"Airless\"\nBiome \"Rock\" { Color (0.5 0.5 0.5 0) }".into(),
            ),
            (
                "custom_Moon.cfg".into(),
                "UseFor \"Earth-Moon|Moon\"\nBiome \"Rock\" { Color (0.4 0.4 0.4 0) }".into(),
            ),
            (
                "custom_Mars.cfg".into(),
                "UseFor \"Sol|Mars\"\nBiome \"Rock\" { Color (0.8 0.4 0.3 0) }".into(),
            ),
        ]
    }

    fn presets() -> SurfacePresets {
        SurfacePresets::from_texts(&preset_files(), &[])
    }

    fn body(class: &str, temp: f64, atmo: bool) -> BodyInfo {
        BodyInfo {
            name: "Test".into(),
            parent: "Star".into(),
            class: class.into(),
            temperature_k: temp,
            has_atmosphere: atmo,
            ..Default::default()
        }
    }

    #[test]
    fn biome_names_round_trip() {
        assert_eq!(BiomeKind::ALL.len(), 21);
        for k in BiomeKind::ALL {
            assert_eq!(BiomeKind::from_name(k.name()), Some(k));
        }
        assert_eq!(BiomeKind::from_name("forestexom"), Some(BiomeKind::ForestExoM));
        assert_eq!(BiomeKind::from_name("Nope"), None);
        assert_eq!(BiomeKind::ALL[0], BiomeKind::Bottom);
        assert_eq!(BiomeKind::ALL[20], BiomeKind::GrassExoM);
    }

    #[test]
    fn parses_layers_and_palettes() {
        let sp = presets();
        assert!(sp.warnings.is_empty(), "{:?}", sp.warnings);
        let d = sp.presets.iter().find(|p| p.file == "default.cfg").unwrap();
        assert_eq!(d.weight, 0.5);
        assert_eq!(d.sun_temp, Some([4900.0, 6000.0]));
        let low = d.biome(BiomeKind::Lowland).unwrap();
        let large = low.large.as_ref().unwrap();
        assert_eq!((large.freq, large.a, large.b), (64.0, 1.0, 0.5));
        assert_eq!(large.texture, "Desert002_COL");
        assert_eq!(low.small.as_ref().unwrap().texture, "Upland002_COL");
        assert_eq!(low.color, [0.590, 0.519, 0.491, 0.0]);
        // Single-line biome block.
        let up = d.biome(BiomeKind::Upland).unwrap();
        assert_eq!(up.color, [0.1, 0.2, 0.3, 0.4]);
        assert!(up.small.is_none());
        assert_eq!(d.ocean_palette, Some([0.040, 0.100, 0.200, 1.0]));
        assert_eq!(d.ocean_shelf, Some([0.050, 0.110, 0.210, 1.0]));
        assert_eq!(d.ocean_absorption, Some([40.0, 5.0, 1.0, 5.0]));
        assert_eq!(d.soils_of("Mineral").unwrap(), ["Gravel gray", "Snow 1"]);
        assert_eq!(d.biomes.len(), 2);
        assert_eq!(d.biomes[0].0, BiomeKind::Lowland);
    }

    #[test]
    fn default_inheritance() {
        let sp = presets();
        let c = sp.presets.iter().find(|p| p.file == "child.cfg").unwrap();
        assert_eq!(c.weight, 0.25);
        assert_eq!(c.temp, "Warm");
        // Inherited from the parent.
        assert_eq!(c.bulk, "Terra");
        assert_eq!(c.atmo, "NonAirless");
        assert_eq!(c.volq, "Arid");
        assert_eq!(c.color_vary, [0.789, 0.747, 0.734]);
        assert_eq!(c.sun_temp, Some([4900.0, 6000.0]));
        assert_eq!(c.ocean_palette, Some([0.040, 0.100, 0.200, 1.0]));
        // Child colour overrides, parent layers kept.
        let low = c.biome(BiomeKind::Lowland).unwrap();
        assert_eq!(low.color, [0.9, 0.8, 0.7, 0.0]);
        assert_eq!(low.large.as_ref().unwrap().texture, "Desert002_COL");
        // Inherited-only and child-only biomes.
        assert_eq!(c.biome(BiomeKind::Upland).unwrap().color, [0.1, 0.2, 0.3, 0.4]);
        assert!(c.biome(BiomeKind::Snow).unwrap().large.is_none());
        // Materials collection replaced.
        assert_eq!(c.soils_of("Mineral").unwrap(), ["Mars red soil 1"]);
        // Biome order follows BiomeKind::ALL.
        let order: Vec<usize> = c.biomes.iter().map(|(k, _)| k.index()).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn default_cycles_and_missing_do_not_panic() {
        let files = vec![
            ("a.cfg".to_string(), "Default \"b.cfg\"".to_string()),
            ("b.cfg".to_string(), "Default \"a.cfg\"".to_string()),
            ("c.cfg".to_string(), "Default \"missing.cfg\"".to_string()),
        ];
        let sp = SurfacePresets::from_texts(&files, &[]);
        assert_eq!(sp.presets.len(), 3);
        assert!(!sp.warnings.is_empty());
    }

    #[test]
    fn use_for_wins() {
        let sp = presets();
        let mut moon = body("Selena", 250.0, false);
        moon.name = "Moon".into();
        moon.parent = "Earth-Moon".into();
        assert_eq!(sp.find_for(&moon).unwrap().file, "custom_Moon.cfg");
        // Name-only (different parent) still matches, case-insensitively.
        moon.name = "moon".into();
        moon.parent = "Earth".into();
        assert_eq!(sp.find_for(&moon).unwrap().file, "custom_Moon.cfg");
        let mut mars = body("Terra", 210.0, true);
        mars.name = "Mars".into();
        mars.parent = "Sol".into();
        assert_eq!(sp.find_for(&mars).unwrap().file, "custom_Mars.cfg");
        // Pinned presets are never picked for other bodies.
        let other = body("Selena", 250.0, false);
        assert_eq!(sp.find_for(&other).unwrap().file, "airless.cfg");
    }

    #[test]
    fn class_matching() {
        let sp = presets();
        // Airless body picks the airless preset, atmosphere picks default/child.
        assert_eq!(sp.find_for(&body("Terra", 300.0, false)).unwrap().file, "airless.cfg");
        let (p, r) = sp.find_for_ex(&body("Terra", 300.0, true), None).unwrap();
        assert!(p.file == "default.cfg" || p.file == "child.cfg");
        // 300 K is Temperate, so the Warm child does not match at the strict level.
        assert_eq!((p.file.as_str(), r), ("default.cfg", Relaxed::Nothing));
        let mut warm = body("Terra", 400.0, true);
        warm.surf_style = 0.9; // cumulative: default 0..0.5, child 0.5..0.75 (of 0.75)
        assert_eq!(sp.find_for(&warm).unwrap().file, "child.cfg");
        warm.surf_style = 0.1;
        assert_eq!(sp.find_for(&warm).unwrap().file, "default.cfg");
    }

    #[test]
    fn se_classes_and_star_temperature() {
        use crate::surface_gen::Classes;
        let files = vec![
            ("dry.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"\nAtmoClass \"NonAirless\"\nVolqClass \"Arid\"".to_string()),
            ("wet.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"\nAtmoClass \"NonAirless\"\nVolqClass \"NonArid\"".to_string()),
            ("red.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"\nAtmoClass \"Airless\"\nVolqClass \"Airless\"\nSunTemp (2000 4000)".to_string()),
            ("sun.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"\nAtmoClass \"Airless\"\nVolqClass \"Airless\"\nSunTemp (4000 7000)".to_string()),
        ];
        let sp = SurfacePresets::from_texts(&files, &[]);
        let pick = |volq: i32, atmo: i32, sun: f64| {
            // has_atmosphere false: SE's classes decide.
            let mut b = body("Terra", 300.0, false);
            b.se = Some(Classes { bulk: 2, temp: 3, atmo, volq, size: 3, surface: 0 });
            b.sun_temp = Some(sun);
            sp.find_for(&b).unwrap().file.clone()
        };
        assert_eq!(pick(1, 2, 5800.0), "dry.cfg");
        assert_eq!(pick(3, 3, 5800.0), "wet.cfg");
        assert_eq!(pick(0, 0, 3000.0), "red.cfg");
        assert_eq!(pick(0, 0, 5800.0), "sun.cfg");
        assert_eq!(pick(0, 0, 4000.0), "red.cfg");
    }

    #[test]
    fn weighted_choice_by_surf_style() {
        let files = vec![
            ("a.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"".to_string()),
            ("b.cfg".to_string(), "Weight 2\nBulkClass \"Terra\"".to_string()),
            ("c.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"".to_string()),
            ("z.cfg".to_string(), "Weight 5\nBulkClass \"Ferria\"".to_string()),
        ];
        let sp = SurfacePresets::from_texts(&files, &[]);
        let pick = |s: f64| {
            let mut b = body("Terra", 300.0, true);
            b.surf_style = s;
            sp.find_for(&b).unwrap().file.clone()
        };
        // Cumulative: a [0,0.25), b [0.25,0.75), c [0.75,1).
        assert_eq!(pick(0.0), "a.cfg");
        assert_eq!(pick(0.24), "a.cfg");
        assert_eq!(pick(0.25), "b.cfg");
        assert_eq!(pick(0.74), "b.cfg");
        assert_eq!(pick(0.75), "c.cfg");
        assert_eq!(pick(0.999), "c.cfg");
        // Out-of-range / NaN styles are clamped.
        assert_eq!(pick(1.5), "c.cfg");
        assert_eq!(pick(-1.0), "a.cfg");
        assert_eq!(pick(f64::NAN), "a.cfg");
    }

    #[test]
    fn relaxes_instead_of_failing() {
        let files = vec![(
            "only.cfg".to_string(),
            "Weight 1\nBulkClass \"Terra\"\nTempClass \"Frigid\"\nAtmoClass \"NonAirless\"\nLifeOrganic \"Multicellular\"".to_string(),
        )];
        let sp = SurfacePresets::from_texts(&files, &[]);
        let b = body("Terra", 300.0, false);
        let (p, r) = sp.find_for_ex(&b, None).unwrap();
        assert_eq!(p.file, "only.cfg");
        assert_eq!(r, Relaxed::Life);
        // Wrong bulk class still yields something as a last resort.
        let (p, r) = sp.find_for_ex(&body("Jupiter", 120.0, true), None).unwrap();
        assert_eq!((p.file.as_str(), r), ("only.cfg", Relaxed::Bulk));
        // Empty set is None, not a panic.
        let empty = SurfacePresets::from_texts(&[], &[]);
        assert!(empty.find_for(&b).is_none());
    }

    #[test]
    fn life_and_sun_constraints() {
        let files = vec![
            ("dead.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"\nLifeOrganic \"None\"".to_string()),
            ("multi.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"\nLifeOrganic \"Multicellular\"".to_string()),
            ("hot.cfg".to_string(), "Weight 1\nBulkClass \"Terra\"\nLifeOrganic \"Multicellular\"\nSunTemp (7000 1e+009)".to_string()),
        ];
        let sp = SurfacePresets::from_texts(&files, &[]);
        let mut b = body("Terra", 288.0, true);
        assert_eq!(sp.find_for(&b).unwrap().file, "dead.cfg");
        b.life_organic = true;
        b.multicellular = true;
        b.surf_style = 0.9;
        assert_eq!(sp.find_for_ex(&b, Some(5800.0)).unwrap().0.file, "multi.cfg");
        assert_eq!(sp.find_for_ex(&b, Some(8000.0)).unwrap().0.file, "hot.cfg");
    }

    #[test]
    fn helpers() {
        assert_eq!(temp_class(50.0), "Frigid");
        assert_eq!(temp_class(89.9), "Frigid");
        assert_eq!(temp_class(90.0), "Cold");
        assert_eq!(temp_class(288.0), "Temperate");
        assert_eq!(temp_class(400.0), "Warm");
        assert_eq!(temp_class(700.0), "Hot");
        assert_eq!(temp_class(5000.0), "Torrid");
        assert_eq!(bulk_class("Selena"), "Terra");
        assert_eq!(bulk_class("Glacia"), "Aquaria");
        assert_eq!(bulk_class("Chthonia"), "Ferria");
        assert_eq!(bulk_class("jupiter"), "Jupiter");
        assert_eq!(bulk_class("Neptune"), "Neptune");
        assert_eq!(bulk_class("Asteroid"), "Asteroid");
        assert_eq!(bulk_class("???"), "Terra");
    }

    #[test]
    fn soils_parse() {
        let pml = r#"
Soil	"Mars red soil 1"
{
	Color (0.781 0.642 0.531 0.000)
	Slope 0.000
	ModColor	1.000
	ModRough	0.300
	ModNorm 	0.500
	ModHeight	0.250
	BiasRough	0.350
	ScaleRough	1.000
	BiasHeight	0.000
	ScaleHeight	0.800
	Layer "Detail"	{  12 1 "GroundAsphaltBroken001_COL" }
	Layer "Modul"	{ 128 0.5 "RockBrown006_COL" }
}
Soil "Mars red soil 1" { Color (0 0 0 0) Layer "Detail" { 1 1 "X_COL" } }
"#;
        let sp = SurfacePresets::from_texts(&[], &[("mars.pml".into(), pml.into())]);
        assert!(sp.warnings.is_empty(), "{:?}", sp.warnings);
        assert_eq!(sp.soils.len(), 2);
        let s = sp.soil("mars red SOIL 1").unwrap();
        assert_eq!(s.color, [0.781, 0.642, 0.531, 0.0]);
        assert_eq!((s.mod_rough, s.mod_norm, s.mod_height), (0.3, 0.5, 0.25));
        assert_eq!((s.bias_rough, s.scale_height), (0.35, 0.8));
        let d = s.detail.as_ref().unwrap();
        assert_eq!((d.freq, d.rot, d.texture.as_str()), (12.0, 1.0, "GroundAsphaltBroken001_COL"));
        let m = s.modul.as_ref().unwrap();
        assert_eq!((m.freq, m.rot), (128.0, 0.5));
        // Duplicates are kept in file order; `soil` returns the first.
        assert_eq!(sp.soils[1].detail.as_ref().unwrap().texture, "X_COL");
        assert!(sp.soil("nope").is_none());
    }

    #[test]
    fn sample_textures_are_distinct_and_sorted() {
        let sp = presets();
        let t = sp.sample_textures();
        assert_eq!(t, ["Desert002_COL", "Rock001_COL", "Upland002_COL"]);
    }

    #[test]
    fn unknown_keys_warn() {
        let mut w = Vec::new();
        let _ = parse_preset("x.cfg", "Bogus 1\nBiome \"Nowhere\" { Color (1 1 1 1) }", &mut w);
        assert_eq!(w.len(), 2, "{w:?}");
    }

    #[test]
    fn surf_style_reads_surface_block() {
        let root = sc::parse("Planet \"X\" { Radius 10 Surface { SurfStyle 0.37 Randomize (1 2 3) } }");
        let b = root.items[0].block.as_ref().unwrap();
        assert_eq!(surf_style(b), Some(0.37));
        assert_eq!(surf_style(b.sub("Surface").unwrap()), Some(0.37));
        let none = sc::parse("Planet \"Y\" { Radius 10 }");
        assert_eq!(surf_style(none.items[0].block.as_ref().unwrap()), None);
    }

    /// Needs the real install; run with `cargo test -p se_core --release -- --ignored biomes`.
    #[test]
    #[ignore]
    fn real_install_presets() {
        let root = std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine");
        let Ok(vfs) = Vfs::mount_install(root) else {
            eprintln!("install not found, skipping");
            return;
        };
        let sp = SurfacePresets::load(&vfs);
        assert!(sp.warnings.is_empty(), "{:#?}", sp.warnings);
        assert!((220..=230).contains(&sp.presets.len()), "{} presets", sp.presets.len());
        let pml_files = vfs.list_ext(MATERIALS_DIR, "pml").len();
        assert_eq!(pml_files, 6);
        assert!(sp.soils.len() > 50);
        assert!(sp.soil("Mars red soil 1").is_some());
        assert!(sp.sample_textures().len() > 10);
        let pick = |name: &str, parent: &str, class: &str| {
            let b = BodyInfo {
                name: name.into(),
                parent: parent.into(),
                class: class.into(),
                temperature_k: 250.0,
                ..Default::default()
            };
            sp.find_for(&b).map(|p| p.file.clone()).unwrap_or_default()
        };
        assert_eq!(pick("Moon", "Earth-Moon", "Selena"), "custom_Moon.cfg");
        assert_eq!(pick("Mars", "Sol", "Terra"), "custom_Mars.cfg");
        assert_eq!(pick("Earth", "Earth-Moon", "Terra"), "custom_Earth.cfg");
        // Terrestrial presets resolve all 21 biomes (custom_Mars has none: it only lists soils).
        let full = sp.presets.iter().filter(|p| p.biomes.len() == 21).count();
        assert!(full > 100, "{full} full presets");
    }
}
