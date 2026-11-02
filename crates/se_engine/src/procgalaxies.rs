//! Procedural galaxies beyond the catalogs (`RG` names), streamed from the galaxy octree of
//! `se_core::proc_galaxy`.
//!
//! * **Far field**: every loaded octree node is one mesh of far sprites (the same shader and vertex layout
//!   as the catalog galaxies, `galaxies.rs`). Vertices are relative to the node centre and the entity
//!   translation holds `centre - camera`, so camera motion only updates transforms. All nodes share one
//!   [`GalaxyFarMaterial`] whose uniforms are copied from the catalog galaxies' material each frame (same
//!   brightness, intergalactic-vision gain and daylight fade).
//! * **Models**: a procedural galaxy that becomes large on screen is added to the catalog at runtime
//!   ([`galaxy_record`]); `galaxies.rs` then builds its 3D sprite model like any catalog galaxy. While that
//!   model fades in, the far sprite fades out through the `resident` list.
//! * **Names**: [`ensure_galaxy`] turns `RG a-level-node-index` into a catalog galaxy.
//! * Setting: [`ProcSettings::galaxies`] (`ShowProcGalaxies`), env `SE_NO_PROCGAL=1` disables it.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::window::PrimaryWindow;
use se_core::catalog::{Catalog, GalaxyRecord, ObjectRef};
use se_core::proc_galaxy::{self, GalNodeKey, ProcGalaxy};
use se_core::proc_galsys::DeepSkyConfig;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::galaxies::{
    ATTR_FAR_CORNER, ATTR_FAR_DATA, ATTR_FAR_SHAPE, GALAXY_FAR_BIAS, GalaxyFarMaterial, GalaxySettings, GalaxyState, RESIDENT_SLOTS, angular_px, far_uniforms, set_far_uniforms,
    pad_aabb, profile_type, type_color,
};
use crate::universe::UniverseSet;

/// Nodes generated at once.
const MAX_IN_FLIGHT: usize = 24;
/// Frames a node may stay unwanted before it is dropped.
const KEEP_FRAMES: u64 = 90;
/// Brightest-galaxy magnitude a node must reach to be worth generating.
const NODE_MAG_LIMIT: f32 = 19.0;
/// Galaxies fainter than this from the camera are not picked or counted in the sky coverage.
const GALAXY_MAG_CULL: f32 = 18.5;
/// A level-`L` node is requested while the camera is within `REACH` node sizes of it.
const REACH: f64 = 1.0;
/// Upper bound on resident nodes.
const MAX_NODES: usize = 1400;

/// User-visible switches of the procedural deep sky (`ShowProcGalaxies`, `ShowProcClusters`,
/// `ShowProcNebulae`) and the generator settings of `config/main-def.cfg`.
#[derive(Resource, Clone)]
pub struct ProcSettings {
    pub galaxies: bool,
    pub clusters: bool,
    pub nebulae: bool,
    pub cfg: DeepSkyConfig,
}

impl Default for ProcSettings {
    fn default() -> Self {
        // SE_NO_PROCGAL=1 turns everything off; SE_PROC=g|c|n|gc... enables only the listed kinds (testing).
        let off = std::env::var_os("SE_NO_PROCGAL").is_some();
        let only = std::env::var("SE_PROC").ok();
        let on = |c: char| !off && only.as_ref().is_none_or(|s| s.contains(c));
        Self { galaxies: on('g'), clusters: on('c'), nebulae: on('n'), cfg: DeepSkyConfig::default() }
    }
}

struct BuiltNode {
    mesh: Option<Mesh>,
    galaxies: Arc<Vec<ProcGalaxy>>,
    /// Sprites in the mesh.
    drawn: usize,
}

enum NodeState {
    Loading(Task<BuiltNode>),
    /// `fade`: while the node fades in (SE's `StarFadeDuration`), its brightness (0-1) and its own
    /// copy of the material.
    Ready { entity: Option<Entity>, galaxies: Arc<Vec<ProcGalaxy>>, max_radius: f64, drawn: usize, fade: Option<(f32, Handle<GalaxyFarMaterial>)> },
}

struct NodeSlot {
    state: NodeState,
    last_used: u64,
}

