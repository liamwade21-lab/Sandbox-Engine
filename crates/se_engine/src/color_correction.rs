//! SE's tone mapping and final colour correction: hdr_tone.glsl with the `ToneMapping` of the chosen
//! `data/shaders/hdrt_*_tone.glh` (main-def.cfg `ToneMappingShader`, "Tone mapping" in the settings),
//! then hdr_final.glsl with hsl.glh `ColorCorrentionLDR`: contrast and brightness, a per-pixel vibrance
//! (`Saturation`) that lifts grey pixels more than vivid ones, gamma, and dithering. Settings from
//! main-def.cfg `PostEffects` (`Brightness`, `Contrast`, `Saturation`, `Gamma`, `ToneMappingShader`) and
//! `RenderingQuality` (`Dithering`).
//!
//! Bevy's own tone mapping pass runs first as `Tonemapping::Linear`: it applies the exposure
//! (`ColorGrading`, `exposure.rs`) and keeps its one `post_process_write` flip. This pass then writes its
//! result back into the same main texture (through a scratch texture) instead of flipping to the other
//! one: Bevy keeps the main-texture index from frame to frame, and an extra flip made every few frames
//! show a wrong image (a flicker).
//!
//! SE's tone shaders output values for its 8-bit target; the port treats them as linear (the swapchain
//! encodes sRGB), which keeps the AgX look the port was calibrated with (assumed: SE's target format was
//! not traced, and its shaders disagree, AgX linearising and ACES gamma-encoding).

use bevy::core_pipeline::tonemapping::{DebandDither, tonemapping};
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only_sized, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::camera::MainCamera;
use crate::exposure::{AutoExposureBuffers, PhotoMode};

/// The main texture of an HDR view (`camera.rs` spawns the main camera with `Hdr`).
const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// SE's tone mapping shaders, the `hdrt_<name>_tone.glh` files FUN_1403238d0 lists ("seRenderer::Init():
/// No tone mapping shader were found" when none), in file order; `color_correction.wesl` implements each.
pub const TONE_SHADERS: [&str; 11] =
    ["ACES_Fitted", "AgX", "AgX_Punchy", "Bruneton", "Exposure", "Filmic", "Filmic_Hejl", "Rec_709", "Reinhard", "Reinhard_SE", "sRGB"];

/// Index of a tone shader by name (case-insensitive), as `ToneMappingShader` names it.
pub fn tone_index(name: &str) -> Option<usize> {
    TONE_SHADERS.iter().position(|n| n.eq_ignore_ascii_case(name.trim()))
}

/// The tone shader and dithering (`ToneMappingShader`, `Dithering`).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct FinalImage {
    /// Index into [`TONE_SHADERS`].
    pub tone: usize,
    pub dithering: bool,
}

impl Default for FinalImage {
    /// main-def.cfg's "AgX" with dithering.
    fn default() -> Self {
        Self { tone: 1, dithering: true }
    }
}

impl FinalImage {
    pub fn tone_name(&self) -> &'static str {
        TONE_SHADERS[self.tone.min(TONE_SHADERS.len() - 1)]
    }
}

/// Camera component of the pass; layout must match `color_correction.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Debug)]
#[extract_app(RenderApp)]
pub struct ColorCorrection {
    /// SE's `bcsg`: x brightness - 1, y contrast, z saturation - 1 (vibrance), w 1 / gamma.
    pub bcsg: Vec4,
    /// x the tone shader, y dithering, z auto exposure (photo mode 2) on.
    pub mode: UVec4,
}

impl Default for ColorCorrection {
    fn default() -> Self {
        Self::new(1.0, 1.0, 1.5, 1.0)
    }
}

impl ColorCorrection {
    pub fn new(brightness: f32, contrast: f32, saturation: f32, gamma: f32) -> Self {
        Self { bcsg: Vec4::new(brightness - 1.0, contrast, saturation - 1.0, 1.0 / gamma.max(1e-3)), mode: UVec4::new(1, 1, 0, 0) }
    }

    /// SE's (`Brightness`, `Contrast`, `Saturation`, `Gamma`).
    pub fn values(&self) -> [f32; 4] {
        [self.bcsg.x + 1.0, self.bcsg.y, self.bcsg.z + 1.0, 1.0 / self.bcsg.w.max(1e-6)]
    }
}

pub struct ColorCorrectionPlugin;

impl Plugin for ColorCorrectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((ExtractComponentPlugin::<ColorCorrection>::default(), UniformComponentPlugin::<ColorCorrection>::default()))
            .init_resource::<FinalImage>()
            .add_systems(Startup, load_final_image)
            .add_systems(PostStartup, attach)
            .add_systems(PostUpdate, sync_mode);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        // On Bevy's (linear) tone mapping output, as SE's tone and final passes.
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_scratch.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, color_correction.after(tonemapping).in_set(Core3dSystems::PostProcess));
    }
}

