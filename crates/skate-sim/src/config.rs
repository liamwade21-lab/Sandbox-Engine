//! Stand-in for upstream `config.rs` (command line and settings): only what the copied code reads.
use bevy::prelude::*;
use std::path::PathBuf;

#[derive(Resource)]
pub(crate) struct Config {
    pub asset_root: PathBuf,
    pub map: Option<skate_data::skate_map::SkateMap>,
}
