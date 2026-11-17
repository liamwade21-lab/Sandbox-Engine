//! Drawing the Sidewinder (ARCHITECTURE.md A6): one entity per drawn draw of each rig node's model (`ShipRig::nodes`), its
//! mesh shared by every node of that model and its `ShipMaterial` shared by every draw of that Elite material; placed each
//! frame from the camera as the props are (`place_props`), each node at its matrix now (`Parts`) under the ship's pose (in
//! hyperspace's tunnel, where no body is near, the jump's pose with the eye at the camera: `hyper::tunnel_place`).
//! What shows (ARCHITECTURE.md A6): the cockpit model carries its own copy of the hull's shell as seen from the seat, and
//! the outside chair is a low-detail twin of the cockpit's, so the two sets are never drawn together. From the seat (the
//! camera at the pilot's eye this frame, `Seat::view`) only the cockpit rig's models and its chair, in the seat's depth
//! slice (`material::SEAT_SLICE`); from anywhere else (parked, walking round it, the free camera) only the hull rig's, the
//! gear rigs' and the outside chair; a node the state machines hide (`Parts::shown`: the `KModelState` activities, the
//! windscreen once breached, its broken twin while whole) in neither. Made once, the first time the files are read; never
//! changed after. The seat's chair moves by its vertices' bones: its meshes carry them and its materials are skinned ones
//! (`ShipMaterial::skinned`), posed by the pilot chair's palette (`ShipPalette`, written in place from `chair::ChairPose`
//! when the pose changes: `skin`); the outside chair stays in its bind pose.

use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::Vec4;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

use crate::camera::FlyCamera;
use crate::sandbox::light::{PropLight, PropMaterial};
use crate::system::SystemBody;
use crate::universe::ScaledSpace;

use super::assets::ShipBuild;
use super::lights::{self, LightList};
use super::chair::ChairPose;
use super::hyper::tunnel_place;
use super::material::{self, Family, GlassEffects, ShipGlass, ShipGlow, ShipLights, ShipMaterial, ShipPalette, ShipScene, TexSource};
use super::parts::Parts;
use super::{Sidewinder, SidewinderAssets};

/// Beyond this the ship is a speck: not drawn (as the props).
const DRAW_M: f64 = 4000.0;

/// One mesh of the ship: the rig node it hangs from, and whether it is of the seat's set (`Group::inside`).
#[derive(Component)]
pub struct ShipPiece {
    node: usize,
    inside: bool,
}

/// Which set shows (`ShipDraw::shown`): (the outside set, the seat's set). Never both.
fn shown(near: bool, seat_view: bool) -> (bool, bool) {
    (near && !seat_view, near && seat_view)
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
    /// The parts' shown nodes as last applied (`Parts::shown_revision`: the `KModelState` activities).
    nodes_shown: u64,
    /// The ship's space to the render frame this frame (its scale the scaled space's), while it is near enough to draw.
    placed: Option<Transform>,
}

impl ShipDraw {
    /// The ship's space to the render frame this frame, while drawn by its body (`shadow.rs` works from it); None in
    /// hyperspace's tunnel, where no star lights it.
    pub fn placed(&self) -> Option<Transform> {
        self.placed
    }

    /// The seat's set shows (else the outside set, while drawn).
    pub fn seat_shown(&self) -> bool {
        self.shown.1
    }
}

pub fn build(app: &mut App) {
    material::build(app);
    lights::build(app);
    app.init_resource::<Parts>()
        .init_resource::<super::facts::ShipFacts>()
        .add_systems(Update, make.run_if(resource_exists::<ShipBuild>))
        .add_systems(PostUpdate, skin.run_if(resource_exists::<SidewinderAssets>).after(super::chair::update))
        .add_systems(
        PostUpdate,
        // (The facts first: from this frame's flight, before the machines, the lights and the sound read them.)
        (super::facts::update, super::parts::update, lights::update, place)
            .chain()
            .run_if(resource_exists::<SidewinderAssets>)
            .after(crate::universe::UniverseSet::Camera)
            .after(super::keep_solid)
            .after(crate::terrain::update_node_transforms)
            .before(crate::universe::UniverseSet::FloatingOrigin)
            .before(bevy::camera::CameraUpdateSystems),
    );
}

