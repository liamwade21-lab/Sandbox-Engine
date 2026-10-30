//! Tutorial interface hooks: SE's `HighlightWidget`, `SetWidgetStyle`, `ShowDialog`,
//! `HideDialog`, `HideAllDialogs`, `HideAllToolbars`, `OpenMenu` and the screen fades
//! `FadeOut` / `FadeIn`.
//!
//! * `HighlightWidget "Main toolbar|Wiki" { Time Freq Color }` (FUN_14055df80): SE draws the named
//!   widget in the highlight colour during the first half of each blink (FUN_140362eb0: phase
//!   below 0.5), `Freq` blinks a second (default 4), for `Time` seconds or, with no `Time`, until
//!   the script's next wait ends (FUN_1402761d0, FUN_14054e040). Default colour (1, 0.2, 0.2, 1)
//!   (FUN_14036c000). The port lays a blinking frame in that colour over the widget: HUD buttons
//!   record their rectangles under SE's names ([`mark`]); a name the port has no button for
//!   blinks its toolbar or window, `"Message box"` the script's caption and `"SelectionPointer"`
//!   the selected object.
//! * Toolbars are SE dialogs: `HideAllToolbars` (FUN_1402536d0) slides all four away, `ShowDialog
//!   "Main toolbar"` brings one back, each over `WidgetSlideOut` / `WidgetSlideIn` 0.2 s.
//! * `SetWidgetStyle "Message box" { Pos (x, y) }` (FUN_14055e790) moves the caption's top-left
//!   corner to that fraction of the screen.
//! * `FadeOut { Time Color }` / `FadeIn { Time }` (FUN_1405554e0 / FUN_140554f90) fade the view to a
//!   colour (default black) and back, by default over `OverlayFadeOut` 0.35 s / `OverlayFadeIn` 0.2 s.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::ephemeris::Ephemeris;
use se_core::se_script::Highlight;

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::hud::HudState;
use crate::i18n::I18n;
use crate::universe::SimTime;

/// FUN_14036c000's highlight defaults.
pub const HIGHLIGHT_COLOR: [f32; 4] = [1.0, 0.2, 0.2, 1.0];
pub const HIGHLIGHT_FREQ: f64 = 4.0;
/// main-def.cfg `WidgetSlideIn` / `WidgetSlideOut` (s).
const SLIDE_S: f32 = 0.2;
/// main-def.cfg `OverlayFadeOut` / `OverlayFadeIn` (s): the screen fades' default times.
pub const FADE_OUT_S: f64 = 0.35;
pub const FADE_IN_S: f64 = 0.2;

/// SE's toolbars, in the order of [`ScriptGui::toolbars`], with the port's egui area ids.
pub const TOOLBARS: [(&str, &str); 4] =
    [("Main toolbar", "toolbar_main"), ("Filters toolbar", "toolbar_filters"), ("Navigation toolbar", "toolbar_navigation"), ("Camera toolbar", "toolbar_camera")];

pub struct ScriptGuiPlugin;

impl Plugin for ScriptGuiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScriptGui>().add_systems(
            EguiPrimaryContextPass,
            draw_gui.after(crate::skin::UiSet::Draw).before(crate::skin::UiSet::Backdrop),
        );
    }
}

/// A running `HighlightWidget`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveHighlight {
    pub widget: String,
    pub color: [f32; 4],
    pub freq: f64,
    /// `Time`; `None` lasts until the script's current wait ends.
    pub time: Option<f64>,
    pub elapsed: f64,
}

impl ActiveHighlight {
    pub fn new(h: &Highlight) -> Option<Self> {
        (!h.widget.is_empty()).then(|| Self {
            widget: h.widget.clone(),
            color: h.color.unwrap_or(HIGHLIGHT_COLOR),
            freq: h.freq.filter(|f| *f > 0.0).unwrap_or(HIGHLIGHT_FREQ),
            time: h.time.filter(|t| *t > 0.0),
            elapsed: 0.0,
        })
    }

    /// Whether the widget shows the highlight colour now (the first half of each blink).
    pub fn lit(&self) -> bool {
        (self.elapsed * self.freq).fract() < 0.5
    }

    pub fn expired(&self) -> bool {
        self.time.is_some_and(|t| self.elapsed >= t)
    }
}

/// A screen fade.
#[derive(Debug, Clone, PartialEq)]
pub struct Fade {
    pub from: f32,
    pub to: f32,
    pub color: [f32; 4],
    pub elapsed: f64,
    pub time: f64,
}

impl Fade {
    pub fn alpha(&self) -> f32 {
        let t = if self.time <= 0.0 { 1.0 } else { (self.elapsed / self.time).clamp(0.0, 1.0) as f32 };
        self.from + (self.to - self.from) * t
    }
}

