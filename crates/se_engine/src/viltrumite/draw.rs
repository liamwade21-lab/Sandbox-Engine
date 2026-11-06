//! Drawing him (spec F13): `HeroMaterial`, his bone palette, his light.
//!
//! - **Palette.** One `ShaderBuffer` (`HeroFrameGpu`) written in place each frame; the meshes' transform is
//!   the identity and the palette is camera-relative, worked out from his `UPos` after the camera has moved,
//!   so he never goes through the scaled space. His own attributes (`ATTR_HERO_JOINTS`, `ATTR_HERO_WEIGHTS`):
//!   Bevy's joint attributes would switch its pipeline to its own skinning.
//! - **Light.** One path: the ground's (`ground_light.wesl` `point_light_at`), run once a frame by a compute
//!   pass (`hero_light.wesl`; per vertex it would read 30 textures 165,706 times), its result read by his
//!   draws. Near a body with a fresh light record the frame holds that body's own light record and atmosphere, as the
//!   scatter's do. Otherwise (a body's record goes stale when it isn't resolved, its tables end at their top,
//!   and its own shine isn't in its record) the CPU fills the same record: the stars with their shadows and
//!   atmospheres on the way, no tables, and the further lights with his own normals.
//! - **Off costs nothing.** All of this is made on first enable and then hidden, never respawned; the compute
//!   pipeline is queued then too, and no system does anything while the mod is off.
//! - **Loading.** The real meshes queue their pipelines (Bevy builds one the first time something would draw
//!   with it) under a palette of zeros, which puts every vertex at the camera, so no pixel is drawn until
//!   `pipeline_watch` says all of his are built: he appears whole, and the row stops reading Loading.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::ecs::system::SystemParam;
use bevy::math::{Affine3A, DVec3, Mat3A, Vec3A};
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexAttributeValues, VertexFormat};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, texture_3d, texture_cube};
use bevy::render::render_resource::encase;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::ShaderRef;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::catalog::ObjectRef;
use se_core::hero_asset::{HeroDraw, HeroModel, MAX_BONES};
use se_core::procgen::civilization::culture;
use se_core::upos::UPos;

use crate::atmosphere::AtmosphereParamsUniform;
use crate::camera::{FlyCamera, MainCamera};
use crate::color_correction::ColorCorrection;
use crate::data::SeData;
use crate::mods::{HeroAssets, ModControl};
use crate::stars::StarSettings;
use crate::system::{AmbientLighting, BatchedSprite, LightingFade, SystemBody, illum_color, planet_shine, star_tint};
use crate::terrain::{ATM_SUN_SCALE, PatchLight, Terrain, TileCaches};
use crate::volumetric_clouds::CloudShadowAssets;

use super::tune;
use super::{Hero, LightPath, Place};

pub const ATTR_HERO_JOINTS: MeshVertexAttribute = MeshVertexAttribute::new("HeroJoints", 0x5E56_1001, VertexFormat::Uint8x4);
pub const ATTR_HERO_WEIGHTS: MeshVertexAttribute = MeshVertexAttribute::new("HeroWeights", 0x5E56_1002, VertexFormat::Unorm8x4);

/// `ground_light.wesl` `LightBody`.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct LightBodyGpu {
    m0: Vec4,
    m1: Vec4,
    m2: Vec4,
    centre: Vec4,
    atm_flags: Vec4,
    key: Vec4,
}

/// `hero_frame.wesl` `HeroFrame`.
#[derive(Clone, Copy, ShaderType)]
pub struct HeroFrameGpu {
    joints: [[Vec4; 3]; MAX_BONES],
    point: Vec4,
    up: Vec4,
    town: Vec4,
    heat: Vec4,
    wind: Vec4,
    shadow_plane: Vec4,
    free_dir: [Vec4; 3],
    free_color: [Vec4; 3],
    body: LightBodyGpu,
    atm: AtmosphereParamsUniform,
    light: PatchLight,
}

