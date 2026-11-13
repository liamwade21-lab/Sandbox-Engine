# skate-sim (from the Skate 3 Rust Engine's `skate-game`)

The per-tick skater simulation of `skate-game`, extracted into a library with a small facade (`Sim`, in `src/sim.rs`).

- Upstream: SK8-ENGINE/skate-3-rust-engine, commit `2e16697797c8358d16f747a8259354ff2921041a` (2026-10-09), `crates/skate-game/src`.
  Checked out read-only at `re/skate3/src`.
- Licence: GPL-3.0 (the repository's `LICENSE`, copied to this project's root).
- Credit: the Skate 3 Rust Engine's authors, building on dumbad's years of Skate 3 reverse engineering and Chasm's work.
- Companion crates in this workspace (own `UPSTREAM.md` files): `skate-core`, `skate-data` (copied unchanged earlier).
  Added with this crate: `skate-dynamics` and `skate-net` (copied unchanged, whole crates; `physics/network.rs` and
  `physics/solid_contacts.rs` name their types), and `skate-mods` (TRIMMED: only `presentation.rs`, `extensions.rs`,
  `lua_list.rs`, `schema.rs`, copied unchanged, plus a new 6-line `lib.rs`; the Lua host, audio and archive code are not
  included, so no mlua, gltf, zip or blake3 is built).

## Behaviour

`Sim::step` is upstream's FixedUpdate chain without the Bevy schedule: `ControllerInput::collect` + `publish_actions`
(`input::publish_actions`), `PlayerControls::update_for_physics` + `publish_gestures` (`physics::controls::sample`),
`physics::frame::advance` (`physics::advance`), then the presentation snapshot (`presentation::capture`, re-implemented
in `sim.rs` because upstream's is a Bevy system over resources). With no mods loaded the `modding` hooks behave as upstream
does without mods (stand-in `modding.rs`; `modding/player_physics.rs` is the upstream file).

Checked: the copied crate compiled against Bevy 0.18.1 (glam 0.30.10, upstream's version) and against Bevy 0.20.0
(glam 0.33.12) gives bit-identical output for the facade test's push/coast/ollie run (FNV digest of every f32 of the
board, all bones, the root and the camera each tick), and the same 17 ignored upstream tests fail in both (their
causes are listed under "Upstream tests" below).

## Facade notes (where `Sim` deliberately differs from upstream's Bevy app)

- `set_world` builds triangles like `skate_world::collision_world`'s portable-map path, but first groups the host's triangles
  by 16 m cell (a stable sort) so the 64-triangle query ranges stay small; `set_world_in_order` keeps the host's order.
  Host triangles get surface 0 (as the test course) and the physics settings' floor contact material; degenerate ones are skipped.
- `Sim::load` builds upstream's test-course `GamePhysics` (`load_with_difficulty(root, None, ..)`), then replaces the course world
  with a flat host floor and the course's rails with an empty grind provider BEFORE `SkaterRuntime::load` (its trajectory binds the
  grind provider), applies the empty customiser profile (`customiser::preferences` does this every frame upstream) and queues a spawn
  at the origin. The default camera angle is High and the aspect 16:9 (`set_camera_high`, `set_aspect_ratio`).
- `Events`: upstream emits only `PhysicsEvent::StateChanged`; `landed` / `wiped_out` / `popped` are derived from those state
  changes, `pushed` / `brake_down` / `brake_up` are edges of the animation's audio latches (`observe` edge-detects them too).
- A tick that fails (`Err`) leaves the sim failed for good, as upstream's `physics.failed`.

## Copied from `crates/skate-game/src` (paths relative to it, copied into `crates/skate-sim/src`)

