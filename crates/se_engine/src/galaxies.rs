//! Galaxy renderer.
//!
//! * **Far field**: every catalog galaxy (about 11k) is one elliptical billboard at infinity
//!   (`shaders/galaxy_far.wesl`), sized by `max(min_px, angular radius)`, with an analytic
//!   radial profile (de Vaucouleurs / disc+bulge / irregular), apparent magnitude from the
//!   catalog absolute magnitude, and a surface brightness that does not depend on distance.
//! * **3D models**: galaxies that become large on screen (angular radius above
//!   [`GalaxySettings::build_px`]) or that contain the camera get a sprite model built by
//!   [`se_core::galaxy_model`] on the async compute pool. The model is drawn as additive emission
//!   billboards plus multiplicative dust billboards (`shaders/galaxy_model.wesl`) using the
//!   `sprites_gal.png` atlas. At most [`GalaxySettings::max_models`] models stay resident
//!   (LRU); the far sprite fades out as the model fades in.
//! * **Density**: [`GalaxyDensity`] keeps the Milky Way's emission voxel map (and an analytic
//!   fallback) so procedural star generation can sample the stellar density.
//!
//! All galaxy geometry lives in parsecs relative to the Sun and is made camera-relative with
//! `f64` on the CPU (`center_rel` uniform), so there is no precision loss at any distance. The
//! sprites are drawn "at infinity" (only directions matter), exactly like the catalog starfield.

use std::collections::HashSet;
use std::f64::consts::PI;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::window::PrimaryWindow;
use se_core::catalog::{GalaxyRecord, ObjectRef};
use se_core::galaxy_model::{
    DensityMap, GalaxyModelDef, ModelCatalog, ModelImages, ModelSelection, RgbaImage, SpriteLayer,
    build_galaxy_model, model_textures, parse_pst,
};
use se_core::star_octree::ArmMap;
use se_core::vfs::Vfs;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::universe::UniverseSet;

/// Focal length (px) at which galaxy surface brightness is defined (60 deg vertical FOV at 900 px).
pub(crate) const FOCAL_REF: f64 = 780.0;
/// Maximum number of simultaneous 3D models (including the pinned Milky Way).
const MAX_SLOTS: usize = 8;
/// Handedness mapping from SE model space to the port frame (see `se_to_port`).
const DEFAULT_MAP_MODE: u32 = 0;

// -------------------------------------------------------------------------------------
// Vertex layouts and materials
// ------------------------------------------------------------------------------------

pub(crate) const ATTR_FAR_DATA: MeshVertexAttribute = MeshVertexAttribute::new("GalFarData", 0x6A1A_0001, VertexFormat::Float32x4);
pub(crate) const ATTR_FAR_SHAPE: MeshVertexAttribute = MeshVertexAttribute::new("GalFarShape", 0x6A1A_0002, VertexFormat::Float32x4);
pub(crate) const ATTR_FAR_CORNER: MeshVertexAttribute = MeshVertexAttribute::new("GalFarCorner", 0x6A1A_0003, VertexFormat::Float32x4);
pub(crate) const ATTR_MODEL_DATA: MeshVertexAttribute = MeshVertexAttribute::new("GalModelData", 0x6A1A_0004, VertexFormat::Float32x4);
pub(crate) const ATTR_MODEL_INFO: MeshVertexAttribute = MeshVertexAttribute::new("GalModelInfo", 0x6A1A_0005, VertexFormat::Float32x4);
pub(crate) const ATTR_MODEL_CORNER: MeshVertexAttribute = MeshVertexAttribute::new("GalModelCorner", 0x6A1A_0006, VertexFormat::Float32x2);

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct FarParams {
    /// xyz: the camera (pc); w: how far a new block of procedural galaxies is still faded out
    /// (SE's `StarFadeDuration`), 0 for the others.
    pub cam_pos: Vec4,
    pub params: Vec4,
    pub params2: Vec4,
    pub norm: Vec4,
    pub resident: [Vec4; 8],
    /// Motion blur, as in `StarParams` (filled in by `stars::update_sprite_blur`).
    pub blur: Vec4,
    pub blur_delta: Vec4,
    pub prev_view: Mat4,
}

/// Far-field sprite material (all catalog galaxies).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GalaxyFarMaterial {
    #[uniform(0)]
    pub uniforms: FarParams,
}

impl Material for GalaxyFarMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/galaxy_far.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/galaxy_far.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // See `GalaxyModelMaterial::depth_bias`.
        -4.0e15
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTR_FAR_DATA.at_shader_location(1),
            ATTR_FAR_SHAPE.at_shader_location(2),
            ATTR_FAR_CORNER.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        debug_depth(descriptor);
        Ok(())
    }
}

fn debug_depth(descriptor: &mut RenderPipelineDescriptor) {
    if std::env::var_os("SE_GAL_NODEPTH").is_some() {
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = Some(bevy::render::render_resource::CompareFunction::Always);
        }
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct ModelParams {
    pub center_rel: Vec4,
    pub params: Vec4,
    pub params2: Vec4,
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct PlaneParams {
    pub center_rel: Vec4,
    pub axis_x: Vec4,
    pub axis_y: Vec4,
    pub params: Vec4,
}

/// Material for a sprite-model galaxy's front texture (`galaxy_plane.wesl`).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GalaxyPlaneMaterial {
    #[uniform(0)]
    pub uniforms: PlaneParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

impl Material for GalaxyPlaneMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/galaxy_plane.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/galaxy_plane.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    fn depth_bias(&self) -> f32 {
        // After the model's sprites and dust, as SE draws it (see `GalaxyModelMaterial`).
        -1.5e15
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        debug_depth(descriptor);
        Ok(())
    }
}

/// Material for the sprites of one 3D galaxy or nebula model (emission, dust and bulge in one mesh).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GalaxyModelMaterial {
    #[uniform(0)]
    pub uniforms: ModelParams,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
}

impl Material for GalaxyModelMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/galaxy_model.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/galaxy_model.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // SE draws all layers in one call with glBlendFunc(ONE, ONE_MINUS_SRC_ALPHA) (FUN_14029bdf0,
        // FUN_140334e90): Bevy's Premultiplied. Emission and bulge return alpha 0 (additive); dust returns
        // (-colour, absBright), which darkens what was drawn before it and subtracts its colour.
        AlphaMode::Premultiplied
    }
    fn depth_bias(&self) -> f32 {
        // Transparent items sort by the view-space depth of their mesh AABB centre plus this bias; our
        // meshes are padded to a symmetric AABB (centre 0), so the bias is the sort key. Galaxies draw
        // before everything else transparent but the stars: an atmosphere's sky shell, drawn over
        // them, lets them through by its transmittance and hides them in a bright sky (`sky.wesl`).
        // Depth testing against the opaque bodies still hides the galaxy behind them. Order: far
        // sprites, model sprites (sorted far to near inside the mesh, see `SpriteOrder`), front texture.
        -3.0e15
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTR_MODEL_DATA.at_shader_location(1),
            ATTR_MODEL_INFO.at_shader_location(2),
            ATTR_MODEL_CORNER.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        debug_depth(descriptor);
        Ok(())
    }
}

// -------------------------------------------------------------------------------------
// Settings, profiles
// ------------------------------------------------------------------------------------

