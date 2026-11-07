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
- **Roads on steep ground (2026-11-06):** streets and arterials are cut level across into hillsides, with
  banks either side and the buildings and trees following. Picture checked on a hilly planet; Earth's
  map data is too smooth to show it. Accepted: objects may be off by up to 2 m at 1-2 km on the first
  levelled level, and road middles by up to 0.4 m near a cube face's corners.
- **Street furniture and 3D cars (2026-11-06):** within 250 m at Ultra (150 High, 100 Low) towns of level 2
  and up have Megaton Rainfall's lamps, benches, bins, hydrants, traffic lights and 14 kinds of car, driving
  the ground's own traffic with their lights on at night. Left as it is: cars pass through each other at
  junctions and ignore the traffic lights (their places are a hash of the clock, which is why they cost
  almost nothing), country roads keep the flat painted cars, level 3's future cities get the same modern
  furniture and cars, and domed colonies keep the flat ones. Very steep test towns show road markings
  stretched down the banks between roads.
- **Speed (Ultra 3440x1440, the user's settings are Custom: no MSAA, solids 85% of 1080):** the ground
  is the biggest cost near a surface (14-16 ms on the user's planet RS 8514-3620-1-7-69 1, was 21-25);
  then the volumetric nebula when a planet sits in one, and the biome detail textures with 4x MSAA.
  The slowdown only a restart cleared is fixed (2026-11-06, the user's planet RS 8514-495-4-971-146 2
  fell from 80 to 40 fps): wgpu's work per pass grows with the most GPU buffers and textures ever alive
  at once, and exploring reached 44,600 buffers. Now about 10,200 and 5,300 textures (one uniform per
  ground material, flat cloud shells only where they are drawn, a tighter node cache). Measured with roads,
  street furniture and cars all in (5 rounds of trips, `session_explore.sh`): the main thread stays at
  2.6-3.3 ms on that planet, 5.2-5.8 ms over New York (it reached 9.9 ms before) and 6.1-6.4 ms at street
  level; the peak stays at 10,205 buffers. The street objects cost nothing measurable (15.6-16.3 ms with,
  15.7 without). Still open: the graphics card's time on that planet reads 8.7 ms or 10.2 ms for the same
  view (not explained yet), memory still grows round by round, and the render graph's time rises from
  3.5 to about 6 ms over the first round. A patch to wgpu-core that could give some of that back (a
  side session's idea, judged safe by reading the code, not built) waits on the user's yes. The ground's
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

- **Mods menu and Viltrumite powers** (the plan, from the user's interview of 2026-11-06:
  `docs/spec-mods-viltrumite-powers.md`, 11 steps; what each did is in its Build log). A new Mods menu; its first mod
  is a playable third-person Omni-Man: flight to past light speed, real gravity when falling, walking, sonic boom and
  aerodynamic heating from real physics, craters, and buildings that collapse when hit. The user still has to
  download 8 Mixamo clips (stand-ins work until then) and read the mod author's note on reusing its assets.
  - **Step 1 done (2026-11-06): the Mods menu.** A **Mods** button at the end of the tools toolbar opens a window
    listing the mods found in `assets/mods`. Viltrumite powers is there and switches on and off, but does nothing yet.
    A row is grey with the reason when a file is missing or bad, when it needs a newer build, while a ship is piloted
    or a script runs; starting either turns the mod off first. A script's HideAllDialogs closes the window. Checked in test
    runs: the button and the row clicked, grey rows, ships, scripts, `SE_MODS=0`.
  - **Step 2 done (2026-11-06): his files are in the project.** A one-time converter (`re/viltrumite/tools/convert.py`, outside git) reads the Omni-Man pack and
    the Superman mod's zips, and refuses by itself to open the mod's compiled code, GTA's archives or the blood suit. It wrote 52 MB into `assets/mods/viltrumite`
    (estimate 50): his model (225,011 triangles, 98 bones plus 30 for the cape), 45 animations under their real names, 18 textures, 29 sounds (real Ogg files made by
    Blender's own encoder; the loops trimmed to 10 s and closed without a click), the crater and rock models, the smoke sprite. Checked: he posed by 21 real clips from those
    files alone, front and side, looks right, stands on his feet, and the cape hangs (`re/viltrumite/sheets/contact_step2.png`); when first turned on the game looks inside every file, and a damaged file
    greys the row (never a crash), and sounds really play in a test run. **You can't see or hear any of it in the game yet** (step 3 draws him, step 6 plays the sounds).
    Eight animations are simple stand-ins until you download the Mixamo clips (walks and turns hand-keyed, the punches are the mod's own): drop the files in
    `re/viltrumite/mixamo` and re-run the converter (tested with made-up files Blender wrote). A review then corrected the leaning and falling animations (they would have come and gone in play), gave the idle back its breath and stopped a thumb spinning on take-off. Not judged: how the sounds sound, how the stand-ins feel, and the real Mixamo files are still untried.
  - **Step 3 done (2026-11-06): you can see him.** Turn the mod on and Omni-Man hovers 6 m in front of the camera, breathing (the camera stays free: he can't move until step 4). He takes exactly the same light as the trees and buildings: checked by eye beside trees by day, under the full moon, under cloud, at dusk, in a town at night, in orbit with the sun behind and in front, in space, and at your 3440x1440. The ground and trees were proven unchanged first, pixel for pixel. He costs nothing measurable on your card (under 0.1 ms). A review then fixed: he now stays with the camera however it is bound, also at x1000 (he used to fly off at once unless it was bound to the planet); clouds below him no longer shade him at high latitudes; street lamps light him only where the ground draws a town of streets (not over the sea or in a village); he no longer vanishes inside a gas giant's cloud decks; he shows in hi-res screenshots; and moonlit pictures now show him beside the trees, in the same ratio as by day. The ground, trees and clouds are proven unchanged with the mod off. Known and left: no shadow on the ground yet (step 5), the cape is a stiff sheet until step 5, far from any star he is black, in a town at night he and the cars are brightly lit while the road stays almost black (the road was already so), under dense trees he stays in full sun (the game has no tree shadows), by moonlight he looks flatly lit (as the trees do), and deep in a giant's decks he stays brighter than the cloud around him. 
  - **Step 4 done (2026-11-06): you can fly him.** Turn the mod on (Mods button, click the row): he appears where the camera looks, seen from behind, the panels go away and a small readout shows speed, Mach, height, gravity. Mouse looks, WASD 20 m/s, Shift boosts (300 m/s after 1 s, 3 km/s after 3 s, 30 km/s after 6 s; in space on past light speed, 1000 c after 30 s), Space and Ctrl up and down, X stops flying and he falls under real gravity or stays in orbit, Space stops the fall and he hovers, Z stops dead, the wheel moves the camera, Tab brings the panels back (and turning off happens there: Mods button, click the row). G goes to the selection and carries him. He flies the same at x1000 and paused. Checked by numbers: crossing between his two kinds of frame near a planet moves him under a millimetre more than his speed explains, flying and falling, at x1 and x1000, on Earth and on Jupiter; hovering stays within a millimetre through minutes of game time at x1000 and a date jump; an orbit stayed the same ellipse for 18 periods; at 128 km/s into the Moon he stops 1 m above the ground, never under it. Real keys and mouse sent to the game's window: Tab, the cursor lock, losing and regaining the focus, typing in a text field all behave. With the mod off the keys and panels are as before. **Not done yet, on purpose:** the ground only stops him just above it (step 5 brings walking and landings), no sound (6), no boom or heat (7). **Not judged:** how any of it feels: every number is in `viltrumite/tune.rs`. A review then fixed: Shift+G and Ctrl+G cancelled themselves (they work now, and only WASD or Z stop a flight); turning on at x1000 threw him hundreds of km against the planet (689 km up instead of 100; now exact, and a Go To starts without a lurch); the whole view could barrel-roll while hovering (up to 85 degrees over 2.4 days at x1000 with the bug back in, none now); flying low up a slope he crawled at about 1 m a frame while the speed read km/s (now the whole step); falling into a gas giant the air was measured from 200 km too deep, so 1,600 m/s where it should be about 200 (now 203-233 m/s, the same with clouds on and off); at 2 c a slanting fall could pass straight through a planet; Find with an empty field killed his keys under a locked cursor; a sudden stop no longer jumps the camera; the Ships button goes with the panels and the row really shows Loading. 
  - **Step 5 done (2026-11-06): you can walk.** Fly him down (Ctrl) or stop flying (X) and he lands: under 12 m/s straight on his feet, up to 35 m/s a short stop, faster a hard landing (no crater yet, that's step 8). W, A, S, D walk at 1.8 m/s, Shift runs at 5, held 1.5 s sprints at 11; he stands on the ground as it's drawn, up slopes to 50 degrees, sliding down steeper ones, walls over 70, off ledges he falls. Space jumps: 40 m on Earth, 241 m on the Moon by the log, six times as high; Space again in the air and he hovers. Standing, his body turns to where you look, with a turn animation past 70 degrees. He has a real shadow now, falling the way the sun says, fading as he climbs (gone at 200 m), under cloud, at dusk and at night, and it takes the sky's blue; a real cape on springs that swings as he walks, streams back in flight and can't explode. The ground's little stones hold still round his feet so they neither sink nor float. Over the sea he stops 1.5 m up and hovers; gas giants, stars and comet models only stop him. Checked by pictures: Earth's forest and the Sahara, the Moon by day and night, New York's street, a very steep hillside, his shadow at noon, low sun, dusk, night, at your 3440x1440, flying, x1000. With the mod off the ground is proven unchanged (a generated planet's ground bit for bit, the forest and Mars only the HUD's digits). **Known and left:** on slopes steeper than about 30 degrees the uphill foot sinks into the ground (no foot IK: a non-goal); walks, runs and turns are still the stand-in animations until you download the Mixamo clips; **Not judged:** how any of it feels, every number is in `viltrumite/tune.rs`. A review then fixed: flying into the ground at 12 m/s or more is now a landing or a crash without Ctrl (at 3 km/s he used to slide along it at 3 km/s), hitting a steep hillside counts as hitting it; walls over 70 degrees are real now, he falls past them and crashes at the bottom of a long one (he rode them down at 25 m/s); walking off a ledge no longer throws him 7.75 m on Earth's equator; no landing in mid-air where the ground hasn't loaded; a finer piece of ground rising past the camera no longer drops him out of whatever he was doing into a hover; his shadow on the sea now shows the sea (it left a flat hole with no waves or glint), starts at his boots on uneven ground, and the little stones hold still along it, fading as he climbs instead of switching at 12 m; the cape no longer blows out sideways when the game clock runs fast, and swings the same at any frame rate. Checked by pictures and numbers against the build before. The wall was only seen in a test copy that called 40 degrees a wall (the test planet's steepest ground is 69), and at the spots looked at the stones are too small to break the shadow either way.
  - **Step 6 done (2026-11-07): he has sound.** Hovering you hear a low hum. Flying, the air going past gets louder and higher with real air pressure and speed: faint at 20 m/s on Earth, nothing but the hum at 20 m/s on Mars, full at 3 km/s on Earth. Past about 2,500 K of heating a second layer comes in, and the right button plays a charge sound. There are one-shots for take off, Shift in a hover, the boost on and off, leaving top speed, a jump, a sprint's jump, X, a hard landing, and the same one low-pitched on the sea. In space you get the hum, and one-shots at a quarter of their volume. Losing the window's focus fades and pauses everything, and it carries on from there; x1000 and paused sound the same. With the mod off no sound of his exists. The boom, punches, thunderclap and breaking have their sounds ready for steps 7, 8 and 10. **Your correction (2026-11-07):** the three "wind" sounds were his breath power, not air going past, so they are gone from the project and a test fails if one comes back. `fastFlight_loop`, pitched low at low speed, is now the air going past; take off is a quiet `fastFlight_start`; X is `sentry_flightfx_loop_end`, newly converted. **Checked by logged numbers only, since no test can listen:** every level follows the table on Earth, Mars, the Moon and in orbit, each one-shot fires once per trigger at 20, 60 and 240 frames a second, with keys held or mashed, at most 8 at a time, nothing stays after 31 times on and off, and every file decodes. **Not run:** a computer with no audio device (tested without real audio output only). **Please listen for:** does the low-pitched loop read as wind at 20-100 m/s, or as something slowed down, and is it loud enough; can you hear the heat layer come in (it's very quiet in its file); is the take-off whoosh too much for a tap of W; is the sound on X right; does the low-pitched explosion pass as a splash; is the right button too thick when its swell and loop play together; is space quiet enough, and is the hum right while he stands or walks on the Moon. The numbers are in `viltrumite/tune.rs`. A review then fixed: falling, orbiting, standing and walking in vacuum were fully silent (you chose "quiet, not silent"), and now keep the hum, while real air stays as it was; the whoosh for leaving top speed also played during plain falls, and now only when a flight really leaves top speed, at X too; holding Ctrl on a gas giant's floor or a star's stop replayed the take-off whoosh every 0.75 s (now he just stays there until Ctrl is let go); during a Go To or Shift+G the air was silent and the readout said 0, and now both go by the real speed through the air, so a landing from orbit roars for about 3 s (please listen: is that too much?). Checked by logged numbers on the Moon, in orbit, on Earth, on Jupiter's floor and at the Sun. 
  - **Step 7 done (2026-11-07): the sonic boom and the heat.** Past each planet's own speed of sound (Earth about 335 m/s low down, Mars 233, Venus 449) you get the boom's sounds and a ring of cloud; a cone of vapour hangs round him up to Mach 1.6, lit by the sun, so nearly gone at night; slowing back under Mach 0.92 plays the ending sound and a puff of the cone, no boom. At most one boom in 3 s. Above the air (55 km on Earth, 16 on Mars, 188 on Venus by the game's numbers) there's none, so a dive from orbit makes none on the way in, nor does turning on at speed; on Pluto or a 1 mbar air there's never one. His skin heats by the real formula: dull orange at 800 K, yellow 2,000, white with a shell of plasma 3,500, a blue-white trail 6,000, looks clamped at 10,000 K, then only the shell and the trail grow. The heat bar in the readout is filled, in the colour of the heat. **Checked by numbers:** at the table's speeds in the game's real air, Earth at the sea and 60 km, Mars and Venus at the ground come within 7 % of the table, 1 mbar's air within 9.1 % (Mars 21 km up; no body in the solar system has 1 mbar, Pluto has 1 Pa and was flown too). **Checked by pictures,** from behind and the side: every stage on Earth low and at 60 km, Mars and Venus by day and night, Pluto, the cone, the ring, dives from orbit by day and night, in clear air and over cloud, in photo 0, 1 and 2 (no jump in the exposure, never dark), your 3440x1440 with Ultra clouds, 4x MSAA. **Cost:** 0.01 ms of the graphics card when hot at your size, nothing when cold or off. **With the mod off** the ground, towns, trees and Mars are unchanged (bit for bit, or only the HUD's digits), Jupiter's floor by 1 level in 0.2 % of a nearly black picture. **Your calls on what was left open:** a Go To through the air booms and heats him too, by its real speed (D9, real physics; the look is clamped at 10,000 K anyway): a landing from orbit (Shift+G) peaks at about 26,000 K, white for about 3 s, then he stands cooling for about 2 s; a plain G from near the ground leaves the air at hundreds of km/s, so he flashes white-hot and fades out over about 2 s on his way up (please look: too much?). The trail lies behind the camera when you fly straight, so from straight behind you don't see it; you do in any turn, or from the side. Catalogs' Jupiter and the other giants really have no air pressure in the game's files, so they take 1 atm at the radius. **Fixed on the way:** deep in a giant he no longer stays brighter than the cloud round him (his light now falls as the decks' does), and at Jupiter's floor he's no longer hidden: the game fades the whole view to black down there on purpose, which covered him too; it now leaves out what's right by the camera, and a faint light keeps him just visible in the black. Titan's air now has Titan's gravity (it was ending 8 times too low). **Not judged, stills and logs only:** how any of it moves: the speed of the flames and the ring, the trail's points dying, the sound by ear; a Go To's pictures in mid-flight. **Seen, and left:** clouds overhead get blocky edges when flying under them at several km/s (the build before this plan does it too); in thin air and at high speed his cape can stand up off his back like a flag (step 5's cloth, worth a look); his cape crosses the shell as a dark shape. A review then fixed: the cone of vapour hung round him in near vacuum (up to 200 km on Earth, on Pluto, in a 1 mbar air), and now shows only where the boom's air is; the trail vanished all at once a second after he slowed, and now each part fades in turn; it's as long at any frame rate (at 165 Hz its tail was cut off while bright), and seen end-on it no longer makes a bright spot; at x1000 a plain G from low over a planet counted the planet's spin a thousand times over (465 km/s over Earth's equator), and now reads as at x1; deep in a gas giant the air now thickens and warms with depth, as it really does (on Jupiter he falls at 63 m/s 100 km down instead of 190); the 30 km/s cap on a Go To's heat is gone. Left as it is: with the 3D clouds off, his shell and trail show through flat clouds in front of them. **Next: step 8**, craters, dust, punches, the thunderclap. Every number is in `viltrumite/tune.rs`.
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
  - **Step 10 done (2026-11-05): interstellar civilizations in space.** A level 4 civilization has
    colonized many of its system's moons and planets: level 3 towns, under glass domes where there is
    no air. Orbital stations (a spinning ring station and a truss station, modelled in Blender by
    script) circle its home world and its big moons, lit by the stars; streams of ship lights arc
    between the home world, its stations and its colonies. The dev command `station N KM` looks at a
    station. Nothing of it costs anything outside such a system, and nothing measurable inside.
  - **The port no longer needs Megaton Rainfall (2026-11-05, the user's decision).** The files it uses
    (its 16 trees, their bark and leaves, the grass, the window tiles: 47 files, 51 MB) are copied into
    `crates/se_engine/assets/mr` and read from there; the game's folder only when that copy is missing.
    Checked with the game's folder pointed nowhere: trees, cities, stations all there. The copy is for
    private use only, not for sharing the port.
  - **Step 11 done (2026-11-05): polish.** Two new graphics settings, **Vegetation** and **Cities**
    (Off / Low / High / Ultra), in Settings > Graphics > "Vegetation and cities" and in every preset
    (Ultra on Ultra, High on High, Low on Medium and Low, Off on Potato). Ultra is what all the step
    pictures showed; High draws trees to 2.2 km and grass to 80 m, Low trees to 1.5 km and no grass;
    Cities High and Low draw the buildings less far. Off leaves the trees (or the buildings, stations
    and space traffic) out; the towns' streets and lights in the ground stay. Settings saved before
    these keys (the user's Custom ones) get Ultra. Broadleaf trees now follow the seasons by latitude
    and date: autumn colours in October, bare in winter, a fresh light green in spring (the tropics
    stay green; conifers and palms stay green). Trees near snow take snow on their tops. Measured at
    the user's screen size, every view is inside the plan's budget (numbers below in the plan).
    **Not done:** roads flattening steep ground (it changes the ground's heights, which trees and
    buildings stand on: a careful job of its own; San Francisco's hills look fine without it), street
    furniture and 3D cars up close, the ground's own forest colour does not change with the seasons
    (the trees fade back to it from 1.5 km, so a forest seen from high up stays green).
  - **The vegetation and civilizations plan is finished** (all 11 steps). Next: whatever the user
    picks; the open items above are the natural follow-ups.

## Notes

- **Folder cleaned up (2026-11-06):** from 16.7 GB to 2.8 GB. Old builds and test pictures went to the
  Recycle Bin; the source, the game's exe, the assets, the notes and the reverse-engineering folders stayed.
  `target/debug` and `target/agent` come back when next used. How to do it again: CLAUDE.md, "Disk space".

- The user's SpaceEngine has a graphics mod installed, so SE screenshots don't show vanilla
  SE's exact textures.
- Seen while comparing: SE has Eris's spot at lat -36.29 lon -0.65 near nightfall around
  2026-10-30 19:00, while the port still has it well lit then. Eris's day/night timing may be
  off in the port (not looked into).
