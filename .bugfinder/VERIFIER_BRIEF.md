# Verifier brief

You are an adversarial verifier for a bug hunt on a Rust/Bevy port of SpaceEngine (repo
`C:\Users\liamw\Desktop\Space Engine Recomp`). Read `.bugfinder/HUNTER_BRIEF.md` for the rules
(no source edits, scratch only in `.bugfinder/scratch/verify-<batch>/`, port runs only through
`.audit/tools/runport.sh`, never run SpaceEngine itself, what counts as a bug, the not-bugs and
already-fixed list).

For each finding in your batch, **try to prove it wrong**:
- Re-read the whole code path yourself, starting from the cited line: callers, system ordering,
  guards elsewhere that already prevent it, whether the trigger can really happen in play.
- Check `docs/missing_features.md`: if it is listed there it is a missing feature, not a bug → drop.
- Reproduce it with a scratch test, a scratch se_core crate, or a short port run where possible.
- Check the stated impact is real and not overstated (e.g. "panic" when it is caught, "leak" that is
  bounded).

Return **only** one line per finding:

`ID | verdict (confirmed / likely / rejected) | corrected severity (critical/high/medium/low) | one-line reason, with the evidence (repro result or the code that proves/disproves it) | fix hint (one line: smallest correct fix, and files it touches)`

"confirmed" = reproduced or proven from code beyond doubt. "likely" = holds up but not reproduced.
"rejected" = wrong, already guarded, not reachable, a missing feature, or a not-bug.
