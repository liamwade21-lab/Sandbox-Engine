// The automatic exposure's state (auto_exposure.wesl: x the adapted compensation in stops, positive darker; y the metered
// median) copied into the ship's own buffer before the opaque pass (sidewinder/exposure.rs), which every ship material binds
// (sidewinder_lit.wesl `ship_adapt`).

@group(0) @binding(0) var<storage, read> auto_state: vec4<f32>;
@group(0) @binding(1) var<storage, read_write> ship_adapt: vec4<f32>;

@compute @workgroup_size(1)
fn copy() {
    ship_adapt = auto_state;
}
