//! Coordinate grids: SE's celestial grids (FUN_140266a90, one grid by FUN_140266db0) and a planet's
//! geographic grid (in its planet pass, FUN_1404c7460).
//!
//! The celestial grids lie at infinity (rotation-only view) in the equatorial, ecliptic (the
//! equator turned by the obliquity) and galactic frames (FUN_140267b60), so they look the same from
//! anywhere, outside the galaxy too. Their lines (FUN_140315270 primitives 9 and 10): the parallels
//! every 10 deg from -80 to +80 and the meridians pole to pole, 24 (one an hour) for the equatorial
//! grid and 36 (every 10 deg) for the others. Labels: each meridian's at latitudes -60, -30, 0, 30
//! and 60 (`0h`..`23h`, else `0°`..`350°`), and every third meridian's parallels (`+80°`..`-80°`).
//! Each is drawn in the skin's `CelEquatorGridColor` / `CelEclipticGridColor` /
//! `CelGalacticGridColor` with SE's prim_line.glsl lines, its alpha times the squared smooth values
//! of `ShowGrids` (the master switch) and its own `ShowGrid*`.
//!
//! The geographic grid, on a body's ellipsoid in its body frame with the 10-degree grid, is drawn on
//! every planet and moon over 20 pixels across in radius, fading in up to 40 (`(px - 20) / 20`
//! smoothstep), in `GeographicGridColor` times the squared `ShowGridGeographic` (not under the
//! master switch); without depth here, only the side facing the camera is drawn.
//! Label offsets from the grid points are assumed (SE moves them by a field-of-view fraction).

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use se_core::coords::{ecliptic_to_universe_quat, eq_to_universe_quat, galactic_to_universe_quat};

use crate::camera::{FlyCamera, MainCamera};
use crate::overlay::{Occluders, PrimLines, SkyConfig, SkyFade, SkyPath, View, ViewCone, overlay_layer};
use crate::system::SystemBody;

/// SE's grid colours when the skin has none (FUN_14003de90): equatorial, ecliptic, galactic,
/// geographic.
const DEFAULT_COLORS: [[f32; 4]; 4] = [[0.0, 0.6, 0.6, 1.0], [0.0, 0.8, 0.0, 1.0], [0.8, 0.6, 0.0, 1.0], [0.8, 0.8, 0.8, 1.0]];
const COLOR_KEYS: [&str; 4] = ["CelEquatorGridColor", "CelEclipticGridColor", "CelGalacticGridColor", "GeographicGridColor"];

/// Sampling of the lines (SE's primitives use 18 segments per 10 degrees of meridian).
const STEP_DEG: f64 = 1.0;
/// The geographic grid shows from this radius in pixels, at full strength from twice it.
const GEO_MIN_PX: f64 = 20.0;

/// A grid-frame point: x towards longitude 0, y longitude 90 deg, z the north pole.
pub fn sph(lon_deg: f64, lat_deg: f64) -> DVec3 {
    let (l, b) = (lon_deg.to_radians(), lat_deg.to_radians());
    DVec3::new(b.cos() * l.cos(), b.cos() * l.sin(), b.sin())
}

/// A grid's lines in its frame: `meridians` meridians pole to pole, then the parallels every 10 deg
/// from -80 to +80.
pub fn grid_lines(meridians: usize) -> Vec<Vec<DVec3>> {
    let n = (180.0 / STEP_DEG) as usize;
    let mut out: Vec<Vec<DVec3>> = (0..meridians)
        .map(|m| {
            let lon = 360.0 * m as f64 / meridians as f64;
            (0..=n).map(|k| sph(lon, -90.0 + k as f64 * STEP_DEG)).collect()
        })
        .collect();
    for lat in (-80..=80).step_by(10) {
        out.push((0..=2 * n).map(|k| sph(k as f64 * STEP_DEG, lat as f64)).collect());
    }
    out
}

/// A grid's labels (FUN_140266db0): (grid-frame direction, text, true for a meridian's label).
pub fn grid_labels(meridians: usize, hours: bool) -> Vec<(DVec3, String, bool)> {
    let step = 360 / meridians as i32;
    let mut out = Vec::new();
    for lat in (-60..=60).step_by(30) {
        for m in 0..meridians as i32 {
            let text = if hours { format!("{m}h") } else { format!("{}°", m * step) };
            out.push((sph((m * step) as f64, lat as f64), text, true));
        }
    }
    for lat in (-80..=80).rev().step_by(10) {
        for m in (0..meridians as i32).step_by(3) {
            out.push((sph((m * step) as f64, lat as f64), format!("{lat:+}°"), false));
        }
    }
    out
}

