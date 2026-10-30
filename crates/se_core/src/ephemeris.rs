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
        let c = self.catalog;
        let orbit = match r {
            ObjectRef::Star(i) => c.stars[i as usize].orbit.as_ref(),
            ObjectRef::Body(i) => c.bodies[i as usize].orbit.as_ref(),
            _ => None,
        };
        if let Some(p) = self.de_position(orbit, jd) {
            return p;
        }
        match r {
            ObjectRef::Star(i) => {
                let s = &c.stars[i as usize];
                match (&s.parent, &s.orbit) {
                    (Some(p), Some(o)) if depth < MAX_DEPTH => match c.find(p) {
                        Some(pr) if pr != r => {
                            let base = self.position_depth(pr, jd, depth + 1);
                            base.offset_m(self.orbit_offset(o, pr, base, jd))
                        }
                        _ => UPos::from_parsecs(s.pos_pc),
                    },
                    (Some(p), None) if !s.has_position && depth < MAX_DEPTH => match c.find(p) {
                        Some(pr) if pr != r => self.position_depth(pr, jd, depth + 1),
                        _ => UPos::from_parsecs(s.pos_pc),
                    },
                    _ => UPos::from_parsecs(s.pos_pc),
                }
            }
            ObjectRef::Body(i) => {
                let b = &c.bodies[i as usize];
                let Some(pr) = c.find(&b.parent).filter(|pr| *pr != r) else {
                    return UPos::ZERO;
                };
                if depth >= MAX_DEPTH {
                    return UPos::ZERO;
                }
                let base = self.position_depth(pr, jd, depth + 1);
                if let Some([lon, lat, dist_km]) = b.fixed_pos_polar() {
                    let dir = DVec3::from_array(crate::se_script::polar_dir(lon, lat));
                    return base.offset_m(self.orientation(pr, jd) * dir * (dist_km * 1e3));
                }
                match &b.orbit {
                    Some(o) => match self.moon_theory_offset(o, jd) {
                        Some(off) => base.offset_m(off),
                        None => base.offset_m(self.orbit_offset(o, pr, base, jd)),
                    },
                    None => base,
                }
            }
            ObjectRef::Galaxy(i) => UPos::from_parsecs(c.galaxies[i as usize].pos_pc),
            ObjectRef::Cluster(i) => UPos::from_parsecs(c.clusters[i as usize].pos_pc),
            ObjectRef::Nebula(i) => UPos::from_parsecs(c.nebulae[i as usize].pos_pc),
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
                // SE's IgnoreMoonsRotationIAU (main-def.cfg, true): a moon's IAU model gives way to
                // tidal locking.
                if let (Some([lon, lat, _]), Some(p), RotationLock::Tidal) = (b.fixed_pos_polar(), parent, b.lock) {
                    return fixed_frame(self.orientation(p, jd), lon, lat, &b.block);
                }
                let moon_iau = matches!(b.rotation, RotationModel::Iau(_)) && matches!(parent, Some(ObjectRef::Body(_)));
                let locked = match b.lock {
                    RotationLock::MoonDe => self.moon_de(jd).or_else(|| parent.and_then(|p| self.tidal_lock(r, p, jd))),
                    RotationLock::Tidal => parent.and_then(|p| self.tidal_lock(r, p, jd)),
                    RotationLock::None if moon_iau => parent.and_then(|p| self.tidal_lock(r, p, jd)),
                    RotationLock::None => None,
                };
                locked.unwrap_or_else(|| b.rotation.orientation(jd))
            }
            ObjectRef::Star(i) => self.catalog.stars[i as usize].rotation.orientation(jd),
            ObjectRef::Galaxy(i) => self.catalog.galaxies[i as usize].orientation,
            ObjectRef::Nebula(i) => self.catalog.nebulae[i as usize].orientation,
            ObjectRef::Cluster(_) => DQuat::IDENTITY,
        }
    }

    /// SE's `Moon-DE` rotation: the Moon's librations from the JPL ephemeris (3-1-3 Euler angles of
    /// its principal axes against the ICRF), within the ephemeris's span.
    fn moon_de(&self, jd: f64) -> Option<DQuat> {
        let e = self.catalog.de.as_ref()?.lunar_euler(jd)?;
        Some(crate::rotation::from_icrf_euler(e.x, e.y, e.z))
    }

    /// A tidally locked body: longitude 0 (+X) faces the parent and the pole (+Y) lies along the
    /// orbit's normal (its angular momentum, so retrograde moons spin retrograde).
    fn tidal_lock(&self, r: ObjectRef, parent: ObjectRef, jd: f64) -> Option<DQuat> {
        let rel = |t: f64| self.position(r, t).delta_m(self.position(parent, t));
        let (a, b) = (rel(jd), rel(jd + 1.0e-3));
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
