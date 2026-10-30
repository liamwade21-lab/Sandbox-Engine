//! Simulation time and the floating-origin / scaled-space mapping from universe
//! positions to render-space transforms.

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use se_core::time::{DateTime, J2000};
use se_core::upos::UPos;

use crate::camera::FlyCamera;

/// Simulation clock in Julian days.
#[derive(Resource, Debug, Clone)]
pub struct SimTime {
    pub jd: f64,
    /// Simulated seconds per real second.
    pub rate: f64,
    pub paused: bool,
}

impl Default for SimTime {
    fn default() -> Self {
        let now = DateTime::now_utc().to_jd();
        Self {
            jd: if now.is_finite() { now } else { J2000 },
            rate: 1.0,
            paused: false,
        }
    }
}

impl SimTime {
    pub fn date(&self) -> DateTime {
        DateTime::from_jd(self.jd)
    }
}

/// High-precision position of an entity in the universe frame.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UPosition(pub UPos);

/// High-precision orientation (body frame -> universe frame).
#[derive(Component, Debug, Clone, Copy)]
pub struct UOrientation(pub DQuat);

impl Default for UOrientation {
    fn default() -> Self {
        Self(DQuat::IDENTITY)
    }
}

/// Local scale in meters applied to the entity's mesh (e.g. a planet radius for a
/// unit-sphere mesh). The scaled-space factor multiplies this.
#[derive(Component, Debug, Clone, Copy)]
pub struct MeterScale(pub DVec3);

impl Default for MeterScale {
    fn default() -> Self {
        Self(DVec3::ONE)
    }
}

/// Use this position (typically the owning body's centre) to compute the
/// scaled-space factor, so all parts of one body share it and stay seamless.
#[derive(Component, Debug, Clone, Copy)]
pub struct ScaleAnchor(pub UPos);

/// Distance (render meters) beyond which positions are logarithmically compressed.
/// Updated every frame so the nearest body and anything around it stay linear.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ScaledSpace {
    pub linear_radius: f64,
}

impl Default for ScaledSpace {
    fn default() -> Self {
        Self { linear_radius: 1.0e7 }
    }
}

impl ScaledSpace {
    /// Compressed distance for a true distance `d` (both in meters).
    pub fn compress(&self, d: f64) -> f64 {
        let l = self.linear_radius;
        if d <= l { d } else { l * (1.0 + (d / l).ln()) }
    }

    /// Scale factor for a true distance `d`.
    pub fn factor(&self, d: f64) -> f64 {
        if d <= self.linear_radius || d == 0.0 { 1.0 } else { self.compress(d) / d }
    }

    /// Camera-relative render translation and the uniform scale factor to apply.
    pub fn map(&self, delta: DVec3) -> (Vec3, f64) {
        let d = delta.length();
        if d <= self.linear_radius || d == 0.0 {
            (delta.as_vec3(), 1.0)
        } else {
            let k = self.compress(d) / d;
            ((delta * k).as_vec3(), k)
        }
    }
}

pub struct UniversePlugin;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum UniverseSet {
    /// Advance time and update body positions.
    Simulate,
    /// Move the camera.
    Camera,
    /// Convert universe positions to render transforms.
    FloatingOrigin,
}

/// SE's frame timing (main-def Export `RealTime` true, `RealTimeFPS` 60; key TimingMode, Shift+Backslash,
/// "Real time mode" / "Frame time mode"): in real time mode everything advances by the frame's real
/// duration, in frame time mode by 1 / RealTimeFPS per frame (motion then follows the frame rate, as
/// for video capture). Bevy's `TimeUpdateStrategy` carries it to every system.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TimingMode {
    pub real_time: bool,
    pub fps: f64,
}

impl Default for TimingMode {
    fn default() -> Self {
        Self { real_time: true, fps: 60.0 }
    }
}

fn load_timing(cfg: Res<crate::config::EngineConfig>, mut mode: ResMut<TimingMode>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let doc = se_core::sc::parse(&text);
    if let Some(v) = doc.find_deep("RealTime").and_then(|i| i.bool()) {
        mode.real_time = v;
    }
    if let Some(v) = doc.find_deep("RealTimeFPS").and_then(|i| i.f64()).filter(|v| *v > 0.0) {
        mode.fps = v;
    }
}

fn apply_timing(mode: Res<TimingMode>, mut commands: Commands) {
    if mode.is_changed() {
        commands.insert_resource(if mode.real_time {
            bevy::time::TimeUpdateStrategy::Automatic
        } else {
            bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / mode.fps.max(1.0)))
        });
    }
}

impl Plugin for UniversePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TimingMode>()
            .add_systems(Startup, load_timing)
            .add_systems(PreUpdate, apply_timing);
        app.init_resource::<SimTime>()
            .init_resource::<ScaledSpace>()
            .configure_sets(
                PostUpdate,
                (UniverseSet::Simulate, UniverseSet::Camera, UniverseSet::FloatingOrigin)
                    .chain()
                    .before(TransformSystems::Propagate),
            )
            .add_systems(PostUpdate, advance_time.in_set(UniverseSet::Simulate))
            .add_systems(PostUpdate, apply_floating_origin.in_set(UniverseSet::FloatingOrigin));
    }
}

pub fn advance_time(mut t: ResMut<SimTime>, time: Res<Time>) {
    if !t.paused {
        t.jd += time.delta_secs_f64() * t.rate / 86_400.0;
    }
}

fn apply_floating_origin(
    cam: Res<FlyCamera>,
    space: Res<ScaledSpace>,
    mut q: Query<(
        &UPosition,
        Option<&UOrientation>,
        Option<&MeterScale>,
        Option<&ScaleAnchor>,
        &mut Transform,
    )>,
) {
    for (pos, rot, scale, anchor, mut tf) in &mut q {
        let delta = pos.0.delta_m(cam.pos);
        let (translation, k) = match anchor {
            Some(a) => {
                let k = space.factor(a.0.delta_m(cam.pos).length());
                ((delta * k).as_vec3(), k)
            }
            None => space.map(delta),
        };
        tf.translation = translation;
        if let Some(r) = rot {
            tf.rotation = r.0.as_quat();
        }
        let s = scale.map(|s| s.0).unwrap_or(DVec3::ONE) * k;
        tf.scale = s.as_vec3();
    }
}