/// The celestial grids' frames (grid frame to universe), FUN_140267b60.
fn frames() -> [DQuat; 3] {
    [eq_to_universe_quat(), ecliptic_to_universe_quat(), galactic_to_universe_quat()]
}

/// The grids' lines and labels (built once): the celestial grids' in the universe frame, the
/// 10-degree grid in the grid frame for the bodies.
pub(crate) struct GridGeometry {
    sky: [Vec<SkyPath>; 3],
    labels: [Vec<(DVec3, String, bool)>; 3],
    body: Vec<Vec<DVec3>>,
}

impl Default for GridGeometry {
    fn default() -> Self {
        let [eq, ecl, gal] = frames();
        let sky = |q: DQuat, m: usize| grid_lines(m).into_iter().map(|l| SkyPath::new(l.into_iter().map(|v| q * v).collect())).collect();
        let labels = |q: DQuat, m: usize, hours: bool| grid_labels(m, hours).into_iter().map(|(v, t, mer)| (q * v, t, mer)).collect();
        Self {
            sky: [sky(eq, 24), sky(ecl, 36), sky(gal, 36)],
            labels: [labels(eq, 24, true), labels(ecl, 36, false), labels(gal, 36, false)],
            body: grid_lines(36),
        }
    }
}

pub struct GridsPlugin;

