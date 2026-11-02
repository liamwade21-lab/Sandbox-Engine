//! Positions and orientations of catalog objects at a given time.
//!
//! A body's position is its parent's position plus its orbit, rotated from the
//! orbit's reference plane into the universe frame. System roots (stars and
//! barycenters with RA/Dec/Dist) are fixed in the universe frame.

use glam::{DMat3, DQuat, DVec3};

use crate::catalog::{Catalog, ObjectRef};
use crate::coords::{ecliptic_to_universe, ecliptic_to_universe_quat};
use crate::orbit::{KeplerOrbit, RefPlane};
use crate::rotation::{RotationLock, RotationModel};
use crate::upos::UPos;

pub struct Ephemeris<'a> {
    pub catalog: &'a Catalog,
}

const MAX_DEPTH: usize = 16;

/// Days between the two positions a tidally locked body's pole is taken from.
const TIDAL_STEP: f64 = 1.0e-3;

impl<'a> Ephemeris<'a> {
    pub fn new(catalog: &'a Catalog) -> Self {
        Self { catalog }
    }

    pub fn position(&self, r: ObjectRef, jd: f64) -> UPos {
        self.position_depth(r, jd, 0)
    }

    /// Barycentric position from the JPL ephemeris for `*-DE` analytic models.
    fn de_position(&self, orbit: Option<&KeplerOrbit>, jd: f64) -> Option<UPos> {
        let de = self.catalog.de.as_ref()?;
        let model = orbit?.analytic_model.as_deref()?;
        let km = de.position_for_model(model, jd)?;
        Some(UPos::from_meters(crate::coords::eq_to_universe(km * 1000.0)))
    }

    /// Parent-relative offset (universe-frame meters) from SpaceEngine's analytic
    /// moon theories (`GUST86`, `L1`, `TASS17`, ...) for `AnalyticModel "Moon-Theory"`
    /// orbits, when the theory tables were loaded from the install's executable.
    fn moon_theory_offset(&self, orbit: &KeplerOrbit, jd: f64) -> Option<DVec3> {
        let th = self.catalog.moon_theories.as_ref()?;
        th.offset_m(orbit.analytic_model.as_deref()?, jd)
    }

    fn position_depth(&self, r: ObjectRef, jd: f64, depth: usize) -> UPos {
        if let Some(p) = self.de_position(self.orbit_of(r), jd) {
            return p;
        }
        match self.parent_of(r) {
            Some(pr) if depth < MAX_DEPTH => {
                let base = self.position_depth(pr, jd, depth + 1);
                self.position_from_parent(r, pr, base, jd, self.fixed_pos_of(r))
            }
            _ => self.root_position(r),
        }
    }

    fn orbit_of(&self, r: ObjectRef) -> Option<&KeplerOrbit> {
        match r {
            ObjectRef::Star(i) => self.catalog.stars[i as usize].orbit.as_ref(),
            ObjectRef::Body(i) => self.catalog.bodies[i as usize].orbit.as_ref(),
            _ => None,
        }
    }

    fn fixed_pos_of(&self, r: ObjectRef) -> Option<[f64; 3]> {
        match r {
            ObjectRef::Body(i) => self.catalog.bodies[i as usize].fixed_pos_polar(),
            _ => None,
        }
    }

    /// The object whose position `r`'s hangs from (outside the JPL ephemeris), found by name;
    /// none for system roots, objects whose parent is missing and stars fixed in space.
    fn parent_of(&self, r: ObjectRef) -> Option<ObjectRef> {
        let c = self.catalog;
        let name = match r {
            ObjectRef::Star(i) => {
                let s = &c.stars[i as usize];
                match (&s.parent, &s.orbit) {
                    (Some(p), Some(_)) => p,
                    (Some(p), None) if !s.has_position => p,
                    _ => return None,
                }
            }
            ObjectRef::Body(i) => &c.bodies[i as usize].parent,
            _ => return None,
        };
        c.find(name).filter(|pr| *pr != r)
    }

