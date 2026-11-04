//! Planet surfaces: a cube-sphere quadtree per body, textured with SpaceEngine's
//! tiled cube maps (colour, water mask, night lights, clouds) and displaced by
//! its 16-bit bump tiles.
//!
//! Each quadtree node is its own entity with a mesh built on a worker thread in
//! double precision relative to the node centre (so f32 vertices keep millimetre
//! precision anywhere on the planet) and a material referencing the deepest
//! available texture tile; nodes deeper than the data reuse an ancestor tile via
//! a UV sub-rectangle.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
// A fixed fast hasher for the per-node maps, looked up thousands of times a frame. Nothing relies
// on their order (std's was random per run).
use bevy::platform::collections::{HashMap as NodeMap, HashSet};
use bevy::image::{ImageSampler, ImageSamplerDescriptor, ImageAddressMode, ImageFilterMode};
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::query::Allow;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat, encase,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::catalog::{BodyRecord, ObjectRef};
use se_core::planet_tiles::{CubeFace, SurfaceParams, TileKey, TileLayer};
use se_core::proc_surface::ProcSurface;
use se_core::upos::UPos;
use se_core::vfs::Vfs;

use crate::atmosphere::{AtmosphereGpu, AtmosphereParamsUniform};
use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::surface_tex::{BiomeUniform, SOIL_WRAP_M, SurfaceTextures};
use crate::system::{BodyLighting, MAX_LIGHTS, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, UOrientation, UPosition, UniverseSet};

/// The star's light on planetary surfaces, in the atmosphere tables' units (unit irradiance):
/// SE's `hdrPlanetSurfaceBright` (main-def.cfg).
pub const ATM_SUN_SCALE: f32 = 5.0;
/// The star's light the atmospheres scatter (sky, haze), times each atmosphere's `Bright`: SE's
/// `hdrAtmoScatterBright` (main-def.cfg).
pub const ATM_SCATTER_SCALE: f32 = 1.0;

/// Grid resolution of a node patch (vertices per side).
pub(crate) const GRID: usize = 33;
/// Deepest geometric subdivision.
pub(crate) const MAX_GEOM_LEVEL: u8 = 20;
/// Split when the camera is closer than this many node edge lengths.
const SPLIT_DISTANCE: f64 = 2.2;
const MAX_IN_FLIGHT: usize = 24;
/// Generated height grids kept per planet before unused ones are dropped (145^2 floats, 84 KB each).
const GEN_TILES_MAX: usize = 1500;
/// A grid with no node of its own is kept this long after it last served a request.
const GEN_TILE_KEEP_FRAMES: u64 = 600;

/// The fixed part of a patch's material. What a relight changes is in the node's [`PatchLight`];
/// the shader reads those components (marked "the slot's") from there.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct PlanetUniform {
    /// xyz: planet centre in node-local (body-frame, meters) coordinates; w: radius (m).
    pub planet: Vec4,
    /// Texture sub-rectangles: xy offset, z size, w enabled (1/0).
    pub rect_color: Vec4,
    pub rect_lights: Vec4,
    pub rect_water: Vec4,
    pub rect_clouds: Vec4,
    /// rgb fallback albedo; w ambient (the slot's).
    pub base_color: Vec4,
    /// x glow brightness (self-emission included; the slot's), y water specular, z cloud opacity,
    /// w water roughness.
    pub params: Vec4,
    /// rgb SE's WaterSurfColor ([`Terrain::water_color`] x ModulateColor, linear); w: 1 = body has
    /// water.
    pub ocean_sea: Vec4,
    /// SE's WaterFogColor ([`Terrain::water_fog`]): the water's absorption, rgb and opacity, per km.
    pub water_fog: Vec4,
    /// x: atmosphere enabled (the slot's), y: sun irradiance scale for the tables, z: the scattered
    /// light's scale relative to y ([`scatter_rel`]), w: the surface's `DayAmbient`.
    pub atm_flags: Vec4,
    /// Procedural palette (lowland, midland, highland, polar), linear rgb.
    pub proc_pal0: Vec4,
    pub proc_pal1: Vec4,
    pub proc_pal2: Vec4,
    pub proc_pal3: Vec4,
    /// x: surface kind id, y: relief (m), z: temperature (K), w: has life.
    pub proc_params: Vec4,
    /// x: radius (m), y: seed (fraction), z: procedural colour enabled, w: SE's volatiles class
    /// (4 Oceanic, 5 Superoceanic: 3D water at every distance, see [`water_reach_m`]).
    pub proc_params2: Vec4,
    pub atm: AtmosphereParamsUniform,
    /// Own rings: x inner, y outer radius (planet radii), z opacity, w 0 none / 0.5 untextured / 1 textured.
    pub ring: Vec4,
    /// xyz: ring plane normal (render orientation); w: SE's rings winter strength, `RingsWinter` x
    /// the rings' mean radius over the planet's (FUN_1404ae4a0), 0 without rings or the key.
    pub ring_normal: Vec4,
    /// Node position on its cube face for metre-scale soil textures: xy the node corner
    /// (m, modulo `SOIL_WRAP_M`), z the node edge length (m); w the split distance in node edges,
    /// for the geomorph (0: none; the slot's).
    pub soil_tile: Vec4,
    /// xyz: the node's +u axis on its cube face at its centre (body frame), for the waves;
    /// w: 1 on the water surface material (`water_fragment`, both sides), 0 on the ground.
    pub water: Vec4,
    /// [`Terrain::surf_modulate`].
    pub modulate: Vec4,
    /// [`Terrain::surf_hapke`].
    pub hapke: Vec4,
    /// SE's thermal emission: x the base temperature (thousand K; SE's surfTemperature), y the lava
    /// coverage, z the sun heating at the subsolar point (thousand K ^ 4), w 1 for a giant.
    pub thermal: Vec4,
    /// x 1 / (pi (R* / d)^2), the star's surface radiance per its irradiance here; y the star's
    /// temperature (K); z SE's ThermalEmissionShift (thousand K); w the self-emission scale
    /// ([`SelfEmission::scale`]; the slot's).
    pub thermal2: Vec4,
    /// SE's GlowColor: rgb the catalog's tint of the glow map, w its mode (1 "Night": off where
    /// light reaches the ground, 0 "Permanent").
    pub glow: Vec4,
    /// SE's ice glint (planet.glsl AICE): x SpecBrightIce, y RoughnessIce, z 1 when the colour map's
    /// alpha is the ice mask (`DiffMapAlpha "Ice"`), else 0 (no glint); w SE's WaterSurfColor.a, the
    /// water display level ([`WaterDisplay`]; the slot's).
    pub ice: Vec4,
    /// The node's record in the material's light page (`PlanetMaterial::light_page`).
    pub slot: u32,
}

/// The lighting of a node's patches (ground, sea surface, cloud layers): what a relight changes,
/// kept per node in its body's light pages, which the shaders read at the patch's `slot`. Rewriting
/// every patch's material instead made Bevy rebuild them all in one frame, a 50 ms hitch each time
/// the sun moved (every ~13 s on fast-orbiting worlds).
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct PatchLight {
    /// xyz: unit direction to the star (render frame); w: intensity.
    pub sun_dir: Vec4,
    /// rgb star colour.
    pub sun_color: Vec4,
    /// Each light's eclipse casters (`EclipseLight` in eclipse.wesl, from [`crate::eclipse::EclipseCasters`]).
    pub eclipse: [crate::eclipse::EclipseLight; MAX_LIGHTS],
    /// The further stars ([`BodyLighting::extra_uniforms`]): xyz direction, w 1 when used.
    pub light_dir: [Vec4; MAX_LIGHTS - 1],
    /// Their rgb light, intensity included.
    pub light_color: [Vec4; MAX_LIGHTS - 1],
    /// SE's GlowColorAtmo ([`glow_color_atmo`]), rgb in the units of `light_color`; 0 without a
    /// glowing atmosphere.
    pub glow_atmo: Vec4,
    /// The relit components of [`PlanetUniform`]: x `base_color.w`, y `params.x`, z `thermal2.w`,
    /// w `ice.w`.
    pub levels: Vec4,
    /// x `atm_flags.x`, y `soil_tile.w`.
    pub flags: Vec4,
}

/// Records per light page.
const LIGHT_PAGE_SLOTS: u32 = 64;

/// A body's [`PatchLight`]s, one slot per built node, in fixed-size storage buffers. A buffer is never
/// resized (a new size re-creates it and rebuilds every material bound to it); more nodes take new
/// pages. Small pages keep the upload of a frame's few changed records small (Bevy writes a
/// changed buffer whole).
#[derive(Default)]
struct LightPages {
    pages: Vec<LightPage>,
    free: BTreeSet<u32>,
}

struct LightPage {
    buffer: Handle<ShaderBuffer>,
    lights: Vec<PatchLight>,
    dirty: bool,
}

impl LightPages {
    /// A free slot: the lowest, so the nodes built in one frame share pages.
    fn alloc(&mut self, buffers: &mut Assets<ShaderBuffer>) -> u32 {
        if let Some(s) = self.free.pop_first() {
            return s;
        }
        let base = self.pages.len() as u32 * LIGHT_PAGE_SLOTS;
        let lights = vec![PatchLight::default(); LIGHT_PAGE_SLOTS as usize];
        self.pages.push(LightPage { buffer: buffers.add(light_buffer(&lights)), lights, dirty: false });
        self.free.extend(base + 1..base + LIGHT_PAGE_SLOTS);
        base
    }

    fn release(&mut self, slot: u32) {
        self.free.insert(slot);
    }

    /// The page holding `slot`, and the slot's index in it (`PlanetUniform::slot`).
    fn page(&self, slot: u32) -> (Handle<ShaderBuffer>, u32) {
        (self.pages[(slot / LIGHT_PAGE_SLOTS) as usize].buffer.clone(), slot % LIGHT_PAGE_SLOTS)
    }

    fn set(&mut self, slot: u32, light: PatchLight) {
        let page = &mut self.pages[(slot / LIGHT_PAGE_SLOTS) as usize];
        let l = &mut page.lights[(slot % LIGHT_PAGE_SLOTS) as usize];
        if *l != light {
            *l = light;
            page.dirty = true;
        }
    }

    /// Sends the changed pages (same size: Bevy writes them into their buffers in place).
    fn upload(&mut self, buffers: &mut Assets<ShaderBuffer>) {
        for page in self.pages.iter_mut().filter(|p| p.dirty) {
            page.dirty = false;
            if let Some(mut b) = buffers.get_mut(&page.buffer) {
                *b = light_buffer(&page.lights);
            }
        }
    }
}

/// A light page's storage buffer (render world only: the page keeps its own copy).
pub(crate) fn light_buffer(lights: &[PatchLight]) -> ShaderBuffer {
    let mut bytes = encase::StorageBuffer::new(Vec::<u8>::new());
    bytes.write(lights).expect("light page");
    ShaderBuffer::new(bytes.into_inner(), RenderAssetUsages::RENDER_WORLD)
}

/// SE's water filter (main-def.cfg ObjectFilters `ShowWater`, key `ToggleWater`): seas and lakes fade
/// out and in (planet.glsl's WaterSurfColor.a "water on/off animation" scales the water mask), here
/// over SE's OverlayFadeOut / OverlayFadeIn seconds.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct WaterDisplay {
    pub show: bool,
    /// The shown share, 0..1, following `show`.
    pub level: f32,
    pub fade_in_s: f32,
    pub fade_out_s: f32,
}

impl Default for WaterDisplay {
    fn default() -> Self {
        Self { show: true, level: 1.0, fade_in_s: 0.2, fade_out_s: 0.35 }
    }
}

impl WaterDisplay {
    /// Moves `level` towards `show` over `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.level = if self.show {
            (self.level + dt / self.fade_in_s.max(1e-3)).min(1.0)
        } else {
            (self.level - dt / self.fade_out_s.max(1e-3)).max(0.0)
        };
    }
}

fn animate_water_display(time: Res<Time>, mut water: ResMut<WaterDisplay>) {
    let target = if water.show { 1.0 } else { 0.0 };
    if water.level != target {
        water.step(time.delta_secs());
    }
}

/// SE's self-emission settings (main-def.cfg `EnableSelfEmission`, `SelfEmissionBright`; key
/// `ToggleSelfEmission`, GUI "Self-emission" / "Self-emission brightness"): city lights, lava and
/// thermal glow of planets (FUN_1404acdd0 drops the EMIS/THERM paths when off; FUN_1404bb110 scales
/// GlowColor and SurfParams4.w by the brightness).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct SelfEmission {
    pub enabled: bool,
    pub bright: f32,
}

impl Default for SelfEmission {
    fn default() -> Self {
        Self { enabled: true, bright: 1.0 }
    }
}

impl SelfEmission {
    /// The factor on glow maps and thermal emission.
    pub fn scale(&self) -> f32 {
        if self.enabled { self.bright.max(0.0) } else { 0.0 }
    }
}

fn load_surface_settings(mut commands: Commands, cfg: Res<crate::config::EngineConfig>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    let d = SelfEmission::default();
    commands.insert_resource(SelfEmission {
        enabled: def.find_deep("EnableSelfEmission").and_then(|i| i.bool()).unwrap_or(d.enabled),
        bright: def.find_deep("SelfEmissionBright").and_then(|i| i.f64()).map_or(d.bright, |v| v as f32),
    });
    let w = WaterDisplay::default();
    let show = def.find_deep("ShowWater").and_then(|i| i.bool()).unwrap_or(w.show);
    let f = |k: &str, d: f32| def.find_deep(k).and_then(|i| i.f64()).map_or(d, |v| v as f32);
    commands.insert_resource(WaterDisplay {
        show,
        level: if show { 1.0 } else { 0.0 },
        fade_in_s: f("OverlayFadeIn", w.fade_in_s),
        fade_out_s: f("OverlayFadeOut", w.fade_out_s),
    });
}

/// SE's `ThermalEmissionShift` (main-def.cfg 0), in thousand K.
const THERMAL_EMISSION_SHIFT: f32 = 0.0;

/// SE's thermal emission inputs (planet.glsl THERM with the tg_*_glow temperature maps): the body's
/// temperature as SE's surfTemperature (giants glow all over at it, `tg_gasgiant_glow`; rocky worlds
/// only where lava covers them, `tg_terra_glow` / `tg_selena_glow`), and the sun's heating
/// `0.010673289 L / d^2` (L in solar luminosities, d in AU) at the subsolar point.
pub(crate) fn thermal_uniform(terrain: &Terrain, light: &crate::system::BodyLighting) -> Vec4 {
    // SE's bulk class decides the generator: Neptune, Jupiter and Chthonia use the gas giant glow.
    let giant = terrain.proc.kind.is_giant() || terrain.surf_gen.classes.bulk >= se_core::surface_gen::bulk::NEPTUNE;
    let lava = terrain
        .surf_gen
        .get(se_core::surface_gen::LAVA_LEVEL)
        .unwrap_or(if terrain.proc.kind.id() == 3.0 { 0.5 } else { 0.0 });
    let t = terrain.proc.temperature_k as f32 / 1000.0;
    Vec4::new(t, lava, 0.010_673_289 * light.heat, if giant { 1.0 } else { 0.0 })
}

/// Bodies casting shadows on a lit body from its main star, kept on the CPU for the sky's sun-disc mask
/// (system.rs); the surface and clouds take eclipse.rs's casters of every light.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct EclipseUniform {
    /// xyz: occluder centre relative to the lit body (universe orientation, lit-body radii);
    /// w: occluder radius in lit-body radii, negative when it has an atmosphere.
    pub occ: [Vec4; 4],
    /// x: star angular radius (rad), y: occluder count.
    pub info: Vec4,
}

/// Whether the casters moved enough to be worth re-uploading the materials.
fn casters_differ(a: &[crate::eclipse::EclipseLight], b: &[crate::eclipse::EclipseLight]) -> bool {
    a.iter().zip(b).any(|(a, b)| {
        a.info.y != b.info.y
            || (a.info.x - b.info.x).abs() > a.info.x * 1e-3
            || a.casters.iter().zip(&b.casters).any(|(c, d)| (*c - *d).abs().max_element() > 2e-4)
    })
}

impl EclipseUniform {
    /// CPU version of `eclipse_light`: starlight reaching `p` (lit-body radii) from direction `l`.
    pub fn light_at(&self, p: DVec3, l: DVec3) -> Vec3 {
        let mut light = Vec3::ONE;
        let rs = (self.info.x as f64).max(1e-7);
        for o in self.occ.iter().take(self.info.y as usize) {
            let v = o.truncate().as_dvec3() - p;
            let dist = v.length();
            let r = (o.w as f64).abs();
            if dist <= r || v.dot(l) <= 0.0 {
                continue;
            }
            let ro = (r / dist).min(1.0).asin();
            let dir = v / dist;
            let sep = dir.cross(l).length().atan2(dir.dot(l));
            let f = disc_overlap(rs, ro, sep) as f32;
            let residual = if o.w < 0.0 { Vec3::new(0.1, 0.03, 0.006) } else { Vec3::ZERO };
            light *= Vec3::ONE - f * (Vec3::ONE - residual);
        }
        light
    }
}

/// Fraction of a disc of angular radius `rs` covered by one of radius `ro` at separation `d`.
pub(crate) fn disc_overlap(rs: f64, ro: f64, d: f64) -> f64 {
    if d >= rs + ro {
        return 0.0;
    }
    if d <= (ro - rs).abs() {
        return if ro >= rs { 1.0 } else { (ro * ro) / (rs * rs) };
    }
    let a1 = rs * rs * ((d * d + rs * rs - ro * ro) / (2.0 * d * rs)).clamp(-1.0, 1.0).acos();
    let a2 = ro * ro * ((d * d + ro * ro - rs * rs) / (2.0 * d * ro)).clamp(-1.0, 1.0).acos();
    let k = ((-d + rs + ro) * (d + rs - ro) * (d - rs + ro) * (d + rs + ro)).max(0.0).sqrt();
    ((a1 + a2 - 0.5 * k) / (std::f64::consts::PI * rs * rs)).clamp(0.0, 1.0)
}

/// Finds the bodies whose shadow (umbra or penumbra) can reach each resolved
/// terrain body, largest apparent size first. None with SE's eclipses switch off (Ctrl+L).
/// Only bodies over half a pixel get any: the others are visited only to clear what they had
/// (`held`, the bodies whose eclipse is not the default; a Terrain starts with the default).
fn update_eclipses(
    switches: Res<crate::switches::RenderSwitches>,
    bodies: Query<(Entity, &SystemBody)>,
    lights: Query<&BodyLighting>,
    mut set: ParamSet<(Query<&Terrain>, Query<&mut Terrain>)>,
    mut held: Local<HashSet<Entity>>,
) {
    let mut occluders: Vec<(Entity, ObjectRef, UPos, f64)> = Vec::new();
    let mut visit: Vec<Entity> = Vec::new();
    for (entity, b) in &bodies {
        if !b.star && b.radius_m > 1000.0 {
            occluders.push((entity, b.obj, b.pos, b.radius_m));
        }
        if b.px_radius > 0.5 {
            visit.push(entity);
        }
    }
    visit.extend(held.iter().copied().filter(|e| !bodies.get(*e).is_ok_and(|(_, b)| b.px_radius > 0.5)));
    let mut changed: Vec<(Entity, EclipseUniform)> = Vec::new();
    let terrains = set.p0();
    for entity in visit {
        // Gone, or its terrain is: nothing to clear.
        let Ok(terrain) = terrains.get(entity) else {
            held.remove(&entity);
            continue;
        };
        let (Ok((_, body)), Ok(light)) = (bodies.get(entity), lights.get(entity)) else { continue };
        let mut e = EclipseUniform::default();
        let star_dist = light.star_pos.delta_m(body.pos).length();
        if switches.eclipses && body.px_radius > 0.5 && light.star_radius_m > 0.0 && star_dist > 0.0 {
            let a_s = (light.star_radius_m / star_dist).min(1.0).asin();
            let mut found: Vec<(f64, Vec4)> = Vec::new();
            for &(occ, obj, pos, r_o) in &occluders {
                if obj == body.obj {
                    continue;
                }
                // Lit body relative to the occluder, and the occluder's anti-star axis.
                let w = body.pos.delta_m(pos);
                let to_star = light.star_pos.delta_m(pos).normalize_or(DVec3::Y);
                let t = -w.dot(to_star);
                if t <= 0.0 {
                    continue;
                }
                let perp = (w + to_star * t).length();
                if perp > body.radius_m + r_o + t * a_s {
                    continue;
                }
                let rel = -w / body.radius_m;
                let r = (r_o / body.radius_m) as f32;
                let atm = terrains.get(occ).is_ok_and(|t| t.atmosphere.is_some());
                found.push((r_o / w.length(), rel.as_vec3().extend(if atm { -r } else { r })));
            }
            found.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (i, (_, o)) in found.iter().take(4).enumerate() {
                e.occ[i] = *o;
            }
            e.info = Vec4::new(a_s as f32, found.len().min(4) as f32, 0.0, 0.0);
        }
        if e != terrain.eclipse {
            debug!("eclipse on {:?}: {:?}", body.obj, e);
            changed.push((entity, e));
        }
        if e == EclipseUniform::default() {
            held.remove(&entity);
        } else {
            held.insert(entity);
        }
    }
    let mut terrains = set.p1();
    for (entity, e) in changed {
        if let Ok(mut terrain) = terrains.get_mut(entity) {
            terrain.eclipse = e;
        }
    }
}

/// Unit direction of a vertex in the body frame (for latitude and noise).
pub const ATTRIBUTE_BODY_DIR: MeshVertexAttribute =
    MeshVertexAttribute::new("BodyDir", 0x5E57_0011, bevy::render::render_resource::VertexFormat::Float32x3);

/// Height of a vertex above the reference radius (sea level), meters.
pub const ATTRIBUTE_HEIGHT: MeshVertexAttribute =
    MeshVertexAttribute::new("TerrainHeight", 0x5E57_0010, bevy::render::render_resource::VertexFormat::Float32);

/// Geomorphing: xyz the offset from the vertex to where the parent node's surface has it (m,
/// node frame), w the parent's height minus the vertex's. A fresh node starts on its parent's
/// shape and morphs into its own as the camera nears it, so detail grows in instead of popping.
pub const ATTRIBUTE_MORPH: MeshVertexAttribute =
    MeshVertexAttribute::new("TerrainMorph", 0x5E57_0012, bevy::render::render_resource::VertexFormat::Float32x4);

