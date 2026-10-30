# Ticket 09: Settings, graphics and post-processing

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `09`; write `.audit/gaps/09.txt`; scratch files go in `.audit/work/09/`.

## Scope
The settings windows and what each graphics option does: main-def.cfg `Display` 47-62,
`RenderingQuality` 62-76, `DepthBuffer` 76-85, `PostEffects` 196-240, global parts of `Lighting` 240-297,
`TextureRendering` (global options), `Projection` 351-387, `Export` 453-475 (screenshots, video via
`config/ffmpeg.cfg`), `Loader` 475-501, `Memory` 6-26, `Log` 501-525, `config/preset-*.cfg`; shaders
`hdr_*.glsl` (HDR pipeline, auto exposure with `hdr_histogram.glsl`, bloom/glow `hdr_gauss_blur.glsl`,
tone mapping operators `hdrt_*_tone.glh`), `fxaa.glsl`, `upscale_*.glsl`, `combine_temporal.glsl`,
`bilateral.glsl`, `proj_cylinder.glsl`, `proj_fisheye.glsl`, `3D_anaglyph.glsl`, `3D_interlace.glsl`
(stereo, not VR), `pbr_*_cubemap.glsl`, `compress.glsl`. Settings persistence, photo mode, screenshot
formats and resolution multipliers, the cubemap/panorama export.

## Notes and boundaries
Out of this ticket: star brightness settings (01), planet/atmosphere quality knobs' visual result (04/05),
UI windows other than settings (08).

## Port starting points
`crates/se_engine/src/{graphics,settings,config,main,camera,lensflare}.rs`, `config/preset-*.cfg` handling in `graphics.rs`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
