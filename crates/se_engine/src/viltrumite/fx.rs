//! What the air does round him (spec F9): the plasma shell, the sonic cone, the rings, the trail, one additive pass after the clouds
//! (`fx_pass`, `hero_fx.wesl`). The glow on his body is in his own material (`glow_record`, `hero.wesl` `shade`).
//!
//! - **One record buffer**, `FxRecordsGpu`, written in place each frame: no material changes, nothing is spawned, the trail is a fixed
//!   ring of 256 points in the body's frame, worked out from the camera each frame.
//! - **The pass** has no depth attachment and no vertex buffers. Every vertex comes from the records (`clip.z = 0.5 w`: nothing is cut by
//!   the near plane). It reads the scene's depth for a soft edge, and the clouds' resolved transmittance beyond their traced distance, so
//!   clouds in front dim it and clouds behind don't. It runs on the main camera alone (the `CloudView` filter), with the clouds on or off,
//!   and does nothing at all while nothing shows: `FxInput::draw`.
//! - **Off costs nothing:** one run condition. All of it is made on first enable.
//! - **Impacts (step 8, `impact.rs`):** the craters are a pass of their own, `fx_decals`, before the clouds: it multiplies what the main
//!   pass drew, so a crater takes the ground's own light, night and cloud shadow, and it reads the depth, so it follows whatever ground is
//!   drawn. The dust is a draw of its own in `fx_pass`, blended over (not additive), drawn first; it takes the colour of the lit ground
//!   round where it rose (`fx_dust_tint`, a compute pass reading the scene before the craters darken it), so it's the ground's own
//!   colour in the ground's own light, night and cloud shadow, whatever the planet. Both run in the aftermath too (the mod turned off
//!   with craters or dust still there: `ModControl::aftermath`), when nothing of his draws.

use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer};
use bevy::render::render_resource::encase;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::{Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderSystems};
use se_core::catalog::ObjectRef;
use se_core::upos::UPos;

use crate::camera::{FlyCamera, MainCamera};
use crate::mods::ModControl;
use crate::system::SystemBody;
use crate::volumetric_clouds::{CloudHistory, CloudTextures, CloudView, CloudsPassSet, HDR_FORMAT, NearHistory};

use super::draw::HeroDraws;
use super::{Hero, tune};

/// `hero_fx.wesl`'s rings and trail points.
pub const RINGS: usize = 4;
pub const TRAIL_POINTS: usize = 256;
pub const CRATERS: usize = 32;
pub const DUST_BURSTS: usize = 64;
/// The capsules of him the craters leave alone (`impact::BodyParts`; `hero_fx.wesl` `parts` holds two points each).
pub const PARTS: usize = 24;

/// The draws, as `instance_index` tells them apart: `hero_fx.wesl` `FX_*`. Rings follow from `FX_RING` on.
const FX_SHELL: u32 = 0;
const FX_CONE: u32 = 1;
const FX_TRAIL: u32 = 2;
const FX_RING: u32 = 3;
/// A crater's draw: the rectangle its box covers on the screen (`hero_fx.wesl` `decal_vertex`).
const DECAL_VERTICES: u32 = 6;
/// `hero_fx.wesl`'s `AROUND`, `SHELL_RINGS`, `CONE_BANDS`.
const AROUND: u32 = 32;
const SHELL_RINGS: u32 = 16;
const CONE_BANDS: u32 = 6;

