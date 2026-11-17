//! The runtime: what the Wwise sound engine does with `sounds.json` (the module's docs list what is built).

use super::curve::{cents_to_ratio, db_to_gain, eval, find, gain_to_db, interpolate};
use super::data::*;
use super::filter;
use super::place::Placement;
use super::{
    event_id, wwise_id, FxStage, SoundBank, SoundError, EQ_PARAM_LFE, EQ_PARAM_OUTPUT_LEVEL, GD_PARAM_DRIVE, GD_PARAM_OUTPUT_LEVEL, GD_PARAM_RECTIFICATION,
    GD_PARAM_TONE, GD_PARAM_TYPE, GD_PARAM_WET, LIMITER_PARAM_OUTPUT_LEVEL, LIMITER_PARAM_THRESHOLD,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// What a voice puts out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// A recording: `sounds.json`'s `media` entry of this source id.
    Audio { source: Id },
    /// A Wwise Motion Generator (controller rumble): `source_plugins` holds its decoded curve.
    Rumble { source: Id },
    /// A Wwise Tone Generator: `source_plugins` holds its parameters (synthesis is not built here).
    Synth { source: Id },
}

/// A voice the player builds now: the first update in which it is heard, or the first after it was virtual (Wwise's
/// virtual voices make no sound and cost the player nothing; the player drops a voice whose update says virtual and builds
/// it again from here when it is heard again).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct VoiceStart {
    /// Seconds into the recording to start from (a voice started between two steps is already this far along; one coming
    /// back from virtual starts where its virtual behaviour says: the beginning, the time it would have reached, or where it
    /// went virtual).
    pub offset: f32,
    pub looping: bool,
    /// How many times it plays from `offset` on, this pass included; `None` = for ever.
    pub loop_count: Option<u32>,
    /// The part that repeats, in sample frames, start inclusive, end exclusive (the recording's smpl loop: its intro plays
    /// once, then this region loops); `None` = the whole recording. Only on a looping voice.
    pub loop_region: Option<(u64, u64)>,
}

/// One band of a Parametric EQ on a voice's way, as its parameters are now (bank values, curves and the plug-in's own clamps
/// applied): `filter` None for a type number out of the plug-in's range. A band that is off, or whose effect is bypassed,
/// passes the signal untouched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqNow {
    pub on: bool,
    pub filter: Option<EqFilter>,
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl EqNow {
    /// A band that passes the signal untouched.
    pub const OFF: EqNow = EqNow { on: false, filter: None, frequency: 0.0, gain_db: 0.0, q: 0.0 };
}

/// A Guitar Distortion on a voice's way as its parameters are now (bank values, curves and its `SetParam`'s clamps
/// applied, `0x144a04b60`: band gains held to -48..48 dB, drive, tone, rectification and wet share to 0-100, the output
/// level to -24..24 dB and made a linear gain by `powf(10, level / 20)`), for [`super::filter::Distortion`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistortionNow {
    pub pre: [EqNow; 3],
    pub post: [EqNow; 3],
    pub kind: u32,
    pub drive: f32,
    pub tone: f32,
    pub rectification: f32,
    /// The output level, a linear gain.
    pub level: f32,
    pub wet: f32,
}

/// One effect on a voice's way, in signal order ([`VoiceUpdate::fx`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxNow {
    /// Bypassed, or an effect whose only work is a gain already in the voice's `gain` (the master's limiter).
    Off,
    /// A Parametric EQ's three bands (its output level is in the voice's `gain`).
    Eq([EqNow; 3]),
    Distortion(DistortionNow),
}

/// The master bus's Peak Limiter now: its threshold (dB) and output level (dB, in every voice's `gain`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimiterNow {
    pub threshold_db: f32,
    pub output_db: f32,
}

/// What an effect on a voice's way does now: what the player runs, and the gain (dB) it adds to the voice.
#[derive(Clone, Copy, Debug, PartialEq)]
struct StageNow {
    fx: FxNow,
    gain_db: f64,
}

/// One voice this step.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceUpdate {
    /// Stable for the voice's life.
    pub voice: u32,
    pub output: Output,
    pub emitter: u32,
    /// The emitter's position from the listener, if the game gave one ([`Mixer::set_emitter`]).
    pub position: Option<[f32; 3]>,
    /// Where the voice sits, for the player (T5d-5): given when the game placed its emitter and its sound's positioning in
    /// force is 3D on the emitter's position ([`Placement`]). The player then applies the distance's curves (volume,
    /// low-pass, high-pass) and works out its matrix itself, from the place it is given each frame ([`Placement::at`]), so
    /// `gain`, `volume_db`, `lowpass` and `highpass` leave the attenuation's share out (and the filter values are not
    /// clamped: the player adds the distance's share, then clamps to 0-100, as the mixer clamps the sum). None: the voice
    /// plays through `matrix`, as given.
    pub place: Option<Arc<Placement>>,
    /// The voice's stereo gains (`[input channel][output channel]`, T5d-7: its sound's route and pan, for its recording's
    /// channel count) as a voice at the listener: what a voice that is not placed plays through for its life (an emitter
    /// the game has not placed is at the listener, so straight ahead; its distance's share, at 0 m, is in `gain`).
    pub matrix: [[f32; 2]; 2],
    /// The recording's own gain ([`super::data::Media::akd_gain_2`], 1 without it): the player multiplies the recording's
    /// samples by it before any filter or effect (Elite multiplies the voice's make-up gain by it at 0x144a78c80, ahead of
    /// the voice's effects: the Guitar Distortion on a node is not linear).
    pub media_gain: f32,
    pub start: Option<VoiceStart>,
    /// Ends now (the player stops it; any fade has already run down to silence).
    pub stop: bool,
    /// Jump to this many seconds into the recording now (a Seek action, or a virtual voice coming back).
    pub seek_to: Option<f32>,
    /// Every volume combined: hierarchy, buses, game parameter curves, states, randomisers, actions, attenuation, ducking
    /// (`volume_db`) times the fades (`fade`).
    pub gain: f32,
    pub volume_db: f32,
    /// Action, transition, switch and stop fades combined, a linear factor.
    pub fade: f32,
    pub pitch: f32,
    pub pitch_cents: f32,
    /// Wwise's 0-100 filter values (a placed voice's without its distance's share: `place`).
    pub lowpass: f32,
    pub highpass: f32,
    /// Cutoff frequencies at [`OUTPUT_RATE`] (Elite's own conversion, [`super::filter`]); `None` where the filter is
    /// bypassed (a value of at most 0.1).
    pub lowpass_hz: Option<f32>,
    pub highpass_hz: Option<f32>,
    /// Below the volume threshold, or beyond a playback limit that makes voices virtual: the player drops it (a virtual
    /// voice is not heard and is not built; it is built again from its next `start`).
    pub virtual_voice: bool,
    /// The effects on its way to the master, in signal order (one a stage, [`SoundBank::fx_stages`]): the node's in force,
    /// then each bus's; the player runs them after the voice's own filters and before its level (a linear effect on a bus
    /// gives the bus's mix what it gives each voice; the Guitar Distortions the data puts on buses act on nothing heard,
    /// `every_effect_on_a_bus_is_linear_where_it_acts`). The EQs' and the limiter's output levels are in `gain` and
    /// `volume_db`; a distortion's is its own. Empty for a virtual voice and a stop.
    pub fx: Vec<FxNow>,
}

/// SplitMix64: the mixer's one source of chance.
#[derive(Clone, Debug)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// In 0..1.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        if hi <= lo {
            lo
        } else {
            lo + (hi - lo) * self.unit()
        }
    }
}

/// A value moving along a curve over time.
#[derive(Clone, Copy, Debug)]
struct Anim {
    from: f64,
    to: f64,
    start: f64,
    dur: f64,
    curve: Interp,
}

impl Anim {
    fn fixed(v: f64) -> Anim {
        Anim { from: v, to: v, start: 0.0, dur: 0.0, curve: Interp::Linear }
    }

    fn at(&self, t: f64) -> f64 {
        if self.dur <= 0.0 || t >= self.start + self.dur {
            self.to
        } else if t <= self.start {
            self.from
        } else {
            interpolate((t - self.start) / self.dur, self.from, self.to, self.curve)
        }
    }

    fn done(&self, t: f64) -> bool {
        self.dur <= 0.0 || t >= self.start + self.dur
    }
}

/// A fade of an instance's level.
#[derive(Clone, Copy, Debug)]
struct Fade {
    start: f64,
    dur: f64,
    curve: Interp,
}

#[derive(Clone, Copy, Debug, Default)]
struct Env {
    fade_in: Option<Fade>,
    fade_out: Option<Fade>,
}

impl Env {
    fn level(&self, t: f64) -> f64 {
        let mut l = 1.0;
        if let Some(f) = self.fade_in {
            if t < f.start {
                l = 0.0;
            } else if f.dur > 0.0 && t < f.start + f.dur {
                l *= interpolate((t - f.start) / f.dur, 0.0, 1.0, f.curve);
            }
        }
        if let Some(f) = self.fade_out {
            if t >= f.start + f.dur {
                l = 0.0;
            } else if t > f.start {
                l *= interpolate((t - f.start) / f.dur, 1.0, 0.0, f.curve);
            }
        }
        l
    }
}

/// A randomiser draw, per property.
#[derive(Clone, Copy, Debug, Default)]
struct Rolls {
    volume: f64,
    pitch: f64,
    lpf: f64,
    hpf: f64,
    delay: f64,
}

impl Rolls {
    fn add(&mut self, rng: &mut Rng, ranged: &Ranged) {
        for (k, r) in ranged {
            let v = rng.range(r.min, r.max);
            match k.as_str() {
                "Volume" => self.volume += v,
                "Pitch" => self.pitch += v,
                "LPF" => self.lpf += v,
                "HPF" => self.hpf += v,
                "InitialDelay" => self.delay += v,
                _ => {}
            }
        }
    }

