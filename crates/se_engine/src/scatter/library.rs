//! The scatter's models: their meshes packed into one vertex and one index buffer, their
//! materials, the (model, LOD) buckets with their caps, the indirect draws, and the impostor atlas
//! baked on a worker thread (`impostor.rs`).
//!
//! The models, in order: Megaton Rainfall's 16 broadleaf trees; Earth's 8 generated species
//! (conifers, palms, an acacia, shrubs: `trees.rs`); the active planet's flora (8 generated species,
//! made again with its impostors when a generated life planet becomes the active body); 4 grass
//! tufts. The materials are fixed: MR's trees' own, then MR's 9 barks and 8 branch cards, the port's
//! conifer and palm cards, and MR's grass atlas.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::dds::decode_level;
use se_core::mr::MrInstall;

use super::impostor::{self, BakeModel, BakeSub, Baked};
use super::trees::{self, Leaf, Recipe};
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
    /// The tree models (0..trees: MR's, Earth's species from `earth0`, the flora from `flora0`) and
    /// the grass tufts after them.
    pub trees: u32,
    pub grass: u32,
    pub earth0: u32,
    pub flora0: u32,
    /// The flora's key (`flora_key`) the flora models were made for.
    pub flora_key: u64,
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

/// A flora the scatter wants for the active body (`Scatter::flora_want`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloraWant {
    pub key: u64,
    pub seed: u32,
    pub gravity: f32,
    pub pressure: f32,
}

/// The key of a flora: its seed and its gravity and air, rounded.
pub fn flora_key(seed: u32, gravity: f32, pressure: f32) -> u64 {
    (seed as u64) << 32 | ((gravity * 10.0).round() as u64 & 0xffff) << 16 | ((pressure * 100.0).round() as u64 & 0xffff)
}

pub struct LibraryPlugin;

impl Plugin for LibraryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScatterLibrary>()
            .add_plugins(ExtractResourcePlugin::<ScatterLibrary>::default())
            .add_systems(Startup, start_library)
            .add_systems(Update, (finish_bake, update_flora).chain());
    }
}

/// Instances per bucket (one model) at most: the meshes near the camera, the impostors farther out;
/// a grass tuft's meshes (about 2.5 a square metre within 120 m, over 4 tufts). Earth's and the
/// flora's species fill a forest with a few models (a taiga is three conifers): more each.
const MESH_CAP: u32 = 2048;
const IMPOSTOR_CAP: u32 = 16_384;
const SPECIES_MESH_CAP: u32 = 4096;
const SPECIES_IMPOSTOR_CAP: u32 = 40_000;
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

/// A model's mesh and sub-meshes (material indices into the library's materials).
#[derive(Clone)]
pub struct ModelSrc {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// (first index, index count, material).
    pub subs: Vec<(u32, u32, u32)>,
    /// Mesh and impostor bucket caps.
    pub caps: (u32, u32),
    /// The crown (centre height, radius) for the leaves' ball normals; `None`: from the leaves.
    pub crown: Option<(f32, f32)>,
}

/// The main world's copy of what the library was built from (for the flora's rebuilds).
#[derive(Resource)]
struct LibrarySources {
    models: Vec<ModelSrc>,
    /// Per material: the decoded texture the bake samples.
    decoded: Vec<impostor::Texture>,
    /// Material indices: MR's barks, its branch cards, the conifer and palm cards.
    bark_mats: Vec<u32>,
    branch_mats: Vec<u32>,
    conifer_mat: u32,
    palm_mat: u32,
    /// The whole atlas as baked (the flora's layers are replaced in it).
    baked: Option<Arc<Baked>>,
}

/// The impostor bake running on a worker thread, with the atlas images made there too (the whole
/// library, or a new flora's layers spliced into the stored atlas), and for a flora its key, its
/// models and the library assembled with it.
#[derive(Resource)]
struct BakeTask {
    task: Task<(Baked, Image, Image)>,
    flora: Option<(u64, Vec<ModelSrc>, Assembled)>,
}

