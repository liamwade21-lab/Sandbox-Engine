# Reverse-engineering notes: SpaceEngine 0.991.49.2095 beta

Target: `system/SpaceEngine.exe`, MSVC x64. It has 11,761 functions and 1.38 M
instructions, no symbols, and 241 RTTI class names (`re/rtti_classes.txt`). Tools used
were Ghidra 12.1.4 through `ghidra-cli` (project `re/ghidra_projects/se`), byte-level
analysis of the data files, and cross-checks against the shipped data.

## Findings index

| Topic | Result | Evidence / doc |
|---|---|---|
| Data packaging | `.pak` = plain ZIP (deflate/stored); pak entries are relative to the pak's folder; `addons/` overrides `data/` | `crates/se_core/src/vfs.rs` |
| Shaders | `data/shaders/Shaders.pak` is fully ZIP-encrypted (141/141 entries); **not decrypted**, the port uses its own WESL shaders | pak headers (flag bit 0) |
| Script syntax (`.sc`/`.cfg`) | `Tag "Name/Alias" { Key value… }`, nested blocks, tuples, sexagesimal RA/Dec, bare numeric rows | `docs/formats/sc_vocabulary.md` |
| Universe frame | right-handed, **X = RA 6h, Y = north celestial pole, Z = RA 0h**; origin = Solar System barycentre | Solved from the Milky Way quaternion (its disk normal hits the NGP to 0.99994); confirmed independently by saved places |
| Galaxy orientation | catalog `Quat (w x y z)` maps universe→model; disk normal = model −Z | as above; `coords.rs` |
| Saved places | `Time` = JD × 2^80 and `Pos` = parsecs × 2^80, both signed hex fixed point; `Rot` (w x y z) | `docs/formats/config_locale.md`, `places.rs` |
| Internal precision | 128-bit fixed-point time and position, implied by the place format | the port uses i128 at 1/1024 m |
| Planet textures | cube-map quadtree `face/L_a_b.ext`, a = row and b = column, 256² tiles without borders; faces follow the OpenGL cube-map convention in the planet frame (+X = lon 0, +Y = north, +Z = 90°W) | `docs/formats/planet_tiles.md`; verified visually on Earth's tiles |
| Earth surface variants | `_c` = colour, `_a` = water mask (`DiffMapAlpha "Water"`) | tile inspection |
| Relief | 16-bit gray PNG; height = v·BumpHeight − BumpOffset (km) | catalog comments plus rendering |
| Atmospheres (`.atm`) | 60-byte header (15 f32 parameters) + Bruneton-2008 tables: inscatter 256×128×32, transmittance 256×64, irradiance 64×16, RGBA f32. **Quirk:** only 64 distinct µ rows; odd rows mirror ν | `docs/atmosphere.md` (verified against numerical integration) |
| 3D models (`.sm`) | `SM01`/`SM02` meshes, 6 vertex formats, packed normals; loader `seMesh::LoadBinary` @ `FUN_1402a5c50` | `docs/formats/sm_model.md` |
| Model scripts (`.sss`, `.cfg`) | plain text placement scripts; textures via `asteroids.sml` | `model_script.rs` |
| Galaxies | `.pst` sprite templates; textures RGB = emission and A = dust; voxel→sprite generator with an LCG `s·1664525+1013904223`, name-seeded | `docs/formats/galaxy_model.md` (decompiled) |
| Nebulae | `NebulaModel` `.pst` (135, CubeMap/Cylindrical/Diffuse/Spherical/Planetary/SNR) built by the same sprite maker as galaxies; `Cylindrical`, `Spherical` and the image-free `Planetary`/`SNR` (`createNebula`) voxel generators (all reproduce the stored `clipRadius`); nebula default-model pick (`FUN_140270120`); far billboard + 3D sprite volume | `docs/formats/nebula.md` (decompiled + validated against stored `clipRadius`/`sumColor`) |
| Ephemerides | `DE436` standard JPL binary (1550–2650 AD, 1018 coefficients per 32-day record); catalog `AnalyticModel "X-DE"` selects it | `jpl_de.rs`; places reproduce |
| Moon theories | `seOrbitL1` (Galilean), `TASS` (8 Saturnian), `GUST86` (5 Uranian), `NOE4` (Mars), `HTC20` (Helene/Telesto/Calypso), `Noy` (Janus/Epimetheus), `NEP` (Triton); tables are static `.data` objects in the exe, evaluation decompiled and checked against the exe run in an emulator (agreement ~1 m); `VSOP87*`/`JPL`/`DEA` are registered but unused by the catalogs | `docs/formats/moon_theories.md`, `moon_theories/` (tables read from the install at runtime) |
| Procedural names | stars `RS <galaxy>-<level>-<node>-<index>`, planets `<star> N`, moons `<planet>.N` | `docs/procgen.md` |
| Procedural galaxies | `seGalNodeCache::CreateUniverseBlock` (`FUN_1404907b0`): 5 Gpc half-size octree, 10 levels = 10 equal-probability slices of N(-22, 5) absolute magnitudes (+-2 sigma), 16384 candidates per node from a node-seeded LCG pair, accepted by a domain-warped Voronoi `F2-F1` cosmic web, type/orientation/radius/model draws, kept only when fainter than `GalaxyMaxAppMagn` 12.1; names `RG 0-<level>-<node>-<index>`. Verified: two galaxies named in saved places sit 111 pc / 18 kpc from the saved cameras | `docs/formats/procedural_deepsky.md`, `proc_galaxy.rs` (decompiled + validated) |
| Cosmic web tables | value noise (Ken Perlin permutation, `2i/255-1`), 64^3 periodic Voronoi points from consecutive LCG draws after `3*64^3` draws of the float noise table (RNG stream starts at 0), 3-octave warp gain 0.6 / lacunarity 2.218282 | `FUN_1400692e0`, `FUN_140069040`, `FUN_140062490`, `FUN_1400621d0` |
| Procedural clusters and nebulae | per-galaxy lists (`FUN_14049b390`): seeds from the galaxy position, nebulae diffuse/SNR/planetary then clusters nucleus/embedded/globular/open, counts and caps from volume x `ClusterGenerator`/`NebulaGenerator` densities, exponential `min - ln(u) 0.0724 (max-min)` draws, home-galaxy brightness cut (8.1 / 6.1 mag); emission-map acceptance approximated; names `RC`/`RN <galaxy>-<index>` | `docs/formats/procedural_deepsky.md`, `proc_galsys.rs` (structure decompiled, placement approximated) |
| Rings | 2-row radial RGBA textures (row 1 lit, row 0 dusty/transmitted) | `docs/formats/rings.md` |
| Locale | `eng-gui.cfg` source-keyed strings; `*-db.cfg` `RObject` records | `locale.rs` |
| Music | `music/context.cfg` classes → tracks | `music.rs` |
| Scripts (`.se`) | `.sc` syntax used as a command list; command table taken from the exe; `WaitTrigger "Kind\|Name"` events, `if`/`loop`, variables, `GotoLocation` = place fields | `docs/formats/se_script.md`, `se_script.rs`, `scripting.rs` |
| Wiki | `*-db.cfg` `RObject`/`PObject` records: `LocName`, `Name`, `Parent`, `Descr` with BBCode-like tags (`[sup]`, `[deg]`, `[EARTH]`, …) | `locale.rs`, `wiki.rs` |
| Procedural rings | rings without `Texture` use `frequency`, `densityScale/Offset/Power`, `colorContrast`, `FrontColor`, `Back*Color`; density = saturate(noise·scale + offset)^power | `rings.rs` (engine) |
| Auroras | `Aurora { Height, TopColor, BottomColor, North/South Lat, Lon, Radius, Width, Rings, Bright, FlashFreq, MoveSpeed }` (km, degrees) | `body_params.rs`, `aurora.rs` |
| Spacecraft | `models/spacecraft/ShipsModels.pak`: `.sss` designs (mass, engine accelerations, warp flags, camera presets, `Module` lines with 4×4 matrices) and module `.cfg` (mesh, materials, `EngineEffect`); engine keys are accelerations in m/s² (turn: rad/s²), `WarpBoostLog` = max log10 speed boost limited by altitude | `docs/formats/spacecraft.md`, `ship.rs` |

## Method notes

- `re/strings_ascii.txt` and `re/strings_utf16.txt` hold every string in the
  executable. Grep them to find a loader, then use `ghidra strings refs` / `x-ref` /
  `decompile`.
- Many formats were resolved from the data alone (size arithmetic, value statistics,
  image comparisons). The decompiler was used where the data was ambiguous: model
  formats and the galaxy generator.
