//! SE's window and display settings (main-def.cfg `Display` and `VSync`): the window opens
//! maximized (`Maximized`) or at its saved size and place (`WindowSizeX/Y`, `WindowPosX/Y`), in
//! `FullScreen` mode 0 (windowed), 1 (fullscreen window) or 2 (exclusive fullscreen), with
//! `VSync` 0 (off), 1 (on) or 2 (auto: adaptive). Alt+Enter switches to and from fullscreen,
//! Shift+Enter moves the window to the next display, Ctrl+Enter minimizes it. The settings are
//! saved with the others (`settings.rs`).
//!
//! Scripted runs keep the fixed `--size` window. Not done: `ScreenResX/Y`, `RefreshRate` (exclusive
//! fullscreen uses the display's current mode) and `AspectRatio`.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, VideoModeSelection, WindowMode, WindowPosition};

use crate::config::EngineConfig;

#[derive(Resource, Debug, Clone, PartialEq)]
pub struct DisplaySettings {
    /// The windowed size and place (physical pixels), kept while maximized or fullscreen.
    pub size: UVec2,
    pub pos: Option<IVec2>,
    pub maximized: bool,
    /// 0 windowed, 1 fullscreen window, 2 exclusive fullscreen.
    pub fullscreen: u8,
    /// 0 off, 1 on, 2 auto (adaptive).
    pub vsync: u8,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        // SE's defaults: a maximized window, VSync auto.
        Self { size: UVec2::new(1600, 900), pos: None, maximized: true, fullscreen: 0, vsync: 2 }
    }
}

impl DisplaySettings {
    /// From the settings file's `Display` block (absent keys keep SE's defaults).
    pub fn load(text: &str) -> Self {
        let mut d = Self::default();
        let doc = se_core::sc::parse(text);
        let Some(b) = doc.sub("Display") else { return d };
        let num = |k: &str| b.f64(k);
        if let (Some(x), Some(y)) = (num("WindowSizeX"), num("WindowSizeY")) {
            d.size = UVec2::new(x.max(320.0) as u32, y.max(240.0) as u32);
        }
        if let (Some(x), Some(y)) = (num("WindowPosX"), num("WindowPosY")) {
            d.pos = Some(IVec2::new(x as i32, y as i32));
        }
        d.maximized = b.bool("Maximized").unwrap_or(d.maximized);
        d.fullscreen = num("FullScreen").map_or(d.fullscreen, |v| v.clamp(0.0, 2.0) as u8);
        d.vsync = num("VSync").map_or(d.vsync, |v| v.clamp(0.0, 2.0) as u8);
        d
    }

    pub fn to_cfg(&self) -> String {
        let pos = self.pos.unwrap_or(IVec2::new(-8, -8));
        format!(
            "Display\n{{\n\tWindowSizeX       {}\n\tWindowSizeY       {}\n\tWindowPosX        {}\n\tWindowPosY        {}\n\tMaximized         {}\n\tFullScreen        {}\n\tVSync             {}\n}}\n",
            self.size.x, self.size.y, pos.x, pos.y, self.maximized, self.fullscreen, self.vsync
        )
    }

    pub fn present_mode(&self) -> PresentMode {
        // SE_NO_VSYNC=1 uncaps the frame rate for performance measurements.
        if std::env::var_os("SE_NO_VSYNC").is_some() {
            return PresentMode::AutoNoVsync;
        }
        match self.vsync {
            0 => PresentMode::AutoNoVsync,
            1 => PresentMode::Fifo,
            _ => PresentMode::AutoVsync,
        }
    }

    pub fn window_mode(&self) -> WindowMode {
        match self.fullscreen {
            1 => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            2 => WindowMode::Fullscreen(MonitorSelection::Current, VideoModeSelection::Current),
            _ => WindowMode::Windowed,
        }
    }

