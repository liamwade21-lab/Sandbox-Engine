//! The active body's weather, live (docs/spec-volumetric-clouds-weather.md, step 6).
//!
//! Two parts make the weather cube the clouds read (`CloudShadowAssets::weather`: the cloud pass's
//! `weather_at`, the ground's far cloud shadows):
//! - the derived weather (`se_core::weather`): maps for dates 10 simulated minutes apart, made ahead
//!   on a worker thread (at once on arrival and after a date jump), blended by the date;
//! - a coarse simulation on the GPU (`weather_sim.wesl`, six faces of 128 x 128 cells): thunderstorm
//!   clusters that start where the derived weather's convection allows, grow, leave cooled air that
//!   keeps new ones away for a while, spread anvils that linger, carried by the derived winds,
//!   stepped every 5 simulated minutes.
//!
//! The same output pass writes a second cube (fog, dust), and a last pass picks the lightning
//! flashes of the moment where the storms are (read by the cloud pass and the effects pass).
//!
//! The simulation runs live for the active body within 20 radii at up to x1000. Faster, farther, or
//! with time running backwards it follows the state it would be seeded with at the date; paused it
//! stands still; a new body or a date jump seeds it again at once (F4). Near the camera the clouds'
//! shapes drift with the local wind.

use super::*;
use se_core::weather::{Climate, MAP_H, MAP_W};

/// The simulation's cells per cube face (about 40 km on Earth; 128 showed as squares from orbit).
const SIM_SIZE: u32 = 256;
/// Its step (simulated seconds).
const STEP_S: f64 = 300.0;
/// At most this many steps a frame; more (after a stall) are skipped.
const MAX_STEPS: u32 = 4;
/// The derived maps' spacing up to x1000 (simulated seconds); faster, half a real second's worth.
const MAP_SLOT_S: f64 = 600.0;
/// Live within this many of the body's radii, at rates up to `LIVE_RATE`.
const LIVE_RADII: f64 = 20.0;
const LIVE_RATE: f64 = 1000.0;
/// Following the seed state: its time constant (real seconds), so crossing x1000 blends.
const FOLLOW_S: f64 = 0.15;
/// The seed pattern's time slices (simulated seconds).
const SEED_SLICE_S: f64 = 3.0 * 3600.0;
const DAY_S: f64 = 86_400.0;
const STATE_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// The effects' clock wraps at this (s), so it stays exact in f32.
const FX_WRAP_S: f64 = 10_000.0;
/// Lightning flash slots.
const FLASHES: u64 = 8;
/// Near flashes are looked for within this distance of the camera (m).
const FLASH_NEAR_M: f64 = 300_000.0;