    /// Position of an object without a parent (or past the depth cap).
    fn root_position(&self, r: ObjectRef) -> UPos {
        let c = self.catalog;
        match r {
            ObjectRef::Star(i) => UPos::from_parsecs(c.stars[i as usize].pos_pc),
            ObjectRef::Body(_) => UPos::ZERO,
            ObjectRef::Galaxy(i) => UPos::from_parsecs(c.galaxies[i as usize].pos_pc),
            ObjectRef::Cluster(i) => UPos::from_parsecs(c.clusters[i as usize].pos_pc),
            ObjectRef::Nebula(i) => UPos::from_parsecs(c.nebulae[i as usize].pos_pc),
        }
    }

    /// Position of `r` given its parent `pr` at `base`; `fixed` is `r`'s `FixedPosPolar`.
    fn position_from_parent(&self, r: ObjectRef, pr: ObjectRef, base: UPos, jd: f64, fixed: Option<[f64; 3]>) -> UPos {
        match r {
            ObjectRef::Star(i) => match &self.catalog.stars[i as usize].orbit {
                Some(o) => base.offset_m(self.orbit_offset(o, pr, base, jd)),
                None => base,
            },
            ObjectRef::Body(i) => {
                if let Some([lon, lat, dist_km]) = fixed {
                    let dir = DVec3::from_array(crate::se_script::polar_dir(lon, lat));
                    return base.offset_m(self.orientation(pr, jd) * dir * (dist_km * 1e3));
                }
                match &self.catalog.bodies[i as usize].orbit {
                    Some(o) => match self.moon_theory_offset(o, jd) {
                        Some(off) => base.offset_m(off),
                        None => base.offset_m(self.orbit_offset(o, pr, base, jd)),
                    },
                    None => base,
                }
            }
            _ => base,
        }
    }

    /// Orbit position relative to the parent, in universe-frame meters.
    pub fn orbit_offset(&self, o: &KeplerOrbit, parent: ObjectRef, parent_pos: UPos, jd: f64) -> DVec3 {
        if o.semi_major_axis <= 0.0 {
            return DVec3::ZERO;
        }
        self.reference_rotation(o.ref_plane, parent, parent_pos, jd) * o.position_at(jd)
    }

    /// Rotation from an orbit reference frame (plane normal = +Z) to the universe.
    pub fn reference_rotation(&self, plane: RefPlane, parent: ObjectRef, parent_pos: UPos, jd: f64) -> DQuat {
        match plane {
            RefPlane::Ecliptic => ecliptic_to_universe_quat(),
            RefPlane::Equator => crate::coords::eq_to_universe_quat(),
            RefPlane::ParentEquator => {
                let pole = self.orientation(parent, jd) * DVec3::Y;
                frame_from_normal(pole, ecliptic_to_universe(DVec3::Z))
            }
            RefPlane::Extrinsic => {
                // Plane of the sky as seen from the Sun; normal points away from us.
                let los = parent_pos.to_meters().normalize_or(DVec3::Z);
                frame_from_normal(los, DVec3::Y)
            }
        }
    }

    /// Body orientation (body frame -> universe) at `jd`.
    pub fn orientation(&self, r: ObjectRef, jd: f64) -> DQuat {
        match r {
            ObjectRef::Body(i) => {
                let b = &self.catalog.bodies[i as usize];
                let parent = self.catalog.find(&b.parent).filter(|p| *p != r);
                self.body_orientation(r, parent, b.fixed_pos_polar(), jd, None)
            }
            ObjectRef::Star(i) => self.catalog.stars[i as usize].rotation.orientation(jd),
            ObjectRef::Galaxy(i) => self.catalog.galaxies[i as usize].orientation,
            ObjectRef::Nebula(i) => self.catalog.nebulae[i as usize].orientation,
            ObjectRef::Cluster(_) => DQuat::IDENTITY,
        }
    }

