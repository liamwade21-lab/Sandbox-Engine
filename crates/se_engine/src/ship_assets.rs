//! Spacecraft rendering assets: the hull and exhaust materials, and the worker-thread
//! builder that turns a [`ShipDesign`] into merged meshes, textures and engine plumes.
//!
//! Every module of the design is instanced with its 4x4 matrix into ship space (metres,
//! forward is -Z, shifted by the design's `Offset`). Instances that share a material are
//! merged into one mesh, so a ship with thousands of modules still costs a handful of draw
//! calls.
//!
//! [`HullMaterial`] is SE's `ship.glsl` (`shaders/ship.wesl`), which also draws the catalog
//! models (`models.rs`): the `.sml` material maps ([`load_hull_textures`]) and parameters
//! ([`hull_uniform`]) are shared.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{DMat3, DMat4, DVec3};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use se_core::dds::decode_dds;
use se_core::model_sm::{SmModel, parse_sm};
use se_core::ship::{
    DiffAlpha, EffectGroup, ModuleConfig, ShipDesign, ShipMaterial, load_module_cfg, load_ship_materials,
    resolve_mesh_path,
};
use se_core::vfs::Vfs;

// ------------------------------------------------------------------------------------
// Materials
// ------------------------------------------------------------------------------------

/// `ship.glsl` variant switches (`flags.x` in `ship.wesl`): SPEC, ASPEC, ATRNS, EMIS, DETDIF,
/// DETNRM and THERM.
pub mod hull_flags {
    pub const SPEC: u32 = 1;
    pub const ASPEC: u32 = 2;
    pub const ATRNS: u32 = 4;
    pub const EMIS: u32 = 8;
    pub const DETDIF: u32 = 16;
    pub const DETNRM: u32 = 32;
    pub const THERM: u32 = 64;
}

/// Uniform block of [`HullMaterial`] (`Hull` in `ship.wesl`; SE's `ShipUniforms`).
#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct HullUniform {
    /// xyz: unit direction towards each of up to 4 lights (SE's `MAX_LIGHTS`); w: 1 for a light in use.
    pub light_dir: [Vec4; 4],
    /// rgb: each light's colour times its intensity and eclipse shadow.
    pub light_color: [Vec4; 4],
    /// x: ambient level (`AmbientParams.x`); y: Lambert to Hapke lerp (`AmbientParams.w`);
    /// z: brightness (`Params.w`); w: the lights' specular power factor (`LightParams.z`).
    pub ambient: Vec4,
    /// `HapkeParams`: spot bright, 1 / spot width, backscatter bright, 1 / its width.
    pub hapke: Vec4,
    /// `ThermalParams`: x `RadTemp` in kK, y `ThermalEmissionShift`, z brightness.
    pub thermal: Vec4,
    pub diff_scale: Vec4,
    pub diff_const: Vec4,
    /// `EmisColor` times `EmisBright`.
    pub emis: Vec4,
    /// `SpecColor`, `SpecPower`.
    pub spec: Vec4,
    /// `DetailScale`: `DetDiffScale`, `DetDiffMag`, `DetBumpScale`, `DetBumpMag`.
    pub detail: Vec4,
    /// `MapCorrections`: `DiffGamma`, `SpecGamma`, `EmisGamma`, `DirectXNormMap`.
    pub corrections: Vec4,
    /// x: [`hull_flags`].
    pub flags: UVec4,
}

/// SE's ship material (`ship.glsl`): diffuse, normal, specular, emissive, detail diffuse,
/// detail normal and temperature maps, and the Planck table for the thermal glow.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(HullMaterialKey)]
pub struct HullMaterial {
    #[uniform(0)]
    pub u: HullUniform,
    #[texture(1)]
    #[sampler(2)]
    pub diffuse: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub normal: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub specular: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    pub emissive: Handle<Image>,
    #[texture(9)]
    #[sampler(10)]
    pub detail_diffuse: Handle<Image>,
    #[texture(11)]
    #[sampler(12)]
    pub detail_normal: Handle<Image>,
    #[texture(13)]
    #[sampler(14)]
    pub temperature: Handle<Image>,
    #[texture(15, sample_type = "float", filterable = false)]
    pub planck: Handle<Image>,
}

/// Pipeline key: alpha-tested (ATRNS) materials use the entry point with the discard; the others
/// keep the early depth test.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct HullMaterialKey {
    cutout: bool,
}

impl From<&HullMaterial> for HullMaterialKey {
    fn from(m: &HullMaterial) -> Self {
        Self { cutout: m.u.flags.x & hull_flags::ATRNS != 0 }
    }
}