#[derive(Clone, Copy, Default, ShaderType)]
struct FxRingGpu {
    centre: Vec4,
    normal: Vec4,
    tint: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct FxPointGpu {
    at: Vec4,
    light: Vec4,
}

/// `hero_fx.wesl` `FxCrater`: `at` xyz its middle on the ground as drawn now, from the camera, w its radius (m); `up` xyz the ground's
/// normal there, w its seed (0..1); `look` x how far in (0..1), y the bowl's depth, z the lip's height, w how far out it reaches (radii);
/// `sea` xyz the sea's up there, w how high its middle stands over the sea (m; 1e9 on a body with none).
#[derive(Clone, Copy, Default, Debug, PartialEq, ShaderType)]
pub struct FxCraterGpu {
    pub at: Vec4,
    pub up: Vec4,
    pub look: Vec4,
    pub sea: Vec4,
}

/// `hero_fx.wesl` `FxDust`: `at` xyz where it rose from, from the camera, w its age (s); `up` xyz up there, w its size (m); `look` rgb its
/// colour (albedo), w its seed; `motion` x its life (s), y gravity (m/s2), z the air's hold on it (s; 0 in vacuum), w its style
/// (`impact::Style`).
#[derive(Clone, Copy, Default, Debug, PartialEq, ShaderType)]
pub struct FxDustGpu {
    pub at: Vec4,
    pub up: Vec4,
    pub look: Vec4,
    pub motion: Vec4,
}

/// `hero_fx.wesl` `FxRecords`.
#[derive(Clone, Copy, ShaderType)]
pub struct FxRecordsGpu {
    him: Vec4,
    along: Vec4,
    shell: Vec4,
    shell_size: Vec4,
    cone: Vec4,
    cloud_px: Vec4,
    /// Towards the star the light comes from (his; in the aftermath the nearest crater's, `draw::aftermath_light`).
    pub sun: Vec4,
    /// His body as capsules, which the craters leave alone: x how many of `parts` (0 when he isn't drawn).
    pub body: Vec4,
    /// The craters' burnt ground: x at the middle, y at the edge; z how high over the sea a point must stand to be ground (m).
    pub decal: Vec4,
    /// The capsules (`impact::BodyParts`): two points from the camera each (m), the first's w its radius.
    pub parts: [Vec4; PARTS * 2],
    rings: [FxRingGpu; RINGS],
    trail: [FxPointGpu; TRAIL_POINTS],
    pub craters: [FxCraterGpu; CRATERS],
    pub dust: [FxDustGpu; DUST_BURSTS],
}

impl Default for FxRecordsGpu {
    fn default() -> Self {
        FxRecordsGpu {
            him: Vec4::ZERO,
            along: Vec4::NEG_Z,
            shell: Vec4::ZERO,
            shell_size: Vec4::ZERO,
            cone: Vec4::ZERO,
            cloud_px: Vec4::ONE,
            sun: Vec4::Y,
            body: Vec4::ZERO,
            decal: Vec4::new(tune::CRATER_DARK.0, tune::CRATER_DARK.1, tune::CRATER_SEA_M, 0.0),
            parts: [Vec4::ZERO; PARTS * 2],
            rings: [FxRingGpu::default(); RINGS],
            trail: [FxPointGpu::default(); TRAIL_POINTS],
            craters: [FxCraterGpu::default(); CRATERS],
            dust: [FxDustGpu::default(); DUST_BURSTS],
        }
    }
}

fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// From `from` at `at.0` to `to` at `at.1`, exponentially: the temperature's stages by brightness, never T^4.
fn by_stage(t: f64, at: (f64, f64), (from, to): (f64, f64)) -> f64 {
    let k = ((t / at.0).ln() / (at.1 / at.0).ln()).clamp(0.0, 1.0);
    from * (to / from).powf(k)
}

/// SE's black body (`planck_rgb`, what `ship.wesl`'s table holds) with its brightest channel 1: the brightness is F9's own.
pub fn heat_colour(t: f64) -> Vec3 {
    let [r, g, b] = crate::blackholes::planck_rgb(t);
    let m = r.max(g).max(b).max(1e-30);
    let (g0, g1) = tune::HEAT_COLOUR_GAMMA;
    let (t0, t1, t2) = tune::HEAT_COLOUR_K;
    let power = g0 + (g1 - g0) * smooth(t0, t1, t) + (1.0 - g1) * smooth(t1, t2, t);
    Vec3::new((r / m).powf(power) as f32, (g / m).powf(power).max(tune::HEAT_COLOUR_LEAST_GREEN) as f32, (b / m).powf(power) as f32)
}

/// How far past the look's clamp the shell's size and the trail's length are.
fn past_clamp(t: f64) -> f64 {
    (t / tune::HEAT_LOOK_K).max(1.0).powf(tune::HEAT_PAST_POWER).min(tune::HEAT_PAST_MAX)
}

/// The glow on his body at this temperature: 0, or between the HDR limits.
pub fn glow_at(t: f64) -> f64 {
    let t = t.min(tune::HEAT_LOOK_K);
    smooth(tune::GLOW_FROM_K.0, tune::GLOW_FROM_K.1, t) * by_stage(t.max(tune::GLOW_FROM_K.1), (tune::GLOW_FROM_K.1, tune::HEAT_LOOK_K), tune::GLOW_HDR)
}

/// The shell's brightness (before its colour).
pub fn shell_at(t: f64) -> f64 {
    let t = t.min(tune::HEAT_LOOK_K);
    smooth(tune::SHELL_FROM_K.0, tune::SHELL_FROM_K.1, t) * by_stage(t.max(tune::SHELL_FROM_K.1), (tune::SHELL_FROM_K.1, tune::HEAT_LOOK_K), tune::SHELL_HDR)
}

/// The trail's brightness at a new point.
pub fn trail_at(t: f64) -> f64 {
    let t = t.min(tune::HEAT_LOOK_K);
    smooth(tune::TRAIL_FROM_K.0, tune::TRAIL_FROM_K.1, t) * by_stage(t.max(tune::TRAIL_FROM_K.1), (tune::TRAIL_FROM_K.1, tune::HEAT_LOOK_K), tune::TRAIL_HDR)
}

/// The readout's bar, 0..1.
pub fn heat_bar(t: f64) -> f32 {
    ((t.max(1.0) / tune::HEAT_BAR_FROM_K).ln() / (tune::HEAT_LOOK_K / tune::HEAT_BAR_FROM_K).ln()).clamp(0.0, 1.0) as f32
}

/// The cone's strength by Mach and the air: F9's visible range, thin air showing less. 0 out of the air the barrier counts.
pub fn cone_at(mach: f64, q: f64, pressure_pa: f64) -> f64 {
    let (a, b, c, d) = tune::CONE_MACH;
    smooth(a, b, mach) * (1.0 - smooth(c, d, mach)) * air_show(q, pressure_pa)
}

/// F9's half-angle, `asin(1 / M)`, in radians; a flat disc up to Mach 1.
pub fn cone_angle(mach: f64) -> f64 {
    (1.0 / mach.max(1.0)).asin()
}

/// How much of the cone and the ring the air shows: from `AIR_SHOW`'s least in thin air to all of it as `log10 q` goes through
/// `BOOM_LOG_Q`, and none where the barrier doesn't count the air (F9: under 100 Pa no boom, so no cone either), fading in to 150 Pa.
pub fn air_show(q: f64, pressure_pa: f64) -> f64 {
    let log_q = if q > 0.0 { q.log10() } else { f64::NEG_INFINITY };
    let thin = tune::AIR_SHOW.0 + (tune::AIR_SHOW.1 - tune::AIR_SHOW.0) * ((log_q - tune::BOOM_LOG_Q.0) / (tune::BOOM_LOG_Q.1 - tune::BOOM_LOG_Q.0)).clamp(0.0, 1.0);
    let (air_on, air_off) = tune::BOOM_AIR_PA;
    thin * smooth(air_off, air_on, pressure_pa)
}

/// His glow's records (`HeroFrame.heat`, `heat_lee`, `heat_dir`): the windward side's light, the lee side's, the way he's going.
/// Off: w 0 and nothing done in the pixels.
pub fn glow_record(heat_k: f64, going: DVec3) -> (Vec4, Vec4, Vec4) {
    let t = heat_k.min(tune::HEAT_LOOK_K);
    let glow = glow_at(t);
    if glow <= 0.0 {
        return (Vec4::ZERO, Vec4::ZERO, Vec4::ZERO);
    }
    let lee = heat_colour(t * tune::GLOW_LEE_T) * (glow * tune::GLOW_LEE).min(tune::GLOW_LEE_MAX) as f32;
    ((heat_colour(t) * glow as f32).extend(1.0), lee.extend(0.0), going.as_vec3().extend(0.0))
}

/// A ring of the air: the boom's, the thunderclap's (step 8).
#[derive(Clone, Copy, Debug, Default)]
struct RingState {
    /// `None`: not in use.
    obj: Option<ObjectRef>,
    /// Where it is and which way it faces in that body's frame.
    local: DVec3,
    normal: DVec3,
    /// `Hero.clock` when it started.
    born: f64,
    /// How much of its light the air shows.
    show: f32,
    /// Its size and life.
    shape: RingShape,
}

/// A ring's size and life: the radius it starts at and ends at (m), its life (s), how long it rides with him (s), and how far it goes
/// along its normal over its life (m: the thunderclap's ring travels away from his hands, so a chase camera sees it go).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingShape {
    pub start_m: f64,
    pub end_m: f64,
    pub life_s: f64,
    pub ride_s: f64,
    pub travel_m: f64,
}

