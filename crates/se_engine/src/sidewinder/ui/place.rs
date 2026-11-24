//! Where the movies are drawn (ARCHITECTURE.md A10, "Where and how they are drawn (UI3)"): one piece for each draw of each
//! `gui_*` model and dark plate the layout gives a movie, with Elite's own mesh and uv0 (the uv rectangles cut the gauge
//! sheet), on its bone, placed every frame by the cockpit's own placement (`draw::place`'s frame of the ship x the node's
//! matrix now, `Parts::matrix`) times the bone's frame from the layout. The pieces are drawn by the UI's own pass
//! (`pass.rs`, after the engine's colour passes, at the view's full size): this file makes their geometry and materials
//! once (`UiGeometry`) and hands the pass each frame's matrices and order (`UiFrame`).
//!
//! - A panel on the cockpit model (`Root`) hangs on the cockpit rig's root node; one on the chair rig
//!   (`CockpitChair_Bone_Rig`) on that rig's node, which the cockpit rig hangs on its `!Pilot_Chair`. A bone the layout lacks
//!   in that frame gives way to the panel's fallback bone (`!Gauges_Panel` -> `!Radar`, the layout's stand-in, B).
//! - Every panel is drawn on its own `gui_*` model, the four components' panel quads (`MovieLayout::quad`) never: the quad's
//!   two triangles turn opposite ways in Elite's own data (indices 0 3 2 and 1 0 3: one of them faces away whatever the
//!   mirror), so with the panels' back-face culling Elite cannot draw it as a mesh (B: it is taken to be the panel's
//!   pointer surface; the comms quad is also 0.139 m off flat). The left and right models are the same rectangles.
//! - The two side panels and their plates are drawn 0.96 of the data's size about a point of their inner edge, the role
//!   panel 0.91 about its middle (`module_scale`: stand-ins, C, fitted on Elite's pictures, for the module manager's
//!   undecoded scales).
//! - The multicrew gauges (`Gui_MC_Gauges` and their plate) are not drawn (`MULTICREW_PANEL`).
//! - The chair rig's motion with the chair (`ChairPose`) is not applied: which of the chair's bones the rig follows is not
//!   decoded (`ui_rendering.md` 2.1: "Not found"), so the chair rig's panels hang on the rig's own node, still in the cockpit.
//! - Not drawn (their place is not decoded and no picture of session ui1 shows them): the target sights' cells and the orbital
//!   HUD. They run and take their messages. A screen movie (`notifications`) is a plain overlay of the screen
//!   (`draw_screens`), under the game's own windows.
//! - Everything is made once the UI's files are read (at the first spawn, with the ship's; the pass's pipelines are built
//!   then), then shown only from the seat, the seat's cockpit drawn (`ShipDraw::seat_shown`), while its movie is wanted and
//!   ready (a plate also while its movie's events show it: `plates.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use bevy_egui::egui;
use elite_ui::feed::Movie as Which;
// (The chair rig's node in the cockpit rig, `ship.json`, is named as the layout's rig.)
use se_core::elite_ui_layout::{BoneFrame, CHAIR_RIG_NAME, GuiSlotValue, Layout, MaterialTag, MovieKind, Rig};

use super::super::draw::ShipDraw;
use super::super::material as ship_material;
use super::super::parts::Parts;
use super::super::seat::Seat;
use super::super::{Sidewinder, SidewinderAssets};
use super::load::{self, slot_repeats, UiFiles};
use super::material::{GuiFlags, PanelParams, PlateParams, GUI_BRIGHTNESS, SCREEN_FIT};
use super::pass::{Built, PieceGeom, PieceKind, PieceRecord, UiFrame, UiGeometry, UiGlobalsGpu, UiMaterialData, UiVertex};
use super::movies::{target_image, Handoff, Slot, UiMovies};
use super::plates::Plates;
use super::{movie_named, UiMovieState, UiReplies, UiWanted, HUD};

