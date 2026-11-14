//! Flying the Sidewinder in normal space (step 1.3): `elite_sim::flight` stepped at a fixed tick from Elite's keys, the ship
//! moving in its body's turning frame (as Elite's ships near a planet hover over a spot), the body's gravity as the outside
//! pull the flight assist holds against, the ground under it from the terrain's loaded heights, a plain touchdown with the
//! gear down.
//!
//! - The tick is 60 Hz (the spec's Q8: Elite's own rate was not found in its exe), at most 6 a frame, the rest dropped (the
//!   spec's edge cases), on real seconds (the spec's F4: the game clock's keys are off while seated).
//! - Not yet: building contact and damage (step 2.3), Elite's touchdown and liftoff rules and slopes (2.4), boost and the
//!   pips (2.1), heat (2.2). The ship's own numbers are partly stand-ins (`elite_sim::data`), until measured.

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{DQuat, DVec3, Vec2};
use bevy::prelude::*;
use elite_sim::flight::{self, Input, Motion};

use crate::data::SeData;
use crate::mods::ModControl;
use crate::sandbox::{Mode, Sandbox};
use crate::system::SystemBody;
use crate::terrain::Terrain;

use super::seat::{action_pressed, Seat};
use super::{Sidewinder, SidewinderAssets};

/// The simulation's tick (the spec's Q8 fallback) and the most ticks a frame.
pub const TICK: f64 = 1.0 / 60.0;
const MOST_TICKS: u32 = 6;
/// Touching the ground with the gear down slower than this lands it; faster it is stopped there (stand-in until step 2.4
/// measures Elite's touchdown rule).
const TOUCHDOWN_MPS: f64 = 10.0;

/// What the seat's keys asked for this frame (read by `seat::take_keys` before it takes the presses from everyone else).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PilotKeys {
    pub gear: bool,
    pub flight_assist: bool,
    pub throttle_zero: bool,
    pub throttle_up: bool,
    pub throttle_down: bool,
    pub yaw: f64,
    pub lateral: f64,
    pub vertical: f64,
    /// Mouse counts this frame (x right, y down).
    pub mouse: Vec2,
    /// The pad's sticks bound to roll and pitch (`ControlPad.binds`: the left stick), after its dead zone: they replace the
    /// mouse's virtual stick while deflected (step 7.1).
    pub pad_roll: f64,
    pub pad_pitch: f64,
}

/// The flight's state while seated.
#[derive(Resource)]
pub struct Pilot {
    pub motion: Motion,
    pub throttle: f64,
    pub flight_assist: bool,
    pub eng_pips: f64,
    pub gear_down: bool,
    /// The gear's animation, 0 down .. 1 up.
    pub gear_t: f32,
    pub landed: bool,
    /// The mouse's virtual stick (x roll, y pitch), -1..1: it stays where the mouse left it (Elite's default: no decay).
    pub stick: Vec2,
    accum: f64,
    pub ticks: u64,
    /// A test's controls held for some seconds (`sw stick`, `sw thrust`).
    pub dev: Option<(Input, f32)>,
    pub log: bool,
}

impl Default for Pilot {
    fn default() -> Self {
        Pilot {
            motion: Motion::default(),
            throttle: 0.0,
            flight_assist: true,
            eng_pips: 2.0,
            gear_down: true,
            gear_t: 0.0,
            landed: true,
            stick: Vec2::ZERO,
            accum: 0.0,
            ticks: 0,
            dev: None,
            log: false,
        }
    }
}

/// The ship's mass for its block: unladen (the journal's), a full tank and the reservoir (no cargo).
pub fn ship_mass_t() -> f64 {
    use elite_sim::data::*;
    JOURNAL_UNLADEN_MASS + FUEL_TANK_T + SIDEWINDER.reserve_fuel
}

