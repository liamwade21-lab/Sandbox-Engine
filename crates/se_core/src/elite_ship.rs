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
    /// Spot lights +0x8c: the inner cone, degrees; +0x18f: with `use_ies` set, the inner cone is 0.9 x the cone instead
    /// (FUN_1435cc920; re/elite/notes/agent_paint.md, "The spot cone"). Typed from `raw` by the converter (`spot_inner`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_from_cone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_ies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_profile_name: Option<String>,
    /// The IES profile: the texture of that name (`tex/<lower case>.dds`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ies_profile: Option<String>,
    /// The intensity animator's fields, typed from `raw` by the converter (`LightAnimator::from_raw`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animator: Option<LightAnimator>,
    /// The export's slot dump of the whole record (rows of `at` byte offset, `u32` / `f32` pairs, pointed-to blocks), kept
    /// for the words no name is proven for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// A light's intensity animator as the exe builds it from the record (point lights FUN_1435cb790, spot lights FUN_1435cc920,
/// both into FUN_1435ea060; evaluated every frame by FUN_143598ba0; re/elite/notes/agent_paint.md section 6). Times in
/// seconds. Every field names the record's offset it comes from (point / spot); a word the dump leaves out is zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightAnimator {
    /// +0x8c / +0xc0: the jitter is not faded by the level (it takes 1 while the exposure factor is above 0).
    pub hard: bool,
    /// +0x90 / +0xc4 (an integer): how much of a period the first jitter draw's timer is spread by.
    pub jitter_spread: u32,
    /// +0x94 / +0xc8 and +0x98 / +0xcc: each jitter draw is uniform in [-jitter_below, jitter_above].
    pub jitter_below: f64,
    pub jitter_above: f64,
    /// +0x9c / +0xd0: draws a second (0: no jitter).
    pub jitter_rate_hz: f64,
    /// +0xa0 (point lights): the jitter runs when a draw in [0, 1) at creation is at most this. Spot lights take a byte of
    /// their runtime object instead (FUN_1435cc920 +0x28c, its writer not traced): None.
    pub jitter_chance: Option<f64>,
    /// The timed blink (FUN_1435ea060): +0xa4 / +0xd8 its rate r (0: none); an on phase lasts duty x 2 / r s and an off
    /// phase (1 - duty) x 2 / r s, with +0xb0 / +0xe4 the duty; +0xa8 / +0xdc a start delay of start / r s; +0xac / +0xe0 a
    /// random extra of up to extra / r s per phase.
    pub blink_rate: f64,
    pub blink_start: f64,
    pub blink_extra: f64,
    pub blink_duty: f64,
    /// +0xb8 / +0xe8: the step pattern (`N` on, `F` off, each followed by its step count; FUN_1435dae40), which replaces the
    /// timed blink; +0xc0 / +0xf0 the pattern's cycle.
    pub pattern: Option<String>,
    pub pattern_cycle_s: f64,
    /// +0xc4 / +0xf4: how long the light stays on after it is switched off, before it fades.
    pub hold_s: f64,
    /// +0xc8 / +0xf8 and +0xcc / +0xfc: the fades' lengths, which also limit how fast the parameters' product moves.
    pub fade_in_s: f64,
    pub fade_out_s: f64,
    /// +0xd0 / +0x100: the parameters' product is raised to this.
    pub exponent: f64,
    /// Point lights +0xe8: the float parameters multiplied into the level (the same names as `Light::driven_by`). No spot
    /// light of the ship has a list there.
    pub float_params: Vec<String>,
    /// Point lights +0xf8, spot lights +0x118: the bool parameters that switch the light, combined by `bool_mode`.
    pub bool_params: Vec<String>,
    /// Point lights +0x125: 0 all true, 1 any true, 2 none true, anything else the light's own on state alone. Spot lights
    /// pass 0. (The byte lies inside the +0x120 word: on a light with a colour parameter it is a byte of that name's
    /// pointer at run time; no such light of the ship has a bool parameter, so it changes nothing there.)
    pub bool_mode: u8,
    /// Point lights +0x120: the colour parameter that replaces the record's colour when it exists (FUN_1435a5500's +0x208).
    pub colour_param: Option<String>,
}

/// The record's offsets of the animator's fields, by kind (FUN_1435cb790 and FUN_1435cc920 build the same 17-float block).
struct AnimatorOffsets {
    hard: usize,
    spread: usize,
    below: usize,
    above: usize,
    rate: usize,
    chance: Option<usize>,
    blink: usize,
    pattern: usize,
    cycle: usize,
    floats: Option<usize>,
    bools: usize,
    mode: Option<usize>,
    colour: Option<usize>,
}

const POINT_ANIMATOR: AnimatorOffsets =
    AnimatorOffsets { hard: 0x8c, spread: 0x90, below: 0x94, above: 0x98, rate: 0x9c, chance: Some(0xa0), blink: 0xa4, pattern: 0xb8, cycle: 0xc0, floats: Some(0xe8), bools: 0xf8, mode: Some(0x125), colour: Some(0x120) };
const SPOT_ANIMATOR: AnimatorOffsets =
    AnimatorOffsets { hard: 0xc0, spread: 0xc4, below: 0xc8, above: 0xcc, rate: 0xd0, chance: None, blink: 0xd8, pattern: 0xe8, cycle: 0xf0, floats: None, bools: 0x118, mode: None, colour: None };

/// The export's slot dump (`Light::raw`): 8-byte rows by offset.
struct Dump<'a>(&'a [Value]);

