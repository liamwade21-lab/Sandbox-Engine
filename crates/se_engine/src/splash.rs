//! SE's startup splash (textures/common/banner.cfg): a random `splash*.jpg` with the
//! `logo0990_1024` logo 5% from its top, a progress bar and loading line at 90% of its height, a
//! random quote (SE's 25, in the interface language) right-aligned at 66%, and the copyright and
//! version along the bottom (from SpaceEngine.exe's version resource). SE shows it in its own window
//! 480 px high at 1080p before the main window opens; here it covers the window while the first
//! seconds of loading run, then fades. Scripted runs skip it (unless `SE_SPLASH` is set).
//!
//! Not done: SE's in-session loading bar (MainMenu.cfg LoadingBarParams).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::config::EngineConfig;
use crate::data::SeData;
use crate::i18n::I18n;

/// banner.cfg `DesiredHeight` at a 1080 px screen.
const DESIRED_HEIGHT: f32 = 480.0;
/// The splash stays this long (s), then fades over `FADE_S`.
const SHOW_S: f32 = 3.0;
const FADE_S: f32 = 0.6;

pub struct SplashPlugin;

impl Plugin for SplashPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Splash>()
            .add_systems(Startup, prepare.after(crate::i18n::init_i18n))
            .add_systems(EguiPrimaryContextPass, draw.after(crate::skin::UiSet::Backdrop));
    }
}

#[derive(Resource, Default)]
struct Splash {
    active: bool,
    background: Option<egui::ColorImage>,
    logo: Option<egui::ColorImage>,
    textures: Option<(egui::TextureHandle, Option<egui::TextureHandle>)>,
    quote: (String, String),
    copyright: String,
    version: String,
    age: f32,
}

fn load_image(data: &SeData, path: &str) -> Option<egui::ColorImage> {
    let bytes = data.vfs.read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// A UTF-16 string value from the exe's version resource (`StringFileInfo` key `key`).
fn version_string(exe: &[u8], key: &str) -> Option<String> {
    let pat: Vec<u8> = key.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let at = exe.windows(pat.len()).rposition(|w| w == pat.as_slice())? + pat.len();
    // The value starts at the next 4-byte boundary.
    let start = at.next_multiple_of(4);
    let words: Vec<u16> = exe.get(start..start + 512)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&w| w != 0).collect();
    Some(String::from_utf16_lossy(&words))
}

fn prepare(mut splash: ResMut<Splash>, cfg: Res<EngineConfig>, data: Res<SeData>, i18n: Res<I18n>) {
    // SE_SPLASH=1 shows it in scripted runs too (screenshots of it), SE_NO_SPLASH=1 never.
    let forced = std::env::var_os("SE_SPLASH").is_some();
    if (crate::settings::scripted(&cfg) && !forced) || std::env::var_os("SE_NO_SPLASH").is_some() {
        return;
    }
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.subsec_nanos() as usize);
    let mut images: Vec<String> = data.vfs.list("textures/common").into_iter().filter(|p| p.rsplit('/').next().is_some_and(|n| n.starts_with("splash"))).collect();
    images.sort();
    if images.is_empty() {
        return;
    }
    splash.background = load_image(&data, &images[seed % images.len()]);
    splash.logo = load_image(&data, "textures/common/logo0990_1024.png");
    // SE's quotes ("text_- author") are its own interface strings: their English keys are listed in
    // every full translation file (eng-gui.cfg holds only the strings English renames).
    let mut keys = Vec::new();
    for lang in [i18n.lang(), "rus"] {
        if let Ok(text) = data.vfs.read_string(&format!("locale/{lang}-gui.cfg")) {
            let mut loc = se_core::locale::Locale::default();
            loc.parse_gui(&text);
            keys = loc.keys().filter(|k| k.contains("_- ")).map(str::to_string).collect();
            if !keys.is_empty() {
                break;
            }
        }
    }
    keys.sort();
    if !keys.is_empty() {
        let q = i18n.tr(&keys[(seed / 7) % keys.len()]).to_string();
        let (text, author) = q.split_once("_- ").unwrap_or((&q, ""));
        splash.quote = (text.to_string(), author.to_string());
    }
    if let Ok(exe) = std::fs::read(cfg.se_path.join("system").join("SpaceEngine.exe")) {
        splash.copyright = version_string(&exe, "LegalCopyright").unwrap_or_default();
        splash.version = version_string(&exe, "ProductVersion").unwrap_or_default();
    }
    splash.active = splash.background.is_some();
}

