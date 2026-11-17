//! Orbital cruise: the zone round a planet where supercruise blends to its approach speed (task N3).
//!
//! Rules (grade C, `re/elite/notes/sidewinder_orbital_cruise.md`): the ship's `ShipPlanetProximityComponent` (constructor
//! `0x142C604A0`; its interface, vtable `0x1453183E0`, is A50's and A60's object "S") keeps, for every body of kind 2 a flag
//! test lets in (`0x142CA3520`), five radii squared made from the body's radius R and its terrain's top h by a record of the
//! system component of type `0x145F05C68` (constructor `0x143CDAB20`; its getters `0x143CF5B90`, `0x143CEF190`,
//! `0x143CF07B0..D0`, `0x143CF0290`), called Z here:
//! - the outer radius `outer = R Z4C + Z50` (the approach zone),
//! - the inner radius `inner = Z58 + (R + max(h, Z5C)) Z54` (orbital cruise; re-made every frame, `0x142C9A3D0`),
//! - the reach `Z60 outer` and `R Z64`: a body counts only inside both.
//!
//! Every frame (`0x142C88F10`) the body with the largest share `k = clamp(1 - (d - R) / (Z60 outer - R), 0, 1)` (d the ship's
//! distance from its centre) inside its reach becomes the target, and the mode is set by it: 2 (orbital cruise) inside the
//! inner radius, else 1 (approach) inside the outer one; a later body that wins on k but lies in neither keeps the mode the
//! earlier one set (the code's order: the mode starts at 0 each frame). w (`0x142C99400`, S's virtual +0x38) is 0 in mode 0;
//! otherwise `1 - clamp((d^2 - r^2) / (outer^2 - r^2), 0, 1)` with `r = inner + 3,000 m`: a blend in the squared distance,
//! 0 at the outer radius, 1 from 3 km above the inner one. Entering mode 2 tells the HUD's interface (`0x142C88F10`'s tail);
//! supercruise reads w for its cap and zero-throttle speed (`supercruise::cruise_cap`, `min_speed`) and for the full-throttle
//! brake (`supercruise::ORBITAL_BRAKE_W`), and S's mode for the safe stop (`supercruise::safe_stop`) and the glide's entry
//! (A60: mode 2 only).
//!
//! Numbers: Z's seven floats are NOT among the records read (the system component's template sits in a location prototype
//! not exported); the 3,000 m is the exe's (`0x1452AB378`). Until Z is read the game keeps `supercruise`'s stand-in floor.

/// The margin over the inner radius where w reaches 1 (m; a double at `0x1452AB378`, C).
pub const BLEND_MARGIN_M: f64 = 3000.0;

/// The system record's zone numbers (Z +0x4C..+0x64, in the order the getters read them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    /// Z4C, Z50: the outer radius's factor on R and its addition (m).
    pub outer_k: f64,
    pub outer_add: f64,
    /// Z54, Z58, Z5C: the inner radius's factor, addition (m) and the least terrain top it takes (m).
    pub inner_k: f64,
    pub inner_add: f64,
    pub inner_top_floor: f64,
    /// Z60: the reach's share of the outer radius; Z64: the reach's factor on R.
    pub reach_share: f64,
    pub reach_k: f64,
}

/// S's mode (virtual +0x28).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// No planet near: w 0; the safe stop does nothing.
    #[default]
    Free,
    /// Inside a planet's outer radius: w blends; the cap's heading factor applies.
    Approach,
    /// Inside the inner radius: orbital cruise; the safe stop may glide.
    Orbital,
}

/// A body of kind 2 as the zone sees it: its radius and its terrain's top over it (m), the ship's distance from its centre (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub radius: f64,
    pub top: f64,
    pub distance: f64,
}

impl Zone {
    pub fn outer(&self, radius: f64) -> f64 {
        radius * self.outer_k + self.outer_add
    }

    pub fn inner(&self, radius: f64, top: f64) -> f64 {
        self.inner_add + (radius + top.max(self.inner_top_floor)) * self.inner_k
    }

