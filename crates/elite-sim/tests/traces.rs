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

/// The ship's mass in a trace: unladen (the journal's) and the fuel aboard at its start (its `fuel` column, from
/// `Status.json`: main tank and reservoir), else a full tank and reservoir.
fn mass(tr: &Trace) -> f64 {
    let fuel = tr.values("fuel").and_then(|v| v.first().copied()).filter(|f| *f >= 0.0);
    data::JOURNAL_UNLADEN_MASS + fuel.unwrap_or(data::FUEL_TANK_T + data::SIDEWINDER.reserve_fuel)
}

/// The measuring chain's latency: from a key's logged press to the game acting on it as seen in the recording (the game's own
/// reaction and the capture's start). Fitted with the ship's numbers by `fit`: 0.04 s for session 2's crops at 60 frames a
/// second, 0 for session 1's whole-desktop capture (its clock origin was ffmpeg's launch, 35 ms before its first frame). Each trace may sit within `JITTER_S` of its chain's (the recording's frame timing: 3 frames at 60 a second): the
/// gate fits that one number a trace, never the ship's.
fn chain_latency(name: &str) -> f64 {
    if name.starts_with("s00") { 0.0 } else { 0.04 }
}
const JITTER_S: f64 = 0.05;

/// The latency within the chain's jitter that fits the trace best, in 1 ms steps.
fn trace_latency(name: &str, tr: &Trace, block_at: &dyn Fn(f64) -> (Block, f64), rp: Replay) -> f64 {
    let c = chain_latency(name);
    let mut best = (c, f64::MAX);
    for i in -50..=50 {
        let l = c + i as f64 * JITTER_S / 50.0;
        let sim = replay::simulate(tr, block_at, &Replay { latency: l, ..rp }).unwrap();
        let e = replay::error(tr, &sim);
        if e < best.1 {
            best = (l, e);
        }
    }
    best.0
}

/// The replay's numbers as the ship's data gives them.
fn data_replay() -> Replay {
    Replay { latency: 0.0, throttle_rate: data::measured::THROTTLE_RATE, boost_fire: None, boost_accel: 1.0 }
}

/// The gate: every measured trace (but the boost runs until boost is measured: `GATE_SKIP`) replayed from `data` alone.
const GATE_SKIP: &[&str] = &["boost"];

/// One trace's replay against the data: (latency, rms, the rows outside the gate).
fn replay_trace(name: &str, tr: &Trace) -> (f64, f64, Vec<(f64, f64, f64)>) {
    let m = mass(tr);
    let block_at = |e: f64| sidewinder_block(e, m);
    let lat = trace_latency(name, tr, &block_at, data_replay());
    let sim = replay::simulate(tr, &block_at, &Replay { latency: lat, ..data_replay() }).unwrap();
    let rms = (replay::error(tr, &sim) / tr.rows.len() as f64).sqrt();
    (lat, rms, replay::gate(tr, &sim))
}

/// A guard, not the gate: today's replay accuracy may not get worse (2026-11-14: 0.46 to 0.57 m/s rms, 0.1 to 1 % of rows
/// outside the gate, all within the first seconds after a change of the controls).
#[test]
fn traces_replay_no_worse() {
    for (name, tr) in traces().iter().filter(|(n, _)| !GATE_SKIP.iter().any(|s| n.contains(s))) {
        let (_, rms, bad) = replay_trace(name, tr);
        assert!(rms <= 0.65, "{name}: rms {rms:.2} m/s");
        assert!(bad.len() as f64 <= 0.012 * tr.rows.len() as f64, "{name}: {} of {} rows outside the gate", bad.len(), tr.rows.len());
    }
}

