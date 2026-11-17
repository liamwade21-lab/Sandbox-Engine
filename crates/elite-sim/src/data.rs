//! The stock Sidewinder's numbers. Each one says where it comes from and its grade (`docs/spec-mods-sidewinder.md`,
//! "What byte for byte honestly means here"):
//!
//! - **B (records)**: Elite's own item and prototype records, read by task N1 (2026-11-16, build 2026.10.23.334925;
//!   `re/elite/notes/sidewinder_numbers.md`, its data `re/elite/work/numbers/numbers.json`): the ship item `sidewinder` in the
//!   item database `201ee494...` (Win64/02), the stock modules' items in `f01b6304...` (Win64/0f), the ship's prototype in
//!   `86ec03fb...` (Win64/08). Each such number names its record and field.
//! - **B (coriolis)**: EDCD's `coriolis-data` (raw files saved in `re/elite/community/coriolis-data/` on 2026-11-13): the
//!   numbers Elite's outfitting screen shows. The records agree with every one used here but the base shield (55, not 40).
//! - **B (journal)**: the user's own Player Journal, `Loadout` of 2026-11-14 (game 4.4.2.1), for the ship they bought stock.
//!
//! Nothing here is tuned. A number that comes from the exe (C) or a measurement (D) lives with the rule that uses it.

use crate::flight::{mass_curve, Block};

/// Masses in tonnes, power in MW, speeds in m/s, rates in degrees a second, times in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Ship {
    /// B (records: the hull record, item data +0x18, +0x00; coriolis `hullMass`).
    pub hull_mass: f64,
    /// B (records: the full-power block's 0x00 and the base block's 0x0C; coriolis `speed`, `boost`): the top speed and boost
    /// speed the outfitting screen shows at the thrusters' optimal mass.
    pub speed: f64,
    pub boost: f64,
    /// The base block's top speed over the full-power block's, in percent (records: 100 / 220; coriolis `minthrust` 45.454 is
    /// this number rounded: what 0 ENG pips gives, the blend 0x141B2E310, C).
    pub min_thrust_pct: f64,
    /// B (records: the boost record's field 3 and fields 0 + 1 + 2; coriolis `boostEnergy`, `boostInt`): the ENG capacitor's
    /// cost of a boost and the boost's whole cycle in seconds.
    pub boost_energy: f64,
    pub boost_interval: f64,
    /// B (records: the full-power block's 0x38, the base block's 0x3C and 0x40; EDSY agrees, coriolis' 42 and 16 do not),
    /// degrees a second at the optimal mass with 4 ENG pips.
    pub pitch: f64,
    pub roll: f64,
    pub yaw: f64,
    /// B: the heat record's capacity (item data +0x20, +0xF0), the hull record's base shield (+0x0C: 55; coriolis says 40,
    /// EDSY 55) and base armour (+0x08), the armour's hardness (20), the mass lock factor (the prototype's MassLockComponent
    /// +0x18).
    pub heat_capacity: f64,
    pub base_shield: f64,
    pub base_armour: f64,
    pub hardness: f64,
    pub mass_lock: f64,
    /// B (records: the prototype's FuelManager +0x10; the journal's `FuelCapacity.Reserve` 0.3).
    pub reserve_fuel: f64,
}

/// B (records; coriolis `sidewinder.json`).
pub const SIDEWINDER: Ship = Ship {
    hull_mass: 25.0,
    speed: 220.0,
    boost: 320.0,
    min_thrust_pct: 100.0 * 100.0 / 220.0,
    boost_energy: 7.0,
    boost_interval: 4.0,
    pitch: 43.0,
    roll: 110.0,
    yaw: 17.0,
    heat_capacity: 140.0,
    base_shield: 55.0,
    base_armour: 60.0,
    hardness: 20.0,
    mass_lock: 6.0,
    reserve_fuel: 0.3,
};

/// Lightweight Alloy bulkheads. B (coriolis `bulkheads[0]`; the armour item's +0x1D0 record: hull x 1.8).
pub const BULKHEADS_MASS: f64 = 0.0;
pub const BULKHEADS_HULL_BOOST: f64 = 0.8;

/// Thrusters 2E. B (coriolis `thrusters.json`, `Int_Engine_Size2_Class1`; the module record's three mass curves, for the
/// accelerations +0x0C, the speeds +0x24 and the turn rates +0x3C, hold these same six numbers each: 0x142EBDF60).
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

/// Frame shift drive 2E. B (coriolis `Int_Hyperdrive_Size2_Class1`; the module record D: D14 48, D18 0.011, D1C 2, D24 0.6, and
/// with the parts' masses they give the journal's 8.149506 ly to six decimals).
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

