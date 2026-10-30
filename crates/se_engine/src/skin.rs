//! SpaceEngine's interface skins (`textures/common/Skins.pak`: 25 `* skin.cfg` over four widget
//! textures; main-def.cfg `Skin "Flat Blue"`). egui draws its own flat widgets, so a skin becomes
//! egui's visuals: the colours its texture gives each widget (the `tc*` rectangles of the window,
//! button, edit field, slider and tab) recoloured by `TextureHSL` and times `WinColor*`, its
//! `TextColor*`, its dialog shadow (`DialogShadowWidth`) and capital dialog headers
//! (`DialogHeaderCapital`). Every window shows the blurred frame behind it as SE's gui.glsl does
//! (`BLUR_BACKGROUND`): `BackgroundBrightness` of the tone-mapped blurred frame
//! ([`crate::bloom::GuiBackdrop`]) added over the window.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, EguiTextureHandle, egui};

use crate::data::SeData;

pub const DEFAULT_SKIN: &str = "Flat Blue";
const DIR: &str = "textures/common/";

/// The interface's drawing order in egui's pass: the windows, then their blurred backgrounds.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum UiSet {
    Draw,
    Backdrop,
}

/// The chosen skin (settings `Skin`).
#[derive(Resource)]
pub struct Skin {
    pub name: String,
    /// The skins in the install.
    pub available: Vec<String>,
    applied: Option<String>,
    /// `BackgroundBrightness`, 0 without `BackgroundBluriness`.
    backdrop: f32,
    /// The windows' corner radius, for their blurred background.
    corner: u8,
}

impl Default for Skin {
    /// SE's default skin, or env `SE_SKIN` (testing; the saved choice wins outside scripted runs).
    fn default() -> Self {
        let name = std::env::var("SE_SKIN").unwrap_or_else(|_| DEFAULT_SKIN.to_string());
        Self { name, available: Vec::new(), applied: None, backdrop: 0.0, corner: 3 }
    }
}

/// What the skin gives the interface's text, kept in egui's context so that every window can read it
/// ([`title`], [`text`]).
#[derive(Clone)]
pub struct SkinText {
    /// `DialogHeaderCapital`: window titles in capitals.
    pub header_capital: bool,
    /// `ObjectInfoColor`: the info table's text.
    pub info: egui::Color32,
    /// `Info*Color` by kind (`Star`, `Planet`, `Moon`, ... as in the keys): the info table's name and
    /// the kind's labels, markers and orbit lines.
    kinds: HashMap<String, egui::Color32>,
    /// `BodySelectedOrbitColor`: the selection's orbit line.
    pub selected_orbit: egui::Color32,
    /// Every `*Color` key as written (RGBA), e.g. the grids' and constellations' line colours.
    colors: HashMap<String, [f32; 4]>,
}

impl Default for SkinText {
    fn default() -> Self {
        Self {
            header_capital: false,
            info: egui::Color32::from_rgb(153, 230, 255),
            kinds: HashMap::new(),
            selected_orbit: egui::Color32::from_rgb(255, 51, 51),
            colors: HashMap::new(),
        }
    }
}

impl SkinText {
    /// `Info<kind>Color` (e.g. `"Planet"`), else `InfoDefaultColor`.
    pub fn kind(&self, kind: &str) -> egui::Color32 {
        self.kinds.get(kind).or_else(|| self.kinds.get("Default")).copied().unwrap_or(self.info)
    }

    /// A skin colour key's RGBA (e.g. `"ConstelLinesColor"`), else `default` (SE's own).
    pub fn rgba(&self, key: &str, default: [f32; 4]) -> [f32; 4] {
        self.colors.get(key).copied().unwrap_or(default)
    }
}

fn text_id() -> egui::Id {
    egui::Id::new("se_skin_text")
}

/// The skin's text settings.
pub fn text(ctx: &egui::Context) -> SkinText {
    ctx.data(|d| d.get_temp::<SkinText>(text_id())).unwrap_or_default()
}

/// A window title as the skin prints it.
pub fn title(ctx: &egui::Context, s: &str) -> String {
    if text(ctx).header_capital { s.to_uppercase() } else { s.to_string() }
}

pub struct SkinPlugin;

impl Plugin for SkinPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Skin>()
            .configure_sets(EguiPrimaryContextPass, (UiSet::Draw, UiSet::Backdrop).chain())
            .add_systems(EguiPrimaryContextPass, (apply_skin.before(UiSet::Draw), paint_backdrops.in_set(UiSet::Backdrop)));
    }
}

