//! Persistent user settings, stored in SpaceEngine's own `.cfg` syntax at
//! `%APPDATA%/SpaceEngine-rs/settings.cfg` (or `./settings.cfg` elsewhere).

use std::path::PathBuf;

use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::graphics::{GraphicsSettings, Quality};
use crate::i18n::I18n;
use crate::bloom::BloomSettings;
use crate::lensflare::LensFlareSettings;
use crate::music::MusicState;
use crate::overlay::{KindSet, MarkerKind, OverlaySettings};
use crate::stars::StarSettings;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, load_settings).add_systems(Update, save_when_changed);
    }
}

/// Scripted runs (`--run`, `--screenshot`) neither read nor write the user's settings,
/// so tests are reproducible and leave no camera state behind.
pub fn scripted(cfg: &EngineConfig) -> bool {
    !cfg.commands.is_empty() || cfg.screenshot.is_some() || cfg.script.is_some()
}

/// A user file in `%APPDATA%/SpaceEngine-rs/` (or the working directory when APPDATA is unset).
pub fn user_file(name: &str) -> PathBuf {
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("SpaceEngine-rs").join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn settings_path() -> PathBuf {
    user_file("settings.cfg")
}

/// Everything persisted, as one comparable snapshot.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    mag_limit: f32,
    star_overbright: f32,
    star_size: f32,
    star_saturation: f32,
    star_desaturation: f32,
    star_color_shift: f32,
    illum_saturation: f32,
    illum_saturation_auto: bool,
    star_points: u32,
    blur_length: f32,
    fov: f32,
    /// SE's `AutoHorizon` (Controls).
    auto_horizon: bool,
    orbits: bool,
    labels_mode: u8,
    markers: bool,
    /// SE's per-kind `ShowOrbits*`, `ShowLabels*`, `ShowMarkers*`.
    kinds: [KindSet; 3],
    star_label_mag: f32,
    show_fps: bool,
    info_full: bool,
    music: bool,
    music_volume: f32,
    /// SE's `SoundVolume` (interface sounds).
    sound_volume: f32,
    language: String,
    /// The interface skin (SE's `Skin`).
    skin: String,
    gfx_preset: String,
    gfx: Quality,
    lens: LensFlareSettings,
    bloom: BloomSettings,
    display: crate::display::DisplaySettings,
}

impl Snapshot {
    /// `ui`: the language and the skin.
    fn capture(s: &StarSettings, o: &OverlaySettings, cam: &FlyCamera, audio: (Option<&MusicState>, f32), ui: (&str, &str), g: (&GraphicsSettings, &LensFlareSettings, &BloomSettings, &crate::display::DisplaySettings)) -> Self {
        Self {
            language: ui.0.to_string(),
            skin: ui.1.to_string(),
            gfx_preset: g.0.preset.clone(),
            gfx: g.0.quality.clone(),
            lens: g.1.clone(),
            bloom: g.2.clone(),
            display: g.3.clone(),
            mag_limit: s.mag_limit,
            star_overbright: s.overbright,
            star_size: s.point_scale,
            star_saturation: s.saturation,
            star_desaturation: s.desaturation,
            star_color_shift: s.color_shift,
            illum_saturation: s.illum_saturation,
            illum_saturation_auto: s.illum_saturation_auto,
            star_points: s.point_style.method(),
            blur_length: s.blur_length,
            fov: cam.fov_deg,
            auto_horizon: cam.auto_horizon,
            orbits: o.orbits,
            labels_mode: o.labels_mode,
            markers: o.markers,
            kinds: [o.orbit_kinds, o.label_kinds, o.marker_kinds],
            star_label_mag: o.star_label_mag,
            show_fps: o.show_fps,
            info_full: o.info_full,
            music: audio.0.map(|m| m.enabled).unwrap_or(true),
            music_volume: audio.0.map(|m| m.volume).unwrap_or(crate::music::DEFAULT_VOLUME),
            sound_volume: audio.1,
        }
    }

