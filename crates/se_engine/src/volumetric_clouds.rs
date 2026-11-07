//! Volumetric clouds (docs/spec-volumetric-clouds-weather.md): the cloud layers of the body the camera
//! is at, ray-marched as full-screen passes over the solids' corner of the main texture, after the main
//! pass and before the solids' upscale (render_scale.rs), so before bloom and tone mapping
//! (`volumetric_clouds.wesl`).
//!
//! The body's SE cloud layers (terrain.rs `CloudLayer`, up to [`MAX_LAYERS`]) keep their heights, relief,
//! colour, wind and cover: a cover map (a cube on SE's tile faces, a channel per layer, in each layer's
//! own frame) is built once per body on the GPU from SE's terra cloud generator, the port's procedural
//! clouds or the catalog cloud maps (loaded here from the install's tiles), and read turned by each
//! layer's drift. Each frame:
//!
//! 1. `depth_down`: the scene's distance at the cloud resolution (the solids' size divided by 2 at
//!    `VolumetricClouds` 2, Ultra, or by 4 at 1, High), the nearest and farthest of each block in a
//!    checkerboard, for the depth-aware upscale;
//! 2. `trace`: the ray-march from the camera to the scene, its start jittered per pixel and frame, lit
//!    by the star through the atmosphere (the body's scattering tables) and the sky, then the haze in
//!    front of the clouds;
//! 3. `resolve`: blended with the previous frames' result, reprojected through the body's frame (so
//!    the floating origin and the body's turn cancel out) and clamped to the new neighbourhood;
//! 4. `composite`: upscaled by depth and blended over the scene (premultiplied light, transmittance).
//!
//! The cover is shaped by the body's weather (`weather_sim`): a weather cube made each frame from the
//! derived weather of the date and a coarse live simulation on the GPU, and near the camera the
//! clouds' shapes drift with the wind.
//!
//! Only the main camera draws them, and not while an offscreen view exists. The body's flat cloud shells (terrain.rs) are hidden only once
//! these passes have drawn it ([`VolumetricClouds::body`]): until the pipelines, the noise volumes and
//! the body's cover map are ready, and on Medium and below, the shells stay.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::{CameraUpdateSystems, MainPassResolutionOverride};
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::math::{DMat3, DMat4, DQuat, DVec3};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, texture_2d_array, texture_3d, texture_cube, texture_depth_2d,
    texture_depth_2d_multisampled, texture_storage_2d_array, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::storage::ShaderBuffer;
use bevy::render::texture::{CachedTexture, FallbackImage, GpuImage, TextureCache};
use bevy::render::view::{Msaa, ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::planet_tiles::{CubeFace, TileKey, TileLayer};

use crate::atmosphere::AtmosphereParamsUniform;
use crate::camera::{FlyCamera, MainCamera};
use crate::graphics::GraphicsSettings;
use crate::switches::RenderSwitches;
use crate::system::{AmbientLighting, BodyLighting, SystemBody, MAX_LIGHTS};
use crate::terrain::{ATM_SUN_SCALE, CloudLayer, Terrain};
use crate::universe::SimTime;

mod detail;
mod weather_sim;
pub(crate) use weather_sim::wind_at;

/// The most cloud layers of a body drawn volumetric (SE gives terrestrial bodies at most 3).
pub const MAX_LAYERS: usize = 4;
/// The cover map: a cube of this many texels per face side (Earth: about 5 km a texel, a cloud pixel
/// from 5,000 km at Ultra), with every mip level.
const COVER_SIZE: u32 = 2048;
const COVER_MIPS: u32 = 12;
/// The catalog cloud maps are resampled to faces of this size.
const MAPPED_SIZE: usize = 2048;
/// The shape volume repeats every 6 km (cells of 1.5 km), the detail volume every 900 m.
const SHAPE_PERIOD_M: f32 = 6_000.0;
const DETAIL_PERIOD_M: f32 = 900.0;
/// The ground's cloud shadow map: texels a side, and its half-width (m) around the camera.
const SHADOW_SIZE: u32 = 512;
const SHADOW_HALF_M: f64 = 40_000.0;
/// Its mips, down to 8 texels a side (10 km each): the ground takes the map's shapes at the cover
/// shadow's mean darkness, the map's own mean read from the last (planet.wesl `cloud_shadow`).
const SHADOW_MIPS: u32 = 7;
/// Bodies farther than this many radii get no volumetric clouds (a few pixels across).
const MAX_RANGE_RADII: f64 = 60.0;
/// Gas giants' decks fade in below this height (scale heights above the radius) and are whole below
/// `GIANT_FULL`: from higher up SE's own flat bands are sharper than the decks' colour map.
const GIANT_NEAR: f64 = 6.0;
const GIANT_FULL: f64 = 2.0;
/// The near pass (spec step 6): the clouds within this distance of the camera (m) traced at half the
/// solids' size, with their own history and shorter steps (the rest at the cloud buffers' size). Near
/// clouds move fast across the view, so the frames' blend cannot sharpen them as it does the far ones.
/// (At the solids' size on Ultra it cost 2.8 ms among the Amazon's afternoon cumulus, 3440x1440.)
const NEAR_RANGE_ULTRA_M: f32 = 4_000.0;
const NEAR_RANGE_HIGH_M: f32 = 2_000.0;
/// The passes fade in over this long (real seconds) once their body is ready.
const READY_FADE_S: f32 = 1.0;
/// The weather in the air and on the flat shells is whole within this many radii and fades out
/// towards `MAX_RANGE_RADII`.
const MEDIA_FULL_RADII: f64 = 40.0;
/// Gas giants' noise periods (m): their billows are larger than a terrestrial cloud's.
const GIANT_SHAPE_PERIOD_M: f32 = 40_000.0;
const GIANT_DETAIL_PERIOD_M: f32 = 6_000.0;

const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// The light volume (clouds plan step 6, after blackrack's, docs/ksp-clouds-notes.md section 3): a grid
/// over the cloud layers round the spot under the camera, `LIGHT_XY` x `LIGHT_XY` texels along the
/// ground (finer near its centre) by `LIGHT_Z` up, holding the star's optical depth from each point to
/// the clouds' top and the cloud's straight up (the sky light's). The march reads it for the long
/// self-shadows and the sky light past its first two light steps. Built `LIGHT_SLICES_PER_FRAME`
/// slices a frame into the back of two volumes, which then swap (all of it every 8 frames).
const LIGHT_XY: u32 = 160;
const LIGHT_Z: u32 = 32;
const LIGHT_SLICES_PER_FRAME: u32 = 4;
/// Its half-width (m): physical, as the clouds' sizes are (about 190 m texels at its centre, 1.3 km at
/// its edge).
const LIGHT_HALF_M: f32 = 60_000.0;
const LIGHT_FORMAT: TextureFormat = TextureFormat::Rg16Float;
const COVER_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const PATTERN_FORMAT: TextureFormat = TextureFormat::R8Unorm;

/// The body the render world last drew volumetric clouds for (`Entity::to_bits() + 1`, 0 none).
#[derive(Resource, Clone, Default)]
pub struct CloudsReady(Arc<AtomicU64>);

/// The main world's view of the volumetric clouds; terrain.rs hides the flat shells of `body`.
#[derive(Resource, Default, Debug)]
pub struct VolumetricClouds {
    pub body: Option<Entity>,
    /// The body the passes work on (drawn or not yet). camera.rs `update_nearest` keeps it in linear
    /// space: the passes read the depth buffer as true distances, which a body compressed into scaled
    /// space (another body nearer, a moon) does not hold, and the clouds were lost.
    pub active: Option<Entity>,
    /// `body`'s flat shells are hidden (these passes draw its clouds alone: within `MEDIA_FULL_RADII`;
    /// out to `MAX_RANGE_RADII` the shells cross-fade with them, `CloudShadow::sun.w`).
    pub flat_hidden: bool,
}

/// The cover map (an image, so the ground's materials can read it for the clouds' shadows) and the
/// shadow record every ground material reads (terrain.rs `PlanetMaterial`).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct CloudShadowAssets {
    pub cover: Handle<Image>,
    /// The weather's own cloud's pattern (`volumetric_clouds.wesl` `weather_pattern`), built with the
    /// cover map, at its size.
    pub pattern: Handle<Image>,
    pub shadow: Handle<ShaderBuffer>,
    /// The transmittance towards the star of the clouds over the ground around the camera.
    pub map: Handle<Image>,
    /// The active body's weather cube (`weather_sim`), shaping the cover.
    pub weather: Handle<Image>,
    /// The derived weather maps it is made from (`weather_sim`).
    pub derived: Handle<Image>,
}

/// The clouds' shadow on the ground of the body they draw. Layout must match `planet.wesl`
/// `CloudShadow`.
#[derive(Clone, Copy, ShaderType, Default, Debug, PartialEq)]
struct CloudShadow {
    /// x: the body's radius (m), y: its seed fraction (as the ground's `proc_params2.xy`), z: 1 on,
    /// w: how much the weather shapes the body's flat shells (clouds.wesl).
    key: Vec4,
    /// xyz: towards the star in the body frame; w: the flat shells' share of the clouds (1 - the
    /// passes' share, `CloudView::lod` x).
    sun: Vec4,
    layer_a: [Vec4; MAX_LAYERS],
    /// As [`CloudView::layer_b`], w: the layer's index in the body's cloud layers + 1 (its flat shell).
    layer_b: [Vec4; MAX_LAYERS],
    /// The shadow map's frame ([`CloudView::shadow_c`]).
    map_c: Vec4,
    map_e1: Vec4,
    map_e2: Vec4,
    /// The Viltrumite mod's hero ([`HeroSoil`]).
    hero: Vec4,
    hero_end: Vec4,
}

/// Where the ground should hold still for the Viltrumite mod's hero, a line from the camera (m): `start` his soles' place on the
/// ground, w how much it holds; `end` where his shadow ends. All 0 with the mod off or nothing of him over land. Goes in the
/// `CloudShadow` record, the ground's one per-frame buffer that is written in place: `planet.wesl`'s soil displacement fades out
/// round that line, so his feet and his shadow's plane, which go by the ground's heights, lie on what's drawn.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub(crate) struct HeroSoil {
    pub start: Vec4,
    pub end: Vec4,
}

fn shadow_buffer(s: &CloudShadow) -> ShaderBuffer {
    let mut bytes = encase::StorageBuffer::new(Vec::<u8>::new());
    bytes.write(s).expect("cloud shadow");
    ShaderBuffer::new(bytes.into_inner(), RenderAssetUsages::RENDER_WORLD)
}

fn init_cloud_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let mut cover = Image::new_uninit(
        Extent3d { width: COVER_SIZE, height: COVER_SIZE, depth_or_array_layers: 6 },
        TextureDimension::D2,
        COVER_FORMAT,
        RenderAssetUsages::RENDER_WORLD,
    );
    cover.texture_descriptor.mip_level_count = COVER_MIPS;
    cover.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST;
    cover.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    cover.sampler = bevy::image::ImageSampler::linear();
    let mut pattern = cover.clone();
    pattern.texture_descriptor.format = PATTERN_FORMAT;
    let mut map = Image::new_target_texture(SHADOW_SIZE, SHADOW_SIZE, TextureFormat::R16Float, None);
    map.texture_descriptor.mip_level_count = SHADOW_MIPS;
    // (Drawn every frame: no data to upload, which would have to hold every mip.)
    map.data = None;
    map.sampler = bevy::image::ImageSampler::linear();
    commands.insert_resource(CloudShadowAssets {
        cover: images.add(cover),
        pattern: images.add(pattern),
        shadow: buffers.add(shadow_buffer(&CloudShadow::default())),
        map: images.add(map),
        weather: images.add(weather_sim::weather_cube()),
        derived: images.add(weather_sim::derived_image()),
    });
}

/// The tiling noise volumes (`se_core::cloud_noise`), once built.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
struct CloudNoise {
    shape: Handle<Image>,
    detail: Handle<Image>,
}

#[derive(Resource, Default)]
struct NoiseJob(Option<Task<(Vec<u8>, Vec<u8>)>>);

/// One layer's cover generator. Layout must match `volumetric_clouds.wesl` `CoverLayer`.
#[derive(Clone, Copy, ShaderType, Default, Debug, PartialEq)]
struct CoverLayer {
    /// x kind (0 none, 1 catalog map, 2 SE's terra clouds, 3 the port's procedural clouds, 4 overcast),
    /// y the mapped cover's channel, z coverage.
    kind: Vec4,
    se: [Vec4; 3],
    proc: Vec4,
    proc2: Vec4,
}

/// Layout must match `volumetric_clouds.wesl` `CoverGen`.
#[derive(Clone, Copy, ShaderType, Default, Debug, PartialEq)]
struct CoverGen {
    layers: [CoverLayer; MAX_LAYERS],
    /// xyz: the star in the layers' frame (tidally locked worlds' substellar storm), w: the body's seed
    /// (0..1, the weather's cloud pattern).
    sun: Vec4,
    /// A gas giant's SE cloud preset (`surface_tex::BiomeUniform::giant_palette`, `giant`), for its
    /// main deck's colour and height (gas_giant.wesl).
    giant_palette: [Vec4; 8],
    giant: [Vec4; 5],
}

