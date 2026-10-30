//! A ship's environment map (SE's seEnvMapManager): a ship with a `Reflections` or `MaterialPBR`
//! material asks for one while it is over 1.5 pixels on screen (FUN_140534b40), and its
//! surroundings are drawn from its centre into a cube map with mipmaps (`EnvmapDimensions`,
//! `MipmapsEnv`). ship.glsl's ENVMAP reflects that map; for ship_pbr.glsl it is prefiltered into
//! EnvMapSpec (half size, pbr_spec_cubemap.glsl) and EnvMapDiff (`IrrmapDimensions`,
//! pbr_diff_cubemap.glsl), FUN_1402ff440.
//!
//! Six cameras at the ship draw the scene without the ships' render layer, one face per frame and
//! all six when the map starts or passes to another ship; `ship_env.wesl` then builds the cubes
//! before the main pass. The port keeps one map, for the reflecting ship largest on screen (SE has
//! a pool of `MaxEnvmaps` chains at three resolutions chosen by the ship's size; others here go
//! without, as SE's do when its pool runs out).

use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_2d_array, texture_cube, texture_storage_2d_array, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};
use bevy::transform::TransformSystems;

use super::{Ship, ShipVisuals};
use crate::camera::{FlyCamera, MainCamera};
use crate::ship_assets::{HullDefaults, HullMaterial, hull_flags};
use crate::universe::{UPosition, UniverseSet};

/// Each face camera's forward and up, in the cube's layer order (+x -x +y -y +z -z; `FWD` and `UP`
/// in ship_env.wesl).
const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::NEG_Z),
    (Vec3::NEG_Y, Vec3::Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// FUN_140534b40: a ship asks for its map above this many pixels of apparent size.
const MIN_PIXELS: f64 = 1.5;

pub struct ShipEnvPlugin;

impl Plugin for ShipEnvPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShipEnv>()
            .init_resource::<ShipEnvFrame>()
            .add_plugins(ExtractResourcePlugin::<ShipEnvFrame>::default())
            .add_systems(
                PostUpdate,
                update_ship_env.after(UniverseSet::FloatingOrigin).before(TransformSystems::Propagate),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Core3d, ship_env_pass.in_set(Core3dSystems::Prepass));
    }
}

#[derive(Resource, Default)]
struct ShipEnv {
    cams: Vec<Entity>,
    faces: Vec<Handle<Image>>,
    /// The ship the map is drawn for.
    target: Option<Entity>,
    /// Bit i: face i holds the target's surroundings.
    valid: u8,
    next: usize,
}

#[derive(Component)]
struct ShipEnvCam(usize);

/// What the render world builds this frame: the faces drawn so far into the cubes.
#[derive(Resource, ExtractResource, Clone, Default)]
#[extract_app(RenderApp)]
pub struct ShipEnvFrame {
    pub run: bool,
    pub faces: Vec<Handle<Image>>,
    pub env: Handle<Image>,
    pub spec: Handle<Image>,
    pub diff: Handle<Image>,
}

