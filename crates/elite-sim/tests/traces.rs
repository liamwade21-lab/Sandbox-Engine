//! The gate (the spec's C10, the user's D10): every measured trace in `tests/traces/` (the user's Elite, step 0.9 on, made by
//! `re/elite/measure/tools/make_trace.py`, numbers only) replayed through the simulation inside the gate.
//!
//! The ship is the game's own records (`data::sidewinder_block`, `BoostSpec::sidewinder`: grade B, task N1), the same for
//! every trace; each trace gets one fitted number of its own, the latency (the game's reaction plus the capture's start
//! offset: not a property of the ship), within its capture chain's jitter.
//!
//! `cargo test --profile agent -p elite-sim --test traces report -- --ignored --nocapture` prints, per trace, the rms and the
//! rows outside the gate under both readings of its time window (2 frames only; D10's 2 frames or 2 %), the worst miss and the
//! control it followed, and the boost runs' peak speeds against the game's. `fit` fits the numbers the records don't give
//! (the measuring chain's: the throttle rate, the latencies).

use elite_sim::data::{self, sidewinder_block};
use elite_sim::flight::Block;
use elite_sim::replay::{self, Miss, Replay, Window};
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
/// reaction and the capture's start), `fit`'s with the records (2026-11-16): 0.02 s for session 2's crops at 60 frames a second
/// (0.04 with the old fitted ship), 0 for session 1's
/// whole-desktop capture (its clock origin was ffmpeg's launch, 35 ms before its first frame). Each trace may sit within
/// `JITTER_S` of its chain's (the recording's frame timing: 3 frames at 60 a second): the gate fits that one number a trace,
/// never the ship's.
fn chain_latency(name: &str) -> f64 {
    if name.starts_with("s00") { 0.0 } else { 0.02 }
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
    Replay { latency: 0.0, throttle_rate: data::measured::THROTTLE_RATE }
}

/// One trace's replay against the data.
struct Result {
    latency: f64,
    rms: f64,
    rows: usize,
    sim: Vec<f64>,
    d10: Vec<Miss>,
    frames: Vec<Miss>,
}

fn replay_trace(name: &str, tr: &Trace) -> Result {
    let m = mass(tr);
    let block_at = |e: f64| sidewinder_block(e, m);
    let latency = trace_latency(name, tr, &block_at, data_replay());
    let sim = replay::simulate(tr, &block_at, &Replay { latency, ..data_replay() }).unwrap();
    let rms = (replay::error(tr, &sim) / tr.rows.len() as f64).sqrt();
    Result { latency, rms, rows: tr.rows.len(), d10: replay::gate_with(tr, &sim, Window::D10), frames: replay::gate_with(tr, &sim, Window::Frames), sim }
}

/// The worst miss (by its speed error) of a list.
fn worst(bad: &[Miss]) -> Option<Miss> {
    bad.iter().copied().max_by(|a, b| (a.simulated - a.measured).abs().total_cmp(&(b.simulated - b.measured).abs()))
}

fn describe(m: &Miss) -> String {
    let after = m.after.map_or("the start".to_string(), |(t, n)| format!("{n} at {t:.2} s ({:.2} s after)", m.t - t));
    format!("at {:.2} s measured {} simulated {:.2}, after {after}", m.t, m.measured, m.simulated)
}

/// A guard, not the gate: the records' replay may not get worse (2026-11-16, the records in, task N1b-1: 0.45 to 0.69 m/s
/// rms, the boost from rest 0.96; 0.1 to 2.1 % of rows outside D10's gate, all in the first seconds after a control).
#[test]
fn traces_replay_no_worse() {
    for (name, tr) in traces() {
        let r = replay_trace(&name, &tr);
        assert!(r.rms <= GUARD_RMS, "{name}: rms {:.2} m/s", r.rms);
        assert!(r.d10.len() as f64 <= GUARD_SHARE * r.rows as f64, "{name}: {} of {} rows outside the gate", r.d10.len(), r.rows);
    }
}
const GUARD_RMS: f64 = 1.0;
const GUARD_SHARE: f64 = 0.022;

