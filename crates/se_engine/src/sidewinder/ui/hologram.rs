//! Elite's two holograms beside the scanner drawn natively (ARCHITECTURE.md A10 "The 3D parts (UI6)";
//! `re/elite/notes/ui_hud3d.md` sections 2 and 9): the own ship (right, inside the shield ring the gauge movie draws) and the
//! target (left).
//!
//! - **Elite's two steps:** a schematic model is rendered into a one-channel buffer of 512 x 512 (the templates' size, A) by
//!   the generation formula (`sidewinder_ui_holo_gen.wesl`, a pass of this file's own in the render world, before the opaque
//!   pass, only while seated), then shown on a billboard by the Sobel display shader (`sidewinder_ui_holo.wesl`) with the
//!   display material's own values (`ui/hud3d/holograms.json`), the GUI colour matrix and brightness, then the same last
//!   step as the panels' (`screen_fit`), by the holograms' own pass in the UI's place in the frame (`pass.rs`'s seam: after
//!   the scanner's pass, at the view's full size, display-referred; the view's group is the scanner's `PartsView`).
//! - **Which model:** the own ship's `sidewinder_schematic` (shipped with the ship), still (measured, notes 10). The target's
//!   by Elite's dispatch (`FUN_142a44e80`, notes 9.2) from the mod's one locked destination: nothing locked, or another
//!   system's star (no body object to resolve: B), the Orrery; a star by its kind (white dwarf, neutron star, black hole,
//!   any other star: A for the rule, B for SpaceEngine's kinds); a planet or a moon the sphere with its class's surface data
//!   from the 27-entry table, the class byte from SpaceEngine's class by `ELITE_CLASS` (B, every row stated there).
//! - **Motion:** the star models scroll their height maps by their materials' speeds (A); nothing turns: the notes found no
//!   rotation in code or data, and session ui1's three pictures a target, 3 s apart, cannot give a rate (C: "turning or
//!   shimmering slowly" without a number), so the models are still (labelled, E).
//! - **What was not found and is measured or a stand-in** (each constant below says which): the generation's view (its turn,
//!   measured from session ui1's pictures, C; orthographic and fitted to the model's bounding sphere, E), the billboard's place
//!   and size (measured, C), the buffer's clear value (any value from 1,000 up gives the same picture: the shader's caps,
//!   E), the display's blend (the GUI materials' premultiplied blend, E), the edge colours (the data's run-time colours: the
//!   stellar record's two for the target, the SchematicsComponent's for the own ship, E for which slot each fills).
//! - **When:** made once the UI is made, drawn while seated on the frames the UI's pass draws (`UiFrame::order`); with
//!   `SE_SW_UI=0` nothing of this runs.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::math::Vec4;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewTarget, ViewUniformOffset};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderDefVal;
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use bytemuck::{Pod, Zeroable};
use se_core::catalog::ObjectRef;
use se_core::elite::to_mod_space;
use se_core::elite_asset::{parse_edm, Edm};
use se_core::star::StarKind;

use crate::color_correction::ColorCorrection;
use crate::data::SeData;
use crate::viltrumite::draw::MainView;

use super::super::draw::ShipDraw;
use super::super::parts::Parts;
use super::super::pilot::Pilot;
use super::super::SidewinderAssets;
use super::load::{read, texture};
use super::pass::{UiFrame, HDR_FORMAT};
use super::place::UiDraw;
use super::scanner::{json_array, json_attrs, json_f32, json_field, json_rows, json_str, PartsView, ScannerPassSet};

pub const HOLO_SHADER: &str = "shaders/sidewinder_ui_holo.wesl";
pub const GEN_SHADER: &str = "shaders/sidewinder_ui_holo_gen.wesl";
const HOLO_FILE: &str = "ui/hud3d/holograms.json";
const OWN_MODEL: &str = "models/sidewinder_schematic.edm";
const OWN_MATERIAL: &str = "schematic";

/// The generated buffer's side: both templates' size (`schematics_component`, 512 x 512, A).
const BUFFER: u32 = 512;
/// The buffer's clear value (E: not found). The display counts a neighbour under 100 as the model and the middle under 1,000
/// as covered, and its edge term saturates long before a step of 1,000: any value from 1,000 up gives the same picture.
const EMPTY: f32 = 10_000.0;
/// The generation's camera: orthographic along its -z, the model's bounding sphere filling the buffer, the eye this many
/// radii from the sphere's middle (E: the view matrix was not found, notes 8.4; the depth's offset only moves every value
/// alike, which the display's differences do not see).
const CAMERA_DISTANCE: f32 = 3.0;

// ---- Measured (C, session ui1's `a00_start.png`; the report gives each fit and its error) ----

/// The generation's view: yaw about the model's up, then pitch about the view's right, then roll about the view's axis
/// (degrees), each hologram's own (the two templates are identical, but the pictures fit each its own).
/// Fitted (`.optimize/scratch/run/ui6/tools/viewfit.py`: the model's vertices projected against the hologram's pixels, the
/// score the mean nearest distance both ways): the own ship (26, -10, -4), 5.1 px, each angle good to (-22 +16, +-2, +-5)
/// degrees for a 1 px worse score (its width dominates the outline, so its yaw is loose); the Orrery's rings alone (its
/// moons hang on a bone of their own and stand elsewhere in Elite's picture) (330, -59, 3), 5.6 px, (+-4, -3 +2, +-3).
pub const OWN_VIEW_DEG: [f32; 3] = [26.0, -10.0, -4.0];
pub const TARGET_VIEW_DEG: [f32; 3] = [330.0, -59.0, 3.0];
/// The view of the bodies and stars (the target models other than the Orrery). Their spheres give no turn of their own, but
/// the gas giant's bands on session ui1's `p1_planet_1`..`3` do: straight through the middle (the view on the body's equator:
/// pitch 0, B: their curvature was not fitted) and turned -7.8 degrees on the screen (the bands' mean direction by their
/// gradients inside the disc, -7.9, -7.8, -7.5 on the three pictures, coherence 0.77: C, +-0.3), so a roll of 7.8. The yaw is
/// the body's own turn, which the pictures cannot give (the body turns in Elite): 0, the decode's (E).
pub const BODY_VIEW_DEG: [f32; 3] = [0.0, 0.0, 7.8];
/// The billboard's middle in its panel bone's frame (metres) and its side (metres): where the generated buffer lies.
/// Elite's buffers are 244 px on the picture, both (the fits' 122 and 123 px for the bounding sphere's radius, which fills
/// half the buffer: the same size, as the two identical templates say), their middles at (1193, 1146.5) and (2262, 1151).
pub const OWN_PLACE_M: [f32; 3] = [-0.0065, 0.0025, 0.0];
pub const OWN_SIDE_M: f32 = 0.285;
pub const TARGET_PLACE_M: [f32; 3] = [0.003, -0.002, 0.0];
pub const TARGET_SIDE_M: f32 = 0.272;
/// The generated depth's scale: the view-space depth of the normalised model (its bounding sphere's radius 1) times this,
/// before the material's offsets are added (Elite's depth unit in this pass was not found). Measured between two runs (C,
/// two points only): inside the own ship's box on `a00_start.png` 29.2 % of the pixels are over 40 levels, mean (91, 13, 1);
/// at 1 (the first run) the whole model lit, at 0.2 54 % and (88, 48, 4) with the edge tint below its first colour, at 0.05
/// 31.6 % and (73, 7, 12) with the edge tint its third (the nearer to Elite's).
const DEPTH_SCALE: f32 = 0.05;
/// The own ship's tint's green over its red, both slots (C, fitted). Elite's own-ship pixels on `a00_start.png`, by level of red: 180-230 (205, 37,
/// 0), 120-180 (143, 15, 0), 70-120 (90, 4, 0), 40-70 (54, 1, 0); never more than 0.2 levels of blue. Brought back through the
/// screen fit (`ui::material::SCREEN_FIT`) each band's green over red in the value is 0.068, 0.088, 0.083: one colour, the
/// shoulder redder only through the fit's toe, so both slots hold the same. (The brightest band, 246, 81, 0, stands past the
/// fit's ceiling of 230: bloom, not used.) None of the SchematicsComponent block's colours gives it: (0.31, 0.12, 0) has
/// green over red 0.39 (ours: (184, 118, 0) at the top band); (0.3, 0.015, 0.03) would show 53 levels of blue in the 180-230
/// band and 6 in the 70-120 band, Elite 0; (0.138, 0.1355, 0.153) is blue-grey. So the red is the block's first colour's
/// (0.31: read from the shipped block) and its green 0.08 +-0.01 of it, no blue.
pub const OWN_GREEN_OVER_RED: f32 = 0.08;
/// How much of the generated buffer the model's bounding sphere fills (its radius over the buffer's half side): not in the
/// decode, which holds the buffer's size only (the templates' 512 x 512, A). Fitted (C) so each model's size on screen is
/// Elite's on session ui1's pictures (width x height of the outline's pixels, after the camera fit from the canopy lamps;
/// +-2 px each): the own ship 205 px long on `a00_start.png` against our 210.5 at 1 (0.974); the Orrery 229 x 152 against
/// 223 x 148 (1.026); the planets, all one sphere, the gas giant 181 x 176 on `p1_planet_1` and the rocky moon 201 x 196 on
/// `p1_moon_1` against our 236 x 216 for both (0.77 and 0.85: 0.81 +-0.04, their mean; Elite's two differ by 11 %); the star
/// 236 x 206 on `p1_star_1` (its loops included) against 219 x 204 (1.04 +-0.03, from the width).
pub const OWN_FILL: f32 = 0.974;
pub const ORRERY_FILL: f32 = 1.026;
pub const PLANET_FILL: f32 = 0.81;
pub const STAR_FILL: f32 = 1.04;