/// The gauge movie's multicrew gauges (`ui_rendering.md` 2.1: "Gui_MC_Gauges (multicrew gauges)") are not drawn, nor their
/// plate: on the Sidewinder's single seat Elite shows no second set of the pips, hull figure and speed beside the scanner
/// that this model's islands cut from the sheet (session ui1's `a00_start.png`, C; which cockpit types draw it was not
/// traced).
const MULTICREW_PANEL: &str = "Gui_MC_Gauges";

/// One drawn piece (its geometry: the same place in `UiGeometry::pieces`): the rig node it hangs on, the bone's frame in that
/// node's model space, its movie, its plate, its material's depth flag.
struct UiPiece {
    node: usize,
    frame: Mat4,
    movie: Which,
    plate: Option<usize>,
    depth_test: bool,
}

/// The UI's files being read.
#[derive(Resource)]
pub struct UiLoading(Task<Result<UiFiles, String>>);

/// The UI could not be made (its files past the mod's check, or a pipeline of its shaders): it stays off for this ship,
/// the ship works.
#[derive(Resource)]
pub struct UiOff;

/// The UI, once made.
#[derive(Resource)]
pub struct UiDraw {
    pieces: Vec<UiPiece>,
    /// The pieces in the order they are drawn: the plates first, then the panels by their render layer (Elite's sort).
    order: Vec<u32>,
    geometry: Arc<UiGeometry>,
    pub plates: Plates,
    /// The screen movies' images (drawn by `draw_screens`).
    screens: Vec<(Which, Handle<Image>)>,
}

/// The first spawn's files read and the ship's materials made (`ShipScene`): the UI's files are read too.
pub fn start_load(mut commands: Commands) {
    info!("sidewinder ui: reading the UI's files");
    commands.insert_resource(UiLoading(AsyncComputeTaskPool::get().spawn(async move { load::load() })));
}

/// The bone's frame as a matrix of its rig's space (columns: its axes, its position).
pub(super) fn bone_matrix(b: &BoneFrame) -> Mat4 {
    let [x, y, z] = b.axes.map(Vec3::from_array);
    Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), Vec3::from_array(b.position).extend(1.0))
}

/// The node and bone frame a piece on `bone` (else `fallback`) of `rig` hangs on.
fn hang(layout: &Layout, nodes: (Option<usize>, Option<usize>), bone: &str, fallback: Option<&str>, rig: Rig) -> Option<(usize, Mat4)> {
    let node = match rig {
        Rig::Cockpit => nodes.0,
        Rig::Chair => nodes.1,
    }?;
    let frame = layout.bone_frame(bone, rig).or_else(|| fallback.and_then(|f| layout.bone_frame(f, rig)))?;
    Some((node, bone_matrix(frame)))
}

/// The side panels' size, a stand-in (grade C) for the UIModuleManager's per-panel scale, which the exe applies to a GUI
/// panel when it opens but whose use was never read (`ui_rendering.md` 2.3 and section 5, "Still open" 2; its value for
/// `Gui_Left_Panel` and `Gui_Right_Panel` is 0.78, which does not fit: E). Fitted instead on session ui1's settled pictures
/// (`re/elite/measure/raw/ui1/shots/p1_left_open_0.png`, the same as `p2_left_3.png` to the pixel, and `p2_right_3.png`,
/// 3440 x 1440, against the game's at the same head turn and the data's -17 degree view): 9 points of the left panel's
/// content and 8 of the right's, each panel taken back onto its plane, give ONE mirrored similarity: Elite's panel is the
/// layout's scaled by 0.9603 +- 0.0025 about `SIDE_PANEL_PIVOT` (the right panel's pivot its mirror about the cockpit's
/// middle), within both panels' own fits (left alone 0.9567 +- 0.0047 about uv (0.98 +- 0.09, 0.30 +- 0.07), right alone
/// 0.9634 +- 0.0017 about uv (-0.16 +- 0.04, 0.37 +- 0.03)). Fitting tools: `.optimize/scratch/run/ui3a/tools/match.py`,
/// `panel_joint.py`.
const SIDE_PANEL_SCALE: f32 = 0.9603;