impl Default for HeroFrameGpu {
    fn default() -> Self {
        HeroFrameGpu {
            joints: [[Vec4::ZERO; 3]; MAX_BONES],
            point: Vec4::ZERO,
            up: Vec4::Y,
            town: Vec4::ZERO,
            heat: Vec4::ZERO,
            wind: Vec4::ZERO,
            shadow_plane: Vec4::ZERO,
            free_dir: [Vec4::ZERO; 3],
            free_color: [Vec4::ZERO; 3],
            body: LightBodyGpu::default(),
            atm: AtmosphereParamsUniform::default(),
            light: PatchLight::default(),
        }
    }
}

/// `hero_frame.wesl` `HeroLight`: written by the compute pass, never by the CPU.
#[derive(Clone, Copy, Default, ShaderType)]
struct HeroLightGpu {
    sun: Vec4,
    sky: Vec4,
    flat_e: Vec4,
    haze_t: Vec4,
    haze_a: Vec4,
}

/// One of his draws. Never changed after it is made: everything per-frame is in `frame`.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(HeroMaterialKey)]
pub struct HeroMaterial {
    #[storage(0, read_only)]
    pub frame: Handle<ShaderBuffer>,
    #[storage(1, read_only)]
    pub light: Handle<ShaderBuffer>,
    #[texture(2)]
    #[sampler(3)]
    pub diffuse: Handle<Image>,
    #[texture(4)]
    #[sampler(5)]
    pub normal: Handle<Image>,
    #[texture(6)]
    #[sampler(7)]
    pub spec: Handle<Image>,
    pub cutout: bool,
}

/// Cutouts take the entry point with the discard, in the alpha-mask phase; the opaque keep the early depth test.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct HeroMaterialKey {
    cutout: bool,
}

impl From<&HeroMaterial> for HeroMaterialKey {
    fn from(m: &HeroMaterial) -> Self {
        Self { cutout: m.cutout }
    }
}

impl Material for HeroMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/hero.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/hero.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        if self.cutout { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque }
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_TANGENT.at_shader_location(3),
            ATTR_HERO_JOINTS.at_shader_location(4),
            ATTR_HERO_WEIGHTS.at_shader_location(5),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        let cutout = key.bind_group_data.cutout;
        if let Some(f) = descriptor.fragment.as_mut() {
            f.entry_point = Some(if cutout { "fragment_cutout" } else { "fragment" }.into());
        }
        if cutout {
            // The hair is thin cards, seen from both sides.
            descriptor.primitive.cull_mode = None;
        }
        Ok(())
    }
}

/// What his first enable made, kept until quit.
#[derive(Resource)]
pub struct HeroDraws {
    entities: Vec<Entity>,
    frame_buffer: Handle<ShaderBuffer>,
    /// The record, kept here so nothing is allocated a frame.
    frame: Box<HeroFrameGpu>,
    /// His pipelines are built: the palette is real from here on.
    ready: bool,
    /// Frames still to wait after they are built (each material's bind group a frame behind at worst).
    settle: u32,
    started: std::time::Instant,
    /// Which light he had last frame, for the log.
    was_near: Option<bool>,
    town_timer: u32,
    /// How far into a town he is, being worked out, and the lamps' colour and strength it scales.
    town_task: Option<(Vec3, Task<f32>)>,
}

impl HeroDraws {
    /// The street lamps' light on him now (for the log).
    pub(super) fn town(&self) -> [f32; 3] {
        self.frame.town.truncate().to_array()
    }
}

/// What the compute pass needs from the main world (made on first enable; off, `on` is false).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct HeroLightInput {
    pub on: bool,
    frame: Handle<ShaderBuffer>,
    light: Handle<ShaderBuffer>,
    /// The near body's inscatter, transmittance and irradiance tables.
    tables: Option<(Handle<Image>, Handle<Image>, Handle<Image>)>,
}

/// The compute pass's pipeline, queued on first enable.
#[derive(Resource, Default)]
struct HeroLightPass {
    layout: Option<BindGroupLayoutDescriptor>,
    pipeline: Option<CachedComputePipelineId>,
    group: Option<BindGroup>,
}

