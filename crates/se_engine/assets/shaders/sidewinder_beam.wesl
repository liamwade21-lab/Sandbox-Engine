// The two parts of the ground's headlight record only the GPU writes (ground_light.wesl `ShipBeam`, volumetric_clouds.rs
// `BeamRecord`), each frame while a headlight is lit, before anything reads the record (sidewinder/exposure.rs): the automatic
// exposure's compensation (as sidewinder_exposure.wesl copies it for the ship), and the headlights' IES profile as the ship's
// own sampler reads it (sidewinder_lit.wesl `ship_lamps`: level 0 at v 0.5), at each texel's centre of its row, so the ground's
// linear lookup between them is the sampler's.

import super::ground_light::CloudShadow;

@group(0) @binding(0) var<storage, read> auto_state: vec4<f32>;
@group(0) @binding(1) var<storage, read_write> record: CloudShadow;
@group(0) @binding(2) var ies_tex: texture_2d<f32>;
@group(0) @binding(3) var ies_smp: sampler;

// One invocation a vec4 of the table (256: four workgroups).
@compute @workgroup_size(64)
fn fill(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i == 0u {
        record.beam.head.w = auto_state.x;
    }
    if i >= 256u {
        return;
    }
    let size = f32(textureDimensions(ies_tex).x);
    var v = vec4<f32>(0.0);
    for (var k = 0u; k < 4u; k++) {
        v[k] = textureSampleLevel(ies_tex, ies_smp, vec2<f32>((f32(i * 4u + k) + 0.5) / size, 0.5), 0.0).r;
    }
    record.beam.profile[i] = v;
}
