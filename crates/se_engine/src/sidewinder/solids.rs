//! The buildings and trees the flying ship meets (checker 4: it flew through them; the spec's S10, F6). **Ours (F)**: Elite's
//! ships meet its own stations and settlements, not SpaceEngine's towns, so how they collide here is the port's.
//!
//! Where buildings and trees stand is known only to the GPU; the scatter's probe lists them round a capsule, 3-5 frames late
//! (`scatter::probe`, as the sandbox's walker and the hero mod use it). In normal flight near the ground the ship asks along its
//! next 1.5 s; each list is turned into boxes in the body's frame: a building's slices as the hero's solids cut them
//! (`viltrumite::solid`, a turned slice by the box round it in the building's frame) and a tree's trunk by its model's bark
//! (`ScatterLibrary::trunks`), as `sandbox/solids.rs` makes them for the walker. Each tick the ship's box (its model's box) is
//! tested against them; an overlap pushes it out the shortest way, takes away its speed into the wall and, hit hard enough,
//! damages it as the ground does. Not yet: standing on a roof (the touchdown is the ground's).

use bevy::math::DVec3;

use crate::scatter::library::ScatterLibrary;
use crate::scatter::probe::ProbeBuilding;
use crate::viltrumite::solid::{Solid, M_DOME};

/// An oriented box: its middle, its three unit axes and its half extents along them (body frame, m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub c: DVec3,
    pub axes: [DVec3; 3],
    pub half: DVec3,
}

/// The separating axis test: the least push (depth, and the unit normal from `b` towards `a`) that parts two boxes, `None`
/// when they don't overlap.
pub fn hit(a: &Obb, b: &Obb) -> Option<(f64, DVec3)> {
    let d = a.c - b.c;
    if d.length() > a.half.length() + b.half.length() {
        return None;
    }
    let mut best: Option<(f64, DVec3)> = None;
    let mut apart = |axis: DVec3| -> bool {
        let len = axis.length();
        if len < 1e-9 {
            return false;
        }
        let n = axis / len;
        let ra: f64 = (0..3).map(|i| a.half[i] * a.axes[i].dot(n).abs()).sum();
        let rb: f64 = (0..3).map(|i| b.half[i] * b.axes[i].dot(n).abs()).sum();
        let dist = d.dot(n);
        let depth = ra + rb - dist.abs();
        if depth <= 0.0 {
            return true;
        }
        if best.is_none_or(|(bd, _)| depth < bd) {
            best = Some((depth, if dist >= 0.0 { n } else { -n }));
        }
        false
    };
    for i in 0..3 {
        if apart(a.axes[i]) || apart(b.axes[i]) {
            return None;
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            if apart(a.axes[i].cross(b.axes[j])) {
                return None;
            }
        }
    }
    best
}

/// A probe list entry's boxes (body frame): a tree's trunk, or a building's slices.
pub fn boxes_of(b: &ProbeBuilding, lib: &ScatterLibrary) -> Vec<Obb> {
    if !b.foot.is_finite() || !b.up.is_finite() {
        return Vec::new();
    }
    let up = b.up.normalize_or(DVec3::Y);
    if b.kind == 4 {
        // (The walker's trunk: its bark's radius, 70 % of the model's height, at least 2.5 m: `sandbox/solids.rs`.)
        let Some(t) = lib.trunks.get(b.model as usize) else { return Vec::new() };
        let scale = b.size.z as f64;
        if !(scale > 0.05 && scale < 50.0) {
            return Vec::new();
        }
        let radius = (t[0] as f64 * scale).clamp(0.05, 2.5);
        let height = (t[1] as f64 * scale * 0.7).max(2.5);
        let x = up.any_orthonormal_vector();
        return vec![Obb { c: b.foot + up * (0.5 * height), axes: [x, up, x.cross(up)], half: DVec3::new(radius, 0.5 * height, radius) }];
    }
    let Some(s) = Solid::of(b) else { return Vec::new() };
    if s.key.model == M_DOME {
        return Vec::new();
    }
    // The building's own frame: x across its width, y up from its foot, z across its depth (as `sandbox/solids.rs`).
    let za = s.xa.cross(up).normalize_or(up.any_orthonormal_vector());
    let xa = up.cross(za);
    let (w, d, h) = (s.size.x, s.size.y, s.size.z);
    s.slices()
        .iter()
        .filter_map(|sl| {
            let (y0, y1) = (sl.y[0] as f64 * h, sl.y[1] as f64 * h);
            if y1 - y0 < 0.02 || sl.half.x as f64 * w < 0.02 || sl.half.y as f64 * d < 0.02 {
                return None;
            }
            // (A turned slice: u = (x / w) cos + (z / d) sin, v = -(x / w) sin + (z / d) cos; the box round its corners.)
            let (si, c) = (sl.turn as f64).sin_cos();
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let (u, v) = (sl.mid.x as f64 + su * sl.half.x as f64, sl.mid.y as f64 + sv * sl.half.y as f64);
                let (x, z) = (w * (u * c - v * si), d * (u * si + v * c));
                lo = [lo[0].min(x), lo[1].min(z)];
                hi = [hi[0].max(x), hi[1].max(z)];
            }
            let (cx, cz) = (0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1]));
            Some(Obb { c: b.foot + xa * cx + up * (0.5 * (y0 + y1)) + za * cz, axes: [xa, up, za], half: DVec3::new(0.5 * (hi[0] - lo[0]), 0.5 * (y1 - y0), 0.5 * (hi[1] - lo[1])) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(c: DVec3, turn: f64) -> Obb {
        let q = bevy::math::DQuat::from_rotation_y(turn);
        Obb { c, axes: [q * DVec3::X, q * DVec3::Y, q * DVec3::Z], half: DVec3::splat(0.5) }
    }

    /// Two unit cubes 0.8 apart along x overlap by 0.2, pushed apart along x; 1.1 apart they don't; turned 45 degrees about y,
    /// the corner reaches 0.707 out, so 1.1 apart along x they overlap by about 0.107.
    #[test]
    fn boxes_overlap_and_part() {
        let (d, n) = hit(&cube(DVec3::new(0.8, 0.0, 0.0), 0.0), &cube(DVec3::ZERO, 0.0)).unwrap();
        assert!((d - 0.2).abs() < 1e-9 && (n - DVec3::X).length() < 1e-9);
        assert_eq!(hit(&cube(DVec3::new(1.1, 0.0, 0.0), 0.0), &cube(DVec3::ZERO, 0.0)), None);
        let (d, n) = hit(&cube(DVec3::new(-1.1, 0.0, 0.0), std::f64::consts::FRAC_PI_4), &cube(DVec3::ZERO, 0.0)).unwrap();
        assert!((d - (0.5 + 0.5 * 2f64.sqrt() - 1.1)).abs() < 1e-9 && n.x < -0.99, "{d} {n}");
        assert_eq!(hit(&cube(DVec3::new(0.0, 3.0, 0.0), 0.3), &cube(DVec3::ZERO, 0.0)), None);
    }
}
