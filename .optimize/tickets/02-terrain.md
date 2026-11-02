# 02 Terrain: per-frame work and hitches on planet surfaces

**Why:** on an RS planet's ground looking around (`rsground`: 2 m up, turning 0.9 deg a frame) the
release build averages 14.6 ms with frames up to 47-77 ms (1% low 34-47 fps); at 300 m
(`rsplanet`) 11.7 ms, max 33 ms. Earth `earth_look` 15 ms.

**Profile (trace):** terrain::update_terrain 1.12 ms/frame (rsg_look), 1.64 (earth_look), 1.41
(rsg_still); update_node_transforms 0.16, update_eclipses 0.27 (earth), keep_camera_above_ground
0.06; Bevy allocate_and_free_meshes spikes to 15.7 ms (mesh uploads). Look for what makes the spikes
when new patches come into view (mesh building on the main thread? height-grid readback waits?
texture uploads? many patches finishing in one frame?).

**Questions:** what does update_terrain do every frame even when nothing changes (walking every
node, sorting, hashing, allocating Vecs)? Is patch mesh building / normal computation / skirt
building on the main thread? Are GPU height-grid readbacks (terrain_gen.rs) waited on
synchronously? Per-frame allocations in node selection? Per-patch uniforms rewritten each frame?
Spreading finished patches over frames changes the frames during loading, so that is "needs your
call".

**Paths:** `crates/se_engine/src/terrain.rs`, `terrain_gen.rs`, `surface_tex.rs`,
`crates/se_engine/assets/shaders/tg_height.wesl`, `crates/se_core/src/surface_gen.rs`,
`proc_surface.rs`, `biomes.rs`, `climate.rs` (only what terrain calls per patch).

**Bench:** `bash .optimize/bench/all.sh OUT rsground rsplanet earth`.
