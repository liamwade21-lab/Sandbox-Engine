//! Precomputed atmospheric scattering (Bruneton & Neyret 2008) sky renderer.
//!
//! Loads SpaceEngine's `models/atmospheres/<Model>.atm` tables into GPU textures and renders a sky shell
//! (a sphere scaled to the top of the atmosphere) that works from space (limb glow) and from inside
//! (blue sky, sunsets). The shared WESL functions in `assets/shaders/atmosphere_common.wesl` can also be
//! called by planet-surface shaders for sun transmittance, sky irradiance and aerial perspective.
//!
//! This module depends only on `bevy`, `se_core` and `std`. See `docs/atmosphere.md`.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, Face, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::atm::{self, AtmParams, AtmTables};
use se_core::vfs::Vfs;

/// Radius of the sky-shell mesh (the caller scales it so that the *transform scale* equals the top
/// radius). Slightly above 1 so the tessellated silhouette always encloses the true atmosphere sphere.
const SHELL_MESH_RADIUS: f32 = 1.02;

// ------------------------------------------------------------------------------------------------
// Plugin
// ------------------------------------------------------------------------------------------------

/// Registers [`SkyMaterial`]. Nothing else global.
pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .init_resource::<AtmosphereCache>()
            .add_systems(Update, upload_preloaded);
    }
}

/// Loaded atmosphere models by (lower-case) name. Loading one (reading, parsing and
/// uploading its tables) takes 50-150 ms, which would stall the frame on a system change,
/// so all shipped models are parsed on a worker thread at startup and uploaded one per
/// frame; a model asked for before that is loaded on the spot.
#[derive(Resource, Default)]
pub struct AtmosphereCache {
    models: std::collections::HashMap<String, Option<AtmosphereGpu>>,
    preload: Option<Task<Vec<(String, AtmTables)>>>,
    parsed: Vec<(String, AtmTables)>,
    started: bool,
    /// SE's `CacheSizeAtmoModels` (0: no limit, main-def.cfg's value): beyond it the least
    /// recently asked-for models are dropped (their bodies keep their handles).
    cap: usize,
    used: std::collections::HashMap<String, u64>,
    tick: u64,
    /// SE's `AtmoAnalyticTransm` for the models it generates.
    analytic_transm: bool,
}

/// What [`AtmosphereCache::get_or_load`] found for a model.
pub enum AtmLoad {
    Ready(AtmosphereGpu),
    /// A `.cfg`-only model still being generated in the background: ask again later.
    Pending,
    Missing,
}

