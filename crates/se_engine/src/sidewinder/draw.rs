//! Drawing the Sidewinder (ARCHITECTURE.md A6): one entity per drawn draw of each rig node's model (`ShipRig::nodes`), its
//! mesh shared by every node of that model and its `ShipMaterial` shared by every draw of that Elite material; placed each
//! frame from the camera as the props are (`place_props`), each node at its matrix now (`Parts`) under the ship's pose.
//! Outside (parked, or flown seen from outside) the hull, the gear and the chair seen from outside show; seated, the cockpit
//! and its chair as well. Made once, the first time the files are read; never changed after.

use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::Vec4;
use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::sandbox::light::{PropLight, PropMaterial};
use crate::system::SystemBody;
use crate::universe::ScaledSpace;

use super::assets::ShipBuild;
use super::material::{self, ShipMaterial};
use super::parts::Parts;
use super::{Sidewinder, SidewinderAssets};

/// Beyond this the ship is a speck: not drawn (as the props).
const DRAW_M: f64 = 4000.0;

/// One mesh of the ship: the rig node it hangs from, and whether it shows only seated.
#[derive(Component)]
pub struct ShipPiece {
    node: usize,
    inside: bool,
}

/// A laser's beam (gun 0 or 1): a thin glowing box, stretched to the beam's length each frame while it shows.
#[derive(Component)]
pub struct Beam(usize);

/// The ship's meshes, once made.
#[derive(Resource)]
pub struct ShipDraw {
    pieces: Vec<Entity>,
    /// What shows: (outside, inside).
    shown: (bool, bool),
}

pub fn build(app: &mut App) {
    material::build(app);
    app.init_resource::<Parts>().add_systems(Update, make.run_if(resource_exists::<ShipBuild>)).add_systems(
        PostUpdate,
        (super::parts::update, place)
            .chain()
            .run_if(resource_exists::<SidewinderAssets>)
            .after(crate::universe::UniverseSet::Camera)
            .after(super::keep_solid)
            .after(crate::terrain::update_node_transforms)
            .before(crate::universe::UniverseSet::FloatingOrigin)
            .before(bevy::camera::CameraUpdateSystems),
    );
}

/// The meshes and materials, from what the first spawn read (`ShipBuild`, dropped once used).
#[allow(clippy::too_many_arguments)]
fn make(
    mut commands: Commands,
    assets: Option<Res<SidewinderAssets>>,
    light: Option<Res<PropLight>>,
    build: Option<ResMut<ShipBuild>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut ship_materials: ResMut<Assets<ShipMaterial>>,
    mut prop_materials: ResMut<Assets<PropMaterial>>,
) {
    let (Some(assets), Some(light), Some(mut build)) = (assets, light, build) else { return };
    let rig = &assets.rig;
    let white = material::white(&mut images);
    let textures: HashMap<(String, bool), Handle<Image>> = std::mem::take(&mut build.images).into_iter().map(|(k, img)| (k, images.add(img))).collect();
    let model_meshes: Vec<Vec<Handle<Mesh>>> = std::mem::take(&mut build.meshes).into_iter().map(|draws| draws.into_iter().map(|m| meshes.add(m)).collect()).collect();
    commands.remove_resource::<ShipBuild>();
    let mut mats: HashMap<String, Handle<ShipMaterial>> = HashMap::new();
    let mut pieces = Vec::new();
    for (i, node) in rig.nodes.iter().enumerate() {
        let Some(model) = node.model else { continue };
        for (key, mesh) in rig.models[model].draws.iter().zip(&model_meshes[model]) {
            let mat = mats
                .entry(key.clone())
                .or_insert_with(|| {
                    let base = rig.materials.get(key).and_then(|m| material::base_colour(m, &rig.ship.paint));
                    let tex = match &base {
                        Some(material::Base::Texture { file, srgb }) => textures.get(&(file.clone(), *srgb)).cloned(),
                        _ => None,
                    };
                    ship_materials.add(ShipMaterial::new(&light, base.as_ref(), tex, &white))
                })
                .clone();
            pieces.push(
                commands
                    .spawn((
                        Name::new(format!("Sidewinder {}", node.name)),
                        ShipPiece { node: i, inside: node.group.inside() },
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(mat),
                        Transform::default(),
                        Visibility::Hidden,
                        NoFrustumCulling,
                    ))
                    .id(),
            );
        }
    }
    info!("sidewinder: the ship's {} meshes are made ({} models, {} materials, {} textures)", pieces.len(), model_meshes.len(), mats.len(), textures.len());
    // The lasers' beams: one box each, from its gun along -Z, a metre long before it is stretched; a light of its own so it
    // shows by day and night.
    let beam_mat = prop_materials.add(PropMaterial {
        frame: light.frame.clone(),
        light: light.light.clone(),
        diffuse: white.clone(),
        tint: Vec4::new(1.0, 0.45, 0.15, 0.0),
        glow: Vec4::new(6.0, 1.0, 1.0, 0.0),
    });
    let beam_mesh = meshes.add(Cuboid::from_corners(Vec3::new(-0.04, -0.04, 0.0), Vec3::new(0.04, 0.04, -1.0)));
    for i in 0..2 {
        commands.spawn((Name::new("Sidewinder laser beam"), Beam(i), Mesh3d(beam_mesh.clone()), MeshMaterial3d(beam_mat.clone()), Transform::default(), Visibility::Hidden, NoFrustumCulling));
    }
    commands.insert_resource(ShipDraw { pieces, shown: (false, false) });
}