/// The active body's cloud setup for the render world; a new `id` builds its cover map again.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
struct CloudBody {
    id: u64,
    body: Option<Entity>,
    cover: CoverGen,
    /// The catalog cloud maps' cover (6 faces, a channel per layer), once loaded.
    mapped: Option<Handle<Image>>,
    /// The cover map waits for `mapped`.
    mapped_pending: bool,
    /// The body's scattering tables: inscatter, transmittance, irradiance.
    atm: Option<[Handle<Image>; 3]>,
    /// Its rings' texture, for their shadow (`CloudView::ring`), set as their material appears.
    ring: Option<Handle<Image>>,
}

/// The main camera's cloud pass inputs. Layout must match `volumetric_clouds.wesl` `CloudView`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug)]
#[extract_app(RenderApp)]
pub struct CloudView {
    /// Camera-relative world (universe axes, the camera at the origin) from clip.
    pub world_from_clip: Mat4,
    /// The previous frame's clip from this frame's camera-relative world, through the body's frame.
    pub prev_clip_from_world: Mat4,
    /// Sphere space (the body frame divided by the ellipsoid's semi-axes, m) from world.
    pub sphere_from_world: Mat4,
    /// xyz: the camera in sphere space (m), w: the body's reference radius (m).
    pub cam: Vec4,
    /// x: |cam| (m), y: the camera's altitude in sphere space (m), z: the lowest layer's bottom (m),
    /// w: 1 to drop the history.
    pub cam2: Vec4,
    /// Per layer: x bottom, y top (m above the radius), z opacity (0: no layer), w how cumuliform.
    pub layer_a: [Vec4; MAX_LAYERS],
    /// Per layer: x, y cos and sin of the layer's turn about the pole, z extinction per m.
    pub layer_b: [Vec4; MAX_LAYERS],
    /// Per layer: rgb albedo, w SE's DayAmbient.
    pub layer_c: [Vec4; MAX_LAYERS],
    /// xyz: towards the star (world), w: 1 when lit.
    pub sun_dir: Vec4,
    /// rgb: the star's light above the atmosphere, in the tables' scale; w: SE's `AmbientLighting`, in
    /// the same scale.
    pub sun_light: Vec4,
    /// x: 1 when drawn, y: shape noise frequency (1/m), z: detail noise frequency (1/m), w: divisor.
    pub params: Vec4,
    /// x: atmosphere on, y: the haze's light relative to the sun's, z: the cover map's texel (m), w:
    /// the highest layer's top (m).
    pub atm_flags: Vec4,
    /// The ground's shadow map: xyz its centre in sphere space (m, on the reference sphere), w its
    /// half-width (m); `shadow_e1`, `shadow_e2` its axes (unit, tangent).
    pub shadow_c: Vec4,
    pub shadow_e1: Vec4,
    pub shadow_e2: Vec4,
    /// xyz: the shape noise's and the detail noise's drift with the wind (sphere space, m, wrapped to
    /// their periods), set by `weather_sim`.
    pub noise_off: Vec4,
    pub detail_off: Vec4,
    /// xyz: the clouds' drift since the last frame (world, m), for the reprojection.
    pub motion: Vec4,
    /// The weather effects: x rain and snow, y lightning, z fog, w dust (1 on).
    pub fx: Vec4,
    /// x: the effects' clock (s, wrapped), y: the body's mean temperature (K), z: lightning's
    /// brightness (the tables' scale), w: 1 to show the weather map (`SE_WEATHER_DEBUG=1`).
    pub fx2: Vec4,
    /// xyz: the wind at the camera (sphere space, m/s).
    pub wind: Vec4,
    /// A gas giant's decks: x 1 for a giant, y their density (fading in as the camera comes down), z
    /// the atmosphere's scale height (m).
    pub giant: Vec4,
    /// The far cross-fade: x the clouds' share (the ray-march; the flat shells draw the rest), y the
    /// weather in the air's (fog, dust, rain shafts); the near pass: z its range (m, 0 off), w its
    /// divisor of the solids' size.
    pub lod: Vec4,
    /// The further lights (`BodyLighting::extra`: SE's other stars, the planets' and moons' shine), which
    /// light the night side: xyz towards each (world), w 1 when used; rgb their light above the
    /// atmosphere, in the tables' scale.
    pub extra_dir: [Vec4; MAX_LIGHTS - 1],
    pub extra_light: [Vec4; MAX_LIGHTS - 1],
    /// The body's rings, for their shadow on the clouds (as terrain.rs gives the ground): x inner, y
    /// outer radius (radii), z opacity, w 0 none, 0.5 untextured, 1 textured; `ring_frame` xyz the
    /// body's semi-axes over its radius (sphere space to the body frame, where the rings lie in the
    /// equator: a flattened giant's ring shadow in sphere space fell in the wrong places), w SE's rings
    /// winter strength (terrain.rs `ring_normal`).
    pub ring: Vec4,
    pub ring_frame: Vec4,
    /// A hot gas giant's own glow (terrain.rs `thermal_uniform`, as its flat bands take it): x its
    /// temperature (thousand K), y the star's heating at the subsolar point, z the glow's scale (SE's
    /// `thermal2.x` times the self-emission scale; 0 none), w the star's temperature (K).
    pub glow: Vec4,
    /// rgb: SE's GlowColorAtmo (a glowing atmosphere's light in its haze, as the ground's), in the
    /// tables' scale.
    pub glow_atmo: Vec4,
    /// The cloud bands, highest first: x bottom, y top (m above the radius), the layers' heights merged
    /// where they overlap (`cloud_bands`); unused ones y < x. The march spreads its steps over the ray's
    /// pieces inside them alone.
    pub bands: [Vec4; MAX_LAYERS],
    /// Each layer's detail cover patches (`detail`, `DETAIL_LEVELS` a layer, finest first): xyz the
    /// centre in the layer's frame, w the tangent of the half-width (0: none).
    pub detail: [Vec4; MAX_LAYERS * detail::DETAIL_LEVELS],
    pub atm: AtmosphereParamsUniform,
}

pub struct VolumetricCloudsPlugin;

