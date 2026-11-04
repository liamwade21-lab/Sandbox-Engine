//! The scatter's models: their meshes packed into one vertex and one index buffer, their
//! materials, the (model, LOD) buckets with their caps, the indirect draws, and the impostor atlas
//! baked on a worker thread (`impostor.rs`). Step 2 holds Megaton Rainfall's `tree_0` only.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::dds::decode_level;
use se_core::mr::MrInstall;

use super::impostor::{self, BakeModel, BakeSub, Baked};
use crate::mr_assets::MrAssets;

/// A vertex of the packed meshes (`scatter_draw.wesl` `VertexIn`).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct LibVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// What a draw draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawKind {
    /// A mesh sub-mesh with its material.
    Mesh { material: u32, leaves: bool },
    /// The model's impostor quads.
    Impostor,
}

/// One indirect draw: a bucket's instances drawn with an index range.
#[derive(Clone, Copy, Debug)]
pub struct DrawEntry {
    pub bucket: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
    pub kind: DrawKind,
}

/// A material: its colour texture and tint (linear, MR's `g_vDiffuse`).
#[derive(Clone, Debug)]
pub struct LibMaterial {
    pub color: Handle<Image>,
    pub tint: [f32; 4],
    pub leaves: bool,
}

/// A model's numbers the shaders need (`scatter.wesl` / `scatter_draw.wesl` `Model`).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct ModelGpu {
    /// The impostor quad's half size (m) and its centre's height above the base (m).
    pub half: f32,
    pub centre_y: f32,
    /// Bounding sphere radius (m) about the centre, for the view test.
    pub radius: f32,
    /// The first impostor layer of the model in the atlas.
    pub layer0: u32,
    /// The crown's centre height (m) for the leaves' ball normals, and the crown's radius.
    pub crown_y: f32,
    pub crown_r: f32,
    /// Mesh and impostor buckets.
    pub mesh_bucket: u32,
    pub impostor_bucket: u32,
}

/// The library, extracted to the render world once ready (and when it changes).
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ScatterLibrary {
    /// Bumped when the content changes (the render world re-uploads it).
    pub generation: u32,
    pub vertices: Arc<Vec<LibVertex>>,
    pub indices: Arc<Vec<u32>>,
    pub materials: Vec<LibMaterial>,
    pub models: Vec<ModelGpu>,
    /// Per bucket: its capacity (instances).
    pub bucket_caps: Vec<u32>,
    pub draws: Vec<DrawEntry>,
    /// The impostor atlas: colour (sRGB, alpha cut-out) and object-space normals.
    pub impostor_color: Option<Handle<Image>>,
    pub impostor_normal: Option<Handle<Image>>,
}

impl ScatterLibrary {
    pub fn ready(&self) -> bool {
        self.generation > 0 && self.impostor_color.is_some()
    }
}

pub struct LibraryPlugin;

impl Plugin for LibraryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScatterLibrary>()
            .add_plugins(ExtractResourcePlugin::<ScatterLibrary>::default())
            .add_systems(Startup, start_library)
            .add_systems(Update, finish_bake);
    }
}

/// The impostor bake running on a worker thread.
#[derive(Resource)]
struct BakeTask(Task<Option<Baked>>);

/// Instances per bucket at most: the meshes near the camera, the impostors farther out.
const MESH_CAP: u32 = 8192;
const IMPOSTOR_CAP: u32 = 65_536;

/// The MR trees the library holds (step 2: one).
const TREES: [&str; 1] = ["tree_0"];

