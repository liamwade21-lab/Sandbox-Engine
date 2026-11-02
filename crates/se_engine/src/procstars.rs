//! Procedural stars of the galaxy the camera is in, streamed from SpaceEngine's star octree around
//! the camera (see `se_core::star_octree`). A galaxy with a sprite model fills its octree from the
//! model's emission voxel map, as SE does (the Milky Way waits for its map up to `MAP_WAIT_S`); one
//! without uses SE's analytic disk, bulge and halo.
//!
//! Every loaded node is a sprite mesh whose vertices are star positions relative
//! to the node centre (universe frame, parsecs). All nodes share one
//! [`StarMaterial`] in "octree mode": the node entity's translation holds the node
//! centre minus the camera, in parsecs, so camera motion only updates transforms. A new node
//! fades in over SE's `StarFadeDuration` on a copy of that material, then joins the shared one.
//! Planemos (rogue planets) are dark: they are in the nodes, never in the meshes.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::catalog::ObjectRef;
use se_core::star::StarKind;
use se_core::star_octree::{GalaxyFrame, HomeCut, NodeKey, ProcStar, StarDensity, StarOctree};

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::stars::{StarMaterial, StarSettings, build_sprite_mesh, push_star, uniforms_for};
use crate::universe::UniverseSet;

const MAX_IN_FLIGHT: usize = 48;
/// Frames a node may stay unused before it is dropped.
const KEEP_FRAMES: u64 = 90;
/// With the magnitude limit raised above the one the node reach is measured for, nodes are loaded
/// all around out to at most this times their usual reach (1.5 magnitudes); SE loads them all the
/// way, which a big raise turns into millions of stars.
const FULL_REACH_MAX: f64 = 2.0;
/// Most nodes per level loaded beyond their usual reach inside the zoomed view.
const FAR_NODES_MAX: usize = 512;
/// SE's default `CacheSizeStarNode` (main-def.cfg), when the memory limits are not loaded.
const STAR_NODES_DEFAULT: usize = 50_000;

struct BuiltNode {
    mesh: Option<Mesh>,
    stars: Arc<Vec<ProcStar>>,
}

enum NodeState {
    Loading(Task<BuiltNode>),
    /// `fade`: while the node fades in, its brightness (0-1) and its own material.
    Ready { entity: Option<Entity>, stars: Arc<Vec<ProcStar>>, fade: Option<(f32, Handle<StarMaterial>)> },
}

struct NodeSlot {
    state: NodeState,
    /// The last frame the node was wanted, except while it is in the current wanted set (`set`).
    last_used: u64,
    /// Id of the last [`WantedSet`] that held the node.
    set: u64,
}

impl NodeSlot {
    /// The last frame the node was wanted: this one while it is in the current set.
    fn used(&self, current: Option<u64>, frame_no: u64) -> u64 {
        if current == Some(self.set) { frame_no } else { self.last_used }
    }
}

/// The nodes `wanted_nodes` gave and the bits of everything it read (octree, camera, reach, cone,
/// limits). On a planet the camera's f64 position steps only every few dozen frames, so most
/// frames reuse the set and skip both the search and a lookup per node.
#[derive(Default)]
struct WantedCache {
    /// Bumped when the octree is replaced.
    octree_gen: u64,
    /// The id last given to a set.
    last_id: u64,
    current: Option<WantedSet>,
    /// The set that ended last and its last frame, for [`settle`](Self::settle). Every slot is
    /// settled each frame, so the sets that ended before are settled already.
    ended: Option<(u64, u64)>,
}

struct WantedSet {
    inputs: WantedInputs,
    keys: Vec<NodeKey>,
    /// Stamped into the slots of its nodes: while the set is current they count as used every
    /// frame, and once it has ended they get its last frame as `last_used` (`settle`).
    id: u64,
    /// Some node of the set has no slot yet (the `MAX_IN_FLIGHT` limit).
    missing: bool,
}

type WantedInputs = [u64; 13];

impl WantedCache {
    fn inputs(&self, cam_model: DVec3, reach: f64, cone: Option<(DVec3, f64)>, max_nodes: usize, mag_limit: f64) -> WantedInputs {
        let (axis, half) = cone.unwrap_or((DVec3::ZERO, 0.0));
        let c = cam_model.to_array().map(f64::to_bits);
        let a = axis.to_array().map(f64::to_bits);
        [
            self.octree_gen,
            c[0],
            c[1],
            c[2],
            reach.to_bits(),
            cone.is_some() as u64,
            a[0],
            a[1],
            a[2],
            half.to_bits(),
            max_nodes as u64,
            FAR_NODES_MAX as u64,
            mag_limit.to_bits(),
        ]
    }

    fn current_id(&self) -> Option<u64> {
        self.current.as_ref().map(|s| s.id)
    }

    /// Ends the current set: its nodes were last wanted the frame before `frame_no`.
    fn end(&mut self, frame_no: u64) {
        if let Some(set) = self.current.take() {
            self.ended = Some((set.id, frame_no - 1));
        }
    }

    /// Gives a node of the set that ended last that set's last frame as `last_used`. Called for
    /// every slot every frame, after `mark` or `end` and before the stale and LRU tests.
    fn settle(&self, slot: &mut NodeSlot) {
        if let Some((id, last)) = self.ended
            && slot.set == id
        {
            slot.last_used = last;
        }
    }

