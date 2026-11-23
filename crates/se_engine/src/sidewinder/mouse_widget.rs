//! Elite's mouse widget (U2, ARCHITECTURE.md A9): with the binds' `MouseGUI` on, a small yellow square on the nose's axis
//! and a split triangle that sits off it along the mouse's virtual stick, the two cross-fading. Every number is from the
//! recordings, `re/elite/notes/sidewinder_hud_marks.md` section W (graded D where measured, E where this extends it).
//!
//! Places are in "tan units": a direction from the eye, fixed to the ship, as its offset on a plane 1 m in front of the eye
//! (x right, y up). The note's pixels are at 3440 x 1440 with a focal of `F_REC` px, so a length in px there is px /
//! `F_REC` here; drawn through the seat's camera (`SeatView`) the widget scales with any window and field of view and
//! follows the head's turn as the cockpit does (W3: it is fixed to the cockpit, D).
//!
//! What is not drawn: the triangle's blur-rounded tip and the glow round it (the video's blur and Elite's bloom: W2), and the
//! dot's soft glow but for its red fringe (W1).

use bevy::math::{Vec2, Vec3};
use bevy_egui::egui;

use super::hud_anchor::SeatView;

/// The recordings' focal length, px at 3440 x 1440 (vertical view 56.249 degrees): the note's "Common ground" (D).
const F_REC: f32 = 1347.4;

/// One step of the recordings' mouse, as a share of full stick: 100 counts at the user's sensitivity 0.2725, full stick at
/// `MOUSE_FULL_COUNTS` counts (the note's "Common ground", v: 0.08473).
pub const STEP_V: f32 = 100.0 * 0.2725 / elite_sim::data::measured::MOUSE_FULL_COUNTS as f32;

/// The arrow's centroid's distance from the dot, tan units: `OFFSET_SLOPE` v + `OFFSET_AT_ZERO` (W2 "The law offset(v)",
/// the fit with an intercept, both axes, steps 3 to 8: D there; below step 3 and beyond step 8 the same line, E).
const OFFSET_SLOPE: f32 = 0.1680;
const OFFSET_AT_ZERO: f32 = 0.0013;

/// The arrow's outline about its triangle's centroid, pointing along the deflection (W2 "Outline about its own centroid",
/// steps 3 to 5: D): the apex ahead of the centroid, the base behind it, the base's corners to each side.
const APEX_AHEAD: f32 = 25.4 / F_REC;
const BASE_BEHIND: f32 = 12.7 / F_REC;
const BASE_HALF: f32 = 17.5 / F_REC;
/// Half the dark split down the arrow's axis (W2's table, "split line: width at half its depth" 3.7 px: D). It is drawn
/// fully dark (the recordings' darkest level is 0.12 to 0.30 of the halves', the blur's: E).
const SPLIT_HALF: f32 = 0.5 * 3.7 / F_REC;
/// Where each half's outer edge meets the split, ahead of the centroid: the outline's edge from the base's corner to the apex
/// at `SPLIT_HALF` from the axis.
const SPLIT_TIP: f32 = APEX_AHEAD - (APEX_AHEAD + BASE_BEHIND) * SPLIT_HALF / BASE_HALF;
/// How far the drawn shape's area centroid lies behind the triangle's own centroid, tan units, from the geometry alone: the
/// shape is the triangle less its split, two equal right triangles (`arrow_halves`) whose corners lie `BASE_BEHIND` behind,
/// `BASE_BEHIND` behind and `SPLIT_TIP` ahead, so its centroid is the mean of those, (`SPLIT_TIP` - 2 `BASE_BEHIND`) / 3
/// (1.34 px at `F_REC`). The law places the arrow's PIXEL centroid (the helper's `widget2.py`: its centroid by s), so the
/// triangle is drawn this far beyond the law and the drawn shape's centroid is on it. (A remark: Elite's own fitted outline
/// lies 0.65 to 0.87 px beyond its pixel centroid measured from the dot, the checker's `dotcrop.py` / `dotfrac.py` in
/// `.optimize/scratch/run/n2_review/`: less than here, because Elite's split is only partly dark and its tip blurred,
/// where the mod draws the split fully dark and the tip sharp: E.)
const DRAWN_BEHIND: f32 = (2.0 * BASE_BEHIND - SPLIT_TIP) / 3.0;

