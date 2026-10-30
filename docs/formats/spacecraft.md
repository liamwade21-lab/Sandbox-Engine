# Spacecraft: ship designs, module configs, materials and flight constants

Parsed by `crates/se_core/src/ship.rs` (designs, module configs, materials) and
`crates/se_core/src/dds.rs` (DXT textures); rendered and flown by `crates/se_engine/src/ship.rs`
and `ship_assets.rs`. Meshes are the `.sm` files of `docs/formats/sm_model.md`.

## 1. Where the data lives

| Data | VFS path | Pak |
|---|---|---|
| Ship designs `.sss` (29) | `models/spacecraft/default/*.sss`, `models/spacecraft/real/ApolloEagle.sss` | `data/models/spacecraft/ShipsModels.pak` |
| Module configs `.cfg` | `models/spacecraft/modules/*.cfg`, `models/spacecraft/default/*.cfg` | same |
| Meshes `.sm` | `models/spacecraft/modules/*.sm`, `models/spacecraft/default/*.sm` | same |
| Materials `.sml` (7) and textures | `textures/spacecraft/default|modules|real/` | `data/textures/spacecraft/ShipsTex.pak` |

All 29 designs resolve except `Test.sss`, which names `modules-ams/double_engine.cfg` (not shipped).
A `Module` names its config relative to `models/` (`Modules/thruster.cfg`, case-insensitive).
A `.cfg` without a `Mesh` key (`thruster.cfg`, `antenna.cfg`, ...) uses the `.sm` next to it.

## 2. Ship design (`.sss`, `.sc` syntax)

```
Name "Fire Dragonfly"   Class "Starship"   Pack "SpaceEngine"   Faction "Aliens"
Length 775.47   Offset (0 0 -15)   Quat (1 0 0 0)   Mass 6.5e8   Albedo 0.1   Exposure 1.5   Color (1 1 1)
RadTemp 1650
MainEngines 300   RetroEngines 0   HoverEngines 0   CorrEngines 20   TurnEngines 0.005
WarpBoostLog 15   Hyperdrive true   (or Warpdrive true)   Aerodynamics false   (AeroModel "shuttle")
Camera { "Behind" (0 70 283) (1 0 0 0) }          type, position (m), orientation (w x y z)
Module "ship" { "default/DragonFly.cfg" (m0 .. m15) }
```

* `Camera` types: `Auto` (0 0 0), `Behind` (chase), `Cockpit`. Positions are metres in ship space.
* `Module` matrix: 16 numbers, a column-major column-vector matrix: elements 12..14 are the translation
  (so `(-1 0 0 0  0 1 0 0  0 0 -1 0  x y z 1)` is 180 degrees about Y plus a shift). It maps module space
  (mesh units times the cfg `Unit`, in metres) to ship space.
* Ship space: right-handed, **forward is -Z**, up +Y (the same as Bevy). The `Behind` cameras sit at +Z.
  `Offset` is added to every module position and engine position; camera presets are already in that frame.
  `Quat` (identity in all shipped designs) rotates the model. Both are approximations of what the executable
  does with them (the executable stores `Offset` in km).
* 25 designs have 1 to 3886 modules (`Asterisk`: 3886 instances of 15 module configs); 7 are single-mesh
  designs (`Module` -> `default/X.cfg`).

### Header values decoded from `SpaceEngine.exe`

`seSpacecraft::ReadSchemeScript` is `FUN_1405308b0`; defaults come from `FUN_140524610`.

| Key | Unit and meaning | Evidence |
|---|---|---|
| `MainEngines`, `RetroEngines`, `HoverEngines`, `CorrEngines` | **acceleration in m/s^2**, independent of `Mass` (not multiples of g) | each is multiplied by 3.2407793e-17 (metres to parsecs) at load; mass only feeds fuel/inertia fields |
| `TurnEngines` | angular acceleration in rad/s^2 per axis, not scaled by mass or length | autopilot rotate planner: time = sqrt(angle * k / Turn) |
| `WarpBoostLog` | maximum of a log10 multiplier; warp velocity = velocity * 10^boost | the current boost (`WarpBoost`, saved in the ship state) is clamped to `[0, WarpBoostLog]` |
| `Warpdrive`, `Hyperdrive` | either one enables the warp drive (FUN_1405308b0 reads both into flag 0x80000) | |
| `Mass` | kg | |
| `Length`, `Offset` | metres in the file, stored as km | |

Defaults: Main 50, Retro 50, Turn 0.002, WarpBoostLog 12, Mass 18000 m/s^2, rad/s^2, log10, kg.

### Flight dynamics in the executable

* **Engines.** Three commanded levels (main, retro, hover), each clamped to its engine value. The current
  level moves toward the command by at most `ramp * engineAccel * dt` per step with `ramp = 2.0` by default
  (`FUN_140523240`), about 0.5 s to spool up. Velocity update: `v += (R * localAccel + externalAccel) * dt`.
  Outside warp the speed is capped at c (9.7156e-9 pc/s). No passive auto-brake.
