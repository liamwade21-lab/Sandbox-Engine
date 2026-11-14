//! Drawing the parked Sidewinder: one mesh per draw of `ship.edm`, lit as the sandbox's props are (`PropMaterial`, the
//! sandbox's light records at the player's place), placed each frame from the camera as the props are (`place_props`), each
//! part at its rest transform under the ship's pose. Made once, the first time the files are read; never changed after.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Vec4;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use se_core::elite_asset::material_flags;

use crate::camera::FlyCamera;
use crate::sandbox::light::{PropLight, PropMaterial};
use crate::system::SystemBody;
use crate::universe::ScaledSpace;

use super::{rest_transforms, Sidewinder, SidewinderAssets};

/// Beyond this the ship is a speck: not drawn (as the props).
const DRAW_M: f64 = 4000.0;

/// One mesh of the ship: which model (the hull's or the cockpit's) and which part of it it hangs from.
#[derive(Component)]
pub struct ShipPiece {
    part: usize,
    cockpit: bool,
}

/// The ship's meshes, once made.
#[derive(Resource)]
pub struct ShipDraw {
    pieces: Vec<Entity>,
    /// What shows: (the hull, the cockpit). Seated, the camera is inside the hull: the cockpit shows instead.
    shown: (bool, bool),
}

pub fn build(app: &mut App) {
    app.add_systems(Update, make.run_if(resource_exists::<SidewinderAssets>)).add_systems(
        PostUpdate,
        place.run_if(resource_exists::<SidewinderAssets>).after(crate::universe::UniverseSet::Camera)
            .after(super::keep_solid)
            .after(crate::terrain::update_node_transforms)
            .before(crate::universe::UniverseSet::FloatingOrigin)
            .before(bevy::camera::CameraUpdateSystems),
    );
}

fn white(images: &mut Assets<Image>) -> Handle<Image> {
    let mut img = Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, vec![255; 4], TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    images.add(img)
}

#[allow(clippy::too_many_arguments)]
fn make(
    mut commands: Commands,
    assets: Option<Res<SidewinderAssets>>,
    light: Option<Res<PropLight>>,
    done: Option<Res<ShipDraw>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<PropMaterial>>,
) {
    let (Some(assets), Some(light), None) = (assets, light, done) else { return };
    let tex = white(&mut images);
    let mut pieces = Vec::new();
    for (m, cockpit) in [(&*assets.ship, false), (&*assets.cockpit, true)] {
        let mats: Vec<Handle<PropMaterial>> = m
            .materials
            .iter()
            .map(|mat| {
                materials.add(PropMaterial {
                    frame: light.frame.clone(),
                    light: light.light.clone(),
                    diffuse: tex.clone(),
                    tint: Vec4::new(mat.tint[0], mat.tint[1], mat.tint[2], 0.0),
                    glow: Vec4::new(0.0, if mat.flags & material_flags::TWO_SIDED != 0 { 1.0 } else { 0.0 }, 0.0, 0.0),
                })
            })
            .collect();
        for d in &m.draws {
            // (Glass waits for a see-through material, step 4: drawn with this opaque one it would hide the view.)
            if m.materials[d.material as usize].flags & material_flags::GLASS != 0 {
                continue;
            }
            let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, d.vertices.iter().map(|v| m.position_m(v)).collect::<Vec<_>>());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, d.vertices.iter().map(|v| v.normal()).collect::<Vec<_>>());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, d.vertices.iter().map(|v| v.uv()).collect::<Vec<_>>());
            mesh.insert_indices(Indices::U16(d.indices.clone()));
            pieces.push(
                commands
                    .spawn((
                        Name::new(if cockpit { "Sidewinder cockpit" } else { "Sidewinder" }),
                        ShipPiece { part: d.part as usize, cockpit },
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(mats[d.material as usize].clone()),
                        Transform::default(),
                        Visibility::Hidden,
                        NoFrustumCulling,
                    ))
                    .id(),
            );
        }
    }
    info!("sidewinder: the ship's {} meshes are made", pieces.len());
    commands.insert_resource(ShipDraw { pieces, shown: (false, false) });
}

/// The parked ship where it stands this frame, from the camera, as `place_props` places the props.
#[allow(clippy::too_many_arguments)]
fn place(
    ship: Res<Sidewinder>,
    seat: Res<super::seat::Seat>,
    pilot: Res<super::pilot::Pilot>,
    assets: Option<Res<SidewinderAssets>>,
    draw: Option<ResMut<ShipDraw>>,
    cam: Res<FlyCamera>,
    space: Res<ScaledSpace>,
    bodies: Query<&SystemBody>,
    mut pieces: Query<(&ShipPiece, &mut Transform, &mut Visibility)>,
) {
    let (Some(assets), Some(mut draw)) = (assets, draw) else { return };
    let placed = ship.parked.as_ref().and_then(|p| Some((p, bodies.iter().find(|b| b.obj == p.obj)?)));
    let near = match placed {
        Some((p, b)) => {
            let cam_body = b.orient.inverse() * cam.pos.delta_m(b.pos);
            let rel = p.pos - cam_body;
            if rel.length() < DRAW_M {
                let k = space.factor(b.pos.delta_m(cam.pos).length());
                let ship_tf = Transform { translation: (b.orient * rel * k).as_vec3(), rotation: (b.orient * p.rot).normalize().as_quat(), scale: Vec3::splat(k as f32) };
                // The gear as the seat's pilot has it (parked: down).
                let gear = if seat.seated { pilot.gear_t } else { 0.0 };
                let (hull, cockpit) = (super::posed_transforms(&assets.ship, "gear_up", gear), rest_transforms(&assets.cockpit));
                for (piece, mut tf, _) in pieces.iter_mut() {
                    *tf = ship_tf * Transform::from_matrix(if piece.cockpit { cockpit[piece.part] } else { hull[piece.part] });
                }
                true
            } else {
                false
            }
        }
        None => false,
    };
    let show = (near && !seat.seated, near && seat.seated);
    if draw.shown != show {
        draw.shown = show;
        for e in draw.pieces.clone() {
            if let Ok((piece, _, mut v)) = pieces.get_mut(e) {
                let on = if piece.cockpit { show.1 } else { show.0 };
                *v = if on { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }
}
