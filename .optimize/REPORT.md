# Optimization report (2026-11-02)

Scope: the whole port (crates/se_core, crates/se_engine, shaders), weighted toward the user's three
complaints: standing on an RS planet and looking around, flying fast through stars, and being inside
volumetric nebulae. Level L (about 94k lines): 9 hunters, 7 verifiers, 8 implementers (4 per round), all Opus at
high effort. Baseline and method: `baseline.md`. Findings: `findings/NN.md`. Verdicts: `verdicts/`.

The user went /afk during the hunt, so round 1 implemented only the items **Recommended with risk
"none"** (28). The user then approved the 14 "Recommended, risk low" rows, done in round 2. The 8
"Needs your call" items are not done. Headline: planet surfaces more than twice as fast, flying
through stars 34-47% faster with much smaller hitches, nebulae 6-16% faster, worst start-up frame
784 → 145 ms, pictures unchanged (see "Final results").

## Where the time went (baseline, release build, RTX 5080 / Ryzen 7900X)
- **Planet surfaces** (Earth 14.7 ms, RS planet ground 14.4 ms, Mars 10.5 ms): the render thread was
  the bottleneck. Bevy re-prepared every star-field node every frame because shared star/galaxy
  materials were touched each frame. Multi-draw bookkeeping ran for each of ~400 ground patches,
  each with its own material. Every ~13 s on the RS planet, a relight rewrote all ~1,600 patch
  materials at once, giving a ~48 ms stall. The info panel (3.4 ms) and moving the Solar System's
  2,517 bodies (3.3 ms of CPU) added to it on Earth.
- **Flying fast through stars** (12-17 ms, 1% low 19-22 fps, spikes to 63 ms): every frame the main
  thread searched all 134,961 catalog stars for the nearest one (9.5 ms average). Each system
  entered or left caused 7-30 ms hitches: unloading the old system's names, cloning the new
  system's data, rebuilding the black-hole list, and building aurora meshes.
- **Inside nebulae**: the 16.7 ms seen once was a 60 fps cap applied from outside the port (window not
  in focus), not real work. The real figure is ~6 ms, CPU-bound. The nebula shader itself takes
  ~5 ms of GPU time at 1600x900, mostly because it ray-marches at a higher resolution than SE does.
- **Start-up**: worst frame ~830 ms, because Bevy's 4-thread background pool was full of galaxy builds
  while the first frame waited for a pipeline. About 300 ms more went on recomputing 130k star colours.

## Implemented (Recommended, risk none)
28 findings (by ID) are in the working tree, not committed. 08-7 was implemented and then reverted
(see "Reverted"). Measured results for the merged build are under "Results".

