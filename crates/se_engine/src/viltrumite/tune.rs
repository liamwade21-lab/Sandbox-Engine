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
/// How fast he reaches the speed the keys ask for, and comes to a stop (m/s2).
pub const WALK_ACCEL: f64 = 12.0;
/// Gaits by speed, rising and falling (m/s): idle under the first, walk under the second, run under the third.
pub const GAIT_IDLE_MS: (f64, f64) = (0.35, 0.2);
pub const GAIT_RUN_MS: (f64, f64) = (3.2, 2.8);
pub const GAIT_SPRINT_MS: (f64, f64) = (8.0, 7.0);
/// A gait's crossfade (s), and from standing to walking.
pub const FADE_GAIT_S: f32 = 0.2;
/// How fast his body turns to the aim while he stands (s): his feet don't step for it.
pub const STAND_TURN_TAU_S: f64 = 0.45;
/// Standing, more than this off the aim (degrees) he plays a turn clip.
pub const TURN_CLIP_OVER_DEG: f64 = 70.0;
/// A turn clip's root ends this far round, as authored (degrees): his body takes it over when it ends.
pub const TURN_CLIP_DEG: f64 = 90.0;
/// The jump's clips: from where in `jump_fall_start_stopped` (its crouch is over by then: he's already off the ground), at
/// what rate, and where it has reached the air pose, when `jump_fall_loop` takes over.
pub const JUMP_CLIP_FROM_S: f32 = 0.45;
pub const JUMP_CLIP_RATE: f32 = 2.2;
pub const JUMP_CLIP_END_S: f32 = 2.2;
/// A landing's clips: `jump_fall_land_hard` from here (its air pose is before this) at this rate.
pub const LAND_CLIP_FROM_S: f32 = 0.12;
pub const LAND_CLIP_RATE: f32 = 1.3;
/// Crossfade into a landing clip, and out of one to standing (s).
pub const FADE_LAND_S: f32 = 0.08;
pub const FADE_UP_S: f32 = 0.4;
/// How fast he comes upright on landing (s): the impact hides it.
pub const LAND_UPRIGHT_TAU_S: f64 = 0.05;
/// Sliding down a slope too steep to stand on: this share of gravity along it, and at most this fast (m/s).
pub const SLIDE_SHARE: f64 = 0.75;
pub const SLIDE_MAX_MS: f64 = 25.0;
/// F6: the ground under him drops further than `LEDGE_DROP_M + v dt tan(SLOPE_WALK_DEG)` in a frame on the same level: he falls.
pub const LEDGE_DROP_M: f64 = 0.6;
/// The side of the triangle of ground samples that gives the normal (m).
pub const NORMAL_SPAN_M: f64 = 1.5;
/// A finer node moved the ground under him: he's drawn where he was, easing out with this time constant (s).
pub const GROUND_EASE_TAU_S: f64 = 0.15;
/// Most the visual offset ever holds (m): beyond that it's a jump, not a new node.
pub const GROUND_EASE_MAX_M: f64 = 40.0;
/// His root drops to the lower foot's ground by at most this (m), smoothed.
pub const ROOT_DROP_MAX_M: f64 = 0.25;
pub const ROOT_DROP_TAU_S: f64 = 0.1;
/// His drawn height over the capsule follows the clip's `ground_z` with this time constant (s).
pub const CLIP_GROUND_TAU_S: f64 = 0.1;
/// The ground's stones hold still round his feet and his shadow (`planet.wesl`) until the shadow's strength is down to this, and let go
/// with what's left of it: nothing changes at any one height.
pub const SOIL_HOLD_SHARE: f64 = 0.3;
/// Hovering over the sea, his soles are this far up (m).
pub const SEA_HOVER_M: f64 = 1.5;

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

// ---- The buildings' probe (step 9, F12.1) ----
/// Round the capsule's axis, beyond each building's half diagonal (m).
pub const PROBE_MARGIN_M: f32 = 40.0;
/// The capsule reaches `v dt (lag + PROBE_AHEAD_FRAMES)` ahead of him; until a list has come back the lag is taken as
/// `PROBE_LAG_FRAMES` (measured, then the measured one, at most `PROBE_LAG_MAX`).
pub const PROBE_AHEAD_FRAMES: f64 = 2.0;
pub const PROBE_LAG_FRAMES: u32 = 4;
pub const PROBE_LAG_MAX: u32 = 8;

