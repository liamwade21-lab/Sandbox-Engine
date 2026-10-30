//! Persistent user settings, stored in SpaceEngine's own `.cfg` syntax at
//! `%APPDATA%/SpaceEngine-rs/settings.cfg` (or `./settings.cfg` elsewhere), never in the SE install.
//!
//! As SE's `config/main-user.cfg` over `main-def.cfg`: the defaults come from the install's main-def.cfg
//! (each module reads its keys), and this file holds the user's values by SE's key names, in SE's blocks
//! (`PostEffects`, `Export`, the graphics preset's keys). SE creates its user file on first run ("Creating
//! user config file") and picks the graphics preset from the video memory on a hardware or version change
//! (FUN_14030b0e0, `DetectVersionChange`); the port does the same on first run or when the saved `GPU` or
//! `Version` differs. "Reset settings" deletes the file and restarts.

use std::path::PathBuf;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::bloom::BloomSettings;
use crate::camera::{FlyCamera, MainCamera};
use crate::color_correction::{ColorCorrection, FinalImage};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::exposure::{ExposureComp, PhotoMode};
use crate::graphics::{GraphicsSettings, Quality};
use crate::i18n::I18n;
use crate::lensflare::LensFlareSettings;
use crate::music::MusicState;
use crate::overlay::{KindSet, MarkerKind, OverlaySettings, SkySwitches};
use crate::screenshot::ScreenshotSettings;
use crate::stars::StarSettings;
use crate::switches::RenderSwitches;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ResetRequest>()
            .add_systems(PostStartup, load_settings)
            .add_systems(Update, (apply_pending_color, save_when_changed, reset_settings).chain())
            .add_systems(Last, restart_after_reset.after(crate::session::save_on_exit));
    }
}

/// "Reset settings" confirmed in the settings window.
#[derive(Resource, Default)]
pub struct ResetRequest(pub bool);

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

/// Every setting persisted, live.
#[derive(SystemParam)]
struct Live<'w, 's> {
    stars: ResMut<'w, StarSettings>,
    overlay: ResMut<'w, OverlaySettings>,
    cam: ResMut<'w, FlyCamera>,
    music: Option<ResMut<'w, MusicState>>,
    sound: ResMut<'w, crate::sound::SoundSettings>,
    i18n: ResMut<'w, I18n>,
    skin: ResMut<'w, crate::skin::Skin>,
    gfx: ResMut<'w, GraphicsSettings>,
    lens: ResMut<'w, LensFlareSettings>,
    bloom: ResMut<'w, BloomSettings>,
    display: Res<'w, crate::display::DisplaySettings>,
    switches: ResMut<'w, RenderSwitches>,
    post: (ResMut<'w, PhotoMode>, ResMut<'w, ExposureComp>, ResMut<'w, FinalImage>, ResMut<'w, ScreenshotSettings>),
    planets: (Option<ResMut<'w, crate::terrain::SelfEmission>>, Option<ResMut<'w, crate::terrain::WaterDisplay>>),
    deep_sky: (ResMut<'w, crate::procgalaxies::ProcSettings>, Option<ResMut<'w, crate::clusters::ClusterSettings>>),
    color: Query<'w, 's, &'static ColorCorrection, With<MainCamera>>,
    adapter: Option<Res<'w, bevy::render::renderer::RenderAdapterInfo>>,
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
    /// `PointFlareBright`, `PointFlareDecay`, `LinkScaleToScreenRes`.
    flare: (f32, f32, bool),
    fov: f32,
    /// SE's `AutoHorizon` (Controls).
    auto_horizon: bool,
    orbits: bool,
    labels_mode: u8,
    markers: bool,
    /// SE's per-kind `ShowOrbits*`, `ShowLabels*`, `ShowMarkers*`.
    kinds: [KindSet; 3],
    /// SE's constellation and grid switches ([`SkySwitches::KEYS`]).
    sky: [bool; 12],
    /// SE's switches for what is drawn around planets ([`RenderSwitches::KEYS`]).
    render: [bool; 9],
    /// `ShowProcGalaxies`, `ShowProcClusters`, `ShowProcNebulae`, `ShowCatClusters`.
    deep_sky: [bool; 4],
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
    photo_mode: u8,
    /// SE's `ExposureComp` (negated: positive darker).
    exposure_comp: f32,
    tone: String,
    /// `Brightness`, `Contrast`, `Saturation`, `Gamma`.
    bcsg: [f32; 4],
    dithering: bool,
    /// `EnableSelfEmission`, `SelfEmissionBright`.
    self_emission: (bool, f32),
    show_water: bool,
    shots: ScreenshotSettings,
    gpu: String,
}

