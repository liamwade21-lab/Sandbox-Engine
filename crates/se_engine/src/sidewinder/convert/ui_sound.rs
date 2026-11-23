//! The ship UI's sounds (US1; `sidewinder/sound/ui.rs` plays them): which events of the UI's audio export ship in
//! `sounds.json` (`sound.rs` merges that export with the ship's and builds the one file), and `ui/sound_tables.json`.
//!
//! The export is `re/elite/frea_export/sidewinder_ui/audio/` (`events.json`, `effects.json`, `media/` in the shape of the
//! ship's `sidewinder_audio`), made by `re/elite/work/ui/sounds/ui_export.py` with the ship's own tools
//! (`sidewinder_events.py`, `sidewinder_effects.py`, `sidewinder_media.py`) from the ship export's 62 bank payloads and the
//! 9 GUI banks `exportx.py` re-makes, its wanted list the one below. The tables are the decoded
//! `sidewinder_ui/decoded/sounds/ui_audio_tables.json` (each interface's UIMovie record's audio block,
//! `re/elite/notes/ui_sounds.md` section 1 step 8).
//!
//! Which events ship: every value of the keyword and name tables of `UI_INTERFACES`, every `UI_EXE_EVENTS` name, and every
//! named event of the export with a `UI_PREFIXES` prefix, each with all it reaches (`sound.rs`). A value no bank holds is
//! silence in Elite (its id is posted and nothing answers: the note's section 1 step 9): listed in `SOURCES.txt`, not
//! shipped; so is an empty entry (CockpitInternalPanel2's `ModulePowerupBegin`). Left out: the other interfaces and the
//! exe's multicrew, wings, mining, docking, interdiction, fighter, synthesis, kill feed and map events.

use std::collections::BTreeMap;

use serde_json::Value;

use se_core::elite_sound::SoundBank;

use super::super::sound::ui::{tables_json, TableSource, UiTables};
use super::sound::Export;
use super::{Ctx, Shipped};

/// The cockpit interfaces whose tables ship (the brief's list: the Sidewinder's cockpit HUD and panels).
pub(super) const UI_INTERFACES: &[(&str, &str)] = &[
    ("CockpitGaugePanel", "the lower gauge panel (shields, hull, the power distributor's pips, heat)"),
    ("CockpitHUDRing", "the HUD ring"),
    ("TargetSights", "the target sights"),
    ("CockpitInfoPanel", "the info panel: the HUD's messages (comms_message, info_message, malfunction_start ...)"),
    ("OrbitalHud", "the orbital HUD (altitude, rate of descent)"),
    ("Notifications", "the notifications"),
    ("HeadlockedText", "the head-locked text"),
    ("TextDisplay", "the text display"),
    ("CockpitFuelScoopGauge", "the fuel scoop gauge"),
    ("CockpitExternalPanel", "the left panel (navigation, transactions, contacts)"),
    ("CockpitInternalPanel2", "the right panel (modules, power, repairs)"),
    ("CockpitChatPanel", "the comms panel"),
    ("RolePanel3", "the role panel"),
    ("FssHUD", "the full spectrum scanner's HUD"),
    ("SaaHUD", "the surface scanner's HUD"),
    ("CockpitAnimation", "the cockpit's animations (telepresence)"),
    ("Vignette", "the vignette"),
];

