//! The Sidewinder mod's ship UI, how Elite's UI movies sit in the cockpit (ARCHITECTURE.md A10): `ui/layout.json`, and the
//! font map `ui/fonts/fontmap.json`, read and written here as serde structs. Both are written by `copy_sidewinder`
//! (`sidewinder/convert/ui_layout.rs`, `ui_movies.rs`) from the decoded UI export's `rendering.json`,
//! `panels_sidewinder_cockpit.json`, `cockpitui_state_machine.json` and `hud3d/` (`re/elite/notes/ui_rendering.md` gives
//! every value's proof); the game reads them (UI3).
//!
//! Numbers are Elite's, unchanged but for the axes. **Frames:** every position, axis and quad vertex is in the mod's axes
//! (x right, y up, -z the nose: Elite's model space with z mirrored, as `sidewinder::assets::ShipSpace` turns it, with no
//! lift), in the frame of what it hangs on, which each field names: the cockpit model's own space (rig `Root`), the chair
//! rig's own space (`CockpitChair_Bone_Rig`), a bone's own frame, or a `gui_*` model's own space. The game places them
//! with the node they hang on (`ShipSpace::matrix`), as it places the cockpit's parts. Pixels are the movie's stage
//! pixels, uv is 0..1 across the stage (Direct3D's: v down). Grades in this file are the notes' (A from Elite's code or
//! data exactly, B with an inference, C measured, E not found).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::elite_ship::Attr;

/// `layout.json`'s format number; `parse_layout` refuses any other.
pub const LAYOUT_FORMAT: u32 = 1;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum UiLayoutError {
    #[error("{file}: {why}")]
    Bad { file: &'static str, why: String },
}

const LAYOUT_FILE: &str = "ui/layout.json";
const FONTMAP_FILE: &str = "ui/fonts/fontmap.json";

fn bad(file: &'static str, why: impl Into<String>) -> UiLayoutError {
    UiLayoutError::Bad { file, why: why.into() }
}

/// `layout.json`: how the movies in the mod's scope sit in the cockpit and how they are shaded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// `LAYOUT_FORMAT`.
    pub format: u32,
    /// The movies in A10's scope, in its order.
    pub movies: Vec<MovieLayout>,
    /// The `gui_*` and quad models the movies are drawn on, by file stem (`gui_left_panel`, `quad`).
    pub models: BTreeMap<String, GuiModel>,
    /// The materials those models and the screen-space movies use, by Elite's name in lower case.
    pub materials: BTreeMap<String, GuiMaterial>,
    /// The bones the panels, plates and head look name, by Elite's bone name (`!Left_Panel`), in each frame Elite
    /// gives them.
    pub bones: BTreeMap<String, BoneFrames>,
    /// Where the chair rig hangs in the cockpit model.
    pub chair_rig: ChairRig,
    /// The `cockpitui` state machine's head-look targets, one per state that sets one.
    pub head_look: Vec<HeadLook>,
    /// The cockpit rig's names for model bones that the head-look targets use (`LeftPanel_Focus` -> `!Left_Panel`).
    pub head_look_joints: Vec<JointBone>,
    /// The gauge movie's sheet: where its parts sit on its stage.
    pub gauge_atlas: GaugeAtlas,
    /// The target-sights movie's three zones of indicator cells (`CockpitGunsightComponent`), in sheet order.
    pub sight_zones: Vec<SightZone>,
    /// The shaders' constants that are data.
    pub shading: Shading,
    /// Values the export marks as not found or inferred, shipped as the handoff's stated stand-ins.
    pub stand_ins: Vec<StandIn>,
}

/// One movie of the scope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovieLayout {
    /// Its file stem under `ui/movies/` (lower case: `cockpitgaugepanel`).
    pub name: String,
    /// Its stage, pixels (width, height): the movie's own.
    pub stage: [u32; 2],
    /// How it is drawn.
    pub kind: MovieKind,
    /// Its `CockpitPanelComponent` (the panel movies only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<Component>,
    /// The GUI panels it is shown on (the component's list, in its order).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<GuiPanel>,
    /// Its dark background plates (the component's list, in its order).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backgrounds: Vec<Background>,
    /// The component's own panel quad (left, right, role and comms panels), which Elite draws the movie on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quad: Option<PanelQuad>,
    /// The gunsights' and orbital HUD's flat quads (no bone), by model stem, in the component's order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flat_quads: Vec<String>,
    /// A screen-space movie's own materials (names in `Layout::materials`), drawn after the tone map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screen_materials: Vec<String>,
}

