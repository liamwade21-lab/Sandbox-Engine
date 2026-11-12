//! Script waypoints: SE's `Waypoint`, `DeleteWaypoint` and `ClearWaypoints` (seWaypointList,
//! FUN_140065f40 / FUN_140549e90 / FUN_140561280), the markers the tutorials send the player
//! through.
//!
//! A waypoint is a shape of `RadiusKm` placed relative to a parent body (`FixedPos*` turns with
//! the body, `StaticPos*` does not). SE draws it as a billboard facing the camera (FUN_14054b0e0:
//! circle, triangle, square, diamond and hexagon meshes, a sphere or a wire sphere), in `Color`
//! with the inside in `ColorEmpty`, both scaled by `WaypointsOpacity`, blended premultiplied so a
//! colour with alpha 0 adds light; `ShowLabelsWaypoints` adds the name. Scripts select waypoints
//! by name (`Select "WP 1"`) and wait for the camera to reach them (`Object|Approach`). SE
//! clears the list when a script ends (FUN_14054d240).
//!
//! The port draws the shapes with egui on the overlay layer, at their true size in the view, and
//! a selection pointer for a selected waypoint (the catalog selection cannot hold one).

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::catalog::{Catalog, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::se_script::{WaypointDef, WaypointShape};
use se_core::upos::UPos;

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::universe::SimTime;

pub struct WaypointsPlugin;

impl Plugin for WaypointsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Waypoints>()
            .add_systems(Update, drop_selection)
            .add_systems(EguiPrimaryContextPass, draw_waypoints.in_set(crate::skin::UiSet::Draw));
    }
}

/// One waypoint (FUN_14054aeb0's record).
#[derive(Debug, Clone, PartialEq)]
pub struct Waypoint {
    pub name: String,
    pub parent: Option<ObjectRef>,
    pub radius_km: f64,
    pub roll_deg: f64,
    pub color: [f32; 4],
    pub color_empty: [f32; 4],
    pub label_color: Option<[f32; 4]>,
    pub line_width: f32,
    pub shape: WaypointShape,
    /// Offset from the parent's centre, km (body frame when `fixed`).
    pub pos_km: DVec3,
    pub fixed: bool,
    pub visible: bool,
    pub label: bool,
    pub triggerable: bool,
}

impl Waypoint {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            parent: None,
            radius_km: WaypointDef::DEFAULT_RADIUS_KM,
            roll_deg: 0.0,
            color: WaypointDef::DEFAULT_COLOR,
            color_empty: [0.0; 4],
            label_color: None,
            line_width: WaypointDef::DEFAULT_LINE_WIDTH,
            shape: WaypointShape::Circle,
            pos_km: DVec3::ZERO,
            fixed: false,
            visible: true,
            label: true,
            triggerable: true,
        }
    }

    /// Applies the keys a `Waypoint` command gives.
    fn apply(&mut self, d: &WaypointDef, parent: Option<ObjectRef>) {
        if d.parent.is_some() {
            self.parent = parent;
        }
        macro_rules! take {
            ($($f:ident),*) => { $( if let Some(v) = d.$f.clone() { self.$f = v; } )* };
        }
        take!(radius_km, roll_deg, color, color_empty, line_width, shape, visible, label, triggerable);
        if d.label_color.is_some() {
            self.label_color = d.label_color;
        }
        if let Some(p) = d.pos {
            self.pos_km = DVec3::from_array(p.km);
            self.fixed = p.fixed;
        }
    }
}

/// The script's waypoints, `WaypointsOpacity` and `ShowLabelsWaypoints`.
#[derive(Resource, Debug)]
pub struct Waypoints {
    pub list: Vec<Waypoint>,
    /// The waypoint the script selected (`Select "WP 1"`).
    pub selected: Option<String>,
    pub opacity: f32,
    pub labels: bool,
}

impl Default for Waypoints {
    fn default() -> Self {
        // main-def.cfg: WaypointsOpacity 1 (FUN_140014d10), ShowLabelsWaypoints true.
        Self { list: Vec::new(), selected: None, opacity: 1.0, labels: true }
    }
}

