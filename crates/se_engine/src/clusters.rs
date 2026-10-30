//! Member stars of nearby star clusters, generated in SE's cluster octree
//! (`seStarNodeCache::CreateClusterBlock`, FUN_14053a760; the stars in `se_core::cluster_stars`).
//!
//! Globular and nucleus (`Kern`) clusters (`FUN_140543c70` node counts, `FUN_140540220` node loading,
//! tables from `FUN_1401d7300`) have an octree of [`LEVELS`] levels below a root cube of half-size the
//! cluster radius. Level `d` holds the members of one of five equal-probability bands of Mv ~ N(3, 6)
//! between -8 and 30 ([`band_edges`]: -8, -1.5, 1.8, 4.7, 8.2, 25), so the root holds the few
//! brightest members and the smallest, deepest nodes the faint majority. A node tries SE's count of
//! uniform points in its cube and keeps those inside the radius with probability exp(-c r/R) (`c`
//! from `CenPow`); the root's first member is the cluster's black hole, which draws nothing. Open
//! clusters have only the root: a blob (mean squared radius 1/CenPow) of stars until their light adds
//! up to the cluster's. A node is loaded while the camera is inside its sphere (radius its half-size)
//! or the brightest star of its band, seen from the sphere's edge, would show above the star
//! magnitude limit. The root loads once the cluster spans 8 px on screen ([`members_range_pc`]).
//! Clusters with `NStars 0` (their stars are in the star catalog: the Pleiades, the Hyades) and `Part`
//! clusters get none, and SE's `ShowCatClusters` / `ShowProcClusters` switch the catalog / procedural
//! clusters off ([`ClusterSettings`]).
//!
//! Members are named as SE names them, `RSC <galaxy>-<cluster>-<level>-<node>-<index>` (FUN_1401c0180,
//! the 8-number form in procedural galaxies), and those names resolve in commands and the search box
//! ([`resolve_member_names`]).
//!
//! Each node is one mesh entity sharing the [`StarMaterial`] "octree mode" with `procstars`: the
//! entity translation is `(centre_pc - camera_pc)` every frame, vertices are offsets from the
//! cluster centre (universe frame, parsecs).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::catalog::{Catalog, ClusterRecord, ClusterType, ObjectRef};
use se_core::cluster_stars::{self as cs, ClassWord};
use se_core::proc_galsys::{GalaxyRef, MILKY_WAY_SE_INDEX, SystemKind};
use se_core::star::blackbody_rgb;
use se_core::star_octree::Rng;

use crate::camera::{CommandQueue, FlyCamera};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::procstars::{NearestProcStar, PROMOTE_PC, ProcStarInfo};
use crate::stars::{StarMaterial, StarSettings, build_sprite_mesh, push_star, uniforms_for};
use crate::universe::UniverseSet;

const MAX_IN_FLIGHT: usize = 8;
/// SE's cluster octree depth (its band tables have five rows).
pub const LEVELS: u8 = 5;
/// SE's most stars in one cluster node (`0x4000`).
pub const NODE_CAP: usize = cs::NODE_CAP;
/// SE draws a cluster's member stars once its radius spans this many pixels (times height/1080 above
/// 1080 rows; `FUN_14045ea50` writes 8 x max(1, h/1080), tested in `FUN_1404a09a0`/`FUN_140499f30`).
/// Its far particle fades out between 1 and 100 px meanwhile (`galaxy_far.wesl`).
const MEMBERS_PX: f64 = 8.0;
/// Hysteresis: loaded clusters and nodes are dropped beyond FAR_FACTOR times their load range.
const FAR_FACTOR: f64 = 1.25;
/// SE's luminosity of a cluster's central black hole (FUN_14053a760).
const BLACK_HOLE_LUM: f64 = 1000.0;

/// Distance (pc) within which a cluster of `radius_pc` spans [`MEMBERS_PX`] on a screen `height_px`
/// tall with a vertical field of view `fov_deg`.
pub fn members_range_pc(radius_pc: f64, fov_deg: f64, height_px: f64) -> f64 {
    let focal = height_px * 0.5 / (fov_deg.to_radians() * 0.5).tan();
    radius_pc * focal / (MEMBERS_PX * (height_px / 1080.0).max(1.0))
}

/// SE's magnitude bands of the octree levels (`FUN_1401d7300`): equal steps of probability of a
/// normal distribution of mean 3 and sigma 6 from Mv -8 to 30 (the top capped at p 0.9999).
pub fn band_edges() -> [f64; 6] {
    let cdf = |m: f64| 0.5 * (1.0 + se_core::surface_gen::erf((m - 3.0) / (6.0 * std::f64::consts::SQRT_2)));
    let (p0, p1) = (cdf(-8.0), cdf(30.0).min(0.9999));
    std::array::from_fn(|i| 3.0 + 6.0 * se_core::surface_gen::inv_norm_cdf(p0 + (p1 - p0) * i as f64 / 5.0))
}

