//! The Sidewinder's shadow, on itself and on the world (ARCHITECTURE.md A6, T4e-1). The engine has no shadow map, so the mod
//! makes its own: a pass of its own draws the mod's own copy of the casters' triangles (`Casters`, made once at the first spawn
//! from the meshes `assets.rs` decoded; never Bevy's mesh buffers or draw functions) into maps orthographic along the star's
//! direction, in the ship's space, over a sphere round every caster in every pose of the gear (`Casters::sphere`), so the ship's
//! own turning only turns the light's frame. The shaders: `sidewinder_shadow.wesl` (the passes) and
//! `sidewinder_shadow_common.wesl` (the maths they and the ship's material share; its header says what is Elite's).
//!
//! - **Casters:** every opaque family's draws (`casts`); never the glass, the overlays or the stamp. Two sets, as drawn
//!   (`draw.rs`): the outside set (the hull, the gear, the outside chair), which is the ship's true outline and so the caster
//!   for the world whatever the view, and the seat's (the cockpit, the seat's chair) for the cockpit's own surfaces while
//!   seated. Each vertex carries its rig node (its matrix this frame: `parts`, written in place when the pose changes; a node
//!   the machines hide is zero) and, for the seat's chair, its bone (posed by the palette the ship's material reads).
//! - **The maps** (`MAP_TEXELS` square, ours): the moments map, two layers (`MAP_FORMAT`; the outside set's, the seat's, drawn
//!   only while seated), which Elite's filter reads (the moment shadow mapping of its lit shaders, grade C:
//!   `sidewinder_shadow_common.wesl`); and the outside set's furthest caster's depth (`Depth32Float`, its own test reversed),
//!   which tells a pixel behind the ship from the ship itself.
//! - **(a) The ship's surfaces:** `sidewinder_lit.wesl` `star_shadow` multiplies the star's light (only the star's) by the
//!   filter's answer at the pixel: bindings 20-22 of `ShipMaterial` (no new pipeline variant).
//! - **(b) Everything else:** one pass over the picture before the clouds (`world_pass`), multiplying it, from the depth buffer:
//!   a pixel further from the star than the furthest caster loses the star's direct light by the engine's rule for a shadow on
//!   the ground (hero.wesl `fragment_shadow`), as much as the filter says it is shadowed.
//! - **When it runs:** only while a ship is drawn (`draw::ShipDraw::placed`: it exists and is within its draw distance) and the
//!   sandbox's light record is on; nothing at all before the first spawn (every system waits for `ShipShadow`). Whether the
//!   star lights the ship's place (night, an eclipse) is known only on the GPU (the light record's star term, worked out by the
//!   hero's light pass from the sandbox's record, whose CPU half the sandbox keeps private): there the passes are recorded but
//!   draw nothing (every vertex collapses, `star_lit`) and the maps stay cleared, so the ship reads lit. The record's `live` is
//!   0 each frame until the map pass has drawn this frame's maps, so a frame without them (the pipelines not built yet, no
//!   ship) never reads an old one. `SE_SHIP_SHADOW=0` leaves it all out, for comparisons (the ship then reads lit, as before).
//!
//! Pipelines added: 4, each built once at the first spawn (the moments and the furthest depth; the pass over the world with
//! and without MSAA). The ship's materials keep their variants.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{DMat4, UVec4, Vec4};
use bevy::mesh::{VertexAttributeValues, VertexBufferLayout};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only_sized, texture_2d_array, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::settings::WgpuFeatures;
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::ShaderDefVal;
use se_core::elite_rig::Values;

use crate::sandbox::light::PropLight;
use crate::viltrumite::draw::MainView;
use crate::volumetric_clouds::{CloudView, CloudsPassSet, HDR_FORMAT};

use super::assets::ShipRig;
use super::draw::ShipDraw;
use super::material::{self, Family, ShipPalette, ATTRIBUTE_BONE, SEAT_SLICE};
use super::parts::Parts;