/// How a movie reaches the screen (`ui_rendering.md` 2.1, 2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovieKind {
    /// On `gui_*` panel models (and a panel quad) hung on bones.
    Panels,
    /// On the gunsight quads (`CockpitGunsightComponent`).
    Sights,
    /// On the orbital HUD's quad (`CockpitOrbitalHudComponent`).
    Orbital,
    /// In screen space after the tone map, by its own materials.
    Screen,
}

/// The chair rig's name, as Elite writes it (and as a head-look target qualifies a bone of it: `CockpitChair_Bone_Rig!RoleSwitch_Panel`).
pub const CHAIR_RIG_NAME: &str = "CockpitChair_Bone_Rig";

/// The rig a bone belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rig {
    /// The cockpit model itself (`sidewinder_cockpit`), Elite's `Root`.
    #[serde(rename = "Root")]
    Cockpit,
    /// The chair rig hung on the cockpit's `!Pilot_Chair` (`CockpitChair_Bone_Rig`): panels on it sway with the chair.
    #[serde(rename = "CockpitChair_Bone_Rig")]
    Chair,
}

/// A movie's `CockpitPanelComponent` record (`ui_rendering.md` 2.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// The component's name (`CockpitModuleComponent_ExternalDataPanel`).
    pub component: String,
    /// Its key (`ExternalDataPanel`): the name the focus state machine's head-look targets use.
    pub key: String,
    /// The movie as Elite names it (`CockpitExternalPanel`).
    pub movie: String,
    /// Its group name (`Left_Panel`).
    pub group: String,
    /// The bone the head looks at when the panel has the focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_bone: Option<String>,
    /// That bone's rig.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_rig: Option<Rig>,
}

/// One GUI panel the movie is shown on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPanel {
    /// Its name (`Gui_Left_Panel`).
    pub name: String,
    /// Its second key, the data it shows (`InternalData`).
    pub data_key: String,
    /// The record's flag word, as stored.
    pub flag: u32,
    /// The bone it hangs on.
    pub bone: String,
    /// The bone used when `bone` is not in the model (B: `!Gauges_Panel` is not in the Sidewinder's, so `!Radar`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone_fallback: Option<String>,
    /// The rig of both bones.
    pub rig: Rig,
    /// The model it is drawn with (a key of `Layout::models`).
    pub model: String,
}

/// One dark background plate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Background {
    /// Its name (`Gui_Left_Panel_Background`).
    pub name: String,
    /// The GUI panel it belongs to.
    pub parent: String,
    /// The record's first flag, as stored.
    pub flag: u32,
    /// Its alpha (0..1), as stored.
    pub alpha: f32,
    /// The record's second flag, as stored.
    pub flag2: u32,
    /// The bone it hangs on.
    pub bone: String,
    pub rig: Rig,
    /// The model it is drawn with (a key of `Layout::models`).
    pub model: String,
}

/// A component's panel quad: four corners in its bone's own frame (top-left, top-right, bottom-left, bottom-right), each
/// x y z (metres, the mod's axes) and u v (0..1 of the stage), and its two triangles' indices in the mod's axes: the
/// data's `0 3 2 1 0 3` with two indices of each triangle swapped as the ship's models are on load
/// (`elite::to_mod_space`), so `0 2 3 1 3 0`. Not drawn: the game draws the panel's `gui_*` model, the same rectangle.
/// The quad is most likely the panel's pointer surface (grade B: Elite's own index list winds its two triangles opposite
/// ways, so it cannot be a culled draw; how Elite uses it is not decoded). As in Elite's data, the first triangle (0,2,3)
/// is counter-clockwise seen from the panel's front (the pilot's side) in the mod's axes, the second (1,3,0) clockwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelQuad {
    pub vertices: [[f32; 5]; 4],
    pub indices: [u8; 6],
    /// The bone whose frame the corners are in (a key of `Layout::bones`): the component's GUI panel's bone (a stand-in
    /// reading, `Layout::stand_ins`: the record holds the quad without a bone of its own).
    pub bone: String,
    /// That bone's rig.
    pub rig: Rig,
}

