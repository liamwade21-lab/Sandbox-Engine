//! Stellar coronae: SE's procedural corona (FUN_1405481c0 with corona_plane_proc.glsl, see
//! `corona.wesl`), three perpendicular planes of animated fbm rays fixed in each resolved star's
//! body frame. Sized and tuned per star by the catalog's `Corona` block or SE's defaults
//! (`se_core::body_params::Corona::resolve`, FUN_140507070); `NoCorona` switches it off. SE's
//! texture corona (corona_plane_tex.glsl, `StarCoronaTextures`) is only chosen in its star editor
//! (body +0x1d04 is 0 for every star it loads), so it is not ported.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use se_core::body_params::{BodyExtras, Corona as CoronaKeys, CoronaStar, corona_temperature_bright};
use se_core::catalog::ObjectRef;

use crate::data::SeData;
use crate::stars::star_render_color;
use crate::system::{StarSphere, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, SimTime, UOrientation, UPosition, UniverseSet};

/// FUN_1405481c0's fixed uniforms (the body's +0x1cf0..+0x1d00, set by FUN_1404d1970):
/// `Bright` = (Gamma 0.06, Brightness 1.5, RayBrightness 10.001, SpotBrightness 15).
const BRIGHT: Vec4 = Vec4::new(0.06, 1.5, 10.001, 15.0);
/// `Shape.z` (Scale) and `Shape.w` (SinFreq).
const SCALE: f32 = 0.25;
const SIN_FREQ: f32 = 13.0;
/// FUN_1405481c0 multiplies the corona's colour by 2.5 (DAT_1406d4e78).
const COLOR_GAIN: f32 = 2.5;
/// The smallest star (radius in pixels) with a corona (DAT_1406d4204, FUN_1404d4100).
const MIN_PX_RADIUS: f64 = 0.2;
/// Days of SE's corona clock are counted from an epoch the port does not know (DAT_140855480);
/// J2000 is used, which only shifts the pattern's phase.
const PHASE_EPOCH_JD: f64 = 2_451_545.0;

pub struct CoronaPlugin;

impl Plugin for CoronaPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CoronaMaterial>::default())
            .init_resource::<CoronaSettings>()
            .add_systems(Startup, load_settings)
            .add_systems(
                PostUpdate,
                (spawn_coronas, update_coronas)
                    .chain()
                    .after(UniverseSet::Camera)
                    .before(UniverseSet::FloatingOrigin),
            );
    }
}

/// main-def.cfg `hdrStarCoronaBright` (SE's HDR photo mode, the port's only mode).
#[derive(Resource)]
struct CoronaSettings {
    bright: f32,
}

impl Default for CoronaSettings {
    fn default() -> Self {
        Self { bright: 1.0 }
    }
}