/// Power distributor 1E. B (coriolis `Int_PowerDistributor_Size1_Class1`; the module record per channel, WEP +0x00, ENG +0x18,
/// SYS +0x30: +0 capacity, +8 `hi`, the rate with every pip in the channel; `power.rs` has `lo` and `p`).
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

/// Shield generator 2E. B (coriolis `Int_ShieldGenerator_Size2_Class1`; the module record: regen +0x30/+0x34, broken
/// +0x38/+0x3C, MJ per MJ +0x44/+0x48, the curve +0x5C..+0x70; the damage multipliers +0x00 0.6, +0x04 1.2, +0x0C 0.5 are these
/// resistances by value).
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

/// Small gimballed pulse laser (the stock pair: the journal's `hpt_pulselaser_gimbal_small` twice). B (coriolis id 18; the
/// weapon record, laser item +0x268, holds the same numbers, named by value only: no exe read of that record was found:
/// damage +0x04, range +0x58, falloff +0x60, interval +0x98, WEP +0x128, heat +0x12C).
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

/// The other modules: (mass, power). B (coriolis; the records' masses and the power plant's 6.4 MW and the life support's
/// 300 s agree): power plant 2E (6.4 MW, efficiency 1), life support 1E (300 s), sensors 1E, fuel tank 1C (2 t, mass 0), cargo
/// rack 2E (4 t, mass 0). The supercruise assist and advanced docking computer the journal lists weigh nothing (the masses
/// below add up to the journal's unladen mass).
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

/// The ship item's performance record (item data +0x30, getter 0x141B3AFF0): grade B, every field as the record stores it
/// (`re/elite/notes/sidewinder_numbers.md`, "Performance record" and "Block layout"). Its proof: the blend of these blocks at 2
/// ENG pips with the thrusters' curve gives session 2's measured cap of 160.86 m/s (HUD 160.9), its raw turn rates 39 / 17 /
/// 110 deg/s (measured 39.28 / 17.18 / 110.1) and its turn curve's standstill factor 0.45 (measured 0.4456).
pub mod record {
    use crate::flight::{Block, Curve};

    /// The base block (record +0x18): what 0 ENG pips gives. The boost fields come from the boost record (+0x24C: field 4,
    /// every upper acceleration limit x 4 while firing, 0x142E6E960; field 5, flight assist's D while boosting).
    pub const BASE: Block = Block {
        top_speed: 100.0,
        boost_speed: 320.0,
        back_share: 0.6,
        side_share: 0.8,
        boost_ramp: 0.0,
        accel_fwd: 45.0,
        accel_rev: 30.0,
        accel_side: 30.0,
        jerk_lin: 25.0,
        fa_d: 1.0,
        boost_side: [1.0, 1.0],
        turn: [35.0, 17.0, 110.0],
        ang_accel: [220.0, 110.0, 240.0],
        ang_jerk: [300.0, 100.0, 600.0],
        fa_r: [0.1, 0.1, 0.1],
        curve_accel: Curve { a: 0.3, b: 0.8, p: 0.5 },
        curve_turn: Curve { a: 0.5, b: 0.4, p: 0.5 },
        curve_roll: Curve { a: 0.8, b: 0.8, p: 0.5 },
        reversal: [1.0, 1.0, 1.0],
        fa_off_damp: [0.2, 0.2, 0.2],
        boost_accel: 4.0,
        boost_fa_d: 1.0,
    };

    /// The full-power block (the pointer at record +0xE8): what 4 ENG pips gives; -1 keeps the base's value. It holds only the
    /// top speed, pitch, pitch's angular jerk and R, and the turn and roll curves' a and b.
    pub const FULL: Block = Block {
        top_speed: 220.0,
        boost_speed: -1.0,
        back_share: -1.0,
        side_share: -1.0,
        boost_ramp: -1.0,
        accel_fwd: -1.0,
        accel_rev: -1.0,
        accel_side: -1.0,
        jerk_lin: -1.0,
        fa_d: -1.0,
        boost_side: [-1.0, -1.0],
        turn: [43.0, -1.0, -1.0],
        ang_accel: [-1.0, -1.0, -1.0],
        ang_jerk: [300.0, -1.0, -1.0],
        fa_r: [0.05, -1.0, -1.0],
        curve_accel: Curve { a: -1.0, b: -1.0, p: -1.0 },
        curve_turn: Curve { a: 0.4, b: 0.3, p: -1.0 },
        curve_roll: Curve { a: -1.0, b: 0.9, p: -1.0 },
        reversal: [1.0, 1.0, 1.0],
        fa_off_damp: [-1.0, -1.0, -1.0],
        boost_accel: -1.0,
        boost_fa_d: -1.0,
    };

