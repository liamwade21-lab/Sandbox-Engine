//! SpaceEngine's bloom (`hdr_down.glsl`, `hdr_gauss_blur.glsl`, `hdr_combine.glsl`), in place of
//! Bevy's: the HDR frame is averaged down into a 6-level chain (level 0 at half resolution), each level
//! is blurred with SE's 9-tap Gaussian, and BloomBright (0.5) times the normalised weighted sum of the
//! levels (0.50, 0.60, 0.60, 0.45, 0.35, 0.23) is added to the frame before tone mapping. The threshold
//! is 0, so everything glows a little; a bright point (a star, whose own sprite is only a pixel or two
//! across, `stars.wesl`) spreads half its light into a wide halo.
//!
//! The per-level blur kernel is normalised here (SE's KERNEL 2 weights sum to 2 per pass, which would
//! quadruple the bloom; its rendering code was not decoded).
//!
//! The chain also gives the interface its blurred background ([`GuiBackdrop`], SE's gui.glsl
//! `BackTex`): the blurred third level, tone-mapped into an image that `skin.rs` adds to every window.

use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::texture::GpuImage;
use bevy::window::PrimaryWindow;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

/// Levels of the bloom chain (SE: six).
const LEVELS: u32 = 6;
const FORMAT: TextureFormat = TextureFormat::Rg11b10Ufloat;
/// The main texture of an HDR view.
const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const BACKDROP_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
/// The chain level the interface's background is read from (1/8 of the frame, blurred: about SE's
/// `BackgroundBluriness` 3.5 mip levels).
const BACKDROP_LEVEL: usize = 2;

/// The blurred frame behind the interface's windows, at 1/8 of the window's size.
#[derive(Resource, ExtractResource, Clone)]
#[extract_app(RenderApp)]
pub struct GuiBackdrop {
    pub image: Handle<Image>,
}

fn init_backdrop(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new_target_texture(16, 16, BACKDROP_FORMAT, None));
    commands.insert_resource(GuiBackdrop { image });
}

fn size_backdrop(backdrop: Res<GuiBackdrop>, windows: Query<&Window, With<PrimaryWindow>>, mut images: ResMut<Assets<Image>>) {
    let Ok(window) = windows.single() else { return };
    let size = (UVec2::new(window.physical_width(), window.physical_height()) / 8).max(UVec2::ONE);
    let current = images.get(&backdrop.image).map(|i| i.size());
    if current.is_some_and(|c| c != size) {
        let _ = images.insert(&backdrop.image, Image::new_target_texture(size.x, size.y, BACKDROP_FORMAT, None));
    }
}

/// SE's bloom settings on the camera (main-def.cfg `PostEffects`).
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Debug)]
#[extract_app(RenderApp)]
pub struct SeBloom {
    /// x: BloomBright, y: BloomThreshold, z: FrameDynamicRange, w: BloomDynamicRange.
    pub params: Vec4,
}

impl Default for SeBloom {
    fn default() -> Self {
        Self { params: Vec4::new(0.5, 0.0, 65535.0, 50.0) }
    }
}

/// The user's bloom settings (SE's `EnableBloom`, `BloomBright`, `BloomThreshold`), copied onto the
/// camera's [`SeBloom`]. Env `SE_NO_BLOOM` starts with bloom off.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct BloomSettings {
    pub enabled: bool,
    pub bright: f32,
    pub threshold: f32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self { enabled: std::env::var_os("SE_NO_BLOOM").is_none(), bright: 0.5, threshold: 0.0 }
    }
}

fn apply_settings(settings: Res<BloomSettings>, mut cams: Query<&mut SeBloom>) {
    if !settings.is_changed() {
        return;
    }
    for mut b in &mut cams {
        b.params.x = if settings.enabled { settings.bright } else { 0.0 };
        b.params.y = settings.threshold;
    }
}

pub struct SeBloomPlugin;

impl Plugin for SeBloomPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BloomSettings>()
            .add_systems(Startup, init_backdrop)
            .add_systems(PostUpdate, (apply_settings, size_backdrop));
        app.add_plugins((
            ExtractComponentPlugin::<SeBloom>::default(),
            UniformComponentPlugin::<SeBloom>::default(),
            ExtractResourcePlugin::<GuiBackdrop>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, se_bloom.before(tonemapping).in_set(Core3dSystems::PostProcess));
    }
}