/// A `gui_*` (or quad) model: the file and what the movie's texture does on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiModel {
    /// The model in the mod's folder (`ui/models/gui_left_panel.edm`).
    pub file: String,
    /// Its bounds in its own space, metres, the mod's axes.
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Its size, metres: the bounds' width (x) and height (y), as the notes' table gives it.
    pub size_m: [f32; 2],
    /// Its draws, in the model's order.
    pub objects: Vec<GuiObject>,
    /// For the gauge models: each connected island of triangles and the gauge parts its stage rectangle holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub islands: Vec<Island>,
}

/// One draw of a `gui_*` model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiObject {
    /// Its material (a key of `Layout::materials`).
    pub material: String,
    pub vertices: u32,
    pub triangles: u32,
    /// Its uv0 rectangle: which part of the movie's stage it shows (0..1, v down).
    pub uv0_min: [f32; 2],
    pub uv0_max: [f32; 2],
    /// Its vertices' bounds in the model's space, metres, the mod's axes.
    pub pos_min: [f32; 3],
    pub pos_max: [f32; 3],
}

/// A connected island of a gauge model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Island {
    /// Its stage rectangle, pixels (x0, y0, x1, y1): its uv times the stage.
    pub stage_rect: [f32; 4],
    pub vertices: u32,
    /// Its vertices' bounds in the model's space, metres, the mod's axes.
    pub pos_min: [f32; 3],
    pub pos_max: [f32; 3],
    /// The gauge parts (`GaugePart::instance`) inside its stage rectangle.
    pub parts: Vec<String>,
}

/// A material: Elite's shader, its kind, its texture slots and every attribute with its values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiMaterial {
    /// Elite's effect (`Alpha_GuiLighting_Gui_GuiBrightnessScale_Saturation_GUIColourRotate`).
    pub shader: String,
    /// Which of the notes' families the shader is.
    pub tag: MaterialTag,
    pub textures: Vec<GuiSlot>,
    /// Every attribute as Elite's material file holds it (`pGuiTint`, `pCloudynessScale`, `pParallaxScale`,
    /// `gStandardMapScales`, the blend and stencil states...).
    pub attrs: Vec<Attr>,
}

/// A material's family by its shader (`ui_rendering.md` 2.5, 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialTag {
    /// A foreground panel: `GuiLighting`.
    #[serde(rename = "fg")]
    Foreground,
    /// A dark plate: `GuiBackground` (or `TonemappedGuiBackground`).
    #[serde(rename = "bg")]
    Background,
    /// The flat quads' UV-scale-offset brightness shader (gunsights, orbital HUD).
    #[serde(rename = "brt")]
    Brightness,
    /// The antialiased sight shader.
    #[serde(rename = "sight")]
    Sight,
    /// A screen-space movie's `GuiParallax` (after the tone map).
    #[serde(rename = "screen")]
    Screen,
    /// The screen-space outline plane, `EdgeDetectionForUI`.
    #[serde(rename = "edge")]
    Edge,
}

/// One of a material's texture slots (no `deny_unknown_fields`: serde cannot have it with `flatten`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuiSlot {
    /// Elite's slot name (`pCloudynessTexture`).
    pub slot: String,
    #[serde(flatten)]
    pub value: GuiSlotValue,
}

/// What a slot holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum GuiSlotValue {
    /// A shipped texture.
    #[serde(rename = "tex")]
    Tex {
        /// Elite's texture name (`material.slot`, lower case).
        texture: String,
        /// The file in the mod's folder (`ui/tex/NAME.dds`).
        file: String,
        /// Sample it as sRGB.
        srgb: bool,
        /// Its block format as the export names it (`BC1`, `BC3`...).
        format: String,
    },
    /// A texture no export holds: not shipped (`SOURCES.txt` lists it).
    #[serde(rename = "missing")]
    Missing { texture: String },
    /// A constant colour: two RGBA8, as Elite stores them.
    #[serde(rename = "dt7")]
    Colour { colour: [[u8; 4]; 2] },
}

/// A bone in each frame Elite gives it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoneFrames {
    /// In the cockpit model's own space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cockpit: Option<BoneFrame>,
    /// In the chair rig's own space (cockpit space at rest: add `ChairRig::offset`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chair_rig: Option<BoneFrame>,
}

impl BoneFrames {
    /// The bone's frame in that rig's space, if Elite gives it one.
    pub fn frame(&self, rig: Rig) -> Option<&BoneFrame> {
        match rig {
            Rig::Cockpit => self.cockpit.as_ref(),
            Rig::Chair => self.chair_rig.as_ref(),
        }
    }
}

