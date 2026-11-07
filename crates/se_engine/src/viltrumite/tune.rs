//! The mod's numbers to tune, all in one place: the spec's "Numbers to tune" table and what later steps add to it.
//! Nothing here has been felt in play by the user yet; what each number really gives is in the spec's Build log.
//! Numbers of steps not built yet are here too, as the spec's estimates.
#![allow(dead_code)] // the later steps' numbers

// ---- Walking (step 5) ----
pub const WALK_MS: f64 = 1.8;
pub const RUN_MS: f64 = 5.0;
pub const SPRINT_MS: f64 = 11.0;
/// Shift held this long before the run turns into a sprint.
pub const SPRINT_AFTER_S: f64 = 1.5;
/// The clip's rate over its stride speed stays between these.
pub const STRIDE_RATE: (f64, f64) = (0.7, 1.5);
/// The jump's impulse up (m/s), no drag rising: Earth 40 m, the Moon about 242 m.
pub const JUMP_MS: f64 = 28.0;

// ---- Flight ----
/// WASD alone.
pub const FLY_MS: f64 = 20.0;
/// The Shift ramp: seconds held, speed (m/s), logarithmic between the points.
pub const RAMP: [(f64, f64); 4] = [(0.0, FLY_MS), (1.0, 300.0), (3.0, 3_000.0), (6.0, 30_000.0)];
/// Speed follows the ramp with this lag (s).
pub const RAMP_TAU_S: f64 = 0.35;
/// Beyond the ramp's end, in space, the speed doubles every this many Shift seconds: c about 13 s more, 1000 c
/// about 23 s more.
pub const SPACE_DOUBLE_S: f64 = 1.0;
/// With no keys the speed halves every this many seconds.
pub const BRAKE_HALF_S: f64 = 0.15;
/// The cap in air (m/s), below `CAP_AIR_SCALE_HEIGHTS`.
pub const CAP_AIR_MS: f64 = 30_000.0;
pub const CAP_AIR_SCALE_HEIGHTS: f64 = 25.0;
/// The cap in space: this times the distance to the nearest surface, per second (c at 100,000 km, 1500 c at 1 AU),
/// but never under `CAP_AIR_MS`.
pub const CAP_SPACE_PER_S: f64 = 3.0;
/// Over a cap he brakes with this time constant (s). The spec's estimate was 0.2: with 3 x the distance as the cap that
/// system is underdamped, and at 4 c he reached the ground at 2 c. It needs under 1 / (4 x 3) to ease in.
pub const CAP_BRAKE_TAU_S: f64 = 0.08;
/// Numeric guard only (m/s).
pub const GUARD_MS: f64 = 1.0e26;
/// How fast his flight turns to where he's steered (s).
pub const STEER_TAU_S: f64 = 0.25;
/// Slow flight (under `FLY_MS` x this) just blends to what's asked, with this time constant (s).
pub const SLOW_FLIGHT_FACTOR: f64 = 1.5;
pub const SLOW_FLIGHT_TAU_S: f64 = 0.12;

// ---- Tiers (the clips) ----
pub const HOVER_UNDER_MS: f64 = 3.0;
pub const CRUISE_RISE_MS: f64 = 120.0;
pub const CRUISE_FALL_MS: f64 = 100.0;
pub const FAST_RISE_MS: f64 = 1_500.0;
pub const FAST_FALL_MS: f64 = 1_200.0;
/// Above this his body points along his velocity, below along the aim.
pub const BODY_ALONG_VELOCITY_MS: f64 = 30.0;
/// How fast his body turns to where it should point (s).
pub const BODY_TAU_S: f64 = 0.18;
/// Climbing steeper than this (degrees), rising and falling, Hyper plays the arm-up clip.
pub const HYPER_UP_DEG: (f64, f64) = (48.0, 44.0);
/// Still this long in Hover (s), he crosses his arms.
pub const ARMS_CROSSED_AFTER_S: f32 = 4.0;
/// Crossfades (s).
pub const FADE_S: f32 = 0.35;
pub const FADE_BREAK_S: f32 = 0.25;
pub const FADE_HYPER_S: f32 = 0.5;
/// How long the take-off's crouch plays (s), the part of `charge_flight_stopped` before its launch.
pub const TAKE_OFF_CROUCH_S: f32 = 0.5;
/// How fast a lean follows the steering (s).
pub const LEAN_TAU_S: f32 = 0.2;
/// A lean clip's weight is full from this lean on, and 0 at no lean (their first frames aren't on the base pose).
pub const LEAN_FULL_WEIGHT_AT: f32 = 0.2;

// ---- Landing (step 5), breaking (step 10) ----
pub const SOFT_LANDING_MS: f64 = 12.0;
pub const LAND_S: f64 = 0.4;
pub const HARD_LANDING_MS: f64 = 35.0;
pub const HARD_LANDING_S: f64 = 0.8;
/// Along a wall's normal: the same as `HARD_LANDING_MS` on purpose (T5).
pub const BREAK_SPEED_MS: f64 = 35.0;

// ---- Falling ----
/// `CdA/m` (m2/kg): Earth's terminal speed 45 m/s. Implicit, `v / (1 + c |v| dt)`: explicit blows up at speed.
pub const DRAG_CDA_M: f64 = 0.0078;