impl Snapshot {
    fn capture(l: &Live) -> Self {
        let (photo, exposure, fi, shots) = (&l.post.0, &l.post.1, &l.post.2, &l.post.3);
        let (s, o) = (&*l.stars, &*l.overlay);
        Self {
            language: l.i18n.lang().to_string(),
            skin: l.skin.name.clone(),
            gfx_preset: l.gfx.preset.clone(),
            gfx: l.gfx.quality.clone(),
            lens: l.lens.clone(),
            bloom: l.bloom.clone(),
            display: l.display.clone(),
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
            flare: (s.flare_bright, s.flare_decay, s.link_screen_res),
            fov: l.cam.fov_deg,
            auto_horizon: l.cam.auto_horizon,
            orbits: o.orbits,
            labels_mode: o.labels_mode,
            markers: o.markers,
            kinds: [o.orbit_kinds, o.label_kinds, o.marker_kinds],
            sky: o.sky.saved(),
            render: l.switches.saved(),
            deep_sky: [l.deep_sky.0.galaxies, l.deep_sky.0.clusters, l.deep_sky.0.nebulae, l.deep_sky.1.as_ref().is_none_or(|c| c.show_cat)],
            star_label_mag: o.star_label_mag,
            show_fps: o.show_fps,
            info_full: o.info_full,
            music: l.music.as_ref().map(|m| m.enabled).unwrap_or(true),
            music_volume: l.music.as_ref().map(|m| m.volume).unwrap_or(crate::music::DEFAULT_VOLUME),
            sound_volume: l.sound.volume,
            photo_mode: photo.0,
            exposure_comp: -exposure.stops,
            tone: fi.tone_name().to_string(),
            bcsg: l.color.iter().next().map(|c| c.values()).unwrap_or(ColorCorrection::default().values()),
            dithering: fi.dithering,
            self_emission: l.planets.0.as_ref().map_or((true, 1.0), |e| (e.enabled, e.bright)),
            show_water: l.planets.1.as_ref().is_none_or(|w| w.show),
            shots: (**shots).clone(),
            gpu: l.adapter.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        }
    }

    fn to_cfg(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        format!(
            "// SpaceEngine-rs user settings (overrides of the install's config/main-def.cfg)\nVersion \"{}\"\nSettings\n{{\n\tStarMagnLimit   {}\n\tPointOverbright {}\n\tPointScale      {}\n\tStarColorSaturation {}\n\tPointDesaturation {}\n\tStarColorShift  {}\n\tSunIllumSaturation {}\n\tIllumSaturationAuto {}\n\tStarPointMethod {}\n\tMotionBlurLength {}\n\tPointFlareBright {}\n\tPointFlareDecay {}\n\tLinkScaleToScreenRes {}\n\tFOV             {}\n\tAutoHorizon     {}\n\tShowOrbits      {}\n\tLabelsMode      {}\n\tShowMarkers     {}\n\tStarNameMagn    {}\n\tFPS             {}\n\tTextMode        {}\n\tMusic           {}\n\tMusicVolume     {}\n\tSoundVolume     {}\n\tLanguage        \"{}\"\n\tSkin            \"{}\"\n\tEnableSelfEmission {}\n\tSelfEmissionBright {}\n\tShowWater       {}\n{}}}\n",
            env!("CARGO_PKG_VERSION"),
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
            self.flare.0,
            self.flare.1,
            b(self.flare.2),
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
            b(self.self_emission.0),
            self.self_emission.1,
            b(self.show_water),
            self.filters_cfg()
        ) + &self.post_cfg()
            + &self.graphics_cfg()
            + &self.display.to_cfg()
    }

