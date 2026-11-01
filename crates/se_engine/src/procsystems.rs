//! Procedural star clusters (`RC`) and nebulae (`RN`) inside galaxies, from `se_core::proc_galsys`.
//!
//! The Milky Way and every galaxy the camera is close to own one generated system (a list of clusters and a
//! list of nebulae in the galaxy's model frame). Both lists are drawn as far sprites (the galaxy far-sprite
//! shader, soft discs) through one mesh per system, with the entity translation `centre - camera`; systems
//! with more than [`MAX_OBJECTS`] objects keep only the brightest as seen from the camera and rebuild when
//! it moves. Objects the camera gets close to are added to the catalog ([`ClusterRecord`] / [`NebulaRecord`]),
//! after which `clusters.rs` generates their member stars and `nebulae.rs` builds their 3D sprite models.
//!
//! Names: `RC <galaxy>-<index>` and `RN <galaxy>-<index>` (galaxy = SE catalog index, 8514 for the Milky
//! Way) or `RC 0-<level>-<node>-<galaxy index>-<index>` for procedural galaxies. [`resolve_names`] turns
//! them into catalog objects when a command, the search box or a saved place mentions them.
//! Switches: [`ProcSettings`] (`ShowProcClusters`, `ShowProcNebulae`), env `SE_NO_PROCGAL=1` disables all.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{Catalog, ClusterRecord, NebulaRecord, ObjectRef};
use se_core::galaxy_model::hubble_type_index;
use se_core::proc_galaxy::parse_galaxy_name;
use se_core::proc_galsys::{
    self, AnalyticSpiral, ClusterClass, DeepSkyConfig, EmissionMap, GalaxyRef, GalaxyShape, GalaxySystem, MILKY_WAY_SE_INDEX, NebulaClass, SystemKind,
};
use se_core::star_octree::GalaxyFrame;

use crate::camera::{CommandQueue, FlyCamera};
use crate::data::{Places, SeData};
use crate::galaxies::{
    ATTR_FAR_CORNER, ATTR_FAR_DATA, ATTR_FAR_SHAPE, GALAXY_FAR_BIAS, GalaxyDensity, GalaxyFarMaterial, GalaxySettings, GalaxyState, NEBULA_FAR_BIAS,
    RESIDENT_SLOTS, angular_px, far_uniforms, pad_aabb,
};
use crate::hud::HudState;
use crate::nebulae::{NebulaSettings, NebulaState, ResidentNebulae};
use crate::procgalaxies::{ProcGalaxyField, ProcSettings, ensure_galaxy};
use crate::universe::UniverseSet;

/// Sprites per system and kind.
const MAX_OBJECTS: usize = 20_000;
/// Objects fainter than this from the camera are not drawn.
const MAG_CULL: f32 = 24.0;
/// Systems alive at once.
const MAX_SYSTEMS: usize = 3;
/// Size (px) of a cluster's star-like far particle (catalog and procedural).
const CLUSTER_MIN_PX: f32 = 1.5;
/// A galaxy's system is active while the camera is within this many radii (hysteresis x1.5).
const ACTIVE_RADII: f64 = 3.0;
/// Frames after which the Milky Way system stops waiting for its emission map.
const FALLBACK_FRAMES: u64 = 2400;
/// Nebulae become catalog entries (3D models) at this angular radius, pixels, ...
const NEBULA_PROMOTE_PX: f64 = 3.0;
/// ... and only when they are brighter than this (SE's `NebulaMagnLimit`/`GlowMagnLimit` are 7), so that the many
/// large but faint nebulae of a galaxy do not all get models.
const NEBULA_PROMOTE_MAG: f32 = 8.0;

/// Everything a generator task needs.
struct GenInput {
    shape: GalaxyShape,
    cfg: DeepSkyConfig,
    neb_pools: [u32; 4],
    emission: Arc<dyn EmissionMap + Send + Sync>,
    uses_map: bool,
    frame: GalaxyFrame,
}

/// Emission of the Milky Way from its model's voxel map.
struct MapEmission {
    density: GalaxyDensity,
    dims: [f32; 3],
}

impl EmissionMap for MapEmission {
    fn at(&self, u: [f32; 3]) -> f32 {
        // `u` is in units of the x extent; the map's z unit is the model's z scale.
        let z = u[2] * self.dims[0] / self.dims[2].max(1.0);
        self.density.relative_to_sun(Vec3::new(u[0], u[1], z)).clamp(0.0, 1.0)
    }
}

enum SlotState {
    Loading(Task<Arc<GalaxySystem>>),
    Ready(Arc<GalaxySystem>),
}

struct Slot {
    state: SlotState,
    frame: GalaxyFrame,
    dims_x: f32,
    uses_map: bool,
    last_used: u64,
    mesh_task: Option<Task<(Option<Mesh>, Option<Mesh>)>>,
    built_cam: Option<DVec3>,
    /// `[clusters, nebulae]` mesh entities.
    entities: [Option<Entity>; 2],
}

#[derive(Component)]
struct SystemSprites {
    center_pc: DVec3,
}

#[derive(Resource)]
pub struct ProcSystems {
    slots: HashMap<u32, Slot>,
    /// Generated systems kept for name lookups (SE's `CacheSizeGalSysModels`, the galaxy subsystems
    /// kept in memory), with the frame each was last used; the least recently used go first.
    cache: HashMap<u32, (Arc<GalaxySystem>, bool)>,
    cache_used: HashMap<u32, u64>,
    cache_cap: usize,
    clu_mat: Handle<GalaxyFarMaterial>,
    neb_mat: Handle<GalaxyFarMaterial>,
    /// The far particles of the catalog clusters (always on: SE's `ShowCatClusters`).
    cat_clu_mat: Handle<GalaxyFarMaterial>,
    frame_no: u64,
    /// `(galaxy, cluster index) -> catalog cluster`, and the same for nebulae.
    promoted_c: HashMap<(u32, u32), u32>,
    promoted_n: HashMap<(u32, u32), u32>,
    /// Catalog nebula -> far-sprite id (for the model fade).
    neb_sprite: HashMap<u32, f32>,
    places_done: bool,
    wanted: Vec<u32>,
}