impl AtmosphereCache {
    /// Start parsing every `models/atmospheres/*.atm` in the background (once), then generating the
    /// tables of every model of the `models/atmospheres/*.cfg` scripts that has no `.atm` file, as SE's
    /// `seAtmosphereModelCache` does (`se_core::atm_gen`, about 12 s a model). Generated tables are
    /// cached in the port's own folder (`%APPDATA%/SpaceEngine-rs-v2/cache/atmospheres`), never the install.
    pub fn start_preload(&mut self, vfs: std::sync::Arc<Vfs>) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let analytic = self.analytic_transm;
        self.preload = Some(AsyncComputeTaskPool::get().spawn(async move {
            let mut out: Vec<(String, AtmTables)> = vfs
                .list_ext("models/atmospheres", "atm")
                .into_iter()
                .filter_map(|path| {
                    let name = path.rsplit('/').next()?.strip_suffix(".atm")?.to_ascii_lowercase();
                    Some((name, AtmTables::parse(&vfs.read(&path).ok()?).ok()?))
                })
                .collect();
            let models: Vec<(String, AtmParams)> = vfs
                .list_ext("models/atmospheres", "cfg")
                .into_iter()
                .filter_map(|p| vfs.read_string(&p).ok())
                .flat_map(|t| se_core::atm_gen::parse_models(&t))
                .collect();
            // The port's folder (settings.rs `user_file` and `USER_DIR`, spelled out: this file is also built into atmo_demo).
            let dir = std::env::var_os("APPDATA").map_or_else(|| std::path::PathBuf::from("cache"), |p| std::path::PathBuf::from(p).join("SpaceEngine-rs-v2").join("cache")).join("atmospheres");
            for (name, params) in models {
                let key = name.to_ascii_lowercase();
                if out.iter().any(|(n, _)| *n == key) {
                    continue;
                }
                let file = dir.join(format!("{name}.atm"));
                if let Some(t) = std::fs::read(&file).ok().and_then(|b| AtmTables::parse(&b).ok()).filter(|t| t.params == params) {
                    out.push((key, t));
                    continue;
                }
                info!("Generating atmosphere model \"{name}\"");
                let t = se_core::atm_gen::generate(&params, analytic);
                if std::fs::create_dir_all(&dir).is_ok() {
                    let _ = std::fs::write(&file, t.to_bytes());
                }
                out.push((key, t));
            }
            out
        }));
    }

    /// SE's `AtmoAnalyticTransm` (main-def.cfg) for generated models.
    pub fn set_analytic_transm(&mut self, on: bool) {
        self.analytic_transm = on;
    }

    pub fn get_or_load(&mut self, vfs: &Vfs, model: &str, images: &mut Assets<Image>) -> AtmLoad {
        let name = model.to_ascii_lowercase();
        // A model being generated in the background is not there yet: no warning, no negative entry.
        if !self.models.contains_key(&name) && self.preload.is_some() && !vfs.is_readable(&format!("models/atmospheres/{model}.atm")) {
            return AtmLoad::Pending;
        }
        if !self.models.contains_key(&name) {
            if let Some(i) = self.parsed.iter().position(|(n, _)| *n == name) {
                let (_, t) = self.parsed.swap_remove(i);
                self.models.insert(name.clone(), Some(upload_tables(t, images)));
            }
        }
        let gpu = self.models.entry(name.clone()).or_insert_with(|| load_atmosphere(vfs, model, images)).clone();
        self.touch(name);
        gpu.map_or(AtmLoad::Missing, AtmLoad::Ready)
    }

    /// Mark `name` as just used and drop the least recently used others beyond the cache size.
    fn touch(&mut self, name: String) {
        self.tick += 1;
        self.used.insert(name.clone(), self.tick);
        if self.cap == 0 {
            return;
        }
        while self.models.len() > self.cap {
            let used = &self.used;
            let Some(old) = self.models.keys().filter(|k| **k != name).min_by_key(|k| used.get(*k).copied().unwrap_or(0)).cloned() else { break };
            self.models.remove(&old);
            self.used.remove(&old);
        }
    }

    /// Set the cache size (memory.rs, from `CacheSizeAtmoModels`).
    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap;
    }
}

fn upload_preloaded(mut cache: ResMut<AtmosphereCache>, mut images: ResMut<Assets<Image>>) {
    if let Some(list) = cache.preload.as_mut().and_then(check_ready) {
        cache.preload = None;
        cache.parsed = list;
    }
    // With a cache size, preloading stops once it is full (it would only push models out).
    if cache.cap > 0 && cache.models.len() >= cache.cap {
        cache.parsed.clear();
    }
    if let Some((name, t)) = cache.parsed.pop() {
        if !cache.models.contains_key(&name) {
            let gpu = upload_tables(t, &mut images);
            cache.models.insert(name.clone(), Some(gpu));
            cache.touch(name);
        }
    }
}

// ------------------------------------------------------------------------------------------------
// GPU resources
// ------------------------------------------------------------------------------------------------

/// The three scattering tables of one atmosphere model, uploaded as filterable `Rgba16Float` textures.
#[derive(Clone)]
pub struct AtmosphereGpu {
    /// Model parameters (radii and heights in km, coefficients in 1/km).
    pub params: AtmParams,
    /// 3D, 512 (nu*32 + mu_s) x 64 (mu) x 32 (r).
    pub inscatter: Handle<Image>,
    /// 2D, 256 (mu) x 64 (r).
    pub transmittance: Handle<Image>,
    /// 2D, 64 (mu_s) x 16 (r).
    pub irradiance: Handle<Image>,
}