// ---- The file ----

#[derive(Debug, Clone)]
pub(in crate::sidewinder) struct Display {
    pub material: String,
    pub shader: String,
    pub attrs: BTreeMap<String, Vec<f64>>,
}

/// One model's generation values (its `Schematic_Generation_Core` material).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::sidewinder) struct Generation {
    /// pSchematic_Data (D, P, Z, W).
    pub data: [f32; 4],
    pub scroll: [f32; 2],
    pub tiling: f32,
    pub heightmap: String,
    /// The model's file.
    pub file: Option<String>,
}

impl Generation {
    fn read(v: &serde_json::Value, f: &str) -> Result<Generation, String> {
        Ok(Generation {
            data: json_array::<4>(json_field(v, "data", f)?, f)?,
            scroll: json_array::<2>(json_field(v, "scroll", f)?, f)?,
            tiling: json_f32(json_field(v, "tiling", f)?, f)?,
            heightmap: json_str(v, "heightmap", f)?,
            file: v.get("file").and_then(|x| x.as_str()).map(str::to_string),
        })
    }
}

/// A row of the planet class table: its five key bytes and its model.
#[derive(Debug, Clone)]
pub(in crate::sidewinder) struct ClassRow {
    pub key: [u8; 5],
    pub model: String,
}

#[derive(Debug, Clone)]
pub(in crate::sidewinder) struct Colours {
    /// The StellarSchematicsData record's two colours (the target panel's with no target).
    pub stellar: [[f32; 3]; 2],
    /// The cockpit's SchematicsComponent colour block (+0x118, +0x128, +0x138).
    pub component: [[f32; 3]; 3],
}

/// `ui/hud3d/holograms.json` (written by the converter, `convert/ui_hud3d.rs`).
#[derive(Debug, Clone)]
pub(in crate::sidewinder) struct HoloFile {
    pub format: u32,
    pub display: BTreeMap<String, Display>,
    pub scanlines: String,
    pub target: BTreeMap<String, Generation>,
    pub planet_classes: Vec<ClassRow>,
    pub class_data: BTreeMap<String, Generation>,
    pub colours: Colours,
}

/// The target models a destination can show (`HoloFile::target`'s keys).
const ORRERY: &str = "orrery";

impl HoloFile {
    pub fn parse(bytes: &[u8]) -> Result<HoloFile, String> {
        let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| format!("{HOLO_FILE}: {e}"))?;
        let ff = HOLO_FILE;
        let object = |k: &str| json_field(&v, k, ff)?.as_object().ok_or_else(|| format!("{ff}: {k} is not an object"));
        let generations = |k: &str| -> Result<BTreeMap<String, Generation>, String> { object(k)?.iter().map(|(n, g)| Ok((n.clone(), Generation::read(g, n)?))).collect() };
        let colours = json_field(&v, "colours", ff)?;
        let rows = json_field(&v, "planet_classes", ff)?.as_array().ok_or_else(|| format!("{ff}: planet_classes is not a list"))?;
        let f = HoloFile {
            format: json_field(&v, "format", ff)?.as_u64().unwrap_or(0) as u32,
            display: object("display")?.iter().map(|(k, d)| Ok((k.clone(), Display { material: json_str(d, "material", k)?, shader: json_str(d, "shader", k)?, attrs: json_attrs(d, k)? }))).collect::<Result<_, String>>()?,
            scanlines: json_str(&v, "scanlines", ff)?,
            target: generations("target")?,
            planet_classes: rows
                .iter()
                .map(|r| {
                    let key = json_array::<5>(json_field(r, "key", ff)?, ff)?;
                    Ok(ClassRow { key: key.map(|b| b as u8), model: json_str(r, "model", ff)? })
                })
                .collect::<Result<_, String>>()?,
            class_data: generations("class_data")?,
            colours: Colours {
                stellar: json_rows::<3>(colours, "stellar", ff)?.try_into().map_err(|_| format!("{ff}: two stellar colours"))?,
                component: json_rows::<3>(colours, "component", ff)?.try_into().map_err(|_| format!("{ff}: three component colours"))?,
            },
        };
        if f.format != 1 {
            return Err(format!("{HOLO_FILE}: format {}", f.format));
        }
        for k in ["own", "target"] {
            let d = f.display.get(k).ok_or_else(|| format!("{HOLO_FILE}: no display {k}"))?;
            if d.shader != "Schematic_Display_BillboardSobel" {
                return Err(format!("{HOLO_FILE}: display {k} is {}", d.shader));
            }
            HoloParams::new(d, [[1.0; 3]; 2], 0.0, 0.0).map_err(|e| format!("{HOLO_FILE} display {k}: {e}"))?;
        }
        for m in [ORRERY, "starschematic", "white_dwarf_schematic", "neutronstarschematic", "blackholeschematic"] {
            if f.target.get(m).and_then(|g| g.file.as_ref()).is_none() {
                return Err(format!("{HOLO_FILE}: no target model {m}"));
            }
        }
        for r in &f.planet_classes {
            if f.class_data.get(&r.model.to_lowercase()).and_then(|g| g.file.as_ref()).is_none() {
                return Err(format!("{HOLO_FILE}: the class row {:?} names {}, which has no data", r.key, r.model));
            }
        }
        Ok(f)
    }

}

// ---- Which model ----

