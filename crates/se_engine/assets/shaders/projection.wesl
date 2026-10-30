// SE's projection modes (`projection.rs`): the six cube faces (raw HDR, D3D order +X right, -X left,
// +Y up, -Y down, +Z ahead, -Z behind, each an upright picture) resampled into the frame:
// proj_fisheye.glsl (mode 2), proj_cylinder.glsl (mode 3) or a mosaic of the faces (mode 1).

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Proj {
    // x mode, y flags (1 flip x, 2 flip y, 4 face edges, 8 fit to screen), z cubemap format
    mode: vec4<u32>,
    // x lens angle (rad), y crop, z zoom, w frame aspect
    fish: vec4<f32>,
    offset: vec4<f32>,
    warp: vec4<f32>,
    // xy frame size (px), z face size
    screen: vec4<f32>,
};

@group(0) @binding(0) var<uniform> proj: Proj;
@group(0) @binding(1) var face0: texture_2d<f32>;
@group(0) @binding(2) var face1: texture_2d<f32>;
@group(0) @binding(3) var face2: texture_2d<f32>;
@group(0) @binding(4) var face3: texture_2d<f32>;
@group(0) @binding(5) var face4: texture_2d<f32>;
@group(0) @binding(6) var face5: texture_2d<f32>;
@group(0) @binding(7) var face_sampler: sampler;
// Per face (column, row) in xy / zw pairs, then the grid (x columns, y rows).
@group(0) @binding(8) var<uniform> cells: array<vec4<u32>, 4>;

const PI: f32 = 3.14159265358;

fn sample_face(face: u32, st: vec2<f32>) -> vec4<f32> {
    switch face {
        case 0u: { return textureSampleLevel(face0, face_sampler, st, 0.0); }
        case 1u: { return textureSampleLevel(face1, face_sampler, st, 0.0); }
        case 2u: { return textureSampleLevel(face2, face_sampler, st, 0.0); }
        case 3u: { return textureSampleLevel(face3, face_sampler, st, 0.0); }
        case 4u: { return textureSampleLevel(face4, face_sampler, st, 0.0); }
        default: { return textureSampleLevel(face5, face_sampler, st, 0.0); }
    }
}

// A camera-space direction (x right, y up, -z ahead) on the cube (`capture.rs` dir_to_face).
fn sample_cube(d: vec3<f32>) -> vec4<f32> {
    let e = vec3<f32>(d.x, d.y, -d.z);
    let a = abs(e);
    var face = 0u;
    var sc = 0.0;
    var tc = 0.0;
    var ma = 1.0;
    if a.x >= a.y && a.x >= a.z {
        ma = a.x;
        if e.x > 0.0 { face = 0u; sc = -e.z; } else { face = 1u; sc = e.z; }
        tc = -e.y;
    } else if a.y >= a.z {
        ma = a.y;
        sc = e.x;
        if e.y > 0.0 { face = 2u; tc = e.z; } else { face = 3u; tc = -e.z; }
    } else {
        ma = a.z;
        tc = -e.y;
        if e.z > 0.0 { face = 4u; sc = e.x; } else { face = 5u; sc = -e.x; }
    }
    let st = (vec2<f32>(sc, tc) / max(ma, 1e-30) + 1.0) * 0.5;
    return sample_face(face, st);
}

// proj_fisheye.glsl: TexCoord = (uv * 2 + Offset.xy) * Offset.zw (SE's coordinates run down, its
// unflipped sign is -1, its frame x ahead, y up, z left), r >= CutOff discarded, the ray
// (cos A, sinr y, sinr x) + Warp with A = LensAngle r.
fn fisheye(uv: vec2<f32>) -> vec4<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let flip = vec2<f32>(select(1.0, -1.0, (proj.mode.y & 1u) != 0u), select(1.0, -1.0, (proj.mode.y & 2u) != 0u));
    let p = vec2<f32>((ndc.x + proj.offset.x) * proj.fish.w, ndc.y - proj.offset.y) * flip / proj.fish.z;
    let r = length(p);
    if r >= proj.fish.y {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let angle = proj.fish.x * r;
    var k = proj.fish.x;
    if r > 1e-6 {
        k = sin(angle) / r;
    }
    let w = proj.warp.xyz;
    let ray = vec3<f32>(k * p.x - w.z, k * p.y + w.y, -(cos(angle) + w.x));
    return sample_cube(ray);
}

// proj_cylinder.glsl: Spherical = ((u * 2 - 0.5) pi, (0.5 - v) pi), the cube coordinate
// (cos lat sin lon, sin lat, cos lat cos lon) in SE's frame (x ahead, y up, z left).
fn cylinder(uv: vec2<f32>) -> vec4<f32> {
    let lon = (uv.x * 2.0 - 0.5) * PI;
    let lat = (0.5 - uv.y) * PI;
    let d = vec3<f32>(-cos(lat) * cos(lon), sin(lat), -cos(lat) * sin(lon));
    return sample_cube(d);
}

fn cell_of(face: u32) -> vec2<f32> {
    let c = cells[face / 2u];
    if (face & 1u) == 0u {
        return vec2<f32>(c.xy);
    }
    return vec2<f32>(c.zw);
}

// The cubic mode: the faces laid out by `CubemapFormat` (`ShowCubempEdges`, `FitCubemapToScreen`).
fn mosaic(uv: vec2<f32>) -> vec4<f32> {
    let grid = vec2<f32>(cells[3].xy);
    let size = proj.screen.xy;
    var area = size;
    if (proj.mode.y & 8u) != 0u {
        // Square faces, centred.
        let cell = min(size.x / grid.x, size.y / grid.y);
        area = grid * cell;
    }
    let origin = (size - area) * 0.5;
    let px = uv * size - origin;
    if any(px < vec2<f32>(0.0)) || any(px >= area) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let g = px / area * grid;
    let at = floor(g);
    var face = 6u;
    for (var i = 0u; i < 6u; i++) {
        if all(cell_of(i) == at) {
            face = i;
        }
    }
    if face == 6u {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    if (proj.mode.y & 4u) != 0u {
        // A one-pixel line round each face.
        let cell_px = area / grid;
        let f = (g - at) * cell_px;
        if any(f < vec2<f32>(1.0)) || any(f > cell_px - 1.0) {
            return vec4<f32>(0.5, 0.5, 0.5, 1.0);
        }
    }
    return sample_face(face, g - at);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var c: vec4<f32>;
    switch proj.mode.x {
        case 1u: { c = mosaic(in.uv); }
        case 3u: { c = cylinder(in.uv); }
        default: { c = fisheye(in.uv); }
    }
    return vec4<f32>(c.rgb, 1.0);
}