impl Plugin for VolumetricCloudsPlugin {
    fn build(&self, app: &mut App) {
        let ready = CloudsReady::default();
        app.insert_resource(ready.clone())
            .init_resource::<VolumetricClouds>()
            .init_resource::<HeroSoil>()
            .init_resource::<detail::DetailCover>()
            .init_resource::<NoiseJob>()
            .init_resource::<CloudBody>()
            .init_resource::<weather_sim::SimFrame>()
            .add_systems(Startup, init_cloud_assets)
            .add_systems(Update, build_noise)
            // After the bodies, their lighting and the camera have moved this frame (a date jump's new
            // sun makes its weather maps).
            .add_systems(
                PostUpdate,
                (update_cloud_view, weather_sim::update_weather)
                    .chain()
                    .after(CameraUpdateSystems)
                    .after(crate::universe::UniverseSet::FloatingOrigin),
            )
            .add_plugins((
                ExtractComponentPlugin::<CloudView>::default(),
                UniformComponentPlugin::<CloudView>::default(),
                ExtractResourcePlugin::<CloudNoise>::default(),
                ExtractResourcePlugin::<CloudBody>::default(),
                ExtractResourcePlugin::<CloudShadowAssets>::default(),
                ExtractResourcePlugin::<weather_sim::SimFrame>::default(),
                ExtractResourcePlugin::<detail::DetailCover>::default(),
            ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .insert_resource(ready)
            .init_resource::<CloudHistory>()
            .init_resource::<NearHistory>()
            .init_resource::<LightVolume>()
            .init_resource::<CoverMap>()
            .init_resource::<weather_sim::SimGpu>()
            .init_resource::<detail::DetailGpu>()
            .add_systems(RenderStartup, (init_pipelines, weather_sim::init_sim_pipelines))
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            .add_systems(
                Core3d,
                clouds_pass.in_set(Core3dSystems::EarlyPostProcess).before(crate::render_scale::SolidUpscaleSet),
            );
    }
}

/// Builds the noise volumes on a worker thread the first time the clouds are wanted.
fn build_noise(
    mut commands: Commands,
    mut job: ResMut<NoiseJob>,
    mut images: ResMut<Assets<Image>>,
    noise: Option<Res<CloudNoise>>,
    gs: Res<GraphicsSettings>,
) {
    if noise.is_some() {
        return;
    }
    if job.0.is_none() {
        if gs.quality.volumetric_clouds > 0 {
            job.0 = Some(AsyncComputeTaskPool::get().spawn(async {
                let t = std::time::Instant::now();
                let out = (se_core::cloud_noise::shape_volume(), se_core::cloud_noise::detail_volume());
                info!("volumetric clouds: noise volumes built in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
                out
            }));
        }
        return;
    }
    let Some((shape, detail)) = job.0.as_mut().and_then(check_ready) else { return };
    job.0 = None;
    let mut volume = |data: Vec<u8>, n: usize| {
        let (all, levels) = se_core::cloud_noise::with_mips(data, n);
        let n = n as u32;
        let mut image = Image::new(
            Extent3d { width: n, height: n, depth_or_array_layers: n },
            TextureDimension::D3,
            all[..(n * n * n * 4) as usize].to_vec(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.data = Some(all);
        image.texture_descriptor.mip_level_count = levels;
        images.add(image)
    };
    commands.insert_resource(CloudNoise {
        shape: volume(shape, se_core::cloud_noise::SHAPE_SIZE),
        detail: volume(detail, se_core::cloud_noise::DETAIL_SIZE),
    });
}

/// One of the active body's drawn layers.
#[derive(Clone, Debug, PartialEq)]
struct LayerSpec {
    /// Its index in `Terrain::clouds`.
    index: usize,
    cover: CoverLayer,
    /// The volume: bottom, top (m above the radius), opacity, how cumuliform.
    geom: Vec4,
    extinction: f32,
    color: Vec3,
    day_ambient: f32,
    /// Its catalog cloud map.
    map: Option<TileLayer>,
    /// Some for a catalog gas giant's main deck: `map` is the giant's colour map, composed in colour
    /// (`compose_face`) and tinted by this (SE's `ModulateColor`).
    tint: Option<Vec3>,
}

/// The body's drawable cloud layers as volumes. SE's layer `Height` is the cloud shell's altitude and
/// `BumpHeight` its tops' relief. The lowest layer stands for the clouds of the lower atmosphere, as a
/// single cloud map does (Earth's), on every body: cumuliform, from a base an eighth of its height over
/// the ground (`Terrain::ground_mid_m`) up to the shell, its clouds rising higher where the cover is
/// denser. Higher layers are thinner sheets around their shells.
fn layer_specs(terrain: &Terrain) -> Vec<LayerSpec> {
    if terrain.proc.kind.is_giant() {
        return giant_specs(terrain);
    }
    let layers = terrain.tile_layers();
    let ground = terrain.ground_mid_m();
    let mut out = Vec::new();
    let mut mapped = 0;
    for (i, c) in terrain.clouds.iter().enumerate() {
        if out.len() == MAX_LAYERS {
            break;
        }
        let map = match &c.map {
            Some(_) => match layers.and_then(|l| l.clouds.get(i).cloned().flatten()) {
                Some(m) => Some(m),
                None => continue,
            },
            None if terrain.cloud_proc.is_none() => continue,
            None => None,
        };
        let slot = out.len();
        let cover = layer_cover(terrain, c, i, slot, &map, &mut mapped);
        let (bottom, top, cumulus) = layer_volume(c, out.is_empty(), ground);
        out.push(LayerSpec {
            index: i,
            cover,
            geom: Vec4::new(bottom, top, c.opacity.clamp(0.0, 1.0), cumulus),
            extinction: 0.008 + 0.022 * cumulus,
            color: c.color,
            day_ambient: c.look.x,
            map: map.filter(|_| !c.opaque_map),
            tint: None,
        });
    }
    out
}

/// Gas giants' volumetric decks (clouds plan step 8) near and below their cloud tops; `SE_GIANT_DECKS=0`
/// leaves SE's flat bands alone (comparisons).
fn giant_decks_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_GIANT_DECKS").map_or(true, |v| v.trim() != "0"))
}

/// How deep the camera may go into a gas giant (m above its radius): two scale heights under the
/// view's dark limit (`giant_dark` in the shader: the deep deck's bottom, -6 H, less two more). None for
/// other bodies and while the decks are off (the flat bands are then its surface); terrain.rs
/// `keep_camera_above_ground` also asks that the passes work on the giant.
pub(crate) fn giant_floor_m(terrain: &Terrain) -> Option<f64> {
    (terrain.proc.kind.is_giant() && giant_decks_on() && terrain.biome.is_some()).then(|| -10.0 * giant_scale_height(terrain))
}

/// A gas giant's atmosphere's scale height (m).
fn giant_scale_height(terrain: &Terrain) -> f64 {
    // (SE's model atmosphere gives Jupiter a few km; its decks span tens of km.)
    terrain.atmosphere.as_ref().map_or(25_000.0, |a| a.uniform().radii.z as f64 * 1000.0).clamp(20_000.0, 200_000.0)
}

/// The cloud bands (`CloudView::bands`): the drawn layers' heights merged where they overlap or lie
/// within 500 m, highest first. (The march once spread its steps over the whole shell from the lowest
/// layer's bottom to the highest's top: from between a moon's heaps at 12-21 km and its veils at 90-137
/// km they were kilometres long at the heaps, which became rings round the spot under the camera, and
/// the heaps appeared only lower down: the user's 2026-11-04 video.)
fn cloud_bands(specs: &[LayerSpec]) -> [Vec4; MAX_LAYERS] {
    let mut h: Vec<(f32, f32)> = specs.iter().filter(|s| s.geom.z > 0.0 && s.geom.y > s.geom.x).map(|s| (s.geom.x, s.geom.y)).collect();
    h.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out = [Vec4::new(0.0, -1.0, 0.0, 0.0); MAX_LAYERS];
    let mut n = 0;
    for (lo, hi) in h {
        if n > 0 && hi >= out[n - 1].x - 500.0 {
            out[n - 1].x = out[n - 1].x.min(lo);
        } else if n < MAX_LAYERS {
            out[n] = Vec4::new(lo, hi, 0.0, 0.0);
            n += 1;
        } else {
            out[n - 1].x = out[n - 1].x.min(lo);
        }
    }
    out
}

/// `SE_CLOUD_LOD=0` brings back the hard switch: the passes alone within `MAX_RANGE_RADII`, the flat
/// shells without the weather (comparisons).
fn lod_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_CLOUD_LOD").map_or(true, |v| v.trim() != "0"))
}

/// `SE_NEAR_PASS=0` leaves the near pass out (comparisons).
fn near_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_NEAR_PASS").map_or(true, |v| v.trim() != "0") && !reference_on())
}

/// `SE_CLOUD_REFERENCE=1`: the clouds as a reference to judge shapes and light against (slow): every
/// pixel at the solids' size traced every frame over the whole ray with twice the steps, no history,
/// no near pass.
fn reference_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_CLOUD_REFERENCE").is_ok_and(|v| v.trim() == "1"))
}

fn smooth01(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How dense a gas giant's decks are at the camera's height (m above the radius): none from above,
/// where SE's own flat bands are sharper, whole near and in the cloud tops.
fn giant_fade(height: f64, scale: f64) -> f64 {
    let t = ((GIANT_NEAR * scale - height) / ((GIANT_NEAR - GIANT_FULL) * scale)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A gas giant's cloud decks (scale height H, the radius the flat bands' level): a thin haze high
/// above (2-4 H, off), the main deck in the band colours with its zones higher than its belts (-2 to
/// 1.5 H), and a dark deep deck (-6 to -3.5 H). Each deck is opaque or nearly so. The main deck's
/// colours are SE's generator's (its preset), or a catalog giant's own colour map (Jupiter), as its
/// flat surface draws them.
fn giant_specs(terrain: &Terrain) -> Vec<LayerSpec> {
    let h = giant_scale_height(terrain) as f32;
    let deck = |kind: f32, bottom: f32, top: f32, opacity: f32, cumulus: f32, tau: f32, color: Vec3| LayerSpec {
        index: usize::MAX,
        cover: CoverLayer { kind: Vec4::new(kind, 0.0, 1.0, 0.0), ..default() },
        geom: Vec4::new(bottom * h, top * h, opacity, cumulus),
        extinction: tau / ((top - bottom) * h),
        color,
        day_ambient: 0.0,
        map: None,
        tint: None,
    };
    let mut main = deck(5.0, -2.0, 1.5, 1.0, 0.6, 60.0, Vec3::ONE);
    if let Some(color) = terrain.tile_layers().and_then(|l| l.color.clone()) {
        main.cover.kind.y = 1.0;
        main.map = Some(color);
        main.tint = Some(terrain.surf_modulate.truncate());
    }
    vec![
        main,
        // Off (opacity 0): SE's atmosphere is the haze. (Even at a vertical optical depth of 0.015 it
        // made a grey fog of the sky from inside it, where SE's sky is blue.)
        deck(6.0, 2.0, 4.0, 0.0, 0.0, 0.03, Vec3::new(0.9, 0.85, 0.75)),
        deck(7.0, -6.0, -3.5, 1.0, 0.3, 80.0, Vec3::new(0.35, 0.25, 0.16)),
    ]
}

/// A layer's bottom, top (m above the radius) and how cumuliform it is, its heights counted over the
/// ground (m above the radius). (SE's generated layers stand 4-30 km over the ground; once only a
/// lowest layer under 15 km was cumuliform, which left most generated worlds with veils alone.) The
/// lowest layer reaches at most 9 km over its base (Earth's 8.1): its heaps stand within 4 km of it,
/// and a thicker layer spreads the march's steps. A layer with an opaque map (Venus's deck) stays a
/// sheet round its shell.
fn layer_volume(c: &CloudLayer, lowest: bool, ground: f32) -> (f32, f32, f32) {
    let h = c.height as f32;
    let over = (h - ground).max(0.0);
    if lowest && !c.opaque_map {
        let bottom = ground + (0.125 * over).clamp(400.0, 2000.0);
        return (bottom, (h + 0.5 * c.bump_m).clamp(bottom + 1500.0, bottom + 9000.0), 1.0);
    }
    let cumulus = (1.0 - (over - 3000.0) / 5000.0).clamp(0.0, 1.0);
    let thick = (c.bump_m * 1.5).max(1500.0 * (0.4 + 0.6 * cumulus)).min(over.max(1000.0) * 1.6);
    let bottom = (h - 0.3 * thick).max(ground + 150.0);
    (bottom, bottom + thick, cumulus)
}

fn layer_cover(terrain: &Terrain, c: &CloudLayer, index: usize, slot: usize, map: &Option<TileLayer>, mapped: &mut usize) -> CoverLayer {
    let mut cover = CoverLayer::default();
    if c.opaque_map {
        cover.kind = Vec4::new(4.0, 0.0, 1.0, 0.0);
    } else if map.is_some() {
        cover.kind = Vec4::new(1.0, slot as f32, 1.0, 0.0);
        *mapped += 1;
    } else if let Some(p) = terrain.cloud_proc {
        let se = crate::terrain::se_cloud_uniform(&terrain.surf_gen, terrain.tidal_lock, true, index);
        cover.kind = Vec4::new(if se[0].x > 0.0 { 2.0 } else { 3.0 }, 0.0, p.x, 0.0);
        cover.se = se;
        cover.proc = p.truncate().extend(1.0);
        cover.proc2 = Vec4::new(
            (terrain.proc.seed as f32 / u64::MAX as f32 + index as f32 * 0.379).fract(),
            0.0,
            if terrain.proc.kind.is_giant() { 0.0 } else { 1.0 },
            0.0,
        );
    }
    cover
}

/// The catalog cloud maps of the mapped layers resampled onto the cover map's faces: `MAPPED_SIZE`^2
/// RGBA per face, layer `slot`'s opacity (the map's red) in channel `slot`, from the layer's tiles at
/// level 4 (or its deepest), averaged down; a thread a face.
fn compose_maps(vfs: &se_core::vfs::Vfs, maps: &[(usize, TileLayer, Option<Vec3>)]) -> Vec<u8> {
    let started = std::time::Instant::now();
    let n = MAPPED_SIZE;
    let mut out = vec![0u8; 6 * n * n * 4];
    std::thread::scope(|scope| {
        for (face, part) in CubeFace::ALL.into_iter().zip(out.chunks_mut(n * n * 4)) {
            scope.spawn(move || compose_face(vfs, maps, face, part));
        }
    });
    // A giant's colour map: its cloud height from its brightness, spread over 0..1 about the mean
    // (bright zones high, dark belts low, as on Jupiter).
    if maps.iter().any(|m| m.2.is_some()) {
        let n = out.len() / 4;
        let mean = out.chunks_exact(4).map(|p| p[3] as f64).sum::<f64>() / n as f64;
        let sd = (out.chunks_exact(4).map(|p| (p[3] as f64 - mean).powi(2)).sum::<f64>() / n as f64).sqrt().max(1.0);
        for p in out.chunks_exact_mut(4) {
            p[3] = ((0.5 + (p[3] as f64 - mean) / (4.0 * sd)).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    info!("volumetric clouds: cloud maps composed in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
    out
}

/// One face of `compose_maps` (`out` its texels): a cloud map's red channel into its slot's channel; a
/// giant's colour map (`tint` Some) into all four, rgb as the square root of its linear colour (as
/// the generator's, `cover_gen`), a its brightness.
fn compose_face(vfs: &se_core::vfs::Vfs, maps: &[(usize, TileLayer, Option<Vec3>)], face: CubeFace, out: &mut [u8]) {
    let n = MAPPED_SIZE;
    let linear = |v: f32| {
        let s = v / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    for (slot, layer, tint) in maps {
        // Level 4 averaged down: about two of its texels to one of the face's.
        let level = layer.max_level.min(4);
        let tiles = 1usize << level;
        for row in 0..tiles {
            for col in 0..tiles {
                let key = TileKey { face, level, row: row as u32, col: col as u32 };
                let Some(img) = vfs.read(&layer.tile_path(key)).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8()) else {
                    continue;
                };
                let (w, h) = img.dimensions();
                let k = w as f32 / 256.0;
                let channel = |c: usize, x: f32, y: f32| {
                    let (x0, y0) = (x.floor().clamp(0.0, w as f32 - 1.0), y.floor().clamp(0.0, h as f32 - 1.0));
                    let (x1, y1) = ((x0 + 1.0).min(w as f32 - 1.0), (y0 + 1.0).min(h as f32 - 1.0));
                    let (fx, fy) = ((x - x0).clamp(0.0, 1.0), (y - y0).clamp(0.0, 1.0));
                    let p = |x: f32, y: f32| img.get_pixel(x as u32, y as u32)[c] as f32;
                    let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * fx;
                    let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * fx;
                    top + (bot - top) * fy
                };
                let span = n / tiles;
                for y in row * span..(row + 1) * span {
                    for x in col * span..(col + 1) * span {
                        // The tile's inner 252 texels (as clouds.wesl `tile_uv`), 2 x 2 samples over the
                        // output texel's footprint.
                        let mut v = Vec3::ZERO;
                        for (sx, sy) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                            let lu = ((x as f32 + 0.5 + sx) / n as f32) * tiles as f32 - col as f32;
                            let lv = ((y as f32 + 0.5 + sy) / n as f32) * tiles as f32 - row as f32;
                            let (tx, ty) = ((2.0 + lu * 252.0) * k - 0.5, (2.0 + lv * 252.0) * k - 0.5);
                            v += match tint {
                                Some(_) => Vec3::new(linear(channel(0, tx, ty)), linear(channel(1, tx, ty)), linear(channel(2, tx, ty))),
                                None => Vec3::splat(channel(0, tx, ty)),
                            } / 4.0;
                        }
                        let at = (y * n + x) * 4;
                        match tint {
                            Some(t) => {
                                let c = (v * *t).clamp(Vec3::ZERO, Vec3::ONE);
                                let rgb = c.to_array().map(|e| (e.sqrt() * 255.0).round() as u8);
                                out[at..at + 3].copy_from_slice(&rgb);
                                out[at + 3] = ((0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z).sqrt() * 255.0).round() as u8;
                            }
                            None => out[at + slot] = v.x.round().clamp(0.0, 255.0) as u8,
                        }
                    }
                }
            }
        }
    }
}

fn mapped_image(data: Vec<u8>) -> Image {
    let n = MAPPED_SIZE as u32;
    let mut image = Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: 6 },
        TextureDimension::D2,
        data,
        COVER_FORMAT,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image
}

/// What the previous frame's reprojection needs.
struct PrevFrame {
    body: Entity,
    clip_from_view: DMat4,
    view_rot: DQuat,
    sphere_from_world: DMat3,
    cam_s: DVec3,
}

/// The previous frame's clip from this frame's camera-relative world: a point `x` is `s x + cam_s` in
/// the body's sphere space, the same point the previous frame saw at `prev.s^-1 (that - prev.cam_s)`.
fn reprojection(prev: &PrevFrame, s: DMat3, cam_s: DVec3) -> DMat4 {
    prev.clip_from_view
        * DMat4::from_quat(prev.view_rot.inverse())
        * DMat4::from_mat3(prev.sphere_from_world.inverse())
        * DMat4::from_translation(cam_s - prev.cam_s)
        * DMat4::from_mat3(s)
}

/// The active body's setup and its catalog map job.
#[derive(Default)]
struct SetupState {
    specs: Vec<LayerSpec>,
    body: Option<Entity>,
    atm: bool,
    job: Option<Task<Vec<u8>>>,
}

/// The active body (the nearest one with drawable cloud layers, within range), its setup, and the
/// camera's cloud view.
#[allow(clippy::too_many_arguments)]
fn update_cloud_view(
    mut commands: Commands,
    (gs, switches, time, data, captures, real, ambient, emission): (
        Res<GraphicsSettings>,
        Res<RenderSwitches>,
        Res<SimTime>,
        Res<crate::data::SeData>,
        Query<(), With<crate::capture::CaptureView>>,
        Res<Time<Real>>,
        Res<AmbientLighting>,
        Res<crate::terrain::SelfEmission>,
    ),
    (cam, hero_soil): (Res<FlyCamera>, Res<HeroSoil>),
    ready: Res<CloudsReady>,
    noise: Option<Res<CloudNoise>>,
    (mut state, mut cloud_body, mut images): (ResMut<VolumetricClouds>, ResMut<CloudBody>, ResMut<Assets<Image>>),
    (assets, mut buffers, mut last_shadow): (Option<Res<CloudShadowAssets>>, ResMut<Assets<ShaderBuffer>>, Local<CloudShadow>),
    (mut detail_cover, mut detail_jobs): (ResMut<detail::DetailCover>, Local<detail::DetailJobs>),
    (rings, ring_mats): (Query<&crate::rings::Rings>, Res<Assets<crate::rings::RingMaterial>>),
    bodies: Query<(Entity, &SystemBody, &Terrain, &BodyLighting)>,
    mut cams: Query<(Entity, &Projection, Option<&mut CloudView>), With<MainCamera>>,
    mut prev: Local<Option<PrevFrame>>,
    mut setup: Local<SetupState>,
    mut map_frame: Local<Option<(Entity, DVec3, DVec3, DVec3)>>,
    mut ready_fade: Local<(Option<Entity>, f32)>,
) {
    let Ok((cam_e, proj, view)) = cams.single_mut() else { return };
    let quality = gs.quality.volumetric_clouds.min(2);
    // The offscreen views (HD screenshot tiles, skybox faces, projection modes) don't draw these
    // passes yet: while one exists the flat shells stand in everywhere (spec step 9).
    let enabled = quality > 0 && switches.clouds && captures.is_empty();
    let active = enabled
        .then(|| {
            bodies
                .iter()
                // Bodies with cloud layers, and those with only an atmosphere for its fog and dust.
                .filter(|(_, _, t, _)| {
                    // (A giant once its SE preset is chosen: its decks' colours come from it.)
                    !t.clouds.is_empty()
                        || (t.atmosphere.is_some() && (if t.proc.kind.is_giant() { giant_decks_on() && t.biome.is_some() } else { switches.fog || switches.dust }))
                })
                .map(|(e, b, t, l)| (e, b, t, l, cam.pos.delta_m(b.pos)))
                .filter(|(_, _, t, _, rel)| {
                    // A gas giant's decks only once the camera comes down towards them.
                    rel.length() < if t.proc.kind.is_giant() { t.radius + GIANT_NEAR * giant_scale_height(t) } else { MAX_RANGE_RADII * t.radius }
                })
                .min_by(|a, b| (a.4.length() - a.2.radius).total_cmp(&(b.4.length() - b.2.radius)))
        })
        .flatten();
    // The setup, rebuilt when the body or its drawable layers change.
    let specs = active.map(|a| layer_specs(a.2)).unwrap_or_default();
    let atm = active.is_some_and(|a| a.2.atmosphere.is_some());
    let body = active.filter(|a| !specs.is_empty() || a.2.atmosphere.is_some()).map(|a| a.0);
    if body != setup.body || specs != setup.specs || atm != setup.atm {
        let terrain = active.map(|a| a.2);
        if let (Some(e), Some(t)) = (body, terrain) {
            let kinds: Vec<String> = specs
                .iter()
                .map(|s| format!("{:.1}-{:.1} km kind {} cover {:.2}", s.geom.x / 1000.0, s.geom.y / 1000.0, s.cover.kind.x, s.cover.kind.z))
                .collect();
            info!(
                "volumetric clouds: {} layers on body {e:?} (radius {:.0} km, ground {:.1} km): {}",
                specs.len(),
                t.radius / 1000.0,
                t.ground_mid_m() / 1000.0,
                kinds.join(", ")
            );
        }
        let maps: Vec<(usize, TileLayer, Option<Vec3>)> = specs.iter().enumerate().filter_map(|(i, s)| s.map.clone().map(|m| (i, m, s.tint))).collect();
        setup.job = (!maps.is_empty()).then(|| {
            let vfs = data.vfs.clone();
            AsyncComputeTaskPool::get().spawn(async move { compose_maps(&vfs, &maps) })
        });
        let mut generator = CoverGen::default();
        for (i, s) in specs.iter().enumerate() {
            generator.layers[i] = s.cover;
        }
        if let Some(b) = active.map(|a| a.2).filter(|t| t.proc.kind.is_giant()).and_then(|t| t.biome.as_ref()) {
            generator.giant_palette = b.giant_palette;
            generator.giant = b.giant;
        }
        if let Some((_, b, t, l, _)) = active.filter(|a| a.2.tidal_lock) {
            let spin = t.clouds.first().map_or(0.0, |c| c.spin_at(time.jd));
            generator.sun = (DQuat::from_rotation_y(-spin) * b.orient.inverse() * l.sun_dir.as_dvec3()).as_vec3().extend(0.0);
        }
        generator.sun.w = active.map_or(0.0, |a| (a.2.proc.seed % 100_003) as f32 / 100_003.0);
        *cloud_body = CloudBody {
            id: cloud_body.id + 1,
            body,
            cover: generator,
            mapped: None,
            mapped_pending: setup.job.is_some(),
            atm: active.and_then(|a| a.2.atmosphere.as_ref()).map(|a| [a.inscatter.clone(), a.transmittance.clone(), a.irradiance.clone()]),
            ring: None,
        };
        *setup = SetupState { specs: specs.clone(), body, atm, job: setup.job.take() };
    }
    if let Some(data) = setup.job.as_mut().and_then(check_ready) {
        setup.job = None;
        cloud_body.mapped = Some(images.add(mapped_image(data)));
        cloud_body.mapped_pending = false;
    }
    let mut out = CloudView::default();
    let mut shadow = CloudShadow::default();
    // The clouds' and the weather's shares by distance (the far cross-fade), for the active body.
    let mut shares = (0.0f64, 0.0f64);
    // The near pass's range and divisor, when the camera is within the range of the cloud layers.
    let mut near_pass = Vec2::ZERO;
    match active.filter(|_| body.is_some()) {
        Some((e, b, terrain, light, rel)) => {
            let giant = terrain.proc.kind.is_giant();
            let s = DMat3::from_diagonal(DVec3::ONE / b.shape) * DMat3::from_quat(b.orient.inverse());
            let cam_s = s * rel;
            let r = cam_s.length();
            let clip_from_view = proj.get_clip_from_view().as_dmat4();
            let view_rot = cam.rot;
            let world_from_clip = DMat4::from_quat(view_rot) * clip_from_view.inverse();
            let (prev_clip_from_world, reset) = match prev.as_ref().filter(|p| p.body == e) {
                Some(p) => (reprojection(p, s, cam_s), false),
                None => (clip_from_view * DMat4::from_quat(view_rot.inverse()), true),
            };
            let mut layer_a = [Vec4::ZERO; MAX_LAYERS];
            let mut layer_b = [Vec4::ZERO; MAX_LAYERS];
            let mut layer_c = [Vec4::ZERO; MAX_LAYERS];
            let mut spins = [0.0f64; MAX_LAYERS];
            for (i, sp) in specs.iter().enumerate() {
                let spin = terrain.clouds.get(sp.index).map_or(0.0, |c| c.spin_at(time.jd));
                spins[i] = spin;
                layer_a[i] = sp.geom;
                layer_b[i] = Vec4::new(spin.cos() as f32, spin.sin() as f32, sp.extinction, 0.0);
                layer_c[i] = sp.color.extend(sp.day_ambient);
            }
            // No layers (an atmosphere's fog and dust only): both 0, the shells empty.
            let lowest = if specs.is_empty() { 0.0 } else { specs.iter().map(|s| s.geom.x).fold(f32::MAX, f32::min) };
            let highest = specs.iter().map(|s| s.geom.y).fold(0.0, f32::max);
            let lit = light.sun_intensity > 0.0;
            let (extra_dirs, extra_colors) = light.extra_uniforms(false);
            // Its rings (terrain.rs takes the ground's the same way).
            let ring = rings.iter().find(|r| r.body == e).and_then(|r| ring_mats.get(&r.material)).map(|m| (m.texture.clone(), m.u.geom.truncate().extend(if m.u.params.z > 0.5 { 1.0 } else { 0.5 })));
            if cloud_body.ring.as_ref() != ring.as_ref().map(|r| &r.0) {
                cloud_body.ring = ring.as_ref().map(|r| r.0.clone());
            }
            // The shadow map: under the camera, kept still until the camera is a quarter of its
            // width away (its texels stay put on the ground, so its shadows don't crawl).
            let under = cam_s.normalize_or(DVec3::Y) * terrain.radius;
            let keep = map_frame.filter(|(fe, c, ..)| *fe == e && (*c - under).length() < SHADOW_HALF_M * 0.25);
            let (_, mc, e1, e2) = keep.unwrap_or_else(|| {
                let n = under.normalize();
                let e1 = n.any_orthonormal_vector();
                (e, under, e1, n.cross(e1))
            });
            *map_frame = Some((e, mc, e1, e2));
            // A hot giant's glow, as terrain.rs gives its flat bands (planet.wesl `thermal_emission`).
            let thermal = crate::terrain::thermal_uniform(terrain, light);
            let glow = if giant && (thermal.x > 0.45 || thermal.z > 0.04) && emission.scale() > 0.0 {
                let d = light.star_pos.delta_m(b.pos).length().max(1.0);
                let omega = (light.star_radius_m / d).powi(2).max(1e-30);
                Vec4::new(thermal.x, thermal.z, (1.0 / (std::f64::consts::PI * omega)) as f32 * emission.scale(), light.star_teff)
            } else {
                Vec4::ZERO
            };
            let atm_u = terrain
                .atmosphere
                .as_ref()
                .filter(|_| switches.atmospheres)
                .map(|a| AtmosphereParamsUniform { look: terrain.atm_look, ..a.uniform() });
            out = CloudView {
                world_from_clip: world_from_clip.as_mat4(),
                prev_clip_from_world: prev_clip_from_world.as_mat4(),
                sphere_from_world: DMat4::from_mat3(s).as_mat4(),
                cam: cam_s.as_vec3().extend(terrain.radius as f32),
                cam2: Vec4::new(r as f32, (r - terrain.radius) as f32, lowest, if reset { 1.0 } else { 0.0 }),
                layer_a,
                layer_b,
                layer_c,
                sun_dir: light.sun_dir.extend(if lit { 1.0 } else { 0.0 }),
                sun_light: (light.sun_color * light.sun_intensity * ATM_SUN_SCALE).extend(ambient.0 * ATM_SUN_SCALE),
                params: Vec4::new(
                    1.0,
                    1.0 / if giant { GIANT_SHAPE_PERIOD_M } else { SHAPE_PERIOD_M },
                    1.0 / if giant { GIANT_DETAIL_PERIOD_M } else { DETAIL_PERIOD_M },
                    if reference_on() {
                        1.0
                    } else if quality == 2 {
                        2.0
                    } else {
                        4.0
                    },
                ),
                atm_flags: Vec4::new(
                    atm_u.is_some() as u32 as f32,
                    crate::terrain::scatter_rel(terrain.atm_bright),
                    (terrain.radius * std::f64::consts::FRAC_PI_2 / COVER_SIZE as f64) as f32,
                    highest,
                ),
                shadow_c: mc.as_vec3().extend(SHADOW_HALF_M as f32),
                shadow_e1: e1.as_vec3().extend(0.0),
                shadow_e2: e2.as_vec3().extend(0.0),
                // The drift is weather_sim's (it runs after this).
                noise_off: Vec4::ZERO,
                detail_off: Vec4::ZERO,
                motion: Vec4::ZERO,
                fx: Vec4::ZERO,
                fx2: Vec4::ZERO,
                wind: Vec4::ZERO,
                giant: Vec4::new(
                    giant as u32 as f32,
                    giant_fade(r - terrain.radius, giant_scale_height(terrain)) as f32,
                    giant_scale_height(terrain) as f32,
                    0.0,
                ),
                lod: Vec4::ONE,
                extra_dir: extra_dirs,
                extra_light: extra_colors.map(|c| (c.truncate() * ATM_SUN_SCALE).extend(0.0)),
                ring: ring.as_ref().map_or(Vec4::ZERO, |r| r.1),
                ring_frame: b.shape.as_vec3().extend(match (&ring, terrain.surface.rings_winter) {
                    (Some((_, g)), Some(w)) => (w * 0.5 * (g.x + g.y) as f64) as f32,
                    _ => 0.0,
                }),
                glow,
                bands: cloud_bands(&specs),
                glow_atmo: (terrain.glow_atmo(emission.scale()) * ATM_SUN_SCALE).extend(0.0),
                detail: [Vec4::ZERO; MAX_LAYERS * detail::DETAIL_LEVELS],
                atm: atm_u.unwrap_or_default(),
            };
            // The ray-march draws the body's clouds at every height; the flat shells take over
            // (cross-fading) only between `MEDIA_FULL_RADII` and `MAX_RANGE_RADII`, where the body is a
            // few degrees across. (A gas giant's decks keep their own fade, `giant_fade`.)
            let fade = if giant { 1.0 } else { smooth01(MAX_RANGE_RADII * terrain.radius, MEDIA_FULL_RADII * terrain.radius, r) };
            shares = (fade, fade);
            let range = if quality == 2 { NEAR_RANGE_ULTRA_M } else { NEAR_RANGE_HIGH_M };
            let near_div = 2.0;
            if lod_on() && near_on() && !giant && !specs.is_empty() && r - terrain.radius < (highest + range) as f64 {
                near_pass = Vec2::new(range, near_div);
            }
            let mut layer_b = layer_b;
            for (i, sp) in specs.iter().enumerate() {
                layer_b[i].w = sp.index as f32 + 1.0;
            }
            shadow = CloudShadow {
                key: Vec4::new(terrain.radius as f32, (terrain.proc.seed % 10_000) as f32 / 10_000.0, if giant { 0.0 } else { 1.0 }, 0.0),
                sun: (b.orient.inverse() * light.sun_dir.as_dvec3()).as_vec3().extend(0.0),
                layer_a,
                layer_b,
                map_c: out.shadow_c,
                map_e1: out.shadow_e1,
                map_e2: out.shadow_e2,
                hero: Vec4::ZERO,
                hero_end: Vec4::ZERO,
            };
            // The detail cover's levels, each once the camera is low enough for its texels to show.
            detail::update(&mut detail_cover, &mut detail_jobs, cloud_body.id, &specs, &spins, cam_s, terrain.radius, lod_on() && !giant, &data.vfs);
            out.detail = detail_cover.frames();
            *prev = Some(PrevFrame { body: e, clip_from_view, view_rot, sphere_from_world: s, cam_s });
        }
        None => {
            *prev = None;
            *map_frame = None;
        }
    }
    let drawn = ready.0.load(Ordering::Relaxed);
    let shown = body.filter(|e| noise.is_some() && drawn == e.to_bits() + 1);
    if state.body != shown {
        state.body = shown;
    }
    if state.active != body {
        state.active = body;
    }
    // The fades: in over READY_FADE_S once the passes draw the body, out at once without them (a
    // new body, the clouds off).
    let ready_share = match shown {
        Some(e) if ready_fade.0 == Some(e) => (ready_fade.1 + real.delta_secs() / READY_FADE_S).min(1.0),
        _ => 0.0,
    };
    *ready_fade = (shown, ready_share);
    let (clouds_share, air_share, weather_share) = if lod_on() {
        let r = ready_share as f64;
        ((r * shares.0) as f32, (r * shares.1) as f32, (r * shares.1) as f32)
    } else {
        (1.0, 1.0, 0.0)
    };
    out.lod = Vec4::new(clouds_share, air_share, near_pass.x, near_pass.y);
    let hidden = shown.is_some() && (!lod_on() || clouds_share >= 0.999);
    if state.flat_hidden != hidden {
        state.flat_hidden = hidden;
    }
    // The ground's shadows only once the clouds draw (the cover map is complete).
    if shown.is_none() {
        shadow = CloudShadow::default();
    }
    shadow.key.w = weather_share;
    shadow.sun.w = 1.0 - clouds_share;
    shadow.hero = hero_soil.start;
    shadow.hero_end = hero_soil.end;
    if let Some(a) = assets
        && shadow != *last_shadow
        && let Some(mut b) = buffers.get_mut(&a.shadow)
    {
        *b = shadow_buffer(&shadow);
        *last_shadow = shadow;
    }
    match view {
        Some(mut v) => *v = out,
        None => {
            commands.entity(cam_e).insert(out);
        }
    }
}

// ---------------------------------------------------------------------------
// Render world.

/// The per-frame sizes. Layout must match `volumetric_clouds.wesl` `CloudSize`.
#[derive(Clone, Copy, ShaderType)]
struct CloudSize {
    /// xy: the solids' size (px), zw: the cloud buffers' used size.
    size: Vec4,
    /// x: divisor, y: 1 to drop the history (2 also twice the steps: `reference_on`), z: frame number,
    /// w: the part of the ray traced (0 all of it, 1 beyond the near pass's range, 2 within it).
    params: Vec4,
    /// The trace buffer: xy its used size, z this frame's phase (0..3), w 1 when it holds a quarter of
    /// the cloud texels (`quarter_on`).
    trace: Vec4,
}

/// The light volume's placement. Layout must match `volumetric_clouds.wesl` `LightFrame`.
#[derive(Clone, Copy, Default, ShaderType)]
struct LightFrame {
    /// xyz the unit direction (sphere space) of its centre, w its half-width (m).
    c: Vec4,
    /// Its axes along the ground.
    e1: Vec4,
    e2: Vec4,
    /// x, y the heights (m above the radius) of its bottom and top, z 1 when it holds light, w the radius.
    h: Vec4,
    /// The star's direction (sphere space) it was built for.
    sun: Vec4,
}

/// One slice's build. Layout must match `volumetric_clouds.wesl` `LightGen`.
#[derive(Clone, Copy, Default, ShaderType)]
struct LightGen {
    frame: LightFrame,
    /// x the slice, y the texels a side, z the slices.
    slice: Vec4,
}

/// The two light volumes: the front one read by the march, the back one being built.
#[derive(Resource, Default)]
struct LightVolume {
    tex: Vec<(Texture, TextureView)>,
    front: usize,
    /// Their placements (the front's `h.z` 0 until one is complete).
    frames: [LightFrame; 2],
    /// The back one's next slice (`LIGHT_Z`: none being built).
    next: u32,
    /// The body (`CloudBody::id`) they are for.
    body: u64,
}

impl LightVolume {
    /// One frame's work: start a build round the spot under the camera when none is going (only while
    /// the camera is near enough the clouds for the march to want it), and the slices to build now
    /// (into the back volume, at its placement). Swaps the volumes once one is complete.
    fn advance(&mut self, device: &RenderDevice, cv: &CloudView, body: u64) -> std::ops::Range<u32> {
        if self.tex.is_empty() {
            self.tex = ["vol_clouds_light_a", "vol_clouds_light_b"]
                .map(|label| {
                    let t = device.create_texture(&TextureDescriptor {
                        label: Some(label),
                        size: Extent3d { width: LIGHT_XY, height: LIGHT_XY, depth_or_array_layers: LIGHT_Z },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D3,
                        format: LIGHT_FORMAT,
                        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    });
                    let v = t.create_view(&TextureViewDescriptor::default());
                    (t, v)
                })
                .into();
            self.next = LIGHT_Z;
        }
        if self.body != body {
            self.body = body;
            self.frames = Default::default();
            self.next = LIGHT_Z;
        }
        let wanted = cv.giant.x < 0.5 && cv.atm_flags.w > 0.0 && cv.cam2.y < cv.atm_flags.w + 0.5 * LIGHT_HALF_M;
        if !wanted {
            self.frames[self.front].h.z = 0.0;
            self.next = LIGHT_Z;
            return 0..0;
        }
        if self.next >= LIGHT_Z {
            let c = cv.cam.truncate().normalize_or(Vec3::Y);
            let pole = if c.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
            let e1 = pole.cross(c).normalize();
            let sun = (cv.sphere_from_world * cv.sun_dir.truncate().normalize_or(Vec3::Y).extend(0.0)).truncate().normalize_or(Vec3::Y);
            self.frames[1 - self.front] = LightFrame {
                c: c.extend(LIGHT_HALF_M),
                e1: e1.extend(0.0),
                e2: c.cross(e1).extend(0.0),
                h: Vec4::new(cv.cam2.z, cv.atm_flags.w, 1.0, cv.cam.w),
                sun: sun.extend(0.0),
            };
            self.next = 0;
        }
        let slices = self.next..(self.next + LIGHT_SLICES_PER_FRAME).min(LIGHT_Z);
        self.next = slices.end;
        slices
    }
}

/// The far trace traces one texel of each 2 x 2 block a frame, taking turns, and the resolve fills in
/// the rest from the history; `SE_CLOUD_QUARTER=0` traces them all (comparisons). The near pass does the
/// same (tracing them all, it cost 15-23 ms inside Earth's low cumulus at Ultra 3440x1440).
fn quarter_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_CLOUD_QUARTER").map_or(true, |v| v.trim() != "0") && !reference_on())
}

/// The far trace buffer's size for a solids' size: whole 2 x 2 blocks of cloud texels (a quarter), or
/// the cloud buffers' own.
fn trace_size(solid: UVec2, div: u32) -> UVec2 {
    cloud_size(solid, if quarter_on() { 2 * div } else { div })
}

#[derive(Resource)]
struct CloudPipelines {
    /// [no MSAA, MSAA].
    depth_layout: [BindGroupLayoutDescriptor; 2],
    trace_layout: BindGroupLayoutDescriptor,
    resolve_layout: BindGroupLayoutDescriptor,
    composite_layout: [BindGroupLayoutDescriptor; 2],
    /// The weather effects near the camera (rain, snow, lightning on the ground), [no MSAA, MSAA].
    fx_layout: [BindGroupLayoutDescriptor; 2],
    gen_layout: BindGroupLayoutDescriptor,
    down_layout: BindGroupLayoutDescriptor,
    /// The detail cover's generator pass (`detail`).
    patch_layout: BindGroupLayoutDescriptor,
    repeat: Sampler,
    clamp: Sampler,
    depth_down: [CachedRenderPipelineId; 2],
    /// [only veils, with heaps]: the march without the solid types' code for a body whose clouds are all
    /// veils (`HEAPS`).
    trace: [CachedRenderPipelineId; 2],
    resolve: CachedRenderPipelineId,
    composite: [CachedRenderPipelineId; 2],
    fx: [CachedRenderPipelineId; 2],
    cover_gen: CachedRenderPipelineId,
    cover_down: CachedRenderPipelineId,
    /// The same for the weather's pattern.
    pattern_down: CachedRenderPipelineId,
    cover_patch: CachedRenderPipelineId,
    shadow_gen: CachedRenderPipelineId,
    /// The shadow map's mips (`cover_down`).
    shadow_down: CachedRenderPipelineId,
    /// The light volume's slices.
    light_gen: CachedRenderPipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
    cache: Res<PipelineCache>,
) {
    let frag = ShaderStages::FRAGMENT;
    let unfilterable = || texture_2d(TextureSampleType::Float { filterable: false });
    let filterable = || texture_2d(TextureSampleType::Float { filterable: true });
    let view_u = || uniform_buffer::<CloudView>(true);
    let size_u = || uniform_buffer::<CloudSize>(false);
    let depth = |msaa: bool| if msaa { texture_depth_2d_multisampled() } else { texture_depth_2d() };
    let smp = || sampler(SamplerBindingType::Filtering);
    let depth_layout = [false, true].map(|m| {
        BindGroupLayoutDescriptor::new("vol_clouds_depth", &BindGroupLayoutEntries::with_indices(frag, ((0, view_u()), (11, size_u()), (1, depth(m)))))
    });
    let trace_layout = BindGroupLayoutDescriptor::new(
        "vol_clouds_trace",
        &BindGroupLayoutEntries::with_indices(
            frag,
            (
                (0, view_u()),
                (11, size_u()),
                (2, unfilterable()),
                (3, texture_3d(TextureSampleType::Float { filterable: true })),
                (4, texture_3d(TextureSampleType::Float { filterable: true })),
                (5, smp()),
                (9, smp()),
                (12, texture_3d(TextureSampleType::Float { filterable: true })),
                (13, filterable()),
                (14, filterable()),
                (15, texture_cube(TextureSampleType::Float { filterable: true })),
                (16, texture_cube(TextureSampleType::Float { filterable: true })),
                (17, texture_cube(TextureSampleType::Float { filterable: true })),
                (18, storage_buffer_read_only_sized(false, None)),
                (19, texture_2d_array(TextureSampleType::Float { filterable: true })),
                (27, texture_cube(TextureSampleType::Float { filterable: true })),
                (28, texture_3d(TextureSampleType::Float { filterable: true })),
                (29, uniform_buffer::<LightFrame>(false)),
                (30, uniform_buffer::<LightGen>(false)),
                (31, filterable()),
            ),
        ),
    );
    let cube = || texture_cube(TextureSampleType::Float { filterable: true });
    let resolve_layout = BindGroupLayoutDescriptor::new(
        "vol_clouds_resolve",
        &BindGroupLayoutEntries::with_indices(
            frag,
            (
                (0, view_u()),
                (11, size_u()),
                (2, unfilterable()),
                (6, filterable()),
                (7, unfilterable()),
                (8, filterable()),
                (9, smp()),
                (15, cube()),
                (16, cube()),
                (19, texture_2d_array(TextureSampleType::Float { filterable: true })),
            ),
        ),
    );
    let fx_layout = [false, true].map(|m| {
        BindGroupLayoutDescriptor::new(
            "vol_clouds_fx",
            &BindGroupLayoutEntries::with_indices(
                frag,
                (
                    (0, view_u()),
                    (11, size_u()),
                    (1, depth(m)),
                    (3, texture_3d(TextureSampleType::Float { filterable: true })),
                    (5, smp()),
                    (9, smp()),
                    (13, filterable()),
                    (14, filterable()),
                    (15, texture_cube(TextureSampleType::Float { filterable: true })),
                    (16, texture_cube(TextureSampleType::Float { filterable: true })),
                    (17, texture_cube(TextureSampleType::Float { filterable: true })),
                    (18, storage_buffer_read_only_sized(false, None)),
                    (19, texture_2d_array(TextureSampleType::Float { filterable: true })),
                    (27, texture_cube(TextureSampleType::Float { filterable: true })),
                ),
            ),
        )
    });
    let composite_layout = [false, true].map(|m| {
        BindGroupLayoutDescriptor::new(
            "vol_clouds_composite",
            &BindGroupLayoutEntries::with_indices(
                frag,
                (
                    (0, view_u()),
                    (11, size_u()),
                    (1, depth(m)),
                    (2, unfilterable()),
                    (10, filterable()),
                    (26, unfilterable()),
                    (9, smp()),
                    (15, cube()),
                    (16, cube()),
                    (19, texture_2d_array(TextureSampleType::Float { filterable: true })),
                ),
            ),
        )
    });
    let gen_layout = BindGroupLayoutDescriptor::new(
        "vol_clouds_cover_gen",
        &BindGroupLayoutEntries::with_indices(
            frag,
            (
                (20, uniform_buffer::<CoverGen>(false)),
                (21, texture_2d_array(TextureSampleType::Float { filterable: true })),
                (9, smp()),
                (23, uniform_buffer::<Vec4>(false)),
            ),
        ),
    );
    let down_layout = BindGroupLayoutDescriptor::new("vol_clouds_cover_down", &BindGroupLayoutEntries::with_indices(frag, ((24, unfilterable()),)));
    let patch_layout = BindGroupLayoutDescriptor::new(
        "vol_clouds_detail_gen",
        &BindGroupLayoutEntries::with_indices(frag, ((20, uniform_buffer::<CoverGen>(false)), (25, uniform_buffer::<detail::DetailGen>(false)))),
    );
    let sampler_with = |mode: AddressMode| {
        device.create_sampler(&SamplerDescriptor {
            address_mode_u: mode,
            address_mode_v: mode,
            address_mode_w: mode,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..Default::default()
        })
    };
    let shader: Handle<Shader> = assets.load("shaders/volumetric_clouds.wesl");
    // `HEAPS`: the solid cloud types' heaps (`layer_density`); only the march has a variant without them.
    let queue_with = |layout: &BindGroupLayoutDescriptor, entry: &'static str, msaa: Option<bool>, heaps: bool, targets: Vec<Option<ColorTargetState>>| {
        let mut shader_defs: Vec<bevy::shader::ShaderDefVal> = match msaa {
            Some(true) => vec!["MSAA".into()],
            _ => vec!["NO_MSAA".into()],
        };
        if heaps {
            shader_defs.push("HEAPS".into());
        }
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("vol_clouds_{entry}").into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs,
                entry_point: Some(entry.into()),
                targets,
                ..default()
            }),
            ..default()
        })
    };
    let queue = |layout: &BindGroupLayoutDescriptor, entry: &'static str, msaa: Option<bool>, targets: Vec<Option<ColorTargetState>>| queue_with(layout, entry, msaa, true, targets);
    let target = |format: TextureFormat, blend: Option<BlendState>| Some(ColorTargetState { format, blend, write_mask: ColorWrites::ALL });
    // Premultiplied light over the scene, dimmed by the clouds' transmittance.
    let over = BlendState {
        color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::SrcAlpha, operation: BlendOperation::Add },
        alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
    };
    let pipelines = CloudPipelines {
        depth_down: [0, 1].map(|m| queue(&depth_layout[m], "depth_down", Some(m == 1), vec![target(TextureFormat::R32Float, None)])),
        trace: [false, true].map(|heaps| queue_with(&trace_layout, "trace", None, heaps, vec![target(HDR_FORMAT, None), target(TextureFormat::R32Float, None)])),
        shadow_gen: queue(&trace_layout, "shadow_gen", None, vec![target(TextureFormat::R16Float, None)]),
        shadow_down: queue(&down_layout, "cover_down", None, vec![target(TextureFormat::R16Float, None)]),
        light_gen: queue(&trace_layout, "light_gen", None, vec![target(LIGHT_FORMAT, None)]),
        resolve: queue(&resolve_layout, "resolve", None, vec![target(HDR_FORMAT, None), target(TextureFormat::R32Float, None)]),
        composite: [0, 1].map(|m| queue(&composite_layout[m], "composite", Some(m == 1), vec![target(HDR_FORMAT, Some(over))])),
        fx: [0, 1].map(|m| queue(&fx_layout[m], "weather_fx", Some(m == 1), vec![target(HDR_FORMAT, Some(over))])),
        cover_gen: queue(&gen_layout, "cover_gen", None, vec![target(COVER_FORMAT, None), target(PATTERN_FORMAT, None)]),
        cover_down: queue(&down_layout, "cover_down", None, vec![target(COVER_FORMAT, None)]),
        pattern_down: queue(&down_layout, "cover_down", None, vec![target(PATTERN_FORMAT, None)]),
        cover_patch: queue(&patch_layout, "cover_patch", None, vec![target(TextureFormat::R8Unorm, None)]),
        depth_layout,
        trace_layout,
        resolve_layout,
        composite_layout,
        fx_layout,
        gen_layout,
        down_layout,
        patch_layout,
        repeat: sampler_with(AddressMode::Repeat),
        clamp: sampler_with(AddressMode::ClampToEdge),
    };
    commands.insert_resource(pipelines);
}