impl Layout {
    /// The bone and rig a head-look target names: a rig-qualified bone (`CockpitChair_Bone_Rig!RoleSwitch_Panel` is
    /// `!RoleSwitch_Panel` of the chair rig), or a cockpit rig joint of `head_look_joints` (a cockpit model bone).
    pub fn joint_bone<'a>(&'a self, joint: &'a str) -> Option<(&'a str, Rig)> {
        match joint.strip_prefix(CHAIR_RIG_NAME) {
            Some(bone) => Some((bone, Rig::Chair)),
            None => self.head_look_joints.iter().find(|j| j.joint == joint).map(|j| (j.bone.as_str(), Rig::Cockpit)),
        }
    }

    /// The frame of `bone` in `rig`'s space, if the layout holds it.
    pub fn bone_frame(&self, bone: &str, rig: Rig) -> Option<&BoneFrame> {
        self.bones.get(bone)?.frame(rig)
    }
}

/// A bone's rest frame in its rig's space: its origin and its own x, y, z axes, the mod's axes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoneFrame {
    /// Metres.
    pub position: [f32; 3],
    /// The bone's x, y and z axes (unit vectors; z is Elite's z mirrored, so this frame is right-handed where Elite's is
    /// left-handed).
    pub axes: [[f32; 3]; 3],
}

/// Where the chair rig hangs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairRig {
    /// The cockpit's bone it hangs on (`!Pilot_Chair`).
    pub bone: String,
    /// The chair rig's origin in the cockpit model's space, metres, the mod's axes (its turn is the identity, to 1.5e-7).
    pub offset: [f32; 3],
}

/// A `HeadlookTargetBoneActivity` of the `cockpitui` state machine (`ui_rendering.md` 4.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadLook {
    /// The state that sets it (`External Panel`).
    pub state: String,
    /// The panel component's key it names (`ExternalDataPanel`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_component: Option<String>,
    /// The joint the head aims at (a `JointBone::joint`, or a rig-qualified bone `CockpitChair_Bone_Rig!RoleSwitch_Panel`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joint: Option<String>,
    /// The head's displacement, metres, and the field-of-view channel, degrees: the first pair and the second, as stored.
    pub displacement_m: f32,
    pub fov_channel_deg: f32,
    pub displacement_m_alt: f32,
    pub fov_channel_deg_alt: f32,
}

/// A cockpit rig joint name and the model bone it stands for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointBone {
    pub joint: String,
    pub bone: String,
}

/// The gauge movie's sheet (`gaugepanel_atlas_layout`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaugeAtlas {
    /// Its stage, pixels.
    pub stage: [u32; 2],
    /// Its `Content` clip's parts, in depth order.
    pub parts: Vec<GaugePart>,
}

/// One part of the gauge sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaugePart {
    /// Its instance name (`heat`); a few parts have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Its ActionScript class (`components.Heat`); a plain clip has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Its character id in the movie.
    pub char_id: u32,
    /// Where the clip places it, pixels, and its scale.
    pub x: f32,
    pub y: f32,
    pub scale: [f32; 2],
    pub depth: u32,
    /// Its drawn bounds on the stage, pixels (x0, y0, x1, y1).
    pub bounds: [f32; 4],
}

/// One zone of the target sights' sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SightZone {
    /// A cell's size, uv.
    pub cell_uv: [f32; 2],
    /// The two counts as stored (the columns, then the cells: three rows each).
    pub counts: [u32; 2],
    /// Where the zone starts, uv.
    pub offset_uv: [f32; 2],
}

/// The shaders' data (`rendering.json` `shader_formulas`, `gui_colour`, `global_constants_cb1`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shading {
    /// The GUI colour matrix's rows (Red, Green, Blue, then the fourth), Elite's default (A): out = c.x*row0 + c.y*row1 +
    /// c.z*row2.
    pub gui_colour_matrix: [[f32; 3]; 4],
    /// The screen-space GUI render target, pixels (A).
    pub screen_target: [u32; 2],
    /// The constants the formulas hold, each with the formula's own words it is read from.
    pub constants: Vec<ShaderConstant>,
}