impl Waypoints {
    /// SE's `Waypoint` (FUN_140065f40): change the waypoint of that name, or add a new one.
    /// Fails, adding nothing, when the parent is not in the catalog.
    pub fn define(&mut self, d: &WaypointDef, cat: &Catalog) -> Result<(), String> {
        let parent = match &d.parent {
            Some(p) => Some(crate::scripting::resolve_object(cat, p).ok_or_else(|| format!("waypoint parent \"{p}\" is not in the catalog"))?),
            None => None,
        };
        match self.list.iter_mut().find(|w| w.name.eq_ignore_ascii_case(&d.name)) {
            Some(w) => w.apply(d, parent),
            None => {
                let mut w = Waypoint::new(&d.name);
                w.apply(d, parent);
                self.list.push(w);
            }
        }
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> bool {
        let n = self.list.len();
        self.list.retain(|w| !w.name.eq_ignore_ascii_case(name));
        if self.selected.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(name)) {
            self.selected = None;
        }
        self.list.len() != n
    }

    pub fn clear(&mut self) {
        self.list.clear();
        self.selected = None;
    }

    /// A waypoint by name, as scripts write it (`"WP 1"`, or a path ending in it).
    pub fn find(&self, name: &str) -> Option<&Waypoint> {
        let last = name.rsplit('|').next().unwrap_or(name).trim();
        self.list.iter().find(|w| w.name.eq_ignore_ascii_case(name)).or_else(|| self.list.iter().find(|w| w.name.eq_ignore_ascii_case(last)))
    }

    pub fn selected(&self) -> Option<&Waypoint> {
        self.selected.as_deref().and_then(|n| self.find(n))
    }

    /// Where a waypoint is, and its radius in metres.
    pub fn place(w: &Waypoint, eph: &Ephemeris, jd: f64) -> (UPos, f64) {
        let off = w.pos_km * 1000.0;
        let pos = match w.parent {
            Some(p) => {
                let q = if w.fixed { eph.orientation(p, jd) } else { DQuat::IDENTITY };
                eph.position(p, jd).offset_m(q * off)
            }
            None => UPos::ZERO.offset_m(off),
        };
        (pos, (w.radius_km * 1000.0).max(1e-3))
    }
}

/// Selecting an object replaces a selected waypoint.
fn drop_selection(cam: Res<FlyCamera>, mut wps: ResMut<Waypoints>) {
    if cam.selected.is_some() && wps.selected.is_some() {
        wps.selected = None;
    }
}