/// Interface state set by scripts.
#[derive(Resource, Debug)]
pub struct ScriptGui {
    pub highlight: Option<ActiveHighlight>,
    /// `SetWidgetStyle "Message box" { Pos }`: the caption's top-left corner, in screen fractions.
    pub message_pos: Option<[f64; 2]>,
    /// `SetWidgetStyle "Message box" { PosPix }`, in pixels.
    pub message_pos_pix: Option<[f64; 2]>,
    /// Hidden toolbars ([`TOOLBARS`] order) and how far each has slid out (0 shown, 1 gone).
    pub toolbars: [bool; 4],
    slide: [f32; 4],
    pub fade: Option<Fade>,
}

impl Default for ScriptGui {
    fn default() -> Self {
        Self { highlight: None, message_pos: None, message_pos_pix: None, toolbars: [false; 4], slide: [0.0; 4], fade: None }
    }
}

impl ScriptGui {
    /// A script ended: the interface goes back to normal (FUN_14054d240).
    pub fn reset(&mut self) {
        self.highlight = None;
        self.message_pos = None;
        self.message_pos_pix = None;
        self.toolbars = [false; 4];
        self.fade = None;
    }

    /// A wait ended: a highlight without `Time` goes out.
    pub fn wait_ended(&mut self) {
        if self.highlight.as_ref().is_some_and(|h| h.time.is_none()) {
            self.highlight = None;
        }
    }

    pub fn toolbar_index(name: &str) -> Option<usize> {
        TOOLBARS.iter().position(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    pub fn update(&mut self, dt: f64) {
        if let Some(h) = &mut self.highlight {
            h.elapsed += dt;
            if h.expired() {
                self.highlight = None;
            }
        }
        if let Some(f) = &mut self.fade {
            f.elapsed += dt;
            if f.elapsed >= f.time && f.to <= 0.0 {
                self.fade = None;
            }
        }
        for i in 0..4 {
            let step = dt as f32 / SLIDE_S;
            self.slide[i] = if self.toolbars[i] { (self.slide[i] + step).min(1.0) } else { (self.slide[i] - step).max(0.0) };
        }
    }

    /// Fade to `color` (`out`) or back from it.
    pub fn start_fade(&mut self, out: bool, time: Option<f64>, color: Option<[f32; 4]>) {
        let cur = self.fade.as_ref().map_or(0.0, Fade::alpha);
        let color = color.or(self.fade.as_ref().map(|f| f.color)).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let time = time.unwrap_or(if out { FADE_OUT_S } else { FADE_IN_S });
        self.fade = Some(Fade { from: cur, to: if out { 1.0 } else { 0.0 }, color, elapsed: 0.0, time });
    }
}

/// The port's windows that SE's scripts open and close by their SE names.
#[derive(SystemParam)]
pub struct Dialogs<'w> {
    pub hud: ResMut<'w, HudState>,
    pub sysbrowser: ResMut<'w, crate::sysbrowser::SysBrowser>,
    pub browser: ResMut<'w, crate::browser::BrowserState>,
    pub gfx: ResMut<'w, crate::graphics::GraphicsUi>,
    pub scripts: ResMut<'w, crate::scripting::ScriptUi>,
    pub gui: ResMut<'w, ScriptGui>,
}

impl Dialogs<'_> {
    /// Whether the dialog SE calls `name` is open; `None` when the port has no such window.
    pub fn is_open(&self, name: &str) -> Option<bool> {
        let n = name.split('|').next().unwrap_or(name).trim().to_ascii_lowercase();
        if let Some(i) = ScriptGui::toolbar_index(&n) {
            return Some(!self.gui.toolbars[i] && !self.hud.hidden);
        }
        Some(match n.as_str() {
            "wiki" => self.hud.show_wiki,
            "settings" => self.hud.show_settings,
            "find object" => self.hud.show_find,
            "locations" => self.hud.show_places,
            "solar system browser" => self.sysbrowser.open,
            "star browser" => self.browser.show_stars,
            _ => return None,
        })
    }

    /// Opens or closes a dialog; false when the port has no such window. `ShowDialog "Settings"
    /// { Tab "Graphics" }` opens the port's Graphics window, SE's Graphics tab.
    pub fn set_open(&mut self, name: &str, open: bool, tab: Option<&str>) -> bool {
        let n = name.trim().to_ascii_lowercase();
        if let Some(i) = ScriptGui::toolbar_index(&n) {
            self.gui.toolbars[i] = !open;
            return true;
        }
        match n.as_str() {
            "wiki" => self.hud.show_wiki = open,
            "settings" => {
                self.hud.show_settings = open;
                if tab.is_some_and(|t| t.eq_ignore_ascii_case("graphics")) || !open {
                    self.gfx.open = open;
                }
            }
            "find object" => self.hud.show_find = open,
            "locations" => self.hud.show_places = open,
            "solar system browser" => self.sysbrowser.open = open,
            "star browser" => self.browser.show_stars = open,
            _ => return false,
        }
        true
    }