    /// The per-kind overlay filters and switches, by SE's keys.
    fn filters_cfg(&self) -> String {
        let mut out = String::new();
        for (set, filter) in self.kinds.iter().zip(["ShowOrbits", "ShowLabels", "ShowMarkers"]) {
            for k in MarkerKind::ALL {
                out += &format!("\t{filter}{} {}\n", k.key(), set.has(k));
            }
        }
        for (key, on) in SkySwitches::KEYS.iter().zip(self.sky) {
            out += &format!("\t{key} {on}\n");
        }
        for (key, on) in RenderSwitches::KEYS.iter().zip(self.render) {
            out += &format!("\t{key} {on}\n");
        }
        for (key, on) in DEEP_SKY_KEYS.iter().zip(self.deep_sky) {
            out += &format!("\t{key} {on}\n");
        }
        out
    }

    /// SE's `PostEffects` (photo mode, exposure, tone shader, colour correction) and `Export` blocks.
    fn post_cfg(&self) -> String {
        let [br, co, sa, ga] = self.bcsg;
        let s = &self.shots;
        format!(
            "PostEffects\n{{\n\tPhotoMode         {}\n\tExposureComp      {}\n\tToneMappingShader \"{}\"\n\tBrightness        {br}\n\tContrast          {co}\n\tSaturation        {sa}\n\tGamma             {ga}\n\tDithering         {}\n}}\nExport\n{{\n\tScreenshotFormat  \"{}\"\n\tJpegQuality       {}\n\tSaveAlpha         {}\n}}\n",
            self.photo_mode, self.exposure_comp, self.tone, self.dithering, s.format, s.jpeg_quality, s.save_alpha
        )
    }

    fn graphics_cfg(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        let l = &self.lens;
        format!(
            "Graphics\n{{\n\tPreset            \"{}\"\n\tGPU               \"{}\"\n{}\tEnableLensFlares  {}\n\tEnableLensGhosts  {}\n\tEnablePseudoFlares {}\n\tLensFlareScale    {}\n\tLensFlareBright   {}\n\tPseudoFlareBright {}\n\tLensFlarePreset   \"{}\"\n\tLensGhostPreset   \"{}\"\n\tPseudoFlarePreset \"{}\"\n\tEnableBloom       {}\n\tBloomBright       {}\n\tBloomThreshold    {}\n}}\n",
            self.gfx_preset,
            self.gpu,
            self.gfx.cfg_lines(),
            b(l.flares),
            b(l.ghosts),
            b(l.pseudo),
            l.scale,
            l.bright,
            l.pseudo_bright,
            l.flare_preset,
            l.ghost_preset,
            l.pseudo_preset,
            b(self.bloom.enabled),
            self.bloom.bright,
            self.bloom.threshold
        )
    }
}

/// SE's deep-sky filters kept in the `Settings` block.
const DEEP_SKY_KEYS: [&str; 4] = ["ShowProcGalaxies", "ShowProcClusters", "ShowProcNebulae", "ShowCatClusters"];

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
    fn capture(&self, l: &Live) -> Snapshot {
        let mut snap = Snapshot::capture(l);
        if let Some(m) = self.keep_music {
            snap.music = m;
        }
        if let Some(lang) = &self.keep_lang {
            snap.language = lang.clone();
        }
        snap
    }
}

/// `Brightness`/`Contrast`/`Saturation`/`Gamma` read before the camera had its colour correction.
#[derive(Resource)]
struct PendingColor([f32; 4]);

fn apply_pending_color(mut commands: Commands, pending: Option<Res<PendingColor>>, mut cams: Query<&mut ColorCorrection, With<MainCamera>>) {
    let Some(p) = pending else { return };
    let [br, co, sa, ga] = p.0;
    let mut done = false;
    for mut c in &mut cams {
        c.bcsg = ColorCorrection::new(br, co, sa, ga).bcsg;
        done = true;
    }
    if done {
        commands.remove_resource::<PendingColor>();
    }
}