| ID | Where | Change | Expected gain | Evidence |
|---|---|---|---|---|
| 01-2 | stars.rs, galaxies.rs + star/galaxy-far shaders | Per-frame camera values of star/galaxy sprites moved into a shared buffer written in place, so ~2,800 entities stop being re-prepared every frame | ~2.5-3 ms/frame on planet surfaces | measured (upper bound) |
| 02-1 | terrain.rs, planet.wesl, clouds | Terrain lighting in a per-patch-slot storage buffer; the relight no longer rebuilds ~1,600 materials | removes a ~35-48 ms stall every ~13 s on the RS planet | measured (trace) |
| 05-1, 05-5 | hud.rs, se_core climate | Info panel: the system's member walk cached per system + catalog change; the star view built once a frame | ~3 ms/frame with a body selected; no ~1 ms hitch on selecting | measured |
| 05-2 | hud.rs | Child counts cached per selection | 1.4-2.1 ms/frame with a star selected | measured |
| 01-4 | galaxies.rs, ellipticals.rs | No material writes for galaxy models/planes/ellipticals that are hidden that frame | ~1-1.5 ms/frame | measured |
| 01-5 / 09-2 | main.rs, camera.rs | GPU light clustering off (the port has no Bevy lights) | ~0.3-0.9 ms render thread | measured (09) |
| 07-2 | procstars.rs, procgalaxies.rs, clusters.rs | Node positions written only when they change | 0.5-0.8 ms/frame | measured |
| 07-4 | nebula_raymarch.rs | Ray-march model constants computed once per model | 0.5-0.7 ms/frame | measured (bench) |
| 07-5 | procstars.rs | Debug-only scan of 1M stars skipped when debug logging is off | removes a 14-16 ms hitch every 120 frames | measured |
| 07-6 | galaxies.rs, ellipticals.rs | Galaxy coverage/wanted list cached; ellipticals scan only candidates | 0.2-0.3 ms/frame | estimated |
| 07-7 | procgalaxies.rs | promote_near skipped when nothing could have changed | 0.15-0.2 ms/frame, no 1 ms hitch | estimated |
| 02-3, 02-4, 02-6 | terrain.rs | Patch-selection walk: horizon once per walk, sort once, fast hasher, poll only loading patches | ~0.4-0.6 ms/frame main thread | measured (bench) |
| 04-3 | universe.rs | Floating origin skips hidden, mesh-less bodies | 0.2-0.5 ms/frame | estimated |
| 04-4 | terrain.rs | Eclipses only for bodies that can have one | 0.15-0.2 ms/frame | estimated |
| 04-5, 04-6 | system.rs, eclipse.rs | Lighting fade constants once a frame; eclipse caster list once per receiver | ~0.2 ms/frame | measured / estimated |
| 05-3a | overlay.rs | Label scan tests "Hidden" last | halves a 2.5 ms hitch every 15 frames | measured |
| 01-6 | system.rs | Body sprite mesh rebuilt only when it changed | ≤0.4 ms on still frames | estimated |
| 03-3 | se_core catalog.rs | Entering a system moves the parsed data instead of cloning it | 10-30 ms off the frame that enters a large system | measured |
| 03-6 | procsystems.rs | Milky Way lookup once a frame, not per slot | 0.05-0.15 ms/frame | estimated |
| 06-4 | galaxy_nebula_raymarch.wesl | Gas loop only where its result is used | ~0.1 ms GPU | measured |
| 08-3 | atmosphere.rs | Atmosphere file existence check without inflating 17 MB | entering the Solar System 743 → 236 ms | measured |
| 08-4 | stars.rs | Star colours memoised per temperature | ~300 ms off the first frame | measured |
| 08-5 | data.rs, main.rs | Game data loads on a thread while the window and renderer start | window opens ~0.5-0.8 s sooner | estimated |
| 08-6 | se_core catalog.rs | Catalog files read and parsed in parallel, merged in the original order | Catalog::load ~520 → 330 ms | measured |

## Round 2: the "Recommended, risk low" items (approved by the user, implemented)
The verifiers found these real and worthwhile, but each could differ from today in some edge case,
usually a tie that is already decided arbitrarily today. They waited while you were away; you then
approved all 14, and four implementers did them in round 2 (results per item under "Round 2 results").

| ID | Where | Change | Gain | Why "low" |
|---|---|---|---|---|
| **01-1 / 09-1** | camera.rs | `NoIndirectDrawing` on the main camera: Bevy's multi-draw path batched nothing (one material per ground patch) but cost per-patch bookkeeping every frame | **~3-4 ms/frame on Earth and the RS planet ground (two hunters measured it independently: 15 → 11 ms)** | draw order can differ only in exact depth ties between meshes sharing a material (ship parts), which today's order already decides arbitrarily; screenshots within run-to-run noise |
| **03-1** | system.rs | Nearest catalog star: exact cached search instead of scanning 135k stars every frame | **~3-5 ms/frame flying fast** | needs a catalog revision counter at every mutation site; a missed site would give a stale nearest star |
| 04-1 | blackholes.rs | Black-hole list extended incrementally | ~1.5 ms/frame flying, removes 4-10 ms hitches | needs the same revision counter for records edited in place |
| 03-2 | se_core catalog.rs | Unloading a system removes only its own names | removes a 10-20 ms hitch per system entered | an overridden record's old alias keys could be left behind |
| 03-4 | aurora.rs | Aurora meshes built when first visible | removes 15-30 ms hitches when entering systems with auroras | the mesh must exist on the first visible frame |
| 03-5 | procstars.rs | Nearest procedural star: visit nodes nearest-first | removes 14-16 ms spikes | exact distance ties may pick another star (already random today) |
| 04-2 | system.rs, se_core ephemeris | Body positions: parents computed once per frame | ~0.6-1 ms CPU/frame (0.2 ms wall) | fallback needed for broken parent chains |
| 07-3 | procstars.rs | Star-octree wanted-node list cached while the camera is still | 0.4-0.9 ms/frame on the ground | bookkeeping of node ages must match exactly |
| 08-1 / 09-5 | main.rs | Background thread pool 4 → 8 threads (respecting MaxThreads) | worst start-up frame ~830 → ~210 ms; terrain and stars load sooner | which frame a galaxy first appears on can shift |
| 09-4 | se_engine/Cargo.toml | Turn off Bevy features the port never uses (UI, sprites, gizmos, picking, gamepad, glTF, animation...) | ~0.15-0.3 ms/frame, shorter builds | not verified or built yet |
| 08-2 | galaxies.rs | Galaxy/nebula sprite meshes kept only on the GPU | ~350 MB less peak memory (500 MB on Mars) | rebuilt mesh must be byte-identical |
| 06-5 | planet.wesl | Skip the second biome's texture sampling when its blend weight is 0 | 0.45-0.6 ms GPU on Earth | derivatives must be computed outside the branch |
| 02-5 | terrain.rs | Skip the patch visibility pass when nothing changed | 0.2-0.4 ms/frame | needs careful reset rules |
| 03-7 | system.rs | Reuse one climate lookup per body | small | bodies may finish spawning a frame sooner |

