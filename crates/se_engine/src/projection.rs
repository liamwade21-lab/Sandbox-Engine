//! SE's projection modes (main-def.cfg `Projection` `ProjectionMode`: 0 perspective, 1 cubic, 2 fish eye,
//! 3 cylinder; Settings > Projection, "Configure projection"). Outside perspective SE renders a cubemap of
//! `CubeFaceResolution` (512) and resamples it into the frame: `proj_fisheye.glsl` (`Param` = lens angle
//! `FisheyeFOV` / 2 and cut-off `FisheyeCrop`, `Offset` = `FisheyeOffset` and `FisheyeZoom`, `Warp` =
//! `FisheyeWarp`, `FisheyeFlipX/Y`; FUN_140596a20 inverts it for picking), `proj_cylinder.glsl` (a 360 x
//! 180 degree map centred ahead) or, in the cubic mode, a mosaic of the faces (`CubemapFormat`,
//! `ShowCubempEdges`, `FitCubemapToScreen`).
//!
//! The port renders the faces with six `capture.rs` views (raw HDR, no post-processing) and draws the
//! resampled frame into the main camera's main texture before its post-processing (bloom, exposure,
//! tone mapping), as SE post-processes the projected frame. The main camera then renders an empty layer.
//! The pass writes the main texture directly (no flip) and exists only while a projection is on: in
//! perspective there are no views and no pass. The interface is drawn flat over the frame (SE's
//! `DistortInterface` off); `DistortInterface`, `FisheyeGUIAngle`, `GUIResolution`, `LensFlareResolution`
//! and `ShowBackground` are kept and saved but not used.

use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};
use bevy::window::PrimaryWindow;

use crate::camera::MainCamera;
use crate::capture::{CaptureView, Post, SceneLayers};

/// SE's projection modes ("Perspective", "Cubemap", "Fish eye", "Cylindrical").
pub const MODES: [&str; 4] = ["Perspective", "Cubemap", "Fish eye", "Cylindrical"];

/// SE's cubemap frame formats (`CubemapFormat`, "Frame format").
pub const CUBEMAP_FORMATS: [&str; 6] = [
    "4x3 Cubemap",
    "3x2 Youtube, GoPro, Gear VR, AirPano",
    "3x2 Facebook",
    "3x2 Pano2VR, GardenGnome",
    "6x1 GoPro",
    "6x1 Photoshop DDS plugin",
];

/// The layer the main camera renders while a projection is on: nothing is on it.
const EMPTY_LAYER: usize = 31;

/// Range of `CubeFaceResolution` offered.
pub const FACE_RES_RANGE: (u32, u32) = (128, 4096);

/// SE's `Projection` keys (main-def.cfg; ranges from FUN_140326220).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ProjectionSettings {
    /// `ProjectionMode`, index into [`MODES`].
    pub mode: u8,
    /// `CubeFaceResolution`.
    pub cube_face_res: u32,
    /// `GUIResolution` (kept).
    pub gui_res: u32,
    /// `LensFlareResolution` (kept).
    pub flare_res: u32,
    /// `FisheyeFOV` (deg, 40..360).
    pub fisheye_fov: f32,
    /// `FisheyeCrop` (1..2): the circle's cut-off radius.
    pub fisheye_crop: f32,
    /// `FisheyeGUIAngle` (deg, -180..180; kept).
    pub fisheye_gui_angle: f32,
    /// `FisheyeZoom` (1..3).
    pub fisheye_zoom: f32,
    pub fisheye_offset: Vec2,
    pub fisheye_warp: Vec3,
    pub fisheye_flip_x: bool,
    pub fisheye_flip_y: bool,
    /// `DistortInterface` (kept).
    pub distort_interface: bool,
    /// `ShowBackground` (kept).
    pub show_background: bool,
    /// `ShowCubempEdges`.
    pub show_edges: bool,
    /// `FitCubemapToScreen`: square faces, else stretched.
    pub fit_to_screen: bool,
    /// `CubemapFormat`, index into [`CUBEMAP_FORMATS`].
    pub cubemap_format: u8,
}

