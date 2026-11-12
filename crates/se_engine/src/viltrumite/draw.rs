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
//! - **Shadow.** The game has no shadow maps. A second entity per draw, the same meshes in a Multiply material, its
//!   vertices projected along the star onto the plane of the ground under him (`hero.wesl` `vertex_shadow`). To darken each
//!   pixel once however many triangles cover it, each fragment writes the depth of its own pixel's ray on that plane, the same
//!   bits for all of them, and the depth test is Greater, so the first one wins.
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
use crate::ship::BodyInfo;
use crate::stars::StarSettings;
use crate::system::{AmbientLighting, BatchedSprite, LightingFade, SystemBody, illum_color, planet_shine, star_tint};
use crate::terrain::{ATM_SUN_SCALE, PatchLight, Terrain, TileCaches};
use crate::volumetric_clouds::CloudShadowAssets;

use super::frames::{Frame, Ground};
use super::ground::{self, Footing, Soles};
use super::solid::Solids;
use super::tune;
use super::{Hero, LightPath};

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
pub(crate) struct HeroFrameGpu {
    joints: [[Vec4; 3]; MAX_BONES],
    point: Vec4,
    up: Vec4,
    town: Vec4,
    // His body's heat glow, gone (the user, 2026-11-12): always 0, kept so the record's layout stays (hero_frame.wesl, read by
    // sandbox_prop.wesl and sandbox_viewmodel.wesl too).
    heat: Vec4,
    heat_lee: Vec4,
    heat_dir: Vec4,
    shadow_plane: Vec4,
    shadow: Vec4,
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
            heat_lee: Vec4::ZERO,
            heat_dir: Vec4::ZERO,
            shadow_plane: Vec4::ZERO,
            shadow: Vec4::ZERO,
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
    /// His shadow's second draw of the mesh.
    pub shadow: bool,
}

/// Cutouts take the entry point with the discard, in the alpha-mask phase; the opaque keep the early depth test; the shadow
/// has its own entry points.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct HeroMaterialKey {
    cutout: bool,
    shadow: bool,
}

impl From<&HeroMaterial> for HeroMaterialKey {
    fn from(m: &HeroMaterial) -> Self {
        Self { cutout: m.cutout, shadow: m.shadow }
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
        if self.shadow {
            AlphaMode::Multiply
        } else if self.cutout {
            AlphaMode::Mask(0.5)
        } else {
            AlphaMode::Opaque
        }
    }
    /// The sort key of the transparent phase: the shadow goes after the sea (1e10), which writes depth: drawn first, the plane just
    /// above the water would keep the sea out of his whole silhouette, its waves and glint with it.
    fn depth_bias(&self) -> f32 {
        if self.shadow { 2.0e10 } else { 0.0 }
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
        let (cutout, shadow) = (key.bind_group_data.cutout, key.bind_group_data.shadow);
        descriptor.vertex.entry_point = Some(if shadow { "vertex_shadow" } else { "vertex" }.into());
        if let Some(f) = descriptor.fragment.as_mut() {
            f.entry_point = Some(if shadow { "fragment_shadow" } else if cutout { "fragment_cutout" } else { "fragment" }.into());
        }
        if cutout || shadow {
            // The hair is thin cards, seen from both sides; projected onto a plane either side of a triangle can face up.
            descriptor.primitive.cull_mode = None;
        }
        if shadow {
            if let Some(d) = descriptor.depth_stencil.as_mut() {
                // Strictly nearer than what's there: the plane's depth is the same bits for every triangle of a pixel, so the
                // first one wins and the others don't darken it again.
                d.depth_compare = Some(CompareFunction::Greater);
                d.depth_write_enabled = Some(true);
            }
        }
        Ok(())
    }
}

/// What his first enable made, kept until quit.
#[derive(Resource)]
pub struct HeroDraws {
    entities: Vec<Entity>,
    /// His shadow's draws, and whether they're showing.
    shadows: Vec<Entity>,
    shadow_on: bool,
    /// The normal of its plane last frame, in the frame of that body.
    shadow_normal: Option<(ObjectRef, DVec3)>,
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
    /// His body as capsules round his bones, as drawn this frame, which the craters leave alone.
    parts: super::impact::BodyParts,
}

impl HeroDraws {
    /// His body as capsules, as drawn this frame (`fx.rs` copies them into the craters' records).
    pub(super) fn parts(&self) -> &super::impact::BodyParts {
        &self.parts
    }

    /// His pipelines are built: he shows.
    pub(super) fn ready(&self) -> bool {
        self.ready
    }

