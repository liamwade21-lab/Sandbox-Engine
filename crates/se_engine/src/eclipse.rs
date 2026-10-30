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

/// The light source of a light seen from `lit` in direction `dir`: of the bodies (a star, or a planet
/// for planet shine) lying that way, the farthest, as any other there is in front of it (a moon
/// eclipsing it).
fn light_source(bodies: &[Body], lit: &Body, dir: Vec3) -> Option<Body> {
    let dir = dir.as_dvec3().normalize_or(DVec3::Y);
    bodies
        .iter()
        .filter(|b| b.entity != lit.entity)
        .map(|b| (b.pos.delta_m(lit.pos), b))
        .filter(|(d, _)| d.normalize_or(DVec3::ZERO).dot(dir) > 0.999_99)
        .max_by(|a, b| a.0.length_squared().total_cmp(&b.0.length_squared()))
        .map(|(_, b)| *b)
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

/// The reach of a body's shadow receivers: its rings' outer edge, else the top of its atmosphere (SE's
/// Radiuses), else its surface.
fn receiver_extent(radius_m: f64, rings_outer: Option<f64>) -> f64 {
    rings_outer.unwrap_or(radius_m * 1.1).max(radius_m * 1.1)
}

pub fn update_eclipse_casters(
    mut commands: Commands,
    all: Query<(Entity, &SystemBody)>,
    mut lit: Query<(&SystemBody, &BodyLighting, Option<&mut EclipseCasters>)>,
    skies: Query<&crate::system::SkyOf>,
    rings: Query<&crate::rings::Rings>,
    ring_mats: Res<Assets<crate::rings::RingMaterial>>,
) {
    // Only the bodies whose sky or rings read them.
    let receivers: Vec<(Entity, Option<f32>)> = skies
        .iter()
        .map(|s| (s.body, None))
        .chain(rings.iter().map(|r| (r.body, ring_mats.get(&r.material).map(|m| m.u.geom.y))))
        .collect();
    if receivers.is_empty() {
        return;
    }
    let bodies: Vec<Body> = all.iter().map(|(entity, b)| Body { entity, pos: b.pos, radius_m: b.radius_m }).collect();
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
        let me = Body { entity, pos: b.pos, radius_m: b.radius_m };
        let outer = receivers.iter().filter(|r| r.0 == entity).find_map(|r| r.1).map(|k| k as f64 * b.radius_m);
        let extent = receiver_extent(b.radius_m, outer);
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