/// A skin file's keys: `Key value` or `Key (a, b, c)` per line, `//` comments.
struct SkinCfg(HashMap<String, String>);

impl SkinCfg {
    fn parse(text: &str) -> Self {
        let mut keys = HashMap::new();
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once(char::is_whitespace) else { continue };
            keys.insert(k.to_string(), v.trim().to_string());
        }
        Self(keys)
    }

    fn nums(&self, key: &str) -> Option<Vec<f32>> {
        let v = self.0.get(key)?;
        let v = v.trim_start_matches('(').trim_end_matches(')');
        let n: Vec<f32> = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        (!n.is_empty()).then_some(n)
    }

    fn f(&self, key: &str, default: f32) -> f32 {
        self.nums(key).map_or(default, |n| n[0])
    }

    fn rgba(&self, key: &str, default: [f32; 4]) -> [f32; 4] {
        self.nums(key).filter(|n| n.len() >= 4).map_or(default, |n| [n[0], n[1], n[2], n[3]])
    }

    fn bool(&self, key: &str) -> bool {
        self.0.get(key).is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    fn str(&self, key: &str) -> Option<String> {
        self.0.get(key).map(|v| v.trim_matches('"').to_string())
    }
}

/// The skin's widget texture, recoloured by `TextureHSL` (hue shift in degrees, saturation and
/// lightness in percent).
struct SkinTexture {
    img: image::RgbaImage,
    hsl: [f32; 3],
}

impl SkinTexture {
    fn px(&self, x: f32, y: f32) -> [f32; 4] {
        let (x, y) = ((x as u32).min(self.img.width() - 1), (y as u32).min(self.img.height() - 1));
        let p = self.img.get_pixel(x, y).0;
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        // HSV, as SE's hsl.glh.
        let max = r.max(g).max(b);
        let d = max - r.min(g).min(b);
        let mut h = if d <= 0.0 {
            0.0
        } else if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } / 6.0;
        let s = if max > 0.0 { d / max } else { 0.0 };
        h = (h + self.hsl[0] / 360.0).rem_euclid(1.0);
        let (s, v) = ((s * self.hsl[1] / 100.0).clamp(0.0, 1.0), (max * self.hsl[2] / 100.0).clamp(0.0, 1.0));
        let f = |n: f32| {
            let k = (n + h * 6.0) % 6.0;
            v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
        };
        [f(5.0), f(3.0), f(1.0), p[3] as f32 / 255.0]
    }

    /// A scalable widget (`tc*` with 8 values: x edges and borders, y edges and borders): its fill
    /// (the centre) and its outline (the most opaque texel crossing the left border at mid-height).
    fn widget(&self, tc: &[f32]) -> ([f32; 4], [f32; 4]) {
        if tc.len() < 8 {
            let c = self.px((tc[0] + tc[1]) * 0.5, (tc[2] + tc[3]) * 0.5);
            return (c, c);
        }
        let fill = self.px((tc[1] + tc[2]) * 0.5, (tc[5] + tc[6]) * 0.5);
        let y = (tc[5] + tc[6]) * 0.5;
        let edge = (tc[0] as u32..tc[1].max(tc[0] + 1.0) as u32)
            .map(|x| self.px(x as f32, y))
            .fold([0.0; 4], |best, p| if p[3] > best[3] { p } else { best });
        (fill, edge)
    }
}

fn color(c: [f32; 4], mul: [f32; 4]) -> egui::Color32 {
    let u = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(u(c[0] * mul[0]), u(c[1] * mul[1]), u(c[2] * mul[2]), u(c[3] * mul[3]))
}