pub struct ProcSystemsPlugin;

impl Plugin for ProcSystemsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(EguiPrimaryContextPass, settings_window.in_set(crate::skin::UiSet::Draw))
            .add_systems(PreUpdate, resolve_names)
            .add_systems(
                PostUpdate,
                (manage_systems, position_sprites, promote_objects, sync_materials)
                    .chain()
                    .after(UniverseSet::Camera)
                    .before(UniverseSet::FloatingOrigin),
            );
    }
}

/// The young clusters of the catalog nebulae that make stars (`GenerateStars`, on for diffuse ones):
/// SE's stars inside nebulae, `RSN <galaxy>-<nebula>-<index>`, the nebula numbered among its galaxy's
/// catalog nebulae as in `RN` names. Their clusters are catalog entries of their own, drawn only
/// through their member stars.
fn add_nebula_clusters(cat: &mut Catalog, cfg: &DeepSkyConfig) {
    let mut per_galaxy: HashMap<String, u32> = HashMap::new();
    let mut add = Vec::new();
    for n in cat.nebulae.iter().filter(|n| n.source != "procedural") {
        let k = per_galaxy.entry(n.galaxy.clone()).or_insert(0);
        let index = *k;
        *k += 1;
        if !n.generate_stars || n.radius_pc <= 0.0 {
            continue;
        }
        let g = match cat.find(&n.galaxy) {
            Some(ObjectRef::Galaxy(g)) => g,
            _ => continue,
        };
        let e = proc_galsys::catalog_embedded(cfg, [n.pos_pc.x as f32, n.pos_pc.y as f32, n.pos_pc.z as f32], n.radius_pc as f32);
        add.push(ClusterRecord {
            names: vec![format!("{} cluster", n.names[0])],
            galaxy: n.galaxy.clone(),
            kind: "Open".into(),
            pos_pc: n.pos_pc,
            radius_pc: e.radius_pc as f64,
            abs_mag: proc_galsys::abs_mag_of_lum(e.lum) as f64,
            age_gyr: e.age_gyr as f64,
            cen_pow: e.cen_pow as f64,
            no_members: false,
            color: None,
            source: "procedural".into(),
            embedded_in: Some(proc_galsys::system_name(SystemKind::Nebula, galaxy_code(cat, g), index)),
        });
    }
    let n = add.len();
    for rec in add {
        cat.add_cluster_record(rec);
    }
    info!("{n} catalog nebulae hold young clusters of RSN stars");
}

fn setup(
    mut commands: Commands,
    mut data: ResMut<SeData>,
    engine: Res<crate::config::EngineConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
    memory: Option<Res<crate::memory::MemoryLimits>>,
) {
    let deep = std::fs::read_to_string(engine.se_path.join("config/main-def.cfg")).map_or_else(|_| DeepSkyConfig::default(), |t| DeepSkyConfig::from_cfg(&t));
    add_nebula_clusters(data.catalog_mut(), &deep);
    let gs = GalaxySettings::default();
    let clu_mat = far_mats.add(GalaxyFarMaterial { uniforms: far_uniforms(Vec3::ZERO, &gs), depth_bias: GALAXY_FAR_BIAS });
    let neb_mat = far_mats.add(GalaxyFarMaterial { uniforms: far_uniforms(Vec3::ZERO, &gs), depth_bias: NEBULA_FAR_BIAS });
    let cat_clu_mat = far_mats.add(GalaxyFarMaterial { uniforms: far_uniforms(Vec3::ZERO, &gs), depth_bias: GALAXY_FAR_BIAS });
    // SE draws every catalog cluster as a star-like particle (`FUN_140496f00`), not only once its
    // member stars exist: one mesh at absolute positions, translated by minus the camera.
    let particles: Vec<Sprite> = data
        .catalog
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| c.source != "procedural")
        .filter_map(|(i, c)| {
            let kind = se_core::catalog::ClusterType::parse(&c.kind);
            let open = matches!(kind, se_core::catalog::ClusterType::Open | se_core::catalog::ClusterType::Part);
            let color = match c.color {
                // `Color (0 0 0)`: no particle, the stars are in the star catalog (Pleiades, Hyades).
                Some(rgb) if rgb.iter().all(|&x| x <= 0.0) => return None,
                Some(rgb) => particle_color(rgb, open),
                None => particle_color(if open { [0.7, 0.8, 1.0] } else { [1.0, 0.9, 0.7] }, open),
            };
            Some(Sprite { rel: c.pos_pc.as_vec3(), color, abs_mag: c.abs_mag as f32, radius: c.radius_pc.max(0.05) as f32, ptype: 4.0, id: i as f32 })
        })
        .collect();
    if let Some(mesh) = sprite_mesh(&particles) {
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(cat_clu_mat.clone()),
            Transform::IDENTITY,
            Visibility::Visible,
            NoFrustumCulling,
            SystemSprites { center_pc: DVec3::ZERO },
        ));
    }
    commands.insert_resource(ProcSystems {
        slots: HashMap::new(),
        cache: HashMap::new(),
        cache_used: HashMap::new(),
        cache_cap: memory.map_or(32, |m| m.gal_sys_models),
        clu_mat,
        neb_mat,
        cat_clu_mat,
        frame_no: 0,
        promoted_c: HashMap::new(),
        promoted_n: HashMap::new(),
        neb_sprite: HashMap::new(),
        places_done: false,
        wanted: Vec::new(),
    });
}

