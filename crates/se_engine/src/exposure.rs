//! SE's exposure: the compensation in photographic stops, the photo modes and auto exposure.
//!
//! SE keeps `ExposureComp` negated (registered by FUN_140011e60 with range -42..4 and step 0.25;
//! main-def.cfg's `MinExposureComp` -4 and `MaxExposureComp` 42 bound the value the interface
//! shows), so +1 shown is twice as bright. `.` / `,` raise / lower it and `/` resets it
//! (IncExposure, DecExposure, ResetExposure in SE's key table FUN_1405ba8f0); FUN_1404c6000 adds
//! the step and shows the splash message.
//!
//! Photo modes (`PhotoMode`, V / `SwitchPhotoMode`): 0 HDR, 1 Manual, 2 Auto. FUN_1404c51a0 sets the
//! frame exposure to exp2(ExposureComp) * `ExposureCalibration`, times the auto exposure factor in mode
//! 2. Auto exposure (FUN_1404c09a0, hdr_histogram.glsl): a 256-bin histogram of log2 luminance over
//! `MinExposureComp`..`MaxExposureComp`, metered on a mip of the frame with a centre mask (luminance
//! times `AutoExpMaskValue` * sqrt(1 - r^2 / `AutoExpMaskSize`^2)), bin 0 (too dark) left out; the
//! `AutoExpLowPercent`..`AutoExpHighPercent` bins give a median, and the compensation
//! log2(median / `AutoExpTargetBright`), clamped to `AutoExpMinExpComp`..`AutoExpMaxExpComp`, is
//! approached by 1 - exp(-dt * speed), `AutoExpSpeedUp` when it grows (darker), else `AutoExpSpeedDown`.
//! The port meters 16 x 16 pixel blocks (`auto_exposure.wesl`) and `color_correction.wesl` applies the
//! factor before the tone shader.
//!
//! Not ported: Manual and Auto also switch SE's object brightness calibrations from the `hdr*` to the
//! `real*` set (main-def.cfg Lighting, e.g. realStarSurfaceBright 1e9 against hdrStarSurfaceBright 2000);
//! those are read by the object renderers, which keep the hdr* values, so Manual looks as HDR.

use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::binding_types::{storage_buffer_sized, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ColorGrading, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::camera::MainCamera;
use crate::color_correction::ColorCorrection;

/// The compensation step (`ExposureComp`'s step).
const STEP: f32 = 0.25;
/// Held keys repeat after this long, every `REPEAT` s (assumed; SE's rate comes from its input
/// system and was not traced).
const REPEAT_DELAY: f32 = 0.35;
const REPEAT: f32 = 0.05;
/// SE's histogram size (FUN_1404c09a0 clamps bins to 0..255).
const BINS: u64 = 256;

/// The exposure compensation as SE's interface shows it (stops, positive brighter).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ExposureComp {
    pub stops: f32,
    pub min: f32,
    pub max: f32,
}

impl Default for ExposureComp {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self { stops: 0.0, min: -4.0, max: 42.0 }
    }
}

impl ExposureComp {
    pub fn set(&mut self, stops: f32) {
        self.stops = stops.clamp(self.min, self.max);
    }
}

/// SE's photo mode (main-def.cfg `PhotoMode`): 0 HDR, 1 Manual, 2 Auto.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PhotoMode(pub u8);

impl PhotoMode {
    /// SE's interface names ("HDR", "Manual", "Auto").
    pub const NAMES: [&str; 3] = ["HDR", "Manual", "Auto"];

    pub fn new(mode: u8) -> Self {
        Self(mode.min(2))
    }

    pub fn auto(self) -> bool {
        self.0 == 2
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self.0.min(2) as usize]
    }

    /// `SwitchPhotoMode`: HDR, Manual, Auto, HDR ...
    pub fn next(self) -> Self {
        Self((self.0 + 1) % 3)
    }
}

/// SE's auto exposure settings (main-def.cfg `PostEffects`).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct AutoExposureSettings {
    pub target_bright: f32,
    pub min_comp: f32,
    pub max_comp: f32,
    pub low_percent: f32,
    pub high_percent: f32,
    pub speed_up: f32,
    pub speed_down: f32,
    pub mask_size: f32,
    pub mask_value: f32,
}

