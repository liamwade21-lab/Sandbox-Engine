//! Comet tails: SpaceEngine's animated particle sprites (comet_tail.glsl, drawn by
//! FUN_140484400 from the body renderer FUN_1404d4100).
//!
//! Every comet gets SE's tail values ([`se_core::procgen::tail_defaults`], FUN_140486280, for what
//! its catalog entry leaves out) and, once its tail is in view, a mesh of `Particles` sprites
//! (FUN_1404858f0): random directions, the first `Particles * GasToDust` gas, the rest dust. The
//! tail is `MaxLength (1 AU / r)^SlopeParam` long (FUN_140485750) and drawn in the universe axes,
//! scaled by that length (SE's model matrix is a translation and that scale). The shader moves
//! each particle through its 0.6-day cycle: gas straight away from the star, dust pushed by light
//! pressure and left behind along the orbit (`CometVel`), both spreading faster the further the
//! comet is from perihelion (`1 - (q / r)^0.25`) and fading out there. The young particles
//! crowded at the nucleus make the coma (SE draws no other).
//!
//! Brightness per particle is SE's `5000 CometTailBright hdrCometTailBright Gas|DustBright /
//! (particles of that kind)` times the kind's colour (main-def.cfg, HDR photo mode). Not ported:
//! SE fades a tail in by the comet's apparent brightness (`(b - 0.05) * 5`, FUN_1404da600) and
//! renders big tails to an impostor (`EnableCometFBO`); the port draws a tail once it is at least
//! two pixels long.

use std::collections::HashSet;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat};
use bevy::shader::ShaderRef;
use se_core::catalog::{BodyTag, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::procgen::{tail_defaults, tail_particles, CometTailGiven};
use se_core::surface_gen::SeRng;
use se_core::units::{EARTH_MASS_KG, METERS_PER_AU};

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::system::{ActiveSystem, BodyLighting, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, SimTime, UOrientation, UPosition, UniverseSet};

/// SE's vertex bytes of a particle (`VertexData`): phase high, phase low, kind, sprite corner.
pub const ATTRIBUTE_COMET_DATA: MeshVertexAttribute = MeshVertexAttribute::new("CometData", 0x5E57_0C01, VertexFormat::Unorm8x4);
/// A particle's life in simulated days (DAT_1406d49b0).
const CYCLE_DAYS: f64 = 0.6;
/// SE's tail brightness scale (DAT_1406d5b58).
const TAIL_BRIGHT: f32 = 5000.0;
/// Tails shorter than this many pixels are not drawn (the port's).
const MIN_TAIL_PX: f64 = 2.0;
/// Particle meshes built per frame at most.
const BUILDS_PER_FRAME: usize = 4;
/// Longest tail drawn (the port's cap on `MaxLength (1 AU / r)^SlopeParam` close to the star).
const MAX_TAIL_AU: f64 = 1.0e4;

#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct CometTailUniform {
    /// x tail length (km), y particle cycle phase, z `1 - (q / r)^0.25`, w sprite scale (SE's
    /// `ScaleTime`).
    pub scale_time: Vec4,
    /// rgb colour times the brightness of one particle of each kind.
    pub gas_color: Vec4,
    pub dust_color: Vec4,
    /// xyz the comet's velocity about its star (km/s, universe axes).
    pub comet_vel: Vec4,
    /// xyz comet minus camera (AU).
    pub eye_pos: Vec4,
    /// xyz comet minus star (AU); w SE's light-pressure factor (its `1 / r^2` clamped to 1, which
    /// SE computes from AU taken for km, so always 1).
    pub sun_pos: Vec4,
    /// rgb the star's light, a 1.
    pub sun_col: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CometMaterial {
    #[uniform(0)]
    pub u: CometTailUniform,
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
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0), ATTRIBUTE_COMET_DATA.at_shader_location(1)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// `CometTailBright` (main-def.cfg `PlanetRendering`) times `hdrCometTailBright` (HDR photo mode).
#[derive(Resource)]
struct CometSettings {
    bright: f32,
}

/// A comet body's tail.
#[derive(Component)]
struct Tail {
    /// The entity drawing the particles.
    part: Entity,
    tail: se_core::procgen::CometTail,
    /// Seed of the particles.
    seed: u32,
    built: bool,
    /// `NoCometTail` unset.
    enabled: bool,
    /// The body the comet orbits, for its velocity.
    parent: Option<ObjectRef>,
    /// Perihelion distance (m) and orbit epoch (JD).
    q_m: f64,
    epoch: f64,
}

pub struct CometsPlugin;

impl Plugin for CometsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CometMaterial>::default())
            .add_systems(Startup, load_settings)
            .add_systems(PostUpdate, (spawn_missing_comets, spawn_tails).chain())
            .add_systems(PostUpdate, update_tails.after(UniverseSet::Camera).before(UniverseSet::FloatingOrigin));
    }
}

