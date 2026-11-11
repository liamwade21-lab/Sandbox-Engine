//! How the sandbox's props and the weapon are lit: as the ground is. The port has no Bevy lights; the ground's light at a point
//! (the star through the air, the sky, cloud shadows, night, haze) comes from `ground_light.wesl` `point_light_at`, which the
//! Viltrumite mod's hero already runs once a frame for his own place (`hero_light.wesl`, `viltrumite/draw.rs`). The sandbox
//! runs the same pass a second time, for the player's place, with records of its own (`LightProbe`, `LightPass`), so it works
//! with the mod off, and every prop's material reads the result (`sandbox_prop.wesl`).

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::math::DVec3;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::ShaderRef;

use crate::camera::FlyCamera;
use crate::universe::UniverseSet;
use crate::viltrumite::draw::{LightParams, LightPass, LightProbe, LightTables, MainView};
use crate::volumetric_clouds::CloudShadowAssets;

use super::Sandbox;

/// A prop's or the weapon's material: a texture, a tint, and the two light records. Made once; never changed.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(PropKey)]
pub struct PropMaterial {
    #[storage(0, read_only)]
    pub frame: Handle<ShaderBuffer>,
    #[storage(1, read_only)]
    pub light: Handle<ShaderBuffer>,
    #[texture(2)]
    #[sampler(3)]
    pub diffuse: Handle<Image>,
    /// rgb over the texture's colour; a the alpha under which pixels are cut out (0: none).
    #[uniform(4)]
    pub tint: Vec4,
    /// x light of its own (a share of daylight on white), y 1 when seen from both sides, z the least that daylight counts as
    /// (so a flash shows at night).
    #[uniform(5)]
    pub glow: Vec4,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PropKey {
    two_sided: bool,
}

impl From<&PropMaterial> for PropKey {
    fn from(m: &PropMaterial) -> Self {
        PropKey { two_sided: m.glow.y > 0.5 }
    }
}

impl Material for PropMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sandbox_prop.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sandbox_prop.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.tint.w > 0.0 { AlphaMode::Mask(self.tint.w) } else { AlphaMode::Opaque }
    }

    fn specialize(_pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        if key.bind_group_data.two_sided {
            descriptor.primitive.cull_mode = None;
        }
        Ok(())
    }
}

/// The sandbox's light records, and what the pass needs of them in the render world.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct PropLight {
    pub on: bool,
    pub frame: Handle<ShaderBuffer>,
    pub light: Handle<ShaderBuffer>,
    tables: Option<LightTables>,
}

/// The record itself, kept so nothing is allocated a frame.
#[derive(Resource)]
struct PropLightRecord(LightProbe);

#[derive(Resource, Default)]
struct PropLightPass(LightPass);

pub fn build(app: &mut App) {
    app.add_plugins(MaterialPlugin::<PropMaterial>::default())
        .add_plugins(ExtractResourcePlugin::<PropLight>::default())
        .add_systems(Startup, make_records)
        // After the camera is where the frame draws from and the ground's own light record is this frame's.
        .add_systems(PostUpdate, update_light.after(UniverseSet::Camera).after(crate::terrain::update_node_transforms).before(UniverseSet::FloatingOrigin));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<PropLightPass>()
        .add_systems(Render, prepare_pass.in_set(RenderSystems::PrepareBindGroups).run_if(resource_exists::<PropLight>))
        .add_systems(Core3d, run_pass.in_set(Core3dSystems::Prepass).before(main_opaque_pass_3d));
}

fn make_records(mut commands: Commands, mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let probe = LightProbe::new();
    let frame = buffers.add(ShaderBuffer::new(probe.frame_bytes(), RenderAssetUsages::default()));
    let light = buffers.add(ShaderBuffer::new(LightProbe::light_bytes(), RenderAssetUsages::RENDER_WORLD));
    commands.insert_resource(PropLight { on: false, frame, light, tables: None });
    commands.insert_resource(PropLightRecord(probe));
}

/// The light at the camera's place while the sandbox has anything to show: his weapon, or props.
fn update_light(sb: Res<Sandbox>, cam: Res<FlyCamera>, mut light: ResMut<PropLight>, mut record: ResMut<PropLightRecord>, mut buffers: ResMut<Assets<ShaderBuffer>>, p: LightParams) {
    let on = sb.world.as_ref().is_some_and(|w| sb.walking() || w.prop_count() > 0);
    if light.on != on {
        light.on = on;
    }
    if !on {
        return;
    }
    let near = record.0.fill(&p, cam.pos, cam.pos, cam.rot * DVec3::Y);
    let tables = near.and_then(|t| t.atmosphere.as_ref()).map(|a| (a.inscatter.clone(), a.transmittance.clone(), a.irradiance.clone()));
    if light.tables != tables {
        light.tables = tables;
    }
    if let Some(mut b) = buffers.get_mut(&light.frame) {
        record.0.write(&mut b);
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_pass(
    input: Res<PropLight>,
    mut pass: ResMut<PropLightPass>,
    shadow: Option<Res<CloudShadowAssets>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
) {
    pass.0.prepare(input.on, &input.frame, &input.light, input.tables.as_ref(), shadow.as_deref(), &buffers, &images, &fallback, &cache, &device, &assets);
}

fn run_pass(_view: ViewQuery<&ExtractedView, MainView>, pass: Res<PropLightPass>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    pass.0.run(&cache, &mut ctx);
}
