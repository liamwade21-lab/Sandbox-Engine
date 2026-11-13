//! skate-sim: the facade's door into `physics`. A child module sees the private items of `physics`
//! (the exchange buffer, the world, the clock, `frame::advance`), so the facade needs no change to
//! the copied files beyond declaring this module. New code; it contains no simulation arithmetic.
use super::*;
use skate_core::{input::controller::ActionMap, physics::phase::PhysicsEvent};

/// One production tick of the physics, animation graphs and camera (`frame::advance`).
pub(crate) fn advance(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
    controls: &mut PlayerControls,
    graphs: &crate::graph_runtime::StockGraphs,
    actions: &mut dyn ActionMap,
    input_available: bool,
    camera: &mut crate::camera::CameraRuntime,
) -> Result<(), String> {
    frame::advance(physics, skater, controls, graphs, actions, input_available, camera)
}

/// Replace the static collision world (and what is derived from it) before the skater is built, or at
/// any later time followed by a teleport (the skater's retained contacts refer to the old world).
pub(crate) fn install_world(physics: &mut GamePhysics, world: BoardWorld) -> Result<(), String> {
    physics.offboard_grab_scene = offboard::grab_scene::Registry::new(&world, Vec::new(), Vec::new())?;
    physics.world = world;
    Ok(())
}

/// The host has no rails: replace the test course's grind geometry with an empty static provider.
/// Must run before `SkaterRuntime::load`, whose trajectory binds a clone of this `Arc`.
pub(crate) fn clear_grind_world(physics: &mut GamePhysics) -> Result<(), String> {
    physics.grind_world = std::sync::Arc::new(crate::grind_world::StaticProvider::new(None)?);
    Ok(())
}

/// The physics events of the tick that just ran (state changes, landing, wipeout, contacts).
pub(crate) fn events(physics: &GamePhysics) -> &[PhysicsEvent] {
    physics.exchange.events()
}

pub(crate) fn world_ref(physics: &GamePhysics) -> &BoardWorld { &physics.world }
pub(crate) fn floor_material(physics: &GamePhysics) -> skate_core::physics::contact::RetailContactMaterial {
    physics.settings.floor_material
}
pub(crate) fn wheel_rest_offset(physics: &GamePhysics) -> f32 {
    physics.settings.wheel_radius - physics.settings.authored[0].translation.y
}
pub(crate) fn failed(physics: &GamePhysics) -> bool { physics.failed }
pub(crate) fn mark_failed(physics: &mut GamePhysics) { physics.failed = true; }
pub(crate) fn contact_count(physics: &GamePhysics) -> usize { physics.contact_count }
