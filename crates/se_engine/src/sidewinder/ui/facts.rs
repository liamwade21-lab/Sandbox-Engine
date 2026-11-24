//! The flight HUD's facts from the game (ARCHITECTURE.md A10 "Data (UI4, UI5)", task UI4b): `HudFacts` holds this frame's
//! `elite_ui::UiFacts`, filled once a frame while seated (`fill`, in `UiSet::Feed` after the flight and `ShipFacts`) from the
//! mod's state, and read by the feeders (`feed.rs`) and the test command `sw ui facts`. Nothing of the flight reads it.
//!
//! `facts_of` is the whole mapping, one line per field naming its source. A field the mod has no source for is `None` (or
//! false) with its reason, and its message is not sent (the user's scope rule). Grades as the project's: B a record's or
//! community data's number, E ours where Elite's rule was not decoded.

use bevy::math::{DQuat, DVec3, Vec3};
use bevy::prelude::*;
use elite_sim::drive::Drive;
use elite_sim::weapons::Weapons;
use elite_ui::facts::{
    location_type, Alerts, Channels, Clock, Damage, Deploy, Destination, Drive as UiDrive, Fuel, Hardpoint, Heat, Landing, LifeAlarm, PipKeys, Planet, Power, Shield, ShieldState,
    Signature, Speed, Status, Weapons as UiWeapons,
};
use elite_ui::UiFacts;
use se_core::catalog::ObjectRef;

use super::super::facts::ShipFacts;
use super::super::panels::{M_LASERS, M_SHIELDS, M_THRUSTERS};
use super::super::pilot::{Pilot, PilotKeys};
use super::super::seat::Seat;
use super::super::Sidewinder;
use super::UiSet;
use crate::mods::ModControl;

/// The frame's facts, and what the burn rate is worked out from.
#[derive(Resource, Default)]
pub struct HudFacts {
    pub facts: UiFacts,
    /// The fuel aboard at the last fill (t), and the burn rate last read (t/s).
    last_fuel_t: Option<f64>,
    burn_t_per_s: f64,
}

/// The stock Sidewinder's lasers as Elite names them (the right panel's Fire Groups page in session ui1 lists the two pulse
/// lasers "1G" under BEAM, both on primary fire in group A, the only group): the item `Hpt_PulseLaser_Gimbal_Small`, its
/// item id (B: coriolis's `edID` for it, `re/elite/community/coriolis-data/pulse_laser.json`, the record `elite_sim`'s
/// `PULSE_LASER_GIMBAL_SMALL` is taken from), its name as the string table gives it (`hpt_pulselaser_gimbal_small_name` in
/// `ui/strings/en.json`: a test checks it), its weapon type (the fire groups' category, 0 beam: `elite_ui::panel_facts`)
/// and its mount (WeaponModes 1, gimballed).
const LASER_ITEM_ID: u32 = 128_049_385;
const LASER_NAME: &str = "Pulse Laser";
const LASER_WEAPON_TYPE: i32 = 0;
const LASER_WEAPON_MODE: i32 = 1;
/// The fire group's answer for both lasers: 1, primary fire (session ui1's Fire Groups page); the active group A (0).
const LASER_TRIGGER: u8 = 1;
const FIRE_GROUP_A: u8 = 0;

/// Everything `facts_of` reads, borrowed for the frame.
pub struct Sources<'a> {
    pub pilot: &'a Pilot,
    pub ship: &'a ShipFacts,
    pub keys: &'a PilotKeys,
    /// The pilot sits in the seat with the mod on.
    pub seated: bool,
    /// The parked pose (from its body's centre, m; model space to the body's frame) and that body's mean radius (m).
    pub place: Option<(DVec3, DQuat, f64)>,
    /// The game's date (Julian day).
    pub jd: f64,
    /// The burn rate this frame (`burn_rate`).
    pub burn_t_per_s: f64,
}

