//! Bodies drawn with 3D models (`Model "planets/Asteroids/67P.sss"` in catalogs)
//! instead of a terrain quadtree. Meshes are built on a worker thread the first
//! time the body is resolved on screen.

use std::sync::Arc;

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::vfs::Vfs;

use crate::data::SeData;
use crate::models::{LoadedModel, ModelMaterial, ModelUniform, build_model_meshes};
use crate::system::{BodyLighting, SystemBody, SystemMember};
use crate::terrain::BodyAttached;
use crate::universe::{MeterScale, ScaleAnchor, UOrientation, UPosition, UniverseSet};

enum ModelState {
    Pending,
    Loading(Task<Option<LoadedModel>>),
    Ready(Vec<(Entity, Handle<ModelMaterial>)>),
    Failed,
}

#[derive(Component)]
pub struct BodyModel {
    pub model_ref: String,
    state: ModelState,
}

impl BodyModel {
    pub fn new(model_ref: &str) -> Self {
        Self {
            model_ref: model_ref.to_string(),
            state: ModelState::Pending,
        }
    }
}

pub struct BodyModelsPlugin;

impl Plugin for BodyModelsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            update_body_models.after(UniverseSet::Camera).before(UniverseSet::FloatingOrigin),
        );
    }
}

/// SE's Hapke settings of a catalog body's `Surface` (as the terrain reads them): the
/// Hapke-to-Lambert mix (`Hapke` or `Lommel`, 0 Lambert) and `HapkeParams` (SpotBright, 1 / SpotWidth,
/// SpotBrightCB, 1 / SpotWidthCB).
fn body_hapke(data: &SeData, body: &SystemBody) -> (f32, Vec4) {
    let se_core::catalog::ObjectRef::Body(i) = body.obj else {
        return (0.0, Vec4::ZERO);
    };
    let s = data.catalog.bodies[i as usize].block.sub("Surface");
    let f = |k: &str| s.and_then(|s| s.f64(k)).unwrap_or(0.0) as f32;
    let inv = |w: f32| if w > 0.0 { 1.0 / w } else { 0.0 };
    let (bw, cw) = (f("SpotWidth"), f("SpotWidthCB"));
    let mix = s.and_then(|s| s.f64("Hapke").or_else(|| s.f64("Lommel"))).unwrap_or(0.0) as f32;
    (mix, Vec4::new(if bw > 0.0 { f("SpotBright") } else { 0.0 }, inv(bw), if cw > 0.0 { f("SpotBrightCB") } else { 0.0 }, inv(cw)))
}

fn update_body_models(
    mut commands: Commands,
    data: Res<SeData>,
    ambient: Res<crate::system::AmbientLighting>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ModelMaterial>>,
    mut bodies: Query<(Entity, &SystemBody, &BodyLighting, &mut BodyModel)>,
    mut vis: Query<&mut Visibility, With<BodyAttached>>,
) {
    for (entity, body, light, mut model) in &mut bodies {
        let resolved = body.px_radius > 0.5;
        match &mut model.state {
            ModelState::Pending if resolved => {
                let vfs: Arc<Vfs> = data.vfs.clone();
                let r = model.model_ref.clone();
                model.state =
                    ModelState::Loading(AsyncComputeTaskPool::get().spawn(async move { build_model_meshes(&vfs, &r) }));
            }
            ModelState::Loading(task) => {
                if let Some(result) = check_ready(task) {
                    model.state = match result {
                        Some(loaded) => {
                            let mut parts = Vec::new();
                            let (hapke_mix, hapke) = body_hapke(&data, body);
                            for (mesh, maps) in loaded.parts {
                                let has_normal = maps.normal.is_some();
                                let mut add = |img: Option<Image>, fallback: fn() -> Image| images.add(img.unwrap_or_else(fallback));
                                let mat = mats.add(ModelMaterial {
                                    u: ModelUniform {
                                        sun_dir: light.sun_dir.extend(light.sun_intensity),
                                        sun_color: light.sun_color.extend(1.0),
                                        base_color: maps.diff_scale.extend(ambient.0),
                                        light_dir: light.extra_uniforms(false).0,
                                        light_color: light.extra_uniforms(false).1,
                                        spec: maps.spec,
                                        detail: maps.detail,
                                        hapke,
                                        params: Vec4::new(hapke_mix, if has_normal { 1.0 } else { 0.0 }, 0.0, 0.0),
                                    },
                                    texture: add(maps.diffuse, crate::models::white_texture),
                                    normal: add(maps.normal, crate::models::flat_normal_texture),
                                    det_diff: add(maps.det_diff, crate::models::white_texture),
                                    det_norm: add(maps.det_norm, crate::models::flat_normal_texture),
                                });
                                let e = commands
                                    .spawn((
                                        SystemMember,
                                        BodyAttached {
                                            body: entity,
                                            center: DVec3::ZERO,
                                        },
                                        Mesh3d(meshes.add(mesh)),
                                        MeshMaterial3d(mat.clone()),
                                        UPosition(body.pos),
                                        UOrientation(body.orient),
                                        ScaleAnchor(body.pos),
                                        MeterScale(DVec3::splat(body.radius_m)),
                                        Transform::IDENTITY,
                                        Visibility::Hidden,
                                    ))
                                    .id();
                                parts.push((e, mat));
                            }
                            info!("loaded model {} ({} parts)", model.model_ref, parts.len());
                            ModelState::Ready(parts)
                        }
                        None => {
                            warn!("failed to load model {}", model.model_ref);
                            ModelState::Failed
                        }
                    };
                }
            }
            ModelState::Ready(parts) => {
                for (e, mat) in parts.iter() {
                    if let Ok(mut v) = vis.get_mut(*e) {
                        let want = if resolved { Visibility::Visible } else { Visibility::Hidden };
                        if *v != want {
                            *v = want;
                        }
                    }
                    if resolved {
                        if let Some(mut m) = mats.get_mut(mat) {
                            m.u.sun_dir = light.sun_dir.extend(light.sun_intensity);
                            m.u.sun_color = light.sun_color.extend(1.0);
                            (m.u.light_dir, m.u.light_color) = light.extra_uniforms(false);
                            m.u.base_color.w = ambient.0;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