/// A view's cloud buffers: the scene distance, the trace (light and distance) and the two
/// histories (read one, write the other, swapped each frame).
#[derive(Component)]
struct CloudTextures {
    low_depth: CachedTexture,
    trace: CachedTexture,
    trace_dist: CachedTexture,
    history: [CachedTexture; 2],
    /// The cover's optical depth along each cloud texel's ray (the resolve's, for the composite).
    tau: CachedTexture,
    /// The near pass's, at its size (`CloudView::lod` z, w).
    near: Option<NearTextures>,
}

struct NearTextures {
    low_depth: CachedTexture,
    trace: CachedTexture,
    trace_dist: CachedTexture,
    history: [CachedTexture; 2],
    tau: CachedTexture,
}

/// The history's state across frames: its size, the frame it was last written and which of the
/// two holds it.
#[derive(Resource, Default)]
struct CloudHistory {
    size: UVec2,
    frame: u32,
    parity: usize,
}

/// The near pass's history.
#[derive(Resource, Default)]
struct NearHistory(CloudHistory);

/// The state of the active body's cover map (`CloudShadowAssets::cover`), built a face a frame, then
/// its mips.
#[derive(Resource, Default)]
struct CoverMap {
    /// The `CloudBody::id` it is (being) built for.
    id: u64,
    /// Faces done (6 then the mips: complete at 7).
    built: u32,
}