    /// Orientation of body `r` with parent `parent` and `FixedPosPolar` `fixed`; `rel(t)` is `r`'s
    /// position relative to its parent at `t`, when the caller has a faster way to it.
    fn body_orientation(&self, r: ObjectRef, parent: Option<ObjectRef>, fixed: Option<[f64; 3]>, jd: f64, rel: Option<&dyn Fn(f64) -> DVec3>) -> DQuat {
        let ObjectRef::Body(i) = r else {
            return self.orientation(r, jd);
        };
        let b = &self.catalog.bodies[i as usize];
        // SE's IgnoreMoonsRotationIAU (main-def.cfg, true): a moon's IAU model gives way to
        // tidal locking.
        if let (Some([lon, lat, _]), Some(p), RotationLock::Tidal) = (fixed, parent, b.lock) {
            return fixed_frame(self.orientation(p, jd), lon, lat, &b.block);
        }
        let moon_iau = matches!(b.rotation, RotationModel::Iau(_)) && matches!(parent, Some(ObjectRef::Body(_)));
        let locked = match b.lock {
            RotationLock::MoonDe => self.moon_de(jd).or_else(|| parent.and_then(|p| self.tidal_lock(r, p, jd, rel))),
            RotationLock::Tidal => parent.and_then(|p| self.tidal_lock(r, p, jd, rel)),
            RotationLock::None if moon_iau => parent.and_then(|p| self.tidal_lock(r, p, jd, rel)),
            RotationLock::None => None,
        };
        locked.unwrap_or_else(|| b.rotation.orientation(jd))
    }

    /// SE's `Moon-DE` rotation: the Moon's librations from the JPL ephemeris (3-1-3 Euler angles of
    /// its principal axes against the ICRF), within the ephemeris's span.
    fn moon_de(&self, jd: f64) -> Option<DQuat> {
        let e = self.catalog.de.as_ref()?.lunar_euler(jd)?;
        Some(crate::rotation::from_icrf_euler(e.x, e.y, e.z))
    }

    /// A tidally locked body: longitude 0 (+X) faces the parent and the pole (+Y) lies along the
    /// orbit's normal (its angular momentum, so retrograde moons spin retrograde).
    fn tidal_lock(&self, r: ObjectRef, parent: ObjectRef, jd: f64, rel: Option<&dyn Fn(f64) -> DVec3>) -> Option<DQuat> {
        let rel = |t: f64| match rel {
            Some(f) => f(t),
            None => self.position(r, t).delta_m(self.position(parent, t)),
        };
        let (a, b) = (rel(jd), rel(jd + TIDAL_STEP));
        let pole = a.cross(b - a).normalize_or_zero();
        let to_parent = -a - pole * (-a).dot(pole);
        if pole == DVec3::ZERO || to_parent.length_squared() <= 0.0 {
            return None;
        }
        let x = to_parent.normalize();
        Some(DQuat::from_mat3(&DMat3::from_cols(x, pole, x.cross(pole))).normalize())
    }

    pub fn rotation_model(&self, r: ObjectRef) -> Option<&RotationModel> {
        match r {
            ObjectRef::Body(i) => Some(&self.catalog.bodies[i as usize].rotation),
            ObjectRef::Star(i) => Some(&self.catalog.stars[i as usize].rotation),
            _ => None,
        }
    }

    /// Physical radius in meters (0 for points like barycenters).
    pub fn radius_m(&self, r: ObjectRef) -> f64 {
        let c = self.catalog;
        match r {
            ObjectRef::Body(i) => c.bodies[i as usize].radius_km * 1e3,
            ObjectRef::Star(i) => {
                let s = &c.stars[i as usize];
                if s.barycenter { 0.0 } else { s.radius_km * 1e3 }
            }
            ObjectRef::Galaxy(i) => c.galaxies[i as usize].radius_pc * crate::units::METERS_PER_PC,
            ObjectRef::Cluster(i) => c.clusters[i as usize].radius_pc * crate::units::METERS_PER_PC,
            ObjectRef::Nebula(i) => c.nebulae[i as usize].radius_pc * crate::units::METERS_PER_PC,
        }
    }

