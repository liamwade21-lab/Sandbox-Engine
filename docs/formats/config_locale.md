# SpaceEngine config, locale, save, script and asset-preset formats

Source: `D:/SteamLibrary/steamapps/common/SpaceEngine/` (read-only) plus extracted archives in the scratchpad `x/` folder.
Scope: what was observed in the files. Items marked **(inferred)** are not confirmed against engine code.

## 1. General .cfg syntax

Used by `config/*.cfg`, `system/*.cfg`, `save/*.cfg`, `data/locale/*.cfg`, and the asset presets (`.pst`, flare `.cfg`, `.se`, `.ssp`).

- **Key/value lines:** `Key value` separated by spaces or tabs. No `=` sign was found in `main-def.cfg` (0 hits). Keys are column-aligned by hand.
- **Value types:**
  - Integers and floats, including exponent form (`1e-07`, `3.240776488385780e-18`, `+258...` hex-like strings in save files, see section 6).
  - Booleans as `true` / `false`. Display/Sound options also use `0`/`1`.
  - Quoted strings, `"..."` (may be empty, `""`).
  - Tuples in parentheses. `main-def.cfg` uses space-separated (`( 1.0 0.9 0.7 0.0 )`), while `.pst` and flare files use comma-separated (`(1.0, 1.0, 1.0)`). A parser must accept both.
  - Lists in `OculusAudioDevices` and similar: a single quoted string with `/` as separator.
- **Blocks:** `Name` on its own line, then `{` on the next line, then `}`. Block bodies may nest. The `.se`/`.ssp` scripts use `Name "arg" { ... }` on one line. Flare `.cfg` files use inline blocks: `Flare { ... }`. `pml` uses `Layer "Detail" { 16 1 "RockSE001_COL" }`.
- **Comments:** `//` to end of line, and full-line banners made of slashes. No `/* */` block comments were found in the config files.
- **Line endings:** CRLF in `save/places-def.cfg` and the `.pst` files. Tabs are used as separators in the locale and places files.
- **Differences from the `.sc` catalog syntax** (from one sample, `catalogs_Catalogs/clusters/Extragalactic.sc`):
  - `.sc` records are typed and named: `Cluster "G1/Meyoll II" { ... }`. Most `.cfg` blocks are singletons with no name (`Display { }`). Named blocks in `.cfg` appear only in save files, `.pst` models, and the locale database.
  - `.sc` uses tab-indented `Key value` lines with no `=`, same as `.cfg`. Both families use `//` comments.
  - `.cfg` files also use `Key value // comment` on the same line, with the comment aligned to the value. Parsers should strip the `//` remainder.

## 2. main-def.cfg

File: `config/main-def.cfg`, 1068 lines, 40 top-level blocks. `config/main-update.cfg` (441 bytes) only contains a `GalaxyGenerator` block with `GalaxyMaxAppMagn 12.1` and the elliptical-galaxy settings. It looks like an override layer for the same keys (inferred: merged over main-def at startup).

Notation: `key = default` as written in main-def.

### Top-level block index