/// One constant of a shader formula.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderConstant {
    /// The formula (`rendering.json` `shader_formulas`'s key).
    pub formula: String,
    /// What it is.
    pub name: String,
    pub values: Vec<f64>,
    /// The formula's text it is read from, as written there.
    pub text: String,
}

/// A value shipped as the handoff's stand-in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandIn {
    pub what: String,
    pub value: String,
    pub grade: String,
    pub why: String,
}

/// `fontmap.json`: the TrueType files under `ui/fonts/` and the font map's names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontMap {
    /// The font library the map names (`fonts_roman`).
    pub library: String,
    /// Each shipped file with the font name and boldness its library gives it.
    pub fonts: Vec<FontFile>,
    /// The font map: each name (`$TitleFont`) with its face and the files to register it with.
    pub map: Vec<MappedFont>,
}

/// One shipped TrueType file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontFile {
    /// Its file name in `ui/fonts/`.
    pub file: String,
    /// Its name in the library (`Euro Caps`).
    pub name: String,
    pub bold: bool,
}

/// One name of the font map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappedFont {
    /// The name the movies use (`$TitleFont`).
    pub name: String,
    /// The library's face it stands for, and whether the map asks for it bold.
    pub font: String,
    pub bold: bool,
    /// The file to register under the name for plain text: the face as the map asks for it (or the family's plain face).
    pub regular: String,
    /// The file for bold text under the name: the family's bold face (or `regular`).
    pub heavy: String,
}

/// Reads `layout.json`, refusing anything a reader could trip on: another format, a panel, plate or flat quad naming a
/// model that isn't there, a model naming a material that isn't there, a panel, plate or quad whose bone (or panel
/// fallback) isn't in `bones` in its rig's frame, a quad index outside its corners, a head-look target whose joint or bone
/// isn't there, a model file outside `ui/models/`, a texture file outside `ui/tex/`.
pub fn parse_layout(bytes: &[u8]) -> Result<Layout, UiLayoutError> {
    let l: Layout = serde_json::from_slice(bytes).map_err(|e| bad(LAYOUT_FILE, e.to_string()))?;
    if l.format != LAYOUT_FORMAT {
        return Err(bad(LAYOUT_FILE, format!("format {}, this build reads {LAYOUT_FORMAT}", l.format)));
    }
    for m in &l.movies {
        let models = m.panels.iter().map(|p| &p.model).chain(m.backgrounds.iter().map(|b| &b.model)).chain(&m.flat_quads);
        if let Some(n) = models.into_iter().find(|n| !l.models.contains_key(*n)) {
            return Err(bad(LAYOUT_FILE, format!("movie {}: model {n} is not in the table", m.name)));
        }
        if let Some(n) = m.screen_materials.iter().find(|n| !l.materials.contains_key(*n)) {
            return Err(bad(LAYOUT_FILE, format!("movie {}: material {n} is not in the table", m.name)));
        }
        // Every bone a panel, plate or quad hangs on is in `bones` in its rig's frame (a panel's, or its fallback's).
        let held = |bone: &str, rig: Rig| l.bone_frame(bone, rig).is_some();
        for p in &m.panels {
            if !held(&p.bone, p.rig) && !p.bone_fallback.as_deref().is_some_and(|f| held(f, p.rig)) {
                return Err(bad(LAYOUT_FILE, format!("movie {}, panel {}: neither its bone {} nor its fallback is in bones in its rig's frame", m.name, p.name, p.bone)));
            }
        }
        if let Some(b) = m.backgrounds.iter().find(|b| !held(&b.bone, b.rig)) {
            return Err(bad(LAYOUT_FILE, format!("movie {}, plate {}: its bone {} is not in bones in its rig's frame", m.name, b.name, b.bone)));
        }
        if let Some(q) = &m.quad {
            if !held(&q.bone, q.rig) {
                return Err(bad(LAYOUT_FILE, format!("movie {}, quad: its bone {} is not in bones in its rig's frame", m.name, q.bone)));
            }
            if q.indices.iter().any(|&i| i >= 4) {
                return Err(bad(LAYOUT_FILE, format!("movie {}, quad: an index is outside its 4 corners", m.name)));
            }
        }
    }
    for j in &l.head_look_joints {
        if !l.bones.contains_key(&j.bone) {
            return Err(bad(LAYOUT_FILE, format!("head-look joint {}: its bone {} is not in bones", j.joint, j.bone)));
        }
    }
    for h in &l.head_look {
        if let Some(joint) = &h.joint {
            match l.joint_bone(joint) {
                Some((bone, rig)) if l.bone_frame(bone, rig).is_some() => {}
                Some((bone, _)) => return Err(bad(LAYOUT_FILE, format!("head look of {}: joint {joint}'s bone {bone} is not in bones in its rig's frame", h.state))),
                None => return Err(bad(LAYOUT_FILE, format!("head look of {}: joint {joint} is not among the head-look joints", h.state))),
            }
        }
    }
    for (name, model) in &l.models {
        if !model.file.starts_with("ui/models/") || !model.file.ends_with(".edm") || model.file.contains("..") {
            return Err(bad(LAYOUT_FILE, format!("model {name}: {} is not a ui/models/*.edm file", model.file)));
        }
        if let Some(o) = model.objects.iter().find(|o| !l.materials.contains_key(&o.material)) {
            return Err(bad(LAYOUT_FILE, format!("model {name}: material {} is not in the table", o.material)));
        }
    }
    for (name, mat) in &l.materials {
        for t in &mat.textures {
            if let GuiSlotValue::Tex { file, .. } = &t.value {
                if !file.starts_with("ui/tex/") || !file.ends_with(".dds") || file.contains("..") {
                    return Err(bad(LAYOUT_FILE, format!("material {name}, slot {}: {file} is not a ui/tex/*.dds file", t.slot)));
                }
            }
        }
    }
    Ok(l)
}

