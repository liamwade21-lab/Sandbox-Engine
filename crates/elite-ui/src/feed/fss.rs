//! The Full Spectrum System scanner screen, `fsshud` (task UI7; `re/elite/notes/ui_messages_screens.md` section 7, the exe
//! report `re/elite/work/ui/msg_screens/exe_fss_saa_landing.md` part 1, the data `messages/data/fss_signal_classes.json`
//! and `fss_scan_constants.json`; grades there). The movie is a renderer of what the exe sends (it handles no key itself:
//! notes 7.5), so the feeder is the exe's FSS controller (`FUN_14139e4b0` and its producers) for what the mod has.
//!
//! - Life: `Open` when the screen opens and `Close` when it closes (the generic movie class's, notes section 1: `open`
//!   shows it, `close` hides it and says `UIClosed`). Each opening is a fresh controller: the system's name once, then
//!   everything again.
//! - Sent, by the exe's rules: `SetSystemName` once; `SetState` on change (0 INACTIVE without the discovery scan, 2 ACTIVE
//!   with it; never 1, the camera's entry, or 3, a zoomed body: the mod has no FSS camera and no zoom), and with each state
//!   `SetControls` (its rows by state, exe report 1.2.10), `SetExtendedTargetInfo []`, `SetGlitchAnimationActive false` and
//!   `SetDiscoveredAndMappedBy` with two empty strings (what the exe sends with no target, 1.2.8, 1.2.9);
//!   `SetScanProgress` on change (the discovered bodies' share by weight, 7.7.6: nothing is ever discovered in the mod, so
//!   0); `SetHistogramData` on change (200 buckets, a Gaussian per undiscovered body, `(v / max)^0.3`, 7.7.3); in state 2
//!   with a tuning value, `SetFrequency` on change, `SetSignalDataAtFrequency` when the tuned bodies change and
//!   `SetSignalDetails` every frame (7.3, 7.7.1).
//! - Not sent (each needs what the mod has not): `SetZoomData` (the overview's record and temperature are not decoded,
//!   E: the zoom scale stays hidden), `SetSignalData` (the sky's signal lines round the aim: the FSS camera is not built),
//!   `SetScannedObjectMarkerLocations` (markers are discovered bodies: none; the exe sends it only when the marker list
//!   changes, and it starts empty), `SetTargetInfo` (state 3 only).
//! - A body's place on the spectrum: its archetype is its planet class's base `StellarForge_<class>` (no volcanism,
//!   atmosphere, `_Green` or `_Ring` suffix: the mod's classes do not carry them, E), its frequency
//!   `band_lo + (band_hi - band_lo) r1` and amplitude `amp_min + (amp_max - amp_min) r2` with r1, r2 Elite's minstd
//!   generator (48271, 2^31 - 1, scaled by 2^-31), seeded by a hash of the mod's body id: Elite hashes its own body id
//!   first (`FUN_141354870`), by a function the notes do not decode, so the hash is SplitMix64's finaliser (`seed_of`, E);
//!   the id being the mod's own is the stated stand-in.

use super::{changed, Movie, Outbox};
use crate::screen_facts::{BodyKind, FssBody, FssFacts};
use crate::Value;

const MOVIE: Movie = Movie::FssHud;

/// The spectrum's buckets and the Gaussian's width (notes 7.7.3, A).
const BUCKETS: usize = 200;
const SIGMA: f64 = 0.002;
/// The highest frequency a body is bucketed at (A).
const FREQ_CLAMP: f64 = 0.9999;
/// The movie's histogram shape (`out = (v / max)^0.3`, exe report 1.2.3, A).
const SHAPE: f64 = 0.3;
/// A source is tuned within this of its band (`FUN_1413999a0`, A; that the band is the source's archetype's is B).
const TUNED_TOLERANCE: f64 = 0.015;
/// Elite's minstd generator (A).
const MINSTD_A: u64 = 48_271;
const MINSTD_M: u64 = 2_147_483_647;
const MINSTD_SCALE: f64 = 1.0 / 2_147_483_648.0;

