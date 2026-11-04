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
