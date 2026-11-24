//! The GPU objects a movie keeps alive must stay flat while it runs: each movie alone runs 10,000 of
//! its frames with its values changing (the HUD movies fed by `elite_ui::feed::Hud` from a generated
//! flight, the left panel by its buttons and its destination data), drawn whenever `changed` says
//! so, and wgpu's own registry (`Instance::generate_report`: the objects whose handles are alive) is
//! read every 1,000 frames, the GPU waited for every frame as a game frame does. Results in
//! `ui1/objects.txt`.

use super::rig::{Rig, opened};
use super::skip::destination;
use crate::Movie;
use elite_ui::facts::{
    Channels, Deploy, Destination, Hardpoint, Landing, LifeAlarm, Planet, ShieldState, Speed, location_type,
};
use elite_ui::feed::{Hud, Movie as Which, Outbox};
use elite_ui::{UiFacts, Value};
use ruffle_render_wgpu::wgpu;
use std::f64::consts::TAU;
use std::fmt::Write as _;

const FRAMES: u64 = 10_000;
const EVERY: u64 = 1_000;

/// The objects alive: buffers, textures, texture views, bind groups, samplers.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Counts([usize; 5]);

impl Counts {
    fn now(rig: &Rig) -> Counts {
        rig.gpu.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let hub = rig.gpu.descriptors.wgpu_instance.generate_report().expect("wgpu's core backend").hub;
        Counts([
            hub.buffers.num_allocated,
            hub.textures.num_allocated,
            hub.texture_views.num_allocated,
            hub.bind_groups.num_allocated,
            hub.samplers.num_allocated,
        ])
    }

    fn line(&self) -> String {
        let [b, t, v, g, s] = self.0;
        format!("buffers {b:>4}, textures {t:>3}, views {v:>3}, bind groups {g:>4}, samplers {s:>2}")
    }
}

/// A flight that keeps every HUD value moving: speed, heat, fuel, pips, the gear, silent running,
/// a destination coming and going and turning, the hardpoints going out and in, the drive charging,
/// the life support failing, the clock, a moon's surface coming and going, the heading turning.
fn flight(frame: u64) -> UiFacts {
    let t = frame as f64 / 30.0;
    let wave = |period: f64| 0.5 - 0.5 * (t * TAU / period).cos();
    let mut f = UiFacts::default();
    f.status.seated = true;
    f.status.powered = true;
    f.status.gear_down = (frame / 600).is_multiple_of(2);
    let demand = wave(20.0);
    f.speed = Speed {
        supercruise: false,
        speed_m_s: demand * 220.0,
        forward_m_s: demand * 220.0,
        top_m_s: 220.0,
        full_top_m_s: 220.0,
        engines_on: true,
        demand,
        flight_assist: (frame / 900) % 3 != 2,
    };
    f.heat.prop = 0.3 + 0.8 * wave(37.0);
    f.fuel.main_t = 2.0 - 1.5 * wave(300.0);
    f.fuel.main_capacity_t = 2.0;
    f.fuel.reservoir_t = 0.3 * (1.0 - wave(11.0) * 0.5);
    f.fuel.reservoir_capacity_t = 0.3;
    f.fuel.burn_t_per_s = 0.0002;
    f.damage.shield.strength = 1.0 - 0.6 * wave(53.0);
    f.damage.shield.state = ShieldState::Active;
    f.damage.hull = 1.0 - 0.3 * wave(71.0);
    let eng = if (frame / 450).is_multiple_of(2) { 4.0 / 8.0 } else { 6.0 / 8.0 };
    f.power.pips = Channels { wep: 2.0 / 8.0, eng, sys: 6.0 / 8.0 - eng + 2.0 / 8.0 };
    f.power.capacitors = Channels { wep: 1.0 - 0.5 * wave(7.0), eng: 1.0 - 0.3 * wave(5.0), sys: 1.0 };
    f.power.keys.eng = frame.is_multiple_of(450);
    f.power.keys.sys = frame % 450 == 150;
    f.power.keys.reset = frame % 450 == 300;
    f.signature.silent = (frame / 750) % 4 == 3;
    if !(frame / 300).is_multiple_of(3) {
        let angle = t * 0.3;
        let dir = [angle.sin() * 0.6, 0.2, (1.0f64 - 0.36 * angle.sin().powi(2) - 0.04).sqrt()];
        f.destination =
            Some(Destination { kind: location_type::PLANET, name: "Ariel".into(), distance_m: 4.2e7 - 1.0e4 * t, dir });
    }
    let cycle = frame % 900;
    let deploy = match cycle {
        0..300 => Deploy::Retracted,
        300..330 => Deploy::Deploying,
        330..600 => Deploy::Deployed,
        600..630 => Deploy::Retracting,
        _ => Deploy::Retracted,
    };
    let laser = Hardpoint {
        module_item_id: 0,
        name: "Pulse Laser".into(),
        weapon_type: 0,
        weapon_mode: 1,
        deploy_s: 1.0,
        retract_s: 1.0,
        deploy,
        module_on: true,
        powered: true,
        trigger: 1,
    };
    f.weapons.armed = (330..600).contains(&cycle);
    f.weapons.firegroup = Some(0);
    f.weapons.hardpoints = Some(vec![laser.clone(), laser]);
    f.drive.charge = (600..690).contains(&cycle).then(|| (cycle - 600) as f64 / 90.0);
    f.drive.cooldown = (690..780).contains(&cycle).then(|| (cycle - 690) as f64 / 90.0);
    if (frame / 1200) % 2 == 1 {
        f.alerts.oxygen_s = 300.0 - (frame % 1200) as f64 / 4.0;
        f.alerts.life_support = if f.alerts.oxygen_s <= 230.0 { LifeAlarm::Critical } else { LifeAlarm::On };
    }
    f.clock.time_of_day_s = Some(43_200.0 + t);
    let radius = 1.2e6;
    let altitude = 3000.0 * (1.0 - wave(120.0)) + 9.0e5 * wave(400.0);
    let landing = (altitude < 2000.0).then(|| Landing {
        roll_deg: 8.0 * (t * 0.7).sin(),
        pitch_deg: -3.0,
        axis_dot_normal: (8.0 * (t * 0.7).sin()).to_radians().cos(),
        speed_m_s: 20.0 * wave(9.0),
        speed_limit_m_s: 15.0,
        suitable_terrain: Some(true),
        landed: false,
    });
    f.planet = Some(Planet {
        radius_m: radius,
        centre_distance_m: radius + altitude,
        inner_height_m: None,
        heading_deg: (40.0 + 10.0 * t) % 360.0,
        pitch_deg: 10.0 * (t * 0.2).sin(),
        roll_deg: 5.0 * (t * 0.5).sin(),
        landing,
    });
    f
}

