# 03 Flying fast through stars: picking and building star systems

**Why:** the user's second complaint. Scene `fly` (from Earth toward Sgr A*, `fly 1e15` km/s =
0.16 pc a frame): release mean 12.1 ms, median 8.9, p99 39 ms, max 54 ms, 1% low 22 fps
(`fly 1e14`: 8.3 ms, 1% low 53 fps).

**Profile (trace, fly_1e15, ms/frame):** system::select_active_system **9.49 avg, 46 max**,
system::spawn_system_members 1.40 (max 5.3), aurora::spawn_auroras 0.57 (max 18.6),
procstars::stream_nodes 0.84, procstars::find_nearest 0.16 (max 2.7; 14-16 ms max in other
scenes), procsystems::manage_systems 0.46, procsystems::promote_objects 0.06 (max 0.9), Bevy
allocate_and_free_meshes 1.12 (max 15.7), system_commands for spawn_system_members 0.12. Earlier
notes (`.audit/work/lag`) logged "generated system ... in 16 ms (2 stars, 1073 bodies)" followed by
841 ms frames.

**Questions:** why does selecting the active system cost ~9.5 ms every frame while flying
(rebuilding or searching every frame? generating systems synchronously on the main thread? cloning
catalogs? `Arc::get_mut` / clone of the catalog)? Is system generation (se_core procgen) on the main
thread? How many entities does each system spawn (1000+ bodies), and could spawning be cheaper
without changing what exists? What does spawn_auroras do on a spike?

**Paths:** `crates/se_engine/src/system.rs` (select_active_system :433, spawn_system_members :589),
`procsystems.rs`, `procstars.rs` (stream_nodes :484, find_nearest :694), `aurora.rs`
(spawn_auroras :283), `crates/se_core/src/procgen/mod.rs`, `proc_galsys.rs`, `star_octree.rs`,
`cluster_stars.rs`, `catalog.rs` (only what these call per frame).

**Bench:** `bash .optimize/bench/all.sh OUT fly`.
