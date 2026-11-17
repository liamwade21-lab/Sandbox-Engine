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
//!
//! The headlights on the ground (`beam.rs`) take the same exposure: a second pass (`fill_beam`, `sidewinder_beam.wesl`) writes
//! the state into the ground's record (`CloudShadowAssets::shadow`, its `beam.head.w`), with the IES profile's row as the
//! ship's sampler reads it (`beam.profile`). It runs each frame while a headlight is lit (`ShipBeam`, extracted as
//! `BeamFill`), before every pass of the frame that reads the record (`Core3dSystems::Prepass`: the scatter's per-instance
//! light, the light passes, the opaque pass). The CPU's rewrite of the record (zeros in those two parts) is a queue write in
//! the render world's prepare step, which wgpu carries out before the frame's command buffers, so the pass's values follow
//! it in the same frame and no reader sees them stale. Its pipeline is queued once the profile's texture is known
//! (`beam::BeamProfile`); a profile of another width than the table's is refused (logged), and the beam then gives no light.

use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::storage::GpuShaderBuffer;
use bevy::render::texture::GpuImage;
use bevy::render::view::ExtractedView;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};

use crate::exposure::AutoExposureBuffers;
use crate::viltrumite::draw::MainView;
use crate::volumetric_clouds::{CloudShadowAssets, ShipBeam, BEAM_PROFILE_TEXELS};

use super::beam::BeamProfile;
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

/// The headlights' pass: its layout and pipeline, made once the profile's texture is known.
#[derive(Resource)]
struct BeamPass {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn queue_beam(mut commands: Commands, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let filterable = TextureSampleType::Float { filterable: true };
    let layout = BindGroupLayoutDescriptor::new(
        "sidewinder_beam",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (storage_buffer_read_only_sized(false, std::num::NonZero::new(16)), storage_buffer_sized(false, None), texture_2d(filterable), sampler(SamplerBindingType::Filtering)),
        ),
    );
    let pipeline = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sidewinder_beam".into()),
        layout: vec![layout.clone()],
        shader: assets.load("shaders/sidewinder_beam.wesl"),
        entry_point: Some("fill".into()),
        ..default()
    });
    commands.insert_resource(BeamPass { layout, pipeline });
}

/// The render world's view of the headlights: lit this frame (`ShipBeam`'s head), and the profile's texture.
#[derive(Resource)]
struct BeamFill {
    lit: bool,
    ies: Handle<Image>,
}

fn extract_beam(mut commands: Commands, beam: Extract<Res<ShipBeam>>, profile: Extract<Option<Res<BeamProfile>>>) {
    if let Some(p) = profile.as_ref() {
        commands.insert_resource(BeamFill { lit: beam.head.x > 0.5, ies: p.ies.clone() });
    }
}

/// The state and the profile into the ground's record, once per frame (the main view only), before anything reads it, while a
/// headlight is lit. The bind group is kept with the ids it was made for.
#[allow(clippy::too_many_arguments)]
fn fill_beam(
    _view: ViewQuery<&ExtractedView, MainView>,
    fill: Res<BeamFill>,
    pass: Res<BeamPass>,
    auto: Option<Res<AutoExposureBuffers>>,
    shadow: Option<Res<CloudShadowAssets>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    cache: Res<PipelineCache>,
    mut group: Local<Option<([BufferId; 2], TextureViewId, SamplerId, BindGroup)>>,
    mut refused: Local<bool>,
    mut ctx: RenderContext,
) {
    let (true, Some(auto), Some(shadow)) = (fill.lit, auto, shadow) else { return };
    let (Some(pipeline), Some(record), Some(ies)) = (cache.get_compute_pipeline(pass.pipeline), buffers.get(&shadow.shadow), images.get(&fill.ies)) else { return };
    if ies.texture_descriptor.size.width as usize != BEAM_PROFILE_TEXELS {
        if !std::mem::replace(&mut *refused, true) {
            warn!("sidewinder: the headlights' IES profile is {} texels wide, not the ground's table's {BEAM_PROFILE_TEXELS}: no beam on the ground", ies.texture_descriptor.size.width);
        }
        return;
    }
    let ids = ([auto.state.id(), record.buffer.id()], ies.texture_view.id(), ies.sampler.id());
    if group.as_ref().is_none_or(|(b, v, s, _)| (*b, *v, *s) != ids) {
        let layout = cache.get_bind_group_layout(&pass.layout);
        let made = ctx.render_device().create_bind_group(
            "sidewinder_beam",
            &layout,
            &BindGroupEntries::sequential((auto.state.as_entire_buffer_binding(), record.buffer.as_entire_buffer_binding(), &ies.texture_view, &ies.sampler)),
        );
        *group = Some((ids.0, ids.1, ids.2, made));
    }
    let Some((_, _, _, group)) = group.as_ref() else { return };
    let mut cpass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("sidewinder_beam"), ..default() });
    cpass.set_bind_group(0, group, &[]);
    cpass.set_pipeline(pipeline);
    cpass.dispatch_workgroups((BEAM_PROFILE_TEXELS / 4).div_ceil(64) as u32, 1, 1);
}

pub fn build(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<ShipScene>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        // (Nothing before the first spawn: both wait for the extracted `ShipScene` and the queued pipeline.)
        .add_systems(ExtractSchedule, extract_present.run_if(resource_exists::<ShipScene>))
        .add_systems(Render, queue_copy.in_set(RenderSystems::PrepareResources).run_if(resource_exists::<ShipScene>).run_if(not(resource_exists::<AdaptCopy>)))
        .add_systems(Core3d, copy.in_set(Core3dSystems::Prepass).before(main_opaque_pass_3d).run_if(resource_exists::<AdaptCopy>))
        // (The headlights' pass: nothing until the ship's materials give the profile's texture, `beam::BeamProfile`.)
        .add_systems(ExtractSchedule, extract_beam.run_if(resource_exists::<ShipScene>))
        .add_systems(Render, queue_beam.in_set(RenderSystems::PrepareResources).run_if(resource_exists::<BeamFill>).run_if(not(resource_exists::<BeamPass>)))
        .add_systems(Core3d, fill_beam.before(Core3dSystems::Prepass).run_if(resource_exists::<BeamPass>).run_if(resource_exists::<BeamFill>));
}
