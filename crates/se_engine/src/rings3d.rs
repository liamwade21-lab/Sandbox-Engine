//! Volumetric rings: SE's 3D ring pass (`VolumetricRings`, FUN_1404e2bd0 with rings_raymarch.glsl),
//! drawn within `VolumDrawDist` of a ring's volume on top of the flat ring (`rings.rs`), which then
//! leaves the nearest 0.95 `VolumDrawDist` to it.
//!
//! - **Subrings** (FUN_1402e9c50): the ring is cut into `max(256, texture width)` equal subrings, at
//!   least 5 `RocksMaxSize` wide, a multiple of 256 and at most 16384 (SE's GL texture limit; assumed
//!   value). Subring `i` takes the alphas of the ring texture's texel `i` (clamped to the texture, as
//!   SE does): its rocks fill `sqrt(front alpha)` of the slab's height and are spaced
//!   `RocksSpacing / max(front alpha, 1e-5)^2` (no rocks beyond `RingRockMaxSpacing`). Its bounds move
//!   by `SubringInner/OuterRadiusAdjust`.
//! - **Rotation** (FUN_1402e9830): every subring's grid turns at the Kepler rate of its middle
//!   radius (SE: relative to the ring's own `RotationPeriod` frame; the port: relative to the planet's
//!   spinning frame, which its rings are drawn in), alternate layers `SubringLayerRotationAdjust`
//!   (55 degrees) either side, as double-precision cosines and sines split in two floats.
//! - **Scale**: a cell is `RingRockScale` (1) x `RocksMaxSize` across; the rocks fade out between
//!   `RingRocksFadeStartSizePix` (3) and `RingRocksFadeEndSizePix` (1.5) pixels across a cell, the
//!   dust between 0.95 and 1 `VolumDrawDist`.
//! - **Rocks**: `DefaultRingsRockTexture` ("SnowSE001") from textures/terrain/materials (its `_COL`,
//!   `_NORM` and `_RHAO` maps), read from the install.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use se_core::ephemeris::Ephemeris;
use se_core::rings::{RingVolume, kepler_period_days};

use crate::camera::{FlyCamera, MainCamera};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::eclipse::{EclipseCasters, EclipseLight};
use crate::rings::{RingMaterial, Rings};
use crate::system::{BodyLighting, MAX_LIGHTS, SystemBody, SystemMember};
use crate::universe::{SimTime, UOrientation, UPosition, UniverseSet};

/// SE's largest subring count (its MarchMap is a texture as wide as the GPU allows; assumed 16384).
const MAX_SUBRINGS: u32 = 16384;
const EARTH_MASS_KG: f64 = 5.972_2e24;
const J2000: f64 = 2_451_545.0;

/// main-def.cfg `PlanetRendering` `VolumetricRings` and the `RingsRendering` keys.
#[derive(Resource, Clone, Debug)]
pub struct RingSettings {
    pub volumetric: bool,
    pub fade_start_px: f32,
    pub fade_end_px: f32,
    pub inner_adjust: f32,
    pub outer_adjust: f32,
    pub layer_rotation_deg: f64,
    pub rock_max_spacing: f32,
    /// `RingRockScale` (SE's config variable 0x1419bf420, 1; not in main-def.cfg).
    pub rock_scale: f32,
    pub rock_texture: String,
    /// `hdrRingDustBright` (1) and `hdrRingRocksBright` (5): the dust's and the rocks' light (SE's
    /// LightingParams take them with the ring's `Brightness`; the flat ring has `hdrPlanetRingsBright`).
    pub dust_bright: f32,
    pub rocks_bright: f32,
}

impl Default for RingSettings {
    fn default() -> Self {
        Self {
            volumetric: true,
            fade_start_px: 3.0,
            fade_end_px: 1.5,
            inner_adjust: 0.0,
            outer_adjust: 0.0,
            layer_rotation_deg: 55.0,
            rock_max_spacing: 10000.0,
            rock_scale: 1.0,
            rock_texture: "SnowSE001".into(),
            dust_bright: 1.0,
            rocks_bright: 5.0,
        }
    }
}