impl Default for AutoExposureSettings {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self {
            target_bright: 0.35,
            min_comp: 0.0,
            max_comp: 42.0,
            low_percent: 10.0,
            high_percent: 90.0,
            speed_up: 5.0,
            speed_down: 1.0,
            mask_size: 0.5,
            mask_value: 1.0,
        }
    }
}

fn load_exposure(cfg: Res<crate::config::EngineConfig>, mut e: ResMut<ExposureComp>, mut photo: ResMut<PhotoMode>, mut auto: ResMut<AutoExposureSettings>) {
    let get = |k: &str| crate::stars::main_def_f64(&cfg, k).map(|v| v as f32);
    e.min = get("MinExposureComp").unwrap_or(e.min);
    e.max = get("MaxExposureComp").unwrap_or(e.max);
    if let Some(v) = get("ExposureComp") {
        e.set(-v);
    }
    if let Some(m) = get("PhotoMode") {
        *photo = PhotoMode::new(m.max(0.0) as u8);
    }
    let a = &mut *auto;
    for (k, v) in [
        ("AutoExpTargetBright", &mut a.target_bright),
        ("AutoExpMinExpComp", &mut a.min_comp),
        ("AutoExpMaxExpComp", &mut a.max_comp),
        ("AutoExpLowPercent", &mut a.low_percent),
        ("AutoExpHighPercent", &mut a.high_percent),
        ("AutoExpSpeedUp", &mut a.speed_up),
        ("AutoExpSpeedDown", &mut a.speed_down),
        ("AutoExpMaskSize", &mut a.mask_size),
        ("AutoExpMaskValue", &mut a.mask_value),
    ] {
        if let Some(x) = get(k) {
            *v = x;
        }
    }
    // Testing: SE_PHOTOMODE=0|1|2.
    if let Some(m) = std::env::var("SE_PHOTOMODE").ok().and_then(|v| v.parse::<u8>().ok()) {
        *photo = PhotoMode::new(m);
    }
}

fn exposure_keys(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut egui: bevy_egui::EguiContexts,
    mut e: ResMut<ExposureComp>,
    mut photo: ResMut<PhotoMode>,
    flight: Res<crate::camera::FlightHold>,
    mut held: Local<(f32, f32)>,
    mut msg: ResMut<crate::messages::Messages>,
    i18n: Res<crate::i18n::I18n>,
) {
    if egui.ctx_mut().is_ok_and(|c| c.egui_wants_keyboard_input()) {
        return;
    }
    let mods = [KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft, KeyCode::AltRight];
    if keys.any_pressed(mods) {
        // Shift: ambient lighting, Ctrl: the glow magnitude limit (SE's table).
        return;
    }
    // V: SwitchPhotoMode (while a ship is flown V belongs to the ship, `ship.rs`).
    if keys.just_pressed(KeyCode::KeyV) && !flight.0 {
        *photo = photo.next();
        msg.show(format!("{}: {}", i18n.tr("Photo mode"), i18n.tr(photo.name())));
    }
    if keys.just_pressed(KeyCode::Slash) {
        e.set(0.0);
        msg.show(format!("{}: {:+.2}", i18n.tr("Exposure compensation"), e.stops));
        return;
    }
    let dir = match (keys.pressed(KeyCode::Period), keys.pressed(KeyCode::Comma)) {
        (true, false) => 1.0,
        (false, true) => -1.0,
        _ => {
            *held = (0.0, 0.0);
            return;
        }
    };
    // One step per press, then repeating while held.
    let (t, next) = &mut *held;
    let step = if keys.just_pressed(KeyCode::Period) || keys.just_pressed(KeyCode::Comma) {
        *t = 0.0;
        *next = REPEAT_DELAY;
        true
    } else {
        *t += time.delta_secs();
        (*t >= *next).then(|| *next += REPEAT).is_some()
    };
    if step {
        let v = e.stops + dir * STEP;
        e.set(v);
        msg.show(format!("{}: {:+.2}", i18n.tr("Exposure compensation"), e.stops));
    }
}