    /// His frame's records (`HeroFrame`), written in place each frame: the rims read his light's direction and up from it.
    pub(super) fn frame_buffer(&self) -> Handle<ShaderBuffer> {
        self.frame_buffer.clone()
    }

    /// Towards the star his light comes from.
    pub(super) fn sun_dir(&self) -> Vec3 {
        self.frame.light.sun_dir.truncate().normalize_or(Vec3::Y)
    }

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
    tables: Option<LightTables>,
}

impl HeroLightInput {
    /// The near body's atmosphere tables for the light pass, when they change.
    fn take_tables(&mut self, near_body: Option<&Terrain>) {
        let tables = near_body.and_then(|t| t.atmosphere.as_ref()).map(|a| (a.inscatter.clone(), a.transmittance.clone(), a.irradiance.clone()));
        if self.tables != tables {
            self.tables = tables;
        }
    }
}

/// The light pass (`hero_light.wesl`) for one frame record and one light record: his own, and the sandbox's (its props
/// and weapon are lit at the player's place as he is at his: `sandbox/light.rs`). The pipeline is queued the first time it's on.
#[derive(Default)]
pub(crate) struct LightPass {
    layout: Option<BindGroupLayoutDescriptor>,
    pipeline: Option<CachedComputePipelineId>,
    group: Option<BindGroup>,
}

/// The compute pass's pipeline, queued on first enable.
#[derive(Resource, Default)]
struct HeroLightPass(LightPass);

/// A light record of its own for something else lit as he is: the sandbox's props and weapon (`sandbox/light.rs`). Filled
/// each frame for a place; the light pass then works the light out there exactly as it does for him.
pub(crate) struct LightProbe {
    frame: Box<HeroFrameGpu>,
    /// The street lamps at its place, as at his (`town_light`).
    town_timer: u32,
    town_task: Option<(Vec3, Task<f32>)>,
}

impl LightProbe {
    pub(crate) fn new() -> Self {
        LightProbe { frame: Box::default(), town_timer: 0, town_task: None }
    }

    /// The frame record's bytes (`HeroFrame`), and an empty light record's (`HeroLight`), to make the buffers from.
    pub(crate) fn frame_bytes(&self) -> Vec<u8> {
        crate::stars::gpu_bytes(&*self.frame)
    }

    pub(crate) fn light_bytes() -> Vec<u8> {
        crate::stars::gpu_bytes(&HeroLightGpu::default())
    }

    /// The record for `pos`, seen from the camera at `cam`; the body whose atmosphere tables the pass must read.
    pub(crate) fn fill<'a>(&mut self, p: &'a LightParams, pos: UPos, cam: UPos, cam_up: DVec3) -> Option<&'a Terrain> {
        let (nearest, near) = light_record(&mut self.frame, p, pos, cam, cam_up, LightPath::Auto);
        town_light(&mut self.frame, &mut self.town_timer, &mut self.town_task, || nearest.filter(|_| near.is_some()).and_then(|(b, t, _)| town_job(p, b, t?, pos)));
        near
    }

    /// Write the record into its buffer, in place.
    pub(crate) fn write(&self, buffer: &mut ShaderBuffer) {
        if let Some(d) = buffer.cast_slice_mut::<u8>() {
            let _ = encase::StorageBuffer::new(d).write(&*self.frame);
        }
    }
}