#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct Rings3DUniform {
    pub ro_hi: Vec4,
    pub ro_lo: Vec4,
    pub to_ring0: Vec4,
    pub to_ring1: Vec4,
    pub to_ring2: Vec4,
    pub rings_size: Vec4,
    pub planet: Vec4,
    pub obl: Vec4,
    pub hapke: Vec4,
    pub hapke2: Vec4,
    pub density: Vec4,
    pub lighting: Vec4,
    pub subring: Vec4,
    pub tex_sample: Vec4,
    pub fade: Vec4,
    pub scale: Vec4,
    pub light_dir: [Vec4; MAX_LIGHTS],
    pub light_color: [Vec4; MAX_LIGHTS],
    pub host: Vec4,
    pub eclipse: [EclipseLight; MAX_LIGHTS],
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct Rings3DMaterial {
    #[uniform(0)]
    pub u: Rings3DUniform,
    #[storage(1, read_only)]
    pub march: Handle<ShaderBuffer>,
    #[texture(2)]
    #[sampler(3)]
    pub ring: Handle<Image>,
    #[texture(4, dimension = "2d_array")]
    #[sampler(5)]
    pub albedo: Handle<Image>,
    #[texture(6, dimension = "2d_array")]
    pub normal: Handle<Image>,
    #[texture(7, dimension = "2d_array")]
    pub rhao: Handle<Image>,
}

impl Material for Rings3DMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/rings3d.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/rings3d.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // SE's glBlendFunc(ONE, ONE_MINUS_SRC_ALPHA).
        AlphaMode::Premultiplied
    }
    fn depth_bias(&self) -> f32 {
        // After the flat ring (`RingMaterial`, SE draws the 3D pass second).
        1.1e10
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// A 2D ring's volume, once made.
#[derive(Component)]
struct HasVolume;

/// The 3D pass of one ring.
#[derive(Component)]
pub struct Rings3D {
    ring: Entity,
    body: Entity,
    material: Handle<Rings3DMaterial>,
    march: Handle<ShaderBuffer>,
    vol: RingVolume,
    inner_km: f64,
    outer_km: f64,
    count: u32,
    /// Each subring's grid rotation (rev/day, from x towards z in the ring frame).
    rates: Vec<f64>,
    /// Rows 2 and 3 of SE's MarchMap.
    rows23: Vec<[f32; 4]>,
    last_jd: f64,
}

/// The rock maps and the screen triangle, shared by every ring.
#[derive(Resource)]
struct RockAssets {
    albedo: Handle<Image>,
    normal: Handle<Image>,
    rhao: Handle<Image>,
    mesh: Handle<Mesh>,
}

pub struct Rings3DPlugin;

impl Plugin for Rings3DPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<Rings3DMaterial>::default())
            .init_resource::<RingSettings>()
            .add_systems(Startup, load_settings)
            .add_systems(Update, attach_volumes)
            .add_systems(
                PostUpdate,
                update_volumes
                    .after(crate::rings::update_rings)
                    .after(crate::eclipse::update_eclipse_casters)
                    .before(UniverseSet::FloatingOrigin),
            );
    }
}

fn load_settings(cfg: Res<EngineConfig>, mut s: ResMut<RingSettings>) {
    let Some(doc) = crate::galaxies::main_def_doc(&cfg) else { return };
    let num = |k: &str| doc.find_deep(k).and_then(|i| i.f64());
    let d = RingSettings::default();
    *s = RingSettings {
        volumetric: doc.find_deep("VolumetricRings").and_then(|i| i.bool()).unwrap_or(d.volumetric),
        fade_start_px: num("RingRocksFadeStartSizePix").map_or(d.fade_start_px, |v| v as f32),
        fade_end_px: num("RingRocksFadeEndSizePix").map_or(d.fade_end_px, |v| v as f32),
        inner_adjust: num("SubringInnerRadiusAdjust").map_or(d.inner_adjust, |v| v as f32),
        outer_adjust: num("SubringOuterRadiusAdjust").map_or(d.outer_adjust, |v| v as f32),
        layer_rotation_deg: num("SubringLayerRotationAdjust").unwrap_or(d.layer_rotation_deg),
        rock_max_spacing: num("RingRockMaxSpacing").map_or(d.rock_max_spacing, |v| v as f32),
        rock_scale: num("RingRockScale").map_or(d.rock_scale, |v| v as f32),
        rock_texture: doc.find_deep("DefaultRingsRockTexture").and_then(|i| i.str()).map_or(d.rock_texture, str::to_owned),
        dust_bright: num("hdrRingDustBright").map_or(d.dust_bright, |v| v as f32),
        rocks_bright: num("hdrRingRocksBright").map_or(d.rocks_bright, |v| v as f32),
    };
}

