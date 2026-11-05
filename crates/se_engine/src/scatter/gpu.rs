//! The scatter's render-world half: the pooled GPU buffers (made once), the compute passes
//! (`scatter.wesl` `place`, `cull`, `finish`) run before the main pass, and the draws: one non-mesh
//! item in Bevy's alpha-mask phase whose render command issues one indirect draw per
//! (model, LOD, sub-mesh). Drawn in the main pass, the objects land in render_scale's solids corner,
//! with the view's MSAA and depth, before the volumetric clouds read the depth buffer.

use bevy::core_pipeline::core_3d::{AlphaMask3d, CORE_3D_DEPTH_FORMAT, main_opaque_pass_3d};
use bevy::core_pipeline::prepass::{OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey};
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::mesh::allocator::MeshSlabs;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem, RenderCommand, RenderCommandResult,
    TrackedRenderPass, ViewBinnedRenderPhases,
};
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, texture_2d_array, texture_3d, texture_cube,
    uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::GpuImage;
use bevy::render::view::ExtractedView;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::library::{DrawKind, ScatterLibrary};
use super::{MAX_JOBS, PAGE_VERTS, PAGES, SLOT_CAP, SLOTS, ScatterFrame, TREE_MAX};
use crate::color_correction::ColorCorrection;

/// The view the scatter draws in: the main camera (as auto exposure picks it).
type MainView = (With<ColorCorrection>, Without<crate::capture::CaptureView>);

/// A main-world entity whose id names the scatter's phase item (bins are keyed by main entity).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
struct ScatterItem(Entity);