    /// The frame's target (an index into `bodies`) and S's mode (`0x142C88F10`).
    pub fn assess(&self, bodies: &[Body]) -> (Option<usize>, Mode) {
        let (mut target, mut mode, mut best) = (None, Mode::Free, 0.0);
        for (i, b) in bodies.iter().enumerate() {
            let d2 = b.distance * b.distance;
            let outer = self.outer(b.radius);
            let near = self.reach_share * outer;
            if d2 >= (b.radius * self.reach_k).powi(2) || d2 >= near * near {
                continue;
            }
            let k = (1.0 - (b.distance - b.radius) / (near - b.radius)).clamp(0.0, 1.0);
            if k > best {
                if d2 < self.inner(b.radius, b.top).powi(2) {
                    mode = Mode::Orbital;
                } else if d2 < outer * outer {
                    mode = Mode::Approach;
                }
                target = Some(i);
                best = k;
            }
        }
        (target, mode)
    }

    /// Orbital cruise's blend w for the target body in `mode` (`0x142C99400`). Not built: the exe's w is also 0 unless the
    /// handle at S+0x98 is set (S+0xA0 not null) and that object's virtual +0x18 answers true; the object is not
    /// identified, so here it is taken as always true.
    pub fn blend(&self, body: &Body, mode: Mode) -> f64 {
        if mode == Mode::Free {
            return 0.0;
        }
        let r2 = (self.inner(body.radius, body.top) + BLEND_MARGIN_M).powi(2);
        let o2 = self.outer(body.radius).powi(2);
        let d2 = body.distance * body.distance;
        let x = if r2 > d2 { r2 } else { o2.min(d2) };
        1.0 - (x - r2) / (o2 - r2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test numbers only (Z is not read): an outer radius of 2 R, an inner one of R + 20 km, the reach 1.5 x the outer one.
    const Z: Zone = Zone { outer_k: 2.0, outer_add: 0.0, inner_k: 1.0, inner_add: 20_000.0, inner_top_floor: 0.0, reach_share: 1.5, reach_k: 10.0 };
    const R: f64 = 1.0e6;

    fn at(d: f64) -> Body {
        Body { radius: R, top: 0.0, distance: d }
    }

    /// The modes by distance: free beyond the reach and outside the outer radius, approach inside it, orbital inside the inner.
    #[test]
    fn the_modes_by_distance() {
        assert_eq!(Z.assess(&[at(3.5 * R)]), (None, Mode::Free));
        assert_eq!(Z.assess(&[at(2.5 * R)]), (Some(0), Mode::Free));
        assert_eq!(Z.assess(&[at(1.5 * R)]), (Some(0), Mode::Approach));
        assert_eq!(Z.assess(&[at(R + 10_000.0)]), (Some(0), Mode::Orbital));
        // The terrain's top raises the inner radius, never under Z5C.
        let high = Body { top: 50_000.0, ..at(R + 60_000.0) };
        assert_eq!(Z.assess(&[high]).1, Mode::Orbital);
    }

    /// w: 0 at the outer radius and beyond, 1 from 3 km over the inner radius down, linear in the squared distance between.
    #[test]
    fn the_blend() {
        assert_eq!(Z.blend(&at(1.5 * R), Mode::Free), 0.0);
        assert_eq!(Z.blend(&at(2.0 * R), Mode::Approach), 0.0);
        assert_eq!(Z.blend(&at(2.5 * R), Mode::Approach), 0.0);
        let r = R + 20_000.0 + BLEND_MARGIN_M;
        assert_eq!(Z.blend(&at(r), Mode::Orbital), 1.0);
        assert_eq!(Z.blend(&at(R + 5000.0), Mode::Orbital), 1.0);
        let mid = ((r * r + 4.0 * R * R) / 2.0).sqrt();
        assert!((Z.blend(&at(mid), Mode::Approach) - 0.5).abs() < 1e-12);
    }

    /// The body with the larger share wins the target and sets the mode by its own radii.
    #[test]
    fn the_nearest_share_wins() {
        let far = at(1.8 * R);
        let close = at(1.2 * R);
        assert_eq!(Z.assess(&[far, close]), (Some(1), Mode::Approach));
        let small = Body { radius: 1000.0, top: 0.0, distance: 1100.0 };
        let wide = Zone { reach_k: 1.0e4, ..Z };
        assert_eq!(wide.assess(&[close, small]), (Some(1), Mode::Orbital));
    }
}