* **Rotation.** The stop-rotation assist applies angular acceleration `Turn` against the rotation and zeroes
  it on crossing zero (`FUN_140539700`); it is an action (`ShipStopRotation`), not unconditional damping.
  The angular state is clamped to +-0.03 per axis in an unidentified unit.
* **Warp.** `WarpBoost` ramps linearly in log10 at 4.0 per second toward its target (so 12 takes 3 s). Each
  frame (`FUN_14051f7b0`) with altitude `alt = refDistance - refRadius` and ship speed `|v|`: if
  `alt / (|v| * 10^WarpBoostLog) < 2 dt` the target becomes `log10(alt / (|v| * 2 dt))` instead of the maximum,
  so a warp step never covers more than about half the remaining altitude. Below `0.01 * radius` of altitude
  the target is 0. This is how the maximum warp speed scales with distance from massive bodies.
* **Warp field efficiency** (`FUN_140530720`): `2.001 |cos a| - 1` clamped to 0..1, with `a` the angle between the
  ship's velocity (relative to its frame) and its long axis, 0 below 1 m/s. The ship step (`FUN_140537060`) moves
  the ship by that velocity times `10^(WarpBoost * efficiency)`: sideways the warp fails. The Warp HUD shows it
  ("Efficiency", red on the GUI's blink phase below 90 %) and, with the drive engaged under 90 %, "Field is
  unstable, turn to the target!" (`FUN_14040a590`). The info panel's "Warp drive" line reads "None", "Normal
  space" or "In warp (Boost factor = ...)" (`FUN_14052d9c0`).
* **Hyperdrive.** SE 0.991 has no separate hyperspace: `Hyperdrive` designs are warp drive designs, the ship
  control panel's "Stop hyperdrive" and "Hyperjump" buttons stop the warp drive and switch to the Warp HUD for a
  target (`FUN_140406630`, `FUN_1404095a0`), and the key table names Alt+Z "Stop hyperdrive". "Hyperspace",
  "Hyperdrive" (as a label), "Hyperjump here" and "Stop warp drive" are in the executable's string table
  (`FUN_14038a450`) but nothing reads them.
* **Ground.** With the `Collisions` cheat flag on, contact starts below `0.5 * Length` of altitude
  (`FUN_14051b720`) and an impact of 25 m/s or more crashes the ship (`FUN_140530810`). Gravity acts from
  the reference body (`RefBodyID`); `Gravity` and `Collisions` are cheat-menu flags.
* **Keys** (`config/def_xbox360_controller.cfg` action names): `ShipMainEnginePlus/Minus`,
  `ShipRetroEnginePlus/Minus`, `ShipHoverEnginePlus/Minus`, `ShipWarpDrivePlus/Minus`, `ShipMoveForward/Back/
  Left/Right/Up/Down`, `ShipTurnLeft/Right/Up/Down/CW/CCW`, `ShipStopRotation`, `ShipStopEngines`,
  `ShipStopWarpDrive`, `ShipHoldAlt`, `ShipFlightAssist`, `ShipSyncVelocity`, and the autopilots `ShipFlyTo`,
  `ShipWarpTo`, `ShipGoTo`, `ShipRotateTo`, `ShipPrograde`, `ShipRetrograde`, `ShipRadial`, ... The keyboard
  defaults are not in the executable strings; the port's bindings are listed in the README.

## 3. Module config (`.cfg`)

```
Unit 1   Length 182.87   Mass 2e6   Type "Engine"   ThrustMain 6e6
Mesh "spacecraft/Modules/linear_nf_engine.sm"      (relative to models/)
Material "SHW_Default"                              (one per mesh object, by object material index)
EngineEffect "Main top" { Group "Main"  Type "Fusion"  Pos (0 53.7 37.6)  Rot (0 0 180)  Size (7 7 300)
                          Color (0.05 0.25 0.6)  Sound "..."  Volume 1  BrightnessHDR 1 }
DockingPort ...   (CorrPos CorrSize CorrRot ShipPos ShipRot, `Type "Passenger"`/...)
```

* `Type`: `Engine`, `Frame`, `Hab`, `Sensor`, `Tank`, `Radiator`, `Shield`, `Cargo`, `Dock`, `Weapon`.
* `EngineEffect.Group`: `Main` (20), `Retro` (3), `Hover` (6), `Thruster` (127, the RCS), `Hyper` (13).
  `Type`: `Fusion`, `Chemical`, `Ion`, `Ion2`, `Aerospike`, `Hyper`.
* **Placement.** `Pos` is in metres in module space (not scaled by `Unit`) and goes through the module matrix.
  `Rot` is Euler angles in degrees applied about X, then Y, then Z (`R = Rz * Ry * Rx`); the exhaust leaves along
  the rotated **-Z**. Checked on the shuttle's aerospike (`Rot (-90 0 90)` points along +X in module space,
  which the module matrix maps to +Z, backwards) and on the AmS-B and Dragonfly main engines. `Size` is the
  plume's width, height and length in metres.
* `ThrustMain` etc. appear in only two engine modules and are not used by the port.

## 4. Materials (`.sml`) and textures

