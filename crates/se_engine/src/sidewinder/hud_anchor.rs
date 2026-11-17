//! Where the seat's HUD hangs in the cockpit (task T4e-4): every group of the HUD has an anchor, a rectangle in the ship's
//! space, and is drawn through it each frame with the camera the seat really uses (its eye, the head's turn, Elite's field
//! of view), so it moves with the cockpit when the head turns, as Elite's does. What each group shows stays `hud.rs`'s and
//! `panels.rs`'s: they draw it as before, in the old flat layout's coordinates (shares of the screen's height from its
//! middle, y down), into a `Sketch`; `Sketch::finish` maps every vertex of it onto its anchor and projects it.
//!
//! The anchors' numbers (`re/elite/notes/sidewinder_hud.md` has the inventory and each one's source; the scripts are
//! `.optimize/scratch/run/t4e4_*.py`): each is given in the cockpit's frame, the eye at its origin (the eye is the cockpit
//! model's `!Camera` bone, its origin, and the cockpit hangs on the hull's `!Cockpit` with no turn: the bone's quaternion
//! y is 2.2e-5), in the ship's axes (x right, y up, -z the nose, metres); a point of the ship's space is the eye plus it.
//! Each anchor carries two grades: its plane's (where its depth and facing come from) and its extent's (where on that
//! plane and how big). The places Elite's own files give are the tutorial's outlines round its HUD's parts
//! (`cockpit_ui_hightlight_*`, hung at the cockpit's `!TutorialGuide_*` bones) and the cockpit's panel bones (`!Radar`,
//! `!Left_Panel`, `!MiniRight_Panel`, `!LRail_Panel`...); where an element sits inside its outline is measured on the
//! user's straight-ahead picture `1.webp` (2000 px wide): Elite's element's parts at their pixels, cast from the eye onto
//! the plane, with the camera under which the decoded outlines line up with that picture (the mod's eye at a focal of
//! 672 px, 62.86 degrees: T4b-1's measure; at the settings' 56.249 the outlines miss Elite's elements by 15 to 45 px,
//! which says the real eye sits about 0.17 m behind the mod's).

use bevy::math::{Quat, Vec2, Vec3};
use bevy_egui::egui;

/// Where a grade comes from (the spec's grades).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    /// Elite's own files: a decoded outline or bone of the cockpit.
    A,
    /// Measured on the user's pictures of Elite (the pixels and the plane in the anchor's comment).
    D,
    /// Ours: the old flat layout's place, kept (no Elite place is known).
    E,
}

/// A rectangle in the cockpit's frame (the eye at its origin, ship axes, metres): its middle and its half-vectors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub centre: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    /// The plane's grade and the extent's.
    pub grade: (Grade, Grade),
}

/// The part of a group's own layout that fills its anchor: its middle `x`, `y` and half-sizes `hw`, `hh`, in shares of the
/// screen's height from its middle, y down (the old flat layout's coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignBox {
    pub x: f32,
    pub y: f32,
    pub hw: f32,
    pub hh: f32,
}

const fn anchor(centre: [f32; 3], right: [f32; 3], up: [f32; 3], grade: (Grade, Grade)) -> Anchor {
    Anchor { centre, right, up, grade }
}

/// The radar's ring. Plane (A): facing the eye through the middle of the decoded `cockpit_ui_hightlight_radar` outline
/// (0, -0.698, -1.890; the outline lies near flat on the dashboard, but the old ring is already drawn as seen from the seat,
/// and the eye never moves: head look turns about it, so a plane facing the eye is seen exactly as the outline's ring).
/// Extent (D): the ring's extremes in 1.webp, left (878, 667), right (1127, 668), top (1002, 610), bottom (1002, 727)
/// (residuals 1 mm).
pub const RADAR: Anchor = anchor([0.0063, -0.7190, -1.8825], [0.3487, -0.0012, 0.0004], [0.0002, 0.1436, -0.0530], (Grade::A, Grade::D));
pub const RADAR_BOX: DesignBox = DesignBox { x: 0.0, y: 0.354, hw: 0.2014, hh: 0.083 };