impl Default for RingShape {
    /// The boom's (F9).
    fn default() -> Self {
        RingShape { start_m: tune::RING_START_M, end_m: tune::RING_START_M + tune::RING_GROW_MS * tune::RING_LIFE_S * 0.5, life_s: tune::RING_LIFE_S, ride_s: tune::RING_RIDE_S, travel_m: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TrailPoint {
    /// In the frame of `FxState::trail_body`.
    local: DVec3,
    born: f64,
    life: f32,
    light: Vec3,
    /// No segment from this point to the next older one.
    cut: bool,
}

/// The effects' state between frames: fixed arrays, nothing grows.
pub struct FxState {
    rings: [RingState; RINGS],
    trail: Box<[TrailPoint; TRAIL_POINTS - 1]>,
    /// The newest point's slot, and how many there are.
    head: usize,
    count: usize,
    trail_body: Option<ObjectRef>,
    /// The last point was laid down last frame (else the next one starts anew).
    laying: bool,
    /// `Hero.clock` of the last pulse of the cone.
    pulse: f64,
    /// The temperature the gas round him goes by (K).
    gas_k: f64,
    /// `Hero.clock` when the last of the trail's points dies: points laid as he cools live less, so the newest isn't always the last.
    alive_until: f64,
}

impl Default for FxState {
    fn default() -> Self {
        FxState {
            rings: [RingState::default(); RINGS],
            trail: Box::new([TrailPoint::default(); TRAIL_POINTS - 1]),
            head: 0,
            count: 0,
            trail_body: None,
            laying: false,
            pulse: f64::NEG_INFINITY,
            gas_k: 0.0,
            alive_until: f64::NEG_INFINITY,
        }
    }
}

impl FxState {
    /// A new ring at `local`, facing `normal`, both in `obj`'s frame: the oldest gives way. The boom's shape.
    pub fn add_ring(&mut self, obj: ObjectRef, local: DVec3, normal: DVec3, clock: f64, show: f32) {
        self.add_ring_shaped(obj, local, normal, clock, show, RingShape::default());
    }

    /// The same, of any size and life (the thunderclap's).
    pub fn add_ring_shaped(&mut self, obj: ObjectRef, local: DVec3, normal: DVec3, clock: f64, show: f32, shape: RingShape) {
        let slot = (0..RINGS).min_by(|&a, &b| {
            let age = |i: usize| if self.rings[i].obj.is_some() { self.rings[i].born } else { f64::NEG_INFINITY };
            age(a).total_cmp(&age(b))
        });
        if let Some(i) = slot {
            self.rings[i] = RingState { obj: Some(obj), local, normal, born: clock, show, shape };
        }
    }

    /// One frame of the gas's temperature, from what the air does to him now and his low-passed heat.
    pub fn ease_gas(&mut self, skin_k: f64, heat_k: f64, dt: f64) {
        let flowing = skin_k.min(heat_k);
        self.gas_k = if flowing >= self.gas_k { flowing } else { self.gas_k + (flowing - self.gas_k) * (1.0 - (-dt / tune::GAS_FALL_S).exp()) };
    }

    pub fn gas_k(&self) -> f64 {
        self.gas_k
    }

    /// A pulse of the cone (Super to Sub).
    pub fn pulse(&mut self, clock: f64) {
        self.pulse = clock;
    }

    /// Lay down a point of the trail if he's far enough on from the last one: at least `TRAIL_SPACING_M`, and at least a 254th of the
    /// longest life still to run, so the ring holds a point's life at any frame rate (F9's `v dt` alone filled it in 255 frames: 1.5 s at
    /// 165 Hz, against lives of up to 4.5 s, and the tail was cut off while still bright).
    fn lay_point(&mut self, obj: ObjectRef, local: DVec3, clock: f64, heat_k: f64) {
        if self.trail_body != Some(obj) {
            self.count = 0;
            self.trail_body = Some(obj);
            self.laying = false;
            self.alive_until = f64::NEG_INFINITY;
        }
        let k = (heat_k.min(tune::HEAT_LOOK_K) - tune::TRAIL_FROM_K.1) / (tune::HEAT_LOOK_K - tune::TRAIL_FROM_K.1);
        let life = (tune::TRAIL_LIFE_S.0 + (tune::TRAIL_LIFE_S.1 - tune::TRAIL_LIFE_S.0) * k.clamp(0.0, 1.0)) * past_clamp(heat_k).powi(2);
        let brightness = trail_at(heat_k);
        if brightness <= 0.0 {
            self.laying = false;
            return;
        }
        if self.laying && self.count > 0 {
            let last = &self.trail[self.head];
            let every = life.max(self.alive_until - clock) / (self.trail.len() - 1) as f64;
            if (last.local - local).length() < tune::TRAIL_SPACING_M || clock - last.born < every {
                return;
            }
        }
        let colour = heat_colour(heat_k.min(tune::HEAT_LOOK_K) * tune::TRAIL_COLOUR_T);
        // (A new stretch: the gap back to the old trail's last point has no segment.)
        let cut = self.count > 0 && !self.laying;
        self.head = (self.head + 1) % self.trail.len();
        self.count = (self.count + 1).min(self.trail.len());
        self.trail[self.head] = TrailPoint { local, born: clock, life: life as f32, light: colour * brightness as f32, cut };
        self.alive_until = self.alive_until.max(clock + life as f32 as f64);
        self.laying = true;
    }

    /// Is anything of the trail still alive? (Any point, not the newest: see `alive_until`.)
    fn trail_alive(&self, clock: f64) -> bool {
        self.count > 0 && clock < self.alive_until
    }

    /// The trail's records, newest first, from each point's place as the camera sees it (`at`), with his place `him` first at no light
    /// while he's laying it: how many segments to draw. A dead point stays in the ribbon at no light, so a newer one laid as he cooled
    /// and dying first doesn't take the older ones with it; the ribbon ends one point past the oldest living one, or, the ring run out
    /// with that one alive, at that one, at no light: never a bright cut end.
    fn trail_records(&self, clock: f64, at: impl Fn(DVec3) -> Vec3, him: Vec3, out: &mut [FxPointGpu; TRAIL_POINTS]) -> u32 {
        // A wake's width: by how far behind him, how old, how far from the camera.
        let width = |behind: f64, age: f64, at: Vec3| {
            (tune::TRAIL_WIDTH_M + tune::TRAIL_WAKE * behind.sqrt() + tune::TRAIL_WIDEN_MS * age) as f32 + tune::TRAIL_WIDEN_RAD as f32 * at.length()
        };
        let mut n = 0usize;
        let mut behind = 0.0f64;
        let mut last = him;
        if self.laying {
            // It starts faint: the shell is what's bright at him.
            out[0] = FxPointGpu { at: him.extend(width(0.0, 0.0, him)), light: Vec4::ZERO };
            n = 1;
        }
        let len = self.trail.len();
        let mut oldest_alive = None;
        for k in 0..self.count {
            let p = self.trail[(self.head + len - k) % len];
            let age = clock - p.born;
            let share = (age / p.life as f64).min(1.0) as f32;
            let place = at(p.local);
            behind += (place - last).length() as f64;
            last = place;
            let w = width(behind, age, place);
            let light = if share < 1.0 {
                oldest_alive = Some(n);
                p.light * (1.0 - share) * (1.0 - share) * smooth(0.0, tune::TRAIL_BEGINS_M, behind) as f32
            } else {
                Vec3::ZERO
            };
            out[n] = FxPointGpu { at: place.extend(if p.cut { -w } else { w }), light: light.extend(share) };
            n += 1;
        }
        let Some(oldest) = oldest_alive else { return 0 };
        let n = (oldest + 2).min(n);
        if n == oldest + 1 {
            out[oldest].light = Vec4::new(0.0, 0.0, 0.0, out[oldest].light.w);
        }
        n.saturating_sub(1) as u32
    }
}

/// The one record buffer, written in place, kept here so nothing is allocated a frame.
#[derive(Resource)]
pub struct FxDraws {
    buffer: Handle<ShaderBuffer>,
    pub(super) records: Box<FxRecordsGpu>,
}

/// What the pass needs from the main world, made on first enable.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct FxInput {
    pub on: bool,
    /// Anything to draw this frame.
    pub draw: bool,
    /// Which draws: the shell, the cone, how many segments of trail, which rings.
    shell: bool,
    cone: bool,
    segments: u32,
    rings: u32,
    /// The craters drawn (bit n: slot n), and each dust burst's quads (0: none).
    pub(super) craters: u32,
    pub(super) dust: [u8; DUST_BURSTS],
    records: Handle<ShaderBuffer>,
    /// His light of this frame (`hero_light.wesl`'s output): the rims take it too (`impact::RimMaterial`).
    pub(super) light: Handle<ShaderBuffer>,
    /// The dust's sprites (`sprites/ptfx_smoke_wispy_anim.dds`, a 7 x 7 flipbook), set by `impact::spawn_impacts`.
    pub(super) sprite: Handle<Image>,
}

/// The passes' pipelines, queued on first enable, each [no MSAA, MSAA]: the additive effects, the dust (blended over), the craters
/// (multiplied).
#[derive(Resource, Default)]
struct FxPass {
    layouts: Option<[BindGroupLayoutDescriptor; 2]>,
    pipelines: [Option<CachedRenderPipelineId>; 2],
    dust: [Option<CachedRenderPipelineId>; 2],
    decals: [Option<CachedRenderPipelineId>; 2],
    sampler: Option<Sampler>,
    /// The dust's colour: each burst's lit ground round it and the seed of the burst it's for (`hero_fx.wesl` `dust_tint`), made once.
    tint: Option<Buffer>,
    tint_layouts: Option<[BindGroupLayoutDescriptor; 2]>,
    tint_pipelines: [Option<CachedComputePipelineId>; 2],
}

pub fn build(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<FxInput>::default()).add_systems(Update, sync_on.run_if(resource_exists::<FxInput>));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<FxPass>()
        .add_systems(Render, queue_pipelines.in_set(RenderSystems::PrepareBindGroups).run_if(fx_made))
        // After `clouds_pass`, before the solids' upscale; the craters before the clouds, so clouds in front hide them (C17).
        .add_systems(Core3d, fx_pass.in_set(Core3dSystems::EarlyPostProcess).after(CloudsPassSet).before(crate::render_scale::SolidUpscaleSet).run_if(fx_wanted))
        .add_systems(Core3d, fx_decals.in_set(Core3dSystems::EarlyPostProcess).before(CloudsPassSet).run_if(decals_wanted))
        // The dust's colour, from the scene before the craters darken it.
        .add_systems(Core3d, fx_dust_tint.in_set(Core3dSystems::EarlyPostProcess).before(fx_decals).before(CloudsPassSet).run_if(dust_wanted));
}

fn fx_made(input: Option<Res<FxInput>>) -> bool {
    input.is_some()
}

fn fx_wanted(input: Option<Res<FxInput>>) -> bool {
    input.is_some_and(|i| i.on && i.draw)
}

fn decals_wanted(input: Option<Res<FxInput>>) -> bool {
    input.is_some_and(|i| i.on && i.craters != 0)
}

fn dust_wanted(input: Option<Res<FxInput>>) -> bool {
    input.is_some_and(|i| i.on && i.dust.iter().any(|&q| q > 0))
}

/// First enable: the buffer. `light` is his own light's buffer, written before the main pass. (The dust's sprite comes with the impacts.)
pub(super) fn spawn(commands: &mut Commands, buffers: &mut Assets<ShaderBuffer>, light: Handle<ShaderBuffer>) {
    let records = Box::<FxRecordsGpu>::default();
    let buffer = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&*records), bevy::asset::RenderAssetUsages::default()));
    commands.insert_resource(FxInput { on: true, draw: false, shell: false, cone: false, segments: 0, rings: 0, craters: 0, dust: [0; DUST_BURSTS], records: buffer.clone(), light, sprite: Handle::default() });
    commands.insert_resource(FxDraws { buffer, records });
}

