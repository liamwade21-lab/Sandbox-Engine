//! The exposure the frame is drawn with, for the ship's shaders (ARCHITECTURE.md A6's `ShipScene`). The frame is multiplied by
//! 2^(the camera's `ColorGrading` exposure), which the CPU knows (`lights.rs` works `ShipScene`'s inputs out from it), and in
//! photo mode 2 by 2^-(the automatic exposure's adapted compensation) as well (`color_correction.wesl`), which lives only in a
//! GPU buffer (`crate::exposure::AutoExposureBuffers::state`, written by the `adapt` pass after the bloom). A small compute
//! pass copies that state into the ship's own buffer (`ShipScene::adapt`) before the opaque pass, so every ship material reads
//! it with no readback; the ship's shaders add it to the scene value only while it applies (`ShipScene`'s `auto`).
//!
//! The copy is the previous frame's state: the frame's own is metered from the finished picture, after the ship is drawn
//! (the colour correction reads it the same frame). While the exposure settles the ship is one adaptation step behind: at
//! 60 frames a second at most 1 - e^(-5/60) = 8 % of the way left to go (`AutoExpSpeedUp` 5/s), none once it has settled.
//!
//! Nothing runs before the first spawn: the pipeline is queued only once `ShipScene` exists, and the pass runs only while a
//! ship exists (`Sidewinder::parked`, extracted each frame as `ShipPresent`), with one bind group kept until either buffer
//! behind it is made anew.

use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{storage_buffer_read_only_sized, storage_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::storage::GpuShaderBuffer;
use bevy::render::view::ExtractedView;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};

use crate::exposure::AutoExposureBuffers;
use crate::viltrumite::draw::MainView;

use super::material::ShipScene;
use super::Sidewinder;

/// The copy's layout and pipeline, made at the first frame with a ship.
#[derive(Resource)]
struct AdaptCopy {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn queue_copy(mut commands: Commands, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let size = std::num::NonZero::new(16);
    let layout = BindGroupLayoutDescriptor::new("sidewinder_adapt", &BindGroupLayoutEntries::sequential(ShaderStages::COMPUTE, (storage_buffer_read_only_sized(false, size), storage_buffer_sized(false, size))));
    let pipeline = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sidewinder_adapt".into()),
        layout: vec![layout.clone()],
        shader: assets.load("shaders/sidewinder_exposure.wesl"),
        entry_point: Some("copy".into()),
        ..default()
    });
    commands.insert_resource(AdaptCopy { layout, pipeline });
}

/// Whether a ship exists this frame (the render world's copy of `Sidewinder::parked.is_some()`).
#[derive(Resource, Default)]
struct ShipPresent(bool);

fn extract_present(mut commands: Commands, ship: Extract<Option<Res<Sidewinder>>>, mut was: Local<bool>) {
    let present = ship.as_ref().is_some_and(|s| s.parked.is_some());
    if present != *was {
        info!("sidewinder: the exposure copy {}", if present { "runs (a ship exists)" } else { "stops (no ship)" });
        *was = present;
    }
    commands.insert_resource(ShipPresent(present));
}

/// The state into `ShipScene::adapt`, once per frame (the main view only), before the opaque pass, while a ship exists. The
/// bind group is kept with the two buffers' ids it was made for.
#[allow(clippy::too_many_arguments)]
fn copy(
    _view: ViewQuery<&ExtractedView, MainView>,
    present: Option<Res<ShipPresent>>,
    scene: Option<Res<ShipScene>>,
    copy: Option<Res<AdaptCopy>>,
    auto: Option<Res<AutoExposureBuffers>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    cache: Res<PipelineCache>,
    mut group: Local<Option<(BufferId, BufferId, BindGroup)>>,
    mut ctx: RenderContext,
) {
    let (Some(true), Some(scene), Some(copy), Some(auto)) = (present.map(|p| p.0), scene, copy, auto) else { return };
    let (Some(pipeline), Some(target)) = (cache.get_compute_pipeline(copy.pipeline), buffers.get(&scene.adapt)) else { return };
    let ids = (auto.state.id(), target.buffer.id());
    if group.as_ref().is_none_or(|(a, b, _)| (*a, *b) != ids) {
        let layout = cache.get_bind_group_layout(&copy.layout);
        let made = ctx.render_device().create_bind_group("sidewinder_adapt", &layout, &BindGroupEntries::sequential((auto.state.as_entire_buffer_binding(), target.buffer.as_entire_buffer_binding())));
        *group = Some((ids.0, ids.1, made));
    }
    let Some((_, _, group)) = group.as_ref() else { return };
    let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("sidewinder_adapt"), ..default() });
    pass.set_bind_group(0, group, &[]);
    pass.set_pipeline(pipeline);
    pass.dispatch_workgroups(1, 1, 1);
}

pub fn build(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<ShipScene>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        // (Nothing before the first spawn: both wait for the extracted `ShipScene` and the queued pipeline.)
        .add_systems(ExtractSchedule, extract_present.run_if(resource_exists::<ShipScene>))
        .add_systems(Render, queue_copy.in_set(RenderSystems::PrepareResources).run_if(resource_exists::<ShipScene>).run_if(not(resource_exists::<AdaptCopy>)))
        .add_systems(Core3d, copy.in_set(Core3dSystems::Prepass).before(main_opaque_pass_3d).run_if(resource_exists::<AdaptCopy>));
}