/// The FSS's states (`enums/State.as`).
const STATE_INACTIVE: i32 = 0;
const STATE_ACTIVE: i32 = 2;
const STATE_SHOWING_BODY: i32 = 3;

/// One of the 12 named signal classes (notes 7.7.1, A: `fss_signal_classes.json` `classes`; `CLASSES[i]` is class i + 1,
/// class 0 has no record and is never tuned): its name key, its band (the union of its archetypes'), its colour as the
/// movie gets it (0xRRGGBB) and the HUD's icon type (3 the scenario class, 4 the USS class, else 0).
struct Class {
    key: &'static str,
    lo: f64,
    hi: f64,
    colour: u32,
    hud_type: i32,
}

#[rustfmt::skip]
const CLASSES: [Class; 12] = [
    Class { key: "$FSS_signal_type_AsteroidCluster;", lo: 0.32, hi: 0.35, colour: 0xFFFFFF, hud_type: 0 },
    Class { key: "$FSS_signal_type_MetalRich;", lo: 0.36, hi: 0.392, colour: 0x7F7F7F, hud_type: 0 },
    Class { key: "$FSS_signal_type_HighMetalContent;", lo: 0.392, hi: 0.424, colour: 0x7F7F7F, hud_type: 0 },
    Class { key: "$FSS_signal_type_Rocky;", lo: 0.424, hi: 0.456, colour: 0x7F7F7F, hud_type: 0 },
    Class { key: "$FSS_signal_type_Icy;", lo: 0.456, hi: 0.488, colour: 0x00FFFF, hud_type: 0 },
    Class { key: "$FSS_signal_type_RockyIcy;", lo: 0.488, hi: 0.52, colour: 0x00FFFF, hud_type: 0 },
    Class { key: "$FSS_signal_type_EarthLike;", lo: 0.52, hi: 0.55, colour: 0x00FF00, hud_type: 0 },
    Class { key: "$FSS_signal_type_AmmoniaWorld;", lo: 0.55, hi: 0.58, colour: 0x7F7F7F, hud_type: 0 },
    Class { key: "$FSS_signal_type_WaterWorld;", lo: 0.58, hi: 0.61, colour: 0x0000FF, hud_type: 0 },
    Class { key: "$FSS_signal_type_Jovian;", lo: 0.61, hi: 0.975, colour: 0xFF7F00, hud_type: 0 },
    Class { key: "$FSS_signal_type_Scenario;", lo: 0.1, hi: 0.296, colour: 0xFFFFFF, hud_type: 3 },
    Class { key: "$FSS_signal_type_USS;", lo: 0.0, hi: 0.096, colour: 0xFFFFFF, hud_type: 4 },
];

/// A planet class's base archetype (`StellarForge_<class>`, A: `fss_signal_classes.json` `entries`, the names by
/// `fss_scan_constants.json` `planet_class_to_name`): its band, amplitude range and pattern (row 0..2, position).
struct Archetype {
    lo: f64,
    hi: f64,
    amp_min: f64,
    amp_max: f64,
    pattern: &'static [(usize, f64)],
}