/// The packed library.
#[derive(Clone)]
struct Assembled {
    vertices: Vec<LibVertex>,
    indices: Vec<u32>,
    models: Vec<ModelGpu>,
    caps: Vec<u32>,
    draws: Vec<DrawEntry>,
}

/// Packs `models` and lists what the bake needs (per model, in order).
fn assemble(models: &[ModelSrc], materials: &[LibMaterial], decoded: &[impostor::Texture]) -> (Assembled, Vec<BakeModel>) {
    let mut a = Assembled { vertices: Vec::new(), indices: vec![0, 1, 2, 2, 1, 3], models: Vec::new(), caps: Vec::new(), draws: Vec::new() };
    let mut bake = Vec::new();
    for (m, src) in models.iter().enumerate() {
        let (mesh_bucket, impostor_bucket) = (a.caps.len() as u32, a.caps.len() as u32 + 1);
        a.caps.extend([src.caps.0, src.caps.1]);
        let base_vertex = a.vertices.len() as i32;
        for i in 0..src.positions.len() {
            a.vertices.push(LibVertex {
                pos: src.positions[i],
                normal: src.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                uv: src.uvs.get(i).copied().unwrap_or([0.0; 2]),
            });
        }
        let mut subs = Vec::new();
        let mut leaf_lo = f32::MAX;
        let (mut leaf_hi, mut leaf_r) = (f32::MIN, 0.0f32);
        for &(first, count, mat) in &src.subs {
            let material = &materials[mat as usize];
            let first_index = a.indices.len() as u32;
            // The indices are absolute within the model: the draw's base vertex is the model's first.
            a.indices.extend(&src.indices[first as usize..(first + count) as usize]);
            a.draws.push(DrawEntry { bucket: mesh_bucket, first_index, index_count: count, base_vertex, kind: DrawKind::Mesh { material: mat, leaves: material.leaves } });
            subs.push(BakeSub { first_index: first, index_count: count, texture: decoded[mat as usize].clone(), tint: material.tint, leaves: material.leaves });
            if material.leaves {
                for &i in &src.indices[first as usize..(first + count) as usize] {
                    let p = src.positions[i as usize];
                    leaf_lo = leaf_lo.min(p[1]);
                    leaf_hi = leaf_hi.max(p[1]);
                    leaf_r = leaf_r.max((p[0] * p[0] + p[2] * p[2]).sqrt());
                }
            }
        }
        a.draws.push(DrawEntry { bucket: impostor_bucket, first_index: 0, index_count: 6, base_vertex: 0, kind: DrawKind::Impostor });
        let (lo, hi) = src.positions.iter().fold((f32::MAX, f32::MIN), |acc, p| (acc.0.min(p[1]), acc.1.max(p[1])));
        let height = hi - lo.min(0.0);
        let horiz = src.positions.iter().map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt()).fold(0.0f32, f32::max);
        // The crown: the leaves' vertices' bounds.
        let (cy, cr) = src.crown.unwrap_or(if leaf_hi > leaf_lo {
            ((leaf_lo + leaf_hi) * 0.5, ((leaf_hi - leaf_lo) * 0.5).max(leaf_r))
        } else {
            (height * 0.6, horiz)
        });
        let half = (horiz.max(height * 0.5)) * 1.04;
        let centre_y = height * 0.5;
        a.models.push(ModelGpu {
            half,
            centre_y,
            radius: (horiz * horiz + (height * 0.5).powi(2)).sqrt(),
            layer0: m as u32 * impostor::VIEWS,
            crown_y: cy,
            crown_r: cr,
            mesh_bucket,
            impostor_bucket,
            mean: [0.25, 0.25, 0.25, 0.5],
        });
        bake.push(BakeModel {
            positions: src.positions.clone(),
            normals: src.normals.clone(),
            uvs: src.uvs.clone(),
            indices: src.indices.clone(),
            subs,
            half,
            centre_y,
            crown_y: cy,
            crown_r: cr,
        });
    }
    (a, bake)
}

