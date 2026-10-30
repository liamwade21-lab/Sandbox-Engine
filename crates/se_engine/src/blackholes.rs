//! Black holes: gravitational lensing, volumetric accretion disks and jets.
//!
//! Everything is a single HDR post-process on the main camera (see
//! `assets/shaders/blackhole_lens.wesl` and `docs/blackholes.md`), SE's einstein.glsl in its
//! single-pass mode. Each frame this module finds the catalog black holes that are big enough
//! on screen to matter, converts them to camera-space uniforms (direction, distance in
//! Schwarzschild radii, disk frame, SE's DiskParams/JetParams) and attaches them to the camera
//! as a [`BlackHoleLens`] component. The shader traces light rays through the Schwarzschild
//! geometry and ray-marches the disk and jet gas along them.
//!
//! Black holes are not spawned as `SystemBody` entities (their catalog radius is 0), so this
//! plugin evaluates them straight from the catalog and ephemeris.

#[path = "blackhole_params.rs"]
pub mod params;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::star::StarKind;
use se_core::units::SOLAR_MASS_KG;

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::universe::{SimTime, UniverseSet};
use params::DiskParams;

/// Maximum number of simultaneously lensed black holes.
const MAX_HOLES: usize = 4;
/// Schwarzschild radius per solar mass, km (SE: 8.87005e-6 km per Earth mass, FUN_1404dcd30).
const RS_PER_MSUN_KM: f64 = 2.953_25;
/// Mass assumed for catalog black holes that have none.
const DEFAULT_MASS_SUN: f64 = 10.0;
/// Smallest on-screen influence radius (pixels) worth a lens pass.
const MIN_REGION_PX: f64 = 1.5;
/// SE's density scale: `Density x Rg(km) x 6.4e7` (_DAT_1406d5c90, FUN_140270900).
const DENSITY_SCALE: f64 = 6.4e7;
/// Entries of SE's Planck table (FUN_1401a60d0).
const PLANCK_SIZE: usize = 4096;
/// The main texture of an HDR view.
const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// Per-hole shader data. Layout must match `Hole` in accretion_disk.wesl.
#[derive(Clone, Copy, ShaderType, Default, Debug)]
pub struct HoleUniform {
    pub dir_d: Vec4,
    pub axis_in: Vec4,
    pub xref_out: Vec4,
    pub params: Vec4,
    pub disk1: Vec4,
    pub disk2: Vec4,
    pub disk3: Vec4,
    pub disk4: Vec4,
    pub disk5: Vec4,
    pub jet1: Vec4,
    pub jet2: Vec4,
    pub jet3: Vec4,
    pub light: Vec4,
}

/// Camera component driving the lensing post-process.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug)]
#[extract_app(RenderApp)]
pub struct BlackHoleLens {
    /// x, y: tan(fov/2) horizontally / vertically, z: frame counter, w: hole count.
    pub view: Vec4,
    /// Columns of the view -> environment rotation; `sky0.w` 1 when the environment map is valid.
    pub sky0: Vec4,
    pub sky1: Vec4,
    pub sky2: Vec4,
    pub holes: [HoleUniform; MAX_HOLES],
}

/// SE's black hole settings from main-def.cfg.
#[derive(Resource, Clone, Debug)]
pub struct BlackHoleSettings {
    /// `hdrAccretionDiskBright` (10000).
    pub disk_bright: f64,
    /// `hdrAccretionJetBright` (10000).
    pub jet_bright: f64,
    /// `BlackHoleWarpRadius` (100): render sphere radius in Rg.
    pub warp_radius: f64,
    /// `WarpPrecision` (0.5): SE's warpStep is 0.2 x 10^(1 - WarpPrecision).
    pub warp_precision: f64,
    /// `EnvmapDimensions` (512): environment map face size, pixels.
    pub env_size: u32,
}

impl Default for BlackHoleSettings {
    fn default() -> Self {
        Self { disk_bright: 10_000.0, jet_bright: 10_000.0, warp_radius: 100.0, warp_precision: 0.5, env_size: 512 }
    }
}

impl BlackHoleSettings {
    fn warp_step(&self) -> f64 {
        0.2 * 10f64.powf(1.0 - self.warp_precision.clamp(0.0, 1.0))
    }
}