/// The base archetypes by Elite's planet class, 0..18.
#[rustfmt::skip]
const ARCHETYPES: [Archetype; 19] = [
    Archetype { lo: 0.52, hi: 0.55, amp_min: 0.5, amp_max: 0.5, pattern: &[(2, 0.0), (1, -0.5), (1, 0.5), (0, -0.5), (0, 0.5)] },
    Archetype { lo: 0.58, hi: 0.61, amp_min: 0.5, amp_max: 0.5, pattern: &[(2, -0.5), (2, 0.5), (1, -0.5), (1, 0.5), (0, -0.5), (0, 0.5)] },
    Archetype { lo: 0.55, hi: 0.58, amp_min: 0.5, amp_max: 0.5, pattern: &[(2, 0.0), (1, 0.0), (0, 0.5), (0, -0.5), (0, 0.5)] },
    Archetype { lo: 0.36, hi: 0.362, amp_min: 0.2, amp_max: 0.2, pattern: &[(0, 0.0), (1, 0.5), (1, -0.5), (2, 0.0)] },
    Archetype { lo: 0.392, hi: 0.394, amp_min: 0.2, amp_max: 0.2, pattern: &[(0, 0.0), (1, 0.0), (2, 0.5), (2, 0.0), (2, -0.5)] },
    Archetype { lo: 0.424, hi: 0.426, amp_min: 0.2, amp_max: 0.2, pattern: &[(0, 0.0), (1, 0.5), (1, 0.0), (1, -0.5), (2, 0.0)] },
    Archetype { lo: 0.456, hi: 0.458, amp_min: 0.2, amp_max: 0.2, pattern: &[(0, 0.0), (1, -0.5), (1, 0.5), (2, -0.5), (2, 0.5)] },
    Archetype { lo: 0.488, hi: 0.49, amp_min: 0.2, amp_max: 0.2, pattern: &[(0, -0.5), (0, 0.5), (1, 0.0), (2, -0.5), (2, 0.5)] },
    Archetype { lo: 0.61, hi: 0.63, amp_min: 0.7, amp_max: 0.7, pattern: &[(2, 0.5), (2, -0.5), (1, 0.0), (0, 0.0), (0, -0.5), (0, 0.5)] },
    Archetype { lo: 0.645, hi: 0.665, amp_min: 0.7, amp_max: 0.7, pattern: &[(2, 0.5), (2, -0.5), (1, 0.0), (1, -0.5), (1, 0.5), (0, 0.0)] },
    Archetype { lo: 0.68, hi: 0.7, amp_min: 0.7, amp_max: 0.7, pattern: &[(2, 0.5), (2, -0.5), (2, 0.0), (1, 0.0), (0, 0.5), (0, -0.5)] },
    Archetype { lo: 0.715, hi: 0.735, amp_min: 0.7, amp_max: 0.7, pattern: &[(2, 0.5), (2, -0.5), (2, 0.0), (1, 0.5), (1, -0.5), (0, 0.0)] },
    Archetype { lo: 0.735, hi: 0.75, amp_min: 0.7, amp_max: 0.7, pattern: &[(2, 0.0), (1, -0.5), (1, 0.0), (1, 0.5), (0, -0.5), (0, 0.0), (0, 0.5)] },
    Archetype { lo: 0.77, hi: 0.785, amp_min: 0.75, amp_max: 0.75, pattern: &[(2, -0.5), (2, 0.5), (1, -0.5), (1, 0.5), (0, -0.5), (0, 0.0), (0, 0.5)] },
    Archetype { lo: 0.8, hi: 0.82, amp_min: 0.8, amp_max: 0.8, pattern: &[(2, -0.5), (2, 0.5), (2, 0.0), (1, -0.5), (1, 0.5), (0, 0.5), (0, -0.5)] },
    Archetype { lo: 0.835, hi: 0.855, amp_min: 0.85, amp_max: 0.85, pattern: &[(2, -0.5), (2, 0.5), (2, 0.0), (1, -0.5), (1, 0.5), (1, 0.0), (0, 0.0)] },
    Archetype { lo: 0.87, hi: 0.89, amp_min: 0.9, amp_max: 0.9, pattern: &[(2, -0.5), (2, 0.5), (2, 0.0), (1, 0.0), (0, -0.5), (0, 0.5), (0, 0.0)] },
    Archetype { lo: 0.905, hi: 0.925, amp_min: 0.95, amp_max: 0.95, pattern: &[(2, -0.5), (2, 0.5), (1, 0.0), (1, 0.5), (1, -0.5), (0, -0.5), (0, 0.5)] },
    Archetype { lo: 0.94, hi: 0.96, amp_min: 1.0, amp_max: 1.0, pattern: &[(2, -0.5), (2, 0.5), (2, 0.0), (1, 0.5), (1, -0.5), (0, -0.5), (0, 0.5), (0, 0.0)] },
];