#[derive(Resource)]
pub struct ProcGalaxyField {
    nodes: HashMap<GalNodeKey, NodeSlot>,
    material: Handle<GalaxyFarMaterial>,
    /// SE's `StarFadeDuration` (s).
    fade_s: f32,
    frame_no: u64,
    /// Procedural galaxies already in the catalog: `(node, index) -> catalog index`.
    promoted: HashMap<(GalNodeKey, u32), u32>,
    /// Catalog index -> far-sprite id of the same galaxy (for the model fade).
    sprite_id: HashMap<u32, f32>,
    loaded_galaxies: usize,
    loaded_nodes: usize,
    drawn_sprites: usize,
    last_cam: DVec3,
    wanted: Vec<GalNodeKey>,
    /// Bumped whenever a node is added, becomes ready or is dropped.
    node_gen: u64,
    /// The inputs of the last `promote_near` run, when it found nothing to promote.
    idle_promote: Option<PromoteKey>,
}

/// What `promote_near` depends on besides the promoted set, which only grows (so a run that found
/// nothing finds nothing again with the same key): bit patterns of the camera, the focal length, the
/// size threshold and the view cone, and the node generation.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PromoteKey {
    cam: [u64; 3],
    focal: u64,
    px_min: u64,
    cone: Option<([u64; 3], u64)>,
    node_gen: u64,
}

impl ProcGalaxyField {
    pub fn loaded_nodes(&self) -> usize {
        self.loaded_nodes
    }

    pub fn loaded_galaxies(&self) -> usize {
        self.loaded_galaxies
    }

    /// Drawn galaxies of the loaded nodes that are not in the catalog yet (catalog entries are
    /// picked as such).
    pub fn unpromoted(&self, cam_pc: DVec3) -> impl Iterator<Item = &ProcGalaxy> + '_ {
        self.nodes
            .values()
            .filter_map(|s| match &s.state {
                NodeState::Ready { galaxies, .. } => Some(galaxies.iter()),
                _ => None,
            })
            .flatten()
            .filter(move |g| {
                !self.promoted.contains_key(&(g.node, g.index))
                    && proc_galaxy::app_mag(g.lum, (g.pos_pc - cam_pc).length() as f32) <= GALAXY_MAG_CULL
            })
    }
}

/// Adds a picked procedural galaxy to the catalog; returns its catalog index.
pub fn promote_galaxy(data: &mut SeData, field: &mut ProcGalaxyField, g: &ProcGalaxy) -> u32 {
    promote(field, data.catalog_mut(), g)
}

#[derive(Component)]
pub struct ProcGalaxyNode {
    pub center_pc: DVec3,
}

pub struct ProcGalaxiesPlugin;

impl Plugin for ProcGalaxiesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProcSettings>().add_systems(Startup, setup_field.after(crate::galaxies::setup_galaxies)).add_systems(
            PostUpdate,
            (stream_nodes, position_nodes, promote_near, auto_gain, sync_material)
                .chain()
                .after(UniverseSet::Camera)
                .before(UniverseSet::FloatingOrigin),
        );
    }
}

/// Far-sprite id of a procedural galaxy (exact in `f32`, distinct enough to avoid clashes in the 8 resident slots).
fn sprite_id(key: GalNodeKey, index: u32) -> f32 {
    let h = (key.id.wrapping_mul(0x9E37_79B1).wrapping_add(key.level as u64 * 0x85EB_CA6B) as u32) ^ index.wrapping_mul(0x27D4_EB2F);
    (h % 16_000_000) as f32
}

fn setup_field(
    mut commands: Commands,
    cfg: Res<crate::config::EngineConfig>,
    mut settings: ResMut<ProcSettings>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
) {
    // The generator settings come from the install's config.
    if let Ok(text) = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")) {
        settings.cfg = DeepSkyConfig::from_cfg(&text);
        if std::env::var_os("SE_NO_PROCGAL").is_none() && std::env::var_os("SE_PROC").is_none() {
            settings.galaxies = settings.cfg.show_galaxies;
            settings.clusters = settings.cfg.show_clusters;
            settings.nebulae = settings.cfg.show_nebulae;
        }
    }
    let material = far_mats.add(GalaxyFarMaterial::new(far_uniforms(Vec3::ZERO, &GalaxySettings::default()), GALAXY_FAR_BIAS));
    commands.insert_resource(ProcGalaxyField {
        nodes: HashMap::new(),
        material,
        fade_s: crate::stars::block_fade_seconds(&cfg),
        frame_no: 0,
        promoted: HashMap::new(),
        sprite_id: HashMap::new(),
        loaded_galaxies: 0,
        loaded_nodes: 0,
        drawn_sprites: 0,
        last_cam: DVec3::splat(f64::MAX),
        wanted: Vec::new(),
        node_gen: 0,
        idle_promote: None,
    });
}

