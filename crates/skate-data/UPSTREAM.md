# skate-data (from the Skate 3 Rust Engine)

Copied unchanged from SK8-ENGINE/skate-3-rust-engine (https://github.com/SK8-ENGINE/skate-3-rust-engine), commit
2e16697797c8358d16f747a8259354ff2921041a (2026-10-09), `crates/skate-data`. Licence: GPL-3.0 (the repository's `LICENSE`,
copied to this project's root). Credit: the Skate 3 Rust Engine's authors, building on dumbad's years of Skate 3 reverse
engineering (DumbadsSkate3ModdingTools) and Chasm's work.

Sandbox Engine uses it only to make the Skate 3 physics mod's files (`crates/se_engine/src/skate/convert.rs`, an ignored
test). Do not edit it here: take a newer copy whole if one is needed, and note the commit above.

Left out: `examples/` and `tests/trigger_volumes.rs`, which read files outside the crate (the engine's game crate and a map
in its repository) and would not build here.
