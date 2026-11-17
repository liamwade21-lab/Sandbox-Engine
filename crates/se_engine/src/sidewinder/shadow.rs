//! The Sidewinder's shadow, on itself and on the world (ARCHITECTURE.md A6, T4e-1, T4e-1c). The engine has no shadow map, so
//! the mod makes its own: passes of its own draw the mod's own copy of the casters' triangles (`Casters`, made once at the first
//! spawn from the meshes `assets.rs` decoded; never Bevy's mesh buffers or draw functions) into maps orthographic along the
//! star's direction, in the ship's space, over a sphere round every caster in every pose of its moving parts
//! (`Casters::radius`), so the ship's own turning only turns the light's frame. The shaders: `sidewinder_shadow.wesl` (the
//! passes), `sidewinder_shadow_common.wesl` (the record and the map's frame they and the ship's material share) and the engine's
//! `moment_shadow.wesl` (Elite's filter and pick, which the world's receivers read the same maps through; its header says what is
//! Elite's).
//!
//! - **Casters:** every opaque family's draws (`casts`); never the glass, the overlays or the stamp. Two sets, as drawn
//!   (`draw.rs`): the outside set (the hull, the gear, the outside chair), which is the ship's true outline and so the caster
//!   for the world whatever the view, and the seat's (the cockpit, the seat's chair) for the cockpit's own surfaces while
//!   seated. Each vertex carries its rig node (its matrix this frame: `parts`, written in place when the pose changes; a node
//!   the machines hide is zero) and, for the seat's chair, its bone (posed by the palette the ship's material reads).
//! - **The softness is Elite's** (re/elite/notes/sidewinder_shadow.md; the study's build table). Elite fits a shadow map to each
//!   slice of the view (a cascade); the user's High `Profile_Space_Cockpit` has 1,536 texels a cascade and cascades ending at
//!   5, 80.8, 739.2 and 6,500 m (B). The mod keeps maps fitted to the ship, but one for each of Elite's first three cascades,
//!   each with Elite's moment texel for that cascade (`cascades`, the study's rule, C) in the projection the frame is drawn
//!   with (`fit_maps`: read after the seat has written its field of view), so its size is the sphere's width over that texel
//!   (`map_sizes`: 1,176, 73 and 8 texels seated at 3440 x 1440), plus the seat's own two maps of the cockpit set at the first
//!   two cascades' texels. Each is made as Elite makes a cascade's: the depth at twice the size, the moments at half of it,
//!   Elite's separable Gaussian over them (`gaussian`, `blur_radius`), then the lookup's B-spline. `EnableFrustum0Override`
//!   (true in the profile) is left out: what it does was not found (open; it may refit the first cascade to the ship).
//! - **Which map a pixel reads is Elite's pick** (its lit shader, FWD 182-231 and 578-589, C; `moment_shadow.wesl`
//!   `cascade_pick`): the first cascade whose box in light space holds the point, faded by the point's distance to that box's
//!   edge, blended with the next cascade holding it. The boxes are Elite's own (`Cascade::ahead`, `Cascade::half`: centred on
//!   each cascade's bounding sphere along the view, as wide as the fit makes them), handed to the shaders in the record; the
//!   maps stay fitted to the ship, only which one a pixel reads follows Elite. So a point straight ahead is the first
//!   cascade's alone to 12.6 m (faded into the second's by 13.9 m), the second's to 203 m, the third's to 1,860 m (at
//!   3440 x 1440, seated). Outside the third box no shadow is read: Elite's fourth cascade's texel (about 32 m) is wider than
//!   the ship. The seat's cockpit set takes the same pick over its two maps (it lies well inside the second box).
//! - **The textures:** every map is a square region of a moments texture (`REGIONS`), allocated once at each region's largest
//!   size; a map drawn smaller uses the region's corner, and the lookup scales its coordinates and keeps its taps inside it.
//!   The outside set's three are in the engine's texture (`volumetric_clouds::SHIP_SHADE_IMAGE`, allocated at start-up for
//!   every player, because a material's bind group is not rebuilt when an image changes in this Bevy), which the world's
//!   receivers bind; the seat's two in the mod's own (`ShipShadow::seat_moments`, made at the first spawn and kept for the
//!   session). A map Elite's texel would make larger than its region is drawn at the region's size: coarser than Elite's
//!   there, never finer (ours, for the memory). With the ship's 13.66 m sphere that is a view narrower than about 54.9 degrees
//!   at 16:9 or 44.2 degrees at 3440 x 1440 (the free camera's 45 degrees at 16:9 wants 1,899 texels): logged when it starts
//!   and ends.
//! - **(a) The ship's surfaces:** `sidewinder_lit.wesl` `star_shadow` multiplies the star's light (only the star's) by the
//!   filter's answer at the pixel: bindings 20-22 of `ShipMaterial` (binding 20 the texture of the material's set; no new
//!   pipeline variant). They read the sandbox's light record without the ship's shadow on the world (`PropLight::ship_light`),
//!   so they are not shadowed twice.
//! - **(b) Everything else:** the shadow is a factor on the star's direct light inside each receiver's own lighting
//!   (`ground_light.wesl` `ship_shade_at`: the ground per pixel, the scatter's objects once an instance, the hero, the props, the
//!   weapon and the skater at their light pass's one point), through the outside set's maps and the same filter and pick
//!   (`moment_shadow.wesl`). The mod fills the engine's record (`volumetric_clouds::ShipShade`, `shade`) each frame the maps are
//!   drawn and zeros it otherwise; the map pass sets its `live` once this frame's maps are drawn. Ours and known: a tree or a
//!   building is shadowed as a whole from one point, every prop follows the player's point, and nothing between two parts of
//!   the ship along the star's ray is special (the ground under a wing is simply shadowed by the maps).
//! - **When it runs:** only while a ship is drawn (`draw::ShipDraw::placed`: it exists and is within its draw distance, not in
//!   the tunnel) and the sandbox's light record is on; nothing at all before the first spawn (every system waits for
//!   `ShipShadow`). Whether the star lights the ship's place (night, an eclipse) is known only on the GPU (the light record's
//!   star term, worked out by the hero's light pass from the sandbox's record, whose CPU half the sandbox keeps private): there
//!   the passes are recorded but draw nothing (every vertex collapses, `star_lit`): the moments keep the last lit frame's (or
//!   zeros, if the star has not lit the ship since the spawn), which only a star term of 0 reads (the filter's answer is always
//!   0..1, never NaN: its saturations are D3D's, `sat`), and the first lit frame makes them anew before anything reads them. The
//!   records' `live` (the ship's and the world's) is 0 each frame until the map pass has drawn this frame's maps, so a frame
//!   without them (the pipelines not built yet, no ship) never reads an old one. `SE_SHIP_SHADOW=0` leaves it all out, for
//!   comparisons (the ship then reads lit, the world's record stays zeros); `SE_SHIP_SHADOW_CUTOFF=X` gives the light-bleeding
//!   cutoff another value, for comparisons.
//!
//! Pipelines: 3, each built once at the first spawn (the casters' depth, the moments' resolve, the blur), none where the GPU
//! can't use the moments' format (`volumetric_clouds::ship_shade_format` is `None`: the shadow is then off for the session and
//! the ship reads lit). The ship's materials keep their variants; the receivers' are the engine's, built at start.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{DMat4, UVec4, Vec4};
use bevy::mesh::{VertexAttributeValues, VertexBufferLayout};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{storage_buffer_read_only_sized, texture_2d, texture_depth_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::texture::GpuImage;
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderSystems};
use se_core::elite_rig::{self, Values};

use crate::camera::MainCamera;
use crate::sandbox::light::PropLight;
use crate::viltrumite::draw::MainView;
use crate::volumetric_clouds::{self as engine, CloudShadowAssets, ShipShade, SHIP_SHADE_IMAGE, SHIP_SHADE_TEXELS};

use super::assets::ShipRig;
use super::draw::ShipDraw;
use super::material::{self, Family, ShipPalette, ATTRIBUTE_BONE};
use super::parts::Parts;

// ---- Elite's directional shadow settings (re/elite/notes/sidewinder_shadow.md) ----

/// `Profile_Space_Cockpit` of `DirectionalShadows_High` (B: the user's `DirectionalShadowQuality` 3): the texels a cascade's
/// cell, the cascades' ends (m), the crossfade fractions inside and for the last.
const CELL_RESOLUTION: f32 = 1536.0;
pub const CASCADE_ENDS: [f32; 4] = [5.0, 80.8, 739.2, 6500.0];
const INTERNAL_TRANSITION: f32 = 0.15;
const EXTERNAL_TRANSITION: f32 = 0.2;
/// The fit's field of view is the engine's plus 2 degrees, re-centred when the engine's leaves [stored - 4, stored] (C:
/// `DAT_145e74038`, `FUN_1405b40c0`; taken as vertical: E).
const FIT_EXTRA_DEG: f32 = 2.0;
const FIT_HYSTERESIS_DEG: f32 = 4.0;
/// A cascade's bounding sphere's radius squared is grown by this (C: `DAT_1450bd21c`, 1.05 squared).
const SPHERE_GROWTH: f32 = 1.1025;
/// The cascades the ship's maps follow: the first three (the fourth's texel is wider than the ship).
const CASCADES: usize = 3;
/// The moment bias and the light-bleeding cutoff (C: `FUN_1428903f0`, 3.0 x 1e-5 and 0.3), and the resolve depth bias when the
/// depth becomes moments (C: 0.1 x 0.001, `c_resolveDepthBias`, in the depth map's 0..1: here the map's own range, E).
const MOMENT_BIAS: f32 = 3.0e-5;
const LIGHT_BLEEDING_CUTOFF: f32 = 0.3;
const RESOLVE_DEPTH_BIAS: f32 = 1.0e-4;
/// Elite's dequantisation (FWD 345-349): b = READ x (q - (OFFSET, 0, 0, 0)), row by row.
const MOMENT_OFFSET: f32 = 0.035956;
const MOMENT_READ: [[f32; 4]; 4] = [
    [0.222774, 0.077197, 0.792699, 0.031942],
    [0.154968, 0.139463, 0.796342, -0.172282],
    [0.145199, 0.212020, 0.725869, -0.275801],
    [0.163127, 0.259143, 0.653909, -0.337613],
];

/// The separable blur's radius for cascade `k` (C: clamp(2 - k, 1, 4), `DAT_145e7403c`).
pub fn blur_radius(k: usize) -> u32 {
    (2 - k as i32).clamp(1, 4) as u32
}

