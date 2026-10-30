//! SE's reduced render resolution of the solids and their upscaling (main-def.cfg `TextureRendering`,
//! the graphics presets): the 3D scene is drawn at `SolidResolution` of the window (FUN_1401910f0: a
//! screen taller than `SolidTargetRes` counts as that tall, both sides rounded down to even), FXAA'd
//! (`SolidFXAA`, fxaa.glsl) and upscaled to the window with `SolidUpscaleMode` (upscale_bicubic.glsl,
//! bicubic_common.glh `textureUpscaled`: nearest, bilinear, bicubic or lanczos, each plain, sharpened or
//! with AMD FidelityFX CAS), sharpened by `SolidSharpness`, or 1 - `PostEffects` `Sharpness` when
//! `SolidAutoSharp` (FUN_14018dae0). The interface (egui) is drawn after, at full resolution.
//!
//! The port renders Bevy's main pass (everything the main camera draws) into the top-left corner of the
//! main texture with `MainPassResolutionOverride`, then, before the passes that read or flip the main
//! texture (projection, black hole lens, bloom), FXAAs or copies that corner into a small scratch texture
//! and upscales it back over the whole main texture (no flip). SE draws only its solid objects at this
//! resolution; Bevy's main pass cannot be split, so the port's stars and sprites are upscaled too (their
//! pixel sizes still follow the window, `view.viewport`). Off while the warp pass reads the depth
//! buffer and in the cube-map projections. Not ported: the checkerboard modes 12 and 13 (drawn as
//! bicubic CAS) and `SolidTemporalUpscale` (kept as read).

use bevy::camera::{CameraUpdateSystems, MainPassResolutionOverride};
use bevy::core_pipeline::tonemapping::tonemapping;
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
use bevy::render::view::{ViewTarget, prepare_view_targets};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::camera::MainCamera;
use crate::color_correction::FinalImage;
use crate::graphics::GraphicsSettings;

/// The main texture of an HDR view.
const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// The interface names of `SolidUpscaleMode` 0..11 (main-def.cfg's comment).
pub const UPSCALE_MODES: [&str; 12] = [
    "Nearest",
    "Nearest sharp",
    "Nearest CAS",
    "Bilinear",
    "Bilinear sharp",
    "Bilinear CAS",
    "Bicubic",
    "Bicubic sharp",
    "Bicubic CAS",
    "Lanczos",
    "Lanczos sharp",
    "Lanczos CAS",
];

/// The solids' render size for a `w` x `h` view (FUN_1401910f0): the view scaled down so that its
/// shorter side is at most `target_res`, times `resolution`, each side rounded down to even.
pub fn solid_size(w: u32, h: u32, target_res: u32, resolution: f32) -> UVec2 {
    let (w, h, t) = (w as i64, h as i64, target_res as i64);
    let (mut sw, mut sh) = (w, h);
    if h < w {
        if t < h {
            sw = t * w / h;
            sh = t;
        }
    } else if t < w {
        sh = t * h / w;
        sw = t;
    }
    let side = |s: i64, full: i64| (((s as f32 * resolution) as i64) & !1).clamp(2, full.max(2)) as u32;
    UVec2::new(side(sw, w), side(sh, h))
}

/// `SolidUpscaleMode` as bicubic_common.glh's (`METHOD`, `SHARP`); the checkerboard modes 12, 13 are
/// not ported and draw as bicubic CAS.
pub fn upscale_method(mode: u32) -> (u32, u32) {
    if mode > 11 { (2, 2) } else { (mode / 3, mode % 3) }
}

/// main-def.cfg `TextureRendering`'s sharpness keys.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct SolidSharpness {
    /// `SolidSharpness`.
    pub sharpness: f32,
    /// `SolidAutoSharp`: use 1 - `PostEffects` `Sharpness` instead.
    pub auto: bool,
}

impl Default for SolidSharpness {
    fn default() -> Self {
        Self { sharpness: 0.0, auto: true }
    }
}

impl SolidSharpness {
    pub fn value(&self, final_sharpness: f32) -> f32 {
        if self.auto { (1.0 - final_sharpness).clamp(0.0, 1.0) } else { self.sharpness.clamp(0.0, 1.0) }
    }
}

/// Camera component of the pass; layout must match `upscale.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Debug, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct SolidUpscale {
    /// x `SolidResolution`, y `SolidTargetRes`, z the upscale sharpness, w on.
    pub params: Vec4,
    /// x METHOD, y SHARP, z `SolidFXAA`.
    pub mode: UVec4,
    /// xy the solids' size, zw the view's (pixels).
    pub size: Vec4,
}

