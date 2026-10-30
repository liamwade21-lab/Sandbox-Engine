//! SE's screenshots (F11): main-def.cfg `Export` `ScreenshotFormat` (jpg, png, dds, tga, tif; "Screenshots
//! format"), `JpegQuality` (95) and `SaveAlpha` ("Save transparency", png, dds, tga and tif only), saved as
//! `scr%05i.<ext>` ("%sscr%05i.%S") numbered after the last one in the folder. SE writes into its
//! `screenshots/` (`UserScreenshotsSubDir`); the port never writes into the install and keeps
//! `<Pictures>/SpaceEngine-rs/`. JPEG and PNG use the `image` crate; TGA, DDS and TIFF are written
//! uncompressed here, in 8 bits, or (for the skybox export's "Pixel format") 16 bits (PNG, DDS
//! A16B16G16R16, TIFF) and 32-bit float (DDS A32B32G32R32F, TIFF).
//!
//! SE's high-resolution screenshot tool (seScreenshotTool, HiresScreenshot Shift+F11): the view
//! rendered in tiles of `ScreenshotTileRes` (1024, 512..1024 in steps of 16) into one image of the
//! "Target resolution" (8000 x 4000, at most 65535 x 65535), with time paused, the landscape at
//! `ScreenshotLandLOD`, "Scale stars" and "Wait loading completeness"; saved as `mosaic%05i.<ext>` in
//! `screenshots-hd/` (FUN_140221d70: "Starting high-resolution screenshot rendering: %i x %i, %i x %i
//! tiles of %i x %i"). The tiles are `capture.rs` views of the main camera with a sub-view of the full
//! frame each.

use std::io::Write;
use std::path::{Path, PathBuf};

use bevy::camera::SubCameraView;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, egui};

use crate::capture::{CaptureView, Grabs, Post};

/// SE's formats, in its order.
pub const FORMATS: [&str; 5] = ["jpg", "png", "dds", "tga", "tif"];

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ScreenshotSettings {
    /// One of [`FORMATS`].
    pub format: String,
    pub jpeg_quality: u8,
    pub save_alpha: bool,
}

impl Default for ScreenshotSettings {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self { format: "jpg".into(), jpeg_quality: 95, save_alpha: false }
    }
}

pub struct ScreenshotPlugin;

impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScreenshotSettings>()
            .init_resource::<ScreenshotNumber>()
            .init_resource::<HiresSettings>()
            .init_resource::<HiresRequest>()
            .init_resource::<HiresUi>()
            .init_resource::<HiresState>()
            .add_systems(Startup, load_settings)
            .add_systems(Update, (hires_key, run_hires).chain())
            .add_systems(bevy_egui::EguiPrimaryContextPass, hires_window.in_set(crate::skin::UiSet::Draw));
    }
}

fn load_settings(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<ScreenshotSettings>, mut h: ResMut<HiresSettings>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    if let Some(f) = def.find_deep("ScreenshotFormat").and_then(|i| i.str()).filter(|f| FORMATS.contains(&f.to_ascii_lowercase().as_str())) {
        s.format = f.to_ascii_lowercase();
    }
    if let Some(q) = def.find_deep("JpegQuality").and_then(|i| i.f64()) {
        s.jpeg_quality = q.clamp(1.0, 100.0) as u8;
    }
    if let Some(a) = def.find_deep("SaveAlpha").and_then(|i| i.bool()) {
        s.save_alpha = a;
    }
    // Registered by FUN_140224450 (not in main-def.cfg; read if it is there).
    if let Some(t) = def.find_deep("ScreenshotTileRes").and_then(|i| i.f64()) {
        h.tile_res = clamp_tile_res(t as u32);
    }
    if let Some(l) = def.find_deep("ScreenshotLandLOD").and_then(|i| i.f64()) {
        h.land_lod = l as f32;
    }
}

/// The screenshots folder, `<Pictures>/SpaceEngine-rs`.
pub fn folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Pictures").join("SpaceEngine-rs"))
        .unwrap_or_else(|| PathBuf::from("screenshots"))
}

/// The number after the highest `<prefix>NNNNN.*` in `names`.
fn next_number<'a>(prefix: &str, names: impl Iterator<Item = &'a str>) -> u32 {
    names
        .filter_map(|n| n.strip_prefix(prefix).and_then(|r| r.split('.').next()).filter(|d| d.len() == 5).and_then(|d| d.parse::<u32>().ok()))
        .max()
        .map_or(0, |m| m + 1)
}