/// Elite's Gaussian kernel of radius `r` (C: `FUN_1405b8840`, w(k) = exp(-0.5 k^2 / (0.5 r)^2), k = -r..r, normalised), its
/// weights at offsets 0, 1 and 2 (0 past the radius).
pub fn gaussian(r: u32) -> [f32; 3] {
    let w = |k: i32| (-0.5 * (k * k) as f32 / (0.5 * r as f32).powi(2)).exp();
    let sum: f32 = (-(r as i32)..=r as i32).map(w).sum();
    [0, 1, 2].map(|k| if k <= r as i32 { w(k) / sum } else { 0.0 })
}

/// The floor of every crossfade fraction, and the edge band in which a cascade is not read at all (C: `FUN_1405b2880` stores
/// 4 / res, max(internal, 4 / res), max(external, 4 / res)): one moment texel in the box's half-width units.
const FADE_FLOOR: f32 = 4.0 / CELL_RESOLUTION;

/// A cascade's crossfade fraction (C: InternalTransition for the inner ones, ExternalTransition for the last; at least
/// `FADE_FLOOR`).
fn crossfade(k: usize) -> f32 {
    if k + 1 == CASCADE_ENDS.len() { EXTERNAL_TRANSITION } else { INTERNAL_TRANSITION }.max(FADE_FLOOR)
}

/// One of Elite's cascades as its fit makes it for a view: its box in light space and its moment texel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cascade {
    /// How far ahead of the camera along the view the box's middle lies (m): the cascade's bounding sphere's centre.
    pub ahead: f32,
    /// The box's half-width across the light (m).
    pub half: f32,
    /// The moment texel (m) the lookup's B-spline reads.
    pub texel: f32,
}

/// Elite's cascades for a view whose fit has the vertical field of view `fit_fov` (radians: the engine's plus 2 degrees,
/// `FitFov`) and the aspect `aspect` (C, notes section 2): each cascade [n, f] is bounded by a sphere (K = 1 + tan^2 hx +
/// tan^2 hy, centre zc = min(f, (n + f) / 2 K) along the view, r^2 = 1.1025 max(zc^2 - 2 zc n + K n^2, zc^2 - 2 zc f + K
/// f^2)); its box's half-width h = r res / (res - 2) / (1 - s), s its crossfade; the depth texel 2h / res and the moments at
/// half that resolution, so the moment texel is 4h / res. (Elite snaps the box's middle to 2 depth texels in its world, under
/// one moment texel: left out, ours.)
pub fn cascades(fit_fov: f32, aspect: f32) -> [Cascade; 4] {
    let ty = (fit_fov * 0.5).tan();
    let tx = ty * aspect;
    let k = 1.0 + tx * tx + ty * ty;
    let mut near = 0.0;
    std::array::from_fn(|i| {
        let (n, f) = (near, CASCADE_ENDS[i]);
        near = f;
        let zc = f.min((n + f) * 0.5 * k);
        let r = (SPHERE_GROWTH * (zc * zc - 2.0 * zc * n + k * n * n).max(zc * zc - 2.0 * zc * f + k * f * f)).sqrt();
        let h = r * CELL_RESOLUTION / (CELL_RESOLUTION - 2.0) / (1.0 - crossfade(i));
        Cascade { ahead: zc, half: h, texel: 4.0 * h / CELL_RESOLUTION }
    })
}

/// The fit's field of view with Elite's hysteresis (`FIT_EXTRA_DEG`, `FIT_HYSTERESIS_DEG`).
#[derive(Clone, Copy, Debug, Default)]
pub struct FitFov(Option<f32>);

impl FitFov {
    /// The fit's vertical field of view (radians) for the view's `fov` this frame.
    pub fn fit(&mut self, fov: f32) -> f32 {
        let (extra, band) = (FIT_EXTRA_DEG.to_radians(), FIT_HYSTERESIS_DEG.to_radians());
        let stored = match self.0 {
            Some(s) if fov <= s && fov >= s - band => s,
            _ => fov + extra,
        };
        self.0 = Some(stored);
        stored
    }
}

// ---- The maps ----

/// The maps: the outside set's at Elite's cascades 0, 1, 2, then the seat's (the cockpit set) at cascades 0 and 1 (nothing of
/// it lies past 80.8 m from the eye, so no third).
const MAPS: usize = 5;
/// Each map's cascade.
const MAP_CASCADE: [usize; MAPS] = [0, 1, 2, 0, 1];
/// The seat's first map (its second follows).
const SEAT_MAP: usize = 3;
/// The maps' regions, (x, y, the largest size), in moment texels of their texture (`MAP_TEXTURE`), in the maps' order. Ours:
/// the first cascade's room holds Elite's texel for the seat's field of view at 16:9 (1,493 texels over the 13.66 m sphere) and
/// every view wider than about 54.9 degrees at 16:9 or 44.2 at 3440 x 1440; the others several times the seat's need. (Their
/// sizes as before the outside set moved into the engine's texture: only the seat's corners moved, into the mod's own.)
const REGIONS: [(u32, u32, u32); MAPS] = [(0, 0, 1536), (0, 1536, 256), (256, 1536, 64), (0, 0, 1536), (0, 1536, 256)];
/// Each map's texture: 0 the engine's (the outside set's, `SHIP_SHADE_IMAGE`, which the world's receivers read), 1 the mod's
/// own (the seat's, `ShipShadow::seat_moments`).
const MAP_TEXTURE: [usize; MAPS] = [0, 0, 0, 1, 1];
/// Both textures' size (the engine's is `SHIP_SHADE_TEXELS`; the mod's the same, for the seat's two regions); their format is
/// the engine's (four 16-bit UNORM channels, C: engine format 0x29, `R16G16B16A16un`: `volumetric_clouds::ship_shade_format`).
const ATLAS: (u32, u32) = SHIP_SHADE_TEXELS;
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
/// The casters' depth target: twice the largest region; the blur's scratch: the largest region.
const DEPTH_TEXELS: u32 = 3072;
const SCRATCH_TEXELS: u32 = 1536;
/// The sphere's radius past the furthest caster vertex in the poses sampled (ours): covers the seat's chair's motion (its
/// vertices are taken in the bind pose) and the moving parts' arcs between the samples.
const MARGIN_M: f32 = 0.25;
/// The moving parts' travel, sampled between their still poses (ours).
const POSE_SAMPLES: usize = 16;
/// The depth (the maps' 0..1) a receiver of the world beyond the maps' sphere is compared at: halfway into the margin, past
/// every caster and short of the empty depth, 1 (ours).
fn deepest(radius: f32) -> f32 {
    1.0 - 0.5 * MARGIN_M / (2.0 * radius)
}

/// A vertex not posed by the palette (`sidewinder_shadow.wesl` `NO_BONE`).
pub const NO_BONE: u32 = u32::MAX;

/// The lookup's offset along the normal at grazing light, in a map's moment texels (ours, by geometry): how far the filter
/// reaches either side of the point (the B-spline's 2 texels, the blur's radius, and the half texel of the half-resolution
/// moments rounded up to 1), so a surface does not read its own depth spread by the filter as a caster. It is scaled by the
/// sine of the light's angle to the normal in the lookup (a surface facing the light gets none).
fn normal_reach(radius: u32) -> f32 {
    2.0 + radius as f32 + 1.0
}

/// Each map's moment size (texels a side) for Elite's cascades `cascades` over a sphere of radius `radius`: its width over the
/// cascade's texel, at least 4, at most its region; and whether a region capped one (the map then coarser than Elite's).
pub fn map_sizes(cascades: &[Cascade; 4], radius: f32) -> ([u32; MAPS], bool) {
    let wanted: [u32; MAPS] = std::array::from_fn(|m| ((2.0 * radius / cascades[MAP_CASCADE[m]].texel).ceil() as u32).max(4));
    let capped = (0..MAPS).any(|m| wanted[m] > REGIONS[m].2);
    (std::array::from_fn(|m| wanted[m].min(REGIONS[m].2)), capped)
}

/// Does a draw of this family cast a shadow? The opaque families only.
pub fn casts(family: Family) -> bool {
    !matches!(family, Family::Overlay | Family::Stamp) && !family.is_glass()
}

/// One caster vertex as the map pass reads it: its place in its node's model space, the node, the bone (`NO_BONE`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CasterVertex {
    pub pos: [f32; 3],
    pub node: u32,
    pub bone: u32,
}

/// The mod's own copy of the casters' triangles: one vertex list, one index list holding the outside set's triangles then the
/// seat's, and the sphere the maps cover.
pub struct Casters {
    pub vertices: Vec<CasterVertex>,
    pub indices: Vec<u32>,
    pub outside: Range<u32>,
    pub seat: Range<u32>,
    /// The maps' middle in the ship's space, and their radius (m).
    pub centre: Vec3,
    pub radius: f32,
}

impl Casters {
    /// From the meshes `assets.rs` made (a mesh per drawn draw of each model, `ShipRig::models`' order) and the drawn
    /// materials resolved: every node's opaque draws, its set's by `Group::inside`.
    pub fn new(rig: &ShipRig, meshes: &[Vec<Mesh>], materials: &HashMap<String, material::Resolved>) -> Casters {
        let mut vertices = Vec::new();
        let (mut outside, mut seat) = (Vec::new(), Vec::new());
        for (node, n) in rig.nodes.iter().enumerate() {
            let Some(model) = n.model else { continue };
            for (key, mesh) in rig.models[model].draws.iter().zip(&meshes[model]) {
                if !materials.get(key).is_some_and(|r| casts(r.family)) {
                    continue;
                }
                let (Some(VertexAttributeValues::Float32x3(pos)), Some(indices)) = (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.indices()) else { continue };
                let bones = match mesh.attribute(ATTRIBUTE_BONE) {
                    Some(VertexAttributeValues::Uint32(b)) if rig.models[model].skinned => Some(b),
                    _ => None,
                };
                let first = vertices.len() as u32;
                vertices.extend(pos.iter().enumerate().map(|(i, p)| CasterVertex { pos: *p, node: node as u32, bone: bones.and_then(|b| b.get(i).copied()).unwrap_or(NO_BONE) }));
                let set = if n.group.inside() { &mut seat } else { &mut outside };
                set.extend(indices.iter().map(|i| first + i as u32));
            }
        }
        let outside_len = outside.len() as u32;
        let indices: Vec<u32> = outside.into_iter().chain(seat).collect();
        let (centre, radius) = sphere(rig, &vertices);
        Casters { outside: 0..outside_len, seat: outside_len..indices.len() as u32, vertices, indices, centre, radius }
    }
}

