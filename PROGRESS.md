# Progress

## Current state (2026-11-04)

- **Version 2 (this folder):** a copy of version 1 (`C:\Users\liamw\Desktop\Space Engine Recomp`)
  made on 2026-11-02, with its git history, for better visuals, add-ons and the like. It has its own
  settings folder (`%APPDATA%\SpaceEngine-rs-v2`). Still to decide: its own window title / exe name.
- **Volumetric clouds and weather: mostly done (the user, 2026-11-04).** On High and Ultra the planet
  you are at has 3D clouds at every height (SE's flat clouds only beyond 40-60 radii, cross-faded):
  cumulus heaps and veils in SE's own cloud patterns, ground shadows, weather that changes with the date
  and a live weather simulation, rain, snow, lightning, fog and dust. Gas giants have volumetric decks
  under their cloud tops (hot ones glow). Plans: `docs/spec-volumetric-clouds-weather.md` (the first,
  10 steps) and `docs/spec-clouds-v3.md` (the second, from the user's Flight Simulator reference: steps
  1-6 and the gas giants done). How it works and what must not be undone: CLAUDE.md's gotchas. The
  step-by-step history and measurements: `docs/progress-archive.md`.
  - Cost at Ultra 3440x1440: 1-2.5 ms on the user's planets, 1.0-1.5 ms in Jupiter's decks.
  - Left for later: HD screenshots, skybox export and the projection modes still show SE's flat clouds
    (first plan, step 9); tuning (step 10): a full overcast is smooth from low orbit, Earth's cloud map
    is soft up close (SE's map is about 10 km a pixel), storm-track storms are too evenly spaced and
    alike, lightning from orbit is a pinprick, rain near the camera falls straight down whatever the
    wind, a few dark specks at some cloud edges from orbit; storms built from parts (second plan, step
    7); gas giants' own storms (SE's spots and cyclones are only in their colour). The detailed ground
    shadows reach about 40 km round the camera (a gentler average beyond).
- **Speed (Ultra 3440x1440, the user's settings are Custom: no MSAA, solids 85% of 1080):** the ground
  is the biggest cost near a surface (14-16 ms on the user's planet RS 8514-3620-1-7-69 1, was 21-25);
  then the volumetric nebula when a planet sits in one, and the biome detail textures with 4x MSAA.
  Long sessions no longer slow down without end (2026-11-04: at most 4,000 terrain nodes and 1 GB of
  SE's tile textures kept; a planet's view settles at 11-12 ms against 9 ms fresh). The ground's
  shaders are built at start and the loading screen waits for the first view. `.optimize/REPORT.md`
  lists 8 "needs your call" speedups that would change the picture.
- **Open oddities:** the `surface` dev command sometimes leaves the camera 1 m above the sea instead of
  at the height asked for; one or two log lines "RenderMesh ... cannot be extracted" while the galaxies
  load (nothing visible found); the user's planet 2 spot (25.5 N, 48.2 W) is clear-skied now (a dry
  belt in the weather), which is expected.
- **Earlier milestones:** V1 is `main` at 97094fd, tagged `v1.0` (local only); the 2026-11-01 bug hunt
  (55 fixes, `24c82fb`, list in `.bugfinder/REPORT.md`); the 2026-11-02 optimization (`083e5d7`, details
  in `.optimize/REPORT.md`). The port covers most of SpaceEngine: README.md "Status" and
  `docs/missing_features.md` list what is left.

## Next

- **Vegetation and intelligent civilizations** (the plan, from the user's interview of 2026-11-04:
  `docs/spec-vegetation-civilizations.md`). Grass, bushes and forests on planets whose life and climate
  allow them, from orbit to the ground; a new life class, multicellular intelligent life (about 1 in 20
  planets with land life; Earth is one), at four levels: primitive villages, modern cities like Earth's,
  futuristic cities, interstellar (stations, colonized moons, space traffic). Budget about 3 ms at Ultra.
  It reuses Megaton Rainfall's trees, grass, building parts, roads and cars, read from the user's copy
  of that game, built step by step (the plan's Steps table; what each step learned is in its "Build
  log"). Research notes: `docs/megaton-rainfall-vegetation.md`, `docs/megaton-rainfall-cities.md`.
  - **Step 1 done (2026-11-04): the port reads Megaton Rainfall's files** from the user's copy at run
    time: its trees, grass, building parts, props and cars (549 of its 557 models; only animated
    people are left out) and all 587 of its textures. Nothing is copied into the project. Without the
    game the port runs as before and se.log says so.
  - **Step 2 next:** the GPU placement system ("scatter"), first tested with one MR tree scattered
    evenly over a planet.

## Notes

- The user's SpaceEngine has a graphics mod installed, so SE screenshots don't show vanilla
  SE's exact textures.
- Seen while comparing: SE has Eris's spot at lat -36.29 lon -0.65 near nightfall around
  2026-10-30 19:00, while the port still has it well lit then. Eris's day/night timing may be
  off in the port (not looked into).