/// An SE colour (HDR, premultiplied) times `opacity` as an egui colour.
pub fn premul(c: [f32; 4], opacity: f32) -> egui::Color32 {
    let f = |v: f32| ((v * opacity).clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_premultiplied(f(c[0]), f(c[1]), f(c[2]), f(c[3]))
}

/// Outline points of a shape of radius `r` px around `c`, turned by `roll` (radians).
pub fn shape_points(shape: WaypointShape, c: egui::Pos2, r: f32, roll: f32) -> Vec<egui::Pos2> {
    let (n, start) = match shape {
        WaypointShape::Triangle => (3, std::f32::consts::FRAC_PI_2),
        WaypointShape::Square => (4, std::f32::consts::FRAC_PI_4),
        WaypointShape::Diamond => (4, 0.0),
        WaypointShape::Hexagon => (6, 0.0),
        _ => (64, 0.0),
    };
    (0..n)
        .map(|i| {
            let a = start + roll + i as f32 * std::f32::consts::TAU / n as f32;
            // Screen y runs down.
            c + egui::vec2(a.cos(), -a.sin()) * r
        })
        .collect()
}

fn draw_waypoints(
    mut contexts: EguiContexts,
    wps: Res<Waypoints>,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    sim: Res<SimTime>,
    cams: Query<&Camera, With<MainCamera>>,
    control: Res<crate::mods::ModControl>,
) -> Result {
    // (Nor over the main menu.)
    if wps.list.is_empty() || control.main_menu {
        return Ok(());
    }
    let Ok(camera) = cams.single() else { return Ok(()) };
    let Some(vp) = camera.logical_viewport_size() else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(crate::overlay::overlay_layer());
    let view = crate::overlay::View::new(&cam, vp);
    let eph = Ephemeris::new(&data.catalog);
    let font = egui::FontId::proportional(13.0);
    let max_px = vp.length() * 2.0;
    for w in wps.list.iter().filter(|w| w.visible) {
        let (pos, r) = Waypoints::place(w, &eph, sim.jd);
        let local = view.local(pos.delta_m(cam.pos));
        let selected = wps.selected.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&w.name));
        let Some(c) = view.project(local) else {
            if selected {
                edge_pointer(&painter, vp, local);
            }
            continue;
        };
        let dist = local.length();
        if dist <= r {
            continue;
        }
        let r_px = ((view.focal * r / (-local.z).max(1e-9)) as f32).max(1.5);
        if r_px > max_px {
            continue;
        }
        // Hidden behind its parent body (SE draws the shapes in the scene, under the bodies); the
        // selection pointer still shows where it is.
        if let Some(p) = w.parent {
            let to_body = eph.position(p, sim.jd).delta_m(cam.pos);
            if behind_sphere(pos.delta_m(cam.pos), to_body, eph.radius_m(p)) {
                if selected {
                    ticks(&painter, c, r_px);
                }
                continue;
            }
        }
        let line = premul(w.color, wps.opacity);
        let fill = premul(w.color_empty, wps.opacity);
        let stroke = egui::Stroke::new(w.line_width.max(0.5), line);
        let roll = w.roll_deg.to_radians() as f32;
        match w.shape {
            WaypointShape::Sphere => {
                painter.circle(c, r_px, premul(w.color, wps.opacity * 0.35), stroke);
            }
            WaypointShape::WireSphere => {
                painter.circle_stroke(c, r_px, stroke);
                // An equator and a meridian of the wire sphere, seen from slightly above.
                let thin = egui::Stroke::new((w.line_width * 0.5).max(0.5), line);
                for (sx, sy) in [(1.0, 0.35), (0.35, 1.0)] {
                    let pts: Vec<egui::Pos2> =
                        (0..48).map(|i| i as f32 * std::f32::consts::TAU / 48.0).map(|a| c + egui::vec2(a.cos() * r_px * sx, a.sin() * r_px * sy)).collect();
                    painter.add(egui::Shape::closed_line(pts, thin));
                }
            }
            shape => {
                let pts = shape_points(shape, c, r_px, roll);
                if fill.a() > 0 || fill.r() > 0 || fill.g() > 0 || fill.b() > 0 {
                    painter.add(egui::Shape::convex_polygon(pts.clone(), fill, egui::Stroke::NONE));
                }
                painter.add(egui::Shape::closed_line(pts, stroke));
            }
        }
        if wps.labels && w.label {
            let lc = premul(w.label_color.unwrap_or(w.color), wps.opacity);
            painter.text(c + egui::vec2(r_px + 4.0, 0.0), egui::Align2::LEFT_CENTER, &w.name, font.clone(), lc);
        }
        if selected {
            ticks(&painter, c, r_px);
        }
    }
    Ok(())
}

/// Whether the sphere at `centre` (radius `r`) hides the point `p` (both from the camera, m).
pub fn behind_sphere(p: DVec3, centre: DVec3, r: f64) -> bool {
    if r <= 0.0 || centre.length() <= r {
        return false;
    }
    let len2 = p.length_squared().max(1e-30);
    let t = (centre.dot(p) / len2).clamp(0.0, 1.0);
    let miss2 = (p * t - centre).length_squared();
    if miss2 >= r * r {
        return false;
    }
    // Where the line of sight enters the sphere, as a fraction of the way to the point.
    let t_in = t - ((r * r - miss2).sqrt() / len2.sqrt());
    t_in < 1.0
}

/// The selection pointer's colour (as the overlay's).
const POINTER: egui::Color32 = egui::Color32::from_rgb(80, 208, 216);

/// Four ticks pointing in at a selected waypoint.
fn ticks(painter: &egui::Painter, c: egui::Pos2, r_px: f32) {
    let r_in = (r_px + 4.0).max(7.0);
    for i in 0..4 {
        let a = std::f32::consts::FRAC_PI_4 + i as f32 * std::f32::consts::FRAC_PI_2;
        let d = egui::vec2(a.cos(), a.sin());
        painter.line_segment([c + d * r_in, c + d * (r_in + 10.0)], egui::Stroke::new(2.0, POINTER));
    }
}

