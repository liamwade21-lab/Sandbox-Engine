//! Comets: a glowing coma around the nucleus plus two tails.
//!
//! Each comet body inside `VISIBLE_AU` of its star gets three child entities:
//! - a coma sphere whose radius grows as the comet nears the star,
//! - a straight bluish ion tail exactly anti-sunward,
//! - a broader, curved yellowish dust tail that bends towards the trailing side
//!   of the orbit (the direction opposite to the comet's velocity).
//!
//! Tails are unit-length tubes scaled by `MeterScale` (their true length in metres)
//! and oriented with `UOrientation`. Lengths and brightness follow r^-2. The
//! material is additive HDR (`AlphaMode::Add`, alpha 0 output). Intensity is faded
//! out when the camera is close to a comet so the tails do not blow out when the
//! camera is inside them.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DMat3, DQuat, DVec3};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use se_core::body_params::{BodyExtras, CometTailParams};
use se_core::catalog::{BodyTag, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::units::METERS_PER_AU;
use se_core::upos::UPos;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::system::{ActiveSystem, BodyLighting, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, SimTime, UOrientation, UPosition, UniverseSet};

/// Comets further than this from their star are not drawn.
const VISIBLE_AU: f64 = 6.0;
/// Kind values passed to the shader.
const KIND_COMA: f32 = 0.0;
const KIND_TAIL: f32 = 1.0;

#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct CometUniform {
    /// rgb tint, w = intensity.
    pub color: Vec4,
    /// x = kind (0 coma, 1 tail).
    pub params: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CometMaterial {
    #[uniform(0)]
    pub u: CometUniform,
}

impl Material for CometMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/comet.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/comet.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
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

#[derive(Resource)]
struct CometMeshes {
    coma: Handle<Mesh>,
    ion: Handle<Mesh>,
    dust: Handle<Mesh>,
}

/// Marks a comet body that already has its visual entities.
#[derive(Component)]
struct CometInfo {
    /// coma, ion tail, dust tail.
    parts: [Entity; 3],
    has_tail: bool,
    coma_rgb: Vec3,
    ion_rgb: Vec3,
    dust_rgb: Vec3,
    ion_gain: f32,
    dust_gain: f32,
}

pub struct CometsPlugin;

impl Plugin for CometsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CometMaterial>::default())
            .add_systems(Startup, setup_meshes)
            .add_systems(PostUpdate, (spawn_missing_comets, spawn_comets).chain())
            .add_systems(
                PostUpdate,
                update_comets.after(UniverseSet::Camera).before(UniverseSet::FloatingOrigin),
            );
    }
}

fn setup_meshes(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(CometMeshes {
        coma: meshes.add(Sphere::new(1.0).mesh().uv(32, 18)),
        ion: meshes.add(tube_mesh(0.0, 0.01, 0.035)),
        dust: meshes.add(tube_mesh(0.25, 0.02, 0.22)),
    });
}