/// Two poses' values mixed: `t` 0 `a`, 1 `b` (a value only one has is kept).
fn between(a: &Values, b: &Values, t: f64) -> Values {
    let mut v = a.clone();
    for (k, y) in b {
        let x = a.get(k).copied().unwrap_or(*y);
        v.insert(k.clone(), x + (y - x) * t);
    }
    v
}

/// Every node's matrix in the poses the moving parts pass through: the gear's travel from down to up (`samples` + 1 poses),
/// and, the gear down and up, every machine's every state (the hardpoints' doors, the vents' flaps, the canopy...) and the
/// way there from the still pose, sampled the same.
fn rig_poses(rig: &ShipRig, samples: usize) -> Vec<Vec<Mat4>> {
    let (down, up) = (rig.static_pose(true), rig.static_pose(false));
    let mut values: Vec<Values> = (0..=samples).map(|i| between(&down, &up, i as f64 / samples as f64)).collect();
    for still in [&down, &up] {
        for state in rig.ship.machines.iter().flat_map(|m| &m.states) {
            let mut there = still.clone();
            there.extend(elite_rig::state_pose(state));
            values.extend((1..=samples).map(|i| between(still, &there, i as f64 / samples as f64)));
        }
    }
    values.iter().filter_map(|v| rig.pose(v).ok()).collect()
}

/// The sphere round every caster vertex in every pose sampled, `MARGIN_M` wider: each node's vertices bounded once by a sphere
/// in its own space, carried by the node's matrix in each pose (its radius by the matrix's largest scale), so the poses cost
/// a node each, not a vertex (a little wider than the vertices' own; the maps keep Elite's texel whatever their width).
fn sphere(rig: &ShipRig, vertices: &[CasterVertex]) -> (Vec3, f32) {
    let mut boxes: HashMap<u32, (Vec3, Vec3)> = HashMap::new();
    for v in vertices {
        let p = Vec3::from_array(v.pos);
        let b = boxes.entry(v.node).or_insert((p, p));
        *b = (b.0.min(p), b.1.max(p));
    }
    let local: Vec<(usize, Vec3, f32)> = boxes
        .iter()
        .map(|(&n, &(lo, hi))| {
            let c = (lo + hi) * 0.5;
            let r = vertices.iter().filter(|v| v.node == n).map(|v| Vec3::from_array(v.pos).distance(c)).fold(0.0f32, f32::max);
            (n as usize, c, r)
        })
        .collect();
    let placed: Vec<(Vec3, f32)> = rig_poses(rig, POSE_SAMPLES)
        .iter()
        .flat_map(|pose| local.iter().map(move |&(n, c, r)| (pose[n].transform_point3(c), r * pose[n].x_axis.truncate().length().max(pose[n].y_axis.truncate().length()).max(pose[n].z_axis.truncate().length()))))
        .collect();
    let (lo, hi) = placed.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &(c, r)| (lo.min(c - r), hi.max(c + r)));
    let centre = (lo + hi) * 0.5;
    let reach = placed.iter().map(|&(c, r)| c.distance(centre) + r).fold(0.0f32, f32::max);
    (centre, reach + MARGIN_M)
}

/// The rows of Elite's read matrix's inverse: the write's (`quantised_moments`).
fn moment_write() -> [Vec4; 4] {
    let read = DMat4::from_cols_array_2d(&MOMENT_READ.map(|r| r.map(f64::from))).transpose();
    let w = read.inverse();
    [0, 1, 2, 3].map(|i| w.row(i).as_vec4())
}

/// The quantised moments of depth `d` (the shader's `quantised_moments`, for the tests).
#[cfg(test)]
fn quantised(d: f32) -> Vec4 {
    let w = moment_write();
    let b = Vec4::new(d, d * d, d * d * d, d * d * d * d);
    Vec4::new(w[0].dot(b), w[1].dot(b), w[2].dot(b), w[3].dot(b)) + Vec4::new(MOMENT_OFFSET, 0.0, 0.0, 0.0)
}

/// `sidewinder_shadow_common.wesl` `ShipShadow`.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct ShadowRecord {
    live: UVec4,
    to_ship: [Vec4; 3],
    sphere: Vec4,
    msm: Vec4,
    read: [Vec4; 4],
    write: [Vec4; 4],
    atlas: Vec4,
    maps: [Vec4; MAPS],
    ahead: Vec4,
    half: Vec4,
    fade: Vec4,
    offsets: [Vec4; 2],
}

impl ShadowRecord {
    fn new(centre: Vec3, radius: f32) -> ShadowRecord {
        ShadowRecord {
            live: UVec4::ZERO,
            to_ship: [Vec4::X, Vec4::Y, Vec4::Z],
            sphere: centre.extend(radius),
            msm: Vec4::new(MOMENT_OFFSET, MOMENT_BIAS, LIGHT_BLEEDING_CUTOFF, RESOLVE_DEPTH_BIAS),
            read: MOMENT_READ.map(Vec4::from_array),
            write: moment_write(),
            atlas: Vec4::new(ATLAS.0 as f32, ATLAS.1 as f32, 1.0 / ATLAS.0 as f32, 1.0 / ATLAS.1 as f32),
            maps: [Vec4::ZERO; MAPS],
            ahead: Vec4::ZERO,
            half: Vec4::ONE,
            // (Elite's `gDirectionalShadow_FadeThresholds` as the lit shader reads it, cb1[326]: the count, then the three values
            // `FUN_1405b2880` stores, in its order (E: which is which by that order).)
            fade: Vec4::new(CASCADE_ENDS.len() as f32, FADE_FLOOR, crossfade(0), crossfade(CASCADE_ENDS.len() - 1)),
            offsets: [Vec4::ZERO; 2],
        }
    }
}

/// The light's axes in the ship's space for the star towards `l` (`sidewinder_shadow_common.wesl` `light_axes`, the same rule):
/// across (the map's x), across (its y).
fn light_axes(l: Vec3) -> (Vec3, Vec3) {
    let helper = if l.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
    let u = helper.cross(l).normalize();
    (u, l.cross(u))
}

/// The world's record (`volumetric_clouds::ShipShade`) for this frame's maps `r`: the ship drawn at `tf` (its space to the
/// render frame, its scale the scaled space's: `draw::ShipDraw::placed`), the star towards `sun` and the camera looking along
/// `view` (both the render frame's axes). A receiver's place is in metres from the camera, so the ship's place is taken without
/// the scale (`draw.rs` `place` multiplies the metres by it). `live` is left 0: the map pass sets it once the maps are drawn.
fn world_shade(r: &ShadowRecord, tf: &Transform, sun: Vec3, view: Vec3) -> ShipShade {
    let k = tf.scale.x.max(1e-30);
    let to_ship = Mat4::from_rotation_translation(tf.rotation, tf.translation / k).inverse();
    let l = to_ship.transform_vector3(sun).normalize_or(Vec3::Y);
    let (u, v) = light_axes(l);
    ShipShade {
        live: UVec4::ZERO,
        to_ship_x: to_ship.row(0),
        to_ship_y: to_ship.row(1),
        to_ship_z: to_ship.row(2),
        axis_u: u.extend(0.0),
        axis_v: v.extend(0.0),
        axis_l: l.extend(0.0),
        sphere: r.sphere,
        eye: to_ship.transform_point3(Vec3::ZERO).extend(1.0),
        view: to_ship.transform_vector3(view).normalize_or(Vec3::NEG_Z).extend(0.0),
        map0: r.maps[0],
        map1: r.maps[1],
        map2: r.maps[2],
        texels: r.atlas,
        ahead: r.ahead,
        half: r.half,
        fade: r.fade,
        msm: r.msm.truncate().extend(deepest(r.sphere.w)),
        read0: r.read[0],
        read1: r.read[1],
        read2: r.read[2],
        read3: r.read[3],
    }
}

/// `sidewinder_shadow.wesl` `MomentPass`: one pass of the making of a map's moments.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct MomentPass {
    src: Vec4,
    dst: Vec4,
    kernel: Vec4,
}

/// The passes of the maps (`3 m`: the resolve, `3 m + 1`: the blur across, `3 m + 2`: down).
const PASSES: usize = 3 * MAPS;

/// The maps' regions as the record holds them, their offsets along the normal, and the passes that make them, for the moment
/// sizes `sizes` and Elite's cascades `cascades`.
fn layout(sizes: [u32; MAPS], cascades: &[Cascade; 4], radius: f32) -> ([Vec4; MAPS], [Vec4; 2], [MomentPass; PASSES]) {
    let mut maps = [Vec4::ZERO; MAPS];
    let mut offsets = [0.0; 8];
    let mut passes = [MomentPass::default(); PASSES];
    for m in 0..MAPS {
        let (x, y, _) = REGIONS[m];
        let n = sizes[m] as f32;
        let cascade = MAP_CASCADE[m];
        let r = blur_radius(cascade);
        // (The map's own texel: Elite's, or coarser where the region capped it.)
        let texel = (2.0 * radius / n).max(cascades[cascade].texel);
        maps[m] = Vec4::new(x as f32, y as f32, n, texel);
        // The offset along the normal is ours (Elite's lit shader has none: its depth bias is the resolve's alone). It grows
        // with the texel: at grazing light about 0.12 m on the first map, 1.5 m on the second and 13.7 m on the third seated
        // at 3440 x 1440 (1.27 m and 11.6 m walking at 1600 x 900), so far away a surface the light grazes loses its shadow
        // on itself within that reach of a caster.
        offsets[m] = normal_reach(r) * texel;
        let [k0, k1, k2] = gaussian(r);
        let kernel = Vec4::new(k0, k1, k2, 0.0);
        let region = Vec2::new(x as f32, y as f32);
        passes[3 * m] = MomentPass { src: Vec4::new(0.0, 0.0, n, 0.0), dst: region.extend(0.0).extend(0.0), kernel: Vec4::ZERO };
        passes[3 * m + 1] = MomentPass { src: region.extend(n).extend(1.0), dst: Vec4::new(0.0, 0.0, r as f32, 0.0), kernel };
        passes[3 * m + 2] = MomentPass { src: Vec4::new(0.0, 0.0, n, 2.0), dst: region.extend(r as f32).extend(0.0), kernel };
    }
    (maps, [Vec4::from_slice(&offsets[..4]), Vec4::from_slice(&offsets[4..])], passes)
}

