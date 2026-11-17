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
use super::lights::{Lamp, LightList, LightsState};
use super::material::{ShipLight, ShipMaterial};
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

/// One headlight's record: its slot `lamp` as `lights.rs` last wrote it to the lights buffer (`r`), placed by `point` and
/// turned by `axis` (the ship's model space to the camera's), with the slot's exposure term, the one the ship's materials take
/// for it (`Lamp::exposure_terms`, `ShipScene`).
fn beam_lamp(lamp: &Lamp, r: &ShipLight, point: impl Fn(Vec3) -> Vec3, axis: impl Fn(Vec3) -> Vec3) -> BeamLamp {
    BeamLamp {
        pos_range: point(r.pos_range.truncate()).extend(r.pos_range.w),
        colour: r.colour,
        dir_cos: axis(r.dir_cos.truncate()).extend(r.dir_cos.w),
        spot: r.spot,
        exposure: lamp.exposure_terms(),
    }
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
        out.lamps[k] = beam_lamp(&list.lamps[i], r, point, axis);
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

    /// The exposure path, which the shipped headlights never take (their exposure switch is off in Elite's data): for a
    /// headlight whose switch is on, the record carries the slot's own term (what the ship's materials read for that slot,
    /// `ShipScene`), and the ground's rule on it (`ground_light.wesl` `beam_exposure`, stated here) gives the ship's factor
    /// (`lights::exposure_factor`), the adapted compensation's stops added to the scene value in photo mode 2.
    #[test]
    fn a_headlight_with_exposure_on_takes_the_ships_factor() {
        use super::super::lights::{exposure_factor, PanelGlow};
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        let rig = super::super::assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets.rig;
        let mut list = LightList::new(&rig, &rig.static_pose(true)).expect("the list");
        let i = headlight_slots(&list).into_iter().find(|&i| list.lamps[i].spot).expect("a spot");
        assert!(!list.lamps[i].light.exposure_enable, "the shipped headlights' exposure switch is off");
        let l = &mut list.lamps[i].light;
        (l.exposure_enable, l.exposure_bias, l.floats_112) = (true, 0.5, [8.0, 8.5]);
        let lamp = &list.lamps[i];
        let r = lamp.record(1.0, &PanelGlow::default());
        let rec = beam_lamp(lamp, &r, |v| v, |v| v);
        assert_eq!(rec.exposure, Vec4::new(1.0, 0.5, 8.0, 8.5));
        assert_eq!(rec.exposure, lamp.exposure_terms(), "the ship's slot term");
        assert_eq!((rec.colour, rec.spot, rec.pos_range.w), (r.colour, r.spot, r.pos_range.w), "the slot as the lights buffer has it");
        // `beam_exposure(head, t)`: head = (lit, scene value, adapt on, adapted stops).
        let ground = |head: Vec4, t: Vec4| if t.x < 0.5 { 1.0 } else { (head.y + head.z * head.w + t.y).clamp(t.z, t.w).exp2() / 8.0 };
        for scene_ev in [5.0f32, 7.6, 7.9, 8.2, 12.0] {
            for adapted in [0.0f32, 0.3, -1.0] {
                let want = exposure_factor(&lamp.light, scene_ev + adapted);
                let got = ground(Vec4::new(1.0, scene_ev, 1.0, adapted), rec.exposure);
                assert!((got - want).abs() <= want * 1e-6, "scene {scene_ev} adapted {adapted}: {got} vs {want}");
            }
            assert_eq!(ground(Vec4::new(1.0, scene_ev, 0.0, 2.0), rec.exposure), exposure_factor(&lamp.light, scene_ev), "photo mode 2 off");
        }
    }
}