/// Texels per edge of a node's normal map: shading detail at twice the mesh's resolution.
const NORMAL_RES: usize = 4 * (GRID - 1) + 1;
/// Height of the first fractal detail octave over its size (`detail_height`).
const DETAIL_AMPLITUDE: f64 = 0.02;
/// Smallest relief the terrain generator adds (m), at every level. SE's generator has fixed octave
/// counts that stop at hundreds of metres; below that its ground is smooth and the soils' normal
/// maps carry the relief. Metre-scale fractal relief would also tilt flat sand into the slopes of
/// SE's gravel and blocks soils (soils are matched by slope).
const FINEST_FEATURE_M: f64 = 20.0;
/// Height scale of the normal map's alpha, `0.5 + 0.5 tanh(h / SHORE_CODE_M)`: fine near sea level
/// (centimetres), coarse beyond a few tens of metres. Same constant in `planet.wesl`.
const SHORE_CODE_M: f64 = 8.0;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(PlanetMaterialKey)]
pub struct PlanetMaterial {
    #[uniform(0)]
    pub u: PlanetUniform,
    #[texture(1)]
    #[sampler(2)]
    pub color: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub lights: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub water: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    pub clouds: Handle<Image>,
    #[texture(9, dimension = "3d")]
    #[sampler(10)]
    pub inscatter: Handle<Image>,
    #[texture(11)]
    pub transmittance: Handle<Image>,
    #[texture(12)]
    pub irradiance: Handle<Image>,
    /// The body's ring texture (for ring shadows), or a placeholder.
    #[texture(13)]
    #[sampler(14)]
    pub ring: Handle<Image>,
    /// Biome tints, sample layers and soils of the body (`surface_tex.rs`).
    #[uniform(15)]
    pub biome: BiomeUniform,
    #[texture(16, dimension = "2d_array")]
    #[sampler(17)]
    pub samples: Handle<Image>,
    #[texture(18, dimension = "2d_array")]
    #[sampler(19)]
    pub soils: Handle<Image>,
    /// The node's normal map (body-frame normals, `NORMAL_RES` texels a side).
    #[texture(20)]
    #[sampler(21)]
    pub normals: Handle<Image>,
    /// The soils' normal and roughness/height/AO maps (sampled with the soils' sampler).
    #[texture(22, dimension = "2d_array")]
    pub soil_norm: Handle<Image>,
    #[texture(23, dimension = "2d_array")]
    pub soil_rhao: Handle<Image>,
    /// The sample layers' relief (normal, roughness, height), sampled with the samples' sampler.
    #[texture(24, dimension = "2d_array")]
    pub sample_relief: Handle<Image>,
    /// SE's 64^3 `Noise3DSampler` (read with `textureLoad`), for the cellular noises of the airless
    /// colour (tg_selena_color), matched to the height generator's.
    #[texture(25, dimension = "3d")]
    pub noise: Handle<Image>,
    /// The light page holding the node's [`PatchLight`] (at `u.slot`).
    #[storage(26, read_only)]
    pub light_page: Handle<ShaderBuffer>,
    /// The volumetric clouds' cover map and shadow record, for their shadows on the ground of the
    /// body they draw (volumetric_clouds.rs `CloudShadowAssets`).
    #[texture(27, dimension = "cube")]
    #[sampler(28)]
    pub cloud_cover: Handle<Image>,
    #[storage(29, read_only)]
    pub cloud_shadow: Handle<ShaderBuffer>,
    #[texture(30)]
    #[sampler(31)]
    pub cloud_map: Handle<Image>,
    /// The body's weather cube shaping that cover, and the weather's cloud pattern (read with the
    /// cover's sampler).
    #[texture(32, dimension = "cube")]
    pub cloud_weather: Handle<Image>,
    #[texture(33, dimension = "cube")]
    pub cloud_pattern: Handle<Image>,
    /// A node's skirts (the walls hung under its edges that close the cracks to coarser
    /// neighbours), drawn after every node's ground ([`Material::alpha_mode`]). Not a binding.
    pub skirt: bool,
    /// The noises the node's pipeline works out at its vertices ([`VertexNoise`]; not a binding):
    /// [`vertex_noise`]'s choice once that variant's pipelines are built, until then `None`, which
    /// draws the same ground with every pixel working them out ([`NodeNoise`]).
    pub variant: VertexNoise,
}

/// Pipeline key: the water surface is drawn from both sides with its own entry points; ground nodes
/// fine enough for it carry the colour generator's kilometre-scale noises at their vertices.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct PlanetMaterialKey {
    water: bool,
    noise: VertexNoise,
}

impl From<&PlanetMaterial> for PlanetMaterialKey {
    fn from(m: &PlanetMaterial) -> Self {
        let water = m.u.water.w > 0.5;
        Self { water, noise: if water { VertexNoise::None } else { m.variant } }
    }
}

/// A ground node's [`VertexNoise`]: the one it should use (`want`) and the one its ground and skirt
/// materials use (`used`). The driver builds each variant's pipeline in seconds (5-6 s for
/// planet.wesl on the user's RTX 5080), and a node whose pipeline is not built is not drawn: landing
/// once hid the ground for 10 s while the finer nodes' variants were built (the user's 2026-11-04
/// video). A node uses `None` until its own variant is ready (`pipeline_watch::GroundPipelines`),
/// then [`upgrade_node_noise`] switches it.
#[derive(Component)]
pub(crate) struct NodeNoise {
    want: VertexNoise,
    pub(crate) used: VertexNoise,
    pub(crate) sea: bool,
    ground: Handle<PlanetMaterial>,
    skirt: Handle<PlanetMaterial>,
}

/// Switches the ground nodes waiting for their [`VertexNoise`] variant to it once its pipelines are
/// built (a material change only then, never every frame).
fn upgrade_node_noise(
    pipes: Res<crate::pipeline_watch::GroundPipelines>,
    mut seen: Local<u32>,
    mut nodes: Query<&mut NodeNoise, Allow<Disabled>>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
) {
    let rev = pipes.revision();
    if rev == *seen {
        return;
    }
    *seen = rev;
    for mut n in &mut nodes {
        if n.used != n.want && pipes.usable(n.want) == n.want {
            let want = n.want;
            for h in [n.ground.clone(), n.skirt.clone()] {
                if let Some(mut m) = materials.get_mut(&h) {
                    m.variant = want;
                }
            }
            n.used = want;
        }
    }
}

/// Which of SE's colour generator noises a ground node works out at its vertices for its pixels to
/// interpolate (`planet.wesl`), instead of every pixel working them out: most of the ground's cost
/// on procedural worlds, though they vary only over kilometres.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum VertexNoise {
    #[default]
    None,
    /// The airless worlds' (`selena_noise`, 43 noises a pixel).
    Selena,
    /// The terra worlds' planet-wide ones (`terra_noise`, 26 a pixel, 34 with plants).
    Terra,
    /// Those and the flatland climate distortion (`terra_climate_noise`, 28 more), whose ridges
    /// by SE's hillsFreq are finer: only the nodes nearest the camera.
    TerraClimate,
    /// Nodes too coarse for all of `terra_noise`: the vertices carry its warp and the octaves of its
    /// sums that fit (`planet.wesl` `vertex_octaves`), each pixel adds its own finer ones. Measured
    /// on the moon RS 8514-916-6-8246-147 5.1 the per-pixel noise on such nodes was about half the
    /// ground's GPU time from 13 to 350 km up (2026-11-04).
    TerraPartial,
}

impl VertexNoise {
    pub(crate) const ALL: [VertexNoise; 5] =
        [VertexNoise::None, VertexNoise::Selena, VertexNoise::Terra, VertexNoise::TerraClimate, VertexNoise::TerraPartial];

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|&v| v == self).unwrap_or(0)
    }

    /// The variant a planet.wesl pipeline was built for, from whether each define is set.
    pub(crate) fn from_defs(has: impl Fn(&str) -> bool) -> VertexNoise {
        let set = |v: VertexNoise| Self::ALL.iter().flat_map(|a| a.shader_defs()).all(|d| has(d) == v.shader_defs().contains(d));
        Self::ALL.into_iter().find(|&v| set(v)).unwrap_or(VertexNoise::None)
    }

    pub(crate) fn shader_defs(self) -> &'static [&'static str] {
        match self {
            VertexNoise::None => &[],
            VertexNoise::Selena => &["SELENA_AT_VERTICES"],
            VertexNoise::Terra => &["TERRA_AT_VERTICES"],
            VertexNoise::TerraClimate => &["TERRA_AT_VERTICES", "TERRA_CLIMATE_AT_VERTICES"],
            VertexNoise::TerraPartial => &["TERRA_AT_VERTICES", "TERRA_PARTIAL"],
        }
    }
}

/// [`VertexNoise`] for a ground node: a set of noises goes to the vertices only where the node's
/// cells (up to 1.5 times their mean size on the cube face) are at most 1/16 of the finest of their
/// features, which their warps can halve. The colour terms then differ from the per-pixel ones by
/// under 1/1000, a quarter of an 8-bit step (measured on RSN 0-7-460129-1254-197-12 4: under
/// 1/16384 with cells of 1/64, and the error grows with the cell's square), and every pixel there
/// keeps all their octaves, as the vertices do. Coarser nodes, and other worlds, keep them per
/// pixel. `textured`: the body has a colour map; `edge_m` the node's edge, `radius_m` the body's
/// (the material's `soil_tile.z`, `proc_params2.x`).
fn vertex_noise(b: &crate::surface_tex::BiomeUniform, textured: bool, edge_m: f32, radius_m: f32) -> VertexNoise {
    let c = &b.climate;
    // The biomes on and SE's own colour generators; not the procedural asteroids' own colour,
    // which reads none of these noises.
    if b.flags.x < 0.5 || c[4].w < 0.5 || (c[7].z > 1.5 && !textured) {
        return VertexNoise::None;
    }
    let lac = 2.218281828459_f64;
    let cell_m = edge_m as f64 / (GRID - 1) as f64 * 1.5;
    // Whether the node is fine enough for noises whose finest octave has frequency `freq` on the
    // unit sphere.
    let fits = |freq: f64| cell_m * 16.0 * freq <= radius_m as f64;
    if c[7].z > 0.5 {
        // Airless: the latitude's (6 octaves from 0.7), the variation's (8 from 0.378) and, on
        // cratered worlds, the young terrain's (8 from 1).
        let mut freq = (0.7 * lac.powi(5)).max(0.378 * lac.powi(7));
        if c[7].w > 0.05 {
            freq = freq.max(lac.powi(7));
        }
        return if fits(freq) { VertexNoise::Selena } else { VertexNoise::None };
    }
    // Terra: the latitude's, the albedo variation's (8 from 1) and with plants the humidity's (8
    // from 1.73); then the climate distortion's ridges (4 octaves from 100 pf) and its colour Fbm (4
    // from df), whose own warp (1.2 Fbm3D at 0.38 df) can shorten its features three times.
    let mut freq = (0.7 * lac.powi(5)).max(lac.powi(7));
    if b.flags.z > 0.5 {
        freq = freq.max(1.73 * lac.powi(7));
    }
    if !fits(freq) {
        // The warp (4 octaves from 0.07) whole at the vertices, the sums split.
        return if fits(0.07 * lac.powi(3)) { VertexNoise::TerraPartial } else { VertexNoise::None };
    }
    let pf = 0.0005 * c[7].x as f64 / (c[7].y as f64 * c[7].y as f64).max(1e-6);
    let df = c[5].y as f64 * 0.005;
    if fits((100.0 * pf).max(3.0 * df) * lac.powi(3)) { VertexNoise::TerraClimate } else { VertexNoise::Terra }
}

