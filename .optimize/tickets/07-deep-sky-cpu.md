# 07 Deep sky, per-frame CPU: star fields, galaxies, nebulae, clusters

**Why:** these systems run every frame in every scene, even with the camera still, and the star
field streams nodes while flying (the user's second complaint).

**Profile (trace, ms/frame):** procstars::stream_nodes 0.99 (rsg_still, camera still), 0.68
(rsg_look), 0.84 (fly_1e15); nebula_raymarch::update_volumes 0.73 (rsg_still), 0.63 (rsg_look);
galaxies::update_galaxies 0.27-0.34; procgalaxies::promote_near 0.17-0.23 (max 1.4);
ellipticals::update_volumes 0.14-0.2; nebulae::update_nebulae 0.12-0.14;
clusters::update_clusters 0.08-0.13; procstars::find_nearest 0.12-0.35 (max 14-16 ms spikes).

**Questions:** what do they recompute every frame when the camera hasn't moved (walking every
octree node, sorting all volumes, rebuilding Vecs/HashMaps, rewriting instance buffers or
materials)? Could they skip work when the camera/zoom/time inputs are unchanged, or keep a sorted
list incrementally, without changing any output? What causes find_nearest's 14-16 ms spikes? While
flying, is node building on the main thread?

**Paths:** `crates/se_engine/src/procstars.rs`, `stars.rs`, `nebula_raymarch.rs` (update_volumes
:241), `nebulae.rs` (update_nebulae :543), `galaxies.rs` (update_galaxies :1263), `procgalaxies.rs`
(stream_nodes :271, promote_near :411), `ellipticals.rs`, `clusters.rs`, `constellations.rs`,
`crates/se_core/src/star_octree.rs`, `galaxy_model.rs`, `proc_galaxy.rs` (only the per-frame paths).

**Bench:** `bash .optimize/bench/all.sh OUT rsground galaxy fly nebula`.
