//! SE's splash messages (main-def.cfg Interface `ShowMessages true`, `MessageTimeout 2`): one line
//! in the middle of the screen after an action, e.g. "Time paused", "Orbits enabled", fading out
//! over `WidgetFadeOut` (0.35 s). SE builds the switch messages from the switch's name and
//! "enabled"/"disabled" (its locale strings); where on the screen it shows them is assumed.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

const TIMEOUT: f32 = 2.0;
const FADE_OUT: f32 = 0.35;

#[derive(Resource)]
pub struct Messages {
    /// SE's `ShowMessages`.
    pub enabled: bool,
    text: String,
    age: f32,
}

impl Default for Messages {
    fn default() -> Self {
        Self { enabled: true, text: String::new(), age: f32::INFINITY }
    }
}

impl Messages {
    pub fn show(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.age = 0.0;
    }

    /// "<name> enabled" or "<name> disabled", translated.
    pub fn switch(&mut self, i18n: &crate::i18n::I18n, name: &str, on: bool) {
        self.show(format!("{} {}", i18n.tr(name), i18n.tr(if on { "enabled" } else { "disabled" })));
    }
}

pub struct MessagesPlugin;

impl Plugin for MessagesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Messages>().add_systems(EguiPrimaryContextPass, draw_message.in_set(crate::skin::UiSet::Draw));
    }
}

fn draw_message(mut contexts: EguiContexts, time: Res<Time>, mut msg: ResMut<Messages>, hud: Res<crate::hud::HudState>) -> Result {
    msg.age += time.delta_secs();
    if !msg.enabled || hud.hidden || msg.age > TIMEOUT + FADE_OUT {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let alpha = 1.0 - ((msg.age - TIMEOUT) / FADE_OUT).clamp(0.0, 1.0);
    let color = crate::skin::text(ctx).info.gamma_multiply(alpha);
    let screen = ctx.viewport_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("splash_message")));
    painter.text(
        egui::pos2(screen.center().x, screen.min.y + screen.height() * 0.25),
        egui::Align2::CENTER_CENTER,
        &msg.text,
        egui::FontId::proportional(20.0),
        color,
    );
    Ok(())
}
