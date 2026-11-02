# 08 Start-up, loading hitches and memory

**Why:** the first 300 frames after start average 14.5 ms with a worst frame of 803 ms (1% low
3 fps); peak memory is 1.7-2.1 GB in every scene. Loading a new place (system, nebula, galaxy
models) causes hitches.

**Profile:** measure it: `bash .optimize/bench/all.sh OUT start` and read the log beside the
screenshot (timestamps of the loading lines). A start-up trace can be taken with the trace build
(`PORT_BIN=target/trace/agent/spaceengine-rs.exe bash .optimize/bench/all.sh OUT start`).

**Questions:** what runs on the main thread during start-up that could run on a task (catalog
parsing, pak/zip reading and decompression, texture decoding, galaxy/nebula model building, music
decoding)? Is anything read or parsed twice? Are large buffers kept that are never used again
(decoded images kept in RAM after upload, whole pak files in memory, duplicate catalogs)? Any
`clone()` of large structures (catalog `Arc` made unique)? The order of results must stay identical
(catalog order decides names and ids).

**Paths:** `crates/se_engine/src/data.rs`, `main.rs`, `splash.rs`, `memory.rs`, `music.rs`,
`galaxies.rs` / `nebulae.rs` (model building only), `surface_tex.rs` (texture loading only),
`crates/se_core/src/catalog.rs`, `sc.rs`, `vfs.rs`, `locale.rs`, `galaxy_model.rs`, `places.rs`,
`model_sm.rs`.

**Bench:** `bash .optimize/bench/all.sh OUT start` (peak memory is printed per scene).