#[derive(Resource)]
struct BloomPipelines {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    down_first: CachedRenderPipelineId,
    down: CachedRenderPipelineId,
    blur_h: CachedRenderPipelineId,
    blur_v: CachedRenderPipelineId,
    combine: CachedRenderPipelineId,
    backdrop: CachedRenderPipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "se_bloom_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<SeBloom>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..Default::default()
    });
    let shader: Handle<Shader> = asset_server.load("shaders/se_bloom.wesl");
    let queue = |entry: &'static str, format: TextureFormat, blend: Option<BlendState>| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("se_bloom_{entry}").into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format, blend, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        })
    };
    let add = BlendState {
        color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
        alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
    };
    let pipelines = BloomPipelines {
        down_first: queue("down_first", FORMAT, None),
        down: queue("down", FORMAT, None),
        blur_h: queue("blur_h", FORMAT, None),
        blur_v: queue("blur_v", FORMAT, None),
        combine: queue("combine", HDR_FORMAT, Some(add)),
        backdrop: queue("backdrop", BACKDROP_FORMAT, None),
        layout,
        sampler,
    };
    commands.insert_resource(pipelines);
}

/// The bloom chain (`a`) and the horizontal-blur scratch chain (`b`) of a view.
#[derive(Component)]
struct BloomTextures {
    a: CachedTexture,
    b: CachedTexture,
    levels: u32,
}

fn level_view(t: &CachedTexture, level: u32) -> TextureView {
    t.texture.create_view(&TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), ..Default::default() })
}

fn prepare_textures(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera), With<SeBloom>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let half = (size / 2).max(UVec2::ONE);
        let levels = LEVELS.min(half.x.min(half.y).ilog2() + 1);
        let desc = |label: &'static str| TextureDescriptor {
            label: Some(label),
            size: Extent3d { width: half.x, height: half.y, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let a = cache.get(&device, desc("se_bloom_a"));
        let b = cache.get(&device, desc("se_bloom_b"));
        commands.entity(entity).insert(BloomTextures { a, b, levels });
    }
}

#[allow(clippy::too_many_arguments)]
fn se_bloom(
    view: ViewQuery<(&ExtractedCamera, &ViewTarget, &BloomTextures, &DynamicUniformIndex<SeBloom>, &SeBloom)>,
    pipelines: Res<BloomPipelines>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<SeBloom>>,
    backdrop: Option<Res<GuiBackdrop>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (camera, target, tex, index, settings) = view.into_inner();
    let backdrop = backdrop.and_then(|b| gpu_images.get(&b.image));
    let bloom = settings.params.x > 0.0;
    if !(bloom || backdrop.is_some()) || !camera.hdr || target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let (Some(binding), Some(down_first), Some(down), Some(blur_h), Some(blur_v), Some(combine), Some(backdrop_pipe)) = (
        uniforms.binding(),
        pipeline_cache.get_render_pipeline(pipelines.down_first),
        pipeline_cache.get_render_pipeline(pipelines.down),
        pipeline_cache.get_render_pipeline(pipelines.blur_h),
        pipeline_cache.get_render_pipeline(pipelines.blur_v),
        pipeline_cache.get_render_pipeline(pipelines.combine),
        pipeline_cache.get_render_pipeline(pipelines.backdrop),
    ) else {
        return;
    };
    let layout = pipeline_cache.get_bind_group_layout(&pipelines.layout);
    let device = ctx.render_device().clone();
    let group = |src: &TextureView| {
        device.create_bind_group("se_bloom_group", &layout, &BindGroupEntries::sequential((src, &pipelines.sampler, binding.clone())))
    };
    let scene = target.main_texture_view().clone();
    let (a, b): (Vec<TextureView>, Vec<TextureView>) = (0..tex.levels).map(|l| (level_view(&tex.a, l), level_view(&tex.b, l))).unzip();
    let full_a = tex.a.texture.create_view(&TextureViewDescriptor::default());

    let encoder = ctx.command_encoder();
    encoder.push_debug_group("se_bloom");
    let mut pass = |dst: &TextureView, pipeline: &RenderPipeline, src: &TextureView| {
        let bind = group(src);
        let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("se_bloom_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: dst,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        p.set_pipeline(pipeline);
        p.set_bind_group(0, &bind, &[index.index()]);
        p.draw(0..3, 0..1);
    };
    // 1. The chain of averages.
    pass(&a[0], down_first, &scene);
    for l in 1..tex.levels as usize {
        pass(&a[l], down, &a[l - 1]);
    }
    // 2. Each level blurred in place (through the scratch chain).
    for l in 0..tex.levels as usize {
        pass(&b[l], blur_h, &a[l]);
        pass(&a[l], blur_v, &b[l]);
    }
    // The interface's blurred background.
    if let Some(gpu) = backdrop {
        pass(&gpu.texture_view, backdrop_pipe, &a[BACKDROP_LEVEL.min(a.len() - 1)]);
    }
    // 3. Added to the frame.
    if bloom {
        let bind = group(&full_a);
        let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("se_bloom_combine"),
            color_attachments: &[Some(target.get_unsampled_color_attachment())],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        p.set_pipeline(combine);
        p.set_bind_group(0, &bind, &[index.index()]);
        p.draw(0..3, 0..1);
    }
    encoder.pop_debug_group();
}
