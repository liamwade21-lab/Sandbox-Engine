//! Elite's cockpit UI's own place in the frame (ARCHITECTURE.md A10, UI3b): a pass of the mod's own in the engine's `Core3d`
//! schedule that draws the movies' panels and their dark plates AFTER the engine's colour passes, at each view's full size,
//! into its display-referred main texture: never in the solids' reduced-size corner, never before the bloom, the exposure
//! or the tone curve (Elite draws its HUD at the screen's own size and outside its bloom).
//!
//! - **Where**: `ui_pass` runs in `Core3dSystems::PostProcess` after the colour correction, FXAA and SMAA, before
//!   Bevy's upscaling to the window; into `ViewTarget::main_texture_view` (Rgba16Float, linear, display-referred: the
//!   swapchain's copy encodes sRGB), loaded, at the view's full size. The views: every one with the engine's colour
//!   correction (`ColorCorrection`: the main camera and the finished capture views, `capture.rs`, which follow it), only while
//!   the main world says the seat's cockpit is drawn (`UiFrame::order` empty otherwise).
//! - **What it binds**: group 0, made per view and kept until its depth texture or a buffer changes: the view's `View`
//!   uniform (dynamic offset), the pass's values (`UiGlobalsGpu`), the pieces' records (`PieceRecord`: model matrix, the ship's
//!   own scale, the depth flag; written in place each frame), the scene's depth texture. Group 1, made once per material
//!   when its textures exist (a new ship makes new ones): the material's values and its textures with their own samplers.
//!   The pieces' geometry is one vertex and one index buffer made when the UI is made; each piece is one indexed draw whose
//!   instance index is its record.
//! - **Depth**: Elite's flag per material decides. With it on, the fragment is tested in its shader against the scene's
//!   depth, read from the view's depth texture at the texel under its pixel (the main pass fills only the texture's top-left
//!   corner under the render scale: `view.main_pass_viewport`), reversed depth, after the fragment is moved towards the eye
//!   by `DEPTH_BIAS` (1 % of its distance, `sidewinder_ui_common.wesl` says why).
//! - **Order, blend, culling**: as Elite's data gives them: the plates first, then the panels by their render layer; each
//!   material's blend and culling from its flags (`GuiFlags`).
//!
//! **The seam** the scanner's and the holograms' passes draw through (`scanner.rs`, `hologram.rs`):
//! - they run `.after(UiPassSet)` (and `.in_set(Core3dSystems::PostProcess)`), on the same views (`With<ColorCorrection>`),
//!   into `ViewTarget::main_texture_view()` with `LoadOp::Load`, at the view's full size (`ExtractedCamera`
//!   `physical_viewport_size`; the view's `viewport` in the `View` uniform);
//! - they import from `sidewinder_ui_common.wesl`: `Globals` (the record `UiFrame::globals` holds, `UiGlobalsGpu`),
//!   `seat_z` (the seat's depth slice), `depth_texel` and `ui_in_front` (the depth test against the view's depth texture,
//!   the bias in that file only) and `screen_fit` (Elite's value to the screen's colour, written as it is).
//! - the same conditions to draw: `UiFrame::order` non-empty is "the seat's cockpit is drawn this frame".

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only_sized, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::mesh::VertexBufferLayout;
use bevy::shader::ShaderDefVal;
use bytemuck::{Pod, Zeroable};

use crate::color_correction::ColorCorrection;

use super::material::GuiFlags;

const PASS_SHADER: &str = "shaders/sidewinder_ui_pass.wesl";

/// The main texture of an HDR view (the scanner's and the holograms' passes draw into it too).
pub(super) const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// One vertex of a piece: position, normal, tangent (its sign in w), uv0, in the piece's bone's space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct UiVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv: [f32; 2],
}

/// Which of Elite's two GUI shaders a material is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    /// `GuiLighting` (tag `fg`): a movie on its panel.
    Panel,
    /// `GuiBackground` (tag `bg`): a dark plate.
    Plate,
}

