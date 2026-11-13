//! Stand-in for upstream `crash_context.rs` (a crash-report ring buffer; diagnostics only).
use skate_core::player::state::PhysicalStateId;

pub(crate) fn requested(_tick: u64, _from: PhysicalStateId, _to: PhysicalStateId) {}
