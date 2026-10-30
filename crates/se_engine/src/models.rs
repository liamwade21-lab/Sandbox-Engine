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
use se_core::model_script::{load_model_script, ModelScript};
use se_core::model_sm::parse_sm;
use se_core::vfs::Vfs;

/// Uniform block of [`ModelMaterial`].
#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct ModelUniform {
    /// xyz: unit direction towards the star (render / world space); w: star intensity.
    pub sun_dir: Vec4,
    /// rgb: star colour.
    pub sun_color: Vec4,
    /// rgb: tint multiplied with the diffuse texture; w: ambient level.
    pub base_color: Vec4,
    /// The further stars ([`crate::system::BodyLighting::extra_uniforms`]).
    pub light_dir: [Vec4; crate::system::MAX_LIGHTS - 1],
    pub light_color: [Vec4; crate::system::MAX_LIGHTS - 1],
}

/// Lambert-lit model material with one diffuse texture.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ModelMaterial {
    #[uniform(0)]
    pub u: ModelUniform,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
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
    /// One entry per `.sm` sub-mesh: normalized mesh and its diffuse texture, if any.
    /// Textures that cannot be decoded (DDS, missing files) are `None`; use [`white_texture`].
    pub parts: Vec<(Mesh, Option<Image>)>,
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

/// Decodes a diffuse texture (PNG/JPEG via the `image` crate), halving oversized images.
fn load_diffuse(vfs: &Vfs, path: &str) -> Option<Image> {
    let bytes = vfs.read(path).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    while img.width() > 4096 || img.height() > 4096 {
        let (w, h) = (img.width() / 2, img.height() / 2);
        img = image::imageops::resize(&img, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
    }
    let (w, h) = img.dimensions();
    let mut image = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        img.into_raw(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    Some(image)
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

    let mut textures: HashMap<String, Option<Image>> = HashMap::new();
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

        let tex_path = script.diffuse_for_material(obj.material_index as usize).map(str::to_owned);
        let image = tex_path.map(|p| {
            textures
                .entry(p.clone())
                .or_insert_with(|| load_diffuse(vfs, &p))
                .clone()
        });
        let image = image.flatten();

        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(idx));
        parts.push((mesh, image));
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
            let textured = m.parts.iter().filter(|(_, img)| img.is_some()).count();
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
