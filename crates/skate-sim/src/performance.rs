//! Stand-in for upstream `performance.rs` (frame-time diagnostics).
use bevy::prelude::*;

#[derive(Resource, Default)]
pub(crate) struct Performance;
impl Performance {
    pub(crate) fn physics(&mut self, _elapsed: std::time::Duration) {}
}