/// The maps' texels a side (ours, the brief's suggestion): over the Sidewinder's sphere a texel is about 1.2 cm.
pub const MAP_TEXELS: u32 = 2048;
/// The moments' format: four unsigned 16-bit fixed-point channels, the precision Elite's optimised quantisation is made for
/// (the read's offset and matrix, FWD 345-349; the writer is not in the files). `MAP_FALLBACK` where the GPU lacks it.
const MAP_FORMAT: TextureFormat = TextureFormat::Rgba16Unorm;
const MAP_FALLBACK: TextureFormat = TextureFormat::Rgba16Float;
/// The moments map's layers: the outside set's, the seat's.
const LAYERS: u32 = 2;
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
/// The sphere's radius past the furthest caster vertex in the poses sampled (ours): covers the seat's chair's motion (its
/// vertices are taken in the bind pose) and the gear's arcs between the samples.
const MARGIN_M: f32 = 0.25;
/// The gear's travel, sampled between its two still poses (ours).
const GEAR_SAMPLES: usize = 16;
/// The lookup's offset along the normal, in texels (ours, by geometry: the filter reaches two texels either way, so a surface
/// facing the star is read from beyond its own footprint).
const NORMAL_OFFSET_TEXELS: f32 = 2.0;
/// The moments' write: the caster's depth moved away from the star by this many times its change across a texel (ours: a
/// slope-scaled bias, so a lit surface does not read its own caster as nearer).
const WRITE_SLOPE: f32 = 1.0;
/// The furthest depth's own slope-scaled bias in its pipeline (ours: the depth's change across two texels), and how far past it
/// (texels of depth at 45 degrees) a pixel must be to count as behind the ship rather than as the ship itself.
const FAR_SLOPE: f32 = 2.0;
const FAR_MARGIN_TEXELS: f32 = 2.0;
/// Elite's moment bias (cb1[261].y) and light-leak bound (cb1[261].x): engine values, not found (the known gap). Stand-ins,
/// ours: the moment bias the method's paper gives for four 16-bit optimised moments (Peters and Klein, "Moment Shadow
/// Mapping", 2015, 3e-5) and no light-leak rescale beyond Elite's smoothstep (0).
const MOMENT_BIAS: f32 = 3.0e-5;
const LEAK_BOUND: f32 = 0.0;
/// Elite's dequantisation (FWD 345-349): b = READ x (q - (OFFSET, 0, 0, 0)), row by row.
const MOMENT_OFFSET: f32 = 0.035956;
const MOMENT_READ: [[f32; 4]; 4] = [
    [0.222774, 0.077197, 0.792699, 0.031942],
    [0.154968, 0.139463, 0.796342, -0.172282],
    [0.145199, 0.212020, 0.725869, -0.275801],
    [0.163127, 0.259143, 0.653909, -0.337613],
];
/// A vertex not posed by the palette (`sidewinder_shadow.wesl` `NO_BONE`).
pub const NO_BONE: u32 = u32::MAX;

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

/// The gear's still poses' values mixed: `t` 0 the gear down, 1 up (a value only one has is kept).
fn gear_between(down: &Values, up: &Values, t: f64) -> Values {
    let mut v = down.clone();
    for (k, b) in up {
        let a = down.get(k).copied().unwrap_or(*b);
        v.insert(k.clone(), a + (b - a) * t);
    }
    v
}

/// Every node's matrix in the poses the gear passes through (`GEAR_SAMPLES` + 1 of them, down to up).
fn gear_poses(rig: &ShipRig, samples: usize) -> Vec<Vec<Mat4>> {
    let (down, up) = (rig.static_pose(true), rig.static_pose(false));
    (0..=samples).filter_map(|i| rig.pose(&gear_between(&down, &up, i as f64 / samples as f64)).ok()).collect()
}