/// A generated species as a model.
fn species_model(r: &Recipe, src: &LibrarySources) -> ModelSrc {
    let g = trees::grow(r);
    let mut indices = g.bark.clone();
    let nb = indices.len() as u32;
    indices.extend(&g.leaves);
    let leaf_mat = match r.leaf {
        Leaf::Branch(i) => src.branch_mats[i as usize % src.branch_mats.len()],
        Leaf::Conifer => src.conifer_mat,
        Leaf::Palm => src.palm_mat,
    };
    let bark_mat = src.bark_mats[r.bark as usize % src.bark_mats.len()];
    ModelSrc {
        positions: g.positions,
        normals: g.normals,
        uvs: g.uvs,
        indices,
        subs: vec![(0, nb, bark_mat), (nb, g.leaves.len() as u32, leaf_mat)],
        caps: (SPECIES_MESH_CAP, SPECIES_IMPOSTOR_CAP),
        crown: None,
    }
}

fn start_library(mut commands: Commands, mut mr: ResMut<MrAssets>, mut images: ResMut<Assets<Image>>, mut lib: ResMut<ScatterLibrary>) {
    let Some(install) = mr.install.clone() else { return };
    let mut materials: Vec<LibMaterial> = Vec::new();
    let mut decoded_all: Vec<impostor::Texture> = Vec::new();
    let mut models: Vec<ModelSrc> = Vec::new();
    // MR's trees.
    for name in TREES {
        let model = match install.model(name) {
            Ok(m) => m,
            Err(e) => {
                warn!("scatter: {e}");
                continue;
            }
        };
        let mut subs = Vec::new();
        for s in &model.submeshes {
            let mat = install.resolve(&s.material);
            let leaves = mat.effect.contains("leaves") || mat.effect.contains("alpha_test");
            let Some(tex) = mat.textures.first() else { continue };
            let Some(color) = mr.image(&mut images, tex, true) else { continue };
            let material = materials.len() as u32;
            materials.push(LibMaterial { color, tint: mat.diffuse(), leaves });
            decoded_all.push(decoded(&install, tex));
            subs.push((s.first_index, s.index_count, material));
        }
        models.push(ModelSrc {
            positions: model.positions.clone(),
            normals: model.normals.clone(),
            uvs: model.uvs.clone(),
            indices: model.indices.clone(),
            subs,
            caps: (MESH_CAP, IMPOSTOR_CAP),
            crown: None,
        });
    }
    if models.is_empty() {
        return;
    }
    // The generated species' materials.
    let mut mat = |images: &mut Assets<Image>, mr: &mut MrAssets, name: &str, leaves: bool| -> Option<u32> {
        let color = mr.image(images, name, true)?;
        materials.push(LibMaterial { color, tint: [1.0; 4], leaves });
        decoded_all.push(decoded(&install, name));
        Some(materials.len() as u32 - 1)
    };
    let mut bark_mats: Vec<u32> = trees::BARKS.iter().filter_map(|b| mat(&mut images, &mut mr, b, false)).collect();
    let mut branch_mats: Vec<u32> = trees::BRANCHES.iter().filter_map(|b| mat(&mut images, &mut mr, b, true)).collect();
    // Without them, the generated species wear MR's trees' own bark and leaves.
    if bark_mats.is_empty() || branch_mats.is_empty() {
        warn!("scatter: Megaton Rainfall's bark or branch textures are missing: the generated trees wear its trees' own");
    }
    if bark_mats.is_empty() {
        bark_mats.extend(materials.iter().position(|m| !m.leaves).map(|i| i as u32));
    }
    if branch_mats.is_empty() {
        branch_mats.extend(materials.iter().position(|m| m.leaves).map(|i| i as u32));
    }
    let mut card = |name: &str| -> u32 {
        let (image, tex) = own_card(name);
        materials.push(LibMaterial { color: images.add(image), tint: [1.0; 4], leaves: true });
        decoded_all.push(tex);
        materials.len() as u32 - 1
    };
    let (conifer_mat, palm_mat) = (card("conifer_branch"), card("palm_frond"));
    if bark_mats.is_empty() || branch_mats.is_empty() {
        return;
    }
    let mut src = LibrarySources { models: Vec::new(), decoded: Vec::new(), bark_mats, branch_mats, conifer_mat, palm_mat, baked: None };
    let earth0 = models.len() as u32;
    for r in trees::earth_species() {
        models.push(species_model(&r, &src));
    }
    let flora0 = models.len() as u32;
    let key = flora_key(0, 9.81, 1.0);
    for r in trees::flora(0, 9.81, 1.0) {
        models.push(species_model(&r, &src));
    }
    let trees_n = models.len() as u32;
    // The grass tufts.
    let mut grass = 0;
    if let Some(color) = mr.image(&mut images, GRASS_TEXTURE, true) {
        let material = materials.len() as u32;
        materials.push(LibMaterial { color, tint: GRASS_TINT, leaves: true });
        decoded_all.push(decoded(&install, GRASS_TEXTURE));
        for t in 0..GRASS_TUFTS {
            let (positions, normals, uvs, idx) = tuft(t);
            let n = idx.len() as u32;
            // The tuft shades as the ground under it: its "crown" far below makes the leaves' ball
            // normals point up (scatter_draw.wesl).
            models.push(ModelSrc { positions, normals, uvs, indices: idx, subs: vec![(0, n, material)], caps: (GRASS_CAP, 64), crown: Some((-50.0, 50.0)) });
            grass += 1;
        }
    }
    let (a, bake) = assemble(&models, &materials, &decoded_all);
    info!("scatter: library of {} models ({} generated), {} vertices, {} draws; baking impostors", a.models.len(), trees_n - earth0, a.vertices.len(), a.draws.len());
    *lib = ScatterLibrary {
        generation: 0,
        trees: trees_n,
        grass,
        earth0,
        flora0,
        flora_key: key,
        vertices: Arc::new(a.vertices),
        indices: Arc::new(a.indices),
        materials,
        models: a.models,
        bucket_caps: a.caps,
        draws: a.draws,
        impostor_color: None,
        impostor_normal: None,
    };
    src.models = models;
    src.decoded = decoded_all;
    commands.insert_resource(src);
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let b = impostor::bake(&bake);
        let (c, n) = impostor::images(&b);
        (b, c, n)
    });
    commands.insert_resource(BakeTask { task, flora: None });
}