/// The flight HUD's facts from the mod's state.
pub fn facts_of(s: &Sources) -> UiFacts {
    let (p, f) = (s.pilot, s.ship);
    let speed = DVec3::from_array(f.motion.vel).length();
    let shields_on = elite_sim::silent::shields_powered(p.powered && p.modules[M_SHIELDS], p.silent);
    UiFacts {
        status: Status {
            seated: s.seated,                // the seat and `ModControl::pilot` (as the UI's own `seated`)
            powered: f.powered,              // `ShipFacts::powered` (the plant runs: fuel aboard)
            gear_down: f.gear_down,          // `ShipFacts::gear_down`, the gear's command (Elite's +0xBB)
            cargo_scoop: p.scoop,            // `Pilot::scoop`
            mass_locked: false,              // no source: the mod has no mass lock (`elite_sim::drive`: "nothing locks the mass")
        },
        speed: Speed {
            supercruise: p.cruise.is_some(),          // `Pilot::cruise`
            speed_m_s: speed,                         // |`ShipFacts::motion.vel`| (the stand-in HUD's number)
            forward_m_s: -f.motion.vel[2],            // the nose is -z in ship space: forward positive
            top_m_s: f.motion.top,                    // `ShipFacts::motion.top`: the pips', mass's and gear's block
            full_top_m_s: f.motion.full_top,          // `ShipFacts::motion.full_top`
            engines_on: p.powered && p.modules[M_THRUSTERS], // the flight's own test for thrust (`fly`)
            demand: f.throttle,                       // `ShipFacts::throttle` (the flight's forward demand, -1..1)
            flight_assist: p.flight_assist,           // `Pilot::flight_assist`
        },
        heat: Heat { prop: f.heat },                  // `ShipFacts::heat`, heat / capacity (a test's `sw lights heat` over it)
        fuel: Fuel {
            main_t: p.fuel.main,                      // `Pilot::fuel`, t
            main_capacity_t: p.fuel.main_cap,
            reservoir_t: p.fuel.reservoir,
            reservoir_capacity_t: p.fuel.reservoir_cap,
            burn_t_per_s: s.burn_t_per_s,             // `burn_rate`: the fuel the flight's ticks used, a second
            jump_cost_t: jump_cost(p),                // `Pilot::jump_fuel` for the locked system
        },
        damage: Damage {
            shield: Shield {
                strength: p.shields.strength / p.shields.max.max(1e-9), // `Pilot::shields`, 0..1
                state: shield_state(p.shields.up, p.modules[M_SHIELDS], shields_on),
            },
            hull: p.hull.strength / p.hull.max.max(1e-9), // `Pilot::hull`, 0..1
        },
        power: Power {
            pips: channels(|c| f64::from(p.dist.half[c]) / f64::from(elite_sim::power::MOST_HALF)), // `Pilot::dist`: half-pips / 8
            capacitors: channels(|c| p.dist.charge[c] / p.dist.caps[c].capacity.max(1e-9)), // charge / capacity
            keys: PipKeys { eng: s.keys.pip_eng, sys: s.keys.pip_sys, wep: s.keys.pip_wep, reset: s.keys.pip_reset }, // `PilotKeys`, the presses the flight took
        },
        signature: Signature {
            channels: None,                           // no source: what Elite's two channels are was not found (E)
            silent: f.silent,                         // `ShipFacts::silent`
        },
        destination: p.dest.as_ref().map(|m| Destination {
            kind: location_kind(m.system, m.obj),     // `DestMark::system` and `obj`
            name: m.name.clone(),                     // `DestMark::name` (the catalog's display name)
            distance_m: m.dist_m,                     // `DestMark::dist_m`: what the stand-in's DESTINATION line reads
            dir: elite_axes(m.dir),                   // `DestMark::dir`, ship space to Elite's axes
        }),
        drive: UiDrive {
            charge: p.drive.charge_share(),           // `Pilot::drive`: the charge's share (the stand-in's bar)
            cooldown: cooldown_done(p.drive),         // the share of the cool-down done
        },
        weapons: UiWeapons {
            armed: p.weapons.deployed,                // `Pilot::weapons.deployed`
            firegroup: Some(FIRE_GROUP_A),            // the stock ship's one group
            hardpoints: Some(lasers(&p.weapons, M_LASERS.map(|m| p.modules[m]), p.powered)),
        },
        alerts: Alerts {
            life_support: life_alarm(p.life.alarm()), // `Pilot::life` (Elite's alarm rule, `elite_sim::life`)
            oxygen_s: p.life.oxygen,                  // the emergency oxygen left, s
        },
        clock: Clock { time_of_day_s: Some(time_of_day_s(s.jd)) }, // the game's date (`SimTime::jd`, UTC)
        planet: p.planet.zip(s.place).map(|(h, (pos, rot, radius_m))| {
            let (radius_m, centre_distance_m) = planet_place(radius_m, h.alt_m);
            // (The ship's up axis against the radial up at its place: the landing guide's alignment, below.)
            let up_dot = (rot * DVec3::Y).dot(pos.normalize_or(DVec3::Y));
            Planet {
                radius_m,                             // the body's mean radius (`SystemBody::radius_m`)
                centre_distance_m,                    // radius + `PlanetHud::alt_m` (ours: `planet_place`)
                inner_height_m: None,                 // no source: Elite's body float H was not found (E)
                heading_deg: h.heading_deg,           // `PlanetHud`: clockwise from the body's north, 0..360
                pitch_deg: h.pitch_deg,               // the nose over the horizon, up positive
                roll_deg: -h.roll_deg,                // `PlanetHud` is right wing down positive; the field left wing down
                landing: (p.landed || h.alt_m <= elite_sim::landing::APPROACH_HEIGHT_M).then(|| Landing {
                    roll_deg: -h.roll_deg,            // stand-in (E): against the horizon, not the landing surface's normal
                    pitch_deg: h.pitch_deg,
                    axis_dot_normal: up_dot,          // stand-in (E): the radial up for the surface's normal
                    speed_m_s: speed,
                    speed_limit_m_s: elite_sim::landing::TOUCHDOWN_MPS, // the landing record's P10
                    suitable_terrain: None,           // no source: the mod has no suitable-terrain rule (the guide is not fed)
                    landed: p.landed,                 // `Pilot::landed`
                }),
            }
        }),
    }
}

