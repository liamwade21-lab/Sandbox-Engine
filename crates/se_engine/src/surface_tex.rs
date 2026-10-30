//! SpaceEngine's planet-surface detail textures.
//!
//! Each body gets a biome preset (`textures/terrain/samples/*.cfg`, chosen by
//! [`se_core::biomes`]): per biome (bottom, shelf, beach, lowland, upland, rock, snow,
//! ice, lava, vegetation) a tint and two tiled "sample" textures, a large-scale and a
//! small-scale one. Close to the ground, soil materials (`textures/terrain/materials`)
//! add metre-scale detail: one soil each for lowland and upland ground, one for cliffs.
//!
//! The sample textures go into a mip-mapped texture array, decoded on a worker thread at
//! startup, with a second array of their relief: a normal map made from each sample's height
//! map (`_DISP`) and its roughness (`_ROUGH`), which lights the ground between the soils' metre
//! scales and orbit. The soils keep SE's full-size BC blocks: three arrays of colour (`_COL`), normal
//! (`_NORM`) and roughness/height/AO (`_RHAO`) maps. `planet.wesl` blends them by height, slope
//! and climate, and lights the soils with their normals, roughness, occlusion and parallax.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, ShaderType, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::biomes::{BiomeKind, BiomeLayer, BiomePreset, BodyInfo, Soil, SurfacePresets};
use se_core::catalog::BodyRecord;
use se_core::vfs::Vfs;

use crate::data::SeData;

/// Biome slots in [`BiomeUniform`] (and `planet.wesl`).
pub const BIOME_SLOTS: usize = 12;
/// Soils a body can use at once (`planet.wesl` picks the best two of them at each point).
pub const SOIL_SLOTS: usize = 32;
const SLOT_KINDS: [BiomeKind; 9] = [
    BiomeKind::Bottom,
    BiomeKind::Shelf,
    BiomeKind::Beach,
    BiomeKind::Lowland,
    BiomeKind::Upland,
    BiomeKind::Rock,
    BiomeKind::Snow,
    BiomeKind::Ice,
    BiomeKind::Lava,
];
/// Texture size of the sample array.
const LAYER_SIZE: u32 = 512;
/// Relief depth of a sample's height map, in units of its repeat period (a 4 km layer gets
/// 160 m of relief, a 100 km one 4 km).
const SAMPLE_RELIEF: f32 = 0.04;
/// SE's soil textures: 1024 texels a side with their full mip chains.
const SOIL_SIZE: u32 = 1024;
const SOIL_LEVELS: u32 = 11;
/// Period (m) of the sample layers at frequency 1. SE's frequencies (1..64) are mapped
/// through a square root and clamped, so large layers stay at 10-100 km features and
/// small ones at 0.5-4 km, leaving the metre scales to the soils (short periods tile
/// visibly).
const LARGE_PERIOD_M: (f64, f64) = (1.0e5, 1.0e4);
const SMALL_PERIOD_M: (f64, f64) = (4.0e3, 5.0e2);
/// A soil layer repeats `freq` times over this many metres (see `soil_vec`).
const SOIL_SPAN_M: f64 = 128.0;
/// Node-local soil coordinates wrap at this many meters (a multiple of every soil period).
pub const SOIL_WRAP_M: f64 = 4096.0;

/// Per-body biome data for `planet.wesl`.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct BiomeUniform {
    /// Per slot: rgb tint (linear).
    pub color: [Vec4; BIOME_SLOTS],
    /// Large / small sample layer per slot: x array layer (-1 none), y period (m),
    /// z strength, w share of the texture's own colour (vs. its luminance only).
    pub large: [Vec4; BIOME_SLOTS],
    pub small: [Vec4; BIOME_SLOTS],
    /// The body's soils: x detail layer (-1 none), y detail period (m), z modulation layer,
    /// w modulation period (m).
    pub soil: [Vec4; SOIL_SLOTS],
    /// Per soil, how much the modulation layer acts on x colour, y normals, z roughness,
    /// w height (SE's `ModColor`, `ModNorm`, `ModRough`, `ModHeight`).
    pub soil_mod: [Vec4; SOIL_SLOTS],
    /// Per soil: x `BiasRough`, y `ScaleRough`, z and w the detail and Modul layers' rotation flags
    /// (SE's `matScaleRot.zw`, for its anti-tiling).
    pub soil_bias: [Vec4; SOIL_SLOTS],
    /// Per soil: xyz its `Color` in HSV (SE's `rgb2hsl` of the stored colour), which the ground's
    /// colour is matched against; w its `Slope`.
    pub soil_color: [Vec4; SOIL_SLOTS],
    /// Per soil, its relief ("3D stones"): x `ScaleHeight`, y `BiasHeight`, z the displacement (m)
    /// of a unit of height ([`SurfaceTextures::detail_disp`] x the detail layer's period), 0 for none.
    pub soil_height: [Vec4; SOIL_SLOTS],
    /// x: biomes enabled, y: soils enabled, z: vegetation enabled, w: number of soils.
    pub flags: Vec4,
    /// SE climate parameters ([`climate_uniform`]); `climate[4].w` 0 when unused.
    pub climate: [Vec4; CLIMATE_VECS],
    /// xyz: the preset's `ColorVary` (linear), the colour the ground darkens towards over
    /// planet-wide patches; w: 1 to apply it (0 for asteroids, whose colour shader leaves it out).
    pub color_vary: Vec4,
    /// SE's gas giant clouds (`tg_gasgiant_color`): the preset's 8-colour `CloudsPalette` (as stored,
    /// gamma encoded: SE scales it by the cloud height before `colorGamma`).
    pub giant_palette: [Vec4; 8],
    /// [`SurfaceTextures::giant_uniform`]: 0 (cloudsFreq, cloudsOctaves, stripeZones, stripeTwist),
    /// 1 (stripeFluct, 1 when on, 0, 0), 2 (cycloneMagn, cycloneFreq, cycloneDensity,
    /// cycloneOctaves), 3 (the polar ones: cycloneMagn2, cycloneFreq2, cycloneLatitude2,
    /// cycloneOctaves2), 4 (SE's Randomize).
    pub giant: [Vec4; 5],
}