// ---- Solid buildings (step 9's second part, F12.8, F6, F7, F2) ----
/// His sphere against the buildings' walls, round his middle (m).
pub const HIT_RADIUS_M: f64 = 0.45;
/// A slice's top is his ground (a roof, F6) once his feet are no lower than this under it (m); lower, it's a wall.
pub const ROOF_STEP_M: f64 = 0.7;
/// Height slices per model, at most (F12.8).
pub const SLICES: usize = 8;
/// Slightly inside a wall (a list's box moved onto him by a finer node, or he slid off a roof's edge): pushed out at this speed
/// (m/s), at least `PUSH_MIN_M` a frame, so it never pops.
pub const PUSH_MS: f64 = 3.0;
pub const PUSH_MIN_M: f64 = 0.02;
/// Further than this inside a building's box (m) is F2's case (a Go To or a jump of the camera put him there): it's let alone
/// until he's out of it, as one that held him when first seen.
pub const DEEP_INSIDE_M: f64 = 1.5;
/// A list older than this many frames isn't solid any more (the probe's lag is 3-5).
pub const SOLID_AGE_MAX: u32 = 30;
/// F7: the camera is pulled in when a building crosses the line of sight, under this distance out (m).
pub const CAM_PULL_UNDER_M: f64 = 40.0;
/// The line of sight kept this far off a wall (m), the camera at least `CAM_PULL_MIN_M` from him, and let back out with this
/// time constant (s) (pulled in at once).
pub const CAM_PULL_CLEAR_M: f64 = 0.3;
pub const CAM_PULL_MIN_M: f64 = 1.0;
pub const CAM_PULL_OUT_TAU_S: f64 = 0.35;

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

// ---- Ground ----
/// His middle (the capsule's) stays this far above the ground (m). Standing his soles reach the ground: his drawn root
/// goes by the clip's own `ground_z` (`ground.rs` `drawn_lift`).
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