/// Off: nothing to draw, unless the aftermath is still draining (craters, dust).
fn sync_on(control: Res<ModControl>, mut input: ResMut<FxInput>) {
    let on = control.hero || control.aftermath;
    if input.on != on {
        input.on = on;
    }
}

/// After his effects and the impacts have been worked out: what draws, and the records written in place once. Runs while he's on or in
/// the aftermath; with the mod off nothing of his draws (his effects were reset by the hand-back, these flags weren't).
pub(super) fn upload(control: Res<ModControl>, draws: Res<HeroDraws>, mut fx: ResMut<FxDraws>, mut input: ResMut<FxInput>, mut buffers: ResMut<Assets<ShaderBuffer>>) {
    if !control.hero && (input.shell || input.cone || input.segments != 0 || input.rings != 0) {
        input.shell = false;
        input.cone = false;
        input.segments = 0;
        input.rings = 0;
    }
    if !control.hero {
        fx.records.body = Vec4::ZERO;
    }
    let draw = draws.ready() && (input.shell || input.cone || input.segments > 0 || input.rings != 0 || input.dust.iter().any(|&q| q > 0));
    if input.draw != draw {
        input.draw = draw;
    }
    if draw || input.craters != 0 {
        // In place: the buffer stays.
        if let Some(mut b) = buffers.get_mut(&fx.buffer) {
            if let Some(d) = b.cast_slice_mut::<u8>() {
                let _ = encase::StorageBuffer::new(d).write(&*fx.records);
            }
        }
    }
}

/// The body a point of the effects is kept in: the nearest, of any kind but a star's.
fn nearest<'a>(bodies: &'a Query<&SystemBody>, pos: UPos) -> Option<&'a SystemBody> {
    bodies
        .iter()
        .filter(|b| !b.star && b.radius_m > 0.0)
        .min_by(|a, b| {
            let h = |x: &SystemBody| x.from_shape(x.orient.inverse() * pos.delta_m(x.pos)).length() - x.radius_m;
            h(a).total_cmp(&h(b))
        })
}

