//! Real air in thruster flight (step 6.3). **Ours (F), not Elite's**: Elite has no drag; the spec (F4) adds it where the port
//! has real air, only in thruster flight (never in supercruise or the glide).
//!
//! Drag is `0.5 x density x speed^2 x area x 1.0` against the motion relative to the air, with the area the hull's own outline
//! seen along the motion: for a closed surface that is half the sum of `|a . d|` over its triangles' area vectors `a` (exact for
//! a convex hull; a concave one counts its hidden folds too, so it comes out somewhat larger). The density comes from the
//! port's `AirModel`. Flight assist fights the drag as it fights gravity, so the top speed in thick air comes out lower by
//! itself. Nothing here is tuned: the drag coefficient is the spec's 1.0.
//!
//! Hot air's heat is left out: the spec asks for it through Elite's star-heat rule fed with the air's heat flux, and that rule's
//! numbers were not found in the exe (step 5.4: its form only), so it would be invented.

use glam::DVec3;

/// The spec's drag coefficient.
pub const DRAG_COEFFICIENT: f64 = 1.0;

/// A triangle's area vector (half the cross product of two edges: its area along its normal).
pub fn area_vector(a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    0.5 * (b - a).cross(c - a)
}

/// The outline's area (m^2) seen along the unit direction `dir`, from a closed surface's area vectors.
pub fn outline_area(areas: &[DVec3], dir: DVec3) -> f64 {
    0.5 * areas.iter().map(|a| a.dot(dir).abs()).sum::<f64>()
}

/// The drag's acceleration (m/s^2) for a velocity relative to the air, the air's density (kg/m^3), the outline's area along the
/// motion (m^2) and the ship's mass (kg).
pub fn drag_acceleration(vel: DVec3, density: f64, area: f64, mass_kg: f64) -> DVec3 {
    if density <= 0.0 || mass_kg <= 0.0 {
        return DVec3::ZERO;
    }
    -0.5 * density * vel.length() * vel * area * DRAG_COEFFICIENT / mass_kg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube's outline: 1 m^2 along an axis, sqrt 2 along a face diagonal, sqrt 3 along the body diagonal; the drag is
    /// against the motion and grows with the square of the speed.
    #[test]
    fn a_cubes_outline_and_the_drag() {
        let v = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
        let c = [v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.), v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)];
        // Two triangles a face, wound outwards.
        let faces = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [0, 4, 7, 3]];
        let mut areas = Vec::new();
        for f in faces {
            areas.push(area_vector(c[f[0]], c[f[1]], c[f[2]]));
            areas.push(area_vector(c[f[0]], c[f[2]], c[f[3]]));
        }
        assert!((outline_area(&areas, DVec3::X) - 1.0).abs() < 1e-12);
        assert!((outline_area(&areas, v(1., 1., 0.).normalize()) - 2f64.sqrt()).abs() < 1e-12);
        assert!((outline_area(&areas, v(1., 1., 1.).normalize()) - 3f64.sqrt()).abs() < 1e-12);
        let a1 = drag_acceleration(v(100., 0., 0.), 1.2, 70.0, 45_000.0);
        let a2 = drag_acceleration(v(200., 0., 0.), 1.2, 70.0, 45_000.0);
        assert!(a1.x < 0.0 && a1.y == 0.0 && (a2.x / a1.x - 4.0).abs() < 1e-12);
        assert!((a1.x + 0.5 * 1.2 * 100.0 * 100.0 * 70.0 / 45_000.0).abs() < 1e-12);
        assert_eq!(drag_acceleration(v(100., 0., 0.), 0.0, 70.0, 45_000.0), DVec3::ZERO);
    }
}