Whole trees: `physics/`, `graph_host/`, `input/`, `scoring_runtime/`, `animation_pose/`, `skater_animation/`, `grind_world/`, `camera/`.
Files: `physics.rs`, `graph_runtime.rs`, `input.rs`, `scoring_runtime.rs`, `animation_pose.rs`, `skater_animation.rs`,
`grind_world.rs`, `camera.rs`, `difficulty.rs`, `custom_difficulty.rs`, `presentation.rs`, `animation.rs`, `apt_text.rs`,
`modding/player_physics.rs`.
Test files pulled in by `#[path]` from `tests/`: `air_playback`, `animation_pose`, `audio_state_capture`, `bail_geometry`,
`difficulty`, `gameplay_gestures`, `graph_runtime`, `grind_bail`, `hippy_playback`, `manual_playback`, `map_startup`,
`offboard_air_playback`, `offboard_jump_playback`, `offboard_midair_playback`, `offboard_recall_playback`,
`offboard_root_trace`, `physics_startup`, `powerslide_playback`, `recorded_playback`, `trigger_points`, `water_drop`,
`wipeout_playback` (every `tests/*.rs` that a copied file names; the ones disabled below are still copied).

NOT copied (outside the closure): `app`, `assets`, `config`, `world`, `skate_world`, `map_*`, `replay`, `performance`, `modding`
(but `player_physics.rs`), `multiplayer`, `game_audio`, `scoring_hud`, `hud_runtime`, `apt_vm`/`apt_movie`/..., `retail_render`,
`customiser*`, `graphics_menu`, `debug_cam`, `trigger_volumes`, `session_marker`, `frame_timing`, `verification`, `updater`, ...

## New files (not upstream)

`Cargo.toml`, `src/lib.rs`, `src/sim.rs` (the facade; its `prepare_world`/`install_world` build `set_world`'s collision off
the host's main thread), `src/tests/oracle_trace.rs` (the parity test: the same file runs in `re/skate3/oracle`, upstream with
SDL3 cut out, and the two traces must be identical; included from `physics.rs`), `src/sim_bridge.rs` (a child module of `physics`, reached by the one-line
declaration in `physics.rs` below: it sees the private items the facade needs; no arithmetic), `src/facade_tests.rs`, and the
stand-ins, each at the path the copied code names, with no-op / empty behaviour: `src/app.rs` (the two `SystemSet` enums, same
definitions as upstream's), `src/config.rs`, `src/crash_context.rs`, `src/customiser.rs` (holds upstream's `apply_preferences` and
`scalar`, copied unchanged), `src/modding.rs` (`Mods`, `player_attached`, `player_suspended`, `override_actions`: no mod, so
false / no-op), `src/performance.rs`, `src/replay.rs` (never active), `src/scoring_hud.rs` (only `localize_trick`),
`src/skate_world.rs` (`collision_world` returns an error: no map files), `src/world.rs` (`PlayerRoot`).

## Files edited after copying (everything else is byte-identical)

None of the edits touch a number, constant, iteration order or float operation.

