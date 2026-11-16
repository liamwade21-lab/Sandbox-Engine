//! Replaying a measured trace through the simulation (the gate's proof, step 0.9 on).
//!
//! A trace (`trace.rs`, made by `re/elite/measure/tools/make_trace.py` from a run in the user's Elite) holds, a row a video
//! frame, the HUD's speed and the controls at that moment: the keys held (`key_w` ..), the mouse's counts, the pips and flight
//! assist. The replay flies the same controls through `flight::step` at the physics rate from the same start and gives the
//! simulated speed curve (`Ticks`: a value a tick, straight between them) and its samples at the rows. The gate (D10) is
//! `gate_curve`: a row passes if the curve comes within 1 m/s of the HUD's number somewhere inside the row's time window (2
//! video frames, or 2 % of the time since the control it answers). `gate_with` reads the samples alone (the readings before
//! task N2, kept for the report) and `gate_read` the curve as the HUD shows a number (`Readout`).
//!
//! What the replay itself assumes (each a parameter of `Replay`, measured or fitted, never tuned by feel):
//! - `latency`: the time from a key's press in the log to the game acting on it, plus the capture's start offset (capture.py
//!   logs when it launched ffmpeg, not its first frame): one number a capture chain, the rig's (`tests/traces.rs`).
//! - `throttle_rate`: how fast the throttle moves while W or S is held (share a second): the keyboard's rule, not found in
//!   the exe, measured off the HUD's throttle marker. (Any rate from 0.4 a second up gives the measured runs the same replay.)
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

/// The simulated speed curve: the speed at the first row's time (`t0`, the trace's own start) and after every tick from there,
/// tick `k` ending at `t0 + k x TICK`. Between adjacent ticks the curve is the straight line between them; outside them it is
/// not known (no extrapolation).
#[derive(Clone, Debug, PartialEq)]
pub struct Ticks {
    pub t0: f64,
    pub speed: Vec<f64>,
}

impl Ticks {
    fn time(&self, k: usize) -> f64 {
        self.t0 + k as f64 * TICK
    }

    fn end(&self) -> f64 {
        self.time(self.speed.len().saturating_sub(1))
    }

    /// The curve at `t`, `None` outside the simulated span.
    pub fn at(&self, t: f64) -> Option<f64> {
        if self.speed.is_empty() || t < self.t0 || t > self.end() {
            return None;
        }
        let x = (t - self.t0) / TICK;
        let k = (x.floor() as usize).min(self.speed.len() - 1);
        let Some(&next) = self.speed.get(k + 1) else { return Some(self.speed[k]) };
        Some(self.speed[k] + (next - self.speed[k]) * (x - k as f64))
    }

    /// The lowest and highest the curve goes over `a..b` (clipped to the simulated span), `None` if they don't meet. The curve
    /// is straight between ticks, so its extremes are at the interval's ends or at the ticks inside it.
    pub fn range(&self, a: f64, b: f64) -> Option<(f64, f64)> {
        let (a, b) = (a.max(self.t0), b.min(self.end()));
        if self.speed.is_empty() || a > b {
            return None;
        }
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut take = |v: f64| {
            lo = lo.min(v);
            hi = hi.max(v);
        };
        take(self.at(a)?);
        take(self.at(b)?);
        let first = ((a - self.t0) / TICK).ceil() as usize;
        (first..self.speed.len()).take_while(|&k| self.time(k) <= b).for_each(|k| take(self.speed[k]));
        Some((lo, hi))
    }
}

/// Simulates the trace with `block_at(eng_pips)` (the block and the full block's top speed): the speed at every row's time,
/// as the tick ending at or after it gives it (a sample a tick, the readings `gate_with` compares).
pub fn simulate(tr: &Trace, block_at: &dyn Fn(f64) -> (Block, f64), rp: &Replay) -> Option<Vec<f64>> {
    simulate_both(tr, block_at, rp).map(|(rows, _)| rows)
}

/// Simulates the trace as `simulate` does: the whole curve, a value a tick (the reading `gate_curve` compares).
pub fn simulate_ticks(tr: &Trace, block_at: &dyn Fn(f64) -> (Block, f64), rp: &Replay) -> Option<Ticks> {
    simulate_both(tr, block_at, rp).map(|(_, ticks)| ticks)
}

