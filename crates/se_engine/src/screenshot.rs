//! SE's screenshots (F11): main-def.cfg `Export` `ScreenshotFormat` (jpg, png, dds, tga, tif; "Screenshots
//! format"), `JpegQuality` (95) and `SaveAlpha` ("Save transparency", png, dds, tga and tif only), saved as
//! `scr%05i.<ext>` ("%sscr%05i.%S") numbered after the last one in the folder. SE writes into its
//! `screenshots/` (`UserScreenshotsSubDir`); the port never writes into the install and keeps
//! `<Pictures>/SpaceEngine-rs/`. JPEG and PNG use the `image` crate; TGA, DDS and TIFF are written
//! uncompressed here.

use std::io::Write;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

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
        app.init_resource::<ScreenshotSettings>().init_resource::<ScreenshotNumber>().add_systems(Startup, load_settings);
    }
}

fn load_settings(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<ScreenshotSettings>) {
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
}

/// The screenshots folder, `<Pictures>/SpaceEngine-rs`.
pub fn folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Pictures").join("SpaceEngine-rs"))
        .unwrap_or_else(|| PathBuf::from("screenshots"))
}

/// The number after the highest `scrNNNNN.*` in `names`.
fn next_number<'a>(names: impl Iterator<Item = &'a str>) -> u32 {
    names
        .filter_map(|n| n.strip_prefix("scr").and_then(|r| r.split('.').next()).filter(|d| d.len() == 5).and_then(|d| d.parse::<u32>().ok()))
        .max()
        .map_or(0, |m| m + 1)
}

/// The number of the next screenshot: the folder is scanned once, then counted up per press, so two
/// presses before the first capture arrives still get two numbers.
#[derive(Resource, Default)]
pub struct ScreenshotNumber(Option<u32>);

impl ScreenshotNumber {
    fn next(&mut self, dir: &Path) -> u32 {
        let n = *self.0.get_or_insert_with(|| {
            let names: Vec<String> = std::fs::read_dir(dir)
                .map(|it| it.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                .unwrap_or_default();
            next_number(names.iter().map(String::as_str))
        });
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
        if let Err(e) = encode(&path, &s, w, h, rgba.as_raw()) {
            warn!("cannot save screenshot {}: {e}", path.display());
        }
    });
}

/// Writes RGBA8 pixels (top row first) as `settings.format`.
fn encode(path: &Path, s: &ScreenshotSettings, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    use image::ImageEncoder;
    let alpha = s.save_alpha && s.format != "jpg";
    let px: Vec<u8> = if alpha { rgba.to_vec() } else { rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect() };
    let bytes = match s.format.as_str() {
        "png" | "jpg" => {
            let mut out = Vec::new();
            let color = if alpha { image::ExtendedColorType::Rgba8 } else { image::ExtendedColorType::Rgb8 };
            let r = if s.format == "png" {
                image::codecs::png::PngEncoder::new(&mut out).write_image(&px, w, h, color)
            } else {
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, s.jpeg_quality.clamp(1, 100)).write_image(&px, w, h, color)
            };
            r.map_err(std::io::Error::other)?;
            out
        }
        "tga" => tga(w, h, &px, alpha),
        "dds" => dds(w, h, &px, alpha),
        _ => tiff(w, h, &px, alpha),
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

/// Uncompressed 32-bit DDS (A8R8G8B8, or X8R8G8B8 without alpha).
fn dds(w: u32, h: u32, px: &[u8], alpha: bool) -> Vec<u8> {
    let n = if alpha { 4 } else { 3 };
    let mut hdr = [0u32; 31];
    hdr[0] = 124; // dwSize
    hdr[1] = 0x1 | 0x2 | 0x4 | 0x8 | 0x1000; // CAPS | HEIGHT | WIDTH | PITCH | PIXELFORMAT
    hdr[2] = h;
    hdr[3] = w;
    hdr[4] = w * 4; // pitch
    hdr[18] = 32; // ddspf.dwSize
    hdr[19] = 0x40 | if alpha { 0x1 } else { 0 }; // RGB | ALPHAPIXELS
    hdr[21] = 32;
    hdr[22] = 0x00ff_0000;
    hdr[23] = 0x0000_ff00;
    hdr[24] = 0x0000_00ff;
    hdr[25] = if alpha { 0xff00_0000 } else { 0 };
    hdr[26] = 0x1000; // DDSCAPS_TEXTURE
    let mut out = b"DDS ".to_vec();
    for v in hdr {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for p in px.chunks_exact(n) {
        out.extend_from_slice(&[p[2], p[1], p[0], if alpha { p[3] } else { 255 }]);
    }
    out
}

/// Baseline uncompressed little-endian TIFF, one strip, RGB(A) with unassociated alpha.
fn tiff(w: u32, h: u32, px: &[u8], alpha: bool) -> Vec<u8> {
    let spp: u32 = if alpha { 4 } else { 3 };
    let mut tags: Vec<(u16, u16, u32, u32)> = Vec::new(); // tag, type (3 short, 4 long, 5 rational), count, value/offset
    let header = 8u32;
    let n_tags = if alpha { 13 } else { 12 };
    let ifd_len = 2 + n_tags * 12 + 4;
    let bps_off = header + ifd_len;
    let res_off = bps_off + 2 * spp;
    let data_off = res_off + 16;
    tags.push((256, 4, 1, w));
    tags.push((257, 4, 1, h));
    tags.push((258, 3, spp, bps_off));
    tags.push((259, 3, 1, 1));
    tags.push((262, 3, 1, 2));
    tags.push((273, 4, 1, data_off));
    tags.push((277, 3, 1, spp));
    tags.push((278, 4, 1, h));
    tags.push((279, 4, 1, w * h * spp));
    tags.push((282, 5, 1, res_off));
    tags.push((283, 5, 1, res_off + 8));
    tags.push((296, 3, 1, 2));
    if alpha {
        tags.push((338, 3, 1, 2));
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
        out.extend_from_slice(&8u16.to_le_bytes());
    }
    for _ in 0..2 {
        out.extend_from_slice(&72u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    out.extend_from_slice(px);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_after_the_last_screenshot() {
        assert_eq!(next_number(["a.png", "scr00004.jpg", "scr00012.png", "scr1.png"].into_iter()), 13);
        assert_eq!(next_number(std::iter::empty()), 0);
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
        for f in FORMATS {
            for alpha in [false, true] {
                let s = ScreenshotSettings { format: f.into(), jpeg_quality: 90, save_alpha: alpha };
                encode(&dir.join(format!("t_{alpha}.{f}")), &s, w, h, &px).unwrap();
            }
        }
    }

    #[test]
    fn writers_lay_out_headers() {
        let px = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let t = tga(2, 1, &px, true);
        assert_eq!((t[2], t[12], t[16], t[17]), (2, 2, 32, 0x28));
        assert_eq!(&t[18..22], &[30, 20, 10, 40]);
        let d = dds(2, 1, &px, true);
        assert_eq!((&d[0..4], d.len()), (&b"DDS "[..], 128 + 8));
        let rgb = [1u8, 2, 3, 4, 5, 6];
        let f = tiff(2, 1, &rgb, false);
        let data_off = 8 + 2 + 12 * 12 + 4 + 6 + 16;
        assert_eq!(&f[data_off..], &rgb);
        assert_eq!(&f[0..4], b"II*\0");
    }
}
