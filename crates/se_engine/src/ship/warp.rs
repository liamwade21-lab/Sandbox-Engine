//! The warp bubble: while the warp drive runs, SE draws an Alcubierre bubble around the ship with
//! its GR shader (einstein.glsl with WARP, FUN_140270900 called for the ship from FUN_140528000
//! while its WarpBoost (+0xdf4) is above 0). The view through the bubble is bent and shifted in
//! colour; the ship is kept in front of it. See `assets/shaders/warp_bubble.wesl`.
//!
//! SE's parameters (FUN_140270900, the ship branch; FUN_140534b40):
//! * `s` = |velocity| x WarpBoost (the log10 boost): MetricParams.x (warpV) = tanh(0.3 max(s,
//!   1e-9)), MetricParams.y (bubbleSig) = s. SE keeps the velocity in parsecs per second, which
//!   makes warpV vanish at every speed a ship reaches; the port takes it in km/s (assumed: the
//!   bubble SE draws is visible), relative to the frame the warp multiplies against;
//! * the shader's ray space is the ship's radius / 0.6 per unit (Params2.y = 0.6 / radius), the
//!   bubble wall at 1, the motion along +x; EyeDist x invGravRadius is half the camera's distance
//!   in it (eyePos = rayOrg x gravRadius / 2), so the ship's own image is bent only when the
//!   camera is outside the wall (warpFactor) and kept whole inside it (insideBubble);
//! * the impostor (render sphere) has `ShipWarpRadius` (10) ship radii;
//! * `ShipWarpQuality` (main-def.cfg 3; FUN_14018a420) sets how often the environment map around
//!   the ship is drawn: 4 every frame, 3 one face per frame, 2 every 0.5 s, 0-1 every 2 s. The port
//!   also traces at full resolution at 4, half below (SE: the single-pass mode at its top setting,
//!   the impostor map otherwise);
//! * `WarpPrecision` gives warpStep as for black holes.
//!
//! The environment map is the black holes' (`blackholes.rs` EnvCapture, six face cameras at the
//! camera); ships are drawn on [`SHIP_LAYER`] only, which those cameras do not see.

use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::math::{DMat3, DVec3};
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, GpuImage, TextureCache};
use bevy::render::view::{Msaa, ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::{Flight, Ship};
use crate::blackholes::{BlackHoleImages, EnvCapture};
use crate::camera::{FlyCamera, MainCamera};
use crate::universe::UPosition;

/// Render layer of every ship mesh: the main camera sees it, the environment cameras do not.
pub const SHIP_LAYER: usize = 7;
/// SE's ray space: ship radius / 0.6 per unit (DAT_1406d442c / radius).
const RAY_UNITS_PER_RADIUS: f64 = 0.6;
/// warpV = tanh(WARP_V_GAIN * max(s, WARP_S_MIN)) (DAT_1406d4908, DAT_1406d4170).
const WARP_V_GAIN: f64 = 0.3;
const WARP_S_MIN: f64 = 1e-9;
/// The bent-direction map.
const MAP_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// main-def.cfg WarpRendering: `ShipWarpQuality`, `ShipWarpRadius`, `WarpPrecision`.
#[derive(Resource, Clone, Copy, Debug)]
pub struct WarpSettings {
    pub quality: i32,
    pub radius: f64,
    pub precision: f64,
}

impl WarpSettings {
    /// main-def.cfg's values.
    const DEFAULT: Self = Self { quality: 3, radius: 10.0, precision: 0.5 };
}

impl FromWorld for WarpSettings {
    fn from_world(world: &mut World) -> Self {
        let cfg = world.get_resource::<crate::config::EngineConfig>();
        let key = |k: &str| cfg.and_then(|c| crate::stars::main_def_f64(c, k));
        let d = Self::DEFAULT;
        Self {
            quality: key("ShipWarpQuality").map_or(d.quality, |v| (v as i32).clamp(0, 4)),
            radius: key("ShipWarpRadius").filter(|v| *v > 0.0).unwrap_or(d.radius),
            precision: key("WarpPrecision").unwrap_or(d.precision),
        }
    }
}

impl WarpSettings {
    /// SE's warpStep: 0.2 x 10^(1 - WarpPrecision) (as `BlackHoleSettings`).
    fn warp_step(&self) -> f64 {
        0.2 * 10f64.powf(1.0 - self.precision.clamp(0.0, 1.0))
    }

    /// Environment map interval at this quality (FUN_14018a420): `None` re-renders it every
    /// frame (4) or a face per frame (3).
    fn env_interval(&self) -> Option<f64> {
        match self.quality {
            q if q >= 3 => None,
            2 => Some(0.5),
            _ => Some(2.0),
        }
    }

    /// Resolution of the traced map, in parts of the frame.
    fn map_scale(&self) -> u32 {
        if self.quality >= 4 { 1 } else { 2 }
    }
}

/// SE's MetricParams for a ship moving at `speed_kms` with warp boost `boost_log`: warpV and
/// bubbleSig.
pub fn metric_params(speed_kms: f64, boost_log: f64) -> (f64, f64) {
    let s = speed_kms * boost_log;
    ((WARP_V_GAIN * s.max(WARP_S_MIN)).tanh(), s)
}

/// Camera component driving the warp pass. Layout must match `Warp` in warp_bubble.wesl.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug)]
#[extract_app(RenderApp)]
pub struct WarpBubble {
    /// tan(fov/2) horizontally / vertically, environment map valid, map scale.
    pub view: Vec4,
    pub sky0: Vec4,
    pub sky1: Vec4,
    pub sky2: Vec4,
    pub bub0: Vec4,
    pub bub1: Vec4,
    pub bub2: Vec4,
    pub eye: Vec4,
    pub metric: Vec4,
}

pub struct WarpPlugin;

impl Plugin for WarpPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WarpSettings>()
            .add_systems(Update, setup_main_camera)
            .add_systems(
                PostUpdate,
                update_warp_bubble
                    .in_set(crate::blackholes::LensSet)
                    .after(crate::blackholes::LensStage::Lens)
                    .before(crate::blackholes::LensStage::EnvCams),
            )
            .add_plugins((ExtractComponentPlugin::<WarpBubble>::default(), UniformComponentPlugin::<WarpBubble>::default()));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_maps.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, warp_pass.after(crate::blackholes::LensPassSet).in_set(Core3dSystems::EarlyPostProcess));
    }
}

