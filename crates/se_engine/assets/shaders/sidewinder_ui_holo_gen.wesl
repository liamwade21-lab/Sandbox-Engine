// The holograms' generation pass (sidewinder/ui/hologram.rs `gen_pass`; ARCHITECTURE.md A10 UI6): Elite's
// `ps_Schematic_Generation_Core_Schematic_0` ported from its disassembly (re/elite/frea_export/sidewinder_ui/decoded/hud3d/
// shaders/; re/elite/notes/ui_hud3d.md 2.2): the schematic model rendered into a one-channel buffer,
//   out.x = D (1 - h)^P x (1 + W ((1 - sat(N . -V))^Z - 1)) + fwd . V (here fwd . V times the depth's scale: ours, E),
// (D, P, Z, W) = pSchematic_Data, h = the height map's red at uv0 + pHeightMapUVScrollSpeed x time, V the point from the eye,
// fwd the view's forward axis (`cb1[279]`), so the value is the point's depth plus the material's small offsets, which the
// display's Sobel turns into lines. Both powers' bases are kept at least 1e-4, as the disassembly's `max`.
// The camera is ours (E: Elite's view matrix was not found, notes 8.4): orthographic down the view's -z, the model normalised
// to its bounding sphere filling the buffer; so V's direction is the view's axis everywhere.

struct GenUniform {
    view_from_model: mat4x4<f32>,
    // x the half side, y the depth's scale (hologram.rs `DEPTH_SCALE`, E), z the eye's distance, w the far plane.
    ortho: vec4<f32>,
    // pSchematic_Data (D, P, Z, W).
    data: vec4<f32>,
    // xy the height map's scroll now, z pVerticalTiling: unread by Elite's pixel shader (this one). Elite's constant buffer for
    // this pass also holds pVerticalTiling and the extrusion flags (cb_layout.json), which its VERTEX shader reads; that shader
    // is not in the dumps, so this pass's vertex stage is ours (a plain transform) and whatever Elite's does to the model there
    // is missing: the likely cause of the maze-like pattern inside our models against Elite's clean features (a known
    // difference).
    scroll: vec4<f32>,
};

@group(0) @binding(0) var<uniform> gen: GenUniform;
@group(0) @binding(1) var height_tex: texture_2d<f32>;
@group(0) @binding(2) var height_smp: sampler;

struct In {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    // The point from the eye, the view's axes.
    @location(1) rel: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vertex(v: In) -> Out {
    var o: Out;
    let p = gen.view_from_model * vec4<f32>(v.position, 1.0);
    o.rel = p.xyz;
    o.normal = (gen.view_from_model * vec4<f32>(v.normal, 0.0)).xyz;
    o.clip = vec4<f32>(p.x / gen.ortho.x, p.y / gen.ortho.x, -p.z / gen.ortho.w, 1.0);
    o.uv = v.uv;
    return o;
}

@fragment
fn fragment(i: Out) -> @location(0) vec4<f32> {
    let fwd = vec3<f32>(0.0, 0.0, -1.0);
    let n = normalize(i.normal);
    // (Orthographic: the ray from the eye is the view's forward axis at every point.)
    let ndv = saturate(dot(n, -fwd));
    let rim = 1.0 + gen.data.w * (pow(max(1.0 - ndv, 1e-4), gen.data.z) - 1.0);
    let h = textureSample(height_tex, height_smp, i.uv + gen.scroll.xy).r;
    let lift = gen.data.x * pow(max(1.0 - h, 1e-4), gen.data.y);
    return vec4<f32>(lift * rim + gen.ortho.y * dot(fwd, i.rel), 0.0, 0.0, 0.0);
}
