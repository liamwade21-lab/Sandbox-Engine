//! Stand-in for upstream `modding/` (the Lua mod host). No mod is ever loaded here, so the
//! hooks the simulation calls behave as upstream does with no mods: nothing attached, nothing
//! overridden. `player_physics` (the skater-side hooks) is the upstream file.
use bevy::prelude::*;

pub(crate) mod player_physics;

#[derive(Resource, Default)]
pub(crate) struct Mods;

pub(crate) fn player_attached(_mods: &Mods) -> bool { false }
pub(crate) fn player_suspended(_mods: &Mods) -> bool { false }
pub(crate) fn override_actions(_mods: Option<&Mods>, _values: &mut [f32; 18]) {}
