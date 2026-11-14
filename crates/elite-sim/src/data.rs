//! The stock Sidewinder's numbers. Each one says where it comes from and its grade (`docs/spec-mods-sidewinder.md`,
//! "What byte for byte honestly means here"):
//!
//! - **B (coriolis)**: EDCD's `coriolis-data` (raw files saved in `re/elite/community/coriolis-data/` on 2026-11-13): the
//!   numbers Elite's outfitting screen shows.
//! - **B (journal)**: the user's own Player Journal, `Loadout` of 2026-11-14 (game 4.4.2.1), for the ship they bought stock.
//!
//! Nothing here is tuned. A number that comes from the exe (C) or a measurement (D) lives with the rule that uses it.

/// Masses in tonnes, power in MW, speeds in m/s, rates in degrees a second, times in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Ship {
    /// B (coriolis `sidewinder.json` `hullMass`).
    pub hull_mass: f64,
    /// B (coriolis `speed`, `boost`): the top speed and boost speed the outfitting screen shows at the thrusters' optimal mass.
    pub speed: f64,
    pub boost: f64,
    /// B (coriolis `minthrust`, percent): the community's name for the lowest share of thrust (meaning to be confirmed, C or D).
    pub min_thrust_pct: f64,
    /// B (coriolis `boostEnergy`, `boostInt`): the ENG capacitor's cost of a boost and the boost's interval in seconds.
    pub boost_energy: f64,
    pub boost_interval: f64,
    /// B (coriolis `pitch`, `roll`, `yaw`), degrees a second at the optimal mass.
    pub pitch: f64,
    pub roll: f64,
    pub yaw: f64,
    /// B (coriolis `pipSpeed`): the community's speed-per-pip factor (3/22). Its use is the community's fit (E7: medium).
    pub pip_speed: f64,
    /// B (coriolis `heatCapacity`, `baseShieldStrength`, `baseArmour`, `hardness`, `masslock`).
    pub heat_capacity: f64,
    pub base_shield: f64,
    pub base_armour: f64,
    pub hardness: f64,
    pub mass_lock: f64,
    /// B (coriolis `reserveFuelCapacity`; the journal's `FuelCapacity.Reserve` 0.3).
    pub reserve_fuel: f64,
}

/// B (coriolis `sidewinder.json`).
pub const SIDEWINDER: Ship = Ship {
    hull_mass: 25.0,
    speed: 220.0,
    boost: 320.0,
    min_thrust_pct: 45.454,
    boost_energy: 7.0,
    boost_interval: 4.0,
    pitch: 42.0,
    roll: 110.0,
    yaw: 16.0,
    pip_speed: 0.136_363_636_363_64,
    heat_capacity: 140.0,
    base_shield: 40.0,
    base_armour: 60.0,
    hardness: 20.0,
    mass_lock: 6.0,
    reserve_fuel: 0.3,
};

/// Lightweight Alloy bulkheads. B (coriolis `bulkheads[0]`).
pub const BULKHEADS_MASS: f64 = 0.0;
pub const BULKHEADS_HULL_BOOST: f64 = 0.8;

/// Thrusters 2E. B (coriolis `thrusters.json`, `Int_Engine_Size2_Class1`).
#[derive(Clone, Debug, PartialEq)]
pub struct Thrusters {
    pub mass: f64,
    pub power: f64,
    pub min_mass: f64,
    pub opt_mass: f64,
    pub max_mass: f64,
    pub min_mul: f64,
    pub opt_mul: f64,
    pub max_mul: f64,
}
pub const THRUSTERS_2E: Thrusters = Thrusters { mass: 2.5, power: 2.0, min_mass: 24.0, opt_mass: 48.0, max_mass: 72.0, min_mul: 0.83, opt_mul: 1.0, max_mul: 1.03 };

