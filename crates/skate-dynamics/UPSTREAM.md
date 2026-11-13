# skate-dynamics (from the Skate 3 Rust Engine)

Copied unchanged from SK8-ENGINE/skate-3-rust-engine (https://github.com/SK8-ENGINE/skate-3-rust-engine), commit
2e16697797c8358d16f747a8259354ff2921041a (2026-10-09), `crates/skate-dynamics`. Licence: GPL-3.0 (the repository's `LICENSE`, copied
to this project's root). Pulled in by `crates/skate-sim` (`physics/network.rs`, `physics/solid_contacts.rs` name its types).
Re-check: `diff -rq re/skate3/src/crates/skate-dynamics crates/skate-dynamics` (prints nothing; run from the project root).
It builds rapier3d 0.35.3 (its own pin) beside the 0.36 that `se_engine` uses; the two never meet.