/// The solids' size of a view: the main pass's reduced size, else the viewport's.
fn solid_size(camera: &bevy::render::camera::ExtractedCamera, low: Option<&MainPassResolutionOverride>) -> Option<UVec2> {
    low.map(|l| l.0).or(camera.physical_viewport_size).filter(|s| s.x > 0 && s.y > 0)
}

/// The cloud buffers' size for a solids' size: whole blocks of `div` x `div` pixels.
fn cloud_size(solid: UVec2, div: u32) -> UVec2 {
    UVec2::new(solid.x.div_ceil(div), solid.y.div_ceil(div)).max(UVec2::ONE)
}

/// Every buffer at the largest size the view can need (the full viewport over the divisor): the
/// solids' reduced size can change within the frame (render_scale.rs), the buffers don't.
fn prepare_textures(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &bevy::render::camera::ExtractedCamera, &CloudView)>,
) {
    for (entity, camera, view) in &views {
        let Some(full) = camera.physical_viewport_size.filter(|_| view.params.x > 0.5) else { continue };
        let div = view.params.w.max(1.0) as u32;
        let tex_at = |cache: &mut TextureCache, label: &'static str, format: TextureFormat, size: UVec2| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let size = cloud_size(full, div);
        let tex = |cache: &mut TextureCache, label: &'static str, format: TextureFormat| tex_at(cache, label, format, size);
        let near = (view.lod.z > 0.0).then(|| {
            let ndiv = view.lod.w.max(1.0) as u32;
            let size = cloud_size(full, ndiv);
            let tsize = trace_size(full, ndiv);
            NearTextures {
                low_depth: tex_at(&mut cache, "vol_clouds_near_low_depth", TextureFormat::R32Float, size),
                trace: tex_at(&mut cache, "vol_clouds_near_trace", HDR_FORMAT, tsize),
                trace_dist: tex_at(&mut cache, "vol_clouds_near_trace_dist", TextureFormat::R32Float, tsize),
                history: [tex_at(&mut cache, "vol_clouds_near_history_a", HDR_FORMAT, size), tex_at(&mut cache, "vol_clouds_near_history_b", HDR_FORMAT, size)],
                tau: tex_at(&mut cache, "vol_clouds_near_tau", TextureFormat::R32Float, size),
            }
        });
        let tsize = trace_size(full, div);
        let textures = CloudTextures {
            low_depth: tex(&mut cache, "vol_clouds_low_depth", TextureFormat::R32Float),
            trace: tex_at(&mut cache, "vol_clouds_trace", HDR_FORMAT, tsize),
            trace_dist: tex_at(&mut cache, "vol_clouds_trace_dist", TextureFormat::R32Float, tsize),
            history: [tex(&mut cache, "vol_clouds_history_a", HDR_FORMAT), tex(&mut cache, "vol_clouds_history_b", HDR_FORMAT)],
            tau: tex(&mut cache, "vol_clouds_tau", TextureFormat::R32Float),
            near,
        };
        commands.entity(entity).insert(textures);
    }
}

