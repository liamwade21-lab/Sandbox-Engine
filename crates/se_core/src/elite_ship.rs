//! The Faulcon DeLacy Sidewinder mod's JSON data files (ARCHITECTURE.md A5), read and written here as serde structs. Field
//! names keep Elite's own names and values its own units (`re/elite/frea_export/sidewinder/decoded/README.md`).
//!
//! `materials.json`: one table for every material the shipped models name (written by `copy_sidewinder` from the decoded
//! export's `materials.json`): Elite's shader name, each texture slot (a texture in `tex/` or a constant colour), every
//! attribute with its values, and for the names no file of Elite's install defines, the material the engine draws them
//! with instead (`fallback_to`).
//!
//! `ship.json`: the ship's assembly (written by `copy_sidewinder` from the decoded export's `animation/`, `paintjobs.json`
//! and the notes on the ship item): the kinematic rigs (each node's model, the bone it hangs from, its motions in list
//! order), every state machine of those files with its states, transitions and typed activities (poses, Bezier curves
//! with their sound cues, gear state, door requests, waits, lights, audio states; the other types keep every named field
//! as the export decoded it), the default paint job's record, and what the ship item says (seats, gear attach points,
//! cameras, hardpoints). Elite's model space (+X starboard, +Y up, +Z nose), seconds, the units Elite stores. The rules
//! that play it are `crate::elite_rig`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ShipDataError {
    #[error("{file}: {why}")]
    Bad { file: &'static str, why: String },
}

const MATERIALS_FILE: &str = "materials.json";
const SHIP_FILE: &str = "ship.json";

fn bad(file: &'static str, why: impl Into<String>) -> ShipDataError {
    ShipDataError::Bad { file, why: why.into() }
}

/// `materials.json`. Keyed by Elite's material name in lower case, as its material files are named (the models write
/// the same names in mixed case: look them up with `get`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Materials {
    pub materials: BTreeMap<String, Material>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// Elite's effect, e.g. `Metalness_Roughness_OpaqueDeferred_SpacecraftTilePainted_Normals`. None for a fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<TextureSlot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attrs: Vec<Attr>,
    /// A name no file of Elite's install defines: the material the engine draws the part with instead (its default
    /// material `null`, itself in the table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_to: Option<String>,
    /// Why it falls back (the export's evidence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// One of a material's texture slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureSlot {
    /// Elite's slot name (`pBaseColour_Roughness`, `pNormalTexture`, ...).
    pub slot: String,
    #[serde(flatten)]
    pub value: SlotValue,
}

/// What a slot holds, by Elite's kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SlotValue {
    /// A texture.
    #[serde(rename = "tex")]
    Tex {
        /// Elite's texture name (`material.slot`, lower case).
        texture: String,
        /// The file in the mod's folder (`tex/NAME.dds`).
        file: String,
        /// Sample it as sRGB (the DDS itself says UNORM).
        srgb: bool,
        /// Its block format as the export names it (`BC1`, `BC3`, `BC4`, `BC5`, `BC7`).
        format: String,
    },
    /// A constant colour: two RGBA8, as Elite stores them.
    #[serde(rename = "dt7")]
    Colour { colour: [[u8; 4]; 2] },
}

/// One attribute (a shader constant) and its values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attr {
    pub name: String,
    /// Elite's type: 0-3 float, vec2, vec3, vec4; 4 int; 5 uint; 6 bool.
    #[serde(rename = "type")]
    pub kind: u8,
    /// The values as numbers (floats as Elite's float32, uints and bools exact).
    pub values: Vec<f64>,
}

impl Materials {
    /// A material by the name a model gives it (any case).
    pub fn get(&self, name: &str) -> Option<&Material> {
        self.materials.get(&name.to_lowercase())
    }
}

/// Reads `materials.json`, refusing anything a reader could trip on: a key not in lower case, a material with neither a
/// shader nor a fallback, a fallback naming a material that isn't there, a texture file outside `tex/`, an unknown type.
pub fn parse_materials(bytes: &[u8]) -> Result<Materials, ShipDataError> {
    let m: Materials = serde_json::from_slice(bytes).map_err(|e| bad(MATERIALS_FILE, e.to_string()))?;
    for (name, mat) in &m.materials {
        if *name != name.to_lowercase() {
            return Err(bad(MATERIALS_FILE, format!("material {name}: the key is not in lower case")));
        }
        match (&mat.shader, &mat.fallback_to) {
            (None, None) => return Err(bad(MATERIALS_FILE, format!("material {name}: neither a shader nor a fallback"))),
            (_, Some(f)) if !m.materials.contains_key(f) => {
                return Err(bad(MATERIALS_FILE, format!("material {name}: falls back to {f}, which isn't in the table")));
            }
            _ => {}
        }
        for t in &mat.textures {
            if let SlotValue::Tex { file, .. } = &t.value {
                if !file.starts_with("tex/") || !file.ends_with(".dds") || file.contains("..") {
                    return Err(bad(MATERIALS_FILE, format!("material {name}, slot {}: {file} is not a tex/*.dds file", t.slot)));
                }
            }
        }
        if let Some(a) = mat.attrs.iter().find(|a| a.kind > 6) {
            return Err(bad(MATERIALS_FILE, format!("material {name}, attribute {}: unknown type {}", a.name, a.kind)));
        }
    }
    Ok(m)
}

/// Writes `materials.json` (pretty-printed, LF line endings).
pub fn write_materials(m: &Materials) -> Vec<u8> {
    let mut b = serde_json::to_vec_pretty(m).expect("the materials table is plain data");
    b.push(b'\n');
    b
}