/// The pattern rows an archetype draws (rows 0..2: notes 7.7.2).
const PATTERN_ROWS: usize = 3;

/// A body's signal on the spectrum: its archetype, frequency and amplitude.
#[derive(Clone, Copy)]
struct Signal {
    archetype: &'static Archetype,
    freq: f64,
    amp: f64,
}

/// Elite's minstd generator's next value (seeded so it never sits at 0).
fn minstd(state: &mut u64) -> f64 {
    *state = *state * MINSTD_A % MINSTD_M;
    *state as f64 * MINSTD_SCALE
}

/// A body id's seed for the minstd generator: SplitMix64's finaliser (Steele, Lea and Flood's standard 64-bit mix; E: Elite's
/// own hash of the body id is not decoded), brought into the generator's range 1..2^31 - 2 (never 0, where it would stay).
fn seed_of(id: u64) -> u64 {
    let mut z = id.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) % (MINSTD_M - 1) + 1
}

/// The body's signal (None for a star or a class with no archetype).
fn signal(body: &FssBody) -> Option<Signal> {
    let BodyKind::Planet(class) = body.kind else { return None };
    let archetype = ARCHETYPES.get(usize::from(class))?;
    let mut state = seed_of(body.id);
    let (r1, r2) = (minstd(&mut state), minstd(&mut state));
    Some(Signal { archetype, freq: archetype.lo + (archetype.hi - archetype.lo) * r1, amp: archetype.amp_min + (archetype.amp_max - archetype.amp_min) * r2 })
}

/// The bodies' signals, in the bodies' id order.
fn signals(bodies: &[FssBody]) -> Vec<Signal> {
    let mut sorted: Vec<&FssBody> = bodies.iter().collect();
    sorted.sort_by_key(|b| b.id);
    sorted.into_iter().filter_map(signal).collect()
}

/// The spectrum as the movie gets it (notes 7.7.3): each undiscovered signal's Gaussian added to its bucket and the one
/// below (a signal in bucket 0 adds nothing), then `(v / max)^0.3` (the max taken as 1 when it is 0).
fn histogram(signals: &[Signal]) -> Vec<f64> {
    let mut v = vec![0.0; BUCKETS];
    for s in signals {
        let idx = (s.freq.min(FREQ_CLAMP) * BUCKETS as f64) as usize;
        if idx == 0 {
            continue;
        }
        for i in [idx - 1, idx] {
            let d = s.freq - (i as f64 + 0.5) / BUCKETS as f64;
            v[i] += (-d * d / (2.0 * SIGMA * SIGMA)).exp() * s.amp;
        }
    }
    let max = v.iter().copied().fold(0.0, f64::max);
    let max = if max == 0.0 { 1.0 } else { max };
    v.iter().map(|x| (x / max).powf(SHAPE)).collect()
}

/// The tuned signals' patterns (`SetSignalDataAtFrequency`'s groups, exe report 1.2.6): per signal tuned at `freq`, its
/// rows of `pos * 0.25 + 0.5`.
fn patterns_at(signals: &[Signal], freq: f64) -> Vec<Vec<Vec<f64>>> {
    signals
        .iter()
        .filter(|s| s.archetype.lo - TUNED_TOLERANCE <= freq && freq <= s.archetype.hi + TUNED_TOLERANCE)
        .map(|s| {
            let mut rows = vec![Vec::new(); PATTERN_ROWS];
            for &(row, pos) in s.archetype.pattern {
                rows[row].push(pos * 0.25 + 0.5);
            }
            rows
        })
        .collect()
}

