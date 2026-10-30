//! Graphics quality presets (SE's `config/preset-potato|low|medium|high|ultra.cfg`, read at runtime) and
//! the Graphics window, which also holds the lens flare settings.
//!
//! Preset keys the port maps: `MSAALevel`, `FXAA`, `AnisotropyLevel` (planet surface textures), `LandLOD`
//! and `LandTargetRes` (terrain subdivision), `MaxTilesPerFrame` (terrain tiles started per frame) and
//! `VolumResolution` (ray-march steps of the nebula volumes). The others drive SE features the port draws
//! differently (render scale, texture compression, impostors, environment maps) and are kept only for
//! display.

use bevy::anti_alias::fxaa::Fxaa;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;

use crate::camera::MainCamera;
use crate::config::EngineConfig;
use crate::lensflare::{FlarePresets, LensFlareSettings};
use crate::surface_tex::SurfaceTextures;

pub const PRESET_NAMES: [&str; 5] = ["Potato", "Low", "Medium", "High", "Ultra"];

/// The mapped values of one preset.
#[derive(Clone, Debug, PartialEq)]
pub struct Quality {
    pub msaa: u32,
    pub fxaa: bool,
    pub anisotropy: u16,
    pub land_lod: f32,
    pub land_target_res: f32,
    pub max_tiles_per_frame: u32,
    pub volume_resolution: f32,
}

impl Default for Quality {
    /// `preset-high.cfg`.
    fn default() -> Self {
        Self { msaa: 0, fxaa: true, anisotropy: 16, land_lod: 0.0, land_target_res: 1080.0, max_tiles_per_frame: 4, volume_resolution: 0.5 }
    }
}

impl Quality {
    /// Reads a preset script; missing keys keep `base`.
    pub fn from_cfg(text: &str, base: &Quality) -> Self {
        let doc = se_core::sc::parse(text);
        let num = |k: &str| doc.find_deep(k).and_then(|i| i.f64());
        let mut q = base.clone();
        if let Some(v) = num("MSAALevel") {
            q.msaa = v.max(0.0) as u32;
        }
        if let Some(v) = doc.find_deep("FXAA").and_then(|i| i.bool()) {
            q.fxaa = v;
        }
        if let Some(v) = num("AnisotropyLevel") {
            q.anisotropy = v.clamp(0.0, 16.0) as u16;
        }
        if let Some(v) = num("LandLOD") {
            q.land_lod = v as f32;
        }
        if let Some(v) = num("LandTargetRes") {
            q.land_target_res = v as f32;
        }
        if let Some(v) = num("MaxTilesPerFrame") {
            q.max_tiles_per_frame = v.max(1.0) as u32;
        }
        if let Some(v) = num("VolumResolution") {
            q.volume_resolution = v as f32;
        }
        q
    }

    /// Terrain split distance factor: SE's `LandLOD` is a power of two of the detail. SE's split
    /// constant (`FUN_1404acdd0`) is `2^LandLOD * max(1, A/B)`: never coarser on small screens, and
    /// finer on screens taller than 1080 only as far as `LandTargetRes` allows ("kept as like the
    /// screen has this resolution"); the port's split distance was tuned at 1080 lines.
    pub fn terrain_split_scale(&self, screen_height: f32) -> f64 {
        let res = (screen_height.min(self.land_target_res) / 1080.0).max(1.0);
        2f64.powf(self.land_lod as f64) * res as f64
    }
}

/// The active quality and the presets of the install.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GraphicsSettings {
    /// One of `PRESET_NAMES`, or "Custom" after an individual change.
    pub preset: String,
    pub quality: Quality,
    pub presets: Vec<(String, Quality)>,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self { preset: "High".into(), quality: Quality::default(), presets: Vec::new() }
    }
}

impl GraphicsSettings {
    pub fn apply_preset(&mut self, name: &str) {
        if let Some((n, q)) = self.presets.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            self.preset = n.clone();
            self.quality = q.clone();
        }
    }
}

/// What the terrain reads each frame (`terrain.rs`).
#[derive(Resource, Clone, Copy, Debug)]
pub struct TerrainQuality {
    pub split_scale: f64,
    pub tiles_per_frame: usize,
}