// ---- Impacts (step 8, F11) ----
/// A punch's layer comes in and goes out over these (s); its hit, when the clip has no `hit` event, at this share of it.
pub const PUNCH_FADE_IN_S: f32 = 0.08;
pub const PUNCH_FADE_OUT_S: f32 = 0.2;
pub const PUNCH_HIT_SHARE: f32 = 0.45;
/// A new punch can start once the last one's hit is this long gone (s): a quick second click chains the other arm.
pub const PUNCH_CHAIN_S: f32 = 0.12;
/// The punch's ray starts this far (m) up from his middle, at his shoulders.
pub const PUNCH_FROM_UP_M: f64 = 0.45;
/// What a punch into the ground throws up (m) and shakes.
pub const PUNCH_DUST_M: f64 = 0.9;
/// The full-body actions' layer (the ground punch, the thunderclap) comes in and goes out over these (s).
pub const ACTION_FADE_IN_S: f32 = 0.12;
pub const ACTION_FADE_OUT_S: f32 = 0.35;
/// The ground punch's crater lies this far (m) ahead of his feet, where the fist lands.
pub const SMASH_AHEAD_M: f64 = 0.7;
/// A thunderclap's ring lives this long (s), from the least charge to the most, and its light.
pub const CLAP_RING_LIFE_S: (f64, f64) = (1.0, 2.2);
pub const CLAP_RING_STRENGTH: (f64, f64) = (0.25, 0.5);
/// The ring along the ground under him takes this share of that: a white line on the ground reads as paint.
pub const CLAP_GROUND_RING: f64 = 0.4;
/// Its ring starts at his hands, this far (m) ahead of his chest, and grows from this radius (m).
pub const CLAP_AHEAD_M: f64 = 0.6;
pub const CLAP_RING_START_M: f64 = 1.0;
/// It goes ahead of him as it grows, this many times its radius: from a chase camera a ring round his hands is round the camera at once.
pub const CLAP_TRAVEL: f64 = 2.0;
/// It kicks up dust from the ground under him while that's within this share of the ring's radius.
pub const CLAP_GROUND_SHARE: f64 = 0.5;
/// The dust it kicks up: this share of the ring's radius across.
pub const CLAP_DUST_SHARE: f64 = 0.16;
/// Craters are freed once he and the camera are both this far (m) away: the buildings' far range plus 500 m, whatever the Cities
/// setting (D3, F11). No fade before.
pub const CRATER_FREE_M: f64 = crate::scatter::FAR_RANGE_M + 500.0;
/// The rim's model is drawn within this distance (m) of the camera, half of its height under the ground (its across scaled to the
/// crater, its height by the square root of that).
pub const RIM_DRAW_M: f64 = 2000.0;
pub const RIM_SINK: f64 = 0.5;
/// The rim's rock, its texture times this: a little darker and warmer, the ground thrown up.
pub const RIM_TINT: [f32; 3] = [0.62, 0.56, 0.5];
/// Frames the last rim stays out as a speck at the camera after first enable, so its pipeline is built before the first crater.
pub const RIM_WARM_FRAMES: u32 = 600;
/// The rim's model across (its own 4.8 m) is the crater's diameter times this: the bowl's wall and lip show round it.
pub const RIM_ACROSS_SHARE: f64 = 0.8;
/// The crater's decal leaves the sea alone: what stands less than this over it (m) is water. Small: the decal works the height out
/// to a millimetre or so (the water is drawn on the ellipsoid itself), and Earth's flat coasts lie a few centimetres over the sea (a
/// Florida beach 3.6 cm: at 0.1 m its craters went). (His body is left alone by capsules round his bones: `impact.rs` `BODY_PARTS`.)
pub const CRATER_SEA_M: f32 = 0.01;
/// A new crater whose middle lands within this share of the larger radius of one already there (ground punches where he stands)
/// deepens that one instead: each crater multiplies the scene, so two in one place would darken it twice.
pub const CRATER_MERGE: f64 = 0.75;
/// A crater comes in over this (s): the blast.
pub const CRATER_IN_S: f64 = 0.08;
/// The crater's look (`hero_fx.wesl` `decal_fragment`): the bowl's depth in radii (the slope of its walls), how dark the burnt
/// ground is at its middle and at its edge, the raised lip's height in radii, and how far out (radii) the thrown-out ground goes.
pub const CRATER_DEPTH: f32 = 0.32;
pub const CRATER_DARK: (f32, f32) = (0.42, 0.72);
pub const CRATER_LIP: f32 = 0.06;
pub const CRATER_REACH: f32 = 1.9;
/// Dust: a burst's quads (at least, and per metre of its size), its life in air (s) by size (at 1 m and 60 m), and the longest a
/// burst lasts in vacuum, where it falls back as it was thrown.
pub const DUST_QUADS: (u32, f64) = (16, 2.0);
pub const DUST_LIFE_S: (f64, f64) = (3.0, 14.0);
pub const DUST_VACUUM_MAX_S: f64 = 30.0;
/// A burst's colour (its albedo): land, an airless body's, and the sea's spray.
pub const DUST_LAND: [f32; 3] = [0.40, 0.35, 0.28];
pub const DUST_AIRLESS: [f32; 3] = [0.30, 0.29, 0.28];
pub const DUST_SPRAY: [f32; 3] = [0.85, 0.88, 0.90];
/// The splash's size (m) by his speed into the sea: `clamp(SPLASH_K sqrt(v), from, to)`.
pub const SPLASH_K: f64 = 0.6;
pub const SPLASH_M: (f64, f64) = (0.6, 60.0);
/// A hard landing's dust is this many times the crater's radius.
pub const DUST_OF_CRATER: f64 = 1.0;
/// The shake (F7): rotation only, at most this (degrees), from an impact of size `s` (m) at distance `d` (m): `SHAKE_K s / (d + s)`
/// of the most, dying with this time constant (s), at about this many wobbles a second.
pub const SHAKE_MAX_DEG: f64 = 1.0;
pub const SHAKE_K: f64 = 1.6;
pub const SHAKE_TAU_S: f64 = 0.35;
pub const SHAKE_HZ: f64 = 13.0;