/// Puts the compensation on the camera's exposure, on top of the tone mapping's own.
fn apply_exposure(e: Res<ExposureComp>, mut base: Local<Option<f32>>, mut q: Query<&mut ColorGrading, With<MainCamera>>) {
    for mut g in &mut q {
        let b = *base.get_or_insert(g.global.exposure);
        let want = b + e.stops;
        if g.global.exposure != want {
            g.global.exposure = want;
        }
    }
}

/// The auto exposure shader's parameters; layout must match `auto_exposure.wesl`.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct AutoParams {
    /// x, y: the histogram's log2 luminance range (`MinExposureComp`, `MaxExposureComp`); z, w the
    /// low and high fractions.
    range: Vec4,
    /// x log2 `AutoExpTargetBright`, y, z the compensation limits, w the frame time (s).
    target: Vec4,
    /// x `AutoExpSpeedUp`, y `AutoExpSpeedDown`, z 1 / `AutoExpMaskSize`^2, w `AutoExpMaskValue`.
    speed: Vec4,
}

/// What the render world needs each frame.
#[derive(Resource, ExtractResource, Clone, Copy, Default)]
#[extract_app(RenderApp)]
struct AutoFrame {
    on: bool,
    params: AutoParams,
}

fn update_auto_frame(photo: Res<PhotoMode>, e: Res<ExposureComp>, a: Res<AutoExposureSettings>, time: Res<Time>, mut frame: ResMut<AutoFrame>) {
    *frame = AutoFrame {
        on: photo.auto(),
        params: AutoParams {
            range: Vec4::new(e.min, e.max, a.low_percent / 100.0, a.high_percent / 100.0),
            target: Vec4::new(a.target_bright.max(1e-6).log2(), a.min_comp, a.max_comp, time.delta_secs()),
            speed: Vec4::new(a.speed_up, a.speed_down, 1.0 / (a.mask_size * a.mask_size).max(1e-6), a.mask_value),
        },
    };
}

pub struct ExposurePlugin;

impl Plugin for ExposurePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExposureComp>()
            .init_resource::<PhotoMode>()
            .init_resource::<AutoExposureSettings>()
            .init_resource::<AutoFrame>()
            .add_plugins(ExtractResourcePlugin::<AutoFrame>::default())
            .add_systems(Startup, load_exposure)
            .add_systems(Update, (exposure_keys, apply_exposure, update_auto_frame).chain());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_auto_exposure)
            .add_systems(Render, prepare_auto.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, auto_exposure.before(tonemapping).after(crate::bloom::SeBloomSet).in_set(Core3dSystems::PostProcess));
    }
}

/// The histogram and the adapted compensation (x, stops; y the metered median log2 luminance), which
/// `color_correction.rs` binds.
#[derive(Resource)]
pub struct AutoExposureBuffers {
    pub state: Buffer,
    bins: Buffer,
    params: UniformBuffer<AutoParams>,
}

#[derive(Resource)]
struct AutoExposurePipelines {
    layout: BindGroupLayoutDescriptor,
    histogram: CachedComputePipelineId,
    adapt: CachedComputePipelineId,
}

fn init_auto_exposure(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let layout = BindGroupLayoutDescriptor::new(
        "auto_exposure",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                uniform_buffer::<AutoParams>(false),
                storage_buffer_sized(false, std::num::NonZero::new(BINS * 4)),
                storage_buffer_sized(false, std::num::NonZero::new(16)),
            ),
        ),
    );
    let shader: Handle<Shader> = assets.load("shaders/auto_exposure.wesl");
    let queue = |entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("auto_exposure_{entry}").into()),
            layout: vec![layout.clone()],
            shader: shader.clone(),
            entry_point: Some(entry.into()),
            ..default()
        })
    };
    let (histogram, adapt) = (queue("histogram"), queue("adapt"));
    let zeros = |label: &'static str, size: usize, usage: BufferUsages| {
        device.create_buffer_with_data(&BufferInitDescriptor { label: Some(label), contents: &vec![0u8; size], usage })
    };
    let state = zeros("auto_exposure_state", 16, BufferUsages::STORAGE);
    let bins = zeros("auto_exposure_bins", BINS as usize * 4, BufferUsages::STORAGE);
    commands.insert_resource(AutoExposureBuffers { state, bins, params: UniformBuffer::default() });
    commands.insert_resource(AutoExposurePipelines { layout, histogram, adapt });
}

