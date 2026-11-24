// Elite's scanner in the cockpit (sidewinder/ui/scanner.rs; ARCHITECTURE.md A10 UI6), drawn by the scanner's pass in the UI's
// place in the frame (sidewinder/ui/pass.rs's seam: after the panels, at the view's full size, display-referred): three of
// the Radar_* effects ported from their disassembly (re/elite/frea_export/sidewinder_ui/decoded/hud3d/shaders/, base
// permutations; the constant buffers' names from cb_layout.json; re/elite/notes/ui_hud3d.md sections 1.4 to 1.7), every
// material value the material's own (`RadarParams`, one a draw), the frame's values in `RadarFrame`:
//   the disc (`vs_/ps_RadarPlane_Forward_0_0_0_0`): position (x, z) x gRadarSize; t = the texture at uv (alpha the mask);
//     rgb = (t.a > 0.01) x t.rgb x 0.5 x InRadarModeOnly x gRadarLuminanceScale x pLuminance; inside 2 |uv - 0.5| <
//     gRadarSweepRadius the pulse: P = 250 + frac(t / cycle) x cycle x speed x 1000, q = floor((500 x 10^(6.56 r - 1) - 250)
//     / 1000) x 1000 + 250 + (1 - rate) / rate x 1000 (r = |uv - 0.5|), f = sat(1 - floor(|P - q| / 1000) x rate), k =
//     max(2 f pRadarPulseAlphaModifier, 0.25); rgb x= k (6.15, 4.19, 1.68), alpha t.a x k;
//   the own-ship marker (`vs_/ps_RadarPlayer_Forward`): position (x, z) x gRadarSize x sat(gRadarMinimumRange / gRadarRange
//     + 0.25) x gRadarScalePlayer.xz x the triangle's scale; rgb = colour x InRadarModeOnly x gRadarLuminanceScale x
//     pLuminance; alpha = sat(sat(1 - |P - q| rate / 1000) pRadarPulseAlphaModifier + 0.15) with q = 500 x 10^-1 + (1 - rate) /
//     rate x 1000 (the marker sits at the disc's middle);
//   the range indicator (`vs_/ps_RadarRangeIndicator_Forward`): p = position x gRadarSize / 2; height (1.25 p.y - 0.26) x
//     scale.y; angle 4.35 + 0.0525 p.x + 0.705 (gRadarRange - min) / (max - min); x = 0.8 gRadarSize cos(angle) + 0.0525 p.x
//     scale.x, z = 0.8 gRadarSize sin(angle) + p.z scale.z + 0.0375 gRadarSweepRadius gRadarSize; rgb = colour x
//     InBothModes x gRadarLuminanceScale x pLuminance, alpha = sat(InBothModes x gRadarLuminanceScale x pLuminance);
//   then the colour matrix (`gRadarColourMatrix` taken as the HUD's GUI colour matrix, B), x gPhaseOutAlpha, x
//   gHDRScalingWrite (`cb1[90].y`).
// Left out: the disc's vertical 0.001 x TEXCOORD1.x (its stream is not in the dumps; the disc is flat), the ECM, scan, glitch
// and sphere variants (no cause in the mod).
// Then the blend (SrcAlpha, One: a stand-in, E) is applied here, the colour weighted by its alpha, and the result goes through
// the same last step as the panels (sidewinder_ui_common.wesl `screen_fit`, ours, grade C), written as it is, added (One, One).

import bevy_render::view::View;
import super::sidewinder_ui_common::{Globals, screen_fit};

// Which of the two candidate blends (the effect's blend state is not in the decode). Both were rendered against session ui1's
// `a00_start.png` (the rings down the column through the disc's middle): Elite's rings 97-182 levels of red, green over red
// 0.12-0.41, each 1-2 px thin; (SrcAlpha, One) 41-87, 0.00-0.11, as thin; (One, One) 106-230, 0.29-0.70, every ring 3-4 times
// as thick and the whole disc lit (the texture's faint ring edges pass the shader's alpha > 0.01 mask at full colour). Neither
// lands inside the picture's error; (SrcAlpha, One) keeps the rings' shape, so it is kept (E), the disc half as bright and
// redder than Elite's (a known difference: gRadarLuminanceScale stays its stand-in 1).
const SRC_ALPHA_BLEND: bool = true;

struct RadarParams {
    // x the draw (0 disc, 1 own-ship marker, 2 range indicator), y pLuminance, z pRadarPulseAlphaModifier, w
    // pRadarPulseAlphaReductionRate.
    kind: vec4<f32>,
    colour: vec4<f32>,
    scale: vec4<f32>,
};

struct RadarFrame {
    // The disc's frame in the frame's world (the radar's units to the world).
    model: mat4x4<f32>,
    // x gRadarTime (within the cycle), y gRadarRange, z gRadarMinimumRange, w gRadarMaximumRange.
    range: vec4<f32>,
    // x gRadarSize, y gRadarSweepRadius, z gRadarPulseCycle, w gRadarPulseSpeed.
    size: vec4<f32>,
    // x gRadarLuminanceScale, y InRadarModeOnly, z InBothModes, w gPhaseOutAlpha.
    look: vec4<f32>,
};