| file | change |
| --- | --- |
| `physics.rs` | 3 test modules (`map_startup`, `audio_state_capture`, `trigger_points`) get `#[cfg(any())]` (they need the map loader, `game_audio`, `trigger_volumes`); the 4 lines declaring `sim_bridge`; the 3 lines including `tests/oracle_trace.rs` (test only). |
| `physics/foot_ik_queries.rs` | test module `mod_platform_tests` gets `#[cfg(any())]` (needs `modding::bridge`, the Lua/Rapier bridge). |
| `physics/offboard/contact_toolkit/tests.rs` | one test (`embedded_static_rwcm_hits_distinct_actor_query_ids`) gets `#[cfg(any())]` (needs `maps/format-demo.skate` and the map loader). |
| `input.rs` | test module `action_docs` gets `#[cfg(any())]` (reads upstream's `skate-mods/src/api.lua`); everything from `pub(crate) struct InputPlugin` to the end is wrapped in `#[cfg(any())] mod plugin_shell { ... }` (the Bevy plugin and its window/menu/mod systems). |
| `input/controllers.rs` | `ControllerInput::tick_input`, `collect`, `publish_actions` widened from `pub(super)` to `pub(crate)` (the facade calls them as `sample_raw_for_test` does). |
| `input/controller_kind.rs` | `family_from_sdl`, `SdlReport`, `from_sdl` and the test module (all need `sdl3` types) get `#[cfg(any())]`. |
| `input/platform.rs` | REPLACED by a stub: lines 1-107 (types, button names, trigger/y conversions, `CapabilityCache`) and the `conversion_tests` / `cache_tests` modules are upstream's; the XInput and SDL3 backends are removed, `poll` / `poll_cached` report `Disconnected`. |
| `scoring_runtime.rs` | the `use` of `apt_vm::Value` / `hud_runtime` and `Runtime::hud_input` (builds the APT movie input) get `#[cfg(any())]`. |
| `scoring_runtime/tests.rs` | test `landed_multiplier_reaches_hud_across_repeated_tricks` gets `#[cfg(any())]` (needs the APT HUD runtime). |
| `camera.rs` | the `use crate::{app::FrameSet, config::Config}` line and everything from `pub(crate) struct CameraPlugin` to the end (Bevy camera/window/render systems) wrapped in `#[cfg(any())] mod rendering_shell { ... }`. |
| `animation.rs` | the two Bevy render `use` lines, `AnimationStatus` ... `present` (skin binding), and the two test modules wrapped in `#[cfg(any())] mod ... { }`; `use bevy::prelude::*;` added. `render_basis` and `native_matrix` are unchanged. |
| `modding/player_physics.rs` | `snapshot` (the Lua-bridge debug snapshot, needs the mod manager) gets `#[cfg(any())]`. |
| `skate-mods` | trimmed to four unchanged files + a new `lib.rs`; `skate-dynamics`, `skate-net`: unchanged. |

Math types: the copied code uses Bevy's glam types in `physics/climbing/*`, `physics/network.rs`, `physics.rs` (map spawn),
`presentation.rs` and `camera/` helpers; they were kept. Bevy 0.20 uses glam 0.33.12, upstream's Bevy 0.18.1 glam 0.30.10; the
output check above found no difference for the exercised path (climbing and networking were not exercised).

## Re-checking

Run from the project root (`C:\Users\liamw\Desktop\Sandbox Engine`):

```sh
U=re/skate3/src/crates/skate-game/src; D=crates/skate-sim/src
# every copied tree: only the files in the table above may differ
for x in physics physics.rs graph_host graph_runtime.rs input input.rs scoring_runtime scoring_runtime.rs animation_pose \
         animation_pose.rs skater_animation skater_animation.rs grind_world grind_world.rs camera camera.rs difficulty.rs \
         custom_difficulty.rs presentation.rs animation.rs apt_text.rs; do diff -rq $U/$x $D/$x; done
diff -q $U/modding/player_physics.rs $D/modding/player_physics.rs
for f in $D/tests/*.rs; do diff -q $U/tests/$(basename $f) $f; done          # only oracle_trace.rs (ours) is missing upstream
# each edit, as inserted lines only (physics.rs, camera.rs, input.rs, scoring_runtime*, animation.rs, ...)
diff $U/physics.rs $D/physics.rs; diff $U/camera.rs $D/camera.rs; diff $U/animation.rs $D/animation.rs
diff $U/input.rs $D/input.rs; diff $U/input/controllers.rs $D/input/controllers.rs        # 3 visibility lines
diff $U/input/controller_kind.rs $D/input/controller_kind.rs; diff $U/scoring_runtime.rs $D/scoring_runtime.rs
diff $U/scoring_runtime/tests.rs $D/scoring_runtime/tests.rs; diff $U/physics/foot_ik_queries.rs $D/physics/foot_ik_queries.rs
diff $U/physics/offboard/contact_toolkit/tests.rs $D/physics/offboard/contact_toolkit/tests.rs
diff $U/modding/player_physics.rs $D/modding/player_physics.rs
# the other crates
for c in skate-dynamics skate-net; do diff -rq re/skate3/src/crates/$c crates/$c; done
for f in presentation extensions lua_list schema; do diff -q re/skate3/src/crates/skate-mods/src/$f.rs crates/skate-mods/src/$f.rs; done
```

Expected: `diff -rq` lists exactly `physics.rs`, `physics/foot_ik_queries.rs`, `physics/offboard/contact_toolkit/tests.rs`,
`input.rs`, `input/controller_kind.rs`, `input/controllers.rs`, `input/platform.rs`, `scoring_runtime.rs`,
`scoring_runtime/tests.rs`, `camera.rs`, `animation.rs`; every `diff` of those shows only `>` lines (inserted) except
`controllers.rs` (3 `pub(super)` -> `pub(crate)`) and `platform.rs` (replaced).

Tests: `cargo test --release -p skate-sim`; with retail data
`SKATE3_ASSET_ROOT=<folder holding private/> cargo test --release -p skate-sim -- --include-ignored`.

## Upstream tests (with `SKATE3_ASSET_ROOT` = the installed assets)

284 pass, 17 fail (301 tests incl. the facade ones; 209 pass and 92 are ignored without the data), with the same 17 under Bevy 0.18.1 and 0.20.0. That rules out the Bevy/glam change; it does not prove the
17 fail upstream too (no upstream build was run). Their causes:

- Need data this install lacks or other env vars: `recorded_controller_sequence_runs_production_gameplay`
  (`SKATE3_INPUT_RECORDING`), `water_drop_trace` (`SKATE3_WATER_MAP`), `grind_bail_trace`,
  `library_handrail_bend_grinds_through` (`SKATE3_GRIND_MAP`), `stock_motion_factory_support_inventory`
  (`SKATE3_MOTION_INVENTORY`), 3 `animation_pose::authored_clips` tests (read `<crate>/../../assets/private/...`, upstream's layout).
- Replace `physics.world` with a metadata-less `BoardWorld::new(..)` and so stop with "Canonical world has no authored query
  metadata" (fixture bug in the test, independent of Bevy): the three `air_tests` roll-off tests and
  `stock_skater_startup_builds_the_graphs_and_physical_body_from_the_same_rig`.
- 3 `climbing` tests: `.unwrap()` on the climbing clips, which load from `private/custom/climbing.json` (absent here).
- `powerslide_playback`: the stock slide latch never started (not investigated).
- `marker_reply_restores_on_foot`: asserts `PhysicsGround` one tick after the teleport, gets `PhysicsAir`; for an on-foot reply
  `player_state::enter_after_teleport` enters `BipedGround`, so the assertion looks stale (not investigated).

## Data read at run time (relative to the folder holding `private/`)

Required (the whole of `Sim::load`; found by reading every `fs::read` site and confirmed by removing files one by one and
comparing the run's output): `private/game.json`; `private/skater.glb` (existence check only, never parsed);
`private/stock/skater-collections.json` (9.8 MB); `private/stock/physics-skeletons.json`;
`private/stock/data/anim/OnBoard.abin` (10.8 MB), `OffBoard.abin` (4.0 MB);
`private/stock/data/state/ActionGraph_OnBoard.stategraph`, `MotionGraph_OnBoard.stategraph` (the paths named by `game.json`);
`private/stock/data/script/camera/Default_cameragraph.stategraph`; `private/stock/data/camera/1.shk`, `2.shk`;
`private/stock/data/joystick/{skater,skater90,skaterN90,skater_air,skater_fingerflip,skaterls,skaterstep}.pat`.
`input.cfg` is NOT read (`StockGameplayConfig::load` is only called by upstream's setup); stategraph includes are not read.
Optional, NotFound-tolerated: `private/custom/climbing.json` (custom climbing clips), `private/custom/crouch-treflip.json`,
`<root>/../settings/custom-difficulty.json` (overlay, read by `custom_difficulty::load_collections`),
`private/hud/runtime/trickdisplay.json` (1.6 MB; only its `language` table, for `Sim::hud` trick text).
