//! Spacecraft rendering assets: the hull and exhaust materials, and the worker-thread
//! builder that turns a [`ShipDesign`] into merged meshes, textures and engine plumes.
//!
//! Every module of the design is instanced with its 4x4 matrix into ship space (metres,
//! forward is -Z, shifted by the design's `Offset`). Instances that share a material
//! (texture, tint, emissive map, alpha mode) are merged into one mesh, so a ship with
//! thousands of modules still costs a handful of draw calls.

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
    EffectGroup, ModuleConfig, ShipDesign, ShipMaterial, load_module_cfg, load_ship_materials, resolve_mesh_path,
};
use se_core::vfs::Vfs;

// ------------------------------------------------------------------------------------
// Materials
// ------------------------------------------------------------------------------------

/// Uniform block of [`ShipSurfaceMaterial`] (`Ship` in `ship.wesl`).
#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct ShipUniform {
    /// xyz: unit direction towards the star; w: star intensity.
    pub sun_dir: Vec4,
    /// rgb: star colour.
    pub sun_color: Vec4,
    /// rgb: diffuse tint; w: ambient level.
    pub tint: Vec4,
    /// rgb: emissive colour times brightness; w: alpha cut-off (negative: opaque).
    pub emis: Vec4,
}

/// Lambert hull material with a diffuse and an emissive texture.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ShipSurfaceMaterial {
    #[uniform(0)]
    pub u: ShipUniform,
    #[texture(1)]
    #[sampler(2)]
    pub diffuse: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub emissive: Handle<Image>,
}

impl Material for ShipSurfaceMaterial {
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
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Uniform block of [`PlumeMaterial`] (`Plume` in `ship_plume.wesl`).
#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct PlumeUniform {
    /// rgb colour, w intensity.
    pub color: Vec4,
    /// x: level 0..1, y: time, z: seed.
    pub params: Vec4,
}