/// `SetSignalDetails`'s object (notes 7.3, 7.7.1): the classes whose band holds `freq` (no tolerance), their names joined
/// by a space, the last one's colour, the type 3 or 4 for the scenario and USS classes; no class: no name (the movie hides
/// the panel).
fn details_at(freq: f64) -> Value {
    let tuned: Vec<&Class> = CLASSES.iter().filter(|c| c.lo <= freq && freq <= c.hi).collect();
    let tuned_type = |t: i32| tuned.iter().any(|c| c.hud_type == t);
    let hud_type = [3, 4].into_iter().find(|t| tuned_type(*t)).unwrap_or(0);
    let name = if tuned.is_empty() { Value::Null } else { tuned.iter().map(|c| c.key).collect::<Vec<_>>().join(" ").into() };
    let colour = tuned.last().map_or(0, |c| c.colour);
    Value::object([("type", Value::from(hud_type)), ("name", name), ("colour", Value::from(colour))])
}

/// A key legend row (`{name, binding}`).
fn row(name: &str, binding: &str) -> Value {
    Value::object([("name", name), ("binding", binding)])
}

/// The key legend for a state (exe report 1.2.10, A): Quit and Show help, then the zoom, stepped zoom and tuning in states
/// 2 and 3 or the discovery scan in 0 and 1, then the target in 3.
fn controls(state: i32) -> Value {
    let mut rows = vec![
        row("$ExplorationFSSQuit;", "$BINDING_input_primary:#value=ExplorationFSSQuit;"),
        row("$ExplorationFSSShowHelp;", "$BINDING_input_primary:#value=ExplorationFSSShowHelp;"),
    ];
    if state >= STATE_ACTIVE {
        rows.push(row("$FSS_zoom_tooltip;", "$FSS_zoom_tooltip_binding;"));
        rows.push(row("$FSS_minizoom_tooltip;", "$FSS_minizoom_tooltip_binding;"));
        rows.push(row("$ExplorationFSSRadioTuningX_Raw;", "$BINDING_input_primary:#value=ExplorationFSSRadioTuningX_Raw;"));
    } else {
        rows.push(row("$ExplorationFSSDiscoveryScan;", "$FSS_hold_discovery_scan;"));
    }
    if state == STATE_SHOWING_BODY {
        rows.push(row("$ExplorationFSSTarget;", "$BINDING_input_primary:#value=ExplorationFSSTarget;"));
    }
    Value::Array(rows)
}

/// The FSS screen's feeder.
#[derive(Clone, Debug, Default)]
pub struct Fss {
    /// `Open` sent and not closed since.
    opened: bool,
    named: bool,
    state: Option<i32>,
    progress: Option<f64>,
    histogram: Option<Vec<f64>>,
    frequency: Option<f64>,
    patterns: Option<Vec<Vec<Vec<f64>>>>,
}

