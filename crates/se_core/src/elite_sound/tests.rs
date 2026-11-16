//! Tests against a real slice of the data (`tests/data/elite/sound/slice.json.z`, cut from the audio export's
//! `events.json` by `re/elite/tools/make_sound_vectors.py`) and the reference answers its own walker worked out
//! (`reference.json.z`).

use super::curve::{eval, interpolate};
use super::data::{Interp, Point, Scaling};
use super::mixer::MAX_STEP_S;
use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn data(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/elite/sound/{name}", env!("CARGO_MANIFEST_DIR"));
    let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    miniz_oxide::inflate::decompress_to_vec_zlib(&b).expect(name)
}

fn slice_bytes() -> Vec<u8> {
    data("slice.json.z")
}

fn reference() -> Value {
    serde_json::from_slice(&data("reference.json.z")).unwrap()
}

fn bank() -> Arc<SoundBank> {
    Arc::new(parse_sounds(&slice_bytes()).expect("the slice parses"))
}

fn id(v: &Value) -> Id {
    Id(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn source(u: &VoiceUpdate) -> Id {
    match u.output {
        Output::Audio { source } | Output::Rumble { source } | Output::Synth { source } => source,
    }
}

#[test]
fn slice_round_trips_with_no_value_changed() {
    let bytes = slice_bytes();
    let b = parse_sounds(&bytes).unwrap();
    let written = write_sounds(&b.file).unwrap();
    let again = parse_sounds(&written).unwrap();
    assert_eq!(again.file, b.file);
    let a: Value = serde_json::from_slice(&bytes).unwrap();
    let w: Value = serde_json::from_slice(&written).unwrap();
    assert_eq!(a, w, "write_sounds changes a value");
}

#[test]
fn damaged_files_are_errors() {
    let bytes = slice_bytes();
    assert!(parse_sounds(&bytes[..bytes.len() / 2]).is_err(), "truncated");
    assert!(parse_sounds(b"not json").is_err());
    let mut v: Value = serde_json::from_slice(&bytes).unwrap();
    v["format"] = Value::String("something else".into());
    assert!(parse_sounds(&serde_json::to_vec(&v).unwrap()).is_err(), "format");
    // An unknown field, an action kind the mixer lacks, a property it does not know, a missing parent: each refused.
    let objs = v["objects"].as_object().unwrap().clone();
    let (action, sound) = (
        objs.iter().find(|(_, o)| o["type"] == "Action").unwrap().0.clone(),
        objs.iter().find(|(_, o)| o["type"] == "Sound").unwrap().0.clone(),
    );
    let refuse = |edit: &dyn Fn(&mut Value)| {
        let mut w: Value = serde_json::from_slice(&bytes).unwrap();
        edit(&mut w);
        parse_sounds(&serde_json::to_vec(&w).unwrap()).is_err()
    };
    assert!(refuse(&|w| { w["objects"][&sound]["surprise"] = Value::Bool(true) }), "unknown field");
    assert!(refuse(&|w| { w["objects"][&action]["kind"] = Value::String("Pause".into()) }), "action kind");
    assert!(refuse(&|w| { w["objects"][&sound]["node"]["props"]["FeedbackVolume"] = serde_json::json!(1.0) }), "property");
    assert!(refuse(&|w| { w["objects"][&sound]["node"]["parent"] = serde_json::json!({"id": "deadbeef", "name": null}) }), "parent");
    assert!(refuse(&|w| { w["objects"][&sound]["node"]["rtpc"] = serde_json::json!([{"game_parameter": {"id": "00000001", "name": null},
        "rtpc_type": "GameParameter", "accumulation": "Additive", "param_id": 13, "param": "PlaybackSpeed", "curve_id": "00000002",
        "scaling": "None", "points": []}]) }), "curve on a parameter not built");
}

#[test]
fn unknown_event_is_an_error() {
    let mut m = Mixer::new(bank(), 1);
    assert!(matches!(m.post("NoSuchEvent_b", 1), Err(SoundError::UnknownEvent(_))));
    assert!(m.post("Break_ShieldGenerator", 1).is_ok());
    assert!(m.post("break_shieldgenerator", 1).is_ok(), "names are hashed lower-cased, as Wwise does");
}

/// Each sample event, nothing set: every voice it starts is one the reference walker allows, with its volume, pitch and
/// filters inside the walker's range (randomisers); over 200 seeds every allowed recording is met.
#[test]
fn events_start_what_the_reference_walker_says() {
    let r = reference();
    let b = bank();
    for (name, cands) in r["at_start"].as_object().unwrap() {
        let cands = cands.as_array().unwrap();
        let allowed: BTreeSet<Id> = cands.iter().map(|c| id(&c["source"])).collect();
        let mut met = BTreeSet::new();
        for seed in 0..200 {
            let mut m = Mixer::new(b.clone(), seed);
            m.post(name, 7).unwrap();
            for u in m.step(0.0).to_vec() {
                assert!(u.start.is_some(), "{name}: a voice at t = 0 must be starting");
                let src = source(&u);
                assert!(allowed.contains(&src), "{name}: {src} is not one the walker allows ({allowed:?})");
                let ok = cands.iter().filter(|c| id(&c["source"]) == src).any(|c| {
                    let within = |v: f32, k: &str| f(&c[k][0]) - 1e-3 <= f64::from(v) && f64::from(v) <= f(&c[k][1]) + 1e-3;
                    within(u.volume_db, "volume") && within(u.pitch_cents, "pitch") && within(u.lowpass, "lowpass") && within(u.highpass, "highpass")
                });
                assert!(ok, "{name}: {src} at {} dB, {} cents, LPF {}, HPF {} is outside the walker's ranges {:?}", u.volume_db, u.pitch_cents, u.lowpass, u.highpass,
                    cands.iter().filter(|c| id(&c["source"]) == src).collect::<Vec<_>>());
                met.insert(src);
            }
        }
        assert_eq!(met, allowed, "{name}: recordings met over 200 seeds");
    }
}

#[test]
fn curves_match_the_reference_at_every_shape() {
    let r = reference();
    let curves = r["curves"].as_array().unwrap();
    let mut shapes = BTreeSet::new();
    for c in curves {
        let pts: Vec<Point> = serde_json::from_value(c["points"].clone()).unwrap();
        let sc: Scaling = serde_json::from_value(c["scaling"].clone()).unwrap();
        for p in &pts[..pts.len() - 1] {
            shapes.insert(format!("{:?}", p.interp));
        }
        for (x, y) in c["x"].as_array().unwrap().iter().zip(c["y"].as_array().unwrap()) {
            let got = eval(&pts, sc, f(x));
            assert!((got - f(y)).abs() <= 1e-9 * (1.0 + f(y).abs()), "curve {pts:?} at {x}: {got} against {y}");
        }
    }
    // The slice covers nine shapes (InvSCurve is in shipped events outside it); every one is checked at its mid-point
    // against its closed form in curve.rs's own test, InvSCurve here once more on a made-up curve.
    assert!(shapes.len() >= 9, "{shapes:?}");
    let inv = interpolate(0.3, 2.0, 6.0, Interp::InvSCurve);
    let t: f64 = 0.3;
    let a = std::f64::consts::PI * t;
    assert!((inv - (2.0 + 4.0 * (a.sin() / 2.0))).abs() < 1e-4);
}

/// `PlayerInsideAndEmitterAttached` (game parameter, the Init bank's switch curve) chooses the interior or the exterior
/// shield break.
#[test]
fn inside_outside_switch_follows_its_game_parameter() {
    let r = reference();
    for (v, want) in r["inside_outside"].as_object().unwrap() {
        let mut m = Mixer::new(bank(), 3);
        m.set_game_parameter("PlayerInsideAndEmitterAttached", v.parse().unwrap(), None);
        m.step(1.0);
        m.post("Break_ShieldGenerator", 1).unwrap();
        let got: BTreeSet<Id> = m.step(0.0).iter().map(source).collect();
        let want: BTreeSet<Id> = want.as_array().unwrap().iter().map(id).collect();
        assert_eq!(got, want, "PlayerInsideAndEmitterAttached = {v}");
    }
}

#[test]
fn state_switch_follows_its_state() {
    let r = reference();
    let s = &r["state_switch"];
    for (state, want) in s["cases"].as_object().unwrap() {
        let mut m = Mixer::new(bank(), 3);
        m.set_state_id(id(&s["group"]), id(&Value::String(state.clone())));
        m.post("hyperdrive_count_internal", 1).unwrap();
        let got: BTreeSet<Id> = m.step(0.0).iter().map(source).collect();
        let want: BTreeSet<Id> = want.as_array().unwrap().iter().map(id).collect();
        assert_eq!(got, want, "state {state}");
    }
}

/// A switch container with continuous validation follows the switch while it plays (the inside / outside switch moved
/// after the start).
#[test]
fn continuous_switch_follows_a_change_while_playing() {
    let r = reference();
    let outside: BTreeSet<Id> = r["inside_outside"]["1.0"].as_array().unwrap().iter().map(id).collect();
    let mut m = Mixer::new(bank(), 3);
    m.post("Break_ShieldGenerator", 1).unwrap();
    let first: Vec<VoiceUpdate> = m.step(0.0).to_vec();
    assert!(!first.is_empty());
    m.set_game_parameter("PlayerInsideAndEmitterAttached", 1.0, None);
    let mut started = BTreeSet::new();
    for _ in 0..30 {
        for u in m.step(1.0 / 30.0) {
            if u.start.is_some() {
                started.insert(source(u));
            }
        }
    }
    let b = bank();
    let sw = b.file.objects.values().find_map(|o| match o {
        Object::SwitchCntr(s) if s.continuous_validation && s.group.name.as_deref() == Some("PlayerInsideAndEmitterAttached") => Some(s.id),
        _ => None,
    });
    if sw.is_some() {
        assert!(started.iter().all(|s| outside.contains(s)), "after the change only the outside sound starts: {started:?}");
    }
}

/// lockOn_b's continuous sequence: its items in playlist order, one every trigger-rate interval, for its loop count (a
/// Silence item keeps its slot and starts no voice).
#[test]
fn sequence_plays_in_order_at_its_trigger_rate() {
    let r = reference();
    let s = &r["sequence"];
    let order: Vec<BTreeSet<Id>> = s["order"].as_array().unwrap().iter().map(|x| x.as_array().unwrap().iter().map(id).collect()).collect();
    let watched: BTreeSet<Id> = order.iter().flatten().copied().collect();
    let interval = f(&s["interval_s"]);
    let mut m = Mixer::new(bank(), 5);
    m.post("lockOn_b", 1).unwrap();
    let mut starts: Vec<(f64, Id)> = Vec::new();
    for k in 0..400 {
        let ups = if k == 0 { m.step(0.0).to_vec() } else { m.step(0.01).to_vec() };
        for u in ups {
            if let Some(st) = u.start {
                if watched.contains(&source(&u)) {
                    starts.push((m.time() - f64::from(st.offset), source(&u)));
                }
            }
        }
    }
    let slots: Vec<(usize, &BTreeSet<Id>)> = order.iter().enumerate().filter(|(_, w)| !w.is_empty()).collect();
    assert_eq!(starts.len(), slots.len(), "items started: {starts:?}");
    for ((t, g), (slot, w)) in starts.iter().zip(&slots) {
        assert!(w.contains(g), "slot {slot}: {g} not in {w:?}");
        assert!((t - *slot as f64 * interval).abs() < 1e-3, "slot {slot} started at {t:.3} s, not {:.3}", *slot as f64 * interval);
    }
}

/// TargetSelectObject's shuffle container over 1,000 picks: never the same item twice running (avoid-repeat 1), each item
/// once in every pass of five (shuffle), the counts even (equal weights). A copy with weights 4:1:1:1:1 follows them.
#[test]
fn random_container_avoids_repeats_and_follows_weights() {
    let r = reference();
    let rr = &r["random"];
    let items: Vec<Id> = rr["items"].as_array().unwrap().iter().map(|x| id(&x[0])).collect();
    let cont = id(&rr["container"]);
    let avoid = rr["avoid_repeat"].as_u64().unwrap() as usize;
    let picks = |b: Arc<SoundBank>| -> Vec<Id> {
        let mut m = Mixer::new(b, 11);
        let mut out = Vec::new();
        for _ in 0..1000 {
            m.play_object(cont, 1);
            let ups = m.step(0.0).to_vec();
            out.extend(ups.iter().filter(|u| u.start.is_some()).map(source));
            m.step(5.0);
        }
        out
    };
    let p = picks(bank());
    assert_eq!(p.len(), 1000);
    for w in p.windows(avoid + 1) {
        let distinct: BTreeSet<&Id> = w.iter().collect();
        assert_eq!(distinct.len(), w.len(), "a repeat within {avoid}");
    }
    if rr["random_mode"] == "Shuffle" {
        for chunk in p.chunks(items.len()) {
            let d: BTreeSet<&Id> = chunk.iter().collect();
            assert_eq!(d.len(), chunk.len(), "a shuffle pass repeats an item");
        }
    }
    let mut count: BTreeMap<Id, usize> = BTreeMap::new();
    for x in &p {
        *count.entry(*x).or_default() += 1;
    }
    for i in &items {
        let c = count.get(i).copied().unwrap_or(0);
        assert!((150..=250).contains(&c), "{i} picked {c} times of 1000");
    }
    // Weighted, normal random mode: the heavy item about 4/8 of the picks.
    let mut file = bank().file.clone();
    if let Some(Object::RanSeqCntr(c)) = file.objects.get_mut(&cont) {
        c.using_weight = true;
        c.random_mode = data::RandomMode::Normal;
        c.avoid_repeat_count = 0;
        for (k, it) in c.playlist.iter_mut().enumerate() {
            it.weight = if k == 0 { 200_000 } else { 50_000 };
        }
    }
    let p = picks(Arc::new(SoundBank::new(file).unwrap()));
    let heavy = p.iter().filter(|x| **x == items[0]).count();
    assert!((440..=560).contains(&heavy), "the 4x item picked {heavy} times of 1000");
}

/// CargoScoopWarning_e's Stop fades CargoScoopWarning_b's container out along its curve and ends it at the fade's end.
#[test]
fn stop_fades_out_along_its_curve() {
    let r = reference();
    let s = &r["stop_fade"];
    let fade = f(&s["fade_s"]);
    let curve: Interp = serde_json::from_value(s["curve"].clone()).unwrap();
    let mut m = Mixer::new(bank(), 2);
    m.post(s["begin"].as_str().unwrap(), 1).unwrap();
    m.step(0.0);
    // (its Play fades in over 3 s: past that the voices' fade is the stop's alone)
    for _ in 0..80 {
        m.step(0.05);
    }
    let before: BTreeMap<u32, f32> = m.step(0.0).iter().map(|u| (u.voice, u.fade)).collect();
    assert!(!before.is_empty(), "the warning plays");
    m.post(s["end"].as_str().unwrap(), 1).unwrap();
    let mut t = 0.0;
    let mut stopped = BTreeMap::new();
    while t < fade + 0.2 {
        let dt = 0.05f32;
        t += f64::from(dt);
        for u in m.step(dt).to_vec() {
            assert!(u.start.is_none(), "a voice started after the stop");
            if !before.contains_key(&u.voice) {
                continue;
            }
            if u.stop {
                stopped.insert(u.voice, t);
            } else if t < fade - 1e-6 {
                let want = interpolate(t / fade, 1.0, 0.0, curve);
                let got = f64::from(u.fade / before[&u.voice]);
                assert!((got - want).abs() < 2e-3, "at {t:.2} s the stop fade is {got}, not {want}");
            }
        }
    }
    // Each voice ends at the fade's end, or earlier where its recording ends first.
    for (v, at) in &stopped {
        assert!(*at <= fade + 0.051, "voice {v} stopped at {at}, after the fade's end {fade}");
    }
    assert!(stopped.values().any(|at| (at - fade).abs() < 0.051), "no voice was still playing at the fade's end");
    let looped_on: Vec<_> = before.keys().filter(|v| !stopped.contains_key(v)).collect();
    assert!(looped_on.is_empty(), "voices that did not stop: {looped_on:?}");
}

/// ShipOrigin_b's looping drive sounds play on until ShipOrigin_e stops them.
#[test]
fn looping_sound_stops_on_its_end_event() {
    let r = reference();
    let le = &r["loop_end"];
    let looping: BTreeSet<Id> = le["looping"].as_array().unwrap().iter().map(id).collect();
    let mut m = Mixer::new(bank(), 4);
    m.post(le["begin"].as_str().unwrap(), 1).unwrap();
    let start: Vec<VoiceUpdate> = m.step(0.0).to_vec();
    let voices: BTreeMap<u32, Id> = start.iter().filter(|u| looping.contains(&source(u))).map(|u| (u.voice, source(u))).collect();
    assert_eq!(voices.len(), looping.len());
    assert!(start.iter().filter(|u| voices.contains_key(&u.voice)).all(|u| u.start.unwrap().looping && u.start.unwrap().loop_count.is_none()));
    for _ in 0..600 {
        let ups = m.step(1.0 / 60.0);
        assert!(voices.keys().all(|v| ups.iter().any(|u| u.voice == *v && !u.stop)), "a loop ended by itself");
    }
    m.post(le["end"].as_str().unwrap(), 1).unwrap();
    let ups = m.step(1.0 / 60.0);
    let stops: BTreeSet<u32> = ups.iter().filter(|u| u.stop).map(|u| u.voice).collect();
    assert!(voices.keys().all(|v| stops.contains(v)), "every loop stops at once (the Stop actions have no fade)");
    assert!(m.step(1.0 / 60.0).is_empty());
}

#[test]
fn game_parameters_ramp_by_the_init_bank() {
    let r = reference();
    for ramp in r["ramps"].as_array().unwrap() {
        let gid = id(&ramp["gp"]);
        let mut m = Mixer::new(bank(), 0);
        m.set_game_parameter_id(gid, f(&ramp["target"]) as f32, None);
        let dt = f(&ramp["dt"]) as f32;
        for (k, want) in ramp["values"].as_array().unwrap().iter().enumerate() {
            m.step(dt);
            let got = f64::from(m.game_parameter_id(gid, None));
            assert!((got - f(want)).abs() < 1e-3 * (1.0 + f(want).abs()), "{} step {k}: {got} against {want}", ramp["kind"]);
        }
    }
}

#[test]
fn state_change_moves_over_its_transition_time() {
    let r = reference();
    let s = &r["state_transition"];
    let (node, group, state) = (id(&s["node"]), id(&s["group"]), id(&s["state"]));
    let (delta, time) = (f(&s["delta"]), f(&s["time_s"]));
    let mut m = Mixer::new(bank(), 0);
    m.set_state_id(group, state);
    assert!(m.states_db(node).abs() < 1e-9);
    for k in 1..=4 {
        m.step((time / 4.0) as f32);
        let want = delta * f64::from(k) / 4.0;
        assert!((m.states_db(node) - want).abs() < 1e-3, "after {k}/4 of the transition: {} against {want}", m.states_db(node));
    }
    m.step(1.0);
    assert!((m.states_db(node) - delta).abs() < 1e-9);
}

#[test]
fn blend_track_crossfades_follow_their_game_parameter() {
    let r = reference();
    for c in r["crossfades"].as_array().unwrap() {
        let (cont, child, gp) = (id(&c["container"]), id(&c["child"]), id(&c["gp"]));
        for (x, g) in c["x"].as_array().unwrap().iter().zip(c["gain"].as_array().unwrap()) {
            let mut m = Mixer::new(bank(), 0);
            m.set_game_parameter_id(gp, f(x) as f32, Some(1));
            m.step(60.0);
            let got = m.crossfade_gain(cont, child, 1);
            assert!((got - f(g)).abs() < 1e-6, "{} at {x}: {got} against {g}", c["gp_name"]);
        }
    }
}

/// Same seed and calls, same updates; another seed may differ.
#[test]
fn deterministic_for_a_seed() {
    let run = |seed: u64| -> String {
        let mut m = Mixer::new(bank(), seed);
        let mut out = String::new();
        m.post("ShipOrigin_b_sidewinder_internal", 1).unwrap();
        m.set_emitter(2, [3.0, 0.0, 4.0]);
        for k in 0..240 {
            if k % 20 == 0 {
                m.post("TargetSelectObject", 2).unwrap();
            }
            if k == 100 {
                m.set_game_parameter("spaceship_speed_relative_self", 120.0, None);
            }
            out.push_str(&format!("{:?}\n", m.step(1.0 / 60.0)));
        }
        out
    };
    assert_eq!(run(9), run(9));
}

#[test]
fn wwise_ids_are_fnv1_of_the_lower_case_name() {
    assert_eq!(wwise_id("AlarmGForce_b"), Id(0x03e65cb7));
    assert_eq!(wwise_id("none"), Id(0x2ca33bdb));
    assert_eq!(event_id("event_00c07b96"), Id(0x00c07b96));
}

/// A copy of the slice with an edit, as a bank (or the load's refusal).
fn edited(edit: impl Fn(&mut SoundFile)) -> Result<SoundBank, SoundError> {
    let mut f = bank().file.clone();
    edit(&mut f);
    SoundBank::new(f)
}

#[test]
fn cycles_are_refused_at_load() {
    let b = bank();
    let cont = b.file.objects.iter().find_map(|(id, o)| matches!(o, Object::LayerCntr(_)).then_some(*id)).unwrap();
    let e = edited(|f| {
        if let Some(Object::LayerCntr(l)) = f.objects.get_mut(&cont) {
            l.children.push(cont);
        }
    });
    assert!(matches!(&e, Err(SoundError::Bad(m)) if m.contains(&cont.to_string())), "a container holding itself: {e:?}");
    // A longer cycle: a container below it holds it.
    let below = b
        .file
        .objects
        .iter()
        .find_map(|(id, o)| match o {
            Object::RanSeqCntr(r) if b.chain(*id).len() > 1 => Some((*id, b.chain(*id)[1], r.children.len())),
            _ => None,
        })
        .filter(|(_, p, _)| matches!(b.file.objects.get(p), Some(Object::RanSeqCntr(_) | Object::SwitchCntr(_) | Object::LayerCntr(_))));
    if let Some((kid, parent, _)) = below {
        let e = edited(|f| {
            if let Some(Object::RanSeqCntr(r)) = f.objects.get_mut(&kid) {
                r.children.push(parent);
            }
        });
        assert!(matches!(&e, Err(SoundError::Bad(m)) if m.contains(&kid.to_string()) && m.contains(&parent.to_string())), "a two-object cycle: {e:?}");
    }
    // An event posting itself without a delay; with a delay it is a loop in time, which is allowed.
    let (eid, aid) = b
        .file
        .events
        .iter()
        .find_map(|(eid, ev)| ev.actions.iter().find(|a| matches!(b.file.objects.get(a), Some(Object::Action(x)) if x.kind == data::ActionKind::PlayEvent)).map(|a| (*eid, *a)))
        .expect("the slice has a PlayEvent action");
    let selfpost = |delay: Option<i64>| {
        edited(move |f| {
            if let Some(Object::Action(a)) = f.objects.get_mut(&aid) {
                a.target = eid;
                if let Some(d) = delay {
                    a.props.insert("DelayTime".into(), data::PropValue::Int(d));
                }
            }
        })
    };
    let e = selfpost(None);
    assert!(matches!(&e, Err(SoundError::Bad(m)) if m.contains(&eid.to_string())), "an event posting itself: {e:?}");
    let ok = selfpost(Some(100)).expect("a delayed self-post loads");
    let mut m = Mixer::new(Arc::new(ok), 1);
    m.post(&format!("event_{eid}"), 1).unwrap();
    for _ in 0..50 {
        m.step(0.05);
    }
}

fn all_finite(u: &VoiceUpdate) -> bool {
    [u.gain, u.volume_db, u.fade, u.pitch, u.pitch_cents, u.lowpass, u.highpass].iter().all(|x| x.is_finite())
        && u.start.is_none_or(|s| s.offset.is_finite())
        && u.seek_to.is_none_or(f32::is_finite)
}

#[test]
fn bad_steps_and_values_change_nothing() {
    let r = reference();
    let mut m = Mixer::new(bank(), 3);
    for name in r["at_start"].as_object().unwrap().keys() {
        m.post(name, 1).unwrap();
    }
    m.step(0.1);
    let gp = "PlayerInsideAndEmitterAttached";
    m.set_game_parameter(gp, 1.0, None);
    m.set_game_parameter(gp, f32::NAN, None);
    m.set_game_parameter(gp, f32::INFINITY, Some(1));
    m.set_emitter(1, [f32::NAN, 0.0, 0.0]);
    let t0 = m.time();
    for dt in [f32::NAN, -1000.0, f32::NEG_INFINITY, -0.0, f32::INFINITY] {
        assert!(m.step(dt).iter().all(all_finite), "after step({dt})");
        assert_eq!(m.time(), t0, "step({dt}) moved time");
    }
    for _ in 0..120 {
        assert!(m.step(1.0 / 60.0).iter().all(all_finite));
        assert!(m.game_parameter(gp, None).is_finite() && m.game_parameter(gp, Some(1)).is_finite());
    }
    assert!((m.game_parameter(gp, None) - 1.0).abs() < 1e-6, "the finite value stands, the others were ignored");
}

#[test]
fn a_huge_step_is_quick_and_leaves_a_working_mixer() {
    let b = bank();
    let mut m = Mixer::new(b.clone(), 8);
    for name in b.file.ship_events.iter().chain(&b.file.open_events) {
        m.post(name, 1).unwrap();
    }
    m.step(0.0);
    let t = std::time::Instant::now();
    let ups = m.step(1e30).to_vec();
    assert!(t.elapsed().as_secs_f64() < 0.5, "step(1e30) took {:?}", t.elapsed());
    assert!(ups.iter().all(all_finite));
    assert!((m.time() - MAX_STEP_S).abs() < 1e-6, "a step counts as an hour at most");
    let before = m.time();
    for _ in 0..10 {
        assert!(m.step(1.0 / 60.0).iter().all(all_finite));
    }
    assert!(m.time() > before);
    m.post("Break_ShieldGenerator", 1).unwrap();
    assert!(m.step(0.0).iter().any(|u| u.start.is_some()), "the mixer still plays after the huge step");
}

/// A limit that uses virtual voices keeps the voice beyond it as a virtual voice; one that kills stops it.
#[test]
fn limits_kill_or_virtualise() {
    let r = reference();
    let cont = id(&r["random"]["container"]);
    let limited = |virtualise: bool| {
        let b = edited(|f| {
            if let Some(Object::RanSeqCntr(c)) = f.objects.get_mut(&cont) {
                c.node.adv.max_instances = 1;
                c.node.adv.kill_newest = false;
                c.node.adv.use_virtual_behavior = virtualise;
            }
        })
        .unwrap();
        let mut m = Mixer::new(Arc::new(b), 4);
        m.play_object(cont, 1);
        let first = m.step(0.0).to_vec();
        m.play_object(cont, 1);
        let second = m.step(0.0).to_vec();
        (first, second)
    };
    let (first, second) = limited(false);
    let old = first[0].voice;
    assert!(second.iter().any(|u| u.voice == old && u.stop), "the oldest is killed: {second:?}");
    assert!(second.iter().any(|u| u.voice != old && u.start.is_some() && !u.virtual_voice));
    let (first, second) = limited(true);
    let old = first[0].voice;
    assert!(second.iter().any(|u| u.voice == old && !u.stop && u.virtual_voice), "the oldest goes virtual: {second:?}");
    assert!(second.iter().any(|u| u.voice != old && u.start.is_some() && !u.virtual_voice));
}

/// `ignore_parent_max_instances`: what plays under such a node does not count against a limit above it.
#[test]
fn ignore_parent_takes_a_node_out_of_the_limits_above() {
    let r = reference();
    let cont = id(&r["random"]["container"]);
    let b = bank();
    let parent = b.chain(cont)[1];
    let run = |ignore: bool| {
        let b = edited(|f| {
            let n = match f.objects.get_mut(&parent) {
                Some(Object::ActorMixer(a)) => &mut a.node,
                Some(Object::SwitchCntr(s)) => &mut s.node,
                Some(Object::LayerCntr(l)) => &mut l.node,
                Some(Object::RanSeqCntr(c)) => &mut c.node,
                _ => return,
            };
            n.adv.max_instances = 1;
            n.adv.kill_newest = true;
            n.adv.use_virtual_behavior = false;
            n.adv.ignore_parent_max_instances = false;
            if let Some(Object::RanSeqCntr(c)) = f.objects.get_mut(&cont) {
                c.node.adv.ignore_parent_max_instances = ignore;
                c.node.adv.max_instances = 0;
            }
        })
        .unwrap();
        let mut m = Mixer::new(Arc::new(b), 4);
        m.play_object(cont, 1);
        m.play_object(cont, 1);
        m.step(0.0).iter().filter(|u| u.start.is_some() && !u.stop).count()
    };
    assert_eq!(run(false), 1, "the parent's limit of 1 holds");
    assert_eq!(run(true), 2, "ignoring the parent's limit, both play");
}

#[test]
fn looping_recordings_report_their_loop_region() {
    let r = reference();
    let le = &r["loop_end"];
    let src = id(&le["looping"][0]);
    let b = edited(|f| {
        if let Some(m) = f.media.get_mut(&src) {
            m.loop_region = Some([100, 1000]);
        }
    })
    .unwrap();
    let mut m = Mixer::new(Arc::new(b), 4);
    m.post(le["begin"].as_str().unwrap(), 1).unwrap();
    let ups = m.step(0.0).to_vec();
    let u = ups.iter().find(|u| source(u) == src).unwrap();
    assert_eq!(u.start.unwrap().loop_region, Some((100, 1001)), "intro once, then frames 100..1001 loop");
    assert!(ups.iter().filter(|u| source(u) != src).all(|u| u.start.unwrap().loop_region.is_none()));
}

/// A state changed in the middle of a transition starts from the value it has then.
#[test]
fn state_change_mid_transition_starts_where_it_is() {
    let r = reference();
    let s = &r["state_transition"];
    let (node, group, state) = (id(&s["node"]), id(&s["group"]), id(&s["state"]));
    let (delta, time) = (f(&s["delta"]), f(&s["time_s"]));
    let mut m = Mixer::new(bank(), 0);
    m.set_state_id(group, state);
    m.step((time / 2.0) as f32);
    let mid = m.states_db(node);
    assert!((mid - delta / 2.0).abs() < 1e-3);
    m.set_state_id(group, wwise_id("none"));
    assert!((m.states_db(node) - mid).abs() < 1e-9, "no jump at the change: {} against {mid}", m.states_db(node));
    m.step(60.0);
    assert!(m.states_db(node).abs() < 1e-9, "it ends at the new state's value");
}