    /// The supercruise block (record +0xF0, read by 0x142E58740 and the supercruise branch of 0x142E86E20): no pip blend and
    /// no thruster curves; its speeds and accelerations go through accel mode 2 (`Block::mode2`). The top speed is 6e11 held
    /// in a 32-bit float (2001.384 c).
    pub const CRUISE: Block = Block {
        top_speed: 599_999_971_328.0,
        boost_speed: 1.0,
        back_share: 1.0,
        side_share: 1.0,
        boost_ramp: 0.0,
        accel_fwd: 44_999_999_488.0,
        accel_rev: 89_999_998_976.0,
        accel_side: 99_999_997_952.0,
        jerk_lin: 10_000_000_000.0,
        fa_d: 1.0,
        boost_side: [1.0, 1.0],
        turn: [38.0, 12.0, 110.0],
        ang_accel: [220.0, 110.0, 240.0],
        ang_jerk: [300.0, 100.0, 0.0],
        fa_r: [0.1, 0.2, 0.1],
        curve_accel: Curve { a: 0.3, b: 0.8, p: 0.5 },
        curve_turn: Curve { a: 0.5, b: 0.6, p: 0.5 },
        curve_roll: Curve { a: 0.8, b: 0.8, p: 0.5 },
        reversal: [1.0, 1.0, 1.0],
        fa_off_damp: [0.0, 0.0, 0.0],
        boost_accel: 1.0,
        boost_fa_d: 1.0,
    };

    /// The boost record (+0x24C, 0x142E512F0) fields 0, 1, 2, 6: warm-up, firing and cool-down (s; they add up to coriolis's
    /// 4 s interval), and the heat a second while firing (A20: label "Engines").
    pub const BOOST_WARM_S: f64 = 1.0;
    pub const BOOST_FIRE_S: f64 = 2.0;
    pub const BOOST_COOL_S: f64 = 1.0;
    pub const BOOST_HEAT_PER_S: f64 = 10.0;

    /// +0x3EC and +0x3E8: the top and boost speeds' multiplier with the landing gear down (read by the gear module,
    /// 0x14307F8A0) and with the cargo scoop out (the cargo bay, 0x142A69A40): `deployables.rs`.
    pub const GEAR_SPEED: f64 = 0.5;
    pub const SCOOP_SPEED: f64 = 0.5;
}

/// Measured in the user's Elite (grade D: step 0.9's traces in `tests/traces`, each named).
pub mod measured {
    /// The keyboard's throttle rate, a share a second: the HUD's throttle marker reached full 1.9 s after W went down in
    /// session 2 (1.75 s in session 1's run, read off a recording that showed the whole gauge). Re-read in task N2 with the
    /// marker found in the blue zone too: full 1.78 s after W in run1, about 1.8 s in s03 and s06 (0.92 of the gauge at 1.63 s, rising 0.5 a second); it leaves idle 0.12-0.13 s after
    /// W in run1, later than the speed starts to climb (the marker trails the throttle), so its start, and with it a better rate,
    /// is not measured. Unchanged: every measured run replays the same with any rate from 0.4 a second up.
    pub const THROTTLE_RATE: f64 = 0.526;
    /// The mouse's counts for full stick at sensitivity 1 (the exe's R: v += counts x sensitivity / R, step 0.11). Session 2's
    /// r02 and r03 (pitch and roll in steps of 100 counts) start turning at 59 counts on both axes; with the user's dead zone
    /// of 0.05 and sensitivity of 0.2725 that is full stick at 1,180 counts, so R = 1,180 x 0.2725 (about +-5 %: the dead
    /// zone's end read to a few counts).
    pub const MOUSE_FULL_COUNTS: f64 = 321.6;
}

/// The stock Sidewinder's performance block at `eng_pips` (0..4) and mass `mass_t`: the current block (the pips' blend of the
/// records' base and full-power blocks, then the thrusters' mass curves: grade C rules, 0x141B2E310, 0x142EBDF60) and the
/// full-power block's top speed (the same curves at t = 1). t = ENG pips / most pips (0x1437FEF60; the most, 4 whole pips,
/// is the distributor prototype's byte +0x13, B).
pub fn sidewinder_block(eng_pips: f64, mass_t: f64) -> (Block, f64) {
    sidewinder_block_with(eng_pips, mass_t, 1.0)
}

/// `sidewinder_block` with the speed multiplier `speed_k` of 0x142E86E20's step 4 (the gear's and the scoop's,
/// `deployables::speed_factor`), which scales both blocks' top and boost speeds after the thrusters' curves (0x141B2DA50).
pub fn sidewinder_block_with(eng_pips: f64, mass_t: f64, speed_k: f64) -> (Block, f64) {
    let t = (eng_pips / 4.0).clamp(0.0, 1.0);
    let th = &THRUSTERS_2E;
    let k = mass_curve(mass_t, [th.min_mass, th.opt_mass, th.max_mass], [th.min_mul, th.opt_mul, th.max_mul]);
    let current = Block::blend(&record::BASE, &record::FULL, t).scaled(k, k, k).speeds_times(speed_k);
    let full_top = Block::blend(&record::BASE, &record::FULL, 1.0).top_speed * k * speed_k;
    (current, full_top)
}

