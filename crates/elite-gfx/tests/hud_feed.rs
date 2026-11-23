//! The flight HUD's feeders (`elite_ui::feed::Hud`, task UI4a) proved on Elite's own movies: each HUD movie is converted
//! and run as the runtime's proofs do (`src/tests/rig.rs`; that rig is private to the crate's unit tests, so the few
//! lines it takes are repeated here), a scripted flight of `UiFacts` is fed through `Hud`, every message goes to its
//! movie, and the run fails on any ActionScript error, any error line in Ruffle's log (as the runtime's own proofs: none
//! is allowed), and any message a movie has no listener for; then it checks what the movies answered (their sounds and
//! events) against the notes, so a silent regression is caught. A heading sweep of the orbital HUD's tape follows. The text host is `elite_ui::Text` itself (the real rules), on the export's string table. Pictures of
//! each movie at a few moments go to `.optimize/scratch/run/ui4a/`, with the log of what was sent and what came out.
//!
//! Ignored: it reads the UI export (`ELITE_UI_EXPORT`, default `re/elite/frea_export/sidewinder_ui/decoded`) and needs a
//! Vulkan GPU. `cargo test --profile agent -p elite-gfx --test hud_feed -- --ignored --nocapture`.

use elite_gfx::{Font, Gpu, Host, Movie, MovieFiles, Out, convert_movie, target};
use elite_ui::facts::{
    Channels, Deploy, Destination, Hardpoint, Landing, LifeAlarm, Planet, ShieldState, location_type,
};
use elite_ui::feed::{Hud, Movie as Which, Outbox};
use elite_ui::{Text, UiFacts, Value};
use ruffle_render_wgpu::backend::{create_wgpu_instance, request_adapter_and_device};
use ruffle_render_wgpu::wgpu;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The game's frame: 60 Hz, as the proofs step.
const STEP: f64 = 1.0 / 60.0;
/// The HUD movies the feeders send to.
const HUD: [Which; 6] = [
    Which::CockpitGaugePanel,
    Which::CockpitHudRing,
    Which::TargetSights,
    Which::CockpitInfoPanel,
    Which::OrbitalHud,
    Which::Notifications,
];
/// The flight's length (s), the moment the movies are loaded again and `Hud::reset` called, and the pictures.
const END_S: f64 = 23.0;
const RELOAD_S: f64 = 16.0;
const SHOTS: [(f64, &str); 8] = [
    (1.0, "parked"),
    (5.5, "liftoff_pips"),
    (7.6, "destination"),
    (9.6, "armed_new_destination"),
    (11.5, "assist_off_cleared"),
    (13.5, "charging_silent"),
    (17.0, "after_reload_cooldown"),
    (20.5, "approach_life_support"),
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn export() -> PathBuf {
    std::env::var_os("ELITE_UI_EXPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join("re/elite/frea_export/sidewinder_ui/decoded"))
}

/// The game's text host, with Elite's own text rules (`elite_ui::Text`); every translated token is kept to look at.
struct TextHost {
    text: Text,
    fonts: HashMap<String, String>,
    seen: Mutex<Vec<(String, String)>>,
}

impl Host for TextHost {
    fn translate(&self, text: &str) -> String {
        let out = self.text.translate(text);
        if text.contains('$') {
            self.seen.lock().unwrap().push((text.to_string(), out.clone()));
        }
        out
    }

    fn get_string(&self, key: &str) -> String {
        self.text.get_string(key)
    }

    fn make_untranslated_string(&self, text: &str) -> String {
        text.to_string()
    }

    fn find_in_translation_string(&self, text: &str, find: &str, from: f64) -> f64 {
        let t = self.translate(text);
        let from = from.max(0.0) as usize;
        t.get(from..).and_then(|rest| rest.find(find)).map_or(-1.0, |i| (i + from) as f64)
    }

    fn localise_number(&self, number: f64, format: &str, extra: Option<&str>) -> String {
        self.text.localise_number(number, format, extra)
    }

    fn does_text_symbol_exist(&self, key: &str) -> bool {
        self.text.contains(key)
    }

    fn get_mapped_font_name(&self, name: &str) -> String {
        self.fonts.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    fn get_database_item_data(&self, _id: u32) -> Value {
        Value::Null
    }
}

/// The font library's faces and the font map's names, as the proofs' rig makes them.
fn fonts(export: &Path) -> Vec<Font> {
    let json = |p: &str| -> serde_json::Value { serde_json::from_slice(&std::fs::read(export.join(p)).unwrap()).unwrap() };
    let mut library: Vec<(String, bool, Arc<Vec<u8>>)> = Vec::new();
    for font in json("fonts/fonts_roman.glyphs.json").as_array().unwrap() {
        let (name, bold) = (font["name"].as_str().unwrap(), font["bold"].as_bool().unwrap());
        if library.iter().any(|(n, b, _)| n == name && *b == bold) {
            continue;
        }
        let file = format!("{}{}__fonts_roman.ttf", name.replace(' ', "_"), if bold { "_Bold" } else { "" });
        library.push((name.to_string(), bold, Arc::new(std::fs::read(export.join("fonts").join(&file)).expect(&file))));
    }
    let face = |family: &str, bold: bool| {
        library
            .iter()
            .find(|(n, b, _)| n == family && *b == bold)
            .or_else(|| library.iter().find(|(n, b, _)| n.starts_with(family) && *b == bold))
            .map(|(_, _, d)| d.clone())
    };
    let mut out: Vec<Font> = library
        .iter()
        .map(|(name, bold, data)| Font { name: name.clone(), bold: *bold, italic: false, data: data.clone() })
        .collect();
    for entry in json("strings/fontmap.json")["entries"].as_array().unwrap() {
        let (name, family, bold) =
            (entry["name"].as_str().unwrap(), entry["font"].as_str().unwrap(), entry["bold"].as_bool().unwrap());
        let regular = face(family, bold).or_else(|| face(family, false)).expect("a font map face");
        let heavy = face(family, true).unwrap_or_else(|| regular.clone());
        out.push(Font { name: name.to_string(), bold: false, italic: false, data: regular });
        out.push(Font { name: name.to_string(), bold: true, italic: false, data: heavy });
    }
    out
}

struct Rig {
    gpu: Gpu,
    host: Arc<TextHost>,
    fonts: Vec<Font>,
    bridge: Vec<u8>,
    export: PathBuf,
}

impl Rig {
    fn new() -> Rig {
        let export = export();
        assert!(export.join("movies").is_dir(), "no UI export at {}", export.display());
        let backends = wgpu::Backends::VULKAN;
        let instance = create_wgpu_instance(backends, wgpu::BackendOptions::default(), None);
        let (adapter, device, queue) = futures::executor::block_on(request_adapter_and_device(
            backends,
            &instance,
            None,
            wgpu::PowerPreference::HighPerformance,
        ))
        .expect("a Vulkan device");
        let text = Text::from_json(&std::fs::read(export.join("strings/en.json")).unwrap()).expect("strings/en.json");
        let map: serde_json::Value =
            serde_json::from_slice(&std::fs::read(export.join("strings/fontmap.json")).unwrap()).unwrap();
        let fonts_map = map["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["name"].as_str().unwrap().to_string(), e["font"].as_str().unwrap().to_string()))
            .collect();
        let host = Arc::new(TextHost { text, fonts: fonts_map, seen: Mutex::new(Vec::new()) });
        let bridge = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("as3/bridge.abc")).unwrap();
        Rig { gpu: Gpu::new(instance, adapter, device, queue), host, fonts: fonts(&export), bridge, export }
    }

    /// The movie converted and loaded, stepped until it is ready, with its stage-sized target.
    fn load(&self, name: &str) -> (Movie, wgpu::Texture) {
        let source = std::fs::read(self.export.join(format!("movies/{name}/{name}.swf"))).unwrap();
        let swf = convert_movie(&source, &self.bridge).unwrap_or_else(|e| panic!("{name}: {e}"));
        let decompressed = ruffle_core::swf::decompress_swf(&swf[..]).unwrap();
        let header = decompressed.header.stage_size();
        let stage = (
            (header.x_max - header.x_min).to_pixels().round() as u32,
            (header.y_max - header.y_min).to_pixels().round() as u32,
        );
        let texture = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d { width: stage.0, height: stage.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let files = MovieFiles { name: name.to_string(), swf, fonts: self.fonts.clone(), host: self.host.clone() };
        let mut movie = Movie::load(&self.gpu, files, texture.clone()).unwrap_or_else(|e| panic!("{e}"));
        let mut t = 0.0;
        while !movie.ready() {
            assert!(t < 10.0, "{name}: not ready after 10 s");
            movie.step(STEP);
            t += STEP;
        }
        (movie, texture)
    }

    /// The texture as a PNG over dark grey (premultiplied colours over grey 24), to look at.
    fn shot(&self, texture: &wgpu::Texture, path: &Path) {
        let (w, h) = (texture.width(), texture.height());
        let padded = (w * 4).div_ceil(256) * 256;
        let device = self.gpu.device();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            texture.size(),
        );
        self.gpu.queue().submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let data = slice.get_mapped_range().expect("mapped range");
        let mut view = image::RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let at = (y * padded + x * 4) as usize;
                let p = &data[at..at + 4];
                let a = u32::from(p[3]);
                let over = |c: u8| (u32::from(c) + 24 * (255 - a) / 255).min(255) as u8;
                view.put_pixel(x, y, image::Rgb([over(p[0]), over(p[1]), over(p[2])]));
            }
        }
        drop(data);
        view.save(path).unwrap();
    }
}

/// A trace that is an ActionScript error (as the proofs' rig reads them) or a message with no listener (the bridge's
/// manager traces "no listener for message NAME").
fn is_error(text: &str) -> bool {
    text.starts_with("Break:")
        || text.contains("Error #")
        || text.contains("no listener for message")
        || ["TypeError", "ReferenceError", "ArgumentError", "RangeError", "VerifyError", "SecurityError"]
            .iter()
            .any(|e| text.contains(e))
        || text.starts_with("tween onComplete:")
}

fn laser(deploy: Deploy) -> Hardpoint {
    Hardpoint {
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
    }
}

/// The scripted flight: the facts at `t` seconds (frame `frame`).
fn facts_at(t: f64, frame: u64) -> UiFacts {
    let between = |a: f64, b: f64| t >= a && t < b;
    let ramp = |a: f64, b: f64| ((t - a) / (b - a)).clamp(0.0, 1.0);
    let mut f = UiFacts::default();
    f.status.seated = true;
    // The power plant off for a moment at 14.6 s: every HUD movie hidden, then shown again.
    f.status.powered = !between(14.6, 15.0);
    f.status.gear_down = t < 3.5 || t >= 18.0;
    f.status.cargo_scoop = false;
    // Lift-off at 2 s, throttle up to full by 5 s, back to a third by 7 s.
    let demand = if t < 2.0 { 0.0 } else if t < 5.0 { ramp(2.0, 5.0) } else { 1.0 - 0.67 * ramp(5.0, 7.0) };
    let eng_pips = if t < 5.0 { 4.0 / 8.0 } else { 6.0 / 8.0 };
    let top = 100.0 + 120.0 * eng_pips;
    let speed = if t < 2.0 { 0.0 } else { demand * top * ramp(2.0, 3.0) };
    f.speed = elite_ui::facts::Speed {
        supercruise: false,
        speed_m_s: speed,
        forward_m_s: speed,
        top_m_s: top,
        full_top_m_s: 220.0,
        engines_on: true,
        demand,
        flight_assist: !between(10.0, 12.0),
    };
    f.heat.prop = 0.4 + 0.9 * ramp(4.0, 14.0);
    let burnt = 0.0002 * t;
    f.fuel = elite_ui::facts::Fuel {
        main_t: 2.0 - burnt,
        main_capacity_t: 2.0,
        reservoir_t: 0.3,
        reservoir_capacity_t: 0.3,
        burn_t_per_s: 0.0002,
        jump_cost_t: None,
    };
    f.damage.shield.strength = 1.0;
    f.damage.shield.state = ShieldState::Active;
    f.damage.hull = 1.0;
    f.power.pips = Channels { wep: 2.0 / 8.0, eng: eng_pips, sys: 4.0 / 8.0 };
    f.power.capacitors = Channels { wep: 1.0, eng: 1.0 - 0.3 * ramp(5.0, 6.0), sys: 1.0 };
    // Each pip key once: ENG at 5.0 s, SYS, WEP and RST a third of a second apart.
    f.power.keys.eng = frame == 300;
    f.power.keys.sys = frame == 320;
    f.power.keys.wep = frame == 340;
    f.power.keys.reset = frame == 360;
    f.signature.silent = between(13.0, 14.5);
    // A destination locked at 7 s, changed at 9 s, cleared at 11 s, locked again at 17 s.
    let angle = t * 0.3;
    let dir = [angle.sin() * 0.6, 0.2, (1.0f64 - 0.36 * angle.sin().powi(2) - 0.04).sqrt()];
    if between(7.0, 9.0) {
        f.destination =
            Some(Destination { kind: location_type::PLANET, name: "Ariel".into(), distance_m: 4.2e7 - 1.0e6 * t, dir });
    } else if between(9.0, 11.0) || t >= 17.0 {
        f.destination = Some(Destination {
            kind: location_type::STAR,
            name: "Sol".into(),
            distance_m: 1.496e11,
            dir: [-dir[0], dir[1], dir[2]],
        });
    }
    // The drive charging from 12 s to 15 s, cooling down from 15 s to 18 s.
    f.drive.charge = between(12.0, 15.0).then(|| ramp(12.0, 15.0));
    f.drive.cooldown = between(15.0, 18.0).then(|| ramp(15.0, 18.0));
    // Hardpoints out at 8 s (deployed at 9 s), firing to 10.5 s, in by 11.5 s.
    let deploy = if t < 8.0 {
        Deploy::Retracted
    } else if t < 9.0 {
        Deploy::Deploying
    } else if t < 10.5 {
        Deploy::Deployed
    } else if t < 11.5 {
        Deploy::Retracting
    } else {
        Deploy::Retracted
    };
    f.weapons.armed = between(8.0, 10.5);
    f.weapons.firegroup = Some(0);
    f.weapons.hardpoints = Some(vec![laser(deploy), laser(deploy)]);
    // The life support failing from 19 s: on, critical from 21 s.
    if t >= 19.0 {
        f.alerts.oxygen_s = 31.5 - (t - 19.0) * 2.0;
        f.alerts.life_support = if f.alerts.oxygen_s <= 30.0 { LifeAlarm::Critical } else { LifeAlarm::On };
    }
    f.clock.time_of_day_s = Some(43_200.0 + t);
    // On a moon's surface, climbing to 1.5 km by 6 s and out past the planetary range by 12 s; approaching again from
    // 18 s, the landing guide on below 2 km.
    let radius = 1.2e6;
    let altitude = if t < 2.0 {
        0.0
    } else if t < 6.0 {
        1500.0 * ramp(2.0, 6.0)
    } else if t < 12.0 {
        1500.0 + 9.0e5 * ramp(6.0, 12.0)
    } else if t < 18.0 {
        2.0e6
    } else {
        2500.0 - 2400.0 * ramp(18.0, 22.0)
    };
    let landing = (altitude < 2000.0).then(|| Landing {
        roll_deg: 8.0 * (1.0 - ramp(18.0, 22.0)),
        pitch_deg: -3.0,
        axis_dot_normal: (8.0f64 * (1.0 - ramp(18.0, 22.0))).to_radians().cos(),
        speed_m_s: if t < 2.0 { 0.0 } else { 20.0 - 10.0 * ramp(18.0, 21.0) },
        speed_limit_m_s: 15.0,
        suitable_terrain: Some(true),
        landed: t < 2.0,
    });
    f.planet = Some(Planet {
        radius_m: radius,
        centre_distance_m: radius + altitude,
        inner_height_m: None,
        heading_deg: (40.0 + 10.0 * t) % 360.0,
        pitch_deg: if t < 6.0 { 10.0 * ramp(2.0, 3.0) } else { -5.0 },
        roll_deg: 5.0 * (t * 0.5).sin(),
        landing,
    });
    f
}

#[test]
#[ignore = "reads the UI export (out of git) and needs a Vulkan GPU"]
fn feeders_on_the_real_movies() {
    let out_dir = workspace().join(".optimize/scratch/run/ui4a");
    std::fs::create_dir_all(&out_dir).unwrap();
    let ruffle_path = out_dir.join("hud_feed.ruffle.log");
    let ruffle_log = Mutex::new(std::fs::File::create(&ruffle_path).unwrap());
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(ruffle_log)
        .finish();
    let rig = Rig::new();
    let mut run = Run::default();
    tracing::subscriber::with_default(subscriber, || {
        let load_all = || -> Vec<(Movie, wgpu::Texture)> { HUD.iter().map(|m| rig.load(m.name())).collect() };
        let mut movies = load_all();
        // The detector's own check: a message no movie listens for must come back as an error trace.
        movies[0].0.send("NoSuchMessageUi4a", &[]);
        movies[0].0.step(STEP);
        let caught = movies[0].0.drain().iter().any(|o| matches!(o, Out::Trace(t) if is_error(t)));
        assert!(caught, "a message without a listener is seen (the detector works)");
        let mut hud = Hud::default();
        let mut shots = SHOTS.iter().peekable();
        let frames = (END_S / STEP).round() as u64;
        let mut reloaded = false;
        for frame in 0..=frames {
            let t = frame as f64 * STEP;
            if !reloaded && t >= RELOAD_S {
                // The movies just loaded again: everything is sent anew.
                let _ = writeln!(run.log, "{t:7.3} RELOAD + Hud::reset");
                movies = load_all();
                hud.reset();
                reloaded = true;
            }
            run.frame(&mut hud, &mut movies, &facts_at(t, frame), t);
            while let Some((at, name)) = shots.peek() {
                if t + 1e-9 < *at {
                    break;
                }
                for (k, (movie, texture)) in movies.iter_mut().enumerate() {
                    movie.draw();
                    rig.shot(texture, &out_dir.join(format!("{name}_{}.png", HUD[k].name())));
                }
                shots.next();
            }
        }
        // The heading tape over the circle: near the moon, the tape held at each heading for half a second.
        let orbital = HUD.iter().position(|m| *m == Which::OrbitalHud).unwrap();
        let mut t = END_S;
        for heading in HEADINGS {
            let mut facts = facts_at(20.5, 0);
            facts.planet.as_mut().unwrap().heading_deg = heading;
            for _ in 0..30 {
                t += STEP;
                run.frame(&mut hud, &mut movies, &facts, t);
            }
            let (movie, texture) = &mut movies[orbital];
            movie.draw();
            rig.shot(texture, &out_dir.join(format!("heading_{heading}_orbitalhud.png")));
        }
    });
    let ruffle = std::fs::read_to_string(&ruffle_path).unwrap_or_default();
    for line in ruffle.lines().filter(|l| l.contains("ERROR")) {
        run.errors.push(format!("ruffle: {line}"));
    }
    let seen = rig.host.seen.lock().unwrap().clone();
    let _ = writeln!(run.log, "messages sent: {:?}", run.sent);
    for (from, to) in &seen {
        let _ = writeln!(run.log, "TRANSLATED {from:?} -> {to:?}");
    }
    std::fs::write(out_dir.join("hud_feed.log"), &run.log).unwrap();
    assert!(run.errors.is_empty(), "errors (see hud_feed.log):\n{}", run.errors.join("\n"));
    // The destination's name and translated distance reached the gauge sheet.
    let distance_shown = seen.iter().any(|(from, to)| from.starts_with("$DISTANCE:") && !to.contains('$'));
    assert!(distance_shown, "the destination's distance was translated by the movie");
    for name in [
        "SetTargetLocation",
        "ClearTargetLocation",
        "SetThrottle",
        "SetFuelLevels",
        "UpdateLandingGuide",
        "AddIndicator",
        "RemoveIndicator",
        "Hide",
        "Show",
        "IndicateEnginePowerInput",
        "IndicateSystemPowerInput",
        "IndicateWeaponPowerInput",
        "IndicateResetPowerInput",
        "SetLocalLegalStatus",
        "SetLocationPanelVisibility",
        "SetVesselStatusVisible",
    ] {
        assert!(run.sent.contains_key(name), "{name} was sent in the flight");
    }
    // What came back, against the notes (`ui_messages_hud.md`): the movies' own sounds and events for what was sent.
    let back = |movie: Which, what: &str| run.back.iter().any(|(m, w)| *m == movie && w == what);
    for (movie, what, why) in [
        (Which::CockpitGaugePanel, "sound GUI_Change_Engine_Power_Distribution", "IndicateEnginePowerInput"),
        (Which::CockpitGaugePanel, "sound GUI_Change_System_Power_Distribution", "IndicateSystemPowerInput"),
        (Which::CockpitGaugePanel, "sound GUI_Change_Weapon_Power_Distribution", "IndicateWeaponPowerInput"),
        (Which::CockpitGaugePanel, "sound GUI_Reset_Power_Distribution", "IndicateResetPowerInput"),
        (Which::CockpitInfoPanel, "sound jump_warning_end", "the charge bar turned off (HyperspaceTimer, type 0)"),
        (Which::CockpitInfoPanel, "event SetBackgroundVisible Gui_Warning_panel_background false", "Hide"),
        (Which::CockpitHudRing, "event ShowComplete", "Show with hardpoints on the arc"),
        (Which::OrbitalHud, "sound AltimeterShow", "SetGaugeVisibility: the altimeter"),
        (Which::OrbitalHud, "sound PitchGaugeShow", "SetGaugeVisibility: the ladder (second argument)"),
        (Which::OrbitalHud, "sound HeadingGaugeShow", "SetGaugeVisibility: the heading tape (third argument)"),
        (Which::OrbitalHud, "sound LandingGaugeShow", "SetGaugeVisibility: the landing guide"),
        (Which::OrbitalHud, "sound AltimeterHide", "leaving the planetary range"),
        (Which::OrbitalHud, "sound Landed", "UpdateLandingGuide with landed"),
        (Which::OrbitalHud, "sound Aligned", "UpdateLandingGuide: aligned on the approach"),
    ] {
        assert!(back(movie, what), "{} did not answer {what} ({why})", movie.name());
    }
    for (movie, what) in [(Which::OrbitalHud, "sound ScannerShow"), (Which::OrbitalHud, "sound RateOfDescentGaugeShow")] {
        assert!(!back(movie, what), "{} answered {what}: a gauge without data was shown", movie.name());
    }
    println!("pictures and log in {}", out_dir.display());
}

/// The headings of the tape's sweep (degrees).
const HEADINGS: [f64; 7] = [0.0, 50.0, 95.0, 179.0, 180.0, 245.0, 359.0];

/// What the flight gave: the log, the errors, the messages sent (by name), and what came back from each movie
/// ("sound NAME", "event NAME ARGS...").
#[derive(Default)]
struct Run {
    log: String,
    errors: Vec<String>,
    sent: HashMap<&'static str, usize>,
    back: Vec<(Which, String)>,
}

impl Run {
    /// One frame: the facts fed, every message sent to its movie, every movie stepped and its answers taken.
    fn frame(&mut self, hud: &mut Hud, movies: &mut [(Movie, wgpu::Texture)], facts: &UiFacts, t: f64) {
        let mut outbox = Outbox::default();
        hud.feed(facts, STEP, &mut outbox);
        for msg in &outbox.msgs {
            let k = HUD.iter().position(|m| *m == msg.movie).expect("a HUD movie");
            let _ = writeln!(self.log, "{t:7.3} SEND {} {} {:?}", msg.movie.name(), msg.name, msg.args);
            *self.sent.entry(msg.name).or_default() += 1;
            movies[k].0.send(msg.name, &msg.args);
        }
        for (k, (movie, _)) in movies.iter_mut().enumerate() {
            if movie.step(STEP) {
                movie.draw();
            }
            for o in movie.drain() {
                let _ = writeln!(self.log, "{t:7.3} OUT {} {o:?}", HUD[k].name());
                match &o {
                    Out::Trace(text) if is_error(text) => self.errors.push(format!("{t:.3} {}: {text}", HUD[k].name())),
                    Out::Sound { name, .. } => self.back.push((HUD[k], format!("sound {name}"))),
                    Out::Event { name, args } => {
                        let args: Vec<String> = args
                            .iter()
                            .map(|a| match a {
                                Value::Str(s) => s.clone(),
                                Value::Bool(b) => b.to_string(),
                                other => format!("{other:?}"),
                            })
                            .collect();
                        let text = if args.is_empty() { format!("event {name}") } else { format!("event {name} {}", args.join(" ")) };
                        self.back.push((HUD[k], text));
                    }
                    _ => {}
                }
            }
        }
    }
}
