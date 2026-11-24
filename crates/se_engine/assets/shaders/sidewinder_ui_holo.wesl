// The holograms' display (sidewinder/ui/hologram.rs; ARCHITECTURE.md A10 UI6), drawn by the holograms' pass in the UI's place
// in the frame (sidewinder/ui/pass.rs's seam: after the scanner, at the view's full size, display-referred): Elite's
// `ps_Schematic_Display_BillboardSobel_Forward_0_0_0` ported from its disassembly (re/elite/frea_export/sidewinder_ui/decoded/
// hud3d/shaders/; constant names from cb_layout.json; re/elite/notes/ui_hud3d.md 2.2), every value the display material's own
// (`HoloParams`, the run-time tints in place of its white ones: hologram.rs says whose):
//   eight neighbours of the generated buffer at +-pSobelEdgeThickness screen pixels (its uv's derivatives); unless one is under
//   100 (the model), nothing. Else c = the middle; g = the 3 x 3 Sobel with weights 0.09375, 0.3125, 0.09375 across and down,
//   G = 32 |g|; dist = min(G x pSobelDistanceScale, pSobelMaxDistance); the scanlines read at the view ray's point at depth c
//   (the ray over its forward depth, times c) dotted with pReferenceRight / pReferenceUp, x pScanlineTile + pScanlineOffset,
//   s = red x pScanlineScaleOffset.x + .y; e = sat(G pSobelTintCoordScale)^pSobelPower; core = s pSobelCoreIntensity
//   pSobelTintLow; col = core + e (pSobelTintHigh pSobelEdgeIntensity - core); col x= dist x pSchematicPhaseInLerpValue^2;
//   alpha = max(sat(dist pDestAlphaScale) x PhaseIn, c < 1000 ? pSchematicOpacity : 0);
//   then x gExposureCompensation x gGuiBrightness, the GUI colour matrix, discard under an alpha of 1e-5, x gHDRScalingWrite.
// Left out (off in the shipped materials or no cause in the mod): the id and colour tinting, the emissive rescaling (its enable
// is 0), the mosaic glitch and shake variants. `cb1[120].z` (the edge thickness's screen factor, not found: E) is 1. Elite
// compares the scene's depth itself; so does this (sidewinder_ui_common.wesl `ui_in_front`, against the view's depth texture).
// Elite samples the buffer with a filtering sampler: here four texel reads (R32Float is not filterable everywhere).
// The billboard faces the camera (Elite's `SchematicBillboard`); its middle and side are hologram.rs's measured ones. Then the
// same last step as the panels (sidewinder_ui_common.wesl `screen_fit`, ours, grade C), written as it is, blended
// premultiplied (E).

import bevy_render::view::View;
import super::sidewinder_ui_common::{Globals, seat_z, depth_texel, ui_in_front, screen_fit};

struct HoloParams {
    // x pSobelTintCoordScale, y pSobelPower, z pSobelCoreIntensity, w pSobelEdgeIntensity.
    sobel: vec4<f32>,
    // x pSobelDistanceScale, y pSobelMaxDistance, z pSobelEdgeThickness, w pDestAlphaScale.
    dist: vec4<f32>,
    // rgb pSobelTintLow, w pSchematicPhaseInLerpValue.
    low: vec4<f32>,
    // rgb pSobelTintHigh, w pSchematicOpacity.
    high: vec4<f32>,
    // xy pScanlineTile, zw pScanlineOffset.
    scan: vec4<f32>,
    // xy pScanlineScaleOffset, z the hologram (0 own, 1 target), w the billboard's half side (metres).
    scan_scale: vec4<f32>,
    // pReferenceRight, pReferenceUp, as the material holds them.
    right: vec4<f32>,
    up: vec4<f32>,
    // x the generated depth's scale (hologram.rs `DEPTH_SCALE`, ours): taken back out for the scanlines.
    depth: vec4<f32>,
};

// The 3D parts' view group (scanner.rs `PartsView`).
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> globals: Globals;
@if(MSAA) @group(0) @binding(2) var scene_depth: texture_depth_multisampled_2d;
@if(NO_MSAA) @group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(1) @binding(0) var<uniform> holo: HoloParams;
// Each hologram's billboard this frame (hologram.rs `HoloFrameGpu`): xyz its middle in the frame's world, w the ship's own
// scale (the billboard's side and the seat's depth slice scale with it); indexed by `scan_scale.z`.
@group(1) @binding(1) var<uniform> places: array<vec4<f32>, 2>;
@group(1) @binding(2) var generated: texture_2d<f32>;
@group(1) @binding(3) var scan_tex: texture_2d<f32>;
@group(1) @binding(4) var scan_smp: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // From the eye to the point.
    @location(1) rel: vec3<f32>,
    // x the clip w, y the ship's own scale (the depth test's: `ui_in_front`).
    @location(2) depth: vec2<f32>,
};

// The GUI colour matrix: out = c.x M0 + c.y M1 + c.z M2.
fn gui_matrix(c: vec3<f32>) -> vec3<f32> {
    return c.x * globals.m0.xyz + c.y * globals.m1.xyz + c.z * globals.m2.xyz;
}