/// A value for each channel, by `elite_sim::power`'s indices.
fn channels(v: impl Fn(usize) -> f64) -> Channels {
    use elite_sim::power::{ENG, SYS, WEP};
    Channels { wep: v(WEP), eng: v(ENG), sys: v(SYS) }
}

/// A direction in ship space (x right, y up, -z the nose) in Elite's ship axes (x right, y up, z forward): z mirrored.
pub fn elite_axes(dir: Vec3) -> [f64; 3] {
    [f64::from(dir.x), f64::from(dir.y), -f64::from(dir.z)]
}

/// The radius and the centre distance handed to the orbital HUD for a ship `alt_m` over the loaded ground of a body of mean
/// radius `radius_m` (ours: the movie's altimeter is radius based, d - R, and SpaceEngine's ground stands kilometres off the
/// mean radius, so d is the radius plus the mod's own altimeter number: the movie then shows the height over the ground,
/// as the stand-in's ALT does, landed 0, not 15 km; the user's earlier decision).
pub fn planet_place(radius_m: f64, alt_m: f64) -> (f64, f64) {
    (radius_m, radius_m + alt_m)
}

/// A destination's LocationType: another system's star a system, a star of this system a star, every body a planet (Elite's
/// icons have no moon; its landable and atmosphere variants are left out: the mod sets down on any loaded ground, and Elite's
/// landable rule is not the mod's: E).
pub fn location_kind(system: bool, obj: ObjectRef) -> i32 {
    match obj {
        _ if system => location_type::SYSTEM,
        ObjectRef::Star(_) => location_type::STAR,
        _ => location_type::PLANET,
    }
}

