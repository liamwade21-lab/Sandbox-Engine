//! The proofs' rig: a headless GPU, the export's fonts and strings, a movie converted and run from a
//! script, its log and pictures under `.optimize/scratch/run/ui1/`.

use super::host::TestHost;
use crate::{Font, Gpu, Movie, MovieFiles, Out, convert_movie, target};
use elite_ui::Value;
use ruffle_render_wgpu::backend::{create_wgpu_instance, request_adapter_and_device};
use ruffle_render_wgpu::wgpu;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The game's frame: the proofs step the movies at 60 Hz (each runs its own frames at its own rate).
pub const STEP: f64 = 1.0 / 60.0;

pub fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The decoded UI export (`ELITE_UI_EXPORT`, default `re/elite/frea_export/sidewinder_ui/decoded`).
pub fn export() -> PathBuf {
    std::env::var_os("ELITE_UI_EXPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join("re/elite/frea_export/sidewinder_ui/decoded"))
}

pub fn out_dir() -> PathBuf {
    let dir = workspace().join(".optimize/scratch/run/ui1");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The font library's fonts under their own names (`fonts/fonts_roman.glyphs.json`; each one's file
/// is `fonts/<Name>[_Bold]__fonts_roman.ttf`), and the font map's names (`strings/fontmap.json`:
/// `$TitleFont` is Euro Caps...) for the face each names, its family's bold face as its bold.
pub fn fonts(export: &Path) -> Vec<Font> {
    let dir = export.join("fonts");
    let json = |p: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(export.join(p)).unwrap()).unwrap()
    };
    let mut library: Vec<(String, bool, Arc<Vec<u8>>)> = Vec::new();
    for font in json("fonts/fonts_roman.glyphs.json").as_array().unwrap() {
        let (name, bold) = (font["name"].as_str().unwrap(), font["bold"].as_bool().unwrap());
        if library.iter().any(|(n, b, _)| n == name && *b == bold) {
            continue;
        }
        let file = format!("{}{}__fonts_roman.ttf", name.replace(' ', "_"), if bold { "_Bold" } else { "" });
        library.push((name.to_string(), bold, Arc::new(std::fs::read(dir.join(&file)).expect(&file))));
    }
    // The face of `family` with that boldness: the family's own, or a "<family> ..." font of it.
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

/// A Vulkan device of the proofs' own (the game passes its own to `Gpu::new`).
pub fn headless_gpu() -> Gpu {
    let backends = wgpu::Backends::VULKAN;
    let instance = create_wgpu_instance(backends, wgpu::BackendOptions::default(), None);
    let (adapter, device, queue) = futures::executor::block_on(request_adapter_and_device(
        backends,
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
    ))
    .expect("a Vulkan device");
    Gpu::new(instance, adapter, device, queue)
}

/// A stage-sized target texture for a movie.
pub fn target(gpu: &Gpu, label: &str, (width, height): (u32, u32)) -> wgpu::Texture {
    gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: target::FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

pub struct Rig {
    pub gpu: Gpu,
    pub host: Arc<TestHost>,
    fonts: Vec<Font>,
    bridge: Vec<u8>,
    export: PathBuf,
}

impl Rig {
    pub fn new() -> Rig {
        let export = export();
        assert!(export.join("movies").is_dir(), "no UI export at {}", export.display());
        let read = |p: &str| std::fs::read_to_string(export.join(p)).unwrap();
        let host = Arc::new(TestHost::new(&read("strings/en.json"), &read("strings/fontmap.json")));
        let bridge = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("as3/bridge.abc")).unwrap();
        Rig { gpu: headless_gpu(), host, fonts: fonts(&export), bridge, export }
    }

    /// The movie converted, loaded, with its target texture (stage-sized).
    pub fn load(&self, name: &str) -> (Movie, wgpu::Texture) {
        let source = std::fs::read(self.export.join(format!("movies/{name}/{name}.swf"))).unwrap();
        let swf = convert_movie(&source, &self.bridge).unwrap_or_else(|e| panic!("{name}: {e}"));
        let stage = stage_size(&swf);
        let texture = target(&self.gpu, name, stage);
        let files = MovieFiles { name: name.to_string(), swf, fonts: self.fonts.clone(), host: self.host.clone() };
        let movie = Movie::load(&self.gpu, files, texture.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(movie.stage(), stage);
        (movie, texture)
    }

    /// The texture's pixels read back (premultiplied RGBA, rows packed), the GPU waited for.
    pub fn pixels(&self, texture: &wgpu::Texture) -> Vec<u8> {
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
        let row = (w * 4) as usize;
        data.chunks(padded as usize).flat_map(|line| &line[..row]).copied().collect()
    }

    /// Writes the texture as a PNG (straight alpha) and a `.view.png` over dark grey to look at.
    pub fn shot(&self, texture: &wgpu::Texture, path: &Path) {
        let (w, h) = (texture.width(), texture.height());
        let data = self.pixels(texture);
        let mut straight = image::RgbaImage::new(w, h);
        let mut view = image::RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let at = ((y * w + x) * 4) as usize;
                let p = &data[at..at + 4];
                let a = u32::from(p[3]);
                let un = |c: u8| ((u32::from(c) * 255 + a / 2).checked_div(a).unwrap_or(0)).min(255) as u8;
                straight.put_pixel(x, y, image::Rgba([un(p[0]), un(p[1]), un(p[2]), p[3]]));
                // Premultiplied over grey 24.
                let over = |c: u8| (u32::from(c) + 24 * (255 - a) / 255).min(255) as u8;
                view.put_pixel(x, y, image::Rgb([over(p[0]), over(p[1]), over(p[2])]));
            }
        }
        straight.save(path).unwrap();
        view.save(path.with_extension("view.png")).unwrap();
    }
}

