//! SE's exposure compensation: the whole image brightened or darkened in photographic stops.
//!
//! SE keeps `ExposureComp` negated (registered by FUN_140011e60 with range -42..4 and step 0.25;
//! main-def.cfg's `MinExposureComp` -4 and `MaxExposureComp` 42 bound the value the interface
//! shows), so +1 shown is twice as bright. `.` / `,` raise / lower it and `/` resets it
//! (IncExposure, DecExposure, ResetExposure in SE's key table FUN_1405ba8f0); FUN_1404c6000 adds
//! the step and shows the splash message.

use bevy::prelude::*;
use bevy::render::view::ColorGrading;

use crate::camera::MainCamera;

/// The compensation step (`ExposureComp`'s step).
const STEP: f32 = 0.25;
/// Held keys repeat after this long, every `REPEAT` s (assumed; SE's rate comes from its input
/// system and was not traced).
const REPEAT_DELAY: f32 = 0.35;
const REPEAT: f32 = 0.05;

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

fn load_exposure(cfg: Res<crate::config::EngineConfig>, mut e: ResMut<ExposureComp>) {
    let get = |k: &str| crate::stars::main_def_f64(&cfg, k).map(|v| v as f32);
    e.min = get("MinExposureComp").unwrap_or(e.min);
    e.max = get("MaxExposureComp").unwrap_or(e.max);
    if let Some(v) = get("ExposureComp") {
        e.set(-v);
    }
}

fn exposure_keys(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut egui: bevy_egui::EguiContexts,
    mut e: ResMut<ExposureComp>,
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

pub struct ExposurePlugin;

impl Plugin for ExposurePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExposureComp>()
            .add_systems(Startup, load_exposure)
            .add_systems(Update, (exposure_keys, apply_exposure).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compensation_stays_in_ses_range() {
        let mut e = ExposureComp::default();
        e.set(50.0);
        assert_eq!(e.stops, 42.0);
        e.set(-10.0);
        assert_eq!(e.stops, -4.0);
    }
}