/// User-tweakable galaxy rendering settings.
#[derive(Resource, Debug, Clone)]
pub struct GalaxySettings {
    pub enabled: bool,
    /// Global brightness of far sprites; 3D models are calibrated to the same surface brightness.
    pub far_brightness: f32,
    /// Extra multiplier on SE's model emission ([`se_glow_bright`]); 1 = SE.
    pub model_brightness: f32,
    /// SE's model brightness chain (main-def.cfg): `GlowMagnLimit` (follows the `]` / `[` keys),
    /// `GlowBrightMin`, `hdrGalaxySpriteBright` and `GalaxyFrontTexBright`.
    pub glow_mag_limit: f32,
    pub glow_bright_min: f32,
    pub sprite_bright: f32,
    pub front_tex_bright: f32,
    /// `GalaxyBrightMin` (cut-off of SE's per-galaxy brightness term, [`se_bright_term`]).
    pub bright_min: f32,
    /// Multiplier on SE's dust strength (absBright = the model's `Transparency`); 1 = SE.
    pub dust_strength: f32,
    /// Minimum far-sprite radius in pixels.
    pub min_px: f32,
    /// A model is built when the galaxy's angular radius exceeds this many pixels.
    pub build_px: f32,
    /// Maximum number of resident 3D models.
    pub max_models: usize,
    /// Reference magnitude: a galaxy of this apparent magnitude has flux 1.
    pub mag_ref: f32,
    /// Far sprites with a peak intensity below this are skipped.
    pub cull: f32,
    /// "Intergalactic vision" (SE `AutoGalaxyMagnitude`): raise the brightness of all galaxies as the
    /// nearest bright galaxy gets farther than `auto_d0_pc`, by `(d / d0)^2`, up to `auto_max`.
    pub auto_gain: bool,
    pub auto_d0_pc: f32,
    pub auto_max: f32,
}

impl Default for GalaxySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            far_brightness: 1.5,
            model_brightness: 1.0,
            glow_mag_limit: 7.0,
            glow_bright_min: 0.006,
            sprite_bright: 0.03,
            front_tex_bright: 1.0,
            bright_min: 0.03,
            dust_strength: 1.0,
            min_px: 2.5,
            build_px: 4.0,
            max_models: 6,
            mag_ref: 7.5,
            cull: 0.003,
            auto_gain: true,
            auto_d0_pc: 3.0e6,
            auto_max: 5000.0,
        }
    }
}

impl GalaxySettings {
    /// Defaults, overridable for quick tuning with `SE_GAL_FAR`, `SE_GAL_MODEL`, `SE_GAL_DUST`,
    /// `SE_GAL_BUILD` environment variables.
    pub fn from_env() -> Self {
        let mut s = Self::default();
        let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
        if let Some(v) = get("SE_GAL_FAR") {
            s.far_brightness = v;
        }
        if let Some(v) = get("SE_GAL_MODEL") {
            s.model_brightness = v;
        }
        if let Some(v) = get("SE_GAL_DUST") {
            s.dust_strength = v;
        }
        if let Some(v) = get("SE_GAL_BUILD") {
            s.build_px = v;
        }
        if let Some(v) = get("SE_GAL_AUTO") {
            s.auto_gain = v > 0.0;
        }
        s
    }
}

/// Radial profile of the far sprite. Type 0 elliptical, 1 disc + bulge, 2 irregular, 3 point.
/// Must match `profile` in `galaxy_far.wesl`.
pub fn galaxy_profile(ptype: u8, x: f32) -> f32 {
    let w = (1.0 - x * x).max(0.0);
    let win = w * w;
    match ptype {
        0 => {
            let xs = (x * x + 1.0e-4).sqrt();
            (-7.67 * ((xs / 0.25).powf(0.25) - 1.0)).exp() * win
        }
        1 => ((-3.2 * x).exp() + 2.5 * (-60.0 * x * x).exp()) * win,
        2 => (-2.5 * x * x).exp() * win,
        _ => (-18.0 * x * x).exp() * win,
    }
}

/// Integral of the profile over the unit disc, `2 pi * int x p(x) dx`.
pub fn profile_norm(ptype: u8) -> f32 {
    let n = 4000;
    let mut s = 0.0f64;
    for i in 0..n {
        let x = (i as f32 + 0.5) / n as f32;
        s += (galaxy_profile(ptype, x) * x) as f64;
    }
    (2.0 * PI * s / n as f64) as f32
}

/// Faintest apparent magnitude at which an unresolved galaxy's far sprite is still drawn, per profile type:
/// `galaxy_far.wesl` hides a sprite whose intensity (at the `min_px` radius, with the full
/// intergalactic-vision `gain`) is below `cull`. `focal` in physical pixels.
pub(crate) fn far_mag_limits(s: &GalaxySettings, gain: f32, focal: f64) -> [f64; 4] {
    let sb = (focal / FOCAL_REF).powi(2);
    std::array::from_fn(|t| {
        let min_flux = s.cull as f64 * profile_norm(t as u8) as f64 * (s.min_px as f64).powi(2)
            / (s.far_brightness as f64 * sb * gain.max(1.0) as f64);
        s.mag_ref as f64 - 2.5 * min_flux.log10()
    })
}

/// Quasars are profiled by their `Type` like any galaxy: SE reads a `Quasar` block with the galaxy
/// reader (FUN_14016b430 -> FUN_14016bef0) and only sets flag 0x800, which its shaders do not use.
pub(crate) fn profile_type(rec: &GalaxyRecord) -> u8 {
    let k = rec.kind.as_str();
    if k.starts_with('S') {
        1
    } else if k.starts_with('I') {
        2
    } else {
        // E, dE, cD, unknown
        0
    }
}

/// Visual colour (linear RGB, max channel about 1) from the Hubble type (quasars too, see `profile_type`).
pub(crate) fn type_color(kind: &str) -> [f32; 3] {
    match kind {
        k if k.starts_with("SB") || k.starts_with('S') => {
            let late = k.trim_start_matches("SB").trim_start_matches('S');
            match late.chars().next() {
                Some('0') | Some('a') => [1.0, 0.88, 0.7],
                Some('b') => [1.0, 0.92, 0.8],
                Some('c') | Some('d') => [0.85, 0.9, 1.0],
                _ => [0.95, 0.92, 0.88],
            }
        }
        k if k.starts_with('I') => [0.75, 0.85, 1.0],
        _ => [1.0, 0.86, 0.66],
    }
}

// -------------------------------------------------------------------------------------
// Model building (runs on the async compute pool)
// ------------------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct SpriteBuffers {
    pub positions: Vec<[f32; 3]>,
    pub data: Vec<[f32; 4]>,
    pub info: Vec<[f32; 4]>,
    pub corners: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// Adds two unreferenced vertices so the mesh AABB is centred on the origin (see `depth_bias`).
pub(crate) fn pad_aabb(positions: &mut Vec<[f32; 3]>) -> f32 {
    let m = positions.iter().flat_map(|p| p.iter()).fold(1.0f32, |a, &c| a.max(c.abs()));
    positions.push([m, m, m]);
    positions.push([-m, -m, -m]);
    m
}

