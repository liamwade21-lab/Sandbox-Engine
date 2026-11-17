//! The Faulcon DeLacy Sidewinder's sounds as Elite Dangerous's Wwise banks define them: the data (`sounds.json`, [`data`])
//! and a pure runtime ([`Mixer`]) doing what the Wwise sound engine does with it: events posted and game parameters,
//! states and switches set over time in, which recordings play, how loud, at what pitch and filtered how out. No Bevy, no
//! audio device, no files, no clock of its own; deterministic for a seed. The game's side (`sidewinder/sound.rs`) plays
//! what [`Mixer::step`] reports.
//!
//! # What is built (the behaviours the shipped events' data uses)
//! Counts are shipped events using each (with the events they post), of the 446 in `sounds.json` (223 `SHIP_EVENTS`,
//! 223 `OPEN_EVENTS`), counted by `re/elite/work/sound/t5b/survey.py`.
//! - Event actions: Play (317), PlayEvent (45), Stop (126), SetState (39), SetSwitch (11), SetGameParameter (23),
//!   SetVolume (6), ResetVolume (8), SetPitch (7), ResetPitch (6), Seek (4, relative to the duration); delays with their
//!   random range; fade times with the seven fade curves in use (Linear 387, SCurve 30, Sine 9, Exp1 7, Log1 5,
//!   SineRecip 5, Exp3 4); scopes game object (397), global (33), global all (39), game object without target (11);
//!   absolute and offset values with their random range.
//! - Sound (317): loop count (infinite on 390 objects, 57 events), initial delay (property 37, randomiser 37, game
//!   parameter curve 4).
//! - RanSeqCntr (249): random (normal, shuffle) and sequence, step and continuous, avoid-repeat (249), weights, loop
//!   counts, per game object (2) or global playlists, reset at each play or not (2); transitions Disabled, Delay,
//!   TriggerRate, SampleAccurate, CrossFadeAmp, CrossFadePower, their random time modifier (46) and the game parameter
//!   on the transition time (`PlayMechanismSpecialTransitionsValue`, 22).
//! - SwitchCntr (80): on switch groups (66), state groups (48) and game-parameter-driven switches (64); continuous
//!   validation (56) with each child's stop / continue / first-only and fade times. A continuous container playing while
//!   its value assigns nothing it can play waits for the next change instead of ending (Wwise's Continuous play mode plays
//!   the new value's object "as soon as a new switch/state is detected", Wwise 2015.1 User Guide ch. 9; wwiser's
//!   `bIsContinuousValidation`): 56 shipped events reach such a container (`re/elite/work/sound/t5c/waiting_switches.txt`),
//!   the boost's layers among them.
//! - Loudness normalisation (527 shipped sounds, gains -17.4 to +36.1 dB): the recording's `akd ` gain
//!   ([`data::Media::loudness_gain`]) where the node in force (the nearest overriding its parent's analysis, wwiser's
//!   `bOverrideAnalysis`, else the top one) has `bNormalizeLoudness`; make-up gain stays its own additive property.
//! - The `akd ` chunk's second gain ([`data::Media::akd_gain_2`], T5g section 1, C: the leaf at 0x144a78c80 multiplies
//!   every voice's make-up gain by it, whatever its positioning; 1 without the chunk): handed to the player apart
//!   ([`VoiceUpdate::media_gain`]), which applies it where the make-up gain acts, with the voice's level after its
//!   effects (T5d-8; until then it went before them, and the 7 sounds with a Guitar Distortion on their node were driven
//!   3 dB harder).
//! - Where a voice sits and how it reaches the stereo output ([`place`], T5d-5 and T5d-7): each Sound's route
//!   ([`SoundBank::route`]) and placement, a voice's matrix ([`VoiceUpdate::matrix`]).
//! - Wwise Meter effects on buses (10 shipped, [`data::Meter`]): an RMS meter without ballistics (2: both on the drives'
//!   bus, one writing 1e5ba700, the switch that turns the cockpit's background beds off while the drives are loud) writes
//!   its bus's level into its game parameter, worked out from the voices' gains and each recording's RMS.
//! - Low-pass / high-pass cutoffs in Hz ([`filter`]): Elite's own conversion and Butterworth sections, decompiled from its
//!   copy of the sound engine (the value-to-cutoff function, the sections, the 8-step ramp and the 0.1 bypass).
//! - The Wwise Parametric EQ (plug-in 0x00690003; 42 shipped, `SoundFile::effects`), in the effect slots of the node in
//!   force on a sound (the nearest that overrides its parent's effects, else the top node: run per voice) and of every bus
//!   on its way to the master ([`SoundBank::fx_stages`]; a linear effect on a bus gives the bus's mix what it gives each
//!   voice, so the player runs it per voice: [`VoiceUpdate::fx`]). From Elite's own copy of the plug-in (registration
//!   record 0x145ee0248; `SetParam` 0x144a10630 with its parameter indices and clamps, `SetParamsBlock` 0x144a107a0,
//!   `Execute` 0x144a0f9c0, the band designs `FUN_144a0f510` / `FUN_144a05110`: [`filter::eq_coefs`]): the game parameter
//!   curves on its parameters (exclusive or added to the block's value; `Log` scaled frequencies), the output level, the
//!   slot bypass bits and the holder's BypassFX properties, curves and states.
//! - The Wwise Guitar Distortion (plug-in 0x007e0003; 16 shipped: 14 in node slots, run per voice; 2 on buses, the GUI and
//!   Voice buses, whose wet share is 0 wherever the mod plays them, so per voice is exact there too), from Elite's own copy
//!   (record 0x145ee00f8; `SetParam` 0x144a04b60, `SetParamsBlock` 0x144a04f50, `Execute` 0x144a04230: [`filter::Distortion`],
//!   its parameters [`DistortionNow`]): pre-EQ, the Overdrive and Heavy curves (the only types in the data), the
//!   rectifier, post-EQ, the wet / dry mix and output level, the DC blocker, every gain ramped over a 1024-frame buffer.
//! - The master bus's Wwise Peak Limiter (plug-in 0x006e0003): its output level is a gain after the limiting (`Execute`
//!   0x144a108d0 runs the limiter, then ramps the buffer by the output gain), so it is in every voice's gain (+1.28 dB at
//!   DynamicRange's default); its threshold (-4 dB) is reported ([`Mixer::master_limiter`]) for the player to count frames
//!   over it. The limiting itself is not built: the mix's peaks stay under the threshold (the count stays 0).
//! - The master bus's last plug-in 0x00BA0003 (T5d-8, [`master`]: an equaliser, a 4-band compressor, the channel gains and
//!   a limiter with 14 dB of make-up, each ported from Elite's own copy): handed to the player ([`Mixer::master_suite`]),
//!   which runs it on the summed output after the voices.
//! - The Wwise Compressor (plug-in 0x006c0003; 20 shipped, all on buses) on ONE bus, [`SUBMIX_BUS`] (T5d-9,
//!   [`compressor`]: Elite's own copy, its linked detector, static curve, attack and release, output gain): that bus's voices
//!   are handed to the player as a 12-channel sub-mix ([`VoiceUpdate::bus`], [`Mixer::bus_now`], [`SoundBank::submix`]),
//!   which runs the compressor on their sum and what comes after it on the sum folded to stereo.
//! - The Wwise Tone Generator (source plug-in 0x00660002; 4 shipped Sounds, T5d-8, [`tone`]): its sine and white noise, in
//!   its envelope and fixed modes, from Elite's own copy; the player plays it as it plays a recording.
//! - Virtual voices are reported without a start and built by the player only when heard ([`VoiceStart`]), from where
//!   their queue behaviour puts them.
//! - LayerCntr (71): all children together, blend tracks with their crossfade curves (4) and track curves (7).
//! - Properties down the actor-mixer chain (additive: Volume, MakeUpGain, Pitch, LPF, HPF, InitialDelay, OutputBus*) and
//!   up the bus chain (Volume, BusVolume, Pitch, LPF, HPF, OutputBus*), randomisers (Volume 51, Pitch 86, LPF 21, HPF 1,
//!   InitialDelay 37).
//! - Game parameter curves (Volume 307, Pitch 217, LPF 214, HPF 150, Priority 134, OutputBusVolume / LPF 128,
//!   MaxNumInstances 3, InitialDelay 4; on buses Volume / BusVolume 312, LPF 223, OutputBus* 260) with every
//!   interpolation shape (all ten in use) and the dB scaling ([`curve`]); game parameter defaults, SlewRate (312) and
//!   FilteringOverTime (313) ramps, the built-in Distance parameter (132).
//! - States (333): their deltas on nodes and buses, transition times (per pair and default).
//! - Distance attenuation (269): VolumeDry, LowPassFilter (134), HighPassFilter (29) curves. A voice on an emitter the game
//!   has placed whose positioning in force is 3D on the emitter's position gets its [`Placement`] ([`place`]: those
//!   curves, the Spread curve and whether it is panned) and its player applies them and the pan from the place it is
//!   given each frame (T5d-5); the mixer applies them itself to every other voice (at distance 0 unless placed).
//! - Bus ducking (45); playback limits with priority and its distance factor (node limits 130, bus limits 21), each
//!   killing or, where `use_virtual_behavior` is set (35 limited nodes), making the voices beyond it virtual, and
//!   `ignore_parent_max_instances` (what plays under such a node does not count against the limits above it); virtual
//!   voice behaviour below the volume threshold (all four behaviours, the three queue behaviours).
//! - Loop regions: a looping recording with an intro (11 of the shipped ones) reports the region it repeats
//!   ([`VoiceStart::loop_region`], from `Media::loop_region`, the bank's smpl loop points).
//! - Source plugins: Wwise Silence (10; timing only, no update), Motion Generator (21; a `Rumble` output), Tone
//!   Generator (2; a `Synth` output carrying the source id: [`tone`] makes its samples, the player plays them).
//! - Loads refuse what would never end: an object holding itself (any cycle of children or switch nodes) and an event
//!   posting itself without a delay (any cycle); every walk is iterative or bounded (64 levels).
//! - Time: a step that is not finite or not positive counts as 0; a step's first second is run event by event and the
//!   rest, up to an hour, at once ([`Mixer::step`]); setters ignore values that are not finite.
//!
//! # Not built (kept in the data, never refused, listed with the shipped events they touch)
//! - The other effects (`re/elite/work/sound/t5d2/bus_fx.py` lists every one on the shipped sounds' ways with its
//!   parameters, `effects.json` holds them): on buses the Wwise Compressor but [`SUBMIX_BUS`]'s (shipped, never run: by
//!   `no_bus_compressor_would_act` none acts at the mix's levels), the Peak Limiter's limiting, the Meters with
//!   ballistics, the DTS 3D-headphone plug-ins (Frontier's company id: 0x03E80C63 `DtsI3daEffectPlugin` in 3 bus slots,
//!   0x03E90C66 `DtsI3daPannerPlugin` as 11 buses' mixer plug-in; the user's Elite has `hp3d` disabled); on nodes
//!   Tremolo (31) and Flanger (5); on the aux paths reverbs and delays. Tremolo and Flanger modulate in time by LFOs. Aux sends (user
//!   291, game 183, their curves 137) and the attenuation's wet curves (269), HDR (every bus carries HDR settings: 317;
//!   HDR envelope 24).
//! - Panning and spatialisation beyond [`place`]'s (Wwise's 2D and 3D panners into a stereo or 7.1.4 route, the spread
//!   and focus curves, the down-mix; the nodes' CenterPCT, PAN_LR and PAN_FR values are read, T5d-7): curves and state
//!   deltas on those three, positioning-type blend curves (10), a 2D pan curve (1), path automation (13: such a voice pans
//!   as one at the listener), held emitter / listener orientation (7), cone attenuation (9; no emitter orientation is
//!   known), the attenuations' own game parameter curves, the buses' own pan values (their connections are direct
//!   assignment).
//! - Modulators: LFO (42) and envelope (10) modulator curves contribute nothing (their output formulas are not public,
//!   and `events.json` names their properties with the node table's names).
//! - The global voice cap (256).
//! - Meters other than RMS without ballistics (8 of the 10 shipped: 5 Peak, 3 RMS with attack / release / hold, whose
//!   formulas are not public): kept, their game parameters stay where the game or their default leaves them. A built
//!   meter's reading counts the EQs' output levels before its slot but not their bands (it reads each recording's RMS: the
//!   EQ in the slot before both built ones, on the drives' bus, has every band switched off by its curves at the
//!   defaults of its game parameters 782e5048 and 7eefbbc4, which the game never sets).
//! - A blend container's continuous validation (true on 7 of the 138, 3 shipped events): a child whose crossfade falls
//!   to 0 plays on silently instead of being stopped and started again when the parameter comes back. With it false (the
//!   other 131) the children likewise play at their crossfade gain.
//!
//! Choices the data and the public sources leave open, made here and named: FilteringOverTime is a first-order low pass
//! whose time constant is the stored time; `PlayMechanismSpecialTransitionsValue` adds seconds to the transition time;
//! playback limits count globally (the limit's scope bit is not in `events.json`); blend track crossfade values are
//! linear gains; crossfade transitions start the next item that long before a recording's end (only where the item is
//! a Sound of known length; otherwise at its end); LPF / HPF contributions add up, clamped to 0-100; trigger-rate
//! intervals are at least 10 ms; a state change in the middle of a transition starts from the values it has then; a
//! waiting switch container whose child ran out on its own under a value that assigns something is done (else every
//! one-shot posted would leave one waiting); a meter reads the voices' mean RMS by their gains summed as powers (Wwise
//! measures the signal itself, buffer by buffer), the bus's voice volume before its slot and its bus volume, ducking and
//! state deltas after; the updates' cutoffs are given at 48 kHz ([`OUTPUT_RATE`]).
//!
//! Not verified against an independent source (the review's list): a fade-out runs the action's curve from 1 to 0
//! (`AkInterpolation` from the start value to the target, as for fades in); FilteringOverTime's time constant; the
//! limits' scope (global); ducking's `max_duck_volume_db` read as the floor of the sum of a bus's ducks; a blend
//! container played as a whole (no step mode: all children start together).

