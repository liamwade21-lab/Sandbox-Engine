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

## 4. The port's ray marcher [P]

SE's shader is encrypted, so `galaxy_nebula_raymarch.wesl` is a reimplementation in the style of Duke's
public nebula shaders, which the parameter names follow:

* Internal space = model space * `Internal_scale` / `Scale`; the bounding shape is `BaseShapeDims`.
* Each volume's field is the signed sum `sign * form` of the enabled `NebulaForm*` terms (plane, disk,
  cylinder, sphere, capsule, torus, spiral, value noise, fbm, Duke's spiral noises with the
  `SpirNoise*`/`*_3D_2` pairs). Matter lies within `Dust_volume_height` / `Gas_volume_height` of the
  field's zero level, scaled by `exp(NebulaMapThickCoef)`, fading over the outer fifth of the bounding shape.
* Forms are evaluated at `q / NebScale`, unless that puts the model's largest form outside its bounding
  shape; then at `q * NebScale` (`form_scale`). The Spirograph Nebula needs the first reading to get SE's
  size and the Omega Nebula's `Diff 1 JackDole` the second to show at all.
* Dust absorbs and scatters the ambient light and, with `Single_star`, the star's light with a two-lobe
  Henyey-Greenstein phase function; gas glows in `Nebula_emission_color_N` with a weight
  `coef_3 * exp(-max(r - coef_1, 0) / coef_2)`, `r` the distance from `coef_0` with the axes weighted by
  `direction` (floors of 0.25 and 0.15 keep a single extreme value from hiding a model). `Render_star` and
  `Render_star_bloom` add Duke's star glow.
* Steps: `Raymarch_step_count * (0.5 + VolumResolution)`; brightness `Bright * 0.02` (matched by eye to
  SE's Spirograph). Premultiplied output: dust also hides the sky behind it.

Unknown: `NebScaleCoef`, `NebScaleAbs*`, `Star_field_scale`, `Randomize` and the expansion keys are read
but unused. The look follows each model's parameters but is not SE's image.