    /// Marks this frame's wanted nodes (from `wanted`, unless `inputs` are those of the current
    /// set) and requests missing ones with `spawn`, in order, while fewer than `MAX_IN_FLIGHT` load.
    fn mark(
        &mut self,
        nodes: &mut HashMap<NodeKey, NodeSlot>,
        frame_no: u64,
        inputs: WantedInputs,
        wanted: impl FnOnce() -> Vec<Vec<NodeKey>>,
        mut spawn: impl FnMut(NodeKey) -> NodeState,
    ) {
        if self.current.as_ref().is_none_or(|s| s.inputs != inputs) {
            self.end(frame_no);
            self.last_id += 1;
            let keys = wanted().into_iter().flatten().collect();
            self.current = Some(WantedSet { inputs, keys, id: self.last_id, missing: true });
        }
        let set = self.current.as_mut().unwrap();
        // Nodes of the current set are never dropped, so with none missing there is nothing to do.
        if !set.missing {
            return;
        }
        set.missing = false;
        let mut in_flight = nodes.values().filter(|n| matches!(n.state, NodeState::Loading(_))).count();
        for &key in &set.keys {
            if let Some(slot) = nodes.get_mut(&key) {
                slot.last_used = frame_no;
                slot.set = set.id;
                continue;
            }
            if in_flight >= MAX_IN_FLIGHT {
                set.missing = true;
                continue;
            }
            nodes.insert(key, NodeSlot { state: spawn(key), last_used: frame_no, set: set.id });
            in_flight += 1;
        }
    }
}

/// Nodes to drop: the stale ones, then beyond `cap` (SE's CacheSizeStarNode) the least recently
/// wanted. `current`: the id of the current wanted set.
fn unwanted(nodes: &HashMap<NodeKey, NodeSlot>, current: Option<u64>, frame_no: u64, cap: usize) -> Vec<NodeKey> {
    let mut out: Vec<NodeKey> = nodes
        .iter()
        .filter(|(_, s)| matches!(s.state, NodeState::Ready { .. }) && frame_no - s.used(current, frame_no) > KEEP_FRAMES)
        .map(|(k, _)| *k)
        .collect();
    let over = crate::memory::lru_victims(
        nodes.len() - out.len(),
        cap,
        nodes
            .iter()
            .map(|(k, s)| (*k, s, s.used(current, frame_no)))
            .filter(|(_, s, used)| matches!(s.state, NodeState::Ready { .. }) && *used < frame_no && frame_no - used <= KEEP_FRAMES)
            .map(|(k, _, used)| (k, used)),
    );
    out.extend(over);
    out
}

#[derive(Resource)]
pub struct ProcStarField {
    pub enabled: bool,
    pub octree: StarOctree,
    pub frame: GalaxyFrame,
    density: StarDensity,
    /// In the home galaxy: what the catalogs hold instead (SE's `StarGenerator` limits).
    home: Option<HomeCut>,
    /// Catalog galaxy the stars belong to; `None` until the first galaxy is set up.
    pub galaxy: Option<usize>,
    milky_way: usize,
    /// A galaxy the camera has entered whose sprite model (and so its voxel map) is still being
    /// built, and since when (seconds).
    pending: Option<(usize, f32)>,
    nodes: HashMap<NodeKey, NodeSlot>,
    wanted: WantedCache,
    material: Handle<StarMaterial>,
    /// SE's `StarFadeDuration` (s).
    fade_s: f32,
    frame_no: u64,
    pub loaded_stars: usize,
}

/// The camera enters a galaxy within this many of its radii and leaves it beyond `LEAVE_RADII`. As in SE
/// a galaxy's brightest stars already show as a cloud around it from a few radii away.
const ENTER_RADII: f64 = 3.0;
const LEAVE_RADII: f64 = 3.5;
/// How long to wait for a galaxy's sprite model (its voxel map) before generating its stars from
/// SE's analytic components.
const MAP_WAIT_S: f32 = 5.0;

/// A procedural star nearer than this (and nearer than any catalog star) becomes a real
/// catalog star with planets, the centre of the active system.
pub const PROMOTE_PC: f64 = 50.0;

/// What SE's star generator made of a field star: its mass, age, [Fe/H] and C/O.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeStarFacts {
    pub mass_sol: f32,
    pub age_gyr: f32,
    pub feh: f32,
    pub c_to_o: f32,
}

/// A procedural star (galaxy field or cluster member) that can become a catalog star.
#[derive(Debug, Clone)]
pub struct ProcStarInfo {
    pub name: String,
    /// Spectral class, e.g. "G2V" ("Planemo" for a rogue planet).
    pub class: String,
    pub abs_mag: f32,
    pub teff: f32,
    /// A cluster member's age (Gyr) and [Fe/H]: SE copies the cluster's age into its members
    /// (FUN_14053a760) before drawing their scatter and metallicity (FUN_1401d6c60). None for
    /// field stars, which carry their own in `se`.
    pub age_feh: Option<(f32, f32)>,
    /// A galaxy field star's values from SE's star generator.
    pub se: Option<SeStarFacts>,
}

/// Nearest procedural star to the camera (from the loaded nodes of the galaxy field
/// and of star clusters).
#[derive(Resource, Debug, Clone)]
pub struct NearestProcStar {
    pub star: Option<ProcStarInfo>,
    /// Universe position, parsecs.
    pub pos_pc: DVec3,
    pub dist_pc: f64,
}

