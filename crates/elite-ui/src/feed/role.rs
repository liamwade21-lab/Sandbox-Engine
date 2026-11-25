//! The role panel, `rolepanel3` (`re/elite/notes/ui_messages_panels.md` part 4; the movie's code `code/rolepanel3/`). The
//! panel is driven by one `SetTabData`, one `InitDataStore` and its DataStore (A: part 4 section 0). What is fed: the lone
//! pilot's tabs, the empty bay pages' constant texts (`EmptyData`), the commander card's name, role and the Disembark
//! button's state and reason (`CommanderData`), the header's ship (`MothershipData`).
//!
//! Left unwritten, and why: the suit card (suit, loadout, grade, slots, tools: the mod has no suit), the ship's schematic
//! pictures (image resources the mod does not ship), `MothershipData/Stats` and the Helm page (shown only to a pilot who is
//! not in his ship), the bay, crew and gunner lists (a Sidewinder has none: the pages show `EmptyData`).

use super::{Movie, Outbox};
use crate::panel_facts::{FlightPhase, PanelFacts};
use crate::Value;

const MOVIE: Movie = Movie::RolePanel3;
const ROOT: &str = "RolePanelDataStore";

/// The lone helm pilot's tabs (A: notes part 4 11.1, `FUN_142ad6ad0`): Commander 4, Fighter 1, SRV 2, the Odyssey account's
/// list (stand-in, C: the measured Elite runs Odyssey, session ui1 saw the three tabs).
const TABS: [i32; 3] = [4, 1, 2];
/// `CommanderData.currentRole` for a ship (A: `FUN_142ab1e10`: vehicle type 1 -> 2).
const ROLE_SHIP: i32 = 2;

/// `EmptyData`, written once (A: `FUN_142ac5690`, notes part 4 3.1): the bay-less ship's Fighter and SRV pages.
const EMPTY_DATA: [(&str, &str); 12] = [
    ("fighterImage", "RolePanelEmptyBgrFighter"),
    ("fighterStatus", "$RolePanel2_EmptyData_Fighter_Status;"),
    ("fighterInfoTitle", "$RolePanel2_EmptyData_Fighter_InfoTitle;"),
    ("fighterInfoLabel", "$RolePanel2_EmptyData_Fighter_InfoLabel;"),
    ("fighterBulletTitle", "$RolePanel2_EmptyData_Fighter_BulletTitle;"),
    ("fighterBulletLabel", "$RolePanel2_EmptyData_Fighter_BulletLabel;"),
    ("srvImage", "RolePanelEmptyBgrSRV"),
    ("srvStatus", "$RolePanel2_EmptyData_SRV_Status;"),
    ("srvInfoTitle", "$RolePanel2_EmptyData_SRV_InfoTitle;"),
    ("srvInfoLabel", "$RolePanel2_EmptyData_SRV_InfoLabel;"),
    ("srvBulletTitle", "$RolePanel2_EmptyData_SRV_BulletTitle;"),
    ("srvBulletLabel", "$RolePanel2_EmptyData_SRV_BulletLabel;"),
];

/// Why the pilot may not get out, as Elite's disembark rule names it (A for the order, B for the meanings: notes part 4
/// 12.2 and 12.3): the gate first (supercruise, which covers the jump: bit 4; the glide: bit 9; silent running: bit 43),
/// then the hull not at rest (bit 19, "Ship is in motion"). Sent only when the game's rule for B and Disembark refuses,
/// which since 2026-11-24 it never does while seated (it allows getting out anywhere); refused while landed, there would
/// be no reason line (Elite's table has none to give). The text is the last matching bit's in Elite's table order; with
/// one bit set, that bit's.
pub fn refusal(facts: &PanelFacts, silent_running: bool) -> Option<&'static str> {
    if facts.leave_allowed {
        return None;
    }
    Some(match facts.flight {
        FlightPhase::Supercruise | FlightPhase::Hyperspace => "$RolePanel2_mothership_in_supercruise_warning;",
        FlightPhase::Glide => "$RolePanel2_just_Arrived_from_supercruise_warning;",
        _ if silent_running => "$RolePanel2_silent_running_warning;",
        FlightPhase::Normal => "$RolePanel2_not_landed_properly_warning;",
        FlightPhase::Landed => return None,
    })
}

/// The role panel's feeder.
#[derive(Clone, Debug, Default)]
pub struct Role {
    started: bool,
    commander: Option<(Option<String>, bool, String)>,
    ship: Option<(String, String)>,
}

