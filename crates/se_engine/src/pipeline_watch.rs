//! Watches Bevy's pipeline cache. Bevy builds each GPU pipeline in the background the first time
//! something is drawn with it, and nothing is drawn with it until it is built. The NVIDIA driver
//! takes 5-6 s for each variant of the ground's shader (planet.wesl, on the user's RTX 5080; Bevy's
//! own work on it is about 0.1 s), so the ground was missing for seconds after loading and again on
//! landing, when the finer nodes asked for their variants (the user's 2026-11-04 video). This:
//! - logs every pipeline that took over half a second (se.log shows what a wait was);
//! - tells the main world which of the ground's variants are built ([`GroundPipelines`]): a node
//!   whose own variant is not built yet draws with the per-pixel one (terrain.rs `NodeNoise`), and
//!   the loading screen waits for what the first view needs (splash.rs);
//! - builds the ground's variants before they are needed ([`warm_ground`]): one degenerate triangle
//!   per variant, which draws no pixel, makes Bevy build its pipeline.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{CachedPipelineState, PipelineCache, PipelineDescriptor};
use bevy::render::storage::ShaderBuffer;
use bevy::render::{Render, RenderApp, RenderSystems};

use crate::terrain::{DefaultTextures, PatchLight, PlanetMaterial, PlanetUniform, VertexNoise};

pub struct PipelineWatchPlugin;

impl Plugin for PipelineWatchPlugin {
    fn build(&self, app: &mut App) {
        let pipes = GroundPipelines::default();
        app.insert_resource(pipes.clone()).add_systems(Update, warm_ground);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.insert_resource(pipes).add_systems(Render, watch_pipelines.in_set(RenderSystems::Cleanup));
    }
}

/// What a ground pipeline draws: the ground, its skirts (the alpha-mask phase), the sea surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GroundKind {
    Ground,
    Skirt,
    Sea,
}

const KINDS: usize = 3;
const SLOTS: usize = VertexNoise::ALL.len() * KINDS;

const UNKNOWN: u8 = 0;
const BUILDING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

fn slot(noise: VertexNoise, kind: GroundKind) -> usize {
    noise.index() * KINDS + kind as usize
}

#[derive(Default)]
struct Shared {
    /// Per ground variant (noise x kind): never asked for, building, built or failed. With several
    /// pipelines of a variant (views of different MSAA or format), building while any is.
    slots: [AtomicU8; SLOTS],
    /// The other pipelines still building.
    others: AtomicU32,
    /// Bumped whenever a slot changes.
    revision: AtomicU32,
    /// Pipelines in the cache (perf.rs logs it: the cache only grows).
    count: AtomicU32,
    /// The render world's images and their size on the GPU (MB, mips included), for perf.rs.
    gpu_images: AtomicU32,
    gpu_image_mb: AtomicU32,
    /// wgpu's live objects: bind groups, buffers, textures, texture views (its report), for perf.rs.
    wgpu: [AtomicU32; 4],
    /// The render world's entities and its mesh instances (perf.rs).
    render_entities: AtomicU32,
    mesh_instances: AtomicU32,
}

/// The ground's pipeline variants as the render world last saw them, shared with the main world.
#[derive(Resource, Clone, Default)]
pub struct GroundPipelines(Arc<Shared>);

impl GroundPipelines {
    fn state(&self, noise: VertexNoise, kind: GroundKind) -> u8 {
        self.0.slots[slot(noise, kind)].load(Ordering::Relaxed)
    }

    pub fn ready(&self, noise: VertexNoise, kind: GroundKind) -> bool {
        self.state(noise, kind) == READY
    }

    /// The variant a ground node can draw with now: `want` once its ground's and skirts' pipelines
    /// are built, else the per-pixel `None` (the same picture, slower).
    pub fn usable(&self, want: VertexNoise) -> VertexNoise {
        if want == VertexNoise::None || (self.ready(want, GroundKind::Ground) && self.ready(want, GroundKind::Skirt)) {
            want
        } else {
            VertexNoise::None
        }
    }

    pub fn revision(&self) -> u32 {
        self.0.revision.load(Ordering::Relaxed)
    }

    /// Pipelines other than the ground's still building.
    pub fn others_building(&self) -> u32 {
        self.0.others.load(Ordering::Relaxed)
    }