/// main-def.cfg's `ToneMappingShader` and `Dithering` (env `SE_TONEMAP=<name>` for testing, `aces` for
/// ACES_Fitted).
fn load_final_image(cfg: Res<crate::config::EngineConfig>, mut fi: ResMut<FinalImage>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    if let Some(t) = def.find_deep("ToneMappingShader").and_then(|i| i.str().map(str::to_string)).and_then(|n| tone_index(&n)) {
        fi.tone = t;
    }
    fi.dithering = def.find_deep("Dithering").and_then(|i| i.bool()).unwrap_or(fi.dithering);
    if let Ok(v) = std::env::var("SE_TONEMAP") {
        let v = if v.eq_ignore_ascii_case("aces") { "ACES_Fitted".to_string() } else { v };
        if let Some(t) = tone_index(&v) {
            fi.tone = t;
        }
    }
}

/// Put SE's settings on the main camera; the pass dithers, so Bevy's dither is off.
fn attach(mut commands: Commands, cfg: Res<crate::config::EngineConfig>, cams: Query<Entity, With<MainCamera>>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    let num = |k: &str, d: f64| def.find_deep(k).and_then(|i| i.f64()).unwrap_or(d) as f32;
    let cc = ColorCorrection::new(num("Brightness", 1.0), num("Contrast", 1.0), num("Saturation", 1.5), num("Gamma", 1.0));
    for e in &cams {
        commands.entity(e).insert((cc, DebandDither::Disabled));
    }
}

/// The tone shader, dithering and photo mode onto the camera's uniform.
fn sync_mode(fi: Res<FinalImage>, photo: Res<PhotoMode>, mut cams: Query<&mut ColorCorrection, With<MainCamera>>) {
    let want = UVec4::new(fi.tone.min(TONE_SHADERS.len() - 1) as u32, fi.dithering as u32, photo.auto() as u32, 0);
    for mut c in &mut cams {
        if c.mode != want {
            c.mode = want;
        }
    }
}

#[derive(Resource)]
struct ColorCorrectionPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    pipeline: CachedRenderPipelineId,
    /// Copies the scratch texture back into the main texture.
    blit: CachedRenderPipelineId,
}

/// The corrected frame, before it is drawn back into the main texture.
#[derive(Component)]
struct Scratch(CachedTexture);

fn prepare_scratch(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera), With<ColorCorrection>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera.physical_target_size else { continue };
        let texture = cache.get(
            &device,
            TextureDescriptor {
                label: Some("color_correction_scratch"),
                size: Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: HDR_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(Scratch(texture));
    }
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "color_correction_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<ColorCorrection>(true),
                storage_buffer_read_only_sized(false, None),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let shader: Handle<Shader> = asset_server.load("shaders/color_correction.wesl");
    let queue = |entry: &'static str| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("color_correction_{entry}").into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        })
    };
    let (pipeline, blit) = (queue("fragment"), queue("blit"));
    commands.insert_resource(ColorCorrectionPipeline { layout, sampler, pipeline, blit });
}

fn color_correction(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<ColorCorrection>, &Scratch)>,
    pipeline: Res<ColorCorrectionPipeline>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<ColorCorrection>>,
    auto: Res<AutoExposureBuffers>,
    mut ctx: RenderContext,
) {
    let (target, index, scratch) = view.into_inner();
    let (Some(correct), Some(blit), Some(binding)) = (
        pipeline_cache.get_render_pipeline(pipeline.pipeline),
        pipeline_cache.get_render_pipeline(pipeline.blit),
        uniforms.binding(),
    ) else {
        return;
    };
    if target.main_texture_format() != HDR_FORMAT || scratch.0.texture.size() != target.main_texture().size() {
        return;
    }
    let layout = pipeline_cache.get_bind_group_layout(&pipeline.layout);
    let device = ctx.render_device().clone();
    let group = |src: &TextureView| {
        device.create_bind_group(
            "color_correction_group",
            &layout,
            &BindGroupEntries::sequential((src, &pipeline.sampler, binding.clone(), auto.state.as_entire_buffer_binding())),
        )
    };
    let (from_main, from_scratch) = (group(target.main_texture_view()), group(&scratch.0.default_view));
    let encoder = ctx.command_encoder();
    // The corrected frame into the scratch texture, then back into the main texture.
    for (attachment, render_pipeline, bind) in [
        (
            RenderPassColorAttachment { view: &scratch.0.default_view, depth_slice: None, resolve_target: None, ops: Operations::default() },
            correct,
            &from_main,
        ),
        (target.get_unsampled_color_attachment(), blit, &from_scratch),
    ] {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("color_correction"),
            color_attachments: &[Some(attachment)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(render_pipeline);
        pass.set_bind_group(0, bind, &[index.index()]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_shaders_by_name() {
        assert_eq!(tone_index("AgX"), Some(1));
        assert_eq!(tone_index("reinhard_se"), Some(9));
        assert_eq!(tone_index("Nope"), None);
        assert_eq!(FinalImage::default().tone_name(), "AgX");
        let v = ColorCorrection::new(1.2, 0.9, 1.5, 2.0).values();
        assert!((v[0] - 1.2).abs() < 1e-6 && (v[1] - 0.9).abs() < 1e-6 && (v[2] - 1.5).abs() < 1e-6 && (v[3] - 2.0).abs() < 1e-5);
    }
}