    /// Sample an orbit path (universe-frame offsets from the parent).
    pub fn orbit_path(&self, r: ObjectRef, jd: f64, segments: usize) -> Option<(ObjectRef, Vec<DVec3>)> {
        let c = self.catalog;
        let (orbit, parent) = match r {
            ObjectRef::Body(i) => (c.bodies[i as usize].orbit.as_ref()?, c.find(&c.bodies[i as usize].parent)?),
            ObjectRef::Star(i) => {
                let s = &c.stars[i as usize];
                (s.orbit.as_ref()?, c.find(s.parent.as_deref()?)?)
            }
            _ => return None,
        };
        let parent_pos = self.position(parent, jd);
        let rot = self.reference_rotation(orbit.ref_plane, parent, parent_pos, jd);
        let pts = orbit.sample_path(segments).into_iter().map(|p| rot * p).collect::<Vec<_>>();
        (!pts.is_empty()).then_some((parent, pts))
    }
}

/// Positions and orientations of many objects at one time, bit for bit those of
/// [`Ephemeris::position`] and [`Ephemeris::orientation`], but with each parent solved once
/// ([`BatchPlan::parent_positions`], parents first) instead of once per child. It holds parents
/// found by name, so build it again whenever the catalog changes.
#[derive(Default)]
pub struct BatchPlan {
    /// The objects' ancestors, each after its parent.
    parents: Vec<PlanNode>,
    /// Per body index, and for other objects, the plan of an object given to [`BatchPlan::new`].
    bodies: Vec<Option<PlanNode>>,
    others: std::collections::HashMap<ObjectRef, PlanNode>,
}

/// A [`BatchPlan`]'s ancestor positions at one time.
#[derive(Default)]
pub struct PlanFrame {
    jd: f64,
    now: Vec<UPos>,
    /// At `jd + TIDAL_STEP`.
    later: Vec<UPos>,
}

#[derive(Clone, Copy)]
struct PlanNode {
    obj: ObjectRef,
    /// The parent and its slot in `parents`.
    parent: Option<(ObjectRef, u32)>,
    fixed: Option<[f64; 3]>,
    /// The object's own slot in `parents`, when it is one.
    own: Option<u32>,
}

impl BatchPlan {
    /// A plan for `objs`. An object whose parent chain doesn't reach a root within the depth cap
    /// (a cyclic catalog) is left out and solved the slow way, since its capped position isn't its
    /// parent's plus an offset.
    pub fn new(catalog: &Catalog, objs: impl IntoIterator<Item = ObjectRef>) -> Self {
        let eph = Ephemeris::new(catalog);
        let mut plan = Self { bodies: vec![None; catalog.bodies.len()], ..Default::default() };
        let mut slot = std::collections::HashMap::<ObjectRef, u32>::new();
        let mut chain = Vec::new();
        for r in objs {
            // r, its parent, ... up to a root; at most MAX_DEPTH steps, like position_depth.
            chain.clear();
            chain.push(r);
            let reaches_root = loop {
                match eph.parent_of(*chain.last().unwrap()) {
                    None => break true,
                    Some(_) if chain.len() > MAX_DEPTH => break false,
                    Some(p) => chain.push(p),
                }
            };
            if !reaches_root {
                continue;
            }
            let node = |k: usize, slot: &std::collections::HashMap<ObjectRef, u32>| PlanNode {
                obj: chain[k],
                parent: chain.get(k + 1).map(|&p| (p, slot[&p])),
                fixed: eph.fixed_pos_of(chain[k]),
                own: slot.get(&chain[k]).copied(),
            };
            for k in (1..chain.len()).rev() {
                if !slot.contains_key(&chain[k]) {
                    let n = node(k, &slot);
                    slot.insert(n.obj, plan.parents.len() as u32);
                    plan.parents.push(n);
                }
            }
            let n = node(0, &slot);
            match r {
                ObjectRef::Body(i) => plan.bodies[i as usize] = Some(n),
                _ => {
                    plan.others.insert(r, n);
                }
            }
        }
        // An object listed after a child of it became an ancestor later: point it at its slot.
        for n in plan.bodies.iter_mut().flatten().chain(plan.others.values_mut()) {
            n.own = slot.get(&n.obj).copied();
        }
        plan
    }

