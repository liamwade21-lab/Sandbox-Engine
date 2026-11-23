//! The cockpit's side panels (step 4.2, first cut): the left panel's Navigation tab and the right panel's tabs (Home, Modules
//! with on and off, Fire groups, Ship functions, Status), drawn as flat shapes and text (4.1's fallback: Elite's holograms
//! aren't built). Elite's own keys (its `.binds`): 1 the left panel, 4 the right, Q and E the tabs, W and S (or the arrows) the rows,
//! Space selects, A and D change a setting, Backspace closes; head look turned far left or right focuses that panel. While a
//! panel is focused those keys work it, not the ship, as in Elite; the mouse still flies. Since T4e-4 an open panel hangs at
//! Elite's place beside the seat (`hud_anchor`), and opening one by its key turns the head to it and narrows the view
//! (`seat::head_aim`, the turns and the views measured on the user's pictures); getting out closes it. What each tab lists and
//! does is ours
//! where Elite's panels hold more (F): the Navigation tab lists this system's bodies and the stars within the jump's range
//! (the spec's F5), nearest first.

use bevy::prelude::*;
use bevy_egui::egui;
use se_core::catalog::ObjectRef;

use super::hud_anchor::{SeatView, Sketch, LEFT_PANEL, PANEL_BOX, PANEL_H, PANEL_W, RIGHT_PANEL};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A row of the Navigation tab.
#[derive(Clone, Debug)]
pub struct NavRow {
    pub obj: ObjectRef,
    pub name: String,
    pub kind: &'static str,
    pub dist_m: f64,
    /// A star of another system (a hyperspace target), not a body of this one.
    pub other_system: bool,
}

/// What a panel asks the ship to do (`fly` does it, with the same rules as the keys).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelAction {
    Target(ObjectRef),
    NavBody(ObjectRef),
    FlightAssist,
    Gear,
    Hardpoints,
    Module(usize),
}

#[derive(Resource, Default)]
pub struct Panels {
    pub open: Option<Side>,
    /// Opened by head look: it closes when the look comes back.
    pub by_look: bool,
    /// Each side's tab, and the highlighted row.
    pub tab: [usize; 2],
    pub row: usize,
    /// The Navigation tab's rows (made by `fly`, which has the bodies and the catalog), and the actions asked this frame.
    pub nav: Vec<NavRow>,
    pub actions: Vec<PanelAction>,
}

pub const LEFT_TABS: [&str; 1] = ["NAVIGATION"];
pub const RIGHT_TABS: [&str; 5] = ["HOME", "MODULES", "FIRE GROUPS", "SHIP", "STATUS"];

/// The stock Sidewinder's modules (the journal's loadout, 2026-11-14) and whether the panel can switch each off (Elite's power
/// plant and its core distributor can't be: E; the rest can, and the ones that do something here are the thrusters, the frame
/// shift drive, the shields, the lasers and the sensors).
pub const MODULES: [(&str, bool); 10] = [
    ("Power plant 2E", false),
    ("Thrusters 2E", true),
    ("Frame shift drive 2E", true),
    ("Life support 1E", true),
    ("Power distributor 1E", false),
    ("Sensors 1E", true),
    ("Fuel tank 1C", false),
    ("Shield generator 2E", true),
    ("Pulse laser 1F, left", true),
    ("Pulse laser 1F, right", true),
];
pub const M_THRUSTERS: usize = 1;
pub const M_FSD: usize = 2;
pub const M_LIFE_SUPPORT: usize = 3;
pub const M_SENSORS: usize = 5;
pub const M_SHIELDS: usize = 7;
pub const M_LASERS: [usize; 2] = [8, 9];

/// The panel keys of a frame (`PilotKeys` reads them from Elite's binds).
#[derive(Clone, Copy, Debug, Default)]
pub struct PanelKeys {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub change_left: bool,
    pub change_right: bool,
    pub select: bool,
    pub back: bool,
    pub next_tab: bool,
    pub prev_tab: bool,
}

impl Panels {
    /// Closes the open panel, however it was opened (getting out of the seat: `seat::free_seat`); each side's tab is kept.
    pub fn close(&mut self) {
        self.open = None;
        self.by_look = false;
        self.row = 0;
    }

    /// How many rows the open tab has (`ship` the Ship functions' count).
    fn rows(&self) -> usize {
        match (self.open, self.tab) {
            (Some(Side::Left), _) => self.nav.len(),
            (Some(Side::Right), [_, 1]) => MODULES.len(),
            (Some(Side::Right), [_, 3]) => SHIP_FUNCTIONS.len(),
            _ => 0,
        }
    }

