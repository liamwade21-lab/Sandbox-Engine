//! Which pose the ship's moving parts have now (ARCHITECTURE.md A6): the rigs' values (`elite_rig::Values`) and every rig
//! node's matrix in the ship's space, for `draw`.
//!
//! Today the pose is one of two still ones (`ShipRig::static_pose`), chosen by whether the pilot's gear is out: a parked ship
//! has it down. The animation task replaces "which values now" in `update` by running the state machines
//! (`elite_rig::Runner`); `draw` reads only `Parts::matrix`.

use bevy::math::Mat4;
use bevy::prelude::*;
use se_core::elite_rig::Values;

use super::pilot::Pilot;
use super::seat::Seat;
use super::SidewinderAssets;

#[derive(Resource, Default)]
pub struct Parts {
    /// A test's gear pose whatever the pilot's (`sw pose up|down|auto`): seated, the free camera can't look at the ship.
    pub dev_gear: Option<bool>,
    values: Values,
    /// The gear the matrices are posed for (None before the first pose).
    gear_down: Option<bool>,
    /// Each rig node's matrix in the ship's space (`ShipRig::nodes`' order).
    matrices: Vec<Mat4>,
}

impl Parts {
    /// A node's matrix now (identity before the first pose).
    pub fn matrix(&self, node: usize) -> Mat4 {
        self.matrices.get(node).copied().unwrap_or(Mat4::IDENTITY)
    }
}

/// Poses the parts when the gear changes: down while parked, else as the pilot's gear is (out or not, as the touchdown
/// counts it).
pub fn update(assets: Res<SidewinderAssets>, seat: Res<Seat>, pilot: Res<Pilot>, mut parts: ResMut<Parts>) {
    let down = parts.dev_gear.unwrap_or(!seat.seated || pilot.gear_out());
    if parts.gear_down == Some(down) {
        return;
    }
    parts.values = assets.rig.static_pose(down);
    match assets.rig.pose(&parts.values) {
        Ok(m) => {
            parts.matrices = m;
            info!("sidewinder: the parts posed with the gear {}", if down { "down, its doors open" } else { "up, its doors closed" });
        }
        // (`load` posed both still poses already; a refusal here would be a bug: keep the last matrices.)
        Err(e) => warn!("sidewinder: the parts can't be posed: {e}"),
    }
    parts.gear_down = Some(down);
}