/// Makes the flora the scatter wants: its 8 species grown, the library assembled with them, and
/// their impostors baked on a worker thread (the library switches to it when they are done).
fn update_flora(
    mut commands: Commands,
    scatter: Res<super::Scatter>,
    lib: Res<ScatterLibrary>,
    src: Option<Res<LibrarySources>>,
    task: Option<Res<BakeTask>>,
) {
    let (Some(src), Some(want)) = (src, scatter.flora_want) else { return };
    if task.is_some() || src.baked.is_none() || want.key == lib.flora_key {
        return;
    }
    let mut models = src.models.clone();
    let f0 = lib.flora0 as usize;
    for (i, r) in trees::flora(want.seed, want.gravity, want.pressure).iter().enumerate() {
        models[f0 + i] = species_model(r, &src);
    }
    let (a, mut bake) = assemble(&models, &lib.materials, &src.decoded);
    let flora_bake: Vec<BakeModel> = bake.drain(f0..f0 + trees::SPECIES as usize).collect();
    info!("scatter: a flora for the body (gravity {:.1} m/s^2, air {:.2} atm): baking its impostors", want.gravity, want.pressure);
    // The flora's layers and mean colours spliced into a copy of the whole atlas, and its images
    // made, on the worker (a copy of about 150 MB).
    let Some(old) = src.baked.clone() else { return };
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let fresh = impostor::bake(&flora_bake);
        let mut b = Baked { layers: old.layers, color: old.color.clone(), normal: old.normal.clone(), means: old.means.clone() };
        let l0 = f0 * impostor::VIEWS as usize;
        for (i, (c, n)) in fresh.color.into_iter().zip(fresh.normal).enumerate() {
            b.color[l0 + i] = c;
            b.normal[l0 + i] = n;
        }
        for (i, m) in fresh.means.into_iter().enumerate() {
            b.means[f0 + i] = m;
        }
        let (c, n) = impostor::images(&b);
        (b, c, n)
    });
    commands.insert_resource(BakeTask { task, flora: Some((want.key, models, a)) });
}

