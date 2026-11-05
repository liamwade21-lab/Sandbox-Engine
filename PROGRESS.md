# Progress

## Current state (2026-11-05)

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
  - **Step 2 done (2026-11-04): the GPU placement system ("scatter").** Trees (for now MR's tree_0,
    evenly, as a test: `SE_SCATTER_TEST=1` or the dev command `scatter test`) stand exactly on the
    drawn ground of any planet, Earth's flattened shape too, as the ground's detail changes; real
    meshes near the camera, baked pictures (impostors) to 3 km, lit and hazed like the ground (the
    ground's lighting is now a shared shader module; the ground looks the same as before). Cost at
    Ultra 3440x1440 with the test cover: 0.1-0.6 ms; GPU objects stay flat over a session.
  - **Step 3 done (2026-11-04): real vegetation.** Planets whose life includes plants on land (Earth,
    and SE's generated terra worlds with multicellular land life) get forests, copses, lone trees and
    grass, on by default (`scatter off` / `SE_SCATTER=0` to turn it off). Earth's forests come from
    its own map (dark green = forest; fields, steppe, deserts and ice told apart by colour); SE's
    worlds use the ground's own biome rules, so trees stand where the ground is drawn as forest. The
    trees take the ground's colour, the forest floor darkens under them, and past 3 km the ground
    draws the treetops instead, so a forest looks the same colour from orbit to the ground. Grass
    within 120 m, trees and grass sway in the weather's wind. Cost at Ultra 3440x1440 (with 4x MSAA):
    about 1.4-1.7 ms. Left open: Earth's taiga and tropics have broadleaf trees until step 4; the
    treetop pattern is faint under Earth's haze from 3-5 km; no Vegetation setting yet.
  - **Step 4 done (2026-11-04): the tree generator.** Trees are grown from recipes (six crown shapes,
    Megaton Rainfall's bark and leaves, two leaf cards made with the image generator). Earth now has
    spruce, pine and fir in its cold forests (the taiga, mountains), palms on tropical coasts, acacias
    and shrubs on the steppe, MR's broadleaf elsewhere. Each generated planet with land plants gets
    its own 8 species, shaped by its gravity (low: tall and thin; high: squat) and its air (dense:
    wide crowns), coloured by its ground: five such planets near the Sun all look different. The trees
    appear about a second after arriving (their pictures are made then). Cost: 0.6-1.1 ms.
  - **Step 5 done (2026-11-04): civilizations exist.** About 1 in 20 generated planets with land plants
    now carries an intelligent civilization: level 1 primitive, 2 modern, 3 futuristic or 4
    interstellar (about 45 / 15 / 20 / 20 %); Earth is level 2. Nothing else about any planet changed.
    The info panel says so ("Organic multicellular intelligent, Level 2 civilization (modern)") and the
    star browser can search for them (Life type "Intelligent", a Civilization filter). Nothing is drawn
    for them yet (steps 6-10). Test planets: level 1 WISE 19101+56342 3, level 2 2MASSI
    J1029216+162652 2 (better: WISEPA J090649.36+473538.6 1), level 3 2MASSW J1338261+414034 4.3, level 4 SDSS J212413.88+010000.2 1 (all
    within 70 light-years). Calibration: 20,000 generated systems, 1 in 20.1.
  - **Step 6 done (2026-11-05): civilizations seen from orbit.** A generated planet's civilization
    now has towns, cities and villages spread over its land (flat, low, fertile land near the coast
    first), each people with its own taste (its lights' colour, its towns' colour, how much it likes
    the coast). By night their lights show from orbit: level 1's fires are too dim to see from space,
    level 2 looks like Earth at night (clusters of town lights and a glow round them), level 3 has
    bright megacities, level 4 whole continents of light. By day the towns and a patchwork of fields
    colour the ground (hard to judge yet: every test planet nearby orbits a deep-red dwarf star).
    Earth is unchanged (its own maps). A better level 2 test planet is WISEPA J090649.36+473538.6 1
    (night side round 35 N 130 E): the first one has light on every side. Cost: 0.04-0.16 ms at
    3440x1440 (part of the ground's pixel).
  - **Step 7 done (2026-11-05): cities up close.** Towns now have streets, sidewalks, crossings
    and lots drawn in the ground, rural roads through the farmland, and buildings: houses in gardens,
    blocks, and towers in the business cores, with windows that light up at night; streets glow at
    night near the ground, with lamps along them and traffic (cars by day, head and tail lights by
    night). Earth's cities come from its lights map (New York looks like a city from 80 m to 10 km, by
    day and night). All buildings show within 7 km, the tall ones to 20 km (the skyline). Cost over
    New York's core: about 0.6 ms. Test switches: `SE_CIV_TEST=2` (a civilization on generated life worlds), `SE_CITY_TEST=1`
    (towns everywhere they can stand). Facades use Megaton Rainfall's own window tiles. Checked:
    flights into Tokyo and New York, the level 2 test planet's towns by night from orbit. Left for
    later: street furniture and 3D cars, a generated planet's town up close.
  - **Step 8 done (2026-11-05): primitive villages.** Level 1 civilizations live in villages of round
    thatched huts and longhouses (mud brick, timber or stone walls) on yards of packed earth, with
    wandering dirt paths and tracks between villages; by night hearth fires glow before the huts. The
    textures were made with the image generator. A temple stands in many villages and watchtowers
    on their edges (made in step 9).
  - **Step 9 done (2026-11-05): futuristic cities.** Level 3 cities have megatowers of 150-1,500 m in
    their business cores (tapering, stepped, twisted and round), skybridges between them, a few
    arcologies (stepped pyramids) and a landmark tower, flying traffic in two layers of lanes (streams
    of lights from afar), neon signs on the lower storeys, light strips and lit windows by night. The
    facades and signs were made with the image generator; the temple, watchtower and landmark were
    modelled in Blender by script (its 3D generators are switched off: they need an API key or the
    Premium plan). Costs about 1.6 ms over a dense core at the user's settings. Not done: elevated
    roads, 3D flying cars close up.
  - **Step 10 next:** level 4: orbital stations, colonized moons and planets, space traffic.

## Notes

- The user's SpaceEngine has a graphics mod installed, so SE screenshots don't show vanilla
  SE's exact textures.
- Seen while comparing: SE has Eris's spot at lat -36.29 lon -0.65 near nightfall around
  2026-10-30 19:00, while the port still has it well lit then. Eris's day/night timing may be
  off in the port (not looked into).