impl Fss {
    /// One frame.
    pub fn feed(&mut self, facts: &FssFacts, out: &mut Outbox) {
        if !facts.open {
            if self.opened {
                out.send(MOVIE, "Close", vec![]);
                *self = Fss::default();
            }
            return;
        }
        if !self.opened {
            out.send(MOVIE, "Open", vec![]);
            self.opened = true;
        }
        if !self.named {
            out.send(MOVIE, "SetSystemName", vec![facts.system.clone().into()]);
            self.named = true;
        }
        let state = if facts.discovery_scan_done { STATE_ACTIVE } else { STATE_INACTIVE };
        if changed(&mut self.state, state) {
            out.send(MOVIE, "SetState", vec![state.into()]);
            out.send(MOVIE, "SetControls", vec![controls(state)]);
            out.send(MOVIE, "SetExtendedTargetInfo", vec![Value::Array(Vec::new())]);
            out.send(MOVIE, "SetGlitchAnimationActive", vec![false.into()]);
            out.send(MOVIE, "SetDiscoveredAndMappedBy", vec![Value::object([("discoveredBy", ""), ("mappedBy", "")])]);
        }
        // Nothing is discovered in the mod (the scope rule): the weighted share of discovered bodies is 0.
        let progress = 0.0;
        if changed(&mut self.progress, progress) {
            out.send(MOVIE, "SetScanProgress", vec![progress.into()]);
        }
        let signals = signals(&facts.bodies);
        let histogram = histogram(&signals);
        if changed(&mut self.histogram, histogram.clone()) {
            out.send(MOVIE, "SetHistogramData", vec![Value::array(histogram)]);
        }
        let Some(freq) = facts.tuning.map(|f| f.clamp(0.0, 1.0)) else { return };
        if state < STATE_ACTIVE {
            return;
        }
        if changed(&mut self.frequency, freq) {
            out.send(MOVIE, "SetFrequency", vec![freq.into()]);
        }
        let patterns = patterns_at(&signals, freq);
        if changed(&mut self.patterns, patterns.clone()) {
            let groups = patterns.into_iter().map(|rows| Value::array(rows.into_iter().map(Value::array)));
            // (The exe's "one more array" round the groups is the message's argument list: the handler's one argument is
            // the groups, `EnergyHistogram.setSignalDataAtFrequency(arg1:Array)` taking `arg1[group][row][k]`.)
            out.send(MOVIE, "SetSignalDataAtFrequency", vec![Value::array(groups)]);
        }
        out.send(MOVIE, "SetSignalDetails", vec![details_at(freq)]);
    }

    /// The movie has just been loaded: everything is sent again (its `Open` too) on the next feed while open.
    pub fn reset(&mut self) {
        *self = Fss::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system() -> FssFacts {
        FssFacts {
            open: true,
            system: "Ross 128".into(),
            bodies: vec![
                FssBody { id: 1, kind: BodyKind::Star },
                FssBody { id: 2, kind: BodyKind::Planet(5) },
                FssBody { id: 3, kind: BodyKind::Planet(12) },
                FssBody { id: 4, kind: BodyKind::Planet(6) },
            ],
            discovery_scan_done: true,
            tuning: None,
        }
    }

    fn names(out: &Outbox) -> Vec<&'static str> {
        out.msgs.iter().map(|m| m.name).collect()
    }

    #[test]
    fn a_bodys_signal_lies_in_its_archetypes_band() {
        for class in 0..19u8 {
            for id in [0u64, 1, 7, 1 << 40, u64::MAX] {
                let s = signal(&FssBody { id, kind: BodyKind::Planet(class) }).unwrap();
                assert!(s.archetype.lo <= s.freq && s.freq < s.archetype.hi, "{class} {id}: {}", s.freq);
                assert_eq!(s.amp, s.archetype.amp_min, "the base archetypes have one amplitude");
                assert!(CLASSES.iter().any(|c| c.lo <= s.archetype.lo && s.archetype.hi <= c.hi), "{class}: inside a class's band");
            }
        }
        assert!(signal(&FssBody { id: 1, kind: BodyKind::Star }).is_none());
        assert!(signal(&FssBody { id: 1, kind: BodyKind::Planet(19) }).is_none());
        // The same body always lands in the same place; different ones (mostly) not.
        let at = |id| signal(&FssBody { id, kind: BodyKind::Planet(12) }).unwrap().freq;
        assert_eq!(at(9), at(9));
        assert_ne!(at(9), at(10));
    }