/// The parked ship where it stands this frame, from the camera, as `place_props` places the props.
#[allow(clippy::too_many_arguments)]
fn place(
    ship: Res<Sidewinder>,
    seat: Res<super::seat::Seat>,
    pilot: Res<super::pilot::Pilot>,
    assets: Res<SidewinderAssets>,
    parts: Res<Parts>,
    draw: Option<ResMut<ShipDraw>>,
    cam: Res<FlyCamera>,
    space: Res<ScaledSpace>,
    bodies: Query<&SystemBody>,
    mut pieces: Query<(&ShipPiece, &mut Transform, &mut Visibility), Without<Beam>>,
    mut beams: Query<(&Beam, &mut Transform, &mut Visibility), Without<ShipPiece>>,
) {
    let Some(mut draw) = draw else { return };
    let placed = ship.parked.as_ref().and_then(|p| Some((p, bodies.iter().find(|b| b.obj == p.obj)?)));
    let near = match placed {
        Some((p, b)) => {
            let cam_body = b.orient.inverse() * cam.pos.delta_m(b.pos);
            let rel = p.pos - cam_body;
            if rel.length() < DRAW_M {
                let k = space.factor(b.pos.delta_m(cam.pos).length());
                let ship_tf = Transform { translation: (b.orient * rel * k).as_vec3(), rotation: (b.orient * p.rot).normalize().as_quat(), scale: Vec3::splat(k as f32) };
                for (piece, mut tf, _) in pieces.iter_mut() {
                    *tf = ship_tf * Transform::from_matrix(parts.matrix(piece.node));
                }
                for (beam, mut tf, mut v) in beams.iter_mut() {
                    let on = seat.seated && pilot.weapons.beam(beam.0) && pilot.beam_len[beam.0] > 0.5;
                    let want = if on { Visibility::Visible } else { Visibility::Hidden };
                    if *v != want {
                        *v = want;
                    }
                    if on {
                        let gun = Vec3::from_array(assets.guns[beam.0]);
                        *tf = ship_tf * Transform::from_translation(gun).with_scale(Vec3::new(1.0, 1.0, pilot.beam_len[beam.0]));
                    }
                }
                true
            } else {
                false
            }
        }
        None => false,
    };
    if !near || !seat.seated || pilot.cruise.is_some() {
        for (_, _, mut v) in beams.iter_mut() {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
        }
    }
    let show = (near, near && seat.seated);
    if draw.shown != show {
        draw.shown = show;
        for e in draw.pieces.clone() {
            if let Ok((piece, _, mut v)) = pieces.get_mut(e) {
                let on = if piece.inside { show.1 } else { show.0 };
                *v = if on { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }
}
