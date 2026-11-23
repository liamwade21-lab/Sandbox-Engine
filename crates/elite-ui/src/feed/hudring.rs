//! The fire groups and hardpoints ring, `cockpithudring` (`re/elite/notes/ui_messages_hud.md`, its section; the senders
//! `FUN_142d33630` (the fitted list), `FUN_142d67de0` (the states), `FUN_142d590e0` (the fire group)).
//!
//! Not sent, and why: `UpdateHardpointsAmmo` (what the bar shows for an energy weapon was not traced, E);
//! `SetAnalysisMode` (no discovery scanner in the mod).

use super::{changed, Movie, Outbox};
use crate::facts::{Deploy, Hardpoint};
use crate::{UiFacts, Value};

const MOVIE: Movie = Movie::CockpitHudRing;

/// The fire groups' letters (A: the string at 0x14531d7a8).
const GROUP_LETTERS: &[u8; 8] = b"ABCDEFGH";

/// WeaponStates codes (A: the movies' `WeaponStates`).
const IDLE: i32 = 0;
const RETRACTING: i32 = 1;
const DEPLOYING: i32 = 2;
const ACTIVE: i32 = 3;
const DAMAGED: i32 = 13;
const UNPOWERED: i32 = 18;

/// A slot's WeaponStates code by the exe's tree (B: `FUN_142ac12a0`, first match wins), for what the mod has: the deploy
/// state first (in: idle; deploying; retracting; out: go on), then the module switched off (13) or unpowered (18), else
/// active (3: with no target held, the tree's last step). Not modelled: idle-but-shown (17, from a controller timer of
/// another class, E), malfunction, jamming, thermal overload, overheating, reloading (the mod has none of them), and
/// firing (14: no branch proven for the hardpoint module's laser, E).
pub fn weapon_state(hp: &Hardpoint) -> i32 {
    match hp.deploy {
        Deploy::Retracted => IDLE,
        Deploy::Deploying => DEPLOYING,
        Deploy::Retracting => RETRACTING,
        Deploy::Deployed if !hp.module_on => DAMAGED,
        Deploy::Deployed if !hp.powered => UNPOWERED,
        Deploy::Deployed => ACTIVE,
    }
}

/// A slot's side on the ring from the fire group's answer (B: `FUN_142d67de0`): 1 -> 1 (right), 2 -> 2 (left), 3 -> 6,
/// 4 -> 7. For 0 the exe asks the module (+0x18, +0x90: utility sides 3 or 4); the mod's weapons are always on a trigger,
/// so 0 is sent as no side (stand-in).
fn side(trigger: u8) -> i32 {
    match trigger {
        1 => 1,
        2 => 2,
        3 => 6,
        4 => 7,
        _ => 0,
    }
}

/// The ring's feeder.
#[derive(Clone, Debug, Default)]
pub struct HudRing {
    fitted: Option<Value>,
    states: Option<Value>,
    firegroup: Option<u8>,
}

impl HudRing {
    pub fn feed(&mut self, facts: &UiFacts, _dt: f64, out: &mut Outbox) {
        if let Some(hardpoints) = &facts.weapons.hardpoints {
            self.hardpoints(hardpoints, out);
        }
        self.firegroup(facts, out);
    }

    /// The movie has just been loaded: everything is sent again.
    pub fn reset(&mut self) {
        *self = HudRing::default();
    }

    /// `SetHardpoints([{id, moduleItemId, deployDuration, retractDuration, weaponType, name, weaponMode}])` when the fitted
    /// weapons change (B: builder `FUN_14466b6d0`, id = the slot's index); `UpdateHardpoints([{id, state,
    /// blockedByUIMode, side}], utilitySlotsEnabled)` when a slot's state, side or blocked flag changes (B: builder
    /// `FUN_14466b8c0`; its `range` field is left out: what the exe puts there was not decoded and the movie ignores it).
    /// Blocked by a UI mode is false (the mod has no analysis mode) and the utility slots are off (none fitted): stand-ins
    /// for the exe's values, not decoded (E). A new fitted list rebuilds the ring, so the states follow it (ours).
    fn hardpoints(&mut self, hardpoints: &[Hardpoint], out: &mut Outbox) {
        let fitted = Value::Array(
            hardpoints
                .iter()
                .enumerate()
                .map(|(id, hp)| {
                    Value::object([
                        ("id", Value::from(id as u32)),
                        ("moduleItemId", hp.module_item_id.into()),
                        ("deployDuration", hp.deploy_s.into()),
                        ("retractDuration", hp.retract_s.into()),
                        ("weaponType", hp.weapon_type.into()),
                        ("name", hp.name.as_str().into()),
                        ("weaponMode", hp.weapon_mode.into()),
                    ])
                })
                .collect(),
        );
        if changed(&mut self.fitted, fitted.clone()) {
            out.send(MOVIE, "SetHardpoints", vec![fitted]);
            self.states = None;
        }
        let states = Value::Array(
            hardpoints
                .iter()
                .enumerate()
                .map(|(id, hp)| {
                    Value::object([
                        ("id", Value::from(id as u32)),
                        ("state", weapon_state(hp).into()),
                        ("blockedByUIMode", false.into()),
                        ("side", side(hp.trigger).into()),
                    ])
                })
                .collect(),
        );
        if changed(&mut self.states, states.clone()) {
            out.send(MOVIE, "UpdateHardpoints", vec![states, false.into()]);
        }
    }