/// A material: its shader, its flags, its values (the shader's uniform, encoded), its textures in binding order (a panel:
/// the movie's target, the cloudiness; a plate: the alpha shape, the cloudiness, the scanlines).
pub struct UiMaterialData {
    pub kind: PieceKind,
    pub flags: GuiFlags,
    pub params: Vec<u8>,
    pub textures: Vec<Handle<Image>>,
}

/// A piece's draw in the geometry's buffers.
#[derive(Clone, Copy, Debug)]
pub struct PieceGeom {
    pub first: u32,
    pub count: u32,
    pub base: i32,
    pub material: u32,
}

/// Every piece's geometry and every material, made once when the UI is made (`place::make`).
#[derive(Default)]
pub struct UiGeometry {
    pub vertices: Vec<UiVertex>,
    pub indices: Vec<u32>,
    pub pieces: Vec<PieceGeom>,
    pub materials: Vec<UiMaterialData>,
}

/// One piece this frame (`sidewinder_ui_pass.wesl` `Piece`): its model matrix in the frame's world, `info` x the ship's own
/// scale (the seat's depth slice's), y 1 when its material tests its depth.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct PieceRecord {
    pub model: [[f32; 4]; 4],
    pub info: [f32; 4],
}

/// The pass's values (`sidewinder_ui_pass.wesl` `Globals`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct UiGlobalsGpu {
    /// x gGuiBrightness, y gExposureCompensation, z gHDRScalingWrite, w gGuiBackgroundMultiplier.
    pub gui: [f32; 4],
    /// x the seat's depth slice (`SEAT_SLICE`), y its near plane (`SEAT_NEAR_M`).
    pub seat: [f32; 4],
    /// The screen fit's A, p, B (`material::SCREEN_FIT`).
    pub fit: [f32; 4],
    /// The GUI colour matrix's rows Red, Green, Blue.
    pub matrix: [[f32; 4]; 3],
}

/// What the pass draws this frame, made by `place::place` and extracted: the geometry (an `Arc`: the render world makes its
/// buffers again only when it is a new one), every piece's record, the pieces to draw in order (empty: nothing is drawn).
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct UiFrame {
    pub geometry: Option<Arc<UiGeometry>>,
    pub records: Vec<PieceRecord>,
    pub order: Vec<u32>,
    pub globals: UiGlobalsGpu,
}

/// Shared by both worlds: a pipeline of the pass failed (the main world then turns the UI off).
#[derive(Resource, Clone, Default)]
pub struct Built(Arc<AtomicBool>);