    /// Small catalog ids spread over their whole band (with the id itself as the seed they all sat at its bottom), and a
    /// system's four gas giants of one class (the Sun's) land apart inside their band: the class's 0.015 is three of the
    /// spectrum's 200 buckets, so at most three separate peaks.
    #[test]
    fn small_ids_spread_across_their_band() {
        for class in 0..19u8 {
            let freqs: Vec<f64> = (0..200u64).map(|id| signal(&FssBody { id, kind: BodyKind::Planet(class) }).unwrap().freq).collect();
            let a = &ARCHETYPES[usize::from(class)];
            let w = a.hi - a.lo;
            let (lo, hi) = freqs.iter().fold((f64::MAX, f64::MIN), |(l, h), f| (l.min(*f), h.max(*f)));
            assert!(lo < a.lo + 0.05 * w && hi > a.hi - 0.05 * w, "class {class}: {lo}..{hi} in {}..{}", a.lo, a.hi);
            let mean = freqs.iter().sum::<f64>() / freqs.len() as f64;
            assert!((mean - (a.lo + 0.5 * w)).abs() < 0.1 * w, "class {class}: mean {mean}");
        }
        for ids in [[1u64, 2, 3, 4], [5, 6, 7, 8], [10, 20, 30, 40]] {
            let buckets: std::collections::BTreeSet<usize> = ids.iter().map(|&id| (signal(&FssBody { id, kind: BodyKind::Planet(12) }).unwrap().freq * BUCKETS as f64) as usize).collect();
            assert!(buckets.len() >= 2 && buckets.iter().all(|b| (147..150).contains(b)), "{ids:?}: buckets {buckets:?}");
        }
    }