/// Half the dot's size, x and y: its half-maximum widths 8.6 x 9.0 px (W1 "shape": D).
const DOT_HALF: Vec2 = Vec2::new(0.5 * 8.6 / F_REC, 0.5 * 9.0 / F_REC);
/// The dot's red fringe: its width (W1 "glow", 2 to 3 px: the middle, D) and its red level at the dot's full red (R 40 to
/// 65: the middle, D), scaled with the dot's own red level as it fades (E: the fringe was measured at rest only).
const FRINGE: f32 = 2.5 / F_REC;
const FRINGE_RED: f32 = 52.5;

/// The arrow's core colour on the screen at steps 0 to 8 (W2 "Offset, size and brightness per step", column "arrow peak
/// RGB", each range's middle: D). Steps 0 and 1 show no arrow; step 2's is "about (12, 0, 0)".
const ARROW_RGB: [[f32; 3]; 9] = [
    [0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0],
    [12.0, 0.0, 0.0],
    [63.0, 7.0, 0.0],
    [152.0, 35.0, 1.0],
    [210.5, 87.5, 0.0],
    [236.0, 144.0, 2.0],
    [248.5, 176.5, 0.0],
    [247.0, 181.0, 0.0],
];

/// The dot's core colour on the screen at steps 0 to 8 (the same table, column "dot core RGB", each range's middle: D).
const DOT_RGB: [[f32; 3]; 9] = [
    [253.0, 202.0, 0.0],
    [251.0, 201.0, 0.0],
    [251.0, 192.0, 0.0],
    [245.5, 178.0, 0.0],
    [241.0, 151.5, 1.0],
    [224.0, 107.5, 1.5],
    [157.0, 45.0, 5.0],
    [23.0, 0.0, 0.0],
    [22.0, 0.0, 0.0],
];

/// The arrow: its pixel centroid's place, the law's (tan units from the nose's axis, x right, y up: the drawn shape's area
/// centroid; the triangle's own centroid lies `DRAWN_BEHIND` further out), the unit direction it points in (outward,
/// along the deflection) and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrow {
    pub at: Vec2,
    pub dir: Vec2,
    pub colour: egui::Color32,
}

/// What the widget shows for a stick: the dot's colour and its fringe's, and the arrow when it is not black.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Widget {
    pub dot: egui::Color32,
    pub fringe: egui::Color32,
    pub arrow: Option<Arrow>,
}

/// A table's colour at the stick's length `v`: a straight line between the measured steps (D at the steps, E between), held
/// at the last step beyond it (E).
fn colour_at(table: &[[f32; 3]; 9], v: f32) -> [f32; 3] {
    let s = (v / STEP_V).max(0.0);
    let last = table.len() - 1;
    if s >= last as f32 {
        return table[last];
    }
    let (i, t) = (s.floor() as usize, s.fract());
    std::array::from_fn(|c| table[i][c] + (table[i + 1][c] - table[i][c]) * t)
}

fn rgb(c: [f32; 3]) -> egui::Color32 {
    let byte = |x: f32| x.round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(byte(c[0]), byte(c[1]), byte(c[2]))
}

/// The widget for the mouse's virtual stick `stick` (`Pilot::stick`, before the dead zone: x mouse right +, y mouse up +).
/// Two axes at once were never recorded: the place, the direction and the fade go by the vector's length and direction (E).
pub fn shape(stick: Vec2) -> Widget {
    let v = stick.length();
    let dot = colour_at(&DOT_RGB, v);
    let fringe = [FRINGE_RED * dot[0] / DOT_RGB[0][0], 0.0, 0.0];
    let arrow = Some(rgb(colour_at(&ARROW_RGB, v))).filter(|c| *c != egui::Color32::BLACK).map(|colour| {
        let dir = stick / v;
        Arrow { at: dir * (OFFSET_SLOPE * v + OFFSET_AT_ZERO), dir, colour }
    });
    Widget { dot: rgb(dot), fringe: rgb(fringe), arrow }
}

