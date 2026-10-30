//! Auroras from the catalog `Aurora` blocks: for each oval, rings of ray curtains
//! around the magnetic pole plus a glowing base band (see `aurora.wesl`).

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use se_core::body_params::{AuroraOval, AuroraParams, BodyExtras};
use se_core::catalog::ObjectRef;

use crate::data::SeData;
use crate::system::{BodyLighting, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, UOrientation, UPosition, UniverseSet};

/// Segments around each ring.
const SEGMENTS: usize = 512;

pub struct AuroraPlugin;

impl Plugin for AuroraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<AuroraMaterial>::default()).add_systems(
            PostUpdate,
            (spawn_auroras, update_auroras)
                .chain()
                .after(UniverseSet::Camera)
                .before(UniverseSet::FloatingOrigin),
        );
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct AuroraUniform {
    /// xyz: unit direction to the star (render frame).
    pub sun_dir: Vec4,
    /// rgb: lower-edge colour, w: brightness.
    pub bottom: Vec4,
    /// rgb: multiplier for the upper edge.
    pub top: Vec4,
    /// x: flash frequency, y: drift speed, z: seed.
    pub params: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct AuroraMaterial {
    #[uniform(0)]
    pub u: AuroraUniform,
}

impl Material for AuroraMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/aurora.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/aurora.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // After the atmosphere sky shells (alpha 1) and the galaxy layers, as for coronae.
        3.6e15
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
            Mesh::ATTRIBUTE_UV_1.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// One auroral oval attached to a body.
#[derive(Component)]
struct AuroraOf {
    body: Entity,
    material: Handle<AuroraMaterial>,
}

/// Marks bodies whose aurora has been looked up.
#[derive(Component)]
struct AuroraChecked;

/// Curtains and base band of one oval, in the body frame (meters from the centre).
fn oval_mesh(radius_m: f64, a: &AuroraParams, o: &AuroraOval, seed: f64) -> Mesh {
    let (lat, lon) = (o.lat_deg.to_radians(), o.lon_deg.to_radians());
    // SE body frame: +X lon 0, +Y north, +Z lon 90 W.
    let pole = DVec3::new(lat.cos() * lon.cos(), lat.sin(), -lat.cos() * lon.sin());
    let t1 = pole.any_orthonormal_vector();
    let t2 = pole.cross(t1);
    let base = radius_m + a.height_km * 1000.0;
    let top = radius_m + a.height_km * 1000.0 * 2.5;
    let rings = o.rings.max(1);
    let r_km = radius_m / 1000.0;
    let band_half = (o.width_km / rings as f64 * 0.5).max(1.0) / r_km;

    let (mut pos, mut nrm, mut uv0, mut uv1, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut strip = |pts: &[(DVec3, f64, f32)], kind: f32, ring_f: f32, pos: &mut Vec<[f32; 3]>| {
        // pts: (unit direction, radius, second uv coordinate) pairs per segment (lower, upper).
        let start = pos.len() as u32;
        for (dir, r, v) in pts {
            pos.push((*dir * *r).as_vec3().to_array());
            nrm.push(dir.as_vec3().to_array());
            let i = (pos.len() as u32 - start - 1) / 2;
            uv0.push([i as f32 / SEGMENTS as f32, *v]);
            uv1.push([ring_f, kind]);
        }
        for i in 0..SEGMENTS as u32 {
            let a = start + i * 2;
            idx.extend_from_slice(&[a, a + 1, a + 2, a + 1, a + 3, a + 2]);
        }
    };
    for k in 0..rings {
        let ring_f = if rings > 1 { k as f64 / (rings - 1) as f64 } else { 0.5 };
        let theta0 = (o.radius_km + (ring_f - 0.5) * o.width_km * (rings > 1) as u8 as f64) / r_km;
        let ph = seed * 6.28 + k as f64 * 1.7;
        let dir_at = |phi: f64, dtheta: f64| {
            // Folded, slightly irregular oval.
            let wiggle = 1.0 + 0.05 * (3.0 * phi + ph).sin() + 0.025 * (7.0 * phi + 2.0 * ph).sin() + 0.012 * (17.0 * phi + ph).sin();
            let theta = theta0 * wiggle + dtheta;
            pole * theta.cos() + (t1 * phi.cos() + t2 * phi.sin()) * theta.sin()
        };
        let mut curtain = Vec::with_capacity((SEGMENTS + 1) * 2);
        let mut band = Vec::with_capacity((SEGMENTS + 1) * 2);
        for i in 0..=SEGMENTS {
            let phi = std::f64::consts::TAU * i as f64 / SEGMENTS as f64;
            let d = dir_at(phi, 0.0);
            curtain.push((d, base, 0.0));
            curtain.push((d, top, 1.0));
            band.push((dir_at(phi, -band_half), base, -1.0));
            band.push((dir_at(phi, band_half), base, 1.0));
        }
        strip(&curtain, 0.0, ring_f as f32, &mut pos);
        strip(&band, 1.0, ring_f as f32, &mut pos);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv0)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uv1)
        .with_inserted_indices(Indices::U32(idx))
}

fn spawn_auroras(
    mut commands: Commands,
    data: Res<SeData>,
    bodies: Query<(Entity, &SystemBody), Without<AuroraChecked>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<AuroraMaterial>>,
) {
    for (entity, body) in &bodies {
        commands.entity(entity).insert(AuroraChecked);
        let ObjectRef::Body(i) = body.obj else { continue };
        let rec = &data.catalog.bodies[i as usize];
        let extras = BodyExtras::from_body_block(&rec.block);
        let Some(a) = extras.aurora.filter(|a| !extras.no_aurora && a.height_km > 0.0) else { continue };
        let seed = (se_core::proc_surface::name_seed(&rec.names[0]) % 1000) as f64 / 1000.0;
        for o in [&a.north, &a.south] {
            if o.radius_km <= 0.0 || o.rings == 0 || o.bright <= 0.0 {
                continue;
            }
            let c = |v: [f64; 3]| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32);
            let material = mats.add(AuroraMaterial {
                u: AuroraUniform {
                    sun_dir: Vec4::Y,
                    bottom: c(a.bottom_color).extend(o.bright as f32 * 1.5),
                    top: c(a.top_color).extend(0.0),
                    params: Vec4::new(o.flash_freq as f32, o.move_speed as f32, seed as f32, 0.0),
                },
            });
            commands.spawn((
                SystemMember,
                AuroraOf {
                    body: entity,
                    material: material.clone(),
                },
                Mesh3d(meshes.add(oval_mesh(body.radius_m, &a, o, seed))),
                MeshMaterial3d(material),
                UPosition(body.pos),
                UOrientation(body.orient),
                ScaleAnchor(body.pos),
                // Over SE's ellipsoid, flattened with the body.
                MeterScale(body.shape),
                Transform::IDENTITY,
                Visibility::Hidden,
                NoFrustumCulling,
            ));
        }
    }
}

fn update_auroras(
    bodies: Query<(&SystemBody, &BodyLighting)>,
    mut auroras: Query<(&AuroraOf, &mut UPosition, &mut UOrientation, &mut ScaleAnchor, &mut Visibility)>,
    mut mats: ResMut<Assets<AuroraMaterial>>,
) {
    for (aurora, mut pos, mut rot, mut anchor, mut vis) in &mut auroras {
        let Ok((b, light)) = bodies.get(aurora.body) else { continue };
        let show = b.px_radius > 3.0;
        let v = if show { Visibility::Visible } else { Visibility::Hidden };
        if *vis != v {
            *vis = v;
        }
        if !show {
            continue;
        }
        pos.0 = b.pos;
        rot.0 = b.orient;
        anchor.0 = b.pos;
        let sun = light.sun_dir.extend(0.0);
        if mats.get(&aurora.material).is_some_and(|m| (m.u.sun_dir - sun).length() > 1e-3) {
            if let Some(mut m) = mats.get_mut(&aurora.material) {
                m.u.sun_dir = sun;
            }
        }
    }
}
