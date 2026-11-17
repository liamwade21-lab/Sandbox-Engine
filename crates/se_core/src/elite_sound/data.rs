//! `sounds.json`'s shape: the Sidewinder's shipped Wwise events and everything they reach, trimmed from the audio export's
//! `events.json` with no value changed (written by `copy_sidewinder`'s `sound.rs`). Fields keep Wwise's names and units as
//! `events.json` has them: dB, cents, 0-100 filter values, ms for action and container times, s for `InitialDelay`.
//!
//! Every struct refuses fields it does not know, and every behaviour a field selects is an enum of the values the
//! `Mixer` implements, so a file that needs something the mixer lacks is an `Err` at load ([`parse_sounds`]), never ignored.

use super::SoundError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

/// The version `sounds.json` carries.
pub const FORMAT: &str = "sidewinder-sounds 1";

/// A Wwise id (FNV-1 hash or object id), written as 8 lower-case hex digits as `events.json` writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Id(pub u32);

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{:08x}", self.0))
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Id, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() != 8 {
            return Err(serde::de::Error::custom(format!("id {s:?} is not 8 hex digits")));
        }
        u32::from_str_radix(&s, 16).map(Id).map_err(|_| serde::de::Error::custom(format!("id {s:?} is not hex")))
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}

/// An id with the name that hashes to it, if one is known.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ref {
    pub id: Id,
    pub name: Option<String>,
}

/// A property value: most are numbers (kept as the integer or float `events.json` has), `AttenuationID` is a reference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropValue {
    Int(i64),
    Num(f64),
    Ref(Ref),
}

impl PropValue {
    pub fn num(&self) -> Option<f64> {
        match self {
            PropValue::Int(i) => Some(*i as f64),
            PropValue::Num(f) => Some(*f),
            PropValue::Ref(_) => None,
        }
    }
}

pub type Props = BTreeMap<String, PropValue>;

/// A randomiser ("ranged modifier"): a value drawn in `min..=max` each time the object plays, added to the property.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

pub type Ranged = BTreeMap<String, Range>;

/// Wwise's `AkCurveInterpolation` (the shape from one curve point to the next, and of fades).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interp {
    Log3,
    Sine,
    Log1,
    InvSCurve,
    Linear,
    SCurve,
    Exp1,
    SineRecip,
    Exp3,
    Constant,
}