/// Tube along +Y from 0 to 1 (uv.x = along), radius `w0 + (w1 - w0) * s`, centreline
/// bent by `bend * s^2` along +X.
fn tube_mesh(bend: f32, w0: f32, w1: f32) -> Mesh {
    const NS: usize = 48;
    const NA: usize = 24;
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity((NS + 1) * (NA + 1));
    let mut nrm: Vec<[f32; 3]> = Vec::with_capacity((NS + 1) * (NA + 1));
    let mut uv: Vec<[f32; 2]> = Vec::with_capacity((NS + 1) * (NA + 1));
    for i in 0..=NS {
        let s = i as f32 / NS as f32;
        let c = Vec3::new(bend * s * s, s, 0.0);
        let t = Vec3::new(2.0 * bend * s, 1.0, 0.0).normalize();
        let e1 = t.cross(Vec3::Z).normalize();
        let e2 = t.cross(e1);
        let w = w0 + (w1 - w0) * s;
        for j in 0..=NA {
            let a = std::f32::consts::TAU * j as f32 / NA as f32;
            let n = e1 * a.cos() + e2 * a.sin();
            pos.push((c + n * w).to_array());
            nrm.push(n.to_array());
            uv.push([s, j as f32 / NA as f32]);
        }
    }
    let row = (NA + 1) as u32;
    let mut idx = Vec::with_capacity(NS * NA * 6);
    for i in 0..NS as u32 {
        for j in 0..NA as u32 {
            let a = i * row + j;
            let b = a + 1;
            let c = a + row;
            let d = c + 1;
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Normalised tint from a catalog RGB, or `default` when the catalog has none.
fn tint(c: [f64; 3], default: [f32; 3]) -> Vec3 {
    let v = Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32);
    let m = v.max_element();
    if m > 1e-9 { v / m } else { Vec3::from_array(default) }
}

fn spawn_part(commands: &mut Commands, mesh: Handle<Mesh>, materials: &mut Assets<CometMaterial>) -> Entity {
    let mat = materials.add(CometMaterial {
        u: CometUniform::default(),
    });
    commands
        .spawn((
            SystemMember,
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            UPosition::default(),
            UOrientation::default(),
            MeterScale(DVec3::ONE),
            ScaleAnchor(UPos::ZERO),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
        ))
        .id()
}

/// The catalog's member list of the active system can omit comets, so when the
/// active system changes, spawn a body entity for every comet of that system that
/// is missing. Other systems then treat it like any other body.
fn spawn_missing_comets(
    mut commands: Commands,
    data: Res<SeData>,
    active: Res<ActiveSystem>,
    sim: Res<SimTime>,
    mut last_root: Local<Option<u32>>,
    bodies: Query<&SystemBody>,
) {
    let Some(root) = active.root else { return };
    if *last_root == Some(root) {
        return;
    }
    *last_root = Some(root);
    let cat = &data.catalog;
    let eph = Ephemeris::new(cat);
    for (i, b) in cat.bodies.iter().enumerate() {
        if !matches!(b.tag, BodyTag::Comet) {
            continue;
        }
        let obj = ObjectRef::Body(i as u32);
        if cat.system_root(obj) != Some(root) {
            continue;
        }
        if bodies.iter().any(|s| matches!(s.obj, ObjectRef::Body(j) if j as usize == i)) {
            continue;
        }
        let radius_m = eph.radius_m(obj);
        if radius_m <= 0.0 {
            continue;
        }
        let pos = eph.position(obj, sim.jd);
        let orient = eph.orientation(obj, sim.jd);
        commands.spawn((
            SystemMember,
            SystemBody {
                obj,
                radius_m,
                star: false,
                pos,
                orient,
                px_radius: 0.0,
                shape: DVec3::ONE,
            },
            BodyLighting::default(),
            UPosition(pos),
            UOrientation(orient),
            MeterScale(DVec3::splat(radius_m)),
            Transform::IDENTITY,
            Visibility::Hidden,
        ));
    }
}

/// Gives each comet body in the active system its coma and tail entities.
fn spawn_comets(
    mut commands: Commands,
    data: Res<SeData>,
    meshes: Res<CometMeshes>,
    mut materials: ResMut<Assets<CometMaterial>>,
    bodies: Query<(Entity, &SystemBody), Without<CometInfo>>,
) {
    let cat = &data.catalog;
    for (entity, body) in &bodies {
        let ObjectRef::Body(i) = body.obj else { continue };
        let b = &cat.bodies[i as usize];
        if !matches!(b.tag, BodyTag::Comet) {
            continue;
        }
        let extras = BodyExtras::from_body_block(&b.block);
        let (bright, gas_bright, dust_bright) = match extras.comet_tail.as_ref() {
            Some(t) => (t.bright, t.gas_bright, t.dust_bright),
            None => (1.0, 1.0, 1.0),
        };
        let tail: CometTailParams = extras.comet_tail.clone().unwrap_or_else(|| CometTailParams {
            bright: 1.0,
            ..Default::default()
        });
        let coma = spawn_part(&mut commands, meshes.coma.clone(), &mut materials);
        let ion = spawn_part(&mut commands, meshes.ion.clone(), &mut materials);
        let dust = spawn_part(&mut commands, meshes.dust.clone(), &mut materials);
        commands.entity(entity).insert(CometInfo {
            parts: [coma, ion, dust],
            has_tail: !extras.no_comet_tail,
            coma_rgb: Vec3::new(0.7, 0.95, 0.85),
            ion_rgb: tint(tail.gas_color, [0.3, 0.6, 1.0]),
            dust_rgb: tint(tail.dust_color, [1.0, 0.85, 0.5]),
            ion_gain: (bright * gas_bright) as f32,
            dust_gain: (bright * dust_bright) as f32,
        });
    }
}

type PartQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut UPosition,
        &'static mut UOrientation,
        &'static mut MeterScale,
        &'static mut ScaleAnchor,
        &'static mut Visibility,
        &'static MeshMaterial3d<CometMaterial>,
    ),
>;

/// Desired state of one visual part for this frame.
struct Part {
    entity: Entity,
    pos: UPos,
    rot: DQuat,
    scale: DVec3,
    visible: bool,
    rgb: Vec3,
    intensity: f32,
    kind: f32,
}

fn apply_part(p: Part, parts: &mut PartQuery<'_, '_>, materials: &mut Assets<CometMaterial>) {
    let Ok((mut upos, mut rot, mut scale, mut anchor, mut vis, mat)) = parts.get_mut(p.entity) else {
        return;
    };
    upos.0 = p.pos;
    anchor.0 = p.pos;
    rot.0 = p.rot;
    scale.0 = p.scale;
    *vis = if p.visible { Visibility::Visible } else { Visibility::Hidden };
    if p.visible {
        if let Some(mut m) = materials.get_mut(&mat.0) {
            m.u.color = p.rgb.extend(p.intensity);
            m.u.params = Vec4::new(p.kind, 0.0, 0.0, 0.0);
        }
    }
}

fn update_comets(
    data: Res<SeData>,
    sim: Res<SimTime>,
    cam: Res<FlyCamera>,
    owners: Query<(&SystemBody, &BodyLighting, &CometInfo)>,
    mut parts: PartQuery,
    mut materials: ResMut<Assets<CometMaterial>>,
) {
    let eph = Ephemeris::new(&data.catalog);
    for (body, light, info) in &owners {
        let [coma, ion, dust] = info.parts;
        let away = body.pos.delta_m(light.star_pos);
        let r_m = away.length();
        let r_au = r_m / METERS_PER_AU;
        let visible = r_au > 1e-6 && r_au < VISIBLE_AU;
        if !visible {
            for e in [coma, ion, dust] {
                apply_part(hidden(e), &mut parts, &mut materials);
            }
            continue;
        }
        let a_hat = away / r_m;
        // Trailing direction: the orbit velocity (from a 1 day step) with its
        // component along the anti-sun axis removed, then negated.
        let v_hat = eph.position(body.obj, sim.jd + 1.0).delta_m(body.pos).normalize_or_zero();
        let lag = -v_hat + a_hat * v_hat.dot(a_hat);
        let x = if lag.length_squared() > 1e-12 {
            lag.normalize()
        } else {
            a_hat.any_orthonormal_vector()
        };
        let y = a_hat;
        let z = x.cross(y);
        let rot_tail = DQuat::from_mat3(&DMat3::from_cols(x, y, z));

        // Sizes and brightness, r^-2 with clamps.
        let inv_r2 = 1.0 / (r_au * r_au);
        let g = inv_r2.min(12.0) as f32;
        let len_km = (4.0e7 * inv_r2).clamp(2.0e6, 1.2e8);
        let ion_m = len_km * 1000.0;
        let dust_m = 0.6 * ion_m;
        let coma_km = (2.0e4 / r_au.sqrt()).clamp(1.0e4, 1.0e5);
        let coma_m = coma_km * 1000.0;

        // Fade by camera distance so nothing blows out when the camera is inside.
        let d_cam = body.pos.delta_m(cam.pos).length();
        let f_coma = smoothstep(0.3 * coma_m, 2.0 * coma_m, d_cam) as f32;
        let f_ion = smoothstep(0.02 * ion_m, 0.25 * ion_m, d_cam) as f32;
        let f_dust = smoothstep(0.02 * dust_m, 0.25 * dust_m, d_cam) as f32;

        apply_part(
            Part {
                entity: coma,
                pos: body.pos,
                rot: DQuat::IDENTITY,
                scale: DVec3::splat(coma_m),
                visible: true,
                rgb: info.coma_rgb,
                intensity: 3.0 * g * f_coma,
                kind: KIND_COMA,
            },
            &mut parts,
            &mut materials,
        );
        let tails_on = info.has_tail;
        apply_part(
            Part {
                entity: ion,
                pos: body.pos,
                rot: rot_tail,
                scale: DVec3::splat(ion_m),
                visible: tails_on,
                rgb: info.ion_rgb,
                intensity: 1.2 * info.ion_gain * g * f_ion,
                kind: KIND_TAIL,
            },
            &mut parts,
            &mut materials,
        );
        apply_part(
            Part {
                entity: dust,
                pos: body.pos,
                rot: rot_tail,
                scale: DVec3::splat(dust_m),
                visible: tails_on,
                rgb: info.dust_rgb,
                intensity: 0.2 * info.dust_gain * g * f_dust,
                kind: KIND_TAIL,
            },
            &mut parts,
            &mut materials,
        );
    }
}

/// A part that is not drawn this frame.
fn hidden(entity: Entity) -> Part {
    Part {
        entity,
        pos: UPos::ZERO,
        rot: DQuat::IDENTITY,
        scale: DVec3::ONE,
        visible: false,
        rgb: Vec3::ZERO,
        intensity: 0.0,
        kind: KIND_COMA,
    }
}