/// A hollow triangle on the screen's edge pointing at a waypoint behind or off the view.
fn edge_pointer(painter: &egui::Painter, vp: Vec2, local: DVec3) {
    let dir = egui::vec2(local.x as f32, -local.y as f32);
    let dir = if dir.length() < 1e-6 { egui::vec2(0.0, 1.0) } else { dir.normalized() };
    let c = egui::pos2(vp.x * 0.5, vp.y * 0.5);
    let k = ((vp.x * 0.5 - 24.0) / dir.x.abs().max(1e-6)).min((vp.y * 0.5 - 24.0) / dir.y.abs().max(1e-6));
    let tip = c + dir * k;
    let side = egui::vec2(-dir.y, dir.x);
    let pts = vec![tip, tip - dir * 16.0 + side * 8.0, tip - dir * 16.0 - side * 8.0];
    painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(2.0, POINTER)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::se_script::WaypointPos;

    fn def(name: &str) -> WaypointDef {
        WaypointDef { name: name.into(), ..Default::default() }
    }

    #[test]
    fn redefining_changes_only_given_keys() {
        let cat = Catalog::default();
        let mut wps = Waypoints::default();
        let mut d = def("WP 1");
        d.radius_km = Some(500.0);
        d.color = Some([2.0, 2.0, 0.2, 0.5]);
        d.pos = Some(WaypointPos { km: [1.0, 2.0, 3.0], fixed: true });
        wps.define(&d, &cat).unwrap();
        let mut d2 = def("wp 1");
        d2.radius_km = Some(200.0);
        wps.define(&d2, &cat).unwrap();
        assert_eq!(wps.list.len(), 1);
        let w = wps.find("WP 1").unwrap();
        assert_eq!((w.radius_km, w.color, w.fixed), (200.0, [2.0, 2.0, 0.2, 0.5], true));
        assert_eq!(w.line_width, 2.0);
        // A missing parent is an error and adds nothing.
        let mut d3 = def("WP 2");
        d3.parent = Some("No such object".into());
        assert!(wps.define(&d3, &cat).is_err());
        assert!(wps.find("WP 2").is_none());
        assert!(wps.find("Some|Path|WP 1").is_some());
        assert!(wps.delete("WP 1") && wps.list.is_empty());
    }

    #[test]
    fn premultiplied_colours() {
        // Alpha 0 still shows (additive), HDR values clamp.
        let c = premul([1.0, 1.0, 0.0, 0.0], 1.0);
        assert_eq!((c.r(), c.g(), c.b(), c.a()), (255, 255, 0, 0));
        let c = premul([2.0, 2.0, 0.2, 0.5], 0.5);
        assert_eq!((c.r(), c.b(), c.a()), (255, 26, 64));
        let c = premul([0.5, 0.0, 0.5, 1.0], 0.5);
        assert_eq!((c.r(), c.a()), (64, 128));
    }

    #[test]
    fn occlusion_by_the_parent() {
        let body = DVec3::new(0.0, 0.0, -100.0);
        // Straight behind the body, in front of it, and beside it.
        assert!(behind_sphere(DVec3::new(0.0, 0.0, -200.0), body, 10.0));
        assert!(!behind_sphere(DVec3::new(0.0, 0.0, -50.0), body, 10.0));
        assert!(!behind_sphere(DVec3::new(30.0, 0.0, -200.0), body, 10.0));
        // On the near surface's side of the body, still in front of it.
        assert!(!behind_sphere(DVec3::new(0.0, 0.0, -89.0), body, 10.0));
    }

    #[test]
    fn shapes() {
        let c = egui::pos2(0.0, 0.0);
        let tri = shape_points(WaypointShape::Triangle, c, 10.0, 0.0);
        assert_eq!(tri.len(), 3);
        // The triangle points up (screen y down).
        assert!((tri[0].y + 10.0).abs() < 1e-4 && tri[0].x.abs() < 1e-4);
        assert_eq!(shape_points(WaypointShape::Hexagon, c, 1.0, 0.0).len(), 6);
        let sq = shape_points(WaypointShape::Square, c, 1.0, 0.0);
        assert!((sq[0].x - sq[0].y.abs()).abs() < 1e-5);
    }
}