// ------------------------------------------------------------------------------------------------ ship.json

/// `ship.json`'s format number; `parse_ship` refuses any other.
pub const SHIP_FORMAT: u32 = 1;

/// `ship.json`: the ship's assembly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ship {
    pub format: u32,
    /// The stock ship's rigs: the hull's, the three gear legs', the cockpit's and the tutorial overlays'.
    pub rigs: Vec<Rig>,
    /// Every state machine of the files those rigs come from.
    pub machines: Vec<Machine>,
    /// The default paint job (`paintjob_sidewinder_default`'s record `DefaultPaintJob`).
    pub paint: Paint,
    /// What the ship item `SideWinder` and its prototype say.
    pub item: Item,
}

/// A kinematic rig (`Project:KRig`): a tree of nodes, each a model hung from its parent's bone and moved by values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rig {
    /// Elite's rig name (`Sidewinder`, `Ext_LandingGear_SideWinder_Left`, ...).
    pub name: String,
    /// The export's animation file it came from (`hull_sidewinder`, `gear_left`, ...).
    pub file: String,
    /// Elite's file holding it (40 hex digits).
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub root: RigNode,
}

/// A rig node. Its matrix starts as `attach_bone`'s bind in its parent node's model times the parent's world matrix
/// (a node without one starts at the parent's, the root at what the caller gives); its motions then act in list order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RigNode {
    pub name: String,
    /// The model it draws (`models/NAME.edm`, lower case), or none.
    #[serde(default)]
    pub model: Option<String>,
    /// The model as Elite's rig names it (a `0x...` djb2 hash where the export couldn't read the name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_ref: Option<String>,
    /// The bone of the parent's model it hangs from (matched case-insensitively, as the engine does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_bone: Option<String>,
    /// The attach record's second bone name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_bone_b: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The node's four flag bytes, hex.
    pub flags: String,
    /// The u32 at node +104.
    pub u104: u32,
    pub motions: Vec<Motion>,
    pub children: Vec<RigNode>,
}

/// A rig motion: the named value moves its node along or about one of its own axes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Motion {
    #[serde(rename = "type")]
    pub kind: MotionKind,
    /// The driven value's name (`Anim_...`).
    pub value: String,
    /// 0 forward (the node's local +Z row), 1 up (+Y), 2 right (+X).
    pub axis: u32,
    pub a: f64,
    pub b: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MotionKind {
    /// Angle = value x sign radians (sign -1 when b < a).
    RotateDirect,
    /// Degrees = value x b for value >= 0, -value x a below.
    RotateDriven,
    /// Value x sign metres along the axis.
    TranslateDirect,
    /// |value| x (b, or a below 0) metres along the axis.
    TranslateDriven,
}

impl Motion {
    /// The Direct motions' sign: -1 when b < a, else 1.
    pub fn sign(&self) -> f64 {
        if self.b < self.a {
            -1.0
        } else {
            1.0
        }
    }
}

/// A code state machine (`Project:CodeStateMachine`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    /// Elite's name (`LandingGearCSM`).
    pub name: String,
    /// The name as stored: `<djb2 of its source path>::<name>`.
    pub stored_name: String,
    /// The rig whose entity owns it (names its `<owner>_<value>_DominantCurveValue` variables).
    pub owner: String,
    /// The export's animation file it came from.
    pub file: String,
    pub initial: String,
    pub states: Vec<State>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub name: String,
    /// Started in order on entering the state, stopped on leaving it.
    pub activities: Vec<Activity>,
    pub transitions: Vec<Transition>,
}

/// An edge of the machine: a requested state is reached by the cheapest route over `cost`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub target: String,
    pub cost: u32,
    /// Leave once the state's running activities have finished (else at once).
    pub wait_for_activities: bool,
    /// The record's other words, as decoded (unread by the rules).
    pub input: u32,
    pub kind: u32,
}

/// An activity: the types the rules act on are typed; every other type keeps the fields the export decoded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Activity {
    Known(KnownActivity),
    Other(OtherActivity),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum KnownActivity {
    /// A pose: each value set at once.
    SetKinematicValueMultiple { values: BTreeMap<String, f64> },
    KinematicBezierCurveMultiple(CurveActivity),
    /// Tells the game the gear's state: 0 up, 1 doors open, 2 down, 3 rising.
    SetLandingGearState { state: u32 },
    /// Asks the doors to open (`door[0]` 1) or close (0); `door[1]` as stored.
    DoorClientRequest { door: [u16; 2] },
    /// Moves the machine to `target`.
    SetTargetState { target: String },
    WaitSomeGameTime { seconds: f64 },
    PointLightActivity(Light),
    SpotLightActivity(Light),
    AudioState { fields: AudioStateFields },
}

/// The type names `KnownActivity` covers: an `Other` with one of them is a damaged record.
pub const KNOWN_ACTIVITIES: &[&str] = &[
    "SetKinematicValueMultiple",
    "KinematicBezierCurveMultiple",
    "SetLandingGearState",
    "DoorClientRequest",
    "SetTargetState",
    "WaitSomeGameTime",
    "PointLightActivity",
    "SpotLightActivity",
    "AudioState",
];

/// An activity type the rules don't act on (particles, voice reporters, explosion, ...): its fields as decoded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OtherActivity {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub fields: Map<String, Value>,
}

