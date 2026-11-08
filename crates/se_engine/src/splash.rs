//! The loading screen: black, with "Sandbox Engine Loading..." in the middle (the user, 2026-11-07:
//! SE's own splash art, logo, quotes and version line are gone). It covers the window while loading
//! runs, at least `SHOW_S`, until the first view is loaded and can be drawn ([`check_loaded`]), then
//! fades. Scripted runs skip it (unless `SE_SPLASH` is set).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::config::EngineConfig;

/// The only thing it shows.
const TEXT: &str = "Sandbox Engine Loading...";
/// The text's height at a 1080 px window.
const TEXT_PX: f32 = 32.0;
/// The screen stays at least this long (s), then fades over `FADE_S` once the first view is loaded
/// and has been for `READY_HOLD_S`, or after `MAX_WAIT_S` whatever is left.
const SHOW_S: f32 = 3.0;
const FADE_S: f32 = 0.6;
const READY_HOLD_S: f32 = 0.5;
const MAX_WAIT_S: f32 = 60.0;

pub struct SplashPlugin;

impl Plugin for SplashPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Splash>()
            .add_systems(Startup, prepare)
            .add_systems(Update, check_loaded)
            .add_systems(EguiPrimaryContextPass, draw.after(crate::skin::UiSet::Backdrop));
    }
}

#[derive(Resource, Default)]
struct Splash {
    active: bool,
    age: f32,
    /// Seconds since it opened, unclamped (`age` takes at most 0.1 s a frame, for the drawing).
    waited: f32,
    /// How long the first view has been loaded without a break.
    ready_s: f32,
    /// The `age` it began to fade at.
    fade_at: Option<f32>,
}

/// Whether the first view is loaded: the saved session's body found (a procedural one waits for its
/// system), every terrain's nodes built, the ground's pipelines its nodes draw with built (the driver
/// takes seconds for each: the ground was missing for 7 s after the splash, the user's 2026-11-04
/// video), and no other pipeline still building.
fn check_loaded(
    mut splash: ResMut<Splash>,
    time: Res<Time>,
    pending: Option<Res<crate::session::PendingRef>>,
    terrains: Query<&crate::terrain::Terrain>,
    nodes: Query<&crate::terrain::NodeNoise, With<crate::terrain::TerrainNode>>,
    pipes: Res<crate::pipeline_watch::GroundPipelines>,
) {
    if !splash.active || splash.fade_at.is_some() {
        return;
    }
    use crate::pipeline_watch::GroundKind as K;
    let ground = nodes.iter().all(|n| {
        pipes.ready(n.used, K::Ground) && pipes.ready(n.used, K::Skirt) && (!n.sea || pipes.ready(crate::terrain::VertexNoise::None, K::Sea))
    });
    let loaded = pending.is_none() && terrains.iter().all(|t| t.loading_nodes() == 0) && ground && pipes.others_building() == 0;
    splash.waited += time.delta_secs();
    splash.ready_s = if loaded { splash.ready_s + time.delta_secs() } else { 0.0 };
}

fn prepare(mut splash: ResMut<Splash>, cfg: Res<EngineConfig>) {
    // SE_SPLASH=1 shows it in scripted runs too (screenshots of it), SE_NO_SPLASH=1 never.
    let forced = std::env::var_os("SE_SPLASH").is_some();
    splash.active = !(crate::settings::scripted(&cfg) && !forced) && std::env::var_os("SE_NO_SPLASH").is_none();
}

fn draw(mut contexts: EguiContexts, mut splash: ResMut<Splash>, time: Res<Time>) -> Result {
    if !splash.active {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    splash.age += time.delta_secs().min(0.1);
    if splash.fade_at.is_none() && splash.age >= SHOW_S && (splash.ready_s >= READY_HOLD_S || splash.waited >= MAX_WAIT_S) {
        if splash.ready_s < READY_HOLD_S {
            warn!("splash: still loading after {MAX_WAIT_S} s, opening the view anyway");
        }
        info!("splash: the first view loaded after {:.1} s", splash.waited);
        splash.fade_at = Some(splash.age);
    }
    let alpha = splash.fade_at.map_or(1.0, |t| 1.0 - ((splash.age - t) / FADE_S).clamp(0.0, 1.0));
    if alpha <= 0.0 {
        splash.active = false;
        return Ok(());
    }
    let screen = ctx.content_rect();
    let size = (TEXT_PX * screen.height() / 1080.0).max(16.0);
    let a = egui::Area::new(egui::Id::new("splash")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false);
    a.show(ctx, |ui| {
        let p = ui.painter();
        p.rect_filled(screen, 0.0, egui::Color32::BLACK.gamma_multiply(alpha));
        p.text(screen.center(), egui::Align2::CENTER_CENTER, TEXT, egui::FontId::proportional(size), egui::Color32::WHITE.gamma_multiply(alpha));
    });
    ctx.request_repaint();
    Ok(())
}