impl Material for HullMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/ship.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/ship.wesl".into()
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(f) = descriptor.fragment.as_mut() {
            f.entry_point = Some(if key.bind_group_data.cutout { "fragment_cutout" } else { "fragment" }.into());
        }
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_TANGENT.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The textures a material without some map binds instead, and SE's Planck table.
#[derive(Resource, Clone)]
pub struct HullDefaults {
    pub white: Handle<Image>,
    pub black: Handle<Image>,
    pub flat_normal: Handle<Image>,
    pub planck: Handle<Image>,
    /// main-def.cfg `ThermalEmissionShift` (default 0), `ThermalParams.y`.
    pub thermal_shift: f32,
}

impl FromWorld for HullDefaults {
    fn from_world(world: &mut World) -> Self {
        let thermal_shift = world
            .get_resource::<crate::config::EngineConfig>()
            .and_then(|c| crate::stars::main_def_f64(c, "ThermalEmissionShift"))
            .unwrap_or(0.0) as f32;
        let mut images = world.resource_mut::<Assets<Image>>();
        let planck: Vec<u8> = crate::blackholes::planck_table().iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        let n = (planck.len() / 16) as u32;
        let planck = Image::new(
            Extent3d { width: n, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2,
            planck,
            TextureFormat::Rgba32Float,
            RenderAssetUsages::RENDER_WORLD,
        );
        HullDefaults {
            white: images.add(flat_texture([255, 255, 255, 255])),
            black: images.add(flat_texture([0, 0, 0, 255])),
            flat_normal: images.add(flat_raw([128, 128, 255, 255])),
            planck: images.add(planck),
            thermal_shift,
        }
    }
}

/// Registers [`HullMaterial`] and its defaults once (ships and catalog models both use it).
pub fn add_hull_material(app: &mut App) {
    if !app.is_plugin_added::<MaterialPlugin<HullMaterial>>() {
        app.add_plugins(MaterialPlugin::<HullMaterial>::default());
    }
    app.init_resource::<HullDefaults>();
}

/// The decoded maps of one material; `None` where the material has none.
#[derive(Default)]
pub struct HullTextures {
    pub diffuse: Option<Image>,
    pub normal: Option<Image>,
    pub specular: Option<Image>,
    pub emissive: Option<Image>,
    pub detail_diffuse: Option<Image>,
    pub detail_normal: Option<Image>,
    pub temperature: Option<Image>,
}

impl HullTextures {
    /// The material for these maps, the defaults filling the gaps.
    pub fn into_material(self, images: &mut Assets<Image>, d: &HullDefaults, u: HullUniform) -> HullMaterial {
        let mut add = |i: Option<Image>, def: &Handle<Image>| i.map(|i| images.add(i)).unwrap_or_else(|| def.clone());
        HullMaterial {
            u,
            diffuse: add(self.diffuse, &d.white),
            normal: add(self.normal, &d.flat_normal),
            specular: add(self.specular, &d.black),
            emissive: add(self.emissive, &d.black),
            detail_diffuse: add(self.detail_diffuse, &d.white),
            detail_normal: add(self.detail_normal, &d.flat_normal),
            temperature: add(self.temperature, &d.black),
            planck: d.planck.clone(),
        }
    }
}

/// How a map is decoded.
#[derive(Clone, Copy, PartialEq, Debug)]
enum MapKind {
    /// sRGB colour (diffuse, specular, emissive).
    Color,
    /// Raw values (normal, detail diffuse, temperature).
    Raw,
    /// A height map turned into a normal map with this height scale (bump_to_norm.glsl).
    Height(f32),
}

/// Loads every map `mat` names. `cache` shares decoded images between materials.
pub fn load_hull_textures(
    vfs: &Vfs,
    mat: &ShipMaterial,
    rad_temp: f64,
    cache: &mut HashMap<String, Option<Image>>,
) -> HullTextures {
    let mut load = |p: &Option<String>, kind: MapKind, max: u32| -> Option<Image> {
        let p = p.as_ref()?;
        let key = format!("{kind:?}|{max}|{p}");
        cache.entry(key).or_insert_with(|| load_map(vfs, p, kind, max)).clone()
    };
    let height = |is_height: bool, h: f32| if is_height { MapKind::Height(h) } else { MapKind::Raw };
    HullTextures {
        diffuse: load(&mat.diff_map, MapKind::Color, 2048),
        normal: load(&mat.norm_map, height(mat.bump_is_height, mat.bump_height), 2048),
        specular: load(&mat.spec_map, MapKind::Color, 2048),
        emissive: load(&mat.emis_map, MapKind::Color, 1024),
        detail_diffuse: load(&mat.det_diff_map, MapKind::Raw, 1024),
        detail_normal: load(&mat.det_norm_map, height(mat.det_is_height, mat.det_bump_height), 1024),
        temperature: if rad_temp > 0.0 { load(&mat.temp_map, MapKind::Raw, 1024) } else { None },
    }
}

/// The uniform block of a material (FUN_14029f200, FUN_14029f670), lights left out. `tex` says
/// which maps loaded; `rad_temp` is the design's `RadTemp` (K, 0 for none) and `shift` SE's
/// `ThermalEmissionShift`.
pub fn hull_uniform(mat: &ShipMaterial, tex: &HullTextures, rad_temp: f64, shift: f32) -> HullUniform {
    use hull_flags::*;
    let mut flags = 0;
    if tex.specular.is_some() {
        flags |= SPEC;
    }
    match mat.diff_alpha {
        DiffAlpha::Spec => flags |= ASPEC,
        DiffAlpha::Transp => flags |= ATRNS,
        DiffAlpha::None => {}
    }
    if tex.emissive.is_some() {
        flags |= EMIS;
    }
    if tex.detail_diffuse.is_some() {
        flags |= DETDIF;
    }
    if tex.detail_normal.is_some() {
        flags |= DETNRM;
    }
    if tex.temperature.is_some() && rad_temp > 0.0 {
        flags |= THERM;
    }
    let v3 = |c: [f32; 3], w: f32| Vec4::new(c[0], c[1], c[2], w);
    let e = mat.emis_bright;
    HullUniform {
        light_dir: [Vec4::new(0.0, 1.0, 0.0, 1.0), Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
        light_color: [Vec4::ONE, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
        ambient: Vec4::new(0.0, 0.0, 1.0, 1.0),
        hapke: Vec4::ZERO,
        thermal: Vec4::new((rad_temp * 1e-3) as f32, shift, 1.0, 0.0),
        diff_scale: v3(mat.diff_color, 0.0),
        diff_const: v3(mat.diff_const, 0.0),
        emis: Vec4::new(mat.emis_color[0] * e, mat.emis_color[1] * e, mat.emis_color[2] * e, 0.0),
        spec: v3(mat.spec_color, mat.spec_power),
        detail: Vec4::new(mat.det_diff_scale, mat.det_diff_mag, mat.det_bump_scale, mat.det_bump_mag),
        corrections: Vec4::new(mat.diff_gamma, mat.spec_gamma, mat.emis_gamma, if mat.directx_norm { 1.0 } else { 0.0 }),
        flags: UVec4::new(flags, 0, 0, 0),
    }
}

/// The Hapke parameters of a body's `Surface` block (SE takes them from the lit body, as the
/// planet shaders do): the Lambert to Hapke lerp, and `HapkeParams`.
pub fn surface_hapke(surface: Option<&se_core::sc::Block>) -> (f32, Vec4) {
    let f = |k: &str| surface.and_then(|s| s.f64(k)).unwrap_or(0.0) as f32;
    let inv = |w: f32| if w > 0.0 { 1.0 / w } else { 0.0 };
    let (bw, cw) = (f("SpotWidth"), f("SpotWidthCB"));
    let hapke = surface.and_then(|s| s.f64("Hapke").or_else(|| s.f64("Lommel"))).unwrap_or(0.0) as f32;
    (
        hapke.clamp(0.0, 1.0),
        Vec4::new(if bw > 0.0 { f("SpotBright") } else { 0.0 }, inv(bw), if cw > 0.0 { f("SpotBrightCB") } else { 0.0 }, inv(cw)),
    )
}

/// Uniform block of [`PlumeMaterial`] (`Plume` in `ship_plume.wesl`; SE's `ShipEngineUniforms`).
#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct PlumeUniform {
    /// rgb: the effect `Color` times the level and brightness factor ([`engine_brightness`]).
    pub color: Vec4,
    /// x: engine type ([`engine_type`]), y: time (s), z: integration step ([`engine_step`]),
    /// w: 1 while the camera is inside the box.
    pub params: Vec4,
}

/// Additive ray-marched exhaust volume (`ship_engines.glsl`).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(PlumeMaterialKey)]
pub struct PlumeMaterial {
    #[uniform(0)]
    pub u: PlumeUniform,
}

/// Pipeline key: from outside, the box's front faces are drawn (the hull in front of the exhaust
/// hides it, the hull behind does not); with the camera inside, its back faces.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct PlumeMaterialKey {
    inside: bool,
}

impl From<&PlumeMaterial> for PlumeMaterialKey {
    fn from(m: &PlumeMaterial) -> Self {
        Self { inside: m.u.params.w > 0.5 }
    }
}

impl Material for PlumeMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/ship_plume.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/ship_plume.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // After the sky shells, which would otherwise hide it in daylight (see `SkyMaterial`).
        1.0e10
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        use bevy::render::render_resource::Face;
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = Some(if key.bind_group_data.inside { Face::Front } else { Face::Back });
        Ok(())
    }
}