/// The view he draws in: the main camera (as the scatter's).
type MainView = (With<ColorCorrection>, Without<crate::capture::CaptureView>);

pub fn build(app: &mut App) {
    app.add_plugins(MaterialPlugin::<HeroMaterial>::default()).add_plugins(ExtractResourcePlugin::<HeroLightInput>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<HeroLightPass>()
        .add_systems(Render, prepare_light_pass.in_set(RenderSystems::PrepareBindGroups).run_if(resource_exists::<HeroLightInput>))
        .add_systems(Core3d, light_pass.in_set(Core3dSystems::Prepass).before(main_opaque_pass_3d));
}

/// One draw as a mesh of its own vertices.
fn draw_mesh(model: &HeroModel, d: &HeroDraw) -> Mesh {
    let vs = &model.vertices[d.first_vertex as usize..(d.first_vertex + d.vertex_count) as usize];
    let idx = model.indices[d.first_index as usize..(d.first_index + d.index_count) as usize].to_vec();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vs.iter().map(|v| v.position).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vs.iter().map(|v| v.normal).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vs.iter().map(|v| v.uv).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, vs.iter().map(|v| v.tangent).collect::<Vec<_>>())
        .with_inserted_attribute(ATTR_HERO_JOINTS, VertexAttributeValues::Uint8x4(vs.iter().map(|v| v.joints).collect()))
        .with_inserted_attribute(ATTR_HERO_WEIGHTS, VertexAttributeValues::Unorm8x4(vs.iter().map(|v| v.weights).collect()))
        .with_inserted_indices(Indices::U32(idx))
}

/// A texture of `tex/`, once; a missing or bad one is a neutral colour, never a failure (the first enable's
/// checks have already looked inside every file).
fn texture(id: &str, name: &str, srgb: bool, neutral: [u8; 4], images: &mut Assets<Image>, cache: &mut HashMap<(String, bool), Handle<Image>>) -> Handle<Image> {
    if let Some(h) = cache.get(&(name.to_string(), srgb)) {
        return h.clone();
    }
    let read = || -> Result<Image, String> {
        let path = crate::mods::mod_file(id, &format!("tex/{name}.dds")).ok_or_else(|| "no file".to_string())?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let info = se_core::dds::parse_dds(&bytes).map_err(|e| e.to_string())?;
        crate::mr_assets::dds_image(&bytes, &info, srgb)
    };
    let image = (!name.is_empty()).then(read).unwrap_or_else(|| Err("no texture".to_string())).unwrap_or_else(|e| {
        if !name.is_empty() {
            warn!("viltrumite: texture {name}: {e}");
        }
        let mut i = crate::ship_assets::image_with_mips(neutral.to_vec(), 1, 1, if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm });
        i.asset_usage = RenderAssetUsages::RENDER_WORLD;
        i
    });
    let h = images.add(image);
    cache.insert((name.to_string(), srgb), h.clone());
    h
}