impl Default for ProjectionSettings {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self {
            mode: 0,
            cube_face_res: 512,
            gui_res: 1280,
            flare_res: 512,
            fisheye_fov: 180.0,
            fisheye_crop: 1.0,
            fisheye_gui_angle: 90.0,
            fisheye_zoom: 1.5,
            fisheye_offset: Vec2::ZERO,
            fisheye_warp: Vec3::ZERO,
            fisheye_flip_x: false,
            fisheye_flip_y: false,
            distort_interface: false,
            show_background: false,
            show_edges: true,
            fit_to_screen: true,
            cubemap_format: 0,
        }
    }
}

impl ProjectionSettings {
    /// Read SE's keys from a `Projection` block (main-def.cfg or the user's settings).
    pub fn read(&mut self, b: &se_core::sc::Block) {
        let num = |k: &str| b.find_deep(k).and_then(|i| i.f64());
        let flag = |k: &str| b.find_deep(k).and_then(|i| i.bool());
        let vec = |k: &str| b.find_deep(k).map(|i| i.nums()).filter(|v| !v.is_empty());
        if let Some(v) = num("ProjectionMode") {
            self.mode = v.clamp(0.0, 3.0) as u8;
        }
        if let Some(v) = num("CubeFaceResolution") {
            self.cube_face_res = (v as u32).clamp(FACE_RES_RANGE.0, FACE_RES_RANGE.1);
        }
        if let Some(v) = num("GUIResolution") {
            self.gui_res = v as u32;
        }
        if let Some(v) = num("LensFlareResolution") {
            self.flare_res = v as u32;
        }
        if let Some(v) = num("FisheyeFOV") {
            self.fisheye_fov = (v as f32).clamp(40.0, 360.0);
        }
        if let Some(v) = num("FisheyeCrop") {
            self.fisheye_crop = (v as f32).clamp(1.0, 2.0);
        }
        if let Some(v) = num("FisheyeGUIAngle") {
            self.fisheye_gui_angle = (v as f32).clamp(-180.0, 180.0);
        }
        if let Some(v) = num("FisheyeZoom") {
            self.fisheye_zoom = (v as f32).clamp(1.0, 3.0);
        }
        if let Some(v) = vec("FisheyeOffset") {
            self.fisheye_offset = Vec2::new(v[0] as f32, v.get(1).copied().unwrap_or(0.0) as f32);
        }
        if let Some(v) = vec("FisheyeWarp") {
            self.fisheye_warp = Vec3::new(v[0] as f32, v.get(1).copied().unwrap_or(0.0) as f32, v.get(2).copied().unwrap_or(0.0) as f32);
        }
        for (k, f) in [
            ("FisheyeFlipX", &mut self.fisheye_flip_x),
            ("FisheyeFlipY", &mut self.fisheye_flip_y),
            ("DistortInterface", &mut self.distort_interface),
            ("ShowBackground", &mut self.show_background),
            ("ShowCubempEdges", &mut self.show_edges),
            ("FitCubemapToScreen", &mut self.fit_to_screen),
        ] {
            if let Some(v) = flag(k) {
                *f = v;
            }
        }
        if let Some(v) = num("CubemapFormat") {
            self.cubemap_format = v.clamp(0.0, 5.0) as u8;
        }
    }

    /// The `Projection` block of the user's settings.
    pub fn to_cfg(&self) -> String {
        let o = self.fisheye_offset;
        let w = self.fisheye_warp;
        format!(
            "Projection\n{{\n\tProjectionMode    {}\n\tGUIResolution     {}\n\tCubeFaceResolution {}\n\tLensFlareResolution {}\n\tFisheyeFOV        {}\n\tFisheyeCrop       {}\n\tFisheyeGUIAngle   {}\n\tFisheyeZoom       {}\n\tFisheyeOffset     ( {} {} )\n\tFisheyeWarp       ( {} {} {} )\n\tFisheyeFlipX      {}\n\tFisheyeFlipY      {}\n\tDistortInterface  {}\n\tShowBackground    {}\n\tShowCubempEdges   {}\n\tFitCubemapToScreen {}\n\tCubemapFormat     {}\n}}\n",
            self.mode,
            self.gui_res,
            self.cube_face_res,
            self.flare_res,
            self.fisheye_fov,
            self.fisheye_crop,
            self.fisheye_gui_angle,
            self.fisheye_zoom,
            o.x,
            o.y,
            w.x,
            w.y,
            w.z,
            self.fisheye_flip_x,
            self.fisheye_flip_y,
            self.distort_interface,
            self.show_background,
            self.show_edges,
            self.fit_to_screen,
            self.cubemap_format
        )
    }

