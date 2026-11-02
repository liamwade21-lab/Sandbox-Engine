# Optimization round 2: brief for implementers

Follow `.optimize/IMPLEMENTER_BRIEF.md` (rules, checks, report format), with these changes for round 2.
The user has approved the 14 "Recommended, risk low" items in `.optimize/REPORT.md`.

## Setup (replaces step 1)
Your worktree was made from the last commit, which lacks the 28 uncommitted round-1 speedups. Before
anything else, copy these files from the main checkout (`C:\Users\liamw\Desktop\Space Engine Recomp`)
into your worktree, unchanged:

```
crates/se_core/src/catalog.rs crates/se_core/src/climate.rs crates/se_core/src/climate/display.rs
crates/se_core/src/climate/tests.rs crates/se_engine/assets/shaders/clouds.wesl
crates/se_engine/assets/shaders/galaxy_far.wesl crates/se_engine/assets/shaders/galaxy_nebula_raymarch.wesl
crates/se_engine/assets/shaders/planet.wesl crates/se_engine/assets/shaders/stars.wesl
crates/se_engine/src/atmosphere.rs crates/se_engine/src/camera.rs crates/se_engine/src/clusters.rs
crates/se_engine/src/data.rs crates/se_engine/src/eclipse.rs crates/se_engine/src/ellipticals.rs
crates/se_engine/src/galaxies.rs crates/se_engine/src/hud.rs crates/se_engine/src/main.rs
crates/se_engine/src/nebula_raymarch.rs crates/se_engine/src/nebulae.rs crates/se_engine/src/overlay.rs
crates/se_engine/src/procgalaxies.rs crates/se_engine/src/procstars.rs crates/se_engine/src/procsystems.rs
crates/se_engine/src/stars.rs crates/se_engine/src/system.rs crates/se_engine/src/terrain.rs
crates/se_engine/src/universe.rs crates/se_engine/src/perf.rs
```

(Line numbers in the findings and verdicts refer to the code before round 1; search by function name.)

## Picture checks (replaces the refA comparison)
The "before" build for this round is `C:\Users\liamw\Desktop\Space Engine Recomp\.optimize\scratch\stage1exe\spaceengine-rs.exe`
(it has its own shaders beside it). Some scenes (space, saturn, tarantula) drift with the real clock,
so take the "before" shots and your shots back to back, and once more "before" afterwards for the
noise floor:
`PORT_BIN=<stage1exe> bash ".../.optimize/bench/refs.sh" <worktree>/.optimize-out/before SCENES`, then
your exe into `.../after`, then the stage1 exe into `.../before2`; compare with
`python -I ".../.optimize/bench/compare.py" before after` against `before before2`.

## Speed checks
Use `.optimize/bench/ab.sh`-style alternating runs: stage1 exe vs your release exe, the same scene,
alternating which goes first, 3 rounds, comparing the median of medians (`ab_table.py` reads a folder
of `base*`/`new*` runs; copy `ab.sh` into your scratch and point BASE/NEW at the two exes).

## Merging
Edit only the files your prompt gives you. Another implementer may own other functions of the same
file in a few named cases; keep your edits inside the functions named in your prompt there, so the
lead can merge both with `git merge-file`.