/// The main camera sees the ship layer, and its depth can be read by the warp pass.
fn setup_main_camera(mut commands: Commands, mut cams: Query<(Entity, &mut Camera3d, Has<RenderLayers>), With<MainCamera>>, mut done: Local<bool>) {
    if *done {
        return;
    }
    for (e, mut c3d, has_layers) in &mut cams {
        let usage = TextureUsages::from(c3d.depth_texture_usages) | TextureUsages::TEXTURE_BINDING;
        c3d.depth_texture_usages = usage.into();
        if !has_layers {
            commands.entity(e).insert(RenderLayers::from_layers(&[0, SHIP_LAYER]));
        }
        *done = true;
    }
}

/// An orthonormal frame whose x axis is `x` (rows: the frame's axes in world coordinates).
fn bubble_frame(x: DVec3) -> DMat3 {
    let x = x.normalize();
    let helper = if x.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
    let z = x.cross(helper).normalize();
    let y = z.cross(x);
    DMat3::from_cols(x, y, z).transpose()
}

#[allow(clippy::too_many_arguments)]
fn update_warp_bubble(
    mut commands: Commands,
    flight: Res<Flight>,
    settings: Res<WarpSettings>,
    time: Res<Time>,
    cam: Res<FlyCamera>,
    ships: Query<(&Ship, &UPosition)>,
    windows: Query<&Window>,
    cams: Query<(Entity, Has<WarpBubble>), With<MainCamera>>,
    mut env: ResMut<EnvCapture>,
    mut last_restart: Local<f64>,
) {
    let Some((cam_e, has)) = cams.iter().next() else { return };
    let ship = flight.ship.and_then(|e| ships.get(e).ok());
    let v = flight.info.warp_vel;
    let bubble = ship.filter(|(s, _)| s.warp > 0.0 && v.length() > 0.0 && std::env::var_os("SE_NO_WARP_BUBBLE").is_none());
    let Some((ship, pos)) = bubble else {
        if has {
            commands.entity(cam_e).remove::<WarpBubble>();
        }
        return;
    };
    let (warp_v, sigma) = metric_params(v.length() * 1e-3, ship.warp);
    let k = RAY_UNITS_PER_RADIUS / ship.radius.max(1e-3);
    let rel = cam.pos.delta_m(pos.0);
    let frame = bubble_frame(v);
    let eye = frame * rel * k;
    let view_to_world = DMat3::from_quat(cam.rot);
    let view_to_bubble = frame * view_to_world;

    // The environment map around the ship, as often as ShipWarpQuality asks.
    env.want = true;
    let now = time.elapsed_secs_f64();
    if !env.cams.is_empty() {
        let stale = env.valid == 0;
        let restart = match settings.env_interval() {
            None => settings.quality >= 4 || stale,
            Some(dt) => {
                env.paused = !env.restart;
                stale || now - *last_restart >= dt
            }
        };
        if restart {
            env.restart = true;
            env.paused = false;
            env.valid = 0;
            env.origin = Some(cam.pos);
            *last_restart = now;
        }
    }
    let env_ok = !env.cams.is_empty() && (env.restart || env.valid == 0b11_1111);

    let (w, h) = windows.iter().next().map(|w| (w.width() as f64, w.height() as f64)).unwrap_or((1280.0, 720.0));
    let tan_y = (cam.fov_deg.to_radians() as f64 * 0.5).tan();
    let col = |m: &DMat3, i: usize| m.col(i).as_vec3().extend(0.0);
    commands.entity(cam_e).insert(WarpBubble {
        view: Vec4::new((tan_y * w / h) as f32, tan_y as f32, if env_ok { 1.0 } else { 0.0 }, settings.map_scale() as f32),
        sky0: col(&view_to_world, 0),
        sky1: col(&view_to_world, 1),
        sky2: col(&view_to_world, 2),
        bub0: col(&view_to_bubble, 0),
        bub1: col(&view_to_bubble, 1),
        bub2: col(&view_to_bubble, 2),
        eye: eye.as_vec3().extend((eye.length() * 0.5) as f32),
        metric: Vec4::new(warp_v as f32, sigma.min(1e6) as f32, settings.warp_step() as f32, (settings.radius * RAY_UNITS_PER_RADIUS) as f32),
    });
}