/// This frame's records, from the camera as the frame draws it.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_fx(
    cam: Res<FlyCamera>,
    mut hero: ResMut<Hero>,
    draws: Res<HeroDraws>,
    mut fx: ResMut<FxDraws>,
    mut input: ResMut<FxInput>,
    bodies: Query<&SystemBody>,
    views: Query<&CloudView, With<MainCamera>>,
) {
    let hero = &mut *hero;
    let fx = &mut *fx;
    let r = &mut *fx.records;
    let clock = hero.clock;
    let pos = hero.world.0;
    let rel = pos.delta_m(cam.pos).as_vec3();
    let going = hero.air_dir.as_vec3();
    let flow = hero.flow;
    let heat = hero.fx.gas_k;
    let camera_m = (hero.dist + hero.zoom).max(0.1);

    r.him = rel.extend((clock % 1000.0) as f32);
    r.along = going.extend(flow.speed as f32);
    r.sun = draws.sun_dir().extend(0.0);
    let parts = draws.parts();
    r.body = Vec4::new(if draws.ready() { parts.count as f32 } else { 0.0 }, 0.0, 0.0, 0.0);
    r.parts = parts.points;

    // The shell.
    let look_t = heat.min(tune::HEAT_LOOK_K);
    let shell = shell_at(heat);
    let stage = ((look_t - tune::SHELL_FROM_K.1) / (tune::HEAT_LOOK_K - tune::SHELL_FROM_K.1)).clamp(0.0, 1.0);
    let lerp = |(a, b): (f64, f64)| a + (b - a) * stage;
    let past = past_clamp(heat);
    let reach = tune::SHELL_CAMERA_SHARE * camera_m;
    r.shell = if shell > 0.0 { (heat_colour(look_t).lerp(Vec3::ONE, tune::SHELL_WHITE as f32) * shell as f32).extend(shell as f32) } else { Vec4::ZERO };
    r.shell_size = Vec4::new(
        (lerp(tune::SHELL_RADIUS_M) * past).min(reach) as f32,
        (lerp(tune::SHELL_LENGTH_M) * past) as f32,
        (lerp(tune::SHELL_NOSE_M) * past).min(reach) as f32,
        0.0,
    );

    // The cone.
    let pulse = tune::CONE_PULSE * (-(clock - hero.fx.pulse) / tune::CONE_PULSE_TAU_S).exp();
    let pa = flow.air.pressure_pa;
    let cone = if flow.in_air() { tune::CONE_STRENGTH * cone_at(flow.mach, flow.q, pa) + pulse * air_show(flow.q, pa) } else { 0.0 };
    r.cone = Vec4::new(cone_angle(flow.mach) as f32, tune::CONE_SLANT_M as f32, cone as f32, tune::CONE_NOSE_M as f32);

    // The clouds' buffers. (`quarter_on`'s trace is half the resolved texels' size.)
    if let Ok(v) = views.single() {
        let quarter = if crate::volumetric_clouds::quarter_on() { 2.0 } else { 1.0 };
        let (far, near) = (v.params.w.max(1.0), v.lod.w.max(1.0));
        r.cloud_px = Vec4::new(far, far * quarter, near, near * quarter);
    }

    let body = nearest(&bodies, pos);

    // The rings: riding with him at first, then where they are on the body. The body gone: gone.
    let mut ring_bits = 0u32;
    for i in 0..RINGS {
        let state = &mut hero.fx.rings[i];
        let Some(obj) = state.obj else { continue };
        let Some(b) = bodies.iter().find(|b| b.obj == obj) else {
            state.obj = None;
            continue;
        };
        let age = clock - state.born;
        let shape = state.shape;
        if age >= shape.life_s {
            state.obj = None;
            continue;
        }
        if age < shape.ride_s {
            state.local = b.orient.inverse() * pos.delta_m(b.pos);
        }
        let k = age / shape.life_s;
        let grown = 1.0 - (1.0 - k).powi(2);
        let radius = shape.start_m + (shape.end_m - shape.start_m) * grown;
        let gone = state.normal * shape.travel_m * grown;
        let fade = (1.0 - k).powi(2) * smooth(0.0, 0.03, age);
        r.rings[i] = FxRingGpu {
            centre: b.pos.offset_m(b.orient * (state.local + gone)).delta_m(cam.pos).as_vec3().extend(radius as f32),
            normal: (b.orient * state.normal).as_vec3().extend((tune::RING_BAND_M.0 + tune::RING_BAND_M.1 * radius) as f32),
            tint: Vec4::new(tune::RING_OWN as f32, tune::RING_OWN as f32, tune::RING_OWN as f32, tune::RING_STRENGTH as f32 * state.show * fade as f32),
        };
        ring_bits |= 1 << i;
    }

    // The trail: he lays it down; a point from the camera each frame, in its body's frame, with its head at him.
    if let Some(b) = body {
        hero.fx.lay_point(b.obj, b.orient.inverse() * pos.delta_m(b.pos), clock, heat);
    }
    let mut segments = 0u32;
    if hero.fx.trail_alive(clock) {
        if let Some(b) = hero.fx.trail_body.and_then(|o| bodies.iter().find(|b| b.obj == o)) {
            segments = hero.fx.trail_records(clock, |local| b.pos.offset_m(b.orient * local).delta_m(cam.pos).as_vec3(), rel, &mut r.trail);
        }
    }

    let (want_shell, want_cone) = (shell > 0.0, cone > 1e-3);
    if input.shell != want_shell || input.cone != want_cone || input.segments != segments || input.rings != ring_bits {
        input.shell = want_shell;
        input.cone = want_cone;
        input.segments = segments;
        input.rings = ring_bits;
    }
    // (What draws, and the buffer's write: `upload`, after the impacts.)
}

fn queue_pipelines(mut pass: ResMut<FxPass>, cache: Res<PipelineCache>, assets: Res<AssetServer>, device: Res<bevy::render::renderer::RenderDevice>) {
    if pass.layouts.is_some() {
        return;
    }
    // (Zeroed: no burst's seed.)
    pass.tint = Some(device.create_buffer(&BufferDescriptor { label: Some("hero_fx_dust_tint"), size: (DUST_BURSTS * 16) as u64, usage: BufferUsages::STORAGE, mapped_at_creation: false }));
    pass.sampler = Some(device.create_sampler(&SamplerDescriptor {
        label: Some("hero_fx_sprite"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    }));
    let shader: Handle<Shader> = assets.load("shaders/hero_fx.wesl");
    let layouts = [false, true].map(|msaa| {
        let f = TextureSampleType::Float { filterable: false };
        BindGroupLayoutDescriptor::new(
            "hero_fx",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    (0, uniform_buffer::<ViewUniform>(true)),
                    (1, storage_buffer_read_only_sized(false, None)),
                    (2, storage_buffer_read_only_sized(false, None)),
                    (3, if msaa { texture_depth_2d_multisampled() } else { texture_depth_2d() }),
                    (4, texture_2d(f)),
                    (5, texture_2d(f)),
                    (6, texture_2d(f)),
                    (7, texture_2d(f)),
                    (8, texture_2d(TextureSampleType::Float { filterable: true })),
                    (9, sampler(SamplerBindingType::Filtering)),
                    (10, storage_buffer_read_only_sized(false, None)),
                ),
            ),
        )
    });
    let tint_layouts = [false, true].map(|msaa| {
        BindGroupLayoutDescriptor::new(
            "hero_fx_dust_tint",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::COMPUTE,
                (
                    (0, uniform_buffer::<ViewUniform>(true)),
                    (1, storage_buffer_read_only_sized(false, None)),
                    (3, if msaa { texture_depth_2d_multisampled() } else { texture_depth_2d() }),
                    (11, texture_2d(TextureSampleType::Float { filterable: false })),
                    (12, storage_buffer_sized(false, None)),
                ),
            ),
        )
    });
    let add = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    let keep = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    // The dust over the scene, premultiplied; the craters times it.
    let over = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add };
    let times = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::Src, operation: BlendOperation::Add };
    for (i, def) in ["NO_MSAA", "MSAA"].into_iter().enumerate() {
        let pipeline = |label: &'static str, (vertex, fragment): (&'static str, &'static str), color: BlendComponent| RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layouts[i].clone()],
            vertex: VertexState { shader: shader.clone(), shader_defs: vec![def.into()], entry_point: Some(vertex.into()), ..default() },
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: vec![def.into()],
                entry_point: Some(fragment.into()),
                targets: vec![Some(ColorTargetState { format: HDR_FORMAT, blend: Some(BlendState { color, alpha: keep }), write_mask: ColorWrites::ALL })],
                ..default()
            }),
            primitive: PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: None, ..default() },
            ..default()
        };
        pass.pipelines[i] = Some(cache.queue_render_pipeline(pipeline("hero_fx", ("vertex", "fragment"), add)));
        pass.dust[i] = Some(cache.queue_render_pipeline(pipeline("hero_fx_dust", ("dust_vertex", "dust_fragment"), over)));
        pass.decals[i] = Some(cache.queue_render_pipeline(pipeline("hero_fx_decals", ("decal_vertex", "decal_fragment"), times)));
        pass.tint_pipelines[i] = Some(cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some("hero_fx_dust_tint".into()),
            layout: vec![tint_layouts[i].clone()],
            shader: shader.clone(),
            shader_defs: vec![def.into()],
            entry_point: Some("dust_tint".into()),
            ..default()
        }));
    }
    pass.layouts = Some(layouts);
    pass.tint_layouts = Some(tint_layouts);
}