pub mod compressor;
pub mod curve;
pub mod data;
pub mod filter;
pub mod master;
mod mixer;
pub mod place;
pub mod tone;

#[cfg(test)]
mod place_vectors;
#[cfg(test)]
mod tests;

pub use data::{parse_sounds, write_sounds, Id, Object, SoundFile};
pub use compressor::CompressorNow;
pub use mixer::{BusNow, BusShare, DistortionNow, EqNow, FxNow, LimiterNow, Mixer, Output, VoiceStart, VoiceUpdate, DETAIL_S, MAX_STEP_S, MIN_TRIGGER_S, OUTPUT_RATE};
pub use place::{Layout, Panner, PlaceNow, Placement, Route};

use data::*;
use std::collections::HashMap;

/// What went wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundError {
    /// `sounds.json` is damaged or needs a behaviour the mixer does not have.
    Bad(String),
    /// An event name no shipped event hashes to.
    UnknownEvent(String),
}

impl std::fmt::Display for SoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SoundError::Bad(s) => write!(f, "{s}"),
            SoundError::UnknownEvent(s) => write!(f, "no sound event {s:?}"),
        }
    }
}

impl std::error::Error for SoundError {}

/// Wwise's id of a name (`AK::SoundEngine::GetIDFromString`): FNV-1 32-bit of the lower-cased name.
pub fn wwise_id(name: &str) -> Id {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h = h.wrapping_mul(16777619) ^ u32::from(b.to_ascii_lowercase());
    }
    Id(h)
}