/// The heat's figure, on the radar's frame. Plane (A): the radar's. Extent (D): Elite's "Ø34%" in 1.webp, its ends (901, 612)
/// and (927, 612), its cap height 607 to 618 (residuals 1 mm).
pub const HEAT: Anchor = anchor([-0.2475, -0.5805, -1.9337], [0.0373, 0.0002, -0.0001], [-0.0001, 0.0145, -0.0054], (Grade::A, Grade::D));
pub const HEAT_BOX: DesignBox = DesignBox { x: -0.125, y: 0.292, hw: 0.0265, hh: 0.011 };

/// The speed and the throttle's gauge. Plane (A): facing the eye through the middle of the decoded
/// `cockpit_ui_hightlight_throttle` outline (0.287, -0.657, -1.760). Extent (D): in 1.webp the speed's figure (1127, 627) and
/// the gauge's arc right of the radar, its foot (1148, 738) and top (1171, 664) (residuals 11 to 24 mm: the old layout's
/// speed sits otherwise against its gauge).
pub const THROTTLE: Anchor = anchor([0.3875, -0.7094, -1.7241], [0.0654, -0.0005, 0.0108], [0.0079, 0.1167, -0.0422], (Grade::A, Grade::D));
pub const THROTTLE_BOX: DesignBox = DesignBox { x: 0.205, y: 0.383, hw: 0.04, hh: 0.075 };

/// The ship's hologram ring with the shields and the hull. Plane (A): the decoded `cockpit_ui_hightlight_health` outline's
/// (at `!TutorialGuide_Health`, turned 15 degrees towards the eye). Extent (D): the shields' ring's extremes in 1.webp, left
/// (1198, 627), right (1355, 627), top (1277, 583), bottom (1277, 672) (residuals 2.5 mm).
pub const HEALTH: Anchor = anchor([0.5338, -0.4197, -1.3013], [0.1370, 0.0118, 0.0367], [0.0, 0.0861, 0.0], (Grade::A, Grade::D));
pub const HEALTH_BOX: DesignBox = DesignBox { x: 0.385, y: 0.303, hw: 0.108, hh: 0.055 };

/// Fuel and the indicators. Plane (A): the decoded `cockpit_ui_hightlight_masslock` outline's. Extent (D): in 1.webp "FUEL"
/// (1507, 651), the figure (1590, 659), the bar's ends (1491, 664) and (1620, 675), the first and third boxes (1496, 679) and
/// (1496, 704) (residuals 1 to 8 mm).
pub const FUEL: Anchor = anchor([1.0354, -0.5175, -1.2539], [0.0893, 0.0057, 0.0441], [-0.0034, 0.0669, -0.0017], (Grade::A, Grade::D));
pub const FUEL_BOX: DesignBox = DesignBox { x: 0.779, y: 0.3975, hw: 0.096, hh: 0.0645 };

/// The pips. Plane (A): the cockpit's `!MiniRight_Panel` bone's (no outline of their own). Extent (D): in 1.webp SYS's and
/// WEP's bars' feet (1398, 681) and (1461, 681) and their labels (1398, 694) and (1461, 700) (residuals 2 mm).
pub const PIPS: Anchor = anchor([0.8659, -0.5171, -1.3486], [0.0690, 0.0089, 0.0251], [-0.0069, 0.0600, -0.0023], (Grade::A, Grade::D));
pub const PIPS_BOX: DesignBox = DesignBox { x: 0.605, y: 0.361, hw: 0.060, hh: 0.049 };