/// First enable: his meshes, textures, materials, buffers and entities. Hidden, never respawned, afterwards.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_hero(
    mut commands: Commands,
    control: Res<ModControl>,
    mut assets: ResMut<HeroAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<HeroMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let started = std::time::Instant::now();
    let id = control.active.clone().unwrap_or_default();
    let frame = Box::<HeroFrameGpu>::default();
    let frame_buffer = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&*frame), RenderAssetUsages::default()));
    let light = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&HeroLightGpu::default()), RenderAssetUsages::RENDER_WORLD));
    let mut cache = HashMap::new();
    let model = &assets.model;
    let entities = model
        .draws
        .iter()
        .map(|d| {
            let material = HeroMaterial {
                frame: frame_buffer.clone(),
                light: light.clone(),
                diffuse: texture(&id, &d.diffuse, true, [160, 160, 160, 255], &mut images, &mut cache),
                normal: texture(&id, &d.normal, false, [128, 128, 255, 255], &mut images, &mut cache),
                spec: texture(&id, &d.spec, false, [0, 0, 0, 255], &mut images, &mut cache),
                cutout: d.cutout(),
            };
            commands
                .spawn((
                    Name::new(format!("Hero {}", d.name)),
                    Mesh3d(meshes.add(draw_mesh(model, d))),
                    MeshMaterial3d(materials.add(material)),
                    Transform::IDENTITY,
                    Visibility::Visible,
                    NoFrustumCulling,
                    RenderLayers::layer(crate::ship::warp::SHIP_LAYER),
                ))
                .id()
        })
        .collect();
    // The meshes are on their way to the GPU: the main world keeps only the skeleton and the clips.
    assets.model.vertices = Vec::new();
    assets.model.indices = Vec::new();
    info!("viltrumite: meshes and {} textures made in {} ms", cache.len(), started.elapsed().as_millis());
    commands.insert_resource(HeroLightInput { on: true, frame: frame_buffer.clone(), light, tables: None });
    commands.insert_resource(HeroDraws { entities, frame_buffer, frame, ready: false, settle: 4, started, was_near: None, town_timer: 0, town_task: None });
}

/// He shows while the mod is on. Runs only after the first enable.
pub(super) fn sync_visibility(control: Res<ModControl>, draws: Res<HeroDraws>, mut vis: Query<&mut Visibility>, mut input: ResMut<HeroLightInput>, mut hero: ResMut<Hero>) {
    if !control.is_changed() && !draws.is_added() {
        return;
    }
    let want = if control.hero { Visibility::Visible } else { Visibility::Hidden };
    for &e in &draws.entities {
        if let Ok(mut v) = vis.get_mut(e) {
            if *v != want {
                *v = want;
            }
        }
    }
    if input.on != control.hero {
        input.on = control.hero;
    }
    if !control.hero {
        hero.placed = false;
    }
}

/// What his light is worked out from.
#[derive(SystemParam)]
pub(crate) struct LightParams<'w, 's> {
    data: Option<Res<'w, SeData>>,
    fade: Res<'w, LightingFade>,
    settings: Res<'w, StarSettings>,
    ambient: Res<'w, AmbientLighting>,
    caches: Res<'w, TileCaches>,
    bodies: Query<'w, 's, (Entity, &'static SystemBody, Option<&'static Terrain>)>,
    occluders: Query<'w, 's, (&'static SystemBody, Option<&'static Terrain>)>,
    sprites: Query<'w, 's, &'static BatchedSprite>,
}

/// The light record from a near body with drawn ground, as the scatter's objects take it; `false` if there is
/// no such body.
fn body_light(f: &mut HeroFrameGpu, b: &SystemBody, t: &Terrain, cam: UPos) -> bool {
    let Some(u) = t.body_uniform.as_ref() else { return false };
    if !t.body_light_fresh {
        return false;
    }
    let m = crate::scatter::DMat3Ext::from_quat_scale(b.orient, b.shape);
    f.body = LightBodyGpu {
        m0: m.x_axis.extend(0.0),
        m1: m.y_axis.extend(0.0),
        m2: m.z_axis.extend(0.0),
        centre: b.pos.delta_m(cam).as_vec3().extend(t.radius as f32),
        atm_flags: Vec4::new(t.body_light.flags.x, u.atm_flags.y, u.atm_flags.z, u.atm_flags.w),
        key: Vec4::new(u.proc_params2.x, u.proc_params2.y, if t.atmosphere.is_some() { 1.0 } else { 0.0 }, 0.0),
    };
    f.atm = u.atm;
    f.light = t.body_light;
    f.free_dir = [Vec4::ZERO; 3];
    f.free_color = [Vec4::ZERO; 3];
    true
}

struct FreeLight {
    flux: f64,
    dir: Vec3,
    color: Vec3,
    star: bool,
}

