//! What asks each of the ship's state machines for which state (ARCHITECTURE.md A6, task T4c): the ship's facts
//! (`facts::ShipFacts`) as Elite's code asks the machines, where the exe shows it, else the states' own names and contents
//! (ours). `parts.rs` runs the machines from these answers and asks a machine only when its answer changes, as Elite's code
//! does (the gear manager asks the legs when its request byte +0xBB changes, FUN_14307f8a0 line 274; the power distributor
//! asks on its state's change, FUN_142dc6dc0 / FUN_142dc2180); the first request of each machine, when a ship is made, is
//! ours. Every row, with its proof (decompiles in `re/elite/ghidra/dec/`, notes in `re/elite/notes/sidewinder_animation.md`,
//! "Who requests the machines' states"):
//!
//! | machine | asked for | source |
//! |---|---|---|
//! | the gear legs' `LandingGearCSM` | `Down` with the gear commanded down, else `Up` (the leg's modes 2 and 0), when the command changes | exe: FUN_1430d3110, FUN_14307f8a0 |
//! | `LandingGearDoorsFront/Left/Right` | open or closed by their leg's `DoorClientRequest` (Start 0x1443b7d40), each leg's door named by its gear slot in the ship item; the door's open and closed states from the item's door table (`GEAR_DOORS`, `DOORS`) | exe + data (item 201ee494, slot +0x80, table +0xf0) |
//! | (a test's `sw put`) | every machine settled afresh to the facts it sets (the gear up), not animated | ours |
//! | `Left/RightHardpointDoor` | `DeployedState` (the table's open state) with the hardpoints deployed, else `IdleState` | data (the door table); the trigger ours |
//! | `HeatVents` | `Active` while the power plant runs, `Inactive` once it is out (the power distributor's states 7 and 4); the flaps then follow the active dissipation (`parts.rs`'s drive) | exe: FUN_142dc2180, FUN_142dc6dc0 |
//! | `Lights` | `ShutDown` once the power is out, its initial state when it is back | exe: FUN_142dc6dc0, FUN_142dc2180 (the machine's own initial state) |
//! | `CockpitEffects` | `Emergency` once the power is out (so a power loss leaves the Emergency state's lamps lit, not the cockpit all dark), `InFlight` when it is back (the reboot's flicker between them, FUN_142dc87e0, is not modelled: `Pilot` has no reboot); `LightsOut`: no code of the exe names it | exe: FUN_142dc6dc0, FUN_142dc2180 |
//! | `JumpEffects` | `Charging`, `Departing`, `Arriving` by the drive's phase, else `Idle`, the `_Internal` twins from the seat | exe: FUN_142c71440 at 0x142c757a8 (phase byte 3, 4, 0); which drive states the phases are, and that its "internal" test is the seat, ours |
//! | `CockpitState` | `Canopy Breached` once the canopy is broken (through `Breaching`), else `Normal` | exe: FUN_1430c6130 makes the machine in one of the two by the canopy; the live request ours |
//! | `OperationsMode` | `Dead` destroyed, `Stealth` silent, `Combat` hardpoints out, `Docked` landed, else `Travel` | ours (the states' names) |
//! | `LandingGearExtension` (and the legs' `ShipRaised`) | nothing: the manager's trigger (FUN_14307e4e0) was not traced | exe shows the requests (FUN_143082320), not when |
//! | `CockpitHeatState` | nothing: FUN_1434d8f00 switches it when a value crosses the middle of two others, inputs not traced (its one activity is steam) | not traced |
//! | `DamageEffects`, the two `Audio` | nothing: one state each | the data |

use super::assets::{MachineOwner, ShipRig};
use super::facts::{JumpPhase, ShipFacts};

/// The ship item's gear slots (item 201ee494, `re/elite/work/numbers/records/item_sidewinder.txt` +0x70 to +0x1a0): each
/// slot's module (the gear rig) and the door it names (grade A, the item's data). The item's record is not in the decoded
/// export (`re/elite/frea_export/sidewinder/decoded/`), so it cannot come through the converter: a constant here.
pub const GEAR_DOORS: [(&str, &str); 3] = [
    ("Ext_LandingGear_SideWinder_Front", "LandingGearDoorsFront"),
    ("Ext_LandingGear_SideWinder_Left", "LandingGearDoorsLeft"),
    ("Ext_LandingGear_SideWinder_Right", "LandingGearDoorsRight"),
];