    fn to_cfg(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        format!(
            "// SpaceEngine-rs user settings\nSettings\n{{\n\tStarMagnLimit   {}\n\tPointOverbright {}\n\tPointScale      {}\n\tStarColorSaturation {}\n\tPointDesaturation {}\n\tStarColorShift  {}\n\tSunIllumSaturation {}\n\tIllumSaturationAuto {}\n\tStarPointMethod {}\n\tMotionBlurLength {}\n\tFOV             {}\n\tAutoHorizon     {}\n\tShowOrbits      {}\n\tLabelsMode      {}\n\tShowMarkers     {}\n\tStarNameMagn    {}\n\tFPS             {}\n\tTextMode        {}\n\tMusic           {}\n\tMusicVolume     {}\n\tSoundVolume     {}\n\tLanguage        \"{}\"\n\tSkin            \"{}\"\n{}}}\n",
            self.mag_limit,
            self.star_overbright,
            self.star_size,
            self.star_saturation,
            self.star_desaturation,
            self.star_color_shift,
            self.illum_saturation,
            b(self.illum_saturation_auto),
            self.star_points,
            self.blur_length,
            self.fov,
            b(self.auto_horizon),
            b(self.orbits),
            self.labels_mode,
            b(self.markers),
            self.star_label_mag,
            b(self.show_fps),
            if self.info_full { 2 } else { 1 },
            b(self.music),
            (self.music_volume * 100.0).round(),
            (self.sound_volume * 100.0).round(),
            self.language,
            self.skin,
            self.filters_cfg()
        ) + &self.graphics_cfg() + &self.display.to_cfg()
    }

    /// The per-kind overlay filters, by SE's keys.
    fn filters_cfg(&self) -> String {
        let mut out = String::new();
        for (set, filter) in self.kinds.iter().zip(["ShowOrbits", "ShowLabels", "ShowMarkers"]) {
            for k in MarkerKind::ALL {
                out += &format!("\t{filter}{} {}\n", k.key(), set.has(k));
            }
        }
        out
    }

    fn graphics_cfg(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        let (q, l) = (&self.gfx, &self.lens);
        format!(
            "Graphics
{{
	Preset            \"{}\"
	MSAALevel         {}
	FXAA              {}
	AnisotropyLevel   {}
	LandLOD           {}
	MaxTilesPerFrame  {}
	VolumResolution   {}
	EnableLensFlares  {}
	EnableLensGhosts  {}
	EnablePseudoFlares {}
	LensFlareScale    {}
	LensFlareBright   {}
	PseudoFlareBright {}
	LensFlarePreset   \"{}\"
	LensGhostPreset   \"{}\"
	PseudoFlarePreset \"{}\"
	EnableBloom       {}
	BloomBright       {}
	BloomThreshold    {}
}}
",
            self.gfx_preset, q.msaa, b(q.fxaa), q.anisotropy, q.land_lod, q.max_tiles_per_frame, q.volume_resolution,
            b(l.flares), b(l.ghosts), b(l.pseudo), l.scale, l.bright, l.pseudo_bright, l.flare_preset, l.ghost_preset, l.pseudo_preset,
            b(self.bloom.enabled), self.bloom.bright, self.bloom.threshold
        )
    }
}

#[derive(Resource)]
struct Saved {
    last: Snapshot,
    timer: f32,
    /// With `SE_NO_MUSIC` set, the saved music preference is kept as it was.
    keep_music: Option<bool>,
    /// With `SE_LANG` set, the saved language is kept as it was.
    keep_lang: Option<String>,
}

impl Saved {
    fn capture(&self, s: &StarSettings, o: &OverlaySettings, cam: &FlyCamera, audio: (Option<&MusicState>, f32), ui: (&str, &str), g: (&GraphicsSettings, &LensFlareSettings, &BloomSettings, &crate::display::DisplaySettings)) -> Snapshot {
        let mut snap = Snapshot::capture(s, o, cam, audio, ui, g);
        if let Some(m) = self.keep_music {
            snap.music = m;
        }
        if let Some(l) = &self.keep_lang {
            snap.language = l.clone();
        }
        snap
    }
}

