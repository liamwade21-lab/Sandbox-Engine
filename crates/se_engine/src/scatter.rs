//! GPU scatter: objects standing on a planet's ground (trees now; later grass, buildings, props and
//! cars), placed and culled on the GPU. The plan is `docs/spec-vegetation-civilizations.md` ("GPU
//! scatter system"); what it learned is in its build log.
//!
//! - **Cells.** Objects belong to fixed cells of the cube-sphere quadtree at one level per class
//!   (trees: about 400-800 m on Earth). A cell's candidates come from a hash of its key and the
//!   body's seed, so the same tree always stands in the same place. A cell within range gets a slot
//!   of the pooled instance buffer and a placement job (`scatter.wesl` `place`); out of range it
//!   gives its slot back.
//! - **Standing on the drawn ground.** Every drawn terrain node under a resident cell gets a page
//!   of the GPU height atlas: its 33 x 33 vertex positions (node-local, with the height in w),
//!   remade from its height grid exactly as `build_node` makes them ([`terrain::node_vertices`]).
//!   Each cell has a small quadtree of the drawn nodes over it ([`build_cell_tree`]): an instance
//!   walks it to the node under it, and the cull pass rebuilds the triangle the ground draws there,
//!   the geomorph included. So trees sit on whatever level of ground is drawn, and follow it as
//!   nodes split, on flattened planets too (the page's frame is the node's own).
//! - **Cull.** Every frame a compute pass (`scatter.wesl` `cull`) lifts each instance, tests the
//!   view, picks mesh or impostor by distance (dithered bands) and appends it to its (model, LOD)
//!   bucket up to the bucket's cap; `finish` writes the indirect draw arguments. No CPU readback.
//! - **GPU memory** is pooled: fixed-size buffers made once (slots, pages, the cell trees, the
//!   output), so the number of GPU objects never grows.
//!
//! Step 2 of the plan scatters Megaton Rainfall's `tree_0` evenly over the land of whatever planet
//! the camera is at, when `SE_SCATTER_TEST=1` (or the dev command `scatter test`). The vegetation
//! itself (where plants grow, the species) is step 3.

pub mod earth;
mod gpu;
mod impostor;
mod library;
pub mod trees;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use se_core::planet_tiles::{CubeFace, TileKey};

use crate::camera::FlyCamera;
use crate::system::SystemBody;
use crate::terrain::{self, GRID, MAX_GEOM_LEVEL, Terrain};
use crate::universe::{ScaledSpace, UniverseSet};

pub use library::ScatterLibrary;

/// Instance slots (one per resident cell): enough for the impostors' range at Ultra with the
/// smallest cells (400 m) and the 15 % kept past it (up to about 270), the grass's cells (20-40 m
/// within 150 m: up to about 180; nearest first, they would otherwise take the far trees' slots), and
/// the cities' buildings (about 320 near cells within 7 km, up to 200 far ones within 20 km).
pub const SLOTS: u32 = 1024;
/// Candidates (and so at most instances) per cell.
pub const SLOT_CAP: u32 = 8192;
/// Pages of the height atlas (drawn nodes under resident cells).
pub const PAGES: u32 = 2048;
/// Vertices of a page.
pub const PAGE_VERTS: u32 = (GRID * GRID) as u32;
/// Entries of all cells' node trees.
pub const TREE_MAX: u32 = 65_536;
/// Placement jobs per frame at most (each one cell, `SLOT_CAP` threads).
pub const MAX_JOBS: u32 = 24;
/// A tree leaf entry's flag; the rest is the page, `NO_PAGE` when the ground there has none.
pub const LEAF: u32 = 0x8000_0000;
pub const NO_PAGE: u32 = 0x7FFF_FFFF;

/// Distances (m, Ultra) of the tree class.
#[derive(Clone, Copy, Debug)]
pub struct LodDistances {
    /// The meshes draw to here; their dithered hand-over to the impostors is `band` wide below it.
    pub mesh_far: f32,
    pub band: f32,
    /// The impostors draw to here (fading over `band` * 4).
    pub impostor_far: f32,
    /// From `thin_start` to `thin_end` three trees in four fade out and the fourth grows to keep the
    /// cover (twice the size: four times the crown's area).
    pub thin_start: f32,
    pub thin_end: f32,
}

impl Default for LodDistances {
    fn default() -> Self {
        LodDistances { mesh_far: 250.0, band: 15.0, impostor_far: 3000.0, thin_start: 1000.0, thin_end: 1500.0 }
    }
}

/// How far below the drawn ground a tree's base stands (m): the soils' "3D stones" move the near
/// ground up and down by decimetres that the pages do not hold.
const SINK_M: f32 = 0.4;
/// Mean ground area (m^2) per tree in the step 2 test (even cover).
const TEST_AREA_PER_TREE: f64 = 900.0;
/// Ground area (m^2) per tree in the densest forest (MR's trees: 12-30 m tall, crowns 11-22 m).
const FOREST_AREA_PER_TREE: f64 = 90.0;

/// How a body's plants are decided (`scatter.wesl` `Body::veg.x`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VegKind {
    /// No land plants.
    #[default]
    None = 0,
    /// The step 2 test: an even cover of one tree over any land.
    Test = 1,
    /// SE's terra generator's climate bands (planet.wesl `se_terra_weights`, `surface_class.wesl`).
    SeTerra = 2,
    /// A colour map's forests, grassland and steppe (Earth's: `earth.rs`).
    Map = 3,
}

