//! The sandbox's props as the player sees them: spawned where he looks as Garry's Mod places them (`commands.lua`
//! `DoPlayerEntitySpawn`), drawn where the physics has them, undone with Z.

use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::system::SystemBody;
use crate::universe::ScaledSpace;

use super::assets::{Gpu, GmodAssets};
use super::audio::Cue;
use super::light::{PropLight, PropMaterial};
use super::movement::UNIT_M;
use super::{Request, Sandbox};

/// Garry's Mod's spawn trace: 2048 units from the eyes (`GetSpawnTrace`).
const SPAWN_REACH: f32 = 2048.0 * UNIT_M;
/// Where a prop goes when he looks at nothing within the ground field: this far ahead, to fall from there.
const SPAWN_AHEAD: f32 = 3.5;
/// Sandbox's `sbox_maxprops` is 200.
const MAX_PROPS: usize = 200;
/// Beyond this a prop is a speck or less: not drawn.
const DRAW_M: f64 = 4000.0;

/// One mesh of a prop in the scene.
#[derive(Component)]
pub struct PropPart;

#[derive(Resource, Default)]
pub struct Props {
    entities: HashMap<u32, Vec<Entity>>,
    /// The props in the order they were spawned: Z takes the last.
    undo: Vec<u32>,
}

/// What the menu, the keys and the test commands asked for: a prop where he looks, the last one undone, all cleared.
#[allow(clippy::too_many_arguments)]
pub fn handle_requests(
    mut commands: Commands,
    mut sb: ResMut<Sandbox>,
    mut props: ResMut<Props>,
    mut assets: ResMut<GmodAssets>,
    mut msg: ResMut<crate::messages::Messages>,
    light: Option<Res<PropLight>>,
    (mut meshes, mut images, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<Image>>, ResMut<Assets<PropMaterial>>),
) {
    let Some(light) = light else { return };
    let sb = &mut *sb;
    for request in std::mem::take(&mut sb.requests) {
        match request {
            Request::Spawn(path) => {
                let Some(world) = sb.world.as_mut() else { continue };
                if world.prop_count() >= MAX_PROPS {
                    msg.show("You've hit the props limit!");
                    sb.cues.push(Cue::ui("buttons/button10.wav"));
                    continue;
                }
                let mut gpu = Gpu { meshes: &mut meshes, images: &mut images, materials: &mut materials, light: &light };
                let Some(asset) = assets.prop(&mut gpu, &path) else {
                    msg.show(format!("{} can't be spawned", super::assets::prop_name(&path)));
                    sb.cues.push(Cue::ui("buttons/button10.wav"));
                    continue;
                };
                // Where he looks: on what the trace meets, else a little ahead in the air.
                let hit = world.ray(sb.eye, sb.forward, SPAWN_REACH, None);
                let (at, normal) = hit.map_or((sb.eye + sb.forward * SPAWN_AHEAD, Vec3::Z), |h| (h.point, h.normal));
                // Facing him (`eyeYaw + 180`), level, its box's lowest corner along the surface's normal on the spot.
                let rot = Quat::from_rotation_z(sb.yaw + std::f32::consts::PI);
                let lowest = (0..8)
                    .map(|i| rot * Vec3::new(if i & 1 == 0 { asset.min.x } else { asset.max.x }, if i & 2 == 0 { asset.min.y } else { asset.max.y }, if i & 4 == 0 { asset.min.z } else { asset.max.z }))
                    .map(|c| c.dot(normal))
                    .fold(f32::MAX, f32::min);
                let pos = at - normal * (lowest - 0.01);
                let Some(id) = world.spawn(&asset.shape, pos, rot, asset.mass, asset.surface) else {
                    msg.show(format!("{} has no shape to be solid with", super::assets::prop_name(&path)));
                    continue;
                };
                world.set_material(id, &asset.material);
                let entities = asset
                    .parts
                    .iter()
                    .map(|(mesh, material)| commands.spawn((Name::new("Sandbox prop"), PropPart, Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), Transform::default(), Visibility::Hidden, NoFrustumCulling)).id())
                    .collect();
                props.entities.insert(id, entities);
                props.undo.push(id);
                sb.cues.push(Cue::ui("ui/buttonclickrelease.wav"));
                info!("sandbox: spawned {path} as prop {id} at {:.2} {:.2} {:.2} m ({:.0} kg, {:?})", pos.x, pos.y, pos.z, asset.mass, asset.surface);
            }
            Request::Undo => {
                let Some(world) = sb.world.as_mut() else { continue };
                while let Some(id) = props.undo.pop() {
                    if world.prop(id).is_none() {
                        continue;
                    }
                    if sb.held == Some(id) {
                        sb.held = None;
                    }
                    world.remove(id);
                    msg.show("Undone Prop");
                    sb.cues.push(Cue::ui("buttons/button15.wav"));
                    break;
                }
            }
            Request::Clear => {
                if let Some(world) = sb.world.as_mut() {
                    world.remove_all();
                }
                sb.held = None;
                props.undo.clear();
                msg.show("Cleaned up everything!");
                sb.cues.push(Cue::ui("buttons/button15.wav"));
            }
        }
    }
}

/// Each prop's meshes where the physics has it this frame, between its last two ticks; placed from the camera as the floating
/// origin places what it moves. Meshes of props that are gone are removed.
pub fn place_props(
    mut commands: Commands,
    sb: Res<Sandbox>,
    cam: Res<FlyCamera>,
    space: Res<ScaledSpace>,
    mut props: ResMut<Props>,
    bodies: Query<&SystemBody>,
    mut parts: Query<(&mut Transform, &mut Visibility), With<PropPart>>,
) {
    let world = sb.world.as_ref();
    props.entities.retain(|id, entities| {
        let keep = world.is_some_and(|w| w.prop(*id).is_some());
        if !keep {
            for e in entities.iter() {
                commands.entity(*e).despawn();
            }
        }
        keep
    });
    let Some(world) = world else { return };
    let Some(b) = bodies.iter().find(|b| b.obj == world.frame.obj) else { return };
    let cam_body = b.orient.inverse() * cam.pos.delta_m(b.pos);
    let k = space.factor(b.pos.delta_m(cam.pos).length());
    let to_world = b.orient * world.frame.rotation();
    let t = sb.tick_fraction();
    for prop in world.props() {
        let Some(entities) = props.entities.get(&prop.id) else { continue };
        let pos = prop.prev.pos.lerp(prop.now.pos, t);
        let rot = prop.prev.rot.slerp(prop.now.rot, t);
        let rel = world.frame.to_body(pos.as_dvec3()) - cam_body;
        let show = rel.length() < DRAW_M;
        let tf = Transform { translation: (b.orient * rel * k).as_vec3(), rotation: (to_world * rot.as_dquat()).normalize().as_quat(), scale: Vec3::splat(k as f32) };
        for e in entities {
            let Ok((mut transform, mut vis)) = parts.get_mut(*e) else { continue };
            *transform = tf;
            let want = if show { Visibility::Visible } else { Visibility::Hidden };
            if *vis != want {
                *vis = want;
            }
        }
    }
}

/// A point of the local frame (metres) in the render frame, as `place_props` places things: for the weapon and the effects.
pub fn render_point(b: &SystemBody, frame: &super::world::LocalFrame, cam: &FlyCamera, p: Vec3) -> Vec3 {
    let cam_body = b.orient.inverse() * cam.pos.delta_m(b.pos);
    (b.orient * (frame.to_body(p.as_dvec3()) - cam_body)).as_vec3()
}
