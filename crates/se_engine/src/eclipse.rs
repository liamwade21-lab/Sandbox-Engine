//! SE's eclipse casters (its `EclipseCasters[MAX_LIGHTS * MAX_ECLIPSES]` uniforms, filled per light by
//! FUN_1404bc7c0 from the light entries' 8 caster slots): for each light of a body, the bodies whose
//! shadow can reach it, stars included (a binary companion in front of its primary), largest apparent
//! size first. The sky shells and the rings read them per fragment (`eclipse.wesl`).

use bevy::math::DVec3;
use bevy::prelude::*;
use se_core::upos::UPos;

pub use crate::atmosphere::{EclipseLight, MAX_ECLIPSES};
use crate::system::{BodyLighting, MAX_LIGHTS, SystemBody};
use crate::universe::UniverseSet;

/// Every light's casters: `lights[0]` the brightest star (`BodyLighting::sun_dir`), then
/// `BodyLighting::extra` in order.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct EclipseCasters {
    pub lights: [EclipseLight; MAX_LIGHTS],
}

/// A body that can cast or receive a shadow.
#[derive(Clone, Copy)]
struct Body {
    entity: Entity,
    pos: UPos,
    radius_m: f64,
    star: bool,
}

/// SE's sky and ring shaders only need casters while the body is drawn as more than a point.
const MIN_PX_RADIUS: f64 = 0.15;

/// The light sources' and casters' bodies are those above 1 km (as the surface eclipses, terrain.rs).
const MIN_CASTER_RADIUS_M: f64 = 1000.0;

pub struct EclipsePlugin;

impl Plugin for EclipsePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            update_eclipse_casters.after(UniverseSet::Camera).before(crate::system::update_sky_shells).before(UniverseSet::FloatingOrigin),
        );
    }
}

/// The light source of a light seen from `lit` in direction `dir`, of the bodies lying that way (within
/// 0.26 degrees): a star when there is one (the nearest to the direction; a moon in front of it, or a
/// body behind it, is not the light), else the planet of planet shine, the largest one there.
fn light_source(bodies: &[Body], lit: &Body, dir: Vec3) -> Option<Body> {
    let dir = dir.as_dvec3().normalize_or(DVec3::Y);
    let cone: Vec<(f64, f64, &Body)> = bodies
        .iter()
        .filter(|b| b.entity != lit.entity)
        .filter_map(|b| {
            let d = b.pos.delta_m(lit.pos);
            let len = d.length().max(1.0);
            let cos = d.dot(dir) / len;
            (cos > 0.999_99).then_some((cos, b.radius_m / len, b))
        })
        .collect();
    let star = cone.iter().filter(|c| c.2.star).max_by(|a, b| a.0.total_cmp(&b.0));
    star.or_else(|| cone.iter().max_by(|a, b| a.1.total_cmp(&b.1))).map(|c| *c.2)
}