impl Default for TerrainQuality {
    fn default() -> Self {
        Self { split_scale: 1.0, tiles_per_frame: 8 }
    }
}

#[derive(Resource, Default)]
pub struct GraphicsUi {
    pub open: bool,
}

pub struct GraphicsPlugin;

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GraphicsSettings>()
            .init_resource::<TerrainQuality>()
            .init_resource::<GraphicsUi>()
            .add_systems(Startup, load_presets)
            // Drawn in egui's pass with the other windows (in `Update` it ran outside the pass, and
            // panicked when open on the first frame).
            .add_systems(bevy_egui::EguiPrimaryContextPass, graphics_window.in_set(crate::skin::UiSet::Draw))
            .add_systems(Update, apply_quality);
    }
}

fn load_presets(cfg: Res<EngineConfig>, mut gs: ResMut<GraphicsSettings>) {
    let base = Quality::default();
    gs.presets = PRESET_NAMES
        .iter()
        .filter_map(|n| {
            let path = cfg.se_path.join(format!("config/preset-{}.cfg", n.to_ascii_lowercase()));
            let text = std::fs::read_to_string(path).ok()?;
            Some((n.to_string(), Quality::from_cfg(&text, &base)))
        })
        .collect();
    info!("graphics presets: {}", gs.presets.len());
    let name = gs.preset.clone();
    gs.apply_preset(&name);
}

fn apply_quality(
    mut commands: Commands,
    gs: Res<GraphicsSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut terrain: ResMut<TerrainQuality>,
    mut camera: Query<(Entity, &mut Msaa, Has<Fxaa>), With<MainCamera>>,
    surface: Option<Res<SurfaceTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut last_aniso: Local<Option<u16>>,
) {
    let q = &gs.quality;
    let h = windows.iter().next().map(|w| w.resolution.physical_height() as f32).unwrap_or(1080.0);
    terrain.split_scale = q.terrain_split_scale(h);
    terrain.tiles_per_frame = (q.max_tiles_per_frame as usize * 2).max(1);
    if !gs.is_changed() && last_aniso.is_some() == surface.is_some() {
        return;
    }
    for (e, mut msaa, has_fxaa) in &mut camera {
        let want = match q.msaa {
            0 | 1 => Msaa::Off,
            2 => Msaa::Sample2,
            3..=4 => Msaa::Sample4,
            _ => Msaa::Sample8,
        };
        if *msaa != want {
            *msaa = want;
        }
        match (q.fxaa, has_fxaa) {
            (true, false) => {
                commands.entity(e).insert(Fxaa::default());
            }
            (false, true) => {
                commands.entity(e).remove::<Fxaa>();
            }
            _ => {}
        }
    }
    if let Some(s) = surface {
        if *last_aniso != Some(q.anisotropy) {
            for h in [&s.samples, &s.soils] {
                if let Some(mut img) = images.get_mut(h)
                    && let ImageSampler::Descriptor(d) = &mut img.sampler
                {
                    d.anisotropy_clamp = q.anisotropy.max(1);
                }
            }
            *last_aniso = Some(q.anisotropy);
        }
    }
}

