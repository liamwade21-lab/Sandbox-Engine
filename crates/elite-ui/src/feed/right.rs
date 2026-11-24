//! The right panel, `cockpitinternalpanel2` (`re/elite/notes/ui_messages_panels.md` part 2; the movie's code
//! `code/cockpitinternalpanel2/`). What is fed: the home page (`SetPlayerInfo`, `SetShipOrVehicleInfo`), the Modules page
//! (the DataStore's module nodes and `SetModuleContext`), its pop-up (`ShowModulePopup`, `HideModulePopup`), the Fire groups
//! page (`SetFireGroups`, `SetActiveFireGroup`), the Ship tab's Functions and Flight assistance lists (`SetOptionsList`,
//! `UpdateOption`), the Inventory page's empty ship cargo (`SetCargoData`) and the game state the tabs follow (the
//! DataStore's `gameState`). `database_item` is the game's answer to the movie's `GetDatabaseItemData` for a module.
//!
//! Left as the movie's own state, and why: the ranks, balance, rebuy and notoriety (no credits or ranks in the mod),
//! Pilot preferences (none of its options is the mod's), Stats and the Status sub-pages (`SetDetailedPlayerInfo`: the title
//! the exe gives each page was not decoded, E, so nothing is sent: the movie's own "no data" page shows), the Inventory's
//! other sub-pages (materials, data, refinery, synthesis, cabins: none in the mod), Storage (the on-foot locker: none, so
//! the tab stays disabled), the music player.

use super::{changed, Movie, Outbox};
use crate::panel_facts::{FireGroups, Functions, Module, PanelFacts};
use crate::panels::{Function, PanelAction};
use crate::Value;
use std::collections::HashMap;

const MOVIE: Movie = Movie::CockpitInternalPanel2;

/// The pages' ids (`InternalOnPanel`, A: `Content.as`, `ShipPanel.as`).
const HOME_PAGE: i32 = 1;
const MODULES_PAGE: i32 = 2;
const FIRE_GROUPS_PAGE: i32 = 3;
const FUNCTIONS_PAGE: i32 = 4;
const PREFERENCES_PAGE: i32 = 5;
const INVENTORY_PAGE: i32 = 6;
const FLIGHT_ASSISTANCE_PAGE: i32 = 16;
/// The options lists' ids (`OptionsPanel`: 0 Functions, 1 Preferences, 2 Flight assistance).
const FUNCTIONS_LIST: i32 = 0;
const FLIGHT_ASSISTANCE_LIST: i32 = 2;

/// The module root's DataStore node. The exe hands the movie its node ids as numbers (`SetModuleContext`, A); which number
/// is the DataStore's own business, so any one works (ours).
const MODULE_ROOT: &str = "1";
const MODULE_ROOT_ID: u32 = 1;

/// Module types never listed (A: notes part 2 13.3, `FUN_141b3cf00`'s set).
const UNLISTED_TYPES: [i32; 29] =
    [1, 2, 3, 5, 11, 12, 13, 17, 27, 28, 33, 38, 41, 44, 45, 47, 48, 53, 54, 55, 56, 57, 60, 76, 78, 81, 82, 87, 88];
/// The module states (A: `ModuleState.as`): BROKEN 0, OFF 2, ON 6.
const STATE_OFF: i32 = 2;
const STATE_ON: i32 = 6;

/// The rating letter as the movie's class number (A: `ClassRatingIcon.setData`: 0 I, 1 H, 2 G, 3 F, 4 E, 5 D, 6 C, 7 B, 8
/// A).
pub fn class_number(rating: char) -> i32 {
    match rating.to_ascii_uppercase() {
        'I' => 0,
        'H' => 1,
        'G' => 2,
        'F' => 3,
        'E' => 4,
        'D' => 5,
        'C' => 6,
        'B' => 7,
        'A' => 8,
        _ => 9,
    }
}

/// The Modules list's type column (B: `FUN_144552500`: module types 7 and 8 weapon 0, 1, 2, 9 and 22 engine 2, the rest
/// system 1).
fn power_type(kind: i32) -> i32 {
    match kind {
        7 | 8 => 0,
        1 | 2 | 9 | 22 => 2,
        _ => 1,
    }
}

