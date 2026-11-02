//! SE's procedural terrain heights on the GPU, as SE makes them: `tg_height.wesl` (its
//! `tg_terra_height` / `tg_selena_height` shaders ported) fills one `GEN_N` x `GEN_N` grid per
//! terrain node, which is read back for the node's mesh, normal map and ground height. Jobs go
//! main world -> render world (compute pass) -> back through a one-shot readback.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{storage_buffer, texture_3d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderGraph, RenderQueue};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderCacheError;

/// Samples a side of a node's grid: the node's 129 normal-map texels and 8 more on each side (the
/// mesh's skirt ring and the parent's vertices just outside the node).
pub const GEN_N: u32 = 145;
/// Grid samples between the node's edge and the grid's.
pub const GEN_BORDER: f64 = 8.0;
/// Grid steps across the node.
pub const GEN_SPAN: f64 = 128.0;
/// Most dispatches per frame (SE's `MaxTilesPerFrame` is 4 at 256^2).
const MAX_DISPATCH_PER_FRAME: usize = 8;

/// Uniforms of `tg_height.wesl`: the 17 of SE's generator vec4s the height code reads
/// ([`TgParams::new`]) and the grid.
#[derive(Clone, Copy, Default, ShaderType)]
pub struct TgParams {
    pub se: [Vec4; 17],
    /// xyz: unit-cube point of sample (0, 0); w: `GEN_N`.
    pub grid_origin: Vec4,
    /// Step per sample along i.
    pub grid_du: Vec4,
    /// Step per sample along j; w: 0 `tg_terra`, 1 `tg_selena`, 2 `tg_asteroid`.
    pub grid_dv: Vec4,
}

impl TgParams {
    /// From SE's 20 vec4s (`se_core::tg::uniforms`), leaving out plantsParams1/2 and lavaParams.
    pub fn new(se: &[[f32; 4]; 20], grid_origin: Vec4, grid_du: Vec4, grid_dv: Vec4) -> Self {
        const KEEP: [usize; 17] = [0, 1, 2, 3, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 19];
        Self { se: KEEP.map(|i| Vec4::from_array(se[i])), grid_origin, grid_du, grid_dv }
    }
}

#[derive(Clone)]
struct GenJob {
    id: u64,
    params: TgParams,
    buffer: Handle<ShaderBuffer>,
}

/// Jobs submitted in the main world; the render world takes those with ids past the last it saw.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct GenQueue {
    jobs: Vec<GenJob>,
    dispatched: Arc<Mutex<Vec<(u64, Handle<ShaderBuffer>)>>>,
    /// Set by the render world when the generator's pipeline fails to build (shader error).
    failed: Arc<AtomicBool>,
}

impl GenQueue {
    /// The generator cannot run: its compute pipeline failed. Slow answers (the window minimized,
    /// the shader still compiling) are not failures; the jobs simply wait.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

/// The main world's handle on the generator.
#[derive(Resource, Clone, Default)]
pub struct TerrainGen {
    next_id: u64,
    results: Arc<Mutex<GenResults>>,
}

/// Grids read back and not yet taken, and the jobs nobody will take (their terrain is gone).
#[derive(Default)]
struct GenResults {
    done: HashMap<u64, Arc<Vec<f32>>>,
    forgotten: HashSet<u64>,
}

impl TerrainGen {
    /// Take a finished grid (`GEN_N`^2 heights in SE's units, row j major).
    pub fn take(&self, id: u64) -> Option<Arc<Vec<f32>>> {
        self.results.lock().ok()?.done.remove(&id)
    }

    /// Jobs whose grids will not be taken (the terrain that asked was removed, or gave up on the
    /// generator): their grids go now, or as they arrive.
    pub fn forget(&self, ids: impl IntoIterator<Item = u64>) {
        let Ok(mut r) = self.results.lock() else { return };
        for id in ids {
            if r.done.remove(&id).is_none() {
                r.forgotten.insert(id);
            }
        }
    }
}

impl GenResults {
    /// A grid read back for job `id`.
    fn deliver(&mut self, id: u64, data: Vec<f32>) {
        if !self.forgotten.remove(&id) {
            self.done.insert(id, Arc::new(data));
        }
    }
}

/// Queue a grid; returns its job id for [`TerrainGen::take`].
pub fn request(tgen: &mut TerrainGen, queue: &mut GenQueue, buffers: &mut Assets<ShaderBuffer>, params: TgParams) -> u64 {
    tgen.next_id += 1;
    let mut buf = ShaderBuffer::from(vec![0f32; (GEN_N * GEN_N) as usize]);
    buf.buffer_usage |= BufferUsages::COPY_SRC;
    let buffer = buffers.add(buf);
    queue.jobs.push(GenJob { id: tgen.next_id, params, buffer });
    tgen.next_id
}

pub struct TerrainGenPlugin;

impl Plugin for TerrainGenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerrainGen>()
            .init_resource::<GenQueue>()
            .add_plugins(ExtractResourcePlugin::<GenQueue>::default())
            .add_systems(First, clear_submitted)
            .add_systems(Update, start_readbacks);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<RenderJobs>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_jobs.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(RenderGraph, dispatch);
    }
}

/// Last frame's submissions were extracted at its end.
fn clear_submitted(mut queue: ResMut<GenQueue>) {
    if !queue.jobs.is_empty() {
        queue.jobs.clear();
    }
}

