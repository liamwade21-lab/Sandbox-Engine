//! Megaton Rainfall's assets (its trees, grass, building parts, roads, props and cars), read at run
//! time from the user's copy of the game (`EngineConfig::mr_path`, `--mr-path` / `MR_PATH`), never
//! copied into the project. The formats are read by `se_core::mr`; this turns them into Bevy
//! textures and meshes. Without the install everything still runs: the vegetation and cities then
//! use generated trees and code-built buildings only (no cars), and se.log says so once.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use se_core::dds::{DdsFormat, DdsInfo, decode_level};
use se_core::mr::{MrInstall, MrModel, MrSubMesh};

use crate::config::EngineConfig;

pub struct MrAssetsPlugin;

impl Plugin for MrAssetsPlugin {
    fn build(&self, app: &mut App) {
        // The copy of the files the port uses, in its own assets (the user's choice of 2026-11-05:
        // the port must not need the game; made by `scatter::library`'s ignored test
        // `copy_megaton_rainfall`); the user's install only when the copy is missing.
        let copy = [crate::config::asset_dir().join("mr"), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("mr")]
            .into_iter()
            .find(|d| d.join("Data").join("global.txt").is_file());
        let path = copy.clone().unwrap_or_else(|| app.world().resource::<EngineConfig>().mr_path.clone());
        let t0 = std::time::Instant::now();
        let opened = match &copy {
            Some(dir) => MrInstall::open_copy(dir),
            None => MrInstall::open(&path),
        };
        let install = match opened {
            Ok(mr) => {
                info!(
                    "Megaton Rainfall assets: {} ({} models, {} materials, opened in {:.0} ms)",
                    path.display(),
                    mr.model_names().len(),
                    mr.material_count(),
                    t0.elapsed().as_secs_f64() * 1e3
                );
                Some(Arc::new(mr))
            }
            Err(e) => {
                warn!("{e}: no vegetation or buildings (the port's copy, assets/mr, is missing too)");
                None
            }
        };
        app.insert_resource(MrAssets { install, images: HashMap::new() });
    }
}

/// The opened install (if any) and the textures already uploaded from it.
#[derive(Resource)]
pub struct MrAssets {
    pub install: Option<Arc<MrInstall>>,
    images: HashMap<(String, bool), Handle<Image>>,
}

impl MrAssets {
    /// The texture of that name (as MR's materials write it), uploaded once and kept. `srgb` for
    /// colour textures; normal maps, masks and noise are linear. A texture the install lacks (or
    /// no install) gives `None`.
    pub fn image(&mut self, images: &mut Assets<Image>, name: &str, srgb: bool) -> Option<Handle<Image>> {
        let key = (name.to_ascii_lowercase(), srgb);
        if let Some(h) = self.images.get(&key) {
            return Some(h.clone());
        }
        let mr = self.install.as_ref()?;
        let image = match mr.texture(name).map_err(|e| e.to_string()).and_then(|(b, info)| dds_image(&b, &info, srgb)) {
            Ok(i) => i,
            Err(e) => {
                warn!("Megaton Rainfall texture {name}: {e}");
                return None;
            }
        };
        let h = images.add(image);
        self.images.insert(key, h.clone());
        Some(h)
    }
}

/// The GPU format a DDS format uploads as without conversion, if any.
fn gpu_format(f: DdsFormat, srgb: bool) -> Option<TextureFormat> {
    Some(match (f, srgb) {
        (DdsFormat::Bc1, false) => TextureFormat::Bc1RgbaUnorm,
        (DdsFormat::Bc1, true) => TextureFormat::Bc1RgbaUnormSrgb,
        (DdsFormat::Bc2, false) => TextureFormat::Bc2RgbaUnorm,
        (DdsFormat::Bc2, true) => TextureFormat::Bc2RgbaUnormSrgb,
        (DdsFormat::Bc3, false) => TextureFormat::Bc3RgbaUnorm,
        (DdsFormat::Bc3, true) => TextureFormat::Bc3RgbaUnormSrgb,
        (DdsFormat::Bc4, _) => TextureFormat::Bc4RUnorm,
        (DdsFormat::Bc5, _) => TextureFormat::Bc5RgUnorm,
        (DdsFormat::Rgba8, false) => TextureFormat::Rgba8Unorm,
        (DdsFormat::Rgba8, true) => TextureFormat::Rgba8UnormSrgb,
        (DdsFormat::Rgba8Snorm, _) => TextureFormat::Rgba8Snorm,
        (DdsFormat::R8, _) => TextureFormat::R8Unorm,
        (DdsFormat::Rg8, _) => TextureFormat::Rg8Unorm,
        (DdsFormat::Rg8Snorm, _) => TextureFormat::Rg8Snorm,
        (DdsFormat::R32F, _) => TextureFormat::R32Float,
        (DdsFormat::Rgba32F, _) => TextureFormat::Rgba32Float,
        // 32-bit BGRA is swizzled to RGBA (and X8 made opaque) on the CPU.
        (DdsFormat::Bgra8 | DdsFormat::Bgrx8, _) => return None,
    })
}