pub fn build(app: &mut App) {
    let item = app.world_mut().spawn_empty().id();
    app.insert_resource(ScatterItem(item)).add_plugins(ExtractResourcePlugin::<ScatterItem>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<SpecializedRenderPipelines<DrawPipeline>>()
        .add_render_command::<AlphaMask3d, DrawScatter>()
        .add_systems(RenderStartup, init_gpu)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
        .add_systems(Render, prepare_bind_groups.in_set(RenderSystems::PrepareBindGroups))
        .add_systems(Render, queue.in_set(RenderSystems::Queue))
        .add_systems(Render, debug_readback.in_set(RenderSystems::Cleanup))
        .add_systems(Core3d, scatter_compute.in_set(Core3dSystems::Prepass).before(main_opaque_pass_3d));
}

/// The body and LOD values the passes read (`scatter.wesl` / `scatter_draw.wesl` `Body`).
#[derive(Clone, Copy, Default, ShaderType)]
struct BodyGpu {
    m0: Vec4,
    m1: Vec4,
    m2: Vec4,
    /// xyz the body's centre relative to the camera, w its radius (m).
    centre: Vec4,
    /// mesh_far, band, impostor_far, thin_start.
    lod: Vec4,
    /// thin_end, sink, time, the lowest ground height trees stand on (m; the sea's shore).
    lod2: Vec4,
    /// slots in use, models, draws, buckets.
    counts: UVec4,
    sun: Vec4,
    sun_color: Vec4,
    /// x ambient.
    levels: Vec4,
    /// As the ground's: x atmosphere on, y sun scale, z scatter scale, w DayAmbient.
    atm_flags: Vec4,
    /// x how plants are decided (`VegKind`), y the noises' footprint on the unit sphere.
    veg: Vec4,
    /// Towards the star in the body frame.
    sun_body: Vec4,
    /// SE's climate parameters (`BiomeUniform::climate` 0-7) and the biomes' flags.
    climate: [Vec4; 8],
    biome_flags: Vec4,
    /// `VegKind::Map`: the plants' patch frame (centre and tan of its half-angle, its two axes).
    patch_c: Vec4,
    patch_e1: Vec4,
    patch_e2: Vec4,
    /// The wind (render frame, m/s).
    wind: Vec4,
    /// SE's forest colour (linear) and the trees' tint gain.
    forest: Vec4,
    tint_gain: Vec4,
    /// rgb SE's grass colour (linear), w the grass's range (m).
    grass: Vec4,
    /// Earth's species' first model, the flora's, 1 when the flora is this body's.
    species: UVec4,
    /// x the ground's relief (m), y the far buildings' range (m).
    flora_k: Vec4,
    /// Cities: the body's civilization (`BodyFrame::city`, `city2`).
    city: Vec4,
    city2: Vec4,
    /// The ground's cloud key (radius m, seed fraction) and x, y of `proc_params2`; z 1 when the body
    /// has atmosphere tables bound.
    key: Vec4,
    atm: crate::atmosphere::AtmosphereParamsUniform,
    light: crate::terrain::PatchLight,
}

/// The view (`Camera` in the shaders).
#[derive(Clone, Copy, Default, ShaderType)]
struct CameraGpu {
    clip_from_world: Mat4,
    /// Left, right, bottom, top and near planes (xyz inward normal, w offset).
    planes: [Vec4; 5],
    /// The main pass's size in pixels (xy) and its inverse (zw).
    viewport: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct MaterialGpu {
    tint: Vec4,
    /// x 1 for leaves (alpha-tested, two-sided).
    flags: Vec4,
}

/// The library on the GPU (made when it arrives; fixed sizes after).
struct LibGpu {
    generation: u32,
    vertices: Buffer,
    indices: Buffer,
    models: Buffer,
    buckets: Buffer,
    draws: Buffer,
    counters: Buffer,
    args: Buffer,
    out: Buffer,
    /// Per drawn instance (as `out`), the meshes' light (f16), and the largest mesh bucket.
    lights: Buffer,
    max_mesh_cap: u32,
    n_buckets: u32,
    n_draws: u32,
    n_models: u32,
    draw_kinds: Vec<DrawKind>,
    /// Per material: its uniform buffer (tint) and bind group once its texture is on the GPU.
    material_params: Vec<Buffer>,
    material_groups: Vec<Option<BindGroup>>,
    impostor_group: Option<BindGroup>,
}

#[derive(Resource)]
struct ScatterGpu {
    /// `SE_SCATTER_DEBUG=1`: the bucket counters are read back every 120 frames and logged.
    debug: Option<DebugReadback>,
    instances: Buffer,
    /// Per kept candidate, the plants' noises at its spot (f16 x4), worked out by the placement.
    noises: Buffer,
    counts: Buffer,
    cells: Buffer,
    tree: Buffer,
    page_pos: Buffer,
    page_info: Buffer,
    jobs: Buffer,
    body: UniformBuffer<BodyGpu>,
    camera: UniformBuffer<CameraGpu>,
    lib: Option<LibGpu>,
    compute_layout: BindGroupLayoutDescriptor,
    place: CachedComputePipelineId,
    cull: CachedComputePipelineId,
    finish: CachedComputePipelineId,
    compute_group: Option<BindGroup>,
    draw_group: Option<BindGroup>,
    light_group: Option<BindGroup>,
    /// The meshes' light, once per instance (`scatter_draw.wesl` `light`): its pipeline, its own view
    /// group and an empty group 1 (the light group is the draws' group 2).
    light: CachedComputePipelineId,
    light_view_layout: BindGroupLayoutDescriptor,
    empty_layout: BindGroupLayoutDescriptor,
    light_view_group: Option<BindGroup>,
    empty_group: Option<BindGroup>,
    active: bool,
    n_jobs: u32,
    slots_used: u32,
    /// Per slot, the job it was last placed with (its cell and seed).
    placed: Vec<Option<[u32; 7]>>,
    /// This frame's draw pipelines: bark, leaves, impostor, building.
    pipelines: [Option<CachedRenderPipelineId>; 4],
}

/// The draw pipelines' layouts and shader.
#[derive(Resource, Clone)]
struct DrawPipeline {
    view_layout: BindGroupLayoutDescriptor,
    /// The ground's light: atmosphere tables and the clouds' shadow (`ground_light.wesl`).
    light_layout: BindGroupLayoutDescriptor,
    material_layout: BindGroupLayoutDescriptor,
    /// The buildings' material: MR's window array.
    building_layout: BindGroupLayoutDescriptor,
    impostor_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
struct DrawKey {
    samples: u32,
    format: TextureFormat,
    /// 0 bark, 1 leaves, 2 impostor, 3 building.
    kind: u8,
}

impl SpecializedRenderPipeline for DrawPipeline {
    type Key = DrawKey;

    fn specialize(&self, key: DrawKey) -> RenderPipelineDescriptor {
        let impostor = key.kind == 2;
        let (vs, fs) = match key.kind {
            2 => ("vertex_impostor", "fragment_impostor"),
            3 => ("vertex_building", "fragment_building"),
            _ => ("vertex_mesh", "fragment_mesh"),
        };
        let mut defs: Vec<bevy::shader::ShaderDefVal> = Vec::new();
        if key.kind == 1 {
            defs.push("LEAVES".into());
        }
        if key.kind == 3 {
            defs.push("BUILDING".into());
        }
        if impostor {
            defs.push("IMPOSTOR".into());
        }
        let buffers = if impostor {
            Vec::new()
        } else {
            vec![VertexBufferLayout::from_vertex_formats(VertexStepMode::Vertex, [VertexFormat::Float32x3, VertexFormat::Float32x3, VertexFormat::Float32x2])]
        };
        RenderPipelineDescriptor {
            label: Some(format!("scatter_draw_{}", ["bark", "leaves", "impostor", "building"][key.kind as usize]).into()),
            layout: vec![
                self.view_layout.clone(),
                match key.kind {
                    2 => self.impostor_layout.clone(),
                    3 => self.building_layout.clone(),
                    _ => self.material_layout.clone(),
                },
                self.light_layout.clone(),
            ],
            vertex: VertexState { shader: self.shader.clone(), shader_defs: defs.clone(), entry_point: Some(vs.into()), buffers, ..default() },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: defs,
                entry_point: Some(fs.into()),
                targets: vec![Some(ColorTargetState { format: key.format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            primitive: PrimitiveState { cull_mode: if key.kind == 0 || key.kind == 3 { Some(Face::Back) } else { None }, ..default() },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            multisample: MultisampleState { count: key.samples, mask: !0, alpha_to_coverage_enabled: false },
            ..default()
        }
    }
}

fn buffer(device: &RenderDevice, label: &'static str, size: u64, usage: BufferUsages) -> Buffer {
    device.create_buffer(&BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false })
}

fn init_gpu(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    cache: Res<PipelineCache>,
    mut specialized: ResMut<SpecializedRenderPipelines<DrawPipeline>>,
) {
    let storage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
    let rw = |n: u64| storage_buffer_sized(false, std::num::NonZero::new(n));
    let ro = |n: u64| storage_buffer_read_only_sized(false, std::num::NonZero::new(n));
    let compute_layout = BindGroupLayoutDescriptor::new(
        "scatter_compute",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<BodyGpu>(false),
                uniform_buffer::<CameraGpu>(false),
                ro(std::mem::size_of::<super::PlaceJob>() as u64),  // jobs
                rw(4),   // instances
                rw(4),   // counts
                ro(32),  // cells
                ro(4),   // tree
                ro(16),  // page positions
                ro(48),  // page info
                ro(48),  // models
                ro(16),  // buckets
                ro(16),  // draws
                rw(4),   // counters
                rw(4),   // args
                rw(32),  // out
                rw(8),   // noises
                texture_2d(TextureSampleType::Float { filterable: true }), // the plants' map patch
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }), // its lights (Earth's towns)
            ),
        ),
    );
    let shader: Handle<Shader> = assets.load("shaders/scatter.wesl");
    let queue = |entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("scatter_{entry}").into()),
            layout: vec![compute_layout.clone()],
            shader: shader.clone(),
            entry_point: Some(entry.into()),
            ..default()
        })
    };
    let (place, cull, finish) = (queue("place"), queue("cull"), queue("finish"));
    let view_layout = BindGroupLayoutDescriptor::new(
        "scatter_view",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (uniform_buffer::<CameraGpu>(false), uniform_buffer::<BodyGpu>(false), ro(32), ro(48), ro(32)),
        ),
    );
    let material_layout = BindGroupLayoutDescriptor::new(
        "scatter_material",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering), uniform_buffer::<MaterialGpu>(false)),
        ),
    );
    let building_layout = BindGroupLayoutDescriptor::new(
        "scatter_building",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_2d_array(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering), uniform_buffer::<MaterialGpu>(false)),
        ),
    );
    let impostor_layout = BindGroupLayoutDescriptor::new(
        "scatter_impostor",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d_array(TextureSampleType::Float { filterable: true }),
                texture_2d_array(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let filt = || TextureSampleType::Float { filterable: true };
    let light_layout = BindGroupLayoutDescriptor::new(
        "scatter_light",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT | ShaderStages::COMPUTE,
            (
                texture_3d(filt()),
                texture_2d(filt()),
                texture_2d(filt()),
                sampler(SamplerBindingType::Filtering),
                texture_cube(filt()),
                sampler(SamplerBindingType::Filtering),
                storage_buffer_read_only_sized(false, None),
                texture_2d(filt()),
                sampler(SamplerBindingType::Filtering),
                texture_cube(filt()),
                texture_cube(filt()),
            ),
        ),
    );
    let draw = DrawPipeline { view_layout, light_layout, material_layout, building_layout, impostor_layout, shader: assets.load("shaders/scatter_draw.wesl") };
    // The meshes' light pass: the body, the drawn instances and models, the light records (written),
    // the bucket counters and the buckets; group 1 empty, group 2 the draws' light.
    let light_view_layout = BindGroupLayoutDescriptor::new(
        "scatter_light_view",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            ((1, uniform_buffer::<BodyGpu>(false)), (2, ro(32)), (3, ro(48)), (5, rw(32)), (6, ro(4)), (7, ro(16))),
        ),
    );
    let empty_layout = BindGroupLayoutDescriptor::new("scatter_empty", &[]);
    let light = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("scatter_light".into()),
        layout: vec![light_view_layout.clone(), empty_layout.clone(), draw.light_layout.clone()],
        shader: draw.shader.clone(),
        entry_point: Some("light".into()),
        ..default()
    });
    // Built at start (the loading screen waits for them): no MSAA and 4x, the HDR target.
    for samples in [1, 4] {
        for kind in 0..3 {
            specialized.specialize(&cache, &draw, DrawKey { samples, format: TextureFormat::Rgba16Float, kind });
        }
    }
    commands.insert_resource(draw);
    commands.insert_resource(ScatterGpu {
        instances: buffer(&device, "scatter_instances", SLOTS as u64 * SLOT_CAP as u64 * 4, storage),
        noises: buffer(&device, "scatter_noises", SLOTS as u64 * SLOT_CAP as u64 * 8, storage),
        counts: buffer(&device, "scatter_counts", SLOTS as u64 * 4, storage),
        cells: buffer(&device, "scatter_cells", SLOTS as u64 * 32, storage),
        tree: buffer(&device, "scatter_tree", TREE_MAX as u64 * 4, storage),
        page_pos: buffer(&device, "scatter_page_pos", PAGES as u64 * PAGE_VERTS as u64 * 16, storage),
        page_info: buffer(&device, "scatter_page_info", PAGES as u64 * 48, storage),
        jobs: buffer(&device, "scatter_jobs", MAX_JOBS as u64 * std::mem::size_of::<super::PlaceJob>() as u64, storage),
        body: UniformBuffer::default(),
        camera: UniformBuffer::default(),
        lib: None,
        compute_layout,
        place,
        cull,
        finish,
        compute_group: None,
        draw_group: None,
        light_group: None,
        light,
        light_view_layout,
        empty_layout,
        light_view_group: None,
        empty_group: None,
        active: false,
        n_jobs: 0,
        slots_used: 0,
        placed: vec![None; SLOTS as usize],
        pipelines: [None; 4],
        debug: std::env::var("SE_SCATTER_DEBUG").is_ok_and(|v| v != "0").then(|| DebugReadback {
            staging: buffer(&device, "scatter_debug", 4096, BufferUsages::MAP_READ | BufferUsages::COPY_DST),
            state: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
            frame: 0,
        }),
    });
}

