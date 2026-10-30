//! Member stars of nearby star clusters, generated procedurally in SE's cluster octree.
//!
//! SE (`FUN_140543c70` node counts, `FUN_140540220` node loading, tables from `FUN_1401d7300`)
//! gives each cluster an octree of [`LEVELS`] levels below a root cube of half-size the cluster
//! radius. Level `d` holds the members of one of five equal-probability bands of Mv ~ N(3, 6)
//! between -8 and 30 ([`band_edges`]: -8, -1.5, 1.8, 4.7, 8.2, 25), so the root holds the few
//! brightest members and the smallest, deepest nodes the faint majority. A node is loaded while the
//! camera is inside its sphere (radius its half-size) or the brightest star of its band, seen from
//! the sphere's edge, would show above the star magnitude limit; SE caps a node at [`NODE_CAP`]
//! stars. So faint members appear as you approach, and only around you. The root loads once the
//! cluster spans 8 px on screen ([`members_range_pc`]). Clusters the catalog already resolves star by
//! star (>= 15 catalog stars inside the radius) are skipped.
//!
//! How many members a band has comes from the port's population model (SE's per-node table needs
//! its star generator, CreateClusterBlock, which is not decoded): a Salpeter mass function
//! truncated at the turn-off mass for the age, plus a few red giants for old clusters, scaled to
//! `n_stars` or to the integrated absolute magnitude, times SE's `StarDensityInCluster`. Stars
//! follow a Plummer profile. A capped node's stars are brightened by the ratio, so its light stays.
//!
//! Each node is one mesh entity sharing the [`StarMaterial`] "octree mode" with `procstars`: the
//! entity translation is `(centre_pc - camera_pc)` every frame, vertices are offsets from the
//! cluster centre (universe frame, parsecs).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::catalog::{Catalog, ClusterRecord};
use se_core::star::{blackbody_rgb, main_sequence_abs_mag};
use se_core::star_octree::Rng;
use se_core::star_octree::ms_class_name;

use crate::camera::FlyCamera;
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::procstars::{NearestProcStar, PROMOTE_PC, ProcStarInfo};
use crate::stars::{StarMaterial, StarSettings, build_sprite_mesh, push_star, uniforms_for};
use crate::universe::UniverseSet;

const MAX_IN_FLIGHT: usize = 8;
/// SE's cluster octree depth (its band tables have five rows).
pub const LEVELS: u8 = 5;
/// SE's most stars in one cluster node (`0x4000`).
pub const NODE_CAP: usize = 16_384;
/// SE draws a cluster's member stars once its radius spans this many pixels (times height/1080 above
/// 1080 rows; `FUN_14045ea50` writes 8 x max(1, h/1080), tested in `FUN_1404a09a0`/`FUN_140499f30`).
/// Its far particle fades out between 1 and 100 px meanwhile (`galaxy_far.wesl`).
const MEMBERS_PX: f64 = 8.0;
/// Hysteresis: loaded clusters and nodes are dropped beyond FAR_FACTOR times their load range.
const FAR_FACTOR: f64 = 1.25;

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
    /// True absolute magnitude (the sprite may be brighter, see `drawn_mag`).
    pub abs_mag: f32,
    /// Absolute magnitude of its sprite: one drawn star can stand for several.
    pub drawn_mag: f32,
    pub teff: f32,
    pub giant: bool,
}

/// A node of a cluster's octree: level and cell (0..2^level per axis).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NodeId {
    cluster: usize,
    level: u8,
    cell: [u16; 3],
}

impl NodeId {
    /// Index of the node among all of its cluster's nodes, level by level (for member names).
    fn linear(&self) -> usize {
        let n = 1usize << self.level;
        let before: usize = (0..self.level).map(|l| 1usize << (3 * l)).sum();
        before + self.cell[0] as usize + n * (self.cell[1] as usize + n * self.cell[2] as usize)
    }
}

/// A cluster's member population, shared by the builds of its nodes.
struct Model {
    name: String,
    radius: f64,
    /// Plummer scale radius.
    a: f64,
    pop: Pop,
    edges: [f64; 6],
    /// Expected members of each band in the whole cluster.
    band_n: [f64; 5],
}