/// One generated member star.
pub struct MemberStar {
    /// Offset from the cluster centre, universe frame, parsecs.
    pub offset: DVec3,
    /// Absolute V magnitude.
    pub abs_mag: f32,
    pub teff: f32,
    pub mass: f32,
    /// The cluster's age with SE's scatter (FUN_1401d6c60 / FUN_1401d63b0), Gyr.
    pub age_gyr: f32,
    pub feh: f32,
    pub class: ClassWord,
    /// Not drawn: the central black hole.
    pub hidden: bool,
}

/// A member's place in its cluster's octree, for its `RSC` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberId {
    pub cluster: usize,
    pub level: u8,
    /// SE's node number ([`cs::node_number`]).
    pub node: u64,
    pub index: u32,
}

/// A node of a cluster's octree: level and cell (0..2^level per axis).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NodeId {
    cluster: usize,
    level: u8,
    cell: [u16; 3],
}

impl NodeId {
    fn member(&self, index: usize) -> MemberId {
        MemberId { cluster: self.cluster, level: self.level, node: cs::node_number(self.level, self.cell), index: index as u32 }
    }
}

/// How a cluster's members are made (SE's cluster types, FUN_140483080).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// One root node filled by light (type 3).
    Open,
    /// The five-level octree with the exp(-c r/R) profile (types 2 globular, 1 nucleus).
    Dense { nucleus: bool },
}

/// A cluster's member model, shared by the builds of its nodes.
struct Model {
    name: String,
    radius: f64,
    /// V luminosity, solar.
    lum: f64,
    age: f64,
    shape: Shape,
    /// Open: the blob's mean squared radius 1/CenPow (cluster radii); dense: the concentration `c`.
    k: f64,
    /// SE's node count factor ([`cs::count_factor`]).
    factor: f64,
    edges: [f64; 6],
    /// The central black hole's mass, solar.
    black_hole: Option<f64>,
    /// The members' name prefix, `RSC <galaxy>-<cluster>`.
    rsc: String,
}

impl Model {
    fn levels(&self) -> u8 {
        match self.shape {
            Shape::Open => 1,
            Shape::Dense { .. } => LEVELS,
        }
    }
    fn node_half(&self, level: u8) -> f64 {
        self.radius / (1u32 << level) as f64
    }
    /// Node centre, offset from the cluster centre.
    fn node_centre(&self, id: NodeId) -> DVec3 {
        let s = 2.0 * self.node_half(id.level);
        DVec3::new(id.cell[0] as f64, id.cell[1] as f64, id.cell[2] as f64) * s + DVec3::splat(s * 0.5 - self.radius)
    }
}

type NodeBuilt = Option<(Mesh, Arc<Vec<MemberStar>>)>;

enum ModelSlot {
    Loading(Task<Option<Arc<Model>>>),
    /// `None`: nothing to draw (`NStars 0`, `Part`, or no members).
    Ready(Option<Arc<Model>>),
}

enum NodeSlot {
    Loading(Task<NodeBuilt>),
    /// Entity `None` when the node draws no stars.
    Ready(Option<Entity>, Arc<Vec<MemberStar>>),
}

/// SE's cluster display switches (main-def.cfg): `ShowCatClusters`; `ShowProcClusters` is
/// `ProcSettings::clusters`.
#[derive(Resource, Debug, Clone)]
pub struct ClusterSettings {
    pub show_cat: bool,
}

impl ClusterSettings {
    /// Whether cluster `rec` is shown: catalog clusters by `ShowCatClusters`, procedural (`RC`) ones by
    /// `ShowProcClusters` (FUN_140499ce0).
    pub fn shows(&self, rec: &ClusterRecord, show_proc: bool) -> bool {
        if rec.source == "procedural" { show_proc } else { self.show_cat }
    }
}

#[derive(Resource)]
pub struct ClusterField {
    material: Handle<StarMaterial>,
    /// Keyed by index into `catalog.clusters`.
    models: HashMap<usize, ModelSlot>,
    nodes: HashMap<NodeId, NodeSlot>,
    /// SE's `StarDensityInCluster` (main-def.cfg): scales every cluster's member count.
    density: f64,
    /// [`centre_names`], read once from the catalog stars.
    centres: Option<HashSet<String>>,
}

impl ClusterField {
    /// Member stars of the loaded nodes: (universe position in parsecs, star, its id).
    pub fn members<'a>(&'a self, cat: &'a Catalog) -> impl Iterator<Item = (DVec3, &'a MemberStar, MemberId)> + 'a {
        self.nodes.iter().flat_map(move |(id, slot)| {
            let stars: &[MemberStar] = match slot {
                NodeSlot::Ready(_, stars) => stars,
                _ => &[],
            };
            let centre = cat.clusters[id.cluster].pos_pc;
            stars.iter().enumerate().map(move |(k, m)| (centre + m.offset, m, id.member(k)))
        })
    }

    /// How many members of cluster `ci` are generated, if its model is loaded (SE counts the stars of
    /// the loaded blocks: "Fly inside to count stars" otherwise).
    pub fn member_count(&self, ci: usize) -> Option<usize> {
        matches!(self.models.get(&ci), Some(ModelSlot::Ready(_))).then(|| {
            self.nodes
                .iter()
                .filter(|(id, _)| id.cluster == ci)
                .map(|(_, s)| match s {
                    NodeSlot::Ready(_, stars) => stars.len(),
                    _ => 0,
                })
                .sum()
        })
    }

    /// Catalog-ready description of member `id`.
    pub fn member_info(&self, cat: &Catalog, id: MemberId, m: &MemberStar) -> ProcStarInfo {
        let rsc = match self.models.get(&id.cluster) {
            Some(ModelSlot::Ready(Some(model))) => model.rsc.clone(),
            _ => rsc_prefix(cat, id.cluster),
        };
        info_of(&rsc, id, m)
    }
}

