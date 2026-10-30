//! Procedural stars of the galaxy the camera is in, streamed from a magnitude-stratified
//! octree around the camera (see `se_core::star_octree`). The Milky Way uses its analytic
//! model; any other galaxy gets [`GalaxyStarDensity`] (its Hubble type, size and luminosity,
//! with the spiral arms of its sprite model), so every galaxy is full of stars from inside.
//!
//! Every loaded node is a sprite mesh whose vertices are star positions relative
//! to the node centre (universe frame, parsecs). All nodes share one
//! [`StarMaterial`] in "octree mode": the node entity's translation holds the node
//! centre minus the camera, in parsecs, so camera motion only updates transforms. A new node
//! fades in over SE's `StarFadeDuration` on a copy of that material, then joins the shared one.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::catalog::ObjectRef;
use se_core::catalog::GalaxyRecord;
use se_core::procgen::procedural_star_name;
use se_core::star_octree::{
    DensityModel, GalaxyForm, GalaxyFrame, GalaxyStarDensity, MilkyWayDensity, NodeKey, ProcStar, StarOctree,
};

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::stars::{StarMaterial, StarSettings, build_sprite_mesh, push_star, uniforms_for};
use crate::universe::UniverseSet;

const MAX_IN_FLIGHT: usize = 48;
/// Frames a node may stay unused before it is dropped.
const KEEP_FRAMES: u64 = 90;
/// With the magnitude limit raised above the one the octree was laid out for, nodes are loaded
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
    density: Arc<dyn DensityModel>,
    /// Catalog galaxy the stars belong to.
    pub galaxy: usize,
    milky_way: usize,
    /// Galaxy part of the star names (`RS <code>-<level>-<node>-<index>`); `None` in the Milky
    /// Way, whose stars keep their short `RS L-x-y-z-i` names.
    code: Option<String>,
    /// A galaxy the camera has entered whose sprite model (and so its arms) is still being
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
/// How long to wait for a galaxy's sprite model before generating its stars without arms.
const ARMS_WAIT_S: f32 = 5.0;

/// A procedural star nearer than this (and nearer than any catalog star) becomes a real
/// catalog star with planets, the centre of the active system.
pub const PROMOTE_PC: f64 = 50.0;

/// A procedural star (Milky Way field or cluster member) that can become a catalog star.
#[derive(Debug, Clone)]
pub struct ProcStarInfo {
    pub name: String,
    /// Spectral class, e.g. "G2V".
    pub class: String,
    pub abs_mag: f32,
    pub teff: f32,
}


/// Nearest procedural star to the camera (from the loaded nodes of the Milky Way field
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

/// Procedural star name in SpaceEngine's "RS" style, from the node address.
pub fn proc_star_name(key: NodeKey, index: u32) -> String {
    format!("RS {}-{}-{}-{}-{}", key.level, key.cell[0], key.cell[1], key.cell[2], index)
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
    let frame = GalaxyFrame {
        center_pc: g.pos_pc,
        rot: g.orientation,
    };
    let sun_model = frame.to_model(DVec3::ZERO);
    register_field_galaxy(g.pos_pc, g.radius_pc);
    let density = milky_way_density(g);
    let octree = StarOctree::new(0, g.radius_pc, settings.faint_limit() as f64);
    info!(
        "procedural Milky Way: {} octree levels, Sun at model ({:.0}, {:.0}, {:.0}) pc",
        octree.levels, sun_model.x, sun_model.y, sun_model.z
    );
    let mut u = uniforms_for(Vec3::ZERO, &settings);
    u.anchor = Vec4::new(0.0, 0.0, 0.0, 2.0);
    let material = materials.add(StarMaterial { uniforms: u });
    commands.insert_resource(ProcStarField {
        enabled: true,
        octree,
        frame,
        density,
        galaxy: gi as usize,
        milky_way: gi as usize,
        code: None,
        pending: None,
        nodes: HashMap::new(),
        material,
        fade_s: crate::stars::block_fade_seconds(&cfg),
        frame_no: 0,
        loaded_stars: 0,
    });
}

fn milky_way_density(g: &GalaxyRecord) -> Arc<dyn DensityModel> {
    let frame = GalaxyFrame {
        center_pc: g.pos_pc,
        rot: g.orientation,
    };
    Arc::new(MilkyWayDensity::new(g.radius_pc, frame.to_model(DVec3::ZERO)))
}

/// Seed of a galaxy's star octree (FNV-1a of its name); the Milky Way keeps 0.
fn galaxy_seed(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193)).max(1)
}