fn load_settings(mut commands: Commands, cfg: Res<crate::config::EngineConfig>) {
    let f = |k: &str| crate::stars::main_def_f64(&cfg, k).unwrap_or(1.0) as f32;
    commands.insert_resource(CometSettings { bright: f("CometTailBright") * f("hdrCometTailBright") });
}

fn fnv1a(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// The catalog's member list of the active system can omit comets, so once the system loader
/// has spawned the members (the body count has held for half a second), spawn a body entity for
/// every comet of that system still missing. Other systems then treat it like any other body.
fn spawn_missing_comets(
    mut commands: Commands,
    data: Res<SeData>,
    active: Res<ActiveSystem>,
    sim: Res<SimTime>,
    mut state: Local<(Option<u32>, usize, u32, bool)>,
    bodies: Query<&SystemBody>,
) {
    let Some(root) = active.root else { return };
    let (last_root, last_count, steady, done) = &mut *state;
    if *last_root != Some(root) {
        *state = (Some(root), active.bodies.len(), 0, false);
        return;
    }
    if *done {
        return;
    }
    if active.bodies.len() != *last_count {
        *last_count = active.bodies.len();
        *steady = 0;
        return;
    }
    *steady += 1;
    if *steady < 30 {
        return;
    }
    *done = true;
    let cat = &data.catalog;
    let eph = Ephemeris::new(cat);
    let have: HashSet<u32> = bodies.iter().filter_map(|s| if let ObjectRef::Body(j) = s.obj { Some(j) } else { None }).collect();
    for (i, b) in cat.bodies.iter().enumerate() {
        if !matches!(b.tag, BodyTag::Comet) || have.contains(&(i as u32)) {
            continue;
        }
        let obj = ObjectRef::Body(i as u32);
        if cat.system_root(obj) != Some(root) {
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
            SystemBody { obj, radius_m, star: false, pos, orient, px_radius: 0.0, shape: DVec3::ONE },
            BodyLighting::default(),
            UPosition(pos),
            UOrientation(orient),
            MeterScale(DVec3::splat(radius_m)),
            Transform::IDENTITY,
            Visibility::Hidden,
        ));
    }
}

/// Gives each comet body of the active system its tail (one per comet, should a comet have two
/// body entities).
fn spawn_tails(
    mut commands: Commands,
    data: Res<SeData>,
    active: Res<ActiveSystem>,
    mut materials: ResMut<Assets<CometMaterial>>,
    bodies: Query<(Entity, &SystemBody), Without<Tail>>,
    mut seen: Local<(Option<u32>, HashSet<u32>)>,
) {
    if seen.0 != active.root {
        *seen = (active.root, HashSet::new());
    }
    let cat = &data.catalog;
    for (entity, body) in &bodies {
        let ObjectRef::Body(i) = body.obj else { continue };
        let b = &cat.bodies[i as usize];
        if !matches!(b.tag, BodyTag::Comet) || !seen.1.insert(i) {
            continue;
        }
        let given = CometTailGiven::from_body_block(&b.block);
        let orbit = b.orbit.as_ref();
        let ecc = orbit.map_or(0.0, |o| o.eccentricity);
        // SE draws the missing values on its thread's generator as the system loads; the port
        // seeds a stream by the comet's name.
        let seed = fnv1a(&b.names[0]);
        let mut rng = SeRng::new(seed, seed.rotate_left(16) ^ 0x9e37_79b9);
        let tail = tail_defaults(&mut rng, &given, b.mass_kg / EARTH_MASS_KG, ecc);
        let q_m = orbit.map_or(0.0, |o| if (o.eccentricity - 1.0).abs() < 1e-9 { o.semi_major_axis } else { o.semi_major_axis * (1.0 - o.eccentricity).abs() });
        let material = materials.add(CometMaterial { u: CometTailUniform::default() });
        let part = commands
            .spawn((
                SystemMember,
                MeshMaterial3d(material),
                UPosition(body.pos),
                UOrientation(DQuat::IDENTITY),
                MeterScale(DVec3::ONE),
                ScaleAnchor(body.pos),
                Transform::IDENTITY,
                Visibility::Hidden,
                NoFrustumCulling,
            ))
            .id();
        let no_tail = b.block.bool("NoCometTail").unwrap_or(false);
        commands.entity(entity).insert(Tail {
            part,
            tail,
            seed,
            built: false,
            enabled: !no_tail,
            parent: cat.find(&b.parent),
            q_m,
            epoch: orbit.map_or(0.0, |o| o.epoch),
        });
    }
}