fn load_settings(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<CoronaSettings>) {
    if let Some(v) = crate::stars::main_def_f64(&cfg, "hdrStarCoronaBright") {
        s.bright = v as f32;
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct CoronaUniform {
    /// rgb: SE's `Color`.
    pub color: Vec4,
    /// SE's `Bright`.
    pub bright: Vec4,
    /// SE's `Shape`: (RayDensity, RayCurvature, Scale, SinFreq).
    pub shape: Vec4,
    /// `EyePos.w` of the three planes.
    pub phase: Vec4,
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
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        // SE turns face culling off for the planes.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The corona planes of a star entity.
#[derive(Component)]
struct CoronaOf {
    star: Entity,
    keys: CoronaKeys,
    material: Handle<CoronaMaterial>,
}

/// Marks stars whose corona was resolved (whether or not they have one).
#[derive(Component)]
struct HasCorona;

/// FUN_1405481c0's three unit quads (half-size 1 = `Radius`): the plane z = 0, the same turned 90
/// degrees about x, then about y as well, so their normals are the body frame's z, y and x axes.
fn planes_mesh() -> Mesh {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    let planes: [(fn(f32, f32) -> [f32; 3], [f32; 3]); 3] =
        [(|x, y| [x, y, 0.0], [0.0, 0.0, 1.0]), (|x, y| [x, 0.0, -y], [0.0, 1.0, 0.0]), (|x, y| [0.0, x, -y], [-1.0, 0.0, 0.0])];
    for (place, n) in planes {
        let base = pos.len() as u32;
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            pos.push(place(x, y));
            nrm.push(n);
            uv.push([(x + 1.0) * 0.5, (y + 1.0) * 0.5]);
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// SE's `Color` uniform: the star's colour scaled to unit luma (0.299, 0.587, 0.114), times
/// FUN_1404c07f0 of its temperature, the corona `Brightness`, 2.5 and `hdrStarCoronaBright`. The body's
/// own `Brightness` is 1 for stars. Renderer +0x35a0 is the last frame's +0x35a4 (FUN_1404c51a0 copies
/// it and resets +0x35a4 to 1): 1 in space; with the camera inside an atmosphere FUN_14044fe70
/// multiplies it by an exp() of the air's extinction, which is not ported (taken as 1).
pub fn corona_color(rgb: [f32; 3], teff: f64, brightness: f64, hdr_bright: f32) -> Vec3 {
    let c = Vec3::from_array(rgb);
    let luma = c.dot(Vec3::new(0.299, 0.587, 0.114));
    let c = if luma > 0.0 { c / luma } else { c };
    c * corona_temperature_bright(teff) as f32 * brightness as f32 * COLOR_GAIN * hdr_bright
}

fn spawn_coronas(
    mut commands: Commands,
    data: Res<SeData>,
    settings: Res<CoronaSettings>,
    sim: Res<SimTime>,
    stars: Query<(Entity, &SystemBody), (With<StarSphere>, Without<HasCorona>)>,
    mut mesh: Local<Option<Handle<Mesh>>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<CoronaMaterial>>,
) {
    for (entity, body) in &stars {
        commands.entity(entity).insert(HasCorona);
        let ObjectRef::Star(i) = body.obj else { continue };
        let s = &data.catalog.stars[i as usize];
        let extras = BodyExtras::from_body_block(&s.block);
        let (lum, letter) = CoronaStar::class_code(&s.class);
        let mass_kg = if s.mass_kg > 0.0 { s.mass_kg } else { se_core::units::SOLAR_MASS_KG };
        let star = CoronaStar {
            lum,
            letter,
            radius_km: body.radius_m / 1000.0,
            teff: s.teff,
            mass_earth: mass_kg / se_core::units::EARTH_MASS_KG,
        };
        let mut rng = crate::blackholes::params::SeRng::from_name(&s.names[0]);
        let Some(keys) = CoronaKeys::resolve(extras.corona.as_ref(), extras.no_corona, &star, &mut |m, sd| rng.gauss(m, sd))
        else {
            continue;
        };
        let color = corona_color(star_render_color(s.class.kind, s.teff), s.teff, keys.brightness, settings.bright);
        let p = keys.phases(sim.jd - PHASE_EPOCH_JD);
        let material = mats.add(CoronaMaterial {
            u: CoronaUniform {
                color: color.extend(0.0),
                bright: BRIGHT,
                shape: Vec4::new(keys.ray_density as f32, keys.ray_curv as f32, SCALE, SIN_FREQ),
                phase: Vec4::new(p[0], p[1], p[2], 0.0),
            },
        });
        let mesh = mesh.get_or_insert_with(|| meshes.add(planes_mesh())).clone();
        commands.spawn((
            SystemMember,
            CoronaOf { star: entity, keys, material: material.clone() },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            UPosition(body.pos),
            UOrientation(body.orient),
            ScaleAnchor(body.pos),
            // FUN_1405481c0 scales the planes by the star's ellipsoid, (1 - oblateness) x Radius per
            // body axis (+0x1200..+0x1208), so the corona hugs a flattened star.
            MeterScale(DVec3::splat(keys.radius_km * 1000.0) * body.shape),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
        ));
    }
}

fn update_coronas(
    sim: Res<SimTime>,
    bodies: Query<&SystemBody>,
    mut coronas: Query<(&CoronaOf, &mut UPosition, &mut UOrientation, &mut ScaleAnchor, &mut Visibility)>,
    mut mats: ResMut<Assets<CoronaMaterial>>,
) {
    for (corona, mut pos, mut orient, mut anchor, mut vis) in &mut coronas {
        let Ok(b) = bodies.get(corona.star) else { continue };
        let show = b.px_radius >= MIN_PX_RADIUS;
        let v = if show { Visibility::Visible } else { Visibility::Hidden };
        if *vis != v {
            *vis = v;
        }
        if !show {
            continue;
        }
        pos.0 = b.pos;
        anchor.0 = b.pos;
        orient.0 = b.orient;
        let p = corona.keys.phases(sim.jd - PHASE_EPOCH_JD);
        let phase = Vec4::new(p[0], p[1], p[2], 0.0);
        if mats.get(&corona.material).is_some_and(|m| m.u.phase != phase)
            && let Some(mut m) = mats.get_mut(&corona.material)
        {
            m.u.phase = phase;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planes_are_perpendicular_unit_quads() {
        let m = planes_mesh();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(n)) = m.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!() };
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
        assert_eq!(p.len(), 12);
        for (q, axis) in [(0, 2), (1, 1), (2, 0)] {
            // Each quad lies in the plane its normal names.
            for v in &p[q * 4..q * 4 + 4] {
                assert_eq!(v[axis], 0.0);
            }
            assert_eq!(n[q * 4][axis].abs(), 1.0);
        }
    }

    #[test]
    fn corona_colour_has_unit_luma_times_the_gains() {
        // The Sun: 0.9 (FUN_1404c07f0 at 5880 K) x 0.5 x 2.5.
        let c = corona_color([1.0, 0.9, 0.8], 5880.0, 0.5, 1.0);
        assert!((c.dot(Vec3::new(0.299, 0.587, 0.114)) - 1.125).abs() < 0.02, "{c}");
        assert_eq!(corona_color([1.0; 3], 500.0, 1.0, 1.0), Vec3::ZERO);
    }
}