impl ProcStarField {
    /// Name, class and brightness of a loaded star.
    pub fn info(&self, s: &ProcStar, key: NodeKey) -> ProcStarInfo {
        let name = match &self.code {
            None => proc_star_name(key, s.index),
            Some(code) => procedural_star_name(code, key.cell.map(|c| c as i64), key.level, s.index),
        };
        ProcStarInfo {
            name,
            class: s.class.clone(),
            abs_mag: s.abs_mag,
            teff: s.teff,
        }
    }

    /// Start generating the stars of another galaxy: drop every node (loading ones are
    /// cancelled) and rebuild the octree in its frame.
    fn switch_to(&mut self, commands: &mut Commands, g: usize, rec: &GalaxyRecord, density: Arc<dyn DensityModel>, mag_limit: f64) {
        for (_, slot) in self.nodes.drain() {
            if let NodeState::Ready { entity: Some(e), .. } = slot.state {
                commands.entity(e).despawn();
            }
        }
        let mw = g == self.milky_way;
        self.galaxy = g;
        self.frame = GalaxyFrame {
            center_pc: rec.pos_pc,
            rot: rec.orientation,
        };
        self.octree = StarOctree::new(if mw { 0 } else { galaxy_seed(&rec.names[0]) }, rec.radius_pc, mag_limit);
        register_field_galaxy(rec.pos_pc, rec.radius_pc);
        self.density = density;
        self.code = (!mw).then(|| rec.names[0].strip_prefix("RG ").map(str::to_string).unwrap_or_else(|| g.to_string()));
        self.pending = None;
        self.loaded_stars = 0;
        info!("procedural stars: inside {} ({}), {} octree levels", rec.names[0], rec.kind, self.octree.levels);
    }

    /// Universe position (pc) of a star of this field named `RS L-x-y-z-i` (Milky Way) or
    /// `RS <code>-L-<node>-i`; generates its node if needed.
    pub fn find_by_name(&self, name: &str) -> Option<DVec3> {
        let rest = name.trim().strip_prefix("RS ")?;
        let n: Vec<i64> = rest.rsplit('-').take(5).map(|s| s.trim().parse().ok()).collect::<Option<_>>()?;
        let key = match &self.code {
            None if n.len() == 5 && rest.split('-').count() == 5 => NodeKey {
                level: n[4] as u8,
                cell: [n[3] as i32, n[2] as i32, n[1] as i32],
            },
            _ => return None,
        };
        let index = n[0] as u32;
        if let Some(NodeSlot { state: NodeState::Ready { stars, .. }, .. }) = self.nodes.get(&key) {
            return stars.iter().find(|s| s.index == index).map(|s| self.frame.to_universe(s.pos));
        }
        let built = build(self.octree.clone(), self.frame, self.density.clone(), self.galaxy == self.milky_way, key);
        built.stars.iter().find(|s| s.index == index).map(|s| self.frame.to_universe(s.pos))
    }

    /// A snapshot for scanning the field's stars off the main thread.
    pub fn scan(&self) -> StarScan {
        StarScan {
            octree: self.octree.clone(),
            frame: self.frame,
            density: self.density.clone(),
            milky_way: self.galaxy == self.milky_way,
            code: self.code.clone(),
        }
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

/// SE's `StarGenerator` limits (main-def.cfg): procedural stars brighter than `StarMaxAppMagn` as
/// seen from the Sun, or nearer to it than `StarMinDistNormal`, are left to the catalogs
/// (FUN_1405945e0 keeps the magnitude as a brightness coefficient and the distances squared).
#[derive(Clone, Copy, Debug)]
struct StarGenLimits {
    max_app_mag: f64,
    min_dist_pc: f64,
}

static STAR_GEN: std::sync::OnceLock<StarGenLimits> = std::sync::OnceLock::new();

fn star_gen() -> StarGenLimits {
    *STAR_GEN.get_or_init(|| StarGenLimits { max_app_mag: 10.0, min_dist_pc: 2.5 })
}

fn load_star_generator(cfg: Res<crate::config::EngineConfig>) {
    let get = |k: &str, d: f64| crate::stars::main_def_f64(&cfg, k).unwrap_or(d);
    let _ = STAR_GEN.set(StarGenLimits { max_app_mag: get("StarMaxAppMagn", 10.0), min_dist_pc: get("StarMinDistNormal", 2.5) });
}

/// The stars of one node, without those the real catalogs already provide (bright-from-Earth
/// stars and the Sun's neighbourhood).
fn node_stars(
    octree: &StarOctree,
    frame: GalaxyFrame,
    density: &dyn DensityModel,
    milky_way: bool,
    key: NodeKey,
    keep: &dyn Fn(DVec3) -> bool,
) -> Vec<ProcStar> {
    let sun_model = frame.to_model(DVec3::ZERO);
    let lim = star_gen();
    let exclude = move |p: DVec3, m: f64| {
        let d = (p - sun_model).length().max(0.01);
        milky_way && (d < lim.min_dist_pc || m + 5.0 * (d / 10.0).log10() < lim.max_app_mag)
    };
    octree.generate_where(key, density, &exclude, keep)
}

/// Which octree levels a search visits: all, those whose magnitude band meets an absolute
/// magnitude range, or only the dead stars' level.
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
    density: Arc<dyn DensityModel>,
    milky_way: bool,
    code: Option<String>,
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
        let mut r = radius_pc.min(10.0);
        loop {
            let found = self.stars_in_sphere(center_pc, r, limit, levels, keep);
            if found.len() >= limit || r >= radius_pc {
                return found;
            }
            r = (r * 2.0).min(radius_pc);
        }
    }

