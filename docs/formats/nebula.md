# Nebulae (`NebulaModel` `.pst`, `Nebulae-sprite.pak`, `seSpriteModelMaker`)

Nebulae are rendered exactly like galaxies: a catalog entry (`Nebula` object in the `.sc` catalogs) is shown
as a far billboard and, when large enough on screen, as a **3D cloud of sprites** built by the same
`seSpriteModelMaker` that makes the galaxy models (see `docs/formats/galaxy_model.md`, which defines the
`.pst` syntax, the voxel -> sprite pass, the LCG and the atlas). This note only records what differs.
Status: [V] verified against the decompile or shipped data, [I] inferred / matched visually.

## 1. Data

| What | Where | Count |
| --- | --- | --- |
| Model templates | `models/nebulae/*.pst` (`NebulaModel "name" { ... }` blocks) | 135 |
| Nebula images | `textures/nebulae/*.png` (cube faces `<name>Pos/NegX/Y/Z`, `...-front`, `...-side`, ...) | 222 |
| Sprite atlas | `sprites_neb.png` (`Common.pak`, 1024^2 grey, 4x4 filamentary tiles, used as coverage: nebula_model.glsl `.a`; loaded next to `sprites_gal` in FUN_140168bd0) | 1 |
| Catalog | `catalogs/.../nebulae*.sc` (`Nebula "Orion Nebula" { Type "Diffuse" Radius, AbsMagn, ...}`) | 193 |

The block keyword is `NebulaModel` instead of `GalaxyModel`; every key of galaxy_model.md section 3 applies
(`Method`, `Radius`, `SpriteObl*`, `Emission`/`Absorption` layers, `ParticleColor`, `ParticleColor2`,
`UseForObject`, `UseForType`, ...). Nebula models additionally store the validation values
`clipRadius` and `sumColor` (written by the exe after generation; they are not inputs) [V].

Model methods used by the shipped files:

| Method | Models | Generator |
| --- | --- | --- |
| `CubeMap` | 73 | galaxy_model.md 4.2 (Pos/Neg face pairs multiplied) [V] |
| `Cylindrical` | 51 | section 3 below [V] |
| `Diffuse` | 3 (`Diff0..5` defaults) | galaxy_model.md 4.3 [V] |
| `Spherical` | 1 (`Plan3`, `NGC6720_small` image) | section 4 below [V] |
| `Planetary` / `SNR` | 3 (`Plan0..2`) / 4 (`SNR0,1,2,4`), no images | `createNebula` (`FUN_1402d3360`), section 5 below [V] |

## 2. Which nebula gets which model [I]

1. A model whose `UseForObject` list contains one of the nebula's names (aliases included) wins.
   119 of the 193 catalog nebulae have such a model.
2. Otherwise the models of the nebula's `Type` (`UseForType`: Diffuse, Planetary, SNR) form a pool and the
   pool entry is `pool[h % pool.len()]`. The exe function is `FUN_140270120` (nebula model cache init) [V]:
   `h = ix*0xcd2 + iy*0x1cb7 - iz*0x4d0f` (u32 wrap), `i* = (int)` of the nebula position (float parsecs,
   *not* scaled by 0.01 like the galaxy hash of galaxy_model.md 2.2) wrapped into `+-(2^31-1)` with
   `x - 2*floor(x/(2^31-1)*0.5+0.5)*(2^31-1)`. A nebula whose type has an empty pool gets no model.
   Port: `se_core::galaxy_model::nebula_position_hash`. The pool order (model load order of the exe) and the
   catalog position unit are [I]; a model without `UseForType` also joins the pool of the type of the first
   nebula it is named for (`model+0x48`, set in the same function), which the port does not do.
*Correction (2026-10-27):* in 0.991 the exe takes the pool from the ray-marched custom models
(`DAT_140d8dcd0` is constant 1, see nebula_raymarch.md section 3), so step 2 applies only to procedural
nebulae; catalog nebulae without a named model are ray-marched.

3. The pool contains **every** model of the type, named ones included (so a random planetary can show the
   Helix image), and since all three procedural generators are implemented the default models `Plan0..3`,
   `SNR0..4` are pool members too. With the shipped catalog this picks a procedural model for about 10
   catalog nebulae (Abell 30 `Plan2`, Abell 33 `Plan1`, Manatee/W50 `SNR1`, IRAS 23206+4637, Wray 16-100,
   NGC 4071, Cat's Eye ... `Plan3`). The "procedural nebulae" of the exe (settings `NebulaGenerator`,
   `NebulaPlanDensity`, `NebulaSNRRadius`, ...) are *generated* objects of the procedural galaxy, which use
   the same default models; the port does not generate them (see known gaps).

Texture names resolve with the rules of galaxy_model.md 2.3 against `textures/nebulae`.

Repeated keys: the exe looks keys up by name inside the block, so the **first** occurrence wins
(`SNR0` writes `emParticleColor` twice where `emParticleColor2` was meant: only the first-wins reading
reproduces its stored `sumColor`). `parse_pst` now ignores later repeats. [V]