/// Vec4s in [`BiomeUniform::climate`] (and `planet.wesl` `Biomes`).
pub const CLIMATE_VECS: usize = 12;

/// SE's climate parameters for `planet.wesl` (heights as fractions of `BumpHeight`):
/// 0 (seaLevel, snowLevel, icecapLatitude, icecapHeight), 1 (climatePole, climateTropic,
/// climateEquator, tropicLatitude), 2 (tropicWidth, heightTempGrad, humidity, beachWidth),
/// 3 (steppe min/max, grass min/max), 4 (forest min/max, BumpHeight m, 1), 5 (colorDistMagn,
/// colorDistFreq, height of the reference radius, base of the lowland/upland bands); worlds
/// without an ocean rise from the reference radius (height 0 there: SE's `BumpOffset` is 0);
/// 6 (Randomize, 1 tidally locked / -1), 7 (hillsFreq, hillsMagn, generator, craterSqrtDensity),
/// 8 (drivenDarkening, venusMagn), and for the airless colour (tg_selena_color, tg_asteroid_color):
/// 9 (craterFreq, craterOctaves, craterRayedFactor, cracksFreq), 10 (cracksOctaves, and the
/// generator's hillsFraction and hills2Fraction thresholds, mainFreq).
pub fn climate_uniform(g: &se_core::surface_gen::SurfaceGen, bump_m: f64, ocean: bool, tidal_lock: bool) -> [Vec4; CLIMATE_VECS] {
    use se_core::surface_gen as sg;
    let v = |i: usize| g.v(i);
    [
        Vec4::new(v(sg::SEA_LEVEL), v(sg::SNOW_LEVEL), v(sg::ICECAP_LATITUDE), v(sg::ICECAP_HEIGHT)),
        Vec4::new(v(sg::CLIMATE_POLE), v(sg::CLIMATE_TROPIC), v(sg::CLIMATE_EQUATOR), v(sg::TROPIC_LATITUDE)),
        Vec4::new(v(sg::TROPIC_WIDTH), v(sg::HEIGHT_TEMP_GRAD), v(sg::HUMIDITY), v(sg::BEACH_WIDTH)),
        Vec4::new(v(sg::CLIMATE_STEPPE_MIN), v(sg::CLIMATE_STEPPE_MAX), v(sg::CLIMATE_GRASS_MIN), v(sg::CLIMATE_GRASS_MAX)),
        Vec4::new(v(sg::CLIMATE_FOREST_MIN), v(sg::CLIMATE_FOREST_MAX), bump_m as f32, 1.0),
        Vec4::new(
            v(sg::COLOR_DIST_MAGN),
            v(sg::COLOR_DIST_FREQ),
            if ocean { v(sg::SEA_LEVEL) } else { 0.0 },
            if ocean { v(sg::SEA_LEVEL) } else { 0.25 },
        ),
        Vec4::new(v(sg::RANDOMIZE), v(sg::RANDOMIZE + 1), v(sg::RANDOMIZE + 2), if tidal_lock { 1.0 } else { -1.0 }),
        Vec4::new(
            v(sg::HILLS_FREQ),
            v(sg::HILLS_MAGN),
            se_core::tg::generator(g).map_or(0.0, |k| k as f32),
            v(sg::CRATER_DENSITY).max(0.0).sqrt(),
        ),
        Vec4::new(v(sg::DRIVEN_DARKENING), v(sg::VENUS_MAGN), 0.0, 0.0),
        Vec4::new(v(sg::CRATER_FREQ), v(sg::CRATER_OCTAVES), v(sg::CRATER_RAYED_FACTOR), v(sg::CRACKS_FREQ)),
        {
            // The generator's cumulative thresholds (se_core::tg::uniforms hillsParams.zw).
            let t = sg::landform_thresholds(g).unwrap_or([0.0; 5]);
            Vec4::new(v(sg::CRACKS_OCTAVES), t[1], t[2], v(sg::MAIN_FREQ))
        },
        Vec4::ZERO,
    ]
}

