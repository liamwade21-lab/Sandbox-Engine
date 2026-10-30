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

fn update_body_models(
    mut commands: Commands,
    data: Res<SeData>,
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
                            for (mesh, image) in loaded.parts {
                                let texture = images.add(image.unwrap_or_else(crate::models::white_texture));
                                let mat = mats.add(ModelMaterial {
                                    u: ModelUniform {
                                        sun_dir: light.sun_dir.extend(light.sun_intensity),
                                        sun_color: light.sun_color.extend(1.0),
                                        base_color: Vec4::new(1.0, 1.0, 1.0, 0.02),
                                    },
                                    texture,
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
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
