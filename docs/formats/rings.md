# SpaceEngine planetary `Rings` blocks

Sources: extracted catalogs `planets/*.sc` (13 `Rings` blocks in 5 files) and
`data/textures/planets/Rings.pak` (6 PNGs). Units marked "inferred" are guesses from
context and comments, not from engine code.

## Where blocks live

A `Rings` block is a child of a planet-like object (`Planet`, `DwarfPlanet`, `Asteroid`,
`Comet`). The key line is `Rings` with `{` on the next line. Presence of the block enables
rings; `NoRings true` sits beside it on some objects and disables them.

| File:line | Owner | Texture | Notes |
|---|---|---|---|
| SolarSys.sc:1196 | Planet "Jupiter" | Jupiter-rings.* | |
| SolarSys.sc:1772 | Planet "Saturn" | Saturn-rings.* | most complete block (all keys except colors) |
| SolarSys.sc:2568 | Planet "Uranus" | Uranus-rings.* | |
| SolarSys.sc:2962 | Planet "Neptune" | Neptune-rings.* | dim values (0.15) |
| SolarSys.sc:3479 | DwarfPlanet "Haumea" | (commented out) | 2252-2322 km |
| SolarSys.sc:3571 | DwarfPlanet "Quaoar" | none | 4147-4149 km, opacity 0.003 |
| KuiperBelt.sc:54 | Comet "Chiron" | none | 324-346 km |
| KuiperBelt.sc:316 | Asteroid "Chariklo" | Chariklo-rings.* | |
| Planemo.sc:49, 75, 163 | Planet "OTS 44 b", "Cha 11091-77344 b", "Proplyd 133-353 b" | none | procedural rings (`frequency`, `densityScale`, ...) |
| ExoPlanets-custom.sc:82 | Planet "Proxima c..." | none | only `OuterRadius 360000` |
| SpaceEngine.sc:97 | Planet "Menu Planet 1" (class Jupiter) | none | only block with the colour tuples and procedural keys |

## Keys (union over all 13 blocks)

Counts are from `docs/formats/sc_vocabulary.md` (13 blocks). "Default" is the value used by
`RingsParams` when the key is absent; it is a port choice, not an engine fact.