/// Whether a body's catalog Life gives it land plants: multicellular (organic or exotic) life whose
/// biome takes in land (`Terrestrial`, `Marine/Terrestrial`). Unicellular life keeps the ground's
/// colours only; ocean-only life has no land plants.
pub fn land_plants(block: &se_core::sc::Block) -> bool {
    se_core::body_params::LifeParams::all_from_body(block).iter().any(|l| {
        let class = l.class.as_deref().unwrap_or("").to_ascii_lowercase();
        let kind = l.kind.as_deref().unwrap_or("").to_ascii_lowercase();
        let biome = l.biome.as_deref().unwrap_or("").to_ascii_lowercase();
        (class == "organic" || class == "exotic") && (kind.contains("multi") || kind.contains("intelligent")) && biome.contains("terrestrial")
    })
}

pub struct ScatterPlugin;

impl Plugin for ScatterPlugin {
    fn build(&self, app: &mut App) {
        let test = std::env::var("SE_SCATTER_TEST").is_ok_and(|v| v != "0");
        let debug = std::env::var("SE_SCATTER_DEBUG").is_ok_and(|v| v != "0");
        let off = std::env::var("SE_SCATTER").is_ok_and(|v| v == "0");
        app.insert_resource(Scatter { enabled: !off, test, debug, ..default() })
            .init_resource::<ScatterFrame>()
            .add_plugins(ExtractResourcePlugin::<ScatterFrame>::default())
            .add_plugins(library::LibraryPlugin)
            .add_systems(PostUpdate, update_scatter.after(UniverseSet::FloatingOrigin));
        gpu::build(app);
    }
}

/// A resident cell.
struct CellSlot {
    slot: u32,
    used: u64,
    job: PlaceJob,
}

/// A page of the height atlas.
struct PageSlot {
    page: u32,
    used: u64,
}

/// The main-world state: which cells and pages are resident.
#[derive(Resource, Default)]
pub struct Scatter {
    /// On (the vegetation).
    pub enabled: bool,
    /// The step 2 test (`SE_SCATTER_TEST=1`, dev command `scatter test`): MR's trees evenly on any land.
    pub test: bool,
    /// The library is ready (MR's models read and their impostors baked): the ground draws its
    /// forest floor and canopy only then (terrain.rs).
    pub ready: bool,
    /// The flora the active body wants (a generated life planet's: `library.rs` makes it), and
    /// whether the library holds it (until then the ground draws no forest floor: terrain.rs).
    pub flora_want: Option<library::FloraWant>,
    pub flora_ready: bool,
    body: Option<Entity>,
    seed: u32,
    /// How the body's plants are decided.
    veg: VegKind,
    /// The colour map's plants around the camera (`VegKind::Map`).
    earth: earth::EarthVeg,
    /// The wind at the camera (body frame, m/s), worked out every second.
    wind: DVec3,
    wind_frame: u64,
    cells: HashMap<TileKey, CellSlot>,
    /// The buildings' cells (cities) by class (1 near, 2 far), sharing the slots with the plants'.
    bcells: HashMap<(TileKey, u32), CellSlot>,
    free_slots: Vec<u32>,
    pages: HashMap<TileKey, PageSlot>,
    free_pages: Vec<u32>,
    frame: u64,
    /// Warned once that the atlas is full.
    warned_pages: bool,
    /// `SE_SCATTER_DEBUG=1`: log the resident cells and pages every 120 frames.
    debug: bool,
    /// Every ancestor of the terrain's drawn nodes, for the cells' trees, and the terrain's
    /// `visible_rev` it was made for.
    split: HashSet<TileKey>,
    split_rev: Option<u64>,
    pub lod: LodDistances,
}

impl Scatter {
    /// Starts over with the body (after a switch of the test or of the vegetation).
    pub fn reset_body(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.cells.clear();
        self.bcells.clear();
        self.pages.clear();
        self.free_slots = (0..SLOTS).rev().collect();
        self.free_pages = (0..PAGES).rev().collect();
        self.body = None;
        self.split_rev = None;
        self.earth = default();
    }

    /// The page holding node `key`, allocated (and queued for upload) when new. `None` when the
    /// atlas is full of pages used this frame.
    fn page(&mut self, key: TileKey, terrain: &Terrain, uploads: &mut Vec<(u32, Arc<Vec<[f32; 4]>>)>) -> Option<u32> {
        let frame = self.frame;
        if let Some(p) = self.pages.get_mut(&key) {
            p.used = frame;
            return Some(p.page);
        }
        let grid = terrain.node_grid(&key)?;
        let page = match self.free_pages.pop() {
            Some(p) => p,
            None => {
                // The least recently used page not used this frame.
                let (&old, _) = self.pages.iter().filter(|(_, p)| p.used < frame).min_by_key(|(_, p)| p.used)?;
                self.pages.remove(&old).map(|p| p.page)?
            }
        };
        uploads.push((page, Arc::new(terrain::node_vertices(key, terrain.radius, grid))));
        self.pages.insert(key, PageSlot { page, used: frame });
        Some(page)
    }
}

/// Per resident cell, what the cull pass reads (`scatter.wesl` `Cell`).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct CellGpu {
    /// The ellipsoid's up at the cell's centre (render frame).
    pub up: [f32; 3],
    /// The cell's node tree's root entry.
    pub root: u32,
    /// Where the cell lies in a coarser drawn node: its column and row offsets in it, and (bits 0-7)
    /// the level difference (0 when the drawn nodes are the cell's level or finer); bit 8 set when the
    /// slot holds a placed cell this frame.
    pub off: [u32; 2],
    pub shift_live: u32,
    /// The cell's placement seed (the cull pass re-derives each instance from it and its number).
    pub seed: u32,
}

impl CellGpu {
    pub const LIVE: u32 = 0x100;
}

/// Per page, per frame (`scatter.wesl` `PageInfo`).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct PageGpu {
    /// The node's centre relative to the camera (render frame, render units).
    pub t: [f32; 3],
    /// The geomorph's split distance (m; 0: no morph).
    pub split_m: f32,
    /// The camera in the node's frame (body frame on the sphere, m from the node's centre).
    pub cam: [f32; 3],
    pub _pad: f32,
    /// The node's centre direction (body frame, the sphere's).
    pub dir: [f32; 3],
    pub _pad2: f32,
}