/// The loaded presets and texture arrays.
#[derive(Resource)]
pub struct SurfaceTextures {
    pub presets: Arc<SurfacePresets>,
    pub samples: Handle<Image>,
    /// Per sample layer: normal xy (rg), roughness (b) and height (a).
    pub sample_relief: Handle<Image>,
    pub soils: Handle<Image>,
    pub soil_norm: Handle<Image>,
    pub soil_rhao: Handle<Image>,
    sample_layer: HashMap<String, u32>,
    soil_layer: HashMap<String, u32>,
    /// main-def.cfg `DisplaceDetailHeight` (0.1 m vertically per metre horizontally), 0 with
    /// `DisplaceDetailTextures` off: SE raises the ground near the camera by the soils' height maps
    /// (planet.glsl DET + DISP). SE scales one amplitude per planet from its detail-texture level
    /// (FUN_1404acdd0: `SurfParams3.z = DisplaceDetailHeight x R (pi/2) / 2^(a + b - 1) / 300 /
    /// BumpHeight`, levels a and b not decoded); the port takes the key's own meaning per soil, a
    /// unit of height raising the ground by this times the detail layer's period.
    pub detail_disp: f32,
}

/// Decoded texture array: `layers` images of LAYER_SIZE with full mip chains, layer-major.
struct ArrayData {
    layers: u32,
    levels: u32,
    data: Vec<u8>,
    index: HashMap<String, u32>,
}

/// Block-compressed texture array (layer-major, full mip chains).
struct BcArray {
    layers: u32,
    data: Vec<u8>,
}

struct Loaded {
    presets: SurfacePresets,
    samples: ArrayData,
    sample_relief: ArrayData,
    soils: [BcArray; 3],
    soil_index: HashMap<String, u32>,
}

#[derive(Resource)]
struct LoadTask(Task<Loaded>);

pub struct SurfaceTexturesPlugin;

impl Plugin for SurfaceTexturesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_load).add_systems(Update, finish_load);
    }
}

fn start_load(mut commands: Commands, data: Res<SeData>) {
    let vfs = data.vfs.clone();
    commands.insert_resource(LoadTask(AsyncComputeTaskPool::get().spawn(async move { load_all(&vfs) })));
}

fn finish_load(
    mut commands: Commands,
    task: Option<ResMut<LoadTask>>,
    mut images: ResMut<Assets<Image>>,
    cfg: Option<Res<crate::config::EngineConfig>>,
) {
    let Some(mut task) = task else { return };
    let Some(l) = check_ready(&mut task.0) else { return };
    commands.remove_resource::<LoadTask>();
    info!(
        "surface textures: {} presets, {} soils, {} sample and {} soil layers",
        l.presets.presets.len(),
        l.presets.soils.len(),
        l.samples.layers,
        l.soils[0].layers
    );
    let [col, norm, rhao] = l.soils;
    commands.insert_resource(SurfaceTextures {
        presets: Arc::new(l.presets),
        samples: images.add(array_image(&l.samples, TextureFormat::Rgba8UnormSrgb)),
        sample_relief: images.add(array_image(&l.sample_relief, TextureFormat::Rgba8Unorm)),
        soils: images.add(bc_image(col, TextureFormat::Bc1RgbaUnormSrgb)),
        soil_norm: images.add(bc_image(norm, TextureFormat::Bc5RgUnorm)),
        soil_rhao: images.add(bc_image(rhao, TextureFormat::Bc1RgbaUnorm)),
        sample_layer: l.samples.index,
        soil_layer: l.soil_index,
        detail_disp: cfg.as_deref().map_or(0.1, detail_displacement),
    });
}

/// main-def.cfg `DisplaceDetailHeight` (default 0.1), 0 with `DisplaceDetailTextures false`.
fn detail_displacement(cfg: &crate::config::EngineConfig) -> f32 {
    let doc = crate::galaxies::main_def_doc(cfg);
    let on = doc.as_ref().and_then(|d| d.find_deep("DisplaceDetailTextures")).and_then(|i| i.bool()).unwrap_or(true);
    let h = doc.as_ref().and_then(|d| d.find_deep("DisplaceDetailHeight")).and_then(|i| i.f64()).unwrap_or(0.1);
    if on { h.clamp(0.0, 1.0) as f32 } else { 0.0 }
}

