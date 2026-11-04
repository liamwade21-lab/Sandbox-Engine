# blackrack's raymarched clouds for Kerbal Space Program: how they work

Notes from studying blackrack's volumetric clouds (EVE-Redux, the clouds plan's step 4c, 2026-11-03),
for the port's own clouds (`docs/spec-clouds-v3.md`). Methods and numbers only: none of the mod's code,
shaders or textures are in the port.

## Sources

Everything studied is in `re/ksp_clouds/` (out of git: someone else's code, no licence):

| folder | what | from |
|---|---|---|
| `src/` | the C# of the user's build's line (`COMMIT.txt`: the `fix-v3-deferred` branch, 2024-06-06) | the public repo github.com/LGhassen/EnvironmentalVisualEnhancements |
| `src-release5/` | the newer C# with the light volume (`release-5` branch, 2026-08) | the same repo |
| `shaders/`, `glsl/` | the compiled shaders of the user's `eveshaders.bundle`, exported with UnityPy 1.25.4 (PyPI); the bundle holds Direct3D 11 bytecode and OpenGL Core GLSL for every shader, so the GLSL (Unity's cross-compiler output: locals `u_xlatN`, uniform names original) was read directly; `glsl/` holds one rich keyword variant per pass, `*.variants.txt` all variants | `C:\Users\liamw\Downloads\v3_01_07_2024` |
| `wiki/` | the mod's wiki (git clone) | github.com/LGhassen/EnvironmentalVisualEnhancements/wiki |
| `notes/` | the three study notes behind this file, with a file:line reference for each fact | |

