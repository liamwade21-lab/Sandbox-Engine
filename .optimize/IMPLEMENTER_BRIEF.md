# Optimization: brief for implementers

You implement approved speedups in the SpaceEngine Rust/Bevy port. You work in your own git worktree
(you were started with worktree isolation); the main checkout is
`C:\Users\liamw\Desktop\Space Engine Recomp` (read `CLAUDE.md` there first and follow its rules).

## Setup
1. Your worktree was made from the last commit, which lacks the uncommitted frame-time logger. Copy
   these three files from the main checkout into your worktree, unchanged, before anything else:
   `crates/se_engine/src/perf.rs`, `crates/se_engine/src/main.rs`, `crates/se_engine/src/camera.rs`.
2. Read your items in the findings files (`.optimize/findings/NN.md`, the hunter's description) and the
   verdict files (`.optimize/verdicts/*.md`, the verifier's conditions). Both are in the main
   checkout. **The verifier's conditions are binding**: implement exactly the variant the verdict
   recommends, not the hunter's original if they differ.

## Rules
- **Output must stay identical:** same pixels (up to the run-to-run noise of the reference shots),
  same behaviour, same results, same ordering, same floats. If an item can't be done that way, or
  grows into a redesign, leave it out and say why.
- Edit only the files you own (listed in your prompt). If an item needs a file you don't own, leave
  that part out and say so.
- Match the surrounding code: naming, idiom, comment density (short comments that say why). Keep
  each file's line endings (many are CRLF; check with `file`); don't use `sed -i` on source.
- Where feasible, add a unit test that pins the item's exactness, for example the cached or fast path
  gives the same result as the old path.
- Don't commit, push or delete anything outside your worktree's build folders.

## Checking each item
- Build and test in your worktree: `cargo build --profile agent` for iteration, then once at the end
  `cargo build --release`, `cargo test -p se_core` and `cargo test -p se_engine --release`. Report any
  failures with their output.
- **Identical pictures:** with your release exe, take reference shots of the scenes your items
  touch: `PORT_BIN=<worktree>/target/release/spaceengine-rs.exe bash "C:/Users/liamw/Desktop/Space Engine Recomp/.optimize/bench/refs.sh" <worktree>/.optimize-out/refs SCENE...`
  (scenes: space galaxy stars earth mars rsplanet rsground nebula tarantula saturn), then compare with
  `python -I "C:/Users/liamw/Desktop/Space Engine Recomp/.optimize/bench/compare.py" "C:/Users/liamw/Desktop/Space Engine Recomp/.optimize/scratch/refA" <worktree>/.optimize-out/refs`.
  Your numbers must be no worse than refA vs refB (`compare.py refA refB`: Earth, Mars and both RS shots
  are exactly identical; the others show small run-to-run noise). The exe finds its shaders through
  its own build path, so a worktree exe uses the worktree's shaders.
- **Faster:** show each item's gain with a micro-benchmark (old vs new path), the per-frame log, or
  the benchmark scenes: `PORT_BIN=... bash "C:/Users/liamw/Desktop/Space Engine Recomp/.optimize/bench/all.sh" <outdir> SCENE...`.
  Other implementers are building at the same time, so scene timings are noisy: repeat them, compare
  medians, and prefer micro-benchmarks. The lead runs the final clean benchmark after merging.
- Port runs go through the scripts, which take a lock (one run at a time across all agents). Don't run
  SpaceEngine itself.

## Report (your final message)
For each item: done / left out (why), what you changed, the test you added, the screenshot comparison
numbers, and the speed evidence. Then the full list of files you changed (paths relative to the repo
root), and your worktree's path.