impl Model {
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
    /// `None`: nothing to draw (resolved by the catalog, or no members).
    Ready(Option<Arc<Model>>),
}

enum NodeSlot {
    Loading(Task<NodeBuilt>),
    /// Entity `None` when the node holds no stars.
    Ready(Option<Entity>, Arc<Vec<MemberStar>>),
}

#[derive(Resource)]
pub struct ClusterField {
    material: Handle<StarMaterial>,
    /// Keyed by index into `catalog.clusters`.
    models: HashMap<usize, ModelSlot>,
    nodes: HashMap<NodeId, NodeSlot>,
    /// SE's `StarDensityInCluster` (main-def.cfg): scales every cluster's member count.
    density: f64,
    /// Positions of the catalog stars (pc), for builds to skip clusters the catalog already
    /// resolves. A snapshot, so builds never hold the catalog itself: while anything shares
    /// it, adding an object deep-copies the whole catalog (a ~300 ms stall).
    star_pos: Arc<Vec<DVec3>>,
    star_count: usize,
}

impl ClusterField {
    /// Member stars of the loaded nodes: (universe position in parsecs, star, cluster index, member
    /// number).
    pub fn members<'a>(&'a self, cat: &'a Catalog) -> impl Iterator<Item = (DVec3, &'a MemberStar, usize, usize)> + 'a {
        self.nodes.iter().flat_map(move |(id, slot)| {
            let stars: &[MemberStar] = match slot {
                NodeSlot::Ready(_, stars) => stars,
                _ => &[],
            };
            let centre = cat.clusters[id.cluster].pos_pc;
            let base = id.linear() * NODE_CAP;
            stars.iter().enumerate().map(move |(k, m)| (centre + m.offset, m, id.cluster, base + k))
        })
    }
}

/// Catalog-ready description of member `k` of cluster `ci`.
pub fn member_info(cat: &Catalog, ci: usize, k: usize, m: &MemberStar) -> ProcStarInfo {
    let cluster = cat.clusters[ci].names.first().map(String::as_str).unwrap_or("RC");
    let lum_class = if m.giant { "III" } else { "V" };
    ProcStarInfo {
        name: format!("{cluster} #{k}"),
        class: format!("{}{}", ms_class_name(m.teff as f64), lum_class),
        abs_mag: m.abs_mag,
        teff: m.teff,
    }
}

#[derive(Component)]
struct ClusterNode {
    /// Cluster centre, universe frame, parsecs.
    centre_pc: DVec3,
}

pub struct ClustersPlugin;