/// The view he draws in: the main camera (as the scatter's).
pub(crate) type MainView = (With<ColorCorrection>, Without<crate::capture::CaptureView>);

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
    mut hero: ResMut<Hero>,
) {
    let started = std::time::Instant::now();
    let id = control.active.clone().unwrap_or_default();
    let frame = Box::<HeroFrameGpu>::default();
    let frame_buffer = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&*frame), RenderAssetUsages::default()));
    let light = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&HeroLightGpu::default()), RenderAssetUsages::RENDER_WORLD));
    let mut cache = HashMap::new();
    let model = &assets.model;
    hero.soles = Soles::of(model);
    if hero.soles.is_none() {
        warn!("viltrumite: no foot bones in the model: his feet go by the clips alone");
    }
    let mut shadows = Vec::new();
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
                shadow: false,
            };
            let mesh = meshes.add(draw_mesh(model, d));
            // The shadow leaves out the cutouts (over half his triangles are hair, cut by a texture this draw doesn't read).
            if !d.cutout() {
                shadows.push(
                    commands
                        .spawn((
                            Name::new(format!("Hero shadow {}", d.name)),
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(materials.add(HeroMaterial { shadow: true, ..material.clone() })),
                            Transform::IDENTITY,
                            Visibility::Visible,
                            NoFrustumCulling,
                            RenderLayers::layer(crate::ship::warp::SHIP_LAYER),
                        ))
                        .id(),
                );
            }
            commands
                .spawn((
                    Name::new(format!("Hero {}", d.name)),
                    Mesh3d(mesh),
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
    super::fx::spawn(&mut commands, &mut buffers, light.clone());
    commands.insert_resource(HeroLightInput { on: true, frame: frame_buffer.clone(), light, tables: None });
    commands.insert_resource(HeroDraws { entities, shadows, shadow_on: true, shadow_normal: None, frame_buffer, frame, ready: false, settle: 4, started, was_near: None, town_timer: 0, town_task: None, parts: default() });
}

/// He shows while the mod is on. Runs only after the first enable.
pub(super) fn sync_visibility(control: Res<ModControl>, mut draws: ResMut<HeroDraws>, mut vis: Query<&mut Visibility>, mut input: ResMut<HeroLightInput>) {
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
    if !control.hero && draws.shadow_on {
        draws.shadow_on = false;
        for &e in &draws.shadows {
            if let Ok(mut v) = vis.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
    }
    if input.on != control.hero {
        input.on = control.hero;
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
    clouds: Res<'w, crate::volumetric_clouds::VolumetricClouds>,
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
fn free_light(f: &mut HeroFrameGpu, p: &LightParams, pos: UPos, rel: Vec3, fill: f32) {
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
    let mut slots = 0;
    for (i, (l, k)) in lights.iter().zip(&k).skip(1).enumerate() {
        f.free_dir[i] = l.dir.extend(1.0);
        f.free_color[i] = (l.color * *k * ATM_SUN_SCALE).extend(0.0);
        slots = i + 1;
    }
    // Far from any light he'd be black against the stars: a faint light from the camera keeps him in sight. It goes as the
    // real light comes, so nothing near a star changes.
    let seen = (color0 * k0).max_element() * ATM_SUN_SCALE;
    if seen < fill && slots < f.free_dir.len() {
        f.free_dir[slots] = (-rel).normalize_or(Vec3::Y).extend(1.0);
        f.free_color[slots] = Vec3::splat(fill - seen).extend(0.0);
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

/// The street lamps' light in a frame record (`town`): how far into a town its place is, asked every `TOWN_EVERY_FRAMES`
/// and off the main thread (`job`: `town_job`), for him and for the sandbox's props and weapon.
fn town_light(f: &mut HeroFrameGpu, timer: &mut u32, task: &mut Option<(Vec3, Task<f32>)>, job: impl FnOnce() -> Option<(Vec3, Task<f32>)>) {
    if let Some((tint, t)) = task.as_mut() {
        if let Some(share) = check_ready(t) {
            f.town = (*tint * share).extend(0.0);
            *task = None;
        }
    } else if *timer == 0 {
        *timer = tune::TOWN_EVERY_FRAMES;
        *task = job();
        if task.is_none() {
            f.town = Vec4::ZERO;
        }
    } else {
        *timer -= 1;
    }
}

fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the light above gets down to him in a giant's decks, `h` m over its radius, its scale height `scale` m. (The ground's rule
/// for a cloud shadow, whose floor is a quarter, would leave him brighter than the deck round him by ten times and more.)
fn deck_light(h: f64, scale: f64) -> f64 {
    let d = (tune::DECK_TOP_H - h / scale).max(0.0);
    let share = (tune::DECK_NEAR.0 * (-d / tune::DECK_NEAR.1).exp() + tune::DECK_FAR.0 * (-d / tune::DECK_FAR.1).exp()).min(1.0);
    share * (1.0 - smooth(tune::DECK_DARK_H.0, tune::DECK_DARK_H.1, -(-h / scale)))
}

/// The body he's attached to, as the frames see it, and his place in its frame.
fn attached_body<'a>(hero: &Hero, p: &'a LightParams) -> Option<(BodyInfo<'a>, DVec3)> {
    let Frame::Attached { obj, local_pos, .. } = hero.frame else { return None };
    let (_, b, t) = p.bodies.iter().find(|(_, b, _)| b.obj == obj)?;
    Some((BodyInfo { obj, pos: b.pos, orient: b.orient, radius: b.radius_m, shape: b.shape, gm: 0.0, terrain: t }, local_pos))
}

/// Where the star's ray through `p` meets the ground (body frame; `l` towards the star), and `None` where nothing is loaded or a star
/// that low never gets there. By steps of a share of what the height says is left, exact on flat ground, then bisection: over
/// uneven ground, or with the star low, one long step overshoots by many times the height.
fn ray_landing(ground: impl Fn(DVec3) -> Ground, p: DVec3, l: DVec3) -> Option<DVec3> {
    let over = |q: DVec3| {
        let g = ground(q);
        g.level.map(|_| q.length() - g.radial)
    };
    let sine = l.dot(p.normalize_or(DVec3::Y));
    let cos = sine.max(tune::SHADOW_RAY_MIN_COS);
    let (mut last, mut h) = (0.0, over(p)?);
    if h <= 0.0 {
        return Some(p);
    }
    if sine <= 0.0 {
        return None;
    }
    for _ in 0..tune::SHADOW_RAY_STEPS {
        let t = last + (h / cos * tune::SHADOW_RAY_SHARE).max(tune::SHADOW_RAY_MIN_STEP_M);
        let hq = over(p - l * t)?;
        if hq <= 0.0 {
            let (mut above, mut under) = (last, t);
            for _ in 0..tune::SHADOW_RAY_BISECT {
                let mid = 0.5 * (above + under);
                if over(p - l * mid)? <= 0.0 {
                    under = mid;
                } else {
                    above = mid;
                }
            }
            return Some(p - l * under);
        }
        (last, h) = (t, hq);
    }
    None
}

/// Where his shadow falls, in the body's frame: the ground the star's ray through his soles lands on and the one through his
/// head does, and the ground's normal along that. Their line is what the shadow's plane contains, so the shadow starts at his
/// boots and ends where the ground really is there, whatever it does between.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShadowFall {
    start: DVec3,
    end: DVec3,
    normal: DVec3,
    /// How strong (1 up to `SHADOW_FADE_M.0` over the ground, to 0).
    strength: f64,
}

/// `None`: no shadow, and nothing for the ground to hold still for: nothing to stand on, nothing loaded, or he's too high.
/// (On a building's roof the roof is the ground: `solids`, standing on `b`.)
fn shadow_fall(b: &BodyInfo, local: DVec3, sun: DVec3, solids: Option<&Solids>) -> Option<ShadowFall> {
    if ground::footing(b, local) == Footing::Nothing {
        return None;
    }
    fall_over(|p: DVec3| ground::with_roof(ground::support_height(b, p, None, 0.0), solids, p, 0.0), local, (b.orient.inverse() * sun).normalize_or(local.normalize_or(DVec3::Y)))
}

/// The same over any ground, `l` towards the star, both in the body's frame.
fn fall_over(ground: impl Fn(DVec3) -> Ground, local: DVec3, l: DVec3) -> Option<ShadowFall> {
    let up = local.normalize_or(DVec3::Y);
    let g = ground(local);
    g.level?;
    let height = local.length() - g.radial - tune::STAND_M;
    let (from, to) = tune::SHADOW_FADE_M;
    let strength = 1.0 - smooth(from, to, height);
    if strength <= 0.0 {
        return None;
    }
    let soles = local - up * tune::STAND_M;
    let head = soles + up * tune::SHADOW_TALL_M;
    // On the ground under the place, if the star's too low for its ray to get there.
    let on_ground = |p: DVec3| p.normalize_or(up) * ground(p).radial;
    let start = ray_landing(&ground, soles, l).unwrap_or_else(|| on_ground(soles));
    // (Far out at a low sun the star's light on the plane is next to nothing, and its reach is what the ground is held for.)
    let reach = smooth(tune::SHADOW_LOW_SUN.0, tune::SHADOW_LOW_SUN.1, l.dot(up)) * tune::SHADOW_REACH_M;
    let end = ray_landing(&ground, head, l).map_or(start, |e| start + (e - start).clamp_length_max(reach));
    let end = on_ground(end);
    let [p0, p1, p2] = ground::ground_triangle(&ground, (start + end) * 0.5, tune::SHADOW_SPAN_M)?;
    let n = (p1 - p0).cross(p2 - p0).normalize_or(up);
    let n = if n.dot(up) < 0.0 { -n } else { n };
    // That plane, turned to contain the line, by how long the line is.
    let along = end - start;
    let len = along.length();
    let flat = if len > 1e-3 { (n - along / len * n.dot(along) / len).normalize_or(n) } else { n };
    let normal = n.lerp(flat, smooth(tune::SHADOW_LINE_M.0, tune::SHADOW_LINE_M.1, len)).normalize_or(n);
    Some(ShadowFall { start, end, normal, strength })
}

/// The plane his shadow falls on, in the render frame (xyz its normal, w so `dot(n, x) + w` is the height over it), lifted a little.
fn shadow_plane(b: &BodyInfo, fall: &ShadowFall, normal: DVec3, cam: UPos) -> Vec4 {
    let n_world = b.orient * normal;
    let base = b.pos.offset_m(b.orient * fall.start).delta_m(cam);
    n_world.as_vec3().extend((-n_world.dot(base) - tune::SHADOW_LIFT_M) as f32)
}

/// Model space (Z up, +Y forward) to the port's (Y up, -Z forward); +X stays his right hand.
fn model_to_port() -> Mat3A {
    Mat3A::from_cols(Vec3A::X, Vec3A::new(0.0, 0.0, -1.0), Vec3A::Y)
}

/// The light record at `pos` (`HeroFrame`'s `point` 0.3 m over it, `up`, the near body's record or the CPU's, the faint fill), from
/// the camera at `cam`; `cam_up` the up with no body near. Returns the nearest body (with the height over its radius) and the body
/// whose record and tables were taken. His own (`update_hero`); in the aftermath the nearest crater's (`aftermath_light`).
fn light_record<'a>(f: &mut HeroFrameGpu, p: &'a LightParams, pos: UPos, cam: UPos, cam_up: DVec3, path: LightPath) -> (Option<(&'a SystemBody, Option<&'a Terrain>, f64)>, Option<&'a Terrain>) {
    let rel_f = pos.delta_m(cam).as_vec3();
    let nearest = p
        .bodies
        .iter()
        .filter(|(_, b, _)| !b.star && b.radius_m > 0.0)
        // (Above the radius in the sphere frame, the atmosphere's and the clouds' heights: on a flattened body the
        // radial distance reads up to flattening x radius too low.)
        .map(|(_, b, t)| (b, t, b.from_shape(b.orient.inverse() * pos.delta_m(b.pos)).length() - b.radius_m))
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let up = nearest.map_or(cam_up, |(b, _, _)| pos.delta_m(b.pos).normalize_or(DVec3::Y)).as_vec3();
    // Deep in the decks of a giant whose clouds are on: only they can take him under its radius, and only what they let through reaches him.
    let down = nearest.and_then(|(b, t, h)| {
        let t = t?;
        let decks = t.proc.kind.is_giant() && p.bodies.iter().any(|(e, o, _)| o.obj == b.obj && p.clouds.active == Some(e));
        decks.then(|| deck_light(h, crate::volumetric_clouds::giant_scale_height(t)))
    });
    f.point = (rel_f + up * 0.3).extend(down.unwrap_or(1.0) as f32);
    f.up = up.extend(0.0);
    // The body whose light he takes: the nearest, under its atmosphere's top.
    let near_body = nearest.filter(|_| path == LightPath::Auto).and_then(|(b, t, h)| {
        let t = t?;
        let limit = t.atmosphere.as_ref().map_or(b.radius_m * tune::BODY_LIGHT_AIRLESS_RADII, |a| a.top_radius_m(b.radius_m) - b.radius_m);
        (h < limit && body_light(f, b, t, cam)).then_some(t)
    });
    if near_body.is_none() {
        // Near a body the night's level, in its shadow in orbit too; the top of its air mustn't be a step in his brightness.
        let far = nearest.map_or(1.0, |(b, _, h)| smooth(tune::FILL_NEAR_RADII * b.radius_m, tune::FILL_FAR_RADII * b.radius_m, h)) as f32;
        let fill = tune::FAINT_FILL_NIGHT + (tune::FAINT_FILL - tune::FAINT_FILL_NIGHT) * far;
        free_light(f, p, pos, rel_f, fill);
    } else {
        // The same faint light once the star has set: over a dark sea at night he'd be nowhere otherwise.
        let night = smooth(tune::FILL_NIGHT_FROM, tune::FILL_NIGHT_TO, -(f.light.sun_dir.truncate().normalize_or(Vec3::Y).dot(up) as f64)) as f32;
        // (Below a giant's decks, where no light gets down.)
        let fill = (tune::FAINT_FILL_NIGHT * night).max(tune::FAINT_FILL_DEEP * (1.0 - smooth(0.0, 0.03, down.unwrap_or(1.0)) as f32));
        if fill > 0.0 {
            f.free_dir[0] = (-rel_f).normalize_or(Vec3::Y).extend(1.0);
            f.free_color[0] = Vec3::splat(fill).extend(0.0);
        }
    }
    (nearest, near_body)
}

/// The aftermath (F3: the mod off, craters kept): the rims and the crater bowls take his light record (`hero_rim.wesl`, `hero_fx.wesl`
/// `light_on`), so it's worked out at the crater (or dust) nearest the camera instead of him, and the light pass runs for it while
/// there's one. Frozen at the hand-back, a crater's rocks stayed in the afternoon's sun into the night. While he's on, `update_hero`
/// writes it; with the pools empty this turns the light pass off again (the last frame these systems run).
#[allow(clippy::too_many_arguments)]
pub(super) fn aftermath_light(
    control: Res<ModControl>,
    cam: Res<FlyCamera>,
    impacts: Res<super::impact::Impacts>,
    mut draws: ResMut<HeroDraws>,
    mut input: ResMut<HeroLightInput>,
    mut fx: ResMut<super::fx::FxDraws>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    p: LightParams,
) {
    if control.hero {
        return;
    }
    let spot = impacts.light_spot();
    if input.on != spot.is_some() {
        input.on = spot.is_some();
    }
    let Some(pos) = spot else { return };
    let draws = &mut *draws;
    let f = &mut *draws.frame;
    let (_, near_body) = light_record(f, &p, pos, cam.pos, cam.rot * DVec3::Y, LightPath::Auto);
    input.take_tables(near_body);
    fx.records.sun = f.light.sun_dir.truncate().normalize_or(Vec3::Y).extend(0.0);
    if let Some(mut b) = buffers.get_mut(&draws.frame_buffer) {
        if let Some(d) = b.cast_slice_mut::<u8>() {
            let _ = encase::StorageBuffer::new(d).write(&*f);
        }
    }
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
    mut vis: Query<&mut Visibility>,
    mut soil: ResMut<crate::volumetric_clouds::HeroSoil>,
    solids: Res<Solids>,
    p: LightParams,
) {
    let dt = time.delta_secs().min(0.1);
    // His place in the universe (`hero::hero_move`, earlier this frame).
    let (pos, rot) = hero.world;
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
    let body = attached_body(&hero, &p);
    // The buildings' roofs, where they stand on his body (step 9): his feet and his shadow take them as the ground.
    let solids = body.as_ref().and_then(|(b, _)| Some(&*solids).filter(|s| s.on(b.obj)));
    let up_there = body.as_ref().map(|(b, local)| b.orient * local.normalize_or(DVec3::Y));
    if draws.ready {
        let hero = &mut *hero;
        hero.sampler.advance(&assets.clips, dt);
        hero.sampler.sample(&assets.model, &assets.clips);
        let lift = ground::post_pose(hero, &assets.clips, body.as_ref(), solids, dt as f64);
        let lifted = rel + up_there.unwrap_or(DVec3::Y) * lift;
        let place = Affine3A { matrix3: Mat3A::from_quat(rot.as_quat()) * model_to_port(), translation: lifted.as_vec3().into() };
        hero.sampler.write_palette(place, &mut f.joints);
        draws.parts.place(&hero.sampler, &assets.model, place);
    }
    // His chest.
    let (nearest, near_body) = light_record(f, &p, pos, cam.pos, cam.rot * DVec3::Y, hero.light_path);
    if draws.was_near != Some(near_body.is_some()) {
        draws.was_near = Some(near_body.is_some());
        info!("viltrumite: his light is now {} ({:.0} m above the nearest radius)", if near_body.is_some() { "the near body's" } else { "the CPU's" }, nearest.map_or(0.0, |n| n.2));
    }
    let fall = body.as_ref().and_then(|(b, local)| Some((b, shadow_fall(b, *local, f.light.sun_dir.truncate().as_dvec3(), solids)?)));
    // The ground holds still where his feet and his shadow lie (`planet.wesl`), a line in the camera's frame, until the shadow has nearly
    // faded, and as that goes, so nothing changes at one height.
    let soil_now = match &fall {
        Some((b, fall)) if !hero.soil_off && ground::footing(b, fall.start) == Footing::Land => {
            let at = |p: DVec3| b.pos.offset_m(b.orient * p).delta_m(cam.pos).as_vec3();
            let hold = (fall.strength / tune::SOIL_HOLD_SHARE).min(1.0) as f32;
            crate::volumetric_clouds::HeroSoil { start: at(fall.start).extend(hold), end: at(fall.end).extend(0.0) }
        }
        _ => default(),
    };
    if *soil != soil_now {
        *soil = soil_now;
    }
    // His shadow: only while there's a plane to fall on; hidden, its 100,000 triangles aren't drawn. The plane's turn is eased, or at a
    // low sun its far end, grazing over bumps, flicks it as he walks; it still goes through where his soles' ray lands.
    let shadow = fall.filter(|_| !hero.shadow_off).map(|(b, fall)| {
        let eased = draws.shadow_normal.filter(|(obj, _)| *obj == b.obj).map_or(fall.normal, |(_, was)| was.lerp(fall.normal, 1.0 - (-(dt as f64) / tune::SHADOW_EASE_S).exp()).normalize_or(fall.normal));
        draws.shadow_normal = Some((b.obj, eased));
        (shadow_plane(b, &fall, eased, cam.pos), fall.strength as f32)
    });
    (f.shadow_plane, f.shadow) = shadow.map_or((Vec4::ZERO, Vec4::ZERO), |(plane, k)| (plane, Vec4::new(k, 0.0, 0.0, 0.0)));
    if draws.ready && draws.shadow_on != shadow.is_some() {
        draws.shadow_on = shadow.is_some();
        let want = if shadow.is_some() { Visibility::Visible } else { Visibility::Hidden };
        for &e in &draws.shadows {
            if let Ok(mut v) = vis.get_mut(e) {
                *v = want;
            }
        }
    }
    input.take_tables(near_body);
    town_light(f, &mut draws.town_timer, &mut draws.town_task, || nearest.filter(|_| near_body.is_some()).and_then(|(b, t, _)| town_job(&p, b, t?, pos)));
    // In place: the buffer and his materials' bind groups stay.
    if let Some(mut b) = buffers.get_mut(&draws.frame_buffer) {
        if let Some(d) = b.cast_slice_mut::<u8>() {
            let _ = encase::StorageBuffer::new(d).write(&*f);
        }
    }
}

/// The pipelines he needs before he shows: opaque, cutout, the shadow, the light's compute.
const HERO_PIPELINES: u32 = 4;

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
    pass.0.prepare(input.on, &input.frame, &input.light, input.tables.as_ref(), shadow.as_deref(), &buffers, &images, &fallback, &cache, &device, &assets);
}

/// The body's inscatter, transmittance and irradiance tables.
pub(crate) type LightTables = (Handle<Image>, Handle<Image>, Handle<Image>);

impl LightPass {
    /// This frame's bind group (none while it's off or something of it isn't on the GPU yet).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        on: bool,
        frame: &Handle<ShaderBuffer>,
        light: &Handle<ShaderBuffer>,
        tables: Option<&LightTables>,
        shadow: Option<&CloudShadowAssets>,
        buffers: &RenderAssets<GpuShaderBuffer>,
        images: &RenderAssets<GpuImage>,
        fallback: &FallbackImage,
        cache: &PipelineCache,
        device: &RenderDevice,
        assets: &AssetServer,
    ) {
        let pass = self;
    pass.group = None;
    if !on {
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
    let (Some(frame), Some(light), Some(rec)) = (buffers.get(frame), buffers.get(light), buffers.get(&sh.shadow)) else { return };
    let get = |h: &Handle<Image>| images.get(h);
    let (Some(cover), Some(pattern), Some(map), Some(weather)) = (get(&sh.cover), get(&sh.pattern), get(&sh.map), get(&sh.weather)) else { return };
    let tables = tables.and_then(|(s, t, e)| Some((images.get(s)?, images.get(t)?, images.get(e)?)));
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

    /// The pass itself: one invocation.
    pub(crate) fn run(&self, cache: &PipelineCache, ctx: &mut RenderContext) {
        let Some(group) = self.group.as_ref() else { return };
        let Some(pipeline) = self.pipeline.and_then(|id| cache.get_compute_pipeline(id)) else { return };
        let mut compute = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("hero_light"), ..default() });
        compute.set_pipeline(pipeline);
        compute.set_bind_group(0, group, &[]);
        compute.dispatch_workgroups(1, 1, 1);
    }
}

fn light_pass(_view: ViewQuery<&ExtractedView, MainView>, pass: Res<HeroLightPass>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    pass.0.run(&cache, &mut ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 1.0e6;

    /// x metres along the ground from the place under him, h over the radius.
    fn at(x: f64, h: f64) -> DVec3 {
        DVec3::new(x, R, 0.0).normalize() * (R + h)
    }

    fn over(height: impl Fn(f64) -> f64) -> impl Fn(DVec3) -> Ground {
        move |p: DVec3| Ground { radial: R + height((p / p.y).x * R), level: Some(9) }
    }

    /// The star at this height, on the -x side: the shadow falls towards +x.
    fn sun(deg: f64) -> DVec3 {
        let (s, c) = deg.to_radians().sin_cos();
        DVec3::new(-c, s, 0.0)
    }

    fn along(p: DVec3) -> f64 {
        (p / p.y).x * R
    }

    fn standing() -> DVec3 {
        at(0.0, tune::STAND_M)
    }

    /// Measured on Jupiter's day side, the decks' own red over their tops' (scale height 20 km). A step down isn't allowed, nor light at the bottom.
    #[test]
    fn deep_in_the_decks_he_gets_what_they_do() {
        let scale = 20e3;
        for (h_km, want) in [(-10.0, 0.5), (-20.0, 0.35), (-40.0, 0.1), (-60.0, 0.1), (-80.0, 0.07), (-100.0, 0.03)] {
            let got = deck_light(h_km * 1e3, scale);
            assert!(got > want * 0.5 && got < want * 1.6, "{h_km} km: {got}, measured {want}");
        }
        assert_eq!(deck_light(100e3, scale), 1.0);
        assert_eq!(deck_light(-160e3, scale), 0.0);
        let mut last = 1.0;
        for i in 0..400 {
            let k = deck_light(100e3 - 1e3 * i as f64, scale);
            assert!(k <= last + 1e-12 && last - k < 0.1, "{i}: {k}");
            last = k;
        }
    }

    #[test]
    fn on_flat_ground_it_starts_at_his_soles() {
        let f = fall_over(over(|_| 0.0), standing(), sun(30.0)).unwrap();
        assert!(along(f.start).abs() < 0.01, "{}", along(f.start));
        assert!((along(f.end) - tune::SHADOW_TALL_M / 30f64.to_radians().tan()).abs() < 0.1, "{}", along(f.end));
        assert!(f.normal.dot(DVec3::Y) > 0.9999, "{}", f.normal);
        assert_eq!(f.strength, 1.0);
    }

    /// The ground rising from 2 m out on the side away from the sun: the plane is through his soles and where the shadow's end really
    /// is. (From a triangle at the ray's landing alone it passed 0.25 m under his feet, and the shadow began 1.5 m from his boots.)
    #[test]
    fn on_uneven_ground_it_runs_from_his_boots_to_where_it_ends() {
        let tan = 10f64.to_radians().tan();
        let f = fall_over(over(move |x| (x - 2.0).max(0.0) * tan), standing(), sun(15.0)).unwrap();
        assert!(along(f.start).abs() < 0.01 && (f.start.length() - R).abs() < 0.01, "{}", f.start);
        // 1.9 - 0.268 x = 0.176 (x - 2)
        assert!((along(f.end) - 5.07).abs() < 0.1, "{}", along(f.end));
        assert!(f.normal.dot(f.end - f.start).abs() < 0.03, "the line is in the plane: {}", f.normal.dot(f.end - f.start));
        assert!(f.normal.dot(DVec3::Y) > 0.9, "{}", f.normal);
    }

    /// A slope steeper than the sun is high: one long step would land many times too far off.
    #[test]
    fn a_low_sun_on_a_slope_does_not_overshoot() {
        let tan = 10f64.to_radians().tan();
        let f = fall_over(over(move |x| x.max(0.0) * tan), standing(), sun(3.0)).unwrap();
        // 1.9 - 0.0524 x = 0.1763 x
        assert!((along(f.end) - 8.31).abs() < 0.2, "{}", along(f.end));
        assert!(along(f.start).abs() < 0.3, "{}", along(f.start));
    }

    #[test]
    fn a_star_under_the_horizon_leaves_his_feet_alone() {
        let f = fall_over(over(|_| 0.0), standing(), sun(-10.0)).unwrap();
        assert!(along(f.start).abs() < 0.01 && (f.end - f.start).length() < 0.01, "{f:?}");
        assert!(f.normal.dot(DVec3::Y) > 0.9999);
        // Just on the horizon, the same: nothing to jump when it crosses.
        let f = fall_over(over(|_| 0.0), standing(), sun(1.0)).unwrap();
        assert!((f.end - f.start).length() < 0.01, "{f:?}");
    }

    #[test]
    fn it_fades_as_he_climbs_and_needs_the_ground() {
        let f = fall_over(over(|_| 0.0), at(0.0, 100.0), sun(45.0)).unwrap();
        assert!(f.strength > 0.0 && f.strength < 1.0, "{f:?}");
        // Under him, down the ray: as far along as he's high.
        assert!((along(f.start) - (100.0 - tune::STAND_M)).abs() < 0.3, "{f:?}");
        assert!(fall_over(over(|_| 0.0), at(0.0, 300.0), sun(45.0)).is_none());
        assert!(fall_over(|p: DVec3| Ground { radial: p.length() - 1.0, level: None }, standing(), sun(45.0)).is_none());
    }
}
