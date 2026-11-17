//! The ship's facts this frame (ARCHITECTURE.md A6, "Who reads whom"): one resource, `ShipFacts`, written once a frame from
//! `Pilot` and `Seat` before the parts run, read by the machines (`machines.rs`, `parts.rs`), the lights and the sound. It
//! keeps the tests' overrides of those facts (`sw lights heat`, `sw pose`, `sw canopy damage`), so no reader holds another's. Nothing of the
//! parts, the lights or the sound is read here. The pilot chair (`chair.rs`) reads the controls and the cockpit's changes of
//! velocity, tick by tick, from here too.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use elite_sim::heat::{visual_dissipation, VisualDissipation};

use super::panels::{Panels, Side};
use super::pilot::{Pilot, PilotKeys, TickChanges};
use super::seat::Seat;

/// The ship's damage the cockpit's cracks read (0..1): the cockpit's `Damage` parameter is bound to the ship's `ShipDamage`
/// (FUN_1430c6130, lines 178-188). E: the parameter's name is `ShipDamage`; its writer was not traced (the name's only
/// reference in the exe's strings is that binding); taken as the hull's damage share, 1 minus the hull's share of its full
/// strength (`Pilot::hull`).
pub fn ship_damage(pilot: &Pilot) -> f32 {
    (1.0 - pilot.hull.strength / pilot.hull.max.max(1e-9)).clamp(0.0, 1.0) as f32
}

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
    /// The side panels open (left, right), as the panels stand (`panels::Panels::open`, written by `update`): the GUI panel
    /// lamps' fades.
    pub panels_open: [bool; 2],
    /// The ship's damage the cracks read: a test's `sw canopy damage F` over `ship_damage`.
    pub damage: f32,
    /// The headlights' switch (`Option_HeadlightsEnabled`): the pilot's lights switch (`Pilot::lights`).
    pub headlights: bool,
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
    /// `sw canopy damage F|auto`: a test's ship damage over `ship_damage`.
    pub dev_damage: Option<f32>,
    /// The flight's demands this frame (`Pilot::controls`, the flight's signs): the throttle (forward +1, reverse -1) and the
    /// turn (pitch nose up +, yaw nose left +, roll right wing up +), each -1..1.
    pub throttle: f64,
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
    /// The primary trigger held: its key or button (`PilotKeys::fire`, as the lasers read it) or a test's `sw fire SECS`.
    pub fire: bool,
    /// The cockpit point's change of velocity of each of this frame's flight ticks, in order, in the ship's axes (m/s):
    /// `Pilot::tick_dv` when the flight recorded it this frame, else none (`Pilot::record_tick` gives the rule).
    pub ticks: TickChanges,
}

impl ShipFacts {
    /// From the pilot's state (which belongs to the ship, parked or flown: `Pilot::boarding`), the seat's keys, the seat, this
    /// frame's number (`FrameCount`: the flight's ticks are taken only if they are this frame's) and the tests' overrides
    /// kept from `prev` (and the panels' state, which `update` writes).
    pub fn of(pilot: &Pilot, keys: &PilotKeys, seated: bool, frame: u32, prev: &ShipFacts) -> ShipFacts {
        use elite_sim::drive::Drive;
        let jump = match pilot.drive {
            Drive::Charging(_) | Drive::JumpCharging(_) => JumpPhase::Charging,
            Drive::Departing(_) | Drive::Tunnel => JumpPhase::Departing,
            Drive::Arriving(_) | Drive::Arrived(_) => JumpPhase::Arriving,
            _ => JumpPhase::Idle,
        };
        let heat = prev.dev_heat.unwrap_or_else(|| pilot.heat.fraction(&pilot.heat_spec));
        let radiator = pilot.powered && !pilot.silent;
        let controls = pilot.controls();
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
            panels_open: prev.panels_open,
            damage: prev.dev_damage.unwrap_or_else(|| ship_damage(pilot)),
            headlights: pilot.lights,
            heat,
            radiator,
            dissipation: visual_dissipation(heat, radiator),
            settle: pilot.put,
            dev_heat: prev.dev_heat,
            dev_gear: prev.dev_gear,
            dev_damage: prev.dev_damage,
            throttle: controls.forward,
            pitch: controls.pitch,
            yaw: controls.yaw,
            roll: controls.roll,
            fire: keys.fire || pilot.dev_fire > 0.0,
            ticks: if pilot.tick_dv.frame == frame { pilot.tick_dv } else { TickChanges::default() },
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

/// Writes the facts once a frame (before `parts::update` and `chair::update`), only when one changed.
pub(super) fn update(pilot: Res<Pilot>, keys: Res<PilotKeys>, seat: Res<Seat>, panels: Res<Panels>, frame: Res<FrameCount>, mut facts: ResMut<ShipFacts>) {
    let panels_open = [panels.open == Some(Side::Left), panels.open == Some(Side::Right)];
    let now = ShipFacts { panels_open, ..ShipFacts::of(&pilot, &keys, seat.seated, frame.0, &facts) };
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
        let keys = PilotKeys::default();
        let f = ShipFacts::of(&pilot, &keys, true, 0, &ShipFacts::default());
        assert!(!f.gear_down && f.powered && f.radiator && f.canopy_intact && !f.settle);
        assert!((f.heat - pilot.heat.fraction(&pilot.heat_spec)).abs() < 1e-12);
        assert_eq!(f.param("HeatRadiator_VisualActiveDissipation"), Some(visual_dissipation(f.heat, true).active));
        assert_eq!(f.param("cockpitAmberLightIntensity"), None);
        assert!(ShipFacts::of(&pilot, &keys, false, 0, &f).gear_down);
        assert!(f.damage == 0.0 && f.panels_open == [false; 2] && !f.headlights, "a whole hull, the lights off");
        let dev = ShipFacts { dev_heat: Some(1.6), dev_gear: Some(true), dev_damage: Some(0.65), panels_open: [true, false], ..f };
        let g = ShipFacts::of(&pilot, &keys, true, 0, &dev);
        assert!(g.gear_down && g.heat == 1.6 && g.dissipation.active == 1.0 && g.dev_heat == Some(1.6) && g.dev_gear == Some(true));
        assert!(g.damage == 0.65 && g.dev_damage == Some(0.65) && g.panels_open == [true, false], "the damage's override; the panels kept");
        // The damage is the hull's lost share (none past empty), the headlights the pilot's switch.
        pilot.lights = true;
        for (left, want) in [(0.5, 0.5), (0.2, 0.8), (0.05, 0.95), (0.0, 1.0)] {
            pilot.hull.strength = left * pilot.hull.max;
            let h = ShipFacts::of(&pilot, &keys, true, 0, &f);
            assert!((h.damage - want).abs() < 1e-6 && h.headlights, "{left}: {}", h.damage);
        }
        assert_eq!(ShipFacts::of(&pilot, &keys, true, 0, &dev).damage, 0.65, "the test's override over the hull's");
        pilot.silent = true;
        assert_eq!(ShipFacts::of(&pilot, &keys, true, 0, &dev).dissipation, VisualDissipation::default());
        // The trigger: its key, or a test's.
        assert!(!f.fire && ShipFacts::of(&pilot, &PilotKeys { fire: true, ..keys }, true, 0, &f).fire);
        pilot.dev_fire = 1.0;
        assert!(ShipFacts::of(&pilot, &keys, true, 0, &f).fire);
    }
}
