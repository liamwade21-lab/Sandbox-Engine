# Render resolution, upscaling and sharpening

Files: `crates/se_engine/src/render_scale.rs` + `assets/shaders/upscale.wesl` (solids' resolution and
upscaling), `crates/se_engine/src/color_correction.rs` + `assets/shaders/color_correction.wesl` (final
sharpening). SE sources: `re/se_shaders/upscale_bicubic.glsl`, `bicubic_common.glh`, `fxaa.glsl`,
`hdr_final.glsl`; FUN_1401910f0 (sizes), FUN_1401933a0 (TextureRendering keys), FUN_14018dae0 (auto sharp).

## Frame order (main camera)

| step | pass | texture |
|---|---|---|
| 1 | Bevy main pass with `MainPassResolutionOverride` | top-left `solid_size` corner of the main texture |
| 2 | `solid_upscale` (Core3d `EarlyPostProcess`, before projection, black hole lens, bloom, tone mapping, FXAA/SMAA): `SolidFXAA` fxaa.glsl, or a texture copy, of the corner | low-resolution scratch |
| 3 | `upscale` (bicubic_common.glh `textureUpscaled`) | whole main texture, same index (no flip) |
| 4 | projection, black hole lens, warp, bloom, exposure, Bevy tonemapping (exposure only) | full resolution |
| 5 | colour correction: `fragment` (hdr_tone) into its scratch, `final_image` (hdr_final: `textureFastCAS` at `Sharpness`, colour correction, dithering) back | full resolution, no flip |
| 6 | Bevy FXAA / SMAA (preset `FXAA`), upscaling to the swapchain, egui (HUD, labels, windows) | full resolution |

## Sizes

`solid_size(w, h, SolidTargetRes, SolidResolution)` (FUN_1401910f0): if the shorter side exceeds
`SolidTargetRes`, the view is scaled to make it that long; then both sides times `SolidResolution`,
rounded down to even. 1600 x 900 at High (0.85, 1080) gives 1360 x 764; 2560 x 1440 at main-def's
0.5 gives 960 x 540. Equal to the view (Ultra on a <= 1440-line screen) means no override.

## Modes

`SolidUpscaleMode` = 3 * METHOD + SHARP: METHOD 0 nearest, 1 bilinear, 2 bicubic (B-spline, SPLINE 0),
3 lanczos (polynomial approximation, SE's weights kept as written); SHARP 0 none, 1 the 4-diagonal
sharpen, 2 FidelityFX CAS (fast variant, around a Reinhard LDR mapping with `br` 0.05). The sharpness is
`SolidSharpness`, or 1 - `PostEffects` `Sharpness` with `SolidAutoSharp` (default: 0, so the upscale's
CAS uses its mildest weight and the final pass sharpens at 1). Presets: Potato/Low bilinear,
Medium lanczos, High/Ultra bicubic CAS; `SolidFXAA` on for High/Ultra.

## Differences from SE

- SE draws only solid objects at this resolution; the port's whole main pass is (stars and sprites
  keep their pixel sizes through `view.viewport`, but are resampled).
- Off while the warp pass is on (it reads the depth buffer at full size) and in the cube-map
  projections.
- fxaa.glsl's `SampleLum` reads raw HDR while `Sample` applies the Reinhard mapping; the port maps both
  (exposure 1). SE's FXAA parameters are not traced; FXAA 3.11's defaults (0.75, 0.166, 0.0833) are used.
- Not ported: checkerboard modes 12, 13 (drawn as bicubic CAS), `SolidTemporalUpscale`
  (combine_temporal.glsl), the volumetrics' `VolumResolution` / `VolumUpscaleMode` / `VolumBilateral`.

Test switches: `SE_SOLID_RES=<0.1..1>`, `SE_SHARPNESS=<0..1>`.