fn load_settings(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<BlackHoleSettings>) {
    let get = |k: &str| crate::stars::main_def_f64(&cfg, k);
    let d = BlackHoleSettings::default();
    *s = BlackHoleSettings {
        disk_bright: get("hdrAccretionDiskBright").unwrap_or(d.disk_bright),
        jet_bright: get("hdrAccretionJetBright").unwrap_or(d.jet_bright),
        warp_radius: get("BlackHoleWarpRadius").unwrap_or(d.warp_radius),
        warp_precision: get("WarpPrecision").unwrap_or(d.warp_precision),
        env_size: get("EnvmapDimensions").map_or(d.env_size, |v| (v as u32).clamp(64, 2048)),
    };
}

/// The shader's lookup textures (SE's PlanckFunction, NoiseIqMap) and the environment map's
/// six cube faces (+x -x +y -y +z -z, world axes).
#[derive(Resource, ExtractResource, Clone)]
#[extract_app(RenderApp)]
pub struct BlackHoleImages {
    pub planck: Handle<Image>,
    pub noise: Handle<Image>,
    pub env: [Handle<Image>; 6],
}

/// Forward and up axes of the environment cube faces (matching `sample_env` in the shader).
const ENV_FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::NEG_Z),
    (Vec3::NEG_Y, Vec3::Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

/// SE's environment map (seEnvMapManager, `EnvmapDimensions`): the sky around the camera,
/// rendered into six cube faces by six cameras, one face per frame while a hole is lensed (all
/// six at once when the map is (re)started). The lens shader takes rays that leave the screen
/// from it.
#[derive(Resource, Default)]
struct EnvCapture {
    cams: Vec<Entity>,
    size: u32,
    /// Bit i: face i holds the sky seen from `origin`.
    valid: u8,
    next: usize,
    /// Camera position the faces were rendered at, and the distance to the lensed hole then.
    origin: Option<se_core::upos::UPos>,
    hole_dist_m: f64,
    /// Set by `update_lens`: some lensed ray can reach the environment map this frame.
    want: bool,
    /// Set by `update_lens`: the faces are stale, so all six render this frame.
    restart: bool,
}

/// Catalog star indices of the holes the lens draws this frame (their disks are on screen as
/// volumes, so the star field leaves out their points).
#[derive(Resource, Default)]
pub struct LensedHoles(pub Vec<u32>);

/// Marks an environment-map camera (face index).
#[derive(Component)]
struct EnvFaceCam(usize);

/// Render layer of world-space geometry that belongs to the main view only (the lens flare
/// quads): the main camera sees layers 0 and 1, the environment cameras layer 0.
pub const MAIN_VIEW_ONLY: RenderLayers = RenderLayers::layer(1);

fn main_camera_layers(mut commands: Commands, cams: Query<Entity, (With<MainCamera>, Without<RenderLayers>)>) {
    for e in &cams {
        commands.entity(e).insert(RenderLayers::from_layers(&[0, 1]));
    }
}

/// Static, camera independent description of one catalog black hole.
struct BhInfo {
    star: u32,
    /// Catalog position (parsecs); only meaningful if `has_pos`.
    pos_pc: DVec3,
    has_pos: bool,
    rs_km: f64,
    spin: f64,
    disk: Option<DiskParams>,
    /// Disk normal used when the catalog gives no rotation model.
    default_axis: DVec3,
}

#[derive(Resource, Default)]
struct BhList {
    n_stars: usize,
    list: Vec<BhInfo>,
}

pub struct BlackHolesPlugin;

impl Plugin for BlackHolesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BhList>()
            .init_resource::<BlackHoleSettings>()
            .init_resource::<EnvCapture>()
            .init_resource::<LensedHoles>()
            .add_systems(Startup, (load_settings, init_images).chain())
            .add_systems(
                PostUpdate,
                (update_lens, update_env_cams).chain().after(UniverseSet::Camera).before(UniverseSet::FloatingOrigin),
            )
            .add_systems(Update, main_camera_layers)
            .add_plugins((
                ExtractComponentPlugin::<BlackHoleLens>::default(),
                UniformComponentPlugin::<BlackHoleLens>::default(),
                ExtractResourcePlugin::<BlackHoleImages>::default(),
            ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Core3d, lens_pass.in_set(Core3dSystems::EarlyPostProcess));
    }
}