/// The exhaust shapes SE has a `ship_<Type>_engine.glh` density for (the engine shader list
/// FUN_1401f4e10 builds); an effect of another `Type` (e.g. `Hyper`) is not drawn
/// (FUN_14048d180 skips an index of -1).
pub const ENGINE_TYPES: [&str; 8] = ["Fusion", "Chemical", "Ion", "Ion2", "Aerospike", "Sphere", "Square", "SuperDraco_1atm"];

/// Index of an effect `Type` in [`ENGINE_TYPES`] (`ship_plume.wesl`'s `params.x`).
pub fn engine_type(kind: &str) -> Option<usize> {
    ENGINE_TYPES.iter().position(|t| t.eq_ignore_ascii_case(kind))
}

/// The ray-march step in box units for `ShipEngineQuality` 0..3 (FUN_14048d180: 0.2, 0.1, 0.05,
/// 0.02; main-def.cfg's default is 2).
pub fn engine_step(quality: i32) -> f32 {
    match quality {
        i32::MIN..=0 => 0.2,
        1 => 0.1,
        2 => 0.05,
        _ => 0.02,
    }
}

/// The exhaust brightness factor of an effect at `level` (FUN_14048d180):
/// `clamp(15 sqrt(level), 0, 20)` times `hdrShipEnginesBright` and the effect's `BrightnessHDR`.
pub fn engine_brightness(level: f32, engines_bright: f32, effect_bright: f32) -> f32 {
    (level.max(0.0).sqrt() * 15.0).clamp(0.0, 20.0) * engines_bright * effect_bright
}

