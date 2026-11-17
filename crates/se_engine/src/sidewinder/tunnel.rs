//! Hyperspace's tunnel in the scene (ARCHITECTURE.md A6, T4e-3): while a jump lasts (`Pilot::jump`) the pilot sits in the
//! cockpit and sees the tunnel through the canopy. It is a sphere round the camera (`RADIUS_M`; the camera is the render
//! frame's origin, so it stays there), turned with the ship, drawn by `TunnelMaterial` (`sidewinder_tunnel.wesl`) in the main
//! pass, opaque, writing a depth just behind the seat's slice (`material::SEAT_SLICE`): the cockpit, its chair and the glass
//! (drawn after it, in the transparent pass) and the HUD (egui) stay in front, and nothing of the universe shows through. Its
//! look is the stand-in the HUD drew flat until then (ours, F: Elite's tunnel isn't built), by the same rule; the shader says
//! how, and how its colour on the screen is kept whatever the exposure.
//!
//! - Nothing before the first spawn: the shell is made once the ship's materials are (`ShipScene` exists), and kept.
//! - Its pipeline is built before any jump can show it: from the shell's making it is drawn with its warm-up flag (every
//!   vertex out of the depth range: no pixel) until the render world sees every pipeline of its shader built (`Built`,
//!   `watch`), and again through each drive charge before a jump (a settings change since, such as the multisampling, asks
//!   for a new one: the charge's seconds build it). Otherwise it is hidden, at no cost.
//! - Its per-frame values (the jump's seconds, the layout's reach and the seat view's field of view, the shell's depth, the
//!   camera's colour correction and bloom) are one small buffer written in place when they change (`TunnelGpu`); the exposure comes
//!   from the ship's own records (`ShipScene`'s, bound again here). The material is never changed after it is made.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::Vec4;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, CachedPipelineState, PipelineCache, PipelineDescriptor, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::ShaderRef;
use bevy::window::PrimaryWindow;
use elite_sim::drive::Drive;

use crate::bloom::SeBloom;
use crate::camera::MainCamera;
use crate::color_correction::ColorCorrection;

use super::material::{self, ShipScene};
use super::pilot::Pilot;
use super::seat::{self, Seat};

const SHADER: &str = "shaders/sidewinder_tunnel.wesl";

/// The shell's radius (m): well beyond the ship (the cockpit's furthest point is 12 m from the eye) and well inside anything
/// else in the tunnel (the arrival is 5 of the star's radii out). Its depth is the shader's own (`TunnelGpu::look`), so the
/// port's near plane (up to 1 km far from any body) never cuts it.
const RADIUS_M: f32 = 2000.0;

/// The shell's depth (reverse z): just behind the seat's slice, which holds the nearest `SEAT_SLICE` of the range.
const DEPTH: f32 = (1.0 - material::SEAT_SLICE) * 0.999;

/// `sidewinder_tunnel.wesl` `TunnelGpu`.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
struct TunnelGpu {
    /// x 1 while a jump shows it (0: the warm-up), y the jump's seconds, z half the screen's diagonal in heights, w 2 tan(half
    /// the seat view's configured vertical field of view).
    clock: Vec4,
    /// x the shell's depth, y the tone shader (`color_correction::TONE_SHADERS` index), z the bloom's gain (`SeBloom`'s
    /// BloomBright, 0 with the bloom off).
    look: Vec4,
    /// The camera's colour correction (`ColorCorrection::bcsg`).
    bcsg: Vec4,
}

/// The shell's material: its values (`TunnelGpu`), and the ship's exposure records (`ShipScene`'s buffer, whose head says
/// whether the adapted compensation applies, and its copy of that compensation). Made once; never changed.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct TunnelMaterial {
    #[storage(0, read_only)]
    state: Handle<ShaderBuffer>,
    #[storage(1, read_only)]
    scene: Handle<ShaderBuffer>,
    #[storage(2, read_only)]
    adapt: Handle<ShaderBuffer>,
}

impl Material for TunnelMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn specialize(_pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, _key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?];
        // (Seen from inside.)
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Shared by both worlds: whether the render world is to look at the pipeline cache (while the shell warms up), and whether
/// every pipeline of the shell's shader is built.
#[derive(Resource, Clone, Default)]
struct Built(Arc<(AtomicBool, AtomicBool)>);

/// The shell's entity.
#[derive(Component)]
struct TunnelShell;

/// The shell, once made.
#[derive(Resource)]
struct Shell {
    state: Handle<ShaderBuffer>,
    last: TunnelGpu,
    /// Its pipeline was seen built (the warm-up is over).
    warmed: bool,
    shown: bool,
}