/// The ship item's door table (the same record, `item_sidewinder.txt` +0xf0: five 48-byte records, the door, its machine,
/// the open state, the closed state, three u32), grade A; not in the decoded export either, so a constant here.
pub const DOORS: [(&str, &str, &str); 5] = [
    ("LeftHardpointDoor", "DeployedState", "IdleState"),
    ("RightHardpointDoor", "DeployedState", "IdleState"),
    ("LandingGearDoorsLeft", "DoorsOpen", "DoorsClosed"),
    ("LandingGearDoorsRight", "DoorsOpen", "DoorsClosed"),
    ("LandingGearDoorsFront", "DoorsOpen", "DoorsClosed"),
];

/// The door table's record of machine `i` (its open and closed states), if it is one of the ship's doors.
pub fn door_states(rig: &ShipRig, i: usize) -> Option<(&'static str, &'static str)> {
    let m = rig.ship.machines.get(i)?;
    if rig.machine_owner(i) != MachineOwner::Hull {
        return None;
    }
    DOORS.iter().find(|(name, ..)| m.name == *name).map(|&(_, open, closed)| (open, closed))
}

/// The door machine a gear leg's door client asks (`GEAR_DOORS`), by the leg machine's owner rig.
pub fn door_of_leg(rig: &ShipRig, leg: usize) -> Option<usize> {
    let owner = &rig.ship.machines.get(leg)?.owner;
    let (_, door) = GEAR_DOORS.iter().find(|(gear, _)| owner.eq_ignore_ascii_case(gear))?;
    (0..rig.ship.machines.len()).find(|&i| rig.ship.machines[i].name == *door && rig.machine_owner(i) == MachineOwner::Hull)
}