/// The key bytes' "any" values (notes 9.2: 0x15 / 4 / 0xf / 4 for bytes 1 to 4).
const ANY: [u8; 4] = [0x15, 4, 0x0f, 4];

/// What a body's own `Life` blocks say, as the class table can use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    None,
    /// Multicellular or intelligent life on land, or a civilization: the scatter's own rule (`scatter::land_plants`).
    Land,
    /// Life of SpaceEngine's class Organic anywhere else (in the air, the sea, under the ice).
    Organic,
    /// Life of SpaceEngine's class Exotic anywhere else.
    Exotic,
}

impl Life {
    pub fn of(block: &se_core::sc::Block, civilization: bool) -> Life {
        if civilization || crate::scatter::land_plants(block) {
            return Life::Land;
        }
        let classes: Vec<String> = se_core::body_params::LifeParams::all_from_body(block).iter().map(|l| l.class.as_deref().unwrap_or("").to_ascii_lowercase()).collect();
        if classes.iter().any(|c| c == "organic") {
            Life::Organic
        } else if classes.iter().any(|c| c == "exotic") {
            Life::Exotic
        } else {
            Life::None
        }
    }
}

/// SpaceEngine's planet class (the catalog's `Class`, any case) and life as Elite's planet class (the table's first key byte).
/// Elite's names for the numbers are its own (A: `FUN_14138fc60` builds `StellarForge_` + the class name,
/// `re/elite/notes/ui_messages_screens.md` 7.7.2): 0 EarthLike, 1 WaterWorld, 2 AmmoniaWorld, 3 MetalRich, 4 HighMetalContent,
/// 5 Rocky, 6 Icy, 7 RockyIcy, 8 WaterGiant, 9 WaterGiantLife, 10 WaterLife (a gas giant with water-based life), 11
/// AmmoniaLife, 12..16 SudarskyClassI..V, 17 HeliumRich, 18 Helium. Which SpaceEngine class is which is B for every row:
/// - life on land or a civilization (Earth), whatever the class -> 0 EarthLike (model `Terrestrial_HabitableOutdoorWorld`;
///   SpaceEngine's Terra is any rocky world, the Moon and Mars too);
/// - a giant (Jupiter, Neptune) with Organic life -> 10 WaterLife (`Jovian_WaterGiant_WithLife`), with Exotic life -> 11
///   AmmoniaLife (`JovianClassification_AmmoniaLife`);
/// - Aquaria (an ocean world) -> 1 WaterWorld (`Terrestrial_WaterWorld`);
/// - Ferria, Chthonia (iron worlds, stripped cores) -> 3 MetalRich (`Terrestrial_MetalRich`);
/// - Glacia (an ice world; life under its ice changes nothing) -> 6 Icy (`Terrestrial_IceMantle`);
/// - Jupiter, Neptune (gas and ice giants) -> 12 SudarskyClassI (`JovianClassification_SudarskyClassI`: Elite names the
///   Sun's four giants so);
/// - Terra, Selena, Carbonia, Desert and anything else, a body with no class too -> 5 Rocky (`Terrestrial_Rocky`).
pub const ELITE_CLASS: [(&str, u8); 7] = [("aquaria", 1), ("ferria", 3), ("chthonia", 3), ("glacia", 6), ("jupiter", 12), ("neptune", 12), ("terra", ROCKY)];
/// 5 Rocky.
const ROCKY: u8 = 5;
/// 0 EarthLike.
const EARTH_LIKE: u8 = 0;
/// 10 WaterLife.
const GIANT_ORGANIC: u8 = 10;
/// 11 AmmoniaLife.
const GIANT_EXOTIC: u8 = 11;

pub fn elite_class(se_class: Option<&str>, life: Life) -> u8 {
    let c = se_class.unwrap_or("").trim().to_lowercase();
    let giant = c == "jupiter" || c == "neptune";
    match life {
        Life::Land => EARTH_LIKE,
        Life::Organic if giant => GIANT_ORGANIC,
        Life::Exotic if giant => GIANT_EXOTIC,
        _ => ELITE_CLASS.iter().find(|(n, _)| *n == c).map_or(ROCKY, |(_, b)| *b),
    }
}

/// The table's first row matching a body of class `class` whose sub-classes the mod does not know (given as "any"): a row
/// matches when each byte is the body's or "any" (first match wins, `FUN_142a44e80`). None: the panel's default.
pub fn class_model(table: &[ClassRow], class: u8) -> Option<&str> {
    let body = [class, ANY[0], ANY[1], ANY[2], ANY[3]];
    table.iter().find(|r| r.key[0] == body[0] && (1..5).all(|k| r.key[k] == body[k] || r.key[k] == ANY[k - 1])).map(|r| r.model.as_str())
}

/// What the target hologram shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// A target model (`HoloFile::target`'s key).
    Model(&'static str),
    /// The planet sphere with this class's data (`HoloFile::class_data`'s key).
    Planet(String),
}

/// Elite's dispatch for the mod's locked destination.
fn shown(pilot: Option<&Pilot>, data: &SeData, table: &[ClassRow]) -> Shown {
    let Some(obj) = pilot.and_then(|p| p.nav_body) else { return Shown::Model(ORRERY) };
    match obj {
        ObjectRef::Star(i) => Shown::Model(match data.catalog.stars.get(i as usize).map(|s| s.class.kind) {
            Some(StarKind::WhiteDwarf) => "white_dwarf_schematic",
            Some(StarKind::NeutronStar) => "neutronstarschematic",
            Some(StarKind::BlackHole) => "blackholeschematic",
            _ => "starschematic",
        }),
        ObjectRef::Body(i) => {
            let body = data.catalog.bodies.get(i as usize);
            let class = body.and_then(|b| b.class.as_deref());
            let life = body.map_or(Life::None, |b| Life::of(&b.block, b.civilization().is_some()));
            class_model(table, elite_class(class, life)).map_or(Shown::Model(ORRERY), |m| Shown::Planet(m.to_lowercase()))
        }
        _ => Shown::Model(ORRERY),
    }
}

// ---- The generation's data ----

/// A vertex of the generation pass: position, normal (the mod's axes, the model normalised to its bounding sphere), uv0.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GenVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

pub struct GenMesh {
    pub vertices: Vec<GenVertex>,
    pub indices: Vec<u32>,
}

/// A schematic model's triangles in the mod's axes, normalised: its bounding box's middle at the origin, the farthest vertex
/// at 1 (the view's fit: E).
fn gen_mesh(edm: &Edm, file: &str) -> Result<GenMesh, String> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for d in &edm.draws {
        let mut v: Vec<_> = d.vertices.iter().map(|v| v.decode()).collect();
        let mut i = d.indices.clone();
        if i.len() % 3 != 0 || i.iter().any(|&k| k as usize >= v.len()) {
            return Err(format!("{file}: a draw has a broken triangle list"));
        }
        to_mod_space(&mut v, &mut i);
        let base = vertices.len() as u32;
        indices.extend(i.iter().map(|&k| base + u32::from(k)));
        vertices.extend(v.iter().map(|v| GenVertex { position: v.position, normal: v.normal, uv: v.uv[0] }));
    }
    let (lo, hi) = vertices.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| (lo.min(Vec3::from_array(v.position)), hi.max(Vec3::from_array(v.position))));
    let middle = (lo + hi) * 0.5;
    let radius = vertices.iter().map(|v| (Vec3::from_array(v.position) - middle).length()).fold(0.0f32, f32::max);
    if vertices.is_empty() || radius <= 0.0 {
        return Err(format!("{file}: no vertices"));
    }
    for v in &mut vertices {
        v.position = ((Vec3::from_array(v.position) - middle) / radius).to_array();
    }
    Ok(GenMesh { vertices, indices })
}