| Block | Purpose |
|---|---|
| Memory | VRAM/RAM caps, cache sizes for solar systems, terrain nodes, star/galaxy nodes, models |
| Vendor | GL vendor/renderer strings, core context, shader version, loader flags |
| Display | Window size/position, fullscreen mode, refresh, aspect |
| RenderingQuality | Quality preset id, VSync, anisotropy, MSAA, FXAA, dithering, texture compression, line width |
| DepthBuffer | Z-buffer mode, ship draw mode, clip height and near plane |
| PlanetRendering | Terrain LOD, planet mesh/texture detail, shine, emission, aurora, comet tails |
| AtmosphereRendering | Atmosphere precision, extinction, horizon fixes, distance limit |
| RingsRendering | Ring mesh LOD, rock fade, subring adjustments |
| PointRendering | Point/sprite method, magnitude limits, brightness cutoffs, galaxy auto-magnitude |
| WarpRendering | Black hole, wormhole and ship warp quality and radii |
| PostEffects | Exposure, tone mapping, bloom, lens flares, auto exposure |
| Lighting | Magnitude fade ranges for planets/rings/flares; HDR and "real" brightness calibration |
| TextureRendering | Render-to-texture FBO toggles, resolutions, upscale modes, sharpness |
| Projection | Projection mode, stereo and fisheye options, cubemap |
| VR | HMD runtime, render scale, controller setup, proximity sensor |
| Sound | Volumes, music repeat/order, crossfade durations |
| Physics | Physics tick, cheat flags (God, UFO, FTL, TARDIS) |
| Export | Screenshot/video formats, quality, bitrate, FPS |
| Loader | Loader mode, threads, per-frame loading budgets, preload |
| Log | Log stamps, catalog log level, CSV log level |
| Interface | Language, skin, fonts, GUI scale, HUD, fades, units of display, panel locks |
| Units | Distance/temperature/pressure display units |
| Toolbars, SolarSystemBrowser, SolarSystemChart, UniverseMap | UI layout and visibility |
| ObjectFilters, OverlayFilters, DebugOverlay | Map/filter visibility toggles, grids, vectors |
| Controls | FOV, zoom, key-motion acceleration, goto/center times, home address |
| VelocityPresets | Speed presets `Velocity00`..`Velocity25`, in parsecs per second (see below) |
| Ephemerides | DEA and JPL ephemeris file names |
| SolarSystemGenerator, LifeGenerator, PlanetGenerator, StarGenerator, ClusterGenerator, NebulaGenerator, GalaxyGenerator | Procedural generation probabilities and parameters |
| Engine | Shipped PBR, detail textures, variable slots, power-save FPS, forced stereo |

### Rendering: stars, galaxies, planets, atmospheres, LOD, FOV, exposure, magnitude