impl SpriteBuffers {
    pub(crate) fn push(&mut self, pos: [f32; 3], data: [f32; 4], info: [f32; 4]) {
        let base = self.positions.len() as u32;
        for c in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            self.positions.push(pos);
            self.data.push(data);
            self.info.push(info);
            self.corners.push(c);
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Each sprite's centre (one per quad).
    pub(crate) fn centres(&self) -> Vec<[f32; 3]> {
        self.positions.iter().step_by(4).copied().collect()
    }

    /// The mesh keeps its main-world copy: `SpriteOrder` swaps its index buffer.
    pub(crate) fn into_mesh(mut self) -> Mesh {
        pad_aabb(&mut self.positions);
        self.data.extend([[0.0; 4]; 2]);
        self.info.extend([[0.0; 4]; 2]);
        self.corners.extend([[0.0; 2]; 2]);
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(ATTR_MODEL_DATA, self.data)
            .with_inserted_attribute(ATTR_MODEL_INFO, self.info)
            .with_inserted_attribute(ATTR_MODEL_CORNER, self.corners)
            .with_inserted_indices(Indices::U32(self.indices))
    }
}

struct BuiltModel {
    /// Emission, dust and bulge sprites in generation order (SE's VBO order before sorting).
    buf: SpriteBuffers,
    map: Option<DensityMap>,
    sprites: usize,
    plane: Option<FrontPlane>,
}

/// A model's front texture (with mipmaps).
struct FrontPlane {
    image: Image,
}

struct BuildInput {
    vfs: Arc<Vfs>,
    textures: Arc<Vec<String>>,
    def: GalaxyModelDef,
    orientation: DQuat,
    radius_pc: f64,
    map_mode: u32,
    bulge_gain: f32,
}

/// SE model space -> port model space.
///
/// Mode 0 (default) is the identity: the SE model frame already is the port frame. The catalog
/// `orientation` maps model space (disk normal = model -Z, pointing at the observer side) to the
/// universe, and the sprite model's image convention (column -> +x, row down -> +y) is the unmirrored
/// view from the -Z side, so no flip is needed. The other modes exist for calibrating the handedness
/// against the textures (`SE_GAL_MAP`): 1 = flip z, 2 = flip x, 3 = flip y, 4 = invert, 5 = rotate 180
/// degrees about X (the old `se_to_port_model`).
fn se_to_port(mode: u32, p: DVec3) -> DVec3 {
    match mode {
        1 => DVec3::new(p.x, p.y, -p.z),
        2 => DVec3::new(-p.x, p.y, p.z),
        3 => DVec3::new(p.x, -p.y, p.z),
        4 => DVec3::new(-p.x, -p.y, -p.z),
        5 => DVec3::new(p.x, -p.y, -p.z),
        _ => p,
    }
}

fn load_image(vfs: &Vfs, name: &Option<String>) -> Option<RgbaImage> {
    let name = name.as_ref()?;
    let bytes = vfs.read(&format!("textures/galaxies/{name}")).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    RgbaImage::new(w, h, img.into_raw())
}

fn build_model(inp: BuildInput) -> Option<BuiltModel> {
    let BuildInput { vfs, textures, def, orientation, radius_pc, map_mode, bulge_gain } = inp;
    let tex = model_textures(&def, &textures);
    let front = load_image(&vfs, &tex.front_image);
    let side = load_image(&vfs, &tex.side_image);
    let cube: Vec<Option<RgbaImage>> = tex.cube.iter().map(|n| load_image(&vfs, n)).collect();
    let images = ModelImages {
        front: front.as_ref(),
        side: side.as_ref(),
        cube: std::array::from_fn(|i| cube[i].as_ref()),
    };
    let data = match build_galaxy_model(&def, &images) {
        Ok(d) => d,
        Err(e) => {
            warn!("galaxy model {:?} failed: {e}", def.name);
            return None;
        }
    };
    // SE model space -> pc offsets from the galaxy centre in the universe frame:
    // scale by Radius * SpriteOblZ, convert to the port frame (disk normal -Z), rotate.
    let ms = def.model_scale();
    let scale = DVec3::new(ms.x as f64, ms.y as f64, ms.z as f64) * radius_pc;
    let mut buf = SpriteBuffers::default();
    for s in &data.sprites {
        let local = DVec3::new(s.pos[0] as f64, s.pos[1] as f64, s.pos[2] as f64) * scale;
        let off = orientation * se_to_port(map_mode, local);
        let size_pc = s.size as f64 * scale.x;
        let layer = s.layer();
        let info = [size_pc as f32, s.atlas_tile() as f32, s.lod_rand as f32 / 255.0, layer as u8 as f32];
        let pos = [off.x as f32, off.y as f32, off.z as f32];
        let mut d = sprite_vertex(s.color, s.brightness);
        // `SE_GAL_BULGE` scales the bulge for comparisons (1 = SE).
        if layer == SpriteLayer::Bulge {
            d[3] *= bulge_gain;
        }
        buf.push(pos, d, info);
    }
    let plane = tex.front_texture.as_deref().and_then(|name| front_plane(&vfs, name));
    Some(BuiltModel {
        sprites: data.sprites.len(),
        buf,
        map: data.emission_map,
        plane,
    })
}

/// Vertex colour and intensity of a model sprite as SE uploads them (FUN_1402da540 / FUN_1402d9a10:
/// `clamp(colour) * 255` and `min(1, intensity) * 255` bytes; the shader reads them back as 0..1).
pub(crate) fn sprite_vertex(color: [f32; 3], intensity: f32) -> [f32; 4] {
    let c = color.map(|c| c.clamp(0.0, 1.0));
    [c[0], c[1], c[2], intensity.clamp(0.0, 1.0)]
}

/// Back-to-front draw order of a sprite model: triangle indices of the quads sorted by the squared
/// distance of their centres from `eye`, farthest first (SE: FUN_140333a30 with the descending quicksort
/// FUN_140334190; one draw for emission, dust and bulge, FUN_140334e90).
pub(crate) fn back_to_front(centres: &[[f32; 3]], eye: [f32; 3]) -> Vec<u32> {
    let mut order: Vec<(f32, u32)> = centres
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2], i as u32)
        })
        .collect();
    order.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let mut idx = Vec::with_capacity(order.len() * 6);
    for (_, i) in order {
        let b = i * 4;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    idx
}

/// SE sorts a galaxy model when it is larger than this (radius, px; the flag set around 0x14029bfe9).
pub(crate) const SORT_PX: f64 = 10.0;

/// The draw order of one resident sprite model. SE rebuilds it per view; the port re-sorts on the async
/// pool when the camera has moved by more than 2% of its distance to the model centre, and swaps the
/// index buffer in when the job is done (one upload per re-sort, none per frame).
pub(crate) struct SpriteOrder {
    centres: Arc<Vec<[f32; 3]>>,
    mesh: Handle<Mesh>,
    sorted_eye: Option<Vec3>,
    task: Option<(Vec3, Task<Vec<u32>>)>,
}

impl SpriteOrder {
    pub(crate) fn new(centres: Vec<[f32; 3]>, mesh: Handle<Mesh>) -> Self {
        Self { centres: Arc::new(centres), mesh, sorted_eye: None, task: None }
    }

    /// `eye`: the camera relative to the model centre (pc, universe frame); `sort`: the model is drawn
    /// large enough for SE to sort it.
    pub(crate) fn update(&mut self, eye: Vec3, sort: bool, meshes: &mut Assets<Mesh>) {
        if let Some((at, task)) = self.task.as_mut() {
            if let Some(idx) = check_ready(task) {
                if let Some(mut m) = meshes.get_mut(&self.mesh) {
                    m.insert_indices(Indices::U32(idx));
                }
                self.sorted_eye = Some(*at);
                self.task = None;
            }
            return;
        }
        let moved = self.sorted_eye.map_or(f32::INFINITY, |p| p.distance(eye));
        if !sort || moved <= 0.02 * eye.length() {
            return;
        }
        let (centres, e) = (self.centres.clone(), eye.to_array());
        self.task = Some((eye, AsyncComputeTaskPool::get().spawn(async move { back_to_front(&centres, e) })));
    }
}

/// SE's magnitude-limit coefficient (FUN_1404c4610): a limit `m` is `exp((m + 0.2) / 1.0587)`
/// (DAT_1406d4204 = 0.2, DAT_1406d46a8 = 1.0587362).
pub(crate) fn se_magn_coef(limit: f32) -> f32 {
    ((limit + 0.2) / 1.058_736_2).exp()
}

/// SE's sprite-model brightness without the fades (emBright of FUN_14029bdf0, PhotoMode 0 = HDR):
/// `DAT_14080b5a4 * [renderer+0x35ec]` = `GlowBrightMin * exp((GlowMagnLimit + 0.2) / 1.0587)`
/// (FUN_1404c4610) times `hdrGalaxySpriteBright` (or `hdrNebulaSpriteBright`, FUN_1404c4ae0 +0x4c / +0x50).
/// The draw multiplies it by the model's `Brightness` and the galaxy's +0x74; the front texture
/// (galaxy_plane.glsl EyePos.w) by `GalaxyFrontTexBright` instead of `Brightness`. 0.162 at SE's defaults.
pub(crate) fn se_glow_bright(glow_limit: f32, glow_bright_min: f32, sprite_bright: f32) -> f32 {
    glow_bright_min * se_magn_coef(glow_limit) * sprite_bright
}

