//! The gate (the spec's C10, the user's D10): every measured trace in `tests/traces/` (the user's Elite, step 0.9 on, made by
//! `re/elite/measure/tools/make_trace.py`, numbers only) replayed through the simulation inside the gate.
//!
//! The ship is the game's own records (`data::sidewinder_block`, `BoostSpec::sidewinder`: grade B, task N1), the same for
//! every trace; the measuring rig adds one number a capture chain, its latency (the game's reaction plus the capture's start
//! offset: not a property of the ship), never one a trace.
//!
//! The gate's reading (the spec's Decisions log T7, tasks N2 and N2b): a row passes if the simulated curve, straight between
//! adjacent ticks and rounded to the nearest m/s as the HUD rounds its number, comes within 1 m/s of the HUD's number
//! somewhere inside D10's window (2 video frames, or 2 % of the time since the control change the row answers). `report`
//! prints, per trace, the rms against the curve and the rows outside under every reading side by side (the samples a tick with
//! a 2-frame window, the samples under D10, the curve's exact value under D10, and the curve rounded under D10: the gate), the
//! worst miss and the control it followed, and the boost runs' peaks against the game's. The other ignored tests are the
//! evidence for what is left (`latency_scan`, `throttle_scan`, `phase_shifts`, `print_stretch`, `marker_frame_assumption`)
//! and `fit`, which fits the rig's numbers.

use elite_sim::data::{self, sidewinder_block};
use elite_sim::replay::{self, Miss, Readout, Replay, Window};
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

/// The measuring chain a trace was recorded with: session 1's whole-desktop capture (`s00`) or session 2's crops.
fn session_one(name: &str) -> bool {
    name.starts_with("s00")
}

/// The measuring chain's latency: from a key's logged press to the game acting on it as seen in the recording (the game's own
/// reaction and the capture's start). One number a recording session, the middle of the bound the throttle marker gives in
/// the videos, never fitted and never one a trace (a trace's own latency would push a model error into time). The replay acts
/// on a control at its ticks, so any latency inside one tick-wide step gives the same replay (`latency_scan`).
///
/// Both sessions' numbers rest on one assumption: the video frame in which the throttle marker moves shows the game's input
/// BEFORE the simulation step that uses it (the marker is drawn from the value the next step reads). If it is drawn after
/// that step, both delays are one tick (1/60 s) lower (`marker_frame_assumption` prints the gate both ways).
fn chain_latency(name: &str) -> f64 {
    if session_one(name) { LATENCY_S1 } else { LATENCY_S2 }
}
/// Session 1: its one clean throttle-marker event, X (the key down at 14.0008 s), bounds it to -0.001..0.032 s (the marker
/// still at full in the frame 1 ms before the key, leaving in the frame 32 ms after it); the bound's middle, by session 2's
/// rule (task N2b; it was 0.000, the least error of `latency_scan`, at the bound's best end and on a tick's edge).
const LATENCY_S1: f64 = 0.0155;
/// Session 2: measured without the flight model (`re/elite/measure/tools/marker_latency.py`): after X in s01 and s03 the
/// throttle marker was still seen 17 and 15 ms after the key and gone 34 and 32 ms after it, so the latency lies in
/// 0.017..0.032 s; its middle. The fit (`latency_scan`) is least over 0.018..0.030: the same.
const LATENCY_S2: f64 = 0.024;

/// The replay's numbers as the ship's data and the rig give them.
fn data_replay(name: &str) -> Replay {
    Replay { latency: chain_latency(name), throttle_rate: data::measured::THROTTLE_RATE }
}

/// One trace's replay against the data: the misses under the four readings (the samples with a 2-frame window, the samples
/// under D10, the curve's exact value under D10, the curve rounded under D10: the gate).
struct Result {
    rms: f64,
    rows: usize,
    peak: f64,
    frames: Vec<Miss>,
    samples: Vec<Miss>,
    /// The curve's exact value under D10 (the gate's reading in task N2): reported.
    exact: Vec<Miss>,
    /// The curve read as the HUD reads it (rounded), under D10: the gate (T7).
    gate: Vec<Miss>,
}

fn replay_trace(name: &str, tr: &Trace) -> Result {
    replay_trace_with(tr, &data_replay(name))
}