/// The shadow's state: the casters, the textures and buffers the passes and the ship's material bind, and this frame's record and
/// passes. Made at the first spawn (`new`, from `draw::make`); extracted each frame it changes.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct ShipShadow {
    casters: Arc<Casters>,
    /// The outside set's moments (the engine's texture, `SHIP_SHADE_IMAGE`: the maps' first three `REGIONS`) and the seat's
    /// (the mod's own, made once at the first spawn and kept: the last two), drawn only by the render world.
    pub moments: Handle<Image>,
    pub seat_moments: Handle<Image>,
    format: TextureFormat,
    /// The record (`ShadowRecord`), written only by the render world (`values`, then `live` by the map pass).
    pub record: Handle<ShaderBuffer>,
    /// Each rig node's matrix in the ship's space (zero for a hidden node), written in place when the pose changes.
    parts: Handle<ShaderBuffer>,
    palette: Handle<ShaderBuffer>,
    frame: Handle<ShaderBuffer>,
    /// The sandbox's light without the ship's shadow on the world (`PropLight::ship_light`): whether the star lights the ship.
    light: Handle<ShaderBuffer>,
    values: ShadowRecord,
    passes: [MomentPass; PASSES],
    /// Each map's moment size this frame (`map_sizes`).
    sizes: [u32; MAPS],
    /// The maps are drawn this frame (a ship is drawn and the light record is on), and the seat's too (seated).
    active: bool,
    seated: bool,
    nodes: usize,
    /// The parts' revisions last written (`Parts::revision`, `shown_revision`).
    written: Option<(u64, u64)>,
    /// The fit's field of view, kept from frame to frame (Elite's hysteresis).
    fit: FitFov,
    /// A region capped a map this frame (`map_sizes`): logged when it starts and when it ends.
    capped: bool,
    /// `SE_SHIP_SHADOW=0` leaves the shadow out (comparisons), as a GPU that can't use the moments' format does: nothing runs
    /// (no pipeline, no target, no record live) and the ship reads lit.
    enabled: bool,
}

impl ShipShadow {
    /// (`_device` is the caller's: the format is the engine's start-up decision, `volumetric_clouds::ship_shade_format`.)
    pub fn new(images: &mut Assets<Image>, buffers: &mut Assets<ShaderBuffer>, light: &PropLight, palette: &ShipPalette, casters: Casters, nodes: usize, _device: Option<&RenderDevice>) -> ShipShadow {
        // (The engine's texture's format, so the passes draw both textures alike; none where this GPU can't use it: the seat's
        // texture is then a 1 x 1 nothing draws, and the shadow stays off.)
        let usable = engine::ship_shade_format();
        let format = usable.unwrap_or(TextureFormat::Rgba8Unorm);
        let mut seat = match usable {
            Some(f) => {
                let mut seat = Image::new_uninit(Extent3d { width: ATLAS.0, height: ATLAS.1, depth_or_array_layers: 1 }, TextureDimension::D2, f, RenderAssetUsages::RENDER_WORLD);
                seat.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT;
                seat
            }
            None => Image::new_fill(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, &[255; 4], format, RenderAssetUsages::RENDER_WORLD),
        };
        // (Elite's ClampSampler: linear, clamped; the gathers read it unfiltered.)
        seat.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        let mut values = ShadowRecord::new(casters.centre, casters.radius);
        // (`SE_SHIP_SHADOW_CUTOFF=X` gives the light-bleeding cutoff another value, for comparisons only.)
        if let Some(cutoff) = std::env::var("SE_SHIP_SHADOW_CUTOFF").ok().and_then(|v| v.parse::<f32>().ok()) {
            warn!("sidewinder: the shadow's light-bleeding cutoff {cutoff} (a comparison; Elite's is {LIGHT_BLEEDING_CUTOFF})");
            values.msm.z = cutoff.clamp(0.0, 0.99);
        }
        info!(
            "sidewinder: the shadow's casters: {} triangles outside, {} seated, {} vertices, in a sphere of {:.2} m round ({:.2}, {:.2}, {:.2}); moments {format:?}",
            casters.outside.len() / 3,
            casters.seat.len() / 3,
            casters.vertices.len(),
            casters.radius,
            casters.centre.x,
            casters.centre.y,
            casters.centre.z,
        );
        ShipShadow {
            casters: Arc::new(casters),
            moments: SHIP_SHADE_IMAGE,
            seat_moments: images.add(seat),
            format,
            record: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&values), RenderAssetUsages::RENDER_WORLD)),
            parts: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&vec![Mat4::IDENTITY; nodes.max(1)]), RenderAssetUsages::default())),
            palette: palette.buffer.clone(),
            frame: light.frame.clone(),
            light: light.ship_light.clone(),
            values,
            passes: [MomentPass::default(); PASSES],
            sizes: [0; MAPS],
            active: false,
            seated: false,
            nodes,
            written: None,
            fit: FitFov::default(),
            capped: false,
            enabled: usable.is_some() && std::env::var("SE_SHIP_SHADOW").map_or(true, |v| v != "0"),
        }
    }
}

/// This frame's state from the ship as drawn (`draw::place` ran): active, seated, the ship's space, the parts' matrices.
fn update(draw: Res<ShipDraw>, parts: Res<Parts>, light: Res<PropLight>, mut shadow: ResMut<ShipShadow>, mut buffers: ResMut<Assets<ShaderBuffer>>, frame: Res<bevy::diagnostic::FrameCount>) {
    let placed = draw.placed().filter(|_| light.on && shadow.enabled);
    let (active, seated) = (placed.is_some(), placed.is_some() && draw.seat_shown());
    if (active, seated) != (shadow.active, shadow.seated) {
        let what = match (active, seated) {
            (false, _) => "stop (no ship drawn)",
            (true, false) => "run (the outside set's)",
            (true, true) => "run (the outside set's and the seat's)",
        };
        info!("sidewinder: the shadow maps {what} from frame {}", frame.0);
        shadow.active = active;
        shadow.seated = seated;
    }
    let Some(tf) = placed else { return };
    let revisions = (parts.revision(), parts.shown_revision());
    if shadow.written != Some(revisions) {
        let matrices: Vec<Mat4> = (0..shadow.nodes.max(1)).map(|n| if parts.shown(n) { parts.matrix(n) } else { Mat4::ZERO }).collect();
        material::write_in_place(&mut buffers, &shadow.parts, &matrices);
        shadow.written = Some(revisions);
    }
    let inv = Mat4::from_scale_rotation_translation(tf.scale, tf.rotation, tf.translation).inverse();
    shadow.values.to_ship = [inv.row(0), inv.row(1), inv.row(2)];
}

/// Elite's cascades for the projection this frame is drawn with, and the maps' sizes, regions and passes from them. It runs
/// after `CameraUpdateSystems`, so the projection is the one the view's matrices are made from: the seat's field of view
/// (`seat::seat_fov` writes it inside that set: the head look's and a side panel's channel included) and the aspect Bevy
/// fits to the viewport there; before it the projection still holds the flight's field of view that
/// `sync_camera_transform` resets every frame (59.84 degrees seated, where the seat draws 56.249).
fn fit_maps(camera: Query<&Projection, With<MainCamera>>, mut shadow: ResMut<ShipShadow>) {
    if !shadow.active {
        return;
    }
    let (fov, aspect) = match camera.iter().next() {
        Some(Projection::Perspective(p)) => (p.fov, p.aspect_ratio),
        _ => (std::f32::consts::FRAC_PI_4, 16.0 / 9.0),
    };
    let fit = shadow.fit.fit(fov);
    let cascades = cascades(fit, aspect);
    let (sizes, capped) = map_sizes(&cascades, shadow.casters.radius);
    if sizes != shadow.sizes {
        info!(
            "sidewinder: the shadow's maps {:?} texels outside, {:?} seated (Elite's moment texels {:.4}, {:.3}, {:.2} m, its boxes {:.1}, {:.1}, {:.0} m wide either side, for the drawn view's {:.3} degrees, the fit's {:.3}, aspect {aspect:.3})",
            &sizes[..CASCADES],
            &sizes[SEAT_MAP..],
            cascades[0].texel,
            cascades[1].texel,
            cascades[2].texel,
            cascades[0].half,
            cascades[1].half,
            cascades[2].half,
            fov.to_degrees(),
            fit.to_degrees()
        );
        shadow.sizes = sizes;
    }
    if capped != shadow.capped {
        if capped {
            info!("sidewinder: the shadow's maps are capped by their regions from here (the view's {:.3} degrees wants finer: the maps are coarser than Elite's)", fov.to_degrees());
        } else {
            info!("sidewinder: the shadow's maps are no longer capped (the view's {:.3} degrees)", fov.to_degrees());
        }
        shadow.capped = capped;
    }
    let (maps, offsets, passes) = layout(sizes, &cascades, shadow.casters.radius);
    shadow.values.maps = maps;
    shadow.values.offsets = offsets;
    shadow.values.ahead = Vec4::from_array(cascades.map(|c| c.ahead));
    shadow.values.half = Vec4::from_array(cascades.map(|c| c.half));
    shadow.passes = passes;
}

/// The world's record (`volumetric_clouds::ShipShade`) this frame: filled while the maps are drawn (`active`: a ship drawn, not
/// in the tunnel, the light record on, `SE_SHIP_SHADOW` not 0), zeros otherwise; logged when it starts and stops. It runs after
/// the maps are fitted and the sandbox's record has its star, before the engine copies it into the ground's record.
fn shade_world(
    draw: Res<ShipDraw>,
    light: Res<PropLight>,
    shadow: Res<ShipShadow>,
    camera: Query<&Transform, With<MainCamera>>,
    mut out: ResMut<ShipShade>,
    frame: Res<bevy::diagnostic::FrameCount>,
) {
    let view = camera.iter().next().map_or(Vec3::NEG_Z, |t| t.forward().as_vec3());
    let now = draw.placed().filter(|_| shadow.active).map(|tf| world_shade(&shadow.values, &tf, light.sun, view)).unwrap_or_default();
    if *out != now {
        let (was, is) = (*out != ShipShade::default(), now != ShipShade::default());
        if was != is {
            info!("sidewinder: the shadow on the world {} from frame {}", if is { "casts" } else { "stops (the record is zeros)" }, frame.0);
        }
        *out = now;
    }
}

// ---- The render world ----

/// The passes' layouts and pipelines, queued once at the first frame with a ship.
#[derive(Resource)]
struct ShadowPipelines {
    map_layout: BindGroupLayoutDescriptor,
    resolve_layout: BindGroupLayoutDescriptor,
    blur_layout: BindGroupLayoutDescriptor,
    depth: CachedRenderPipelineId,
    resolve: CachedRenderPipelineId,
    blur: CachedRenderPipelineId,
}