/// Writes `layout.json` (pretty-printed, LF line endings).
pub fn write_layout(l: &Layout) -> Vec<u8> {
    let mut b = serde_json::to_vec_pretty(l).expect("the layout is plain data");
    b.push(b'\n');
    b
}

/// Reads `fontmap.json`, refusing a map name whose files aren't among `fonts`, or a file outside `ui/fonts/`.
pub fn parse_fontmap(bytes: &[u8]) -> Result<FontMap, UiLayoutError> {
    let m: FontMap = serde_json::from_slice(bytes).map_err(|e| bad(FONTMAP_FILE, e.to_string()))?;
    if let Some(f) = m.fonts.iter().find(|f| !f.file.ends_with(".ttf") || f.file.contains(['/', '\\'])) {
        return Err(bad(FONTMAP_FILE, format!("{} is not a .ttf file name", f.file)));
    }
    for n in &m.map {
        if let Some(f) = [&n.regular, &n.heavy].into_iter().find(|f| !m.fonts.iter().any(|x| x.file == **f)) {
            return Err(bad(FONTMAP_FILE, format!("{}: {f} is not one of the fonts", n.name)));
        }
    }
    Ok(m)
}

/// Writes `fontmap.json` (pretty-printed, LF line endings).
pub fn write_fontmap(m: &FontMap) -> Vec<u8> {
    let mut b = serde_json::to_vec_pretty(m).expect("the font map is plain data");
    b.push(b'\n');
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        let model = GuiModel {
            file: "ui/models/gui_left_panel.edm".into(),
            bounds_min: [-0.54, -0.229, -0.022],
            bounds_max: [0.33, 0.259, 0.139],
            size_m: [0.87, 0.488],
            objects: vec![GuiObject {
                material: "left_panel".into(),
                vertices: 4,
                triangles: 2,
                uv0_min: [0.0, 0.0],
                uv0_max: [0.999, 0.999],
                pos_min: [-0.54, -0.229, -0.022],
                pos_max: [0.33, 0.259, 0.139],
            }],
            islands: vec![Island { stage_rect: [1.0, 2.0, 3.0, 4.0], vertices: 4, pos_min: [0.0; 3], pos_max: [1.0; 3], parts: vec!["heat".into()] }],
        };
        let material = GuiMaterial {
            shader: "Alpha_GuiLighting_Gui_GuiBrightnessScale_Saturation_GUIColourRotate".into(),
            tag: MaterialTag::Foreground,
            textures: vec![
                GuiSlot {
                    slot: "pCloudynessTexture".into(),
                    value: GuiSlotValue::Tex { texture: "analysis_panel.pcloudynesstexture".into(), file: "ui/tex/a.dds".into(), srgb: false, format: "BC1".into() },
                },
                GuiSlot { slot: "pDiffuse_AlphaTexture".into(), value: GuiSlotValue::Colour { colour: [[0x36, 0x36, 0x36, 0xff]; 2] } },
                GuiSlot { slot: "pScanlinesTexture".into(), value: GuiSlotValue::Missing { texture: "x.pscanlinestexture".into() } },
            ],
            attrs: vec![Attr { name: "pGuiTint".into(), kind: 2, values: vec![1.0, 0.068, 0.025] }],
        };
        Layout {
            format: LAYOUT_FORMAT,
            movies: vec![MovieLayout {
                name: "cockpitexternalpanel".into(),
                stage: [1152, 660],
                kind: MovieKind::Panels,
                component: Some(Component {
                    component: "CockpitModuleComponent_ExternalDataPanel".into(),
                    key: "ExternalDataPanel".into(),
                    movie: "CockpitExternalPanel".into(),
                    group: "Left_Panel".into(),
                    focus_bone: Some("!Left_Panel".into()),
                    focus_rig: Some(Rig::Chair),
                }),
                panels: vec![GuiPanel {
                    name: "Gui_Left_Panel".into(),
                    data_key: "InternalData".into(),
                    flag: 1,
                    bone: "!Left_Panel".into(),
                    bone_fallback: None,
                    rig: Rig::Chair,
                    model: "gui_left_panel".into(),
                }],
                backgrounds: vec![Background {
                    name: "Gui_Left_Panel_Background".into(),
                    parent: "Gui_Left_Panel".into(),
                    flag: 1,
                    alpha: 0.5,
                    flag2: 1,
                    bone: "!Left_Panel".into(),
                    rig: Rig::Cockpit,
                    model: "gui_left_panel".into(),
                }],
                quad: Some(PanelQuad { vertices: [[-0.5, 0.26, 0.05, 0.0, 0.0]; 4], indices: [0, 2, 3, 1, 3, 0], bone: "!Left_Panel".into(), rig: Rig::Chair }),
                flat_quads: vec![],
                screen_materials: vec!["left_panel".into()],
            }],
            models: BTreeMap::from([("gui_left_panel".into(), model)]),
            materials: BTreeMap::from([("left_panel".into(), material)]),
            bones: BTreeMap::from([(
                "!Left_Panel".into(),
                BoneFrames {
                    cockpit: Some(BoneFrame { position: [1.0, 2.0, 3.0], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] }),
                    chair_rig: Some(BoneFrame { position: [1.0, 3.0, 3.0], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] }),
                },
            )]),
            chair_rig: ChairRig { bone: "!Pilot_Chair".into(), offset: [0.0, -1.6098, -0.12855] },
            head_look: vec![HeadLook {
                state: "External Panel".into(),
                panel_component: Some("ExternalDataPanel".into()),
                joint: Some("LeftPanel_Focus".into()),
                displacement_m: 0.0,
                fov_channel_deg: -17.0,
                displacement_m_alt: 0.0,
                fov_channel_deg_alt: -13.0,
            }],
            head_look_joints: vec![JointBone { joint: "LeftPanel_Focus".into(), bone: "!Left_Panel".into() }],
            gauge_atlas: GaugeAtlas {
                stage: [1100, 900],
                parts: vec![GaugePart { instance: Some("heat".into()), class: Some("components.Heat".into()), char_id: 1538, x: 24.0, y: 4.3, scale: [1.0, 1.0], depth: 1, bounds: [4.6, -1.4, 253.4, 253.3] }],
            },
            sight_zones: vec![SightZone { cell_uv: [0.166667, 0.190476], counts: [6, 18], offset_uv: [0.0, 0.0] }],
            shading: Shading {
                gui_colour_matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]],
                screen_target: [1920, 1080],
                constants: vec![ShaderConstant { formula: "f".into(), name: "n".into(), values: vec![1.6], text: "pow(col.rgb, 1.6)".into() }],
            },
            stand_ins: vec![StandIn { what: "render target".into(), value: "the stage size".into(), grade: "B".into(), why: "no call found".into() }],
        }
    }

    #[test]
    fn layout_round_trips() {
        let l = layout();
        let bytes = write_layout(&l);
        assert_eq!(parse_layout(&bytes).unwrap(), l);
        assert!(bytes.ends_with(b"\n") && !bytes.contains(&b'\r'));
        let text = String::from_utf8(bytes).unwrap();
        for word in ["\"CockpitChair_Bone_Rig\"", "\"Root\"", "\"fg\"", "\"kind\": \"tex\"", "\"kind\": \"missing\"", "\"kind\": \"dt7\"", "\"panels\""] {
            assert!(text.contains(word), "{word}");
        }
    }

    #[test]
    fn layout_refuses_what_a_reader_trips_on() {
        let mut l = layout();
        l.format = 2;
        assert!(parse_layout(&write_layout(&l)).is_err());
        let mut l = layout();
        l.movies[0].panels[0].model = "gui_nothing".into();
        assert!(parse_layout(&write_layout(&l)).is_err());
        let mut l = layout();
        l.models.get_mut("gui_left_panel").unwrap().objects[0].material = "nothing".into();
        assert!(parse_layout(&write_layout(&l)).is_err());
        let mut l = layout();
        l.models.get_mut("gui_left_panel").unwrap().file = "models/gui_left_panel.edm".into();
        assert!(parse_layout(&write_layout(&l)).is_err());
        let mut l = layout();
        if let GuiSlotValue::Tex { file, .. } = &mut l.materials.get_mut("left_panel").unwrap().textures[0].value {
            *file = "tex/a.dds".into();
        }
        assert!(parse_layout(&write_layout(&l)).is_err());
        assert!(parse_layout(b"{\"format\": 1, \"unknown\": 0}").is_err());
        // What a reader of the bones trips on: a quad index past its corners, a bone missing from its rig's frame, a
        // panel whose bone and fallback are both missing, a head-look joint that names nothing.
        let refused = |l: &Layout, word: &str| {
            let e = parse_layout(&write_layout(l)).unwrap_err().to_string();
            assert!(e.contains(LAYOUT_FILE) && e.contains(word), "{e}");
        };
        let mut l = layout();
        l.movies[0].quad.as_mut().unwrap().indices[2] = 4;
        refused(&l, "outside its 4 corners");
        let mut l = layout();
        l.bones.get_mut("!Left_Panel").unwrap().chair_rig = None;
        refused(&l, "Gui_Left_Panel");
        let mut l = layout();
        l.movies[0].quad.as_mut().unwrap().bone = "!Comms_Panel".into();
        refused(&l, "quad: its bone !Comms_Panel");
        let mut l = layout();
        l.movies[0].panels[0].bone = "!Gauges_Panel".into();
        l.movies[0].panels[0].bone_fallback = Some("!Left_Panel".into());
        assert!(parse_layout(&write_layout(&l)).is_ok(), "a missing bone with a fallback in bones is fine");
        l.movies[0].panels[0].bone_fallback = Some("!Radar".into());
        refused(&l, "nor its fallback");
        let mut l = layout();
        l.head_look[0].joint = Some("Nowhere_Focus".into());
        refused(&l, "Nowhere_Focus");
        let mut l = layout();
        l.head_look[0].joint = Some("CockpitChair_Bone_Rig!Nothing".into());
        refused(&l, "!Nothing");
    }

    #[test]
    fn a_head_look_joint_names_its_bone_and_rig() {
        let l = layout();
        assert_eq!(l.joint_bone("LeftPanel_Focus"), Some(("!Left_Panel", Rig::Cockpit)));
        assert_eq!(l.joint_bone("CockpitChair_Bone_Rig!RoleSwitch_Panel"), Some(("!RoleSwitch_Panel", Rig::Chair)));
        assert_eq!(l.joint_bone("Nowhere"), None);
        assert!(l.bone_frame("!Left_Panel", Rig::Chair).is_some());
    }

    #[test]
    fn fontmap_round_trips_and_refuses_a_missing_file() {
        let m = FontMap {
            library: "fonts_roman".into(),
            fonts: vec![FontFile { file: "Euro_Caps.ttf".into(), name: "Euro Caps".into(), bold: false }],
            map: vec![MappedFont { name: "$TitleFont".into(), font: "Euro Caps".into(), bold: false, regular: "Euro_Caps.ttf".into(), heavy: "Euro_Caps.ttf".into() }],
        };
        assert_eq!(parse_fontmap(&write_fontmap(&m)).unwrap(), m);
        let mut bad = m.clone();
        bad.map[0].heavy = "Sintony.ttf".into();
        assert!(parse_fontmap(&write_fontmap(&bad)).is_err());
        let mut bad = m;
        bad.fonts[0].file = "../x.ttf".into();
        assert!(parse_fontmap(&write_fontmap(&bad)).is_err());
    }
}