/// The catalog record of a procedural galaxy.
pub fn galaxy_record(g: &ProcGalaxy) -> GalaxyRecord {
    let q = g.quat;
    GalaxyRecord {
        names: vec![g.name()],
        kind: g.kind().to_string(),
        quasar: false,
        pos_pc: g.pos_pc,
        radius_pc: g.display_radius_pc(),
        abs_mag: g.abs_mag as f64,
        vis_mag: g.vis_abs_mag() as f64,
        orientation: DQuat::from_xyzw(q[1] as f64, q[2] as f64, q[3] as f64, q[0] as f64).normalize(),
        model_bright: 1.0,
        source: "procedural".into(),
        model_roll: g.model_roll,
    }
}

/// The node's galaxies and their sprites: all of them, as the camera may come close to any while
/// the node stays loaded; the shader hides the faint ones each frame (galaxy_far.wesl).
fn build_node(key: GalNodeKey, pools: [u32; 17]) -> BuiltNode {
    let galaxies = proc_galaxy::generate_node(key, &pools);
    let center = key.center();
    let (mut p, mut d, mut s, mut c, mut ix) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for g in &galaxies {
        let rec = galaxy_record(g);
        let ptype = profile_type(&rec);
        let normal = if ptype == 1 { (rec.orientation * DVec3::NEG_Z).as_vec3() } else { Vec3::ZERO };
        let col = type_color(&rec.kind);
        let rel = (g.pos_pc - center).as_vec3();
        let id = sprite_id(key, g.index);
        let base = p.len() as u32;
        for k in [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            p.push(rel.to_array());
            d.push([col[0], col[1], col[2], g.vis_abs_mag()]);
            s.push([normal.x, normal.y, normal.z, rec.radius_pc as f32]);
            c.push([k[0], k[1], id, ptype as f32]);
        }
        ix.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let p_len = p.len() / 4;
    let mesh = if p.is_empty() {
        None
    } else {
        pad_aabb(&mut p);
        d.extend([[0.0; 4]; 2]);
        s.extend([[0.0; 4]; 2]);
        c.extend([[0.0; 4]; 2]);
        Some(
            Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, p)
                .with_inserted_attribute(ATTR_FAR_DATA, d)
                .with_inserted_attribute(ATTR_FAR_SHAPE, s)
                .with_inserted_attribute(ATTR_FAR_CORNER, c)
                .with_inserted_indices(Indices::U32(ix)),
        )
    };
    let drawn = p_len;
    BuiltNode { mesh, galaxies: Arc::new(galaxies), drawn }
}

/// The octree nodes around `cam` that are worth drawing: per level the nodes within [`REACH`] node sizes
/// whose brightest possible galaxy reaches [`NODE_MAG_LIMIT`].
fn wanted_nodes(cam: DVec3) -> Vec<GalNodeKey> {
    let mut out = Vec::new();
    for level in 0..proc_galaxy::LEVELS {
        let size = 2.0 * GalNodeKey { level, id: 0 }.half();
        for key in GalNodeKey::nodes_around(level, cam, size * REACH) {
            if key.distance_to(cam) <= size * REACH && proc_galaxy::node_visible(key, cam, NODE_MAG_LIMIT) {
                out.push(key);
            }
        }
    }
    out.truncate(MAX_NODES);
    out
}