// ---- Slopes (step 5) ----
pub const SLOPE_WALK_DEG: f64 = 50.0;
pub const SLOPE_SLIDE_DEG: f64 = 70.0;

// ---- His shadow (step 5) ----
/// The plane it falls on comes from a triangle of ground samples this far apart (m), lifted this far (m).
pub const SHADOW_SPAN_M: f64 = 2.0;
pub const SHADOW_LIFT_M: f64 = 0.03;
/// It fades out as he climbs from and to this height (m over the ground).
pub const SHADOW_FADE_M: (f64, f64) = (10.0, 200.0);
/// His height, for the ray that says where the shadow ends (m).
pub const SHADOW_TALL_M: f64 = 1.9;
/// The ray's steps down to the ground: this share of what the height says is left, at least this long (m), at most this many, then
/// this many halvings. A star lower than this (the sine of its height) counts as this low.
pub const SHADOW_RAY_SHARE: f64 = 0.7;
pub const SHADOW_RAY_MIN_STEP_M: f64 = 0.25;
pub const SHADOW_RAY_STEPS: usize = 16;
pub const SHADOW_RAY_BISECT: usize = 6;
pub const SHADOW_RAY_MIN_COS: f64 = 0.05;
/// How far along the ground the shadow's line goes at most (m): a shadow this long has next to no star left to take away.
pub const SHADOW_REACH_M: f64 = 40.0;
/// The sine of the star's height where that reach goes to nothing and comes in full. (The shader draws no shadow under the first.)
pub const SHADOW_LOW_SUN: (f64, f64) = (0.02, 0.05);
/// A line shorter than the first of these (m) says nothing about the slope, longer than the second all of it along the line.
pub const SHADOW_LINE_M: (f64, f64) = (0.3, 1.0);
/// How fast the plane's turn follows the ground (s).
pub const SHADOW_EASE_S: f64 = 0.08;

// ---- His cape (step 5) ----
/// The fixed time step (s), and at most this many steps a frame: a hitch runs less time, it never runs a long step.
pub const CAPE_STEP_S: f32 = 1.0 / 120.0;
pub const CAPE_MAX_STEPS: usize = 4;
/// Solver passes over its lengths a step.
pub const CAPE_SOLVE: usize = 4;
/// The air's speed past him for the cloth stops counting at this (m/s): above it the cape just streams back.
pub const CAPE_WIND_MAX_MS: f32 = 60.0;
/// How hard the air takes the cloth with it (per second).
pub const CAPE_WIND_K: f32 = 3.0;
/// Air this dense or denser counts in full (kg/m3): less and it takes the cloth less. None: inertia only.
pub const CAPE_AIR_FULL: f32 = 0.02;
/// The most his own acceleration or gravity moves the cloth (m/s2): 1000 c is reached in 30 s, a hard stop is one frame.
pub const CAPE_ACCEL_MAX: f32 = 40.0;
/// A point's speed against his body is never more than this (m/s).
pub const CAPE_SPEED_MAX: f32 = 45.0;
/// Velocity kept per step, without air.
pub const CAPE_DAMP: f32 = 0.992;
/// How much the cloth turns with his chest: 0 stays put in space while he turns, 1 is rigid with him.
pub const CAPE_FOLLOW: f32 = 0.7;
/// How hard the cloth goes back to hanging behind him where nothing else moves it (per second squared).
pub const CAPE_REST_K: f32 = 5.0;
/// How much of the outer chains' extra length goes (1: every chain as long as the middle one, a level hem; 0: the diamond it's cut as).
pub const CAPE_HEM_LEVEL: f32 = 0.85;
/// How far the rest shape's links lean to straight down his body: the rest is the link's rest direction.
pub const CAPE_HANG: f32 = 0.85;

// ---- Boom (F9) ----
/// The air counts from this pressure, and stops counting under the second (Pa).
pub const BOOM_AIR_PA: (f64, f64) = (150.0, 100.0);
/// Sub to Super at the first Mach number, Super to Sub at the second.
pub const BOOM_MACH: (f64, f64) = (1.0, 0.92);
/// The least time between two booms (s).
pub const BOOM_COOLDOWN_S: f64 = 3.0;