/// Frame shift drive 2E. B (coriolis `Int_Hyperdrive_Size2_Class1`).
#[derive(Clone, Debug, PartialEq)]
pub struct Fsd {
    pub mass: f64,
    pub power: f64,
    pub opt_mass: f64,
    /// The most fuel one jump may take, tonnes.
    pub max_fuel: f64,
    /// The jump fuel formula's linear constant and power (`fsd.rs`).
    pub fuel_mul: f64,
    pub fuel_power: f64,
}
pub const FSD_2E: Fsd = Fsd { mass: 2.5, power: 0.16, opt_mass: 48.0, max_fuel: 0.6, fuel_mul: 0.011, fuel_power: 2.0 };

/// Power distributor 1E. B (coriolis `Int_PowerDistributor_Size1_Class1`): capacities in MJ, recharge in MW (the outfitting
/// screen's figure: A20 reads it as the rate with every pip in the channel, `power.rs`'s `hi`; inferred).
#[derive(Clone, Debug, PartialEq)]
pub struct Distributor {
    pub mass: f64,
    pub power: f64,
    pub sys_cap: f64,
    pub sys_rate: f64,
    pub eng_cap: f64,
    pub eng_rate: f64,
    pub wep_cap: f64,
    pub wep_rate: f64,
}
pub const DISTRIBUTOR_1E: Distributor =
    Distributor { mass: 1.3, power: 0.32, sys_cap: 8.0, sys_rate: 0.4, eng_cap: 8.0, eng_rate: 0.4, wep_cap: 10.0, wep_rate: 1.2 };

/// Shield generator 2E. B (coriolis `Int_ShieldGenerator_Size2_Class1`).
#[derive(Clone, Debug, PartialEq)]
pub struct ShieldGenerator {
    pub mass: f64,
    pub power: f64,
    pub regen: f64,
    pub broken_regen: f64,
    pub dist_draw: f64,
    pub min_mass: f64,
    pub opt_mass: f64,
    pub max_mass: f64,
    pub min_mul: f64,
    pub opt_mul: f64,
    pub max_mul: f64,
    pub kinetic_res: f64,
    pub thermal_res: f64,
    pub explosive_res: f64,
}
pub const SHIELD_2E: ShieldGenerator = ShieldGenerator {
    mass: 2.5,
    power: 0.9,
    regen: 1.0,
    broken_regen: 1.6,
    dist_draw: 0.6,
    min_mass: 28.0,
    opt_mass: 55.0,
    max_mass: 138.0,
    min_mul: 0.3,
    opt_mul: 0.8,
    max_mul: 1.3,
    kinetic_res: 0.4,
    thermal_res: -0.2,
    explosive_res: 0.5,
};

/// Small gimballed pulse laser (the stock pair: the journal's `hpt_pulselaser_gimbal_small` twice). B (coriolis id 18).
#[derive(Clone, Debug, PartialEq)]
pub struct Laser {
    pub mass: f64,
    pub power: f64,
    pub damage: f64,
    pub fire_interval: f64,
    pub dist_draw: f64,
    pub thermal_load: f64,
    pub range_m: f64,
    pub falloff_m: f64,
}
pub const PULSE_LASER_GIMBAL_SMALL: Laser =
    Laser { mass: 2.0, power: 0.39, damage: 1.56, fire_interval: 0.25, dist_draw: 0.31, thermal_load: 0.31, range_m: 3000.0, falloff_m: 500.0 };