    /// Pipelines in Bevy's cache.
    pub fn count(&self) -> u32 {
        self.0.count.load(Ordering::Relaxed)
    }

    /// The render world's entities and mesh instances.
    pub fn render_counts(&self) -> (u32, u32) {
        (self.0.render_entities.load(Ordering::Relaxed), self.0.mesh_instances.load(Ordering::Relaxed))
    }

    /// wgpu's live bind groups, buffers, textures and texture views.
    pub fn wgpu_objects(&self) -> [u32; 4] {
        self.0.wgpu.each_ref().map(|a| a.load(Ordering::Relaxed))
    }

    /// The images on the GPU: their count and size (MB).
    pub fn gpu_images(&self) -> (u32, u32) {
        (self.0.gpu_images.load(Ordering::Relaxed), self.0.gpu_image_mb.load(Ordering::Relaxed))
    }
}

/// A planet.wesl pipeline's variant.
fn ground_slot(d: &PipelineDescriptor) -> Option<usize> {
    let PipelineDescriptor::RenderPipelineDescriptor(r) = d else { return None };
    if !r.vertex.shader.path().is_some_and(|p| p.path().ends_with("planet.wesl")) {
        return None;
    }
    let has = |name: &str| r.vertex.shader_defs.iter().any(|d| matches!(d, bevy::shader::ShaderDefVal::Bool(n, true) if n == name));
    let kind = if has("PREMULTIPLY_ALPHA") || has("BLEND_PREMULTIPLIED_ALPHA") {
        GroundKind::Sea
    } else if has("MAY_DISCARD") {
        GroundKind::Skirt
    } else {
        GroundKind::Ground
    };
    Some(slot(VertexNoise::from_defs(has), kind))
}

#[derive(Default)]
struct Seen {
    start: Option<std::time::Instant>,
    /// When each pipeline still building was first seen.
    waiting: HashMap<usize, f64>,
    /// How many pipelines were ever seen (the cache only grows).
    count: usize,
    /// The longest wait and the count built in the current batch, logged once it completes.
    batch: Option<(f64, usize)>,
}

/// A pipeline's shader file and defines, for the log.
fn describe(d: &PipelineDescriptor) -> String {
    let (label, shader, defs) = match d {
        PipelineDescriptor::RenderPipelineDescriptor(r) => (r.label.as_deref(), &r.vertex.shader, &r.vertex.shader_defs),
        PipelineDescriptor::ComputePipelineDescriptor(c) => (c.label.as_deref(), &c.shader, &c.shader_defs),
        _ => return "mesh pipeline".into(),
    };
    let file = shader.path().map_or_else(|| "?".to_string(), |p| p.path().display().to_string());
    use bevy::shader::ShaderDefVal as V;
    let defs: Vec<String> = defs
        .iter()
        .map(|d| match d {
            V::Bool(n, true) => n.to_string(),
            V::Bool(n, false) => format!("!{n}"),
            V::Int(n, v) => format!("{n}={v}"),
            V::UInt(n, v) => format!("{n}={v}"),
        })
        .collect();
    format!("{} [{}] ({})", label.unwrap_or("-"), file, defs.join(", "))
}