impl Plugin for ClustersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_clusters).add_systems(
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
    let density = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg"))
        .ok()
        .and_then(|t| se_core::sc::parse(&t).find_deep("StarDensityInCluster").and_then(|i| i.f64()))
        .unwrap_or(1.0)
        .max(0.0);
    commands.insert_resource(ClusterField {
        material,
        models: HashMap::new(),
        nodes: HashMap::new(),
        density,
        star_pos: Arc::new(Vec::new()),
        star_count: usize::MAX,
    });
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

fn update_clusters(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<StarSettings>,
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
    let is_far = |rec: &ClusterRecord, cam_pc: DVec3| (rec.pos_pc - cam_pc).length() > view_range(rec) * FAR_FACTOR;
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
    if field.star_count != cat.stars.len() {
        field.star_count = cat.stars.len();
        field.star_pos = Arc::new(cat.stars.iter().filter(|s| !s.barycenter && s.has_position).map(|s| s.pos_pc).collect());
    }
    let mut in_flight = field.models.values().filter(|s| matches!(s, ModelSlot::Loading(_))).count()
        + field.nodes.values().filter(|s| matches!(s, NodeSlot::Loading(_))).count();
    for (i, rec) in cat.clusters.iter().enumerate() {
        if in_flight >= MAX_IN_FLIGHT {
            break;
        }
        if field.models.contains_key(&i) || (rec.pos_pc - cam_pc).length() > view_range(rec) {
            continue;
        }
        let (rec, star_pos, density) = (rec.clone(), field.star_pos.clone(), field.density);
        let task = AsyncComputeTaskPool::get().spawn(async move { build_model(&rec, &star_pos, density).map(Arc::new) });
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
            if id.level + 1 >= LEVELS {
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
    let keep: std::collections::HashSet<NodeId> = wanted.iter().map(|(id, _)| *id).collect();

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
    let mut best: Option<(f64, DVec3, &MemberStar, usize, usize)> = None;
    for (gap, id, stars) in near {
        if best.is_some_and(|b| gap * gap > b.0) {
            break;
        }
        let centre = cat.clusters[id.cluster].pos_pc;
        for (k, m) in stars.iter().enumerate() {
            let p = centre + m.offset;
            let d2 = (p - cam_pc).length_squared();
            if best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, p, m, id.cluster, id.linear() * NODE_CAP + k));
            }
        }
    }
    if let Some((d2, p, m, ci, k)) = best {
        if d2.sqrt() < nearest.dist_pc {
            nearest.star = Some(member_info(cat, ci, k, m));
            nearest.pos_pc = p;
            nearest.dist_pc = d2.sqrt();
        }
    }
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

/// Stellar population of a cluster: stars are drawn from a Salpeter mass
/// function truncated at the turn-off mass, plus red giants when old.
struct Pop {
    m_to: f64,
    giant_frac: f64,
}

/// Returns (absolute V magnitude, effective temperature).
fn sample_star(rng: &mut Rng, pop: &Pop) -> (f64, f64) {
    if rng.f64() < pop.giant_frac {
        let m = -1.0 + 1.5 * rng.f64();
        let teff = 3800.0 + 1100.0 * rng.f64();
        return (m, teff);
    }
    // Salpeter (alpha = 2.35): inverse CDF of m^-1.35 between the limits.
    let (m_lo, m_hi) = (0.1f64, pop.m_to.max(0.1));
    let a = m_lo.powf(-1.35);
    let b = m_hi.powf(-1.35);
    let mass = (a + (b - a) * rng.f64()).powf(-1.0 / 1.35);
    let teff = (5772.0 * mass.powf(0.57)).clamp(2400.0, 40_000.0);
    (main_sequence_abs_mag(teff), teff)
}

/// A member of the band `[lo, hi)`: the population's own draw when it falls in the band (a few
/// hundred tries), else a star at a random magnitude of the band.
fn sample_in_band(rng: &mut Rng, pop: &Pop, lo: f64, hi: f64) -> (f64, f64) {
    for _ in 0..400 {
        let s = sample_star(rng, pop);
        if s.0 >= lo && s.0 < hi {
            return s;
        }
    }
    (lo + (hi - lo) * rng.f64(), if lo < 1.0 { 4400.0 } else { 5772.0 })
}

/// V-band luminosity (solar units) of a star with absolute magnitude `m`.
fn lum_of(m: f64) -> f64 {
    10f64.powf(-0.4 * (m - 4.83))
}

fn mean_lum(rng: &mut Rng, pop: &Pop) -> f64 {
    const N: usize = 512;
    let mut s = 0.0;
    for _ in 0..N {
        s += lum_of(sample_star(rng, pop).0);
    }
    (s / N as f64).max(1e-9)
}

/// Plummer density relative to the centre's, at radius `r` for scale radius `a`.
fn plummer_rho(r: f64, a: f64) -> f64 {
    (1.0 + (r * r) / (a * a)).powf(-2.5)
}

/// The cluster's member model, or `None` when the catalog stars at `star_pos` already resolve it
/// or it has no members.
fn build_model(rec: &ClusterRecord, star_pos: &[DVec3], density: f64) -> Option<Model> {
    let radius = rec.radius_pc.max(0.5);
    let r2 = radius * radius;
    // Catalog stars already cover this cluster.
    let mut found = 0;
    for p in star_pos {
        if (*p - rec.pos_pc).length_squared() < r2 {
            found += 1;
            if found >= 15 {
                return None;
            }
        }
    }
    let name = rec.names.first().cloned().unwrap_or_default();
    let mut rng = Rng(seed_of(&name));
    let age = rec.age_gyr.max(0.01);
    let pop = Pop {
        // Turn-off mass for the age (lifetime ~ m^-2.5, 10 Gyr at 1 solar mass).
        m_to: (10.0 / age).powf(0.4).clamp(0.1, 40.0),
        giant_frac: if age > 1.0 { 0.002 } else { 0.0 },
    };
    // Member count: from the catalog, or from the integrated luminosity.
    let l_tot = 10f64.powf(-0.4 * (rec.abs_mag - 4.83));
    let total = match rec.n_stars {
        Some(n) if n > 0.0 => n,
        _ => l_tot / mean_lum(&mut rng, &pop),
    } * density;
    if !total.is_finite() || total < 1.0 {
        return None;
    }
    // The population's share in each band.
    let edges = band_edges();
    const N: usize = 8192;
    let mut share = [0usize; 5];
    for _ in 0..N {
        let m = sample_star(&mut rng, &pop).0;
        if let Some(b) = (0..5).find(|&b| m >= edges[b] && m < edges[b + 1]) {
            share[b] += 1;
        }
    }
    Some(Model {
        name,
        radius,
        // Plummer scale: catalog radius ~ 2 half-light radii.
        a: 0.5 * radius / 1.305,
        pop,
        edges,
        band_n: std::array::from_fn(|b| total * share[b] as f64 / N as f64),
    })
}

/// The stars of one node (and their mesh): its band's members in its cube, as many as the
/// Plummer profile puts there.
fn build_node(model: &Model, id: NodeId) -> NodeBuilt {
    let mut h = seed_of(&model.name);
    for v in [id.level as u64, id.cell[0] as u64, id.cell[1] as u64, id.cell[2] as u64] {
        h = (h ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    }
    let mut rng = Rng(h);
    let (r, a) = (model.radius, model.a);
    let half = model.node_half(id.level);
    let lo = model.node_centre(id) - DVec3::splat(half);
    let s = 2.0 * half;
    // The node's share of the cluster: the profile averaged over a 4^3 grid of the cube, over
    // its integral inside the radius ((4 pi / 3) a^3 x^3 / (1 + x^2)^1.5, x = r / a).
    let mut sum = 0.0;
    for i in 0..64 {
        let g = DVec3::new((i % 4) as f64, ((i / 4) % 4) as f64, (i / 16) as f64);
        let p = lo + (g + 0.5) * (s / 4.0);
        if p.length() <= r {
            sum += plummer_rho(p.length(), a);
        }
    }
    let x = r / a;
    let norm = 4.0 / 3.0 * std::f64::consts::PI * a.powi(3) * x.powi(3) / (1.0 + x * x).powf(1.5);
    let expected = model.band_n[id.level as usize] * sum / 64.0 * s.powi(3) / norm;
    let n = rng.poisson(expected) as usize;
    if n == 0 {
        return None;
    }
    let drawn = n.min(NODE_CAP);
    // Each drawn star stands for n / drawn: brighten it to keep the light.
    let shift = -2.5 * (n as f64 / drawn as f64).log10();
    // Positions: uniform in the cube and the radius, kept by the profile over its highest value in
    // the cube (at the cube's point nearest the centre).
    let rho_max = plummer_rho(DVec3::ZERO.clamp(lo, lo + DVec3::splat(s)).length(), a);
    let (band_lo, band_hi) = (model.edges[id.level as usize], model.edges[id.level as usize + 1]);
    let (mut p, mut d, mut c, mut ix) = (
        Vec::with_capacity(drawn * 4),
        Vec::with_capacity(drawn * 4),
        Vec::with_capacity(drawn * 4),
        Vec::with_capacity(drawn * 6),
    );
    let mut stars = Vec::with_capacity(drawn);
    let mut tries = 0usize;
    while stars.len() < drawn && tries < drawn * 200 {
        tries += 1;
        let pos = lo + DVec3::new(rng.f64(), rng.f64(), rng.f64()) * s;
        let len = pos.length();
        if len > r || rng.f64() * rho_max > plummer_rho(len, a) {
            continue;
        }
        let (m, teff) = sample_in_band(&mut rng, &model.pop, band_lo, band_hi);
        push_star(&mut p, &mut d, &mut c, &mut ix, pos.as_vec3().to_array(), blackbody_rgb(teff), (m + shift) as f32);
        stars.push(MemberStar {
            offset: pos,
            abs_mag: m as f32,
            drawn_mag: (m + shift) as f32,
            teff: teff as f32,
            giant: teff < 5000.0 && m < 1.0,
        });
    }
    if stars.is_empty() {
        return None;
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
}
