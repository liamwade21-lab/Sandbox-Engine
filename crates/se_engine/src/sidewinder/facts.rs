//! The ship's facts this frame (ARCHITECTURE.md A6, "Who reads whom"): one resource, `ShipFacts`, written once a frame from
//! `Pilot` and `Seat` before the parts run, read by the machines (`machines.rs`, `parts.rs`), the lights and the sound. It
//! keeps the tests' overrides of those facts (`sw lights heat`, `sw pose`, `sw canopy damage`), so no reader holds another's. Nothing of the
//! parts, the lights or the sound is read here. The pilot chair (`chair.rs`) reads the controls and the cockpit's changes of
//! velocity, tick by tick, from here too, and the sound its engine inputs' numbers (`SoundMotion`).

use bevy::diagnostic::FrameCount;
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use elite_sim::heat::{visual_dissipation, VisualDissipation};

use super::panels::{Panels, Side};
use super::pilot::{Pilot, PilotKeys, TickChanges, TICK};
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

/// What the sound's engine inputs read from the flight this frame (ARCHITECTURE.md A7, "The engine's inputs"): what Elite's
/// audio update reads from the flight controller and the physics world (`re/elite/notes/sidewinder_sound_events.md`, T5e
/// items 2, 4-6), each value from the function the flight itself flies by. The sound works its inputs out from these alone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SoundMotion {
    /// The full-power block's top speed, the current block's top speed and forward acceleration (m/s, m/s^2): `Pilot::block`
    /// (Elite's controller +0x81c [0], +0x750 [0] and [0x1C]).
    pub full_top: f64,
    pub top: f64,
    pub accel_fwd: f64,
    /// The speed-limited turn rates the controller divides the spin by (rad/s; x pitch, y yaw, z roll):
    /// `elite_sim::flight::share_limits` of the current block at the speed now.
    pub share_limits: [f64; 3],
    /// The physics time simulated this frame: the flight's ticks this frame x the tick (s; 0 in a frame with none).
    pub physics_dt: f64,
    /// The velocity and the spin in the ship's axes (m/s, rad/s; x right, y up, z back; spin x pitch nose up, y yaw nose left,
    /// z roll right wing up).
    pub vel: [f64; 3],
    pub omega: [f64; 3],
    /// The thrusters' mass multiplier at the ship's mass now (`elite_sim::data::thrusters_mass_k`).
    pub mass_k: f64,
}

impl SoundMotion {
    /// From the current block and the full-power top speed (`Pilot::block`), the velocity and spin in the ship's axes, the
    /// flight ticks run this frame and the ship's mass (t).
    pub fn new(block: &elite_sim::flight::Block, full_top: f64, vel: [f64; 3], omega: [f64; 3], ticks: usize, mass_t: f64) -> SoundMotion {
        let speed = vel.iter().map(|v| v * v).sum::<f64>().sqrt();
        SoundMotion {
            full_top,
            top: block.top_speed,
            accel_fwd: block.accel_fwd,
            share_limits: elite_sim::flight::share_limits(block, speed).to_array(),
            physics_dt: ticks as f64 * TICK,
            vel,
            omega,
            mass_k: elite_sim::data::thrusters_mass_k(mass_t),
        }
    }

    /// From the pilot's flight and the number of flight ticks run this frame.
    fn of(pilot: &Pilot, ticks: usize) -> SoundMotion {
        let (block, full_top) = pilot.block();
        // (elite_sim's glam may not be Bevy's: through arrays.)
        let rot = DQuat::from_array(pilot.motion.rot.to_array());
        let vel = rot.inverse() * DVec3::from_array(pilot.motion.vel.to_array());
        SoundMotion::new(&block, full_top, vel.to_array(), pilot.motion.omega.to_array(), ticks, pilot.mass_t())
    }
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
    /// What the sound's engine inputs read from the flight (`SoundMotion`).
    pub motion: SoundMotion,
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
        let ticks = if pilot.tick_dv.frame == frame { pilot.tick_dv } else { TickChanges::default() };
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
            ticks,
            motion: SoundMotion::of(pilot, ticks.as_slice().len()),
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

    /// The sound's numbers are the flight's: the blocks it flies by (`Pilot::block`: the pips, the mass, the gear's factor),
    /// the velocity in the ship's axes, the controller's divisors at that speed, the ticks' physics time, the thrusters'
    /// mass multiplier.
    #[test]
    fn the_sound_motion_is_the_flights() {
        let mut pilot = Pilot::default();
        pilot.gear_down = false;
        // The nose (-z) turned to the world's -x, flying that way at 100 m/s, spinning.
        pilot.motion.rot = glam::DQuat::from_array(DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2).to_array());
        pilot.motion.vel = glam::DVec3::new(-100.0, 0.0, 0.0);
        pilot.motion.omega = glam::DVec3::new(0.1, -0.2, 0.3);
        let m = ShipFacts::of(&pilot, &PilotKeys::default(), true, 0, &ShipFacts::default()).motion;
        let (block, full_top) = pilot.block();
        assert!((DVec3::from_array(m.vel) - DVec3::new(0.0, 0.0, -100.0)).length() < 1e-9, "{:?}", m.vel);
        assert_eq!((m.full_top, m.top, m.accel_fwd, m.omega), (full_top, block.top_speed, block.accel_fwd, [0.1, -0.2, 0.3]));
        let w = elite_sim::flight::share_limits(&block, 100.0).to_array();
        assert!(m.share_limits.iter().zip(w).all(|(a, b)| (a - b).abs() < 1e-12), "{:?}", m.share_limits);
        assert_eq!(m.mass_k, elite_sim::data::thrusters_mass_k(pilot.mass_t()));
        assert_eq!(m.physics_dt, 0.0, "no tick this frame");
        assert_eq!(SoundMotion::new(&block, full_top, m.vel, m.omega, 3, pilot.mass_t()).physics_dt, 3.0 * TICK);
        // The gear down halves both top speeds (the flight's own factor).
        pilot.gear_down = true;
        let down = ShipFacts::of(&pilot, &PilotKeys::default(), true, 0, &ShipFacts::default()).motion;
        assert!((down.full_top - 0.5 * full_top).abs() < 1e-9 && (down.top - 0.5 * block.top_speed).abs() < 1e-9);
    }
}