impl AtmosphereGpu {
    /// Uniform block for [`AtmParams`] in the shaders (`AtmParams` struct in `atmosphere_common.wesl`).
    pub fn uniform(&self) -> AtmosphereParamsUniform {
        AtmosphereParamsUniform::from(&self.params)
    }

    /// Atmosphere top radius (m) the sky shell must be scaled to so that the table's ground sphere maps
    /// exactly onto a planet of `planet_radius_m`: `planet_radius * Rt / Rg`. Prefer this over
    /// `planet_radius + height` when the catalog height differs from the table's `Rt - Rg`.
    pub fn top_radius_m(&self, planet_radius_m: f64) -> f64 {
        planet_radius_m * (self.params.radius_top / self.params.radius_ground) as f64
    }

    /// The model drawn `height_m` thick on a body of `planet_radius_m`, as SE draws every atmosphere
    /// at the body's visual height (its `Radiuses`: tables read at the altitude's fraction of the
    /// height): the top radius and scale heights stretched, the scattering coefficients shrunk by the
    /// same factor so the optical depths stay the model's.
    pub fn with_height(&self, planet_radius_m: f64, height_m: f64) -> Self {
        let mut gpu = self.clone();
        let p = &mut gpu.params;
        let thick = p.radius_top - p.radius_ground;
        let want = (height_m / planet_radius_m) as f32 * p.radius_ground;
        if thick > 0.0 && want > 0.0 && want.is_finite() {
            let k = want / thick;
            p.radius_top = p.radius_ground + want;
            p.rayleigh_h *= k;
            p.mie_h *= k;
            for beta in [&mut p.rayleigh_beta, &mut p.mie_sca_beta, &mut p.mie_ext_beta] {
                for c in beta.iter_mut() {
                    *c /= k;
                }
            }
        }
        gpu
    }
}

/// Transmittance of an atmosphere from `x` (table km, planet-centred) along `v` out to space: SE's
/// `transmittanceAnalytic` with its Chapman-style `opticalDepth` (`atmo_transm.glsl`). 1 for a ray
/// that misses the atmosphere, 0 for one that meets the ground.
pub fn transmittance_to_space(p: &AtmParams, x: DVec3, v: DVec3) -> DVec3 {
    let (rg, rt) = (p.radius_ground as f64, p.radius_top as f64);
    let mut r = x.length().max(1e-9);
    let mut mu = x.dot(v) / r;
    let disc = r * r * (mu * mu - 1.0) + rt * rt;
    if disc < 0.0 {
        return DVec3::ONE;
    }
    if r > rt {
        // In space: start where the ray enters.
        let d = -r * mu - disc.sqrt();
        if d < 0.0 {
            return DVec3::ONE;
        }
        mu = (r * mu + d) / rt;
        r = rt;
    }
    if mu < 0.0 && r * r * (mu * mu - 1.0) + rg * rg >= 0.0 {
        return DVec3::ZERO;
    }
    let d = -r * mu + (r * r * (mu * mu - 1.0) + rt * rt).max(0.0).sqrt();
    let depth = |h: f64| {
        let rcp_h = 1.0 / h;
        let a = (0.5 * r * rcp_h).sqrt();
        let dr = d / r;
        let (a0, a1) = (a * mu, a * (dr + mu));
        let (s0, s1) = (a0.signum(), a1.signum());
        let x = if s1 > s0 { (a0 * a0).exp() } else { 0.0 };
        let y0 = s0 / (2.3193 * a0.abs() + (1.52 * a0 * a0 + 4.0).sqrt());
        let y1 = s1 / (2.3193 * a1.abs() + (1.52 * a1 * a1 + 4.0).sqrt()) * (-d * rcp_h * (0.5 * dr + mu)).exp();
        (6.2831 * h * r).sqrt() * ((rg - r) * rcp_h).exp() * (x + y0 - y1)
    };
    let beta = |b: [f32; 3]| DVec3::new(b[0] as f64, b[1] as f64, b[2] as f64);
    let tau = beta(p.rayleigh_beta) * depth(p.rayleigh_h as f64) + beta(p.mie_ext_beta) * depth(p.mie_h as f64);
    let t = DVec3::new((-tau.x).exp(), (-tau.y).exp(), (-tau.z).exp());
    if t.is_finite() { t } else { DVec3::ONE }
}