| Key | Blocks | Type | Unit / range (observed) | Inferred meaning | Example |
|---|---|---|---|---|---|
| Texture | 5 | string | file stem with `.*` wildcard | ring radial texture, see below | `"Saturn-rings.*"` |
| InnerRadius | 12 | number | km; 2 to 1.2e5 | inner edge of ring disc, distance from planet centre | `64535` |
| OuterRadius | 13 | number | km; 2.3e2 to 4e6 | outer edge of ring disc | `318120` |
| MeanRadius | 1 | number | km | radius of the densest part, used by RingsWinter effect (comment in Saturn) | `106000` |
| EdgeRadius | 4 | number | km | outer edge of the dense part (Saturn comment: skips E ring) | `137000` |
| ChartRadius | 2 | number | km | outer radius shown in chart/map mode | `137000` |
| Thickness | 6 | number | km, half the slab (SE's shaders use `2 x` as the full thickness) | the 3D rings' slab; default `5e-6 U(0.25, 1)` of the mean radius, 0.0125-25 km (FUN_140508600) | `0.15`, `10` |
| RocksMaxSize | 5 | number | km | the 3D rings' cell size (one rock at most per cell, `RingRockScale` x this); default 0.04 `Thickness` | `0.008` (8 m) |
| RocksSpacing | 2 | number | multiplier | the rocks' spread in their cells, `RocksSpacing / alpha^2` (no rocks above `RingRockMaxSpacing`); default 1 | `1000` |
| DustDrawDist | 1 | number | none | not read by SE 0.991 (its parser FUN_14046cef0 knows `VolumDrawDist`) | `1854.6` |
| VolumDrawDist | 0 | number | km | the 3D rings are drawn within this distance of the ring volume, the dust fading out at 0.95 of it; default the least of 20000 km, the inner radius, 3000 `Thickness` and twice the width | |
| Density | 11 | number | dimensionless optical depth scale; 1e-4 to 10 | ring density multiplier | `1.0`, `1e-4` |
| Opacity | 9 | number | 0 to 1 (1e-4 to 1 seen) | overall opacity multiplier | `1.0`, `0.003` |
| SelfShadow | 5 | number | 0 to 1 | strength of ring-on-ring shadowing | `1.0` |
| PlanetShadow | 5 | number | 0 to 1 | strength of planet shadow on rings | `1.0` |
| Brightness | 5 | number | multiplier (0.15 to 4) | overall brightness | `1.0` |
| FrontBright | 5 | number | multiplier (1 to 5) | brightness of the front (sun-facing, lit from the front) side | `2.0` |
| BackBright | 5 | number | multiplier (0.15 to 5) | brightness of the back (forward-scatter) side | `5.0` |
| Hapke | 2 | number | 1 | Hapke photometric model switch/scale for ring scattering (inferred) | `1.0` |
| SpotBright | 2 | number | 0.45 to 1 | opposition "spot" amplitude, shadow-hiding (SH) term; Saturn comment cites French et al. 2007 | `0.45` |
| SpotWidth | 2 | number | radians (inferred; 0.015 = 0.86 deg) | angular width of the opposition spot, SH term | `0.015` |
| SpotBrightCB | 2 | number | multiplier | opposition spot amplitude, coherent-backscatter (CB) term (comment: "x5 to match photos") | `1.95` |
| SpotWidthCB | 2 | number | radians (inferred) | angular width of CB spot; Saturn comment says it includes the Sun's angular width | `0.00245` |
| RotationPeriod | 1 | number | hours (FUN_14046cef0 divides by 24) | the ring frame's period; default the Kepler period halfway between `InnerRadius` and `EdgeRadius` | `6.2864` |
| frequency | 4 | number | noise frequency (1/km or 1/planet radius, unknown) | procedural ring noise frequency | `6000`, `157.93` |
| densityScale | 4 | number | multiplier | procedural density noise scale | `6` |
| densityOffset | 4 | number | additive | procedural density noise offset | `-1.4504` |
| densityPower | 4 | number | exponent | procedural density curve exponent | `0.5`, `2.5385` |
| colorContrast | 4 | number | 0 to 1 | procedural colour contrast | `0.5` |
| FrontColor | 1 | RGB tuple | 0 to 1 per channel | colour of the front side | `(0.700 0.700 0.700)` |
| BackThickColor | 1 | RGB tuple | 0 to 1 | colour of thick back-side rings | `(0.800 0.600 0.400)` |
| BackIceColor | 1 | RGB tuple | 0 to 1 | colour of icy back-side particles | `(0.300 0.700 1.000)` |
| BackDustColor | 1 | RGB tuple | 0 to 1 | colour of dusty back-side particles | `(1.000 0.980 0.880)` |

Values use `.` decimals and `e` exponents (`4e6`, `1.1774e+05`). Tuples may have commas or
not. Comments follow values (`// km`), and some keys are commented out (`//EdgeRadius`).

Keys outside the block but related: `RingsWinter` (planet level, 0 to 0.84), `NoRings`,
`NorthRings` / `SouthRings` (int, 2 or 3, planet level).

## Ring textures (Rings.pak)

Rings.pak is a ZIP. Contents (all RGBA PNG, **height 2 rows**, not 1):

| File | Size (px) | Mode | Bytes |
|---|---|---|---|
| Saturn-rings.png | 16384 x 2 | RGBA | 14258 |
| Uranus-rings.png | 8868 x 2 | RGBA | 3733 |
| Jupiter-rings.png | 1024 x 2 | RGBA | 2392 |
| J1407b-rings.png | 512 x 2 | RGBA | 1136 |
| Chariklo-rings.png | 256 x 2 | RGBA | 394 |
| Neptune-rings.png | 256 x 2 | RGBA | 495 |

Layout, as observed:
- Width x is the radial coordinate. Row 0 and row 1 are two colour variants of the same profile.
- Alpha is identical in both rows (checked on Saturn). Alpha 0 means no ring material.
- Saturn: row 0 is a darker, warm tone (about 90,81,62 at x = w/8); row 1 is brighter
  (about 255,222,187 at the same x). Chariklo row 1 is blue (151,200,255), so row 1 is likely
  the icy colour and row 0 the dust/rock colour. Which row the engine picks by lighting side
  is not verified.
- Saturn alpha profile: zero for x/w < 0.016 and x/w > 0.306, nonzero in between. Not
  confirmed, but this fits a linear mapping of x from InnerRadius (0) to OuterRadius (1):
  the outer end then lands near 142000 km, close to `EdgeRadius 137000`. The Cassini Division
  (about 117500 to 122100 km, x/w about 0.21 to 0.23) is not zero-alpha in the texture, so
  the mapping or the texture's meaning is still open.

## How catalogs reference textures

`Texture "Saturn-rings.*"` names a file stem. The `.*` means "any extension" and the engine
looks for the file under `textures/planets/`. Only `.png` exists in this pak, so the Rust port
resolves `.*` to `.png`. Textures are absent for most objects, which then use procedural
rings (`frequency`, `density*`, `colorContrast`).

VFS path for a catalog texture: `textures/planets/<stem>.png` (relative to the data dir, as
used by `crate::vfs`).

## Volumetric rings (3D)

With `VolumetricRings` (main-def.cfg `PlanetRendering`, true) SE adds a second pass (FUN_1404e2bd0,
`rings_raymarch.glsl`) when the camera is within `VolumDrawDist` of the ring volume (the annulus
`InnerRadius`-`OuterRadius`, `Thickness` above and below the plane). The flat ring then draws only
fragments at least 0.95 `VolumDrawDist` from the eye and fades out within two `Thickness` of the
plane (rings.glsl `Fade3DParams`, `RingsParams.z`).

- **Subrings** (FUN_1402e9c50): `max(256, texture width)` equal subrings, at least five
  `RocksMaxSize` wide, a multiple of 256 (SE's float `floor`), at most the GPU's texture size. Subring
  `i` reads texel `i` of the texture's last row (front) and first row (back), clamped to the width,
  and stores in its `MarchMap` (4 rows x subrings, RGBA32F):
  - row 2: inner and outer radius (+ `SubringInner/OuterRadiusAdjust`), `sqrt(front alpha) x Thickness`
    and `back alpha x Thickness`;
  - row 3: `sqrt(front alpha)` (the share of the slab with rocks), back alpha, front alpha, and
    `1 / max(front alpha, 1e-5)^2` (times `RocksSpacing` in the shader: the rocks' spread);
  - rows 0 and 1, each frame the time moves (FUN_1402e9830): the cosine and sine (split in two floats)
    of `frac(t x rate) 2 pi -+ SubringLayerRotationAdjust` (55 degrees), `rate = 1 / RotationPeriod -
    1 / P(r)` per day, `P` the Kepler period at the subring's middle (planet mass in Earth masses x
    3.0034894e-6 solar masses, years x 365.24219); odd layers of cells use row 1.
- **Uniforms** (block `Rings3DUniforms` at 0x1419bfb10, 0x710 bytes): the scale is
  `RingRockScale x RocksMaxSize` km per cell; `FadeParams` = `0.5 / tan(px / f)` cells for
  `RingRocksFadeStartSizePix` (3) and `...EndSizePix` (1.5), `f` the focal length in pixels
  (`height / (2 tan(fov / 2))`, DAT_1419d5ab8), and 0.95 and 1 `VolumDrawDist`; `LightingParams` =
  `Brightness x hdrRingDustBright x FrontBright / BackBright` for the dust and
  `Brightness x hdrRingRocksBright` (5) for the rocks; `DensityParams.w` = `Opacity x SelfShadow`;
  `DetailParams` = `OuterRadius x RingDetailNoiseScale`, `0.05 x scale / f`.
- **Rocks** use `DefaultRingsRockTexture` (`SnowSE001`, `textures/terrain/materials/*_COL/_NORM/_RHAO.dds`)
  with triplanar PBR (GGX), positions as emulated doubles. The port's version is `rings3d.rs` /
  `rings3d.wesl`.