/// Events the exe posts itself for the HUD (the note's section 4, `events_table.md` part 2, `work/ui/sounds/handles_map.json`:
/// a handle is posted as `<name>` or `<name>_b` / `<name>_e`, whichever the banks hold), each with its sender.
pub(super) const UI_EXE_EVENTS: &[(&str, &str)] = &[
    // Target selection (FUN_1441971c0; handles made by FUN_1441a3cd0): by the target's type, the scan loop and the status.
    ("TargetDeselect", "target selection d[2] (+0x1710)"),
    ("TargetSelectStation", "target selection, type 1 (+0x1770)"),
    ("TargetSelectObject", "target selection, types 2 and 5 (+0x17d0)"),
    ("TargetSelectNavigational", "target selection, type 3 (+0x1a10)"),
    ("TargetSelectShipSubsystem", "target selection, type 4 (+0x1ad0)"),
    ("TargetScan_b", "target selection d[3], the scan loop's begin (+0x1b30)"),
    ("TargetScan_e", "target selection d[3], the scan loop's end (+0x1b30)"),
    ("TargetScanObjects_b", "target selection d[3] for what is not a ship, begin (+0x1b90)"),
    ("TargetScanObjects_e", "target selection d[3] for what is not a ship, end (+0x1b90)"),
    ("TargetScanProgress_InfoStatus", "target selection d[4] (+0x1bf0)"),
    ("TargetScanProgress_ShieldStatus", "target selection d[5] (+0x1c50)"),
    ("TargetScanProgress_WantedStatus", "target selection d[6] (+0x1cb0)"),
    // Radar (FUN_144197c50; handles made by FUN_1441a2240).
    ("RadarPulse", "radar d[0]"),
    ("RadiusIncrease", "radar zoom in d[0x54]"),
    ("RadiusDecrease", "radar zoom out d[0x55]"),
    ("Indicator_b", "radar zoom in / out d[0x54], d[0x55]: the indicator's begin"),
    ("Indicator_e", "radar d[0x56]: the indicator's end"),
    ("ContactsIncrease", "radar: the count at d+0x30 above the last frame's"),
    ("HostileContactsIncrease", "radar: the count at d+0x38 above the last frame's"),
    ("HardpointsDeployedContactsIncrease", "radar: the count at d+0x48 above the last frame's"),
    ("FuzzyContactsDecrease", "radar: the count at d+0x40 above the last frame's"),
    // The HUD's message categories (FUN_142d21440) that are events.
    ("MessageInfo_b", "HUD message categories 0, 1, 0xb"),
    ("WarningMalfunction_b", "HUD message categories 2, 3, 0xd, 0x14"),
    ("MessageInfoPositive_b", "HUD message categories 4, 5, 0xc, 0x13"),
    ("MessageResourceCollected", "HUD message category 9"),
    ("MessageResourceCollectedAlt", "HUD message category 0xa"),
    ("MessageExploration", "HUD message categories 0xe, 0x15, 0x16"),
    ("MessageExplorationWarning", "HUD message category 0xf"),
    // Module power-up and repair (FUN_1441968b0; handles made by FUN_1441a1270).
    ("ModulePowerUp_Start_CockpitInternalPanel", "module power-up d[0xe]"),
    ("ModulePowerUp_Charge_CockpitInternalPanel_b", "module power-up: the charge loop's begin, with the progress flag"),
    ("ModulePowerUp_Charge_CockpitInternalPanel_e", "module power-up: the charge loop's end"),
    ("ModulePowerUp_Kill_CockpitInternalPanel", "module power-up d[4]"),
    ("ModulePowerUp_Complete_PanelNotVisible_CockpitInternalPanel", "module power-up d[0x11]"),
    ("ModuleRepair_Start_CockpitInternalPanel", "module repair d[0x13]"),
    ("ModuleRepair_Increase_CockpitInternalPanel_b", "module repair: the increase loop's begin"),
    ("ModuleRepair_Increase_CockpitInternalPanel_e", "module repair: the increase loop's end"),
    ("ModuleRepair_Stopped_CockpitInternalPanel", "module repair d[0x15]"),
    ("ModuleRepair_Complete_CockpitInternalPanel", "module repair d[0x16]"),
    // The surface scanner's probe set.
    ("DetailedSurfaceScanComplete", "surface scanner FUN_1442b0980"),
];

/// Names the exe posts for the HUD that no bank holds (the export lists them absent): silence in Elite.
pub(super) const UI_EXE_SILENT: &[(&str, &str)] = &[
    ("ModuleRepair_CockpitCanopy_CockpitInternalPanel", "module repair while the module type is 0x15 (+0xa70): no bank holds it, nor its _b / _e"),
    ("MessageSurfaceDiscovery", "HUD message category 0x11: no bank holds it"),
];

/// Every named event of the export with one of these prefixes ships (the brief: the FSS's and the surface scanner's sets;
/// their senders are the exe's SystemScan and SAA functions, `events_table.md` part 2, the rest named by the dictionary
/// search).
pub(super) const UI_PREFIXES: &[(&str, &str)] = &[("SystemScan_", "the full spectrum scanner (FSS)"), ("SAA_", "the surface scanner (SAA)")];

/// The UI's events picked from the export.
pub(super) struct UiPick {
    /// To ship, each once (the first spelling met: the tables', then the lists', then the export's).
    pub events: Vec<String>,
    /// Table values no bank holds (silence), each once.
    pub silent: Vec<String>,
}

/// The UI's audio export (`sidewinder_ui/audio` beside the ship's).
pub(super) fn audio(ctx: &Ctx) -> std::path::PathBuf {
    ctx.ui.join("audio")
}

/// The decoded tables of `UI_INTERFACES`: keyword -> value, name -> value (empty for an empty entry), flagged names.
pub(super) fn read_tables(ctx: &Ctx) -> Result<BTreeMap<String, TableSource>, String> {
    let path = ctx.ui.join("decoded").join("sounds").join("ui_audio_tables.json");
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let event = |e: &Value| e.get("event").and_then(Value::as_str).unwrap_or("").to_string();
    let mut out = BTreeMap::new();
    for (name, _) in UI_INTERFACES {
        let t = v.pointer(&format!("/interfaces/{name}")).ok_or_else(|| format!("ui_audio_tables.json has no interface {name}"))?;
        let keywords: BTreeMap<String, String> = t.get("keywords").and_then(Value::as_object).ok_or_else(|| format!("{name}: no keywords"))?.iter().map(|(k, e)| (k.clone(), event(e))).collect();
        if keywords.len() != 29 {
            return Err(format!("{name}: {} keywords, Elite's audio block has 29", keywords.len()));
        }
        let mut names = BTreeMap::new();
        for p in t.get("pairs").and_then(Value::as_array).ok_or_else(|| format!("{name}: no pairs"))? {
            let n = p.get("name").and_then(Value::as_str).ok_or_else(|| format!("{name}: a pair without a name"))?;
            if names.insert(n.to_string(), event(p)).is_some() {
                return Err(format!("{name}: the name {n} twice"));
            }
        }
        let flagged = t
            .get("names_in_set")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{name}: no names_in_set"))?
            .iter()
            .map(|n| n.as_str().map(str::to_string).ok_or_else(|| format!("{name}: a flagged name is not a string")))
            .collect::<Result<Vec<String>, String>>()?;
        out.insert(name.to_string(), TableSource { keywords, names, flagged });
    }
    Ok(out)
}