## 3. `Cylindrical` generator (`createCylindrical`, `FUN_1402d75c0`) [V]

Inputs: a *front* image (radial profile / colour, seen along the axis) and a *side* image (profile along the
axis). Emission (absorption is analogous, thresholded on the alpha channel):

* `nxy = trunc(DetailR * 64)` (64 if 0); `nz = trunc(side_aspect / SpriteObl * DetailZ)` with
  `side_aspect = side.height * 64 / side.width` (15 if the side image is missing); voxel grid scale
  `(1, 1, nz / nxy)`.
* Per voxel: `r = sqrt(x^2 + y^2)` (unit disc), density = front image at `(x, y)` times side image sampled at
  `+-r` along the axis, multiplied by the Diffuse-style normalisation (`norm_diffuse`), random atlas tile.
* A voxel survives when `sum < lerp_edge(MinC, MinE, r)`, exactly like the spiral generator's threshold.
* Colour comes from the front image; `ParticleColor2` tints the second layer.

The unit tests in `crates/se_core/src/galaxy_model.rs` and a dump of all 126 buildable models match the stored
`clipRadius` / `sumColor` for 101 models; the other 25 differ because the stored values are stale or were
written in a float form that does not round-trip (`ringhalo` has no cube images at all and fails to build, as
in the exe).

Two points that differ from galaxies:

* The absorption channel is the **last channel of the PNG**: RGB images (no alpha) use their blue channel for
  dust, RGBA images use alpha [V from the loader].
* Nebula model space is scaled by `Radius.x` times the catalog radius; the sprites of the shipped models stay
  within about 1.05 to 1.45 model radii.

## 4. `Spherical` generator (`createSpherical`, `FUN_1402d6920`; `Plan3`) [V]

One image, `FrontImage`, interpreted as a *profile* that is revolved about the z axis. Cubic grids
`n = trunc(Detail*64)` (0 -> 64) per layer, scale (1,1,1), same normalisation as `CubeMap`
(`b / ((sizeC+sizeE)*DetailR*0.5)`). For voxel `(x, y, z)` with `a = sqrt(x^2 + y^2)` and `t = x*0.5+0.5`:

```
emission   = lerp_clamped( front.rgb(+a, z), front.rgb(-a, z), t )
absorption = lerp_clamped( front.last_channel(+a, z), front.last_channel(-a, z), t )    (grey)
```

Sprite pass: the common spherical one (galaxy_model.md section 5) with the jitter of `createNebula`
(see below). `Plan3`'s dust threshold (`MinBright` 10) rejects every dust voxel. Reproduces the stored
`clipRadius` 1.01831 and `sumColor`.

## 5. `Planetary` / `SNR` generator (`createNebula`, `FUN_1402d3360`) [V]

No images. Both layers use cubic grids `n = trunc(Detail*64)` (0 -> 64; `DetailZ` is unused), scale (1,1,1),
the brightness normalisation of `CubeMap`, and the sprite pass of galaxy_model.md section 5 with the
*spherical* radius, `size = step * lerp(SizeC, SizeE, maxchannel)` and **`DispR` used for all three axes**
(`DispZ` is ignored: the exe copies `0x4c0` to `0x4c4/0x4c8`; this also holds for `Spherical`). RNG (the
seeded LCG of galaxy_model.md section 5), in the exe's order:

1. `thick = u1 * 0.35 + 0.3` (x0.2 for SNR), `inner = 0.95 - thick`; `squash = u2 * 0.3 + 0.7`
   (`u = (float)state * 2^-32`, evaluated as `raw * 0.35 * 2^-32 + 0.3`).
2. **Emission map**, voxel loop x, y, z (centres of galaxy_model.md section 4):
   `r2 = z*z/squash + y*y + x*x`. SNR draws one number per voxel first and drops the voxel when
   `state % 100 < 50`. A voxel is kept when `inner <= r2 <= 0.95`; its value is `t = (r2 - inner)/thick`,
   `v_k = lerp(1, ParticleColor2_k / ParticleColor_k, t)` per channel, so after the sprite pass the colour
   (`v * ParticleColor / max(v)`) runs from `ParticleColor` on the inner surface to `ParticleColor2` on
   the outer one.
3. **Dust map**: `athick = u3 * 0.2 + 0.05`; one draw per voxel; with `r2 = x^2+y^2+z^2` (no squash; the z
   *start* coordinate is `step_z*0.5/squash - 1`, the increment stays `step_z`, so the dust grid is shifted
   slightly in z) a voxel is kept when `inner <= r2 <= athick*thick + inner` and `state % 100 > 29`
   (70 %); value = `absParticleColor`. The dust is a skin on the inside of the shell.
4. Sprite passes: emission, then dust (atlas tile random 1..15 for both; jitter and LOD byte as usual).