impl SolidUpscale {
    fn low(&self) -> UVec2 {
        UVec2::new(self.size.x as u32, self.size.y as u32)
    }

    fn full(&self) -> UVec2 {
        UVec2::new(self.size.z as u32, self.size.w as u32)
    }

    /// Drawn at a reduced size this frame.
    fn active(&self) -> bool {
        self.params.w > 0.0 && self.low() != self.full() && self.low().cmpgt(UVec2::ONE).all()
    }
}

pub struct RenderScalePlugin;

impl Plugin for RenderScalePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((ExtractComponentPlugin::<SolidUpscale>::default(), UniformComponentPlugin::<SolidUpscale>::default()))
            .init_resource::<SolidSharpness>()
            .add_systems(Startup, load_sharpness)
            .add_systems(PostUpdate, sync_upscale.after(CameraUpdateSystems));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_upscale.in_set(RenderSystems::PrepareViews).before(prepare_view_targets))
            // Right after the main pass, before every pass that reads the main texture or flips it.
            .add_systems(
                Core3d,
                solid_upscale
                    .in_set(Core3dSystems::EarlyPostProcess)
                    .before(crate::projection::ProjectionPassSet)
                    .before(crate::blackholes::LensPassSet)
                    .before(crate::bloom::SeBloomSet)
                    .before(tonemapping)
                    .before(bevy::anti_alias::fxaa::fxaa)
                    .before(bevy::anti_alias::smaa::smaa),
            );
    }
}

fn load_sharpness(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<SolidSharpness>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    if let Some(v) = def.find_deep("SolidSharpness").and_then(|i| i.f64()) {
        s.sharpness = v as f32;
    }
    if let Some(v) = def.find_deep("SolidAutoSharp").and_then(|i| i.bool()) {
        s.auto = v;
    }
}

/// The settings and this frame's sizes onto the main camera. Env `SE_SOLID_RES=<0.1..1>` overrides
/// `SolidResolution` for tests.
fn sync_upscale(
    mut commands: Commands,
    gs: Res<GraphicsSettings>,
    fi: Res<FinalImage>,
    sharp: Res<SolidSharpness>,
    proj: Res<crate::projection::ProjectionSettings>,
    mut env_res: Local<Option<Option<f32>>>,
    mut cams: Query<(Entity, &Camera, Option<&mut SolidUpscale>), With<MainCamera>>,
) {
    let env = *env_res.get_or_insert_with(|| std::env::var("SE_SOLID_RES").ok().and_then(|v| v.parse::<f32>().ok()));
    let q = &gs.quality;
    let resolution = env.unwrap_or(q.solid_resolution).clamp(0.1, 1.0);
    let (method, sharp_mode) = upscale_method(q.solid_upscale_mode);
    for (e, camera, current) in &mut cams {
        let full = camera.physical_viewport_size().unwrap_or(UVec2::ONE);
        let low = solid_size(full.x, full.y, q.solid_target_res, resolution);
        let want = SolidUpscale {
            params: Vec4::new(resolution, q.solid_target_res as f32, sharp.value(fi.sharpness), (proj.mode == 0) as u32 as f32),
            mode: UVec4::new(method, sharp_mode, q.solid_fxaa as u32, 0),
            size: Vec4::new(low.x as f32, low.y as f32, full.x as f32, full.y as f32),
        };
        match current {
            Some(mut c) => {
                if *c != want {
                    *c = want;
                }
            }
            None => {
                commands.entity(e).insert(want);
            }
        }
    }
}

/// The solids' scratch texture (their size).
#[derive(Component)]
struct LowTexture(CachedTexture);