/// The left panel's pivot in the cockpit's frame (the eye at `!Camera`, metres: on the left quad's plane at its uv
/// (1.06 +- 0.06, 0.34 +- 0.04), the middle of its inner edge a third of the way down).
const SIDE_PANEL_PIVOT: Vec3 = Vec3::new(-0.9136, -0.1949, -0.9284);

/// The role panel's size, the same stand-in (C) for its module-manager scale (1.28 in the data, E): fitted the same way on
/// `p2_role_2.png` (the nine role pictures agree within 0.01 degrees) against the game's at the measured role turn, 6
/// points of its content's frame and labels: 0.9120 +- 0.0017 about the bone's origin, the quad's middle (free, the pivot
/// comes out at uv (0.48 +- 0.01, 0.52 +- 0.01): the middle; rms 2.7 px). (For the notes: 1056 / 1152 = 0.9167, the role
/// stage's width over the width its quad's aspect gives at its height, is 2.8 sigma off it.) The comms panel has no picture
/// of Elite's and is left as the data gives it.
const ROLE_PANEL_SCALE: f32 = 0.9120;

/// The panel's size from the module manager's stand-ins (`SIDE_PANEL_SCALE`, `ROLE_PANEL_SCALE`), as a matrix of its rig's
/// space (the bone's frame `frame` gives the role panel's pivot): the identity for every other panel (the plates follow
/// their panel by its name). It is put into the piece's mesh (`in_bone`, `sized`), never into its placement: the shaders
/// take the model matrix's scale as the ship's own (the seat's depth slice, `seat_z`), and a smaller one put the panel
/// behind the cockpit's bezels.
fn module_scale(layout: &Layout, panel: &str, rig: Rig, frame: Mat4) -> Mat4 {
    let named = |n: &str| panel.eq_ignore_ascii_case(n);
    let (scale, pivot) = if named("Gui_Left_Panel") || named("Gui_Right_Panel") {
        let side = if named("Gui_Left_Panel") { 1.0 } else { -1.0 };
        let rig_offset = if rig == Rig::Chair { Vec3::from_array(layout.chair_rig.offset) } else { Vec3::ZERO };
        (SIDE_PANEL_SCALE, SIDE_PANEL_PIVOT * Vec3::new(side, 1.0, 1.0) - rig_offset)
    } else if named("Gui_RoleSwitch_Panel") {
        (ROLE_PANEL_SCALE, frame.w_axis.truncate())
    } else {
        return Mat4::IDENTITY;
    };
    Mat4::from_translation(pivot) * Mat4::from_scale(Vec3::splat(scale)) * Mat4::from_translation(-pivot)
}

/// A matrix of the rig's space as the same change in the bone's own space (`frame` the bone's frame).
fn in_bone(rig_space: Mat4, frame: Mat4) -> Mat4 {
    frame.inverse() * rig_space * frame
}

/// A material's values as its uniform holds them.
fn uniform_bytes<T: bevy::render::render_resource::ShaderType + bevy::render::render_resource::encase::internal::WriteInto>(v: &T) -> Vec<u8> {
    let mut b = bevy::render::render_resource::encase::UniformBuffer::new(Vec::new());
    // (Writing into a growing Vec cannot fail.)
    let _ = b.write(v);
    b.into_inner()
}

/// Everything the build needs to make the materials and the geometry.
struct Maker<'a> {
    layout: &'a Layout,
    textures: HashMap<load::TexKey, Handle<Image>>,
    white: Handle<Image>,
    geometry: UiGeometry,
    /// A material's place in `geometry.materials` by its name (a panel's also by its movie: the movie's target is its
    /// texture), and its render layer.
    made: HashMap<(String, Option<Which>), (u32, f32)>,
}