// ---- Heat (F9) ----
/// The temperature's low-pass, rising and falling (s).
pub const HEAT_RISE_S: f64 = 0.4;
pub const HEAT_FALL_S: f64 = 1.5;
/// The look stops changing here (K): past it only the shell's size and the trail's length grow, up to what `HEAT_PAST_MAX` says.
pub const HEAT_LOOK_K: f64 = 10_000.0;
/// How far past the clamp the size and the length go: as (T / `HEAT_LOOK_K`) to this power, at most this much.
pub const HEAT_PAST_POWER: f64 = 0.4;
pub const HEAT_PAST_MAX: f64 = 1.5;
/// F9's stages are named by how they read, and the black body's colours in linear light are redder than that: the colour is raised to a
/// power, this one up to the first temperature (dull orange), then to this one at the second (yellow), then to 1 from the third on (K).
pub const HEAT_COLOUR_GAMMA: (f64, f64) = (0.42, 0.6);
pub const HEAT_COLOUR_K: (f64, f64, f64) = (800.0, 2000.0, 6000.0);
/// SE's black body has no green at all under about 1,300 K (out of sRGB's gamut, clipped): dull orange needs at least this much.
pub const HEAT_COLOUR_LEAST_GREEN: f64 = 0.22;
/// The glow on his body: nothing under the first temperature, the HDR limits' lowest at the second (K), the highest at `HEAT_LOOK_K`,
/// exponentially between: never T^4.
pub const GLOW_FROM_K: (f64, f64) = (600.0, 800.0);
pub const GLOW_HDR: (f64, f64) = (0.3, 12.0);
/// His lee side, what the chase camera sees: its colour is the windward side's at this share of the temperature, its light this share.
pub const GLOW_LEE_T: f64 = 0.8;
pub const GLOW_LEE: f64 = 0.35;
/// At most this much, or from behind he's a white shape with no suit left.
pub const GLOW_LEE_MAX: f64 = 1.6;
/// The readout's bar: empty at the first temperature, full at `HEAT_LOOK_K`, by the logarithm.
pub const HEAT_BAR_FROM_K: f64 = 400.0;

// ---- Heat's effects (step 7, F9; hero_fx.wesl has the same HDR limits) ----
/// The shell and the trail are hot gas going past, so they go by the air's own temperature, no higher than his heat, and fall this fast
/// when it does (s): his skin cools slowly (`HEAT_FALL_S`), a shell standing round him at a stop would make no sense.
pub const GAS_FALL_S: f64 = 0.2;
/// The plasma shell fades in between these (K), and from its first brightness at the second to its last at `HEAT_LOOK_K`.
pub const SHELL_FROM_K: (f64, f64) = (3000.0, 3500.0);
pub const SHELL_HDR: (f64, f64) = (0.8, 3.0);
/// Its radius, length and how far its nose is ahead of his middle (m), at `SHELL_FROM_K.1` and at `HEAT_LOOK_K`.
pub const SHELL_RADIUS_M: (f64, f64) = (1.0, 1.7);
pub const SHELL_LENGTH_M: (f64, f64) = (2.8, 4.2);
pub const SHELL_NOSE_M: (f64, f64) = (1.2, 1.5);
/// However big, its radius and nose are at most this share of the camera's distance: a shell round the camera would be a flash.
pub const SHELL_CAMERA_SHARE: f64 = 0.7;
/// How much of the colour is white: it isn't a black body.
pub const SHELL_WHITE: f64 = 0.3;
/// The trail fades in between these temperatures (K), a point of it from this share of its light at the second to all of it at `HEAT_LOOK_K`.
pub const TRAIL_FROM_K: (f64, f64) = (5000.0, 6000.0);
pub const TRAIL_HDR: (f64, f64) = (1.5, 5.0);
/// A point's life (s), at `TRAIL_FROM_K.1` and at `HEAT_LOOK_K`, and past that by `HEAT_PAST_*` squared.
pub const TRAIL_LIFE_S: (f64, f64) = (0.8, 2.0);
/// Points are at least this far apart (m); fast, a point a frame.
pub const TRAIL_SPACING_M: f64 = 4.0;
/// Half its width at his place (m), how it widens: this times the square root of the metres behind him, as a wake does, and this much
/// a second of a point's age; and the least half-width per metre of distance from the camera (radians): far away it must still be seen.
pub const TRAIL_WIDTH_M: f64 = 0.15;
pub const TRAIL_WAKE: f64 = 0.14;
pub const TRAIL_WIDEN_MS: f64 = 1.5;
pub const TRAIL_WIDEN_RAD: f64 = 0.0015;
/// Its light comes in over this far behind him (m).
pub const TRAIL_BEGINS_M: f64 = 2.0;
/// Its colour is the black body's at this many times his temperature: ionised gas, blue-white.
pub const TRAIL_COLOUR_T: f64 = 1.6;