impl Material for PlanetMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/planet.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/planet.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // The sea surface lets the seabed (drawn by the ground pass) show through from above.
        // A node's skirts in the alpha-mask phase, which Bevy draws after the opaque one in the same
        // pass (nothing is discarded): hidden behind the neighbours' ground already drawn, their
        // pixels fail the depth test before shading. Drawn with their node they were shaded in full
        // wherever they came before the neighbour that hides them: a third of the ground's GPU time
        // near the surface (2026-11-04, the user's planet at Ultra 3440x1440: 9 of 25 ms).
        if self.u.water.w > 0.5 {
            AlphaMode::Premultiplied
        } else if self.skirt {
            AlphaMode::Mask(0.5)
        } else {
            AlphaMode::Opaque
        }
    }
    fn depth_bias(&self) -> f32 {
        // The sea surface after the sky shells, as the clouds: from under water the sky shows through
        // it (water.glsl's alpha), and looking up the shell (sorted by the planet's centre, behind
        // the camera) would come after it and fail its depth.
        if self.u.water.w > 0.5 { 1.0e10 } else { 0.0 }
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let water = key.bind_group_data.water;
        for def in key.bind_group_data.noise.shader_defs() {
            descriptor.vertex.shader_defs.push((*def).into());
            if let Some(f) = descriptor.fragment.as_mut() {
                f.shader_defs.push((*def).into());
            }
        }
        if water {
            descriptor.primitive.cull_mode = None;
            // Still hides what is drawn after it beyond it (far from the camera it is opaque).
            if let Some(ds) = descriptor.depth_stencil.as_mut() {
                ds.depth_write_enabled = Some(true);
            }
        }
        if let Some(f) = descriptor.fragment.as_mut() {
            f.entry_point = Some(if water { "water_fragment" } else { "fragment" }.into());
        }
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            ATTRIBUTE_HEIGHT.at_shader_location(3),
            ATTRIBUTE_BODY_DIR.at_shader_location(4),
            ATTRIBUTE_MORPH.at_shader_location(5),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TerrainLayers {
    pub color: Option<TileLayer>,
    pub water: Option<TileLayer>,
    pub bump: Option<TileLayer>,
    pub lights: Option<TileLayer>,
    /// One per `Terrain::clouds` layer (`None` for a procedural or missing map).
    pub clouds: Vec<Option<TileLayer>>,
}

/// One of SE's cloud layers: each `Clouds` block of a body is its own shell (Titan has three haze
/// layers, Mars two textured ones and a procedural one).
#[derive(Debug, Clone)]
pub struct CloudLayer {
    pub map: Option<String>,
    /// `DiffMapAlpha None`: the map is the clouds' colour and the layer is opaque (Venus). Otherwise
    /// (SE's default "Transparent") the map is the clouds' opacity.
    pub opaque_map: bool,
    pub opacity: f32,
    /// Altitude above the reference radius, meters.
    pub height: f64,
    /// The clouds' albedo, SE's `ModulateColor` x `ModulateBright`.
    pub color: Vec3,
    /// x `DayAmbient`, y `SubsurfScatBright`, z `SubsurfScatPower`, w `Hapke`.
    pub look: Vec4,
    /// The relief of the cloud tops (m): SE's `BumpHeight` when the layer has a `BumpMap` (SE's
    /// Earth uses its cloud map), else 0.
    pub bump_m: f32,
    /// The layer turns about the pole by `FlipMap` (180 degrees) plus its `Velocity` drift
    /// (`rad_per_s`, signed by the body's sense of rotation) over the simulation time.
    pub flip: bool,
    pub rad_per_s: f64,
}

impl CloudLayer {
    fn from_block(c: &se_core::sc::Block, radius: f64, sense: f64) -> Self {
        let f = |k: &str, d: f64| c.f64(k).unwrap_or(d) as f32;
        let m = c.vec3("ModulateColor").unwrap_or([1.0; 3]);
        let k = c.f64("ModulateBright").unwrap_or(1.0);
        let height = c.f64("Height").unwrap_or(5.0) * 1000.0;
        let flip = c.bool("FlipMap").unwrap_or(false);
        Self {
            map: c.str("DiffMap").map(str::to_string),
            opaque_map: c.str("DiffMapAlpha").is_some_and(|a| a.eq_ignore_ascii_case("None")),
            opacity: f("Opacity", 1.0),
            height,
            color: Vec3::new((m[0] * k) as f32, (m[1] * k) as f32, (m[2] * k) as f32),
            // SE's catalog loader defaults (FUN_1405057e0): DayAmbient 0.07, no subsurface scattering.
            look: Vec4::new(f("DayAmbient", 0.07), f("SubsurfScatBright", 0.0), f("SubsurfScatPower", 0.0), f("Hapke", 0.0)),
            // A procedural layer's relief is its generated height map's (tg_clouds_height.glsl).
            bump_m: if c.str("BumpMap").is_some() || c.str("DiffMap").is_none() {
                (c.f64("BumpHeight").unwrap_or(0.0) * 1000.0) as f32
            } else {
                0.0
            },
            flip,
            rad_per_s: sense * c.f64("Velocity").unwrap_or(0.0) / 3.6 / (radius + height),
        }
    }

    /// One of SE's generated layers (procedural, the loader's defaults otherwise).
    fn generated(g: &se_core::climate::CloudGen, radius: f64, sense: f64) -> Self {
        let mut l = Self::from_block(&se_core::sc::Block::default(), radius, sense);
        l.height = g.height_km as f64 * 1000.0;
        l.bump_m = g.bump_km * 1000.0;
        l.rad_per_s = sense * g.velocity_kmh as f64 / 3.6 / (radius + l.height);
        l
    }

    /// SE's default layer (5 km up, no relief) on an Earth-sized body, for tests.
    #[cfg(test)]
    pub fn test_layer() -> Self {
        Self::from_block(&se_core::sc::Block::default(), 6.4e6, 1.0)
    }

    /// The layer's turn about the body's pole (rad) at simulation time `jd`.
    pub fn spin_at(&self, jd: f64) -> f64 {
        let drift = (self.rad_per_s * (jd - se_core::time::J2000) * 86_400.0).rem_euclid(std::f64::consts::TAU);
        drift + if self.flip { std::f64::consts::PI } else { 0.0 }
    }
}

enum NodeState {
    Loading(Task<NodeBuild>),
    /// The ground patch and one patch per drawn cloud layer.
    Ready(Entity, Vec<Entity>),
}

struct NodeSlot {
    state: NodeState,
    last_used: u64,
    /// Frame the node's entity was spawned. Spawns apply at the end of that frame, so
    /// the node is only drawable from the next one (else a tile-sized hole flashes
    /// between hiding the parent and showing the children).
    ready_frame: u64,
    /// The `Terrain::light_gen` its materials were last lit for.
    light_gen: u64,
    /// Its [`PatchLight`] in `Terrain::lights` (once built).
    light: u32,
}

#[derive(Component)]
pub struct Terrain {
    pub radius: f64,
    pub surface: SurfaceParams,
    /// The body's cloud layers, in catalog order.
    pub clouds: Vec<CloudLayer>,
    /// Layers without a map are procedural, with the body's (coverage, frequency, octaves, 0):
    /// SE keeps these keys on the body, so every layer shares them (the last block's win).
    /// `None` for giants, which draw their own look.
    pub cloud_proc: Option<Vec4>,
    /// The surface's `DayAmbient` (SE's loader default 0.07): ambient light on the day side.
    pub day_ambient: f32,
    /// The surface's tint, SE's `ModulateColor` x `ModulateBright` (on the colour map), and in w its
    /// `Hapke` (or `Lommel`): how far the shading goes from Lambert to SE's Hapke BRDF.
    pub surf_modulate: Vec4,
    /// SE's opposition surge (`HapkeParams`): `SpotBright`, 1 / `SpotWidth`, `SpotBrightCB`,
    /// 1 / `SpotWidthCB` (a width of 0: no spot).
    pub surf_hapke: Vec4,
    pub base_color: Vec3,
    pub has_ocean: bool,
    /// Procedural surface (used where real data is missing or too coarse).
    pub proc: ProcSurface,
    /// SE's generated surface parameters (classes, SurfStyle, climate, landforms).
    pub surf_gen: se_core::surface_gen::SurfaceGen,
    /// Water `SpecBrightWater` and `RoughnessWater` (catalog, else SE's class defaults).
    pub water_spec: (f32, f32),
    /// SE's water colour and absorption from the body's preset ([`SurfaceTextures::ocean_palette`]):
    /// `OceanPalette` `Deep` (linear) and `Absorption` (per km). SE reads no `colorSea` key.
    pub water_color: Vec3,
    pub water_fog: Vec4,
    /// Precomputed scattering tables, if the body has an atmosphere.
    pub atmosphere: Option<AtmosphereGpu>,
    /// The atmosphere's `Bright` and look (Opacity, Hue, Saturation, SkyLight), set with `atmosphere`.
    pub atm_bright: f32,
    pub atm_look: Vec4,
    /// The equilibrium and mean surface temperatures (K) SE's glowing atmosphere is drawn from
    /// ([`glow_color_atmo`]).
    pub glow_temps: (f32, f32),
    /// Biome texturing; `None` until chosen (and for bodies without a preset).
    pub(crate) biome: Option<BiomeUniform>,
    biome_chosen: bool,
    layers: Option<Arc<TerrainLayers>>,
    /// Height grids of loaded nodes, for ground-height queries.
    node_heights: NodeMap<TileKey, Arc<Vec<f32>>>,
    /// Lowest and highest ground of loaded nodes (m above the reference radius), for the LOD.
    node_hrange: NodeMap<TileKey, (f32, f32)>,
    /// Nodes drawn this frame (sorted): the ground the camera collides with.
    visible: Vec<TileKey>,
    /// Bumped whenever `visible` changes (the scatter's height pages follow it).
    pub(crate) visible_rev: u64,
    /// The body's lighting as every node's light record holds it (without the geomorph distance),
    /// for objects standing on the ground (`scatter.rs`).
    pub(crate) body_light: PatchLight,
    /// A built node's material uniform: its per-body fields (atmosphere, rings, glow, the cloud key)
    /// are the same for every node, for objects standing on the ground.
    pub(crate) body_uniform: Option<PlanetUniform>,
    /// Nodes with a patch (ground or cloud) shown: the only ones whose visibility can change next
    /// frame, so the thousands of cached ones are not touched every frame.
    drawn: Vec<TileKey>,
    /// The turned cloud layers' node lists and the clouds switch the visibility pass last ran for
    /// (with `visible`); `None` when it must run. With the same inputs it would change nothing.
    vis_inputs: Option<(Vec<Option<Vec<TileKey>>>, bool)>,
    /// The `light_gen` every node in `drawn` is lit for; `None` once `drawn` has changed.
    drawn_lit: Option<u64>,
    /// Radii of the highest and the lowest ground, for the horizon test.
    horizon_top: f64,
    horizon_floor: f64,
    nodes: NodeMap<TileKey, NodeSlot>,
    /// The nodes still being built (`NodeState::Loading`), in the order they were started: the
    /// only ones to poll.
    loading: Vec<TileKey>,
    /// SE's record flag "locked to a star" (+0x165c bit 2), for its height generator's climate.
    pub tidal_lock: bool,
    /// SE's own height generator on the GPU (`terrain_gen`), for worlds with SE surface parameters
    /// and no height data: requested grids (job id, frame) and the grids read back (with the frame
    /// they last served a request).
    gen_jobs: NodeMap<TileKey, (u64, u64)>,
    gen_tiles: NodeMap<TileKey, (Arc<HeightTile>, u64)>,
    gen_failed: bool,
    frame: u64,
    last_sun: Vec4,
    last_ambient: f32,
    last_emission: SelfEmission,
    last_water: f32,
    /// `TerrainQuality::split_scale` the materials' geomorph distance (`soil_tile.w`) was last set from.
    last_split: f64,
    last_extra: [Vec4; MAX_LIGHTS - 1],
    /// Current shadows from other bodies, and the value last sent to the materials.
    pub eclipse: EclipseUniform,
    /// The materials' casters ([`crate::eclipse::EclipseCasters`]) and the last uploaded ones.
    casters: [crate::eclipse::EclipseLight; MAX_LIGHTS],
    last_casters: [crate::eclipse::EclipseLight; MAX_LIGHTS],
    /// Bumped when the lighting changes: shown nodes take it at once, cached ones when shown again.
    light_gen: u64,
    /// The built nodes' lighting.
    lights: LightPages,
}

/// The light an atmosphere of `Bright` `atm_bright` scatters (haze, the sky seen in water) relative to
/// the light on the ground: SE multiplies its inscatter by `Bright` x `hdrAtmoScatterBright`, its
/// surfaces by `hdrPlanetSurfaceBright`.
pub fn scatter_rel(atm_bright: f32) -> f32 {
    atm_bright * ATM_SCATTER_SCALE / ATM_SUN_SCALE
}

/// SE's GlowColorAtmo (FUN_1404c4250, HDR photo mode), the light of a hot atmosphere that sky.glsl
/// and planet.glsl (THERM) scatter as if lit from the planet's centre. Its strength comes from the
/// equilibrium temperature `t_eq` (record +0x1248; none below 644.211 K):
/// `(T / 5572)^4 x emission / hdrAtmoScatterBright x 10^(8.36619 - 3880.12 / T - 1380540 / T^2
/// - 0.775597 ln T) x` the star surface gain at T (FUN_1404c4090, [`crate::system::star_surface_gain`]),
/// none under 0.001 and `(k - 0.001) / 0.002` up to 0.003; its colour from the PlanckFunction table
/// at the surface temperature `t_surf` (+0x1250). `emission` is the self-emission scale (renderer
/// +0xc80, 0 with self-emission off).
pub fn glow_color_atmo(t_eq: f64, t_surf: f64, emission: f32) -> Vec3 {
    if t_eq < 644.211 || emission <= 0.0 {
        return Vec3::ZERO;
    }
    let x = t_eq.min(1.0e5);
    let curve = 10f64.powf(8.366_19 - 3880.12 / x - 1_380_540.0 / (x * x) - 0.775_597 * x.ln());
    let mut k = (t_eq / 5572.0).powi(4) * emission as f64 / ATM_SCATTER_SCALE as f64
        * curve
        * crate::system::star_surface_gain(t_eq);
    if k < 0.001 {
        return Vec3::ZERO;
    }
    if k < 0.003 {
        k = (k - 0.001) / 0.002;
    }
    let c = crate::blackholes::planck_lookup(t_surf);
    Vec3::new((k * c[0]) as f32, (k * c[1]) as f32, (k * c[2]) as f32)
}

impl Terrain {
    /// Nodes whose tiles are still being built (the HD screenshot's and skybox export's "Wait
    /// loading completeness", `capture.rs`).
    pub fn loading_nodes(&self) -> usize {
        self.loading.len()
    }

    /// The terrain's resident nodes and kept generated grids (perf.rs).
    pub fn resident(&self) -> (usize, usize) {
        (self.nodes.len(), self.gen_tiles.len())
    }

    /// The middle of the ground's relief (m above the radius), never below the sea at the radius: where
    /// SE's generated cloud layers count their heights from (se_core `cloud_layers`' base), and where
    /// the volumetric clouds' lower atmosphere begins.
    pub fn ground_mid_m(&self) -> f32 {
        let mid = if self.proc.se.is_some() {
            (0.5 - gen_ref(&self.proc)) * self.proc.bump_m
        } else {
            (0.5 * self.surface.bump_height_km - self.surface.bump_offset_km) * 1000.0
        };
        mid.max(0.0) as f32
    }

    /// SE's GlowColorAtmo for this body's atmosphere at self-emission scale `emission`; 0 without one.
    pub fn glow_atmo(&self, emission: f32) -> Vec3 {
        if self.atmosphere.is_none() {
            return Vec3::ZERO;
        }
        glow_color_atmo(self.glow_temps.0 as f64, self.glow_temps.1 as f64, emission)
    }

    /// `completion` is SE's generated atmosphere and ocean for a body the catalog leaves without;
    /// `bump_km` its `BumpHeight` (catalog or SE's default).
    pub fn new(
        radius: f64,
        body: &BodyRecord,
        temperature_k: f64,
        surf_gen: se_core::surface_gen::SurfaceGen,
        completion: Option<&se_core::climate::Completion>,
        bump_km: f64,
    ) -> Self {
        let b = &body.block;
        let surface = b.sub("Surface").map(SurfaceParams::from_block).unwrap_or_default();
        let cloud_blocks: Vec<&se_core::sc::Block> = if b.bool("NoClouds").unwrap_or(false) {
            Vec::new()
        } else {
            b.all("Clouds").filter_map(|i| i.block.as_ref()).collect()
        };
        let sense = match &body.rotation {
            se_core::rotation::RotationModel::Iau(r) if r.rotation_rate < 0.0 => -1.0,
            se_core::rotation::RotationModel::Simple(r) if r.period_hours < 0.0 => -1.0,
            _ => 1.0,
        };
        let mut clouds: Vec<CloudLayer> = cloud_blocks.iter().map(|c| CloudLayer::from_block(c, radius, sense)).collect();
        // A body without a `Clouds` block gets SE's cloud layers when its completion forms clouds
        // (above 0.05 atm, FUN_1405057e0): 1 to PlanetCloudsTerra of them at SE's heights, reliefs
        // and winds, with the layer defaults and SE's generated cloud parameters.
        // Giants and brown dwarfs keep their clouds in the surface: SE's height shader leaves
        // their cloud layers empty (tg_gasgiant_height.glsl / tg_browndwarf_height.glsl).
        let giant = (se_core::surface_gen::bulk::NEPTUNE..=se_core::surface_gen::bulk::BROWN_DWARF).contains(&surf_gen.classes.bulk);
        let no_clouds = b.bool("NoClouds").unwrap_or(false) || b.sub("Atmosphere").is_some() || giant;
        if clouds.is_empty() && !no_clouds {
            if let Some(c) = completion.filter(|c| c.has_clouds) {
                clouds.extend(c.clouds.iter().map(|g| CloudLayer::generated(g, radius, sense)));
            }
        }
        let base = body
            .color
            .map(|c| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32))
            .or(surf_gen.color.map(Vec3::from_array))
            .unwrap_or_else(|| class_color(body.class.as_deref().unwrap_or("")));
        let has_atm = (b.sub("Atmosphere").is_some() && !b.bool("NoAtmosphere").unwrap_or(false))
            || completion.is_some_and(|c| c.has_atmosphere());
        let has_ocean = (b.sub("Ocean").is_some() && !b.bool("NoOcean").unwrap_or(false)) || completion.is_some_and(|c| c.has_ocean);
        let mut proc = ProcSurface::for_body(
            body.names.first().map(String::as_str).unwrap_or("body"),
            body.class.as_deref(),
            radius,
            body.mass_kg,
            temperature_k,
            has_atm,
            has_ocean,
            b.sub("Life").is_some(),
        );
        // Without a bump map the relief is SE's procedural one, spanning `BumpHeight` (catalog or
        // SE's default) up from the bottom (`gen_ref`). Asteroids keep theirs whole (SE's default
        // is up to 0.9 R, their irregular shapes); other bodies are held within 0.2 R.
        let asteroid = surf_gen.classes.bulk == se_core::surface_gen::bulk::ASTEROID;
        let max_relief = radius * if asteroid { 1.0 } else { 0.2 };
        if surface.bump_map.is_none() {
            proc = proc.with_se(surf_gen.clone(), bump_km.min(max_relief / 1000.0));
        }
        let hmax = if proc.se.is_some() {
            (1.0 - gen_ref(&proc)).max(0.3) * proc.bump_m + 300.0
        } else {
            (surface.bump_height_km - surface.bump_offset_km).max(0.0) * 1000.0 + 300.0
        }
        .min(max_relief);
        let hmin = if proc.se.is_some() {
            (gen_ref(&proc) + 0.05) * proc.bump_m + 300.0
        } else {
            surface.bump_offset_km.max(0.0) * 1000.0 + 300.0
        }
        .min(max_relief);
        let (horizon_top, horizon_floor) = (radius + hmax, radius - hmin);
        // Giants draw their own look.
        // SE's generated (or given) cloud parameters: Coverage, mainFreq, mainOctaves.
        let cloud_proc = (clouds.iter().any(|c| c.map.is_none()) && !proc.kind.is_giant()).then(|| {
            use se_core::surface_gen as sg;
            let g = |i: usize, d: f32| surf_gen.get(i).unwrap_or(d);
            Vec4::new(g(sg::CLOUDS_COVERAGE, 0.5), g(sg::CLOUDS_MAIN_FREQ, 1.0), g(sg::CLOUDS_MAIN_OCTAVES, 6.0), 0.0)
        });
        let water_spec = (
            b.sub("Surface")
                .and_then(|s| s.f64("SpecBrightWater"))
                .map_or_else(|| se_core::surface_gen::water_spec_default(surf_gen.classes.surface), |v| v as f32),
            b.sub("Surface").and_then(|s| s.f64("RoughnessWater")).map_or(0.35, |v| v as f32),
        );
        Self {
            radius,
            proc,
            water_spec,
            // SE's defaults (FUN_140166410) until the preset is chosen.
            water_color: Vec3::new(0.04, 0.1, 0.2).powf(2.2),
            water_fog: Vec4::new(40.0, 5.0, 1.0, 5.0),
            surf_gen,
            surface,
            clouds,
            cloud_proc,
            day_ambient: b.sub("Surface").and_then(|s| s.f64("DayAmbient")).unwrap_or(0.07) as f32,
            surf_modulate: {
                let s = b.sub("Surface");
                let m = s.and_then(|s| s.vec3("ModulateColor")).unwrap_or([1.0; 3]);
                let k = s.and_then(|s| s.f64("ModulateBright")).unwrap_or(1.0);
                let hapke = s.and_then(|s| s.f64("Hapke").or_else(|| s.f64("Lommel"))).unwrap_or(0.0);
                Vec4::new((m[0] * k) as f32, (m[1] * k) as f32, (m[2] * k) as f32, hapke as f32)
            },
            surf_hapke: {
                let s = b.sub("Surface");
                let f = |k: &str| s.and_then(|s| s.f64(k)).unwrap_or(0.0) as f32;
                let inv = |w: f32| if w > 0.0 { 1.0 / w } else { 0.0 };
                let (bw, cw) = (f("SpotWidth"), f("SpotWidthCB"));
                Vec4::new(if bw > 0.0 { f("SpotBright") } else { 0.0 }, inv(bw), if cw > 0.0 { f("SpotBrightCB") } else { 0.0 }, inv(cw))
            },
            base_color: base,
            atmosphere: None,
            atm_bright: 1.0,
            atm_look: crate::atmosphere::ATM_LOOK_DEFAULT,
            glow_temps: (0.0, 0.0),
            biome: None,
            biome_chosen: false,
            has_ocean,
            layers: None,
            node_heights: NodeMap::new(),
            node_hrange: NodeMap::new(),
            visible: Vec::new(),
            visible_rev: 0,
            body_light: PatchLight::default(),
            body_uniform: None,
            drawn: Vec::new(),
            vis_inputs: None,
            drawn_lit: None,
            horizon_top,
            horizon_floor,
            nodes: NodeMap::new(),
            loading: Vec::new(),
            tidal_lock: false,
            gen_jobs: NodeMap::new(),
            gen_tiles: NodeMap::new(),
            gen_failed: false,
            frame: 0,
            last_sun: Vec4::ZERO,
            last_ambient: 0.0,
            last_emission: SelfEmission::default(),
            last_water: 1.0,
            last_split: 0.0,
            last_extra: [Vec4::ZERO; MAX_LIGHTS - 1],
            eclipse: EclipseUniform::default(),
            casters: Default::default(),
            last_casters: Default::default(),
            light_gen: 0,
            lights: LightPages::default(),
        }
    }

    /// Height of the rendered surface (m above the reference radius) in body-frame
    /// direction `dir`: from the node drawn there, else the deepest loaded one; `None`
    /// if nothing is loaded. A coarser node stays on screen until all its children
    /// load, and it can sit metres above them, so the drawn one is what counts.
    /// What a ship rests on: the ground, or the sea's surface over it (ships float).
    pub fn surface_height(&self, dir: DVec3) -> f64 {
        self.surface_sample(dir).map_or(0.0, |s| s.0)
    }

    /// [`Self::surface_height`] and the level of the node it comes from (how fine it is); `None`
    /// if nothing is loaded there.
    pub fn surface_sample(&self, dir: DVec3) -> Option<(f64, u8)> {
        let (ground, level) = self.ground_sample(dir)?;
        Some((if self.has_ocean { ground.max(0.0) } else { ground }, level))
    }

    /// The nearest coast to `dir` in the loaded terrain (body frame): a point `back` m from it
    /// on the land side, and the level direction from there towards the sea.
    pub fn nearest_shore(&self, dir: DVec3, radius: f64, back: f64) -> Option<(DVec3, DVec3)> {
        let e1 = dir.any_orthonormal_vector();
        let e2 = dir.cross(e1);
        let at = |b: f64, d: f64| (dir * (d / radius).cos() + (e1 * b.cos() + e2 * b.sin()) * (d / radius).sin()).normalize();
        let wet0 = self.ground_height(dir)? < 0.0;
        let (mut prev, mut d) = (0.0, 50.0);
        while d < 300e3 {
            for k in 0..48 {
                let b = k as f64 * std::f64::consts::TAU / 48.0;
                if self.ground_height(at(b, d)).is_none_or(|h| (h < 0.0) == wet0) {
                    continue;
                }
                let (mut lo, mut hi) = (prev, d);
                for _ in 0..24 {
                    let m = 0.5 * (lo + hi);
                    if self.ground_height(at(b, m)).is_some_and(|h| (h < 0.0) == wet0) { lo = m } else { hi = m }
                }
                let land = at(b, if wet0 { hi + back } else { hi - back });
                let outward = at(b, hi + 1.0) - at(b, hi);
                let to_sea = if wet0 { -outward } else { outward };
                return Some((land, (to_sea - land * to_sea.dot(land)).normalize()));
            }
            prev = d;
            d *= 1.3;
        }
        None
    }

    /// Radius (m) no ground rises above: up to twice the radius on an asteroid, whose relief
    /// spans up to a whole radius.
    pub fn top_radius(&self) -> f64 {
        self.horizon_top
    }

    /// Radius (m) up to which a node shows past the horizon: its ground or its highest cloud layer.
    fn cull_top(&self) -> f64 {
        self.clouds.iter().map(|c| self.radius + c.height).fold(self.horizon_top, f64::max)
    }

    pub fn ground_height(&self, dir: DVec3) -> Option<f64> {
        self.ground_sample(dir).map(|s| s.0)
    }

    /// The nodes drawn this frame (sorted): a partition of the ground up to the horizon.
    pub(crate) fn drawn_nodes(&self) -> &[TileKey] {
        &self.visible
    }

    /// A loaded node's height grid (`GRID` x `GRID`, m above the radius, row by row along v).
    pub(crate) fn node_grid(&self, key: &TileKey) -> Option<&Arc<Vec<f32>>> {
        self.node_heights.get(key)
    }

    /// The graphics quality's split factor the nodes' geomorph distance is set from.
    pub(crate) fn split_scale(&self) -> f64 {
        self.last_split
    }

    /// [`Self::ground_height`] and the level of the node it comes from.
    fn ground_sample(&self, dir: DVec3) -> Option<(f64, u8)> {
        let (face, u, v) = CubeFace::from_dir(dir);
        let mut drawn: Option<(TileKey, &Arc<Vec<f32>>)> = None;
        let mut deepest: Option<(TileKey, &Arc<Vec<f32>>)> = None;
        for level in 0..=MAX_GEOM_LEVEL {
            let n = (1u64 << level) as f64;
            let key = TileKey {
                face,
                level,
                row: ((v * n).floor() as u32).min(n as u32 - 1),
                col: ((u * n).floor() as u32).min(n as u32 - 1),
            };
            if let Some(h) = self.node_heights.get(&key) {
                deepest = Some((key, h));
                if self.visible.binary_search(&key).is_ok() {
                    drawn = Some((key, h));
                }
            }
        }
        let (key, h) = drawn.or(deepest)?;
        let (u0, v0, size) = key.uv_rect();
        let fx = ((u - u0) / size * (GRID - 1) as f64).clamp(0.0, (GRID - 1) as f64);
        let fy = ((v - v0) / size * (GRID - 1) as f64).clamp(0.0, (GRID - 1) as f64);
        let (x0, y0) = ((fx.floor() as usize).min(GRID - 2), (fy.floor() as usize).min(GRID - 2));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let at = |x: usize, y: usize| h[y * GRID + x] as f64;
        // The mesh splits each cell along the (x+1, y)-(x, y+1) diagonal; follow its
        // triangles rather than a bilinear patch, which can bulge above them.
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
        let h = if tx + ty <= 1.0 {
            a + (b - a) * tx + (c - a) * ty
        } else {
            d + (c - d) * (1.0 - tx) + (b - d) * (1.0 - ty)
        };
        Some((h, key.level))
    }
}

/// Fractal detail from `start_scale` (m) down to `finest_scale` (m). The octaves
/// are a fixed function of direction, so coarser LODs share their lower octaves.
/// Fractal detail below the resolution of the height data. `rough` scales it: 1 on
/// plains, up to 4 in high mountains, where the octaves also sharpen into ridges.
/// Each octave tilts the ground by about the same few degrees, so the amplitude stays small: SE's
/// ground between its data texels is smooth, and steeper plains would take SE's gravel and
/// blocks soils (laid by slope) instead of its sand.
fn detail_height(proc: &ProcSurface, dir: DVec3, start_scale: f64, finest_scale: f64, rough: f64) -> f64 {
    if proc.kind.is_giant() {
        return 0.0;
    }
    let ridged = ((rough - 1.0) / 2.0).clamp(0.0, 1.0);
    let mut scale = start_scale;
    let mut amp = start_scale * DETAIL_AMPLITUDE * rough;
    let mut h = 0.0;
    let mut octave = 0u64;
    while scale > finest_scale && octave < 14 {
        let p = dir * (proc.radius_m / scale);
        let n = se_core::noise::gradient(p, proc.seed ^ (0xD37A_11 + octave));
        h += amp * (n * (1.0 - ridged) + (0.35 - n.abs()) * ridged);
        scale *= 0.5;
        amp *= 0.48;
        octave += 1;
    }
    h
}

/// A normal map texel's rgb: the surface normal `n`'s departure from the smooth sphere's `up`
/// (body frame), each component square-root encoded (`sign(d) sqrt|d|`, -1..1 to 0..255). A smooth
/// body's map is then exactly flat, where whole normals in 8 bits quantised the sphere's own
/// curvature into a grid that showed near the terminator (04-014), and gentle slopes keep finer
/// steps than steep ones. planet.wesl `ground_normal` decodes it.
fn encode_normal_offset(n: DVec3, up: DVec3) -> [u8; 3] {
    encode_offset((n - up).as_vec3())
}