/// The same record filled by the CPU: the stars of the most flux at `pos` (as ship.rs lights a ship), each
/// dimmed by what lies on its way, and the shine of the bodies around him (as system.rs `update_lighting`
/// lights a body). No tables, true metres. The brightest star is the record's main light, so its highlight and
/// the rest of `point_light_at` hold; the further lights go to `free_*`, taken with his own normals.
fn free_light(f: &mut HeroFrameGpu, p: &LightParams, pos: UPos) {
    use se_core::units::METERS_PER_AU;
    let mut lights: Vec<FreeLight> = Vec::with_capacity(8);
    let mut stars: Vec<(f64, UPos, Vec3)> = Vec::with_capacity(4);
    if let Some(cat) = p.data.as_deref().map(|d| &d.catalog) {
        for (b, _) in p.occluders.iter().filter(|b| b.0.star) {
            let ObjectRef::Star(i) = b.obj else { continue };
            let s = &cat.stars[i as usize];
            let lum = se_core::star::lum_from_abs_mag(s.abs_mag);
            let d = b.pos.delta_m(pos).length().max(1.0);
            let c = illum_color(Vec3::from_array(crate::stars::star_render_color(s.class.kind, s.teff)), s.teff as f32, &p.settings);
            let tint = star_tint(b, pos, &p.occluders, true);
            lights.push(FreeLight { flux: lum / (d / METERS_PER_AU).powi(2), dir: b.pos.delta_m(pos).normalize_or(DVec3::Y).as_vec3(), color: c * tint, star: true });
            stars.push((lum, b.pos, c));
        }
        lights.sort_by(|a, b| b.flux.total_cmp(&a.flux));
        if p.fade.planet_shine && !stars.is_empty() {
            let end_flux = p.fade.end_flux() / p.fade.planet_shine_bright.max(1e-30);
            let mut lit: Vec<(f64, UPos, Vec3)> = Vec::with_capacity(4);
            for (e, b, t) in &p.bodies {
                let ObjectRef::Body(i) = b.obj else { continue };
                if b.star {
                    continue;
                }
                let albedo = Some(cat.bodies[i as usize].albedo).filter(|&a| a > 0.0).unwrap_or(0.36);
                lit.clear();
                let mut most = 0.0f64;
                for &(lum, star, c) in &stars {
                    let flux = lum / (star.delta_m(b.pos).length().max(1.0) / METERS_PER_AU).powi(2);
                    most = most.max(flux);
                    lit.push((flux, star, c));
                }
                // Most bodies are far too faint: skip them before the trigonometry (as update_lighting).
                let power = 2.0 * most * b.radius_m * b.radius_m * albedo * stars.len() as f64;
                if power < end_flux * b.pos.delta_m(pos).length_squared() {
                    continue;
                }
                let color = p.sprites.iter().find(|s| s.owner == e).map_or(Vec3::ONE, |s| Vec3::from_array(s.rgb));
                let hapke = t.map(|t| (t.surf_modulate.w, t.surf_hapke.x, t.surf_hapke.y));
                if let Some((flux, color)) = planet_shine(b.pos, b.radius_m, albedo, color, hapke, &lit, pos) {
                    let dir = b.pos.delta_m(pos).normalize_or(DVec3::Y).as_vec3();
                    lights.push(FreeLight { flux: flux * p.fade.planet_shine_bright, dir, color, star: false });
                }
            }
        }
    }
    if let Some((_, rest)) = lights.split_first_mut() {
        rest.sort_by(|a, b| b.flux.total_cmp(&a.flux));
    }
    lights.truncate(crate::system::MAX_LIGHTS);
    let k = p.fade.planet_intensities(&lights.iter().map(|l| (l.flux, l.star)).collect::<Vec<_>>());
    // (The list of intensities ends at the first light too faint to count: it can be empty, far from any star.)
    let (dir0, color0, k0) = lights.first().map_or((Vec3::Y, Vec3::ZERO, 0.0), |l| (l.dir, l.color, k.first().copied().unwrap_or(0.0)));
    f.light = PatchLight {
        sun_dir: dir0.extend(k0),
        sun_color: color0.extend(1.0),
        levels: Vec4::new(p.ambient.0, 0.0, 0.0, 0.0),
        ..default()
    };
    f.free_dir = [Vec4::ZERO; 3];
    f.free_color = [Vec4::ZERO; 3];
    for (i, (l, k)) in lights.iter().zip(&k).skip(1).enumerate() {
        f.free_dir[i] = l.dir.extend(1.0);
        f.free_color[i] = (l.color * *k * ATM_SUN_SCALE).extend(0.0);
    }
    // `point_light_at` sees a body 1,000 km under him along the main light, so its horizon is no shadow; the
    // record's clouds never match it.
    f.body = LightBodyGpu {
        m0: Vec4::X,
        m1: Vec4::Y,
        m2: Vec4::Z,
        centre: (f.point.truncate() - dir0 * 1.0e6).extend(1.0e3),
        atm_flags: Vec4::new(0.0, ATM_SUN_SCALE, 0.0, 0.0),
        key: Vec4::new(-1.0, 0.0, 0.0, 0.0),
    };
    f.atm = AtmosphereParamsUniform::default();
}

