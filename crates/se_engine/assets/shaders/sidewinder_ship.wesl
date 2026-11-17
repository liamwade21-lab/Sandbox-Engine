// The Sidewinder's hull, gear, cockpit and chairs (sidewinder/material.rs `ShipMaterial`): Elite's models, each material
// worked out by its Elite shader family's surface function and lit by Elite's lit maths (ARCHITECTURE.md A6). This file is
// the vertex stage, the seat's depth slice and the family dispatch:
// - sidewinder_surface_hull.wesl: the outside's families (painted hull, bare metal, opaque, emissive, decals, the stamp);
// - sidewinder_surface_cockpit.wesl: the cockpit's tile, chair and console families;
// - sidewinder_glass.wesl: the glass, with its own entry points (`vertex_glass`, `fragment_glass`: it reads the vertex's
//   colour);
// - sidewinder_lit.wesl: the lighting (the sandbox's light record at the player's place, which hero_light.wesl writes before
//   the main pass, as the props take it, and the ship's own lights buffer).
// Each material compiles only its own family's code (its key's shader def, material.rs `Family::def`). Nothing in a material
// changes a frame: the light is in the storage buffers. A skinned material (the seat's chair, `SHIP_SKINNED`) poses each
// vertex by its bone's palette matrix before the model's transform (`posed`); an unskinned one compiles as before.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world, mesh_tangent_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::view;
import super::hero_frame::{HeroFrame, HeroLight};
import super::sidewinder_lit::{Surface, Scene, shade, stamp_factor, star_shadow, outside_light};
import super::sidewinder_surface_hull::{ShipParams, Frame, ship_depth, surface_painted, surface_single_tile, surface_opaque, surface_emissive, surface_overlay, surface_stamp, surface_plain};
import super::sidewinder_surface_cockpit::{surface_cockpit, surface_cockpit_emissive, surface_cockpit_tile};
import super::sidewinder_glass::{GlassVertex, GlassOut, glass_vertex, glass_cockpit, glass_tinted, glass_edges};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> frame: HeroFrame;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> ship_light: HeroLight;
// The family's textures in its own order (sidewinder_surface_hull.wesl, material.rs `Family::slots`); one sampler.
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var tex0: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var tex1: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var tex2: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var tex3: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var tex4: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var tex5: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var<uniform> params: ShipParams;
// (Bindings 10-13, the ship's lights, are sidewinder_lit.wesl's.)
// The pilot chair's bone palette (material.rs `ShipPalette`, `PALETTE_BONES` long): per bone, the bind pose to the posed one
// in the model's space, in the mod's axes. Bound to every material; only a skinned one (the seat's chair) declares it.
@if(SHIP_SKINNED) @group(constants::MATERIAL_BIND_GROUP) @binding(19) var<storage, read> palette: array<mat4x4<f32>, 142>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // xyz Elite's tangent, w the frame's sign (both already in the mod's axes).
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) uv2: vec2<f32>,
    // A skinned model's: the vertex's one bone (material.rs `ATTRIBUTE_BONE`).
    @if(SHIP_SKINNED) @location(7) bone: u32,
};

// The vertex posed by its bone, in the model's space, before the model's own transform: position, normal and tangent (the
// bones turn and scale evenly, so the matrix's 3x3 turns the directions; the world transform normalises them after).
@if(SHIP_SKINNED)
fn posed(v: Vertex) -> Vertex {
    let b = palette[v.bone];
    let r = mat3x3<f32>(b[0].xyz, b[1].xyz, b[2].xyz);
    var o = v;
    o.position = (b * vec4<f32>(v.position, 1.0)).xyz;
    o.normal = r * v.normal;
    o.tangent = vec4<f32>(r * v.tangent.xyz, v.tangent.w);
    return o;
}

// (Unskinned: as it is.)
@if(!SHIP_SKINNED)
fn posed(v: Vertex) -> Vertex {
    return v;
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_tangent: vec4<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) uv2: vec2<f32>,
    @location(5) world_pos: vec3<f32>,
};

@vertex
fn vertex(input: Vertex) -> VertexOut {
    let v = posed(input);
    let m = get_world_from_local(v.instance_index);
    let world = mesh_position_local_to_world(m, vec4<f32>(v.position, 1.0));
    var o: VertexOut;
    o.clip = position_world_to_clip(world.xyz);
    // (The seat's depth slice and Elite's depth bias: sidewinder_surface_hull.wesl `ship_depth`.)
    o.clip.z = ship_depth(o.clip, length(m[0].xyz), params);
    o.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    o.world_tangent = mesh_tangent_local_to_world(m, v.tangent, v.instance_index);
    o.uv0 = v.uv0;
    o.uv1 = v.uv1;
    o.uv2 = v.uv2;
    o.world_pos = world.xyz;
    return o;
}

