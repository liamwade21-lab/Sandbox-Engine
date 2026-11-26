# Progress archive

Finished work's detailed notes, moved out of PROGRESS.md (read only when needed).

## Hand-over results (2026-11-03; the hand-over was dropped later that day)

- **Done 2026-11-03: no popping, sharper clouds** (`docs/spec-cloud-lod-and-popping.md`, from the
  user's two videos; the step-by-step notes are in `docs/progress-archive.md`). On High and Ultra:
  - **From orbit:** SE's own sharp flat clouds draw the clouds, shaped by the same weather. Down from
    15 to 5 times the highest cloud top (Earth: 138 km to 46 km) the 3D clouds fade in. Every switch
    fades instead of popping.
  - **Below that:** a detailed cloud map around the camera (about 800 m a texel) keeps SE's shapes.
    The nearest 4 km of cloud is traced at full resolution on Ultra (2 km at half on High).
  - **Fixed along the way:**
    - The torrid world that flipped: its moon left the planet compressed, and its clouds lost.
    - Earth's beige overcast from orbit: the weather was too strong.
    - Dust and fog veils.
    - The weather squares.
    - A hard twilight line.
    - SE's 5% cloud cut applied twice.
  - **Speed at Ultra 3440x1440:** from orbit the clouds dropped from 8.6 to 9.9 ms to 0.34 ms
    (frames 15-17 down to 8-9 ms). On planet 2 at your spot the views now cost 2.6, 3.9, 0.64 and
    5.5 ms (frames 32, 21, 11 and 24). After step 5 they cost 0.93, 0.94, 0.35 and 1.44 ms, because
    the weather had wrongly cleared that sky. Now SE's three thin high cloud layers show there again,
    and thin wide cover is the most costly kind to draw. The near pass is 0.05 to 0.26 ms of it.
  - **Not shown in a test picture:** the near pass's effect. In still pictures the frame blend
    already sharpens both versions; it should show when flying fast past nearby clouds.
  - **Still open:**
    - From 11,000 km a faint straight line crosses Earth's clouds over Alaska; it is fixed to the
      planet, cause not found. It's now only seen in orbit views where SE's flat clouds draw anyway.
    - The 3D clouds still look a little more solid and cream than SE's flat ones in the hand-over.
  - **Test switches:** `SE_CLOUD_LOD=0` (the old hard switch), `SE_DETAIL_COVER=0`,
    `SE_NEAR_PASS=0`, `SE_WEATHER_NEUTRAL=1`.

## No popping, sharper clouds (2026-11-03)

- **Next, before step 8: no popping, sharper clouds** (`docs/spec-cloud-lod-and-popping.md`,
  written with the user on 2026-11-03 from their two videos). What it fixes:
  - **Popping:** the clouds jump between SE's flat look and the volumetric look at 60 radii and when
    the volumetric clouds finish loading.
  - **The tan veil:** dust and fog cover Earth from orbit.
  - **The flipping world:** a torrid world (RS 8514-3151-2-61-189 4) flips into an opaque pale-blue
    ball at about 3 radii.
  - **Weather squares:** about 100 km wide, seen from orbit.

  The plan, on High and Ultra only:
  - SE's sharp flat clouds, shaped by the weather and relit, draw the clouds from orbit.
  - They blend into the volumetric clouds between 150 and 50 km up (on Earth).
  - Near the surface, the nearest few km of clouds are traced at a higher resolution.
  - The weather grid doubles (256 a face).
  - Everything fades instead of switching; dust and fog show from space only as real storms.
  - Cost: no more than today.

  7 steps. **Step 1 done (2026-11-03): the flipping world.**
  - **The cause:** the flip happened whenever its moon "4.2" was the nearest body (the readout at the
    bottom right). The port then draws farther bodies "compressed" so they fit the depth buffer. The
    volumetric clouds read that compressed depth as real distance, so they stopped short of the
    planet's clouds and drew nothing, while the flat clouds were hidden. That left the bare dark
    ground. Earth seen from the Moon had the same problem.
  - **The fix:** the body the volumetric clouds draw is now always kept uncompressed.
  - **Checked:** a flight from the moon to the planet keeps its clouds the whole way. Tests pass.
    Cloud and frame times on planet 2 are unchanged (clouds 0.95, 0.99, 0.41 and 2.33 ms; frames
    32, 18, 12 and 20 ms).
  - **The pale-blue ball is the right look:** this world has two full-cover cloud decks, 39 to 49 km
    and 71 to 80 km up, like Venus. SE's own flat clouds (Medium preset) show the same ball.

  **Step 2 done (2026-11-03): the tan veil.**
  - **The real cause:** not dust or fog but the clouds themselves. Turning the date's weather off
    (`SE_WEATHER_NEUTRAL=1`, new) gave a blue Earth from 2,483 km. With it on, the storm track turned
    SE's sparse cloud map into a beige overcast.
  - **Fixes:**
    - The weather now reshapes SE's clouds gently: storms thicken the clouds already there rather
      than cover clear sky.
    - The volumetric clouds drop cloud-map values under 5%, as SE's flat clouds do. Those had made a
      faint textured haze over clear ocean.
    - The cloud noise is averaged out from orbit, where it showed as a fine honeycomb.
    - Fog only forms banks (about 3% of Earth at a time), and dust only regional storms (about a
      tenth of a dry world).
  - **Checked:** Earth from the video's spot is blue with clouds again. Mars shows tan dust-storm
    patches, and is clear without them. Tests pass. Clouds got a little cheaper: 0.84, 0.92, 0.33
    and 1.64 ms (frames 29, 18, 10 and 20 ms).
  - **Still different from SE's flat clouds:** seen from orbit, the volumetric clouds are more opaque
    and cream-coloured; SE's are thinner and blue-grey. Step 4 matches the looks, and above 150 km
    step 5 shows SE's flat clouds anyway.
  - **Not ours:** the stair-stepped edge along Mars's limb at High is there with the clouds pass
    off, so it comes from the planet's own drawing.

  **Step 3 done (2026-11-03): no weather squares.**
  - **Weather grid:** the simulation now has 256 cells a face (about 40 km on Earth), and its storm
    cells have soft edges. The big squares from your video were mostly those cells, already toned
    down in step 2.
  - **Earth's cloud copy:** the volumetric clouds' copy of SE's cloud map now comes from the finer
    level-3 tiles, averaged down (takes about 1 s on a worker thread).
  - **Twilight:** the clouds' sunlight fades in over about 3 degrees above their horizon, instead of
    over about 30 km, and the orange band at the terminator is dimmer.
  - **Checked:** Earth from 11,300 km has no squares. Tests pass. Clouds 0.95, 0.95, 0.34 and 1.68 ms
    (about 0.1 ms more on the horizon view for the finer grid); frames 30, 18, 10 and 20 ms.
  - **Found, still open:**
    - **The checker:** the leftover checker is SE's real open-cell clouds (honeycomb stratocumulus in
      its map). The volumetric clouds make each faint cell solid and bright, where SE's flat clouds
      keep them faint. That's for step 4.
    - **A seam:** from 11,000 km over Alaska there's a faint straight line across the clouds. It is
      fixed to the planet and in the cloud density, cause not found. Check whether it still shows
      below 150 km once step 5 is in.
  - **The user (2026-11-03):** the flat far look is only wanted if the far clouds still look
    splotchy and low-res. They still are softer than SE's flat clouds, so steps 4 and 5 go ahead
    ("up to you").

  **Steps 4 and 5 done (2026-11-03): the hand-over.**
  - **Above the hand-over:** on High and Ultra, SE's own sharp flat clouds draw the clouds, shaped
    by the same weather as the volumetric ones. The weather's effect on them fades out between 40 and
    60 radii.
  - **The hand-over:** the volumetric clouds take over between 15 and 5 times the highest cloud top
    above the ground. On Earth that's 138 km down to 46 km.
  - **No pops:** the volumetric clouds fade in over a second once ready, instead of popping in. The
    ray-march is skipped entirely above the hand-over; fog and dust still draw from orbit.
  - **Checked:** a descent at 30° S 100° W shows SE's sharp clouds at 150 and 125 km where the old
    switch showed soft blobs, blending smoothly below. Tests pass.
  - **Speed:** surface views are unchanged (clouds 0.93, 0.94, 0.35 and 1.44 ms). From orbit at your
    settings the clouds went from 8.6 to 9.9 ms down to 0.34 ms, and frames from 15 to 17 ms down
    to 8 to 9 ms.
  - **Test switch:** `SE_CLOUD_LOD=0` brings back the old hard switch.
  - **Next, added to the plan:** below about 100 km the volumetric clouds still turn into soft blobs.
    Their shapes come from a 10 km-a-texel copy of the cloud map, where SE's flat clouds use tiles
    down to about 1 km. A detailed cloud map around the camera, from SE's finer tiles (step 5b),
    fixes that. Then step 6, the higher-resolution near pass.

  **Step 5b done (2026-11-03): a detailed cloud map around the camera.**
  - **The map:** below 1.5 times the hand-over's start, each cloud layer gets a 1024-texel patch
    about 800 km across (about 800 m a texel on Earth) under the camera, drifting with its layer. It
    moves once the camera is 100 km from its centre.
  - **Where it comes from:** for Earth, SE's level-5 cloud tiles on a worker thread (14 tiles,
    53 ms). For generated clouds (your planet 2), SE's cloud generator on the GPU.
  - **How it's used:** the volumetric clouds read it near the camera and blend into the 10 km map
    towards its edge.
  - **Checked:** in the Earth descent at 30° S 100° W the clouds keep SE's finer shapes all the way
    down, where before they turned into soft blobs below 100 km.
  - **Also fixed:** the volumetric clouds' cover had SE's 5% cut applied twice since step 2. Their
    copy of the map is stored already cut. Now only the flat shells, which read SE's raw values, cut
    it.