fn load_settings(
    mut commands: Commands,
    cfg: Res<EngineConfig>,
    mut stars: ResMut<StarSettings>,
    mut overlay: ResMut<OverlaySettings>,
    mut cam: ResMut<FlyCamera>,
    mut music: Option<ResMut<MusicState>>,
    mut i18n: ResMut<I18n>,
    data: Res<SeData>,
    (mut gfx, mut lens, mut bloom, mut skin, mut sound): (ResMut<GraphicsSettings>, ResMut<LensFlareSettings>, ResMut<BloomSettings>, ResMut<crate::skin::Skin>, ResMut<crate::sound::SoundSettings>),
    display: Res<crate::display::DisplaySettings>,
) {
    if scripted(&cfg) {
        return;
    }
    let no_music = std::env::var_os("SE_NO_MUSIC").is_some();
    // SE_LANG (testing) wins over the saved language, as SE_NO_MUSIC does for music.
    let lang_forced = std::env::var_os("SE_LANG").is_some();
    let mut saved_music = true;
    let mut saved_lang: Option<String> = None;
    if let Ok(text) = std::fs::read_to_string(settings_path()) {
        let doc = se_core::sc::parse(&text);
        if let Some(b) = doc.sub("Settings") {
            let f = |k: &str, v: &mut f32| {
                if let Some(x) = b.f64(k) {
                    *v = x as f32;
                }
            };
            let bo = |k: &str, v: &mut bool| {
                if let Some(x) = b.bool(k) {
                    *v = x;
                }
            };
            // SE's key names (the port's earlier StarMagLimit/StarBrightness/StarSize/StarSaturation
            // meant something else and are ignored).
            f("StarMagnLimit", &mut stars.mag_limit);
            stars.mag_limit_reset = stars.mag_limit;
            f("PointOverbright", &mut stars.overbright);
            f("PointScale", &mut stars.point_scale);
            f("StarColorSaturation", &mut stars.saturation);
            f("PointDesaturation", &mut stars.desaturation);
            f("StarColorShift", &mut stars.color_shift);
            f("SunIllumSaturation", &mut stars.illum_saturation);
            bo("IllumSaturationAuto", &mut stars.illum_saturation_auto);
            f("MotionBlurLength", &mut stars.blur_length);
            if let Some(m) = b.f64("StarPointMethod") {
                stars.point_style = crate::stars::PointStyle::from_method(m as u32);
            }
            f("FOV", &mut cam.fov_deg);
            bo("AutoHorizon", &mut cam.auto_horizon);
            bo("ShowOrbits", &mut overlay.orbits);
            bo("ShowMarkers", &mut overlay.markers);
            match (b.f64("LabelsMode"), b.bool("ShowLabels")) {
                (Some(m), _) => overlay.labels_mode = m.clamp(0.0, 2.0) as u8,
                // The port's earlier ShowLabels / ShowStarNames.
                (None, Some(on)) => overlay.labels_mode = on as u8,
                _ => {}
            }
            if let Some(on) = b.bool("ShowStarNames") {
                overlay.label_kinds.set(MarkerKind::Star, on);
            }
            let o = &mut *overlay;
            for k in MarkerKind::ALL {
                for (filter, set) in [("ShowOrbits", &mut o.orbit_kinds), ("ShowLabels", &mut o.label_kinds), ("ShowMarkers", &mut o.marker_kinds)] {
                    if let Some(on) = b.bool(&format!("{filter}{}", k.key())) {
                        set.set(k, on);
                    }
                }
            }
            f("StarNameMagn", &mut overlay.star_label_mag);
            bo("FPS", &mut overlay.show_fps);
            if let Some(m) = b.f64("TextMode") {
                overlay.info_full = m >= 2.0;
            }
            bo("Music", &mut saved_music);
            if let Some(s) = b.str("Skin") {
                skin.name = s.to_string();
            }
            saved_lang = b.str("Language").map(str::to_string);
            if let Some(v) = b.f64("SoundVolume") {
                sound.volume = (v / 100.0).clamp(0.0, 1.0) as f32;
            }
            if let (false, Some(l)) = (lang_forced, &saved_lang) {
                i18n.set_language(&data.vfs, l);
            }
            if let Some(m) = music.as_mut() {
                // The SE_NO_MUSIC override wins over the saved preference.
                if !no_music {
                    m.enabled = saved_music;
                }
                // SE's 0..100 (the port wrote 0..1 before).
                if let Some(v) = b.f64("MusicVolume") {
                    m.volume = (if v > 1.0 { v / 100.0 } else { v }).clamp(0.0, 1.0) as f32;
                }
            }
            info!("loaded settings from {}", settings_path().display());
        }
        if let Some(g) = doc.sub("Graphics") {
            load_graphics(g, &mut gfx, &mut lens, &mut bloom);
        }
    }
    let mut saved = Saved {
        last: Snapshot::capture(&stars, &overlay, &cam, (music.as_deref(), sound.volume), (i18n.lang(), &skin.name), (&gfx, &lens, &bloom, &display)),
        timer: 0.0,
        keep_music: no_music.then_some(saved_music),
        keep_lang: lang_forced.then(|| saved_lang.clone().unwrap_or_else(|| se_core::locale::DEFAULT_LANG.to_string())),
    };
    saved.last = saved.capture(&stars, &overlay, &cam, (music.as_deref(), sound.volume), (i18n.lang(), &skin.name), (&gfx, &lens, &bloom, &display));
    commands.insert_resource(saved);
}

