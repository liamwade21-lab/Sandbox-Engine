//! Bodies drawn with 3D models (`Model "planets/Asteroids/67P.sss"` in catalogs)
//! instead of a terrain quadtree. Meshes are built on a worker thread the first
//! time the body is resolved on screen. SE draws them with its ship shader (the
//! spacecraft [`HullMaterial`]), lit by the body's stars, with the Lambert / Hapke mix and
//! opposition spots of the body's `Surface` (as FUN_14029f200 reads them from the lit body).

use std::sync::Arc;

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::catalog::ObjectRef;
use se_core::vfs::Vfs;

use crate::data::SeData;
use crate::models::{LoadedModel, build_model_meshes};
use crate::ship_assets::{HullDefaults, HullMaterial, HullUniform, surface_hapke};
use crate::system::{AmbientLighting, BodyLighting, SystemBody, SystemMember};
use crate::terrain::BodyAttached;
use crate::universe::{MeterScale, ScaleAnchor, UOrientation, UPosition, UniverseSet};

enum ModelState {
    Pending,
    Loading(Task<Option<LoadedModel>>),
    Ready(Vec<(Entity, Handle<HullMaterial>)>),
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

/// The body's brightest star and up to three more, as the hull material's four lights.
fn set_lights(u: &mut HullUniform, light: &BodyLighting, ambient: f32) {
    let (dirs, colors) = light.extra_uniforms(false);
    u.light_dir[0] = light.sun_dir.extend(if light.sun_intensity > 0.0 { 1.0 } else { 0.0 });
    u.light_color[0] = (light.sun_color * light.sun_intensity).extend(1.0);
    for i in 0..3 {
        u.light_dir[i + 1] = dirs[i];
        u.light_color[i + 1] = colors[i];
    }
    u.ambient.x = ambient;
}

#[allow(clippy::too_many_arguments)]
fn update_body_models(
    mut commands: Commands,
    data: Res<SeData>,
    defaults: Res<HullDefaults>,
    ambient: Res<AmbientLighting>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<HullMaterial>>,
    mut bodies: Query<(Entity, &SystemBody, &BodyLighting, &mut BodyModel)>,
    mut vis: Query<&mut Visibility, With<BodyAttached>>,
) {
    for (entity, body, light, mut model) in &mut bodies {
        let resolved = body.px_radius > 0.5;
        match &mut model.state {
            ModelState::Pending if resolved => {
                let vfs: Arc<Vfs> = data.vfs.clone();
                let r = model.model_ref.clone();
                let shift = defaults.thermal_shift;
                model.state =
                    ModelState::Loading(AsyncComputeTaskPool::get().spawn(async move { build_model_meshes(&vfs, &r, shift) }));
            }
            ModelState::Loading(task) => {
                if let Some(result) = check_ready(task) {
                    model.state = match result {
                        Some(loaded) => {
                            let surface = match body.obj {
                                ObjectRef::Body(i) => data.catalog.bodies.get(i as usize).and_then(|b| b.block.sub("Surface")),
                                _ => None,
                            };
                            let (lerp, hapke) = surface_hapke(surface);
                            let mut parts = Vec::new();
                            for p in loaded.parts {
                                let mut u = p.u;
                                (u.ambient.y, u.hapke) = (lerp, hapke);
                                set_lights(&mut u, light, ambient.0);
                                let mat = mats.add(p.tex.into_material(&mut images, &defaults, u));
                                let e = commands
                                    .spawn((
                                        SystemMember,
                                        BodyAttached {
                                            body: entity,
                                            center: DVec3::ZERO,
                                        },
                                        Mesh3d(meshes.add(p.mesh)),
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
                            set_lights(&mut m.u, light, ambient.0);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
