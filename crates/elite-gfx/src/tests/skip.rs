//! The frame test (`frames.rs`) proved on Elite's movies: every movie frame the target is read back
//! after the skipping draw and again after a full draw of the same frame, and the two must be equal
//! byte for byte (two copies of a movie side by side cannot be compared: `Math.random` is seeded from
//! the clock, and the gauge sheet's signal lines are noise); the frames drawn are counted against the
//! frames whose pixels really changed. Then the cost of a
//! movie frame with and without the skip, idle and changing. Results in `ui1/skip.txt`, `ui1/skip_costs.txt`.

use super::rig::{Rig, opened, out_dir, script};
use crate::Movie;
use elite_ui::Value;
use ruffle_render_wgpu::wgpu;
use std::fmt::Write as _;
use std::time::Instant;

/// What a stretch of movie frames gave.
#[derive(Default)]
struct Tally {
    frames: usize,
    /// Frames the skipping movie drew.
    drawn: usize,
    /// Frames whose pixels changed (the full draws).
    moved: usize,
    /// Frames drawn although no pixel changed (a different list drawing the same picture).
    extra: usize,
    /// Frames where the skipping target differs from the full draw: must be none.
    wrong: usize,
}

/// One movie skipping frames that change nothing, checked every frame against a full draw.
struct Pair {
    name: &'static str,
    movie: Movie,
    target: wgpu::Texture,
    /// The full draw's pixels after the last frame.
    shown: Vec<u8>,
    log: String,
}

impl Pair {
    fn new(rig: &Rig, name: &'static str, file: &str) -> Pair {
        let (movie, target) = opened(rig, name, file);
        let shown = rig.pixels(&target);
        Pair { name, movie, target, shown, log: String::new() }
    }

    fn send(&mut self, msg: &str, args: &[Value]) {
        self.movie.send(msg, args);
    }

    /// `n` movie frames, `input` called before each (with the frame's index). Each frame is drawn
    /// with the skip and read back, then drawn again in full (skip off: the frame rendered anew and
    /// submitted) and read back: the two must be equal, and `changed` must say what `draw` did.
    fn frames(&mut self, rig: &Rig, label: &str, n: usize, input: fn(&mut Pair, usize)) -> Tally {
        let dt = 1.0 / self.movie.frame_rate();
        let mut tally = Tally::default();
        for f in 0..n {
            input(self, f);
            self.movie.step(dt);
            let said = self.movie.changed();
            assert_eq!(self.movie.changed(), said, "asking twice gives the same answer");
            let before = self.movie.draws();
            self.movie.draw();
            let drawn = self.movie.draws() - before;
            assert_eq!(drawn == 1, said, "{} {label} frame {f}: changed() says what draw() did", self.name);
            let skipped = rig.pixels(&self.target);
            self.movie.set_skipping(false);
            self.movie.draw();
            self.movie.set_skipping(true);
            assert_eq!(self.movie.draws(), before + drawn + 1, "without the skip the frame is drawn");
            let full = rig.pixels(&self.target);
            let moved = full != self.shown;
            tally.frames += 1;
            tally.drawn += drawn as usize;
            tally.moved += usize::from(moved);
            tally.extra += usize::from(drawn == 1 && !moved);
            if skipped != full {
                if tally.wrong == 0 {
                    let path = out_dir().join(format!("skip_wrong_{}_{label}.png", self.name));
                    rig.shot(&self.target, &path);
                }
                tally.wrong += 1;
            }
            self.shown = full;
            self.movie.drain();
        }
        let _ = writeln!(
            self.log,
            "  {:<22} {:>4} frames: drawn {:>4}, pixels changed {:>4}, drawn with no pixel changed {:>3}, targets differ {}",
            label, tally.frames, tally.drawn, tally.moved, tally.extra, tally.wrong
        );
        assert_eq!(tally.wrong, 0, "{} {label}: the skipping target differs from the full draw", self.name);
        tally
    }