/// SE's per-object brightness term of a sprite model (FUN_140299fe0 galaxies, FUN_1402b3020 nebulae):
/// `clamp(scale * coef(limit) * L / d^2 - bright_min, 0, 1)`, with the object's flux `L / d^2` taken as
/// `exp(-(m + 0.2) / 1.0587)` on SE's magnitude scale (its units were not traced). Galaxies use scale 1
/// (DAT_14080b58c) and `GalaxyBrightMin`, nebulae `NebulaBrightMin` for both. 0 hides the model.
pub(crate) fn se_bright_term(limit: f32, m_app: f32, scale: f32, bright_min: f32) -> f32 {
    (scale * ((limit - m_app) / 1.058_736_2).exp() - bright_min).clamp(0.0, 1.0)
}

/// SE's `DSOGamma` (main-def.cfg), applied to the front texture (galaxy_plane.glsl).
const DSO_GAMMA: f32 = 0.7;
/// Loads a front texture with its mip chain.
fn front_plane(vfs: &Vfs, name: &str) -> Option<FrontPlane> {
    let bytes = vfs.read(&format!("textures/galaxies/{name}")).ok()?;
    let mut level = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = level.dimensions();
    let mut data = Vec::new();
    let mut mips = 0;
    loop {
        data.extend_from_slice(level.as_raw());
        mips += 1;
        if level.width() == 1 && level.height() == 1 {
            break;
        }
        let (nw, nh) = ((level.width() / 2).max(1), (level.height() / 2).max(1));
        level = image::imageops::resize(&level, nw, nh, image::imageops::FilterType::Triangle);
    }
    let mut image = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = mips;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    Some(FrontPlane { image })
}

/// A unit quad (corners at +-1) with the texture's columns along +x and its rows down +y.
fn plane_mesh() -> Mesh {
    let pos = vec![[-1.0f32, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]];
    let uv = vec![[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

// -------------------------------------------------------------------------------------
// Density API
// ------------------------------------------------------------------------------------

/// Stellar density of the Milky Way for the procedural star generator.
///
/// * [`relative_emission`](Self::relative_emission) samples the generator's emission voxel map
///   (the "Milky Way" `.pst` model): units are the map's linear brightness `(r+g+b)/3`, dimensionless,
///   `0` outside the disc, about `0.1..1` inside. The map covers `[-1,1]^2` model units
///   (1 = disc radius `radius_pc * Radius.x`) with 2/nx voxels and a thin z range.
/// * [`stars_per_pc3`](Self::stars_per_pc3) converts it to stars per cubic parsec by scaling so that the
///   Sun's neighbourhood has [`SOLAR_DENSITY`] stars/pc^3. Until the map is built (or outside it) it falls
///   back to [`analytic_stars_per_pc3`](Self::analytic_stars_per_pc3), an exponential disc plus bulge.
///
/// Positions are parsecs in the universe frame relative to the Sun.
#[derive(Resource, Clone)]
pub struct GalaxyDensity {
    pub centre_pc: DVec3,
    pub radius_pc: f64,
    /// Model (port frame, disk normal -Z) to universe.
    pub orientation: DQuat,
    /// Parsecs per SE model unit along x, y, z (`radius * Radius.x`, ..., `* SpriteOblZ`).
    pub scale_pc: DVec3,
    map: Option<Arc<DensityMap>>,
    sun_value: f32,
}

/// Local stellar number density used to normalise the Milky Way profile (stars per pc^3).
#[allow(dead_code)] // public API for the procedural star generator
pub const SOLAR_DENSITY: f32 = 0.14;

impl Default for GalaxyDensity {
    fn default() -> Self {
        Self {
            centre_pc: DVec3::ZERO,
            radius_pc: 1.0,
            orientation: DQuat::IDENTITY,
            scale_pc: DVec3::ONE,
            map: None,
            sun_value: 0.0,
        }
    }
}

#[allow(dead_code)] // public API for the procedural star generator
impl GalaxyDensity {
    /// True when the voxel map from the Milky Way model is available.
    pub fn has_map(&self) -> bool {
        self.map.is_some()
    }

    /// Universe-frame position (pc from the Sun) -> SE model units (disc radius = 1, z up the axis).
    pub fn model_from_universe(&self, pos_pc: DVec3) -> DVec3 {
        let port = self.orientation.inverse() * (pos_pc - self.centre_pc);
        se_to_port(0, port) / self.scale_pc
    }

    /// Emission map sampled at `model_pos` (SE model units, see [`model_from_universe`](Self::model_from_universe)).
    pub fn relative_emission(&self, model_pos: Vec3) -> f32 {
        let Some(m) = &self.map else { return 0.0 };
        m.sample(model_pos.x, model_pos.y, model_pos.z)
    }

    /// Emission map relative to its value at the Sun (1 in the solar neighbourhood); 0 without a map.
    pub fn relative_to_sun(&self, model_pos: Vec3) -> f32 {
        if self.sun_value <= 0.0 {
            return 0.0;
        }
        self.relative_emission(model_pos) / self.sun_value
    }

    /// Exponential disc (scale length 2.6 kpc, thin 300 pc + thick 900 pc) plus bulge, normalised to
    /// [`SOLAR_DENSITY`] at the Sun. Stars per pc^3.
    pub fn analytic_stars_per_pc3(&self, pos_pc: DVec3) -> f32 {
        let rel = pos_pc - self.centre_pc;
        let n = self.orientation * DVec3::NEG_Z;
        let z = rel.dot(n).abs();
        let rr = (rel - n * rel.dot(n)).length();
        let r0 = self.centre_pc.length();
        let disc = (-(rr - r0) / 2600.0).exp() * (0.92 * (-z / 300.0).exp() + 0.08 * (-z / 900.0).exp());
        let bulge = 30.0 * (-(rel.length() / 450.0)).exp();
        SOLAR_DENSITY * disc as f32 + bulge as f32
    }

    /// Stellar number density (stars/pc^3) at a universe-frame position in parsecs from the Sun.
    pub fn stars_per_pc3(&self, pos_pc: DVec3) -> f32 {
        let analytic = self.analytic_stars_per_pc3(pos_pc);
        if self.map.is_none() || self.sun_value <= 0.0 {
            return analytic;
        }
        let m = self.model_from_universe(pos_pc);
        let v = self.relative_emission(m.as_vec3());
        if v <= 0.0 {
            // Outside the thin voxel layer the disc falls off exponentially; use the analytic tail.
            return analytic * 0.02;
        }
        SOLAR_DENSITY * v / self.sun_value
    }
}

// -------------------------------------------------------------------------------------
// State
// ------------------------------------------------------------------------------------

enum SlotState {
    Loading(Task<Option<BuiltModel>>),
    Ready(ReadyModel),
    Failed,
}

struct ReadyModel {
    entity: Entity,
    mat: Handle<GalaxyModelMaterial>,
    order: SpriteOrder,
    /// The front texture's disc: entity, material.
    plane: Option<(Entity, Handle<GalaxyPlaneMaterial>)>,
    sprites: usize,
    /// Emission voxel map (the density of the stars generated inside the galaxy).
    map: Option<Arc<DensityMap>>,
}

/// Marks the entities of 3D galaxy models.
#[derive(Component)]
struct GalaxyEntity;

struct Slot {
    galaxy: usize,
    state: SlotState,
    last_wanted: f32,
    pinned: bool,
}

/// The sprite atlas shared by galaxy and nebula models.
#[derive(Resource, Clone)]
pub(crate) struct SpriteAtlas {
    pub handle: Handle<Image>,
    pub tile_mean: [f32; 16],
}


#[derive(Resource)]
pub(crate) struct GalaxyState {
    models: Arc<ModelCatalog>,
    textures: Arc<Vec<String>>,
    /// Per catalog galaxy: index of its sprite model in `models`.
    model_of: Vec<Option<usize>>,
    tile_mean: [f32; 16],
    atlas: Handle<Image>,
    far: Handle<GalaxyFarMaterial>,
    slots: Vec<Slot>,
    failed: HashSet<usize>,
    milky_way: Option<usize>,
    /// Smoothed intergalactic-vision gain.
    pub(crate) gain: f32,
    /// Catalog galaxies whose 3D model is fading in this frame (`(catalog index, fade)`); the streamed
    /// procedural far sprites of `procgalaxies.rs` hide behind them.
    pub(crate) resident: Vec<(usize, f32)>,
    /// Gain asked for by `procgalaxies.rs` so that a few hundred galaxies stay visible in intergalactic space.
    pub(crate) adaptive_gain: f32,
}

impl GalaxyState {
    /// Spiral arms for the stars generated inside catalog galaxy `g`, from its sprite model's
    /// emission map: `None` while the model is still being built, `Some(None)` when there is none.
    pub(crate) fn interior_arms(&self, g: usize, radius_pc: f64) -> Option<Option<ArmMap>> {
        let Some(Some(m)) = self.model_of.get(g) else { return Some(None) };
        if self.failed.contains(&g) {
            return Some(None);
        }
        match self.slots.iter().find(|s| s.galaxy == g).map(|s| &s.state) {
            Some(SlotState::Ready(r)) => Some(r.map.clone().map(|map| {
                let ms = self.models.models()[*m].model_scale();
                ArmMap::new(map, DVec3::new(ms.x as f64, ms.y as f64, ms.z as f64) * radius_pc)
            })),
            Some(SlotState::Failed) => Some(None),
            _ => None,
        }
    }

    /// Number of sprite models per Hubble type index (the random pools of procedural galaxies).
    pub(crate) fn model_pools(&self) -> [u32; 17] {
        let mut p = [0u32; 17];
        for (t, n) in p.iter_mut().enumerate() {
            *n = self.models.pool_len(t as u8) as u32;
        }
        p
    }

    /// Model scale `(R, R, R * SpriteOblZ)` of the sprite model a catalog-style record would get.
    pub(crate) fn model_scale_of(&self, rec: &GalaxyRecord) -> Option<[f32; 3]> {
        select_model(&self.models, rec).map(|m| self.models.models()[m].model_scale().to_array())
    }

    /// The material of the catalog far sprites.
    pub(crate) fn far_handle(&self) -> Handle<GalaxyFarMaterial> {
        self.far.clone()
    }

    /// Whether catalog galaxy `g` has a sprite model (ellipticals have none: `ellipticals.rs` draws them).
    pub(crate) fn has_model(&self, g: usize) -> bool {
        self.model_of.get(g).is_some_and(Option::is_some)
    }
}

pub struct GalaxiesPlugin;

impl Plugin for GalaxiesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<GalaxyFarMaterial>::default(),
            MaterialPlugin::<GalaxyModelMaterial>::default(),
            MaterialPlugin::<GalaxyPlaneMaterial>::default(),
        ))
        .insert_resource(GalaxySettings::from_env())
        .init_resource::<GalaxyDensity>()
        .add_systems(Startup, setup_galaxies)
        .add_systems(PostUpdate, (sync_mag_limit, update_galaxies).chain().after(UniverseSet::FloatingOrigin));
    }
}