impl Maker<'_> {
    /// A material's texture of `slot` (white when the slot holds none of the shipped files).
    fn texture(&self, name: &str, slot: &str, images: &mut Assets<Image>) -> Handle<Image> {
        let m = &self.layout.materials[name];
        match m.textures.iter().find(|t| t.slot == slot).map(|t| &t.value) {
            Some(GuiSlotValue::Tex { file, srgb, .. }) => self.textures.get(&(file.clone(), *srgb, slot_repeats(slot))).cloned().unwrap_or_else(|| self.white.clone()),
            Some(GuiSlotValue::Colour { colour }) => ship_material::constant(images, colour[0]),
            _ => self.white.clone(),
        }
    }

    /// A material's place and render layer, made the first time it is asked for: a panel's (`movie` its movie's target)
    /// or a plate's.
    fn material(&mut self, name: &str, movie: Option<(Which, &Handle<Image>)>, images: &mut Assets<Image>) -> Result<(u32, f32), String> {
        let key = (name.to_string(), movie.map(|(w, _)| w));
        if let Some(made) = self.made.get(&key) {
            return Ok(*made);
        }
        let m = &self.layout.materials[name];
        let flags = GuiFlags::of(&m.attrs).map_err(|e| format!("material {name}: {e}"))?;
        let data = match movie {
            Some((_, target)) => UiMaterialData {
                kind: PieceKind::Panel,
                flags,
                params: uniform_bytes(&PanelParams::new(&m.attrs).map_err(|e| format!("material {name}: {e}"))?),
                textures: vec![target.clone(), self.texture(name, "pCloudynessTexture", images)],
            },
            None => UiMaterialData {
                kind: PieceKind::Plate,
                flags,
                params: uniform_bytes(&PlateParams::new(&m.attrs).map_err(|e| format!("material {name}: {e}"))?),
                textures: vec![self.texture(name, "pDiffuse_AlphaTexture", images), self.texture(name, "pCloudynessTexture", images), self.texture(name, "pScanlinesTexture", images)],
            },
        };
        let made = (self.geometry.materials.len() as u32, render_layer(m)?);
        self.geometry.materials.push(data);
        self.made.insert(key, made);
        Ok(made)
    }

    /// A model's draw as a piece of the geometry: its positions moved by `size` (a uniform scale about a point in the bone's
    /// space, `module_scale`: the normals and tangents stay), drawn with `material`.
    fn piece(&mut self, mesh: &Mesh, size: Mat4, material: u32) -> Result<(), String> {
        let attr = |a| mesh.attribute(a).ok_or("a UI model's draw without its attributes");
        let (Some(VertexAttributeValues::Float32x3(p)), Some(VertexAttributeValues::Float32x3(n)), Some(VertexAttributeValues::Float32x4(t)), Some(VertexAttributeValues::Float32x2(uv))) =
            (attr(Mesh::ATTRIBUTE_POSITION).ok(), attr(Mesh::ATTRIBUTE_NORMAL).ok(), attr(Mesh::ATTRIBUTE_TANGENT).ok(), attr(Mesh::ATTRIBUTE_UV_0).ok())
        else {
            return Err("a UI model's draw without its position, normal, tangent and uv0".to_string());
        };
        let Some(Indices::U16(indices)) = mesh.indices() else { return Err("a UI model's draw without its 16-bit indices".to_string()) };
        let g = &mut self.geometry;
        g.pieces.push(PieceGeom { first: g.indices.len() as u32, count: indices.len() as u32, base: g.vertices.len() as i32, material });
        g.indices.extend(indices.iter().map(|&i| i as u32));
        g.vertices.extend((0..p.len()).map(|i| UiVertex { position: size.transform_point3(Vec3::from_array(p[i])).to_array(), normal: n[i], tangent: t[i], uv: uv[i] }));
        Ok(())
    }
}