/// The street lamps where he is: their colour and strength, fading out a few hundred metres up, and a task
/// asking how much town stands under him (the ground's own density, from the sources it draws its streets by; a
/// map body's reads a lights tile, so never on the main thread). `None` where there are none: not over a colony's
/// domes, and not below level 2 (a village has hearths, no street lamps).
fn town_job(p: &LightParams, b: &SystemBody, t: &Terrain, pos: UPos) -> Option<(Vec3, Task<f32>)> {
    let (level, seed) = t.civ?;
    if t.domes {
        return None;
    }
    let local = b.from_shape(b.orient.inverse() * pos.delta_m(b.pos));
    let dir = local.try_normalize()?;
    let ground = t.surface_sample(dir)?.0;
    let height = local.length() - t.radius - ground;
    if height >= tune::TOWN_TOP_M {
        return None;
    }
    let ctx = t.town_ctx(&p.caches)?;
    let vfs = p.data.as_deref()?.vfs.clone();
    let rgb = if t.has_color_map() { [1.0, 0.68, 0.32] } else { culture(seed, level).light };
    let m = rgb[0].max(rgb[1]).max(rgb[2]).max(1e-3);
    let fade = 1.0 - smooth(tune::TOWN_FADE_M, tune::TOWN_TOP_M, height);
    let tint = Vec3::from_array(rgb) / m * fade as f32 * tune::TOWN_LAMPS;
    Some((tint, AsyncComputeTaskPool::get().spawn(async move { ctx.town_at(dir, ground as f32, &vfs) })))
}

fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Model space (Z up, +Y forward) to the port's (Y up, -Z forward); +X stays his right hand.
fn model_to_port() -> Mat3A {
    Mat3A::from_cols(Vec3A::X, Vec3A::new(0.0, 0.0, -1.0), Vec3A::Y)
}