fn load_all(vfs: &Vfs) -> Loaded {
    let presets = SurfacePresets::load(vfs);
    for w in &presets.warnings {
        warn!("biome presets: {w}");
    }
    let sample_names = presets.sample_textures();
    let samples = load_array(&sample_names, |name| {
        let bytes = vfs.read(&format!("textures/terrain/samples/{name}.png")).ok()?;
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        Some((img.width(), img.height(), img.into_raw()))
    });
    let sample_relief = load_array(&sample_names, |name| relief_layer(vfs, name));
    let mut soil_names: Vec<String> = presets
        .soils
        .iter()
        .flat_map(|s| [&s.detail, &s.modul])
        .flatten()
        .map(|l| l.texture.clone())
        .collect();
    soil_names.sort();
    soil_names.dedup();
    // Neutral blocks for missing maps: grey, flat (0.5, 0.5), and rough 0.8, height 0.5, AO 1.
    let soils = [
        load_bc(vfs, &soil_names, "COL", b"DXT1", &[0x10, 0x84, 0x10, 0x84, 0, 0, 0, 0]),
        load_bc(vfs, &soil_names, "NORM", b"ATI2", &[128, 128, 0, 0, 0, 0, 0, 0, 128, 128, 0, 0, 0, 0, 0, 0]),
        load_bc(vfs, &soil_names, "RHAO", b"DXT1", &[0x1F, 0xCC, 0x1F, 0xCC, 0, 0, 0, 0]),
    ];
    let soil_index = soil_names.iter().enumerate().map(|(i, n)| (n.to_ascii_lowercase(), i as u32)).collect();
    Loaded { presets, samples, sample_relief, soils, soil_index }
}

/// A sample's relief layer (`<base>_COL` -> `<base>_DISP`, `<base>_ROUGH`): the normal of its
/// height map at [`SAMPLE_RELIEF`] depth (xy in rg, x along the texture's u, y along v),
/// roughness in b and height in a.
fn relief_layer(vfs: &Vfs, name: &str) -> Option<(u32, u32, Vec<u8>)> {
    let base = name.len().checked_sub(4).filter(|&k| name[k..].eq_ignore_ascii_case("_col")).map(|k| &name[..k])?;
    let gray = |suffix: &str| {
        let bytes = vfs.read(&format!("textures/terrain/samples/{base}_{suffix}.png")).ok()?;
        Some(image::load_from_memory(&bytes).ok()?.to_luma8())
    };
    let disp = gray("DISP").or_else(|| gray("Disp"))?;
    let (w, h) = disp.dimensions();
    let rough = gray("ROUGH").filter(|r| r.dimensions() == (w, h));
    let at = |x: i64, y: i64| disp.get_pixel(x.rem_euclid(w as i64) as u32, y.rem_euclid(h as i64) as u32).0[0] as f32 / 255.0;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            // Height gradient per texture unit, then the normal of the scaled relief.
            let gx = (at(x + 1, y) - at(x - 1, y)) * w as f32 * 0.5 * SAMPLE_RELIEF;
            let gy = (at(x, y + 1) - at(x, y - 1)) * h as f32 * 0.5 * SAMPLE_RELIEF;
            let n = Vec3::new(-gx, -gy, 1.0).normalize();
            let r = rough.as_ref().map_or(204, |r| r.get_pixel(x as u32, y as u32).0[0]);
            out.extend_from_slice(&[((n.x * 0.5 + 0.5) * 255.0).round() as u8, ((n.y * 0.5 + 0.5) * 255.0).round() as u8, r, (at(x, y) * 255.0) as u8]);
        }
    }
    Some((w, h, out))
}

/// SE's ring rock maps (`DefaultRingsRockTexture`, rings3d.rs): one-layer arrays of the material's
/// `_COL`, `_NORM` and `_RHAO` maps, as the soils load them.
pub(crate) fn rock_maps(vfs: &Vfs, name: &str) -> [Image; 3] {
    let names = [name.to_string()];
    [
        bc_image(load_bc(vfs, &names, "COL", b"DXT1", &[0x10, 0x84, 0x10, 0x84, 0, 0, 0, 0]), TextureFormat::Bc1RgbaUnormSrgb),
        bc_image(load_bc(vfs, &names, "NORM", b"ATI2", &[128, 128, 0, 0, 0, 0, 0, 0, 128, 128, 0, 0, 0, 0, 0, 0]), TextureFormat::Bc5RgUnorm),
        bc_image(load_bc(vfs, &names, "RHAO", b"DXT1", &[0x1F, 0xCC, 0x1F, 0xCC, 0, 0, 0, 0]), TextureFormat::Bc1RgbaUnorm),
    ]
}

/// One soil map per name (`<base>_<suffix>.dds`, the name being SE's `<base>_COL`), as the
/// file's own BC blocks; a missing or unexpected file becomes `neutral` blocks.
fn load_bc(vfs: &Vfs, names: &[String], suffix: &str, fourcc: &[u8; 4], neutral: &[u8]) -> BcArray {
    let block = neutral.len();
    let len: usize = (0..SOIL_LEVELS).map(|l| ((SOIL_SIZE >> l).div_ceil(4) as usize).pow(2) * block).sum();
    let mut data = Vec::with_capacity(len * names.len().max(1));
    for name in names {
        let base = name.rsplit_once('_').map_or(name.as_str(), |(b, _)| b);
        let path = format!("textures/terrain/materials/{base}_{suffix}.dds");
        let ok = vfs.read(&path).ok().filter(|b| {
            let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
            b.len() >= 128 + len && &b[..4] == b"DDS " && u(12) == SOIL_SIZE && u(16) == SOIL_SIZE && u(28) >= SOIL_LEVELS && &b[84..88] == fourcc
        });
        match ok {
            Some(b) => data.extend_from_slice(&b[128..128 + len]),
            None => {
                warn!("soil map {path}: not found or not a {SOIL_SIZE}px {} DDS", String::from_utf8_lossy(fourcc));
                data.extend(neutral.iter().cycle().take(len));
            }
        }
    }
    if names.is_empty() {
        data.extend(neutral.iter().cycle().take(len));
    }
    BcArray {
        layers: names.len().max(1) as u32,
        data,
    }
}

