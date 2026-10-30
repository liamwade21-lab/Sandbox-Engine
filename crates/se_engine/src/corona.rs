//! Stellar coronae: a soft, streamered glow around each resolved star of the
//! active system (see `corona.wesl`).

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use se_core::catalog::ObjectRef;

use crate::data::SeData;
use crate::stars::star_render_color;
use crate::system::{StarSphere, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, UOrientation, UPosition, UniverseSet};

/// Corona extent in star radii.
const EXTENT: f32 = 7.0;

pub struct CoronaPlugin;

impl Plugin for CoronaPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CoronaMaterial>::default()).add_systems(
            PostUpdate,
            (spawn_coronas, update_coronas)
                .chain()
                .after(UniverseSet::Camera)
                .before(UniverseSet::FloatingOrigin),
        );
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct CoronaUniform {
    /// rgb colour, w intensity at the limb.
    pub color: Vec4,
    /// x: extent (star radii), y: seed, z: streamer strength.
    pub params: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CoronaMaterial {
    #[uniform(0)]
    pub u: CoronaUniform,
}

impl Material for CoronaMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/corona.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/corona.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // Sort after the atmosphere sky shells (which blend with alpha 1) and the
        // galaxy layers, like the galaxy renderer does; depth still hides it behind bodies.
        3.5e15
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Only the far half of the shell, so each pixel is drawn once (the camera may be inside).
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

/// The corona shell of a star entity.
#[derive(Component)]
struct Corona {
    star: Entity,
}

/// Marks stars that already have a corona.
#[derive(Component)]
struct HasCorona;

fn spawn_coronas(
    mut commands: Commands,
    data: Res<SeData>,
    stars: Query<(Entity, &SystemBody), (With<StarSphere>, Without<HasCorona>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<CoronaMaterial>>,
) {
    for (entity, body) in &stars {
        commands.entity(entity).insert(HasCorona);
        let ObjectRef::Star(i) = body.obj else { continue };
        let s = &data.catalog.stars[i as usize];
        let rgb = star_render_color(s.class.kind, s.teff * 0.9);
        let seed = (se_core::proc_surface::name_seed(&s.names[0]) % 1000) as f32 / 1000.0;
        let material = mats.add(CoronaMaterial {
            u: CoronaUniform {
                color: Vec4::new(rgb[0], rgb[1], rgb[2], 0.7),
                params: Vec4::new(EXTENT, seed, 0.8, 0.0),
            },
        });
        commands.spawn((
            SystemMember,
            Corona { star: entity },
            Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(48, 24))),
            MeshMaterial3d(material),
            UPosition(body.pos),
            UOrientation(body.orient),
            ScaleAnchor(body.pos),
            MeterScale(DVec3::splat(body.radius_m)),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
        ));
    }
}

fn update_coronas(
    bodies: Query<&SystemBody>,
    mut coronas: Query<(&Corona, &mut UPosition, &mut ScaleAnchor, &mut Visibility)>,
) {
    for (corona, mut pos, mut anchor, mut vis) in &mut coronas {
        let Ok(b) = bodies.get(corona.star) else { continue };
        // Below a couple of pixels the star sprite's glare does this job.
        let show = b.px_radius > 2.0;
        let v = if show { Visibility::Visible } else { Visibility::Hidden };
        if *vis != v {
            *vis = v;
        }
        if show {
            pos.0 = b.pos;
            anchor.0 = b.pos;
        }
    }
}