/// The derived maps' image: two dates, each its weather and its wind layer (`weather_map`).
pub(super) fn derived_image() -> Image {
    let mut image = Image::new(
        Extent3d { width: MAP_W as u32, height: MAP_H as u32, depth_or_array_layers: 4 },
        TextureDimension::D2,
        [128u8, 128, 0, 0].repeat(MAP_W * MAP_H * 4),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// The weather cube (SE's tile faces, as the cover map), written by the simulation's output pass:
/// R the cover multiplier / 2, G the cover offset x 0.5 + 0.5, B convection, A precipitation.
pub(super) fn weather_cube() -> Image {
    let mut image = Image::new_uninit(
        Extent3d { width: SIM_SIZE, height: SIM_SIZE, depth_or_array_layers: 6 },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING;
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// A body's climate for `se_core::weather`: its spin from the catalog (signed, retrograde negative),
/// how wet it is (its ocean), whether it is tidally locked.
fn climate(data: &crate::data::SeData, body: &SystemBody, terrain: &Terrain) -> Climate {
    use se_core::rotation::RotationModel;
    let day_hours = match body.obj {
        se_core::catalog::ObjectRef::Body(i) => match &data.catalog.bodies[i as usize].rotation {
            RotationModel::Iau(r) if r.rotation_rate != 0.0 => 360.0 / r.rotation_rate * 24.0,
            RotationModel::Simple(s) => s.period_hours,
            _ => 0.0,
        },
        _ => 24.0,
    };
    let humidity = if terrain.has_ocean { 0.4 + 0.6 * terrain.proc.ocean_fraction.clamp(0.0, 1.0) } else { 0.25 };
    Climate { seed: terrain.proc.seed, day_hours, radius_m: terrain.radius, humidity, tidal_lock: terrain.tidal_lock }
}

/// The derived maps kept around the date: made ahead in time's direction, the pair around the date
/// uploaded.
#[derive(Default)]
pub(super) struct DerivedMaps {
    body: Option<Entity>,
    have: Vec<(f64, Vec<u8>)>,
    job: Option<(f64, Task<Vec<u8>>)>,
    /// The dates of the pair in the image.
    shown: Option<(f64, f64)>,
}

impl DerivedMaps {
    /// No weather: the neutral maps, put in `image` once for the body.
    fn neutral(&mut self, e: Entity, images: &mut Assets<Image>, image: &Handle<Image>) -> f32 {
        if self.body != Some(e) || self.shown.is_some() {
            *self = Self { body: Some(e), ..default() };
            if let Some(mut img) = images.get_mut(image) {
                img.data = Some([128u8, 128, 0, 0].repeat(MAP_W * MAP_H * 4));
            }
        }
        0.0
    }

    /// Keeps the maps for the date and puts the pair around it in `image`; their blend at `jd`.
    #[allow(clippy::too_many_arguments)]
    fn update(&mut self, e: Entity, c: &Climate, sun: DVec3, jd: f64, rate: f64, images: &mut Assets<Image>, image: &Handle<Image>) -> f32 {
        if self.body != Some(e) {
            *self = Self { body: Some(e), ..default() };
        }
        let slot = if rate.abs() <= LIVE_RATE { MAP_SLOT_S } else { (rate.abs() * 0.5 / MAP_SLOT_S).ceil() * MAP_SLOT_S } / DAY_S;
        let lo = (jd / slot).floor() * slot;
        let hi = lo + slot;
        if let Some((t, task)) = self.job.as_mut()
            && let Some(map) = check_ready(task)
        {
            let t = *t;
            self.job = None;
            self.have.push((t, map));
        }
        let sun_at = |t: f64| se_core::weather::sun_after(c, sun, (t - jd) * DAY_S);
        // On arrival or after a date jump, the two around the date at once: the weather shows in this
        // frame.
        if !self.have.iter().any(|(t, _)| (t - jd).abs() < 2.0 * slot) {
            let start = std::time::Instant::now();
            self.job = None;
            self.have = [lo, hi].map(|t| (t, se_core::weather::weather_map(c, sun_at(t), t))).into();
            info!(
                "volumetric clouds: weather on body {e:?} ({} cells, day {:.1} h, humidity {:.2}), maps in {:.2} ms",
                se_core::weather::cells(c),
                c.day_hours,
                c.humidity,
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        let by_date = |a: &&(f64, Vec<u8>), b: &&(f64, Vec<u8>)| a.0.total_cmp(&b.0);
        let before = self.have.iter().filter(|(t, _)| *t <= jd).max_by(by_date);
        let after = self.have.iter().filter(|(t, _)| *t > jd).min_by(by_date);
        let Some((a, b)) = (match (before, after) {
            (Some(a), Some(b)) => Some((a, b)),
            (Some(x), None) | (None, Some(x)) => Some((x, x)),
            (None, None) => None,
        }) else {
            return 0.0;
        };
        let blend = if b.0 > a.0 { ((jd - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let pair = (a.0, b.0);
        if self.shown != Some(pair) {
            if let Some(mut img) = images.get_mut(image) {
                img.data = Some([a.1.as_slice(), b.1.as_slice()].concat());
            }
            self.shown = Some(pair);
        }
        if self.job.is_none() {
            let next = if rate < 0.0 { [hi, lo, lo - slot] } else { [lo, hi, hi + slot] };
            if let Some(t) = next.into_iter().find(|t| !self.have.iter().any(|(h, _)| (h - t).abs() < 1e-7)) {
                let (c, s) = (*c, sun_at(t));
                self.job = Some((t, AsyncComputeTaskPool::get().spawn(async move { se_core::weather::weather_map(&c, s, t) })));
            }
        }
        self.have.retain(|(t, _)| (*t > lo - slot - 1e-7 && *t < hi + slot + 1e-7) || *t == pair.0 || *t == pair.1);
        blend
    }
}

/// What the simulation does in a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum SimMode {
    #[default]
    Off,
    /// Paused: nothing changes.
    Frozen,
    /// Stepped with the simulated time.
    Live,
    /// Pulled towards the seed state at the date by this weight (fast or backwards time, far away).
    Follow(f32),
}

/// The simulation's clock, on the main world's side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct SimClock {
    key: Option<u64>,
    last_jd: f64,
    /// Bumped on every seeding at once (a new body or setup, a date jump).
    seed_id: u64,
    /// The date the state was seeded for (on the step grid) and the steps since.
    t0: f64,
    target: u32,
}

impl SimClock {
    /// One frame: `key` the body's setup (None: no simulation), `jd` the date, `rate` and `paused` the
    /// time controls, `dt` the frame's real seconds, `near` the camera within the live range.
    fn advance(&mut self, key: Option<u64>, jd: f64, rate: f64, paused: bool, dt: f64, near: bool) -> SimMode {
        let Some(k) = key else {
            *self = Self { seed_id: self.seed_id, ..default() };
            return SimMode::Off;
        };
        // A date that moved otherwise than the clock runs is a jump.
        let expected = if paused { 0.0 } else { rate * dt };
        let moved = (jd - self.last_jd) * DAY_S;
        let jumped = self.key != Some(k) || (moved - expected).abs() > (2.0 * 60.0f64).max(0.5 * expected.abs());
        self.key = Some(k);
        self.last_jd = jd;
        let grid = (jd * DAY_S / STEP_S).floor() * STEP_S / DAY_S;
        if jumped {
            self.seed_id += 1;
            self.t0 = grid;
            self.target = 0;
        }
        if paused {
            return SimMode::Frozen;
        }
        if !(0.0..=LIVE_RATE).contains(&rate) || !near {
            self.t0 = grid;
            self.target = 0;
            return SimMode::Follow((1.0 - (-dt / FOLLOW_S).exp()) as f32);
        }
        self.target = ((jd - self.t0) * DAY_S / STEP_S).floor().max(0.0) as u32;
        SimMode::Live
    }

    /// How far the date is into the current step (0..1).
    fn frac(&self, jd: f64) -> f32 {
        ((jd - self.t0) * DAY_S / STEP_S - self.target as f64).clamp(0.0, 1.0) as f32
    }
}

/// The noise's drift with the wind near the camera (sphere space, m, wrapped to each noise's period).
#[derive(Default)]
pub(super) struct Drift {
    key: Option<u64>,
    shape: DVec3,
    detail: DVec3,
    /// The effects' clock (s): lightning, falling rain and snow. It runs at most as fast as real
    /// time, stands still when paused, and starts from the date after a jump.
    fx: f64,
}

/// The simulation passes' inputs. Layout must match `weather_sim.wesl` `SimUniform`.
#[derive(Clone, Copy, ShaderType, Default, Debug, PartialEq)]
pub(super) struct SimUniform {
    /// x: the derived maps' blend, y: the body's radius (m), z: humidity, w: the step (s).
    maps: Vec4,
    /// x: the step's index (mod 2^20, per step), y: the body's seed fraction, z: the trigger noise's
    /// frequency (per radius), w: the seed's weight (per pass).
    step: Vec4,
    /// x, y: the seed pattern's slice (mod 4096) and how far into it, z: the output's interpolation
    /// from the previous state to the current one, w: 1 to use the cells (0: the derived weather
    /// alone, `SE_WEATHER_SIM=0`, for comparisons).
    seed: Vec4,
    /// x: the effects' clock (s, wrapped), y, z: the heights lightning flashes between (m above the
    /// radius), w: 1 with lightning on.
    flash: Vec4,
    /// xyz: towards the camera (sphere space, unit), w: the angle around it near flashes are looked
    /// for in (rad).
    cam: Vec4,
}

/// `SE_WEATHER_SIM=0` leaves the simulation's cells out of the weather cube (comparisons).
fn cells_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_WEATHER_SIM").map_or(true, |v| v.trim() != "0"))
}

/// `SE_WEATHER_NEUTRAL=1` turns the derived weather off (every body's cover as its own map or
/// generator makes it; comparisons with SE's flat clouds).
fn neutral_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_WEATHER_NEUTRAL").is_ok_and(|v| v.trim() == "1"))
}

/// `SE_WEATHER_DEBUG=1` shows the weather as a map at the top of the screen (tuning).
fn debug_map() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_WEATHER_DEBUG").is_ok_and(|v| v.trim() == "1"))
}

/// The frame's simulation for the render world.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub(super) struct SimFrame {
    /// The body's setup (`CloudBody::id`) and its seedings at once.
    body_id: u64,
    seed_id: u64,
    mode: SimMode,
    /// Steps since the seed, and the seed's step on the date grid (mod 2^20).
    target: u32,
    base: u32,
    uniform: SimUniform,
}

/// The active body's derived maps, the simulation's clock, and the clouds' drift near the camera.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update_weather(
    (time, real, data, cam, switches): (Res<SimTime>, Res<Time>, Res<crate::data::SeData>, Res<FlyCamera>, Res<RenderSwitches>),
    cloud_body: Res<CloudBody>,
    assets: Option<Res<CloudShadowAssets>>,
    bodies: Query<(&SystemBody, &Terrain, &BodyLighting)>,
    mut images: ResMut<Assets<Image>>,
    mut frame: ResMut<SimFrame>,
    mut views: Query<&mut CloudView, With<MainCamera>>,
    (mut maps, mut clock, mut drift): (Local<DerivedMaps>, Local<SimClock>, Local<Drift>),
) {
    let found = cloud_body.body.and_then(|e| Some((e, bodies.get(e).ok()?)));
    let (Some(assets), Some((e, (b, t, l)))) = (assets, found) else {
        clock.advance(None, time.jd, time.rate, time.paused, 0.0, false);
        *frame = SimFrame::default();
        *maps = DerivedMaps::default();
        *drift = Drift::default();
        return;
    };
    let c = climate(&data, b, t);
    let sun = (b.orient.inverse() * l.sun_dir.as_dvec3()).normalize_or(DVec3::X);
    let jd = time.jd;
    // A gas giant's bands and storms are SE's own (its decks' colour): no weather maps.
    let blend = if t.proc.kind.is_giant() || neutral_on() { maps.neutral(e, &mut images, &assets.derived) } else { maps.update(e, &c, sun, jd, time.rate, &mut images, &assets.derived) };

    let rel = cam.pos.delta_m(b.pos);
    let key = Some(cloud_body.id);
    let (last_jd, last_seed) = (clock.last_jd, clock.seed_id);
    let mode = clock.advance(key, jd, time.rate, time.paused, real.delta_secs_f64(), rel.length() < LIVE_RADII * t.radius);

    // The noise drifts with the wind at the camera while the weather runs live.
    if drift.key != key {
        *drift = Drift { key, fx: (jd * DAY_S).rem_euclid(FX_WRAP_S), ..default() };
    }
    let p = (b.orient.inverse() * rel).normalize_or(DVec3::Y);
    let w = se_core::weather::weather_at(&c, p, sun, jd).wind;
    let east = DVec3::new(p.z, 0.0, -p.x).normalize_or(DVec3::Z);
    let north = p.cross(east);
    let wind = east * w[0] as f64 + north * w[1] as f64;
    let mut motion = DVec3::ZERO;
    if mode == SimMode::Live && clock.seed_id == last_seed {
        let step = (wind * ((jd - last_jd) * DAY_S)).clamp_length_max(2_000.0);
        let s = step / b.shape;
        drift.shape = (drift.shape + s).rem_euclid(DVec3::splat(SHAPE_PERIOD_M as f64));
        drift.detail = (drift.detail + s).rem_euclid(DVec3::splat(DETAIL_PERIOD_M as f64));
        motion = b.orient * step;
    }
    if clock.seed_id != last_seed {
        drift.fx = (jd * DAY_S).rem_euclid(FX_WRAP_S);
    } else if !time.paused {
        drift.fx = (drift.fx + real.delta_secs_f64() * time.rate.abs().min(1.0)).rem_euclid(FX_WRAP_S);
    }
    let mut flash_h = Vec2::ZERO;
    if let Ok(mut v) = views.single_mut() {
        v.noise_off = drift.shape.as_vec3().extend(0.0);
        v.detail_off = drift.detail.as_vec3().extend(0.0);
        v.motion = motion.as_vec3().extend(0.0);
        let flag = |on: bool| on as u32 as f32;
        v.fx = Vec4::new(flag(switches.rain), flag(switches.lightning), flag(switches.fog), flag(switches.dust));
        // Lightning peaks at about Earth's sunlight on the cloud around it.
        v.fx2 = Vec4::new(drift.fx as f32, t.proc.temperature_k as f32, 0.6 * ATM_SUN_SCALE, debug_map() as u32 as f32);
        v.wind = (wind / b.shape).as_vec3().extend(0.0);
        flash_h = Vec2::new(v.cam2.z, v.atm_flags.w.min(v.cam2.z + 10_000.0));
    }

    let follow = matches!(mode, SimMode::Follow(_));
    let slice = if follow { jd } else { clock.t0 } * DAY_S / SEED_SLICE_S;
    *frame = SimFrame {
        body_id: cloud_body.id,
        seed_id: clock.seed_id,
        mode,
        target: clock.target,
        base: ((clock.t0 * DAY_S / STEP_S).round() as i64).rem_euclid(1 << 20) as u32,
        uniform: SimUniform {
            maps: Vec4::new(blend, t.radius as f32, c.humidity as f32, STEP_S as f32),
            step: Vec4::new(0.0, (c.seed % 10_000) as f32 / 10_000.0, (t.radius / 300_000.0).clamp(3.0, 25.0) as f32, 0.0),
            seed: Vec4::new(slice.floor().rem_euclid(4096.0) as f32, slice.fract() as f32, if follow { 1.0 } else { clock.frac(jd) }, cells_on() as u32 as f32),
            flash: Vec4::new(drift.fx as f32, flash_h.x, flash_h.y, switches.lightning as u32 as f32),
            cam: ((b.orient.inverse() * rel) / b.shape).normalize_or(DVec3::Y).as_vec3().extend((FLASH_NEAR_M / t.radius) as f32),
        },
    };
}

// ---------------------------------------------------------------------------
// Render world.

#[derive(Resource)]
pub(super) struct SimPipelines {
    sim_layout: BindGroupLayoutDescriptor,
    out_layout: BindGroupLayoutDescriptor,
    flash_layout: BindGroupLayoutDescriptor,
    seed: CachedComputePipelineId,
    step: CachedComputePipelineId,
    out: CachedComputePipelineId,
    flash: CachedComputePipelineId,
    /// Linear, repeating (the derived maps wrap in longitude).
    smp: Sampler,
}

pub(super) fn init_sim_pipelines(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let cs = ShaderStages::COMPUTE;
    let cube = || texture_cube(TextureSampleType::Float { filterable: true });
    let maps = || texture_2d_array(TextureSampleType::Float { filterable: true });
    let u = || uniform_buffer::<SimUniform>(false);
    let smp = || sampler(SamplerBindingType::Filtering);
    let sim_layout = BindGroupLayoutDescriptor::new(
        "weather_sim",
        &BindGroupLayoutEntries::with_indices(
            cs,
            ((0, u()), (1, cube()), (3, maps()), (4, smp()), (5, texture_storage_2d_array(STATE_FORMAT, StorageTextureAccess::WriteOnly))),
        ),
    );
    let out_layout = BindGroupLayoutDescriptor::new(
        "weather_sim_out",
        &BindGroupLayoutEntries::with_indices(
            cs,
            (
                (0, u()),
                (1, cube()),
                (2, cube()),
                (3, maps()),
                (4, smp()),
                (6, texture_storage_2d_array(TextureFormat::Rgba8Unorm, StorageTextureAccess::WriteOnly)),
                (7, texture_storage_2d_array(TextureFormat::Rgba8Unorm, StorageTextureAccess::WriteOnly)),
            ),
        ),
    );
    let flash_layout = BindGroupLayoutDescriptor::new(
        "weather_sim_flash",
        &BindGroupLayoutEntries::with_indices(cs, ((0, u()), (8, cube()), (4, smp()), (9, storage_buffer_sized(false, None)))),
    );
    let shader: Handle<Shader> = assets.load("shaders/weather_sim.wesl");
    let queue = |layout: &BindGroupLayoutDescriptor, entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("weather_{entry}").into()),
            layout: vec![layout.clone()],
            shader: shader.clone(),
            entry_point: Some(entry.into()),
            ..default()
        })
    };
    commands.insert_resource(SimPipelines {
        seed: queue(&sim_layout, "sim_seed"),
        step: queue(&sim_layout, "sim_step"),
        out: queue(&out_layout, "sim_out"),
        flash: queue(&flash_layout, "sim_flash"),
        sim_layout,
        out_layout,
        flash_layout,
        smp: device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        }),
    });
}

