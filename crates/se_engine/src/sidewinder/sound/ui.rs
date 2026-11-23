//! The ship UI's sounds (US1): what a movie's `PlaySound(name, source)` and the HUD events the exe posts itself become,
//! by Elite's rule (`re/elite/notes/ui_sounds.md` section 1, grade A): the name is looked up case-insensitively in its
//! interface's table (`FUN_144574060`: djb2 over the name with A-Z lower-cased, matched with `_strnicmp`) and the table's
//! value is posted as the event's name (`FUN_1440b83a0` hashes it with `GetIDFromString`). No entry posts nothing
//! (`FUN_144572060`); a value no bank holds posts an id nothing answers: silence. The name itself is never posted, and
//! nothing falls back.
//!
//! The tables are `ui/sound_tables.json` (written by `copy_sidewinder` from the decoded `ui_audio_tables.json`): for each
//! cockpit interface, its 29 keywords (the `AudioEvents` fields, which `FUN_144534910` stores in the same map under their
//! names; the decoded tables label the fields 0x10 and 0x11 `extra0x10` and `extra0x11`, their constants' names not being
//! found), its free names lower-cased, each to the table's value as it is, and its flagged names (the set at `H+0x20`:
//! kept, not used, since what the flag does is in the queue's reader, which is not found).
//!
//! **Where the sounds play (a stand-in, labelled):** Elite's reader of the queue, which turns the queued ids into posts and
//! picks the emitter by the `source` argument, is not found (the note's section 6). Every UI sound is posted on one
//! emitter of its own, [`EMITTER`], the listener's (never placed: its voices play through their sounds' routes at the
//! listener); `source` is kept in the queue record and logged. Tested and refused (US1b, `mod.rs`
//! `ui_panel_placement_hypothesis`): an emitter at the panel's bone (the left panel 1.26 m from the eye) by the mixer's own
//! placement does not give what Elite plays (the US1 review's correlation of the recordings with session ui1): closing the
//! left panel and moving a row there sit at one place and turn, so placement gives both the same gains (forward L +1.1 /
//! R -9.5 dB, turned to the panel 0.0 / -4.4), where Elite plays the close +2.3 / -2.9 dB and the row move -8 / -8 dB as
//! one signal against this emitter: their difference is the buses' (a034728d against d424e3b1), not a place.
//!
//! **The game parameters and states the shipped UI events read** are left at the banks' defaults, but for those the
//! ship's triggers already set (`triggers.rs`: `OnFootLocation` NotOnFoot, `player_location` outside_station,
//! `PlayerInsideAndEmitterAttached`, `FrameShiftState`, `LandingGear`, `spaceship_speed_self`, `spaceship_heat_self`,
//! `hyperdrive_countdown_self`). `PlayerInSeat` (default 1, read by 223 of the UI events' objects) and the state group
//! `PlayerVesselShutdown` (default none; read by 106) have no setter in the exe that the ship study found
//! (`re/elite/notes/sidewinder_sound_events.md`, its game syncs table: "never set"), and nothing traces which of the ship's
//! states Elite gives them, so neither is set, as for the ship's own sounds; nor `POV` (never set: `mod.rs`), nor the FSS's,
//! the surface scanner's, the radar's and the panels' own parameters (`SystemScan_*`, `SAA_*`, `Gui_*`,
//! `progress_Module*`, `LandingProgress_self`, `supercruise_countdown_self` ...), which only those screens would set.
//!
//! The queue ([`UiSounds`]) exists from the sounds' first read on (with `ShipSound`); `mod.rs` `drive` empties it once a
//! frame: posted while seated, dropped unheard otherwise.

#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use serde_json::Value;

