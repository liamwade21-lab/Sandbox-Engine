//! The gate (the spec's C10, the user's D10): every measured trace in `tests/traces/` (the user's Elite, step 0.9 on, made by
//! `re/elite/measure/tools/make_trace.py`, numbers only) replayed through the simulation inside the gate.
//!
//! Each trace gets one fitted number of its own, the latency (the game's reaction plus the capture's start offset: not a
//! property of the ship); everything else is the ship's (`data`), the same for every trace.
//!
//! `cargo test --release -p elite-sim --test traces fit -- --ignored --nocapture` fits the ship's unmeasured numbers (the
//! stand-ins and hints listed in `data.rs`) to all the traces at once and prints them: they go into `data.rs` as grade D, with
//! the traces named as their source.

use elite_sim::data::{self, sidewinder_block};
use elite_sim::flight::Block;
use elite_sim::replay::{self, Replay};
use elite_sim::trace::Trace;

fn traces() -> Vec<(String, Trace)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("traces");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "csv")).collect();
    paths.sort();
    for p in paths {
        let text = std::fs::read_to_string(&p).unwrap();
        let tr = Trace::parse(&text).unwrap_or_else(|e| panic!("{}: {e:?}", p.display()));
        if tr.column("speed").is_some() && tr.column("key_w").is_some() {
            out.push((p.file_stem().unwrap().to_string_lossy().into_owned(), tr));
        }
    }
    out
}

fn mass() -> f64 {
    data::JOURNAL_UNLADEN_MASS + data::FUEL_TANK_T + data::SIDEWINDER.reserve_fuel
}

/// The latency that fits a trace best (0 to 0.6 s, in 1 ms steps after a coarse pass), with the ship's block maker.
fn best_latency(tr: &Trace, block_at: &dyn Fn(f64) -> (Block, f64), throttle_rate: f64) -> (f64, f64) {
    let err = |lat: f64| {
        let sim = replay::simulate(tr, block_at, &Replay { latency: lat, throttle_rate }).unwrap();
        replay::error(tr, &sim)
    };
    let mut best = (0.0, f64::MAX);
    for i in 0..=60 {
        let l = i as f64 * 0.01;
        let e = err(l);
        if e < best.1 {
            best = (l, e);
        }
    }
    let c = best.0;
    for i in -10..=10 {
        let l = (c + i as f64 * 0.001).max(0.0);
        let e = err(l);
        if e < best.1 {
            best = (l, e);
        }
    }
    best
}

#[test]
fn measured_traces_pass_the_gate() {
    let all = traces();
    let m = mass();
    let block_at = |e: f64| sidewinder_block(e, m);
    let mut failures = Vec::new();
    for (name, tr) in &all {
        let (lat, _) = best_latency(tr, &block_at, data::stand_in::THROTTLE_RATE);
        let sim = replay::simulate(tr, &block_at, &Replay { latency: lat, throttle_rate: data::stand_in::THROTTLE_RATE }).unwrap();
        let bad = replay::gate(tr, &sim);
        if !bad.is_empty() {
            let worst = bad.iter().cloned().fold((0.0f64, 0.0f64, 0.0f64), |w, b| if (b.2 - b.1).abs() > (w.2 - w.1).abs() { b } else { w });
            failures.push(format!("{name}: {} of {} rows outside the gate (latency {lat:.3} s); worst at {:.2} s: measured {}, simulated {:.2}", bad.len(), tr.rows.len(), worst.0, worst.1, worst.2));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The ship's numbers a fit may set (each a stand-in or a hint in `data.rs`).
#[derive(Clone, Copy, Debug)]
struct Fit {
    throttle_rate: f64,
    accel_fwd: f64,
    accel_rev: f64,
    fa_d: f64,
    jerk: f64,
}

fn block_with(f: Fit, m: f64) -> impl Fn(f64) -> (Block, f64) {
    move |e: f64| {
        let (mut b, top) = sidewinder_block(e, m);
        // The block's accelerations are the full block's scaled by the mass curve: replace the full block's value.
        b.accel_fwd *= f.accel_fwd / data::edsy::FWD_ACC;
        b.accel_rev *= f.accel_rev / data::edsy::REV_ACC;
        b.fa_d = f.fa_d;
        b.jerk_lin = f.jerk;
        (b, top)
    }
}

#[test]
#[ignore]
fn fit() {
    let all = traces();
    assert!(!all.is_empty(), "no traces in tests/traces");
    let m = mass();
    let total = |f: Fit| -> f64 {
        let block_at = block_with(f, m);
        all.iter().map(|(_, tr)| best_latency(tr, &block_at, f.throttle_rate).1).sum()
    };
    let mut f = Fit { throttle_rate: data::stand_in::THROTTLE_RATE, accel_fwd: data::edsy::FWD_ACC, accel_rev: data::edsy::REV_ACC, fa_d: 0.0, jerk: 0.0 };
    let mut best = total(f);
    println!("start {f:?}: {best:.1}");
    // Coordinate descent with shrinking steps.
    let mut step = [0.2, 2.0, 2.0, 0.2, 20.0];
    for round in 0..12 {
        for k in 0..5 {
            for dir in [-1.0, 1.0] {
                loop {
                    let mut g = f;
                    let v = match k {
                        0 => &mut g.throttle_rate,
                        1 => &mut g.accel_fwd,
                        2 => &mut g.accel_rev,
                        3 => &mut g.fa_d,
                        _ => &mut g.jerk,
                    };
                    *v = (*v + dir * step[k]).max(0.0);
                    let e = total(g);
                    if e < best {
                        best = e;
                        f = g;
                    } else {
                        break;
                    }
                }
            }
        }
        for s in step.iter_mut() {
            *s *= 0.5;
        }
        println!("round {round}: {f:?}: {best:.2}");
    }
    let block_at = block_with(f, m);
    for (name, tr) in &all {
        let (lat, e) = best_latency(tr, &block_at, f.throttle_rate);
        let sim = replay::simulate(tr, &block_at, &Replay { latency: lat, throttle_rate: f.throttle_rate }).unwrap();
        let bad = replay::gate(tr, &sim);
        println!("{name}: latency {lat:.3} s, squared error {e:.1}, {} of {} rows outside the gate", bad.len(), tr.rows.len());
    }
}