/// A placement job (`scatter.wesl` `Job`).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct PlaceJob {
    pub slot: u32,
    pub face: u32,
    pub level: u32,
    pub row: u32,
    pub col: u32,
    /// The chance a candidate is kept.
    pub keep: f32,
    pub seed: u32,
    /// The models its candidates take: the first, and how many in the high 16 bits.
    pub models: u32,
    /// 0 plants, 1 buildings (cities: one candidate a point of a 90 x 90 grid over the cell).
    pub kind: u32,
    /// The cell's corner in the city layout's frame (`city.wesl`): its 4,096 m tile of face metres,
    /// the corner's face metres in it, and the cell's edge in face metres.
    pub tile: [i32; 2],
    pub local: [f32; 2],
    pub edge: f32,
}

/// The body the objects stand on, per frame.
#[derive(Clone, Default, Debug)]
pub struct BodyFrame {
    /// Body frame (sphere) to render frame: orientation x the ellipsoid's axes x the space's scale.
    pub m: Mat3,
    /// The body's centre relative to the camera (render frame).
    pub centre: Vec3,
    pub radius_m: f32,
    pub light: terrain::PatchLight,
    pub uniform: terrain::PlanetUniform,
    /// The atmosphere tables (inscatter, transmittance, irradiance), if the body has them.
    pub atmosphere: Option<(Handle<Image>, Handle<Image>, Handle<Image>)>,
    pub veg: VegKind,
    /// SE's climate parameters (`BiomeUniform::climate`) and the biomes' flags.
    pub climate: [Vec4; 8],
    pub biome_flags: Vec4,
    /// Towards the star in the body frame (the ground's `g_sun_body`).
    pub sun_body: Vec3,
    /// `VegKind::Map`: the plants' patch (classes, colour) and its frame (centre and tan of the
    /// half-angle, then its two axes).
    pub patch: Option<Handle<Image>>,
    /// The lights map over the patch (Earth's towns for the cities).
    pub patch_lights: Option<Handle<Image>>,
    pub patch_frame: [Vec4; 3],
    /// The wind (render frame, m/s).
    pub wind: Vec3,
    /// SE's forest slot colour (`BiomeUniform::color[11]`, linear), the trees' colour on its terra
    /// worlds, and the gain from the trees' tint colour to the ground's look (`scatter_draw.wesl`).
    pub forest: Vec4,
    /// SE's grass slot colour (`color[10]`), the grass's there; the first grass model and how many.
    pub grass: Vec4,
    pub grass_models: UVec2,
    /// Earth's species' first model, the flora's, 1 when the flora is this body's, the buildings' box
    /// model; the ground's relief (m) for the flora's highland species.
    pub species: UVec4,
    pub relief_m: f32,
    pub tint_gain: Vec3,
    /// The body's civilization (cities): x its level (0 none), y the culture seed (fraction, as the
    /// ground's `civ.y`), z its grid turn, w the radius (m); `city2` x the buildings' range (m), y 1
    /// under `SE_CITY_TEST`, z the ground's lowest point and w its height range (m, the towns' land).
    pub city: Vec4,
    pub city2: Vec4,
}

/// What the render world needs this frame (extracted).
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ScatterFrame {
    pub active: bool,
    /// Every resident cell's placement, nearest first (the render world places the new ones).
    pub jobs: Vec<PlaceJob>,
    pub page_uploads: Vec<(u32, Arc<Vec<[f32; 4]>>)>,
    /// Indexed by page (up to the highest used).
    pub pages: Arc<Vec<PageGpu>>,
    /// Indexed by slot (up to the highest used).
    pub cells: Arc<Vec<CellGpu>>,
    pub tree: Arc<Vec<u32>>,
    pub body: BodyFrame,
    pub lod: LodDistances,
    pub sink: f32,
    /// The scatter's clock (s), for the wind.
    pub time: f32,
}

/// A cell's largest real edge (m): cells are the nodes of the cube-sphere quadtree split until
/// their edges are no longer than this, so every cell is 400-800 m across wherever it lies on the
/// cube (one fixed level would give cells differing nine times in area: the cube's corners and the
/// level's rounding).
pub const CELL_EDGE_M: f64 = 800.0;

/// Cities (the plan's step 7): buildings stand within this range (m) of the camera, one a lot of
/// the city layout (`city.wesl`), in the plants' 800 m cells (a cell of their own class).
pub const BUILD_RANGE_M: f64 = 7000.0;
/// The far buildings (the tall ones alone, from 85 % of the near range) to this range (m), in cells
/// of at most this edge (m): the city's skyline from afar.
pub const FAR_RANGE_M: f64 = 20000.0;
const FAR_EDGE_M: f64 = 3200.0;
/// The city layout's tile of face metres (`city.wesl` CITY_TILE, the ground's `SOIL_WRAP_M`).
const CITY_TILE_M: f64 = 4096.0;

/// `SE_CITY_TEST=1` (tests only): a civilization's towns cover all its suitable land (the ground's
/// streets and the buildings), to check them anywhere.
pub fn city_test() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_CITY_TEST").is_ok_and(|v| v == "1"))
}

/// Whether the active body's buildings are placed: a civilization on a generated planet (its towns
/// from the settlement field) or on Earth (from its lights map), the library holding the box, and
/// not the test cover.
fn buildings_on(sc: &Scatter, lib: &ScatterLibrary, terrain: &Terrain) -> bool {
    let towns = sc.veg == VegKind::SeTerra || (sc.veg == VegKind::Map && sc.earth.lights.is_some());
    terrain.civ.is_some() && towns && (lib.building as usize) < lib.models.len()
}

