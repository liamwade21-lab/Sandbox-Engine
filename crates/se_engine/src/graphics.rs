//! Graphics quality presets: SE's `config/preset-potato|low|medium|high|ultra.cfg`, read whole at runtime
//! (FUN_140288db0 loads the chosen file over the settings), and the first-run choice by video memory.
//!
//! Preset keys the port maps: `MSAALevel`, `FXAA`, `AnisotropyLevel` (planet surface textures), `LandLOD`
//! and `LandTargetRes` (terrain subdivision), `MaxTilesPerFrame` (terrain tiles started per frame),
//! `VolumResolution` (ray-march steps of the nebula volumes), `StarPointMethod` (star points) and
//! `WarpPrecision` (black hole ray-march step). The others (VideoMemorySize, TextureCompress,
//! PlanetPointMethod, PlanetShineMode, AuroraQuality, ShipEngineQuality, BlackHoleQuality,
//! ShipWarpQuality, the TextureRendering resolutions and environment maps, the loader time limits) drive
//! SE features the port draws differently; they are kept as read ([`Quality::keys`]), saved with the
//! settings and shown in the Graphics settings.

use bevy::anti_alias::fxaa::Fxaa;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera::MainCamera;
use crate::config::EngineConfig;
use crate::surface_tex::SurfaceTextures;

pub const PRESET_NAMES: [&str; 5] = ["Potato", "Low", "Medium", "High", "Ultra"];

/// Keys read into [`Quality`]'s own fields (the rest go to [`Quality::keys`]).
const MAPPED: [&str; 9] =
    ["MSAALevel", "FXAA", "AnisotropyLevel", "LandLOD", "LandTargetRes", "MaxTilesPerFrame", "VolumResolution", "StarPointMethod", "WarpPrecision"];

/// The values of one preset.
#[derive(Clone, Debug, PartialEq)]
pub struct Quality {
    pub msaa: u32,
    pub fxaa: bool,
    pub anisotropy: u16,
    pub land_lod: f32,
    pub land_target_res: f32,
    pub max_tiles_per_frame: u32,
    pub volume_resolution: f32,
    /// `StarPointMethod` (1 sprites, 2 motion blur, 3 motion and rotation blur).
    pub star_points: u32,
    /// `WarpPrecision` (black holes' ray-march step).
    pub warp_precision: f32,
    /// The preset's other keys and values, as written (`"true"`, `"0.85"`, ...), in file order.
    pub keys: Vec<(String, String)>,
}

impl Default for Quality {
    /// `preset-high.cfg`.
    fn default() -> Self {
        Self {
            msaa: 0,
            fxaa: true,
            anisotropy: 16,
            land_lod: 0.0,
            land_target_res: 1080.0,
            max_tiles_per_frame: 4,
            volume_resolution: 0.5,
            star_points: 2,
            warp_precision: 0.8,
            keys: Vec::new(),
        }
    }
}

/// A script value as a config file writes it.
fn value_text(v: &se_core::sc::Value) -> Option<String> {
    use se_core::sc::Value;
    match v {
        Value::Num(x) => Some(x.to_string()),
        Value::Ident(s) => Some(s.clone()),
        Value::Str(s) => Some(format!("\"{s}\"")),
        Value::Tuple(_) => None,
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
        if let Some(v) = num("StarPointMethod") {
            q.star_points = v.clamp(1.0, 3.0) as u32;
        }
        if let Some(v) = num("WarpPrecision") {
            q.warp_precision = v as f32;
        }
        // Every other leaf key, in file order (blocks are SE's groups: Memory, RenderingQuality, ...).
        fn walk(b: &se_core::sc::Block, out: &mut Vec<(String, String)>) {
            for item in &b.items {
                if let Some(inner) = &item.block {
                    walk(inner, out);
                } else if let (false, Some(v)) = (MAPPED.contains(&item.key.as_str()), item.args.first().and_then(value_text)) {
                    match out.iter_mut().find(|(k, _)| *k == item.key) {
                        Some(slot) => slot.1 = v,
                        None => out.push((item.key.clone(), v)),
                    }
                }
            }
        }
        walk(&doc, &mut q.keys);
        q
    }

    /// The preset as config lines (`\tKey value`), the mapped keys first.
    pub fn cfg_lines(&self) -> String {
        let mut out = format!(
            "\tMSAALevel         {}\n\tFXAA              {}\n\tAnisotropyLevel   {}\n\tLandLOD           {}\n\tLandTargetRes     {}\n\tMaxTilesPerFrame  {}\n\tVolumResolution   {}\n\tStarPointMethod   {}\n\tWarpPrecision     {}\n",
            self.msaa, self.fxaa, self.anisotropy, self.land_lod, self.land_target_res, self.max_tiles_per_frame, self.volume_resolution, self.star_points, self.warp_precision
        );
        for (k, v) in &self.keys {
            out += &format!("\t{k} {v}\n");
        }
        out
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

/// FUN_14030b0e0's "optimal config" on a hardware or version change: the preset for the video memory
/// (MB): up to 2 GB Potato, 3 GB Low, 4 GB Medium, else High.
pub fn optimal_preset(vram_mb: u64) -> &'static str {
    match vram_mb {
        ..=2048 => "Potato",
        ..=3072 => "Low",
        ..=4096 => "Medium",
        _ => "High",
    }
}

/// The largest adapter's video memory in MB, from the display adapters' registry entries (Windows;
/// the port has no other way to ask without a new dependency).
pub fn video_memory_mb() -> Option<u64> {
    if !cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new("reg")
        .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}", "/s", "/v", "HardwareInformation.qwMemorySize"])
        .output()
        .ok()?;
    parse_vram(&String::from_utf8_lossy(&out.stdout))
}