/// The measured trims (US1c, a stated stand-in: the data gives no cause for them), each for a bus and an emitter, handed to
/// every mixer the mod makes ([`mixer`]). One: bus `9f83e9e8` (under the GUI bus), the UI's own emitter ([`EMITTER`]),
/// -7.9 dB, folded (both sides the recording's mono signal).
/// - **Measured (grade C)** on the side panels' four navigation sounds, `Navi_b_Cockpit`, `NaviPane_b_Cockpit`,
///   `PopUpShow_b_Cockpit` and `Select_b_Cockpit`: Elite's own recording of session ui1 (2026-11-23) holds, on both sides,
///   the shipped file's mono signal (L+R)/2 at the voice's scalar gain less 7.9 +- 0.4 dB, over about 45 presses (the US1
///   review's cross-correlation, `.optimize/scratch/run/us1_review/` `xcorr.py`, `gains.py`, `xcorr_mono.txt`).
/// - **Applied (grade B)** to every UI sound on the same emitter through the same bus (137 more shipped events: the gauge
///   panel's power and shield sounds, the panels' toggles and values, the orbital HUD's gauges and warnings, the role
///   panel, most of the FSS and surface scanner sets...), not themselves measured.
/// - **Not applied** to the ship's own voice and cockpit cues, which share the bus (58 `ship_events` / `open_events`, the
///   `VoxV01_*` lines, `Cockpit_FlightAssist_*`, `LandingGear_*`...) on their own emitters: never measured against this,
///   left as the banks say (`mod.rs` `no_ship_voice_is_ever_trimmed`). `Open_b` / `Close_b` (bus `d424e3b1`) match the
///   port within 0.5 dB and are not trimmed.
///
/// The cause was not found (US1b's Build log entry): the candidates are game parameter `35fb439e` on that bus (never set,
/// unnamed) and the GUI bus's compressor `72391599`.
pub const BUS_TRIMS: &[(se_core::elite_sound::Id, u32, se_core::elite_sound::BusTrim)] =
    &[(se_core::elite_sound::Id(0x9f83_e9e8), EMITTER, se_core::elite_sound::BusTrim { gain_db: -7.9, fold: true })];

/// The mixer the mod plays with: the shipped data's, with the measured trims ([`BUS_TRIMS`]).
pub fn mixer(bank: std::sync::Arc<se_core::elite_sound::SoundBank>, seed: u64) -> se_core::elite_sound::Mixer {
    let mut m = se_core::elite_sound::Mixer::new(bank, seed);
    for (bus, emitter, trim) in BUS_TRIMS {
        m.set_bus_trim(*bus, *emitter, *trim);
    }
    m
}

/// The emitter every UI sound is posted on (the listener's own: never placed). After the ship's own (`triggers::Emitter`
/// 0x100..0x302) and before the cue emitters (`mod.rs` `FIRST_CUE_EMITTER`, 0x1000).
pub const EMITTER: u32 = 0x400;

/// The version `ui/sound_tables.json` carries.
pub const TABLES_FORMAT: &str = "sidewinder-ui-sound-tables 1";

/// What the ship's UI asks to be heard. Any part of the mod may push; the sound systems drain it once a frame.
#[derive(Resource, Default)]
pub struct UiSounds {
    queue: Vec<UiRequest>,
}

impl UiSounds {
    /// A movie's `PlaySound(name, source)`. `interface` is Elite's interface name ("CockpitExternalPanel"...).
    pub fn play(&mut self, interface: &str, name: &str, source: Option<&str>) {
        self.queue.push(UiRequest { ask: UiAsk::Play { interface: interface.to_string(), name: name.to_string() }, source: source.map(str::to_string) });
    }

    /// An event the game itself posts for the HUD, by its Wwise name ("RadarPulse"...).
    pub fn post(&mut self, event: &str) {
        self.queue.push(UiRequest { ask: UiAsk::Post { event: event.to_string() }, source: None });
    }

    /// Everything asked since the last call.
    pub(super) fn take(&mut self) -> Vec<UiRequest> {
        std::mem::take(&mut self.queue)
    }
}

/// One thing asked of the UI's sound.
#[derive(Clone, Debug, PartialEq)]
pub struct UiRequest {
    pub ask: UiAsk,
    /// The movie's `source` argument (`AudioSources`: `InfoData`, `OtherData` ...), kept for the log (see the module's docs).
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiAsk {
    Play { interface: String, name: String },
    Post { event: String },
}

/// What a request becomes.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolved {
    /// This event is posted.
    Event(String),
    /// No table for the interface (or the tables were not read): nothing is posted.
    NoTable,
    /// The interface's table has no entry for the name: nothing is posted (`FUN_144572060`).
    NoEntry,
    /// The entry's value (or the posted name) is no event of the bank: Elite posts its id and nothing answers.
    NoEvent(String),
}

impl std::fmt::Display for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Resolved::Event(e) => write!(f, "event {e}"),
            Resolved::NoTable => write!(f, "silent: no entry (no table for the interface)"),
            Resolved::NoEntry => write!(f, "silent: no entry"),
            Resolved::NoEvent(e) => write!(f, "silent: no event ({e:?} is in no bank)"),
        }
    }
}

impl UiRequest {
    /// The interface, name and source as the log gives them.
    pub fn describe(&self) -> String {
        let source = self.source.as_deref().unwrap_or("none");
        match &self.ask {
            UiAsk::Play { interface, name } => format!("PlaySound {interface} {name:?} source {source}"),
            UiAsk::Post { event } => format!("post {event} (the exe's own) source {source}"),
        }
    }