| Block.key | Default | Meaning |
|---|---|---|
| PlanetRendering.LandLOD | 0 | Terrain LOD bias, range -2..+2 |
| PlanetRendering.LandTargetRes | 1080 | Screen height that drives terrain LOD |
| PlanetRendering.LandTargetResVR | 1440 | Same, in VR |
| PlanetRendering.PlanetMaxLevel | 24 | Terrain subdivision cap |
| PlanetRendering.PlanetTextureRes | 256 | Terrain texture resolution (power of two) |
| PlanetRendering.PlanetSphereMeshLOD / PlanetCubeMeshLOD | 5 / 5 | Mesh detail |
| PlanetRendering.PlanetShineMode | 1 | Secondary light mode: 0 low, higher is better |
| PlanetRendering.PlanetShineUpdateFrames | 10 | Refresh period of planet-shine colour |
| PlanetRendering.AuroraBright / CometTailBright / SelfEmissionBright | 1 / 1 / 1 | Effect brightness |
| PlanetRendering.SunIllumSaturation | 0.5 | Sunlight saturation |
| AtmosphereRendering.AtmoDistanceLimit | 30 | Max atmosphere render distance |
| AtmosphereRendering.AtmoMinWidthPix | 2 | Minimum atmosphere thickness on screen |
| AtmosphereRendering.AtmoBottomOffset | 1e-7 | Horizon artefact fix |
| PointRendering.StarPointMethod | 2 | 0 points, 1 sprites, 2 sprites with motion blur, 3 with rotation blur |
| PointRendering.PlanetPointMethod | 1 | Same scale for planets |
| PointRendering.PointScale | 1 | Point source scale |
| PointRendering.LinkScaleToScreenRes | true | Scale with screen resolution |
| PointRendering.StarMagnLimit | 7 | Star limiting apparent magnitude |
| PointRendering.PlanetMagnLimit | 7 | Planet limiting magnitude |
| PointRendering.ClusterMagnLimit / NebulaMagnLimit / GalaxyMagnLimit / GlowMagnLimit | 7 / 7 / 7 / 7 | Limits for other object classes |
| PointRendering.StarBrightMin | 0.03 | Star cut-off pixel brightness |
| PointRendering.GalaxyBrightMin | 0.03 | Galaxy cut-off |
| PointRendering.GlowBrightMin | 0.006 | Galaxy/nebula sprite cut-off |
| PointRendering.AutoGalaxyMagnitude / AutoGalaxyMagnSpeed | true / 10 | Automatic galaxy magnitude when inside |
| PointRendering.StarFrustumClipFOV | 30 | Max FOV for star frustum clipping |
| PointRendering.PointOverbright / PointFlareBright / PointDesaturation | 1 / 1 / 0.1 | Point look |
| GalaxyGenerator.GalaxyMaxAppMagn | 12.1 | Max apparent magnitude of procedural galaxies from Solar system (also in main-update.cfg) |
| GalaxyGenerator.GalaxyEllipticalScale / Color / Bright / Gamma | 2.4 / (1.0 0.9 0.7 0.0) / 1.0 / 0.7 | Elliptical galaxy model look |
| PostEffects.Saturation / Contrast / Brightness / Gamma | 1.5 / 1 / 1 / 1 | Final image grading |
| PostEffects.DSOBrightness / DSOGamma | 1 / 0.7 | Deep-space-object grading |
| PostEffects.ToneMappingShader | "AgX" | Tone mapper |
| PostEffects.PhotoMode | 0 | 0 HDR, 1 manual exposure, 2 auto exposure |
| PostEffects.ExposureComp | 0 | Exposure compensation |
| PostEffects.AutoExpTargetBright | 0.35 | Target median brightness for auto exposure |
| PostEffects.AutoExpMinExpComp / AutoExpMaxExpComp | 0 / 42 | Auto exposure range |
| PostEffects.AutoExpSpeedUp / AutoExpSpeedDown | 5 / 1 | Adaptation speed |
| PostEffects.BloomBright / BloomThreshold | 0.5 / 0.0 | Bloom |
| PostEffects.LensFlarePreset / LensGhostPreset / PseudoFlarePreset | "Astroniki2" / "Fireinthehole" / "Default" | Names of flare config files (section 8) |
| Lighting.PlanetLightingFadeBeginMagn / EndMagn | -7 / -5 | Sun magnitude over which planet lighting fades |
| Lighting.RingsLightingFadeBeginMagn / EndMagn | -12 / -10 | Rings lighting fade |
| Lighting.LensFlareFadeBeginMagn / EndMagn | -30 / -14 | Sun magnitude over which flares fade |
| Lighting.hdr* / real* | hdr group and real group | Brightness calibration for HDR photo mode and for auto/manual mode (e.g. `realGalaxySpriteBright 0.01`, `realStarSurfaceBright 1e+09`) |
| Projection.ProjectionMode | 0 | 0 perspective, 1 cubic, other fisheye/cylindrical values |
| Projection.FisheyeFOV | 180 | Fisheye lens angle |
| Projection.StereoIPD | 6.4 | Interpupillary distance (mm) |
| Projection.GUIResolution | 1280 | GUI texture resolution in VR/fisheye |
| Controls.FOV | 45 | Startup FOV (degrees) |
| Controls.DefaultFOV | 45 | Default FOV |
| Controls.ZoomFOV | 20 | Middle-click zoom FOV |
| Controls.MinFOV / MaxFOV | 0.00027778 / 120 | Zoom limits |
| Controls.ZoomAccel | 0.3 | Mouse wheel zoom speed |
| Controls.MoveKeyAccel / MoveKeyDecel | 200 / 500 ms | Movement key acceleration/deceleration |
| Controls.TurnKeyAccel / TurnKeyDecel | 200 / 500 ms | Turn key acceleration/deceleration |
| Controls.TurnSmoothing | 0 | Rotation smoothing (0 = hard) |
| Controls.GotoSpeed / GotoSpeedVR | 10 / 5 | Goto acceleration |
| Controls.LongGotoTime / FastGotoTime / CenterTime / ZoomTime | 20 / 3 / 1.5 / 0.25 s | Animation durations |
| Controls.AutoHorizon | true | Auto horizon view on planets |
| Controls.HomeGalaxy / HomeSystem / HomeStar / HomeWorld | "Milky Way" / "Solar System" / "Sun" / "Earth" | Home bookmark (Shift-H cycles it) |
| Physics.PhysicsTick | 0.015 | Physics step (s) |

Time flow and mouse sensitivity: `main-def.cfg` has no mouse sensitivity key. Time-related keys are `RealTime`, `RealTimeFPS` (Export), and `StereobaseToTimeScale`. Keyboard bindings are not in this file (not located in this pass).

### Velocity presets

`VelocityPresets` `Velocity00`..`Velocity25` are values in parsecs per second, with the comment giving the human label. Example: `Velocity00 3.24e-18 // 0.1 m/s`, `Velocity12 4.848e-06 // 1 AU/s`, `Velocity17 1.0 // 1 pc/s`, `Velocity25 1e+08 // 100 Mpc/s`. This unit is important for save files (section 6).

### Other notable blocks