fn prepare_auto(frame: Option<Res<AutoFrame>>, mut bufs: ResMut<AutoExposureBuffers>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let Some(f) = frame.filter(|f| f.on) else { return };
    bufs.params.set(f.params);
    bufs.params.write_buffer(&device, &queue);
}

fn auto_exposure(
    view: ViewQuery<&ViewTarget, (With<ColorCorrection>, Without<crate::capture::CaptureView>)>,
    frame: Option<Res<AutoFrame>>,
    pipes: Res<AutoExposurePipelines>,
    bufs: Res<AutoExposureBuffers>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    if !frame.is_some_and(|f| f.on) {
        return;
    }
    let target = view.into_inner();
    let (Some(histogram), Some(adapt), Some(params)) =
        (cache.get_compute_pipeline(pipes.histogram), cache.get_compute_pipeline(pipes.adapt), bufs.params.binding())
    else {
        return;
    };
    let layout = cache.get_bind_group_layout(&pipes.layout);
    let group = ctx.render_device().create_bind_group(
        "auto_exposure",
        &layout,
        &BindGroupEntries::sequential((
            target.main_texture_view(),
            params,
            bufs.bins.as_entire_buffer_binding(),
            bufs.state.as_entire_buffer_binding(),
        )),
    );
    let size = target.main_texture().size();
    // One thread per 16 x 16 pixel block, 16 x 16 threads per group.
    let groups = |n: u32| n.div_ceil(16).div_ceil(16).max(1);
    let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("auto_exposure"), ..default() });
    pass.set_bind_group(0, &group, &[]);
    pass.set_pipeline(histogram);
    pass.dispatch_workgroups(groups(size.width), groups(size.height), 1);
    pass.set_pipeline(adapt);
    pass.dispatch_workgroups(1, 1, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `auto_exposure.wesl`'s `adapt`, as a reference model of FUN_1404c09a0.
    impl AutoExposureSettings {
        /// The compensation (stops, positive darker) aimed at for a median log2 luminance.
        fn target_comp(&self, median_log2: f32) -> f32 {
            (median_log2 - self.target_bright.max(1e-6).log2()).clamp(self.min_comp, self.max_comp)
        }

        /// One frame of the adaptation from `comp` towards `target`.
        fn adapt(&self, comp: f32, target: f32, dt: f32) -> f32 {
            let speed = if target > comp { self.speed_up } else { self.speed_down };
            comp + (target - comp) * (1.0 - (-dt * speed).exp()).clamp(0.0, 1.0)
        }
    }

    #[test]
    fn compensation_stays_in_ses_range() {
        let mut e = ExposureComp::default();
        e.set(50.0);
        assert_eq!(e.stops, 42.0);
        e.set(-10.0);
        assert_eq!(e.stops, -4.0);
    }

    #[test]
    fn photo_modes_and_auto_exposure() {
        assert_eq!(PhotoMode(2).next(), PhotoMode(0));
        assert_eq!(PhotoMode::new(7).name(), "Auto");
        let a = AutoExposureSettings::default();
        // A median at the target brightness needs no compensation; 8x brighter, 3 stops darker.
        assert!(a.target_comp(0.35f32.log2()).abs() < 1e-6);
        assert!((a.target_comp((0.35f32 * 8.0).log2()) - 3.0).abs() < 1e-5);
        // A dark frame is never brightened (AutoExpMinExpComp 0).
        assert_eq!(a.target_comp(-10.0), 0.0);
        // Darkening uses AutoExpSpeedUp (5/s), brightening AutoExpSpeedDown (1/s).
        let up = a.adapt(0.0, 1.0, 0.1);
        let down = a.adapt(1.0, 0.0, 0.1);
        assert!((up - (1.0 - (-0.5f32).exp())).abs() < 1e-6);
        assert!((down - (-0.1f32).exp()).abs() < 1e-6);
    }
}
