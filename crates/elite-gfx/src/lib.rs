//! Running Elite Dangerous's ship-UI movies (ARCHITECTURE.md A10): load a converted SWF, send it the
//! game's messages, step its clock, draw it into a GPU texture it is given, and hand back what it
//! sends. Ruffle (`vendor/ruffle`, patched) runs the movies; the bridge's ActionScript (`as3/`)
//! stands in for what Elite's exe supplies natively. No Bevy, no files.

pub mod bridge;
pub mod convert;
pub mod easing;
mod frames;
pub mod gpu;
pub mod host;
pub mod movie;
pub mod target;

#[cfg(test)]
mod tests;

pub use convert::convert_movie;
pub use gpu::Gpu;
pub use host::Host;
pub use movie::{Font, Movie, MovieFiles, Out};