impl Dump<'_> {
    fn row(&self, at: usize) -> Option<&Value> {
        self.0.iter().find(|r| r.get("at").and_then(Value::as_u64) == Some(at as u64))
    }

    /// The 32-bit word at `at` (4-aligned); zero when the dump leaves it out.
    fn word(&self, at: usize) -> u32 {
        self.row(at & !7).and_then(|r| r.get("u32")?.get((at & 4) / 4)?.as_u64()).map_or(0, |w| w as u32)
    }

    fn f32(&self, at: usize) -> f64 {
        f32::from_bits(self.word(at)) as f64
    }

    fn byte(&self, at: usize) -> u8 {
        (self.word(at & !3) >> (8 * (at & 3))) as u8
    }

    fn text(&self, at: usize) -> Option<String> {
        self.row(at).and_then(|r| r.get("str")?.as_str()).map(str::to_string)
    }

    /// A list of strings: the pointer row's block (a string every 8 bytes) and the count at `at` + 8, which must agree.
    fn texts(&self, at: usize) -> Result<Vec<String>, String> {
        let count = self.word(at + 8) as usize;
        let names: Vec<String> = match self.row(at).and_then(|r| r.get("block")?.as_array()) {
            Some(block) => (0..count).map(|i| block.iter().find(|r| r.get("at").and_then(Value::as_u64) == Some(8 * i as u64)).and_then(|r| r.get("str")?.as_str()).map(str::to_string)).collect::<Option<_>>().ok_or_else(|| format!("the list at +{at:#x} holds fewer than its {count} names"))?,
            None if count == 0 => Vec::new(),
            None => return Err(format!("the list at +{at:#x} counts {count} names but points at none")),
        };
        Ok(names)
    }
}

impl LightAnimator {
    /// The animator's fields from a light's slot dump (`spot`: a `SpotLightActivity`'s offsets).
    pub fn from_raw(raw: &[Value], spot: bool) -> Result<LightAnimator, String> {
        let o = if spot { &SPOT_ANIMATOR } else { &POINT_ANIMATOR };
        let d = Dump(raw);
        if spot && d.row(0x108).is_some_and(|r| r.get("block").is_some()) {
            return Err("a spot light with a list at +0x108, whose meaning isn't known".into());
        }
        let pattern = d.text(o.pattern);
        if let Some(p) = &pattern {
            parse_pattern(p)?;
        }
        Ok(LightAnimator {
            hard: d.f32(o.hard) != 0.0,
            jitter_spread: d.word(o.spread),
            jitter_below: d.f32(o.below),
            jitter_above: d.f32(o.above),
            jitter_rate_hz: d.f32(o.rate),
            jitter_chance: o.chance.map(|at| d.f32(at)),
            blink_rate: d.f32(o.blink),
            blink_start: d.f32(o.blink + 4),
            blink_extra: d.f32(o.blink + 8),
            blink_duty: d.f32(o.blink + 12),
            pattern,
            pattern_cycle_s: d.f32(o.cycle),
            hold_s: d.f32(o.cycle + 4),
            fade_in_s: d.f32(o.cycle + 8),
            fade_out_s: d.f32(o.cycle + 12),
            exponent: d.f32(o.cycle + 16),
            float_params: o.floats.map(|at| d.texts(at)).transpose()?.unwrap_or_default(),
            bool_params: d.texts(o.bools)?,
            bool_mode: o.mode.map_or(0, |at| d.byte(at)),
            colour_param: o.colour.and_then(|at| d.text(at)),
        })
    }
}

/// A spot light's inner cone from its slot dump: (+0x8c in degrees, +0x18f set).
pub fn spot_inner(raw: &[Value]) -> (f64, bool) {
    let d = Dump(raw);
    (d.f32(0x8c), d.byte(0x18f) != 0)
}

/// A blink pattern's steps (FUN_1435dae40): letters are taken upper case; spaces, tabs and line breaks are skipped; `N`
/// starts an on run and `F` an off run, and the next characters must be its decimal step count (a count of 0 adds
/// nothing). Anything else where a letter is due, or no digit where a count is due, is an error; a last letter without a
/// count adds nothing (the exe only checks for the count when it reads the next character). No steps: the timed blink.
pub fn parse_pattern(text: &str) -> Result<Vec<bool>, String> {
    let mut steps = Vec::new();
    let mut run: Option<bool> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let c = c.to_ascii_uppercase();
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            continue;
        }
        match run.take() {
            Some(on) => {
                let mut digits = String::from(c);
                if !c.is_ascii_digit() {
                    return Err(format!("a step count is missing in the blink pattern {text:?}"));
                }
                while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    digits.push(*d);
                    chars.next();
                }
                let n: usize = digits.parse().map_err(|_| format!("a step count too large in the blink pattern {text:?}"))?;
                steps.extend(std::iter::repeat_n(on, n));
            }
            None => {
                run = Some(match c {
                    'N' => true,
                    'F' => false,
                    _ => return Err(format!("invalid light state {c:?} in the blink pattern {text:?}")),
                });
            }
        }
    }
    Ok(steps)
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

// ------------------------------------------------------------------------------------------------ chair/
//
// The pilot chair's animation (written by `copy_sidewinder` from the decoded export's `animation/chair_clips.json`,
// `chair_clips.bin` and `chair_motiongraph.json`; the proofs are `re/elite/notes/sidewinder_chair.md` and
// `agent_motiongraph.md`). The rules that play it are `crate::elite_chair`.
//
// `chair/clips.bin`: each shipped clip's 16-bit keys exactly as the game's own decoder writes them, little-endian, 4 x
// i16 per key, frame-major: the rotations `[frames][rot channels][x, y, z, w]` at `rot_keys_at`, then the positions
// `[frames][pos channels][x, y, z, 32767]` at `pos_keys_at` (byte offsets). `chair/clips.json` describes them;
// `chair/graph.json` is the motion graph trimmed to the states a seated solo pilot's chair can be in.

/// `chair/clips.json` and `chair/graph.json`'s format number; the parsers refuse any other.
pub const CHAIR_FORMAT: u32 = 1;