    /// The pass's uniform for a `size` (physical pixels) frame.
    pub fn uniform(&self, size: Vec2) -> ProjUniform {
        let flags = self.fisheye_flip_x as u32 | (self.fisheye_flip_y as u32) << 1 | (self.show_edges as u32) << 2 | (self.fit_to_screen as u32) << 3;
        ProjUniform {
            mode: UVec4::new(self.mode as u32, flags, self.cubemap_format as u32, 0),
            fish: Vec4::new((self.fisheye_fov * 0.5).to_radians(), self.fisheye_crop, self.fisheye_zoom, size.x / size.y.max(1.0)),
            offset: self.fisheye_offset.extend(0.0).extend(0.0),
            warp: self.fisheye_warp.extend(0.0),
            screen: Vec4::new(size.x, size.y, self.cube_face_res as f32, 0.0),
        }
    }
}

/// SE's fish-eye ray (`proj_fisheye.glsl`, camera space x right, y up, -z ahead) for a point of the
/// frame (`ndc` -1..1, y up), or none outside the crop circle. SE's frame is x ahead, y up, z left; its
/// texture coordinates run down with a -1 sign unflipped, so unflipped the picture is upright here.
/// The CPU twin of `projection.wesl` `fisheye`, for the tests.
#[cfg(test)]
pub fn fisheye_ray(s: &ProjectionSettings, ndc: Vec2, aspect: f32) -> Option<Vec3> {
    let sx = if s.fisheye_flip_x { -1.0 } else { 1.0 };
    let sy = if s.fisheye_flip_y { -1.0 } else { 1.0 };
    let p = Vec2::new((ndc.x + s.fisheye_offset.x) * aspect * sx, (ndc.y - s.fisheye_offset.y) * sy) / s.fisheye_zoom;
    let r = p.length();
    if r >= s.fisheye_crop {
        return None;
    }
    let lens = (s.fisheye_fov * 0.5).to_radians();
    let a = lens * r;
    let k = if r > 1e-6 { a.sin() / r } else { lens };
    let w = s.fisheye_warp;
    Some(Vec3::new(k * p.x - w.z, k * p.y + w.y, -(a.cos() + w.x)))
}

/// The mosaic cell (column, row) of each face ([`crate::capture::FACES`] order) and the grid of a
/// `CubemapFormat`. The face order of each named layout follows that layout's convention; SE's own
/// placement was not traced.
pub fn mosaic_layout(format: u8) -> (UVec2, [UVec2; 6]) {
    let c = |x: u32, y: u32| UVec2::new(x, y);
    // Faces: 0 right, 1 left, 2 up, 3 down, 4 ahead, 5 behind.
    match format {
        // Horizontal cross: up over ahead; left, ahead, right, behind; down.
        0 => (c(4, 3), [c(2, 1), c(0, 1), c(1, 0), c(1, 2), c(1, 1), c(3, 1)]),
        // Left, ahead, right / down, behind, up.
        1 => (c(3, 2), [c(2, 0), c(0, 0), c(2, 1), c(0, 1), c(1, 0), c(1, 1)]),
        // Right, left, up / down, ahead, behind.
        2 => (c(3, 2), [c(0, 0), c(1, 0), c(2, 0), c(0, 1), c(1, 1), c(2, 1)]),
        // Ahead, right, behind / left, up, down.
        3 => (c(3, 2), [c(1, 0), c(0, 1), c(1, 1), c(2, 1), c(0, 0), c(2, 0)]),
        // Ahead, right, behind, left, up, down.
        4 => (c(6, 1), [c(1, 0), c(3, 0), c(4, 0), c(5, 0), c(0, 0), c(2, 0)]),
        // +X, -X, +Y, -Y, +Z, -Z.
        _ => (c(6, 1), [c(0, 0), c(1, 0), c(2, 0), c(3, 0), c(4, 0), c(5, 0)]),
    }
}

