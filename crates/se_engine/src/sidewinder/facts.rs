//! The ship's facts this frame (ARCHITECTURE.md A6, "Who reads whom"): one resource, `ShipFacts`, written once a frame from
//! `Pilot` and `Seat` before the parts run, read by the machines (`machines.rs`, `parts.rs`), the lights and the sound. It
//! keeps the tests' overrides of those facts (`sw lights heat`, `sw pose`), so no reader holds another's. Nothing of the
//! parts, the lights or the sound is read here.

use bevy::prelude::*;
use elite_sim::heat::{visual_dissipation, VisualDissipation};

use super::pilot::Pilot;
use super::seat::Seat;

/// The frame shift drive's phase as `JumpEffects` reads it (FUN_142c71440 at 0x142c757a8: the drive's phase byte +0x759).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JumpPhase {
    #[default]
    Idle,
    Charging,
    Departing,
    Arriving,
}

/// What of the ship's state the machines, the lights and the sound read this frame.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ShipFacts {
    pub powered: bool,
    pub silent: bool,
    pub destroyed: bool,
    pub hardpoints: bool,
    pub landed: bool,
    /// The gear's command (Elite's request byte +0xBB): the pilot's, a test's `sw pose`, and down while nobody sits in it.
    pub gear_down: bool,
    pub canopy_intact: bool,
    pub seated: bool,
    pub jump: JumpPhase,
    /// The heat fraction (heat / capacity), a test's `sw lights heat F` over the ship's.
    pub heat: f64,
    /// The radiator runs: powered and not silent (`elite_sim::heat::visual_dissipation`'s switch).
    pub radiator: bool,
    /// The radiator's visual values from `heat` (`elite_sim::heat::visual_dissipation`, FUN_144399d90).
    pub dissipation: VisualDissipation,
    /// A test's placing (`sw put`) happened this frame: the machines are settled to what it set (the gear up), not animated.
    pub settle: bool,
    /// `sw lights heat F|auto`: a test's heat fraction over the ship's.
    pub dev_heat: Option<f64>,
    /// `sw pose up|down|auto`: a test's gear command over the pilot's (seated, the free camera can't look at the ship).
    pub dev_gear: Option<bool>,
}

impl ShipFacts {
    /// From the pilot's state (which belongs to the ship, parked or flown: `Pilot::boarding`) and the seat, the tests'
    /// overrides kept from `prev`.
    pub fn of(pilot: &Pilot, seated: bool, prev: &ShipFacts) -> ShipFacts {
        use elite_sim::drive::Drive;
        let jump = match pilot.drive {
            Drive::Charging(_) | Drive::JumpCharging(_) => JumpPhase::Charging,
            Drive::Departing(_) | Drive::Tunnel => JumpPhase::Departing,
            Drive::Arriving(_) | Drive::Arrived(_) => JumpPhase::Arriving,
            _ => JumpPhase::Idle,
        };
        let heat = prev.dev_heat.unwrap_or_else(|| pilot.heat.fraction(&pilot.heat_spec));
        let radiator = pilot.powered && !pilot.silent;
        ShipFacts {
            powered: pilot.powered,
            silent: pilot.silent,
            destroyed: pilot.destroyed,
            hardpoints: pilot.weapons.deployed,
            landed: pilot.landed,
            gear_down: prev.dev_gear.unwrap_or(!seated || pilot.gear_down),
            canopy_intact: pilot.life.canopy,
            seated,
            jump,
            heat,
            radiator,
            dissipation: visual_dissipation(heat, radiator),
            settle: pilot.put,
            dev_heat: prev.dev_heat,
            dev_gear: prev.dev_gear,
        }
    }

    /// A generic parameter the ship's facts feed, by name (the radiator's two visual values); None for any other.
    pub fn param(&self, name: &str) -> Option<f64> {
        match name {
            "HeatRadiator_VisualActiveDissipation" => Some(self.dissipation.active),
            "HeatRadiator_VisualPassiveDissipation" => Some(self.dissipation.passive),
            _ => None,
        }
    }
}

/// Writes the facts once a frame (before `parts::update`), only when one changed.
pub(super) fn update(pilot: Res<Pilot>, seat: Res<Seat>, mut facts: ResMut<ShipFacts>) {
    let now = ShipFacts::of(&pilot, seat.seated, &facts);
    if *facts != now {
        *facts = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The facts follow the pilot; nobody seated, the gear is commanded down; the tests' overrides win and are kept.
    #[test]
    fn the_facts_and_their_overrides() {
        let mut pilot = Pilot::default();
        pilot.gear_down = false;
        let f = ShipFacts::of(&pilot, true, &ShipFacts::default());
        assert!(!f.gear_down && f.powered && f.radiator && f.canopy_intact && !f.settle);
        assert!((f.heat - pilot.heat.fraction(&pilot.heat_spec)).abs() < 1e-12);
        assert_eq!(f.param("HeatRadiator_VisualActiveDissipation"), Some(visual_dissipation(f.heat, true).active));
        assert_eq!(f.param("cockpitAmberLightIntensity"), None);
        assert!(ShipFacts::of(&pilot, false, &f).gear_down);
        let dev = ShipFacts { dev_heat: Some(1.6), dev_gear: Some(true), ..f };
        let g = ShipFacts::of(&pilot, true, &dev);
        assert!(g.gear_down && g.heat == 1.6 && g.dissipation.active == 1.0 && g.dev_heat == Some(1.6) && g.dev_gear == Some(true));
        pilot.silent = true;
        assert_eq!(ShipFacts::of(&pilot, true, &dev).dissipation, VisualDissipation::default());
    }
}
