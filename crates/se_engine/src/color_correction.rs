//! SE's final colour correction (`hdr_final.glsl` with hsl.glh `ColorCorrentionLDR`): after tone
//! mapping, contrast and brightness, a per-pixel vibrance (`Saturation`) that lifts grey pixels more
//! than vivid ones, and gamma. Settings from main-def.cfg `PostEffects` (`Brightness`, `Contrast`,
//! `Saturation`, `Gamma`) and `RenderingQuality` (`Dithering`).
//!
//! The pass writes its result back into the same main texture (through a scratch texture) instead of
//! flipping to the other one with `post_process_write`: Bevy keeps the main-texture index from frame
//! to frame, and the extra flip made every few frames show a wrong image (a flicker).

use bevy::core_pipeline::tonemapping::{DebandDither, tonemapping};
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::camera::MainCamera;

/// The main texture of an HDR view (`camera.rs` spawns the main camera with `Hdr`).
const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

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

pub struct ColorCorrectionPlugin;

impl Plugin for ColorCorrectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((ExtractComponentPlugin::<ColorCorrection>::default(), UniformComponentPlugin::<ColorCorrection>::default()))
            .add_systems(PostStartup, attach);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        // On the tone-mapped image, as SE's final pass.
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_scratch.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, color_correction.after(tonemapping).in_set(Core3dSystems::PostProcess));
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
        device.create_bind_group("color_correction_group", &layout, &BindGroupEntries::sequential((src, &pipeline.sampler, binding.clone())))
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