/// The fuel burnt a second by this frame's flight ticks: the fuel aboard that went (t) over the physics time they simulated
/// (s, `ShipFacts::motion.physics_dt`). No new reading (the last stands) in the first frame, a frame without a tick, one where
/// the fuel rose (a restock, a new ship) or the hyperspace jump's frame (`jumped`: the jump's fuel goes in that frame).
/// Smoothed only by the feeder.
pub fn burn_rate(last_fuel_t: Option<f64>, fuel_t: f64, physics_dt: f64, jumped: bool, last_rate: f64) -> f64 {
    match last_fuel_t {
        Some(last) if physics_dt > 0.0 && !jumped && fuel_t <= last => (last - fuel_t) / physics_dt,
        _ => last_rate,
    }
}

/// The jump's fuel for the locked system: `Pilot::jump_fuel` while it names the target (worked out when the jump's charge
/// started; the mod keeps no cost for a locked system before that, so the bar's overlay shows only during a charge).
fn jump_cost(p: &Pilot) -> Option<f64> {
    p.jump_fuel.filter(|(t, _)| Some(*t) == p.target).map(|(_, fuel)| fuel)
}

/// The shield zone's state (E: which of Elite's states the mod's cases are was not decoded): up, active; down with the
/// generator switched off, turned off; down and powered (it comes back after its delay), recharging; down unpowered (the
/// plant off or silent running), offline.
fn shield_state(up: bool, module_on: bool, powered: bool) -> ShieldState {
    match (up, module_on, powered) {
        (true, _, _) => ShieldState::Active,
        (false, false, _) => ShieldState::TurnedOff,
        (false, true, true) => ShieldState::Recharging,
        (false, true, false) => ShieldState::Offline,
    }
}

/// The share of the drive's cool-down done, 0 at the drop and 1 when the drive takes the key again (the facts' rule; E,
/// Elite's direction was not decoded): over the mod's whole span of `Refusal::Cooldown`, after a drop `Arriving` then
/// `Cooldown`, after a jump `Arrived`. None otherwise.
pub fn cooldown_done(drive: Drive) -> Option<f64> {
    use elite_sim::drive::{stand_in::JUMP_COOLDOWN_S, ARRIVING_AFTER_JUMP_S, ARRIVING_S, COOLDOWN_S};
    let (left, span) = match drive {
        Drive::Arriving(t) => (t + COOLDOWN_S, ARRIVING_S + COOLDOWN_S),
        Drive::Cooldown(t) => (t, ARRIVING_S + COOLDOWN_S),
        Drive::Arrived(t) => (t, ARRIVING_AFTER_JUMP_S + JUMP_COOLDOWN_S),
        _ => return None,
    };
    Some((1.0 - left / span).clamp(0.0, 1.0))
}

/// The hardpoints' deployment from the lasers' (`deployed` asked, `deploy_t` 0 in .. 1 out).
pub fn deploy_of(w: &Weapons) -> Deploy {
    match (w.deployed, w.ready()) {
        (true, true) => Deploy::Deployed,
        (true, false) => Deploy::Deploying,
        (false, _) if w.deploy_t > 0.0 => Deploy::Retracting,
        (false, _) => Deploy::Retracted,
    }
}

/// The two lasers as the ring is fed them: switched on (the Modules panel's `on`) and powered (the plant runs).
fn lasers(w: &Weapons, on: [bool; 2], powered: bool) -> Vec<Hardpoint> {
    on.iter()
        .map(|&module_on| Hardpoint {
            module_item_id: LASER_ITEM_ID,
            name: LASER_NAME.to_string(),
            weapon_type: LASER_WEAPON_TYPE,
            weapon_mode: LASER_WEAPON_MODE,
            // The hardpoints' travel, both ways (`elite_sim::weapons::stand_in::DEPLOY_S`, E).
            deploy_s: elite_sim::weapons::stand_in::DEPLOY_S,
            retract_s: elite_sim::weapons::stand_in::DEPLOY_S,
            deploy: deploy_of(w),
            module_on,
            powered,
            trigger: LASER_TRIGGER,
        })
        .collect()
}