/// The supercruise block as the record holds it (no pips, no thruster curves: 0x142E86E20's supercruise branch). Its turn
/// rates (38 / 12 / 110 deg/s), angular accelerations, jerks, R and curves are what supercruise turns by; its speed curve's
/// x is the speed over supercruise's cap (`pilot.rs` turns by it in accel mode 2, whose top speed is the cap; with the gear
/// or the scoop out, the cap times their factor: `supercruise::cruise_block`).
pub fn sidewinder_cruise_block() -> Block {
    record::CRUISE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::Curve;

    /// The pips blend the records' blocks and the mass curve scales them: at 4 pips the full block, at 0 the base, whose top
    /// speed is `min_thrust_pct` of the full's; at the optimal mass the outfitting screen's speeds.
    #[test]
    fn pips_and_mass_scale_the_block() {
        let m = JOURNAL_UNLADEN_MASS + FUEL_TANK_T + SIDEWINDER.reserve_fuel;
        let (four, full_top) = sidewinder_block(4.0, m);
        let (zero, _) = sidewinder_block(0.0, m);
        assert!((four.top_speed - full_top).abs() < 1e-9);
        assert!((zero.top_speed / four.top_speed - SIDEWINDER.min_thrust_pct / 100.0).abs() < 1e-9);
        let (opt, _) = sidewinder_block(4.0, THRUSTERS_2E.opt_mass);
        assert!((opt.top_speed - SIDEWINDER.speed).abs() < 1e-6 && (opt.boost_speed - SIDEWINDER.boost).abs() < 1e-6);
        // The boost speed and the accelerations have no full-power value: the same at every pip setting.
        assert_eq!((zero.boost_speed, zero.accel_fwd, zero.accel_rev), (four.boost_speed, four.accel_fwd, four.accel_rev));
        assert_eq!((opt.turn[0], opt.turn[1], opt.turn[2]), (SIDEWINDER.pitch, SIDEWINDER.yaw, SIDEWINDER.roll));
    }

    /// The proof of the records (N1): at 2 ENG pips and session 2's 44.6 t the cap is the HUD's 160.9 m/s and the turn
    /// curve's standstill factor 0.45 (measured 0.4456).
    #[test]
    fn the_blend_gives_session_twos_cap() {
        let (b, _) = sidewinder_block(2.0, 44.6);
        assert!((b.top_speed - 160.86).abs() < 0.01, "{}", b.top_speed);
        assert!((b.curve_turn.a - 0.45).abs() < 1e-12 && (b.curve_turn.b - 0.35).abs() < 1e-12 && (b.curve_roll.b - 0.85).abs() < 1e-12);
        assert!((b.fa_r[0] - 0.075).abs() < 1e-12 && b.fa_r[1] == 0.1);
    }

    /// The turn rates at 0, 2 and 4 ENG pips (at the optimal mass): pitch from the base's 35 to the full's 43, yaw and roll
    /// the same at every setting; the turn curve's a and b go with them.
    #[test]
    fn turn_rates_at_zero_two_and_four_pips() {
        for (pips, pitch, a, b) in [(0.0, 35.0, 0.5, 0.4), (2.0, 39.0, 0.45, 0.35), (4.0, 43.0, 0.4, 0.3)] {
            let (blk, _) = sidewinder_block(pips, THRUSTERS_2E.opt_mass);
            assert!((blk.turn[0] - pitch).abs() < 1e-9 && (blk.turn[1] - 17.0).abs() < 1e-9 && (blk.turn[2] - 110.0).abs() < 1e-9);
            assert!((blk.curve_turn.a - a).abs() < 1e-12 && (blk.curve_turn.b - b).abs() < 1e-12);
            assert_eq!(blk.ang_accel, [220.0, 110.0, 240.0]);
        }
        assert_eq!(sidewinder_cruise_block().turn, [38.0, 12.0, 110.0]);
        assert_eq!(sidewinder_block(4.0, THRUSTERS_2E.opt_mass).0.curve_turn, Curve { a: 0.4, b: 0.3, p: 0.5 });
    }

    #[test]
    fn parts_weigh_what_the_journal_says() {
        assert!((unladen_mass() - JOURNAL_UNLADEN_MASS).abs() < 1e-9, "{}", unladen_mass());
        assert!((record::BOOST_WARM_S + record::BOOST_FIRE_S + record::BOOST_COOL_S - SIDEWINDER.boost_interval).abs() < 1e-12);
    }
}