impl Activity {
    /// Elite's type name.
    pub fn type_name(&self) -> &str {
        match self {
            Activity::Other(o) => &o.kind,
            Activity::Known(k) => match k {
                KnownActivity::SetKinematicValueMultiple { .. } => "SetKinematicValueMultiple",
                KnownActivity::KinematicBezierCurveMultiple(_) => "KinematicBezierCurveMultiple",
                KnownActivity::SetLandingGearState { .. } => "SetLandingGearState",
                KnownActivity::DoorClientRequest { .. } => "DoorClientRequest",
                KnownActivity::SetTargetState { .. } => "SetTargetState",
                KnownActivity::WaitSomeGameTime { .. } => "WaitSomeGameTime",
                KnownActivity::PointLightActivity(_) => "PointLightActivity",
                KnownActivity::SpotLightActivity(_) => "SpotLightActivity",
                KnownActivity::AudioState { .. } => "AudioState",
            },
        }
    }
}

/// `KinematicBezierCurveMultiple`: Bezier curves on values, in seconds, with sound cues.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveActivity {
    pub curves: Vec<Curve>,
    /// The latest last-key time over the curves.
    pub duration_s: f64,
    pub sound_cues: Vec<SoundCue>,
    /// The four option bytes at +0x60, hex (loop, start_at_time_zero, publish_dominant_value, counts_as_running).
    pub flags: String,
    pub options: CurveOptions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    /// The kinematic value it drives (a rig motion's), or none.
    pub value: Option<String>,
    /// A float generic parameter it also writes, or none.
    pub param: Option<String>,
    /// Time s, value, in slope, in length, out slope, out length.
    pub keys: Vec<[f64; 6]>,
    /// The two Wwise game parameters it feeds.
    pub audio: Option<CurveAudio>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveAudio {
    pub progress_rtpc: Option<String>,
    pub size_rtpc: Option<String>,
    /// The block's other words, as the export dumped them (unnamed; only the cockpit's Emergency curve has any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<Value>,
}

/// The curve activity's own fields (agent_csm.md section 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveOptions {
    /// A value scaling the clock's rate (1 without).
    pub rate_input: Option<String>,
    pub input_registry: bool,
    /// A value driving the clock: time = clamp(it, 0, 1) x duration.
    pub driven_input: Option<String>,
    pub count_when_full: bool,
    /// A value weighting every curve: (v - start) x weight + start.
    pub weight_input: Option<String>,
    #[serde(rename = "loop")]
    pub looping: bool,
    /// Start the clock at 0 (else where the dominant curve passes the current value).
    pub start_at_time_zero: bool,
    /// Publish `<owner>_<value>_DominantCurveValue`, a straight ramp the resume rule solves.
    pub publish_dominant_value: bool,
    /// Whether a free-running clock reports itself running (so a waiting transition waits for it).
    pub counts_as_running: bool,
    pub blend_in_s: f64,
    pub job_order: u32,
}

/// A sound cue: at each window's start the begin event, at its end the end event (flag 1); flag 0 posts one event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundCue {
    pub cue: String,
    pub flag: u32,
    /// The emitter bones (the audio emitter component owning them names the event).
    pub bones: Vec<String>,
    pub value: Option<String>,
    pub window_count: u32,
    /// (start s, end s) into the curve.
    pub windows_s: Vec<[f64; 2]>,
    pub events: CueEvents,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub single_event_candidates: Vec<EventCandidate>,
}

/// The Wwise events a cue posts (from the export's `index.json`, proven by hash), or why it plays nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueEvents {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emitter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub begin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub begin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub banks: Vec<String>,
    /// A flag-0 cue's one event, when a bank has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plays_nothing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventCandidate {
    pub name: String,
    pub id: String,
    pub in_banks: bool,
}

/// `PointLightActivity` / `SpotLightActivity` (agent_csm.md section 5). Field names are the export's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Light {
    /// `Model!Bone` (the bone's name without its leading `!`): a light at each.
    pub bones: Vec<String>,
    pub holder: Option<String>,
    /// The flag bytes +0x58..+0x5b (+0x59 the light's priority / shadow level).
    pub bytes_88: [u8; 4],
    pub colour: [f64; 3],
    pub exposure_enable: bool,
    pub exposure_bias: f64,
    /// The exposure term's min and max.
    pub floats_112: [f64; 2],
    /// The "off" level of the intensity.
    pub float_120: f64,
    /// The "on" level: the intensity, in the renderer's units.
    pub intensity: f64,
    pub range_m: f64,
    pub source_size: f64,
    /// Point lights: +0x88, the specular scale (the raw word; spot lights never take it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specular_scale: Option<f64>,
    /// Point lights: the parameters driving the level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driven_by: Option<Vec<String>>,
    /// Spot lights: the cone, degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cone_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_fraction: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_ies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_profile_name: Option<String>,
    /// The IES profile: the texture of that name (`tex/<lower case>.dds`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ies_profile: Option<String>,
    /// The export's slot dump of the whole record (rows of `at` byte offset, `u32` / `f32` pairs, pointed-to blocks),
    /// kept for the fields no name is proven for: the intensity animator's (+0x8c..+0xd0, +0x125 on point lights: fade
    /// rates, blink timer, jitter, random-off chance; agent_csm.md section 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioStateFields {
    pub state_group: String,
    pub state: String,
    pub unread_18: u64,
}