fn watch_pipelines(
    cache: Res<PipelineCache>,
    pipes: Res<GroundPipelines>,
    gpu_images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    instance: Res<bevy::render::renderer::RenderInstance>,
    entities: &bevy::ecs::entity::Entities,
    mesh_instances: Option<Res<bevy::pbr::RenderMeshInstances>>,
    mut seen: Local<Seen>,
    mut frame: Local<u32>,
) {
    *frame += 1;
    if *frame % 60 == 1 {
        let mut bytes = 0u64;
        let mut n = 0u32;
        for (_, g) in gpu_images.iter() {
            let t = &g.texture_descriptor;
            let (bw, bh) = t.format.block_dimensions();
            let block = t.format.block_copy_size(None).unwrap_or(4) as u64;
            let level0 = t.size.width.div_ceil(bw) as u64 * t.size.height.div_ceil(bh) as u64 * t.size.depth_or_array_layers as u64 * block;
            bytes += if t.mip_level_count > 1 { level0 * 4 / 3 } else { level0 };
            n += 1;
        }
        pipes.0.gpu_images.store(n, Ordering::Relaxed);
        pipes.0.render_entities.store(entities.count_spawned() as u32, Ordering::Relaxed);
        if let Some(m) = mesh_instances {
            let n = match &*m {
                bevy::pbr::RenderMeshInstances::CpuBuilding(c) => c.len(),
                bevy::pbr::RenderMeshInstances::GpuBuilding(g) => g.len(),
            };
            pipes.0.mesh_instances.store(n as u32, Ordering::Relaxed);
        }
        if let Some(r) = instance.generate_report() {
            let h = r.hub_report();
            for (a, c) in pipes.0.wgpu.iter().zip([&h.bind_groups, &h.buffers, &h.textures, &h.texture_views]) {
                a.store(c.num_allocated as u32, Ordering::Relaxed);
            }
        }
        pipes.0.gpu_image_mb.store((bytes >> 20) as u32, Ordering::Relaxed);
    }
    let start = *seen.start.get_or_insert_with(std::time::Instant::now);
    let now = start.elapsed().as_secs_f64();
    let mut slots = [UNKNOWN; SLOTS];
    let mut others = 0;
    let mut n = 0;
    for (id, p) in cache.pipelines().enumerate() {
        n = id + 1;
        let state = match p.state {
            CachedPipelineState::Queued | CachedPipelineState::Creating(_) => BUILDING,
            CachedPipelineState::Ok(_) => READY,
            CachedPipelineState::Err(_) => FAILED,
        };
        match ground_slot(&p.descriptor) {
            Some(s) => slots[s] = merge(slots[s], state),
            None if state == BUILDING => others += 1,
            None => {}
        }
        if id >= seen.count {
            if state == BUILDING {
                seen.waiting.insert(id, now);
            }
            continue;
        }
        if state != BUILDING && let Some(t0) = seen.waiting.remove(&id) {
            let wait = now - t0;
            let b = seen.batch.get_or_insert((0.0, 0));
            b.0 = b.0.max(wait);
            b.1 += 1;
            if wait > 0.5 {
                info!("pipeline built in {wait:.1} s (at {now:.1} s): {}", describe(&p.descriptor));
            }
        }
    }
    seen.count = seen.count.max(n);
    if seen.waiting.is_empty() && let Some((longest, built)) = seen.batch.take() && longest > 0.5 {
        info!("pipelines: {built} built, all ready at {now:.1} s (longest wait {longest:.1} s)");
    }
    let mut changed = false;
    for (a, s) in pipes.0.slots.iter().zip(slots) {
        changed |= a.swap(s, Ordering::Relaxed) != s;
    }
    if changed {
        pipes.0.revision.fetch_add(1, Ordering::Relaxed);
    }
    pipes.0.others.store(others, Ordering::Relaxed);
    pipes.0.count.store(n as u32, Ordering::Relaxed);
}

/// Building wins over built, built over failed.
fn merge(a: u8, b: u8) -> u8 {
    for s in [BUILDING, READY, FAILED] {
        if a == s || b == s {
            return s;
        }
    }
    UNKNOWN
}

/// The ground's variants in the order they are built ahead of need: the per-pixel ones every body
/// draws first, then the terra worlds' (most bodies), then the airless ones'.
const WARM_ORDER: [(VertexNoise, GroundKind); 11] = [
    (VertexNoise::None, GroundKind::Ground),
    (VertexNoise::None, GroundKind::Skirt),
    (VertexNoise::None, GroundKind::Sea),
    (VertexNoise::Terra, GroundKind::Ground),
    (VertexNoise::Terra, GroundKind::Skirt),
    (VertexNoise::TerraPartial, GroundKind::Ground),
    (VertexNoise::TerraPartial, GroundKind::Skirt),
    (VertexNoise::TerraClimate, GroundKind::Ground),
    (VertexNoise::TerraClimate, GroundKind::Skirt),
    (VertexNoise::Selena, GroundKind::Ground),
    (VertexNoise::Selena, GroundKind::Skirt),
];
/// Variants built ahead at once: each holds a thread of the background pool (which also builds the
/// terrain) for seconds.
const WARM_AT_ONCE: usize = 3;
/// A variant not built by then is left to be built when a node needs it.
const WARM_GIVE_UP_S: f32 = 60.0;

#[derive(Default)]
struct Warm {
    next: usize,
    mesh: Option<Handle<Mesh>>,
    light: Option<Handle<ShaderBuffer>>,
    dummies: Vec<(VertexNoise, GroundKind, Entity, f32)>,
}