/// SE's subring count (FUN_1402e9c50, in its single precision).
fn subring_count(inner_km: f64, outer_km: f64, rocks_km: f64, tex_width: u32) -> u32 {
    let width = (outer_km - inner_km) as f32;
    let texels = tex_width.max(256) as f32;
    let sub = ((rocks_km * 5.0) as f32).max(width / texels);
    let n = ((width / sub).floor() as u32) & !0xff;
    n.clamp(256, MAX_SUBRINGS)
}

/// SE's MarchMap rows 2 and 3 for subring `i` (FUN_1402e9c50): its bounds (km) and how much of the
/// slab its rocks fill, from the texel's front and back alphas.
fn subring_rows(i: u32, count: u32, inner_km: f64, outer_km: f64, vol: &RingVolume, alpha: [f32; 2], s: &RingSettings) -> ([f32; 4], [f32; 4]) {
    let width = (outer_km - inner_km) as f32;
    let w = width / count as f32;
    let r0 = i as f32 * w + inner_km as f32;
    let r1 = r0 + w;
    let [front, back] = alpha;
    let fs = front.max(0.0).sqrt().min(1.0);
    let bs = back.clamp(0.0, 1.0);
    let f_min = front.clamp(1e-5, 1.0);
    let th = vol.thickness_km as f32;
    let size = [r0 + s.inner_adjust, r1 + s.outer_adjust, if front != 0.0 { fs * th } else { r1 - r0 }, if back != 0.0 { bs * th } else { r1 - r0 }];
    let dens = [fs, bs, front, 1.0 / (f_min * f_min)];
    (size, dens)
}

/// A double split in two floats (SE's df64: hi + lo).
fn split(x: f64) -> (f32, f32) {
    let hi = x as f32;
    (hi, (x - hi as f64) as f32)
}

/// The planet's spin about its frame's +Y axis (rev/day, signed), from its orientation a moment on.
fn spin_rev_per_day(eph: &Ephemeris, obj: se_core::catalog::ObjectRef, jd: f64) -> f64 {
    const DT: f64 = 0.01;
    let dq = eph.orientation(obj, jd).inverse() * eph.orientation(obj, jd + DT);
    let (axis, angle) = dq.normalize().to_axis_angle();
    if axis.y.abs() < 0.5 {
        return 0.0;
    }
    let a = if angle > std::f64::consts::PI { angle - std::f64::consts::TAU } else { angle };
    a * axis.y.signum() / DT / std::f64::consts::TAU
}

