//! The sandbox's props as the player sees them: spawned where he looks as Garry's Mod places them (`commands.lua`
//! `DoPlayerEntitySpawn`), drawn where the physics has them, undone with Z (the parked Sidewinder too: an entry of its own,
//! removed through `ModControl::remove_ship`).

use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::mods::ModControl;
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

/// What Z can take back: a prop, or the parked Sidewinder (an entry of its own: its prop comes and goes as he boards and
/// gets out, its place in the list stays; ARCHITECTURE.md A11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spawned {
    Prop(u32),
    Ship,
}

#[derive(Resource, Default)]
pub struct Props {
    entities: HashMap<u32, Vec<Entity>>,
    /// What was spawned, in order: Z takes the last.
    undo: Vec<Spawned>,
}

impl Props {
    /// A Sidewinder was spawned (`sidewinder/mod.rs`): it goes on top of the list, and the ship it replaced leaves it.
    pub fn ship_spawned(&mut self) {
        self.undo.retain(|s| *s != Spawned::Ship);
        self.undo.push(Spawned::Ship);
    }

    /// Takes the newest entry that is still there off the list (`prop_here` says whether a prop is, `ship_here` whether the
    /// ship is parked); entries whose prop or ship has gone are dropped on the way. While someone sits in the ship it is kept:
    /// its entry stays and nothing is undone.
    fn undo_next(&mut self, prop_here: impl Fn(u32) -> bool, ship_here: bool, seated: bool) -> Option<Spawned> {
        while let Some(&last) = self.undo.last() {
            match last {
                Spawned::Prop(id) if !prop_here(id) => {}
                Spawned::Ship if !ship_here => {}
                Spawned::Ship if seated => return None,
                _ => return self.undo.pop(),
            }
            self.undo.pop();
        }
        None
    }

    /// "Clean up everything": the list emptied but for the ship's entry while someone sits in it. `true` if the ship is to go.
    fn clear(&mut self, ship_here: bool, seated: bool) -> bool {
        let keep_ship = ship_here && seated;
        self.undo.retain(|s| *s == Spawned::Ship && keep_ship);
        ship_here && !seated
    }
}

/// What the menu, the keys and the test commands asked for: a prop where he looks, the last one undone, all cleared.
#[allow(clippy::too_many_arguments)]
pub fn handle_requests(
    mut commands: Commands,
    mut sb: ResMut<Sandbox>,
    mut props: ResMut<Props>,
    mut assets: ResMut<GmodAssets>,
    mut msg: ResMut<crate::messages::Messages>,
    mut control: ResMut<ModControl>,
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
                props.undo.push(Spawned::Prop(id));
                sb.cues.push(Cue::ui("ui/buttonclickrelease.wav"));
                info!("sandbox: spawned {path} as prop {id} at {:.2} {:.2} {:.2} m ({:.0} kg, {:?})", pos.x, pos.y, pos.z, asset.mass, asset.surface);
            }
            Request::Undo => {
                let Some(world) = sb.world.as_mut() else { continue };
                match props.undo_next(|id| world.prop(id).is_some(), control.ship_parked, control.pilot) {
                    Some(Spawned::Prop(id)) => {
                        if sb.held == Some(id) {
                            sb.held = None;
                        }
                        world.remove(id);
                        msg.show("Undone Prop");
                        info!("sandbox: undone prop {id}");
                    }
                    // (The ship's module takes it out of the world, its prop and a held one with it.)
                    Some(Spawned::Ship) => {
                        control.remove_ship();
                        msg.show("Undone Sidewinder");
                        info!("sandbox: undone the Sidewinder");
                    }
                    None => continue,
                }
                sb.cues.push(Cue::ui("buttons/button15.wav"));
            }
            Request::Clear => {
                if let Some(world) = sb.world.as_mut() {
                    world.remove_all();
                }
                sb.held = None;
                let ship = props.clear(control.ship_parked, control.pilot);
                if ship {
                    control.remove_ship();
                }
                info!("sandbox: cleaned up everything{}", if ship { ", the Sidewinder too" } else { "" });
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
        let pose = prop.between(t);
        let rel = world.frame.to_body(pose.pos.as_dvec3()) - cam_body;
        let show = rel.length() < DRAW_M;
        let tf = Transform { translation: (b.orient * rel * k).as_vec3(), rotation: (to_world * pose.rot.as_dquat()).normalize().as_quat(), scale: Vec3::splat(k as f32) };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn list(entries: &[Spawned]) -> Props {
        Props { entities: HashMap::new(), undo: entries.to_vec() }
    }

    #[test]
    fn undo_takes_props_and_the_ship_in_the_order_they_were_spawned() {
        let mut p = list(&[Spawned::Prop(1)]);
        p.ship_spawned();
        p.undo.push(Spawned::Prop(2));
        let all = |_: u32| true;
        assert_eq!(p.undo_next(all, true, false), Some(Spawned::Prop(2)));
        assert_eq!(p.undo_next(all, true, false), Some(Spawned::Ship));
        // (The ship is gone now: `ship_here` false from here on.)
        assert_eq!(p.undo_next(all, false, false), Some(Spawned::Prop(1)));
        assert_eq!(p.undo_next(all, false, false), None);
    }

    #[test]
    fn a_gone_ships_entry_is_skipped_as_a_gone_props_is() {
        let mut p = list(&[Spawned::Prop(1), Spawned::Ship, Spawned::Prop(2)]);
        // Prop 2 undone some other way, the ship removed from its Mods row: Z takes prop 1, and the list is empty after.
        assert_eq!(p.undo_next(|id| id != 2, false, false), Some(Spawned::Prop(1)));
        assert!(p.undo.is_empty());
    }

    #[test]
    fn a_replaced_ship_leaves_the_list_and_the_new_one_is_on_top() {
        let mut p = list(&[]);
        p.ship_spawned();
        p.undo.push(Spawned::Prop(1));
        p.ship_spawned();
        assert_eq!(p.undo, vec![Spawned::Prop(1), Spawned::Ship]);
        assert_eq!(p.undo_next(|_| true, true, false), Some(Spawned::Ship));
        assert_eq!(p.undo_next(|_| true, false, false), Some(Spawned::Prop(1)));
    }

    #[test]
    fn seated_the_ship_is_not_undone() {
        let mut p = list(&[Spawned::Prop(1), Spawned::Ship]);
        assert_eq!(p.undo_next(|_| true, true, true), None);
        assert_eq!(p.undo, vec![Spawned::Prop(1), Spawned::Ship]);
    }

    #[test]
    fn clean_up_removes_the_ship_unless_someone_sits_in_it() {
        let mut p = list(&[Spawned::Prop(1), Spawned::Ship, Spawned::Prop(2)]);
        assert!(p.clear(true, false));
        assert!(p.undo.is_empty());
        // Seated: the ship stays, and its entry with it.
        let mut p = list(&[Spawned::Prop(1), Spawned::Ship, Spawned::Prop(2)]);
        assert!(!p.clear(true, true));
        assert_eq!(p.undo, vec![Spawned::Ship]);
        // No ship parked: nothing to remove, and a stale entry goes.
        let mut p = list(&[Spawned::Ship, Spawned::Prop(2)]);
        assert!(!p.clear(false, false));
        assert!(p.undo.is_empty());
    }
}