/// An engine effect's box: the nozzle on its z = +1 face, `size` the half extents (SE's
/// `Size`), reaching `2 size.z` along the exhaust. Returns the translation, rotation and scale in
/// ship space (FUN_14048d180: scale, shift by `-size.z`, rotate, move to `Pos`).
pub fn plume_box(pos: Vec3, dir: Vec3, x_axis: Vec3, size: Vec3) -> Transform {
    // Box -z along the exhaust, +x along the effect's x axis.
    let rot = plume_rotation(dir, x_axis) * Quat::from_rotation_x(std::f32::consts::PI);
    Transform { translation: pos + dir.normalize_or(Vec3::Z) * size.z, rotation: rot, scale: size.max(Vec3::splat(1e-3)) }
}

/// The unit box `[-1, 1]^3` the exhaust is marched in.
pub fn plume_mesh() -> Mesh {
    Cuboid::new(2.0, 2.0, 2.0).mesh().build()
}

// ------------------------------------------------------------------------------------
// Loaded ship
// ------------------------------------------------------------------------------------

/// One merged mesh with its maps and material parameters (lights left out).
pub struct HullPart {
    pub mesh: Mesh,
    pub tex: HullTextures,
    pub u: HullUniform,
}

/// An engine plume placed in ship space.
#[derive(Clone, Debug)]
pub struct PlumeSpec {
    pub group: EffectGroup,
    pub kind: String,
    pub color: [f32; 3],
    /// Nozzle position (m).
    pub pos: Vec3,
    /// Unit exhaust direction.
    pub dir: Vec3,
    /// Local transform basis: x axis hint (so flat plumes keep their orientation).
    pub x_axis: Vec3,
    /// `Size`: the half width, half height and half length of the exhaust box in metres.
    pub size: Vec3,
    /// `BrightnessHDR`.
    pub brightness: f32,
    /// Looping sound of the effect (cfg `Sound` or the group's default) and its `Volume`.
    pub sound: Option<String>,
    pub volume: f32,
}

pub struct LoadedShip {
    pub groups: Vec<HullPart>,
    pub plumes: Vec<PlumeSpec>,
    /// Distance from the pivot to the lowest hull point along -Y (m), for ground contact.
    pub clearance: f64,
    /// Bounding radius around the pivot (m).
    pub radius: f64,
    pub vertices: usize,
    pub modules_missing: usize,
}

#[derive(Default)]
struct GroupAcc {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    tan: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

struct CfgEntry {
    cfg: ModuleConfig,
    sm: Option<Arc<SmModel>>,
    /// Material per cfg `Material` slot.
    mats: Vec<Option<ShipMaterial>>,
}

/// Area-weighted vertex normals for meshes that carry none.
fn vertex_normals(pos: &[[f32; 3]], idx: &[u32]) -> Vec<[f32; 3]> {
    let mut acc = vec![Vec3::ZERO; pos.len()];
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (Vec3::from(pos[t[0] as usize]), Vec3::from(pos[t[1] as usize]), Vec3::from(pos[t[2] as usize]));
        let n = (b - a).cross(c - a);
        for &i in t {
            acc[i as usize] += n;
        }
    }
    acc.into_iter().map(|n| n.try_normalize().unwrap_or(Vec3::Y).to_array()).collect()
}

