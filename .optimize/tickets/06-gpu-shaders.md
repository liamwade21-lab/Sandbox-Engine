# 06 GPU: volumetric nebulae and the other expensive shaders

**Why:** the user's third complaint. Inside a volumetric nebula (scene `nebula`, inside the
Skylias Stellar Nursery) the release build takes 16.7 ms a frame while the CPU work is only ~6 ms
(trace `neb_still`), so it is GPU-bound. The timed passes show main_transparent_pass_3d 5.25 ms
GPU; the rest of the GPU time is in passes without timestamps (the port's own bloom, colour
correction, render-scale upscale, egui...). Tarantula interior: 5.3 ms frames, transparent pass
4.5 ms. Earth surface: main_opaque_pass_3d 4.0 ms GPU.

**Questions:** where does the nebula's GPU time go (raymarch step count, texture fetches per step,
per-step work that is constant along the ray, overdraw from several volumes, full-screen passes at
full resolution)? Which passes run every frame at full resolution, and do any run when their effect
is off? Shader changes must give **the same output**: hoisting loop-invariant work, removing dead
code, skipping work whose result is multiplied by zero, early-outs that provably give the same
pixel. Anything that changes step counts, precision or sampling is "needs your call". To time a
pass, Bevy's `RenderContext::diagnostic_recorder()` time spans show up in the perf log (`perf.rs`
lists every `render/*/elapsed_gpu`); try that in a worktree copy only.

**Paths:** `crates/se_engine/assets/shaders/galaxy_nebula_raymarch.wesl`, `planet.wesl`,
`sky.wesl`, `atmosphere_common.wesl`, `clouds.wesl`, `se_bloom.wesl`, `color_correction.wesl`,
`upscale.wesl`, `stars.wesl`; `crates/se_engine/src/nebula_raymarch.rs`, `bloom.rs`,
`render_scale.rs`, `color_correction.rs`, `exposure.rs`, `graphics.rs`. Shaders load at run time from
`crates/se_engine/assets/shaders` (the exe finds them through its build path, or an `assets` folder
beside the exe), so a shader experiment needs no rebuild: copy the release exe and an `assets` folder
with your edited shader into your scratch folder and run that exe with `PORT_BIN`.

**Bench:** `bash .optimize/bench/all.sh OUT nebula tarantula earth`.