/// GPU mirror of the shaders' `AtmParams` struct.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct AtmosphereParamsUniform {
    /// x: Rg (km), y: Rt (km), z: Rayleigh scale height (km), w: Mie scale height (km).
    pub radii: Vec4,
    /// x: Mie g, y: Mie twilight fade.
    pub mie: Vec4,
    pub beta_r: Vec4,
    pub beta_m_sca: Vec4,
    pub beta_m_ext: Vec4,
    /// The body's catalog look ([`ATM_LOOK_DEFAULT`] for the model alone): x `Opacity` (mixes every
    /// transmittance towards 1), y `Hue` (turns) and z `Saturation` (factor) of the scattered light,
    /// w `SkyLight` (the sky's irradiance on the ground).
    pub look: Vec4,
}

/// Opacity 1, no hue shift, saturation 1, SkyLight 1.
pub const ATM_LOOK_DEFAULT: Vec4 = Vec4::new(1.0, 0.0, 1.0, 1.0);

impl From<&AtmParams> for AtmosphereParamsUniform {
    fn from(p: &AtmParams) -> Self {
        let v3 = |a: [f32; 3]| Vec4::new(a[0], a[1], a[2], 0.0);
        Self {
            radii: Vec4::new(p.radius_ground, p.radius_top, p.rayleigh_h, p.mie_h),
            mie: Vec4::new(p.mie_g, p.mie_fade, 0.0, 0.0),
            beta_r: v3(p.rayleigh_beta),
            beta_m_sca: v3(p.mie_sca_beta),
            beta_m_ext: v3(p.mie_ext_beta),
            look: ATM_LOOK_DEFAULT,
        }
    }
}

/// Round-to-nearest-even `f32` -> IEEE binary16 bits.
fn f32_to_f16_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let mut h = (m >> shift) as u16;
        let rem = m & ((1u32 << shift) - 1);
        let half = 1u32 << (shift - 1);
        if rem > half || (rem == half && (h & 1) == 1) {
            h += 1;
        }
        return sign | h;
    }
    let mut h = (((e as u32) << 10) | (man >> 13)) as u16;
    let rem = man & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    sign | h
}

fn f16_bytes(data: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &f in data {
        out.extend_from_slice(&f32_to_f16_bits(f).to_le_bytes());
    }
    out
}

fn table_image(dim: TextureDimension, size: (u32, u32, u32), data: &[f32]) -> Image {
    let mut img = Image::new(
        Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: size.2,
        },
        dim,
        f16_bytes(data),
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        address_mode_w: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Nearest,
        ..default()
    });
    img
}

/// Load `models/atmospheres/<model>.atm` from the game VFS and upload the tables. Returns `None` (and
/// logs a warning) if the model is missing or malformed.
pub fn load_atmosphere(vfs: &Vfs, model: &str, images: &mut Assets<Image>) -> Option<AtmosphereGpu> {
    let path = format!("models/atmospheres/{model}.atm");
    let bytes = match vfs.read(&path) {
        Ok(b) => b,
        Err(e) => {
            warn!("atmosphere model {model}: cannot read {path}: {e}");
            return None;
        }
    };
    match AtmTables::parse(&bytes) {
        Ok(t) => Some(upload_tables(t, images)),
        Err(e) => {
            warn!("atmosphere model {model}: {e}");
            None
        }
    }
}

/// Upload parsed tables as textures.
fn upload_tables(t: AtmTables, images: &mut Assets<Image>) -> AtmosphereGpu {
    let inscatter = images.add(table_image(
        TextureDimension::D3,
        ((atm::RES_MU_S * atm::RES_NU) as u32, atm::RES_MU as u32, atm::RES_R as u32),
        &t.inscatter,
    ));
    let transmittance = images.add(table_image(
        TextureDimension::D2,
        (atm::TRANSMITTANCE_W as u32, atm::TRANSMITTANCE_H as u32, 1),
        &t.transmittance,
    ));
    let irradiance = images.add(table_image(
        TextureDimension::D2,
        (atm::SKY_W as u32, atm::SKY_H as u32, 1),
        &t.irradiance,
    ));
    AtmosphereGpu {
        params: t.params,
        inscatter,
        transmittance,
        irradiance,
    }
}