    /// Elite's rule: a `PlaySound` through its interface's table, a post as it is; either posted only if `has_event`
    /// (the bank holds it).
    pub fn resolve(&self, tables: Option<&UiTables>, has_event: impl Fn(&str) -> bool) -> Resolved {
        let event = match &self.ask {
            UiAsk::Post { event } => event.as_str(),
            UiAsk::Play { interface, name } => {
                let Some(table) = tables.and_then(|t| t.tables.get(interface)) else { return Resolved::NoTable };
                match table.entries.get(&name.to_ascii_lowercase()) {
                    Some(value) => value.as_str(),
                    None => return Resolved::NoEntry,
                }
            }
        };
        if has_event(event) {
            Resolved::Event(event.to_string())
        } else {
            Resolved::NoEvent(event.to_string())
        }
    }
}

/// The interfaces' tables as `PlaySound` reads them.
#[derive(Debug)]
pub struct UiTables {
    tables: HashMap<String, Table>,
}

#[derive(Debug)]
struct Table {
    /// Lower-cased keyword or name -> the table's value.
    entries: HashMap<String, String>,
    #[allow(dead_code)]
    flagged: HashSet<String>,
}

/// One interface's table as the converter takes it from the decoded tables: keywords (field name -> value), names (as the
/// table gives them -> value), flagged names.
#[cfg(test)]
pub struct TableSource {
    pub keywords: BTreeMap<String, String>,
    pub names: BTreeMap<String, String>,
    pub flagged: Vec<String>,
}

/// `ui/sound_tables.json` from the decoded tables (the converter's): the names lower-cased, every value as it is.
#[cfg(test)]
pub fn tables_json(about: &str, interfaces: &BTreeMap<String, TableSource>) -> Value {
    let lower = |m: &BTreeMap<String, String>| -> serde_json::Map<String, Value> { m.iter().map(|(k, v)| (k.to_ascii_lowercase(), Value::String(v.clone()))).collect() };
    let out: serde_json::Map<String, Value> = interfaces
        .iter()
        .map(|(i, t)| {
            let flagged: Vec<String> = t.flagged.iter().map(|n| n.to_ascii_lowercase()).collect();
            (i.clone(), serde_json::json!({ "keywords": t.keywords, "names": lower(&t.names), "flagged": flagged }))
        })
        .collect();
    serde_json::json!({ "format": TABLES_FORMAT, "about": about, "interfaces": out })
}

impl UiTables {
    /// Reads `ui/sound_tables.json`. Refused: another format, a field it does not know, a name not lower-cased, two
    /// entries of one table that are the same name but for case (which one Elite keeps is not read).
    pub fn parse(bytes: &[u8]) -> Result<UiTables, String> {
        let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("ui/sound_tables.json: {e}"))?;
        let obj = v.as_object().ok_or("ui/sound_tables.json: not an object")?;
        if let Some(k) = obj.keys().find(|k| !["format", "about", "interfaces"].contains(&k.as_str())) {
            return Err(format!("ui/sound_tables.json: unknown field {k:?}"));
        }
        if obj.get("format").and_then(Value::as_str) != Some(TABLES_FORMAT) {
            return Err(format!("ui/sound_tables.json: format {:?}, expected {TABLES_FORMAT:?} (re-make it with copy_sidewinder)", obj.get("format")));
        }
        let interfaces = obj.get("interfaces").and_then(Value::as_object).ok_or("ui/sound_tables.json: no interfaces")?;
        let mut tables = HashMap::new();
        for (name, t) in interfaces {
            let at = |s: String| format!("ui/sound_tables.json: {name}: {s}");
            let t = t.as_object().ok_or_else(|| at("not an object".into()))?;
            if let Some(k) = t.keys().find(|k| !["keywords", "names", "flagged"].contains(&k.as_str())) {
                return Err(at(format!("unknown field {k:?}")));
            }
            let strings = |field: &str| -> Result<Vec<(String, String)>, String> {
                let m = t.get(field).and_then(Value::as_object).ok_or_else(|| at(format!("no {field}")))?;
                m.iter().map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())).ok_or_else(|| at(format!("{field} {k:?} is not a string")))).collect()
            };
            let mut entries = HashMap::new();
            for (key, value) in strings("keywords")? {
                if entries.insert(key.to_ascii_lowercase(), value).is_some() {
                    return Err(at(format!("two entries named {key:?} but for case")));
                }
            }
            for (key, value) in strings("names")? {
                if key != key.to_ascii_lowercase() {
                    return Err(at(format!("name {key:?} is not lower-cased")));
                }
                if entries.insert(key.clone(), value).is_some() {
                    return Err(at(format!("name {key:?} is also a keyword or a name but for case")));
                }
            }
            let flagged = t
                .get("flagged")
                .and_then(Value::as_array)
                .ok_or_else(|| at("no flagged".into()))?
                .iter()
                .map(|v| v.as_str().map(str::to_string).ok_or_else(|| at("a flagged name is not a string".into())))
                .collect::<Result<HashSet<String>, String>>()?;
            tables.insert(name.clone(), Table { entries, flagged });
        }
        Ok(UiTables { tables })
    }

    /// The interfaces held (the converter's read-back).
    #[cfg(test)]
    pub fn interfaces(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
}