/// The other modules: (mass, power). B (coriolis): power plant 2E (6.4 MW, efficiency 1), life support 1E (300 s), sensors 1E,
/// fuel tank 1C (2 t, mass 0), cargo rack 2E (4 t, mass 0). The supercruise assist and advanced docking computer the
/// journal lists weigh nothing (the masses below add up to the journal's unladen mass).
pub const POWER_PLANT_2E_MASS: f64 = 2.5;
pub const POWER_PLANT_2E_MW: f64 = 6.4;
pub const LIFE_SUPPORT_1E_MASS: f64 = 1.3;
pub const LIFE_SUPPORT_1E_POWER: f64 = 0.32;
pub const LIFE_SUPPORT_1E_SECONDS: f64 = 300.0;
pub const SENSORS_1E_MASS: f64 = 1.3;
pub const SENSORS_1E_POWER: f64 = 0.16;
/// B (journal `FuelCapacity.Main`).
pub const FUEL_TANK_T: f64 = 2.0;
/// B (journal `CargoCapacity`).
pub const CARGO_T: f64 = 4.0;

/// B (journal `UnladenMass`, `MaxJumpRange` 8.149506 ly), the numbers the other masses are checked against. The journal
/// prints the mass as 42.900002: that is 42.9 held in a 32-bit float (42.900001525...), and only 42.9 gives the journal's
/// range to its sixth decimal (42.900002 gives 8.149505).
pub const JOURNAL_UNLADEN_MASS: f64 = 42.9;
pub const JOURNAL_MAX_JUMP_LY: f64 = 8.149_506;

/// The stock ship's unladen mass from its parts (hull, bulkheads, the modules, two lasers).
pub fn unladen_mass() -> f64 {
    SIDEWINDER.hull_mass
        + BULKHEADS_MASS
        + POWER_PLANT_2E_MASS
        + THRUSTERS_2E.mass
        + FSD_2E.mass
        + LIFE_SUPPORT_1E_MASS
        + DISTRIBUTOR_1E.mass
        + SENSORS_1E_MASS
        + SHIELD_2E.mass
        + 2.0 * PULSE_LASER_GIMBAL_SMALL.mass
}

/// EDSY's numbers for the Sidewinder (`re/elite/community/edsy/eddb.js`, saved 2026-11-14; the spec's E8: **hints**, used only
/// where nothing better exists yet, each to be replaced by step 0.9's measurement): accelerations m/s^2, angular
/// accelerations deg/s^2, and the pitch rate at the base block (EDSY's `minpitch`).
pub mod edsy {
    pub const FWD_ACC: f64 = 44.39;
    pub const REV_ACC: f64 = 29.96;
    pub const LAT_ACC: f64 = 29.96;
    pub const PITCH_ACC: f64 = 220.0;
    pub const YAW_ACC: f64 = 110.0;
    pub const ROLL_ACC: f64 = 240.0;
    pub const PITCH: f64 = 43.0;
    pub const MIN_PITCH: f64 = 34.0;
}

