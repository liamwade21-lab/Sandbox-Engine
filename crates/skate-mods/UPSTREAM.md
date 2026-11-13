# skate-mods (TRIMMED copy of the Skate 3 Rust Engine's crate)

From SK8-ENGINE/skate-3-rust-engine, commit 2e16697797c8358d16f747a8259354ff2921041a, `crates/skate-mods`. Licence: GPL-3.0.
Only the data types `crates/skate-sim` names (`skate_mods::presentation::{CameraShotTuning, CameraAngle}`,
`skate_mods::extensions::{PartOverride, JointOverride}`) are included: `src/presentation.rs`, `src/extensions.rs`,
`src/lua_list.rs`, `src/schema.rs` are byte-identical to upstream; `src/lib.rs` and `Cargo.toml` are new (upstream's pull in
mlua with a vendored Lua, gltf, zip and blake3). The Lua mod host itself is NOT here.
Re-check: `for f in presentation extensions lua_list schema; do diff -q re/skate3/src/crates/skate-mods/src/$f.rs crates/skate-mods/src/$f.rs; done`