    /// A stretch where nothing is sent: once the movie has settled nothing may be drawn.
    fn idle(&mut self, rig: &Rig, label: &str, n: usize) {
        let tally = self.frames(rig, label, n, |_, _| {});
        if tally.moved == 0 {
            assert_eq!(tally.drawn, 0, "{} {label}: an idle movie whose pixels did not change was drawn", self.name);
        }
    }
}

const HEAT_REST: [f64; 2] = [1.5 / 3.5, 2.5 / 3.5];

fn throttle(x: f64) -> Value {
    Value::object([
        ("speedProp", Value::from(x)),
        ("speedString", Value::from(format!("$UNIT:#value={};", (x * 220.0) as i32))),
        ("optimal", Value::from(0.36)),
        ("desiredVisible", Value::from(false)),
        ("desired", Value::from(x)),
    ])
}

fn button(pair: &mut Pair, which: &str, f: usize) {
    match f {
        0 => pair.send("OnInputButton", &[Value::from(which), Value::from("buttonDown")]),
        2 => pair.send("OnInputButton", &[Value::from(which), Value::from("buttonUp")]),
        _ => {}
    }
}

/// The left panel's destination data (its script's) with the first row's distance replaced.
pub(super) fn destination(distance: &str) -> Value {
    let steps = script("left_panel.json");
    let step = steps.iter().find(|s| s.msg.as_deref() == Some("SetDestinationData")).expect("destination data");
    let mut data = step.args[0].clone();
    let Value::Object(fields) = &mut data else { panic!("an object") };
    let (_, Value::Array(rows)) = fields.iter_mut().find(|(k, _)| k == "destinationList").expect("rows") else {
        panic!("a list")
    };
    let Value::Object(row) = &mut rows[0] else { panic!("a row") };
    row.iter_mut().find(|(k, _)| k == "distance").expect("a distance").1 = Value::from(distance);
    data
}

/// The destination data again with one number changed.
fn one_distance(pair: &mut Pair, f: usize) {
    if f == 0 {
        pair.send("SetDestinationData", &[destination("291.50Ls")]);
    }
}

/// A stretch after one message changing one number: exactly the frames that change are drawn.
fn exact(tally: Tally, what: &str) {
    assert!(tally.drawn > 0 && tally.extra == 0, "{what}: drawn {}, of them unchanged {}", tally.drawn, tally.extra);
}

#[test]
#[ignore = "reads the UI export (out of git) and needs a Vulkan GPU"]
fn unchanged_frames_are_not_drawn() {
    let rig = Rig::new();
    let mut report = String::new();

    let mut gauge = Pair::new(&rig, "cockpitgaugepanel", "gauge.json");
    gauge.frames(&rig, "opening", 60, |_, _| {});
    gauge.idle(&rig, "idle", 120);
    let heat = gauge.frames(&rig, "one number (heat)", 60, |p, f| {
        if f == 0 {
            p.send("SetHeat", &[Value::from(0.6), Value::from(HEAT_REST[0]), Value::from(HEAT_REST[1])]);
        }
    });
    exact(heat, "the gauge's heat");
    gauge.frames(&rig, "speed every frame", 120, |p, f| p.send("SetThrottle", &[throttle(f as f64 / 120.0)]));
    gauge.idle(&rig, "idle again", 60);

    let mut left = Pair::new(&rig, "cockpitexternalpanel", "left_panel.json");
    left.frames(&rig, "show effect", 60, |_, _| {});
    left.idle(&rig, "idle", 120);
    left.frames(&rig, "a row moving (down)", 60, |p, f| button(p, "UI_Down", f));
    left.idle(&rig, "idle again", 60);
    let distance = left.frames(&rig, "one number (distance)", 60, one_distance);
    exact(distance, "the left panel's distance");
    left.idle(&rig, "idle at the end", 60);

    let mut info = Pair::new(&rig, "cockpitinfopanel", "info_panel.json");
    info.frames(&rig, "opening + message", 90, |_, _| {});
    info.idle(&rig, "idle", 120);
    info.frames(&rig, "clock every frame", 60, |p, f| p.send("SetTime", &[Value::from(49025 + f as u32)]));

    let mut notifications = Pair::new(&rig, "notifications", "open.json");
    notifications.frames(&rig, "opening", 60, |_, _| {});
    notifications.idle(&rig, "idle (empty)", 120);

    for pair in [&gauge, &left, &info, &notifications] {
        let _ = writeln!(report, "{} ({} fps):\n{}", pair.name, pair.movie.frame_rate(), pair.log);
    }
    print!("{report}");
    std::fs::write(out_dir().join("skip.txt"), report).unwrap();
}