/// The main pass size: reduced while the view is the size the settings were computed for, the warp pass
/// (which reads the depth buffer at full size) is off.
fn prepare_upscale(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(
        Entity,
        &ExtractedCamera,
        &SolidUpscale,
        Option<&MainPassResolutionOverride>,
        Has<crate::ship::warp::WarpBubble>,
    )>,
) {
    for (entity, camera, up, current, warp) in &views {
        let on = up.active() && !warp && camera.physical_viewport_size == Some(up.full());
        if !on {
            if current.is_some() {
                commands.entity(entity).remove::<(MainPassResolutionOverride, LowTexture)>();
            }
            continue;
        }
        let low = up.low();
        if current.map(|c| c.0) != Some(low) {
            commands.entity(entity).insert(MainPassResolutionOverride(low));
        }
        let texture = cache.get(
            &device,
            TextureDescriptor {
                label: Some("solid_upscale_low"),
                size: Extent3d { width: low.x, height: low.y, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: HDR_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(LowTexture(texture));
    }
}

#[derive(Resource)]
struct UpscalePipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fxaa: CachedRenderPipelineId,
    upscale: CachedRenderPipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "solid_upscale_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<SolidUpscale>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let shader: Handle<Shader> = asset_server.load("shaders/upscale.wesl");
    let queue = |entry: &'static str| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("solid_upscale_{entry}").into()),
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
    let (fxaa, upscale) = (queue("fxaa"), queue("upscale"));
    commands.insert_resource(UpscalePipeline { layout, sampler, fxaa, upscale });
}

/// The solids' corner of the main texture into the scratch texture (FXAA'd or copied), then upscaled
/// back over the whole main texture.
fn solid_upscale(
    view: ViewQuery<(&ViewTarget, &SolidUpscale, &DynamicUniformIndex<SolidUpscale>, &LowTexture, &MainPassResolutionOverride)>,
    pipeline: Res<UpscalePipeline>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<SolidUpscale>>,
    mut ctx: RenderContext,
) {
    let (target, up, index, low, size) = view.into_inner();
    let (Some(fxaa), Some(upscale), Some(binding)) = (
        pipeline_cache.get_render_pipeline(pipeline.fxaa),
        pipeline_cache.get_render_pipeline(pipeline.upscale),
        uniforms.binding(),
    ) else {
        return;
    };
    let main_size = target.main_texture().size();
    if target.main_texture_format() != HDR_FORMAT
        || low.0.texture.size() != (Extent3d { width: size.0.x, height: size.0.y, depth_or_array_layers: 1 })
        || size.0.x > main_size.width
        || size.0.y > main_size.height
    {
        return;
    }
    let layout = pipeline_cache.get_bind_group_layout(&pipeline.layout);
    let device = ctx.render_device().clone();
    let group = |src: &TextureView| {
        device.create_bind_group("solid_upscale_group", &layout, &BindGroupEntries::sequential((src, &pipeline.sampler, binding.clone())))
    };
    let from_low = group(&low.0.default_view);
    if up.mode.z != 0 {
        // `SolidFXAA`: fxaa.glsl from the corner into the scratch texture.
        let from_main = group(target.main_texture_view());
        let attachment =
            RenderPassColorAttachment { view: &low.0.default_view, depth_slice: None, resolve_target: None, ops: Operations::default() };
        fullscreen(ctx.command_encoder(), attachment, fxaa, &from_main, index.index());
    } else {
        ctx.command_encoder().copy_texture_to_texture(
            target.main_texture().as_image_copy(),
            low.0.texture.as_image_copy(),
            Extent3d { width: size.0.x, height: size.0.y, depth_or_array_layers: 1 },
        );
    }
    fullscreen(ctx.command_encoder(), target.get_unsampled_color_attachment(), upscale, &from_low, index.index());
}

fn fullscreen(encoder: &mut CommandEncoder, attachment: RenderPassColorAttachment, pipeline: &RenderPipeline, bind: &BindGroup, offset: u32) {
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("solid_upscale"),
        color_attachments: &[Some(attachment)],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[offset]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_size_as_se() {
        // 1600 x 900 under the 1080-line target: 0.85 of the window, even.
        assert_eq!(solid_size(1600, 900, 1080, 0.85), UVec2::new(1360, 764));
        // 2560 x 1440: as if 1920 x 1080.
        assert_eq!(solid_size(2560, 1440, 1080, 0.5), UVec2::new(960, 540));
        // Portrait: the width is the shorter side.
        assert_eq!(solid_size(1440, 2560, 1080, 1.0), UVec2::new(1080, 1920));
        assert_eq!(solid_size(1920, 1080, 1440, 1.0), UVec2::new(1920, 1080));
    }

    #[test]
    fn upscale_modes() {
        assert_eq!(upscale_method(8), (2, 2));
        assert_eq!(upscale_method(3), (1, 0));
        assert_eq!(upscale_method(10), (3, 1));
        assert_eq!(upscale_method(12), (2, 2));
        assert_eq!(SolidSharpness::default().value(1.0), 0.0);
        assert_eq!(SolidSharpness { sharpness: 0.3, auto: false }.value(1.0), 0.3);
    }
}