fn uniform_buffer_of<T: ShaderType + encase::internal::WriteInto>(device: &RenderDevice, label: &'static str, value: &T) -> Option<Buffer> {
    let mut bytes = encase::UniformBuffer::new(Vec::<u8>::new());
    bytes.write(value).ok()?;
    Some(device.create_buffer_with_data(&BufferInitDescriptor { label: Some(label), contents: bytes.as_ref(), usage: BufferUsages::UNIFORM }))
}

fn color_attachment(v: &TextureView) -> Option<RenderPassColorAttachment<'_>> {
    Some(RenderPassColorAttachment { view: v, depth_slice: None, resolve_target: None, ops: Operations::default() })
}

fn fullscreen_pass(encoder: &mut CommandEncoder, label: &'static str, colors: &[Option<RenderPassColorAttachment>], pipeline: &RenderPipeline, group: &BindGroup, offsets: &[u32], area: UVec2) {
    let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: colors,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    p.set_viewport(0.0, 0.0, area.x as f32, area.y as f32, 0.0, 1.0);
    p.set_pipeline(pipeline);
    p.set_bind_group(0, group, offsets);
    p.draw(0..3, 0..1);
}

/// One step of the cover map (a face, or all the mips once the faces are done); true once complete.
#[allow(clippy::too_many_arguments)]
fn build_cover(
    map: &mut CoverMap,
    body: &CloudBody,
    pipelines: &CloudPipelines,
    cache: &PipelineCache,
    device: &RenderDevice,
    images: &RenderAssets<GpuImage>,
    fallback: &FallbackImage,
    (cover, pattern): (&GpuImage, &GpuImage),
    encoder: &mut CommandEncoder,
) -> bool {
    if map.id != body.id {
        map.id = body.id;
        map.built = 0;
    }
    if map.built >= 7 {
        return true;
    }
    let mapped = match &body.mapped {
        Some(h) => match images.get(h) {
            Some(g) => &g.texture_view,
            None => return false,
        },
        None if body.mapped_pending => return false,
        None => &fallback.d2_array.texture_view,
    };
    let (Some(gen_pipe), Some(down), Some(pattern_down)) =
        (cache.get_render_pipeline(pipelines.cover_gen), cache.get_render_pipeline(pipelines.cover_down), cache.get_render_pipeline(pipelines.pattern_down))
    else {
        return false;
    };
    let face_view_of = |texture: &Texture, face: u32, mip: u32| {
        texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2),
            base_mip_level: mip,
            mip_level_count: Some(1),
            base_array_layer: face,
            array_layer_count: Some(1),
            ..Default::default()
        })
    };
    if map.built < 6 {
        let face = map.built;
        let (Some(gen_buf), Some(face_buf)) = (
            uniform_buffer_of(device, "vol_clouds_cover_gen", &body.cover),
            uniform_buffer_of(device, "vol_clouds_cover_face", &Vec4::new(face as f32, COVER_SIZE as f32, 0.0, 0.0)),
        ) else {
            return false;
        };
        let group = device.create_bind_group(
            "vol_clouds_cover_gen",
            &cache.get_bind_group_layout(&pipelines.gen_layout),
            &BindGroupEntries::with_indices(((20, gen_buf.as_entire_binding()), (21, mapped), (9, &pipelines.clamp), (23, face_buf.as_entire_binding()))),
        );
        let (dst, pdst) = (face_view_of(&cover.texture, face, 0), face_view_of(&pattern.texture, face, 0));
        fullscreen_pass(encoder, "vol_clouds_cover_gen", &[color_attachment(&dst), color_attachment(&pdst)], gen_pipe, &group, &[], UVec2::splat(COVER_SIZE));
        map.built += 1;
        return false;
    }
    let layout = cache.get_bind_group_layout(&pipelines.down_layout);
    for (texture, pipe) in [(&cover.texture, down), (&pattern.texture, pattern_down)] {
        for mip in 1..COVER_MIPS {
            for face in 0..6 {
                let src = face_view_of(texture, face, mip - 1);
                let dst = face_view_of(texture, face, mip);
                let group = device.create_bind_group("vol_clouds_cover_down", &layout, &BindGroupEntries::with_indices(((24, &src),)));
                fullscreen_pass(encoder, "vol_clouds_cover_down", &[color_attachment(&dst)], pipe, &group, &[], UVec2::splat((COVER_SIZE >> mip).max(1)));
            }
        }
    }
    map.built = 7;
    info!("volumetric clouds: cover map ready");
    true
}

