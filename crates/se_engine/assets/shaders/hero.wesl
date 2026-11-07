// The Viltrumite mod's hero (viltrumite/draw.rs, spec F13): his own bone palette, one storage buffer
// written in place each frame. The meshes' transform is the identity and the palette is camera-relative,
// so he never goes through the scaled space. His light is the ground's (hero_light.wesl writes it once a
// frame); the pixels add only the normal's part, the street_surface highlight and the lamps. The opaque
// draws never discard, so the early depth test stays; cutouts (the hair) take `fragment_cutout` in the
// alpha-mask phase. His shadow is the same meshes again (`vertex_shadow`, `fragment_shadow`, the Multiply phase).

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::view_transformations::position_world_to_clip;
import super::ground_light::{PointLight, lit_surface};
import super::hero_frame::{HeroFrame, HeroLight};

const PI: f32 = 3.14159265358979;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> frame: HeroFrame;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> hero_light: HeroLight;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diff_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var diff_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var norm_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var norm_smp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var spec_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var spec_smp: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) @interpolate(flat) l_sun: vec3<f32>,
    @location(5) @interpolate(flat) l_sky: vec3<f32>,
    @location(6) @interpolate(flat) l_flat: vec3<f32>,
    @location(7) @interpolate(flat) haze_t: vec3<f32>,
    @location(8) @interpolate(flat) haze_a: vec3<f32>,
}

// The vertex's skinning matrix, as its three rows, by weight.
fn skin_rows(v: Vertex) -> array<vec4<f32>, 3> {
    let w = v.weights;
    let j0 = frame.joints[v.joints.x];
    let j1 = frame.joints[v.joints.y];
    let j2 = frame.joints[v.joints.z];
    let j3 = frame.joints[v.joints.w];
    return array<vec4<f32>, 3>(
        j0[0] * w.x + j1[0] * w.y + j2[0] * w.z + j3[0] * w.w,
        j0[1] * w.x + j1[1] * w.y + j2[1] * w.z + j3[1] * w.w,
        j0[2] * w.x + j1[2] * w.y + j2[2] * w.z + j3[2] * w.w,
    );
}

@vertex
fn vertex(v: Vertex) -> VertexOut {
    let rows = skin_rows(v);
    let r0 = rows[0];
    let r1 = rows[1];
    let r2 = rows[2];
    let p = vec4<f32>(v.position, 1.0);
    let world = vec3<f32>(dot(r0, p), dot(r1, p), dot(r2, p));
    var o: VertexOut;
    o.clip = position_world_to_clip(world);
    o.world = world;
    o.normal = vec3<f32>(dot(r0.xyz, v.normal), dot(r1.xyz, v.normal), dot(r2.xyz, v.normal));
    o.tangent = vec4<f32>(dot(r0.xyz, v.tangent.xyz), dot(r1.xyz, v.tangent.xyz), dot(r2.xyz, v.tangent.xyz), v.tangent.w);
    o.uv = v.uv;
    o.l_sun = hero_light.sun.rgb;
    o.l_sky = hero_light.sky.rgb;
    o.l_flat = hero_light.flat_e.rgb;
    o.haze_t = hero_light.haze_t.rgb;
    o.haze_a = hero_light.haze_a.rgb;
    return o;
}

fn shade(in: VertexOut, albedo: vec3<f32>, flip: f32) -> vec4<f32> {
    let up = frame.up.xyz;
    let l = normalize(frame.light.sun_dir.xyz);
    let normal = normalize(in.normal) * flip;
    var n = normal;
    if dot(in.tangent.xyz, in.tangent.xyz) > 1e-12 {
        // The normal map (RGB). Its green points down the image, as +v (DirectX's way; the curl of the maps
        // says so), and in these meshes that is cross(t, n) * w, as ship.wesl's.
        let bump = textureSample(norm_tex, norm_smp, in.uv).xyz * 2.0 - 1.0;
        let t = normalize(in.tangent.xyz - normal * dot(normal, in.tangent.xyz));
        let b = cross(t, normal) * in.tangent.w;
        n = normalize(t * bump.x + b * bump.y + normal * max(bump.z, 0.0));
    }
    let pl = PointLight(in.l_sun, in.l_sky, in.l_flat, in.haze_t, in.haze_a);
    var color = lit_surface(pl, l, albedo, n, up, 0.0, 0.0);
    // The further lights in space, by his own normals (near a body they are in the point's light).
    var free = vec3<f32>(0.0);
    for (var i = 0; i < 3; i++) {
        if frame.free_dir[i].w < 0.5 {
            break;
        }
        free += frame.free_color[i].rgb * max(dot(n, frame.free_dir[i].xyz), 0.0);
    }
    color += albedo / PI * free;
    // What shines: as the street objects (scatter_draw.wesl street_surface), by the suit's spec map.
    let strength = textureSample(spec_tex, spec_smp, in.uv).r;
    let v = normalize(-in.world);
    let h = normalize(l + v);
    let fres = pow(1.0 - max(dot(n, v), 0.0), 5.0);
    let gloss = 16.0 + 48.0 * strength;
    let spec = 0.04 + 0.3 * strength;
    let sheen = 0.02 + 0.04 * strength;
    let shine = (in.l_sun * pow(max(dot(n, h), 0.0), gloss) * max(dot(n, l), 0.0) * spec + in.l_sky * (1.0 + 2.0 * fres) * sheen) / PI;
    // By night the street lamps light him from above, as they do the road (its `lamp`) and the cars.
    let night = 1.0 - smoothstep(-0.12, 0.03, dot(up, l));
    let lamps = albedo * frame.town.rgb * night * (0.25 + 0.2 * max(dot(n, up), 0.0));
    // The heat of the air (F9): on the windward side at his skin's temperature, cooler round the back; in the main pass, so before bloom.
    var glow = vec3<f32>(0.0);
    if frame.heat.w > 0.0 {
        // Hot gas flows round him, so his outline shows it from any side; the suit's own colours keep him readable.
        let rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.5;
        let suit = 0.45 + 0.75 * dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
        glow = (mix(frame.heat_lee.rgb, frame.heat.rgb, smoothstep(-0.2, 0.9, dot(n, frame.heat_dir.xyz))) + frame.heat.rgb * rim) * suit;
        // F9's limit (tune.rs GLOW_HDR), keeping the colour.
        glow *= min(12.0 / max(max(glow.r, max(glow.g, glow.b)), 1e-6), 1.0);
    }
    return vec4<f32>(color + (shine + lamps + glow) * in.haze_t, 1.0);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, textureSample(diff_tex, diff_smp, in.uv).rgb, 1.0);
}