impl Default for NearestProcStar {
    fn default() -> Self {
        Self {
            star: None,
            pos_pc: DVec3::ZERO,
            dist_pc: f64::INFINITY,
        }
    }
}

#[derive(Component)]
pub struct ProcStarNode {
    /// Node centre, universe frame, parsecs.
    pub center_pc: DVec3,
}

pub struct ProcStarsPlugin;

/// Runs `find_nearest`; cluster members are compared against its result after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcStarSet;

impl Plugin for ProcStarsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NearestProcStar>()
            .add_systems(Startup, (load_star_generator, setup_field))
            .add_systems(
                PostUpdate,
                (follow_galaxy, stream_nodes, position_nodes, find_nearest)
                    .chain()
                    .in_set(ProcStarSet)
                    .after(UniverseSet::Camera)
                    .before(UniverseSet::FloatingOrigin),
            );
    }
}

/// Centre (universe frame, pc) and radius (pc) of every galaxy the field has streamed stars of
/// (the Milky Way from start-up, others as the camera enters them); read by
/// [`galaxy_radius_fraction`].
static FIELD_GALAXIES: std::sync::RwLock<Vec<(DVec3, f64)>> = std::sync::RwLock::new(Vec::new());

fn register_field_galaxy(center_pc: DVec3, radius_pc: f64) {
    if let Ok(mut g) = FIELD_GALAXIES.write() {
        if radius_pc > 0.0 && !g.iter().any(|&(c, _)| c == center_pc) {
            g.push((center_pc, radius_pc));
        }
    }
}

/// A star's distance from the centre of its own galaxy (the streamed galaxy it lies deepest in)
/// in galaxy radii, which SE's star generator reads for its metallicity and C/O (FUN_1401d63b0;
/// SE scales each axis by the galaxy model's extent, assumed spherical here). A star's galaxy is
/// registered before any of its stars exist, so the value does not depend on where the camera
/// is. `None` before the field is set up.
pub fn galaxy_radius_fraction(pos_pc: DVec3) -> Option<f64> {
    FIELD_GALAXIES.read().ok()?.iter().map(|&(c, r)| pos_pc.distance(c) / r).min_by(f64::total_cmp)
}

/// A placeholder octree until the first galaxy is set up (no levels hold anything near the camera).
fn empty_octree() -> StarOctree {
    StarOctree::new(
        &se_core::star_octree::GalaxyStarInput {
            dims: DVec3::ONE,
            radius_pc: 1.0,
            hubble: 0,
            has_model: false,
            seed: 0,
            galaxy: se_core::proc_galsys::GalaxyRef::Catalog(0),
        },
        7.0,
    )
}

fn setup_field(
    mut commands: Commands,
    cfg: Res<crate::config::EngineConfig>,
    data: Res<SeData>,
    settings: Res<StarSettings>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    let cat = &data.catalog;
    let Some(ObjectRef::Galaxy(gi)) = cat.find("Milky Way") else {
        warn!("Milky Way not found in catalogs; procedural stars disabled");
        return;
    };
    let g = &cat.galaxies[gi as usize];
    register_field_galaxy(g.pos_pc, g.radius_pc);
    let mut u = uniforms_for(Vec3::ZERO, &settings);
    u.anchor = Vec4::new(0.0, 0.0, 0.0, 2.0);
    let material = materials.add(StarMaterial { uniforms: u, camera: crate::stars::SPRITE_CAMERA });
    commands.insert_resource(ProcStarField {
        enabled: true,
        octree: empty_octree(),
        frame: GalaxyFrame { center_pc: g.pos_pc, rot: g.orientation },
        density: StarDensity::Analytic,
        home: None,
        galaxy: None,
        milky_way: gi as usize,
        pending: None,
        nodes: HashMap::new(),
        wanted: WantedCache::default(),
        material,
        fade_s: crate::stars::block_fade_seconds(&cfg),
        frame_no: 0,
        loaded_stars: 0,
    });
}

impl ProcStarField {
    /// Name, class and brightness of a loaded star.
    pub fn info(&self, s: &ProcStar, key: NodeKey) -> ProcStarInfo {
        star_info(&self.octree, s, key)
    }

    /// Start generating the stars of another galaxy: drop every node (loading ones are
    /// cancelled) and rebuild the octree in its frame.
    fn switch_to(&mut self, commands: &mut Commands, g: usize, cat: &se_core::catalog::Catalog, octree: StarOctree, density: StarDensity) {
        for (_, slot) in self.nodes.drain() {
            if let NodeState::Ready { entity: Some(e), .. } = slot.state {
                commands.entity(e).despawn();
            }
        }
        let rec = &cat.galaxies[g];
        self.galaxy = Some(g);
        self.frame = GalaxyFrame { center_pc: rec.pos_pc, rot: rec.orientation };
        let lim = star_gen();
        self.home = (g == self.milky_way).then(|| HomeCut {
            sun: self.frame.to_model(DVec3::ZERO),
            max_app_mag: lim.max_app_mag,
            min_dist_normal: lim.min_dist_normal,
            min_dist_brown: lim.min_dist_brown,
            min_dist_remnant: lim.min_dist_remnant,
        });
        register_field_galaxy(rec.pos_pc, rec.radius_pc);
        info!(
            "procedural stars: inside {} ({}), size class {}, {}^3 roots, {} levels, {} star density",
            rec.names[0],
            rec.kind,
            octree.size_class,
            1u32 << octree.depth,
            octree.levels,
            if matches!(density, StarDensity::Voxel(_)) { "voxel map" } else { "analytic" }
        );
        self.octree = octree;
        self.wanted.octree_gen += 1;
        self.density = density;
        self.pending = None;
        self.loaded_stars = 0;
    }