fn render_layer(m: &se_core::elite_ui_layout::GuiMaterial) -> Result<f32, String> {
    m.attrs.iter().find(|a| a.name == "pRenderLayer").and_then(|a| a.values.first()).map(|v| *v as f32).ok_or_else(|| "a material without pRenderLayer".to_string())
}

/// The files read: the movies' slots and images, the materials, meshes and pieces (`make`); a failure leaves the UI off.
#[allow(clippy::too_many_arguments)]
pub fn build(world: &mut World) {
    let Some(mut loading) = world.get_resource_mut::<UiLoading>() else { return };
    let Some(done) = futures::check_ready(&mut loading.0) else { return };
    world.remove_resource::<UiLoading>();
    let made = done.and_then(|files| {
        if world.resource::<Sidewinder>().parked.is_none() {
            return Err("the ship went while its UI's files were read".to_string());
        }
        make(world, files)
    });
    if let Err(e) = made {
        off(world, &e);
    }
}

fn make(world: &mut World, files: UiFiles) -> Result<(), String> {
    let UiFiles { layout, fonts, host, mut movies, models, textures, panels } = files;
    world.insert_resource(panels);
    let rig = world.resource::<SidewinderAssets>().rig.clone();
    let nodes = (rig.cockpit_node(), rig.nodes.iter().position(|n| n.group == super::super::assets::Group::Cockpit && n.name.eq_ignore_ascii_case(CHAIR_RIG_NAME)));
    if nodes.0.is_none() || nodes.1.is_none() {
        return Err(format!("the cockpit's nodes (its root {:?}, the chair rig {:?}) are not in the ship's rig", nodes.0, nodes.1));
    }
    let mut slots = Vec::new();
    let mut screens = Vec::new();
    let handoff = world.resource::<Handoff>().clone();
    let draw = world.resource_scope(|_, mut images: Mut<Assets<Image>>| -> Result<UiDraw, String> {
        let white = ship_material::white(&mut images);
        let tex: HashMap<load::TexKey, Handle<Image>> = textures.into_iter().map(|(k, i)| (k, images.add(i))).collect();
        let mut maker = Maker { layout: &layout, textures: tex, white, geometry: UiGeometry::default(), made: HashMap::new() };
        // (Each piece with its draw order's key: plates first, then the panels by their render layer.)
        let mut pieces: Vec<(UiPiece, (bool, f32))> = Vec::new();
        let mut plate_index = 0usize;
        for m in &layout.movies {
            let Some(which) = movie_named(&m.name) else {
                warn!("sidewinder ui: the layout's movie {} is not one of the scope's", m.name);
                plate_index += m.backgrounds.len();
                continue;
            };
            let drawn = matches!(m.kind, MovieKind::Panels | MovieKind::Screen);
            let mut slot = Slot::new(which, m.stage, drawn, m.kind == MovieKind::Screen, movies.remove(&m.name).unwrap_or_default());
            if drawn {
                let image = images.add(target_image(m.stage, slot.srgb_view));
                handoff.want(image.id());
                slot.image = Some(image.clone());
                if m.kind == MovieKind::Screen {
                    screens.push((which, image.clone()));
                }
            }
            if m.kind == MovieKind::Panels {
                let image = slot.image.clone().ok_or("a panel movie without its image")?;
                for p in m.panels.iter().filter(|p| p.name != MULTICREW_PANEL) {
                    let Some((node, frame)) = hang(&layout, nodes, &p.bone, p.bone_fallback.as_deref(), p.rig) else {
                        warn!("sidewinder ui: {}: no frame of {} on {:?}", p.name, p.bone, p.rig);
                        continue;
                    };
                    let size = in_bone(module_scale(&layout, &p.name, p.rig, frame), frame);
                    let draws = models.get(&p.model).ok_or_else(|| format!("{}: no model {}", p.name, p.model))?;
                    for d in draws {
                        let (material, layer) = maker.material(&d.material, Some((which, &image)), &mut images)?;
                        maker.piece(&d.mesh, size, material)?;
                        let depth_test = maker.geometry.materials[material as usize].flags.depth_test();
                        pieces.push((UiPiece { node, frame, movie: which, plate: None, depth_test }, (true, layer)));
                    }
                }
                for b in &m.backgrounds {
                    let index = plate_index;
                    plate_index += 1;
                    if b.parent == MULTICREW_PANEL {
                        continue;
                    }
                    let Some((node, frame)) = hang(&layout, nodes, &b.bone, None, b.rig) else {
                        warn!("sidewinder ui: {}: no frame of {} on {:?}", b.name, b.bone, b.rig);
                        continue;
                    };
                    let size = in_bone(module_scale(&layout, &b.parent, b.rig, frame), frame);
                    let draws = models.get(&b.model).ok_or_else(|| format!("{}: no model {}", b.name, b.model))?;
                    for d in draws.iter().filter(|d| layout.materials.get(&d.material).is_some_and(|x| x.tag == MaterialTag::Background)) {
                        let (material, layer) = maker.material(&d.material, None, &mut images)?;
                        maker.piece(&d.mesh, size, material)?;
                        let depth_test = maker.geometry.materials[material as usize].flags.depth_test();
                        pieces.push((UiPiece { node, frame, movie: which, plate: Some(index), depth_test }, (false, layer)));
                    }
                }
            } else {
                plate_index += m.backgrounds.len();
            }
            slots.push(slot);
        }
        let mut order: Vec<u32> = (0..pieces.len() as u32).collect();
        order.sort_by(|&a, &b| {
            let (ka, kb) = (pieces[a as usize].1, pieces[b as usize].1);
            ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        });
        let geometry = Arc::new(maker.geometry);
        info!(
            "sidewinder ui: {} pieces made ({} materials, {} vertices), drawn by the UI's pass once its pipelines are built",
            pieces.len(),
            geometry.materials.len(),
            geometry.vertices.len()
        );
        Ok(UiDraw { pieces: pieces.into_iter().map(|(p, _)| p).collect(), order, geometry, plates: Plates::new(&layout), screens: std::mem::take(&mut screens) })
    })?;
    world.resource::<Built>().reset();
    world.insert_resource(UiFrame {
        geometry: Some(draw.geometry.clone()),
        records: vec![PieceRecord::default(); draw.pieces.len()],
        order: Vec::new(),
        globals: globals(layout.shading.gui_colour_matrix),
    });
    world.insert_resource(draw);
    world.insert_resource(super::UiLive(true));
    world.insert_non_send(UiMovies::new(slots, fonts, host));
    let mut wanted = world.resource_mut::<UiWanted>();
    if wanted.movies.is_empty() {
        wanted.movies = HUD.to_vec();
    }
    Ok(())
}