/// Each burst's colour: the lit ground on a ring round where it rose, read from the scene after the main pass, before the craters darken
/// it (`hero_fx.wesl` `dust_tint`): 64 invocations, one workgroup.
#[allow(clippy::type_complexity)]
fn fx_dust_tint(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &ViewUniformOffset, &Msaa), With<CloudView>>,
    pass: Res<FxPass>,
    input: Res<FxInput>,
    cache: Res<PipelineCache>,
    uniforms: Res<ViewUniforms>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mut ctx: RenderContext,
) {
    let (target, depth, offset, msaa) = view.into_inner();
    let m = (msaa.samples() > 1) as usize;
    let (Some(layouts), Some(pipeline), Some(view_binding), Some(tint)) =
        (pass.tint_layouts.as_ref(), pass.tint_pipelines[m].and_then(|id| cache.get_compute_pipeline(id)), uniforms.uniforms.binding(), pass.tint.as_ref())
    else {
        return;
    };
    let Some(records) = buffers.get(&input.records) else { return };
    if target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let device = ctx.render_device().clone();
    let depth_view = depth.texture().create_view(&TextureViewDescriptor { aspect: TextureAspect::DepthOnly, ..default() });
    let group = device.create_bind_group(
        "hero_fx_dust_tint",
        &cache.get_bind_group_layout(&layouts[m]),
        &BindGroupEntries::with_indices((
            (0, view_binding),
            (1, records.buffer.as_entire_binding()),
            (3, &depth_view),
            (11, target.main_texture_view()),
            (12, tint.as_entire_binding()),
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "hero_fx_dust_tint");
    let mut compute = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("hero_fx_dust_tint"), ..default() });
    compute.set_pipeline(pipeline);
    compute.set_bind_group(0, &group, &[offset.offset]);
    compute.dispatch_workgroups(1, 1, 1);
    drop(compute);
    span.end(ctx.command_encoder());
}

/// The passes' one bind group: the clouds' buffers are `drawn`, or the fallback where they didn't draw (the craters' pass, before them).
#[allow(clippy::too_many_arguments)]
fn bind_group(
    device: &bevy::render::renderer::RenderDevice,
    cache: &PipelineCache,
    layout: &BindGroupLayoutDescriptor,
    view_binding: BindingResource,
    (records, light): (&GpuShaderBuffer, &GpuShaderBuffer),
    depth: &TextureView,
    clouds: [(&TextureView, &TextureView); 2],
    (sprite, sampler): (&TextureView, &Sampler),
    tint: &Buffer,
) -> BindGroup {
    device.create_bind_group(
        "hero_fx",
        &cache.get_bind_group_layout(layout),
        &BindGroupEntries::with_indices((
            (0, view_binding),
            (1, records.buffer.as_entire_binding()),
            (2, light.buffer.as_entire_binding()),
            (3, depth),
            (4, clouds[0].0),
            (5, clouds[0].1),
            (6, clouds[1].0),
            (7, clouds[1].1),
            (8, sprite),
            (9, sampler),
            (10, tint.as_entire_binding()),
        )),
    )
}