    /// Universe position (pc) of a star of this field named `RS <galaxy>-<root>-<level>-<node>-<index>`;
    /// generates its node if needed.
    pub fn find_by_name(&self, name: &str) -> Option<DVec3> {
        self.galaxy?;
        let (key, index) = self.octree.parse_name(name)?;
        if let Some(NodeSlot { state: NodeState::Ready { stars, .. }, .. }) = self.nodes.get(&key) {
            return stars.iter().find(|s| s.index == index).map(|s| self.frame.to_universe(s.pos));
        }
        let built = build(self.octree.clone(), self.frame, self.density.clone(), self.home, key);
        built.stars.iter().find(|s| s.index == index).map(|s| self.frame.to_universe(s.pos))
    }

    /// A snapshot for scanning the field's stars off the main thread.
    pub fn scan(&self) -> StarScan {
        StarScan { octree: self.octree.clone(), frame: self.frame, density: self.density.clone(), home: self.home, ready: self.galaxy.is_some() }
    }

    /// Stars of the loaded nodes: (universe position in parsecs, star, node).
    pub fn loaded_stars(&self) -> impl Iterator<Item = (DVec3, &ProcStar, NodeKey)> {
        self.nodes.iter().flat_map(move |(key, slot)| {
            let stars: &[ProcStar] = match &slot.state {
                NodeState::Ready { stars, .. } => stars,
                _ => &[],
            };
            stars.iter().map(move |s| (self.frame.to_universe(s.pos), s, *key))
        })
    }
}

fn star_info(octree: &StarOctree, s: &ProcStar, key: NodeKey) -> ProcStarInfo {
    ProcStarInfo {
        name: octree.star_name(key, s.index),
        class: s.class.clone(),
        abs_mag: s.abs_mag,
        teff: s.teff,
        age_feh: None,
        se: Some(SeStarFacts { mass_sol: s.mass_sol, age_gyr: s.age_gyr, feh: s.feh, c_to_o: s.c_to_o }),
    }
}

/// SE's `StarGenerator` limits (main-def.cfg): procedural stars brighter than `StarMaxAppMagn` as
/// seen from the Sun, or nearer to it than `StarMinDistNormal` (`StarMinDistBrownDwarf` for brown
/// dwarfs, `StarMinDistRemnant` for remnants and planemos), are left to the catalogs (FUN_1405945e0
/// keeps the magnitude as a brightness coefficient and the distances squared).
#[derive(Clone, Copy, Debug)]
struct StarGenLimits {
    max_app_mag: f64,
    min_dist_normal: f64,
    min_dist_brown: f64,
    min_dist_remnant: f64,
}

static STAR_GEN: std::sync::OnceLock<StarGenLimits> = std::sync::OnceLock::new();

fn star_gen() -> StarGenLimits {
    *STAR_GEN.get_or_init(|| StarGenLimits { max_app_mag: 10.0, min_dist_normal: 2.5, min_dist_brown: 2.0, min_dist_remnant: 3.0 })
}

fn load_star_generator(cfg: Res<crate::config::EngineConfig>) {
    let get = |k: &str, d: f64| crate::stars::main_def_f64(&cfg, k).unwrap_or(d);
    let _ = STAR_GEN.set(StarGenLimits {
        max_app_mag: get("StarMaxAppMagn", 10.0),
        min_dist_normal: get("StarMinDistNormal", 2.5),
        min_dist_brown: get("StarMinDistBrownDwarf", 2.0),
        min_dist_remnant: get("StarMinDistRemnant", 3.0),
    });
}

/// Which octree levels a search visits: all, those whose magnitude band meets an absolute
/// magnitude range, or only the deepest (its lone remnants and planemos).
#[derive(Clone, Copy, Debug)]
pub enum ScanLevels {
    All,
    Mags(f64, f64),
    Remnants,
}

/// What a background scan needs of the field: every star within a radius, as the field
/// would stream them (the Star browser's search).
#[derive(Clone)]
pub struct StarScan {
    octree: StarOctree,
    frame: GalaxyFrame,
    density: StarDensity,
    home: Option<HomeCut>,
    ready: bool,
}

impl StarScan {
    /// The `limit` nearest stars within `radius_pc` of `center_pc` (universe frame) that pass
    /// `keep`: position (universe pc) and star, nearest first. The sphere grows from 10 pc until
    /// it holds `limit` of them (the nearest 10,000 systems lie within about 100 ly, whatever
    /// radius was asked for).
    pub fn stars_within(
        &self,
        center_pc: DVec3,
        radius_pc: f64,
        limit: usize,
        levels: ScanLevels,
        keep: &(dyn Fn(&ProcStar) -> bool + Sync),
    ) -> Vec<(DVec3, ProcStarInfo)> {
        if !self.ready {
            return Vec::new();
        }
        let mut r = radius_pc.min(10.0);
        loop {
            let found = self.stars_in_sphere(center_pc, r, limit, levels, keep);
            if found.len() >= limit || r >= radius_pc {
                return found;
            }
            r = (r * 2.0).min(radius_pc);
        }
    }

