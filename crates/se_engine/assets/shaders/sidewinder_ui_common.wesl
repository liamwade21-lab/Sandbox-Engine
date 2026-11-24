// The seam of Elite's cockpit UI's own pass (sidewinder/ui/pass.rs; ARCHITECTURE.md A10 UI3b): what every drawing in that
// pass shares, as functions of their arguments (no bindings), so the panels and plates (sidewinder_ui_pass.wesl) and later
// the scanner and the holograms take the same last step, the same depth test and the same bias.
//
// The pass draws after the engine's colour passes (color_correction, FXAA / SMAA) into the view's main texture at the
// view's full size, where the values are display-referred and linear (the swapchain's copy encodes sRGB): a GUI value goes
// through `screen_fit` and is written as it is. The scene's depth is the view's own depth texture, which the main pass
// fills only in its top-left corner under the render scale (`view.main_pass_viewport`): `depth_texel` maps a full-size
// pixel into it, `ui_in_front` tests a UI fragment against it.

// The UI pass's values (pass.rs `UiGlobalsGpu`), bound by the panels' pass and by the scanner's and holograms' passes.
struct Globals {
    // x gGuiBrightness, y gExposureCompensation, z gHDRScalingWrite, w gGuiBackgroundMultiplier.
    gui: vec4<f32>,
    // x the seat's depth slice, y its near plane.
    seat: vec4<f32>,
    // The screen fit's A, p, B.
    fit: vec4<f32>,
    // The GUI colour matrix's rows Red, Green, Blue.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
};

// How far, as a share of the fragment's own distance from the eye, a UI fragment is moved towards the eye before its
// depth test (the ONE place it is set). The scene's depth is sampled at the solids' reduced size (1.57 screen pixels a
// texel on the user's settings), so on a surface seen at a slant the texel's depth differs from the depth under the full-size
// pixel: across one texel of 1.57 / 1347 rad at 1 m, a surface at 80 degrees from face-on changes by 1347-th of 1.57 x
// tan 80 = 6.6 mm, a share of 0.0066. 0.01 keeps every GUI piece lying on or just over the cockpit (the gauges on the dash,
// the plates on their bezels) in front of its own surface up to 83 degrees, while the cockpit's frame, the chair and the
// pilot, which stand centimetres to decimetres in front of the panels they cross, still hide them.
const DEPTH_BIAS: f32 = 0.01;

// A clip position's depth in the seat's depth slice (sidewinder_surface_hull.wesl `ship_depth`'s seat branch): `w` the clip
// w, `k` the model matrix's scale (the ship's own: never a GUI piece's), `slice` and `near` `SEAT_SLICE`, `SEAT_NEAR_M`.
fn seat_z(w: f32, k: f32, slice: f32, near: f32) -> f32 {
    return w * (1.0 - slice) + slice * near * k;
}

// The scene depth texel under a full-size pixel `frag` (its framebuffer position): `viewport` and `main_pass` the view's
// `viewport` and `main_pass_viewport`, `last` the depth texture's last texel.
fn depth_texel(frag: vec2<f32>, viewport: vec4<f32>, main_pass: vec4<f32>, last: vec2<i32>) -> vec2<i32> {
    let t = (frag - viewport.xy) * main_pass.zw / max(viewport.zw, vec2<f32>(1.0)) + main_pass.xy;
    return clamp(vec2<i32>(floor(t)), vec2<i32>(0), last);
}

// A UI fragment in the seat's slice (clip w `w`, model scale `k`) is not behind the scene's depth `scene` (reversed depth,
// as the engine has it: larger is nearer), once moved towards the eye by `DEPTH_BIAS`.
fn ui_in_front(w: f32, k: f32, slice: f32, near: f32, scene: f32) -> bool {
    let wb = w * (1.0 - DEPTH_BIAS);
    return seat_z(wb, k, slice, near) / wb >= scene;
}

// Elite's GUI value (the shader's result, premultiplied) to the linear colour Elite's screen shows: A v^p / (v^p + B), the
// fit measured on session ui1's pictures (sidewinder/ui/material.rs `SCREEN_FIT`, grade C; `fit` its A, p, B).
fn screen_fit(v: vec3<f32>, fit: vec3<f32>) -> vec3<f32> {
    let vp = pow(max(v, vec3<f32>(0.0)), vec3<f32>(fit.y));
    return fit.x * vp / (vp + fit.z);
}