/// Per-vertex tangents for meshes that carry none, in the `.sm` convention `ship.glsl` reads:
/// `xyz` along increasing u, and `w` the sign that makes the binormal `w * cross(T, N)` point
/// along increasing v.
pub fn generate_tangents(pos: &[[f32; 3]], nrm: &[[f32; 3]], uv: &[[f32; 2]], idx: &[u32]) -> Vec<[f32; 4]> {
    let mut tan = vec![Vec3::ZERO; pos.len()];
    let mut bit = vec![Vec3::ZERO; pos.len()];
    for t in idx.chunks_exact(3) {
        let [i0, i1, i2] = [t[0] as usize, t[1] as usize, t[2] as usize];
        let (p0, p1, p2) = (Vec3::from(pos[i0]), Vec3::from(pos[i1]), Vec3::from(pos[i2]));
        let (e1, e2) = (p1 - p0, p2 - p0);
        let (du1, dv1) = (uv[i1][0] - uv[i0][0], uv[i1][1] - uv[i0][1]);
        let (du2, dv2) = (uv[i2][0] - uv[i0][0], uv[i2][1] - uv[i0][1]);
        let r = du1 * dv2 - du2 * dv1;
        if r.abs() < 1e-12 {
            continue;
        }
        let (s, v) = ((e1 * dv2 - e2 * dv1) / r, (e2 * du1 - e1 * du2) / r);
        for i in [i0, i1, i2] {
            tan[i] += s;
            bit[i] += v;
        }
    }
    (0..pos.len())
        .map(|i| {
            let n = Vec3::from(nrm[i]);
            let t = (tan[i] - n * n.dot(tan[i])).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            let w = if t.cross(n).dot(bit[i]) < 0.0 { -1.0 } else { 1.0 };
            [t.x, t.y, t.z, w]
        })
        .collect()
}

/// The `.sm` tangents of an object when it has usable ones, else generated ones.
pub fn object_tangents(stored: &[[f32; 4]], pos: &[[f32; 3]], nrm: &[[f32; 3]], uv: &[[f32; 2]], idx: &[u32]) -> Vec<[f32; 4]> {
    let usable = stored.len() == pos.len() && stored.iter().all(|t| t[3] != 0.0 && t[0] * t[0] + t[1] * t[1] + t[2] * t[2] > 1e-6);
    if usable { stored.to_vec() } else { generate_tangents(pos, nrm, uv, idx) }
}

/// SE's bump_to_norm.glsl (seTextureGenerator::ConvertToNormalMap, FUN_1402fd4e0): a height
/// map (red channel, times `scale`) to a normal map, the average of the four corner normals
/// around each texel; alpha keeps the height.
pub fn bump_to_normal(rgba: &[u8], w: u32, h: u32, scale: f32) -> Vec<u8> {
    let (wi, hi) = (w as i64, h as i64);
    let hgt = |x: i64, y: i64| rgba[((y.rem_euclid(hi) * wi + x.rem_euclid(wi)) * 4) as usize] as f32 / 255.0 * scale;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..hi {
        for x in 0..wi {
            let (pc, pl, pr, pu, pd) = (hgt(x, y), hgt(x - 1, y), hgt(x + 1, y), hgt(x, y - 1), hgt(x, y + 1));
            let s = (Vec3::new(pu - pc, pc - pl, 1.0).normalize()
                + Vec3::new(pc - pd, pc - pl, 1.0).normalize()
                + Vec3::new(pc - pd, pr - pc, 1.0).normalize()
                + Vec3::new(pu - pc, pr - pc, 1.0).normalize())
            .normalize();
            let o = ((y * wi + x) * 4) as usize;
            let c = s * 0.5 + 0.5;
            out[o..o + 4].copy_from_slice(&[
                (c.x * 255.0).round() as u8,
                (c.y * 255.0).round() as u8,
                (c.z * 255.0).round() as u8,
                rgba[o],
            ]);
        }
    }
    out
}

/// Decodes a map (PNG/JPEG via `image`, DXT/BC5 DDS via `se_core::dds`) with a full mip
/// chain, at most `max_dim` pixels on a side.
fn load_map(vfs: &Vfs, path: &str, kind: MapKind, max_dim: u32) -> Option<Image> {
    let bytes = vfs.read(path).ok()?;
    let (mut w, mut h, mut rgba) = if path.to_ascii_lowercase().ends_with(".dds") {
        let d = decode_dds(&bytes).ok()?;
        (d.width, d.height, d.rgba)
    } else {
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        (img.width(), img.height(), img.into_raw())
    };
    if let MapKind::Height(scale) = kind {
        rgba = bump_to_normal(&rgba, w, h, scale);
    }
    while w > max_dim || h > max_dim {
        rgba = box_down(&rgba, w, h);
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let format = if kind == MapKind::Color { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    Some(image_with_mips(rgba, w, h, format))
}

/// Halves an RGBA8 image with a 2x2 box filter (odd sizes drop the last row/column).
pub(crate) fn box_down(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (nw, nh) = ((w / 2).max(1) as usize, (h / 2).max(1) as usize);
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let (x0, y0) = ((x * 2).min(w - 1), (y * 2).min(h - 1));
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            for c in 0..4 {
                let s = src[(y0 * w + x0) * 4 + c] as u32
                    + src[(y0 * w + x1) * 4 + c] as u32
                    + src[(y1 * w + x0) * 4 + c] as u32
                    + src[(y1 * w + x1) * 4 + c] as u32;
                out[(y * nw + x) * 4 + c] = ((s + 2) / 4) as u8;
            }
        }
    }
    out
}

