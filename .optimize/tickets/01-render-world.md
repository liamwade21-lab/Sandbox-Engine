# 01 Render-world cost of drawing (planet surfaces first)

**Why:** on an RS planet's ground (scene `rsground`, the user's main complaint: "on the surface of
an RS planet, looking around") and on Earth the render app is the bottleneck. In the trace,
`rsg_look` spends ~19 ms/frame in the render schedule vs ~10 ms in the main app.

**Profile (trace, rsg_look, ms/frame):** write_binned_instance_buffers 2.75 (max 14),
specialize_material_meshes 2.10, prepare_preprocess_bind_groups 1.97, queue_material_meshes 1.46,
prepared_mesh_producer 3.78 (17 calls a frame, parallel), submit_pending_command_buffers 3.60,
queue_submit 2.27, collect_meshes_for_gpu_building 0.65, prepare_material_bind_groups 0.51 (max 16.9),
several prepare_erased_assets<MeshMaterial3d<..>> 0.1-0.56 each, and in the main app
check_entities_needing_specialization<M> 0.14-0.24 each for ~20 material types,
mark_3d_meshes_as_changed_if_their_assets_changed 0.13, calculate_bounds 0.18, visibility
par_for_each 0.67. Earth (`earth_look`) shows the same pattern. Run a scene to get the mean entity count.

**Questions to answer:** how many mesh entities / materials / mesh assets exist in these scenes and
what are they (terrain patches, water and cloud copies, labels, orbit lines, sprites, star nodes,
far galaxies...)? Which are hidden but still extracted, queued or specialized? Which meshes or
materials are touched mutably every frame (`Assets::get_mut`, `ResMut<Assets<..>>` deref, re-inserted
components, `Mesh3d`/`MeshMaterial3d` reassigned) so Bevy re-prepares them? Could entities share
materials or meshes? Are there per-frame `commands.insert` / despawn+spawn churns? Bevy render
settings (GPU preprocessing, clustering with no lights, MSAA, prepasses) that cost CPU for nothing?

**Paths:** `crates/se_engine/src/terrain.rs` (patch spawning, visibility, materials),
`surface_tex.rs`, `atmosphere.rs`, `system.rs` (body meshes), `rings.rs`, `corona.rs`, `aurora.rs`,
`stars.rs`, `procstars.rs` (star node meshes), `galaxies.rs`, `overlay.rs` (label/marker meshes if
any), `graphics.rs`, `render_scale.rs`, `main.rs`, and every `impl Material` (grep). Bevy 0.20 source
is in `~/.cargo/registry/src/*/bevy_pbr-0.20.0`, `bevy_render-0.20.0`, `bevy_core_pipeline-0.20.0`, to
see what triggers re-specialization and re-preparation.

**Bench:** `bash .optimize/bench/all.sh OUT rsground earth` (see HUNTER_BRIEF.md).