The configs were read in place (`v3_01_07_2024\GameData\StockVolumetricClouds\`). The repo has the C#
but not the cloud shaders: those exist only compiled, in the bundle.

**Which version.** The download is the Release 3 line (EVE 2.2.x, first out 20 Aug 2023) with backports
up to 1 July 2024 (the changelog's dates are day/month/year). The **light volume came in Release 4**
(22 Dec 2023) and is **not in the user's build**: its parts below come from the `release-5` C# and the
wiki (its shaders are not public). The v3 C# also sets an "orbit mode" for the temporal passes that the
bundle's shaders never read, so the C# is a little newer than the bundle.

**Scale.** Kerbin's radius is 600 km (Earth 6,371 km, 10.6x). Nothing in the mod scales by the
planet's radius: each planet's config sets plain metres (Jool, 6,000 km, is retuned by hand). Two kinds
of numbers:
- *Cloud-scale* ones (cloud heights 2.5-7 km, noise tiles, step lengths, raindrops) are physical and
  Earth-like already: they carry over as they are.
- *Planet-scale* ones (the hand-over height, the light volume's reach, lightning's spawn range, the
  far shadow) are tuned for a small planet: scale them, or (better) make them geometric.

## 1. Cloud shapes

**Cloud types.** A layer holds up to 16 types. A type is an altitude range, a noise tile (metres per
3D-noise repeat), a density (extinction per metre), a detail (erosion) strength and **one coverage
curve over the type's height** (Hermite keys). Two 2D maps over the planet, read by direction: the
coverage (alpha) and the type (red, 0..1). Type value t picks types floor(t(N-1)) and the next and lerps
everything between them (density, detail strength, the two types' noise values, their curves). The
curves are baked into one **128 x 128 R8 table** (x: type position, y: height across all the types'
range; each type stretched to the pair's lerped bottom and top), one bilinear fetch a sample. A cloud is
a radial column: maps and curves don't change along the ray except with height.

Kerbin's low layer (2D layer at 2,700 m; noise Worley, 8 octaves, 3 cells, contrast 1.5, lift 0.5;
detail tile 785 m):

| type | altitude m | tile m | density /m | detail | curve (height fraction: value) |
|---|---|---|---|---|---|
| stratus | 2,500-2,800 | 1,852 | 0.05 | 0.5 | 0: 0.01, 0.31: 0.95, 0.99: 0 |
| cumulus 1 | 2,500-3,100 | 1,853 | 0.1 | 0.5 | 0.02: 0, 0.21: 0.94, 0.97: 0 |
| cumulus 2 | 2,500-3,600 | 1,854 | 0.1 | 0.5 | 0.02: 0, 0.09: 0.96, 0.47: 0.89, 0.97: 0 |
| cumulonimbus top | 2,500-5,500 | 3,000 | 0.1 | 0.3 | 0.02: 0.98, 0.68: -0.17, 0.91: 0.40, 1.0: 0 |
| cumulonimbus core | 2,500-5,600 | 4,000 | 0.2 | 0.2 | 0.02: 0.98, 0.67: 0.47, 0.94: 0.97, 1.0: 0 |

Cumulus curves rise to ~0.95 within 10-20 % of the height (flat bases, rounded tops); the towers dip
in the middle and come back as an anvil bump near 90 %. The high layer (6,100-7,000 m) has the same
three cumulus-like types with density 0.01, detail 0.75-0.9, a separate Perlin detail noise and curl.
Rain, fog and thunder are their own very thin layers from the ground to 2,500 m (section 6).

**Density at a point** (`glsl/EVE_RaymarchCloud.p0.frag`; c = coverage x curve):
1. If the curve or the coverage < 0.01: zero, no noise read.
2. Curl: the noise lookup point moves by (curl texel x 2 - 1) x strength (Kerbin's high layer: 50 m on a
   2 km tile), the curl texture read at its mip 1. Only the noise lookups move, not the maps.
3. Base shape: **d0 = max(0, n + c - 1)** for the noise n (each type at its own tile, lerped). With the
   noise lifted to [0.5, 1], holes open only where c < 0.5; above, the cloud is solid and only its
   density varies. One value sets both how far the cloud reaches and how thick it is.
4. Erosion, only where d0 > 0: e = strength x (1 - detail noise), **d = max(0, d0 - e) / (1 - e)**.
5. sigma = type density x d. A step with sigma x step <= 0.01 adds nothing (thin clouds vanish: not
   worth copying).

**Noise textures**: 128^3 R8, tileable, full mip chain, generated at load. Worley as **1 - F1** (rounded
billows), octave k amplitude 0.5^(k+1), cells doubling per octave, then contrast, brightness and lift;
Perlin fbm optional, the two multiplied. The detail noise is the same kind of texture (or the base one
again) at a finer tile. Curl: 128^3 RGB565 from a Perlin fbm's differences. Anti-tiling (on by default):
two 2D taps of a blue-noise image pick offsets for two 3D taps cross-faded, so each noise lookup costs
two 3D and two 2D taps ("performance intensive", the wiki).

**Ours**: SE's cover map times the weather is the coverage; `layer_density` thresholds a Perlin-Worley
shape noise by the area share (the same idea as step 3, written as a remap) and erodes with a detail
noise; one profile per layer from its cumulus share; no type map, no per-type curves.

## 2. The march and detail from orbit to the ground

- **Shell**: the ray against the lowest bottom's and the highest top's spheres; from outside a ray that
  dips into the hollow gives two segments, each its own pass; cut by the scene depth and the planet.
- **Steps**: step = min(45 m + 0.0022 x distance, 180 m) (Kerbin's defaults; the cap from 61 km; weather
  layers 300 m + 0.001/m, cap 1,000 m; Jool 750 m, cap 3,000 m). Density read at each step's end.
  **No empty-space skipping** in this build (only the noise reads are skipped); stops at transmittance
  < 0.02 or 4,096 steps; no maximum distance.
- **Jitter**: the first step is a blue-noise fraction of a step scaled to the step length at the ray's
  start (one base step at the camera, a full 180 m from 31 km on). Spatiotemporal blue noise (NVIDIA's
  STBN, `stbn.R8`, 128 x 128 x 64 R8 by its size), the slice changing once per temporal-upscaling cycle.
- **Detail with distance**: no noise is faded out with distance; detail goes through the noise mips
  (an integer level from the ray's screen-space derivatives, never below mip 1) and the growing steps.
  The mip estimate has an odd 1/128 vs 128 factor at the ray's ends that keeps near samples at mip 1;
  unclear if intended.
- **Orbit**: the camera's altitude switches technique. Below 100 km (0.17 radii on Kerbin) volumetric
  only; a flat 2D cloud layer fades in over 100-107.5 km and the volumetrics out over 107.5-130 km; from
  130 km (0.22 radii) and in the map view only the flat layer (Jool: 3,000-5,300 km, 0.5-0.9 radii).
  **The port keeps its volumetric clouds out to 40-60 radii**, so this part doesn't carry over: the mod
  never draws its volumetrics from far away.
- **Steps per ray** (estimates, Kerbin's low layer 2.5-5.6 km): ~58 straight up, ~160 at the horizon,
  ~460 along the layer inside it, ~17 down through it from orbit, ~680 along the limb.

## 3. Light

**The user's build (no light volume):**
- Sun: a short march per in-cloud sample, 4 steps over 800 m by default (Kerbin's high layer 1,800 m;
  rain and fog 1 step over 7 km), each step a full density evaluation. Two modes: a sample at least
  2 km away, behind < 75 % transmittance and on screen last frame takes N jittered steps; any other
  (near, at a cloud's front, newly seen) 2N unjittered half steps: noisy only where the history can
  average it. Another layer's shadow comes from one 2D-cover lookup where the sun ray crosses it.
- **Four scattering octaves** on the sun's transmittance: sum over k = 1..4 of 2^-k exp(-2^-k tau)
  (0.94 at tau 0; the tail exp(-tau/16)/16: a soft inner glow without another march).
- A powder-like boost to the in-scatter that grows with the local density: dense cores glow, thin edges
  stay dim (my reading: about 6x, 10x, 13x per unit transmittance at sigma 0.05, 0.1, 0.2 /m).
- Phase: a near-isotropic Henyey-Greenstein (g 0.1) plus 0.3 of a Cornette-Shanks lobe (g 0.9): 0.64
  away from the sun, 0.84 side-on, 6 on the sun (a narrow silver lining). Release 4: four HG lobes
  (single scattering 0.95 / 0.05 and 0.8 / 0.15, multiple 0.2 / 2.0 and -0.4 / 0.2, eccentricity /
  weight).
- Ambient: flat, cloud colour x opacity x 1.8 x a sky multiplier (no height or occlusion term): the
  "dark creases" our plan wants are not in this build.
- Planet shadow: analytic, from angles, at the optical-depth-weighted mean cloud position of the ray,
  a penumbra of about +-0.05 rad, on the night side only. No sunset colours in the cloud shader (they
  come from the Scatterer mod).
- Colour stored through a filmic curve into [0, 1] (radiance x 5) for the temporal passes.

**The light volume (Release 4/5; C# and wiki):**
- One 3D R16F texture, **256 x 256 x (32 direct + 32 ambient) slices**, 8.4 MB, twice (flip/flop).
- **Placement**: a paraboloid (hemisphere) projection about a point on the camera's vertical below
  the camera, the slices over the layers' heights, so it reaches the horizon and is densest overhead.
  The point and reach come from the camera's radius a = max(r_cam, R + 2,500 m): horizon dh =
  sqrt(a^2 - R^2), dc = dh + sqrt(R_top^2 - R^2), reach = sqrt(dc^2 - (dh dc / a)^2): about 140-146 km
  on Kerbin near the ground, **about 570 km on an Earth-sized planet** (clouds to 12 km), so one texel
  is ~250 m overhead on Kerbin, over 1 km at the horizon, and several times that on Earth.
  `maxLightVolumeRadius` caps the reach; past it the old 2D shadows fade in.
- **Schedule**: the direct light (the sun's transmittance, 50 march steps a texel) 4 slices a frame,
  all in 8 frames; the ambient (sky visibility, stochastic directions from a unit-vector STBN) 1 slice
  a frame, 32 frames, blended into its history at 7 % falling to 1 % per cycle. Re-centred only when its
  point moves or its reach changes by more than 3 %: then the whole volume is reprojected once and the
  ambient's blend restarts. It also turns with the slowest layer each frame. Faster under time warp.
- **Use**: the march reads it for the long self-shadow and the ambient and keeps its short sun march
  for fine detail; Release 5's ground shadows come from it too (section 5). Cost: about 13 M density
  evaluations a frame for the direct part (4 x 65,536 x 50).

**Ours**: a light march (steps doubling from 100 m over 3.1 km near the camera, the cloud above's mean
from 20-40 km on), forward and back lobes plus one bounce octave, sunlight reddened by the atmosphere;
step 6 plans a light volume of about 128 x 128 x 32.

## 4. Temporal reconstruction and ghosting

- **Layout**: the trace is the screen divided by a block (x1 1x1, x2 2x1, x4 2x2, **x8 4x2 the
  default**, x16 4x4, x32 8x4, x64 8x8; Release 4 adds 3x1, 3x2, 3x3, 4x3; Release 5's default 3x3).
  One pixel of each block a frame, in a spread-out order (4x2: 0, 4, 2, 6, 3, 7, 1, 5), its ray through
  that pixel's centre with that pixel's full-resolution depth. Wiki: x1 about 20 fps on a decent GPU,
  x8 60-90 fps; x8 suits 1080p-1440p, x16 looks worse, x32 bad at the horizon. (Ours: a quarter of the
  cloud texels, 2x2, at half or quarter of the screen.)
- **Motion**: each traced ray writes a motion vector: the screen motion of the optical-depth-weighted
  **mean cloud position** of the ray (-99 for no cloud), with the planet's motion, the floating origin,
  the layer's turn and the noise's scroll in the previous view matrix. With several layers the more
  opaque one's vector is kept. (Ours: the trace's distance, weighted the same way.)
- **Resolve** (full resolution):
  1. Clear at once if the ray misses the shells or the cloud starts behind the scene (as our 4b fix).
  2. A box (min/max of colour and transmittance) from the 3x3 traced neighbours whose scene distance is
     within **10 %** of this pixel's; the box **widens to include clear** where a depth-valid neighbour
     is clear; all nine if none pass.
  3. History at the reprojected position with a **5-tap Catmull-Rom** (sharper than bilinear under
     repeated resampling), clamped to the box.
  4. A mismatch weight m = clamp((|mv_history - mv| - 0.001) x 100, 0, 1) (screen units): where the
     motion changed, fresh data wins. A traced pixel takes 0.4 + 0.6 m of the fresh value, an untraced
     one m of the fresh bilinear upsample. **Alpha (transmittance) is always the fresh value**: only the
     colour accumulates, so no see-through ghosts.
  5. The history is stored and clamped in the bounded filmic space, so the bright silver lining doesn't
     poison it.
- **Orbit mode** (v3 C#: on when the camera is above the top layer by more than twice its height,
  ~21 km on Kerbin): its effect is not in the bundle's shaders. Release 5 adds a pre-pass choosing which
  pixel of a block to trace, motion and depth dilation passes, and "unstable masks" where lightning
  (and the cloud painter) are, so flashes don't smear (as our step 7 plans). Changelog fixes: line
  artifacts at high upscaling, black artifacts at the horizon and in motion, trails on fast turns,
  smearing of overlapped layers far away.

## 5. Shadows on the ground

- **The user's build**: no shadow map. A full-screen pass per 2D layer multiplies the lit colour
  (ambient included). Per pixel: the sun ray from the ground point meets the layer's sphere
  (R + the 2D layer's altitude, 2,700 m on Kerbin), and **one sample of the volumetric density
  function** there (the 2.2.0 "closely matching volumetrics"), at a fixed height fraction (0.065 on the
  low layer), without the type's density or height curve. Strength 1 - 0.75 x (0.7 saturate(4d))^2,
  so the darkest ground is x0.63 (the high layer x0.85); layers multiply.
- **The mask**: only points below the layer's sphere, a 100 m fade under it, and **saturate(100 x the
  sine of the sun's elevation)**: no shadow at the terminator, full from 0.57 degrees (the 2.1.0 fix for
  "a harsh line at the terminator"; the port's 4b line was the other way round: shadows cut off while the
  ground was still lit).
- **Far**: from space a Unity projector reads the 2D cloud texture; the switch between near and far is
  a hard cut at the volumetrics' fade (100-130 km); no seam fix in the changelogs.
- Release 4 moved long-distance shadows to the light volume; Release 5 renders all of them into a
  half-resolution R8 mask (2D layers, the light volume, eclipses) that darkens only the sun's light, and
  fixed ground shadows "sliding around with zoom at some distances".
- **Ours** (after 4b): the shadow map near the camera (40 km) with the cover's mean, the cover's shadow
  farther out, both through the round planet's sun path, past the terminator; the sun's light only.

## 6. Lightning and rain

- **Lightning**: a schedule of random times per 100 s of game time at N a second (Kerbin 2/s); a strike
  needs fewer than 4 live bolts and lands at a random point in a box of half-size max(20 km, 2 x the
  camera's altitude) around the camera, raised to the spawn altitude (2.5 km), accepted where the cover
  is > 0.1 and a random number < the type's lightning frequency (only the thunder and rain types).
  Lasts **0.5 s**, intensity falling linearly, no flicker. The clouds' light: each march sample adds the
  flash colour x 2.5 x intensity x life fraction x **exp(-10 d / range)** (range 10 km on Kerbin: an
  e-fold of 1 km; Jool 150 km), unshadowed, no phase. The ground and the rain get a real point light
  (range 10 km). The bolt: a camera-facing quad from a 4x3 sheet (2 x 2.5 km on Kerbin, 35 km on Jool),
  hidden behind cloud by a small march (a 32x32 grid per bolt) of the transmittance T to it: alpha x
  (1 - 2^(-21.6 T)). Thunder starts with the flash (the real delay is an option, off).
- **Rain in the clouds' pass**: rain, thunder and fog are very thin layers from the ground to 2.5 km
  (density 0.001-0.002 against the cumulus' 0.1), no detail noise, their noise scrolled down at 10 m/s,
  long steps (300-1,000 m), one sun step over 7 km, on in timed windows (8 h of every 18 h on Kerbin).
  Drawn in the same march as the clouds, as our plan has it.
- **Rain near the camera**: one static mesh of quads at random positions wrapped by fract() into a cube
  around the camera (rain 200,000 in 70 m: 0.58 a cubic metre; snow 400,000 in 100 m), each with a
  random number compared with a cover ramp ((cover - 0.3) / 0.45 x the type's density) so the field
  thins without pops; streaks 0.75 m x 3 cm (width x sine of the angle between the view and the fall),
  falling at 12 m/s along down + wind - the craft's velocity with a slow wobble; alpha 0.18; faded over
  0.1 m at the near plane and 1 m before a surface. Snow 5 cm, 0.75 m/s.
- Cockpit droplets (inside the craft only) and wet surfaces (Release 5) are outside the port's plans.

## 7. What it costs

The mod gives no milliseconds; the wiki gives frame rates (x1 ~20 fps, x8 60-90 fps). x8 at
3440 x 1440 traces 619 k rays a frame. A dense step costs roughly 75-135 texture reads with its light
march (each light step is a full density evaluation: two types' noise, anti-tiling, maps, erosion).
Its savings: one ray per 8 pixels, noise reads skipped where the curve or cover is empty, the light
march cut to N jittered steps where the history averages it, and the 2D layer from 130 km up.

## What the port takes from it

(Folded into the plan's steps 5-7, `docs/spec-clouds-v3.md`.)

- **Step 5, shapes**: the height table (types blended by their curves in one fetch, ours with two
  channels: density and erosion); the threshold shape max(0, n + c - 1) with the noise lifted, so the
  cover sets both a cloud's reach and its thickness; erosion as max(0, d - e) / (1 - e) scaled by the
  erosion curve; a 1 - F1 Worley billow noise with mips (cells about 1-2 km, as the mod's physical sizes);
  wisps from curl moving the noise lookup only (about 3 % of a tile); the mip from the footprint, not
  the mod's start/end trick; no 0.01 cut.
- **Step 6, light**: the four-octave sum for the sun's transmittance and the density-dependent
  in-scatter boost; the light volume's layout (a paraboloid about a point below the camera, reaching
  the horizon) and schedule (direct in 8 frames, ambient in 32 with a 7-1 % blend, re-centre after a 3 %
  move with one reprojection); finer than 256^2 near the camera on Earth-sized planets or a nested
  second volume, since one paraboloid spans 570 km there; two sun-march modes (jittered and short where
  the history averages it).
- **Temporal**: transmittance always fresh (only the colour accumulates); the box widened to clear
  beside clear neighbours, its members within 10 % depth; a Catmull-Rom history fetch; the history in a
  bounded space; a fresh-data weight from the change of motion. (The march's first-step jitter scaled
  to the step where the ray enters, as the mod's, was tried on 2026-11-03 against saw-tooth cloud edges
  seen from 1,000 km: no visible change, so left out.)
- **Step 7**: lightning's schedule (random times, a box of max(range, 2 x the camera's altitude), cover
  and type tests, a few live bolts, 0.5 s with 2-3 bumps of our own), its light exp(-10 d / range) on
  the clouds, the rain and the ground, ranges scaled for Earth (Jool's ~150 km); the bolt faded by the
  transmittance to it; rain streaks wrapped around the camera with the random-number-against-cover
  density, fall along down + wind - the camera's velocity, the near and surface fades.
- **Not taken**: the hand-over to flat clouds from 0.17-0.22 radii (the port keeps its volumetrics far
  out and is sharp from orbit since step 4); the shadow from one density slice; multiplying the final
  colour for shadows; the hard near/far switch; the 0.01 optical-depth cut; anti-tiling by two 3D taps
  (a hashed offset per big cell instead, if tiling shows).