## Volumetric clouds, weather and speed (2026-11-02 to 2026-11-04), as PROGRESS.md had it

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
- **2026-11-04, thin clouds changing as the camera zooms in and out (fixed and committed; the
  user's videos 10-40-37 and 13-07-25):** over the user's planet RS 8514-912-7-1071286-73 2, from
  1,800 to 3,200 km up, whole cloud features (an S-shaped band, long streaks) came and went with the
  camera's height, and the thin layers looked dotted. It was the morning's cost fix for the moon's
  rings (7ad5b7c): it let a step of the far trace grow past 1/24 of the path, up to 2 km here for a
  4.4 km layer, while each ray's random start still spanned only 0.46 km of it, so the steps sat at
  nearly the same heights in the layer and those heights moved with the camera's. Now every piece of
  a ray inside a cloud layer gets at least 8 steps, and each piece's start is jittered over its whole
  first step. Pictures from 7 heights now keep the same clouds and match the reference; Earth looks
  as before. Clouds' ms at Ultra 3440x1440: the user's planet RS 8514-3620-1-7-69 1 1.0 / 1.7 / 1.9 /
  1.8 (was 1.0 / 1.6 / 1.75 / 1.7), the moon 5.1 2.2 / 1.9 / 2.1 / 2.6 / 2.6 / 2.0 (was 2.0 / 1.9 /
  1.9 / 2.4 / 2.4 / 2.3).
- **2026-11-04, the game getting slower the longer it is played (fixed and committed; the user's
  find):** test runs always start fresh, so they never saw it. Session tests that keep exploring
  (`.optimize/bench/session_long.sh`, `session_rs.sh`) showed the same view of the user's planet
  RS 8514-912-7-1071286-73 2 going from 10 to 16.8 ms in five rounds of landing elsewhere on it (the
  graphics card's own time unchanged), and Earth's view and video memory creeping up too. Two causes:
  - *Ground patches kept:* SE's CacheSizeLandNode keeps 10,000 terrain nodes; each of the port's holds
    about 6 GPU buffers, 4 bind groups and a texture, so a full cache meant 60,000 buffers, and wgpu's
    bookkeeping for every command buffer grows with the GPU objects ever alive at once (a Chrome trace
    of the render thread: every pass's command buffer took three times longer to finish; the game's
    own systems stayed at 4.5 ms). The port now keeps at most 4,000 (`memory.rs` `PORT_LAND_NODES`,
    SE's own advice for LandLOD 0). The same test: 9.0 / 10.8 / 10.5 / 11.1 / 11.0 ms, and nine rounds
    stay within 11-14 ms instead of climbing.
  - *SE's planet texture tiles* (Earth, Mars, the Moon: colour, lights, water, cloud tiles, 0.25-1 MB
    each) were never released: video memory grew all session. They now go, least recently used first,
    beyond 1 GB (`terrain.rs` `TEXTURE_CACHE_MB`).
  - The speed logger (`SE_PERF`) now also records the main asset counts, wgpu's live objects, the
    render world's mesh instances, the tile cache, the terrain caches and the game's memory, per segment.
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
  optimization options that would change the picture (the biggest: render nebulae at SE's own
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

### The earlier "Recently done" list

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

## Mods menu and Viltrumite powers, steps 1-9 (2026-11-06 and 07; moved from PROGRESS.md on 2026-11-07)

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
  - **Step 7 done (2026-11-07): the sonic boom and the heat.** Past each planet's own speed of sound (Earth about 335 m/s low down, Mars 233, Venus 449) you get the boom's sounds and a ring of cloud; a cone of vapour hangs round him up to Mach 1.6, lit by the sun, so nearly gone at night; slowing back under Mach 0.92 plays the ending sound and a puff of the cone, no boom. At most one boom in 3 s. Above the air (55 km on Earth, 16 on Mars, 188 on Venus by the game's numbers) there's none, so a dive from orbit makes none on the way in, nor does turning on at speed; on Pluto or a 1 mbar air there's never one. His skin heats by the real formula: dull orange at 800 K, yellow 2,000, white with a shell of plasma 3,500, a blue-white trail 6,000, looks clamped at 10,000 K, then only the shell and the trail grow. The heat bar in the readout is filled, in the colour of the heat. **Checked by numbers:** at the table's speeds in the game's real air, Earth at the sea and 60 km, Mars and Venus at the ground come within 7 % of the table, 1 mbar's air within 9.1 % (Mars 21 km up; no body in the solar system has 1 mbar, Pluto has 1 Pa and was flown too). **Checked by pictures,** from behind and the side: every stage on Earth low and at 60 km, Mars and Venus by day and night, Pluto, the cone, the ring, dives from orbit by day and night, in clear air and over cloud, in photo 0, 1 and 2 (no jump in the exposure, never dark), your 3440x1440 with Ultra clouds, 4x MSAA. **Cost:** 0.01 ms of the graphics card when hot at your size, nothing when cold or off. **With the mod off** the ground, towns, trees and Mars are unchanged (bit for bit, or only the HUD's digits), Jupiter's floor by 1 level in 0.2 % of a nearly black picture. **Your calls on what was left open:** a Go To through the air booms and heats him too, by its real speed (D9, real physics; the look is clamped at 10,000 K anyway): a landing from orbit (Shift+G) peaks at about 26,000 K, white for about 3 s, then he stands cooling for about 2 s; a plain G from near the ground leaves the air at hundreds of km/s, so he flashes white-hot and fades out over about 2 s on his way up (please look: too much?). The trail lies behind the camera when you fly straight, so from straight behind you don't see it; you do in any turn, or from the side. Catalogs' Jupiter and the other giants really have no air pressure in the game's files, so they take 1 atm at the radius. **Fixed on the way:** deep in a giant he no longer stays brighter than the cloud round him (his light now falls as the decks' does), and at Jupiter's floor he's no longer hidden: the game fades the whole view to black down there on purpose, which covered him too; it now leaves out what's right by the camera, and a faint light keeps him just visible in the black. Titan's air now has Titan's gravity (it was ending 8 times too low). **Not judged, stills and logs only:** how any of it moves: the speed of the flames and the ring, the trail's points dying, the sound by ear; a Go To's pictures in mid-flight. **Seen, and left:** clouds overhead get blocky edges when flying under them at several km/s (the build before this plan does it too); in thin air and at high speed his cape can stand up off his back like a flag (step 5's cloth, worth a look); his cape crosses the shell as a dark shape. A review then fixed: the cone of vapour hung round him in near vacuum (up to 200 km on Earth, on Pluto, in a 1 mbar air), and now shows only where the boom's air is; the trail vanished all at once a second after he slowed, and now each part fades in turn; it's as long at any frame rate (at 165 Hz its tail was cut off while bright), and seen end-on it no longer makes a bright spot; at x1000 a plain G from low over a planet counted the planet's spin a thousand times over (465 km/s over Earth's equator), and now reads as at x1; deep in a gas giant the air now thickens and warms with depth, as it really does (on Jupiter he falls at 63 m/s 100 km down instead of 190); the 30 km/s cap on a Go To's heat is gone. Left as it is: with the 3D clouds off, his shell and trail show through flat clouds in front of them. Every number is in `viltrumite/tune.rs`.
  - **Step 8 done (2026-11-07): craters, dust, punches, the thunderclap.** A hard landing (35 m/s or more) leaves a crater: 2.4 m in radius at 35 m/s, 12 m at 900 m/s, 60 m at most, a burnt bowl drawn on the ground itself (so it follows the hills, takes cloud shadow and goes dark at night), broken rock round him, and a burst of dust in the ground's own colour (green-brown on grass, sand on the Sahara, white on snow, grey on the Moon, where it flies out and falls back with no air). On the sea a white splash instead. **Left button:** a punch (a puff where it hits the ground within 5 m); aimed 45 degrees or more down within 15 m of the ground, a ground punch: he goes down to it and leaves a 6 m crater. **Right button:** hold 0.4 to 2.5 s and let go for the thunderclap: a ring of air 30 to 150 m by how long you held it, going away ahead of him, and over the ground a second ring and dust thrown out along it; it breaks nothing. Big impacts shake the view a little (1 degree at most). Craters stay until both he and the camera are 20.5 km away, with no fading before that, also after the mod is turned off; 32 at most, the farthest giving way. **Checked by pictures,** from behind and the side: real dives onto Kansas farmland, Irish grass in a town (by day and night), the Sahara, the Moon and Greenland's snow; a fall from 300 m; ground punches; a punch into the ground and in the air; claps at three charges on the ground and 130 m up, in clear air, under Manaus's rain clouds and inside a cloud (where nothing shows, as it should); splashes from a fall and a 411 m/s dive, in sun and under rain. **Checked by the log:** a crater is still there after flying 3 km and 10 km away and back (it stays on the ground as the ground's detail changes), freed at 20.51 km, gone on the way back; turned off, it stays and is drawn. **Cost** at your 3440x1440 with Ultra clouds: 0.2-0.25 ms for six or seven craters in view, 0.05 ms for a big crater's dust, 0.3 ms at a full clap's peak (measured with the window capped at 60 fps by Windows, so likely a bit high); nothing with the mod off. **With the mod off** the ground, towns and Mars are unchanged against the build before the plan (bit for bit, or only the HUD's digits, or what two runs of the old build differ by). **Differs from the plan:** the rocks round a crater use a material of their own that takes the ground's light (the planned one ignored clouds: under rain they stood sunlit), and they're scaled flatter (at full scale a big crater's slabs buried him to the chest); the dust takes its colour from the ground round it; the clap's ring travels ahead, or from behind it would surround the camera at once. **Please look at / listen for:** how the dust billows and settles, the shake, the clap's ring and dust in motion (stills only so far); whether a full clap's dust is too much (it fills the view from 22 m); the punches are still stand-ins (the batman and thor clips raise the arm, more uppercut than punch) until you download the Mixamo ones; the punch, ground punch and clap sounds by ear. **A review then fixed:** a ground punch from a jump no longer throws him (and the view) 6-8 m sideways for a frame; turned off beside a crater, its rocks now go dark at dusk and night (they stayed in the sunlight of the moment you turned off); punching the ground several times in one spot makes one crater, not ever darker ones; the crater's darkening no longer falls on his fist or arms (it now follows his pose) nor on the sea; the charge sound plays only while a clap really charges; letting go early no longer jumps his pose; opening the menus (Tab) or leaving the window while charging cancels the clap instead of firing it; a clap far out in space now shows its ring and shakes the view, and a clap in the air shakes it properly; the clouds now shake with the view; nothing of the craters is drawn any more once the mod is off and they're gone. Left as it is: with 3D clouds off, a crater under SE's flat clouds darkens the cloud over it, and a crater 10-20 km away stays a little too dark in the haze.
  - **Step 9, first part done (2026-11-07): the game can tell him which buildings are round him.** Solid buildings need to know where each building is, and the buildings are placed by the graphics card, so each frame the card now also lists the buildings near him and his path ahead, and the game reads that list back. Nothing is solid yet (the next part makes him stop at walls, slide along them and stand on roofs). Measured: the list arrives 3 to 5 frames later at any frame rate (31 ms at 130 fps, 98 ms at 51 fps on your screen with Ultra); it holds 5-80 buildings in play over New York and a level 3 city, up to about 300 at 30 km/s, never more than it can hold. The worry was that a building seen from near and the same one seen from afar might not be recognised as the same (they are placed by two different grids): measured over 919 buildings in New York, a level 3 city and a domed colony, the two always lie within a quarter of a metre of each other, so the planned 4 m match is plenty and nothing had to change. What can differ is a building's height (a tower near can be a low block from afar: that is how the cities already look, and step 10 handles it by name). No two buildings were ever mixed up. With the mod off this step changes nothing: the pictures are bit for bit the same as before it and it costs nothing. Found on the way, not from this step: since step 8's review fixes, New York by night with the mod off differs very slightly from the game before the plan (a fine grain on the window edges, not visible by eye; by day it is identical); worth a look later. Also measured for the next part: which building shapes work as 8 stacked boxes (most do; the twisted tower, the spires, the watchtower's legs and the landmark's frame need care). A review then fixed: when the buildings stop being drawn (`scatter off`, Cities and Vegetation off, leaving the planet) the list is now emptied at once instead of keeping the last buildings (they would have been solid while invisible), and the game now knows how old a list is; with the mod off the probe's work now stops completely; a frame whose list the card had no room to copy is now counted (and two more copy buffers make it unlikely: none in a test at your size with Ultra). The record now says honestly that a building's middle can move between near and far: a quarter metre measured, at most about 2-3 m in theory (near a corner of the planet's cube), still inside the 4 m match. **Next: step 9's second part** (stopping, sliding, roofs, the camera kept out of buildings).

## The Sidewinder's finishing run (2026-11-16 to 17, moved from PROGRESS.md on 2026-11-17)


One session now owns all of the mod's files and works through this list with two writers at a time; the plan and the
rules are `ARCHITECTURE.md` A5 to A8. Baseline checked 2026-11-16: the tree as it stood builds (release) and
`elite-sim` and `se_core` tests pass. Writers iterate with `cargo build --profile agent`.

**Next (2026-11-18, morning):** committed: A (0109fb9), T3 (bcaec70), N1b-1 (41678f5), T4a-1 (d0bce8f), T4b-1
(2296db0), T3c with T5b-1 (d71096e), T4a-2a with T4a-2b (0d90b56: Elite's materials, the paint job, the glass, the
lit maths), N1b-2 with the seat fixes (bb65059), N2 and N2b (5bfabc7, ebb8942: the gate's new reading; the gate
itself still open at 10 rows, see its line below), T4a-3 (638ff85: the ship's lights), the ship's sound in the game (b65a821: T5b-2, T5c and the fix round
T5d-1; reviewed twice; its open items are T5d-2 in the queue and T5b's line below), N3 (41a67b1: the gear's and scoop's limits; orbital
cruise's rule, not switched on, see N3's line), T4a-4 (39b0bcd: the lights' fix round, see T4a's line), X1 (527f552: the old stand-in `ship.edm` /
`cockpit.edm`, the Blender cockpit folder and the code that only served them are gone), N4 (ceeff2e: the gear's
factor in supercruise and supercruise's 0.5 s time constant, in the flight crate), T5d-2 (1c4341b: the sound's
one output source and Elite's equaliser), T4c with T4c-2 (17201a5: the ship's state machines run), T4e-4
(b5a67dd: the HUD anchored in the cockpit), S1 (9fef26c: the seat's field of view by Elite's rule; the head turns to
an opened side panel), T5d-3 (aedbc2c: Elite's distortion effect, the limiter's gain and the player's context states
in the sound), T4d-a (3e0b71d: the pilot chair's animation runs from the controls; not drawn yet), T4a-5 (93dc42f: the
cockpit's panel lamps, its brightness measured again, the broken canopy), T5d-4 (e0b9d4f: the sound's voices in
blocks; no click at supercruise entry), S1b (da49a6c: Elite's head-look camera; the side panels fitted on each
picture's own camera), T4d-b (d06e65a: the pilot chair drawn with its animation), T4e-2 (afb1ef1: the
headlights light the ground), S1c (d1cd006: the right panel's state column on screen, its rectangle refitted), T5d-5 (48325b9: the gear's
sounds at their legs, the cues from the machines' events, the drive at Elite's pitch), T4d-c (208b6c1: a knock
shakes the chair the same at any frame rate), T4e-1 (d4e7499: the ship's shadow with Elite's own filter), T5d-6
(23e8fe0: the engine's sound inputs in Elite's units, flight assist the right way round) with its fix rounds
T5d-6b and T5d-6c (af2e1cc, f6924e8), T4e-3 (507f2e6: the hyperspace tunnel seen from inside the cockpit), T4e-2b
(5521ebc: ground outside the headlights' beams no longer pays for them), T5d-7 (ed0e477: every voice reaches the
stereo output by Wwise's own rules; the left landing leg is heard 10 dB to the left), T4e-3c (8f3ae9a: nothing
of the universe's overlays over the tunnel; the exposure held through a jump), T5d-8 with T5d-9 (2e02874: Elite's
master effect with its 14 dB, the tone generators, the supercruise bus's compressor; reviewed) and T5d-10 (dc7f522:
three small notes). THE SOUND IS FINISHED. The shadow as a whole (b773a6b: T4e-1c, T4e-1d, T4e-1b and their
review's fixes: as soft as Elite's, and on the world through each thing's own lighting). Not committed:
- **T6, the wrap-up**: running: the release build; one reviewer on the shadow as a whole; a second on the whole
  mod (the spec's checks, the ship against your Elite screenshots, a full play-through).
- **T4e-1c (study)**: done (`re/elite/notes/sidewinder_shadow.md`): Elite's own shadow settings read from its
  plain configuration file and its exe. Your Elite is on High shadows: three ranges by distance from the camera,
  each with its own softness (a shadow texel of 2 cm under 5 m, 38 cm to 81 m, 3.4 m beyond), blurred before
  the filter; measured on your picture `9.webp` the leg's shadow fades over 0.17 to 0.25 m where that rule says
  0.37. The two stand-in filter inputs are settled (the bias was right; the light-bleeding cutoff is 0.3, not
  0). To be built: one map of the ship for each of Elite's ranges, made as Elite makes them.
The measuring session could not start (see the top of this file). The mod's folder is 1,031 files, 187.6 MB.
You asked (2026-11-18) to finish up: reviews are batched (one for the shadow as a whole, one for the last two
sound tasks). Queue, in order: T4e-1b (the world's shadow in the receivers' own lighting, after T4e-1d); the sound, two more
tasks one after another in the same files (the design is `ARCHITECTURE.md` A7's last block): T5d-8 (the master's
last effect with its 14 dB, from the study T5f; the four tone generators; each recording's gain moved beside
the make-up gain), T5d-9 (the supercruise bus mixed in its 12 channels with its compressor, which may act by up
to 3.5 dB at supercruise's loudest); then T6 (measuring the sound against your recordings again is cut). Still
unexplained in the sound: Elite's hum at rest. Three looks did not find it among the ship's sounds (lines at
multiples of 19.43 Hz, loudest at 58 to 97 Hz; the recording was made in open space near the system's star;
every recording, synth source and modulator of the ship's 63 sound banks is ruled out). The guess left: it is
the star's own rumble, a sound of the world, not of the ship; a recording at rest far from any star would tell
(YOUR step, or a measuring session). A recording of the gear going down with the head straight would check the
left / right law against Elite itself.

- [x] W0 The tree builds and its tests pass (checked 2026-11-16)
- [x] T3a Convert the real models, materials, textures and bones (`models/*.edm`, `tex/*.dds`, `materials.json`):
  done and committed (bcaec70). Written 2026-11-16 (104 models, 240,663 triangles, 74 textures, 46 materials, 122 MB, every byte checked against
  the export; the game still draws the stand-ins until T4a). Reviewed 2026-11-16: pass (the reviewer re-checked every
  vertex and index byte, all 273,911 decoded vertices against the reference decoder, every texture and material, and
  3,400 damaged-file cases: no faults). Four small follow-ups go to T3b's writer when it finishes: `parse_edm` must
  refuse a vertex naming a bone the file lacks (one overlay model has no bones); a re-check test of the shipped files
  against the export; the paint mask's and IES profile's sources in `SOURCES.txt`; `materials.json` and `ship.json`
  checked by their own parsers when the mod is first turned on. All four done; the arena-only drive bolt-ons were
  dropped (99 models, 73 textures, 45 materials, 123 MB)
- [x] T3b Convert the assembly (rigs, state machines, lights, paint job: `ship.json`) and build Elite's rig and state
  machine rules as tested code (`se_core::elite_rig`): done and committed (bcaec70). It matches the reference code to
  1e-14; gear times proven by tests; the ship rests 4.718 m over its feet. The review's two runner bugs are fixed
  (`SetTargetState` takes the route when one exists; no recursion, loops refused)
- [x] T3c Convert the chair's clips and motion graph and build their rules as tested code (`se_core::elite_chair`):
  done and committed (d71096e): 3 states and 13 clips for a seated solo pilot; positions and blends bit for bit
  against the reference; damaged data is refused, never a crash
- [ ] T4a Draw the real hull and gear with Elite's materials: the default paint job, the decals, the ship's lights.
  T4a-1 (the real ship in the game, plain shading, real solids, feet, gun mounts, ship space) is done and committed
  (d0bce8f; pictures `.optimize/scratch/run/t4a1_*.png`: the hull, gear and cockpit frame match your screenshots;
  the review's two findings fixed: the air's drag uses the hull's convex outline, a second spawn is not blocked by
  the old ship). T4a-2a and T4a-2b (Elite's material maths for every family the ship draws, the default paint job,
  the glass, Elite's lit maths, the hull's wear by the game's own rule) are done and committed (0d90b56; pictures
  `.optimize/scratch/run/t4a2a_*.png`, `t4a2b_*.png`). T4a-3 (the ship's own lights: all 84 from Elite's data with
  the exe's blink rules, the vents' orange glow with heat) is done and committed (638ff85; the outside views match
  your pictures). T4a-4 (its review's fix round) is done and committed (39b0bcd): a light goes dark at once when
  its state ends; the seat takes only the lights Elite flags for it; the lamps follow the exposure the frame is
  really drawn with (the automatic exposure's too); the cockpit's lamps carry one measured stand-in, 1/21.6 (the
  console against the red canopy lamps in your seat picture: 0.016 now, Elite 0.021, 0.88 before), because
  Elite's exe did not give the missing brightness value. Left (T4a-5, small): the two GUI panel lamps are dark in
  the mod (their colour is the HUD's) and light the side consoles in Elite, so the stand-in is partly making up
  for them (measured fairly it is 1/30 to 1/42); the exposure copy keeps running after the ship is gone. T4a-5
  is done and committed (93dc42f; reviewed): the five GUI panel lamps follow Elite's own rule; the cockpit's
  brightness, measured again by steps, is 1/11.4 (brighter than before; good to about a factor of two); the broken
  canopy's own model is drawn once breached, with Elite's cracks rule. Left: in play the cracks get no damage to
  follow yet, so a breach shows broken glass without cracks (T4d-b feeds them the hull's damage); the side
  consoles Elite shows lit stay black (not the panel lamps' light: open); the cracks' inner reflection and the
  blowout shards are not built. Left for T4e: by day the sun lights the cockpit's inside with no shadow
- [ ] T4b Draw the real cockpit, its glass and the chair from the seat (Elite's field of view): T4b-1 done and
  committed (2296db0): only the cockpit set from the seat, 56.249 degrees, never cut by the near plane, the eye at the
  cockpit model's own `!Camera` bone (against your straight-ahead screenshot the crossbar matches within 3 px of
  2,000, the side rails are 14 px high and the dashboard's edge 16 to 30 px low: the reviewer's reading is that
  Elite's head sits 0.1 to 0.2 m off the bone, for example its movement under acceleration, and that your picture is
  a crop). The glass and the cockpit's materials are T4a-2b
- [x] T4c Animate the gear, doors, hardpoint doors and heat vents in the game from the rigs and state machines:
  done and committed (17201a5; reviewed twice). The 20 state machines run; the gear's doors open in 2.50 / 2.73 /
  2.63 s, each leg lowers in 5.33 s once its doors are open and rises in 4.23 to 5.03 s; hardpoint doors 1.07 s
  out, 2.57 s back; the vents' flaps follow the heat; the gear's sound cues post at their data times. The flight
  reads the gear's real state (the 3 s stand-in is gone); a ship coming down on moving legs is held by them; the
  gear cannot be raised while landed. Not built: the breached windscreen's own model and cracks (T4a-5), steam,
  sparks and glass particles, the raised stance
- [x] T4d Animate the pilot chair (throttle, stick, trigger, buffets): T4d-a done and committed (3e0b71d): Elite's
  motion graph and clips run from the controls every frame (the throttle lever and the stick's grip move in the
  simulation). T4d-b is done and committed (d06e65a; reviewed): the chair is drawn with that pose (the lever, the
  stick, its twist and the trigger move on screen), stepped at a fixed 60 a second. T4d-c is done and committed (208b6c1): a knock shakes the chair
  the same at any frame rate; a new ship takes no knock from the lost one's speed. Scratch
  to clear at the wrap-up: `.optimize/scratch/run/t4d_b_target` (4 GB; its `t4d_b_rigid/assets` and
  `t4d_b_before/assets` hold LINKS to the real asset folders: `rmdir` each link, never a recursive delete)
- [ ] T4e The headlight's IES beam on the ground; the cockpit UI overlays in the HUD's layout: T4e-4 (the HUD
  anchored in the cockpit at Elite's places, turning with the cockpit) is done and committed (b5a67dd; reviewed).
  S1 is done and committed (9fef26c; reviewed): the seat's field of view was RIGHT (Elite's own rule makes your
  setting's 56.249 the vertical angle on 16:9 and wider screens, and the measuring sessions' video frames fit it;
  your three seat screenshots were taken at wider views, so the earlier "1.14 times too big" was the pictures,
  not the mod); a side panel opened by its key turns the head to it. S1b is done and committed (da49a6c;
  reviewed): Elite's head-look camera read from its code (the head aims at a point each focused panel names and
  the view narrows by that panel's own amount, 4.5 to 5 degrees measured on your pictures); the panels fitted on
  each picture's own camera (the left's corners within 3 px); on your screen the turns are exactly the measured
  ones. S1c is done and committed (d1cd006): the right panel's ON / OFF column, which sat in the 1.6 degrees
  that are off your screen's edge, is laid out inside the screen; the right panel is refitted with its bottom
  edge. T4e-1 is done and committed (d4e7499; reviewed): by day the ship shadows itself (no sun through the
  cockpit's roof, the struts' shadows on the dashboard, a dark underside) and the ground under it, with Elite's
  own shadow filter, found in its shader. Left: T4e-1b (the shadow on the WORLD is wrong in places: a prop's side
  facing away from the sun goes black inside it, and your weapon stays sunlit: it moves into the ground's, the
  plants' and the props' own lighting), T4e-1c (the shadow's edges are much sharper than Elite's: take the
  softness from Elite's own shadow settings: being written). T4e-3 is done and committed (507f2e6; reviewed):
  through a hyperspace jump you sit in the cockpit and see the tunnel outside it (before, a flat picture covered
  everything, and the cockpit was not drawn at all). T4e-3c (8f3ae9a) fixed its review's notes (no constellations, grids or
  waypoints over the tunnel; the exposure held through a jump). Not built, known: the canopy's glass is drawn before the clouds, so its reflections, dirt and cracks
  fade where a thick cloud is behind (see `ARCHITECTURE.md` A6, T4e-3)
- [x] T5a Read the ship's sound events, mix data and triggers out of the exported banks and the exe: done 2026-11-16,
  nothing decrypted. `re/elite/frea_export/sidewinder_audio/events.json` (all 59,537 objects of the 63 banks parse to
  their exact size: 809 events with their sounds, containers, volumes, pitch, filters, randomisers, game-parameter
  curves, states, switches, attenuations, buses; the Init bank with 1,034 game parameters is at rest),
  `triggers.json` (37 triggers from Elite's code: engine loops and their 20 motion parameters, boost, the frame shift
  drive's charge / countdown / cancel, the three audio modes, cockpit cues, plus the 8 proven before) and
  `re/elite/notes/sidewinder_sound_events.md`. Still open: the pulse laser's shot event (in the weapon's data file,
  not at rest), the jump event's sender, the targeting UI cues, the low-fuel and other voice lines' senders; 261
  sounds the events reach still need rewrapping to Ogg (the old tool capped a role at 40 files); the old manifest has
  3 wrong loop flags and calls the positional cockpit loops "thrusters"
- [x] T5b Play them: gear cues, flight assist, boost, thrusters, lasers, supercruise, hyperspace, heat, hull, shields,
  the voice. Done and committed: T5b-1 (d71096e: `sounds.json`, 849 recordings in `sfx/`, the pure mixer
  `se_core::elite_sound`) and the sound in the game (b65a821: `sidewinder/sound/`, triggers from Elite's code, the
  mixer's second round, the review's fixes). It plays while seated: engine and cockpit loops, boost, flight assist,
  the drive's charge and countdown, supercruise and hyperspace, gear, scoop, lights, silent running, the heat, hull
  and shield voice lines. The dead audio after a supercruise drop is fixed (a bug in the audio library; the voices
  resample themselves now). The level is unity, within 1 dB of your recordings on the flight-assist beeps.
  OPEN (T5d-2, T5d-3): the flight loops are 5 to 9 dB under Elite's (full throttle -5.5, the boost's firing -8.5;
  the likely cause is the buses' effects, not built: the drives' bus has an equaliser with a +5 dB bass shelf);
  silent copies pile up over a long session (500 supercruise trips: 300 a boost); a supercruise entry blocks one
  frame for 14 ms; every positioned sound plays centred; the gear's cues wait for T4c (both done by T5d-5,
  48325b9: the gear's sounds come from each leg's own side and turn with the head; every other sound stays
  centred because Elite's data names bones for them that are in no model); the pulse laser's shot, the
  jump event, the targeting cues and the low-fuel line have no proven trigger (nothing is played for them);
  reverb is not built. FINISHED 2026-11-18 (the commits b65a821 to 2e02874; each reviewed): one output of our
  own mixes every voice; Elite's equaliser, distortion, master effect (the 14 dB), tone generators and the
  supercruise bus's compressor are ported from Elite's own code; every voice reaches the stereo output by
  Wwise's own rules (the gear's legs sound from their own side); the engine's inputs are Elite's own, and
  flight assist plays the right beep each way. Known differences that stay (the last measurement, before you
  cut measuring): the ship is 2 dB louder than Elite on the beeps; its 2 to 4 kHz is 9 to 19 dB too strong at
  full throttle and in the boost; Elite's low hum at rest is missing (not found among the ship's sounds:
  probably the star's rumble); reverb is not built; the pulse laser's shot, the jump event, the targeting cues
  and the low-fuel line have no proven trigger and stay silent
- [x] A The ship systems finished and checked (committed 0109fb9): silent running (Delete), the cargo scoop (Home), the lights' switch
  (Insert), the canopy and the 5-minute emergency oxygen, the safe-disengage sign, the fades, supercruise's wider wells:
  written and shown working in game runs 2026-11-16 (the Build log has the numbers). Reviewed; the three findings
  fixed (the oxygen clock now runs in the hyperspace tunnel and at any frame rate; a breached ship at 0 is lost).
  The reviewer's warning for N1: the wells' width formula gives even a 5 km asteroid a 22 AU well (Sol has 2,516 bodies)
- [x] N1 Real numbers from Elite's data and exe for every stand-in: study done 2026-11-16
  (`re/elite/notes/sidewinder_numbers.md`, `re/elite/work/numbers/numbers.json`). 70 of 99 stand-ins have the game's
  own value (grade B: read from the ship item's, its stock modules' and its prototype's records; the records alone
  give the journal's 8.149506 ly range and the 42.9 t mass), 3 a rule from the exe, 26 stay to measure
  (`re/elite/measure/BACKLOG.md`: a 10-run plan). What disagreed with the numbers in use: base shield 55 (not 40),
  pitch 43 (35 at 0 pips) and yaw 17, forward acceleration 45 at every pip setting (the fit said 43.42 at 0 pips),
  supercruise's cap floor 200 km/s near planets (30 km/s is the zero-throttle speed), supercruise charge 5 s (not 15),
  alignment 10 degrees (not 60), touchdown 15 m/s and 20 degrees, boost acceleration x4, the capacitors' curve power
  1.1. Four files were opened with the offline reader for this (the ship item, the stock modules, the prototypes, the
  laser's ammunition); only the Sidewinder's values were kept. Not yet applied to the code: that is N1b
- [x] N1b Apply N1's numbers in `elite-sim` and `pilot.rs`: done. N1b-1 (`elite-sim`) committed 41678f5 (reviewed:
  every one of the 99 entries compared, no value or unit wrong). N1b-2 (`pilot.rs` on those numbers, one way out of
  the seat, the overlays' field of view from the seat) committed bb65059 (reviewed: every way out re-tested in game
  runs). Left for N3: by a planet the cap is still the stand-in 30 km/s, which holds supercruise's turn at 60 % of
  its best there; small notes for the next writer in those files: a stale comment (`pilot.rs` ~527), review notes
  written into code (`seat.rs` ~477, ~485), the `sw canopy SECS` hook (works only after the 6.5 s grace)
- [ ] N2 The strict trace gate passing: as far as the exe and the recordings go, done and committed (N2 5bfabc7,
  N2b ebb8942; reviewed). The gate reads the simulated curve between ticks, rounded as the HUD rounds its number,
  with one measured capture delay per recording session (the spec's Decisions log T7). 10 of 18,344 rows still
  miss (s01 1, s04 9), all about 2 s after W, the moment the throttle reaches full: the test stays ignored. No
  ship number changed. What is left is in the keys, not the flight rules: Elite acts on W and S 67 to 92 ms after
  the key, on a thruster key's release 55 to 59 ms, on the boost 26 to 32 ms, on X and a thruster key's press 13
  to 22 ms; the replay acts on all alike. Elite's exe holds no rule for that: its keys go through an input graph
  loaded as data (`re/elite/notes/sidewinder_input.md`). To close it: read that graph's file with the offline
  reader (YOUR decision: the run's rule was to decrypt nothing new), or measure the throttle keys' delay without
  the flight model
- [ ] N3 Orbital cruise, the landing gear's limits, the scoop: the limits are done and committed (41a67b1,
  reviewed): the gear down or the scoop out halves the top and boost speeds (Elite's code and records; 110.5 m/s
  against 221 on the Moon), the boost stays allowed. Orbital cruise's rule is read out of Elite's exe and built
  (`elite_sim::orbital`) but NOT switched on: its seven zone numbers are in a game record that was not exported,
  so near planets the stand-in 30 km/s cap stays and supercruise turns at 60 % of its best there. To finish it:
  read that record with the offline reader (YOUR decision: this run decrypts nothing new), or measure it in Elite
  (run G1 in `re/elite/measure/BACKLOG.md`, which needs the ship refuelled). Small notes left for the flight's
  next writer in `pilot.rs` (once T4c-2 is committed), three lines from N4 (ceeff2e), so a gear down in
  supercruise halves the cruise speed as Elite's code does: ~1041 `let turning = supercruise::cruise_block(cap,
  speed_k, c.speed);`, ~1048 `flight::step(&turning, supercruise::VSC, false, &rin, ...)`, ~1052
  `speed.step_held_with(throttle, cap, speed_k, TICK);` (`speed_k` is `deployables::speed_factor(pilot.gear_down,
  pilot.scoop)`), and the comment at ~1053 (the jerk memory is reset by every drop and glide)
- [x] M Measuring sessions in Elite: CUT by you (2026-11-18: "Cut the measuring sounds and measuring session
  4"). Session 4 was prepared and never ran (the launcher's access prompt); the 26 numbers it would have measured
  stay as they are, each marked in `elite_sim::data` and listed in `re/elite/measure/BACKLOG.md`. Measuring the
  mod's sound against your Elite recordings again is cut too: the last tables are the ones in the spec's Build
  log (T5d-6, T5d-7), taken before the master's effect was built
- [ ] T6 The ship checked against your screenshots, the spec's checks, a tested release zip, the notes

For `CLAUDE.md`'s Gotchas at the wrap-up (T6), so nothing is lost: never let rodio resample a ship voice (its
`SampleRateConverter` overflows and hangs the audio thread for minutes: `VoiceSource` always reports 48 kHz and
applies the speed itself; keep the player's speed at 1); the gear down or the scoop out halves the top and boost
speeds (`elite_sim::deployables`; anything that builds the block uses `sidewinder_block_with`); supercruise's jerk
memory is `Motion::last_acc`; orbital cruise's rule is `elite_sim::orbital`, unused until its zone record is read;
`docs/elite-flight-model.md` has mixed line endings (patch it with Python bytes, never an edit tool); the gate's
reading is the spec's Decisions log T7; a leg waits for its doors through `Runner::step_with`. The ship's machines
(`parts.rs`, the requests in `machines.rs`) read `facts::ShipFacts` (written once a frame after the flight, holding
the tests' heat and gear overrides); a gear door is asked in its leg's own frame (keep `parts::frame`'s sub-frames
and the zero-time door step together); the flight's ground contact is `Parts::feet`; test verbs `sw parts`,
`sw parts log on`, `sw parts ask MACHINE STATE|auto`, `sw pose up|down|auto` (put `wait 900` after `sw spawn`).
The sound: one output source (`voices.rs` `OutputSource`) mixes every heard voice, virtual voices are never built;
effects ship only through `effects.json` (`re/elite/tools/sidewinder_effects.py`: re-make it after `events.json`);
a game parameter an effect reads must be defined in `sounds.json` or the bank refuses it. The HUD is drawn through
`hud_anchor` (`Sketch` -> `finish`): a new element is drawn in the old layout's coordinates inside a group, never
straight on the painter; an anchor's numbers come from `.optimize/scratch/run/t4e4_anchors.py`. The mod's files
no longer hold `ship.edm` / `cockpit.edm`; the converter does not delete a file it stops shipping (`git rm` it).
The sentences that still describe the stand-ins: `CLAUDE.md` 178-186, `README.md` 405-406, the spec 289 and its
claim C6 (`geometry_is_elites` is gone: `shipped_files_are_the_exports` re-checks the models).
The seat's view: `seat::Head` is the head's turn AND the view's extra angle; the camera, the projection and the
HUD all take it from the one function (`head_turn`); the user's three seat pictures each have their own field of
view (63.5, 59.0, 58.5 degrees): use them for places only after fitting each picture's own camera, never the
game's 56.249. The side panels' turns are measured numbers (`PANEL_TURN_*`): don't "fix" the right panel's
outer edge being off the screen by turning further. In Elite's sound flight assist ON is the state `On` with
`FlightAssist_b` (T5e); the engine's inputs are percent of the full-power top speed and shares of the
speed-limited turn rate, never m/s or rad/s.

## The Sidewinder mod, finished sections (2026-11-17 to 24; moved from PROGRESS.md on 2026-11-26)

### Sidewinder: B gets you out anywhere (2026-11-24): done

Your rule, not Elite's (Elite refuses unless landed). B, and the role panel's Disembark:
- **Landed:** as before, you step out beside the ship.
- **On or near the ground but not landed** (hovering up to about 6 m, tilted, even on its side or upside down): you
  step out beside it and the ship stays; it is set back on its feet. A ship on the ground is never deleted by leaving
  it, whichever way you leave.
- **Anywhere else** (higher in the air, in space, in supercruise, in a hyperspace jump): the Sidewinder disappears and
  you are in free flight (as with V) exactly where you sat. V from there walks if there is ground below.
- Checked by an independent reviewer (it caught one fault, fixed: getting out in the first moments of a jump threw the
  view back to the old system). Tests: the release build is made; the whole workspace in release: 2,513 passed, 1 failed (the known one in the untouched Skate 3 copy), 213 ignored. Commit: 75dceac. The design is `ARCHITECTURE.md` A12;
  the decision is D8a and the Build log entry B1 in `docs/spec-mods-sidewinder.md`.
- Also fixed on the way: after any flight the ship is set on its feet when you get out (it could drop or slide).
- Known: B has no "are you sure": one press in space and the ship is gone (spawn another from the Q menu, on foot).
- **Next (found on the way, older than this change, not fixed):** on sloping ground the parked ship sometimes does
  not come to rest and creeps a few centimetres every ten seconds. Also: a ship spawned seconds after the game
  starts can pop and tilt while the ground is still loading under it (not seen once the ground has loaded).

### Sidewinder: the three faults from your second play, and the lasting slowdown (2026-11-24): fixed

- **Z did not undo the ship; the physics gun could not move it.** The parked ship is now a real sandbox prop: Z undoes
  it in order with your props, "Clean up everything" removes it, and the physics gun grabs, carries, throws and freezes
  it (its gear, lights and shadow go with it). Board a ship you left hanging or tilted and you are flying it (B gets you
  out of it all the same: see above). E does not board while the gun is holding something.
- **"The under side does not have a texture."** It was a hole: Elite fills it with a separate part, the cargo hatch,
  which had never been exported. Elite's own hatch is now fitted; the cargo scoop's switch (Home) opens its doors.
  Not there: the hatch's own sounds and lights.
- **The slowdown after using the panels** (the task card): each of Elite's screens kept hundreds of small graphics
  buffers; they are pooled now (opening every panel adds about 60 where it added about 5,300), pictures identical.
- Checked by independent reviewers. Tests: the release build is made; the whole workspace in release: 2,505 passed, 1 failed (the known one in the untouched Skate 3 copy), 213 ignored. Commits: 09e813f (the buffer pooling) and b8c2037 (the prop and the hatch) (never pushed). The design is
  `ARCHITECTURE.md` A11; the Build log entries are P1, P2, P3 in `docs/spec-mods-sidewinder.md`.
- Known: a thrown ship that leaves the ground near you hangs frozen at its edge until you walk towards it (as props
  do); the ship re-settles a little when the ground under it loads finer; the skater passes through the parked ship.

### Sidewinder: Elite's ship UI (2026-11-23 to 24, `/project-coordinator`): done

**What you get when you board the Sidewinder now:** Elite's own cockpit UI, on by default.
- The flight HUD: Elite's gauges on their cockpit panels (speed and throttle, heat, fuel, shields and hull, pips, the
  compass, the status lamps), the weapons ring, the info strip with the clock, all reading the ship's real numbers.
- The four panels with Elite's keys: 1 left (Navigation, with Elite's lock pop-up), 4 right (Home, Modules, Fire groups,
  Ship, Inventory, Status), 2 comms, 3 role (Disembark). Your head turns to the panel; holding Left Shift with a
  direction also picks one.
- The scanner disc (PageUp / PageDown change its range) and the two holograms (your ship; the Orrery, or the locked star
  or planet).
- The FSS scanner screen (the ' key, in supercruise at zero throttle): the system's spectrum. Nothing is scanned.
- Elite's UI sounds, through the ship's sound mixer.
- All of it is drawn at the screen's full sharpness, after the game's glow and colour steps.
- `SE_SW_UI=0` brings back the older flat HUD and panels; they also take over by themselves if Elite's UI fails.

**How it works, in one line:** Elite's own Flash screens are run inside the game by the open-source Ruffle player
(`vendor/ruffle`), fed by plain rules (`crates/elite-ui`) from the ship's simulation. The design is `ARCHITECTURE.md` A10
and its addendum; every step (UI0 to UI9, US1) has an entry in the Build log of `docs/spec-mods-sidewinder.md`; the
gotchas are in `CLAUDE.md` ("Elite's cockpit UI").

**Checked:** every part by an independent reviewer as it was built, and the whole once more as you play it (default
on): no crash, no stuck state, no dead keys. Tests: the release build is made; the whole workspace in release: 2,496 passed, 1 failed (the known one in the untouched Skate 3 copy), 213 ignored. Commits: 13a04f4 (the runtime) and fafda76 (the game side), after the eight earlier ones of this task (never pushed).

**Your word, 2026-11-24: "the UI looks good enough to me, no need for further effort in getting things to be 1:1 from
elite."** So these stay as they are (the full list is the Build log's UI8 entry):
- The gunsight and the planet HUD are still the old flat readouts (where Elite puts them on screen was never found;
  one Elite screenshot of each, hardpoints out and near a planet, would give it).
- No contacts on the scanner (no other ships); pages with nothing behind them (missions, cargo, contacts) are empty.
- The FSS's tuning keys do nothing; the surface scanner screen does not exist (the stock Sidewinder has none in Elite).
- The ship's hologram shows a maze of lines inside; the holograms darken a sunlit dashboard a little.
- The comms panel shows the game's own messages as its lines (our choice: Elite has none without other players).

**Costs to know:**
- About 2 ms a frame of the main thread while seated (Elite's screens running).
- (The lasting slowdown after opening the panels is fixed: see the section above.)
- About 500 MB more memory while seated.

**Open, yours to decide:**
- The players' zip has not been remade with any of this (the mod's folder is now 340 MB, 1,824 files).
- The notes files (`PROGRESS.md`, `CLAUDE.md`, `README.md`, `ARCHITECTURE.md`, the spec, `docs/elite-flight-model.md`)
  are updated but not committed, as before: they also hold other sessions' uncommitted changes.

### Sidewinder: the faults from your first play (2026-11-23)

You played the mod and reported two faults. Both are fixed, reviewed and committed (2c4eccb, not pushed; the design is
`ARCHITECTURE.md` A9, the reports the spec's Build log U1 and U2). `target\release\spaceengine-rs.exe` is built from it.

1. **"Selecting a planet from the left side screen doesn't actually select it and I can't see where to go."** The pick
   was stored and named in one line of text, but nothing in the view showed where it is, and the port's own cyan
   selection pointer stayed on Earth. Now (U1):
   - Space on a row of the left panel locks it as the destination (Space again unlocks): the row gets a mark.
   - A ring with the name and distance sits on the destination in the view, and turns with your head.
   - The compass left of the radar shows which way to turn: the dot in the middle is dead ahead, on the edge is a right
     angle off; filled is ahead of you, hollow is behind.
   - The ship has ONE destination, as Elite: picking a planet clears a picked star system and the other way round (J
     jumps with a system locked, and is supercruise otherwise). Getting out and boarding again keeps it.
   - Coming down to the planet you locked, the marker, the DESTINATION line and the altimeter read the same number.
   - The cyan pointer is hidden while you sit in the ship.
2. **"There is no crosshair in the middle of the screen to show how much you are turning with the mouse."** Elite's
   mouse widget is built (U2): a yellow dot in the middle of the view and a split arrow that moves off it the way and as
   far as the mouse is deflected; the dot fades as the arrow brightens. Its size, travel and colours were measured from
   the videos of measuring sessions 1 and 2 (`re/elite/notes/sidewinder_hud_marks.md`); measured back at 3440x1440 the
   arrow sits within 0.4 px of Elite's. As in Elite it is not drawn while a side panel is open, and it shows nothing
   for a small deflection (under about a sixth of full stick the arrow is still invisible: Elite's own look).

**Checks:** `cargo build --release`; `cargo test --workspace --release --no-fail-fast`: 2,336 passed, 1 failed (the known
test in the untouched upstream `skate-core`), 184 ignored. Two independent reviews of each part (pass, their faults fixed
and checked again); a run of the release exe shows the marker, the compass and the widget.

**What is ours, not Elite's (no recording shows Elite's: nothing was ever locked in them):** the look of the ring on the
destination, the compass's size and the way its dot moves. The compass's place is Elite's own. One Elite screenshot from
the seat with a planet locked from the left panel (ideally one with it ahead and one with it off to the side) would let
them be made Elite's.

**Known:** by day the widget is drawn covering what is behind it (so it stays readable; it only added light at first and
washed out to pale pink over the sky); how Elite's own looks over a bright sky is not in the recordings.

**Mouse sensitivity (U3; you, 2026-11-23: "Yes, use 0.27"):** the ship's mouse now has your own Elite's sensitivity,
0.2725 (it had Elite's default 1.0, which made it about 3.7 times as quick). It is a setting of the game: Settings, the
Camera tab, "Ship mouse sensitivity" (0.05 to 2; Elite's own default is 1.0). Your saved settings needed no change: a
settings file without the value gets 0.2725. The bindings files shipped with the mod are still Elite's own, unchanged.
Checked: an independent review (pass, nothing to fix); the release exe rebuilt and run; all tests: 2,338 passed, 1
failed (the known test in the untouched upstream `skate-core`), 184 ignored. Not checked by a test: dragging the slider
with a real mouse (a test can't drag); loading, saving and the slider's look were. Committed (e34b850, not pushed).

**Not released:** the players' zip in `release/` is still the build of 2026-11-17, without these two fixes.

### Sidewinder: what is yours to do (2026-11-17)

The mod is finished as far as the run of 2026-11-16 to 17 could take it (see "Finishing the Sidewinder" below). These
are the things only you can do or decide:

1. **Play it, with the keyboard and mouse and with a pad:** in the sandbox, Q, Mods, Sidewinder to spawn it; walk up and
   E to board; lift off with R (up thrust), throttle W/S/X, the mouse to pitch and roll, A/D yaw, Q/E/R/F thrust, Tab
   boost, the arrows for pips, Z flight assist, L gear, Insert the lights; J supercruise (dive at a planet for the
   glide), T to target a star ahead and J to jump; 1 and 4 the panels; come down slowly with the gear down; B to get out
   once landed (README's table has every key). The tests can't press a real mouse, keyboard or pad: say how it feels
   against Elite and anything wrong. The ship's sound is 14 dB louder than it was (Elite's own master effect).
2. **Two decisions about Elite's data** (this run decrypted nothing new, as you said): may the offline reader open (a)
   the input graph's file (it would close the strict trace gate's last 10 rows of 18,344: the keys' different delays)
   and (b) the game record with orbital cruise's seven zone numbers (the rule is built in `elite_sim::orbital` and waits
   for them; until then supercruise near planets uses the stand-in 30 km/s cap)?
3. **The notes are NOT committed:** `PROGRESS.md`, `CLAUDE.md`, `README.md`, `ARCHITECTURE.md`,
   `docs/spec-mods-sidewinder.md` and `docs/elite-flight-model.md` held other sessions' uncommitted changes when the
   run began, so only the code and the mod's files were committed (never pushed). Say the word and they are committed
   whole.
4. **When you next play Elite in the Sidewinder** (no hurry): one hyperspace jump gives the `FSDJump` line the mod's
   jump fuel is checked against (the spec's C16); the ship's tank was at 0.8 t of 2.
5. **Measuring is cut** (you, 2026-11-17: session 4 and measuring the sound against your recordings). If you ever want
   them: session 4 is still prepared (`.rea/tickets/measure/04-heat-gear-lasers-supercruise.md`, the scripts
   `re/elite/measure/scripts/m1..m10`); a recording at rest far from any star would tell whether Elite's low hum is
   the star's rumble; a recording of the gear going down with your head straight would check the left / right law.
6. **A question:** your three seat screenshots fit a field of view of 63.5, 59.0 and 58.5 degrees, but your Elite
   settings file says 56.249 (which the measuring sessions' videos confirm). Were they taken with another setting?
7. **To know:** the earlier study N1 opened four more of Elite's data files with the existing offline reader for the
   ship's numbers (the ship item, the stock modules, the prototypes, the laser's ammunition); only the Sidewinder's
   values were kept.

### Original Sidewinder assets

Plan in `ARCHITECTURE.md`.

- [x] T1 Build the offline FREA reader (`re/elite/tools/frea/`)
- [x] T2 Catalog the archives and decode the Sidewinder assets (2026-11-15/16). Every encrypted file opens (54,315 of
  54,320 pass every check; `re/elite/frea_export/catalog.csv`). Decoded into `re/elite/frea_export/sidewinder/decoded/`
  (**start with its `README.md`**) by `python -I re/elite/tools/sidewinder_decode.py`: 93 models (the hull's 4 levels of
  detail, every door, plate and vent, the shield, the hologram, the second hull `pg_sidewinder`, the landing gear and its
  pieces, the cockpit, its glass and UI overlays), 40 materials, 81 textures (full mip chains, DDS), the bones, and the
  disassembly of the 801 compiled shader variants the materials use. The vertex positions were solved from Elite's own
  vertex shader (a per-vertex exponent): all 93 models fill their own true-metre bounds within 0.02 mm and the hull
  matches your top screenshot. Sounds: `re/elite/frea_export/sidewinder_audio/` (1,710 Ogg files, proven by their Wwise
  name hashes). How the formats work: `re/elite/notes/sidewinder_assets.md`. Labelled gaps: the README's
  "Left open". Stopped here at your word: the next session integrates them into the mod (T3-T6).
- [x] T2b Animations and the missing sounds (2026-11-16). Elite animates the ship with rigs and state machines, not
  clips: all 10 files' rigs and 39 state machines decoded into `decoded/animation/` (gear, doors, hardpoint doors, heat
  vents, cockpit), with `kinpose.py` (the exe's posing and curve rules) and the formats in
  `re/elite/notes/sidewinder_animation.md`. Checked: the gear posed up and down sits right, 188 of 189 curve end values
  match the next pose. The landing gear's mechanical sounds were found: cues inside the gear curves, played as
  `<cue>_b/_e_landinggear` (120 files, roles `gear_*`, with their exact times). No separate heat, hull or shield-down
  alarm tone exists for the ship (all 1,309 sound banks indexed by `re/elite/tools/bank_index.py`): the game uses the
  voice lines, the shield break sounds and the cockpit lamps.
- [x] T2c Paint, hull choice, lights (2026-11-16). The ship item proves the game draws the hull `sidewinder` and the cockpit
  `sidewinder_cockpit` (the `pg_` ones are the CQC arena's); that cockpit was missing and is now decoded (119 models).
  The default paint job (`PaintJob_Sidewinder_Default`, found by Elite's own name hashing) is decoded: four colour keys and
  a mask texture; drawn by the shader's rule it matches your top screenshot (white hull, red stripes). All 70 lights are
  decoded (colour, brightness, range, cone). Notes: `re/elite/notes/sidewinder_paintjob.md`.
- [x] T2d The pilot chair (2026-11-16). Found how Elite names its encrypted files (a SHA-1 of the file's path; an item's
  content is its path plus `/Data`; `re/elite/tools/frea_name.py`), which proves the Sidewinder's chair is the standard
  `Cockpit_Chair` (file `781c37a2...`).
- [x] T2e Everything else decoded (2026-11-16). The chair's 91 clips (the game's own decoder run in an emulator; sampling
  checked against the game's samplers bit for bit) and its motion graph (which clip plays when, the inputs from the
  throttle / stick / trigger / buffets, the template values: a seated pilot gets `Mode_Ship`); every bone's parent and
  local transform; every state machine activity typed; the paint record, decal colours and shader constant layouts
  read from Elite's own code and tables; missing materials drawn with Elite's fallback `null`; the headlight's IES
  profile and lighting maths; the sound triggers (heat, hull, flight assist measured from a recording, rumble). Export:
  123 models, 52 materials, 94 textures, 0 problems. Handoff: `re/elite/frea_export/sidewinder/decoded/README.md`. The one
  value no static search found: the lighting manager's scene exposure input (the mod's own exposure stands in). Also
  not found: what raises the overheating malfunction message (`WarningMalfunction_b` plays with any module-malfunction
  message). An in-game overheating test would settle it, but it damages modules; the user chose to leave it
  (2026-11-16): don't run it or ask again unless they bring it up.
- [x] T2f The ship's UI decoded for a later session to build (2026-11-23, nothing in the engine yet): every cockpit
  panel, screen and HUD movie (Scaleform GFx: art, layouts, images, fonts, decompiled ActionScript), the data the game
  feeds each and the events back, placement on the cockpit and the look, the keys and focus rules, the scanner and the
  holograms, every UI sound (`.ogg`), the English text. Checked against the running game (session ui1). The galaxy and
  system maps are cut from the mod (the user). Handoff: `re/elite/frea_export/sidewinder_ui/decoded/README.md` (out of
  git); notes `re/elite/notes/ui_*.md`; tools `re/elite/tools/gfx/`.
### Finishing the Sidewinder (the run of 2026-11-16 to 17, `/project-coordinator`): done

Every part was written by an executor, reviewed by a checker and committed (never pushed): 42 commits, 0109fb9 to
68ee0cc (`git log --oneline --grep=Sidewinder`). The step-by-step record of the run is in `docs/progress-archive.md`
("The Sidewinder's finishing run"); each task's report is in the spec's Build log (`docs/spec-mods-sidewinder.md`);
the designs are `ARCHITECTURE.md` A5 to A8; the things to know when changing it are in `CLAUDE.md`'s Sidewinder Gotcha.

**What the ship has now**
- **Elite's own ship:** the real hull and cockpit, materials, paint job, decals and glass, drawn by Elite's lighting
  maths; the canopy cracks with damage and breaks when breached (1,031 files, 188 MB, every byte checked against the
  export; the old stand-ins are gone).
- **Lights:** all 84 with Elite's blink rules, the cockpit's panel lamps, the vents' glow; at night the headlights
  light the ground, grass, trees and buildings.
- **Moving parts:** the gear, its doors, the hardpoint doors and the vents by Elite's rigs and state machines at their
  real timings; the pilot's chair (throttle lever, stick, trigger, knocks) follows your controls.
- **The shadow:** Elite's own filter and softness, on the ship itself and, through each thing's own lighting, on the
  ground, plants, props and your weapon.
- **The sound:** the ship's own recordings, triggered by Elite's rules and mixed by Elite's audio engine's rules
  (its equaliser, distortion, compressors, tone generators and master effect ported from Elite's own code; the engine's
  inputs in Elite's units; the gear's legs heard from their own side).
- **The seat:** Elite's field of view; the HUD and the side panels at Elite's places in the cockpit, turning with your
  head; through a hyperspace jump you stay in the cockpit and see the tunnel outside.
- **Systems and numbers:** silent running, the scoop, the lights' switch, the canopy and its 5 minutes of oxygen; 70
  of the 99 stand-in numbers replaced from Elite's own records and 3 from its code; the gear's and scoop's limits.

**The checks at the end (2026-11-17)**
- `cargo build --release`: built. `cargo test --workspace --release --no-fail-fast`: 2,318 passed, 1 failed (the known
  test in the untouched upstream `skate-core`), 184 ignored.
- The spec's claims C1 to C20: all pass that can be run; C9 (the strict gate) misses 10 of 18,344 rows, as known.
- Two final reviews (the shadow as a whole; the whole mod with a full play-through and your Elite pictures side by
  side): pass with notes, the notes fixed. Without a ship the game is pixel-identical to the build before and no
  slower.
- The release zip (`release/Sandbox-Engine-sidewinder-2026-11-17-windows.zip`) was unzipped in a fresh folder with
  empty user folders: it found SpaceEngine through Steam, read all 1,030 of the mod's files from its own assets
  folder, spawned the ship and showed the cockpit.

**Open, and why**
- **The strict trace gate** (10 rows) and **orbital cruise** (its zone numbers): both need Elite data this run did not
  open (your decision, item 2 at the top).
- **26 numbers** still marked as stand-ins or community values (`elite_sim::data`, `re/elite/measure/BACKLOG.md`):
  measuring was cut.

**Known differences from Elite that stay**
- The picture: the scene's exposure is the port's own (the cockpit's brightness is a measured stand-in, good to a
  factor of two); the side consoles Elite shows lit stay dark; no steam, sparks or glass particles; the HUD's
  holograms are flat text and lines; the canopy's glass is drawn before the clouds; the hyperspace tunnel's look is the
  port's and it does not light the cockpit; side panels cannot be opened in the tunnel.
- The shadow: a tree or a building is shadowed as a whole; props follow the player's light point; the ground in the
  ship's full shadow is nearly black close up (the ground's own sky light here is about 6 % of sunlight); the
  headlights' beam casts no shadows.
- The sound (the last measurement, before measuring was cut): 2 dB over Elite on the beeps; 2 to 4 kHz is 9 to 19 dB
  too strong at full throttle and in the boost; Elite's low hum at rest is missing (not found among the ship's sounds);
  reverb is not built; the pulse laser's shot, the jump event, the targeting cues and the low-fuel line have no proven
  trigger and stay silent.
- The two gaps you said to leave: the scene exposure input, and what raises the overheating malfunction message.

**Scratch left on disk** (can go; nothing here is needed to play): `.optimize/scratch/run` (about 6 GB of test
pictures and logs the notes point at), `re/elite/work` (5.4 GB of recordings and study files: `re/` is out of git, keep
what you want of it), `target/agent` (28 GB of test builds; `target/release` holds the exe you play).

Reference screenshots: `re/elite/screenshots/2026-11-14_user/` (2026-11-14).