fn info_of(rsc: &str, id: MemberId, m: &MemberStar) -> ProcStarInfo {
    ProcStarInfo {
        name: format!("{rsc}-{}-{}-{}", id.level, id.node, id.index),
        class: m.class.name(),
        abs_mag: m.abs_mag,
        teff: m.teff,
        age_feh: Some((m.age_gyr, m.feh)),
    }
}

/// `RSC <galaxy>-<cluster>`, SE's member name prefix of catalog cluster `ci` (FUN_1401c0180): a
/// procedural cluster's `RC` numbers, else the galaxy's catalog index (8514 for the Milky Way) and the
/// cluster's index among the galaxy's catalog clusters (they come before the procedural ones).
pub fn rsc_prefix(cat: &Catalog, ci: usize) -> String {
    let rec = &cat.clusters[ci];
    if let Some(rest) = rec.names.first().and_then(|n| n.strip_prefix("RC ")) {
        return format!("RSC {rest}");
    }
    let index = cat.clusters[..ci].iter().filter(|c| c.galaxy == rec.galaxy && c.source != "procedural").count() as u32;
    let rc = se_core::proc_galsys::system_name(SystemKind::Cluster, GalaxyRef::Catalog(galaxy_code(cat, &rec.galaxy)), index);
    format!("RSC {}", &rc[3..])
}

/// The catalog index SE names a galaxy's clusters by: 8514 for the Milky Way, else the port's index.
fn galaxy_code(cat: &Catalog, galaxy: &str) -> u32 {
    match cat.find(galaxy) {
        _ if galaxy == "Milky Way" => MILKY_WAY_SE_INDEX,
        Some(ObjectRef::Galaxy(g)) => g,
        _ => 0,
    }
}

/// The catalog cluster an `RC` name stands for: a promoted procedural cluster of that name, or the
/// galaxy's catalog cluster of that index.
pub fn cluster_of_rc(cat: &Catalog, rc: &str) -> Option<usize> {
    if let Some(ObjectRef::Cluster(i)) = cat.find(rc) {
        return Some(i as usize);
    }
    let (_, GalaxyRef::Catalog(g), index) = se_core::proc_galsys::parse_system_name(rc)? else { return None };
    cat.clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| c.source != "procedural" && galaxy_code(cat, &c.galaxy) == g)
        .nth(index as usize)
        .map(|(i, _)| i)
}

#[derive(Component)]
struct ClusterNode {
    /// Cluster centre, universe frame, parsecs.
    centre_pc: DVec3,
}

pub struct ClustersPlugin;

impl Plugin for ClustersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_clusters).add_systems(PreUpdate, resolve_member_names).add_systems(
            PostUpdate,
            (update_clusters, position_clusters, nearest_member)
                .chain()
                .after(UniverseSet::Camera)
                .after(crate::procstars::ProcStarSet)
                .before(UniverseSet::FloatingOrigin),
        );
    }
}

fn setup_clusters(
    mut commands: Commands,
    settings: Res<StarSettings>,
    cfg: Res<EngineConfig>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    let mut u = uniforms_for(Vec3::ZERO, &settings);
    u.anchor = Vec4::new(0.0, 0.0, 0.0, 2.0);
    let material = materials.add(StarMaterial { uniforms: u });
    let main_def = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).map(|t| se_core::sc::parse(&t)).ok();
    let key = |k: &str| main_def.as_ref().and_then(|d| d.find_deep(k));
    let density = key("StarDensityInCluster").and_then(|i| i.f64()).unwrap_or(1.0).max(0.0);
    let show_cat = key("ShowCatClusters").and_then(|i| i.bool()).unwrap_or(true);
    commands.insert_resource(ClusterSettings { show_cat });
    commands.insert_resource(ClusterField { material, models: HashMap::new(), nodes: HashMap::new(), density, centres: None });
}

/// The magnitude limits cluster stars load to: `base`, and `zoomed` (raised by the zoom) for the
/// nodes inside `cone` (the view, when narrower than `StarFrustumClipFOV`) or everywhere without one.
struct Limits {
    base: f64,
    zoomed: f64,
    cone: Option<(DVec3, f64)>,
}

/// Whether node `id` of `model` (cluster centre `centre`) is to be loaded for a camera at `cam`:
/// inside its sphere, or its band's brightest star at the sphere's edge brighter than the limit
/// (apparent magnitude). `slack` > 1 widens the range (hysteresis for loaded nodes).
fn node_wanted(model: &Model, centre: DVec3, id: NodeId, cam: DVec3, lim: &Limits, slack: f64) -> bool {
    let r = model.node_half(id.level);
    let c = centre + model.node_centre(id);
    let d = (c - cam).length() / slack;
    let seen = lim.cone.is_none_or(|k| se_core::star_octree::sphere_meets_cone(cam, k, c, r * 3f64.sqrt()));
    let limit = if seen { lim.zoomed } else { lim.base };
    d <= r || se_core::star::app_mag_from_abs(model.edges[id.level as usize], (d - r).max(1e-6)) < limit
}