/// His frame for this GPU frame, after the camera has moved. Everything he shows is worked out here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_hero(
    mut control: ResMut<ModControl>,
    time: Res<Time<Real>>,
    cam: Res<FlyCamera>,
    assets: Res<HeroAssets>,
    pipes: Res<crate::pipeline_watch::GroundPipelines>,
    mut hero: ResMut<Hero>,
    mut draws: ResMut<HeroDraws>,
    mut input: ResMut<HeroLightInput>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut proj: Query<&mut Projection, With<MainCamera>>,
    p: LightParams,
) {
    let dt = time.delta_secs().min(0.1);
    if !hero.placed {
        hero.place_ahead(&cam, &p.bodies);
        hero.start_clip(&assets.clips, "bf_idle");
    }
    // His place in the universe.
    hero.keep_standing(&assets, &p.bodies);
    let (pos, rot) = hero.world_pose(&cam, &p.bodies);
    let rel = pos.delta_m(cam.pos);
    // F7: `near_plane` is 30 m at 30 km height: pull it in so he isn't cut.
    let dist = rel.length();
    let near = (dist * tune::NEAR_SHARE).clamp(tune::NEAR_MIN_M, tune::NEAR_MAX_M) as f32;
    for mut pr in &mut proj {
        if let Projection::Perspective(q) = pr.as_mut() {
            if q.near > near {
                q.near = near;
            }
        }
    }
    // Loading: he stays degenerate until his pipelines are all built.
    if !draws.ready {
        let (built, building, _) = pipes.hero();
        if built >= HERO_PIPELINES && building == 0 {
            if draws.settle == 0 {
                draws.ready = true;
                control.loading = false;
                info!("viltrumite: ready, {built} pipelines, {:.1} s after the first enable", draws.started.elapsed().as_secs_f32());
            } else {
                draws.settle -= 1;
            }
        }
    } else if control.loading {
        control.loading = false;
    }
    let draws = &mut *draws;
    let f = &mut *draws.frame;
    let rel_f = rel.as_vec3();
    if draws.ready {
        let hero = &mut *hero;
        hero.sampler.advance(&assets.clips, dt);
        hero.sampler.sample(&assets.model, &assets.clips);
        let place = Affine3A { matrix3: Mat3A::from_quat(rot.as_quat()) * model_to_port(), translation: rel_f.into() };
        hero.sampler.write_palette(place, &mut f.joints);
    }
    hero.clock += dt as f64;
    // His chest.
    let nearest = p
        .bodies
        .iter()
        .filter(|(_, b, _)| !b.star && b.radius_m > 0.0)
        // (Above the radius in the sphere frame, the atmosphere's and the clouds' heights: on a flattened body the
        // radial distance reads up to flattening x radius too low.)
        .map(|(_, b, t)| (b, t, b.from_shape(b.orient.inverse() * pos.delta_m(b.pos)).length() - b.radius_m))
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let up = nearest.map_or(cam.rot * DVec3::Y, |(b, _, _)| pos.delta_m(b.pos).normalize_or(DVec3::Y)).as_vec3();
    f.point = (rel_f + up * 0.3).extend(0.0);
    f.up = up.extend(0.0);
    // The body whose light he takes: the nearest, under its atmosphere's top.
    let near_body = nearest.filter(|_| hero.light_path == LightPath::Auto).and_then(|(b, t, h)| {
        let t = t?;
        let limit = t.atmosphere.as_ref().map_or(b.radius_m * tune::BODY_LIGHT_AIRLESS_RADII, |a| a.top_radius_m(b.radius_m) - b.radius_m);
        (h < limit && body_light(f, b, t, cam.pos)).then_some(t)
    });
    if near_body.is_none() {
        free_light(f, &p, pos);
    }
    if draws.was_near != Some(near_body.is_some()) {
        draws.was_near = Some(near_body.is_some());
        info!("viltrumite: his light is now {} ({:.0} m above the nearest radius)", if near_body.is_some() { "the near body's" } else { "the CPU's" }, nearest.map_or(0.0, |n| n.2));
    }
    let tables = near_body.and_then(|t| t.atmosphere.as_ref()).map(|a| (a.inscatter.clone(), a.transmittance.clone(), a.irradiance.clone()));
    if input.tables != tables {
        input.tables = tables;
    }
    // How far into a town: not every frame, and off the main thread.
    if let Some((tint, task)) = draws.town_task.as_mut() {
        if let Some(share) = check_ready(task) {
            f.town = (*tint * share).extend(0.0);
            draws.town_task = None;
        }
    } else if draws.town_timer == 0 {
        draws.town_timer = tune::TOWN_EVERY_FRAMES;
        draws.town_task = nearest.filter(|_| near_body.is_some()).and_then(|(b, t, _)| town_job(&p, b, t?, pos));
        if draws.town_task.is_none() {
            f.town = Vec4::ZERO;
        }
    } else {
        draws.town_timer -= 1;
    }
    // In place: the buffer and his materials' bind groups stay.
    if let Some(mut b) = buffers.get_mut(&draws.frame_buffer) {
        if let Some(d) = b.cast_slice_mut::<u8>() {
            let _ = encase::StorageBuffer::new(d).write(&*f);
        }
    }
}

