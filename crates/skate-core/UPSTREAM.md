# skate-core (from the Skate 3 Rust Engine)

Copied unchanged from SK8-ENGINE/skate-3-rust-engine (https://github.com/SK8-ENGINE/skate-3-rust-engine), commit
2e16697797c8358d16f747a8259354ff2921041a (2026-10-09), `crates/skate-core`. Licence: GPL-3.0 (the repository's `LICENSE`,
copied to this project's root). Credit: the Skate 3 Rust Engine's authors, building on dumbad's years of Skate 3 reverse
engineering (DumbadsSkate3ModdingTools) and Chasm's work.

Sandbox Engine uses it for the Skate 3 physics mod (`crates/se_engine/src/skate/`: the push, the brake, the steering's tilt,
the jump, the pose maths). Do not edit it here: take a newer copy whole if one is needed, and note the commit above.

Its tests: 654 of 655 pass here; `physics::skeleton_body::collision_feedback_tests::a_moving_group_8_body_reaches_native_impact_feedback_for_a_stationary_actor`
fails in the untouched upstream copy as well (checked 2026-11-13).