/// Median main-thread time per movie frame (send + step + changed + draw, the GPU waited for), idle
/// and changing, with and without the skip; and the frame test's own cost.
#[test]
#[ignore = "reads the UI export (out of git) and needs a Vulkan GPU"]
fn skip_costs() {
    type Input = fn(&mut Movie, usize);
    let rig = Rig::new();
    let cases: [(&str, &str, Input); 3] = [
        ("cockpitgaugepanel", "gauge.json", |m, f| m.send("SetThrottle", &[throttle((f % 120) as f64 / 120.0)])),
        ("cockpitexternalpanel", "left_panel.json", |m, f| {
            let which = if (f / 20) % 2 == 0 { "UI_Down" } else { "UI_Up" };
            match f % 20 {
                0 => m.send("OnInputButton", &[Value::from(which), Value::from("buttonDown")]),
                2 => m.send("OnInputButton", &[Value::from(which), Value::from("buttonUp")]),
                _ => {}
            }
        }),
        ("cockpitinfopanel", "info_panel.json", |m, f| m.send("SetTime", &[Value::from(49025 + f as u32)])),
    ];
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let mut report = String::from("per movie frame, median ms (send + step + changed + draw, GPU waited for)\n");
    for (name, file, input) in cases {
        for skipping in [true, false] {
            let (mut movie, _target) = opened(&rig, name, file);
            movie.set_skipping(skipping);
            let dt = 1.0 / movie.frame_rate();
            let frame = |movie: &mut Movie, f: Option<usize>| {
                let t = Instant::now();
                if let Some(f) = f {
                    input(movie, f);
                }
                movie.step(dt);
                if movie.changed() {
                    movie.draw();
                }
                movie.drain();
                rig.gpu.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            };
            for _ in 0..90 {
                frame(&mut movie, None);
            }
            let (draws, checks) = (movie.draws(), movie.checking());
            let idle = median((0..300).map(|_| frame(&mut movie, None)).collect());
            let idle_draws = movie.draws() - draws;
            let changing = median((0..300).map(|f| frame(&mut movie, Some(f))).collect());
            let changing_draws = movie.draws() - draws - idle_draws;
            let (spent, count) = movie.checking();
            let per_check = (spent - checks.0).as_secs_f64() * 1e6 / (count - checks.1).max(1) as f64;
            let _ = writeln!(
                report,
                "  {name:<22} skip {}: idle {idle:.3} ms ({idle_draws}/300 drawn), changing {changing:.3} ms ({changing_draws}/300 drawn){}",
                if skipping { "on " } else { "off" },
                if skipping { format!(", the test {per_check:.1} us a frame") } else { String::new() },
            );
        }
    }
    print!("{report}");
    std::fs::write(out_dir().join("skip_costs.txt"), report).unwrap();
}

/// The orders a caller may use between stepping and drawing.
#[derive(Clone, Copy, Debug)]
enum Order {
    /// `step`, `draw` (no `changed`).
    Plain,
    /// `step`, `changed`, `step`, `draw`.
    ChangedStepDraw,
    /// `step`, `changed`, `send`, `draw`.
    ChangedSendDraw,
    /// `step`, `changed`, `step`, `changed`, `draw`.
    ChangedStepChanged,
    /// `step`, `changed`, then 30 steps with neither (the pilot away), `changed`, `draw`.
    ChangedAwayChanged,
}