/// The pass's values: Elite's globals (gGuiBrightness `GUI_BRIGHTNESS`; gExposureCompensation, gHDRScalingWrite and
/// gGuiBackgroundMultiplier 1, E: their product with the scene is in `SCREEN_FIT`), the seat's depth slice, the screen fit, the
/// GUI colour matrix.
fn globals(matrix: [[f32; 3]; 4]) -> UiGlobalsGpu {
    UiGlobalsGpu {
        gui: [GUI_BRIGHTNESS, 1.0, 1.0, 1.0],
        seat: [super::super::material::SEAT_SLICE, super::super::material::SEAT_NEAR_M, 0.0, 0.0],
        fit: [SCREEN_FIT[0], SCREEN_FIT[1], SCREEN_FIT[2], 0.0],
        matrix: std::array::from_fn(|i| [matrix[i][0], matrix[i][1], matrix[i][2], 0.0]),
    }
}

/// The ship gone: the movies, their images and the pieces go (a new spawn makes them again).
pub fn teardown(world: &mut World) {
    drop_all(world);
    info!("sidewinder ui: the ship is gone: its movies and their images are dropped");
}

/// A new ship replaced the parked one: the old one's movies, images and pieces go, and `load_due` makes the new one's.
pub fn renew(world: &mut World) {
    drop_all(world);
    info!("sidewinder ui: a new ship: the old ship's movies are dropped and loaded again");
}