fn replay_trace_with(tr: &Trace, rp: &Replay) -> Result {
    let m = mass(tr);
    let block_at = |e: f64| sidewinder_block(e, m);
    let sim = replay::simulate(tr, &block_at, rp).unwrap();
    let ticks = replay::simulate_ticks(tr, &block_at, rp).unwrap();
    let rms = (replay::error_curve(tr, &ticks) / tr.rows.len() as f64).sqrt();
    Result {
        rms,
        rows: tr.rows.len(),
        peak: ticks.speed.iter().copied().fold(0.0, f64::max),
        frames: replay::gate_with(tr, &sim, Window::Frames),
        samples: replay::gate_with(tr, &sim, Window::D10),
        exact: replay::gate_curve(tr, &ticks, Window::D10),
        gate: replay::gate_read(tr, &ticks, Window::D10, Readout::Rounded),
    }
}

/// The worst miss (by its speed error) of a list.
fn worst(bad: &[Miss]) -> Option<Miss> {
    bad.iter().copied().max_by(|a, b| (a.simulated - a.measured).abs().total_cmp(&(b.simulated - b.measured).abs()))
}

fn describe(m: &Miss) -> String {
    let after = m.after.map_or("the start".to_string(), |(t, n)| format!("{n} at {t:.2} s ({:.2} s after)", m.t - t));
    format!("at {:.2} s measured {} simulated {:.2}, after {after}", m.t, m.measured, m.simulated)
}

/// A guard, not the gate: the records' replay may not get worse (task N2b, the curve read rounded and one latency a session:
/// 0.46 to 0.73 m/s rms, the boost from rest 0.98; at most 0.4 % of rows outside the gate, s04's 9 of 2413).
#[test]
fn traces_replay_no_worse() {
    for (name, tr) in traces() {
        let r = replay_trace(&name, &tr);
        assert!(r.rms <= GUARD_RMS, "{name}: rms {:.2} m/s", r.rms);
        assert!(r.gate.len() as f64 <= GUARD_SHARE * r.rows as f64, "{name}: {} of {} rows outside the gate", r.gate.len(), r.rows);
    }
}
const GUARD_RMS: f64 = 1.0;
const GUARD_SHARE: f64 = 0.004;