fn bc_image(a: BcArray, format: TextureFormat) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: SOIL_SIZE,
            height: SOIL_SIZE,
            depth_or_array_layers: a.layers,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(a.data);
    image.texture_descriptor.mip_level_count = SOIL_LEVELS;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    image
}

/// Decode, resize to LAYER_SIZE and mip every named texture; missing ones become
/// neutral grey layers.
fn load_array(names: &[String], decode: impl Fn(&str) -> Option<(u32, u32, Vec<u8>)>) -> ArrayData {
    let mut data = Vec::new();
    let mut index = HashMap::new();
    let mut levels = 0;
    for (i, name) in names.iter().enumerate() {
        let mut rgba = match decode(name) {
            Some((w, h, px)) if w == h && w >= LAYER_SIZE && w.is_power_of_two() => {
                let (mut px, mut size) = (px, w);
                while size > LAYER_SIZE {
                    px = crate::ship_assets::box_down(&px, size, size);
                    size /= 2;
                }
                px
            }
            other => {
                if other.is_some() {
                    warn!("surface texture {name}: unsupported size");
                } else {
                    warn!("surface texture {name}: not found");
                }
                vec![128; (LAYER_SIZE * LAYER_SIZE * 4) as usize]
            }
        };
        // Repeat-tiled textures: the mip chain down to 1x1 (whose texel is the mean colour,
        // which the shader uses to normalise them).
        let mut size = LAYER_SIZE;
        levels = 1;
        data.extend_from_slice(&rgba);
        while size > 1 {
            rgba = crate::ship_assets::box_down(&rgba, size, size);
            size /= 2;
            levels += 1;
            data.extend_from_slice(&rgba);
        }
        index.insert(name.to_ascii_lowercase(), i as u32);
    }
    if names.is_empty() {
        // Arrays need a layer; keep shaders valid.
        data = vec![128; 4];
        levels = 1;
    }
    ArrayData {
        layers: names.len().max(1) as u32,
        levels,
        data,
        index,
    }
}

fn array_image(a: &ArrayData, format: TextureFormat) -> Image {
    let size = if a.levels == 1 && a.data.len() == 4 { 1 } else { LAYER_SIZE };
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: a.layers,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(a.data.clone());
    image.texture_descriptor.mip_level_count = a.levels;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

fn srgb_to_linear(c: f32) -> f32 {
    c.max(0.0).powf(2.2)
}

fn soil_rgb(s: &Soil) -> [f32; 3] {
    [srgb_to_linear(s.color[0]), srgb_to_linear(s.color[1]), srgb_to_linear(s.color[2])]
}

/// SE's `rgb2hsl` (hsl.glh), which is hue (0..1), saturation and value of a stored (gamma) colour.
fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let d = max - c[0].min(c[1]).min(c[2]);
    let h = if d <= 0.0 {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0) / 6.0
    } else if max == c[1] {
        ((c[2] - c[0]) / d + 2.0) / 6.0
    } else {
        ((c[0] - c[1]) / d + 4.0) / 6.0
    };
    [h, if max > 0.0 { d / max } else { 0.0 }, max.clamp(0.0, 1.0)]
}

/// How unlike ground of colour `g` (linear) a soil of colour `s` (linear) is: SE's colour term of
/// `ComputeMaterialWeight` (tg_detail_maps.glsl, as `planet.wesl` `soil_weight`), the summed HSV
/// gaps of the gamma-encoded colours with the hue looped one way as SE does.
fn colour_distance(g: [f32; 3], s: [f32; 3]) -> f32 {
    let hsv = |c: [f32; 3]| rgb_to_hsv(c.map(|x| x.max(0.0).powf(1.0 / 2.2)));
    let (a, b) = (hsv(g), hsv(s));
    let dh = a[0] - b[0];
    dh.min(1.0 - dh).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}