fn image_with_mips(rgba: Vec<u8>, w: u32, h: u32, format: TextureFormat) -> Image {
    let mut data = rgba.clone();
    let level0 = rgba.clone();
    let (mut cw, mut ch, mut cur) = (w, h, rgba);
    let mut levels = 1;
    while cw > 1 || ch > 1 {
        cur = box_down(&cur, cw, ch);
        cw = (cw / 2).max(1);
        ch = (ch / 2).max(1);
        data.extend_from_slice(&cur);
        levels += 1;
    }
    let mut image = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        level0,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    if levels > 1 {
        image.texture_descriptor.mip_level_count = levels;
        image.data = Some(data);
    }
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

/// 1x1 sRGB texture of one colour, used when a material has no map.
pub fn flat_texture(rgba: [u8; 4]) -> Image {
    Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// 1x1 raw texture (a flat normal map).
fn flat_raw(rgba: [u8; 4]) -> Image {
    Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// The material of a module object without a known `.sml` preset: grey, no maps.
fn missing_material() -> ShipMaterial {
    ShipMaterial { name: String::new(), diff_color: [0.6; 3], spec_color: [0.0; 3], diff_alpha: DiffAlpha::None, ..Default::default() }
}

/// Builds all meshes, textures and plume placements of a design. Runs on a worker thread.
/// `thermal_shift` is SE's `ThermalEmissionShift`.
pub fn build_ship(vfs: &Vfs, design: &ShipDesign, thermal_shift: f32) -> Option<LoadedShip> {
    let presets = load_ship_materials(vfs);
    let ship_rot = DMat4::from_quat(design.quat);
    let mut cfgs: HashMap<String, Option<Arc<CfgEntry>>> = HashMap::new();
    let mut meshes: HashMap<String, Option<Arc<SmModel>>> = HashMap::new();
    // Merged geometry per material name ("" for objects without a preset).
    let mut accs: HashMap<String, (ShipMaterial, GroupAcc)> = HashMap::new();
    let mut plumes = Vec::new();
    let (mut mn, mut mx) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
    let mut radius2 = 0.0f64;
    let mut missing = 0;
    let mut vertices = 0;

    for module in &design.modules {
        let key = module.cfg.to_ascii_lowercase();
        let entry = cfgs
            .entry(key)
            .or_insert_with(|| {
                let cfg = load_module_cfg(vfs, &design.path, &module.cfg)?;
                let sm = resolve_mesh_path(vfs, &cfg).and_then(|p| {
                    meshes
                        .entry(p.to_ascii_lowercase())
                        .or_insert_with(|| vfs.read(&p).ok().and_then(|b| parse_sm(&b).ok()).map(Arc::new))
                        .clone()
                });
                let mats = cfg.materials.iter().map(|n| presets.get(&n.to_ascii_lowercase()).cloned()).collect();
                Some(Arc::new(CfgEntry { cfg, sm, mats }))
            })
            .clone();
        let Some(entry) = entry else {
            missing += 1;
            continue;
        };

        // Module space (metres) -> ship space: model orientation, then the header `Offset`.
        let full = ship_rot * module.transform();
        let lin = DMat3::from_mat4(full);
        let normal_m = if lin.determinant().abs() > 1e-18 { lin.inverse().transpose() } else { DMat3::IDENTITY };
        let flip = lin.determinant() < 0.0;
        let unit = entry.cfg.unit;
        let to_ship = |p: DVec3| full.transform_point3(p) + design.offset;

        if let Some(sm) = &entry.sm {
            for obj in &sm.objects {
                let tri = obj.triangle_indices();
                if obj.positions.is_empty() || tri.is_empty() {
                    continue;
                }
                let slot = (obj.material_index as usize).min(entry.mats.len().saturating_sub(1));
                let mat = entry.mats.get(slot).and_then(|m| m.as_ref());
                let name = mat.map_or(String::new(), |m| m.name.to_ascii_lowercase());
                let (_, acc) = accs
                    .entry(name)
                    .or_insert_with(|| (mat.cloned().unwrap_or_else(missing_material), GroupAcc::default()));
                let base = acc.pos.len() as u32;
                let n = obj.positions.len();
                let uvs = if obj.uvs.len() == n { obj.uvs.clone() } else { vec![[0.0, 0.0]; n] };
                let mut local_pos = Vec::with_capacity(n);
                for p in &obj.positions {
                    let q = to_ship(DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) * unit);
                    mn = mn.min(q);
                    mx = mx.max(q);
                    radius2 = radius2.max(q.length_squared());
                    local_pos.push(q.as_vec3().to_array());
                }
                let mut idx = tri.clone();
                if flip {
                    for t in idx.chunks_exact_mut(3) {
                        t.swap(1, 2);
                    }
                }
                let nrm: Vec<[f32; 3]> = if obj.normals.len() == n {
                    obj.normals
                        .iter()
                        .map(|v| (normal_m * DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64)).normalize_or(DVec3::Y).as_vec3().to_array())
                        .collect()
                } else {
                    vertex_normals(&local_pos, &idx)
                };
                // Tangents follow the module matrix; a mirroring one flips the binormal sign.
                let stored: Vec<[f32; 4]> = obj
                    .tangents
                    .iter()
                    .map(|t| {
                        let v = (lin * DVec3::new(t[0] as f64, t[1] as f64, t[2] as f64)).normalize_or_zero().as_vec3();
                        [v.x, v.y, v.z, if flip { -t[3] } else { t[3] }]
                    })
                    .collect();
                acc.tan.extend(object_tangents(&stored, &local_pos, &nrm, &uvs, &idx));
                acc.pos.extend(local_pos);
                acc.nrm.extend(nrm);
                acc.uv.extend(uvs);
                acc.idx.extend(idx.iter().map(|i| base + i));
                vertices += n;
            }
        }

        for e in &entry.cfg.effects {
            let r = e.rotation();
            let dir = (lin * (r * DVec3::NEG_Z)).normalize_or(DVec3::Z);
            let xa = (lin * (r * DVec3::X)).normalize_or(DVec3::X);
            plumes.push(PlumeSpec {
                group: e.group,
                kind: e.kind.clone(),
                color: e.color,
                pos: to_ship(e.pos).as_vec3(),
                dir: dir.as_vec3(),
                x_axis: xa.as_vec3(),
                size: e.size.as_vec3(),
                brightness: e.brightness_hdr as f32,
                sound: e.sound_file().map(str::to_owned),
                volume: e.volume as f32,
            });
        }
    }

    // Texture decode, shared between groups.
    let mut tex_cache: HashMap<String, Option<Image>> = HashMap::new();
    let mut groups = Vec::new();
    let mut keys: Vec<String> = accs.keys().cloned().collect();
    keys.sort();
    for key in keys {
        let (mat, acc) = accs.remove(&key)?;
        if acc.idx.is_empty() {
            continue;
        }
        let tex = load_hull_textures(vfs, &mat, design.rad_temp, &mut tex_cache);
        let u = hull_uniform(&mat, &tex, design.rad_temp, thermal_shift);
        debug!(
            "ship group {key}: diffuse {:?} (decoded {}), normal {:?}, flags {:#x}, {} vertices",
            mat.diff_map,
            tex.diffuse.is_some(),
            mat.norm_map,
            u.flags.x,
            acc.pos.len()
        );
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, acc.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, acc.nrm)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, acc.uv)
            .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, acc.tan)
            .with_inserted_indices(Indices::U32(acc.idx));
        groups.push(HullPart { mesh, tex, u });
    }
    if groups.is_empty() {
        return None;
    }
    let clearance = if mn.y.is_finite() && mn.y < 0.0 { (-mn.y).clamp(1.0, design.length_m * 0.5) } else { design.length_m * 0.1 };
    Some(LoadedShip {
        groups,
        plumes,
        clearance,
        radius: radius2.sqrt().max(1.0),
        vertices,
        modules_missing: missing,
    })
}

