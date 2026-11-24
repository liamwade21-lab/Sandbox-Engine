//! The FSS screen's feeder (`elite_ui::feed::Fss`, task UI7) proved on Elite's own `fsshud` movie: the movie is converted
//! and run as `panels_feed.rs` runs the panels (its loading code is repeated here: the proofs' rig is private to the crate's
//! unit tests), `Fss` feeds `FssFacts` for a system of a star, a gas giant, rocky bodies and moons: the screen opened
//! without the discovery scan (INACTIVE), then with it (ACTIVE), the tuning moved across the spectrum, the screen closed.
//! The run fails on any ActionScript error, any error line in Ruffle's log and any message the movie has no listener for;
//! then it checks what came back (the movie's `UIOpened` and `UIClosed`, its sounds). The text host is `elite_ui::Text` on
//! the FULL string table (the shipped `ui/strings/en.json`). Pictures of each state go to `.optimize/scratch/run/ui7/`,
//! with the log of what was sent and what came out.
//!
//! Ignored: it reads the UI export (`ELITE_UI_EXPORT`, default `re/elite/frea_export/sidewinder_ui/decoded`) and the mod's
//! shipped `ui/` files, and needs a Vulkan GPU.
//! `cargo test --profile agent -p elite-gfx --test fss_feed -- --ignored --nocapture`.

use elite_gfx::{Font, Gpu, Host, Movie, MovieFiles, Out, convert_movie, target};
use elite_ui::feed::{Fss, Movie as Which, Outbox};
use elite_ui::screen_facts::{BodyKind, FssBody, FssFacts};
use elite_ui::text::{Bound, Chord};
use elite_ui::{Text, Value};
use ruffle_render_wgpu::backend::{create_wgpu_instance, request_adapter_and_device};
use ruffle_render_wgpu::wgpu;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The game's frame: 60 Hz, as the proofs step.
const STEP: f64 = 1.0 / 60.0;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn export() -> PathBuf {
    std::env::var_os("ELITE_UI_EXPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join("re/elite/frea_export/sidewinder_ui/decoded"))
}

fn shipped(file: &str) -> PathBuf {
    workspace().join("crates/se_engine/assets/mods/sidewinder/ui").join(file)
}

/// The game's text host, with Elite's own text rules on the full table. Every translated token is kept to look at.
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

    /// The FSS asks for no item (it has no item list).
    fn get_database_item_data(&self, _id: u32) -> Value {
        Value::Null
    }
}

/// The font library's faces and the font map's names, as `hud_feed.rs` makes them.
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
        let mut text = Text::from_json(&std::fs::read(shipped("strings/en.json")).unwrap()).expect("the full string table");
        bind_legend(&mut text);
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

/// The legend's actions' keys from the mod's shipped binds (`binds/KeyboardMouseOnly.binds`), as the game hands them to its
/// text table (`screens::bind_keys`; read here by plain text search: the proofs do not link the binds' reader).
fn bind_legend(text: &mut Text) {
    let binds = std::fs::read_to_string(workspace().join("crates/se_engine/assets/mods/sidewinder/binds/KeyboardMouseOnly.binds")).unwrap();
    let key = |block: &str, tag: &str| -> Option<Chord> {
        let at = block.find(&format!("<{tag} "))?;
        let line = &block[at..block[at..].find('>')? + at];
        let attr = |name: &str| line.split(&format!("{name}=\"")).nth(1)?.split('"').next().map(str::to_string);
        let (device, key) = (attr("Device")?, attr("Key")?);
        (device != "{NoDevice}" && !key.is_empty()).then_some(Chord { key, modifiers: Vec::new() })
    };
    for action in ["ExplorationFSSQuit", "ExplorationFSSShowHelp", "ExplorationFSSZoomIn", "ExplorationFSSZoomOut", "ExplorationFSSMiniZoomIn", "ExplorationFSSMiniZoomOut", "ExplorationFSSRadioTuningX_Raw", "ExplorationFSSDiscoveryScan", "ExplorationFSSTarget"] {
        let start = binds.find(&format!("<{action}>")).unwrap_or_else(|| panic!("{action} is not in the shipped binds"));
        let end = binds[start..].find(&format!("</{action}>")).unwrap() + start;
        let block = &binds[start..end];
        text.bind(action, Bound { primary: key(block, "Primary"), secondary: key(block, "Secondary"), axis: key(block, "Binding") });
    }
}