/// What a model build needs from the catalog.
struct ModelInput {
    rec: ClusterRecord,
    galaxy_radius_pc: f64,
    rsc: String,
    /// A catalog star is the cluster's centre: no black hole is generated (FUN_140471df0).
    has_centre_star: bool,
}

/// The lower-cased objects catalog stars are `CenterOf` (galaxies' central black holes, BlackHoles.sc).
fn centre_names(cat: &Catalog) -> HashSet<String> {
    cat.stars.iter().filter(|s| s.source != "procedural").filter_map(|s| s.block.str("CenterOf")).map(str::to_lowercase).collect()
}

fn model_input(cat: &Catalog, ci: usize, centres: &HashSet<String>) -> ModelInput {
    let rec = cat.clusters[ci].clone();
    let galaxy_radius_pc = match cat.find(&rec.galaxy) {
        Some(ObjectRef::Galaxy(g)) => cat.galaxies[g as usize].radius_pc,
        _ => 0.0,
    };
    // A star `CenterOf` the cluster, or of the galaxy for its nucleus (Sgr A* of the Milky Way's).
    let nucleus = ClusterType::parse(&rec.kind) == ClusterType::Kern;
    let has_centre_star =
        rec.names.iter().any(|n| centres.contains(&n.to_lowercase())) || (nucleus && centres.contains(&rec.galaxy.to_lowercase()));
    ModelInput { rsc: rsc_prefix(cat, ci), rec, galaxy_radius_pc, has_centre_star }
}

#[allow(clippy::too_many_arguments)]
fn update_clusters(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<StarSettings>,
    filter: Res<ClusterSettings>,
    proc_settings: Res<crate::procgalaxies::ProcSettings>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    field: Option<ResMut<ClusterField>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(mut field) = field else { return };
    let field = &mut *field;
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map_or(900.0, |w| w.resolution.physical_height() as f64);
    let fov = cam.fov_deg as f64;
    let view_range = |rec: &ClusterRecord| members_range_pc(rec.radius_pc, fov, height);
    let shown = |rec: &ClusterRecord| filter.shows(rec, proc_settings.clusters);
    let is_far = |rec: &ClusterRecord, cam_pc: DVec3| !shown(rec) || (rec.pos_pc - cam_pc).length() > view_range(rec) * FAR_FACTOR;
    // Stars show down to about the magnitude limit plus the point overbright; zoomed in, fainter
    // ones too, inside the view (SE raises its cluster coefficient by the zoom, FUN_1404c51a0).
    let limits = Limits {
        base: (settings.faint_limit() as f64).min(settings.zoomed_load_limit() as f64),
        zoomed: settings.zoomed_load_limit() as f64,
        cone: settings.view_cone(&cam, crate::stars::window_aspect(&windows)),
    };
    // The live catalog: clusters added at runtime (procedural `RC` clusters) are included.
    let cat = data.catalog.clone();
    let material = field.material.clone();

    // 1. Cluster models: finish, request inside the view range, drop (with their nodes) outside.
    for slot in field.models.values_mut() {
        if let ModelSlot::Loading(task) = slot {
            if let Some(model) = check_ready(task) {
                *slot = ModelSlot::Ready(model);
            }
        }
    }
    let centres = field.centres.get_or_insert_with(|| centre_names(&cat));
    let mut in_flight = field.models.values().filter(|s| matches!(s, ModelSlot::Loading(_))).count()
        + field.nodes.values().filter(|s| matches!(s, NodeSlot::Loading(_))).count();
    for (i, rec) in cat.clusters.iter().enumerate() {
        if in_flight >= MAX_IN_FLIGHT {
            break;
        }
        if field.models.contains_key(&i) || !shown(rec) || (rec.pos_pc - cam_pc).length() > view_range(rec) {
            continue;
        }
        let (input, density) = (model_input(&cat, i, centres), field.density);
        let task = AsyncComputeTaskPool::get().spawn(async move { build_model(&input, density).map(Arc::new) });
        field.models.insert(i, ModelSlot::Loading(task));
        in_flight += 1;
    }
    field.models.retain(|&i, _| !is_far(&cat.clusters[i], cam_pc));

    // 2. The nodes each loaded cluster wants: down the octree from the root while nodes load.
    let mut wanted: Vec<(NodeId, Arc<Model>)> = Vec::new();
    for (&ci, slot) in &field.models {
        let ModelSlot::Ready(Some(model)) = slot else { continue };
        let centre = cat.clusters[ci].pos_pc;
        let mut stack = vec![NodeId { cluster: ci, level: 0, cell: [0; 3] }];
        while let Some(id) = stack.pop() {
            wanted.push((id, model.clone()));
            if id.level + 1 >= model.levels() {
                continue;
            }
            for k in 0..8u16 {
                let child = NodeId {
                    cluster: ci,
                    level: id.level + 1,
                    cell: [id.cell[0] * 2 + (k & 1), id.cell[1] * 2 + ((k >> 1) & 1), id.cell[2] * 2 + (k >> 2)],
                };
                let slack = if field.nodes.contains_key(&child) { FAR_FACTOR } else { 1.0 };
                if node_wanted(model, centre, child, cam_pc, &limits, slack) {
                    stack.push(child);
                }
            }
        }
    }
    // Shallow (bright) nodes first.
    wanted.sort_by_key(|(id, _)| id.level);
    let keep: HashSet<NodeId> = wanted.iter().map(|(id, _)| *id).collect();

    // 3. Finished node builds become entities.
    let mut done = Vec::new();
    for (id, slot) in field.nodes.iter_mut() {
        if let NodeSlot::Loading(task) = slot {
            if let Some(built) = check_ready(task) {
                done.push((*id, built));
            }
        }
    }
    for (id, built) in done {
        let (mesh, stars) = match built {
            Some((m, s)) => (Some(m), s),
            None => (None, Arc::new(Vec::new())),
        };
        let entity = mesh.filter(|_| keep.contains(&id)).map(|m| {
            commands
                .spawn((
                    Mesh3d(meshes.add(m)),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    Visibility::Visible,
                    NoFrustumCulling,
                    ClusterNode { centre_pc: cat.clusters[id.cluster].pos_pc },
                ))
                .id()
        });
        field.nodes.insert(id, NodeSlot::Ready(entity, stars));
    }

    // 4. Request the wanted nodes not there yet.
    for (id, model) in wanted {
        if in_flight >= MAX_IN_FLIGHT {
            break;
        }
        if field.nodes.contains_key(&id) {
            continue;
        }
        let task = AsyncComputeTaskPool::get().spawn(async move { build_node(&model, id) });
        field.nodes.insert(id, NodeSlot::Loading(task));
        in_flight += 1;
    }

    // 5. Drop the nodes no longer wanted (a dropped task is cancelled).
    let stale: Vec<NodeId> = field.nodes.keys().filter(|id| !keep.contains(id)).copied().collect();
    for id in stale {
        if let Some(NodeSlot::Ready(Some(e), _)) = field.nodes.remove(&id) {
            commands.entity(e).despawn();
        }
    }
}