// The sandbox's light at the player's place, as sandbox_prop.wesl reads it; the star's light at `world_pos` (normal `n`) past
// the ship's own casters (sidewinder_lit.wesl `star_shadow`, the seat's map for the seat's draws).
fn scene(world_pos: vec3<f32>, n: vec3<f32>) -> Scene {
    let sun_dir = normalize(frame.light.sun_dir.xyz);
    let up = normalize(frame.up.xyz);
    let night = 1.0 - smoothstep(-0.12, 0.03, dot(up, sun_dir));
    let shadow = star_shadow(world_pos, n, sun_dir, view.world_position, -normalize(view.world_from_view[2].xyz), params.view.x > 0.5);
    // (Everything of it that is light from outside the ship times `outside_light`: 0 in hyperspace's tunnel. At 1 the terms are
    // taken as they are, not multiplied: a multiply by 1 is exact, but it would let the compiler fuse differently downstream
    // (an FMA into the haze's add moved the glass's pixels by one level), and the pictures must stay bit for bit.)
    let k = outside_light();
    let whole = k == 1.0;
    let sun = ship_light.sun.rgb * shadow;
    let town = frame.town.rgb * night;
    return Scene(
        sun_dir,
        select(sun * k, sun, whole),
        select(ship_light.sky.rgb * k, ship_light.sky.rgb, whole),
        select(ship_light.flat_e.rgb * k, ship_light.flat_e.rgb, whole),
        ship_light.haze_t.rgb,
        select(ship_light.haze_a.rgb * k, ship_light.haze_a.rgb, whole),
        up,
        select(town * k, town, whole),
    );
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let f = Frame(n, in.world_tangent.xyz, cross(n, in.world_tangent.xyz) * select(1.0, -1.0, in.world_tangent.w < 0.0));
    let sc = scene(in.world_pos, n);
    var s: Surface;
    @if(SHIP_PAINTED) {
        s = surface_painted(f, in.uv0, in.uv1, in.uv2, tex0, tex1, tex2, tex3, tex4, tex5, smp, params);
    }
    @if(SHIP_SINGLE_TILE) {
        s = surface_single_tile(f, in.uv0, in.uv1, tex0, tex1, tex2, tex3, smp, params);
    }
    @if(SHIP_OPAQUE) {
        s = surface_opaque(f, in.uv0, tex0, tex1, tex2, smp, params);
    }
    @if(SHIP_EMISSIVE) {
        s = surface_emissive(f, in.uv0, tex0, tex1, tex2, tex3, smp, params);
    }
    @if(SHIP_OVERLAY) {
        s = surface_overlay(f, in.uv0, tex0, tex1, smp, params);
    }
    @if(SHIP_PLAIN) {
        s = surface_plain(f, in.uv0, tex0, smp, params);
    }
    @if(SHIP_COCKPIT) {
        s = surface_cockpit(f, in.uv0, tex0, tex1, tex2, smp, params);
    }
    @if(SHIP_COCKPIT_EMISSIVE) {
        s = surface_cockpit_emissive(f, in.uv0, tex0, tex1, tex2, tex3, smp, params);
    }
    @if(SHIP_COCKPIT_TILE) {
        s = surface_cockpit_tile(f, in.uv0, in.uv1, in.uv2, tex0, tex1, tex2, tex3, tex4, tex5, smp, params);
    }
    @if(SHIP_STAMP) {
        // A multiplying blend over what is under it (sidewinder_surface_hull.wesl's header).
        let st = surface_stamp(f, in.uv0, tex0, tex1, smp, params);
        return vec4<f32>(vec3<f32>(stamp_factor(n, st.normal, st.ao, sc)), 1.0);
    }
    @if(!SHIP_STAMP) {
        return vec4<f32>(shade(s, in.world_pos, view.world_position, sc, params.lights.x, params.lights.y), s.alpha);
    }
}

// The glass's entry points (material.rs `Family::is_glass`): its own vertex stage, and its family's function, premultiplied.
@if(SHIP_GLASS)
@vertex
fn vertex_glass(v: GlassVertex) -> GlassOut {
    return glass_vertex(v, params);
}

@if(SHIP_GLASS)
@fragment
fn fragment_glass(in: GlassOut) -> @location(0) vec4<f32> {
    let sc = scene(in.world_pos, normalize(in.world_normal));
    @if(SHIP_GLASS_COCKPIT) {
        return glass_cockpit(in, tex0, tex1, smp, params, sc);
    }
    @if(SHIP_GLASS_TINTED) {
        return glass_tinted(in, tex0, tex1, tex2, smp, params, sc);
    }
    @if(SHIP_GLASS_EDGES) {
        return glass_edges(in, tex0, tex1, tex2, smp, params, sc);
    }
}