/// Runs `movie` for `FRAMES` frames, `input` before each; the counts every `EVERY` frames.
fn run(rig: &Rig, movie: &mut Movie, mut input: impl FnMut(&mut Movie, u64)) -> (Vec<Counts>, u64) {
    let dt = 1.0 / movie.frame_rate();
    let draws = movie.draws();
    let mut counts = vec![Counts::now(rig)];
    for frame in 1..=FRAMES {
        input(movie, frame);
        movie.step(dt);
        if movie.changed() {
            movie.draw();
        }
        movie.drain();
        // A game frame waits for the GPU (its present does); without it the CPU runs frames ahead and
        // Ruffle's staging belt makes a chunk for each frame still in flight.
        rig.gpu.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
        if frame % EVERY == 0 {
            counts.push(Counts::now(rig));
        }
    }
    (counts, movie.draws() - draws)
}

#[test]
#[ignore = "reads the UI export (out of git) and needs a Vulkan GPU"]
fn gpu_objects_stay_flat() {
    let rig = Rig::new();
    let mut report = format!("objects alive (wgpu's registry), each movie alone, {FRAMES} movie frames, every {EVERY}\n");
    let empty = Counts::now(&rig);
    let _ = writeln!(report, "the device alone: {}", empty.line());
    let hud = [
        Which::CockpitGaugePanel,
        Which::CockpitHudRing,
        Which::TargetSights,
        Which::CockpitInfoPanel,
        Which::OrbitalHud,
        Which::Notifications,
    ];
    let mut failures = Vec::new();
    for which in hud.iter().copied().map(Some).chain([None]) {
        let (name, (mut movie, target)) = match which {
            Some(which) => {
                let (mut movie, target) = rig.load(which.name());
                while !movie.ready() {
                    movie.step(1.0 / movie.frame_rate());
                }
                (which.name(), (movie, target))
            }
            None => ("cockpitexternalpanel", opened(&rig, "cockpitexternalpanel", "left_panel.json")),
        };
        let mut sent = std::collections::BTreeMap::new();
        let (counts, draws) = match which {
            Some(which) => {
                let mut feed = Hud::default();
                run(&rig, &mut movie, |movie, frame| {
                    let mut outbox = Outbox::default();
                    feed.feed(&flight(frame), 1.0 / 30.0, &mut outbox);
                    for msg in outbox.msgs.iter().filter(|m| m.movie == which) {
                        *sent.entry(msg.name).or_insert(0usize) += 1;
                        movie.send(msg.name, &msg.args);
                    }
                })
            }
            None => run(&rig, &mut movie, |movie, frame| {
                let press = |movie: &mut Movie, which: &str, state: &str| {
                    movie.send("OnInputButton", &[Value::from(which), Value::from(state)]);
                };
                let which = if (frame / 40) % 2 == 0 { "UI_Down" } else { "UI_Up" };
                match frame % 20 {
                    0 => press(movie, which, "buttonDown"),
                    2 => press(movie, which, "buttonUp"),
                    10 => movie.send("SetDestinationData", &[destination(&format!("{:.2}Ls", frame as f64 / 7.0))]),
                    _ => {}
                }
            }),
        };
        drop((movie, target));
        let after = Counts::now(&rig);
        let fed = if sent.is_empty() { String::from("buttons and destination data") } else { format!("{sent:?}") };
        let _ = writeln!(report, "{name} ({draws} of {FRAMES} frames drawn; sent {fed}):");
        for (k, c) in counts.iter().enumerate() {
            let _ = writeln!(report, "  {:>6} {}", k as u64 * EVERY, c.line());
        }
        let _ = writeln!(report, "  dropped {}", after.line());
        // Flat: nothing in the second half above the most seen by its middle.
        let half = counts.len() / 2;
        for i in 0..5 {
            let by_middle = counts[..=half].iter().map(|c| c.0[i]).max().unwrap();
            let later = counts[half..].iter().map(|c| c.0[i]).max().unwrap();
            if later > by_middle {
                failures.push(format!("{name}: kind {i} grew from {by_middle} to {later} after frame {}", half as u64 * EVERY));
            }
        }
        if after != empty {
            failures.push(format!("{name}: dropping the movie left {} (the device alone {})", after.line(), empty.line()));
        }
    }
    print!("{report}");
    std::fs::write(super::rig::out_dir().join("objects.txt"), &report).unwrap();
    assert!(failures.is_empty(), "{failures:#?}");
}
