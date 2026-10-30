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
//!
//! Between the chain and the combine come SE's lens flares (`lens_flare.glsl`, see `lensflare.rs`):
//! a probe pass reads the frame and the chain at each sun ([`FlareDraw`]), and the flare sprites
//! drawn after it are scaled by that colour, so whatever covers or reddens the sun dims and tints its
//! flare. The combine also adds SE's pseudo lens flare (`hdr_combine.glsl` FLARES): ghosts and a halo
//! of the bloom, modulated by the lens dirt and starburst textures, and the dirt's bloom mask.

use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::window::PrimaryWindow;
use bevy::render::render_resource::binding_types::{self, sampler, texture_2d, uniform_buffer};
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
/// Suns whose flares are drawn at once (one probe texel each).
pub const MAX_FLARE_SOURCES: usize = 4;
/// Floats per flare sprite vertex: NDC xy, atlas uv, rgb, source index.
pub const FLARE_VERTEX_FLOATS: usize = 8;

/// The lens flares of a view, built by `lensflare.rs` each frame.
#[derive(Component, ExtractComponent, Clone, Default, Debug)]
#[extract_app(RenderApp)]
pub struct FlareDraw {
    /// Per sun (`lens_flare.glsl` `SunScreenPos`, `Params2`): xy its screen position (uv, y down),
    /// z the bloom level read, w the frame (0) to bloom (1) mix.
    pub sources: [Vec4; MAX_FLARE_SOURCES],
    pub count: u32,
    /// The frame sample's brightness limit (`Params2.w`, BloomDynamicRange).
    pub frame_limit: f32,
    /// The samples' gain to SE's brightness scale (`lensflare.rs` STAR_FRAME_GAIN).
    pub frame_gain: f32,
    /// Sprite batches: the atlas and its vertices ([`FLARE_VERTEX_FLOATS`] each, six per sprite).
    pub batches: Vec<(AssetId<Image>, Vec<f32>)>,
}

/// The probe pass's uniform. Layout must match `lens_flare.wesl`.
#[derive(Clone, Copy, ShaderType)]
struct ProbeUniform {
    sources: [Vec4; MAX_FLARE_SOURCES],
    /// x: frame brightness limit, y: sample gain.
    params: Vec4,
}

/// The pseudo lens flare's dirt (`DirtTexture`) and starburst (`RaysTexture`) images.
#[derive(Resource, ExtractResource, Clone, Default, Debug, PartialEq)]
#[extract_app(RenderApp)]
pub struct LensTextures {
    pub dirt: Option<AssetId<Image>>,
    pub rays: Option<AssetId<Image>>,
}
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

/// SE's bloom settings on the camera (main-def.cfg `PostEffects`), with the pseudo lens flare's
/// (`hdr_combine.glsl` uniforms, set by `lensflare.rs`). Layout must match `se_bloom.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Debug, PartialEq)]
#[extract_app(RenderApp)]
pub struct SeBloom {
    /// x: BloomBright, y: BloomThreshold, z: FrameDynamicRange, w: BloomDynamicRange.
    pub params: Vec4,
    /// `BrightParams` yzw: x GhostsBright, y HaloBright (both times PseudoFlareBright), z DirtBright,
    /// w 1 while the pseudo lens flare is on.
    pub flare_bright: Vec4,
    /// `BlurParams`: x GhostsBlur, y HaloBlur, z DirtDispersal (mip levels), w viewport scale.
    pub flare_blur: Vec4,
    /// `FlareParams`: x GhostsDispersal, y GhostsCount, z HaloRadius, w Chromaticism.
    pub flare_shape: Vec4,
    /// x screen width / height, y 1 with a dirt texture, z 1 with a rays texture, w the ghosts' and
    /// halo's bloom gain.
    pub lens: Vec4,
    /// `LensRaysMatr` by columns (xy, zw).
    pub rays: Vec4,
}