/// Apparent radius in pixels of a ship `radius_m` across at `dist_m`.
fn apparent_pixels(radius_m: f64, dist_m: f64, px_per_rad: f64) -> f64 {
    radius_m.max(0.0) * px_per_rad / dist_m.max(1e-3)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_ship_env(
    mut commands: Commands,
    mut env: ResMut<ShipEnv>,
    mut frame: ResMut<ShipEnvFrame>,
    defaults: Res<HullDefaults>,
    cam: Res<FlyCamera>,
    windows: Query<&Window>,
    main: Query<&Projection, (With<MainCamera>, Without<ShipEnvCam>)>,
    ships: Query<(Entity, &Ship, &UPosition, &Transform, &ShipVisuals), Without<ShipEnvCam>>,
    mut hull_mats: ResMut<Assets<HullMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut faces: Query<(&ShipEnvCam, &mut Camera, &mut Transform, &mut Projection), Without<Ship>>,
) {
    let main_p = main.iter().next().and_then(|p| if let Projection::Perspective(p) = p { Some(p.clone()) } else { None });
    let h = windows.iter().next().map(|w| w.height() as f64).unwrap_or(900.0);
    let fov = main_p.as_ref().map_or(std::f32::consts::FRAC_PI_4, |p| p.fov) as f64;
    let px_per_rad = h * 0.5 / (fov * 0.5).tan();

    // The reflecting ship largest on screen.
    let mut best: Option<(Entity, f64, Vec3)> = None;
    for (e, ship, pos, tf, vis) in &ships {
        let reflects = vis.hull.iter().any(|m| hull_mats.get(m).is_some_and(|m| m.u.flags.x & hull_flags::REFLECT != 0));
        if !reflects {
            continue;
        }
        let px = apparent_pixels(ship.radius, pos.0.delta_m(cam.pos).length(), px_per_rad);
        if px > MIN_PIXELS && best.is_none_or(|b| px > b.1) {
            best = Some((e, px, tf.translation));
        }
    }
    let target = best.map(|b| b.0);
    let restart = target.is_some() && target != env.target;
    if target != env.target {
        env.valid = 0;
        env.target = target;
    }

    if let Some((_, _, at)) = best {
        if env.cams.is_empty() {
            // Spawned on first use (after the main camera, so egui keeps the main camera's context).
            let size = defaults.env_size;
            for (i, (fwd, up)) in FACES.iter().enumerate() {
                let image = images.add(Image::new_target_texture(size, size, HDR_FORMAT, None));
                env.faces.push(image.clone());
                let e = commands
                    .spawn((
                        Camera3d::default(),
                        Camera { order: -20 - i as isize, is_active: false, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
                        bevy::camera::RenderTarget::Image(image.into()),
                        bevy::camera::Hdr,
                        bevy::core_pipeline::tonemapping::Tonemapping::None,
                        bevy::core_pipeline::tonemapping::DebandDither::Disabled,
                        bevy::render::view::Msaa::Off,
                        Projection::Perspective(PerspectiveProjection { fov: std::f32::consts::FRAC_PI_2, aspect_ratio: 1.0, near: 0.05, ..default() }),
                        Transform::from_translation(at).looking_to(*fwd, *up),
                        // The scene without the ships (`warp::SHIP_LAYER`).
                        RenderLayers::layer(0),
                        ShipEnvCam(i),
                    ))
                    .id();
                env.cams.push(e);
            }
            env.valid = 0;
        } else {
            let all = restart || env.valid == 0;
            let next = env.next;
            for (face, mut c, mut tf, mut proj) in &mut faces {
                let on = all || face.0 == next;
                c.is_active = on;
                let (fwd, up) = FACES[face.0];
                *tf = Transform::from_translation(at).looking_to(fwd, up);
                if let (Projection::Perspective(p), Some(m)) = (proj.as_mut(), main_p.as_ref()) {
                    p.near = m.near;
                }
            }
            env.valid |= if all { 0b11_1111 } else { 1 << next };
            env.next = (next + 1) % 6;
        }
    } else if env.valid != 0 || faces.iter().any(|f| f.1.is_active) {
        env.valid = 0;
        for (_, mut c, _, _) in &mut faces {
            c.is_active = false;
        }
    }

    // The cubes hold the target's surroundings once all six faces are drawn.
    let ready = env.valid == 0b11_1111 && env.target.is_some();
    frame.run = target.is_some() && !env.faces.is_empty() && faces.iter().any(|f| f.1.is_active);
    if frame.faces.is_empty() && !env.faces.is_empty() {
        *frame = ShipEnvFrame {
            run: frame.run,
            faces: env.faces.clone(),
            env: defaults.env_map.clone(),
            spec: defaults.env_spec.clone(),
            diff: defaults.env_diff.clone(),
        };
    }

    // ENVMAP / image-based light on the target's materials only, with FUN_14029f200's EnvMapParams:
    // the specular cube's resolution and last level (512 and 9 without a map).
    let spec_size = (defaults.env_size / 2).max(1);
    let on_params = Vec2::new(spec_size as f32, spec_size.ilog2() as f32);
    for (e, _, _, _, vis) in &ships {
        let on = ready && Some(e) == env.target;
        let params = if on { on_params } else { Vec2::new(512.0, 9.0) };
        for h in &vis.hull {
            let Some(m) = hull_mats.get(h) else { continue };
            let reflects = m.u.flags.x & hull_flags::REFLECT != 0;
            let want = on && reflects;
            if (m.u.flags.x & hull_flags::ENV != 0) == want && m.u.env.truncate().truncate() == params {
                continue;
            }
            if let Some(mut m) = hull_mats.get_mut(h) {
                m.u.flags.x = if want { m.u.flags.x | hull_flags::ENV } else { m.u.flags.x & !hull_flags::ENV };
                (m.u.env.x, m.u.env.y) = (params.x, params.y);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Render world

#[derive(Resource)]
struct ShipEnvPipelines {
    copy_layout: BindGroupLayoutDescriptor,
    down_layout: BindGroupLayoutDescriptor,
    filter_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    copy: CachedComputePipelineId,
    down: CachedComputePipelineId,
    spec: CachedComputePipelineId,
    irr: CachedComputePipelineId,
}

fn init_pipelines(mut commands: Commands, device: Res<bevy::render::renderer::RenderDevice>, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let raw = TextureSampleType::Float { filterable: false };
    let store = texture_storage_2d_array(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly);
    let copy_layout = BindGroupLayoutDescriptor::new(
        "ship_env_copy",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, texture_2d(raw)),
                (1, texture_2d(raw)),
                (2, texture_2d(raw)),
                (3, texture_2d(raw)),
                (4, texture_2d(raw)),
                (5, texture_2d(raw)),
                (6, store),
            ),
        ),
    );
    let down_layout = BindGroupLayoutDescriptor::new(
        "ship_env_down",
        &BindGroupLayoutEntries::with_indices(ShaderStages::COMPUTE, ((7, texture_2d_array(raw)), (8, store))),
    );
    let filter_layout = BindGroupLayoutDescriptor::new(
        "ship_env_filter",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (9, texture_cube(TextureSampleType::Float { filterable: true })),
                (10, sampler(SamplerBindingType::Filtering)),
                (11, store),
                (12, uniform_buffer::<Vec4>(false)),
            ),
        ),
    );
    let shader: Handle<Shader> = assets.load("shaders/ship_env.wesl");
    let queue = |layout: &BindGroupLayoutDescriptor, entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("ship_env_{entry}").into()),
            layout: vec![layout.clone()],
            shader: shader.clone(),
            entry_point: Some(entry.into()),
            ..default()
        })
    };
    let pipes = ShipEnvPipelines {
        copy: queue(&copy_layout, "copy_faces"),
        down: queue(&down_layout, "downsample"),
        spec: queue(&filter_layout, "prefilter_spec"),
        irr: queue(&filter_layout, "irradiance"),
        sampler: device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..default()
        }),
        copy_layout,
        down_layout,
        filter_layout,
    };
    commands.insert_resource(pipes);
}