    #[test]
    fn the_histogram_peaks_at_each_body_and_is_flat_elsewhere() {
        let sigs = signals(&system().bodies);
        assert_eq!(sigs.len(), 3, "the star has no signal");
        let h = histogram(&sigs);
        assert_eq!(h.len(), BUCKETS);
        assert!(h.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(h.contains(&1.0), "normalised by its max");
        for s in &sigs {
            let idx = (s.freq * BUCKETS as f64) as usize;
            assert!(h[idx] > 0.5, "{}: bucket {idx} is {}", s.freq, h[idx]);
        }
        let lit = h.iter().filter(|v| **v > 0.0).count();
        assert!(lit <= 2 * sigs.len(), "only each body's two buckets are lit: {lit}");
        assert!(histogram(&[]).iter().all(|v| *v == 0.0), "no bodies: flat");
    }

    #[test]
    fn opening_sends_the_screen_whole_then_only_changes() {
        let mut fss = Fss::default();
        let mut facts = system();
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        assert_eq!(
            names(&out),
            ["Open", "SetSystemName", "SetState", "SetControls", "SetExtendedTargetInfo", "SetGlitchAnimationActive", "SetDiscoveredAndMappedBy", "SetScanProgress", "SetHistogramData"]
        );
        assert!(out.msgs.iter().all(|m| m.movie == Movie::FssHud));
        assert_eq!(out.msgs[2].args, [Value::from(2)]);
        let Value::Array(rows) = &out.msgs[3].args[0] else { panic!() };
        assert_eq!(rows.len(), 5, "Quit, Show help, Zoom, Stepped zoom, Tuning");
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        assert!(out.msgs.is_empty(), "nothing changed: {:?}", names(&out));
        // The tuning: the frequency on change, the patterns when the tuned bodies change, the details every frame.
        facts.tuning = Some(0.1);
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        assert_eq!(names(&out), ["SetFrequency", "SetSignalDataAtFrequency", "SetSignalDetails"]);
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        assert_eq!(names(&out), ["SetSignalDetails"]);
        // Closing and opening again is a fresh controller.
        facts.open = false;
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        assert_eq!(names(&out), ["Close"]);
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        assert!(out.msgs.is_empty(), "closed: nothing");
        facts.open = true;
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        assert_eq!(names(&out)[..3], ["Open", "SetSystemName", "SetState"]);
    }

    #[test]
    fn without_the_discovery_scan_the_screen_asks_for_it_and_takes_no_tuning() {
        let mut fss = Fss::default();
        let facts = FssFacts { discovery_scan_done: false, tuning: Some(0.5), ..system() };
        let mut out = Outbox::default();
        fss.feed(&facts, &mut out);
        let state = out.msgs.iter().find(|m| m.name == "SetState").unwrap();
        assert_eq!(state.args, [Value::from(0)]);
        let controls = out.msgs.iter().find(|m| m.name == "SetControls").unwrap();
        let Value::Array(rows) = &controls.args[0] else { panic!() };
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].get("name"), Some(&Value::from("$ExplorationFSSDiscoveryScan;")));
        assert!(!names(&out).contains(&"SetFrequency"));
    }

    #[test]
    fn the_details_name_the_tuned_classes() {
        let d = details_at(0.43);
        assert_eq!(d.get("name"), Some(&Value::from("$FSS_signal_type_Rocky;")));
        assert_eq!(d.get("colour"), Some(&Value::from(0x7F7F7Fu32)));
        assert_eq!(d.get("type"), Some(&Value::from(0)));
        assert_eq!(details_at(0.2).get("type"), Some(&Value::from(3)));
        assert_eq!(details_at(0.05).get("type"), Some(&Value::from(4)));
        assert_eq!(details_at(0.355).get("name"), Some(&Value::Null), "the gap between clusters and metal rich bodies");
        assert_eq!(details_at(0.55).get("name"), Some(&Value::from("$FSS_signal_type_EarthLike; $FSS_signal_type_AmmoniaWorld;")));
        assert_eq!(details_at(0.55).get("colour"), Some(&Value::from(0x7F7F7Fu32)), "the last tuned class's colour");
    }

    #[test]
    fn the_patterns_are_the_tuned_bodies_combs() {
        let sigs = signals(&system().bodies);
        let rocky = sigs.iter().find(|s| s.archetype.lo == ARCHETYPES[5].lo).unwrap();
        let groups = patterns_at(&sigs, rocky.freq);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], [vec![0.5], vec![0.625, 0.5, 0.375], vec![0.5]]);
        assert!(patterns_at(&sigs, 0.05).is_empty());
    }

    /// The constants are the decoded data's: every class's band, colour and type, every base archetype's band, amplitudes
    /// and pattern (`fss_signal_classes.json`).
    #[test]
    #[ignore = "reads the UI export's data (out of git)"]
    fn the_tables_are_the_decoded_data() {
        let dir = std::env::var("ELITE_UI_EXPORT")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/elite/frea_export/sidewinder_ui/decoded").into());
        let path = format!("{dir}/messages/data/fss_signal_classes.json");
        let data: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap();
        let close = |a: f64, b: &serde_json::Value| (a - b.as_f64().unwrap()).abs() < 1e-6;
        for (i, c) in CLASSES.iter().enumerate() {
            let d = &data["classes"][i + 1];
            assert_eq!(d["name_key"].as_str(), Some(c.key));
            assert!(close(c.lo, &d["band_lo"]) && close(c.hi, &d["band_hi"]), "{}", c.key);
            assert_eq!(u32::from_str_radix(d["colour_sent_to_movie_uint"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap(), c.colour);
            assert_eq!(d["hud_type"].as_i64(), Some(i64::from(c.hud_type)));
        }
        let path = format!("{dir}/messages/data/fss_scan_constants.json");
        let rules: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap();
        let entries = data["entries"].as_array().unwrap();
        for (class, a) in ARCHETYPES.iter().enumerate() {
            let name = rules["signal_name_builder"]["planet_class_to_name"][class.to_string()].as_str().unwrap();
            let e = entries.iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("{name} not in the data"));
            assert!(close(a.lo, &e["band_lo"]) && close(a.hi, &e["band_hi"]), "{name}");
            assert!(close(a.amp_min, &e["amp_min"]) && close(a.amp_max, &e["amp_max"]), "{name}");
            let pattern: Vec<(usize, f64)> = e["pattern"].as_array().unwrap().iter().map(|p| (p[0].as_u64().unwrap() as usize, p[1].as_f64().unwrap())).collect();
            assert_eq!(pattern, a.pattern, "{name}");
        }
    }
}
