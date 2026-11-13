//! The Skate 3 Rust Engine's per-tick skater simulation, extracted from `skate-game` (see UPSTREAM.md).
//! Everything below except `lib.rs`, `sim.rs` and the `standins` modules is a verbatim copy.
#![allow(dead_code, unused_imports, unused_variables, unused_mut, unused_assignments, ambiguous_glob_reexports, mismatched_lifetime_syntaxes, clippy::all)]

mod animation;
mod animation_pose;
mod camera;
mod custom_difficulty;
mod difficulty;
mod graph_host;
mod graph_runtime;
mod grind_world;
mod input;
mod physics;
mod presentation;
mod scoring_runtime;
mod apt_text;
mod scoring_hud;
mod skater_animation;

// Stand-ins for what lives outside the extracted closure, at the same paths.
mod app;
mod config;
mod crash_context;
mod customiser;
mod modding;
mod performance;
mod replay;
mod skate_world;
mod world;

mod sim;
pub use sim::*;

#[cfg(test)]
mod facade_tests;
