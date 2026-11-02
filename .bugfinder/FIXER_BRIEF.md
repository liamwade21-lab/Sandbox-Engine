# Fixer brief

You fix verified bugs in a Rust/Bevy 0.20 port of SpaceEngine. You run in your **own git worktree**
(a copy of the repo at `main`); the main repo is `C:\Users\liamw\Desktop\Space Engine Recomp`
(called MAIN below). Read MAIN's `CLAUDE.md` first. Your bug list is in `.bugfinder/fixers/FN.md`
in MAIN; each item points to the hunter's finding (`.bugfinder/findings/`) and the verifier's
verdict with a fix hint (`.bugfinder/verdicts/`). Read both before fixing. The hint is a suggestion;
make the smallest correct fix.

## Rules
- **Only edit the files your list says you own.** Other fixers are editing the other files at the
  same time in their own worktrees. If a fix truly needs a change in a file you don't own, don't
  make it: describe the exact change in your final report.
- Do **not** commit, push, stash, reset or delete anything. Leave your changes uncommitted in your
  worktree; the main session will take them with `git diff`.
- Keep each file's line endings (many are CRLF). Use the Edit tool; never `sed -i`.
- Match the surrounding code's style and comment density. No drive-by refactors.
- If a fix grows into a redesign, stop, leave it out, and say why in your report.
- Never run SpaceEngine itself. Never edit MAIN's files.
- Keep command output short (`2>&1 | tail -30`).

## Build and test (in your worktree)
1. First, so you don't rebuild Bevy from scratch, copy MAIN's release build into your worktree:
   `cp -a "C:/Users/liamw/Desktop/Space Engine Recomp/target/release" ./target/` (create `target/`
   first). Do this once, before any cargo command.
2. For each fix, write a regression test that fails first where feasible (unit tests next to the
   code, in the file you own; se_core tests are cheap). Then fix, then run the tests.
3. Tests: `cargo test -p se_core 2>&1 | tail -30` and `cargo test -p se_engine --release 2>&1 | tail -30`.
4. Build: `cargo build --release 2>&1 | tail -30` must pass with no new warnings at the end.
5. Port runs (optional, for checks a test can't cover): use MAIN's lock script with your binary:
   `PORT_BIN="$PWD/target/release/spaceengine-rs.exe" bash "C:/Users/liamw/Desktop/Space Engine Recomp/.audit/tools/runport.sh" "C:/Users/liamw/Desktop/Space Engine Recomp/.bugfinder/scratch/fix-FN/NAME.png" FRAMES "dev; cmds"`.
   Your exe reads shaders from your worktree's `crates/se_engine/assets` (baked at compile time).
   Only one run happens at a time across all agents, so keep runs few.

## Report (your final message)
For each item: `ID | fixed / partly fixed / not fixed | what you changed (file:line) | test added
(name) or why none | result`. Then: the full list of files you changed, the final
`cargo test -p se_core`, `cargo test -p se_engine --release` and `cargo build --release` result
lines, and any change needed in a file you don't own.