/// Loads a 4x4 sprite atlas (`sprites_gal.png`, `sprites_neb.png`) as R8 and measures each tile's mean.
pub(crate) fn load_atlas(vfs: &Vfs, path: &str) -> Option<(Image, [f32; 16])> {
    let bytes = vfs.read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_luma8();
    // Box filter down to 512^2 (tiles of 128 px) to avoid minification aliasing.
    let (w, h) = img.dimensions();
    let f = (w / 512).max(1);
    let (nw, nh) = (w / f, h / f);
    let mut out = vec![0u8; (nw * nh) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let mut s = 0u32;
            for dy in 0..f {
                for dx in 0..f {
                    s += img.get_pixel(x * f + dx, y * f + dy).0[0] as u32;
                }
            }
            out[(y * nw + x) as usize] = (s / (f * f)) as u8;
        }
    }
    let mut tile_mean = [0.0f32; 16];
    let (tw, th) = (nw / 4, nh / 4);
    for (t, mean) in tile_mean.iter_mut().enumerate() {
        let (cx, cy) = ((t as u32 % 4) * tw, (t as u32 / 4) * th);
        let mut s = 0.0f64;
        for y in 0..th {
            for x in 0..tw {
                s += out[((cy + y) * nw + cx + x) as usize] as f64 / 255.0;
            }
        }
        *mean = (s / (tw * th) as f64) as f32;
    }
    let mut image = Image::new(
        Extent3d { width: nw, height: nh, depth_or_array_layers: 1 },
        TextureDimension::D2,
        out,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    Some((image, tile_mean))
}

/// Intensity `inten` of a galaxy `px` pixels in radius under the intergalactic-vision `gain`:
/// unresolved galaxies get it in full (it raises the magnitude limit, as SpaceEngine's
/// `AutoGalaxyMagnitude`), galaxies big on screen keep their own surface brightness, and
/// nothing is raised beyond `cap`, so nearby galaxies keep their colour instead of turning
/// into white blobs. Must match `vision` in galaxy_far.wesl.
pub(crate) fn vision(inten: f32, gain: f32, px: f32, cap: f32) -> f32 {
    let t = ((px - 3.0) / 37.0).clamp(0.0, 1.0);
    let resolved = t * t * (3.0 - 2.0 * t);
    let boosted = inten * gain.max(1.0).powf(1.0 - resolved);
    inten.max(boosted.min(cap))
}

/// Minimum radius (px) of a 3D model's sprites. Smaller than the far sprites' `min_px`:
/// widening them spreads a distant model's light until it disappears into the background.
const MODEL_MIN_PX: f32 = 1.5;

/// Peak sprite intensity the intergalactic vision can raise a far galaxy to.
pub(crate) const VISION_CAP: f32 = 1.0;
/// Mean surface brightness it can raise a 3D galaxy model to.
pub(crate) const VISION_CAP_MODEL: f32 = 0.05;

/// The port's far-sprite reference magnitude at SE's limiting magnitude: `mag_ref = limit + 0.5`
/// (7.5 at the default `GalaxyMagnLimit` / `NebulaMagnLimit` 7).
pub(crate) const MAG_REF_OFFSET: f32 = 0.5;

/// SE's `main-def.cfg`, parsed (deep-sky display keys are read from it at startup).
pub(crate) fn main_def_doc(cfg: &crate::config::EngineConfig) -> Option<se_core::sc::Block> {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).ok()?;
    Some(se_core::sc::parse(&text))
}

/// `GalaxyMagnLimit` (the `]` / `[` keys, `stars::mag_limit_keys`) -> the reference magnitude of the
/// galaxy far sprites and models (procedural galaxies and ellipticals read the same setting).
fn sync_mag_limit(stars: Res<crate::stars::StarSettings>, mut gs: ResMut<GalaxySettings>) {
    let m = stars.galaxy_mag_limit + MAG_REF_OFFSET;
    if (gs.mag_ref - m).abs() > 1e-6 || gs.glow_mag_limit != stars.glow_mag_limit {
        gs.mag_ref = m;
        gs.glow_mag_limit = stars.glow_mag_limit;
    }
}