/// The target and the destination. Plane (A): the decoded `cockpit_ui_hightlight_targetpanel` outline's (Elite's target
/// panel, lower left). Extent (D): Elite's first two lines there in 1.webp, the first's middle (492, 636), the second's
/// middle (525, 656) and left end (420, 665) (residuals up to 27 mm: Elite's lines are left aligned, the old ones centred).
pub const TARGET: Anchor = anchor([-0.8788, -0.4201, -1.2031], [0.1228, 0.0106, -0.0593], [-0.0011, 0.0164, 0.0006], (Grade::A, Grade::D));
pub const TARGET_BOX: DesignBox = DesignBox { x: 0.0, y: 0.469, hw: 0.15, hh: 0.018 };

/// The messages under the view (the notes, the drive's charge, a held drive, the way out): no Elite place known. Plane (A):
/// facing the eye at the radar outline's middle. Extent (E): the old layout's, seen straight ahead at 56.249 degrees.
pub const MESSAGES: Anchor = anchor([0.0, -0.6524, -1.9071], [0.5097, 0.0, 0.0], [0.0, 0.1812, -0.0669], (Grade::A, Grade::E));
pub const MESSAGES_BOX: DesignBox = DesignBox { x: 0.0, y: 0.32, hw: 0.25, hh: 0.10 };

/// The top of the view (the planet HUD's heading, the oxygen's clock, the jump's countdown): no Elite place known. Plane
/// (A): facing the eye at the cockpit's `!Alert_Panel` bone (0, 0.297, -0.768). Extent (E): the old layout's straight ahead.
pub const TOP: Anchor = anchor([0.0, 0.2138, -0.8001], [0.2138, 0.0, 0.0], [0.0, 0.1245, 0.0482], (Grade::A, Grade::E));
pub const TOP_BOX: DesignBox = DesignBox { x: 0.0, y: -0.25, hw: 0.25, hh: 0.16 };

/// The planet HUD's altitude and vertical speed, on the cockpit's right rail `!RRail_Panel` (A: its place and facing; Elite's
/// rails either side of the view at eye height, the mapping ours). Extent (E): the old layout's size at the bone's depth.
pub const ALTITUDE: Anchor = anchor([0.5108, -0.0169, -1.3258], [0.0709, 0.0, 0.0004], [0.0, 0.0177, -0.0001], (Grade::A, Grade::E));
pub const ALTITUDE_BOX: DesignBox = DesignBox { x: 0.35, y: 0.0325, hw: 0.05, hh: 0.0125 };

/// The planet HUD's pitch and roll, on the left rail `!LRail_Panel` (as `ALTITUDE`).
pub const ATTITUDE: Anchor = anchor([-0.5106, -0.0172, -1.3239], [0.0708, 0.0, 0.0004], [0.0, 0.0177, -0.0001], (Grade::A, Grade::E));
pub const ATTITUDE_BOX: DesignBox = DesignBox { x: -0.35, y: 0.0325, hw: 0.05, hh: 0.0125 };

/// The side panels' own layout: (0, 0) the panel's top left, `PANEL_W` x `PANEL_H` shares (the width from the measured
/// panel's 0.887 x 0.633 m).
pub const PANEL_W: f32 = 0.701;
pub const PANEL_H: f32 = 0.5;
pub const PANEL_BOX: DesignBox = DesignBox { x: PANEL_W / 2.0, y: PANEL_H / 2.0, hw: PANEL_W / 2.0, hh: PANEL_H / 2.0 };

/// The left panel (Navigation). Plane (A): the cockpit's `!Left_Panel` bone's (-1.046, -0.391, -0.577, facing the eye from
/// 70 degrees left). Extent (D): its frame's corners in the user's 2.webp, (193, 350), (869, 335), (895, 705), (208, 778),
/// with the head's turn there fitted from parts seen in both pictures (39.25 degrees left, 7.25 down; residuals 2 to 19 px),
/// cast onto the plane (residuals 3 to 7 cm: the four corners don't make one rectangle on it).
pub const LEFT_PANEL: Anchor = anchor([-1.1095, -0.3391, -0.4063], [0.1532, -0.0286, -0.4131], [0.0054, 0.3143, -0.0197], (Grade::A, Grade::D));
/// The right panel: the left's mirror (the bones are mirrors within 3 mm). The check, 3.webp's top corners at its fitted
/// head turn (38.5 degrees right, 2 down): (1120, 419) and (1834, 387) against the picture's (1158, 405) and (1910, 420).
pub const RIGHT_PANEL: Anchor = anchor([1.1095, -0.3391, -0.4063], [0.1532, 0.0286, 0.4131], [-0.0054, 0.3143, -0.0197], (Grade::A, Grade::D));