fn scan_number(prefix: &str, dir: &Path) -> u32 {
    let names: Vec<String> = std::fs::read_dir(dir)
        .map(|it| it.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    next_number(prefix, names.iter().map(String::as_str))
}

/// The number of the next screenshot: the folder is scanned once, then counted up per press, so two
/// presses before the first capture arrives still get two numbers.
#[derive(Resource, Default)]
pub struct ScreenshotNumber(Option<u32>);

impl ScreenshotNumber {
    fn next(&mut self, dir: &Path) -> u32 {
        let n = *self.0.get_or_insert_with(|| scan_number("scr", dir));
        self.0 = Some(n + 1);
        n
    }
}

/// Takes a screenshot of the window and saves it in SE's format (F11).
pub fn take(commands: &mut Commands, settings: &ScreenshotSettings, number: &mut ScreenshotNumber) {
    let dir = folder();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("scr{:05}.{}", number.next(&dir), settings.format));
    let s = settings.clone();
    info!("screenshot -> {}", path.display());
    commands.spawn(Screenshot::primary_window()).observe(move |ev: On<ScreenshotCaptured>| {
        let Ok(img) = ev.image.clone().try_into_dynamic() else {
            warn!("screenshot: unsupported capture format");
            return;
        };
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        if let Err(e) = encode(&path, &s.format, s.jpeg_quality, &Picture::rgba8(w, h, rgba.into_raw(), s.save_alpha)) {
            warn!("cannot save screenshot {}: {e}", path.display());
        }
    });
}

// ---------------------------------------------------------------------------
// Pictures and writers

/// SE's "Pixel format" choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Depth {
    U8,
    U16,
    F32,
}

impl Depth {
    pub const NAMES: [&'static str; 3] = ["8-bit", "16-bit", "32-bit (floating-point)"];
    pub const ALL: [Depth; 3] = [Depth::U8, Depth::U16, Depth::F32];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// The deepest of `want` a format can hold: JPEG and TGA 8 bits, PNG 16, DDS and TIFF also float.
pub fn best_depth(format: &str, want: Depth) -> Depth {
    match format {
        "jpg" | "tga" => Depth::U8,
        "png" => want.min(Depth::U16),
        _ => want,
    }
}

/// Samples, RGB or RGBA per pixel, top row first. Integer samples are display (sRGB) values, float
/// samples linear.
#[derive(Clone, Debug, PartialEq)]
pub enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub alpha: bool,
    pub samples: Samples,
}

fn srgb_decode(c: f32) -> f32 {
    if c > 0.04045 { ((c + 0.055) / 1.055).powf(2.4) } else { c / 12.92 }
}

impl Picture {
    /// RGBA8 pixels, kept as RGB without `alpha`.
    pub fn rgba8(w: u32, h: u32, rgba: Vec<u8>, alpha: bool) -> Self {
        let px = if alpha { rgba } else { rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect() };
        Self { w, h, alpha, samples: Samples::U8(px) }
    }

    fn channels(&self) -> usize {
        if self.alpha { 4 } else { 3 }
    }

    fn depth(&self) -> Depth {
        match self.samples {
            Samples::U8(_) => Depth::U8,
            Samples::U16(_) => Depth::U16,
            Samples::F32(_) => Depth::F32,
        }
    }

    /// Sample `i` as a 0..1 display value (alpha stays linear).
    fn display(&self, i: usize) -> f32 {
        match &self.samples {
            Samples::U8(v) => v[i] as f32 / 255.0,
            Samples::U16(v) => v[i] as f32 / 65535.0,
            Samples::F32(v) if i % self.channels() == 3 => v[i].clamp(0.0, 1.0),
            Samples::F32(v) => crate::capture::srgb_encode(v[i]),
        }
    }

    fn len(&self) -> usize {
        self.w as usize * self.h as usize * self.channels()
    }

    fn to_u8(&self) -> Vec<u8> {
        match &self.samples {
            Samples::U8(v) => v.clone(),
            _ => (0..self.len()).map(|i| (self.display(i) * 255.0 + 0.5) as u8).collect(),
        }
    }

    fn to_u16(&self) -> Vec<u16> {
        match &self.samples {
            Samples::U16(v) => v.clone(),
            _ => (0..self.len()).map(|i| (self.display(i) * 65535.0 + 0.5) as u16).collect(),
        }
    }

    fn to_f32(&self) -> Vec<f32> {
        match &self.samples {
            Samples::F32(v) => v.clone(),
            _ => (0..self.len()).map(|i| if i % self.channels() == 3 { self.display(i) } else { srgb_decode(self.display(i)) }).collect(),
        }
    }