/// The movie with bitmap caches on screen for the call orders: the info panel draws its filters'
/// caches from its first frame, and two copies of it stay alike frame for frame (the left panel's
/// show effect, the gauge's noise lines and the other panels' caches go by the clock or by
/// `Math.random`). `RESEND` changes nothing it shows.
const CACHED: (&str, &str) = ("cockpitinfopanel", "info_panel.json");
const RESEND: (&str, i32) = ("SetHeadlightState", 1);

fn resend(movie: &mut Movie) {
    movie.send(RESEND.0, &[Value::from(RESEND.1)]);
}

impl Order {
    /// One cycle on the skipping movie; the steps it took.
    fn cycle(self, movie: &mut Movie, dt: f64) -> usize {
        movie.step(dt);
        let steps = match self {
            Order::Plain => 1,
            Order::ChangedStepDraw => {
                movie.changed();
                movie.step(dt);
                2
            }
            Order::ChangedSendDraw => {
                movie.changed();
                resend(movie);
                1
            }
            Order::ChangedStepChanged => {
                movie.changed();
                movie.step(dt);
                movie.changed();
                2
            }
            Order::ChangedAwayChanged => {
                movie.changed();
                for _ in 0..30 {
                    movie.step(dt);
                }
                movie.changed();
                31
            }
        };
        movie.draw();
        steps
    }
}

/// The movie twice: one skipping and driven in `order`, the other drawing in full every step (its
/// bitmap caches drawn as Ruffle marks them, no frame held or skipped), the same messages and
/// steps; their targets compared after each cycle. Returns the cycles whose targets differ and the
/// cache entries each drew (skipping, full).
fn order_case(rig: &Rig, order: Order, cycles: usize) -> (usize, u64, u64) {
    let (name, file) = CACHED;
    let (mut skip, skip_target) = opened(rig, name, file);
    let (mut full, full_target) = opened(rig, name, file);
    full.set_skipping(false);
    let dt = 1.0 / skip.frame_rate();
    let mut wrong = 0;
    for c in 0..cycles {
        let steps = order.cycle(&mut skip, dt);
        for _ in 0..steps {
            full.step(dt);
        }
        if matches!(order, Order::ChangedSendDraw) {
            resend(&mut full);
        }
        full.draw();
        if rig.pixels(&skip_target) != rig.pixels(&full_target) {
            if wrong == 0 {
                let out = out_dir();
                rig.shot(&skip_target, &out.join(format!("order_{name}_{order:?}_{c}_skip.png")));
                rig.shot(&full_target, &out.join(format!("order_{name}_{order:?}_{c}_full.png")));
            }
            wrong += 1;
        }
        skip.drain();
        full.drain();
    }
    (wrong, skip.caches_drawn(), full.caches_drawn())
}

/// Every order a caller may use keeps the target equal to a copy drawn in full, on a movie with
/// bitmap caches on screen: a frame rendered by `changed` and then thrown away must not lose the
/// drawing of its bitmap caches (Ruffle marks them clean when it renders). Results in
/// `ui1/skip_orders.txt`.
#[test]
#[ignore = "reads the UI export (out of git) and needs a Vulkan GPU"]
fn every_call_order_keeps_the_caches() {
    let rig = Rig::new();
    let mut report = String::new();
    let mut failed = Vec::new();
    for order in [
        Order::Plain,
        Order::ChangedStepDraw,
        Order::ChangedSendDraw,
        Order::ChangedStepChanged,
        Order::ChangedAwayChanged,
    ] {
        let (wrong, skip_caches, full_caches) = order_case(&rig, order, 60);
        let _ = writeln!(
            report,
            "  {} {order:<20?} 60 cycles: targets differ in {wrong:>2}, cache entries drawn {skip_caches} (full {full_caches})",
            CACHED.0
        );
        assert!(full_caches > 0, "{order:?}: the movie draws bitmap caches");
        if wrong > 0 {
            failed.push(format!("{order:?}"));
        }
    }
    print!("{report}");
    std::fs::write(out_dir().join("skip_orders.txt"), &report).unwrap();
    assert!(failed.is_empty(), "orders whose target differs from the full draw: {failed:?}");
}