/// The head's turn as `seat::seat_camera` makes it (yaw about the ship's up, then pitch).
pub fn head_turn(look: Vec2) -> Quat {
    Quat::from_rotation_y(look.x) * Quat::from_rotation_x(look.y)
}

/// Nothing nearer the eye than this along the view is drawn (m).
const NEAR_M: f32 = 0.05;

/// The seat's camera this frame, as the cockpit is drawn with it: the eye (ship space), the head's turn, the vertical field
/// of view, and the screen (egui's points).
#[derive(Clone, Copy, Debug)]
pub struct SeatView {
    eye: Vec3,
    turn_inv: Quat,
    focal: f32,
    screen: egui::Rect,
}

impl SeatView {
    pub fn new(eye: Vec3, look: Vec2, fov_deg: f32, screen: egui::Rect) -> SeatView {
        SeatView { eye, turn_inv: head_turn(look).inverse(), focal: 0.5 * screen.height() / (0.5 * fov_deg.to_radians()).tan(), screen }
    }

    /// A point of the ship's space on the screen; None behind the eye (or nearer than `NEAR_M`).
    pub fn project(&self, p: Vec3) -> Option<egui::Pos2> {
        let c = self.turn_inv * (p - self.eye);
        (c.z < -NEAR_M).then(|| self.screen.center() + egui::vec2(self.focal * c.x / -c.z, -self.focal * c.y / -c.z))
    }

    /// A point of `anchor` (`s`, `t` in -1..1 across its right and up half-vectors) in the ship's space.
    fn on(&self, anchor: &Anchor, s: f32, t: f32) -> Vec3 {
        self.eye + Vec3::from_array(anchor.centre) + s * Vec3::from_array(anchor.right) + t * Vec3::from_array(anchor.up)
    }

    /// The anchor's corners on the screen, or None when one is behind the eye or the whole rectangle is off the screen.
    pub fn corners(&self, anchor: &Anchor) -> Option<[egui::Pos2; 4]> {
        let c = [self.project(self.on(anchor, -1.0, 1.0))?, self.project(self.on(anchor, 1.0, 1.0))?, self.project(self.on(anchor, 1.0, -1.0))?, self.project(self.on(anchor, -1.0, -1.0))?];
        let bounds = egui::Rect::from_points(&c);
        bounds.intersects(self.screen).then_some(c)
    }
}

/// One group of the HUD drawn in the old flat layout's coordinates (`rect` the screen: a point is `rect`'s middle plus
/// shares of its height), collected as shapes until `finish` puts them on their anchor. The drawing methods are the
/// painter's that the HUD uses.
pub struct Sketch<'a> {
    painter: &'a egui::Painter,
    rect: egui::Rect,
    shapes: Vec<egui::Shape>,
}