/// The id an event name posts: Wwise's hash of the name, or, for the banks' unnamed events, the id written as
/// `event_<8 hex>` (how `events.json` names them).
pub fn event_id(name: &str) -> Id {
    match name.strip_prefix("event_").filter(|h| h.len() == 8).and_then(|h| u32::from_str_radix(h, 16).ok()) {
        Some(id) => Id(id),
        None => wwise_id(name),
    }
}

/// Properties the mixer applies (on nodes, buses, in state deltas and game parameter curves).
const BUILT_PARAMS: &[&str] = &[
    "Volume", "MakeUpGain", "BusVolume", "OutputBusVolume", "Pitch", "LPF", "HPF", "OutputBusLPF", "OutputBusHPF", "InitialDelay",
    "Priority", "PriorityDistanceOffset", "MaxNumInstances", "PlayMechanismSpecialTransitionsValue", "Loop", "AttenuationID",
    "BypassFX0", "BypassFX1", "BypassFX2", "BypassFX3", "BypassAllFX",
];

/// Properties kept and listed as not built (see the module's docs).
const NOT_BUILT_PARAMS: &[&str] = &[
    "UserAuxSendVolume0", "UserAuxSendVolume1", "UserAuxSendVolume2", "UserAuxSendVolume3", "GameAuxSendVolume", "ReflectionBusVolume",
    "CenterPCT", "PAN_LR", "PAN_FR", "HDRActiveRange", "HDRBusThreshold", "HDRBusRatio", "HDRBusReleaseTime", "HDRBusGameParamMax",
    "PositioningTypeBlend", "Position_PAN_X_2D", "Position_PAN_Y_2D",
];

/// The plug-ins the mixer runs as effects (`SoundFile::effects`): the Wwise Parametric EQ, Guitar Distortion and Peak
/// Limiter, and the master's last plug-in 0x00BA0003 ([`master`]).
pub const PARAMETRIC_EQ: &str = "00690003";
pub const GUITAR_DISTORTION: &str = "007e0003";
pub const PEAK_LIMITER: &str = "006e0003";
pub const MASTER_SUITE: &str = "00ba0003";
/// The Wwise Compressor ([`compressor`]): run only on [`SUBMIX_BUS`].
pub const COMPRESSOR: &str = "006c0003";
/// The compressor's parameter indices (its `SetParam`, `0x1449f5180`): threshold 0, ratio 1, attack 2, release 3, output
/// level 4, LFE 5, channel link 6.
pub const COMPRESSOR_PARAM_THRESHOLD: u32 = 0;
pub const COMPRESSOR_PARAM_OUTPUT_LEVEL: u32 = 4;
pub const COMPRESSOR_PARAM_LINK: u32 = 6;
/// The one bus the output mixes as a sub-mix of its own (T5d-9): the supercruise layers' 12-channel bus, whose compressor
/// (87c72d00: -35.5 dB, 20:1) is the only one of the buses' that can act at the ship's levels (T5d-7: its input reads
/// about 9 dB over its threshold in stereo; every other compressor stays 8 dB or more under, `no_bus_compressor_would_act`).
/// Its voices are summed in its 12 channels, the compressor runs on that sum, and what comes after it on the way to the
/// master runs on the sum folded to stereo ([`SoundBank::submix`]).
pub const SUBMIX_BUS: Id = Id(0xaa29_9971);
/// The Parametric EQ's parameter indices (its `SetParam` in Elite's exe, `0x144a10630`: band = index / 5).
pub const EQ_PARAM_OUTPUT_LEVEL: u32 = 15;
pub const EQ_PARAM_LFE: u32 = 16;
/// The Guitar Distortion's parameter indices (its `SetParam`, `0x144a04b60`: pre-EQ band n at 10n + 0..4 (type, gain,
/// frequency, Q, on), post-EQ band n at 30 + 10n + 0..4, then these).
pub const GD_PARAM_TYPE: u32 = 60;
pub const GD_PARAM_DRIVE: u32 = 61;
pub const GD_PARAM_TONE: u32 = 62;
pub const GD_PARAM_RECTIFICATION: u32 = 63;
pub const GD_PARAM_OUTPUT_LEVEL: u32 = 64;
pub const GD_PARAM_WET: u32 = 65;
/// The Peak Limiter's parameter indices (its `SetParam`, `0x144a12330`): threshold 0, ratio 1, lookahead 2, release 3, these.
pub const LIMITER_PARAM_THRESHOLD: u32 = 0;
pub const LIMITER_PARAM_OUTPUT_LEVEL: u32 = 4;
pub const LIMITER_PARAM_LINK: u32 = 6;

