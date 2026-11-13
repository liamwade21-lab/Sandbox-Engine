# Progress

## Current state (2026-11-12)

- **The project moved to `C:\Users\liamw\Desktop\Sandbox Engine`** (2026-11-12, the user's: the version 2 folder had grown
  bloated). A git clone of version 2 (history included, its GitHub remote kept) plus `re/` and `.audit/work`; the build files
  (`target`, 4.8 GB) and test pictures stayed behind. The test scripts point here. Release zips go in `release/` (out of git):
  `release\Sandbox-Engine-1.1.0-windows.zip` is this state, made here.
- **The exe holds only code; everything else is in an `assets` folder beside it** (2026-11-12, the user's: the exe smaller and
  the big files in the game's folder, for every file that isn't SpaceEngine's). A player's download is now `Sandbox Engine\`
  with `spaceengine-rs.exe` (92 MB, was 197 MB with the assets built in) and `assets\` (shaders, textures, models,
  Megaton Rainfall's files, the mods, and Garry's Mod's packs in `gmod`); the zip is 621 MB. Nothing is unpacked to `%LOCALAPPDATA%` any more
  (`build.rs` and `embedded_assets.rs` are gone). The exe without its assets folder stops with a message saying where the
  folder belongs.
- **Garry's Mod's files ship with the game** (2026-11-12, the user's: only SpaceEngine is read from outside). Every file the
  sandbox uses (all 4,963 props of the stock spawn lists with their textures and collision, the weapons' models, the sound
  scripts and the 420 waves it plays, the spawn lists, the menu's 51 backgrounds shrunk to 1920x1200) is one pack,
  `crates/se_engine/assets/gmod/` (21,070 files, 1,084 MB deflated to 518 MB, in pieces of at most 90 MB by kind so GitHub
  takes them), read a file at a time from disk. Garry's
  Mod's install is never read while the game runs (only the pack tool `copy_garrys_mod` and the gmod tests read it); without
  the pack or with `SE_GMOD=0` the plain props and sounds remain. The pack tool reads everything back from the new pack and
  matched the install file for file.
- Both checked: a test run in the project (crate, drum, gravity gun, the Q menu's icons), `SE_GMOD=0`, and the release zip
  unpacked outside the project with empty APPDATA and LOCALAPPDATA (the pack, Megaton Rainfall's files, the mod and the
  shaders all read from its `assets` folder; nothing written to LOCALAPPDATA). A release build takes about 10 minutes again.
  Possible next step: a smaller pack (fewer props or smaller textures) if 518 MB is too big to share.
- **The gravity gun's and physics gun's beam no longer draws over the gun** (2026-11-12): the beam is a flat line on top of
  the picture, so it is now left out wherever it lies over the gun's own outline (the view model's triangles as posed this
  frame, `weapons.rs` `gun_outline`, `beam_start`): it appears where it comes out from behind the gun. Checked with the
  gravity gun; test pictures don't show the physics gun's beam even uncut (a capture quirk), so that one is yours to try.
- **Your three changes of 2026-11-12** (one commit each):
  1. **Omni-Man is silent in space** (your new D12): every sound of his fades out as the air thins (from 100 Pa down to 1 Pa) and
     is fully silent in orbit and on airless ground. Checked by the logs (every sound at 0 in orbit and on the Moon, Kansas as
     before) and by recording the speakers: nothing at all in orbit, his sounds as before in Earth's air.
     A sound already playing when he leaves the air (a boom, then up to orbit) fades out too (a follow-up commit, recorded).
  2. **No heat bar, no heat glow on Omni-Man:** the readout's Heat row is gone and his body no longer glows orange to white in fast
     flight; heat is still worked out (the roar, the plasma shell round him and the trail still follow it). Checked with pictures of
     a fast low flight before and after, the heat table, and the mod-off picture (unchanged). The plasma shell round him (from
     3,500 K) stays: your decision, 2026-11-12.
  3. **A main menu like Garry's Mod's** after the loading screen: one of your screenshots fills the screen and slowly zooms
     and changes (Garry's Mod's own backgrounds if you had none, plain dark without either), "Sandbox Engine" top left, Play,
     Mods, Options and Quit down the left (lit under the mouse, with Garry's Mod's hover and click sounds), a thin bar along
     the bottom. Esc while playing brings it back with Resume on top; Play then puts you back on Earth on foot. The world loads
     behind it, so Play is instant. Checked in test runs (every entry, Esc as walker, hero and in free flight, Esc with a
     ship flown still leaves flight, no Garry's Mod, no screenshots) and once as a player would: the game started normally,
     clicked and keyed with real Windows input (Play, walking, Esc, Esc held, Resume, Options, Quit).
     The independent check found map labels, orbits and the port's other windows drawing over the menu: now hidden while it
     is up and back after. Not done: the Settings window opens at its usual top-left place, over the menu's column; not
     tested: the window losing focus with the menu up, Play while a spacecraft is flown, how the sounds feel by ear.

- **The sandbox (2026-11-11): the game now starts on Earth's ground in first person, like Garry's Mod.**
  - **What works (all checked in the game):** it starts on foot in whichever of eight open places has the Sun highest
    (the Serengeti, the Pampas, Salisbury Plain, Kansas...). You walk, run, jump and duck at Garry's Mod's own speeds
    (200 / 400 / 100 / 60 units a second). Holding Q opens the spawn menu and letting go closes it: Garry's Mod's own spawn
    lists with a picture for every prop (4,963 props), a Weapons tab and a Mods tab. A click on a prop spawns it where you
    look; Z undoes it. Props fall under the planet's own gravity and rest on the ground (9.79 m/s2 on Earth, 1.63 on the
    Moon, where a jump goes 4 m up), you can push them by walking into them and stand on them. Six weapons with Half-Life
    2's sounds: crowbar, gravity gun, physics gun (carry, turn with E, freeze with the right button), pistol, SMG,
    shotgun. Half-Life 2's health and ammunition display. Clicking **Viltrumite powers** in the Mods tab plays as him;
    clicking **Yourself** comes back. V switches the old free flight on and off (and nothing else: photo mode is now B).
    A fall from 480 m lands on the ground and costs 10 health.
  - **Garry's Mod's files** are read from your install while the game runs (nothing copied, nothing changed there).
    Without Garry's Mod (a friend's PC) there are six plain props and plain sounds instead.
  - **Your second round (2026-11-11: "get the feel and sound, fix the weapon models, buildings/trees solid"), all
    checked in the game:**
    - **Weapons:** what you hold is now Half-Life 2's own first-person model with its hands, playing its own
      animations (draw, fire, reload, the shotgun's pump, the crowbar's swings, the gravity gun's prongs opening on
      what it can lift), with Half-Life 2's bob, sway and recoil and its firing rules. At night in a town the street
      lamps light it.
    - **Buildings and trees are solid:** walls and trunks stop you, bullets and props, and roofs hold you (checked in
      New York and in the Pampas). It uses the Viltrumite mod's list of nearby buildings, asked for round you. Found
      and fixed on the way: after a jump to another continent a town's near buildings and trees never appeared.
    - **Sound:** every sound is played as Half-Life 2's own sound scripts say (which recordings, how loud, what pitch),
      and the surface you walk on or hit decides the sound (grass, concrete in town, wood, metal). I recorded what the
      speakers play and measured it, which found two real faults, now fixed: sounds in the world came from the wrong
      ear (left and right were swapped), and the SMG's shots piled up to the loudness limit (in Half-Life 2 each shot
      cuts the last one off). Nothing clips at your volume setting; a barrel dropped on your right is louder on the
      right.
  - **Things not done:** props have no shadows; you walk on the sea; props more than 60 m away freeze until you come
    back; a tree is solid as its trunk only; no enemies, so the weapons only push props; no decals where bullets hit;
    without Garry's Mod the weapons are plain shapes.
  - **For you to judge:** how it feels in your hands. The rules and numbers are Valve's and the sound is measured,
    but only you can say whether it feels like Garry's Mod: the mouse's speed (its default), the eye height (Source's
    64 units: 1.22 m), how loud steps are next to guns (Half-Life 2's own ratio: steps are quiet).
- **Version 2 (this folder):** a copy of version 1 (`C:\Users\liamw\Desktop\Space Engine Recomp`)
  made on 2026-11-02, with its git history, for better visuals, add-ons and the like. It has its own
  settings folder (`%APPDATA%\SpaceEngine-rs-v2`). Still to decide: its own window title / exe name.
- **Sharing with friends (2026-11-07, fixed for other PCs 2026-11-09):** the first time it starts, it finds SpaceEngine
  through Steam (or asks for SpaceEngine.exe) and accepts it only if Steam installed it for an account that has signed in
  on that computer (`ownership.rs`; README, "The ownership check"). Every later start checks it again silently. The
  loading screen is only "Sandbox Engine Loading..." on black.
  - **Release 1.0.0 was black on other people's PCs:** the exe read its shaders, textures and models from this project
    folder, which only exists on this PC. **Since 1.0.1 everything the game needs except SpaceEngine is built into the
    exe** (shaders, textures, models, Megaton Rainfall's files, the Omni-Man mod: the user's decision of 2026-11-09). On
    a player's first start it unpacks them (about 115 MB) to `%LOCALAPPDATA%\SpaceEngine-rs-v2\assets`. The exe is
    197 MB; the download is a zip with just the exe in a `Sandbox Engine` folder, made by
    `bash .audit/tools/package_release.sh 1.0.1 OUT_FOLDER`. Tested 2026-11-09 from a fresh folder with blank AppData:
    unpacks, finds SpaceEngine through Steam, Earth draws, the mod loads. (Since 2026-11-12 the assets are a folder beside the exe instead: see the top.)
  - Released 2026-11-09 as GitHub Release v1.0.1 (`Sandbox-Engine-1.0.1-windows.zip`, 76 MB; the latest release),
    with the user's OK. The website's Download button is the other session's job.
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

- **Mods menu and Viltrumite powers** (the plan, from the user's interview of 2026-11-06: `docs/spec-mods-viltrumite-powers.md`,
  11 steps; what each step did, found and changed is in its Build log; the steps' longer notes in `docs/progress-archive.md`).
  - **Done (steps 1-9, and your controls):** the Mods button and window; Omni-Man drawn and lit like the ground, with a shadow and a
    cape on springs; flight to past light speed, real falling and orbits, walking, running, jumping, landings; sounds by the real air;
    the sonic boom and heating by real physics per planet; craters, dust, punches, the ground punch, the thunderclap, splashes; buildings
    are solid (he stands on roofs and slides along walls; the camera pulls in between them).
  - **Your controls (2026-11-07):** the ones of your own Viltrumite mod (README's Mods table): Space taps to take off or drop and holds
    to charge a launch (a full one breaks the sound barrier), W twice for fast flight and S twice back, W and S its speed, Ctrl dives,
    a full charge past the barrier adds a boost stack (x1.5 and a boom, up to 5), Shift and W twice a super run, the left button a combo
    and a charged punch, Ctrl+Space the super jump. The camera shakes and widens on every boom and launch; his sounds are twice as loud;
    the air rushes past in fast flight and the roar plays past the barrier; a line of keys, a cross and a charge bar are on screen.
    Checked over Kansas: a full ground charge broke the barrier with a boom, a full charge past it gave a boost stack and another boom.
  - **Step 10 (done):** buildings hit hard (35 m/s or more, or a punch) sink into the ground and leave rubble; they come back once he
    and the camera are 20 km away.
  - **Your second round (2026-11-07):** each boost stack now lifts the speed limit near planets with it (30 km/s in air, x1.5 a stack:
    228 km/s at 5); the wind is louder and follows his speed through the air in any flight (the roar file was four times quieter than
    the others: each loop now has its own gain), and a dive roars too; past the sound barrier the view buffets steadily (more with
    Mach and stacks) and faint speed lines run out from the middle of the screen.
  - **Your third round (2026-11-07): the lock-on and a new run.** In supersonic fast flight the left button selects what's under the
    cross (a planet or moon, or a star or galaxy through the game's own pick) instead of punching; G locks on (G again lets go); holding
    W flies him there, faster the further it is (to 10 million times light's speed), the view swinging round to it, easing in to stop
    in a hover 1.5 radii from a planet's middle (10 from a star's). Nothing more than 5 minutes away can be selected or locked on (about
    80 light years). A gold mark shows the target with its distance and time left. Checked: Earth to the Moon in about 10 s (from
    orbit, and from the ground climbing out over the horizon), Alpha Centauri in 34 s, Betelgeuse refused. The run and the sprint are
    new: leaning in, high knees, heel kicked up, a hop between steps, the arms pumping, the planted foot not sliding.
  - **Next:** step 11 (the rest of the spec's checks, the mod-off picture check, costs, notes).
  - **For you to judge (U3):** how it all feels and sounds; every number is in `crates/se_engine/src/viltrumite/tune.rs`. The walk, the
    run and sprint (now a solved running stride), turns and the first two punches are still stand-ins until you put the 8 Mixamo clips in `re/viltrumite/mixamo` and re-run the
    converter (U1); the mod author's note on reusing assets is still to read (U2).
  - **Known and left:** the uphill foot sinks on slopes over about 30 degrees (no foot IK); with the 3D clouds off his glow and trail
    show through flat clouds; in thin air at high speed the cape can stand up like a flag; deep in Jupiter with the mod off one shade in
    0.2 % of a nearly black picture differs (step 7's fix there).
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

- **Disk (2026-11-11):** building the sandbox left `target/agent` (the quick builds) at 9.5 GB and `target/debug` at
  1.4 GB. Both can go to the Recycle Bin whenever you like (CLAUDE.md, "Disk space"); they come back when next used.
- **Folder cleaned up again (2026-11-07):** from 24.8 GB to 4.1 GB. Old builds (`target/agent`, `target/debug`, unused
  files in `target/release`) and test pictures (`.optimize/scratch`, all but `baseline` and `baseline_noassets`,
  which the mod's step 11 compares against) went to the Recycle Bin. Emptying it is the user's step.
- **Folder cleaned up (2026-11-06):** from 16.7 GB to 2.8 GB. Old builds and test pictures went to the
  Recycle Bin; the source, the game's exe, the assets, the notes and the reverse-engineering folders stayed.
  `target/debug` and `target/agent` come back when next used. How to do it again: CLAUDE.md, "Disk space".

- The user's SpaceEngine has a graphics mod installed, so SE screenshots don't show vanilla
  SE's exact textures.
- Seen while comparing: SE has Eris's spot at lat -36.29 lon -0.65 near nightfall around
  2026-10-30 19:00, while the port still has it well lit then. Eris's day/night timing may be
  off in the port (not looked into).
