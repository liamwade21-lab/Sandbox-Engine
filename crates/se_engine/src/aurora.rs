//! Auroras as SpaceEngine draws them (aurora.glsl; FUN_1404543d0 completes the catalog's `Aurora`
//! block, FUN_140453af0 builds the particles, FUN_140451fe0 draws them): each oval is a set of
//! sprite particles on rings around its axis, at the top of the atmosphere, displaced, stretched
//! up by the aurora's `Height` and coloured by noise in `aurora.wesl`. No day/night mask (SE has
//! none).

use std::f64::consts::{FRAC_PI_2, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::DVec3;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use se_core::body_params::{AuroraOval, AuroraParams, BodyExtras};
use se_core::catalog::ObjectRef;
use se_core::surface_gen::bulk;

use crate::data::SeData;
use crate::system::{BodyLighting, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, SimTime, UOrientation, UPosition, UniverseSet};

pub struct AuroraPlugin;

impl Plugin for AuroraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<AuroraMaterial>::default())
            .init_resource::<AuroraSettings>()
            .add_systems(Startup, load_aurora_settings)
            .add_systems(
                PostUpdate,
                (spawn_auroras, update_auroras)
                    .chain()
                    .after(UniverseSet::Camera)
                    .before(UniverseSet::FloatingOrigin),
            );
    }
}

/// main-def.cfg `AuroraQuality` (0: SE's TEX_NOISE shader, 1: DETAIL) and `hdrAuroraBright`.
#[derive(Resource, Clone, Copy, Debug)]
pub struct AuroraSettings {
    pub quality: u32,
    pub hdr_bright: f32,
    noise: Option<AssetId<Image>>,
}

impl Default for AuroraSettings {
    fn default() -> Self {
        Self { quality: 0, hdr_bright: 1.0, noise: None }
    }
}