/// A curve's y scaling (`AkCurveScaling`): as is, dB stored as linear gain - 1, or `Log`: the value is 10^y. Log is met
/// only on effect parameters that are frequencies (the Parametric EQ's bands: y 2 to 4.3 = 100 Hz to 20 kHz; wwiser's
/// `_ScalingFromLog` reads 10^(y/20), which would put every band at 1-2 Hz).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scaling {
    None,
    #[serde(rename = "dB")]
    DB,
    Log,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub interp: Interp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RtpcType {
    GameParameter,
    Modulator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accumulation {
    Additive,
    Exclusive,
    Boolean,
}

/// A game-parameter (or modulator) curve driving one parameter of an object, bus or attenuation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rtpc {
    pub game_parameter: Ref,
    pub rtpc_type: RtpcType,
    pub accumulation: Accumulation,
    pub param_id: u32,
    pub param: String,
    pub curve_id: Id,
    pub scaling: Scaling,
    pub points: Vec<Point>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateSync {
    Immediate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateProp {
    pub param_id: u32,
    pub param: String,
    pub accumulation: Accumulation,
    pub in_db: bool,
}

/// One state of a group with what it adds to the object's properties (the State object's deltas, inline).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateUse {
    pub state: Ref,
    pub state_object: Id,
    pub deltas: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateGroupUse {
    pub group: Ref,
    pub sync: StateSync,
    pub states: Vec<StateUse>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StateBlock {
    pub props: Vec<StateProp>,
    pub groups: Vec<StateGroupUse>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Positioning {
    pub override_parent: bool,
    pub listener_relative_routing: bool,
    pub panner_type: String,
    pub position_3d_type: String,
    pub bits: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatialization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_attenuation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_emitter_pos_and_orient: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_listener_orient: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_diffraction: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bits_3d: Option<String>,
    /// Path automation (3D position type other than Emitter): kept, not built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BelowThreshold {
    ContinueToPlay,
    KillVoice,
    SetAsVirtualVoice,
    KillIfOneShotElseVirtual,
}

/// Advanced settings: playback limit and virtual voice behaviour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adv {
    pub kill_newest: bool,
    pub use_virtual_behavior: bool,
    pub ignore_parent_max_instances: bool,
    pub virtual_voice_opt_override_parent: bool,
    pub virtual_queue_behavior: u32,
    pub max_instances: u32,
    pub below_threshold_behavior: BelowThreshold,
    pub override_hdr_envelope: bool,
    pub override_analysis: bool,
    pub normalize_loudness: bool,
    pub enable_envelope: bool,
}

/// One effect slot of a node or bus (wwiser's `FXChunk`): its index (0-3) and the effect in it (an effect share set or a
/// custom effect: `SoundFile::effects` holds the ones the mixer runs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FxSlot {
    pub index: u32,
    pub fx_id: Option<Ref>,
    pub is_share_set: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_rendered: Option<bool>,
}

/// A node's or bus's effect slots: `bypass_bits` bit n bypasses slot n, bit 4 all of them (wwiser's `bitsFXBypass`); a
/// node's `override_parent` (Wwise's "Override parent" on the Effects tab): the effects in force on a sound are those of the
/// nearest node up its chain that overrides, else the top node's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FxChain {
    pub count: u32,
    pub fx: Vec<FxSlot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass_bits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_parent: Option<bool>,
}

impl FxChain {
    /// Slot `slot` bypassed by the bank's bits (its own bit or the bit for all).
    pub fn bypassed(&self, slot: u32) -> bool {
        let b = self.bypass_bits.unwrap_or(0);
        b & (1 << slot.min(3)) != 0 || b & 0x10 != 0
    }
}

/// The parameters every actor-mixer hierarchy node carries (`NodeBaseParams`). `aux` is kept but not built; of the effects
/// in `fx` the Parametric EQ is run ([`Effect`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub fx: FxChain,
    pub override_attachment_params: bool,
    pub override_bus: Option<Ref>,
    pub parent: Option<Ref>,
    pub priority_override_parent: bool,
    pub priority_apply_dist_factor: bool,
    pub props: Props,
    pub ranged: Ranged,
    pub positioning: Positioning,
    pub aux: Value,
    pub adv: Adv,
    pub states: StateBlock,
    pub rtpc: Vec<Rtpc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub plugin: String,
    pub plugin_type: u32,
    pub stream_type: String,
    pub source_id: Id,
    pub in_memory_size: u64,
    pub source_bits: String,
    pub is_language_specific: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_params_hex: Option<String>,
    pub node: Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RsMode {
    Random,
    Sequence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RandomMode {
    Normal,
    Shuffle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionMode {
    Disabled,
    CrossFadeAmp,
    CrossFadePower,
    Delay,
    SampleAccurate,
    TriggerRate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaylistItem {
    pub id: Id,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RanSeqCntr {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub loop_count: u32,
    pub loop_infinite: bool,
    pub loop_mod_min: i32,
    pub loop_mod_max: i32,
    pub transition_time_ms: f64,
    pub transition_time_mod: [f64; 2],
    pub avoid_repeat_count: u32,
    pub transition_mode: TransitionMode,
    pub random_mode: RandomMode,
    pub mode: RsMode,
    pub using_weight: bool,
    pub reset_playlist_each_play: bool,
    pub restart_backward: bool,
    pub continuous: bool,
    pub global_: bool,
    pub children: Vec<Id>,
    pub playlist: Vec<PlaylistItem>,
    pub node: Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupType {
    Switch,
    State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnSwitch {
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchEntry {
    pub value: Ref,
    pub nodes: Vec<Id>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchParam {
    pub node: Id,
    pub first_only: bool,
    pub continue_playback: bool,
    pub on_switch: OnSwitch,
    pub fade_out_ms: i64,
    pub fade_in_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchCntr {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub group_type: GroupType,
    pub group: Ref,
    pub default: Option<Ref>,
    pub continuous_validation: bool,
    pub children: Vec<Id>,
    pub switches: Vec<SwitchEntry>,
    pub switch_params: Vec<SwitchParam>,
    pub node: Node,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Association {
    pub child: Id,
    pub crossfade: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub layer: Id,
    pub rtpc: Vec<Rtpc>,
    pub crossfade_game_parameter: Option<Ref>,
    pub crossfade_rtpc_type: RtpcType,
    pub associations: Vec<Association>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerCntr {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub children: Vec<Id>,
    pub layers: Vec<Layer>,
    pub continuous_validation: bool,
    pub node: Node,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorMixer {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub children: Vec<Id>,
    pub node: Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    Play,
    PlayEvent,
    Stop,
    SetState,
    SetSwitch,
    SetGameParameter,
    SetVolume,
    ResetVolume,
    SetPitch,
    ResetPitch,
    Seek,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "game object, this object")]
    GameObject,
    #[serde(rename = "global, this object")]
    Global,
    #[serde(rename = "global, all")]
    GlobalAll,
    #[serde(rename = "game object (no target)")]
    GameObjectNoTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueMeaning {
    #[serde(rename = "Independent (absolute)")]
    Absolute,
    #[serde(rename = "Offset (relative)")]
    Offset,
}

/// A value block of the Set / Reset actions: `base` plus a random draw in `min..=max`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueRange {
    pub base: f64,
    pub min: f64,
    pub max: f64,
}

/// What an action's kind adds (each field present only for the kinds that have it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ActionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_curve: Option<Interp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exceptions: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<ValueRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_meaning: Option<ValueMeaning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass_transition: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_group: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch_group: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_to_dynamic_sequence: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_to_state_transitions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seek: Option<ValueRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seek_relative_to_duration: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap_to_nearest_marker: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    #[serde(rename = "type")]
    pub kind_tag: String,
    pub id: Id,
    pub name: Option<String>,
    pub action_type: String,
    pub kind: ActionKind,
    pub scope: Scope,
    pub target: Id,
    pub target_is_bus: bool,
    pub params: ActionParams,
    pub target_name: Option<String>,
    pub props: Props,
    pub ranged: Ranged,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Duck {
    pub bus: Ref,
    pub duck_volume_db: f64,
    pub fade_out_ms: i64,
    pub fade_in_ms: i64,
    pub fade_curve: Interp,
    pub target_prop: u32,
}

/// An audio bus (or aux bus met as a parent). `mixer_plugin` and `aux` are kept, not built; of the effects in `fx` the
/// Parametric EQ is run ([`Effect`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub parent_bus: Option<Ref>,
    pub positioning: Positioning,
    pub aux: Value,
    pub kill_newest: bool,
    pub use_virtual_behavior: bool,
    pub max_instances: u32,
    pub channel_config: String,
    pub is_hdr_bus: bool,
    pub hdr_release_exponential: bool,
    pub recovery_time_ms: i64,
    pub max_duck_volume_db: f64,
    pub ducks: Vec<Duck>,
    pub fx: FxChain,
    pub mixer_plugin: Value,
    pub override_attachment_params: bool,
    pub props: Props,
    pub rtpc: Vec<Rtpc>,
    pub states: StateBlock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_shareset: Option<Ref>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cone {
    pub inside_deg: f64,
    pub outside_deg: f64,
    pub outside_volume_db: f64,
    pub lpf: f64,
    pub hpf: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttCurve {
    pub scaling: Scaling,
    pub points: Vec<Point>,
}

/// Distance attenuation: `curve_to_use` maps each use (VolumeDry, LowPassFilter ...) to an index into `curves` (-1 none).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attenuation {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub cone_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cone: Option<Cone>,
    pub curve_to_use: BTreeMap<String, i32>,
    pub curves: Vec<AttCurve>,
    pub rtpc: Vec<Rtpc>,
}

/// An object of `sounds.json`, by its `type`.
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Sound(Sound),
    RanSeqCntr(RanSeqCntr),
    SwitchCntr(SwitchCntr),
    LayerCntr(LayerCntr),
    ActorMixer(ActorMixer),
    Action(Action),
    Bus(Bus),
    Attenuation(Attenuation),
}

impl Serialize for Object {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Object::Sound(o) => o.serialize(s),
            Object::RanSeqCntr(o) => o.serialize(s),
            Object::SwitchCntr(o) => o.serialize(s),
            Object::LayerCntr(o) => o.serialize(s),
            Object::ActorMixer(o) => o.serialize(s),
            Object::Action(o) => o.serialize(s),
            Object::Bus(o) => o.serialize(s),
            Object::Attenuation(o) => o.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Object, D::Error> {
        use serde::de::Error;
        let v = Value::deserialize(d)?;
        let t = v.get("type").and_then(Value::as_str).ok_or_else(|| D::Error::custom("object without a type"))?.to_string();
        let r = match t.as_str() {
            "Sound" => serde_json::from_value(v).map(Object::Sound),
            "RanSeqCntr" => serde_json::from_value(v).map(Object::RanSeqCntr),
            "SwitchCntr" => serde_json::from_value(v).map(Object::SwitchCntr),
            "LayerCntr" => serde_json::from_value(v).map(Object::LayerCntr),
            "ActorMixer" => serde_json::from_value(v).map(Object::ActorMixer),
            "Action" => serde_json::from_value(v).map(Object::Action),
            "Bus" | "AuxBus" => serde_json::from_value(v).map(Object::Bus),
            "Attenuation" => serde_json::from_value(v).map(Object::Attenuation),
            other => return Err(D::Error::custom(format!("object type {other} is not one the mixer plays"))),
        };
        r.map_err(|e| D::Error::custom(format!("{t}: {e}")))
    }
}

impl Object {
    /// The actor-mixer hierarchy parameters, for the node types.
    pub fn node(&self) -> Option<&Node> {
        match self {
            Object::Sound(o) => Some(&o.node),
            Object::RanSeqCntr(o) => Some(&o.node),
            Object::SwitchCntr(o) => Some(&o.node),
            Object::LayerCntr(o) => Some(&o.node),
            Object::ActorMixer(o) => Some(&o.node),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Sound(_) => "Sound",
            Object::RanSeqCntr(_) => "RanSeqCntr",
            Object::SwitchCntr(_) => "SwitchCntr",
            Object::LayerCntr(_) => "LayerCntr",
            Object::ActorMixer(_) => "ActorMixer",
            Object::Action(_) => "Action",
            Object::Bus(_) => "Bus",
            Object::Attenuation(_) => "Attenuation",
        }
    }
}

/// An event: its name (none for the unnamed events other events post) and its actions, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub name: Option<String>,
    pub actions: Vec<Id>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ramp {
    None,
    SlewRate,
    FilteringOverTime,
}

/// A game parameter as the Init bank defines it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameParameter {
    pub name: Option<String>,
    pub defined_in_init: bool,
    pub default: f64,
    pub ramp: Ramp,
    pub ramp_up: f64,
    pub ramp_down: f64,
    pub built_in: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateTransition {
    pub from_: Ref,
    pub to: Ref,
    pub time_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateGroup {
    pub name: Option<String>,
    pub default_transition_ms: i64,
    pub transitions: Vec<StateTransition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchPoint {
    pub x: f64,
    /// None: no switch value (id 0 in the bank).
    pub switch: Option<Ref>,
    pub interp: Interp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchGroup {
    pub name: Option<String>,
    pub driven_by_game_parameter: Option<Ref>,
    pub curve: Option<Vec<SwitchPoint>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameSyncs {
    pub source: String,
    pub volume_threshold_db: f64,
    pub max_voices: u32,
    pub game_parameters: BTreeMap<Id, GameParameter>,
    pub state_groups: BTreeMap<Id, StateGroup>,
    pub switch_groups: BTreeMap<Id, SwitchGroup>,
}

/// A shipped recording: its file in the mod's folder and what it decodes to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    pub file: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub samples: u64,
    /// The recording's loop points as the bank gives them (its smpl chunk): first and last frame of the part a looping
    /// voice repeats (the end inclusive, as smpl stores it). Absent when the recording has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_region: Option<[u64; 2]>,
    /// The stream's `akd ` chunk (Wwise's analysis of the recording), its first two floats as stored: the loudness
    /// normalisation gain (a linear factor: it brings the recording's integrated loudness to one target, -23.5 +- 2 LUFS
    /// over 42 recordings measured by `re/elite/work/sound/t5c/akd_vs_lufs.py`) and a second gain (1 or 1.414 on mono
    /// streams, 1 on stereo: its meaning is not proven and the mixer does not apply it). Absent without the chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness_gain: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub akd_gain_2: Option<f64>,
    /// The decoded recording's RMS (linear, full scale 1, every channel's samples together), and its loop region's alone
    /// (measured by `sidewinder_media.py` from the game's decode): what a bus meter reads of it ([`Meter`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_rms: Option<f64>,
}

/// What a Wwise Meter effect measures (wwiser's `CAkMeterFX__AkMeterMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeterMode {
    Peak,
    #[serde(rename = "RMS")]
    Rms,
}

/// Whose game parameter a meter writes (wwiser's `CAkMeterFX__AkMeterScope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeterScope {
    Global,
    GameObject,
}

/// A Wwise Meter effect (plugin 0x00810003) in a bus's effect slot: it measures the bus's signal at that slot and writes
/// the level (dB, clamped to `min_db..max_db`) into a game parameter. Fields as wwiser's `CAkMeterFXParams__SetParamsBlock`
/// names them for a 27-byte block; times in seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    /// The effect share set's id, the bus and its slot (0-3).
    pub fx: Id,
    pub bus: Id,
    pub slot: u32,
    pub game_parameter: Id,
    pub attack: f64,
    pub release: f64,
    pub min_db: f64,
    pub max_db: f64,
    pub hold: f64,
    pub mode: MeterMode,
    pub scope: MeterScope,
    pub apply_downstream_volume: bool,
}

impl Media {
    pub fn duration(&self) -> f64 {
        self.samples as f64 / self.sample_rate.max(1) as f64
    }
}

/// A Parametric EQ band's filter (wwiser's `CAkParameterEQ__AkFilterType`, the band's `eFilterType` 0-6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EqFilter {
    #[serde(rename = "LOWPASS")]
    LowPass,
    #[serde(rename = "HIPASS")]
    HiPass,
    #[serde(rename = "BANDPASS")]
    BandPass,
    #[serde(rename = "NOTCH")]
    Notch,
    #[serde(rename = "LOWSHELF")]
    LowShelf,
    #[serde(rename = "HISHELF")]
    HiShelf,
    #[serde(rename = "PEAKINGEQ")]
    PeakingEq,
}

impl EqFilter {
    /// The filter of a type number (a curve sets the type as a float, which the plug-in truncates to an integer).
    pub fn from_index(i: i64) -> Option<EqFilter> {
        Some(match i {
            0 => EqFilter::LowPass,
            1 => EqFilter::HiPass,
            2 => EqFilter::BandPass,
            3 => EqFilter::Notch,
            4 => EqFilter::LowShelf,
            5 => EqFilter::HiShelf,
            6 => EqFilter::PeakingEq,
            _ => return None,
        })
    }

    /// The filter of a design type of `FUN_144a05110` (the band designer the Parametric EQ reaches through `FUN_144a0f510`'s
    /// map, and the Guitar Distortion calls with its band's own type, `FUN_144a06470`): 0 low shelf, 1 peaking, 2 high
    /// shelf, 3 low-pass, 4 high-pass, 5 band-pass, 6 notch. Its types 7 and 8 are not built (no shipped band uses them).
    pub fn from_design(i: i64) -> Option<EqFilter> {
        Some(match i {
            0 => EqFilter::LowShelf,
            1 => EqFilter::PeakingEq,
            2 => EqFilter::HiShelf,
            3 => EqFilter::LowPass,
            4 => EqFilter::HiPass,
            5 => EqFilter::BandPass,
            6 => EqFilter::Notch,
            _ => return None,
        })
    }
}

/// One band of a Parametric EQ as its parameter block stores it (`EQModuleParams`): gain in dB, frequency in Hz.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqBand {
    pub filter: EqFilter,
    pub gain_db: f64,
    pub frequency: f64,
    pub q: f64,
    pub on: bool,
}

/// A Wwise Parametric EQ's parameter block (plug-in 0x00690003, `CAkParameterEQFXParams`, three bands in bank v135):
/// decoded by the converter from `params_hex`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamEq {
    pub bands: Vec<EqBand>,
    pub output_level_db: f64,
    pub process_lfe: bool,
}

/// A Wwise Guitar Distortion's parameter block (plug-in 0x007e0003, `AkGuitarDistortionFXParams`; Elite's own
/// `SetParamsBlock` at 0x144a04f50 copies it as stored): three pre-EQ and three post-EQ bands (their filter read from the
/// band's type by [`EqFilter::from_design`]), the distortion's type ([`super::filter::DISTORTION_OVERDRIVE`] ...), drive,
/// tone and rectification (0-100), the output level (dB) and the wet share (0-100).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuitarDistortion {
    pub pre_eq: Vec<EqBand>,
    pub post_eq: Vec<EqBand>,
    pub distortion: u32,
    pub drive: f64,
    pub tone: f64,
    pub rectification: f64,
    pub output_level_db: f64,
    pub wet_dry: f64,
}

/// A Wwise Peak Limiter's parameter block (plug-in 0x006e0003, `AkPeakLimiterFXParams`; Elite's `SetParam` at 0x144a12330
/// numbers them 0-6 in this order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeakLimiter {
    pub threshold_db: f64,
    pub ratio: f64,
    pub lookahead_s: f64,
    pub release_s: f64,
    pub output_level_db: f64,
    pub process_lfe: bool,
    pub channel_link: bool,
}

/// An effect the mixer runs: a Wwise Parametric EQ (`eq`), Guitar Distortion (`distortion`) or Peak Limiter (`limiter`),
/// exactly one of them, by `plugin` (the module's docs list the others, kept out). `rtpc` are the game parameter curves on
/// its parameters, by the plug-in's own parameter index (`param_id`, as its `SetParam` in Elite's exe numbers them: the
/// EQ's `0x144a10630`, band n's type, gain, frequency, Q and on/off at 5n..5n+4, the output level at 15, LFE at 16; the
/// Guitar Distortion's `0x144a04b60`, pre-EQ band n at 10n..10n+4, post-EQ band n at 30+10n.., then type 60, drive 61, tone
/// 62, rectification 63, output level 64, wet share 65; the limiter's `0x144a12330`, its fields' order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Id,
    pub name: Option<String>,
    pub plugin: String,
    pub params_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<ParamEq>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distortion: Option<GuitarDistortion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limiter: Option<PeakLimiter>,
    pub rtpc: Vec<Rtpc>,
}

/// A Wwise source plugin met as a Sound (no recording): Silence, Tone Generator or Motion Generator, with its parameters
/// as the audio export decoded them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePlugin {
    pub plugin: String,
    pub name: String,
    pub params: Value,
    pub params_hex: String,
}

/// The whole file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundFile {
    pub format: String,
    pub about: String,
    pub units: BTreeMap<String, String>,
    /// The events the game posts (`SHIP_EVENTS`) and those shipped although their sender is not found (`OPEN_EVENTS`).
    pub ship_events: Vec<String>,
    pub open_events: Vec<String>,
    pub events: BTreeMap<Id, Event>,
    pub objects: BTreeMap<Id, Object>,
    pub game_syncs: GameSyncs,
    pub media: BTreeMap<Id, Media>,
    pub source_plugins: BTreeMap<Id, SourcePlugin>,
    pub names: BTreeMap<Id, String>,
    /// The Meter effects on the buses the shipped sounds go through (each writes a game parameter; absent in older files).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meters: Vec<Meter>,
    /// The effects the mixer runs, in the slots of the nodes and buses kept (absent in older files).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub effects: BTreeMap<Id, Effect>,
}

/// Reads `sounds.json`: the shape, then every reference and every behaviour checked ([`super::check`]).
pub fn parse_sounds(bytes: &[u8]) -> Result<super::SoundBank, SoundError> {
    let file: SoundFile = serde_json::from_slice(bytes).map_err(|e| SoundError::Bad(format!("sounds.json: {e}")))?;
    if file.format != FORMAT {
        return Err(SoundError::Bad(format!("sounds.json: format {:?}, expected {FORMAT:?}", file.format)));
    }
    super::SoundBank::new(file)
}

/// Writes `sounds.json` (compact JSON, the converter's output).
pub fn write_sounds(file: &SoundFile) -> Result<Vec<u8>, SoundError> {
    serde_json::to_vec(file).map_err(|e| SoundError::Bad(format!("sounds.json: {e}")))
}