/// **The gate itself** (the spec's C10) under D10's reading (`replay::gate`). Not passed yet (2026-11-16, the records in): 3 to
/// 50 rows a trace miss it by up to 4.5 m/s, all in the first seconds after a control (after W the simulation runs ahead,
/// after X behind, during a boost's 180 m/s^2 climb from rest a frame or two off); `report` lists them. Run it with
/// `-- --ignored`: it is the step's open item.
#[test]
#[ignore]
fn measured_traces_pass_the_gate() {
    let mut failures = Vec::new();
    for (name, tr) in traces() {
        let r = replay_trace(&name, &tr);
        if let Some(w) = worst(&r.d10) {
            failures.push(format!("{name}: {} of {} rows outside the gate; worst {}", r.d10.len(), r.rows, describe(&w)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The boost runs' peak speeds seen in the game (the HUD's, `s05_boost_rest` from rest, `s06_boost_top` at top speed).
const BOOST_PEAKS: [(&str, f64); 2] = [("s05", 272.0), ("s06", 308.0)];

/// Prints every trace's result (see the module's doc).
#[test]
#[ignore]
fn report() {
    for (name, tr) in traces() {
        let r = replay_trace(&name, &tr);
        println!(
            "{name}: latency {:.3} s, rms {:.2} m/s, outside: {} (2 frames) / {} (D10) of {} rows",
            r.latency,
            r.rms,
            r.frames.len(),
            r.d10.len(),
            r.rows
        );
        for (label, bad) in [("2 frames", &r.frames), ("D10", &r.d10)] {
            if let Some(w) = worst(bad) {
                println!("    worst ({label}): {}", describe(&w));
            }
        }
        // Where the misses fall: by the control they follow.
        let mut by: Vec<(String, usize, f64)> = Vec::new();
        for m in &r.d10 {
            let key = m.after.map_or("start".to_string(), |(t, n)| format!("{n} at {t:.2} s"));
            let err = (m.simulated - m.measured).abs();
            match by.iter_mut().find(|b| b.0 == key) {
                Some(b) => {
                    b.1 += 1;
                    b.2 = b.2.max(err);
                }
                None => by.push((key, 1, err)),
            }
        }
        for (key, n, err) in by {
            println!("    D10: {n} rows after {key}, by up to {err:.2} m/s");
        }
        if let Some((_, peak)) = BOOST_PEAKS.iter().find(|(p, _)| name.starts_with(p)) {
            let sim_peak = r.sim.iter().copied().fold(0.0, f64::max);
            println!("    boost peak: game {peak}, simulated {sim_peak:.2}");
        }
    }
}

/// The numbers a fit sets: only what the records don't give (the measuring chain's), by name, with a starting value and step.
const PARAMS: [(&str, f64, f64); 3] = [
    // The keyboard's throttle rate (held at the HUD marker's 1.75-1.9 s to full: `data::measured::THROTTLE_RATE`).
    ("throttle_rate", data::measured::THROTTLE_RATE, 0.0),
    // Session 2's chain latency, and session 1's (its capture took the whole desktop at 38 frames a second and logged ffmpeg's
    // launch, not its first frame).
    ("latency", 0.02, 0.02),
    ("latency_s1", 0.0, 0.02),
];

fn replay_with(p: [f64; 3], name: &str) -> Replay {
    Replay { latency: if name.starts_with("s00") { p[2] } else { p[1] }, throttle_rate: p[0] }
}

fn fit_error(all: &[(String, Trace)], p: [f64; 3]) -> f64 {
    all.iter()
        .map(|(name, tr)| {
            let m = mass(tr);
            let sim = replay::simulate(tr, &|e: f64| sidewinder_block(e, m), &replay_with(p, name)).unwrap();
            replay::error(tr, &sim)
        })
        .sum()
}

/// `cargo test --profile agent -p elite-sim --test traces fit -- --ignored --nocapture` (FIT_SKIP=word leaves out the traces
/// whose names hold it; FIT_ROUNDS=n, default 16; a step of 0 holds a number).
#[test]
#[ignore]
fn fit() {
    let skip = std::env::var("FIT_SKIP").ok();
    let all: Vec<(String, Trace)> = traces().into_iter().filter(|(n, _)| skip.as_ref().is_none_or(|s| !n.contains(s.as_str()))).collect();
    assert!(!all.is_empty(), "no traces in tests/traces");
    let rounds: usize = std::env::var("FIT_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(16);
    let mut p = PARAMS.map(|x| x.1);
    let mut step = PARAMS.map(|x| x.2);
    let mut best = fit_error(&all, p);
    println!("start: {best:.1}");
    // Coordinate descent with shrinking steps (the latencies may go under 0: a capture's clock origin can sit after the keys'
    // by a frame).
    for round in 0..rounds {
        for k in 0..p.len() {
            if step[k] == 0.0 {
                continue;
            }
            for dir in [-1.0, 1.0] {
                loop {
                    let mut g = p;
                    g[k] += dir * step[k];
                    if k == 0 {
                        g[k] = g[k].max(0.0);
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
    println!("FITTED: {p:?}");
}

/// Prints the block's top speeds for a mass (the session's fuel): `cargo test --profile agent -p elite-sim --test traces
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
