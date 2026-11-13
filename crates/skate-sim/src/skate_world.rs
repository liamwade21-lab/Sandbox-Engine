//! Stand-in for upstream `skate_world.rs` (map rendering + collision). Host worlds are installed
//! through `Sim::set_world`; a map file is never loaded here.
use skate_core::physics::{board_world::BoardWorld, contact::RetailContactMaterial};
use skate_data::skate_map::SkateMap;

pub(crate) fn collision_world(_map: &SkateMap, _material: RetailContactMaterial) -> Result<BoardWorld, String> {
    Err("skate-sim loads no map files; use Sim::set_world".into())
}
