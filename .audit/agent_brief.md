# Brief for feature agents (read fully before starting)

You are porting missing SpaceEngine 0.991 features into a Rust / Bevy 0.20 port of it. The
owner wants a one-to-one copy of SE. The gaps are audited in `docs/missing_features.md`
(each `### <id>` entry has SE evidence and the port location); raw auditor and checker lines
are in `.audit/gaps/` and `.audit/checks/`; `.audit/fixed.txt` shows how earlier fixes were
done and written up. `README.md` "Status" and `docs/` describe the port.

## Where things are
- You work in your own git worktree on your own branch. Edit only files inside it.
- Main checkout (read only for you): `C:/Users/liamw/Desktop/Space Engine Recomp`. Its
  `re/` has the Ghidra project (`re/ghidra_projects/se`), SE's decrypted shaders
  (`re/se_shaders`), exe strings and the RTTI class list; these are not in your worktree.
- SE install (read only, never write into it, never copy its assets into the repo):
  `D:/SteamLibrary/steamapps/common/SpaceEngine`. The port reads assets from it at runtime.
- Shaders: `crates/se_engine/assets/shaders/*.wesl`, loaded at runtime from the crate that
  was built, so your build uses your worktree's shaders.

## Rules
- Vulkan only. VR and the editors are out of scope. Don't defeat DRM.
- Don't work on the gaps in `.audit/deferred.txt` (deferred by the owner), even where they touch
  your files; `08-*` there means the whole Interface area. Adding a control for a setting your
  own work needs to an existing window is fine.
- Never run SpaceEngine itself and never take over the screen. Running the port is fine.
- Never write SE's files to disk, not even into scratch folders: read them in memory (the
  install's loose files, or paks through `se_core::vfs`) and print only the lines you need.
- Don't install software. Don't ask the user anything; make the call, note it.
- Don't edit `README.md`, `.audit/fixed.txt` or `docs/missing_features.md`, and stay out of the
  files your prompt says belong to someone else. The main session merges and writes the docs.
- Match the surrounding code: naming, comment density, doc comments citing SE functions
  (e.g. `(FUN_14047b670)`) and config keys. Prefer SE's own constants, read from its configs
  where the port already does that (`main_def_f64` in `stars.rs`, `EngineConfig`).
- Keep output small: run builds/tests with `-q` and `2>&1 | tail -30`; search before reading;
  read big files in parts.

## Reverse engineering SE
Run these from the main checkout (`cd "C:/Users/liamw/Desktop/Space Engine Recomp"`); several
agents can call Ghidra at once.
- `python -I .audit/tools/dec.py FUN_x [FUN_y]` decompiles into `.audit/work/dec/FUN_x.c`.
- `python -I .audit/tools/gx.py "Some string"` lists functions referencing a string.
- `ghidra x-ref to 0xADDR --projects-dir re/ghidra_projects --project se --json`,
  `ghidra function get ADDR --projects-dir re/ghidra_projects --project se`.
- Config vars are registered with `FUN_1405a4a00(&var, name, default, min, max, step)`; the
  value pointer is at var+0x20. Key bindings are in SE's key table `FUN_1405ba8f0`.
- SE's shaders: `re/se_shaders/*.glsl`; configs and locale in the install's `config/`,
  `data/` and `.pak` files (the port's `se_core::vfs` reads paks).

## Build, test, run
Use your own target dir (given in your prompt; it is pre-seeded so only the port's crates
rebuild) and the `agent` profile (release optimisation without LTO, incremental: about a
minute a rebuild instead of six). Never build into the main checkout's `target/`.
```
export CARGO_TARGET_DIR=<your dir>
cargo build --profile agent -q 2>&1 | tail -30
cargo test --profile agent -q 2>&1 | tail -30
```
Screenshots: `PORT_BIN="$CARGO_TARGET_DIR/agent/spaceengine-rs.exe" bash .audit/tools/runport.sh OUT.png FRAMES "cmds"`
(cmds separated by `;`, e.g. `goto Sirius 0.01; wait 30; fov 20; shot b.png`). The dev
commands are parsed in `crates/se_engine/src/camera.rs` around line 1000. `SE_FRAMETIME_LOG=file`
writes `frame ms nearest` lines. Other agents share the GPU, so compare frame times against a
baseline run of the unmodified port built the same way,
`C:/Users/liamw/Desktop/se-targets/agentseed/agent/spaceengine-rs.exe` (don't rebuild it),
taken at the same time, not against absolute numbers. Put screenshots and scratch files in
`.audit/work/<your package>/` of your worktree (git ignores it). Look at your screenshots.

## Done means
1. Each gap in your package is done, or the part you couldn't do is stated with the reason.
2. `cargo build --profile agent` has no new warnings and `cargo test --profile agent` passes; add unit
   tests for logic you add (constants, formulas, parsing), like the existing tests.
3. Screenshots show the feature working, and frame times are not worse than the baseline.
4. `.audit/notes/<gap id>.md` per gap, under 15 lines: what SE does (functions, keys,
   constants), what you ported, how you checked it, what is not ported, and any README line
   you propose (controls table, status).
5. Everything committed on your branch (`git add -A && git commit`), message ending with
   `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

Final reply, at most 12 lines: branch and last commit, each gap done / partial / not done,
files touched outside your own, screenshot paths worth looking at, risks for the merge.