`Material "name" { DiffColor, EmisColor, EmisBright, SpecColor, SpecPower, DiffMap, EmisMap, SpecMap, NormMap,
BumpMap, TempMap, DetBumpMap, DiffMapAlpha, ... }`. 19 materials, every key of the parser FUN_1402a1fb0 read
(`crates/se_core/src/ship.rs`) and drawn by `ship.wesl` as ship.glsl. `Reflections true` (only `Skylone`) sets
material flag 0x10: the ship gets an environment map (`crates/se_engine/src/ship/envmap.rs`) whose reflection adds
to the specular light (ENVMAP). SHW.sml's `EnvMap true` is not a key SE reads.

`MaterialPBR "name" { ... }` blocks (only `ApolloEaglePBR`, which no design uses; its `ApolloEagle_RMAO.dds` is not
in the install) take the same keys plus `PbrMap` (roughness, metalness, occlusion), `RoughnessScale` 1,
`RoughnessConst` 0, `MetallnessScale` 1, `MetallnessConst` 0 and `AmbOcclScale` 1; flags 0x30, so they always
reflect, and a `"Spec"` alpha becomes `"None"`. They are drawn as ship_pbr.glsl (`fragment_pbr`): its analytic
lights, image-based light from the environment map's prefiltered cubes and SE's `ibl_brdf_lut.png` (loaded only
with `EnableShipPBR`), emission, no thermal glow. ship_pbr.glsl's analytic lights use the camera-to-fragment
vector where the direction to the camera belongs, so n.v and v.h stay at their floors: Fresnel 1, no diffuse,
a weak highlight, each light's terms added once more unlit. The port keeps all of it. A missing `PbrMap` reads
SE's default (roughness 0, metalness 0, occlusion 0.5).

* Texture names are relative to the `.sml` directory, its parent, or `textures/spacecraft/` (case-insensitive).
  `name.*` means any extension; the port prefers PNG, then DDS.
* DDS files are DXT1 or DXT5 (and one `ATI2` normal map) with a full mip chain, 512 to 2048 pixels.
  The port decodes DXT1/3/5 and uncompressed 24/32-bit images (`se_core::dds`), top mip only, then builds
  its own mip chain.
* UVs are used as stored (no flip) for both PNG and DDS: the shuttle's white upper side and black tiled belly
  come out right.
* Many ships are dark by design (Dragonfly diffuse averages (45, 37, 107), its wings (11, 22, 44)).

## 5. Port behaviour and approximations

* **Rendering.** Module instances sharing texture, tint, emissive map and alpha mode are merged into one mesh
  (the 3886-module `Asterisk` is 4 meshes, 243k vertices). Lighting as ship.glsl: up to 4 stars with eclipse
  shadows, Lambert / Hapke, normal, specular, emissive, detail and temperature maps; no shadows cast.
* **Environment map.** SE renders one for each reflecting ship over 1.5 px (FUN_140534b40) from a pool of
  `MaxEnvmaps` chains at three resolutions. The port keeps one, for the reflecting ship largest on screen: six
  cameras at its centre draw the scene without ships (one face per frame), and `ship_env.wesl` copies them into
  an `EnvmapDimensions` cube with its mip chain, then builds EnvMapSpec (half size, level i of n prefiltered at
  roughness ((i + 1) / n), 32 GGX samples, pbr_spec_cubemap.glsl) and EnvMapDiff (`IrrmapDimensions`, 96 cosine
  samples of the source's last level, which is where SE's pass reads, pbr_diff_cubemap.glsl). EnvMapParams is
  the specular cube's size and last level, (512, 9) without a map.
* **Plumes.** One additive teardrop mesh per `EngineEffect`, sized by `Size`, scaled by the level of its group:
  `Main` by the spooled throttle, `Retro` by negative throttle, `Hover` by upward RCS, `Hyper` by warp boost,
  `Thruster` by the RCS or turn commands it can serve. No sound.
* **Physics** (universe frame, f64, `UPos` position): engines as above; angular rate clamped to 0.1 rad/s per axis
  (the executable's clamp is unit-uncertain); gravity summed over every body of the active system with
  `GM = G * mass` from the catalog (3000 kg/m^3 when a body has no mass); ship time follows simulated time
  up to 10x. The ship starts at rest in the frame of the nearest body (its orbital velocity, plus its
  rotation within one radius, nothing beyond 100 radii). Reference speed on the HUD is relative to that frame.
* **Warp.** `10^boost` multiplies the displacement, boost limited as in the executable, plus a hard cap of half
  the remaining altitude per step. Ramp 4 log10/s. Available only to ships with `Warpdrive` or `Hyperdrive`.
* **Ground.** The ship's pivot is held `clearance` (the hull's lowest point below the pivot, at most half the
  length) above `ground_height` of the terrain (or the bare sphere). Touchdown below 0.5 m/s relative to the
  surface pins the ship to the rotating body ("landed"); lift-off when the thrust upward exceeds 0.5 m/s^2 net of
  gravity. An impact of 25 m/s or more shows a crash notice (the ship is not destroyed).