Validation: the exe wrote `clipRadius` / `sumColor` into the shipped `Plan0..2`, `SNR0,1,2,4`, `Plan3`, and
the generators reproduce `clipRadius` to all five stored digits and `sumColor` to +-1 byte for all eight
models (unit test `planetary_matches_the_exe_validation_values`; example
`se_core/examples/nebula_check.rs`, whose `all` mode compares every shipped model: 105 of 135 match).

**SNR bug in 0.991**: `createNebula` tests `(method << 28) >> 28 == 8` on a *signed* 4-bit field (the
disassembly is `shl 28; sar 28; cmp 8`, never true for 8) and the `switch` of `FUN_1402db910` has no case for
method 8. The shipped exe therefore cannot take the SNR branch, but the stored validation values of the SNR
models prove that an earlier build did. The port implements the evident intent (thin shell, 50 % cull),
which is what the stored values describe.

## 6. Port rendering (`crates/se_engine/src/nebulae.rs`)

* **Far field**: one billboard per catalog nebula at infinity, drawn by the galaxy far shader
  (`galaxy_far.wesl`, profile 2 = soft disc) with flux from `AbsMagn` and the distance
  (`m = M + 5 log10(d/10pc)`, same photometric constants as galaxies). The colour is the emission-weighted mean
  of the generated sprite colours (`sum(colour * brightness * tile_mean * size^2)`, normalised), computed
  for all nebulae in two background tasks at start-up (brightest first); until then a type colour is used.
  The stored `sumColor` is **not** usable for this (it disagrees with the generated colour for most models).
* **Near volume**: when the angular radius exceeds `build_px` (4 px) the model is built on the async pool
  (at most 4 resident, least recently wanted evicted) and drawn with `galaxy_model.wesl`: emission as
  `AlphaMode::Add` (premultiplied, alpha 0) and dust with SE's `ONE, ONE_MINUS_SRC_ALPHA` blend
  (`AlphaMode::Premultiplied`, output `(-dust colour * emBright, Transparency) * coverage`, see galaxy_model.md
  section 11). Sprite colour and intensity are clamped to 0..1 like SE's vertex bytes. The far sprite cross-fades
  out through `model_fade` and the model scale is calibrated so that its mean surface brightness equals the
  far sprite's (`K = target / S_raw`, same as galaxies).
* **Ordering and daylight**: draw order far < emission < dust, all with a huge positive `depth_bias` and a
  symmetric padded AABB (see galaxy_model.md section 11) so the atmosphere sky shell cannot erase them; they
  fade with the shared `SkyFade` resource written by the galaxy renderer.
* Defaults: far brightness 0.8, model brightness 1.0, dust 1 (= SE). The reference magnitude is SE's
  `NebulaMagnLimit` + 0.5 (Shift+`]`/`[`); `ShowCatNebulae false` hides them. Catalog nebulae with `NoModel true`
  (M 43, Sh2-279; FUN_140172400 model index 0xfffe) get no model and no far sprite: the M 42 model draws them.
  Env overrides for tuning: `SE_NEB_FAR`, `SE_NEB_MODEL`, `SE_NEB_DUST`.

Results: `goto Orion Nebula 2e14` (21 ly) shows the pink H-II region with the dark dust lane and the Trapezium
region; at 8e14 to 1e15 km it is a compact pink core with the blue reflection nebulae (Running Man, M43)
around it; from Earth at night it is a faint pink smudge below the belt (not blown out). Typical cost is one
extra draw of about 25k sprites per resident model (60 fps in a debug build in the worst, closest view).

## 7. Known gaps

* Procedural (generated) nebulae of the exe (`NebulaGenerator`, `NebulaPlanDensity`, ... settings) are not
  generated; only catalog nebulae are shown. Their models are the `Plan*`/`SNR*`/`Diff*` defaults, which work.
* The pool of a type does not include models without `UseForType` that are named for a nebula of that type,
  and the exe's model load order is only approximated (alphabetical file order): which tiny catalog nebula
  gets which default model is a best guess.
* Catalog planetaries/SNRs are physically bright for their size (`AbsMagn` over a few thousand AU): the
  calibrated model saturates to white when you fly to them (e.g. IRAS 23206+4637, M57 at 3e13 km), while
  faint ones (Abell 33, the Manatee remnant) are nearly invisible at default brightness. `SE_NEB_MODEL`
  scales the model brightness for inspection; `SE_NEB_FORCE="Ring Nebula=Plan0"` forces a default model
  onto one nebula.
* The far sprite of a shell nebula is a soft disc, not a ring (its colour is the emission-weighted mean of
  the generated sprites).
* Cube-map handedness / orientation is the identity (the same as the galaxy models); not independently
  verified against a reference screenshot beyond the overall look of M42.
* Dust uses SE's strength now; where the calibrated emission is far above 1 (close to bright models) the
  subtracted dust colour shows as orange/black (see galaxy_model.md section 11).
