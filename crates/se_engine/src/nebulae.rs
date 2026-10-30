//! Nebula renderer (diffuse, planetary and supernova-remnant nebulae of the catalog).
//!
//! Nebulae use the same machinery as galaxies (see `galaxies.rs` and `docs/formats/nebula.md`): SE builds
//! them with `seSpriteModelMaker` from `models/nebulae/*.pst` (`NebulaModel` blocks, 135 models) and the
//! `textures/nebulae` images, using the `CubeMap` (73), `Cylindrical` (51), `Diffuse` (3), `Spherical` (1) and
//! the image-free procedural `Planetary` (3) / `SNR` (4) generators that `se_core::galaxy_model` implements
//! (`createNebula`: a random-width spherical emission shell with a dust skin; the pool of a nebula `Type`
//! contains these default models too, so tiny catalog planetaries and remnants get a 3D shell when close).
//!
//! * **Far field**: one billboard per catalog nebula at infinity, drawn with the galaxy far-sprite shader
//!   (a second `GalaxyFarMaterial` instance, soft profile), coloured by the model's stored `sumColor`.
//! * **Near volume**: when the angular radius exceeds `build_px` or the camera is inside, the model is built
//!   on the async pool and drawn with the galaxy model shader (emission `Add` + dust `Multiply`).
//! * Like the galaxies, everything sorts after all other transparent items and fades out in daylight.

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::window::PrimaryWindow;
use se_core::catalog::NebulaRecord;
use se_core::galaxy_model::{
    GalaxyModelDef, ModelCatalog, ModelImages, ModelMethod, ModelSelection, RgbaImage, SpriteLayer, build_galaxy_model,
    model_textures, nebula_position_hash, parse_pst_kind,
};
use se_core::vfs::Vfs;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::galaxies::{
    ATTR_FAR_CORNER, ATTR_FAR_DATA, ATTR_FAR_SHAPE, FOCAL_REF, GalaxyFarMaterial, GalaxyModelMaterial, GalaxySettings,
    ModelParams, SpriteAtlas, SpriteBuffers, angular_px, far_uniforms, model_fade, pad_aabb, setup_galaxies,
    update_galaxies,
};

/// User-tweakable nebula settings.
#[derive(Resource, Debug, Clone)]
pub struct NebulaSettings {
    pub enabled: bool,
    /// Far-sprite brightness; the 3D model is calibrated to the same mean surface brightness.
    pub brightness: f32,
    /// Extra multiplier on the model emission.
    pub model_brightness: f32,
    /// Dust optical-depth scale.
    pub dust_strength: f32,
    /// Build a model when the angular radius exceeds this many pixels.
    pub build_px: f32,
    /// Maximum resident models.
    pub max_models: usize,
}

impl Default for NebulaSettings {
    fn default() -> Self {
        let mut s = Self {
            enabled: true,
            brightness: 0.8,
            model_brightness: 1.0,
            dust_strength: 0.06,
            build_px: 4.0,
            max_models: 4,
        };
        let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
        if let Some(v) = get("SE_NEB_FAR") {
            s.brightness = v;
        }
        if let Some(v) = get("SE_NEB_MODEL") {
            s.model_brightness = v;
        }
        if let Some(v) = get("SE_NEB_DUST") {
            s.dust_strength = v;
        }
        s
    }
}

struct BuiltNebula {
    em: SpriteBuffers,
    dust: SpriteBuffers,
    s_raw: f32,
    sprites: usize,
}

struct BuildInput {
    vfs: Arc<Vfs>,
    textures: Arc<Vec<String>>,
    def: GalaxyModelDef,
    orientation: DQuat,
    radius_pc: f64,
    tile_mean: [f32; 16],
}

/// Loads a nebula texture. Like the exe, the absorption channel is the *last* channel of the image, so an
/// RGB image (no alpha) uses its blue channel for absorption.
fn load_image(vfs: &Vfs, name: &Option<String>) -> Option<RgbaImage> {
    let name = name.as_ref()?;
    let bytes = vfs.read(&format!("textures/nebulae/{name}")).ok()?;
    let dynimg = image::load_from_memory(&bytes).ok()?;
    let has_alpha = dynimg.color().has_alpha();
    let mut img = dynimg.to_rgba8();
    if !has_alpha {
        for p in img.pixels_mut() {
            p.0[3] = p.0[2];
        }
    }
    let (w, h) = img.dimensions();
    RgbaImage::new(w, h, img.into_raw())
}