/// Grass: cells of at most this edge (m) within this range of the camera (Megaton Rainfall's 140-150
/// m), about this many tufts a square metre (MR: 2.2), shrinking to nothing over the last 10 m.
pub const GRASS_EDGE_M: f64 = 40.0;
pub const GRASS_RANGE_M: f32 = 120.0;
const GRASS_PER_M2: f64 = 2.5;

/// The longest edge (m) of node `key` on a sphere of radius `r` (chords of its corners).
fn node_edge(key: TileKey, r: f64) -> f64 {
    let (u0, v0, s) = key.uv_rect();
    let p = |a: f64, b: f64| key.face.uv_to_dir(u0 + a * s, v0 + b * s);
    let (a, b, c, d) = (p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0), p(1.0, 1.0));
    [(a, b), (a, c), (b, d), (c, d)].iter().map(|(x, y)| (*x - *y).length()).fold(0.0, f64::max) * r
}

/// Whether node `key` is a cell (no longer split) on a body of radius `r`.
fn is_cell(key: TileKey, r: f64, edge: f64) -> bool {
    key.level >= MAX_GEOM_LEVEL || node_edge(key, r) <= edge
}

/// Angular radius (rad) of node `key` about its centre direction.
fn angular_radius(key: TileKey) -> f64 {
    let (u0, v0, s) = key.uv_rect();
    let c = key.center_dir();
    [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        .iter()
        .map(|&(a, b)| key.face.uv_to_dir(u0 + a * s, v0 + b * s).dot(c).clamp(-1.0, 1.0).acos())
        .fold(0.0, f64::max)
}

/// Cells whose nearest point lies within `range` (m) of a camera at direction `dir` (body frame,
/// unit) and `height` m above the ground, on a body of radius `r`.
fn cells_in_range(dir: DVec3, height: f64, r: f64, range: f64, edge: f64, out: &mut Vec<(TileKey, f64)>) {
    fn walk(key: TileKey, dir: DVec3, height: f64, r: f64, range: f64, edge: f64, out: &mut Vec<(TileKey, f64)>) {
        let ang = key.center_dir().dot(dir).clamp(-1.0, 1.0).acos();
        let ground = ((ang - angular_radius(key)).max(0.0)) * r;
        let d = (ground * ground + height * height).sqrt();
        if d > range {
            return;
        }
        if is_cell(key, r, edge) {
            out.push((key, d));
            return;
        }
        for c in key.children() {
            walk(c, dir, height, r, range, edge, out);
        }
    }
    for face in CubeFace::ALL {
        walk(TileKey::root(face), dir, height, r, range, edge, out);
    }
}

/// Builds cell `cell`'s tree of drawn nodes into `tree`; returns (root, column/row offset in a
/// coarser drawn node, level difference). Leaves hold pages (`page` allocates them).
///
/// `split` holds every ancestor of a drawn node: a node neither drawn nor in it has no drawn ground
/// under it (past the horizon), and is a leaf without a page instead of a tree down to the deepest
/// level (which filled the tree buffer).
fn build_cell_tree(
    cell: TileKey,
    drawn: &[TileKey],
    split: &HashSet<TileKey>,
    tree: &mut Vec<u32>,
    page: &mut dyn FnMut(TileKey) -> Option<u32>,
) -> (u32, [u32; 2], u32) {
    let leaf = |p: Option<u32>| LEAF | p.unwrap_or(NO_PAGE);
    // A drawn node at the cell's level or coarser covers it all.
    for l in (0..=cell.level).rev() {
        let anc = cell.ancestor_at(l);
        if drawn.binary_search(&anc).is_ok() {
            let d = (cell.level - l) as u32;
            let root = tree.len() as u32;
            tree.push(leaf(page(anc)));
            return (root, [cell.col - (anc.col << d), cell.row - (anc.row << d)], d);
        }
    }
    fn fill(
        key: TileKey,
        at: usize,
        drawn: &[TileKey],
        split: &HashSet<TileKey>,
        tree: &mut Vec<u32>,
        page: &mut dyn FnMut(TileKey) -> Option<u32>,
    ) {
        if drawn.binary_search(&key).is_ok() {
            tree[at] = LEAF | page(key).unwrap_or(NO_PAGE);
            return;
        }
        if !split.contains(&key) || key.level >= MAX_GEOM_LEVEL || tree.len() + 4 > TREE_MAX as usize {
            tree[at] = LEAF | NO_PAGE;
            return;
        }
        let first = tree.len();
        tree.extend([0; 4]);
        tree[at] = first as u32;
        // Children in the shader's order: index = 2 * (row bit) + (column bit).
        for q in 0..4u32 {
            let child = TileKey { face: key.face, level: key.level + 1, row: key.row * 2 + q / 2, col: key.col * 2 + q % 2 };
            fill(child, first + q as usize, drawn, split, tree, page);
        }
    }
    let root = tree.len();
    tree.push(0);
    fill(cell, root, drawn, split, tree, page);
    (root as u32, [0, 0], 0)
}

/// A u32 hash of a cell key and the body's seed (the placement's per-cell seed).
fn cell_seed(key: TileKey, body_seed: u32) -> u32 {
    let mut h = body_seed ^ 0x9E37_79B9;
    for v in [key.face as u32, key.level as u32, key.row, key.col] {
        h ^= v.wrapping_mul(0x85EB_CA6B);
        h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xE654_6B64);
    }
    h
}

/// Area (m^2) of cell `key` on a sphere of radius `r`: its solid angle (the cube's cells differ
/// about 5x across a face).
fn cell_area(key: TileKey, r: f64) -> f64 {
    let (u0, v0, s) = key.uv_rect();
    let p = |a: f64, b: f64| key.face.uv_to_dir(u0 + a * s, v0 + b * s);
    // Two spherical triangles (Van Oosterom & Strackee).
    let tri = |a: DVec3, b: DVec3, c: DVec3| {
        let num = a.dot(b.cross(c)).abs();
        let den = 1.0 + a.dot(b) + b.dot(c) + c.dot(a);
        2.0 * num.atan2(den)
    };
    let (a, b, c, d) = (p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0), p(1.0, 1.0));
    (tri(a, b, d) + tri(a, d, c)) * r * r
}