fn load_settings(mut commands: Commands, cfg: Res<EngineConfig>, data: Res<SeData>, mut live: Live) {
    if scripted(&cfg) {
        return;
    }
    let no_music = std::env::var_os("SE_NO_MUSIC").is_some();
    // SE_LANG (testing) wins over the saved language, as SE_NO_MUSIC does for music.
    let lang_forced = std::env::var_os("SE_LANG").is_some();
    // SE_NO_PROCGAL / SE_PROC (testing) win over the saved deep-sky filters.
    let proc_forced = std::env::var_os("SE_NO_PROCGAL").is_some() || std::env::var_os("SE_PROC").is_some();
    let mut saved_music = true;
    let mut saved_lang: Option<String> = None;
    let text = std::fs::read_to_string(settings_path()).ok();
    // FUN_14030b0e0: the optimal preset on first run or a hardware / version change.
    let doc = text.as_deref().map(se_core::sc::parse);
    let gpu_now = live.adapter.as_ref().map(|a| a.name.clone()).unwrap_or_default();
    let saved_gpu = doc.as_ref().and_then(|d| d.sub("Graphics")).and_then(|g| g.str("GPU").map(str::to_string));
    let saved_version = doc.as_ref().and_then(|d| d.str("Version").map(str::to_string));
    // A file from an older port has neither key: not a change (its saved preset stays), but it is
    // rewritten at once so they are there from then on.
    let changed = doc.is_none()
        || saved_gpu.as_deref().is_some_and(|g| g != gpu_now)
        || saved_version.as_deref().is_some_and(|v| v != env!("CARGO_PKG_VERSION"));
    let rewrite = changed || saved_version.is_none() || saved_gpu.is_none();
    if let Some(doc) = &doc {
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
            let stars = &mut *live.stars;
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
            f("PointFlareBright", &mut stars.flare_bright);
            f("PointFlareDecay", &mut stars.flare_decay);
            bo("LinkScaleToScreenRes", &mut stars.link_screen_res);
            if let Some(m) = b.f64("StarPointMethod") {
                stars.point_style = crate::stars::PointStyle::from_method(m as u32);
            }
            f("FOV", &mut live.cam.fov_deg);
            bo("AutoHorizon", &mut live.cam.auto_horizon);
            let o = &mut *live.overlay;
            bo("ShowOrbits", &mut o.orbits);
            bo("ShowMarkers", &mut o.markers);
            match (b.f64("LabelsMode"), b.bool("ShowLabels")) {
                (Some(m), _) => o.labels_mode = m.clamp(0.0, 2.0) as u8,
                // The port's earlier ShowLabels / ShowStarNames.
                (None, Some(on)) => o.labels_mode = on as u8,
                _ => {}
            }
            if let Some(on) = b.bool("ShowStarNames") {
                o.label_kinds.set(MarkerKind::Star, on);
            }
            for k in MarkerKind::ALL {
                for (filter, set) in [("ShowOrbits", &mut o.orbit_kinds), ("ShowLabels", &mut o.label_kinds), ("ShowMarkers", &mut o.marker_kinds)] {
                    if let Some(on) = b.bool(&format!("{filter}{}", k.key())) {
                        set.set(k, on);
                    }
                }
            }
            for key in SkySwitches::KEYS {
                if let Some(on) = b.bool(key) {
                    o.sky.set_var(key, on);
                }
            }
            for key in RenderSwitches::KEYS {
                if let Some(on) = b.bool(key) {
                    live.switches.set_var(key, on);
                }
            }
            if !proc_forced {
                let p = &mut *live.deep_sky.0;
                bo("ShowProcGalaxies", &mut p.galaxies);
                bo("ShowProcClusters", &mut p.clusters);
                bo("ShowProcNebulae", &mut p.nebulae);
            }
            if let Some(c) = live.deep_sky.1.as_mut() {
                bo("ShowCatClusters", &mut c.show_cat);
            }
            f("StarNameMagn", &mut o.star_label_mag);
            bo("FPS", &mut o.show_fps);
            if let Some(m) = b.f64("TextMode") {
                o.info_full = m >= 2.0;
            }
            if let Some(e) = live.planets.0.as_mut() {
                bo("EnableSelfEmission", &mut e.enabled);
                f("SelfEmissionBright", &mut e.bright);
            }
            if let (Some(w), Some(show)) = (live.planets.1.as_mut(), b.bool("ShowWater")) {
                w.show = show;
                w.level = if show { 1.0 } else { 0.0 };
            }
            bo("Music", &mut saved_music);
            if let Some(s) = b.str("Skin") {
                live.skin.name = s.to_string();
            }
            saved_lang = b.str("Language").map(str::to_string);
            if let Some(v) = b.f64("SoundVolume") {
                live.sound.volume = (v / 100.0).clamp(0.0, 1.0) as f32;
            }
            if let (false, Some(l)) = (lang_forced, &saved_lang) {
                live.i18n.set_language(&data.vfs, l);
            }
            if let Some(m) = live.music.as_mut() {
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
        if let Some(p) = doc.sub("PostEffects") {
            load_post(p, &mut live, &mut commands);
        }
        if let Some(e) = doc.sub("Export") {
            let s = &mut *live.post.3;
            if let Some(f) = e.str("ScreenshotFormat").map(str::to_ascii_lowercase).filter(|f| crate::screenshot::FORMATS.contains(&f.as_str())) {
                s.format = f;
            }
            if let Some(q) = e.f64("JpegQuality") {
                s.jpeg_quality = q.clamp(1.0, 100.0) as u8;
            }
            s.save_alpha = e.bool("SaveAlpha").unwrap_or(s.save_alpha);
        }
        if let Some(g) = doc.sub("Graphics") {
            load_graphics(g, &mut live.gfx, &mut live.lens, &mut live.bloom);
        }
    }
    if changed {
        if let Some(mb) = crate::graphics::video_memory_mb() {
            let preset = crate::graphics::optimal_preset(mb);
            info!("{} - loading the {preset} graphics preset ({mb} MB of video memory)", if doc.is_none() { "first run" } else { "hardware or version change detected" });
            live.gfx.pick_preset(preset);
        }
    }
    let mut saved = Saved {
        last: Snapshot::capture(&live),
        timer: 0.0,
        keep_music: no_music.then_some(saved_music),
        keep_lang: lang_forced.then(|| saved_lang.clone().unwrap_or_else(|| se_core::locale::DEFAULT_LANG.to_string())),
    };
    saved.last = saved.capture(&live);
    if rewrite {
        // A first run or a hardware / version change writes the file at once (SE's "Creating user
        // config file", "Performing full update of the config file").
        saved.last.gpu.push('\0');
        saved.timer = 2.0;
    }
    commands.insert_resource(saved);
}

/// SE's `PostEffects`: photo mode, exposure, tone shader and the final colour correction.
fn load_post(p: &se_core::sc::Block, live: &mut Live, commands: &mut Commands) {
    if let Some(m) = p.f64("PhotoMode") {
        *live.post.0 = PhotoMode::new(m.max(0.0) as u8);
    }
    if let Some(v) = p.f64("ExposureComp") {
        live.post.1.set(-v as f32);
    }
    if let Some(t) = p.str("ToneMappingShader").and_then(crate::color_correction::tone_index) {
        live.post.2.tone = t;
    }
    live.post.2.dithering = p.bool("Dithering").unwrap_or(live.post.2.dithering);
    let d = ColorCorrection::default().values();
    let v = |k: &str, i: usize| p.f64(k).map_or(d[i], |x| x as f32);
    commands.insert_resource(PendingColor([v("Brightness", 0), v("Contrast", 1), v("Saturation", 2), v("Gamma", 3)]));
}

fn save_when_changed(time: Res<Time>, saved: Option<ResMut<Saved>>, live: Live) {
    let Some(mut saved) = saved else { return };
    saved.timer += time.delta_secs();
    if saved.timer < 2.0 {
        return;
    }
    saved.timer = 0.0;
    let now = saved.capture(&live);
    if now != saved.last {
        let path = settings_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match write_atomic(&path, &now.to_cfg()) {
            Ok(()) => debug!("saved settings to {}", path.display()),
            Err(e) => warn!("cannot save settings to {}: {e}", path.display()),
        }
        saved.last = now;
    }
}

/// Writes `text` to `<path>.tmp`, then renames it over `path`, so a crash mid-write never leaves a
/// truncated file.
pub fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Set by "Reset settings": the port starts again once this one has saved its session and is exiting.
#[derive(Resource)]
struct RestartOnExit;

/// "Reset settings": the user file goes, and the port restarts on main-def.cfg's values (SE restarts too).
fn reset_settings(mut commands: Commands, mut req: ResMut<ResetRequest>, mut exit: MessageWriter<AppExit>) {
    if !req.0 {
        return;
    }
    req.0 = false;
    commands.remove_resource::<Saved>();
    let path = settings_path();
    if let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        warn!("cannot remove {}: {e}", path.display());
    }
    commands.insert_resource(RestartOnExit);
    exit.write(AppExit::Success);
}

