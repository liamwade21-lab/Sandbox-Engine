// The Sidewinder's shadow passes (sidewinder/shadow.rs; the shared maths and the record: sidewinder_shadow_common.wesl).
//
// - The map pass (`map_vertex` with `map_moments`, or `map_vertex` alone): the mod's own copy of the casters' triangles (shadow.rs
//   `Casters`: a ship-space position, its rig node and, for the seat's chair, its bone), each node at its matrix this frame
//   (`parts`, written in place when the pose changes) and the chair's vertices posed by the same palette the ship's material
//   reads. `map_moments` writes the nearest caster's moments (a layer of the moments map: the outside set's or the seat's);
//   the vertex stage alone, with the depth test reversed, keeps the furthest caster's depth (the outside set only).
// - The pass over the world (`world_vertex`, `world_fragment`): before the clouds, multiplying what the main pass drew, from
//   the depth buffer, as the Viltrumite mod's craters do (hero_fx.wesl `decal_fragment`). A pixel further from the star than
//   the outside set's furthest caster there is behind the ship and loses the star's direct light by the engine's rule for a
//   shadow on the ground (hero.wesl `fragment_shadow`, its few lines copied below), as much as Elite's filter says it is in
//   shadow. A pixel no further than that is the ship itself (its own shader shadows it) or in front of it: left alone; so is
//   the sky and the seat's depth slice.
//   Known and accepted (ARCHITECTURE.md A6): a thing standing between two parts of the ship along the star's ray (a prop under
//   a wing in front of a gear leg) is no further than the furthest caster there, so it gets no shadow there; and the canopy's
//   glass, drawn before this pass without depth, is darkened with a shadowed pixel behind it.
// Both stages collapse every vertex to one point outside the view (nothing is drawn) while the star gives no light at the
// ship's place (night, an eclipse: `star_lit`), and the world's while this frame's maps are not drawn (`live`).

import bevy_render::view::View;
import super::hero_frame::{HeroFrame, HeroLight};
import super::sidewinder_shadow_common::{ShipShadow, to_ship_point, to_ship_dir, map_point, map_uv, quantised_moments, filtered_moments, visibility, star_lit};

@group(0) @binding(0) var<storage, read> ship_shadow: ShipShadow;
// Each rig node's matrix in the ship's space now (a hidden node's is zero: its triangles have no area).
@group(0) @binding(1) var<storage, read> parts: array<mat4x4<f32>>;
// The pilot chair's palette (material.rs `ShipPalette`, `PALETTE_BONES` long).
@group(0) @binding(2) var<storage, read> palette: array<mat4x4<f32>, 142>;
// The sandbox's light records at the player's place, as the ship's material binds them (bindings 0 and 1 there).
@group(0) @binding(3) var<storage, read> frame: HeroFrame;
@group(0) @binding(4) var<storage, read> light: HeroLight;
@group(0) @binding(5) var<uniform> view: View;
@if(MSAA) @group(0) @binding(6) var scene_depth: texture_depth_multisampled_2d;
@if(!MSAA) @group(0) @binding(6) var scene_depth: texture_depth_2d;
@group(0) @binding(7) var moments: texture_2d_array<f32>;
@group(0) @binding(8) var moments_smp: sampler;
@group(0) @binding(9) var far_map: texture_depth_2d;

// shadow.rs `NO_BONE`: a vertex not posed by the palette.
const NO_BONE: u32 = 0xffffffffu;

// Every vertex here: outside the clip volume, all at one point.
const NOWHERE: vec4<f32> = vec4<f32>(2.0, 2.0, 2.0, 1.0);

struct MapOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) depth: f32,
};

@vertex
fn map_vertex(@location(0) position: vec3<f32>, @location(1) part: u32, @location(2) bone: u32) -> MapOut {
    var o: MapOut;
    if !star_lit(light.sun.rgb) {
        o.clip = NOWHERE;
        o.depth = 1.0;
        return o;
    }
    var p = vec4<f32>(position, 1.0);
    if bone != NO_BONE {
        p = palette[bone] * p;
    }
    let m = map_point(ship_shadow, (parts[part] * p).xyz, to_ship_dir(ship_shadow, frame.light.sun_dir.xyz));
    o.clip = vec4<f32>(m.xy, m.z, 1.0);
    o.depth = m.z;
    return o;
}

// The nearest caster's moments, its depth moved away from the star by its slope across a texel times the write's slope bias
// (ours: the receiving surface itself then reads as lit, `ShadowRecord::bias`).
@fragment
fn map_moments(in: MapOut) -> @location(0) vec4<f32> {
    let slope = max(abs(dpdx(in.depth)), abs(dpdy(in.depth)));
    return quantised_moments(ship_shadow, in.depth + ship_shadow.bias.w * slope);
}

