//! Catalog 3D models (`Model "...sss"` bodies such as asteroids and comets).
//!
//! [`build_model_meshes`] resolves a catalog model reference through
//! `se_core::model_script`, decodes the binary `.sm` mesh with `se_core::model_sm` and
//! turns each sub-mesh into a Bevy [`Mesh`] plus its diffuse texture. Mesh positions are
//! centred and scaled so the bounding radius is 1.0; the caller scales the model by the
//! body's radius in metres. Rendering uses [`ModelMaterial`] (`assets/shaders/model.wesl`).

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use se_core::model_script::{load_model_script, MaterialPreset, ModelScript};
use se_core::model_sm::parse_sm;
use se_core::vfs::Vfs;

/// Uniform block of [`ModelMaterial`] (SE's ship.glsl uniforms for catalog body models).
#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct ModelUniform {
    /// xyz: unit direction towards the star (render / world space); w: star intensity.
    pub sun_dir: Vec4,
    /// rgb: star colour.
    pub sun_color: Vec4,
    /// rgb: SE's DiffColorScale (times the diffuse map); w: SE's AmbientParams.x, the ambient level.
    pub base_color: Vec4,
    /// The further stars ([`crate::system::BodyLighting::extra_uniforms`]).
    pub light_dir: [Vec4; crate::system::MAX_LIGHTS - 1],
    pub light_color: [Vec4; crate::system::MAX_LIGHTS - 1],
    /// rgb: SE's SpecColorScale (`SpecColor`), w: `SpecPower` (0: no highlight).
    pub spec: Vec4,
    /// SE's DetailScale: detail diffuse tiling, its mix, detail bump tiling, its mix (0: none).
    pub detail: Vec4,
    /// SE's HapkeParams: SpotBright, 1 / SpotWidth, SpotBrightCB, 1 / SpotWidthCB.
    pub hapke: Vec4,
    /// x: SE's AmbientParams.w, the Hapke-to-Lambert mix (the body's `Hapke`); y: 1 with a normal map.
    pub params: Vec4,
}

/// SE's model material for catalog bodies (ship.glsl): diffuse map tinted by DiffColorScale, the
/// tangent-space normal map, the two-scale detail diffuse and normal maps, Lambert mixed with the
/// Hapke opposition surge, and a Blinn-Phong highlight.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ModelMaterial {
    #[uniform(0)]
    pub u: ModelUniform,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    /// Tangent-space normals (xy; z rebuilt), flat when the material has none.
    #[texture(3)]
    #[sampler(4)]
    pub normal: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub det_diff: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    pub det_norm: Handle<Image>,
}

/// One part's textures and material constants, as built off the main thread.
pub struct ModelPartMaps {
    pub diffuse: Option<Image>,
    pub normal: Option<Image>,
    pub det_diff: Option<Image>,
    pub det_norm: Option<Image>,
    /// [`ModelUniform`] `base_color.rgb`, `spec`, `detail`.
    pub diff_scale: Vec3,
    pub spec: Vec4,
    pub detail: Vec4,
}

impl ModelPartMaps {
    /// SE's material constants from the `.sml` preset: `DiffColorScale`, else `DiffColor` (the
    /// colour of an unmapped material, the tint of a mapped one); `SpecColor` / `SpecPower`; the
    /// detail tilings and mixes (a detail map that is missing mixes nothing).
    pub fn constants(p: Option<&MaterialPreset>) -> (Vec3, Vec4, Vec4) {
        let v3 = |c: [f64; 3]| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32);
        let Some(p) = p else {
            return (Vec3::ONE, Vec4::ZERO, Vec4::ZERO);
        };
        let diff = p.diff_color_scale.or(p.diff_color).map_or(Vec3::ONE, v3);
        let spec = p.spec_color.map_or(Vec3::ZERO, v3).extend(p.spec_power.unwrap_or(0.0) as f32);
        let f = |v: Option<f64>, d: f64| v.unwrap_or(d) as f32;
        let detail = Vec4::new(
            f(p.det_diff_scale, 1.0),
            if p.det_diff_map.is_some() { f(p.det_diff_mag, 1.0) } else { 0.0 },
            f(p.det_bump_scale, 1.0),
            if p.det_bump_map.is_some() { f(p.det_bump_mag, 1.0) } else { 0.0 },
        );
        (diff, spec, detail)
    }
}