fn simulate_both(tr: &Trace, block_at: &dyn Fn(f64) -> (Block, f64), rp: &Replay) -> Option<(Vec<f64>, Ticks)> {
    let c = columns(tr)?;
    if tr.rows.is_empty() {
        return Some((Vec::new(), Ticks { t0: 0.0, speed: Vec::new() }));
    }
    let times: Vec<f64> = tr.rows.iter().map(|r| r[c.t]).collect();
    let mut m = Motion { vel: DVec3::new(0.0, 0.0, -tr.rows[0][c.speed]), rot: DQuat::IDENTITY, ..Motion::default() };
    let mut dist = Distributor::new(&crate::data::DISTRIBUTOR_1E);
    let mut boost = Boost::default();
    let mut throttle = 0.0f64;
    let mut last = Controls::default();
    let mut out = Vec::with_capacity(times.len());
    let mut ticks = Ticks { t0: times[0], speed: vec![m.vel.length()] };
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
        ticks.speed.push(m.vel.length());
        // Every row passed in this tick gets this tick's speed.
        while row < times.len() && times[row] <= t {
            out.push(m.vel.length());
            row += 1;
        }
    }
    Some((out, ticks))
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
    let Some(c) = columns(tr) else { return vec![no_columns()] };
    let times: Vec<f64> = tr.rows.iter().map(|r| r[c.t]).collect();
    let mut bad = Vec::new();
    for (i, r) in tr.rows.iter().enumerate() {
        let (t, hud) = (times[i], r[c.speed]);
        if (sim[i] - hud).abs() <= GATE_SPEED_MPS {
            continue;
        }
        let after = last_control_change(tr, i);
        let window = window_s(w, t, times[0], after);
        let near = times.iter().zip(sim).any(|(tt, s)| (tt - t).abs() <= window && (s - hud).abs() <= GATE_SPEED_MPS);
        if !near {
            bad.push(Miss { t, measured: hud, simulated: sim[i], after });
        }
    }
    bad
}

/// The gate against the simulated curve (the gate's reading since task N2): a row passes if the curve, straight between
/// adjacent ticks (`Ticks`), comes within 1 m/s of the HUD's speed anywhere inside the row's time window. The game's ticks and
/// ours have an arbitrary phase and the HUD can be read between them, so the samples alone (`gate_with`) step by a tick's
/// change (3 m/s in a boost) against the 1 m/s band. Nothing is smoothed and nothing is extrapolated: outside the simulated
/// span a row misses.
pub fn gate_curve(tr: &Trace, ticks: &Ticks, w: Window) -> Vec<Miss> {
    gate_read(tr, ticks, w, Readout::Exact)
}

/// How the simulated curve is read before it meets the HUD's number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readout {
    /// The curve's own value (the gate's reading).
    Exact,
    /// As the HUD shows a speed: rounded to the nearest m/s (task N2: the four steady speeds of session 2, 160.86, 100.54,
    /// 96.52 and 221.18 m/s by the records, read 161, 101, 97 and 221; cutting the fraction off would read 160, 100, 96).
    Rounded,
}

/// The gate against the curve as `readout` reads it (`gate_curve` is `Readout::Exact`).
pub fn gate_read(tr: &Trace, ticks: &Ticks, w: Window, readout: Readout) -> Vec<Miss> {
    let Some(c) = columns(tr) else { return vec![no_columns()] };
    let t_first = tr.rows.first().map_or(0.0, |r| r[c.t]);
    let read = |v: f64| match readout {
        Readout::Exact => v,
        Readout::Rounded => v.round(),
    };
    let mut bad = Vec::new();
    for (i, r) in tr.rows.iter().enumerate() {
        let (t, hud) = (r[c.t], r[c.speed]);
        let after = last_control_change(tr, i);
        let window = window_s(w, t, t_first, after);
        // The curve is continuous, so its reading takes every value between the readings of its extremes.
        let inside = ticks.range(t - window, t + window).is_some_and(|(lo, hi)| read(lo) - GATE_SPEED_MPS <= hud && hud <= read(hi) + GATE_SPEED_MPS);
        if !inside {
            bad.push(Miss { t, measured: hud, simulated: ticks.at(t).unwrap_or(f64::NAN), after });
        }
    }
    bad
}