fn start_library(mut commands: Commands, mut mr: ResMut<MrAssets>, mut images: ResMut<Assets<Image>>, mut lib: ResMut<ScatterLibrary>) {
    let Some(install) = mr.install.clone() else { return };
    let mut vertices: Vec<LibVertex> = Vec::new();
    // The impostors' quad (its four corners come from the vertex index).
    let mut indices: Vec<u32> = vec![0, 1, 2, 2, 1, 3];
    let mut materials: Vec<LibMaterial> = Vec::new();
    let mut models = Vec::new();
    let mut draws = Vec::new();
    let mut caps = Vec::new();
    let mut bake = Vec::new();
    for name in TREES {
        let model = match install.model(name) {
            Ok(m) => m,
            Err(e) => {
                warn!("scatter: {e}");
                continue;
            }
        };
        let m = models.len() as u32;
        let (mesh_bucket, impostor_bucket) = (caps.len() as u32, caps.len() as u32 + 1);
        caps.extend([MESH_CAP, IMPOSTOR_CAP]);
        let base_vertex = vertices.len() as i32;
        for i in 0..model.positions.len() {
            vertices.push(LibVertex {
                pos: model.positions[i],
                normal: model.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                uv: model.uvs.get(i).copied().unwrap_or([0.0; 2]),
            });
        }
        let mut subs = Vec::new();
        for s in &model.submeshes {
            let mat = install.resolve(&s.material);
            let leaves = mat.effect.contains("leaves") || mat.effect.contains("alpha_test");
            let Some(tex) = mat.textures.first() else { continue };
            let Some(color) = mr.image(&mut images, tex, true) else { continue };
            let material = materials.len() as u32;
            materials.push(LibMaterial { color, tint: mat.diffuse(), leaves });
            let first_index = indices.len() as u32;
            // MR's indices are absolute: made relative to the model's first vertex.
            indices.extend(model.indices[s.first_index as usize..(s.first_index + s.index_count) as usize].iter());
            draws.push(DrawEntry {
                bucket: mesh_bucket,
                first_index,
                index_count: s.index_count,
                base_vertex,
                kind: DrawKind::Mesh { material, leaves },
            });
            subs.push(BakeSub {
                first_index: s.first_index,
                index_count: s.index_count,
                texture: decoded(&install, tex),
                tint: mat.diffuse(),
                leaves,
            });
        }
        draws.push(DrawEntry { bucket: impostor_bucket, first_index: 0, index_count: 6, base_vertex: 0, kind: DrawKind::Impostor });
        let (lo, hi) = model.bounds();
        let height = hi[1] - lo[1].min(0.0);
        let horiz = model.positions.iter().map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt()).fold(0.0f32, f32::max);
        // The crown: the leaves' vertices' bounds.
        let (mut cy, mut cr) = (height * 0.6, horiz);
        let leaf_sub = model.submeshes.iter().find(|s| {
            let e = &install.resolve(&s.material).effect;
            e.contains("leaves")
        });
        if let Some(s) = leaf_sub {
            let range = s.first_vertex as usize..(s.first_vertex + s.vertex_count) as usize;
            let (ylo, yhi) = model.positions[range.clone()].iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
            cy = (ylo + yhi) * 0.5;
            cr = ((yhi - ylo) * 0.5).max(model.positions[range].iter().map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt()).fold(0.0, f32::max));
        }
        let half = (horiz.max(height * 0.5)) * 1.04;
        let centre_y = height * 0.5;
        models.push(ModelGpu {
            half,
            centre_y,
            radius: (horiz * horiz + (height * 0.5).powi(2)).sqrt(),
            layer0: m * impostor::VIEWS,
            crown_y: cy,
            crown_r: cr,
            mesh_bucket,
            impostor_bucket,
        });
        bake.push(BakeModel { positions: model.positions.clone(), normals: model.normals.clone(), uvs: model.uvs.clone(), indices: model.indices.clone(), subs, half, centre_y, crown_y: cy, crown_r: cr });
    }
    if models.is_empty() {
        return;
    }
    info!("scatter: library of {} models, {} vertices, {} draws; baking impostors", models.len(), vertices.len(), draws.len());
    *lib = ScatterLibrary {
        generation: 0,
        vertices: Arc::new(vertices),
        indices: Arc::new(indices),
        materials,
        models,
        bucket_caps: caps,
        draws,
        impostor_color: None,
        impostor_normal: None,
    };
    let task = AsyncComputeTaskPool::get().spawn(async move { Some(impostor::bake(&bake)) });
    commands.insert_resource(BakeTask(task));
}

/// A texture decoded to RGBA8 at its top level (the bake samples it), or a grey square.
fn decoded(mr: &Arc<MrInstall>, name: &str) -> impostor::Texture {
    if let Ok((bytes, info)) = mr.texture(name) {
        if let Ok(rgba) = decode_level(&bytes, &info, 0, 0) {
            return impostor::Texture { width: info.width, height: info.height, rgba: Arc::new(rgba) };
        }
    }
    impostor::Texture { width: 1, height: 1, rgba: Arc::new(vec![128, 128, 128, 255]) }
}

fn finish_bake(mut commands: Commands, task: Option<ResMut<BakeTask>>, mut lib: ResMut<ScatterLibrary>, mut images: ResMut<Assets<Image>>) {
    let Some(mut task) = task else { return };
    let Some(result) = check_ready(&mut task.0) else { return };
    commands.remove_resource::<BakeTask>();
    let Some(baked) = result else { return };
    let (color, normal) = impostor::images(&baked);
    lib.impostor_color = Some(images.add(color));
    lib.impostor_normal = Some(images.add(normal));
    lib.generation += 1;
    info!("scatter: impostors baked ({} layers of {} px)", baked.layers, impostor::SIZE);
}