## Round 2 results (per item, from the implementers; stage1 = the round-1 build)
- **03-1 + catalog revision counters:** the catalog now counts changes ("revision") and in-place
  edits ("edits"); every mutating method bumps them. Nearest catalog star: 2.7-3.0 → 0.04 ms/frame
  in game (bench, identical results on 1,200 frames). Test against a full scan over 900 frames with
  catalog edits on the way.
- **04-1:** black-hole list extended incrementally; full rebuild on in-place edits, warp radius or a
  new catalog. Entering a multiple-star system still forces a full rebuild (load_generated edits the
  root's mass in place). Test against a full build, confirmed to fail if the late-placement handling
  is removed.
- **03-2:** unloading a system removes only its own names (falls back to the full sweep for a record
  replaced in place, closing the verifier's known gap). 9.0 → 2.0 ms average, worst 16.4 → 9.5 ms.
  Test against the old full sweep over 12 systems and awkward cases.
- **03-7:** one climate lookup per body instead of two.
- **Fly scene, stage1 → these items:** median 8.45 → 4.74 ms (1e14 km/s) and 14.58 → 6.91 ms (1e15),
  p99 50 → 32 ms, worst 70 → 49 ms (busy machine).
- **04-2:** ancestors solved once per frame; 2,517 Solar System bodies 1.5 → 0.7 ms of CPU (the loop
  is parallel, so ~0.2 ms of frame time). All 15,102 positions/orientations byte-identical at 6
  dates; tests on a made-up catalog (binaries, cycles, depth cap) and the real install.
- **01-1 / 09-1:** `NoIndirectDrawing` on the main camera. Earth 11.8 → 5.7 ms, RS ground 8.0-9.1 →
  5.4-5.7 ms (with 08-1, 06-5, 02-5 and 09-4; stage1 vs these items).
- **08-1 / 09-5:** background pool = a third of the threads, at most 8, when MaxThreads is 0; a
  user-set MaxThreads is kept. Worst start-up frame 644 → 134 ms. Alone it raises start-up peak
  memory 100-200 MB; 08-2 more than pays it back.
- **06-5:** second biome sampled only when its weight is above 0, derivatives taken outside the
  branch. Earth pixel-identical; Earth opaque pass 3.99 → 3.76 ms GPU (still), 3.42 → 3.06 (look).
- **02-5:** patch visibility pass skipped when nothing changed (1,180-1,240 of 1,500 frames):
  0.17-0.48 → 0.03-0.04 ms. A temporary build re-ran the full pass on every skipped frame: zero
  mismatches.
- **09-4:** Bevy and bevy_egui default features trimmed (UI, sprites, text, gizmos, picking,
  gamepad, scenes/glTF, animation, states, WebGL...). Verified by the implementer: same feature
  tree for images, audio, shaders, window/input; same logs; pictures within noise. ~0.2-0.3 ms per
  frame; exe 107 → 82 MB; Cargo.lock loses ~870 lines.
- **03-5:** nearest procedural star: the camera's own finest node is scanned first, then the old
  loop. Normal case unchanged (0.40 ms); the unlucky node order that caused the 14-16 ms spikes
  47.6 → 0.34 ms. (The nearest-first heap variant was slower and dropped.)
- **07-3:** wanted star-octree nodes cached on 13 input bit patterns plus an octree generation, with
  exact last-used bookkeeping (test over 345 frames compares drops, keys, last_used and map order
  with the old marking). Saves 0.4-7 ms a frame when still, depending on the magnitude limit.
- **03-4:** aurora meshes built the first frame the aurora is visible, byte-identical to before.
- **08-2:** galaxy and nebula sprite-model meshes kept only on the GPU; the sort rebuilds them
  byte-identically. Start-up peak memory about 300-450 MB lower in a quiet session. (The nebula
  half was applied by the lead in nebulae.rs from the implementer's notes.) Noted by its implementer:
  in the fly scene the typical frame was 0.3-0.5 ms slower while the spikes were much smaller;
  see the final benchmark below.
- **Found, not caused, by round 2:** the old orientation code recurses forever (stack overflow) on a
  catalog where a tidally locked body's parent has a parent-equator orbit pointing back. No stock
  catalog does this; left as it is (a bug, not a speed item).

## Needs your call (changes the picture, or a design decision)
| ID | Change | Gain | Why |
|---|---|---|---|
| 09-3 | Release build: fat LTO, panic = abort, CPU-specific code | ~0.5-1.5 ms/frame (one noisy measurement) | release builds took ~38 min instead of ~9; a crash no longer unwinds; a `native` exe may not run on other PCs |
| 06-1 | Render volumetric nebulae at SE's VolumResolution (0.35-0.5) and upscale, as SE does | ~3 ms GPU inside nebulae, more at large windows | changes the picture (lower-res nebula, upscaled), toward SE's own look |
| 06-2 | Scalar spiral noise in the nebula shader when both halves are equal | 0.6-0.85 ms GPU | ~2% of nebula pixels change by more than 1% |
| 06-3 | Nebula pipelines specialised on their flags | 0.4-0.5 ms GPU | tiny pixel shifts; a nebula can be missing for a few frames the first time it appears |
| 02-7 | One material per planet instead of per patch | several ms (mostly covered by 01-1) | large rework of the terrain material system |
| 05-3b | Candidate cache for the star label scan | the remaining ~1.3 ms hitch every 15 frames | misses stars edited in place unless the catalog gets a revision counter |
| 05-4 | Marker labels built only for drawn markers | ~0.1 ms | a label could show the new name a frame early |
| 08-8 | Spread start-up uploads over frames | start-up p99 maybe 89-146 → 40 ms | models appear a frame or two later while loading |

## Dropped
| ID | Why |
|---|---|
| 02-2, 01-3 | duplicates of 07-5 and 07-2 |
| 04-7 | under 0.1 ms, with two traps |
| 06-6 | no measurable gain, and every pixel shifts slightly |

## Reverted
| ID | Why |
|---|---|
| 08-7 | Converting the atmosphere tables to half floats inside the loader task put that work on Bevy's 4-thread background pool, which the start-up stall already waits on. A/B over 6 alternating rounds: worst start-up frame 908-1315 ms with it vs 650-796 ms without, and no measurable drop in peak memory. Reverted; 08-3 (the file check) stays. |

## Final results (both rounds, against this morning's build)
The morning baseline exe (with its own shaders) against the final build, scene by scene,
alternating which runs first, 3 rounds (`scratch/final2/ab`). The machine was quiet this time, so
the runs agree closely. "Typical" = median of the 3 runs' median frame times.

| Scene | Frame time, typical (ms) | 99th percentile (ms) | Worst frame, typical (ms) |
|---|---|---|---|
| RS planet ground (2 m), looking around | 13.00 → **5.56** (-57%) | 15.3 → 7.2 | 45.7 → **7.8** |
| RS planet ground (2 m), still | 11.82 → **5.47** (-54%) | 13.5 → 6.9 | 50.0 → **7.0** |
| RS planet, 300 m, looking around | 10.29 → **4.82** (-53%) | 12.1 → 6.2 | 32.3 → 7.2 |
| RS planet, 300 m, still | 10.55 → **4.87** (-54%) | 12.0 → 6.2 | 13.3 → 6.5 |
| Earth surface, looking around | 13.81 → **5.71** (-59%) | 16.1 → 8.9 | 18.3 → 10.0 |
| Earth surface, still | 13.66 → **5.58** (-59%) | 15.7 → 6.9 | 16.8 → 7.4 |
| Mars surface, looking around / still | 9.09 / 9.04 → **4.99 / 4.78** (-45%) | 11.9 → 6.1-8.5 | 13.5-14.1 → 6.5-9.8 |
| 20,000 km from Earth | 7.70 → **4.31** (-44%) | 10.6 → 5.9 | 12.4 → 6.4 |
| Milky Way from 30 kpc | 4.95 → **2.90** (-41%) | 7.6 → 3.8 | 8.9 → 4.2 |
| Flying through stars, 1e14 km/s | 6.72 → **3.56** (-47%) | 10.7 → 9.3 | 26.0 → 14.6 |
| Flying through stars, 1e15 km/s | 7.40 → **4.87** (-34%) | 36.0 → **17.3** | 52.9 → **20.5** |
| Inside the Skylias nebula, looking / still | 4.60 / 5.94 → **3.85 / 5.57** (-16% / -6%) | 6.3 → 5.7 | 8.4 → 5.9 |
| Inside the Tarantula, looking / still | 5.52 / 5.42 → 5.19 / 5.11 (-6%) | 6.6 → 6.3 | 7.7 → 6.5 |
| Start-up, first 300 frames | median 8.36 → **5.94** | 84.8 → 85.2 | **783.9 → 144.7** |

Peak memory (median of 3 runs, MB): start-up 1794 → 1800, RS ground 2000 → 1912, Mars 1802 → 1724,
Earth 1758 → 1781, galaxy view 1794 → 1707. Galaxy/nebula meshes kept only on the GPU (08-2) save
memory, and the larger start-up thread pool (08-1) spends about the same at start-up.

**Identical output (round 2):** reference shots of 10 fixed scenes, round-1 build → final → round-1
build again: the final build differs from the round-1 build no more than the round-1 build differs
from itself. Earth and Mars are pixel-identical; the RS planet shots are 85-88 dB (two round-1 runs:
89 dB); the rest sit at the same noise floor (43-62 dB). Round 1 showed the same for the morning
build → round-1 build (below).

**Tests (final tree):** `cargo test -p se_core` 306 passed, 22 ignored; with `--ignored` (reads the
install) all 22 pass. `cargo test -p se_engine --release` 201 + 3 passed, 4 ignored (benches).
`cargo build --release`: no warnings. 24 tests were added across both rounds, plus 2 that read the install.

What this means for the three complaints:
- **Standing on an RS planet, looking around:** more than twice as fast (13.0 → 5.6 ms, ~180 fps),
  and the ~13-second stutter is gone (worst frame 46 → 8 ms).
- **Flying fast through stars:** 34-47% faster, and the hitches when passing star systems are much
  smaller (99th percentile 36 → 17 ms, worst 53 → 21 ms).
- **Inside volumetric nebulae:** 6-16% faster. The nebula shader's own GPU cost is unchanged; the
  big saving there (06-1, render it at SE's lower volumetric resolution) changes the picture and
  needs your call.

## Round 1 results (before the 14 approved items)
Old build (baseline exe with its own shaders) against the merged build, scene by scene, alternating
which runs first, 3 rounds each (`.optimize/bench/ab.sh`, output in `scratch/ab`). The machine was
busier than in the morning baseline (other programs used the CPU), so the absolute numbers are higher
than `baseline.md`'s; the comparison is between runs taken side by side. "Typical" is the median of
the 3 runs' medians.

| Scene | Frame time, typical (ms) | Best run (ms) | Worst frame, typical (ms) | 99th percentile (ms) |
|---|---|---|---|---|
| RS planet ground (2 m), looking around | 15.07 → **11.03** (-27%) | 14.86 → 10.74 | 63.5 → **19.8** | 21.2 → 17.2 |
| RS planet ground (2 m), still | 13.89 → **11.41** (-18%) | 12.83 → 9.06 | 55.0 → **22.6** | 17.3 → 14.6 |
| RS planet, 300 m, looking around | 12.23 → **8.82** (-28%) | 11.60 → 8.42 | 30.7 → **14.5** | 17.2 → 12.3 |
| RS planet, 300 m, still | 12.29 → **8.75** (-29%) | 11.82 → 8.22 | 34.5 → **11.0** | 14.3 → 10.5 |
| Earth surface, looking around | 16.59 → **13.07** (-21%) | 14.79 → 12.85 | 39.4 → 23.8 | 21.6 → 19.4 |
| Earth surface, still | 16.98 → **14.27** (-16%) | 14.57 → 12.89 | 25.0 → 20.5 | 22.0 → 19.9 |
| Mars surface, looking around | 10.71 → **7.25** (-32%) | 9.76 → 6.52 | 18.9 → 11.9 | 14.3 → 10.8 |
| Mars surface, still | 11.77 → **6.89** (-41%) | 9.61 → 6.21 | 18.6 → 11.5 | 15.9 → 9.1 |
| 20,000 km from Earth | 9.39 → **5.62** (-40%) | 8.30 → 5.31 | 15.3 → 9.6 | 12.5 → 7.5 |
| Milky Way from 30 kpc | 8.66 → **4.53** (-48%) | 5.81 → 3.33 | 16.2 → 9.6 | 12.8 → 6.5 |
| Inside the Skylias nebula, still | 5.97 → 5.84 (-2%) | 5.95 → 5.73 | 8.8 → 7.6 | 6.8 → 6.5 |
| Inside the Skylias nebula, looking around | 5.39 → **4.34** (-19%) | 4.99 → 3.91 | 10.6 → 8.9 | 7.1 → 6.3 |
| Inside the Tarantula, still / looking | 5.52 / 5.56 → 5.21 / 5.29 (-5%) | 5.46 → 5.20 | 8.8 → 6.1 | 6.5 → 6.0 |
| Flying through stars, 1e14 km/s | 7.84 → 8.31 (noise) | 7.27 → 6.86 | 28.5 → 30.4 | 12.5 → 13.5 |
| Flying through stars, 1e15 km/s | 9.26 → 9.45 (noise) | 8.21 → 7.75 | 55.3 → 57.6 | 40.2 → 40.4 |

What this means for the three complaints:
- **RS planet surface, looking around:** about 27-29% faster, and the regular ~55-63 ms stutter is
  gone (worst frames now ~20 ms). One more ~3-4 ms is waiting for approval (01-1).
- **Flying fast through stars:** unchanged. Its big fixes (03-1 nearest-star search, 03-2 unload,
  03-4 auroras, 04-1 black holes) were all rated "low" risk, so they wait for your approval.
- **Inside nebulae:** 2-19% faster. The nebula's own GPU cost is unchanged: the large saving there
  (06-1, rendering it at SE's lower volumetric resolution) changes the picture and needs your call.

GPU pass times are unchanged (the terrain and sprite values now read from storage buffers cost
nothing measurable), and the light-clustering pass is gone.

**Identical output:** reference shots of 10 fixed scenes (time paused) from the merged build against
two runs of the old build differ no more than the two old runs differ from each other. Earth and
Mars are pixel-identical; both RS planet shots are 96-100 dB (a few pixels off by 1/255; old against
old 102-110 dB); galaxy, stars, space, Saturn and the nebulae are all at the old-vs-old noise floor
(42-47 dB). Two known, intended differences:
- Star motion-blur trails are sized by frame time above 60 fps (SE's own rule), so a faster build
  draws slightly different trails while moving. This is behaviour, not a change to it.
- Nebula far sprites used to lose their motion blur on some frames, depending on the order in which
  two systems happened to run. They now always get it, as the code meant.

**Tests:** `cargo test -p se_core` 304 passed (+3 new), 21 ignored (+1 new), and the 21 install-reading tests
pass with `--ignored` (including the new parallel-vs-serial catalog check). `cargo test -p se_engine
--release` 193 passed (+9 new), 4 ignored (benches). The release build has no warnings.

**Start-up** (6 alternating rounds, `scratch/ab_start2`, final build with 08-7 reverted):
- Time from launch to the game running (first dev command): ~1.8-2.2 s → **~0.8-1.2 s**.
- First 300 frames: mean 14.7-18.9 → **10.6-14.6 ms**, median 9.8-13.2 → **6.8-9.0 ms**.
- Worst frame: 795-931 → **455-833 ms** (under 500 ms in 4 of 6 runs). The rest of that stall is
  the background thread pool; 08-1 (waiting for approval) measured ~210 ms.
- Loading the catalogs: 916 → 394 ms; star colours 305-326 → 12-25 ms; entering the Solar System
  ~300 → ~170 ms.

**Memory:** unchanged (peak 1.6-2.1 GB at start-up, ~2.5 GB in the busiest scene, old and new
alike). The memory items (08-2, -350 MB) wait for approval.

## What's left
- The 8 "Needs your call" items change the picture or are design decisions. The most valuable is
  06-1 (~3 ms of GPU time inside nebulae, more at large windows, by rendering them at SE's own
  volumetric resolution).
- The remaining frame time is spread thin; a new profile of the final build would be the place to
  start a next round.
- Housekeeping: done on 2026-11-02 at the user's request. The agent worktrees, everything in
  `.optimize/scratch` (traces, saved exes, the A/B and reference-shot outputs named above) and the
  trace build in `target/trace` were deleted (about 100 GB); the worktrees' changes were checked to
  be in `083e5d7` first (all but the reverted 08-7). The bench scripts recreate their output folders.