/// A row's time window: 2 frames, or under D10 2 frames or 2 % of the time since the control change it answers (the trace's
/// start if none).
fn window_s(w: Window, t: f64, t_first: f64, after: Option<(f64, &'static str)>) -> f64 {
    match w {
        Window::Frames => 2.0 * VIDEO_FRAME_S,
        Window::D10 => gate_time_s(after.map_or(t - t_first, |(at, _)| t - at)),
    }
}

fn no_columns() -> Miss {
    Miss { t: 0.0, measured: f64::NAN, simulated: f64::NAN, after: None }
}

/// The sum of squared speed errors (a fit's measure).
pub fn error(tr: &Trace, sim: &[f64]) -> f64 {
    let Some(c) = columns(tr) else { return f64::MAX };
    tr.rows.iter().zip(sim).map(|(r, s)| (s - r[c.speed]).powi(2)).sum()
}

/// The sum of squared speed errors against the curve at each row's time (`Ticks::at`).
pub fn error_curve(tr: &Trace, ticks: &Ticks) -> f64 {
    let Some(c) = columns(tr) else { return f64::MAX };
    tr.rows.iter().map(|r| ticks.at(r[c.t]).map_or(f64::MAX, |s| (s - r[c.speed]).powi(2))).sum()
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

    /// A trace a row a video frame: W at 0.5 s, then the HUD's speed climbing at 120 m/s^2 (2 m/s a frame) for a second.
    fn climb(t: f64) -> f64 {
        120.0 * (t - 0.5).clamp(0.0, 1.0)
    }

    fn climb_trace() -> Trace {
        let mut tr = Trace::new(&["t", "speed", "key_w"]);
        for i in 0..120 {
            let t = i as f64 * VIDEO_FRAME_S;
            tr.rows.push(vec![t, climb(t), (t >= 0.5) as i32 as f64]);
        }
        tr
    }

    fn curve(f: impl Fn(f64) -> f64) -> Ticks {
        Ticks { t0: 0.0, speed: (0..120).map(|k| f(k as f64 * TICK)).collect() }
    }

    /// The curve is straight between ticks and unknown outside them.
    #[test]
    fn the_curve_is_straight_between_ticks() {
        let c = Ticks { t0: 1.0, speed: vec![0.0, 3.0, 3.0, 0.0] };
        assert!((c.at(1.0 + 0.5 * TICK).unwrap() - 1.5).abs() < 1e-9);
        assert_eq!(c.at(0.99), None);
        assert_eq!(c.at(1.0 + 3.5 * TICK), None);
        let (lo, hi) = c.range(1.0 + 0.5 * TICK, 1.0 + 1.5 * TICK).unwrap();
        assert!((lo - 1.5).abs() < 1e-9 && hi == 3.0, "{lo} {hi}");
        assert_eq!(c.range(0.0, 0.5), None);
    }

    /// The curve's reading keeps D10's band and window: a curve 3 video frames late on a 2 m/s-a-frame climb (inside the
    /// first 1.67 s after W, so the window is 2 frames) still misses; 1 frame late passes.
    #[test]
    fn a_curve_three_frames_late_fails() {
        let tr = climb_trace();
        let late3 = curve(|t| climb(t - 3.0 * VIDEO_FRAME_S));
        let miss = gate_curve(&tr, &late3, Window::D10);
        assert!(!miss.is_empty() && miss.iter().all(|m| m.after == Some((0.5, "W"))), "{miss:?}");
        assert!(gate_curve(&tr, &curve(|t| climb(t - VIDEO_FRAME_S)), Window::D10).is_empty());
    }

    /// The band is 1 m/s: a curve 0.9 m/s off on a steady speed passes, 1.1 m/s off fails.
    #[test]
    fn a_curve_off_by_more_than_the_band_fails() {
        let mut tr = Trace::new(&["t", "speed", "key_w"]);
        for i in 0..120 {
            tr.rows.push(vec![i as f64 * VIDEO_FRAME_S, 100.0, 0.0]);
        }
        assert!(gate_curve(&tr, &curve(|_| 100.9), Window::D10).is_empty());
        assert!(gate_curve(&tr, &curve(|_| 99.1), Window::D10).is_empty());
        assert_eq!(gate_curve(&tr, &curve(|_| 101.1), Window::D10).len(), 120);
        assert_eq!(gate_curve(&tr, &curve(|_| 98.9), Window::D10).len(), 120);
    }
}
