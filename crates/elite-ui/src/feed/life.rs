//! The flight HUD movies' life cycle (`re/elite/notes/ui_messages_hud.md` sections 1 and 5, each movie's Show / Hide).
//! `Open` is what the game sends a movie it has loaded (A: section 5, `UIApplication.open`: open and visible). Who sends
//! `Show` and `Hide` to the HUD movies was not found (E: one shared string, not traced; the cockpit's power-up path and
//! its start-up animation are out of scope), so this is a stand-in that uses the plain messages the notes name: every
//! HUD movie is shown while someone is seated and the power plant runs, and hidden otherwise. `Enable` and `Close` are
//! not sent (the HUD movies take no input, and the game unloads them itself).

use super::{changed, Movie, Outbox};
use crate::UiFacts;

/// The flight HUD's movies (A10's scope).
const HUD: [Movie; 6] = [
    Movie::CockpitGaugePanel,
    Movie::CockpitHudRing,
    Movie::TargetSights,
    Movie::CockpitInfoPanel,
    Movie::OrbitalHud,
    Movie::Notifications,
];

/// The life cycle's feeder.
#[derive(Clone, Debug, Default)]
pub struct Life {
    opened: bool,
    shown: Option<bool>,
}

impl Life {
    pub fn feed(&mut self, facts: &UiFacts, _dt: f64, out: &mut Outbox) {
        if !self.opened {
            for movie in HUD {
                out.send(movie, "Open", vec![]);
            }
            self.opened = true;
        }
        let shown = facts.status.seated && facts.status.powered;
        if changed(&mut self.shown, shown) {
            for movie in HUD {
                out.send(movie, if shown { "Show" } else { "Hide" }, vec![]);
            }
        }
    }

    /// The movies have just been loaded: each is opened again.
    pub fn reset(&mut self) {
        *self = Life::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_then_shown_with_power() {
        let mut life = Life::default();
        let mut facts = UiFacts::default();
        facts.status.seated = true;
        facts.status.powered = true;
        let mut out = Outbox::default();
        life.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs.len(), 12);
        assert!(out.msgs[..6].iter().all(|m| m.name == "Open"));
        assert!(out.msgs[6..].iter().all(|m| m.name == "Show"));
        let mut out = Outbox::default();
        life.feed(&facts, 0.0, &mut out);
        assert!(out.msgs.is_empty());
        facts.status.powered = false;
        life.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs.len(), 6);
        assert!(out.msgs.iter().all(|m| m.name == "Hide"));
        life.reset();
        let mut out = Outbox::default();
        life.feed(&facts, 0.0, &mut out);
        assert_eq!(out.msgs.len(), 12, "opened and hidden again after reset");
    }
}
