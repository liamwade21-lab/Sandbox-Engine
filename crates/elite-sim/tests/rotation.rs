//! The turn rates at rest against session 2's measurements (the gate's 1 deg/s, C10): the mouse's pitch and roll in steps of
//! 100 counts and yaw on A, at 2 ENG pips and throttle zero (`re/elite/measure/raw/session2/rotation.md`, measured by star
//! tracking; numbers only here). The stick comes from the counts by the exe's mouse rule (step 0.11: v = counts x
//! sensitivity / R, the dead zone and linearity of the user's file: 0.2725, 0.05, 1).

use elite_sim::data::{self, sidewinder_block};
use elite_sim::flight::{step, Input, Motion};
use glam::DVec3;

/// Session 2 (2026-11-14): (mouse counts, pitch deg/s, roll deg/s), the plateaus' means.
const STEPS: [(f64, f64, f64); 9] = [
    (0.0, 0.0, 0.0),
    (100.0, 1.44, 4.06),
    (200.0, 4.95, 13.98),
    (300.0, 8.52, 23.96),
    (400.0, 12.04, 33.92),
    (500.0, 15.47, 43.86),
    (600.0, 17.65, 53.71),
    (700.0, 17.54, 63.75),
    (800.0, 17.65, 73.08),
];
/// Yaw on A at rest (r01).
const YAW_A: f64 = 7.70;
/// The session's mass: unladen and 1.655 t of fuel.
const MASS_T: f64 = data::JOURNAL_UNLADEN_MASS + 1.655;
const SENSITIVITY: f64 = 0.2725;
const DEAD_ZONE: f64 = 0.05;

fn stick(counts: f64) -> f64 {
    let v = (counts * SENSITIVITY / data::measured::MOUSE_FULL_COUNTS).clamp(-1.0, 1.0);
    if v.abs() <= DEAD_ZONE { 0.0 } else { ((v.abs() - DEAD_ZONE) / (1.0 - DEAD_ZONE)).copysign(v) }
}

/// The steady turn rate (deg/s) for a held input, after 4 s at 60 Hz from rest.
fn steady(input: Input) -> DVec3 {
    let (block, full_top) = sidewinder_block(2.0, MASS_T);
    let mut m = Motion::default();
    for _ in 0..240 {
        step(&block, full_top, false, &input, DVec3::ZERO, &mut m, 1.0 / 60.0);
    }
    DVec3::new(m.omega.x.to_degrees(), m.omega.y.to_degrees(), m.omega.z.to_degrees())
}

#[test]
fn turn_rates_at_rest_are_the_measured() {
    let mut bad = Vec::new();
    for (counts, pitch, roll) in STEPS {
        let s = stick(counts);
        let p = steady(Input { pitch: s, flight_assist: true, ..Input::default() }).x.abs();
        let r = steady(Input { roll: s, flight_assist: true, ..Input::default() }).z.abs();
        if (p - pitch).abs() > 1.0 {
            bad.push(format!("pitch at {counts} counts: measured {pitch}, simulated {p:.2}"));
        }
        if (r - roll).abs() > 1.0 {
            bad.push(format!("roll at {counts} counts: measured {roll}, simulated {r:.2}"));
        }
    }
    let y = steady(Input { yaw: 1.0, flight_assist: true, ..Input::default() }).y.abs();
    if (y - YAW_A).abs() > 1.0 {
        bad.push(format!("yaw on A: measured {YAW_A}, simulated {y:.2}"));
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