/// The state the ship asks machine `i` for now, None when nothing asks it (the table above). The gear doors are asked by
/// their legs (`parts.rs` acts on the legs' door requests), so here they are None.
pub fn wanted<'a>(rig: &'a ShipRig, i: usize, f: &ShipFacts) -> Option<&'a str> {
    if rig.is_leg(i) {
        return Some(if f.gear_down { "Down" } else { "Up" });
    }
    let m = rig.ship.machines.get(i)?;
    match (rig.machine_owner(i), m.name.as_str()) {
        (MachineOwner::Hull, "LeftHardpointDoor" | "RightHardpointDoor") => door_states(rig, i).map(|(open, closed)| if f.hardpoints { open } else { closed }),
        (MachineOwner::Hull, "HeatVents") => Some(if f.powered { "Active" } else { "Inactive" }),
        (MachineOwner::Hull, "Lights") => Some(if f.powered { m.initial.as_str() } else { "ShutDown" }),
        (MachineOwner::Hull, "OperationsMode") => Some(if f.destroyed {
            "Dead"
        } else if f.silent {
            "Stealth"
        } else if f.hardpoints {
            "Combat"
        } else if f.landed {
            "Docked"
        } else {
            "Travel"
        }),
        (MachineOwner::Hull, "JumpEffects") => Some(match (f.jump, f.seated) {
            (JumpPhase::Idle, _) => "Idle",
            (JumpPhase::Charging, false) => "Charging",
            (JumpPhase::Charging, true) => "Charging_Internal",
            (JumpPhase::Departing, false) => "Departing",
            (JumpPhase::Departing, true) => "Departing_Internal",
            (JumpPhase::Arriving, false) => "Arriving",
            (JumpPhase::Arriving, true) => "Arriving_Internal",
        }),
        (MachineOwner::Cockpit, "CockpitEffects") => Some(if f.powered { "InFlight" } else { "Emergency" }),
        (MachineOwner::Cockpit, "CockpitState") => Some(if f.canopy_intact { "Normal" } else { "Canopy Breached" }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> std::sync::Arc<ShipRig> {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        super::super::assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets.rig
    }

    /// The table on the mod's ship: every state it names is one of its machine's; each leg has its own side's doors; a
    /// parked, powered ship stands on its gear with its lights, vents and cockpit in flight; the power out shuts the
    /// lights, closes the vents' machine and gives the cockpit's emergency; the doors follow their table.
    #[test]
    fn the_request_table() {
        let rig = rig();
        let n = rig.ship.machines.len();
        let of = |name: &str, owner: MachineOwner| (0..n).find(|&i| rig.ship.machines[i].name == name && rig.machine_owner(i) == owner).unwrap();
        let parked = ShipFacts { powered: true, landed: true, gear_down: true, canopy_intact: true, ..Default::default() };
        let mut facts = vec![parked];
        for f in [
            ShipFacts { powered: false, ..parked },
            ShipFacts { silent: true, hardpoints: true, ..parked },
            ShipFacts { destroyed: true, canopy_intact: false, gear_down: false, seated: true, ..parked },
        ] {
            facts.push(f);
        }
        for jump in [JumpPhase::Charging, JumpPhase::Departing, JumpPhase::Arriving] {
            facts.push(ShipFacts { jump, ..parked });
            facts.push(ShipFacts { jump, seated: true, ..parked });
        }
        for f in &facts {
            for i in 0..n {
                if let Some(s) = wanted(&rig, i, f) {
                    assert!(rig.ship.machines[i].state(s).is_some(), "{} has no state {s}", rig.ship.machines[i].name);
                }
            }
        }
        let legs: Vec<usize> = (0..n).filter(|&i| rig.is_leg(i)).collect();
        assert_eq!(legs.len(), 3);
        let doors: Vec<&str> = legs.iter().map(|&l| rig.ship.machines[door_of_leg(&rig, l).unwrap()].name.as_str()).collect();
        let owners: Vec<&str> = legs.iter().map(|&l| rig.ship.machines[l].owner.as_str()).collect();
        for (door, owner) in doors.iter().zip(&owners) {
            let side = ["Front", "Left", "Right"].into_iter().find(|s| owner.ends_with(s)).unwrap();
            assert!(door.ends_with(side), "{owner} -> {door}");
        }
        let get = |f: &ShipFacts, name: &str, owner: MachineOwner| wanted(&rig, of(name, owner), f);
        use MachineOwner::{Cockpit, Hull};
        assert_eq!(wanted(&rig, legs[0], &parked), Some("Down"));
        assert_eq!(get(&parked, "Lights", Hull), Some("State_SidewinderLights"));
        assert_eq!(get(&parked, "HeatVents", Hull), Some("Active"));
        assert_eq!(get(&parked, "CockpitEffects", Cockpit), Some("InFlight"));
        assert_eq!(get(&parked, "OperationsMode", Hull), Some("Docked"));
        assert_eq!(get(&parked, "LeftHardpointDoor", Hull), Some("IdleState"));
        assert_eq!(get(&parked, "LandingGearDoorsLeft", Hull), None);
        assert_eq!(get(&parked, "LandingGearExtension", Hull), None);
        let dark = facts[1];
        assert_eq!((get(&dark, "Lights", Hull), get(&dark, "HeatVents", Hull), get(&dark, "CockpitEffects", Cockpit)), (Some("ShutDown"), Some("Inactive"), Some("Emergency")));
        let fight = facts[2];
        assert_eq!((get(&fight, "RightHardpointDoor", Hull), get(&fight, "OperationsMode", Hull)), (Some("DeployedState"), Some("Stealth")));
        let lost = facts[3];
        assert_eq!((wanted(&rig, legs[1], &lost), get(&lost, "OperationsMode", Hull), get(&lost, "CockpitState", Cockpit)), (Some("Up"), Some("Dead"), Some("Canopy Breached")));
        assert_eq!(get(&ShipFacts { jump: JumpPhase::Charging, seated: true, ..parked }, "JumpEffects", Hull), Some("Charging_Internal"));
    }
}