// The hair: cut by the map's alpha, both sides.
@fragment
fn fragment_cutout(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let diff = textureSample(diff_tex, diff_smp, in.uv);
    if diff.a < 0.5 {
        discard;
    }
    return shade(in, diff.rgb, select(-1.0, 1.0, front));
}

// His shadow: the vertex along the star onto the plane. A star on or under the plane's horizon, or no shadow, draws nothing.
@vertex
fn vertex_shadow(v: Vertex) -> @builtin(position) vec4<f32> {
    let rows = skin_rows(v);
    let p = vec4<f32>(v.position, 1.0);
    let world = vec3<f32>(dot(rows[0], p), dot(rows[1], p), dot(rows[2], p));
    let n = frame.shadow_plane.xyz;
    let l = normalize(frame.light.sun_dir.xyz);
    let denom = dot(n, l);
    if denom < 0.02 || frame.shadow.x <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    // Every vertex ends on the plane: along the star's ray, and what dips under the plane straight up to it. (Left where it is, its
    // triangles would lie on his legs, and the pixels' depth on the plane would darken them from in front.)
    let over = dot(n, world) + frame.shadow_plane.w;
    return position_world_to_clip(select(world - n * over, world - l * over / denom, over > 0.0));
}

struct ShadowFragment {
    // The Multiply phase: what's there times this.
    @location(0) factor: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

// Each pixel's factor once, however many triangles cover it: the depth is this pixel's ray on the plane, the same bits from any
// triangle, and the test is Greater, so the first wins. Through `main_pass_viewport`: under the render scale the pixels fill only a
// corner of the target (Bevy's own NDC helper goes by `view.viewport`, the whole of it).
@fragment
fn fragment_shadow(@builtin(position) coord: vec4<f32>) -> ShadowFragment {
    let vp = view.main_pass_viewport;
    let ndc = vec2<f32>((coord.x - vp.x) / vp.z * 2.0 - 1.0, 1.0 - (coord.y - vp.y) / vp.w * 2.0);
    let near = view.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let origin = view.world_position;
    let ray = near.xyz / near.w - origin;
    let n = frame.shadow_plane.xyz;
    let t = -(dot(n, origin) + frame.shadow_plane.w) / dot(n, ray);
    let clip = view.clip_from_world * vec4<f32>(origin + ray * t, 1.0);
    // The light the plane gets, without the star's: as the ground's `lit` term. Night fades it by itself, there's little star to take
    // away. Clouds and an eclipse take the sky light with the star's (`point_light_at`), so that ratio stays: it's their share of the
    // direct light that fades it, gone where nothing direct is left (the clouds' floor is a quarter: `cloud_shadow_at`).
    let sun = hero_light.sun.rgb * max(dot(n, normalize(frame.light.sun_dir.xyz)), 0.0);
    let rest = hero_light.sky.rgb + hero_light.flat_e.rgb + vec3<f32>(1e-6);
    let factor = rest / (sun + rest);
    let direct = clamp((hero_light.sun.w - 0.25) / 0.75, 0.0, 1.0);
    var o: ShadowFragment;
    o.factor = vec4<f32>(mix(vec3<f32>(1.0), factor, frame.shadow.x * direct), 1.0);
    o.depth = clip.z / clip.w;
    return o;
}
