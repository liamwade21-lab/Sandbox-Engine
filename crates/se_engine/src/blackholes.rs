//! Black holes, neutron stars and white dwarfs: gravitational lensing, the light-bent star
//! surface, volumetric accretion disks and jets.
//!
//! Everything is a single HDR post-process on the main camera (see
//! `assets/shaders/blackhole_lens.wesl` and `docs/blackholes.md`), SE's einstein.glsl in its
//! single-pass mode. Each frame this module finds the catalog black holes, neutron stars and
//! white dwarfs (SE's class-11 stars, all drawn by its GR shader) whose render sphere is big
//! enough on screen to matter, converts them to camera-space uniforms (direction, distance in
//! SE's gravRadius, disk frame, SE's MetricParams/StarParams/DiskParams/JetParams) and attaches
//! them to the camera as a [`BlackHoleLens`] component. The shader traces light rays through the
//! Kerr-Newman geometry and ray-marches the disk and jet gas along them.
//!
//! Black holes are not spawned as `SystemBody` entities (their catalog radius is 0), so this
//! plugin evaluates them straight from the catalog and ephemeris. The ordinary sphere and corona
//! of a neutron star or white dwarf are hidden while the lens draws its surface.

#[path = "blackhole_params.rs"]
pub mod params;

use bevy::asset::RenderAssetUsages;
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
const RS_PER_MSUN_KM: f64 = params::RG_PER_MSUN_KM;
/// Mass assumed for catalog black holes that have none.
const DEFAULT_MASS_SUN: f64 = 10.0;
/// Smallest on-screen influence radius (pixels) worth a lens pass.
const MIN_REGION_PX: f64 = 1.5;
/// Influence region of a hole in radii of its render sphere: the shader fades to the untouched
/// frame over the outer 30% of the region, and nothing changes outside the sphere.
const REGION_PER_SPHERE: f64 = 1.45;
/// SE's StarParams.x limit (DAT_1406d5c18) and the disk temperature limit (DAT_1406d5c2c), K.
const STAR_TEMP_MAX: f64 = 60_000.0;
const DISK_TEMP_MAX: f64 = 80_000.0;
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
    /// SE's MetricParams: KerrSpin, KerrCharge, their squares.
    pub metric: Vec4,
    /// SE's StarParams: surface temperature, radius (units; 0 for a hole), equator v/c, and
    /// starTime (DiskParams1.z).
    pub star: Vec4,
    /// KerrM (DiskParams5.w), invGravRadius (Radiuses.w), warpStep (Params.y), starRotAng
    /// (DiskParams2.w).
    pub gr: Vec4,
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
    /// `BlackHoleQuality` (3; 0 disabled .. 4 ultra): 0 draws no warp effect (the renderers' flag in
    /// FUN_1404d4100 / FUN_1404dd010), the rest set how often the environment map is redrawn
    /// (`env_interval`).
    pub quality: i32,
}

impl Default for BlackHoleSettings {
    fn default() -> Self {
        Self { disk_bright: 10_000.0, jet_bright: 10_000.0, warp_radius: 100.0, warp_precision: 0.5, env_size: 512, quality: 3 }
    }
}

impl BlackHoleSettings {
    fn warp_step(&self) -> f64 {
        0.2 * 10f64.powf(1.0 - self.warp_precision.clamp(0.0, 1.0))
    }
}