impl Material for ModelMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/model.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/model.wesl".into()
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
        Ok(())
    }
}

/// Registers [`ModelMaterial`] with the renderer.
pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ModelMaterial>::default());
    }
}

/// A decoded model, ready to be turned into entities.
pub struct LoadedModel {
    /// One entry per `.sm` sub-mesh: normalized mesh and its material's maps. Maps that are not
    /// set or cannot be decoded are `None`; use [`white_texture`] / [`flat_normal_texture`].
    pub parts: Vec<(Mesh, ModelPartMaps)>,
    /// Always 1.0: positions are centred on the bounding-sphere centre and scaled so
    /// the farthest vertex is at this distance.
    #[allow(dead_code)] // Parsed metadata kept for callers; not read yet.
    pub bounding_radius: f32,
    /// Bounding radius of the original `.sm` data, in its model units.
    #[allow(dead_code)] // Parsed metadata kept for callers; not read yet.
    pub source_radius: f32,
    /// Resolved script data (paths, orientation, placement) for the caller.
    #[allow(dead_code)] // Resolved for callers; not read yet.
    pub script: ModelScript,
}

/// 1x1 opaque white texture, used when a model part has no decodable diffuse map.
pub fn white_texture() -> Image {
    Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[255, 255, 255, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// 1x1 flat tangent-space normal (linear), for parts without a normal map.
pub fn flat_normal_texture() -> Image {
    Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[128, 128, 255, 255],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn repeat_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    })
}