/// A pipeline of the UI's pass failed (`pass.rs` logged it once): the UI goes off for this ship.
pub fn turn_off(world: &mut World) {
    off(world, "a pipeline of its pass failed");
}

/// Everything of the UI goes and it stays off for this ship, which works without it: the stand-in HUD and side panels take
/// over at once (`UiLive` false), with this one log line.
pub fn off(world: &mut World, why: &str) {
    drop_all(world);
    world.insert_resource(UiOff);
    warn!("sidewinder ui: Elite's UI is off ({why}): the stand-in HUD and panels take over");
}

/// A pipeline of the UI's pass failed.
pub fn failed(built: Res<Built>, off: Option<Res<UiOff>>) -> bool {
    off.is_none() && built.failed()
}

/// The movies, their images, the pieces and the loader's state go.
fn drop_all(world: &mut World) {
    world.insert_resource(super::UiLive(false));
    // (What the UI held in the seat's and panels' shared state goes with it, or the stand-ins would take it over: an FSS
    // covering the view, a panel opened by the focus machine with its field of view.)
    world.insert_resource(super::screens::UiScreens::default());
    world.resource_mut::<Seat>().screen = false;
    world.resource_mut::<super::super::panels::Panels>().close();
    world.resource::<Built>().reset();
    world.remove_resource::<UiDraw>();
    world.insert_resource(UiFrame::default());
    world.remove_resource::<UiLoading>();
    world.remove_resource::<UiOff>();
    world.remove_non_send::<UiMovies>();
    world.resource::<Handoff>().clear();
    let mut state = world.resource_mut::<UiMovieState>();
    state.ready.clear();
    state.fresh.clear();
}

/// Each frame once made: the plates' events and times (while seated), and the pass's frame: every piece where its node is now and the pieces shown, in order (`UiFrame`).
#[allow(clippy::too_many_arguments)]
pub fn place(
    draw: Option<Res<ShipDraw>>,
    parts: Res<Parts>,
    mut ui: ResMut<UiDraw>,
    mut frame: ResMut<UiFrame>,
    (state, wanted, replies): (Res<UiMovieState>, Res<UiWanted>, Res<UiReplies>),
    (seat, control, projection): (Res<Seat>, Res<crate::mods::ModControl>, Res<crate::projection::ProjectionSettings>),
    time: Res<Time<Real>>,
) {
    let ui = &mut *ui;
    let seated = super::seated(&seat, &control);
    if seated {
        for (movie, out) in &replies.outs {
            ui.plates.apply(movie.name(), out);
        }
        ui.plates.tick(time.delta_secs());
    }
    // (Drawn from the seat, the seat's cockpit drawn, in the plain view: the projections draw the scene through cube faces.)
    let ship = draw.as_ref().filter(|d| seated && d.seat_shown() && projection.mode == 0).and_then(|d| d.frame());
    let frame = &mut *frame;
    frame.order.clear();
    let Some(ship) = ship.map(|s| s.to_matrix()) else { return };
    for (record, piece) in frame.records.iter_mut().zip(&ui.pieces) {
        let model = ship * parts.matrix(piece.node) * piece.frame;
        *record = PieceRecord { model: model.to_cols_array_2d(), info: [model.x_axis.truncate().length(), if piece.depth_test { 1.0 } else { 0.0 }, 0.0, 0.0] };
    }
    for &i in &ui.order {
        let piece = &ui.pieces[i as usize];
        let movie_on = wanted.movies.contains(&piece.movie) && state.ready.contains(&piece.movie);
        let plate_on = piece.plate.is_none_or(|k| ui.plates.plates.get(k).is_some_and(|p| p.shown()));
        if movie_on && plate_on {
            frame.order.push(i);
        }
    }
}