#[allow(clippy::too_many_arguments)]
fn clouds_pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthStencilTexture,
        &bevy::render::camera::ExtractedCamera,
        &CloudView,
        &DynamicUniformIndex<CloudView>,
        &CloudTextures,
        &Msaa,
        Option<&MainPassResolutionOverride>,
    )>,
    pipelines: Res<CloudPipelines>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<CloudView>>,
    (noise, body, fallback, assets): (Option<Res<CloudNoise>>, Res<CloudBody>, Res<FallbackImage>, Option<Res<CloudShadowAssets>>),
    images: Res<RenderAssets<GpuImage>>,
    ready: Res<CloudsReady>,
    frame: Res<bevy::diagnostic::FrameCount>,
    (mut history, mut near_history, mut cover, mut light): (ResMut<CloudHistory>, ResMut<NearHistory>, ResMut<CoverMap>, ResMut<LightVolume>),
    (sim_pipes, sim_frame, mut sim_gpu): (Res<weather_sim::SimPipelines>, Res<weather_sim::SimFrame>, ResMut<weather_sim::SimGpu>),
    (detail_cover, mut detail_gpu): (Res<detail::DetailCover>, ResMut<detail::DetailGpu>),
    mut ctx: RenderContext,
) {
    let (target, depth, camera, cv, index, tex, msaa, low) = view.into_inner();
    let image = |h: fn(&CloudShadowAssets) -> &Handle<Image>| assets.as_ref().and_then(|a| images.get(h(a)));
    let (cover_image, pattern, weather, derived) = (image(|a| &a.cover), image(|a| &a.pattern), image(|a| &a.weather), image(|a| &a.derived));
    let (Some(cover_image), Some(pattern), Some(weather), Some(derived)) = (cover_image, pattern, weather, derived) else {
        ready.0.store(0, Ordering::Relaxed);
        return;
    };
    if cv.params.x < 0.5 || target.main_texture_format() != HDR_FORMAT || body.body.is_none() {
        ready.0.store(0, Ordering::Relaxed);
        return;
    }
    let device = ctx.render_device().clone();
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    // Timed as one span (`SE_PERF`, perf.rs: render/volumetric_clouds).
    let time_span = diagnostics.time_span(ctx.command_encoder(), "volumetric_clouds");
    let encoder = ctx.command_encoder();
    encoder.push_debug_group("volumetric_clouds");
    let complete = build_cover(&mut cover, &body, &pipelines, &cache, &device, &images, &fallback, (cover_image, pattern), encoder);
    let detailed = detail::build(&mut detail_gpu, &detail_cover, &body, &pipelines, &cache, &device, encoder);
    let detail_view = detail_gpu.view().unwrap_or(&fallback.d2_array.texture_view);
    let map = assets.as_ref().and_then(|a| images.get(&a.map));
    // The weather cube for this frame, then the clouds through it. (Their parts are timed on their own
    // too: render/volumetric_clouds/sim, shadow, trace, resolve, near, fx.)
    let span = diagnostics.time_span(encoder, "sim");
    let simulated = complete && weather_sim::run_sim(encoder, &device, &sim_pipes, &cache, &sim_frame, &mut sim_gpu, derived, weather);
    span.end(encoder);
    let drawn = match sim_gpu.fx() {
        Some(fx) if simulated && detailed => draw_clouds(encoder, diagnostics, &device, &pipelines, &cache, &uniforms, noise.as_deref(), &images, &fallback, (&cover_image.texture_view, map, &weather.texture_view, fx, detail_view, &pattern.texture_view), &body, (target, depth, camera, cv, index, tex, msaa, low), frame.0, (&mut history, &mut near_history.0, &mut light)),
        _ => false,
    };
    encoder.pop_debug_group();
    time_span.end(encoder);
    ready.0.store(if drawn { body.body.map_or(0, |e| e.to_bits() + 1) } else { 0 }, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw_clouds(
    encoder: &mut CommandEncoder,
    diagnostics: Option<&bevy::render::diagnostic::DiagnosticsRecorder>,
    device: &RenderDevice,
    pipelines: &CloudPipelines,
    cache: &PipelineCache,
    uniforms: &ComponentUniforms<CloudView>,
    noise: Option<&CloudNoise>,
    images: &RenderAssets<GpuImage>,
    fallback: &FallbackImage,
    (cube, shadow_map, weather, (fx_cube, flashes), detail_cover, pattern): (&TextureView, Option<&GpuImage>, &TextureView, (&TextureView, &Buffer), &TextureView, &TextureView),
    body: &CloudBody,
    (target, depth, camera, cv, index, tex, msaa, low): (
        &ViewTarget,
        &ViewDepthStencilTexture,
        &bevy::render::camera::ExtractedCamera,
        &CloudView,
        &DynamicUniformIndex<CloudView>,
        &CloudTextures,
        &Msaa,
        Option<&MainPassResolutionOverride>,
    ),
    frame: u32,
    (history, near_history, light): (&mut CloudHistory, &mut CloudHistory, &mut LightVolume),
) -> bool {
    let m = (msaa.samples() > 1) as usize;
    let (Some(depth_down), Some(trace), Some(resolve), Some(composite), Some(fx), Some(binding)) = (
        cache.get_render_pipeline(pipelines.depth_down[m]),
        cache.get_render_pipeline(pipelines.trace[(0..MAX_LAYERS).any(|i| cv.layer_a[i].z > 0.0 && cv.layer_a[i].w > 0.0) as usize]),
        cache.get_render_pipeline(pipelines.resolve),
        cache.get_render_pipeline(pipelines.composite[m]),
        cache.get_render_pipeline(pipelines.fx[m]),
        uniforms.binding(),
    ) else {
        return false;
    };
    let Some((shape, detail)) = noise.and_then(|n| Some((images.get(&n.shape)?, images.get(&n.detail)?))) else { return false };
    let atm: [&TextureView; 3] = match &body.atm {
        Some([s, t, e]) => match (images.get(s), images.get(t), images.get(e)) {
            (Some(s), Some(t), Some(e)) => [&s.texture_view, &t.texture_view, &e.texture_view],
            _ => return false,
        },
        None => [&fallback.d3.texture_view, &fallback.d2.texture_view, &fallback.d2.texture_view],
    };
    let Some(solid) = solid_size(camera, low) else { return false };
    let div = cv.params.w.max(1.0) as u32;
    let size = cloud_size(solid, div);
    let tsize = trace_size(solid, div);
    if size.x > tex.history[0].texture.width() || size.y > tex.history[0].texture.height() || tsize.x > tex.trace.texture.width() || tsize.y > tex.trace.texture.height() {
        return false;
    }
    // The history is kept only if it was written last frame at this size.
    let reset = history.size != size || history.frame.wrapping_add(1) != frame;
    let (read, write) = (history.parity, 1 - history.parity);
    history.size = size;
    history.frame = frame;
    history.parity = write;

    let su = CloudSize {
        size: Vec4::new(solid.x as f32, solid.y as f32, size.x as f32, size.y as f32),
        params: Vec4::new(
            div as f32,
            if reference_on() { 2.0 } else { reset as u32 as f32 },
            (frame % 4096) as f32,
            if cv.lod.z > 0.0 && tex.near.is_some() { 1.0 } else { 0.0 },
        ),
        trace: Vec4::new(tsize.x as f32, tsize.y as f32, (frame % 4) as f32, quarter_on() as u32 as f32),
    };
    let Some(size_buf) = uniform_buffer_of(device, "vol_clouds_size", &su) else { return false };
    let depth_view = depth.texture().create_view(&TextureViewDescriptor { aspect: TextureAspect::DepthOnly, ..Default::default() });
    let layout = |l: &BindGroupLayoutDescriptor| cache.get_bind_group_layout(l);
    // The light volume: this frame's slices of the back one (read by nothing yet), then the front one for
    // the march (swapped in when the back one is complete).
    let slices = light.advance(device, cv, body.id);
    let back = 1 - light.front;
    let gen_frame = light.frames[back];
    let complete = slices.end == LIGHT_Z && !slices.is_empty();
    let Some(front_buf) = uniform_buffer_of(device, "vol_clouds_light_frame", &light.frames[light.front]) else { return false };
    let Some(no_gen) = uniform_buffer_of(device, "vol_clouds_light_gen", &LightGen::default()) else { return false };
    let depth_group = device.create_bind_group(
        "vol_clouds_depth",
        &layout(&pipelines.depth_layout[m]),
        &BindGroupEntries::with_indices(((0, binding.clone()), (11, size_buf.as_entire_binding()), (1, &depth_view))),
    );
    let ring_view = body.ring.as_ref().and_then(|h| images.get(h)).map_or(&fallback.d2.texture_view, |i| &i.texture_view);
    let trace_group_with = |size_buf: &Buffer, low_depth: &TextureView, light_view: &TextureView, light_buf: &Buffer, gen_buf: &Buffer| device.create_bind_group(
        "vol_clouds_trace",
        &layout(&pipelines.trace_layout),
        &BindGroupEntries::with_indices((
            (0, binding.clone()),
            (11, size_buf.as_entire_binding()),
            (2, low_depth),
            (3, &shape.texture_view),
            (4, &detail.texture_view),
            (5, &pipelines.repeat),
            (9, &pipelines.clamp),
            (12, atm[0]),
            (13, atm[1]),
            (14, atm[2]),
            (15, cube),
            (16, weather),
            (17, fx_cube),
            (18, flashes.as_entire_binding()),
            (19, detail_cover),
            (27, pattern),
            (28, light_view),
            (29, light_buf.as_entire_binding()),
            (30, gen_buf.as_entire_binding()),
            (31, ring_view),
        )),
    );
    if let Some(light_gen) = cache.get_render_pipeline(pipelines.light_gen).filter(|_| !slices.is_empty()) {
        let span = diagnostics.time_span(encoder, "light");
        for s in slices.clone() {
            let slice_gen = LightGen { frame: gen_frame, slice: Vec4::new(s as f32, LIGHT_XY as f32, LIGHT_Z as f32, 0.0) };
            let Some(gen_buf) = uniform_buffer_of(device, "vol_clouds_light_gen", &slice_gen) else { continue };
            let group = trace_group_with(&size_buf, &tex.low_depth.default_view, &light.tex[light.front].1, &front_buf, &gen_buf);
            let slice = RenderPassColorAttachment { view: &light.tex[back].1, depth_slice: Some(s), resolve_target: None, ops: Operations::default() };
            fullscreen_pass(encoder, "vol_clouds_light", &[Some(slice)], light_gen, &group, &[index.index()], UVec2::splat(LIGHT_XY));
        }
        span.end(encoder);
        if complete {
            light.front = back;
        }
    } else if !slices.is_empty() {
        // (The pipeline not ready yet: these slices again next frame.)
        light.next = slices.start;
    }
    let Some(front_buf) = uniform_buffer_of(device, "vol_clouds_light_frame", &light.frames[light.front]) else { return false };
    let light_view = &light.tex[light.front].1;
    let trace_group_for = |size_buf: &Buffer, low_depth: &TextureView| trace_group_with(size_buf, low_depth, light_view, &front_buf, &no_gen);
    let trace_group = trace_group_for(&size_buf, &tex.low_depth.default_view);
    let fx_group = device.create_bind_group(
        "vol_clouds_fx",
        &layout(&pipelines.fx_layout[m]),
        &BindGroupEntries::with_indices((
            (0, binding.clone()),
            (11, size_buf.as_entire_binding()),
            (1, &depth_view),
            (3, &shape.texture_view),
            (5, &pipelines.repeat),
            (9, &pipelines.clamp),
            (13, atm[1]),
            (14, atm[2]),
            (15, cube),
            (16, weather),
            (17, fx_cube),
            (18, flashes.as_entire_binding()),
            (19, detail_cover),
            (27, pattern),
        )),
    );
    let resolve_group = device.create_bind_group(
        "vol_clouds_resolve",
        &layout(&pipelines.resolve_layout),
        &BindGroupEntries::with_indices((
            (0, binding.clone()),
            (11, size_buf.as_entire_binding()),
            (2, &tex.low_depth.default_view),
            (6, &tex.trace.default_view),
            (7, &tex.trace_dist.default_view),
            (8, &tex.history[read].default_view),
            (9, &pipelines.clamp),
            (15, cube),
            (16, weather),
            (19, detail_cover),
        )),
    );
    let composite_group = device.create_bind_group(
        "vol_clouds_composite",
        &layout(&pipelines.composite_layout[m]),
        &BindGroupEntries::with_indices((
            (0, binding.clone()),
            (11, size_buf.as_entire_binding()),
            (1, &depth_view),
            (2, &tex.low_depth.default_view),
            (10, &tex.history[write].default_view),
            (26, &tex.tau.default_view),
            (9, &pipelines.clamp),
            (15, cube),
            (16, weather),
            (19, detail_cover),
        )),
    );
    let offsets = [index.index()];
    // The ground's shadow map, through the same clouds.
    let span = diagnostics.time_span(encoder, "shadow");
    if let (Some(map), Some(shadow_gen), Some(down)) =
        (shadow_map, cache.get_render_pipeline(pipelines.shadow_gen), cache.get_render_pipeline(pipelines.shadow_down))
    {
        let mip = |level: u32| map.texture.create_view(&TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), ..Default::default() });
        fullscreen_pass(encoder, "vol_clouds_shadow", &[color_attachment(&mip(0))], shadow_gen, &trace_group, &offsets, UVec2::splat(SHADOW_SIZE));
        let layout = cache.get_bind_group_layout(&pipelines.down_layout);
        for level in 1..SHADOW_MIPS {
            let (src, dst) = (mip(level - 1), mip(level));
            let group = device.create_bind_group("vol_clouds_shadow_down", &layout, &BindGroupEntries::with_indices(((24, &src),)));
            fullscreen_pass(encoder, "vol_clouds_shadow_down", &[color_attachment(&dst)], down, &group, &[], UVec2::splat(SHADOW_SIZE >> level));
        }
    }
    span.end(encoder);
    let span = diagnostics.time_span(encoder, "trace");
    fullscreen_pass(encoder, "vol_clouds_depth", &[color_attachment(&tex.low_depth.default_view)], depth_down, &depth_group, &offsets, size);
    fullscreen_pass(
        encoder,
        "vol_clouds_trace",
        &[color_attachment(&tex.trace.default_view), color_attachment(&tex.trace_dist.default_view)],
        trace,
        &trace_group,
        &offsets,
        tsize,
    );
    span.end(encoder);
    let span = diagnostics.time_span(encoder, "resolve");
    fullscreen_pass(encoder, "vol_clouds_resolve", &[color_attachment(&tex.history[write].default_view), color_attachment(&tex.tau.default_view)], resolve, &resolve_group, &offsets, size);
    // Over the scene: its colour is loaded (the default would clear it).
    let scene = RenderPassColorAttachment {
        view: target.main_texture_view(),
        depth_slice: None,
        resolve_target: None,
        ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
    };
    fullscreen_pass(encoder, "vol_clouds_composite", &[Some(scene)], composite, &composite_group, &offsets, solid);
    span.end(encoder);
    // The clouds within the near pass's range at its finer size, over the far ones: the same passes on
    // the near part of the rays (`CloudSize::params` w 2), with their own history.
    let span = diagnostics.time_span(encoder, "near");
    if let Some(near) = tex.near.as_ref().filter(|_| cv.lod.z > 0.0) {
        let ndiv = cv.lod.w.max(1.0) as u32;
        let nsize = cloud_size(solid, ndiv);
        let ntsize = trace_size(solid, ndiv);
        if nsize.x <= near.history[0].texture.width() && nsize.y <= near.history[0].texture.height() && ntsize.x <= near.trace.texture.width() && ntsize.y <= near.trace.texture.height() {
            let nreset = near_history.size != nsize || near_history.frame.wrapping_add(1) != frame;
            let (nread, nwrite) = (near_history.parity, 1 - near_history.parity);
            *near_history = CloudHistory { size: nsize, frame, parity: nwrite };
            let nsu = CloudSize {
                size: Vec4::new(solid.x as f32, solid.y as f32, nsize.x as f32, nsize.y as f32),
                params: Vec4::new(ndiv as f32, nreset as u32 as f32, (frame % 4096) as f32, 2.0),
                trace: Vec4::new(ntsize.x as f32, ntsize.y as f32, (frame % 4) as f32, quarter_on() as u32 as f32),
            };
            if let Some(nbuf) = uniform_buffer_of(device, "vol_clouds_near_size", &nsu) {
                let ndepth_group = device.create_bind_group(
                    "vol_clouds_depth",
                    &layout(&pipelines.depth_layout[m]),
                    &BindGroupEntries::with_indices(((0, binding.clone()), (11, nbuf.as_entire_binding()), (1, &depth_view))),
                );
                let ntrace_group = trace_group_for(&nbuf, &near.low_depth.default_view);
                let nresolve_group = device.create_bind_group(
                    "vol_clouds_resolve",
                    &layout(&pipelines.resolve_layout),
                    &BindGroupEntries::with_indices((
                        (0, binding.clone()),
                        (11, nbuf.as_entire_binding()),
                        (2, &near.low_depth.default_view),
                        (6, &near.trace.default_view),
                        (7, &near.trace_dist.default_view),
                        (8, &near.history[nread].default_view),
                        (9, &pipelines.clamp),
                        (15, cube),
                        (16, weather),
                        (19, detail_cover),
                    )),
                );
                let ncomposite_group = device.create_bind_group(
                    "vol_clouds_composite",
                    &layout(&pipelines.composite_layout[m]),
                    &BindGroupEntries::with_indices((
                        (0, binding.clone()),
                        (11, nbuf.as_entire_binding()),
                        (1, &depth_view),
                        (2, &near.low_depth.default_view),
                        (10, &near.history[nwrite].default_view),
                        (26, &near.tau.default_view),
                        (9, &pipelines.clamp),
                        (15, cube),
                        (16, weather),
                        (19, detail_cover),
                    )),
                );
                fullscreen_pass(encoder, "vol_clouds_near_depth", &[color_attachment(&near.low_depth.default_view)], depth_down, &ndepth_group, &offsets, nsize);
                fullscreen_pass(
                    encoder,
                    "vol_clouds_near_trace",
                    &[color_attachment(&near.trace.default_view), color_attachment(&near.trace_dist.default_view)],
                    trace,
                    &ntrace_group,
                    &offsets,
                    ntsize,
                );
                fullscreen_pass(
                    encoder,
                    "vol_clouds_near_resolve",
                    &[color_attachment(&near.history[nwrite].default_view), color_attachment(&near.tau.default_view)],
                    resolve,
                    &nresolve_group,
                    &offsets,
                    nsize,
                );
                let scene = RenderPassColorAttachment {
                    view: target.main_texture_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
                };
                fullscreen_pass(encoder, "vol_clouds_near_composite", &[Some(scene)], composite, &ncomposite_group, &offsets, solid);
            }
        }
    }
    span.end(encoder);
    // The effects near the camera, over the clouds.
    let scene = RenderPassColorAttachment {
        view: target.main_texture_view(),
        depth_slice: None,
        resolve_target: None,
        ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
    };
    let span = diagnostics.time_span(encoder, "fx");
    fullscreen_pass(encoder, "vol_clouds_fx", &[Some(scene)], fx, &fx_group, &offsets, solid);
    span.end(encoder);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reprojection_follows_the_body() {
        let proj = DMat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.5);
        let shape = DVec3::new(1.0, 0.99, 1.0);
        let sphere = |orient: DQuat| DMat3::from_diagonal(DVec3::ONE / shape) * DMat3::from_quat(orient.inverse());
        // The camera rides on the turning body: the same place and view in the body's frame.
        let (q0, q1) = (DQuat::from_rotation_y(0.3), DQuat::from_rotation_y(0.3 + 0.01));
        let cam_body = DVec3::new(0.0, 6.4e6, 2.0e5);
        let view_body = DQuat::from_rotation_x(-0.2);
        let prev = PrevFrame {
            body: Entity::PLACEHOLDER,
            clip_from_view: proj,
            view_rot: q0 * view_body,
            sphere_from_world: sphere(q0),
            cam_s: cam_body / shape,
        };
        let m = reprojection(&prev, sphere(q1), cam_body / shape);
        // A cloud fixed on the body, 30 km ahead: where this frame sees it (camera-relative world)...
        let cloud_body = cam_body + view_body * DVec3::new(3000.0, -2000.0, -30_000.0);
        let x_now = q1 * (cloud_body - cam_body);
        // ...lands where the previous frame drew it.
        let clip_prev = proj * DMat4::from_quat((q0 * view_body).inverse()) * (q0 * (cloud_body - cam_body)).extend(1.0);
        let got = m * x_now.extend(1.0);
        let (a, b) = (got.truncate() / got.w, clip_prev.truncate() / clip_prev.w);
        assert!((a - b).abs().max_element() < 1e-9, "{a} vs {b}");
        // The same frame twice: the plain projection of the point.
        let same = reprojection(&PrevFrame { view_rot: q1 * view_body, sphere_from_world: sphere(q1), ..prev }, sphere(q1), cam_body / shape);
        let c = same * x_now.extend(1.0);
        let d = proj * DMat4::from_quat((q1 * view_body).inverse()) * x_now.extend(1.0);
        assert!((c / c.w - d / d.w).abs().max_element() < 1e-9);
    }

    #[test]
    fn cloud_buffers_cover_whole_blocks() {
        assert_eq!(cloud_size(UVec2::new(3440, 1440), 2), UVec2::new(1720, 720));
        assert_eq!(cloud_size(UVec2::new(2193, 918), 4), UVec2::new(549, 230));
        assert_eq!(cloud_size(UVec2::new(1, 1), 4), UVec2::ONE);
    }

    #[test]
    fn layers_become_volumes_by_height() {
        let layer_over = |height: f64, bump_m: f32, ground: f32| {
            let mut c = CloudLayer::test_layer();
            c.height = height;
            c.bump_m = bump_m;
            (layer_volume(&c, true, ground), layer_volume(&c, false, ground))
        };
        let layer = |height: f64, bump_m: f32| layer_over(height, bump_m, 0.0);
        // Earth's single layer (its map's shell 9 km up): the lower atmosphere's clouds, 1.1 to 9 km.
        let ((b, t, cu), _) = layer(9000.0, 0.0);
        assert!(cu == 1.0 && (b - 1125.0).abs() < 1.0 && (t - 9000.0).abs() < 1.0);
        // A low upper layer: cumuliform, from just under the shell up by its relief (at most 1.6 times
        // its height).
        let (_, (b, t, cu)) = layer(2000.0, 2500.0);
        assert!(cu > 0.99 && (t - b - 3200.0).abs() < 1.0 && b < 2000.0 && b >= 150.0);
        // A high one: stratiform and at least the thin minimum.
        let (_, (b, t, cu)) = layer(12_000.0, 0.0);
        assert!(cu == 0.0 && (t - b - 600.0).abs() < 1.0 && b > 11_000.0);
        // Never below the ground.
        assert!(layer(100.0, 5000.0).1.0 >= 150.0 && layer(100.0, 5000.0).0.0 >= 400.0);
        // SE's generated layer over a dry world's 20 km relief (the user's planet: 15.7 km up, the
        // ground's middle at 10 km): cumuliform from just over the ground's middle.
        let ((b, t, cu), _) = layer_over(15_700.0, 2600.0, 10_000.0);
        assert!(cu == 1.0 && (b - 10_712.5).abs() < 1.0 && (t - 17_000.0).abs() < 1.0);
        // A layer high in a dense atmosphere: still the lower atmosphere's heaps, at most 9 km deep.
        let ((b, t, cu), _) = layer(30_000.0, 0.0);
        assert!(cu == 1.0 && b == 2000.0 && t == 11_000.0);
        // A layer 2 km over a high ground's middle: cumuliform, as over Earth's sea.
        assert!(layer_over(12_000.0, 2500.0, 10_000.0).1 .2 > 0.99);
        // An opaque map (Venus's deck) stays a sheet round its shell.
        let mut c = CloudLayer::test_layer();
        c.height = 60_000.0;
        c.opaque_map = true;
        let (b, _, cu) = layer_volume(&c, true, 0.0);
        assert!(cu == 0.0 && b > 59_000.0);
    }
}