// The 3D parts' view group (scanner.rs `PartsView`; its depth texture is the holograms').
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> draws: array<RadarParams, 3>;
@group(1) @binding(1) var<uniform> scene: RadarFrame;
@group(1) @binding(2) var disc_tex: texture_2d<f32>;
@group(1) @binding(3) var disc_smp: sampler;

struct Vertex {
    // The draw (`draws`' index).
    @builtin(instance_index) draw: u32,
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) draw: u32,
};

// The GUI colour matrix: out = c.x M0 + c.y M1 + c.z M2.
fn gui_matrix(c: vec3<f32>) -> vec3<f32> {
    return c.x * globals.m0.xyz + c.y * globals.m1.xyz + c.z * globals.m2.xyz;
}

// The pulse's place: P = 250 + frac(t / cycle) x cycle x speed x 1000 (gRadarTime is kept within the cycle on the CPU).
fn pulse_place() -> f32 {
    let span = scene.size.w * scene.size.z * 1000.0;
    return fract(scene.size.w * scene.range.x * 1000.0 / span) * span + 250.0;
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    var o: VertexOut;
    o.uv = v.uv;
    o.draw = v.draw;
    let radar = draws[v.draw];
    let size = scene.size.x;
    let kind = u32(radar.kind.x + 0.5);
    var local = vec3<f32>(v.position.x * size, 0.0, v.position.z * size);
    if kind == 1u {
        let s = saturate(scene.range.z / scene.range.y + 0.25);
        local = vec3<f32>(s * v.position.x * size * radar.scale.x * v.uv.x, 0.0, s * v.position.z * size * radar.scale.z * v.uv.x);
    } else if kind == 2u {
        let p = v.position * size * 0.5;
        let height = (p.y * 1.25 - 0.26) * radar.scale.y;
        let angle = p.x * 0.0525 + 4.35 + (scene.range.y - scene.range.z) / (scene.range.w - scene.range.z) * 0.705;
        let r = size * 0.8;
        local = vec3<f32>(cos(angle) * r + p.x * 0.0525 * radar.scale.x, height, sin(angle) * r + p.z * radar.scale.z + 0.0375 * scene.size.y * size);
    }
    o.clip = view.clip_from_world * (scene.model * vec4<f32>(local, 1.0));
    // (Drawn over everything, `pDepthEnable 0`: no depth test; any depth inside the range, so the near plane cuts nothing.)
    o.clip.z = 0.5 * o.clip.w;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let radar = draws[in.draw];
    let kind = u32(radar.kind.x + 0.5);
    let lum = radar.kind.y;
    let modifier = radar.kind.z;
    let rate = radar.kind.w;
    // (Sampled for every draw, in uniform control flow: the draw is a varying here; only the disc uses it.)
    let t = textureSample(disc_tex, disc_smp, in.uv);
    var rgb = vec3<f32>(0.0);
    var a = 0.0;
    if kind == 0u {
        let r = length(in.uv - vec2<f32>(0.5));
        rgb = select(0.0, 1.0, t.a > 0.01) * t.rgb * 0.5 * (scene.look.y * scene.look.x * lum);
        a = t.a;
        if 2.0 * r < scene.size.y {
            var q = 500.0 * exp2((r * 6.56 - 1.0) * 3.321928) - 250.0;
            q = floor(q * 0.001) * 1000.0 + 250.0 + (1.0 - rate) / rate * 1000.0;
            let f = saturate(1.0 - floor(abs(pulse_place() - q) * 0.001) * rate);
            let k = max(2.0 * f * modifier, 0.25);
            rgb = k * rgb * vec3<f32>(6.15, 4.19, 1.68);
            a = k * a;
        }
    } else if kind == 1u {
        let q = 500.0 * exp2(-3.321928) + (1.0 - rate) / rate * 1000.0;
        let f = saturate(1.0 - abs(pulse_place() - q) * rate * 0.001);
        a = saturate(f * modifier + 0.15);
        rgb = radar.colour.rgb * scene.look.y * scene.look.x * lum;
    } else {
        let l = scene.look.z * scene.look.x * lum;
        rgb = radar.colour.rgb * l;
        a = saturate(l);
    }
    // The blend (not in the decode): (SrcAlpha, One) weights the colour by the alpha; (One, One) adds it as it is.
    let weight = select(1.0, a * scene.look.w, SRC_ALPHA_BLEND);
    let v = gui_matrix(rgb) * globals.gui.z * weight;
    if all(v <= vec3<f32>(0.0)) {
        discard;
    }
    return vec4<f32>(screen_fit(v, globals.fit.xyz), 0.0);
}