// The billboard: a square of side 1 round its middle, two triangles from the vertex index (no vertex buffer), uv 0..1, v down.
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<u32, 6>(0u, 2u, 1u, 0u, 3u, 2u);
    let c = corners[index];
    let corner = vec2<f32>(select(-0.5, 0.5, c == 1u || c == 2u), select(-0.5, 0.5, c < 2u));
    var o: VertexOut;
    let place = places[u32(holo.scan_scale.z + 0.5)];
    let scale = place.w;
    let half = holo.scan_scale.w * scale;
    let world = place.xyz + view.world_from_view[0].xyz * (2.0 * half * corner.x) + view.world_from_view[1].xyz * (2.0 * half * corner.y);
    o.clip = view.clip_from_world * vec4<f32>(world, 1.0);
    // (The seat's depth slice: the depth the cockpit itself is drawn with.)
    o.clip.z = seat_z(o.clip.w, scale, globals.seat.x, globals.seat.y);
    o.depth = vec2<f32>(o.clip.w, scale);
    o.uv = vec2<f32>(corner.x + 0.5, 0.5 - corner.y);
    o.rel = world - view.world_position;
    return o;
}

// The generated buffer at uv, bilinear from four texels (clamped at its edges).
fn buffer_at(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(generated));
    let p = uv * size - 0.5;
    let i = floor(p);
    let f = p - i;
    let hi = vec2<i32>(size) - 1;
    let a = vec2<i32>(clamp(i, vec2<f32>(0.0), vec2<f32>(hi)));
    let b = vec2<i32>(clamp(i + 1.0, vec2<f32>(0.0), vec2<f32>(hi)));
    let t00 = textureLoad(generated, a, 0).r;
    let t10 = textureLoad(generated, vec2<i32>(b.x, a.y), 0).r;
    let t01 = textureLoad(generated, vec2<i32>(a.x, b.y), 0).r;
    let t11 = textureLoad(generated, b, 0).r;
    return mix(mix(t00, t10, f.x), mix(t01, t11, f.x), f.y);
}

// Behind the scene's depth (the cockpit's frame, the pilot), as Elite's shader compares it.
fn hidden(in: VertexOut) -> bool {
    let last = vec2<i32>(textureDimensions(scene_depth)) - vec2<i32>(1);
    let scene = textureLoad(scene_depth, depth_texel(in.clip.xy, view.viewport, view.main_pass_viewport, last), 0);
    return !ui_in_front(in.depth.x, in.depth.y, globals.seat.x, globals.seat.y, scene);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    // (x max(cb1[120].z, 1): cb1[120].z not found, 1.) In the screen's own pixels: the pass draws at the view's full size.
    let k = holo.dist.z;
    let dx = dpdx(in.uv) * k;
    let dy = dpdy(in.uv) * k;
    let uv = in.uv;
    // Left column (-dx), right column (+dx), each from -dy to +dy; the top (-dy) and bottom (+dy) rows' middles.
    let left = vec3<f32>(buffer_at(uv - dx - dy), buffer_at(uv - dx), buffer_at(uv - dx + dy));
    let right = vec3<f32>(buffer_at(uv + dx - dy), buffer_at(uv + dx), buffer_at(uv + dx + dy));
    let top = buffer_at(uv - dy);
    let bottom = buffer_at(uv + dy);
    var col = vec3<f32>(0.0);
    var a = 0.0;
    if any(left < vec3<f32>(100.0)) || any(right < vec3<f32>(100.0)) || top < 100.0 || bottom < 100.0 {
        let c = buffer_at(uv);
        let w = vec3<f32>(0.09375, 0.3125, 0.09375);
        let gx = dot(w, right) - dot(w, left);
        let gy = dot(w, vec3<f32>(left.z, bottom, right.z)) - dot(w, vec3<f32>(left.x, top, right.x));
        let g = sqrt(gx * gx + gy * gy) * 32.0;
        let dist = min(g * holo.dist.x, holo.dist.y);
        let fwd = -view.world_from_view[2].xyz;
        // (The decode reads the buffer's value here as it is; ours carries the depth's scale, which is not the decode's: divided
        // out, so the scanlines see the view depth.)
        let p = in.rel / dot(in.rel, fwd) * (c / holo.depth.x);
        let sc = vec2<f32>(dot(p, holo.right.xyz) * holo.scan.x, dot(p, holo.up.xyz) * holo.scan.y) + holo.scan.zw;
        let s = textureSampleLevel(scan_tex, scan_smp, sc, 0.0).r * holo.scan_scale.x + holo.scan_scale.y;
        let e = pow(saturate(g * holo.sobel.x), holo.sobel.y);
        let core = s * holo.sobel.z * holo.low.rgb;
        col = (core + e * (holo.high.rgb * holo.sobel.w - core)) * dist * holo.low.w * holo.low.w;
        a = max(saturate(dist * holo.dist.w) * holo.low.w, select(0.0, holo.high.w, c < 1000.0));
    }
    col = gui_matrix(col * globals.gui.y * globals.gui.x);
    if a < 1e-5 || hidden(in) {
        discard;
    }
    return vec4<f32>(screen_fit(col * globals.gui.z, globals.fit.xyz), a);
}