    /// The same picture with RGBA samples (alpha 1 added when there was none).
    fn with_alpha(&self) -> Picture {
        if self.alpha {
            return self.clone();
        }
        let n = self.w as usize * self.h as usize;
        let samples = match &self.samples {
            Samples::U8(v) => Samples::U8((0..n).flat_map(|p| [v[3 * p], v[3 * p + 1], v[3 * p + 2], 255]).collect()),
            Samples::U16(v) => Samples::U16((0..n).flat_map(|p| [v[3 * p], v[3 * p + 1], v[3 * p + 2], 65535]).collect()),
            Samples::F32(v) => Samples::F32((0..n).flat_map(|p| [v[3 * p], v[3 * p + 1], v[3 * p + 2], 1.0]).collect()),
        };
        Picture { w: self.w, h: self.h, alpha: true, samples }
    }
}

/// Writes `pic` as `format` (one of [`FORMATS`]), at the deepest of its depth the format holds.
pub fn encode(path: &Path, format: &str, jpeg_quality: u8, pic: &Picture) -> std::io::Result<()> {
    use image::ImageEncoder;
    let alpha = pic.alpha && format != "jpg";
    let pic = if alpha == pic.alpha {
        pic.clone()
    } else {
        // JPEG: drop the alpha.
        let rgb: Vec<u8> = pic.to_u8().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        Picture { w: pic.w, h: pic.h, alpha: false, samples: Samples::U8(rgb) }
    };
    let depth = best_depth(format, pic.depth());
    let (w, h) = (pic.w, pic.h);
    let bytes = match format {
        "png" | "jpg" => {
            let mut out = Vec::new();
            let r = if format == "png" && depth == Depth::U16 {
                let px: Vec<u8> = pic.to_u16().iter().flat_map(|v| v.to_ne_bytes()).collect();
                let color = if alpha { image::ExtendedColorType::Rgba16 } else { image::ExtendedColorType::Rgb16 };
                image::codecs::png::PngEncoder::new(&mut out).write_image(&px, w, h, color)
            } else {
                let px = pic.to_u8();
                let color = if alpha { image::ExtendedColorType::Rgba8 } else { image::ExtendedColorType::Rgb8 };
                if format == "png" {
                    image::codecs::png::PngEncoder::new(&mut out).write_image(&px, w, h, color)
                } else {
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, jpeg_quality.clamp(1, 100)).write_image(&px, w, h, color)
                }
            };
            r.map_err(std::io::Error::other)?;
            out
        }
        "tga" => tga(w, h, &pic.to_u8(), alpha),
        "dds" => {
            let mut out = dds_header(w, h, depth, alpha, false);
            out.extend(dds_data(&pic, depth));
            out
        }
        _ => {
            let spp = pic.channels() as u32;
            match depth {
                Depth::U8 => tiff(w, h, &pic.to_u8(), spp, 8, false),
                Depth::U16 => tiff(w, h, &pic.to_u16().iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(), spp, 16, false),
                Depth::F32 => tiff(w, h, &pic.to_f32().iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(), spp, 32, true),
            }
        }
    };
    std::fs::File::create(path)?.write_all(&bytes)
}

/// Uncompressed true-colour TGA, top-left origin, BGR(A).
fn tga(w: u32, h: u32, px: &[u8], alpha: bool) -> Vec<u8> {
    let n = if alpha { 4 } else { 3 };
    let mut out = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.push(8 * n as u8);
    out.push(0x20 | if alpha { 8 } else { 0 });
    for p in px.chunks_exact(n) {
        out.extend_from_slice(&[p[2], p[1], p[0]]);
        if alpha {
            out.push(p[3]);
        }
    }
    out
}

/// D3DFMT codes used as DDS FourCCs: A16B16G16R16 and A32B32G32R32F.
const D3DFMT_A16B16G16R16: u32 = 36;
const D3DFMT_A32B32G32R32F: u32 = 116;

