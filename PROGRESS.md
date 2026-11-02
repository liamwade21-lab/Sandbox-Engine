# Progress

## Current state (2026-11-02)

- The port runs and covers most of SpaceEngine; see README.md "Status" and
  `docs/missing_features.md` for what is left.
- **V1:** `main` at 97094fd is the port's official V1, tagged `v1.0` (local only).
- **Bug hunt (2026-11-01):** 55 bug fixes, committed as `24c82fb` on 2026-11-02. The full list,
  with what's left, is in `.bugfinder/REPORT.md`.
- **Optimization (/optimize, 2026-11-02), not committed:** 42 speedups that keep every picture and
  behaviour the same are in the working tree (28 rated "risk none", done while the user was away,
  plus 14 rated "risk low" that the user approved). One more was tried and reverted because it made
  start-up worse. The full list and measurements are in `.optimize/REPORT.md`, which also lists 8
  "needs your call" items that would change the picture (the biggest: render nebulae at SE's own
  lower volumetric resolution, ~3 ms of GPU time). Also uncommitted: a frame-time logger for
  measuring (`perf.rs`, the `perf` dev command; only active with `SE_PERF=FILE`).
- Next: the user playtests the faster build and says whether to commit; then maybe the "needs your
  call" items.

## Recently done

- **Optimization (2026-11-02, uncommitted):** measured side by side with the morning build
  (typical frame time):
  - RS planet ground, looking around: 13.0 → 5.6 ms (about 180 fps); the stutter every ~13 s (a
    lighting update that rebuilt every ground patch, frames of 45-60 ms) is gone (worst ~8 ms).
  - RS planet from 300 m 10.3 → 4.8 ms. Earth surface 13.8 → 5.7 ms. Mars 9.1 → 5.0 ms. Near Earth
    7.7 → 4.3 ms. Milky Way view 5.0 → 2.9 ms.
  - Flying fast through stars: 7.4 → 4.9 ms, hitches much smaller (worst 53 → 21 ms).
  - Inside nebulae: 6-16% faster.
  - Start-up: running about 1 s sooner; worst start-up frame 784 → 145 ms.
  - Pictures: identical on Earth and Mars, and elsewhere no more different than two runs of the
    old build. All tests pass (306 core, 201 engine).
  - Biggest causes: Bevy re-preparing thousands of star-field objects every frame because shared
    materials were touched every frame; Bevy's multi-draw path doing per-patch work that batched
    nothing; the nearest-star search scanning all 135,000 catalog stars every frame while flying.