    /// The ancestors' positions at `jd` (and a moment later, for tidal locks), for [`BatchPlan::solve`].
    pub fn solve_parents(&self, eph: &Ephemeris, jd: f64, out: &mut PlanFrame) {
        out.jd = jd;
        for (t, pos) in [(jd, &mut out.now), (jd + TIDAL_STEP, &mut out.later)] {
            pos.clear();
            for n in &self.parents {
                let p = Self::node_position(eph, n, t, pos);
                pos.push(p);
            }
        }
    }

    /// `r`'s position and orientation at the time of `frame`, this plan's `solve_parents`.
    pub fn solve(&self, eph: &Ephemeris, r: ObjectRef, frame: &PlanFrame) -> (UPos, DQuat) {
        let jd = frame.jd;
        let n = match r {
            ObjectRef::Body(i) => self.bodies.get(i as usize).and_then(|n| n.as_ref()),
            _ => self.others.get(&r),
        };
        let Some(n) = n else {
            return (eph.position(r, jd), eph.orientation(r, jd));
        };
        let at = |parents: &[UPos], t: f64| match n.own {
            Some(s) => parents[s as usize],
            None => Self::node_position(eph, n, t, parents),
        };
        let pos = at(&frame.now, jd);
        let orient = match (r, n.parent) {
            (ObjectRef::Body(_), Some((p, s))) => {
                let rel = |t: f64| {
                    if t == jd {
                        pos.delta_m(frame.now[s as usize])
                    } else {
                        debug_assert_eq!(t, jd + TIDAL_STEP);
                        at(&frame.later, t).delta_m(frame.later[s as usize])
                    }
                };
                eph.body_orientation(r, Some(p), n.fixed, jd, Some(&rel))
            }
            (ObjectRef::Body(_), None) => eph.body_orientation(r, None, n.fixed, jd, None),
            _ => eph.orientation(r, jd),
        };
        (pos, orient)
    }

    /// `position_depth(n.obj, jd, 0)`, with the parent's taken from `parents`: the same value,
    /// as the parent's own chain ends before the depth cap.
    fn node_position(eph: &Ephemeris, n: &PlanNode, jd: f64, parents: &[UPos]) -> UPos {
        if let Some(p) = eph.de_position(eph.orbit_of(n.obj), jd) {
            return p;
        }
        match n.parent {
            Some((pr, s)) => eph.position_from_parent(n.obj, pr, parents[s as usize], jd, n.fixed),
            None => eph.root_position(n.obj),
        }
    }
}

/// Orthonormal frame with +Z = `normal` and +X perpendicular to `hint`.
fn frame_from_normal(normal: DVec3, hint: DVec3) -> DQuat {
    let z = normal.normalize_or(DVec3::Z);
    let mut x = hint.cross(z);
    if x.length_squared() < 1e-12 {
        x = DVec3::X.cross(z);
    }
    let x = x.normalize();
    let y = z.cross(x);
    DQuat::from_mat3(&DMat3::from_cols(x, y, z))
}