/// The screen movies (`notifications`) while seated, wanted and ready: their 1920 x 1080 target over the screen, its height
/// the screen's, centred (ours, B: the screen GUI target is 1920 x 1080, A; how Elite fits it to a wider screen is not
/// decoded), on egui's background layer, under the game's own windows. Their colours as they are: Elite's screen-space
/// shader without the matrix is `pow(rgb x gStandardMapScales.x x pGuiTint, 2.2 / gamma)` (`notifications`: 1 and 1), the
/// identity at a gamma of 2.2 (a stand-in, E: `pToneMap_brightness_contrast_sat_gamma.w` is not found).
pub fn draw_screens(mut contexts: bevy_egui::EguiContexts, ui: Res<UiDraw>, state: Res<UiMovieState>, wanted: Res<UiWanted>, seat: Res<Seat>, control: Res<crate::mods::ModControl>) -> Result {
    if !super::seated(&seat, &control) || control.main_menu {
        return Ok(());
    }
    let shown: Vec<egui::TextureId> = ui
        .screens
        .iter()
        .filter(|(m, _)| wanted.movies.contains(m) && state.ready.contains(m))
        .map(|(_, image)| contexts.add_image(bevy_egui::EguiTextureHandle::Weak(image.id())))
        .collect();
    if shown.is_empty() {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let r = ctx.content_rect();
    let w = r.height() * 1920.0 / 1080.0;
    let rect = egui::Rect::from_center_size(r.center(), egui::vec2(w, r.height()));
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("sidewinder ui screens")));
    for id in shown {
        p.image(id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui/layout.json");
        se_core::elite_ui_layout::parse_layout(&std::fs::read(path).unwrap()).unwrap()
    }

    /// The panels' stand-in scales hold their pivots (the right's the left's mirror, both in the cockpit's frame; the role
    /// panel's its bone's origin) and scale everything else about them; every other panel is left as the data gives it.
    #[test]
    fn panels_scale_about_their_pivots() {
        let l = layout();
        let off = Vec3::from_array(l.chair_rig.offset);
        let role = bone_matrix(l.bone_frame("!RoleSwitch_Panel", Rig::Chair).unwrap());
        for (panel, at, scale) in [
            ("Gui_Left_Panel", SIDE_PANEL_PIVOT - off, SIDE_PANEL_SCALE),
            ("Gui_Right_Panel", SIDE_PANEL_PIVOT * Vec3::new(-1.0, 1.0, 1.0) - off, SIDE_PANEL_SCALE),
            ("Gui_RoleSwitch_Panel", role.w_axis.truncate(), ROLE_PANEL_SCALE),
        ] {
            let m = module_scale(&l, panel, Rig::Chair, role);
            assert!(m.transform_point3(at).distance(at) < 1e-5, "{panel}");
            let away = at + Vec3::new(0.3, -0.2, 0.1);
            let d = m.transform_point3(away) - at;
            assert!((d.length() - scale * (away - at).length()).abs() < 1e-5, "{panel}");
        }
        for panel in ["Gui_Comms_panel", "Gui_Gauges_Panel", "Gui_Info_panel"] {
            assert_eq!(module_scale(&l, panel, Rig::Chair, role), Mat4::IDENTITY, "{panel}");
        }
        // The pivot lies on the left panel's plane (the fit's uv 1.06, 0.34), within a millimetre.
        let movie = l.movies.iter().find(|m| m.name == "cockpitexternalpanel").unwrap();
        let q = movie.quad.as_ref().unwrap();
        let frame = bone_matrix(l.bone_frame(&q.bone, q.rig).unwrap());
        let c: Vec<Vec3> = q.vertices.iter().map(|v| frame.transform_point3(Vec3::new(v[0], v[1], v[2])) + off).collect();
        let n = (c[1] - c[0]).cross(c[2] - c[0]).normalize();
        assert!((SIDE_PANEL_PIVOT - c[0]).dot(n).abs() < 1e-3);
    }
}