/// One triangle with every vertex in the same place: the ground's vertex layout, no pixel.
fn degenerate_mesh() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3])
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 3])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 3])
        .with_inserted_attribute(crate::terrain::ATTRIBUTE_HEIGHT, vec![0.0f32; 3])
        .with_inserted_attribute(crate::terrain::ATTRIBUTE_BODY_DIR, vec![[0.0f32, 1.0, 0.0]; 3])
        .with_inserted_attribute(crate::terrain::ATTRIBUTE_MORPH, vec![[0.0f32; 4]; 3])
        .with_inserted_indices(Indices::U32(vec![0, 1, 2]))
}

/// Builds the ground's pipeline variants ahead of need, [`WARM_AT_ONCE`] at a time: an entity per
/// variant drawing a degenerate triangle with a ground material of that variant, despawned once its
/// pipeline is built (Bevy keeps the pipeline).
#[allow(clippy::too_many_arguments)]
fn warm_ground(
    mut commands: Commands,
    pipes: Res<GroundPipelines>,
    time: Res<Time>,
    mut warm: Local<Warm>,
    defaults: Option<Res<DefaultTextures>>,
    surface: Option<Res<crate::surface_tex::SurfaceTextures>>,
    clouds: Option<Res<crate::volumetric_clouds::CloudShadowAssets>>,
    (mut materials, mut meshes, mut buffers): (ResMut<Assets<PlanetMaterial>>, ResMut<Assets<Mesh>>, ResMut<Assets<ShaderBuffer>>),
) {
    if warm.next >= WARM_ORDER.len() && warm.dummies.is_empty() {
        return;
    }
    let (Some(d), Some(s), Some(c)) = (defaults, surface, clouds) else { return };
    let dt = time.delta_secs();
    warm.dummies.retain_mut(|(n, k, e, age)| {
        *age += dt;
        let st = pipes.state(*n, *k);
        let keep = st != READY && st != FAILED && *age < WARM_GIVE_UP_S;
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    while warm.dummies.len() < WARM_AT_ONCE && warm.next < WARM_ORDER.len() {
        let (noise, kind) = WARM_ORDER[warm.next];
        warm.next += 1;
        if pipes.state(noise, kind) != UNKNOWN {
            continue;
        }
        let mesh = warm.mesh.get_or_insert_with(|| meshes.add(degenerate_mesh())).clone();
        let light = warm.light.get_or_insert_with(|| buffers.add(crate::terrain::light_buffer(&[PatchLight::default()]))).clone();
        let sea = if kind == GroundKind::Sea { 1.0 } else { 0.0 };
        let material = materials.add(PlanetMaterial {
            u: PlanetUniform { water: Vec4::new(1.0, 0.0, 0.0, sea), ..default() },
            color: d.white.clone(),
            lights: d.black.clone(),
            water: d.black.clone(),
            clouds: d.black.clone(),
            inscatter: d.black3d.clone(),
            transmittance: d.black.clone(),
            irradiance: d.black.clone(),
            ring: d.white.clone(),
            biome: default(),
            samples: s.samples.clone(),
            soils: s.soils.clone(),
            normals: d.white.clone(),
            soil_norm: s.soil_norm.clone(),
            soil_rhao: s.soil_rhao.clone(),
            sample_relief: s.sample_relief.clone(),
            noise: d.noise3d.clone(),
            light_page: light,
            cloud_cover: c.cover.clone(),
            cloud_shadow: c.shadow.clone(),
            cloud_map: c.map.clone(),
            cloud_weather: c.weather.clone(),
            cloud_pattern: c.pattern.clone(),
            skirt: kind == GroundKind::Skirt,
            variant: noise,
        });
        let e = commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY, Visibility::Visible, NoFrustumCulling)).id();
        warm.dummies.push((noise, kind, e, 0.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_from_defines() {
        for v in VertexNoise::ALL {
            assert_eq!(VertexNoise::from_defs(|d| v.shader_defs().contains(&d)), v);
        }
        assert_eq!(merge(READY, BUILDING), BUILDING);
        assert_eq!(merge(UNKNOWN, FAILED), FAILED);
        assert_eq!(merge(FAILED, READY), READY);
    }
}