    /// SE's HideAllDialogs: every window, not the toolbars.
    pub fn hide_all(&mut self) {
        for d in ["wiki", "settings", "find object", "locations", "solar system browser", "star browser"] {
            self.set_open(d, false, None);
        }
        self.hud.show_help = false;
        self.scripts.show = false;
    }
}

// ---- widget rectangles ---------------------------------------------------------------

/// SE's name of a HUD button: the port's label on SE's toolbar `toolbar`.
fn se_widget_name(toolbar: &str, label: &str) -> String {
    let name = match (toolbar, label) {
        ("Main toolbar", "Find") => "Find object",
        ("Main toolbar", "Places") => "Locations",
        ("Main toolbar", "Search") => "Star browser",
        ("Filters toolbar", "Celestial grids") => "Grids",
        (_, l) => l,
    };
    format!("{toolbar}|{name}")
}

fn rect_id(name: &str) -> egui::Id {
    egui::Id::new(("se_widget", name.to_ascii_lowercase()))
}

/// Records a HUD button's rectangle under SE's name (`Main toolbar|Wiki`) for `HighlightWidget`.
pub fn mark(ui: &egui::Ui, toolbar: &str, label: &str, r: &egui::Response) {
    mark_named(ui.ctx(), &se_widget_name(toolbar, label), r.rect);
}

/// Records a widget's rectangle under its full SE name (`Navigation toolbar|Go to object`) and
/// passes the response on.
pub fn tag(name: &str, r: egui::Response) -> egui::Response {
    mark_named(&r.ctx, name, r.rect);
    r
}

/// Records a widget's rectangle under its SE name.
pub fn mark_named(ctx: &egui::Context, name: &str, rect: egui::Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(rect_id(name), (rect, pass)));
}

/// A rectangle recorded this pass or the last.
fn marked(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(egui::Rect, u64)>(rect_id(name))).filter(|(_, p)| p + 1 >= pass).map(|(r, _)| r)
}

/// The visible rectangle of an egui area (a toolbar or a window).
fn area(ctx: &egui::Context, id: egui::Id) -> Option<egui::Rect> {
    let layer = egui::LayerId::new(egui::Order::Middle, id);
    ctx.memory(|m| m.areas().is_visible(&layer).then(|| m.area_rect(id)).flatten())
}

/// The port's window id of an SE dialog.
fn window_id(ctx: &egui::Context, i18n: &I18n, dialog: &str) -> Option<egui::Id> {
    let title = match dialog.to_ascii_lowercase().as_str() {
        "solar system browser" => return Some(egui::Id::new("sysbrowser")),
        "wiki" => "Wiki",
        "settings" => "Settings",
        "find object" => "Find object",
        "locations" => "Places",
        "star browser" => "Star browser",
        "filter settings" => "Filter settings",
        _ => return None,
    };
    Some(egui::Id::new(crate::skin::title(ctx, i18n.tr(title))))
}