fn position_clusters(cam: Res<FlyCamera>, mut q: Query<(&ClusterNode, &mut Transform)>) {
    let cam_pc = cam.pos.to_parsecs();
    for (node, mut tf) in &mut q {
        tf.translation = (node.centre_pc - cam_pc).as_vec3();
    }
}

/// A member star nearer than the nearest Milky Way field star takes its place as the
/// candidate that `select_active_system` turns into a real star with planets.
fn nearest_member(cam: Res<FlyCamera>, data: Res<SeData>, field: Option<Res<ClusterField>>, mut nearest: ResMut<NearestProcStar>) {
    let Some(field) = field else { return };
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    // Only stars close enough to become the active system matter: scan the loaded nodes within
    // that reach, nearest first, and stop at the first one that cannot win.
    let reach = PROMOTE_PC.min(nearest.dist_pc);
    let mut near: Vec<(f64, NodeId, &[MemberStar])> = field
        .nodes
        .iter()
        .filter_map(|(&id, slot)| {
            let NodeSlot::Ready(_, stars) = slot else { return None };
            let ModelSlot::Ready(Some(model)) = field.models.get(&id.cluster)? else { return None };
            let c = cat.clusters[id.cluster].pos_pc + model.node_centre(id);
            let half = model.node_half(id.level);
            let gap = ((c - cam_pc).abs() - DVec3::splat(half)).max(DVec3::ZERO).length();
            (gap < reach && !stars.is_empty()).then_some((gap, id, stars.as_slice()))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut best: Option<(f64, DVec3, &MemberStar, MemberId)> = None;
    for (gap, id, stars) in near {
        if best.is_some_and(|b| gap * gap > b.0) {
            break;
        }
        let centre = cat.clusters[id.cluster].pos_pc;
        for (k, m) in stars.iter().enumerate() {
            let p = centre + m.offset;
            let d2 = (p - cam_pc).length_squared();
            if best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, p, m, id.member(k)));
            }
        }
    }
    if let Some((d2, p, m, id)) = best {
        if d2.sqrt() < nearest.dist_pc {
            nearest.star = Some(field.member_info(cat, id, m));
            nearest.pos_pc = p;
            nearest.dist_pc = d2.sqrt();
        }
    }
}

/// The member name inside a command line: `goto RSC 8514-12-0-0-3 0.01` -> `RSC 8514-12-0-0-3`.
fn member_name_in_command(cmd: &str) -> Option<&str> {
    let (verb, arg) = cmd.trim().split_once(' ')?;
    if !matches!(verb.to_ascii_lowercase().as_str(), "goto" | "select" | "look") {
        return None;
    }
    let arg = arg.trim();
    let name = match arg.rsplit_once(' ') {
        Some((n, d)) if d.parse::<f64>().is_ok() => n.trim(),
        _ => arg,
    };
    cs::parse_member_name(name).is_some().then_some(name)
}