#[allow(clippy::too_many_arguments)]
fn update_scatter(
    mut sc: ResMut<Scatter>,
    mut out: ResMut<ScatterFrame>,
    cam: Res<FlyCamera>,
    space: Res<ScaledSpace>,
    time: Res<Time>,
    lib: Res<ScatterLibrary>,
    data: Option<Res<crate::data::SeData>>,
    sim: Res<crate::universe::SimTime>,
    mut images: ResMut<Assets<Image>>,
    bodies: Query<(Entity, &SystemBody, &Terrain)>,
) {
    out.jobs.clear();
    out.page_uploads.clear();
    out.active = false;
    // Wrapped at 200 pi s: the wind's frequencies are whole multiples of 0.01 rad/s, so its sines
    // never jump at the wrap (Bevy's own wrap is an hour).
    out.time = (time.elapsed_secs_f64() % (200.0 * std::f64::consts::PI)) as f32;
    sc.ready = lib.ready();
    sc.flora_ready = sc.flora_want.is_none_or(|w| w.key == lib.flora_key);
    if sc.free_slots.is_empty() && sc.cells.is_empty() && sc.bcells.is_empty() {
        sc.reset();
    }
    if !sc.enabled || !lib.ready() {
        if sc.body.is_some() {
            sc.reset();
        }
        return;
    }
    sc.frame += 1;
    let lod = sc.lod;
    let range = lod.impostor_far as f64 + 50.0;
    // The body whose ground is nearest (it is drawn in linear space).
    let best = bodies
        .iter()
        // Bodies with land: not stars, nor giants (gas and ice giants, brown dwarfs: terrain.rs's test).
        .filter(|(_, b, t)| !b.star && !t.proc.kind.is_giant() && t.surf_gen.classes.bulk < se_core::surface_gen::bulk::NEPTUNE)
        .map(|(e, b, t)| (e, b, t, cam.pos.delta_m(b.pos).length() - t.radius))
        .filter(|x| x.3 < range * 4.0)
        .min_by(|a, b| a.3.total_cmp(&b.3));
    let Some((entity, body, terrain, _)) = best else {
        if sc.body.is_some() {
            sc.reset();
        }
        return;
    };
    let rel = cam.pos.delta_m(body.pos);
    let k = space.factor(rel.length());
    if (k - 1.0).abs() > 1e-9 || terrain.drawn_nodes().is_empty() {
        return;
    }
    if sc.body != Some(entity) {
        sc.reset();
        sc.body = Some(entity);
        sc.seed = (terrain.proc.seed as u32) ^ (terrain.radius as u32).rotate_left(7);
        sc.veg = veg_kind(sc.test, terrain);
        sc.flora_want = (sc.veg == VegKind::SeTerra).then(|| flora_want(body, terrain, data.as_deref()));
        info!("scatter: {:?} on the body at {:.0} km radius", sc.veg, terrain.radius / 1000.0);
    }
    if sc.veg == VegKind::None {
        return;
    }
    let r = terrain.radius;
    // The camera in the body frame, on the sphere the terrain is built on.
    let cam_body = body.from_shape(body.orient.inverse() * rel);
    let dir = cam_body.normalize();
    let ground = terrain.ground_height(dir).unwrap_or(0.0);
    let height = (cam_body.length() - r - ground).max(0.0);
    if sc.veg == VegKind::Map {
        // The colour map's plants around the camera; nothing until the first patch is made.
        let (Some(d), Some(layers)) = (data.as_deref(), terrain.tile_layers()) else { return };
        let Some(color) = layers.color.as_ref() else { return };
        sc.earth.update(&d.vfs, color, layers.water.as_ref(), layers.lights.as_ref(), r, dir, &mut images);
        if sc.earth.color.is_none() {
            return;
        }
    }
    // The wind the trees sway in: the derived weather's at the camera, near the ground.
    let sun_body = (body.orient.inverse() * terrain.body_light.sun_dir.truncate().as_dvec3()).normalize_or_zero();
    if sc.frame >= sc.wind_frame + 60 || sc.wind_frame == 0 {
        sc.wind_frame = sc.frame;
        sc.wind = data.as_deref().map_or(DVec3::ZERO, |d| crate::volumetric_clouds::wind_at(d, body, terrain, dir, sun_body, sim.jd));
    }
    let mut wanted = Vec::new();
    cells_in_range(dir, height, r, range, CELL_EDGE_M, &mut wanted);
    // Keep cells a little past the range (no thrashing at the edge).
    let mut keep: Vec<(TileKey, f64)> = Vec::new();
    cells_in_range(dir, height, r, range * 1.15, CELL_EDGE_M, &mut keep);
    // The grass's finer cells near the camera (not in the test).
    let mut grass_cells = HashSet::new();
    if sc.veg != VegKind::Test && lib.grass > 0 {
        let mut g = Vec::new();
        cells_in_range(dir, height, r, GRASS_RANGE_M as f64 + 10.0, GRASS_EDGE_M, &mut g);
        grass_cells.extend(g.iter().map(|c| c.0));
        wanted.extend(g);
        let mut gk = Vec::new();
        cells_in_range(dir, height, r, (GRASS_RANGE_M as f64 + 10.0) * 1.15, GRASS_EDGE_M, &mut gk);
        keep.extend(gk);
    }
    let keep: std::collections::HashSet<TileKey> = keep.into_iter().map(|c| c.0).collect();
    sc.cells.retain(|k, _| keep.contains(k));
    // The buildings' cells (a civilization's generated planet; not in the test).
    let mut bwanted: Vec<(TileKey, f64, u32)> = Vec::new();
    if buildings_on(&sc, &lib, terrain) {
        let (mut near, mut far, mut nk, mut fk) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        cells_in_range(dir, height, r, BUILD_RANGE_M, CELL_EDGE_M, &mut near);
        cells_in_range(dir, height, r, FAR_RANGE_M, FAR_EDGE_M, &mut far);
        bwanted.extend(near.into_iter().map(|(k, d)| (k, d, 1)));
        // Not the far cells wholly inside the near buildings' range (they would draw none there).
        bwanted.extend(far.into_iter().filter(|&(k, d)| d + node_edge(k, r) * 1.5 > BUILD_RANGE_M * 0.85).map(|(k, d)| (k, d, 2)));
        cells_in_range(dir, height, r, BUILD_RANGE_M * 1.15, CELL_EDGE_M, &mut nk);
        cells_in_range(dir, height, r, FAR_RANGE_M * 1.15, FAR_EDGE_M, &mut fk);
        let bk: HashSet<(TileKey, u32)> = nk.into_iter().map(|c| (c.0, 1)).chain(fk.into_iter().map(|c| (c.0, 2))).collect();
        sc.bcells.retain(|k, _| bk.contains(k));
    } else {
        sc.bcells.clear();
    }
    let used: std::collections::HashSet<u32> = sc.cells.values().chain(sc.bcells.values()).map(|c| c.slot).collect();
    sc.free_slots = (0..SLOTS).rev().filter(|s| !used.contains(s)).collect();
    // New cells, nearest first. Every resident cell's job goes to the render world, nearest first:
    // it places the slots it has not placed yet, a few a frame (`gpu.rs` `prepare`), so a job is
    // never lost while the passes are not ready.
    wanted.sort_by(|a, b| a.1.total_cmp(&b.1));
    let frame = sc.frame;
    let seed = sc.seed;
    for (key, _) in &wanted {
        if let Some(c) = sc.cells.get_mut(key) {
            c.used = frame;
        } else if let Some(slot) = sc.free_slots.pop() {
            let grass = grass_cells.contains(key);
            let per_tree = if grass {
                1.0 / GRASS_PER_M2
            } else if sc.veg == VegKind::Test {
                TEST_AREA_PER_TREE
            } else {
                FOREST_AREA_PER_TREE
            };
            let keep = (cell_area(*key, r) / per_tree / SLOT_CAP as f64).min(1.0) as f32;
            let models = if grass { lib.trees | (lib.grass << 16) } else { lib.trees << 16 };
            let job = PlaceJob {
                slot,
                face: key.face as u32,
                level: key.level as u32,
                row: key.row,
                col: key.col,
                keep,
                seed: cell_seed(*key, seed),
                models,
                ..default()
            };
            sc.cells.insert(*key, CellSlot { slot, used: frame, job });
        } else {
            continue;
        }
        out.jobs.push(sc.cells[key].job);
    }
    // The buildings' cells, nearest first, after the plants' (one building a lot: the layout's).
    bwanted.sort_by(|a, b| a.1.total_cmp(&b.1));
    for &(key, _, kind) in &bwanted {
        if let Some(c) = sc.bcells.get_mut(&(key, kind)) {
            c.used = frame;
        } else if let Some(slot) = sc.free_slots.pop() {
            let edge = r * std::f64::consts::FRAC_PI_2 / (1u64 << key.level) as f64;
            let (x0, y0) = (key.col as f64 * edge, key.row as f64 * edge);
            let tile = [(x0 / CITY_TILE_M).floor(), (y0 / CITY_TILE_M).floor()];
            let job = PlaceJob {
                slot,
                face: key.face as u32,
                level: key.level as u32,
                row: key.row,
                col: key.col,
                keep: 1.0,
                seed: cell_seed(key, seed ^ 0xb11d_0c75),
                models: lib.building | (1 << 16),
                kind,
                tile: [tile[0] as i32, tile[1] as i32],
                local: [(x0 - tile[0] * CITY_TILE_M) as f32, (y0 - tile[1] * CITY_TILE_M) as f32],
                edge: edge as f32,
            };
            sc.bcells.insert((key, kind), CellSlot { slot, used: frame, job });
        } else {
            continue;
        }
        out.jobs.push(sc.bcells[&(key, kind)].job);
    }
    // Cells kept past the range (not wanted this frame) last.
    for c in sc.cells.values().chain(sc.bcells.values()) {
        if c.used != frame {
            out.jobs.push(c.job);
        }
    }
    // The cells' trees of drawn nodes and their pages.
    let drawn = terrain.drawn_nodes();
    if sc.split_rev != Some(terrain.visible_rev) {
        sc.split.clear();
        for k in drawn {
            let mut a = *k;
            while let Some(p) = a.parent() {
                if !sc.split.insert(p) {
                    break;
                }
                a = p;
            }
        }
        sc.split_rev = Some(terrain.visible_rev);
    }
    let split = std::mem::take(&mut sc.split);
    let mut tree: Vec<u32> = Vec::with_capacity(4096);
    let mut cells = vec![CellGpu::default(); SLOTS as usize];
    let mut uploads = Vec::new();
    let resident: Vec<(TileKey, u32, u32)> = sc.cells.iter().map(|(k, c)| (*k, c.slot, c.job.seed)).chain(sc.bcells.iter().map(|(k, c)| (k.0, c.slot, c.job.seed))).collect();
    let mut max_slot = 0;
    let mut full = false;
    for (key, slot, seed) in resident {
        let sc_ref = &mut *sc;
        let mut page = |n: TileKey| {
            let p = sc_ref.page(n, terrain, &mut uploads);
            full |= p.is_none();
            p
        };
        // A cell whose tree no longer fits is left out this frame.
        let fits = tree.len() < TREE_MAX as usize;
        let (root, off, shift) = if fits { build_cell_tree(key, drawn, &split, &mut tree, &mut page) } else { (0, [0; 2], 0) };
        let live = if fits && tree.len() <= TREE_MAX as usize { CellGpu::LIVE } else { 0 };
        let c = key.center_dir();
        let up = (body.orient * (c / body.shape).normalize()).as_vec3();
        cells[slot as usize] = CellGpu { up: up.to_array(), root, off, shift_live: shift | live, seed };
        max_slot = max_slot.max(slot + 1);
    }
    sc.split = split;
    if full && !sc.warned_pages {
        warn!("scatter: the height atlas is full ({PAGES} pages): some trees are left out");
        sc.warned_pages = true;
    }
    cells.truncate(max_slot as usize);
    // The pages' frames this frame.
    let split_scale = terrain.split_scale();
    let mut pages = vec![PageGpu::default(); sc.pages.values().map(|p| p.page + 1).max().unwrap_or(0) as usize];
    let orient = body.orient;
    let inv = orient.inverse();
    for (key, p) in &sc.pages {
        if p.used != frame {
            continue;
        }
        let center = key.center_dir() * r;
        let node_pos = body.pos.offset_m(orient * body.to_shape(center));
        let t = node_pos.delta_m(cam.pos);
        let cam_local = body.from_shape(inv * (-t));
        let edge = r * std::f64::consts::FRAC_PI_2 / (1u64 << key.level) as f64;
        pages[p.page as usize] = PageGpu {
            t: t.as_vec3().to_array(),
            split_m: (edge as f32) * terrain::node_split(key.level, split_scale),
            cam: cam_local.as_vec3().to_array(),
            _pad: 0.0,
            dir: key.center_dir().as_vec3().to_array(),
            _pad2: 0.0,
        };
    }
    let m = DMat3Ext::from_quat_scale(orient, body.shape * k);
    if sc.debug && frame % 120 == 0 {
        info!(
            "scatter debug (cpu): height {height:.0} m, cells {} ({} jobs), pages {} of {PAGES}, tree {}, drawn nodes {} (levels {}..{})",
            sc.cells.len(),
            out.jobs.len(),
            sc.pages.values().filter(|p| p.used == frame).count(),
            tree.len(),
            drawn.len(),
            drawn.iter().map(|k| k.level).min().unwrap_or(0),
            drawn.iter().map(|k| k.level).max().unwrap_or(0)
        );
    }
    out.active = true;
    out.page_uploads = uploads;
    out.pages = Arc::new(pages);
    out.cells = Arc::new(cells);
    out.tree = Arc::new(tree);
    out.lod = lod;
    out.sink = SINK_M;
    out.body = BodyFrame {
        m,
        centre: body.pos.delta_m(cam.pos).as_vec3(),
        radius_m: r as f32,
        light: terrain.body_light,
        uniform: terrain.body_uniform.unwrap_or_default(),
        atmosphere: terrain.atmosphere.as_ref().map(|a| (a.inscatter.clone(), a.transmittance.clone(), a.irradiance.clone())),
        veg: sc.veg,
        climate: terrain.biome.as_ref().map_or([Vec4::ZERO; 8], |b| std::array::from_fn(|i| b.climate[i])),
        biome_flags: terrain.biome.as_ref().map_or(Vec4::ZERO, |b| b.flags),
        sun_body: sun_body.as_vec3(),
        wind: (body.orient * sc.wind).as_vec3(),
        forest: terrain.biome.as_ref().map_or(Vec4::ZERO, |b| b.color[11]),
        grass: terrain.biome.as_ref().map_or(Vec4::ZERO, |b| b.color[10]).truncate().extend(GRASS_RANGE_M),
        grass_models: UVec2::new(lib.trees, lib.grass),
        species: UVec4::new(lib.earth0, lib.flora0, sc.flora_want.is_some_and(|w| w.key == lib.flora_key) as u32, lib.building),
        relief_m: terrain.proc.relief_m as f32,
        // Matched to the ground with the trees off (2026-11-04, pixel means 0.7-3 km away): the
        // crowns' ball shading is about 1.5 times darker than level ground; Earth's ground also
        // brightens its colour map by its detail textures' brightness (about 1.6), SE's terra ground
        // draws its slot colours (its sample layers keep their mean at 1).
        tint_gain: if sc.veg == VegKind::Map { Vec3::new(2.6, 2.3, 3.5) } else { Vec3::new(1.7, 1.8, 1.8) },
        city: terrain.civ.map_or(Vec4::ZERO, |(level, seed)| {
            let c = se_core::procgen::civilization::culture(seed, level);
            Vec4::new(level as f32, (seed % 10_000) as f32 / 10_000.0, c.grid_turn, r as f32)
        }),
        city2: {
            let (lo, span) = terrain.ground_range_m();
            Vec4::new(BUILD_RANGE_M as f32, if city_test() { 1.0 } else { 0.0 }, lo, span)
        },
        patch: sc.earth.color.clone(),
        patch_lights: sc.earth.lights.clone(),
        patch_frame: {
            let (e1, e2) = earth::frame_axes(sc.earth.centre);
            [sc.earth.centre.as_vec3().extend(sc.earth.tan_a as f32), e1.as_vec3().extend(0.0), e2.as_vec3().extend(0.0)]
        },
    };
}