    /// A frame of the panel keys and the head look (`look_yaw` radians, left positive, `None` without head look).
    pub fn update(&mut self, k: &PanelKeys, look_yaw: Option<f32>) {
        self.actions.clear();
        let toggle = |p: &mut Panels, side: Side| {
            p.open = if p.open == Some(side) { None } else { Some(side) };
            p.by_look = false;
            p.row = 0;
        };
        if k.left {
            toggle(self, Side::Left);
        }
        if k.right {
            toggle(self, Side::Right);
        }
        match look_yaw {
            Some(y) if y > LOOK_FOCUS_RAD && self.open.is_none() => {
                self.open = Some(Side::Left);
                self.by_look = true;
            }
            Some(y) if y < -LOOK_FOCUS_RAD && self.open.is_none() => {
                self.open = Some(Side::Right);
                self.by_look = true;
            }
            Some(y) if self.by_look && y.abs() < LOOK_FOCUS_RAD * 0.6 => {
                self.open = None;
                self.by_look = false;
            }
            None if self.by_look => {
                self.open = None;
                self.by_look = false;
            }
            _ => {}
        }
        let Some(side) = self.open else { return };
        if k.back {
            self.open = None;
            return;
        }
        let s = side as usize;
        let tabs = if side == Side::Left { LEFT_TABS.len() } else { RIGHT_TABS.len() };
        if k.next_tab {
            self.tab[s] = (self.tab[s] + 1) % tabs;
            self.row = 0;
        }
        if k.prev_tab {
            self.tab[s] = (self.tab[s] + tabs - 1) % tabs;
            self.row = 0;
        }
        let n = self.rows();
        if n == 0 {
            self.row = 0;
            return;
        }
        if k.down {
            self.row = (self.row + 1) % n;
        }
        if k.up {
            self.row = (self.row + n - 1) % n;
        }
        self.row = self.row.min(n - 1);
        let act = k.select || k.change_left || k.change_right;
        if !act {
            return;
        }
        match (side, self.tab[s]) {
            (Side::Left, _) if k.select => {
                if let Some(r) = self.nav.get(self.row) {
                    self.actions.push(if r.other_system { PanelAction::Target(r.obj) } else { PanelAction::NavBody(r.obj) });
                }
            }
            (Side::Right, 1) if MODULES[self.row].1 => self.actions.push(PanelAction::Module(self.row)),
            (Side::Right, 3) => self.actions.push(SHIP_FUNCTIONS[self.row].1),
            _ => {}
        }
    }
}

/// The locked row's marker's height, a share of the row's height, and its width, a share of its height (ours, F).
const LOCK_MARK: f32 = 0.5;
const LOCK_MARK_W: f32 = 0.7;

/// How far head look must turn (radians) to focus a side panel.
pub const LOOK_FOCUS_RAD: f32 = 0.6;

pub const SHIP_FUNCTIONS: [(&str, PanelAction); 3] = [("Flight assist", PanelAction::FlightAssist), ("Landing gear", PanelAction::Gear), ("Hardpoints", PanelAction::Hardpoints)];

/// A distance as Elite's panels show it: metres, km, light seconds, light years.
pub fn distance_text(m: f64) -> String {
    const LS: f64 = 299_792_458.0;
    if m >= 0.05 * se_core::units::METERS_PER_LY {
        format!("{:.2} LY", m / se_core::units::METERS_PER_LY)
    } else if m >= LS {
        format!("{:.1} LS", m / LS)
    } else if m >= 1000.0 {
        format!("{:.0} KM", m / 1000.0)
    } else {
        format!("{:.0} M", m)
    }
}

/// A height over the ground as the planet HUD's altimeter shows it (metres under 10 km, else km to a tenth); a destination
/// whose distance is that height shows it the same way (`DestMark::distance_text`).
pub fn altitude_text(m: f64) -> String {
    if m.abs() >= 10_000.0 {
        format!("{:.1} KM", m / 1000.0)
    } else {
        format!("{:.0} M", m)
    }
}

/// What the right panel's text tabs say (lines), from the ship's state.
pub struct StatusLines {
    pub home: Vec<String>,
    pub fire: Vec<String>,
    pub status: Vec<String>,
}

/// The panel's columns in its own layout (`rect`, its whole frame there): the part of it across that is on the screen
/// (`across`, `SeatView::visible_across`: -1..1 left to right), so the tabs, the rows' text and their right-hand column stay
/// on the screen where the window's view cuts the panel (the right panel at Elite's turn on the user's screen). The frame and
/// the background stay the whole panel; where nothing is cut the columns are the whole panel.
pub fn columns(rect: egui::Rect, across: (f32, f32)) -> egui::Rect {
    let x = |s: f32| rect.left() + 0.5 * (s + 1.0) * rect.width();
    egui::Rect::from_x_y_ranges(x(across.0)..=x(across.1), rect.y_range())
}