/// The sphere round every caster vertex in every gear pose sampled, `MARGIN_M` wider (its middle the middle of their box).
fn sphere(rig: &ShipRig, vertices: &[CasterVertex]) -> (Vec3, f32) {
    let poses = gear_poses(rig, GEAR_SAMPLES);
    let points = || poses.iter().flat_map(|m| vertices.iter().map(move |v| m[v.node as usize].transform_point3(Vec3::from_array(v.pos))));
    let (lo, hi) = points().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let centre = (lo + hi) * 0.5;
    let reach = points().map(|p| p.distance(centre)).fold(0.0f32, f32::max);
    (centre, reach + MARGIN_M)
}

/// The rows of Elite's read matrix's inverse: the write's (`quantised_moments`).
fn moment_write() -> [Vec4; 4] {
    let read = DMat4::from_cols_array_2d(&MOMENT_READ.map(|r| r.map(f64::from))).transpose();
    let w = read.inverse();
    [0, 1, 2, 3].map(|i| w.row(i).as_vec4())
}

/// The quantised moments of depth `d` (the shader's `quantised_moments`, for the clear value and the tests).
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
    map: Vec4,
    bias: Vec4,
    msm: Vec4,
    read: [Vec4; 4],
    write: [Vec4; 4],
}

impl ShadowRecord {
    fn new(centre: Vec3, radius: f32) -> ShadowRecord {
        let texels = MAP_TEXELS as f32;
        let texel_m = 2.0 * radius / texels;
        ShadowRecord {
            live: UVec4::ZERO,
            to_ship: [Vec4::X, Vec4::Y, Vec4::Z],
            sphere: centre.extend(radius),
            map: Vec4::new(texels, 1.0 / texels, texel_m, 1.0 - SEAT_SLICE),
            // (A depth of 1 is the sphere's far side; a pixel behind the ship is compared halfway into the margin, beyond every
            // caster and short of the clear value's depth, 1.)
            bias: Vec4::new(NORMAL_OFFSET_TEXELS * texel_m, FAR_MARGIN_TEXELS / texels, 1.0 - 0.5 * MARGIN_M / (2.0 * radius), WRITE_SLOPE),
            msm: Vec4::new(MOMENT_OFFSET, MOMENT_BIAS, LEAK_BOUND, 0.0),
            read: MOMENT_READ.map(Vec4::from_array),
            write: moment_write(),
        }
    }
}

/// The shadow's state: the casters, the maps and buffers the passes and the ship's material bind, and this frame's record.
/// Made at the first spawn (`new`, from `draw::make`); extracted each frame it changes.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct ShipShadow {
    casters: Arc<Casters>,
    /// The moments map (`MAP_TEXELS` square, `LAYERS` layers), drawn only by the render world.
    pub moments: Handle<Image>,
    format: TextureFormat,
    /// The record (`ShadowRecord`), written only by the render world (`values`, then `live` by the map pass).
    pub record: Handle<ShaderBuffer>,
    /// Each rig node's matrix in the ship's space (zero for a hidden node), written in place when the pose changes.
    parts: Handle<ShaderBuffer>,
    palette: Handle<ShaderBuffer>,
    frame: Handle<ShaderBuffer>,
    light: Handle<ShaderBuffer>,
    values: ShadowRecord,
    /// The maps are drawn this frame (a ship is drawn and the light record is on), and the seat's too (seated).
    active: bool,
    seated: bool,
    nodes: usize,
    /// The parts' revisions last written (`Parts::revision`, `shown_revision`).
    written: Option<(u64, u64)>,
    /// `SE_SHIP_SHADOW=0` leaves the shadow out (comparisons): nothing runs and the ship reads lit.
    enabled: bool,
}