// elite-sim's glam and Bevy's may be different versions: through arrays.
fn to_sim_v(v: DVec3) -> glam::DVec3 {
    glam::DVec3::from_array(v.to_array())
}
fn from_sim_v(v: glam::DVec3) -> DVec3 {
    DVec3::from_array(v.to_array())
}
fn to_sim_q(q: DQuat) -> glam::DQuat {
    glam::DQuat::from_array(q.to_array())
}
fn from_sim_q(q: glam::DQuat) -> DQuat {
    DQuat::from_array(q.to_array())
}

/// Elite's name for a pad button (`ControlPad.binds`, an Xbox 360 pad) as Bevy's.
pub fn pad_button(name: &str) -> Option<GamepadButton> {
    use GamepadButton as B;
    Some(match name {
        "Pad_A" => B::South,
        "Pad_B" => B::East,
        "Pad_X" => B::West,
        "Pad_Y" => B::North,
        "Pad_LBumper" => B::LeftTrigger,
        "Pad_RBumper" => B::RightTrigger,
        "Pad_LTrigger" => B::LeftTrigger2,
        "Pad_RTrigger" => B::RightTrigger2,
        "Pad_Back" => B::Select,
        "Pad_Start" => B::Start,
        "Pad_LThumb" => B::LeftThumb,
        "Pad_RThumb" => B::RightThumb,
        "Pad_DPadUp" => B::DPadUp,
        "Pad_DPadDown" => B::DPadDown,
        "Pad_DPadLeft" => B::DPadLeft,
        "Pad_DPadRight" => B::DPadRight,
        _ => return None,
    })
}

/// Elite's name for a pad stick axis as Bevy's.
pub fn pad_axis(name: &str) -> Option<GamepadAxis> {
    Some(match name {
        "Pad_LStickX" => GamepadAxis::LeftStickX,
        "Pad_LStickY" => GamepadAxis::LeftStickY,
        "Pad_RStickX" => GamepadAxis::RightStickX,
        "Pad_RStickY" => GamepadAxis::RightStickY,
        _ => return None,
    })
}

fn is_pad(device: &str) -> bool {
    !matches!(device, "Keyboard" | "Mouse" | "{NoDevice}" | "")
}

/// An axis action's value from the pad: its file's binding, inverted flag and dead zone (the dead zone rescaled as the mouse's
/// is: the exe's axis rule was not looked for, a stand-in).
fn pad_value(b: &se_core::elite_asset::Binds, g: &Gamepad, action: &str) -> f64 {
    let Some(a) = b.get(action) else { return 0.0 };
    let Some(k) = a.axis.as_ref().filter(|k| is_pad(&k.device)) else { return 0.0 };
    let Some(axis) = pad_axis(&k.key) else { return 0.0 };
    let v = g.get(axis).unwrap_or(0.0) as f64 * if a.inverted == Some(true) { -1.0 } else { 1.0 };
    let dz = a.deadzone.unwrap_or(0.0).clamp(0.0, 0.95);
    if v.abs() <= dz { 0.0 } else { ((v.abs() - dz) / (1.0 - dz)).copysign(v) }
}