fn queue_pipelines(mut commands: Commands, shadow: Res<ShipShadow>, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    // (None while the shadow is off: its targets' format may be one this GPU can't draw into.)
    if !shadow.enabled {
        return;
    }
    let shader: Handle<Shader> = assets.load("shaders/sidewinder_shadow.wesl");
    let stages = ShaderStages::VERTEX_FRAGMENT;
    let storage = || storage_buffer_read_only_sized(false, None);
    let map_layout = BindGroupLayoutDescriptor::new("sidewinder_shadow_map", &BindGroupLayoutEntries::with_indices(stages, ((0, storage()), (1, storage()), (2, storage()), (3, storage()), (4, storage()))));
    let resolve_layout = BindGroupLayoutDescriptor::new("sidewinder_shadow_resolve", &BindGroupLayoutEntries::with_indices(stages, ((0, storage()), (4, storage()), (10, texture_depth_2d()), (12, storage()))));
    let blur_layout = BindGroupLayoutDescriptor::new(
        "sidewinder_shadow_blur",
        &BindGroupLayoutEntries::with_indices(stages, ((0, storage()), (4, storage()), (11, texture_2d(TextureSampleType::Float { filterable: false })), (12, storage()))),
    );
    // (`CasterVertex`, packed: position, node, bone at locations 0, 1, 2.)
    let vertex_buffer = VertexBufferLayout::from_vertex_formats(VertexStepMode::Vertex, [VertexFormat::Float32x3, VertexFormat::Uint32, VertexFormat::Uint32]);
    let vertex = VertexState { shader: shader.clone(), entry_point: Some("map_vertex".into()), buffers: vec![vertex_buffer], ..default() };
    let depth_state = |compare, bias| DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(compare), stencil: default(), bias };
    let triangles = PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: None, ..default() };
    let depth = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("sidewinder_shadow_depth".into()),
        layout: vec![map_layout.clone()],
        vertex,
        fragment: None,
        primitive: triangles,
        depth_stencil: Some(depth_state(CompareFunction::Less, DepthBiasState::default())),
        ..default()
    });
    let pass = |label: &'static str, layout: &BindGroupLayoutDescriptor, fragment: &'static str| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: VertexState { shader: shader.clone(), entry_point: Some("pass_vertex".into()), ..default() },
            fragment: Some(FragmentState { shader: shader.clone(), entry_point: Some(fragment.into()), targets: vec![Some(ColorTargetState { format: shadow.format, blend: None, write_mask: ColorWrites::ALL })], ..default() }),
            primitive: triangles,
            ..default()
        })
    };
    let resolve = pass("sidewinder_shadow_resolve", &resolve_layout, "resolve_fragment");
    let blur = pass("sidewinder_shadow_blur", &blur_layout, "blur_fragment");
    info!("sidewinder: the shadow's 3 pipelines are queued");
    commands.insert_resource(ShadowPipelines { map_layout, resolve_layout, blur_layout, depth, resolve, blur });
}

/// The render world's own: the casters' buffers (for the copy they were made from), the passes' records, the targets, a word of
/// 1 for `live`, and whether the record was last written active.
#[derive(Resource)]
struct ShadowGpu {
    casters: usize,
    vertices: Buffer,
    indices: Buffer,
    passes: Buffer,
    /// The casters' depth (twice a map's size, sampled by the resolve), the blur's scratch.
    depth: TextureView,
    scratch: TextureView,
    one: Buffer,
    written_active: bool,
}

fn target(device: &RenderDevice, label: &'static str, texels: u32, format: TextureFormat, usage: TextureUsages) -> TextureView {
    device
        .create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d { width: texels, height: texels, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&TextureViewDescriptor::default())
}

/// The casters' buffers and the targets (once, and anew for a new copy of the casters); the record's values with `live` 0 and
/// the passes' records each frame the maps are drawn, and the record once when they stop.
fn prepare(mut commands: Commands, shadow: Res<ShipShadow>, gpu: Option<ResMut<ShadowGpu>>, device: Res<RenderDevice>, queue: Res<RenderQueue>, buffers: Res<RenderAssets<GpuShaderBuffer>>) {
    // (No targets while the shadow is off: their format may be one this GPU can't draw into.)
    if !shadow.enabled {
        return;
    }
    let key = Arc::as_ptr(&shadow.casters) as usize;
    let Some(mut gpu) = gpu.filter(|g| g.casters == key) else {
        let c = &shadow.casters;
        let both = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
        commands.insert_resource(ShadowGpu {
            casters: key,
            vertices: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_shadow_vertices"), contents: bytemuck::cast_slice(&c.vertices), usage: BufferUsages::VERTEX }),
            indices: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_shadow_indices"), contents: bytemuck::cast_slice(&c.indices), usage: BufferUsages::INDEX }),
            passes: device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("sidewinder_shadow_passes"),
                contents: &crate::stars::gpu_bytes(&shadow.passes),
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            }),
            depth: target(&device, "sidewinder_shadow_depth", DEPTH_TEXELS, DEPTH_FORMAT, both),
            scratch: target(&device, "sidewinder_shadow_scratch", SCRATCH_TEXELS, shadow.format, both),
            one: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_shadow_one"), contents: bytemuck::bytes_of(&1u32), usage: BufferUsages::COPY_SRC }),
            written_active: false,
        });
        return;
    };
    if !shadow.active && !gpu.written_active {
        return;
    }
    let Some(record) = buffers.get(&shadow.record) else { return };
    queue.write_buffer(&record.buffer, 0, &crate::stars::gpu_bytes(&shadow.values));
    if shadow.active {
        queue.write_buffer(&gpu.passes, 0, &crate::stars::gpu_bytes(&shadow.passes));
    }
    gpu.written_active = shadow.active;
}

/// The bind groups the map pass made, kept with the ids they were made for.
struct MapBinds {
    ids: [BufferId; 5],
    textures: [TextureId; 2],
    map: BindGroup,
    resolve: BindGroup,
    /// The blur reading each texture (across: `MAP_TEXTURE`'s order), and reading the scratch (down).
    blur_from: [BindGroup; 2],
    blur_scratch: BindGroup,
}

/// A pass drawing one triangle over the square `(x, y, n)` of `view` (kept as it is around it).
fn square_pass(encoder: &mut CommandEncoder, label: &'static str, view: &TextureView, (x, y, n): (u32, u32, u32), pipeline: &RenderPipeline, group: &BindGroup, pass: usize) {
    let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    p.set_viewport(x as f32, y as f32, n as f32, n as f32, 0.0, 1.0);
    p.set_pipeline(pipeline);
    p.set_bind_group(0, group, &[]);
    p.draw(0..3, pass as u32..pass as u32 + 1);
}

/// The maps, once per frame (the main view only) before the opaque pass: for each of the outside set's cascades (into the
/// engine's texture) and, seated, the seat's maps (into the mod's), the casters' depth at twice its size, its moments, the blur
/// across and down; then `live` set in the ship's record and in the world's (the ground's record, `ShipShade::live`).
#[allow(clippy::too_many_arguments)]
fn map_pass(
    _view: ViewQuery<&ExtractedView, MainView>,
    shadow: Res<ShipShadow>,
    gpu: Option<Res<ShadowGpu>>,
    pipes: Res<ShadowPipelines>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    world: Option<Res<CloudShadowAssets>>,
    cache: Res<PipelineCache>,
    mut binds: Local<Option<MapBinds>>,
    mut ctx: RenderContext,
) {
    let (true, Some(gpu)) = (shadow.active, gpu) else { return };
    let (Some(depth), Some(resolve), Some(blur)) = (cache.get_render_pipeline(pipes.depth), cache.get_render_pipeline(pipes.resolve), cache.get_render_pipeline(pipes.blur)) else {
        return;
    };
    let (Some(record), Some(parts), Some(palette), Some(frame), Some(light), Some(outside), Some(seat)) = (
        buffers.get(&shadow.record),
        buffers.get(&shadow.parts),
        buffers.get(&shadow.palette),
        buffers.get(&shadow.frame),
        buffers.get(&shadow.light),
        images.get(&shadow.moments),
        images.get(&shadow.seat_moments),
    ) else {
        return;
    };
    let textures = [outside, seat];
    let ids = [record, parts, palette, frame, light].map(|b| b.buffer.id());
    let texture_ids = textures.map(|t| t.texture.id());
    if binds.as_ref().is_none_or(|b| b.ids != ids || b.textures != texture_ids) {
        let device = ctx.render_device().clone();
        let group = |label: &'static str, layout: &BindGroupLayoutDescriptor, entries: &[BindGroupEntry]| device.create_bind_group(label, &cache.get_bind_group_layout(layout), entries);
        let map = group(
            "sidewinder_shadow_map",
            &pipes.map_layout,
            &BindGroupEntries::with_indices((
                (0, record.buffer.as_entire_binding()),
                (1, parts.buffer.as_entire_binding()),
                (2, palette.buffer.as_entire_binding()),
                (3, frame.buffer.as_entire_binding()),
                (4, light.buffer.as_entire_binding()),
            )),
        );
        let resolve = group("sidewinder_shadow_resolve", &pipes.resolve_layout, &BindGroupEntries::with_indices(((0, record.buffer.as_entire_binding()), (4, light.buffer.as_entire_binding()), (10, &gpu.depth), (12, gpu.passes.as_entire_binding()))));
        let blur_of = |source: &TextureView| group("sidewinder_shadow_blur", &pipes.blur_layout, &BindGroupEntries::with_indices(((0, record.buffer.as_entire_binding()), (4, light.buffer.as_entire_binding()), (11, source), (12, gpu.passes.as_entire_binding()))));
        *binds = Some(MapBinds { ids, textures: texture_ids, map, resolve, blur_from: textures.map(|t| blur_of(&t.texture_view)), blur_scratch: blur_of(&gpu.scratch) });
    }
    let Some(b) = binds.as_ref() else { return };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "sidewinder_shadow_maps");
    let encoder = ctx.command_encoder();
    let casters = |m: usize| if m >= SEAT_MAP { shadow.casters.seat.clone() } else { shadow.casters.outside.clone() };
    let maps = (0..CASCADES).chain(if shadow.seated { SEAT_MAP..MAPS } else { MAPS..MAPS });
    for m in maps {
        let n = shadow.sizes[m];
        let (x, y, _) = REGIONS[m];
        {
            let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("sidewinder_shadow_depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment { view: &gpu.depth, depth_ops: Some(Operations { load: LoadOp::Clear(1.0), store: StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_viewport(0.0, 0.0, (2 * n) as f32, (2 * n) as f32, 0.0, 1.0);
            p.set_pipeline(depth);
            p.set_bind_group(0, &b.map, &[]);
            p.set_vertex_buffer(0, *gpu.vertices.slice(..));
            p.set_index_buffer(*gpu.indices.slice(..), IndexFormat::Uint32);
            p.draw_indexed(casters(m), 0, 0..1);
        }
        let target = &textures[MAP_TEXTURE[m]].texture_view;
        square_pass(encoder, "sidewinder_shadow_resolve", target, (x, y, n), resolve, &b.resolve, 3 * m);
        square_pass(encoder, "sidewinder_shadow_blur", &gpu.scratch, (0, 0, n), blur, &b.blur_from[MAP_TEXTURE[m]], 3 * m + 1);
        square_pass(encoder, "sidewinder_shadow_blur", target, (x, y, n), blur, &b.blur_scratch, 3 * m + 2);
    }
    // (`live` is the record's first word; the world's is `ShipShade::live`'s, the ground's record's last field's first.)
    encoder.copy_buffer_to_buffer(&gpu.one, 0, &record.buffer, 0, Some(4));
    if let Some(w) = world.as_deref().and_then(|w| buffers.get(&w.shadow)) {
        encoder.copy_buffer_to_buffer(&gpu.one, 0, &w.buffer, engine::ship_shade_live_offset(), Some(4));
    }
    span.end(ctx.command_encoder());
}