/// A Guitar Distortion parameter index its `SetParam` knows.
pub fn gd_param_known(p: u32) -> bool {
    (p < 60 && p % 10 < 5) || (GD_PARAM_TYPE..=GD_PARAM_WET).contains(&p)
}

const RANGED_BUILT: &[&str] = &["Volume", "Pitch", "LPF", "HPF", "InitialDelay"];

/// A cycle in a graph (iterative depth-first search), as the ids along it, the first repeated at the end.
fn find_cycle(nodes: impl Iterator<Item = Id>, next: impl Fn(Id) -> Vec<Id>) -> Option<Vec<Id>> {
    // 1 = on the current path, 2 = done.
    let mut mark: HashMap<Id, u8> = HashMap::new();
    for start in nodes {
        if mark.contains_key(&start) {
            continue;
        }
        let mut stack: Vec<(Id, Vec<Id>, usize)> = vec![(start, next(start), 0)];
        mark.insert(start, 1);
        while let Some(top) = stack.last_mut() {
            if top.2 < top.1.len() {
                let n = top.1[top.2];
                top.2 += 1;
                match mark.get(&n) {
                    Some(1) => {
                        let at = stack.iter().position(|s| s.0 == n).unwrap_or(0);
                        let mut c: Vec<Id> = stack[at..].iter().map(|s| s.0).collect();
                        c.push(n);
                        return Some(c);
                    }
                    Some(_) => {}
                    None => {
                        mark.insert(n, 1);
                        let nn = next(n);
                        stack.push((n, nn, 0));
                    }
                }
            } else {
                mark.insert(top.0, 2);
                stack.pop();
            }
        }
    }
    None
}

fn ids_text(ids: &[Id]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(" -> ")
}

/// One effect the mixer runs on a sound's way to the master: the node or bus holding it, its slot, the effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxStage {
    pub holder: Id,
    pub slot: u32,
    pub effect: Id,
    /// On a bus (else in the actor-mixer node whose effects are in force: run per voice).
    pub on_bus: bool,
}

/// The bus mixed as a sub-mix ([`SUBMIX_BUS`]): its compressor's slot and effect, and the effects after the compressor on
/// the way to the master, in signal order (the bus's later slots, then each bus above it): they run on the sub-mix once
/// compressed and folded to stereo, so the voices through it run only the effects before them.
#[derive(Clone, Debug, PartialEq)]
pub struct Submix {
    pub bus: Id,
    pub slot: u32,
    pub effect: Id,
    pub after: Vec<FxStage>,
}

/// `sounds.json` read and checked, with the lookups the mixer needs.
#[derive(Debug)]
pub struct SoundBank {
    pub file: SoundFile,
    /// Each node's actor-mixer chain: itself, then its parents up to the root.
    chains: HashMap<Id, Vec<Id>>,
    /// Each bus's chain: itself, then its parents up to the master.
    bus_chains: HashMap<Id, Vec<Id>>,
    /// Each Sound's effects the mixer runs, in signal order: the slots of the node in force (the nearest up its chain whose
    /// effects override its parent's, else the top node), then each bus from its output bus to the master.
    fx_stages: HashMap<Id, Vec<FxStage>>,
    /// Each Sound's way to the output ([`SoundBank::route`]).
    routes: HashMap<Id, place::Route>,
    /// The sub-mix, when the file holds its bus and the bus's compressor ([`SoundBank::submix`]), and for each Sound mixed
    /// into it how many of its effects run on the voice ([`SoundBank::submix_stages`]).
    submix: Option<Submix>,
    submix_stages: HashMap<Id, usize>,
}

fn bad(s: String) -> SoundError {
    SoundError::Bad(s)
}

impl SoundBank {
    /// Checks every reference and every behaviour ([`parse_sounds`] calls it).
    pub fn new(file: SoundFile) -> Result<SoundBank, SoundError> {
        let mut chains = HashMap::new();
        let mut bus_chains = HashMap::new();
        for (id, o) in &file.objects {
            match o {
                Object::Bus(_) => {
                    let mut c = vec![*id];
                    let mut b = o;
                    while let Object::Bus(bus) = b {
                        let Some(p) = &bus.parent_bus else { break };
                        if c.contains(&p.id) {
                            return Err(bad(format!("bus {id}: its parents loop")));
                        }
                        b = file.objects.get(&p.id).ok_or_else(|| bad(format!("bus {id}: parent bus {} is missing", p.id)))?;
                        if !matches!(b, Object::Bus(_)) {
                            return Err(bad(format!("bus {id}: parent {} is not a bus", p.id)));
                        }
                        c.push(p.id);
                    }
                    bus_chains.insert(*id, c);
                }
                o if o.node().is_some() => {
                    let mut c = vec![*id];
                    let mut n = o.node().unwrap();
                    while let Some(p) = &n.parent {
                        if c.contains(&p.id) {
                            return Err(bad(format!("{id}: its parents loop")));
                        }
                        n = file.objects.get(&p.id).and_then(Object::node).ok_or_else(|| bad(format!("{id}: parent {} is missing", p.id)))?;
                        c.push(p.id);
                    }
                    chains.insert(*id, c);
                }
                _ => {}
            }
        }
        let mut bank = SoundBank { file, chains, bus_chains, fx_stages: HashMap::new(), routes: HashMap::new(), submix: None, submix_stages: HashMap::new() };
        bank.fx_stages = bank.stages();
        bank.routes = bank.routes().map_err(bad)?;
        bank.check()?;
        (bank.submix, bank.submix_stages) = bank.submix_of().map_err(|e| bad(format!("the sub-mix bus {SUBMIX_BUS}: {e}")))?;
        bank.check_playable()?;
        bank.check_cycles()?;
        Ok(bank)
    }

