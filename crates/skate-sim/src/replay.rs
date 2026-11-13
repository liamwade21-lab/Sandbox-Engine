//! Stand-in for upstream `replay.rs` (the replay recorder and player): never active here.
use crate::presentation::Snapshot;
use bevy::prelude::*;

#[derive(Resource, Default)]
pub(crate) struct Replay {
    pub active: bool,
    pub free_camera: Option<Transform>,
}
impl Replay {
    pub(crate) fn sample(&self) -> Option<(&Snapshot, &Snapshot, f32)> { None }
    pub(crate) fn record(&mut self, _snapshot: Snapshot, _delta: f64, _reset: bool) {}
}