fn draw(mut contexts: EguiContexts, mut splash: ResMut<Splash>, time: Res<Time>) -> Result {
    if !splash.active {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    splash.age += time.delta_secs().min(0.1);
    let alpha = 1.0 - ((splash.age - SHOW_S) / FADE_S).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        splash.active = false;
        splash.textures = None;
        return Ok(());
    }
    if splash.textures.is_none() {
        let bg = splash.background.take().expect("splash image");
        let tex = ctx.load_texture("splash", bg, egui::TextureOptions::LINEAR);
        let logo = splash.logo.take().map(|l| ctx.load_texture("splash_logo", l, egui::TextureOptions::LINEAR));
        splash.textures = Some((tex, logo));
    }
    let (tex, logo) = splash.textures.clone().expect("splash textures");
    let screen = ctx.content_rect();
    let h = (screen.height() * DESIRED_HEIGHT / 1080.0).max(240.0);
    let [tw, th] = tex.size();
    let w = h * tw as f32 / th as f32;
    let rect = egui::Rect::from_center_size(screen.center(), egui::vec2(w, h));
    let fade = |c: egui::Color32| c.gamma_multiply(alpha);
    let a = egui::Area::new(egui::Id::new("splash")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false);
    a.show(ctx, |ui| {
        let p = ui.painter();
        p.rect_filled(screen, 0.0, fade(egui::Color32::BLACK));
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        p.image(tex.id(), rect, uv, fade(egui::Color32::WHITE));
        let at = |x: f32, y: f32| rect.min + egui::vec2(x * w, y * h);
        if let Some(l) = &logo {
            let [lw, lh] = l.size();
            let lwpx = w * 0.6;
            let lr = egui::Rect::from_min_size(egui::pos2(rect.center().x - lwpx * 0.5, at(0.0, 0.05).y), egui::vec2(lwpx, lwpx * lh as f32 / lw as f32));
            p.image(l.id(), lr, uv, fade(egui::Color32::WHITE));
        }
        let k = h / DESIRED_HEIGHT;
        // Progress bar (ProgressBar label) with the loading line in it.
        let bar = egui::Rect::from_min_size(at(0.0625, 0.90), egui::vec2(w * 0.875, h * 0.03125));
        let progress = (splash.age / SHOW_S).clamp(0.0, 1.0);
        p.rect_stroke(bar, 0.0, egui::Stroke::new(1.0, fade(egui::Color32::from_gray(204))), egui::StrokeKind::Inside);
        p.rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * progress, bar.height())), 0.0, fade(egui::Color32::from_white_alpha(77)));
        p.text(at(0.07, 0.90 + 0.03125 * 0.5), egui::Align2::LEFT_CENTER, "Loading...", egui::FontId::proportional(14.0 * k), fade(egui::Color32::WHITE));
        // Quote (QuoteText): right-aligned, italic in SE.
        let (q, author) = &splash.quote;
        if !q.is_empty() {
            let text = if author.is_empty() { q.clone() } else { format!("{q}\n- {author}") };
            let job = egui::text::LayoutJob::simple(text, egui::FontId::proportional(18.0 * k), fade(egui::Color32::WHITE), w * 0.5375);
            let mut job = job;
            job.halign = egui::Align::RIGHT;
            for s in &mut job.sections {
                s.format.italics = true;
            }
            let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
            let right = at(0.40 + 0.5375, 0.66 + 0.115);
            p.galley(egui::pos2(right.x, right.y - galley.size().y * 0.5), galley, egui::Color32::WHITE);
        }
        let bottom = at(0.0625, 0.93125 + 0.034);
        p.text(bottom, egui::Align2::LEFT_CENTER, &splash.copyright, egui::FontId::proportional(13.0 * k), fade(egui::Color32::from_gray(204)));
        p.text(at(0.9375, 0.93125 + 0.034), egui::Align2::RIGHT_CENTER, &splash.version, egui::FontId::proportional(13.0 * k), fade(egui::Color32::from_gray(204)));
    });
    ctx.request_repaint();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_resource_value() {
        let mut exe = vec![0u8; 6];
        exe.extend("ProductVersion".encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        while exe.len() % 4 != 0 {
            exe.push(0);
        }
        exe.extend("0.991".encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        exe.extend([0u8; 600]);
        assert_eq!(version_string(&exe, "ProductVersion").as_deref(), Some("0.991"));
    }
}