/// Draws the open panel on its anchor beside the seat (`hud_anchor::LEFT_PANEL`, `RIGHT_PANEL`: Elite's place, seen when the
/// head turns to it), laid out in its own shares (`hud_anchor::PANEL_W` x `PANEL_H`, (0, 0) its top left), its columns in
/// the part of it on the screen (`columns`). `on` is each module's switch, `ship` each ship function's state, `locked` the
/// ship's destination (its Navigation row is marked).
#[allow(clippy::too_many_arguments)]
pub fn draw(p: &egui::Painter, r: egui::Rect, view: &SeatView, panels: &Panels, on: &[bool; 10], ship: [bool; 3], lines: &StatusLines, locked: Option<ObjectRef>, orange: egui::Color32, dim: egui::Color32) {
    let Some(side) = panels.open else { return };
    let anchor = if side == Side::Left { &LEFT_PANEL } else { &RIGHT_PANEL };
    let h = r.height();
    let at = |x: f32, y: f32| egui::pos2(r.center().x + x * h, r.center().y + y * h);
    let mut g = Sketch::new(p, r);
    let rect = egui::Rect::from_min_max(at(0.0, 0.0), at(PANEL_W, PANEL_H));
    let cols = columns(rect, view.visible_across(anchor));
    let w = cols.width();
    g.rect_filled(rect, 4.0, egui::Color32::from_rgba_unmultiplied(20, 8, 0, 170));
    g.rect_stroke(rect, 4.0, egui::Stroke::new((0.0016 * h).max(1.0), orange), egui::StrokeKind::Inside);
    let font = |s: f32| egui::FontId::monospace(s * h);
    let s = side as usize;
    let tabs: &[&str] = if side == Side::Left { &LEFT_TABS } else { &RIGHT_TABS };
    let mut tx = cols.left() + 0.01 * h;
    for (i, t) in tabs.iter().enumerate() {
        let c = if i == panels.tab[s] { orange } else { dim };
        let galley = g.layout_no_wrap(t.to_string(), font(0.011), c);
        let tw = galley.size().x;
        g.galley(egui::pos2(tx, rect.top() + 0.008 * h), galley, c);
        if i == panels.tab[s] {
            g.line_segment([egui::pos2(tx, rect.top() + 0.026 * h), egui::pos2(tx + tw, rect.top() + 0.026 * h)], egui::Stroke::new(0.002 * h, orange));
        }
        tx += tw + 0.014 * h;
    }
    let row_h = 0.022 * h;
    let top = rect.top() + 0.04 * h;
    let line = |g: &mut Sketch, i: usize, text: String, right: String, lit: bool, selected: bool, marked: bool| {
        let y = top + row_h * i as f32;
        if y + row_h > rect.bottom() {
            return;
        }
        if selected {
            g.rect_filled(egui::Rect::from_min_size(egui::pos2(cols.left() + 0.004 * h, y), egui::vec2(w - 0.008 * h, row_h)), 2.0, orange.gamma_multiply(0.35));
        }
        // The locked destination's row: a filled triangle pointing at it at the row's left edge, half the row high (ours, F:
        // Elite's panel art is not built); every row's text starts past it, so the columns stay aligned.
        let (half, mark_w) = (row_h * LOCK_MARK / 2.0, row_h * LOCK_MARK * LOCK_MARK_W);
        let x = cols.left() + 0.0045 * h;
        if marked {
            let m = y + row_h * 0.5;
            g.add(egui::Shape::convex_polygon(vec![egui::pos2(x, m - half), egui::pos2(x + mark_w, m), egui::pos2(x, m + half)], orange, egui::Stroke::NONE));
        }
        let c = if lit { orange } else { dim };
        g.text(egui::pos2(cols.left() + 0.012 * h + mark_w, y + row_h * 0.5), egui::Align2::LEFT_CENTER, text, font(0.0115), c);
        g.text(egui::pos2(cols.right() - 0.012 * h, y + row_h * 0.5), egui::Align2::RIGHT_CENTER, right, font(0.0115), c);
    };
    match (side, panels.tab[s]) {
        (Side::Left, _) => {
            // (Long lists scroll with the highlight.)
            let shown = ((rect.height() - 0.05 * h) / row_h) as usize;
            let first = panels.row.saturating_sub(shown.saturating_sub(1));
            for (k, n) in panels.nav.iter().enumerate().skip(first).take(shown) {
                line(&mut g, k - first, format!("{} {}", n.kind, n.name), distance_text(n.dist_m), true, k == panels.row, Some(n.obj) == locked);
            }
            if panels.nav.is_empty() {
                line(&mut g, 0, "Nothing in range".to_string(), String::new(), false, false, false);
            }
        }
        (Side::Right, 1) => {
            for (i, (name, switchable)) in MODULES.iter().enumerate() {
                let state = if !switchable { "ALWAYS ON" } else if on[i] { "ON" } else { "OFF" };
                line(&mut g, i, name.to_string(), state.to_string(), on[i], i == panels.row, false);
            }
        }
        (Side::Right, 3) => {
            let words = [("ON", "OFF"), ("DOWN", "UP"), ("DEPLOYED", "RETRACTED")];
            for (i, (name, _)) in SHIP_FUNCTIONS.iter().enumerate() {
                line(&mut g, i, name.to_string(), (if ship[i] { words[i].0 } else { words[i].1 }).to_string(), true, i == panels.row, false);
            }
        }
        (Side::Right, t) => {
            let ls = match t {
                0 => &lines.home,
                2 => &lines.fire,
                _ => &lines.status,
            };
            for (i, l) in ls.iter().enumerate() {
                line(&mut g, i, l.clone(), String::new(), true, false, false);
            }
        }
    }
    g.text(egui::pos2(cols.center().x, rect.bottom() - 0.012 * h), egui::Align2::CENTER_CENTER, "Q E tabs  W S rows  Space select  A D change  Backspace close", egui::FontId::proportional(0.009 * h), dim);
    g.finish(view, anchor, PANEL_BOX);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The columns are the part of the panel's layout across that is on the screen: all of it with nothing cut.
    #[test]
    fn the_columns_are_the_visible_part() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(886.0, 500.0));
        assert_eq!(columns(rect, (-1.0, 1.0)), rect);
        let c = columns(rect, (-1.0, 0.8));
        assert_eq!((c.left(), c.top(), c.bottom()), (rect.left(), rect.top(), rect.bottom()));
        assert!((c.right() - (rect.left() + 0.9 * rect.width())).abs() < 1e-3);
    }

    /// 1 and 4 open and close the panels; Q/E change tabs; W/S move round the rows; Space on the Navigation tab targets a star
    /// of another system or picks a body; on Modules it switches a module that can be switched; head look focuses a side.
    #[test]
    fn panel_keys() {
        let mut p = Panels::default();
        let press = |f: fn(&mut PanelKeys)| {
            let mut k = PanelKeys::default();
            f(&mut k);
            k
        };
        p.nav = vec![
            NavRow { obj: ObjectRef::Body(1), name: "Earth".into(), kind: "PLANET", dist_m: 1e6, other_system: false },
            NavRow { obj: ObjectRef::Star(7), name: "Rigil Kentaurus".into(), kind: "STAR", dist_m: 4e16, other_system: true },
        ];
        p.update(&press(|k| k.left = true), None);
        assert_eq!(p.open, Some(Side::Left));
        p.update(&press(|k| k.down = true), None);
        p.update(&press(|k| k.select = true), None);
        assert_eq!(p.actions, vec![PanelAction::Target(ObjectRef::Star(7))]);
        p.update(&press(|k| k.down = true), None);
        p.update(&press(|k| k.select = true), None);
        assert_eq!(p.actions, vec![PanelAction::NavBody(ObjectRef::Body(1))]);
        p.update(&press(|k| k.right = true), None);
        assert_eq!(p.open, Some(Side::Right));
        p.update(&press(|k| k.next_tab = true), None);
        assert_eq!(p.tab[1], 1);
        p.update(&press(|k| k.select = true), None);
        assert!(p.actions.is_empty(), "the power plant can't be switched");
        p.update(&press(|k| k.down = true), None);
        p.update(&press(|k| k.change_right = true), None);
        assert_eq!(p.actions, vec![PanelAction::Module(M_THRUSTERS)]);
        p.update(&press(|k| k.back = true), None);
        assert_eq!(p.open, None);
        p.update(&PanelKeys::default(), Some(-0.8));
        assert_eq!((p.open, p.by_look), (Some(Side::Right), true));
        p.update(&PanelKeys::default(), Some(0.0));
        assert_eq!(p.open, None);
        assert_eq!(distance_text(4.367 * se_core::units::METERS_PER_LY), "4.37 LY");
        assert_eq!(distance_text(3.0e8), "1.0 LS");
    }
}