- **Memory:** `VideoMemorySize 3072` (MB), `RAMSizeMax 8192`, `CacheSizeLandNode 10000`, `CacheSizeStarNode 50000`, `CacheSizeGalNode 10000`.
- **Loader:** `LoaderMode 2`, `MaxThreads 0` (auto), `MaxTilesPerFrame 4`, `MaxTimePerFrame 10` ms.
- **Interface:** `Language "eng"`, `Skin "Flat Blue"`, `PrimaryFont "NotoSans-Regular.ttf"`, `PrimaryFontBold "NotoSans-Medium.ttf"`, GUI scale keys, `FullConstellationNames`, `ShowDwarfPlanets`.
- **Export:** `ScreenshotFormat "jpg"`, `VideoEncoder "libaom-av1"`, `VideoFrameRate 60`.
- **Engine:** `EnableShipPBR true`, `PowerSaveMaxFPS 4`, `ForceStereoscopicMode 0`.
- **Generators:** probabilities such as `ProbBinaryStar 0.5`, `ProbPlanemo 0.45`, `ProbLifeOrgTerra 0.3`, `LifeOrgAgeUni 0.6`, `PlanetCloudsTerra 3`.

## 3. Preset differences (`config/preset-*.cfg`)

Five files, each about 6.5 KB with about 60 key/value lines. They are a subset of main-def keys, and `QualityPreset` in main-def selects one (0 potato, 1 low, 2 medium, 3 high, 4 ultra).

Only keys whose values differ across presets are shown. Columns are in the order potato, low, medium, high, ultra. Values that match across all five are not shown.

| Key | potato | low | medium | high | ultra |
|---|---|---|---|---|---|
| VideoMemorySize | 2048 | 2048 | 3072 | 4096 | 8192 |
| AnisotropyLevel | 0 | 8 | 16 | 16 | 16 |
| FXAA | false | true | true | true | true |
| TextureCompress | true | true | true | false | false |
| LandLOD | -2 | -1 | -0.5 | 0 | 0 |
| LandTargetRes | 480 | 720 | 1080 | 1080 | 1080 |
| LandTargetResVR | 480 | 720 | 1080 | 1080 | 1440 |
| PlanetDetailTextures | false | false | true | true | true |
| PlanetShineMode | 0 | 0 | 1 | 2 | 2 |
| MaxTilesPerFrame | 1 | 1 | 2 | 4 | 10 |
| MaxTilesPerFrameVR | 1 | 1 | 1 | 2 | 2 |
| SolidResolution | 0.5 | 0.6 | 0.7 | 0.85 | 1.0 |
| SolidTargetRes | 1080 | 1080 | 1080 | 1080 | 1440 |
| SolidUpscaleMode | 3 | 3 | 9 | 8 | 8 |
| SolidFXAA | false | false | false | true | true |
| VolumResolution | 0.25 | 0.25 | 0.35 | 0.5 | 1.0 |
| ImpostorResolution | 0.25 | 0.35 | 0.35 | 0.35 | 0.333 |
| SkyboxResolution | 0.35 | 0.5 | 0.75 | 1.0 | 1.0 |
| WarpPrecision | 0.2 | 0.3 | 0.5 | 0.8 | 1.0 |
| BlackHoleQuality | 1 | 1 | 2 | 3 | 4 |
| ShipWarpQuality | 1 | 1 | 2 | 3 | 4 |
| ShipEngineQuality | 0 | 0 | 1 | 2 | 3 |
| AuroraQuality | 0 | 0 | 0 | 1 | 1 |
| StarPointMethod | 1 | 1 | 2 | 2 | 2 |
| MSAALevel | 0 | 0 | 0 | 0 | 4 |
| MSAALevelVR | 0 | 0 | 0 | 0 | 4 |
| MSAALevelImp | 0 | 0 | 4 | 8 | 8 |
| MaxPlanPickers | 8 | 8 | 12 | 16 | 20 |
| MaxImpostors | 1 | 1 | 2 | 2 | 2 |
| MaxEnvmaps | 1 | 1 | 1 | 2 | 2 |
| EnvmapDimensions | 256 | 512 | 512 | 1024 | 2048 |
| MipmapsFrame / MipmapsEnv / MipmapsGUI | false | false | true | true | true |
| Use3Dmenu | false | false | true | true | true |

Keys that are identical across all five presets were not listed. Every key in the table is present in all five files.

