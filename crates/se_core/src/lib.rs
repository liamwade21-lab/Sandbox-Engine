//! Engine-independent core of the SpaceEngine port: data formats, catalogs and
//! astronomy. Nothing here depends on Bevy, so it can be unit-tested quickly and
//! reused by tools.

pub mod atm;
pub mod biomes;
pub mod body_params;
pub mod catalog;
pub mod classname;
pub mod climate;
pub mod cluster_stars;
pub mod coords;
pub mod custom_model;
pub mod dds;
pub mod ephemeris;
pub mod galaxy_model;
pub mod jpl_de;
pub mod lens_flare;
pub mod locale;
pub mod model_script;
pub mod model_sm;
pub mod moon_theories;
pub mod noise;
pub mod orbit;
pub mod places;
pub mod proc_galaxy;
pub mod proc_galsys;
pub mod planet_tiles;
pub mod procgen;
pub mod proc_surface;
pub mod rings;
pub mod rotation;
pub mod sc;
pub mod se_script;
pub mod ship;
pub mod star;
pub mod star_octree;
pub mod surface_gen;
pub mod system_gen;
pub mod tg;
pub mod time;
pub mod units;
pub mod upos;
pub mod vfs;

pub use glam;