impl ShipShadow {
    pub fn new(images: &mut Assets<Image>, buffers: &mut Assets<ShaderBuffer>, light: &PropLight, palette: &ShipPalette, casters: Casters, nodes: usize, device: Option<&RenderDevice>) -> ShipShadow {
        let format = if device.is_some_and(|d| d.features().contains(WgpuFeatures::TEXTURE_FORMAT_16BIT_NORM)) {
            MAP_FORMAT
        } else {
            warn!("sidewinder: no 16-bit fixed-point textures on this GPU: the shadow's moments are half floats, less precise");
            MAP_FALLBACK
        };
        let mut map = Image::new_uninit(Extent3d { width: MAP_TEXELS, height: MAP_TEXELS, depth_or_array_layers: LAYERS }, TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
        map.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT;
        map.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
        // (Elite's ClampSampler: linear, clamped; the gathers read it unfiltered.)
        map.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        let values = ShadowRecord::new(casters.centre, casters.radius);
        info!(
            "sidewinder: the shadow's casters: {} triangles outside, {} seated, {} vertices; its maps {MAP_TEXELS} texels over a sphere of {:.2} m round ({:.2}, {:.2}, {:.2}) ({:.1} mm a texel), moments {format:?}",
            casters.outside.len() / 3,
            casters.seat.len() / 3,
            casters.vertices.len(),
            casters.radius,
            casters.centre.x,
            casters.centre.y,
            casters.centre.z,
            values.map.z * 1000.0
        );
        ShipShadow {
            casters: Arc::new(casters),
            moments: images.add(map),
            format,
            record: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&values), RenderAssetUsages::RENDER_WORLD)),
            parts: buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&vec![Mat4::IDENTITY; nodes.max(1)]), RenderAssetUsages::default())),
            palette: palette.buffer.clone(),
            frame: light.frame.clone(),
            light: light.light.clone(),
            values,
            active: false,
            seated: false,
            nodes,
            written: None,
            enabled: std::env::var("SE_SHIP_SHADOW").map_or(true, |v| v != "0"),
        }
    }
}

/// This frame's state from the ship as drawn (`draw::place` ran): active, seated, the ship's space and the parts' matrices.
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

// ---- The render world ----

/// The passes' layouts and pipelines, queued once at the first frame with a ship.
#[derive(Resource)]
struct ShadowPipelines {
    map_layout: BindGroupLayoutDescriptor,
    moments: CachedRenderPipelineId,
    far: CachedRenderPipelineId,
    world_layouts: [BindGroupLayoutDescriptor; 2],
    world: [CachedRenderPipelineId; 2],
}