/// The modules the list shows, in Elite's order (B: notes part 2 12.1, `FUN_142cd4790`): highest power draw first, ties
/// by fitting order; the types of `UNLISTED_TYPES` left out. (The second list, the hull reinforcements, the mod has none
/// of.)
pub fn listed_modules(modules: &[Module]) -> Vec<&Module> {
    let mut list: Vec<(usize, &Module)> = modules.iter().enumerate().filter(|(_, m)| !UNLISTED_TYPES.contains(&m.kind)).collect();
    list.sort_by(|(ia, a), (ib, b)| b.power_mw.total_cmp(&a.power_mw).then(ia.cmp(ib)));
    list.into_iter().map(|(_, m)| m).collect()
}

fn state(m: &Module) -> i32 {
    if m.on { STATE_ON } else { STATE_OFF }
}

/// The game's `GetDatabaseItemData(id)` for a module (A: the builder `FUN_14452a0a0`'s keys): its texts by its item name,
/// its size and class, its mount. The market fields, the price and the mass are not the mod's (empty, 0). None for an id
/// that is no module.
pub fn database_item(facts: &PanelFacts, id: u32) -> Option<Value> {
    let m = facts.modules.iter().find(|m| m.id == id)?;
    Some(Value::object([
        ("name", Value::from(format!("${}_Name;", m.item))),
        ("info", format!("${}_Info;", m.item).into()),
        ("longName", format!("${}_LongName;", m.item).into()),
        ("marketCategoryName", "".into()),
        ("marketCategoryIcon", "".into()),
        ("marketCategoryAbbrev", "".into()),
        ("basePrice", 0.into()),
        ("moduleSize", u32::from(m.size).into()),
        ("moduleClass", class_number(m.rating).into()),
        ("modulePowerConsumption", m.power_mw.into()),
        ("weaponMode", m.weapon_mode.unwrap_or(-1).into()),
        ("isGuardianModule", false.into()),
        ("isWingModule", false.into()),
        ("moduleMass", 0.into()),
    ]))
}

/// One module's slot node's properties (A: `FUN_144552500` computes, `FUN_144550ce0` writes; the table in notes part 2
/// 2.2): no ammunition, grade, engineering or repair; always the player's; the health's text empty (the movie shows the
/// percentage); no priority (the mod has none).
fn slot_properties(m: &Module, sort_index: usize) -> Vec<(&'static str, Value)> {
    let priority = Value::object([
        ("hasPriority", Value::from(m.priority.is_some())),
        ("priority", u32::from(m.priority.unwrap_or(0)).into()),
    ]);
    let module_state = Value::object([("state", Value::from(state(m))), ("powerUpProgress", 0.into()), ("powerUpDuration", 0.into())]);
    vec![
        ("sortIndex", (sort_index as u32).into()),
        ("callbackID", m.id.into()),
        ("databaseId", m.id.into()),
        ("isPowered", m.on.into()),
        ("isMarkedForRepair", false.into()),
        ("powerUsage", m.power_mw.into()),
        ("type", power_type(m.kind).into()),
        ("slotSize", u32::from(m.size).into()),
        ("ammoData", "".into()),
        ("boostGrade", 0.into()),
        ("hideClassAndRating", false.into()),
        ("isModified", false.into()),
        ("isMercModified", false.into()),
        ("isMercCoinsActive", false.into()),
        ("isOwnedByPlayer", true.into()),
        ("health", Value::object([("string", Value::from("")), ("health", m.health.into())])),
        ("priority", priority),
        ("state", module_state),
    ]
}

/// The module root's properties (A: `FUN_142d77460` lines 997-1015): the power plant's full and present output, the draw
/// of every module that is not off, and the priority groups (one per priority of the modules that have one and are not
/// broken or off: their draw summed; offline when one waits for power, which the mod never has).
fn root_properties(facts: &PanelFacts) -> Vec<(&'static str, Value)> {
    let used: f64 = facts.modules.iter().filter(|m| state(m) != STATE_OFF).map(|m| m.power_mw).sum();
    let mut groups: Vec<(u8, f64)> = Vec::new();
    for m in facts.modules.iter().filter(|m| state(m) != STATE_OFF) {
        if let Some(p) = m.priority {
            match groups.iter_mut().find(|(g, _)| *g == p) {
                Some((_, usage)) => *usage += m.power_mw,
                None => groups.push((p, m.power_mw)),
            }
        }
    }
    groups.sort_by_key(|(p, _)| *p);
    let groups: Vec<Value> = groups
        .iter()
        .map(|(p, usage)| Value::object([("priority", Value::from(u32::from(*p))), ("powerUsage", (*usage).into()), ("offline", false.into())]))
        .collect();
    let (base, now) = (facts.power.capacity_mw, facts.power.available_mw);
    let power_data = Value::object([
        ("powerPriorityGroups", Value::from(groups)),
        ("baseTotalCharge", base.into()),
        ("currentCharge", now.into()),
        ("currentPowerUsed", used.into()),
    ]);
    vec![
        ("blocked", false.into()),
        ("baseTotalCharge", base.into()),
        ("currentCharge", now.into()),
        ("currentPowerUsed", used.into()),
        ("powerData", power_data),
    ]
}