/// Local-to-ship rotation of a plume: mesh +Z along the exhaust, mesh +X along the
/// effect's x axis (orthogonalised), right-handed.
pub fn plume_rotation(dir: Vec3, x_axis: Vec3) -> Quat {
    let z = dir.normalize_or(Vec3::Z);
    let mut x = (x_axis - z * x_axis.dot(z)).normalize_or(Vec3::ZERO);
    if x == Vec3::ZERO {
        x = z.any_orthonormal_vector();
    }
    let y = z.cross(x);
    Quat::from_mat3(&Mat3::from_cols(x, y, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plume_rotation_points_mesh_z_along_exhaust() {
        let q = plume_rotation(Vec3::new(0.0, 0.0, 1.0), Vec3::X);
        assert!((q * Vec3::Z - Vec3::Z).length() < 1e-5);
        let q = plume_rotation(Vec3::NEG_X, Vec3::Y);
        assert!((q * Vec3::Z - Vec3::NEG_X).length() < 1e-5);
        assert!((q * Vec3::X - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn mip_chain_has_expected_size() {
        let img = image_with_mips(vec![255; 8 * 4 * 4], 8, 4, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(img.texture_descriptor.mip_level_count, 4);
        // 8x4 + 4x2 + 2x1 + 1x1 texels, 4 bytes each.
        assert_eq!(img.data.as_ref().unwrap().len(), (32 + 8 + 2 + 1) * 4);
    }

    #[test]
    fn generated_tangents_follow_u_and_binormal_v() {
        // A quad in the xy plane facing +z, u along +x and v along +y.
        let pos = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let nrm = [[0.0, 0.0, 1.0]; 4];
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let t = generate_tangents(&pos, &nrm, &uv, &[0, 1, 2, 0, 2, 3]);
        for v in &t {
            assert!((Vec3::new(v[0], v[1], v[2]) - Vec3::X).length() < 1e-5);
            // ship.glsl's binormal w * cross(T, N) = -cross(X, Z) = +Y; the .sm files use w = -1 here too.
            assert_eq!(v[3], -1.0);
        }
        // Stored tangents win unless degenerate.
        let stored = [[0.0, 1.0, 0.0, 1.0]; 4];
        assert_eq!(object_tangents(&stored, &pos, &nrm, &uv, &[0, 1, 2]), stored.to_vec());
        assert_eq!(object_tangents(&[[0.0; 4]; 4], &pos, &nrm, &uv, &[0, 1, 2, 0, 2, 3]), t);
    }

    #[test]
    fn bump_to_normal_matches_se() {
        // Flat: straight up, height kept in alpha.
        let flat = bump_to_normal(&[100; 4 * 4 * 4], 4, 4, 2.0);
        assert_eq!(&flat[0..4], &[128, 128, 255, 100]);
        // Height rising along +x tilts SE's normal towards +y (pc - pl in the y slot).
        let mut ramp = vec![0u8; 4 * 1 * 4];
        for x in 0..4 {
            ramp[x * 4] = (x * 60) as u8;
        }
        let n = bump_to_normal(&ramp, 4, 1, 1.0);
        let px = &n[4..8];
        assert!(px[1] > 140 && (px[0] as i32 - 128).abs() <= 1, "{px:?}");
    }

    #[test]
    fn hull_uniform_switches_follow_the_maps() {
        let m = ShipMaterial { diff_alpha: DiffAlpha::Transp, spec_power: 50.0, emis_bright: 0.5, ..Default::default() };
        let tex = HullTextures { specular: Some(flat_texture([0; 4])), temperature: Some(flat_texture([0; 4])), ..Default::default() };
        let u = hull_uniform(&m, &tex, 1500.0, 0.0);
        assert_eq!(u.flags.x, hull_flags::SPEC | hull_flags::ATRNS | hull_flags::THERM);
        assert_eq!((u.spec.w, u.emis.x, u.thermal.x), (50.0, 0.5, 1.5));
        // No RadTemp: no thermal glow even with a TempMap.
        assert_eq!(hull_uniform(&m, &tex, 0.0, 0.0).flags.x & hull_flags::THERM, 0);
        assert_eq!(u.detail, Vec4::new(32.0, 0.5, 32.0, 0.25));
    }

    #[test]
    fn plume_box_starts_at_the_nozzle() {
        let size = Vec3::new(7.0, 7.0, 300.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        let tf = plume_box(Vec3::new(0.0, 53.7, 37.6), dir, Vec3::X, size);
        // The z = +1 face is at the nozzle, z = -1 2 size.z down the exhaust.
        assert!((tf.transform_point(Vec3::Z) - Vec3::new(0.0, 53.7, 37.6)).length() < 1e-3);
        assert!((tf.transform_point(-Vec3::Z) - Vec3::new(0.0, 53.7, 637.6)).length() < 1e-2);
        assert!((tf.transform_point(Vec3::X) - tf.translation - Vec3::X * 7.0).length() < 1e-3);
        let m = plume_mesh();
        let ext = m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap().iter().map(|p| p[2]).fold(0.0f32, f32::max);
        assert!((ext - 1.0).abs() < 1e-6);
    }

    #[test]
    fn engine_constants_match_the_executable() {
        assert_eq!(engine_type("Fusion"), Some(0));
        assert_eq!(engine_type("superdraco_1atm"), Some(7));
        assert_eq!(engine_type("Hyper"), None);
        assert_eq!([engine_step(0), engine_step(1), engine_step(2), engine_step(3)], [0.2, 0.1, 0.05, 0.02]);
        assert_eq!(engine_brightness(1.0, 1.0, 1.0), 15.0);
        assert_eq!(engine_brightness(4.0, 1.0, 2.0), 40.0);
        assert_eq!(engine_brightness(0.0, 1.0, 1.0), 0.0);
    }
}