fn queue_pipelines(mut commands: Commands, shadow: Res<ShipShadow>, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let shader: Handle<Shader> = assets.load("shaders/sidewinder_shadow.wesl");
    let stages = ShaderStages::VERTEX_FRAGMENT;
    let map_layout = BindGroupLayoutDescriptor::new(
        "sidewinder_shadow_map",
        &BindGroupLayoutEntries::with_indices(
            stages,
            (
                (0, storage_buffer_read_only_sized(false, None)),
                (1, storage_buffer_read_only_sized(false, None)),
                (2, storage_buffer_read_only_sized(false, None)),
                (3, storage_buffer_read_only_sized(false, None)),
                (4, storage_buffer_read_only_sized(false, None)),
            ),
        ),
    );
    // (`CasterVertex`, packed: position, node, bone at locations 0, 1, 2.)
    let vertex_buffer = VertexBufferLayout::from_vertex_formats(VertexStepMode::Vertex, [VertexFormat::Float32x3, VertexFormat::Uint32, VertexFormat::Uint32]);
    let vertex = VertexState { shader: shader.clone(), entry_point: Some("map_vertex".into()), buffers: vec![vertex_buffer], ..default() };
    let depth = |compare, bias| DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(compare), stencil: default(), bias };
    let moments = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("sidewinder_shadow_moments".into()),
        layout: vec![map_layout.clone()],
        vertex: vertex.clone(),
        fragment: Some(FragmentState { shader: shader.clone(), entry_point: Some("map_moments".into()), targets: vec![Some(ColorTargetState { format: shadow.format, blend: None, write_mask: ColorWrites::ALL })], ..default() }),
        primitive: PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: None, ..default() },
        depth_stencil: Some(depth(CompareFunction::Less, DepthBiasState::default())),
        ..default()
    });
    let far = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("sidewinder_shadow_far".into()),
        layout: vec![map_layout.clone()],
        vertex,
        fragment: None,
        primitive: PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: None, ..default() },
        depth_stencil: Some(depth(CompareFunction::Greater, DepthBiasState { constant: 0, slope_scale: FAR_SLOPE, clamp: 0.0 })),
        ..default()
    });
    let world_layouts = [false, true].map(|msaa| {
        BindGroupLayoutDescriptor::new(
            "sidewinder_shadow_world",
            &BindGroupLayoutEntries::with_indices(
                stages,
                (
                    (0, storage_buffer_read_only_sized(false, None)),
                    (3, storage_buffer_read_only_sized(false, None)),
                    (4, storage_buffer_read_only_sized(false, None)),
                    (5, uniform_buffer::<ViewUniform>(true)),
                    (6, if msaa { texture_depth_2d_multisampled() } else { texture_depth_2d() }),
                    (7, texture_2d_array(TextureSampleType::Float { filterable: true })),
                    (8, sampler(SamplerBindingType::Filtering)),
                    (9, texture_depth_2d()),
                ),
            ),
        )
    });
    // (What the main pass drew times the factor; its alpha kept.)
    let times = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::Src, operation: BlendOperation::Add };
    let keep = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    let world = [0, 1].map(|i| {
        let defs: Vec<ShaderDefVal> = if i == 1 { vec!["MSAA".into()] } else { Vec::new() };
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("sidewinder_shadow_world".into()),
            layout: vec![world_layouts[i].clone()],
            vertex: VertexState { shader: shader.clone(), shader_defs: defs.clone(), entry_point: Some("world_vertex".into()), ..default() },
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: defs,
                entry_point: Some("world_fragment".into()),
                targets: vec![Some(ColorTargetState { format: HDR_FORMAT, blend: Some(BlendState { color: times, alpha: keep }), write_mask: ColorWrites::ALL })],
                ..default()
            }),
            primitive: PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: None, ..default() },
            ..default()
        })
    });
    info!("sidewinder: the shadow's 4 pipelines are queued");
    commands.insert_resource(ShadowPipelines { map_layout, moments, far, world_layouts, world });
}

/// The render world's own: the casters' buffers (for the copy they were made from), the depth targets, a word of 1 for
/// `live`, and whether the record was last written active.
#[derive(Resource)]
struct ShadowGpu {
    casters: usize,
    vertices: Buffer,
    indices: Buffer,
    /// The moments passes' depth test (nearest caster), then the furthest caster's depth.
    near: TextureView,
    far: TextureView,
    one: Buffer,
    written_active: bool,
}

fn depth_target(device: &RenderDevice, label: &'static str, usage: TextureUsages) -> TextureView {
    device
        .create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d { width: MAP_TEXELS, height: MAP_TEXELS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage,
            view_formats: &[],
        })
        .create_view(&TextureViewDescriptor::default())
}