/// The fire groups (A: notes part 2 3.2 and 3.3, `FUN_142d66300`): the weapons in their category, the categories in the
/// exe's order (Beam {0}, Projectile {3, 4}, Launcher {2}, Limpet {10}); each entry with its bindings per group, every cell
/// selectable when bound and otherwise when not every state is taken (two states, primary and secondary: B, the rule of
/// `FUN_142d8cd60` without utilities).
fn fire_groups(facts: &PanelFacts) -> Vec<Value> {
    const CATEGORIES: [(&str, &[i32]); 4] = [
        ("$INT_PANEL_firegroup_beam_weapons;", &[0]),
        ("$INT_PANEL_firegroup_projectile_weapons;", &[3, 4]),
        ("$INT_PANEL_firegroup_launcher_weapons;", &[2]),
        ("$INT_PANEL_firegroup_drones;", &[10]),
    ];
    let fg: &FireGroups = &facts.fire_groups;
    let mut out = Vec::new();
    for (label, types) in CATEGORIES {
        let entries: Vec<Value> = fg
            .weapons
            .iter()
            .filter(|w| types.contains(&w.weapon_type))
            .filter_map(|w| {
                let m = facts.modules.iter().find(|m| m.id == w.module)?;
                let groups: Vec<Value> = (0..usize::from(fg.count))
                    .map(|g| {
                        let binding = w.bindings.get(g).copied().unwrap_or(0);
                        let taken = |state: u8| {
                            fg.weapons.iter().any(|o| o.slot == w.slot && o.trigger != w.trigger && o.bindings.get(g) == Some(&state))
                        };
                        let selectable = binding != 0 || !(taken(1) && taken(2));
                        Value::object([("binding", Value::from(u32::from(binding))), ("selectable", selectable.into())])
                    })
                    .collect();
                Some(Value::object([
                    ("label", Value::from(format!("${}_Name;", m.item))),
                    ("groups", groups.into()),
                    ("moduleSize", u32::from(m.size).into()),
                    ("moduleClass", class_number(m.rating).into()),
                    ("weaponMode", m.weapon_mode.unwrap_or(0).into()),
                    ("ownedByPlayer", true.into()),
                    ("utilityTrigger", false.into()),
                    ("slotIndex", w.slot.into()),
                    ("triggerIndex", w.trigger.into()),
                ]))
            })
            .collect();
        if !entries.is_empty() {
            out.push(Value::object([("label", Value::from(label)), ("type", 0.into()), ("entries", entries.into())]));
        }
    }
    out
}

/// One option row (A: `FUN_1444ff6a0`'s keys; a combo, type 0). `on` gives the text and the next state (`nextOption` 1
/// off, 2 on: what a select turns it to).
fn option(id: u32, label: &str, value: &str, on: bool, disabled: bool, column: u32, row: u32) -> Value {
    Value::object([
        ("option", Value::from(id)),
        ("type", 0.into()),
        ("label", label.into()),
        ("curValue", value.into()),
        ("nextOption", (if on { 1 } else { 2 }).into()),
        ("disabled", disabled.into()),
        ("columnIndex", column.into()),
        ("rowPriority", row.into()),
    ])
}

/// The on and off texts of the plain switches (A: the exe's two literals at 0x145321d58 and 0x145321d60, read by the value
/// builder `FUN_142d0bb20`).
fn on_off(on: bool) -> &'static str {
    if on { "$on;" } else { "$off;" }
}