/// How deep activities may start activities (a group, a trigger, a button, a clip): a bound so no file can make the
/// rules walk without end. The stock graph nests 3 deep (the buffet group, its triggers, their clips).
pub const CHAIR_MAX_NESTING: usize = 8;

const CHAIR_CLIPS_FILE: &str = "chair/clips.json";
const CHAIR_CLIPS_BIN: &str = "chair/clips.bin";
const CHAIR_GRAPH_FILE: &str = "chair/graph.json";

/// `chair/clips.json` with the keys of `chair/clips.bin` (`keys`, not in the JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairClips {
    pub format: u32,
    /// Elite's clip file (`SharedChairAnimation`, 40 hex).
    pub source: String,
    /// The model whose skeleton the clips were matched to.
    pub skeleton: String,
    /// The key file in the mod's folder.
    pub keys_file: String,
    pub key_layout: String,
    pub clips: Vec<ChairClip>,
    /// The key file's 16-bit values, in file order.
    #[serde(skip)]
    pub keys: Vec<i16>,
}

/// One clip (`mani`), as Elite's record holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairClip {
    /// Elite's name in lower case (the motion graph writes it in mixed case: match without case).
    pub name: String,
    /// The animation set (`manis`) it came from.
    pub set: String,
    pub duration_s: f32,
    /// One key per frame, evenly spaced over the duration.
    pub frames: u32,
    /// The record's flags: bit 0 additive, bit 2 compressed, bit 3 set at load.
    pub flags: u32,
    /// Flag bit 0: the keys are deltas laid over the pose under them.
    pub additive: bool,
    pub fps: f32,
    /// Bone count of the skeleton the clip was made for (142, or 144 with the centre-screen arm).
    pub skeleton_bones: u32,
    pub rot_channels: Vec<ChairChannel>,
    pub pos_channels: Vec<ChairChannel>,
    /// Each position table entry's offset and scale: a key is `x / 32767 x scale + offset`, float32.
    pub pos_offset: Vec<[f32; 3]>,
    pub pos_scale: Vec<[f32; 3]>,
    /// Byte offsets of the rotation and position keys in the key file.
    pub rot_keys_at: u64,
    pub pos_keys_at: u64,
}

/// A channel: the bone it moves (by name; its index in the skeleton the clip was made for), and for a position
/// channel its table entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairChannel {
    pub bone: String,
    pub skeleton_index: u16,
    /// False for the two centre-screen bones the Sidewinder's chair does not have (they drive nothing).
    pub in_chair: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<u8>,
}