/// Up to `n` of a preset's soils: all when they fit; otherwise first the closest to each target
/// tint (ground soils for all but the last, a cliff soil for the last), then each time the one
/// least like those taken, so the ground colours of a mapped world (Earth: greens, sands, snow,
/// rock) each find a soil.
fn pick_soils<'a>(soils: &[&'a Soil], n: usize, targets: &[[f32; 3]]) -> Vec<&'a Soil> {
    let mut out: Vec<&Soil> = Vec::new();
    let add = |s: &'a Soil, out: &mut Vec<&'a Soil>| {
        if out.len() < n && !out.iter().any(|o| o.name == s.name) {
            out.push(s);
        }
    };
    let unique: Vec<&Soil> = soils.iter().enumerate().filter(|(i, s)| !soils[..*i].iter().any(|o| o.name == s.name)).map(|(_, s)| *s).collect();
    if unique.len() <= n {
        return unique;
    }
    for (k, t) in targets.iter().enumerate() {
        let cliff = k + 1 == targets.len();
        let best = unique
            .iter()
            .filter(|s| (s.slope >= 0.5) == cliff)
            .min_by(|a, b| colour_distance(*t, soil_rgb(a)).total_cmp(&colour_distance(*t, soil_rgb(b))));
        if let Some(s) = best {
            add(s, &mut out);
        }
    }
    let gap = |a: &Soil, b: &Soil| colour_distance(soil_rgb(a), soil_rgb(b)) + (a.slope - b.slope).powi(2) * 0.3;
    // Cliffs are few; the rest of the slots go to ground colours.
    let cliffs = |out: &[&Soil]| out.iter().filter(|o| o.slope >= 0.5).count();
    while out.len() < n {
        let far = unique
            .iter()
            .filter(|s| !out.iter().any(|o| o.name == s.name) && (s.slope < 0.5 || cliffs(&out) < 4))
            .max_by(|a, b| {
                let d = |s: &Soil| out.iter().map(|o| gap(s, o)).fold(f32::INFINITY, f32::min);
                d(a).total_cmp(&d(b))
            });
        match far {
            Some(s) => add(s, &mut out),
            None => break,
        }
    }
    out
}

impl SurfaceTextures {
    /// The biome preset of a body; presets without biomes (`custom_Mars.cfg`) fall back to
    /// the class-based choice.
    fn preset_for(&self, info: &BodyInfo) -> Option<&BiomePreset> {
        match self.presets.find_for(info) {
            Some(p) if !p.biomes.is_empty() => Some(p),
            _ => {
                let anon = BodyInfo {
                    name: String::new(),
                    parent: String::new(),
                    ..info.clone()
                };
                self.presets.find_for(&anon).filter(|p| !p.biomes.is_empty())
            }
        }
    }

    fn layer(&self, l: Option<&BiomeLayer>, (base_period, min_period): (f64, f64)) -> Vec4 {
        match l.and_then(|l| self.sample_layer.get(&l.texture.to_ascii_lowercase()).map(|i| (l, *i))) {
            Some((l, i)) if l.a > 0.0 => Vec4::new(
                i as f32,
                (base_period / (l.freq.max(1.0) as f64).sqrt()).max(min_period) as f32,
                l.a.min(1.0),
                l.b.clamp(0.0, 1.0),
            ),
            _ => Vec4::new(-1.0, 1.0, 0.0, 0.0),
        }
    }

    fn soil_vec(&self, s: &Soil) -> Vec4 {
        let layer = |l: Option<&se_core::biomes::SoilLayer>| {
            // The second number is not a weight: SE's table has it as the layer's rotation flag
            // (`matScaleRot.zw`, its anti-tiling lookups), and samples the layer either way.
            l.and_then(|l| self.soil_layer.get(&l.texture.to_ascii_lowercase()).map(|i| (*i, l.freq)))
        };
        // A layer's `freq` is how often it repeats, not its size: the finest photos get the
        // largest values (sand grains 256, grass blades 96-128) and the coarsest the smallest
        // (sand ripples and snow dunes 4-12, grass's patchy dirt 2-5). SE passes the integer to
        // its (encrypted) shader as is; the period is SOIL_SPAN_M / freq, snapped to divide
        // SOIL_WRAP_M.
        let period = |f: f32| {
            let p = SOIL_SPAN_M / f.max(1.0) as f64;
            (SOIL_WRAP_M / (SOIL_WRAP_M / p).round().max(1.0)) as f32
        };
        let (d, dp) = layer(s.detail.as_ref()).map_or((-1.0, 1.0), |(i, f)| (i as f32, period(f)));
        let (m, mp) = layer(s.modul.as_ref()).map_or((-1.0, 1.0), |(i, f)| (i as f32, period(f)));
        Vec4::new(d, dp, m, mp)
    }

    /// SE's water colours of a body (FUN_1404ae4a0): its preset's `OceanPalette` `Deep`, the
    /// WaterSurfColor (linear, before `ModulateColor`), and `Absorption`, the WaterFogColor (rgb and
    /// opacity, per km). SE's defaults (FUN_140166410) where the preset has none, or the body no
    /// preset.
    pub fn ocean_palette(&self, info: &BodyInfo) -> ([f32; 3], [f32; 4]) {
        let p = self.presets.find_for(info);
        let deep = p.and_then(|p| p.ocean_palette).unwrap_or([0.04, 0.1, 0.2, 1.0]);
        let fog = p.and_then(|p| p.ocean_absorption).unwrap_or([40.0, 5.0, 1.0, 5.0]);
        ([srgb_to_linear(deep[0]), srgb_to_linear(deep[1]), srgb_to_linear(deep[2])], fog)
    }