impl Built {
    pub fn failed(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// A new UI: its pipelines are looked at again.
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// The pass's place in `Core3d`: the scanner's and the holograms' passes run after it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UiPassSet;

#[derive(Resource)]
struct PassLayouts {
    /// The view's group, without and with MSAA (the depth texture's kind).
    view: [BindGroupLayoutDescriptor; 2],
    panel: BindGroupLayoutDescriptor,
    plate: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

/// A view's group's key: the depth texture and the buffers it binds.
type ViewKey = (TextureId, BufferId, BufferId, BufferId);

#[derive(Resource, Default)]
struct PassGpu {
    /// The geometry the buffers were made from (`Arc::as_ptr`).
    key: usize,
    vertices: Option<Buffer>,
    indices: Option<Buffer>,
    records: Option<Buffer>,
    globals: Option<Buffer>,
    written: Option<UiGlobalsGpu>,
    params: Vec<Buffer>,
    materials: Vec<Option<BindGroup>>,
    /// Each material's pipeline, without and with MSAA.
    pipelines: Vec<[CachedRenderPipelineId; 2]>,
    views: HashMap<Entity, (ViewKey, BindGroup)>,
    logged: bool,
}

fn init_layouts(mut commands: Commands, assets: Res<AssetServer>) {
    let view = [false, true].map(|msaa| {
        BindGroupLayoutDescriptor::new(
            "sidewinder_ui_view",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    if msaa { texture_depth_2d_multisampled() } else { texture_depth_2d() },
                ),
            ),
        )
    });
    let f = TextureSampleType::Float { filterable: true };
    let s = SamplerBindingType::Filtering;
    let panel = BindGroupLayoutDescriptor::new(
        "sidewinder_ui_panel",
        &BindGroupLayoutEntries::sequential(ShaderStages::VERTEX_FRAGMENT, (uniform_buffer_sized(false, None), texture_2d(f), sampler(s), texture_2d(f), sampler(s))),
    );
    let plate = BindGroupLayoutDescriptor::new(
        "sidewinder_ui_plate",
        &BindGroupLayoutEntries::sequential(ShaderStages::VERTEX_FRAGMENT, (uniform_buffer_sized(false, None), texture_2d(f), sampler(s), texture_2d(f), sampler(s), texture_2d(f), sampler(s))),
    );
    commands.insert_resource(PassLayouts { view, panel, plate, shader: assets.load(PASS_SHADER) });
}

/// A material's pipeline: its shader's half, its culling and blend, the depth texture's kind.
fn pipeline(layouts: &PassLayouts, m: &UiMaterialData, msaa: bool) -> RenderPipelineDescriptor {
    let (def, group) = match m.kind {
        PieceKind::Panel => ("PANEL", &layouts.panel),
        PieceKind::Plate => ("PLATE", &layouts.plate),
    };
    let defs: Vec<ShaderDefVal> = vec![def.into(), if msaa { "MSAA" } else { "NO_MSAA" }.into()];
    let mut target = ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: ColorWrites::ALL };
    let mut primitive = PrimitiveState { topology: PrimitiveTopology::TriangleList, ..default() };
    m.flags.apply(&mut primitive, &mut target);
    RenderPipelineDescriptor {
        label: Some(format!("sidewinder_ui_{}", def.to_lowercase()).into()),
        layout: vec![layouts.view[msaa as usize].clone(), group.clone()],
        vertex: VertexState {
            shader: layouts.shader.clone(),
            shader_defs: defs.clone(),
            entry_point: Some("vertex".into()),
            buffers: vec![VertexBufferLayout::from_vertex_formats(VertexStepMode::Vertex, [VertexFormat::Float32x3, VertexFormat::Float32x3, VertexFormat::Float32x4, VertexFormat::Float32x2])],
            ..default()
        },
        fragment: Some(FragmentState { shader: layouts.shader.clone(), shader_defs: defs, entry_point: Some("fragment".into()), targets: vec![Some(target)], ..default() }),
        primitive,
        ..default()
    }
}