/// Half the offset is stored: its length is 2 sin(slope / 2), so whole offsets past 60 degrees of
/// slope would not fit the -1..1 range.
fn encode_offset(d: Vec3) -> [u8; 3] {
    (d * 0.5).to_array().map(|c| ((c.signum() * c.abs().sqrt() * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn decode_normal_offset(rgb: [u8; 3]) -> Vec3 {
    let e = Vec3::from_array(rgb.map(|c| c as f32 / 127.5 - 1.0));
    2.0 * e * e.abs()
}

/// A node's normal map (`n` x `n` RGBA8: normal offset, shore code) followed by its mip chain, each
/// level the 2x2 mean of the one above. Returns the data and the level count.
fn with_mips(level0: Vec<u8>, n: usize) -> (Vec<u8>, u32) {
    let mut all = level0;
    let (mut start, mut m, mut levels) = (0, n, 1);
    while m > 1 {
        let m2 = m / 2;
        let mut next = Vec::with_capacity(m2 * m2 * 4);
        for y in 0..m2 {
            for x in 0..m2 {
                let (mut off, mut a) = (Vec3::ZERO, 0.0);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let k = start + ((2 * y + dy).min(m - 1) * m + (2 * x + dx).min(m - 1)) * 4;
                    off += decode_normal_offset([all[k], all[k + 1], all[k + 2]]);
                    a += all[k + 3] as f32;
                }
                next.extend(encode_offset(off * 0.25));
                next.push((a / 4.0).round() as u8);
            }
        }
        start = all.len();
        all.extend_from_slice(&next);
        m = m2;
        levels += 1;
    }
    (all, levels)
}

/// Fallback albedo by SpaceEngine planet class.
fn class_color(class: &str) -> Vec3 {
    match class.to_ascii_lowercase().as_str() {
        "terra" => Vec3::new(0.45, 0.42, 0.36),
        "ferria" | "carbonia" => Vec3::new(0.42, 0.39, 0.36),
        "aquaria" => Vec3::new(0.35, 0.45, 0.6),
        "glacia" | "ice" => Vec3::new(0.85, 0.87, 0.9),
        "jupiter" | "gasgiant" => Vec3::new(0.8, 0.7, 0.55),
        "neptune" => Vec3::new(0.45, 0.6, 0.9),
        "selena" => Vec3::new(0.55, 0.53, 0.5),
        _ => Vec3::new(0.6, 0.57, 0.52),
    }
}

/// Decoded data shared between node builds.
#[derive(Clone)]
struct HeightTile {
    w: usize,
    h: usize,
    data: Vec<f32>,
    /// Texel position of the tile's u = 0 edge, and texels across it to u = 1.
    border: f64,
    span: f64,
}

impl HeightTile {
    /// Height at tile coordinates `u, v` in 0..1, Catmull-Rom between texels as SE's vertex shader
    /// reads magnified tiles (`interpolate`, planet.glsl), so no creases run along texel lines.
    /// SE's tiles carry a 2-texel border shared
    /// with their neighbours (each repeats the other's 4 edge texels; fitted against the parent
    /// tiles, texel `i` sits at `(i - 1.5) / 252`): the tile proper spans texels 2 to 254. Sampled
    /// over all 256, patches on either side of a tile edge disagreed by up to a texel, hundreds of
    /// metres on Mars's steep ground: straight cliffs along the tile edges.
    fn sample(&self, u: f64, v: f64) -> f64 {
        let x = (self.border + u * self.span).clamp(0.0, (self.w - 1) as f64);
        let y = (self.border + v * self.span).clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (x.floor() as isize, y.floor() as isize);
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let p = |xx: isize, yy: isize| {
            let (xx, yy) = (xx.clamp(0, self.w as isize - 1) as usize, yy.clamp(0, self.h as isize - 1) as usize);
            self.data[yy * self.w + xx] as f64
        };
        let row = |yy: isize| catmull_rom([p(x0 - 1, yy), p(x0, yy), p(x0 + 1, yy), p(x0 + 2, yy)], fx);
        catmull_rom([row(y0 - 1), row(y0), row(y0 + 1), row(y0 + 2)], fy)
    }
}

/// SE's `interpolate` (planet.glsl): Catmull-Rom through `r[1]` (t = 0) and `r[2]` (t = 1).
fn catmull_rom(r: [f64; 4], t: f64) -> f64 {
    let a = -0.5 * r[0] + 1.5 * r[1] - 1.5 * r[2] + 0.5 * r[3];
    let b = r[0] - 2.5 * r[1] + 2.0 * r[2] - 0.5 * r[3];
    let c = -0.5 * r[0] + 0.5 * r[2];
    ((a * t + b) * t + c) * t + r[1]
}

/// Caches shared by all terrains.
#[derive(Resource, Clone, Default)]
pub struct TileCaches {
    heights: Arc<Mutex<HashMap<String, Arc<HeightTile>>>>,
    textures: Arc<Mutex<HashMap<String, CachedTexture>>>,
}

impl TileCaches {
    /// The texture tiles kept: their count and size (MB), and the height tiles kept (perf.rs).
    pub fn sizes(&self) -> (usize, usize, usize) {
        let (n, bytes) = self.textures.lock().map_or((0, 0), |c| (c.len(), c.values().map(|t| t.bytes).sum::<usize>()));
        (n, bytes >> 20, self.heights.lock().map_or(0, |c| c.len()))
    }
}

/// A catalog planet's texture tile (SE's colour, lights, water and cloud tiles) kept for the nodes
/// that use it, with its size and the frame it was last handed to a node.
struct CachedTexture {
    handle: Handle<Image>,
    bytes: usize,
    used: u32,
}

/// The texture tiles kept beyond what the drawn nodes hold (MB). Over it the least recently used go
/// (their nodes keep their own handles; a node that needs one again decodes it anew). Kept without a
/// limit, every tile of every catalog planet visited stayed in video memory for the session (0.25 to
/// 1 MB each, decoded): the longer the user explored, the slower the game became (2026-11-04).
const TEXTURE_CACHE_MB: usize = 1024;
/// A tile handed to a node within this many frames is never dropped.
const TEXTURE_KEEP_FRAMES: u32 = 600;

#[derive(Resource)]
pub struct DefaultTextures {
    pub white: Handle<Image>,
    pub black: Handle<Image>,
    /// 1x1x1 3D texture standing in for absent scattering tables.
    pub black3d: Handle<Image>,
    /// SE's 64^3 noise texture ([`PlanetMaterial::noise`]).
    pub noise3d: Handle<Image>,
}

struct DecodedTexture {
    path: String,
    image: Image,
}

struct NodeBuild {
    /// The ground's grid, and its skirts apart (drawn after all the ground, [`PlanetMaterial::skirt`]).
    mesh: Mesh,
    skirt: Mesh,
    /// GRID x GRID heights above the reference radius (m), for ground queries.
    grid_heights: Vec<f32>,
    /// Sea surface patch (same centre as `mesh`), where the node reaches below sea level.
    water: Option<Mesh>,
    /// Cloud shell patches and their centres (body frame), with their layer index.
    cloud: Vec<(usize, Mesh, DVec3)>,
    /// Node centre in body frame (meters, relative to body centre).
    center: DVec3,
    textures: Vec<DecodedTexture>,
    normals: Image,
}

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PlanetMaterial>::default())
            .add_plugins(MaterialPlugin::<CloudMaterial>::default())
            .add_plugins(crate::brown_dwarf::BrownDwarfPlugin)
            .init_resource::<TileCaches>()
            .init_resource::<SelfEmission>()
            .init_resource::<WaterDisplay>()
            .add_systems(Startup, (make_default_textures, load_surface_settings))
            .add_systems(Update, (animate_water_display, upgrade_node_noise))
            .add_observer(forget_gen_jobs)
            .add_systems(
                PostUpdate,
                // After update_terrain, so the camera collides with this frame's nodes. Before Bevy
                // turns the projection into this frame's matrix: `keep_camera_above_ground` brings
                // the near plane in to the ground, and `sync_camera_transform` sets it each frame from
                // the reference sphere, kilometres below high ground (11 km on Eris, so the near
                // plane sat 11 m out and cut away the ground in front of a camera 1.5 m up).
                (update_eclipses, update_terrain, cull_sea_patches, keep_camera_above_ground, update_node_transforms)
                    .chain()
                    .after(UniverseSet::Camera)
                    .before(UniverseSet::FloatingOrigin)
                    .before(bevy::camera::CameraUpdateSystems),
            );
    }
}

/// A terrain going away (its system unloaded) forgets the grids still being made for it, which
/// would otherwise wait for it in the generator's results for the rest of the session.
fn forget_gen_jobs(ev: On<Remove<Terrain>>, terrains: Query<&Terrain>, tgen: Option<Res<crate::terrain_gen::TerrainGen>>) {
    if let (Ok(t), Some(tgen)) = (terrains.get(ev.entity), tgen) {
        tgen.forget(t.gen_jobs.values().map(|&(id, _)| id));
    }
}

fn solid_image(rgba: [u8; 4]) -> Image {
    Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba.to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn make_default_textures(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let black3d = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D3,
        vec![0, 0, 0, 0],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    let n = se_core::tg::NOISE3D_SIZE as u32;
    let noise3d = Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: n },
        TextureDimension::D3,
        se_core::tg::noise3d(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    commands.insert_resource(DefaultTextures {
        white: images.add(solid_image([255, 255, 255, 255])),
        black: images.add(solid_image([0, 0, 0, 255])),
        black3d: images.add(black3d),
        noise3d: images.add(noise3d),
    });
}

fn discover_layers(vfs: &Vfs, t: &Terrain) -> TerrainLayers {
    let s = &t.surface;
    let color = s.diff_map.as_deref().and_then(|m| TileLayer::discover(vfs, m, &["_c"]));
    let water = if s.diff_map_alpha_water {
        s.diff_map.as_deref().and_then(|m| TileLayer::discover(vfs, m, &["_a"])).filter(|l| l.suffix == "_a")
    } else {
        None
    };
    TerrainLayers {
        color,
        water,
        bump: s.bump_map.as_deref().and_then(|m| TileLayer::discover(vfs, m, &[])),
        lights: s.glow_map.as_deref().and_then(|m| TileLayer::discover(vfs, m, &[])),
        clouds: t.clouds.iter().map(|c| c.map.as_deref().and_then(|m| TileLayer::discover(vfs, m, &[]))).collect(),
    }
}

impl Terrain {
    /// The body's tile layers, once discovered.
    pub fn tile_layers(&self) -> Option<&Arc<TerrainLayers>> {
        self.layers.as_ref()
    }

    /// The cloud layers drawn as shells: (layer index, shell radius m). A layer is drawn with its
    /// map, or procedurally without one.
    fn drawn_clouds(&self, layers: &TerrainLayers) -> Vec<(usize, f64)> {
        self.clouds
            .iter()
            .enumerate()
            .filter(|(i, c)| if c.map.is_some() { layers.clouds[*i].is_some() } else { self.cloud_proc.is_some() })
            .map(|(i, c)| (i, self.radius + c.height))
            .collect()
    }

    /// The layer whose map shadows the ground: the first with a map that does not turn (the ground
    /// samples it in its own tile, so a turned map would cast its shadows in the wrong place).
    fn shadow_clouds(&self, layers: &TerrainLayers) -> Option<usize> {
        (0..self.clouds.len()).find(|&i| layers.clouds[i].is_some() && !self.clouds[i].flip && self.clouds[i].rad_per_s == 0.0)
    }
}

/// Decode an 8-bit tile into an sRGB (or linear) RGBA image.
fn decode_texture(bytes: &[u8], srgb: bool) -> Option<Image> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut image = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        img.into_raw(),
        if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    Some(image)
}

fn decode_height(bytes: &[u8]) -> Option<HeightTile> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let data: Vec<f32> = match img {
        image::DynamicImage::ImageLuma16(b) => b.into_raw().into_iter().map(|v| v as f32 / 65535.0).collect(),
        other => other.to_luma32f().into_raw(),
    };
    Some(HeightTile { w, h, data, border: 1.5, span: w as f64 - 4.0 })
}

struct BuildInput {
    key: TileKey,
    radius: f64,
    surface: SurfaceParams,
    layers: Arc<TerrainLayers>,
    vfs: Arc<Vfs>,
    caches: TileCaches,
    /// Texture tiles (path, srgb) that still need decoding.
    textures: Vec<(String, bool)>,
    /// The drawn cloud layers: (layer index, shell radius m).
    cloud_radii: Vec<(usize, f64)>,
    proc: ProcSurface,
    /// The body has an ocean: the mesh is clamped to sea level (SE's flat ocean surface).
    ocean: bool,
    /// SE's generator's grid for this node or its ancestor, and the height (SE's units) and
    /// metres per unit of the reference radius.
    generated: Option<(TileKey, Arc<HeightTile>, f64, f64)>,
}

fn build_node(input: BuildInput) -> NodeBuild {
    let started = std::time::Instant::now();
    let BuildInput {
        key,
        radius,
        surface,
        layers,
        vfs,
        caches,
        textures,
        cloud_radii,
        proc,
        ocean,
        generated,
    } = input;
    // Heights.
    let height = layers.bump.as_ref().and_then(|bl| {
        let anc = bl.source(key);
        let path = bl.tile_path(anc);
        let cached = caches.heights.lock().ok()?.get(&path).cloned();
        let tile = match cached {
            Some(t) => t,
            None => {
                let t = Arc::new(decode_height(&vfs.read(&path).ok()?)?);
                let mut c = caches.heights.lock().ok()?;
                if c.len() > 768 {
                    c.clear();
                }
                c.insert(path, t.clone());
                t
            }
        };
        Some((anc, tile))
    });
    let (u0, v0, size) = key.uv_rect();
    let center = key.center_dir() * radius;
    // Real bump data resolves features down to about one tile pixel.
    let data_scale = layers
        .bump
        .as_ref()
        .map(|bl| radius * std::f64::consts::FRAC_PI_2 / (1u64 << bl.max_level) as f64 / 256.0 * 2.0);
    // One surface for every level, as SE's generator has fixed octave counts: a node samples the
    // same heights as its parent, only more densely, so coasts and lakes stay put as nodes split.
    let finest = FINEST_FEATURE_M;
    // `u, v` may lie a sample past the face's edge (the outer ring for the normals, along `dir`).
    let h_at = |u: f64, v: f64, dir: DVec3| -> f64 {
        if let Some((gk, tile, h_ref, metres)) = &generated {
            // SE's own heights: below their finest level its tiles are only interpolated. Their
            // border (`GEN_BORDER`) holds the heights past the face's edge: clamped there, the outer
            // ring stood a step out at the edge's height and halved the slope along the cube's edges.
            let (gu, gv, gs) = gk.uv_rect();
            return (tile.sample((u - gu) / gs, (v - gv) / gs) - h_ref) * metres;
        }
        let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        match &height {
            Some((anc, tile)) => {
                let (au, av, asz) = anc.uv_rect();
                let base = surface.height_m(tile.sample((u - au) / asz, (v - av) / asz));
                let rough = 1.0 + (base / 2500.0).clamp(0.0, 3.0);
                let detail = detail_height(&proc, dir, data_scale.unwrap_or(4000.0), finest, rough);
                if ocean {
                    // The detail (hundreds of metres) would raise shallow seas into islands and sink
                    // coastal plains into ponds: it fades out towards sea level, and never moves
                    // ground across it, so the coast stays where the data has it.
                    let k = (base.abs() / 300.0).min(1.0);
                    let h = base + detail * k * k;
                    if base < 0.0 { h.min(base * 0.25) } else { h.max(base * 0.25) }
                } else {
                    base + detail
                }
            }
            None if proc.kind.is_giant() => 0.0,
            None => {
                let base_scale = (radius / 1500.0).max(200.0);
                proc.height_m_lod(dir, finest) + detail_height(&proc, dir, base_scale, finest, 1.0)
            }
        }
    };
    // Position and true height. The ground keeps the seabed; an ocean's surface is a separate
    // mesh at sea level (SE's ocean sphere), below.
    let pos_at = |i: f64, j: f64| -> (DVec3, f64) {
        let u = u0 + size * i / (GRID - 1) as f64;
        let v = v0 + size * j / (GRID - 1) as f64;
        let dir = key.face.uv_to_dir(u, v);
        let h = h_at(u, v, dir);
        (dir * (radius + h), h)
    };
    let mut dirs: Vec<[f32; 3]> = Vec::with_capacity(GRID * GRID + 4 * GRID);
    let mut heights: Vec<f32> = Vec::with_capacity(GRID * GRID + 4 * GRID);
    // Extended grid for normals.
    let n_ext = GRID + 2;
    let mut ext = Vec::with_capacity(n_ext * n_ext);
    for j in 0..n_ext {
        for i in 0..n_ext {
            ext.push(pos_at(i as f64 - 1.0, j as f64 - 1.0));
        }
    }
    let at = |i: usize, j: usize| ext[(j + 1) * n_ext + (i + 1)].0;
    let edge_m = radius * std::f64::consts::FRAC_PI_2 * size;
    let skirt = edge_m * 0.03 + 20.0;
    let mut positions = Vec::with_capacity(GRID * GRID + 4 * GRID);
    let mut normals = Vec::with_capacity(GRID * GRID + 4 * GRID);
    let mut uvs = Vec::with_capacity(GRID * GRID + 4 * GRID);
    for j in 0..GRID {
        for i in 0..GRID {
            let p = at(i, j);
            let dx = ext[(j + 1) * n_ext + (i + 2)].0 - ext[(j + 1) * n_ext + i].0;
            let dy = ext[(j + 2) * n_ext + (i + 1)].0 - ext[j * n_ext + (i + 1)].0;
            let mut n = dy.cross(dx).normalize_or(p.normalize());
            if n.dot(p) < 0.0 {
                n = -n;
            }
            positions.push((p - center).as_vec3().to_array());
            normals.push(n.as_vec3().to_array());
            uvs.push([i as f32 / (GRID - 1) as f32, j as f32 / (GRID - 1) as f32]);
            heights.push(ext[(j + 1) * n_ext + (i + 1)].1 as f32);
            dirs.push(p.normalize().as_vec3().to_array());
        }
    }
    // The parent's surface at each vertex: its own vertices (every other one of ours)
    // coarser detail) and, between them, the midpoints of its straight edges and diagonals.
    let nc = (GRID - 1) / 2 + 3;
    let coarse: Vec<(DVec3, f64)> = if key.level == 0 {
        Vec::new()
    } else {
        (0..nc * nc)
            .map(|k| {
                let (i, j) = (2.0 * (k % nc) as f64 - 2.0, 2.0 * (k / nc) as f64 - 2.0);
                let u = u0 + size * i / (GRID - 1) as f64;
                let v = v0 + size * j / (GRID - 1) as f64;
                let dir = key.face.uv_to_dir(u, v);
                let h = h_at(u, v, dir);
                (dir * (radius + h), h)
            })
            .collect()
    };
    let parent_at = |i: usize, j: usize| -> (DVec3, f64) {
        let c = |a: usize, b: usize| coarse[(b + 1) * nc + (a + 1)];
        let mid = |x: (DVec3, f64), y: (DVec3, f64)| ((x.0 + y.0) * 0.5, (x.1 + y.1) * 0.5);
        let (a, b) = (i / 2, j / 2);
        match (i % 2, j % 2) {
            (0, 0) => c(a, b),
            (1, 0) => mid(c(a, b), c(a + 1, b)),
            (0, 1) => mid(c(a, b), c(a, b + 1)),
            // The parent splits its cells along the (x+1, y)-(x, y+1) diagonal.
            _ => mid(c(a + 1, b), c(a, b + 1)),
        }
    };
    let mut morph: Vec<[f32; 4]> = Vec::with_capacity(GRID * GRID + 4 * GRID);
    for j in 0..GRID {
        for i in 0..GRID {
            if coarse.is_empty() {
                morph.push([0.0; 4]);
            } else {
                let (pc, hc) = parent_at(i, j);
                let (p, h) = ext[(j + 1) * n_ext + (i + 1)];
                morph.push([(pc.x - p.x) as f32, (pc.y - p.y) as f32, (pc.z - p.z) as f32, (hc - h) as f32]);
            }
        }
    }
    // Normal map: the surface at four times the mesh's resolution (SE: 256 texels on 33 vertices),
    // so relief finer than the mesh still shades from afar and a split changes little.
    // Alpha holds the height near sea level (`SHORE_CODE_M`), so the shader finds the coast per
    // pixel, as SE's `step(height, seaLevel)` on its height tile does, not along mesh triangles.
    let normals_img = {
        let n = NORMAL_RES;
        let ne = n + 2;
        let pts: Vec<(DVec3, f64)> = (0..ne * ne)
            .map(|k| {
                let (i, j) = ((k % ne) as f64 - 1.0, (k / ne) as f64 - 1.0);
                let u = u0 + size * i / (n - 1) as f64;
                let v = v0 + size * j / (n - 1) as f64;
                let dir = key.face.uv_to_dir(u, v);
                let h = h_at(u, v, dir);
                (dir * (radius + h), h)
            })
            .collect();
        let mut data = Vec::with_capacity(n * n * 4);
        for j in 0..n {
            for i in 0..n {
                let (p, h) = pts[(j + 1) * ne + i + 1];
                let dx = pts[(j + 1) * ne + i + 2].0 - pts[(j + 1) * ne + i].0;
                let dy = pts[(j + 2) * ne + i + 1].0 - pts[j * ne + i + 1].0;
                let mut nn = dy.cross(dx).normalize_or(p.normalize());
                if nn.dot(p) < 0.0 {
                    nn = -nn;
                }
                data.extend(encode_normal_offset(nn, p.normalize()));
                data.push(((0.5 + 0.5 * (h / SHORE_CODE_M).tanh()) * 255.0).round() as u8);
            }
        }
        let (all, levels) = with_mips(data, n);
        let mut image = Image::new(
            Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            all[..n * n * 4].to_vec(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.data = Some(all);
        image.texture_descriptor.mip_level_count = levels;
        // Mipmapped and anisotropic, as SE's tiles (`MipmapsLand`, `AnisotropyLevel 16`).
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 16,
            ..default()
        });
        image
    };
    let mut indices: Vec<u32> = Vec::with_capacity((GRID - 1) * (GRID - 1) * 6 + (GRID - 1) * 24);
    let idx = |i: usize, j: usize| (j * GRID + i) as u32;
    for j in 0..GRID - 1 {
        for i in 0..GRID - 1 {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i, j + 1), idx(i + 1, j + 1));
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    // Skirts along the four edges (walk each edge, duplicate vertices pushed down).
    let edges: [Vec<(usize, usize)>; 4] = [
        (0..GRID).map(|i| (i, 0)).collect(),
        (0..GRID).map(|j| (GRID - 1, j)).collect(),
        (0..GRID).rev().map(|i| (i, GRID - 1)).collect(),
        (0..GRID).rev().map(|j| (0, j)).collect(),
    ];
    for edge in &edges {
        let base = positions.len() as u32;
        for &(i, j) in edge {
            let p = at(i, j);
            let down = p - p.normalize() * skirt;
            positions.push((down - center).as_vec3().to_array());
            normals.push(normals[(j * GRID + i) as usize]);
            uvs.push(uvs[(j * GRID + i) as usize]);
            heights.push(heights[(j * GRID + i) as usize]);
            dirs.push(dirs[(j * GRID + i) as usize]);
            morph.push(morph[j * GRID + i]);
        }
        for k in 0..edge.len() - 1 {
            let (i0, j0) = edge[k];
            let (i1, j1) = edge[k + 1];
            let (t0, t1) = (idx(i0, j0), idx(i1, j1));
            let (s0, s1) = (base + k as u32, base + k as u32 + 1);
            // Edges walk clockwise around the patch as seen from outside, so the
            // skirt faces outward with this winding; both sides are cheap anyway.
            indices.extend_from_slice(&[t0, t1, s0, s0, t1, s1, t0, s0, t1, s0, s1, t1]);
        }
    }
    let grid_heights = heights[..GRID * GRID].to_vec();
    // The sea surface over this node, where any of it lies below sea level: the grid moved to
    // the sphere, keeping the seabed height for the depth. Instead of skirts (walls that showed
    // from just under the surface) each edge gets a flange: one more row of the grid, continued
    // along the sphere over the neighbour and a hair below it, which covers the crack between
    // a coarser neighbour's straight edge and the sphere from above and below.
    let water = (ocean && grid_heights.iter().any(|&h| h < 0.0)).then(|| {
        let seg = edge_m / (GRID - 1) as f64;
        let drop = seg * 1e-4;
        let sea = |k: usize| DVec3::from_array(dirs[k].map(f64::from)).normalize();
        let mut wp: Vec<[f32; 3]> = (0..GRID * GRID).map(|k| (sea(k) * radius - center).as_vec3().to_array()).collect();
        let mut wn: Vec<[f32; 3]> = dirs[..GRID * GRID].to_vec();
        let mut wu: Vec<[f32; 2]> = uvs[..GRID * GRID].to_vec();
        let mut wh: Vec<f32> = grid_heights.clone();
        let mut wd: Vec<[f32; 3]> = dirs[..GRID * GRID].to_vec();
        let mut wi: Vec<u32> = indices[..(GRID - 1) * (GRID - 1) * 6].to_vec();
        for (e, edge) in edges.iter().enumerate() {
            let base = wp.len() as u32;
            for &(i, j) in edge {
                let k = j * GRID + i;
                // The grid point one row inside, mirrored through the edge.
                let k_in = match e {
                    0 => GRID + i,
                    1 => j * GRID + GRID - 2,
                    2 => (GRID - 2) * GRID + i,
                    _ => j * GRID + 1,
                };
                let out = (sea(k) * 2.0 - sea(k_in)).normalize();
                wp.push((out * (radius - drop) - center).as_vec3().to_array());
                wn.push(out.as_vec3().to_array());
                wu.push([wu[k][0] * 2.0 - wu[k_in][0], wu[k][1] * 2.0 - wu[k_in][1]]);
                wh.push(wh[k] * 2.0 - wh[k_in]);
                wd.push(out.as_vec3().to_array());
            }
            for k in 0..edge.len() - 1 {
                let (t0, t1) = (idx(edge[k].0, edge[k].1), idx(edge[k + 1].0, edge[k + 1].1));
                let (s0, s1) = (base + k as u32, base + k as u32 + 1);
                wi.extend_from_slice(&[t0, t1, s0, s0, t1, s1]);
            }
        }
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, wp)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, wn)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, wu)
            .with_inserted_attribute(ATTRIBUTE_HEIGHT, wh)
            .with_inserted_attribute(ATTRIBUTE_MORPH, vec![[0.0f32; 4]; wd.len()])
            .with_inserted_attribute(ATTRIBUTE_BODY_DIR, wd)
            .with_inserted_indices(Indices::U32(wi))
    });
    // The skirts in a mesh of their own: their triangles and the vertices they use (the grid's edge
    // rows and the copies pushed down), renumbered; the grid keeps the rest.
    let n_tri = (GRID - 1) * (GRID - 1) * 6;
    let mut remap = vec![u32::MAX; positions.len()];
    let mut keep: Vec<usize> = Vec::with_capacity(8 * GRID);
    let skirt_indices: Vec<u32> = indices[n_tri..]
        .iter()
        .map(|&v| {
            let r = &mut remap[v as usize];
            if *r == u32::MAX {
                *r = keep.len() as u32;
                keep.push(v as usize);
            }
            *r
        })
        .collect();
    fn pick<T: Copy>(a: &[T], keep: &[usize]) -> Vec<T> {
        keep.iter().map(|&k| a[k]).collect()
    }
    let skirt = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pick(&positions, &keep))
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, pick(&normals, &keep))
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, pick(&uvs, &keep))
        .with_inserted_attribute(ATTRIBUTE_HEIGHT, pick(&heights, &keep))
        .with_inserted_attribute(ATTRIBUTE_BODY_DIR, pick(&dirs, &keep))
        .with_inserted_attribute(ATTRIBUTE_MORPH, pick(&morph, &keep))
        .with_inserted_indices(Indices::U32(skirt_indices));
    positions.truncate(GRID * GRID);
    normals.truncate(GRID * GRID);
    uvs.truncate(GRID * GRID);
    heights.truncate(GRID * GRID);
    dirs.truncate(GRID * GRID);
    morph.truncate(GRID * GRID);
    indices.truncate(n_tri);
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_attribute(ATTRIBUTE_HEIGHT, heights)
        .with_inserted_attribute(ATTRIBUTE_BODY_DIR, dirs)
        .with_inserted_attribute(ATTRIBUTE_MORPH, morph)
        .with_inserted_indices(Indices::U32(indices));
    let cloud = cloud_radii.into_iter().map(|(layer, rc)| {
        // Lifted by its cells' sag (cell^2 / 8 r): the flat cells of a coarse node otherwise dip
        // below the cloud height, on a giant down through the ground (5 km for a level-1 node of
        // Jupiter, shown while the finer ones load).
        let cell = rc * std::f64::consts::FRAC_PI_2 * size / (GRID - 1) as f64;
        let rc = rc + cell * cell / (8.0 * rc);
        let c_center = key.center_dir() * rc;
        let (mut cp, mut cn, mut cu) = (Vec::new(), Vec::new(), Vec::new());
        for j in 0..GRID {
            for i in 0..GRID {
                let u = u0 + size * i as f64 / (GRID - 1) as f64;
                let v = v0 + size * j as f64 / (GRID - 1) as f64;
                let dir = key.face.uv_to_dir(u, v);
                cp.push((dir * rc - c_center).as_vec3().to_array());
                cn.push(dir.as_vec3().to_array());
                cu.push([i as f32 / (GRID - 1) as f32, j as f32 / (GRID - 1) as f32]);
            }
        }
        let mut ci: Vec<u32> = Vec::with_capacity((GRID - 1) * (GRID - 1) * 6);
        for j in 0..GRID - 1 {
            for i in 0..GRID - 1 {
                let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i, j + 1), idx(i + 1, j + 1));
                ci.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        let m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, cp)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, cn)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, cu)
            .with_inserted_indices(Indices::U32(ci));
        (layer, m, c_center)
    }).collect();
    let mut decoded = Vec::new();
    for (path, srgb) in textures {
        if let Ok(bytes) = vfs.read(&path) {
            if let Some(image) = decode_texture(&bytes, srgb) {
                decoded.push(DecodedTexture { path, image });
            }
        }
    }
    debug!("node {key:?} built in {:.1} ms", started.elapsed().as_secs_f64() * 1e3);
    NodeBuild {
        mesh,
        skirt,
        grid_heights,
        water,
        cloud,
        center,
        textures: decoded,
        normals: normals_img,
    }
}