/// The stage size of a (converted, uncompressed) SWF in pixels.
fn stage_size(swf: &[u8]) -> (u32, u32) {
    let buf = ruffle_core::swf::decompress_swf(swf).unwrap();
    let r = buf.header.stage_size();
    ((r.x_max - r.x_min).to_pixels().round() as u32, (r.y_max - r.y_min).to_pixels().round() as u32)
}

/// One step of a script, at `t` seconds after the movie said it is ready.
#[derive(serde::Deserialize)]
pub struct Step {
    pub t: f64,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub args: Vec<Value>,
    /// A DataStore write: [context..., property, value] as `store` takes them.
    #[serde(default)]
    pub store: Option<(Vec<String>, String, Value)>,
    /// A picture named so.
    #[serde(default)]
    pub shot: Option<String>,
}

/// What a run gave.
pub struct Run {
    pub outs: Vec<(f64, Out)>,
    /// Lines of the log that are ActionScript errors (uncaught, Break) or player errors.
    pub errors: Vec<String>,
    /// Ruffle's warnings, counted by text.
    pub warnings: Vec<String>,
    pub shots: Vec<PathBuf>,
    pub ready_after: f64,
}

/// Loads and runs movie `name` with `steps`, writing `<prefix>.log` (what was sent and what came
/// out), `<prefix>.ruffle.log` (Ruffle's warnings) and the pictures `<prefix>_<shot>.png`.
pub fn run(rig: &Rig, name: &str, prefix: &str, steps: &[Step]) -> Run {
    let out = out_dir();
    let warnings_path = out.join(format!("{prefix}.ruffle.log"));
    let ruffle_log = Mutex::new(std::fs::File::create(&warnings_path).unwrap());
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(ruffle_log)
        .finish();
    let mut log = String::new();
    let mut run = Run { outs: Vec::new(), errors: Vec::new(), warnings: Vec::new(), shots: Vec::new(), ready_after: f64::NAN };
    tracing::subscriber::with_default(subscriber, || {
        let (mut movie, texture) = rig.load(name);
        let _ = writeln!(log, "[{name}] stage {:?}", movie.stage());
        let mut t = 0.0;
        let collect = |movie: &mut Movie, t: f64, log: &mut String, run: &mut Run| {
            for o in movie.drain() {
                let _ = writeln!(log, "{t:7.3} OUT {o:?}");
                run.outs.push((t, o));
            }
        };
        while !movie.ready() {
            if t >= 10.0 {
                let _ = writeln!(log, "{t:7.3} NOT READY");
                return;
            }
            movie.step(STEP);
            t += STEP;
            collect(&mut movie, t, &mut log, &mut run);
        }
        run.ready_after = t;
        let _ = writeln!(log, "{t:7.3} READY");
        let end = steps.iter().map(|s| s.t).fold(0.0, f64::max) + STEP;
        let mut clock = 0.0;
        let mut next = 0;
        while clock <= end {
            while next < steps.len() && steps[next].t <= clock + 1e-9 {
                let step = &steps[next];
                if let Some(msg) = &step.msg {
                    let _ = writeln!(log, "{clock:7.3} SEND {msg} {:?}", step.args);
                    movie.send(msg, &step.args);
                }
                if let Some((context, property, value)) = &step.store {
                    let _ = writeln!(log, "{clock:7.3} STORE {context:?}.{property} = {value:?}");
                    let context: Vec<&str> = context.iter().map(String::as_str).collect();
                    movie.store(&context, property, value.clone());
                }
                if let Some(shot) = &step.shot {
                    movie.draw();
                    let path = out.join(format!("{prefix}_{shot}.png"));
                    rig.shot(&texture, &path);
                    let _ = writeln!(log, "{clock:7.3} SHOT {}", path.display());
                    run.shots.push(path);
                }
                next += 1;
            }
            if movie.step(STEP) {
                movie.draw();
            }
            clock += STEP;
            collect(&mut movie, clock, &mut log, &mut run);
        }
    });
    for (_, o) in &run.outs {
        if let Out::Trace(text) = o
            && is_error(text)
        {
            run.errors.push(text.clone());
        }
    }
    let ruffle = std::fs::read_to_string(&warnings_path).unwrap_or_default();
    let mut counted: std::collections::BTreeMap<String, usize> = Default::default();
    for line in ruffle.lines().filter(|l| !l.trim().is_empty()) {
        if line.contains("ERROR") {
            run.errors.push(line.to_string());
        }
        *counted.entry(line.trim().to_string()).or_default() += 1;
    }
    run.warnings = counted.iter().map(|(l, n)| format!("{n} x {l}")).collect();
    let missing = rig.host.missing.lock().unwrap().clone();
    let _ = writeln!(log, "missing string keys so far: {missing:?}");
    let _ = writeln!(log, "ruffle warnings: {:#?}", run.warnings);
    std::fs::File::create(out.join(format!("{prefix}.log"))).unwrap().write_all(log.as_bytes()).unwrap();
    assert!(run.ready_after.is_finite(), "{name}: not ready after 10 s (see {prefix}.log)");
    run
}

/// An ActionScript error as the player traces it (uncaught errors in debug mode), or a Break.
fn is_error(text: &str) -> bool {
    text.starts_with("Break:")
        || text.contains("Error #")
        || ["TypeError", "ReferenceError", "ArgumentError", "RangeError", "VerifyError", "SecurityError"]
            .iter()
            .any(|e| text.contains(e))
        || text.starts_with("tween onComplete:")
}

/// Reads a script (JSON: an array of steps) from `src/tests/scripts/`.
pub fn script(file: &str) -> Vec<Step> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/scripts").join(file);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("{file}: {e}"))
}

/// A movie made ready and sent its script's first second (opened, its data in), not stepped further.
pub fn opened(rig: &Rig, name: &str, file: &str) -> (Movie, wgpu::Texture) {
    let (mut movie, texture) = rig.load(name);
    while !movie.ready() {
        movie.step(1.0 / movie.frame_rate());
    }
    for step in script(file).iter().filter(|s| s.t < 1.0) {
        if let Some(msg) = &step.msg {
            movie.send(msg, &step.args);
        }
    }
    movie.drain();
    (movie, texture)
}