/// The pass's settings; layout must match `projection.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Debug, PartialEq)]
#[extract_app(RenderApp)]
pub struct ProjUniform {
    /// x the mode, y flags (1 flip x, 2 flip y, 4 face edges, 8 fit to screen), z the cubemap format.
    pub mode: UVec4,
    /// x lens angle (rad, FisheyeFOV / 2), y crop, z zoom, w frame aspect.
    pub fish: Vec4,
    /// xy `FisheyeOffset`.
    pub offset: Vec4,
    /// xyz `FisheyeWarp`.
    pub warp: Vec4,
    /// xy frame size (px), z face size.
    pub screen: Vec4,
}

/// The face images, on the main camera while a projection is on.
#[derive(Component, ExtractComponent, Clone, Debug)]
#[extract_app(RenderApp)]
pub struct ProjFaces(pub [Handle<Image>; 6]);

/// The face views of the projection.
#[derive(Resource, Default)]
struct Rig {
    views: Vec<(Entity, Handle<Image>)>,
    res: u32,
    /// The main camera's layers before it was emptied (`None`: it had none).
    main_layers: Option<Option<RenderLayers>>,
}

fn load_settings(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<ProjectionSettings>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    if let Some(b) = def.find_deep("Projection").and_then(|i| i.block()) {
        s.read(b);
    }
    if let Some(m) = std::env::var("SE_PROJECTION").ok().and_then(|v| v.parse::<u8>().ok()) {
        s.mode = m.min(3);
    }
}

/// Spawn or drop the face views with the mode, empty or restore the main camera's layers, and keep
/// the pass's uniform current.
#[allow(clippy::too_many_arguments)]
fn apply_projection(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    settings: Res<ProjectionSettings>,
    mut rig: ResMut<Rig>,
    mut scene: ResMut<SceneLayers>,
    windows: Query<&Window, With<PrimaryWindow>>,
    main: Query<(Entity, Option<&RenderLayers>, Option<&ProjUniform>), (With<MainCamera>, Without<CaptureView>)>,
) {
    let Ok((cam, layers, uniform)) = main.single() else { return };
    let on = settings.mode != 0;
    if !on {
        if rig.views.is_empty() && rig.main_layers.is_none() {
            return;
        }
        crate::capture::despawn_views(&mut commands, &mut images, &rig.views);
        rig.views.clear();
        let mut ec = commands.entity(cam);
        ec.remove::<(ProjUniform, ProjFaces)>();
        match rig.main_layers.take() {
            Some(Some(l)) => {
                ec.insert(l);
            }
            Some(None) => {
                ec.remove::<RenderLayers>();
            }
            None => {}
        }
        scene.0 = None;
        return;
    }
    if rig.main_layers.is_none() {
        rig.main_layers = Some(layers.cloned());
        scene.0 = Some(layers.cloned().unwrap_or_default());
        commands.entity(cam).insert(RenderLayers::layer(EMPTY_LAYER));
    }
    let res = settings.cube_face_res.clamp(FACE_RES_RANGE.0, FACE_RES_RANGE.1);
    if rig.views.is_empty() || rig.res != res {
        crate::capture::despawn_views(&mut commands, &mut images, &rig.views);
        rig.views = (0..6)
            .map(|i| {
                let v = CaptureView { rel: crate::capture::face_rotation(i), fov: Some(std::f32::consts::FRAC_PI_2), post: Post::HDR };
                crate::capture::spawn_view(&mut commands, &mut images, UVec2::splat(res), v, -40 - i as isize, false)
            })
            .collect();
        rig.res = res;
        let faces: [Handle<Image>; 6] = std::array::from_fn(|i| rig.views[i].1.clone());
        commands.entity(cam).insert(ProjFaces(faces));
    }
    let size = windows.iter().next().map(|w| Vec2::new(w.physical_width() as f32, w.physical_height() as f32)).unwrap_or(Vec2::new(1920.0, 1080.0));
    let u = settings.uniform(size);
    if uniform != Some(&u) {
        commands.entity(cam).insert(u);
    }
}

