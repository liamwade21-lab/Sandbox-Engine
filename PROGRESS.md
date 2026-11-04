# Progress

## Current state (2026-11-04)

- **Version 2 (this folder):** a copy of version 1 (`C:\Users\liamw\Desktop\Space Engine Recomp`)
  made on 2026-11-02, including its git history and the uncommitted surface-speed work below, for
  better visuals, add-ons and the like. It has its own settings folder since 2026-11-02 (below).
  Still to decide: its own window title / exe name.
- **2026-11-04, the ground drawn faster (committed):** the ground's
  share of each frame was measured piece by piece at Ultra 3440x1440 (one piece switched off at a
  time in a test copy of the game). Two changes, built into `target\release`:
  - *The skirts.* Every terrain patch hangs short walls under its edges to close cracks to its
    neighbours. They were drawn together with their patch, and the patches are drawn in no
    particular distance order, so wherever a wall came before the neighbour that hides it, it was
    fully coloured and then covered: about a third of the ground's time near the surface. The walls
    are now drawn after all the ground, so their hidden parts are skipped before colouring.
  - *SE's colour noise on far patches.* Patches too coarse to carry the noise at their corners now
    carry its broad layers there, and each pixel works out only the fine ones (the same accuracy
    rule the near patches already use).
  - Ground's GPU time in ms, before -> after (one run of each with the release build):
    the user's planet RS 8514-3620-1-7-69 1 from 3,400 km / 114 km / 3 km / the ground:
    6.3 / 4.1 / 21.2 / 25.4 -> 4.7 / 3.1 / 14.2 / 16.4; RSN planet 2 at the horizon / looking down /
    looking around: 25.0 / 14.4 / 8.4 -> 13.5 / 5.6 / 5.5; the moon 5.1 from 346 / 72 / 30 / 13 / 7 km:
    8.4 / 9.9 / 9.9 / 6.6 / 7.6 -> 6.5 / 7.5 / 7.6 / 5.3 / 6.0, flying as in the user's video (72, 22,
    5 km up): 8.7 / 8.8 / 10.5 -> 6.2 / 6.2 / 7.3.
  - Pictures: cloud-free views from 30 to 3,400 km are the same to within one brightness level;
    near-ground views differ from before only as much as two runs of the old version differ from each
    other (clouds and patch loading vary a little between runs).
  - Still costly near the ground: the soils within 2 km (~7 ms), the sample layers (3-4 ms), the haze
    and the cloud shadows (~1.7 ms each); from high up, SE's climate noise per pixel on far patches.
    The same split could be done for airless worlds' colour noise (the user's RSN planet 4).
  - (Correction, found later the same day: the bench's "114 km" segment also holds the jump to 3 km
    and its first 600 frames. Before the fix below, the finest patches there were not drawn while
    their shader was being prepared, so it read low: 3.1 ms. It now reads about 10 ms, the real cost;
    measured at 114 km alone, both versions take 4.7 ms.)
- **2026-11-04, the ground missing after loading and on landing (fixed and committed; the
  user's video 10-40-37):**
  - *Why:* the graphics card's version of each ground shader variant is made the first time a patch
    needs it, and the patch is not drawn until then. The NVIDIA driver takes 5-6 s for each of the
    ground's 11 variants (the port's own part is 0.1 s). After loading, the planet was see-through for
    about 7 s; landing asked for the finer patches' variants and the ground vanished again for about
    10 s. (NVIDIA's own shader cache folder is full, 1.1 GB against its 1 GB default limit, so it
    rarely helps across starts.)
  - *Fix* (`pipeline_watch.rs`): all 11 variants are prepared in the background from the start, 3 at
    a time (all ready about 18 s after starting); a patch whose own variant is not ready yet draws with
    the plain per-pixel one (the same picture, a little slower) and switches when it is; the loading
    screen now stays until the saved place's planet is loaded and can be drawn (8.2 s in a test start
    from the user's saved place), not a fixed 3 s. se.log now names every shader that took over half
    a second and when the loading screen closed.
  - Checked: landing 30 frames after a jump to 3 km shows the ground; the speed at 114 km alone and on
    the ground is unchanged.
- **2026-11-04, thin clouds warping while moving (not found yet):** the user's video shows the thin
  cloud layers smearing and breaking into small squares while flying in and out (2,000-2,300 km up,
  122 fps, 0.011 c). Test runs at the user's settings (Ultra clouds, 3440x1440, solids 85% of 1080)
  flying in and out at the same speed, orbiting and turning, paused and with time running, against the
  reference clouds: the clouds stay as close to the reference while moving as when still, and a change
  to how the reused frames are kept in range made no measurable difference (not kept). The video's
  low bit rate (1.8 Mbit/s) adds blocks of its own. Waiting for more detail from the user.
- **In progress: volumetric clouds and weather.** The plan is `docs/spec-volumetric-clouds-weather.md`
  (2026-11-02): volumetric clouds and a hybrid weather system on all worlds with an atmosphere,
  gas giants included, plus rain/snow, lightning, fog and dust storms, in 10 steps. Steps 1 to 7 are
  done: on Ultra and High the planet you are at now has real volumetric clouds with ground shadows,
  weather that changes with the date, a live weather simulation, and rain, snow, lightning, fog and
  dust. Step 8 (gas giants) is done too, see below. The user allowed moving from step to step
  without asking (2026-11-02).
