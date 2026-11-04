# SpaceEngine Rust/Bevy port, version 2

**This folder is version 2** (`C:\Users\liamw\Desktop\SpaceEngine Rust Port Version 2`), copied on
2026-11-02 from version 1 (`C:\Users\liamw\Desktop\Space Engine Recomp`), which stays as it is.
Version 2 is where better visuals, add-ons and the like go. Work here, not in the version 1
folder. The test scripts (`.audit/tools`, `.optimize/bench`) point at this folder; the older
briefs and reports in `.audit`, `.bugfinder` and `.optimize` still name the version 1 folder, as
history. Version 2 keeps the user's settings, log, saved session, places and ships in its own
`%APPDATA%\SpaceEngine-rs-v2` folder (`settings.rs` `USER_DIR`; the atmosphere cache too). On its
first run it copied version 1's `%APPDATA%\SpaceEngine-rs` there (done 2026-11-02, by a test run);
it never writes version 1's folder. Screenshots still go to the shared `Pictures\SpaceEngine-rs`.
Test runs write `se-test.log` there (since 2026-11-04; the user's own `se.log` is kept for looking
into what they saw), and skip the settings and session.

A private rebuild of SpaceEngine 0.991 in Rust (Bevy 0.20), drawn with Vulkan. It reads the
game's own files from the SpaceEngine install at run time; nothing from the game is copied here.
README.md explains the features, controls and command-line options; `docs/progress-archive.md` holds
finished work's detailed notes moved out of PROGRESS.md; `docs/` holds the notes on
SE's file formats and reverse engineering; `docs/missing_features.md` lists what still differs
from SE. `.bugfinder/REPORT.md` is the 2026-11-01 bug hunt: what was found, fixed and left.
`docs/ksp-clouds-notes.md` is how blackrack's volumetric clouds for Kerbal Space Program work (studied
for the clouds plan); what it was learned from (his public C#, the user's download's shaders exported
as GLSL, his wiki) is in `re/ksp_clouds/`, which stays out of git: someone else's code. Take methods
from it, never its code, shaders or textures.

## Build and play

- Build: `cargo build --release` (always release). The user plays from
  `target\release\spaceengine-rs.exe`, so rebuild release after every change.
- Tests: `cargo test -p se_core` and `cargo test -p se_engine --release`.
- SpaceEngine install (read only): `D:\SteamLibrary\steamapps\common\SpaceEngine`.
- No .bat launchers yet.

## Rules from the user

- Vulkan only. VR and SE's editors are out of scope.
- Game assets are read from the install, never copied into the project.
- Ask before taking over the screen to run SpaceEngine. When it is allowed, a Sonnet agent
  (high effort) drives SE and takes the screenshots.
- No Opus high-effort subagents, except in the /bugfinder and /optimize skills, which may use
  any agents they want.
- Commit only when asked; never push. Commit messages end with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Keep each file's line endings (many files are CRLF): don't use `sed -i` on them.
- Don't defeat DRM; don't delete data without asking.

## Gotchas

- Shaders are loaded at run time from `crates/se_engine/assets/shaders` (no rebuild needed to
  try a shader change), but an edited shader takes a few hundred frames to compile in a test
  run: the ground is missing until then, so wait ~900 frames before judging a screenshot.