// ------------------------------------------------------------------------------------------------
// Sky material
// ------------------------------------------------------------------------------------------------

/// The further stars a sky is lit by besides the brightest (`system::MAX_LIGHTS` - 1).
pub const SKY_EXTRA_LIGHTS: usize = 3;

/// SE's `MAX_ECLIPSES` (planet.glsl, sky.glsl, water.glsl) and `MAX_RINGS_ECLIPSES` (rings.glsl): casters
/// per light.
pub const MAX_ECLIPSES: usize = 8;

/// The eclipse casters of one light (`EclipseLight` in `eclipse.wesl`, filled by `eclipse.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct EclipseLight {
    /// xyz: caster centre relative to the lit body (render orientation, lit-body radii); w its radius in
    /// the same units, 0 ending the list (SE's `if (casterRadius <= 0.0) break`).
    pub casters: [Vec4; MAX_ECLIPSES],
    /// x: the light's angular radius seen from the lit body (rad), y: number of casters.
    pub info: Vec4,
}

/// Uniform block of the sky shader (`SkyUniform` in `sky.wesl`).
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct SkyUniform {
    pub atm: AtmosphereParamsUniform,
    /// xyz: unit direction towards the star (world / render space); w: star angular radius in radians
    /// (0 = do not draw a sun disc).
    pub sun_dir: Vec4,
    /// xyz: sun irradiance in scene radiance units (rgb); w: brightness multiplier (catalog `Bright`).
    pub sun_irradiance: Vec4,
    /// Body in front of the star during an eclipse: xyz direction, w angular radius (0 = none).
    pub occluder: Vec4,
    /// rgb: star irradiance for the disc, before eclipse dimming.
    pub disc_irradiance: Vec4,
    /// The further stars: xyz direction, w 1 when used; rgb their irradiance like `sun_irradiance`.
    pub light_dir: [Vec4; SKY_EXTRA_LIGHTS],
    pub light_irradiance: [Vec4; SKY_EXTRA_LIGHTS],
    /// The body's rings for their shadow on the sky (SE's RingsParams): x inner, y outer radius
    /// (planet radii), z `PlanetShadow`, w 1 when drawn (rings and `AtmoRingsShadow`), 0.5 untextured.
    pub ring: Vec4,
    /// x the planet's radius over the shell's (top) radius, y the eclipse intensity (SE's
    /// AmbientColor.a), z SE's `AtmoDistanceLimit` in shell radii (0 none), w 1 with `AtmoExtinction`.
    pub shadow: Vec4,
    /// Each light's eclipse casters, the main star first ([`crate::eclipse::EclipseCasters`]).
    pub eclipse: [EclipseLight; SKY_EXTRA_LIGHTS + 1],
    /// SE's GlowColorAtmo (terrain.rs `glow_color_atmo`) in the units of `light_irradiance`; 0 none.
    pub glow: Vec4,
}