    fn check(&self) -> Result<(), SoundError> {
        let f = &self.file;
        let obj = |id: &Id| f.objects.get(id);
        for name in f.ship_events.iter().chain(&f.open_events) {
            if !f.events.contains_key(&event_id(name)) {
                return Err(bad(format!("event {name} is listed but not in the file")));
            }
        }
        for (eid, e) in &f.events {
            if let Some(n) = &e.name {
                if wwise_id(n) != *eid {
                    return Err(bad(format!("event {eid}: its name {n} hashes to {}", wwise_id(n))));
                }
            }
            for a in &e.actions {
                let Some(Object::Action(act)) = obj(a) else { return Err(bad(format!("event {eid}: action {a} is missing"))) };
                self.check_action(act)?;
            }
        }
        for (id, o) in &f.objects {
            let at = |s: String| bad(format!("{} {id}: {s}", o.type_name()));
            if let Some(n) = o.node() {
                self.check_node(n).map_err(at)?;
            }
            match o {
                Object::Sound(s) => {
                    if s.plugin_type == 2 {
                        let p = f.source_plugins.get(&s.source_id).ok_or_else(|| at(format!("source plugin {} has no parameters", s.source_id)))?;
                        if !matches!(p.name.as_str(), "Wwise Silence" | "Wwise Motion Generator" | "Wwise Tone Generator") {
                            return Err(at(format!("source plugin {} is not one the mixer knows", p.name)));
                        }
                        if p.name == "Wwise Silence" && p.params.get("fDuration").and_then(serde_json::Value::as_f64).is_none() {
                            return Err(at("silence without fDuration".into()));
                        }
                        if p.name == "Wwise Tone Generator" {
                            tone::ToneParams::from_block(&p.params_hex).map_err(|e| at(format!("Tone Generator {}: {e}", s.source_id)))?;
                        }
                    } else if !f.media.contains_key(&s.source_id) {
                        return Err(at(format!("media {} is not shipped", s.source_id)));
                    }
                }
                Object::RanSeqCntr(r) => {
                    if r.restart_backward {
                        return Err(at("restart backward is not built".into()));
                    }
                    if r.loop_mod_min != 0 || r.loop_mod_max != 0 {
                        return Err(at("loop count randomiser is not built".into()));
                    }
                    for p in &r.playlist {
                        if !r.children.contains(&p.id) {
                            return Err(at(format!("playlist item {} is not a child", p.id)));
                        }
                    }
                }
                // A switch value may name a node that is not in the file: one whose bank is not among those at rest (6 such
                // in the export), or the child of a switch container only met as the parent of what plays. It plays
                // nothing, as in Wwise with that bank not loaded; `check_playable` holds everything a Play reaches.
                Object::SwitchCntr(_) => {}
                Object::LayerCntr(l) => {
                    for layer in &l.layers {
                        self.check_rtpcs(&layer.rtpc).map_err(at)?;
                        if layer.crossfade_rtpc_type != RtpcType::GameParameter {
                            return Err(at("a blend track crossfade by a modulator is not built".into()));
                        }
                    }
                }
                Object::ActorMixer(_) => {}
                Object::Bus(b) => {
                    self.check_props(&b.props).map_err(at)?;
                    self.check_rtpcs(&b.rtpc).map_err(at)?;
                    self.check_states(&b.states).map_err(at)?;
                    for d in &b.ducks {
                        if d.target_prop != 0 {
                            return Err(at(format!("ducking property {} is not built (only volume)", d.target_prop)));
                        }
                        // A ducked bus not in the file carries none of the shipped sounds (nothing to duck).
                        if obj(&d.bus.id).is_some_and(|o| !matches!(o, Object::Bus(_))) {
                            return Err(at(format!("ducked {} is not a bus", d.bus.id)));
                        }
                    }
                }
                Object::Attenuation(a) => {
                    self.check_rtpcs(&a.rtpc).map_err(at)?;
                    if !a.rtpc.is_empty() {
                        return Err(at("game parameter curves on an attenuation are not built".into()));
                    }
                    for (k, i) in &a.curve_to_use {
                        if !matches!(k.as_str(), "VolumeDry" | "LowPassFilter" | "HighPassFilter" | "VolumeWetGame" | "VolumeWetUser" | "Spread" | "Focus") {
                            return Err(at(format!("attenuation curve use {k} is not known")));
                        }
                        if *i >= a.curves.len() as i32 {
                            return Err(at(format!("{k} names curve {i}")));
                        }
                    }
                }
                Object::Action(_) => {}
            }
        }
        for (id, e) in &f.effects {
            self.check_effect(e).map_err(|s| bad(format!("effect {id}: {s}")))?;
        }
        for m in &f.meters {
            if !self.bus_chains.contains_key(&m.bus) {
                return Err(bad(format!("meter {}: bus {} is missing", m.fx, m.bus)));
            }
            if !(m.min_db.is_finite() && m.max_db.is_finite()) {
                return Err(bad(format!("meter {}: its range is not finite", m.fx)));
            }
        }
        Ok(())
    }