/// The particle mesh of a tail: four corners per particle sharing its direction and bytes.
fn tail_mesh(seed: u32, particles: u32, gas_to_dust: f32) -> Mesh {
    let ps = tail_particles(seed, particles, gas_to_dust);
    let mut pos = Vec::with_capacity(ps.len() * 4);
    let mut data = Vec::with_capacity(ps.len() * 4);
    let mut idx = Vec::with_capacity(ps.len() * 6);
    for (k, p) in ps.iter().enumerate() {
        for corner in [0x00u8, 0x40, 0x80, 0xc0] {
            pos.push(p.dir);
            data.push([p.data[0], p.data[1], p.data[2], corner]);
        }
        let a = k as u32 * 4;
        idx.extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(ATTRIBUTE_COMET_DATA, VertexAttributeValues::Unorm8x4(data))
        .with_inserted_indices(Indices::U32(idx))
}

/// Tail length (km) at `r_m` from the star (FUN_140485750).
fn tail_length_km(t: &se_core::procgen::CometTail, r_m: f64) -> f64 {
    if t.max_length_km <= 0.0 || r_m <= 0.0 {
        return 0.0;
    }
    let l = t.max_length_km as f64 * (METERS_PER_AU / r_m).powf(t.slope_param as f64);
    if l.is_finite() { l.min(MAX_TAIL_AU * METERS_PER_AU / 1000.0) } else { 0.0 }
}

#[allow(clippy::too_many_arguments)]
fn update_tails(
    mut commands: Commands,
    data: Res<SeData>,
    sim: Res<SimTime>,
    cam: Res<FlyCamera>,
    settings: Res<CometSettings>,
    windows: Query<&Window>,
    cams: Query<&Projection, With<MainCamera>>,
    mut owners: Query<(&SystemBody, &BodyLighting, &mut Tail)>,
    mut parts: Query<(&mut UPosition, &mut MeterScale, &mut ScaleAnchor, &mut Visibility, &MeshMaterial3d<CometMaterial>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CometMaterial>>,
) {
    let h = windows.iter().next().map_or(900.0, |w| w.height() as f64);
    let fov = cams.iter().next().and_then(|p| if let Projection::Perspective(p) = p { Some(p.fov as f64) } else { None }).unwrap_or(1.0);
    let px_per_rad = h * 0.5 / (fov * 0.5).tan();
    let eph = Ephemeris::new(&data.catalog);
    let mut builds = 0;
    for (body, light, mut tail) in &mut owners {
        let Ok((mut upos, mut scale, mut anchor, mut vis, mat)) = parts.get_mut(tail.part) else { continue };
        let away = body.pos.delta_m(light.star_pos);
        let r_m = away.length();
        let len_km = tail_length_km(&tail.tail, r_m);
        let d_cam = body.pos.delta_m(cam.pos).length().max(1.0);
        let shown = tail.enabled && tail.tail.particles > 0 && light.sun_intensity > 0.0 && len_km * 1000.0 / d_cam * px_per_rad >= MIN_TAIL_PX;
        if !shown || (!tail.built && builds >= BUILDS_PER_FRAME) {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        if !tail.built {
            let mesh = meshes.add(tail_mesh(tail.seed, tail.tail.particles, tail.tail.gas_to_dust));
            commands.entity(tail.part).insert(Mesh3d(mesh));
            tail.built = true;
            builds += 1;
        }
        // The comet's velocity about its star, from a 0.01 day step.
        let dt = 0.01;
        let rel = |jd: f64| match tail.parent {
            Some(p) => eph.position(body.obj, jd).delta_m(eph.position(p, jd)),
            None => eph.position(body.obj, jd).delta_m(light.star_pos),
        };
        let vel_kms = (rel(sim.jd + dt) - rel(sim.jd)) / (dt * 86_400.0) / 1000.0;
        let t = &tail.tail;
        let n_gas = t.particles as f32 * t.gas_to_dust;
        let n_dust = t.particles as f32 - n_gas;
        let k = TAIL_BRIGHT * settings.bright;
        let gas = Vec3::from_array(t.gas_color) * (k * t.gas_bright / n_gas.max(1.0));
        let dust = Vec3::from_array(t.dust_color) * (k * t.dust_bright / n_dust.max(1.0));
        let phase = ((sim.jd - tail.epoch) / CYCLE_DAYS).rem_euclid(1.0);
        let z = 1.0 - (tail.q_m / r_m).powf(0.25);
        if let Some(mut m) = materials.get_mut(&mat.0) {
            m.u = CometTailUniform {
                scale_time: Vec4::new(len_km as f32, phase as f32, z as f32, 1.0),
                gas_color: gas.extend(0.0),
                dust_color: dust.extend(0.0),
                comet_vel: vel_kms.as_vec3().extend(0.0),
                eye_pos: (body.pos.delta_m(cam.pos) / METERS_PER_AU).as_vec3().extend(0.0),
                sun_pos: (away / METERS_PER_AU).as_vec3().extend(1.0),
                sun_col: (light.sun_color * light.sun_intensity).extend(1.0),
            };
        }
        upos.0 = body.pos;
        anchor.0 = body.pos;
        scale.0 = DVec3::splat(len_km * 1000.0);
        *vis = Visibility::Visible;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(max_au: f64, slope: f32) -> se_core::procgen::CometTail {
        se_core::procgen::CometTail {
            max_length_km: (max_au * METERS_PER_AU / 1000.0) as f32,
            particles: 1000,
            gas_to_dust: 0.25,
            gas_bright: 0.5,
            dust_bright: 0.5,
            gas_color: [0.04, 0.12, 0.2],
            dust_color: [1.0; 3],
            abs_magn: 8.0,
            slope_param: slope,
        }
    }

    #[test]
    fn tail_length_goes_as_r_to_minus_slope() {
        let t = tail(0.5, 4.0);
        let au = |r: f64| tail_length_km(&t, r * METERS_PER_AU) * 1000.0 / METERS_PER_AU;
        assert!((au(1.0) - 0.5).abs() < 1e-6);
        assert!((au(2.0) - 0.5 / 16.0).abs() < 1e-6);
        assert!((au(0.5) - 8.0).abs() < 1e-5);
        // Capped close to the star; nothing without a length.
        assert!((au(1e-4) - MAX_TAIL_AU).abs() < 1e-3);
        assert_eq!(tail_length_km(&tail(0.0, 4.0), METERS_PER_AU), 0.0);
    }

    #[test]
    fn tail_mesh_has_four_corners_per_particle() {
        let m = tail_mesh(5, 100, 0.25);
        assert_eq!(m.count_vertices(), 400);
        assert_eq!(m.indices().map(|i| i.len()), Some(600));
        let Some(VertexAttributeValues::Unorm8x4(d)) = m.attribute(ATTRIBUTE_COMET_DATA) else { panic!("no data") };
        assert_eq!([d[0][3], d[1][3], d[2][3], d[3][3]], [0x00, 0x40, 0x80, 0xc0]);
        assert_eq!(d[0][..3], d[3][..3]);
    }
}