fn generate(vfs: &Vfs, textures: &[String], def: &GalaxyModelDef) -> Option<se_core::galaxy_model::GalaxyModelData> {
    let tex = model_textures(def, textures);
    let front = load_image(vfs, &tex.front_image);
    let side = load_image(vfs, &tex.side_image);
    let cube: Vec<Option<RgbaImage>> = tex.cube.iter().map(|n| load_image(vfs, n)).collect();
    let images = ModelImages { front: front.as_ref(), side: side.as_ref(), cube: std::array::from_fn(|i| cube[i].as_ref()) };
    match build_galaxy_model(def, &images) {
        Ok(d) => Some(d),
        Err(e) => {
            warn!("nebula model {:?} failed: {e}", def.name);
            None
        }
    }
}

/// Emission-weighted mean colour of a model (normalised to max 1), used for the far sprite.
fn nebula_color(vfs: &Vfs, textures: &[String], def: &GalaxyModelDef, tile_mean: &[f32; 16]) -> Option<[f32; 3]> {
    let data = generate(vfs, textures, def)?;
    let mut c = [0.0f64; 3];
    for s in data.sprites.iter().filter(|s| s.layer() != SpriteLayer::Absorption) {
        let w = (s.brightness * tile_mean[s.atlas_tile() as usize]) as f64 * (s.size as f64).powi(2);
        for k in 0..3 {
            c[k] += s.color[k] as f64 * w;
        }
    }
    let m = c[0].max(c[1]).max(c[2]);
    (m > 0.0).then(|| [(c[0] / m) as f32, (c[1] / m) as f32, (c[2] / m) as f32])
}

fn build_nebula(inp: BuildInput) -> Option<BuiltNebula> {
    let BuildInput { vfs, textures, def, orientation, radius_pc, tile_mean } = inp;
    let data = generate(&vfs, &textures, &def)?;
    let ms = def.model_scale();
    let scale = DVec3::new(ms.x as f64, ms.y as f64, ms.z as f64) * radius_pc;
    let mut em = SpriteBuffers::default();
    let mut dust = SpriteBuffers::default();
    let mut cover = 0.0f64;
    for s in &data.sprites {
        let local = DVec3::new(s.pos[0] as f64, s.pos[1] as f64, s.pos[2] as f64) * scale;
        let off = orientation * local;
        let size_pc = s.size as f64 * scale.x;
        let layer = s.layer();
        let info = [size_pc as f32, s.atlas_tile() as f32, s.lod_rand as f32 / 255.0, layer as u8 as f32];
        let pos = [off.x as f32, off.y as f32, off.z as f32];
        let d = [s.color[0], s.color[1], s.color[2], s.brightness * def.brightness];
        if layer == SpriteLayer::Absorption {
            dust.push(pos, d, info);
        } else {
            em.push(pos, d, info);
            let lum = 0.2126 * s.color[0] + 0.7152 * s.color[1] + 0.0722 * s.color[2];
            let side = 2.0 * s.size as f64;
            cover += (d[3] * lum) as f64 * tile_mean[s.atlas_tile() as usize] as f64 * side * side;
        }
    }
    Some(BuiltNebula { sprites: data.sprites.len(), em, dust, s_raw: (cover / PI) as f32 })
}

enum SlotState {
    Loading(Task<Option<BuiltNebula>>),
    Ready(Ready),
    Failed,
}

struct Ready {
    entities: [Entity; 2],
    mats: [Handle<GalaxyModelMaterial>; 2],
    s_raw: f32,
}

struct Slot {
    nebula: usize,
    state: SlotState,
    last_wanted: f32,
}

#[derive(Resource)]
pub(crate) struct NebulaState {
    models: Arc<ModelCatalog>,
    textures: Arc<Vec<String>>,
    /// Model indices per nebula `Type` (`UseForType`), restricted to the supported generators.
    pools: HashMap<String, Vec<usize>>,
    model_of: Vec<Option<usize>>,
    atlas: SpriteAtlas,
    far: Handle<GalaxyFarMaterial>,
    slots: Vec<Slot>,
    failed: HashSet<usize>,
    far_mesh: Handle<Mesh>,
    /// First vertex of each nebula in the far mesh.
    vert_of: Vec<Option<u32>>,
    /// Nebulae whose far colour is still to be derived from their model (brightest last).
    color_queue: Vec<usize>,
    color_tasks: Vec<(usize, Task<Option<[f32; 3]>>)>,
}