impl Plugin for GridsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(EguiPrimaryContextPass, draw_grids.in_set(crate::skin::UiSet::Draw).before(crate::overlay::draw_orbits));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_grids(
    mut contexts: EguiContexts,
    fade: Res<SkyFade>,
    cfg: Option<Res<SkyConfig>>,
    cam: Res<FlyCamera>,
    cams: Query<&Camera, With<MainCamera>>,
    bodies: Query<&SystemBody>,
    mut geo: Local<Option<GridGeometry>>,
) -> Result {
    let sq = |v: f32| v * v;
    let master = sq(fade.grids);
    let geographic = sq(fade.grid[3]);
    if master <= 0.0 && geographic <= 0.0 {
        return Ok(());
    }
    let Ok(camera) = cams.single() else { return Ok(()) };
    let Some(vp) = camera.logical_viewport_size() else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let skin = crate::skin::text(ctx);
    let cfg = cfg.as_deref().copied().unwrap_or_default();
    let geo = geo.get_or_insert_with(GridGeometry::default);
    let view = View::new(&cam, vp);
    let occ = Occluders::new(bodies.iter(), &cam);
    let mut mesh = PrimLines::new(cfg.lines_width, ctx.pixels_per_point(), vp);
    let mut pts: Vec<(DVec3, f32)> = Vec::new();
    let mut labels: Vec<(egui::Pos2, String, bool, egui::Color32)> = Vec::new();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(vp.x, vp.y));
    let cone = ViewCone::new(&cam, &view);
    for i in 0..3 {
        // FUN_140266a90: alpha = the grid's colour alpha x its switch^2 x ShowGrids^2, Width.z = 1.
        let p3 = sq(fade.grid[i]) * master;
        if p3 <= 0.0 {
            continue;
        }
        let c = skin.rgba(COLOR_KEYS[i], DEFAULT_COLORS[i]);
        let alpha = c[3] * p3;
        for path in &geo.sky[i] {
            for range in path.visible(&cone) {
                pts.clear();
                pts.extend(path.pts[range].iter().map(|&u| {
                    let l = view.local(u);
                    (l, if l.z < 0.0 && occ.hides_dir(u) { 0.0 } else { alpha })
                }));
                mesh.polyline(&view, &pts, [c[0], c[1], c[2]], 1.0);
            }
        }
        let [r, g, b, a] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let color = egui::Color32::from_rgba_unmultiplied(r, g, b, a).gamma_multiply(p3);
        for (u, text, meridian) in &geo.labels[i] {
            if let Some(p) = view.project(view.local(*u)).filter(|p| screen.contains(*p)) {
                if !occ.hides_dir(*u) {
                    labels.push((p, text.clone(), *meridian, color));
                }
            }
        }
    }
    if geographic > 0.0 {
        // SE multiplies all four channels by ShowGridGeographic^2 and fades the line itself by the
        // body's size (Width.z), drawn premultiplied (FUN_1404c7460).
        let c = skin.rgba(COLOR_KEYS[3], DEFAULT_COLORS[3]);
        let k = geographic * geographic;
        let rgb = [c[0] * k, c[1] * k, c[2] * k];
        mesh.premultiplied = true;
        for b in bodies.iter().filter(|b| !b.star && b.radius_m > 0.0 && b.px_radius > GEO_MIN_PX) {
            let size = crate::overlay::smoothstep(0.0, 1.0, ((b.px_radius - GEO_MIN_PX) / GEO_MIN_PX) as f32);
            let centre = b.pos.delta_m(cam.pos);
            for line in &geo.body {
                pts.clear();
                pts.extend(line.iter().map(|&v| {
                    // Grid frame to SE's body frame: +X longitude 0, +Y north, +Z longitude 90 W.
                    let n = DVec3::new(v.x, v.z, -v.y);
                    let p = centre + b.orient * (n * b.shape * b.radius_m);
                    let facing = (b.orient * (n / b.shape)).dot(-p) > 0.0;
                    (view.local(p), if facing { c[3] * k } else { 0.0 })
                }));
                mesh.polyline(&view, &pts, rgb, size);
            }
        }
        mesh.premultiplied = false;
    }
    let painter = ctx.layer_painter(overlay_layer());
    if !mesh.mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh.mesh));
    }
    let font = egui::FontId::proportional(12.0);
    for (p, text, meridian, color) in labels {
        let (anchor, off) = if meridian { (egui::Align2::LEFT_BOTTOM, egui::vec2(3.0, -1.0)) } else { (egui::Align2::LEFT_TOP, egui::vec2(3.0, 1.0)) };
        painter.text(p + off, anchor, text, font.clone(), color);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_line_counts() {
        let eq = grid_lines(24);
        assert_eq!(eq.len(), 24 + 17);
        assert!((eq[0][0] - DVec3::NEG_Z).length() < 1e-12 && (eq[0][180] - DVec3::Z).length() < 1e-12);
        assert_eq!(grid_lines(36).len(), 36 + 17);
        let labels = grid_labels(24, true);
        assert_eq!(labels.len(), 24 * 5 + 8 * 17);
        assert_eq!(labels[1].1, "1h");
        assert_eq!(grid_labels(36, false)[1].1, "10°");
        assert_eq!(labels[24 * 5].1, "+80°");
    }

    #[test]
    fn ground_hides_the_sky_below_the_horizon() {
        use se_core::catalog::ObjectRef;
        use se_core::upos::UPos;
        // Earth's ellipsoid; the camera 300 m up at latitude 40 deg, inside the equatorial sphere.
        let (a, f) = (6_378_137.0, 1.0 / 298.257);
        let earth = SystemBody {
            obj: ObjectRef::Body(0),
            radius_m: a,
            star: false,
            pos: UPos::ZERO,
            orient: DQuat::IDENTITY,
            px_radius: 1e6,
            shape: DVec3::new(1.0, 1.0 - f, 1.0),
        };
        let lat = 40f64.to_radians();
        let n = DVec3::new(lat.cos(), lat.sin(), 0.0);
        let ground = n * earth.shape * a;
        let up = (n / earth.shape).normalize();
        let cam = FlyCamera { pos: UPos::from_meters(ground + up * 300.0), ..Default::default() };
        assert!((ground + up * 300.0).length() < a);
        let occ = Occluders::new(std::iter::once(&earth), &cam);
        let east = DVec3::Z.cross(up).normalize() * -1.0;
        assert!(!occ.hides_dir(up) && !occ.hides_dir(east * 0.99 + up * 0.14));
        assert!(occ.hides_dir(-up) && occ.hides_dir(east * 0.99 - up * 0.14));
    }

    #[test]
    fn sky_paths_cull_what_is_out_of_view() {
        let cam = FlyCamera::default();
        let view = View::new(&cam, Vec2::new(1600.0, 900.0));
        let cone = ViewCone::new(&cam, &view);
        // The equator seen looking down -Z: only the part round -Z is in view.
        let path = SkyPath::new((0..=360).map(|k| sph(k as f64, 0.0)).map(|v| DVec3::new(v.y, v.z, v.x)).collect());
        let vis = path.visible(&cone);
        assert!(!vis.is_empty() && vis.iter().map(|r| r.len()).sum::<usize>() < 200);
        assert!(vis.iter().all(|r| r.clone().all(|i| path.pts[i].z < 0.2)));
    }

    #[test]
    fn grid_frames() {
        let [eq, ecl, gal] = frames();
        // The equatorial grid's pole is the celestial pole (universe +Y), 0h the vernal equinox (+Z).
        assert!((eq * DVec3::Z - DVec3::Y).length() < 1e-12 && (eq * DVec3::X - DVec3::Z).length() < 1e-12);
        // The ecliptic shares the equinox and its pole is the obliquity away from the celestial one.
        assert!((ecl * DVec3::X - DVec3::Z).length() < 1e-12);
        let obl = (ecl * DVec3::Z).angle_between(DVec3::Y).to_degrees();
        assert!((obl - se_core::coords::OBLIQUITY_J2000_DEG).abs() < 1e-9);
        assert!((gal * DVec3::Z - se_core::coords::galactic_north_pole()).length() < 1e-9);
    }
}
