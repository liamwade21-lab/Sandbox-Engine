//! The Faulcon DeLacy Sidewinder mod's ship rules: Elite Dangerous's, as far as each can be proven
//! (`docs/spec-mods-sidewinder.md`; each rule's formula, grade and source is in `docs/elite-flight-model.md`).
//!
//! No Bevy, no files, f64, a fixed tick: it runs and is tested without the game (`cargo test --release -p elite-sim`), and
//! its trace tests (measured traces of the real game replayed inside the gate, `trace.rs`) are the proof.

pub mod data;
pub mod fsd;
pub mod trace;