/// Reads the seat's keys by Elite's own bindings: the keyboard's (`KeyboardMouseOnly.binds`) and, if a pad is there, the pad's
/// (`ControlPad.binds`) (`seat::take_keys` calls this before it takes the presses).
pub fn read_keys(b: &se_core::elite_asset::Binds, pad_binds: &se_core::elite_asset::Binds, pad: Option<&Gamepad>, keys: &ButtonInput<KeyCode>, motion: &AccumulatedMouseMotion, real: bool) -> PilotKeys {
    let pad_btn = |action: &str, just: bool| -> bool {
        let (Some(g), Some(a)) = (pad, pad_binds.get(action)) else { return false };
        a.keys().filter(|k| is_pad(&k.device)).filter_map(|k| pad_button(&k.key)).any(|bt| if just { g.just_pressed(bt) } else { g.pressed(bt) })
    };
    let held = |a: &str| action_pressed(b, keys, a, false) || pad_btn(a, false);
    let press = |a: &str| action_pressed(b, keys, a, true) || pad_btn(a, true);
    let axis = |plus: &str, minus: &str| held(plus) as i8 as f64 - held(minus) as i8 as f64;
    let pv = |action: &str| pad.map_or(0.0, |g| pad_value(pad_binds, g, action));
    PilotKeys {
        gear: press("LandingGearToggle"),
        flight_assist: press("ToggleFlightAssist"),
        throttle_zero: press("SetSpeedZero"),
        throttle_up: held("ForwardKey"),
        throttle_down: held("BackwardKey"),
        // (Yaw on the pad's axis: Elite's yaw axis is positive to the right, the simulation's to the left.)
        yaw: (axis("YawLeftButton", "YawRightButton") - pv("YawAxisRaw")).clamp(-1.0, 1.0),
        lateral: (axis("RightThrustButton", "LeftThrustButton") + pv("LateralThrustRaw")).clamp(-1.0, 1.0),
        vertical: (axis("UpThrustButton", "DownThrustButton") + pv("VerticalThrustRaw")).clamp(-1.0, 1.0),
        mouse: if real { motion.delta } else { Vec2::ZERO },
        // (Roll on the pad: stick right rolls right, a negative roll about the back axis.)
        pad_roll: -pv("RollAxisRaw"),
        pad_pitch: pv("PitchAxisRaw"),
    }
}