fn bytes<T: bytemuck::Pod>(v: &[T]) -> &[u8] {
    bytemuck::cast_slice(v)
}

/// Uploads the library when it changes (its buffers are made once per library).
fn upload_library(gpu: &mut ScatterGpu, lib: &ScatterLibrary, device: &RenderDevice, queue: &RenderQueue) {
    if gpu.lib.as_ref().is_some_and(|l| l.generation == lib.generation) {
        return;
    }
    let storage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
    let init = |label: &'static str, contents: &[u8], usage: BufferUsages| {
        device.create_buffer_with_data(&BufferInitDescriptor { label: Some(label), contents, usage })
    };
    // Buckets laid out one after the other in the output.
    let mut base = 0u32;
    let mut buckets: Vec<[u32; 4]> = Vec::new();
    for &cap in &lib.bucket_caps {
        buckets.push([base, cap, 0, 0]);
        base += cap;
    }
    let draws: Vec<[u32; 4]> = lib.draws.iter().map(|d| [d.bucket, d.index_count, d.first_index, d.base_vertex as u32]).collect();
    let material_params = lib
        .materials
        .iter()
        .map(|m| {
            let mut ub = UniformBuffer::from(MaterialGpu { tint: Vec4::from_array(m.tint), flags: Vec4::new(if m.leaves { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0) });
            ub.write_buffer(device, queue);
            ub.buffer().unwrap().clone()
        })
        .collect::<Vec<_>>();
    gpu.lib = Some(LibGpu {
        generation: lib.generation,
        vertices: init("scatter_vertices", bytes(&lib.vertices), BufferUsages::VERTEX),
        indices: init("scatter_indices", bytes(&lib.indices), BufferUsages::INDEX),
        models: init("scatter_models", bytes(&lib.models), storage),
        buckets: init("scatter_buckets", bytes(&buckets), storage),
        draws: init("scatter_draws", bytes(&draws), storage),
        counters: buffer(device, "scatter_counters", lib.bucket_caps.len() as u64 * 4, storage | BufferUsages::COPY_SRC),
        args: buffer(device, "scatter_args", lib.draws.len() as u64 * 20, storage | BufferUsages::INDIRECT),
        out: buffer(device, "scatter_out", base as u64 * 32, storage),
        lights: buffer(device, "scatter_lights", base as u64 * 32, storage),
        max_mesh_cap: lib.bucket_caps.iter().step_by(2).copied().max().unwrap_or(0),
        n_buckets: lib.bucket_caps.len() as u32,
        n_draws: lib.draws.len() as u32,
        n_models: lib.models.len() as u32,
        draw_kinds: lib.draws.iter().map(|d| d.kind).collect(),
        material_groups: vec![None; lib.materials.len()],
        material_params,
        impostor_group: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    mut gpu: ResMut<ScatterGpu>,
    frame: Option<Res<ScatterFrame>>,
    lib: Option<Res<ScatterLibrary>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    views: Query<&ExtractedView, MainView>,
) {
    gpu.active = false;
    let (Some(frame), Some(lib)) = (frame, lib) else { return };
    // The height pages first: the main world sends each once, whatever this frame draws.
    for (page, verts) in &frame.page_uploads {
        if *page < PAGES {
            queue.write_buffer(&gpu.page_pos, *page as u64 * PAGE_VERTS as u64 * 16, bytes(verts.as_slice()));
        }
    }
    if gpu.debug.as_ref().is_some_and(|d| d.frame % 120 == 1) {
        info!("scatter debug (render): frame active {}, library ready {}, views {}", frame.active, lib.ready(), views.iter().count());
    }
    if !frame.active || !lib.ready() {
        return;
    }
    upload_library(&mut gpu, &lib, &device, &queue);
    let Some(view) = views.iter().next() else { return };
    // Jobs: the resident slots not placed with their cell yet, nearest first, once the passes
    // are ready (a job is marked done only when it runs). Their slots start empty and are not
    // drawn until placed.
    let ready = [gpu.place, gpu.cull, gpu.finish].iter().all(|&id| cache.get_compute_pipeline(id).is_some());
    let key = |j: &super::PlaceJob| [j.face, j.level, j.row, j.col, j.seed, j.keep.to_bits(), 1];
    let mut jobs = Vec::new();
    let mut pending = vec![false; SLOTS as usize];
    for j in &frame.jobs {
        let s = j.slot as usize;
        if s >= SLOTS as usize || gpu.placed[s] == Some(key(j)) {
            continue;
        }
        if ready && jobs.len() < MAX_JOBS as usize {
            gpu.placed[s] = Some(key(j));
            jobs.push(*j);
        } else {
            pending[s] = true;
        }
    }
    for j in &jobs {
        queue.write_buffer(&gpu.counts, j.slot as u64 * 4, &0u32.to_le_bytes());
    }
    if !jobs.is_empty() {
        queue.write_buffer(&gpu.jobs, 0, bytes(&jobs));
    }
    let n_jobs = jobs.len();
    if !frame.pages.is_empty() {
        queue.write_buffer(&gpu.page_info, 0, bytes(frame.pages.as_slice()));
    }
    if !frame.cells.is_empty() {
        let mut cells = frame.cells.as_ref().clone();
        for (s, c) in cells.iter_mut().enumerate() {
            if pending[s] {
                c.shift_live &= !super::CellGpu::LIVE;
            }
        }
        queue.write_buffer(&gpu.cells, 0, bytes(&cells));
    }
    if !frame.tree.is_empty() {
        let n = frame.tree.len().min(TREE_MAX as usize);
        queue.write_buffer(&gpu.tree, 0, bytes(&frame.tree[..n]));
    }
    let b = &frame.body;
    let lib_gpu = gpu.lib.as_ref().unwrap();
    let counts = UVec4::new(frame.cells.len() as u32, lib_gpu.n_models, lib_gpu.n_draws, lib_gpu.n_buckets);
    let u = &b.uniform;
    let has_ocean = u.ocean_sea.w > 0.5;
    gpu.body.set(BodyGpu {
        m0: b.m.x_axis.extend(0.0),
        m1: b.m.y_axis.extend(0.0),
        m2: b.m.z_axis.extend(0.0),
        centre: b.centre.extend(b.radius_m),
        lod: Vec4::new(frame.lod.mesh_far, frame.lod.band, frame.lod.impostor_far, frame.lod.thin_start),
        lod2: Vec4::new(frame.lod.thin_end, frame.sink, frame.time, if has_ocean { 2.0 } else { -1e9 }),
        counts,
        sun: b.light.sun_dir,
        sun_color: b.light.sun_color,
        levels: b.light.levels,
        atm_flags: Vec4::new(b.light.flags.x, u.atm_flags.y, u.atm_flags.z, u.atm_flags.w),
        // The plants' noises at about 4 m (their spacing).
        veg: Vec4::new(b.veg as u32 as f32, 4.0 / b.radius_m.max(1.0), b.grass_models.x as f32, b.grass_models.y as f32),
        sun_body: b.sun_body.extend(0.0),
        climate: b.climate,
        biome_flags: b.biome_flags,
        patch_c: b.patch_frame[0],
        patch_e1: b.patch_frame[1],
        patch_e2: b.patch_frame[2],
        wind: b.wind.extend(0.0),
        forest: b.forest,
        tint_gain: b.tint_gain.extend(0.0),
        grass: b.grass,
        species: b.species,
        flora_k: Vec4::new(b.relief_m, super::FAR_RANGE_M as f32, b.civ_light as f32, 0.0),
        city: b.city,
        city2: b.city2,
        key: Vec4::new(u.proc_params2.x, u.proc_params2.y, if b.atmosphere.is_some() { 1.0 } else { 0.0 }, 0.0),
        atm: u.atm,
        light: b.light,
    });
    gpu.body.write_buffer(&device, &queue);
    // The view: the camera sits at the origin.
    let clip_from_world = view.clip_from_world.unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
    let r = clip_from_world.transpose();
    let (r0, r1, r3) = (r.x_axis, r.y_axis, r.w_axis);
    let plane = |p: Vec4| p / p.truncate().length().max(1e-20);
    let planes = [plane(r3 + r0), plane(r3 - r0), plane(r3 + r1), plane(r3 - r1), plane(r3 - r.z_axis)];
    let (w, h) = (view.viewport.z.max(1) as f32, view.viewport.w.max(1) as f32);
    gpu.camera.set(CameraGpu { clip_from_world, planes, viewport: Vec4::new(w, h, 1.0 / w, 1.0 / h) });
    gpu.camera.write_buffer(&device, &queue);
    gpu.n_jobs = n_jobs as u32;
    gpu.slots_used = frame.cells.len() as u32;
    gpu.active = true;
}

#[allow(clippy::too_many_arguments)]
fn prepare_bind_groups(
    mut gpu: ResMut<ScatterGpu>,
    lib: Option<Res<ScatterLibrary>>,
    frame: Option<Res<ScatterFrame>>,
    shadow: Option<Res<crate::volumetric_clouds::CloudShadowAssets>>,
    buffers: Res<RenderAssets<bevy::render::storage::GpuShaderBuffer>>,
    fallback: Res<bevy::render::texture::FallbackImage>,
    zero: Res<bevy::render::texture::FallbackImageZero>,
    draw: Res<DrawPipeline>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    images: Res<RenderAssets<GpuImage>>,
) {
    gpu.compute_group = None;
    gpu.draw_group = None;
    gpu.light_group = None;
    gpu.light_view_group = None;
    gpu.empty_group = None;
    if !gpu.active {
        return;
    }
    let gpu = &mut *gpu;
    let (Some(body), Some(camera)) = (gpu.body.binding(), gpu.camera.binding()) else { return };
    let Some(l) = gpu.lib.as_mut() else { return };
    let compute_layout = cache.get_bind_group_layout(&gpu.compute_layout);
    // The plants' map patch (Bevy's black fallback until it is on the GPU: no plants from it).
    let patch = frame.as_deref().and_then(|f| f.body.patch.as_ref()).and_then(|p| images.get(p)).unwrap_or(&**zero);
    let patch_lights = frame.as_deref().and_then(|f| f.body.patch_lights.as_ref()).and_then(|p| images.get(p)).unwrap_or(&**zero);
    gpu.compute_group = Some(device.create_bind_group(
        "scatter_compute",
        &compute_layout,
        &BindGroupEntries::sequential((
            body.clone(),
            camera.clone(),
            gpu.jobs.as_entire_binding(),
            gpu.instances.as_entire_binding(),
            gpu.counts.as_entire_binding(),
            gpu.cells.as_entire_binding(),
            gpu.tree.as_entire_binding(),
            gpu.page_pos.as_entire_binding(),
            gpu.page_info.as_entire_binding(),
            l.models.as_entire_binding(),
            l.buckets.as_entire_binding(),
            l.draws.as_entire_binding(),
            l.counters.as_entire_binding(),
            l.args.as_entire_binding(),
            l.out.as_entire_binding(),
            gpu.noises.as_entire_binding(),
            &patch.texture_view,
            &patch.sampler,
            &patch_lights.texture_view,
        )),
    ));
    let view_layout = cache.get_bind_group_layout(&draw.view_layout);
    gpu.draw_group = Some(device.create_bind_group(
        "scatter_view",
        &view_layout,
        &BindGroupEntries::sequential((camera, body.clone(), l.out.as_entire_binding(), l.models.as_entire_binding(), l.lights.as_entire_binding())),
    ));
    let light_view_layout = cache.get_bind_group_layout(&gpu.light_view_layout);
    gpu.light_view_group = Some(device.create_bind_group(
        "scatter_light_view",
        &light_view_layout,
        &BindGroupEntries::with_indices((
            (1, body),
            (2, l.out.as_entire_binding()),
            (3, l.models.as_entire_binding()),
            (5, l.lights.as_entire_binding()),
            (6, l.counters.as_entire_binding()),
            (7, l.buckets.as_entire_binding()),
        )),
    ));
    let empty_layout = cache.get_bind_group_layout(&gpu.empty_layout);
    gpu.empty_group = Some(device.create_bind_group("scatter_empty", &empty_layout, &[]));
    // The ground's light: the body's atmosphere tables (Bevy's fallbacks without) and the clouds'
    // shadow (the record says whether it is this body's).
    if let (Some(f), Some(sh)) = (frame.as_deref(), shadow.as_deref()) {
        let atm = f.body.atmosphere.as_ref().and_then(|(s, t, e)| Some((images.get(s)?, images.get(t)?, images.get(e)?)));
        let (s3, t2, e2) = atm.map_or((&fallback.d3, &fallback.d2, &fallback.d2), |a| a);
        let get = |h: &Handle<Image>| images.get(h);
        if let (Some(cover), Some(pattern), Some(map), Some(weather), Some(rec)) =
            (get(&sh.cover), get(&sh.pattern), get(&sh.map), get(&sh.weather), buffers.get(&sh.shadow))
        {
            let layout = cache.get_bind_group_layout(&draw.light_layout);
            gpu.light_group = Some(device.create_bind_group(
                "scatter_light",
                &layout,
                &BindGroupEntries::sequential((
                    &s3.texture_view,
                    &t2.texture_view,
                    &e2.texture_view,
                    &s3.sampler,
                    &cover.texture_view,
                    &cover.sampler,
                    rec.buffer.as_entire_binding(),
                    &map.texture_view,
                    &map.sampler,
                    &weather.texture_view,
                    &pattern.texture_view,
                )),
            ));
        }
    }
    // Materials: made once their textures are on the GPU.
    let Some(lib) = lib else { return };
    let material_layout = cache.get_bind_group_layout(&draw.material_layout);
    let building_layout = cache.get_bind_group_layout(&draw.building_layout);
    for (i, m) in lib.materials.iter().enumerate() {
        if l.material_groups[i].is_some() {
            continue;
        }
        let Some(img) = images.get(&m.color) else { continue };
        l.material_groups[i] = Some(device.create_bind_group(
            "scatter_material",
            if m.building { &building_layout } else { &material_layout },
            &BindGroupEntries::sequential((&img.texture_view, &img.sampler, l.material_params[i].as_entire_binding())),
        ));
    }
    if l.impostor_group.is_none() {
        let get = |h: &Option<Handle<Image>>| h.as_ref().and_then(|h| images.get(h));
        if let (Some(c), Some(n)) = (get(&lib.impostor_color), get(&lib.impostor_normal)) {
            let layout = cache.get_bind_group_layout(&draw.impostor_layout);
            l.impostor_group =
                Some(device.create_bind_group("scatter_impostor", &layout, &BindGroupEntries::sequential((&c.texture_view, &n.texture_view, &c.sampler))));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn queue(
    item: Option<Res<ScatterItem>>,
    mut gpu: ResMut<ScatterGpu>,
    draw: Res<DrawPipeline>,
    cache: Res<PipelineCache>,
    mut specialized: ResMut<SpecializedRenderPipelines<DrawPipeline>>,
    fns: Res<DrawFunctions<AlphaMask3d>>,
    mut phases: ResMut<ViewBinnedRenderPhases<AlphaMask3d>>,
    views: Query<(&ExtractedView, &Msaa), MainView>,
) {
    let Some(item) = item else { return };
    let main = MainEntity::from(item.0);
    let draw_fn = fns.read().id::<DrawScatter>();
    for (view, msaa) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else { continue };
        phase.remove(main);
        if !gpu.active {
            continue;
        }
        let key = |kind| DrawKey { samples: msaa.samples(), format: view.target_format, kind };
        let ids = [0u8, 1, 2, 3].map(|k| specialized.specialize(&cache, &draw, key(k)));
        gpu.pipelines = ids.map(Some);
        phase.add(
            OpaqueNoLightmap3dBatchSetKey { pipeline: ids[0], draw_function: draw_fn, material_bind_group_index: None, slabs: MeshSlabs::default() },
            OpaqueNoLightmap3dBinKey { asset_id: AssetId::<Mesh>::default().untyped() },
            (Entity::PLACEHOLDER, main),
            InputUniformIndex::default(),
            BinnedRenderPhaseType::NonMesh,
        );
    }
}

/// Places the new cells' candidates, culls every resident instance into the buckets and writes the
/// indirect draws.
fn scatter_compute(_view: ViewQuery<&ExtractedView, MainView>, gpu: Res<ScatterGpu>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    if !gpu.active {
        return;
    }
    let (Some(group), Some(l)) = (gpu.compute_group.as_ref(), gpu.lib.as_ref()) else { return };
    let (Some(place), Some(cull), Some(finish)) =
        (cache.get_compute_pipeline(gpu.place), cache.get_compute_pipeline(gpu.cull), cache.get_compute_pipeline(gpu.finish))
    else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "scatter");
    ctx.command_encoder().clear_buffer(&l.counters, 0, None);
    {
        let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("scatter"), ..default() });
        pass.set_bind_group(0, group, &[]);
        if gpu.n_jobs > 0 {
            pass.set_pipeline(place);
            pass.dispatch_workgroups(SLOT_CAP / 64, gpu.n_jobs, 1);
        }
        if gpu.slots_used > 0 {
            pass.set_pipeline(cull);
            pass.dispatch_workgroups(SLOT_CAP / 64, gpu.slots_used, 1);
        }
        pass.set_pipeline(finish);
        pass.dispatch_workgroups(l.n_draws.div_ceil(64), 1, 1);
        // The meshes' light, once per instance (the mesh buckets are the even ones).
        if let (Some(light), Some(g0), Some(g1), Some(g2)) =
            (cache.get_compute_pipeline(gpu.light), gpu.light_view_group.as_ref(), gpu.empty_group.as_ref(), gpu.light_group.as_ref())
        {
            pass.set_pipeline(light);
            pass.set_bind_group(0, g0, &[]);
            pass.set_bind_group(1, g1, &[]);
            pass.set_bind_group(2, g2, &[]);
            pass.dispatch_workgroups(l.max_mesh_cap.div_ceil(64), l.n_buckets.div_ceil(2), 1);
        }
    }
    span.end(ctx.command_encoder());
    if let Some(d) = &gpu.debug {
        use std::sync::atomic::Ordering;
        if d.frame % 120 == 0 && d.state.load(Ordering::Acquire) == 0 {
            let n = (l.n_buckets as u64 * 4).min(4096);
            ctx.command_encoder().copy_buffer_to_buffer(&l.counters, 0, &d.staging, 0, Some(n));
            d.state.store(1, Ordering::Release);
        }
    }
}

/// The bucket counters' read-back for `SE_SCATTER_DEBUG` (state 0 idle, 1 copied, 2 mapping, 3 mapped).
struct DebugReadback {
    staging: Buffer,
    state: std::sync::Arc<std::sync::atomic::AtomicU32>,
    frame: u64,
}

/// Maps the copied counters after the frame's submission and logs them once mapped.
fn debug_readback(mut gpu: ResMut<ScatterGpu>, frame: Option<Res<ScatterFrame>>, device: Res<RenderDevice>) {
    use std::sync::atomic::Ordering;
    let n_buckets = gpu.lib.as_ref().map_or(0, |l| l.n_buckets);
    let Some(d) = gpu.debug.as_mut() else { return };
    d.frame += 1;
    match d.state.load(Ordering::Acquire) {
        2 => {
            if let Err(e) = device.poll(bevy::render::render_resource::PollType::Poll) {
                warn!("scatter debug: poll: {e}");
            }
        }
        1 => {
            d.state.store(2, Ordering::Release);
            let state = d.state.clone();
            d.staging.slice(..).map_async(MapMode::Read, move |r| {
                if let Err(e) = &r {
                    warn!("scatter debug: read-back failed: {e}");
                }
                state.store(if r.is_ok() { 3 } else { 0 }, Ordering::Release)
            });
        }
        3 => {
            let counts: Vec<u32> = {
                match d.staging.slice(..).get_mapped_range() {
                    Ok(view) => bytemuck::cast_slice::<u8, u32>(&view)[..n_buckets as usize].to_vec(),
                    Err(_) => Vec::new(),
                }
            };
            d.staging.unmap();
            d.state.store(0, Ordering::Release);
            let f = frame.as_deref();
            info!(
                "scatter debug: buckets {:?}; cells {}, pages {}, tree {}, jobs {}",
                counts,
                f.map_or(0, |f| f.cells.iter().filter(|c| c.shift_live & super::CellGpu::LIVE != 0).count()),
                f.map_or(0, |f| f.pages.iter().filter(|p| p.t != [0.0; 3]).count()),
                f.map_or(0, |f| f.tree.len()),
                f.map_or(0, |f| f.jobs.len())
            );
        }
        _ => {}
    }
}

/// Draws every (model, LOD, sub-mesh) with its indirect arguments.
struct DrawScatter;

impl<P: PhaseItem> RenderCommand<P> for DrawScatter {
    type Param = (SRes<ScatterGpu>, SRes<PipelineCache>);
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _: &P,
        _: ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (gpu, cache): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (gpu, cache) = (gpu.into_inner(), cache.into_inner());
        if !gpu.active {
            return RenderCommandResult::Success;
        }
        let (Some(view), Some(light), Some(l)) = (gpu.draw_group.as_ref(), gpu.light_group.as_ref(), gpu.lib.as_ref()) else {
            return RenderCommandResult::Success;
        };
        let pipes = gpu.pipelines.map(|p| p.and_then(|id| cache.get_render_pipeline(id)));
        pass.set_bind_group(0, view, &[]);
        pass.set_bind_group(2, light, &[]);
        pass.set_vertex_buffer(0, l.vertices.slice(..));
        pass.set_index_buffer(l.indices.slice(..), IndexFormat::Uint32);
        for (i, kind) in l.draw_kinds.iter().enumerate() {
            let (pipe, group) = match *kind {
                DrawKind::Mesh { material, leaves } => (pipes[leaves as usize], l.material_groups[material as usize].as_ref()),
                DrawKind::Impostor => (pipes[2], l.impostor_group.as_ref()),
                DrawKind::Building { material } => (pipes[3], l.material_groups[material as usize].as_ref()),
            };
            let (Some(pipe), Some(group)) = (pipe, group) else { continue };
            pass.set_render_pipeline(pipe);
            pass.set_bind_group(1, group, &[]);
            pass.draw_indexed_indirect(&l.args, i as u64 * 20);
        }
        RenderCommandResult::Success
    }
}
