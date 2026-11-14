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

/// Power distributor 1E. B (coriolis `Int_PowerDistributor_Size1_Class1`): capacities in MJ, recharge in MW at 2 pips.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts add up to the journal's unladen mass (B against B).
    #[test]
    fn parts_weigh_what_the_journal_says() {
        assert!((unladen_mass() - JOURNAL_UNLADEN_MASS).abs() < 1e-9, "{}", unladen_mass());
    }
}
