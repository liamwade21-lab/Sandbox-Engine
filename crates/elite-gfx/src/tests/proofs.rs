//! The proofs on Elite's movies (ignored: they read the UI export, `ELITE_UI_EXPORT`, and write
//! pictures and logs under `.optimize/scratch/run/ui1/`). Each movie in ARCHITECTURE.md A10's scope
//! is converted, loaded, made ready with no ActionScript error, sent real messages
//! (`messages/*.json`'s shapes; the scripts are `src/tests/scripts/`) and drawn.

use super::rig::{Rig, Run, run, script};
use crate::Out;
use elite_ui::Value;

fn assert_clean(name: &str, run: &Run) {
    assert!(run.errors.is_empty(), "{name}: errors {:#?}", run.errors);
    println!("{name}: ready after {:.3} s; warnings {:#?}; pictures {:?}", run.ready_after, run.warnings, run.shots);
}

/// Whether the movie asked to translate a text holding `token` and got `text` in the result.
fn translated(rig: &Rig, token: &str, text: &str) -> bool {
    rig.host.seen.lock().unwrap().iter().any(|(from, to)| from.contains(token) && to.contains(text) && !to.contains(token))
}

#[test]
#[ignore]
fn gauge_sheet_states() {
    let rig = Rig::new();
    let r = run(&rig, "cockpitgaugepanel", "cockpitgaugepanel", &script("gauge.json"));
    assert_clean("cockpitgaugepanel", &r);
    assert!(translated(&rig, "$GAUGE_PANEL_landing_gear;", "Landing gear"));
    assert!(translated(&rig, "$GAUGE_PANEL_mass_lock_warning;", "Mass locked"));
    assert!(translated(&rig, "$FUEL_USAGE_PER_HOUR:", "/h"));
}

#[test]
#[ignore]
fn left_panel_rows_buttons_select_and_tab() {
    let rig = Rig::new();
    let r = run(&rig, "cockpitexternalpanel", "cockpitexternalpanel", &script("left_panel.json"));
    assert_clean("cockpitexternalpanel", &r);
    let selected = r.outs.iter().any(|(_, o)| {
        matches!(o, Out::Event { name, args } if name == "LocationSelected" && args.first() == Some(&Value::Num(103.0)))
    });
    assert!(selected, "Select on the third row sends LocationSelected [103]");
    // The select sound and the location pop-up follow the game's answer to LocationSelected.
    for sound in ["showBegin", "focusChanged", "tabChanged"] {
        assert!(
            r.outs.iter().any(|(_, o)| matches!(o, Out::Sound { name, .. } if name == sound)),
            "the panel plays {sound}"
        );
    }
}

#[test]
#[ignore]
fn left_panel_timeline() {
    let rig = Rig::new();
    let r = run(&rig, "cockpitexternalpanel", "cockpitexternalpanel_timeline", &script("left_panel_timeline.json"));
    assert_clean("cockpitexternalpanel", &r);
}

#[test]
#[ignore]
fn right_panel_home_and_modules() {
    let rig = Rig::new();
    let r = run(&rig, "cockpitinternalpanel2", "cockpitinternalpanel2", &script("right_panel.json"));
    assert_clean("cockpitinternalpanel2", &r);
    assert!(r.outs.iter().any(|(_, o)| matches!(o, Out::Event { name, args } if name == "InternalOnPanel" && args.first() == Some(&Value::Num(2.0)))));
}

#[test]
#[ignore]
fn info_panel_message_and_clock() {
    let rig = Rig::new();
    let r = run(&rig, "cockpitinfopanel", "cockpitinfopanel", &script("info_panel.json"));
    assert_clean("cockpitinfopanel", &r);
    assert!(translated(&rig, "$alarm_flightassist;", "Assist off"));
}

#[test]
#[ignore]
fn role_comms_and_hud_movies() {
    let rig = Rig::new();
    for (name, file) in [
        ("rolepanel3", "role_panel.json"),
        ("cockpitchatpanel", "comms_panel.json"),
        ("orbitalhud", "orbital_hud.json"),
        ("cockpithudring", "open.json"),
        ("targetsights", "open.json"),
        ("notifications", "open.json"),
        ("fsshud", "open.json"),
        ("saahud", "open.json"),
    ] {
        let r = run(&rig, name, name, &script(file));
        assert_clean(name, &r);
    }
}

/// Main-thread time per movie frame (send + step + draw, the GPU waited for) for the gauge sheet, the
/// left panel and the info panel together, as the trial measured; written to `speed.txt`.
#[test]
#[ignore]
fn speed_per_frame() {
    use super::rig::out_dir;
    use std::time::Instant;
    let rig = Rig::new();
    let names = ["cockpitgaugepanel", "cockpitexternalpanel", "cockpitinfopanel"];
    let scripts = ["gauge.json", "left_panel.json", "info_panel.json"];
    let mut movies: Vec<_> = names.iter().map(|n| rig.load(n).0).collect();
    // Ready, then the scripts' first second (opened, data in, the show effects done).
    for (movie, file) in movies.iter_mut().zip(scripts) {
        while !movie.ready() {
            movie.step(1.0 / 30.0);
        }
        for step in script(file).iter().filter(|s| s.t < 1.0) {
            if let Some(msg) = &step.msg {
                movie.send(msg, &step.args);
            }
        }
        for _ in 0..60 {
            if movie.step(1.0 / 30.0) {
                movie.draw();
            }
        }
        movie.drain();
    }
    let frames = 600;
    let mut per: Vec<Vec<f64>> = vec![Vec::new(); movies.len()];
    let mut whole = Vec::new();
    for f in 0..frames {
        let x = f as f64 / frames as f64;
        let t0 = Instant::now();
        for (i, movie) in movies.iter_mut().enumerate() {
            let t = Instant::now();
            if i == 0 {
                movie.send("SetHeat", &[Value::from(0.2 + 0.3 * x), Value::from(1.5 / 3.5), Value::from(2.5 / 3.5)]);
                movie.send(
                    "SetThrottle",
                    &[Value::object([
                        ("speedProp", Value::from(x)),
                        ("speedString", Value::from(format!("$UNIT:#value={};", (x * 220.0) as i32))),
                        ("optimal", Value::from(0.36)),
                        ("desiredVisible", Value::from(false)),
                        ("desired", Value::from(x)),
                    ])],
                );
            }
            if movie.step(1.0 / 30.0) {
                movie.draw();
            }
            movie.drain();
            per[i].push(t.elapsed().as_secs_f64() * 1000.0);
        }
        rig.gpu.device().poll(ruffle_render_wgpu::wgpu::PollType::wait_indefinitely()).unwrap();
        whole.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        (v[v.len() / 2], v[v.len() * 95 / 100], v.iter().sum::<f64>() / v.len() as f64)
    };
    let (m, p, a) = stat(&mut whole);
    let mut report = format!("{frames} movie frames (30 fps), 3 movies, GPU waited: whole frame median {m:.3} ms, p95 {p:.3}, mean {a:.3}\n");
    for (name, v) in names.iter().zip(per.iter_mut()) {
        let (m, p, a) = stat(v);
        report += &format!("  {name} (send + step + draw): median {m:.3} ms, p95 {p:.3}, mean {a:.3}\n");
    }
    print!("{report}");
    std::fs::write(out_dir().join("speed.txt"), report).unwrap();
}