/// The Functions list: the options Elite's list has that the mod has a switch for, with the table's columns and rows (A:
/// notes part 2 4.2): external lights (id 5, the ship's), landing gear (7: retracted / deployed, unavailable while it
/// cannot move), cargo scoop (12: retracted / lowered), silent running (15). The lights: the notes' table gives the
/// lights' values as Dipped Beam and Full Beam, and session ui1 saw "OFF" with them off (C); the mod's lights are one
/// switch, so on is Elite's first beam, Dipped Beam, and off is `$off;` (B). The rest of Elite's list (night vision,
/// beacon, reboot/repair, turret mode, self destruct, crew options) the mod does not have.
fn functions_list(f: &Functions) -> Vec<Value> {
    let gear = if f.gear_down { "$INT_PANEL_langing_gear_deployed;" } else { "$INT_PANEL_landing_gear_retracted;" };
    let scoop = if f.cargo_scoop { "$INT_PANEL_scoop_lowered;" } else { "$INT_PANEL_scoop_raised;" };
    let lights = if f.lights { "$INT_PANEL_dipped_beam;" } else { on_off(false) };
    vec![
        option(Function::Lights.option(), "$INT_PANEL_ship_lights;", lights, f.lights, false, 0, 0),
        option(Function::LandingGear.option(), "$INT_PANEL_landing_gear;", gear, f.gear_down, f.gear_locked, 0, 2),
        option(Function::CargoScoop.option(), "$INT_PANEL_cargo_scoop;", scoop, f.cargo_scoop, false, 0, 3),
        option(Function::SilentRunning.option(), "$INT_PANEL_button_up;", on_off(f.silent_running), f.silent_running, false, 1, 2),
    ]
}

/// The Flight assistance list: flight assist (id 1, A). Its other rows (rotational correction, auto dock, launch, land)
/// the mod does not have.
fn flight_assistance_list(f: &Functions) -> Vec<Value> {
    vec![option(Function::FlightAssist.option(), "$INT_PANEL_option_flight_assistance;", on_off(f.flight_assist), f.flight_assist, false, 0, 0)]
}