    /// Every node the sphere touches, level by level on all threads.
    fn stars_in_sphere(
        &self,
        center_pc: DVec3,
        radius_pc: f64,
        limit: usize,
        levels: ScanLevels,
        keep: &(dyn Fn(&ProcStar) -> bool + Sync),
    ) -> Vec<(DVec3, ProcStarInfo)> {
        let nearer = |a: &(DVec3, ProcStarInfo), b: &(DVec3, ProcStarInfo)| {
            a.0.distance_squared(center_pc).total_cmp(&b.0.distance_squared(center_pc))
        };
        // Millions of stars lie within a few hundred light years: keep only the nearest.
        let trim = |v: &mut Vec<(DVec3, ProcStarInfo)>| {
            if v.len() > limit {
                v.select_nth_unstable_by(limit, nearer);
                v.truncate(limit);
            }
        };
        let c = self.frame.to_model(center_pc);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
        let inside = |p: DVec3| (p - c).length() <= radius_pc;
        let mut out: Vec<(DVec3, ProcStarInfo)> = Vec::new();
        let deepest = self.octree.levels - 1;
        for level in 0..self.octree.levels {
            let wanted = match levels {
                ScanLevels::All => true,
                ScanLevels::Mags(lo, hi) => {
                    let (b, f) = self.octree.mag_band(level);
                    // The deepest level also holds remnants with magnitudes of their own.
                    (b < hi && f > lo) || level == deepest
                }
                ScanLevels::Remnants => level == deepest,
            };
            if !wanted {
                continue;
            }
            let mut keys = Vec::new();
            self.octree.for_cells_near(level, c, radius_pc, |k| {
                if self.octree.box_distance(k, c) <= radius_pc {
                    keys.push(k);
                }
            });
            if keys.is_empty() {
                continue;
            }
            let chunk = keys.len().div_ceil(threads).max(1);
            let part: Vec<(DVec3, ProcStarInfo)> = std::thread::scope(|sc| {
                let handles: Vec<_> = keys
                    .chunks(chunk)
                    .map(|kc| {
                        let inside = &inside;
                        sc.spawn(move || {
                            let mut v = Vec::new();
                            for &key in kc {
                                for st in self.octree.generate_where(key, &self.density, self.home.as_ref(), inside) {
                                    if keep(&st) {
                                        v.push((self.frame.to_universe(st.pos), star_info(&self.octree, &st, key)));
                                    }
                                }
                                if v.len() > limit * 2 + 4096 {
                                    trim(&mut v);
                                }
                            }
                            trim(&mut v);
                            v
                        })
                    })
                    .collect();
                handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            });
            out.extend(part);
            trim(&mut out);
        }
        out.sort_by(nearer);
        out
    }
}