- Judging screenshots (the user's rule, after Claude fooled itself twice on 2026-11-03):
  - Make sure a screenshot is from THIS run: the port exits a few frames after requesting it
    once the file exists, so an old file with the same name used to stay (runport.sh now deletes
    it first; still check the run's log says "Screenshot saved" and the file's time).
  - Let the game load fully first. After any shader edit (planet.wesl above all) the edited
    pipelines compile for a long time and the ground or clouds are missing meanwhile (a milky or
    hazy picture with no ground is that, not a lighting bug): wait 2400+ frames, and compare with
    a run of the unedited shader before blaming the change.
- Test runs: `bash .audit/tools/runport.sh OUT.png FRAMES "cmds"` (one run at a time). Useful
  dev commands: `select NAME`, `surface LAT LON ALT_KM PITCH`, `agl M` (hold M metres above
  the ground), `fly KMS`, `spin DEG` (turn in place), `orbit DEG` (swing round the selection, as the
  right mouse button does; degrees a second), `photo 0|1|2` (photo mode: 2 is auto exposure), `ambient X` (SE's
  ambient light), `shot FILE`, `wait N`, `date`, `pause`. A 1x1 screenshot
  (72 bytes) means the window was minimised at that moment: take each picture twice.
- The user's own `se.log` and `settings.cfg` (in `%APPDATA%\SpaceEngine-rs-v2`) can be hidden
  from Claude's shell by the desktop app's sandbox (they were readable on 2026-11-02); if so, copy
  them out with a process started through `Invoke-CimMethod Win32_Process Create`.
- Testing settings save/load (scripted runs skip the settings): run the exe unscripted with
  `APPDATA` set to a scratch folder and `timeout 60` (first run writes `settings.cfg` there at once).
  Edit that file and delete its `Version` line, run again: the port rewrites the file from what it
  loaded. Never point such runs at the user's real folder.
- Volumetric clouds (`volumetric_clouds.rs`, `volumetric_clouds.wesl`, plan in
  `docs/spec-volumetric-clouds-weather.md`): full-screen passes in `Core3d` after the main pass,
  before render_scale's `SolidUpscaleSet`, drawing into the solids' corner of the main texture. They
  read the depth buffer (the main camera's `depth_texture_usages` includes TEXTURE_BINDING). The
  active body's flat cloud shells are hidden only while the passes draw it (`VolumetricClouds::body`,
  read by terrain.rs). Positions go through "sphere space" (body frame / ellipsoid semi-axes) and the
  camera's altitude comes from f64, so ray-sphere maths stays exact near the surface. GPU time shows
  in `SE_PERF` as `volumetric_clouds/elapsed_gpu`. The cloud cover generators (SE's terra clouds, the
  port's procedural ones) live in `cloud_cover.wesl`, shared by the flat shells (`clouds.wesl`) and
  the volumetric cover map: change them there and check the flat clouds stay identical. The passes
  read the depth buffer as true distances, so their body must be in linear space, not scaled space.
  `VolumetricClouds::active` makes camera.rs `update_nearest` stretch the linear radius over it: a
  nearer moon once left the planet compressed and its clouds missing (fixed 2026-11-03). Ground shadows:
  `planet.wesl` `cloud_shadow` reads the cover map, the shadow record and the shadow map
  (`CloudShadowAssets`, bindings 27-31 of `PlanetMaterial`) and the weather cube (32, with the cover's
  sampler). The shadow map (`shadow_gen`, 40 km around the spot under the camera) and the cover's
  shadow farther out must give the same light, or a bright square of ground follows the camera (it did
  three times): both a quarter under thick cloud; near the camera the ground takes the map's shapes at
  the cover shadow's mean darkness (the map over its last mip, 8 texels of 10 km, times the cover's
  shadow: a patchy cloud lets more light through than an even one); both follow the star's ray over
  the round body (`cloud_cover.wesl` `climb_dist`: over flat ground a low sun's ray passed above the
  clouds); both go on past the terminator (cut there, the shadows ended in a line).
- Weather (`se_core/src/weather.rs`, `volumetric_clouds/weather_sim.rs`, `weather_sim.wesl`): the
  derived weather is a pure function of climate, place and date; its maps (two dates 10 simulated
  minutes apart, weather + wind layers) are made on a worker thread, at once on arrival or a date
  jump. A GPU simulation (cube 6 x 128 x 128: storm activity, stable air, anvils) runs inside
  `clouds_pass` before the trace and writes the weather cube the clouds and the ground read. Its rules
  (`SimClock`): live within 20 radii up to x1000; faster, farther or backwards it follows its seed state;
  paused it is frozen; a new body or a date jump seeds it at once. Near the camera the noise drifts with
  the wind (`CloudView` `noise_off`/`detail_off`/`motion`, also used by the reprojection).
  `SE_WEATHER_SIM=0` leaves the simulation's cells out (comparisons); `SE_WEATHER_NEUTRAL=1` turns
  the derived weather off as well (the cover exactly as SE's map or generator makes it). The cover a
  layer gets is `cloud_cover.wesl` `weather_cover` (the weather and the opacity on SE's cut cover:
  `se_cut` once, on raw map or generator values; the cover map is stored already cut), used by
  the march, the rain and the ground's cloud shadows: keep them on it. The weather must keep SE's
  cloud map recognisable: stronger terms once made Earth's storm track a beige overcast from orbit.
  `cargo test -p se_core print_cover_stats -- --ignored --nocapture` prints the weather's effect. Dev commands for weather tests:
  `timing frame` (a fixed 1/60 s a frame, so runs reach the same date whatever their speed), `resume`
  (undo `pause`), `switch KEY on|off` (a render switch such as `ShowRain`, for on/off comparisons).
- The clouds' plan since 2026-11-03 is `docs/spec-clouds-v3.md` (one cloud system, the Flight
  Simulator look, speed first). On High and Ultra the ray-march draws the active body's clouds at
  every height; SE's flat shells only cross-fade with it between 40 and 60 radii (`CloudView::lod`,
  `VolumetricClouds::flat_hidden`; there clouds.wesl reads the weather cube and the `CloudShadow`
  record: key.w the weather's share, sun.w the shells' share, layer_b.w the layer index + 1), and it
  fades in over 1 s once ready. The earlier hand-over (flat shells from orbit down to 15x the cloud
  tops, docs/spec-cloud-lod-and-popping.md) was dropped: while the flat shells drew the clouds, the
  trace's rain shafts showed through them as dark patches, and it saved nothing near the ground.
  Whatever lies under the clouds (rain, fog, dust) must stay in the same march as the clouds, so they
  hide it. `SE_CLOUD_LOD=0` is the old hard switch without fades (comparisons).
  The cover map is 2048 a face (`COVER_SIZE`; catalog maps composed from SE's level-4 tiles, a thread
  a face). Each layer gets a stack of detail cover patches (`volumetric_clouds/detail.rs`,
  `DETAIL_LEVELS` 3, 2048 texels, Earth 800 / 1,600 / 3,200 km across, in the layer's frame, each made
  below 3.5x its half-width and re-centred after a quarter of it; catalog maps from their deepest
  tiles on a worker thread, generated layers by `cover_patch` on the GPU, one a frame); `cover_at`
  takes the finest that holds a point at its footprint (`CloudView::detail`, binding 19). The near
  pass traces the first 4 km of the rays (Ultra; High 2 km), at half the solids' size, a quarter of its texels a frame, with its own
  history, over the far trace (`CloudSize::params.w` the ray part, `CloudView::lod` z/w). The far
  trace does one texel of each 2x2 block a frame in turn (`CloudSize::trace`, `SE_CLOUD_QUARTER=0`
  all; the resolve reprojects each texel by the trace's distance, which must be the real scene
  distance for a clear texel: capped at 10,000 km, orbiting a planet turned its clouds into blocky
  ghosts, and a texel whose ray misses the cloud shell is always clear); the march takes long steps through clear air (`maybe_cloud`: never step back over what was
  sampled, that once hid whole decks) and the light from 20-40 km on comes from the cloud above
  (`light_column`, the area's mean: a low sun crosses the gaps). The composite sharpens far clouds to
  full resolution by the cover (`cover_tau`, the resolve's second target). The weather's own cloud is
  shaped by a baked pattern map (`weather_pattern`, `CloudShadowAssets::pattern`, binding 27; planet
  material binding 33) through `cloud_cover.wesl` `weather_shaped`: the march, the rain and the
  ground's shadows all use it (the flat shells not yet). `SE_DETAIL_COVER=0`, `SE_NEAR_PASS=0` leave
  parts out; `SE_CLOUD_REFERENCE=1` draws the reference to judge shapes and light against (every pixel
  at full size traced every frame over the whole ray, twice the steps, no history: slow); the dev command `vclouds 0|1|2` switches the 3D clouds off/High/Ultra within a run (flat
  vs 3D pictures; note the flat shells then draw without the weather). The march (`trace`) is nearly
  all of the clouds' cost; the worst case
  so far is the user's planet RS 8514-3620-1-7-69 1 (one layer, then a high veil at 14-18 km, and the weather's cloud): 11-16 ms
  at Ultra 3440x1440 before the plan, 1.4-2.7 ms after its step 4 (`.optimize/bench/rs8514_ultra.sh NAME "extra commands"`; planet 2's
  thin high layers ~5.5 ms looking around). Since 2026-11-04 that layer is cumulus heaps at 10.7-17 km, 1.2-1.8 ms.
- Layer heights (`layer_volume`) count from the ground's middle (`Terrain::ground_mid_m`: the middle of
  the relief, never below the sea; SE's generated layers stand on it). Every body's lowest layer is the
  lower atmosphere's cumulus, as Earth's (at most 9 km deep; an opaque map like Venus's stays a sheet).
  Once only a lowest layer under 15 km above the radius was cumulus: dry worlds with 20 km of relief
  (the ground's middle 10 km up) then got veils alone (the user: "procedural planets are missing the
  detail"). Earth's middle is 0.
- The far trace's steps grow with the distance from the camera (`step_rate`: q t, q bisected so the
  steps cover the ray's pieces inside the cloud bands, at least 0.004 t), and a sample's footprint is at
  least a quarter of its own step. The march goes through the cloud bands alone (`ray_piece`,
  `CloudView::bands` from `cloud_bands`: the layers' heights merged where they overlap), skipping the
  gaps between layers, each piece from its own jittered start. With the whole shell from the lowest
  layer to the highest as its path, a moon's heaps under veils 70 km higher got km-long steps: rings
  round the spot under the camera, heaps that appeared only lower down. Cloud density must stay inside
  each layer's bottom..top (the march never samples outside the bands). With even steps (path / MAX_STEPS) a ray skimming a layer from inside it (150-300 km) had km-long
  steps next to the camera: the heaps there became the layer's mean, a flat band over the far heaps'
  tops that zooming in magnified (the user's 2026-11-04 video, a 10 degree view). The near pass keeps
  even steps.
- Night light: the clouds take the further lights as the ground does (`BodyLighting::extra`: other
  stars and planet shine; `CloudView::extra_dir` / `extra_light`, `night_light` in the shader, no light
  march of their own; their haze too) and SE's `AmbientLighting` (`sun_light.w`; the flat shells take
  it as well). The sun's light march is skipped where the sun has set (`tw_l` 0). Without these the
  night side's clouds were black over a moonlit ground. Test with `photo 2` (auto exposure, as the user
  plays) and Earth under a full moon (2026-11-24 14:53, 20 N 137 E is midnight under it).
- Lightning (`weather_sim.wesl` `sim_flash`): 6 of the 8 slots look for storms only around the camera
  (300 km or its height), 2 anywhere, each the most active storm of its 32 tries by the storms' own
  activity (the fx cube's B, written by `sim_out`), not the weather's convection (low at night even in
  a storm). The derived weather has convection at night (se_core weather.rs: half the afternoon's at
  midnight). Before, no flash ever went near a camera on the night side.
- Cloud shapes (the clouds plan's step 5): cumuliform layers are fields of heaps (`layer_density`,
  `heaps`): the shape noise, spread evenly over 0..1 by a logistic fitted to its distribution (about
  0.833 +- 0.041: se_core `cargo test -p se_core print_shape_quantiles -- --ignored --nocapture`; change
  the shape volume and refit it), read on the layer's base in two layouts (one turned: the 6 km tile
  alone lined the gaps up in rows), picks where heaps stand and each point's strength; the types blend
  their tops (`type_top`, `type_height`, a heap's height from its width) by weight, never their
  densities (that stood faint 8 km towers over every heap). Keep `heaps_ceiling` (the clear-air test's
  limit) and the mean curves (`type_density`, the far mean and the light from above) in step with
  `heaps`. Cloud types come from the regional cover (`regional_cover`, the cover map's mip 2), not the
  local one: SE's maps give each cumulus blob a cover near 1. The weather's own cloud (`weather_shaped`)
  must fade out with its share, never switch on at a level: a switch at 2 % cut its clouds into
  saw-tooth walls along the weather cube's texels.
- Cloud light (step 6): the light volume (`LightVolume` in volumetric_clouds.rs, `light_gen` and
  `light_at` in the shader, bindings 28-30 of the trace's group): 160 x 160 x 32 over the layers within
  60 km of the spot under the camera, two of them, the back one built 4 slices a frame, swapped when
  complete; only while the camera is within 30 km of the clouds' top. The march takes its first three
  light steps itself, the rest from the volume. The march (`trace`) is compiled twice (`HEAPS` set or
  not, `CloudPipelines::trace`): code compiled into it costs registers even where it never runs (the
  solid types' code slowed a body of veils alone by a third), so measure the user's planet
  (`.optimize/bench/rs8514_ultra.sh`, heaps) and a body of veils alone after adding to the march.
- Weather effects (step 7): the trace draws rain/snow shafts, fog and dust (`media`) and lightning's
  glow in clouds; `weather_fx` (full resolution, after the composite) draws near rain streaks or snow
  flakes and lightning's light on the ground. A second cube holds fog and dust; `sim_flash` picks 8
  flash slots into a storage buffer. The effects' clock (`Drift::fx`) runs at most at real speed and
  stops when paused. Bodies with an atmosphere but no cloud layers also get the passes (fog, dust).
  `SE_WEATHER_DEBUG=1` draws a weather map at the top of the screen (red rain, green convection, blue
  fog, orange dust, cyan flashes, white dot the camera): use it to find test spots before taking
  pictures. Flashes last about 0.3 s, so a single screenshot rarely catches one.
- SE's gas giant cloud generator lives in `gas_giant.wesl` (noise primitives, `gg_height`,
  `gas_giant_cloud`), shared by `planet.wesl`, `brown_dwarf.wesl` and the volumetric giant decks.
  Change it there and check flat Jupiter stays identical (it was pixel-checked when split out).
- Gas giants' volumetric decks (`giant_specs`, `giant_sample`; on by default, `SE_GIANT_DECKS=0` off)
  fade in within 6 scale heights of the cloud tops and must match SE's flat bands there: a deck point's
  light counts the cloud above it from the main deck's own top there (`giant_sample` puts that column in
  the sample's `wx.x`), never the deck's mean top (that made belts five times too dark); they take the
  atmosphere's haze down to the radius (the tables' ground) like the ground; a catalog giant's colours
  come from its own colour tiles (`compose_face` with a tint, cover kind 5 with `kind.y` 1), else SE's
  generator. The rings shade every body's clouds (`ring_shadow`, in the body frame: in sphere space a
  flattened giant's shadow fell in the wrong places) and SE's rings winter fades a giant's deck. The
  camera may go down to 10 scale heights under a giant's radius while the passes work on it
  (`giant_floor_m`, terrain.rs `keep_camera_above_ground`); below the radius terrain.rs hides the
  giant's flat bands (seen from below, their nodes' skirts were a grid of lines). A hot giant's decks
  glow as its bands do (`deck_glow`, `CloudView::glow` from terrain.rs `thermal_uniform`; hotter with
  depth below the main deck's top, `giant_sample` puts that depth in `wx.y`), and a glowing atmosphere's
  light is in the decks' haze (`glow_atmo`). Giant lightning: `sim_flash` with
  `flash.w` 2 (`giant_storm` cells, flashes about the main deck's base). Compare with `SE_GIANT_DECKS=0`
  and `photo 0` (fixed exposure); find a giant's day side first (look down from 3,000 km at four
  longitudes).
- The clouds' weather makes the user's planet 2 spot (25.46 N, -48.19) clear: it lies in a dry
  subtropical belt. Cloudy test spots change with the date; check orbit pictures first to find one.
- Reaching the user's RSN planets by dev commands (`.optimize/bench/rsn_ultra.sh`) needs the
  `--size 3440x1440` window: at the default size the nebula's star was "not found".
- Python on Windows: `read_text`/`write_text` turn LF files into CRLF. Patch files with
  `read_bytes`/`write_bytes` (and keep each file's own line endings).
- wgpu's `Operations::default()` CLEARS the attachment: a pass that blends over the scene must use
  `LoadOp::Load`.
- Reverse engineering: Ghidra project in `re/ghidra_projects/se`; decompile with
  `python -I .audit/tools/dec.py FUN_x`; SE's decrypted shaders are in `re/se_shaders`.
- Flattened planets (Earth, Saturn, fast-spinning giants) are drawn on an ellipsoid. Any shader
  maths that uses a sphere (atmosphere tables, camera height) must undo the ellipsoid per axis
  first, as `planet.wesl` `table_local` and the sky shell do. Dividing by one radius left
  flattened giants mostly flat blue.
- Reaching a star in another galaxy with dev commands: `select RG …` twice, `go <km inside it>`,
  `wait 600`, then `rs RS …`. The planet names then work with `select`. A nebula's star (`RSN …`):
  from inside the galaxy (`go 30000000000000000`), `goto RN …` twice with `wait 1500`, then
  `goto RSN …` (put a harmless command such as `spin 0` before it after a long wait: the name
  resolver gives up after 3000 frames of a name waiting at the front of the queue).
- Test runs skip the user's settings: they use the High preset (solids at 85% of 1080 lines, no
  MSAA) and a 1920x1080 window. The user plays Ultra (4x MSAA, full resolution) maximised on a
  3440x1440 screen (RTX 5080), which costs several times more GPU time. Measure what they see with
  the `preset Ultra` dev command and `--size 3440x1440` (`.optimize/bench/rsn_ultra.sh`).
- Ground shader: on procedural worlds' fine nodes the colour generator's kilometre-scale noises are
  worked out per vertex (`planet.wesl` `SELENA_AT_VERTICES`, `TERRA_AT_VERTICES`,
  `TERRA_CLIMATE_AT_VERTICES`, chosen per node by terrain.rs `vertex_noise`), coarser nodes per
  pixel; both paths must keep calling the same `selena_noise` / `terra_noise` /
  `terra_climate_noise`. A new noise term with features finer than kilometres stays per pixel (or
  gets its own frequency in `vertex_noise`). Terra nodes too coarse for all of `terra_noise` split it
  (`TERRA_PARTIAL`, since 2026-11-04): the vertices carry its warp and each sum's coarse octaves
  (`vertex_octaves`, passed to the pixels as a flat value so the two stages never disagree), the
  pixels add the finer ones (`ter_fbm_from`, the same points and amplitudes as `ter_fbm`), and a pixel
  that keeps fewer octaves than the vertices works the whole sum out itself. `from` is a reserved
  word in WGSL (a shader using it fails to compile and the ground vanishes).
- Ground node skirts (the walls hung under each node's edges that close the cracks to coarser
  neighbours) are a mesh of their own, a child entity of the node, with the node's material cloned
  with `skirt: true`: `PlanetMaterial::alpha_mode` puts it in Bevy's alpha-mask phase, drawn after all
  the opaque ground in the same pass, so hidden skirt pixels fail the depth test before they are
  shaded. Bevy orders opaque draws by pipeline and material, not by distance: drawn with their node,
  the skirts were shaded wherever they came before the neighbour hiding them, a third of the ground's
  GPU time near the surface (2026-11-04). Keep them apart, and never `discard` in the ground's
  `fragment` (it would turn the early depth test off).
- What the ground costs (2026-11-04, Ultra 3440x1440, `main_opaque_pass_3d`): found by switching one
  part of planet.wesl off at a time in a copy of the exe with its own `assets/shaders` beside it (the
  exe reads those first; run the benches with `PORT_BIN=` that copy, so the repo's shaders stay as
  they are). The pixel shader alone is nearly all of it (a flat colour: 2.5-3 ms is raster, vertices
  and MSAA). Near the ground on the user's planet: the soils (within 2 km) ~7 ms, the sample layers
  3-4, the haze 1.7, cloud shadows 1.6, the normal map's jitter 0.3, sky light 0. From altitude SE's
  colour noise per pixel on coarse nodes is about half (the moon 5.1: the colour generator is ~80% of
  the ground from 13 to 350 km). Code that never runs cost nothing measurable (a far-only shader
  without the soils, a terra-only one: no gain). Probes that change the colour also change which
  soils the ground picks, which moves the soils' cost: read them with care.
- Measuring speed: `SE_PERF=FILE` turns on the frame-time logger (`perf.rs`); the dev command
  `perf NAME` … `perf end` marks a segment. The scripts in `.optimize/bench/` (`all.sh` scenes,
  `refs.sh` reference shots, `compare.py`) do this for fixed scenes; `.optimize/REPORT.md` has the
  2026-11-02 optimization results. A segment whose median sits at ~16.6 ms was capped at 60 fps from
  outside the port (window not in focus): rerun it with the window in front. A segment with passes'
  GPU times missing or tiny (seen once with the clouds off) is broken the same way: rerun it. The
  clouds' parts show as `volumetric_clouds/sim`, `shadow`, `trace`, `resolve`, `near`, `fx`.
- Never change a Bevy material asset every frame (`Assets::get_mut`, `iter_mut`): Bevy then
  re-prepares every object using it. Per-frame values go in a storage buffer written in place: the
  star/galaxy sprites' camera values (`stars.rs` `SPRITE_CAMERA`) and terrain lighting (`terrain.rs`
  light pages). Write material assets only when a value really changes.
- After editing a shader, an older exe reading the new shaders can fail (bindings differ): rebuild.
- The catalog counts its changes (`Catalog::revision()`, and `edits()` for records changed in
  place); the nearest-star cache and the black-hole list rely on them. Change the catalog only
  through its own methods or `SeData::catalog_mut` / `catalog_get_mut`, and bump `edits` when code
  edits an existing record in place.
