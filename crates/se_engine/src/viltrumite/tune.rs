//! The mod's numbers to tune, all in one place (the spec's table; later steps add theirs). None of them has
//! been felt in play yet.

/// Turning the mod on, he appears this far (m) in front of the camera (F2).
pub const SPAWN_AHEAD_M: f64 = 6.0;
/// Spawned under the ground, he is lifted this far (m) above it (F2).
pub const SPAWN_LIFT_M: f64 = 2.0;
/// The near plane is at most this share of the distance to him, within `NEAR_MIN_M`..`NEAR_MAX_M` (F7).
pub const NEAR_SHARE: f64 = 0.05;
pub const NEAR_MIN_M: f64 = 0.02;
pub const NEAR_MAX_M: f64 = 0.5;
/// How far the cape's links lean to straight down until the springs arrive (step 5).
pub const CAPE_HANG: f32 = 0.85;
/// Within this many of a body's radii (until the real frames arrive, step 4) he stands upright on it.
pub const ATTACH_RADII: f64 = 20.0;
/// Above a body's atmosphere (or this share of its radius, without one) his light is the CPU's: the body's own
/// shine on him counts there, and the atmosphere's tables stop at their top.
pub const BODY_LIGHT_AIRLESS_RADII: f64 = 0.02;
/// How high (m above the ground) the street lamps reach him fully, and where they stop.
pub const TOWN_FADE_M: f64 = 50.0;
pub const TOWN_TOP_M: f64 = 800.0;
/// The street lamps' strength on him: 1 is what the street objects and cars take (scatter_draw.wesl
/// street_surface), about a quarter of daylight.
pub const TOWN_LAMPS: f32 = 1.0;
/// Frames between working out how far into a town he is.
pub const TOWN_EVERY_FRAMES: u32 = 30;