/// Grids computed by the render world: read each back once into the results.
fn start_readbacks(mut commands: Commands, queue: Res<GenQueue>, tgen: Res<TerrainGen>) {
    let done: Vec<_> = queue.dispatched.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default();
    for (id, buffer) in done {
        let results = tgen.results.clone();
        let handle = buffer.clone();
        commands.spawn(ReadbackOnce::buffer(buffer)).observe(
            move |ev: On<ReadbackComplete>, mut buffers: ResMut<Assets<ShaderBuffer>>, mut commands: Commands| {
                let data: Vec<f32> = ev.data.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
                if let Ok(mut r) = results.lock() {
                    r.deliver(id, data);
                }
                buffers.remove(&handle);
                commands.entity(ev.entity).despawn();
            },
        );
    }
}

#[derive(Resource)]
struct GenPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
    noise: TextureView,
}

#[derive(Resource, Default)]
struct RenderJobs {
    last_id: u64,
    pending: Vec<GenJob>,
    ready: Vec<(u64, Handle<ShaderBuffer>, BindGroup)>,
}

fn init_pipeline(mut commands: Commands, assets: Res<AssetServer>, cache: Res<PipelineCache>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let layout = BindGroupLayoutDescriptor::new(
        "tg_height",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer::<Vec<f32>>(false),
                uniform_buffer::<TgParams>(false),
                texture_3d(TextureSampleType::Float { filterable: false }),
            ),
        ),
    );
    let pipeline = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("tg_height".into()),
        layout: vec![layout.clone()],
        shader: assets.load("shaders/tg_height.wesl"),
        ..default()
    });
    let n = se_core::tg::NOISE3D_SIZE as u32;
    let texture = device.create_texture_with_data(
        &queue,
        &TextureDescriptor {
            label: Some("tg_noise3d"),
            size: Extent3d { width: n, height: n, depth_or_array_layers: n },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D3,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &se_core::tg::noise3d(),
    );
    let noise = texture.create_view(&TextureViewDescriptor::default());
    commands.insert_resource(GenPipeline { layout, pipeline, noise });
}

/// New jobs in; bind groups for those whose buffers have reached the GPU.
fn prepare_jobs(
    queue: Res<GenQueue>,
    mut jobs: ResMut<RenderJobs>,
    pipeline: Option<Res<GenPipeline>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let last = jobs.last_id;
    let new: Vec<GenJob> = queue.jobs.iter().filter(|j| j.id > last).cloned().collect();
    if let Some(max) = new.iter().map(|j| j.id).max() {
        jobs.last_id = max;
    }
    jobs.pending.extend(new);
    let Some(pipeline) = pipeline else { return };
    let layout = cache.get_bind_group_layout(&pipeline.layout);
    let pending = std::mem::take(&mut jobs.pending);
    for job in pending {
        let Some(buf) = buffers.get(&job.buffer) else {
            jobs.pending.push(job);
            continue;
        };
        let mut params = UniformBuffer::from(job.params);
        params.write_buffer(&device, &render_queue);
        let Some(binding) = params.binding() else { continue };
        let bind_group = device.create_bind_group(
            "tg_height",
            &layout,
            &BindGroupEntries::sequential((buf.buffer.as_entire_buffer_binding(), binding, &pipeline.noise)),
        );
        jobs.ready.push((job.id, job.buffer, bind_group));
    }
}

fn dispatch(mut ctx: RenderContext, cache: Res<PipelineCache>, pipeline: Option<Res<GenPipeline>>, mut jobs: ResMut<RenderJobs>, queue: Res<GenQueue>) {
    let Some(pipeline) = pipeline else { return };
    match cache.get_compute_pipeline_state(pipeline.pipeline) {
        // The shader (or an import) still loading: wait.
        CachedPipelineState::Err(ShaderCacheError::ShaderNotLoaded(_) | ShaderCacheError::ShaderImportNotYetAvailable) => return,
        CachedPipelineState::Err(e) => {
            if !queue.failed.swap(true, Ordering::Relaxed) {
                error!("terrain generator pipeline failed: {e}");
            }
            return;
        }
        _ => {}
    }
    let Some(compute) = cache.get_compute_pipeline(pipeline.pipeline) else { return };
    if jobs.ready.is_empty() {
        return;
    }
    let take = jobs.ready.len().min(MAX_DISPATCH_PER_FRAME);
    let batch: Vec<_> = jobs.ready.drain(..take).collect();
    {
        let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("tg_height"), ..default() });
        pass.set_pipeline(compute);
        for (_, _, bind_group) in &batch {
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(GEN_N.div_ceil(8), GEN_N.div_ceil(8), 1);
        }
    }
    if let Ok(mut d) = queue.dispatched.lock() {
        d.extend(batch.into_iter().map(|(id, buffer, _)| (id, buffer)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forgotten_jobs_leave_no_grids() {
        let tgen = TerrainGen::default();
        let deliver = |id| tgen.results.lock().unwrap().deliver(id, vec![0.0; 4]);
        // Job 1 is back, 2 still being made, 3 kept: forgetting 1 and 2 drops 1 now and 2 on arrival.
        deliver(1);
        tgen.forget([1, 2]);
        deliver(2);
        deliver(3);
        assert!(tgen.take(1).is_none() && tgen.take(2).is_none());
        assert!(tgen.take(3).is_some());
        let r = tgen.results.lock().unwrap();
        assert!(r.done.is_empty() && r.forgotten.is_empty());
    }
}
