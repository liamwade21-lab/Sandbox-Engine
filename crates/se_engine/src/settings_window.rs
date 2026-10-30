//! SE's settings dialog (F4; Ctrl+F4 and the Graphics button open it on Graphics): tabs Graphics (groups
//! "Graphics quality", anti-aliasing, post-effects, tone mapping, lens flares, point sources, planets,
//! volumetrics), Camera, Display, Interface and Sound, and "Reset settings" ("Really reset settings?",
//! "This will reset all settings except controls and restart SpaceEngine"). SE's Projection / 3D and
//! Controls tabs have no counterpart in the port (stereo and VR projections, key binding editor) and are
//! left out.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::bloom::BloomSettings;
use crate::camera::{FlyCamera, MainCamera};
use crate::color_correction::{ColorCorrection, FinalImage, TONE_SHADERS};
use crate::exposure::{ExposureComp, PhotoMode};
use crate::graphics::{GraphicsSettings, GraphicsUi};
use crate::i18n::I18n;
use crate::lensflare::{FlarePresets, LensFlareSettings};
use crate::stars::StarSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Graphics,
    Camera,
    Display,
    Interface,
    Sound,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Graphics, Tab::Camera, Tab::Display, Tab::Interface, Tab::Sound];

    fn label(self) -> &'static str {
        match self {
            Tab::Graphics => "Graphics",
            Tab::Camera => "Camera",
            Tab::Display => "Display",
            Tab::Interface => "Interface",
            Tab::Sound => "Sound",
        }
    }
}

#[derive(Default)]
struct WindowState {
    tab: Tab,
    /// `GraphicsUi::open` last frame, to switch to the Graphics tab when it opens.
    graphics_was_open: bool,
    confirm_reset: bool,
}

pub struct SettingsWindowPlugin;

impl Plugin for SettingsWindowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(bevy_egui::EguiPrimaryContextPass, settings_window.in_set(crate::skin::UiSet::Draw));
    }
}

/// A combo box over `names`; true when the choice changed.
fn combo(ui: &mut egui::Ui, id: &str, label: &str, value: &mut usize, names: &[&str], i18n: &I18n) -> bool {
    let before = *value;
    ui.horizontal(|ui| {
        ui.label(i18n.tr(label));
        egui::ComboBox::from_id_salt(id).selected_text(i18n.tr(names.get(*value).copied().unwrap_or(""))).show_ui(ui, |ui| {
            for (i, n) in names.iter().enumerate() {
                ui.selectable_value(value, i, i18n.tr(n));
            }
        });
    });
    *value != before
}

fn group(ui: &mut egui::Ui, title: &str, i18n: &I18n, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(i18n.tr(title)).default_open(true).show(ui, add);
}