/// Where the widget SE calls `name` is on screen: its own button, else the nearest enclosing
/// toolbar or window.
fn widget_rect(ctx: &egui::Context, i18n: &I18n, name: &str) -> Option<egui::Rect> {
    let mut path = name.trim().to_string();
    loop {
        if let Some(r) = marked(ctx, &path) {
            return Some(r);
        }
        if let Some(i) = ScriptGui::toolbar_index(&path) {
            return area(ctx, egui::Id::new(TOOLBARS[i].1));
        }
        if let Some(r) = window_id(ctx, i18n, &path).and_then(|id| area(ctx, id)) {
            return Some(r);
        }
        let (parent, _) = path.rsplit_once('|')?;
        path = parent.to_string();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_gui(
    mut contexts: EguiContexts,
    time: Res<Time>,
    mut gui: ResMut<ScriptGui>,
    hud: Res<HudState>,
    i18n: Res<I18n>,
    cam: Res<FlyCamera>,
    sim: Res<SimTime>,
    data: Res<SeData>,
    wps: Res<crate::waypoints::Waypoints>,
    cams: Query<&Camera, With<MainCamera>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    gui.update(time.delta_secs_f64());
    // Toolbars slide off their screen edges (main left, filters right, the two bottom ones down).
    let screen = ctx.content_rect();
    for (i, (_, id)) in TOOLBARS.iter().enumerate() {
        let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new(*id));
        let s = gui.slide[i];
        let shift = match ctx.memory(|m| m.area_rect(egui::Id::new(*id))) {
            Some(r) if s > 0.0 => match i {
                0 => egui::vec2(-(r.right() + 8.0) * s, 0.0),
                1 => egui::vec2((screen.right() - r.left() + 8.0) * s, 0.0),
                _ => egui::vec2(0.0, (screen.bottom() - r.top() + 8.0) * s),
            },
            _ => egui::Vec2::ZERO,
        };
        ctx.set_transform_layer(layer, egui::emath::TSTransform::from_translation(shift));
    }
    // The screen fade, over the scene and its overlays, under the windows.
    if let Some(f) = &gui.fade {
        let a = f.alpha() * f.color[3];
        if a > 0.0 {
            let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("script_fade"));
            ctx.move_to_top(layer);
            let c = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0).round() as u8;
            let color = egui::Color32::from_rgba_premultiplied(c(f.color[0]), c(f.color[1]), c(f.color[2]), (a.clamp(0.0, 1.0) * 255.0) as u8);
            ctx.layer_painter(layer).rect_filled(screen, 0.0, color);
        }
    }
    let Some(h) = gui.highlight.clone().filter(|h| h.lit()) else { return Ok(()) };
    let color = crate::waypoints::premul(h.color, 1.0);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("script_highlight")));
    if h.widget.eq_ignore_ascii_case("SelectionPointer") {
        // The selected object (or waypoint) on screen.
        let Ok(camera) = cams.single() else { return Ok(()) };
        let Some(vp) = camera.logical_viewport_size() else { return Ok(()) };
        let view = crate::overlay::View::new(&cam, vp);
        let eph = Ephemeris::new(&data.catalog);
        let target = match (cam.selected, wps.selected()) {
            (Some(s), _) => Some((eph.position(s, sim.jd), eph.radius_m(s))),
            (None, Some(w)) => Some(crate::waypoints::Waypoints::place(w, &eph, sim.jd)),
            _ => None,
        };
        if let Some((pos, r)) = target {
            let local = view.local(pos.delta_m(cam.pos));
            if let Some(c) = view.project(local) {
                let r_px = ((view.focal * r / (-local.z).max(1e-9)) as f32).clamp(6.0, vp.min_element() * 0.4);
                painter.circle_stroke(c, r_px + 10.0, egui::Stroke::new(3.0, color));
            }
        }
        return Ok(());
    }
    if hud.hidden && !h.widget.eq_ignore_ascii_case("Message box") {
        return Ok(());
    }
    if let Some(r) = widget_rect(ctx, &i18n, &h.widget) {
        let fill = egui::Color32::from_rgba_premultiplied(
            (color.r() as f32 * 0.3) as u8,
            (color.g() as f32 * 0.3) as u8,
            (color.b() as f32 * 0.3) as u8,
            (color.a() as f32 * 0.3) as u8,
        );
        painter.rect(r.expand(2.0), 4.0, fill, egui::Stroke::new(2.5, color), egui::StrokeKind::Outside);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_blinks_and_expires() {
        let mut h = ActiveHighlight::new(&Highlight { widget: "Main toolbar|Wiki".into(), time: Some(1.0), ..Default::default() }).unwrap();
        assert_eq!((h.freq, h.color), (HIGHLIGHT_FREQ, HIGHLIGHT_COLOR));
        assert!(h.lit());
        // Four blinks a second: off from 0.125 s to 0.25 s.
        h.elapsed = 0.2;
        assert!(!h.lit());
        h.elapsed = 0.3;
        assert!(h.lit() && !h.expired());
        h.elapsed = 1.0;
        assert!(h.expired());
        assert!(ActiveHighlight::new(&Highlight::default()).is_none());
    }

    #[test]
    fn untimed_highlight_ends_with_the_wait() {
        let mut gui = ScriptGui::default();
        gui.highlight = ActiveHighlight::new(&Highlight { widget: "Message box".into(), ..Default::default() });
        gui.update(10.0);
        assert!(gui.highlight.is_some());
        gui.wait_ended();
        assert!(gui.highlight.is_none());
    }

    #[test]
    fn toolbars_slide_and_fades_run() {
        let mut gui = ScriptGui::default();
        gui.toolbars[2] = true;
        gui.update(0.1);
        assert!((gui.slide[2] - 0.5).abs() < 1e-5 && gui.slide[0] == 0.0);
        gui.start_fade(true, Some(2.0), None);
        gui.update(1.0);
        assert!((gui.fade.as_ref().unwrap().alpha() - 0.5).abs() < 1e-5);
        gui.update(1.0);
        gui.start_fade(false, Some(0.5), None);
        gui.update(0.6);
        assert!(gui.fade.is_none());
        assert_eq!(se_widget_name("Main toolbar", "Find"), "Main toolbar|Find object");
        assert_eq!(ScriptGui::toolbar_index("navigation toolbar"), Some(2));
    }
}