fn stream_nodes(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    settings: Res<ProcSettings>,
    field: Option<ResMut<ProcGalaxyField>>,
    galaxy_state: Option<Res<GalaxyState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
    time: Res<Time<Real>>,
    memory: Option<Res<crate::memory::MemoryLimits>>,
) {
    let (Some(mut field), Some(gs)) = (field, galaxy_state) else { return };
    let field = &mut *field;
    field.frame_no += 1;
    let frame_no = field.frame_no;
    let cam_pc = cam.pos.to_parsecs();

    // Recompute the wanted set when the camera moved a little (or every 30 frames).
    if settings.galaxies && (field.last_cam.distance(cam_pc) > 1.0 || frame_no % 30 == 0) {
        field.wanted = wanted_nodes(cam_pc);
        field.last_cam = cam_pc;
    }
    let wanted: Vec<GalNodeKey> = if settings.galaxies { field.wanted.clone() } else { Vec::new() };

    let mut in_flight = field.nodes.values().filter(|n| matches!(n.state, NodeState::Loading(_))).count();
    for key in wanted {
        if let Some(slot) = field.nodes.get_mut(&key) {
            slot.last_used = frame_no;
            continue;
        }
        if in_flight >= MAX_IN_FLIGHT {
            continue;
        }
        let pools = gs.model_pools();
        let task = AsyncComputeTaskPool::get().spawn(async move { build_node(key, pools) });
        field.nodes.insert(key, NodeSlot { state: NodeState::Loading(task), last_used: frame_no });
        field.node_gen += 1;
        in_flight += 1;
    }

    // Finish builds; new nodes fade in as SE's blocks of galaxies do (as its star blocks,
    // `procstars::stream_nodes`).
    let material = field.material.clone();
    let step = time.delta_secs() / field.fade_s;
    let (mut n_gal, mut n_ready, mut n_drawn) = (0, 0, 0);
    for (key, slot) in field.nodes.iter_mut() {
        if let NodeState::Ready { entity: Some(e), fade: fade @ Some(_), .. } = &mut slot.state {
            let f = &mut fade.as_mut().unwrap().0;
            *f = (*f + step).min(1.0);
            if *f >= 1.0 {
                commands.entity(*e).insert(MeshMaterial3d(material.clone()));
                *fade = None;
            }
        }
        if let NodeState::Loading(task) = &mut slot.state {
            if let Some(built) = check_ready(task) {
                let mut fade = None;
                let entity = built.mesh.map(|m| {
                    let own = far_mats.get(&material).map(|s| s.uniforms).map(|mut u| {
                        u.cam_pos.w = 1.0;
                        far_mats.add(GalaxyFarMaterial::new(u, GALAXY_FAR_BIAS))
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
                            ProcGalaxyNode { center_pc: key.center() },
                        ))
                        .id()
                });
                let max_radius = built.galaxies.iter().map(|g| g.display_radius_pc()).fold(0.0, f64::max);
                slot.state = NodeState::Ready { entity, galaxies: built.galaxies, max_radius, drawn: built.drawn, fade };
                field.node_gen += 1;
            }
        }
        if let NodeState::Ready { galaxies, drawn, .. } = &slot.state {
            n_gal += galaxies.len();
            n_drawn += *drawn;
            n_ready += 1;
        }
    }
    field.loaded_galaxies = n_gal;
    field.drawn_sprites = n_drawn;
    field.loaded_nodes = n_ready;

    // Drop stale nodes.
    let stale: Vec<GalNodeKey> = field
        .nodes
        .iter()
        .filter(|(_, s)| matches!(s.state, NodeState::Ready { .. }) && frame_no - s.last_used > KEEP_FRAMES)
        .map(|(k, _)| *k)
        .collect();
    // SE's CacheSizeGalNode: beyond it the least recently wanted nodes go first.
    let cap = memory.map_or(usize::MAX, |m| m.gal_nodes);
    let over = crate::memory::lru_victims(
        field.nodes.len() - stale.len(),
        cap,
        field.nodes.iter().filter(|(_, s)| matches!(s.state, NodeState::Ready { .. }) && s.last_used < frame_no && frame_no - s.last_used <= KEEP_FRAMES).map(|(k, s)| (*k, s.last_used)),
    );
    for k in stale.into_iter().chain(over) {
        field.node_gen += 1;
        if let Some(NodeSlot { state: NodeState::Ready { entity: Some(e), .. }, .. }) = field.nodes.remove(&k) {
            commands.entity(e).despawn();
        }
    }
    if frame_no % 300 == 0 {
        info!(
            "procgalaxies: {} of {} nodes ready, {} galaxies ({} sprites), sky coverage {:.2e}, gain {:.1}",
            field.loaded_nodes,
            field.nodes.len(),
            field.loaded_galaxies,
            field.drawn_sprites,
            gs.coverage,
            gs.gain
        );
    }
}

