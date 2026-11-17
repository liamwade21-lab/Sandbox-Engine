//! The ship's facts this frame (ARCHITECTURE.md A6, "Who reads whom"): one resource, `ShipFacts`, written once a frame from
//! `Pilot` and `Seat` before the parts run, read by the machines (`machines.rs`, `parts.rs`), the lights and the sound. It
//! keeps the tests' overrides of those facts (`sw lights heat`, `sw pose`, `sw canopy damage`), so no reader holds another's. Nothing of the
//! parts, the lights or the sound is read here. The pilot chair (`chair.rs`) reads the controls and the cockpit's change of
//! velocity from here too.

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use elite_sim::heat::{visual_dissipation, VisualDissipation};

use super::panels::{Panels, Side};
use super::pilot::{Pilot, PilotKeys};
use super::seat::Seat;
use super::SidewinderAssets;

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
    /// The cockpit point's velocity in the flight's rest frame (m/s): the ship's, plus its spin about the ship's origin
    /// carried to the cockpit (v + w x r; Elite's cockpit-point velocity, `sidewinder_chair.md` "Buffets").
    pub cockpit_velocity: DVec3,
    /// The flight's motion is the ship's this frame: not in supercruise (whose speed and turn live in `Cruise`), not in the
    /// tunnel, not on a test's placing (which resets it).
    pub motion_live: bool,
    /// The cockpit point's change of velocity since the last frame, in the ship's space (m/s); zero unless the motion was
    /// the ship's this frame and the last.
    pub cockpit_dv: Vec3,
}