## 4. paths-def.cfg and paths-port.cfg

Two path tables with the same keys. Both are in `system/`. `paths-def` is the default install layout (user data under `%USERPROFILE%` and `%LOCALAPPDATA%`). `paths-port` is a relocatable/portable layout: `UserDir ""`, cache under `cache/`, screenshots and logs next to the install **(inferred: which one is loaded depends on a launcher setting that was not checked)**.

Keys (paths-def values):

- **Config and save roots:** `DefConfigDir "config/"`, `DefSaveDir "save/"`, `DefPreviewsDir "save/"`.
- **User dir:** `UserDir "%USERPROFILE%/Documents/Cosmographic/SpaceEngine/"`. Subdirs are relative to it: `UserConfigSubDir "config/"`, `UserSaveSubDir "save/"`, `UserScreenshotsSubDir "screenshots/"`, `UserScreenshotsHDSubDir`, `UserVideoSubDir "video/"`, `UserLogsSubDir "logs/"`, `UserExportSubDir "export/"`, `UserImportSubDir "import/"`, `UserWorkshopSubDir "workshop/projects/"`.
- **Cache:** `CacheDir "%LOCALAPPDATA%/Cosmographic/SpaceEngine/"`, with `CacheShadersSubDir "shaders/"`, `CacheModelsSubDir`, `CacheGalaxiesSubDir "models/galaxies/"`, `CacheNebulaeSubDir "models/nebulae/"`.
- **Assets:** `DataDir "data/"`, `AddonsDir "addons/"`, `UserAddonsDir "%USERPROFILE%/Documents/Cosmographic/SpaceEngine/addons/"` (portable: `addons/`).
- **Asset subdirectories:** the `#` placeholder is replaced by each asset root (the install root, and each Steam Workshop item path). Keys: `CatalogsDir "#/catalogs/"`, `EphemeridesDir`, `LocaleDir "#/locale/"`, `ModelsDir`, `MusicDir`, `ScriptsDir "#/scripts/"`, `ShadersDir`, `SoundsDir`, `TexturesDir "#/textures/"`, plus finer keys such as `CatalogsGalaxiesSubDir "#/catalogs/galaxies/"`, `CatalogsStarsSubDir`, `CatalogsClustersSubDir`, `CatalogsNebulaeSubDir`, `CatalogsPlanetsSubDir`, `TexturesCommonSubDir "#/textures/common/"`.

Lookup model (inferred from the naming): a file is searched first in the user dir and addons, then in each asset root (`#`), so an addon can shadow a base file. Environment variables in `%...%` are expanded by the engine.

## 5. Locale: data/locale/eng-gui.cfg and eng-db.cfg

Other languages present: about 40 files (`*-gui.cfg`, `*-db.cfg`), including `WGSBN_asteroids-eng-db.cfg` (asteroid names).

### eng-gui.cfg (141 lines, 93 translation pairs)

Three parts:

1. **Header metadata:** `InternationalName "English"`, `LocalName "English"`, `FlagIndex 109`, `LocNameSteam "english"` (matched against the Steam UI language to pick this file).
2. **Settings:** `GenderPlanet`, `GenderAsteroid`, `GenderEarth`, etc. (`"masculine"`, `"feminine"`, `"neuter"`), `PlanetClassOrderTemp 0` through `PlanetClassOrderLife 4` (word order in planet class strings), `ClassNameAdjectiveSeparator ""`, `ClassNameNounSeparator ""`.
3. **Two empty blocks:** `Translations { }` and `ScriptTranslations { }`.

The translation pairs are `"source" "translated"`, one per line. The key is the English source string. A `#context` suffix disambiguates it. Five examples:

```
"temperate#masculine" "temperate"
"spiral#galaxy_type" "spiral"
"Format#video" "Format"
"Black body colouration" "Black body coloration"
"marine#masculine" "marine"
```

(Also present: `"Spacecraft#plural" "spacecraft#plural"`, and keys without a context.) Note the `"Spacecraft#plural" "spacecraft#plural"` entry: its value still carries the marker, so the lookup must strip it or treat the value as a fallback.

### eng-db.cfg (952 lines, 79 objects)

Object database: 79 `RObject { ... }` blocks, tab-indented fields. Every object has `LocName`, `Name` and `Parent`. Some have `Descr` (21 objects, long multi-paragraph text) and `Pioneer` plus `Date` (71 objects, discoverer and discovery date).