/// The three passes of a sky shell, drawn in this order (SE: `atmo_transm.glsl` dims what lies
/// behind the sky by the transmittance along the ray, per channel, then `sky.glsl` adds the
/// inscattered light).
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SkyPass {
    /// Stars, galaxies and nebulae (the empty background, where nothing wrote depth) fade out in a
    /// bright sky. SE hides them by the eye's adaptation to the sky; the port's exposure is fixed.
    Daylight,
    /// Everything behind the sky, bodies included, times the transmittance (red near the horizon).
    Extinction,
    /// The sky's own light, added.
    #[default]
    Inscatter,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(SkyMaterialKey)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub uniform: SkyUniform,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub inscatter: Handle<Image>,
    #[texture(3)]
    pub transmittance: Handle<Image>,
    /// The body's ring texture (its alpha is the rings' opacity), Bevy's white image without rings.
    #[texture(4)]
    #[sampler(5)]
    pub ring: Handle<Image>,
    pub pass: SkyPass,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct SkyMaterialKey {
    pass: SkyPass,
}

impl From<&SkyMaterial> for SkyMaterialKey {
    fn from(m: &SkyMaterial) -> Self {
        Self { pass: m.pass }
    }
}

impl SkyMaterial {
    /// Material with the sun straight up, white irradiance 1 and no sun disc.
    pub fn new(gpu: &AtmosphereGpu, bright: f32) -> Self {
        Self {
            uniform: SkyUniform {
                atm: gpu.uniform(),
                sun_dir: Vec4::new(0.0, 1.0, 0.0, 0.0),
                sun_irradiance: Vec4::new(1.0, 1.0, 1.0, bright),
                occluder: Vec4::ZERO,
                disc_irradiance: Vec4::ONE,
                light_dir: [Vec4::ZERO; SKY_EXTRA_LIGHTS],
                light_irradiance: [Vec4::ZERO; SKY_EXTRA_LIGHTS],
                ring: Vec4::ZERO,
                shadow: Vec4::new(gpu.params.radius_ground / gpu.params.radius_top, 1.0, 0.0, 1.0),
                eclipse: Default::default(),
                glow: Vec4::ZERO,
            },
            inscatter: gpu.inscatter.clone(),
            transmittance: gpu.transmittance.clone(),
            ring: Handle::default(),
            pass: SkyPass::Inscatter,
        }
    }

    /// Set the star direction (world/render space, will be normalised), its irradiance (rgb, scene
    /// radiance units) and angular radius (radians, 0 = no disc).
    pub fn set_sun(&mut self, dir_to_star: Vec3, irradiance: Vec3, angular_radius: f32) {
        let d = dir_to_star.normalize_or_zero();
        self.uniform.sun_dir = d.extend(angular_radius);
        self.uniform.sun_irradiance = irradiance.extend(self.uniform.sun_irradiance.w);
        self.uniform.disc_irradiance = irradiance.extend(1.0);
    }

    /// Eclipse state: the occluding body's direction and angular radius (0 = none), and the
    /// undimmed star irradiance used for the visible part of the disc.
    pub fn set_occluder(&mut self, occluder: Vec4, disc_irradiance: Vec3) {
        self.uniform.occluder = occluder;
        self.uniform.disc_irradiance = disc_irradiance.extend(1.0);
    }

    // Kept as the public sun-brightness control; no caller wires it up yet.
    #[allow(dead_code)]
    pub fn set_brightness(&mut self, bright: f32) {
        self.uniform.sun_irradiance.w = bright;
    }
}

impl Material for SkyMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sky.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/sky.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // Transparent: the inscatter is added (Bevy's Add is premultiplied blending, alpha 0) and the
        // other two passes multiply (`specialize`). The stars, galaxies and nebulae sort before the
        // shells (depth biases -5e15 to -2e15), what lies inside or in front of an atmosphere after
        // them (clouds, rings and engine plumes +1e10; corona, aurora and lens flares 3.5e15 and up).
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // Every shell's dimming before any shell's light (render distances stay below ~3e8).
        match self.pass {
            SkyPass::Daylight => -2.0e9,
            SkyPass::Extinction => -1.0e9,
            SkyPass::Inscatter => 0.0,
        }
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        let pass = key.bind_group_data.pass;
        // dst = dst * src, per channel.
        let multiply = BlendState {
            color: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::Src, operation: BlendOperation::Add },
            alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
        };
        let (vertex, fragment, blend) = match pass {
            // On the far plane: with reversed depth it passes only where nothing wrote depth.
            SkyPass::Daylight => ("vertex_far", "daylight", Some(multiply)),
            SkyPass::Extinction => ("vertex", "extinction", Some(multiply)),
            SkyPass::Inscatter => ("vertex", "fragment", None),
        };
        descriptor.vertex.entry_point = Some(vertex.into());
        if let Some(f) = descriptor.fragment.as_mut() {
            f.entry_point = Some(fragment.into());
            if blend.is_some() {
                for t in f.targets.iter_mut().flatten() {
                    t.blend = blend;
                }
            }
        }
        // Cull front faces: the far side of the shell is always drawn, so the shell works from inside and
        // outside. Additive blending (AlphaMode::Add) does not write depth.
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