/// The new process starts after the session was written (`session.rs`, also in `Last`).
fn restart_after_reset(restart: Option<Res<RestartOnExit>>) {
    if restart.is_none() {
        return;
    }
    match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn()) {
        Ok(_) => info!("settings reset; restarting"),
        Err(e) => warn!("settings reset; cannot restart: {e}"),
    }
}

/// The `Graphics` block: a preset name, then its values (a "Custom" preset keeps the individual ones).
fn load_graphics(g: &se_core::sc::Block, gfx: &mut GraphicsSettings, lens: &mut LensFlareSettings, bloom: &mut BloomSettings) {
    if let Some(p) = g.str("Preset") {
        gfx.apply_preset(p);
        gfx.preset = p.to_string();
    }
    // The saved values over the preset's: the block holds the whole preset (SE's main-user.cfg does too).
    let mut text = String::from("Graphics\n{\n");
    for item in &g.items {
        if let Some(v) = item.args.first() {
            let v = match v {
                se_core::sc::Value::Str(s) => format!("\"{s}\""),
                se_core::sc::Value::Num(x) => x.to_string(),
                se_core::sc::Value::Ident(s) => s.clone(),
                se_core::sc::Value::Tuple(_) => continue,
            };
            text.push_str(&format!("{} {v}\n", item.key));
        }
    }
    text.push_str("}\n");
    let mut q = Quality::from_cfg(&text, &gfx.quality);
    // Only preset keys: the window, lens flare and bloom keys of the block are not part of it.
    let preset_keys: Vec<String> = gfx.presets.first().map(|(_, p)| p.keys.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default();
    if !preset_keys.is_empty() {
        q.keys.retain(|(k, _)| preset_keys.contains(k));
    }
    gfx.quality = q;
    gfx.loaded = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_block_round_trips_the_whole_preset() {
        let preset = Quality::from_cfg("Memory\n{\n\tVideoMemorySize 4096\n}\nTextureRendering\n{\n\tSolidResolution 0.85\n\tEnableGalaxyFBO true\n}\n", &Quality::default());
        let mut gfx = GraphicsSettings { presets: vec![("High".into(), preset.clone())], ..default() };
        gfx.apply_preset("High");
        gfx.quality.msaa = 4;
        gfx.quality.keys[1].1 = "0.5".into();
        let block = format!("Graphics\n{{\n\tPreset \"Custom\"\n\tGPU \"x\"\n{}\tEnableBloom false\n}}\n", gfx.quality.cfg_lines());
        let doc = se_core::sc::parse(&block);
        let mut back = GraphicsSettings { presets: gfx.presets.clone(), ..default() };
        let (mut lens, mut bloom) = (LensFlareSettings::default(), BloomSettings::default());
        load_graphics(doc.sub("Graphics").unwrap(), &mut back, &mut lens, &mut bloom);
        assert_eq!(back.preset, "Custom");
        assert_eq!(back.quality, gfx.quality);
    }
}