/// The view's turn (degrees: yaw about up, pitch about right, roll about the view's axis).
fn view_turn(deg: [f32; 3]) -> Quat {
    let [yaw, pitch, roll] = deg.map(f32::to_radians);
    Quat::from_rotation_z(roll) * Quat::from_rotation_x(pitch) * Quat::from_rotation_y(yaw)
}

/// `sidewinder_ui_holo_gen.wesl` `GenUniform`.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct GenUniformGpu {
    /// The normalised model to the view (the eye at +z `CAMERA_DISTANCE`, looking down -z).
    pub view_from_model: Mat4,
    /// x the view's half side (1 over the fill: the bounding sphere's share of the buffer), y the depth's scale, z the eye's
    /// distance, w the far plane.
    pub ortho: Vec4,
    /// pSchematic_Data (D, P, Z, W).
    pub data: Vec4,
    /// xy the height map's scroll now (speed x time, within one repeat), z pVerticalTiling.
    pub scroll: Vec4,
}

/// One hologram's generation this frame.
#[derive(Clone)]
pub struct GenSlot {
    pub mesh: Arc<GenMesh>,
    pub heightmap: Handle<Image>,
    pub target: Handle<Image>,
    pub uniform: GenUniformGpu,
}

/// What the render world draws into the two buffers this frame (own ship, target); None: not drawn.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct HoloGen {
    pub slots: [Option<GenSlot>; 2],
}

// ---- The display's values ----

/// `sidewinder_ui_holo.wesl` `HoloParams`: the display material's own values.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct HoloParams {
    /// x pSobelTintCoordScale, y pSobelPower, z pSobelCoreIntensity, w pSobelEdgeIntensity.
    pub sobel: Vec4,
    /// x pSobelDistanceScale, y pSobelMaxDistance, z pSobelEdgeThickness, w pDestAlphaScale.
    pub dist: Vec4,
    /// rgb pSobelTintLow, w pSchematicPhaseInLerpValue.
    pub low: Vec4,
    /// rgb pSobelTintHigh, w pSchematicOpacity.
    pub high: Vec4,
    /// xy pScanlineTile, zw pScanlineOffset.
    pub scan: Vec4,
    /// xy pScanlineScaleOffset, z the hologram (0 own, 1 target), w the billboard's half side (metres).
    pub scan_scale: Vec4,
    /// xyz pReferenceRight, pReferenceUp, as the material holds them (in the render frame's axes: the decode dots them with
    /// the view ray's point, in the space of `cb1[279]`, the camera-relative world).
    pub right: Vec4,
    pub up: Vec4,
    /// x the generated depth's scale (`DEPTH_SCALE`): the scanlines read the buffer's depth without it.
    pub depth: Vec4,
}

impl HoloParams {
    /// From the material's attributes, with the run-time tints (low, high) in place of its white ones.
    pub fn new(d: &Display, tints: [[f32; 3]; 2], slot: f32, half_side: f32) -> Result<HoloParams, String> {
        let v = |n: &str, k: usize| -> Result<Vec4, String> {
            let a = d.attrs.get(n).filter(|a| a.len() >= k).ok_or_else(|| format!("{}: {n} has fewer than {k} values", d.material))?;
            Ok(Vec4::from_array(std::array::from_fn(|i| a.get(i).filter(|_| i < k).map_or(0.0, |x| *x as f32))))
        };
        let s = |n: &str| v(n, 1).map(|x| x.x);
        let tile = v("pScanlineTile", 2)?;
        let offset = v("pScanlineOffset", 2)?;
        let so = v("pScanlineScaleOffset", 2)?;
        let low = v("pSobelTintLow", 3)?.truncate() * Vec3::from_array(tints[0]);
        let high = v("pSobelTintHigh", 3)?.truncate() * Vec3::from_array(tints[1]);
        Ok(HoloParams {
            sobel: Vec4::new(s("pSobelTintCoordScale")?, s("pSobelPower")?, s("pSobelCoreIntensity")?, s("pSobelEdgeIntensity")?),
            dist: Vec4::new(s("pSobelDistanceScale")?, s("pSobelMaxDistance")?, s("pSobelEdgeThickness")?, s("pDestAlphaScale")?),
            low: low.extend(s("pSchematicPhaseInLerpValue")?),
            high: high.extend(s("pSchematicOpacity")?),
            scan: Vec4::new(tile.x, tile.y, offset.x, offset.y),
            scan_scale: Vec4::new(so.x, so.y, slot, half_side),
            right: v("pReferenceRight", 3)?,
            up: v("pReferenceUp", 3)?,
            depth: Vec4::new(DEPTH_SCALE, 0.0, 0.0, 0.0),
        })
    }
}

/// `sidewinder_ui_holo.wesl` `places`: each hologram's billboard this frame (own ship, target), written in place: xyz its
/// middle in the frame's world, w the ship's own scale.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct HoloFrameGpu {
    pub places: [[f32; 4]; 2],
}

/// The holograms' display, made once: each one's values (`HoloParams`, encoded), its generated buffer, the scanlines.
pub struct HoloParts {
    params: [Vec<u8>; 2],
    generated: [Handle<Image>; 2],
    scanlines: Handle<Image>,
}

/// What the holograms' pass draws this frame, made by `update` and extracted: the parts (None: no holograms), the places, and
/// which of the two are shown.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct HoloShow {
    parts: Option<Arc<HoloParts>>,
    frame: HoloFrameGpu,
    shown: [bool; 2],
}

// ---- Loading and making ----

/// A model and its generation values, read.
struct ModelRead {
    mesh: Arc<GenMesh>,
    generation: Generation,
    heightmap: Image,
}

struct Read {
    file: HoloFile,
    layout: se_core::elite_ui_layout::Layout,
    own: ModelRead,
    /// The target models by `HoloFile::target`'s key, and each class's sphere by `class_data`'s key.
    targets: HashMap<String, ModelRead>,
    classes: HashMap<String, ModelRead>,
    scanlines: Image,
}

fn read_model(file: &str, generation: Generation) -> Result<ModelRead, String> {
    let edm = parse_edm(&read(file)?).map_err(|e| format!("{file}: {e}"))?;
    Ok(ModelRead { mesh: Arc::new(gen_mesh(&edm, file)?), heightmap: texture(&generation.heightmap, false, true)?, generation })
}

/// The own ship's generation values from the ship's `materials.json` (its `schematic`).
fn own_generation() -> Result<Generation, String> {
    let mats = se_core::elite_ship::parse_materials(&read("materials.json")?).map_err(|e| e.to_string())?;
    let m = mats.materials.get(OWN_MATERIAL).ok_or("materials.json: no schematic")?;
    let attr = |n: &str| m.attrs.iter().find(|a| a.name == n).map(|a| a.values.iter().map(|v| *v as f32).collect::<Vec<_>>()).ok_or_else(|| format!("schematic: no {n}"));
    let tex = m
        .textures
        .iter()
        .find(|t| t.slot == "pHeightMapTexture")
        .and_then(|t| match &t.value {
            se_core::elite_ship::SlotValue::Tex { file, .. } => Some(file.clone()),
            _ => None,
        })
        .ok_or("schematic: no height map")?;
    let (d, s, t) = (attr("pSchematic_Data")?, attr("pHeightMapUVScrollSpeed")?, attr("pVerticalTiling")?);
    if d.len() < 4 || s.len() < 2 || t.is_empty() {
        return Err("schematic: short values".into());
    }
    Ok(Generation { data: [d[0], d[1], d[2], d[3]], scroll: [s[0], s[1]], tiling: t[0], heightmap: tex, file: Some(OWN_MODEL.into()) })
}

