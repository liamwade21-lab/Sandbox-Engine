# Procedural deep sky: galaxies, star clusters and nebulae

Decompiled from `SpaceEngine.exe` (Ghidra, function addresses below). Code: `crates/se_core/src/proc_galaxy.rs`
(galaxies, exact) and `crates/se_core/src/proc_galsys.rs` (clusters and nebulae inside a galaxy, structure decoded,
placement approximated); engine side `procgalaxies.rs`, `procsystems.rs`.

Tags: **[V]** decoded and checked against the exe's own data, **[D]** decoded from the code, **[A]** approximated.

## 1. Procedural galaxies (`seGalNodeCache::CreateUniverseBlock`, `FUN_1404907b0`)

### 1.1 Octree

* Universe = cube of half-size **5 Gpc** centred on the origin (the Sun's frame), 10 levels (0..9). [V]
  Root half-size from the node centres of two saved places (`RG 0-9-110030168-905` and `RG 0-3-116-2668` in
  `save/places-def.cfg`): the ratio `place position / node centre` is 5.00e9 pc on every axis.
* Child of node `p` in octant `o` (`FUN_140494f30`): `id = p.id * 8 + o`, half = `p.half / 2`,
  centre = `p.centre + half * dir[o]`, `dir` table at `0x1407f4be0`: bit 2 = -x, bit 1 = -y, bit 0 = -z. [V]
  The node id in the name is therefore the base-8 path from the root. Level 9 nodes are 19.5 Mpc cubes.
* Every node generates **16384 candidates** (`min(100000, 0x4000)`, level table float `[3]` = 100000). [D]
* Node visibility in the exe is `(d - r)^2 * 0.05 < K * L_level` (`FUN_1404940c0`, `K` is a runtime brightness);
  the port uses its own reach rule (see section 4).

### 1.2 Levels are equal-probability magnitude bands (`FUN_1401bda90`)

Absolute magnitudes are N(mean -22, sigma 5) truncated at +-2 sigma (-32 .. -12). The span is cut into 10
slices of equal probability (quantiles `Phi(-2) + k * (Phi(2) - Phi(-2)) / 10`, rounded to `f32` at each step):

| level | M range | node size |
|---|---|---|
| 0 | -32.00 .. -27.92 | 10 Gpc |
| 1 | -27.92 .. -25.97 | 5 Gpc |
| 2 | -25.97 .. -24.49 | 2.5 Gpc |
| 3 | -24.49 .. -23.21 | 1.25 Gpc |
| 4 | -23.21 .. -22.00 | 625 Mpc |
| 5 | -22.00 .. -20.79 | 312 Mpc |
| 6 | -20.79 .. -19.51 | 156 Mpc |
| 7 | -19.51 .. -18.03 | 78 Mpc |
| 8 | -18.03 .. -16.08 | 39 Mpc |
| 9 | -16.08 .. -12.00 | 19.5 Mpc |

So coarse nodes hold the rare luminous galaxies and the finest nodes (only built near the camera) the dwarfs.
SE itself has **no galaxies at level 0**: every `RG 0-0-0-*` is "Object not found" in 0.991, while levels 1-3
match the port galaxy for galaxy (`RG 0-1-0-0` Sc M -26.34 L 2.94e12, `RG 0-2-0-1` SBb -24.94 8.11e11,
`RG 0-3-0-769` Sc -24.03 3.5177e11; test `matches_se_lookups`). The port's `generate_node` returns none there.

Nearly all galaxies of levels 1-3 hit the 50 kpc radius cap, so their light per area is up to tens of times that
of the radius law; SE draws them like any other (its `RG 0-1-0-0` and `RG 0-3-0-769` at 3 radii look like normal
spirals, dimmer than M 31). The port draws every procedural galaxy with `ProcGalaxy::vis_abs_mag`, the
luminosity the law gives its radius (`GalaxyRecord::vis_mag`); the Selection panel keeps SE's magnitude.
`L = 10^((4.83 - M) * 0.4)` (solar units, float arithmetic). [D]

### 1.3 Random streams and the candidate loop

* Two LCG streams `x -> x * 1664525 + 1013904223` (u32), both seeded with
  `seed = (int)node_id * 0xCB0 + a * 0x875 + level * 0x19A2` (32-bit wrap; `a` = 0 for the universe octree).
  Stream S1 (TLS `+0x24`) drives positions, acceptance, type, orientation, radius and model; S0 (`+0x20`) only the
  magnitude. [V]
* Per candidate (S1): three draws `off = (float)(2*half) * (float)u * 2^-32 - half` (float arithmetic, then
  widened), one draw for the acceptance test. [V]
* **Cosmic web density** (`FUN_140061120`, `FUN_140062490`, `FUN_1400621d0`): with `s = pos_pc * 2e-8`
  1. warp = 3-octave vector value-noise fBm of `s * 2.3179` (gain 0.6, lacunarity 2.218282; the y/z outputs
     sample at offsets `(3.33, 5.71, 1.96)` and `(7.77, 2.65, 4.37)`), `q = s + 0.15 * warp`;
  2. `F1, F2` = squared distances to the nearest and second-nearest feature point of a **64^3 periodic** Voronoi
     grid (one random point per cell, 27 cells searched);
  3. `density = (1 - |sqrt(F2) - sqrt(F1)|)^20`; the candidate is accepted when `u * 2^-32 <= density`.
  Galaxies therefore pile up on Voronoi walls and edges: filaments, sheets and voids with a 50 Mpc cell. [V]
* Tables (`FUN_1400692e0`): value noise = Ken Perlin's permutation with `value[i] = 2 i / 255 - 1`;
  Voronoi points = consecutive LCG draws `(x, y, z)` per cell in `x + 64 (y + 64 z)` order, drawn from the process
  RNG stream that starts at 0 and has consumed `3 * 64^3` draws in `FUN_140069040` (another noise table). [V]
* Accepted candidate:
  1. `M = truncated_normal(-22, 5, level_lo, level_hi)` from S0: `p = (erf(a) + u * (erf(b) - erf(a)) + 1) / 2`,
     `M = -22 + 5 * ndtri(p)` (`FUN_140443fd0`);
  2. Hubble type from `L` and one S1 draw (`FUN_1401bd930`): `L < 1e9`: 40 % E (E0..E7 uniformly), 5 % -> E7/S0/Sa
     (`10 - trunc(3 * ...)`), 55 % Irr; `L >= 1e9`: 11 % E, 15 % S0, 71 % spirals (`Sa/SBa`, `Sb/SBb`, `Sc/SBc`
     split 0.157 / 0.301 / 0.542 of the range, bar fractions 0.529 / 0.539 / 0.443), 3 % Irr. Indices as in
     `docs/formats/galaxy_model.md` 2.1 (E0..E7 = 1..8, S0 9, Sa 10, Sb 11, Sc 12, SBa 13, SBb 14, SBc 15, Irr 16);
  3. orientation = 4 draws, each `1 - 2u`, normalised (`FUN_1404415a0`; component 0 is treated as `w`);
  4. if `L > 1e12`: one draw, 5 % become *compact* (radius computed from `L / 1000`);
  5. radius `r = min(sqrt(L / s), 50000)` pc with `s = (9 - n) * 30` for E`n-1`, 30 for S/SB, `u * 5 * 300` for
     Irr (one more draw), 1000 otherwise (`FUN_140464db0`). E galaxies are drawn `GalaxyEllipticalScale` = 2.4
     times larger with axes `(1 - n/10, 1, 1 - n/10)`;
  6. for types 9..16 one draw `% pool size` picks the sprite model (the same per-type pool as for catalog
     galaxies, sizes 10/13/26/11/4/25/42/11 in the shipped data).
* Finally the galaxy is **kept only if its apparent magnitude from the Sun exceeds `GalaxyMaxAppMagn` = 12.1**
  (`FUN_1401d8300`: `5 log10(d) - 5 + 4.83 - 2.5 log10 L`): everything brighter is left to the catalogs. The
  *index* in the name counts kept galaxies; rejected candidates still consume their random draws. [V]

### 1.4 Name

`RG %i-%i-%I64d-%i` = `RG 0-<level>-<node id>-<index>` (`FUN_1401c0520`, parser `FUN_1401c2480`). [V]

### 1.5 Verification

`cargo test -p se_core proc_galaxy::tests::matches_saved_places_of_the_exe`: galaxy 905 of node (9, 110030168)
("The Smallest Heart") is an `Irr` of radius 60 pc, **111 pc** from the saved camera position; galaxy 2668 of node
(3, 116) (parent of `RC 0-3-116-2668-52089`) is a 50 kpc `SBa`, 18 kpc from the saved camera. A wrong noise table,
RNG order or acceptance rule would shift every later index, so these two hits confirm the whole chain (including
the zero seed of the noise tables).

## 2. Clusters and nebulae inside a galaxy (`seGalSysModelCache::CreateModel`, `FUN_14049b390`)

Every galaxy owns one cluster list and one nebula list, generated on demand (objects are numbered per galaxy;
the nebula object id is offset by `0x4000000` inside the exe, `FUN_1401c0770`).

* Seed `h = (int)(x*0.001)*0xB7823 - (int)(z*0.001)*0xE6490 + (int)(y*0.001)*0xCD7D4` of the galaxy's float
  position (for procedural galaxies the offset inside the node). Nebulae: `S24 = h * 0x17D63`, `S20 = h * 0x151F1`;
  clusters: `S24 = h * 0x34CB`, `S20 = h * 0xB5E1`. Nebulae are generated first. [D]
* Extents `X, Y, Z` = `r * (model R, R, R * SpriteOblZ)` for types 9..16 with a sprite model, `(1 - n/10, 1, 1 - n/10) R`
  for ellipticals. **Groups and order** (index = catalog entries of the galaxy first, then in this order): [D]

| nebulae | count `min(volume * density * 1e4, 8000 + u % 4000)`, volume = `X Y Z * 1e-13` | placement |
|---|---|---|
| diffuse (class 1) | `NebulaDiffDensity` (1) | blob x `(X, X, 0.02 X)`, accepted by the galaxy emission map |
| SNR (class 3) | `NebulaSNRDensity` (0.1) | blob x `(X, X, 0.08 X)`, emission map |
| planetary (class 2) | `NebulaPlanDensity` (5) | blob x `(X, X, 0.3 X)` (ellipsoid for E) |

Diffuse and SNR groups exist only for sprite-model galaxies. Per accepted nebula: luminosity
`min - ln(u') * 0.0724 * (max - min)` with the `Nebula*Lum` range, radius likewise with the `Nebula*Radius` range
(`FUN_140444350`), random orientation, one model of the class pool (two stream steps, `% pool size`).

| clusters | count | placement |
|---|---|---|
| nucleus (class 1) | 1 | centre; radius/luminosity from `ClusterKern*` x size factor |
| one per diffuse nebula | one each | at the nebula; radius from `OpenRadius` capped by `N(0.4, 0.02) x nebula radius`, young (20 Myr) |
| globular (class 2) | E: `10^(-0.4 (M+15)) (a f + 1)`, S0: x3, others `X^2 * 1500 * 1.786e-12 * GlobDensity * 200`, capped `80000 + u % 40000` | blob x `(X, X, 0.7 X)` (halo), not for plain Irr |
| open (class 3) | `X Y Z * 1.389e-13 * OpenDensity * 1e5`, capped `350000 + u % 100000` | blob x `(X, X, 0.05 X)`, emission map |

Globular luminosity from `ClusterGlobAbsMagn` (N(-7.45, 0.95)); radius/luminosity ranges and the ages
(`N(0.15, 0.1)` Gyr open, `N(11, 3)` globular) are in `DeepSkyConfig` and read from `config/main-def.cfg`.
The blob (`FUN_140441860(k = 0.25)`) is a random point with `r^2 = -k ln u` and an isotropic direction.

* **Home galaxy cut**: in the galaxy that contains the Sun, nebulae and globular/open clusters whose brightness
  from the Sun exceeds `NebulaMaxAppMagn` (6.1) / `ClusterMaxAppMagn` (8.1) are dropped (`lum <= d^2 *
  10^(0.4 (-0.17 - m_lim))`). The embedded young clusters and the nucleus are not cut. So from the Sun's sky
  procedural objects are fainter than the catalogs' limits and only a few are naked-eye or binocular.
* Counts for the Milky Way come out near 80 000 clusters and 16 000 nebulae with the port's emission map; giant
  spirals reach 120 000 clusters.
* Names: `RC %i-%i` / `RN %i-%i` = (galaxy catalog index, object index), e.g. `RN 8514-9602` (the Milky Way's SE
  catalog index is 8514); `RC %i-%i-%I64d-%i-%i` / `RN ...` = (0, level, node, galaxy index, object index), e.g.
  `RC 0-3-116-2668-52089`. Stars: `RS`, `RSC` (in a cluster), `RSN` (in a nebula) in `FUN_1401c0180`; their first
  numbers are the galaxy id, then the star-octree root, level, node and index.

### 2.1 What is approximated [A]

* The exe accepts the emission groups against the galaxy model's emission map (`FUN_1402e44d0`, `FUN_140440840`)
  and passes a hidden register to `cos`/`sin` in the blob. The port supplies the Milky Way's voxel map from the
  sprite model (an analytic two-arm spiral for other galaxies) and takes the azimuth from the first draw. The
  acceptance scale is guessed. Counts, group order, classes, parameter ranges, seeds, caps and the home cut follow
  the exe, but **individual object indices do not land on the same objects as in SpaceEngine** (for example the
  saved place `RN 8514-9602` is a planetary nebula here, not the "Hidden Star Forge").
* The galaxy index in `RC`/`RN` names is SE's catalog index only for the Milky Way (8514); other catalog galaxies
  use the port's catalog index.

## 3. Constants and functions

| item | address |
|---|---|
| `CreateUniverseBlock` | `FUN_1404907b0`; node init `FUN_140494f30`; level table `FUN_1401bda90` |
| noise tables | `FUN_140069040` (float table), `FUN_1400692e0` (value noise + Voronoi); lookup `FUN_140061200`, warp `FUN_140062490`, F1/F2 `FUN_1400621d0` |
| record init / radius / type | `FUN_1404648b0` / `FUN_140464db0` / `FUN_1401bd930`; quaternion `FUN_1404415a0` |
| helpers | truncated normal `FUN_140443fd0`, `ndtri` `FUN_140444dc0`, `erf` `FUN_14013ece0`, apparent magnitude `FUN_1401d8300` |
| subsystems | `FUN_14049b390` (orchestrator), nebulae `FUN_14049def0`, clusters `FUN_14049cf40`, class init `FUN_1404826a0` / `FUN_1404829f0`, nebula model pick `FUN_14026f250` |
| names | RG `FUN_1401c0520`, RC/RN `FUN_1401c0770`, RS `FUN_1401c0180`, parser `FUN_1401c2480` |
| settings | `GalaxyGenerator`, `ClusterGenerator`, `NebulaGenerator`, `ShowProcGalaxies/Clusters/Nebulae` in `config/main-def.cfg` |

## 4. Port behaviour

* `procgalaxies.rs`: per level, nodes within one node size of the camera whose brightest possible galaxy reaches
  magnitude 19 are generated on the compute pool (about 140 nodes, 380 000 galaxies at 50 Mpc, a few ms per node) and
  drawn as far sprites through the catalog galaxies' shader, one mesh per node (vertices relative to the node,
  entity translation = centre - camera). Galaxies fainter than 18.5 are skipped when a node is built.
