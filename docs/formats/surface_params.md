# Surface parameters and biome-preset choice (SpaceEngine.exe 0.991)

Decoded from the exe with Ghidra (function addresses below). Implemented in
`se_core::surface_gen`; the biome preset matcher is in `se_core::biomes`.

## Where it runs

`seSolarSystem` initialisation (FUN_140513d20, "Initializing planetary system of star") calls the
generator FUN_1402f37c0 for every body (stars too) after the body's classes are set. The
generator fills every surface slot that the catalog left unset (all slots start at the
"unset" value -1e38, `FUN_1402fa1f0`), then assigns the biome preset (`seBiomePresetStorage::
AssignToPlanet`, FUN_140160c00) with the generated `SurfStyle`.

The surface slots live in the body record from offset 0x50 (`Randomize`) to 0x1b8 (`SurfStyle`),
4 bytes each, in the order of `surface_gen::KEYS` (the `Surface` reader FUN_14046f840). Slots
0x188/0x18c/0x190 are the `Clouds` block's `mainFreq`, `mainOctaves` and `Coverage`; slot 0x16c
has no key (read as a lava/ice cover fraction, never generated).

## Random numbers

Two thread-local LCG streams, `x = x * 1664525 + 1013904223` (32 bit):

| stream | functions |
|---|---|
| A (TLS+0x24) | uniform `u * scale * 2^-32` (f32, FUN_140444850), integer `u % n` (FUN_140443910) |
| B (TLS+0x20) | Gaussian: sum of 12 uniforms - 6 (FUN_140443d50); truncated normal (FUN_140443fd0) |

Truncated normal `(mean, sigma, lo, hi)`: `k = sqrt2 * sigma`, `a = erf((lo-mean)/k)`,
`d = erf((hi-mean)/k) - a`; if `d == 0` it returns `lo + (hi-lo) u`, if `u == 0` it returns `hi`,
else `mean + k * Phi^-1((a + d u + 1) / 2) / sqrt2` (Phi^-1 a double-precision quantile,
FUN_140444dc0). `lo > hi` is allowed and used.

Seeds. The generator sets stream A to record+0x48 and B to record+0x4c. Catalog bodies
(FUN_1404652f0): `h = h*131 + (signed char)c` over `"<header names>|<ParentBody>"` (the header
string as written, e.g. `Proxima b/Proxima Cen b/ALF Cen C b`), then
`A = h*0x14ab8d + 0x47f7e9f`, `B = h*0x56a18d + 0x44796b`. Procedural bodies take seeds from the
system seed and their index (FUN_1404f9210, e.g. `seed*0x2e3 + 0x1486`).

## Body classes

| class | record | rule (function) |
|---|---|---|
| object type | +0x20 | 3 Star, 4 Planet, 5 Moon, 6 DwarfPlanet, 7 DwarfMoon, 8 Asteroid, 9 Comet (table 0x140687370) |
| temperature | +0x28 | > 1000 Torrid, > 500 Hot, > 330 Warm, > 250 Temperate, > 170 Cool, > 90 Cold, else Frigid. First from T = (Teq^4 + heat*2^20)^0.25 (<= 3500) + greenhouse (FUN_1404d6850), then, before the generator runs, from the temperature model's average (+0x1860, FUN_14025f1c0; see climate.md) |
| atmosphere | +0x2c | pressure (atm) > 1000 Ultrabaric, > 10 Hyperbaric, > 0.1 Mesobaric, > 0.001 Hypobaric, > 1e-6 Infrabaric, else Airless; giants 7 (FUN_1404caaa0) |
| volatiles | +0x34 | asteroids and giants 9; Airless/Arid by atmosphere; with an ocean: depth <= 0.1 BumpHeight Lacustrine, <= 0.9 Marine, else Oceanic; global ocean Superoceanic (or Oceanic) (FUN_140505680). Glacial/Superglacial are never assigned |
| size | +0x38 | mass (Earth) > 0.002/0.02/0.2/2/10 (Neptunes 4/10/25/62.5, Jupiters 0.2/0.5/2.5/12.5 MJ) (FUN_1404cabf0) |
| bulk | +0x3c | from the composition: Asteroid, Ferria, Terra, Carbonia, Aquaria, Neptune, Jupiter, Chthonia, BrownDwarf, Sun, None (table 0x140687290) |
| surface | +0x40 | `InitSurfaceClass` (FUN_1404cb030): Jupiter/Chthonia 8, Neptune 9, Asteroid 7; atmosphere class < 2: 5 (aquaria 6); else by volatiles: Arid 4 (aquaria 6), Lacustrine/Marine 2 (aquaria 3), Oceanic/Superoceanic 1 |