fn load() -> Result<Read, String> {
    let file = HoloFile::parse(&read(HOLO_FILE)?)?;
    let layout = se_core::elite_ui_layout::parse_layout(&read("ui/layout.json")?).map_err(|e| e.to_string())?;
    let own = read_model(OWN_MODEL, own_generation()?)?;
    let mut targets = HashMap::new();
    for (k, g) in &file.target {
        let f = g.file.clone().ok_or_else(|| format!("{k}: no file"))?;
        targets.insert(k.clone(), read_model(&f, g.clone())?);
    }
    let mut classes = HashMap::new();
    for (k, g) in &file.class_data {
        let f = g.file.clone().ok_or_else(|| format!("{k}: no file"))?;
        classes.insert(k.clone(), read_model(&f, g.clone())?);
    }
    let scanlines = texture(&file.scanlines, false, true)?;
    Ok(Read { file, layout, own, targets, classes, scanlines })
}

#[derive(Resource)]
struct HoloLoading(Task<Result<Read, String>>);

#[derive(Resource)]
struct HoloOff;

/// A model ready to generate: its triangles, values and height map.
struct Model {
    mesh: Arc<GenMesh>,
    generation: Generation,
    heightmap: Handle<Image>,
}

/// One hologram, made.
struct Holo {
    node: usize,
    /// The billboard's middle in the node's model space.
    frame: Mat4,
    target: Handle<Image>,
    /// The generation's view: the own ship's, or the Orrery's (the bodies' is `BODY_VIEW_DEG`).
    view: Quat,
}

/// The holograms, once made.
#[derive(Resource)]
pub struct Holograms {
    own: Holo,
    other: Holo,
    own_model: Model,
    targets: HashMap<String, Model>,
    classes: HashMap<String, Model>,
    table: Vec<ClassRow>,
    pub shown: Shown,
    clock: f64,
}

/// A target image: the generated buffer (R32Float, drawn into by the generation pass, read by the display).
fn target_image() -> Image {
    let mut image = Image::new_fill(
        Extent3d { width: BUFFER, height: BUFFER, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &EMPTY.to_le_bytes(),
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    image
}

fn load_due(ui: Option<Res<UiDraw>>, holo: Option<Res<Holograms>>, loading: Option<Res<HoloLoading>>, off: Option<Res<HoloOff>>) -> bool {
    ui.is_some() && holo.is_none() && loading.is_none() && off.is_none()
}

fn start_load(mut commands: Commands) {
    commands.insert_resource(HoloLoading(AsyncComputeTaskPool::get().spawn(async move { load() })));
}

fn build(world: &mut World) {
    let Some(mut loading) = world.get_resource_mut::<HoloLoading>() else { return };
    let Some(done) = futures::check_ready(&mut loading.0) else { return };
    world.remove_resource::<HoloLoading>();
    if let Err(e) = done.and_then(|r| make(world, r)) {
        warn!("sidewinder ui: the holograms are off: {e}");
        world.insert_resource(HoloOff);
    }
}

/// A panel bone's frame times the billboard's place, in the cockpit rig's model space.
fn place_frame(layout: &se_core::elite_ui_layout::Layout, bone: &str, place: [f32; 3]) -> Result<Mat4, String> {
    let b = layout.bone_frame(bone, se_core::elite_ui_layout::Rig::Cockpit).ok_or_else(|| format!("the layout has no {bone}"))?;
    Ok(super::place::bone_matrix(b) * Mat4::from_translation(Vec3::from_array(place)))
}

fn make(world: &mut World, r: Read) -> Result<(), String> {
    let node = world.resource::<SidewinderAssets>().rig.cockpit_node().ok_or("the cockpit's root is not in the ship's rig")?;
    let c = &r.file.colours;
    let mut images = world.resource_mut::<Assets<Image>>();
    let scanlines = images.add(r.scanlines);
    let mut add_model = |m: ModelRead| Model { mesh: m.mesh, generation: m.generation, heightmap: images.add(m.heightmap) };
    let own_model = add_model(r.own);
    let targets: HashMap<String, Model> = r.targets.into_iter().map(|(k, m)| (k, add_model(m))).collect();
    let classes: HashMap<String, Model> = r.classes.into_iter().map(|(k, m)| (k, add_model(m))).collect();
    let buffers = [images.add(target_image()), images.add(target_image())];
    // (The run-time tints replace the material's white pSobelTintLow (cb2[14]: the core's) and pSobelTintHigh (cb2[15]: the
    // edge's); which run-time colour fills which is not in the decode. The own ship's: `OWN_GREEN_OVER_RED`'s, fitted. The target's
    // are the stellar record's two, low the blue-grey (0.238, 0.2855, 0.353), high the grey (0.5, 0.5, 0.5): E for the
    // order; its outline is white, as measured.)
    let own_tint = [c.component[0][0], OWN_GREEN_OVER_RED * c.component[0][0], 0.0];
    let sets = [
        ("own", "!CockpitSchematic_Panel", OWN_PLACE_M, OWN_SIDE_M, [own_tint, own_tint], OWN_VIEW_DEG),
        ("target", "!CockpitSchematicTarget_Panel", TARGET_PLACE_M, TARGET_SIDE_M, [c.stellar[1], c.stellar[0]], TARGET_VIEW_DEG),
    ];
    let mut holos = Vec::new();
    let mut params = Vec::new();
    for (k, (name, bone, place, side, tints, view)) in sets.into_iter().enumerate() {
        params.push(crate::stars::gpu_bytes(&HoloParams::new(&r.file.display[name], tints, k as f32, side * 0.5)?));
        holos.push(Holo { node, frame: place_frame(&r.layout, bone, place)?, target: buffers[k].clone(), view: view_turn(view) });
    }
    let [own, other]: [Holo; 2] = holos.try_into().map_err(|_| "two holograms")?;
    let params: [Vec<u8>; 2] = params.try_into().map_err(|_| "two holograms")?;
    world.insert_resource(HoloShow { parts: Some(Arc::new(HoloParts { params, generated: buffers, scanlines })), ..default() });
    info!("sidewinder ui: the holograms are made ({} target models, {} planet classes), drawn by their pass while seated", targets.len(), r.file.class_data.len());
    world.insert_resource(Holograms {
        own,
        other,
        own_model,
        targets,
        classes,
        table: r.file.planet_classes,
        shown: Shown::Model(ORRERY),
        clock: 0.0,
    });
    Ok(())
}

fn teardown(world: &mut World) {
    world.remove_resource::<Holograms>();
    world.insert_resource(HoloGen::default());
    world.insert_resource(HoloShow::default());
    world.remove_resource::<HoloLoading>();
    world.remove_resource::<HoloOff>();
}

fn ui_gone(ui: Option<Res<UiDraw>>, holo: Option<Res<Holograms>>, off: Option<Res<HoloOff>>) -> bool {
    ui.is_none() && (holo.is_some() || off.is_some())
}

/// A slot's generation this frame.
fn slot(mesh: &Arc<GenMesh>, g: &Generation, heightmap: &Handle<Image>, holo: &Holo, (view, fill): (Quat, f32), clock: f64) -> GenSlot {
    let scroll = |s: f32| ((f64::from(s) * clock) % 1.0) as f32;
    GenSlot {
        mesh: mesh.clone(),
        heightmap: heightmap.clone(),
        target: holo.target.clone(),
        uniform: GenUniformGpu {
            view_from_model: Mat4::from_translation(Vec3::new(0.0, 0.0, -CAMERA_DISTANCE)) * Mat4::from_quat(view),
            ortho: Vec4::new(1.0 / fill, DEPTH_SCALE, CAMERA_DISTANCE, 2.0 * CAMERA_DISTANCE),
            data: Vec4::from_array(g.data),
            scroll: Vec4::new(scroll(g.scroll[0]), scroll(g.scroll[1]), g.tiling, 0.0),
        },
    }
}

/// Each frame once made: the target's model from the lock, the generation's slots, the billboards placed and shown.
#[allow(clippy::too_many_arguments)]
fn update(
    mut holo: ResMut<Holograms>,
    mut holo_gen: ResMut<HoloGen>,
    mut show: ResMut<HoloShow>,
    pilot: Option<Res<Pilot>>,
    data: Option<Res<SeData>>,
    draw: Option<Res<ShipDraw>>,
    parts: Res<Parts>,
    seat: Res<super::super::seat::Seat>,
    control: Res<crate::mods::ModControl>,
    time: Res<Time<Real>>,
) {
    let h = &mut *holo;
    let seated = super::seated(&seat, &control);
    let ship = draw.as_ref().filter(|d| d.seat_shown()).and_then(|d| d.frame());
    let on = seated && ship.is_some();
    if let Some(data) = data.as_ref() {
        let now = shown(pilot.as_deref(), data, &h.table);
        if now != h.shown {
            info!("sidewinder ui: the target hologram shows {now:?}");
            h.shown = now;
        }
    }
    if on {
        h.clock += f64::from(time.delta_secs());
        let own = slot(&h.own_model.mesh, &h.own_model.generation, &h.own_model.heightmap, &h.own, (h.own.view, OWN_FILL), h.clock);
        let body = view_turn(BODY_VIEW_DEG);
        let other = match &h.shown {
            Shown::Model(m) => {
                let look = if *m == ORRERY { (h.other.view, ORRERY_FILL) } else { (body, STAR_FILL) };
                h.targets.get(*m).map(|t| slot(&t.mesh, &t.generation, &t.heightmap, &h.other, look, h.clock))
            }
            Shown::Planet(c) => h.classes.get(c).map(|t| slot(&t.mesh, &t.generation, &t.heightmap, &h.other, (body, PLANET_FILL), h.clock)),
        };
        holo_gen.slots = [Some(own), other];
    } else if holo_gen.slots.iter().any(Option::is_some) {
        holo_gen.slots = [None, None];
    }
    let shown = [on, on && holo_gen.slots[1].is_some()];
    // (The billboard's middle and the model matrix's scale: the scaled space's factor, which its side takes.)
    let place = |ship: Mat4, x: &Holo| {
        let m = ship * parts.matrix(x.node) * x.frame;
        m.w_axis.truncate().extend(m.x_axis.truncate().length()).to_array()
    };
    let frame = ship.filter(|_| on).map_or(show.frame, |s| HoloFrameGpu { places: [place(s.to_matrix(), &h.own), place(s.to_matrix(), &h.other)] });
    if show.shown != shown || show.frame != frame {
        show.shown = shown;
        show.frame = frame;
    }
}

// ---- The display pass (render world) ----

/// The buffers and groups made from one `HoloParts` (a new ship's are new).
struct HoloBuffers {
    /// The parts they were made from (`Arc::as_ptr`).
    key: usize,
    params: [Buffer; 2],
    frame: Buffer,
    written: Option<HoloFrameGpu>,
    /// Each hologram's group, made once its textures exist.
    materials: [Option<BindGroup>; 2],
}

#[derive(Resource)]
struct HoloGpu {
    layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    /// Without and with MSAA (the view group's depth texture), queued with the first holograms and kept.
    pipelines: Option<[CachedRenderPipelineId; 2]>,
    made: Option<HoloBuffers>,
    logged: bool,
}

fn init_display(mut commands: Commands, assets: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "sidewinder_ui_holo",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                uniform_buffer_sized(false, None),
                uniform_buffer_sized(false, None),
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    commands.insert_resource(HoloGpu { layout, shader: assets.load(HOLO_SHADER), pipelines: None, made: None, logged: false });
}

/// The display's pipeline: the GUI materials' blend (One, InvSrcAlpha: premultiplied, E for the display's), both faces (a
/// billboard), no vertex buffer; the depth test in the shader (`ui_in_front`), as Elite's shader compares the scene's depth.
fn display_pipeline(gpu: &HoloGpu, view: &BindGroupLayoutDescriptor, msaa: bool) -> RenderPipelineDescriptor {
    let defs: Vec<ShaderDefVal> = vec![if msaa { "MSAA" } else { "NO_MSAA" }.into()];
    let c = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add };
    RenderPipelineDescriptor {
        label: Some("sidewinder_ui_holo".into()),
        layout: vec![view.clone(), gpu.layout.clone()],
        vertex: VertexState { shader: gpu.shader.clone(), shader_defs: defs.clone(), entry_point: Some("vertex".into()), ..default() },
        fragment: Some(FragmentState {
            shader: gpu.shader.clone(),
            shader_defs: defs,
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState { format: HDR_FORMAT, blend: Some(BlendState { color: c, alpha: c }), write_mask: ColorWrites::ALL })],
            ..default()
        }),
        primitive: PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: None, ..default() },
        ..default()
    }
}