/// A trace that is an ActionScript error or a message with no listener (as `hud_feed.rs` reads them).
fn is_error(text: &str) -> bool {
    text.starts_with("Break:")
        || text.contains("Error #")
        || text.contains("no listener for message")
        || ["TypeError", "ReferenceError", "ArgumentError", "RangeError", "VerifyError", "SecurityError"]
            .iter()
            .any(|e| text.contains(e))
        || text.starts_with("tween onComplete:")
}

/// The `$KEY` tokens of a text (a `$KEY;` or `$KEY:...;`).
fn raw_keys(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in text.split('$').skip(1) {
        let key: String = piece.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if !key.is_empty() && matches!(piece[key.len()..].chars().next(), Some(';' | ':')) {
            out.push(key);
        }
    }
    out
}

/// A system of a star, four gas giants of one class (as the Sun's), two moons (icy and rocky), two rocky planets, an
/// Earth-like world, a water world and a metal-rich moon (Elite's planet classes: notes 7.7.2).
fn system() -> FssFacts {
    let planet = |id, class| FssBody { id, kind: BodyKind::Planet(class) };
    FssFacts {
        open: false,
        system: "Ross 128".into(),
        bodies: vec![
            FssBody { id: 1, kind: BodyKind::Star },
            planet(2, 12),
            planet(3, 6),
            planet(4, 5),
            planet(5, 5),
            planet(6, 5),
            planet(7, 0),
            planet(8, 1),
            planet(9, 3),
            planet(10, 12),
            planet(11, 12),
            planet(12, 12),
        ],
        discovery_scan_done: false,
        tuning: None,
    }
}