/// Marks a sky shell entity and keeps its material handle for per-frame updates.
#[derive(Component, Clone)]
pub struct SkyShell {
    pub material: Handle<SkyMaterial>,
    /// The [`SkyPass::Daylight`] and [`SkyPass::Extinction`] materials (on child entities).
    pub dimming: [Handle<SkyMaterial>; 2],
}

/// Spawn a sky shell for a planet. The entity's `Transform` must be set every frame by the caller:
/// `translation` = planet centre relative to the camera (times the scaled-space factor k) and
/// `scale = Vec3::splat(top_radius * k)` where `top_radius` = `planet_radius_m + atmosphere_height_m`
/// (or [`AtmosphereGpu::top_radius_m`]). The initial transform is the unscaled planet at the origin.
pub fn spawn_sky_shell(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SkyMaterial>,
    gpu: &AtmosphereGpu,
    planet_radius_m: f64,
    atmosphere_height_m: f64,
    bright: f32,
    look: Vec4,
) -> Entity {
    let mesh = meshes.add(Sphere::new(SHELL_MESH_RADIUS).mesh().uv(96, 48));
    let new = |pass| {
        let mut m = SkyMaterial { pass, ..SkyMaterial::new(gpu, bright) };
        m.uniform.atm.look = look;
        m
    };
    let material = materials.add(new(SkyPass::Inscatter));
    let dimming = [SkyPass::Daylight, SkyPass::Extinction].map(|pass| materials.add(new(pass)));
    let top = (planet_radius_m + atmosphere_height_m) as f32;
    let shell = commands
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_scale(Vec3::splat(top)),
            Visibility::default(),
            NoFrustumCulling,
            SkyShell { material, dimming: dimming.clone() },
        ))
        .id();
    // The dimming passes: the same shell, as children.
    for m in dimming {
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(m), Transform::IDENTITY, Visibility::Inherited, NoFrustumCulling, ChildOf(shell)));
    }
    shell
}

/// Update the sun parameters of a sky shell's materials (call each frame, or when they change).
pub fn sky_uniform_update(
    materials: &mut Assets<SkyMaterial>,
    shell: &SkyShell,
    dir_to_star: Vec3,
    sun_irradiance: Vec3,
    sun_angular_radius: f32,
    occluder: Vec4,
    disc_irradiance: Vec3,
    extra: ([Vec4; SKY_EXTRA_LIGHTS], [Vec4; SKY_EXTRA_LIGHTS]),
    glow: Vec3,
) {
    for handle in std::iter::once(&shell.material).chain(&shell.dimming) {
        if let Some(mut m) = materials.get_mut(handle) {
            m.set_sun(dir_to_star, sun_irradiance, sun_angular_radius);
            m.set_occluder(occluder, disc_irradiance);
            (m.uniform.light_dir, m.uniform.light_irradiance) = extra;
            m.uniform.glow = glow.extend(0.0);
        }
    }
}

/// The shadows on a sky shell (SE's sky.glsl RINGS and ECL): the rings (`ring` as
/// [`SkyUniform::ring`], their texture), [`SkyUniform::shadow`] and the eclipse casters; and the
/// atmosphere as drawn this frame (`atm`, its look kept), widened at a distance (SE's AtmoMinWidthPix).
pub fn sky_shadow_update(
    materials: &mut Assets<SkyMaterial>,
    shell: &SkyShell,
    ring: (Vec4, Option<&Handle<Image>>),
    shadow: Vec4,
    eclipse: [EclipseLight; SKY_EXTRA_LIGHTS + 1],
    atm: Option<AtmosphereParamsUniform>,
) {
    for handle in std::iter::once(&shell.material).chain(&shell.dimming) {
        if let Some(mut m) = materials.get_mut(handle) {
            let u = &mut m.uniform;
            u.ring = ring.0;
            u.shadow = shadow;
            u.eclipse = eclipse;
            if let Some(mut a) = atm {
                a.look = u.atm.look;
                u.atm = a;
            }
            if let Some(tex) = ring.1.filter(|t| **t != m.ring) {
                m.ring = tex.clone();
            }
        }
    }
}