/// The display's buffers made when the holograms' parts arrive, the frame's places written in place, each hologram's group
/// made once its textures exist; a pipeline's failure logged once (the holograms are then not drawn, the ship works).
#[allow(clippy::too_many_arguments)]
fn prepare_display(show: Res<HoloShow>, view: Res<PartsView>, mut gpu: ResMut<HoloGpu>, device: Res<RenderDevice>, queue: Res<RenderQueue>, cache: Res<PipelineCache>, images: Res<RenderAssets<GpuImage>>) {
    let gpu = &mut *gpu;
    let Some(parts) = show.parts.as_ref() else {
        gpu.made = None;
        return;
    };
    if gpu.pipelines.is_none() {
        gpu.pipelines = Some([false, true].map(|msaa| cache.queue_render_pipeline(display_pipeline(gpu, view.layout(msaa), msaa))));
    }
    let key = Arc::as_ptr(parts) as usize;
    if gpu.made.as_ref().is_none_or(|m| m.key != key) {
        let init = |label: &'static str, contents: &[u8], usage: BufferUsages| device.create_buffer_with_data(&BufferInitDescriptor { label: Some(label), contents, usage });
        gpu.made = Some(HoloBuffers {
            key,
            params: parts.params.each_ref().map(|p| init("sidewinder_ui_holo_params", p, BufferUsages::UNIFORM)),
            frame: init("sidewinder_ui_holo_frame", bytemuck::bytes_of(&show.frame), BufferUsages::UNIFORM | BufferUsages::COPY_DST),
            written: Some(show.frame),
            materials: [None, None],
        });
    }
    let Some(made) = gpu.made.as_mut() else { return };
    if made.written != Some(show.frame) {
        queue.write_buffer(&made.frame, 0, bytemuck::bytes_of(&show.frame));
        made.written = Some(show.frame);
    }
    for k in 0..2 {
        if made.materials[k].is_some() {
            continue;
        }
        let (Some(generated), Some(scan)) = (images.get(&parts.generated[k]), images.get(&parts.scanlines)) else { continue };
        let entries = BindGroupEntries::sequential((made.params[k].as_entire_binding(), made.frame.as_entire_binding(), &generated.texture_view, &scan.texture_view, &scan.sampler));
        made.materials[k] = Some(device.create_bind_group("sidewinder_ui_holo", &cache.get_bind_group_layout(&gpu.layout), &entries));
    }
    for id in gpu.pipelines.iter().flatten() {
        if let CachedPipelineState::Err(e) = cache.get_render_pipeline_state(*id)
            && !std::mem::replace(&mut gpu.logged, true)
        {
            error!("sidewinder ui: the holograms' pipeline failed, they are not drawn: {e}");
        }
    }
}