/// Builds a ring's 3D pass the first time it is drawn.
#[allow(clippy::too_many_arguments)]
fn attach_volumes(
    mut commands: Commands,
    rings: Query<(Entity, &Rings, &Visibility), Without<HasVolume>>,
    bodies: Query<&SystemBody>,
    data: Res<SeData>,
    sim: Res<SimTime>,
    settings: Res<RingSettings>,
    assets: Option<Res<RockAssets>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<Rings3DMaterial>>,
) {
    if !settings.volumetric {
        return;
    }
    let mut assets_now: Option<(Handle<Image>, Handle<Image>, Handle<Image>, Handle<Mesh>)> = None;
    for (e, ring, vis) in &rings {
        if *vis == Visibility::Hidden {
            continue;
        }
        let Ok(body) = bodies.get(ring.body) else { continue };
        commands.entity(e).insert(HasVolume);
        let rock = match (&assets, &assets_now) {
            (Some(a), _) => (a.albedo.clone(), a.normal.clone(), a.rhao.clone(), a.mesh.clone()),
            (None, Some(a)) => a.clone(),
            (None, None) => {
                let maps = crate::surface_tex::rock_maps(&data.vfs, &settings.rock_texture);
                let tri = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
                    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-1.0f32, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]]);
                let a = (images.add(maps[0].clone()), images.add(maps[1].clone()), images.add(maps[2].clone()), meshes.add(tri));
                commands.insert_resource(RockAssets { albedo: a.0.clone(), normal: a.1.clone(), rhao: a.2.clone(), mesh: a.3.clone() });
                assets_now = Some(a.clone());
                a
            }
        };
        let p = &ring.params;
        let mass_earth = crate::ship::body_mass(&data, body.obj, body.radius_m) / EARTH_MASS_KG;
        let vol = p.volume(mass_earth, ring.thickness_draw);
        let count = subring_count(p.inner_radius_km, p.outer_radius_km, vol.rocks_max_size_km * settings.rock_scale as f64, ring.alpha.len() as u32);
        let eph = Ephemeris::new(&data.catalog);
        let spin = spin_rev_per_day(&eph, body.obj, sim.jd);
        let prograde = if spin != 0.0 { spin.signum() } else { 1.0 };
        let mut rates = Vec::with_capacity(count as usize);
        let mut rows23 = vec![[0.0f32; 4]; 2 * count as usize];
        for i in 0..count {
            let k = (i as usize).min(ring.alpha.len().saturating_sub(1));
            let alpha = ring.alpha.get(k).copied().unwrap_or([1.0, 1.0]);
            let (size, dens) = subring_rows(i, count, p.inner_radius_km, p.outer_radius_km, &vol, alpha, &settings);
            rows23[i as usize] = size;
            rows23[(count + i) as usize] = dens;
            let mid = (size[0] + size[1]) as f64 * 0.5 - (settings.inner_adjust + settings.outer_adjust) as f64 * 0.5;
            rates.push(spin - prograde / kepler_period_days(mid, mass_earth));
        }
        let march = buffers.add(ShaderBuffer::new(vec![[0.0f32; 4]; 4 * count as usize], RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD));
        let material = materials.add(Rings3DMaterial {
            u: Rings3DUniform::default(),
            march: march.clone(),
            ring: ring.texture.clone(),
            albedo: rock.0,
            normal: rock.1,
            rhao: rock.2,
        });
        info!(
            "rings 3D: {count} subrings, cells {:.1} m, thickness {:.3} km, draw distance {:.0} km",
            vol.rocks_max_size_km * settings.rock_scale as f64 * 1000.0,
            vol.thickness_km,
            vol.volum_draw_dist_km
        );
        commands.spawn((
            SystemMember,
            Rings3D {
                ring: e,
                body: ring.body,
                material: material.clone(),
                march,
                vol,
                inner_km: p.inner_radius_km,
                outer_km: p.outer_radius_km,
                count,
                rates,
                rows23,
                last_jd: f64::NAN,
            },
            Mesh3d(rock.3),
            MeshMaterial3d(material),
            UPosition::default(),
            UOrientation::default(),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
        ));
    }
}

/// The camera's signed distance to a ring's volume (km): SE's test before its 3D pass.
fn volume_distance(cam_ring_km: DVec3, inner: f64, outer: f64, thickness: f64) -> f64 {
    let r = (cam_ring_km.x * cam_ring_km.x + cam_ring_km.z * cam_ring_km.z).sqrt();
    let radial = (inner - r).max(r - outer);
    let vertical = cam_ring_km.y.abs() - thickness;
    (radial.max(0.0).powi(2) + vertical.max(0.0).powi(2)).sqrt() + vertical.max(radial).min(0.0)
}

