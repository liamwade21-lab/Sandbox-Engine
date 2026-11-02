# 09 Build settings, Bevy configuration and scheduling

**Why:** compiler settings, Bevy plugins/features that do nothing for this port, and systems that
can't run in parallel cost time in every frame.

**Profile:** in every trace the main app's PostUpdate is much longer than the sum of the port's own
systems in it, and "multithreaded executor" spans are frequent (588k in rsg_look). GPU light
clustering (`clustering/elapsed_gpu` 0.06 ms, `cluster_on_gpu`, `prepare_clusters_for_gpu_clustering`
0.16 ms) runs although the port may use no Bevy lights. `uniform_allocation` and `bin_unpacking`
GPU passes appear in surface scenes. The release profile is `lto = "thin"`, `codegen-units = 1`.

**Questions:** compiler/profile settings (fat LTO, panic strategy, `-C target-cpu`; mark any that
could change float results or crash behaviour as "needs your call"); Bevy default features/plugins
the port doesn't use (PBR light clustering, shadows, prepasses, gltf, a second audio backend...)
whose systems still run every frame; system sets forcing serial execution (many systems taking
`ResMut` of the same resource, exclusive systems, `.chain()` where not needed); logging (`info!` /
`debug!` formatting in per-frame paths, the log file layer); per-frame `Commands` churn; task pool
sizes.

**Paths:** `Cargo.toml`, `crates/se_engine/Cargo.toml`, `crates/se_engine/src/main.rs`,
`graphics.rs`, `display.rs`, `log_file.rs`, `memory.rs`, and the plugin `build` functions across
`crates/se_engine/src/*.rs` (for schedules and ordering only).

**Bench:** `bash .optimize/bench/all.sh OUT space rsground fly`.