/// Reads a skin and sets egui's visuals from it.
fn load(vfs: &se_core::vfs::Vfs, name: &str, ctx: &egui::Context, skin: &mut Skin) -> Option<()> {
    let cfg = SkinCfg::parse(&vfs.read_string(&format!("{DIR}{name} skin.cfg")).ok()?);
    let tex_name = cfg.str("SkinTexture").unwrap_or_else(|| "interface_mat.*".into()).replace(".*", ".png");
    let img = image::load_from_memory(&vfs.read(&format!("{DIR}{tex_name}")).ok()?).ok()?.to_rgba8();
    let hsl = cfg.nums("TextureHSL").filter(|n| n.len() >= 3).map_or([0.0, 100.0, 100.0], |n| [n[0], n[1], n[2]]);
    let tex = SkinTexture { img, hsl };
    let widget = |key: &str| cfg.nums(key).map(|tc| tex.widget(&tc));
    let one = [1.0; 4];
    let (win_norm, win_high, win_act) = (cfg.rgba("WinColorNormal", one), cfg.rgba("WinColorHighlighted", one), cfg.rgba("WinColorActive", one));
    let text_norm = color(one, cfg.rgba("TextColorNormal", [0.7, 0.7, 0.7, 0.7]));
    let text_high = color(one, cfg.rgba("TextColorHighlighted", one));
    let text_act = color(one, cfg.rgba("TextColorActive", one));
    let text_dis = cfg.rgba("TextColorDisabled", [0.2, 0.2, 0.2, 0.0]);
    let text_dis = color(one, [text_dis[0], text_dis[1], text_dis[2], text_dis[3].max(0.35)]);

    let (dialog_fill, dialog_edge) = widget("tcDialog.Norm").unwrap_or(([0.05, 0.11, 0.19, 1.0], [0.3, 0.45, 0.55, 1.0]));
    let window_fill = color(dialog_fill, win_norm);
    // An outline the same colour as the fill draws none; SE's flat dialogs have none either.
    let window_edge = color(dialog_edge, win_norm);
    let button = |state: &str| widget(&format!("tcButton.{state}")).unwrap_or(([0.0; 4], [0.3, 0.46, 0.54, 1.0]));
    let (b_norm, b_high, b_act) = (button("Norm"), button("High"), button("Act"));
    let (slide, _) = widget("tcSlideBar.Norm").unwrap_or(([0.16, 0.28, 0.39, 1.0], [0.0; 4]));
    let border = |key: &str| cfg.nums(key).filter(|tc| tc.len() >= 8).map_or(3.0, |tc| tc[1] - tc[0]);
    let corner = border("tcDialog.Norm").clamp(0.0, 8.0) as u8;
    let button_corner = border("tcButton.Norm").clamp(0.0, 9.0) as u8;
    let shadow = cfg.f("DialogShadowWidth", 0.0).clamp(0.0, 64.0) as u8;

    let mut v = egui::Visuals::dark();
    v.window_fill = window_fill;
    v.panel_fill = window_fill;
    v.window_stroke = egui::Stroke::new(1.0, window_edge);
    v.window_corner_radius = egui::CornerRadius::same(corner);
    v.menu_corner_radius = egui::CornerRadius::same(corner);
    v.window_shadow = egui::Shadow { offset: [0, 0], blur: shadow, spread: 0, color: egui::Color32::from_black_alpha(if shadow > 0 { 160 } else { 0 }) };
    v.popup_shadow = egui::Shadow { offset: [0, 0], blur: shadow / 2, spread: 0, color: egui::Color32::from_black_alpha(if shadow > 0 { 120 } else { 0 }) };
    v.extreme_bg_color = color([dialog_fill[0] * 0.6, dialog_fill[1] * 0.6, dialog_fill[2] * 0.6, 1.0], win_norm);
    v.text_edit_bg_color = Some(v.extreme_bg_color);
    // SE's table bands (alpha 0: added).
    let band = cfg.rgba("TableBandColor", [0.1, 0.1, 0.1, 0.0]);
    let u = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    v.faint_bg_color = egui::Color32::from_rgba_premultiplied(u(band[0]), u(band[1]), u(band[2]), 0);
    v.selection.bg_fill = color(slide, win_act);
    v.selection.stroke = egui::Stroke::new(1.0, text_high);
    let info = color(one, cfg.rgba("ObjectInfoColor", [0.6, 0.9, 1.0, 1.0]));
    v.hyperlink_color = info;
    let track = |state: &str| widget(&format!("tcSliderHorBack.{state}")).map_or([0.3, 0.47, 0.55, 1.0], |w| w.0);
    // egui fills buttons with `weak_bg_fill` (SE's flat buttons: their outline, a faint fill where the
    // texture has none) and slider rails and check boxes with `bg_fill` (SE's slider track, dimmed).
    let state = |w: &mut egui::style::WidgetVisuals, (fill, edge): ([f32; 4], [f32; 4]), track: [f32; 4], mul: [f32; 4], text: egui::Color32| {
        let fill = if fill[3] < 0.05 { [edge[0], edge[1], edge[2], 0.12] } else { fill };
        w.weak_bg_fill = color(fill, mul);
        w.bg_fill = color([track[0] * 0.6, track[1] * 0.6, track[2] * 0.6, track[3]], mul);
        w.bg_stroke = egui::Stroke::new(1.0, color(edge, mul));
        w.fg_stroke = egui::Stroke::new(1.0, text);
        w.corner_radius = egui::CornerRadius::same(button_corner);
    };
    state(&mut v.widgets.inactive, b_norm, track("Norm"), win_norm, text_norm);
    state(&mut v.widgets.hovered, b_high, track("High"), win_high, text_high);
    state(&mut v.widgets.active, b_act, track("Act"), win_act, text_act);
    state(&mut v.widgets.open, b_act, track("Act"), win_act, text_act);
    v.widgets.noninteractive.bg_fill = window_fill;
    v.widgets.noninteractive.weak_bg_fill = window_fill;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, color(b_norm.1, [1.0, 1.0, 1.0, 0.5]));
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_norm);
    v.widgets.noninteractive.corner_radius = egui::CornerRadius::same(corner);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.expansion = 0.0;
    v.weak_text_color = Some(text_dis);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);

    skin.backdrop = if cfg.f("BackgroundBluriness", 0.0) > 0.0 { cfg.f("BackgroundBrightness", 0.0) } else { 0.0 };
    skin.corner = corner;
    let kinds = cfg
        .0
        .keys()
        .filter_map(|k| k.strip_prefix("Info")?.strip_suffix("Color").map(|kind| (kind.to_string(), k.clone())))
        .map(|(kind, k)| (kind, color(one, cfg.rgba(&k, one))))
        .collect();
    let selected_orbit = color(one, cfg.rgba("BodySelectedOrbitColor", [1.0, 0.2, 0.2, 1.0]));
    let colors = cfg.0.keys().filter(|k| k.ends_with("Color")).filter_map(|k| Some((k.clone(), cfg.nums(k).filter(|n| n.len() >= 4).map(|n| [n[0], n[1], n[2], n[3]])?))).collect();
    let text = SkinText { header_capital: cfg.bool("DialogHeaderCapital"), info, kinds, selected_orbit, colors };
    ctx.data_mut(|d| d.insert_temp(text_id(), text));
    Some(())
}