    /// No object holds itself (container children, switch nodes, any length of cycle) and no event posts itself without a
    /// delay (PlayEvent actions whose delay can be 0, any length of cycle): either would never end.
    fn check_cycles(&self) -> Result<(), SoundError> {
        let f = &self.file;
        let kids = |id: Id| -> Vec<Id> {
            match f.objects.get(&id) {
                Some(Object::RanSeqCntr(r)) => r.children.clone(),
                Some(Object::LayerCntr(l)) => l.children.clone(),
                Some(Object::SwitchCntr(s)) => {
                    let mut k = s.children.clone();
                    k.extend(s.switches.iter().flat_map(|e| e.nodes.iter().copied()).filter(|n| f.objects.contains_key(n)));
                    k
                }
                _ => Vec::new(),
            }
        };
        let posts = |id: Id| -> Vec<Id> {
            f.events
                .get(&id)
                .map(|e| {
                    e.actions
                        .iter()
                        .filter_map(|a| match f.objects.get(a) {
                            Some(Object::Action(act)) if act.kind == ActionKind::PlayEvent => {
                                let least = act.props.get("DelayTime").and_then(PropValue::num).unwrap_or(0.0) + act.ranged.get("DelayTime").map(|r| r.min).unwrap_or(0.0);
                                (least <= 0.0).then_some(act.target)
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        if let Some(c) = find_cycle(f.objects.keys().copied(), kids) {
            return Err(bad(format!("objects hold themselves: {}", ids_text(&c))));
        }
        if let Some(c) = find_cycle(f.events.keys().copied(), posts) {
            return Err(bad(format!("events post themselves without a delay: {}", ids_text(&c))));
        }
        Ok(())
    }

    /// Everything a Play action can reach is in the file: its target and, down the tree, every child. (An object only
    /// stopped, sought or set may list children that nothing plays; they are left out.)
    fn check_playable(&self) -> Result<(), SoundError> {
        let mut todo: Vec<Id> = self
            .file
            .objects
            .values()
            .filter_map(|o| match o {
                Object::Action(a) if a.kind == ActionKind::Play => Some(a.target),
                _ => None,
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            let kids: &[Id] = match self.file.objects.get(&id) {
                Some(Object::RanSeqCntr(r)) => &r.children,
                Some(Object::SwitchCntr(s)) => &s.children,
                Some(Object::LayerCntr(l)) => &l.children,
                Some(Object::Sound(_)) => &[],
                _ => return Err(bad(format!("{id}, which a Play action reaches, is missing or not playable"))),
            };
            todo.extend(kids.iter().copied());
        }
        Ok(())
    }

    /// An effect the mixer runs: a Parametric EQ of three bands, a Guitar Distortion of three and three bands with a curve
    /// the mixer builds, or a Peak Limiter, each with exactly its own parameter block, whose curves drive its parameters by
    /// game parameters (a modulator's curve is kept, not built).
    fn check_effect(&self, e: &Effect) -> Result<(), String> {
        let blocks = [e.eq.is_some(), e.distortion.is_some(), e.limiter.is_some(), e.master.is_some(), e.compressor.is_some()];
        let known: fn(u32) -> bool = match e.plugin.as_str() {
            PARAMETRIC_EQ if blocks == [true, false, false, false, false] => |p| p <= EQ_PARAM_LFE,
            GUITAR_DISTORTION if blocks == [false, true, false, false, false] => gd_param_known,
            PEAK_LIMITER if blocks == [false, false, true, false, false] => |p| p <= LIMITER_PARAM_LINK,
            // (No curve or state drives the master's plug-in in the data: one would not be built.)
            MASTER_SUITE if blocks == [false, false, false, true, false] => |_| false,
            COMPRESSOR if blocks == [false, false, false, false, true] => |p| p <= COMPRESSOR_PARAM_LINK,
            PARAMETRIC_EQ | GUITAR_DISTORTION | PEAK_LIMITER | MASTER_SUITE | COMPRESSOR => return Err(format!("plug-in {} with another plug-in's block", e.plugin)),
            _ => return Err(format!("plug-in {} is not one the mixer runs", e.plugin)),
        };
        if let Some(m) = &e.master {
            master::MasterChain::new(m)?;
        }
        if let Some(eq) = &e.eq {
            if eq.bands.len() != 3 {
                return Err(format!("{} bands, the Parametric EQ of bank v135 has 3", eq.bands.len()));
            }
        }
        if let Some(d) = &e.distortion {
            if d.pre_eq.len() != 3 || d.post_eq.len() != 3 {
                return Err(format!("{} + {} bands, the Guitar Distortion has 3 + 3", d.pre_eq.len(), d.post_eq.len()));
            }
            if d.distortion > filter::DISTORTION_HEAVY {
                return Err(format!("distortion type {} is not built (Fuzz and Clip are not in the data)", d.distortion));
            }
        }
        for r in &e.rtpc {
            if !known(r.param_id) || r.param != format!("plugin parameter {}", r.param_id) {
                return Err(format!("a curve on {} ({}) is not a parameter of plug-in {}", r.param, r.param_id, e.plugin));
            }
            if e.plugin == GUITAR_DISTORTION && r.param_id == GD_PARAM_TYPE {
                return Err("a curve on the distortion's type is not built".into());
            }
            if r.accumulation == Accumulation::Boolean {
                return Err(format!("a boolean curve on parameter {} is not built", r.param_id));
            }
            // (A parameter's default sets the band where nothing sets it: one the file does not define would read 0.)
            if r.rtpc_type == RtpcType::GameParameter && !self.file.game_syncs.game_parameters.contains_key(&r.game_parameter.id) {
                return Err(format!("game parameter {} of a curve on parameter {} is not defined", r.game_parameter.id, r.param_id));
            }
        }
        Ok(())
    }

    /// The effects of one node's or bus's slots the mixer runs on its voices, in slot order, into `out`: every shipped
    /// effect but the compressors (a bus's compressor acts on its mix: only the sub-mix's is built, on the sub-mix).
    fn shipped_stages(&self, c: &FxChain, holder: Id, on_bus: bool, out: &mut Vec<FxStage>) {
        let f = &self.file;
        let mut slots: Vec<&FxSlot> = c.fx.iter().filter(|s| s.fx_id.as_ref().and_then(|r| f.effects.get(&r.id)).is_some_and(|e| e.compressor.is_none())).collect();
        slots.sort_by_key(|s| s.index);
        out.extend(slots.into_iter().map(|s| FxStage { holder, slot: s.index, effect: s.fx_id.as_ref().unwrap().id, on_bus }));
    }

    /// Every Sound's effects the mixer runs ([`SoundBank::fx_stages`]).
    fn stages(&self) -> HashMap<Id, Vec<FxStage>> {
        let f = &self.file;
        let shipped = |c: &FxChain, holder: Id, on_bus: bool, out: &mut Vec<FxStage>| self.shipped_stages(c, holder, on_bus, out);
        let mut all = HashMap::new();
        for (id, o) in &f.objects {
            if !matches!(o, Object::Sound(_)) {
                continue;
            }
            let chain = self.chain(*id);
            let mut out = Vec::new();
            let holder = chain.iter().copied().find(|a| self.node(*a).is_some_and(|n| n.fx.override_parent == Some(true))).or_else(|| chain.last().copied());
            if let Some(h) = holder {
                if let Some(n) = self.node(h) {
                    shipped(&n.fx, h, false, &mut out);
                }
            }
            if let Some(b) = self.output_bus(*id) {
                for bid in self.bus_chain(b) {
                    if let Some(bus) = self.bus(*bid) {
                        shipped(&bus.fx, *bid, true, &mut out);
                    }
                }
            }
            all.insert(*id, out);
        }
        all
    }

    /// A Sound's effects the mixer runs, in signal order.
    pub fn fx_stages(&self, sound: Id) -> &[FxStage] {
        self.fx_stages.get(&sound).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The bus mixed as a sub-mix ([`SUBMIX_BUS`]), when the file holds it with its compressor.
    pub fn submix(&self) -> Option<&Submix> {
        self.submix.as_ref()
    }

    /// For a Sound mixed into the sub-mix (its bus or any bus under it), how many of its effects ([`SoundBank::fx_stages`])
    /// run on its voice: those before the compressor (its node's, the buses' under the sub-mix bus, the bus's earlier
    /// slots). None for any other Sound.
    pub fn submix_stages(&self, sound: Id) -> Option<usize> {
        self.submix_stages.get(&sound).copied()
    }

    /// The sub-mix and its Sounds' splits, checked: [`SUBMIX_BUS`] holds one compressor in its slots, linked (the unlinked
    /// routine is not built) and with no curve on what only its Init reads (Process LFE, Channel Link); every Sound mixed
    /// into it mixes in 7.1.4 (the sub-mix's 12 channels); every effect after the compressor on the way to the master is
    /// linear and the same on every channel (an EQ, the limiter's output level: they run on the sum once folded to stereo,
    /// which is exact only for such); each such Sound's effects end with those. None when the file holds no such bus or
    /// compressor (an older file: every voice plays as before).
    fn submix_of(&self) -> Result<(Option<Submix>, HashMap<Id, usize>), String> {
        let none = Ok((None, HashMap::new()));
        let Some(bus) = self.bus(SUBMIX_BUS) else { return none };
        let comps: Vec<(u32, &Effect)> = bus.fx.fx.iter().filter_map(|s| self.file.effects.get(&s.fx_id.as_ref()?.id).filter(|e| e.compressor.is_some()).map(|e| (s.index, e))).collect();
        let (slot, e) = match comps[..] {
            [] => return none,
            [one] => one,
            _ => return Err(format!("{} compressors in its slots (one is built)", comps.len())),
        };
        if !e.compressor.as_ref().is_some_and(|c| c.channel_link) {
            return Err(format!("its compressor {} does not link its channels (the unlinked routine is not built)", e.id));
        }
        if e.rtpc.iter().any(|r| r.param_id > COMPRESSOR_PARAM_OUTPUT_LEVEL) {
            return Err(format!("a curve on its compressor {}'s Process LFE or Channel Link (read once, at Init) is not built", e.id));
        }
        let mut after = Vec::new();
        let mut own = Vec::new();
        self.shipped_stages(&bus.fx, SUBMIX_BUS, true, &mut own);
        after.extend(own.into_iter().filter(|s| s.slot > slot));
        for b in self.bus_chain(SUBMIX_BUS).iter().skip(1) {
            if let Some(x) = self.bus(*b) {
                self.shipped_stages(&x.fx, *b, true, &mut after);
            }
        }
        if let Some(st) = after.iter().find(|st| self.file.effects.get(&st.effect).is_some_and(|x| x.distortion.is_some())) {
            return Err(format!("the Guitar Distortion {} on bus {} after its compressor is not linear: it cannot run on the folded sum", st.effect, st.holder));
        }
        let mut splits = HashMap::new();
        for (id, o) in &self.file.objects {
            if !matches!(o, Object::Sound(_)) || !self.output_bus(*id).is_some_and(|b| self.bus_chain(b).contains(&SUBMIX_BUS)) {
                continue;
            }
            if self.route(*id).layout != place::Layout::Height714 {
                return Err(format!("Sound {id} mixes into it in stereo (its 12 channels are built)"));
            }
            let all = self.fx_stages(*id);
            let n = all.len().checked_sub(after.len()).filter(|n| all[*n..] == after[..]).ok_or_else(|| format!("Sound {id}: its effects do not end with those after the compressor"))?;
            splits.insert(*id, n);
        }
        Ok((Some(Submix { bus: SUBMIX_BUS, slot, effect: e.id, after }), splits))
    }

    /// The positioning in force on an object: the nearest node up its chain that overrides its parent's, else the top one.
    pub fn positioning(&self, obj: Id) -> Option<&Positioning> {
        let chain = self.chain(obj);
        chain.iter().filter_map(|a| self.node(*a)).find(|n| n.positioning.override_parent).or_else(|| chain.last().and_then(|a| self.node(*a))).map(|n| &n.positioning)
    }

    /// The bus a sound mixes into: the nearest output bus up its chain.
    pub fn output_bus(&self, obj: Id) -> Option<Id> {
        self.chain(obj).iter().find_map(|a| self.node(*a).and_then(|n| n.override_bus.as_ref().map(|b| b.id)))
    }

    /// A Sound's way to the output ([`place::Route`]; a stereo route, direct assignment, for an id that is not a Sound).
    pub fn route(&self, sound: Id) -> place::Route {
        self.routes.get(&sound).copied().unwrap_or_default()
    }

    /// Every Sound's route, checked: the layout its output bus mixes in (T5g-2 section 2: a bus's own channel configuration,
    /// else its parent's, the master's the device's), every 12-channel bus on it connecting by direct assignment (the
    /// connections built: 12 to 12 the identity, 12 to stereo Elite's down-mix); the 2D panner of its positioning in force;
    /// and its CenterPCT, PAN_LR and PAN_FR, each the nearest value up its chain as far as the node whose positioning is in
    /// force (`levels.py`'s rule; 0 where none is set: the property table 0x145ee0b00). A sound with no output bus on its
    /// chain mixes into the master.
    fn routes(&self) -> Result<HashMap<Id, place::Route>, String> {
        use place::{Layout, Panner, Route};
        let mut own = HashMap::new();
        for (id, o) in &self.file.objects {
            if let Object::Bus(b) = o {
                let word = b.channel_config.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()).ok_or_else(|| format!("bus {id}: channel configuration {:?} is not a number", b.channel_config))?;
                own.insert(*id, Layout::of_config(word).map_err(|e| format!("bus {id}: {e}"))?);
            }
        }
        let mut routes = HashMap::new();
        for (id, o) in &self.file.objects {
            if !matches!(o, Object::Sound(_)) {
                continue;
            }
            let at = |e: String| format!("Sound {id}: {e}");
            let buses = self.output_bus(*id).map(|b| self.bus_chain(b)).unwrap_or(&[]);
            let layouts: Vec<Option<Layout>> = buses.iter().map(|b| own.get(b).copied().flatten()).collect();
            let layout = Layout::of_route(&layouts).map_err(at)?;
            for (i, b) in buses.iter().enumerate() {
                if Layout::of_route(&layouts[i..]).map_err(at)? == Layout::Height714 && self.bus(*b).is_some_and(|bus| bus.positioning.panner_type != "DirectSpeakerAssignment") {
                    return Err(at(format!("12-channel bus {b} connects by {} (only direct assignment is built)", self.bus(*b).map_or("", |x| x.positioning.panner_type.as_str()))));
                }
            }
            let panner = self.positioning(*id).map_or(Ok(Panner::DirectSpeakerAssignment), |p| Panner::of_name(&p.panner_type)).map_err(at)?;
            let value = |k: &str| {
                for n in self.chain(*id).iter().filter_map(|a| self.node(*a)) {
                    if let Some(v) = n.props.get(k).and_then(PropValue::num) {
                        return v;
                    }
                    if n.positioning.override_parent {
                        break;
                    }
                }
                0.0
            };
            routes.insert(*id, Route { layout, panner, centre_pct: value("CenterPCT"), pan_lr: value("PAN_LR"), pan_fr: value("PAN_FR") });
        }
        Ok(routes)
    }

    /// The effects the mixer hands out that are NOT EXACT because a modulator drives one of their parameters and
    /// modulators are not built (the parameter keeps its value without the modulator's curve): each effect's id with the
    /// modulator and the parameter. In the shipped data one: the Guitar Distortion `d1b95c6e` on the ship voice's node
    /// `1d2c221f`, whose wet share is the LFO modulator `2bf46665`'s curve (0 to 100 %); that LFO's depth is itself an
    /// exclusive curve of VesselVoiceGlitch (0 at 0, the parameter's default, which the ship never sets), and what an LFO
    /// of depth 0 outputs is not given by Audiokinetic's documentation, so the distortion plays dry (wet share 0).
    pub fn effects_not_exact(&self) -> Vec<(Id, Id, u32)> {
        self.file
            .effects
            .values()
            .flat_map(|e| e.rtpc.iter().filter(|r| r.rtpc_type == data::RtpcType::Modulator).map(move |r| (e.id, r.game_parameter.id, r.param_id)))
            .collect()
    }

    fn check_props(&self, p: &Props) -> Result<(), String> {
        for (k, v) in p {
            if !BUILT_PARAMS.contains(&k.as_str()) && !NOT_BUILT_PARAMS.contains(&k.as_str()) {
                return Err(format!("property {k} is not one the mixer knows"));
            }
            if k == "AttenuationID" {
                let PropValue::Ref(r) = v else { return Err("AttenuationID is not a reference".into()) };
                if !matches!(self.file.objects.get(&r.id), Some(Object::Attenuation(_))) {
                    return Err(format!("attenuation {} is missing", r.id));
                }
            } else if v.num().is_none() {
                return Err(format!("property {k} is not a number"));
            }
        }
        Ok(())
    }

    fn check_rtpcs(&self, rs: &[Rtpc]) -> Result<(), String> {
        for r in rs {
            let p = r.param.as_str();
            if !BUILT_PARAMS.contains(&p) && !NOT_BUILT_PARAMS.contains(&p) {
                return Err(format!("a curve on {p} is not built"));
            }
            if r.accumulation == Accumulation::Exclusive && !matches!(p, "MaxNumInstances" | "PositioningTypeBlend") {
                return Err(format!("an exclusive curve on {p} is not built"));
            }
        }
        Ok(())
    }

    fn check_states(&self, s: &StateBlock) -> Result<(), String> {
        for g in &s.groups {
            for st in &g.states {
                for k in st.deltas.keys() {
                    if !BUILT_PARAMS.contains(&k.as_str()) && !NOT_BUILT_PARAMS.contains(&k.as_str()) {
                        return Err(format!("state delta on {k} is not built"));
                    }
                }
            }
        }
        Ok(())
    }

    fn check_node(&self, n: &Node) -> Result<(), String> {
        self.check_props(&n.props)?;
        for k in n.ranged.keys() {
            if !RANGED_BUILT.contains(&k.as_str()) {
                return Err(format!("randomiser on {k} is not built"));
            }
        }
        self.check_rtpcs(&n.rtpc)?;
        self.check_states(&n.states)?;
        if let Some(b) = &n.override_bus {
            if !self.bus_chains.contains_key(&b.id) {
                return Err(format!("output bus {} is missing", b.id));
            }
        }
        Ok(())
    }

    fn check_action(&self, a: &Action) -> Result<(), SoundError> {
        let at = |s: &str| bad(format!("action {}: {s}", a.id));
        if a.params.exceptions.as_ref().is_some_and(|e| !e.is_empty()) {
            return Err(at("exceptions are not built"));
        }
        if a.target_is_bus {
            return Err(at("actions on buses are not built"));
        }
        for k in a.props.keys() {
            if !matches!(k.as_str(), "DelayTime" | "TransitionTime" | "Probability") {
                return Err(at(&format!("property {k} is not built")));
            }
        }
        for k in a.ranged.keys() {
            if k != "DelayTime" {
                return Err(at(&format!("randomiser {k} is not built")));
            }
        }
        // A Stop / Set / Seek target need not be in the file: it is matched against the chains of what plays.
        let node_target = || match self.file.objects.get(&a.target) {
            Some(o) if o.node().is_none() => Err(at(&format!("target {} is not an actor-mixer node", a.target))),
            _ => Ok(()),
        };
        match a.kind {
            ActionKind::Play => match self.file.objects.get(&a.target) {
                Some(Object::Sound(_) | Object::RanSeqCntr(_) | Object::SwitchCntr(_) | Object::LayerCntr(_)) => {}
                _ => return Err(at(&format!("play target {} is missing or not playable", a.target))),
            },
            ActionKind::Stop | ActionKind::SetVolume | ActionKind::ResetVolume | ActionKind::SetPitch | ActionKind::ResetPitch => node_target()?,
            ActionKind::Seek => {
                node_target()?;
                if a.params.seek_relative_to_duration != Some(true) || a.params.snap_to_nearest_marker != Some(false) || a.params.seek.is_none() {
                    return Err(at("only a seek relative to the duration, without markers, is built"));
                }
            }
            ActionKind::PlayEvent => {
                if !self.file.events.contains_key(&a.target) {
                    return Err(at(&format!("posted event {} is missing", a.target)));
                }
            }
            ActionKind::SetState => {
                if a.params.state_group.is_none() || a.params.state.is_none() {
                    return Err(at("SetState without its group and state"));
                }
            }
            ActionKind::SetSwitch => {
                if a.params.switch_group.is_none() || a.params.switch.is_none() {
                    return Err(at("SetSwitch without its group and value"));
                }
            }
            ActionKind::SetGameParameter => {
                if a.params.value.is_none() || a.params.value_meaning.is_none() {
                    return Err(at("SetGameParameter without its value"));
                }
            }
        }
        if matches!(a.kind, ActionKind::SetVolume | ActionKind::SetPitch | ActionKind::ResetVolume | ActionKind::ResetPitch) && a.params.value.is_none() {
            return Err(at("a set / reset without its value"));
        }
        Ok(())
    }

    /// The event a name posts, if it is in the file.
    pub fn event(&self, name: &str) -> Option<&Event> {
        self.file.events.get(&event_id(name))
    }

    pub fn object(&self, id: Id) -> Option<&Object> {
        self.file.objects.get(&id)
    }

    /// A node's actor-mixer chain (itself first).
    pub fn chain(&self, id: Id) -> &[Id] {
        self.chains.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn bus_chain(&self, id: Id) -> &[Id] {
        self.bus_chains.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn node(&self, id: Id) -> Option<&Node> {
        self.file.objects.get(&id).and_then(Object::node)
    }

    fn bus(&self, id: Id) -> Option<&Bus> {
        match self.file.objects.get(&id) {
            Some(Object::Bus(b)) => Some(b),
            _ => None,
        }
    }
}