/// A DDS header: 32-bit A8R8G8B8 (X8R8G8B8 without alpha), or A16B16G16R16 / A32B32G32R32F by FourCC;
/// `cube` marks all six cubemap faces.
fn dds_header(w: u32, h: u32, depth: Depth, alpha: bool, cube: bool) -> Vec<u8> {
    let mut hdr = [0u32; 31];
    hdr[0] = 124; // dwSize
    hdr[1] = 0x1 | 0x2 | 0x4 | 0x8 | 0x1000; // CAPS | HEIGHT | WIDTH | PITCH | PIXELFORMAT
    hdr[2] = h;
    hdr[3] = w;
    hdr[18] = 32; // ddspf.dwSize
    match depth {
        Depth::U8 => {
            hdr[4] = w * 4; // pitch
            hdr[19] = 0x40 | if alpha { 0x1 } else { 0 }; // RGB | ALPHAPIXELS
            hdr[21] = 32;
            hdr[22] = 0x00ff_0000;
            hdr[23] = 0x0000_ff00;
            hdr[24] = 0x0000_00ff;
            hdr[25] = if alpha { 0xff00_0000 } else { 0 };
        }
        Depth::U16 | Depth::F32 => {
            hdr[4] = w * if depth == Depth::U16 { 8 } else { 16 };
            hdr[19] = 0x4; // FOURCC
            hdr[20] = if depth == Depth::U16 { D3DFMT_A16B16G16R16 } else { D3DFMT_A32B32G32R32F };
        }
    }
    hdr[26] = 0x1000 | if cube { 0x8 } else { 0 }; // TEXTURE | COMPLEX
    hdr[27] = if cube { 0x200 | 0xfc00 } else { 0 }; // CUBEMAP | all six faces
    let mut out = b"DDS ".to_vec();
    for v in hdr {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// The pixels of a DDS image of `depth` (always four channels).
fn dds_data(pic: &Picture, depth: Depth) -> Vec<u8> {
    let n = pic.channels();
    match depth {
        Depth::U8 => pic.to_u8().chunks_exact(n).flat_map(|p| [p[2], p[1], p[0], if n == 4 { p[3] } else { 255 }]).collect(),
        Depth::U16 => pic.with_alpha().to_u16().iter().flat_map(|v| v.to_le_bytes()).collect(),
        Depth::F32 => pic.with_alpha().to_f32().iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

/// A cubemap DDS of six faces (+X, -X, +Y, -Y, +Z, -Z) of one size and depth: SE's "dds cubemap".
pub fn dds_cube(faces: &[Picture]) -> Vec<u8> {
    let Some(f0) = faces.first() else { return Vec::new() };
    let depth = f0.depth();
    let mut out = dds_header(f0.w, f0.h, depth, true, true);
    for f in faces {
        out.extend(dds_data(&f.with_alpha(), depth));
    }
    out
}

/// Baseline uncompressed little-endian TIFF, one strip, `spp` samples of `bits` (float with `float`),
/// RGB(A) with unassociated alpha.
fn tiff(w: u32, h: u32, data: &[u8], spp: u32, bits: u16, float: bool) -> Vec<u8> {
    let alpha = spp == 4;
    let mut tags: Vec<(u16, u16, u32, u32)> = Vec::new(); // tag, type (3 short, 4 long, 5 rational), count, value/offset
    let header = 8u32;
    let n_tags = 12 + alpha as u32 + float as u32;
    let ifd_len = 2 + n_tags * 12 + 4;
    let bps_off = header + ifd_len;
    let res_off = bps_off + 2 * spp;
    let fmt_off = res_off + 16;
    let data_off = fmt_off + if float { 2 * spp } else { 0 };
    tags.push((256, 4, 1, w));
    tags.push((257, 4, 1, h));
    tags.push((258, 3, spp, bps_off));
    tags.push((259, 3, 1, 1));
    tags.push((262, 3, 1, 2));
    tags.push((273, 4, 1, data_off));
    tags.push((277, 3, 1, spp));
    tags.push((278, 4, 1, h));
    tags.push((279, 4, 1, data.len() as u32));
    tags.push((282, 5, 1, res_off));
    tags.push((283, 5, 1, res_off + 8));
    tags.push((296, 3, 1, 2));
    if alpha {
        tags.push((338, 3, 1, 2));
    }
    if float {
        tags.push((339, 3, spp, fmt_off)); // SampleFormat: IEEE float
    }
    let mut out = b"II".to_vec();
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&header.to_le_bytes());
    out.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    for (tag, ty, count, value) in tags {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if ty == 3 && count == 1 {
            out.extend_from_slice(&(value as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        } else {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..spp {
        out.extend_from_slice(&bits.to_le_bytes());
    }
    for _ in 0..2 {
        out.extend_from_slice(&72u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    if float {
        for _ in 0..spp {
            out.extend_from_slice(&3u16.to_le_bytes());
        }
    }
    out.extend_from_slice(data);
    out
}

// ---------------------------------------------------------------------------
// High-resolution screenshot tool (seScreenshotTool)

/// SE's "Max dimensions" (FUN_140224450: 0xffff x 0xffff).
pub const MAX_DIM: u32 = 0xffff;
/// `ScreenshotTileRes` range and step (FUN_140224450: 1024, 512..1024, 16).
pub const TILE_RES_MIN: u32 = 512;
pub const TILE_RES_MAX: u32 = 1024;

pub fn clamp_tile_res(r: u32) -> u32 {
    (r.clamp(TILE_RES_MIN, TILE_RES_MAX) / 16) * 16
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct HiresSettings {
    /// "Target resolution" (FUN_140224450: 8000 x 4000).
    pub target: UVec2,
    /// `ScreenshotTileRes`, "Tile resolution".
    pub tile_res: u32,
    /// `ScreenshotLandLOD`: the landscape LOD while the tiles render.
    pub land_lod: f32,
    /// "Scale stars": point sizes as on a screen of the target's height.
    pub scale_stars: bool,
    /// "Wait loading completeness": each tile waits for the landscape tiles.
    pub wait_loading: bool,
}

impl Default for HiresSettings {
    fn default() -> Self {
        Self { target: UVec2::new(8000, 4000), tile_res: TILE_RES_MAX, land_lod: 0.0, scale_stars: true, wait_loading: true }
    }
}

/// Tiles across and down, and the size of each (the target split evenly, no tile above `tile_res`).
pub fn tile_grid(target: UVec2, tile_res: u32) -> (UVec2, UVec2) {
    let t = tile_res.max(1);
    let n = UVec2::new(target.x.div_ceil(t).max(1), target.y.div_ceil(t).max(1));
    (n, UVec2::new(target.x.div_ceil(n.x), target.y.div_ceil(n.y)))
}

/// "Max size in memory": the RGBA8 image.
pub fn memory_bytes(target: UVec2) -> u64 {
    target.x as u64 * target.y as u64 * 4
}

/// SE's checks: "Incorrect resolution", "Too big resolution!" (above the max dimensions, or a file
/// over 4 GB, which TIFF and DDS cannot address).
pub fn check_target(target: UVec2) -> Result<(), &'static str> {
    if target.x == 0 || target.y == 0 {
        Err("Incorrect resolution")
    } else if target.x > MAX_DIM || target.y > MAX_DIM || memory_bytes(target) >= u32::MAX as u64 {
        Err("Too big resolution!")
    } else {
        Ok(())
    }
}

/// The vertical field of view of the HD image (FUN_140221d70): a target wider than the window keeps
/// the window's horizontal field, otherwise its vertical one.
pub fn hires_fov(fov: f32, window_aspect: f32, target_aspect: f32) -> f32 {
    if window_aspect < target_aspect { 2.0 * ((fov * 0.5).tan() * (window_aspect / target_aspect)).atan() } else { fov }
}

/// Start the tool (its Start button, the `hires` dev command).
#[derive(Resource, Default)]
pub struct HiresRequest(pub bool);

/// The tool's window (Shift+F11, SE's HiresScreenshot).
#[derive(Resource, Default)]
pub struct HiresUi {
    pub open: bool,
}

struct HiresJob {
    target: UVec2,
    grid: UVec2,
    tile: UVec2,
    index: u32,
    view: (Entity, Handle<Image>),
    grabs: Grabs,
    frames: u32,
    quiet: u32,
    requested: bool,
    out: Vec<u8>,
    alpha: bool,
    path: PathBuf,
    settings: ScreenshotSettings,
    restore: (bool, f32),
}

#[derive(Resource, Default)]
struct HiresState(Option<HiresJob>);

impl HiresState {
    pub fn running(&self) -> bool {
        self.0.is_some()
    }
}

fn hires_key(keys: Res<ButtonInput<KeyCode>>, mut egui: EguiContexts, mut ui: ResMut<HiresUi>, mut env_read: Local<bool>) {
    if !std::mem::replace(&mut *env_read, true) {
        // Env `SE_UI_OPEN` (testing): `hires` opens the tool.
        ui.open |= std::env::var("SE_UI_OPEN").is_ok_and(|v| v.split(',').any(|w| w.trim() == "hires"));
    }
    if egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false) {
        return;
    }
    if keys.just_pressed(KeyCode::F11) && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        ui.open = !ui.open;
    }
}

#[allow(clippy::too_many_arguments)]
fn run_hires(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut req: ResMut<HiresRequest>,
    mut state: ResMut<HiresState>,
    settings: Res<HiresSettings>,
    shots: Res<ScreenshotSettings>,
    (cam, mut sim, mut gfx, mut stars): (
        Res<crate::camera::FlyCamera>,
        ResMut<crate::universe::SimTime>,
        ResMut<crate::graphics::GraphicsSettings>,
        ResMut<crate::stars::StarSettings>,
    ),
    proj: Res<crate::projection::ProjectionSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cams: Query<&mut Camera, With<CaptureView>>,
    terrains: Query<&crate::terrain::Terrain>,
    (mut msg, i18n): (ResMut<crate::messages::Messages>, Res<crate::i18n::I18n>),
) {
    if std::mem::take(&mut req.0) && state.0.is_none() {
        if proj.mode != 0 {
            msg.show(i18n.tr("This tool can only be used in 2D mode with perspective projection. Do you want to switch to that mode and open the tool?"));
            return;
        }
        let s = settings.clone();
        if let Err(e) = check_target(s.target) {
            msg.show(i18n.tr(e));
            return;
        }
        let mut out = Vec::new();
        if out.try_reserve_exact(memory_bytes(s.target) as usize).is_err() {
            msg.show(i18n.tr("Too big resolution!"));
            return;
        }
        out.resize(memory_bytes(s.target) as usize, 0);
        let (grid, tile) = tile_grid(s.target, clamp_tile_res(s.tile_res));
        let win = windows.iter().next().map(|w| w.width() / w.height().max(1.0)).unwrap_or(16.0 / 9.0);
        let fov = hires_fov(cam.fov_deg.to_radians(), win, s.target.x as f32 / s.target.y as f32);
        // No bloom: spread over each tile's own pixels it would show the tile edges.
        let view = CaptureView { rel: Quat::IDENTITY, fov: Some(fov), post: Post { bloom: false, correct: true } };
        let view = crate::capture::spawn_view(&mut commands, &mut images, tile, view, -60, shots.save_alpha);
        let dir = folder().join("screenshots-hd");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("mosaic{:05}.{}", scan_number("mosaic", &dir), shots.format));
        info!(
            "Starting high-resolution screenshot rendering: {} x {}, {} x {} tiles of {} x {}",
            s.target.x, s.target.y, grid.x, grid.y, tile.x, tile.y
        );
        // Time stops and the landscape takes the screenshot LOD while the tiles render (FUN_140221d70).
        let restore = (sim.paused, gfx.quality.land_lod);
        sim.paused = true;
        gfx.quality.land_lod = s.land_lod;
        state.0 = Some(HiresJob {
            target: s.target,
            grid,
            tile,
            index: 0,
            view,
            grabs: Grabs::new(1),
            frames: 0,
            quiet: 0,
            requested: false,
            out,
            alpha: shots.save_alpha && shots.format != "jpg",
            path,
            settings: shots.clone(),
            restore,
        });
    }
    let Some(job) = state.0.as_mut() else { return };
    let total = job.grid.x * job.grid.y;
    let (tx, ty) = (job.index % job.grid.x, job.index / job.grid.x);
    let sub = Some(SubCameraView { full_size: job.target, offset: Vec2::new((tx * job.tile.x) as f32, (ty * job.tile.y) as f32), size: job.tile });
    match cams.get_mut(job.view.0) {
        Ok(mut c) if c.sub_camera_view != sub => c.sub_camera_view = sub,
        Ok(_) => {}
        // Spawned this frame: start counting once it renders.
        Err(_) => return,
    }
    if settings.scale_stars {
        // After `track_screen_scale` (PreUpdate): points as on a screen of the target's height.
        stars.screen_scale = job.target.y as f32 / 1080.0;
    }
    job.frames += 1;
    let mut finish: Option<bool> = None;
    if !job.requested {
        if crate::capture::terrain_loading(&terrains) == 0 {
            job.quiet += 1;
        } else {
            job.quiet = 0;
        }
        let loaded = !settings.wait_loading || job.quiet >= 3;
        if job.frames >= crate::capture::SETTLE_FRAMES && loaded {
            job.grabs.request(&mut commands, 0, job.view.1.clone());
            job.requested = true;
            msg.show(format!("{} {} / {}, {}%", i18n.tr("Rendering tile"), job.index + 1, total, (job.index + 1) * 100 / total));
        } else if job.frames > crate::capture::LOAD_TIMEOUT {
            msg.show(i18n.tr("Cannot finish loading current tile; reduce tile resolution and/or landscape LOD and try again."));
            finish = Some(false);
        }
    } else if let Some(data) = job.grabs.take(0) {
        let px = crate::capture::decode_rgba16f(&data, job.tile.x, job.tile.y);
        let (x0, y0) = (tx * job.tile.x, ty * job.tile.y);
        let w = job.tile.x.min(job.target.x - x0) as usize;
        let h = job.tile.y.min(job.target.y - y0) as usize;
        for y in 0..h {
            let src = &px[y * job.tile.x as usize..y * job.tile.x as usize + w];
            let dst = ((y0 as usize + y) * job.target.x as usize + x0 as usize) * 4;
            for (i, p) in src.iter().enumerate() {
                let o = dst + i * 4;
                for c in 0..3 {
                    job.out[o + c] = (crate::capture::srgb_encode(p[c]) * 255.0 + 0.5) as u8;
                }
                job.out[o + 3] = if job.alpha { (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 255 };
            }
        }
        job.index += 1;
        job.frames = 0;
        job.quiet = 0;
        job.requested = false;
        job.grabs = Grabs::new(1);
        if job.index >= total {
            finish = Some(true);
        }
    }
    let Some(save) = finish else { return };
    let job = state.0.take().expect("job");
    crate::capture::despawn_views(&mut commands, &mut images, std::slice::from_ref(&job.view));
    (sim.paused, gfx.quality.land_lod) = job.restore;
    if !save {
        return;
    }
    info!("Finishing high-resolution screenshot rendering");
    msg.show(i18n.tr("Saving"));
    let HiresJob { target, out, alpha, path, settings, .. } = job;
    std::thread::spawn(move || {
        let pic = Picture::rgba8(target.x, target.y, out, alpha);
        match encode(&path, &settings.format, settings.jpeg_quality, &pic) {
            Ok(()) => info!("high-resolution screenshot -> {}", path.display()),
            Err(e) => warn!("cannot save {}: {e}", path.display()),
        }
    });
}

/// SE's "High resolution screenshot" dialog.
fn hires_window(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<HiresUi>,
    mut settings: ResMut<HiresSettings>,
    mut req: ResMut<HiresRequest>,
    state: Res<HiresState>,
    mut proj: ResMut<crate::projection::ProjectionSettings>,
    i18n: Res<crate::i18n::I18n>,
) -> Result {
    if !ui_state.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let mut open = true;
    let mut s = settings.clone();
    egui::Window::new(crate::skin::title(ctx, i18n.tr("High resolution screenshot"))).open(&mut open).default_width(320.0).show(ctx, |ui| {
        if proj.mode != 0 {
            ui.label(i18n.tr("This tool can only be used in 2D mode with perspective projection. Do you want to switch to that mode and open the tool?"));
            ui.horizontal(|ui| {
                if ui.button(i18n.tr("Yes")).clicked() {
                    proj.mode = 0;
                }
                if ui.button(i18n.tr("No")).clicked() {
                    ui_state.open = false;
                }
            });
            return;
        }
        ui.horizontal(|ui| {
            ui.label(i18n.tr("Target resolution"));
            ui.add(egui::DragValue::new(&mut s.target.x).range(1..=MAX_DIM));
            ui.label("x");
            ui.add(egui::DragValue::new(&mut s.target.y).range(1..=MAX_DIM));
        });
        ui.label(format!("{}: {} x {}", i18n.tr("Max dimensions"), MAX_DIM, MAX_DIM));
        ui.label(format!("{}: {:.2} GB", i18n.tr("Max size in memory"), memory_bytes(s.target) as f64 / (1u64 << 30) as f64));
        ui.add(egui::Slider::new(&mut s.tile_res, TILE_RES_MIN..=TILE_RES_MAX).step_by(16.0).text(i18n.tr("Tile resolution")));
        let (grid, tile) = tile_grid(s.target, clamp_tile_res(s.tile_res));
        ui.label(format!("{}: {} x {} ({} x {})", i18n.tr("Tile count"), grid.x, grid.y, tile.x, tile.y));
        ui.checkbox(&mut s.scale_stars, i18n.tr("Scale stars"));
        ui.add(egui::Slider::new(&mut s.land_lod, -3.0..=3.0).step_by(0.5).text(i18n.tr("Landscape LOD")));
        ui.checkbox(&mut s.wait_loading, i18n.tr("Wait loading completeness"));
        if let Err(e) = check_target(s.target) {
            ui.colored_label(egui::Color32::LIGHT_RED, i18n.tr(e));
        }
        ui.add_enabled_ui(!state.running() && check_target(s.target).is_ok(), |ui| {
            if ui.button(i18n.tr("Start")).clicked() {
                req.0 = true;
            }
        });
    });
    if !open {
        ui_state.open = false;
    }
    if s != *settings {
        *settings = s;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_after_the_last_screenshot() {
        assert_eq!(next_number("scr", ["a.png", "scr00004.jpg", "scr00012.png", "scr1.png"].into_iter()), 13);
        assert_eq!(next_number("scr", std::iter::empty()), 0);
        assert_eq!(next_number("mosaic", ["mosaic00002.jpg", "scr00012.png"].into_iter()), 3);
        // Counted up per press without waiting for the files.
        let mut n = ScreenshotNumber::default();
        let empty = PathBuf::from("no such folder");
        assert_eq!((n.next(&empty), n.next(&empty)), (0, 1));
    }

    /// Writes a gradient in every format into `SE_SHOT_TEST_DIR` (to open in other programs).
    #[test]
    #[ignore]
    fn writes_every_format() {
        let dir = PathBuf::from(std::env::var("SE_SHOT_TEST_DIR").expect("SE_SHOT_TEST_DIR"));
        let (w, h) = (64u32, 32u32);
        let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 4) as u8, (i / w * 8) as u8, 128, if i % w < 32 { 255 } else { 64 }]).collect();
        let lin: Vec<[f32; 4]> = (0..w * h).map(|i| [(i % w) as f32 / 16.0, (i / w) as f32 / 32.0, 0.5, if i % w < 32 { 1.0 } else { 0.25 }]).collect();
        for f in FORMATS {
            for alpha in [false, true] {
                encode(&dir.join(format!("t_{alpha}.{f}")), f, 90, &Picture::rgba8(w, h, px.clone(), alpha)).unwrap();
                for d in [Depth::U16, Depth::F32] {
                    let pic = crate::capture::to_picture(w, h, &lin, alpha, best_depth(f, d));
                    encode(&dir.join(format!("t_{alpha}_{d:?}.{f}")), f, 90, &pic).unwrap();
                }
            }
        }
        let faces: Vec<Picture> = (0..6).map(|i| crate::capture::to_picture(w, w, &vec![[i as f32 / 5.0, 0.2, 0.1, 1.0]; (w * w) as usize], true, Depth::U8)).collect();
        std::fs::write(dir.join("cube.dds"), dds_cube(&faces)).unwrap();
    }

    #[test]
    fn writers_lay_out_headers() {
        let px = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let t = tga(2, 1, &px, true);
        assert_eq!((t[2], t[12], t[16], t[17]), (2, 2, 32, 0x28));
        assert_eq!(&t[18..22], &[30, 20, 10, 40]);
        let pic = Picture { w: 2, h: 1, alpha: true, samples: Samples::U8(px.to_vec()) };
        let mut d = dds_header(2, 1, Depth::U8, true, false);
        d.extend(dds_data(&pic, Depth::U8));
        assert_eq!((&d[0..4], d.len()), (&b"DDS "[..], 128 + 8));
        let rgb = [1u8, 2, 3, 4, 5, 6];
        let f = tiff(2, 1, &rgb, 3, 8, false);
        let data_off = 8 + 2 + 12 * 12 + 4 + 6 + 16;
        assert_eq!(&f[data_off..], &rgb);
        assert_eq!(&f[0..4], b"II*\0");
        // Float TIFF: a SampleFormat tag and its three values before the data.
        let fl: Vec<u8> = [1.0f32, 0.5, 0.25].iter().flat_map(|v| v.to_le_bytes()).collect();
        let f = tiff(1, 1, &fl, 3, 32, true);
        let data_off = 8 + 2 + 13 * 12 + 4 + 6 + 16 + 6;
        assert_eq!(&f[data_off..], &fl[..]);
        assert_eq!(u16::from_le_bytes([f[data_off - 2], f[data_off - 1]]), 3);
        // 16-bit and float DDS by FourCC, alpha added.
        let p16 = Picture { w: 1, h: 1, alpha: false, samples: Samples::U16(vec![1, 2, 3]) };
        let mut d = dds_header(1, 1, Depth::U16, false, false);
        assert_eq!(u32::from_le_bytes(d[84..88].try_into().unwrap()), D3DFMT_A16B16G16R16);
        d.extend(dds_data(&p16, Depth::U16));
        assert_eq!(&d[128..], &[1, 0, 2, 0, 3, 0, 255, 255]);
        let cube = dds_cube(&vec![Picture { w: 1, h: 1, alpha: false, samples: Samples::F32(vec![0.5, 0.5, 0.5]) }; 6]);
        assert_eq!(cube.len(), 128 + 6 * 16);
        assert_eq!(u32::from_le_bytes(cube[112..116].try_into().unwrap()), 0xfe00);
    }

    #[test]
    fn depths_per_format() {
        assert_eq!(best_depth("jpg", Depth::F32), Depth::U8);
        assert_eq!(best_depth("png", Depth::F32), Depth::U16);
        assert_eq!(best_depth("tif", Depth::F32), Depth::F32);
        let p = Picture { w: 1, h: 1, alpha: false, samples: Samples::F32(vec![1.0, 0.0, 0.214]) };
        let u = p.to_u8();
        assert_eq!((u[0], u[1]), (255, 0));
        assert!((u[2] as i32 - 128).abs() <= 1, "{}", u[2]);
    }

    #[test]
    fn hires_tiles_and_fov() {
        // SE's default 8000 x 4000 in tiles of at most 1024.
        assert_eq!(tile_grid(UVec2::new(8000, 4000), 1024), (UVec2::new(8, 4), UVec2::new(1000, 1000)));
        assert_eq!(tile_grid(UVec2::new(1025, 100), 1024), (UVec2::new(2, 1), UVec2::new(513, 100)));
        assert_eq!(clamp_tile_res(1000), 992);
        assert_eq!(clamp_tile_res(100), 512);
        assert!(check_target(UVec2::new(8000, 4000)).is_ok());
        assert_eq!(check_target(UVec2::new(70000, 10)), Err("Too big resolution!"));
        assert_eq!(check_target(UVec2::new(65535, 65535)), Err("Too big resolution!"));
        assert_eq!(check_target(UVec2::new(0, 10)), Err("Incorrect resolution"));
        // A 2:1 target from a 16:9 window keeps the horizontal field.
        let v = 60f32.to_radians();
        let f = hires_fov(v, 16.0 / 9.0, 2.0);
        assert!(((f * 0.5).tan() * 2.0 - (v * 0.5).tan() * 16.0 / 9.0).abs() < 1e-5);
        assert_eq!(hires_fov(v, 16.0 / 9.0, 1.0), v);
    }
}