- **2026-11-03: the new clouds plan** (`docs/spec-clouds-v3.md`), from the user's videos of their
  planet RS 8514-3620-1-7-69 1 and a Flight Simulator 2020 video of the look they want; the user
  asked for the whole plan. Done: steps 1-6.
  - **Step 1, one cloud system:** the hand-over to SE's flat clouds is dropped (it caused the dark
    patches in the user's videos and made far views slower). SE's flat clouds only beyond 40-60 radii.
  - **Step 2, a quarter of the pixels a frame:** the far trace does one pixel of each 2x2 block a frame
    in turn; the blend fills in the rest (`SE_CLOUD_QUARTER=0` traces all). About 2.3x cheaper.
  - **Step 3, cheaper march:** long steps through clear air, steps growing with distance (at least 24
    over a ray), the light towards the sun cheaper (its far samples without the fine noise) and from
    20-40 km on worked out from the cloud above instead of marched. Two bugs found on the way and
    fixed: re-marching the same stretch hid whole decks; the low sun's estimate turned sunsets grey.
  - **Step 4, sharp from far:** the cloud map is twice as fine (2048 a face, from SE's tiles one level
    deeper, a thread a face); the detail patch is now a stack of three per layer (Earth 800, 1,600 and
    3,200 km across, 2048 texels, made from about 1,400 / 2,800 / 5,600 km down); far-away clouds are
    sharpened to full resolution by the cloud map in the composite (`cover_tau`). The weather's own
    cloud got a real pattern (a baked map, `weather_pattern`, shaped by `weather_shaped`): it made
    the honeycomb veil and the soft round blobs from orbit (the user's "splotchy"). The ground's cloud
    shadows use the same pattern. Earth from 400 km went from blurry blobs to crisp clouds; the
    user's planet from orbit is about as sharp as SE's flat clouds.
  - **Measured at Ultra 3440x1440 on the user's planet** (clouds' ms: start of the plan -> now): from
    3,400 km 14.5 -> 1.4, from 114 km 15.9 -> 2.4, 3 km up 11.0 -> 2.7, on the ground 11.2 -> 2.5.
    Planet 2 looking around: 2.6. The ground itself still costs 18-24 ms near the surface.
  - **2026-11-03, two bugs from the user's videos fixed (before step 5):**
    - *Ghosting while swinging round a planet:* clouds turned into blocky squares and hung out into
      space beside the planet's edge. A cloud-free spot stored its distance capped at 10,000 km, so
      from farther away the last frame's picture was moved to the wrong place and its clouds were
      kept next to the real ones. Now it stores the real distance, and a spot whose view misses the
      cloud layers is always clear. Checked orbiting the moon RS 8514-3840-6-8364-20 3.3 at Ultra and
      High and the user's planet from 3,000 and 300 km (new dev command `orbit DEG`, degrees a second).
    - *Ground shadows fading near the ground:* within 40 km of the spot under the camera the shadow
      comes from a shadow map traced through the clouds, farther out from the cloud map. The map let
      about 0.56 of the sunlight through a thick cloud, the cloud map 0.25, so a bright square of
      ground followed the camera (the V-shaped edge in the user's video). Both now give a quarter
      under thick cloud, and the far shadows of thin veils follow the sun's slant path as the map's
      do. Under an overcast the ground near the camera is now darker, as it already was farther away.
    - *The square was still there in the user's next video* (the moon RS 8514-3840-6-8364-20 3.3, its
      clouds 14-18 km up, the sun low): the shadow map took the sun's ray as rising over flat ground
      and stopped it after 40 km, so with a low sun the ray passed above the clouds that should shade
      the ground. Both shadows now follow the ray over the round planet. Checked from 171, 390 and
      22 km at the video's spot: no square. On the user's planet the map was still a little lighter
      than the far shadow under patchy cloud (it lets more light through than an even one), so near
      the camera the ground now takes the map's shapes at the far shadow's average darkness (the map
      got mips; its 10 km average is read from the last). The gap went from about 0.1-0.17 to a few
      hundredths.
    - *A line where the night side starts (the user's screenshot):* the shadows stopped where the sun
      sets on the ground, though the ground there is still lit. They now go on past it. Checked at the
      screenshot's spot from 1,058 km.
  - **Step 4c, blackrack's clouds studied (2026-11-03):** how his volumetric clouds for Kerbal Space
    Program work, written up in `docs/ksp-clouds-notes.md` (shapes, light, the light volume, how it
    reuses the last frames without ghosts, ground shadows, lightning, rain, costs) and folded into
    steps 5-7 of the plan. His C# is public (github.com/LGhassen/EnvironmentalVisualEnhancements); his
    shaders were read from the user's download, whose shader bundle holds them as readable OpenGL code.
    Everything taken from it is in `re/ksp_clouds/` (kept out of git; none of it goes into the port).
    The user's download is his Release 3 line (2023, with 2024 fixes): its light volume came later, in
    Release 4, so that part was learned from his newer C# and his wiki. Tools: UnityPy 1.25.4 (Python,
    from PyPI, already installed) to export the shaders, `gh` and `git` for the source and wiki; no
    decompiler was needed.
  - **Step 5, cloud shapes: done (2026-11-03).** The reference switch `SE_CLOUD_REFERENCE=1` (slow, for
    judging shapes and light); the saw-tooth edges of the weather's own clouds from far away fixed (they
    were cut off where the weather's share of cloud passed 2 %; they now fade out). Cumulus are now
    fields of separate heaps: a coarse noise lays out where heaps stand (SE's cloud map sets how much
    of the ground they cover: 90 % at full cover, gaps between them), each heap a dome with a flat base
    whose height follows its width (fair-weather cumulus about 0.6 times as tall as wide, taller with
    the weather's convection), lumpy sides and billowed tops. Cloud types: fair-weather cumulus,
    stratocumulus (a lumpy sheet with gaps), an overcast deck (a closed lumpy sheet), towering cumulus
    up to 4 km (real storm cells are step 7), and veils for high layers. Before, each of SE's cloud
    blobs (cover near 1 inside) was filled from the ground up: flat slabs with blurred edges.
    Problems found and fixed on the way:
    - far away the heaps were smaller than the cloud texels and turned into stripes (they now switch to
      their average sooner);
    - the shape noise repeats every 6 km and lined the gaps up in rows towards the horizon (a second,
      turned layout fixes it);
    - mixing the types by their density stood faint 8 km "ghost towers" over every heap (they now mix
      their heights);
    - stopping the billows far away made distant clouds fatter, a white wall at the horizon.
  - **Step 6, light: done (2026-11-03).** A light volume (blackrack's design): a 3D grid of 160 x 160 x 32
    over the cloud layers within 60 km of the spot under the camera, rebuilt 4 slices a frame (all of
    it every 8 frames). For each point it holds how much sunlight gets through the clouds above it,
    and how much cloud is straight overhead. The clouds take their long self-shadows from it (one heap
    shading the next) and the sky's light (bases and lower parts dimmer). Also: denser cumulus (as real
    ones; before they let too much light through to their bases), three bounce "octaves" for the soft
    inner glow, dense cores brighter than thin edges, a little of the ground's light on the bases, and
    shorter first light steps (40, 80, 180 m) for the billows' own shadows. The blend with the last
    frames now uses a sharper filter (moving clouds stay crisp) and ignores neighbours at a very
    different depth (clouds no longer bleed across mountain edges). Not taken from blackrack: keeping
    the see-through-ness only from the fresh frame (it would make glowing ghosts here instead), and
    storing the history squashed (not needed so far).
  - **Speed (Ultra 3440x1440, clouds' ms):** the near pass now traces a quarter of its pixels each frame
    at half size (it cost 15-23 ms inside Earth's low cumulus). Earth's central US cumulus from 400 km,
    12 km, 4.5 km, 1.5 km: 1.4 / 2.2 / 2.8 / 2.8 (17-25 ms before at the low ones). The Amazon's dense
    afternoon cumulus from 15 km, 2.5 km, 1.8 km, inside: 2.7 / 3.1 / 2.4 / 2.4. The user's planet from
    3,400 km, 114 km, 3 km, the ground: 1.8 / 3.0 / 3.4 / 3.4 (after step 4: 1.4 / 2.4 / 2.7 / 2.5). The
    march is built twice, with and without the cumulus code: for a body whose clouds are all veils
    (the user's planet) the code it never uses slowed it by a third.
  - **2026-11-04, the same cloud detail on every planet (user's request):** procedural planets had only
    thin veils. A planet's lowest layer became cumulus heaps only when it was under 15 km up, and SE
    puts its generated layers over the middle of the ground's relief: on dry worlds with 20 km of relief
    (the user's planet, planet 2) that middle is 10 km up, so their layers stood at 15-16 km. Heights now
    count from that middle (`Terrain::ground_mid_m`): every planet's lowest layer is cumulus heaps, as
    Earth's (at most 9 km deep; an opaque deck like Venus's stays a sheet). The user's planet now has
    fair-weather cumulus 10.7-17 km up, planet 2 too under its two high veils; Earth is unchanged.
    Clouds' ms at Ultra: the user's planet 1.2 / 1.7 / 1.8 / 1.7 (the veil was 1.8 / 3.0 / 3.4 / 3.4);
    planet 2 looking round 1.9 / 2.3 / 0.8 / 2.5 (was 1.4 / 2.2 / 0.7 / 2.6).
  - **2026-11-04, three fixes from the user's videos:**
    - *A flat band hid the far cloud tops* (video 1, zoomed in to a 10 degree view above the heaps):
      the far pass spread its 112 steps evenly along each ray, and a ray skimming the cloud layer from
      inside it is 150-300 km long, so its steps were 1.3-2.7 km even next to the camera and the heaps
      along it were averaged into a flat band. It was always there as a thin strip at the horizon;
      zooming in magnified it. The steps now grow with the distance (short near the camera, long far
      away, the same number). The zoomed view now matches the slow reference; ordinary views are
      unchanged (under one brightness level on average). Clouds' ms: the user's planet 1.17 / 1.72 /
      1.89 / 1.84 (was 1.2 / 1.7 / 1.8 / 1.7); planet 2 1.67 / 1.59 / 0.80 / 2.19 (was 1.9 / 2.3 / 0.8 /
      2.5).
    - *Night-side clouds pitch black* (video 2): the 3D clouds took only the main star's light, while
      the ground also takes the further lights (other stars, and the light moons and planets reflect,
      SE's planet shine) and SE's ambient light. The clouds now take them too (without a light march of
      their own: the cloud above dims them), with the moonlit haze in front of them; SE's flat clouds
      now take the ambient light too. Where the sun has set its light march is skipped, so the night
      side costs less. Checked on Earth under the full moon (2026-11-24) with auto exposure: the night
      clouds show, lit as by day for the moon's strength. They stay darker than SE's flat clouds at
      night (about a third): the 3D clouds are a little darker than a white surface by day too, where
      the bright end of the picture hides it.
    - *No lightning at night:* storms only formed in the afternoon; the flash picker took the strongest
      of 32 tries, half of them anywhere on the planet, so it nearly always chose the afternoon side;
      and its test read the broad convection, low at night even in a storm. Now there is convection at
      night (half the afternoon's at midnight, least at 9 a.m.), 6 of the 8 flash slots look only
      around the camera (300 km, or the camera's height), flashes go by the storms' own strength,
      strokes fade over 50 ms (was 30) and light the storm's top over about 20 km. Checked: flashes over
      the night side's tropics from 800 km (fewer than by day, as on Earth).
    - New test commands: `photo 0|1|2` (photo mode; 2 is auto exposure, as the user plays) and
      `ambient X` (SE's ambient light).
  - **2026-11-04, two more bugs from the user's videos (fixed, committed as 7ad5b7c):**
    - *Going into a gas giant you first went through its "floor"* (a hot giant, RS 8514-3620-0-0-70 2):
      under the cloud-top radius the flat bands' surface was still drawn, and from below only its tiles'
      edges showed, a red grid over the sky. It is now hidden while the camera is under it and the 3D
      decks draw the giant. The decks also lacked the giant's own heat glow (SE's flat bands glow on a hot
      giant): they turned from purple to navy as they faded in and were black inside. They now glow as
      the bands do at their tops (checked: 66,68,119 against the bands' 62,65,118 at 6 km) and hotter
      deeper down (red, then orange, then gold), and a glowing atmosphere's haze is in front of them.
      Cool giants such as Jupiter don't glow, so nothing changes there.
    - *Rings and clouds appearing from nowhere* on the user's moon RS 8514-916-6-8246-147 5.1 (heaps at
      12-21 km, veils at 90-137 km): the far trace spread its steps over the whole height from the lowest
      cloud to the highest, gaps included, so from between the layers (72 km) or above them its steps were
      kilometres long by the heaps: they showed as rings round the spot under the camera, and the heaps
      appeared only lower down. The steps now go over the cloud layers' heights alone and skip the gaps
      (each part from its own random start). The heaps now show from 72 km and through the veils.
      Clouds' ms at Ultra 3440x1440 on that moon: 2.0 / 1.9 / 1.9 / 2.4 / 2.4 / 2.3 from 4,782 / 346 /
      72 / 30 / 13 / 7 km (was 1.8 / 1.6 / 1.3 / 2.0 / 2.0 / 1.8); the user's planet 1.0 / 1.6 / 1.9 /
      1.7 (unchanged); Jupiter's decks 1.2-1.5 (was 1.0-1.4).
    - *The moon's slowness:* in test runs the same views run at 70-100 fps near the moon and 220 fps
      out at 8,000 km; the clouds are about 2 ms of a 10-14 ms frame there, the ground 6-11 ms. Both of
      the user's videos ran about 2.5 times slower than test runs of the same views (the giant at 80 fps
      against 200), so something on the PC during the recording slowed everything. Test runs now write
      `se-test.log`, so the user's own `se.log` stays for looking into such things.
  - **Next:** step 7 (storms built from parts, lightning without smears, rain tuned, final measurements
    and docs). Seen while testing: far clouds' edges keep a faint grain; the cumulus heaps are all about
    the same size; over low basins of high-relief worlds the heaps stand high (their base follows the
    ground's middle, not the ground below); from 25 km on the user's planet a plateau's far ground shows
    sharp zig-zag edges (the ground's, older: the veil hid it). (The net of dark lines in the user's
    planet's veil from 3 km went with the veil.)
    Seen while fixing the band: zoomed in, the clouds beyond about 100 km are still their smooth
    average (the slow reference too), with a faint straight line where the ground's horizon is.
  - **Still open:** the faint straight line over Alaska from 11,000 km; SE's flat clouds (40-60 radii)
    still draw the weather's cloud without its pattern; faint grey specks at some cloud edges.
  - **Test switches:** `SE_CLOUD_QUARTER=0`, `SE_CLOUD_LOD=0` (a hard switch, no fades),
    `SE_DETAIL_COVER=0`, `SE_NEAR_PASS=0`, `SE_WEATHER_NEUTRAL=1`, `SE_CLOUD_REFERENCE=1` (step 5's
    reference: every pixel traced every frame at full size with twice the steps, no history; slow),
    `SE_WEATHER_DEBUG=1` (a weather map at the top of the screen);
    dev command `vclouds 0|1|2` (the 3D clouds off, High, Ultra, for side-by-side pictures).
- **Step 8, gas giants: done (2026-11-04, the user asked for it while away).** Gas giants now have
  volumetric cloud decks near and below their cloud tops, on by default (`SE_GIANT_DECKS=0` turns them
  off for comparisons). From orbit they keep SE's flat look; the decks fade in within about 120 km of
  the cloud tops (they match the flat bands there, so the change is hard to see).
  - The decks: the main deck in the band colours, its zones standing higher than its belts, and a dark
    deep deck below; the light dims and reddens inside them and fades to black under the deep deck.
    The thin high haze is off (SE's atmosphere is the haze: from inside, the haze made a grey fog of
    SE's blue sky).
  - Colour and light matched to SE's flat bands: the belts were shaded as if under 10 km of cloud (the
    cloud above was counted from the deck's average top, and belt tops lie below it), the atmosphere's
    haze was left out and an Earth-cloud factor halved their light: Jupiter's deck was a dull olive,
    four to six times too dark. A catalog giant (Jupiter, Saturn) now takes its own colour map, composed
    from its tiles as the cloud maps are, its cloud height from its brightness (bright zones high);
    procedural giants keep SE's generator. Checked against the flat bands: Jupiter at 30 km 192,170,136
    against 192,169,136; your system's giant planet 3 the same within a few levels.
  - Saturn: the rings' shadow falls on the decks (and on any ringed planet's clouds), with SE's "rings
    winter" fading the shaded hemisphere, as on its flat surface.
  - The camera can go down through the decks (to 10 scale heights below the cloud tops; Jupiter about
    200 km): from 10 km over tan tops under a blue sky, into the main deck's orange fog, a gap between
    the decks at -50 km, the dark red deep deck, then black past the dark limit (-160 km on Jupiter). A
    hot giant glows instead, brighter with depth.
  - Lightning in the deep water clouds (giants have no weather maps): storm cells about 180 km across on
    Jupiter, the flashes some 50 km down lighting the deck above them over about 50 km.
  - Cost at Ultra 3440x1440 on Jupiter: 1.0-1.4 ms (about 200 fps there).
  - Not done: the decks' own storms (SE's spots and cyclones are only in their colour); the camera's
    near plane under a giant's radius is not tuned (nothing wrong seen).
- **Known gaps of the volumetric clouds so far** (later steps): HD screenshots, skybox export and the
  projection modes still show the flat clouds (step 9); a
  few dark specks remain at some cloud edges seen from orbit; the detailed ground shadows reach about
  40 km around you (beyond that a gentler average). A full overcast looks smooth and featureless from
  low orbit (400 km) and from just above it, and Earth's cloud map is soft up close (SE's map is about
  10 km a pixel): to tune in step 10. The storm clusters may be a little too widespread, and the
  storm-track storms are too evenly spaced and alike (seen on the weather map); lightning seen from
  orbit is only a pinprick; rain near the camera falls straight down whatever the wind (tuning). The
  live weather's storm-track storms (`se_core` weather) on Jupiter-sized worlds took 32 ms to map; gas
  giants now skip the weather maps (SE's own bands and storms stand in). Since
  step 5 the log shows one or two lines "RenderMesh ... cannot be extracted" while the galaxies load
  (probably a galaxy's sprite re-sort skipped once; nothing visible found; cause not traced yet).
- **Your planet 2 spot is now clear-skied:** with the weather, the place of your video (25.5 N,
  48.2 W) lies in a dry subtropical belt, so its sky is clear where step 4 showed clouds overhead.
  Clouds are in the wet belts and storm tracks (e.g. 52 N there has an overcast).
- **Camera oddity seen in tests (not from the clouds):** the `surface` dev command sometimes leaves the
  camera 1 m above the sea instead of at the height asked for (the info panel then shows a negative
  height above the surface). Not traced yet.
- The port runs and covers most of SpaceEngine; see README.md "Status" and
  `docs/missing_features.md` for what is left.
- **V1:** `main` at 97094fd is the port's official V1, tagged `v1.0` (local only).
- **Bug hunt (2026-11-01):** 55 bug fixes, committed as `24c82fb` on 2026-11-02. The full list,
  with what's left, is in `.bugfinder/REPORT.md`.
- **Optimization (/optimize, 2026-11-02):** committed as `083e5d7`. 42 speedups that keep every picture and
  behaviour the same are in (28 rated "risk none", done while the user was away,
  plus 14 rated "risk low" that the user approved). One more was tried and reverted because it made
  start-up worse. The full list and measurements are in `.optimize/REPORT.md`, which also lists 8
  "needs your call" items that would change the picture (the biggest: render nebulae at SE's own
  lower volumetric resolution, ~3 ms of GPU time). Also added: a frame-time logger for measuring
  (`perf.rs`, the `perf` dev command; only active with `SE_PERF=FILE`).
- **Surface FPS fix (user's video, 2026-11-02 afternoon, not committed):** on procedural planets
  (airless and Earth-like) the ground's colour noise is now worked out per terrain vertex where the
  terrain is fine enough, instead of per pixel. On the user's planet at their settings: horizon
  30 → 44 fps, looking down 67 → 120 fps; on an Earth-like planet looking down 56 → 107 fps.
  Also added the `preset NAME` dev command for measuring at the user's Ultra settings.
- **Clouds (tried, reverted, 2026-11-02):** on Earth-like procedural planets the cloud layer is the
  biggest cost left (~17 ms at the horizon, ~24 ms looking up at Ultra 3440x1440), nearly all of it
  SE's warped cloud noise per pixel. The per-vertex trick does not fit it: every generated layer has
  relief, whose lighting comes from the density's fine per-pixel slopes, and vertex values changed
  that lighting on ~97% of cloud pixels (by over 1/64 on a third). Reverted. Options that would change
  the look, for the user's call: bake the clouds into per-patch textures as SE itself does, or draw
  them at a lower resolution.
- Next: the clouds plan's steps 5-7 (`docs/spec-clouds-v3.md`), then the gas giants. Other big costs
  left on planet surfaces at Ultra 3440x1440: the ground itself (14-16 ms near the surface on the
  user's planet since 2026-11-04, was 21-25), the volumetric nebula when a planet sits in one, and the biome detail textures with
  4x MSAA.

## Recently done

- **Weather effects (2026-11-03, clouds plan step 7):** rain, snow, lightning, fog and dust, each with
  its own switch (Settings > Graphics > Clouds and weather).
  - Rain and snow: grey shafts under raining clouds (steady rain under a deck, showers in shafts,
    slanted by the wind), and around you, when under a raining cloud, falling streaks of rain or
    snowflakes; snow where the air is below freezing (from the planet's temperature, the latitude and
    the height). Checked: snow falling near Antarctica, the sea's horizon greying out in rain.
  - Lightning: in thunderstorms, flashes of three quick strokes every few seconds, lighting the
    clouds from inside and the ground around them; they stop when paused.
  - Fog: on calm, humid nights and mornings, mostly over the cool subtropical seas, in patches.
    Checked: a morning sea fog off California with the sun diffused through it.
  - Dust: dry, windy worlds lift regional dust storms that drift with the winds (Mars shows big
    butterscotch veils over its surface). Planets with an atmosphere but no cloud layers now get the
    effects too.
  - Cost at your settings (Ultra, 3440x1440): planet 2's clouds and effects 0.4-2.2 ms (about 0.2 ms
    more than before), frames unchanged; Earth in falling snow 1.1 ms at the horizon, 3.7 ms looking
    up into the snowing clouds (frames 10 and 5 ms).
  - Test tools: `SE_WEATHER_DEBUG=1` shows a weather map at the top of the screen; the dev command
    `switch KEY on|off` turns a render switch on or off.
  - Tests pass (315 core, 209 engine), with a new one for fog and dust.

- **Live weather on the graphics card (2026-11-03, clouds plan step 6):** the planet you are at now
  has a small weather simulation running on the graphics card (a grid of 6 x 128 x 128 cells over the
  planet), on top of the date's weather.
  - What it does: thunderstorm clusters start in the afternoon where the weather allows them, grow,
    tower up, rain, leave cooled air behind that keeps new storms away for a while, spread anvils that
    linger into the evening, and die out at night; everything is carried by the winds. From orbit an
    afternoon over a warm ocean gains big bright storm systems.
  - The date's weather now blends smoothly between maps 10 simulated minutes apart (no more jumps
    every 10 minutes), and its winds now also blow north-south (the trade winds meet at the equator)
    and swirl a little.
  - Near you the clouds drift with the local wind (checked: 8 m/s from the east on Earth's trades).
  - Time rules: up to x1000 and within 20 planet radii it runs live; faster, farther or with time
    running backwards it follows the date's weather (blending over a fraction of a second); paused
    it stands still (two paused runs give the same picture: on average 0.02 out of 255 apart); a date
    jump shows the new date's weather at once (checked against a fresh start at that date).
  - The far ground shadows now follow the weather too.
  - Cost at your settings (Ultra, 3440x1440, planet 2): the clouds take 0.7 ms at the horizon, 0.8 ms
    looking up, 0.2 ms at the ground, 2.0 ms looking around; frames 30, 19, 10 and 20 ms (same as
    before). High: clouds 0.1-0.6 ms, frames unchanged.
  - New dev commands for tests: `timing frame` / `timing real`, `resume`. `SE_WEATHER_SIM=0` turns the
    simulation's storms off for comparisons.
  - Tests pass (314 core, 209 engine), new ones for the simulation's clock and the winds.

- **Weather that changes with the date (2026-11-03, clouds plan step 5):** the clouds of the planet
  you are at now follow a weather pattern worked out from its climate and the date, on top of SE's
  own cloud layers.
  - The climate comes from the planet: its day length (from the catalog; slow spinners get one wide
    belt per hemisphere, fast ones up to 7), how wet it is (its ocean), whether it is tidally locked.
  - The pattern: cloudy and clear latitude belts that move north and south with the seasons, storms
    that drift east along the storm tracks and live about 5 days (with a comma-shaped spiral), towers
    that grow in the afternoon, and patches that slowly change and drift with the wind.
  - It is a small map (256x128) made again every 10 simulated minutes on a worker thread; on arrival
    at a planet or after a jump of more than a day it is made at once (2.4 ms on your planet 2, 4.5 ms
    on Earth) so the new weather shows in the same frame.
  - Checked: Earth at the same spot on Nov 2 and Nov 4 shows different clouds while its cloud map's
    look stays recognisable; after three hours at x3600 the clouds have moved on.
  - Cost at your settings (Ultra, 3440x1440, planet 2): the clouds take 0.9-1.7 ms (horizon 0.86,
    looking up 1.05, at the ground 0.18, looking around 1.72); High 0.1-0.5 ms. Frames: horizon 30 ms,
    looking up 19 ms, looking around 21 ms.
  - Tests pass (313 core, 207 engine), with 6 new ones for the weather.

- **Volumetric clouds on solid worlds (2026-11-03, clouds plan step 4):** on Ultra (half resolution)
  and High (quarter), the planet you are at draws its SE cloud layers as 3D clouds; Medium and below
  keep the flat clouds. Your game now shows them (the step 3 test switch is gone).
  - Where the clouds are: a cover map per planet (a cube of six 1024x1024 faces, a channel per layer)
    built in about half a second on arrival, from SE's own cloud generator for procedural planets
    (each texel averages 3x3 samples, as the flat clouds' fine wisps average out per pixel) or from
    the catalog cloud map for Earth and the like (its tiles read from the install). SE's layer drift
    turns them as before. The flat clouds' shader was split so both use the same generator code
    (checked: the flat clouds are unchanged, at most 2 levels out of 255 on a procedural planet).
  - What they look like: the lowest layer stands for the clouds of the lower air (Earth: 1.1 to 9 km),
    cumulus about 1 km deep under a thin cover, towers only under a dense one, flat sheets under a full
    overcast; higher layers are thin veils. Seen from afar the clouds are exactly as opaque as the flat
    ones (Earth and your planet 2 from orbit match their flat cover); up close they break into clouds
    with gaps. Lit by the star through the atmosphere (sunset colours at the terminator), by the sky,
    with light passing diffusely through thick clouds (an overcast's base is grey, not black), and with
    the haze in front of them.
  - Ground shadows: traced through the clouds themselves in an 80 km square around you each frame,
    with the light that still gets through an overcast; farther out a gentler average from the cover.
  - Cost at your settings (Ultra, 3440x1440, planet 2 of your system): the clouds take 1.7 ms at the
    horizon, 2.6 ms looking up, 3.2 ms looking around (budget 12 ms); High 0.4-0.8 ms (budget 6 ms).
    Frames are faster than with the flat clouds: horizon 43 -> 32 ms, looking up 45 -> 22 ms, looking
    around 45 -> 24 ms.
  - Tests pass (308 core, 207 engine), with new ones for the layer heights.
  - Lessons (the user caught both): a screenshot file reused between runs kept the old picture (the
    test script now deletes it first), and after a shader edit the game needs time to load fully
    before a picture means anything (a milky picture without ground was the ground shader still
    compiling). Both are in CLAUDE.md.

- **Volumetric clouds, drawing skeleton (2026-11-02, clouds plan step 3):** new `volumetric_clouds.rs`
  and `volumetric_clouds.wesl`. Four passes after the main picture is drawn: the scene's distance at
  the cloud resolution, the ray-march through the cloud layer, blending with the previous frames, and a
  depth-aware blend into the picture. Ultra traces at half the solids' resolution, High at a quarter;
  Medium and below keep today's flat clouds. The cloud noise (two tiling 3D noise volumes, new
  `se_core/src/cloud_noise.rs`) is built in about 0.3 s on worker threads the first time it is needed.
  - For now it draws only one fixed test layer (1.5 to 6 km up), and only with env `SE_CLOUDS_TEST=1`
    (`SE_CLOUDS_COVER=0..1` sets its coverage). Without it the game is as before: you keep today's
    clouds until step 4 draws the real ones. While it draws a body, that body's flat cloud shells are
    hidden; until the cloud shaders are ready, the shells stay.
  - Checked: Earth from orbit and from the ground at Ultra and High (3D cumulus-like clouds on the
    horizon); with the test switch on, cloudless scenes (the Moon) and Earth at Medium differ from
    switch-off runs no more than two identical runs differ. New tests: the reprojection maths that
    carries the clouds across the floating origin and the planet's turn, the buffer sizes, the noise
    volumes. All tests pass (308 core, 206 engine).
  - Cost (Ultra, 3440x1440): the cloud passes themselves 0.3-0.6 ms, High about 0.1 ms (timed as
    `volumetric_clouds` in `SE_PERF`). Whole frames are faster than with today's flat clouds because the
    shells are gone: Earth horizon 23.4 -> 14.1 ms (half covered); planet 2 of your system horizon 43 ->
    30 ms, looking up 45 -> 19 ms, looking around 45 -> 20 ms. The test layer is cheap partly because it
    is simple; the real clouds of step 4 will cost more.
  - Bug found and fixed on the way: wgpu's default for a render pass clears the target, which wiped
    the picture under the clouds (black sky); the blend pass now loads it.

- **Cloud and weather settings (2026-11-02, clouds plan step 2):** the Settings window's Graphics tab
  has a new "Clouds and weather" group: Clouds (the same switch as Shift+C), Volumetric clouds (Off,
  Quarter resolution, Half resolution), Rain and snow, Lightning, Fog and haze, Dust storms.
  - The graphics presets set Volumetric clouds: Ultra half resolution, High quarter, Medium, Low and
    Potato off (today's flat clouds). Changing it by hand makes the preset "Custom", like the other
    quality settings. Your saved settings (Ultra) had no such key yet, so they get half resolution.
  - Saved in settings.cfg as `VolumetricClouds` (with the graphics keys) and `ShowRain`,
    `ShowLightning`, `ShowFog`, `ShowDust` (with the other switches, on by default). SE scripts can
    read and set them too (`Set ShowRain false`).
  - Nothing is drawn by them yet: the volumetric clouds come in step 3, the weather effects in step 7.
  - Checked: the settings window shows half resolution at Ultra and Off at Medium; two runs with a
    scratch settings folder showed the switches and the cloud setting are written and read back. Your
    own settings file was not touched. Tests pass (306 core, 204 engine, a new one for the presets).
  - Frame times unchanged (your planet and planet 2 at Ultra 3440x1440, within run-to-run noise).

- **Version 2's own settings folder (2026-11-02, clouds plan step 1):** version 2 now keeps its
  settings, log, saved session, places, ships, tutorial progress and atmosphere cache in
  `%APPDATA%\SpaceEngine-rs-v2`; version 1 keeps `%APPDATA%\SpaceEngine-rs`.
  - On its first run version 2 copies version 1's folder (only if version 2's doesn't exist yet). That
    first run was a test run on 2026-11-02, so version 2 starts with your settings, session and ships
    as they were then. From now on changes in one version no longer reach the other.
  - Checked: version 1's files unchanged byte for byte, the copy identical, no second copy on the next
    run. If a copy ever fails, version 2 starts with default settings and says so in its log.
  - Screenshots still go to `Pictures\SpaceEngine-rs` for both versions.
  - New test for the copy; all tests pass (306 core, 203 engine). Frame times unchanged (your planet,
    Ultra at 3440x1440: horizon 22.8 ms, looking up 16.6, looking down 8.3, looking around 12.2).
  - Baseline for the cloud work, planet 2 of your system (the Earth-like one), Ultra 3440x1440, today's
    flat clouds: horizon 43 ms (23 fps), looking up 45 ms, looking around 45 ms; the see-through pass
    (clouds, sky, nebula) takes 24-37 ms of GPU time of that; looking down 8.5 ms. Rerun with the
    computer idle: the same (the first run's 16.6 ms looking down and 124 ms hitches came from other
    use of the computer). `.optimize/bench/rsn_ultra.sh` now takes the planet number as its fifth
    argument.

- **Clean-up (2026-11-02, user's request):** deleted about 108 GB of leftovers: the 10 agent worktrees
  (their changes checked to be in `main` first), `.optimize/scratch` (traces, saved exes, benchmark
  outputs), the profiling and agent builds (`target/trace`, `target/agent`), `.bugfinder/scratch`,
  the old test screenshots and logs in `.audit/work` (its decompile cache and text extracts kept),
  `shots/` and `water_vs_se.jpg`. Kept: `target/release`, `target/debug`, `re/` (Ghidra project and
  SE's shaders). The leftover `worktree-agent-*` git branches were left in place (no files).

- **Surface FPS on Earth-like procedural planets (2026-11-02):** the same per-vertex trick for SE's
  terra colour generator (rocky worlds with a real atmosphere). Tested on RSN
  0-7-460129-1254-197-12 2 (in the user's system), Ultra at 3440x1440.
  - Its planet-wide noises (latitude, albedo variation, humidity on plant worlds; features 10+ km)
    go to the vertices on nodes up to ~20 km across; the climate distortion's ridges (features
    ~160 m on that planet) only on the nearest nodes; metre-scale detail stays per pixel. Plant
    worlds also stop computing one warp twice (12 noises a pixel, identical result).
  - Picture: ground colour terms within 1/1024 everywhere tested except 0.004-0.007% of pixels,
    none off by a full 8-bit step. Earth and Mars shots byte-identical (Earth uses its own designed
    biomes, not SE's generator).
  - Result: ground GPU time at the horizon 30.8 → 18.7 ms, looking down 16.4 → 8.1 ms (whole frame
    17.9 → 9.4 ms, 56 → 107 fps). The horizon frame only went 57.5 → 43.1 ms: that planet's cloud
    layer costs ~17 ms there (~24 ms looking up), the next big cost. Tests pass (202 engine).

- **Surface FPS on the user's planet (2026-11-02, user's video):** RSN 0-7-460129-1254-197-12 4,
  played on Ultra at 3440x1440. The drops to 25-30 fps came when looking towards the horizon.
  - Why: at Ultra on that screen the GPU shades 2.5 times the pixels of the test runs, with 4x MSAA.
    Half the ground's cost was one step, the airless worlds' planet-wide colour noise (43 noise
    evaluations per pixel), though its features are kilometres across. The rest of the frame is the
    biome detail textures (~7 ms) and, looking up, the volumetric nebula the planet sits in
    (~5 ms at the horizon, ~11 ms looking up).
  - Fix: terrain nodes fine enough (cells at most 1/16 of the noise's finest feature) work it out at
    their vertices and the pixels interpolate it, in a second version of the ground shader; coarser
    nodes and other planets are unchanged. The colour differs by under 1/16384 (measured per pixel),
    far below what the screen shows; Earth and Mars shots are byte-identical; the earlier RS test
    planet changes by a fraction of one colour level on under 1% of pixels.
  - Result (median frame time, the user's spot and settings): horizon 33.5 → 22.9 ms
    (30 → 44 fps), looking down 14.9 → 8.4 ms (67 → 120 fps), looking around 15.6 → 12.3 ms,
    looking up 17.3 → 16.6 ms. At 1080p High the RS test planet's ground went 6.0 → 5.3 ms.
    No holes when the new shader version is first compiled (two descents checked). Tests pass
    (202 engine).

- **Optimization (2026-11-02, `083e5d7`):** measured side by side with the morning build
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
