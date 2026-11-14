//! Replaying a measured trace through the simulation (the gate's proof, step 0.9 on).
//!
//! A trace (`trace.rs`, made by `re/elite/measure/tools/make_trace.py` from a run in the user's Elite) holds, a row a video
//! frame, the HUD's speed and the controls at that moment: the keys held (`key_w` ..), the mouse's counts, the pips and flight
//! assist. The replay flies the same controls through `flight::step` at 60 Hz from the same start and gives the simulated
//! speed at every row; `gate` says whether each row is inside the user's gate (D10): within 1 m/s, or reached within the
//! time gate (2 video frames or 2 %).
//!
//! What the replay itself assumes (each a parameter of `Replay`, fitted or stated, never tuned by feel):
//! - `latency`: the time from a key's press in the log to the game acting on it, plus the capture's start offset (capture.py
//!   logs when it launched ffmpeg, not its first frame): one number a run, fitted.
//! - `throttle_rate`: how fast the throttle moves while W or S is held (share a second): the keyboard's rule, not found in
//!   the exe, fitted.
//! - The ship's block comes from the caller (`data::sidewinder_block` or a candidate during a fit).

use crate::boost::{Boost, BoostSpec};
use crate::flight::{self, Block, Input, Motion};
use crate::power::Distributor;
use crate::trace::{gate_time_s, Trace, GATE_SPEED_MPS};
use glam::{DQuat, DVec3};

pub const TICK: f64 = 1.0 / 60.0;

/// The replay's own numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Replay {
    pub latency: f64,
    pub throttle_rate: f64,
}

/// The controls at one moment, from a trace row.
#[derive(Clone, Copy, Debug, Default)]
struct Controls {
    w: bool,
    s: bool,
    x: bool,
    tab: bool,
    lat: f64,
    vert: f64,
    fa_off: bool,
    eng_half: f64,
}

struct Columns {
    t: usize,
    speed: usize,
    keys: [Option<usize>; 8],
    pip_eng: Option<usize>,
    fa_off: Option<usize>,
}

fn columns(tr: &Trace) -> Option<Columns> {
    let k = |n: &str| tr.column(n);
    Some(Columns {
        t: k("t")?,
        speed: k("speed")?,
        keys: [k("key_w"), k("key_s"), k("key_x"), k("key_tab"), k("key_e"), k("key_q"), k("key_r"), k("key_f")],
        pip_eng: k("pip_eng"),
        fa_off: k("fa_off"),
    })
}

fn controls(tr: &Trace, c: &Columns, row: usize) -> Controls {
    let r = &tr.rows[row];
    let on = |i: usize| c.keys[i].is_some_and(|j| r[j] > 0.5);
    Controls {
        w: on(0),
        s: on(1),
        x: on(2),
        tab: on(3),
        lat: on(4) as i8 as f64 - on(5) as i8 as f64,
        vert: on(6) as i8 as f64 - on(7) as i8 as f64,
        fa_off: c.fa_off.is_some_and(|j| r[j] > 0.5),
        eng_half: c.pip_eng.map_or(4.0, |j| r[j]),
    }
}

/// Simulates the trace with `block_at(eng_pips)` (the block and the full block's top speed): the speed at every row's time.
pub fn simulate(tr: &Trace, block_at: &dyn Fn(f64) -> (Block, f64), rp: &Replay) -> Option<Vec<f64>> {
    let c = columns(tr)?;
    if tr.rows.is_empty() {
        return Some(Vec::new());
    }
    let times: Vec<f64> = tr.rows.iter().map(|r| r[c.t]).collect();
    let mut m = Motion { vel: DVec3::new(0.0, 0.0, -tr.rows[0][c.speed]), rot: DQuat::IDENTITY, ..Motion::default() };
    let mut dist = Distributor::new(&crate::data::DISTRIBUTOR_1E);
    let mut boost = Boost::default();
    let mut throttle = 0.0f64;
    let mut last = Controls::default();
    let mut out = Vec::with_capacity(times.len());
    let mut t = times[0];
    let mut row = 0usize;
    let mut ctl_row = 0usize;
    // (The ship's starting throttle is taken from the first speed: at rest 0; a run starting at speed is not supported.)
    while row < times.len() {
        // The controls in force: the row the game is acting on, `latency` ago.
        while ctl_row + 1 < times.len() && times[ctl_row + 1] <= t - rp.latency {
            ctl_row += 1;
        }
        let k = if times[0] <= t - rp.latency { controls(tr, &c, ctl_row) } else { Controls::default() };
        let (block, full_top) = block_at(k.eng_half / 2.0);
        let spec = BoostSpec::sidewinder(block.boost_ramp);
        if k.x {
            throttle = 0.0;
        }
        throttle = (throttle + rp.throttle_rate * TICK * (k.w as i8 - k.s as i8) as f64).clamp(-1.0, 1.0);
        if k.tab && !last.tab {
            let _ = boost.press(&spec, &mut dist, true);
        }
        last = k;
        dist.recharge(TICK, true);
        boost.tick(&spec, TICK);
        let input = Input { forward: boost.forward(&spec, throttle), lateral: k.lat, vertical: k.vert, flight_assist: !k.fa_off, ..Input::default() };
        flight::step(&block, full_top, boost.firing(), &input, DVec3::ZERO, &mut m, TICK);
        t += TICK;
        // Every row passed in this tick gets this tick's speed.
        while row < times.len() && times[row] <= t {
            out.push(m.vel.length());
            row += 1;
        }
    }
    Some(out)
}

/// The gate for one trace: the rows outside it (time, measured, simulated). A row passes within 1 m/s, or if the simulation
/// reaches the measured speed within the time gate of that row: `gate_time_s` of the time since the last change of the
/// controls (the event's own length so far: 2 frames or 2 % of it).
pub fn gate(tr: &Trace, sim: &[f64]) -> Vec<(f64, f64, f64)> {
    let Some(c) = columns(tr) else { return vec![(0.0, f64::NAN, f64::NAN)] };
    let times: Vec<f64> = tr.rows.iter().map(|r| r[c.t]).collect();
    let control_cols: Vec<usize> = tr.columns.iter().enumerate().filter(|(_, n)| n.starts_with("key_") || n.starts_with("mouse_") || n.starts_with("pip_") || *n == "fa_off").map(|(i, _)| i).collect();
    let mut since = times.first().copied().unwrap_or(0.0);
    let mut bad = Vec::new();
    for (i, r) in tr.rows.iter().enumerate() {
        if i > 0 && control_cols.iter().any(|&j| tr.rows[i - 1][j] != r[j]) {
            since = times[i];
        }
        let (t, hud) = (times[i], r[c.speed]);
        if (sim[i] - hud).abs() <= GATE_SPEED_MPS {
            continue;
        }
        let window = gate_time_s(t - since);
        let near = times.iter().zip(sim).any(|(tt, s)| (tt - t).abs() <= window && (s - hud).abs() <= GATE_SPEED_MPS);
        if !near {
            bad.push((t, hud, sim[i]));
        }
    }
    bad
}

/// The sum of squared speed errors (a fit's measure).
pub fn error(tr: &Trace, sim: &[f64]) -> f64 {
    let Some(c) = columns(tr) else { return f64::MAX };
    tr.rows.iter().zip(sim).map(|(r, s)| (s - r[c.speed]).powi(2)).sum()
}