pub(crate) fn far_uniforms(cam_pc: Vec3, s: &GalaxySettings) -> FarParams {
    FarParams {
        cam_pos: cam_pc.extend(0.0),
        params: Vec4::new(s.far_brightness, s.min_px, FOCAL_REF as f32, s.mag_ref),
        params2: Vec4::new(s.cull, 1.0, 1.0, 0.0),
        norm: Vec4::new(profile_norm(0), profile_norm(1), profile_norm(2), profile_norm(3)),
        resident: [Vec4::new(-1.0, 0.0, 0.0, 0.0); 8],
        blur: Vec4::ZERO,
        blur_delta: Vec4::ZERO,
        prev_view: Mat4::IDENTITY,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn setup_galaxies(
    mut commands: Commands,
    data: Res<SeData>,
    mut settings: ResMut<GalaxySettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut density: ResMut<GalaxyDensity>,
    (cfg, mut stars): (Res<crate::config::EngineConfig>, ResMut<crate::stars::StarSettings>),
) {
    // SE's deep-sky display keys (main-def.cfg): limits, intergalactic vision, catalog filter.
    if let Some(doc) = main_def_doc(&cfg) {
        let num = |k: &str| doc.find_deep(k).and_then(|i| i.f64()).map(|v| v as f32);
        let flag = |k: &str| doc.find_deep(k).and_then(|i| i.bool());
        stars.galaxy_mag_limit = num("GalaxyMagnLimit").unwrap_or(stars.galaxy_mag_limit);
        stars.nebula_mag_limit = num("NebulaMagnLimit").unwrap_or(stars.nebula_mag_limit);
        stars.glow_mag_limit = num("GlowMagnLimit").unwrap_or(stars.glow_mag_limit);
        stars.dso_mag_limit_reset = [stars.galaxy_mag_limit, stars.nebula_mag_limit, stars.glow_mag_limit];
        settings.mag_ref = stars.galaxy_mag_limit + MAG_REF_OFFSET;
        settings.glow_mag_limit = stars.glow_mag_limit;
        settings.glow_bright_min = num("GlowBrightMin").unwrap_or(settings.glow_bright_min);
        settings.sprite_bright = num("hdrGalaxySpriteBright").unwrap_or(settings.sprite_bright);
        settings.front_tex_bright = num("GalaxyFrontTexBright").unwrap_or(settings.front_tex_bright);
        settings.bright_min = num("GalaxyBrightMin").unwrap_or(settings.bright_min);
        // SE_GAL_AUTO (testing) wins over AutoGalaxyMagnitude.
        if std::env::var_os("SE_GAL_AUTO").is_none() {
            settings.auto_gain = flag("AutoGalaxyMagnitude").unwrap_or(settings.auto_gain);
        }
        settings.enabled &= flag("ShowCatGalaxies").unwrap_or(true);
    }
    let vfs = data.vfs.clone();
    let cat = data.catalog.clone();

    // --- models (.pst) and texture list -------------------------------------------------
    let mut defs = Vec::new();
    for p in vfs.list_ext("models/galaxies", "pst") {
        match vfs.read_string(&p).map(|t| parse_pst(&t)) {
            Ok(Ok(m)) => defs.extend(m),
            Ok(Err(e)) => warn!("{p}: {e}"),
            Err(e) => warn!("{p}: {e}"),
        }
    }
    let prefix = "textures/galaxies/";
    let textures: Vec<String> = vfs
        .list("textures/galaxies")
        .into_iter()
        .filter_map(|p| p.strip_prefix(prefix).map(str::to_string))
        .filter(|n| !n.contains('/'))
        .collect();
    let models = ModelCatalog::new(defs);

    // --- which catalog galaxies get a sprite model --------------------------------------
    let mut model_of = Vec::with_capacity(cat.galaxies.len());
    let mut with_model = 0;
    for rec in &cat.galaxies {
        let sel = select_model(&models, rec);
        if sel.is_some() {
            with_model += 1;
        }
        model_of.push(sel);
    }
    info!(
        "galaxies: {} models, {} textures, {} of {} catalog galaxies use a sprite model",
        models.models().len(),
        textures.len(),
        with_model,
        cat.galaxies.len()
    );

    // --- atlas ------------------------------------------------------------------------------
    let (atlas_img, tile_mean) = load_atlas(&vfs, "textures/common/sprites_gal.png").unwrap_or_else(|| {
        warn!("sprites_gal.png not found; galaxy models will be invisible");
        let img = Image::new(
            Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            TextureDimension::D2,
            vec![255; 16],
            TextureFormat::R8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        (img, [0.5; 16])
    });
    let atlas = images.add(atlas_img);

    // --- far-field mesh ---------------------------------------------------------------------
    let n = cat.galaxies.len();
    let mut positions = Vec::with_capacity(n * 4);
    let mut fdata = Vec::with_capacity(n * 4);
    let mut shape = Vec::with_capacity(n * 4);
    let mut corner = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);
    for (i, rec) in cat.galaxies.iter().enumerate() {
        if !rec.vis_mag.is_finite() || !rec.radius_pc.is_finite() || rec.radius_pc <= 0.0 {
            continue;
        }
        let ptype = profile_type(rec);
        let normal = if ptype == 1 {
            (rec.orientation * DVec3::NEG_Z).as_vec3()
        } else {
            Vec3::ZERO
        };
        let col = type_color(&rec.kind);
        let p = rec.pos_pc.as_vec3();
        let base = positions.len() as u32;
        for c in [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            positions.push([p.x, p.y, p.z]);
            fdata.push([col[0], col[1], col[2], rec.vis_mag as f32]);
            shape.push([normal.x, normal.y, normal.z, rec.radius_pc as f32]);
            corner.push([c[0], c[1], i as f32, ptype as f32]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    pad_aabb(&mut positions);
    fdata.extend([[0.0; 4]; 2]);
    shape.extend([[0.0; 4]; 2]);
    corner.extend([[0.0; 4]; 2]);
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(ATTR_FAR_DATA, fdata)
        .with_inserted_attribute(ATTR_FAR_SHAPE, shape)
        .with_inserted_attribute(ATTR_FAR_CORNER, corner)
        .with_inserted_indices(Indices::U32(indices));
    let far = far_mats.add(GalaxyFarMaterial { uniforms: far_uniforms(Vec3::ZERO, &settings) });
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(far.clone()),
        Transform::IDENTITY,
        NoFrustumCulling,
    ));

    // --- Milky Way: pinned model and density ------------------------------------------------
    let milky_way = match cat.find("Milky Way") {
        Some(ObjectRef::Galaxy(i)) => Some(i as usize),
        _ => None,
    };
    let mut state = GalaxyState {
        models: Arc::new(models),
        textures: Arc::new(textures),
        model_of,
        tile_mean,
        atlas,
        far,
        slots: Vec::new(),
        failed: HashSet::new(),
        milky_way,
        gain: 1.0,
        resident: Vec::new(),
        adaptive_gain: 1.0,
    };
    if let Some(g) = milky_way {
        let rec = &cat.galaxies[g];
        if let Some(mi) = state.model_of[g] {
            let def = &state.models.models()[mi];
            let ms = def.model_scale();
            density.centre_pc = rec.pos_pc;
            density.radius_pc = rec.radius_pc;
            density.orientation = rec.orientation;
            density.scale_pc = DVec3::new(ms.x as f64, ms.y as f64, ms.z as f64) * rec.radius_pc;
            spawn_build(&mut state, &vfs, &cat.galaxies[g], g, 0.0, true);
        }
    }
    commands.insert_resource(SpriteAtlas { handle: state.atlas.clone(), tile_mean: state.tile_mean });
    commands.insert_resource(state);
}

/// Pick the sprite model of a catalog galaxy: any alias that a model names via `UseForObject`,
/// else the type pool (hash of the position).
fn select_model(models: &ModelCatalog, rec: &GalaxyRecord) -> Option<usize> {
    // Procedural galaxies (runtime catalog entries) carry the raw draw of SE's pool pick.
    if let Some(roll) = rec.model_roll {
        return models.pool_model(se_core::galaxy_model::hubble_type_index(&rec.kind), roll);
    }
    for n in &rec.names {
        // "E0" never has a type pool, so only an exact `UseForObject` match returns a model.
        if let ModelSelection::Model(i) = models.select(n, "E0", [0.0; 3]) {
            return Some(i);
        }
    }
    let name = rec.names.first().map(String::as_str).unwrap_or("");
    match models.select(name, &rec.kind, [rec.pos_pc.x, rec.pos_pc.y, rec.pos_pc.z]) {
        ModelSelection::Model(i) => Some(i),
        ModelSelection::Procedural => None,
    }
}

/// Display gain of the model's bulge sprites: 1, as in SE (galaxy_model.glsl has no per-layer gain;
/// `SE_GAL_BULGE` overrides it for comparisons).
const BULGE_GAIN: f32 = 1.0;

fn spawn_build(state: &mut GalaxyState, vfs: &Arc<Vfs>, rec: &GalaxyRecord, g: usize, now: f32, pinned: bool) {
    let Some(mi) = state.model_of[g] else { return };
    let input = BuildInput {
        vfs: vfs.clone(),
        textures: state.textures.clone(),
        def: state.models.models()[mi].clone(),
        orientation: rec.orientation,
        radius_pc: rec.radius_pc,
        map_mode: std::env::var("SE_GAL_MAP").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_MAP_MODE),
        bulge_gain: std::env::var("SE_GAL_BULGE").ok().and_then(|v| v.parse().ok()).unwrap_or(BULGE_GAIN),
    };
    let task = AsyncComputeTaskPool::get().spawn(async move { build_model(input) });
    state.slots.push(Slot { galaxy: g, state: SlotState::Loading(task), last_wanted: now, pinned });
}

/// Angular radius in pixels of a sphere of radius `r` at distance `d` (infinite when inside).
pub(crate) fn angular_px(r: f64, d: f64, focal: f64) -> f64 {
    if d <= r * 1.02 {
        f64::INFINITY
    } else {
        focal * r / (d * d - r * r).sqrt()
    }
}

pub(crate) fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Fade of the 3D model (and inverse fade of the far sprite).
pub(crate) fn model_fade(px: f64, build_px: f64) -> f32 {
    if px.is_infinite() {
        1.0
    } else {
        smoothstep(build_px * 1.5, build_px * 4.0, px) as f32
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_galaxies(
    mut commands: Commands,
    time: Res<Time>,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<GalaxySettings>,
    mut st: ResMut<GalaxyState>,
    mut density: ResMut<GalaxyDensity>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut model_mats: ResMut<Assets<GalaxyModelMaterial>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
    mut vis: Query<&mut Visibility>,
    (mut plane_mats, mut images): (ResMut<Assets<GalaxyPlaneMaterial>>, ResMut<Assets<Image>>),
) {
    let st = &mut *st;
    let cat = &data.catalog;
    // Galaxies added to the catalog at runtime (procedural `RG` galaxies) get their model choice here.
    while st.model_of.len() < cat.galaxies.len() {
        let sel = select_model(&st.models, &cat.galaxies[st.model_of.len()]);
        st.model_of.push(sel);
    }
    st.resident.clear();
    let now = time.elapsed_secs();
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();

    // ---- which galaxies want a model ------------------------------------------------------
    let mut wanted: Vec<(f64, usize)> = Vec::new();
    // Distance to the nearest bright galaxy, and how far outside every galaxy the camera is
    // (in radii of the closest one), for the intergalactic-vision gain.
    let mut nearest_bright = f64::INFINITY;
    let mut outside = f64::INFINITY;
    for rec in cat.galaxies.iter() {
        let d = (rec.pos_pc - cam_pc).length();
        outside = outside.min(d / rec.radius_pc.max(1.0));
        if rec.vis_mag < -17.0 {
            nearest_bright = nearest_bright.min((d - rec.radius_pc).max(0.0));
        }
    }
    let target_gain = if settings.auto_gain && nearest_bright.is_finite() {
        ((nearest_bright / settings.auto_d0_pc as f64).powi(2)).clamp(1.0, settings.auto_max as f64) as f32
    } else {
        1.0
    };
    // Adaptive exposure from the procedural galaxy streamer (never dims). Like SpaceEngine's
    // intergalactic vision it starts as soon as the camera leaves a galaxy, not only in the
    // voids between groups: from one to two radii out of the closest galaxy.
    let target_gain = if settings.auto_gain {
        let w = smoothstep(1.0, 2.0, outside) as f32;
        target_gain.max(st.adaptive_gain.powf(w))
    } else {
        target_gain
    };
    // Smooth in log space (about 0.3 s time constant).
    let k = (time.delta_secs() * 3.0).min(1.0);
    st.gain = (st.gain.ln() + (target_gain.ln() - st.gain.ln()) * k).exp();
    let gain = st.gain;
    if settings.enabled {
        for (g, rec) in cat.galaxies.iter().enumerate() {
            if st.model_of[g].is_none() {
                continue;
            }
            let d = (rec.pos_pc - cam_pc).length();
            let px = angular_px(rec.radius_pc, d, focal);
            if px > settings.build_px as f64 && !st.failed.contains(&g) {
                wanted.push((px, g));
            }
        }
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0));
        wanted.truncate(settings.max_models.min(MAX_SLOTS));
    }
    for &(_, g) in &wanted {
        if let Some(slot) = st.slots.iter_mut().find(|s| s.galaxy == g) {
            slot.last_wanted = now;
            continue;
        }
        if st.slots.len() >= settings.max_models.min(MAX_SLOTS) {
            // Evict the least recently wanted slot that is neither pinned nor wanted now.
            let victim = st
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.pinned && !wanted.iter().any(|w| w.1 == s.galaxy))
                .min_by(|a, b| a.1.last_wanted.total_cmp(&b.1.last_wanted))
                .map(|(i, _)| i);
            let Some(i) = victim else { continue };
            let old = st.slots.remove(i);
            despawn_slot(&mut commands, old);
        }
        spawn_build(st, &data.vfs, &cat.galaxies[g], g, now, false);
    }

    // ---- poll finished builds -------------------------------------------------------------
    for slot in &mut st.slots {
        let polled = match &mut slot.state {
            SlotState::Loading(task) => check_ready(task),
            _ => None,
        };
        let Some(result) = polled else { continue };
        let Some(built) = result else {
            slot.state = SlotState::Failed;
            st.failed.insert(slot.galaxy);
            continue;
        };
        info!(
            "galaxy model {}: {} sprites{}",
            cat.galaxies[slot.galaxy].names[0],
            built.sprites,
            if built.plane.is_some() { ", front texture" } else { "" }
        );
        let map = built.map.map(Arc::new);
        if Some(slot.galaxy) == st.milky_way {
            if let Some(map) = &map {
                density.map = Some(map.clone());
                // Sun value: emission at the origin (the Sun) in model units.
                let m = density.model_from_universe(DVec3::ZERO).as_vec3();
                density.sun_value = density.relative_emission(m);
            }
        }
        let mat = model_mats.add(GalaxyModelMaterial {
            uniforms: ModelParams {
                center_rel: Vec4::ZERO,
                params: Vec4::new(1.0, MODEL_MIN_PX, 0.0, 0.0),
                params2: Vec4::ZERO,
            },
            atlas: st.atlas.clone(),
        });
        let centres = built.buf.centres();
        let mesh = meshes.add(built.buf.into_mesh());
        let entity = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::IDENTITY,
                Visibility::Hidden,
                NoFrustumCulling,
                GalaxyEntity,
            ))
            .id();
        let plane = built.plane.map(|fp| {
            let mat = plane_mats.add(GalaxyPlaneMaterial {
                uniforms: PlaneParams { center_rel: Vec4::ZERO, axis_x: Vec4::ZERO, axis_y: Vec4::ZERO, params: Vec4::ZERO },
                texture: images.add(fp.image),
            });
            let e = commands
                .spawn((
                    Mesh3d(meshes.add(plane_mesh())),
                    MeshMaterial3d(mat.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    NoFrustumCulling,
                    GalaxyEntity,
                ))
                .id();
            (e, mat)
        });
        slot.state = SlotState::Ready(ReadyModel {
            entity,
            mat,
            order: SpriteOrder::new(centres, mesh),
            plane,
            sprites: built.sprites,
            map,
        });
    }
    // Failed slots are dropped (the galaxy stays in `failed` so it is not retried).
    st.slots.retain(|s| !matches!(s.state, SlotState::Failed));

    // ---- release models nobody looks at ---------------------------------------------------
    let mut i = 0;
    while i < st.slots.len() {
        let s = &st.slots[i];
        let stale = !s.pinned && now - s.last_wanted > 4.0 && matches!(s.state, SlotState::Ready(_));
        if stale {
            let old = st.slots.remove(i);
            despawn_slot(&mut commands, old);
        } else {
            i += 1;
        }
    }

    // ---- per-frame uniforms ---------------------------------------------------------------
    let mut far_u = far_uniforms(cam_pc.as_vec3(), &settings);
    if !settings.enabled {
        // `ShowCatGalaxies false`: no catalog galaxy is drawn.
        far_u.params.x = 0.0;
    }
    far_u.params2.z = gain;
    far_u.params2.w = VISION_CAP;
    let mut n_res = 0;
    for slot in st.slots.iter_mut() {
        let SlotState::Ready(ready) = &mut slot.state else { continue };
        let rec = &cat.galaxies[slot.galaxy];
        let def = &st.models.models()[st.model_of[slot.galaxy].unwrap()];
        let d = (rec.pos_pc - cam_pc).length();
        let px = angular_px(rec.radius_pc, d, focal);
        let fade = if settings.enabled { model_fade(px, settings.build_px as f64) } else { 0.0 };
        let center_rel = (rec.pos_pc - cam_pc).as_vec3().extend(fade);

        // SE's brightness chain (FUN_14029bdf0 / FUN_140299fe0): emBright = se_glow_bright x model
        // Brightness x the galaxy's +0x74 (catalog ModelBright) x p3 (1 - px 1e-7) x renderer +0x35ac
        // (deep-sky extinction inside an atmosphere, FUN_14044fe70; 1 in space) x +0x35b8 (1) x galaxy
        // +0xa4 (the show/hide fader, 1 in steady state); absBright = Transparency x p1, p1 = fade-in x
        // bright_term. The model fade `center_rel.w` stands in for the fade-in.
        let glow = se_glow_bright(settings.glow_mag_limit, settings.glow_bright_min, settings.sprite_bright)
            * settings.model_brightness
            * rec.model_bright as f32;
        // bright_term: DAT_14080b5a0 = GalaxyBrightMin x coef(GalaxyMagnLimit) x the intergalactic-vision
        // gain (FUN_1404c4610, FUN_1404c51a0).
        let m_app = rec.vis_mag + 5.0 * (d.max(1e-9) / 10.0).log10();
        let limit = settings.mag_ref - MAG_REF_OFFSET;
        let bright = se_bright_term(limit, m_app as f32, settings.bright_min * gain, settings.bright_min);
        let fade = if bright > 0.0 { fade } else { 0.0 };
        let center_rel = center_rel.truncate().extend(fade);
        let k_emission = glow * def.brightness;
        let k_dust = settings.dust_strength * def.transparency * bright;
        // The front texture (galaxy_plane.glsl): EyePos.w = p2 x GalaxyFrontTexBright, p2 = p1 x the chain
        // without the model's Brightness (FUN_140299fe0:189).
        if let Some((e, mat)) = &ready.plane {
            let unit_pc = rec.radius_pc * def.model_scale().x as f64;
            let axis_x = rec.orientation * se_to_port(DEFAULT_MAP_MODE, DVec3::new(unit_pc, 0.0, 0.0));
            let axis_y = rec.orientation * se_to_port(DEFAULT_MAP_MODE, DVec3::new(0.0, unit_pc, 0.0));
            let k_plane = glow * settings.front_tex_bright * bright;
            if let Some(mut m) = plane_mats.get_mut(mat) {
                m.uniforms = PlaneParams {
                    center_rel,
                    axis_x: axis_x.as_vec3().extend(0.0),
                    axis_y: axis_y.as_vec3().extend(0.0),
                    params: Vec4::new(k_plane, 1.0 / DSO_GAMMA, unit_pc as f32, 0.0),
                };
            }
            if let Ok(mut v) = vis.get_mut(*e) {
                *v = if fade > 0.001 { Visibility::Visible } else { Visibility::Hidden };
            }
        }
        if let Some(mut m) = model_mats.get_mut(&ready.mat) {
            m.uniforms.center_rel = center_rel;
            // The dust colour is subtracted at the emission brightness (SE: Color.rgb *= emBright).
            m.uniforms.params = Vec4::new(k_emission, MODEL_MIN_PX, k_dust, 0.0);
        }
        if let Ok(mut v) = vis.get_mut(ready.entity) {
            *v = if fade > 0.001 { Visibility::Visible } else { Visibility::Hidden };
        }
        ready.order.update(-center_rel.truncate(), fade > 0.001 && px > SORT_PX, &mut meshes);
        if n_res < 8 && fade > 0.0 {
            far_u.resident[n_res] = Vec4::new(slot.galaxy as f32, fade, 0.0, 0.0);
            n_res += 1;
        }
        if fade > 0.0 {
            st.resident.push((slot.galaxy, fade));
        }
        let _ = ready.sprites;
    }
    if let Some(mut m) = far_mats.get_mut(&st.far) {
        m.uniforms = far_u;
    }
}