/// Measured in the user's Elite (grade D: step 0.9's traces in `tests/traces`, each named).
pub mod measured {
    /// Block 0x04, 0x08: the reverse and side speeds' shares of the cap. Session 2 (2026-11-14, `s02_reverse`, `s07_lat_vert`,
    /// 2 ENG pips): reverse 97 m/s and lateral and vertical 129 against the forward cap of 160.9 (the HUD rounds; 0.6 and 0.8
    /// give 96.5 and 128.7).
    pub const BACK_SHARE: f64 = 0.6;
    pub const SIDE_SHARE: f64 = 0.8;
    /// The keyboard's throttle rate, a share a second: the HUD's throttle marker reached full 1.9 s after W went down in
    /// session 2 (1.75 s in session 1's run, read off a recording that showed the whole gauge).
    pub const THROTTLE_RATE: f64 = 0.526;
    /// The mouse's counts for full stick at sensitivity 1 (the exe's R: v += counts x sensitivity / R, step 0.11). Session 2's
    /// r02 and r03 (pitch and roll in steps of 100 counts) start turning at 59 counts on both axes; with the user's dead zone
    /// of 0.05 and sensitivity of 0.2725 that is full stick at 1,180 counts, so R = 1,180 x 0.2725 (about +-5 %: the dead
    /// zone's end read to a few counts).
    pub const MOUSE_FULL_COUNTS: f64 = 321.6;
    /// The raw turn rates (deg/s at the thrusters' optimal mass: pitch, yaw, roll), the rates the stick scales (the exe's
    /// rule: target = clamp(stick x W_raw, +-F x W_raw), `rotation-clamp.md`). Session 2 at rest, 2 ENG pips, mass 44.6 t
    /// (the mass curve's 1.0055 taken out): the mouse's pitch rises 3.52 deg/s and roll 9.88 deg/s per 100 counts (r02, r03),
    /// which is 39.5 and 110.7 deg/s per unit of stick; yaw's raw rate comes from its cap at rest (7.70 deg/s on A, r01)
    /// over the turn curve's factor there (`TURN_CURVE_A`, shared by pitch and yaw in the exe). How the pips change them is
    /// not measured yet: the same at every pip setting until session 3's four-pip run (a stand-in for that part).
    pub const TURN_RAW: [f64; 3] = [39.28, 17.18, 110.1];
    /// The turn curve's factor at a standstill (`a` of block 0x78.., pitch and yaw): pitch caps at 17.6 deg/s at rest against
    /// its stick scale of 39.5 (session 2, r02).
    pub const TURN_CURVE_A: f64 = 0.4456;
    /// Fitted to every flight-assist-on trace of sessions 1 and 2 without a boost (`tests/traces.rs` `fit`, `FIT_SKIP=boost`,
    /// 2026-11-14, the throttle rate held at the marker's; re-fitted after checker 2 found the fit scaling the accelerations
    /// twice): they replay those traces at 0.42 to 0.61 m/s rms. The forward acceleration of the full block (4 ENG pips) and of
    /// the base block (0 pips), m/s^2: the pips do change it (EDSY's 44.39 is near the full block's); reverse and lateral (EDSY
    /// 29.96); block 0x2C, flight assist's time constant's scale D (the approach to the cap: 0.87 s at 4 pips, 0.58 at 2, the
    /// 0.5 s floor at 0, read off the traces); block 0x28, the acceleration's jerk limit, m/s^3. (A HUD display lag was tried
    /// as well: the fit took it to nothing, so the HUD shows the speed as it is.)
    pub const FWD_ACC: f64 = 44.85;
    pub const FWD_ACC_BASE: f64 = 43.42;
    pub const REV_ACC: f64 = 30.26;
    pub const LAT_ACC: f64 = 30.23;
    pub const FA_D: f64 = 0.990;
    pub const JERK_LIN: f64 = 24.90;
}

/// Values the rules need that have no source yet: **stand-ins** (as the stand-in hull is), each the least-assuming choice the
/// code allows, listed here so a measurement (step 0.9) replaces every one. None is tuned by feel.
pub mod stand_in {
    /// Block 0x5C..0x64: 0, so the code's own floor holds (tau_r = dt).
    pub const FA_R: [f64; 3] = [0.0; 3];
    /// Block 0x50..0x58: no angular jerk limits.
    pub const ANG_JERK: [f64; 3] = [0.0; 3];
    /// Block 0x10: the boost's ramp to full demand, s.
    pub const BOOST_RAMP: f64 = 0.0;
    /// Block 0xC0..0xC8: no damping of a centred axis with flight assist off.
    pub const FA_OFF_DAMP: [f64; 3] = [0.0; 3];
    /// The blue-zone curves: flat (no speed effect) until measured.
    pub const CURVE: crate::flight::Curve = crate::flight::Curve::FLAT;
}