/// The frame's buffers and groups: made when a new UI's geometry arrives, its records and values written in place each
/// frame, the materials' groups made once their textures exist; a pipeline's failure reported to the main world.
#[allow(clippy::too_many_arguments)]
fn prepare(
    frame: Res<UiFrame>,
    layouts: Res<PassLayouts>,
    mut gpu: ResMut<PassGpu>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    built: Res<Built>,
) {
    let Some(geometry) = frame.geometry.as_ref() else {
        if gpu.key != 0 {
            *gpu = PassGpu::default();
        }
        return;
    };
    let key = Arc::as_ptr(geometry) as usize;
    if gpu.key != key {
        let init = |label: &'static str, contents: &[u8], usage: BufferUsages| device.create_buffer_with_data(&BufferInitDescriptor { label: Some(label), contents, usage });
        let record_bytes = (geometry.pieces.len().max(1) * std::mem::size_of::<PieceRecord>()) as u64;
        *gpu = PassGpu {
            key,
            vertices: Some(init("sidewinder_ui_vertices", bytemuck::cast_slice(&geometry.vertices), BufferUsages::VERTEX)),
            indices: Some(init("sidewinder_ui_indices", bytemuck::cast_slice(&geometry.indices), BufferUsages::INDEX)),
            records: Some(device.create_buffer(&BufferDescriptor { label: Some("sidewinder_ui_records"), size: record_bytes, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, mapped_at_creation: false })),
            globals: Some(init("sidewinder_ui_globals", bytemuck::bytes_of(&frame.globals), BufferUsages::UNIFORM | BufferUsages::COPY_DST)),
            written: Some(frame.globals),
            params: geometry.materials.iter().map(|m| init("sidewinder_ui_params", &m.params, BufferUsages::UNIFORM)).collect(),
            materials: geometry.materials.iter().map(|_| None).collect(),
            pipelines: geometry.materials.iter().map(|m| [false, true].map(|msaa| cache.queue_render_pipeline(pipeline(&layouts, m, msaa)))).collect(),
            views: HashMap::new(),
            logged: false,
        };
    }
    let gpu = &mut *gpu;
    if let Some(records) = gpu.records.as_ref().filter(|_| !frame.order.is_empty() && frame.records.len() == geometry.pieces.len()) {
        queue.write_buffer(records, 0, bytemuck::cast_slice(&frame.records));
    }
    if gpu.written != Some(frame.globals)
        && let Some(globals) = gpu.globals.as_ref()
    {
        queue.write_buffer(globals, 0, bytemuck::bytes_of(&frame.globals));
        gpu.written = Some(frame.globals);
    }
    for (k, m) in geometry.materials.iter().enumerate() {
        if gpu.materials[k].is_some() {
            continue;
        }
        let textures: Option<Vec<&GpuImage>> = m.textures.iter().map(|h| images.get(h)).collect();
        let Some(textures) = textures else { continue };
        let layout = cache.get_bind_group_layout(match m.kind {
            PieceKind::Panel => &layouts.panel,
            PieceKind::Plate => &layouts.plate,
        });
        let mut entries = vec![BindGroupEntry { binding: 0, resource: gpu.params[k].as_entire_binding() }];
        for (i, t) in textures.iter().enumerate() {
            entries.push(BindGroupEntry { binding: 1 + 2 * i as u32, resource: BindingResource::TextureView(&t.texture_view) });
            entries.push(BindGroupEntry { binding: 2 + 2 * i as u32, resource: BindingResource::Sampler(&t.sampler) });
        }
        gpu.materials[k] = Some(device.create_bind_group("sidewinder_ui_material", &layout, &entries));
    }
    // (Every material's pipeline without MSAA and with; one failed: logged once, the UI turned off.)
    for id in gpu.pipelines.iter().flatten() {
        if let CachedPipelineState::Err(e) = cache.get_render_pipeline_state(*id)
            && !std::mem::replace(&mut gpu.logged, true)
        {
            error!("sidewinder ui: a pipeline of the UI's pass failed: {e}");
            built.0.store(true, Ordering::Relaxed);
        }
    }
}

/// Each view's group: kept until its depth texture or one of the buffers changes; a view gone, its group goes.
fn prepare_views(
    views: Query<(Entity, &ViewDepthStencilTexture, &Msaa), With<ColorCorrection>>,
    layouts: Res<PassLayouts>,
    mut gpu: ResMut<PassGpu>,
    uniforms: Res<ViewUniforms>,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
) {
    let gpu = &mut *gpu;
    let (Some(records), Some(globals), Some(view_buffer), Some(view_binding)) = (gpu.records.as_ref(), gpu.globals.as_ref(), uniforms.uniforms.buffer(), uniforms.uniforms.binding()) else {
        gpu.views.clear();
        return;
    };
    gpu.views.retain(|e, _| views.contains(*e));
    for (entity, depth, msaa) in &views {
        let key = (depth.texture().id(), view_buffer.id(), records.id(), globals.id());
        if gpu.views.get(&entity).is_some_and(|(k, _)| *k == key) {
            continue;
        }
        let depth_view = depth.texture().create_view(&TextureViewDescriptor { aspect: TextureAspect::DepthOnly, ..default() });
        let group = device.create_bind_group(
            "sidewinder_ui_view",
            &cache.get_bind_group_layout(&layouts.view[(msaa.samples() > 1) as usize]),
            &BindGroupEntries::sequential((view_binding.clone(), globals.as_entire_binding(), records.as_entire_binding(), &depth_view)),
        );
        gpu.views.insert(entity, (key, group));
    }
}