fn graphics_window(
    mut contexts: bevy_egui::EguiContexts,
    mut ui_state: ResMut<GraphicsUi>,
    mut gs: ResMut<GraphicsSettings>,
    mut lens: ResMut<LensFlareSettings>,
    flare_presets: Option<Res<FlarePresets>>,
    mut bloom: ResMut<crate::bloom::BloomSettings>,
    mut display: ResMut<crate::display::DisplaySettings>,
) {
    if !ui_state.open {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut open = true;
    let mut pick: Option<String> = None;
    let mut q = gs.quality.clone();
    let mut l = lens.clone();
    let mut b = bloom.clone();
    egui::Window::new(crate::skin::title(ctx, "Graphics")).open(&mut open).default_width(320.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Quality preset");
            egui::ComboBox::from_id_salt("gfx_preset").selected_text(gs.preset.as_str()).show_ui(ui, |ui| {
                for (n, _) in &gs.presets {
                    if ui.selectable_label(*n == gs.preset, n.as_str()).clicked() {
                        pick = Some(n.clone());
                    }
                }
            });
        });
        // SE's Display settings: window mode and vertical synchronization.
        let mut d = display.clone();
        egui::ComboBox::from_label("Window mode").selected_text(["Windowed", "Fullscreen window", "Fullscreen"][d.fullscreen as usize]).show_ui(ui, |ui| {
            for (v, name) in ["Windowed", "Fullscreen window", "Fullscreen"].into_iter().enumerate() {
                ui.selectable_value(&mut d.fullscreen, v as u8, name);
            }
        });
        egui::ComboBox::from_label("Vertical synchronization").selected_text(["Disabled", "Enabled", "Auto"][d.vsync as usize]).show_ui(ui, |ui| {
            for (v, name) in ["Disabled", "Enabled", "Auto"].into_iter().enumerate() {
                ui.selectable_value(&mut d.vsync, v as u8, name);
            }
        });
        if d != *display {
            *display = d;
        }
        egui::ComboBox::from_label("MSAA").selected_text(format!("{}x", q.msaa.max(1))).show_ui(ui, |ui| {
            for v in [0u32, 2, 4, 8] {
                ui.selectable_value(&mut q.msaa, v, format!("{}x", v.max(1)));
            }
        });
        ui.checkbox(&mut q.fxaa, "FXAA");
        ui.add(egui::Slider::new(&mut q.anisotropy, 0..=16).text("anisotropic filtering"));
        ui.add(egui::Slider::new(&mut q.land_lod, -3.0..=1.0).step_by(0.5).text("landscape LOD"));
        ui.add(egui::Slider::new(&mut q.max_tiles_per_frame, 1..=16).text("terrain tiles per frame"));
        ui.add(egui::Slider::new(&mut q.volume_resolution, 0.1..=1.0).text("volumetric quality"));
        ui.separator();
        ui.label("Bloom");
        ui.checkbox(&mut b.enabled, "bloom");
        ui.add(egui::Slider::new(&mut b.bright, 0.0..=2.0).text("bloom brightness"));
        ui.add(egui::Slider::new(&mut b.threshold, 0.0..=10.0).text("bloom threshold"));
        ui.separator();
        ui.label("Lens flares");
        ui.checkbox(&mut l.flares, "lens flares");
        ui.checkbox(&mut l.ghosts, "lens ghosts");
        ui.checkbox(&mut l.pseudo, "pseudo lens flares");
        ui.add(egui::Slider::new(&mut l.scale, 0.1..=3.0).text("flare size"));
        ui.add(egui::Slider::new(&mut l.bright, 0.0..=3.0).text("flare brightness"));
        ui.add(egui::Slider::new(&mut l.pseudo_bright, 0.0..=3.0).text("pseudo flare brightness"));
        if let Some(p) = &flare_presets {
            let mut combo = |label: &str, value: &mut String, names: Vec<&String>| {
                egui::ComboBox::from_label(label).selected_text(value.as_str()).show_ui(ui, |ui| {
                    for n in names {
                        if ui.selectable_label(n == value, n.as_str()).clicked() {
                            *value = n.clone();
                        }
                    }
                });
            };
            combo("flare style", &mut l.flare_preset, p.flares.keys().collect());
            combo("ghost style", &mut l.ghost_preset, p.ghosts.keys().collect());
            combo("pseudo flare style", &mut l.pseudo_preset, p.pseudo.keys().collect());
        }
    });
    ui_state.open = open;
    if let Some(n) = pick {
        gs.apply_preset(&n);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_preset_keys_and_scales_terrain() {
        let q = Quality::from_cfg("Graphics\n{\n\tMSAALevel 4\n\tFXAA false\n\tLandLOD -2\n\tLandTargetRes 480\n}\n", &Quality::default());
        assert_eq!((q.msaa, q.fxaa, q.land_lod, q.max_tiles_per_frame), (4, false, -2.0, 4));
        // A quarter of the detail; screens are never made coarser, and the 1080-line target caps
        // how much finer a tall screen gets.
        assert!((q.terrain_split_scale(960.0) - 0.25).abs() < 1e-12);
        assert!((Quality::default().terrain_split_scale(720.0) - 1.0).abs() < 1e-12);
        assert!((Quality::default().terrain_split_scale(2160.0) - 1.0).abs() < 1e-12);
    }
}