/// Whether the widget is drawn: the binds' `MouseGUI` is not "0" (a missing setting counts as on: E) and no side panel is
/// open (W3: with a panel focused Elite doesn't draw it, D). Free head look without a panel still draws it (E). In the
/// hyperspace tunnel it stays drawn with the stick as it was held (`fly` returns there before the stick moves: E, the
/// recordings never show the tunnel).
pub fn shown(mouse_gui: Option<&str>, panel_open: bool) -> bool {
    !panel_open && mouse_gui.is_none_or(|v| v.trim() != "0")
}

/// The arrow's two halves (tan units): right triangles back to back either side of the split, each from its base corner to
/// the split's foot and up the split to where the outline's edge meets it, about the triangle's own centroid, which lies
/// `DRAWN_BEHIND` beyond the law's place so that the halves' joint centroid is on it.
fn arrow_halves(a: &Arrow) -> [[Vec2; 3]; 2] {
    let side = a.dir.perp();
    let centre = a.at + a.dir * DRAWN_BEHIND;
    [-1.0f32, 1.0].map(|s| {
        let base = centre - a.dir * BASE_BEHIND;
        [base + side * (s * BASE_HALF), base + side * (s * SPLIT_HALF), centre + a.dir * SPLIT_TIP + side * (s * SPLIT_HALF)]
    })
}

/// A rectangle's corners from its corner `lo` to its corner `hi` (tan units), going round (top left, top right, bottom
/// right, bottom left with y up), taken as they are (so
/// rectangles that share a corner share it to the bit and meet without a seam).
fn rect_between(lo: Vec2, hi: Vec2) -> [Vec2; 4] {
    [Vec2::new(lo.x, hi.y), hi, Vec2::new(hi.x, lo.y), lo]
}

/// The dot and its fringe as five rectangles that tile the fringe's outer square without overlapping: the dot first, then
/// the fringe as a frame round it (a strip `FRINGE` wide above and below across the whole width, and one each side between
/// them), so every pixel of the dot is one shape over the scene.
fn dot_frame() -> [[Vec2; 4]; 5] {
    let (i, o) = (DOT_HALF, DOT_HALF + Vec2::splat(FRINGE));
    [
        rect_between(-i, i),
        rect_between(Vec2::new(-o.x, i.y), o),
        rect_between(-o, Vec2::new(o.x, -i.y)),
        rect_between(Vec2::new(-o.x, -i.y), Vec2::new(-i.x, i.y)),
        rect_between(Vec2::new(i.x, -i.y), Vec2::new(o.x, i.y)),
    ]
}

/// The colour a measured screen colour is drawn with: premultiplied, its alpha its largest channel, so it covers the scene
/// by its brightness (E). Over black it gives exactly the recordings' colour; over a bright background the full-bright dot
/// and arrow nearly hide it and a faint one mostly lets it through. Chosen because the rest of the mod's HUD is drawn
/// covering and Elite's HUD stays readable over bright bodies; how Elite's widget looks over a bright background is not in
/// the recordings (they are over black space, where adding and covering give the same pixels).
fn covering(c: egui::Color32) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), c.r().max(c.g()).max(c.b()))
}

/// Tan-unit corners projected through the seat's camera (`eye` in the ship's space: a corner (x, y) is the point
/// `eye + (x, y, -1)`); None when one is behind the eye.
fn project<const N: usize>(view: &SeatView, eye: Vec3, corners: [Vec2; N]) -> Option<[egui::Pos2; N]> {
    let mut pts = [egui::Pos2::ZERO; N];
    for (p, c) in pts.iter_mut().zip(corners) {
        *p = view.project(eye + Vec3::new(c.x, c.y, -1.0))?;
    }
    Some(pts)
}