#[allow(clippy::too_many_arguments)]
fn update_volumes(
    mut vols: Query<(&mut Rings3D, &mut UPosition, &mut Visibility)>,
    rings: Query<(&Rings, &Visibility), Without<Rings3D>>,
    bodies: Query<(&SystemBody, &BodyLighting, Option<&EclipseCasters>)>,
    cam: Res<FlyCamera>,
    cams: Query<&Camera, With<MainCamera>>,
    sim: Res<SimTime>,
    settings: Res<RingSettings>,
    mut ring_mats: ResMut<Assets<RingMaterial>>,
    mut materials: ResMut<Assets<Rings3DMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut frame: Local<u32>,
) {
    *frame = frame.wrapping_add(1);
    let height_px = cams.iter().next().and_then(|c| c.physical_viewport_size()).map_or(1080.0, |s| s.y as f64);
    // SE's DAT_1419d5ab8: the focal length in pixels.
    let focal_px = height_px / (2.0 * ((cam.fov_deg as f64).to_radians() * 0.5).tan());
    for (mut v, mut pos, mut vis) in &mut vols {
        let Ok((ring, ring_vis)) = rings.get(v.ring) else { continue };
        let Ok((body, light, casters)) = bodies.get(v.body) else { continue };
        let r_km = body.radius_m / 1000.0;
        let scale_km = v.vol.rocks_max_size_km * settings.rock_scale as f64;
        let inv = 1.0 / scale_km;
        let q: DQuat = body.orient.normalize();
        let cam_ring_km = q.inverse() * cam.pos.delta_m(body.pos) / 1000.0;
        let dist = volume_distance(cam_ring_km, v.inner_km, v.outer_km, v.vol.thickness_km);
        // The flat ring leaves the nearest 0.95 VolumDrawDist (and the slab) to this pass.
        let fade3d = if settings.volumetric {
            Vec4::new((0.95 * v.vol.volum_draw_dist_km / r_km) as f32, (v.vol.thickness_km / r_km) as f32, 1.0, 0.0)
        } else {
            Vec4::ZERO
        };
        if ring_mats.get(&ring.material).is_some_and(|m| m.u.fade3d != fade3d)
            && let Some(mut m) = ring_mats.get_mut(&ring.material)
        {
            m.u.fade3d = fade3d;
        }
        let show = settings.volumetric && *ring_vis != Visibility::Hidden && dist <= v.vol.volum_draw_dist_km;
        *vis = if show { Visibility::Visible } else { Visibility::Hidden };
        if !show {
            continue;
        }
        pos.0 = body.pos;
        // Rows 0 and 1: the subrings' rotations, when the time moves.
        if v.last_jd != sim.jd {
            v.last_jd = sim.jd;
            let t = sim.jd - J2000;
            let adj = settings.layer_rotation_deg.to_radians();
            let mut buf = buffers.get_mut(&v.march);
            if let Some(data) = buf.as_mut().and_then(|b| b.cast_slice_mut::<[f32; 4]>()) {
                let n = v.count as usize;
                for (i, &rate) in v.rates.iter().enumerate() {
                    let turn = t * rate;
                    let a = (turn - turn.floor()) * std::f64::consts::TAU - adj;
                    let b = a + 2.0 * adj;
                    let ((c1, c1l), (s1, s1l)) = (split(a.cos()), split(a.sin()));
                    let ((c2, c2l), (s2, s2l)) = (split(b.cos()), split(b.sin()));
                    data[i] = [c1, s1, c1l, s1l];
                    data[n + i] = [c2, s2, c2l, s2l];
                }
                data[2 * n..].copy_from_slice(&v.rows23);
            }
        }
        let Some(rm) = ring_mats.get(&ring.material).map(|m| m.u) else { continue };
        let Some(mut m) = materials.get_mut(&v.material) else { continue };
        let cam_cells = cam_ring_km * inv;
        let (hx, lx) = split(cam_cells.x);
        let (hy, ly) = split(cam_cells.y);
        let (hz, lz) = split(cam_cells.z);
        let to_ring = Mat3::from_quat(q.inverse().as_quat());
        let rows = to_ring.transpose();
        let u = &mut m.u;
        u.ro_hi = Vec4::new(hx, hy, hz, 1.0);
        u.ro_lo = Vec4::new(lx, ly, lz, *frame as f32);
        let eclipse_k = rm.hapke2.w;
        u.to_ring0 = rows.x_axis.extend(eclipse_k);
        u.to_ring1 = rows.y_axis.extend(0.0);
        u.to_ring2 = rows.z_axis.extend(0.0);
        let half = v.vol.thickness_km * inv;
        u.rings_size = Vec4::new((v.inner_km * inv) as f32, (v.outer_km * inv) as f32, half as f32, (1.0 / half) as f32);
        u.planet = Vec4::new((r_km * inv) as f32, (r_km * inv) as f32, (scale_km / r_km) as f32, v.vol.rocks_spacing as f32);
        let shape = body.shape.max(DVec3::splat(1e-3));
        u.obl = Vec4::new((1.0 / shape.x) as f32, (1.0 / shape.y) as f32, (1.0 / shape.z) as f32, 0.0);
        u.hapke = rm.hapke;
        u.hapke2 = Vec4::new(rm.hapke2.x, (v.outer_km * inv) as f32, rm.hapke2.y, (scale_km * 0.05 / focal_px) as f32);
        // SE's DensityParams.w is Opacity x SelfShadow.
        u.density = Vec4::new(rm.density.x, rm.density.y, rm.density.z, rm.density.z * rm.density.w);
        let bright = rm.geom.w;
        let (dust, rocks) = (bright * settings.dust_bright, bright * settings.rocks_bright);
        u.lighting = Vec4::new(rm.params.x * dust, rm.params.y * dust, rocks, 0.0);
        u.subring = Vec4::new(settings.inner_adjust, settings.outer_adjust, v.count as f32, settings.rock_max_spacing);
        let w = (v.outer_km - v.inner_km) * inv;
        u.tex_sample = Vec4::new((1.0 / w) as f32, (-v.inner_km * inv / w) as f32, (v.count as f64 / w) as f32, (-v.inner_km * inv * v.count as f64 / w) as f32);
        let fade_px = |px: f32| 0.5 / (px as f64 / focal_px).tan();
        let draw = v.vol.volum_draw_dist_km * inv;
        u.fade = Vec4::new(fade_px(settings.fade_start_px) as f32, fade_px(settings.fade_end_px) as f32, (0.95 * draw) as f32, draw as f32);
        u.scale = Vec4::new(scale_km as f32, inv as f32, (scale_km * 1000.0) as f32, 0.0);
        // The stars, as the flat ring has them (their light includes the rings' fade).
        let eclipse = casters.map_or_else(Default::default, |c| c.lights);
        let to_r = |d: Vec3| (to_ring * d).normalize_or_zero();
        u.light_dir = [Vec4::ZERO; MAX_LIGHTS];
        u.light_color = [Vec4::ZERO; MAX_LIGHTS];
        if light.rings_intensity > 0.0 {
            u.light_dir[0] = to_r(light.sun_dir).extend(eclipse[0].info.x.max(1e-7));
            u.light_color[0] = (light.sun_color * light.rings_intensity).extend(1.0);
        }
        for k in 0..MAX_LIGHTS - 1 {
            if rm.light_dir[k].w > 0.5 {
                u.light_dir[k + 1] = to_r(rm.light_dir[k].truncate()).extend(eclipse[k + 1].info.x.max(1e-7));
                u.light_color[k + 1] = rm.light_color[k].truncate().extend(1.0);
            }
        }
        u.host = rm.host;
        u.eclipse = eclipse;
    }
}

