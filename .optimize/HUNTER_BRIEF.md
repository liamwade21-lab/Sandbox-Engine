# Optimization hunt: brief for every hunter

You are hunting for **speedups** in the SpaceEngine Rust/Bevy port (`C:\Users\liamw\Desktop\Space Engine Recomp`).
Read `CLAUDE.md` first and follow its rules. Then read your ticket in `.optimize/tickets/` and look only at the paths it lists.

## The one hard rule: output must stay identical
Same visuals (pixel for pixel where the renderer is deterministic), gameplay, physics results, audio,
saves, timing feel and public behaviour. No LOD / resolution / texture / sample-count cuts, no changed
timesteps, no float-precision changes that alter results, no dropped features, no reordering that
changes float summation results. A speedup that needs any of those still gets reported, but with
risk "medium" and a note that it changes output: the user decides.

Typical safe wins: work done every frame that only needs doing when something changed (change
detection, caching, dirty flags); allocations and clones in hot loops; O(n^2) loops and repeated
lookups; Bevy assets/components touched mutably every frame (that makes Bevy re-upload or
re-specialize them even when nothing changed); entities that are hidden but still extracted/queued;
broad queries; systems that could run in parallel; blocking work on the main thread that could go to
a task; logging in hot paths.

## Don't edit source
Do not edit anything under `crates/`, `Cargo.toml` or the shaders in the main checkout. Scratch files
and micro-benchmarks go in `.optimize/scratch/<your ticket>/` (or `.optimize/bench/` for a reusable
harness). If you must try a code change to measure it, do it in a separate copy (e.g.
`git worktree add .optimize/scratch/<ticket>/wt HEAD` and copy over `crates/se_engine/src/perf.rs`,
`main.rs` and `camera.rs` from the main checkout, which hold the uncommitted frame-time logger),
build there with `cargo build --profile agent --target-dir .optimize/scratch/<ticket>/target`, and
remove the worktree when done. Builds take minutes and slow down everyone's timings, so do this only
for a finding whose gain you cannot otherwise estimate.

## Profile data already taken (read these first)
- `.optimize/baseline.md`: frame times of every benchmark scene (release build, RTX 5080, Ryzen 7900X).
- `.optimize/scratch/trace/<segment>.full.txt`: the top 160 tracing spans of one segment from a
  `bevy/trace_chrome` build (ms/frame, call count, worst call). Segments: `rsg_look`, `rsg_still`
  (RS planet ground at 2 m, looking around / still), `earth_look`, `fly_1e15` (flying through stars),
  `neb_still` (inside a volumetric nebula). The trace build is slower than release (spans cost time),
  so use the numbers as proportions. Bevy runs the render app on its own thread (pipelined), so a
  frame takes about max(main app, render app).
- The raw traces (`.optimize/scratch/trace/<scene>.json`, about 6 GB each) can be summarised for
  another segment: `python -I .optimize/bench/trace_top.py TRACE.json 160 TRACE_SCENE.png.log SEGMENT`
  (takes a few minutes; the `.png.log` beside each trace names its segments).

## Running the port (allowed; one run at a time, the script waits for a lock)
- Benchmark scenes: `bash .optimize/bench/all.sh OUTDIR SCENE...` (scenes: start space galaxy fly
  earth mars rsplanet rsground nebula tarantula). Prints mean / median / p99 / max frame ms, 1% low
  fps, mean entity count, and per-render-pass GPU ms. Set `PORT_BIN=path/to/exe` to time another build.
- Your own scene: `bash .optimize/bench/scene.sh NAME "dev; commands; perf SEG; wait 600; perf end" OUTDIR`.
- Profiling build (already built): `PORT_BIN=target/trace/agent/spaceengine-rs.exe` writes a
  `trace-*.json` into the repo root; move it into your scratch folder and delete it when done (they
  are huge).
- Dev commands are in `crates/se_engine/src/camera.rs` (`run_commands`).
- Don't run SpaceEngine itself.

## What to return
Only a list of findings, at most ~8, best first, one per line in this format:

`ID | file:line | what's slow | proposed change | expected gain (ms/frame or hitch ms; measured/estimated) | risk to output (none/low/medium) | evidence`

Use your ticket number in the ID (e.g. `03-2`). Evidence = the trace numbers, a micro-benchmark you
ran, or the code path that proves the cost. Skip anything under ~0.1 ms/frame unless it causes
hitches. Be honest about risk: if the change could alter any float result, ordering, or what is drawn
in some edge case, say so.