/// **The gate itself** (the spec's C10) under D10's reading against the curve read as the HUD reads it (`replay::gate_read`
/// with `Readout::Rounded`, the Decisions log's T7). Not passed yet (task N2b): 10 rows of 18,344 miss it, all 1.9-2.1 s
/// after W: s01 1 (3.4 m/s) and s04 9 (up to 3.9 m/s); against the curve's exact value 46 miss (s00 1, s01 5, s04 30, s05 8,
/// s06 2). `phase_shifts` shows what the rest is: in every manoeuvre the
/// curve's shape fits the HUD all through (jerk ramp, climb, approach: one time shift fits each part), but the game acts on
/// each key after its own delay (time from the key to the first tick it acts on, N2's review): X 15-22 ms, a thruster key's
/// press 13-20 ms, the boost 26-32 ms, a thruster key's release 55-59 ms, the throttle keys W and S 67-92 ms; the replay acts
/// on every key alike, one latency a session. With any throttle rate from 0.4 a second to instant the misses are the same
/// (`throttle_scan`). The exe's path from a key to the flight controller's demands is read only part of the way (task N2b,
/// `re/elite/notes/sidewinder_input.md`): the keys go through a node graph the game loads as data, so the delays have no
/// rule yet. Run it with `-- --ignored`: it is the step's open item.
#[test]
#[ignore]
fn measured_traces_pass_the_gate() {
    let mut failures = Vec::new();
    for (name, tr) in traces() {
        let r = replay_trace(&name, &tr);
        if let Some(w) = worst(&r.gate) {
            failures.push(format!("{name}: {} of {} rows outside the gate; worst {}", r.gate.len(), r.rows, describe(&w)));
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
            "{name}: latency {:.4} s, rms {:.2} m/s, outside: {} (samples, 2 frames) / {} (samples, D10) / {} (curve exact, D10) / {} (curve rounded, D10: the gate) of {} rows",
            chain_latency(&name),
            r.rms,
            r.frames.len(),
            r.samples.len(),
            r.exact.len(),
            r.gate.len(),
            r.rows
        );
        for (label, bad) in [("samples, 2 frames", &r.frames), ("curve exact, D10", &r.exact), ("the gate", &r.gate)] {
            if let Some(w) = worst(bad) {
                println!("    worst ({label}): {}", describe(&w));
            }
        }
        // Where the gate's misses fall: by the control they follow.
        let mut by: Vec<(String, usize, f64)> = Vec::new();
        for m in &r.gate {
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
            println!("    the gate: {n} rows after {key}, by up to {err:.2} m/s");
        }
        if let Some((_, peak)) = BOOST_PEAKS.iter().find(|(p, _)| name.starts_with(p)) {
            println!("    boost peak: game {peak}, simulated {:.2}", r.peak);
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
    Replay { latency: if session_one(name) { p[2] } else { p[1] }, throttle_rate: p[0] }
}

fn fit_error(all: &[(String, Trace)], p: [f64; 3]) -> f64 {
    all.iter()
        .map(|(name, tr)| {
            let m = mass(tr);
            let ticks = replay::simulate_ticks(tr, &|e: f64| sidewinder_block(e, m), &replay_with(p, name)).unwrap();
            replay::error_curve(tr, &ticks)
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

/// Each chain's latency by a scan, the fit's other way (a replay acts on the controls at its ticks, so the error steps with
/// the latency a tick at a time and coordinate descent stalls): for every latency in 2 ms steps, the chain's summed squared
/// error and its rows outside the gate. `cargo test --profile agent -p elite-sim --test traces latency_scan -- --ignored
/// --nocapture`.
#[test]
#[ignore]
fn latency_scan() {
    let all = traces();
    for (chain, s1) in [("session 1", true), ("session 2", false)] {
        println!("{chain}:");
        for i in -25..=50 {
            let l = i as f64 * 0.002;
            let (mut e, mut exact, mut gate) = (0.0, 0usize, 0usize);
            for (name, tr) in all.iter().filter(|(n, _)| session_one(n) == s1) {
                let m = mass(tr);
                let rp = Replay { latency: l, ..data_replay(name) };
                let ticks = replay::simulate_ticks(tr, &|x: f64| sidewinder_block(x, m), &rp).unwrap();
                e += replay::error_curve(tr, &ticks);
                exact += replay::gate_curve(tr, &ticks, Window::D10).len();
                gate += replay::gate_read(tr, &ticks, Window::D10, Readout::Rounded).len();
            }
            println!("    latency {l:.3} s: error {e:.1}, outside: {exact} (curve exact) / {gate} (the gate)");
        }
    }
}

/// The one assumption under both sessions' delays (`chain_latency`): the marker's frame shows the input before the step that
/// uses it. Per trace, the rows outside (curve exact / the gate) as assumed, and with both delays one tick lower (the marker
/// drawn after that step). `cargo test --profile agent -p elite-sim --test traces marker_frame_assumption -- --ignored
/// --nocapture`.
#[test]
#[ignore]
fn marker_frame_assumption() {
    for (label, shift) in [("before the step (as used)", 0.0), ("after the step (one tick lower)", replay::TICK)] {
        let mut total = (0usize, 0usize);
        let counts: Vec<String> = traces()
            .iter()
            .map(|(name, tr)| {
                let rp = Replay { latency: chain_latency(name) - shift, ..data_replay(name) };
                let r = replay_trace_with(tr, &rp);
                total.0 += r.exact.len();
                total.1 += r.gate.len();
                format!("{} {}/{}", &name[..3], r.exact.len(), r.gate.len())
            })
            .collect();
        println!("{label}: {} (all {}/{})", counts.join(", "), total.0, total.1);
    }
}

/// How much the keyboard throttle's model matters: every trace replayed with other throttle rates (the chain latencies
/// held), each trace's rows outside the gate and rms. (2026-11-16: from 0.4 a second to instant the same, to the mm/s: the
/// jerk limit and the acceleration limits hold the speed far under the throttle's target all through a W ramp.) `cargo test --profile agent -p elite-sim --test traces throttle_scan -- --ignored
/// --nocapture`.
#[test]
#[ignore]
fn throttle_scan() {
    let all = traces();
    for rate in [0.3, 0.4, data::measured::THROTTLE_RATE, 0.7, 1.0, 1e6] {
        let counts: Vec<String> = all
            .iter()
            .map(|(name, tr)| {
                let m = mass(tr);
                let rp = Replay { throttle_rate: rate, ..data_replay(name) };
                let ticks = replay::simulate_ticks(tr, &|x: f64| sidewinder_block(x, m), &rp).unwrap();
                let rms = (replay::error_curve(tr, &ticks) / tr.rows.len() as f64).sqrt();
                format!("{} {} ({rms:.3} m/s)", &name[..3], replay::gate_read(tr, &ticks, Window::D10, Readout::Rounded).len())
            })
            .collect();
        println!("throttle rate {rate}: {}", counts.join(", "));
    }
}

/// One trace's HUD speed and the simulated curve side by side over a stretch: `TRACE=s04 FROM=2.8 TO=6 cargo test --profile
/// agent -p elite-sim --test traces print_stretch -- --ignored --nocapture` (every row, the curve at its time, the difference).
#[test]
#[ignore]
fn print_stretch() {
    let want = std::env::var("TRACE").unwrap_or_else(|_| "s04".into());
    let from: f64 = std::env::var("FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let to: f64 = std::env::var("TO").ok().and_then(|v| v.parse().ok()).unwrap_or(f64::MAX);
    for (name, tr) in traces().iter().filter(|(n, _)| n.starts_with(want.as_str())) {
        let m = mass(tr);
        let ticks = replay::simulate_ticks(tr, &|x: f64| sidewinder_block(x, m), &data_replay(name)).unwrap();
        let (t, s) = (tr.column("t").unwrap(), tr.column("speed").unwrap());
        for r in tr.rows.iter().filter(|r| r[t] >= from && r[t] <= to) {
            let c = ticks.at(r[t]).unwrap_or(f64::NAN);
            println!("{name} {:.4} hud {} curve {c:.2} diff {:+.2}", r[t], r[s], c - r[s]);
        }
    }
}

/// What the remaining errors are, a control change at a time: over the 4 s after each change (to the next one), the time
/// shift of the curve (later +) that fits the HUD best if the HUD rounds its number and if it cuts the fraction off (the
/// curve then sits half a unit over the HUD on average), with each fit's rms. `cargo test --profile agent -p elite-sim --test
/// traces phase_shifts -- --ignored --nocapture`.
#[test]
#[ignore]
fn phase_shifts() {
    for (name, tr) in traces() {
        let m = mass(&tr);
        let ticks = replay::simulate_ticks(&tr, &|x: f64| sidewinder_block(x, m), &data_replay(&name)).unwrap();
        let (t, s) = (tr.column("t").unwrap(), tr.column("speed").unwrap());
        let mut changes: Vec<(f64, &str)> = (1..tr.rows.len()).filter_map(|i| replay::last_control_change(&tr, i)).collect();
        changes.dedup();
        for (k, (at, what)) in changes.iter().enumerate() {
            let next = changes.get(k + 1).map_or(f64::MAX, |c| c.0);
            // The whole answer (to 4 s), then its parts: the jerk limit's ramp, the climb, the approach to the cap.
            for (a, b) in [(0.2, 4.0), (0.2, 1.0), (1.0, 2.0), (2.0, 4.0)] {
                let rows: Vec<(f64, f64)> = tr.rows.iter().filter(|r| r[t] > at + a && r[t] < (at + b).min(next)).map(|r| (r[t], r[s])).collect();
                let moved = rows.first().zip(rows.last()).map_or(0.0, |(f, l)| (l.1 - f.1).abs());
                if rows.len() < 20 || moved < 5.0 {
                    continue;
                }
                let fit = |bias: f64| {
                    let mut best = (0.0, f64::MAX);
                    for i in -60..=60 {
                        let d = i as f64 * 0.002;
                        let e: Vec<f64> = rows.iter().filter_map(|(tt, h)| ticks.at(tt - d).map(|c| c - h - bias)).collect();
                        let rms = (e.iter().map(|x| x * x).sum::<f64>() / e.len() as f64).sqrt();
                        if rms < best.1 {
                            best = (d, rms);
                        }
                    }
                    best
                };
                let (round, cut) = (fit(0.0), fit(0.5));
                println!(
                    "{name} after {what} at {at:.2}, {a:.1}-{b:.1} s: rounding: shift {:+.3} s rms {:.2}; cutting: shift {:+.3} s rms {:.2}",
                    round.0, round.1, cut.0, cut.1
                );
            }
        }
    }
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
