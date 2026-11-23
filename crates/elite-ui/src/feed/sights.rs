//! The indicators placed in the view, `targetsights` (`re/elite/notes/ui_messages_hud.md`, its section). The movie is an
//! atlas of cells; the game puts each cell on the screen, so the cell (the `uid`) is a contract with the drawing side.
//!
//! What is sent: the central gunsight while armed (B: "2008 CENTRAL_GUNSIGHT, always when armed"). Not sent, and why: the
//! lasers' own gunsights (which of the projectile or beam types the Sidewinder's pulse lasers use was not decoded, and
//! they follow a target the mod does not have, E); the nav marker (its state, mode and eta rules were not decoded, E);
//! target, contact and other indicators (no other ships in the mod).

use super::{Movie, Outbox};
use crate::{UiFacts, Value};

const MOVIE: Movie = Movie::TargetSights;

/// The central gunsight's cell: the first small cell (uids 54..125 are the small ones: A, the movie's factory). Which cell
/// the exe gives it was not decoded (E); this one is ours, and the drawing side places it at the view's centre.
pub const CENTRAL_GUNSIGHT_UID: u32 = 54;
/// Its indicator type (A: the movie's `SmallIndicators`).
const CENTRAL_GUNSIGHT: i32 = 2008;

/// The sights' feeder.
#[derive(Clone, Debug, Default)]
pub struct Sights {
    gunsight: bool,
}

impl Sights {
    /// While armed: `AddIndicator({uid, type})` makes the gunsight (hidden: its constructor hides it) and
    /// `UpdateIndicator({uid, type})` shows it (its `updateData` only shows it); disarmed, `RemoveIndicator(uid)`.
    pub fn feed(&mut self, facts: &UiFacts, _dt: f64, out: &mut Outbox) {
        let armed = facts.weapons.armed;
        if armed == self.gunsight {
            return;
        }
        self.gunsight = armed;
        if armed {
            let indicator = || Value::object([("uid", Value::from(CENTRAL_GUNSIGHT_UID)), ("type", CENTRAL_GUNSIGHT.into())]);
            out.send(MOVIE, "AddIndicator", vec![indicator()]);
            out.send(MOVIE, "UpdateIndicator", vec![indicator()]);
        } else {
            out.send(MOVIE, "RemoveIndicator", vec![CENTRAL_GUNSIGHT_UID.into()]);
        }
    }

    /// The movie has just been loaded: the gunsight is made again if armed (not for boarding: see `feed`).
    pub fn reset(&mut self) {
        *self = Sights::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gunsight_follows_the_hardpoints() {
        let mut sights = Sights::default();
        let mut facts = UiFacts::default();
        let mut out = Outbox::default();
        sights.feed(&facts, 0.0, &mut out);
        assert!(out.msgs.is_empty(), "not armed: nothing");
        facts.weapons.armed = true;
        sights.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["AddIndicator", "UpdateIndicator"]);
        assert_eq!(out.msgs[0].args[0].get("type").and_then(Value::as_f64), Some(2008.0));
        assert_eq!(out.msgs[0].args[0].get("uid").and_then(Value::as_f64), Some(54.0));
        let mut out = Outbox::default();
        sights.feed(&facts, 0.0, &mut out);
        assert!(out.msgs.is_empty());
        sights.reset();
        sights.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs.len(), 2, "again after reset");
        facts.weapons.armed = false;
        let mut out = Outbox::default();
        sights.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs[0].name, "RemoveIndicator");
        assert_eq!(out.msgs[0].args, vec![Value::from(54u32)]);
    }
}
