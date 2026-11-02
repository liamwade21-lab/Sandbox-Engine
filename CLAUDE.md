# SpaceEngine Rust/Bevy port

A private rebuild of SpaceEngine 0.991 in Rust (Bevy 0.20), drawn with Vulkan. It reads the
game's own files from the SpaceEngine install at run time; nothing from the game is copied here.
README.md explains the features, controls and command-line options; `docs/` holds the notes on
SE's file formats and reverse engineering; `docs/missing_features.md` lists what still differs
from SE. `.bugfinder/REPORT.md` is the 2026-11-01 bug hunt: what was found, fixed and left.

## Build and play

- Build: `cargo build --release` (always release). The user plays from
  `target\release\spaceengine-rs.exe`, so rebuild release after every change.
- Tests: `cargo test -p se_core` and `cargo test -p se_engine --release`.
- SpaceEngine install (read only): `D:\SteamLibrary\steamapps\common\SpaceEngine`.
- No .bat launchers yet.

## Rules from the user

- Vulkan only. VR and SE's editors are out of scope.
- Game assets are read from the install, never copied into the project.
- Ask before taking over the screen to run SpaceEngine. When it is allowed, a Sonnet agent
  (high effort) drives SE and takes the screenshots.
- No Opus high-effort subagents, except in the /bugfinder skill, which may use any agents it
  wants.
- Commit only when asked; never push. Commit messages end with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Keep each file's line endings (many files are CRLF): don't use `sed -i` on them.
- Don't defeat DRM; don't delete data without asking.

## Gotchas

- Shaders are loaded at run time from `crates/se_engine/assets/shaders` (no rebuild needed to
  try a shader change), but an edited shader takes a few hundred frames to compile in a test
  run: the ground is missing until then, so wait ~900 frames before judging a screenshot.
- Test runs: `bash .audit/tools/runport.sh OUT.png FRAMES "cmds"` (one run at a time). Useful
  dev commands: `select NAME`, `surface LAT LON ALT_KM PITCH`, `agl M` (hold M metres above
  the ground), `fly KMS`, `shot FILE`, `wait N`, `date`, `pause`.
- The user's own `se.log` and `settings.cfg` (in `%APPDATA%\SpaceEngine-rs`) are hidden from
  Claude's shell by the desktop app's sandbox; copy them out with a process started through
  `Invoke-CimMethod Win32_Process Create`.
- Reverse engineering: Ghidra project in `re/ghidra_projects/se`; decompile with
  `python -I .audit/tools/dec.py FUN_x`; SE's decrypted shaders are in `re/se_shaders`.
- Flattened planets (Earth, Saturn, fast-spinning giants) are drawn on an ellipsoid. Any shader
  maths that uses a sphere (atmosphere tables, camera height) must undo the ellipsoid per axis
  first, as `planet.wesl` `table_local` and the sky shell do. Dividing by one radius left
  flattened giants mostly flat blue.
- Reaching a star in another galaxy with dev commands: `select RG …` twice, `go <km inside it>`,
  `wait 600`, then `rs RS …`. The planet names then work with `select`.