fn build(octree: StarOctree, frame: GalaxyFrame, density: StarDensity, home: Option<HomeCut>, key: NodeKey) -> BuiltNode {
    let stars = octree.generate_where(key, &density, home.as_ref(), &|_| true);
    let center_model = octree.node_center(key);
    let (mut p, mut d, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for s in stars.iter().filter(|s| s.luminous()) {
        let rel = (frame.rot * (s.pos - center_model)).as_vec3();
        push_star(&mut p, &mut d, &mut c, &mut i, rel.to_array(), s.rgb, s.abs_mag);
    }
    let mesh = (!p.is_empty()).then(|| build_sprite_mesh(p, d, c, i));
    BuiltNode {
        mesh,
        stars: Arc::new(stars),
    }
}

#[allow(clippy::too_many_arguments)]
fn stream_nodes(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    settings: Res<StarSettings>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    field: Option<ResMut<ProcStarField>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StarMaterial>>,
    time: Res<Time<Real>>,
    memory: Option<Res<crate::memory::MemoryLimits>>,
) {
    let Some(mut field) = field else { return };
    let field = &mut *field;
    field.frame_no += 1;
    let frame_no = field.frame_no;
    let cam_model = field.frame.to_model(cam.pos.to_parsecs());
    // The reach of the nodes follows the star magnitude limit.
    field.octree.mag_limit = settings.faint_limit() as f64;

    // SE keeps a node while its brightest star could show under the limit raised by the zoom
    // (FUN_1404a0e80), and below StarFrustumClipFOV only inside the view frustum (FUN_14047b670).
    let reach = 10f64.powf(0.2 * (settings.zoomed_load_limit() as f64 - field.octree.mag_limit));
    let cone = settings
        .view_cone(&cam, crate::stars::window_aspect(&windows))
        .map(|(axis, half)| (field.frame.rot.inverse() * axis, half));
    let reach = if cone.is_some() { reach } else { reach.min(FULL_REACH_MAX) };

    // Mark wanted nodes, request missing ones (coarse levels first, nearest first).
    if field.enabled && field.galaxy.is_some() {
        // SE's CacheSizeStarNode bounds the nodes wanted as well as those kept.
        let max_nodes = memory.as_ref().map_or(STAR_NODES_DEFAULT, |m| m.star_nodes);
        let inputs = field.wanted.inputs(cam_model, reach, cone, max_nodes, field.octree.mag_limit);
        let (octree, frame, density, home) = (&field.octree, field.frame, &field.density, field.home);
        field.wanted.mark(
            &mut field.nodes,
            frame_no,
            inputs,
            || octree.wanted_nodes(cam_model, reach, cone, FAR_NODES_MAX, max_nodes),
            |key| {
                let (o, f, d, h) = (octree.clone(), frame, density.clone(), home);
                NodeState::Loading(AsyncComputeTaskPool::get().spawn(async move { build(o, f, d, h, key) }))
            },
        );
    } else {
        // The nodes are no longer wanted: they go stale.
        field.wanted.end(frame_no);
    }

    // Finish builds; new nodes fade in (SE raises a block's brightness by dt / StarFadeDuration a
    // frame, from 0 once it is ready).
    let material = field.material.clone();
    let step = time.delta_secs() / field.fade_s;
    let mut loaded = 0;
    for (key, slot) in field.nodes.iter_mut() {
        field.wanted.settle(slot);
        if let NodeState::Loading(task) = &mut slot.state {
            if let Some(built) = check_ready(task) {
                let mut fade = None;
                let entity = built.mesh.map(|m| {
                    let center_pc = field.frame.to_universe(field.octree.node_center(*key));
                    let own = materials.get(&material).map(|s| s.uniforms).map(|mut u| {
                        u.tint = Vec4::new(0.0, 0.0, 0.0, 1.0);
                        materials.add(StarMaterial { uniforms: u, camera: crate::stars::SPRITE_CAMERA })
                    });
                    let node_material = own.clone().unwrap_or_else(|| material.clone());
                    fade = own.map(|h| (0.0, h));
                    commands
                        .spawn((
                            Mesh3d(meshes.add(m)),
                            MeshMaterial3d(node_material),
                            Transform::IDENTITY,
                            Visibility::Visible,
                            NoFrustumCulling,
                            ProcStarNode { center_pc },
                        ))
                        .id()
                });
                slot.state = NodeState::Ready { entity, stars: built.stars, fade };
                continue;
            }
        }
        if let NodeState::Ready { entity: Some(e), fade: fade @ Some(_), .. } = &mut slot.state {
            let (f, own) = fade.as_mut().unwrap();
            *f = (*f + step).min(1.0);
            if *f >= 1.0 {
                commands.entity(*e).insert(MeshMaterial3d(material.clone()));
                *fade = None;
            } else if let Some(mut m) = materials.get_mut(own) {
                m.uniforms.tint = Vec4::new(*f, *f, *f, 1.0);
            }
        }
        if let NodeState::Ready { stars, .. } = &slot.state {
            loaded += stars.len();
        }
    }
    field.loaded_stars = loaded;

    let cap = memory.map_or(usize::MAX, |m| m.star_nodes);
    for k in unwanted(&field.nodes, field.wanted.current_id(), frame_no, cap) {
        if let Some(NodeSlot {
            state: NodeState::Ready { entity: Some(e), .. },
            ..
        }) = field.nodes.remove(&k)
        {
            commands.entity(e).despawn();
        }
    }
}

/// Follow the camera into other galaxies: the field moves to the galaxy the camera is deepest
/// inside, within `ENTER_RADII` of a new one and `LEAVE_RADII` of the current one (the Milky Way
/// from the start). Outside every galaxy it stays where it is (no node of its octree is then near
/// the camera). A galaxy with a sprite model waits up to `MAP_WAIT_S` for its voxel map.
#[allow(clippy::too_many_arguments)]
fn follow_galaxy(
    mut commands: Commands,
    time: Res<Time>,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<StarSettings>,
    galaxies: Option<Res<crate::galaxies::GalaxyState>>,
    density: Option<Res<crate::galaxies::GalaxyDensity>>,
    field: Option<ResMut<ProcStarField>>,
) {
    let Some(mut field) = field else { return };
    let Some(gs) = galaxies else { return };
    if field.frame_no % 8 != 0 {
        return;
    }
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    let mut best: Option<(f64, usize)> = None;
    for (g, rec) in cat.galaxies.iter().enumerate() {
        if rec.quasar || rec.radius_pc <= 0.0 {
            continue;
        }
        // Depth in units of the entry (or, for the current galaxy, exit) distance: below 1 the
        // camera is inside, and of two overlapping galaxies the current one is favoured.
        let limit = if Some(g) == field.galaxy { LEAVE_RADII } else { ENTER_RADII };
        let k = (rec.pos_pc - cam_pc).length() / (rec.radius_pc * limit);
        if k < 1.0 && best.is_none_or(|(bk, _)| k < bk) {
            best = Some((k, g));
        }
    }
    // The Milky Way's stars from the start, wherever the camera is.
    let target = best.map(|(_, g)| g).or(field.galaxy.is_none().then_some(field.milky_way));
    let Some(g) = target.filter(|&g| Some(g) != field.galaxy) else {
        field.pending = None;
        return;
    };
    let Some(input) = crate::procsystems::star_input(g as u32, cat, &gs) else { return };
    let now = time.elapsed_secs();
    let since = match field.pending {
        Some((p, t)) if p == g => t,
        _ => {
            field.pending = Some((g, now));
            now
        }
    };
    let map = if g == field.milky_way {
        match density.as_ref().and_then(|d| d.map()) {
            Some(m) => Some(m),
            None if input.has_model && now - since < MAP_WAIT_S => return,
            None => None,
        }
    } else {
        match gs.interior_map(g) {
            Some(m) => m,
            None if now - since < MAP_WAIT_S => return,
            None => None,
        }
    };
    let density = match map {
        Some(m) if input.has_model => StarDensity::Voxel(m),
        _ => StarDensity::Analytic,
    };
    let octree = StarOctree::new(&input, settings.faint_limit() as f64);
    field.switch_to(&mut commands, g, cat, octree, density);
}

fn position_nodes(cam: Res<FlyCamera>, mut q: Query<(&ProcStarNode, &mut Transform)>) {
    let cam_pc = cam.pos.to_parsecs();
    for (node, mut tf) in &mut q {
        // Only when it moves (bit for bit): a changed transform re-extracts the node's mesh.
        let t = (node.center_pc - cam_pc).as_vec3();
        if tf.translation.to_array().map(f32::to_bits) != t.to_array().map(f32::to_bits) {
            tf.translation = t;
        }
    }
}

/// The star of the ready `nodes` nearest `cam_model`: (squared distance, star, node).
fn nearest_star<'a>(octree: &StarOctree, nodes: &'a HashMap<NodeKey, NodeSlot>, cam_model: DVec3) -> Option<(f64, &'a ProcStar, NodeKey)> {
    let ready = |slot: &'a NodeSlot| match &slot.state {
        NodeState::Ready { stars, .. } => Some(stars),
        _ => None,
    };
    let mut best: Option<(f64, &ProcStar, NodeKey)> = None;
    // Start from the finest ready node holding the camera: with a near best at once, the boxes of
    // the other nodes rule out nearly all of their stars.
    let seed = (0..octree.levels)
        .rev()
        .map(|level| NodeKey { level, cell: octree.cell_of(level, cam_model) })
        .find_map(|key| nodes.get(&key).and_then(ready).map(|stars| (key, stars)));
    if let Some((key, stars)) = seed {
        for s in stars.iter() {
            let d = (s.pos - cam_model).length_squared();
            if best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, s, key));
            }
        }
    }
    // The nearest star can live at any level (bright stars sit in coarse nodes),
    // so scan every node whose box could still beat the current best.
    for (key, slot) in nodes {
        let gap = octree.box_distance(*key, cam_model);
        if best.is_some_and(|(bd, _, _)| gap * gap > bd) {
            continue;
        }
        if let Some(stars) = ready(slot) {
            for s in stars.iter() {
                let d = (s.pos - cam_model).length_squared();
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, s, *key));
                }
            }
        }
    }
    best
}

