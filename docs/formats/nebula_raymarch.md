# Ray-marched nebulae (`models/custom/*.cfg`)

Port: `se_core::custom_model`, `se_engine/src/nebula_raymarch.rs`, shader `galaxy_nebula_raymarch.wesl`.
Tags: [V] verified in the exe, [I] inferred, [P] port choice.

## 1. Files

`data/models/custom/Nebulae-raymarch.pak`: `BambusDude.cfg`, `Duke.cfg`, `HD-Phunnie-v9r4.cfg`,
`JackDole.cfg`, `SpaceEngine.cfg`, `Tarantula.cfg`; 112 `CustomModel "name" { ... }` blocks (63 for
`Nebula/Diffuse`, 32 `Nebula/Planetary`, 16 `Nebula/SNR`, Tarantula `Nebula/NONE`), all with
`Shader "nebula_raymarch.glsl"` and `ShaderUniforms "nebula_raymarch_uniforms.cfg"` (both in the
encrypted `Shaders.pak`). 25 name a nebula in `UseForObject` (the 20 of `catalogs/nebulae/RaymarchBambus.sc`,
Manatee, M 82 Starburst, Schleier, Skylias Stellar Nursery, 30 Dor).

## 2. Keys

The exe (`seCustomModel::ReadFromScript`) reads `UseForObject`, `UseForType`, `EnableImpostors`,
`EnableBlend`, `EnableDepthTest/Write`, `Shader`, `ShaderUniforms`, `BaseShape` (`box`, `sphere`,
`cylinder`), `BaseShapeDims`, `Scale`, `Randomize`, `ParticleColor`, `ExpansionBegin`,
`ExpansionDuration`, textures [V]. Every other key goes to the shader as a uniform of the same name
(`CustomModelUniforms2`: `uFlags`, `uIntData`, `uFloatData`, "Rename uniforms to match names of
parameters in the script") [V]. Most of those come in `(dust gas)` pairs: one value for a dust volume, one
for an emitting gas volume.

## 3. Which nebula gets which model [V]

1. A custom model naming the nebula (`FUN_140486cb0`) wins, over a named sprite model too.
2. Then a named sprite model (`models/nebulae/*.pst`, nebula.md section 2).
3. Every other catalog nebula takes `pool[h % len]` from the **custom** models of its type
   (`FUN_140270120`, with `h` the position hash of nebula.md section 2): the switch between the sprite and
   the custom pools (`DAT_140d8dcd0`) is set to 1 at start and never cleared. Pool order = file name order.

With the shipped data 75 catalog nebulae are ray-marched. Procedural `RN` nebulae
keep the sprite pools in the port.

## 4. The shader [V]

`galaxy_nebula_raymarch.wesl` is SE's `nebula_raymarch.glsl` and the parts of `raymarch_common.glh` it uses,
translated from the decrypted shader (`re/se_shaders`):

* Uniforms: the script's keys fill `CustomModelUniforms2` as `nebula_raymarch_uniforms.glh` lays it out
  (`uFlags.x` bits, `uIntData[0].x` = `Raymarch_step_count`, `uFloatData[39]`; `nebula_raymarch.rs` `FLAGS` /
  `FLOATS`). Every shipped model sets every key. The draw adds `uShapeDims` (`BaseShapeDims`), `uRandomize`
  (the catalog nebula's `Randomize`, FUN_140172400, plus the model's) and `uParamsImg.zw` (FUN_1404877f0).
* Internal space = model space * `Internal_scale` (`ro = uEyePos * Internal_scale`), model space = nebula radii /
  `Scale`. The ray starts where it enters the bounding sphere / cylinder / box and steps by
  `max(d * 0.1 * min(1.65, light distance), 0.1)`, `d = max(dust distance, 0.75)` dithered by 0.8..0.88, for at
  most `Raymarch_step_count` steps or until the transmittance is below 0.01.
* Fields: `NebulaFormFunc` (signed sum of the plane, disk (`length8`), cylinder, capsule, sphere, torus, spiral
  SDFs and the iq / fbm / spiral noises, each on its own axis order), evaluated at `p / NebScale` and scaled
  back, plus `NebScaleCoef`, the `NebScaleAbs*` folds, the optional twist and `NebulaMapThickCoef`; the bounding
  shape and the star's cavity are eroded by `abs(1 - 1.2 noise)`. Noise is iq's 2-channel value noise on SE's
  16 x 16 `IqNoiseTex` (`blackholes::noise_texture`), offset by `uRandomize.xy`; the spiral noises add
  `uRandomize * 10`.
* Dust: density from the dust distance (remapped over `Dust_volume_height`, times 0.4..0.5 per pixel), extinction
  `(scattering + absorption) * Dust_volume_density_factor` per channel (0.748, 1, 1.324), ambient light and,
  inside `Light_radius_single_star`, the star's light with SE's two-lobe phase function (whose comma expression
  makes it `(1 + c^2)(1 - g^2) / (2 + g^2) + g c`, with the light direction unnormalised).
* Gas: `e = (max(D, 1e-4) + coef_0.x) dir.x + (|min(D, -1e-4)| + coef_0.y) dir.y + (|D| + coef_0.z) dir.z`
  per gas, emission `e * colour * coef_1 * exp(-e^coef_2 * coef_3)`, clamped to 0..1, then absorbed and emitted
  within `Gas_volume_height`. `Render_star` / `Render_star_bloom` add the star's glow at every step.
* Output: `sum^3 (6 sum^2 - 15 sum + 10) * colour brightness`, alpha `(1 - grey transmittance) * alpha brightness`.
  Colour brightness = `Bright * 10 * GlowBrightMin * coef(GlowMagnLimit) * hdrNebulaRaymarchBright * f`, alpha
  brightness `f`, with `f = clamp((px - 4) / 3) * clamp(NebulaBrightMin coef(NebulaMagnLimit) L / d^2 -
  NebulaBrightMin)` (FUN_140488a20), `px` the nebula's radius on screen.

Port differences [P]: up to 4 volumes at a time, drawn at full resolution (SE marches at `VolumResolution`
into its own buffer); the dither texture `DitherNoiseTex` (contents not traced) is replaced by a per-pixel hash;
SE's per-channel transmittance (`OutTransm`, dual-source blending) becomes one grey alpha; a negative `e` in
the gas emission zeroes that sample's emission (SE's `pow` gives NaN there, which its `clamp` turns to 0 on
NVIDIA). `ExpansionBegin` / `ExpansionDuration` only feed `uParamsTime.x` (FUN_1404877f0), which
`nebula_raymarch.glsl` never reads, and `Star_field_scale` / `td_break_value` are declared but unused: SE's
nebulae do not expand.