/// Draws the widget for `stick` through the seat's camera `view` (the eye `eye`, ship space), every shape covering by its
/// brightness (`covering`, E). The dot and its fringe are one mesh of `dot_frame`'s rectangles (shared corners project
/// alike, so they meet without a seam or an overlap); the arrow's halves are two polygons, apart across the split. Nothing
/// of a black colour or with a corner behind the eye. (egui's mesh and polygons own their points: their only allocations.)
pub fn draw(painter: &egui::Painter, view: &SeatView, eye: Vec3, stick: Vec2) {
    let w = shape(stick);
    let mut mesh = egui::Mesh::default();
    for (k, quad) in dot_frame().into_iter().enumerate() {
        let colour = covering(if k == 0 { w.dot } else { w.fringe });
        let Some(pts) = project(view, eye, quad).filter(|_| colour != egui::Color32::TRANSPARENT) else { continue };
        let first = mesh.vertices.len() as u32;
        for p in pts {
            mesh.colored_vertex(p, colour);
        }
        mesh.add_triangle(first, first + 1, first + 2);
        mesh.add_triangle(first, first + 2, first + 3);
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
    if let Some(a) = &w.arrow {
        for half in arrow_halves(a) {
            if let Some(pts) = project(view, eye, half) {
                painter.add(egui::Shape::convex_polygon(pts.to_vec(), covering(a.colour), egui::Stroke::NONE));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::seat::Head;
    use super::*;

    /// The note's table (W2 "Offset, size and brightness per step") as written there, each colour a range per channel.
    type Range = [(f32, f32); 3];
    const ARROW_NOTE: [Option<Range>; 9] = [
        None,
        None,
        Some([(12.0, 12.0), (0.0, 0.0), (0.0, 0.0)]),
        Some([(61.0, 65.0), (6.0, 8.0), (0.0, 0.0)]),
        Some([(152.0, 152.0), (35.0, 35.0), (1.0, 1.0)]),
        Some([(209.0, 212.0), (86.0, 89.0), (0.0, 0.0)]),
        Some([(236.0, 236.0), (141.0, 147.0), (0.0, 4.0)]),
        Some([(246.0, 251.0), (175.0, 178.0), (0.0, 0.0)]),
        Some([(246.0, 248.0), (180.0, 182.0), (0.0, 0.0)]),
    ];
    const DOT_NOTE: [Range; 9] = [
        [(253.0, 253.0), (202.0, 202.0), (0.0, 0.0)],
        [(251.0, 251.0), (201.0, 201.0), (0.0, 0.0)],
        [(251.0, 251.0), (192.0, 192.0), (0.0, 0.0)],
        [(244.0, 247.0), (178.0, 178.0), (0.0, 0.0)],
        [(241.0, 241.0), (148.0, 155.0), (1.0, 1.0)],
        [(223.0, 225.0), (106.0, 109.0), (0.0, 3.0)],
        [(152.0, 162.0), (38.0, 52.0), (0.0, 10.0)],
        [(23.0, 23.0), (0.0, 0.0), (0.0, 0.0)],
        [(22.0, 22.0), (0.0, 0.0), (0.0, 0.0)],
    ];

    fn up(n: f32) -> Widget {
        shape(Vec2::new(0.0, n * STEP_V))
    }

    fn matches(c: egui::Color32, r: Range) -> bool {
        [c.r(), c.g(), c.b()].iter().zip(r).all(|(&x, (lo, hi))| (x as f32 - 0.5 * (lo + hi)).abs() <= 0.5)
    }

    /// Steps 3 to 8: the arrow's pixel centroid from the dot (the law), in px at the recordings' focal, is the note's r02
    /// column within 0.3 px.
    #[test]
    fn the_offsets_are_the_recordings() {
        for (n, px) in [(3, 59.55), (4, 78.67), (5, 97.91), (6, 117.10), (7, 136.04), (8, 155.03)] {
            let a = up(n as f32).arrow.unwrap();
            let got = a.at.length() * F_REC;
            assert!((got - px).abs() <= 0.3, "step {n}: {got:.2} px against {px}");
        }
    }

    /// The polygons actually drawn (the two halves) have their joint area centroid on the law's place within 1e-5 tan, at
    /// steps 3 to 8 up, and for a sideways and a diagonal stick: worked out numerically (each polygon's shoelace area and
    /// centroid, weighted by area), not from `DRAWN_BEHIND`.
    #[test]
    fn the_drawn_shape_is_centred_on_the_law() {
        let sticks = (3..=8).map(|n| Vec2::new(0.0, n as f32 * STEP_V)).chain([Vec2::new(4.0 * STEP_V, 0.0), Vec2::new(-0.3, 0.45)]);
        for stick in sticks {
            let a = shape(stick).arrow.unwrap();
            let (mut area, mut moment) = (0.0f64, bevy::math::DVec2::ZERO);
            for poly in arrow_halves(&a) {
                // (The halves are mirror images, wound opposite ways: each one's signed area and moment, then by its size.)
                let (mut a_i, mut m_i) = (0.0f64, bevy::math::DVec2::ZERO);
                for (p, q) in poly.iter().zip(poly.iter().cycle().skip(1)) {
                    let (p, q) = (p.as_dvec2(), q.as_dvec2());
                    let cross = p.perp_dot(q);
                    a_i += 0.5 * cross;
                    m_i += (p + q) * cross / 6.0;
                }
                area += a_i.abs();
                moment += m_i * a_i.signum();
            }
            let centroid = moment / area;
            assert!((centroid - a.at.as_dvec2()).length() < 1e-5, "stick {stick}: the drawn centroid {centroid} against the law's {}", a.at);
        }
        // By hand: 1.34 px behind the triangle's own centroid at the recordings' focal.
        assert!((DRAWN_BEHIND * F_REC - 1.343).abs() < 0.005, "{}", DRAWN_BEHIND * F_REC);
    }

    /// At every measured step the dot's and the arrow's colours are the middles of the note's ranges (to the nearest level);
    /// steps 0 and 1 have no arrow, and none at v 0.0847 either.
    #[test]
    fn the_colours_are_the_recordings() {
        for n in 0..9 {
            let w = up(n as f32);
            assert!(matches(w.dot, DOT_NOTE[n]), "step {n}: dot {:?}", w.dot);
            match (w.arrow, ARROW_NOTE[n]) {
                (None, None) => {}
                (Some(a), Some(r)) => assert!(matches(a.colour, r), "step {n}: arrow {:?}", a.colour),
                (a, r) => panic!("step {n}: arrow {a:?} against the note's {r:?}"),
            }
        }
        assert!(shape(Vec2::ZERO).arrow.is_none() && shape(Vec2::new(0.0, 0.0847)).arrow.is_none());
        // Held beyond the last step; the fringe at rest is the note's middle.
        assert_eq!(shape(Vec2::new(0.0, 1.0)).arrow.unwrap().colour, up(8.0).arrow.unwrap().colour);
        assert_eq!(shape(Vec2::ZERO).fringe, egui::Color32::from_rgb(53, 0, 0));
    }

    /// The triangle's outline: base 35.0 px wide and 38.1 px from the base to the apex at the recordings' focal; the split
    /// 3.7 px wide.
    #[test]
    fn the_triangle_is_the_recordings() {
        let a = up(4.0).arrow.unwrap();
        let [l, r] = arrow_halves(&a);
        assert!(((l[0] - r[0]).length() * F_REC - 35.0).abs() < 1e-3);
        assert!(((l[1] - r[1]).length() * F_REC - 3.7).abs() < 1e-3);
        assert!(((APEX_AHEAD + BASE_BEHIND) * F_REC - 38.1).abs() < 1e-3);
        // Each half's top lies on the outline's edge from its base corner to the apex.
        let apex = a.at + a.dir * (DRAWN_BEHIND + APEX_AHEAD);
        assert!((l[0] - apex).perp_dot(l[2] - apex).abs() < 1e-9);
    }

    /// Mouse up puts the arrow above the dot pointing up, mouse right to its right pointing right, on the screen too; with
    /// the head straight the dot is the view's middle and the arrow's place in px is the tan value times the focal.
    #[test]
    fn the_arrow_points_along_the_mouse() {
        let pointing = |stick: Vec2, want: Vec2| {
            let a = shape(stick).arrow.unwrap();
            assert!((a.dir - want).length() < 1e-6 && (a.at.normalize() - want).length() < 1e-6, "{a:?}");
            let [l, r] = arrow_halves(&a);
            let along = |p: Vec2| (p - a.at).dot(a.dir);
            assert!(along(l[2]) > 0.0 && along(r[2]) > 0.0 && along(l[0]) < 0.0 && along(r[0]) < 0.0, "outward");
        };
        pointing(Vec2::new(0.0, 0.4), Vec2::Y);
        pointing(Vec2::new(0.4, 0.0), Vec2::X);
        pointing(Vec2::new(0.0, -0.4), Vec2::NEG_Y);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(3440.0, 1440.0));
        let eye = Vec3::new(0.0001, 6.7223, 1.3465);
        let view = SeatView::new(eye, Head::default(), screen);
        let on = |t: Vec2| view.project(eye + Vec3::new(t.x, t.y, -1.0)).unwrap();
        assert!((on(Vec2::ZERO) - screen.center()).length() < 1e-3);
        let a = up(4.0).arrow.unwrap();
        let px = on(a.at) - screen.center();
        assert!(px.x.abs() < 1e-3 && (-px.y - a.at.y * F_REC).abs() < 0.05, "above the dot by the focal's px: {px:?}");
        let a = shape(Vec2::new(4.0 * STEP_V, 0.0)).arrow.unwrap();
        assert!((on(a.at) - screen.center()).x > 70.0);
    }

    /// The fringe's four rectangles overlap neither the dot's nor each other, each is `FRINGE` thick on the dot's side it
    /// lies on, and with the dot they tile the fringe's outer square (their areas add up to it).
    #[test]
    fn the_fringe_is_a_frame_round_the_dot() {
        let boxes = dot_frame().map(|q| {
            let (lo, hi) = (q.iter().fold(Vec2::INFINITY, |m, p| m.min(*p)), q.iter().fold(Vec2::NEG_INFINITY, |m, p| m.max(*p)));
            (lo, hi)
        });
        for (k, a) in boxes.iter().enumerate() {
            for b in &boxes[k + 1..] {
                let over = (a.1.min(b.1) - a.0.max(b.0)).max(Vec2::ZERO);
                assert!(over.x * over.y == 0.0, "{a:?} overlaps {b:?}");
            }
        }
        let size = |b: &(Vec2, Vec2)| b.1 - b.0;
        assert!((size(&boxes[1]).y - FRINGE).abs() < 1e-7 && (size(&boxes[2]).y - FRINGE).abs() < 1e-7);
        assert!((size(&boxes[3]).x - FRINGE).abs() < 1e-7 && (size(&boxes[4]).x - FRINGE).abs() < 1e-7);
        let outer = 4.0 * (DOT_HALF.x + FRINGE) * (DOT_HALF.y + FRINGE);
        let sum: f32 = boxes.iter().map(|b| size(b).x * size(b).y).sum();
        assert!((sum - outer).abs() < 1e-9, "{sum} against {outer}");
        assert!((size(&boxes[0]) - 2.0 * DOT_HALF).length() < 1e-7);
    }

    /// The drawn colour: premultiplied, its alpha its largest channel, the channels unchanged (so over black, exactly the
    /// measured colour); black is fully transparent.
    #[test]
    fn the_colour_covers_by_its_brightness() {
        let c = covering(egui::Color32::from_rgb(152, 35, 1));
        assert_eq!((c.r(), c.g(), c.b(), c.a()), (152, 35, 1, 152));
        assert_eq!(covering(egui::Color32::from_rgb(253, 202, 0)).a(), 253);
        assert_eq!(covering(egui::Color32::BLACK), egui::Color32::TRANSPARENT);
    }

    /// The gate: MouseGUI "0" off, "1" on, missing on; a side panel open off.
    #[test]
    fn the_gate() {
        assert!(!shown(Some("0"), false));
        assert!(shown(Some("1"), false));
        assert!(shown(None, false));
        assert!(!shown(Some("1"), true) && !shown(None, true));
    }
}
