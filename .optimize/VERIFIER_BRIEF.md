# Optimization hunt: brief for verifiers

You check speedup findings made by a hunter on the SpaceEngine Rust/Bevy port
(`C:\Users\liamw\Desktop\Space Engine Recomp`). Read `CLAUDE.md` and `.optimize/HUNTER_BRIEF.md` (the
rules the hunters followed, where the profile data is, how to run benchmarks), then the findings file
you are given in `.optimize/findings/`.

Your job is adversarial: try to prove each finding wrong. For each one:
1. **Is the cost real?** Re-read the code path yourself. Does the slow thing actually happen in the
   scene and as often as claimed? Check the trace numbers in `.optimize/scratch/trace/*.full.txt` (or
   the hunter's bench in `.optimize/scratch/<ticket>/`). Is the claimed gain plausible for the frame
   time? (Remember the render app runs in parallel with the main app: a main-thread gain only shortens
   the frame if the main app is the longer of the two in that scene, but it always removes hitches
   that it causes.)
2. **Could the proposed change alter any output?** Visuals, gameplay, physics results, saves,
   which objects exist, names/ids, ordering (ties in sorts, HashMap iteration feeding results, catalog
   order), float results (summation order, fused ops, f32 vs f64), timing-dependent logic (things that
   happen "this frame" vs "next frame", time budgets), edge cases (paused, high time rates, empty
   catalogs, unloaded systems, add-ons that edit records in place, scripts that change settings). Look
   for every reader of the data the change would cache or stop updating.
3. **Is it worth it?** Gain vs. complexity and risk of introducing bugs.

You may run the port through the scripts and build micro-benchmarks in `.optimize/scratch/verify-<x>/`.
Do not edit source in the main checkout. Other agents are running too, so timings are noisy.

Return only one line per finding, in this format:

`ID | verdict | measured/estimated | gain you believe | output risk after your check (none/low/medium) | reason (what you checked, what could break, conditions the implementer must keep)`

Verdicts:
- **recommend**: real, worth it, and the output stays identical if done as described (add any condition
  the implementer must respect).
- **needs-call**: real, but it changes output in some case (however small), or needs a design decision.
- **drop**: not real, not measurable, not worth it, or a duplicate (name the one it duplicates).