/// The frame of a `TidalLocked` body at `FixedPosPolar` (lon, lat) on a parent turned by `parent`:
/// SE derives its up vector (+Y) from the position ("up vector automatically derived from FixedPos
/// coordinates"); +Z points south along the parent's surface, and the catalog's `RotationOffset`
/// (azimuth, degrees), `Obliquity` and `EqAscendNode` (tilt adjustments, degrees) turn it. The order
/// and signs of those three are assumed (the tilts are a few degrees at most).
fn fixed_frame(parent: DQuat, lon: f64, lat: f64, b: &crate::sc::Block) -> DQuat {
    let up = parent * DVec3::from_array(crate::se_script::polar_dir(lon, lat));
    let pole = parent * DVec3::Y;
    let north = (pole - up * pole.dot(up)).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
    let east = north.cross(up);
    let local = DQuat::from_mat3(&DMat3::from_cols(east, up, -north));
    let deg = |k: &str| b.f64(k).unwrap_or(0.0).to_radians();
    (local
        * DQuat::from_rotation_y(-deg("RotationOffset"))
        * DQuat::from_rotation_x(deg("Obliquity"))
        * DQuat::from_rotation_z(deg("EqAscendNode")))
    .normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::METERS_PER_AU;

    #[test]
    fn fixed_position_turns_with_its_parent_and_stands_upright() {
        let mut c = Catalog::new();
        c.load_sc(
            "StarBarycenter \"Sys\" { RA 1 Dec 2 Dist 10 }\n\
             Star \"Sys A\" { ParentBody \"Sys\" Class \"G2V\" MassSol 1 }\n\
             Planet \"P\" { ParentBody \"Sys A\" Radius 1700 Mass 0.01 RotationPeriod 10\n Orbit { SemiMajorAxis 1 Eccentricity 0 } }\n\
             Spacecraft \"L\" { ParentBody \"P\" TidalLocked true RotationOffset 90 FixedPosPolar (23.47 0.67 1695.5) }\n",
            "t.sc",
        );
        let eph = Ephemeris::new(&c);
        let (p, l) = (c.find("P").unwrap(), c.find("L").unwrap());
        for jd in [2_451_545.0, 2_451_545.2] {
            let rel = eph.position(l, jd).delta_m(eph.position(p, jd));
            assert!((rel.length() - 1_695_500.0).abs() < 1.0, "{}", rel.length());
            // On the parent's surface at its (lon, lat), whatever the parent's rotation.
            let local = eph.orientation(p, jd).inverse() * rel.normalize();
            let want = DVec3::from_array(crate::se_script::polar_dir(23.47, 0.67));
            assert!(local.dot(want) > 0.999_999, "{local} {want}");
            // Up (+Y) is the local vertical; +Z points south, turned 90 degrees by RotationOffset.
            let o = eph.orientation(l, jd);
            assert!((o * DVec3::Y).dot(rel.normalize()) > 0.999_999);
            let pole = eph.orientation(p, jd) * DVec3::Y;
            assert!((o * DVec3::Z).dot(pole).abs() < 1e-6, "turned off south: {}", (o * DVec3::Z).dot(pole));
        }
    }

    #[test]
    fn tidally_locked_moon_faces_its_planet() {
        let mut c = Catalog::new();
        c.load_sc(
            "StarBarycenter \"Sys\" { RA 1 Dec 2 Dist 10 }\n\
             Star \"Sys A\" { ParentBody \"Sys\" Class \"G2V\" MassSol 1 }\n\
             Planet \"P\" { ParentBody \"Sys A\" Radius 6000 Mass 1\n Orbit { SemiMajorAxis 1 Eccentricity 0 } }\n\
             Moon \"M\" { ParentBody \"P\" Radius 1000 TidalLocked true\n Orbit { SemiMajorAxis 0.002 Period 0.05 Eccentricity 0 } }\n\
             Moon \"N\" { ParentBody \"P\" Radius 1000 RotationModel \"IAU\" { PoleRA 10 PoleDec 20 RotationRate 100 }\n Orbit { SemiMajorAxis 0.003 Period 0.08 Eccentricity 0 } }\n",
            "t.sc",
        );
        let eph = Ephemeris::new(&c);
        let (p, m, n) = (c.find("P").unwrap(), c.find("M").unwrap(), c.find("N").unwrap());
        for jd in [2_451_545.0, 2_451_550.3, 2_451_601.7] {
            for moon in [m, n] {
                let to_parent = eph.position(p, jd).delta_m(eph.position(moon, jd)).normalize();
                let lon0 = eph.orientation(moon, jd) * DVec3::X;
                assert!(lon0.dot(to_parent) > 0.9999, "{moon:?} at {jd}: {} lock {:?} lon0 {lon0} to {to_parent}", lon0.dot(to_parent), c.bodies[1].lock);
            }
        }
    }

    /// `BatchPlan::solve` against `position` and `orientation`, bit for bit, for every object in
    /// `objs` (in that order and reversed, so children come both before and after their parents).
    fn assert_batch_matches(c: &Catalog, objs: &[ObjectRef], jds: &[f64]) {
        let eph = Ephemeris::new(c);
        for order in [objs.to_vec(), objs.iter().rev().copied().collect()] {
            let plan = BatchPlan::new(c, order.iter().copied());
            let mut frame = PlanFrame::default();
            for &jd in jds {
                plan.solve_parents(&eph, jd, &mut frame);
                for &r in &order {
                    let (p, q) = plan.solve(&eph, r, &frame);
                    assert_eq!(p, eph.position(r, jd), "position of {r:?} at {jd}");
                    let want = eph.orientation(r, jd);
                    assert_eq!(q.to_array().map(f64::to_bits), want.to_array().map(f64::to_bits), "orientation of {r:?} at {jd}");
                }
            }
        }
    }

    #[test]
    fn batch_plan_matches_position_and_orientation() {
        let mut c = Catalog::new();
        let mut sc = String::from(
            "StarBarycenter \"Sys\" { RA 1 Dec 2 Dist 10 }\n\
             Star \"Sys A\" { ParentBody \"Sys\" Class \"G2V\" MassSol 1\n Orbit { SemiMajorAxis 0.1 Period 30 Eccentricity 0.2 } }\n\
             Star \"Sys B\" { ParentBody \"Sys\" Class \"K2V\" MassSol 0.5\n Orbit { SemiMajorAxis 0.2 Period 30 Eccentricity 0.2 ArgOfPericenter 180 } }\n\
             Planet \"P\" { ParentBody \"Sys A\" Radius 6000 Mass 1 RotationPeriod 20 Obliquity 23\n Orbit { SemiMajorAxis 1 Eccentricity 0.1 Inclination 3 } }\n\
             Moon \"M\" { ParentBody \"P\" Radius 1000 TidalLocked true\n Orbit { RefPlane \"Equator\" SemiMajorAxis 0.002 Period 0.05 Eccentricity 0.05 Inclination 5 } }\n\
             Moon \"N\" { ParentBody \"P\" Radius 1000 RotationModel \"IAU\" { PoleRA 10 PoleDec 20 RotationRate 100 }\n Orbit { SemiMajorAxis 0.003 Period 0.08 Eccentricity 0 } }\n\
             Moon \"MM\" { ParentBody \"M\" Radius 10 TidalLocked true\n Orbit { SemiMajorAxis 0.0001 Period 0.01 Eccentricity 0 } }\n\
             Spacecraft \"L\" { ParentBody \"P\" TidalLocked true RotationOffset 90 FixedPosPolar (23.47 0.67 6100) }\n\
             Spacecraft \"F\" { ParentBody \"M\" FixedPosPolar (-40 10 1200) }\n\
             Spacecraft \"FL\" { ParentBody \"F\" TidalLocked true\n Orbit { SemiMajorAxis 0.00001 Period 0.001 } }\n\
             Planet \"E\" { ParentBody \"Sys B\" Radius 600 Mass 0.1\n Orbit { RefPlane \"Extrinsic\" SemiMajorAxis 2 Eccentricity 0.3 } }\n\
             Moon \"Orphan\" { ParentBody \"Unloaded\" Radius 100 TidalLocked true\n Orbit { SemiMajorAxis 0.01 } }\n\
             Moon \"Selfish\" { ParentBody \"Selfish\" Radius 100\n Orbit { SemiMajorAxis 0.01 } }\n\
             Moon \"CycA\" { ParentBody \"CycB\" Radius 100 TidalLocked true\n Orbit { RefPlane \"Ecliptic\" SemiMajorAxis 0.01 Period 1 } }\n\
             Moon \"CycB\" { ParentBody \"CycA\" Radius 100\n Orbit { RefPlane \"Ecliptic\" SemiMajorAxis 0.02 Period 2 } }\n",
        );
        // A chain of moons longer than the depth cap: the deep ones are clamped, the planner must
        // leave them to the slow path. (Ecliptic orbits and few locks: a locked parent's equator
        // makes the old path exponential in the depth, and overflows the stack in a cycle.)
        for k in 0..22 {
            let parent = if k == 0 { "P".to_string() } else { format!("D{}", k - 1) };
            let lock = [3, 12, 15, 16, 21].contains(&k);
            sc += &format!("Moon \"D{k}\" {{ ParentBody \"{parent}\" Radius 1 TidalLocked {lock}\n Orbit {{ RefPlane \"Ecliptic\" SemiMajorAxis 0.001 Period {} }} }}\n", 0.1 + k as f64 * 0.01);
        }
        c.load_sc(&sc, "t.sc");
        let mut objs: Vec<ObjectRef> = (0..c.bodies.len() as u32).map(ObjectRef::Body).collect();
        objs.extend((0..c.stars.len() as u32).map(ObjectRef::Star));
        assert_batch_matches(&c, &objs, &[2_451_545.0, 2_451_550.3, 2_451_601.7, 2_400_000.5]);
        // The members whose parent chain is too long, the cyclic ones and only those, are left out:
        // D13 is 16 steps from Sys (the cap, still exact), D14 is 17 and is clamped at Sys A.
        let plan = BatchPlan::new(&c, objs.iter().copied());
        let skipped: Vec<&str> = c.bodies.iter().zip(&plan.bodies).filter(|(_, n)| n.is_none()).map(|(b, _)| b.names[0].as_str()).collect();
        assert_eq!(skipped, ["CycA", "CycB", "D14", "D15", "D16", "D17", "D18", "D19", "D20", "D21"]);
    }

    #[test]
    #[ignore = "reads the SpaceEngine install"]
    fn batch_plan_matches_on_the_solar_system() {
        let install = std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine");
        let Ok(vfs) = crate::vfs::Vfs::mount_install(install) else {
            eprintln!("install not found, skipping");
            return;
        };
        let mut c = Catalog::load(&vfs);
        if let Some(p) = vfs.list_ext("ephemerides", "436").into_iter().next() {
            c.de = Some(std::sync::Arc::new(crate::jpl_de::JplEphemeris::parse(vfs.read(&p).unwrap()).unwrap()));
        }
        c.moon_theories = crate::moon_theories::MoonTheories::from_install(install).ok().map(std::sync::Arc::new);
        crate::system_gen::complete_catalog_orbits(&mut c);
        assert!(c.de.is_some() && c.moon_theories.is_some());
        let mut objs = c.system_members("Solar System");
        objs.push(c.find("Solar System").unwrap());
        assert!(objs.len() > 2500, "{}", objs.len());
        // Inside and outside the DE436 span (1850-2150), where the planets fall back to orbits.
        assert_batch_matches(&c, &objs, &[2_461_212.123, 2_461_289.7, 2_458_211.75, 2_451_545.0, 2_200_000.5, 2_600_000.25]);
    }

    #[test]
    fn planet_orbits_star_at_semimajor_axis() {
        let mut c = Catalog::new();
        c.load_sc(
            "StarBarycenter \"Sys\" { RA 1 Dec 2 Dist 10 }\n\
             Star \"Sys A\" { ParentBody \"Sys\" Class \"G2V\" MassSol 1 }\n\
             Planet \"Sys b\" { ParentBody \"Sys A\" Radius 6000 Mass 1\n Orbit { SemiMajorAxis 1 Eccentricity 0 } }\n",
            "t.sc",
        );
        let eph = Ephemeris::new(&c);
        let star = eph.position(c.find("Sys A").unwrap(), crate::time::J2000);
        let planet = eph.position(c.find("Sys b").unwrap(), crate::time::J2000 + 10.0);
        let d = planet.delta_m(star).length();
        assert!((d / METERS_PER_AU - 1.0).abs() < 1e-9, "{}", d / METERS_PER_AU);
        let sys = c.stars.iter().position(|s| s.names[0] == "Sys").unwrap();
        assert!((star.to_parsecs().length() - 10.0).abs() < 1e-9);
        assert_eq!(eph.radius_m(ObjectRef::Star(sys as u32)), 0.0);
    }
}
