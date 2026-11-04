# Clouds, new plan: one cloud system, the Flight Simulator look, fast

Status: the user asked for the whole plan (2026-11-03); steps 1-4 done; steps 5-7 revised after a
survey of other games' clouds (2026-11-03, "What other games do" below). Replaces the hand-over in
`spec-cloud-lod-and-popping.md` and comes before step 8 (gas giants) of
`spec-volumetric-clouds-weather.md`.

## Why a new plan

The user, 2026-11-03, with two videos of their planet RS 8514-3620-1-7-69 1 and a Microsoft Flight
Simulator 2020 cloud-settings video as the look they want:

1. Some clouds are dark from far away and turn white when the 3D clouds take over.
2. The switch from SE's flat clouds to the 3D clouds did not seem to make the game faster: drop it.
3. Start again with a plan for clouds that look great and run well.

What the tests showed:

- **The dark clouds** are the weather's rain shafts. The 3D pass drew them at every height, but
  while SE's flat clouds drew the clouds (above the hand-over), nothing hid the rain below those
  clouds, so it showed through them as dark grey patches (checked: white with `switch ShowRain off`).
  With the 3D clouds drawing the clouds the cloud tops hide it.
- **Where the frame time goes** (Ultra, 3440x1440, RTX 5080), on the user's planet at the place and
  date of their videos, time paused; whole frame in ms, the clouds' part in brackets:

  | View | Clouds off | With the hand-over | One system (step 1) |
  |---|---|---|---|
  | From 3,400 km | 7.7 | 24.2 (11.8, plus 4.7 for SE's flat clouds) | 21.5 (14.5) |
  | From 114 km | 8.3 | 27.3 (16.4, plus 2.4 flat) | 24.8 (15.9) |
  | 3 km up | 20.2 | 34.8 (11.1) | 34.1 (11.0) |
  | On the ground | 25.5 | 36.6 (11.1) | 36.7 (11.2) |

  Nearly all of the clouds' time is the march (`trace`): the ground shadow map, the blend, the near
  pass and the rain effects are under 0.6 ms together, and rain, fog and dust off changed nothing.
  This planet's high layer and the weather's veil cover nearly every ray. On it the hand-over made
  things slower: SE's flat clouds cost more than the march it skipped.
- Lessons:
  - Two looks for one cloud layer (flat far away, 3D near) always leave a seam somewhere: their
    colour, the weather and the rain must all match on both sides. One system avoids it.
  - Near the ground the ground itself costs 18-24 ms of the frame (a separate job); the clouds
    another 11.
  - The 3D clouds' softness from far away comes from their cloud map being coarse (about 10 km a
    texel on Earth, where SE's own tiles go down to about 1 km), not from their being 3D.

## The look we want (from the Flight Simulator video)

- **Cumulus** with crisp, billowing "cauliflower" tops and sides, flat darker bases, clear sky
  between them; sizes from a few hundred metres to a few kilometres.
- **Broken and overcast decks** seen from above as a field of rounded puffs, lit gold at sunset,
  with deep soft shadows in the gaps between them.
- **Storm towers**: tall bright towers with dark grey bases and grey rain curtains reaching the
  ground; the light under them dim.
- **Light**: tops bright white in the sun, the sides away from the sun grey-blue, a bright rim when
  looking towards the sun, a soft glow inside thin parts, dark creases between puffs ("ambient
  occlusion" in the video's settings), distant clouds fading into the haze.
- **No popping or crawling**: still when the camera is still, steady while flying.

## The design

### 1. One cloud system at every height

On High and Ultra the 3D clouds draw the active body's clouds from the ground up to space. SE's flat
clouds draw only bodies the camera is far from: between 40 and 60 radii away (the body a few
degrees across) the two cross-fade, and the 3D clouds fade in over a second when they first become
ready. Medium and Low keep SE's flat clouds everywhere. (Done on 2026-11-03 as step 1.)

### 2. Sharp from far away: SE's cloud detail where the view needs it

The clouds read SE's cloud maps at the detail the screen needs, as SE's flat clouds do:

- the whole-planet cloud map twice as fine (2048 a face: about 5 km a texel on Earth, the size of a
  cloud pixel from 10,000 km), made from SE's tiles on a worker thread, coarse first;
- the detail patch under the camera (today one 800 km patch, 800 m a texel, below 200 km) becomes a
  stack of three patches, each twice the size of the one inside it (on Earth 800, 1,600 and 3,200 km
  across), followed down from about 5,000 km.

### 3. Speed

- **A quarter of the cloud pixels each frame.** Each frame traces one pixel of every 2x2 block of the
  cloud image, taking turns; the other three come from the last frames, moved to where the camera
  sees them now (as today's blend already does). Where the last frames hold nothing (the view turned
  fast, something came out from behind a mountain) the traced neighbours fill in. About 4x fewer rays.
  The nearest clouds (the near pass, the first 4 km) are traced fully every frame, since they move
  fastest across the view.
- **Long steps through clear air.** The march takes long, cheap steps that only ask the cloud map
  whether a cloud could be there, and short detailed steps only inside clouds.
- **Cheaper sunlight.** The steps towards the sun beyond the first few use the cloud's rough shape
  (no fine noise); far from the camera fewer of them.
- **Fewer, longer steps far away**, where a pixel covers more cloud.
- Stops when the cloud is opaque and at the ground (as today).

### 4. Shapes

- Cloud types from SE's layer and the weather: fair-weather cumulus (separate puffs), stratocumulus
  (puffs merged into a lumpy sheet with gaps), overcast (a closed deck, flat base, lumpy top), storm
  towers (to the layer's top, with anvils), high thin layers (streaks and veils, no puffs).
- New noise: billows (rounded bulges in three sizes) on the tops and sunlit sides of cumulus and
  towers; wisps (stretched, swirled) at the bases and in thin layers.
- Denser inside the clouds, so edges are crisp instead of smoky.
- Each cloud type gets two height curves, kept apart: how dense the cloud is at each height, and how
  much its edge is eaten away there. A density curve rising over a short height gives flat bases; the
  erosion stays small at the base so it doesn't fray it.
- The tops vary from place to place with the weather (towers where convection is strong), not one
  height for the whole layer: a single top height flattens every silhouette.
- The big rounded shape comes first and stays in one piece; billows and wisps only change its surface
  (finer noise alone gives shredded fog, not cauliflower heads).
- Shapes are judged under flat, even light first: if they don't read as cumulus then, light won't
  fix them.

### 5. Light

- The sun's light through the cloud over several kilometres (today 3 km), so bases and the far
  sides go grey.
- Light bouncing inside the cloud (several bounces), the soft inner glow.
- Dark creases: the sky's light dimmed by how much cloud lies above a point (from the cloud map and
  the layer's shape, no extra march).
- The sky's colour from above, a little of the ground's colour on the bases, the bright rim towards
  the sun, sunset colours and haze from the atmosphere (as today).
- **A light volume** (as in blackrack's clouds for Kerbal Space Program from their Release 4, which
  work from orbit to the ground; `docs/ksp-clouds-notes.md`): a 3D grid around the camera, spanning the
  cloud layers and finer near the camera (his: a paraboloid out to the horizon), holding
  how much sunlight and sky light reaches each point through the clouds. It is refreshed a part at a
  time over several frames and moves with the camera. It gives the long self-shadow (grey bases and
  far sides) and the dark creases at once; the march's short sunlight steps stay for the fine detail.
  If it costs too much or flickers, a longer cone of sunlight steps instead (implementation notes).
- The ground's shadows use the same measure as the clouds' own light (how much sunlight gets through
  the cloud above), near the camera and far, so they don't change as the camera comes down.

### 6. Storms and rain

Rain shafts, fog and dust stay in the same pass as the clouds, so a cloud always hides what is under
it (the dark-cloud bug cannot come back). Storm bases dark from the self-shadow and the creases.

- Storm cells built from big parts: the base, the rising trunk, the anvil spreading at the top, an
  overshooting dome above it and a lean downwind; the noise only on their surfaces.
- Lightning lights the clouds for a fraction of a second: where a flash is, the clouds' light is not
  taken from the last frames (their shape still is), so flashes don't smear.

## Budget

Ultra at 3440x1440 on the user's RTX 5080, the clouds and weather effects together:

| View (the user's planet) | Today (one system, step 1) | Target |
|---|---|---|
| From orbit (3,400 km) | 14.5 ms | 2.5 ms |
| Coming down (114 km) | 15.9 ms | 3 ms |
| 3 km up and on the ground | 11 ms | 2 ms |
| Planet 2, looking around under thin high cloud | 5.5 ms | 2 ms |

High: about half of Ultra. (Estimates: a quarter of the pixels gives about 4x; most of the march's
samples are its steps towards the sun, so the cheaper sunlight gives another 2-3x; the new look
takes some of it back.)

## Steps

Each step builds, passes the tests, is checked in pictures (the test views below) and measured at
Ultra 3440x1440 before the next.

| # | Step | Done when |
|---|---|---|
| 1 | One cloud system: drop the hand-over (done 2026-11-03). | No dark clouds on the user's planet; no pops from orbit to the ground. |
| 2 | Speed: a quarter of the pixels each frame (done 2026-11-03: 2.3x cheaper). | Same pictures; clouds about 3-4x cheaper; no smearing while flying. |
| 3 | Speed: long steps through clear air, cheaper sunlight, far steps (done 2026-11-03). | Same pictures; budget met at today's look. |
| 4 | Sharp from far: the finer cloud map and the patch stack (done 2026-11-03, plus the composite sharpened by the cover and the weather's cloud pattern). | From orbit as sharp as SE's flat clouds; no seams. |
| 4b | Fixes from the user's videos (2026-11-03): cloud ghosts beside a planet's edge when orbiting it fast; the ground's cloud shadows fading near the ground (done 2026-11-03, see Results). | No ghosts while orbiting fast; shadows as dark near the camera as far away. |
| 4c | Learn how blackrack's raymarched clouds for Kerbal Space Program (the user's download, the Release 3 line with 2024 backports; the light volume is Release 4's) work: its public source, or else its decompiled code and shaders, kept in `re/ksp_clouds/` (out of git); notes in `docs/ksp-clouds-notes.md`; steps 5-7 updated from it. Methods only: none of its code, shaders or textures in the port (done 2026-11-03: its C# is public, its shaders were read as the bundle's OpenGL GLSL). | The notes cover its shapes, light volume, temporal reconstruction, orbit-to-ground detail, ground shadows, lightning and rain, and costs, with the numbers scaled to our planets. |
| 5 | Shapes: a reference switch first (`SE_CLOUD_REFERENCE=1`); the saw-tooth edges of the weather's own cloud; cloud types with their curves, varying tops, billows, wisps (done 2026-11-03: fields of heaps, see Results). | Test views look like the video's cumulus, decks and towers, and like the reference. |
| 6 | Light: the light volume (long self-shadow, sky light, creases), scattering octaves and the dense cores' glow, ground colour; the temporal pass's defences from blackrack's (done 2026-11-03, see Results; fresh transmittance and a bounded history left out). | Bases and creases dark, tops bright, as in the video. |
| 7 | Storms from parts, lightning (blackrack's schedule and light, scaled) without smears, rain tuned (his near-rain tricks); final measurements (the near pass's resolution too) and docs. | Budget met; README, PROGRESS, CLAUDE updated. |

Then the old plan's steps 8 (gas giants), 9 (integration) and 10.

## Test views

Fixed scripted views (time paused), each compared with the matching frame of the Flight Simulator
video and with the last step's picture:

1. Earth, fair-weather cumulus from 3 km, looking at the horizon (the video's "broken clouds").
2. An overcast deck from above at sunset (the video's "overcast").
3. A storm with rain from 2 km (the video's "thunderstorm").
4. Inside a cloud, and coming out of it.
5. From orbit (11,000 km and 3,400 km) next to SE's flat clouds: sharpness.
6. The user's planets: RS 8514-3620-1-7-69 1 (their videos) and RSN 0-7-460129-1254-197-12 2.
7. A flight from orbit to the ground, as a video: no pops or colour changes.

## Implementation notes

- Step 2: `trace` into a buffer of half the cloud size, each texel the 2x2 block's pixel of this
  frame's phase (order (0,0), (1,1), (1,0), (0,1)); `resolve` at the cloud size takes the fresh sample
  at the phase pixel and the reprojected history elsewhere, clamped to the fresh samples' 3x3
  neighbourhood, falling back to their bilinear upsample where the history is invalid. The near pass
  keeps tracing every pixel.
- Step 3: a coarse step (a fraction of the layer's depth, growing with distance) tests the cheap
  density (cover x profile, no noise); on a hit step back and march fine; back to coarse after a few
  empty fine samples. The light march's samples from the third on use the cheap density.
- Step 4: the cover cube at 2048 a face (from SE's tiles at about the cube's texel size, coarse levels
  first); `detail.rs` slots become three levels per layer (half-widths 400, 800, 1600 km on Earth,
  scaled by radius; R8 2D array of 12), `cover_at` takes the finest level whose patch and texel fit.
- Step 5: first a reference switch (`SE_CLOUD_REFERENCE=1`, done: every pixel at the solids' size
  traced every frame over the whole ray, twice the steps, no history, no near pass; slow) to judge the
  shapes and the light against. Then the saw-tooth edges of the weather's own cloud (terraces seen from
  1,000 km on the moon RS 8514-3840-6-8364-20 3.3: the same in the reference, gone with
  `SE_WEATHER_NEUTRAL=1`, still there without the simulation or the detail patches, not the march's
  step jitter). Then, from blackrack's clouds (`docs/ksp-clouds-notes.md` section 1): cloud types in a
  small height table (x the type, from the layer's cumulus share, the cover and the weather's
  convection; y the height fraction; RG8: density curve D(h) and erosion curve E(h)), one fetch, types
  blended by their curves (not by sampling the noise twice); the shape d0 = max(0, n + c - 1) with
  c = cover x D(h) and the noise lifted to [0.5, 1], so the cover sets a cloud's reach and thickness at
  once; erosion d = max(0, d0 - e) / (1 - e), e = strength x E(h) x (1 - detail); a 128^3 billow volume
  (1 - F1 Worley, amplitudes halving, cells doubling, mips; cells of about 1-2 km, as blackrack's
  physical sizes, not scaled by the radius) and curl moving only the noise lookup (about 3 % of a
  tile) for wisps; the top per place from the cover and the convection (today's `top` is a start); the
  noise's mip from the footprint, never finer than its mip 1 where the steps can't resolve it.
- Step 6: a light volume around the camera (blackrack's design, `docs/ksp-clouds-notes.md` section 3:
  his is 256 x 256 x (32 direct + 32 ambient) R16F, a paraboloid about a point on the camera's
  vertical below it, out to the horizon, direct light in 4 slices a frame with 50 steps a texel, the
  ambient 1 slice a frame blended 7 % falling to 1 %, re-centred only after a 3 % move with one
  reprojection): ours about 128 x 128 x 32 texels over the layers' height to start (settled by
  measuring); one paraboloid spans about 570 km on Earth (texels of several km), so finer near the
  camera, or a second nested volume; per texel the sun's optical depth to the clouds' top (a march
  towards the sun through the cheap density) and the sky light's visibility (a few upward directions);
  the direct part refreshed in slices over about 8 frames, the sky light over about 32. The march reads it for the shadow beyond its own five sunlight steps (only the part
  of the path past them, so no cloud is counted twice) and for the ambient. Scattering octaves as
  blackrack's four (sum over k of 2^-k exp(-2^-k tau)) and his in-scatter boost growing with the local
  density (dense cores glow, thin edges stay dim); the ground's colour into the bases. The temporal
  pass's defences from his: the transmittance always from the fresh trace (only the light accumulates),
  the clamp box widened to clear beside clear neighbours and its members within 10 % of the depth, a
  Catmull-Rom history fetch, the history kept in a bounded space.
  Fallback: a cone light march of six steps over about 6 km, the creases as the sky light times
  exp(-k x the cloud column above, from cover and profile).
- Step 7: storm cells from the weather's convection as an envelope (base radius, trunk, an anvil
  widening near the top, an overshoot, a lean along the upper wind) that sets the density curve and
  the top, the noise on its surface (blackrack's towers are two types whose curves dip mid-height and
  bump at 90 % for the anvil). Lightning (his, `docs/ksp-clouds-notes.md` section 6): random times at a
  rate per type, a few live flashes in a box of max(range, 2 x the camera's altitude), where the cover
  is > 0.1; 0.5 s with two or three bumps; the clouds lit by exp(-10 d / range) per sample, the ground
  and the rain too, ranges scaled for Earth (his Jool's ~150 km); the resolve skips the light history
  (not the transmittance's) in texels near an active flash (his Release 5 does the same). Near rain
  (`weather_fx`): streaks thinned by a random number against a cover ramp (no pops), falling along
  down + wind - the camera's velocity, faded 0.1 m from the camera and 1 m before a surface. Measure the near pass traced coarser than the far
  trace (Horizon Forbidden West draws its near clouds at half the far resolution).

## What other games do (survey, 2026-11-03)

The user had another AI survey how games made good-looking, fast volumetric clouds (Horizon Zero Dawn
and Forbidden West, Frostbite, Unreal, Flight Simulator, Red Dead Redemption 2, Ghost of Tsushima,
Star Citizen, blackrack's clouds for Kerbal Space Program and others). The port already follows
Horizon's design (Guerrilla's "Nubis"). What came of it:

- **Already in the port:** the energy-correct sum of light over each step (Frostbite's formula);
  forward and back scattering lobes plus one bounce octave; sunlight reddened by the atmosphere at each
  sample (as Unreal); each cloud pixel's distance weighted by where its cloud is, and the wind's drift,
  for moving the last frames into place; far clouds keeping their noise's mean see-through-ness.
- **Taken into the plan:** the two height curves and varying tops (step 5), the reference switch (step
  5), the light volume (step 6), storms from parts and lightning without smears (step 7), the near
  pass's resolution measured (step 7). For the ghosting fix (4b): rays that miss the cloud layers drawn
  clear whatever the last frames held, and true distances for moving the last frames (Star Citizen also
  weighs its history by motion and depth). If a ghost stays on the planet's side of its edge: trace
  every pixel there for a few frames.
- **blackrack's clouds in depth** (step 4c): `docs/ksp-clouds-notes.md`, from their public C# and the
  user's build's compiled shaders; folded into the implementation notes of steps 5-7.
- **Not taken:** Horizon Forbidden West's voxel clouds (artists build each sky with fluid simulations:
  not possible for generated planets; perhaps later for a few storm shapes); Ghost of Tsushima's sky
  painted into a texture over 60 frames (made for a game on the ground, not for flying around planets);
  Sea of Thieves's cloud models. Flight Simulator's method is unpublished: its look has to come from
  the shapes and the light.
- **To read** (links from the survey, not checked):
  - Step 5: Nubis 2015
    (https://advances.realtimerendering.com/s2015/The%20Real-time%20Volumetric%20Cloudscapes%20of%20Horizon%20-%20Zero%20Dawn%20-%20ARTR.pdf)
    and 2017
    (https://d3d3g8mu99pzk9.cloudfront.net/AndrewSchneider/Nubis-Authoring-Realtime-Volumetric-Cloudscapes-with-the-Decima-Engine-Final.pdf);
    blackrack's cloud settings
    (https://github.com/LGhassen/EnvironmentalVisualEnhancements/wiki/Raymarched-cloud-configuration).
  - Step 6: blackrack's light volume
    (https://github.com/LGhassen/EnvironmentalVisualEnhancements/wiki/Light-volume-%28Release-4%29);
    Frostbite, Hillaire 2016, section 5.8 on multiple scattering
    (https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/s2016-pbs-frostbite-sky-clouds-new.pdf).
  - Step 7: Nubis Evolved 2022, storms
    (https://d3d3g8mu99pzk9.cloudfront.net/AndrewSchneider/NubisEvolved/NubisEvolved-NoVideos.pdf);
    Nubis Cubed 2023, near and far resolution
    (https://d3d3g8mu99pzk9.cloudfront.net/AndrewSchneider/Nubis%20Cubed.pdf).
  - Ghosting: Star Citizen's monthly reports, February to April 2024
    (https://robertsspaceindustries.com/en/comm-link/transmission/19950-Star-Citizen-Monthly-Report-April-2024).

## Results so far

Clouds' ms at Ultra 3440x1440 on the user's planet after step 4: orbit 1.4, 114 km 2.4, 3 km 2.7,
ground 2.5 (the budget's 2.5 / 3 / 2 / 2); planet 2 looking around 2.6. The user's planet has one
high layer (14-18 km), not two decks as first noted; its far veil and honeycomb were the weather's
own cloud, now patterned (`weather_pattern`).

Step 4b (2026-11-03), checked at Ultra 3440x1440 in pictures:

- Ghosts: a clear texel's trace distance was capped at 10,000 km, so from farther out the resolve moved
  the last frames to the wrong place and kept their clouds beside the real ones (blocky squares out in
  space by the planet's edge). Now a clear texel stores the real scene distance, and `resolve_light`
  draws a texel clear when its ray misses the clouds' shell (and the 8 km of weather air below it),
  whatever the history holds. Checked orbiting the moon RS 8514-3840-6-8364-20 3.3 at 120 degrees a
  second (dev command `orbit DEG`) from 22,500 km at Ultra and High, and the user's planet from 3,000
  and 300 km: no ghosts; moving clouds a little softer than still ones.
- Shadows, four causes:
  - The shadow map (`shadow_gen`, 40 km around the spot under the camera) let 0.56 of the light through
    a thick cloud (a two-stream share), the cover's shadow farther out 0.25: both now give a quarter,
    and the far shadow of a thin veil follows the star's slant path as the map's does.
  - Both took the star's ray as climbing `dh / mu` over flat ground, and the map cut it at 40 km. On
    the moon (one layer at 14-18 km) with the sun 6 degrees up, the map's ray was already above the
    clouds where it should have met them: a square of sunlit ground under the cloud shadows, centred
    under the camera, seen from 20 to 400 km (the user's second video). Both now measure the ray over
    the round body (`cloud_cover.wesl` `climb_dist`), the map over the whole layer.
  - Both stopped at the terminator (`mu <= 0`), where the ground is still lit: the shadows ended in a
    line there (the user's screenshot). Past it they now go on as for a grazing star.
  - Under patchy cloud the map's mean is lighter than the cover's shadow of the same mean depth (the
    mean of exp(-od) is above exp of the mean od): on the user's planet's overcast spot (47.65 N,
    125.78 E) about 0.1-0.17 lighter. The map now has mips (`SHADOW_MIPS` 7, by `cover_down`) and the
    ground near the camera takes the cover's shadow times the map over its last mip (8 texels, 10 km):
    the map's shapes at the cover's mean. Gap left: a few hundredths.
  - Shadow-only pictures (map and cover shadows in separate colours, and their difference as grey)
    agree inside and outside the map at 171 km, 390 km and 22 km on the moon, and from 30, 20 and 8 km
    over the user's planet's overcast spot.
- Seen on the way: saw-tooth edges on some clouds seen from 1,000 km (fixed in step 5, below).

Step 5 so far (2026-11-03):

- The reference switch `SE_CLOUD_REFERENCE=1`.
- Saw-tooth edges (the moon RS 8514-3840-6-8364-20 3.3 from 400-1,500 km): the weather's own cloud
  was shaped only from a 2 % share on, with a pattern level that left 8 % of the area cloudy there,
  so its clouds ended in a cut along the weather's 2 % line, kinked at each texel of the weather cube
  (30 km there). `cloud_cover.wesl` `weather_shaped` now takes the level from a normal distribution
  (`normal_above`) and fades the clouds out with the share below 2 %; `maybe_cloud` tests a share
  over 0.2 %. Checked from 400 and 1,500 km straight down and 1,058 km at a slant: no teeth.
- Cloud types (`type_weights`, `type_depth`, `type_density`, `type_erosion`, `layer_shape`): fair-weather
  cumulus (1.8 km), stratocumulus (1 km), a deck (900 m), towers and veils, picked by the regional cover
  (`regional_cover`, the cover map's mip 2, ~20 km: SE's maps fill each cumulus blob with a cover near
  1, which the local cover took for an overcast); `light_column` reads their curves (`shape_above`).
- Found on the way, the reasons Earth's cumulus were soft see-through pancakes:
  - the shape value lay within 0.77-0.89 (se_core `print_shape_quantiles`: nearly normal about 0.833,
    deviation 0.041, the same to mip 3), so any cover over a fifth filled the whole area: now spread
    evenly (a logistic) before the threshold, so a share c of the area lies above 1 - c;
  - a solid cloud's optical depth was capped at the cover's (5.3, about 0.005 /m in a 1.8 km
    cumulus: fog): now 24 through its depth (0.02 /m), the mean seen from afar unchanged (the area
    share sets it);
  - the overcast closing filled clouds to the top wherever the local cover passed 0.75 (every blob of
    SE's maps): now only their lower part;
  - the shape noise's 1.5 km cells were larger than SE's cover texels near the camera (400 m), so the
    outlines were the map's: cumuliform layers now use 500 m cells (veils keep 1.5 km).
- Under a morning sun (Earth, central US, 2026-07-15 13:30 UTC) the cumulus are now solid with lumpy
  tops, crisp edges and sharp ground shadows; still too flat-topped and pillowy for the video's
  look. The user's planet (one stratiform layer) is unchanged from orbit, 114 km and 3 km.
- Cost (Ultra 3440x1440, Earth's cumulus field, clouds' GPU ms before -> after): 400 km 1.9 -> 2.2,
  12 km 3.5 -> 4.9, 4.5 km 12.3 -> 17.1, 1.5 km 19.3 -> 25.4. Nearly all of it is the near pass
  (10.6 -> 14.9 and 17.6 -> 23.4 ms at 4.5 and 1.5 km): inside a low cumulus layer it traces every
  pixel at full size with no history. Not caught before because the user's planet's clouds are at
  14-18 km. Next: the near pass's cost (half size or a quarter a frame, as the far trace), then the
  tops (taller heaps, billows), then step 6's light.

Steps 5 and 6 finished (2026-11-03), checked at Ultra 3440x1440 in pictures (Earth's central US and
Amazon cumulus, from below, among them, from 15 and 400 km; towering cumulus over the Caribbean; an
overcast deck at 62 S from above, below and under a low sun; the user's planet from 3,400, 114 and 3 km):

- Near pass: a quarter of its pixels a frame, at half the solids' size on Ultra too (full size cost
  2.8 ms among the Amazon's afternoon cumulus).
- Cumulus as fields of heaps (`heaps`): a coarse noise (1.5 km cells, read on the layer's base, and a
  second layout 1.37 times as large and turned, the two combined so the share stays exact: the
  noise's 6 km tile otherwise lined the gaps up in rows) picks where heaps stand, over the share
  `type_field` x cover of the area (cumulus 0.9, a deck 1), and each point's strength; the cloud reaches
  up to the types' blended top for that strength (`type_top`, `type_height`: fair-weather cumulus about
  0.6 times as tall as wide, from the field's share; taller with convection; towers to 4 km), from a
  flat base, under a 350 m skin (below the top and in from the side) that the erosion carves into
  billows (the detail noise at a 450 m tile for solid clouds: cells of 225, 112 and 56 m), sharpened to
  about 20 m, softer where the march's steps are long. Lumps (500 m cells) move the strength in 3D.
  Inside an optical depth of 60 through 0.6 of the types' depth (0.055 /m: real cumulus'). The far
  switch to the mean (`far_share`) for heaps from a quarter of a cell, before they shrink below the
  traced texels; the erosion's mean kept where it is averaged out. The clear-air test knows the heaps'
  ceiling (`heaps_ceiling`). Found on the way: SE's cover blobs (near 1 inside) filled from the ground
  up were slabs; heaps 1.8 km tall whatever their width were spires; mixing the types by density
  stood faint 8 km towers over every heap (now by height; halved the march's cost).
- Light: the light volume (`LightVolume`, `light_gen`, `light_at`): 160 x 160 x 32 RG16F twice, over
  the layers within 60 km of the spot under the camera (texels about 190 m at its centre, 1.3 km at its
  edge; up the layers finer near their bottom), 4 slices a frame; R the sun's optical depth from each
  point to the clouds' top (24 steps, then the mean), G the cloud's straight up (10 steps). The march:
  three steps (40, 80, 180 m, the first two eroded), then the volume; from 20-40 km on the volume at the
  point alone; elsewhere the old march and mean. Sky light through the cloud above as a thick slab
  passes diffuse light, 1 / (1 + 0.11 tau). Three bounce octaves (amplitude, extinction and phase
  halving) with the phases per ray, the bounces and the diffuse light brighter in dense parts, the
  ground's light (a mean land's albedo) on the lower parts, a thin step's light reused from the last
  one. Temporal: a 5-tap Catmull-Rom history; the clamp box from the neighbours within 10 % of the
  scene's depth. Left out: the transmittance only from the fresh trace (the history's premultiplied
  light would then glow where the fresh trace is clear) and a bounded history space (not needed so
  far). The volume costs 0.25 ms.
- The march is compiled twice (`HEAPS`): for a body whose clouds are all veils without the solid
  types' code, whose registers slowed every veil sample by a third (the user's planet's trace 2.1 /
  2.2 ms at 114 / 3 km instead of 3.0 / 3.3).
- Measured (Ultra 3440x1440, clouds' GPU ms): Earth's central US cumulus from 400 / 12 / 4.5 / 1.5 km
  1.4 / 2.2 / 2.8 / 2.8; the Amazon's afternoon cumulus from 15 / 2.5 / 1.8 km / inside 2.7 / 3.1 / 2.4
  / 2.4; the user's planet from 3,400 / 114 / 3 km / the ground 1.8 / 3.0 / 3.4 / 3.4 (after step 4:
  1.4 / 2.4 / 2.7 / 2.5).
- Left for later: far clouds' edges keep a faint grain (the steps' jitter at their crisp skin); the
  heaps are all about one size; a net of dark lines in the user's planet's veil seen from 3 km (older).

## Decisions

- One system, no hand-over (the user, 2026-11-03).
- Flight Simulator 2020's clouds as the target look (the user's video).
- Speed first (steps 2-3), so the new look has room in the budget.
- SE's flat clouds stay for far bodies and on Medium and Low.
- The survey of other games' clouds taken into steps 5-7 (the user, 2026-11-03).
- blackrack's clouds studied (step 4c, the user, 2026-11-03) and their methods taken into steps 5-7;
  not taken: their hand-over to flat clouds from 0.17-0.22 radii (ours stay volumetric to 40-60 radii)
  and their ground shadow from one density slice.

## Open questions

- The ground's cost (most of the frame near the surface) is outside this plan.
- Whether the near pass is still worth its cost once a quarter of the pixels are traced each frame
  (measured in step 2). Horizon Forbidden West draws its near clouds coarser than the far ones, the
  other way round: measured in step 7.