// ---------------------------------------------------------------------------
// Render passes

#[derive(Resource)]
struct WarpPipelines {
    compute_layout: BindGroupLayoutDescriptor,
    render_layout: BindGroupLayoutDescriptor,
    render_layout_ms: BindGroupLayoutDescriptor,
    sampler: Sampler,
    compute: CachedRenderPipelineId,
    render: CachedRenderPipelineId,
    render_ms: CachedRenderPipelineId,
}

/// The view's bent-direction map.
#[derive(Component)]
struct WarpMap(CachedTexture);

fn render_layout_entries(ms: bool) -> Vec<BindGroupLayoutEntry> {
    let tex = || texture_2d(TextureSampleType::Float { filterable: true });
    let mut v = BindGroupLayoutEntries::with_indices(
        ShaderStages::FRAGMENT,
        (
            (0, tex()),
            (1, sampler(SamplerBindingType::Filtering)),
            (2, uniform_buffer::<WarpBubble>(true)),
            (3, tex()),
            (4, tex()),
            (5, tex()),
            (6, tex()),
            (7, tex()),
            (8, tex()),
            (9, tex()),
        ),
    )
    .to_vec();
    let depth = if ms { (11, texture_depth_2d_multisampled()) } else { (10, texture_depth_2d()) };
    v.extend(BindGroupLayoutEntries::with_indices(ShaderStages::FRAGMENT, (depth,)).to_vec());
    v
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let compute_layout = BindGroupLayoutDescriptor::new(
        "warp_compute_layout",
        &BindGroupLayoutEntries::with_indices(ShaderStages::FRAGMENT, ((2, uniform_buffer::<WarpBubble>(true)),)),
    );
    let render_layout = BindGroupLayoutDescriptor::new("warp_render_layout", &render_layout_entries(false));
    let render_layout_ms = BindGroupLayoutDescriptor::new("warp_render_layout_ms", &render_layout_entries(true));
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..Default::default()
    });
    let shader: Handle<Shader> = asset_server.load("shaders/warp_bubble.wesl");
    let queue = |label: &'static str, layout: &BindGroupLayoutDescriptor, entry: &'static str, format: TextureFormat| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        })
    };
    let compute = queue("warp_compute", &compute_layout, "compute", MAP_FORMAT);
    let render = queue("warp_render", &render_layout, "render", HDR_FORMAT);
    let render_ms = queue("warp_render_ms", &render_layout_ms, "render_ms", HDR_FORMAT);
    commands.insert_resource(WarpPipelines { compute_layout, render_layout, render_layout_ms, sampler, compute, render, render_ms });
}