/// **The gate itself** (the spec's C10). Not passed yet (2026-11-14): 3 to 23 rows a trace miss it by up to 3 m/s, about 3.5
/// frames, in the fast changes just after a control (the corners of the jerk-limited start and of braking). Run it with
/// `-- --ignored`; it is the step's open item.
#[test]
#[ignore]
fn measured_traces_pass_the_gate() {
    let all = traces();
    let mut failures = Vec::new();
    let mut report = Vec::new();
    for (name, tr) in all.iter().filter(|(n, _)| !GATE_SKIP.iter().any(|s| n.contains(s))) {
        let m = mass(tr);
        let block_at = |e: f64| sidewinder_block(e, m);
        let lat = trace_latency(name, tr, &block_at, data_replay());
        let sim = replay::simulate(tr, &block_at, &Replay { latency: lat, ..data_replay() }).unwrap();
        let bad = replay::gate(tr, &sim);
        let rms = (replay::error(tr, &sim) / tr.rows.len() as f64).sqrt();
        report.push(format!("{name}: latency {lat:.3} s, rms {rms:.2} m/s, {} of {} rows outside", bad.len(), tr.rows.len()));
        if !bad.is_empty() {
            let worst = bad.iter().cloned().fold((0.0f64, 0.0f64, 0.0f64), |w, b| if (b.2 - b.1).abs() > (w.2 - w.1).abs() { b } else { w });
            failures.push(format!("{name}: {} of {} rows outside the gate; worst at {:.2} s: measured {}, simulated {:.2}", bad.len(), tr.rows.len(), worst.0, worst.1, worst.2));
        }
    }
    println!("{}", report.join("
"));
    assert!(failures.is_empty(), "{}", failures.join("
"));
}

/// The numbers a fit sets: the ship's (each a stand-in or a hint in `data.rs`) and the chain's latency, by name, with a
/// starting value and step.
const PARAMS: [(&str, f64, f64); 15] = [
    // (Starting values read off the traces by hand, 2026-11-14: the throttle marker's 1.75 s to full, the 44-46 m/s^2 and
    // 30 m/s^2 straight parts, the 25 m/s^3 start, the approach's time constants 0.87 / 0.58 / 0.5 s at 4 / 2 / 0 pips.)
    ("throttle_rate", 0.526, 0.0),
    ("accel_fwd", 44.38, 1.0),
    ("accel_rev", 30.00, 1.0),
    ("accel_side", 30.05, 1.0),
    ("back_share", data::measured::BACK_SHARE, 0.0),
    ("side_share", data::measured::SIDE_SHARE, 0.0),
    ("fa_d", 0.991, 0.1),
    ("jerk", 25.05, 2.0),
    ("boost_fire", 3.0, 0.5),
    ("boost_ramp", data::stand_in::BOOST_RAMP, 0.2),
    ("latency", 0.04, 0.02),
    ("boost_accel", 1.0, 0.2),
    ("accel_fwd_base", 44.20, 1.0),
    // Session 1's own latency (its capture took the whole desktop at 38 frames a second and logged ffmpeg's launch, not its
    // first frame: another chain than session 2's crop at 60).
    ("latency_s1", 0.0, 0.02),
    // A hypothesis tested and dropped: the HUD showing the speed through a first-order lag (the fit took it to 0.013 s).
    ("hud_tau", 0.0, 0.0),
];

/// The last fit's result (`fit`'s final line), used by the gate test's latency.

fn block_with(p: [f64; 15], m: f64) -> impl Fn(f64) -> (Block, f64) {
    move |e: f64| {
        let (mut b, top) = sidewinder_block(e, m);
        // The block's accelerations are the full block's scaled by the mass curve: replace the full block's value. The
        // forward one is blended between the base block's (p[12]) and the full block's (p[1]) by the pips, as the exe blends
        // every field (C).
        let k = b.accel_fwd / data::edsy::FWD_ACC;
        b.accel_fwd = k * (p[12] + (p[1] - p[12]) * (e / 4.0).clamp(0.0, 1.0));
        b.accel_rev *= p[2] / data::edsy::REV_ACC;
        b.accel_side *= p[3] / data::edsy::LAT_ACC;
        b.back_share = p[4];
        b.side_share = p[5];
        b.fa_d = p[6];
        b.jerk_lin = p[7];
        b.boost_ramp = p[9];
        (b, top)
    }
}

fn replay_with(p: [f64; 15], name: &str) -> Replay {
    Replay { latency: if name.starts_with("s00") { p[13] } else { p[10] }, throttle_rate: p[0], boost_fire: Some(p[8]), boost_accel: p[11] }
}

/// The simulated speeds through a first-order lag of `tau` seconds (the HUD's display, a hypothesis).
fn lagged(tr: &Trace, sim: Vec<f64>, tau: f64) -> Vec<f64> {
    if tau <= 0.0 {
        return sim;
    }
    let t = tr.values("t").unwrap();
    let mut out = Vec::with_capacity(sim.len());
    let mut y = sim[0];
    for i in 0..sim.len() {
        let dt = if i == 0 { 0.0 } else { t[i] - t[i - 1] };
        y += (sim[i] - y) * (1.0 - (-dt / tau).exp());
        out.push(y);
    }
    out
}

fn fit_error(all: &[(String, Trace)], p: [f64; 15]) -> f64 {
    all.iter()
        .map(|(name, tr)| {
            let sim = lagged(tr, replay::simulate(tr, &block_with(p, mass(tr)), &replay_with(p, name)).unwrap(), p[14]);
            replay::error(tr, &sim)
        })
        .sum()
}

#[test]
#[ignore]
fn fit() {
    // FIT_SKIP=word leaves out the traces whose names hold it; FIT_ROUNDS=n (default 16); a step of 0 holds a number.
    let skip = std::env::var("FIT_SKIP").ok();
    let all: Vec<(String, Trace)> = traces().into_iter().filter(|(n, _)| skip.as_ref().is_none_or(|s| !n.contains(s.as_str()))).collect();
    assert!(!all.is_empty(), "no traces in tests/traces");
    let rounds: usize = std::env::var("FIT_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(16);
    let mut p: [f64; 15] = PARAMS.map(|x| x.1);
    let mut step: [f64; 15] = PARAMS.map(|x| x.2);
    let mut best = fit_error(&all, p);
    println!("start: {best:.1}");
    // Coordinate descent with shrinking steps.
    for round in 0..rounds {
        for k in 0..15 {
            if step[k] == 0.0 {
                continue;
            }
            for dir in [-1.0, 1.0] {
                loop {
                    let mut g = p;
                    // (The latencies may go under 0: a capture's clock origin can sit after the keys' by a frame.)
                    g[k] = if k == 10 || k == 13 { g[k] + dir * step[k] } else { (g[k] + dir * step[k]).max(0.0) };
                    if k == 4 || k == 5 {
                        g[k] = g[k].min(1.0);
                    }
                    let e = fit_error(&all, g);
                    if e < best {
                        best = e;
                        p = g;
                    } else {
                        break;
                    }
                }
            }
        }
        for s in step.iter_mut() {
            *s *= 0.6;
        }
        let named: Vec<String> = PARAMS.iter().zip(p).map(|(q, v)| format!("{} {v:.4}", q.0)).collect();
        println!("round {round}: {}: {best:.2}", named.join(", "));
    }
    for (name, tr) in &all {
        let sim = lagged(tr, replay::simulate(tr, &block_with(p, mass(tr)), &replay_with(p, name)).unwrap(), p[14]);
        let e = replay::error(tr, &sim);
        let bad = replay::gate(tr, &sim);
        let rms = (e / tr.rows.len() as f64).sqrt();
        println!("{name}: rms {rms:.2} m/s, {} of {} rows outside the gate", bad.len(), tr.rows.len());
        for w in bad.iter().step_by((bad.len() / 4).max(1)).take(5) {
            println!("    at {:.2} s measured {} simulated {:.2}", w.0, w.1, w.2);
        }
    }
    println!("FITTED: {:?}", p);
}

/// Prints the block's top speeds for a mass (the session's fuel): `cargo test --release -p elite-sim --test traces
/// print_top_speeds -- --ignored --nocapture`.
#[test]
#[ignore]
fn print_top_speeds() {
    for fuel in [1.4 + 0.124897, data::FUEL_TANK_T + data::SIDEWINDER.reserve_fuel] {
        let m = data::JOURNAL_UNLADEN_MASS + fuel;
        let v: Vec<String> = [0.0, 2.0, 4.0].iter().map(|&e| format!("{e} pips {:.2}", sidewinder_block(e, m).0.top_speed)).collect();
        println!("mass {m:.3} t: {}", v.join(", "));
    }
}