/// The pipelines he needs before he shows: opaque, cutout, the light's compute.
const HERO_PIPELINES: u32 = 3;

#[allow(clippy::too_many_arguments)]
fn prepare_light_pass(
    input: Res<HeroLightInput>,
    mut pass: ResMut<HeroLightPass>,
    shadow: Option<Res<CloudShadowAssets>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
) {
    pass.group = None;
    if !input.on {
        return;
    }
    if pass.layout.is_none() {
        let filt = || TextureSampleType::Float { filterable: true };
        let layout = BindGroupLayoutDescriptor::new(
            "hero_light",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_sized(false, None),
                    texture_3d(filt()),
                    texture_2d(filt()),
                    texture_2d(filt()),
                    sampler(SamplerBindingType::Filtering),
                    texture_cube(filt()),
                    sampler(SamplerBindingType::Filtering),
                    storage_buffer_read_only_sized(false, None),
                    texture_2d(filt()),
                    sampler(SamplerBindingType::Filtering),
                    texture_cube(filt()),
                    texture_cube(filt()),
                ),
            ),
        );
        pass.pipeline = Some(cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some("hero_light".into()),
            layout: vec![layout.clone()],
            shader: assets.load("shaders/hero_light.wesl"),
            entry_point: Some("light".into()),
            ..default()
        }));
        pass.layout = Some(layout);
    }
    let Some(sh) = shadow else { return };
    let (Some(frame), Some(light), Some(rec)) = (buffers.get(&input.frame), buffers.get(&input.light), buffers.get(&sh.shadow)) else { return };
    let get = |h: &Handle<Image>| images.get(h);
    let (Some(cover), Some(pattern), Some(map), Some(weather)) = (get(&sh.cover), get(&sh.pattern), get(&sh.map), get(&sh.weather)) else { return };
    let tables = input.tables.as_ref().and_then(|(s, t, e)| Some((images.get(s)?, images.get(t)?, images.get(e)?)));
    let (s3, t2, e2) = tables.map_or((&fallback.d3, &fallback.d2, &fallback.d2), |a| a);
    let layout = cache.get_bind_group_layout(pass.layout.as_ref().unwrap());
    let group = device.create_bind_group(
        "hero_light",
        &layout,
        &BindGroupEntries::sequential((
            frame.buffer.as_entire_binding(),
            light.buffer.as_entire_binding(),
            &s3.texture_view,
            &t2.texture_view,
            &e2.texture_view,
            &s3.sampler,
            &cover.texture_view,
            &cover.sampler,
            rec.buffer.as_entire_binding(),
            &map.texture_view,
            &map.sampler,
            &weather.texture_view,
            &pattern.texture_view,
        )),
    );
    pass.group = Some(group);
}

fn light_pass(_view: ViewQuery<&ExtractedView, MainView>, pass: Res<HeroLightPass>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    let Some(group) = pass.group.as_ref() else { return };
    let Some(pipeline) = pass.pipeline.and_then(|id| cache.get_compute_pipeline(id)) else { return };
    let mut compute = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("hero_light"), ..default() });
    compute.set_pipeline(pipeline);
    compute.set_bind_group(0, group, &[]);
    compute.dispatch_workgroups(1, 1, 1);
}

/// Where a `Place` puts him this frame, for the log.
pub(super) fn describe(place: &Place, bodies: &Query<(Entity, &SystemBody, Option<&Terrain>)>) -> String {
    match place {
        Place::Free => "free".to_string(),
        Place::Rides { locked, local, .. } => format!("riding what carries the camera ({}), {:.0} m from its centre", if *locked { "turning with it" } else { "moving with it" }, local.length()),
        Place::On { body, local } => match bodies.get(*body) {
            Ok((_, b, _)) => format!("{:.0} m above the radius of the body at {:.0} km", local.length() - b.radius_m, b.radius_m / 1000.0),
            Err(_) => "on a body gone".to_string(),
        },
    }
}