    /// `SetCurrentFiregroup(letter, warning)` when the group changes (B: `FUN_142d590e0`). The warning's condition (the
    /// fire-group interface +0x80, "probably the group has nothing to fire") was not identified (E); the mod's one group
    /// holds both lasers, so it is sent false (stand-in).
    fn firegroup(&mut self, facts: &UiFacts, out: &mut Outbox) {
        let Some(group) = facts.weapons.firegroup else { return };
        let Some(&letter) = GROUP_LETTERS.get(usize::from(group)) else { return };
        if changed(&mut self.firegroup, group) {
            out.send(MOVIE, "SetCurrentFiregroup", vec![char::from(letter).to_string().into(), false.into()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laser() -> Hardpoint {
        Hardpoint {
            module_item_id: 128_049_381,
            name: "Pulse Laser".into(),
            weapon_type: 1,
            weapon_mode: 1,
            deploy_s: 1.0,
            retract_s: 1.0,
            deploy: Deploy::Retracted,
            module_on: true,
            powered: true,
            trigger: 1,
        }
    }

    #[test]
    fn states_by_the_tree() {
        let mut hp = laser();
        assert_eq!(weapon_state(&hp), 0);
        hp.deploy = Deploy::Deploying;
        assert_eq!(weapon_state(&hp), 2);
        hp.deploy = Deploy::Retracting;
        assert_eq!(weapon_state(&hp), 1);
        hp.deploy = Deploy::Deployed;
        assert_eq!(weapon_state(&hp), 3);
        hp.powered = false;
        assert_eq!(weapon_state(&hp), 18);
        hp.module_on = false;
        assert_eq!(weapon_state(&hp), 13, "switched off before unpowered");
        hp.deploy = Deploy::Retracted;
        assert_eq!(weapon_state(&hp), 0, "the deploy state comes first");
    }

    #[test]
    fn ring_messages() {
        let mut ring = HudRing::default();
        let mut facts = UiFacts::default();
        facts.weapons.hardpoints = Some(vec![laser(), laser()]);
        facts.weapons.firegroup = Some(0);
        let mut out = Outbox::default();
        ring.feed(&facts, 0.0, &mut out);
        let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
        assert_eq!(names, ["SetHardpoints", "UpdateHardpoints", "SetCurrentFiregroup"]);
        let slots = &out.msgs[0].args[0];
        let Value::Array(slots) = slots else { panic!("an array") };
        assert_eq!(slots[1].get("id").and_then(Value::as_f64), Some(1.0));
        assert_eq!(slots[1].get("weaponMode").and_then(Value::as_f64), Some(1.0));
        assert_eq!(out.msgs[1].args[1], Value::Bool(false));
        let Value::Array(states) = &out.msgs[1].args[0] else { panic!("an array") };
        assert_eq!(states[0].get("side").and_then(Value::as_f64), Some(1.0), "primary: the right side");
        assert_eq!(out.msgs[2].args, vec![Value::from("A"), false.into()]);

        let mut out = Outbox::default();
        ring.feed(&facts, 0.0, &mut out);
        assert!(out.msgs.is_empty());
        facts.weapons.hardpoints.as_mut().unwrap()[0].deploy = Deploy::Deploying;
        ring.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["UpdateHardpoints"]);
        ring.reset();
        let mut out = Outbox::default();
        ring.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs.len(), 3, "everything again after reset");
    }
}