pub fn build(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<ShipShadow>::default())
        .add_systems(PostUpdate, update.run_if(resource_exists::<ShipShadow>).after(super::draw::place).before(crate::universe::UniverseSet::FloatingOrigin))
        .add_systems(PostUpdate, fit_maps.run_if(resource_exists::<ShipShadow>).after(update).after(bevy::camera::CameraUpdateSystems))
        .add_systems(
            PostUpdate,
            shade_world
                .run_if(resource_exists::<ShipShadow>)
                .after(fit_maps)
                // (After the sandbox's record has this frame's star: it runs before the floating origin.)
                .after(crate::universe::UniverseSet::FloatingOrigin)
                .before(crate::volumetric_clouds::CloudRecordSet),
        );
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        // (Nothing before the first spawn: everything waits for the extracted `ShipShadow`.)
        .add_systems(Render, queue_pipelines.in_set(RenderSystems::PrepareResources).run_if(resource_exists::<ShipShadow>).run_if(not(resource_exists::<ShadowPipelines>)))
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources).run_if(resource_exists::<ShipShadow>))
        // (Before the whole prepass, as the headlights' fill: the light passes there, the scatter's and the hero's and the
        // sandbox's, read the maps and the world's `live` this frame.)
        .add_systems(Core3d, map_pass.before(Core3dSystems::Prepass).run_if(resource_exists::<ShadowPipelines>));
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::assets::{self, Loaded};
    use se_core::elite_asset::parse_edm;
    use se_core::elite_chair::{Chair, ChairInputs, ChairSkeleton};
    use se_core::elite_ship::{parse_chair_clips, parse_chair_graph};

    fn read(rel: &str) -> Result<Vec<u8>, String> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string())
    }

    fn shipped() -> Loaded {
        assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}"))
    }

    /// `sidewinder_shadow_common.wesl` `map_point` (with `light_axes`), the tests' copy.
    fn map_point(centre: Vec3, radius: f32, l: Vec3, p: Vec3) -> Vec3 {
        let helper = if l.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
        let u = helper.cross(l).normalize();
        let v = l.cross(u);
        let q = (p - centre) / radius;
        Vec3::new(q.dot(u), q.dot(v), 0.5 - 0.5 * q.dot(l))
    }

    /// Star directions all round, the poles and near-poles too (where the map's axes change helper).
    fn directions() -> Vec<Vec3> {
        let mut out = vec![Vec3::Y, -Vec3::Y, Vec3::new(0.01, 0.9999, 0.0).normalize(), Vec3::new(0.2, -0.98, 0.0).normalize()];
        for i in 0..12 {
            for j in 1..6 {
                let (az, el) = (i as f32 * std::f32::consts::TAU / 12.0, -1.4 + j as f32 * 0.56);
                out.push(Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin()));
            }
        }
        out
    }

    /// The user's screen and field of view (re/elite/notes/sidewinder_shadow.md section 1): 3440 x 1440, 56.249 degrees.
    const USER_FOV_DEG: f32 = 56.249;
    const WIDE: f32 = 3440.0 / 1440.0;
    const SQUARE: f32 = 16.0 / 9.0;

    /// The casters as the game makes them (`Casters::new` from the shipped files: the maps' sphere), and the files.
    fn ship_casters() -> (Casters, Loaded) {
        let loaded = shipped();
        (Casters::new(&loaded.assets.rig, &loaded.build.meshes, &loaded.build.materials), loaded)
    }

    /// The maps' sizes and cap for a drawn view's vertical field of view `fov` (radians), as `fit_maps` makes them.
    fn sizes_for(fov: f32, aspect: f32, radius: f32) -> ([u32; MAPS], bool) {
        map_sizes(&cascades(FitFov::default().fit(fov), aspect), radius)
    }

    /// Elite's cascades against the study's tables (section 2: texel.py) at both window shapes: the moment texels, and the
    /// widths at 3440 x 1440 (17.9 m, 288 m, 2,639 m, 24,651 m).
    #[test]
    fn cascades_are_the_studys() {
        let fit = (USER_FOV_DEG + FIT_EXTRA_DEG).to_radians();
        // (Each value to half the table's last digit.)
        let cases = [(WIDE, [0.023, 0.376, 3.44, 32.0], [0.0005, 0.0005, 0.005, 0.5]), (SQUARE, [0.018, 0.296, 2.71, 25.3], [0.0005, 0.0005, 0.005, 0.05])];
        for (aspect, want, tol) in cases {
            let got = cascades(fit, aspect);
            for i in 0..4 {
                assert!((got[i].texel - want[i]).abs() <= tol[i], "aspect {aspect}: {got:?} against {want:?}");
            }
            println!("aspect {aspect:.3}: {got:?}");
        }
        // (To the table's rounding: its 17.9 m and its first depth texel, 0.0116 m = 17.82 / 1536, bracket the 17.85 m.)
        let widths = cascades(fit, WIDE).map(|c| 2.0 * c.half);
        for (got, want) in widths.iter().zip([17.9, 288.0, 2639.0, 24651.0]) {
            assert!((got - want).abs() <= want * 0.004, "{widths:?}");
        }
    }

    /// The maps are sized for the view the frame is drawn with (the pure part; `fit_maps` reads the projection after the seat
    /// has written it): over the game's own sphere, seated at 3440 x 1440 Elite's rule gives 1,176, 73 and 8 texels, with the
    /// right panel focused (its narrower view) more, and at the walker's 59.84 degrees fewer. A region caps the first map only
    /// below about 54.9 degrees at 16:9 and 44.2 at 3440 x 1440 (printed), the free camera's 45 degrees at 16:9 among them.
    #[test]
    fn the_maps_follow_the_drawn_view() {
        use super::super::{panels::Side, seat};
        let (c, loaded) = ship_casters();
        let r = c.radius;
        let fov = seat::seat_vertical_fov(WIDE);
        assert!((fov.to_degrees() - USER_FOV_DEG).abs() < 1e-3);
        let (sizes, capped) = sizes_for(fov, WIDE, r);
        println!("the sphere {r:.3} m; seated at 3440 x 1440: {sizes:?}");
        assert_eq!(sizes, [1176, 73, 8, 1176, 73]);
        assert!(!capped);
        let eye = Vec3::from_array(loaded.assets.eye);
        let panel = seat::view_vertical_fov(WIDE, seat::panel_head(Side::Right, eye, WIDE).fov_add);
        let (narrow, _) = sizes_for(panel, WIDE, r);
        println!("the right panel's {:.3} degrees: {narrow:?}", panel.to_degrees());
        assert!(narrow[0] > sizes[0] && narrow[1] > sizes[1]);
        let (walking, _) = sizes_for(59.84f32.to_radians(), WIDE, r);
        println!("walking (59.84 degrees): {walking:?}");
        assert!(walking[0] < sizes[0]);
        let (square, capped) = sizes_for(seat::seat_vertical_fov(SQUARE), SQUARE, r);
        println!("seated at 16:9: {square:?}");
        assert!(!capped && square[0] < REGIONS[0].2);
        let (free, capped) = sizes_for(45f32.to_radians(), SQUARE, r);
        assert!(capped && free[0] == REGIONS[0].2, "{free:?}");
        for (aspect, want) in [(SQUARE, 54.9), (WIDE, 44.2)] {
            let edge = (100..900).map(|i| i as f32 * 0.1).find(|&deg| !sizes_for(deg.to_radians(), aspect, r).1).unwrap();
            println!("aspect {aspect:.3}: capped below {edge:.1} degrees");
            assert!((edge - want).abs() <= 0.15, "{edge}");
        }
    }

    /// `moment_shadow.wesl` `cascade_pick` on the CPU (the shader's lines, FWD 182-231; `the_pick_is_elites` checks the shader
    /// still holds the lines this mirrors): for each of Elite's
    /// cascades in turn, the point in the box's units across the light (`light_axes`; the box's middle `ahead` m along the view
    /// from the eye, `half` m wide either side); inside if both are within 1; the weight the smoothstep of the distance to the
    /// box's edge between the fade's floor and its top (a step at the floor if the two are equal). The first cascade with a
    /// weight is the one read; the next with a weight is blended in only if the first's is not 1. `None`: no cascade.
    fn pick(c: &[Cascade; 4], eye: Vec3, ahead: Vec3, l: Vec3, p: Vec3) -> (Option<(usize, f32)>, Option<(usize, f32)>) {
        let helper = if l.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
        let u = helper.cross(l).normalize();
        let v = l.cross(u);
        let (mut first, mut second) = (None, None);
        for (k, b) in c.iter().enumerate() {
            let (lo, hi) = (FADE_FLOOR, crossfade(k));
            let q = p - (eye + ahead * b.ahead);
            let (x, y) = (q.dot(u) / b.half, q.dot(v) / b.half);
            let inside = x.abs() <= 1.0 && y.abs() <= 1.0;
            let edge = (1.0 - x.abs()).min(1.0 - y.abs());
            let w = if hi != lo {
                let t = ((edge - lo) / (hi - lo)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            } else if edge >= lo {
                1.0
            } else {
                0.0
            };
            if inside && w != 0.0 {
                match first {
                    None => first = Some((k, w)),
                    Some((_, w1)) if second.is_none() && w1 != 1.0 => second = Some((k, w)),
                    _ => {}
                }
            }
        }
        (first, second)
    }

    /// `cascade_blend` (FWD 578-589): the first cascade's visibility `v1` (weight `w1`) over the second's `v2` (weight `w2`),
    /// each 1 with weight 0 where there is none.
    fn blend(w1: f32, w2: f32, v1: f32, v2: f32) -> f32 {
        let second = if w1 == 1.0 {
            0.0
        } else if w2 != 1.0 {
            1.0 + w2 * (v2 - 1.0)
        } else {
            v2
        };
        w1 * (v1 - second) + second
    }

    /// Elite's pick, seated at 3440 x 1440 with the star straight overhead and the view level (the boxes' axes then lie along
    /// the view and across it): which cascade and what weight for points straight ahead and off to the side at the first
    /// box's edge, against the numbers worked by hand from the shader's lines (FWD 182-231: thresholds 4 / 1536 and 0.15;
    /// boxes 5 m ahead and 8.924 m wide either side, 80.8 m and 144.205 m, 739.2 m and 1,319.26 m).
    #[test]
    fn the_pick_is_elites() {
        let c = cascades((USER_FOV_DEG + FIT_EXTRA_DEG).to_radians(), WIDE);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let (eye, ahead, l) = (Vec3::new(1.0, 2.0, 3.0), Vec3::NEG_Z, Vec3::Y);
        // (Straight ahead d m: 13 m is x = 8 / 8.924 = 0.8965 across the first box, its edge 0.1035, t = (0.1035 - 0.0026) /
        // (0.15 - 0.0026) = 0.6845, w = t^2 (3 - 2 t) = 0.764; 13.5 m: 0.2215; the second box holds both whole.)
        let ahead_cases = [
            (3.0, (0, 1.0), None),
            (5.0, (0, 1.0), None),
            (8.0, (0, 1.0), None),
            (12.0, (0, 1.0), None),
            (13.0, (0, 0.7642), Some((1, 1.0))),
            (13.5, (0, 0.2215), Some((1, 1.0))),
            (20.0, (1, 1.0), None),
            (80.0, (1, 1.0), None),
            (150.0, (1, 1.0), None),
            (250.0, (2, 1.0), None),
        ];
        for (d, want1, want2) in ahead_cases {
            let (first, second) = pick(&c, eye, ahead, l, eye + ahead * d);
            let (k, w) = first.unwrap();
            assert!(k == want1.0 && close(w, want1.1), "{d} m: {first:?}");
            assert_eq!(second.map(|s| s.0), want2.map(|s: (usize, f32)| s.0), "{d} m: {second:?}");
            assert!(second.zip(want2).is_none_or(|(a, b)| close(a.1, b.1)), "{d} m: {second:?}");
        }
        // (5 m ahead, s x 8.924 m to the side: 0.9 -> w 0.7329, 0.95 -> 0.2437; within 4 / 1536 of the edge the second alone.)
        for (s, want1, want2) in [(0.8, (0, 1.0), None), (0.9, (0, 0.7329), Some(1)), (0.95, (0, 0.2437), Some(1)), (0.999, (1, 1.0), None), (1.01, (1, 1.0), None)] {
            let (first, second) = pick(&c, eye, ahead, l, eye + ahead * 5.0 + Vec3::X * s * c[0].half);
            let (k, w) = first.unwrap();
            assert!(k == want1.0 && close(w, want1.1), "side {s}: {first:?}");
            assert_eq!(second.map(|s| s.0), want2, "side {s}");
        }
        // (Past every box: no cascade, lit.)
        assert_eq!(pick(&c, eye, ahead, l, eye + ahead * 30000.0), (None, None));
        // The shader is still what `pick` and `blend` mirror, line for line (the comparisons, the weight, the first-and-second
        // rule, the blend), and the fade's thresholds are still read as `pick` reads them.
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/shaders/moment_shadow.wesl")).unwrap();
        let src: Vec<&str> = src.lines().map(str::trim).collect();
        for line in [
            "let lo = fade.y;",
            "let hi = select(fade.z, fade.w, k + 1u == count);",
            "let q = p - (eye + dir * ahead[k]);",
            "let b = vec2<f32>(dot(q, a[0]), dot(q, a[1])) / half[k];",
            "let inside = abs(b.x) <= 1.0 && abs(b.y) <= 1.0;",
            "let edge = min(1.0 - abs(b.x), 1.0 - abs(b.y));",
            "let t = sat((edge - lo) / (hi - lo));",
            "let w = select(select(0.0, 1.0, edge >= lo), t * t * (3.0 - 2.0 * t), hi != lo);",
            "if inside && w != 0.0 {",
            "if !found {",
            "pick = vec4<f32>(f32(k), w, NO_CASCADE, 0.0);",
            "} else if !second && pick.y != 1.0 {",
            "pick = vec4<f32>(pick.xy, f32(k), w);",
            "if pick.y != 1.0 {",
            "second = select(v2, 1.0 + pick.w * (v2 - 1.0), pick.w != 1.0);",
            "return pick.y * (v1 - second) + second;",
        ] {
            assert!(src.contains(&line), "moment_shadow.wesl no longer holds `{line}`: update `pick` / `blend` with it");
        }
        let fade = ShadowRecord::new(Vec3::ZERO, 1.0).fade;
        assert_eq!(fade, Vec4::new(4.0, FADE_FLOOR, crossfade(0), crossfade(3)), "the fade the shader reads as count, floor, inner top, last top");
        // The blend: a shadowed first cascade fading into a lit second; fading out with no second; one alone; none.
        assert!(close(blend(0.7642, 1.0, 0.0, 1.0), 0.2358));
        assert!(close(blend(0.25, 0.0, 0.0, 1.0), 0.75));
        assert!(close(blend(1.0, 0.0, 0.3, 1.0), 0.3));
        assert!(close(blend(0.0, 0.0, 1.0, 1.0), 1.0));
    }

    /// Elite's hysteresis: the fit keeps its field of view while the view's stays within 4 degrees under it, and is re-centred
    /// 2 degrees over it when the view's leaves that band.
    #[test]
    fn the_fit_keeps_its_field_of_view() {
        let deg = |d: f32| d.to_radians();
        let mut f = FitFov::default();
        assert!((f.fit(deg(56.0)) - deg(58.0)).abs() < 1e-6);
        assert!((f.fit(deg(57.5)) - deg(58.0)).abs() < 1e-6);
        assert!((f.fit(deg(54.5)) - deg(58.0)).abs() < 1e-6);
        assert!((f.fit(deg(53.0)) - deg(55.0)).abs() < 1e-6);
        assert!((f.fit(deg(58.0)) - deg(60.0)).abs() < 1e-6);
    }

    /// Elite's Gaussian kernels as the notes give them (section 3): r 1 [0.1065, 0.7870, 0.1065], r 2 [0.0545, 0.2442, 0.4026,
    /// 0.2442, 0.0545]; the radius 2 for the first cascade, 1 for the others.
    #[test]
    fn the_blur_is_elites() {
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(close(gaussian(1), [0.7870, 0.1065, 0.0]), "{:?}", gaussian(1));
        assert!(close(gaussian(2), [0.4026, 0.2442, 0.0545]), "{:?}", gaussian(2));
        assert_eq!([blur_radius(0), blur_radius(1), blur_radius(2)], [2, 1, 1]);
        for r in 1..=2 {
            let [a, b, c] = gaussian(r);
            assert!((a + 2.0 * b + 2.0 * c - 1.0).abs() < 1e-6);
        }
    }

    /// The write is the inverse of Elite's read: the moments of every depth come back, and their quantised form fits the
    /// map's fixed-point channels.
    #[test]
    fn moments_round_trip() {
        let read = MOMENT_READ.map(Vec4::from_array);
        for i in 0..=100 {
            let d = i as f32 / 100.0;
            let q = quantised(d);
            assert!(q.min_element() >= -1e-4 && q.max_element() <= 1.0 + 1e-4, "d {d}: {q}");
            let c = q - Vec4::new(MOMENT_OFFSET, 0.0, 0.0, 0.0);
            let b = Vec4::new(read[0].dot(c), read[1].dot(c), read[2].dot(c), read[3].dot(c));
            let want = Vec4::new(d, d * d, d * d * d, d * d * d * d);
            assert!((b - want).abs().max_element() < 1e-4, "d {d}: {b} for {want}");
        }
    }

    /// Each map's regions lie inside its texture (the engine's for the outside set, at `SHIP_SHADE_TEXELS`) and apart from the
    /// others in it, and its passes read and write inside them.
    #[test]
    fn the_maps_keep_to_their_regions() {
        for (i, a) in REGIONS.iter().enumerate() {
            assert!(a.0 + a.2 <= ATLAS.0 && a.1 + a.2 <= ATLAS.1);
            assert!(2 * a.2 <= DEPTH_TEXELS && a.2 <= SCRATCH_TEXELS);
            for (j, b) in REGIONS.iter().enumerate().skip(i + 1).filter(|(j, _)| MAP_TEXTURE[*j] == MAP_TEXTURE[i]) {
                assert!(a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.2 <= b.1 || b.1 + b.2 <= a.1, "{i} {a:?} and {j} {b:?} overlap");
            }
        }
        assert!((0..MAPS).all(|m| (MAP_TEXTURE[m] == 0) == (m < SEAT_MAP)), "the outside set's maps in the engine's texture, the seat's in the mod's");
        let sizes = [1536, 256, 64, 1536, 256];
        let (maps, _, passes) = layout(sizes, &cascades((USER_FOV_DEG + 2.0).to_radians(), SQUARE), ship_casters().0.radius);
        for m in 0..MAPS {
            assert_eq!((maps[m].x as u32, maps[m].y as u32, maps[m].z as u32), (REGIONS[m].0, REGIONS[m].1, sizes[m]));
            assert_eq!(passes[3 * m + 1].src.truncate().truncate(), passes[3 * m + 2].dst.truncate().truncate());
        }
    }

    /// The world's record takes a receiver's place (m from the camera) into the same ship space and map point as the ship's own
    /// shaders take its scaled place: for a ship drawn at a scale, turned and moved, a point of the ship's space placed in
    /// metres comes back, the light's axes are the shader's (`map_point`), the camera's place and view are the ship's-space
    /// ones, and the maps, boxes and filter's values are the ship's record's (the deepest depth past every caster).
    #[test]
    fn the_worlds_record_reads_the_ships_maps() {
        let (centre, radius) = (Vec3::new(0.3, 1.2, -0.4), 13.66);
        let mut r = ShadowRecord::new(centre, radius);
        let (maps, _, _) = layout([1176, 73, 8, 1176, 73], &cascades((USER_FOV_DEG + FIT_EXTRA_DEG).to_radians(), WIDE), radius);
        r.maps = maps;
        let k = 0.75;
        let rot = Quat::from_euler(EulerRot::YXZ, 0.7, -0.2, 0.1);
        let t_m = Vec3::new(12.0, -3.0, 40.0);
        let tf = Transform { translation: t_m * k, rotation: rot, scale: Vec3::splat(k) };
        let (sun, view) = (Vec3::new(0.3, 0.8, -0.5).normalize(), Vec3::new(0.1, -0.2, -1.0).normalize());
        let s = world_shade(&r, &tf, sun, view);
        let ship = |p: Vec3| Vec3::new(s.to_ship_x.dot(p.extend(1.0)), s.to_ship_y.dot(p.extend(1.0)), s.to_ship_z.dot(p.extend(1.0)));
        for p in [Vec3::ZERO, centre, Vec3::new(-5.0, 2.0, 7.0)] {
            assert!((ship(rot * p + t_m) - p).length() < 1e-4, "{p}");
        }
        let l = rot.inverse() * sun;
        assert!((s.axis_l.truncate() - l).length() < 1e-5);
        let q = Vec3::new(4.0, -1.0, 2.0);
        let m = map_point(centre, radius, l, q);
        let w = (q - centre) / radius;
        let mine = Vec3::new(w.dot(s.axis_u.truncate()), w.dot(s.axis_v.truncate()), 0.5 - 0.5 * w.dot(s.axis_l.truncate()));
        assert!((m - mine).length() < 1e-5, "{m} {mine}");
        assert!((s.eye.truncate() - rot.inverse() * -t_m).length() < 1e-4);
        assert!((s.view.truncate() - rot.inverse() * view).length() < 1e-5);
        assert_eq!((s.map0, s.map1, s.map2), (r.maps[0], r.maps[1], r.maps[2]));
        assert_eq!((s.ahead, s.half, s.fade, s.texels), (r.ahead, r.half, r.fade, r.atlas));
        assert_eq!(s.msm.truncate(), Vec3::new(MOMENT_OFFSET, MOMENT_BIAS, LIGHT_BLEEDING_CUTOFF));
        assert!(s.msm.w < 1.0 && s.msm.w > 1.0 - MARGIN_M / (2.0 * radius));
        assert_eq!([s.read0, s.read1, s.read2, s.read3], r.read);
        assert_eq!(s.live, UVec4::ZERO);
    }

    /// The caster lists hold exactly the opaque families' triangles of each set, never a glass, overlay or stamp draw's; only
    /// the seat's chair's vertices carry bones.
    #[test]
    fn casters_are_the_opaque_draws_of_each_set() {
        let Loaded { assets, build } = shipped();
        let rig = &assets.rig;
        let c = Casters::new(rig, &build.meshes, &build.materials);
        let (mut outside, mut seat, mut left_out) = (0usize, 0usize, 0usize);
        for n in &rig.nodes {
            let Some(model) = n.model else { continue };
            for (key, mesh) in rig.models[model].draws.iter().zip(&build.meshes[model]) {
                let Some(r) = build.materials.get(key) else { continue };
                let count = mesh.indices().map_or(0, |i| i.len());
                if !casts(r.family) {
                    left_out += count;
                } else if n.group.inside() {
                    seat += count;
                } else {
                    outside += count;
                }
            }
        }
        assert_eq!(c.outside.len(), outside);
        assert_eq!(c.seat.len(), seat);
        assert!(outside > 0 && seat > 0 && left_out > 0, "outside {outside}, seat {seat}, left out {left_out}");
        assert!(c.indices.iter().all(|&i| (i as usize) < c.vertices.len()));
        for v in &c.vertices {
            let n = &rig.nodes[v.node as usize];
            let skinned = n.model.is_some_and(|m| rig.models[m].skinned);
            assert_eq!(v.bone != NO_BONE, skinned, "node {}", n.name);
            assert!(v.bone == NO_BONE || (v.bone as usize) < material::PALETTE_BONES);
        }
        println!("casters: {} triangles outside, {} seated, {} left out (glass, overlays, stamp)", outside / 3, seat / 3, left_out / 3);
    }

    /// The pilot chair's palettes over a run of every control at its limits (Elite's own chair, `se_core::elite_chair`, as
    /// `chair.rs` runs it), in the mod's axes.
    fn chair_palettes(rig: &ShipRig) -> Vec<Vec<Mat4>> {
        let edm = parse_edm(&read("models/cockpit_chair.edm").unwrap()).unwrap();
        let skeleton = ChairSkeleton::from_edm(&edm).unwrap();
        let clips = parse_chair_clips(&read("chair/clips.json").unwrap(), &read("chair/clips.bin").unwrap()).unwrap();
        let graph = parse_chair_graph(&read("chair/graph.json").unwrap()).unwrap();
        let mut chair = Chair::new(&skeleton, Arc::new(clips), Arc::new(graph)).unwrap();
        let mut out = Vec::new();
        for (throttle, roll, pitch, yaw, fire) in [(0.0, 0.0, 0.0, 0.0, false), (1.0, 1.0, 1.0, 1.0, true), (-1.0, -1.0, -1.0, -1.0, false), (1.0, -1.0, 1.0, -1.0, true), (0.5, 1.0, -1.0, 1.0, false)] {
            let inputs = ChairInputs { seated: true, throttle, roll, pitch, yaw, fire, velocity_change: [0.0; 3] };
            for step in 0..240 {
                chair.step(1.0 / 60.0, &inputs);
                if step % 20 == 19 {
                    out.push(chair.skin_matrices().iter().map(|m| rig.space.skin(*m)).collect());
                }
            }
        }
        out
    }

    /// The light's frame covers every caster vertex for any star direction and any pose: in poses four times finer than the
    /// sphere was made from (the gear, every machine's every state: the hardpoints' doors, the vents' flaps...) and with the
    /// pilot chair's bones posed by every control at its limits, every vertex lies inside the sphere, so on the maps' square and
    /// in their depth range. The seat's two maps serve every pixel of the cockpit set: by Elite's pick every point of it the eye
    /// sees lies inside the second cascade's box, short of its fade (so none fades out to lit for want of a third map).
    #[test]
    fn the_map_covers_every_caster() {
        let Loaded { assets, build } = shipped();
        let rig = &assets.rig;
        let c = Casters::new(rig, &build.meshes, &build.materials);
        let dirs = directions();
        let palettes = chair_palettes(rig);
        let still = rig.pose(&rig.static_pose(true)).unwrap();
        // (Inside the sphere is on the maps for every direction; the directions themselves are checked in the still pose.)
        let mut most = 0.0f32;
        let mut check = |p: Vec3| most = most.max(p.distance(c.centre));
        let (down, up) = (rig.static_pose(true), rig.static_pose(false));
        let fine = POSE_SAMPLES * 4;
        let mut poses: Vec<Vec<Mat4>> = (0..=fine).filter_map(|i| rig.pose(&between(&down, &up, i as f64 / fine as f64)).ok()).collect();
        poses.extend(rig_poses(rig, POSE_SAMPLES));
        println!("{} poses", poses.len());
        for pose in &poses {
            for v in c.vertices.iter().filter(|v| v.bone == NO_BONE) {
                check(pose[v.node as usize].transform_point3(Vec3::from_array(v.pos)));
            }
        }
        for palette in &palettes {
            for v in c.vertices.iter().filter(|v| v.bone != NO_BONE) {
                check(still[v.node as usize].transform_point3(palette[v.bone as usize].transform_point3(Vec3::from_array(v.pos))));
            }
        }
        for v in c.vertices.iter().step_by(7) {
            let p = still[v.node as usize].transform_point3(Vec3::from_array(v.pos));
            for &l in &dirs {
                let m = map_point(c.centre, c.radius, l, p);
                assert!(m.x.abs() <= 1.0 && m.y.abs() <= 1.0 && (0.0..=1.0).contains(&m.z), "{p} at {l}: {m}");
            }
        }
        assert!(most <= c.radius - 0.5 * MARGIN_M, "a caster {most} m out, the sphere {} m", c.radius);
        let eye = Vec3::from_array(assets.eye);
        let seat_points = c.indices[c.seat.start as usize..c.seat.end as usize].iter().map(|&i| &c.vertices[i as usize]);
        let reach: Vec<f32> = seat_points.map(|v| still[v.node as usize].transform_point3(Vec3::from_array(v.pos)).distance(eye)).collect();
        let farthest = reach.iter().copied().fold(0.0f32, f32::max);
        let beyond = reach.iter().filter(|&&d| d >= CASCADE_ENDS[0]).count() as f32 / reach.len() as f32;
        println!(
            "the sphere: {:.3} m round {}, the furthest caster {most:.3} m out; the cockpit set reaches {farthest:.2} m from the eye, {:.1} % of its triangles' corners past 5 m",
            c.radius,
            c.centre,
            beyond * 100.0
        );
        // The seat's two maps hold the cockpit set wherever the eye sees it, never fading out to lit: a point the eye sees within
        // `farthest` of it lies, across the light, at most max(zc, sqrt(f^2 + zc^2 - 2 f zc cos b)) from the second box's middle
        // (zc its distance ahead, b the fit's frustum's corner angle from the view), which must be short of that box's fade, for
        // the seat's view and the right panel's narrower one at both window shapes.
        use super::super::{panels::Side, seat};
        for aspect in [SQUARE, WIDE] {
            for fov in [seat::seat_vertical_fov(aspect), seat::view_vertical_fov(aspect, seat::panel_head(Side::Right, eye, aspect).fov_add)] {
                let fit = FitFov::default().fit(fov);
                let ty = (fit * 0.5).tan();
                let corner = (ty * ty * (1.0 + aspect * aspect)).sqrt().atan();
                let b = cascades(fit, aspect)[1];
                let across = b.ahead.max((farthest * farthest + b.ahead * b.ahead - 2.0 * farthest * b.ahead * corner.cos()).sqrt());
                println!("aspect {aspect:.3}, {:.2} degrees: the cockpit set at most {across:.1} m across the light from the second box's middle, its fade from {:.1} m", fov.to_degrees(), (1.0 - crossfade(1)) * b.half);
                assert!(across <= (1.0 - crossfade(1)) * b.half);
            }
        }
    }
}