/// The holograms' pass: after the scanner's, each shown hologram's billboard into the view's main texture at its full size.
fn display_pass(
    view: ViewQuery<(Entity, &ViewTarget, &ViewUniformOffset, &Msaa), With<ColorCorrection>>,
    show: Res<HoloShow>,
    frame: Res<UiFrame>,
    parts_view: Res<PartsView>,
    gpu: Res<HoloGpu>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (entity, target, offset, msaa) = view.into_inner();
    let (Some(parts), Some(made), Some(view_group)) = (show.parts.as_ref(), gpu.made.as_ref(), parts_view.group(entity, &frame, target)) else { return };
    let pipeline = gpu.pipelines.and_then(|p| cache.get_render_pipeline(p[(msaa.samples() > 1) as usize]));
    let (Some(pipeline), true, true) = (pipeline, show.shown.contains(&true), made.key == Arc::as_ptr(parts) as usize) else { return };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "sidewinder_ui_holo");
    let scene = RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } };
    let encoder = ctx.command_encoder();
    let mut p = encoder.begin_render_pass(&RenderPassDescriptor { label: Some("sidewinder_ui_holo"), color_attachments: &[Some(scene)], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
    p.set_pipeline(pipeline);
    p.set_bind_group(0, view_group, &[offset.offset]);
    for (shown, material) in show.shown.iter().zip(&made.materials) {
        let (true, Some(material)) = (*shown, material) else { continue };
        p.set_bind_group(1, material, &[]);
        p.draw(0..6, 0..1);
    }
    drop(p);
    span.end(ctx.command_encoder());
}

// ---- The generation pass (render world) ----

#[derive(Resource)]
struct GenPipeline {
    layout: BindGroupLayoutDescriptor,
    id: CachedRenderPipelineId,
}

const GEN_DEPTH: TextureFormat = TextureFormat::Depth32Float;

fn queue_pipeline(mut commands: Commands, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let shader: Handle<Shader> = assets.load(GEN_SHADER);
    let layout = BindGroupLayoutDescriptor::new(
        "sidewinder_holo_gen",
        &BindGroupLayoutEntries::with_indices(ShaderStages::VERTEX_FRAGMENT, ((0, uniform_buffer_sized(false, None)), (1, texture_2d(TextureSampleType::Float { filterable: true })), (2, sampler(SamplerBindingType::Filtering)))),
    );
    let buffers = vec![VertexBufferLayout::from_vertex_formats(VertexStepMode::Vertex, [VertexFormat::Float32x3, VertexFormat::Float32x3, VertexFormat::Float32x2])];
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("sidewinder_holo_gen".into()),
        layout: vec![layout.clone()],
        vertex: VertexState { shader: shader.clone(), entry_point: Some("vertex".into()), buffers, ..default() },
        fragment: Some(FragmentState { shader, entry_point: Some("fragment".into()), targets: vec![Some(ColorTargetState { format: TextureFormat::R32Float, blend: None, write_mask: ColorWrites::ALL })], ..default() }),
        // (Back faces culled: the generation materials carry no pCullMode, so Direct3D's default state applies, CULL_BACK (B).
        // Drawn both sided, a ribbon's front and back faces fought in the depth buffer and their different rim offsets lit the
        // whole ribbon through the Sobel.)
        primitive: PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: Some(Face::Back), ..default() },
        depth_stencil: Some(DepthStencilState { format: GEN_DEPTH, depth_write_enabled: Some(true), depth_compare: Some(CompareFunction::Less), stencil: default(), bias: default() }),
        ..default()
    });
    commands.insert_resource(GenPipeline { layout, id });
}

/// The render world's own: each slot's model buffers (for the copy they were made from), its uniform, the depth.
#[derive(Resource, Default)]
struct GenGpu {
    slots: [Option<SlotGpu>; 2],
    depth: Option<TextureView>,
}

struct SlotGpu {
    mesh: usize,
    vertices: Buffer,
    indices: Buffer,
    count: u32,
    uniform: Buffer,
}