/// A node's ground patch (its sea surface, if any, is a child entity). Its materials are never
/// changed after the build: the lighting is in the node's [`PatchLight`].
#[derive(Component)]
pub struct TerrainNode;

#[derive(Component)]
pub struct CloudNode {
    /// Index into `Terrain::clouds`.
    pub layer: usize,
}

/// A patch rigidly attached to a body: centre in body frame (meters).
#[derive(Component)]
pub struct BodyAttached {
    pub body: Entity,
    pub center: DVec3,
}

/// A node's sea surface (SE's 3D water mesh, water.glsl), a child of its ground node; `extent_m`
/// bounds the node's reach from its centre.
#[derive(Component)]
pub struct SeaPatch {
    extent_m: f64,
}

/// SE's WaterVisibilityHeight (config variable, default 10 km; not in main-def.cfg).
const WATER_VISIBILITY_HEIGHT_M: f64 = 10_000.0;

/// How far from the camera SE draws its 3D water, 2 / WaterParams.z (FUN_1404acdd0): 200 km from sea
/// level down to 2 km at WaterVisibilityHeight and above; past it the ground pass draws a flat sea.
/// `None` (everywhere) from under water (1 / (2 R)) and on Oceanic and Superoceanic worlds
/// (volatiles class 4, 5: 0). `alt_m` is the camera's height above sea level.
pub fn water_reach_m(alt_m: f64, volq: i32) -> Option<f64> {
    if volq == 4 || volq == 5 || alt_m <= 0.0 {
        return None;
    }
    let t = (alt_m / WATER_VISIBILITY_HEIGHT_M).min(1.0);
    Some(2.0 * (100.0 - 99.0 * t) * 1000.0)
}