/// The `Show*` switches of the procedural deep sky, next to the Settings window.
fn settings_window(
    mut contexts: EguiContexts,
    hud: Res<HudState>,
    mut settings: ResMut<ProcSettings>,
    mut clusters: ResMut<crate::clusters::ClusterSettings>,
    field: Option<Res<ProcGalaxyField>>,
) -> Result {
    if !hud.show_settings || hud.hidden {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    egui::Window::new(crate::skin::title(ctx, "Procedural deep sky")).default_pos([320.0, 40.0]).show(ctx, |ui| {
        ui.checkbox(&mut settings.galaxies, "Procedural galaxies (ShowProcGalaxies)");
        ui.checkbox(&mut clusters.show_cat, "Catalog star clusters (ShowCatClusters)");
        ui.checkbox(&mut settings.clusters, "Procedural star clusters (ShowProcClusters)");
        ui.checkbox(&mut settings.nebulae, "Procedural nebulae (ShowProcNebulae)");
        if let Some(f) = field {
            ui.label(format!("{} galaxy nodes, {} galaxies loaded", f.loaded_nodes(), f.loaded_galaxies()));
        }
    });
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------------------------

fn lum_of(abs_mag: f64) -> f32 {
    10f32.powf((4.83 - abs_mag as f32) * 0.4)
}

fn galaxy_code(cat: &Catalog, g: u32) -> GalaxyRef {
    let rec = &cat.galaxies[g as usize];
    if rec.source == "procedural" {
        if let Some((key, idx)) = rec.names.first().and_then(|n| parse_galaxy_name(n)) {
            return GalaxyRef::Procedural(key, idx);
        }
    }
    if rec.names.iter().any(|n| n == "Milky Way") {
        GalaxyRef::Catalog(MILKY_WAY_SE_INDEX)
    } else {
        GalaxyRef::Catalog(g)
    }
}

fn galaxy_name_of(cat: &Catalog, g: u32) -> String {
    cat.galaxies[g as usize].names.first().cloned().unwrap_or_default()
}

/// What SE's galaxy record holds of catalog galaxy `g` for its generators: `None` for quasars and
/// galaxies without a radius.
struct Basics {
    hubble: u8,
    /// The record radius (rec +0x20): an elliptical's catalog radius is 2.4 times it.
    radius: f32,
    /// Half extents of the box (rec +0x40..+0x48).
    dims: [f32; 3],
    seed_pos: [f32; 3],
    model_scale: Option<[f32; 3]>,
}

fn basics(g: u32, cat: &Catalog, gs: &GalaxyState) -> Option<Basics> {
    let rec = cat.galaxies.get(g as usize)?;
    let hubble = hubble_type_index(&rec.kind);
    let r = rec.radius_pc as f32;
    if rec.quasar || !(r > 0.0) || !r.is_finite() {
        return None;
    }
    let model_scale = if hubble >= 9 { gs.model_scale_of(rec) } else { None };
    let (radius, dims) = if hubble < 9 {
        // E galaxies: the record radius is the 2.4x visual radius; En has axis ratio 1 - n/10.
        let e = r;
        let f = 1.0 - (hubble as f32 - 1.0) / 10.0;
        (r / 2.4, [f * e, e, f * e])
    } else if let Some(s) = model_scale {
        (r, [r * s[0], r * s[1], r * s[2]])
    } else {
        (r, [r, r, r])
    };
    let seed_pos = if rec.source == "procedural" {
        let key = rec.names.first().and_then(|n| parse_galaxy_name(n)).map(|(k, _)| k)?;
        let off = rec.pos_pc - key.center();
        [off.x as f32, off.y as f32, off.z as f32]
    } else {
        [rec.pos_pc.x as f32, rec.pos_pc.y as f32, rec.pos_pc.z as f32]
    };
    Some(Basics { hubble, radius, dims, seed_pos, model_scale })
}

/// What SE's star octree reads of catalog galaxy `g` (FUN_14053b5d0): its box, radius, type, model
/// and seed, and its name part.
pub(crate) fn star_input(g: u32, cat: &Catalog, gs: &GalaxyState) -> Option<se_core::star_octree::GalaxyStarInput> {
    let b = basics(g, cat, gs)?;
    let shape = GalaxyShape {
        seed_pos: b.seed_pos,
        hubble: b.hubble,
        radius_pc: b.radius,
        lum: 0.0,
        compact: false,
        dims: b.dims,
        has_model: b.model_scale.is_some(),
        home: None,
        catalog_clusters: 0,
        catalog_nebulae: 0,
        glob_clusters: None,
    };
    Some(se_core::star_octree::GalaxyStarInput {
        dims: DVec3::new(b.dims[0] as f64, b.dims[1] as f64, b.dims[2] as f64),
        radius_pc: b.radius as f64,
        hubble: b.hubble,
        has_model: b.model_scale.is_some(),
        seed: shape.seed(),
        galaxy: galaxy_code(cat, g),
    })
}

/// The generator input of catalog galaxy `g`, or `None` when it cannot hold clusters/nebulae (quasars, odd types).
///
/// The Milky Way follows the emission map of its sprite model, which is built in the background at start-up; its system
/// waits for it (`fallback` allows the analytic spiral after a while).
fn gen_input(g: u32, cat: &Catalog, gs: &GalaxyState, ns: &NebulaState, density: &GalaxyDensity, cfg: &DeepSkyConfig, fallback: bool) -> Option<GenInput> {
    let rec = cat.galaxies.get(g as usize)?;
    let hubble = hubble_type_index(&rec.kind);
    if hubble == 0 {
        return None;
    }
    let b = basics(g, cat, gs)?;
    let is_mw = rec.names.iter().any(|n| n == "Milky Way");
    let frame = GalaxyFrame { center_pc: rec.pos_pc, rot: rec.orientation };
    let model_scale = b.model_scale;
    if is_mw && model_scale.is_some() && !density.has_map() && !fallback {
        return None;
    }
    let (lum_radius, dims, seed_pos) = (b.radius, b.dims, b.seed_pos);
    // The generated objects are numbered after the catalog's own: the promoted ones (`source`
    // "procedural") must not count, or every regeneration would renumber them and promote them again.
    let count = |name: &str| cat.clusters.iter().filter(|c| c.galaxy == name && c.source != "procedural").count() as u32;
    let count_n = |name: &str| cat.nebulae.iter().filter(|c| c.galaxy == name && c.source != "procedural").count() as u32;
    let (cc, cn) = if is_mw { (count("Milky Way"), count_n("Milky Way")) } else { (0, 0) };
    let home = is_mw.then(|| {
        let h = frame.to_model(DVec3::ZERO);
        [h.x as f32, h.y as f32, h.z as f32]
    });
    let shape = GalaxyShape {
        seed_pos,
        hubble,
        radius_pc: lum_radius,
        lum: lum_of(rec.abs_mag),
        compact: false,
        dims,
        has_model: model_scale.is_some(),
        home,
        catalog_clusters: cc,
        catalog_nebulae: cn,
        glob_clusters: cat.glob_clusters.get(&g).copied(),
    };
    let use_map = is_mw && density.has_map() && model_scale.is_some();
    let emission: Arc<dyn EmissionMap + Send + Sync> = if use_map { Arc::new(MapEmission { density: density.clone(), dims }) } else { Arc::new(AnalyticSpiral) };
    Some(GenInput {
        shape,
        cfg: cfg.clone(),
        neb_pools: [0, ns.pool_len("Diffuse"), ns.pool_len("Planetary"), ns.pool_len("SNR")],
        emission,
        uses_map: use_map,
        frame,
    })
}

fn run_generate(input: &GenInput) -> Arc<GalaxySystem> {
    Arc::new(proc_galsys::generate_system(&input.shape, &input.cfg, input.emission.as_ref(), input.neb_pools))
}

// ---------------------------------------------------------------------------------------------
// Meshes
// ---------------------------------------------------------------------------------------------

struct Sprite {
    rel: Vec3,
    color: [f32; 3],
    abs_mag: f32,
    radius: f32,
    ptype: f32,
    id: f32,
}

fn sprite_mesh(items: &[Sprite]) -> Option<Mesh> {
    if items.is_empty() {
        return None;
    }
    let (mut p, mut d, mut s, mut c, mut ix) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for it in items {
        let base = p.len() as u32;
        for k in [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            p.push(it.rel.to_array());
            d.push([it.color[0], it.color[1], it.color[2], it.abs_mag]);
            s.push([0.0, 0.0, 0.0, it.radius]);
            c.push([k[0], k[1], it.id, it.ptype]);
        }
        ix.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
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
}

/// Colour of a cluster's far particle: open clusters at a quarter of the brightness
/// (`FUN_1404826a0` record +0x38: 0.25 open, 1 globular and nucleus).
fn particle_color(rgb: [f32; 3], open: bool) -> [f32; 3] {
    let k = if open { 0.25 } else { 1.0 };
    rgb.map(|x| x * k)
}

fn neb_color(c: NebulaClass) -> [f32; 3] {
    match c {
        NebulaClass::Planetary => [0.45, 0.95, 0.85],
        NebulaClass::Snr => [1.0, 0.6, 0.65],
        NebulaClass::Diffuse => [1.0, 0.6, 0.75],
    }
}

fn neb_sprite_id(g: u32, index: u32) -> f32 {
    ((g.wrapping_mul(0x9E37_79B1) ^ index.wrapping_mul(0x85EB_CA6B)) % 16_000_000) as f32
}

/// The brightest [`MAX_OBJECTS`] clusters and nebulae of a system as seen from `cam`.
fn build_meshes(sys: &GalaxySystem, frame: GalaxyFrame, cam: DVec3, g: u32) -> (Option<Mesh>, Option<Mesh>) {
    let mag = |abs: f32, pos: [f32; 3]| -> (f32, Vec3) {
        let rel = (frame.rot * DVec3::new(pos[0] as f64, pos[1] as f64, pos[2] as f64)).as_vec3();
        let d = (frame.center_pc + rel.as_dvec3() - cam).length().max(1.0) as f32;
        (abs + 5.0 * (d.log10() - 1.0), rel)
    };
    let mut clusters: Vec<(f32, Sprite)> = Vec::new();
    for c in &sys.clusters {
        let abs = proc_galsys::abs_mag_of_lum(c.lum);
        let (m, rel) = mag(abs, c.pos);
        if m < MAG_CULL {
            clusters.push((m, Sprite { rel, color: particle_color(c.color, c.class == ClusterClass::Open), abs_mag: abs, radius: c.radius_pc.max(0.05), ptype: 4.0, id: c.index as f32 }));
        }
    }
    let mut nebulae: Vec<(f32, Sprite)> = Vec::new();
    for n in &sys.nebulae {
        let abs = proc_galsys::abs_mag_of_lum(n.lum);
        let (m, rel) = mag(abs, n.pos);
        if m < MAG_CULL {
            nebulae.push((m, Sprite { rel, color: neb_color(n.class), abs_mag: abs, radius: n.radius_pc.max(0.01), ptype: 2.0, id: neb_sprite_id(g, n.index) }));
        }
    }
    if std::env::var_os("SE_PROC_DEBUG").is_some() {
        let mut best: Vec<(f32, f32, f32, f32)> = clusters.iter().map(|(m, s)| (*m, s.rel.length(), s.radius, s.id)).collect();
        best.sort_by(|a, b| a.0.total_cmp(&b.0));
        info!("brightest clusters (mag, |rel| pc, radius pc): {:?}", &best[..best.len().min(6)]);
        let mut best: Vec<(f32, f32, f32, f32)> = nebulae.iter().map(|(m, s)| (*m, s.rel.length(), s.radius, s.id)).collect();
        best.sort_by(|a, b| a.0.total_cmp(&b.0));
        info!("brightest nebulae (mag, |rel| pc, radius pc): {:?}", &best[..best.len().min(6)]);
    }
    let top = |mut v: Vec<(f32, Sprite)>| -> Vec<Sprite> {
        if v.len() > MAX_OBJECTS {
            v.select_nth_unstable_by(MAX_OBJECTS, |a, b| a.0.total_cmp(&b.0));
            v.truncate(MAX_OBJECTS);
        }
        v.into_iter().map(|(_, s)| s).collect()
    };
    let (mut clusters, mut nebulae) = (top(clusters), top(nebulae));
    // Debug aid: SE_PROC_DEBUG paints clusters green and nebulae red and makes them 4x larger.
    if std::env::var_os("SE_PROC_DEBUG").is_some() {
        for s in &mut clusters {
            s.color = [0.0, 1.0, 0.0];
            s.radius *= 4.0;
        }
        for s in &mut nebulae {
            s.color = [1.0, 0.0, 0.0];
            s.radius *= 4.0;
        }
    }
    debug!("procedural system {g}: {} cluster and {} nebula sprites", clusters.len(), nebulae.len());
    (sprite_mesh(&clusters), sprite_mesh(&nebulae))
}

// ---------------------------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn manage_systems(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<ProcSettings>,
    density: Res<GalaxyDensity>,
    gs: Option<Res<GalaxyState>>,
    ns: Option<Res<NebulaState>>,
    sys: Option<ResMut<ProcSystems>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Some(mut sys), Some(gs), Some(ns)) = (sys, gs, ns) else { return };
    let sys = &mut *sys;
    sys.frame_no += 1;
    let frame_no = sys.frame_no;
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    let any_on = settings.clusters || settings.nebulae;

    // Which galaxies want a system (the Milky Way, and galaxies the camera is inside or near).
    if frame_no % 10 == 1 {
        let mut near: Vec<(f64, u32)> = Vec::new();
        if any_on {
            for (i, g) in cat.galaxies.iter().enumerate() {
                let is_mw = g.names.iter().any(|n| n == "Milky Way");
                let d = (g.pos_pc - cam_pc).length();
                let limit = g.radius_pc * ACTIVE_RADII * if sys.slots.contains_key(&(i as u32)) { 1.5 } else { 1.0 };
                // The Milky Way is generated early so that `RC 8514-n` names resolve and its sprites are ready.
                if (is_mw && d < 5.0e5) || (d < limit && g.radius_pc > 100.0 && !g.quasar) {
                    near.push((d / g.radius_pc.max(1.0), i as u32));
                }
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.truncate(MAX_SYSTEMS);
        sys.wanted = near.into_iter().map(|(_, i)| i).collect();
    }
    let wanted = sys.wanted.clone();
    for &g in &wanted {
        if let Some(slot) = sys.slots.get_mut(&g) {
            slot.last_used = frame_no;
            // The Milky Way was built before its emission map existed: redo it once with the real map.
            if g as usize == cat.galaxies.iter().position(|r| r.names.iter().any(|n| n == "Milky Way")).unwrap_or(usize::MAX)
                && !slot.uses_map
                && density.has_map()
                && matches!(slot.state, SlotState::Ready(_))
            {
                for e in slot.entities.iter().flatten() {
                    commands.entity(*e).despawn();
                }
                sys.slots.remove(&g);
                sys.cache.remove(&g);
                sys.promoted_c.retain(|k, _| k.0 != g);
                sys.promoted_n.retain(|k, _| k.0 != g);
            }
            continue;
        }
        let Some(input) = gen_input(g, cat, &gs, &ns, &density, &settings.cfg, frame_no > FALLBACK_FRAMES) else { continue };
        let (frame, dims_x, uses_map) = (input.frame, input.shape.dims[0], input.uses_map);
        let task = AsyncComputeTaskPool::get().spawn(async move { run_generate(&input) });
        sys.slots.insert(
            g,
            Slot { state: SlotState::Loading(task), frame, dims_x, uses_map, last_used: frame_no, mesh_task: None, built_cam: None, entities: [None, None] },
        );
    }

    // Poll generation and mesh builds.
    let keys: Vec<u32> = sys.slots.keys().copied().collect();
    for g in keys {
        let slot = sys.slots.get_mut(&g).unwrap();
        if let SlotState::Loading(task) = &mut slot.state {
            if let Some(system) = check_ready(task) {
                info!(
                    "procedural system of {}: {} clusters, {} nebulae",
                    galaxy_name_of(cat, g),
                    system.clusters.len(),
                    system.nebulae.len()
                );
                cache_put(&mut sys.cache, &mut sys.cache_used, (sys.cache_cap, sys.frame_no), g, (system.clone(), slot.uses_map), &wanted);
                slot.state = SlotState::Ready(system);
            }
        }
        let SlotState::Ready(system) = &slot.state else { continue };
        if let Some(task) = slot.mesh_task.as_mut() {
            if let Some((c, n)) = check_ready(task) {
                slot.mesh_task = None;
                for (i, mesh) in [c, n].into_iter().enumerate() {
                    if let Some(e) = slot.entities[i].take() {
                        commands.entity(e).despawn();
                    }
                    let mat = if i == 0 { sys.clu_mat.clone() } else { sys.neb_mat.clone() };
                    if let Some(m) = mesh {
                        slot.entities[i] = Some(
                            commands
                                .spawn((
                                    Mesh3d(meshes.add(m)),
                                    MeshMaterial3d(mat),
                                    Transform::IDENTITY,
                                    Visibility::Visible,
                                    NoFrustumCulling,
                                    SystemSprites { center_pc: slot.frame.center_pc },
                                ))
                                .id(),
                        );
                    }
                }
            }
        }
        // (Re)build the sprite meshes: once, and again when a big system's brightest set may have changed.
        let big = system.clusters.len().max(system.nebulae.len()) > MAX_OBJECTS;
        let moved = slot.built_cam.is_none_or(|b| big && b.distance(cam_pc) > slot.dims_x as f64 * 0.05);
        if slot.mesh_task.is_none() && moved {
            let (system, frame) = (system.clone(), slot.frame);
            slot.built_cam = Some(cam_pc);
            slot.mesh_task = Some(AsyncComputeTaskPool::get().spawn(async move { build_meshes(&system, frame, cam_pc, g) }));
        }
    }

    // Drop systems nobody wants (and everything when switched off).
    let stale: Vec<u32> = sys
        .slots
        .iter()
        .filter(|(g, s)| (!any_on || (frame_no - s.last_used > 60 && !wanted.contains(g))) && s.mesh_task.is_none())
        .map(|(g, _)| *g)
        .collect();
    for g in stale {
        if let Some(slot) = sys.slots.remove(&g) {
            for e in slot.entities.iter().flatten() {
                commands.entity(*e).despawn();
            }
        }
    }
}

fn position_sprites(cam: Res<FlyCamera>, mut q: Query<(&SystemSprites, &mut Transform)>) {
    let cam_pc = cam.pos.to_parsecs();
    for (node, mut tf) in &mut q {
        tf.translation = (node.center_pc - cam_pc).as_vec3();
    }
}

/// Copies the far-sprite uniforms of the catalog galaxies (clusters) and nebulae (nebulae) every frame.
fn sync_materials(
    sys: Option<Res<ProcSystems>>,
    gs: Option<Res<GalaxyState>>,
    ns: Option<Res<NebulaState>>,
    resident: Res<ResidentNebulae>,
    settings: Res<ProcSettings>,
    clusters: Option<Res<crate::clusters::ClusterSettings>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
) {
    let (Some(sys), Some(gs), Some(ns)) = (sys, gs, ns) else { return };
    let show_cat = clusters.is_none_or(|c| c.show_cat);
    let mut zero_hidden = |mat: &Handle<GalaxyFarMaterial>, src: Handle<GalaxyFarMaterial>, on: bool, res: &[(usize, f32)]| {
        let Some(mut u) = far_mats.get(&src).map(|m| m.uniforms) else { return };
        u.cam_pos = Vec4::ZERO;
        u.resident = [Vec4::new(-1.0, 0.0, 0.0, 0.0); RESIDENT_SLOTS];
        let ids = res.iter().filter_map(|&(c, fade)| sys.neb_sprite.get(&(c as u32)).map(|&id| Vec4::new(id, fade, 0.0, 0.0)));
        for (slot, v) in u.resident.iter_mut().zip(ids) {
            *slot = v;
        }
        if !on {
            u.params.x = 0.0;
        }
        // Intergalactic vision and the larger galaxy points are for galaxies only: they
        // would turn the clusters of a galaxy seen from outside into glitter.
        if *mat == sys.clu_mat || *mat == sys.cat_clu_mat {
            u.params.y = CLUSTER_MIN_PX;
        }
        u.params2.z = 1.0;
        if let Some(mut m) = far_mats.get_mut(mat) {
            m.uniforms = u;
        }
    };
    zero_hidden(&sys.clu_mat, gs.far_handle(), settings.clusters, &[]);
    zero_hidden(&sys.cat_clu_mat, gs.far_handle(), show_cat, &[]);
    zero_hidden(&sys.neb_mat, ns.far_handle(), settings.nebulae, &resident.0);
}

// ---------------------------------------------------------------------------------------------
// Catalog promotion
// ---------------------------------------------------------------------------------------------

fn cluster_record(cat: &Catalog, g: u32, c: &proc_galsys::ProcCluster, frame: GalaxyFrame) -> ClusterRecord {
    let kind = match c.class {
        ClusterClass::Globular => "Globular",
        ClusterClass::Kern => "Kern",
        ClusterClass::Open => "Open",
    };
    let pos = frame.center_pc + frame.rot * DVec3::new(c.pos[0] as f64, c.pos[1] as f64, c.pos[2] as f64);
    ClusterRecord {
        names: vec![proc_galsys::system_name(SystemKind::Cluster, galaxy_code(cat, g), c.index)],
        galaxy: galaxy_name_of(cat, g),
        kind: kind.into(),
        pos_pc: pos,
        radius_pc: c.radius_pc as f64,
        abs_mag: proc_galsys::abs_mag_of_lum(c.lum) as f64,
        age_gyr: c.age_gyr as f64,
        cen_pow: c.cen_pow as f64,
        no_members: false,
        color: None,
        source: "procedural".into(),
        embedded_in: c.nebula.map(|n| proc_galsys::system_name(SystemKind::Nebula, galaxy_code(cat, g), n)),
    }
}

fn nebula_record(cat: &Catalog, g: u32, n: &proc_galsys::ProcNebula, frame: GalaxyFrame) -> NebulaRecord {
    let pos = frame.center_pc + frame.rot * DVec3::new(n.pos[0] as f64, n.pos[1] as f64, n.pos[2] as f64);
    let q = n.quat;
    NebulaRecord {
        names: vec![proc_galsys::system_name(SystemKind::Nebula, galaxy_code(cat, g), n.index)],
        galaxy: galaxy_name_of(cat, g),
        kind: n.class.kind().into(),
        pos_pc: pos,
        radius_pc: n.radius_pc as f64,
        abs_mag: proc_galsys::abs_mag_of_lum(n.lum) as f64,
        orientation: frame.rot * DQuat::from_xyzw(q[1] as f64, q[2] as f64, q[3] as f64, q[0] as f64).normalize(),
        source: "procedural".into(),
        model_roll: n.model_roll,
        randomize: [0.0; 3],
        generate_stars: n.class == proc_galsys::NebulaClass::Diffuse,
    }
}

/// Clusters the camera is close to (member stars are generated by `clusters.rs`) and nebulae that look
/// large on screen (3D models by `nebulae.rs`) become catalog entries.
fn promote_objects(
    cam: Res<FlyCamera>,
    windows: Query<&Window, With<PrimaryWindow>>,
    settings: Res<ProcSettings>,
    neb_settings: Res<NebulaSettings>,
    star_settings: Res<crate::stars::StarSettings>,
    mut data: ResMut<SeData>,
    sys: Option<ResMut<ProcSystems>>,
) {
    let Some(mut sys) = sys else { return };
    let sys = &mut *sys;
    if sys.frame_no % 10 != 4 {
        return;
    }
    // Zoomed in, objects all around grow past the thresholds: only those in view are promoted.
    let cone = star_settings.view_cone(&cam, crate::stars::window_aspect(&windows));
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();
    // Nebulae are drawn only in view (`nebula_raymarch.rs`): promote those on screen, largest first, so
    // the ones off screen do not hold back the ones about to fade in.
    let aspect = crate::stars::window_aspect(&windows);
    let half_fov = (cam.fov_deg as f64 * 0.5).to_radians().tan();
    let screen = ((cam.rot * DVec3::NEG_Z).normalize(), (half_fov * (1.0 + aspect * aspect).sqrt()).atan());
    let mut new_c: Vec<(u32, proc_galsys::ProcCluster, GalaxyFrame)> = Vec::new();
    let mut new_n: Vec<(f64, u32, proc_galsys::ProcNebula, GalaxyFrame)> = Vec::new();
    for (&g, slot) in &sys.slots {
        let SlotState::Ready(system) = &slot.state else { continue };
        let rel_cam = slot.frame.rot.inverse() * (cam_pc - slot.frame.center_pc);
        let view = cone.map(|(axis, half)| (slot.frame.rot.inverse() * axis, half));
        let in_view = |p: [f32; 3], r: f64| {
            let p = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64);
            view.is_none_or(|k| se_core::star_octree::sphere_meets_cone(rel_cam, k, p, r))
        };
        let screen = (slot.frame.rot.inverse() * screen.0, screen.1);
        let on_screen = |p: [f32; 3], r: f64| {
            let p = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64);
            se_core::star_octree::sphere_meets_cone(rel_cam, screen, p, r)
        };
        let rel_cam = [rel_cam.x as f32, rel_cam.y as f32, rel_cam.z as f32];
        let d2 = |p: [f32; 3]| -> f64 {
            let (x, y, z) = (p[0] - rel_cam[0], p[1] - rel_cam[1], p[2] - rel_cam[2]);
            (x * x + y * y + z * z) as f64
        };
        if settings.clusters {
            for c in &system.clusters {
                // Where `clusters.rs` builds the member stars (8 px of radius on screen).
                let range = crate::clusters::members_range_pc(c.radius_pc as f64, cam.fov_deg as f64, height);
                if d2(c.pos) < range * range && in_view(c.pos, c.radius_pc as f64) && !sys.promoted_c.contains_key(&(g, c.index)) {
                    new_c.push((g, c.clone(), slot.frame));
                }
            }
        }
        if settings.nebulae {
            for n in &system.nebulae {
                let d = d2(n.pos).sqrt().max(1.0);
                let px = angular_px(n.radius_pc as f64, d, focal);
                // A model's bounding shape reaches up to ~1.7 nebula radii (a box's corners).
                if px > NEBULA_PROMOTE_PX.max(neb_settings.build_px as f64 * 0.7)
                    && proc_galsys::abs_mag_of_lum(n.lum) + 5.0 * ((d as f32).log10() - 1.0) < NEBULA_PROMOTE_MAG
                    && on_screen(n.pos, n.radius_pc as f64 * 2.0)
                    && !sys.promoted_n.contains_key(&(g, n.index))
                {
                    new_n.push((px, g, n.clone(), slot.frame));
                }
            }
        }
    }
    if new_c.is_empty() && new_n.is_empty() {
        return;
    }
    new_c.sort_by(|a, b| (a.1.radius_pc).total_cmp(&b.1.radius_pc));
    new_c.truncate(3);
    new_n.sort_by(|a, b| b.0.total_cmp(&a.0));
    new_n.truncate(3);
    let cat = data.catalog_mut();
    for (g, c, frame) in new_c {
        let rec = cluster_record(cat, g, &c, frame);
        let i = cat.add_cluster_record(rec);
        sys.promoted_c.insert((g, c.index), i);
    }
    for (_, g, n, frame) in new_n {
        let rec = nebula_record(cat, g, &n, frame);
        let i = cat.add_nebula_record(rec);
        sys.promoted_n.insert((g, n.index), i);
        sys.neb_sprite.insert(i, neb_sprite_id(g, n.index));
        info!("procedural nebula {} -> catalog {}", cat.nebulae[i as usize].names[0], i);
    }
}

// ---------------------------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------------------------

/// The object name inside a command line: `goto RC 8514-12 5000` -> `RC 8514-12`.
fn name_in_command(cmd: &str) -> Option<String> {
    let (verb, arg) = cmd.trim().split_once(' ')?;
    if !matches!(verb.to_ascii_lowercase().as_str(), "goto" | "select" | "look") {
        return None;
    }
    let arg = arg.trim();
    match arg.rsplit_once(' ') {
        Some((n, d)) if d.parse::<f64>().is_ok() && n.len() > 3 => Some(n.trim().to_string()),
        _ => Some(arg.to_string()),
    }
}

fn is_proc_name(s: &str) -> bool {
    se_core::proc_galaxy::parse_galaxy_name(s).is_some() || proc_galsys::parse_system_name(s).is_some()
}

impl ProcSystems {
    /// The generated system of catalog galaxy `g` (from a live slot, the cache, or generated now).
    fn system_of(&mut self, g: u32, input: impl Fn() -> Option<GenInput>) -> Option<(Arc<GalaxySystem>, GalaxyFrame)> {
        if let Some(Slot { state: SlotState::Ready(s), frame, .. }) = self.slots.get(&g) {
            return Some((s.clone(), *frame));
        }
        if let Some((s, _)) = self.cache.get(&g) {
            self.cache_used.insert(g, self.frame_no);
            // The frame only depends on the (unchanging) catalog record, so it is cheap to rebuild by the caller.
            let inp = input()?;
            return Some((s.clone(), inp.frame));
        }
        let inp = input()?;
        let system = run_generate(&inp);
        cache_put(&mut self.cache, &mut self.cache_used, (self.cache_cap, self.frame_no), g, (system.clone(), inp.uses_map), &self.wanted);
        Some((system, inp.frame))
    }
}

/// Keep galaxy `g`'s system in the cache, dropping the least recently used others beyond its size
/// `cap` (galaxies still wanted only when nothing else can go).
fn cache_put(
    cache: &mut HashMap<u32, (Arc<GalaxySystem>, bool)>,
    used: &mut HashMap<u32, u64>,
    (cap, frame): (usize, u64),
    g: u32,
    entry: (Arc<GalaxySystem>, bool),
    wanted: &[u32],
) {
    cache.insert(g, entry);
    used.insert(g, frame);
    let age = |k: &u32| used.get(k).copied().unwrap_or(0);
    let mut drop = crate::memory::lru_victims(cache.len(), cap, cache.keys().filter(|k| **k != g && !wanted.contains(k)).map(|k| (*k, age(k))));
    if cache.len() - drop.len() > cap {
        let rest = cache.len() - drop.len();
        drop.extend(crate::memory::lru_victims(rest, cap, cache.keys().filter(|k| **k != g && wanted.contains(k)).map(|k| (*k, age(k)))));
    }
    for k in drop {
        cache.remove(&k);
        used.remove(&k);
    }
}

/// Turns `RG ...`, `RC ...` and `RN ...` names mentioned by a command, the search box or a saved place into catalog
/// objects, so `goto`, selection, the wiki and the search box work on them like on catalog objects.
#[allow(clippy::too_many_arguments)]
fn resolve_names(
    mut data: ResMut<SeData>,
    hud: Option<Res<HudState>>,
    places: Option<Res<Places>>,
    settings: Res<ProcSettings>,
    density: Res<GalaxyDensity>,
    gal_field: Option<ResMut<ProcGalaxyField>>,
    sys: Option<ResMut<ProcSystems>>,
    gs: Option<Res<GalaxyState>>,
    ns: Option<Res<NebulaState>>,
    mut queue_mut: ResMut<CommandQueue>,
    mut frames: Local<u32>,
) {
    let (Some(mut gal_field), Some(mut sys), Some(gs), Some(ns)) = (gal_field, sys, gs, ns) else { return };
    *frames += 1;
    let queue = &*queue_mut;
    let fallback = *frames as u64 > FALLBACK_FRAMES;
    // (name, comes from a scripted command that must wait for the result)
    let mut names: Vec<(String, bool)> = Vec::new();
    if let Some(cmd) = queue.queue.front() {
        if let Some(n) = name_in_command(cmd) {
            names.push((n, true));
        }
    }
    if let Some(h) = hud.as_ref() {
        let s = h.search.trim();
        if s.len() > 3 {
            names.push((s.to_string(), false));
        }
    }
    // Saved places mention procedural objects: make them selectable once the Milky Way's emission map exists.
    if !sys.places_done && *frames > 30 && (density.has_map() || fallback) {
        sys.places_done = true;
        if let Some(p) = places.as_ref() {
            for pl in &p.0 {
                names.push((pl.body.clone(), false));
                names.push((pl.parent.clone(), false));
            }
        }
    }
    let mut hold = false;
    for (name, scripted) in names {
        // A cluster member's `RSC` name needs its `RC` cluster (`clusters::resolve_member_names`),
        // unless that is a catalog cluster or already promoted.
        let name = match se_core::cluster_stars::parse_member_name(&name) {
            Some((rc, ..)) if crate::clusters::cluster_of_rc(&data.catalog, &rc).is_some() => continue,
            Some((rc, ..)) => rc,
            None => name,
        };
        if !is_proc_name(&name) || data.catalog.find(&name).is_some() {
            continue;
        }
        // Milky Way objects depend on its emission map: hold a scripted command until it exists.
        if !density.has_map() && !fallback && matches!(proc_galsys::parse_system_name(&name), Some((_, GalaxyRef::Catalog(MILKY_WAY_SE_INDEX), _))) {
            hold |= scripted;
            continue;
        }
        let pools = gs.model_pools();
        resolve_one(&mut data, &mut gal_field, &mut sys, &gs, &ns, &density, &settings.cfg, &pools, &name, fallback);
    }
    if hold {
        queue_mut.wait_frames = queue_mut.wait_frames.max(2);
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_one(
    data: &mut SeData,
    gal_field: &mut ProcGalaxyField,
    sys: &mut ProcSystems,
    gs: &GalaxyState,
    ns: &NebulaState,
    density: &GalaxyDensity,
    cfg: &DeepSkyConfig,
    pools: &[u32; 17],
    name: &str,
    fallback: bool,
) -> Option<ObjectRef> {
    if se_core::proc_galaxy::parse_galaxy_name(name).is_some() {
        return ensure_galaxy(data, gal_field, pools, name);
    }
    let (kind, gref, index) = proc_galsys::parse_system_name(name)?;
    // Which catalog galaxy?
    let g = match gref {
        GalaxyRef::Catalog(i) if i == MILKY_WAY_SE_INDEX => match data.catalog.find("Milky Way") {
            Some(ObjectRef::Galaxy(g)) => g,
            _ => return None,
        },
        GalaxyRef::Catalog(i) => (i as usize).lt(&data.catalog.galaxies.len()).then_some(i)?,
        GalaxyRef::Procedural(key, gi) => match ensure_galaxy(data, gal_field, pools, &se_core::proc_galaxy::galaxy_name(key, gi))? {
            ObjectRef::Galaxy(g) => g,
            _ => return None,
        },
    };
    let cat = data.catalog.clone();
    let (system, frame) = sys.system_of(g, || gen_input(g, &cat, gs, ns, density, cfg, fallback))?;
    drop(cat);
    match kind {
        SystemKind::Cluster => {
            let c = system.clusters.iter().find(|c| c.index == index)?.clone();
            let cat = data.catalog_mut();
            let rec = cluster_record(cat, g, &c, frame);
            let i = cat.add_cluster_record(rec);
            sys.promoted_c.insert((g, c.index), i);
            Some(ObjectRef::Cluster(i))
        }
        SystemKind::Nebula => {
            let n = system.nebulae.iter().find(|n| n.index == index)?.clone();
            let cat = data.catalog_mut();
            let rec = nebula_record(cat, g, &n, frame);
            let i = cat.add_nebula_record(rec);
            sys.promoted_n.insert((g, n.index), i);
            sys.neb_sprite.insert(i, neb_sprite_id(g, n.index));
            Some(ObjectRef::Nebula(i))
        }
    }
}
