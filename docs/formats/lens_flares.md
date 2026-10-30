# Lens flares (`textures/flares/Flares.pak`)

Port: `se_core::lens_flare` (parser), `se_engine/src/lensflare.rs` (renderer), shaders `lens_flare.wesl`
and `pseudo_flare.wesl`. Tags: [V] verified in the exe, [I] inferred, [P] port choice.

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
screen-space lens flare (ghosts and a halo mirrored through the centre from the bright parts of the frame,
chromatic distortion, the lens colour gradient, lens dirt and a starburst). The exe binds
`BloomTex, FrameTex, GhostTexAtlas` for it.

## 4. Port

* Light sources: up to 4 stars of the current system whose disc touches the screen, brightest first. A
  flare fades linearly in apparent magnitude from `LensFlareFadeBeginMagn` to `EndMagn` (the Sun from Earth
  keeps 80 %) and with the part of the disc hidden by the system's bodies (`terrain::disc_overlap`; on low
  ground the body's horizon plane). Terrain, ships and rings do not occlude. [P]
* SE scales each flare by the frame's brightness at the sun (`smoothstep(3, 10, luma)` of the bloom or
  frame there): the port does not read the frame, but takes that brightness as the photosphere's (about 2)
  plus the sun's glare sprite spread over the disc, so the flare is whole while the disc is within the
  glare (`StarSettings::max_radius_px`) and gone once it is about 2.8 times wider (`disc_fade`). [P]
* Sprites are placed on the CPU each frame and drawn in screen space over everything (reverse-Z depth 1,
  additive, depth bias 1e16). `FLARE_GAIN` 8 matches SE's saturated core; ghosts get 3x more (their atlas
  tiles peak at 0.13) and lose brightness with area past 0.1 half-heights, so the big faint ghosts of a
  preset stay washes as in SE. [P]
* Pseudo flare: ghosts, halo, chromatic distortion and the lens colour; no dirt or starburst textures
  (the fullscreen pass binds only the frame). [P]
* The Graphics window (Settings > Graphics) toggles the three layers and picks the presets; the choice is
  saved in `settings.cfg`.