* Large procedural spirals/irregulars (angular radius above 0.7 x the model build size) become catalog galaxies
  (`Catalog::add_galaxy_record`), so `galaxies.rs` builds their 3D models and fades the far sprite out; names are
  resolved from commands, the search box and saved places (`goto RG 0-9-110030168-905 9e15`).
* The "intergalactic vision" gain (`AutoGalaxyMagnitude`): the streamer sums the sky covered by the drawn
  procedural galaxies (`galaxies::sky_coverage`) for `galaxies.rs`, which adds the catalog galaxies and applies
  SE's gain (galaxy_model.md, "Intergalactic vision").
* `procsystems.rs`: the Milky Way and galaxies the camera is near get a generated system; clusters and nebulae are
  far sprites (brightest 20 000 of each as seen from the camera); clusters within `max(300 pc, 60 radii)` and
  nebulae above ~3 px become catalog entries, after which `clusters.rs` makes their member stars and `nebulae.rs`
  their 3D models.
* Switches: Settings window ("Procedural deep sky"), `ShowProc*` from `main-def.cfg`, env `SE_NO_PROCGAL=1`
  (all off) or `SE_PROC=g|c|n|gc...` (only those on), `SE_PROC_DEBUG=1` (clusters green, nebulae red, 4x larger).