/// The stock Sidewinder's performance block at `eng_pips` (0..4) and mass `mass_t`: the current block (after the pips'
/// blend and the thrusters' mass curve, both grade C rules) and the full-power block's top speed.
///
/// - Full-power block (t = 1, what the outfitting screen shows): speeds B (coriolis); the raw turn rates, the turn curve's
///   standstill factor, the accelerations, flight assist's D and the jerk limit measured (D, `measured`); the angular
///   accelerations EDSY hints; the rest stand-ins.
/// - Base block (t = 0): the speeds at coriolis' `minthrust` share (the community's reading of that number: a hint, which
///   session 2's top speeds at 0, 2 and 4 pips bear out), the forward acceleration measured (D), everything else as the full
///   block (stand-in: no pip effect; for the turn rates until a four-pip rotation run measures it).
/// - t = ENG pips / most pips (C, step 0.11: 0x1437FEF60); the most pips a channel takes, 4, is Elite's well-known rule but
///   comes from data the exe loads (D until measured).
/// - The mass curve: thrusters 2E (B, coriolis), the same curve for speeds, accelerations and turn rates until their
///   separate curves are known (the exe applies one curve each; the module data has only these numbers).
pub fn sidewinder_block(eng_pips: f64, mass_t: f64) -> (crate::flight::Block, f64) {
    use crate::flight::{mass_curve, Block};
    let s = &SIDEWINDER;
    let full = Block {
        top_speed: s.speed,
        boost_speed: s.boost,
        back_share: measured::BACK_SHARE,
        side_share: measured::SIDE_SHARE,
        boost_ramp: stand_in::BOOST_RAMP,
        accel_fwd: measured::FWD_ACC,
        accel_rev: measured::REV_ACC,
        accel_side: measured::LAT_ACC,
        jerk_lin: measured::JERK_LIN,
        fa_d: measured::FA_D,
        turn: measured::TURN_RAW,
        ang_accel: [edsy::PITCH_ACC, edsy::YAW_ACC, edsy::ROLL_ACC],
        ang_jerk: stand_in::ANG_JERK,
        fa_r: stand_in::FA_R,
        curve_accel: stand_in::CURVE,
        // (The turn curve's a measured; its b and p stand-ins until session 3's turn rates at five throttles. The roll curve
        // is flat: at rest roll did not cap up to 0.66 of full stick, so its a is at least 0.66.)
        curve_turn: crate::flight::Curve { a: measured::TURN_CURVE_A, ..stand_in::CURVE },
        curve_roll: stand_in::CURVE,
        fa_off_damp: stand_in::FA_OFF_DAMP,
    };
    let share = s.min_thrust_pct / 100.0;
    let mut base = full.clone();
    base.accel_fwd = measured::FWD_ACC_BASE;
    base.top_speed *= share;
    base.boost_speed *= share;
    let t = (eng_pips / 4.0).clamp(0.0, 1.0);
    let th = &THRUSTERS_2E;
    let k = mass_curve(mass_t, [th.min_mass, th.opt_mass, th.max_mass], [th.min_mul, th.opt_mul, th.max_mul]);
    let current = Block::blend(&base, &full, t).scaled(k, k, k);
    (current, full.top_speed * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts add up to the journal's unladen mass (B against B).
    #[test]
    fn pips_and_mass_scale_the_block() {
        let m = JOURNAL_UNLADEN_MASS + FUEL_TANK_T + SIDEWINDER.reserve_fuel;
        let (four, full_top) = sidewinder_block(4.0, m);
        let (zero, _) = sidewinder_block(0.0, m);
        assert!((four.top_speed - full_top).abs() < 1e-9);
        assert!((zero.top_speed / four.top_speed - SIDEWINDER.min_thrust_pct / 100.0).abs() < 1e-9);
        // At the thrusters' optimal mass the speeds are the outfitting screen's.
        let (opt, _) = sidewinder_block(4.0, THRUSTERS_2E.opt_mass);
        assert!((opt.top_speed - SIDEWINDER.speed).abs() < 1e-6 && (opt.boost_speed - SIDEWINDER.boost).abs() < 1e-6);
    }

    #[test]
    fn parts_weigh_what_the_journal_says() {
        assert!((unladen_mass() - JOURNAL_UNLADEN_MASS).abs() < 1e-9, "{}", unladen_mass());
    }
}