/// The largest `REG_QWORD 0x...` in `reg query` output, in MB.
fn parse_vram(text: &str) -> Option<u64> {
    text.lines()
        .filter(|l| l.contains("qwMemorySize"))
        .filter_map(|l| l.split_whitespace().last().and_then(|h| u64::from_str_radix(h.trim_start_matches("0x"), 16).ok()))
        .max()
        .map(|b| b / (1024 * 1024))
}

/// The active quality and the presets of the install.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GraphicsSettings {
    /// One of `PRESET_NAMES`, or "Custom" after an individual change.
    pub preset: String,
    pub quality: Quality,
    pub presets: Vec<(String, Quality)>,
    /// Set when a preset is picked: its star point method then replaces the user's (SE loads the
    /// whole file over the settings).
    pub picked: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self { preset: "High".into(), quality: Quality::default(), presets: Vec::new(), picked: false }
    }
}

impl GraphicsSettings {
    pub fn apply_preset(&mut self, name: &str) {
        if let Some((n, q)) = self.presets.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            self.preset = n.clone();
            self.quality = q.clone();
        }
    }

    /// A preset chosen by the user (or by the first-run choice): applied with all its keys.
    pub fn pick_preset(&mut self, name: &str) {
        self.apply_preset(name);
        self.picked = true;
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

/// The graphics settings opened from the toolbar or Ctrl+F4 (the settings window's Graphics tab).
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
    mut gs: ResMut<GraphicsSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut terrain: ResMut<TerrainQuality>,
    mut camera: Query<(Entity, &mut Msaa, Has<Fxaa>), With<MainCamera>>,
    surface: Option<Res<SurfaceTextures>>,
    mut images: ResMut<Assets<Image>>,
    (mut stars, holes): (ResMut<crate::stars::StarSettings>, Option<ResMut<crate::blackholes::BlackHoleSettings>>),
    mut last_aniso: Local<Option<u16>>,
) {
    let q = gs.quality.clone();
    let h = windows.iter().next().map(|w| w.resolution.physical_height() as f32).unwrap_or(1080.0);
    terrain.split_scale = q.terrain_split_scale(h);
    terrain.tiles_per_frame = (q.max_tiles_per_frame as usize * 2).max(1);
    if !gs.is_changed() && last_aniso.is_some() == surface.is_some() {
        return;
    }
    if gs.picked {
        gs.picked = false;
        stars.point_style = crate::stars::PointStyle::from_method(q.star_points);
    }
    if let Some(mut b) = holes
        && b.warp_precision != q.warp_precision as f64
    {
        b.warp_precision = q.warp_precision as f64;
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

    #[test]
    fn keeps_every_preset_key_and_round_trips() {
        let text = "Memory\n{\n\tVideoMemorySize 4096\n}\nRenderingQuality\n{\n\tFXAA true\n\tTextureCompress false\n}\nPointRendering\n{\n\tStarPointMethod 3\n}\nWarpRendering\n{\n\tWarpPrecision 0.6\n}\n";
        let q = Quality::from_cfg(text, &Quality::default());
        assert_eq!((q.star_points, q.warp_precision), (3, 0.6));
        assert_eq!(q.keys, vec![("VideoMemorySize".to_string(), "4096".to_string()), ("TextureCompress".to_string(), "false".to_string())]);
        let again = Quality::from_cfg(&format!("Graphics\n{{\n{}}}\n", q.cfg_lines()), &Quality::default());
        assert_eq!(again, q);
    }

    #[test]
    fn preset_by_video_memory() {
        assert_eq!(optimal_preset(2048), "Potato");
        assert_eq!(optimal_preset(3000), "Low");
        assert_eq!(optimal_preset(4096), "Medium");
        assert_eq!(optimal_preset(16304), "High");
        let reg = "    HardwareInformation.qwMemorySize    REG_QWORD    0x20000000\r\n    HardwareInformation.qwMemorySize    REG_QWORD    0x3faf00000\r\n";
        assert_eq!(parse_vram(reg), Some(16303));
    }
}
