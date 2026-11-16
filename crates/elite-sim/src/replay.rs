//! Replaying a measured trace through the simulation (the gate's proof, step 0.9 on).
//!
//! A trace (`trace.rs`, made by `re/elite/measure/tools/make_trace.py` from a run in the user's Elite) holds, a row a video
//! frame, the HUD's speed and the controls at that moment: the keys held (`key_w` ..), the mouse's counts, the pips and flight
//! assist. The replay flies the same controls through `flight::step` at the physics rate from the same start and gives the
//! simulated speed at every row; `gate` says whether each row is inside the user's gate (D10): within 1 m/s, or reached within
//! the time gate (2 video frames or 2 % of the time since the control it answers).
//!
//! What the replay itself assumes (each a parameter of `Replay`, fitted or stated, never tuned by feel):
//! - `latency`: the time from a key's press in the log to the game acting on it, plus the capture's start offset (capture.py
//!   logs when it launched ffmpeg, not its first frame): one number a run, fitted.
//! - `throttle_rate`: how fast the throttle moves while W or S is held (share a second): the keyboard's rule, not found in
//!   the exe, measured off the HUD's throttle marker.
//! - The ship's block comes from the caller (`data::sidewinder_block` or a candidate during a fit); the boost is the
//!   Sidewinder's own (`BoostSpec::sidewinder`, the records).

use crate::boost::{Boost, BoostSpec};
use crate::flight::{self, Block, Input, Motion, PHYSICS_HZ};
use crate::power::Distributor;
use crate::trace::{gate_time_s, Trace, GATE_SPEED_MPS, VIDEO_FRAME_S};
use glam::{DQuat, DVec3};

/// The replay's tick: the physics world's rate (`flight::PHYSICS_HZ`, B). The game's flight step runs on the frame's own dt
/// (0x1401EB050): at the recordings' 60 frames a second that is this tick.
pub const TICK: f64 = 1.0 / PHYSICS_HZ;

/// The replay's own numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Replay {
    pub latency: f64,
    pub throttle_rate: f64,
}

/// The controls at one moment, from a trace row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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

impl Controls {
    /// The first control that differs from `o`, by name (for the gate's report).
    fn changed(&self, o: &Controls) -> Option<&'static str> {
        [
            (self.w != o.w, "W"),
            (self.s != o.s, "S"),
            (self.x != o.x, "X"),
            (self.tab != o.tab, "boost"),
            (self.lat != o.lat, "lateral"),
            (self.vert != o.vert, "vertical"),
            (self.fa_off != o.fa_off, "flight assist"),
            (self.eng_half != o.eng_half, "ENG pips"),
        ]
        .into_iter()
        .find_map(|(c, n)| c.then_some(n))
    }
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

/// The last change of the controls at or before `row` (its logged time and which control), `None` if they never changed.
pub fn last_control_change(tr: &Trace, row: usize) -> Option<(f64, &'static str)> {
    let c = columns(tr)?;
    (1..=row.min(tr.rows.len().saturating_sub(1))).rev().find_map(|i| controls(tr, &c, i).changed(&controls(tr, &c, i - 1)).map(|n| (tr.rows[i][c.t], n)))
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

/// How wide the gate's time window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// 2 video frames, whatever the row (the narrower reading, reported beside D10's).
    Frames,
    /// D10's: 2 video frames or 2 % of the time since the last change of the controls before the row, whichever is larger.
    D10,
}

/// A row outside the gate: its time, the HUD's and the simulated speed, and the control change it answers (its time and name).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Miss {
    pub t: f64,
    pub measured: f64,
    pub simulated: f64,
    pub after: Option<(f64, &'static str)>,
}

/// The gate for one trace under D10's window (`Window::D10`).
pub fn gate(tr: &Trace, sim: &[f64]) -> Vec<Miss> {
    gate_with(tr, sim, Window::D10)
}

/// The gate for one trace: the rows outside it. A row passes within 1 m/s, or if the simulation reaches the measured speed
/// within the time window of that row. The event a row times is the game's answer to the last control change before it (a
/// key, a boost, a pip), so under D10 the window is `gate_time_s(time since that change)`: 2 frames, or 2 % of the answer's
/// time so far once that is larger (after 1.67 s).
pub fn gate_with(tr: &Trace, sim: &[f64], w: Window) -> Vec<Miss> {
    let Some(c) = columns(tr) else { return vec![Miss { t: 0.0, measured: f64::NAN, simulated: f64::NAN, after: None }] };
    let times: Vec<f64> = tr.rows.iter().map(|r| r[c.t]).collect();
    let mut bad = Vec::new();
    for (i, r) in tr.rows.iter().enumerate() {
        let (t, hud) = (times[i], r[c.speed]);
        if (sim[i] - hud).abs() <= GATE_SPEED_MPS {
            continue;
        }
        let after = last_control_change(tr, i);
        let window = match w {
            Window::Frames => 2.0 * VIDEO_FRAME_S,
            Window::D10 => gate_time_s(after.map_or(t - times[0], |(at, _)| t - at)),
        };
        let near = times.iter().zip(sim).any(|(tt, s)| (tt - t).abs() <= window && (s - hud).abs() <= GATE_SPEED_MPS);
        if !near {
            bad.push(Miss { t, measured: hud, simulated: sim[i], after });
        }
    }
    bad
}

/// The sum of squared speed errors (a fit's measure).
pub fn error(tr: &Trace, sim: &[f64]) -> f64 {
    let Some(c) = columns(tr) else { return f64::MAX };
    tr.rows.iter().zip(sim).map(|(r, s)| (s - r[c.speed]).powi(2)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace() -> Trace {
        let mut tr = Trace::new(&["t", "speed", "key_w"]);
        for i in 0..400 {
            let t = i as f64 / 60.0;
            tr.rows.push(vec![t, if t < 1.0 { 0.0 } else { 10.0 }, (i >= 30) as i32 as f64]);
        }
        tr
    }

    /// The control change a row answers is found; D10's window grows with the time since it (2 % past 1.67 s), the frames'
    /// window doesn't.
    #[test]
    fn the_gate_window_follows_the_last_control() {
        let tr = trace();
        assert_eq!(last_control_change(&tr, 10), None);
        assert_eq!(last_control_change(&tr, 300), Some((0.5, "W")));
        // A simulation 0.08 s late on the step at 1.0 s (0.5 s after W): outside both windows.
        let late: Vec<f64> = tr.rows.iter().map(|r| if r[0] < 1.08 { 0.0 } else { 10.0 }).collect();
        assert!(!gate_with(&tr, &late, Window::Frames).is_empty() && !gate_with(&tr, &late, Window::D10).is_empty());
        // A step at 6 s (5.5 s after W: D10's window 0.11 s) met 0.08 s late passes D10 only.
        let mut tr6 = tr.clone();
        for r in tr6.rows.iter_mut() {
            r[1] = if r[0] < 6.0 { 0.0 } else { 10.0 };
        }
        let late6: Vec<f64> = tr6.rows.iter().map(|r| if r[0] < 6.08 { 0.0 } else { 10.0 }).collect();
        assert!(gate_with(&tr6, &late6, Window::D10).is_empty());
        let miss = gate_with(&tr6, &late6, Window::Frames);
        assert!(!miss.is_empty() && miss[0].after == Some((0.5, "W")));
    }
}