fn find_nearest(cam: Res<FlyCamera>, field: Option<Res<ProcStarField>>, mut nearest: ResMut<NearestProcStar>) {
    let Some(field) = field else { return };
    let cam_model = field.frame.to_model(cam.pos.to_parsecs());
    let best = nearest_star(&field.octree, &field.nodes, cam_model);
    // The report scans every loaded star (over a million): only when debug logging would show it.
    if field.frame_no % 120 == 0 && bevy::log::tracing::enabled!(bevy::log::Level::DEBUG) {
        let ready = field.nodes.values().filter(|n| matches!(n.state, NodeState::Ready { .. })).count();
        // The nearest rogue planet too (there is no way to see one).
        let planemo = field
            .loaded_stars()
            .filter(|(_, s, _)| s.kind == StarKind::Planemo)
            .min_by(|a, b| (a.1.pos - cam_model).length_squared().total_cmp(&(b.1.pos - cam_model).length_squared()))
            .map(|(_, s, k)| (field.octree.star_name(k, s.index), (s.pos - cam_model).length()));
        debug!(
            "procstars: {} nodes ({} ready), {} stars, nearest {:?} pc, nearest planemo {:?}",
            field.nodes.len(),
            ready,
            field.loaded_stars,
            best.map(|(d2, s, k)| (d2.sqrt(), k.level, s.kind == StarKind::Planemo)),
            planemo
        );
    }
    match best {
        Some((d2, s, key)) => {
            nearest.pos_pc = field.frame.to_universe(s.pos);
            nearest.dist_pc = d2.sqrt();
            nearest.star = Some(field.info(s, key));
        }
        None => {
            nearest.star = None;
            nearest.dist_pc = f64::INFINITY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn milky_way(mag_limit: f64) -> StarOctree {
        let r = 19_329.9;
        StarOctree::new(
            &se_core::star_octree::GalaxyStarInput {
                dims: DVec3::splat(r),
                radius_pc: r,
                hubble: 15,
                has_model: true,
                seed: -8_628_293,
                galaxy: se_core::proc_galsys::GalaxyRef::Catalog(8514),
            },
            mag_limit,
        )
    }

    #[test]
    fn nearest_star_matches_a_full_scan() {
        let t = milky_way(7.0);
        let home = DVec3::new(8_500.0, 0.0, 10.0);
        let ready = |stars| NodeSlot { state: NodeState::Ready { entity: None, stars: Arc::new(stars), fade: None }, last_used: 0, set: 0 };
        let mut map: HashMap<NodeKey, NodeSlot> = t
            .wanted_nodes(home, 1.0, None, 0, 400)
            .into_iter()
            .flatten()
            .map(|k| (k, ready(t.generate_where(k, &StarDensity::Analytic, None, &|_| true))))
            .collect();
        // A node still loading is skipped, even the camera's own (the next level's seeds the scan).
        let pool = bevy::tasks::TaskPool::new();
        let own = NodeKey { level: t.levels - 1, cell: t.cell_of(t.levels - 1, home) };
        assert!(map.contains_key(&own));
        map.insert(own, NodeSlot { state: NodeState::Loading(pool.spawn(std::future::pending::<BuiltNode>())), last_used: 0, set: 0 });
        let nodes: Vec<(NodeKey, Arc<Vec<ProcStar>>)> = map
            .iter()
            .filter_map(|(k, s)| match &s.state {
                NodeState::Ready { stars, .. } => Some((*k, stars.clone())),
                _ => None,
            })
            .collect();
        assert!(nodes.iter().map(|n| n.1.len()).sum::<usize>() > 10_000);
        let offsets = [DVec3::ZERO, DVec3::new(3.0, -2.0, 0.5), DVec3::new(40.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 900.0), DVec3::splat(1e5)];
        for (i, off) in offsets.into_iter().enumerate() {
            let cam = home + off;
            let full = nodes
                .iter()
                .flat_map(|(k, v)| v.iter().map(move |s| ((s.pos - cam).length_squared(), s, *k)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap();
            let fast = nearest_star(&t, &map, cam).unwrap();
            assert_eq!(fast.0.to_bits(), full.0.to_bits(), "camera {i}");
            assert_eq!((fast.2, fast.1.index), (full.2, full.1.index), "camera {i}");
        }
        assert!(nearest_star(&t, &HashMap::new(), home).is_none());
    }

    /// The old marking: `wanted_nodes` every frame, each wanted slot stamped with the frame.
    fn old_mark(nodes: &mut HashMap<NodeKey, NodeSlot>, frame_no: u64, wanted: Vec<Vec<NodeKey>>, mut spawn: impl FnMut(NodeKey) -> NodeState) {
        let mut in_flight = nodes.values().filter(|n| matches!(n.state, NodeState::Loading(_))).count();
        for key in wanted.into_iter().flatten() {
            if let Some(slot) = nodes.get_mut(&key) {
                slot.last_used = frame_no;
                continue;
            }
            if in_flight >= MAX_IN_FLIGHT {
                continue;
            }
            nodes.insert(key, NodeSlot { state: spawn(key), last_used: frame_no, set: 0 });
            in_flight += 1;
        }
    }

    #[test]
    fn cached_wanted_nodes_match_the_old_marking() {
        let t = milky_way(7.0);
        let pool = bevy::tasks::TaskPool::new();
        let loading = || NodeState::Loading(pool.spawn(std::future::pending::<BuiltNode>()));
        let hasher = std::collections::hash_map::RandomState::new();
        let mut old: HashMap<NodeKey, NodeSlot> = HashMap::with_hasher(hasher.clone());
        let mut new: HashMap<NodeKey, NodeSlot> = HashMap::with_hasher(hasher);
        let mut cache = WantedCache::default();
        let (max_nodes, cap) = (500, 650);
        let a = DVec3::new(8_500.0, 0.0, 10.0);
        let cone = Some((DVec3::new(-1.0, 0.3, 0.1).normalize(), 0.05));
        let mut wanted_calls = 0;
        // Stay, move, stay long enough for stale nodes to go, come back, zoom, switch the field
        // off and on, replace the octree.
        let mut path: Vec<(DVec3, f64, Option<(DVec3, f64)>, bool)> = Vec::new();
        path.extend((0..40).map(|_| (a, 1.0, None, true)));
        path.extend((0..20).map(|i| (a + DVec3::new(2.0 * i as f64, 1.0, 0.0), 1.0, None, true)));
        path.extend((0..120).map(|_| (a + DVec3::new(60.0, 1.0, 0.0), 1.0, None, true)));
        path.extend((0..30).map(|_| (a, 1.0, None, true)));
        path.extend((0..30).map(|_| (a, 8.0, cone, true)));
        path.extend((0..5).map(|_| (a, 1.0, None, false)));
        path.extend((0..100).map(|i| (a + DVec3::splat((i / 25) as f64), 1.0, None, i % 40 != 7)));
        let mut drops = 0;
        for (f, &(cam, reach, cone, enabled)) in path.iter().enumerate() {
            let frame_no = f as u64 + 1;
            if f == 300 {
                old.clear();
                new.clear();
                cache.octree_gen += 1;
            }
            // Builds finish in map order, the same in both maps.
            for map in [&mut old, &mut new] {
                for slot in map.values_mut().filter(|s| matches!(s.state, NodeState::Loading(_))).take(30) {
                    slot.state = NodeState::Ready { entity: None, stars: Arc::new(Vec::new()), fade: None };
                }
            }
            if enabled {
                old_mark(&mut old, frame_no, t.wanted_nodes(cam, reach, cone, FAR_NODES_MAX, max_nodes), |_| loading());
                let inputs = cache.inputs(cam, reach, cone, max_nodes, t.mag_limit);
                let wanted = || {
                    wanted_calls += 1;
                    t.wanted_nodes(cam, reach, cone, FAR_NODES_MAX, max_nodes)
                };
                cache.mark(&mut new, frame_no, inputs, wanted, |_| loading());
            } else {
                cache.end(frame_no);
            }
            // As the build loop does.
            new.values_mut().for_each(|s| cache.settle(s));
            let current = cache.current_id();
            let gone = unwanted(&old, None, frame_no, cap);
            assert_eq!(gone, unwanted(&new, current, frame_no, cap), "frame {frame_no}");
            drops += gone.len();
            for k in &gone {
                old.remove(k);
                new.remove(k);
            }
            let snap = |m: &HashMap<NodeKey, NodeSlot>, current| {
                m.iter().map(|(k, s)| (*k, s.used(current, frame_no), matches!(s.state, NodeState::Ready { .. }))).collect::<Vec<_>>()
            };
            assert_eq!(snap(&old, None), snap(&new, current), "frame {frame_no}");
        }
        assert!(drops > 100, "{drops}");
        assert!(wanted_calls < path.len() / 3, "{wanted_calls}");
    }
}