/// The pass: the frame's pieces in order into the view's main texture, at its full size.
fn ui_pass(view: ViewQuery<(Entity, &ViewTarget, &ViewUniformOffset, &Msaa), With<ColorCorrection>>, frame: Res<UiFrame>, gpu: Res<PassGpu>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    let (entity, target, offset, msaa) = view.into_inner();
    let (Some(geometry), Some(vertices), Some(indices), Some((_, view_group))) = (frame.geometry.as_ref(), gpu.vertices.as_ref(), gpu.indices.as_ref(), gpu.views.get(&entity)) else { return };
    if frame.order.is_empty() || target.main_texture_format() != HDR_FORMAT || gpu.key != Arc::as_ptr(geometry) as usize {
        return;
    }
    let m = (msaa.samples() > 1) as usize;
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "sidewinder_ui");
    let scene = RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } };
    let encoder = ctx.command_encoder();
    let mut p = encoder.begin_render_pass(&RenderPassDescriptor { label: Some("sidewinder_ui"), color_attachments: &[Some(scene)], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
    p.set_bind_group(0, view_group, &[offset.offset]);
    p.set_vertex_buffer(0, *vertices.slice(..));
    p.set_index_buffer(*indices.slice(..), IndexFormat::Uint32);
    for &i in &frame.order {
        let Some(piece) = geometry.pieces.get(i as usize) else { continue };
        let k = piece.material as usize;
        let (Some(Some(group)), Some(pipeline)) = (gpu.materials.get(k), gpu.pipelines.get(k).and_then(|p| cache.get_render_pipeline(p[m]))) else { continue };
        p.set_pipeline(pipeline);
        p.set_bind_group(1, group, &[]);
        p.draw_indexed(piece.first..piece.first + piece.count, piece.base, i..i + 1);
    }
    drop(p);
    span.end(ctx.command_encoder());
}

/// The capture views' depth is read by the pass (the main camera's already is, `camera.rs`): made readable as `ship/warp.rs`
/// makes the main camera's.
fn capture_depth_readable(mut views: Query<&mut Camera3d, With<crate::capture::CaptureView>>) {
    for mut c in &mut views {
        let usage = TextureUsages::from(c.depth_texture_usages);
        if !usage.contains(TextureUsages::TEXTURE_BINDING) {
            c.depth_texture_usages = (usage | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}

pub(super) fn plugin(app: &mut App, built: Built) {
    app.add_plugins(ExtractResourcePlugin::<UiFrame>::default()).init_resource::<UiFrame>().add_systems(PostUpdate, capture_depth_readable.run_if(resource_exists::<super::place::UiDraw>));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .insert_resource(built)
        .init_resource::<PassGpu>()
        .add_systems(RenderStartup, init_layouts)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
        .add_systems(Render, prepare_views.in_set(RenderSystems::PrepareBindGroups))
        .add_systems(
            Core3d,
            ui_pass
                .in_set(UiPassSet)
                .in_set(Core3dSystems::PostProcess)
                // (After the colour correction, which runs before FXAA and SMAA, `color_correction.rs`.)
                .after(bevy::anti_alias::fxaa::fxaa)
                .after(bevy::anti_alias::smaa::smaa),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vertex layout the pipelines declare is `UiVertex` as the buffer holds it, and the records and values are the
    /// shader's structs' sizes.
    #[test]
    fn layouts_match_the_buffers() {
        let formats = [VertexFormat::Float32x3, VertexFormat::Float32x3, VertexFormat::Float32x4, VertexFormat::Float32x2];
        let layout = VertexBufferLayout::from_vertex_formats(VertexStepMode::Vertex, formats);
        assert_eq!(layout.array_stride, std::mem::size_of::<UiVertex>() as u64);
        assert_eq!(layout.attributes.iter().map(|a| a.offset).collect::<Vec<_>>(), vec![0, 12, 24, 40]);
        assert_eq!(std::mem::size_of::<PieceRecord>(), 80);
        assert_eq!(std::mem::size_of::<UiGlobalsGpu>(), 96);
    }
}