// ---- Frames ----
/// Attached below `min(ATTACH_RADII x radius, ATTACH_MAX_M)` above the body's highest ground, leaving at
/// `LEAVE_FACTOR` x that.
pub const ATTACH_RADII: f64 = 0.5;
pub const ATTACH_MAX_M: f64 = 300_000.0;
pub const LEAVE_FACTOR: f64 = 1.2;
/// A carrier gives way only to a body this much nearer (the current one's height over the new one's).
pub const CARRIER_HYSTERESIS: f64 = 1.25;
/// Sphere trace over the ground: a step is `TRACE_SHARE` of the height, at least `TRACE_MIN_M`, at most this many.
pub const TRACE_SHARE: f64 = 0.8;
pub const TRACE_MIN_M: f64 = 1.0;
pub const TRACE_STEPS: u32 = 16;
/// Halvings to find where a path that ran out of those steps goes in.
pub const TRACE_BISECT: u32 = 24;

// ---- Ground, until step 5's walking ----
/// His middle stays this far above the ground (m): his soles are about 1 m below it.
pub const STAND_M: f64 = 1.1;
/// Where nothing of the ground is loaded he isn't let lower than this over the highest ground could be (m).
pub const UNLOADED_MARGIN_M: f64 = 100.0;
/// He stops this far above a star's or a sphere's radius, as a share of the radius; the camera is kept at 2e-6 R
/// (terrain.rs `SPHERE_MIN_ALTITUDE`).
pub const SPHERE_STOP_SHARE: f64 = 3.0e-6;
/// In a gas giant whose decks draw, he stops this far above the camera's own floor (m).
pub const GIANT_STOP_M: f64 = 50.0;

// ---- Camera ----
pub const CAM_DIST_M: f64 = 6.0;
pub const CAM_WHEEL: f64 = 0.9;
pub const CAM_MIN_M: f64 = 2.5;
pub const CAM_MAX_M: f64 = 200.0;
/// Further back by this x log10(v / 30) over 30 m/s.
pub const CAM_SPEED_ZOOM: f64 = 0.8;
/// The camera sits this far above his middle, along the view's up (his head, plus the spec's 0.3 m).
pub const CAM_UP_M: f64 = 1.0;
/// The camera's floor over the ground (m): `keep_camera_above_ground`'s 1.5 m push must never fire.
pub const CAM_FLOOR_M: f64 = 2.0;
/// How fast the roll goes out of the view below the horizon's height (s).
pub const ROLL_LEVEL_TAU_S: f64 = 1.0;
/// The near plane is at most this share of the distance to him, within `NEAR_MIN_M`..`NEAR_MAX_M` (F7).
pub const NEAR_SHARE: f64 = 0.05;
pub const NEAR_MIN_M: f64 = 0.02;
pub const NEAR_MAX_M: f64 = 0.5;
/// How fast the speed's part of the camera's distance follows the speed (s): a sudden stop mustn't jump the camera.
pub const CAM_ZOOM_TAU_S: f64 = 0.3;
/// The camera moved this far (m) by anything but him (Places, a dev command): he goes with it.
pub const TELEPORT_M: f64 = 50.0;

// ---- Turning on ----
/// Turning the mod on, he appears this far (m) in front of the camera (F2).
pub const SPAWN_AHEAD_M: f64 = 6.0;
/// Spawned under the ground, he is lifted this far (m) above it (F2).
pub const SPAWN_LIFT_M: f64 = 2.0;
/// Turning off this many radii (of height) or fewer from a body, the camera turns with it (F3).
pub const OFF_BIND_RADII: f64 = 2.0;

// ---- Punches and the thunderclap (step 8) ----
pub const PUNCH_REACH_M: f64 = 5.0;
pub const GROUND_PUNCH_DEG: f64 = 45.0;
pub const GROUND_PUNCH_REACH_M: f64 = 15.0;
pub const GROUND_PUNCH_CRATER_M: f64 = 6.0;
/// Crater radius: `clamp(CRATER_K sqrt(v), CRATER_MIN_M, CRATER_MAX_M)`.
pub const CRATER_K: f64 = 0.4;
pub const CRATER_MIN_M: f64 = 2.0;
pub const CRATER_MAX_M: f64 = 60.0;
pub const THUNDERCLAP_HOLD_S: (f64, f64) = (0.4, 2.5);
pub const THUNDERCLAP_RING_M: (f64, f64) = (30.0, 150.0);

// ---- Slopes (step 5) ----
pub const SLOPE_WALK_DEG: f64 = 50.0;
pub const SLOPE_SLIDE_DEG: f64 = 70.0;

// ---- His light (step 3) ----
/// How far the cape's links lean to straight down until the springs arrive (step 5).
pub const CAPE_HANG: f32 = 0.85;
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
/// Far from any light a faint light from the camera keeps him in sight: the least a channel of his further lights may
/// add up to. (Measured: his cape's red reads 51 of 255 between the stars, at 0.03 it was 5, which hides him on black.)
pub const FAINT_FILL: f32 = 0.2;
/// The same at night near a body, where the ground is dark too and he needs less.
pub const FAINT_FILL_NIGHT: f32 = 0.008;
/// From this many radii of height over the nearest body to this many it goes from the night's to the far one.
pub const FILL_NEAR_RADII: f64 = 2.0;
pub const FILL_FAR_RADII: f64 = 50.0;
/// Near a body it comes in as the star goes down: the sine of how far under his horizon it is, from and to.
pub const FILL_NIGHT_FROM: f64 = 0.0;
pub const FILL_NIGHT_TO: f64 = 0.2;