/// Catalog nebulae whose 3D model is fading in (`(catalog index, fade)`, written by `update_nebulae`);
/// the streamed procedural nebula sprites hide behind them.
#[derive(Resource, Default)]
pub(crate) struct ResidentNebulae(pub Vec<(usize, f32)>);

impl NebulaState {
    /// Number of sprite models of a nebula `Type` (the random pool of procedural nebulae).
    pub(crate) fn pool_len(&self, kind: &str) -> u32 {
        self.pools.get(kind).map_or(0, |p| p.len() as u32)
    }

    /// The material of the catalog far sprites.
    pub(crate) fn far_handle(&self) -> Handle<GalaxyFarMaterial> {
        self.far.clone()
    }

    /// Model of a nebula added to the catalog at runtime: the raw pool draw of a procedural nebula, else
    /// the position hash of the catalog nebulae.
    fn pick_runtime(&self, rec: &NebulaRecord) -> Option<usize> {
        let pool = self.pools.get(&rec.kind).filter(|p| !p.is_empty())?;
        let i = match rec.model_roll {
            Some(r) => r % pool.len() as u32,
            None => nebula_position_hash([rec.pos_pc.x, rec.pos_pc.y, rec.pos_pc.z]) % pool.len() as u32,
        };
        Some(pool[i as usize]).filter(|&m| supported(self.models.models()[m].method))
    }
}

pub struct NebulaePlugin;

impl Plugin for NebulaePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NebulaSettings>()
            .init_resource::<ResidentNebulae>()
            .add_systems(Startup, setup_nebulae.after(setup_galaxies))
            .add_systems(PostUpdate, update_nebulae.after(update_galaxies));
    }
}

fn supported(m: ModelMethod) -> bool {
    matches!(
        m,
        ModelMethod::CubeMap
            | ModelMethod::Cylindrical
            | ModelMethod::Diffuse
            | ModelMethod::Spherical
            | ModelMethod::Planetary
            | ModelMethod::Snr
    )
}

fn profile_color(rec: &NebulaRecord) -> [f32; 3] {
    match rec.kind.as_str() {
        "Planetary" => [0.45, 0.95, 0.85],
        "SNR" => [1.0, 0.6, 0.65],
        _ => [1.0, 0.6, 0.75],
    }
}