/// The casters' buffers and the targets (once, and anew for a new copy of the casters); the record's values with `live` 0
/// each frame the maps are drawn, and once when they stop.
fn prepare(mut commands: Commands, shadow: Res<ShipShadow>, gpu: Option<ResMut<ShadowGpu>>, device: Res<RenderDevice>, queue: Res<RenderQueue>, buffers: Res<RenderAssets<GpuShaderBuffer>>) {
    let key = Arc::as_ptr(&shadow.casters) as usize;
    let Some(mut gpu) = gpu.filter(|g| g.casters == key) else {
        let c = &shadow.casters;
        commands.insert_resource(ShadowGpu {
            casters: key,
            vertices: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_shadow_vertices"), contents: bytemuck::cast_slice(&c.vertices), usage: BufferUsages::VERTEX }),
            indices: device.create_buffer_with_data(&BufferInitDescriptor { label: Some("sidewinder_shadow_indices"), contents: bytemuck::cast_slice(&c.indices), usage: BufferUsages::INDEX }),
            near: depth_target(&device, "sidewinder_shadow_near", TextureUsages::RENDER_ATTACHMENT),
            far: depth_target(&device, "sidewinder_shadow_far", TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING),
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
    gpu.written_active = shadow.active;
}

/// The bind group and the layers' views the map pass made, kept with the ids they were made for.
struct MapBinds {
    ids: [BufferId; 5],
    texture: TextureId,
    group: BindGroup,
    layers: Vec<TextureView>,
}

/// The maps, once per frame (the main view only) before the opaque pass: the outside set's moments and, seated, the seat's;
/// the outside set's furthest depth; then `live` set.
#[allow(clippy::too_many_arguments)]
fn map_pass(
    _view: ViewQuery<&ExtractedView, MainView>,
    shadow: Res<ShipShadow>,
    gpu: Option<Res<ShadowGpu>>,
    pipes: Res<ShadowPipelines>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    cache: Res<PipelineCache>,
    mut binds: Local<Option<MapBinds>>,
    mut ctx: RenderContext,
) {
    let (true, Some(gpu)) = (shadow.active, gpu) else { return };
    let (Some(moments), Some(far)) = (cache.get_render_pipeline(pipes.moments), cache.get_render_pipeline(pipes.far)) else { return };
    let (Some(record), Some(parts), Some(palette), Some(frame), Some(light), Some(map)) =
        (buffers.get(&shadow.record), buffers.get(&shadow.parts), buffers.get(&shadow.palette), buffers.get(&shadow.frame), buffers.get(&shadow.light), images.get(&shadow.moments))
    else {
        return;
    };
    let ids = [record, parts, palette, frame, light].map(|b| b.buffer.id());
    if binds.as_ref().is_none_or(|b| b.ids != ids || b.texture != map.texture.id()) {
        let layout = cache.get_bind_group_layout(&pipes.map_layout);
        let group = ctx.render_device().create_bind_group(
            "sidewinder_shadow_map",
            &layout,
            &BindGroupEntries::with_indices((
                (0, record.buffer.as_entire_binding()),
                (1, parts.buffer.as_entire_binding()),
                (2, palette.buffer.as_entire_binding()),
                (3, frame.buffer.as_entire_binding()),
                (4, light.buffer.as_entire_binding()),
            )),
        );
        let layers = (0..LAYERS)
            .map(|i| map.texture.create_view(&TextureViewDescriptor { dimension: Some(TextureViewDimension::D2), base_array_layer: i, array_layer_count: Some(1), ..default() }))
            .collect();
        *binds = Some(MapBinds { ids, texture: map.texture.id(), group, layers });
    }
    let Some(b) = binds.as_ref() else { return };
    let clear = quantised(1.0);
    let clear = LoadOp::Clear(LinearRgba::new(clear.x, clear.y, clear.z, clear.w).into());
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "sidewinder_shadow_maps");
    let encoder = ctx.command_encoder();
    let sets = [(0usize, shadow.casters.outside.clone())].into_iter().chain(shadow.seated.then(|| (1usize, shadow.casters.seat.clone())));
    for (layer, range) in sets {
        let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("sidewinder_shadow_moments"),
            color_attachments: &[Some(RenderPassColorAttachment { view: &b.layers[layer], depth_slice: None, resolve_target: None, ops: Operations { load: clear, store: StoreOp::Store } })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment { view: &gpu.near, depth_ops: Some(Operations { load: LoadOp::Clear(1.0), store: StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        p.set_pipeline(moments);
        p.set_bind_group(0, &b.group, &[]);
        p.set_vertex_buffer(0, *gpu.vertices.slice(..));
        p.set_index_buffer(*gpu.indices.slice(..), IndexFormat::Uint32);
        p.draw_indexed(range, 0, 0..1);
    }
    {
        let mut p = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("sidewinder_shadow_far"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment { view: &gpu.far, depth_ops: Some(Operations { load: LoadOp::Clear(0.0), store: StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        p.set_pipeline(far);
        p.set_bind_group(0, &b.group, &[]);
        p.set_vertex_buffer(0, *gpu.vertices.slice(..));
        p.set_index_buffer(*gpu.indices.slice(..), IndexFormat::Uint32);
        p.draw_indexed(shadow.casters.outside.clone(), 0, 0..1);
    }
    // (`live` is the record's first word.)
    encoder.copy_buffer_to_buffer(&gpu.one, 0, &record.buffer, 0, Some(4));
    span.end(ctx.command_encoder());
}

/// The shadow on everything else the main pass drew, before the clouds (the main view only), while the maps are drawn.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn world_pass(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &ExtractedCamera, &ViewUniformOffset, &Msaa, Option<&MainPassResolutionOverride>), With<CloudView>>,
    shadow: Res<ShipShadow>,
    gpu: Option<Res<ShadowGpu>>,
    pipes: Res<ShadowPipelines>,
    uniforms: Res<ViewUniforms>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, depth, camera, offset, msaa, low) = view.into_inner();
    let (true, Some(gpu)) = (shadow.active, gpu) else { return };
    let m = (msaa.samples() > 1) as usize;
    let (Some(pipeline), Some(view_binding)) = (cache.get_render_pipeline(pipes.world[m]), uniforms.uniforms.binding()) else { return };
    let (Some(record), Some(frame), Some(light), Some(map)) = (buffers.get(&shadow.record), buffers.get(&shadow.frame), buffers.get(&shadow.light), images.get(&shadow.moments)) else { return };
    let Some(solid) = low.map(|l| l.0).or(camera.physical_viewport_size).filter(|s| s.x > 0 && s.y > 0) else { return };
    if target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let device = ctx.render_device().clone();
    let depth_view = depth.texture().create_view(&TextureViewDescriptor { aspect: TextureAspect::DepthOnly, ..default() });
    let group = device.create_bind_group(
        "sidewinder_shadow_world",
        &cache.get_bind_group_layout(&pipes.world_layouts[m]),
        &BindGroupEntries::with_indices((
            (0, record.buffer.as_entire_binding()),
            (3, frame.buffer.as_entire_binding()),
            (4, light.buffer.as_entire_binding()),
            (5, view_binding),
            (6, &depth_view),
            (7, &map.texture_view),
            (8, &map.sampler),
            (9, &gpu.far),
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "sidewinder_shadow_world");
    let scene = RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } };
    let mut p = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor { label: Some("sidewinder_shadow_world"), color_attachments: &[Some(scene)], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
    p.set_viewport(0.0, 0.0, solid.x as f32, solid.y as f32, 0.0, 1.0);
    p.set_pipeline(pipeline);
    p.set_bind_group(0, &group, &[offset.offset]);
    p.draw(0..3, 0..1);
    drop(p);
    span.end(ctx.command_encoder());
}

pub fn build(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<ShipShadow>::default()).add_systems(
        PostUpdate,
        update.run_if(resource_exists::<ShipShadow>).after(super::draw::place).before(crate::universe::UniverseSet::FloatingOrigin),
    );
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        // (Nothing before the first spawn: everything waits for the extracted `ShipShadow`.)
        .add_systems(Render, queue_pipelines.in_set(RenderSystems::PrepareResources).run_if(resource_exists::<ShipShadow>).run_if(not(resource_exists::<ShadowPipelines>)))
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources).run_if(resource_exists::<ShipShadow>))
        .add_systems(Core3d, map_pass.in_set(Core3dSystems::Prepass).before(main_opaque_pass_3d).run_if(resource_exists::<ShadowPipelines>))
        .add_systems(Core3d, world_pass.in_set(Core3dSystems::EarlyPostProcess).before(CloudsPassSet).run_if(resource_exists::<ShadowPipelines>));
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::assets::{self, Loaded};

    fn shipped() -> Loaded {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
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

    /// The light's frame covers every caster vertex for any star direction and any gear state: every vertex in poses four
    /// times finer than the sphere was made from lies inside it, so on the map's square and in its depth range.
    #[test]
    fn the_map_covers_every_caster() {
        let Loaded { assets, build } = shipped();
        let rig = &assets.rig;
        let c = Casters::new(rig, &build.meshes, &build.materials);
        let dirs = directions();
        let mut most = 0.0f32;
        for pose in gear_poses(rig, GEAR_SAMPLES * 4) {
            for v in &c.vertices {
                let p = pose[v.node as usize].transform_point3(Vec3::from_array(v.pos));
                most = most.max(p.distance(c.centre));
                for &l in &dirs {
                    let m = map_point(c.centre, c.radius, l, p);
                    assert!(m.x.abs() <= 1.0 && m.y.abs() <= 1.0 && (0.0..=1.0).contains(&m.z), "{p} at {l}: {m}");
                }
            }
        }
        assert!(most <= c.radius - 0.5 * MARGIN_M, "a caster {most} m out, the sphere {} m", c.radius);
        println!("the sphere: {:.3} m round {}, the furthest caster {most:.3} m out", c.radius, c.centre);
    }

    /// The outside set rasterised onto the map as the passes do (texel centres), the gear down and up, from every direction:
    /// wherever a caster is, the furthest depth is never nearer than the nearest, and a pixel behind the ship is compared at a
    /// depth beyond every caster (`bias.z`), so the filter can say it is in shadow.
    #[test]
    fn the_furthest_depth_is_behind_the_nearest() {
        let Loaded { assets, build } = shipped();
        let rig = &assets.rig;
        let c = Casters::new(rig, &build.meshes, &build.materials);
        let record = ShadowRecord::new(c.centre, c.radius);
        const N: usize = 256;
        for pose in gear_poses(rig, 1) {
            for l in directions().into_iter().step_by(7) {
                let (mut near, mut far) = (vec![f32::MAX; N * N], vec![f32::MIN; N * N]);
                let pts: Vec<Vec3> = c.vertices.iter().map(|v| map_point(c.centre, c.radius, l, pose[v.node as usize].transform_point3(Vec3::from_array(v.pos)))).collect();
                for t in c.indices[c.outside.start as usize..c.outside.end as usize].chunks(3) {
                    let [a, b, e] = [pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]];
                    let texel = |p: Vec3| Vec2::new((p.x * 0.5 + 0.5) * N as f32, (0.5 - p.y * 0.5) * N as f32);
                    let (ta, tb, te) = (texel(a), texel(b), texel(e));
                    let area = (tb - ta).perp_dot(te - ta);
                    if area.abs() < 1e-12 {
                        continue;
                    }
                    let lo = ta.min(tb).min(te).floor().max(Vec2::ZERO);
                    let hi = ta.max(tb).max(te).ceil().min(Vec2::splat(N as f32));
                    for y in lo.y as usize..hi.y as usize {
                        for x in lo.x as usize..hi.x as usize {
                            let s = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                            let (w1, w2) = ((s - ta).perp_dot(te - ta) / area, (tb - ta).perp_dot(s - ta) / area);
                            let w0 = 1.0 - w1 - w2;
                            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                                continue;
                            }
                            let d = w0 * a.z + w1 * b.z + w2 * e.z;
                            near[y * N + x] = near[y * N + x].min(d);
                            far[y * N + x] = far[y * N + x].max(d);
                        }
                    }
                }
                let covered: Vec<usize> = (0..N * N).filter(|&i| far[i] >= 0.0 && near[i] < f32::MAX).collect();
                assert!(!covered.is_empty(), "nothing on the map at {l}");
                for &i in &covered {
                    assert!(far[i] >= near[i], "texel {i} at {l}: far {} near {}", far[i], near[i]);
                    assert!(far[i] + record.bias.y < record.bias.z, "texel {i} at {l}: far {} against the receivers' {}", far[i], record.bias.z);
                }
            }
        }
    }
}