/// The right panel's feeder.
#[derive(Clone, Debug, Default)]
pub struct Right {
    page: i32,
    game_state: Option<bool>,
    player: Option<Option<String>>,
    ship: Option<Value>,
    /// The module nodes' properties as last written, by (node, property).
    written: HashMap<(String, &'static str), Value>,
    context_sent: bool,
    fire: Option<Vec<Value>>,
    active: Option<u8>,
    lists: Option<[Vec<Value>; 2]>,
    cargo: Option<f64>,
    /// The module pop-up: asked for (`ToggleModuleState`), open, or to be closed.
    popup: Option<ModulePopup>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ModulePopup {
    Asked(u32),
    Open(u32),
    Closing,
}

impl Right {
    pub fn feed(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        self.game_state(facts, out);
        if matches!(self.page, HOME_PAGE | FUNCTIONS_PAGE) {
            self.home(facts, out);
        }
        if self.page == MODULES_PAGE {
            self.modules(facts, out);
        }
        if self.page == FIRE_GROUPS_PAGE {
            self.fire_groups(facts, out);
        }
        if matches!(self.page, FUNCTIONS_PAGE | PREFERENCES_PAGE | FLIGHT_ASSISTANCE_PAGE) {
            self.options(facts, out);
        }
        if self.page == INVENTORY_PAGE {
            self.cargo(facts, out);
        }
        self.popup(facts, out);
    }

    /// The movie has just been loaded.
    pub fn reset(&mut self) {
        *self = Right::default();
    }

    /// The `gameState` node the tabs follow (B: `FUN_14115ed60` writes it; the panel reads `isHyperspace` and the rest,
    /// `Content.onGameStateChanged`): in the hyperspace tunnel the panel shows its jump page. `hasSeasonTwo` is true, the
    /// Horizons account the measured Elite runs (stand-in, C: session ui1's home page has no "Requires Horizons" on its
    /// Holo-Me and Engineers tiles). `hasSeasonThree` (Odyssey) is left unwritten, false: it would open the Storage tab, the
    /// on-foot locker the mod has none of. The other flags (CQC, fighter, training, shuttle, cargo off) are never set for a
    /// lone ship and left unwritten (false).
    fn game_state(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        let hyperspace = facts.flight == crate::panel_facts::FlightPhase::Hyperspace;
        if changed(&mut self.game_state, hyperspace) {
            out.store(MOVIE, &["gameState"], "hasSeasonTwo", true.into());
            out.store(MOVIE, &["gameState"], "isHyperspace", hyperspace.into());
        }
    }

    /// The Inventory page's ship cargo (A: `SetCargoData`, sent on page 6, the record's keys by its builder
    /// `FUN_1444fd2c0`): the default cargo sub-page (type 0) empty, with the racks' capacity; no filters, no transfer.
    /// The record's `label` is not shown by the page and its text was not decoded (E): empty. With no capacity known,
    /// nothing is sent.
    fn cargo(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        let Some(capacity) = facts.cargo_capacity_t else { return };
        if !changed(&mut self.cargo, capacity) {
            return;
        }
        let record = Value::object([
            ("label", Value::from("")),
            ("type", 0.into()),
            ("selectable", true.into()),
            ("totalCargo", 0.into()),
            ("capacity", capacity.into()),
            ("cargo", Value::Array(Vec::new())),
            ("activeFilters", Value::Array(Vec::new())),
            ("enableTransfer", false.into()),
        ]);
        out.send(MOVIE, "SetCargoData", vec![Value::array([record])]);
    }

    /// The home page (A: `SetPlayerInfo`, builder `FUN_14454d2b0`, every key; `SetShipOrVehicleInfo`, sent on pages 1 and
    /// 4 when the ship changed). No squadron, avatar, ranks, crimes, balance rows or alerts: the mod has none.
    fn home(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        if changed(&mut self.player, facts.commander.clone()) {
            let info = Value::object([
                ("pilotName", Value::from(facts.commander.as_deref().unwrap_or(""))),
                ("squadronID", "".into()),
                ("avatarSource", "".into()),
                ("ranks", Value::Array(Vec::new())),
                ("wanted", "".into()),
                ("staticRows", Value::Array(Vec::new())),
                ("numNewInLog", 0.into()),
                ("galnetAlert", false.into()),
                ("holomeAlert", false.into()),
                ("powerPlayAlert", false.into()),
            ]);
            out.send(MOVIE, "SetPlayerInfo", vec![info]);
        }
        // The schematic and the maker's icon name image resources the mod does not ship: empty (the movie then shows none).
        let ship = Value::object([
            ("type", Value::from(format!("${}_name;", facts.ship.model))),
            ("name", facts.ship.name.as_deref().unwrap_or("").into()),
            ("id", facts.ship.ident.as_deref().unwrap_or("").into()),
            ("schematic", "".into()),
            ("manufacturerIcon", "".into()),
            ("isVehicle", false.into()),
        ]);
        if changed(&mut self.ship, ship) {
            out.send(MOVIE, "SetShipOrVehicleInfo", vec![self.ship.clone().unwrap_or_default()]);
        }
    }

    /// The Modules page: the DataStore's root, its `Slots` and a `Slot <callback id>` node per listed module, each property
    /// written when it changed; then the root's path, once a visit (A: notes part 2 2.2; the movie listens to the nodes).
    fn modules(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        let mut write = |out: &mut Outbox, node: &[&str], props: Vec<(&'static str, Value)>| {
            let key = node.join("/");
            for (property, value) in props {
                let k = (key.clone(), property);
                if self.written.get(&k) != Some(&value) {
                    out.store(MOVIE, node, property, value.clone());
                    self.written.insert(k, value);
                }
            }
        };
        write(out, &[MODULE_ROOT], root_properties(facts));
        for (i, m) in listed_modules(&facts.modules).into_iter().enumerate() {
            let slot = format!("Slot {}", m.id);
            write(out, &[MODULE_ROOT, "Slots", &slot], slot_properties(m, i));
        }
        if !self.context_sent {
            out.send(MOVIE, "SetModuleContext", vec![Value::array([MODULE_ROOT_ID])]);
            self.context_sent = true;
        }
    }

    /// The Fire groups page: the groups when they changed, the active group when it changed (A: `FUN_142d590e0`).
    fn fire_groups(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        if changed(&mut self.fire, fire_groups(facts)) {
            out.send(MOVIE, "SetFireGroups", vec![self.fire.clone().unwrap_or_default().into()]);
        }
        if changed(&mut self.active, facts.fire_groups.active) {
            out.send(MOVIE, "SetActiveFireGroup", vec![u32::from(facts.fire_groups.active).into()]);
        }
    }

    /// The Ship tab's lists: both whole on a visit, then `UpdateOption` for each row that changed (A: `FUN_142d79290`).
    fn options(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        let now = [functions_list(&facts.functions), flight_assistance_list(&facts.functions)];
        let ids = [FUNCTIONS_LIST, FLIGHT_ASSISTANCE_LIST];
        match &self.lists {
            None => {
                for (id, list) in ids.iter().zip(&now) {
                    out.send(MOVIE, "SetOptionsList", vec![(*id).into(), list.clone().into()]);
                }
            }
            Some(before) => {
                for ((id, list), old) in ids.iter().zip(&now).zip(before) {
                    for (row, was) in list.iter().zip(old) {
                        if row != was {
                            out.send(MOVIE, "UpdateOption", vec![(*id).into(), row.clone()]);
                        }
                    }
                }
            }
        }
        self.lists = Some(now);
    }

    /// The module pop-up (B: `FUN_142d50a00`): the module's name, the ship icon (0), its mount, and one radio, the power
    /// switch (id 0), labelled by the state (`INT_PANEL_module_active` / `_inactive`), selectable when the module may be
    /// switched; and a Back button (id 2, `EXT_PANEL_back`). No repair radio (the mod has no repairs), no sub-heading
    /// (its text was not decoded, E), no info or perks.
    fn popup(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        match self.popup {
            Some(ModulePopup::Asked(id)) => {
                let Some(m) = facts.modules.iter().find(|m| m.id == id) else {
                    self.popup = None;
                    return;
                };
                let label = if m.on { "$INT_PANEL_module_active;" } else { "$INT_PANEL_module_inactive;" };
                let radio = Value::object([
                    ("id", Value::from(0)),
                    ("label", label.into()),
                    ("selectable", m.switchable.into()),
                    ("state", state(m).into()),
                    ("powerUpProgress", 0.into()),
                    ("powerUpDuration", 0.into()),
                ]);
                let options = Value::object([("type", Value::from(0)), ("options", Value::array([radio]))]);
                let back = Value::object([
                    ("type", Value::from(3)),
                    ("label", "$EXT_PANEL_back;".into()),
                    ("selectable", true.into()),
                    ("id", 2.into()),
                ]);
                // The mount's icon (`weaponsTypeStrings`: 0 fixed, 1 gimballed, 2 turret, 3 none).
                let gimbal = m.weapon_mode.filter(|mode| (0..=2).contains(mode)).unwrap_or(3);
                let record = Value::object([
                    ("moduleTitle", Value::from(format!("${}_Name;", m.item))),
                    ("headingIcon", 0.into()),
                    ("subHeading", "".into()),
                    ("items", Value::array([options, back])),
                    ("state", 1.into()),
                    ("isModified", false.into()),
                    ("isMercModified", false.into()),
                    ("gimbalIcon", gimbal.into()),
                ]);
                out.send(MOVIE, "ShowModulePopup", vec![record]);
                self.popup = Some(ModulePopup::Open(id));
            }
            Some(ModulePopup::Closing) => {
                out.send(MOVIE, "HideModulePopup", vec![]);
                self.popup = None;
            }
            _ => {}
        }
    }

    /// The panel's own events; the module switch the pop-up's radio means comes back.
    pub fn hear(&mut self, action: &PanelAction, facts: &PanelFacts) -> Option<PanelAction> {
        match *action {
            PanelAction::Page { page, .. } => {
                if page != self.page {
                    self.page = page;
                    // A page is sent whole when entered (the controllers' rebuild flags, notes part 2 0.).
                    self.player = None;
                    self.ship = None;
                    self.context_sent = false;
                    self.fire = None;
                    self.active = None;
                    self.lists = None;
                    self.cargo = None;
                }
                None
            }
            PanelAction::ModuleSelected(id) => {
                self.popup = Some(ModulePopup::Asked(id));
                None
            }
            PanelAction::ModuleOption(0) => {
                // A: `FUN_142d387b0`, only while the pop-up is open: id 0 switches the module's power, then the pop-up closes.
                let Some(ModulePopup::Open(id)) = self.popup else { return None };
                self.popup = Some(ModulePopup::Closing);
                let m = facts.modules.iter().find(|m| m.id == id)?;
                m.switchable.then_some(PanelAction::SetModule { id, on: !m.on })
            }
            PanelAction::Popup { state: 0, .. } => {
                // The movie closed the pop-up itself (its Back, `UI_Back`: `InternalOnModulePopup [0]`).
                if matches!(self.popup, Some(ModulePopup::Open(_))) {
                    self.popup = None;
                }
                None
            }
            PanelAction::ModuleRow(2) => {
                // A: `FUN_142d38db0`: the pop-up's Back (id 2) closes it.
                if self.popup.is_some() {
                    self.popup = Some(ModulePopup::Closing);
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel_facts::{FireWeapon, Power};

    fn module(id: u32, item: &str, kind: i32, size: u8, rating: char, power_mw: f64) -> Module {
        Module { id, item: item.into(), kind, size, rating, power_mw, on: true, switchable: true, health: 1.0, priority: None, weapon_mode: None }
    }

    /// A ship like the stock one (the numbers are this test's).
    fn facts() -> PanelFacts {
        let mut laser = module(9, "Hpt_Laser", 7, 1, 'G', 0.4);
        laser.weapon_mode = Some(1);
        let mut laser2 = laser.clone();
        laser2.id = 10;
        PanelFacts {
            modules: vec![
                module(1, "Int_Plant", 10, 2, 'E', 0.0),
                module(2, "Int_Engine", 22, 2, 'E', 2.0),
                module(3, "Int_Tank", 11, 1, 'C', 0.0),
                module(4, "Int_Shield", 4, 2, 'E', 0.9),
                laser,
                laser2,
            ],
            power: Power { capacity_mw: 6.0, available_mw: 6.0 },
            fire_groups: FireGroups {
                count: 1,
                active: 0,
                weapons: vec![
                    FireWeapon { module: 9, slot: 0, trigger: 0, weapon_type: 0, bindings: vec![1] },
                    FireWeapon { module: 10, slot: 1, trigger: 0, weapon_type: 0, bindings: vec![1] },
                ],
            },
            ..PanelFacts::default()
        }
    }

    #[test]
    fn modules_are_ordered_by_power_and_filtered() {
        let f = facts();
        let ids: Vec<u32> = listed_modules(&f.modules).iter().map(|m| m.id).collect();
        assert_eq!(ids, [2, 4, 9, 10, 1], "power down, ties by fitting order, the fuel tank never listed");
        assert_eq!(class_number('E'), 4);
        assert_eq!(class_number('G'), 2);
        assert_eq!(class_number('A'), 8);
        assert_eq!((power_type(7), power_type(22), power_type(10)), (0, 2, 1));
    }

    #[test]
    fn module_nodes_and_totals() {
        let mut f = facts();
        f.modules[3].on = false;
        let mut r = Right::default();
        r.hear(&PanelAction::Page { movie: MOVIE, page: MODULES_PAGE }, &f);
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        let get = |node: &str, prop: &str| {
            out.stores.iter().find(|s| s.context.join("/") == node && s.property == prop).map(|s| s.value.clone())
        };
        assert_eq!(get("1", "baseTotalCharge"), Some(Value::from(6.0)));
        assert_eq!(get("1", "currentPowerUsed"), Some(Value::from(2.0 + 0.4 + 0.4)), "the shield is off");
        assert_eq!(get("1/Slots/Slot 2", "sortIndex"), Some(Value::from(0)));
        assert_eq!(get("1/Slots/Slot 4", "state").and_then(|s| s.get("state").cloned()), Some(Value::from(STATE_OFF)));
        assert_eq!(get("1/Slots/Slot 9", "type"), Some(Value::from(0)));
        assert!(get("1/Slots/Slot 3", "sortIndex").is_none(), "the fuel tank is no row");
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetModuleContext"]);
        assert_eq!(out.msgs[0].args, vec![Value::array([1])]);
        let Value::Object(power) = get("1", "powerData").unwrap() else { panic!() };
        assert_eq!(power.len(), 4);
        // Nothing changed: nothing written; a switch: just that slot's state and the totals.
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        assert!(out.stores.is_empty() && out.msgs.is_empty());
        f.modules[3].on = true;
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        let written: Vec<String> = out.stores.iter().map(|s| format!("{} {}", s.context.join("/"), s.property)).collect();
        assert_eq!(written, ["1 currentPowerUsed", "1 powerData", "1/Slots/Slot 4 isPowered", "1/Slots/Slot 4 state"]);
    }

    #[test]
    fn the_module_popup_switches_its_module() {
        let f = facts();
        let mut r = Right::default();
        r.hear(&PanelAction::ModuleSelected(4), &f);
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        let popup = out.msgs.iter().find(|m| m.name == "ShowModulePopup").unwrap();
        assert_eq!(popup.args[0].get("moduleTitle").and_then(Value::as_str), Some("$Int_Shield_Name;"));
        assert_eq!(r.hear(&PanelAction::ModuleOption(0), &f), Some(PanelAction::SetModule { id: 4, on: false }));
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["HideModulePopup"]);
        assert_eq!(r.hear(&PanelAction::ModuleOption(0), &f), None, "closed");
        let mut fixed = f.clone();
        fixed.modules[0].switchable = false;
        r.hear(&PanelAction::ModuleSelected(1), &fixed);
        r.feed(&fixed, &mut Outbox::default());
        assert_eq!(r.hear(&PanelAction::ModuleOption(0), &fixed), None, "the power plant cannot be switched");
    }

    #[test]
    fn fire_groups_and_options() {
        let mut f = facts();
        let groups = fire_groups(&f);
        assert_eq!(groups.len(), 1, "one category: beam");
        let entries = match groups[0].get("entries") {
            Some(Value::Array(e)) => e.clone(),
            _ => panic!(),
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].get("label").and_then(Value::as_str), Some("$Hpt_Laser_Name;"));
        assert_eq!(entries[0].get("moduleClass").and_then(Value::as_f64), Some(2.0));
        let Some(Value::Array(cells)) = entries[0].get("groups") else { panic!() };
        assert_eq!(cells, &vec![Value::object([("binding", Value::from(1)), ("selectable", true.into())])]);
        f.functions = Functions { gear_down: true, gear_locked: true, silent_running: true, ..Functions::default() };
        let list = functions_list(&f.functions);
        assert_eq!(list[1].get("curValue").and_then(Value::as_str), Some("$INT_PANEL_langing_gear_deployed;"));
        assert_eq!(list[1].get("disabled").and_then(Value::as_bool), Some(true));
        assert_eq!(list[1].get("nextOption").and_then(Value::as_f64), Some(1.0));
        assert_eq!(list[3].get("curValue").and_then(Value::as_str), Some("$on;"));
        assert_eq!(list[0].get("curValue").and_then(Value::as_str), Some("$off;"));
        let lit = Functions { lights: true, ..Functions::default() };
        assert_eq!(functions_list(&lit)[0].get("curValue").and_then(Value::as_str), Some("$INT_PANEL_dipped_beam;"));
        let mut r = Right::default();
        r.hear(&PanelAction::Page { movie: MOVIE, page: FUNCTIONS_PAGE }, &f);
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        let lists: Vec<f64> = out.msgs.iter().filter(|m| m.name == "SetOptionsList").map(|m| m.args[0].as_f64().unwrap()).collect();
        assert_eq!(lists, [0.0, 2.0]);
        f.functions.flight_assist = true;
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
        assert_eq!(names, ["UpdateOption"]);
        assert_eq!(out.msgs[0].args[0], Value::from(2));
    }

    #[test]
    fn the_inventory_shows_the_empty_racks() {
        let mut f = facts();
        let mut r = Right::default();
        r.hear(&PanelAction::Page { movie: MOVIE, page: INVENTORY_PAGE }, &f);
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        assert!(out.msgs.iter().all(|m| m.name != "SetCargoData"), "no capacity known: nothing");
        f.cargo_capacity_t = Some(4.0);
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        let cargo = out.msgs.iter().find(|m| m.name == "SetCargoData").expect("the cargo");
        let Value::Array(records) = &cargo.args[0] else { panic!() };
        assert_eq!(records[0].get("capacity").and_then(Value::as_f64), Some(4.0));
        assert_eq!(records[0].get("totalCargo").and_then(Value::as_f64), Some(0.0));
        assert_eq!(records[0].get("cargo"), Some(&Value::Array(Vec::new())));
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        assert!(out.msgs.is_empty(), "once a visit");
    }

    #[test]
    fn database_items_name_their_texts() {
        let f = facts();
        let item = database_item(&f, 9).unwrap();
        assert_eq!(item.get("longName").and_then(Value::as_str), Some("$Hpt_Laser_LongName;"));
        assert_eq!(item.get("moduleClass").and_then(Value::as_f64), Some(2.0));
        assert_eq!(item.get("weaponMode").and_then(Value::as_f64), Some(1.0));
        assert!(database_item(&f, 77).is_none());
    }
}
