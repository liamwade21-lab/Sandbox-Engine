# Bug hunt brief (read this, then your ticket)

Project: a Rust/Bevy 0.20 rebuild of SpaceEngine 0.991 (Vulkan). It reads SE's own data files at run
time from `D:\SteamLibrary\steamapps\common\SpaceEngine` (read only, never copy from it). Read the
repo's `CLAUDE.md` and `PROGRESS.md` first. `main` is V1 (tag `v1.0`).

## Rules
- **Do not edit any file under `crates/`** or anywhere outside `.bugfinder/scratch/`. Scratch files,
  repro crates and screenshots go in `.bugfinder/scratch/<your-ticket-number>/`.
- **Never run SpaceEngine itself.** Running the port is fine, but only through
  `bash .audit/tools/runport.sh OUT.png FRAMES "dev; commands"` (it holds a lock so only one run
  happens at a time; other agents share it, so keep runs few and short). The binary is
  `target/release/spaceengine-rs.exe`; don't rebuild it (other agents use it). Dev commands:
  `select NAME`, `goto NAME DIST_KM`, `surface LAT LON ALT_KM PITCH`, `agl M`, `fly KMS`, `shot FILE`,
  `wait N`, `date`, `pause`. A run's log is `OUT.png.log`. See README.md for more.
- **Repros for `se_core` code**: make a tiny crate in `.bugfinder/scratch/<NN>/repro/` with an empty
  `[workspace]` table and `se_core = { path = "../../../../crates/se_core" }`, and build it with
  `CARGO_TARGET_DIR=.bugfinder/scratch/target`. se_core has only tiny deps, so this is quick.
  Don't build `se_engine` in a scratch crate (Bevy is huge). Existing tests:
  `cargo test -p se_core` is fine; avoid `cargo test -p se_engine --release` unless you really need it
  (slow, and it shares `target/` with the others).
- Keep command output small (`2>&1 | tail -30`). Search before reading; read big files in parts.
- Anything listed in `docs/missing_features.md` is a missing feature, not a bug. Grep it before
  reporting something that is "SE does X, port doesn't".

## What counts
**Bad bugs only:** crashes, panics (index out of bounds, unwrap on bad data, divide by zero → NaN that
spreads, integer overflow in release-wrap that gives garbage), data loss, settings/save corruption,
softlocks/hangs/infinite loops, broken game logic, physics or input errors, state desyncs (CPU vs GPU,
cache vs reality, two systems disagreeing), race conditions between async tasks and frames, leaks
(GPU buffers, entities, caches that never shrink), security holes, and wrong results (wrong position,
wrong time, wrong unit conversion, wrong object). Skip style, naming, performance nits and
"could be cleaner".

Not bugs (user said so): SE screenshots look different because the user's SE has a graphics mod.
"Go to" rotating with planets is on purpose. The info panel's distance wobbling at very high time
rates is harmless.

Already found and fixed, do not report again (but hunt for **more of the same kind**):
- A cache cleanup threw away newly made terrain height grids, so from the second landing on the
  ground stayed blurry or flickered (terrain.rs). → look for other caches/evictions that drop
  things still needed, or keep things forever.
- The near clipping plane was set after the frame's camera was already built, so the ground right in
  front of the camera got cut away (terrain.rs). → look for other system-ordering bugs: values set
  after the system that reads them, one-frame-late state, missing `.before/.after` ordering.
- The terrain generator treated "shader still loading" as a failure and switched to backup heights,
  giving holes and height jumps (terrain_gen.rs). → look for other async/pending states treated as
  failure, or failure treated as success.
- Ground textures blurred into blobs up close and changed colour with distance (planet.wesl). → look
  for other LOD/mip/precision/scale errors.
- Clicks on the top-left info panel went through to the 3D view and selected the star behind it
  (hud.rs). → look for other input that leaks through UI, or keys acting while typing in a text box.
- The star count included barycentres, so the Sun showed about 200 stars (hud.rs). → look for other
  counts/filters/queries that include the wrong kind of object.

## How to work
Trace real execution paths (who calls this, with what values, in what order, on which frame). Back
up a suspected bug with a build, test, scratch repro or port run when you can. Verify numbers
(units: km vs m vs AU vs pc vs ly; degrees vs radians; f32 precision at large distances).

## What to return
Return **only** a list of findings, at most ~8, best first, each one line in this format:

`severity (critical/high/medium) | file:line | trigger (inputs/state) | what goes wrong | evidence | confidence (high/medium/low)`

If you find nothing that clears the bar, return "no findings" and one line on what you checked.
