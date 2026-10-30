//! Catalog 3D models (`Model "...sss"` bodies such as asteroids and comets).
//!
//! [`build_model_meshes`] resolves a catalog model reference through
//! `se_core::model_script`, decodes the binary `.sm` mesh with `se_core::model_sm` and
//! turns each sub-mesh into a Bevy [`Mesh`] plus its `.sml` material maps. Mesh positions are
//! centred and scaled so the bounding radius is 1.0; the caller scales the model by the
//! body's radius in metres. SE draws these with its ship shader, so they use the spacecraft
//! [`HullMaterial`] (`ship_assets.rs`, `shaders/ship.wesl`) with the materials of
//! `textures/planets/Asteroids/*.sml`.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use se_core::model_script::{load_model_script, ModelScript};
use se_core::model_sm::parse_sm;
use se_core::ship::{DiffAlpha, ShipMaterial};
use se_core::vfs::Vfs;

use crate::ship_assets::{HullPart, add_hull_material, hull_uniform, load_hull_textures, object_tangents};

/// Registers the model material ([`crate::ship_assets::HullMaterial`]) with the renderer.
pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        add_hull_material(app);
    }
}

/// A decoded model, ready to be turned into entities.
pub struct LoadedModel {
    /// One entry per `.sm` sub-mesh: normalized mesh, its maps and material parameters.
    pub parts: Vec<HullPart>,
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
/// `thermal_shift` is SE's `ThermalEmissionShift` (models have no `RadTemp`, so unused today).
///
/// Returns `None` if the script, its `.cfg`, the `.sm` mesh cannot be found or decoded.
pub fn build_model_meshes(vfs: &Vfs, model_ref: &str, thermal_shift: f32) -> Option<LoadedModel> {
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

    // A material the `.sml` files do not define: SE's grey asteroid without maps.
    let fallback = ShipMaterial { diff_color: [0.14; 3], spec_color: [0.0; 3], diff_alpha: DiffAlpha::None, ..Default::default() };
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
        let tangents = object_tangents(&obj.tangents, &pos, &normals, &uvs, &idx);

        let mat = script.material_defs.get(obj.material_index as usize).and_then(|m| m.as_ref()).unwrap_or(&fallback);
        let tex = load_hull_textures(vfs, mat, 0.0, &mut textures);
        let u = hull_uniform(mat, &tex, 0.0, thermal_shift);

        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tangents)
            .with_inserted_indices(Indices::U32(idx));
        parts.push(HullPart { mesh, tex, u });
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
            let m = build_model_meshes(&vfs, r, 0.0).unwrap_or_else(|| panic!("failed to load {r}"));
            let verts: usize = m.parts.iter().map(|p| p.mesh.count_vertices()).sum();
            let tris: usize = m
                .parts
                .iter()
                .map(|p| match p.mesh.indices() {
                    Some(Indices::U32(i)) => i.len() / 3,
                    _ => 0,
                })
                .sum();
            let textured = m.parts.iter().filter(|p| p.tex.diffuse.is_some()).count();
            let flags: Vec<u32> = m.parts.iter().map(|p| p.u.flags.x).collect();
            println!(
                "{r}: parts={} verts={verts} tris={tris} textured={textured}/{} flags={flags:?} source_radius={:.4} mesh={:?} materials={:?}",
                m.parts.len(),
                m.parts.len(),
                m.source_radius,
                m.script.mesh_path,
                m.script.material_defs.iter().map(|d| d.as_ref().map(|d| d.name.clone())).collect::<Vec<_>>()
            );
            assert!(verts > 0 && tris > 0, "{r} has no geometry");
            assert!(m.parts.iter().all(|p| p.mesh.count_vertices() > 0));
        }
    }
}
