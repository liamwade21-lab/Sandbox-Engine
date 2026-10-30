# Lens flares (`textures/flares/Flares.pak`)

Port: `se_core::lens_flare` (parser), `se_engine/src/lensflare.rs` (sources and sprites),
`se_engine/src/bloom.rs` (drawing), shaders `lens_flare.wesl`, `lens_flare_probe.wesl` and `se_bloom.wesl`
(`combine_flares`). Tags: [V] verified in the exe, [I] inferred, [P] port choice.

## 1. Files and settings

| File | Kind | Selected by (`config/main-def.cfg`) | Default |
| --- | --- | --- | --- |
| `<Name> flares.cfg` | central glow and diffraction spikes | `LensFlarePreset` | `Astroniki2` |
| `<Name> ghosts.cfg` | ghosts along the sun - centre axis | `LensGhostPreset` | `Fireinthehole` |
| `<Name> pseudo.cfg` | screen-space "pseudo lens flare" | `PseudoFlarePreset` | `Default` |

The shipped pak has 10 flare, 9 ghost and 4 pseudo presets plus their atlases (`AstroNiki.png`,
`Anamorphic.png`, ...) and `lenscolor_harb.png` (256x1 lens colour gradient). Other keys:
`EnableLensFlares`, `EnableLensGhosts`, `EnablePseudoFlares`, `LensFlareScale`, `LensFlareBright`,
`PseudoFlareBright` (`PointRendering`), `LensFlareFadeBeginMagn` -30 and `LensFlareFadeEndMagn` -14
(`Lighting`).

## 2. Flare and ghost scripts [V]

```
ScaleAll  0.03
BrightAll 0.6
Texture  "AstroNiki.*"
Flare { (r g b) bright_c bright_e scale_c scale_e aspect position angle_c angle_e chroma (x y w h) }
```

`seLensFlareEffect::ReadConfigScript` (`FUN_14026a9c0`) builds 4 vertices of 88 bytes per `Flare`:
corner (+-1, +-1), brightness * `BrightAll`, scale * `ScaleAll`, angles * pi/180, `1 - position`, aspect,
colour + chroma, atlas rectangle in pixels and the corner's texel. The vertex shader (`lens_flare.glsl`,
encrypted) receives `Projection, UpVector, CenterVector, AtlasRes, Params, Params2, SunScreenPos,
SunWorldPos` (uniform block text in the exe).

Port reading [I, checked against SE screenshots]: `_c`/`_e` interpolate by the sun's distance from the
screen centre (0 centre, 1 edge); a sprite's centre is `sun * (1 - position)` in screen units (0 at the
sun, 1 at the centre, 2 mirrored); its half size is `scale` screen half-heights, its height `scale *
aspect`; `angle` rotates it. Chroma is not used (0 in the shipped presets).

## 3. Pseudo flare scripts [V keys, I semantics]

`RaysTexture`, `DirtTexture`, `GhostsCount`, `GhostsDispersal`, `GhostsBright`, `GhostsBlur`,
`HaloRadius`, `HaloBright`, `HaloBlur`, `DirtBright`, `DirtDispersal`, `Chromaticism`: John Chapman's
screen-space lens flare, drawn by `hdr_combine.glsl` (FLARES): ghosts and a halo of the bloom mip chain
mirrored through the centre (at GhostsBlur / HaloBlur levels, chromatic offsets of Chromaticism frame
texels, ghost weight `smoothstep(0, 5.5, luma)`), times the `LensRays` starburst x3 plus the `LensDirt`
texture; the frame plus bloom is mixed towards the bloom at DirtDispersal levels by dirt x DirtBright. The
script reader (`FUN_14026c610`) clears both texture names first, so `Clean lens` has no dirt. [V]

## 4. Port

* Light sources: up to 4 stars of the current system whose disc touches the screen, brightest first. A
  flare fades linearly in apparent magnitude from `LensFlareFadeBeginMagn` to `EndMagn` (the Sun from Earth
  keeps 80 %).
* Like SE, each flare is scaled by the frame at the sun: after the bloom chain, a probe pass reads the
  frame (limited to BloomDynamicRange) and the bloom at level `clamp(7 - log2(height / radius px), 0, 5)`,
  mixes them by `smoothstep(LensFlareSmallMethodRadius 5, LargeMethodRadius 8, radius px)`, multiplies by
  `smoothstep(3, 10, luma)` and takes `pow(1/2.2)` (`lens_flare.glsl`, FUN_1402698e0, FUN_1404e6240), so
  bodies, terrain, clouds, rings and ships over the sun hide the flare and the atmosphere tints it. [V]
* Sprites are placed on the CPU each frame and drawn after the probe, before the bloom combine, so they are
  not bloomed. Ghosts get 3x more (their atlas tiles peak at 0.13) and lose brightness with area past 0.1
  half-heights, so the big faint ghosts of a preset stay washes as in SE. [P]
* Pseudo flare: SE's FLARES code in the bloom combine; `LensRaysMatr` is not decoded, the starburst turns
  with the camera as in Chapman's original. The lens colour gradient is not used (SE's shader does not). [P]
* The Graphics window (Settings > Graphics) toggles the three layers and picks the presets; the choice is
  saved in `settings.cfg`.