/// A block-compressed DDS uploaded as stored, its whole mip chain included: DXT1/3/5 (BC1-3) and
/// ATI1/ATI2 (BC4/BC5, SE's compressed normal maps). `None` for other layouts.
fn dds_image(bytes: &[u8], srgb: bool) -> Option<Image> {
    let u = |o: usize| bytes.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    if bytes.get(0..4)? != b"DDS " {
        return None;
    }
    let (h, w, mips) = (u(12)?, u(16)?, u(28)?.max(1));
    let (format, block) = match bytes.get(84..88)? {
        b"DXT1" => (if srgb { TextureFormat::Bc1RgbaUnormSrgb } else { TextureFormat::Bc1RgbaUnorm }, 8),
        b"DXT3" => (if srgb { TextureFormat::Bc2RgbaUnormSrgb } else { TextureFormat::Bc2RgbaUnorm }, 16),
        b"DXT5" => (if srgb { TextureFormat::Bc3RgbaUnormSrgb } else { TextureFormat::Bc3RgbaUnorm }, 16),
        b"ATI1" | b"BC4U" => (TextureFormat::Bc4RUnorm, 8),
        b"ATI2" | b"BC5U" => (TextureFormat::Bc5RgUnorm, 16),
        _ => return None,
    };
    if w % 4 != 0 || h % 4 != 0 {
        return None;
    }
    // Keep the levels down to 4 x 4 texels (one block a side); wgpu wants block-sized levels.
    let (mut size, mut levels, mut lw, mut lh) = (0usize, 0u32, w, h);
    while levels < mips && lw >= 4 && lh >= 4 {
        size += (lw / 4) as usize * (lh / 4) as usize * block;
        levels += 1;
        (lw, lh) = (lw / 2, lh / 2);
    }
    let data = bytes.get(128..128 + size)?.to_vec();
    let mut image = Image::new_uninit(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = repeat_sampler();
    Some(image)
}

/// RGBA8 with a box-filtered mip chain.
fn rgba_image(mut rgba: Vec<u8>, mut w: u32, mut h: u32, srgb: bool) -> Image {
    while w > 4096 || h > 4096 {
        rgba = crate::ship_assets::box_down(&rgba, w, h);
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
    }
    let mut data = rgba.clone();
    let (mut cw, mut ch, mut cur, mut levels) = (w, h, rgba, 1);
    while cw > 1 || ch > 1 {
        cur = crate::ship_assets::box_down(&cur, cw, ch);
        (cw, ch) = ((cw / 2).max(1), (ch / 2).max(1));
        data.extend_from_slice(&cur);
        levels += 1;
    }
    let mut image = Image::new_uninit(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = repeat_sampler();
    image
}

/// Decodes a colour (`srgb`) or normal map: block-compressed DDS as stored, PNG/JPEG through the
/// `image` crate.
fn load_map(vfs: &Vfs, path: &str, srgb: bool) -> Option<Image> {
    let bytes = vfs.read(path).ok()?;
    if path.to_ascii_lowercase().ends_with(".dds") {
        return dds_image(&bytes, srgb);
    }
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(rgba_image(img.into_raw(), w, h, srgb))
}

/// SE's bump_to_norm.glsl: tangent-space normals from a height map (`height` texture units per unit
/// of the map's value), as RGBA8 in SE's tangent frame: x along the map's columns (+u), y up the map
/// (-v, against its rows).
pub fn bump_to_normals(h: &[f32], w: usize, ht: usize, height: f32) -> Vec<u8> {
    let at = |x: isize, y: isize| h[(y.rem_euclid(ht as isize) as usize) * w + x.rem_euclid(w as isize) as usize] * height;
    let mut out = Vec::with_capacity(w * ht * 4);
    for y in 0..ht as isize {
        for x in 0..w as isize {
            let dx = (at(x + 1, y) - at(x - 1, y)) * 0.5;
            let dy = (at(x, y + 1) - at(x, y - 1)) * 0.5;
            let n = Vec3::new(-dx, dy, 1.0).normalize();
            out.extend(n.to_array().map(|c| ((c * 0.5 + 0.5) * 255.0).round() as u8));
            out.push(255);
        }
    }
    out
}

/// A bump (height) map turned into a normal map, `height` high (SE's BumpHeight / DetBumpHeight).
fn load_bump(vfs: &Vfs, path: &str, height: f32) -> Option<Image> {
    let img = image::load_from_memory(&vfs.read(path).ok()?).ok()?.to_luma16();
    let (w, h) = img.dimensions();
    let hs: Vec<f32> = img.pixels().map(|p| p.0[0] as f32 / 65535.0).collect();
    Some(rgba_image(bump_to_normals(&hs, w as usize, h as usize, height), w, h, false))
}


/// Normals for vertices whose stored normal is (near) zero: area-weighted face normals.
fn fill_zero_normals(pos: &[[f32; 3]], normals: &mut [[f32; 3]], idx: &[u32]) {
    if normals.iter().all(|n| Vec3::from(*n).length_squared() > 0.25) {
        return;
    }
    let mut acc = vec![Vec3::ZERO; pos.len()];
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (Vec3::from(pos[t[0] as usize]), Vec3::from(pos[t[1] as usize]), Vec3::from(pos[t[2] as usize]));
        let n = (b - a).cross(c - a);
        for &i in t {
            acc[i as usize] += n;
        }
    }
    for (n, a) in normals.iter_mut().zip(acc) {
        if Vec3::from(*n).length_squared() <= 0.25 {
            *n = a.try_normalize().unwrap_or(Vec3::Y).to_array();
        }
    }
}

/// Loads a catalog model (e.g. `planets/Asteroids/67P.sss`, relative to `models/`).
///
/// Returns `None` if the script, its `.cfg`, the `.sm` mesh cannot be found or decoded.
pub fn build_model_meshes(vfs: &Vfs, model_ref: &str) -> Option<LoadedModel> {
    let script = load_model_script(vfs, model_ref)?;
    let mesh_path = script.mesh_path.clone()?;
    let model = parse_sm(&vfs.read(&mesh_path).ok()?).ok()?;

    let (mn, mx) = model.bounds()?;
    let center = (Vec3::from(mn) + Vec3::from(mx)) * 0.5;
    let radius = model
        .objects
        .iter()
        .flat_map(|o| o.positions.iter())
        .map(|p| Vec3::from(*p).distance(center))
        .fold(0.0f32, f32::max);
    if radius <= 0.0 || !radius.is_finite() {
        return None;
    }
    let inv = 1.0 / radius;

    // Decoded maps by (path, kind), shared between parts.
    let mut textures: HashMap<(String, u8), Option<Image>> = HashMap::new();
    let mut map = |path: Option<&String>, kind: u8, height: f32| -> Option<Image> {
        let p = path?;
        textures
            .entry((p.clone(), kind))
            .or_insert_with(|| match kind {
                // Colour maps as stored, without the sRGB decode: SE's DiffGamma 1 (MapCorrections.x)
                // leaves them so, and their DiffColor tints are matched to that (Eros 0.56 x 0.4,
                // 67P 0.43 x 0.1: the bodies' albedos).
                0 | 1 => load_map(vfs, p, false),
                _ => load_bump(vfs, p, height),
            })
            .clone()
    };
    let mut parts = Vec::with_capacity(model.objects.len());
    for obj in &model.objects {
        let idx = obj.triangle_indices();
        if obj.positions.is_empty() || idx.is_empty() {
            continue;
        }
        let pos: Vec<[f32; 3]> = obj.positions.iter().map(|p| ((Vec3::from(*p) - center) * inv).to_array()).collect();
        let mut normals = if obj.normals.len() == pos.len() {
            obj.normals.clone()
        } else {
            vec![[0.0; 3]; pos.len()]
        };
        fill_zero_normals(&pos, &mut normals, &idx);
        let uvs = if obj.uvs.len() == pos.len() { obj.uvs.clone() } else { vec![[0.0; 2]; pos.len()] };

        let preset = script.preset_for_material(obj.material_index as usize);
        let (diff_scale, spec, detail) = ModelPartMaps::constants(preset);
        let maps = ModelPartMaps {
            diffuse: map(preset.and_then(|p| p.diff_map.as_ref()), 0, 0.0),
            normal: match preset {
                Some(p) if p.norm_map.is_some() => map(p.norm_map.as_ref(), 1, 0.0),
                Some(p) => map(p.bump_map.as_ref(), 2, p.bump_height.unwrap_or(1.0) as f32),
                None => None,
            },
            det_diff: map(preset.and_then(|p| p.det_diff_map.as_ref()), 0, 0.0),
            det_norm: preset.and_then(|p| map(p.det_bump_map.as_ref(), 2, p.det_bump_height.unwrap_or(1.0) as f32)),
            diff_scale,
            spec,
            detail,
        };

        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(idx));
        parts.push((mesh, maps));
    }
    if parts.is_empty() {
        return None;
    }
    Some(LoadedModel { parts, bounding_radius: 1.0, source_radius: radius, script })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn material_constants_follow_se_keys() {
        let p = MaterialPreset {
            diff_color: Some([0.4, 0.4, 0.4]),
            spec_power: Some(60.0),
            det_diff_map: Some("d.png".into()),
            det_diff_scale: Some(30.0),
            det_diff_mag: Some(0.5),
            det_bump_scale: Some(30.0),
            det_bump_mag: Some(0.5),
            ..Default::default()
        };
        let (diff, spec, detail) = ModelPartMaps::constants(Some(&p));
        assert_eq!(diff, Vec3::splat(0.4));
        assert_eq!(spec, Vec4::new(0.0, 0.0, 0.0, 60.0));
        // No DetBumpMap: its mix is 0 whatever DetBumpMag says.
        assert_eq!(detail, Vec4::new(30.0, 0.5, 30.0, 0.0));
        // DiffColorScale wins over DiffColor (67P).
        let q = MaterialPreset { diff_color: Some([0.4; 3]), diff_color_scale: Some([0.1; 3]), ..Default::default() };
        assert_eq!(ModelPartMaps::constants(Some(&q)).0, Vec3::splat(0.1));
        assert_eq!(ModelPartMaps::constants(None).0, Vec3::ONE);
    }

    #[test]
    fn bump_to_normals_tilts_against_the_slope() {
        // A ramp rising along x: the normal leans to -x, y flat.
        let h: Vec<f32> = (0..16).map(|i| (i % 4) as f32 * 0.25).collect();
        let n = bump_to_normals(&h, 4, 4, 1.0);
        let px = |i: usize| (n[i * 4] as f32 / 127.5 - 1.0, n[i * 4 + 1] as f32 / 127.5 - 1.0);
        let (x, y) = px(5);
        assert!(x < -0.1 && y.abs() < 0.01, "{x} {y}");
        // Rising down the rows (+v): SE's y runs up the map (its stored tangents' binormal is -v),
        // so the normal leans to +y.
        let h: Vec<f32> = (0..16).map(|i| (i / 4) as f32 * 0.25).collect();
        let n = bump_to_normals(&h, 4, 4, 1.0);
        let (x, y) = (n[20] as f32 / 127.5 - 1.0, n[21] as f32 / 127.5 - 1.0);
        assert!(y > 0.1 && x.abs() < 0.01, "{x} {y}");
    }

    #[test]
    fn dds_bc5_keeps_block_sized_levels() {
        // An 8x8 ATI2 file with 4 levels stored: only 8x8 and 4x4 are kept.
        let mut b = vec![0u8; 128];
        b[0..4].copy_from_slice(b"DDS ");
        b[12..16].copy_from_slice(&8u32.to_le_bytes());
        b[16..20].copy_from_slice(&8u32.to_le_bytes());
        b[28..32].copy_from_slice(&4u32.to_le_bytes());
        b[84..88].copy_from_slice(b"ATI2");
        b.extend(vec![0u8; 4 * 16 + 16 + 16 + 16]);
        let img = dds_image(&b, false).unwrap();
        assert_eq!(img.texture_descriptor.format, TextureFormat::Bc5RgUnorm);
        assert_eq!(img.texture_descriptor.mip_level_count, 2);
        assert_eq!(img.data.as_ref().unwrap().len(), 4 * 16 + 16);
    }

    /// Mean stored value of the asteroids' PNG/JPEG albedo maps (gamma or linear?). Run with
    /// --ignored --nocapture.
    #[test]
    #[ignore]
    fn albedo_map_means() {
        let vfs = Vfs::mount_install(Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("mount install");
        for p in ["Eros_albedo.png", "Itokawa_albedo.png", "2014MU69_albedo.jpg"] {
            let path = format!("textures/planets/asteroids/{p}");
            let img = image::load_from_memory(&vfs.read(&path).unwrap()).unwrap().to_rgb8();
            let n = (img.width() * img.height()) as f64;
            let m = img.pixels().fold(0.0, |a, p| a + p.0[1] as f64) / n / 255.0;
            println!("{p}: mean green {m:.3}");
        }
    }

    /// Loads every asteroid model from the real install and checks the counts.
    /// Run with: cargo test -p se_engine asteroid_models -- --ignored --nocapture
    #[test]
    #[ignore]
    fn asteroid_models_load_from_install() {
        let vfs = Vfs::mount_install(Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("mount install");
        let refs = [
            "planets/Asteroids/2014MU69.sss",
            "planets/Asteroids/67P.sss",
            "planets/Asteroids/Eros.sss",
            "planets/Asteroids/Itokawa.sss",
            "planets/Asteroids/Oumuamua.sss",
        ];
        for r in refs {
            let m = build_model_meshes(&vfs, r).unwrap_or_else(|| panic!("failed to load {r}"));
            let verts: usize = m.parts.iter().map(|(mesh, _)| mesh.count_vertices()).sum();
            let tris: usize = m
                .parts
                .iter()
                .map(|(mesh, _)| match mesh.indices() {
                    Some(Indices::U32(i)) => i.len() / 3,
                    _ => 0,
                })
                .sum();
            let textured = m.parts.iter().filter(|(_, maps)| maps.diffuse.is_some()).count();
            let normals = m.parts.iter().filter(|(_, maps)| maps.normal.is_some()).count();
            println!("{r}: normal maps {normals}, detail {:?}", m.parts.first().map(|(_, maps)| maps.detail));
            println!(
                "{r}: parts={} verts={verts} tris={tris} textured={textured}/{} source_radius={:.4} mesh={:?} diffuse={:?}",
                m.parts.len(),
                m.parts.len(),
                m.source_radius,
                m.script.mesh_path,
                m.script.textures.first()
            );
            assert!(verts > 0 && tris > 0, "{r} has no geometry");
            assert!(m.parts.iter().all(|(mesh, _)| mesh.count_vertices() > 0));
        }
    }
}
