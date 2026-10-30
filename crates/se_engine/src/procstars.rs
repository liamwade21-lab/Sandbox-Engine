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
    last_used: u64,
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
    let material = materials.add(StarMaterial { uniforms: u });
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
    let mut in_flight = field
        .nodes
        .values()
        .filter(|n| matches!(n.state, NodeState::Loading(_)))
        .count();
    if field.enabled && field.galaxy.is_some() {
        for keys in field.octree.wanted_nodes(cam_model, reach, cone, FAR_NODES_MAX) {
            for key in keys {
                if let Some(slot) = field.nodes.get_mut(&key) {
                    slot.last_used = frame_no;
                    continue;
                }
                if in_flight >= MAX_IN_FLIGHT {
                    continue;
                }
                let (o, f, d, h) = (field.octree.clone(), field.frame, field.density.clone(), field.home);
                let task = AsyncComputeTaskPool::get().spawn(async move { build(o, f, d, h, key) });
                field.nodes.insert(
                    key,
                    NodeSlot {
                        state: NodeState::Loading(task),
                        last_used: frame_no,
                    },
                );
                in_flight += 1;
            }
        }
    }

    // Finish builds; new nodes fade in (SE raises a block's brightness by dt / StarFadeDuration a
    // frame, from 0 once it is ready).
    let material = field.material.clone();
    let step = time.delta_secs() / field.fade_s;
    let mut loaded = 0;
    for (key, slot) in field.nodes.iter_mut() {
        if let NodeState::Loading(task) = &mut slot.state {
            if let Some(built) = check_ready(task) {
                let mut fade = None;
                let entity = built.mesh.map(|m| {
                    let center_pc = field.frame.to_universe(field.octree.node_center(*key));
                    let own = materials.get(&material).map(|s| s.uniforms).map(|mut u| {
                        u.tint = Vec4::new(0.0, 0.0, 0.0, 1.0);
                        materials.add(StarMaterial { uniforms: u })
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

    // Drop stale nodes.
    let stale: Vec<NodeKey> = field
        .nodes
        .iter()
        .filter(|(_, s)| matches!(s.state, NodeState::Ready { .. }) && frame_no - s.last_used > KEEP_FRAMES)
        .map(|(k, _)| *k)
        .collect();
    // SE's CacheSizeStarNode: beyond it the least recently wanted nodes go first.
    let cap = memory.map_or(usize::MAX, |m| m.star_nodes);
    let over = crate::memory::lru_victims(
        field.nodes.len() - stale.len(),
        cap,
        field.nodes.iter().filter(|(_, s)| matches!(s.state, NodeState::Ready { .. }) && s.last_used < frame_no && frame_no - s.last_used <= KEEP_FRAMES).map(|(k, s)| (*k, s.last_used)),
    );
    for k in stale.into_iter().chain(over) {
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
        tf.translation = (node.center_pc - cam_pc).as_vec3();
    }
}

fn find_nearest(cam: Res<FlyCamera>, field: Option<Res<ProcStarField>>, mut nearest: ResMut<NearestProcStar>) {
    let Some(field) = field else { return };
    let cam_model = field.frame.to_model(cam.pos.to_parsecs());
    let mut best: Option<(f64, &ProcStar, NodeKey)> = None;
    // The nearest star can live at any level (bright stars sit in coarse nodes),
    // so scan every node whose box could still beat the current best.
    for (key, slot) in &field.nodes {
        let gap = field.octree.box_distance(*key, cam_model);
        if best.is_some_and(|(bd, _, _)| gap * gap > bd) {
            continue;
        }
        if let NodeState::Ready { stars, .. } = &slot.state {
            for s in stars.iter() {
                let d = (s.pos - cam_model).length_squared();
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, s, *key));
                }
            }
        }
    }
    if field.frame_no % 120 == 0 {
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