    /// File name of the biome preset a body gets.
    pub fn preset_file(&self, info: &BodyInfo) -> Option<String> {
        self.preset_for(info).map(|p| p.file.clone())
    }

    /// A giant's biome uniform: biomes off, its preset's `CloudsPalette` and SE's cloud parameters
    /// for `HeightMapCloudsGasGiant` in `planet.wesl` (`None` without a preset with 8 cloud layers).
    pub fn giant_uniform(&self, info: &BodyInfo, g: &se_core::surface_gen::SurfaceGen) -> Option<BiomeUniform> {
        use se_core::surface_gen as sg;
        let preset = self.presets.find_for(info)?;
        if preset.clouds_layers.len() < 8 {
            return None;
        }
        let mut u = BiomeUniform::default();
        for (i, c) in preset.clouds_layers.iter().take(8).enumerate() {
            u.giant_palette[i] = Vec4::new(c[0], c[1], c[2], c[3]);
        }
        let v = |i: usize| g.v(i);
        u.giant = [
            Vec4::new(v(sg::CLOUDS_MAIN_FREQ).max(0.1), v(sg::CLOUDS_MAIN_OCTAVES).clamp(1.0, 15.0), v(sg::STRIPE_ZONES), v(sg::STRIPE_TWIST)),
            Vec4::new(v(sg::STRIPE_FLUCT), 1.0, 0.0, 0.0),
            Vec4::new(v(sg::CYCLONE_MAGN), v(sg::CYCLONE_FREQ), v(sg::CYCLONE_DENSITY), v(sg::CYCLONE_OCTAVES)),
            Vec4::new(v(sg::CYCLONE_MAGN2), v(sg::CYCLONE_FREQ2), v(sg::CYCLONE_LATITUDE2), v(sg::CYCLONE_OCTAVES2)),
            Vec4::new(v(0), v(1), v(2), 0.0),
        ];
        Some(u)
    }

    /// Build the biome uniform of a body (`None` for giants and bodies without a preset).
    pub fn biome_uniform(&self, info: &BodyInfo) -> Option<BiomeUniform> {
        let preset = self.preset_for(info)?;
        let mut u = BiomeUniform::default();
        // SE's `ColorVary` is no shift of the palette but a per-pixel darkening, `color *=
        // mix(colorVary, 1, albedoVary)` (tg_terra_color.glsl:158, tg_selena_color.glsl:138), which
        // `tg_asteroid_color` never applies. The stored colours are gamma encoded, as `Color`.
        let asteroid = info.se.is_some_and(|c| c.bulk == se_core::surface_gen::bulk::ASTEROID);
        let cv = preset.color_vary.map(|c| c.clamp(0.0, 1.0).powf(2.2));
        u.color_vary = Vec4::new(cv[0], cv[1], cv[2], if asteroid { 0.0 } else { 1.0 });
        let biome_rgb = |k: BiomeKind| preset.biome(k).map(|b| [srgb_to_linear(b.color[0]), srgb_to_linear(b.color[1]), srgb_to_linear(b.color[2])]);
        let lowland = preset.biome(BiomeKind::Lowland);
        let mut set = |slot: usize, k: BiomeKind| {
            let b = preset.biome(k).or(lowland);
            let c = biome_rgb(k).or_else(|| biome_rgb(BiomeKind::Lowland)).unwrap_or([0.3, 0.28, 0.25]);
            u.color[slot] = Vec4::new(c[0], c[1], c[2], 1.0);
            u.large[slot] = self.layer(b.and_then(|b| b.large.as_ref()), LARGE_PERIOD_M);
            u.small[slot] = self.layer(b.and_then(|b| b.small.as_ref()), SMALL_PERIOD_M);
        };
        for (slot, k) in SLOT_KINDS.iter().enumerate() {
            set(slot, *k);
        }
        // Vegetation: organic or exotic, multicellular or unicellular.
        let veg = match (info.life_organic, info.life_exotic, info.multicellular) {
            (true, _, true) => [BiomeKind::SteppeOrgM, BiomeKind::GrassOrgM, BiomeKind::ForestOrgM],
            (true, _, false) => [BiomeKind::SteppeOrgU, BiomeKind::GrassOrgU, BiomeKind::ForestOrgU],
            (false, true, true) => [BiomeKind::SteppeExoM, BiomeKind::GrassExoM, BiomeKind::ForestExoM],
            _ => [BiomeKind::SteppeExoU, BiomeKind::GrassExoU, BiomeKind::ForestExoU],
        };
        for (i, k) in veg.into_iter().enumerate() {
            set(9 + i, k);
        }
        // Soils. SE lays them by the ground's colour and slope: Earth's biomes are all one
        // grey-beige, yet its grassland gets the grass soils listed among its "Mineral" ones. The
        // shader picks the best two of up to SOIL_SLOTS soils at each point; vegetation soils come
        // from the life collection of the body's life (SE ships only the multicellular ones).
        let veg_on = info.life_organic || info.life_exotic;
        let soils_in = |coll: &str| -> Vec<&Soil> {
            preset.soils_of(coll).unwrap_or(&[]).iter().filter_map(|n| self.presets.soil(n)).collect()
        };
        let veg_coll = if info.life_organic { "LifeOrganicMulti" } else { "LifeExoticMulti" };
        let mut veg_soils = if veg_on { soils_in(veg_coll) } else { Vec::new() };
        veg_soils.truncate(SOIL_SLOTS / 2);
        let tint = |slot: usize| [u.color[slot].x, u.color[slot].y, u.color[slot].z];
        let mineral = pick_soils(&soils_in("Mineral"), SOIL_SLOTS - veg_soils.len(), &[tint(3), tint(4), tint(5)]);
        let chosen: Vec<&Soil> = mineral.into_iter().chain(veg_soils).collect();
        for (i, s) in chosen.iter().enumerate() {
            u.soil[i] = self.soil_vec(s);
            u.soil_mod[i] = Vec4::new(s.mod_color, s.mod_norm, s.mod_rough, s.mod_height);
            let rot = |l: Option<&se_core::biomes::SoilLayer>| l.map_or(0.0, |l| l.rot);
            u.soil_bias[i] = Vec4::new(s.bias_rough, s.scale_rough, rot(s.detail.as_ref()), rot(s.modul.as_ref()));
            let c = rgb_to_hsv([s.color[0], s.color[1], s.color[2]]);
            u.soil_color[i] = Vec4::new(c[0], c[1], c[2], s.slope);
            let period = if u.soil[i].x >= 0.0 { u.soil[i].y } else { u.soil[i].w };
            u.soil_height[i] = Vec4::new(s.scale_height, s.bias_height, self.detail_disp * period.max(0.0), 0.0);
        }
        let n = chosen.len();
        debug!("soils of {} ({}): {:?}", info.name, preset.file, chosen.iter().map(|s| s.name.as_str()).collect::<Vec<_>>());
        u.flags = Vec4::new(1.0, if n > 0 { 1.0 } else { 0.0 }, if veg_on { 1.0 } else { 0.0 }, n as f32);
        Some(u)
    }
}