    fn plus(&self, o: &Rolls) -> Rolls {
        Rolls { volume: self.volume + o.volume, pitch: self.pitch + o.pitch, lpf: self.lpf + o.lpf, hpf: self.hpf + o.hpf, delay: self.delay + o.delay }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Out {
    Audio(Id),
    Rumble(Id),
    Synth(Id),
    Silence,
}

#[derive(Clone, Debug)]
struct Voice {
    id: u32,
    out: Out,
    start_at: f64,
    /// Seconds of the recording played, all passes counted.
    pos: f64,
    /// One pass (s), if known.
    dur: Option<f64>,
    /// None = for ever.
    loops: Option<u32>,
    /// The repeating part, in sample frames, start inclusive, end exclusive, and the sample rate (a recording with an intro).
    region: Option<(u64, u64, f64)>,
    started: bool,
    reported: bool,
    /// The player holds it (a start was reported and no virtual update since).
    built: bool,
    offset: f64,
    seek: Option<f64>,
    virt: bool,
    virt_pos: f64,
}

impl Voice {
    /// Where `pos` (seconds played, every pass counted) lies in the recording and how many plays are left from there, this
    /// one included: the first pass runs to the loop region's end (or the recording's), each later one from the region's
    /// start, the last on to the recording's end (as the player plays them).
    fn place(&self, pos: f64) -> (f64, Option<u32>) {
        let Some(d) = self.dur.filter(|d| *d > 0.0) else { return (pos.max(0.0), self.loops) };
        if self.loops == Some(1) {
            return (pos.clamp(0.0, d), Some(1));
        }
        let (restart, end) = self.region.map(|(a, b, rate)| (a as f64 / rate, (b as f64 / rate).min(d))).unwrap_or((0.0, d));
        let again = (end - restart).max(1e-9);
        if pos < end {
            return (pos.max(0.0), self.loops);
        }
        let k = ((pos - end) / again).floor();
        let mut within = pos - end - k * again;
        let mut k = k as u64;
        if let Some(n) = self.loops {
            // (The last play runs on past the region's end: the passes beyond the last are in its tail.)
            let last = u64::from(n.max(2)) - 2;
            if k >= last {
                within += (k - last) as f64 * again;
                k = last;
            }
            let left = u64::from(n).saturating_sub(k + 1) as u32;
            return ((restart + within).min(d), Some(left.max(1)));
        }
        let _ = k;
        (restart + within, None)
    }

    fn total(&self) -> Option<f64> {
        match (self.dur, self.loops) {
            (Some(d), Some(n)) => {
                // Played n times: the whole recording once and its loop region (or all of it) n - 1 times more.
                let again = self.region.map(|(a, b, rate)| (b - a) as f64 / rate).unwrap_or(d);
                Some(d + again * f64::from(n.max(1) - 1))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct RsInst {
    key: (Id, Option<u32>),
    /// Items still to start; None = for ever.
    left: Option<u64>,
    next_at: Option<f64>,
    current: Option<u64>,
    current_start: f64,
}

#[derive(Clone, Debug)]
struct SwInst {
    value: Option<Id>,
    kids: BTreeMap<Id, u64>,
}

#[derive(Clone, Debug)]
enum Kind {
    Voice(Voice),
    RanSeq(RsInst),
    Switch(SwInst),
    Layer,
}

/// A playing object: a voice, or a container scheduling its children.
#[derive(Clone, Debug)]
struct Inst {
    obj: Id,
    emitter: u32,
    parent: Option<u64>,
    kids: Vec<u64>,
    rolls: Rolls,
    env: Env,
    /// The blend container whose tracks apply to this child.
    layer: Option<Id>,
    /// Stopping: no new children, and gone at this time.
    stop_at: Option<f64>,
    kind: Kind,
}

/// The playlist state of a random / sequence container, per game object or global.
#[derive(Clone, Debug, Default)]
struct Playlist {
    next: usize,
    history: Vec<usize>,
    bag: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
struct Gp {
    value: f64,
    target: Anim,
}

/// A state group now: moving from a blend of states (weights summing to 1: one state, or the blend a change met in the
/// middle of a transition) to `current` over `dur` seconds.
#[derive(Clone, Debug)]
struct StateNow {
    current: Id,
    from: Vec<(Id, f64)>,
    start: f64,
    dur: f64,
}

impl StateNow {
    fn progress(&self, now: f64) -> f64 {
        if self.dur <= 0.0 {
            1.0
        } else {
            ((now - self.start) / self.dur).clamp(0.0, 1.0)
        }
    }

    /// Each state's weight now.
    fn weights(&self, now: f64) -> Vec<(Id, f64)> {
        let k = self.progress(now);
        let mut w: Vec<(Id, f64)> = self.from.iter().map(|(s, x)| (*s, x * (1.0 - k))).collect();
        match w.iter_mut().find(|(s, _)| *s == self.current) {
            Some(e) => e.1 += k,
            None => w.push((self.current, k)),
        }
        w.retain(|(_, x)| *x > 1e-12);
        w
    }
}

#[derive(Clone, Copy, Debug)]
struct DuckState {
    level: Anim,
    on: bool,
    off_since: f64,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    due: f64,
    action: Id,
    emitter: u32,
}

/// The Wwise sound engine's work on the Sidewinder's banks, for one listener.
pub struct Mixer {
    bank: Arc<SoundBank>,
    rng: Rng,
    now: f64,
    seq: u64,
    gps: BTreeMap<(Id, Option<u32>), Gp>,
    states: BTreeMap<Id, StateNow>,
    switches: BTreeMap<(Id, u32), Id>,
    emitters: BTreeMap<u32, [f32; 3]>,
    /// Each sound's way to the output (made once a sound: `placement_of`).
    placements: BTreeMap<Id, Arc<Placement>>,
    /// Delayed actions by (due time's bits, order posted): due times are never negative, so their bits sort as they do.
    pending: BTreeMap<(u64, u64), Pending>,
    insts: BTreeMap<u64, Inst>,
    next_voice: u32,
    volume_over: BTreeMap<(Id, Option<u32>), Anim>,
    pitch_over: BTreeMap<(Id, Option<u32>), Anim>,
    playlists: BTreeMap<(Id, Option<u32>), Playlist>,
    ducks: BTreeMap<(Id, usize), DuckState>,
    /// Voices removed since the last step (their stop update is still owed).
    ended: Vec<Ended>,
    updates: Vec<VoiceUpdate>,
    post_depth: usize,
}

/// The state every group starts in (Wwise's "None").
fn state_none() -> Id {
    wwise_id("none")
}

/// What one voice's parameters add up to.
#[derive(Clone, Copy, Debug, Default)]
struct Mix {
    db: f64,
    cents: f64,
    lpf: f64,
    hpf: f64,
    delay: f64,
    priority_add: f64,
    transition_add: f64,
    /// The distance attenuation's shares (already in `db`, `lpf`, `hpf`): a placed voice's player applies them itself.
    att_db: f64,
    att_lpf: f64,
    att_hpf: f64,
}

impl Mix {
    fn add(&mut self, param: &str, v: f64) {
        match param {
            "Volume" | "MakeUpGain" | "BusVolume" | "OutputBusVolume" => self.db += v,
            "Pitch" => self.cents += v,
            "LPF" | "OutputBusLPF" => self.lpf += v,
            "HPF" | "OutputBusHPF" => self.hpf += v,
            "InitialDelay" => self.delay += v,
            "Priority" => self.priority_add += v,
            "PlayMechanismSpecialTransitionsValue" => self.transition_add += v,
            _ => {}
        }
    }
}

impl Mixer {
    pub fn new(bank: Arc<SoundBank>, seed: u64) -> Mixer {
        Mixer {
            bank,
            rng: Rng(seed),
            now: 0.0,
            seq: 0,
            gps: BTreeMap::new(),
            states: BTreeMap::new(),
            switches: BTreeMap::new(),
            emitters: BTreeMap::new(),
            placements: BTreeMap::new(),
            pending: BTreeMap::new(),
            insts: BTreeMap::new(),
            next_voice: 1,
            volume_over: BTreeMap::new(),
            pitch_over: BTreeMap::new(),
            playlists: BTreeMap::new(),
            ducks: BTreeMap::new(),
            ended: Vec::new(),
            updates: Vec::new(),
            post_depth: 0,
        }
    }

    /// Seconds since the mixer was made (the sum of every `step`'s `dt`).
    pub fn time(&self) -> f64 {
        self.now
    }

    /// What plays now, by what the events started: each live top-level instance (a Play action's target) and its emitter,
    /// sorted. Unlike the voices, it does not change with a random container's pick or a sequence's gap.
    pub fn playing_objects(&self) -> Vec<(u32, Id)> {
        let mut v: Vec<(u32, Id)> = self.insts.values().filter(|i| i.parent.is_none() && i.stop_at.is_none()).map(|i| (i.emitter, i.obj)).collect();
        v.sort_by_key(|(e, o)| (*e, o.0));
        v
    }

    /// Posts an event on an emitter (by name, or `event_<8 hex>` for an unnamed one). Its actions run in order: those
    /// without delay at once, the others in a later `step`. An event the file does not hold is an `Err`.
    pub fn post(&mut self, event: &str, emitter: u32) -> Result<(), SoundError> {
        let id = event_id(event);
        if !self.bank.file.events.contains_key(&id) {
            return Err(SoundError::UnknownEvent(event.to_string()));
        }
        self.post_id(id, emitter);
        Ok(())
    }

    fn post_id(&mut self, id: Id, emitter: u32) {
        if self.post_depth >= MAX_DEPTH {
            return;
        }
        self.post_depth += 1;
        self.post_actions(id, emitter);
        self.post_depth -= 1;
    }

    fn post_actions(&mut self, id: Id, emitter: u32) {
        let bank = self.bank.clone();
        let Some(e) = bank.file.events.get(&id) else { return };
        for a in &e.actions {
            let Some(Object::Action(act)) = bank.object(*a) else { continue };
            let delay_ms = prop(&act.props, "DelayTime") + act.ranged.get("DelayTime").map(|r| self.rng.range(r.min, r.max)).unwrap_or(0.0);
            let due = self.now + delay_ms.max(0.0) / 1000.0;
            if delay_ms <= 0.0 {
                self.run_action(act, emitter);
            } else {
                self.seq += 1;
                self.pending.insert((due.to_bits(), self.seq), Pending { due, action: act.id, emitter });
            }
        }
    }

    /// Sets a game parameter globally (`emitter` None) or on one emitter; it ramps to the value by the Init bank's rule.
    /// A value that is not finite is ignored.
    pub fn set_game_parameter(&mut self, name: &str, value: f32, emitter: Option<u32>) {
        self.set_game_parameter_id(wwise_id(name), value, emitter);
    }

    fn set_gp(&mut self, id: Id, value: f64, emitter: Option<u32>, fade: Option<(f64, Interp)>) {
        let start = self.gp_value(id, emitter.unwrap_or(u32::MAX));
        let now = self.now;
        let target = match fade {
            Some((dur, curve)) if dur > 0.0 => {
                let from = self.gps.get(&(id, emitter)).map(|g| g.target.at(now)).unwrap_or(start);
                Anim { from, to: value, start: now, dur, curve }
            }
            _ => Anim::fixed(value),
        };
        let e = self.gps.entry((id, emitter)).or_insert(Gp { value: start, target });
        e.target = target;
    }

    /// Sets a state group's state (global). It moves to the new state's values over the Init bank's transition time.
    pub fn set_state(&mut self, group: &str, state: &str) {
        self.set_state_id(wwise_id(group), wwise_id(state));
    }

    /// `set_state` by ids (for the groups and states no known name hashes to).
    pub fn set_state_id(&mut self, group: Id, state: Id) {
        let cur = self.states.get(&group).map(|s| s.current).unwrap_or_else(state_none);
        if cur == state {
            return;
        }
        let from = self.states.get(&group).map(|s| s.weights(self.now)).unwrap_or_else(|| vec![(state_none(), 1.0)]);
        let dur = self
            .bank
            .file
            .game_syncs
            .state_groups
            .get(&group)
            .map(|g| g.transitions.iter().find(|t| t.from_.id == cur && t.to.id == state).map(|t| t.time_ms).unwrap_or(g.default_transition_ms))
            .unwrap_or(0) as f64
            / 1000.0;
        self.states.insert(group, StateNow { current: state, from, start: self.now, dur });
    }

    /// Sets a switch group's value on an emitter.
    pub fn set_switch(&mut self, group: &str, value: &str, emitter: u32) {
        self.set_switch_id(wwise_id(group), wwise_id(value), emitter);
    }

    /// `set_switch` by ids.
    pub fn set_switch_id(&mut self, group: Id, value: Id, emitter: u32) {
        self.switches.insert((group, emitter), value);
    }

    /// A game parameter's value now (after its ramp) on an emitter, or the global value.
    pub fn game_parameter(&self, name: &str, emitter: Option<u32>) -> f32 {
        self.game_parameter_id(wwise_id(name), emitter)
    }

    /// `game_parameter` by id.
    pub fn game_parameter_id(&self, id: Id, emitter: Option<u32>) -> f32 {
        match emitter {
            Some(e) => self.gp_value(id, e) as f32,
            None => self.gps.get(&(id, None)).map(|g| g.value).unwrap_or_else(|| self.gp_def(id).map(|d| d.default).unwrap_or(0.0)) as f32,
        }
    }

    /// `set_game_parameter` by id.
    pub fn set_game_parameter_id(&mut self, id: Id, value: f32, emitter: Option<u32>) {
        if value.is_finite() {
            self.set_gp(id, f64::from(value), emitter, None);
        }
    }

    /// Plays an object directly, as a Play action without fade would (tests).
    #[cfg(test)]
    pub(super) fn play_object(&mut self, obj: Id, emitter: u32) {
        self.spawn(obj, emitter, None, None, None);
    }

    /// What a node's state groups add to its volume now (tests).
    #[cfg(test)]
    pub(super) fn states_db(&self, node: Id) -> f64 {
        let mut m = Mix::default();
        if let Some(n) = self.bank.node(node) {
            self.add_states(&n.states, &mut m);
        }
        m.db
    }

    /// A blend track's crossfade gain on a child now (tests).
    #[cfg(test)]
    pub(super) fn crossfade_gain(&self, container: Id, child: Id, emitter: u32) -> f64 {
        self.layer_gain(container, child, emitter, &mut Mix::default())
    }

    /// Where an emitter is, from the listener (metres, the listener's axes: x right, y up, -z ahead). It drives
    /// attenuation, Distance and priority, and makes its voices placed ([`VoiceUpdate::place`]). A position that is not
    /// finite is ignored.
    pub fn set_emitter(&mut self, emitter: u32, position_from_listener: [f32; 3]) {
        if position_from_listener.iter().all(|x| x.is_finite()) {
            self.emitters.insert(emitter, position_from_listener);
        }
    }

    /// Where an emitter is from the listener, if the game placed it.
    pub fn emitter(&self, emitter: u32) -> Option<[f32; 3]> {
        self.emitters.get(&emitter).copied()
    }

    /// Runs time on by `dt` seconds: delayed actions, starts and ends, container schedules, ramps, fades. Returns one
    /// update per voice that is playing, started or stopped (by voice id).
    ///
    /// A `dt` that is not finite, or not positive, counts as 0. A step's first [`DETAIL_S`] seconds are run event by event
    /// (each start, end, schedule and delayed action at its own time); the rest of a longer step, up to [`MAX_STEP_S`] (a
    /// longer `dt` counts as that), is applied at once: voices and fades that end within it end, delayed actions falling
    /// in it run at its end, ramps and transitions move on by all of it, and a container schedules at most its next item.
    pub fn step(&mut self, dt: f32) -> &[VoiceUpdate] {
        self.updates.clear();
        let dt = if dt.is_finite() && dt > 0.0 { f64::from(dt).min(MAX_STEP_S) } else { 0.0 };
        let detail_end = self.now + dt.min(DETAIL_S);
        // Everything that falls due inside the detailed part happens at its own time, in order.
        for _ in 0..MAX_EVENTS_PER_STEP {
            let Some(t) = self.next_due(detail_end) else { break };
            self.advance_voices(t - self.now);
            self.now = t;
            self.run_due();
        }
        let end = self.now.max(detail_end) + (dt - DETAIL_S).max(0.0);
        self.advance_voices(end - self.now);
        self.now = end;
        self.update_meters();
        self.ramp_gps(dt);
        self.run_due();
        self.update_ducks();
        self.emit();
        &self.updates
    }

    // ---------------------------------------------------------------- game syncs

    fn gp_def(&self, id: Id) -> Option<&GameParameter> {
        self.bank.file.game_syncs.game_parameters.get(&id)
    }

    /// A game parameter's value on an emitter: Distance if built in, else the emitter's, else the global, else the
    /// default.
    fn gp_value(&self, id: Id, emitter: u32) -> f64 {
        let def = self.gp_def(id);
        if def.is_some_and(|d| d.built_in == 1) {
            return self.emitters.get(&emitter).map(|p| f64::from((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt())).unwrap_or(0.0);
        }
        if let Some(g) = self.gps.get(&(id, Some(emitter))) {
            return g.value;
        }
        if let Some(g) = self.gps.get(&(id, None)) {
            return g.value;
        }
        def.map(|d| d.default).unwrap_or(0.0)
    }

    fn ramp_gps(&mut self, dt: f64) {
        let now = self.now;
        let defs = &self.bank.file.game_syncs.game_parameters;
        for ((id, _), g) in self.gps.iter_mut() {
            let target = g.target.at(now);
            let (ramp, up, down) = defs.get(id).map(|d| (d.ramp, d.ramp_up, d.ramp_down)).unwrap_or((Ramp::None, 0.0, 0.0));
            let rate = if target >= g.value { up } else { down };
            g.value = match ramp {
                Ramp::None => target,
                _ if rate <= 0.0 => target,
                Ramp::SlewRate => {
                    let step = rate * dt;
                    if (target - g.value).abs() <= step {
                        target
                    } else {
                        g.value + step * (target - g.value).signum()
                    }
                }
                Ramp::FilteringOverTime => g.value + (target - g.value) * (1.0 - (-dt / rate).exp()),
            };
        }
    }

    /// What a block's states add to `mix` now: each state's deltas by its weight in the group's blend (linear over the
    /// transition time).
    fn add_states(&self, block: &StateBlock, mix: &mut Mix) {
        for g in &block.groups {
            let Some(st) = self.states.get(&g.group.id) else { continue };
            for (s, w) in st.weights(self.now) {
                if let Some(x) = g.states.iter().find(|x| x.state.id == s) {
                    for (k, v) in &x.deltas {
                        mix.add(k, v * w);
                    }
                }
            }
        }
    }

    fn add_rtpcs(&self, rtpcs: &[Rtpc], emitter: u32, mix: &mut Mix) {
        for r in rtpcs {
            if r.rtpc_type != RtpcType::GameParameter || r.accumulation != Accumulation::Additive {
                continue;
            }
            mix.add(&r.param, eval(&r.points, r.scaling, self.gp_value(r.game_parameter.id, emitter)));
        }
    }

    fn switch_value(&self, group: Id, emitter: u32) -> Option<Id> {
        if let Some(sg) = self.bank.file.game_syncs.switch_groups.get(&group) {
            if let (Some(gp), Some(curve)) = (&sg.driven_by_game_parameter, &sg.curve) {
                if !curve.is_empty() {
                    let v = self.gp_value(gp.id, emitter);
                    let pick = curve.iter().rev().find(|p| p.x <= v).unwrap_or(&curve[0]);
                    return pick.switch.as_ref().map(|s| s.id);
                }
            }
        }
        self.switches.get(&(group, emitter)).copied()
    }

    // ---------------------------------------------------------------- actions

    fn run_action(&mut self, a: &Action, emitter: u32) {
        let fade_ms = prop(&a.props, "TransitionTime");
        let curve = a.params.fade_curve.unwrap_or(Interp::Linear);
        let fade = (fade_ms / 1000.0, curve);
        let scope_emitter = match a.scope {
            Scope::GameObject | Scope::GameObjectNoTarget => Some(emitter),
            Scope::Global | Scope::GlobalAll => None,
        };
        if let Some(p) = a.props.get("Probability").and_then(PropValue::num) {
            if self.rng.unit() * 100.0 >= p {
                return;
            }
        }
        match a.kind {
            ActionKind::Play => {
                let fade_in = (fade.0 > 0.0).then_some(Fade { start: self.now, dur: fade.0, curve });
                self.spawn(a.target, emitter, None, None, fade_in);
            }
            ActionKind::PlayEvent => self.post_id(a.target, emitter),
            ActionKind::Stop => self.stop_matching(a.target, scope_emitter, fade),
            ActionKind::SetState => {
                let (g, s) = (a.params.state_group.as_ref().unwrap().id, a.params.state.as_ref().unwrap().id);
                self.set_state_id(g, s);
            }
            ActionKind::SetSwitch => {
                let (g, s) = (a.params.switch_group.as_ref().unwrap().id, a.params.switch.as_ref().unwrap().id);
                self.switches.insert((g, emitter), s);
            }
            ActionKind::SetGameParameter => {
                let v = self.value_of(a);
                let id = a.target;
                let value = match a.params.value_meaning {
                    Some(ValueMeaning::Offset) => self.gps.get(&(id, scope_emitter)).map(|g| g.target.to).unwrap_or_else(|| self.gp_value(id, emitter)) + v,
                    _ => v,
                };
                self.set_gp(id, value, scope_emitter, Some(fade));
                if a.params.bypass_transition == Some(true) {
                    if let Some(g) = self.gps.get_mut(&(id, scope_emitter)) {
                        g.value = value;
                    }
                }
            }
            ActionKind::SetVolume | ActionKind::ResetVolume | ActionKind::SetPitch | ActionKind::ResetPitch => {
                let reset = matches!(a.kind, ActionKind::ResetVolume | ActionKind::ResetPitch);
                let v = if reset { 0.0 } else { self.value_of(a) };
                let offset = !reset && a.params.value_meaning == Some(ValueMeaning::Offset);
                let now = self.now;
                let map = if matches!(a.kind, ActionKind::SetVolume | ActionKind::ResetVolume) { &mut self.volume_over } else { &mut self.pitch_over };
                let cur = map.get(&(a.target, scope_emitter)).map(|o| (o.at(now), o.to)).unwrap_or((0.0, 0.0));
                let to = if offset { cur.1 + v } else { v };
                map.insert((a.target, scope_emitter), Anim { from: cur.0, to, start: now, dur: fade.0, curve });
            }
            ActionKind::Seek => {
                let s = a.params.seek.unwrap();
                let frac = s.base + self.rng.range(s.min, s.max);
                self.seek_matching(a.target, scope_emitter, frac);
            }
        }
    }

    /// A Set action's value: its base plus a draw in its range.
    fn value_of(&mut self, a: &Action) -> f64 {
        let v = a.params.value.unwrap_or(ValueRange { base: 0.0, min: 0.0, max: 0.0 });
        v.base + self.rng.range(v.min, v.max)
    }

    fn run_due(&mut self) {
        // Delayed actions, in order.
        while let Some(entry) = self.pending.first_entry() {
            if entry.get().due > self.now {
                break;
            }
            let p = entry.remove();
            let bank = self.bank.clone();
            if let Some(Object::Action(a)) = bank.object(p.action) {
                self.run_action(a, p.emitter);
            }
        }
        let now = self.now;
        // Voices reaching their start, their end; stops running out; container schedules.
        let ids: Vec<u64> = self.insts.keys().copied().collect();
        for id in &ids {
            let Some(inst) = self.insts.get_mut(id) else { continue };
            if let Kind::Voice(v) = &mut inst.kind {
                if !v.started && v.start_at <= now {
                    v.started = true;
                    v.pos = v.offset + (now - v.start_at);
                }
            }
        }
        for id in &ids {
            let Some(inst) = self.insts.get(id) else { continue };
            if inst.stop_at.is_some_and(|t| t <= now + EPS) {
                self.remove(*id);
                continue;
            }
            if let Kind::Voice(v) = &inst.kind {
                if v.started && v.total().is_some_and(|t| v.pos >= t - EPS) {
                    self.remove(*id);
                }
            }
        }
        for id in &ids {
            if let Some(Inst { kind: Kind::RanSeq(r), stop_at: None, .. }) = self.insts.get(id) {
                if r.next_at.is_some_and(|t| t <= now) {
                    self.ranseq_next(*id);
                }
            }
        }
        self.validate_switches();
        self.collect_done();
    }

    /// The earliest moment in `(now, end]` at which something happens, if any.
    fn next_due(&self, end: f64) -> Option<f64> {
        let now = self.now;
        let mut t = f64::INFINITY;
        if let Some(p) = self.pending.values().next() {
            t = t.min(p.due);
        }
        for inst in self.insts.values() {
            if let Some(s) = inst.stop_at {
                t = t.min(s);
            }
            match &inst.kind {
                Kind::Voice(v) => {
                    if !v.started {
                        t = t.min(v.start_at);
                    } else if let Some(total) = v.total() {
                        let rate = self.voice_rate(inst);
                        if rate > 0.0 {
                            t = t.min(now + (total - v.pos).max(0.0) / rate);
                        }
                    }
                }
                Kind::RanSeq(r) => {
                    if let Some(n) = r.next_at {
                        t = t.min(n);
                    }
                }
                _ => {}
            }
        }
        (t <= end).then_some(t.max(now))
    }

    /// Moves every playing voice on by `dt` (to the step's end) at its pitch.
    fn advance_voices(&mut self, dt: f64) {
        let rates: Vec<(u64, f64)> = self.insts.iter().filter(|(_, i)| matches!(&i.kind, Kind::Voice(v) if v.started)).map(|(k, i)| (*k, self.voice_rate(i))).collect();
        for (id, rate) in rates {
            if let Some(Inst { kind: Kind::Voice(v), .. }) = self.insts.get_mut(&id) {
                v.pos += dt * rate;
            }
        }
    }

    fn voice_rate(&self, inst: &Inst) -> f64 {
        match &inst.kind {
            Kind::Voice(v) if matches!(v.out, Out::Audio(_)) => cents_to_ratio(self.mix_of(inst).cents),
            _ => 1.0,
        }
    }

    // ---------------------------------------------------------------- playing

    /// Starts an object as an instance (a Play action's target, or a container's child). None if a playback limit
    /// refuses it or it plays nothing.
    fn spawn(&mut self, obj: Id, emitter: u32, parent: Option<u64>, layer: Option<Id>, fade_in: Option<Fade>) -> Option<u64> {
        if self.path_ids(parent).len() >= MAX_DEPTH {
            return None;
        }
        let bank = self.bank.clone();
        let o = bank.object(obj)?;
        let node = o.node()?;
        let mut rolls = Rolls::default();
        rolls.add(&mut self.rng, &node.ranged);
        if parent.is_none() {
            for a in bank.chain(obj).iter().skip(1) {
                if let Some(n) = bank.node(*a) {
                    rolls.add(&mut self.rng, &n.ranged);
                }
            }
        }
        self.seq += 1;
        let id = self.seq;
        let kind = match o {
            Object::Sound(s) => {
                let out = match bank.file.source_plugins.get(&s.source_id) {
                    Some(p) if s.plugin_type == 2 => match p.name.as_str() {
                        "Wwise Silence" => Out::Silence,
                        "Wwise Motion Generator" => Out::Rumble(s.source_id),
                        _ => Out::Synth(s.source_id),
                    },
                    _ => Out::Audio(s.source_id),
                };
                let dur = match out {
                    Out::Audio(src) => bank.file.media.get(&src).map(Media::duration),
                    _ => {
                        let p = &bank.file.source_plugins[&s.source_id].params;
                        let f = |k: &str| p.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
                        Some(match out {
                            Out::Silence => (f("fDuration") + self.rng.range(f("fRandomizedLengthMinus"), f("fRandomizedLengthPlus"))).max(0.0),
                            Out::Rumble(_) => f("length_s"),
                            _ if p.get("eGenMode").and_then(serde_json::Value::as_str) == Some("FIX") => f("fFixDur"),
                            _ => f("fAttackDur") + f("fDecayDur") + f("fSustainDur") + f("fReleaseDur"),
                        })
                    }
                };
                let loops = match prop_opt(&node.props, "Loop") {
                    Some(n) if n <= 0.0 => None,
                    Some(n) => Some(n as u32),
                    None => Some(1),
                };
                // The loop region of a looping recording, unless it is the whole recording.
                let region = match out {
                    Out::Audio(src) if loops != Some(1) => bank.file.media.get(&src).and_then(|m| {
                        let [a, b] = m.loop_region?;
                        (a > 0 || b + 1 < m.samples).then_some((a, b + 1, f64::from(m.sample_rate.max(1))))
                    }),
                    _ => None,
                };
                Kind::Voice(Voice {
                    id: 0,
                    out,
                    start_at: 0.0,
                    pos: 0.0,
                    dur,
                    loops,
                    region,
                    started: false,
                    reported: false,
                    built: false,
                    offset: 0.0,
                    seek: None,
                    virt: false,
                    virt_pos: 0.0,
                })
            }
            Object::RanSeqCntr(r) => {
                let key = (obj, if r.global_ { None } else { Some(emitter) });
                if r.continuous && r.reset_playlist_each_play {
                    self.playlists.remove(&key);
                }
                let n = r.playlist.len() as u64;
                let left = if !r.continuous {
                    Some(1)
                } else if r.loop_infinite {
                    None
                } else {
                    Some(n * u64::from(r.loop_count.max(1)))
                };
                Kind::RanSeq(RsInst { key, left, next_at: None, current: None, current_start: self.now })
            }
            Object::SwitchCntr(_) => Kind::Switch(SwInst { value: None, kids: BTreeMap::new() }),
            Object::LayerCntr(_) => Kind::Layer,
            _ => return None,
        };
        let inst = Inst { obj, emitter, parent, kids: Vec::new(), rolls, env: Env { fade_in, fade_out: None }, layer, stop_at: None, kind };
        if !self.admit(&inst) {
            return None;
        }
        self.insts.insert(id, inst);
        if let Some(p) = parent {
            if let Some(pi) = self.insts.get_mut(&p) {
                pi.kids.push(id);
            }
        }
        match o {
            Object::Sound(_) => {
                let delay = {
                    let i = &self.insts[&id];
                    let m = self.mix_of(i);
                    (m.delay + self.path_rolls(id).delay).max(0.0)
                };
                self.next_voice += 1;
                let vid = self.next_voice - 1;
                let now = self.now;
                if let Some(Inst { kind: Kind::Voice(v), .. }) = self.insts.get_mut(&id) {
                    v.id = vid;
                    v.start_at = now + delay;
                    if delay <= 0.0 {
                        v.started = true;
                    }
                }
            }
            Object::RanSeqCntr(_) => self.ranseq_next(id),
            Object::SwitchCntr(s) => {
                let value = self.switch_now(s, emitter);
                if let Some(Inst { kind: Kind::Switch(sw), .. }) = self.insts.get_mut(&id) {
                    sw.value = value;
                }
                for n in switch_nodes(s, value) {
                    if let Some(k) = self.spawn(n, emitter, Some(id), None, None) {
                        if let Some(Inst { kind: Kind::Switch(sw), .. }) = self.insts.get_mut(&id) {
                            sw.kids.insert(n, k);
                        }
                    }
                }
            }
            Object::LayerCntr(l) => {
                for c in &l.children {
                    self.spawn(*c, emitter, Some(id), Some(obj), None);
                }
            }
            _ => {}
        }
        Some(id)
    }

    fn switch_now(&self, s: &SwitchCntr, emitter: u32) -> Option<Id> {
        let v = match s.group_type {
            GroupType::Switch => self.switch_value(s.group.id, emitter),
            GroupType::State => {
                let st = self.states.get(&s.group.id).map(|x| x.current);
                st.filter(|x| *x != state_none())
            }
        };
        v.or_else(|| s.default.as_ref().map(|d| d.id))
    }

    /// The playback limits that apply to an instance: its own object's, and for a voice those of its actor-mixer ancestors
    /// that are not playing instances above it (up to the first that ignores its parent's limit) and of its buses.
    fn limits_of(&self, obj: Id, emitter: u32, parent: Option<u64>, is_voice: bool) -> Vec<Limit> {
        let bank = &self.bank;
        let path_objs: BTreeSet<Id> = self.path_ids(parent).into_iter().map(|i| self.insts[&i].obj).collect();
        let mut out = Vec::new();
        for (k, a) in bank.chain(obj).iter().enumerate() {
            let Some(n) = bank.node(*a) else { continue };
            if k == 0 || (is_voice && !path_objs.contains(a)) {
                let mut max = n.adv.max_instances;
                for r in &n.rtpc {
                    if r.param == "MaxNumInstances" && r.rtpc_type == RtpcType::GameParameter {
                        max = eval(&r.points, r.scaling, self.gp_value(r.game_parameter.id, emitter)).max(0.0) as u32;
                    }
                }
                if max > 0 {
                    out.push(Limit { node: Some(*a), bus: None, own: k == 0, max, kill_newest: n.adv.kill_newest, virtualise: n.adv.use_virtual_behavior });
                }
            }
            // "Ignore parent": what plays under this node does not count against the limits above it.
            if n.adv.ignore_parent_max_instances {
                break;
            }
        }
        if is_voice {
            if let Some(b) = self.output_bus(obj) {
                for bid in bank.bus_chain(b) {
                    if let Some(bus) = bank.bus(*bid) {
                        if bus.max_instances > 0 {
                            out.push(Limit { node: None, bus: Some(*bid), own: false, max: bus.max_instances, kill_newest: bus.kill_newest, virtualise: bus.use_virtual_behavior });
                        }
                    }
                }
            }
        }
        out
    }

    /// The instances a limit counts (not stopping), oldest first.
    fn holders(&self, l: &Limit) -> Vec<u64> {
        let bank = &self.bank;
        self.insts
            .iter()
            .filter(|(_, i)| i.stop_at.is_none())
            .filter(|(_, i)| match (l.node, l.bus) {
                (Some(n), _) if l.own => i.obj == n,
                (Some(n), _) => {
                    matches!(i.kind, Kind::Voice(_)) && {
                        let ch = bank.chain(i.obj);
                        ch.iter().position(|x| *x == n).is_some_and(|at| !ch[..at].iter().any(|x| bank.node(*x).is_some_and(|nn| nn.adv.ignore_parent_max_instances)))
                    }
                }
                (None, Some(b)) => matches!(i.kind, Kind::Voice(_)) && self.output_bus(i.obj).is_some_and(|ob| bank.bus_chain(ob).contains(&b)),
                _ => false,
            })
            .map(|(k, _)| *k)
            .collect()
    }

    /// Holders ranked from the one kept first to the one given up first: higher priority first; among equals the oldest
    /// first if the limit discards the newest, else the newest first.
    fn ranked(&self, l: &Limit, mut holders: Vec<u64>) -> Vec<u64> {
        let pri: BTreeMap<u64, f64> = holders.iter().map(|h| (*h, self.priority(&self.insts[h]))).collect();
        holders.sort_by(|a, b| pri[b].partial_cmp(&pri[a]).unwrap_or(std::cmp::Ordering::Equal).then(if l.kill_newest { a.cmp(b) } else { b.cmp(a) }));
        holders
    }

    /// The playback limits on the way to a new instance: a limit that kills makes room by stopping the instance ranked
    /// last, or refuses the newcomer if it ranks last itself; a limit that uses virtual voices admits it (the voices beyond
    /// the limit are made virtual at each update, [`Self::limited_virtual`]).
    fn admit(&mut self, inst: &Inst) -> bool {
        let is_voice = matches!(inst.kind, Kind::Voice(_));
        let new_pri = self.priority(inst);
        for l in self.limits_of(inst.obj, inst.emitter, inst.parent, is_voice) {
            if l.virtualise {
                continue;
            }
            let holders = self.holders(&l);
            if (holders.len() as u32) < l.max {
                continue;
            }
            let ranked = self.ranked(&l, holders);
            let Some(&last) = ranked.last() else { continue };
            let lp = self.priority(&self.insts[&last]);
            // The newcomer is the newest: it goes before an equal if the newest is discarded.
            if lp < new_pri || (lp == new_pri && !l.kill_newest) {
                self.remove(last);
            } else {
                return false;
            }
        }
        true
    }

    /// The voices beyond a limit that uses virtual voices: every voice under an instance ranked past the limit's count.
    fn limited_virtual(&self) -> BTreeSet<u64> {
        let mut out = BTreeSet::new();
        let mut done: BTreeSet<(Option<Id>, Option<Id>, bool)> = BTreeSet::new();
        let voices: Vec<u64> = self.insts.iter().filter(|(_, i)| matches!(i.kind, Kind::Voice(_))).map(|(k, _)| *k).collect();
        for v in &voices {
            for path_inst in self.path_ids(Some(*v)) {
                let pi = &self.insts[&path_inst];
                let is_voice = path_inst == *v;
                for l in self.limits_of(pi.obj, pi.emitter, pi.parent, is_voice).into_iter().filter(|l| l.virtualise) {
                    if !l.own && !is_voice {
                        continue;
                    }
                    if !done.insert((l.node, l.bus, l.own)) {
                        continue;
                    }
                    let ranked = self.ranked(&l, self.holders(&l));
                    for over in ranked.iter().skip(l.max as usize) {
                        out.insert(*over);
                    }
                }
            }
        }
        // A container instance beyond its limit makes every voice under it virtual.
        voices.into_iter().filter(|v| self.path_ids(Some(*v)).iter().any(|p| out.contains(p))).collect()
    }

    /// An instance's priority: the nearest priority setting on its chain, its curves, the distance offset.
    fn priority(&self, inst: &Inst) -> f64 {
        let bank = &self.bank;
        let chain = bank.chain(inst.obj);
        let from = chain.iter().position(|a| bank.node(*a).is_some_and(|n| n.priority_override_parent)).unwrap_or(chain.len().saturating_sub(1));
        let Some(n) = chain.get(from).and_then(|a| bank.node(*a)) else { return 50.0 };
        let mut p = prop_opt(&n.props, "Priority").unwrap_or(50.0);
        if n.priority_apply_dist_factor {
            let off = prop(&n.props, "PriorityDistanceOffset");
            if let Some(att) = self.attenuation(inst.obj) {
                let max_d = att_max_distance(att);
                if max_d > 0.0 {
                    p += off * (self.distance(inst.emitter) / max_d).min(1.0);
                }
            }
        }
        p + self.mix_of(inst).priority_add
    }

    fn ranseq_next(&mut self, id: u64) {
        let bank = self.bank.clone();
        let Some(inst) = self.insts.get(&id).cloned() else { return };
        let Some(Object::RanSeqCntr(r)) = bank.object(inst.obj) else { return };
        let Kind::RanSeq(rs) = &inst.kind else { return };
        if rs.left == Some(0) || inst.stop_at.is_some() || r.playlist.is_empty() {
            if let Some(Inst { kind: Kind::RanSeq(rs), .. }) = self.insts.get_mut(&id) {
                rs.next_at = None;
            }
            return;
        }
        let key = rs.key;
        let emitter = inst.emitter;
        let prev = rs.current;
        let t = self.transition_time(&inst, r);
        let cross = matches!(r.transition_mode, TransitionMode::CrossFadeAmp | TransitionMode::CrossFadePower) && prev.is_some_and(|p| self.insts.contains_key(&p));
        let (fin, fout) = match r.transition_mode {
            TransitionMode::CrossFadePower => (Interp::Sine, Interp::SineRecip),
            _ => (Interp::Linear, Interp::Linear),
        };
        let pick = self.pick(r, key);
        let child = r.playlist[pick].id;
        let now = self.now;
        if cross {
            if let Some(p) = prev.and_then(|p| self.insts.get_mut(&p)) {
                p.env.fade_out = Some(Fade { start: now, dur: t, curve: fout });
                p.stop_at = Some(now + t);
            }
        }
        let fade_in = cross.then_some(Fade { start: now, dur: t, curve: fin });
        let kid = self.spawn(child, emitter, Some(id), None, fade_in);
        let kid_total = kid.and_then(|k| self.insts.get(&k)).and_then(|k| match &k.kind {
            Kind::Voice(v) => v.total().map(|tot| (k.clone(), tot, v.start_at)),
            _ => None,
        });
        let next_at = if !r.continuous {
            None
        } else {
            match r.transition_mode {
                TransitionMode::TriggerRate => Some(now + t.max(MIN_TRIGGER_S)),
                TransitionMode::CrossFadeAmp | TransitionMode::CrossFadePower => kid_total.map(|(k, tot, start)| {
                    let rate = self.voice_rate(&k).max(1e-6);
                    (start + tot / rate - t).max(now)
                }),
                _ => None,
            }
        };
        if let Some(Inst { kind: Kind::RanSeq(rs), .. }) = self.insts.get_mut(&id) {
            rs.current = kid;
            rs.current_start = now;
            rs.left = rs.left.map(|l| l - 1);
            rs.next_at = if rs.left == Some(0) { None } else { next_at };
        }
    }

    /// A continuous container's transition time now (s): its time, the random modifier, the curve on it.
    fn transition_time(&mut self, inst: &Inst, r: &RanSeqCntr) -> f64 {
        let mut m = Mix::default();
        self.add_rtpcs(&r.node.rtpc, inst.emitter, &mut m);
        let t = (r.transition_time_ms + self.rng.range(r.transition_time_mod[0], r.transition_time_mod[1])) / 1000.0 + m.transition_add;
        t.max(0.0)
    }

    /// The next playlist item of a random / sequence container (index into its playlist).
    fn pick(&mut self, r: &RanSeqCntr, key: (Id, Option<u32>)) -> usize {
        let n = r.playlist.len();
        let mut pl = self.playlists.remove(&key).unwrap_or_default();
        let i = match r.mode {
            RsMode::Sequence => {
                let i = pl.next % n;
                pl.next = (i + 1) % n;
                i
            }
            RsMode::Random => {
                let avoid = (r.avoid_repeat_count as usize).min(n.saturating_sub(1));
                let recent: Vec<usize> = pl.history.iter().rev().take(avoid).copied().collect();
                let mut pool: Vec<usize> = if r.random_mode == RandomMode::Shuffle {
                    if pl.bag.is_empty() {
                        pl.bag = (0..n).collect();
                    }
                    pl.bag.clone()
                } else {
                    (0..n).collect()
                };
                let fresh: Vec<usize> = pool.iter().copied().filter(|i| !recent.contains(i)).collect();
                if !fresh.is_empty() {
                    pool = fresh;
                }
                let weight = |i: usize| if r.using_weight { f64::from(r.playlist[i].weight) } else { 1.0 };
                let total: f64 = pool.iter().map(|&i| weight(i)).sum();
                let mut x = self.rng.unit() * total;
                let mut chosen = pool[pool.len() - 1];
                for &i in &pool {
                    x -= weight(i);
                    if x < 0.0 {
                        chosen = i;
                        break;
                    }
                }
                pl.bag.retain(|&b| b != chosen);
                pl.history.push(chosen);
                if pl.history.len() > n.max(1) {
                    pl.history.remove(0);
                }
                chosen
            }
        };
        self.playlists.insert(key, pl);
        i
    }

    /// Continuous switch containers follow their switch: children leaving stop with their fade, children arriving start.
    fn validate_switches(&mut self) {
        let bank = self.bank.clone();
        let ids: Vec<u64> = self.insts.iter().filter(|(_, i)| matches!(i.kind, Kind::Switch(_)) && i.stop_at.is_none()).map(|(k, _)| *k).collect();
        for id in ids {
            let (obj, emitter, old) = match self.insts.get(&id) {
                Some(Inst { obj, emitter, kind: Kind::Switch(sw), .. }) => (*obj, *emitter, sw.value),
                _ => continue,
            };
            let Some(Object::SwitchCntr(s)) = bank.object(obj) else { continue };
            if !s.continuous_validation {
                continue;
            }
            let value = self.switch_now(s, emitter);
            if value == old {
                continue;
            }
            let new_nodes = switch_nodes(s, value);
            let kids = match self.insts.get(&id) {
                Some(Inst { kind: Kind::Switch(sw), .. }) => sw.kids.clone(),
                _ => continue,
            };
            let now = self.now;
            for (n, k) in &kids {
                let p = switch_param(s, *n);
                let keep = new_nodes.contains(n) && p.is_some_and(|p| p.continue_playback);
                if !keep && self.insts.contains_key(k) {
                    let dur = p.map(|p| p.fade_out_ms).unwrap_or(0) as f64 / 1000.0;
                    self.stop_inst(*k, (dur, Interp::Linear));
                }
            }
            let mut next_kids = BTreeMap::new();
            for n in new_nodes {
                let p = switch_param(s, n);
                if let Some(k) = kids.get(&n).filter(|k| self.insts.contains_key(k) && self.insts[k].stop_at.is_none()) {
                    next_kids.insert(n, *k);
                    continue;
                }
                if p.is_some_and(|p| p.first_only) {
                    continue;
                }
                let dur = p.map(|p| p.fade_in_ms).unwrap_or(0) as f64 / 1000.0;
                let fade = (dur > 0.0).then_some(Fade { start: now, dur, curve: Interp::Linear });
                if let Some(k) = self.spawn(n, emitter, Some(id), None, fade) {
                    next_kids.insert(n, k);
                }
            }
            if let Some(Inst { kind: Kind::Switch(sw), .. }) = self.insts.get_mut(&id) {
                sw.value = value;
                sw.kids = next_kids;
            }
        }
    }

    /// Instances with this object on their actor-mixer chain (and on this emitter, if given) begin to stop.
    fn stop_matching(&mut self, target: Id, emitter: Option<u32>, fade: (f64, Interp)) {
        let bank = self.bank.clone();
        let hits: Vec<u64> = self
            .insts
            .iter()
            .filter(|(_, i)| emitter.is_none_or(|e| i.emitter == e) && bank.chain(i.obj).contains(&target))
            .filter(|(_, i)| !i.parent.is_some_and(|p| self.insts.get(&p).is_some_and(|pi| bank.chain(pi.obj).contains(&target))))
            .map(|(k, _)| *k)
            .collect();
        for h in hits {
            self.stop_inst(h, fade);
        }
    }

    fn stop_inst(&mut self, id: u64, fade: (f64, Interp)) {
        let now = self.now;
        let Some(i) = self.insts.get_mut(&id) else { return };
        if fade.0 <= 0.0 {
            self.remove(id);
            return;
        }
        let end = now + fade.0;
        if i.stop_at.is_some_and(|s| s <= end) {
            return;
        }
        i.env.fade_out = Some(Fade { start: now, dur: fade.0, curve: fade.1 });
        i.stop_at = Some(end);
        if let Kind::RanSeq(r) = &mut i.kind {
            r.next_at = None;
        }
    }

    fn seek_matching(&mut self, target: Id, emitter: Option<u32>, frac: f64) {
        let bank = self.bank.clone();
        for i in self.insts.values_mut() {
            if emitter.is_some_and(|e| i.emitter != e) || !bank.chain(i.obj).contains(&target) {
                continue;
            }
            if let Kind::Voice(v) = &mut i.kind {
                let Some(d) = v.dur else { continue };
                let p = (frac.clamp(0.0, 1.0) * d).min(d);
                if v.built {
                    v.seek = Some(p);
                    v.pos = p;
                } else {
                    v.offset = p;
                    if v.started {
                        v.pos = p;
                    }
                }
            }
        }
    }

    /// Removes an instance and everything below it (children first, while the path above them still stands); every
    /// voice that started owes a stop update (one that never reached an update also owes its start).
    fn remove(&mut self, id: u64) {
        let Some(kids) = self.insts.get(&id).map(|i| i.kids.clone()) else { return };
        for k in kids {
            self.remove(k);
        }
        let premix = match self.insts.get(&id) {
            Some(Inst { kind: Kind::Voice(v), .. }) if v.started && !v.reported => Some(self.voice_mix(id)),
            _ => None,
        };
        let Some(inst) = self.insts.remove(&id) else { return };
        if let Some(p) = inst.parent {
            if let Some(pi) = self.insts.get_mut(&p) {
                pi.kids.retain(|k| *k != id);
            }
            self.child_ended(p, id);
        }
        if matches!(&inst.kind, Kind::Voice(v) if v.started) {
            self.ended.push(Ended { inst, premix });
        }
    }

    /// A container's child is gone: a continuous sequence without overlap starts its next item.
    fn child_ended(&mut self, parent: u64, kid: u64) {
        let bank = self.bank.clone();
        let Some(p) = self.insts.get(&parent) else { return };
        if p.stop_at.is_some() {
            return;
        }
        let Kind::RanSeq(rs) = &p.kind else { return };
        if rs.current != Some(kid) || rs.left == Some(0) || rs.next_at.is_some() {
            return;
        }
        let Some(Object::RanSeqCntr(r)) = bank.object(p.obj) else { return };
        if !r.continuous {
            return;
        }
        let p = p.clone();
        let now = self.now;
        match r.transition_mode {
            TransitionMode::Delay => {
                let t = self.transition_time(&p, r);
                if let Some(Inst { kind: Kind::RanSeq(rs), .. }) = self.insts.get_mut(&parent) {
                    rs.current = None;
                    rs.next_at = Some(now + t);
                }
            }
            TransitionMode::TriggerRate => {}
            _ => {
                if let Some(Inst { kind: Kind::RanSeq(rs), .. }) = self.insts.get_mut(&parent) {
                    rs.current = None;
                    rs.next_at = Some(now);
                }
            }
        }
    }

    /// A continuous switch container that is playing and not stopping, whose switch value now assigns nothing it could play:
    /// it waits for the switch to change (Wwise's Continuous play mode plays the new value's object "as soon as a new
    /// switch/state is detected"; wwiser names the flag `bIsContinuousValidation`). Once what it played has run out on its
    /// own under a value that does assign something, it is done (the public sources do not say; otherwise every one-shot
    /// posted would leave a container waiting for ever).
    fn waiting(&self, inst: &Inst) -> bool {
        let Kind::Switch(sw) = &inst.kind else { return false };
        let Some(Object::SwitchCntr(s)) = self.bank.object(inst.obj) else { return false };
        let entry = sw.value.and_then(|v| s.switches.iter().find(|e| e.value.id == v));
        s.continuous_validation && inst.stop_at.is_none() && entry.is_none_or(|e| e.nodes.iter().all(|n| self.bank.node(*n).is_none()))
    }

    /// Containers with nothing playing and nothing to come are done.
    fn collect_done(&mut self) {
        loop {
            let done: Vec<u64> = self
                .insts
                .iter()
                .filter(|(_, i)| {
                    i.kids.is_empty()
                        && match &i.kind {
                            Kind::Voice(_) => false,
                            Kind::RanSeq(r) => r.next_at.is_none(),
                            Kind::Switch(_) => !self.waiting(i),
                            Kind::Layer => true,
                        }
                })
                .map(|(k, _)| *k)
                .collect();
            if done.is_empty() {
                break;
            }
            for d in done {
                self.remove(d);
            }
        }
    }

    // ---------------------------------------------------------------- mixing

    fn path_ids(&self, mut at: Option<u64>) -> Vec<u64> {
        let mut out = Vec::new();
        while let Some(i) = at {
            out.push(i);
            at = self.insts.get(&i).and_then(|x| x.parent);
        }
        out
    }

    fn path_rolls(&self, id: u64) -> Rolls {
        self.path_ids(Some(id)).iter().fold(Rolls::default(), |acc, i| acc.plus(&self.insts[i].rolls))
    }

    fn distance(&self, emitter: u32) -> f64 {
        self.emitters.get(&emitter).map(|p| f64::from((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt())).unwrap_or(0.0)
    }

    fn output_bus(&self, obj: Id) -> Option<Id> {
        self.bank.output_bus(obj)
    }

    fn positioning(&self, obj: Id) -> Option<&Positioning> {
        self.bank.positioning(obj)
    }

    /// A sound's way to the output (made once a sound and kept): its route ([`SoundBank::route`]), positioned in space when
    /// its positioning in force routes relative to the listener with a 3D spatialisation (Wwise's dispatch,
    /// `FUN_144aac180`: otherwise the 2D panner), and its attenuation's curves.
    pub fn placement_of(&mut self, obj: Id) -> Arc<Placement> {
        if let Some(p) = self.placements.get(&obj) {
            return p.clone();
        }
        let pan = self.positioning(obj).is_some_and(|p| p.listener_relative_routing && p.spatialization.as_deref().is_some_and(|s| s != "None"));
        let p = Arc::new(Placement::routed(self.bank.route(obj), pan, self.attenuation(obj)));
        self.placements.insert(obj, p.clone());
        p
    }

    /// The placement a voice on `emitter` hands its player to place it by: when the game placed the emitter and the
    /// sound's positioning in force is 3D on the emitter's position (listener-relative routing, position type Emitter),
    /// panned or attenuated. A path automation's voice and a 2D sound's are not placed (they play as at the listener).
    fn placed(&mut self, obj: Id, emitter: u32) -> Option<Arc<Placement>> {
        let on_emitter = self.positioning(obj).is_some_and(|p| p.listener_relative_routing && p.position_3d_type == "Emitter");
        let p = self.placement_of(obj);
        (self.emitters.contains_key(&emitter) && on_emitter && (p.pan || self.attenuation(obj).is_some())).then_some(p)
    }

    /// The gain (dB) a voice's level takes after each effect slot (0-3) of each bus on its way, its output bus first: that
    /// bus's later slots' output levels, its bus volume, and everything of the buses above. For the check that the buses'
    /// compressors, which are not built, would not act at the voices' levels (T5d-7: a compressor's input is the voice's
    /// output less this). Empty for a voice that is not playing.
    pub fn gains_after_slots(&self, voice: u32) -> Vec<(Id, [f64; 4])> {
        let Some(inst) = self.insts.values().find(|i| matches!(&i.kind, Kind::Voice(v) if v.id == voice)) else { return Vec::new() };
        let Some(b) = self.output_bus(inst.obj) else { return Vec::new() };
        let stages = self.fx_now(inst);
        let on = |bus: Id, from: u32| stages.iter().filter(|(st, _)| st.on_bus && st.holder == bus && st.slot >= from).filter_map(|(_, n)| n.map(|n| n.gain_db)).sum::<f64>();
        let mut above = 0.0;
        let mut out: Vec<(Id, [f64; 4])> = Vec::new();
        for bid in self.bank.bus_chain(b).iter().rev() {
            let (pre, post) = self.bus_mix(*bid, inst.emitter);
            out.push((*bid, [1, 2, 3, 4].map(|s| on(*bid, s) + post + above)));
            above += pre.db + post + on(*bid, 0);
        }
        out.reverse();
        out
    }

    /// A voice's recording's channel count and its own gain (`akd_gain_2`, 1 without it or for a plug-in's voice).
    fn media_of(&self, out: Out) -> (usize, f32) {
        let Out::Audio(src) = out else { return (1, 1.0) };
        self.bank.file.media.get(&src).map_or((1, 1.0), |m| (m.channels.max(1) as usize, m.akd_gain_2.filter(|g| g.is_finite() && *g > 0.0).unwrap_or(1.0) as f32))
    }

    /// The attenuation that applies: the nearest AttenuationID on the chain, if the positioning in force is 3D with
    /// attenuation on.
    fn attenuation(&self, obj: Id) -> Option<&Attenuation> {
        let bank = &self.bank;
        let chain = bank.chain(obj);
        if self.positioning(obj)?.enable_attenuation != Some(true) {
            return None;
        }
        chain.iter().filter_map(|a| bank.node(*a)).find_map(|n| match n.props.get("AttenuationID") {
            Some(PropValue::Ref(r)) => match bank.object(r.id) {
                Some(Object::Attenuation(a)) => Some(a),
                _ => None,
            },
            _ => None,
        })
    }

    /// An instance's parameters now, without the randomisers of its instance path (those are added per voice).
    fn mix_of(&self, inst: &Inst) -> Mix {
        let bank = &self.bank;
        let mut m = Mix::default();
        let e = inst.emitter;
        for a in bank.chain(inst.obj) {
            let Some(n) = bank.node(*a) else { continue };
            for (k, v) in &n.props {
                if matches!(k.as_str(), "Volume" | "MakeUpGain" | "BusVolume" | "OutputBusVolume" | "Pitch" | "LPF" | "HPF" | "OutputBusLPF" | "OutputBusHPF" | "InitialDelay") {
                    m.add(k, v.num().unwrap_or(0.0));
                }
            }
            self.add_rtpcs(&n.rtpc, e, &mut m);
            self.add_states(&n.states, &mut m);
            for (map, vol) in [(&self.volume_over, true), (&self.pitch_over, false)] {
                for key in [(*a, None), (*a, Some(e))] {
                    if let Some(o) = map.get(&key) {
                        if vol {
                            m.db += o.at(self.now);
                        } else {
                            m.cents += o.at(self.now);
                        }
                    }
                }
            }
        }
        m
    }

    /// Everything one voice's level, pitch and filters add up to now: (mix, linear fade and crossfade factor).
    fn voice_mix(&self, id: u64) -> (Mix, f64) {
        self.voice_mix_fx(id, None)
    }

    /// `voice_mix`, and the voice's effects into `fx` if given ([`VoiceUpdate::fx`]).
    fn voice_mix_fx(&self, id: u64, fx: Option<&mut Vec<FxNow>>) -> (Mix, f64) {
        let bank = &self.bank;
        let inst = &self.insts[&id];
        let mut m = self.mix_of(inst);
        let rolls = self.path_rolls(id);
        m.db += rolls.volume;
        m.cents += rolls.pitch;
        m.lpf += rolls.lpf;
        m.hpf += rolls.hpf;
        let mut fade = 1.0;
        for p in self.path_ids(Some(id)) {
            let pi = &self.insts[&p];
            fade *= pi.env.level(self.now);
            if let Some(lc) = pi.layer {
                fade *= self.layer_gain(lc, pi.obj, inst.emitter, &mut m);
            }
        }
        if let Some(att) = self.attenuation(inst.obj) {
            let d = self.distance(inst.emitter);
            let use_curve = |k: &str| att.curve_to_use.get(k).copied().filter(|i| *i >= 0).map(|i| &att.curves[i as usize]);
            if let Some(c) = use_curve("VolumeDry") {
                m.att_db = eval(&c.points, c.scaling, d);
                m.db += m.att_db;
            }
            if let Some(c) = use_curve("LowPassFilter") {
                m.att_lpf = eval(&c.points, c.scaling, d);
                m.lpf += m.att_lpf;
            }
            if let Some(c) = use_curve("HighPassFilter") {
                m.att_hpf = eval(&c.points, c.scaling, d);
                m.hpf += m.att_hpf;
            }
        }
        m.db += self.normalisation_db(inst);
        // The effects' output levels that are plain gains (linear, so a voice's share of it is the whole of it).
        let stages = self.fx_now(inst);
        m.db += stages.iter().filter_map(|(_, e)| e.as_ref().map(|e| e.gain_db)).sum::<f64>();
        if let Some(out) = fx {
            out.clear();
            out.extend(stages.iter().map(|(_, e)| e.map_or(FxNow::Off, |e| e.fx)));
        }
        if let Some(b) = self.output_bus(inst.obj) {
            for bid in bank.bus_chain(b) {
                let (pre, post) = self.bus_mix(*bid, inst.emitter);
                m.db += pre.db + post;
                m.cents += pre.cents;
                m.lpf += pre.lpf;
                m.hpf += pre.hpf;
            }
        }
        (m, fade)
    }

    /// The effects on an instance's way (a voice's), each with what it does now, or None if bypassed.
    fn fx_now(&self, inst: &Inst) -> Vec<(FxStage, Option<StageNow>)> {
        if !matches!(inst.kind, Kind::Voice(_)) {
            return Vec::new();
        }
        self.bank.fx_stages(inst.obj).iter().map(|st| (*st, (!self.fx_bypassed(st, inst.emitter)).then(|| self.stage_now(st, inst.emitter)))).collect()
    }

    /// A slot bypassed now: by the bank's bits, or by the holder's `BypassFX<slot>` / `BypassAllFX` properties, curves and
    /// states (a sum of at least 0.5: the data's curves and deltas are 0 or 1, a state transition blends them).
    fn fx_bypassed(&self, st: &FxStage, emitter: u32) -> bool {
        const SLOT: [&str; 4] = ["BypassFX0", "BypassFX1", "BypassFX2", "BypassFX3"];
        let (fx, props, rtpc, states) = match self.bank.object(st.holder) {
            Some(Object::Bus(b)) => (&b.fx, &b.props, &b.rtpc, &b.states),
            Some(o) => match o.node() {
                Some(n) => (&n.fx, &n.props, &n.rtpc, &n.states),
                None => return false,
            },
            None => return false,
        };
        if fx.bypassed(st.slot) {
            return true;
        }
        let keys = [SLOT[st.slot.min(3) as usize], "BypassAllFX"];
        let mut v: f64 = keys.iter().map(|k| prop(props, k)).sum();
        for r in rtpc.iter().filter(|r| r.rtpc_type == RtpcType::GameParameter && keys.contains(&r.param.as_str())) {
            v += eval(&r.points, r.scaling, self.gp_value(r.game_parameter.id, emitter));
        }
        for g in &states.groups {
            let Some(now) = self.states.get(&g.group.id) else { continue };
            for (sid, w) in now.weights(self.now) {
                if let Some(x) = g.states.iter().find(|x| x.state.id == sid) {
                    v += keys.iter().filter_map(|k| x.deltas.get(*k)).sum::<f64>() * w;
                }
            }
        }
        v >= 0.5
    }

    /// A plug-in's parameters now: each its block value (`base`), or its curves' (an exclusive curve replaces it, additive
    /// ones add to it), and whether a curve sets it (such a value goes through the plug-in's `SetParam`, its clamps).
    /// (A Modulator curve on an effect's parameter is not read: modulators are not built, and such an effect is NOT EXACT:
    /// `SoundBank::effects_not_exact` lists them, and the game logs each when it reads the data.)
    fn plugin_params<const N: usize>(&self, e: &Effect, base: [f64; N], emitter: u32) -> ([f64; N], [bool; N]) {
        let (mut exclusive, mut added, mut set) = ([None::<f64>; N], [0.0f64; N], [false; N]);
        for r in e.rtpc.iter().filter(|r| r.rtpc_type == RtpcType::GameParameter && (r.param_id as usize) < N) {
            let p = r.param_id as usize;
            let y = eval(&r.points, r.scaling, self.gp_value(r.game_parameter.id, emitter));
            set[p] = true;
            match r.accumulation {
                Accumulation::Exclusive => exclusive[p] = Some(y),
                _ => added[p] += y,
            }
        }
        (std::array::from_fn(|p| exclusive[p].unwrap_or(base[p] + added[p])), set)
    }

    /// What an effect does now ([`StageNow`]).
    fn stage_now(&self, st: &FxStage, emitter: u32) -> StageNow {
        let Some(e) = self.bank.file.effects.get(&st.effect) else { return StageNow { fx: FxNow::Off, gain_db: 0.0 } };
        if let Some(eq) = &e.eq {
            let (bands, out) = self.eq_now(e, eq, emitter);
            StageNow { fx: FxNow::Eq(bands), gain_db: out }
        } else if let Some(d) = &e.distortion {
            StageNow { fx: FxNow::Distortion(self.distortion_now(e, d, emitter)), gain_db: 0.0 }
        } else if let Some(l) = &e.limiter {
            StageNow { fx: FxNow::Off, gain_db: f64::from(self.limiter_params(e, l, emitter).output_db) }
        } else {
            StageNow { fx: FxNow::Off, gain_db: 0.0 }
        }
    }

    /// A Parametric EQ's bands and output level (dB) now; a parameter a curve sets goes through the plug-in's `SetParam`
    /// (`0x144a10630`): the type truncated to an integer, the gains and the output level held to -24..24 dB, on/off from
    /// a value other than 0 (one that is not a number is off: `ucomiss` with 0, then `jz`, at 0x144a106e9); frequencies
    /// and Qs as they are.
    fn eq_now(&self, e: &Effect, eq: &ParamEq, emitter: u32) -> ([EqNow; 3], f64) {
        let mut base = [0.0f64; 17];
        for (k, b) in eq.bands.iter().take(3).enumerate() {
            base[k * 5..k * 5 + 5].copy_from_slice(&[b.filter as u8 as f64, b.gain_db, b.frequency, b.q, if b.on { 1.0 } else { 0.0 }]);
        }
        base[EQ_PARAM_OUTPUT_LEVEL as usize] = eq.output_level_db;
        base[EQ_PARAM_LFE as usize] = if eq.process_lfe { 1.0 } else { 0.0 };
        let (value, set) = self.plugin_params(e, base, emitter);
        let level = |p: usize| if set[p] { held(value[p], -24.0, 24.0) } else { value[p] };
        let mut bands = [EqNow::OFF; 3];
        for (k, b) in bands.iter_mut().enumerate() {
            let p = k * 5;
            let filter = if set[p] { EqFilter::from_index(value[p] as i64) } else { eq.bands.get(k).map(|b| b.filter) };
            let on = if set[p + 4] { switch(value[p + 4]) } else { eq.bands.get(k).is_some_and(|b| b.on) };
            *b = EqNow { on, filter, gain_db: level(p + 1) as f32, frequency: value[p + 2] as f32, q: value[p + 3] as f32 };
        }
        (bands, level(EQ_PARAM_OUTPUT_LEVEL as usize))
    }

    /// A Guitar Distortion's parameters now; a parameter a curve sets goes through its `SetParam` (`0x144a04b60`): a band's
    /// type truncated to an integer (`FUN_144a05110`'s design type), its gain held to -48..48 dB (`DAT_1457dc0f4`,
    /// `DAT_1457dc0f0`), on/off from a value other than 0 (not a number: off, `ucomiss` / `jz` at 0x144a04be3); drive, tone,
    /// rectification and the wet share held to 0-100 (`DAT_144dec67c`); the output level held to -24..24 dB (`DAT_1457db4a8`,
    /// `DAT_145220f4c`). The output level becomes `powf(10, level / 20)` whether a curve sets it or the block does
    /// (`SetParamsBlock` `0x144a04f50` converts it too).
    fn distortion_now(&self, e: &Effect, d: &GuitarDistortion, emitter: u32) -> DistortionNow {
        let stored: Vec<&EqBand> = d.pre_eq.iter().chain(&d.post_eq).collect();
        let at = |k: usize| (k / 3) * 30 + (k % 3) * 10;
        let mut base = [0.0f64; 66];
        for (k, b) in stored.iter().take(6).enumerate() {
            base[at(k)..at(k) + 5].copy_from_slice(&[design_of(b.filter), b.gain_db, b.frequency, b.q, if b.on { 1.0 } else { 0.0 }]);
        }
        base[GD_PARAM_TYPE as usize] = f64::from(d.distortion);
        base[GD_PARAM_DRIVE as usize] = d.drive;
        base[GD_PARAM_TONE as usize] = d.tone;
        base[GD_PARAM_RECTIFICATION as usize] = d.rectification;
        base[GD_PARAM_OUTPUT_LEVEL as usize] = d.output_level_db;
        base[GD_PARAM_WET as usize] = d.wet_dry;
        let (value, set) = self.plugin_params(e, base, emitter);
        let pct = |p: u32| (if set[p as usize] { held(value[p as usize], 0.0, 100.0) } else { value[p as usize] }) as f32;
        let band = |k: usize| {
            let p = at(k);
            let b = stored.get(k);
            EqNow {
                on: if set[p + 4] { switch(value[p + 4]) } else { b.is_some_and(|b| b.on) },
                filter: if set[p] { EqFilter::from_design(value[p] as i64) } else { b.map(|b| b.filter) },
                gain_db: (if set[p + 1] { held(value[p + 1], -48.0, 48.0) } else { value[p + 1] }) as f32,
                frequency: value[p + 2] as f32,
                q: value[p + 3] as f32,
            }
        };
        let lp = GD_PARAM_OUTPUT_LEVEL as usize;
        let level_db = (if set[lp] { held(value[lp], -24.0, 24.0) } else { value[lp] }) as f32;
        DistortionNow {
            pre: std::array::from_fn(band),
            post: std::array::from_fn(|k| band(k + 3)),
            kind: d.distortion,
            drive: pct(GD_PARAM_DRIVE),
            tone: pct(GD_PARAM_TONE),
            rectification: pct(GD_PARAM_RECTIFICATION),
            level: 10f32.powf(level_db * 0.05),
            wet: pct(GD_PARAM_WET),
        }
    }

    /// A Peak Limiter's threshold and output level (dB) now; set by a curve, its `SetParam` (`0x144a12330`) holds the
    /// threshold to -96.3..0 dB (`DAT_1457db3f8`) and the output level to -24..24 dB. The output level is a gain after the
    /// limiting: its `Execute` (`0x144a108d0`) runs the limiter's routine on the buffer first, then `FUN_1449f4310` ramps
    /// the buffer from the last buffer's output gain to this one's.
    fn limiter_params(&self, e: &Effect, l: &PeakLimiter, emitter: u32) -> LimiterNow {
        let base = [l.threshold_db, l.ratio, l.lookahead_s, l.release_s, l.output_level_db, f64::from(u8::from(l.process_lfe)), f64::from(u8::from(l.channel_link))];
        let (value, set) = self.plugin_params(e, base, emitter);
        let (t, o) = (LIMITER_PARAM_THRESHOLD as usize, LIMITER_PARAM_OUTPUT_LEVEL as usize);
        LimiterNow {
            threshold_db: (if set[t] { held(value[t], -96.3, 0.0) } else { value[t] }) as f32,
            output_db: (if set[o] { held(value[o], -24.0, 24.0) } else { value[o] }) as f32,
        }
    }

    /// The master bus's Peak Limiter now, or None if the master holds none or it is bypassed (for the player's count of
    /// samples over its threshold). Its curves read the global values: a bus's effect works on the bus's mix.
    pub fn master_limiter(&self) -> Option<LimiterNow> {
        let (master, bus) = self.bank.file.objects.iter().find_map(|(id, o)| match o {
            Object::Bus(b) if b.parent_bus.is_none() && b.fx.fx.iter().any(|s| s.fx_id.as_ref().is_some_and(|r| self.bank.file.effects.get(&r.id).is_some_and(|e| e.limiter.is_some()))) => Some((*id, b)),
            _ => None,
        })?;
        bus.fx.fx.iter().find_map(|s| {
            let fx = s.fx_id.as_ref()?.id;
            let e = self.bank.file.effects.get(&fx)?;
            let l = e.limiter.as_ref()?;
            let st = FxStage { holder: master, slot: s.index, effect: fx, on_bus: true };
            (!self.fx_bypassed(&st, NO_EMITTER)).then(|| self.limiter_params(e, l, NO_EMITTER))
        })
    }

    /// What loudness normalisation adds to a voice (dB): the recording's `akd ` gain where the node in force normalises.
    /// The node in force is the nearest on the chain that overrides its parent's analysis settings (wwiser's
    /// `bOverrideAnalysis`), else the top one; its `bNormalizeLoudness` switches the gain on. Make-up gain is a property of
    /// its own, added as every volume is (in `mix_of`).
    fn normalisation_db(&self, inst: &Inst) -> f64 {
        let Kind::Voice(Voice { out: Out::Audio(src), .. }) = &inst.kind else { return 0.0 };
        let chain = self.bank.chain(inst.obj);
        let n = chain.iter().filter_map(|a| self.bank.node(*a)).find(|n| n.adv.override_analysis).or_else(|| chain.last().and_then(|a| self.bank.node(*a)));
        match (n.is_some_and(|n| n.adv.normalize_loudness), self.bank.file.media.get(src).and_then(|m| m.loudness_gain)) {
            (true, Some(g)) if g > 0.0 && g.is_finite() => gain_to_db(g),
            _ => 0.0,
        }
    }

    /// One bus's share of a voice going through it: what applies to the voice as it enters (its voice volume, pitch and
    /// filters, their curves and states: `pre`) and the bus volume applied to the bus's mix after its effects (BusVolume,
    /// OutputBusVolume and ducking: `post`, dB).
    fn bus_mix(&self, bid: Id, emitter: u32) -> (Mix, f64) {
        let bank = &self.bank;
        let (mut pre, mut all) = (Mix::default(), Mix::default());
        let Some(bus) = bank.bus(bid) else { return (pre, 0.0) };
        for (k, v) in &bus.props {
            if matches!(k.as_str(), "Volume" | "BusVolume" | "Pitch" | "LPF" | "HPF" | "OutputBusVolume" | "OutputBusLPF" | "OutputBusHPF") {
                all.add(k, v.num().unwrap_or(0.0));
                if !matches!(k.as_str(), "BusVolume" | "OutputBusVolume") {
                    pre.add(k, v.num().unwrap_or(0.0));
                }
            }
        }
        for r in &bus.rtpc {
            if r.rtpc_type == RtpcType::GameParameter && r.accumulation == Accumulation::Additive {
                let v = eval(&r.points, r.scaling, self.gp_value(r.game_parameter.id, emitter));
                all.add(&r.param, v);
                if !matches!(r.param.as_str(), "BusVolume" | "OutputBusVolume") {
                    pre.add(&r.param, v);
                }
            }
        }
        let mut st = Mix::default();
        self.add_states(&bus.states, &mut st);
        all.db += st.db;
        let ducked: f64 = self.ducks.iter().filter(|((src, i), _)| bank.bus(*src).is_some_and(|s| s.ducks[*i].bus.id == bid)).map(|(_, d)| d.level.at(self.now)).sum();
        let post = all.db - pre.db + ducked.max(bus.max_duck_volume_db);
        // (A state's deltas are not told apart by kind here: they count with the bus volume.)
        pre.cents += st.cents;
        pre.lpf += st.lpf;
        pre.hpf += st.hpf;
        (pre, post)
    }

    /// A blend container's tracks on one child: the product of its crossfade curves (by their game parameters), and the
    /// tracks' own curves added to `m`.
    fn layer_gain(&self, lc: Id, child: Id, emitter: u32, m: &mut Mix) -> f64 {
        let Some(Object::LayerCntr(l)) = self.bank.object(lc) else { return 1.0 };
        let mut g = 1.0;
        for layer in &l.layers {
            for assoc in layer.associations.iter().filter(|a| a.child == child) {
                if let Some(gp) = &layer.crossfade_game_parameter {
                    if !assoc.crossfade.is_empty() {
                        g *= find(&assoc.crossfade, self.gp_value(gp.id, emitter)).max(0.0);
                    }
                }
                self.add_rtpcs(&layer.rtpc, emitter, m);
            }
        }
        g
    }

    /// The Meter effects (`SoundFile::meters`): each RMS meter's level on its bus, written into its game parameter (which
    /// then ramps by the Init bank's rule, as any value set). The level is worked out from what plays, not from a signal:
    /// every voice going through the bus counts with its gain as it reaches the meter's slot (everything down the chain up
    /// to the bus's voice volume; the bus volume, ducking and the buses beyond come after the slot unless
    /// `apply_downstream_volume`) times its recording's RMS (`Media::rms`, the loop region's for a voice that loops), the
    /// voices summed as powers; nothing playing reads the meter's floor. Built for RMS meters without ballistics (attack,
    /// release and hold all 0: the level is taken as it is); the others (Peak meters, and meters with ballistics, whose
    /// formulas are not public) are kept and leave their game parameter at its value. Not built either: the effects in the
    /// slots before a meter (a parametric EQ before both built ones).
    fn update_meters(&mut self) {
        let bank = self.bank.clone();
        for meter in bank.file.meters.iter().filter(|m| meter_built(m)) {
            let mut power: BTreeMap<Option<u32>, f64> = BTreeMap::new();
            if meter.scope == MeterScope::GameObject {
                for e in self.insts.values().map(|i| i.emitter) {
                    power.entry(Some(e)).or_insert(0.0);
                }
            } else {
                power.insert(None, 0.0);
            }
            for (id, inst) in &self.insts {
                let Kind::Voice(v) = &inst.kind else { continue };
                let (Out::Audio(src), true, false) = (v.out, v.started, v.virt) else { continue };
                let Some(chain) = self.output_bus(inst.obj).map(|b| bank.bus_chain(b)) else { continue };
                let Some(at) = chain.iter().position(|b| *b == meter.bus) else { continue };
                let Some(media) = bank.file.media.get(&src) else { continue };
                let rms = if v.loops != Some(1) && v.region.is_some() { media.loop_rms.or(media.rms) } else { media.rms };
                let Some(rms) = rms else { continue };
                let (m, fade) = self.voice_mix(*id);
                let mut db = m.db;
                // (The output levels of effects after the meter's slot: later slots of its bus, and the buses beyond.)
                for (st, e) in self.fx_now(inst) {
                    let after = st.on_bus && chain.iter().position(|b| *b == st.holder).is_some_and(|k| k > at || (k == at && st.slot > meter.slot));
                    if let (true, Some(e)) = (after, e) {
                        db -= e.gain_db;
                    }
                }
                if !meter.apply_downstream_volume {
                    db -= self.bus_mix(meter.bus, inst.emitter).1;
                    for b in &chain[at + 1..] {
                        let (pre, post) = self.bus_mix(*b, inst.emitter);
                        db -= pre.db + post;
                    }
                }
                let a = db_to_gain(db) * fade * rms;
                *power.entry(if meter.scope == MeterScope::GameObject { Some(inst.emitter) } else { None }).or_insert(0.0) += a * a;
            }
            for (who, p) in power {
                let level = if p > 0.0 { 10.0 * p.log10() } else { meter.min_db };
                let level = level.clamp(meter.min_db.min(meter.max_db), meter.max_db.max(meter.min_db));
                self.set_gp(meter.game_parameter, level, who, None);
            }
        }
    }

    fn update_ducks(&mut self) {
        let bank = self.bank.clone();
        let now = self.now;
        let mut active: BTreeSet<Id> = BTreeSet::new();
        for i in self.insts.values() {
            if let Kind::Voice(v) = &i.kind {
                if v.started && i.stop_at.is_none() {
                    if let Some(b) = self.output_bus(i.obj) {
                        active.extend(bank.bus_chain(b).iter().copied());
                    }
                }
            }
        }
        for (id, o) in &bank.file.objects {
            let Object::Bus(bus) = o else { continue };
            for (k, d) in bus.ducks.iter().enumerate() {
                let on = active.contains(id);
                let st = self.ducks.entry((*id, k)).or_insert(DuckState { level: Anim::fixed(0.0), on: false, off_since: now });
                if on && !st.on {
                    let from = st.level.at(now);
                    st.level = Anim { from, to: d.duck_volume_db, start: now, dur: d.fade_out_ms as f64 / 1000.0, curve: d.fade_curve };
                    st.on = true;
                } else if !on && st.on {
                    st.on = false;
                    st.off_since = now;
                }
                if !st.on && st.level.to != 0.0 && now - st.off_since >= bus.recovery_time_ms as f64 / 1000.0 {
                    let from = st.level.at(now);
                    st.level = Anim { from, to: 0.0, start: now, dur: d.fade_in_ms as f64 / 1000.0, curve: d.fade_curve };
                }
                if !st.on && st.level.to == 0.0 && st.level.done(now) {
                    self.ducks.remove(&(*id, k));
                }
            }
        }
    }

    /// The virtual voice behaviour in force: the nearest node that overrides it, else the root.
    fn below_threshold(&self, obj: Id) -> (BelowThreshold, u32) {
        let bank = &self.bank;
        let chain = bank.chain(obj);
        let n = chain.iter().filter_map(|a| bank.node(*a)).find(|n| n.adv.virtual_voice_opt_override_parent).or_else(|| chain.last().and_then(|a| bank.node(*a)));
        n.map(|n| (n.adv.below_threshold_behavior, n.adv.virtual_queue_behavior)).unwrap_or((BelowThreshold::ContinueToPlay, 1))
    }

    fn emit(&mut self) {
        let threshold = self.bank.file.game_syncs.volume_threshold_db;
        let ids: Vec<u64> = self.insts.iter().filter(|(_, i)| matches!(&i.kind, Kind::Voice(v) if v.started && v.out != Out::Silence)).map(|(k, _)| *k).collect();
        let mut kills = Vec::new();
        let over_limit = self.limited_virtual();
        let mut fx = Vec::new();
        for id in ids {
            let (obj, emitter) = (self.insts[&id].obj, self.insts[&id].emitter);
            let place = self.placed(obj, emitter);
            let routed = self.placement_of(obj);
            let (m, fade) = self.voice_mix_fx(id, Some(&mut fx));
            let level_db = m.db + gain_to_db(fade);
            let (behaviour, queue) = self.below_threshold(self.insts[&id].obj);
            let inst = &self.insts[&id];
            let Kind::Voice(v) = &inst.kind else { continue };
            let below = level_db < threshold;
            let looping = v.loops != Some(1);
            let mut virt = false;
            match behaviour {
                BelowThreshold::ContinueToPlay => {}
                BelowThreshold::KillVoice if below => kills.push(id),
                BelowThreshold::KillIfOneShotElseVirtual if below && !looping => kills.push(id),
                BelowThreshold::KillIfOneShotElseVirtual | BelowThreshold::SetAsVirtualVoice => virt = below,
                BelowThreshold::KillVoice => {}
            }
            if kills.last() == Some(&id) {
                continue;
            }
            virt |= over_limit.contains(&id);
            let position = self.emitters.get(&emitter).copied();
            let (channels, media_gain) = self.media_of(v.out);
            let Some(out) = output(v.out) else { continue };
            let Some(Inst { kind: Kind::Voice(v), .. }) = self.insts.get_mut(&id) else { continue };
            let mut seek = v.seek.take();
            if virt && !v.virt {
                v.virt_pos = v.pos;
            } else if !virt && v.virt && v.reported {
                // Back from virtual: by the queue behaviour (0 from the beginning, 1 from the elapsed time, 2 resume).
                match queue {
                    0 => v.pos = 0.0,
                    2 => v.pos = v.virt_pos,
                    _ => {}
                }
            }
            // Heard and not held by the player: built now, from where it is (a seek asked meanwhile is where it is).
            let start = (!virt && !v.built).then(|| {
                seek = None;
                let (offset, plays) = v.place(v.pos);
                VoiceStart { offset: offset as f32, looping: v.loops != Some(1), loop_count: plays, loop_region: v.region.map(|(a, b, _)| (a, b)) }
            });
            v.built = !virt;
            v.virt = virt;
            v.reported = true;
            if virt {
                seek = None;
            }
            // (A placed voice's player applies the attenuation's shares itself.)
            let (db, lowpass, highpass) = if place.is_some() { (m.db - m.att_db, m.lpf - m.att_lpf, m.hpf - m.att_hpf) } else { (m.db, m.lpf.clamp(0.0, 100.0), m.hpf.clamp(0.0, 100.0)) };
            let gain = db_to_gain(db) * fade;
            self.updates.push(VoiceUpdate {
                voice: v.id,
                output: out,
                emitter,
                position,
                place,
                matrix: routed.listener_matrix(channels),
                media_gain,
                start,
                stop: false,
                seek_to: seek.map(|s| s as f32),
                gain: gain as f32,
                volume_db: db as f32,
                fade: fade as f32,
                pitch: cents_to_ratio(m.cents) as f32,
                pitch_cents: m.cents as f32,
                lowpass: lowpass as f32,
                highpass: highpass as f32,
                lowpass_hz: filter::cutoff(m.lpf.clamp(0.0, 100.0) as f32, false, OUTPUT_RATE),
                highpass_hz: filter::cutoff(m.hpf.clamp(0.0, 100.0) as f32, true, OUTPUT_RATE),
                virtual_voice: virt,
                fx: if virt { Vec::new() } else { fx.clone() },
            });
        }
        for k in kills {
            self.remove(k);
        }
        self.collect_done();
        let ended = std::mem::take(&mut self.ended);
        let gone: BTreeSet<u32> = ended.iter().filter_map(|e| if let Kind::Voice(v) = &e.inst.kind { Some(v.id) } else { None }).collect();
        if !gone.is_empty() {
            self.updates.retain(|u| !gone.contains(&u.voice));
        }
        for Ended { inst, premix } in ended {
            let Kind::Voice(v) = &inst.kind else { continue };
            let Some(out) = output(v.out) else { continue };
            let (channels, media_gain) = self.media_of(v.out);
            let matrix = self.placement_of(inst.obj).listener_matrix(channels);
            let (m, fade) = premix.unwrap_or_default();
            self.updates.push(VoiceUpdate {
                voice: v.id,
                output: out,
                emitter: inst.emitter,
                position: self.emitters.get(&inst.emitter).copied(),
                place: None,
                matrix,
                media_gain,
                start: None,
                stop: true,
                seek_to: None,
                gain: if premix.is_some() { (db_to_gain(m.db) * fade) as f32 } else { 0.0 },
                volume_db: m.db as f32,
                fade: fade as f32,
                pitch: cents_to_ratio(m.cents) as f32,
                pitch_cents: m.cents as f32,
                lowpass: m.lpf.clamp(0.0, 100.0) as f32,
                highpass: m.hpf.clamp(0.0, 100.0) as f32,
                lowpass_hz: filter::cutoff(m.lpf.clamp(0.0, 100.0) as f32, false, OUTPUT_RATE),
                highpass_hz: filter::cutoff(m.hpf.clamp(0.0, 100.0) as f32, true, OUTPUT_RATE),
                virtual_voice: false,
                fx: Vec::new(),
            });
        }
        self.updates.sort_by_key(|u| u.voice);
    }
}

/// A playback limit: on a node (`own`: counting that node's instances, else the voices under it) or on a bus.
#[derive(Clone, Copy, Debug)]
struct Limit {
    node: Option<Id>,
    bus: Option<Id>,
    own: bool,
    max: u32,
    kill_newest: bool,
    /// Beyond the limit: virtual voices instead of killing.
    virtualise: bool,
}

/// A voice gone: its last state, and its mix if it never reached an update.
struct Ended {
    inst: Inst,
    premix: Option<(Mix, f64)>,
}

/// Floating tolerance on times (s).
const EPS: f64 = 1e-6;

/// The output sample rate the updates' cutoffs are given at (Hz): the sound engine's in Elite's exe at rest
/// (`DAT_145ee0a4c` = 48,000 in its data section). A player at another rate works the cutoff out again with
/// [`super::filter::cutoff`].
pub const OUTPUT_RATE: f32 = 48000.0;

/// How much of one step is run event by event (s): see [`Mixer::step`].
pub const DETAIL_S: f64 = 1.0;

/// The longest step (s): a longer `dt` counts as this.
pub const MAX_STEP_S: f64 = 3600.0;

/// The shortest trigger-rate interval (s): a curve can take the data's 0 ms rates to nothing, which would start items
/// without end. Wwise's own floor is not public; 10 ms is shorter than any rate the data sets (67 ms).
pub const MIN_TRIGGER_S: f64 = 0.01;

/// A guard on the events one step runs (with [`MIN_TRIGGER_S`] and [`DETAIL_S`] a step needs far fewer).
const MAX_EVENTS_PER_STEP: usize = 20_000;

/// Deepest nesting of events posting events and of containers inside containers (the load check refuses cycles; this
/// keeps every walk finite whatever the data).
const MAX_DEPTH: usize = 64;

fn output(o: Out) -> Option<Output> {
    match o {
        Out::Audio(s) => Some(Output::Audio { source: s }),
        Out::Rumble(s) => Some(Output::Rumble { source: s }),
        Out::Synth(s) => Some(Output::Synth { source: s }),
        Out::Silence => None,
    }
}

/// No emitter: a game parameter read at its global value.
const NO_EMITTER: u32 = u32::MAX;

/// A plug-in `SetParam`'s clamp as Elite's copies compile it (`comiss` with the low bound, `jc` to it): under `lo`, or not a
/// number, is `lo`; else at most `hi`.
fn held(v: f64, lo: f64, hi: f64) -> f64 {
    if v >= lo { v.min(hi) } else { lo }
}

/// A plug-in's on / off parameter: on for a value other than 0; one that is not a number is off (`ucomiss` with 0 sets the
/// zero flag on an unordered compare, and the `jz` takes it as 0).
fn switch(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

/// A filter's design type of `FUN_144a05110` ([`EqFilter::from_design`]'s inverse), as a parameter value.
fn design_of(f: EqFilter) -> f64 {
    match f {
        EqFilter::LowShelf => 0.0,
        EqFilter::PeakingEq => 1.0,
        EqFilter::HiShelf => 2.0,
        EqFilter::LowPass => 3.0,
        EqFilter::HiPass => 4.0,
        EqFilter::BandPass => 5.0,
        EqFilter::Notch => 6.0,
    }
}

fn prop(p: &Props, k: &str) -> f64 {
    prop_opt(p, k).unwrap_or(0.0)
}

fn prop_opt(p: &Props, k: &str) -> Option<f64> {
    p.get(k).and_then(PropValue::num)
}

fn switch_nodes(s: &SwitchCntr, value: Option<Id>) -> Vec<Id> {
    value.and_then(|v| s.switches.iter().find(|e| e.value.id == v)).map(|e| e.nodes.clone()).unwrap_or_default()
}

/// The meters [`Mixer`] works out (`update_meters`).
pub(super) fn meter_built(m: &Meter) -> bool {
    m.mode == MeterMode::Rms && m.attack == 0.0 && m.release == 0.0 && m.hold == 0.0
}

fn switch_param(s: &SwitchCntr, node: Id) -> Option<&SwitchParam> {
    s.switch_params.iter().find(|p| p.node == node)
}

/// The attenuation's reach: the last point of its dry volume curve.
fn att_max_distance(a: &Attenuation) -> f64 {
    a.curve_to_use.get("VolumeDry").copied().filter(|i| *i >= 0).and_then(|i| a.curves.get(i as usize)).and_then(|c| c.points.last()).map(|p| p.x).unwrap_or(0.0)
}