impl ChairClips {
    /// A clip by name, without case.
    pub fn clip(&self, name: &str) -> Option<&ChairClip> {
        self.clips.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// The byte range of a clip's rotation or position keys in the key file, when it can be worked out without overflow.
    pub fn key_range(clip: &ChairClip, rotations: bool) -> Option<std::ops::Range<u64>> {
        let (start, count) = if rotations { (clip.rot_keys_at, clip.rot_channels.len()) } else { (clip.pos_keys_at, clip.pos_channels.len()) };
        let bytes = (clip.frames as u64).checked_mul(count as u64)?.checked_mul(8)?;
        Some(start..start.checked_add(bytes)?)
    }

    /// What `parse_chair_clips` refuses (the key file is `keys`): another format, a clip twice, no frames, a duration
    /// that isn't positive and finite, an additive flag that disagrees with bit 0, a position channel without a table
    /// entry in range, offset and scale lists of different lengths or not finite, a channel's keys outside the key file,
    /// off a key's boundary or past what the numbers can hold. Every sampling index then lies inside `keys`.
    pub fn check(&self) -> Result<(), ShipDataError> {
        let fail = |why: String| bad(CHAIR_CLIPS_FILE, why);
        if self.format != CHAIR_FORMAT {
            return Err(fail(format!("format {}, this build reads {CHAIR_FORMAT}", self.format)));
        }
        let bin_len = self.keys.len() as u64 * 2;
        for (i, clip) in self.clips.iter().enumerate() {
            let at = |why: String| fail(format!("clip {}: {why}", clip.name));
            if self.clips[..i].iter().any(|o| o.name.eq_ignore_ascii_case(&clip.name)) {
                return Err(at("appears twice".into()));
            }
            if clip.frames == 0 || !(clip.duration_s > 0.0 && clip.duration_s.is_finite()) {
                return Err(at(format!("{} frames over {} s", clip.frames, clip.duration_s)));
            }
            if clip.additive != (clip.flags & 1 != 0) {
                return Err(at(format!("additive {} but flags {:#x}", clip.additive, clip.flags)));
            }
            if clip.pos_offset.len() != clip.pos_scale.len() {
                return Err(at(format!("{} position offsets for {} scales", clip.pos_offset.len(), clip.pos_scale.len())));
            }
            if clip.pos_offset.iter().chain(&clip.pos_scale).flatten().any(|v| !v.is_finite()) {
                return Err(at("a position offset or scale is not finite".into()));
            }
            if let Some(ch) = clip.rot_channels.iter().find(|ch| ch.table.is_some()) {
                return Err(at(format!("rotation channel {} names a position table entry", ch.bone)));
            }
            if let Some(ch) = clip.pos_channels.iter().find(|ch| ch.table.map_or(true, |t| t as usize >= clip.pos_offset.len())) {
                return Err(at(format!("position channel {} has no table entry among the {}", ch.bone, clip.pos_offset.len())));
            }
            for (what, rotations) in [("rotation", true), ("position", false)] {
                let start = if rotations { clip.rot_keys_at } else { clip.pos_keys_at };
                match Self::key_range(clip, rotations) {
                    None => return Err(at(format!("its {what} keys' offset {start} and size overflow"))),
                    Some(r) if start % 8 != 0 || r.end > bin_len => {
                        return Err(bad(CHAIR_CLIPS_BIN, format!("clip {}: its {what} keys ({} bytes at {start}) are not inside the {bin_len}-byte key file", clip.name, r.end - r.start)));
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }
}

/// Reads `chair/clips.json` and `chair/clips.bin`, refusing what `ChairClips::check` refuses and a key file of odd length.
pub fn parse_chair_clips(json: &[u8], bin: &[u8]) -> Result<ChairClips, ShipDataError> {
    let mut c: ChairClips = serde_json::from_slice(json).map_err(|e| bad(CHAIR_CLIPS_FILE, e.to_string()))?;
    if bin.len() % 2 != 0 {
        return Err(bad(CHAIR_CLIPS_BIN, format!("{} bytes, not whole 16-bit keys", bin.len())));
    }
    c.keys = bin.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    c.check()?;
    Ok(c)
}

/// Writes `chair/clips.json` (pretty-printed, LF line endings) and `chair/clips.bin`.
pub fn write_chair_clips(c: &ChairClips) -> (Vec<u8>, Vec<u8>) {
    let mut json = serde_json::to_vec_pretty(c).expect("the clip table is plain data");
    json.push(b'\n');
    (json, c.keys.iter().flat_map(|k| k.to_le_bytes()).collect())
}

/// `chair/graph.json`: the motion graph `CockpitChairMotionGraph` for the states a seated solo pilot's chair can be
/// in (the template's initial, occupied and vacated values and the cheapest routes between them), with the variables,
/// enums, blend hierarchy and the chair templates' values the rules read. Values as Elite's file holds them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairGraph {
    pub format: u32,
    /// Elite's motion graph file (40 hex).
    pub source: String,
    pub build: String,
    pub variables: Vec<ChairVariable>,
    /// The loop enum (`CockpitChairAction_Loop`'s names, in order) and the one-shot enum.
    pub loop_enum: Vec<String>,
    pub oneshot_enum: Vec<String>,
    /// The root action handler's `VariableNamePattern`.
    pub action_pattern: String,
    /// The root handler's results that are states: key = loop enum index + 1.
    pub root_outputs: Vec<ChairOutput>,
    pub states: Vec<ChairState>,
    /// By the export's id (`pool:offset`).
    pub activities: BTreeMap<String, ChairActivity>,
    /// Each transition block's first group (the only one the exe reads), by id.
    pub transition_blocks: BTreeMap<String, ChairBlock>,
    /// The root's default block, used when the current state lists no transition to the target.
    pub default_block: String,
    pub hierarchy: ChairHierarchy,
    pub templates: ChairTemplates,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairVariable {
    pub name: String,
    /// 1 float, 2 event, 3 enum.
    #[serde(rename = "type")]
    pub kind: u32,
    pub type_name: String,
    pub min: f32,
    pub max: f32,
    pub default_i32: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairOutput {
    pub key: u32,
    pub enum_name: String,
    pub state: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairState {
    pub id: u32,
    /// Activity ids, in the order the state starts them.
    pub activities: Vec<String>,
    /// "From this state you may go to those states" through the block (targets outside the shipped states dropped).
    pub transitions: Vec<ChairTransition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairTransition {
    pub block: String,
    pub targets: Vec<u32>,
}

/// An activity, by Elite's type name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ChairActivity {
    AnimationActivity {
        clip: String,
        /// Bit 0 loop, 1 additive edge, 2 mirror, 3 edge event weight, 4 LoopEvent at the wrap, 5 re-evaluate, 6 no stream.
        flags: u32,
        /// The blend layer it enters.
        layer: u32,
        weight: ChairParam,
        /// Clip seconds per second (negative: backwards).
        rate: ChairParam,
        /// Below 0: random.
        start_phase: f32,
        phase_variable: Option<String>,
        phase_offset: f32,
        phase_output_variable: Option<String>,
    },
    GroupedActivityActivity { children: Vec<String> },
    TriggeredActivityActivity {
        trigger_variable: String,
        activity: String,
        fade_in_s: f32,
        fade_out_s: f32,
        hold_min_flag: u8,
        min_time_s: f32,
        output_variable: Option<String>,
    },
    JoystickButtonActivity {
        button_variable: String,
        #[serde(rename = "in")]
        in_activity: String,
        #[serde(rename = "loop")]
        loop_activity: String,
        #[serde(rename = "out")]
        out_activity: String,
    },
}

/// A ParamBlock: a constant, or a curve of a bound variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairParam {
    #[serde(rename = "const")]
    pub constant: f32,
    pub bound: bool,
    pub variable: Option<String>,
    pub keys: Vec<ChairKey>,
}

/// A curve key: the left key's `interp` shapes the span to the next (0 step, 1 linear, 2 pow, 3 exp, 4 smoothstep,
/// 5 hermite with tangents `a`, `b`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairKey {
    pub x: f32,
    pub y: f32,
    pub interp: u8,
    pub a: f32,
    pub b: f32,
}

/// A transition block's group 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairBlock {
    /// Where it sits in Elite's file.
    pub at: String,
    /// Without a curve: a linear fade over this; below 0: 0.2 s; 0: a cut.
    pub fade_time_s: f32,
    pub curve: Option<Vec<ChairKey>>,
    /// Wait for the current state's completion (or its LoopEvent) before changing.
    pub wait_flag: u16,
    pub wait_slack_s: f32,
    /// Route along the cheapest path when no listed transition matches (the default block).
    pub route_by_path_table: u16,
    /// 0xffff: unusable for routing.
    pub path_cost: u16,
}

/// The stored blend hierarchy (the layer nodes under the hook are made at run time).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairHierarchy {
    pub flags: u32,
    pub top_node: u32,
    pub nodes: Vec<ChairHierNode>,
    /// (parent, child) node indices.
    pub links: Vec<[u32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairHierNode {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: u32,
    pub type_name: String,
}

/// The chair prototype's template values a seated pilot needs (decrypted prototype data, values only).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChairTemplates {
    /// ChairMotiongraphComponent +0x78: the loop handler the seat writes to.
    pub loop_handler: String,
    /// MotionGraphComponent +0x58: the loop value the graph starts with.
    pub initial_loop: String,
    /// ChairMotiongraphComponent +0x80 (written on occupy) and +0x90 (on vacate).
    pub occupied_loop: String,
    pub vacated_loop: String,
    /// +0x10 / +0x18: the graph variable fed the smoothed throttle and the control it reads.
    pub throttle_variable: String,
    pub throttle_control: String,
    /// +0x20 / +0x28: the trigger.
    pub trigger_variable: String,
    pub trigger_control: String,
    /// +0x30 (stick y), +0x38 (stick x), +0x40.
    pub pitch_control: String,
    pub roll_control: String,
    pub yaw_control: String,
    /// +0x48, +0x50, +0x58.
    pub joystick_weight_variable: String,
    pub joystick_prop_variable: String,
    pub joystick_yaw_variable: String,
    /// CockpitCollisionAnimationComponent +0x10: the six buffet events in table order (x < 0, x >= 0, z >= 0, z < 0,
    /// y < 0, y >= 0).
    pub buffet_events: Vec<String>,
    /// +0x20, +0x28.
    pub buffet_intensity_variable: String,
    pub buffet_speed_variable: String,
    /// CockpitComponent +0x40, +0x44, +0x48: the per-axis thresholds of the velocity change (x, y, z).
    pub buffet_thresholds: [f32; 3],
}

impl ChairGraph {
    pub fn state(&self, id: u32) -> Option<&ChairState> {
        self.states.iter().find(|s| s.id == id)
    }

    pub fn variable(&self, name: &str) -> Option<&ChairVariable> {
        self.variables.iter().find(|v| v.name == name)
    }

    /// The state the root handler picks for a loop value (by name), if it is a shipped state.
    pub fn output_state(&self, loop_value: &str) -> Option<u32> {
        let index = self.loop_enum.iter().position(|n| n.eq_ignore_ascii_case(loop_value))?;
        self.root_outputs.iter().find(|o| o.key as usize == index + 1).map(|o| o.state)
    }

    /// Every clip an activity names.
    pub fn clips(&self) -> Vec<&str> {
        self.activities.values().filter_map(|a| if let ChairActivity::AnimationActivity { clip, .. } = a { Some(clip.as_str()) } else { None }).collect()
    }

    /// What `parse_chair_graph` refuses: another format, a state twice, a state, activity, block or variable named and
    /// missing, a root output outside the loop enum, a curve without keys or whose keys don't increase, an unknown
    /// interpolation, a number that isn't finite, activities that start themselves (through any chain) or nest deeper
    /// than `CHAIR_MAX_NESTING`, six buffet events not, or template loop values not in the loop enum.
    pub fn check(&self) -> Result<(), ShipDataError> {
        let fail = |why: String| bad(CHAIR_GRAPH_FILE, why);
        let g = self;
        if g.format != CHAIR_FORMAT {
            return Err(fail(format!("format {}, this build reads {CHAIR_FORMAT}", g.format)));
        }
        for (i, s) in g.states.iter().enumerate() {
            if g.states[..i].iter().any(|o| o.id == s.id) {
                return Err(fail(format!("state {} appears twice", s.id)));
            }
            if let Some(a) = s.activities.iter().find(|a| !g.activities.contains_key(*a)) {
                return Err(fail(format!("state {}: no activity {a}", s.id)));
            }
            for t in &s.transitions {
                if !g.transition_blocks.contains_key(&t.block) {
                    return Err(fail(format!("state {}: no transition block {}", s.id, t.block)));
                }
                if let Some(n) = t.targets.iter().find(|n| g.state(**n).is_none()) {
                    return Err(fail(format!("state {}: a transition to state {n}, which isn't there", s.id)));
                }
            }
        }
        for o in &g.root_outputs {
            if o.key == 0 || g.loop_enum.get(o.key as usize - 1) != Some(&o.enum_name) || g.state(o.state).is_none() {
                return Err(fail(format!("root output {} ({}): not loop enum index + 1, or no state {}", o.key, o.enum_name, o.state)));
            }
        }
        for (id, a) in &g.activities {
            let at = |why: String| fail(format!("activity {id}: {why}"));
            let var = |n: &String| if g.variable(n).is_some() { Ok(()) } else { Err(at(format!("no variable {n}"))) };
            if let Some(n) = a.children().into_iter().find(|n| !g.activities.contains_key(*n)) {
                return Err(at(format!("no activity {n}")));
            }
            match a {
                ChairActivity::AnimationActivity { weight, rate, phase_variable, start_phase, phase_offset, .. } => {
                    check_param(weight, g).map_err(|e| at(format!("weight: {e}")))?;
                    check_param(rate, g).map_err(|e| at(format!("rate: {e}")))?;
                    if let Some(v) = phase_variable {
                        var(v)?;
                    }
                    if !start_phase.is_finite() || !phase_offset.is_finite() {
                        return Err(at("its start phase or phase offset is not finite".into()));
                    }
                }
                ChairActivity::GroupedActivityActivity { .. } => {}
                ChairActivity::TriggeredActivityActivity { trigger_variable, fade_in_s, fade_out_s, min_time_s, .. } => {
                    var(trigger_variable)?;
                    if ![fade_in_s, fade_out_s, min_time_s].iter().all(|v| v.is_finite()) {
                        return Err(at("a fade or minimum time is not finite".into()));
                    }
                }
                ChairActivity::JoystickButtonActivity { button_variable, .. } => var(button_variable)?,
            }
        }
        for s in &g.states {
            g.check_nesting(s).map_err(|e| fail(format!("state {}: {e}", s.id)))?;
        }
        if !g.transition_blocks.contains_key(&g.default_block) {
            return Err(fail(format!("no default block {}", g.default_block)));
        }
        for (id, b) in &g.transition_blocks {
            if !b.fade_time_s.is_finite() || !b.wait_slack_s.is_finite() {
                return Err(fail(format!("block {id}: its fade time or wait slack is not finite")));
            }
            if let Some(c) = &b.curve {
                if c.is_empty() {
                    return Err(fail(format!("block {id}: a curve without keys")));
                }
                check_keys(c).map_err(|e| fail(format!("block {id}: {e}")))?;
            }
        }
        let t = &g.templates;
        for v in [&t.initial_loop, &t.occupied_loop, &t.vacated_loop] {
            if !g.loop_enum.iter().any(|n| n.eq_ignore_ascii_case(v)) {
                return Err(fail(format!("template loop value {v} is not in the loop enum")));
            }
        }
        if t.buffet_events.len() != 6 {
            return Err(fail(format!("{} buffet events, not 6", t.buffet_events.len())));
        }
        if !t.buffet_thresholds.iter().all(|v| v.is_finite()) {
            return Err(fail("a buffet threshold is not finite".into()));
        }
        for v in t.buffet_events.iter().chain([
            &t.throttle_variable,
            &t.trigger_variable,
            &t.joystick_weight_variable,
            &t.joystick_prop_variable,
            &t.joystick_yaw_variable,
            &t.buffet_intensity_variable,
            &t.buffet_speed_variable,
        ]) {
            if g.variable(v).is_none() {
                return Err(fail(format!("template variable {v} is not a graph variable")));
            }
        }
        Ok(())
    }

    /// The activities a state starts, walked without recursion: none may start itself through any chain, none may nest
    /// deeper than `CHAIR_MAX_NESTING`. (Every named activity exists: checked before.)
    fn check_nesting(&self, s: &ChairState) -> Result<(), String> {
        // how deep the activities under each one already walked go (so a shared child is walked once)
        let mut height: BTreeMap<&str, usize> = BTreeMap::new();
        for root in &s.activities {
            let mut path: Vec<(&str, usize)> = vec![(root.as_str(), 0)];
            while let Some(&(id, next)) = path.last() {
                let kids = self.activities.get(id).map(ChairActivity::children).unwrap_or_default();
                let Some(kid) = kids.get(next).map(|k| k.as_str()) else {
                    let h = 1 + kids.iter().filter_map(|k| height.get(k.as_str())).max().copied().unwrap_or(0);
                    height.insert(id, h);
                    path.pop();
                    continue;
                };
                if let Some(top) = path.last_mut() {
                    top.1 += 1;
                }
                if path.iter().any(|(p, _)| *p == kid) {
                    let chain: Vec<&str> = path.iter().map(|(p, _)| *p).chain([kid]).collect();
                    return Err(format!("activity {kid} starts itself ({})", chain.join(" -> ")));
                }
                if path.len() + height.get(kid).copied().unwrap_or(1) > CHAIR_MAX_NESTING {
                    return Err(format!("activity {root} nests activities deeper than {CHAIR_MAX_NESTING}"));
                }
                if !height.contains_key(kid) {
                    path.push((kid, 0));
                }
            }
        }
        Ok(())
    }
}

impl ChairActivity {
    /// The activities this one starts in its turn (a group's children, a trigger's clip, a button's three stages).
    pub fn children(&self) -> Vec<&String> {
        match self {
            ChairActivity::AnimationActivity { .. } => vec![],
            ChairActivity::GroupedActivityActivity { children } => children.iter().collect(),
            ChairActivity::TriggeredActivityActivity { activity, .. } => vec![activity],
            ChairActivity::JoystickButtonActivity { in_activity, loop_activity, out_activity, .. } => vec![in_activity, loop_activity, out_activity],
        }
    }
}

fn check_param(p: &ChairParam, g: &ChairGraph) -> Result<(), String> {
    if let Some(v) = &p.variable {
        if g.variable(v).is_none() {
            return Err(format!("no variable {v}"));
        }
        if p.keys.is_empty() {
            return Err(format!("{v}: a curve without keys"));
        }
    }
    if !p.constant.is_finite() {
        return Err("its constant is not finite".into());
    }
    check_keys(&p.keys)
}

fn check_keys(keys: &[ChairKey]) -> Result<(), String> {
    if let Some(k) = keys.iter().find(|k| k.interp > 5) {
        return Err(format!("a key's interpolation {} is unknown", k.interp));
    }
    if keys.iter().any(|k| ![k.x, k.y, k.a, k.b].iter().all(|v| v.is_finite())) {
        return Err("a key's number is not finite".into());
    }
    if keys.windows(2).any(|w| !(w[1].x >= w[0].x)) {
        return Err("key positions don't increase".into());
    }
    Ok(())
}

/// Reads `chair/graph.json`, refusing what `ChairGraph::check` refuses.
pub fn parse_chair_graph(bytes: &[u8]) -> Result<ChairGraph, ShipDataError> {
    let g: ChairGraph = serde_json::from_slice(bytes).map_err(|e| bad(CHAIR_GRAPH_FILE, e.to_string()))?;
    g.check()?;
    Ok(g)
}

/// Writes `chair/graph.json` (pretty-printed, LF line endings).
pub fn write_chair_graph(g: &ChairGraph) -> Vec<u8> {
    let mut b = serde_json::to_vec_pretty(g).expect("the graph is plain data");
    b.push(b'\n');
    b
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

    /// The animator's fields from a slot dump laid out as the ship's red beacon's (point) and headlights' (spot) are: the
    /// pattern, cycle, fades, exponent, bool list and mode at their offsets; a word the dump leaves out is zero.
    #[test]
    fn animator_from_the_dump() {
        let f = |x: f32| x.to_bits() as u64;
        let point = serde_json::json!([
            {"at": 136, "u32": [f(1.0), 0]},
            {"at": 144, "u32": [1, f(50.0)]},
            {"at": 152, "u32": [f(50.0), f(60.0)]},
            {"at": 160, "u32": [f(1.0), 0]},
            {"at": 176, "u32": [f(0.5), 0]},
            {"at": 184, "str": "f25n1f25"},
            {"at": 192, "u32": [f(2.0), f(1.0)]},
            {"at": 200, "u32": [f(0.02), f(0.1)]},
            {"at": 208, "u32": [f(8.0), 0]},
            {"at": 232, "block": [{"at": 0, "str": "HeatRadiator_VisualActiveDissipation"}], "ptr": [4, 1]},
            {"at": 240, "u32": [1, 0]},
            {"at": 248, "block": [{"at": 0, "str": "Warning_DangerProximity"}, {"at": 8, "str": "Warning_Overheating"}], "ptr": [5, 2]},
            {"at": 256, "u32": [2, 1]},
            // (+0x125, the bool mode: the second byte of the word at +0x124.)
            {"at": 288, "u32": [0, 0x0100]},
        ]);
        let a = LightAnimator::from_raw(point.as_array().unwrap(), false).unwrap();
        assert!(!a.hard && a.jitter_spread == 1 && a.jitter_below == 50.0 && a.jitter_above == 50.0 && a.jitter_rate_hz == 60.0);
        assert_eq!((a.jitter_chance, a.blink_rate, a.blink_duty), (Some(1.0), 0.0, 0.5));
        assert_eq!(a.pattern.as_deref(), Some("f25n1f25"));
        assert_eq!((a.pattern_cycle_s, a.hold_s, a.fade_in_s as f32, a.fade_out_s as f32, a.exponent), (2.0, 1.0, 0.02, 0.1, 8.0));
        assert_eq!(a.float_params, vec!["HeatRadiator_VisualActiveDissipation".to_string()]);
        assert_eq!(a.bool_params, vec!["Warning_DangerProximity".to_string(), "Warning_Overheating".to_string()]);
        assert_eq!((a.bool_mode, a.colour_param.as_deref()), (1, None));
        let gui = serde_json::json!([{"at": 288, "str": "OverrideColour_Right"}]);
        let g = LightAnimator::from_raw(gui.as_array().unwrap(), false).unwrap();
        assert_eq!((g.bool_mode, g.colour_param.as_deref()), (0, Some("OverrideColour_Right")));
        let spot = serde_json::json!([
            {"at": 192, "u32": [0, 1]},
            {"at": 224, "u32": [0, f(0.5)]},
            {"at": 240, "u32": [f(1.0), 0]},
            {"at": 248, "u32": [f(0.1), f(0.4)]},
            {"at": 256, "u32": [f(1.0), 0]},
            {"at": 280, "block": [{"at": 0, "str": "Option_HeadlightsEnabled"}], "ptr": [4, 1]},
            {"at": 288, "u32": [1, 0]},
            {"at": 392, "u32": [f(0.8), 0x0101_0103]},
        ]);
        assert_eq!(spot_inner(spot.as_array().unwrap()), (0.0, true), "+0x8c 0 degrees, +0x18f set");
        let s = LightAnimator::from_raw(spot.as_array().unwrap(), true).unwrap();
        assert_eq!((s.jitter_chance, s.blink_duty, s.pattern_cycle_s, s.fade_in_s as f32, s.fade_out_s as f32, s.exponent), (None, 0.5, 1.0, 0.1, 0.4, 1.0));
        assert_eq!(s.bool_params, vec!["Option_HeadlightsEnabled".to_string()]);
        assert!(s.float_params.is_empty() && s.colour_param.is_none() && s.bool_mode == 0);
        // A count without its names is refused.
        let broken = serde_json::json!([{"at": 288, "u32": [2, 0]}]);
        assert!(LightAnimator::from_raw(broken.as_array().unwrap(), true).is_err());
    }

    /// FUN_1435dae40's pattern rules.
    #[test]
    fn blink_patterns() {
        let red = parse_pattern("f25n1f25").unwrap();
        assert_eq!(red.len(), 51);
        assert_eq!(red.iter().position(|&s| s), Some(25));
        assert_eq!(red.iter().filter(|&&s| s).count(), 1);
        let blue = parse_pattern("f25n1f1n1f25").unwrap();
        assert_eq!((blue.len(), blue[25], blue[26], blue[27]), (53, true, false, true));
        assert_eq!(parse_pattern("N 2 f\t1").unwrap(), vec![true, true, false]);
        assert_eq!(parse_pattern("n0f2n").unwrap(), vec![false, false], "a 0 count adds nothing, a last letter without one too");
        assert!(parse_pattern("").unwrap().is_empty());
        assert!(parse_pattern("x3").is_err() && parse_pattern("nf2").is_err());
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
            inner_deg: None,
            inner_from_cone: None,
            animator: None,
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

#[cfg(test)]
mod chair_tests {
    //! The chair's files, from `tests/data/elite/chair/` (the shipped clips and graph as make_chair_vectors.py trims them).
    use super::*;

    fn data(name: &str) -> Vec<u8> {
        let path = format!("{}/tests/data/elite/chair/{name}", env!("CARGO_MANIFEST_DIR"));
        miniz_oxide::inflate::decompress_to_vec_zlib(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect(name)
    }

    #[test]
    fn chair_clips_round_trip() {
        let c = parse_chair_clips(&data("clips.json.z"), &data("clips.bin.z")).unwrap();
        assert!(c.clip("ChairMale_RightStickMove").is_some_and(|k| k.additive && k.frames == 41));
        let (json, bin) = write_chair_clips(&c);
        assert_eq!(bin, data("clips.bin.z"));
        assert_eq!(parse_chair_clips(&json, &bin).unwrap(), c);
    }

    #[test]
    fn chair_clips_refusals_name_the_fault() {
        let (json, bin) = (data("clips.json.z"), data("clips.bin.z"));
        let c = parse_chair_clips(&json, &bin).unwrap();
        let fails = |c: &ChairClips, bin: &[u8], what: &str| {
            let e = parse_chair_clips(&write_chair_clips(c).0, bin).unwrap_err().to_string();
            assert!((e.starts_with("chair/clips.json: ") || e.starts_with("chair/clips.bin: ")) && e.contains(what), "{e}");
        };
        let last = c.clips.iter().max_by_key(|k| k.pos_keys_at).unwrap().name.clone();
        fails(&c, &bin[..bin.len() - 8], &format!("chair/clips.bin: clip {last}: its position keys"));
        fails(&c, &bin[..bin.len() - 1], "chair/clips.bin: ");
        let mut d = c.clone();
        d.clips[0].rot_keys_at += 2;
        fails(&d, &bin, "rotation keys");
        // offsets and sizes whose sum or product overflows are refused, never wrapped
        let mut d = c.clone();
        d.clips[0].rot_keys_at = u64::MAX - 7;
        fails(&d, &bin, &format!("chair/clips.json: clip {}: its rotation keys' offset {} and size overflow", d.clips[0].name, u64::MAX - 7));
        let mut d = c.clone();
        d.clips[0].frames = u32::MAX;
        fails(&d, &bin, "chair/clips.bin: clip chairmale_rightstickmove: its rotation keys (68719476720 bytes at 0)");
        // (JSON holds no infinity; a table built in code is checked the same way)
        let mut d = c.clone();
        d.clips[0].pos_scale[0][1] = f32::INFINITY;
        assert!(d.check().unwrap_err().to_string().contains("not finite"));
        let mut d = c.clone();
        d.clips[1].additive = !d.clips[1].additive;
        fails(&d, &bin, "additive");
        let mut d = c.clone();
        d.clips[0].pos_channels[0].table = Some(9);
        fails(&d, &bin, "no table entry");
        let mut d = c.clone();
        d.clips[2].name = d.clips[3].name.to_uppercase();
        fails(&d, &bin, "appears twice");
        let mut d = c.clone();
        d.format = 2;
        fails(&d, &bin, "format 2");
        assert!(parse_chair_clips(b"{}", &bin).unwrap_err().to_string().starts_with("chair/clips.json: "));
    }

    #[test]
    fn chair_graph_round_trip() {
        let g = parse_chair_graph(&data("graph.json.z")).unwrap();
        assert_eq!(parse_chair_graph(&write_chair_graph(&g)).unwrap(), g);
        assert_eq!((g.output_state("Mode_Ship"), g.output_state("empty_ship"), g.output_state("Uninitialised")), (Some(8), Some(1), Some(40)));
        assert_eq!(g.output_state("Action_GunnerAlertMode"), None);
        assert_eq!(g.templates.buffet_thresholds, [1e-5; 3]);
        let text = String::from_utf8(write_chair_graph(&g)).unwrap();
        for k in ["\"type\": \"JoystickButtonActivity\"", "\"in\": ", "\"const\": 1.0", "\"occupied_loop\": \"Mode_Ship\""] {
            assert!(text.contains(k), "{k}");
        }
    }

    #[test]
    fn chair_graph_refusals_name_the_fault() {
        let g = parse_chair_graph(&data("graph.json.z")).unwrap();
        let fails = |g: &ChairGraph, what: &str| {
            let e = parse_chair_graph(&write_chair_graph(g)).unwrap_err().to_string();
            assert!(e.starts_with("chair/graph.json: ") && e.contains(what), "{e}");
        };
        let mut d = g.clone();
        d.states[0].activities.push("9:9".into());
        fails(&d, "no activity 9:9");
        let mut d = g.clone();
        d.states[0].transitions[0].targets.push(99);
        fails(&d, "state 99");
        let mut d = g.clone();
        d.root_outputs[0].key += 1;
        fails(&d, "root output");
        let mut d = g.clone();
        d.templates.occupied_loop = "Mode_Boat".into();
        fails(&d, "Mode_Boat");
        let mut d = g.clone();
        if let Some(ChairActivity::AnimationActivity { weight, .. }) = d.activities.values_mut().find(|a| matches!(a, ChairActivity::AnimationActivity { weight, .. } if weight.variable.is_some())) {
            weight.keys[0].interp = 9;
        }
        fails(&d, "interpolation 9");
        let mut d = g.clone();
        d.templates.buffet_events.pop();
        fails(&d, "5 buffet events");
        let mut d = g.clone();
        d.default_block = "0:0".into();
        fails(&d, "no default block");
        // a group that starts itself, through another group: the state and the chain are named
        let mut d = g.clone();
        let gid = d.activities.iter().find(|(_, a)| matches!(a, ChairActivity::GroupedActivityActivity { .. })).map(|(k, _)| k.clone()).unwrap();
        d.activities.insert("9:1".into(), ChairActivity::GroupedActivityActivity { children: vec![gid.clone()] });
        if let Some(ChairActivity::GroupedActivityActivity { children }) = d.activities.get_mut(&gid) {
            children.push("9:1".into());
        }
        let owner = d.states.iter().find(|s| s.activities.contains(&gid)).unwrap().id;
        fails(&d, &format!("state {owner}: activity {gid} starts itself ({gid} -> 9:1 -> {gid})"));
        let mut d = g.clone();
        d.transition_blocks.values_mut().for_each(|b| b.fade_time_s = f32::INFINITY);
        assert!(d.check().unwrap_err().to_string().contains("not finite"));
    }
}