fn prepare(holo_gen: Res<HoloGen>, mut gpu: ResMut<GenGpu>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    if holo_gen.slots.iter().all(Option::is_none) {
        return;
    }
    if gpu.depth.is_none() {
        let t = device.create_texture(&TextureDescriptor {
            label: Some("sidewinder_holo_gen_depth"),
            size: Extent3d { width: BUFFER, height: BUFFER, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: GEN_DEPTH,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        gpu.depth = Some(t.create_view(&TextureViewDescriptor::default()));
    }
    for (k, s) in holo_gen.slots.iter().enumerate() {
        let Some(s) = s else { continue };
        let key = Arc::as_ptr(&s.mesh) as usize;
        if gpu.slots[k].as_ref().is_none_or(|g| g.mesh != key) {
            gpu.slots[k] = Some(SlotGpu {
                mesh: key,
                vertices: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_holo_gen_vertices"), contents: bytemuck::cast_slice(&s.mesh.vertices), usage: BufferUsages::VERTEX }),
                indices: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_holo_gen_indices"), contents: bytemuck::cast_slice(&s.mesh.indices), usage: BufferUsages::INDEX }),
                count: s.mesh.indices.len() as u32,
                uniform: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_holo_gen_uniform"), contents: &uniform_bytes(&s.uniform), usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST }),
            });
        } else if let Some(g) = &gpu.slots[k] {
            queue.write_buffer(&g.uniform, 0, &uniform_bytes(&s.uniform));
        }
    }
}

fn uniform_bytes(u: &GenUniformGpu) -> Vec<u8> {
    let mut b = encase::UniformBuffer::new(Vec::new());
    let _ = b.write(u);
    b.into_inner()
}

/// The two buffers, once per frame (the main view only) before the opaque pass: each drawn slot's model into its buffer.
fn gen_pass(_view: ViewQuery<&ExtractedView, MainView>, holo_gen: Res<HoloGen>, gpu: Res<GenGpu>, pipe: Res<GenPipeline>, images: Res<RenderAssets<GpuImage>>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    let (Some(pipeline), Some(depth)) = (cache.get_render_pipeline(pipe.id), gpu.depth.as_ref()) else { return };
    let device = ctx.render_device().clone();
    let layout = cache.get_bind_group_layout(&pipe.layout);
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "sidewinder_holo_gen");
    for (s, g) in holo_gen.slots.iter().zip(&gpu.slots) {
        let (Some(s), Some(g)) = (s, g) else { continue };
        let (Some(target), Some(map)) = (images.get(&s.target), images.get(&s.heightmap)) else { continue };
        // (A bind group a frame for each drawn slot: two small groups.)
        let group = device.create_bind_group("sidewinder_holo_gen", &layout, &BindGroupEntries::with_indices(((0, g.uniform.as_entire_binding()), (1, &map.texture_view), (2, &map.sampler))));
        let encoder = ctx.command_encoder();
        let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("sidewinder_holo_gen"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &target.texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(LinearRgba::new(EMPTY, 0.0, 0.0, 0.0).into()), store: StoreOp::Store },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment { view: depth, depth_ops: Some(Operations { load: LoadOp::Clear(1.0), store: StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        p.set_pipeline(pipeline);
        p.set_bind_group(0, &group, &[]);
        p.set_vertex_buffer(0, *g.vertices.slice(..));
        p.set_index_buffer(*g.indices.slice(..), IndexFormat::Uint32);
        p.draw_indexed(0..g.count, 0, 0..1);
    }
    span.end(ctx.command_encoder());
}

fn gen_drawn(holo_gen: Res<HoloGen>) -> bool {
    holo_gen.slots.iter().any(Option::is_some)
}

pub(super) fn plugin(app: &mut App) {
    app.add_plugins((ExtractResourcePlugin::<HoloGen>::default(), ExtractResourcePlugin::<HoloShow>::default()))
        .init_resource::<HoloGen>()
        .init_resource::<HoloShow>()
        .add_systems(Update, (start_load.run_if(load_due), build.run_if(resource_exists::<HoloLoading>), teardown.run_if(ui_gone)))
        .add_systems(PostUpdate, update.in_set(super::UiSet::Place).run_if(resource_exists::<Holograms>));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<GenGpu>()
        .add_systems(RenderStartup, init_display)
        // (Queued when a hologram is first drawn, not at start: with the mod never used nothing of it is built.)
        .add_systems(Render, queue_pipeline.in_set(RenderSystems::PrepareResources).run_if(not(resource_exists::<GenPipeline>)).run_if(gen_drawn))
        .add_systems(Render, (prepare, prepare_display).in_set(RenderSystems::PrepareResources).run_if(resource_exists::<HoloGen>))
        .add_systems(Core3d, gen_pass.before(Core3dSystems::Prepass).run_if(resource_exists::<GenPipeline>).run_if(gen_drawn))
        .add_systems(Core3d, display_pass.after(ScannerPassSet).in_set(Core3dSystems::PostProcess));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> HoloFile {
        let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(HOLO_FILE)).unwrap();
        HoloFile::parse(&bytes).unwrap()
    }

    /// The class mapping table: each SpaceEngine class reaches its stated Elite model through the shipped 27-entry table.
    #[test]
    fn the_class_table_maps_as_stated() {
        let f = shipped();
        assert_eq!(f.planet_classes.len(), 27);
        let model = |c: Option<&str>| class_model(&f.planet_classes, elite_class(c, Life::None)).map(str::to_string);
        let living = |c: &str, class: &str, biome: &str| {
            let block = se_core::sc::parse(&format!("Life {{ Class \"{class}\" Type \"Multicellular\" Biome \"{biome}\" }}"));
            class_model(&f.planet_classes, elite_class(Some(c), Life::of(&block, false))).map(str::to_string)
        };
        assert_eq!(living("Terra", "Organic", "Terrestrial").as_deref(), Some("Terrestrial_HabitableOutdoorWorld"), "a terra with land life");
        assert_eq!(living("Jupiter", "Organic", "Aerial").as_deref(), Some("Jovian_WaterGiant_WithLife"), "a gas giant with aerial life");
        assert_eq!(living("Neptune", "Exotic", "Aerial").as_deref(), Some("JovianClassification_AmmoniaLife"), "a giant with exotic life");
        assert_eq!(living("Glacia", "Organic", "Subglacial").as_deref(), Some("Terrestrial_IceMantle"), "an ice world with subglacial life");
        assert_eq!(living("Terra", "Organic", "Marine").as_deref(), Some("Terrestrial_Rocky"), "sea life is not land life");
        assert_eq!(class_model(&f.planet_classes, elite_class(Some("Selena"), Life::Land)), Some("Terrestrial_HabitableOutdoorWorld"));
        assert_eq!(model(Some("Terra")).as_deref(), Some("Terrestrial_Rocky"));
        // Elite's numbers (5 Rocky, 6 Icy, 10 WaterLife, 11 AmmoniaLife, 0 EarthLike, 12 SudarskyClassI).
        assert_eq!(elite_class(Some("Terra"), Life::None), 5);
        assert_eq!(elite_class(Some("Glacia"), Life::None), 6);
        assert_eq!(elite_class(Some("Jupiter"), Life::Organic), 10);
        assert_eq!(elite_class(Some("Jupiter"), Life::Exotic), 11);
        assert_eq!(elite_class(Some("Jupiter"), Life::None), 12);
        assert_eq!(elite_class(None, Life::Land), 0);
        assert_eq!(model(Some("aquaria")).as_deref(), Some("Terrestrial_WaterWorld"));
        assert_eq!(model(Some("Ferria")).as_deref(), Some("Terrestrial_MetalRich"));
        assert_eq!(model(Some("Chthonia")).as_deref(), Some("Terrestrial_MetalRich"));
        assert_eq!(model(Some("Glacia")).as_deref(), Some("Terrestrial_IceMantle"));
        assert_eq!(model(Some("Jupiter")).as_deref(), Some("JovianClassification_SudarskyClassI"));
        assert_eq!(model(Some("Neptune")).as_deref(), Some("JovianClassification_SudarskyClassI"));
        for c in [Some("Selena"), Some("Carbonia"), Some("Desert"), None] {
            assert_eq!(model(c).as_deref(), Some("Terrestrial_Rocky"), "{c:?}");
        }
        // Each model it reaches has its data and its height map shipped.
        for c in ELITE_CLASS.iter().map(|(_, b)| *b).chain([ROCKY, EARTH_LIKE, GIANT_ORGANIC, GIANT_EXOTIC]) {
            let m = class_model(&f.planet_classes, c).unwrap().to_lowercase();
            assert!(f.class_data.contains_key(&m), "{m}");
        }
    }

    /// A row whose sub-bytes are not "any" is passed over for a body whose sub-classes are unknown (Volcanic before Rocky).
    #[test]
    fn the_first_any_row_wins() {
        let rows = vec![ClassRow { key: [5, 21, 4, 15, 3], model: "Terrestrial_Volcanic".into() }, ClassRow { key: [5, 21, 4, 15, 4], model: "Terrestrial_Rocky".into() }];
        assert_eq!(class_model(&rows, 5), Some("Terrestrial_Rocky"));
        assert_eq!(class_model(&rows, 9), None);
    }

    /// Every model is normalised to its bounding sphere (the farthest vertex at 1) round its box's middle.
    #[test]
    fn the_models_are_normalised() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder");
        for file in ["ui/hud3d/models/orrery.edm", OWN_MODEL] {
            let edm = parse_edm(&std::fs::read(dir.join(file)).unwrap()).unwrap();
            let m = gen_mesh(&edm, file).unwrap();
            let far = m.vertices.iter().map(|v| Vec3::from_array(v.position).length()).fold(0.0f32, f32::max);
            assert!((far - 1.0).abs() < 1e-5, "{file}: {far}");
            assert_eq!(m.indices.len() % 3, 0);
        }
    }

    /// The display materials' values reach the shader (the own ship's edge 9.0, the target's 8.5; the notes' 2.2).
    #[test]
    fn the_display_values_are_the_materials() {
        let f = shipped();
        let own = HoloParams::new(&f.display["own"], [[1.0; 3]; 2], 0.0, 0.1).unwrap();
        let target = HoloParams::new(&f.display["target"], [[1.0; 3]; 2], 1.0, 0.1).unwrap();
        assert_eq!((own.sobel.w, target.sobel.w), (9.0, 8.5));
        assert_eq!((own.sobel.x, target.sobel.x), (2.3, 1.6));
        assert_eq!(own.dist, Vec4::new(15.9, 1.0, 0.5, 0.125));
        assert_eq!(own.high.w, 0.6);
    }
}