## Generator (FUN_1402f37c0)

In order, each only where the slot is unset:

1. `craterOctaves` default `ceil(log_1.8(R / 0.03 km))` (0.015 km for asteroids), 1..30.
2. Body `Color` for solid bodies without one: HSL hue 15..40 deg (aquaria 12..42), saturation
   0.1..0.3 (0.1..0.4), lightness 0.4..0.8, desaturated x0.05 with probability 0.65 (0.4 for
   aquaria below 100 K). Four stream-A draws.
3. `SurfStyle = u(1)`; `Randomize = u(2) - 1` (x3) when all zero. Then the preset is assigned.
4. Giants (bulk > 4) continue with stripes and cyclones (not ported).
5. Solid bodies: `tropicLatitude = sin(obliquity)`; climate: a greenhouse "warmth"
   `w = k log10(P)` (k 2 above 180 K, else 6) `+ 10 P max(T - 350, 0)`;
   `climatePole = lerp(0.9375 (0.4375 above 353 K, aquaria 100 K), 0.5625, w - 1)`,
   `climateTropic = lerp(0.3125, 0.5625, w)`, `climateEquator = lerp(0.6875, 0.5625, w)`,
   `tropicWidth = 0.07 + 0.43 w^2`; vegetation ranges steppe/forest/grass (dry: 0..0.3125,
   0.25..0.5625, 0.5..0.8125; with ocean 0.3125..0.5625, 0.5..0.6875, 0.625..0.8125; -1 without
   life); `humidity`; `beachWidth` (Gaussian); an equatorial-ridge roll for airless bodies
   (moons near their parent's rings, moons >= 500 km, ringed planets); clouds' stripe/cyclone/
   coverage values when the body has clouds.
6. A branch per surface class with the truncated-normal ranges in `surface_gen::generate`
   (montes, dunes, hills, canyons, cracks, craters, volcanoes, rivers, rifts, maria, terraces,
   erosion, colour distortion, venus-type features, sea level).
7. Common tail: equatorial ridge; `snowLevel` from the height where the air reaches 258.15 K
   (90.65 K for aquaria) over `BumpHeight`; `heightTempGrad = 0.9375 - climateTropic`; polar caps;
   frequencies scaled by `R / 5700 km` (asteroids `(R / 80 km)^2`); maria only below 0.7 g and
   not under water; craters reduced by the atmosphere `(log10 P + 3) / 3`; `craterMagn x 10 km /
   BumpHeight`; relief faded above Teq 1000 K; `volcanoTemp`; volcano density scaled by
   `volcanoActivity + 1` (never generated, so unset activity zeroes it); landform shares
   (FUN_1402f9fa0) normalised to cumulative thresholds dunes | hills | hills2 | canyons | montes.

Not decoded, approximated in `catalog_input`: the atmosphere scale height and lapse factor used
for the snow line (record +0x1814/+0x1844), the flags at +0x15f8 and +0x165c, `detailScale`.

## Preset choice (FUN_140160c00, FUN_140161470)

A preset whose `UseFor` names the body wins. Otherwise every preset with weight > 0 whose classes
match is a candidate: bulk (`None` = any), temperature, atmosphere (`NonAirless` = any but
Airless), volatiles (`NonArid` = Lacustrine..Superoceanic), life, and `SunTemp` (lo, hi] against
the star temperature. The pick walks the cumulative weights in storage order to
`SurfStyle * total`. With no candidate SE logs "Can't find a suitable biome preset" and gives up
(the port relaxes constraints instead).

Star temperature: the generator passes the temperature of the body's host star (record +0x1d0c)
only when that star's index in the system is above 0; otherwise 5800 K. A single star is the
system's first object, so its planets are matched as if their star had 5800 K; components of
multiple systems use their own temperature (`surface_gen::preset_sun_temp`).

## Water

`InitSurfaceClass` also sets the lighting defaults a body's `Surface` block leaves out:
`SpecBrightWater` (record +0x1150) 0.65 for surface classes 1..3, 0.3 for class 4, 0 or 0.05
otherwise; `SpecBrightIce` 0.85 (classes 1, 2), 0.3 (class 4); `RoughnessWater` 0.35;
`RoughnessIce` 0.45; `SpecularScale` 1; `RoughnessBias` 0.5 (0.8 airless and asteroids, 0.2 icy
airless); `RoughnessScale` 1; `DayAmbient` 0.07; `Exposure` 1 (giants 1.5, asteroids 2.25, comets 0.5).
The `Ocean` block reader (FUN_14046eab0) takes `Height`, `Depth`, `Density`, `BottomCompositon`,
maps, `Opacity`, Hapke/Lommel and spot parameters; there is no ocean colour key. The planet
shader's uniform block has `WaterSurfColor`, `WaterFogColor`, `SpecParams` and `WaterParams`
(not traced to the code that fills them).

The port draws the ocean as SE's ocean sphere does: each terrain node of a body with an ocean that
reaches below sea level gets a second mesh, flat at sea level (a child entity, drawn from both
sides by `water_fragment` in planet.wesl), while the ground mesh keeps the seabed. Where Earth's
water map says land, the water surface is cut away (land below sea level stays dry). Seen from
above the surface is see-through (premultiplied alpha, still writing depth): it adds the sky's
reflection (the atmosphere's sky radiance) and the star's glint, GGX with `RoughnessWater`
(catalog or 0.35) and `SpecBrightWater` (catalog or the class default above), over `1 - Fresnel`
of what the ground pass drew below it. The ground pass draws the seabed where it is, faded to the
deep colour along its own light path (`exp(-depth (1 + 1/mu) / 120 m)`), so it keeps its depth
as the camera comes down (painted on the surface, it had looked a few metres down from any
height). The glint's gain (x10) was matched by eye earlier. Instead of skirts the sea patches
have flanges: one more row of the grid continued along the sphere over the neighbour, a hair
lower, drawn only from below (from above they would blend a second surface over the neighbour);
they close the cracks along coarser neighbours' straight edges. On bodies with an ocean the
terrain's fractal detail (hundreds of metres) fades out within 300 m of sea level and never moves
ground across it, so shallow seas such as the Dogger Bank (-22 m in Earth's bump map) stay seas.

Waves (SE draws them close up: at 16 m above Earth's sea the glint breaks into fine chop; from
30 m it is a smooth streak) are normals only: three swells 12-27 m long and five octaves of
wind-stretched value noise (2 m to 12.5 cm cells) drifting downwind, tiled over 4096 m of the
node's cube face. Waves under about a pixel fade into the glint's roughness, so all of them
together give SE's roughness from afar.

The camera can go under water as in SE (down to the seabed; SE's `ClipHeight` 1.5 m above it).
Under water (the camera's height above the sea sphere is worked out per node in the vertex
shader; within a centimetre of the surface far nodes may disagree, so each branch mirrors the view
into its own side) the view is fogged: daylight dims with depth by `0.025 + 0.35 (1 - c)^3` per metre (`c`
the deep colour over its largest channel; red goes first for Earth's sea), sight lines by 0.04
more, and the water glows in its deep colour, brighter looking up. The surface seen from below
shows the sky in Snell's window (refracted, Fresnel) and the water below reflected outside it.
The star's lens flare under water moves into Snell's window and dims along its path through the
water. Compared by eye with SE at 16 m above and 24-27 m below Earth's sea at noon and sunset;
SE's water shader is in the encrypted Shaders.pak, so this is a reimplementation. SE's underwater
views are about 1.7 times brighter at sunset, so under water the light is scaled up as the
daylight above fades, `sqrt(0.5 E_sun / E_level)` between 1 and 2. Near the surface the near
plane comes in to a millimetre (`camera::near_plane`), and the HUD shows the depth under water.
Ships rest on the sea's surface (`Terrain::surface_height`).

Inland seas below sea level keep the ocean sphere's level: the Caspian (its bed and shores near
-28 m in the bump map) is drawn with its water at 0 m, as SE's single ocean sphere would.