fn life_alarm(a: elite_sim::life::Alarm) -> LifeAlarm {
    match a {
        elite_sim::life::Alarm::Off => LifeAlarm::Off,
        elite_sim::life::Alarm::On => LifeAlarm::On,
        elite_sim::life::Alarm::Critical => LifeAlarm::Critical,
    }
}

/// Seconds since midnight (UTC) of a Julian day (whose days start at noon).
pub fn time_of_day_s(jd: f64) -> f64 {
    (jd + 0.5).rem_euclid(1.0) * 86_400.0
}

/// Once a frame while seated (`UiSet::Feed`): the facts from the mod.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill(
    pilot: Res<Pilot>,
    ship_facts: Res<ShipFacts>,
    keys: Res<PilotKeys>,
    seat: Res<Seat>,
    control: Res<ModControl>,
    ship: Res<Sidewinder>,
    bodies: Query<&crate::system::SystemBody>,
    sim: Res<crate::universe::SimTime>,
    mut hud: ResMut<HudFacts>,
) {
    let fuel_t = pilot.fuel.total();
    hud.burn_t_per_s = burn_rate(hud.last_fuel_t, fuel_t, ship_facts.motion.physics_dt, pilot.jump.is_some(), hud.burn_t_per_s);
    hud.last_fuel_t = Some(fuel_t);
    let place = ship.parked.as_ref().and_then(|p| bodies.iter().find(|b| b.obj == p.obj).map(|b| (p.pos, p.rot, b.radius_m)));
    let sources = Sources { pilot: &pilot, ship: &ship_facts, keys: &keys, seated: seat.seated && control.pilot, place, jd: sim.jd, burn_t_per_s: hud.burn_t_per_s };
    hud.facts = facts_of(&sources);
}