/// The paint job record (448 bytes, `re/elite/notes/sidewinder_paintjob.md`), every named field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paint {
    /// The paint job item (`paintjob_sidewinder_default`).
    pub paint_job: String,
    /// The record's name (`DefaultPaintJob`).
    pub name: String,
    /// Elite's file holding it.
    pub source: String,
    pub folder: String,
    pub stored_number: u32,
    /// The mask texture in the mod's folder.
    pub texture_file: String,
    pub texture_block_a: PaintTextures,
    pub texture_block_b: PaintTextures,
    pub u32_48_unread: u32,
    pub high_quality_resource: Option<String>,
    /// RedKey, GreenKey, BlueKey, Background, in that order.
    pub keys: Vec<PaintKey>,
    /// Four groups: r, g, b, alpha, specular scale, roughness, metalness.
    pub decal_colour_groups: Vec<DecalGroup>,
    pub use_iridescence: u8,
    pub iridescence_axes: IridescenceAxes,
    pub iridescence_masks: IridescenceMasks,
    pub extended_effect: bool,
}

/// The paint key names in the record's order.
pub const PAINT_KEYS: [&str; 4] = ["RedKey", "GreenKey", "BlueKey", "Background"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaintTextures {
    pub separate_texture_per_mask: u8,
    pub texture: Option<String>,
    pub red_texture: Option<String>,
    pub green_texture: Option<String>,
    pub blue_texture: Option<String>,
    pub render_mode_rgb: [u8; 3],
    pub byte_3b_unread: u8,
    pub use_3x3_filter: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaintKey {
    pub name: String,
    /// Linear.
    pub colour: [f64; 3],
    pub f3_unread: f64,
    pub roughness: f64,
    pub emissive_scale: f64,
    pub metalness: f64,
    /// None on Background.
    pub sharpening_flag: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecalGroup {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub alpha: f64,
    pub specular_scale: f64,
    pub roughness: f64,
    pub metalness: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IridescenceAxes {
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(non_snake_case)]
pub struct IridescenceMasks {
    pub RedKey: f64,
    pub GreenKey: f64,
    pub BlueKey: f64,
    pub Background: f64,
}

/// What the ship item `SideWinder` and its prototype say (`sources` names where each fact comes from).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub item: String,
    pub item_path: String,
    pub hull_model: String,
    pub cockpit_model: String,
    /// The seat bones, `Model!Bone` (bone without its `!`).
    pub seats: Vec<String>,
    pub chair_models: Vec<String>,
    pub gear: Vec<GearAttach>,
    /// `Model!Bone`.
    pub cameras: Vec<String>,
    pub schematic_model: String,
    pub shadow_model: String,
    /// The hull's bones whose names say hardpoint (`!Hardpoint0`..).
    pub hardpoints: Vec<String>,
    /// The hull's bones whose names say utility mount (none on the Sidewinder).
    pub utility_mounts: Vec<String>,
    /// For each field above, where it comes from.
    pub sources: BTreeMap<String, String>,
}

/// A gear rig hung at a bone of a hull rig node's model (`LandingGearRoot!AttachLandingGearLeft`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearAttach {
    pub rig: String,
    pub node: String,
    pub bone: String,
}

impl Ship {
    /// The first rig of that name, any case (the cockpit's and the tutorial overlays' rigs are both
    /// `Sidewinder_Cockpit`: tell them apart by `file`).
    pub fn rig(&self, name: &str) -> Option<&Rig> {
        self.rigs.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// A machine by the export file it came from and its name.
    pub fn machine(&self, file: &str, name: &str) -> Option<&Machine> {
        self.machines.iter().find(|m| m.file == file && m.name == name)
    }
}

impl RigNode {
    /// This node and every node below it, parents first.
    pub fn walk(&self) -> Vec<&RigNode> {
        let mut out = vec![self];
        let mut i = 0;
        while i < out.len() {
            out.extend(out[i].children.iter());
            i += 1;
        }
        out
    }
}

impl Machine {
    pub fn state(&self, name: &str) -> Option<&State> {
        self.states.iter().find(|s| s.name == name)
    }
}

fn check_rig(rig: &Rig) -> Result<(), String> {
    let mut names = std::collections::BTreeSet::new();
    for n in rig.root.walk() {
        if !names.insert(n.name.as_str()) {
            return Err(format!("rig {}: node {} appears twice", rig.name, n.name));
        }
        if let Some(m) = n.motions.iter().find(|m| m.axis > 2) {
            return Err(format!("rig {}, node {}: motion {} has axis {}", rig.name, n.name, m.value, m.axis));
        }
        if let Some(model) = &n.model {
            if model.is_empty() || model.contains(['/', '\\', '.']) || *model != model.to_lowercase() {
                return Err(format!("rig {}, node {}: {model} is not a model file name", rig.name, n.name));
            }
        }
    }
    Ok(())
}

fn check_curves(a: &CurveActivity) -> Result<(), String> {
    if a.curves.is_empty() {
        return Err("a curve activity without curves".into());
    }
    let mut last = 0.0f64;
    for c in &a.curves {
        let name = c.value.as_deref().or(c.param.as_deref()).ok_or("a curve driving nothing")?;
        if c.keys.is_empty() {
            return Err(format!("curve {name} has no keys"));
        }
        if c.keys.windows(2).any(|w| w[1][0] <= w[0][0]) {
            return Err(format!("curve {name}: its key times don't increase"));
        }
        last = last.max(c.keys[c.keys.len() - 1][0]);
    }
    if (last - a.duration_s).abs() > 1e-6 {
        return Err(format!("duration {} s, but the curves end at {last} s", a.duration_s));
    }
    for q in &a.sound_cues {
        if q.windows_s.iter().any(|w| w[1] < w[0]) || q.windows_s.len() != q.window_count as usize {
            return Err(format!("sound cue {}: bad windows", q.cue));
        }
    }
    Ok(())
}

impl Machine {
    /// The index of the state of that name.
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }

    /// Whether some route of transitions leads from state `from` to state `to` (indices; a state reaches itself).
    pub fn reaches(&self, from: usize, to: usize) -> bool {
        let mut seen = vec![false; self.states.len()];
        let mut todo = vec![from];
        while let Some(u) = todo.pop() {
            if u == to {
                return true;
            }
            if std::mem::replace(&mut seen[u], true) {
                continue;
            }
            todo.extend(self.states[u].transitions.iter().filter_map(|t| self.state_index(&t.target)));
        }
        false
    }

    /// Where a state's `SetTargetState` moves the machine at once: its (last) target, when no route of transitions
    /// leads there (the `JumpTo*` states); None when it has none, targets itself, or can route there.
    pub fn jump_of(&self, state: usize) -> Option<usize> {
        let target = self.states[state].activities.iter().rev().find_map(|a| match a {
            Activity::Known(KnownActivity::SetTargetState { target }) => self.state_index(target),
            _ => None,
        })?;
        (target != state && !self.reaches(state, target)).then_some(target)
    }

    /// What `parse_ship` refuses in a machine (also checked by `elite_rig::Runner::new` for one made otherwise): see
    /// `parse_ship`.
    pub fn check(&self) -> Result<(), String> {
        check_machine(self)
    }
}

fn check_machine(m: &Machine) -> Result<(), String> {
    let has = |s: &str| m.states.iter().any(|x| x.name == s);
    if !has(&m.initial) {
        return Err(format!("initial state {} isn't a state", m.initial));
    }
    for s in &m.states {
        if m.states.iter().filter(|x| x.name == s.name).count() > 1 {
            return Err(format!("state {} appears twice", s.name));
        }
        if let Some(t) = s.transitions.iter().find(|t| !has(&t.target)) {
            return Err(format!("state {}: a transition to {}, which isn't a state", s.name, t.target));
        }
        for a in &s.activities {
            let at = |why: String| format!("state {}, {}: {why}", s.name, a.type_name());
            match a {
                Activity::Other(o) if KNOWN_ACTIVITIES.contains(&o.kind.as_str()) => {
                    let why = serde_json::to_value(o)
                        .and_then(serde_json::from_value::<KnownActivity>)
                        .err()
                        .map_or("its fields don't match".to_string(), |e| e.to_string());
                    return Err(at(why));
                }
                Activity::Known(KnownActivity::SetTargetState { target }) if !has(target) => {
                    return Err(at(format!("targets {target}, which isn't a state")));
                }
                Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) => check_curves(c).map_err(at)?,
                _ => {}
            }
        }
    }
    // SetTargetState jumps (no route to their target) that come back round would move the machine for ever
    for start in 0..m.states.len() {
        let mut at = start;
        for _ in 0..m.states.len() {
            match m.jump_of(at) {
                Some(next) if next == start => {
                    return Err(format!("state {}'s SetTargetState jumps come back round to it", m.states[start].name));
                }
                Some(next) => at = next,
                None => break,
            }
        }
    }
    Ok(())
}

/// Reads `ship.json`, refusing what the rules would trip on: another format, a node twice in a rig, a motion axis
/// over 2, a model that isn't a file name, a machine whose initial state, transition or `SetTargetState` names no
/// state, a curve without keys or with key times that don't increase, a duration that isn't its curves' end, a cue
/// window ending before it starts, a known activity type whose fields don't match, `SetTargetState` jumps forming a
/// loop, a paint job without its four keys in order or a mask outside `tex/`.
pub fn parse_ship(bytes: &[u8]) -> Result<Ship, ShipDataError> {
    let ship: Ship = serde_json::from_slice(bytes).map_err(|e| bad(SHIP_FILE, e.to_string()))?;
    if ship.format != SHIP_FORMAT {
        return Err(bad(SHIP_FILE, format!("format {}, this build reads {SHIP_FORMAT}", ship.format)));
    }
    for r in &ship.rigs {
        if ship.rigs.iter().filter(|x| x.file == r.file && x.name == r.name).count() > 1 {
            return Err(bad(SHIP_FILE, format!("rig {} of {} appears twice", r.name, r.file)));
        }
        check_rig(r).map_err(|e| bad(SHIP_FILE, e))?;
    }
    for m in &ship.machines {
        if ship.machines.iter().filter(|x| x.file == m.file && x.name == m.name).count() > 1 {
            return Err(bad(SHIP_FILE, format!("machine {} of {} appears twice", m.name, m.file)));
        }
        check_machine(m).map_err(|e| bad(SHIP_FILE, format!("machine {} of {}: {e}", m.name, m.file)))?;
    }
    let p = &ship.paint;
    if p.keys.iter().map(|k| k.name.as_str()).ne(PAINT_KEYS) {
        return Err(bad(SHIP_FILE, format!("paint job {}: its keys are not {PAINT_KEYS:?} in order", p.name)));
    }
    if p.decal_colour_groups.len() != 4 {
        return Err(bad(SHIP_FILE, format!("paint job {}: {} decal colour groups, not 4", p.name, p.decal_colour_groups.len())));
    }
    if !p.texture_file.starts_with("tex/") || !p.texture_file.ends_with(".dds") || p.texture_file.contains("..") {
        return Err(bad(SHIP_FILE, format!("paint job {}: {} is not a tex/*.dds file", p.name, p.texture_file)));
    }
    Ok(ship)
}

/// Writes `ship.json`: indented, but a list of plain numbers or names on one line (a curve key, a matrix row), LF
/// line endings.
pub fn write_ship(ship: &Ship) -> Vec<u8> {
    let v = serde_json::to_value(ship).expect("the ship is plain data");
    let mut out = String::new();
    write_value(&v, 0, &mut out);
    out.push('\n');
    out.into_bytes()
}

fn write_value(v: &Value, depth: usize, out: &mut String) {
    let pad = |d: usize| "  ".repeat(d);
    let flat = |v: &Value| !v.is_array() && !v.is_object();
    match v {
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Array(items) if items.iter().all(flat) => {
            let parts: Vec<String> = items.iter().map(|x| x.to_string()).collect();
            out.push('[');
            out.push_str(&parts.join(", "));
            out.push(']');
        }
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, x) in items.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                write_value(x, depth + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        Value::Object(map) => {
            out.push_str("{\n");
            for (i, (k, x)) in map.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_value(x, depth + 1, out);
                out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
        _ => out.push_str(&v.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Materials {
        let hull = Material {
            shader: Some("Metalness_Roughness_OpaqueDeferred".into()),
            textures: vec![
                TextureSlot {
                    slot: "pBaseColour_Roughness".into(),
                    value: SlotValue::Tex {
                        texture: "hull.pbasecolour_roughness".into(),
                        file: "tex/hull__pbasecolour_roughness.dds".into(),
                        srgb: true,
                        format: "BC7".into(),
                    },
                },
                TextureSlot { slot: "pNormalTexture".into(), value: SlotValue::Colour { colour: [[127, 127, 0, 127], [127, 127, 0, 255]] } },
            ],
            attrs: vec![
                Attr { name: "pVerticalTiling".into(), kind: 0, values: vec![0.25] },
                Attr { name: "pRenderLayerOverride".into(), kind: 5, values: vec![4294967295.0] },
                Attr { name: "gStandardMapScales".into(), kind: 2, values: vec![1.0, 0.1f32 as f64, -3.5] },
            ],
            fallback_to: None,
            why: None,
        };
        let null = Material { shader: Some("Null".into()), textures: vec![], attrs: vec![], fallback_to: None, why: None };
        let grey = Material { shader: None, textures: vec![], attrs: vec![], fallback_to: Some("null".into()), why: Some("no such file".into()) };
        Materials { materials: [("hull".to_string(), hull), ("null".to_string(), null), ("grey".to_string(), grey)].into_iter().collect() }
    }

    #[test]
    fn materials_round_trip() {
        let m = sample();
        let b = write_materials(&m);
        assert_eq!(parse_materials(&b).unwrap(), m);
        assert!(m.get("Hull").is_some() && m.get("GREY").unwrap().fallback_to.as_deref() == Some("null"));
        // Elite's own field names reach the file.
        let text = String::from_utf8(b).unwrap();
        for k in ["\"kind\": \"tex\"", "\"kind\": \"dt7\"", "\"type\": 5", "\"fallback_to\": \"null\"", "\"slot\": \"pNormalTexture\""] {
            assert!(text.contains(k), "{k}");
        }
    }

    #[test]
    fn materials_refusals_name_the_fault() {
        let fails = |m: &Materials, what: &str| {
            let e = parse_materials(&write_materials(m)).unwrap_err().to_string();
            assert!(e.starts_with("materials.json: ") && e.contains(what), "{e}");
        };
        let mut m = sample();
        m.materials.remove("null");
        fails(&m, "falls back to null");
        let mut m = sample();
        let hull = m.materials.remove("hull").unwrap();
        m.materials.insert("Hull".into(), hull);
        fails(&m, "lower case");
        let mut m = sample();
        m.materials.get_mut("hull").unwrap().attrs[0].kind = 9;
        fails(&m, "unknown type 9");
        let mut m = sample();
        if let SlotValue::Tex { file, .. } = &mut m.materials.get_mut("hull").unwrap().textures[0].value {
            *file = "../x.dds".into();
        }
        fails(&m, "not a tex/*.dds");
        let mut m = sample();
        m.materials.get_mut("grey").unwrap().fallback_to = None;
        fails(&m, "neither a shader nor a fallback");
        assert!(parse_materials(b"{\"materials\": [").unwrap_err().to_string().starts_with("materials.json: "));
        assert!(parse_materials(b"{\"materials\": {\"a\": {\"shader\": \"s\", \"textures\": [{\"slot\": \"p\", \"kind\": \"xyz\"}]}}}").is_err());
    }

    fn ship_sample() -> Ship {
        let motion = |kind, value: &str, axis| Motion { kind, value: value.into(), axis, a: 0.1, b: 0.0 };
        let leg = RigNode {
            name: "LandingGearLeft".into(),
            model: Some("landinggearleft".into()),
            model_ref: Some("landinggearleft".into()),
            attach_bone: None,
            attach_bone_b: None,
            parent: None,
            flags: "01010000".into(),
            u104: 128,
            motions: vec![motion(MotionKind::TranslateDirect, "Anim_TRight", 2), motion(MotionKind::RotateDriven, "Pitch", 0)],
            children: vec![RigNode {
                name: "Toe".into(),
                model: Some("reargeartoe1".into()),
                model_ref: Some("0xd5596308".into()),
                attach_bone: Some("!AttachToe".into()),
                attach_bone_b: Some("!AttachToe".into()),
                parent: Some("LandingGearLeft".into()),
                flags: "01010000".into(),
                u104: 128,
                motions: vec![],
                children: vec![],
            }],
        };
        let curve = CurveActivity {
            curves: vec![Curve {
                value: Some("Anim_TRight".into()),
                param: None,
                keys: vec![[0.0, -1.3973, 0.0, 0.3333333, 0.2145239, 0.3338828], [5.3333335, 0.0062, 0.0, 0.0111109, 0.0, 0.0]],
                audio: Some(CurveAudio { progress_rtpc: Some("kinematic_progress".into()), size_rtpc: Some("kinematic_size".into()), other: None }),
            }],
            duration_s: 5.3333335,
            sound_cues: vec![SoundCue {
                cue: "Sidewinder_Rear_Deploy_Motor01".into(),
                flag: 1,
                bones: vec!["ModelOrigin".into()],
                value: Some("Anim_TRight".into()),
                window_count: 1,
                windows_s: vec![[1.13333, 3.1]],
                events: CueEvents {
                    emitter: Some("landinggear".into()),
                    begin: Some("Sidewinder_Rear_Deploy_Motor01_b_landinggear".into()),
                    end: Some("Sidewinder_Rear_Deploy_Motor01_e_landinggear".into()),
                    begin_id: Some("4406570f".into()),
                    end_id: Some("7a239cec".into()),
                    banks: vec!["shipshared_ext_lod1_modules".into()],
                    single: None,
                    plays_nothing: false,
                    why: None,
                },
                single_event_candidates: vec![],
            }],
            flags: "00000101".into(),
            options: CurveOptions {
                rate_input: None,
                input_registry: false,
                driven_input: None,
                count_when_full: false,
                weight_input: None,
                looping: false,
                start_at_time_zero: false,
                publish_dominant_value: true,
                counts_as_running: true,
                blend_in_s: 0.0,
                job_order: 0,
            },
        };
        let light = Light {
            bones: vec!["Sidewinder!NAVLIGHT_PORT".into()],
            holder: Some("Nav".into()),
            bytes_88: [0, 1, 0, 0],
            colour: [1.0, 0.1096, 0.0],
            exposure_enable: false,
            exposure_bias: 0.0,
            floats_112: [-4.0, 20.0],
            float_120: 25.0,
            intensity: 400.0,
            range_m: 3.0,
            source_size: 0.01,
            specular_scale: Some(1.0),
            driven_by: Some(vec!["cockpitAmberLightIntensity".into()]),
            cone_deg: None,
            inner_fraction: None,
            use_ies: None,
            other_profile_name: None,
            ies_profile: None,
            raw: Some(serde_json::json!([{"at": 136, "u32": [1065353216, 0], "f32": [1.0, 0.0]}])),
        };
        let mut other = Map::new();
        other.insert("unread_8".into(), Value::from(0));
        let state = |name: &str, activities: Vec<Activity>, to: &[(&str, bool)]| State {
            name: name.into(),
            activities,
            transitions: to.iter().map(|(t, w)| Transition { target: t.to_string(), cost: 2, wait_for_activities: *w, input: 1, kind: 2 }).collect(),
        };
        let known = Activity::Known;
        let machine = Machine {
            name: "LandingGearCSM".into(),
            stored_name: "3086715247::LandingGearCSM".into(),
            owner: "Ext_LandingGear_SideWinder_Left".into(),
            file: "gear_left".into(),
            initial: "Up".into(),
            states: vec![
                state(
                    "Up",
                    vec![
                        known(KnownActivity::SetKinematicValueMultiple { values: [("Anim_TRight".to_string(), -1.3973)].into_iter().collect() }),
                        known(KnownActivity::SetLandingGearState { state: 0 }),
                        known(KnownActivity::DoorClientRequest { door: [0, 1] }),
                        Activity::Other(OtherActivity { kind: "KeepAudioEmitterComponentDisabled".into(), fields: other }),
                    ],
                    &[("Lowering", true)],
                ),
                state("Lowering", vec![known(KnownActivity::KinematicBezierCurveMultiple(curve))], &[("Down", true), ("Up", false)]),
                state(
                    "Down",
                    vec![
                        known(KnownActivity::PointLightActivity(light)),
                        known(KnownActivity::WaitSomeGameTime { seconds: 0.15 }),
                        known(KnownActivity::AudioState {
                            fields: AudioStateFields { state_group: "CockpitCrack".into(), state: "CockpitCrack_Blowout".into(), unread_18: 0 },
                        }),
                        known(KnownActivity::SetTargetState { target: "Up".into() }),
                    ],
                    &[],
                ),
            ],
        };
        let key = |name: &str, sharpening_flag| PaintKey {
            name: name.into(),
            colour: [0.279061, 0.032559, 0.022036],
            f3_unread: 0.4,
            roughness: 0.1,
            emissive_scale: 0.0,
            metalness: 0.0,
            sharpening_flag,
        };
        let block = PaintTextures {
            separate_texture_per_mask: 0,
            texture: Some("texture".into()),
            red_texture: None,
            green_texture: None,
            blue_texture: None,
            render_mode_rgb: [1, 1, 1],
            byte_3b_unread: 1,
            use_3x3_filter: 0,
        };
        let group = DecalGroup { r: 0.078431, g: 0.019608, b: 0.011765, alpha: 0.988235, specular_scale: 0.01, roughness: 0.73, metalness: 0.0 };
        Ship {
            format: SHIP_FORMAT,
            rigs: vec![Rig {
                name: "Ext_LandingGear_SideWinder_Left".into(),
                file: "gear_left".into(),
                source: "98c94da8f570e6c2253c8712c361df4c6156ddc3".into(),
                category: None,
                root: leg,
            }],
            machines: vec![machine],
            paint: Paint {
                paint_job: "paintjob_sidewinder_default".into(),
                name: "DefaultPaintJob".into(),
                source: "b9979cbb9b5db39d69d95c3b6a6a8e55e40cbd09".into(),
                folder: "items\\ships\\sidewinder".into(),
                stored_number: 2779398536,
                texture_file: "tex/paintjob_sidewinder_default__texture.dds".into(),
                texture_block_a: block.clone(),
                texture_block_b: PaintTextures { texture: None, ..block },
                u32_48_unread: 1,
                high_quality_resource: None,
                keys: vec![key("RedKey", Some(0)), key("GreenKey", Some(0)), key("BlueKey", Some(0)), key("Background", None)],
                decal_colour_groups: vec![group.clone(), group.clone(), group.clone(), group],
                use_iridescence: 0,
                iridescence_axes: IridescenceAxes { x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0] },
                iridescence_masks: IridescenceMasks { RedKey: 1.0, GreenKey: 1.0, BlueKey: 1.0, Background: 1.0 },
                extended_effect: false,
            },
            item: Item {
                item: "SideWinder".into(),
                item_path: "Items/Ships/SideWinder/SideWinder".into(),
                hull_model: "sidewinder".into(),
                cockpit_model: "sidewinder_cockpit".into(),
                seats: vec!["Sidewinder_Cockpit!Pilot_Chair".into()],
                chair_models: vec!["cockpit_chair".into()],
                gear: vec![GearAttach { rig: "Ext_LandingGear_SideWinder_Left".into(), node: "LandingGearRoot".into(), bone: "!AttachLandingGearLeft".into() }],
                cameras: vec!["Sidewinder!Camera_Ship_Back".into()],
                schematic_model: "sidewinder_schematic".into(),
                shadow_model: "shadow_sidewinder".into(),
                hardpoints: vec!["!Hardpoint0".into()],
                utility_mounts: vec![],
                sources: [("seats".to_string(), "a note".to_string())].into_iter().collect(),
            },
        }
    }

    #[test]
    fn ship_round_trip() {
        let ship = ship_sample();
        let b = write_ship(&ship);
        assert_eq!(parse_ship(&b).unwrap(), ship);
        let text = String::from_utf8(b).unwrap();
        // Elite's names reach the file; a key and a window stay on one line
        for k in [
            "\"type\": \"TranslateDirect\"",
            "\"type\": \"KinematicBezierCurveMultiple\"",
            "\"type\": \"KeepAudioEmitterComponentDisabled\"",
            "\"unread_8\": 0",
            "\"loop\": false",
            "[0.0, -1.3973, 0.0, 0.3333333, 0.2145239, 0.3338828]",
            "[1.13333, 3.1]",
            "\"model_ref\": \"0xd5596308\"",
        ] {
            assert!(text.contains(k), "{k}");
        }
        assert!(!text.contains('\r'));
        let m = &parse_ship(text.as_bytes()).unwrap().machines[0];
        assert_eq!(m.states[0].activities[3].type_name(), "KeepAudioEmitterComponentDisabled");
        assert!(matches!(m.states[0].activities[3], Activity::Other(_)));
    }

    #[test]
    fn ship_refusals_name_the_fault() {
        let fails = |s: &Ship, what: &str| {
            let e = parse_ship(&write_ship(s)).unwrap_err().to_string();
            assert!(e.starts_with("ship.json: ") && e.contains(what), "{e}");
        };
        let mut s = ship_sample();
        s.format = 2;
        fails(&s, "format 2");
        let mut s = ship_sample();
        s.machines[0].states[1].transitions[0].target = "Sideways".into();
        fails(&s, "a transition to Sideways");
        let mut s = ship_sample();
        s.machines[0].initial = "Nowhere".into();
        fails(&s, "initial state Nowhere");
        let mut s = ship_sample();
        s.machines[0].states[2].activities[3] = Activity::Known(KnownActivity::SetTargetState { target: "Gone".into() });
        fails(&s, "targets Gone");
        let mut s = ship_sample();
        if let Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) = &mut s.machines[0].states[1].activities[0] {
            c.curves[0].keys.swap(0, 1);
        }
        fails(&s, "key times don't increase");
        let mut s = ship_sample();
        if let Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) = &mut s.machines[0].states[1].activities[0] {
            c.duration_s = 6.0;
        }
        fails(&s, "duration 6 s");
        let mut s = ship_sample();
        s.paint.keys.swap(0, 1);
        fails(&s, "keys are not");
        let mut s = ship_sample();
        s.rigs[0].root.children[0].model = Some("../x.edm".into());
        fails(&s, "is not a model file name");
        let mut s = ship_sample();
        s.rigs[0].root.motions[0].axis = 3;
        fails(&s, "axis 3");
        // a known type whose fields don't match is refused, not kept as an unknown type
        let mut fields = Map::new();
        fields.insert("state".into(), Value::from("two"));
        let mut s = ship_sample();
        s.machines[0].states[0].activities[1] = Activity::Other(OtherActivity { kind: "SetLandingGearState".into(), fields });
        fails(&s, "state Up, SetLandingGearState");
        assert!(parse_ship(b"{\"format\": 1").unwrap_err().to_string().starts_with("ship.json: "));
        // Up and Down jump to each other by SetTargetState, neither with a route to the other
        let mut s = ship_sample();
        s.machines[0].states[1].transitions.clear();
        s.machines[0].states[0].transitions.clear();
        s.machines[0].states[0].activities.push(Activity::Known(KnownActivity::SetTargetState { target: "Down".into() }));
        fails(&s, "machine LandingGearCSM of gear_left: state Up's SetTargetState jumps come back round");
        // with a route from Down back to Up, Down's SetTargetState routes there and nothing loops
        s.machines[0].states[2].transitions.push(Transition { target: "Up".into(), cost: 2, wait_for_activities: true, input: 1, kind: 2 });
        assert!(parse_ship(&write_ship(&s)).is_ok());
    }
}