// ---- The sonic cone and the ring (step 7, F9) ----
/// The cone comes in between the first two Mach numbers and goes between the last two.
pub const CONE_MACH: (f64, f64, f64, f64) = (0.95, 1.0, 1.3, 1.6);
/// The length of its side (m) and how far its tip is ahead of his middle (m).
pub const CONE_SLANT_M: f64 = 3.2;
pub const CONE_NOSE_M: f64 = 0.45;
/// How much light it scatters at its strongest: 1 is a white surface facing the star.
pub const CONE_STRENGTH: f64 = 0.35;
/// Super to Sub: a pulse of it, this strong, dying with this time constant (s).
pub const CONE_PULSE: f64 = 0.5;
pub const CONE_PULSE_TAU_S: f64 = 0.25;
/// Thin air shows them less: from the first to the second share as `log10 q` goes through `BOOM_LOG_Q`.
pub const AIR_SHOW: (f64, f64) = (0.35, 1.0);
/// A ring rides with him this long (the camera would pass it in 15 ms otherwise), then stays where it is on the body.
pub const RING_RIDE_S: f64 = 0.25;
/// Its life (s), and its radius: at the start, and growing by this much a second, slowing to nothing over its life.
pub const RING_LIFE_S: f64 = 1.6;
pub const RING_START_M: f64 = 1.5;
pub const RING_GROW_MS: f64 = 32.0;
/// How far the band reaches either side (m), at the start and per metre of radius.
pub const RING_BAND_M: (f64, f64) = (0.25, 0.05);
/// How much light it scatters (1: a white surface facing the star), and its own, faint, for the night.
pub const RING_STRENGTH: f64 = 1.0;
pub const RING_OWN: f64 = 0.015;

// ---- His sounds (step 6, F10) ----
/// A voice's fade, turning off, losing the focus and coming back (s): a loop can cross its whole range in this.
pub const SOUND_FADE_S: f32 = 0.2;
/// A volume this small is silence: the sink pauses, and a smaller change isn't sent.
pub const SOUND_SILENT: f32 = 1e-4;
/// `fastFlight_loop` is the air going past at every speed: its volume goes 0 to 1 as `log10 q` (q in Pa) goes from the first to the second.
pub const FAST_LOG_Q: (f64, f64) = (1.0, 6.0);
/// Its pitch: from the first to the second as Mach goes (logarithmically) between these. Low at low speed, so it reads as wind.
pub const FAST_PITCH: (f32, f32) = (0.6, 1.3);
pub const FAST_PITCH_MACH: (f64, f64) = (0.05, 20.0);
/// How fast a pitch follows (s).
pub const PITCH_TAU_S: f32 = 0.15;
/// `fasterFlight_loop`'s volume by the temperature (K).
pub const FASTER_HEAT_K: (f64, f64) = (2500.0, 6000.0);
/// The hum (`sentry_flightfx_loop`): this loud while he hovers or flies, and out of the air whatever he's doing (D12: there it goes
/// over `SHOT_AIR_PA`), fading as `log10 q` goes between these, at this pitch.
pub const HUM_VOLUME: f32 = 0.12;
pub const HUM_LOG_Q: (f64, f64) = (0.0, 2.0);
pub const HUM_PITCH: f32 = 0.6;
/// At most this many one-shots at a time: a new one takes the place of the one that's played longest.
pub const SHOTS_MAX: usize = 8;
/// A one-shot's share of its volume in vacuum (D12), up to all of it between these pressures (Pa, logarithmically): what the sounds
/// call vacuum and air.
pub const SHOT_VACUUM: f32 = 0.25;
pub const SHOT_AIR_PA: (f64, f64) = (1.0, 100.0);
/// The least time between two of the same one-shot (s): the long whooshes more, or a tapped key piles them up.
pub const SHOT_GAP_S: f64 = 0.1;
pub const WHOOSH_GAP_S: f64 = 0.75;
/// A one-shot's longest life (s) where nothing can say it's over (no audio device): over the longest file at the lowest pitch.
pub const SHOT_MAX_S: f32 = 9.0;
/// `explode_ground1` on the sea: this pitch (the mod has no splash; judge by ear), this loud from the first to the second speed (m/s).
pub const SPLASH_PITCH: f32 = 0.6;
pub const SPLASH_GAIN: (f32, f32) = (0.15, 1.0);
pub const SPLASH_SPEED_MS: (f64, f64) = (2.0, 35.0);
/// The boom's loudness (step 7), from the first to the second as `log10 q` goes between these: 1.5 is the top of Earth's boom, 4.8 sea level.
pub const BOOM_GAIN: (f32, f32) = (0.6, 1.0);
pub const BOOM_LOG_Q: (f64, f64) = (1.5, 4.8);