fn hash_unit(seed: u64, salt: u64) -> f64 {
    let mut x = seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

// ---------------------------------------------------------------------------
// SE's Planck table and noise texture

/// Wyman, Sloan and Shirley's analytic fit of the CIE 1931 colour matching functions.
fn cie_xyz(lambda_nm: f64) -> [f64; 3] {
    let g = |l: f64, mu: f64, s1: f64, s2: f64| {
        let t = (l - mu) / if l < mu { s1 } else { s2 };
        (-0.5 * t * t).exp()
    };
    let l = lambda_nm;
    [
        1.056 * g(l, 599.8, 37.9, 31.0) + 0.362 * g(l, 442.0, 16.0, 26.7) - 0.065 * g(l, 501.1, 20.4, 26.2),
        0.821 * g(l, 568.8, 46.9, 40.5) + 0.286 * g(l, 530.9, 16.3, 31.1),
        1.217 * g(l, 437.0, 11.8, 36.0) + 0.681 * g(l, 459.0, 26.0, 13.8),
    ]
}

/// Planck's law, SE's constants (2hc^2 = 1.19104e-16, hc/k = 0.0143877519), wavelength in m.
fn planck_b(lambda_m: f64, t: f64) -> f64 {
    1.191_042_805_64e-16 / (lambda_m.powi(5) * ((0.014_387_751_9 / (lambda_m * t)).exp() - 1.0))
}

/// SE's blackbody radiance in linear sRGB (FUN_1401a5670 + FUN_1401a5560): the Planck spectrum
/// at 380..780 nm in 5 nm steps against the colour matching functions, relative to the
/// spectral radiance at 555 nm of a 5700 K body, through the sRGB (D65) matrix, negatives
/// clipped.
pub fn planck_rgb(t: f64) -> [f64; 3] {
    let norm = 1.0 / planck_b(555e-9, 5700.0);
    let mut xyz = [0.0; 3];
    for k in 0..81 {
        let l = 380.0 + 5.0 * k as f64;
        let b = planck_b(l * 1e-9, t.max(1.0));
        let c = cie_xyz(l);
        for i in 0..3 {
            xyz[i] += b * c[i];
        }
    }
    let [x, y, z] = xyz.map(|v| v * norm);
    [
        (3.2406 * x - 1.5372 * y - 0.4986 * z).max(0.0),
        (-0.9689 * x + 1.8758 * y + 0.0415 * z).max(0.0),
        (0.0557 * x - 0.2040 * y + 1.0570 * z).max(0.0),
    ]
}

/// SE's PlanckFunction texture (FUN_1401a60d0): entry i holds the radiance at
/// `T = 1000 exp(i / 4096 / 0.188 - 0.7)` (500 K .. 101 kK), raised below 2000 K by
/// `1000^(1 - (T - 500) / 1500)`.
pub fn planck_table() -> Vec<[f32; 4]> {
    (0..PLANCK_SIZE)
        .map(|i| {
            let t = 1000.0 * ((i as f64 / PLANCK_SIZE as f64) / 0.188 - 0.7).exp();
            let mut c = planck_rgb(t);
            if t < 2000.0 {
                let k = 1000f64.powf(1.0 - (t - 500.0) / 1500.0);
                c = c.map(|v| v * k);
            }
            [c[0] as f32, c[1] as f32, c[2] as f32, 1.0]
        })
        .collect()
}

/// SE's 16 x 16 NoiseIqMap: random R and B, with G and A the R and B values of the texel
/// (-11, -5) away, so that `mix(res.yw, res.xz, f.z)` interpolates between noise layers.
pub fn noise_texture(seed: u64) -> Vec<[u8; 4]> {
    let v = |x: i32, y: i32, c: u64| {
        let i = (x.rem_euclid(16) + 16 * y.rem_euclid(16)) as u64;
        (hash_unit(seed ^ (i * 4 + c), 0x5EED) * 255.0).round() as u8
    };
    let mut out = Vec::with_capacity(256);
    for y in 0..16 {
        for x in 0..16 {
            out.push([v(x, y, 0), v(x - 11, y - 5, 0), v(x, y, 1), v(x - 11, y - 5, 1)]);
        }
    }
    out
}

fn init_images(mut commands: Commands, mut images: ResMut<Assets<Image>>, settings: Res<BlackHoleSettings>, mut env: ResMut<EnvCapture>) {
    let size = |w, h| Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let planck: Vec<u8> = planck_table().iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    let planck = Image::new(size(PLANCK_SIZE as u32, 1), TextureDimension::D2, planck, TextureFormat::Rgba32Float, RenderAssetUsages::RENDER_WORLD);
    let noise: Vec<u8> = noise_texture(0x1A_9E37).into_iter().flatten().collect();
    let mut noise = Image::new(size(16, 16), TextureDimension::D2, noise, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    noise.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    env.size = settings.env_size;
    let faces = std::array::from_fn(|_| images.add(Image::new_target_texture(env.size, env.size, HDR_FORMAT, None)));
    commands.insert_resource(BlackHoleImages { planck: images.add(planck), noise: images.add(noise), env: faces });
}

// ---------------------------------------------------------------------------
// Catalog holes

/// Collect every catalog black hole and derive its disk parameters.
fn build_list(cat: &se_core::catalog::Catalog) -> Vec<BhInfo> {
    let mut out = Vec::new();
    for (i, s) in cat.stars.iter().enumerate() {
        // A hole whose parent never resolved has no real position (it would sit on the Sun).
        if s.barycenter || !s.has_position || s.class.kind != StarKind::BlackHole {
            continue;
        }
        let mass_sun = if s.mass_kg > 0.0 { s.mass_kg / SOLAR_MASS_KG } else { DEFAULT_MASS_SUN };
        let name = &s.names[0];
        let rs_km = RS_PER_MSUN_KM * mass_sun;
        let spin = s
            .block
            .f64("KerrSpin")
            .map(f64::abs)
            .unwrap_or_else(|| params::default_spin(mass_sun, &mut params::SeRng::from_name(&format!("{name}/spin"))));
        let disk = params::of_star(&s.block, name, mass_sun);
        let seed = se_core::proc_surface::name_seed(name);
        // Deterministic tilt for holes without a rotation model: 30-70 degrees off the pole.
        let tilt = (30.0 + 40.0 * hash_unit(seed, 1)).to_radians();
        let az = std::f64::consts::TAU * hash_unit(seed, 2);
        let default_axis = DVec3::new(tilt.sin() * az.cos(), tilt.cos(), tilt.sin() * az.sin());
        out.push(BhInfo { star: i as u32, pos_pc: s.pos_pc, has_pos: s.has_position, rs_km, spin, disk, default_axis });
    }
    out
}

/// The light of a catalog black hole's accretion disk seen from afar: SE adds the disk's
/// `Luminosity` (solar units, visual; `LuminosityBol` for the bolometric sum) to the light of
/// the hole's system (FUN_14050ed10, FUN_14050ec50). Only a catalogued `Luminosity` counts here:
/// the default SE derives from the accretion rate (FUN_140507690) is computed when a system is
/// built, and how it reaches the distant point is not decoded. Returns the sprite colour
/// (blackbody at the disk temperature) and absolute magnitude, or `None` for a dark hole.
pub fn disk_light(s: &se_core::catalog::StarRecord) -> Option<([f32; 3], f32)> {
    if s.class.kind != StarKind::BlackHole {
        return None;
    }
    let lum = s.block.sub("AccretionDisk").and_then(|b| params::DiskKeys::parse(b).luminosity)?;
    let mass_sun = if s.mass_kg > 0.0 { s.mass_kg / SOLAR_MASS_KG } else { DEFAULT_MASS_SUN };
    let dk = params::of_star(&s.block, &s.names[0], mass_sun)?;
    if !(lum > 0.0) {
        return None;
    }
    let abs_mag = se_core::units::SOLAR_ABS_MAG_V - 2.5 * lum.log10();
    Some((se_core::star::blackbody_rgb(dk.temperature.min(80_000.0)), abs_mag as f32))
}

/// SE's volumetric brightness (ImpParams.z, FUN_140270900 in the default "hdr" mode):
/// `hdrAccretionDiskBright x 10^(6984.51/T + 1.22651 - 0.803771 ln T)` (T clamped to
/// 808.86..1e5 K), times `1 / (1 - (1 - d)^(1/cos(pi/4)))` for the disk's opacity
/// `d = clamp(diskDensity x 1e-6)` and `Brightness^-0.8`.
pub fn volume_brightness(hdr_bright: f64, temp: f64, disk_density: f64, brightness: f64) -> f64 {
    let t = temp.clamp(808.859_985, 1.0e5);
    let k = 10f64.powf(6984.509_77 / t + 1.226_510_05 - 0.803_771_019 * t.ln());
    let d = (disk_density * 1.0e-6).clamp(1.0e-12, 1.0);
    let opacity = 1.0 / (1.0 - (1.0 - d).powf(1.0 / std::f64::consts::FRAC_PI_4.cos()));
    hdr_bright * k * opacity * brightness.max(1e-30).powf(-0.8)
}

/// The disk and jet uniforms of a hole (FUN_140270900's DiskParams1-5, JetParams1-3,
/// LightColor), in Schwarzschild radii.
fn disk_uniform(u: &mut HoleUniform, dk: &DiskParams, rs_km: f64, set: &BlackHoleSettings, sim_s: f64) {
    let rs = rs_km;
    // SE's render sphere (FUN_1404d6710): BlackHoleWarpRadius Rg, the disk and the jets.
    let sphere_km = (rs * set.warp_radius).max(dk.outer_km).max(dk.jet_length_km);
    let grav_s = rs / sphere_km;
    let temp = dk.temperature.min(80_000.0);
    let thick_in = dk.inner_thick_km / rs;
    let thick_out = dk.outer_thick_km / rs;
    let disk_density = dk.density * rs * DENSITY_SCALE;
    let jet_density = dk.jet_density * rs * DENSITY_SCALE;
    let (inner, outer) = (dk.inner_km / rs, dk.outer_km / rs);
    let disk_on = outer > inner && disk_density > 0.0 && thick_in.max(thick_out) >= 0.01;
    let (jet_len, jet_r0, jet_r1) = (dk.jet_length_km / rs, dk.jet_start_radius_km / rs, dk.jet_end_radius_km / rs);
    let jets_on = jet_len > 0.0 && jet_density > 0.0;
    let flags = if disk_on { 1.0 } else { 0.0 } + if jets_on { 2.0 } else { 0.0 };
    // Disk animation loop: 135 orbits of 6 pi Rg / (c / sqrt 2) (FUN_1404e5340), 256 steps.
    let period = 6.0 * std::f64::consts::PI * rs / (std::f64::consts::FRAC_1_SQRT_2 * 299_792.458);
    let loop_s = period * 135.0;
    let anim = (sim_s / loop_s).rem_euclid(1.0);
    let step = set.warp_step() * thick_in.max(thick_out).min(1.0);
    let jet_reach = jet_len.hypot(1.25 * jet_r1 + 4.0 * dk.jet_thickness * jet_r0);
    let bound = if jets_on { outer.max(jet_reach) } else { outer } * 1.05;
    // LightColor: the disk's colour at 0.8 T, normalised, x Brightness x LightingBright x 0.01.
    let lc = planck_rgb(temp * 0.8);
    let m = lc[0].max(lc[1]).max(lc[2]).max(1e-30);
    let lk = dk.brightness * dk.lighting_bright * 0.01 / m;

    u.axis_in.w = if disk_on || jets_on { inner as f32 } else { 0.0 };
    u.xref_out.w = outer as f32;
    u.params.y = grav_s as f32;
    u.params.w = volume_brightness(set.disk_bright, temp, disk_density, dk.brightness) as f32;
    u.disk1 = Vec4::new(temp as f32, dk.twist as f32, flags, anim as f32);
    u.disk2 = Vec4::new(0.0, dk.brightness as f32, if disk_on { disk_density as f32 } else { 0.0 }, step as f32);
    u.disk3 = Vec4::new(thick_in as f32, thick_out as f32, dk.detail_r as f32, dk.detail_v as f32);
    u.disk4 = Vec4::new(dk.octave_dist[0] as f32, dk.octave_dist[1] as f32, dk.octave_dist[2] as f32, dk.octave_scale as f32);
    u.disk5 = Vec4::new(dk.noise_contrast as f32, dk.outer_spin as f32, dk.temp_contrast as f32, bound as f32);
    u.jet1 = Vec4::new(dk.jet_start_temp.min(80_000.0) as f32, dk.jet_end_temp.min(80_000.0) as f32, dk.shadow_contrast as f32, dk.shadow_length as f32);
    u.jet2 = Vec4::new(jet_r0 as f32, jet_r1 as f32, jet_len as f32, dk.jet_velocity as f32);
    let jet_bright = dk.jet_brightness * set.jet_bright / set.disk_bright;
    u.jet3 = Vec4::new(jet_bright as f32, if jets_on { jet_density as f32 } else { 0.0 }, dk.jet_distortion as f32, dk.jet_thickness as f32);
    u.light = Vec4::new((lc[0] * lk) as f32, (lc[1] * lk) as f32, (lc[2] * lk) as f32, 0.0);
}

#[allow(clippy::too_many_arguments)]
fn update_lens(
    mut commands: Commands,
    data: Res<SeData>,
    sim: Res<SimTime>,
    cam: Res<FlyCamera>,
    settings: Res<BlackHoleSettings>,
    windows: Query<&Window>,
    cams: Query<(Entity, Has<BlackHoleLens>), With<MainCamera>>,
    mut list: ResMut<BhList>,
    mut env: ResMut<EnvCapture>,
    mut lensed: ResMut<LensedHoles>,
    mut frame: Local<u32>,
) {
    env.want = false;
    env.restart = false;
    lensed.0.clear();
    let Some((cam_e, has_lens)) = cams.iter().next() else { return };
    if std::env::var_os("SE_NO_BLACKHOLES").is_some() {
        return;
    }
    let cat = &data.catalog;
    if list.n_stars != cat.stars.len() {
        list.n_stars = cat.stars.len();
        list.list = build_list(cat);
    }
    let (w, h) = windows.iter().next().map(|w| (w.width() as f64, w.height() as f64)).unwrap_or((1280.0, 720.0));
    let tan_y = (cam.fov_deg.to_radians() as f64 * 0.5).tan();
    let tan_x = tan_y * w / h;
    let px_ang = 2.0 * tan_y / h;
    let cam_pc = cam.pos.to_parsecs();
    let inv_rot = cam.rot.inverse();
    let eph = Ephemeris::new(cat);
    let ecl_north = se_core::coords::ecliptic_to_universe(DVec3::Z);
    let sim_s = (sim.jd - 2_451_545.0) * 86_400.0;

    let mut found: Vec<(f64, HoleUniform)> = Vec::new();
    let mut nearest_m = f64::INFINITY;
    let mut want_env = false;
    for info in &list.list {
        if info.has_pos && info.pos_pc.distance(cam_pc) > 2.0 {
            continue;
        }
        let obj = ObjectRef::Star(info.star);
        let delta = eph.position(obj, sim.jd).delta_m(cam.pos);
        let dist = delta.length();
        let d_rs = dist / (info.rs_km * 1e3);
        if d_rs < 1.5 || dist <= 0.0 {
            continue;
        }
        let mut u = HoleUniform::default();
        if let Some(dk) = &info.disk {
            disk_uniform(&mut u, dk, info.rs_km, &settings, sim_s);
        }
        let bound = u.disk5.w as f64;
        // Angular reach of the effect: several Einstein radii, the shadow, the disk and jets.
        let theta_e = (2.0 / d_rs).sqrt();
        let mut region = (16.0 * theta_e).max((12.0 / d_rs).min(1.5));
        if bound > 0.0 {
            let ang = if d_rs <= bound * 1.1 { std::f64::consts::PI } else { 1.3 * (bound / d_rs).asin() };
            region = region.max(ang);
        }
        if d_rs < 8.0 {
            region = std::f64::consts::PI;
        }
        region = region.min(std::f64::consts::PI);
        if region < MIN_REGION_PX * px_ang {
            continue;
        }
        let dir = (inv_rot * (delta / dist)).as_vec3();
        let orient = eph.orientation(obj, sim.jd);
        let pole = (orient * DVec3::Y).normalize();
        let axis_w = if pole.dot(ecl_north) > 0.999_999 { info.default_axis } else { pole };
        let axis = (inv_rot * axis_w.normalize()).normalize();
        let helper = if axis.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
        let xref = axis.cross(helper).normalize();
        u.dir_d = dir.extend(d_rs as f32);
        u.axis_in = axis.as_vec3().extend(u.axis_in.w);
        u.xref_out = xref.as_vec3().extend(u.xref_out.w);
        u.params.x = region as f32;
        u.params.z = info.spin as f32;
        // blendFactor: frame vs environment map, smoothstep(3, 10, distance in Rg).
        let s = ((d_rs - 3.0) / 7.0).clamp(0.0, 1.0);
        u.light.w = (s * s * (3.0 - 2.0 * s)) as f32;
        nearest_m = nearest_m.min(dist);
        want_env |= reaches_env(dir.as_dvec3(), region, d_rs, tan_x, tan_y, px_ang);
        found.push((region / px_ang, u));
        lensed.0.push(info.star);
    }

    if found.is_empty() {
        if has_lens {
            commands.entity(cam_e).remove::<BlackHoleLens>();
        }
        return;
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    *frame = frame.wrapping_add(1);
    env.want = want_env;
    env.hole_dist_m = nearest_m;
    // A stale map restarts here, so its six faces (rendered before the main view this frame) are
    // already the ones the lens reads.
    if want_env && !env.cams.is_empty() && !env_current(&env, &cam) {
        env.restart = true;
        env.valid = 0;
        env.origin = Some(cam.pos);
    }
    let rot = bevy::math::DMat3::from_quat(cam.rot);
    let env_ok = want_env && !env.cams.is_empty() && (env.restart || env.valid == 0b11_1111);
    let mut lens = BlackHoleLens {
        view: Vec4::new(tan_x as f32, tan_y as f32, (*frame % 4096) as f32, found.len().min(MAX_HOLES) as f32),
        sky0: rot.x_axis.as_vec3().extend(if env_ok { 1.0 } else { 0.0 }),
        sky1: rot.y_axis.as_vec3().extend(if std::env::var_os("SE_BH_ENVTEST").is_some() { 1.0 } else { 0.0 }),
        sky2: rot.z_axis.as_vec3().extend(0.0),
        holes: [HoleUniform::default(); MAX_HOLES],
    };
    for (slot, (_, hole)) in lens.holes.iter_mut().zip(found.iter()) {
        *slot = *hole;
    }
    if *frame % 120 == 1 && std::env::var_os("SE_BH_DEBUG").is_some() {
        info!("black hole lens: {:?}", lens.holes[0]);
    }
    commands.entity(cam_e).insert(lens);
}

/// Some ray the lens bends can take its sky from the environment map: near the hole (the
/// blendFactor is below 1 under 10 Rg), when the bent region reaches a screen edge (rays leave the
/// frame), or when the shadow is large enough that its photon ring, which sees the whole sky, is
/// more than a few pixels wide.
fn reaches_env(dir: DVec3, region: f64, d_rs: f64, tan_x: f64, tan_y: f64, px_ang: f64) -> bool {
    if d_rs < 10.0 || dir.z >= 0.0 || 2.6 / d_rs > 4.0 * px_ang {
        return true;
    }
    let (ax, ay) = ((dir.x / -dir.z).abs().atan(), (dir.y / -dir.z).abs().atan());
    region >= tan_x.atan() - ax || region >= tan_y.atan() - ay
}

/// The environment faces still show the sky from here: the camera has moved less than 10% of
/// its distance to the hole since they were rendered.
fn env_current(env: &EnvCapture, cam: &FlyCamera) -> bool {
    env.origin.is_some_and(|o| cam.pos.delta_m(o).length() < 0.1 * env.hole_dist_m)
}

/// Drive the environment-map cameras: off while no hole is lensed; all six faces at once when
/// the map is (re)started, then one face per frame.
fn update_env_cams(
    mut commands: Commands,
    mut env: ResMut<EnvCapture>,
    images: Option<Res<BlackHoleImages>>,
    main: Query<&Projection, (With<MainCamera>, Without<EnvFaceCam>)>,
    mut faces: Query<(&EnvFaceCam, &mut Camera, &mut Transform, &mut Projection)>,
) {
    if !env.want {
        if env.valid != 0 || faces.iter().any(|f| f.1.is_active) {
            env.valid = 0;
            env.origin = None;
            for (_, mut c, _, _) in &mut faces {
                c.is_active = false;
            }
        }
        return;
    }
    let Some(images) = images else { return };
    if env.cams.is_empty() {
        // Spawned on first use, after the main camera (so egui keeps the main camera's context).
        for (i, (fwd, up)) in ENV_FACES.iter().enumerate() {
            let e = commands
                .spawn((
                    Camera3d::default(),
                    Camera { order: -1 - i as isize, is_active: false, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
                    bevy::camera::RenderTarget::Image(images.env[i].clone().into()),
                    bevy::camera::Hdr,
                    bevy::core_pipeline::tonemapping::Tonemapping::None,
                    bevy::render::view::Msaa::Off,
                    Projection::Perspective(PerspectiveProjection { fov: std::f32::consts::FRAC_PI_2, aspect_ratio: 1.0, near: 0.05, ..default() }),
                    Transform::IDENTITY.looking_to(*fwd, *up),
                    EnvFaceCam(i),
                ))
                .id();
            env.cams.push(e);
        }
        return;
    }
    let restart = env.restart;
    let near = main.iter().next().and_then(|p| if let Projection::Perspective(p) = p { Some(p.near) } else { None });
    let next = env.next;
    for (face, mut c, mut tf, mut proj) in &mut faces {
        let on = restart || face.0 == next;
        c.is_active = on;
        if on {
            let (fwd, up) = ENV_FACES[face.0];
            *tf = Transform::IDENTITY.looking_to(fwd, up);
            if let (Projection::Perspective(p), Some(n)) = (proj.as_mut(), near) {
                p.near = n;
            }
        }
    }
    env.valid |= if restart { 0b11_1111 } else { 1 << next };
    env.next = (next + 1) % 6;
}

// ---------------------------------------------------------------------------
// Render pass

#[derive(Resource)]
struct LensPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    noise_sampler: Sampler,
    pipeline: CachedRenderPipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<bevy::render::renderer::RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "blackhole_lens_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BlackHoleLens>(true),
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    let linear = |mode: AddressMode| SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
        address_mode_u: mode,
        address_mode_v: mode,
        ..Default::default()
    };
    let sampler = render_device.create_sampler(&linear(AddressMode::ClampToEdge));
    let noise_sampler = render_device.create_sampler(&linear(AddressMode::Repeat));
    let shader: Handle<Shader> = asset_server.load("shaders/blackhole_lens.wesl");
    let pipeline = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("blackhole_lens".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader,
            targets: vec![Some(ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(LensPipeline { layout, sampler, noise_sampler, pipeline });
}

fn lens_pass(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<BlackHoleLens>)>,
    pipe: Res<LensPipeline>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<BlackHoleLens>>,
    images: Option<Res<BlackHoleImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, index) = view.into_inner();
    if target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let (Some(binding), Some(pipeline), Some(images)) =
        (uniforms.uniforms().binding(), pipeline_cache.get_render_pipeline(pipe.pipeline), images)
    else {
        return;
    };
    let (Some(planck), Some(noise)) = (gpu_images.get(&images.planck), gpu_images.get(&images.noise)) else {
        return;
    };
    let env: Vec<&GpuImage> = images.env.iter().filter_map(|h| gpu_images.get(h)).collect();
    if env.len() != 6 {
        return;
    }
    let layout = pipeline_cache.get_bind_group_layout(&pipe.layout);
    let post = target.post_process_write();
    let bind = ctx.render_device().create_bind_group(
        "blackhole_lens_group",
        &layout,
        &BindGroupEntries::sequential((
            post.source,
            &pipe.sampler,
            binding,
            &planck.texture_view,
            &noise.texture_view,
            &pipe.noise_sampler,
            &env[0].texture_view,
            &env[1].texture_view,
            &env[2].texture_view,
            &env[3].texture_view,
            &env[4].texture_view,
            &env[5].texture_view,
        )),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("blackhole_lens_pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind, &[index.index()]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planck_table_scale() {
        // SE normalises to the 555 nm radiance of a 5700 K body: Y = sum of ybar over 5 nm steps,
        // about 21 near that temperature.
        let c = planck_rgb(5700.0);
        let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        assert!((15.0..28.0).contains(&y), "{y}");
        // Hot bodies are blue, cool ones red.
        let hot = planck_rgb(30_000.0);
        assert!(hot[2] > hot[0]);
        let cool = planck_rgb(2500.0);
        assert!(cool[0] > 3.0 * cool[2]);
        let table = planck_table();
        assert_eq!(table.len(), PLANCK_SIZE);
        // Entry 0 is 1000 e^-0.7 = 496.6 K, raised 1000x.
        assert!(table[0][0] > 0.0 && table[4095][2] > table[2048][2]);
    }

    #[test]
    fn noise_layers_line_up() {
        let t = noise_texture(7);
        let at = |x: i32, y: i32| t[(x.rem_euclid(16) + 16 * y.rem_euclid(16)) as usize];
        for (x, y) in [(0, 0), (3, 9), (15, 15)] {
            assert_eq!(at(x, y)[0], at(x + 11, y + 5)[1]);
            assert_eq!(at(x, y)[2], at(x + 11, y + 5)[3]);
        }
    }

    #[test]
    fn volume_brightness_follows_se() {
        // Optically thick disk (d = 1), Brightness 1, 6000 K: 1e4 x 10^(1.16408 + 1.22651 - 6.99239).
        let b = volume_brightness(1e4, 6000.0, 2e6, 1.0);
        let k = 1e4 * 10f64.powf(6984.50977 / 6000.0 + 1.22651005 - 0.803771019 * 6000f64.ln());
        assert!((b / k - 1.0).abs() < 1e-9, "{b} {k}");
        // A thin disk is brightened by about 1 / (sqrt2 d).
        let thin = volume_brightness(1e4, 6000.0, 1000.0, 1.0) / k;
        assert!((thin * 1.0e-3 * std::f64::consts::SQRT_2 - 1.0).abs() < 0.01, "{thin}");
    }

}
