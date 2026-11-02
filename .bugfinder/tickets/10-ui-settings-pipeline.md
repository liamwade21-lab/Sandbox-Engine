# 10 Interface, settings and the render pipeline / screenshots

## Paths in scope
- crates/se_engine/src/hud.rs (info panel, already had two fixed bugs), overlay.rs (1.7k lines: labels,
  markers, orbits), browser.rs, sysbrowser.rs
- crates/se_engine/src/settings.rs, settings_window.rs, config.rs-like saving, log_file.rs, main.rs,
  memory.rs, splash.rs
- crates/se_engine/src/graphics.rs, display.rs, render_scale.rs, bloom.rs, exposure.rs,
  color_correction.rs, lensflare.rs, capture.rs, screenshot.rs
- shaders only where data layout or pipeline setup matters: se_bloom, auto_exposure, color_correction,
  upscale, lens_flare*

## What it does
egui interface (info panel, object labels and markers, star/system browsers), the settings window and
settings.cfg in %APPDATA%\SpaceEngine-rs (saved and loaded each run), the log file, the post-process
chain (render scale/upscale, bloom, auto exposure, colour correction, lens flare), window/display
modes and screenshots/video capture.

## Likely bug classes
- Settings: a bad or old settings.cfg → panic or silently reset everything (data loss), partial
  writes on crash, values not clamped (render scale 0 → zero-size texture → wgpu panic), a setting
  changed in the window not applied, or applied but not saved.
- Input through UI: clicks/scroll/keys on egui windows also reaching the 3D view (one fixed already
  for the info panel; check every other window and the browsers), hotkeys firing while typing.
- Counts/lists showing the wrong kinds of object (barycentres etc.), stale lists after the system changes.
- Render pipeline: resizing/minimising (zero-size window) → panic; textures recreated every frame
  (leak); exposure NaN that never recovers; screenshots with wrong size/format/colour space,
  file name clashes overwriting old screenshots.
- Ordering: post-process reading last frame's values.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. Port runs can take screenshots with `shot FILE`. The user's own
settings.cfg is hidden by the sandbox; the port writes one per run in %APPDATA%\SpaceEngine-rs, so read
the code for its location and don't touch the user's file.