/// What the presets need to know about a catalog body.
pub fn body_info(cat: &se_core::catalog::Catalog, b: &BodyRecord, temperature_k: f64, has_atmosphere: bool, has_ocean: bool) -> BodyInfo {
    let life = se_core::body_params::LifeParams::all_from_body(&b.block);
    let has = |class: &str| life.iter().any(|l| l.class.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(class)));
    let parent = cat
        .find(&b.parent)
        .map(|r| cat.display_name(r).to_string())
        .unwrap_or_else(|| b.parent.clone());
    BodyInfo {
        name: b.names.first().cloned().unwrap_or_default(),
        parent,
        class: b.class.clone().unwrap_or_default(),
        temperature_k,
        has_atmosphere,
        has_ocean,
        life_organic: has("Organic"),
        life_exotic: has("Exotic"),
        multicellular: life.iter().any(|l| l.kind.as_deref().is_some_and(|k| k.to_ascii_lowercase().contains("multi"))),
        se: None,
        sun_temp: None,
        surf_style: b
            .block
            .sub("Surface")
            .and_then(se_core::biomes::surf_style)
            .unwrap_or_else(|| (se_core::proc_surface::name_seed(b.names.first().map_or("", |s| s.as_str())) % 10_000) as f64 / 10_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::surface_gen as sg;

    #[test]
    fn climate_uniform_carries_the_airless_colour_parameters() {
        let mut g = sg::SurfaceGen { p: [sg::UNSET; sg::N_PARAMS], color: None, classes: Default::default() };
        g.p[sg::CRATER_FREQ] = 12.0;
        g.p[sg::CRATER_OCTAVES] = 9.0;
        g.p[sg::CRATER_RAYED_FACTOR] = 0.4;
        g.p[sg::CRACKS_FREQ] = 0.7;
        g.p[sg::CRACKS_OCTAVES] = 3.0;
        g.p[sg::MAIN_FREQ] = 1.5;
        let c = climate_uniform(&g, 5000.0, false, false);
        assert_eq!(c.len(), CLIMATE_VECS);
        assert_eq!(c[9], Vec4::new(12.0, 9.0, 0.4, 0.7));
        assert_eq!((c[10].x, c[10].w), (3.0, 1.5));
        // Unset slots read 0, as the shader expects (no rays, no cracks).
        let none = climate_uniform(&sg::SurfaceGen { p: [sg::UNSET; sg::N_PARAMS], color: None, classes: Default::default() }, 1.0, false, false);
        assert_eq!((none[9].z, none[10].x), (0.0, 0.0));
    }
}
