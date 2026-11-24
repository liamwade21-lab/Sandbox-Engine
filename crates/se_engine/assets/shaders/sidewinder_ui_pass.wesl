// Elite's UI movies on their panels and the dark plates behind them, drawn by the UI's own pass (sidewinder/ui/pass.rs;
// ARCHITECTURE.md A10 UI3b) after the engine's colour passes, at the view's full size, into its display-referred main texture.
//
// The panels (`PANEL`): Elite's foreground panel shader `Alpha_GuiLighting_Gui_GuiBrightnessScale_Saturation_
// GUIColourRotate_Forward`, its base permutation, ported from its disassembly (re/elite/frea_export/sidewinder_ui/decoded/
// shaders/ps_/vs_Alpha_GuiLighting_..._Forward0_0_0_0_0_0_0_0_Win64_SM50.txt; rendering.json shader_formulas "GuiLighting"),
// every parameter the material's own (`PanelParams`, from the layout):
//   cloudiness: two samples of the cloudiness texture's green at the panel's planar coordinates (`planar`) x pCloudynessTile
//     + pCloudynessScrollsAB, x pCloudynessScale;
//   the parallax ghost: P = (V.t, V.b) x pParallaxScale (V the view direction), two lerps of pParallaxLerp towards the movie
//     sampled from uv + 2 P pDiffuse_AlphaMetrics.zw back by P pDiffuse_AlphaMetrics.zw, desaturated by pParallaxDesaturate
//     (luma 0.3086, 0.6094, 0.0820);
//   col = ghost x pCloudynessTintA x ca + ghost x pCloudynessTintB x cb + movie (rgba); discard if col.a < 1e-5;
//   rgb = pow(col.rgb, 1.6) x gStandardMapScales.x x pGuiTint x gGuiBrightness x gExposureCompensation; the GUI colour
//     matrix; the saturation luma + pSaturation (rgb - luma); discard if rgb and col.a are all under 5/255;
//   out = (rgb x gHDRScalingWrite, col.a), blended premultiplied (the material's flags).
// The movie's texels are sampled as stored (Rgba8Unorm: sRGB-encoded, premultiplied), as Elite's movie target is read.
//
// The plates (`PLATE`): Elite's `Alpha_UnlitNoScale_GuiBackground_GUIColourRotate_Forward` ported from its disassembly
// (..._GuiBackground_GUIColourRotate_Forward0_0_0_0_Win64_SM50.txt; its constant buffer's names from cb_layouts_ui.json),
// every parameter the material's own (`PlateParams`):
//   vertex: the planar coordinates x pCloudynessTile + pCloudynessScrollsAB.xy for the cloudiness, x pScanlineTile for the
//     scanlines; uv0 for the alpha shape;
//   c = (1 + cloudiness.g x pCloudynessScale) x scanlines.a x pGuiTint x pScanlineScale;
//   a = alpha.a - pAlphaClip, discarded under 0, saturated; c = c x a x pBackgroundAlpha x gExposureCompensation x
//   gGuiBackgroundMultiplier; out.a = saturate(a x pDestAlphaScale x pBackgroundAlpha), discarded under 1e-5; the GUI colour
//   matrix; out.rgb = c x gHDRScalingWrite; blended premultiplied (the material's flags).
// Left out (a stand-in, E): the term behind `if (cb1[49].x)`, the scene colour behind the plate x pBackgroundScale (cb1[49]'s
// value and the scene texture Elite binds there are not decoded): the branch Elite takes when the flag is 0.
//
// The last step is ours (grade C), because Elite writes these values into its HDR scene and its tone map, which is not
// decoded, makes the screen's colours: sidewinder_ui_common.wesl `screen_fit` turns the value into the linear colour
// Elite's screen shows, and the pass writes it as it is (nothing of the engine's exposure, bloom or tone curve comes after).
// A material with its depth test on is hidden where the scene is in front (sidewinder_ui_common.wesl
// `ui_in_front`, the scene's depth from the view's depth texture).

import bevy_render::view::View;
import super::sidewinder_ui_common::{Globals, seat_z, depth_texel, ui_in_front, screen_fit};