fn save_when_changed(
    time: Res<Time>,
    saved: Option<ResMut<Saved>>,
    stars: Res<StarSettings>,
    overlay: Res<OverlaySettings>,
    cam: Res<FlyCamera>,
    music: Option<Res<MusicState>>,
    i18n: Res<I18n>,
    (gfx, lens, bloom, skin, sound): (Res<GraphicsSettings>, Res<LensFlareSettings>, Res<BloomSettings>, Res<crate::skin::Skin>, Res<crate::sound::SoundSettings>),
    display: Res<crate::display::DisplaySettings>,
) {
    let Some(mut saved) = saved else { return };
    saved.timer += time.delta_secs();
    if saved.timer < 2.0 {
        return;
    }
    saved.timer = 0.0;
    let now = saved.capture(&stars, &overlay, &cam, (music.as_deref(), sound.volume), (i18n.lang(), &skin.name), (&gfx, &lens, &bloom, &display));
    if now != saved.last {
        let path = settings_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&path, now.to_cfg()) {
            Ok(()) => debug!("saved settings to {}", path.display()),
            Err(e) => warn!("cannot save settings to {}: {e}", path.display()),
        }
        saved.last = now;
    }
}

/// The `Graphics` block: a preset name, then its values (a "Custom" preset keeps the individual ones).
fn load_graphics(g: &se_core::sc::Block, gfx: &mut GraphicsSettings, lens: &mut LensFlareSettings, bloom: &mut BloomSettings) {
    if let Some(p) = g.str("Preset") {
        gfx.apply_preset(p);
        gfx.preset = p.to_string();
    }
    let mut text = String::new();
    for item in &g.items {
        if let Some(v) = item.f64() {
            text.push_str(&format!("{} {}
", item.key, v));
        } else if let Some(v) = item.bool() {
            text.push_str(&format!("{} {}
", item.key, v));
        }
    }
    gfx.quality = Quality::from_cfg(&text, &gfx.quality);
    let f = |k: &str, v: &mut f32| {
        if let Some(x) = g.f64(k) {
            *v = x as f32;
        }
    };
    let bo = |k: &str, v: &mut bool| {
        if let Some(x) = g.bool(k) {
            *v = x;
        }
    };
    let st = |k: &str, v: &mut String| {
        if let Some(x) = g.str(k) {
            *v = x.to_string();
        }
    };
    bo("EnableLensFlares", &mut lens.flares);
    bo("EnableLensGhosts", &mut lens.ghosts);
    bo("EnablePseudoFlares", &mut lens.pseudo);
    f("LensFlareScale", &mut lens.scale);
    f("LensFlareBright", &mut lens.bright);
    f("PseudoFlareBright", &mut lens.pseudo_bright);
    st("LensFlarePreset", &mut lens.flare_preset);
    st("LensGhostPreset", &mut lens.ghost_preset);
    st("PseudoFlarePreset", &mut lens.pseudo_preset);
    // SE_NO_BLOOM wins over the saved setting.
    if std::env::var_os("SE_NO_BLOOM").is_none() {
        bo("EnableBloom", &mut bloom.enabled);
    }
    f("BloomBright", &mut bloom.bright);
    f("BloomThreshold", &mut bloom.threshold);
}
