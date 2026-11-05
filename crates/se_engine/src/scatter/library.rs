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
    /// The mean colour of its impostor views (linear) and their coverage, for the tint that gives
    /// the trees the ground's forest colour (set when the bake is done).
    pub mean: [f32; 4],
}

/// The library, extracted to the render world once ready (and when it changes).
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ScatterLibrary {
    /// Bumped when the content changes (the render world re-uploads it).
    pub generation: u32,
    /// The trees (models 0..trees) and the grass tufts after them.
    pub trees: u32,
    pub grass: u32,
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

/// Instances per bucket (one model) at most: the meshes near the camera, the impostors farther out;
/// a grass tuft's meshes (about 2.5 a square metre within 120 m, over 4 tufts).
const MESH_CAP: u32 = 2048;
const IMPOSTOR_CAP: u32 = 16_384;
const GRASS_CAP: u32 = 40_000;

/// Megaton Rainfall's grass: its `billboard_grass` material (the 2x2 tuft atlas `Billboard_Grass`,
/// tint 0.45 0.50 0.29), one tuft per atlas cell as its `bunch_N` models (made by its code, not
/// stored: crossed quads); here three quads crossed at 60 degrees, 0.66 m high and wide (MR's tuft).
pub const GRASS_TUFTS: u32 = 4;
const GRASS_TEXTURE: &str = "Billboard_Grass";
const GRASS_TINT: [f32; 4] = [0.45, 0.50, 0.29, 1.0];
const GRASS_SIZE: f32 = 0.66;

/// Megaton Rainfall's 16 trees (temperate broadleaf; "Realtime 3D Trees XL", the pack's LOD1 meshes).
const TREES: [&str; 16] = [
    "tree_0", "tree_1", "tree_2", "tree_3", "tree_4", "tree_5", "tree_6", "tree_7", "tree_8", "tree_9", "tree_10", "tree_11", "tree_12",
    "tree_13", "tree_14", "tree_15",
];

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
            // MR's indices are absolute within the model: the draw's base vertex is the model's first.
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
            mean: [0.25, 0.25, 0.25, 0.5],
        });
        bake.push(BakeModel { positions: model.positions.clone(), normals: model.normals.clone(), uvs: model.uvs.clone(), indices: model.indices.clone(), subs, half, centre_y, crown_y: cy, crown_r: cr });
    }
    if models.is_empty() {
        return;
    }
    let trees = models.len() as u32;
    // The grass tufts.
    if let Some(color) = mr.image(&mut images, GRASS_TEXTURE, true) {
        let material = materials.len() as u32;
        materials.push(LibMaterial { color, tint: GRASS_TINT, leaves: true });
        let tex = decoded(&install, GRASS_TEXTURE);
        for t in 0..GRASS_TUFTS {
            let m = models.len() as u32;
            let (mesh_bucket, impostor_bucket) = (caps.len() as u32, caps.len() as u32 + 1);
            caps.extend([GRASS_CAP, 64]);
            let (positions, normals, uvs, idx) = tuft(t);
            let base_vertex = vertices.len() as i32;
            for i in 0..positions.len() {
                vertices.push(LibVertex { pos: positions[i], normal: normals[i], uv: uvs[i] });
            }
            let first_index = indices.len() as u32;
            indices.extend(idx.iter());
            draws.push(DrawEntry { bucket: mesh_bucket, first_index, index_count: idx.len() as u32, base_vertex, kind: DrawKind::Mesh { material, leaves: true } });
            draws.push(DrawEntry { bucket: impostor_bucket, first_index: 0, index_count: 6, base_vertex: 0, kind: DrawKind::Impostor });
            let half = GRASS_SIZE * 0.52;
            // The tuft shades as the ground under it: its "crown" far below makes the leaves' ball
            // normals point up (scatter_draw.wesl).
            let (crown_y, crown_r) = (-50.0, 50.0);
            models.push(ModelGpu {
                half,
                centre_y: GRASS_SIZE * 0.5,
                radius: GRASS_SIZE * 0.75,
                layer0: m * impostor::VIEWS,
                crown_y,
                crown_r,
                mesh_bucket,
                impostor_bucket,
                mean: [0.25, 0.25, 0.25, 0.5],
            });
            let subs = vec![BakeSub { first_index: 0, index_count: idx.len() as u32, texture: tex.clone(), tint: GRASS_TINT, leaves: true }];
            bake.push(BakeModel { positions, normals, uvs, indices: idx, subs, half, centre_y: GRASS_SIZE * 0.5, crown_y, crown_r });
        }
    }
    let grass = models.len() as u32 - trees;
    info!("scatter: library of {} models, {} vertices, {} draws; baking impostors", models.len(), vertices.len(), draws.len());
    *lib = ScatterLibrary {
        generation: 0,
        trees,
        grass,
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

/// Grass tuft `t`: three quads crossed at 60 degrees about the up axis, textured with atlas cell `t`
/// (positions, normals, uvs, indices). The normals point up: the tuft is lit as the ground.
#[allow(clippy::type_complexity)]
fn tuft(t: u32) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    let (u0, v0) = ((t % 2) as f32 * 0.5, (t / 2) as f32 * 0.5);
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let h = GRASS_SIZE * 0.5;
    for q in 0..3 {
        let a = q as f32 * std::f32::consts::PI / 3.0;
        let (dx, dz) = (a.cos() * h, a.sin() * h);
        let b = p.len() as u32;
        for (x, y, z, s, tt) in [(-dx, 0.0, -dz, 0.0, 1.0), (dx, 0.0, dz, 1.0, 1.0), (-dx, GRASS_SIZE, -dz, 0.0, 0.0), (dx, GRASS_SIZE, dz, 1.0, 0.0)] {
            p.push([x, y, z]);
            n.push([0.0, 1.0, 0.0]);
            uv.push([u0 + s * 0.5, v0 + tt * 0.5]);
        }
        idx.extend([b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
    (p, n, uv, idx)
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
    for (m, mean) in lib.models.iter_mut().zip(&baked.means) {
        m.mean = *mean;
    }
    lib.impostor_color = Some(images.add(color));
    lib.impostor_normal = Some(images.add(normal));
    lib.generation += 1;
    info!("scatter: impostors baked ({} layers of {} px)", baked.layers, impostor::SIZE);
}
