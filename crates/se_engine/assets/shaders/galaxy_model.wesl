// 3D galaxy model sprites (emission and dust) generated from a SpaceEngine .pst model.
//
// Vertices: position = sprite centre relative to the galaxy centre (pc, universe frame),
// data = (tint rgb, intensity), info = (size pc, atlas tile, lod random 0..1, layer),
// corner = quad corner. Each sprite is a camera-facing billboard with a random in-plane
// rotation, drawn at infinity (like the starfield) so only directions matter.
//
// One mesh per model holds emission, dust and bulge, sorted far to near (SpriteOrder, as SE's
// FUN_140333a30), drawn with SE's blend (galaxy_model.glsl / nebula_model.glsl, glBlendFunc(ONE,
// ONE_MINUS_SRC_ALPHA) in FUN_140334e90, Bevy's AlphaMode::Premultiplied):
// Emission and bulge: output (rgb * emBright, 0), i.e. additive.
// Dust:     output (-rgb * emBright, absBright) * coverage: darkens what is behind by absBright and
//           subtracts the dust colour (the tonemapper clamps the result at 0).
// Both are multiplied by SE's Reddening (1 - x, 1 - 2x, 1 - 4x), x = distance / 4.227e9 pc (FUN_140299fe0).

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_world_from_local;

struct ModelParams {
    // xyz: galaxy centre relative to the camera (pc), w: model fade (0..1)
    center_rel: vec4<f32>,
    // x: emission brightness (emBright), y: min sprite radius (px), z: absorption strength (absBright), w unused
    params: vec4<f32>,
    // unused
    params2: vec4<f32>,
};

// SE's reddening distance (DAT_14075eb8c, pc: about c / H0).
const REDDEN_PC: f32 = 4.227e9;

fn reddening(d_pc: f32) -> vec3<f32> {
    let x = d_pc / REDDEN_PC;
    return clamp(vec3<f32>(1.0) - x * vec3<f32>(1.0, 2.0, 4.0), vec3<f32>(0.0), vec3<f32>(1.0));
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> model: ModelParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var atlas: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atlas_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) data: vec4<f32>,
    @location(2) info: vec4<f32>,
    @location(3) corner: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tile: f32,
    @location(3) weight: f32,
    // Sprite layer: 0 emission, 1 dust, 2 bulge (SE's layer byte VertexData.w: 0 marks dust).
    @location(4) @interpolate(flat) layer: f32,
};

fn hidden() -> VertexOut {
    var o: VertexOut;
    o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    o.color = vec3<f32>(0.0);
    o.uv = vec2<f32>(0.0);
    o.tile = 0.0;
    o.weight = 0.0;
    o.layer = 0.0;
    return o;
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let rel = model.center_rel.xyz + v.position;
    let d = max(length(rel), 1e-9);
    let size_pc = v.info.x;
    let focal = view.clip_from_view[1][1] * view.viewport.w * 0.5;
    let s_px = size_pc / d * focal;
    // SE fades a sprite as the eye comes within 1.5 of its radii, gone inside half a radius
    // (galaxy_model.glsl / nebula_model.glsl: clamp(dist / spriteSize - 0.5, 0, 1)); by distance,
    // so zooming in leaves the glow as it is.
    let big = clamp(d / max(size_pc, 1e-9) - 0.5, 0.0, 1.0);
    if big <= 0.0 || model.center_rel.w <= 0.0 {
        return hidden();
    }
    let r_px = max(s_px, model.params.y);
    // Sub-pixel sprites keep their flux: scale intensity by the covered area.
    let small = min(s_px / r_px, 1.0);

    let world_from_local = get_world_from_local(v.instance_index);
    let wdir = normalize((world_from_local * vec4<f32>(rel / d, 0.0)).xyz);
    var clip = view.clip_from_world * vec4<f32>(wdir, 0.0);
    if clip.w <= 0.0 {
        return hidden();
    }
    // Cull sprites whose quad cannot touch the screen (this also avoids huge, numerically
    // unstable triangles for centres near the camera plane).
    let margin = r_px * 2.0 * 1.5 / view.viewport.zw;
    let ndc = clip.xy / clip.w;
    if abs(ndc.x) > 1.0 + margin.x || abs(ndc.y) > 1.0 + margin.y {
        return hidden();
    }
    // Random in-plane rotation per sprite.
    let ang = v.info.z * 6.2831853;
    let c = cos(ang);
    let s = sin(ang);
    let rc = vec2<f32>(c * v.corner.x - s * v.corner.y, s * v.corner.x + c * v.corner.y);
    clip = vec4<f32>(clip.xy + rc * r_px * 2.0 / view.viewport.zw * clip.w, 0.0, clip.w);

    var o: VertexOut;
    o.clip = clip;
    // Reddening of the whole model by its centre's distance (SE passes one value per galaxy).
    o.color = v.data.rgb * model.params.x * reddening(length(model.center_rel.xyz));
    o.uv = v.corner;
    o.tile = v.info.y;
    o.weight = v.data.w * small * small * big * model.center_rel.w;
    o.layer = v.info.w;
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let t = floor(in.tile + 0.5);
    let col = t - 4.0 * floor(t / 4.0);
    let row = floor(t / 4.0);
    let q = in.uv * 0.48 + vec2<f32>(0.5, 0.5);
    let uv = (vec2<f32>(col, row) + q) * 0.25;
    // Fade the quad edge to zero so huge near sprites never show their rectangular outline.
    let edge = 1.0 - smoothstep(0.75, 1.0, max(abs(in.uv.x), abs(in.uv.y)));
    let w = textureSample(atlas, atlas_sampler, uv).r * edge * in.weight;
    if abs(in.layer - 1.0) > 0.5 {
        return vec4<f32>(in.color * w, 0.0);
    }
    return vec4<f32>(-in.color * w, clamp(model.params.z * w, 0.0, 1.0));
}