/// How the plants of `body` are decided: the test's even cover; for bodies with land plants, a
/// colour map's forests (Earth) or SE's climate bands on its generated worlds; else none.
/// The ground (terrain.rs, planet.wesl) asks the same for its forest floor and canopy.
pub fn veg_kind(test: bool, terrain: &Terrain) -> VegKind {
    if test {
        return VegKind::Test;
    }
    if !terrain.land_plants {
        return VegKind::None;
    }
    // A colour map: its plants come from its colours (the ground's `proc_params2.z` 0).
    let Some(layers) = terrain.tile_layers() else { return VegKind::None };
    if layers.color.is_some() {
        return VegKind::Map;
    }
    // SE's generator (climate[4].w), its terra colour (climate[7].z 0) and plants on (flags.z).
    let Some(b) = terrain.biome.as_ref() else { return VegKind::None };
    if b.climate[4].w > 0.5 && b.climate[7].z < 0.5 && b.flags.z > 0.5 {
        return VegKind::SeTerra;
    }
    VegKind::None
}

/// The flora a generated life planet wants: its seed, its surface gravity and its air's pressure
/// (the catalog's mass; the pressure SE's generator works with, else the catalog's `Atmosphere {
/// Pressure }`, else Earth's).
fn flora_want(body: &SystemBody, terrain: &Terrain, data: Option<&crate::data::SeData>) -> library::FloraWant {
    let rec = match (data, body.obj) {
        (Some(d), se_core::catalog::ObjectRef::Body(i)) => d.catalog.bodies.get(i as usize),
        _ => None,
    };
    let r = terrain.radius.max(1.0);
    let gravity = rec.map_or(9.81, |b| (se_core::units::G * b.mass_kg / (r * r)) as f32);
    let gravity = if gravity.is_finite() && gravity > 0.01 { gravity } else { 9.81 };
    let pressure = Some(terrain.pressure_atm)
        .filter(|p| *p > 0.0)
        .or_else(|| rec.and_then(|b| b.block.sub("Atmosphere").map(|a| se_core::body_params::AtmosphereParams::from_block(a).pressure_atm as f32)))
        .filter(|p| *p > 0.0)
        .unwrap_or(1.0);
    let seed = (terrain.proc.seed as u32) ^ (terrain.radius as u32).rotate_left(11);
    library::FloraWant { key: library::flora_key(seed, gravity, pressure), seed, gravity, pressure }
}