/// A Bevy image of a whole DDS file: every layer and mip, block-compressed formats uploaded as
/// they are (the GPU decodes them). Uncompressed BGRA, and block formats whose size is not a
/// multiple of 4 (wgpu refuses those), are decoded to RGBA8 here. Repeating, trilinear and 16x
/// anisotropic sampling (bark, facades and roads tile).
pub fn dds_image(bytes: &[u8], info: &DdsInfo, srgb: bool) -> Result<Image, String> {
    let aligned = !info.format.is_block() || (info.width % 4 == 0 && info.height % 4 == 0);
    let (format, data) = match gpu_format(info.format, srgb).filter(|_| aligned) {
        Some(f) => (f, bytes[info.data_offset..info.data_offset + info.data_bytes()].to_vec()),
        None => {
            let mut data = Vec::new();
            for layer in 0..info.layers {
                for mip in 0..info.mips {
                    data.extend(decode_level(bytes, info, layer, mip).map_err(|e| e.to_string())?);
                }
            }
            (if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm }, data)
        }
    };
    let mut image = Image::new_uninit(
        Extent3d { width: info.width, height: info.height, depth_or_array_layers: info.layers },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = info.mips;
    let dimension = if info.cube {
        if info.layers > 6 { TextureViewDimension::CubeArray } else { TextureViewDimension::Cube }
    } else if info.layers > 1 {
        TextureViewDimension::D2Array
    } else {
        TextureViewDimension::D2
    };
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(dimension), ..default() });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    Ok(image)
}

/// One sub-mesh of a model as a Bevy mesh (its own vertices, indices made relative to them):
/// position, normal, UV0 and, where the model has them, tangent (w = 1) and colour (the raw
/// bytes as 0..1: MR keeps data there, e.g. the facade's window type in blue). Vertices beyond
/// a stream's length get an up normal, UV 0 and white.
#[allow(dead_code)] // Props drawn as Bevy meshes (the cities' street furniture, later steps).
pub fn submesh_mesh(model: &MrModel, sub: &MrSubMesh) -> Mesh {
    let range = sub.first_vertex as usize..(sub.first_vertex + sub.vertex_count) as usize;
    let pos: Vec<[f32; 3]> = model.positions[range.clone()].to_vec();
    let normals: Vec<[f32; 3]> = range.clone().map(|i| model.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0])).collect();
    let uvs: Vec<[f32; 2]> = range.clone().map(|i| model.uvs.get(i).copied().unwrap_or([0.0; 2])).collect();
    let idx: Vec<u32> = model.indices[sub.first_index as usize..(sub.first_index + sub.index_count) as usize]
        .iter()
        .map(|&i| i - sub.first_vertex)
        .collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(idx));
    if model.tangents.len() >= range.end {
        let t: Vec<[f32; 4]> = range.clone().map(|i| {
            let t = model.tangents[i];
            [t[0], t[1], t[2], 1.0]
        })
        .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, t);
    }
    if !model.colors.is_empty() {
        let c: Vec<[f32; 4]> = range
            .map(|i| model.colors.get(i).map_or([1.0; 4], |c| c.map(|v| v as f32 / 255.0)))
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::mr::DEFAULT_MR_PATH;

    fn install() -> Option<MrInstall> {
        let root = std::env::var_os("MR_PATH").map(std::path::PathBuf::from).unwrap_or_else(|| DEFAULT_MR_PATH.into());
        MrInstall::open(&root).map_err(|e| eprintln!("skipping: {e}")).ok()
    }

    /// The textures the vegetation and cities use become images of the right size and format
    /// (with every layer's mip chain), and the trees become meshes.
    #[test]
    fn images_and_meshes() {
        let Some(mr) = install() else { return };
        let cases = [
            ("windows", true, TextureFormat::Bc3RgbaUnormSrgb, 18),
            ("windows_NG", false, TextureFormat::Bc3RgbaUnorm, 18),
            ("Trees\\Bark1A", true, TextureFormat::Bc2RgbaUnormSrgb, 1),
            ("Trees\\Branch1A", true, TextureFormat::Bc3RgbaUnormSrgb, 1),
            ("Billboard_Grass", true, TextureFormat::Bc3RgbaUnormSrgb, 1),
            ("Asphalt", true, TextureFormat::Bc1RgbaUnormSrgb, 1),
            ("bricks", true, TextureFormat::Rgba8UnormSrgb, 1),
            ("noise", false, TextureFormat::Rgba8Unorm, 1),
            ("NoiseTileableR8G8", false, TextureFormat::Rg8Snorm, 1),
            ("circle", true, TextureFormat::Rgba8UnormSrgb, 1),
        ];
        for (name, srgb, format, layers) in cases {
            let (bytes, info) = mr.texture(name).unwrap();
            let img = dds_image(&bytes, &info, srgb).unwrap();
            assert_eq!(img.texture_descriptor.format, format, "{name}");
            assert_eq!(img.texture_descriptor.size.depth_or_array_layers, layers, "{name}");
            assert_eq!(img.texture_descriptor.mip_level_count, info.mips, "{name}");
            // What Bevy will upload: every layer's whole chain, in its own size rules.
            let expect: usize = (0..info.mips)
                .map(|m| {
                    let size = img.texture_descriptor.size.mip_level_size(m, TextureDimension::D2).physical_size(format);
                    let (bw, bh) = format.block_dimensions();
                    (size.width / bw * size.height / bh) as usize * format.block_copy_size(None).unwrap() as usize
                })
                .sum::<usize>()
                * layers as usize;
            assert_eq!(img.data.as_ref().unwrap().len(), expect, "{name}");
        }
        let tree = mr.model("tree_0").unwrap();
        let bark = submesh_mesh(&tree, &tree.submeshes[0]);
        let leaves = submesh_mesh(&tree, &tree.submeshes[1]);
        assert_eq!(bark.count_vertices() + leaves.count_vertices(), 2437);
        assert_eq!(leaves.indices().unwrap().len(), 756);
        assert!(leaves.indices().unwrap().iter().all(|i| i < 754));
        assert!(bark.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
        let car = mr.model("vehicle_6").unwrap();
        assert!(submesh_mesh(&car, &car.submeshes[0]).attribute(Mesh::ATTRIBUTE_COLOR).is_some());
    }
}