impl<'a> Sketch<'a> {
    pub fn new(painter: &'a egui::Painter, rect: egui::Rect) -> Sketch<'a> {
        Sketch { painter, rect, shapes: Vec::new() }
    }

    pub fn add(&mut self, shape: impl Into<egui::Shape>) {
        self.shapes.push(shape.into());
    }

    pub fn line_segment(&mut self, points: [egui::Pos2; 2], stroke: impl Into<egui::Stroke>) {
        self.add(egui::Shape::line_segment(points, stroke));
    }

    pub fn circle_filled(&mut self, centre: egui::Pos2, radius: f32, colour: egui::Color32) {
        self.add(egui::Shape::circle_filled(centre, radius, colour));
    }

    pub fn rect_filled(&mut self, rect: egui::Rect, corner: f32, colour: egui::Color32) {
        self.add(egui::Shape::rect_filled(rect, corner, colour));
    }

    pub fn rect_stroke(&mut self, rect: egui::Rect, corner: f32, stroke: impl Into<egui::Stroke>, kind: egui::StrokeKind) {
        self.add(egui::Shape::rect_stroke(rect, corner, stroke, kind));
    }

    pub fn layout_no_wrap(&self, text: String, font: egui::FontId, colour: egui::Color32) -> std::sync::Arc<egui::Galley> {
        self.painter.layout_no_wrap(text, font, colour)
    }

    pub fn galley(&mut self, pos: egui::Pos2, galley: std::sync::Arc<egui::Galley>, colour: egui::Color32) {
        if !galley.is_empty() {
            self.add(egui::Shape::galley(pos, galley, colour));
        }
    }

    pub fn text(&mut self, pos: egui::Pos2, align: egui::Align2, text: impl ToString, font: egui::FontId, colour: egui::Color32) {
        let galley = self.layout_no_wrap(text.to_string(), font, colour);
        let at = align.anchor_size(pos, galley.size()).min;
        self.galley(at, galley, colour);
    }

    /// Puts the group on its anchor: every vertex of its shapes (tessellated as egui would) is taken from the layout's
    /// coordinates to the anchor (`design` fills it) and projected with `view`. Nothing is drawn when the anchor is behind
    /// the eye or off the screen, or a vertex falls behind the eye.
    pub fn finish(self, view: &SeatView, anchor: &Anchor, design: DesignBox) {
        if self.shapes.is_empty() || view.corners(anchor).is_none() {
            return;
        }
        let ctx = self.painter.ctx();
        let mut tess = egui::epaint::Tessellator::new(ctx.pixels_per_point(), ctx.tessellation_options(|o| *o), ctx.fonts(|f| f.font_image_size()), Vec::new());
        let mut mesh = egui::Mesh::default();
        for shape in self.shapes {
            tess.tessellate_shape(shape, &mut mesh);
        }
        let (mid, h) = (self.rect.center(), self.rect.height());
        for v in &mut mesh.vertices {
            let s = ((v.pos.x - mid.x) / h - design.x) / design.hw;
            let t = -((v.pos.y - mid.y) / h - design.y) / design.hh;
            match view.project(view.on(anchor, s, t)) {
                Some(p) => v.pos = p,
                None => return,
            }
        }
        self.painter.add(egui::Shape::mesh(mesh));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Mat4, Vec4Swizzles};
    use bevy::camera::CameraProjection;
    use bevy::prelude::{PerspectiveProjection, Transform};

    const FOV: f32 = super::super::seat::SEAT_FOV_DEG;
    const EYE: Vec3 = Vec3::new(0.0001, 6.7223, 1.3465);

    fn screen(w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))
    }

    /// A point straight ahead of the eye lands in the screen's middle at 1600 x 900 and at 3440 x 1440; one behind the eye
    /// isn't drawn, nor is an anchor behind it.
    #[test]
    fn ahead_is_the_middle_and_behind_is_nothing() {
        for (w, h) in [(1600.0, 900.0), (3440.0, 1440.0)] {
            let view = SeatView::new(EYE, Vec2::ZERO, FOV, screen(w, h));
            let p = view.project(EYE + Vec3::new(0.0, 0.0, -1.5)).unwrap();
            assert!((p - egui::pos2(w / 2.0, h / 2.0)).length() < 1e-3, "{p:?}");
            let ahead = anchor([0.0, 0.0, -1.5], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0], (Grade::A, Grade::A));
            let c = view.corners(&ahead).unwrap();
            assert!((egui::Rect::from_points(&c).center() - egui::pos2(w / 2.0, h / 2.0)).length() < 1e-3);
            assert!(view.project(EYE + Vec3::new(0.0, 0.0, 1.5)).is_none());
            let behind = anchor([0.0, 0.0, 1.5], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0], (Grade::A, Grade::A));
            assert!(view.corners(&behind).is_none());
        }
        // Off the screen (far to the right of a straight view) is nothing too; the head turned to it shows it.
        let view = SeatView::new(EYE, Vec2::ZERO, FOV, screen(1600.0, 900.0));
        let side = anchor([5.0, 0.0, -1.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0], (Grade::A, Grade::A));
        assert!(view.corners(&side).is_none());
        assert!(SeatView::new(EYE, Vec2::new(-1.37, 0.0), FOV, screen(1600.0, 900.0)).corners(&side).is_some());
    }

    /// With the head turned 30 degrees (left, and left and down), a cockpit point lands where Bevy's own camera puts it,
    /// the camera made as `seat::seat_camera` makes it (the eye, the ship's turn times the head's) with Elite's field of
    /// view: the HUD and the cockpit's picture agree.
    #[test]
    fn a_head_turn_matches_the_cockpits_projection() {
        let ship = Quat::from_euler(bevy::math::EulerRot::YXZ, 0.7, -0.2, 0.3);
        let at = Vec3::new(12.0, -4.0, 30.0);
        let (w, h) = (3440.0, 1440.0);
        for look in [Vec2::new(30f32.to_radians(), 0.0), Vec2::new(30f32.to_radians(), -0.25)] {
            let view = SeatView::new(EYE, look, FOV, screen(w, h));
            let cam = Transform { translation: at + ship * EYE, rotation: (ship * head_turn(look)).normalize(), ..Default::default() };
            let proj = PerspectiveProjection { fov: FOV.to_radians(), aspect_ratio: w / h, ..Default::default() };
            let clip_from_world: Mat4 = proj.get_clip_from_view() * cam.to_matrix().inverse();
            for p in [RADAR.centre, LEFT_PANEL.centre, HEALTH.centre, [-0.3, 0.1, -1.2]] {
                let ship_p = EYE + Vec3::from_array(p);
                let Some(ours) = view.project(ship_p) else { continue };
                let c = clip_from_world * (at + ship * ship_p).extend(1.0);
                let ndc = c.xyz() / c.w;
                let bevy = egui::pos2((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h);
                assert!((ours - bevy).length() < 0.1, "{p:?}: ours {ours:?}, Bevy's {bevy:?}");
            }
        }
    }

    /// Every anchor is in front of the eye with real half-vectors, and seen straight ahead the HUD's groups are on the
    /// screen while the side panels are not (they show with the head turned to them).
    #[test]
    fn the_anchors_are_where_the_cockpit_is() {
        let view = SeatView::new(EYE, Vec2::ZERO, FOV, screen(1600.0, 900.0));
        for a in [RADAR, HEAT, THROTTLE, HEALTH, FUEL, PIPS, TARGET, MESSAGES, TOP, ALTITUDE, ATTITUDE] {
            assert!(a.centre[2] < -0.5 && Vec3::from_array(a.right).length() > 0.01 && Vec3::from_array(a.up).length() > 0.01, "{a:?}");
            assert!(view.corners(&a).is_some(), "{a:?} off the straight view");
        }
        for (a, yaw) in [(LEFT_PANEL, 0.69f32), (RIGHT_PANEL, -0.67)] {
            assert!(view.corners(&a).map_or(true, |c| !egui::Rect::from_points(&c).intersects(screen(1600.0, 900.0).shrink(400.0))));
            let turned = SeatView::new(EYE, Vec2::new(yaw, -0.12), FOV, screen(1600.0, 900.0));
            let c = turned.corners(&a).unwrap();
            assert!(screen(1600.0, 900.0).contains(egui::Rect::from_points(&c).center()), "{c:?}");
        }
    }
}
