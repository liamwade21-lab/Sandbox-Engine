# Fixer brief, round 2

You fix verified bugs in a Rust/Bevy 0.20 port of SpaceEngine. You run in your **own git worktree**;
the main repo is `C:\Users\liamw\Desktop\Space Engine Recomp` (called MAIN below). Read MAIN's
`CLAUDE.md` first. Your bug list is `.bugfinder/fixers/GN.md` in MAIN. Each item points to the
hunter's finding (`.bugfinder/findings/NN.md`) and the verifier's verdict with a fix hint
(`.bugfinder/verdicts/NN.md`); read both. Line numbers there are from before round 1, so search
for the code rather than trusting them. The hint is a suggestion; make the smallest correct fix.

**Important:** MAIN has 46 uncommitted fixes from round 1 (see `.bugfinder/REPORT.md`) that your
worktree doesn't have yet. Start by bringing them in (step 2 below), so you work on the current code.

## Rules
- **Only edit the files your list says you own.** Other fixers are editing other files at the same
  time. If a fix truly needs a change in a file you don't own, don't make it: describe the exact
  change in your report.
- Do **not** commit, push, stash, reset or delete anything. Leave changes uncommitted.
- Keep each file's line endings (many are CRLF). Use the Edit tool; never `sed -i`.
- Match the surrounding code's style and comment density. No drive-by refactors.
- If a fix grows into a redesign, stop, leave it out, and say why.
- Never run SpaceEngine itself. Never edit MAIN's files.
- Keep command output short (`2>&1 | tail -30`).

## Setup (once, in your worktree, before any cargo command)
1. `mkdir -p target && cp -a "C:/Users/liamw/Desktop/Space Engine Recomp/target/release" ./target/`
2. `git -C "C:/Users/liamw/Desktop/Space Engine Recomp" diff --binary > ../base-GN.patch && git apply --whitespace=nowarn ../base-GN.patch`
   then check `git status --short` shows the ~34 modified files. This is your baseline; don't
   touch files outside your list.

## Reverse engineering (only if your items need it)
SE's Ghidra project lives only in MAIN. Run the decompiler from MAIN:
`cd "C:/Users/liamw/Desktop/Space Engine Recomp" && python -I .audit/tools/dec.py FUN_x` (results are
cached in MAIN's `.audit/work/dec/`, and earlier decompiles there are worth grepping first). If
Ghidra says the project is locked, another agent is using it: wait a minute and retry. SE's
decrypted shaders are in MAIN's `re/se_shaders`; `docs/` has notes on SE's formats. SE's data is in
the install `D:\SteamLibrary\steamapps\common\SpaceEngine` (read only).

## Build and test (in your worktree)
- For each fix, write a regression test that fails first where feasible, then fix.
- `cargo test -p se_core 2>&1 | tail -30` and `cargo test -p se_engine --release 2>&1 | tail -30`.
- `cargo build --release 2>&1 | tail -30` must pass with no warnings.
- Port runs (optional): `PORT_BIN="$PWD/target/release/spaceengine-rs.exe" bash "C:/Users/liamw/Desktop/Space Engine Recomp/.audit/tools/runport.sh" "C:/Users/liamw/Desktop/Space Engine Recomp/.bugfinder/scratch/fix-GN/NAME.png" FRAMES "dev; cmds"`.
  One run at a time across agents (the script waits for a lock), so keep runs few.

## Report (your final message)
For each item: `ID | fixed / partly fixed / not fixed | what you changed (file + function) | test
added (name) or why none | result`. Then: the exact list of files you changed (relative to the
baseline), the final test and build result lines, and any change needed in a file you don't own.