Five examples (abridged):

```
RObject
{
	LocName "Sun"
	Name	"Sol"
	Parent	"Solar System"
	Descr	"Sol, or the Sun, is the star at the center of the Solar system. ..."
}
RObject { LocName "Mercury"  Name "Mercury"  Parent "Sol" }
    Pioneer "Asaph Hall"   Date "1877.08.18 00:00:00.00"   (discovery fields in one object)
```

`LocName` is the localized display name. `Name` is the catalog/internal name. `Parent` links to another object's `Name`. The file is a set of records keyed by `Name`, and only the display text is localized (`Descr` and `LocName`).

## 6. save/places-def.cfg and save/save-def.cfg

### places-def.cfg (75 places, CRLF, tab-indented)

One `Place "<title>" { ... }` block per bookmark:

```
Place "Night in the Pleiades"
{
	Ver        991
	Body       "HD 23479 6.3"
	Parent     "HD 23479 6"
	Date       " 0.01.07 10:29:41.21"
	Time       +1A42E7EFF1C1D37C100D936928
	Pos        (+6737348A92C6DFE0E07233 +378EF35869E7D9385589DA +4424EF9DA06E6960193A6E)
	Rot        (0.07722946276188447 0.5676260679295289 -0.6393251258587985 0.512932393726507)
	Vel        1.2901775e-14
	Stereobase 1
	BindMode   2
	PhotoMode  0
	Exposure   0
}
```

Encoding:

- `Body` and `Parent`: the target body and its parent, by name.
- `Date`: `"YYYY.MM.DD HH:MM:SS.cc"` as a string (years may be negative or 0 with a leading space).
- `Time`: a `+`-prefixed hexadecimal integer (around 26 hex digits). The fixed-point scale is **(inferred, not decoded)**.
- `Pos`: three `+`/`-`-prefixed hexadecimal integers, each in a fixed-point, body-relative or absolute form. Digit counts vary (16 to 24), so the scale is per field. Decoding is not verified.
- `Rot`: a unit quaternion of four decimal floats (w,x,y,z order is **not verified**).
- `Vel`: a decimal float in **parsecs per second**. This matches the `VelocityPresets` units in main-def.
- `Stereobase`, `BindMode` (0 Free, 1 Follow, 2 SyncRot), `PhotoMode`, `Exposure`: plain values.

### save-def.cfg (user state)

Two state blocks, `Planetarium` and `Game`, plus top-level `UserName "Player"` and `LastMode 0` (0 planetarium, 1 single player, 2 menu).

Each state has: `MoveMode` (1 spectator, 2 spacecraft, 3 aircraft, 4 piloting), `BindMode`, `RefBody`, `RefParent`, `AbsPosition` and `RelPosition` (hex tuples as above), `Orientation` (quaternion), `Velocity` (pc/s), `TimeScale`, `Time` (hex), `Date`. `Game` also has `Spacecraft <id>` (ID of the last controlled ship).

The save format and the places format use the same position and time encoding. A Rust port should share one decoder.

## 7. Script language (.se) and .ssp

Scripts live in `scripts/` (tutorials, menu, CheckLoc). Command frequency across the 12 `.se` files:

| Group | Commands (count across scripts) |
|---|---|
| Variables | `Set <Name> <value>` (274), `WaitVar` (25), `if` / `endif` (8 each) |
| Waiting and triggers | `WaitTrigger "<name>" { MaxWaitTime <s> }` (249), `WaitMessage` (65), `Wait` (53), `BeginMultiTrigger` / `EndMultiTrigger` (8 / 5) |
| Messages and UI | `ShowMessage` (220), `HideMessage`, `HighlightWidget "<widget>" { Time 1.0 }` (172), `ShowDialog` / `HideDialog` (33 / 3), `HideAllDialogs` (11), `HideAllToolbars` (9), `OpenMenu` (7), `SetWidgetStyle` (14) |
| Camera and location | `GotoLocation { Body Parent Date Time Pos Rot Vel Stereobase BindMode PhotoMode Exposure }` (16), `Goto` (6), `Select` / `Unselect`, `FOV`, `Orbit` / `StopOrbit`, `Waypoint` / `ClearWaypoints` (13 / 5), `SyncRot`, `UserMoveControl`, `UserTimeControl`, `UserRotationControl`, `Interpolate` |
| Time and force | `TimeScale` (13), `SetForce` (8) |
| Flow and environment | `Run "<file.se>"` (8), `SaveVars` / `RestoreVars` (8 each), `CheckVersion 991`, `SetLocale "eng"`, `PrintTranslations "<file>"`, `PlayMusic`, `StarBrowserReset` |