/// The events to ship and the silent values; an error if the export is not the one these lists make (a wanted name it
/// neither holds nor lists absent: re-make it with `ui_export.py`).
pub(super) fn pick(tables: &BTreeMap<String, TableSource>, x: &Export) -> Result<UiPick, String> {
    let (mut events, mut silent) = (BTreeMap::new(), BTreeMap::new());
    let stale = |n: &str| format!("{n} is neither an event of the UI export nor listed absent in it: re-make it with re/elite/work/ui/sounds/ui_export.py");
    for t in tables.values() {
        for v in t.keywords.values().chain(t.names.values()).filter(|v| !v.is_empty()) {
            if x.has(v) {
                events.entry(v.to_lowercase()).or_insert_with(|| v.clone());
            } else if x.absent(v) {
                silent.entry(v.to_lowercase()).or_insert_with(|| v.clone());
            } else {
                return Err(stale(v));
            }
        }
    }
    for (n, _) in UI_EXE_EVENTS {
        if !x.has(n) {
            return Err(stale(n));
        }
        events.entry(n.to_lowercase()).or_insert_with(|| n.to_string());
    }
    for (n, _) in UI_EXE_SILENT {
        if x.has(n) || !x.absent(n) {
            return Err(format!("{n}: the UI export {} (UI_EXE_SILENT says no bank holds it)", if x.has(n) { "holds it" } else { "does not list it absent" }));
        }
        silent.entry(n.to_lowercase()).or_insert_with(|| n.to_string());
    }
    for name in x.names() {
        if UI_PREFIXES.iter().any(|(p, _)| name.starts_with(p)) {
            events.entry(name.to_lowercase()).or_insert_with(|| name.to_string());
        }
    }
    Ok(UiPick { events: events.into_values().collect(), silent: silent.into_values().collect() })
}

/// `ui/sound_tables.json`: written, read back as the game reads it.
pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let tables = read_tables(ctx)?;
    let about = "The ship UI's PlaySound tables (sidewinder/sound/ui.rs reads them): per cockpit interface its 29 keywords (the \
                 AudioEvents fields, spelt as Elite spells them; 0x10 and 0x11 labelled extra0x10 / extra0x11, their names not \
                 found) and its names, lower-cased, each to the event name the table holds, as it is (an empty value or one no \
                 bank holds is silence), and its flagged names, lower-cased. Written by copy_sidewinder (sidewinder/convert/ui_sound.rs) from the decoded \
                 ui_audio_tables.json; Elite's lookup is case-insensitive (re/elite/notes/ui_sounds.md section 1).";
    let bytes = serde_json::to_vec_pretty(&tables_json(about, &tables)).map_err(|e| e.to_string())?;
    let rel = "ui/sound_tables.json";
    ctx.write(rel, &bytes)?;
    let entries: usize = tables.values().map(|t| t.keywords.len() + t.names.len()).sum();
    shipped.add(
        rel,
        &["re/elite/frea_export/sidewinder_ui/decoded/sounds/ui_audio_tables.json (the UIMovie records of the UIStreams archive ac954620287b)".to_string()],
        "A",
        &format!("The PlaySound tables of {} cockpit interfaces ({entries} entries), the values as Elite's data holds them.", tables.len()),
    );
    check_tables(&ctx.read(rel)?)
}

/// `ui/sound_tables.json` as the game reads it, holding exactly `UI_INTERFACES`.
pub(super) fn check_tables(bytes: &[u8]) -> Result<(), String> {
    let t = UiTables::parse(bytes)?;
    let mut held: Vec<&str> = t.interfaces().collect();
    held.sort_unstable();
    let mut want: Vec<&str> = UI_INTERFACES.iter().map(|(n, _)| *n).collect();
    want.sort_unstable();
    if held != want {
        return Err(format!("its interfaces are {held:?}, UI_INTERFACES {want:?}"));
    }
    Ok(())
}

/// The bank holds every `UI_EXE_EVENTS` event and none of `UI_EXE_SILENT`.
pub(super) fn check_bank(bank: &SoundBank) -> Result<(), String> {
    if let Some((n, _)) = UI_EXE_EVENTS.iter().find(|(n, _)| bank.event(n).is_none()) {
        return Err(format!("UI event {n} is missing"));
    }
    if let Some((n, _)) = UI_EXE_SILENT.iter().find(|(n, _)| bank.event(n).is_some()) {
        return Err(format!("{n} is in the bank, but UI_EXE_SILENT says no bank holds it"));
    }
    Ok(())
}