fn position_nodes(cam: Res<FlyCamera>, mut q: Query<(&ProcGalaxyNode, &mut Transform)>) {
    let cam_pc = cam.pos.to_parsecs();
    for (node, mut tf) in &mut q {
        // Only when it moves (bit for bit): a changed transform re-extracts the node's mesh.
        let t = (node.center_pc - cam_pc).as_vec3();
        if tf.translation.to_array().map(f32::to_bits) != t.to_array().map(f32::to_bits) {
            tf.translation = t;
        }
    }
}

/// Adds a procedural galaxy to the catalog (idempotent) and remembers its far-sprite id.
fn promote(field: &mut ProcGalaxyField, cat: &mut Catalog, g: &ProcGalaxy) -> u32 {
    if let Some(&i) = field.promoted.get(&(g.node, g.index)) {
        return i;
    }
    let idx = cat.add_galaxy_record(galaxy_record(g));
    field.promoted.insert((g.node, g.index), idx);
    field.sprite_id.insert(idx, sprite_id(g.node, g.index));
    idx
}

/// Procedural galaxies that grow large on screen become catalog galaxies, so `galaxies.rs` builds their models
/// (ellipticals get the volume of `ellipticals.rs`), clicks and the HUD find them and stars fill them.
fn promote_near(
    cam: Res<FlyCamera>,
    windows: Query<&Window, With<PrimaryWindow>>,
    settings: Res<ProcSettings>,
    gal_settings: Res<GalaxySettings>,
    star_settings: Res<crate::stars::StarSettings>,
    mut data: ResMut<SeData>,
    field: Option<ResMut<ProcGalaxyField>>,
) {
    let Some(mut field) = field else { return };
    if !settings.galaxies {
        return;
    }
    let field = &mut *field;
    if field.frame_no % 4 != 2 {
        return;
    }
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();
    let px_min = gal_settings.build_px as f64 * 0.7;
    // Zoomed in, galaxies all around grow past the threshold: only those in view are promoted.
    let cone = star_settings.view_cone(&cam, crate::stars::window_aspect(&windows));
    let key = PromoteKey {
        cam: cam_pc.to_array().map(f64::to_bits),
        focal: focal.to_bits(),
        px_min: px_min.to_bits(),
        cone: cone.map(|(a, h)| (a.to_array().map(f64::to_bits), h.to_bits())),
        node_gen: field.node_gen,
    };
    if field.idle_promote == Some(key) {
        return;
    }
    let in_view = |p: DVec3, r: f64| cone.is_none_or(|k| se_core::star_octree::sphere_meets_cone(cam_pc, k, p, r));
    let mut found: Vec<ProcGalaxy> = Vec::new();
    for (key, slot) in &field.nodes {
        let NodeState::Ready { galaxies, max_radius, .. } = &slot.state else { continue };
        // Node too far for its largest galaxy to reach the threshold.
        if angular_px(*max_radius, key.distance_to(cam_pc).max(1.0), focal) < px_min {
            continue;
        }
        for g in galaxies.iter() {
            // Cheap size test first; the hash lookup only for the few large galaxies.
            let d2 = (g.pos_pc - cam_pc).length_squared();
            let r = g.display_radius_pc();
            if d2 > r * r * (focal / px_min).powi(2) * 1.0001 + r * r {
                continue;
            }
            let px = angular_px(r, d2.sqrt(), focal);
            if px > px_min && in_view(g.pos_pc, r) && !field.promoted.contains_key(&(g.node, g.index)) {
                found.push(g.clone());
            }
        }
    }
    found.sort_by(|a, b| (a.pos_pc - cam_pc).length().total_cmp(&(b.pos_pc - cam_pc).length()));
    found.truncate(4);
    field.idle_promote = found.is_empty().then_some(key);
    if found.is_empty() {
        return;
    }
    let t = std::time::Instant::now();
    let shared = Arc::strong_count(&data.catalog) > 1;
    let cat = data.catalog_mut();
    for g in &found {
        let i = promote(field, cat, g);
        info!("procedural galaxy {} -> catalog {} ({}, {:.0} pc)", g.name(), i, g.kind(), g.display_radius_pc());
    }
    if shared {
        warn!("the catalog was shared while adding procedural galaxies: copied in {:.0?}", t.elapsed());
    }
}