/// Additive exhaust glow.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PlumeMaterial {
    #[uniform(0)]
    pub u: PlumeUniform,
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
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Teardrop plume along +Z (nozzle at z=0 with radius ~0.55, tip at z=1), unit width.
pub fn plume_mesh() -> Mesh {
    const SEGS: usize = 14;
    const RINGS: usize = 12;
    let radius = |t: f32| (1.0 - t) * (0.55 + 0.9 * t.sqrt());
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    for r in 0..=RINGS {
        let t = r as f32 / RINGS as f32;
        let rad = radius(t);
        let slope = (radius((t + 0.01).min(1.0)) - radius((t - 0.01).max(0.0))) / 0.02;
        for s in 0..=SEGS {
            let a = s as f32 / SEGS as f32 * std::f32::consts::TAU;
            let (sa, ca) = a.sin_cos();
            pos.push([ca * rad, sa * rad, t]);
            nrm.push(Vec3::new(ca, sa, -slope).normalize().to_array());
            uv.push([t, s as f32 / SEGS as f32]);
        }
    }
    let mut idx = Vec::new();
    for r in 0..RINGS {
        for s in 0..SEGS {
            let a = (r * (SEGS + 1) + s) as u32;
            let b = a + (SEGS + 1) as u32;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

// ------------------------------------------------------------------------------------
// Loaded ship
// ------------------------------------------------------------------------------------

/// One merged mesh with its material parameters.
pub struct LoadedGroup {
    pub mesh: Mesh,
    pub diffuse: Option<Image>,
    pub emissive: Option<Image>,
    pub tint: [f32; 3],
    /// Emissive colour times brightness.
    pub emis: [f32; 3],
    /// Alpha cut-off, negative for opaque materials.
    pub cutoff: f32,
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
    /// Width, height, length in metres.
    pub size: Vec3,
    pub brightness: f32,
    /// Looping sound of the effect (cfg `Sound` or the group's default) and its `Volume`.
    pub sound: Option<String>,
    pub volume: f32,
}

pub struct LoadedShip {
    pub groups: Vec<LoadedGroup>,
    pub plumes: Vec<PlumeSpec>,
    /// Distance from the pivot to the lowest hull point along -Y (m), for ground contact.
    pub clearance: f64,
    /// Bounding radius around the pivot (m).
    pub radius: f64,
    pub vertices: usize,
    pub modules_missing: usize,
}

#[derive(PartialEq, Eq, Hash, Clone)]
struct GroupKey {
    diffuse: Option<String>,
    emissive: Option<String>,
    tint: [i32; 3],
    emis: [i32; 3],
    cutoff: bool,
}

#[derive(Default)]
struct GroupAcc {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
    tint: [f32; 3],
    emis: [f32; 3],
    cutoff: bool,
}

struct CfgEntry {
    cfg: ModuleConfig,
    sm: Option<Arc<SmModel>>,
    /// Material per cfg `Material` slot.
    mats: Vec<Option<ShipMaterial>>,
}

fn quant(c: [f32; 3]) -> [i32; 3] {
    [(c[0] * 1000.0) as i32, (c[1] * 1000.0) as i32, (c[2] * 1000.0) as i32]
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

/// Decodes a texture (PNG/JPEG via `image`, DXT DDS via `se_core::dds`) into an
/// sRGB image with a full mip chain, at most `max_dim` pixels on a side.
pub fn load_texture(vfs: &Vfs, path: &str, max_dim: u32) -> Option<Image> {
    let bytes = vfs.read(path).ok()?;
    let (mut w, mut h, mut rgba) = if path.to_ascii_lowercase().ends_with(".dds") {
        let d = decode_dds(&bytes).ok()?;
        (d.width, d.height, d.rgba)
    } else {
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        (img.width(), img.height(), img.into_raw())
    };
    while w > max_dim || h > max_dim {
        rgba = box_down(&rgba, w, h);
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    Some(image_with_mips(rgba, w, h))
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

fn image_with_mips(rgba: Vec<u8>, w: u32, h: u32) -> Image {
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
        TextureFormat::Rgba8UnormSrgb,
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

/// 1x1 texture of one colour, used when a material has no map.
pub fn flat_texture(rgba: [u8; 4]) -> Image {
    Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Builds all meshes, textures and plume placements of a design. Runs on a worker thread.
pub fn build_ship(vfs: &Vfs, design: &ShipDesign) -> Option<LoadedShip> {
    let presets = load_ship_materials(vfs);
    let ship_rot = DMat4::from_quat(design.quat);
    let mut cfgs: HashMap<String, Option<Arc<CfgEntry>>> = HashMap::new();
    let mut meshes: HashMap<String, Option<Arc<SmModel>>> = HashMap::new();
    let mut accs: HashMap<GroupKey, GroupAcc> = HashMap::new();
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
                let (diffuse, emissive, tint, emis_col, bright) = match mat {
                    Some(m) => (m.diff_map.clone(), m.emis_map.clone(), m.diff_color, m.emis_color, m.emis_bright),
                    None => (None, None, [0.6; 3], [0.0; 3], 0.0),
                };
                let cutoff = mat.is_some_and(|m| m.diff_alpha.as_deref().is_some_and(|a| a.eq_ignore_ascii_case("transp")));
                let emis = [emis_col[0] * bright, emis_col[1] * bright, emis_col[2] * bright];
                let gkey = GroupKey {
                    emissive: emissive.filter(|_| bright > 0.0),
                    diffuse,
                    tint: quant(tint),
                    emis: quant(emis),
                    cutoff,
                };
                let acc = accs.entry(gkey).or_insert_with(|| GroupAcc { tint, emis, cutoff, ..Default::default() });
                let base = acc.pos.len() as u32;
                let normals = if obj.normals.len() == obj.positions.len() { Some(&obj.normals) } else { None };
                let mut local_pos = Vec::with_capacity(obj.positions.len());
                for (i, p) in obj.positions.iter().enumerate() {
                    let q = to_ship(DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) * unit);
                    mn = mn.min(q);
                    mx = mx.max(q);
                    radius2 = radius2.max(q.length_squared());
                    local_pos.push(q.as_vec3().to_array());
                    acc.pos.push(q.as_vec3().to_array());
                    if let Some(n) = normals {
                        let nv = normal_m * DVec3::new(n[i][0] as f64, n[i][1] as f64, n[i][2] as f64);
                        acc.nrm.push(nv.normalize_or(DVec3::Y).as_vec3().to_array());
                    }
                }
                if normals.is_none() {
                    let mut idx = tri.clone();
                    if flip {
                        for t in idx.chunks_exact_mut(3) {
                            t.swap(1, 2);
                        }
                    }
                    acc.nrm.extend(vertex_normals(&local_pos, &idx));
                }
                if obj.uvs.len() == obj.positions.len() {
                    acc.uv.extend_from_slice(&obj.uvs);
                } else {
                    acc.uv.extend(std::iter::repeat_n([0.0, 0.0], obj.positions.len()));
                }
                for t in tri.chunks_exact(3) {
                    if flip {
                        acc.idx.extend_from_slice(&[base + t[0], base + t[2], base + t[1]]);
                    } else {
                        acc.idx.extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
                    }
                }
                vertices += obj.positions.len();
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
                brightness: (e.brightness_hdr as f32).max(0.1),
                sound: e.sound_file().map(str::to_owned),
                volume: e.volume as f32,
            });
        }
    }

    // Texture decode, shared between groups.
    let mut tex_cache: HashMap<String, Option<Image>> = HashMap::new();
    let mut groups = Vec::new();
    let mut keys: Vec<GroupKey> = accs.keys().cloned().collect();
    keys.sort_by(|a, b| (&a.diffuse, &a.emissive).cmp(&(&b.diffuse, &b.emissive)));
    for key in keys {
        let acc = accs.remove(&key)?;
        if acc.idx.is_empty() {
            continue;
        }
        let mut load = |p: &Option<String>, max: u32| -> Option<Image> {
            let p = p.as_ref()?;
            tex_cache.entry(p.clone()).or_insert_with(|| load_texture(vfs, p, max)).clone()
        };
        let diffuse = load(&key.diffuse, 2048);
        let emissive = load(&key.emissive, 1024);
        debug!(
            "ship group: diffuse {:?} (decoded {}), emissive {:?}, tint {:?}, {} vertices",
            key.diffuse,
            diffuse.is_some(),
            key.emissive,
            acc.tint,
            acc.pos.len()
        );
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, acc.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, acc.nrm)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, acc.uv)
            .with_inserted_indices(Indices::U32(acc.idx));
        groups.push(LoadedGroup {
            mesh,
            diffuse,
            emissive,
            tint: acc.tint,
            emis: acc.emis,
            cutoff: if acc.cutoff { 0.5 } else { -1.0 },
        });
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
        let img = image_with_mips(vec![255; 8 * 4 * 4], 8, 4);
        assert_eq!(img.texture_descriptor.mip_level_count, 4);
        // 8x4 + 4x2 + 2x1 + 1x1 texels, 4 bytes each.
        assert_eq!(img.data.as_ref().unwrap().len(), (32 + 8 + 2 + 1) * 4);
    }

    #[test]
    fn plume_mesh_is_closed_teardrop() {
        let m = plume_mesh();
        assert!(m.count_vertices() > 100);
        let tip = m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap().iter().map(|p| p[2]).fold(0.0f32, f32::max);
        assert!((tip - 1.0).abs() < 1e-6);
    }
}