/// One level of a cube as a 2D array, for a storage write or a load.
fn level_view(gpu: &GpuImage, level: u32) -> TextureView {
    gpu.texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        base_mip_level: level,
        mip_level_count: Some(1),
        ..default()
    })
}

/// FUN_1402ff440's Params: the source's resolution, 6 res^2 / 4 pi, roughness^4 (EnvMapSpec level
/// i of n: ((i + 1) / n)^4), 1 / the source's exposure (1: the port's map is drawn as the scene).
fn filter_params(source_size: u32, level: u32, levels: u32) -> Vec4 {
    let s = source_size as f32;
    let r = (level + 1) as f32 / levels.max(1) as f32;
    Vec4::new(s, 6.0 * s * s / (4.0 * std::f32::consts::PI), r.powi(4), 1.0)
}

fn ship_env_pass(
    // The main view only (the environment cameras have no colour correction).
    _view: ViewQuery<&ViewTarget, (With<crate::color_correction::ColorCorrection>, Without<crate::capture::CaptureView>)>,
    frame: Option<Res<ShipEnvFrame>>,
    pipes: Option<Res<ShipEnvPipelines>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (Some(frame), Some(pipes)) = (frame, pipes) else { return };
    if !frame.run || frame.faces.len() != 6 {
        return;
    }
    let (Some(copy), Some(down), Some(spec_p), Some(irr_p)) = (
        cache.get_compute_pipeline(pipes.copy),
        cache.get_compute_pipeline(pipes.down),
        cache.get_compute_pipeline(pipes.spec),
        cache.get_compute_pipeline(pipes.irr),
    ) else {
        return;
    };
    let faces: Vec<&GpuImage> = frame.faces.iter().filter_map(|h| gpu_images.get(h)).collect();
    let (Some(env), Some(spec), Some(diff)) = (gpu_images.get(&frame.env), gpu_images.get(&frame.spec), gpu_images.get(&frame.diff)) else {
        return;
    };
    if faces.len() != 6 {
        return;
    }
    let device = ctx.render_device().clone();
    let size = env.texture.width();
    let groups = |n: u32| n.div_ceil(8);

    let copy_layout = cache.get_bind_group_layout(&pipes.copy_layout);
    let down_layout = cache.get_bind_group_layout(&pipes.down_layout);
    let filter_layout = cache.get_bind_group_layout(&pipes.filter_layout);
    let env_levels: Vec<TextureView> = (0..env.texture.mip_level_count()).map(|l| level_view(env, l)).collect();
    let copy_group = device.create_bind_group(
        "ship_env_copy",
        &copy_layout,
        &BindGroupEntries::with_indices((
            (0, &faces[0].texture_view),
            (1, &faces[1].texture_view),
            (2, &faces[2].texture_view),
            (3, &faces[3].texture_view),
            (4, &faces[4].texture_view),
            (5, &faces[5].texture_view),
            (6, &env_levels[0]),
        )),
    );
    let down_groups: Vec<BindGroup> = (1..env_levels.len())
        .map(|l| device.create_bind_group("ship_env_down", &down_layout, &BindGroupEntries::with_indices(((7, &env_levels[l - 1]), (8, &env_levels[l])))))
        .collect();
    let filter_group = |dst: &TextureView, params: Vec4| {
        let bytes: Vec<u8> = params.to_array().iter().flat_map(|f| f.to_le_bytes()).collect();
        let buf = device.create_buffer_with_data(&BufferInitDescriptor { label: Some("ship_env_params"), contents: &bytes, usage: BufferUsages::UNIFORM });
        device.create_bind_group(
            "ship_env_filter",
            &filter_layout,
            &BindGroupEntries::with_indices(((9, &env.texture_view), (10, &pipes.sampler), (11, dst), (12, buf.as_entire_binding()))),
        )
    };
    let spec_levels = spec.texture.mip_level_count();
    let spec_groups: Vec<(BindGroup, u32)> = (0..spec_levels)
        .map(|l| (filter_group(&level_view(spec, l), filter_params(size, l, spec_levels)), (spec.texture.width() >> l).max(1)))
        .collect();
    let irr_size = diff.texture.width();
    let irr_group = filter_group(&level_view(diff, 0), filter_params(size, 0, 1));

    let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("ship_env"), ..default() });
    pass.set_pipeline(copy);
    pass.set_bind_group(0, &copy_group, &[]);
    pass.dispatch_workgroups(groups(size), groups(size), 6);
    pass.set_pipeline(down);
    for (l, g) in down_groups.iter().enumerate() {
        let s = (size >> (l + 1)).max(1);
        pass.set_bind_group(0, g, &[]);
        pass.dispatch_workgroups(groups(s), groups(s), 6);
    }
    pass.set_pipeline(spec_p);
    for (g, s) in &spec_groups {
        pass.set_bind_group(0, g, &[]);
        pass.dispatch_workgroups(groups(*s), groups(*s), 6);
    }
    pass.set_pipeline(irr_p);
    pass.set_bind_group(0, &irr_group, &[]);
    pass.dispatch_workgroups(groups(irr_size), groups(irr_size), 6);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_levels_follow_se() {
        // EnvMapSpec of a 512 map: 256 wide, 9 levels; level 0 is roughness 1/9, the last 1.
        let p = filter_params(512, 0, 9);
        assert!((p.y - 6.0 * 512.0 * 512.0 / (4.0 * std::f32::consts::PI)).abs() < 1.0);
        assert!((p.z - (1.0f32 / 9.0).powi(4)).abs() < 1e-9);
        assert_eq!(filter_params(512, 8, 9).z, 1.0);
        // A 30 m ship 1 km away at 1000 px per radian is 15 px: it gets a map; at 20 km it does not.
        assert!(apparent_pixels(15.0, 1000.0, 1000.0) > MIN_PIXELS);
        assert!(apparent_pixels(15.0, 20_000.0, 1000.0) < MIN_PIXELS);
    }
}
