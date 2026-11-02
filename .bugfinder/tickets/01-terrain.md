# 01 Terrain streaming and ground rendering

## Paths in scope
- crates/se_engine/src/terrain.rs (2.8k lines, the main suspect: quadtree LOD, patch build, height-grid cache)
- crates/se_engine/src/terrain_gen.rs (GPU height generation, readback)
- crates/se_engine/src/surface_tex.rs (surface texture tiles, loading/caching)
- crates/se_core/src/planet_tiles.rs, crates/se_core/src/tg.rs
- crates/se_engine/assets/shaders/tg_height.wesl, planet.wesl (only where they decide heights, LOD,
  texture coordinates, precision)

## What it does
Splits each planet into a cube-sphere quadtree of terrain patches that refine as the camera gets
near. Heights come from a GPU compute shader (tg_height.wesl) read back asynchronously, cached per
planet; patches are meshed on worker tasks and drawn with planet.wesl. Surface texture tiles from
SE's install are streamed in and cached. Camera near/far planes are set from altitude.

## Likely bug classes
- Cache eviction dropping data still needed (see the fixed 10 s grid bug), or caches/GPU buffers
  that never shrink (leaks across many planets/landings).
- Async readback: stale results applied to the wrong patch/planet after switching planet, results
  for despawned patches, generation counters, a pending state treated as failure.
- System ordering within a frame (camera, LOD selection, near plane, transforms).
- Seams/cracks between LOD levels, wrong neighbour lookup at cube-face edges, index math on the
  edges of a face (off-by-one, out of bounds).
- f32 precision at planet radii (camera-relative origin), NaN on poles or face corners.
- Despawn/respawn when leaving a planet and coming back; two planets near each other.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. Port runs: `surface LAT LON ALT_KM PITCH`, `agl M`, `select NAME`.
Wait ~900 frames before judging the ground.
