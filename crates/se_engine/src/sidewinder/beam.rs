//! The Sidewinder's headlights on the ground (ARCHITECTURE.md A6, T4e-2). In Elite the headlights (the lights' switch, Insert)
//! light the ground in front of a landed or low-flying ship; here the ground's own lighting takes them as a term
//! (`ground_light.wesl` `ship_beam_at`: the ground's colour times the lamp's light, by the same rule as the ship's own lamps),
//! read from one record appended to the ground's per-frame record (`volumetric_clouds.rs` `CloudShadow`, its `beam`).
//!
//! The mod writes the engine's resource (`volumetric_clouds::ShipBeam`, as the hero writes `HeroSoil`), which the engine copies
//! into the record only when it changes. Each frame: the headlights' slots of the lights buffer as `lights.rs` last wrote them
//! (colour x level x the stand-in unit, the apex pulled back, the cone, the profile's K), placed from the camera in metres along
//! the render frame's axes (the ship's pose as `draw.rs` places its meshes, without the scaled space's factor), with each
//! slot's exposure term and the scene value. All zeros with no ship, the headlights out (their fade-out included: the slots'
//! own levels) or the ship unpowered, so the ground's shaders skip the term and draw as without the mod.
//!
//! Two parts of the record only the GPU writes (`exposure.rs`, each frame while a headlight is lit, before anything reads it):
//! the automatic exposure's compensation, and the IES profile's row as the ship's own sampler reads it (`BeamProfile`: the very
//! texture the ship's materials bind).
//!
//! The headlights are the slots the lights' switch drives (`Option_HeadlightsEnabled`: the headlight spot activity's two bones,
//! 500 m spots with the IES profile, and the point light activity's two at their housings, 5 m); the record holds
//! `SHIP_BEAM_LAMPS` of them, which a test checks against the shipped `ship.json`.

use bevy::math::{DVec3, Vec3, Vec4};
use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::system::SystemBody;
use crate::volumetric_clouds::{BeamLamp, ShipBeam, SHIP_BEAM_LAMPS};

use super::facts::ShipFacts;
use super::lights::{LightList, LightsState};
use super::material::ShipMaterial;
use super::Sidewinder;

/// The parameter of the lights' switch: the slots whose animator it drives are the headlights.
const SWITCH: &str = "Option_HeadlightsEnabled";

/// The headlights' slots in the lights list, in its order.
pub fn headlight_slots(list: &LightList) -> Vec<usize> {
    list.lamps.iter().enumerate().filter(|(_, l)| l.animator.bool_params.iter().any(|p| p == SWITCH)).map(|(i, _)| i).collect()
}

/// The headlights' IES profile: the texture every ship material binds (`ShipMaterial::ies`), for the profile's table the GPU
/// writes (`exposure.rs`). Found once the materials exist.
#[derive(Resource, Clone)]
pub struct BeamProfile {
    pub ies: Handle<Image>,
}

/// The record this frame: `None` (zeros) without a powered ship whose headlights give light.
fn lit(ship: &Sidewinder, facts: &ShipFacts, list: &LightList, state: &LightsState, cam: &FlyCamera, bodies: &Query<&SystemBody>) -> Option<ShipBeam> {
    if !facts.powered {
        return None;
    }
    let p = ship.parked.as_ref()?;
    let b = bodies.iter().find(|b| b.obj == p.obj)?;
    let (scene, auto) = state.scene()?;
    // The ship's model space to the render frame's axes from the camera, in metres (`draw.rs` `place`, without the factor).
    let rel = p.pos - b.orient.inverse() * cam.pos.delta_m(b.pos);
    let point = |v: Vec3| (b.orient * (rel + p.rot * v.as_dvec3())).as_vec3();
    let axis = |v: Vec3| (b.orient * p.rot * v.as_dvec3()).normalize_or(DVec3::ZERO).as_vec3();
    let mut out = ShipBeam::default();
    for (k, i) in headlight_slots(list).into_iter().take(SHIP_BEAM_LAMPS).enumerate() {
        let Some(r) = state.written().get(i).filter(|r| r.colour.truncate() != Vec3::ZERO) else { continue };
        out.lamps[k] = BeamLamp {
            pos_range: point(r.pos_range.truncate()).extend(r.pos_range.w),
            colour: r.colour,
            dir_cos: axis(r.dir_cos.truncate()).extend(r.dir_cos.w),
            spot: r.spot,
            exposure: list.lamps[i].exposure_terms(),
        };
    }
    if out.lamps.iter().all(|l| l.colour.truncate() == Vec3::ZERO) {
        return None;
    }
    out.head = Vec4::new(1.0, scene.scene_ev, if auto { 1.0 } else { 0.0 }, 0.0);
    Some(out)
}

/// One frame: the record written where it changed (zeros once when the ship goes or its headlights go out), and the profile's
/// texture found once.
#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    ship: Res<Sidewinder>,
    facts: Res<ShipFacts>,
    list: Option<Res<LightList>>,
    state: Res<LightsState>,
    profile: Option<Res<BeamProfile>>,
    materials: Res<Assets<ShipMaterial>>,
    cam: Res<FlyCamera>,
    bodies: Query<&SystemBody>,
    mut beam: ResMut<ShipBeam>,
) {
    if profile.is_none() {
        if let Some((_, m)) = materials.iter().next() {
            commands.insert_resource(BeamProfile { ies: m.ies.clone() });
        }
    }
    let now = list.as_deref().and_then(|list| lit(&ship, &facts, list, &state, &cam, &bodies)).unwrap_or_default();
    if *beam != now {
        if (beam.head.x > 0.5) != (now.head.x > 0.5) {
            info!("sidewinder: the headlights' beam on the ground {}", if now.head.x > 0.5 { "lit" } else { "out (the record is zeros)" });
        }
        *beam = now;
    }
}

pub fn build(app: &mut App) {
    // (After the lights' levels this frame and with the camera and bodies `draw.rs` places the ship with; before the floating
    // origin, so before the engine copies the record: `volumetric_clouds::update_cloud_view` runs after it.)
    app.add_systems(
        PostUpdate,
        update
            .run_if(resource_exists::<super::SidewinderAssets>)
            .after(super::lights::update)
            .before(crate::universe::UniverseSet::FloatingOrigin),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped ship's headlights: exactly the record's count, two spots with the IES profile and two point lights, each a
    /// slot of the hull's unflagged lights (so the ship's own outside materials light with the same slots).
    #[test]
    fn headlights_fill_the_record() {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        let rig = super::super::assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets.rig;
        let list = LightList::new(&rig, &rig.static_pose(true)).expect("the list");
        let slots = headlight_slots(&list);
        assert_eq!(slots.len(), SHIP_BEAM_LAMPS, "the record holds every headlight of ship.json");
        for &i in &slots {
            let l = &list.lamps[i];
            assert!(i < list.hull && !l.flagged() && l.spot == (l.light.use_ies == Some(true)), "slot {i}: {}", l.holder());
        }
        assert_eq!(slots.iter().filter(|&&i| list.lamps[i].spot).count(), 2, "the two spots");
    }
}
