# 04 Per-frame body positions, lighting, eclipses and lensing

**Why:** moving every body of the active system each frame is the largest port-side cost on Earth
after the info panel.

**Profile (trace, earth_look, ms/frame):** par_for_each over (SystemBody, UPosition, ...) 3.28
(17 chunks a frame; 0.24 in fly, 0.55 in the nebula scene), system::update_bodies 0.44,
update_lighting 0.58, eclipse::update_eclipse_casters 0.25, terrain::update_eclipses 0.27,
universe::apply_floating_origin 0.23, update_sprites 0.14, update_projected_size 0.11,
camera::update_nearest 0.12, transform propagation (par_traversal_mark_dirty 0.51, mark_dirty_trees
0.2-0.5). In the fly scene blackholes::update_lens 1.77 (max 9.8).

**Questions:** how many bodies does the Solar System have here, and is the full Kepler / rotation /
ephemeris solve redone for each every frame, including bodies far too small to see? (Skipping them
would change their state: only allowed if everything drawn or shown stays identical.) Repeated
lookups (catalog by name, HashMap per body per frame), allocations, constant terms (orbital elements
to matrices) recomputed every frame, transforms written every frame even when unchanged (which
triggers Bevy transform propagation and re-extraction), O(n^2) eclipse caster searches. What does
update_lens scan while flying?

**Paths:** `crates/se_engine/src/system.rs` (update_bodies :1115, update_lighting :1232,
update_projected_size :1380, update_sprites :1453 and the parallel body query), `universe.rs`
(apply_floating_origin :216), `eclipse.rs`, `terrain.rs` (update_eclipses :294 only), `blackholes.rs`
(update_lens :623), `camera.rs` (update_nearest :1029), `crates/se_core/src/orbit.rs`, `rotation.rs`
and whatever else the body update calls.

**Bench:** `bash .optimize/bench/all.sh OUT space earth fly`.