impl Default for SeBloom {
    fn default() -> Self {
        Self {
            params: Vec4::new(0.5, 0.0, 65535.0, 50.0),
            flare_bright: Vec4::ZERO,
            flare_blur: Vec4::new(1.0, 2.0, 3.5, 1.0),
            flare_shape: Vec4::new(0.39, 5.0, 0.75, 2.2),
            lens: Vec4::new(16.0 / 9.0, 0.0, 0.0, 0.0),
            rays: Vec4::new(1.0, 0.0, 0.0, 1.0),
        }
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
        app.init_resource::<LensTextures>().add_plugins((
            ExtractComponentPlugin::<SeBloom>::default(),
            UniformComponentPlugin::<SeBloom>::default(),
            ExtractComponentPlugin::<FlareDraw>::default(),
            ExtractResourcePlugin::<GuiBackdrop>::default(),
            ExtractResourcePlugin::<LensTextures>::default(),
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
    /// Trilinear, for the fractional levels the flares read.
    sampler_mip: Sampler,
    down_first: CachedRenderPipelineId,
    down: CachedRenderPipelineId,
    blur_h: CachedRenderPipelineId,
    blur_v: CachedRenderPipelineId,
    combine: CachedRenderPipelineId,
    backdrop: CachedRenderPipelineId,
    /// The combine with the pseudo lens flare: chain, sampler, uniform, dirt, rays.
    flares_layout: BindGroupLayoutDescriptor,
    combine_flares: CachedRenderPipelineId,
    /// Frame, chain, sampler, probe uniform.
    probe_layout: BindGroupLayoutDescriptor,
    probe: CachedRenderPipelineId,
    /// Atlas, sampler, probe texture.
    sprite_layout: BindGroupLayoutDescriptor,
    sprite: CachedRenderPipelineId,
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
    let sampler_mip = render_device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..Default::default()
    });
    let tex = || texture_2d(TextureSampleType::Float { filterable: true });
    let flares_layout = BindGroupLayoutDescriptor::new(
        "se_bloom_flares_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (tex(), binding_types::sampler(SamplerBindingType::Filtering), uniform_buffer::<SeBloom>(true), tex(), tex()),
        ),
    );
    let probe_layout = BindGroupLayoutDescriptor::new(
        "lens_flare_probe_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (tex(), tex(), binding_types::sampler(SamplerBindingType::Filtering), uniform_buffer::<ProbeUniform>(false)),
        ),
    );
    let sprite_layout = BindGroupLayoutDescriptor::new(
        "lens_flare_sprite_layout",
        &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (tex(), binding_types::sampler(SamplerBindingType::Filtering), tex())),
    );
    let shader: Handle<Shader> = asset_server.load("shaders/se_bloom.wesl");
    let flare_shader: Handle<Shader> = asset_server.load("shaders/lens_flare.wesl");
    let probe_shader: Handle<Shader> = asset_server.load("shaders/lens_flare_probe.wesl");
    let queue = |shader: &Handle<Shader>, layout: &BindGroupLayoutDescriptor, entry: &'static str, format: TextureFormat, blend: Option<BlendState>| {
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
    // hdr_combine's `mix(frame, dirtMask, dirt * DirtBright)` plus the added bloom and flare: the
    // shader returns the added light in rgb and the dirt's grey level in alpha.
    let mix_add = BlendState {
        color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add },
        alpha: add.alpha,
    };
    let sprite = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("lens_flare_sprite".into()),
        layout: vec![sprite_layout.clone()],
        vertex: VertexState {
            shader: flare_shader.clone(),
            entry_point: Some("sprite_vertex".into()),
            buffers: vec![VertexBufferLayout::from_vertex_formats(
                VertexStepMode::Vertex,
                [VertexFormat::Float32x2, VertexFormat::Float32x2, VertexFormat::Float32x4],
            )],
            ..default()
        },
        fragment: Some(FragmentState {
            shader: flare_shader.clone(),
            entry_point: Some("sprite_fragment".into()),
            targets: vec![Some(ColorTargetState { format: HDR_FORMAT, blend: Some(add), write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    });
    let pipelines = BloomPipelines {
        down_first: queue(&shader, &layout, "down_first", FORMAT, None),
        down: queue(&shader, &layout, "down", FORMAT, None),
        blur_h: queue(&shader, &layout, "blur_h", FORMAT, None),
        blur_v: queue(&shader, &layout, "blur_v", FORMAT, None),
        combine: queue(&shader, &layout, "combine", HDR_FORMAT, Some(add)),
        backdrop: queue(&shader, &layout, "backdrop", BACKDROP_FORMAT, None),
        combine_flares: queue(&shader, &flares_layout, "combine_flares", HDR_FORMAT, Some(mix_add)),
        probe: queue(&probe_shader, &probe_layout, "probe", HDR_FORMAT, None),
        sprite,
        layout,
        flares_layout,
        probe_layout,
        sprite_layout,
        sampler,
        sampler_mip,
    };
    commands.insert_resource(pipelines);
}

/// The bloom chain (`a`), the horizontal-blur scratch chain (`b`) and the lens flare probe (one
/// texel per sun) of a view.
#[derive(Component)]
struct BloomTextures {
    a: CachedTexture,
    b: CachedTexture,
    probe: CachedTexture,
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
        let probe = cache.get(
            &device,
            TextureDescriptor {
                label: Some("lens_flare_probe"),
                size: Extent3d { width: MAX_FLARE_SOURCES as u32, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: HDR_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(BloomTextures { a, b, probe, levels });
    }
}

/// Little-endian bytes of a float slice (vertex data).
fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

#[allow(clippy::too_many_arguments)]
fn se_bloom(
    view: ViewQuery<(&ExtractedCamera, &ViewTarget, &BloomTextures, &DynamicUniformIndex<SeBloom>, &SeBloom, Option<&FlareDraw>)>,
    pipelines: Res<BloomPipelines>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<SeBloom>>,
    backdrop: Option<Res<GuiBackdrop>>,
    lens: Option<Res<LensTextures>>,
    fallback: Res<FallbackImage>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (camera, target, tex, index, settings, flares) = view.into_inner();
    let backdrop = backdrop.and_then(|b| gpu_images.get(&b.image));
    let bloom = settings.params.x > 0.0;
    let pseudo = settings.flare_bright.w > 0.0;
    let flares = flares.filter(|f| f.count > 0 && f.batches.iter().any(|b| !b.1.is_empty()));
    if !(bloom || pseudo || flares.is_some() || backdrop.is_some()) || !camera.hdr || target.main_texture_format() != HDR_FORMAT {
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
    let combine_flares = pipeline_cache.get_render_pipeline(pipelines.combine_flares);
    let probe_pipe = pipeline_cache.get_render_pipeline(pipelines.probe);
    let sprite_pipe = pipeline_cache.get_render_pipeline(pipelines.sprite);
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
    // Lens flares: the frame and the chain at each sun, then the sprites scaled by it (SE draws them
    // after the bloom textures are made, so they are not bloomed themselves).
    if let (Some(fd), Some(probe_pipe), Some(sprite_pipe)) = (flares, probe_pipe, sprite_pipe) {
        let probe_view = tex.probe.texture.create_view(&TextureViewDescriptor::default());
        let u = ProbeUniform { sources: fd.sources, params: Vec4::new(fd.frame_limit, fd.frame_gain, 0.0, 0.0) };
        let mut ub = encase::UniformBuffer::new(Vec::<u8>::new());
        if ub.write(&u).is_ok() {
            let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("lens_flare_probe_uniform"),
                contents: &ub.into_inner(),
                usage: BufferUsages::UNIFORM,
            });
            let layout = pipeline_cache.get_bind_group_layout(&pipelines.probe_layout);
            let bind = device.create_bind_group(
                "lens_flare_probe_group",
                &layout,
                &BindGroupEntries::sequential((&scene, &full_a, &pipelines.sampler_mip, buffer.as_entire_binding())),
            );
            let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("lens_flare_probe"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &probe_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations::default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(probe_pipe);
            p.set_bind_group(0, &bind, &[]);
            p.draw(0..3, 0..1);
        }
        let layout = pipeline_cache.get_bind_group_layout(&pipelines.sprite_layout);
        let draws: Vec<(BindGroup, Buffer, u32)> = fd
            .batches
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .filter_map(|(atlas, v)| {
                let atlas = gpu_images.get(*atlas)?;
                let bind = device.create_bind_group(
                    "lens_flare_sprite_group",
                    &layout,
                    &BindGroupEntries::sequential((&atlas.texture_view, &pipelines.sampler, &probe_view)),
                );
                let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("lens_flare_sprites"),
                    contents: &f32_bytes(v),
                    usage: BufferUsages::VERTEX,
                });
                Some((bind, buffer, (v.len() / FLARE_VERTEX_FLOATS) as u32))
            })
            .collect();
        if !draws.is_empty() {
            let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("lens_flare_sprites"),
                color_attachments: &[Some(target.get_unsampled_color_attachment())],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(sprite_pipe);
            for (bind, buffer, n) in &draws {
                p.set_bind_group(0, bind, &[]);
                p.set_vertex_buffer(0, *buffer.slice(..));
                p.draw(0..*n, 0..1);
            }
        }
    }
    // 3. Added to the frame, with the pseudo lens flare when it is on.
    let flare_combine = combine_flares.filter(|_| pseudo);
    if bloom || flare_combine.is_some() {
        let bind = match flare_combine {
            Some(_) => {
                let layout = pipeline_cache.get_bind_group_layout(&pipelines.flares_layout);
                let image = |id: Option<AssetId<Image>>| id.and_then(|i| gpu_images.get(i)).unwrap_or(&fallback.d2);
                let (dirt, rays) = lens.as_deref().map(|l| (l.dirt, l.rays)).unwrap_or_default();
                device.create_bind_group(
                    "se_bloom_flares_group",
                    &layout,
                    &BindGroupEntries::sequential((
                        &full_a,
                        &pipelines.sampler_mip,
                        binding.clone(),
                        &image(dirt).texture_view,
                        &image(rays).texture_view,
                    )),
                )
            }
            None => group(&full_a),
        };
        let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("se_bloom_combine"),
            color_attachments: &[Some(target.get_unsampled_color_attachment())],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        p.set_pipeline(flare_combine.unwrap_or(combine));
        p.set_bind_group(0, &bind, &[index.index()]);
        p.draw(0..3, 0..1);
    }
    encoder.pop_debug_group();
}