pub struct ProjectionPlugin;

impl Plugin for ProjectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<ProjUniform>::default(),
            UniformComponentPlugin::<ProjUniform>::default(),
            ExtractComponentPlugin::<ProjFaces>::default(),
        ))
        .init_resource::<ProjectionSettings>()
        .init_resource::<Rig>()
        .add_systems(Startup, load_settings)
        .add_systems(Update, apply_projection);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        // The projected scene goes into the main texture before every pass that reads it (the black
        // hole lens and warp passes, bloom, exposure, tone mapping), as the main pass would.
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Core3d, projection_pass.before(crate::blackholes::LensPassSet).in_set(Core3dSystems::EarlyPostProcess));
    }
}

#[derive(Resource)]
struct ProjPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    pipeline: CachedRenderPipelineId,
    cells: UniformBuffer<[UVec4; 4]>,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let tex = || texture_2d(TextureSampleType::Float { filterable: true });
    let layout = BindGroupLayoutDescriptor::new(
        "projection_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<ProjUniform>(true),
                tex(),
                tex(),
                tex(),
                tex(),
                tex(),
                tex(),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<[UVec4; 4]>(false),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let shader: Handle<Shader> = asset_server.load("shaders/projection.wesl");
    let pipeline = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("projection".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader,
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState { format: crate::capture::FORMAT, blend: None, write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(ProjPipeline { layout, sampler, pipeline, cells: UniformBuffer::default() });
}

/// The mosaic cells packed for the shader: per face (column, row) in xy / zw pairs, then the grid.
fn packed_cells(format: u8) -> [UVec4; 4] {
    let (grid, cells) = mosaic_layout(format);
    [
        UVec4::new(cells[0].x, cells[0].y, cells[1].x, cells[1].y),
        UVec4::new(cells[2].x, cells[2].y, cells[3].x, cells[3].y),
        UVec4::new(cells[4].x, cells[4].y, cells[5].x, cells[5].y),
        UVec4::new(grid.x, grid.y, 0, 0),
    ]
}

#[allow(clippy::too_many_arguments)]
fn projection_pass(
    view: ViewQuery<(&ViewTarget, &ProjFaces, &ProjUniform, &DynamicUniformIndex<ProjUniform>)>,
    mut pipeline: ResMut<ProjPipeline>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<ProjUniform>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    queue: Res<bevy::render::renderer::RenderQueue>,
    mut ctx: RenderContext,
) {
    let (target, faces, u, index) = view.into_inner();
    let (Some(render_pipeline), Some(binding)) = (pipeline_cache.get_render_pipeline(pipeline.pipeline), uniforms.binding()) else {
        return;
    };
    if target.main_texture_format() != crate::capture::FORMAT {
        return;
    }
    let views: Vec<&GpuImage> = faces.0.iter().filter_map(|h| gpu_images.get(h)).collect();
    if views.len() != 6 {
        return;
    }
    let device = ctx.render_device().clone();
    let cells = packed_cells(u.mode.z as u8);
    if *pipeline.cells.get() != cells || pipeline.cells.buffer().is_none() {
        pipeline.cells.set(cells);
        pipeline.cells.write_buffer(&device, &queue);
    }
    let Some(cells_binding) = pipeline.cells.binding() else { return };
    let layout = pipeline_cache.get_bind_group_layout(&pipeline.layout);
    let group = device.create_bind_group(
        "projection_group",
        &layout,
        &BindGroupEntries::sequential((
            binding.clone(),
            &views[0].texture_view,
            &views[1].texture_view,
            &views[2].texture_view,
            &views[3].texture_view,
            &views[4].texture_view,
            &views[5].texture_view,
            &pipeline.sampler,
            cells_binding,
        )),
    );
    let encoder = ctx.command_encoder();
    // Every pixel is written: the main pass drew nothing (the main camera's layer is empty).
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("projection"),
        color_attachments: &[Some(target.get_unsampled_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &group, &[index.index()]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fisheye_matches_se_shader() {
        let s = ProjectionSettings { fisheye_zoom: 1.0, ..default() };
        // The centre looks ahead; the circle's edge (r = 1) is FisheyeFOV / 2 = 90 degrees off.
        assert!((fisheye_ray(&s, Vec2::ZERO, 1.0).unwrap() - Vec3::NEG_Z).length() < 1e-6);
        let edge = fisheye_ray(&s, Vec2::new(0.0, 0.999), 1.0).unwrap();
        assert!(edge.z.abs() < 0.01 && edge.y > 0.99, "{edge}");
        let right = fisheye_ray(&s, Vec2::new(0.5, 0.0), 1.0).unwrap();
        assert!(right.x > 0.0 && (right.normalize().dot(Vec3::NEG_Z).acos().to_degrees() - 45.0).abs() < 1e-3);
        // Cut off at the crop circle; the aspect stretches x.
        assert!(fisheye_ray(&s, Vec2::new(0.0, 1.0), 1.0).is_none());
        assert!(fisheye_ray(&s, Vec2::new(0.6, 0.0), 16.0 / 9.0).is_none());
        // Flips mirror, zoom shrinks the angle, the offset moves the centre.
        let f = ProjectionSettings { fisheye_flip_x: true, ..s.clone() };
        assert!(fisheye_ray(&f, Vec2::new(0.5, 0.0), 1.0).unwrap().x < 0.0);
        let z = ProjectionSettings { fisheye_zoom: 2.0, ..s.clone() };
        let r = fisheye_ray(&z, Vec2::new(0.5, 0.0), 1.0).unwrap();
        assert!((r.normalize().dot(Vec3::NEG_Z).acos().to_degrees() - 22.5).abs() < 1e-3);
        let o = ProjectionSettings { fisheye_offset: Vec2::new(0.0, 0.3), ..s.clone() };
        assert!((fisheye_ray(&o, Vec2::new(0.0, 0.3), 1.0).unwrap() - Vec3::NEG_Z).length() < 1e-6);
        // Warp x pushes along the axis, as SE's `ray + Warp`.
        let w = ProjectionSettings { fisheye_warp: Vec3::new(1.0, 0.0, 0.0), ..s };
        assert!((fisheye_ray(&w, Vec2::ZERO, 1.0).unwrap() - Vec3::new(0.0, 0.0, -2.0)).length() < 1e-6);
    }

    #[test]
    fn mosaics_place_every_face_once() {
        for f in 0..6u8 {
            let (grid, cells) = mosaic_layout(f);
            for (i, a) in cells.iter().enumerate() {
                assert!(a.x < grid.x && a.y < grid.y);
                assert!(cells[i + 1..].iter().all(|b| b != a), "format {f}");
            }
        }
        // The cross: ahead in the middle, up above it.
        let (_, cross) = mosaic_layout(0);
        assert_eq!((cross[4], cross[2]), (UVec2::new(1, 1), UVec2::new(1, 0)));
    }

    #[test]
    fn reads_and_writes_se_keys() {
        let text = "Projection\n{\n\tProjectionMode 2\n\tFisheyeFOV 200\n\tFisheyeOffset ( 0.1 -0.2 )\n\tFisheyeWarp ( 0 0.5 0 )\n\tFisheyeFlipY true\n\tCubemapFormat 9\n\tCubeFaceResolution 1024\n}\n";
        let doc = se_core::sc::parse(text);
        let mut s = ProjectionSettings::default();
        s.read(doc.find_deep("Projection").and_then(|i| i.block()).unwrap());
        assert_eq!((s.mode, s.fisheye_fov, s.cubemap_format, s.cube_face_res), (2, 200.0, 5, 1024));
        assert_eq!((s.fisheye_offset, s.fisheye_warp.y, s.fisheye_flip_y), (Vec2::new(0.1, -0.2), 0.5, true));
        let back = se_core::sc::parse(&s.to_cfg());
        let mut t = ProjectionSettings::default();
        t.read(back.find_deep("Projection").and_then(|i| i.block()).unwrap());
        assert_eq!(s, t);
        let u = s.uniform(Vec2::new(1920.0, 1080.0));
        assert_eq!(u.mode, UVec4::new(2, 2 | 4 | 8, 5, 0));
        assert!((u.fish.x - 100f32.to_radians()).abs() < 1e-6);
    }
}