/// How often the environment map around a lensed hole is redrawn (FUN_14018a6f0), from the quality
/// in effect (time stopped counts as 1): `None` at 3 and 4 (4 every frame, 3 a face per frame), else
/// the whole map every 1 s (2) or 5 s (0-1), every 0.2 s once the camera has moved 500 km since the
/// last one, and at once while time runs 10x or faster.
fn env_interval(quality: i32, moved_km: f64, rate: f64) -> Option<f64> {
    if quality >= 3 {
        return None;
    }
    Some(if rate.abs() >= 10.0 {
        0.0
    } else if moved_km >= 500.0 {
        0.2
    } else if quality == 2 {
        1.0
    } else {
        5.0
    })
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
        quality: get("BlackHoleQuality").map_or(d.quality, |v| (v as i32).clamp(0, 4)),
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
pub(crate) struct EnvCapture {
    pub(crate) cams: Vec<Entity>,
    size: u32,
    /// Bit i: face i holds the sky seen from `origin`.
    pub(crate) valid: u8,
    next: usize,
    /// Camera position the faces were rendered at, and the distance to the lensed hole then.
    pub(crate) origin: Option<se_core::upos::UPos>,
    hole_dist_m: f64,
    /// Set by `update_lens` (and the warp bubble, `ship/warp.rs`): some bent ray can reach the
    /// environment map this frame.
    pub(crate) want: bool,
    /// Set by `update_lens`: the faces are stale, so all six render this frame.
    pub(crate) restart: bool,
    /// Set by the warp bubble at a low `ShipWarpQuality`: no face renders between restarts.
    pub(crate) paused: bool,
}

/// Catalog star indices of the black holes and neutron stars the lens draws this frame (their
/// disks are on screen as volumes, so the star field leaves out their points; their light is bent
/// strongly, so the lens flares leave out the neutron stars). White dwarfs bend light by Rg / R,
/// too little to matter, and are not listed.
#[derive(Resource, Default)]
pub struct LensedHoles(pub Vec<u32>);

/// Catalog star indices of the neutron stars and white dwarfs whose surface the lens draws this
/// frame.
#[derive(Resource, Default)]
struct LensedStars(Vec<u32>);

/// The systems that fill [`LensedHoles`] each frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LensSet;

/// Inside [`LensSet`]: the lens uniforms, then the environment cameras (the warp bubble,
/// `ship/warp.rs`, asks for the map between the two).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum LensStage {
    Lens,
    EnvCams,
}

/// The lens render pass (Core3d).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LensPassSet;

/// Marks an environment-map camera (face index).
#[derive(Component)]
struct EnvFaceCam(usize);

/// Static, camera independent description of one catalog black hole, neutron star or white
/// dwarf.
struct BhInfo {
    star: u32,
    kind: params::Remnant,
    /// Catalog position (parsecs); only meaningful if `has_pos`.
    pos_pc: DVec3,
    has_pos: bool,
    /// SE's gravRadius (FUN_140270900): the Schwarzschild radius, the star's radius for a white
    /// dwarf (km). Every length the shader sees is in these units.
    unit_km: f64,
    /// KerrM = starRadiusScale: 1, Rg / R for a white dwarf.
    kerr_m: f64,
    /// `KerrSpin`, `KerrCharge` (body +0x122c, +0x1230).
    spin: f64,
    charge: f64,
    /// Surface radius (km; 0 for a hole), `Teff` (K) and `RotationPeriod` (h; 0 if none).
    radius_km: f64,
    teff: f64,
    rot_period_h: f64,
    /// SE's render sphere, km (FUN_1404d6710).
    sphere_km: f64,
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
            .init_resource::<LensedStars>()
            .add_systems(Startup, (load_settings, init_images).chain())
            .add_systems(
                PostUpdate,
                (update_lens.in_set(LensStage::Lens), update_env_cams.in_set(LensStage::EnvCams))
                    .chain()
                    .in_set(LensSet)
                    .after(UniverseSet::Camera)
                    .before(UniverseSet::FloatingOrigin),
            )
            // After the systems that show star spheres and coronae (they run before the floating
            // origin), before visibility is propagated.
            .add_systems(
                PostUpdate,
                hide_lensed_stars
                    .after(UniverseSet::FloatingOrigin)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            )
            .add_plugins((
                ExtractComponentPlugin::<BlackHoleLens>::default(),
                UniformComponentPlugin::<BlackHoleLens>::default(),
                ExtractResourcePlugin::<BlackHoleImages>::default(),
            ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Core3d, lens_pass.in_set(LensPassSet).in_set(Core3dSystems::EarlyPostProcess));
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
            let c = planck_entry(i);
            [c[0] as f32, c[1] as f32, c[2] as f32, 1.0]
        })
        .collect()
}

