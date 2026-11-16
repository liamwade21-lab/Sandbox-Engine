//! Which pose the moving parts have now and which state each of the ship's state machines is in (ARCHITECTURE.md A6): the
//! rigs' values (`elite_rig::Values`), every rig node's matrix in the ship's space, for `draw`, and every machine's state
//! name, for `lights`.
//!
//! The machines are not run yet: each machine's state is chosen by a small table from the ship's public state
//! (`chosen_state`), and the pose is every machine's chosen state's own values (`ShipRig::state_values`). The animation task
//! replaces the table by running the machines (`elite_rig::Runner::state`) and writes the same `states`; `draw` reads only
//! `Parts::matrix`, `lights` only `Parts::states`.

use bevy::math::Mat4;
use bevy::prelude::*;
use se_core::elite_rig::Values;

use super::assets::{MachineOwner, ShipRig};
use super::pilot::Pilot;
use super::seat::Seat;
use super::SidewinderAssets;

#[derive(Resource, Default)]
pub struct Parts {
    /// A test's gear pose whatever the pilot's (`sw pose up|down|auto`): seated, the free camera can't look at the ship.
    pub dev_gear: Option<bool>,
    /// A test's states whatever the table says (`sw lights state MACHINE STATE|auto`), by machine name.
    pub dev_states: Vec<(String, String)>,
    values: Values,
    /// The gear the matrices are posed for (None before the first pose).
    gear_down: Option<bool>,
    /// Each rig node's matrix in the ship's space (`ShipRig::nodes`' order).
    matrices: Vec<Mat4>,
    /// Every machine's state now, in `ship.machines`' order (empty before the first update).
    states: Vec<String>,
    /// Counts the changes of the states (and so of the pose).
    revision: u64,
}

impl Parts {
    /// A node's matrix now (identity before the first pose).
    pub fn matrix(&self, node: usize) -> Mat4 {
        self.matrices.get(node).copied().unwrap_or(Mat4::IDENTITY)
    }

    /// The rigs' values now.
    pub fn values(&self) -> &Values {
        &self.values
    }

    /// Every machine's state now, in `ship.machines`' order.
    pub fn states(&self) -> &[String] {
        &self.states
    }

    /// Is the gear posed down (None before the first pose)?
    pub fn gear_down(&self) -> Option<bool> {
        self.gear_down
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// What of the ship's public state (`Pilot`, `Seat`) chooses the machines' states.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShipFacts {
    pub powered: bool,
    pub silent: bool,
    pub destroyed: bool,
    pub hardpoints: bool,
    pub landed: bool,
    pub gear_down: bool,
    pub canopy_intact: bool,
}

impl ShipFacts {
    /// From the pilot's state (which belongs to the ship, parked or flown: `Pilot::boarding`) and the gear's pose.
    pub fn of(pilot: &Pilot, gear_down: bool) -> ShipFacts {
        ShipFacts {
            powered: pilot.powered,
            silent: pilot.silent,
            destroyed: pilot.destroyed,
            hardpoints: pilot.weapons.deployed,
            landed: pilot.landed,
            gear_down,
            canopy_intact: pilot.life.canopy,
        }
    }
}

/// The state the table puts machine `i` in, or None for its initial state. Every choice is ours until the machines run
/// (the game's requests that move them were not traced); each follows the states' own names and contents
/// (re/elite/notes/sidewinder_animation.md, "The ship's sequences"):
/// - the gear legs Down or Up and the hull's gear doors DoorsOpen or DoorsClosed, as the gear is (the still poses' rule);
/// - the hull's `Lights`: ShutDown (the state with no lights) while the power plant is off, else State_SidewinderLights,
///   its initial state, the one holding every outside light. A parked ship nobody sits in keeps it: nothing in the data
///   or the notes powers a parked ship down, and the machine starts in it when the ship is made;
/// - the hull's `HeatVents`: Active (the vent lights and the vents' drive, which follow the radiator) while the radiator
///   runs, which is elite-sim's rule (powered, not silent), else Inactive;
/// - the hull's `OperationsMode`: Dead once destroyed, Stealth while silent, Combat with the hardpoints out, Docked when
///   landed, else Travel (its initial state);
/// - the cockpit's `CockpitEffects`: LightsOut while the power plant is off, else InFlight (its initial state);
///   Emergency's trigger is not known: only a test's `sw lights state` reaches it;
/// - the cockpit's `CockpitState`: Canopy Breached once the canopy is broken, else Normal.
pub fn chosen_state(rig: &ShipRig, i: usize, f: &ShipFacts) -> Option<&'static str> {
    if rig.is_leg(i) {
        return Some(if f.gear_down { "Down" } else { "Up" });
    }
    if rig.is_gear_door(i) {
        return Some(if f.gear_down { "DoorsOpen" } else { "DoorsClosed" });
    }
    let name = rig.ship.machines.get(i)?.name.as_str();
    match (rig.machine_owner(i), name) {
        (MachineOwner::Hull, "Lights") => Some(if f.powered { "State_SidewinderLights" } else { "ShutDown" }),
        (MachineOwner::Hull, "HeatVents") => Some(if f.powered && !f.silent { "Active" } else { "Inactive" }),
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
        (MachineOwner::Cockpit, "CockpitEffects") => Some(if f.powered { "InFlight" } else { "LightsOut" }),
        (MachineOwner::Cockpit, "CockpitState") => Some(if f.canopy_intact { "Normal" } else { "Canopy Breached" }),
        _ => None,
    }
}