Examples:

```
CheckVersion 991
SaveVars
Run "tutorials/DefaultSettings.se"
Set ShowLabelsStars false
WaitTrigger "LoadingComplete" { MaxWaitTime 30 }
HighlightWidget "Message box" { Time 1.0 }
```

Many `Set` names are the same keys as in main-def (for example `EnablePlanetShine`), so `Set` writes runtime variables that shadow settings.

### .ssp (spline camera path)

`scripts/menu/Scene1.ssp` is a camera path for the menu scene. Structure:

```
SplinePath "Scene1 camera wobble"
{
	Body      "Startup Moon 1.2"
	Parent    "Startup Planet 1"
	SyncRot   true
	Duration  18
	PosSpline "Catmull-Rom"
	RotSpline "B-spline"
	StereoSpline "Catmull-Rom"
	FollowQuat (...)
	SplineData { ... control points ... }
}
```

It is a time-parameterised camera path with per-channel spline types. `SplineData` holds the control points (contents not decoded in detail).

## 8. .pst, flare .cfg, materials and sample presets

### .pst (procedural models)

Plain ASCII text with CRLF, not binary.

- `galaxies_Galaxies-sprite/`: `Default.pst` and `Kexitt.pst`. Each has about 9 `GalaxyModel "<name>" { ... }` blocks in Default.
- `nebulae_Nebulae-sprite/`: five files (`Default`, `Astroniki`, `Bambusdude`, `HarbingerDawn`, `Voekoevaka`). `Default.pst` has about 15 `NebulaModel "<name>" { ... }` blocks.

Galaxy model block keys: `Method`, `LODbase`, `BBoxRes`, `Radius`, `Position`, `floatLOD`, `UseForObject`, and paired particle/detail sets: `emParticleColor`, `emParticleSizeCenter` / `Edge`, `emParticleBrightnessCenter` / `Edge`, `emParticleMinBrightCenter` / `Edge`, `emParticleDispR` / `Z`, `emDetailR` / `Z`, with `abs*` counterparts (absorption dust).

Nebula model block keys: the same particle/detail set, plus `UseForType` (for example `"Diffuse"`), `Method` (values seen: `Diffuse`, `Planetary`, `SNR`, `CubeMap`), `Yaw`/`Pitch`/`Roll`, `clipRadius`, `sumColor`, `emParticleColor2`, noise parameters `emNoiseRandomize`, `emNoiseFreq`, `emNoiseOctaves`, `emNoiseDistort`, `emNoiseOffset`, `emNoiseLacunarity`, `emNoiseH` (and `abs*`), `SpriteOblZ`, `FrontImage "<name>.*"`, and `CubePos[XYZ]Image "<name>.*"` for cube-map methods.

### Flare config (`textures_Flares/*.cfg`, 23 files; 9 PNG textures)

Example, `Anamorphic flares.cfg`:

```
ScaleAll  0.03
BrightAll 1.0
Texture  "Anamorphic.*"
Flare { (1.0, 1.0, 1.0)  0.5  0.3  29.0  29.0  0.333  0.0  0.0  0.0  0.0  ( 0 0 512 256 ) }
```

Field order per `Flare` row (from the file header comment): colour (RGB), brightness centre and edge, scale centre and edge, aspect ratio, then four values for position, angle centre, angle edge and chroma, then the texture rectangle `( x y w h )` in the atlas. Diagonal spikes use a non-zero angle (`-38.0 -24.0`). `Anamorphic ghosts.cfg` has the same structure with `ScaleAll 0.02` and `BrightAll 0.25`. The flare presets are the names referenced in main-def `PostEffects` (section 2).

### Materials presets (`materials_MaterialsPresets/*.pml`, 6 files: aquaria, grass, mars, selena, snow, terra)

Text blocks. One per material:

```
Soil "Blocks icy white 1"
{
	Color (0.965 0.965 0.965 1.000)
	Slope 0.000
	ModColor 1.000   ModRough 1.000   ModNorm 1.000   ModHeight 1.000
	BiasRough 0.250  ScaleRough 1.000
	BiasHeight 0.400 ScaleHeight 0.250
	Layer "Detail" { 16 1 "RockSE001_COL" }
	Layer "Modul"  { 128 1 "GroundMuddy001_COL" }
}
```

A `Layer` is `{ <tiling scale> <weight?> "<texture base name>" }`. The two numeric fields after the name are **(inferred: tiling and strength)**. Material names are referenced from sample presets.

### Sample presets (`samples_SamplesPresets/*.cfg`, 225 files)

One file per planet class combination, named `<bulk>_<atmo>_<variant>.cfg`. Example, `aquaria_airless_default.cfg`:

```
Weight     0.2
BulkClass  "Aquaria"
TempClass  "Any"
AtmoClass  "Airless"
VolqClass  "Airless"
SunClass   "Any"
LifeOrganic "DontMatter"
LifeExotic  "DontMatter"
ColorVary  (0.789 0.747 0.734)
Materials  "Mineral"
{
	"Gravel gray"
	"Blocks icy white 1"
	"Pebbles icy orange 1"
}
```

`Weight` is the selection weight. The class fields match the planet classes in `eng-gui.cfg` (`PlanetClassOrder*`). `Materials` lists material names from the `.pml` files.

## Open items and unverified points

- Hex fixed-point scale for `Time` and `Pos` in save and places files is not decoded.
- Quaternion component order in `Rot` / `Orientation` is not verified.
- Which of `paths-def` and `paths-port` is loaded is not verified.
- Key bindings and mouse sensitivity are not in main-def; their file was not located.
- The `.sc` comparison is based on one catalog file only.

## Places format (decoded)

Decoded for `save/places-def.cfg` and implemented in `crates/se_core/src/places.rs`. This supersedes the "not decoded" notes in section 6.

- **Time**: signed hex fixed point, `value / 2^80` = Julian Date. Checked on "Earth: Civilization": `2457865.4956410` equals 2017-04-21 23:53:43.39 (JD 2457864.5 + 86023.39/86400), to 1e-6 day.
- **Pos**: heliocentric (Sun-centred) camera position in **parsecs * 2^80**, signed hex. Equatorial frame with X = RA 6h, Y = north celestial pole, Z = RA 0h. Evidence:
  - Direction: for 7 solar-system epochs (Earth x3, Mars, Jupiter, Saturn x3, Neptune), `Pos / |Pos|` matches the Sun-to-planet direction from Standish 1800-2050 elements to cos 1.00000 at this frame. Moons (Io, Moon, LM Eagle) match their planet's heliocentric direction.
  - Scale: `|Pos| / r_planet` is 5.83-5.87e18 units per AU across all checks (Earth, Mars, Jupiter, Saturn, Neptune, Pluto, Eris). Saturn and Neptune agree with `2^80 / 206264.806` = 5.86125e18 to 0.1%.
  - Far objects: Andromeda 0.81 Mpc, M82 3.7 Mpc, M74 6.2 Mpc, Whirlpool 7.4 Mpc, Hidden Star Forge about 10 kpc (Milky Way). These match real distances.
  - Reference: the Sun's centre was checked to about 0.5%. A barycentre reference is not excluded.
  - A body-relative reading is ruled out: "The Eagle Has Landed" (LM Eagle, parent Moon) has `|Pos|` near 2^62.37, the same as Earth's, not the metres-scale offset a lander would need.
- **Rot**: file order is **w x y z**. Stored as `[x, y, z, w]`. Under the standard matrix from the quaternion, the camera forward (-Z) matches the Sun-to-object direction for the far objects: M74 0.0 deg, Ring Nebula 0.7 deg, Whirlpool 4.7 deg. Under x y z w order no axis matches. Confidence: medium-high. Looking along -Z is an assumption, though the far-object matches support it.
- **Vel**: parsecs per second, consistent with `VelocityPresets` (Velocity00 = 3.2408e-18 pc/s = 0.1 m/s).
- **Galaxy-entry dates**: Andromeda, M74, M82 and similar entries carry a placeholder `Time` (JD 11684494.376, Date "27278.12.15"). Treat their date as meaningless.
- **Confidence**: unit (2^-80 pc) high; frame high; quaternion order medium-high; Vel unit medium (taken from the doc, not independently checked).