- **Bug hunt (2026-11-01):** 11 hunters checked the whole port and 64 suspected bugs were
  verified (62 real). These are fixed:
  - **Stars and galaxies:** stars no longer drawn twice (Polaris, EPS Eri and about 360 others);
    hand-made galaxies no longer overwritten by the NGC list.
  - **Tutorials:** tutorial 1 no longer gets stuck at its "follow" step.
  - **Ships:** they keep up with their planet at high time rates and while paused; land on the
    real (flattened) ground; parked ships stay put when you visit another system.
  - **Screenshots and window:** wide hi-res screenshots no longer lose the ground at the edges;
    the window no longer grows each launch on scaled displays.
  - **Planets:** planets with no class (hot Jupiters) are drawn as giants; SE's temperature
    formula is used.
  - **Procedural universe:** far galaxies no longer give every star the same system; revisited
    multiple-star systems come out the same.
  - **Rotation and orbits:** Mercury's rotation is right; parabolic comets move at the right
    speed; moons with no RefPlane orbit in their planet's equator.
  - **Crashes:** a script line with non-English characters, an unbalanced script, an aurora with
    no radius, and a non-Vorbis music file no longer crash or hang the port.
  - **Settings:** closing mid-tutorial no longer saves the tutorial's settings over yours.
  - **Camera:** it no longer keeps flying while you type, or jumps when zooming inside a galaxy.
  - **Black holes:** they stay visible beyond 2 pc.
  - **Smaller fixes:** many more; see the report.
  - **Round 2:**
    - The star search lists the planets of stars you haven't visited.
    - Add-on atmospheres still being generated are attached when ready.
    - NGC 7185 no longer shares the Milky Way's object names.
    - Clusters stay right after the Milky Way's map loads late.
    - 13 tiny moons with no mass now orbit.
    - The surface generator uses SE's tilt (Earth's is now 20.9°, measured against the
      Earth-Moon orbit; not checked in SE itself).
    - Frame-time mode's clock no longer drifts.
    - A landed ship you left stays put on coarse ground.
    - Leftover terrain-generator data is cleared when you leave a system.
  - **Gas giant fix (user's video, 2026-11-02):** a fast-spinning (flattened) gas giant was
    mostly flat blue, with flickering triangles where its cloud bands met the blue, and a waffle
    grid from afar. The surface shader's haze step assumed a round planet and also took each
    triangle's slight sag for "under the ground". Fixed in `planet.wesl` (shader only; no rebuild
    needed). Checked on that planet and on Earth, Mars, Jupiter and Saturn.
  - **Aurora flicker fix (user's video, 2026-11-02):** auroras jumped in brightness every frame.
    Their flash timer ran on seconds where SE's runs on days since J2000 (86400 times too fast).
    Fixed in `aurora.rs`, with a test; checked steady over consecutive frames in the port.

- **Go to rotates with planets (2026-11-01):** Go to (G, and double G) now binds the camera as
  "Rotate with object" for planets, moons and other bodies, so their surface holds still under
  the camera; stars, galaxies and the rest are still followed. SE itself only follows ("Following
  Moon" in its tutorial); this is the user's choice. Checked at 3600x time on Mars: the camera
  stays at the same point in Mars's frame. Seen while testing: at very high time rates the info
  panel's distance wobbles (it samples a moving planet at a slightly different moment); harmless.

- **Info panel counts (2026-11-01):** the top-left info panel's "N children" row is replaced by
  SE's counts, each opening to a list: a star or star system shows "Number of planets",
  "Number of dwarf planets" and "Number of stars"; a planet shows "Number of moons". Counts look
  through barycentres (the Sun has 8 planets including Earth; Earth has 1 moon, Pluto 5) and skip
  asteroids, comets and spacecraft. Clicking those rows no longer also selects the star behind
  them (the panel is on egui's background layer, which egui does not count as interface).

- **Ground stops refining from the second landing on (2026-11-01):** each planet keeps up to
  1500 generated height grids; past that, the cleanup threw away every newly arrived grid whose
  terrain patch didn't exist yet, though the patch can only be built from it. So after a first
  landing filled the cache, new terrain under the camera could no longer be built: it stayed
  coarse, or flickered between sharp and blurry. Now a grid is kept for 10 s after it was last
  used. Checked with three fast landings on RS 8514-1828-5-27938-309 2: all reach the finest
  terrain and stay sharp. This is probably also behind the user's original "limit to how close
  I can get" report.

- **Nebulae (2026-10-31):** procedural nebulae are volumetric like SE's, nearer nebulae draw
  over farther ones, and nebulae no longer pop in and out. The user confirmed the look.
- **Ground up close (2026-11-01)**, for the user's two reports ("a limit to how close I can get"
  and "the surface fades into a different texture as I get closer"):
  - Ground textures no longer blur into blobs when the camera is a few metres up: a finer copy of
    the same texture fades in as it gets magnified, so the ground stays sharp down to 1.5 m.
  - The ground keeps its colour at every distance (it used to turn from orange to gold, with
    green and red specks, on the way down).
  - Close-range texture now also shows on coarser terrain patches, so while finer terrain is
    still loading the ground at your feet no longer looks like it is hundreds of metres away.
  - The near clipping plane is now always set before the frame's camera is built (it could stay
    11 m out on Eris and cut away the ground in front of the camera).
  - The terrain generator only falls back to the port's own heights when it truly fails, not
    when it is slow (e.g. window minimized), which used to give planets holes and height jumps.
  - Checked on Eris, the purple planet RS 238-21-2-44-404 6 (Draco Dwarf) and Earth; all tests
    pass.

## Notes

- The user's SpaceEngine has a graphics mod installed, so SE screenshots don't show vanilla
  SE's exact textures.
- Seen while comparing: SE has Eris's spot at lat -36.29 lon -0.65 near nightfall around
  2026-10-30 19:00, while the port still has it well lit then. Eris's day/night timing may be
  off in the port (not looked into).