/// SE's atmosphere at a distance (FUN_14044fe70:338-343): once the true `height_m` is below
/// `AtmoMinWidthPix` on screen (`min_width_m`, the pixels' size at the eye's distance), the height
/// drawn is that width capped at 0.05 of the radius (so a tall atmosphere, Titan's or Pluto's, is
/// drawn thinner from afar), eased back to the true height as the planet leaves the middle of the
/// view (`cos_view`, the cosine between the view direction and the eye's position from the planet:
/// -1 straight ahead); and SE's brightness for it, `(height / drawn) ^ 0.25`.
pub fn widened_height(height_m: f64, radius_m: f64, min_width_m: f64, cos_view: f64) -> (f64, f64) {
    let widened = if height_m <= min_width_m { min_width_m.min(radius_m * 0.05) } else { height_m };
    let k = (1.0 + cos_view).clamp(0.0, 1.0);
    let drawn = (height_m - widened) * k + widened;
    (drawn, (height_m / drawn.max(1e-9)).powf(0.25))
}

#[cfg(test)]
mod tests {
    use super::f32_to_f16_bits;

    #[test]
    fn f16_conversion() {
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        assert_eq!(f32_to_f16_bits(-2.0), 0xc000);
        assert_eq!(f32_to_f16_bits(65504.0), 0x7bff);
        assert_eq!(f32_to_f16_bits(1e-5), 0x00a8);
        assert_eq!(f32_to_f16_bits(0.0), 0);
        assert_eq!(f32_to_f16_bits(1e9), 0x7c00);
    }

    #[test]
    fn atmosphere_widened_at_a_distance() {
        use super::widened_height;
        // Earth (100 km) seen from afar, straight ahead: 2 px are 500 km there, under the 0.05 R cap.
        let (h, b) = widened_height(100e3, 6371e3, 250e3, -1.0);
        assert_eq!(h, 250e3);
        assert!((b - 0.4f64.powf(0.25)).abs() < 1e-12);
        // Capped at 0.05 R, and the true height at the edge of the view or when thick enough.
        assert_eq!(widened_height(100e3, 6371e3, 1e7, -1.0).0, 6371e3 * 0.05);
        assert_eq!(widened_height(100e3, 6371e3, 250e3, 0.0).0, 100e3);
        assert_eq!(widened_height(100e3, 6371e3, 50e3, -1.0), (100e3, 1.0));
        // Halfway: halfway between.
        assert_eq!(widened_height(100e3, 6371e3, 300e3, -0.5).0, 200e3);
        // Titan (200 km over 2575 km) from afar: drawn at 0.05 R, thinner and brighter.
        let (h, b) = widened_height(200e3, 2575e3, 400e3, -1.0);
        assert_eq!(h, 2575e3 * 0.05);
        assert!(b > 1.0);
    }

    #[test]
    fn a_model_still_being_generated_is_pending_not_missing() {
        use super::{AtmLoad, AtmosphereCache};
        use bevy::prelude::{Assets, Image};
        use bevy::tasks::{AsyncComputeTaskPool, TaskPool};
        let vfs = se_core::vfs::Vfs::new();
        let mut images = Assets::<Image>::default();
        let mut cache = AtmosphereCache::default();
        // A `.cfg`-only model while the preload (which generates it) runs: asked for again later.
        cache.preload = Some(AsyncComputeTaskPool::get_or_init(TaskPool::new).spawn(async { Vec::new() }));
        assert!(matches!(cache.get_or_load(&vfs, "Pinkworld", &mut images), AtmLoad::Pending));
        assert!(cache.models.is_empty(), "no negative entry while pending");
        // Once the preload is done, a model it did not make is missing.
        cache.preload = None;
        assert!(matches!(cache.get_or_load(&vfs, "Pinkworld", &mut images), AtmLoad::Missing));
    }
}