/// One step of the script.
#[derive(Clone, Copy, Debug)]
enum Do {
    Open,
    DiscoveryScan,
    /// The tuning set to a value.
    Tune(f64),
    /// The tuning swept from its value to this one over the seconds given.
    Sweep(f64, f64),
    Close,
    Shot(&'static str),
}

/// The script: (seconds, step).
fn script() -> Vec<(f64, Do)> {
    use Do::*;
    let mut s: Vec<(f64, Do)> = Vec::new();
    let mut t = 0.0;
    let mut at = |dt: f64, d: Do| {
        t += dt;
        s.push((t, d));
    };
    at(0.0, Open);
    at(1.5, Shot("inactive"));
    at(0.1, DiscoveryScan);
    at(1.5, Shot("active"));
    at(0.1, Tune(0.0));
    at(1.0, Shot("tuned_0"));
    at(0.1, Sweep(1.0, 4.0));
    at(4.2, Shot("tuned_1"));
    // Each class's base band (fss_signal_classes.json): a rocky body, an icy moon, the Earth-like world, the gas giant.
    at(0.1, Tune(0.425));
    at(1.0, Shot("tuned_rocky"));
    at(0.1, Tune(0.457));
    at(1.0, Shot("tuned_icy"));
    at(0.1, Tune(0.53));
    at(1.0, Shot("tuned_earthlike"));
    at(0.1, Tune(0.74));
    at(1.0, Shot("tuned_gas_giant"));
    at(0.1, Tune(0.355));
    at(1.0, Shot("tuned_gap"));
    at(0.1, Close);
    at(1.0, Shot("closed"));
    s
}

#[test]
#[ignore = "reads the UI export (out of git) and the mod's shipped ui files, and needs a Vulkan GPU"]
fn fss_on_the_real_movie() {
    let out_dir = workspace().join(".optimize/scratch/run/ui7");
    std::fs::create_dir_all(&out_dir).unwrap();
    let ruffle_path = out_dir.join("fss_feed.ruffle.log");
    let ruffle_log = Mutex::new(std::fs::File::create(&ruffle_path).unwrap());
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(ruffle_log)
        .finish();
    let rig = Rig::new();
    let mut log = String::new();
    let mut errors: Vec<String> = Vec::new();
    let mut sent: HashMap<&'static str, usize> = HashMap::new();
    let mut back: Vec<String> = Vec::new();
    tracing::subscriber::with_default(subscriber, || {
        let (mut movie, texture) = rig.load(Which::FssHud.name());
        // The detector's own check: a message the movie does not listen for must come back as an error trace.
        movie.send("NoSuchMessageUi7", &[]);
        movie.step(STEP);
        let caught = movie.drain().iter().any(|o| matches!(o, Out::Trace(t) if is_error(t)));
        assert!(caught, "a message without a listener is seen (the detector works)");
        let mut fss = Fss::default();
        let mut facts = system();
        let steps = script();
        let end = steps.last().unwrap().0 + 0.5;
        let mut next = 0;
        let mut sweep: Option<(f64, f64, f64, f64)> = None;
        let frames = (end / STEP).round() as u64;
        for frame in 0..=frames {
            let t = frame as f64 * STEP;
            while next < steps.len() && steps[next].0 <= t + 1e-9 {
                let (_, step) = steps[next];
                next += 1;
                let _ = writeln!(log, "{t:7.3} DO {step:?}");
                match step {
                    Do::Open => facts.open = true,
                    Do::DiscoveryScan => facts.discovery_scan_done = true,
                    Do::Tune(f) => {
                        facts.tuning = Some(f);
                        sweep = None;
                    }
                    Do::Sweep(to, secs) => sweep = Some((t, facts.tuning.unwrap_or(0.0), to, secs)),
                    Do::Close => facts.open = false,
                    Do::Shot(name) => {
                        movie.draw();
                        rig.shot(&texture, &out_dir.join(format!("{name}.png")));
                    }
                }
            }
            if let Some((t0, from, to, secs)) = sweep {
                let k = ((t - t0) / secs).clamp(0.0, 1.0);
                facts.tuning = Some(from + (to - from) * k);
            }
            let mut out = Outbox::default();
            fss.feed(&facts, &mut out);
            for m in &out.msgs {
                *sent.entry(m.name).or_default() += 1;
                if m.name != "SetSignalDetails" || frame % 30 == 0 {
                    let _ = writeln!(log, "{t:7.3} SEND {} {:?}", m.name, m.args);
                }
                movie.send(m.name, &m.args);
            }
            if movie.step(STEP) {
                movie.draw();
            }
            for o in movie.drain() {
                let _ = writeln!(log, "{t:7.3} OUT {o:?}");
                match &o {
                    Out::Trace(text) if is_error(text) => errors.push(format!("{t:.3} {text}")),
                    Out::Sound { name, .. } => back.push(format!("sound {name}")),
                    Out::Event { name, .. } => back.push(format!("event {name}")),
                    Out::Opened => back.push("opened".into()),
                    Out::Closed => back.push("closed".into()),
                    _ => {}
                }
            }
        }
    });
    let ruffle = std::fs::read_to_string(&ruffle_path).unwrap_or_default();
    for line in ruffle.lines().filter(|l| l.contains("ERROR")) {
        errors.push(format!("ruffle: {line}"));
    }
    let seen = rig.host.seen.lock().unwrap().clone();
    let _ = writeln!(log, "messages sent: {sent:?}");
    let _ = writeln!(log, "came back: {back:?}");
    for (from, to) in &seen {
        let _ = writeln!(log, "TRANSLATED {from:?} -> {to:?}");
    }
    std::fs::write(out_dir.join("fss_feed.log"), &log).unwrap();
    assert!(errors.is_empty(), "errors (see fss_feed.log):\n{}", errors.join("\n"));
    let raw: Vec<&(String, String)> = seen.iter().filter(|(_, to)| raw_keys(to).iter().any(|k| rig.host.text.contains(k))).collect();
    assert!(raw.is_empty(), "a raw $KEY; shown where the full table has a string: {raw:?}");
    let unformatted: Vec<&(String, String)> = seen.iter().filter(|(_, to)| to.contains("$#")).collect();
    assert!(unformatted.is_empty(), "a field left unformatted (a raw `$#value:...;`): {unformatted:?}");
    for name in [
        "Open",
        "SetSystemName",
        "SetState",
        "SetControls",
        "SetExtendedTargetInfo",
        "SetGlitchAnimationActive",
        "SetDiscoveredAndMappedBy",
        "SetScanProgress",
        "SetHistogramData",
        "SetFrequency",
        "SetSignalDataAtFrequency",
        "SetSignalDetails",
        "Close",
    ] {
        assert!(sent.contains_key(name), "{name} was sent in the run");
    }
    assert_eq!(sent.get("SetState"), Some(&2), "INACTIVE, then ACTIVE");
    assert!(back.iter().any(|b| b == "opened"), "Open: the movie's UIOpened");
    assert!(back.iter().any(|b| b == "closed"), "Close: the movie's UIClosed");
    println!("came back: {back:?}");
    println!("pictures and log in {}", out_dir.display());
}