    /// Every node the sphere touches, level by level on all threads, typing only the stars
    /// inside it. The deep levels' nodes are tiny (0.2 pc) and nearly always empty: with the
    /// sphere's peak density as a bound, most are dismissed on their first random draw.
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
        // Peak density over the sphere and a margin (sampled, doubled for safety).
        let reach = radius_pc * 1.25;
        let mut rho_bound: f64 = 0.0;
        for i in 0..=16 {
            for j in 0..=16 {
                for k in 0..=16 {
                    let f = |a: i32| (a as f64 / 8.0 - 1.0) * reach;
                    rho_bound = rho_bound.max(self.density.density(c + DVec3::new(f(i), f(j), f(k))));
                }
            }
        }
        rho_bound *= 2.0;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
        let inside = |p: DVec3| (p - c).length() <= radius_pc;
        let mut out: Vec<(DVec3, ProcStarInfo)> = Vec::new();
        for level in 0..self.octree.levels {
            let wanted = match levels {
                ScanLevels::All => true,
                ScanLevels::Mags(lo, hi) => {
                    let (b, f) = self.octree.mag_band(level);
                    b < hi && f > lo
                }
                ScanLevels::Remnants => level == self.octree.remnant_level(),
            };
            if !wanted {
                continue;
            }
            let s = self.octree.node_size(level);
            // Nodes well inside the sampled box: their expected count is bounded.
            let max_expected = (s < radius_pc * 0.25).then(|| rho_bound * self.octree.level_rate(level) * s * s * s);
            let lo = self.octree.cell_of(level, c - DVec3::splat(radius_pc));
            let hi = self.octree.cell_of(level, c + DVec3::splat(radius_pc));
            let n = (1i64 << level) as i32;
            let (x0, x1) = (lo[0].max(0), hi[0].min(n - 1));
            if x1 < x0 {
                continue;
            }
            let xs: Vec<i32> = (x0..=x1).collect();
            let chunk = xs.len().div_ceil(threads).max(1);
            let part: Vec<(DVec3, ProcStarInfo)> = std::thread::scope(|sc| {
                let handles: Vec<_> = xs
                    .chunks(chunk)
                    .map(|xc| {
                        let inside = &inside;
                        sc.spawn(move || {
                            let mut v = Vec::new();
                            // Distance from the centre to a cell's span along one axis (0 inside).
                            let origin = -self.octree.node_size(0) * 0.5;
                            let gap = |cell: i32, cc: f64| {
                                let a = origin + cell as f64 * s;
                                (a - cc).max(cc - (a + s)).max(0.0)
                            };
                            let r2 = radius_pc * radius_pc;
                            for &x in xc {
                                let gx = gap(x, c.x);
                                for y in lo[1].max(0)..=hi[1].min(n - 1) {
                                    let gy = gap(y, c.y);
                                    if gx * gx + gy * gy > r2 {
                                        continue;
                                    }
                                    // The column's chord through the sphere: every cell on it touches the sphere.
                                    let zr = (r2 - gx * gx - gy * gy).sqrt();
                                    let z0 = self.octree.cell_of(level, DVec3::new(0.0, 0.0, c.z - zr))[2].max(0);
                                    let z1 = self.octree.cell_of(level, DVec3::new(0.0, 0.0, c.z + zr))[2].min(n - 1);
                                    for z in z0..=z1 {
                                        let key = NodeKey { level, cell: [x, y, z] };
                                        if max_expected.is_some_and(|m| self.octree.surely_empty(key, m)) {
                                            continue;
                                        }
                                        for st in node_stars(&self.octree, self.frame, self.density.as_ref(), self.milky_way, key, inside) {
                                            if keep(&st) {
                                                v.push((self.frame.to_universe(st.pos), self.info(&st, key)));
                                            }
                                        }
                                        if v.len() > limit * 2 + 4096 {
                                            trim(&mut v);
                                        }
                                    }
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

    fn info(&self, s: &ProcStar, key: NodeKey) -> ProcStarInfo {
        let name = match &self.code {
            None => proc_star_name(key, s.index),
            Some(code) => procedural_star_name(code, key.cell.map(|c| c as i64), key.level, s.index),
        };
        ProcStarInfo {
            name,
            class: s.class.clone(),
            abs_mag: s.abs_mag,
            teff: s.teff,
        }
    }
}

fn build(octree: StarOctree, frame: GalaxyFrame, density: Arc<dyn DensityModel>, milky_way: bool, key: NodeKey) -> BuiltNode {
    let stars = node_stars(&octree, frame, density.as_ref(), milky_way, key, &|_| true);
    let mesh = if stars.is_empty() {
        None
    } else {
        let center_model = octree.node_center(key);
        let (mut p, mut d, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for s in &stars {
            let rel = (frame.rot * (s.pos - center_model)).as_vec3();
            push_star(&mut p, &mut d, &mut c, &mut i, rel.to_array(), s.rgb, s.abs_mag);
        }
        Some(build_sprite_mesh(p, d, c, i))
    };
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
) {
    let Some(mut field) = field else { return };
    let field = &mut *field;
    field.frame_no += 1;
    let frame_no = field.frame_no;
    let cam_model = field.frame.to_model(cam.pos.to_parsecs());

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
    if field.enabled {
        for keys in field.octree.wanted_nodes(cam_model, reach, cone, FAR_NODES_MAX) {
            for key in keys {
                if let Some(slot) = field.nodes.get_mut(&key) {
                    slot.last_used = frame_no;
                    continue;
                }
                if in_flight >= MAX_IN_FLIGHT {
                    continue;
                }
                let (o, f, d, mw) = (field.octree.clone(), field.frame, field.density.clone(), field.galaxy == field.milky_way);
                let task = AsyncComputeTaskPool::get().spawn(async move { build(o, f, d, mw, key) });
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
    for k in stale {
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
/// inside, within `ENTER_RADII` of a new one and `LEAVE_RADII` of the current one.
/// Outside every galaxy it stays where it is (no node of its octree is then near the camera).
fn follow_galaxy(
    mut commands: Commands,
    time: Res<Time>,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<StarSettings>,
    galaxies: Option<Res<crate::galaxies::GalaxyState>>,
    field: Option<ResMut<ProcStarField>>,
) {
    let Some(mut field) = field else { return };
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
        let limit = if g == field.galaxy { LEAVE_RADII } else { ENTER_RADII };
        let k = (rec.pos_pc - cam_pc).length() / (rec.radius_pc * limit);
        if k < 1.0 && best.is_none_or(|(bk, _)| k < bk) {
            best = Some((k, g));
        }
    }
    let Some((_, g)) = best.filter(|&(_, g)| g != field.galaxy) else {
        field.pending = None;
        return;
    };
    let rec = &cat.galaxies[g];
    let density = if g == field.milky_way {
        milky_way_density(rec)
    } else {
        let now = time.elapsed_secs();
        let since = match field.pending {
            Some((p, t)) if p == g => t,
            _ => {
                field.pending = Some((g, now));
                now
            }
        };
        let arms = match galaxies.as_ref().map(|s| s.interior_arms(g, rec.radius_pc)) {
            Some(Some(arms)) => arms,
            // Wait for the sprite model: its emission map puts the stars into its arms.
            Some(None) if now - since < ARMS_WAIT_S => return,
            _ => None,
        };
        Arc::new(GalaxyStarDensity::new(GalaxyForm::from_type(&rec.kind), rec.radius_pc, arms))
    };
    field.switch_to(&mut commands, g, rec, density, settings.faint_limit() as f64);
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
        let half = field.octree.node_size(key.level) * 0.5;
        let gap = ((cam_model - field.octree.node_center(*key)).abs() - DVec3::splat(half)).max(DVec3::ZERO);
        if best.is_some_and(|(bd, _, _)| gap.length_squared() > bd) {
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
        debug!(
            "procstars: {} nodes ({} ready), {} stars, nearest {:?} pc",
            field.nodes.len(),
            ready,
            field.loaded_stars,
            best.map(|(d2, _, k)| (d2.sqrt(), k.level))
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