fn despawn_slot(commands: &mut Commands, slot: Slot) {
    if let SlotState::Ready(r) = slot.state {
        for e in std::iter::once(r.entity).chain(r.plane.map(|p| p.0)) {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_vertex_clamps_like_se_bytes() {
        // SE uploads clamp(colour) and min(1, intensity) as bytes; the model Brightness multiplies emBright.
        assert_eq!(sprite_vertex([0.08, 0.24, 1.7], 5.5), [0.08, 0.24, 1.0, 1.0]);
        assert_eq!(sprite_vertex([1.0, 0.5, -0.2], 0.25), [1.0, 0.5, 0.0, 0.25]);
    }

    #[test]
    fn sprites_sort_far_to_near() {
        let c = [[0.0, 0.0, 1.0], [0.0, 0.0, 5.0], [0.0, 0.0, -3.0]];
        assert_eq!(back_to_front(&c, [0.0; 3]), vec![4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn se_model_brightness_chain() {
        // exp((7 + 0.2) / 1.0587) = 898.
        assert!((se_magn_coef(7.0) - 898.3).abs() < 0.5, "{}", se_magn_coef(7.0));
        // GlowBrightMin 0.006 x exp(7.2 / 1.0587) x hdrGalaxySpriteBright 0.03.
        assert!((se_glow_bright(7.0, 0.006, 0.03) - 0.1619).abs() < 1e-3);
        // One magnitude more on the limit: x exp(1 / 1.0587) = 2.57.
        let r = se_glow_bright(8.0, 0.006, 0.03) / se_glow_bright(7.0, 0.006, 0.03);
        assert!((r - 2.572).abs() < 1e-2, "{r}");
    }

    #[test]
    fn quasars_use_their_galaxy_type() {
        let mut rec = GalaxyRecord {
            names: vec!["QSO".into()],
            kind: "Sb".into(),
            quasar: true,
            pos_pc: DVec3::ZERO,
            radius_pc: 1.0e4,
            abs_mag: -25.0,
            vis_mag: -25.0,
            orientation: DQuat::IDENTITY,
            model_bright: 1.0,
            source: String::new(),
            model_roll: None,
        };
        assert_eq!(profile_type(&rec), 1);
        rec.kind = "E2".into();
        assert_eq!(profile_type(&rec), 0);
        assert_eq!(type_color("E2"), [1.0, 0.86, 0.66]);
    }
}