/// Frames the search box's text must stay unchanged before an `RSC` name in it is resolved (so a
/// name being typed does not add every prefix's member).
const SEARCH_SETTLE_FRAMES: u32 = 30;

/// State of [`resolve_member_names`].
#[derive(Default)]
struct MemberResolver {
    /// Lower-cased names already resolved or given up on.
    tried: HashSet<String>,
    /// Frames a scripted command has waited for its procedural cluster.
    held: u32,
    /// The search box's text and for how many frames it has stayed so.
    search: (String, u32),
    /// Member builds in flight: (lower-cased name, the member's catalog record).
    tasks: Vec<(String, Task<Option<se_core::catalog::StarRecord>>)>,
}

/// Turns `RSC ...` member names mentioned by a command or the search box (once it stops changing)
/// into catalog stars, so `goto`, selection and the search box work on them: the member's node is
/// generated in the background. A procedural `RC` cluster is first made a catalog cluster by
/// `procsystems` (the Milky Way's once its emission map exists); a scripted command waits for both.
fn resolve_member_names(
    mut data: ResMut<SeData>,
    hud: Option<Res<crate::hud::HudState>>,
    mut queue: ResMut<CommandQueue>,
    field: Option<Res<ClusterField>>,
    mut st: Local<MemberResolver>,
) {
    let Some(field) = field else { return };
    let st = &mut *st;
    // Finished builds become catalog stars.
    let mut i = 0;
    while i < st.tasks.len() {
        let Some(rec) = check_ready(&mut st.tasks[i].1) else {
            i += 1;
            continue;
        };
        // The task is finished: dropping it cancels nothing.
        drop(st.tasks.swap_remove(i));
        if let Some(rec) = rec.filter(|r| data.catalog.find(&r.names[0]).is_none()) {
            let name = rec.names[0].clone();
            let idx = crate::system::promote_star(data.catalog_mut(), rec);
            info!("{name} -> catalog star {idx}");
        }
    }
    // (name, from a scripted command)
    let mut names: Vec<(String, bool)> = Vec::new();
    if let Some(n) = queue.queue.front().and_then(|c| member_name_in_command(c)) {
        names.push((n.to_string(), true));
    }
    if let Some(h) = hud.as_ref() {
        let text = h.search.trim();
        if text == st.search.0 {
            st.search.1 = st.search.1.saturating_add(1);
        } else {
            st.search = (text.to_string(), 0);
        }
        if st.search.1 == SEARCH_SETTLE_FRAMES && cs::parse_member_name(text).is_some() {
            names.push((text.to_string(), false));
        }
    }
    let mut hold = false;
    for (name, scripted) in names {
        let key = name.to_ascii_lowercase();
        if data.catalog.find(&name).is_some() {
            continue;
        }
        if st.tasks.iter().any(|(k, _)| *k == key) {
            hold |= scripted;
            continue;
        }
        if st.tried.contains(&key) {
            continue;
        }
        let Some((rc, level, node, index)) = cs::parse_member_name(&name) else { continue };
        let Some(ci) = cluster_of_rc(&data.catalog, &rc) else {
            if scripted && st.held < 3000 {
                hold = true;
            } else if scripted {
                st.tried.insert(key);
                warn!("star cluster of {name} not found");
            }
            continue;
        };
        st.tried.insert(key.clone());
        let centres = field.centres.clone().unwrap_or_else(|| centre_names(&data.catalog));
        let (input, density, pos_pc) = (model_input(&data.catalog, ci, &centres), field.density, data.catalog.clusters[ci].pos_pc);
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let model = build_model(&input, density)?;
            let cell = cs::node_cell(level, node).filter(|_| level < model.levels())?;
            let id = NodeId { cluster: ci, level, cell };
            let (_, stars) = build_node(&model, id)?;
            let Some(m) = stars.get(index as usize) else {
                warn!("{name}: the node has {} stars", stars.len());
                return None;
            };
            Some(member_record(&info_of(&model.rsc, id.member(index as usize), m), pos_pc + m.offset, m))
        });
        st.tasks.push((key, task));
        hold |= scripted;
    }
    if hold {
        st.held += 1;
        queue.wait_frames = queue.wait_frames.max(2);
    } else {
        st.held = 0;
    }
}

/// The catalog record of member `m`: the procedural star's, with SE's mass and the member's age and
/// metallicity (`Age`, `FeH`; SE copies the cluster's age into its members, FUN_14053a760).
pub fn member_record(info: &ProcStarInfo, pos_pc: DVec3, m: &MemberStar) -> se_core::catalog::StarRecord {
    use se_core::sc::{Item, Value};
    let mut rec = crate::system::procedural_star_record(info, pos_pc);
    if m.mass > 0.0 {
        rec.mass_kg = m.mass as f64 * se_core::units::SOLAR_MASS_KG;
        // A remnant's radius follows its mass (FUN_14053a760, FUN_1401d5fe0).
        if let Some(r) = se_core::procgen::se_stars::remnant_radius_km(rec.class.kind, m.mass as f64) {
            rec.radius_km = r;
        }
    }
    for (key, v) in [("Age", m.age_gyr), ("FeH", m.feh)] {
        rec.block.items.push(Item { key: key.into(), args: vec![Value::Num(v as f64)], ..Default::default() });
    }
    rec
}