/// Entry `i` of SE's PlanckFunction table ([`planck_table`]).
pub fn planck_entry(i: usize) -> [f64; 3] {
    let t = 1000.0 * ((i as f64 / PLANCK_SIZE as f64) / 0.188 - 0.7).exp();
    let c = planck_rgb(t);
    if t < 2000.0 {
        let k = 1000f64.powf(1.0 - (t - 500.0) / 1500.0);
        return c.map(|v| v * k);
    }
    c
}

/// SE's lookup of a temperature in that table (FUN_1404c4250): entry `(ln(T / 1000) + 0.7) x 0.188
/// x 4096`, rounded down and clamped.
pub fn planck_lookup(t: f64) -> [f64; 3] {
    let i = (((t.max(1e-6) * 0.001).ln() + 0.7) * 0.188 * PLANCK_SIZE as f64).floor();
    planck_entry(i.clamp(0.0, (PLANCK_SIZE - 1) as f64) as usize)
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

/// SE's render sphere (FUN_1404d6710), km: `DistortionRadiusKm` / `DistortionRadiusAU` when the
/// catalog gives one; otherwise `BlackHoleWarpRadius` Rg, at least the star's radius (3 R for a
/// white dwarf), widened to the disk's outer radius and the jet length.
pub fn render_sphere_km(kind: params::Remnant, radius_km: f64, rg_km: f64, warp_radius: f64, distortion_km: Option<f64>, disk: Option<&DiskParams>) -> f64 {
    if let Some(d) = distortion_km.filter(|d| *d > 0.0) {
        return d;
    }
    let base = match kind {
        params::Remnant::WhiteDwarf => (3.0 * radius_km).max(rg_km * warp_radius),
        _ => radius_km.max(rg_km * warp_radius),
    };
    disk.map_or(base, |d| base.max(d.outer_km).max(d.jet_length_km))
}

/// SE's StarParams.z, the surface's equator speed in c (FUN_140270900): `R 2 pi / (RotationPeriod
/// x 6378.14 x 86400) / 299792448` with R in km, a literal port: it comes out near 1e-9, so SE's
/// Doppler shift of the rotating surface is negligible.
pub fn equator_speed(radius_km: f64, period_h: f64) -> f64 {
    if !(period_h > 0.0) {
        return 0.0;
    }
    radius_km * std::f64::consts::TAU / (period_h * 6378.140_136_718_75 * 86_400.0) / 299_792_448.0
}

/// SE's starTime (DiskParams1.z): the surface spots loop over `1000 sqrt(R / km)` seconds, in
/// 256 steps.
pub fn star_time(sim_s: f64, radius_km: f64) -> f64 {
    (sim_s / (1000.0 * radius_km.max(1e-6).sqrt())).rem_euclid(1.0) * 256.0
}

/// Collect every catalog black hole, neutron star and white dwarf and derive its disk parameters.
fn build_list(cat: &se_core::catalog::Catalog, warp_radius: f64) -> Vec<BhInfo> {
    let mut out = Vec::new();
    for (i, s) in cat.stars.iter().enumerate() {
        // A star whose parent never resolved has no real position (it would sit on the Sun).
        let Some(kind) = params::Remnant::of(s.class.kind) else { continue };
        if s.barycenter || !s.has_position || s.hidden() {
            continue;
        }
        let hole = kind == params::Remnant::BlackHole;
        let mass_sun = if s.mass_kg > 0.0 { s.mass_kg / SOLAR_MASS_KG } else { DEFAULT_MASS_SUN };
        let name = &s.names[0];
        let rg_km = RS_PER_MSUN_KM * mass_sun;
        let radius_km = if hole { 0.0 } else { s.radius_km.max(1e-3) };
        let spin = s
            .block
            .f64("KerrSpin")
            .unwrap_or_else(|| params::default_spin(kind, mass_sun, &mut params::SeRng::from_name(&format!("{name}/spin"))));
        let charge = s.block.f64("KerrCharge").unwrap_or(0.0);
        let disk = params::of_star(&s.block, name, kind, mass_sun, radius_km);
        let distortion = [s.block.f64("DistortionRadiusKm"), s.block.f64("DistortionRadiusAU").map(|v| v * 149_597_870.7)]
            .into_iter()
            .rev()
            .find_map(|v| v);
        let r_sphere = if hole { rg_km } else { radius_km };
        let sphere_km = render_sphere_km(kind, r_sphere, rg_km, warp_radius, distortion, disk.as_ref());
        // FUN_140270900: a white dwarf's metric is scaled so that its radius is the unit.
        let (unit_km, kerr_m) = if kind == params::Remnant::WhiteDwarf { (radius_km, rg_km / radius_km) } else { (rg_km, 1.0) };
        let seed = se_core::proc_surface::name_seed(name);
        // Deterministic tilt for holes without a rotation model: 30-70 degrees off the pole.
        let tilt = (30.0 + 40.0 * hash_unit(seed, 1)).to_radians();
        let az = std::f64::consts::TAU * hash_unit(seed, 2);
        let default_axis = DVec3::new(tilt.sin() * az.cos(), tilt.cos(), tilt.sin() * az.sin());
        out.push(BhInfo {
            star: i as u32,
            kind,
            pos_pc: s.pos_pc,
            has_pos: s.has_position,
            unit_km,
            kerr_m,
            spin,
            charge,
            radius_km,
            teff: s.teff,
            rot_period_h: s.block.f64("RotationPeriod").unwrap_or(0.0),
            sphere_km,
            disk,
            default_axis,
        });
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
    let dk = params::of_star(&s.block, &s.names[0], params::Remnant::BlackHole, mass_sun, 0.0)?;
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
    let d = (disk_density * 1.0e-6).clamp(1.0e-12, 1.0);
    let opacity = 1.0 / (1.0 - (1.0 - d).powf(1.0 / std::f64::consts::FRAC_PI_4.cos()));
    hdr_bright * temperature_exposure(temp) * opacity * brightness.max(1e-30).powf(-0.8)
}

/// The temperature factor of SE's volumBright, `10^(6984.51/T + 1.22651 - 0.803771 ln T)` (T
/// clamped to 808.86..1e5 K); alone (times hdrAccretionDiskBright) when no disk is drawn.
pub fn temperature_exposure(temp: f64) -> f64 {
    let t = temp.clamp(808.859_985, 1.0e5);
    10f64.powf(6984.509_77 / t + 1.226_510_05 - 0.803_771_019 * t.ln())
}

/// SE's StarParams.x: `Teff` (body +0x1248), raised to the disk's temperature when the disk is
/// hotter (FUN_140507690, not for holes), at most 60000 K.
fn star_temperature(info: &BhInfo, disk: Option<&DiskParams>) -> f64 {
    let mut t = info.teff.max(0.0);
    if info.kind != params::Remnant::BlackHole {
        if let Some(d) = disk {
            t = t.max(d.temperature);
        }
    }
    t.min(STAR_TEMP_MAX)
}

/// The disk and jet uniforms (FUN_140270900's DiskParams1-5, JetParams1-3, LightColor), in
/// units of `info.unit_km`.
fn disk_uniform(u: &mut HoleUniform, dk: &DiskParams, info: &BhInfo, set: &BlackHoleSettings, sim_s: f64) {
    let rs = info.unit_km;
    let temp = dk.temperature.min(DISK_TEMP_MAX);
    let thick_in = dk.inner_thick_km / rs;
    let thick_out = dk.outer_thick_km / rs;
    let disk_density = dk.density * rs * DENSITY_SCALE;
    let jet_density = dk.jet_density * rs * DENSITY_SCALE;
    let (inner, outer) = (dk.inner_km / rs, dk.outer_km / rs);
    let disk_on = outer > inner && disk_density > 0.0 && thick_in.max(thick_out) >= 0.01;
    let (jet_len, jet_r0, jet_r1) = (dk.jet_length_km / rs, dk.jet_start_radius_km / rs, dk.jet_end_radius_km / rs);
    let jets_on = jet_len > 0.0 && jet_density > 0.0;
    let flags = if disk_on { 1.0 } else { 0.0 } + if jets_on { 2.0 } else { 0.0 };
    // Disk animation loop: 135 orbits of 6 pi Rg / (c / sqrt 2) (FUN_1404e5340), 256 steps; for a
    // white dwarf the Kepler period at the inner edge around one solar mass.
    let period = if info.kind == params::Remnant::WhiteDwarf {
        (dk.inner_km / 149_597_870.7).powi(3).sqrt() * 31_556_925.203
    } else {
        6.0 * std::f64::consts::PI * rs / (std::f64::consts::FRAC_1_SQRT_2 * 299_792.458)
    };
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
    // volumBright's temperature is the hotter of the disk and the star's surface.
    let t_exp = temp.max(star_temperature(info, Some(dk)));
    u.params.w = volume_brightness(set.disk_bright, t_exp, disk_density, dk.brightness) as f32;
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
/// Catalog distance, pc, past which a hole's lens region (far away ~`REGION_PER_SPHERE` x sphere /
/// distance radians) is under `MIN_REGION_PX` pixels of `px_ang` radians, with 10% slack: a cheap
/// cut before the ephemeris (M87*: ~60 pc at a 45 deg FOV).
fn lens_cutoff_pc(sphere_km: f64, px_ang: f64) -> f64 {
    1.1 * REGION_PER_SPHERE * sphere_km * 1e3 / (MIN_REGION_PX * px_ang * se_core::units::METERS_PER_PC)
}

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
    mut surfaces: ResMut<LensedStars>,
    mut frame: Local<u32>,
    (time, mut last_env): (Res<Time>, Local<Option<(f64, se_core::upos::UPos)>>),
) {
    env.want = false;
    env.restart = false;
    env.paused = false;
    lensed.0.clear();
    surfaces.0.clear();
    let Some((cam_e, has_lens)) = cams.iter().next() else { return };
    if settings.quality == 0 || std::env::var_os("SE_NO_BLACKHOLES").is_some() {
        return;
    }
    let cat = &data.catalog;
    if list.n_stars != cat.stars.len() {
        list.n_stars = cat.stars.len();
        list.list = build_list(cat, settings.warp_radius);
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
    // Test override of every lensed object's KerrSpin and KerrCharge: SE_BH_KERR="a,q".
    let kerr_test = std::env::var("SE_BH_KERR").ok().map(|v| {
        let mut it = v.split(',').map(|x| x.trim().parse::<f64>().unwrap_or(0.0));
        (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
    });
    // Test view: SE_BH_EDGE=1 turns every spin axis to the screen's up direction.
    let edge_test = std::env::var_os("SE_BH_EDGE").is_some();

    let mut found: Vec<(f64, HoleUniform, u32, params::Remnant)> = Vec::new();
    let mut nearest_m = f64::INFINITY;
    let mut want_env = false;
    for info in &list.list {
        if info.has_pos && info.pos_pc.distance(cam_pc) > lens_cutoff_pc(info.sphere_km, px_ang) {
            continue;
        }
        let obj = ObjectRef::Star(info.star);
        let delta = eph.position(obj, sim.jd).delta_m(cam.pos);
        let dist = delta.length();
        let d_rs = dist / (info.unit_km * 1e3);
        // Nothing to trace from inside the horizon or the star.
        let star_r = info.radius_km / info.unit_km;
        let floor = if info.kind == params::Remnant::BlackHole { 1.5 } else { star_r * 1.001 };
        if d_rs < floor || dist <= 0.0 {
            continue;
        }
        // SE draws its GR impostor over the render sphere; outside it nothing changes.
        let sphere = info.sphere_km / info.unit_km;
        let region = if d_rs * 0.999 <= sphere * REGION_PER_SPHERE {
            std::f64::consts::PI
        } else {
            (REGION_PER_SPHERE * (sphere / d_rs).asin()).min(std::f64::consts::PI)
        };
        if region < MIN_REGION_PX * px_ang {
            continue;
        }
        let mut u = HoleUniform::default();
        if let Some(dk) = &info.disk {
            disk_uniform(&mut u, dk, info, &settings, sim_s);
        }
        let dir = (inv_rot * (delta / dist)).as_vec3();
        let orient = eph.orientation(obj, sim.jd);
        let pole = (orient * DVec3::Y).normalize();
        let mut axis_w = if pole.dot(ecl_north) > 0.999_999 { info.default_axis } else { pole }.normalize();
        if edge_test {
            // Test view: the spin axis along the screen's up direction (seen edge-on).
            let (up, d) = (cam.rot * DVec3::Y, delta / dist);
            axis_w = (up - d * up.dot(d)).normalize();
        }
        // The disk frame's x axis is fixed in the world (not turning with the camera).
        let helper = if axis_w.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
        let xref_w = axis_w.cross(helper).normalize();
        let axis = (inv_rot * axis_w).normalize();
        let xref = (inv_rot * xref_w).normalize();
        // starRotAng: the prime meridian's angle about the axis in the disk frame (x, z).
        let meridian = orient * DVec3::X;
        let rot_ang = meridian.dot(axis_w.cross(xref_w)).atan2(meridian.dot(xref_w));
        let (spin, charge) = kerr_test.unwrap_or((info.spin, info.charge));
        u.dir_d = dir.extend(d_rs as f32);
        u.axis_in = axis.as_vec3().extend(u.axis_in.w);
        u.xref_out = xref.as_vec3().extend(u.xref_out.w);
        u.params.x = region as f32;
        u.params.y = (1.0 / sphere) as f32;
        u.params.z = spin as f32;
        u.metric = Vec4::new(spin as f32, charge as f32, (spin * spin) as f32, (charge * charge) as f32);
        let t_star = star_temperature(info, info.disk.as_ref());
        if info.disk.is_none() {
            // No disk: diskTemp is the star's temperature, volumBright its exposure alone.
            u.disk1.x = t_star as f32;
            u.params.w = (settings.disk_bright * temperature_exposure(t_star)) as f32;
        }
        if info.kind != params::Remnant::BlackHole {
            u.star = Vec4::new(t_star as f32, star_r as f32, equator_speed(info.radius_km, info.rot_period_h) as f32, star_time(sim_s, info.radius_km) as f32);
        }
        u.gr = Vec4::new(info.kerr_m as f32, sphere as f32, settings.warp_step() as f32, rot_ang as f32);
        // blendFactor: frame vs environment map, smoothstep(3, 10, distance in Rg).
        let s = ((d_rs - 3.0) / 7.0).clamp(0.0, 1.0);
        u.light.w = (s * s * (3.0 - 2.0 * s)) as f32;
        nearest_m = nearest_m.min(dist);
        want_env |= reaches_env(dir.as_dvec3(), region, d_rs, tan_x, tan_y, px_ang);
        found.push((region / px_ang, u, info.star, info.kind));
    }

    if found.is_empty() {
        if has_lens {
            commands.entity(cam_e).remove::<BlackHoleLens>();
        }
        return;
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found.truncate(MAX_HOLES);
    for &(_, _, star, kind) in &found {
        if kind != params::Remnant::WhiteDwarf {
            lensed.0.push(star);
        }
        if kind != params::Remnant::BlackHole {
            surfaces.0.push(star);
        }
    }
    *frame = frame.wrapping_add(1);
    env.want = want_env;
    env.hole_dist_m = nearest_m;
    // A restarted map renders its six faces before the main view this frame, so the lens already
    // reads them. At quality 4 every frame; at 3 when the camera has moved 10% of its distance to the
    // hole, the faces rolling one per frame between; below that at SE's intervals, frozen between.
    if want_env && !env.cams.is_empty() {
        let quality = if sim.paused || sim.rate == 0.0 { 1 } else { settings.quality };
        let now = time.elapsed_secs_f64();
        let restart = match last_env.map(|(t, p)| (now - t, cam.pos.delta_m(p).length() * 1e-3)) {
            _ if quality >= 4 || env.valid == 0 => true,
            None => true,
            Some((age, moved_km)) => match env_interval(quality, moved_km, sim.rate) {
                None => !env_current(&env, &cam),
                Some(dt) => {
                    env.paused = true;
                    age >= dt
                }
            },
        };
        if restart {
            env.restart = true;
            env.paused = false;
            env.valid = 0;
            env.origin = Some(cam.pos);
            *last_env = Some((now, cam.pos));
        }
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
    for (slot, (_, hole, _, _)) in lens.holes.iter_mut().zip(found.iter()) {
        *slot = *hole;
    }
    if *frame % 120 == 1 && std::env::var_os("SE_BH_DEBUG").is_some() {
        info!("black hole lens: {:?}", lens.holes[0]);
    }
    commands.entity(cam_e).insert(lens);
}

/// The lens draws a lensed neutron star's or white dwarf's surface itself (SE draws these stars
/// only with its GR shader, FUN_1404d4100, and their glow after it): their ordinary sphere and
/// corona are hidden meanwhile. A neutron star bends light strongly, so anything of it left in
/// the frame would be bent into a ring around it: its point sprite's glare is hidden too once the
/// surface is half a pixel wide (the lensed surface, and the bloom of it, then show the star; a
/// smaller star keeps its sprite, whose ghost ring falls outside SE's integration sphere or into
/// the glare), and the lens flares leave it out (lensflare.rs reads `LensedHoles`).
#[allow(clippy::type_complexity)]
fn hide_lensed_stars(
    surfaces: Res<LensedStars>,
    lensed: Res<LensedHoles>,
    bodies: Query<&crate::system::SystemBody>,
    mut spheres: Query<(&crate::system::SystemBody, &mut Visibility), With<crate::system::StarSphere>>,
    mut coronas: Query<
        (&crate::universe::UPosition, &mut Visibility),
        (With<MeshMaterial3d<crate::corona::CoronaMaterial>>, Without<crate::system::StarSphere>, Without<crate::system::BodySprite>),
    >,
    mut sprites: Query<(&crate::system::BodySprite, &mut Visibility), Without<crate::system::StarSphere>>,
) {
    if surfaces.0.is_empty() {
        return;
    }
    let mut hidden = Vec::new();
    for (b, mut vis) in &mut spheres {
        if let ObjectRef::Star(i) = b.obj
            && surfaces.0.contains(&i)
        {
            *vis = Visibility::Hidden;
            hidden.push((b.pos, lensed.0.contains(&i)));
        }
    }
    if hidden.is_empty() {
        return;
    }
    for (p, mut vis) in &mut coronas {
        if hidden.iter().any(|h| h.0 == p.0) {
            *vis = Visibility::Hidden;
        }
    }
    for (s, mut vis) in &mut sprites {
        if bodies.get(s.owner).is_ok_and(|b| b.px_radius >= 0.5 && hidden.contains(&(b.pos, true))) {
            *vis = Visibility::Hidden;
        }
    }
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
        let on = restart || (face.0 == next && !env.paused);
        c.is_active = on;
        if on {
            let (fwd, up) = ENV_FACES[face.0];
            *tf = Transform::IDENTITY.looking_to(fwd, up);
            if let (Projection::Perspective(p), Some(n)) = (proj.as_mut(), near) {
                p.near = n;
            }
        }
    }
    if restart || !env.paused {
        env.valid |= if restart { 0b11_1111 } else { 1 << next };
        env.next = (next + 1) % 6;
    }
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
    // `&BlackHoleLens` too: once the last lens is removed, Bevy keeps the view's stale uniform
    // index while the buffer behind it is refilled from fresh staging memory (garbage), so a
    // pass keyed on the index alone ran on a random hole count and hung the GPU.
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<BlackHoleLens>, &BlackHoleLens)>,
    pipe: Res<LensPipeline>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<BlackHoleLens>>,
    images: Option<Res<BlackHoleImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, index, _) = view.into_inner();
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
    fn environment_map_follows_black_hole_quality() {
        assert_eq!(env_interval(4, 0.0, 1.0), None);
        assert_eq!(env_interval(3, 1e6, 1.0), None);
        assert_eq!(env_interval(2, 0.0, 1.0), Some(1.0));
        assert_eq!(env_interval(1, 0.0, 1.0), Some(5.0));
        assert_eq!(env_interval(0, 600.0, 1.0), Some(0.2));
        assert_eq!(env_interval(2, 0.0, -10.0), Some(0.0));
    }

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
        assert!((temperature_exposure(6000.0) * 1e4 / k - 1.0).abs() < 1e-12);
    }

    #[test]
    fn render_sphere_follows_se() {
        use params::Remnant;
        // FUN_1404d6710: 100 Rg (BlackHoleWarpRadius), at least R; 3 R for a white dwarf.
        let rg = RS_PER_MSUN_KM * 1.4;
        assert!((render_sphere_km(Remnant::NeutronStar, 12.0, rg, 100.0, None, None) - 100.0 * rg).abs() < 1e-9);
        assert!((render_sphere_km(Remnant::WhiteDwarf, 7000.0, 2.95, 100.0, None, None) - 21_000.0).abs() < 1e-9);
        assert!((render_sphere_km(Remnant::BlackHole, 29.5, 29.5, 100.0, None, None) - 2950.0).abs() < 1e-9);
        // A catalog DistortionRadius wins, even over the disk.
        assert_eq!(render_sphere_km(Remnant::BlackHole, 29.5, 29.5, 100.0, Some(5.0e7), None), 5.0e7);
        // The disk and the jets widen it.
        let keys = params::DiskKeys { temperature: Some(6000.0), outer_km: Some(1.0e5), ..Default::default() };
        let d = params::resolve(Some(&keys), false, Remnant::BlackHole, 10.0, 29.5, 29.5, &mut params::SeRng::from_name("x")).unwrap();
        assert!((render_sphere_km(Remnant::BlackHole, 29.5, 29.5, 100.0, None, Some(&d)) - 1.0e5).abs() < 1e-6);
    }

    #[test]
    fn lens_cutoff_follows_screen_size() {
        // M87* (6.5e9 solar masses) at a 45 deg FOV, 720 px high: still lensed well past 2 pc.
        let rg = RS_PER_MSUN_KM * 6.5e9;
        let sphere_km = render_sphere_km(params::Remnant::BlackHole, rg, rg, 100.0, None, None);
        let px_ang = 2.0 * 22.5f64.to_radians().tan() / 720.0;
        let cut = lens_cutoff_pc(sphere_km, px_ang);
        assert!(cut > 30.0 && cut < 100.0, "{cut}");
        // Just inside the cut (less the slack) the region is the MIN_REGION_PX limit.
        let d = cut / 1.1 * se_core::units::METERS_PER_PC * 1e-3;
        let region = REGION_PER_SPHERE * (sphere_km / d).asin();
        assert!((region / (MIN_REGION_PX * px_ang) - 1.0).abs() < 1e-6);
        // A stellar hole's cut is far below a parsec.
        assert!(lens_cutoff_pc(100.0 * RS_PER_MSUN_KM * 10.0, px_ang) < 1e-3);
    }

    #[test]
    fn star_surface_params() {
        // SE's equator speed formula comes out tiny even for a millisecond pulsar.
        let v = equator_speed(10.0, 9.306_349_528e-6);
        assert!(v > 0.0 && v < 1e-6, "{v}");
        assert_eq!(equator_speed(10.0, 0.0), 0.0);
        // starTime loops over 1000 sqrt(R) s in 256 steps.
        let r = 10.0_f64;
        let period = 1000.0 * r.sqrt();
        assert!((star_time(0.25 * period, r) - 64.0).abs() < 1e-6);
        assert!((star_time(1.25 * period, r) - star_time(0.25 * period, r)).abs() < 1e-6);
    }

}