// One piece (pass.rs `PieceRecord`): its model matrix in the frame's world; `info` x the ship's own scale, y 1 when its
// material tests its depth (its flag in Elite's data).
struct Piece {
    model: mat4x4<f32>,
    info: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> globals: Globals;
@group(0) @binding(2) var<storage, read> pieces: array<Piece>;
@if(MSAA) @group(0) @binding(3) var scene_depth: texture_depth_multisampled_2d;
@if(NO_MSAA) @group(0) @binding(3) var scene_depth: texture_depth_2d;

struct PanelParams {
    tint: vec4<f32>,
    cloud: vec4<f32>,
    scrolls: vec4<f32>,
    tint_a: vec4<f32>,
    tint_b: vec4<f32>,
    parallax: vec4<f32>,
    metrics: vec4<f32>,
};

struct PlateParams {
    tint: vec4<f32>,
    alpha: vec4<f32>,
    cloud: vec4<f32>,
    scrolls: vec4<f32>,
    scan: vec4<f32>,
};

@if(PANEL) @group(1) @binding(0) var<uniform> panel: PanelParams;
@if(PANEL) @group(1) @binding(1) var movie_tex: texture_2d<f32>;
@if(PANEL) @group(1) @binding(2) var movie_smp: sampler;
@if(PLATE) @group(1) @binding(0) var<uniform> plate: PlateParams;
@if(PLATE) @group(1) @binding(1) var alpha_tex: texture_2d<f32>;
@if(PLATE) @group(1) @binding(2) var alpha_smp: sampler;
@if(PLATE) @group(1) @binding(5) var scan_tex: texture_2d<f32>;
@if(PLATE) @group(1) @binding(6) var scan_smp: sampler;
@group(1) @binding(3) var cloud_tex: texture_2d<f32>;
@group(1) @binding(4) var cloud_smp: sampler;

const LUMA: vec3<f32> = vec3<f32>(0.3086, 0.6094, 0.0820);

struct Vertex {
    @builtin(instance_index) piece: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The two cloudiness coordinates (a plate: the cloudiness's and the scanlines').
    @location(0) cloud_uv: vec4<f32>,
    @location(1) bitangent: vec3<f32>,
    // From the eye to the point (the frame's world axes).
    @location(2) view_vec: vec3<f32>,
    @location(3) tangent: vec3<f32>,
    @location(4) uv: vec2<f32>,
    // x the clip w, y the ship's own scale (the depth test's: `ui_in_front`), z 1 when the material tests its depth.
    @location(5) depth: vec3<f32>,
};

// The panel's planar coordinates: its local position on the vertex's tangent and on cross(N, T), unsigned (both GUI vertex
// stages: `dp3` of the position with the tangent and with that cross product; only the parallax's bitangent, `o1`, takes the
// tangent's sign). In the mod's axes, where the model is mirrored in z, Elite's cross(N, T) is minus ours.
fn planar(v: Vertex) -> vec2<f32> {
    return vec2<f32>(dot(v.position, v.tangent.xyz), -dot(v.position, cross(v.normal, v.tangent.xyz)));
}

// The GUI colour matrix: out = c.x M0 + c.y M1 + c.z M2.
fn gui_matrix(c: vec3<f32>) -> vec3<f32> {
    return c.x * globals.m0.xyz + c.y * globals.m1.xyz + c.z * globals.m2.xyz;
}

// The vertex's place, frame and depth (the seat's depth slice: the clip z is the depth the cockpit itself is drawn with).
fn place(v: Vertex) -> VertexOut {
    var o: VertexOut;
    let p = pieces[v.piece];
    let m = p.model;
    let world = m * vec4<f32>(v.position, 1.0);
    o.clip = view.clip_from_world * world;
    o.clip.z = seat_z(o.clip.w, p.info.x, globals.seat.x, globals.seat.y);
    o.depth = vec3<f32>(o.clip.w, p.info.x, p.info.y);
    let m3 = mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);
    let n = normalize(m3 * v.normal);
    let t = normalize(m3 * v.tangent.xyz);
    // (The ship's frame of the bitangent: b = cross(n, t), turned round when the tangent's sign is negative, and again where
    // the model matrix mirrors, as Bevy's `mesh_tangent_local_to_world` turns it.)
    let sign = v.tangent.w * select(1.0, -1.0, determinant(m3) < 0.0);
    o.tangent = t;
    o.bitangent = cross(n, t) * sign;
    o.view_vec = world.xyz - view.world_position;
    o.uv = v.uv;
    return o;
}