impl ShipFacts {
    /// From the pilot's state (which belongs to the ship, parked or flown: `Pilot::boarding`), the seat's keys, the seat and
    /// the cockpit point (`SidewinderAssets::eye`, the ship's space), the tests' overrides and the last cockpit velocity
    /// kept from `prev` (and the panels' state, which `update` writes).
    pub fn of(pilot: &Pilot, keys: &PilotKeys, seated: bool, cockpit: Vec3, prev: &ShipFacts) -> ShipFacts {
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
        // (The flight's motion: `vel` in the rest frame, `rot` ship axes to it, `omega` in ship axes; elite_sim's glam, so
        // through arrays. The ship's axes are the ship's space's: X right, Y up, -Z the nose.)
        let rot = DQuat::from_array(pilot.motion.rot.to_array());
        let omega = DVec3::from_array(pilot.motion.omega.to_array());
        let cockpit_velocity = DVec3::from_array(pilot.motion.vel.to_array()) + rot * omega.cross(cockpit.as_dvec3());
        let motion_live = pilot.cruise.is_none() && pilot.jump.is_none() && !pilot.put;
        let cockpit_dv = if motion_live && prev.motion_live { (rot.inverse() * (cockpit_velocity - prev.cockpit_velocity)).as_vec3() } else { Vec3::ZERO };
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
            cockpit_velocity,
            motion_live,
            cockpit_dv,
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
pub(super) fn update(pilot: Res<Pilot>, keys: Res<PilotKeys>, seat: Res<Seat>, panels: Res<Panels>, assets: Res<SidewinderAssets>, mut facts: ResMut<ShipFacts>) {
    let panels_open = [panels.open == Some(Side::Left), panels.open == Some(Side::Right)];
    let now = ShipFacts { panels_open, ..ShipFacts::of(&pilot, &keys, seat.seated, Vec3::from_array(assets.eye), &facts) };
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
        let f = ShipFacts::of(&pilot, &keys, true, Vec3::ZERO, &ShipFacts::default());
        assert!(!f.gear_down && f.powered && f.radiator && f.canopy_intact && !f.settle);
        assert!((f.heat - pilot.heat.fraction(&pilot.heat_spec)).abs() < 1e-12);
        assert_eq!(f.param("HeatRadiator_VisualActiveDissipation"), Some(visual_dissipation(f.heat, true).active));
        assert_eq!(f.param("cockpitAmberLightIntensity"), None);
        assert!(ShipFacts::of(&pilot, &keys, false, Vec3::ZERO, &f).gear_down);
        assert!(f.damage == 0.0 && f.panels_open == [false; 2] && !f.headlights, "a whole hull, the lights off");
        let dev = ShipFacts { dev_heat: Some(1.6), dev_gear: Some(true), dev_damage: Some(0.65), panels_open: [true, false], ..f };
        let g = ShipFacts::of(&pilot, &keys, true, Vec3::ZERO, &dev);
        assert!(g.gear_down && g.heat == 1.6 && g.dissipation.active == 1.0 && g.dev_heat == Some(1.6) && g.dev_gear == Some(true));
        assert!(g.damage == 0.65 && g.dev_damage == Some(0.65) && g.panels_open == [true, false], "the damage's override; the panels kept");
        // The damage is the hull's lost share (none past empty), the headlights the pilot's switch.
        pilot.lights = true;
        for (left, want) in [(0.5, 0.5), (0.2, 0.8), (0.05, 0.95), (0.0, 1.0)] {
            pilot.hull.strength = left * pilot.hull.max;
            let h = ShipFacts::of(&pilot, &keys, true, Vec3::ZERO, &f);
            assert!((h.damage - want).abs() < 1e-6 && h.headlights, "{left}: {}", h.damage);
        }
        assert_eq!(ShipFacts::of(&pilot, &keys, true, Vec3::ZERO, &dev).damage, 0.65, "the test's override over the hull's");
        pilot.silent = true;
        assert_eq!(ShipFacts::of(&pilot, &keys, true, Vec3::ZERO, &dev).dissipation, VisualDissipation::default());
    }

    /// The cockpit point's change of velocity: the ship's own change turned into its axes, plus the spin carried to the
    /// cockpit; nothing on the first frame, in supercruise or on the frame after it; the trigger from the key or a test.
    #[test]
    fn the_cockpits_change_of_velocity() {
        let keys = PilotKeys::default();
        let cockpit = Vec3::new(0.0, 6.7, -1.3);
        let mut pilot = Pilot::default();
        // (The ship turned 90 degrees left about its up axis: its nose, -Z in its axes, points along -X of the rest frame.)
        pilot.motion.rot = glam::DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2);
        let first = ShipFacts::of(&pilot, &keys, true, cockpit, &ShipFacts::default());
        assert!(first.motion_live && first.cockpit_dv == Vec3::ZERO && !first.fire);
        // 3 m/s more along the rest frame's -X: straight ahead of the ship, its -Z.
        pilot.motion.vel = glam::DVec3::new(-3.0, 0.0, 0.0);
        let next = ShipFacts::of(&pilot, &keys, true, cockpit, &first);
        assert!((next.cockpit_dv - Vec3::new(0.0, 0.0, -3.0)).length() < 1e-5, "{}", next.cockpit_dv);
        // A spin of 1 rad/s about the ship's up axis (to the left) moves the cockpit (1.3 m ahead of the origin) to the left.
        pilot.motion.omega = glam::DVec3::new(0.0, 1.0, 0.0);
        let spun = ShipFacts::of(&pilot, &keys, true, cockpit, &next);
        assert!((spun.cockpit_dv - Vec3::new(-1.3, 0.0, 0.0)).length() < 1e-5, "{}", spun.cockpit_dv);
        // Supercruise's motion isn't the flight's: nothing then, nor on the frame after.
        pilot.motion.vel = glam::DVec3::new(-500.0, 0.0, 0.0);
        pilot.cruise = Some(super::super::pilot::Cruise { pos: Default::default(), rot: DQuat::IDENTITY, speed: 3e4, cap: 3e4, carrier: None });
        let cruise = ShipFacts::of(&pilot, &keys, true, cockpit, &spun);
        pilot.cruise = None;
        let after = ShipFacts::of(&pilot, &keys, true, cockpit, &cruise);
        assert!(!cruise.motion_live && cruise.cockpit_dv == Vec3::ZERO && after.motion_live && after.cockpit_dv == Vec3::ZERO);
        pilot.dev_fire = 1.0;
        assert!(ShipFacts::of(&pilot, &keys, true, cockpit, &after).fire);
        assert!(ShipFacts::of(&Pilot::default(), &PilotKeys { fire: true, ..keys }, true, cockpit, &after).fire);
    }
}