// ---- His light (step 3) ----
/// Deep in a giant's decks: how much of his light gets down to him, as the decks' own brightness goes (measured on Jupiter's day side, 5
/// to -100 km). Counted from this many of the decks' scale heights over the radius: `DECK_NEAR.0 exp(-d / DECK_NEAR.1) + DECK_FAR.0
/// exp(-d / DECK_FAR.1)`, d in scale heights. Then none, from the deep deck's bottom on, as `giant_dark` in the clouds' shader.
pub const DECK_TOP_H: f64 = 0.25;
pub const DECK_NEAR: (f64, f64) = (0.85, 0.9);
pub const DECK_FAR: (f64, f64) = (0.15, 3.0);
pub const DECK_DARK_H: (f64, f64) = (-6.0, -8.0);
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
/// The same under a giant's decks, where no light gets down and the view is black: nothing can be seen, so he must be.
pub const FAINT_FILL_DEEP: f32 = 0.05;
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

// ---- The controls as in the user's own Viltrumite mod (the user, 2026-11-07; `controls.rs`) ----
/// Space let go sooner than this is a tap: take off on the ground, drop in a hover (s).
pub const TAP_S: f64 = 0.22;
/// A launch's charge fills in this long (s); a boost stack's, past the barrier, in `STACK_CHARGE_S`.
pub const LAUNCH_CHARGE_S: f64 = 1.4;
pub const STACK_CHARGE_S: f64 = 0.7;
/// A charge let go under this share does nothing (a tap that ran on).
pub const CHARGE_LEAST: f64 = 0.08;
/// Two presses of W (or S) within this long are a double tap (s).
pub const DOUBLE_TAP_S: f64 = 0.3;
/// Fast flight's least speed (m/s): under 0.9 of it (S braking), he's back in a hover.
pub const FAST_MIN_MS: f64 = 60.0;
/// A full launch reaches this many times the local speed of sound (out of the air `SOUND_REF_MS`'s): past the barrier, a boom.
pub const LAUNCH_TOP_MACH: f64 = 1.3;
pub const SOUND_REF_MS: f64 = 343.0;
/// Double W into fast flight at least this share over `FAST_MIN_MS`.
pub const FAST_ENTRY_SHARE: f64 = 1.35;
/// Fast flight turns towards the look at this rate (1/s); past the barrier `STEER_SUPER` x (1 + `STEER_STACK` x stacks).
pub const STEER_FAST: f64 = 3.2;
pub const STEER_SUPER: f64 = 3.8;
pub const STEER_STACK: f64 = 0.6;
/// S in fast flight halves the speed every this long (s).
pub const S_BRAKE_HALF_S: f64 = 0.35;
/// Boost stacks (a full charge past the barrier): x this each, at most this many, each with a boom.
pub const STACK_FACTOR: f64 = 1.5;
pub const STACKS_MAX: u32 = 5;
/// The dive (Ctrl in fast flight): straight down, turning at this rate (1/s) and gaining this (m/s2).
pub const DIVE_TURN: f64 = 6.0;
pub const DIVE_ACCEL: f64 = 30.0;
/// Ctrl let go in a dive: a hover with this share of his speed; double S out of fast flight: this one.
pub const DIVE_PULL_UP_SHARE: f64 = 0.25;
pub const FAST_TO_HOVER_SHARE: f64 = 0.35;
/// A tap of Space on the ground takes him up to this height (m) over the first `TAKE_OFF_RISE_S` of the hover.
pub const TAKE_OFF_HEIGHT_M: f64 = 3.0;
pub const TAKE_OFF_RISE_S: f64 = 0.8;
/// A ground launch goes at least this far over the level (degrees).
pub const LAUNCH_MIN_UP_DEG: f64 = 15.0;
/// The hover holds still while Space charges: its speed goes as exp(-this x t).
pub const CHARGE_HOLD_RATE: f64 = 5.0;
/// The ground charge's crouch: `charge_flight_stopped` held at this time into it (s; its launch is at 0.5).
pub const GROUND_CHARGE_HOLD_S: f32 = 0.42;
/// Super run (Shift held, W twice, on the ground): this speed (m/s), with a boom at its start.
pub const SUPER_RUN_MS: f64 = 80.0;
/// The camera's kick: a boom 1, a launch from the first to the second by its charge, a stack's boom `KICK_STACK`, a charged punch
/// up to `KICK_PUNCH`. It shakes by 0.8 of it and widens the view by `FOV_KICK_DEG` at 1.
pub const KICK_BOOM: f64 = 1.0;
pub const KICK_LAUNCH: (f64, f64) = (0.35, 0.95);
pub const KICK_STACK: f64 = 1.0;
pub const KICK_PUNCH: f64 = 0.7;
/// The charge's tremble, from the first to the second as it fills (the ground's; the hover's is half).
pub const CHARGE_TREMBLE: (f64, f64) = (0.15, 0.9);
/// The shake dies at this much a second, the kick at this (units of 1).
pub const SHAKE_DECAY: f64 = 1.8;
pub const KICK_DECAY: f64 = 0.9;
/// His shake turns the view up to this at 1 (degrees; the user wanted a shake when he breaks the barrier: more than F7's 1 degree).
pub const HERO_SHAKE_DEG: f64 = 1.5;
/// The view widens by `FOV_KICK_DEG` at a kick of 1, by `FOV_SPEED_DEG` at fast flight's top, by `FOV_STACK_DEG` a stack; eased.
pub const FOV_KICK_DEG: f64 = 14.0;
pub const FOV_SPEED_DEG: f64 = 12.0;
pub const FOV_STACK_DEG: f64 = 3.0;
pub const FOV_EASE: f64 = 4.0;
/// The left button: held past `PUNCH_WIND_S` it winds up (`charge_attack`); let go past `PUNCH_CHARGE_S` a charged punch, full at
/// `PUNCH_FULL_S`; clicks within `COMBO_S` chain the combo.
pub const PUNCH_WIND_S: f64 = 0.15;
pub const PUNCH_CHARGE_S: f64 = 0.55;
pub const PUNCH_FULL_S: f64 = 2.2;
pub const COMBO_S: f64 = 0.6;
/// A charged punch's blast: a crater from the first to the second radius by the charge (m) where it meets the ground, dust in the air.
pub const CHARGED_CRATER_M: (f64, f64) = (2.5, 10.0);
/// Every sound of his this much louder (the user, 2026-11-07: "the sound effect is to quiet").
pub const SOUND_GAIN: f32 = 2.0;
/// The roar past the barrier (`fasterFlight_loop`), as the user's mod plays it: from the first to the second by the speed, more by
/// `ROAR_STACK` a stack, pitched up by the same; fast flight under the barrier `fastFlight_loop` from the first to the second.
pub const ROAR_LEVEL: (f32, f32) = (0.6, 1.0);
pub const ROAR_STACK: f32 = 0.08;
pub const FAST_LOOP_LEVEL: (f32, f32) = (0.35, 0.85);