struct WorldOut {
    @builtin(position) clip: vec4<f32>,
};

// One triangle over the whole view.
@vertex
fn world_vertex(@builtin(vertex_index) vi: u32) -> WorldOut {
    var o: WorldOut;
    if ship_shadow.live.x == 0u || !star_lit(light.sun.rgb) {
        o.clip = NOWHERE;
        return o;
    }
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    o.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn depth_at(pix: vec2<i32>) -> f32 {
    return textureLoad(scene_depth, pix, 0);
}

// The point a pixel sees, in the render frame. (Through `main_pass_viewport`: under the render scale the pixels fill a corner.)
fn world_at(pix: vec2<i32>, d: f32) -> vec3<f32> {
    let vp = view.main_pass_viewport;
    let c = vec2<f32>(pix) + 0.5;
    let ndc = vec2<f32>((c.x - vp.x) / vp.z * 2.0 - 1.0, 1.0 - (c.y - vp.y) / vp.w * 2.0);
    let w = view.world_from_clip * vec4<f32>(ndc, max(d, 1e-12), 1.0);
    return w.xyz / w.w;
}

// The step to the neighbour on the same surface: of the two on an axis, the one whose depth is nearer the pixel's.
fn surface_step(pix: vec2<i32>, p: vec3<f32>, d: f32, step: vec2<i32>) -> vec3<f32> {
    let da = depth_at(pix + step);
    let db = depth_at(pix - step);
    if abs(da - d) <= abs(db - d) {
        return world_at(pix + step, da) - p;
    }
    return p - world_at(pix - step, db);
}

@fragment
fn world_fragment(in: WorldOut) -> @location(0) vec4<f32> {
    let none = vec4<f32>(1.0);
    let pix = vec2<i32>(in.clip.xy);
    let d = depth_at(pix);
    // The sky, and the seat's slice (the cockpit, the chair, the glass: its own shader shadows it).
    if d <= 0.0 || d >= ship_shadow.map.w {
        return none;
    }
    let p = world_at(pix, d);
    // Which way what it sees faces, from its neighbours, turned towards the camera.
    var n = normalize(cross(surface_step(pix, p, d, vec2<i32>(1, 0)), surface_step(pix, p, d, vec2<i32>(0, 1))));
    n = select(n, -n, dot(n, view.world_position - p) < 0.0);
    let l = normalize(frame.light.sun_dir.xyz);
    // The most direct light it can have had (ours): by its own facing, or by the ground's (hero.wesl's plane is the ground; the
    // scatter's plants are lit per instance as the ground is, whatever their cards face). A face turned from the star that is
    // not the ground's loses the ground's share: too dark there, the rule's one known error.
    let nl = max(max(dot(n, l), dot(normalize(frame.up.xyz), l)), 0.0);
    if nl <= 0.0 {
        return none;
    }
    let m = map_point(ship_shadow, to_ship_point(ship_shadow, p), to_ship_dir(ship_shadow, l));
    // Off the map's square, or nearer the star than the ship's sphere: nothing of the ship is between it and the star.
    if abs(m.x) >= 1.0 || abs(m.y) >= 1.0 || m.z < 0.0 {
        return none;
    }
    let uv = map_uv(m.xy);
    let texel = clamp(vec2<i32>(uv * ship_shadow.map.x), vec2<i32>(0), vec2<i32>(i32(ship_shadow.map.x) - 1));
    // Not behind the furthest caster: the ship itself, or a thing in front of or among its parts.
    if m.z <= textureLoad(far_map, texel, 0) + ship_shadow.bias.y {
        return none;
    }
    let lit = visibility(ship_shadow, filtered_moments(moments, moments_smp, 0, uv, ship_shadow), min(m.z, ship_shadow.bias.z));
    // hero.wesl `fragment_shadow`: the light the surface gets without the star's over the light it gets with it, by how much of
    // the star's direct light the clouds and an eclipse let through (`HeroLight::sun.w`; their floor is a quarter). Its N.L is
    // `nl` above (his shadow's is its plane's), and the share is what Elite's filter says is in shadow.
    let sun = light.sun.rgb * nl;
    let rest = light.sky.rgb + light.flat_e.rgb + vec3<f32>(1e-6);
    let factor = rest / (sun + rest);
    let direct = clamp((light.sun.w - 0.25) / 0.75, 0.0, 1.0);
    return vec4<f32>(mix(vec3<f32>(1.0), factor, direct * (1.0 - lit)), 1.0);
}