/// The shell, once the ship's materials exist (`ShipScene`: the first spawn's files are read): drawn for its warm-up.
fn make(mut commands: Commands, scene: Res<ShipScene>, built: Res<Built>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<TunnelMaterial>>, mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let state = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&TunnelGpu::default()), RenderAssetUsages::default()));
    let material = materials.add(TunnelMaterial { state: state.clone(), scene: scene.buffer.clone(), adapt: scene.adapt.clone() });
    let mesh = meshes.add(Sphere::new(RADIUS_M).mesh().uv(48, 24));
    commands.spawn((Name::new("Sidewinder hyperspace tunnel"), TunnelShell, Mesh3d(mesh), MeshMaterial3d(material), Transform::default(), Visibility::Visible, NoFrustumCulling));
    built.0.0.store(true, Ordering::Relaxed);
    commands.insert_resource(Shell { state, last: TunnelGpu::default(), warmed: false, shown: false });
    info!("sidewinder: the hyperspace tunnel's shell is made (drawn without pixels until its pipeline is built)");
}

/// Each frame once made: shown while a jump lasts and the camera is at the eye, turned with the ship; drawn without pixels
/// while it warms up or the drive charges a jump; else hidden. Its values written only when they change.
#[allow(clippy::too_many_arguments)]
fn update(
    mut shell: ResMut<Shell>,
    built: Res<Built>,
    seat: Res<Seat>,
    pilot: Res<Pilot>,
    window: Query<&Window, With<PrimaryWindow>>,
    grading: Query<(&ColorCorrection, Option<&SeBloom>), With<MainCamera>>,
    mut placed: Query<(&mut Transform, &mut Visibility), With<TunnelShell>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    frame: Res<bevy::diagnostic::FrameCount>,
) {
    if !shell.warmed && built.0.1.load(Ordering::Relaxed) {
        shell.warmed = true;
        built.0.0.store(false, Ordering::Relaxed);
        info!("sidewinder: the hyperspace tunnel's pipeline is built (frame {})", frame.0);
    }
    let jump = pilot.jump.filter(|_| seat.view);
    let charging = seat.seated && matches!(pilot.drive, Drive::JumpCharging(_) | Drive::Departing(_));
    let gpu = match jump {
        Some(j) => {
            let aspect = window.single().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
            let (tone, bcsg, bloom) = grading.iter().next().map_or((1, ColorCorrection::default().bcsg, 0.0), |(g, b)| (g.mode.x, g.bcsg, b.map_or(0.0, |b| b.params.x)));
            TunnelGpu {
                clock: Vec4::new(1.0, j.seconds as f32, 0.5 * aspect.hypot(1.0), 2.0 * (0.5 * seat::seat_vertical_fov(aspect)).tan()),
                look: Vec4::new(DEPTH, tone as f32, bloom, 0.0),
                bcsg,
            }
        }
        None => TunnelGpu { clock: Vec4::ZERO, ..shell.last },
    };
    if gpu != shell.last {
        material::write_in_place(&mut buffers, &shell.state, &gpu);
        shell.last = gpu;
    }
    if jump.is_some() != shell.shown {
        shell.shown = jump.is_some();
        info!("sidewinder: the hyperspace tunnel {} at frame {}{}", if shell.shown { "shows" } else { "is gone" }, frame.0, if shell.shown && !shell.warmed { " (its pipeline NOT yet built)" } else { "" });
    }
    let Ok((mut tf, mut vis)) = placed.single_mut() else { return };
    let want = if jump.is_some() || charging || !shell.warmed { Visibility::Visible } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if let Some(j) = jump {
        let rot = j.rot.as_quat();
        if tf.rotation != rot {
            tf.rotation = rot;
        }
    }
}

/// The render world, while the shell warms up: whether every pipeline of its shader is built.
fn watch(cache: Res<PipelineCache>, built: Res<Built>, mut failed: Local<bool>) {
    let mut any = false;
    let mut all = true;
    for p in cache.pipelines() {
        let PipelineDescriptor::RenderPipelineDescriptor(d) = &p.descriptor else { continue };
        if !d.vertex.shader.path().is_some_and(|path| path.path().ends_with("sidewinder_tunnel.wesl")) {
            continue;
        }
        any = true;
        match &p.state {
            CachedPipelineState::Ok(_) => {}
            CachedPipelineState::Err(e) => {
                if !std::mem::replace(&mut *failed, true) {
                    error!("sidewinder: the hyperspace tunnel's pipeline failed: {e}");
                }
                all = false;
            }
            _ => all = false,
        }
    }
    if any && all {
        built.0.1.store(true, Ordering::Relaxed);
    }
}

pub fn build(app: &mut App) {
    let built = Built::default();
    app.add_plugins(MaterialPlugin::<TunnelMaterial>::default())
        .insert_resource(built.clone())
        .add_systems(Update, make.run_if(resource_exists::<ShipScene>).run_if(not(resource_exists::<Shell>)))
        // (After the seat has written the camera and the jump's state for this frame, before the transforms are made.)
        .add_systems(
            PostUpdate,
            update
                .run_if(resource_exists::<Shell>)
                .after(seat::seat_camera)
                .before(crate::universe::UniverseSet::FloatingOrigin)
                .before(bevy::camera::CameraUpdateSystems),
        );
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app.insert_resource(built).add_systems(Render, watch.in_set(RenderSystems::Cleanup).run_if(|b: Res<Built>| b.0.0.load(Ordering::Relaxed)));
}
