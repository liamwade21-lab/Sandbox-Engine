//! SE's final colour correction (`hdr_final.glsl` with hsl.glh `ColorCorrentionLDR`): after tone
//! mapping, contrast and brightness, a per-pixel vibrance (`Saturation`) that lifts grey pixels more
//! than vivid ones, and gamma. Settings from main-def.cfg `PostEffects` (`Brightness`, `Contrast`,
//! `Saturation`, `Gamma`) and `RenderingQuality` (`Dithering`).

use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::schedule::Core3dSystems;
use bevy::core_pipeline::tonemapping::{DebandDither, tonemapping};
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::render::RenderApp;
use bevy::shader::ShaderRef;

use crate::camera::MainCamera;

/// Camera component of the pass; layout must match `color_correction.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Debug)]
#[extract_app(RenderApp)]
pub struct ColorCorrection {
    /// SE's `bcsg`: x brightness - 1, y contrast, z saturation - 1 (vibrance), w 1 / gamma.
    pub bcsg: Vec4,
}

impl Default for ColorCorrection {
    fn default() -> Self {
        Self::new(1.0, 1.0, 1.5, 1.0)
    }
}

impl ColorCorrection {
    pub fn new(brightness: f32, contrast: f32, saturation: f32, gamma: f32) -> Self {
        Self { bcsg: Vec4::new(brightness - 1.0, contrast, saturation - 1.0, 1.0 / gamma.max(1e-3)) }
    }
}

impl FullscreenMaterial for ColorCorrection {
    fn fragment_shader() -> ShaderRef {
        "shaders/color_correction.wesl".into()
    }

    /// On the tone-mapped image, as SE's final pass.
    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system.in_set(Core3dSystems::PostProcess).after(tonemapping)
    }
}

pub struct ColorCorrectionPlugin;

impl Plugin for ColorCorrectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FullscreenMaterialPlugin::<ColorCorrection>::default())
            .add_systems(PostStartup, attach);
    }
}

/// Put SE's settings on the main camera (and turn dithering off when `Dithering` is false).
fn attach(mut commands: Commands, cfg: Res<crate::config::EngineConfig>, cams: Query<Entity, With<MainCamera>>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    let num = |k: &str, d: f64| def.find_deep(k).and_then(|i| i.f64()).unwrap_or(d) as f32;
    let cc = ColorCorrection::new(num("Brightness", 1.0), num("Contrast", 1.0), num("Saturation", 1.5), num("Gamma", 1.0));
    let dither = def.find_deep("Dithering").and_then(|i| i.bool()).unwrap_or(true);
    for e in &cams {
        let mut ec = commands.entity(e);
        ec.insert(cc);
        if !dither {
            ec.insert(DebandDither::Disabled);
        }
    }
}