/// The craters: each multiplies what the main pass drew where its box covers the screen, before the clouds (`decal_fragment`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn fx_decals(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &bevy::render::camera::ExtractedCamera, &ViewUniformOffset, &Msaa, Option<&MainPassResolutionOverride>), With<CloudView>>,
    pass: Res<FxPass>,
    input: Res<FxInput>,
    cache: Res<PipelineCache>,
    uniforms: Res<ViewUniforms>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    mut ctx: RenderContext,
) {
    let (target, depth, camera, offset, msaa, low) = view.into_inner();
    let m = (msaa.samples() > 1) as usize;
    let (Some(layouts), Some(pipeline), Some(view_binding), Some(sampler), Some(tint)) =
        (pass.layouts.as_ref(), pass.decals[m].and_then(|id| cache.get_render_pipeline(id)), uniforms.uniforms.binding(), pass.sampler.as_ref(), pass.tint.as_ref())
    else {
        return;
    };
    let (Some(records), Some(light)) = (buffers.get(&input.records), buffers.get(&input.light)) else { return };
    let Some(solid) = low.map(|l| l.0).or(camera.physical_viewport_size).filter(|s| s.x > 0 && s.y > 0) else { return };
    if target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let device = ctx.render_device().clone();
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "fx_decals");
    let depth_view = depth.texture().create_view(&TextureViewDescriptor { aspect: TextureAspect::DepthOnly, ..default() });
    let none = &fallback.d2.texture_view;
    let sprite = images.get(&input.sprite).map_or(none, |i| &i.texture_view);
    let group = bind_group(&device, &cache, &layouts[m], view_binding, (records, light), &depth_view, [(none, none), (none, none)], (sprite, sampler), tint);
    let scene = RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } };
    let encoder = ctx.command_encoder();
    let mut p = encoder.begin_render_pass(&RenderPassDescriptor { label: Some("fx_decals"), color_attachments: &[Some(scene)], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
    p.set_viewport(0.0, 0.0, solid.x as f32, solid.y as f32, 0.0, 1.0);
    p.set_pipeline(pipeline);
    p.set_bind_group(0, &group, &[offset.offset]);
    for i in 0..CRATERS as u32 {
        if input.craters & (1 << i) != 0 {
            p.draw(0..DECAL_VERTICES, i..i + 1);
        }
    }
    drop(p);
    span.end(ctx.command_encoder());
}

/// The effects, into the solids' corner of the main texture. With the clouds off, or before they've drawn, the fallback images dim nothing.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn fx_pass(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &bevy::render::camera::ExtractedCamera, &ViewUniformOffset, &Msaa, Option<&MainPassResolutionOverride>, Option<&CloudTextures>), With<CloudView>>,
    pass: Res<FxPass>,
    input: Res<FxInput>,
    cache: Res<PipelineCache>,
    uniforms: Res<ViewUniforms>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    (history, near_history, frame): (Res<CloudHistory>, Res<NearHistory>, Res<bevy::diagnostic::FrameCount>),
    mut ctx: RenderContext,
) {
    let (target, depth, camera, offset, msaa, low, clouds) = view.into_inner();
    let m = (msaa.samples() > 1) as usize;
    let (Some(layouts), Some(pipeline), Some(view_binding), Some(sampler), Some(tint)) =
        (pass.layouts.as_ref(), pass.pipelines[m].and_then(|id| cache.get_render_pipeline(id)), uniforms.uniforms.binding(), pass.sampler.as_ref(), pass.tint.as_ref())
    else {
        return;
    };
    let (Some(records), Some(light)) = (buffers.get(&input.records), buffers.get(&input.light)) else { return };
    // (The dust's pipeline may still be building: then it waits, the rest draws.)
    let dust_pipeline = pass.dust[m].and_then(|id| cache.get_render_pipeline(id));
    let Some(solid) = low.map(|l| l.0).or(camera.physical_viewport_size).filter(|s| s.x > 0 && s.y > 0) else { return };
    if target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let device = ctx.render_device().clone();
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "hero_fx");
    let depth_view = depth.texture().create_view(&TextureViewDescriptor { aspect: TextureAspect::DepthOnly, ..default() });
    let none = &fallback.d2.texture_view;
    let [far, near] = clouds.map_or([None, None], |t| t.drawn(&history, &near_history, frame.0));
    let sprite = images.get(&input.sprite).map_or(none, |i| &i.texture_view);
    let group = bind_group(&device, &cache, &layouts[m], view_binding, (records, light), &depth_view, [far.unwrap_or((none, none)), near.unwrap_or((none, none))], (sprite, sampler), tint);
    let scene = RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } };
    let encoder = ctx.command_encoder();
    let mut p = encoder.begin_render_pass(&RenderPassDescriptor { label: Some("hero_fx"), color_attachments: &[Some(scene)], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
    p.set_viewport(0.0, 0.0, solid.x as f32, solid.y as f32, 0.0, 1.0);
    p.set_bind_group(0, &group, &[offset.offset]);
    // The dust first, blended over the scene: the glowing effects add on top of it.
    if let Some(dust) = dust_pipeline.filter(|_| input.dust.iter().any(|&q| q > 0)) {
        p.set_pipeline(dust);
        for (i, &quads) in input.dust.iter().enumerate() {
            if quads > 0 {
                p.draw(0..6 * quads as u32, i as u32..i as u32 + 1);
            }
        }
    }
    p.set_pipeline(pipeline);
    if input.segments > 0 {
        p.draw(0..6 * input.segments, FX_TRAIL..FX_TRAIL + 1);
    }
    if input.cone {
        p.draw(0..6 * AROUND * CONE_BANDS, FX_CONE..FX_CONE + 1);
    }
    if input.shell {
        p.draw(0..6 * AROUND * SHELL_RINGS, FX_SHELL..FX_SHELL + 1);
    }
    for i in 0..RINGS as u32 {
        if input.rings & (1 << i) != 0 {
            p.draw(0..6, FX_RING + i..FX_RING + i + 1);
        }
    }
    drop(p);
    span.end(ctx.command_encoder());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glow_stays_within_its_limits_and_is_never_t4() {
        assert_eq!(glow_at(500.0), 0.0);
        assert!((glow_at(800.0) - tune::GLOW_HDR.0).abs() < 1e-6);
        assert!((glow_at(10_000.0) - tune::GLOW_HDR.1).abs() < 1e-6);
        // The clamp: 23,300 K (30 km/s at sea level) looks like 10,000 K.
        assert_eq!(glow_at(23_300.0), glow_at(10_000.0));
        // T^4 would be 400 times over 800 to 3,600 K.
        assert!(glow_at(3600.0) / glow_at(800.0) < 12.0);
        let mut last = 0.0;
        for t in (600..12000).step_by(100) {
            let g = glow_at(t as f64);
            assert!(g >= last && g <= tune::GLOW_HDR.1 + 1e-9, "{t}: {g}");
            last = g;
        }
    }

    /// F9's stages, in order: no shell under 3,000 K, all of it at 3,500; the trail at 6,000.
    #[test]
    fn the_stages_come_in_where_the_spec_says() {
        assert!(glow_at(800.0) > 0.0);
        assert_eq!(shell_at(2999.0), 0.0);
        assert!(shell_at(3500.0) > 0.0);
        assert_eq!(trail_at(4999.0), 0.0);
        assert!((trail_at(6000.0) - tune::TRAIL_HDR.0).abs() < 1e-6);
        assert!(shell_at(23_000.0) <= tune::SHELL_HDR.1 + 1e-9 && trail_at(23_000.0) <= tune::TRAIL_HDR.1 + 1e-9);
        // Past the clamp the size grows, with a limit.
        assert_eq!(past_clamp(9000.0), 1.0);
        assert!(past_clamp(20_000.0) > 1.2);
        assert_eq!(past_clamp(1e6), tune::HEAT_PAST_MAX);
    }

    /// `cargo test -p se_engine --release print_heat_colours -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_heat_colours() {
        for t in [600.0, 800.0, 1000.0, 1500.0, 2000.0, 2500.0, 3000.0, 3500.0, 4500.0, 6000.0, 8000.0, 10_000.0] {
            let [r, g, b] = crate::blackholes::planck_rgb(t);
            let m = r.max(g).max(b);
            println!("{t}: black body {:.3} {:.3} {:.3}, the look {:.3}", r / m, g / m, b / m, heat_colour(t));
        }
    }

    #[test]
    fn the_colours_go_from_orange_through_white_to_blue() {
        let red_over_blue = |t: f64| {
            let c = heat_colour(t);
            c.x / c.z.max(1e-3)
        };
        let dull = heat_colour(800.0);
        assert!(dull.x == 1.0 && dull.y >= 0.2 && dull.y < 0.4 && dull.z < 0.05, "{dull}");
        let yellow = heat_colour(2000.0);
        assert!(yellow.y > dull.y && yellow.z < 0.2, "{yellow}");
        assert!(red_over_blue(800.0) > red_over_blue(2000.0) && red_over_blue(2000.0) > red_over_blue(3500.0) && red_over_blue(3500.0) > red_over_blue(6000.0));
        let hot = heat_colour(10_000.0);
        assert!(hot.z == 1.0 && hot.x < 1.0, "{hot}");
    }

    #[test]
    fn the_bar_fills_with_the_heat() {
        assert_eq!(heat_bar(300.0), 0.0);
        assert_eq!(heat_bar(10_000.0), 1.0);
        assert_eq!(heat_bar(30_000.0), 1.0);
        assert!(heat_bar(800.0) > 0.1 && heat_bar(800.0) < heat_bar(2000.0) && heat_bar(3500.0) < heat_bar(6000.0));
    }

    #[test]
    fn the_cone_is_seen_where_the_spec_says() {
        let (q, p) = (6e4, 1e5);
        assert_eq!(cone_at(0.9, q, p), 0.0);
        assert!(cone_at(0.97, q, p) > 0.0);
        assert!(cone_at(1.2, q, p) == air_show(q, p));
        assert_eq!(cone_at(1.6, q, p), 0.0);
        // Half-angle asin(1/M).
        assert!((cone_angle(2.0) - 30f64.to_radians()).abs() < 1e-9);
        assert!((cone_angle(0.97) - 90f64.to_radians()).abs() < 1e-9);
        // Thin air shows less, but shows it: Earth at 20 km, about 5,500 Pa.
        assert!(cone_at(1.2, 500.0, 5500.0) < cone_at(1.2, q, p) && cone_at(1.2, 500.0, 5500.0) > 0.3);
        // Where the barrier counts no air, none: Earth at 150 km (0.0013 Pa), Pluto (1 Pa), a 1 mbar world; between 100 and 150 Pa
        // it fades in.
        for (q, p) in [(1.2e-3, 1.3e-3), (0.7, 1.0), (70.0, 100.0)] {
            assert_eq!(cone_at(1.2, q, p), 0.0, "{p} Pa");
            assert_eq!(air_show(q, p), 0.0, "{p} Pa");
        }
        assert!(cone_at(1.2, 30.0, 125.0) > 0.0 && cone_at(1.2, 30.0, 125.0) < cone_at(1.2, 30.0, 150.0));
        assert_eq!(cone_at(1.2, 30.0, 150.0), tune::AIR_SHOW.0);
    }

    /// The same at near vacuum in the game's own air: Earth from 100 km up, at Mach 1.2 by its own sound speed.
    #[test]
    fn no_cone_above_the_boom() {
        let air = super::super::air::AirModel::from_params(se_core::ship::aero::AtmoParams { p0_atm: 1.0, t0_k: 281.57, molar_mass: 28.92, gravity: 9.798, adiabat: 0.28, height_m: 0.0 });
        for km in [100.0, 150.0, 200.0] {
            let a = air.at(km * 1e3);
            let f = air.flow(km * 1e3, 1.2 * a.sound_speed);
            assert!(f.in_air(), "{km} km");
            assert_eq!(cone_at(f.mach, f.q, f.air.pressure_pa), 0.0, "{km} km: {f:?}");
        }
        let low = air.flow(0.0, 1.2 * air.at(0.0).sound_speed);
        assert!((cone_at(low.mach, low.q, low.air.pressure_pa) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_new_ring_takes_the_oldest_ones_place() {
        let mut s = FxState::default();
        for i in 0..RINGS {
            s.add_ring(ObjectRef::Body(0), DVec3::X, DVec3::Y, i as f64, 1.0);
        }
        assert!(s.rings.iter().all(|r| r.obj.is_some()));
        s.add_ring(ObjectRef::Body(1), DVec3::X, DVec3::Y, 9.0, 1.0);
        assert_eq!(s.rings.iter().map(|r| r.born).fold(f64::INFINITY, f64::min), 1.0);
    }

    #[test]
    fn the_trail_holds_its_spacing_and_never_grows() {
        let mut s = FxState::default();
        let obj = ObjectRef::Body(0);
        // At 3 m a frame: a point every other frame, at least 4 m apart.
        for i in 0..100 {
            s.lay_point(obj, DVec3::X * 3.0 * i as f64, i as f64 / 60.0, 8000.0);
        }
        assert_eq!(s.count, 50);
        assert!((s.trail[s.head].local - s.trail[(s.head + s.trail.len() - 1) % s.trail.len()].local).length() >= tune::TRAIL_SPACING_M);
        // At 500 m a frame: every frame, and the ring stays full.
        for i in 0..1000 {
            s.lay_point(obj, DVec3::X * 500.0 * i as f64, i as f64 / 60.0, 8000.0);
        }
        assert_eq!(s.count, s.trail.len());
        // Too cool: nothing laid, and what's there dies. Then a new stretch, cut from the old.
        let last = s.trail[s.head].local;
        s.lay_point(obj, DVec3::Y * 1e6, 20.0, 3000.0);
        assert_eq!(s.trail[s.head].local, last);
        assert!(!s.trail_alive(40.0));
        s.lay_point(obj, DVec3::Y * 2e6, 40.0, 8000.0);
        assert!(s.trail[s.head].cut);
        s.lay_point(obj, DVec3::Y * 3e6, 40.1, 8000.0);
        assert!(!s.trail[s.head].cut);
        // Another body: it starts anew.
        s.lay_point(ObjectRef::Body(1), DVec3::Y, 41.0, 8000.0);
        assert_eq!(s.count, 1);
    }

    /// Flying a straight line on from `x` (m) at `metres` a frame, `hz` frames a second, at `heat_k`, from `start` (s), `secs` long: the
    /// last frame's time and place.
    fn fly_line(s: &mut FxState, (start, x): (f64, f64), secs: f64, hz: f64, metres: f64, heat_k: f64) -> (f64, f64) {
        let mut end = (start, x);
        for i in 0..(secs * hz) as usize {
            end = (start + i as f64 / hz, x + metres * i as f64);
            s.lay_point(ObjectRef::Body(0), DVec3::X * end.1, end.0, heat_k);
        }
        end
    }

    fn records(s: &FxState, clock: f64) -> (u32, Box<[FxPointGpu; TRAIL_POINTS]>) {
        let mut out = Box::new([FxPointGpu::default(); TRAIL_POINTS]);
        let n = if s.trail_alive(clock) { s.trail_records(clock, |l| l.as_vec3(), Vec3::ZERO, &mut out) } else { 0 };
        (n, out)
    }

    /// He lets go at 10,000 K: the points laid as he cools live 0.8 s, the hot ones before them 2 s. When the cool ones die, the hot ones
    /// stay, at their own light, until each dies in turn.
    #[test]
    fn the_trail_dies_point_by_point() {
        let mut s = FxState::default();
        let (t, x) = fly_line(&mut s, (0.0, 0.0), 1.0, 60.0, 160.0, 10_000.0);
        let (end, _) = fly_line(&mut s, (t + 1.0 / 60.0, x + 160.0), 0.3, 60.0, 100.0, 5_500.0);
        // The cool ones dead, the hot ones not.
        let at = end + 0.9;
        let (n, out) = records(&s, at);
        assert!(n > 10, "{n} segments");
        let lit: Vec<f32> = out[..=n as usize].iter().map(|p| p.light.truncate().max_element()).collect();
        assert!(lit.iter().any(|&l| l > 0.3), "{lit:?}");
        assert_eq!(lit[1], 0.0, "the newest point is dead");
        // The end: no light.
        assert_eq!(*lit.last().unwrap(), 0.0);
        // Each in turn, and then none.
        let (later, _) = records(&s, at + 0.6);
        assert!(later > 0 && later < n, "{later} of {n}");
        assert_eq!(records(&s, end + 2.1).0, 0);
    }

    /// The ring holds a point's whole life at any frame rate: at 240 Hz at 10,000 K the oldest point kept is 2 s old, and past the clamp
    /// (life 3.9 s) at 165 Hz too. Its end is at no light.
    #[test]
    fn the_trail_is_as_long_at_any_frame_rate() {
        for (hz, heat) in [(60.0, 10_000.0), (240.0, 10_000.0), (165.0, 23_300.0)] {
            let mut s = FxState::default();
            let (end, _) = fly_line(&mut s, (0.0, 0.0), 8.0, hz, 30e3 / hz, heat);
            let life = s.trail[s.head].life as f64;
            let oldest = (0..s.count).map(|k| s.trail[(s.head + s.trail.len() - k) % s.trail.len()].born).fold(f64::INFINITY, f64::min);
            assert!(end - oldest >= life * 0.99, "{hz} Hz: holds {} s of a {life} s life", end - oldest);
            let (n, out) = records(&s, end);
            assert_eq!(out[n as usize].light.truncate(), Vec3::ZERO, "{hz} Hz");
        }
    }

    /// At a stop his skin stays hot for seconds, the gas round him doesn't.
    #[test]
    fn the_gas_goes_as_the_air_does() {
        let mut s = FxState::default();
        s.ease_gas(9000.0, 3000.0, 1.0 / 60.0);
        assert_eq!(s.gas_k, 3000.0, "it comes no faster than his heat");
        s.ease_gas(9000.0, 8000.0, 1.0 / 60.0);
        assert_eq!(s.gas_k, 8000.0);
        for _ in 0..60 {
            s.ease_gas(0.0, 7000.0, 1.0 / 60.0);
        }
        assert!(s.gas_k < 8000.0 * 0.01, "{}", s.gas_k);
    }

    #[test]
    fn a_go_to_through_the_air_stays_within_the_look() {
        let (glow, lee, _) = glow_record(26_000.0, DVec3::X);
        let (same, ..) = glow_record(tune::HEAT_LOOK_K, DVec3::X);
        assert_eq!(glow, same);
        assert!(glow.truncate().max_element() <= tune::GLOW_HDR.1 as f32 + 1e-3);
        assert!(lee.truncate().max_element() < glow.truncate().max_element());
    }
}