// The fragment's depth test, when its material has one.
fn hidden(in: VertexOut) -> bool {
    if in.depth.z < 0.5 {
        return false;
    }
    let last = vec2<i32>(textureDimensions(scene_depth)) - vec2<i32>(1);
    let scene = textureLoad(scene_depth, depth_texel(in.clip.xy, view.viewport, view.main_pass_viewport, last), 0);
    return !ui_in_front(in.depth.x, in.depth.y, globals.seat.x, globals.seat.y, scene);
}

@if(PANEL)
@vertex
fn vertex(v: Vertex) -> VertexOut {
    var o = place(v);
    let p = planar(v);
    o.cloud_uv = p.xyxy * panel.cloud.zwzw + panel.scrolls;
    return o;
}

@if(PANEL)
@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let ca = textureSample(cloud_tex, cloud_smp, in.cloud_uv.xy).g * panel.cloud.x;
    let cb = textureSample(cloud_tex, cloud_smp, in.cloud_uv.zw).g * panel.cloud.x;
    let vn = normalize(in.view_vec);
    let p = vec2<f32>(dot(vn, in.tangent), dot(vn, in.bitangent)) * panel.parallax.z;
    let movie = textureSample(movie_tex, movie_smp, in.uv);
    var uv = in.uv + 2.0 * p * panel.metrics.zw;
    var ghost = vec4<f32>(0.0);
    for (var i = 0; i < 2; i++) {
        ghost += panel.parallax.y * (textureSample(movie_tex, movie_smp, uv) - ghost);
        uv -= p * panel.metrics.zw;
    }
    ghost = vec4<f32>(mix(ghost.rgb, vec3<f32>(dot(ghost.rgb, LUMA)), panel.parallax.x), ghost.a);
    let col = ghost * panel.tint_a * ca + ghost * panel.tint_b * cb + movie;
    var rgb = pow(max(col.rgb, vec3<f32>(0.0)), vec3<f32>(1.6)) * panel.tint.w * panel.tint.rgb * globals.gui.x * globals.gui.y;
    rgb = gui_matrix(rgb);
    let l = dot(rgb, LUMA);
    rgb = l + panel.cloud.y * (rgb - l);
    if col.a < 1e-5 || (all(rgb < vec3<f32>(5.0 / 255.0)) && col.a < 5.0 / 255.0) || hidden(in) {
        discard;
    }
    return vec4<f32>(screen_fit(rgb * globals.gui.z, globals.fit.xyz), col.a);
}

@if(PLATE)
@vertex
fn vertex(v: Vertex) -> VertexOut {
    var o = place(v);
    let p = planar(v);
    o.cloud_uv = vec4<f32>(p * plate.cloud.zw + plate.scrolls.xy, p * plate.scan.xy);
    return o;
}

@if(PLATE)
@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let cloud = textureSample(cloud_tex, cloud_smp, in.cloud_uv.xy).g;
    let shape = textureSample(alpha_tex, alpha_smp, in.uv).a;
    let scan = textureSample(scan_tex, scan_smp, in.cloud_uv.zw).a;
    var c = (cloud * plate.cloud.x + 1.0) * scan * plate.tint.rgb * plate.tint.w;
    let a0 = shape - plate.alpha.x;
    let a = saturate(a0);
    c = c * a * plate.alpha.y * globals.gui.y * globals.gui.w;
    let out_a = saturate(a * plate.alpha.w * plate.alpha.y);
    if a0 < 0.0 || out_a < 1e-5 || hidden(in) {
        discard;
    }
    c = gui_matrix(c) * globals.gui.z;
    return vec4<f32>(screen_fit(c, globals.fit.xyz), out_a);
}