/// The casters of light `src` on the body `lit`, `extent_m` its reach from its centre (its rings' outer
/// radius, or the top of its atmosphere): the bodies between them whose penumbra cone can reach it.
fn gather(bodies: &[Body], lit: &Body, extent_m: f64, src: &Body) -> EclipseLight {
    let mut e = EclipseLight::default();
    let src_dist = src.pos.delta_m(lit.pos).length();
    if src_dist <= src.radius_m || lit.radius_m <= 0.0 {
        return e;
    }
    e.info.x = (src.radius_m / src_dist).min(1.0).asin() as f32;
    let mut found: Vec<(f64, Vec4)> = Vec::new();
    for c in bodies {
        if c.entity == lit.entity || c.entity == src.entity || c.radius_m < MIN_CASTER_RADIUS_M {
            continue;
        }
        if let Some((size, rel)) = shadow_reach(lit.pos.delta_m(c.pos), src.pos.delta_m(c.pos), src.radius_m, c.radius_m, extent_m) {
            found.push((size, (-rel / lit.radius_m).as_vec3().extend((c.radius_m / lit.radius_m) as f32)));
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (slot, (_, o)) in e.casters.iter_mut().zip(&found) {
        *slot = *o;
    }
    e.info.y = found.len().min(MAX_ECLIPSES) as f32;
    e
}

/// Whether a caster of radius `rc` throws its penumbra over something `extent` across around `w` (the
/// lit body relative to the caster) for a light `to_src` away of radius `rs`: the caster lies between
/// them and the lit body is within the cone's radius there. Returns the caster's apparent size from
/// the lit body and `w`.
fn shadow_reach(w: DVec3, to_src: DVec3, rs: f64, rc: f64, extent: f64) -> Option<(f64, DVec3)> {
    let d_src = to_src.length();
    if d_src <= rs {
        return None;
    }
    let axis = to_src / d_src;
    let t = -w.dot(axis);
    if t <= 0.0 {
        return None;
    }
    let a_s = (rs / d_src).min(1.0).asin();
    let perp = (w + axis * t).length();
    (perp <= extent + rc + t * a_s.tan()).then(|| (rc / w.length().max(1.0), w))
}

/// What receives a body's shadows beyond its surface: its sky shell (the top radius drawn, `m`) or its
/// rings (their outer radius in planet radii).
#[derive(Clone, Copy)]
enum Reach {
    Sky(f64),
    Rings(f32),
}

/// The reach of a body's shadow receivers from its centre (m): its rings' outer edge, the top of its
/// atmosphere as drawn (SE's Radiuses; a widened shell, Pluto's 1.25 R), at least its surface.
fn receiver_extent(radius_m: f64, reach: impl Iterator<Item = Reach>) -> f64 {
    reach
        .map(|r| match r {
            Reach::Sky(m) => m,
            Reach::Rings(k) => k as f64 * radius_m,
        })
        .fold(radius_m, f64::max)
}

pub fn update_eclipse_casters(
    mut commands: Commands,
    all: Query<(Entity, &SystemBody)>,
    mut lit: Query<(&SystemBody, &BodyLighting, Option<&mut EclipseCasters>)>,
    skies: Query<(&crate::system::SkyOf, &crate::universe::MeterScale)>,
    rings: Query<&crate::rings::Rings>,
    ring_mats: Res<Assets<crate::rings::RingMaterial>>,
) {
    // Only the bodies whose sky or rings read them.
    let receivers: Vec<(Entity, Reach)> = skies
        .iter()
        .map(|(s, scale)| (s.body, Reach::Sky(scale.0.max_element())))
        .chain(rings.iter().filter_map(|r| ring_mats.get(&r.material).map(|m| (r.body, Reach::Rings(m.u.geom.y)))))
        .collect();
    if receivers.is_empty() {
        return;
    }
    let bodies: Vec<Body> = all.iter().map(|(entity, b)| Body { entity, pos: b.pos, radius_m: b.radius_m, star: b.star }).collect();
    let mut done: Vec<Entity> = Vec::new();
    for &(entity, _) in &receivers {
        if done.contains(&entity) {
            continue;
        }
        done.push(entity);
        let Ok((b, light, casters)) = lit.get_mut(entity) else { continue };
        if b.star || b.px_radius < MIN_PX_RADIUS {
            continue;
        }
        let me = Body { entity, pos: b.pos, radius_m: b.radius_m, star: false };
        let extent = receiver_extent(b.radius_m, receivers.iter().filter(|r| r.0 == entity).map(|r| r.1));
        let mut set = EclipseCasters::default();
        // The main light is the star at `star_pos`; the others are found by their direction.
        let main = bodies.iter().find(|s| s.pos == light.star_pos && s.entity != entity).copied();
        if let Some(src) = main.or_else(|| light_source(&bodies, &me, light.sun_dir)) {
            set.lights[0] = gather(&bodies, &me, extent, &src);
        }
        for (slot, x) in set.lights[1..].iter_mut().zip(light.extra.iter().take(light.n_extra)) {
            if let Some(src) = light_source(&bodies, &me, x.dir) {
                *slot = gather(&bodies, &me, extent, &src);
            }
        }
        match casters {
            Some(mut c) => {
                if *c != set {
                    *c = set;
                }
            }
            None => {
                commands.entity(entity).insert(set);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CPU twin of `eclipse_value` in `eclipse.wesl` (SE's `EclipseValue`, eclipse_common.glh): the
    /// share of a light of angular radius `rl` hidden by a caster of angular radius `rc` whose centre is
    /// `d` away, the penumbra shaped by a smoothstep.
    fn eclipse_value(rl: f64, rc: f64, d: f64) -> f64 {
        let sum = rl + rc;
        if d >= sum {
            return 0.0;
        }
        let (min_r, max_phase) = if rl < rc {
            (rl, 1.0)
        } else if rl < 0.001 {
            (rc, rc * rc / (rl * rl))
        } else {
            (rc, (1.0 - rc.cos()) / (1.0 - rl.cos()))
        };
        if d <= rl.max(rc) - min_r {
            return max_phase;
        }
        let diff = (rl - rc).abs();
        let x = ((d - diff) / (sum - diff)).clamp(0.0, 1.0);
        // smoothstep(1, 0, x)
        let s = 1.0 - x;
        max_phase * s * s * (3.0 - 2.0 * s)
    }

    #[test]
    fn eclipse_value_matches_se() {
        // Apart, inside (annular: the caster's area share), total, and halfway through the penumbra.
        assert_eq!(eclipse_value(0.01, 0.005, 0.02), 0.0);
        let annular = eclipse_value(0.01, 0.005, 0.0);
        assert!((annular - (1.0 - 0.005f64.cos()) / (1.0 - 0.01f64.cos())).abs() < 1e-9);
        assert_eq!(eclipse_value(0.005, 0.01, 0.002), 1.0);
        let half = eclipse_value(0.01, 0.01, 0.01);
        assert!((half - 0.5).abs() < 1e-9, "{half}");
        // Tiny lights use the squared ratio.
        assert!((eclipse_value(0.0005, 0.0001, 0.0) - 0.04).abs() < 1e-9);
    }

    #[test]
    fn light_source_is_the_star() {
        let body = |i: u32, x: f64, r: f64, star: bool| Body { entity: Entity::from_raw_u32(i).unwrap(), pos: UPos::from_meters(DVec3::new(x, 0.0, 0.0)), radius_m: r, star };
        let lit = body(0, 0.0, 6e6, false);
        // A moon in front of the star and a planet far behind it, all in the light's direction.
        let bodies = [lit, body(1, 4e8, 1.7e6, false), body(2, 1.5e11, 7e8, true), body(3, 8e11, 7e7, false)];
        assert_eq!(light_source(&bodies, &lit, Vec3::X).map(|b| b.entity), Some(bodies[2].entity));
        // Planet shine: the largest planet there, not one beyond it.
        let shine = [lit, body(1, 4e8, 1.7e6, false), body(3, 8e11, 7e7, false)];
        assert_eq!(light_source(&shine, &lit, Vec3::X).map(|b| b.entity), Some(shine[1].entity));
        // The receiver reaches the sky's top or the rings' edge, at least the surface.
        assert_eq!(receiver_extent(6e6, [Reach::Sky(7.5e6), Reach::Rings(2.0)].into_iter()), 1.2e7);
        assert_eq!(receiver_extent(6e6, std::iter::empty()), 6e6);
    }

    #[test]
    fn shadow_cone_reach() {
        // A moon 10 radii sunward of the planet, the Sun far beyond it: the planet is in its shadow.
        let to_src = DVec3::new(1.5e11, 0.0, 0.0);
        let w = DVec3::new(-10.0, 0.0, 0.0);
        assert!(shadow_reach(w, to_src, 7e8, 0.3, 1.0).is_some());
        // Off to the side by more than the penumbra: no shadow.
        assert!(shadow_reach(DVec3::new(-10.0, 5.0, 0.0), to_src, 7e8, 0.3, 1.0).is_none());
        // Behind the light's side of the caster: none.
        assert!(shadow_reach(DVec3::new(10.0, 0.0, 0.0), to_src, 7e8, 0.3, 1.0).is_none());
        // A wider receiver (the rings) catches it further out.
        assert!(shadow_reach(DVec3::new(-10.0, 2.0, 0.0), to_src, 7e8, 0.3, 2.5).is_some());
    }
}