fn prepare_maps(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    views: Query<(Entity, &bevy::render::camera::ExtractedCamera, &WarpBubble)>,
) {
    for (entity, camera, bubble) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let scale = (bubble.view.w as u32).max(1);
        let texture = texture_cache.get(
            &render_device,
            TextureDescriptor {
                label: Some("warp_map"),
                size: Extent3d { width: size.x.div_ceil(scale).max(1), height: size.y.div_ceil(scale).max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: MAP_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(WarpMap(texture));
    }
}

#[allow(clippy::too_many_arguments)]
fn warp_pass(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<WarpBubble>, &WarpMap, &ViewDepthStencilTexture, &Msaa)>,
    pipes: Res<WarpPipelines>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<WarpBubble>>,
    images: Option<Res<BlackHoleImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, index, map, depth, msaa) = view.into_inner();
    if target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let ms = msaa.samples() > 1;
    let (render_id, render_layout) = if ms { (pipes.render_ms, &pipes.render_layout_ms) } else { (pipes.render, &pipes.render_layout) };
    let (Some(binding), Some(compute), Some(render), Some(images)) = (
        uniforms.uniforms().binding(),
        pipeline_cache.get_render_pipeline(pipes.compute),
        pipeline_cache.get_render_pipeline(render_id),
        images,
    ) else {
        return;
    };
    let env: Vec<&GpuImage> = images.env.iter().filter_map(|h| gpu_images.get(h)).collect();
    if env.len() != 6 {
        return;
    }
    // Trace into the map.
    let compute_bind = ctx.render_device().create_bind_group(
        "warp_compute_group",
        &pipeline_cache.get_bind_group_layout(&pipes.compute_layout),
        &BindGroupEntries::with_indices(((2, binding.clone()),)),
    );
    {
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("warp_compute_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &map.0.default_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(Default::default()), store: StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(compute);
        pass.set_bind_group(0, &compute_bind, &[index.index()]);
        pass.draw(0..3, 0..1);
    }
    // Shade the frame.
    let post = target.post_process_write();
    let depth_view = &depth.attachment.texture.default_view;
    let depth_slot = if ms { 11 } else { 10 };
    let bind = ctx.render_device().create_bind_group(
        "warp_render_group",
        &pipeline_cache.get_bind_group_layout(render_layout),
        &BindGroupEntries::with_indices((
            (0, post.source),
            (1, &pipes.sampler),
            (2, binding),
            (3, &map.0.default_view),
            (4, &env[0].texture_view),
            (5, &env[1].texture_view),
            (6, &env[2].texture_view),
            (7, &env[3].texture_view),
            (8, &env[4].texture_view),
            (9, &env[5].texture_view),
            (depth_slot, depth_view),
        )),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("warp_render_pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(render);
    pass.set_bind_group(0, &bind, &[index.index()]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_params_follow_se() {
        // At rest the shift vanishes (warpV = tanh(0.3e-9)); it saturates at high s.
        let (v0, s0) = metric_params(0.0, 12.0);
        assert!(v0 < 1e-9 && s0 == 0.0);
        let (v, s) = metric_params(0.5, 2.0);
        assert!((v - 0.3f64.tanh()).abs() < 1e-12 && s == 1.0);
        assert!(metric_params(1e3, 15.0).0 > 0.999_999);
    }

    #[test]
    fn bubble_frame_is_orthonormal_with_x_along_motion() {
        for v in [DVec3::X, DVec3::new(0.0, 3.0, 0.0), DVec3::new(1.0, -2.0, 0.5)] {
            let m = bubble_frame(v);
            assert!((m * v.normalize() - DVec3::X).length() < 1e-12);
            assert!((m * m.transpose() - DMat3::IDENTITY).to_cols_array().iter().all(|x| x.abs() < 1e-12));
        }
    }

    #[test]
    fn quality_sets_env_interval_and_map_scale() {
        let q = |quality| WarpSettings { quality, ..WarpSettings::DEFAULT };
        assert_eq!((q(4).env_interval(), q(4).map_scale()), (None, 1));
        assert_eq!((q(3).env_interval(), q(3).map_scale()), (None, 2));
        assert_eq!(q(2).env_interval(), Some(0.5));
        assert_eq!(q(0).env_interval(), Some(2.0));
        assert!((q(3).warp_step() - 0.2 * 10f64.sqrt()).abs() < 1e-12);
    }
}