/// Seated in Pilot mode: the frame's controls, the ticks, the ground. The camera follows from the ship's pose (`seat_camera`,
/// which runs after this).
#[allow(clippy::too_many_arguments)]
pub fn fly(
    time: Res<Time<Real>>,
    data: Res<SeData>,
    sb: Res<Sandbox>,
    seat: Res<Seat>,
    control: Res<ModControl>,
    keys: Res<PilotKeys>,
    assets: Option<Res<SidewinderAssets>>,
    mut ship: ResMut<Sidewinder>,
    mut pilot: ResMut<Pilot>,
    bodies: Query<(&SystemBody, Option<&Terrain>)>,
    mut msg: ResMut<crate::messages::Messages>,
) {
    let (Some(assets), true) = (assets, seat.seated && control.pilot && sb.mode == Mode::Pilot) else { return };
    let Some(p) = ship.parked.as_mut() else { return };
    let Some((b, terrain)) = bodies.iter().find(|(b, _)| b.obj == p.obj) else { return };
    let pilot = &mut *pilot;
    let dt = (time.delta_secs() as f64).min(0.25);
    // The controls of this frame.
    if keys.gear {
        pilot.gear_down = !pilot.gear_down;
        msg.show(if pilot.gear_down { "Landing gear deployed" } else { "Landing gear retracted" }.to_string());
    }
    if keys.flight_assist {
        pilot.flight_assist = !pilot.flight_assist;
        msg.show(if pilot.flight_assist { "Flight assist on" } else { "Flight assist off" }.to_string());
    }
    if keys.throttle_zero {
        pilot.throttle = 0.0;
    }
    let rate = elite_sim::data::stand_in::THROTTLE_RATE * dt;
    pilot.throttle = (pilot.throttle + rate * (keys.throttle_up as i8 - keys.throttle_down as i8) as f64).clamp(-1.0, 1.0);
    // The mouse's virtual stick: counts move it, it stays (no decay in Elite's default file); its dead zone is the file's.
    let setting = |name: &str, or: f32| assets.keys.value(name).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(or);
    let full = elite_sim::data::stand_in::MOUSE_FULL_COUNTS as f32 / setting("MouseSensitivity", 1.0).max(0.01);
    pilot.stick = (pilot.stick + Vec2::new(keys.mouse.x, -keys.mouse.y) / full).clamp(Vec2::splat(-1.0), Vec2::splat(1.0));
    // The exe's rule (0x1408F2210, step 0.11): stick = sign(v) x ((|v| - dead zone) / (1 - dead zone)) ^ linearity.
    let (dead, lin) = (setting("MouseDeadzone", 0.0), setting("MouseLinearity", 1.0));
    let dz = |v: f32| if v.abs() <= dead { 0.0 } else { ((v.abs() - dead) / (1.0 - dead)).powf(lin).copysign(v) as f64 };
    let mut input = Input {
        // (Mouse right rolls right: the right wing down, a negative roll about the back axis.) The pad's stick, while
        // deflected, instead.
        roll: if keys.pad_roll != 0.0 { keys.pad_roll } else { -dz(pilot.stick.x) },
        pitch: if keys.pad_pitch != 0.0 { keys.pad_pitch } else { dz(pilot.stick.y) },
        yaw: keys.yaw,
        lateral: keys.lateral,
        vertical: keys.vertical,
        forward: pilot.throttle,
        flight_assist: pilot.flight_assist,
    };
    if let Some((dev, left)) = pilot.dev.as_mut() {
        input = Input { flight_assist: pilot.flight_assist, forward: if dev.forward != 0.0 { dev.forward } else { pilot.throttle }, ..*dev };
        *left -= dt as f32;
        if *left <= 0.0 {
            pilot.dev = None;
        }
    }
    // The gear's animation (3 s, the converter's `gear_up`).
    let gear_s = assets.ship.animations.iter().find(|a| a.name == "gear_up").map_or(3.0, |a| a.seconds);
    let towards = if pilot.gear_down { 0.0 } else { 1.0 };
    pilot.gear_t += (towards - pilot.gear_t).clamp(-(dt as f32) / gear_s, dt as f32 / gear_s);

    let gm = se_core::units::G * crate::ship::body_mass(&data, b.obj, b.radius_m);
    let (block, full_top) = elite_sim::data::sidewinder_block(pilot.eng_pips, ship_mass_t());
    // The lowest point over the model's origin: the gear's feet (0) or the hull's underside.
    let hull_bottom = assets.shape.hulls.first().map_or(0.0, |h| h.iter().map(|p| p.y).fold(f32::MAX, f32::min)) as f64;
    pilot.accum += dt;
    let mut n = 0;
    while pilot.accum >= TICK && n < MOST_TICKS {
        pilot.accum -= TICK;
        n += 1;
        pilot.ticks += 1;
        if pilot.landed {
            // Lift off: up thrust (Elite lifts a landed ship on its vertical thrusters).
            if input.vertical <= 0.0 {
                continue;
            }
            pilot.landed = false;
            pilot.motion = Motion { rot: to_sim_q(p.rot), ..Motion::default() };
            msg.show("Liftoff".to_string());
            info!("sidewinder: liftoff");
        }
        let r = p.pos.length().max(1.0);
        let up = p.pos / r;
        let outside = -up * gm / (r * r);
        pilot.motion.rot = to_sim_q(p.rot);
        flight::step(&block, full_top, false, &input, to_sim_v(outside), &mut pilot.motion, TICK);
        p.rot = from_sim_q(pilot.motion.rot);
        p.pos += from_sim_v(pilot.motion.vel) * TICK;
        // The ground under the ship (its loaded heights).
        let (ground, _) = crate::ship::ground_on(b.radius_m, b.shape, terrain, p.pos);
        let lowest = if pilot.gear_t < 0.5 { 0.0 } else { hull_bottom };
        let h = p.pos.length() - ground + lowest;
        if h < 0.0 {
            let up = p.pos.normalize();
            let vel = from_sim_v(pilot.motion.vel);
            let down_speed = -vel.dot(up);
            if pilot.gear_down && pilot.gear_t < 0.05 && vel.length() < TOUCHDOWN_MPS {
                // Touchdown: on the ground, level with it, the heading kept.
                p.pos = up * ground;
                let fwd = p.rot * DVec3::NEG_Z;
                let flat = (fwd - up * fwd.dot(up)).normalize_or(up.any_orthonormal_vector());
                let right = flat.cross(up);
                p.rot = DQuat::from_mat3(&bevy::math::DMat3::from_cols(right, up, -flat)).normalize();
                pilot.motion = Motion { rot: to_sim_q(p.rot), ..Motion::default() };
                pilot.landed = true;
                pilot.throttle = 0.0;
                msg.show("Touchdown".to_string());
                info!("sidewinder: touchdown at {:.1} m/s down", down_speed);
                break;
            }
            // Stopped by the ground (no damage until step 2.3).
            p.pos = up * (ground - lowest);
            if down_speed > 0.0 {
                pilot.motion.vel = to_sim_v(vel + up * down_speed);
            }
        }
    }
    if pilot.accum > TICK {
        pilot.accum = 0.0;
    }
    if pilot.log && n > 0 {
        let v = from_sim_v(pilot.motion.vel);
        let (ground, _) = crate::ship::ground_on(b.radius_m, b.shape, terrain, p.pos);
        info!("sidewinder: {:.1} m/s, {:.1} m over the ground, throttle {:.2}, FA {}, gear {}", v.length(), p.pos.length() - ground, pilot.throttle, pilot.flight_assist, pilot.gear_down);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped(name: &str) -> se_core::elite_asset::Binds {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/binds").join(name);
        se_core::elite_asset::Binds::parse(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    /// Step 7.1: the shipped pad file gives the spec's pad layout, and every pad name in it has a Bevy button or axis.
    #[test]
    fn pad_table_is_the_specs() {
        let b = shipped("ControlPad.binds");
        let axis = |a: &str| b.get(a).and_then(|x| x.axis.as_ref()).map(|k| k.key.clone()).unwrap_or_default();
        let button = |a: &str| b.get(a).map(|x| x.keys().filter(|k| is_pad(&k.device)).map(|k| k.key.clone()).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!((axis("RollAxisRaw"), axis("PitchAxisRaw")), ("Pad_LStickX".into(), "Pad_LStickY".into()));
        assert_eq!((axis("LateralThrustRaw"), axis("VerticalThrustRaw")), ("Pad_RStickX".into(), "Pad_RStickY".into()));
        for (action, key) in [
            ("ForwardKey", "Pad_RBumper"),
            ("BackwardKey", "Pad_LBumper"),
            ("UseBoostJuice", "Pad_B"),
            ("PrimaryFire", "Pad_RTrigger"),
            ("SecondaryFire", "Pad_LTrigger"),
            ("DeployHardpointToggle", "Pad_Back"),
            ("IncreaseEnginesPower", "Pad_DPadUp"),
            ("IncreaseWeaponsPower", "Pad_DPadRight"),
            ("IncreaseSystemsPower", "Pad_DPadLeft"),
            ("ResetPowerDistribution", "Pad_DPadDown"),
            ("UIFocus", "Pad_X"),
            ("SelectTarget", "Pad_Y"),
            ("HeadLookToggle", "Pad_RThumb"),
        ] {
            assert_eq!(button(action), vec![key.to_string()], "{action}");
        }
        for a in b.actions.values() {
            for k in a.keys().chain(a.axis.iter()).filter(|k| k.device == "XB360 Pad") {
                assert!(pad_button(&k.key).is_some() || pad_axis(&k.key).is_some() || k.key.starts_with("Pos_") || k.key.starts_with("Neg_") || k.key.starts_with("GamePad_"), "{}", k.key);
            }
        }
    }

    /// Every keyboard key Elite's default file binds has a Bevy key code (or the seat couldn't take it from the others).
    #[test]
    fn keyboard_keys_all_known() {
        let b = shipped("KeyboardMouseOnly.binds");
        let missing: Vec<String> = b.actions.values().flat_map(|a| a.keys()).filter(|k| k.device == "Keyboard").filter(|k| super::super::seat::key_code(&k.key).is_none()).map(|k| k.key.clone()).collect();
        assert!(missing.is_empty(), "{missing:?}");
    }
}