/// `sw ui facts`: the frame's facts, one line a group (the heat also as the movie labels it, heatProp / 1.5; the burn rate
/// also per hour, as the gauge's text).
pub(super) fn log(world: &World) {
    let Some(hud) = world.get_resource::<HudFacts>() else {
        warn!("sw ui facts: none (Elite's UI is off)");
        return;
    };
    let f = &hud.facts;
    info!("sidewinder ui facts: status {:?}", f.status);
    info!("sidewinder ui facts: speed {:.3?}", f.speed);
    info!("sidewinder ui facts: heat {:.4} (the movie's label {:.1} %)", f.heat.prop, f.heat.prop / 1.5 * 100.0);
    info!("sidewinder ui facts: fuel {:.6?} ({:.3} t/h)", f.fuel, f.fuel.burn_t_per_s * 3600.0);
    info!("sidewinder ui facts: damage {:.4?}", f.damage);
    info!("sidewinder ui facts: power {:.4?}", f.power);
    info!("sidewinder ui facts: signature {:?}", f.signature);
    info!("sidewinder ui facts: destination {:.5?}", f.destination);
    info!("sidewinder ui facts: drive {:.3?}", f.drive);
    info!("sidewinder ui facts: weapons armed {}, group {:?}, {:?}", f.weapons.armed, f.weapons.firegroup, f.weapons.hardpoints.iter().flatten().map(|h| (h.deploy, h.module_on, h.powered)).collect::<Vec<_>>());
    info!("sidewinder ui facts: alerts {:.1?}, clock {:.0?}", f.alerts, f.clock.time_of_day_s);
    info!("sidewinder ui facts: planet {:.3?}", f.planet);
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<HudFacts>().add_systems(PostUpdate, fill.in_set(UiSet::Feed));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ship space's nose (-z) is Elite's +z; right and up stay.
    #[test]
    fn directions_in_elites_axes() {
        assert_eq!(elite_axes(Vec3::NEG_Z), [0.0, 0.0, 1.0]);
        assert_eq!(elite_axes(Vec3::Z), [0.0, 0.0, -1.0]);
        assert_eq!(elite_axes(Vec3::X), [1.0, 0.0, 0.0]);
        assert_eq!(elite_axes(Vec3::Y), [0.0, 1.0, 0.0]);
        let d = elite_axes(Vec3::new(0.6, -0.48, -0.64));
        assert!((d[0] - 0.6).abs() < 1e-6 && (d[1] + 0.48).abs() < 1e-6 && (d[2] - 0.64).abs() < 1e-6, "{d:?}");
    }

    /// The orbital HUD's own arithmetic (d - R) gives back the mod's altimeter number: 0 landed, whatever the radius.
    #[test]
    fn the_altitude_is_the_height_over_the_ground() {
        for (radius, alt) in [(6_371_000.0, 0.0), (6_371_000.0, 1234.5), (1_737_400.0, 99_999.0), (69_911_000.0, 15_000.0)] {
            let (r, d) = planet_place(radius, alt);
            assert_eq!(r, radius);
            assert!((d - r - alt).abs() < 1e-6, "{radius} {alt}: {}", d - r);
        }
    }

    #[test]
    fn location_types() {
        assert_eq!(location_kind(true, ObjectRef::Star(3)), location_type::SYSTEM);
        assert_eq!(location_kind(false, ObjectRef::Star(3)), location_type::STAR);
        assert_eq!(location_kind(false, ObjectRef::Body(2)), location_type::PLANET);
    }

    /// The burn is the fuel the ticks used over their time; no reading without ticks, when the fuel rose, in the jump's frame
    /// or first.
    #[test]
    fn burn_rate_from_ticks() {
        let idle = elite_sim::fuel::IDLE_T_PER_HOUR / 3600.0;
        let dt = 2.0 * super::super::super::pilot::TICK;
        let rate = burn_rate(Some(1.5), 1.5 - idle * dt, dt, false, 0.0);
        assert!((rate - idle).abs() < 1e-12, "{rate} against {idle}");
        assert_eq!(burn_rate(None, 1.5, dt, false, 0.25), 0.25, "first");
        assert_eq!(burn_rate(Some(1.5), 1.5, 0.0, false, 0.25), 0.25, "no tick");
        assert_eq!(burn_rate(Some(1.0), 2.3, dt, false, 0.25), 0.25, "a restock");
        assert_eq!(burn_rate(Some(1.5), 0.9, dt, true, 0.25), 0.25, "the jump's frame");
        assert_eq!(burn_rate(Some(0.0), 0.0, dt, false, 0.25), 0.0, "dry: nothing burnt");
    }

    /// The cool-down's share done: 0 at the drop, rising through Arriving and Cooldown to 1; after a jump over Arrived.
    #[test]
    fn cooldown_share_done() {
        use elite_sim::drive::{ARRIVING_S, COOLDOWN_S};
        assert_eq!(cooldown_done(Drive::Arriving(ARRIVING_S)), Some(0.0));
        let mid = cooldown_done(Drive::Cooldown(COOLDOWN_S)).unwrap();
        assert!((mid - ARRIVING_S / (ARRIVING_S + COOLDOWN_S)).abs() < 1e-12);
        assert!((cooldown_done(Drive::Cooldown(1e-9)).unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(cooldown_done(Drive::arrived()), Some(0.0));
        assert_eq!(cooldown_done(Drive::Ready), None);
        assert_eq!(cooldown_done(Drive::Charging(1.0)), None);
    }

    #[test]
    fn hardpoint_deployment() {
        let mut w = Weapons::default();
        assert_eq!(deploy_of(&w), Deploy::Retracted);
        w.deployed = true;
        w.deploy_t = 0.4;
        assert_eq!(deploy_of(&w), Deploy::Deploying);
        w.deploy_t = 1.0;
        assert_eq!(deploy_of(&w), Deploy::Deployed);
        w.deployed = false;
        w.deploy_t = 0.6;
        assert_eq!(deploy_of(&w), Deploy::Retracting);
    }

    #[test]
    fn the_clock() {
        assert!((time_of_day_s(2_451_545.0) - 43_200.0).abs() < 1e-3, "J2000 is noon");
        assert!(time_of_day_s(2_451_544.5).abs() < 1e-3, "midnight");
    }

    /// The lasers' name is the shipped string table's.
    #[test]
    fn the_laser_name_is_the_tables() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui/strings/en.json");
        let table: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(table["hpt_pulselaser_gimbal_small_name"].as_str(), Some(LASER_NAME));
    }

    /// Through the feeders: 1,234.5 m over the loaded ground of a body whose ground stands 15 km over its mean radius, the
    /// orbital HUD's altimeter label is the mod's altimeter number, and the heading tape and the roll are the planet HUD's
    /// (the roll's sign turned).
    #[test]
    fn the_orbital_hud_reads_the_mods_altimeter() {
        let mut pilot = Pilot::default();
        pilot.planet = Some(super::super::super::pilot::PlanetHud { alt_m: 1234.5, vspeed: 0.0, heading_deg: 90.0, pitch_deg: 5.0, roll_deg: 10.0 });
        let keys = PilotKeys::default();
        let ship = ShipFacts::of(&pilot, &keys, true, 0, &ShipFacts::default());
        let radius = 1_737_400.0;
        let pos = DVec3::Y * (radius + 15_000.0 + 1234.5);
        let f = facts_of(&Sources { pilot: &pilot, ship: &ship, keys: &keys, seated: true, place: Some((pos, DQuat::IDENTITY, radius)), jd: 2_451_545.0, burn_t_per_s: 0.0 });
        let mut out = elite_ui::feed::Outbox::default();
        elite_ui::feed::Hud::default().feed(&f, 1.0 / 60.0, &mut out);
        let find = |name: &str| out.msgs.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no {name}")).args.clone();
        assert_eq!(find("UpdateAltitude")[1], elite_ui::Value::from("$DISTANCE:#value=1234.5;"));
        assert_eq!(find("UpdateHeadingGauges"), vec![(-90.0).into(), 5.0.into(), (-10.0).into()]);
    }

    /// A new ship seated at rest: the facts are the pilot's (its fuel, pips, shields, hull, the lasers in, the gear down, no
    /// destination, no planet without a place).
    #[test]
    fn the_facts_of_a_new_ship() {
        let pilot = Pilot::default();
        let keys = PilotKeys::default();
        let ship = ShipFacts::of(&pilot, &keys, true, 0, &ShipFacts::default());
        let f = facts_of(&Sources { pilot: &pilot, ship: &ship, keys: &keys, seated: true, place: None, jd: 2_451_545.0, burn_t_per_s: 0.0 });
        assert!(f.status.seated && f.status.powered && f.status.gear_down && !f.status.mass_locked);
        assert_eq!(f.fuel.main_t, pilot.fuel.main);
        assert_eq!(f.fuel.jump_cost_t, None);
        assert_eq!(f.power.pips, Channels { wep: 0.5, eng: 0.5, sys: 0.5 });
        assert_eq!((f.damage.shield.strength, f.damage.shield.state, f.damage.hull), (1.0, ShieldState::Active, 1.0));
        let hp = f.weapons.hardpoints.as_ref().unwrap();
        assert_eq!(hp.len(), 2);
        assert!(hp.iter().all(|h| h.deploy == Deploy::Retracted && h.module_on && h.powered && h.trigger == 1));
        assert!(f.destination.is_none() && f.planet.is_none() && f.signature.channels.is_none());
        assert_eq!(f.clock.time_of_day_s, Some(43_200.0));
    }
}