/// `SE_SANDBOX_SOUNDS=1`: log each UI request and the voices it starts (as the sandbox logs its sounds; a test can't listen).
pub fn logged() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SE_SANDBOX_SOUNDS").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> UiTables {
        let mut i = BTreeMap::new();
        i.insert(
            "CockpitExternalPanel".to_string(),
            TableSource {
                keywords: [("focusChanged", "Navi_b_Cockpit"), ("hideBack", "CloseBack_b_Cockpit")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
                names: [("ModulePowerupBegin", ""), ("Shields_Begin", "ShieldRecharge_b")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
                flagged: Vec::new(),
            },
        );
        let bytes = serde_json::to_vec(&tables_json("test", &i)).unwrap();
        UiTables::parse(&bytes).unwrap()
    }

    fn bank(e: &str) -> bool {
        ["Navi_b_Cockpit", "ShieldRecharge_b", "RadarPulse"].iter().any(|x| x.eq_ignore_ascii_case(e))
    }

    fn play(i: &str, n: &str) -> UiRequest {
        let mut q = UiSounds::default();
        q.play(i, n, Some("InfoData"));
        q.take().remove(0)
    }

    #[test]
    fn the_lookup_ignores_case() {
        let t = tables();
        for n in ["focusChanged", "FOCUSCHANGED", "focuschanged"] {
            assert_eq!(play("CockpitExternalPanel", n).resolve(Some(&t), bank), Resolved::Event("Navi_b_Cockpit".into()), "{n}");
        }
        assert_eq!(play("CockpitExternalPanel", "shields_begin").resolve(Some(&t), bank), Resolved::Event("ShieldRecharge_b".into()));
    }

    #[test]
    fn no_entry_is_silent_and_the_name_is_never_posted() {
        let t = tables();
        // (A name that is itself an event of the bank, but not in the table: still silent.)
        assert_eq!(play("CockpitExternalPanel", "RadarPulse").resolve(Some(&t), bank), Resolved::NoEntry);
        assert_eq!(play("CockpitInfoPanel", "focusChanged").resolve(Some(&t), bank), Resolved::NoTable);
        assert_eq!(play("CockpitExternalPanel", "focusChanged").resolve(None, bank), Resolved::NoTable);
    }

    #[test]
    fn an_entry_without_an_event_is_silent() {
        let t = tables();
        assert_eq!(play("CockpitExternalPanel", "hideBack").resolve(Some(&t), bank), Resolved::NoEvent("CloseBack_b_Cockpit".into()));
        // (The empty entry: Elite posts the id of "".)
        assert_eq!(play("CockpitExternalPanel", "modulepowerupbegin").resolve(Some(&t), bank), Resolved::NoEvent(String::new()));
    }

    #[test]
    fn a_post_is_the_event_itself_if_the_bank_holds_it() {
        let mut q = UiSounds::default();
        q.post("RadarPulse");
        q.post("MessageSurfaceDiscovery");
        let asked = q.take();
        assert!(q.take().is_empty());
        assert_eq!(asked[0].resolve(None, bank), Resolved::Event("RadarPulse".into()));
        assert_eq!(asked[1].resolve(None, bank), Resolved::NoEvent("MessageSurfaceDiscovery".into()));
        assert_eq!(asked[1].source, None);
    }

    #[test]
    fn tables_that_clash_are_refused() {
        let mut i = BTreeMap::new();
        i.insert(
            "X".to_string(),
            TableSource {
                keywords: [("select".to_string(), "A".to_string())].into_iter().collect(),
                names: [("SELECT".to_string(), "B".to_string())].into_iter().collect(),
                flagged: Vec::new(),
            },
        );
        assert!(UiTables::parse(&serde_json::to_vec(&tables_json("t", &i)).unwrap()).is_err());
        assert!(UiTables::parse(br#"{"format":"sidewinder-ui-sound-tables 0","about":"","interfaces":{}}"#).is_err());
    }
}