fn load_aurora_settings(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<AuroraSettings>) {
    if let Some(v) = crate::stars::main_def_f64(&cfg, "AuroraQuality") {
        s.quality = v.max(0.0) as u32;
    }
    if let Some(v) = crate::stars::main_def_f64(&cfg, "hdrAuroraBright") {
        s.hdr_bright = v as f32;
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct AuroraUniform {
    /// The oval's frame in the body frame (columns; y its axis); `frame_x.w` the base radius (m).
    pub frame_x: Vec4,
    pub frame_y: Vec4,
    pub frame_z: Vec4,
    /// SE's ScaleTime: sprite width (base radii), sprite length (Height / base), the ring's
    /// displacement gain sin(width / (rings + 1)) x 20, the flash phase.
    pub scale_time: Vec4,
    /// SE's TopColor and BottomColor times the brightness.
    pub top: Vec4,
    pub bottom: Vec4,
    /// xy SE's SunVecClip.xy, z 1 for the DETAIL shader (AuroraQuality 1).
    pub sun_vec: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct AuroraMaterial {
    #[uniform(0)]
    pub u: AuroraUniform,
    #[texture(1)]
    #[sampler(2)]
    pub noise: Handle<Image>,
}

impl Material for AuroraMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/aurora.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/aurora.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // SE draws them additively (glBlendFunc mode 1).
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
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0), Mesh::ATTRIBUTE_COLOR.at_shader_location(1)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// SE's 256 x 256 `NoiseTex` (its contents were not traced: random texels, filtered and
/// repeating, the value noise its Fbm sums expect).
fn noise_image() -> Image {
    const N: u32 = 256;
    let mut state = 0x2545_f491u32;
    let data: Vec<u8> = (0..N * N * 4)
        .map(|_| {
            state = state.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
            (state >> 24) as u8
        })
        .collect();
    let mut img = Image::new(Extent3d { width: N, height: N, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    img
}

/// One oval as SE draws it, angles in radians (FUN_1404543d0's conversions and defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeOval {
    pub lat: f64,
    pub lon: f64,
    /// Angular radius and width: asin(km / base radius).
    pub radius: f64,
    pub width: f64,
    pub rings: u32,
    pub bright: f64,
    pub flash_freq: f64,
    pub move_speed: f64,
    pub particles: u32,
}

/// SE's LCG (the thread's stream, FUN_140443d50 and the particle builder).
fn lcg(s: &mut u32) -> u32 {
    *s = s.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
    *s
}

/// SE's normal draw (FUN_140443d50): twelve uniforms summed.
fn normal(s: &mut u32, mean: f64, sigma: f64) -> f64 {
    let sum: f64 = (0..12).map(|_| lcg(s) as f64 / 4_294_967_296.0).sum();
    mean + (sum - 6.0) * sigma
}

impl SeOval {
    /// The catalog oval completed as FUN_1404543d0 does for a body of bulk class `bulk` with the
    /// aurora's base at `base_km`: degrees to radians, km to angles, and SE's defaults for what the
    /// catalog leaves out (FlashFreq N(70, 5), MoveSpeed 10^N(0, 0.1), 2 to 5 rings, particles 5000
    /// for brown dwarfs, 10000 for giants, 50000 otherwise, brightness 0.3 to 1 by size).
    pub fn from_catalog(o: &AuroraOval, base_km: f64, radius_km: f64, bulk_class: i32, seed: &mut u32) -> Self {
        let ang = |km: f64| (km / base_km).clamp(-1.0, 1.0).asin();
        let rings = if o.rings >= 1 { o.rings } else { (lcg(seed) & 3) + 2 };
        let particles = o.particles.unwrap_or(match bulk_class {
            bulk::BROWN_DWARF => 5000,
            bulk::NEPTUNE | bulk::JUPITER => 10000,
            _ => 50000,
        });
        let bright = o.bright.unwrap_or_else(|| {
            let x = radius_km / 6400.0 - 1.0;
            if x < 0.0 { 0.3 } else if x > 1.0 { 1.0 } else { x * 0.7 + 0.3 }
        });
        Self {
            lat: o.lat_deg.to_radians(),
            lon: o.lon_deg.to_radians(),
            radius: ang(o.radius_km),
            width: ang(o.width_km),
            rings,
            bright,
            flash_freq: o.flash_freq.unwrap_or_else(|| normal(seed, 70.0, 5.0)),
            move_speed: o.move_speed.unwrap_or_else(|| 10f64.powf(normal(seed, 0.0, 0.1))),
            particles,
        }
    }

    /// The oval's axis in SE's body frame (+X lon 0, +Y north, +Z lon 90 W).
    pub fn axis(&self) -> DVec3 {
        DVec3::new(self.lat.cos() * self.lon.cos(), self.lat.sin(), -self.lat.cos() * self.lon.sin())
    }
}

/// SE's default colours (FUN_1404543d0): brown dwarfs red over pink, giants white over pink,
/// others white over green.
fn default_colors(bulk_class: i32) -> ([f64; 3], [f64; 3]) {
    match bulk_class {
        bulk::BROWN_DWARF => ([1.0, 0.0, 0.0], [1.0, 0.0, 0.5]),
        bulk::NEPTUNE | bulk::JUPITER => ([1.0, 1.0, 1.0], [1.0, 0.0, 0.5]),
        _ => ([1.0, 1.0, 1.0], [0.0, 1.0, 0.0]),
    }
}

/// The particles of one oval, as FUN_140453af0 builds them: per ring, a share of the particles by
/// its circumference (the last ring takes the rest), one random start angle and a pair of random
/// phase bytes; each particle a quad of four vertices at its point of the unit sphere (axis +y),
/// data (phase low, phase high, 0, corner 0 / 0x40 / 0x80 / 0xc0), all /255.
pub fn oval_particles(o: &SeOval, seed: &mut u32) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let rings = o.rings.max(1) as i32;
    let n = o.particles as i32;
    let delta = (o.width / (rings + 1) as f64) as f32;
    let theta0 = (FRAC_PI_2 - o.radius) as f32;
    let c0 = theta0.cos();
    let (mut pos, mut data) = (Vec::with_capacity(n as usize * 4), Vec::with_capacity(n as usize * 4));
    let mut verts = 0i32;
    for r in 0..rings {
        let ang = (rings - 1) as f32 * delta * 0.5 + theta0 - r as f32 * delta;
        let s = ang.cos();
        // An oval of radius 0 (no NorthRadius/SouthRadius) has c0 <= 0: share the particles evenly
        // rather than by a ratio that runs to billions. No ring takes more than is left.
        let mut nr = if c0 > 0.0 && (s / c0).is_finite() { (s / c0 * n as f32 / rings as f32) as i32 } else { n / rings };
        let left = n - verts / 4;
        nr = if r == rings - 1 { left } else { nr.clamp(0, left.max(0)) };
        let mut phi = lcg(seed) as f32 * std::f32::consts::TAU * 2.328_306_4e-10;
        let dphi = (rings as f64 * TAU / nr as f64) as f32;
        let lo = (lcg(seed) % 255) as f32 / 255.0;
        let hi = (lcg(seed) % 255) as f32 / 255.0;
        if nr <= 0 {
            continue;
        }
        let y = ang.sin();
        for _ in 0..nr {
            let p = [phi.sin() * s, y, phi.cos() * s];
            phi += dphi;
            for corner in [0u8, 0x40, 0x80, 0xc0] {
                pos.push(p);
                data.push([lo, hi, 0.0, corner as f32 / 255.0]);
            }
        }
        verts += nr * 4;
    }
    (pos, data)
}

/// Advances `seed` as [`oval_particles`] does: three draws a ring, whatever its particles.
fn skip_particles(o: &SeOval, seed: &mut u32) {
    for _ in 0..o.rings.max(1) * 3 {
        lcg(seed);
    }
}

fn particle_mesh(pos: Vec<[f32; 3]>, data: Vec<[f32; 4]>) -> Mesh {
    let quads = pos.len() as u32 / 4;
    let idx: Vec<u32> = (0..quads).flat_map(|q| [0, 1, 2, 1, 3, 2].map(|i| q * 4 + i)).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, data)
        .with_inserted_indices(Indices::U32(idx))
}

/// One auroral oval attached to a body.
#[derive(Component)]
struct AuroraOf {
    body: Entity,
    material: Handle<AuroraMaterial>,
    oval: SeOval,
    /// Base radius (km): the top of the atmosphere.
    base_km: f64,
    height_km: f64,
    top: Vec3,
    bottom: Vec3,
    /// The seed of its particles until they are built, the first frame the oval shows (most are
    /// never seen): its `Mesh3d` handle is reserved at spawn and filled then.
    particle_seed: Option<u32>,
}

/// Marks bodies whose aurora has been looked up.
#[derive(Component)]
struct AuroraChecked;

#[allow(clippy::too_many_arguments)]
fn spawn_auroras(
    mut commands: Commands,
    data: Res<SeData>,
    // A body waiting for its atmosphere model has no terrain yet to tell its bulk class.
    bodies: Query<(Entity, &SystemBody, Option<&crate::terrain::Terrain>), (Without<AuroraChecked>, Without<crate::system::PendingAtmosphere>)>,
    meshes: Res<Assets<Mesh>>,
    mut mats: ResMut<Assets<AuroraMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut settings: ResMut<AuroraSettings>,
) {
    for (entity, body, terrain) in &bodies {
        commands.entity(entity).insert(AuroraChecked);
        let ObjectRef::Body(i) = body.obj else { continue };
        let rec = &data.catalog.bodies[i as usize];
        let extras = BodyExtras::from_body_block(&rec.block);
        let Some(a) = extras.aurora.filter(|_| !extras.no_aurora) else { continue };
        let a: AuroraParams = a;
        let radius_km = body.radius_m / 1000.0;
        // The base: the top of the atmosphere (sea level + its Height, FUN_1404543d0).
        let atm_km = rec.block.sub("Atmosphere").and_then(|b| b.f64("Height")).unwrap_or(0.0).max(0.0);
        let base_km = radius_km + atm_km;
        let bulk_class = terrain.map_or(bulk::TERRA, |t| t.surf_gen.classes.bulk);
        let mut seed = se_core::proc_surface::name_seed(&rec.names[0]) as u32;
        let ovals = [&a.north, &a.south].map(|o| SeOval::from_catalog(o, base_km, radius_km, bulk_class, &mut seed));
        // SE draws an aurora only when both ovals have particles.
        if ovals.iter().any(|o| o.particles == 0) {
            continue;
        }
        let (top_d, bottom_d) = default_colors(bulk_class);
        let c = |v: [f64; 3]| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32);
        let (top, bottom) = (c(a.top_color.unwrap_or(top_d)), c(a.bottom_color.unwrap_or(bottom_d)));
        // SE's Height default: 1.4 x the atmosphere's height (capped), when the block has none.
        let height_km = if a.height_km > 0.0 { a.height_km } else { atm_km * 1.4 };
        let noise = match settings.noise.and_then(|id| images.get_strong_handle(id)) {
            Some(h) => h,
            None => {
                let h = images.add(noise_image());
                settings.noise = Some(h.id());
                h
            }
        };
        // North then south, the south starting where the north's particles left the seed.
        let mut pseed = seed;
        for oval in ovals {
            let particle_seed = pseed;
            skip_particles(&oval, &mut pseed);
            let material = mats.add(AuroraMaterial {
                u: AuroraUniform {
                    frame_x: Vec4::X,
                    frame_y: Vec4::Y,
                    frame_z: Vec4::Z,
                    scale_time: Vec4::ZERO,
                    top: Vec4::ZERO,
                    bottom: Vec4::ZERO,
                    sun_vec: Vec4::ZERO,
                },
                noise: noise.clone(),
            });
            commands.spawn((
                SystemMember,
                AuroraOf { body: entity, material: material.clone(), oval, base_km, height_km, top, bottom, particle_seed: Some(particle_seed) },
                Mesh3d(meshes.reserve_handle()),
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

/// SE's brightness and visibility of an oval seen from `cam_km` (body frame, km), with `px_per_rad`
/// pixels per radian (FUN_140451fe0): hidden below 2 pixels of oval radius, faded in up to 20, and
/// dimmed to 0.1 as the oval shrinks below 0.2 of its distance. 0 when hidden.
pub fn oval_brightness(o: &SeOval, base_km: f64, cam_km: DVec3, px_per_rad: f64) -> f64 {
    let oval_r = o.radius.sin() * base_km;
    let dist = (cam_km - o.axis() * base_km).length().max(1e-6);
    let px = oval_r * px_per_rad / (dist * dist - oval_r * oval_r).max(1.0).sqrt();
    let x = ((oval_r / dist - 0.2) / 0.8).clamp(0.0, 1.0);
    let fade = (3.0 - 2.0 * x) * x * x * 0.9 + 0.1;
    if px <= 2.0 {
        return 0.0;
    }
    fade * if px < 20.0 { (px - 2.0) / 18.0 } else { 1.0 }
}

/// The flash phase (ScaleTime.w): FlashFreq x t wrapped into +-1000, / 10, with t SE's clock in days
/// since J2000 (FUN_140451fe0; the epoch DAT_140855480 is set to JD 2451545 by FUN_1405d30b0). In
/// seconds it ran 86400 times too fast and the noise it moves changed every frame: a flicker.
fn flash_phase(flash_freq: f64, jd: f64) -> f64 {
    let ft = flash_freq * (jd - 2_451_545.0);
    (ft - 2000.0 * (ft / 2000.0).round()) / 10.0
}

#[allow(clippy::too_many_arguments)]
fn update_auroras(
    cam: Res<crate::camera::FlyCamera>,
    sim: Res<SimTime>,
    windows: Query<&Window>,
    cams: Query<&Projection, With<crate::camera::MainCamera>>,
    bodies: Query<(&SystemBody, &BodyLighting)>,
    mut auroras: Query<(&mut AuroraOf, &Mesh3d, &mut UPosition, &mut UOrientation, &mut ScaleAnchor, &mut Visibility)>,
    mut mats: ResMut<Assets<AuroraMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    switches: Res<crate::switches::RenderSwitches>,
    settings: Res<AuroraSettings>,
) {
    let h = windows.iter().next().map(|w| w.height() as f64).unwrap_or(900.0);
    let fov = cams
        .iter()
        .next()
        .and_then(|p| match p {
            Projection::Perspective(p) => Some(p.fov as f64),
            _ => None,
        })
        .unwrap_or(1.0);
    let px_per_rad = h * 0.5 / (fov * 0.5).tan();
    for (mut aurora, mesh, mut pos, mut rot, mut anchor, mut vis) in &mut auroras {
        let Ok((b, light)) = bodies.get(aurora.body) else { continue };
        let cam_km = b.from_shape(b.orient.inverse() * cam.pos.delta_m(b.pos)) / 1000.0;
        let k = if switches.aurora { oval_brightness(&aurora.oval, aurora.base_km, cam_km, px_per_rad) } else { 0.0 };
        let v = if k > 0.0 { Visibility::Visible } else { Visibility::Hidden };
        vis.set_if_neq(v);
        if k <= 0.0 {
            continue;
        }
        // Shown for the first time: build its particles now, so it is never drawn without them.
        if let Some(mut seed) = aurora.particle_seed {
            aurora.particle_seed = None;
            let (p, d) = oval_particles(&aurora.oval, &mut seed);
            meshes.insert(mesh.id(), particle_mesh(p, d)).expect("reserved aurora mesh handle");
        }
        let aurora = &*aurora;
        let o = &aurora.oval;
        pos.0 = b.pos;
        rot.0 = b.orient;
        anchor.0 = b.pos;
        // The oval's frame (FUN_140441140(lon, lat - pi/2)): y its axis.
        let y = o.axis();
        let x = DVec3::new(-o.lon.sin(), 0.0, -o.lon.cos());
        let x = (x - y * x.dot(y)).normalize_or(y.any_orthonormal_vector());
        let z = x.cross(y);
        let base_m = aurora.base_km * 1000.0;
        // SE's sprite width: the ring's circumference over the particles, x 200000 (km), here in
        // base radii.
        let width = TAU * o.radius.sin() * 200_000.0 / o.particles.max(1) as f64 / aurora.base_km;
        let gain = (o.width / (o.rings + 1) as f64).sin() * 20.0;
        let phase = flash_phase(o.flash_freq, sim.jd);
        // The pattern's drift: MoveSpeed along the star's direction in the oval's plane.
        let sun_body = b.orient.inverse() * light.sun_dir.as_dvec3();
        let s2 = DVec3::new(sun_body.dot(x), 0.0, sun_body.dot(z));
        let drift = s2.normalize_or_zero() * o.move_speed;
        // Bright x AuroraBright x 200 / 10000 x hdrAuroraBright (SE's photo-mode factors 1).
        let bright = (o.bright * k * 0.02) as f32 * switches.aurora_bright * settings.hdr_bright;
        let u = AuroraUniform {
            frame_x: x.as_vec3().extend(base_m as f32),
            frame_y: y.as_vec3().extend(0.0),
            frame_z: z.as_vec3().extend(0.0),
            scale_time: Vec4::new(width as f32, (aurora.height_km / aurora.base_km) as f32, gain as f32, phase as f32),
            top: aurora.top.extend(0.0) * bright,
            bottom: aurora.bottom.extend(0.0) * bright,
            sun_vec: Vec4::new(drift.x as f32, drift.z as f32, if settings.quality >= 1 { 1.0 } else { 0.0 }, 0.0),
        };
        if let Some(mut m) = mats.get_mut(&aurora.material) {
            m.u = u;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earth_north() -> AuroraOval {
        AuroraOval {
            lat_deg: 82.0,
            lon_deg: -113.0,
            radius_km: 2500.0,
            width_km: 600.0,
            rings: 3,
            bright: Some(0.3),
            flash_freq: None,
            move_speed: None,
            particles: None,
        }
    }

    #[test]
    fn earth_gets_se_defaults() {
        let mut seed = 7;
        let o = SeOval::from_catalog(&earth_north(), 6438.0, 6378.0, bulk::TERRA, &mut seed);
        assert_eq!(o.particles, 50000);
        assert!((o.radius - (2500.0f64 / 6438.0).asin()).abs() < 1e-12);
        assert!((50.0..90.0).contains(&o.flash_freq));
        assert!((0.5..2.0).contains(&o.move_speed));
        assert_eq!(o.rings, 3);
    }

    #[test]
    fn particles_fill_the_rings() {
        let mut seed = 7;
        let o = SeOval::from_catalog(&earth_north(), 6438.0, 6378.0, bulk::TERRA, &mut seed);
        let (pos, data) = oval_particles(&o, &mut seed);
        assert_eq!(pos.len(), 50000 * 4);
        assert_eq!(data.len(), pos.len());
        // Every particle sits on a ring within the oval's band around +y.
        let lo = (o.radius - o.width * 0.5).cos() as f32;
        let hi = (o.radius + o.width * 0.5).cos() as f32;
        assert!(pos.iter().all(|p| p[1] <= lo + 1e-4 && p[1] >= hi - 1e-4));
        // Corners in SE's order.
        assert_eq!(data[..4].iter().map(|d| (d[3] * 255.0).round() as u32).collect::<Vec<_>>(), [0, 64, 128, 192]);
    }

    fn mesh_bytes(m: &Mesh) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = m.attributes().map(|(_, v)| v.get_bytes().to_vec()).collect();
        out.push(match m.indices() {
            Some(Indices::U32(i)) => i.iter().flat_map(|v| v.to_le_bytes()).collect(),
            other => panic!("{other:?}"),
        });
        out
    }

    #[test]
    fn lazy_ovals_build_the_eager_meshes() {
        let south = AuroraOval { lat_deg: -80.0, rings: 0, ..earth_north() };
        for (north, south) in [(earth_north(), south.clone()), (AuroraOval { rings: 0, ..earth_north() }, south)] {
            let mut seed = 0x1234_5678;
            let ovals = [&north, &south].map(|o| SeOval::from_catalog(o, 6438.0, 6378.0, bulk::TERRA, &mut seed));
            // As the ovals were built at spawn: north then south from one seed.
            let mut eager_seed = seed;
            let eager = ovals.map(|o| {
                let (p, d) = oval_particles(&o, &mut eager_seed);
                mesh_bytes(&particle_mesh(p, d))
            });
            // Each from the seed it was given at spawn, south first.
            let mut pseed = seed;
            let seeds = ovals.map(|o| {
                let s = pseed;
                skip_particles(&o, &mut pseed);
                s
            });
            assert_eq!(pseed, eager_seed);
            for i in [1, 0] {
                let (p, d) = oval_particles(&ovals[i], &mut { seeds[i] });
                assert!(mesh_bytes(&particle_mesh(p, d)) == eager[i], "oval {i}");
            }
        }
    }

    #[test]
    fn flash_phase_moves_slowly() {
        // SE's clock is in days since J2000: 0 there, FlashFreq / 10 a day later.
        assert_eq!(flash_phase(70.0, 2_451_545.0), 0.0);
        assert!((flash_phase(70.0, 2_451_546.0) - 7.0).abs() < 1e-6);
        // A frame at 60 fps moves the noise it slides by a sliver of a texel (0.37 x phase in
        // texture units, 256 texels), not a new random value every frame.
        let jd = 2_461_346.6;
        let step = (flash_phase(70.0, jd + 1.0 / 60.0 / 86_400.0) - flash_phase(70.0, jd)).abs();
        assert!(step * 0.37 * 256.0 < 0.01, "{step}");
    }

    #[test]
    fn ovals_without_a_radius_keep_the_particle_budget() {
        // An add-on Aurora block with Width but no NorthRadius: radius 0 (c0 just below 0 in f32),
        // and one with a radius under half its width.
        for radius in [0.0, 0.01] {
            let o = SeOval { radius, width: 0.1, rings: 4, particles: 1000, ..SeOval::from_catalog(&earth_north(), 6438.0, 6378.0, bulk::TERRA, &mut 7) };
            let (pos, data) = oval_particles(&o, &mut 7);
            assert_eq!((pos.len(), data.len()), (4000, 4000), "radius {radius}");
            assert!(pos.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn small_ovals_are_hidden() {
        let o = SeOval::from_catalog(&earth_north(), 6438.0, 6378.0, bulk::TERRA, &mut 7);
        let far = DVec3::new(0.0, 4e8, 0.0);
        assert_eq!(oval_brightness(&o, 6438.0, far, 1000.0), 0.0);
        // Beyond five oval radii SE keeps a tenth of the brightness; inside 1.25 radii all of it.
        let mid = o.axis() * 40_000.0;
        assert!((oval_brightness(&o, 6438.0, mid, 1000.0) - 0.1).abs() < 1e-9);
        let near = o.axis() * (6438.0 + 1000.0);
        assert!(oval_brightness(&o, 6438.0, near, 1000.0) > 0.99);
    }
}