/// Resolves an `RG a-level-node-index` name: generates the node if needed and adds the galaxy to the catalog.
pub fn ensure_galaxy(data: &mut SeData, field: &mut ProcGalaxyField, pools: &[u32; 17], name: &str) -> Option<ObjectRef> {
    let (key, idx) = proc_galaxy::parse_galaxy_name(name)?;
    if let Some(r) = data.catalog.find(&proc_galaxy::galaxy_name(key, idx)) {
        return Some(r);
    }
    let g = match field.nodes.get(&key) {
        Some(NodeSlot { state: NodeState::Ready { galaxies, .. }, .. }) => galaxies.get(idx as usize).cloned(),
        _ => proc_galaxy::generate_node(key, pools).into_iter().nth(idx as usize),
    }?;
    let cat = data.catalog_mut();
    let i = promote(field, cat, &g);
    Some(ObjectRef::Galaxy(i))
}

/// The sky covered by the drawn procedural galaxies (`galaxies::sky_coverage`, those not in the catalog
/// yet), for SE's intergalactic vision; `galaxies.rs` adds the catalog galaxies and eases the sum.
#[derive(Default)]
struct AutoGainState {
    task: Option<Task<f32>>,
}

fn auto_gain(
    cam: Res<FlyCamera>,
    settings: Res<ProcSettings>,
    field: Option<Res<ProcGalaxyField>>,
    gs: Option<ResMut<GalaxyState>>,
    mut st: Local<AutoGainState>,
) {
    let (Some(field), Some(mut gs)) = (field, gs) else { return };
    if let Some(task) = st.task.as_mut() {
        if let Some(c) = check_ready(task) {
            gs.proc_coverage = c;
            st.task = None;
        }
    }
    if st.task.is_some() || field.frame_no % 10 != 5 {
        return;
    }
    let nodes: Vec<Arc<Vec<ProcGalaxy>>> = if settings.galaxies {
        field
            .nodes
            .values()
            .filter_map(|s| match &s.state {
                NodeState::Ready { galaxies, .. } => Some(galaxies.clone()),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    let promoted: std::collections::HashSet<(GalNodeKey, u32)> = field.promoted.keys().copied().collect();
    let cam_pc = cam.pos.to_parsecs();
    st.task = Some(AsyncComputeTaskPool::get().spawn(async move {
        let mut c = 0.0;
        for g in nodes.iter().flat_map(|n| n.iter()) {
            let d = (g.pos_pc - cam_pc).length();
            if !promoted.contains(&(g.node, g.index)) && proc_galaxy::app_mag(g.lum, d as f32) <= GALAXY_MAG_CULL {
                c += crate::galaxies::sky_coverage(g.display_radius_pc(), d);
            }
        }
        c as f32
    }));
}

/// Copies the catalog galaxies' far-sprite uniforms (brightness, gain, daylight fade) and maps the resident
/// models of promoted procedural galaxies onto their own sprites.
fn sync_material(
    field: Option<Res<ProcGalaxyField>>,
    gs: Option<Res<GalaxyState>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
) {
    let (Some(field), Some(gs)) = (field, gs) else { return };
    let Some(src) = far_mats.get(&gs.far_handle()).map(|m| m.uniforms) else { return };
    let mut u = src;
    u.cam_pos = Vec4::ZERO;
    u.resident = [Vec4::new(-1.0, 0.0, 0.0, 0.0); RESIDENT_SLOTS];
    let mut n = 0;
    for &(g, fade) in &gs.resident {
        if let Some(&id) = field.sprite_id.get(&(g as u32)) {
            if n < RESIDENT_SLOTS {
                u.resident[n] = Vec4::new(id, fade, 0.0, 0.0);
                n += 1;
            }
        }
    }
    // Only changes are written: the shared material is drawn by every streamed node.
    set_far_uniforms(&mut far_mats, &field.material, &u);
    for slot in field.nodes.values() {
        if let NodeState::Ready { fade: Some((f, own)), .. } = &slot.state {
            let mut own_u = u;
            own_u.cam_pos.w = 1.0 - f;
            set_far_uniforms(&mut far_mats, own, &own_u);
        }
    }
}