/// The meshes and materials, from what the first spawn read and resolved (`ShipBuild`, dropped once used), the ship's lights
/// (their list, `lights::LightList`, and buffer), its scene exposure inputs, the heat effect's glow and the canopy's glass
/// effects: each material lit by its set's range of the list (`LightList::range`), the hull's ElementGlow taking the heat
/// effect's values.
#[allow(clippy::too_many_arguments)]
fn make(
    mut commands: Commands,
    assets: Option<Res<SidewinderAssets>>,
    light: Option<Res<PropLight>>,
    build: Option<ResMut<ShipBuild>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut ship_materials: ResMut<Assets<ShipMaterial>>,
    mut prop_materials: ResMut<Assets<PropMaterial>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
) {
    let (Some(assets), Some(light), Some(mut build)) = (assets, light, build) else { return };
    let rig = &assets.rig;
    let white = material::white(&mut images);
    let read = std::mem::take(&mut build.images);
    let textures: HashMap<(String, bool), Handle<Image>> = read.into_iter().map(|(k, img)| (k, images.add(img))).collect();
    let ies = textures.get(&(material::IES_FILE.to_string(), true)).cloned().unwrap_or_else(|| white.clone());
    // (The shadow's own copy of the casters' triangles, from the meshes before they become assets.)
    let casters = super::shadow::Casters::new(rig, &build.meshes, &build.materials);
    let model_meshes: Vec<Vec<Handle<Mesh>>> = std::mem::take(&mut build.meshes).into_iter().map(|draws| draws.into_iter().map(|m| meshes.add(m)).collect()).collect();
    commands.remove_resource::<ShipBuild>();
    let list = match LightList::new(rig, &rig.static_pose(true)) {
        Ok(l) => l,
        Err(e) => {
            warn!("sidewinder: the ship's lights: {e}; drawn without them");
            LightList { lamps: Vec::new(), hull: 0 }
        }
    };
    let lights = ShipLights::new(&mut buffers);
    let scene = ShipScene::new(&mut buffers, &lights::scene_inputs(0.0, crate::exposure::AutoExposureSettings::default().target_bright));
    let glow = ShipGlow::new(&mut buffers);
    let glass = ShipGlass::new(&mut buffers);
    let palette = ShipPalette::new(&mut buffers);
    let shadow = super::shadow::ShipShadow::new(&mut images, &mut buffers, &light, &palette, casters, rig.nodes.len(), device.as_deref());
    // (The heat effect's element: the outside materials of that name; only the hull's model has one outside.)
    let glow_element = rig.ship.machines.iter().flat_map(|m| &m.states).find_map(lights::HeatEffect::of_state).map(|e| e.element.to_lowercase());
    let resolved = std::mem::take(&mut build.materials);
    let mut constants: HashMap<[u8; 4], Handle<Image>> = HashMap::new();
    // One material per Elite material, set (the seat's pieces draw in the seat's depth slice) and skinning (the seat's
    // chair's draws carry their bones).
    let mut mats: HashMap<(String, bool, bool), Handle<ShipMaterial>> = HashMap::new();
    let mut pieces = Vec::new();
    for (i, node) in rig.nodes.iter().enumerate() {
        let Some(model) = node.model else { continue };
        let inside = node.group.inside();
        let skinned = rig.models[model].skinned;
        for (key, mesh) in rig.models[model].draws.iter().zip(&model_meshes[model]) {
            // (Every drawn material was resolved when the files were read: `ShipBuild::materials`.)
            let Some(r) = resolved.get(key) else { continue };
            let mat = mats
                .entry((key.clone(), inside, skinned))
                .or_insert_with(|| {
                    if let Some(m) = rig.materials.get(key) {
                        let unread = material::unaccounted_attributes(m);
                        if !unread.is_empty() {
                            warn!("sidewinder: material {key}: attributes its family neither reads nor lists: {unread:?}");
                        }
                    }
                    let tex: Vec<Handle<Image>> = r
                        .textures
                        .iter()
                        .map(|s| match s {
                            TexSource::File { file, srgb } => textures.get(&(file.clone(), *srgb)).cloned().unwrap_or_else(|| white.clone()),
                            TexSource::Constant(c) => constants.entry(*c).or_insert_with(|| material::constant(&mut images, *c)).clone(),
                        })
                        .collect();
                    let mut params = if inside { r.params.in_seat() } else { r.params };
                    params.lights = list.range(inside);
                    params.heat_glow.x = (!inside && glow_element.as_deref() == Some(key.as_str())) as u32;
                    ship_materials.add(ShipMaterial { skinned, ..ShipMaterial::new(&light, &lights, &scene, &glow, &glass, &palette, &shadow, r.family, &tex, &white, &ies, params, r.cockpit) })
                })
                .clone();
            pieces.push(
                commands
                    .spawn((
                        Name::new(format!("Sidewinder {}", node.name)),
                        ShipPiece { node: i, inside },
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
    let families: std::collections::HashSet<(Family, bool, bool)> = mats.keys().filter_map(|(k, inside, skinned)| Some((resolved.get(k)?.family, *inside, *skinned))).collect();
    info!("sidewinder: the ship's materials: {} ({} pipelines by family, set and skinning), {} textures, {} constants", mats.len(), families.len(), textures.len(), constants.len());
    info!("sidewinder: the ship's lights: {} slots, {} the hull's (the outside's range), {} the cockpit's", list.lamps.len(), list.hull, list.lamps.len() - list.hull);
    commands.insert_resource(list);
    commands.insert_resource(lights);
    commands.insert_resource(scene);
    commands.insert_resource(glow);
    commands.insert_resource(glass);
    commands.insert_resource(palette);
    commands.insert_resource(shadow);
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
    commands.insert_resource(ShipDraw { pieces, shown: (false, false), nodes_shown: u64::MAX, placed: None });
}

/// The pilot chair's palette (`ShipPalette`) written in place when its pose changed (`ChairPose::revision`): each bone's
/// skinning matrix turned into the mod's axes (`ShipSpace::skin`); identities without a ship (the bind pose).
fn skin(assets: Res<SidewinderAssets>, pose: Res<ChairPose>, palette: Option<ResMut<ShipPalette>>, mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let Some(mut palette) = palette else { return };
    let space = assets.rig.space;
    palette.write(&mut buffers, pose.revision(), pose.skin_matrices().iter().map(|m| space.skin(*m)));
}

/// The parked ship where it stands this frame, from the camera, as `place_props` places the props; the canopy's glass effects
/// written where they changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn place(
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
    frame: Res<bevy::diagnostic::FrameCount>,
    lights: Option<Res<ShipLights>>,
    glass: Option<ResMut<ShipGlass>>,
    scene: Option<ResMut<ShipScene>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let Some(mut draw) = draw else { return };
    // No light from outside the ship in hyperspace's tunnel (ours): the star's, the sky's and the street lamps' terms at 0.
    if let Some(mut scene) = scene {
        let outside = if pilot.jump.is_some() { 0.0 } else { 1.0 };
        if scene.outside() != outside {
            scene.set_outside(&mut buffers, outside);
        }
    }
    // The canopy's glass effects: the crack amount the cockpit's cracks keep (`Parts::crack`), the rest as made.
    if let Some(mut g) = glass {
        g.write(&mut buffers, GlassEffects { crack: parts.crack(), ..GlassEffects::stock() });
    }
    draw.placed = None;
    // In hyperspace's tunnel no body is near (the old system's go): the ship is the jump's pose, its eye at the camera
    // (`seat::seat_camera` writes the camera there), the render frame's origin. Not `placed` there: no star lights it in the
    // tunnel (the outside light's scale, `ShipScene::set_outside`), so the shadow passes rest.
    let tunnel = pilot.jump.filter(|_| seat.view).map(|j| tunnel_place(&j, Vec3::from_array(assets.eye)));
    let near = match tunnel.or_else(|| {
        let p = ship.parked.as_ref()?;
        let b = bodies.iter().find(|b| b.obj == p.obj)?;
        let cam_body = b.orient.inverse() * cam.pos.delta_m(b.pos);
        let rel = p.pos - cam_body;
        (rel.length() < DRAW_M).then(|| {
            let k = space.factor(b.pos.delta_m(cam.pos).length());
            Transform { translation: (b.orient * rel * k).as_vec3(), rotation: (b.orient * p.rot).normalize().as_quat(), scale: Vec3::splat(k as f32) }
        })
    }) {
        Some(ship_tf) => {
            if tunnel.is_none() {
                draw.placed = Some(ship_tf);
            }
            for (piece, mut tf, _) in pieces.iter_mut() {
                *tf = ship_tf * Transform::from_matrix(parts.matrix(piece.node));
            }
            // (The lights' loop reads the ship's space: its pose this frame, while any light is on.)
            if let Some(l) = lights.as_ref().filter(|l| l.any_on()) {
                l.write_pose(&mut buffers, &ship_tf);
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
    let show = shown(near, seat.view);
    if draw.shown != show || draw.nodes_shown != parts.shown_revision() {
        if draw.shown != show {
            let what = match show {
                (true, _) => "the outside set (hull, gear, outside chair)",
                (_, true) => "the seat's set (cockpit, its chair)",
                _ => "nothing",
            };
            info!("sidewinder: drawing {what} from frame {}", frame.0);
        }
        draw.shown = show;
        draw.nodes_shown = parts.shown_revision();
        for e in draw.pieces.clone() {
            if let Ok((piece, _, mut v)) = pieces.get_mut(e) {
                // (A node the machines hide, `KModelState`, isn't drawn in either set: the windscreen once breached.)
                let on = if piece.inside { show.1 } else { show.0 } && parts.shown(piece.node);
                *v = if on { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::assets::{self, Group};

    /// One set at a time: never both, and exactly one whenever the ship is near enough to draw.
    #[test]
    fn one_set_at_a_time() {
        for near in [false, true] {
            for seat_view in [false, true] {
                let (outside, seat) = shown(near, seat_view);
                assert!(!(outside && seat), "near {near}, seat view {seat_view}");
                assert_eq!(outside || seat, near, "near {near}, seat view {seat_view}");
                assert_eq!(seat, near && seat_view);
            }
        }
    }

    /// The mod's ship (its shipped files, in git): every drawn node is in exactly one of the two sets; the seat's set is
    /// the cockpit rig's models and `cockpit_chair`, the outside set the hull rig's, the gear rigs' and `ext_cockpit_chair`.
    #[test]
    fn the_sets_split_every_drawn_node() {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        let rig = assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets.rig;
        let drawn: Vec<&assets::ShipNode> = rig.nodes.iter().filter(|n| n.model.is_some()).collect();
        let (seat, outside): (Vec<&&assets::ShipNode>, Vec<&&assets::ShipNode>) = drawn.iter().partition(|n| n.group.inside());
        assert_eq!(seat.len() + outside.len(), drawn.len());
        assert!(seat.iter().all(|n| matches!(n.group, Group::Cockpit | Group::InsideChair)));
        assert!(outside.iter().all(|n| matches!(n.group, Group::Hull | Group::Gear(_) | Group::OutsideChair)));
        let names = |set: &[&&assets::ShipNode]| set.iter().map(|n| n.name.to_lowercase()).collect::<Vec<_>>();
        let (seat_names, outside_names) = (names(&seat), names(&outside));
        assert!(seat_names.iter().any(|n| n == "cockpit_chair") && !seat_names.iter().any(|n| n == "ext_cockpit_chair"), "{seat_names:?}");
        assert!(outside_names.iter().any(|n| n == "ext_cockpit_chair") && !outside_names.iter().any(|n| n == "cockpit_chair"), "{outside_names:?}");
        for g in [Group::Hull, Group::Gear(0), Group::Cockpit] {
            assert!(drawn.iter().any(|n| std::mem::discriminant(&n.group) == std::mem::discriminant(&g)), "nothing drawn of {g:?}");
        }
        println!("{} drawn nodes: {} in the seat's set, {} outside", drawn.len(), seat.len(), outside.len());
    }
}
