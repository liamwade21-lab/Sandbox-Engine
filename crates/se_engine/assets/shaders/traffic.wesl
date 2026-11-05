// A level 4 civilization's space traffic (orbital.rs, the vegetation and civilizations plan's step
// 10): a stream of ship lights along a quadratic arc from `a` through near `c` to `b`
// (camera-relative metres), each a round dot of at least `misc.z` pixels, moving along it at `b.w`
// cycles a second; shrunk by the universe's scaled space as the bodies at its ends are (`k`: their
// centres' factors, from the start's to the end's), so the arcs meet the bodies as they are drawn.
// With `misc.w` 1 it is one fixed light (a station's beacon from afar). Additive.
//
// Every stream's values are a record of one storage buffer written in place each frame (orbital.rs
// `TRAFFIC_MAX` records); a stream's material holds its record's index alone, so it never changes
// (a material changed every frame is prepared again: it cost about a millisecond).

import bevy_pbr::render::mesh_view_bindings::view;

struct Traffic {
    // xyz the start, w the ships' count
    a: vec4<f32>,
    // xyz the end, w the cycles a second
    b: vec4<f32>,
    // xyz the arc's control point
    c: vec4<f32>,
    // rgb the lights' colour, w their brightness
    color: vec4<f32>,
    // x the clock (s), y a ship's size (m), z its least size (pixels), w 1 for a fixed light
    misc: vec4<f32>,
    // x, y the scaled space's factor at the start's and the end's bodies
    k: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> slot: vec4<u32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> streams: array<Traffic>;

struct Vertex {
    // x the ship's index, yz the quad's corner (-1..1)
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    // rgb the light, the ends' fade in
    @location(1) color: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let tr = streams[slot.x];
    let n = max(tr.a.w, 1.0);
    var t = fract(v.position.x / n + tr.misc.x * tr.b.w);
    if tr.misc.w > 0.5 {
        t = 1.0;
    }
    let u = 1.0 - t;
    // (Relative to the world's origin, where the main camera sits.)
    let p = (u * u * tr.a.xyz + 2.0 * u * t * tr.c.xyz + t * t * tr.b.xyz) * mix(tr.k.x, tr.k.y, t);
    // A pixel's size at the ship's distance from this view, its dot at least `misc.z` of them.
    let px = 2.0 * length(p - view.world_position) / (view.viewport.w * view.clip_from_view[1][1]);
    let size = max(tr.misc.y, tr.misc.z * px);
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let world = p + (right * v.position.y + up * v.position.z) * size;
    var o: VertexOut;
    o.clip = view.clip_from_world * vec4<f32>(world, 1.0);
    o.corner = v.position.yz;
    // Ships fade in and out at the arc's ends.
    let fade = select(smoothstep(0.0, 0.04, t) * (1.0 - smoothstep(0.96, 1.0, t)), 1.0, tr.misc.w > 0.5);
    o.color = tr.color.rgb * (tr.color.w * fade);
    return o;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let g = exp(-dot(in.corner, in.corner) * 4.0);
    if g * max(in.color.r, max(in.color.g, in.color.b)) < 0.01 {
        discard;
    }
    return vec4<f32>(in.color * g, 0.0);
}