fn settings_window(
    mut contexts: EguiContexts,
    mut hud: ResMut<crate::hud::HudState>,
    mut gfx_ui: ResMut<GraphicsUi>,
    mut state: Local<WindowState>,
    (mut i18n, data, mut skin): (ResMut<I18n>, Res<crate::data::SeData>, ResMut<crate::skin::Skin>),
    (mut stars, mut overlay, mut cam): (ResMut<StarSettings>, ResMut<crate::overlay::OverlaySettings>, ResMut<FlyCamera>),
    (mut music, mut sound): (Option<ResMut<crate::music::MusicState>>, ResMut<crate::sound::SoundSettings>),
    (mut gs, mut lens, flare_presets, mut bloom, mut display): (
        ResMut<GraphicsSettings>,
        ResMut<LensFlareSettings>,
        Option<Res<FlarePresets>>,
        ResMut<BloomSettings>,
        ResMut<crate::display::DisplaySettings>,
    ),
    (mut exposure, mut photo, mut fi, mut cc): (ResMut<ExposureComp>, ResMut<PhotoMode>, ResMut<FinalImage>, Query<&mut ColorCorrection, With<MainCamera>>),
    (mut emission, mut water, mut shots, mut reset): (
        Option<ResMut<crate::terrain::SelfEmission>>,
        Option<ResMut<crate::terrain::WaterDisplay>>,
        ResMut<crate::screenshot::ScreenshotSettings>,
        ResMut<crate::settings::ResetRequest>,
    ),
) -> Result {
    if gfx_ui.open && !state.graphics_was_open {
        state.tab = Tab::Graphics;
    }
    state.graphics_was_open = gfx_ui.open;
    if !hud.show_settings && !gfx_ui.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let mut open = true;
    let mut pick_lang: Option<String> = None;
    let mut pick_preset: Option<String> = None;
    // Edited on copies; written back only on a change, so the settings are not marked changed every frame.
    let mut q = gs.quality.clone();
    let mut l = lens.clone();
    let mut b = bloom.clone();
    let mut d = display.clone();
    let mut f = fi.clone();
    let mut ph = photo.0 as usize;
    let mut ex = *exposure;
    let mut sh = shots.clone();
    let mut em = emission.as_deref().copied();
    let mut wa = water.as_deref().map(|w| w.show);
    let bcsg0 = cc.iter().next().map(|c| c.values());
    let mut bcsg = bcsg0;
    let WindowState { tab, confirm_reset: confirm, .. } = &mut *state;
    egui::Window::new(crate::skin::title(ctx, i18n.tr("Settings"))).open(&mut open).default_width(380.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            for t in Tab::ALL {
                ui.selectable_value(tab, t, i18n.tr(t.label()));
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| match *tab {
            Tab::Graphics => {
                ui.horizontal(|ui| {
                    ui.label(i18n.tr("Graphics quality"));
                    egui::ComboBox::from_id_salt("gfx_preset").selected_text(i18n.tr(&gs.preset)).show_ui(ui, |ui| {
                        for (n, _) in &gs.presets {
                            if ui.selectable_label(*n == gs.preset, i18n.tr(n)).clicked() {
                                pick_preset = Some(n.clone());
                            }
                        }
                    });
                });
                group(ui, "Quality", &i18n, |ui| {
                    ui.add(egui::Slider::new(&mut q.anisotropy, 0..=16).text(i18n.tr("Anisotropic filtering")));
                    ui.add(egui::Slider::new(&mut q.land_lod, -3.0..=1.0).step_by(0.5).text(i18n.tr("Landscape LOD")));
                    ui.add(egui::Slider::new(&mut q.max_tiles_per_frame, 1..=16).text(i18n.tr("Terrain tiles per frame")));
                    ui.collapsing(i18n.tr("Other preset values"), |ui| {
                        for (k, v) in &q.keys {
                            ui.label(format!("{k}  {v}"));
                        }
                    });
                });
                group(ui, "Anti-aliasing", &i18n, |ui| {
                    egui::ComboBox::from_label("MSAA").selected_text(format!("{}x", q.msaa.max(1))).show_ui(ui, |ui| {
                        for v in [0u32, 2, 4, 8] {
                            ui.selectable_value(&mut q.msaa, v, format!("{}x", v.max(1)));
                        }
                    });
                    ui.checkbox(&mut q.fxaa, "FXAA");
                });
                group(ui, "Post-effects", &i18n, |ui| {
                    combo(ui, "photo_mode", "Photo mode", &mut ph, &PhotoMode::NAMES, &i18n);
                    let (lo, hi) = (ex.min, ex.max);
                    ui.add(egui::Slider::new(&mut ex.stops, lo..=hi).step_by(0.25).text(i18n.tr("Exposure compensation")));
                    if let Some(v) = bcsg.as_mut() {
                        ui.add(egui::Slider::new(&mut v[0], 0.0..=2.0).text(i18n.tr("Brightness")));
                        ui.add(egui::Slider::new(&mut v[1], 0.0..=2.0).text(i18n.tr("Contrast")));
                        ui.add(egui::Slider::new(&mut v[2], 0.0..=3.0).text(i18n.tr("Saturation")));
                        ui.add(egui::Slider::new(&mut v[3], 0.2..=3.0).text(i18n.tr("Gamma")));
                    }
                    ui.checkbox(&mut f.dithering, i18n.tr("Dithering"));
                    ui.checkbox(&mut b.enabled, i18n.tr("Bloom"));
                    ui.add(egui::Slider::new(&mut b.bright, 0.0..=2.0).text(i18n.tr("Bloom brightness")));
                    ui.add(egui::Slider::new(&mut b.threshold, 0.0..=10.0).text(i18n.tr("Bloom threshold")));
                });
                group(ui, "Tone mapping", &i18n, |ui| {
                    combo(ui, "tone_shader", "Tone mapping", &mut f.tone, &TONE_SHADERS, &i18n);
                });
                group(ui, "Lens flares", &i18n, |ui| {
                    ui.checkbox(&mut l.flares, i18n.tr("Lens flares"));
                    ui.checkbox(&mut l.ghosts, i18n.tr("Lens ghosts"));
                    ui.checkbox(&mut l.pseudo, i18n.tr("Pseudo lens flares"));
                    ui.add(egui::Slider::new(&mut l.scale, 0.1..=3.0).text(i18n.tr("Lens flare size")));
                    ui.add(egui::Slider::new(&mut l.bright, 0.0..=3.0).text(i18n.tr("Lens flare brightness")));
                    ui.add(egui::Slider::new(&mut l.pseudo_bright, 0.0..=3.0).text(i18n.tr("Pseudo lens flare brightness")));
                    if let Some(p) = &flare_presets {
                        let mut pick = |label: &str, value: &mut String, names: Vec<&String>| {
                            egui::ComboBox::from_label(i18n.tr(label)).selected_text(value.as_str()).show_ui(ui, |ui| {
                                for n in names {
                                    if ui.selectable_label(n == value, n.as_str()).clicked() {
                                        *value = n.clone();
                                    }
                                }
                            });
                        };
                        pick("Lens flare style", &mut l.flare_preset, p.flares.keys().collect());
                        pick("Lens ghosts style", &mut l.ghost_preset, p.ghosts.keys().collect());
                        pick("Pseudo lens flare style", &mut l.pseudo_preset, p.pseudo.keys().collect());
                    }
                });
                group(ui, "Point sources", &i18n, |ui| {
                    ui.add(egui::Slider::new(&mut stars.mag_limit, 0.0..=16.0).text(i18n.tr("Star mag limit")));
                    ui.add(egui::Slider::new(&mut stars.overbright, -3.0..=6.0).text(i18n.tr("Stars overbright")));
                    ui.add(egui::Slider::new(&mut stars.point_scale, 0.25..=4.0).text(i18n.tr("Points scale")));
                    ui.add(egui::Slider::new(&mut stars.saturation, 0.0..=3.0).text(i18n.tr("Stars color saturation")));
                    ui.add(egui::Slider::new(&mut stars.desaturation, 0.0..=1.0).text(i18n.tr("Desaturate dim stars")));
                    ui.add(egui::Slider::new(&mut stars.color_shift, -1.0..=1.0).text(i18n.tr("Stars color shift")));
                    ui.horizontal(|ui| {
                        ui.label(i18n.tr("Star points style"));
                        egui::ComboBox::from_id_salt("star_points").selected_text(i18n.tr(stars.point_style.label())).show_ui(ui, |ui| {
                            for p in crate::stars::PointStyle::ALL {
                                ui.selectable_value(&mut stars.point_style, p, i18n.tr(p.label()));
                            }
                        });
                    });
                    if stars.point_style != crate::stars::PointStyle::Sprites {
                        ui.add(egui::Slider::new(&mut stars.blur_length, 0.25..=4.0).logarithmic(true).text(i18n.tr("motion blur length")));
                    }
                });
                group(ui, "Planets", &i18n, |ui| {
                    ui.horizontal(|ui| {
                        let auto = stars.illum_saturation_auto;
                        ui.add_enabled(!auto, egui::Slider::new(&mut stars.illum_saturation, 0.0..=2.0).text(i18n.tr("Illumination saturation")));
                        ui.checkbox(&mut stars.illum_saturation_auto, i18n.tr("Auto"));
                    });
                    if let Some(e) = em.as_mut() {
                        ui.checkbox(&mut e.enabled, i18n.tr("Self-emission"));
                        ui.add_enabled(e.enabled, egui::Slider::new(&mut e.bright, 0.0..=4.0).text(i18n.tr("Self-emission brightness")));
                    }
                    if let Some(w) = wa.as_mut() {
                        ui.checkbox(w, i18n.tr("Water"));
                    }
                });
                group(ui, "Volumetrics", &i18n, |ui| {
                    ui.add(egui::Slider::new(&mut q.volume_resolution, 0.1..=1.0).text(i18n.tr("Volumetric quality")));
                });
            }
            Tab::Camera => {
                ui.add(egui::Slider::new(&mut cam.fov_deg, crate::camera::MIN_FOV_DEG..=crate::camera::MAX_FOV_DEG).logarithmic(true).text(i18n.tr("field of view")));
                ui.checkbox(&mut cam.auto_horizon, i18n.tr("Auto horizon on planets"));
            }
            Tab::Display => {
                let modes = ["Windowed", "Fullscreen window", "Fullscreen"];
                let mut m = d.fullscreen as usize;
                if combo(ui, "window_mode", "Window mode", &mut m, &modes, &i18n) {
                    d.fullscreen = m as u8;
                }
                let mut v = d.vsync as usize;
                if combo(ui, "vsync", "Vertical synchronization", &mut v, &["Disabled", "Enabled", "Auto"], &i18n) {
                    d.vsync = v as u8;
                }
                ui.separator();
                let mut fmt = crate::screenshot::FORMATS.iter().position(|x| *x == sh.format).unwrap_or(0);
                if combo(ui, "shot_format", "Screenshots format", &mut fmt, &crate::screenshot::FORMATS, &i18n) {
                    sh.format = crate::screenshot::FORMATS[fmt].to_string();
                }
                ui.add_enabled(sh.format == "jpg", egui::Slider::new(&mut sh.jpeg_quality, 1..=100).text(i18n.tr("JPEG quality")));
                ui.add_enabled(sh.format != "jpg", egui::Checkbox::new(&mut sh.save_alpha, i18n.tr("Save transparency")));
            }
            Tab::Interface => {
                ui.horizontal(|ui| {
                    ui.label(i18n.tr("Language"));
                    egui::ComboBox::from_id_salt("ui_language").selected_text(i18n.name_of(i18n.lang())).show_ui(ui, |ui| {
                        for (code, name) in &i18n.languages {
                            if ui.selectable_label(code == i18n.lang(), name).clicked() {
                                pick_lang = Some(code.clone());
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(i18n.tr("Interface skin"));
                    egui::ComboBox::from_id_salt("ui_skin").selected_text(skin.name.clone()).show_ui(ui, |ui| {
                        for name in skin.available.clone() {
                            if ui.selectable_label(name == skin.name, &name).clicked() {
                                skin.name = name;
                            }
                        }
                    });
                });
                ui.separator();
                crate::hud::overlay_filters(ui, &mut overlay, &i18n);
                ui.checkbox(&mut overlay.selection_pointer, i18n.tr("Selection pointer"));
                ui.checkbox(&mut overlay.show_fps, i18n.tr("Show FPS"));
                ui.add(egui::Slider::new(&mut overlay.star_label_mag, -2.0..=6.0).text(i18n.tr("star name magnitude limit")));
            }
            Tab::Sound => {
                // SE's volumes run 0..100.
                if let Some(m) = music.as_mut() {
                    ui.checkbox(&mut m.enabled, i18n.tr("Music"));
                    let mut v = (m.volume * 100.0).round();
                    if ui.add(egui::Slider::new(&mut v, 0.0..=100.0).text(i18n.tr("Music volume"))).changed() {
                        m.volume = v / 100.0;
                    }
                }
                let mut v = (sound.volume * 100.0).round();
                if ui.add(egui::Slider::new(&mut v, 0.0..=100.0).text(i18n.tr("Sound effects"))).changed() {
                    sound.volume = v / 100.0;
                }
            }
        });
        ui.separator();
        if *confirm {
            ui.label(i18n.tr("Really reset settings?"));
            ui.label(i18n.tr("This will reset all settings except controls and restart SpaceEngine"));
            ui.horizontal(|ui| {
                if ui.button(i18n.tr("Yes")).clicked() {
                    reset.0 = true;
                }
                if ui.button(i18n.tr("No")).clicked() {
                    *confirm = false;
                }
            });
        } else if ui.button(i18n.tr("Reset settings")).clicked() {
            *confirm = true;
        }
    });
    if !open {
        hud.show_settings = false;
        gfx_ui.open = false;
        state.confirm_reset = false;
    }
    if let Some(code) = pick_lang {
        i18n.set_language(&data.vfs, &code);
    }
    if let Some(n) = pick_preset {
        gs.pick_preset(&n);
    } else if q != gs.quality {
        gs.quality = q;
        gs.preset = "Custom".into();
    }
    if l != *lens {
        *lens = l;
    }
    if b != *bloom {
        *bloom = b;
    }
    if d != *display {
        *display = d;
    }
    if f != *fi {
        *fi = f;
    }
    if ph as u8 != photo.0 {
        *photo = PhotoMode::new(ph as u8);
    }
    if ex != *exposure {
        *exposure = ex;
    }
    if sh != *shots {
        *shots = sh;
    }
    if let (Some(e), Some(cur)) = (em, emission.as_mut())
        && e != **cur
    {
        **cur = e;
    }
    if let (Some(w), Some(cur)) = (wa, water.as_mut())
        && w != cur.show
    {
        cur.show = w;
    }
    if let (Some(v), true) = (bcsg, bcsg != bcsg0) {
        for mut c in &mut cc {
            c.bcsg = ColorCorrection::new(v[0], v[1], v[2], v[3]).bcsg;
        }
    }
    Ok(())
}