impl Role {
    pub fn feed(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        if !self.started {
            // A: `SetTabData` then `InitDataStore`, once when the role state starts (`FUN_142ac90b0`); `EmptyData` once
            // (its stores go before the messages, so the panel reads them on `InitDataStore`).
            let tabs: Vec<Value> = TABS.iter().map(|t| Value::object([("tabType", Value::from(*t))])).collect();
            out.send(MOVIE, "SetTabData", vec![tabs.into()]);
            out.send(MOVIE, "InitDataStore", vec![]);
            for (key, text) in EMPTY_DATA {
                out.store(MOVIE, &[ROOT, "EmptyData"], key, text.into());
            }
            out.store(MOVIE, &[ROOT, "CommanderData"], "currentRole", ROLE_SHIP.into());
            self.started = true;
        }
        // The commander card (A: `FUN_142acd000`'s keys; B: `canDeploy` is the disembark check, `warning` its reason, no
        // bounty: icon 0, not red).
        let reason = refusal(facts, facts.functions.silent_running).unwrap_or("").to_string();
        let now = (facts.commander.clone(), facts.leave_allowed, reason);
        if self.commander.as_ref() != Some(&now) {
            let data = [ROOT, "CommanderData"];
            if let Some(name) = &now.0 {
                out.store(MOVIE, &data, "commanderName", name.as_str().into());
            }
            out.store(MOVIE, &data, "canDeploy", now.1.into());
            out.store(MOVIE, &data, "warning", now.2.as_str().into());
            out.store(MOVIE, &data, "warningIcon", 0.into());
            out.store(MOVIE, &data, "warningIsRed", false.into());
            self.commander = Some(now);
        }
        // The header's ship (A: `FUN_142ad1a10`'s keys): the model's name, and the ship's own name or ident.
        let model = format!("${}_name;", facts.ship.model);
        let name = facts.ship.name.clone().or_else(|| facts.ship.ident.clone()).unwrap_or_default();
        let ship = (model, name);
        if self.ship.as_ref() != Some(&ship) {
            out.store(MOVIE, &[ROOT, "MothershipData"], "shipModel", ship.0.as_str().into());
            out.store(MOVIE, &[ROOT, "MothershipData"], "shipName", ship.1.as_str().into());
            self.ship = Some(ship);
        }
    }

    /// The movie has just been loaded.
    pub fn reset(&mut self) {
        *self = Role::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disembark_reason_by_flight() {
        let mut f = PanelFacts::default();
        let reason = |f: &PanelFacts, silent| refusal(f, silent);
        f.flight = FlightPhase::Normal;
        assert_eq!(reason(&f, false), Some("$RolePanel2_not_landed_properly_warning;"));
        assert_eq!(reason(&f, true), Some("$RolePanel2_silent_running_warning;"), "the gate before the motion");
        f.flight = FlightPhase::Supercruise;
        assert_eq!(reason(&f, true), Some("$RolePanel2_mothership_in_supercruise_warning;"));
        f.flight = FlightPhase::Glide;
        assert_eq!(reason(&f, false), Some("$RolePanel2_just_Arrived_from_supercruise_warning;"));
        f.flight = FlightPhase::Landed;
        assert_eq!(reason(&f, false), None, "refused by the mod's rule with no reason of Elite's");
        f.leave_allowed = true;
        f.flight = FlightPhase::Normal;
        assert_eq!(reason(&f, true), None, "allowed");
    }

    #[test]
    fn tabs_store_and_changes() {
        let mut f = PanelFacts::default();
        f.ship.model = "sidewinder".into();
        let mut r = Role::default();
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetTabData", "InitDataStore"]);
        assert_eq!(
            out.msgs[0].args[0],
            Value::array([4, 1, 2].map(|t| Value::object([("tabType", Value::from(t))])))
        );
        let get = |out: &Outbox, prop: &str| out.stores.iter().find(|s| s.property == prop).map(|s| s.value.clone());
        assert_eq!(get(&out, "srvInfoTitle"), Some(Value::from("$RolePanel2_EmptyData_SRV_InfoTitle;")));
        assert_eq!(get(&out, "canDeploy"), Some(Value::from(false)));
        assert_eq!(get(&out, "warning"), Some(Value::from("$RolePanel2_not_landed_properly_warning;")));
        assert_eq!(get(&out, "shipModel"), Some(Value::from("$sidewinder_name;")));
        assert!(get(&out, "commanderName").is_none(), "the mod has no name: unwritten");
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        assert!(out.msgs.is_empty() && out.stores.is_empty(), "nothing changed");
        f.flight = FlightPhase::Landed;
        f.leave_allowed = true;
        let mut out = Outbox::default();
        r.feed(&f, &mut out);
        assert_eq!(get(&out, "canDeploy"), Some(Value::from(true)));
        assert_eq!(get(&out, "warning"), Some(Value::from("")));
    }
}