/// Every machine's state by the table, a test's choice (by machine name) over it.
pub fn states(rig: &ShipRig, f: &ShipFacts, dev: &[(String, String)]) -> Vec<String> {
    rig.ship
        .machines
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let dev = dev.iter().find(|(name, state)| name.eq_ignore_ascii_case(&m.name) && m.state(state).is_some()).map(|(_, s)| s.as_str());
            dev.or(chosen_state(rig, i, f)).unwrap_or(&m.initial).to_string()
        })
        .collect()
}

/// Chooses the machines' states each frame and poses the parts when they change.
pub fn update(assets: Res<SidewinderAssets>, seat: Res<Seat>, pilot: Res<Pilot>, mut parts: ResMut<Parts>) {
    let down = parts.dev_gear.unwrap_or(!seat.seated || pilot.gear_out());
    let rig = &assets.rig;
    let states = states(rig, &ShipFacts::of(&pilot, down), &parts.dev_states);
    if parts.states == states {
        return;
    }
    let values = rig.state_values(&states);
    match rig.pose(&values) {
        Ok(m) => {
            parts.matrices = m;
            if parts.gear_down != Some(down) {
                info!("sidewinder: the parts posed with the gear {}", if down { "down, its doors open" } else { "up, its doors closed" });
            }
        }
        // (`load` posed both still poses already; a refusal here would be a bug: keep the last matrices.)
        Err(e) => warn!("sidewinder: the parts can't be posed: {e}"),
    }
    parts.values = values;
    parts.gear_down = Some(down);
    parts.states = states;
    parts.revision += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> std::sync::Arc<ShipRig> {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        super::super::assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets.rig
    }

    /// The table on the mod's ship: a parked, powered ship has its outside lights' state and the gear down; the power off
    /// shuts the lights and the cockpit's lamps down; silent running stops the vents; a test's state wins where the
    /// machine has it; the still poses are the table's poses.
    #[test]
    fn the_states_table() {
        let rig = rig();
        let of = |name: &str, owner: MachineOwner| (0..rig.ship.machines.len()).find(|&i| rig.ship.machines[i].name == name && rig.machine_owner(i) == owner).unwrap();
        let (lights, vents, ops, effects) = (of("Lights", MachineOwner::Hull), of("HeatVents", MachineOwner::Hull), of("OperationsMode", MachineOwner::Hull), of("CockpitEffects", MachineOwner::Cockpit));
        let parked = ShipFacts { powered: true, landed: true, gear_down: true, canopy_intact: true, ..Default::default() };
        let s = states(&rig, &parked, &[]);
        assert_eq!((s[lights].as_str(), s[vents].as_str(), s[ops].as_str(), s[effects].as_str()), ("State_SidewinderLights", "Active", "Docked", "InFlight"));
        let dark = states(&rig, &ShipFacts { powered: false, ..parked }, &[]);
        assert_eq!((dark[lights].as_str(), dark[vents].as_str(), dark[effects].as_str()), ("ShutDown", "Inactive", "LightsOut"));
        let silent = states(&rig, &ShipFacts { silent: true, ..parked }, &[]);
        assert_eq!((silent[vents].as_str(), silent[ops].as_str()), ("Inactive", "Stealth"));
        let dev = states(&rig, &parked, &[("cockpiteffects".into(), "Emergency".into()), ("Lights".into(), "NoSuchState".into())]);
        assert_eq!((dev[effects].as_str(), dev[lights].as_str()), ("Emergency", "State_SidewinderLights"));
        for down in [true, false] {
            assert_eq!(rig.state_values(&states(&rig, &ShipFacts { gear_down: down, ..parked }, &[])), rig.static_pose(down), "gear down {down}");
        }
    }
}