/// Body frame to render frame as an f32 matrix: rotation `q` after a per-axis scale `s`.
struct DMat3Ext;

impl DMat3Ext {
    fn from_quat_scale(q: DQuat, s: DVec3) -> Mat3 {
        let r = bevy::math::DMat3::from_quat(q);
        Mat3::from_cols((r.x_axis * s.x).as_vec3(), (r.y_axis * s.y).as_vec3(), (r.z_axis * s.z).as_vec3())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_400_to_800_m() {
        // Earth: at a face's centre, its edges and a corner.
        let r = 6_371_000.0;
        for (u, v) in [(0.5, 0.5), (0.01, 0.5), (0.002, 0.003)] {
            let mut key = TileKey::root(CubeFace::PosY);
            while !is_cell(key, r, CELL_EDGE_M) {
                let l = key.level + 1;
                let n = (1u64 << l) as f64;
                key = TileKey { face: key.face, level: l, row: (v * n) as u32, col: (u * n) as u32 };
            }
            let e = node_edge(key, r);
            assert!(e > 380.0 && e <= 800.0, "{e} at {u} {v}");
        }
    }

    #[test]
    fn cell_areas_cover_the_sphere() {
        let r = 1000.0;
        let total: f64 = CubeFace::ALL.iter().flat_map(|&f| TileKey::root(f).children()).map(|k| cell_area(k, r)).sum();
        assert!((total - 4.0 * std::f64::consts::PI * r * r).abs() / total < 1e-9);
    }

    #[test]
    fn cell_trees() {
        let cell = TileKey { face: CubeFace::PosX, level: 3, row: 2, col: 5 };
        let mut pages = 0u32;
        let mut page = |_k: TileKey| {
            pages += 1;
            Some(pages - 1)
        };
        // A coarser drawn node covers the cell.
        let drawn = vec![cell.ancestor_at(1)];
        let ancestors = |drawn: &[TileKey]| {
            let mut s = HashSet::new();
            for k in drawn {
                let mut a = *k;
                while let Some(p) = a.parent() {
                    s.insert(p);
                    a = p;
                }
            }
            s
        };
        let mut tree = Vec::new();
        let (root, off, shift) = build_cell_tree(cell, &drawn, &ancestors(&drawn), &mut tree, &mut page);
        assert_eq!((root, shift), (0, 2));
        assert_eq!(off, [5 - (cell.ancestor_at(1).col << 2), 2 - (cell.ancestor_at(1).row << 2)]);
        assert_eq!(tree, vec![LEAF]);
        // The cell split into its children, one of which split again.
        let mut drawn: Vec<TileKey> = cell.children().to_vec();
        let split = drawn.remove(3);
        drawn.extend(split.children());
        drawn.sort();
        let mut tree = Vec::new();
        let (root, _, shift) = build_cell_tree(cell, &drawn, &ancestors(&drawn), &mut tree, &mut page);
        assert_eq!((root, shift), (0, 0));
        assert_eq!(tree.len(), 1 + 4 + 4);
        assert!(tree[..5].iter().filter(|&&e| e & LEAF != 0).count() == 3);
        // Child 3 (row bit 1, column bit 1) is the split one.
        assert_eq!(tree[4], 5);
        let c3 = TileKey { face: cell.face, level: 4, row: cell.row * 2 + 1, col: cell.col * 2 + 1 };
        assert!(drawn.binary_search(&c3).is_err());
        // No drawn node over or under the cell (past the horizon): one leaf without a page.
        let far: Vec<TileKey> = vec![TileKey::root(CubeFace::NegZ)];
        let mut tree = Vec::new();
        build_cell_tree(cell, &far, &ancestors(&far), &mut tree, &mut page);
        assert_eq!(tree, vec![LEAF | NO_PAGE]);
    }
}