/// Hides the sea patches SE would not draw (beyond [`water_reach_m`]); their water is the ground
/// pass's flat sea there.
fn cull_sea_patches(
    cam: Res<FlyCamera>,
    bodies: Query<(&SystemBody, &Terrain)>,
    nodes: Query<&BodyAttached, Allow<Disabled>>,
    mut seas: Query<(&SeaPatch, &ChildOf, &mut Visibility), Allow<Disabled>>,
) {
    for (sea, parent, mut vis) in &mut seas {
        let Ok(node) = nodes.get(parent.parent()) else { continue };
        let Ok((body, terrain)) = bodies.get(node.body) else { continue };
        let cam_body = body.from_shape(body.orient.inverse() * cam.pos.delta_m(body.pos));
        let reach = water_reach_m(cam_body.length() - terrain.radius, terrain.surf_gen.classes.volq);
        let near = reach.is_none_or(|r| (cam_body - node.center).length() - sea.extent_m < r);
        vis.set_if_neq(if near { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// Layer rect for a node: (ancestor key, uniform rect).
fn layer_rect(layer: Option<&TileLayer>, key: TileKey) -> (Option<String>, Vec4) {
    match layer {
        Some(l) => {
            let anc = l.source(key);
            let (u, v, s) = key.rect_in(anc);
            (Some(l.tile_path(anc)), Vec4::new(u as f32, v as f32, s as f32, 1.0))
        }
        None => (None, Vec4::ZERO),
    }
}

/// Height range (m above the reference radius) of a node's ground, or of its nearest loaded
/// ancestor's (a node not built yet lies about within its parent's range).
/// SE's generated height (0..1 of `BumpHeight`) at the reference radius: `BumpOffset / BumpHeight`,
/// where `BumpOffset` is the ocean's height, and 0 without one (FUN_1405057e0 sets it; the renderer,
/// FUN_1404ae4a0, puts height 0 at `R - BumpOffset`). So the ground of a dry world or an asteroid
/// rises from the radius, and sea level is the radius of an ocean world.
fn gen_ref(proc: &ProcSurface) -> f64 {
    match &proc.se {
        Some(g) if proc.ocean_fraction > 0.0 => g.v(se_core::surface_gen::SEA_LEVEL) as f64,
        _ => 0.0,
    }
}

/// Deepest level SE's generator is asked for: grid steps of at least 2 m (its point is an f32 unit
/// vector, good to about half a metre on an Earth-sized world). Deeper nodes interpolate it.
fn gen_max_level(radius: f64) -> u8 {
    let steps = radius * std::f64::consts::FRAC_PI_2 / (crate::terrain_gen::GEN_SPAN * 2.0);
    (steps.max(1.0).log2().floor() as u8).min(MAX_GEOM_LEVEL)
}

/// The grid of node `key` for SE's generator `g`: `GEN_N` samples a side, `GEN_BORDER` beyond the
/// node's edges, as unit-cube points (the port's cube is SE's: its tiles load on it).
fn gen_params(key: TileKey, se: &[[f32; 4]; 20], g: u32) -> crate::terrain_gen::TgParams {
    let (u0, v0, size) = key.uv_rect();
    let step = size / crate::terrain_gen::GEN_SPAN;
    let b = crate::terrain_gen::GEN_BORDER * step;
    let c = |u: f64, v: f64| key.face.uv_to_cube(u, v);
    let origin = c(u0 - b, v0 - b);
    let du = c(u0 + step, v0) - c(u0, v0);
    let dv = c(u0, v0 + step) - c(u0, v0);
    crate::terrain_gen::TgParams::new(
        se,
        origin.as_vec3().extend(crate::terrain_gen::GEN_N as f32),
        du.as_vec3().extend(0.0),
        dv.as_vec3().extend(g as f32),
    )
}

fn height_range(t: &Terrain, key: TileKey) -> (f64, f64) {
    let mut k = Some(key);
    while let Some(c) = k {
        if let Some(&(lo, hi)) = t.node_hrange.get(&c) {
            return (lo as f64, hi as f64);
        }
        k = c.parent();
    }
    (0.0, 0.0)
}

/// `scale` is the graphics quality's factor (`graphics::TerrainQuality`, SE's `LandLOD`);
/// `hrange` the node's ground heights ([`height_range`]); `dir` its centre direction.
fn wants_split(key: TileKey, dir: DVec3, radius: f64, cam_body: DVec3, scale: f64, hrange: (f64, f64)) -> bool {
    if key.level >= MAX_GEOM_LEVEL {
        return false;
    }
    let edge = radius * std::f64::consts::FRAC_PI_2 / (1u64 << key.level) as f64;
    // The patch at the height of its ground nearest the camera's: measured on the reference sphere,
    // high ground (Mars's highlands, 500 m up at 0 N 82 E) left the camera hundreds of metres from
    // every patch, and the ones under it stopped splitting at a few hundred metres, a blurry half
    // of the view beside sharp ones.
    let h = (cam_body.length() - radius).clamp(hrange.0, hrange.1.max(hrange.0));
    let center = dir * (radius + h);
    // Distance to the patch, approximated by distance to centre minus its half diagonal.
    let d = (cam_body - center).length() - edge * 0.7;
    d < edge * SPLIT_DISTANCE * scale
}

/// A node material's `soil_tile.w`: the split distance in node edges at quality `scale`, for the
/// geomorph (0 on the roots, which do not morph).
pub(crate) fn node_split(level: u8, scale: f64) -> f32 {
    if level == 0 { 0.0 } else { (SPLIT_DISTANCE * scale) as f32 }
}

/// A node's vertex positions as `build_node` makes them (`dir * (radius + h) - centre`, worked
/// out in f64, body frame on the sphere) with the height in w, from its height grid.
pub(crate) fn node_vertices(key: TileKey, radius: f64, grid: &[f32]) -> Vec<[f32; 4]> {
    let (u0, v0, size) = key.uv_rect();
    let center = key.center_dir() * radius;
    let mut out = Vec::with_capacity(GRID * GRID);
    for j in 0..GRID {
        for i in 0..GRID {
            let u = u0 + size * i as f64 / (GRID - 1) as f64;
            let v = v0 + size * j as f64 / (GRID - 1) as f64;
            let h = grid[j * GRID + i] as f64;
            let p = key.face.uv_to_dir(u, v) * (radius + h) - center;
            out.push([p.x as f32, p.y as f32, p.z as f32, h as f32]);
        }
    }
    out
}

/// Whether node `key` lies wholly past the horizon: ground no lower than `floor` (radius) hides
/// everything up to `top` beyond it. Measured from the lowest ground, the test holds down to the
/// surface; it used to be dropped within 0.01 % of the reference radius (638 m on Earth), and
/// every node on the far side became wanted at once, the terrain falling back to coarse nodes
/// until they loaded and the camera, pushed up by the coarse ground, flickering across the line.
/// `gap` is the node's [`Horizon::gap`].
fn beyond_horizon(gap: f64) -> bool {
    gap > 0.0
}

/// The camera's horizon for [`select`]'s walk, worked out once (it depends only on the camera and
/// the terrain's radii, which the walk does not change).
struct Horizon {
    /// The camera's direction from the centre, `None` at the centre (everything may show).
    cam_dir: Option<DVec3>,
    /// Arc from below the camera to the horizon of the lowest ground, plus on to where the
    /// highest ground (or cloud top) rises over it.
    arc: f64,
}

impl Horizon {
    fn new(cam_body: DVec3, top: f64, floor: f64) -> Self {
        let rc = cam_body.length();
        if rc <= 0.0 {
            return Self { cam_dir: None, arc: 0.0 };
        }
        let r0 = floor.min(rc);
        let arc = (r0 / rc).clamp(-1.0, 1.0).acos() + (r0 / top.max(r0)).clamp(-1.0, 1.0).acos();
        Self { cam_dir: Some(cam_body.normalize()), arc }
    }

    /// How far node `key` (centre direction `dir`) lies past the horizon (radians of arc;
    /// negative when some of it may show).
    fn gap(&self, key: TileKey, dir: DVec3) -> f64 {
        let Some(cam_dir) = self.cam_dir else { return -1.0 };
        let ang = cam_dir.dot(dir).clamp(-1.0, 1.0).acos();
        ang - (self.arc + node_angle(key.level) + 0.01)
    }
}

/// Angular radius (generous) of a node at `level`.
fn node_angle(level: u8) -> f64 {
    std::f64::consts::FRAC_PI_4 * 1.5 / (1u64 << level) as f64
}

#[allow(clippy::too_many_arguments)]
fn update_terrain(
    mut commands: Commands,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    caches: Res<TileCaches>,
    defaults: Option<Res<DefaultTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
    mut cloud_mats: ResMut<Assets<CloudMaterial>>,
    mut bodies: Query<(Entity, &SystemBody, &BodyLighting, &mut Terrain, Option<&crate::eclipse::EclipseCasters>)>,
    (ambient, emission, water_display, switches, volumetric, cloud_assets, pipes): (
        Res<crate::system::AmbientLighting>,
        Res<SelfEmission>,
        Res<WaterDisplay>,
        Res<crate::switches::RenderSwitches>,
        Res<crate::volumetric_clouds::VolumetricClouds>,
        Option<Res<crate::volumetric_clouds::CloudShadowAssets>>,
        Res<crate::pipeline_watch::GroundPipelines>,
    ),
    clouds_q: Query<&CloudNode, Allow<Disabled>>,
    mut vis_q: Query<(&mut Visibility, Has<Disabled>), (Or<(With<TerrainNode>, With<CloudNode>)>, Allow<Disabled>)>,
    rings: Query<&crate::rings::Rings>,
    (ring_mats, surface_tex, quality, mut tgen, mut gen_queue, mut shader_buffers, time, memory, mut tick, children): (
        Res<Assets<crate::rings::RingMaterial>>,
        Option<Res<SurfaceTextures>>,
        Res<crate::graphics::TerrainQuality>,
        ResMut<crate::terrain_gen::TerrainGen>,
        ResMut<crate::terrain_gen::GenQueue>,
        ResMut<Assets<bevy::render::storage::ShaderBuffer>>,
        Res<crate::universe::SimTime>,
        Res<crate::memory::MemoryLimits>,
        Local<u32>,
        Query<&Children, Allow<Disabled>>,
    ),
) {
    let (Some(defaults), Some(surface_tex), Some(cloud_assets)) = (defaults, surface_tex, cloud_assets) else { return };
    let mut in_flight: usize = bodies.iter().map(|(_, _, _, t, _)| t.loading.len()).sum();
    // Tiles started this frame, over all bodies (SE's `MaxTilesPerFrame`).
    let mut started = 0usize;
    for (body_entity, body, light, mut terrain, casters) in &mut bodies {
        // SE's EclipseCasters per light (eclipse.rs), none with the eclipses switch off.
        terrain.casters = casters.filter(|_| switches.eclipses).map_or_else(Default::default, |c| c.lights);
        let terrain = &mut *terrain;
        if !terrain.biome_chosen {
            terrain.biome_chosen = true;
            let giant = terrain.proc.kind.is_giant() || terrain.surf_gen.classes.bulk >= se_core::surface_gen::bulk::NEPTUNE;
            if let ObjectRef::Body(i) = body.obj {
                let cat = &data.catalog;
                let mut info = crate::surface_tex::body_info(
                    cat,
                    &cat.bodies[i as usize],
                    terrain.proc.temperature_k,
                    terrain.atmosphere.is_some(),
                    terrain.has_ocean,
                );
                // SE's choice: generated SurfStyle, SE classes, SE's star temperature.
                info.surf_style = terrain.surf_gen.surf_style() as f64;
                info.se = Some(terrain.surf_gen.classes);
                info.sun_temp = Some(se_core::surface_gen::preset_sun_temp(cat, i as usize));
                debug!(
                    "surface of {}: preset {:?}, SurfStyle {:.4}, classes {:?}",
                    info.name,
                    surface_tex.preset_file(&info),
                    info.surf_style,
                    terrain.surf_gen.classes
                );
                terrain.biome = if giant { surface_tex.giant_uniform(&info, &terrain.surf_gen) } else { surface_tex.biome_uniform(&info) };
                let (deep, fog) = surface_tex.ocean_palette(&info);
                terrain.water_color = Vec3::from_array(deep);
                terrain.water_fog = Vec4::from_array(fog);
                if let (Some(u), true, false) = (terrain.biome.as_mut(), terrain.proc.se.is_some(), giant) {
                    u.climate = crate::surface_tex::climate_uniform(&terrain.surf_gen, terrain.proc.bump_m, terrain.proc.ocean_fraction > 0.0, terrain.tidal_lock);
                }
            }
        }
        terrain.frame += 1;
        let frame = terrain.frame;
        let resolved = body.px_radius > 0.5;
        // The body's rings, for their shadow on the surface.
        let ring = rings
            .iter()
            .find(|r| r.body == body_entity)
            .and_then(|r| ring_mats.get(&r.material))
            .map(|m| (m.texture.clone(), m.u.geom.truncate().extend(if m.u.params.z > 0.5 { 1.0 } else { 0.5 })));
        // SE's rings winter: `RingsWinter` x MeanRadius / R (the ring's inner and outer radius here
        // are in planet radii; MeanRadius is taken as their middle).
        let winter = match (&ring, terrain.surface.rings_winter) {
            (Some((_, g)), Some(w)) => (w * 0.5 * (g.x + g.y) as f64) as f32,
            _ => 0.0,
        };
        let ring_normal = (body.orient * DVec3::Y).as_vec3().extend(winter);
        // Poll finished builds.
        let shadow = terrain.layers.as_ref().and_then(|l| terrain.shadow_clouds(l));
        let thermal = thermal_uniform(terrain, light);
        let keys = terrain.loading.clone();
        let glow_atmo_u = terrain.glow_atmo(emission.scale()).extend(0.0);
        let setup_start = std::time::Instant::now();
        for key in keys {
            if quality.time_budget_ms.is_some_and(|ms| setup_start.elapsed().as_secs_f32() * 1000.0 >= ms) {
                break;
            }
            let slot = terrain.nodes.get_mut(&key).unwrap();
            let NodeState::Loading(task) = &mut slot.state else { continue };
            let Some(build) = check_ready(task) else { continue };
            terrain.loading.retain(|k| *k != key);
            in_flight = in_flight.saturating_sub(1);
            let layers = terrain.layers.clone().unwrap();
            let mut tex_cache = caches.textures.lock().unwrap();
            for t in build.textures {
                if !tex_cache.contains_key(&t.path) {
                    let bytes = t.image.data.as_ref().map_or(0, |d| d.len());
                    let handle = images.add(t.image);
                    tex_cache.insert(t.path, CachedTexture { handle, bytes, used: *tick });
                }
            }
            let now = *tick;
            let mut tex = |path: &Option<String>, fallback: &Handle<Image>| {
                path.as_ref()
                    .and_then(|p| tex_cache.get_mut(p))
                    .map(|c| {
                        c.used = now;
                        c.handle.clone()
                    })
                    .unwrap_or_else(|| fallback.clone())
            };
            let (pc, rc) = layer_rect(layers.color.as_ref(), key);
            let (pl, rl) = layer_rect(layers.lights.as_ref(), key);
            let (pw, rw) = layer_rect(layers.water.as_ref(), key);
            let (pcl, rcl) = layer_rect(shadow.and_then(|i| layers.clouds[i].as_ref()), key);
            let s = &terrain.surface;
            let (u0n, v0n, sizen) = key.uv_rect();
            let (extra_dir, extra_color) = light.extra_uniforms(false);
            let light_slot = terrain.lights.alloc(&mut shader_buffers);
            let (light_page, page_slot) = terrain.lights.page(light_slot);
            let u = PlanetUniform {
                planet: (-build.center).as_vec3().extend(terrain.radius as f32),
                rect_color: rc,
                rect_lights: rl,
                rect_water: rw,
                rect_clouds: rcl,
                base_color: terrain.base_color.extend(ambient.0),
                params: Vec4::new(
                    s.glow_bright as f32 * 1.5 * emission.scale(),
                    terrain.water_spec.0,
                    shadow.map_or(0.0, |i| terrain.clouds[i].opacity.min(1.0)),
                    terrain.water_spec.1,
                ),
                // SE's WaterSurfColor: ModulateColor x the preset's Deep (FUN_1404ae4a0).
                ocean_sea: (terrain.water_color * terrain.surf_modulate.truncate())
                    .extend(if terrain.has_ocean || layers.water.is_some() { 1.0 } else { 0.0 }),
                water_fog: terrain.water_fog,
                atm_flags: Vec4::new(
                    if terrain.atmosphere.is_some() && switches.atmospheres { 1.0 } else { 0.0 },
                    ATM_SUN_SCALE,
                    scatter_rel(terrain.atm_bright),
                    terrain.day_ambient,
                ),
                proc_pal0: Vec3::from_array(terrain.proc.palette[0]).extend(1.0),
                proc_pal1: Vec3::from_array(terrain.proc.palette[1]).extend(1.0),
                proc_pal2: Vec3::from_array(terrain.proc.palette[2]).extend(1.0),
                proc_pal3: Vec3::from_array(terrain.proc.palette[3]).extend(1.0),
                proc_params: Vec4::new(
                    terrain.proc.kind.id(),
                    terrain.proc.relief_m as f32,
                    terrain.proc.temperature_k as f32,
                    if terrain.proc.has_life { 1.0 } else { 0.0 },
                ),
                proc_params2: Vec4::new(
                    terrain.radius as f32,
                    (terrain.proc.seed % 10_000) as f32 / 10_000.0,
                    if layers.color.is_none() { 1.0 } else { 0.0 },
                    terrain.surf_gen.classes.volq as f32,
                ),
                atm: terrain
                    .atmosphere
                    .as_ref()
                    .map(|a| AtmosphereParamsUniform { look: terrain.atm_look, ..a.uniform() })
                    .unwrap_or_else(|| (&se_core::atm::AtmParams {
                        radius_ground: 6360.0,
                        radius_top: 6420.0,
                        rayleigh_h: 8.0,
                        mie_h: 1.2,
                        rayleigh_beta: [0.0; 3],
                        mie_sca_beta: [0.0; 3],
                        mie_ext_beta: [0.0; 3],
                        mie_g: 0.8,
                        mie_fade: 0.1,
                    }).into()),
                ring: ring.as_ref().map_or(Vec4::ZERO, |r| r.1),
                ring_normal,
                soil_tile: {
                    let edge = terrain.radius * std::f64::consts::FRAC_PI_2 / (1u64 << key.level) as f64;
                    let wrap = |i: u32| (i as f64 * edge).rem_euclid(SOIL_WRAP_M) as f32;
                    Vec4::new(wrap(key.col), wrap(key.row), edge as f32, node_split(key.level, quality.split_scale))
                },
                water: {
                    let (cu, cv) = (u0n + sizen * 0.5, v0n + sizen * 0.5);
                    let du = key.face.uv_to_dir(cu + sizen * 0.01, cv) - key.face.uv_to_dir(cu - sizen * 0.01, cv);
                    du.normalize().as_vec3().extend(0.0)
                },
                modulate: terrain.surf_modulate,
                hapke: terrain.surf_hapke,
                thermal,
                thermal2: {
                    let d = light.star_pos.delta_m(body.pos).length().max(1.0);
                    let omega = (light.star_radius_m / d).powi(2).max(1e-30);
                    Vec4::new((1.0 / (std::f64::consts::PI * omega)) as f32, light.star_teff, THERMAL_EMISSION_SHIFT, emission.scale())
                },
                glow:Vec3::from_array(s.glow_color.map(|c| c as f32)).extend(if s.glow_permanent { 0.0 } else { 1.0 }),
                ice: Vec4::new(
                    s.spec_bright_ice
                        .unwrap_or_else(|| se_core::planet_tiles::ice_spec_default(terrain.surf_gen.classes.surface, terrain.surf_gen.classes.atmo > 0))
                        as f32,
                    s.roughness_ice.unwrap_or(se_core::planet_tiles::ROUGHNESS_ICE_DEFAULT) as f32,
                    if s.diff_map_alpha_ice && layers.color.is_some() { 1.0 } else { 0.0 },
                    water_display.level,
                ),
                slot: page_slot,
            };
            // The node's lighting now (its patches all take the same).
            terrain.lights.set(
                light_slot,
                PatchLight {
                    sun_dir: light.sun_dir.extend(light.sun_intensity),
                    sun_color: light.sun_color.extend(1.0),
                    eclipse: terrain.casters,
                    light_dir: extra_dir,
                    light_color: extra_color,
                    glow_atmo: glow_atmo_u,
                    levels: Vec4::new(u.base_color.w, u.params.x, u.thermal2.w, u.ice.w),
                    flags: Vec4::new(u.atm_flags.x, u.soil_tile.w, 0.0, 0.0),
                },
            );
            terrain.body_uniform = Some(u);
            let water_u = PlanetUniform { water: u.water.truncate().extend(1.0), ..u };
            // Its own noise variant once that pipeline is built, until then the per-pixel one (`NodeNoise`).
            let biome_u = terrain.biome.unwrap_or_default();
            let want_noise = vertex_noise(&biome_u, u.rect_color.w > 0.5, u.soil_tile.z, u.proc_params2.x);
            let used_noise = pipes.usable(want_noise);
            let ground = PlanetMaterial {
                u,
                color: tex(&pc, &defaults.white),
                lights: tex(&pl, &defaults.black),
                water: tex(&pw, &defaults.black),
                clouds: tex(&pcl, &defaults.black),
                inscatter: terrain.atmosphere.as_ref().map(|a| a.inscatter.clone()).unwrap_or_else(|| defaults.black3d.clone()),
                transmittance: terrain.atmosphere.as_ref().map(|a| a.transmittance.clone()).unwrap_or_else(|| defaults.black.clone()),
                irradiance: terrain.atmosphere.as_ref().map(|a| a.irradiance.clone()).unwrap_or_else(|| defaults.black.clone()),
                ring: ring.as_ref().map_or_else(|| defaults.white.clone(), |r| r.0.clone()),
                biome: biome_u,
                samples: surface_tex.samples.clone(),
                soils: surface_tex.soils.clone(),
                normals: images.add(build.normals),
                soil_norm: surface_tex.soil_norm.clone(),
                soil_rhao: surface_tex.soil_rhao.clone(),
                sample_relief: surface_tex.sample_relief.clone(),
                noise: defaults.noise3d.clone(),
                light_page: light_page.clone(),
                cloud_cover: cloud_assets.cover.clone(),
                cloud_shadow: cloud_assets.shadow.clone(),
                cloud_map: cloud_assets.map.clone(),
                cloud_weather: cloud_assets.weather.clone(),
                cloud_pattern: cloud_assets.pattern.clone(),
                skirt: false,
                variant: used_noise,
            };
            let skirt_mat = materials.add(PlanetMaterial { skirt: true, ..ground.clone() });
            let water_mat = build.water.as_ref().map(|_| materials.add(PlanetMaterial { u: water_u, ..ground.clone() }));
            let ground_u = ground.u;
            let ground_atm = (ground.inscatter.clone(), ground.transmittance.clone(), ground.irradiance.clone());
            let material = materials.add(ground);
            let cloud_texs: Vec<(Handle<Image>, Vec4)> = build
                .cloud
                .iter()
                .map(|(i, ..)| {
                    let (p, r) = layer_rect(layers.clouds[*i].as_ref(), key);
                    (tex(&p, &defaults.black), r)
                })
                .collect();
            drop(tex_cache);
            let cloud_entities: Vec<Entity> = build
                .cloud
                .into_iter()
                .zip(cloud_texs)
                .map(|((layer, cmesh, ccenter), (ctex, crect))| {
                    let c = &terrain.clouds[layer];
                    let procedural = c.map.is_none();
                    let cmat = cloud_mats.add(CloudMaterial {
                        u: CloudUniform {
                            rect: crect,
                            params: Vec4::new(c.opacity.min(1.0), c.bump_m, if c.opaque_map { 1.0 } else { 0.0 }, 0.0),
                            planet: (-ccenter).as_vec3().extend(terrain.radius as f32),
                            proc: terrain
                                .cloud_proc
                                .filter(|_| procedural)
                                .map_or(Vec4::ZERO, |p| p.truncate().extend(1.0)),
                            proc2: Vec4::new(
                                // Each layer its own pattern.
                                (terrain.proc.seed as f32 / u64::MAX as f32 + layer as f32 * 0.379).fract(),
                                layer as f32,
                                if terrain.proc.kind.is_giant() { 0.0 } else { 1.0 },
                                (terrain.proc.seed % 10_000) as f32 / 10_000.0,
                            ),
                            se: se_cloud_uniform(&terrain.surf_gen, terrain.tidal_lock, procedural, layer),
                            color: c.color.extend(1.0),
                            look: c.look,
                            atm_flags: Vec4::new(ground_u.atm_flags.x, ATM_SUN_SCALE, ground_u.atm_flags.z, 0.0),
                            atm: ground_u.atm,
                            slot: page_slot,
                        },
                        clouds: ctex,
                        inscatter: ground_atm.0.clone(),
                        transmittance: ground_atm.1.clone(),
                        irradiance: ground_atm.2.clone(),
                        light_page: light_page.clone(),
                        cloud_weather: cloud_assets.weather.clone(),
                        cloud_shadow: cloud_assets.shadow.clone(),
                    });
                    commands
                        .spawn((
                            SystemMember,
                            CloudNode { layer },
                            BodyAttached {
                                body: body_entity,
                                center: ccenter,
                            },
                            Mesh3d(meshes.add(cmesh)),
                            MeshMaterial3d(cmat),
                            UPosition(body.pos.offset_m(body.orient * body.to_shape(ccenter))),
                            UOrientation(body.orient),
                            ScaleAnchor(body.pos),
                            MeterScale(body.shape),
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            Disabled,
                        ))
                        .id()
                })
                .collect();
            let mesh = meshes.add(build.mesh);
            let center_u = body.pos.offset_m(body.orient * body.to_shape(build.center));
            let node_noise = NodeNoise { want: want_noise, used: used_noise, sea: build.water.is_some(), ground: material.clone(), skirt: skirt_mat.clone() };
            let mut node = commands.spawn((
                    SystemMember,
                    TerrainNode,
                    node_noise,
                    BodyAttached {
                        body: body_entity,
                        center: build.center,
                    },
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    UPosition(center_u),
                    UOrientation(body.orient),
                    ScaleAnchor(body.pos),
                    MeterScale(body.shape),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    Disabled,
                ));
            // The sea surface shares the node's centre, transform, visibility and `Disabled`.
            if let (Some(wm), Some(wmat)) = (build.water, water_mat) {
                let extent_m = terrain.radius * node_angle(key.level);
                node.with_child((SeaPatch { extent_m }, Mesh3d(meshes.add(wm)), MeshMaterial3d(wmat), Transform::IDENTITY, Visibility::Inherited, Disabled));
            }
            // Its skirts, drawn after all the ground (`PlanetMaterial::skirt`), go with it as the sea does.
            node.with_child((Mesh3d(meshes.add(build.skirt)), MeshMaterial3d(skirt_mat), Transform::IDENTITY, Visibility::Inherited, Disabled));
            let e = node.id();
            let range = build.grid_heights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
            terrain.node_hrange.insert(key, range);
            terrain.node_heights.insert(key, Arc::new(build.grid_heights));
            slot.state = NodeState::Ready(e, cloud_entities);
            slot.light_gen = terrain.light_gen;
            slot.light = light_slot;
            slot.ready_frame = frame;
        }

        if !resolved {
            terrain.lights.upload(&mut shader_buffers);
            terrain.visible.clear();
            terrain.visible_rev += 1;
            terrain.vis_inputs = None;
            terrain.drawn_lit = None;
            // Hide what is shown; keep nodes cached for a while.
            for key in std::mem::take(&mut terrain.drawn) {
                if let Some(NodeSlot { state: NodeState::Ready(e, c), .. }) = terrain.nodes.get(&key) {
                    for &e in std::iter::once(e).chain(c) {
                        if let Ok((mut vis, disabled)) = vis_q.get_mut(e) {
                            show_patch(&mut commands, &children, e, &mut vis, disabled, false);
                        }
                    }
                }
            }
            continue;
        }
        if terrain.layers.is_none() {
            terrain.layers = Some(Arc::new(discover_layers(&data.vfs, terrain)));
        }
        let layers = terrain.layers.clone().unwrap();

        // Camera in body frame.
        let cam_body = body.from_shape(body.orient.inverse() * cam.pos.delta_m(body.pos));
        let mut visible: Vec<TileKey> = Vec::new();
        let mut requests: Vec<TileKey> = Vec::new();
        select_all(terrain, &cam_body, frame, &mut visible, &mut requests, quality.split_scale);
        // Under a gas giant's flat bands (its cloud tops' surface) while the volumetric clouds draw its
        // decks, the bands are hidden: seen from below only the nodes' skirts showed, a grid of lines
        // over the sky (the user's 2026-11-04 video). (Its nodes still load, for the way back up.)
        if terrain.proc.kind.is_giant() && volumetric.body == Some(body_entity) && cam_body.length() < terrain.radius {
            visible.clear();
        }
        // A turned cloud layer (FlipMap, Velocity) shows each node's patch elsewhere: its patches are
        // those of the nodes a camera turned back by the layer's spin would see.
        let cloud_visible: Vec<Option<Vec<TileKey>>> = (0..terrain.clouds.len())
            .map(|i| {
                let c = &terrain.clouds[i];
                let spin = (c.flip || c.rad_per_s != 0.0).then(|| c.spin_at(time.jd));
                spin.map(|spin| {
                    let cam_l = DQuat::from_rotation_y(-spin) * cam_body;
                    let mut v = Vec::new();
                    select_all(terrain, &cam_l, frame, &mut v, &mut requests, quality.split_scale);
                    v
                })
            })
            .collect();
        requests.sort();
        requests.dedup();
        // Start loads: SE's PlanetReverseLoading puts the deepest nodes first, then nearest first.
        requests.sort_by(|a, b| {
            let da = (a.center_dir() * terrain.radius - cam_body).length();
            let db = (b.center_dir() * terrain.radius - cam_body).length();
            let by_level = if quality.reverse_loading { b.level.cmp(&a.level) } else { std::cmp::Ordering::Equal };
            by_level.then(da.total_cmp(&db))
        });
        // SE's generator for this body: grids come back a few frames after they are asked for.
        let generator = (layers.bump.is_none() && !terrain.gen_failed && terrain.proc.se.is_some() && !terrain.proc.kind.is_giant())
            .then(|| se_core::tg::generator(&terrain.surf_gen))
            .flatten();
        if generator.is_some() {
            let done: Vec<(TileKey, Arc<Vec<f32>>)> =
                terrain.gen_jobs.iter().filter_map(|(k, &(id, _))| tgen.take(id).map(|d| (*k, d))).collect();
            for (k, d) in done {
                terrain.gen_jobs.remove(&k);
                debug!("generated {k:?}: {:.4}..{:.4}", d.iter().cloned().fold(f32::INFINITY, f32::min), d.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
                let n = crate::terrain_gen::GEN_N as usize;
                let tile = HeightTile { w: n, h: n, data: (*d).clone(), border: crate::terrain_gen::GEN_BORDER, span: crate::terrain_gen::GEN_SPAN };
                terrain.gen_tiles.insert(k, (Arc::new(tile), frame));
            }
            // A generator that cannot run (its pipeline failed): back to the port's own heights.
            // Only then: falling back because answers were slow (a job waiting 600 frames, as when
            // the window was minimized) gave the rest of the planet other heights, kilometres off
            // the nodes already loaded, with holes between them.
            if gen_queue.failed() {
                warn!("terrain generator failed; using the port's own heights");
                terrain.gen_failed = true;
                tgen.forget(terrain.gen_jobs.drain().map(|(_, (id, _))| id));
            }
            // Over the limit, grids whose node is gone and that served no request for a while go.
            // A grid arrives before its node exists (the node is built from it), so dropping every
            // grid without a node threw away each new one as it came in, once a planet had 1500 (a
            // first landing makes that many): it was asked for again, and again, and the terrain
            // under the camera stopped refining from the second landing on.
            if terrain.gen_tiles.len() > GEN_TILES_MAX {
                let nodes = &terrain.nodes;
                terrain.gen_tiles.retain(|k, &mut (_, used)| nodes.contains_key(k) || frame - used < GEN_TILE_KEEP_FRAMES);
            }
        }
        let gen_level = gen_max_level(terrain.radius);
        for key in requests {
            if in_flight >= MAX_IN_FLIGHT || (quality.time_budget_ms.is_none() && started >= quality.tiles_per_frame) {
                break;
            }
            if terrain.nodes.contains_key(&key) {
                continue;
            }
            let mut generated = None;
            if let Some(g) = generator {
                let mut gk = key;
                while gk.level > gen_level {
                    gk = gk.parent().unwrap();
                }
                match terrain.gen_tiles.get_mut(&gk) {
                    Some((tile, used)) => {
                        *used = frame;
                        generated = Some((gk, tile.clone(), gen_ref(&terrain.proc), terrain.proc.bump_m));
                    }
                    None => {
                        if !terrain.gen_jobs.contains_key(&gk) {
                            let se = se_core::tg::uniforms(&terrain.surf_gen, terrain.tidal_lock);
                            let id = crate::terrain_gen::request(&mut tgen, &mut gen_queue, &mut shader_buffers, gen_params(gk, &se, g));
                            terrain.gen_jobs.insert(gk, (id, frame));
                            started += 1;
                        }
                        continue;
                    }
                }
            }
            let tex_cache = caches.textures.lock().unwrap();
            let mut textures = Vec::new();
            for (layer, srgb) in [
                (layers.color.as_ref(), true),
                (layers.lights.as_ref(), true),
                (layers.water.as_ref(), false),
            ]
            .into_iter()
            .chain(layers.clouds.iter().zip(&terrain.clouds).map(|(l, c)| (l.as_ref(), c.opaque_map)))
            {
                if let (Some(path), _) = layer_rect(layer, key) {
                    if !tex_cache.contains_key(&path) && !textures.iter().any(|(p, _)| *p == path) {
                        textures.push((path, srgb));
                    }
                }
            }
            drop(tex_cache);
            let input = BuildInput {
                key,
                radius: terrain.radius,
                surface: terrain.surface.clone(),
                layers: layers.clone(),
                vfs: data.vfs.clone(),
                caches: caches.clone(),
                textures,
                cloud_radii: terrain.drawn_clouds(&layers),
                proc: terrain.proc.clone(),
                ocean: terrain.has_ocean,
                generated,
            };
            let task = AsyncComputeTaskPool::get().spawn(async move { build_node(input) });
            terrain.nodes.insert(
                key,
                NodeSlot {
                    state: NodeState::Loading(task),
                    last_used: frame,
                    ready_frame: u64::MAX,
                    light_gen: 0,
                    light: u32::MAX,
                },
            );
            terrain.loading.push(key);
            in_flight += 1;
            started += 1;
        }
        // Visibility: of the nodes drawn now and those shown last frame (every other is hidden).
        // The same lists as last frame change nothing (their nodes are all ready and, touched every
        // frame, cannot be evicted), so the pass is skipped and `drawn` stays.
        // The flat cloud shells, unless the volumetric clouds draw this body's alone (within 40 radii,
        // volumetric_clouds.rs).
        let flat_clouds = switches.clouds && !(volumetric.body == Some(body_entity) && volumetric.flat_hidden);
        if !vis_inputs_same(terrain, &visible, &cloud_visible, flat_clouds) {
            let mut touch: Vec<TileKey> = std::mem::take(&mut terrain.drawn);
            let before = touch.clone();
            touch.extend(visible.iter().copied());
            touch.extend(cloud_visible.iter().flatten().flatten().copied());
            touch.sort_unstable();
            touch.dedup();
            terrain.visible = visible;
            terrain.visible_rev += 1;
            let visible = &terrain.visible;
            let mut drawn = Vec::with_capacity(touch.len());
            for key in touch {
                let Some(NodeSlot { state: NodeState::Ready(e, c), .. }) = terrain.nodes.get(&key) else { continue };
                let want = visible.binary_search(&key).is_ok();
                let mut shown = false;
                for &e in std::iter::once(e).chain(c) {
                    let cloud = clouds_q.get(e).ok();
                    let want = match cloud.and_then(|cn| cloud_visible.get(cn.layer)?.as_ref()) {
                        Some(v) => v.binary_search(&key).is_ok(),
                        None => want,
                    } && (cloud.is_none() || flat_clouds);
                    shown |= want;
                    if let Ok((mut vis, disabled)) = vis_q.get_mut(e) {
                        show_patch(&mut commands, &children, e, &mut vis, disabled, want);
                    }
                }
                if shown {
                    drawn.push(key);
                }
            }
            if drawn != before {
                terrain.drawn_lit = None;
            }
            terrain.drawn = drawn;
            terrain.vis_inputs = Some((cloud_visible, flat_clouds));
        }
        // Update lighting on materials when the sun moves noticeably.
        let sun = light.sun_dir.extend(light.sun_intensity);
        // SE's atmospheres switch (Shift+A) also takes the aerial perspective off the ground and clouds.
        let atm_on = if terrain.atmosphere.is_some() && switches.atmospheres { 1.0 } else { 0.0 };
        if terrain.last_ambient != ambient.0
            || terrain.last_emission != *emission
            || terrain.last_water != water_display.level
            || switches.is_changed()
        {
            terrain.last_ambient = ambient.0;
            terrain.last_emission = *emission;
            terrain.last_water = water_display.level;
            terrain.last_sun = Vec4::ZERO;
        }
        let glow_bright = terrain.surface.glow_bright as f32 * 1.5 * emission.scale();
        let glow_atmo = terrain.glow_atmo(emission.scale()).extend(0.0);
        let eclipse = terrain.casters;
        let (extra_dir, extra_color) = light.extra_uniforms(false);
        let extra_moved = extra_dir.iter().zip(&terrain.last_extra).any(|(a, b)| (*a - *b).length() > 1e-3);
        // A new LandLOD, graphics preset or window height changes the split distance: the nodes
        // built before morph over the old one unless it is set again (with the lighting).
        let split_scale = quality.split_scale;
        if (sun - terrain.last_sun).length() > 1e-3
            || extra_moved
            || casters_differ(&eclipse, &terrain.last_casters)
            || terrain.last_split != split_scale
        {
            terrain.last_sun = sun;
            terrain.last_extra = extra_dir;
            terrain.last_casters = eclipse;
            terrain.last_split = split_scale;
            terrain.light_gen += 1;
        }
        // The shown nodes are lit now; cached ones when they are shown again. Only their light
        // records change: the materials (ground, sea surface, clouds) read them by slot.
        let light_gen = terrain.light_gen;
        let lit = PatchLight {
            sun_dir: sun,
            sun_color: light.sun_color.extend(1.0),
            eclipse,
            light_dir: extra_dir,
            light_color: extra_color,
            glow_atmo,
            levels: Vec4::new(ambient.0, glow_bright, emission.scale(), water_display.level),
            flags: Vec4::new(atm_on, 0.0, 0.0, 0.0),
        };
        terrain.body_light = lit;
        // Lit for this generation already and none shown since: nothing to relight.
        if terrain.drawn_lit != Some(light_gen) {
            for key in &terrain.drawn {
                let Some(slot) = terrain.nodes.get_mut(key) else { continue };
                if slot.light_gen == light_gen {
                    continue;
                }
                slot.light_gen = light_gen;
                if let NodeState::Ready(..) = &slot.state {
                    terrain.lights.set(slot.light, PatchLight { flags: Vec4::new(atm_on, node_split(key.level, split_scale), 0.0, 0.0), ..lit });
                }
            }
            terrain.drawn_lit = Some(light_gen);
        }
        terrain.lights.upload(&mut shader_buffers);
    }
    // SE's CacheSizeLandNode (main-def.cfg 10000): the nodes of all planets together; beyond it the
    // least recently used ones go first (only nodes unused for STALE_FRAMES, never a root or one
    // still loading).
    let total: usize = bodies.iter().map(|(_, _, _, t, _)| t.nodes.len()).sum();
    let cap = memory.land_nodes;
    *tick += 1;
    if *tick % 60 == 0 {
        trim_texture_cache(&caches, *tick);
    }
    if *tick % 300 == 0 {
        debug!("terrain: {total} resident nodes (cache {cap})");
    }
    if total > cap {
        let mut stale: Vec<(u64, Entity, TileKey)> = bodies
            .iter()
            .flat_map(|(e, _, _, t, _)| {
                t.nodes
                    .iter()
                    .filter(|(k, s)| k.level > 0 && !matches!(s.state, NodeState::Loading(_)) && t.frame - s.last_used > STALE_FRAMES)
                    .map(move |(k, s)| (t.frame - s.last_used, e, *k))
            })
            .collect();
        stale.sort_unstable_by_key(|s| std::cmp::Reverse(s.0));
        for (_, e, k) in stale.into_iter().take(evict_count(total, cap)) {
            let Ok((_, _, _, mut terrain, _)) = bodies.get_mut(e) else { continue };
            terrain.node_heights.remove(&k);
            terrain.node_hrange.remove(&k);
            if let Some(slot) = terrain.nodes.remove(&k) {
                if let NodeState::Ready(e, c) = slot.state {
                    for e in std::iter::once(e).chain(c) {
                        commands.entity(e).despawn();
                    }
                    terrain.lights.release(slot.light);
                }
            }
        }
    }
}

/// Show or hide a node's patch. A hidden one is also `Disabled`, so the thousands of cached patches
/// (SE's CacheSizeLandNode, 10000 nodes) drop out of every per-entity scan (transforms, visibility,
/// render extraction); shown again, it is enabled before the floating origin places it (the
/// commands apply at the sync point after `update_terrain`). Its sea surface (a child) goes with it.
fn show_patch(commands: &mut Commands, children: &Query<&Children, Allow<Disabled>>, e: Entity, vis: &mut Mut<Visibility>, disabled: bool, show: bool) {
    vis.set_if_neq(if show { Visibility::Visible } else { Visibility::Hidden });
    if show == disabled {
        for e in std::iter::once(e).chain(children.get(e).into_iter().flatten().copied()) {
            if show {
                commands.entity(e).remove::<Disabled>();
            } else {
                commands.entity(e).insert(Disabled);
            }
        }
    }
}

/// Whether the visibility pass last ran for these very lists (each cloud layer's `None`, following
/// the ground, or its own list) and clouds switch.
fn vis_inputs_same(t: &Terrain, visible: &[TileKey], cloud_visible: &[Option<Vec<TileKey>>], clouds_on: bool) -> bool {
    t.vis_inputs.as_ref().is_some_and(|(c, on)| *on == clouds_on && c.as_slice() == cloud_visible) && t.visible == visible
}

/// Drops the least recently used texture tiles beyond [`TEXTURE_CACHE_MB`].
fn trim_texture_cache(caches: &TileCaches, now: u32) {
    // (`SE_TILE_CACHE_MB` overrides the limit, for comparisons.)
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let cap = *CAP.get_or_init(|| std::env::var("SE_TILE_CACHE_MB").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(TEXTURE_CACHE_MB)) << 20;
    let Ok(mut c) = caches.textures.lock() else { return };
    let mut total: usize = c.values().map(|t| t.bytes).sum();
    if total <= cap {
        return;
    }
    let mut old: Vec<(u32, String, usize)> =
        c.iter().filter(|(_, t)| now.wrapping_sub(t.used) > TEXTURE_KEEP_FRAMES).map(|(p, t)| (t.used, p.clone(), t.bytes)).collect();
    old.sort_unstable_by_key(|o| o.0);
    for (_, path, bytes) in old {
        if total <= cap {
            break;
        }
        c.remove(&path);
        total -= bytes;
    }
}

/// A node unused for this many frames may be evicted.
const STALE_FRAMES: u64 = 30;

/// Nodes to evict from `total` over the cache size `cap`: the excess, at least 64 at a time.
pub fn evict_count(total: usize, cap: usize) -> usize {
    if total > cap { (total - cap).max(64) } else { 0 }
}

/// Walks the quadtrees of all six faces ([`select`]); `visible` comes back sorted.
fn select_all(t: &mut Terrain, cam_body: &DVec3, frame: u64, visible: &mut Vec<TileKey>, requests: &mut Vec<TileKey>, scale: f64) {
    let horizon = Horizon::new(*cam_body, t.cull_top(), t.horizon_floor);
    for face in CubeFace::ALL {
        let root = TileKey::root(face);
        select(t, root, root.center_dir(), -1.0, cam_body, &horizon, frame, visible, requests, scale);
    }
    // Each node is visited once a walk, so this is the list the sorted inserts made.
    visible.sort_unstable();
}

/// Quadtree traversal. Pushes nodes to draw into `visible` (in walk order) and nodes to load
/// into `requests`. Returns whether this subtree is drawable. `dir` is the node's centre direction
/// and `gap` its [`Horizon::gap`] (unused on a root), both from its parent.
#[allow(clippy::too_many_arguments)]
fn select(
    t: &mut Terrain,
    key: TileKey,
    dir: DVec3,
    gap: f64,
    cam_body: &DVec3,
    horizon: &Horizon,
    frame: u64,
    visible: &mut Vec<TileKey>,
    requests: &mut Vec<TileKey>,
    scale: f64,
) -> bool {
    // In use even past the horizon: a node there stays cached under its parent. Evicted, it came
    // back into view as the horizon receded (flying up) and the parent's coarse whole stood in for
    // it, often the whole view, for the frame or two it took to reload (a flicker).
    if let Some(slot) = t.nodes.get_mut(&key) {
        slot.last_used = frame;
    }
    if key.level > 0 && beyond_horizon(gap) {
        return true;
    }
    let ready = |t: &Terrain, k: &TileKey| {
        t.nodes.get(k).is_some_and(|s| matches!(s.state, NodeState::Ready(..)) && s.ready_frame < frame)
    };
    if wants_split(key, dir, t.radius, *cam_body, scale, height_range(t, key)) {
        let children = key.children();
        let dirs = children.map(|c| c.center_dir());
        let gaps: [f64; 4] = std::array::from_fn(|i| horizon.gap(children[i], dirs[i]));
        let all_ready = (0..4).all(|i| ready(t, &children[i]) || beyond_horizon(gaps[i]));
        if all_ready {
            // Children within a node of the horizon load ahead, ready when it recedes.
            for (c, g) in children.iter().zip(gaps) {
                if !t.nodes.contains_key(c) && g < node_angle(c.level) {
                    requests.push(*c);
                }
            }
            let mut ok = true;
            for i in 0..4 {
                ok &= select(t, children[i], dirs[i], gaps[i], cam_body, horizon, frame, visible, requests, scale);
            }
            if ok {
                return true;
            }
        } else {
            for (c, g) in children.iter().zip(gaps) {
                if !t.nodes.contains_key(c) && !beyond_horizon(g) {
                    requests.push(*c);
                }
            }
        }
    }
    if ready(t, &key) {
        visible.push(key);
        true
    } else {
        if !t.nodes.contains_key(&key) {
            requests.push(key);
        }
        false
    }
}

/// Minimum eye height above the rendered ground, meters (SpaceEngine's `ClipHeight`,
/// 0.0015 km in `config/main-def.cfg`).
const MIN_ALTITUDE_M: f64 = 1.5;

/// Minimum height above a body drawn as a plain sphere (stars, and bodies without terrain), as a
/// fraction of its radius. Such a body is one mesh scaled to its radius and placed relative to the
/// camera in `f32`, which is only good to about `radius * 1.2e-7` (80 m for the Sun); closer than
/// a few dozen of those steps the surface could land on the camera's side and open up. SpaceEngine
/// stops the camera at its stars' procedural surface, 1.6 km above the Sun's photosphere.
const SPHERE_MIN_ALTITUDE: f64 = 2.0e-6;

/// Push the camera back to `min_r` from a body centre if it is closer, and stop its motion into
/// the body; like SpaceEngine, hitting a surface resets the assigned velocity to 100 m/s.
fn push_out(cam: &mut FlyCamera, center: UPos, rel: DVec3, dist: f64, min_r: f64) {
    if dist >= min_r {
        return;
    }
    let up = rel / dist;
    cam.pos = center.offset_m(up * min_r);
    let v = cam.velocity;
    let into = v.dot(up);
    if into < 0.0 {
        cam.velocity = v - up * into;
        cam.speed = cam.speed.min(100.0);
    }
}

/// Push the camera out of the terrain of any nearby body and out of stars and other bodies
/// drawn as spheres, and keep the near clipping plane in front of the ground:
/// `nearest_surface` (which sets it) counts from the reference sphere, which lies kilometres
/// below the camera in high mountains.
fn keep_camera_above_ground(
    mut cam: ResMut<FlyCamera>,
    clouds: Res<crate::volumetric_clouds::VolumetricClouds>,
    bodies: Query<(Entity, &SystemBody, &Terrain)>,
    spheres: Query<&SystemBody, Without<Terrain>>,
    mut proj: Query<&mut Projection, With<crate::camera::MainCamera>>,
) {
    let mut clearance = f64::INFINITY;
    let mut depth: f64 = 0.0;
    for body in &spheres {
        let rel = cam.pos.delta_m(body.pos);
        let dist = rel.length();
        if body.radius_m <= 0.0 || dist > body.radius_m * 1.2 || dist <= 0.0 {
            continue;
        }
        let min_r = body.radius_m + (body.radius_m * SPHERE_MIN_ALTITUDE).max(MIN_ALTITUDE_M);
        push_out(&mut cam, body.pos, rel, dist, min_r);
        clearance = clearance.min(dist.max(min_r) - body.radius_m);
    }
    for (e, body, terrain) in &bodies {
        let rel = cam.pos.delta_m(body.pos);
        let dist = rel.length();
        if dist > terrain.top_radius().max(body.radius_m) * 1.2 || dist <= 0.0 {
            continue;
        }
        // On SE's ellipsoid: the terrain sphere's direction drawn along the camera's, and how far out
        // along it the ellipsoid lies relative to the sphere.
        let dir_body = body.from_shape(body.orient.inverse() * rel).normalize();
        let k = body.to_shape(dir_body).length();
        if let (Some(back), true) = (cam.find_shore.take(), terrain.has_ocean) {
            match terrain.nearest_shore(dir_body, body.radius_m, back) {
                Some((land, to_sea)) => {
                    let up = body.orient * land;
                    let g = terrain.ground_height(land).unwrap_or(0.0).max(0.0);
                    cam.pos = body.pos.offset_m(body.orient * body.to_shape(land * (body.radius_m + g)) + up * 2.0);
                    cam.rot = crate::camera::look_rotation((body.orient * to_sea - up * 0.035).normalize(), up);
                    cam.velocity = DVec3::ZERO;
                }
                None => warn!("shore: no coast in the loaded terrain"),
            }
            continue;
        }
        // A gas giant whose volumetric decks are drawn: the camera goes down through them, to below the
        // view's dark limit (its flat bands are only their tops).
        let floor = crate::volumetric_clouds::giant_floor_m(terrain).filter(|_| clouds.active == Some(e));
        // Before any of the surface loads, a world without oceans has its ground at the
        // reference radius; one with oceans is left alone, as its seabed is unknown.
        let Some(ground) = floor.or_else(|| terrain.ground_height(dir_body)).or((!terrain.has_ocean).then_some(0.0)) else {
            clearance = clearance.min((dist - body.radius_m * k).abs());
            continue;
        };
        let min_r = (body.radius_m + ground) * k + MIN_ALTITUDE_M;
        let (rel, dist) = match cam.hold_agl {
            Some(agl) => {
                let r = (body.radius_m + ground) * k + agl.max(MIN_ALTITUDE_M);
                cam.pos = body.pos.offset_m(rel * (r / dist));
                (rel * (r / dist), r)
            }
            None => (rel, dist),
        };
        push_out(&mut cam, body.pos, rel, dist, min_r);
        clearance = clearance.min(dist.max(min_r) - (min_r - MIN_ALTITUDE_M));
        // The camera passes through an ocean's surface (down to the seabed, as in SE), but
        // the surface still needs the near plane in front of it.
        if terrain.has_ocean {
            let alt = dist.max(min_r) - body.radius_m * k;
            clearance = clearance.min(alt.abs());
            depth = depth.max(-alt);
        }
    }
    cam.depth_m = depth;
    // Under water `update_nearest` (from the reference sphere) says 0: use the distance to the
    // sea surface or the seabed, which also brings the near plane in to millimetres at the surface.
    if clearance < cam.nearest_surface || depth > 0.0 {
        cam.nearest_surface = clearance.max(0.001);
        for mut p in &mut proj {
            if let Projection::Perspective(p) = p.as_mut() {
                // `min`: `ship_near_plane` may already have brought it closer for the hull.
                p.near = p.near.min(crate::camera::near_plane(cam.nearest_surface));
            }
        }
    }
}

/// Keep node world positions/orientations attached to their moving, rotating body.
fn update_node_transforms(
    time: Res<crate::universe::SimTime>,
    bodies: Query<(&SystemBody, Option<&Terrain>)>,
    mut nodes: Query<(&BodyAttached, Option<&CloudNode>, &Visibility, &mut UPosition, &mut UOrientation, &mut ScaleAnchor)>,
) {
    for (node, cloud, vis, mut pos, mut rot, mut anchor) in &mut nodes {
        if *vis == Visibility::Hidden {
            continue;
        }
        if let Ok((b, terrain)) = bodies.get(node.body) {
            // A cloud layer turns about the pole (FlipMap, Velocity); the shell stays on the
            // ellipsoid as long as the body is round about its pole, as every one with clouds is.
            let spin = cloud
                .zip(terrain)
                .and_then(|(c, t)| t.clouds.get(c.layer))
                .map_or(DQuat::IDENTITY, |l| DQuat::from_rotation_y(l.spin_at(time.jd)));
            pos.0 = b.pos.offset_m(b.orient * b.to_shape(spin * node.center));
            rot.0 = b.orient * spin;
            anchor.0 = b.pos;
        }
    }
}

// ---------------------------------------------------------------------------
// Cloud layer material.

/// SE's HeightMapCloudsTerra inputs: 0 (cloudsFreq, cloudsOctaves, stripeZones, stripeTwist),
/// 1 (cycloneMagn, cycloneFreq, cycloneDensity, cycloneOctaves), 2 (Randomize, tidal lock +-1);
/// zeros (the port's own clouds) for mapped layers and giants. SE moves the k-th layer's (from 1)
/// Randomize by k (0.716, -0.239, 0.381) (FUN_1402fba00; 0 is the ground), so each has its pattern.
pub(crate) fn se_cloud_uniform(g: &se_core::surface_gen::SurfaceGen, tidal_lock: bool, procedural: bool, layer: usize) -> [Vec4; 3] {
    use se_core::surface_gen as sg;
    if !procedural || g.get(sg::CLOUDS_MAIN_FREQ).is_none() || (sg::bulk::NEPTUNE..=sg::bulk::BROWN_DWARF).contains(&g.classes.bulk) {
        return [Vec4::ZERO; 3];
    }
    let v = |i: usize| g.v(i);
    let k = (layer + 1) as f32;
    [
        Vec4::new(v(sg::CLOUDS_MAIN_FREQ), v(sg::CLOUDS_MAIN_OCTAVES).clamp(1.0, 12.0), v(sg::STRIPE_ZONES), v(sg::STRIPE_TWIST)),
        Vec4::new(v(sg::CYCLONE_MAGN), v(sg::CYCLONE_FREQ), v(sg::CYCLONE_DENSITY), v(sg::CYCLONE_OCTAVES).clamp(0.0, 8.0)),
        Vec4::new(
            v(sg::RANDOMIZE) + k * 0.716,
            v(sg::RANDOMIZE + 1) - k * 0.239,
            v(sg::RANDOMIZE + 2) + k * 0.381,
            if tidal_lock { 1.0 } else { -1.0 },
        ),
    ]
}

/// The fixed part of a cloud patch's material; its lighting is its node's [`PatchLight`].
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct CloudUniform {
    /// Cloud tile sub-rectangle: xy offset, z size, w enabled.
    pub rect: Vec4,
    /// x opacity, y relief of the cloud tops (m; SE's `BumpHeight` with a `BumpMap`, else 0),
    /// z 1 when the map is the colour of an opaque layer (`CloudLayer::opaque_map`).
    pub params: Vec4,
    /// xyz: planet centre in node-local coordinates (m); w: planet radius (m).
    pub planet: Vec4,
    /// Procedural clouds (no texture): x coverage, y base frequency, z octaves, w enabled.
    pub proc: Vec4,
    /// x seed, y the layer's index in `Terrain::clouds`, z latitude belt strength, w the body's seed
    /// fraction (with `planet.w`, which body the volumetric clouds' record is for).
    pub proc2: Vec4,
    /// SE's terra clouds ([`se_cloud_uniform`]).
    pub se: [Vec4; 3],
    /// rgb: albedo (`CloudLayer::color`).
    pub color: Vec4,
    /// `CloudLayer::look`.
    pub look: Vec4,
    /// x: atmosphere enabled (the slot's), y: sun irradiance scale for the tables, z: the scattered
    /// light's scale relative to y, w unused (SkyLight is in `atm.look`).
    pub atm_flags: Vec4,
    pub atm: AtmosphereParamsUniform,
    /// The node's record in the material's light page.
    pub slot: u32,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CloudMaterial {
    #[uniform(0)]
    pub u: CloudUniform,
    #[texture(1)]
    #[sampler(2)]
    pub clouds: Handle<Image>,
    /// The body's atmosphere tables (placeholders without one), as on [`PlanetMaterial`].
    #[texture(3, dimension = "3d")]
    #[sampler(4)]
    pub inscatter: Handle<Image>,
    #[texture(5)]
    pub transmittance: Handle<Image>,
    #[texture(6)]
    pub irradiance: Handle<Image>,
    /// The light page of the patch's node ([`PlanetMaterial::light_page`]).
    #[storage(7, read_only)]
    pub light_page: Handle<ShaderBuffer>,
    /// The volumetric clouds' weather cube and record (as on [`PlanetMaterial`]): for the body they
    /// draw, the weather shapes the shell's cover and the far cross-fade fades it.
    #[texture(8, dimension = "cube")]
    #[sampler(9)]
    pub cloud_weather: Handle<Image>,
    #[storage(10, read_only)]
    pub cloud_shadow: Handle<ShaderBuffer>,
}

impl Material for CloudMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/clouds.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/clouds.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        // After the sky shells, which would otherwise hide it in daylight (see `SkyMaterial`).
        1.0e10
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glowing_atmospheres() {
        // None below SE's 644.211 K, or with self-emission off.
        assert_eq!(glow_color_atmo(600.0, 900.0, 1.0), Vec3::ZERO);
        assert_eq!(glow_color_atmo(2500.0, 2500.0, 0.0), Vec3::ZERO);
        // A 2500 K hot Jupiter: (2500/5572)^4 x 10^0.525 x 23.9 = 3.24 times a red Planck colour.
        let g = glow_color_atmo(2500.0, 2500.0, 1.0);
        let c = crate::blackholes::planck_lookup(2500.0);
        assert!((g.x as f64 / c[0] - 3.24).abs() < 0.05, "{g:?} {c:?}");
        assert!(g.x > g.y && g.y > g.z && g.z >= 0.0);
        // At 4000 K: 0.2656 x 10^0.877 x 1.226 = 2.45 (the gain falls as the Planck colour rises).
        assert!((glow_color_atmo(4000.0, 2500.0, 1.0).x as f64 / c[0] - 2.45).abs() < 0.05);
        // The colour follows the surface temperature.
        let w = glow_color_atmo(2500.0, 6000.0, 1.0);
        assert!(w.z / w.x > g.z / g.x);
    }

    #[test]
    fn colour_noises_at_vertices_only_on_fine_nodes() {
        use VertexNoise as V;
        // An airless world (SE's generator 1) with young terrain, like RSN 0-7-460129-1254-197-12 4
        // (radius 2909.6 km): its finest noise feature is 2909.6 km / 2.218^7 = 11 km, so the nodes
        // carry the noises at their vertices up to cells of 11 km / 16 / 1.5 = 459 m (15 km edges).
        let mut b = crate::surface_tex::BiomeUniform::default();
        b.flags.x = 1.0;
        b.climate[4].w = 1.0;
        b.climate[7].z = 1.0;
        b.climate[7].w = 0.3;
        let r = 2_909_640.0;
        assert_eq!(vertex_noise(&b, false, 14_000.0, r), V::Selena);
        assert_eq!(vertex_noise(&b, false, 16_000.0, r), V::None);
        // Without young terrain the finest is the variation's, 2.6 times coarser.
        b.climate[7].w = 0.0;
        assert_eq!(vertex_noise(&b, false, 30_000.0, r), V::Selena);
        // Not for the procedural asteroids' own colour, or with the biomes off.
        b.climate[7].z = 2.0;
        assert_eq!(vertex_noise(&b, false, 100.0, r), V::None);
        assert_eq!(vertex_noise(&b, true, 100.0, r), V::Selena);
        b.climate[7].z = 1.0;
        b.flags.x = 0.0;
        assert_eq!(vertex_noise(&b, false, 100.0, r), V::None);

        // A terra world (generator 0) like RSN 0-7-460129-1254-197-12 2 (radius 4376 km): the
        // albedo variation's finest feature is 16.6 km (nodes up to 22 km), the climate ridges' by
        // hillsFreq 400 and hillsMagn 0.13 are 339 m (nodes up to 452 m).
        b.flags.x = 1.0;
        b.climate[7] = Vec4::new(400.0, 0.13, 0.0, 0.0);
        b.climate[5].y = 800.0;
        let r = 4_376_000.0;
        // Coarser nodes split it: the warp's finest features (1.3 radii) fit even the roots'.
        assert_eq!(vertex_noise(&b, false, 23_000.0, r), V::TerraPartial);
        assert_eq!(vertex_noise(&b, false, r * std::f32::consts::FRAC_PI_2, r), V::TerraPartial);
        assert_eq!(vertex_noise(&b, false, 20_000.0, r), V::Terra);
        assert_eq!(vertex_noise(&b, false, 500.0, r), V::Terra);
        assert_eq!(vertex_noise(&b, false, 400.0, r), V::TerraClimate);
        // With plants the humidity's 9.6 km features are the finest of the first set.
        b.flags.z = 1.0;
        assert_eq!(vertex_noise(&b, false, 20_000.0, r), V::TerraPartial);
        assert_eq!(vertex_noise(&b, false, 12_000.0, r), V::Terra);
        // Not SE's generator (the designed biomes of Earth and other catalog worlds).
        b.climate[4].w = 0.0;
        assert_eq!(vertex_noise(&b, false, 100.0, r), V::None);
    }

    #[test]
    fn sea_reach_follows_se() {
        // 2 / WaterParams.z: 200 km at sea level, 2 km from WaterVisibilityHeight (10 km) up.
        assert!((water_reach_m(1.0, 3).unwrap() - 199_980.2).abs() < 0.1);
        assert_eq!(water_reach_m(5_000.0, 3), Some(101_000.0));
        assert_eq!(water_reach_m(20_000.0, 2), Some(2_000.0));
        // Under water, and on Oceanic and Superoceanic worlds, everywhere.
        assert_eq!(water_reach_m(-3.0, 3), None);
        assert_eq!(water_reach_m(50_000.0, 4), None);
        assert_eq!(water_reach_m(50_000.0, 5), None);
    }

    #[test]
    fn normal_offset_is_flat_on_a_smooth_sphere() {
        let up = DVec3::new(0.3, 0.8, -0.52).normalize();
        assert!(decode_normal_offset(encode_normal_offset(up, up)).length() < 1e-4);
    }

    #[test]
    fn normal_offset_round_trips_finer_on_gentle_slopes() {
        let up = DVec3::new(0.0, 1.0, 0.0);
        for (tilt, tol) in [(0.01, 0.002), (0.1, 0.004), (0.6, 0.02)] {
            let n = DVec3::new(tilt, 1.0, 0.0).normalize();
            let back = (up.as_vec3() + decode_normal_offset(encode_normal_offset(n, up))).normalize();
            assert!((back - n.as_vec3()).length() < tol, "tilt {tilt}: {back} vs {n}");
        }
    }

    #[test]
    fn water_display_fades_over_se_overlay_times() {
        let mut w = WaterDisplay { show: false, ..Default::default() };
        w.step(0.175);
        assert!((w.level - 0.5).abs() < 1e-5, "{}", w.level);
        w.step(1.0);
        assert_eq!(w.level, 0.0);
        w.show = true;
        w.step(0.1);
        assert!((w.level - 0.5).abs() < 1e-5);
        w.step(1.0);
        assert_eq!(w.level, 1.0);
    }

    #[test]
    fn self_emission_scale() {
        assert_eq!(SelfEmission::default().scale(), 1.0);
        assert_eq!(SelfEmission { enabled: true, bright: 2.5 }.scale(), 2.5);
        assert_eq!(SelfEmission { enabled: false, bright: 2.5 }.scale(), 0.0);
    }

    #[test]
    fn normal_offset_mips_average() {
        let flat = encode_offset(Vec3::ZERO);
        let level0: Vec<u8> = (0..4).flat_map(|_| [flat[0], flat[1], flat[2], 200]).collect();
        let (all, levels) = with_mips(level0, 2);
        assert_eq!(levels, 2);
        assert!(decode_normal_offset([all[16], all[17], all[18]]).length() < 1e-4);
        assert_eq!(all[19], 200);
    }

    fn test_terrain(radius: f64) -> Terrain {
        let rec = BodyRecord {
            tag: se_core::catalog::BodyTag::Planet,
            names: vec!["Test".into()],
            parent: String::new(),
            class: None,
            radius_km: radius / 1000.0,
            mass_kg: 6.0e24,
            oblateness: 0.0,
            color: None,
            albedo: 0.3,
            orbit: None,
            rotation: Default::default(),
            lock: Default::default(),
            source: String::new(),
            block: Default::default(),
        };
        use se_core::surface_gen as sg;
        let surf_gen = sg::SurfaceGen { p: [sg::UNSET; sg::N_PARAMS], color: None, classes: Default::default() };
        Terrain::new(radius, &rec, 288.0, surf_gen, None, 5.0)
    }

    #[test]
    fn light_pages_keep_their_size_and_records() {
        let mut buffers = Assets::<ShaderBuffer>::default();
        let mut pages = LightPages::default();
        let slots: Vec<u32> = (0..70).map(|_| pages.alloc(&mut buffers)).collect();
        assert_eq!(slots, (0..70).collect::<Vec<_>>());
        assert_eq!(pages.pages.len(), 2);
        // Freed slots are taken again, lowest first, before any new one.
        pages.release(66);
        pages.release(5);
        assert_eq!([pages.alloc(&mut buffers), pages.alloc(&mut buffers), pages.alloc(&mut buffers)], [5, 66, 70]);
        let rec = PatchLight {
            sun_dir: Vec4::new(0.6, 0.8, 0.0, 0.7),
            sun_color: Vec4::new(1.0, 0.9, 0.8, 1.0),
            light_dir: [Vec4::new(0.0, 1.0, 0.0, 1.0); MAX_LIGHTS - 1],
            levels: Vec4::new(0.01, 1.5, 1.0, 0.5),
            flags: Vec4::new(1.0, 2.2, 0.0, 0.0),
            ..Default::default()
        };
        pages.set(67, rec);
        assert!(!pages.pages[0].dirty && pages.pages[1].dirty);
        pages.upload(&mut buffers);
        assert!(!pages.pages[1].dirty);
        // Setting the same record again changes nothing.
        pages.set(67, rec);
        assert!(!pages.pages[1].dirty);
        // A page is always the same size (a resize would rebuild its materials), and reads back.
        assert_eq!(PatchLight::min_size().get(), 752);
        for p in &pages.pages {
            assert_eq!(buffers.get(&p.buffer).unwrap().buffer_size(), 752 * LIGHT_PAGE_SLOTS as u64);
        }
        let b = buffers.get(&pages.pages[1].buffer).unwrap();
        let bevy::render::storage::ShaderBufferData::Initialized { data, .. } = &b.data else { panic!() };
        let back: Vec<PatchLight> = encase::StorageBuffer::new(&data[..]).create().unwrap();
        assert_eq!(back[3], rec);
        assert_eq!(pages.page(67).1, 3);
    }

    // The walk before the horizon was cached and `visible` sorted once (02-3), for the test below.
    fn old_horizon_gap(key: TileKey, cam_body: DVec3, top: f64, floor: f64) -> f64 {
        let rc = cam_body.length();
        if rc <= 0.0 {
            return -1.0;
        }
        let r0 = floor.min(rc);
        let horizon = (r0 / rc).clamp(-1.0, 1.0).acos() + (r0 / top.max(r0)).clamp(-1.0, 1.0).acos();
        let ang = cam_body.normalize().dot(key.center_dir()).clamp(-1.0, 1.0).acos();
        ang - (horizon + node_angle(key.level) + 0.01)
    }

    fn old_beyond_horizon(key: TileKey, cam_body: DVec3, top: f64, floor: f64) -> bool {
        old_horizon_gap(key, cam_body, top, floor) > 0.0
    }

    fn old_select(t: &mut Terrain, key: TileKey, cam_body: &DVec3, frame: u64, visible: &mut Vec<TileKey>, requests: &mut Vec<TileKey>, scale: f64) -> bool {
        if let Some(slot) = t.nodes.get_mut(&key) {
            slot.last_used = frame;
        }
        if key.level > 0 && old_beyond_horizon(key, *cam_body, t.cull_top(), t.horizon_floor) {
            return true;
        }
        let ready = |t: &Terrain, k: &TileKey| t.nodes.get(k).is_some_and(|s| matches!(s.state, NodeState::Ready(..)) && s.ready_frame < frame);
        if wants_split(key, key.center_dir(), t.radius, *cam_body, scale, height_range(t, key)) {
            let children = key.children();
            let all_ready = children.iter().all(|c| ready(t, c) || old_beyond_horizon(*c, *cam_body, t.cull_top(), t.horizon_floor));
            if all_ready {
                for c in children {
                    if !t.nodes.contains_key(&c) && old_horizon_gap(c, *cam_body, t.cull_top(), t.horizon_floor) < node_angle(c.level) {
                        requests.push(c);
                    }
                }
                let mut ok = true;
                for c in children {
                    ok &= old_select(t, c, cam_body, frame, visible, requests, scale);
                }
                if ok {
                    return true;
                }
            } else {
                for c in children {
                    if !t.nodes.contains_key(&c) && !old_beyond_horizon(c, *cam_body, t.cull_top(), t.horizon_floor) {
                        requests.push(c);
                    }
                }
            }
        }
        if ready(t, &key) {
            if let Err(pos) = visible.binary_search(&key) {
                visible.insert(pos, key);
            }
            true
        } else {
            if !t.nodes.contains_key(&key) {
                requests.push(key);
            }
            false
        }
    }

    #[test]
    fn select_walk_matches_the_old_one() {
        let radius = 6.4e6;
        // On the ground, low, high, far, and at the centre; the camera drifts each frame.
        for (alt, dir) in [(2.0, DVec3::new(0.3, -0.5, 0.8)), (300.0, DVec3::new(-0.9, 0.1, 0.2)), (4.0e5, DVec3::X), (2.0e7, DVec3::new(0.2, 0.9, -0.4)), (-radius, DVec3::Y)] {
            let (mut a, mut b) = (test_terrain(radius), test_terrain(radius));
            for frame in 1..60u64 {
                let d = (dir.normalize() + DVec3::new(0.0, 1e-7, 2e-7) * frame as f64).normalize();
                let cam = d * (radius + alt);
                let (mut va, mut ra, mut vb, mut rb) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                for face in CubeFace::ALL {
                    old_select(&mut a, TileKey::root(face), &cam, frame, &mut va, &mut ra, 1.0);
                }
                select_all(&mut b, &cam, frame, &mut vb, &mut rb, 1.0);
                assert_eq!((&va, &ra), (&vb, &rb), "alt {alt} frame {frame}");
                // The requested nodes are ready the next frame, some of them with relief.
                for (i, k) in ra.iter().enumerate().take(400) {
                    for t in [&mut a, &mut b] {
                        t.nodes.entry(*k).or_insert(NodeSlot { state: NodeState::Ready(Entity::PLACEHOLDER, Vec::new()), last_used: frame, ready_frame: frame, light_gen: 0, light: 0 });
                        if i % 3 == 0 {
                            t.node_hrange.insert(*k, (-50.0, 900.0 + i as f32));
                        }
                    }
                }
            }
            let used = |t: &Terrain| t.nodes.iter().map(|(k, s)| (*k, s.last_used)).collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(used(&a), used(&b));
            assert!(a.nodes.len() > 20, "alt {alt}: {} nodes", a.nodes.len());
            if alt == 2.0 {
                assert!(a.nodes.len() > 1000 && a.nodes.keys().any(|k| k.level >= 15), "{} nodes", a.nodes.len());
            }
        }
    }

    #[test]
    fn visibility_pass_skips_only_the_same_inputs() {
        let mut t = test_terrain(6.4e6);
        let k = |l| TileKey::root(CubeFace::ALL[l]);
        t.visible = vec![k(0), k(1)];
        // Never run (or everything hidden since): the pass must run.
        assert!(!vis_inputs_same(&t, &[k(0), k(1)], &[None], true));
        t.vis_inputs = Some((vec![None, Some(vec![k(2)])], true));
        assert!(vis_inputs_same(&t, &[k(0), k(1)], &[None, Some(vec![k(2)])], true));
        assert!(!vis_inputs_same(&t, &[k(0)], &[None, Some(vec![k(2)])], true));
        assert!(!vis_inputs_same(&t, &[k(0), k(1)], &[None, Some(vec![k(2)])], false));
        assert!(!vis_inputs_same(&t, &[k(0), k(1)], &[None, Some(vec![])], true));
        // A layer that starts or stops turning changes shape, even with no patches.
        t.vis_inputs = Some((vec![None, Some(vec![])], true));
        assert!(!vis_inputs_same(&t, &[k(0), k(1)], &[None, None], true));
        assert!(!vis_inputs_same(&t, &[k(0), k(1)], &[None], true));
    }

    /// Times one walk, old and new, over a settled tree 2 m above a 6400 km planet:
    /// `cargo test -p se_engine --release bench_select -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_select() {
        let radius = 6.4e6;
        let mut t = test_terrain(radius);
        t.clouds.push(CloudLayer::from_block(&se_core::sc::Block::default(), radius, 1.0));
        let cam = DVec3::new(0.31, 0.83, 0.47).normalize() * (radius + 2.0);
        let mut frame = 1;
        for _ in 0..400 {
            frame += 1;
            let (mut v, mut r) = (Vec::new(), Vec::new());
            select_all(&mut t, &cam, frame, &mut v, &mut r, 1.0);
            r.sort();
            r.dedup();
            if r.is_empty() {
                break;
            }
            for k in r {
                t.nodes.insert(k, NodeSlot { state: NodeState::Ready(Entity::PLACEHOLDER, Vec::new()), last_used: frame, ready_frame: frame, light_gen: 0, light: 0 });
                t.node_hrange.insert(k, ((k.row % 7) as f32 * -10.0, (k.col % 5) as f32 * 30.0 + 50.0));
            }
        }
        let n = 3000;
        let (mut vo, mut vn) = (Vec::new(), Vec::new());
        for round in 0..3 {
            let t0 = std::time::Instant::now();
            for _ in 0..n {
                frame += 1;
                let (mut v, mut r) = (Vec::new(), Vec::new());
                for face in CubeFace::ALL {
                    old_select(&mut t, TileKey::root(face), &cam, frame, &mut v, &mut r, 1.0);
                }
                vo = std::hint::black_box(v);
            }
            let old = t0.elapsed().as_secs_f64() * 1e3 / n as f64;
            let t1 = std::time::Instant::now();
            for _ in 0..n {
                frame += 1;
                let (mut v, mut r) = (Vec::new(), Vec::new());
                select_all(&mut t, &cam, frame, &mut v, &mut r, 1.0);
                vn = std::hint::black_box(v);
            }
            let new = t1.elapsed().as_secs_f64() * 1e3 / n as f64;
            // The poll loop's old key scan (nothing loading once settled) against the loading list.
            let t2 = std::time::Instant::now();
            let mut hits = 0;
            for _ in 0..n {
                hits += t.nodes.values().filter(|n| matches!(n.state, NodeState::Loading(_))).count();
                let keys: Vec<TileKey> = t.nodes.keys().copied().collect();
                let start = std::time::Instant::now();
                for key in keys {
                    if start.elapsed().as_secs_f32() * 1000.0 >= 10.0 {
                        break;
                    }
                    let slot = t.nodes.get_mut(&key).unwrap();
                    let NodeState::Loading(_) = &mut slot.state else { continue };
                    hits += 1;
                }
            }
            let poll = t2.elapsed().as_secs_f64() * 1e3 / n as f64;
            println!("round {round}: {} nodes, {} visible: walk old {old:.3} ms, new {new:.3} ms; old poll scan {poll:.3} ms ({hits})", t.nodes.len(), vn.len());
        }
        assert_eq!(vo, vn);
    }

    // update_eclipses before it skipped the bodies under half a pixel (04-4), for the test below.
    fn old_update_eclipses(
        switches: Res<crate::switches::RenderSwitches>,
        mut set: ParamSet<(Query<(&SystemBody, Option<&Terrain>)>, Query<(&SystemBody, &BodyLighting, &mut Terrain)>)>,
    ) {
        let occluders: Vec<(ObjectRef, UPos, f64, bool)> = set
            .p0()
            .iter()
            .filter(|(b, _)| !b.star && b.radius_m > 1000.0)
            .map(|(b, t)| (b.obj, b.pos, b.radius_m, t.is_some_and(|t| t.atmosphere.is_some())))
            .collect();
        for (body, light, mut terrain) in &mut set.p1() {
            let mut e = EclipseUniform::default();
            let star_dist = light.star_pos.delta_m(body.pos).length();
            if switches.eclipses && body.px_radius > 0.5 && light.star_radius_m > 0.0 && star_dist > 0.0 {
                let a_s = (light.star_radius_m / star_dist).min(1.0).asin();
                let mut found: Vec<(f64, Vec4)> = Vec::new();
                for &(obj, pos, r_o, atm) in &occluders {
                    if obj == body.obj {
                        continue;
                    }
                    let w = body.pos.delta_m(pos);
                    let to_star = light.star_pos.delta_m(pos).normalize_or(DVec3::Y);
                    let t = -w.dot(to_star);
                    if t <= 0.0 {
                        continue;
                    }
                    let perp = (w + to_star * t).length();
                    if perp > body.radius_m + r_o + t * a_s {
                        continue;
                    }
                    let rel = -w / body.radius_m;
                    let r = (r_o / body.radius_m) as f32;
                    found.push((r_o / w.length(), rel.as_vec3().extend(if atm { -r } else { r })));
                }
                found.sort_by(|a, b| b.0.total_cmp(&a.0));
                for (i, (_, o)) in found.iter().take(4).enumerate() {
                    e.occ[i] = *o;
                }
                e.info = Vec4::new(a_s as f32, found.len().min(4) as f32, 0.0, 0.0);
            }
            if e != terrain.eclipse {
                terrain.eclipse = e;
            }
        }
    }

    /// Times update_eclipses, old and new, over 2500 bodies of which 3 are resolved:
    /// `cargo test -p se_engine --release bench_eclipses -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_eclipses() {
        for (name, new) in [("old", false), ("new", true), ("old", false), ("new", true)] {
            let mut w = World::new();
            w.insert_resource(crate::switches::RenderSwitches::default());
            let mut s = Schedule::default();
            if new {
                s.add_systems(update_eclipses);
            } else {
                s.add_systems(old_update_eclipses);
            }
            let light = BodyLighting { star_radius_m: 7.0e8, ..Default::default() };
            for i in 0..2500u32 {
                let a = i as f64;
                let p = DVec3::new(a.sin(), (a * 1.7).cos(), (a * 0.3).sin()) * 1.5e11 * (1.0 + 0.001 * a);
                let b = SystemBody {
                    obj: ObjectRef::Body(i),
                    radius_m: 1.0e6,
                    star: false,
                    pos: UPos::ZERO.offset_m(p),
                    orient: DQuat::IDENTITY,
                    px_radius: if i < 3 { 50.0 } else { 0.01 },
                    shape: DVec3::ONE,
                };
                w.spawn((b, light, test_terrain(1.0e6)));
            }
            s.run(&mut w);
            let n = 300;
            let t0 = std::time::Instant::now();
            for _ in 0..n {
                s.run(&mut w);
            }
            println!("{name}: {:.3} ms", t0.elapsed().as_secs_f64() * 1e3 / n as f64);
        }
    }

    #[test]
    fn eclipses_match_visiting_every_body() {
        let mut worlds: Vec<(World, Schedule)> = (0..2)
            .map(|i| {
                let mut w = World::new();
                w.insert_resource(crate::switches::RenderSwitches::default());
                let mut s = Schedule::default();
                if i == 0 {
                    s.add_systems(old_update_eclipses);
                } else {
                    s.add_systems(update_eclipses);
                }
                (w, s)
            })
            .collect();
        let au = 1.5e11;
        let light = BodyLighting { star_radius_m: 7.0e8, ..Default::default() };
        let body = |i: u32, x: f64, y: f64, r: f64, px: f64| SystemBody {
            obj: ObjectRef::Body(i),
            radius_m: r,
            star: false,
            pos: UPos::ZERO.offset_m(DVec3::new(x, y, 0.0)),
            orient: DQuat::IDENTITY,
            px_radius: px,
            shape: DVec3::ONE,
        };
        let atm = || AtmosphereGpu {
            params: se_core::atm::AtmParams {
                radius_ground: 6360.0,
                radius_top: 6420.0,
                rayleigh_h: 8.0,
                mie_h: 1.2,
                rayleigh_beta: [0.0; 3],
                mie_sca_beta: [0.0; 3],
                mie_ext_beta: [0.0; 3],
                mie_g: 0.8,
                mie_fade: 0.1,
            },
            inscatter: Handle::default(),
            transmittance: Handle::default(),
            irradiance: Handle::default(),
        };
        let mut ids = Vec::new();
        for (w, _) in &mut worlds {
            let mut e = Vec::new();
            // A lit planet, a moon with an atmosphere and one without shading it, a body with no
            // terrain in front of both, one behind, and small far bodies.
            e.push(w.spawn((body(0, au, 0.0, 6.4e6, 40.0), light, test_terrain(6.4e6))).id());
            let mut moon = test_terrain(1.7e6);
            moon.atmosphere = Some(atm());
            e.push(w.spawn((body(1, au - 4.0e8, 2.0e6, 1.7e6, 3.0), light, moon)).id());
            e.push(w.spawn((body(2, au - 3.0e8, -1.0e6, 1.7e6, 2.0), light, test_terrain(1.7e6))).id());
            e.push(w.spawn(body(3, au - 6.0e8, 0.5e6, 2.5e6, 0.2)).id());
            e.push(w.spawn((body(4, au + 4.0e8, 0.0, 1.0e6, 0.1), light, test_terrain(1.0e6))).id());
            for i in 5..12 {
                e.push(w.spawn((body(i, au + 1.0e9 * i as f64, 0.0, 2.0e5, 0.01), light, test_terrain(2.0e5))).id());
            }
            ids.push(e);
        }
        assert_eq!(ids[0], ids[1]);
        let ids = ids[0].clone();
        // Frame by frame: as set up, the planet under half a pixel, back, the moon moving out of
        // line, the eclipses switched off and on, and the planet removed.
        for frame in 0..8 {
            for (w, s) in &mut worlds {
                match frame {
                    1 => w.get_mut::<SystemBody>(ids[0]).unwrap().px_radius = 0.3,
                    2 => w.get_mut::<SystemBody>(ids[0]).unwrap().px_radius = 40.0,
                    3 => w.get_mut::<SystemBody>(ids[1]).unwrap().pos = UPos::ZERO.offset_m(DVec3::new(au - 4.0e8, 5.0e7, 0.0)),
                    4 => w.resource_mut::<crate::switches::RenderSwitches>().eclipses = false,
                    5 => w.resource_mut::<crate::switches::RenderSwitches>().eclipses = true,
                    6 => w.get_mut::<SystemBody>(ids[1]).unwrap().pos = UPos::ZERO.offset_m(DVec3::new(au - 4.0e8, 2.0e6, 0.0)),
                    7 => {
                        w.despawn(ids[0]);
                    }
                    _ => {}
                }
                s.run(w);
            }
            let eclipses = |w: &World| ids.iter().map(|e| w.get::<Terrain>(*e).map(|t| t.eclipse)).collect::<Vec<_>>();
            assert_eq!(eclipses(&worlds[0].0), eclipses(&worlds[1].0), "frame {frame}");
            if frame == 0 {
                let e = worlds[0].0.get::<Terrain>(ids[0]).unwrap().eclipse;
                assert!(e.info.y >= 2.0 && e.occ.iter().any(|o| o.w < 0.0), "{e:?}");
            }
        }
    }
}