/// The two states (read one, write the other), as arrays to write and cubes to read.
struct SimTextures {
    array: [TextureView; 2],
    cube: [TextureView; 2],
    /// The fog and dust cube (written as an array, read as a cube) and the flashes.
    fx_array: TextureView,
    fx_cube: TextureView,
    flashes: Buffer,
}

/// The simulation's state on the GPU: `cur` the latest, the other one step (or frame) older.
#[derive(Resource, Default)]
pub(super) struct SimGpu {
    tex: Option<SimTextures>,
    cur: usize,
    body_id: u64,
    seed_id: u64,
    seeded: bool,
    step: u32,
}

/// The frame's simulation passes and the weather cube's output; false while the passes aren't ready
/// (the cube then holds nothing to draw with).
#[allow(clippy::too_many_arguments)]
pub(super) fn run_sim(
    encoder: &mut CommandEncoder,
    device: &RenderDevice,
    pipes: &SimPipelines,
    cache: &PipelineCache,
    frame: &SimFrame,
    gpu: &mut SimGpu,
    derived: &GpuImage,
    weather: &GpuImage,
) -> bool {
    if frame.mode == SimMode::Off {
        return false;
    }
    let (Some(seed), Some(step), Some(out), Some(flash)) = (
        cache.get_compute_pipeline(pipes.seed),
        cache.get_compute_pipeline(pipes.step),
        cache.get_compute_pipeline(pipes.out),
        cache.get_compute_pipeline(pipes.flash),
    ) else {
        return false;
    };
    let tex = gpu.tex.get_or_insert_with(|| {
        let views = [0, 1].map(|i| {
            let t = device.create_texture(&TextureDescriptor {
                label: Some(if i == 0 { "weather_sim_a" } else { "weather_sim_b" }),
                size: Extent3d { width: SIM_SIZE, height: SIM_SIZE, depth_or_array_layers: 6 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: STATE_FORMAT,
                usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = |dimension| t.create_view(&TextureViewDescriptor { dimension: Some(dimension), ..Default::default() });
            (view(TextureViewDimension::D2Array), view(TextureViewDimension::Cube))
        });
        let [(a0, c0), (a1, c1)] = views;
        let fx = device.create_texture(&TextureDescriptor {
            label: Some("weather_fx"),
            size: Extent3d { width: SIM_SIZE, height: SIM_SIZE, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let fx_view = |dimension| fx.create_view(&TextureViewDescriptor { dimension: Some(dimension), ..Default::default() });
        let flashes = device.create_buffer(&BufferDescriptor {
            label: Some("weather_flashes"),
            size: FLASHES * 16,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        SimTextures {
            array: [a0, a1],
            cube: [c0, c1],
            fx_array: fx_view(TextureViewDimension::D2Array),
            fx_cube: fx_view(TextureViewDimension::Cube),
            flashes,
        }
    });
    let groups = SIM_SIZE.div_ceil(8);
    let dispatch = |encoder: &mut CommandEncoder, label: &'static str, pipe: &ComputePipeline, group: &BindGroup| {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor { label: Some(label), ..default() });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(groups, groups, 6);
    };
    // A pass from the current state into the other, which becomes the current one.
    let sim_layout = cache.get_bind_group_layout(&pipes.sim_layout);
    let mut pass = |cur: &mut usize, label: &'static str, pipe: &ComputePipeline, u: &SimUniform| {
        let Some(buf) = uniform_buffer_of(device, "weather_sim", u) else { return };
        let group = device.create_bind_group(
            "weather_sim",
            &sim_layout,
            &BindGroupEntries::with_indices((
                (0, buf.as_entire_binding()),
                (1, &tex.cube[*cur]),
                (3, &derived.texture_view),
                (4, &pipes.smp),
                (5, &tex.array[1 - *cur]),
            )),
        );
        dispatch(encoder, label, pipe, &group);
        *cur = 1 - *cur;
    };
    let mut u = frame.uniform;
    // Seeded at once: both states, so the output's interpolation has nothing older to show.
    if !gpu.seeded || gpu.body_id != frame.body_id || gpu.seed_id != frame.seed_id {
        u.step.w = 1.0;
        pass(&mut gpu.cur, "weather_sim_seed", seed, &u);
        pass(&mut gpu.cur, "weather_sim_seed", seed, &u);
        (gpu.seeded, gpu.body_id, gpu.seed_id, gpu.step) = (true, frame.body_id, frame.seed_id, 0);
    }
    match frame.mode {
        SimMode::Follow(w) => {
            u.step.w = w;
            pass(&mut gpu.cur, "weather_sim_follow", seed, &u);
            gpu.step = frame.target;
        }
        SimMode::Live => {
            let n = frame.target.saturating_sub(gpu.step).min(MAX_STEPS);
            for k in 0..n {
                u.step.x = ((frame.base + gpu.step + k + 1) % (1 << 20)) as f32;
                pass(&mut gpu.cur, "weather_sim_step", step, &u);
            }
            gpu.step = frame.target;
        }
        SimMode::Frozen | SimMode::Off => {}
    }
    let Some(buf) = uniform_buffer_of(device, "weather_sim_out", &frame.uniform) else { return false };
    let target = weather.texture.create_view(&TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..Default::default() });
    let group = device.create_bind_group(
        "weather_sim_out",
        &cache.get_bind_group_layout(&pipes.out_layout),
        &BindGroupEntries::with_indices((
            (0, buf.as_entire_binding()),
            (1, &tex.cube[gpu.cur]),
            (2, &tex.cube[1 - gpu.cur]),
            (3, &derived.texture_view),
            (4, &pipes.smp),
            (6, &target),
            (7, &tex.fx_array),
        )),
    );
    dispatch(encoder, "weather_sim_out", out, &group);
    // The lightning of the moment, from the cube just written.
    let group = device.create_bind_group(
        "weather_sim_flash",
        &cache.get_bind_group_layout(&pipes.flash_layout),
        &BindGroupEntries::with_indices(((0, buf.as_entire_binding()), (8, &weather.texture_view), (4, &pipes.smp), (9, tex.flashes.as_entire_binding()))),
    );
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor { label: Some("weather_sim_flash"), ..default() });
    pass.set_pipeline(flash);
    pass.set_bind_group(0, &group, &[]);
    pass.dispatch_workgroups(1, 1, 1);
    true
}

impl SimGpu {
    /// The fog and dust cube and the lightning flashes, once made.
    pub(super) fn fx(&self) -> Option<(&TextureView, &Buffer)> {
        self.tex.as_ref().map(|t| (&t.fx_cube, &t.flashes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JD: f64 = 2_461_000.25;

    #[test]
    fn live_steps_jumps_seed_pause_freezes() {
        let mut c = SimClock::default();
        let dt = 1.0 / 60.0;
        // Arrival seeds.
        assert_eq!(c.advance(Some(1), JD, 100.0, false, dt, true), SimMode::Live);
        let (seed, t0) = (c.seed_id, c.t0);
        assert_eq!(c.target, 0);
        // At x100, 40 real seconds later: 4000 s, 13 steps of 5 minutes, no new seed.
        let mut jd = JD;
        for _ in 0..2400 {
            jd += 100.0 * dt / DAY_S;
            assert_eq!(c.advance(Some(1), jd, 100.0, false, dt, true), SimMode::Live);
        }
        assert_eq!((c.seed_id, c.t0), (seed, t0));
        assert_eq!(c.target, ((jd - t0) * DAY_S / STEP_S).floor() as u32);
        assert!(c.target >= 13);
        // Paused: frozen, the same steps.
        let steps = c.target;
        assert_eq!(c.advance(Some(1), jd, 100.0, true, dt, true), SimMode::Frozen);
        assert_eq!((c.seed_id, c.target), (seed, steps));
        // A date jump while paused seeds at once.
        assert_eq!(c.advance(Some(1), jd + 3.0, 100.0, true, dt, true), SimMode::Frozen);
        assert_eq!((c.seed_id, c.target), (seed + 1, 0));
        // Another body seeds too.
        c.advance(Some(2), jd + 3.0, 1.0, false, dt, true);
        assert_eq!(c.seed_id, seed + 2);
    }

    #[test]
    fn fast_far_or_backwards_time_follows() {
        let mut c = SimClock::default();
        let dt = 1.0 / 60.0;
        c.advance(Some(1), JD, 1.0, false, dt, true);
        let seed = c.seed_id;
        // x10000 is not a jump: it follows the seed state, blending over a fraction of a second.
        let jd = JD + 10_000.0 * dt / DAY_S;
        let SimMode::Follow(w) = c.advance(Some(1), jd, 10_000.0, false, dt, true) else { panic!("not following") };
        assert!(w > 0.05 && w < 0.2, "{w}");
        assert_eq!(c.seed_id, seed);
        // Backwards time and a camera far away follow as well.
        assert!(matches!(c.advance(Some(1), jd - 10.0 * dt / DAY_S, -10.0, false, dt, true), SimMode::Follow(_)));
        assert!(matches!(c.advance(Some(1), jd - 9.0 * dt / DAY_S, 1.0, false, dt, false), SimMode::Follow(_)));
        assert_eq!(c.seed_id, seed);
        // Back to live: stepping from the date it followed to.
        assert_eq!(c.advance(Some(1), jd - 8.0 * dt / DAY_S, 1.0, false, dt, true), SimMode::Live);
        assert_eq!(c.target, 0);
    }
}
