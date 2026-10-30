//! UI translations. SpaceEngine's `locale/<lang>-gui.cfg` files map each English UI
//! string to its translation; [`I18n::tr`] looks a string up and falls back to the
//! English text when SE has no translation. The UI font is switched with the language
//! so Cyrillic and CJK glyphs render.

use std::sync::Arc;
use std::time::Instant;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::locale::{DEFAULT_LANG, Locale, available_languages, header_local_name, ui_font_file};
use se_core::vfs::Vfs;

use crate::data::SeData;

/// The active UI language and its translations.
#[derive(Resource)]
pub struct I18n {
    lang: String,
    locale: Locale,
    /// Every language SE ships a `*-gui.cfg` for: (code, name in that language).
    pub languages: Vec<(String, String)>,
}

impl I18n {
    pub fn new(vfs: &Vfs, lang: &str) -> Self {
        let languages = language_list(vfs);
        let mut i18n = Self { lang: String::new(), locale: Locale::default(), languages };
        i18n.set_language(vfs, lang);
        i18n
    }

    /// Current language code, e.g. `"eng"`.
    pub fn lang(&self) -> &str {
        &self.lang
    }

    /// Switches language and reloads its translations. Unknown codes fall back to English.
    pub fn set_language(&mut self, vfs: &Vfs, lang: &str) {
        if lang == self.lang {
            return;
        }
        let lang = if self.languages.iter().any(|(code, _)| code == lang) { lang } else { DEFAULT_LANG };
        if lang == self.lang {
            return;
        }
        self.lang = lang.to_string();
        self.locale = Locale::load(vfs, lang);
    }

    /// Translation of an English UI string, or `s` itself when SE has none.
    pub fn tr<'a>(&'a self, s: &'a str) -> &'a str {
        self.locale.tr(s)
    }

    /// SE's planet classification string in this language (`se_core::classname`, main-def.cfg's
    /// default style).
    pub fn planet_class(&self, c: &se_core::surface_gen::Classes, life: bool) -> String {
        se_core::classname::planet_class(&self.locale, c, life, se_core::classname::ClassStyle::default())
    }

    /// Display name of a language code (its own name, e.g. `"Русский"`).
    pub fn name_of<'a>(&'a self, code: &'a str) -> &'a str {
        self.languages.iter().find(|(c, _)| c == code).map(|(_, n)| n.as_str()).unwrap_or(code)
    }
}

fn language_list(vfs: &Vfs) -> Vec<(String, String)> {
    available_languages(vfs)
        .into_iter()
        .map(|code| {
            let name = vfs
                .read_string(&format!("locale/{code}-gui.cfg"))
                .ok()
                .and_then(|text| header_local_name(&text))
                .unwrap_or_else(|| code.clone());
            (code, name)
        })
        .collect()
}

pub struct I18nPlugin;

impl Plugin for I18nPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, init_i18n).add_systems(EguiPrimaryContextPass, apply_ui_font.before(crate::skin::UiSet::Draw));
    }
}

/// Language from `SE_LANG` (testing), else English. Settings may change it at PostStartup.
pub fn init_i18n(mut commands: Commands, data: Res<SeData>) {
    let lang = std::env::var("SE_LANG").unwrap_or_else(|_| DEFAULT_LANG.to_string());
    let t = Instant::now();
    let i18n = I18n::new(&data.vfs, &lang);
    info!(
        "ui language {} ({} strings, {} languages) in {:.0?}",
        i18n.lang(),
        i18n.locale.gui_len(),
        i18n.languages.len(),
        t.elapsed()
    );
    commands.insert_resource(i18n);
}

/// SE's interface fonts (main-def.cfg): `PrimaryFont` / `PrimaryFontBold`, then `SecondaryFont` /
/// `SecondaryFontBold` and `TertiaryFont` for glyphs the first lacks (SE's `TertiaryFontBold` is
/// empty, so the bold chain ends on the regular DejaVu).
const FONT_CHAIN: [&str; 3] = ["NotoSans-Regular.ttf", "NotoSansCJKsc-Regular.otf", "DejaVuSans.ttf"];
const BOLD_CHAIN: [&str; 3] = ["NotoSans-Medium.ttf", "NotoSansCJKsc-Medium.otf", "DejaVuSans.ttf"];

/// Set once the fonts with the bold family are in use (a pass after `set_fonts`).
static BOLD_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The bold interface font family (SE's `PrimaryFontBold` chain); window titles use it. Until it is
/// installed, the proportional family.
pub fn bold_family() -> egui::FontFamily {
    if BOLD_READY.load(std::sync::atomic::Ordering::Relaxed) {
        egui::FontFamily::Name("bold".into())
    } else {
        egui::FontFamily::Proportional
    }
}

/// Installs SE's font chain as the proportional fonts and its bold chain as [`bold_family`] (also
/// the heading style), with egui's defaults last. A language whose own script SE draws from the
/// CJK font (`ui_font_file`) gets that font first. Runs only when the language changes.
fn apply_ui_font(
    mut contexts: EguiContexts,
    i18n: Res<I18n>,
    data: Res<SeData>,
    mut applied: Local<Option<String>>,
    mut pending_heading: Local<bool>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    // The pass after `set_fonts` has the new fonts: titles can use the bold family now.
    if *pending_heading {
        *pending_heading = false;
        BOLD_READY.store(true, std::sync::atomic::Ordering::Relaxed);
        ctx.all_styles_mut(|s| {
            if let Some(h) = s.text_styles.get_mut(&egui::TextStyle::Heading) {
                h.family = bold_family();
            }
        });
    }
    if applied.as_deref() == Some(i18n.lang()) {
        return Ok(());
    }
    let t = Instant::now();
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |file: &str| -> Option<String> {
        if fonts.font_data.contains_key(file) {
            return Some(file.to_string());
        }
        match data.vfs.read(&format!("locale/{file}")) {
            Ok(bytes) => {
                fonts.font_data.insert(file.to_string(), Arc::new(egui::FontData::from_owned(bytes)));
                Some(file.to_string())
            }
            Err(e) => {
                warn!("ui font locale/{file}: {e}");
                None
            }
        }
    };
    let first = ui_font_file(i18n.lang());
    let order = |chain: [&'static str; 3]| -> Vec<&'static str> {
        let mut v: Vec<&str> = first.filter(|f| f.contains("CJK") && !chain[0].contains("CJK")).map(|f| if chain[0].contains("Medium") { "NotoSansCJKsc-Medium.otf" } else { f }).into_iter().collect();
        for c in chain {
            if !v.contains(&c) {
                v.push(c);
            }
        }
        v
    };
    let regular: Vec<String> = order(FONT_CHAIN).into_iter().filter_map(&mut load).collect();
    let bold: Vec<String> = order(BOLD_CHAIN).into_iter().filter_map(&mut load).collect();
    let defaults = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    let prop = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    for (i, f) in regular.iter().enumerate() {
        prop.insert(i, f.clone());
    }
    fonts.families.insert(egui::FontFamily::Name("bold".into()), bold.into_iter().chain(defaults).collect());
    ctx.set_fonts(fonts);
    *pending_heading = true;
    info!("ui fonts {:?} loaded in {:.0?}", regular, t.elapsed());
    *applied = Some(i18n.lang().to_string());
    Ok(())
}