fn setup_nebulae(
    mut commands: Commands,
    data: Res<SeData>,
    settings: Res<NebulaSettings>,
    atlas: Res<SpriteAtlas>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
) {
    let vfs = data.vfs.clone();
    let cat = data.catalog.clone();
    let mut defs: Vec<GalaxyModelDef> = Vec::new();
    for p in vfs.list_ext("models/nebulae", "pst") {
        match vfs.read_string(&p).map(|t| parse_pst_kind(&t, "NebulaModel")) {
            Ok(Ok(m)) => defs.extend(m),
            Ok(Err(e)) => warn!("{p}: {e}"),
            Err(e) => warn!("{p}: {e}"),
        }
    }
    let prefix = "textures/nebulae/";
    let textures: Vec<String> = vfs
        .list("textures/nebulae")
        .into_iter()
        .filter_map(|p| p.strip_prefix(prefix).map(str::to_string))
        .filter(|n| !n.contains('/'))
        .collect();
    // Type pools (UseForType) restricted to generators we implement.
    let mut pools: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, d) in defs.iter().enumerate() {
        if let (Some(t), true) = (&d.use_for_type, supported(d.method)) {
            pools.entry(t.clone()).or_default().push(i);
        }
    }
    let models = ModelCatalog::new(defs);
    // Ray-marched models (`models/custom/*.cfg`): named ones win over the sprite models, and nebulae
    // without a named model of either kind take one from the ray-marched pool of their type.
    let custom_files: Vec<(String, String)> =
        vfs.list_ext("models/custom", "cfg").into_iter().filter_map(|p| vfs.read_string(&p).ok().map(|t| (p, t))).collect();
    let custom = std::sync::Arc::new(se_core::custom_model::CustomModelSet::new(custom_files));
    let mut raymarch_of = Vec::with_capacity(cat.nebulae.len());

    let force = std::env::var("SE_NEB_FORCE").ok().and_then(|v| v.split_once('=').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())));
    let mut model_of = Vec::with_capacity(cat.nebulae.len());
    let mut with_model = 0;
    for rec in &cat.nebulae {
        let mut pick = None;
        let mut raymarch = custom.named(&rec.names);
        for n in rec.names.iter().filter(|_| raymarch.is_none()) {
            // Hubble type "E0" has no pool, so only an exact `UseForObject` match returns a model.
            if let ModelSelection::Model(i) = models.select(n, "E0", [0.0; 3]) {
                pick = Some(i);
                break;
            }
        }
        if pick.is_none() && raymarch.is_none() {
            raymarch = custom.pooled(&rec.kind, [rec.pos_pc.x, rec.pos_pc.y, rec.pos_pc.z]);
        }
        if pick.is_none() && raymarch.is_none() {
            if let Some(pool) = pools.get(&rec.kind).filter(|p| !p.is_empty()) {
                let h = nebula_position_hash([rec.pos_pc.x, rec.pos_pc.y, rec.pos_pc.z]);
                pick = Some(pool[(h % pool.len() as u32) as usize]);
            }
        }
        // Debug override: SE_NEB_FORCE="Ring Nebula=Plan0" gives that catalog nebula that model.
        if let Some((target, model)) = force.as_ref() {
            if rec.names.iter().any(|n| n.eq_ignore_ascii_case(target)) {
                pick = models.models().iter().position(|m| m.name.eq_ignore_ascii_case(model)).or(pick);
                raymarch = None;
            }
        }
        // Models whose generator is missing (none left in the shipped data) are skipped.
        let pick = pick.filter(|&i| supported(models.models()[i].method));
        if pick.is_some() {
            with_model += 1;
        }
        model_of.push(pick);
        raymarch_of.push(raymarch);
    }
    info!(
        "nebulae: {} ray-marched models, {} catalog nebulae ray-marched",
        custom.models.len(),
        raymarch_of.iter().flatten().count()
    );
    commands.insert_resource(crate::nebula_raymarch::RaymarchAssign { set: custom, model_of: raymarch_of });
    info!(
        "nebulae: {} models, {} textures, {} of {} catalog nebulae use a sprite model",
        models.models().len(),
        textures.len(),
        with_model,
        cat.nebulae.len()
    );

    // ---- far-field mesh (shares the galaxy far shader: profile type 2 = soft disc) --------------------------------
    let n = cat.nebulae.len();
    let mut positions = Vec::with_capacity(n * 4);
    let mut fdata = Vec::with_capacity(n * 4);
    let mut shape = Vec::with_capacity(n * 4);
    let mut corner = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);
    let mut vert_of = vec![None; n];
    for (i, rec) in cat.nebulae.iter().enumerate() {
        if !rec.abs_mag.is_finite() || !rec.radius_pc.is_finite() || rec.radius_pc <= 0.0 {
            continue;
        }
        let mut col = model_of[i]
            .and_then(|m| models.models()[m].sum_color)
            .map(|c| {
                let mx = c[0].max(c[1]).max(c[2]).max(1e-3);
                [c[0] / mx, c[1] / mx, c[2] / mx]
            })
            .unwrap_or_else(|| profile_color(rec));
        if col.iter().all(|c| *c < 0.05) {
            col = profile_color(rec);
        }
        let p = rec.pos_pc.as_vec3();
        let base = positions.len() as u32;
        vert_of[i] = Some(base);
        for c in [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            positions.push([p.x, p.y, p.z]);
            fdata.push([col[0], col[1], col[2], rec.abs_mag as f32]);
            shape.push([0.0, 0.0, 0.0, rec.radius_pc as f32]);
            corner.push([c[0], c[1], i as f32, 2.0]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    pad_aabb(&mut positions);
    fdata.extend([[0.0; 4]; 2]);
    shape.extend([[0.0; 4]; 2]);
    corner.extend([[0.0; 4]; 2]);
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(ATTR_FAR_DATA, fdata)
        .with_inserted_attribute(ATTR_FAR_SHAPE, shape)
        .with_inserted_attribute(ATTR_FAR_CORNER, corner)
        .with_inserted_indices(Indices::U32(indices));
    let gs = GalaxySettings { far_brightness: settings.brightness, min_px: 2.0, cull: 0.002, ..GalaxySettings::default() };
    let far = far_mats.add(GalaxyFarMaterial { uniforms: far_uniforms(Vec3::ZERO, &gs) });
    let far_mesh = meshes.add(mesh);
    let mut color_queue: Vec<usize> = (0..n).filter(|&i| model_of[i].is_some() && vert_of[i].is_some()).collect();
    // Brightest first (popped from the back).
    color_queue.sort_by(|&a, &b| cat.nebulae[b].abs_mag.total_cmp(&cat.nebulae[a].abs_mag));
    commands.spawn((Mesh3d(far_mesh.clone()), MeshMaterial3d(far.clone()), Transform::IDENTITY, NoFrustumCulling));

    commands.insert_resource(NebulaState {
        models: Arc::new(models),
        textures: Arc::new(textures),
        pools,
        model_of,
        atlas: atlas.clone(),
        far,
        slots: Vec::new(),
        failed: HashSet::new(),
        far_mesh,
        vert_of,
        color_queue,
        color_tasks: Vec::new(),
    });
}

fn spawn_build(st: &mut NebulaState, vfs: &Arc<Vfs>, rec: &NebulaRecord, n: usize, now: f32) {
    let Some(mi) = st.model_of[n] else { return };
    let input = BuildInput {
        vfs: vfs.clone(),
        textures: st.textures.clone(),
        def: st.models.models()[mi].clone(),
        orientation: rec.orientation,
        radius_pc: rec.radius_pc,
        tile_mean: st.atlas.tile_mean,
    };
    let task = AsyncComputeTaskPool::get().spawn(async move { build_nebula(input) });
    st.slots.push(Slot { nebula: n, state: SlotState::Loading(task), last_wanted: now });
}

fn despawn_slot(commands: &mut Commands, slot: Slot) {
    if let SlotState::Ready(r) = slot.state {
        for e in r.entities {
            commands.entity(e).despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_nebulae(
    mut commands: Commands,
    time: Res<Time>,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<NebulaSettings>,
    mut st: ResMut<NebulaState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut model_mats: ResMut<Assets<GalaxyModelMaterial>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
    mut vis: Query<&mut Visibility>,
    mut resident_out: ResMut<ResidentNebulae>,
) {
    let st = &mut *st;
    let cat = &data.catalog;
    // Nebulae added to the catalog at runtime (procedural `RN` nebulae) get their model choice here.
    while st.model_of.len() < cat.nebulae.len() {
        let pick = st.pick_runtime(&cat.nebulae[st.model_of.len()]);
        st.model_of.push(pick);
        st.vert_of.push(None);
    }
    resident_out.0.clear();
    let now = time.elapsed_secs();
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();
    let max_models = settings.max_models.min(8);

    let mut wanted: Vec<(f64, usize)> = Vec::new();
    if settings.enabled {
        for (n, rec) in cat.nebulae.iter().enumerate() {
            if st.model_of[n].is_none() {
                continue;
            }
            let px = angular_px(rec.radius_pc, (rec.pos_pc - cam_pc).length(), focal);
            if px > settings.build_px as f64 && !st.failed.contains(&n) {
                wanted.push((px, n));
            }
        }
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0));
        wanted.truncate(max_models);
    }
    for &(_, n) in &wanted {
        if let Some(slot) = st.slots.iter_mut().find(|s| s.nebula == n) {
            slot.last_wanted = now;
            continue;
        }
        if st.slots.len() >= max_models {
            let victim = st
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| !wanted.iter().any(|w| w.1 == s.nebula))
                .min_by(|a, b| a.1.last_wanted.total_cmp(&b.1.last_wanted))
                .map(|(i, _)| i);
            let Some(i) = victim else { continue };
            let old = st.slots.remove(i);
            despawn_slot(&mut commands, old);
        }
        spawn_build(st, &data.vfs, &cat.nebulae[n], n, now);
    }

    // derive far-sprite colours from the models, a couple at a time in the background
    while st.color_tasks.len() < 2 {
        let Some(n) = st.color_queue.pop() else { break };
        let (vfs, tex, tm) = (data.vfs.clone(), st.textures.clone(), st.atlas.tile_mean);
        let def = st.models.models()[st.model_of[n].unwrap()].clone();
        let task = AsyncComputeTaskPool::get().spawn(async move { nebula_color(&vfs, &tex, &def, &tm) });
        st.color_tasks.push((n, task));
    }
    let mut colored: Vec<(usize, [f32; 3])> = Vec::new();
    st.color_tasks.retain_mut(|(n, task)| match check_ready(task) {
        Some(res) => {
            if let Some(c) = res {
                colored.push((*n, c));
            }
            false
        }
        None => true,
    });
    if !colored.is_empty() {
        if let Some(mut mesh) = meshes.get_mut(&st.far_mesh) {
            if let Some(VertexAttributeValues::Float32x4(v)) = mesh.attribute_mut(ATTR_FAR_DATA) {
                for (n, c) in &colored {
                    if let Some(base) = st.vert_of[*n] {
                        for k in 0..4 {
                            let e = &mut v[base as usize + k];
                            e[0] = c[0];
                            e[1] = c[1];
                            e[2] = c[2];
                        }
                    }
                }
            }
        }
    }

    // poll builds
    for slot in &mut st.slots {
        let polled = match &mut slot.state {
            SlotState::Loading(task) => check_ready(task),
            _ => None,
        };
        let Some(result) = polled else { continue };
        let Some(built) = result else {
            slot.state = SlotState::Failed;
            st.failed.insert(slot.nebula);
            continue;
        };
        info!("nebula model {}: {} sprites", cat.nebulae[slot.nebula].names[0], built.sprites);
        let mut mk = |buf: SpriteBuffers, dust: bool| {
            let mat = model_mats.add(GalaxyModelMaterial {
                uniforms: ModelParams {
                    center_rel: Vec4::ZERO,
                    params: Vec4::new(1.0, 2.0, 0.0, 0.0),
                    params2: Vec4::new(if dust { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
                },
                atlas: st.atlas.handle.clone(),
                dust,
            });
            (mat, meshes.add(buf.into_mesh()))
        };
        let (m_em, mesh_em) = mk(built.em, false);
        let (m_du, mesh_du) = mk(built.dust, true);
        let mut spawn = |mesh: Handle<Mesh>, mat: &Handle<GalaxyModelMaterial>| {
            commands
                .spawn((Mesh3d(mesh), MeshMaterial3d(mat.clone()), Transform::IDENTITY, Visibility::Hidden, NoFrustumCulling))
                .id()
        };
        let e0 = spawn(mesh_em, &m_em);
        let e1 = spawn(mesh_du, &m_du);
        slot.state = SlotState::Ready(Ready { entities: [e0, e1], mats: [m_em, m_du], s_raw: built.s_raw });
    }
    st.slots.retain(|s| !matches!(s.state, SlotState::Failed));
    let mut i = 0;
    while i < st.slots.len() {
        let s = &st.slots[i];
        if now - s.last_wanted > 4.0 && matches!(s.state, SlotState::Ready(_)) {
            let old = st.slots.remove(i);
            despawn_slot(&mut commands, old);
        } else {
            i += 1;
        }
    }

    // per-frame uniforms
    let gs = GalaxySettings { far_brightness: settings.brightness, min_px: 2.0, cull: 0.002, ..GalaxySettings::default() };
    let mut far_u = far_uniforms(cam_pc.as_vec3(), &gs);
    let mut n_res = 0;
    for slot in &st.slots {
        let SlotState::Ready(ready) = &slot.state else { continue };
        let rec = &cat.nebulae[slot.nebula];
        let def = &st.models.models()[st.model_of[slot.nebula].unwrap()];
        let d = (rec.pos_pc - cam_pc).length();
        let px = angular_px(rec.radius_pc, d, focal);
        let fade = if settings.enabled { model_fade(px, settings.build_px as f64) } else { 0.0 };
        let center_rel = (rec.pos_pc - cam_pc).as_vec3().extend(fade);
        // Same mean surface brightness as the far sprite.
        let r = rec.radius_pc * def.radius[0] as f64;
        let flux = 10f64.powf(0.4 * (gs.mag_ref as f64 - rec.abs_mag)) * 100.0;
        let target = settings.brightness as f64 * flux / (PI * r * r * FOCAL_REF * FOCAL_REF);
        let k_em = if ready.s_raw > 0.0 { (target / ready.s_raw as f64) as f32 * settings.model_brightness } else { 0.0 };
        let k_dust = settings.dust_strength * def.transparency;
        for (e, (mat, dust)) in ready.entities.iter().zip(ready.mats.iter().zip([false, true])) {
            if let Some(mut m) = model_mats.get_mut(mat) {
                m.uniforms.center_rel = center_rel;
                m.uniforms.params = Vec4::new(if dust { k_dust } else { k_em }, 2.0, 0.0, 0.0);
            }
            if let Ok(mut v) = vis.get_mut(*e) {
                *v = if fade > 0.001 { Visibility::Visible } else { Visibility::Hidden };
            }
        }
        if n_res < 8 && fade > 0.0 {
            far_u.resident[n_res] = Vec4::new(slot.nebula as f32, fade, 0.0, 0.0);
            n_res += 1;
        }
        if fade > 0.0 {
            resident_out.0.push((slot.nebula, fade));
        }
    }
    if let Some(mut m) = far_mats.get_mut(&st.far) {
        m.uniforms = far_u;
    }
}