    /// The primary window to open: `cfg`'s fixed size for scripted runs, else these settings.
    pub fn window(&self, cfg: &EngineConfig, title: &str) -> Window {
        let mut w = Window { title: title.into(), present_mode: self.present_mode(), ..default() };
        if crate::settings::scripted(cfg) {
            w.resolution = (cfg.width, cfg.height).into();
            return w;
        }
        w.resolution = bevy::window::WindowResolution::new(self.size.x, self.size.y);
        if let Some(p) = self.pos {
            w.position = WindowPosition::At(p);
        }
        w.mode = self.window_mode();
        if self.maximized {
            w.set_maximized(true);
        }
        w
    }
}

/// The display settings the window opened with (read before the app starts).
pub fn startup_settings(cfg: &EngineConfig) -> DisplaySettings {
    if crate::settings::scripted(cfg) {
        return DisplaySettings::default();
    }
    std::fs::read_to_string(crate::settings::user_file("settings.cfg")).map_or_else(|_| DisplaySettings::default(), |t| DisplaySettings::load(&t))
}

pub struct DisplayPlugin(pub DisplaySettings);

impl Plugin for DisplayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.0.clone()).add_systems(Update, (display_keys, apply_settings, track_window).chain());
    }
}

fn display_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<DisplaySettings>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    monitors: Query<Entity, With<bevy::window::Monitor>>,
    mut egui: bevy_egui::EguiContexts,
) {
    if egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false) || !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if alt && !shift && !ctrl {
        // SE's FullScreen action: back to the window, or to the fullscreen window.
        settings.fullscreen = if settings.fullscreen == 0 { 1 } else { 0 };
    } else if shift && !ctrl && !alt {
        // Switch to next display: the window centred on the next monitor.
        let n = monitors.iter().count().max(1);
        let cur = match window.position {
            WindowPosition::Centered(MonitorSelection::Index(i)) => i,
            _ => 0,
        };
        window.position = WindowPosition::Centered(MonitorSelection::Index((cur + 1) % n));
    } else if ctrl && !shift && !alt {
        window.set_minimized(true);
    }
}

/// Settings changed (keys or the Graphics window) go to the window.
fn apply_settings(settings: Res<DisplaySettings>, mut window: Single<&mut Window, With<PrimaryWindow>>) {
    if !settings.is_changed() || settings.is_added() {
        return;
    }
    let mode = settings.window_mode();
    if window.mode != mode {
        window.mode = mode;
    }
    let pm = settings.present_mode();
    if window.present_mode != pm {
        window.present_mode = pm;
    }
}

/// Keep the windowed size and place, and whether the window is maximized (from winit, on the main
/// thread), as SE does.
fn track_window(
    _main: bevy::ecs::system::NonSendMarker,
    cfg: Res<EngineConfig>,
    mut settings: ResMut<DisplaySettings>,
    window: Single<(Entity, &Window), With<PrimaryWindow>>,
) {
    if crate::settings::scripted(&cfg) {
        return;
    }
    let (entity, w) = *window;
    if !matches!(w.mode, WindowMode::Windowed) {
        return;
    }
    let state = bevy::winit::WINIT_WINDOWS.with_borrow(|ww| ww.get_window(entity).map(|win| (win.is_maximized(), win.is_minimized().unwrap_or(false), win.outer_position().ok())));
    let Some((maximized, minimized, outer)) = state else { return };
    if minimized {
        return;
    }
    let mut d = settings.clone();
    d.maximized = maximized;
    if !maximized {
        d.size = UVec2::new(w.resolution.physical_width(), w.resolution.physical_height());
        if let Some(p) = outer {
            d.pos = Some(IVec2::new(p.x, p.y));
        }
    }
    if d != *settings {
        // Not a change to apply back to the window.
        settings.bypass_change_detection().clone_from(&d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_block_round_trip() {
        let d = DisplaySettings { size: UVec2::new(1200, 700), pos: Some(IVec2::new(30, -8)), maximized: false, fullscreen: 2, vsync: 0 };
        assert_eq!(DisplaySettings::load(&d.to_cfg()), d);
        assert_eq!(DisplaySettings::load(""), DisplaySettings::default());
    }
}