fn finish_bake(
    mut commands: Commands,
    task: Option<ResMut<BakeTask>>,
    src: Option<ResMut<LibrarySources>>,
    scatter: Res<super::Scatter>,
    mut lib: ResMut<ScatterLibrary>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(mut task), Some(mut src)) = (task, src) else { return };
    let Some((baked, color, normal)) = check_ready(&mut task.task) else { return };
    let flora = task.flora.take();
    commands.remove_resource::<BakeTask>();
    if let Some((key, models, a)) = flora {
        // The camera came back to a body whose flora the library holds already: keep it.
        if scatter.flora_want.is_some_and(|w| w.key == lib.flora_key && w.key != key) {
            return;
        }
        lib.vertices = Arc::new(a.vertices);
        lib.indices = Arc::new(a.indices);
        lib.models = a.models;
        lib.bucket_caps = a.caps;
        lib.draws = a.draws;
        lib.flora_key = key;
        src.models = models;
    }
    for (m, mean) in lib.models.iter_mut().zip(&baked.means) {
        m.mean = *mean;
    }
    lib.impostor_color = Some(images.add(color));
    lib.impostor_normal = Some(images.add(normal));
    lib.generation += 1;
    info!("scatter: impostors baked ({} layers of {} px)", baked.layers, impostor::SIZE);
    src.baked = Some(Arc::new(baked));
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

/// One of the port's own leaf cards (`assets/textures/vegetation/NAME.png`, made with the image
/// generator: see the vegetation plan): a GPU image with its mips (coverage-kept alpha, as the
/// impostors') and the decoded top level for the bake. A grey square if the file is missing.
fn own_card(name: &str) -> (Image, impostor::Texture) {
    // (Beside the exe's own assets first; a copy of the exe with only its shaders beside it finds
    // the cards in the source tree.)
    let rel = std::path::Path::new("textures").join("vegetation").join(format!("{name}.png"));
    let mut path = crate::config::asset_dir().join(&rel);
    if !path.is_file() {
        path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join(&rel);
    }
    let rgba = std::fs::read(&path).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8());
    let (w, h, px) = match rgba {
        Some(i) if i.width() == i.height() && i.width().is_power_of_two() => (i.width(), i.height(), i.into_raw()),
        _ => {
            warn!("scatter: leaf card {} missing or not a power-of-two square", path.display());
            (1, 1, vec![128, 128, 128, 255])
        }
    };
    let top: Vec<[f32; 4]> = px.chunks_exact(4).map(|p| [impostor::srgb_to_linear(p[0]), impostor::srgb_to_linear(p[1]), impostor::srgb_to_linear(p[2]), p[3] as f32 / 255.0]).collect();
    let levels = impostor::mips(top, w as usize);
    let mut data = Vec::new();
    for l in &levels {
        data.extend(l.iter().flat_map(|p| [impostor::linear_to_srgb(p[0]), impostor::linear_to_srgb(p[1]), impostor::linear_to_srgb(p[2]), (p[3] * 255.0).round() as u8]));
    }
    let mut image = Image::new_uninit(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels.len() as u32;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    (image, impostor::Texture { width: w, height: h, rgba: Arc::new(px) })
}