fn seed_of(name: &str) -> u64 {
    // FNV-1a.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The cluster's member model, or `None` when it has none: `NStars 0` (its stars are catalog
/// stars), a `Part` cluster (FUN_140543c70 gives it no stars) or an unknown type.
fn build_model(input: &ModelInput, density: f64) -> Option<Model> {
    let rec = &input.rec;
    if rec.no_members {
        return None;
    }
    let shape = match ClusterType::parse(&rec.kind) {
        ClusterType::Open => Shape::Open,
        ClusterType::Globular => Shape::Dense { nucleus: false },
        ClusterType::Kern => Shape::Dense { nucleus: true },
        ClusterType::Part | ClusterType::Other => return None,
    };
    let name = rec.names.first().cloned().unwrap_or_default();
    let lum = se_core::star::lum_from_abs_mag(rec.abs_mag);
    let cen_pow = rec.cen_pow.max(1e-3);
    let mut rng = Rng(seed_of(&name) ^ 0x5bd1_e995);
    let (k, black_hole) = match shape {
        Shape::Open => (1.0 / cen_pow, None),
        Shape::Dense { nucleus } => {
            let hole = cs::central_black_hole(&mut rng, nucleus, rec.age_gyr, input.galaxy_radius_pc);
            (cs::concentration(cen_pow), hole.filter(|_| !input.has_centre_star))
        }
    };
    Some(Model {
        name,
        radius: rec.radius_pc.max(0.05),
        lum,
        age: rec.age_gyr,
        shape,
        k,
        factor: cs::count_factor(lum, density),
        edges: band_edges(),
        black_hole,
        rsc: input.rsc.clone(),
    })
}

/// The stars of one node (and their mesh), as SE's CreateClusterBlock makes them.
fn build_node(model: &Model, id: NodeId) -> NodeBuilt {
    let mut h = seed_of(&model.name);
    for v in [id.level as u64, id.cell[0] as u64, id.cell[1] as u64, id.cell[2] as u64] {
        h = (h ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    }
    let mut rng = Rng(h);
    let r = model.radius;
    let mut stars: Vec<MemberStar> = Vec::new();
    let star = |offset: DVec3, m: cs::Member| MemberStar {
        offset,
        abs_mag: m.abs_mag,
        teff: m.teff,
        mass: m.mass,
        age_gyr: m.age_gyr,
        feh: m.feh,
        class: m.class,
        hidden: false,
    };
    match model.shape {
        Shape::Open => {
            // The blob's stars, each at most half the cluster's light, until they add up to it.
            let mut sum = 0.0;
            let mut tries = 0usize;
            while stars.len() < NODE_CAP && sum < model.lum && tries < 4 * NODE_CAP * 64 {
                tries += 1;
                let p = cs::blob_point(&mut rng, model.k);
                if p[0] * p[0] + p[1] * p[1] + p[2] * p[2] > 1.0 {
                    continue;
                }
                let m = cs::open_member(&mut rng, model.age);
                let l = se_core::star::lum_from_abs_mag(m.abs_mag as f64);
                if l > model.lum * 0.5 {
                    continue;
                }
                sum += l;
                stars.push(star(DVec3::from_array(p) * r, m));
            }
        }
        Shape::Dense { nucleus } => {
            if id.level == 0 {
                if let Some(mass) = model.black_hole {
                    let mv = se_core::star::abs_mag_from_lum(BLACK_HOLE_LUM) as f32;
                    let m = cs::Member { abs_mag: mv, teff: 0.0, mass: mass as f32, age_gyr: model.age as f32, feh: 0.0, class: cs::black_hole_class() };
                    stars.push(MemberStar { hidden: true, ..star(DVec3::ZERO, m) });
                }
            }
            // Uniform points of the node's cube inside the radius, kept by the profile.
            let half = model.node_half(id.level);
            let centre = model.node_centre(id);
            let (lo, hi) = (model.edges[id.level as usize], model.edges[id.level as usize + 1]);
            let leaf = (id.level + 1 == LEVELS).then_some(nucleus);
            for _ in 0..cs::node_tries(id.level, model.factor, leaf) {
                if stars.len() >= NODE_CAP {
                    break;
                }
                let p = centre + DVec3::new(1.0 - 2.0 * rng.f64(), 1.0 - 2.0 * rng.f64(), 1.0 - 2.0 * rng.f64()) * half;
                let x2 = p.length_squared() / (r * r);
                if x2 > 1.0 || rng.f64() > (-model.k * x2.sqrt()).exp() {
                    continue;
                }
                let m = match leaf {
                    Some(_) => cs::leaf_member(&mut rng, nucleus, model.age, lo, hi),
                    None => Some(cs::dense_member(&mut rng, nucleus, model.age, lo, hi)),
                };
                if let Some(m) = m {
                    stars.push(star(p, m));
                }
            }
        }
    }
    if stars.is_empty() {
        return None;
    }
    let n = stars.len();
    let (mut p, mut d, mut c, mut ix) = (Vec::with_capacity(n * 4), Vec::with_capacity(n * 4), Vec::with_capacity(n * 4), Vec::with_capacity(n * 6));
    for s in stars.iter().filter(|s| !s.hidden) {
        push_star(&mut p, &mut d, &mut c, &mut ix, s.offset.as_vec3().to_array(), blackbody_rgb(s.teff as f64), s.abs_mag);
    }
    Some((build_sprite_mesh(p, d, c, ix), Arc::new(stars)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn se_band_edges() {
        let e = band_edges();
        let want = [-8.0, -1.49, 1.79, 4.73, 8.19, 25.3];
        for (a, b) in e.iter().zip(want) {
            assert!((a - b).abs() < 0.05, "{e:?}");
        }
    }

    fn record(kind: &str, abs_mag: f64, radius: f64, cen_pow: f64, age: f64) -> ClusterRecord {
        ClusterRecord {
            names: vec!["Test".into()],
            galaxy: "Milky Way".into(),
            kind: kind.into(),
            pos_pc: DVec3::ZERO,
            radius_pc: radius,
            abs_mag,
            age_gyr: age,
            cen_pow,
            no_members: false,
            color: None,
            source: "test".into(),
        }
    }

    fn model(rec: ClusterRecord) -> Option<Model> {
        build_model(&ModelInput { rec, galaxy_radius_pc: 15_000.0, rsc: "RSC 8514-0".into(), has_centre_star: false }, 1.0)
    }

    #[test]
    fn open_cluster_fills_its_light() {
        // Pozzo 1: 114 L_sun in the root, each member at most half of it.
        let m = model(record("Open", 4.83 - 2.5 * 114.2f64.log10(), 0.88, 1.5, 0.15)).unwrap();
        assert_eq!(m.levels(), 1);
        let (_, stars) = build_node(&m, NodeId { cluster: 0, level: 0, cell: [0; 3] }).unwrap();
        let sum: f64 = stars.iter().map(|s| se_core::star::lum_from_abs_mag(s.abs_mag as f64)).sum();
        assert!(sum >= 114.2 && sum < 114.2 * 1.5, "{sum} from {}", stars.len());
        assert!(stars.iter().all(|s| s.offset.length() <= 0.88 + 1e-9));
        assert!(model(ClusterRecord { no_members: true, ..record("Open", -3.6, 2.3, 0.5, 0.13) }).is_none());
        assert!(model(record("Part", -5.0, 300.0, 1.5, 0.1)).is_none());
    }

    #[test]
    fn globular_follows_se_profile() {
        // Omega Cen: the root's 5e5 tries keep about pi / c^3 of themselves, centrally concentrated,
        // after the central black hole.
        let m = model(record("Globular", -10.29, 43.96, 1.61, 12.0)).unwrap();
        let (_, stars) = build_node(&m, NodeId { cluster: 0, level: 0, cell: [0; 3] }).unwrap();
        assert!(stars[0].hidden && stars[0].class.kind() == se_core::star::StarKind::BlackHole);
        let n = stars.len() as f64;
        let expect = 5e5 * std::f64::consts::PI / m.k.powi(3);
        assert!((n / expect - 1.0).abs() < 0.2, "{n} vs {expect}");
        // exp(-c r/R) with c = 9.26: P(c r/R < 2.3) of a Gamma(3) is 0.41 inside a quarter radius.
        let inner = stars.iter().filter(|s| s.offset.length() < 43.96 * 0.25).count() as f64;
        assert!((inner / n - 0.41).abs() < 0.06, "{inner} of {n}");
        // Its brightest band is giants.
        assert!(stars[1..].iter().all(|s| s.class.lum() <= 6));
        let id = NodeId { cluster: 0, level: 0, cell: [0; 3] }.member(3);
        assert_eq!(info_of(&m.rsc, id, &stars[3]).name, "RSC 8514-0-0-0-3");
    }

    #[test]
    fn show_switches() {
        let s = ClusterSettings { show_cat: false };
        let cat = record("Open", -3.0, 2.0, 1.5, 0.1);
        let proc = ClusterRecord { source: "procedural".into(), ..cat.clone() };
        assert!(!s.shows(&cat, true) && s.shows(&proc, true) && !s.shows(&proc, false));
        // A nucleus is a dense cluster with its own black hole unless a catalog star is its centre.
        let k = |centre| build_model(&ModelInput { rec: record("Kern", -10.0, 3.0, 2.0, 0.1), galaxy_radius_pc: 15_000.0, rsc: String::new(), has_centre_star: centre }, 1.0);
        assert!(k(false).unwrap().black_hole.is_some() && k(true).unwrap().black_hole.is_none());
    }

    #[test]
    fn member_names_in_commands() {
        assert_eq!(member_name_in_command("goto RSC 8514-12-0-0-3 0.01"), Some("RSC 8514-12-0-0-3"));
        assert_eq!(member_name_in_command("select RSC 8514-12-2-63-10"), Some("RSC 8514-12-2-63-10"));
        assert_eq!(member_name_in_command("goto RC 8514-12"), None);
    }
}