/// The front and back alphas of a decoded ring texture's texels (its last and first rows, SE's
/// FrontMap and BackMap).
pub fn texture_alphas(img: &Image) -> Arc<Vec<[f32; 2]>> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let Some(px) = img.data.as_ref() else { return Arc::new(Vec::new()) };
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return Arc::new(Vec::new());
    }
    let a = |x: usize, y: usize| px[(y * w + x) * 4 + 3] as f32 / 255.0;
    Arc::new((0..w).map(|x| [a(x, h - 1), a(x, 0)]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subrings_follow_se() {
        // Saturn: a 2048-texel texture and 8 m rocks give one subring per texel.
        assert_eq!(subring_count(64535.0, 318120.0, 0.008, 2048), 2048);
        // A narrow ring of big rocks: at least 256.
        assert_eq!(subring_count(1000.0, 1010.0, 0.5, 64), 256);
        // Rows 2 and 3: bounds, slab share and spacing from the alphas.
        let vol = RingVolume { thickness_km: 0.2, rocks_max_size_km: 0.008, rocks_spacing: 1.0, volum_draw_dist_km: 600.0, rotation_period_days: 0.5 };
        let (size, dens) = subring_rows(1, 256, 1000.0, 1256.0, &vol, [0.25, 0.5], &RingSettings::default());
        assert_eq!(size, [1001.0, 1002.0, 0.1, 0.1]);
        assert_eq!(dens, [0.5, 0.5, 0.25, 16.0]);
    }

    #[test]
    fn camera_distance_to_the_ring_volume() {
        // Above the ring: the height beyond the half thickness; inside: negative.
        assert!((volume_distance(DVec3::new(1100.0, 10.0, 0.0), 1000.0, 1200.0, 0.5) - 9.5).abs() < 1e-9);
        assert!(volume_distance(DVec3::new(0.0, 0.0, 1100.0), 1000.0, 1200.0, 0.5) < 0.0);
        // In the gap, level with the ring: the radial distance to the inner edge.
        assert!((volume_distance(DVec3::new(900.0, 0.0, 0.0), 1000.0, 1200.0, 0.5) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn split_keeps_the_double() {
        let x = 39_765_432.123_456_789;
        let (hi, lo) = split(x);
        assert!(((hi as f64 + lo as f64) - x).abs() < 1e-6);
    }
}