/// Lists the skins and applies the chosen one (again when it changes).
fn apply_skin(mut contexts: EguiContexts, data: Res<SeData>, mut skin: ResMut<Skin>) -> Result {
    if skin.applied.as_deref() == Some(skin.name.as_str()) {
        return Ok(());
    }
    if skin.available.is_empty() {
        let mut list: Vec<String> = data
            .vfs
            .list_ext(DIR, "cfg")
            .iter()
            .filter_map(|p| p.rsplit('/').next()?.strip_suffix(" skin.cfg").map(str::to_string))
            .collect();
        list.sort();
        skin.available = list;
    }
    let ctx = contexts.ctx_mut()?;
    let name = skin.name.clone();
    if load(&data.vfs, &name, ctx, &mut skin).is_none() && name != DEFAULT_SKIN {
        warn!("interface skin {name:?} not found; using {DEFAULT_SKIN}");
        skin.name = DEFAULT_SKIN.to_string();
        load(&data.vfs, DEFAULT_SKIN, ctx, &mut skin);
    }
    info!("interface skin {}", skin.name);
    skin.applied = Some(skin.name.clone());
    Ok(())
}

/// Adds the blurred frame behind every window to it (egui's windows are its middle-order layers;
/// premultiplied colour with zero alpha adds).
fn paint_backdrops(
    mut contexts: EguiContexts,
    skin: Res<Skin>,
    backdrop: Option<Res<crate::bloom::GuiBackdrop>>,
    mut tex: Local<Option<egui::TextureId>>,
) -> Result {
    let Some(backdrop) = backdrop.filter(|_| skin.backdrop > 0.0) else { return Ok(()) };
    let id = *tex.get_or_insert_with(|| contexts.add_image(EguiTextureHandle::Weak(backdrop.image.id())));
    let ctx = contexts.ctx_mut()?;
    let screen = ctx.viewport_rect();
    let layers: Vec<(egui::LayerId, egui::Rect)> = ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .into_iter()
            .filter(|l| l.order == egui::Order::Middle)
            .filter_map(|l| m.area_rect(l.id).map(|r| (l, r)))
            .collect()
    });
    let k = (skin.backdrop.clamp(0.0, 1.0) * 255.0).round() as u8;
    let tint = egui::Color32::from_rgba_premultiplied(k, k, k, 0);
    for (layer, rect) in layers {
        let uv = egui::Rect::from_min_max(
            egui::pos2((rect.min.x - screen.min.x) / screen.width(), (rect.min.y - screen.min.y) / screen.height()),
            egui::pos2((rect.max.x - screen.min.x) / screen.width(), (rect.max.y - screen.min.y) / screen.height()),
        );
        let shape = egui::epaint::RectShape::filled(rect, egui::CornerRadius::same(skin.corner), tint).with_texture(id, uv);
        ctx.layer_painter(layer).add(shape);
    }
    Ok(())
}
