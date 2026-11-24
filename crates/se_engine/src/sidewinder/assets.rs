//! The mod's files, read and made ready at the first spawn (ARCHITECTURE.md A6), on a worker thread through the mods' checked
//! reader (`mods::load_sidewinder`): Elite's models (`models/NAME.edm`, decoded exactly by `se_core::elite::Vertex::decode`
//! and turned into the mod's axes by `se_core::elite::to_mod_space`), `materials.json`, `ship.json` (the rigs, machines,
//! paint job and the ship item's facts) and the textures the drawn materials' families read. From them: the ship's
//! assembly (`ShipRig`: every rig node in one list, posed by `ShipRig::pose`), its solid shape, feet and belly (`ShipShape`),
//! the pilot's eye and the guns' points.
//!
//! Two spaces, and `ShipSpace` is the only thing that converts between them:
//! - Elite's model space: +X starboard, +Y up, +Z the nose (left-handed), metres: `ship.json`, the bones, `elite_rig`.
//! - The ship's: X right, Y up, -Z the nose, the origin on the ground under Elite's model origin with the gear down: what
//!   `Parked`, the solids, the seat and the flight use. It is Elite's mirrored in z and lifted by the rest height, which the
//!   three gear rigs posed in their `Down` state give (the lowest point of the legs on the ground).
//!
//! Where things hang (the ship item's facts, `ship.json` `item`): the gear rigs at the hull rig's `LandingGearRoot` bones;
//! the cockpit rig at the hull's `!Cockpit` bone (`COCKPIT_BONE`); the two chairs at the item's two seat bones
//! (`Sidewinder!Pilot_Chair`, `Sidewinder_Cockpit!Pilot_Chair`), paired with its two chair models in order.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::math::{DMat4, DVec3, Mat4, Vec3};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use se_core::elite::{to_mod_space, DecodedVertex};
use se_core::elite_asset::{parse_edm, Binds, Edm, VertexDecode};
use se_core::elite_rig::{self, BoneTable, NodePose, RigError, Values};
use se_core::elite_ship::{parse_materials, parse_ship, Machine, Materials, Paint, Rig, Ship};

use super::material::{self, Family, ATTRIBUTE_BONE, ATTRIBUTE_COLOUR, ATTRIBUTE_UV_2};

const SHIP_FILE: &str = "ship.json";
const MATERIALS_FILE: &str = "materials.json";
pub(crate) const KEYS_FILE: &str = "binds/KeyboardMouseOnly.binds";
const PAD_FILE: &str = "binds/ControlPad.binds";
/// The cockpit's rig is the one from the export file `cockpit`: `ship.json` holds two rigs named `Sidewinder_Cockpit`, and
/// the other (`cockpit_tutorial`) hangs the same cockpit with the tutorial's highlight overlays.
const COCKPIT_RIG_FILE: &str = "cockpit";
/// The hull's bone the cockpit rig hangs from. The item's facts name the cockpit's model but no bone; the hull's own
/// `!Cockpit` is the one: the cockpit model carries a copy of the hull's shell as seen from the seat (its painted plates,
/// glows and bespoke parts), which lands within 4 cm of the hull's from this bone (18 cm, had the cockpit's seat bone been put on the hull's) (the ignored test
/// `the_cockpit_fits_the_hull`).
const COCKPIT_BONE: &str = "!Cockpit";
/// The gear legs' machines' states (`LandingGearCSM`) and the hull's gear doors' (`LandingGearDoors*`).
const LEG_DOWN: &str = "Down";
const LEG_UP: &str = "Up";
const DOORS_OPEN: &str = "DoorsOpen";
const DOORS_CLOSED: &str = "DoorsClosed";
/// The guns' mounts: the first two of the item's hardpoint bones (`!Hardpoint0`, `!Hardpoint1`, either side of the nose under
/// the plates the hardpoint doors' machines open). The Sidewinder has two small hardpoints; the item's facts don't give the
/// bones' sizes, and `!Hardpoint2`, `!Hardpoint3` (at the wing tips) have no doors. The weapon's own model is not among the
/// mod's files, so the beam starts at the mount.
const GUN_HARDPOINTS: [usize; 2] = [0, 1];
/// The pilot's eye: the cockpit model's own camera bone (grade B: Elite's named point; the exe's use of it is not traced).
/// It sits at the cockpit model's origin with no turn (decoded `sidewinder_cockpit.json`: 1.610 m over its `!Pilot_Chair`,
/// 0.129 m behind it), so the eye looks along the nose. The hull's own `!Camera` is not it (0.36 m lower, 0.65 m nearer the
/// nose: from it the frame's outline disagrees with the user's `1.webp` on twice as many pixels). Checked against `1.webp` by the frame's outline
/// over the planet (`re/elite/work/eye`, T4b-1's report): within 5 px of the picture with a focal of 672 px, i.e. a field of
/// view near 63 degrees; at the settings' 56.249 the picture would put the eye 0.17 m further back.
const EYE_BONE: &str = "!Camera";
/// A foot is the middle of a leg's points within this of its lowest (its pad's sole).
const FOOT_BAND_M: f32 = 0.01;

/// Elite's model space and the ship's (the module's notes): the one conversion between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShipSpace {
    /// The rest height: how far Elite's model origin stands over the ground with the gear down (m).
    pub lift: f64,
}

/// Elite's z against the ship's: the mirror.
const MIRROR: DMat4 = DMat4::from_diagonal(bevy::math::DVec4::new(1.0, 1.0, -1.0, 1.0));

impl ShipSpace {
    /// A point in Elite's model space, in the ship's.
    pub fn point(&self, p: DVec3) -> Vec3 {
        DVec3::new(p.x, p.y + self.lift, -p.z).as_vec3()
    }

    /// A direction in Elite's model space, in the ship's.
    pub fn direction(&self, d: DVec3) -> Vec3 {
        DVec3::new(d.x, d.y, -d.z).as_vec3()
    }

    /// A node's matrix (its model's space to Elite's model space), as one from the model's vertices in the mod's axes
    /// (`to_mod_space`) to the ship's space.
    pub fn matrix(&self, m: DMat4) -> Mat4 {
        (DMat4::from_translation(DVec3::new(0.0, self.lift, 0.0)) * MIRROR * m * MIRROR).as_mat4()
    }

    /// A skinning matrix of a model's bone (bind pose to posed, both in the model's own space in Elite's axes, as
    /// `elite_chair` gives them), as one for the model's vertices in the mod's axes (`to_mod_space`): the mirror on the right
    /// takes the vertex into Elite's axes, where the matrix applies, and the one on the left takes the result back. No lift:
    /// the node's matrix (`matrix`) places the model after.
    pub fn skin(&self, m: Mat4) -> Mat4 {
        (MIRROR * m.as_dmat4() * MIRROR).as_mat4()
    }

    /// `point` undone.
    #[cfg(test)]
    pub fn elite_point(&self, p: Vec3) -> DVec3 {
        let p = p.as_dvec3();
        DVec3::new(p.x, p.y - self.lift, -p.z)
    }

    /// `matrix` undone.
    #[cfg(test)]
    pub fn elite_matrix(&self, m: Mat4) -> DMat4 {
        MIRROR * DMat4::from_translation(DVec3::new(0.0, -self.lift, 0.0)) * m.as_dmat4() * MIRROR
    }
}

/// What a rig node belongs to: which parts show outside and which only seated, and which leg a gear node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Hull,
    /// A gear leg's node (its index in the item's gear list).
    Gear(usize),
    Cockpit,
    /// The chair seen from outside (`ext_cockpit_chair`, at the hull's seat bone).
    OutsideChair,
    /// The chair seen seated (`cockpit_chair`, at the cockpit's seat bone).
    InsideChair,
}

impl Group {
    /// Drawn only while seated.
    pub fn inside(self) -> bool {
        matches!(self, Group::Cockpit | Group::InsideChair)
    }
}

/// Whose a state machine is (`ShipRig::machine_owner`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineOwner {
    Hull,
    Gear,
    Cockpit,
    /// Another file's (the cockpit tutorial's).
    Other,
}

/// One rig node (or chair) of the ship, in `ShipRig::pose`'s order.
#[derive(Clone, Debug)]
pub struct ShipNode {
    pub name: String,
    /// Its model in `ShipRig::models`; None when it has none or it isn't drawn yet (`material::model_drawn`).
    pub model: Option<usize>,
    pub group: Group,
}

/// A model the ship draws: its draws that are drawn now (`material::material_drawn`), each by its material's key in
/// `materials.json`, and whether its meshes carry the vertices' bones to be skinned by the pilot chair's palette (the seat's
/// chair alone).
#[derive(Clone, Debug)]
pub struct ShipModel {
    pub draws: Vec<String>,
    pub skinned: bool,
}

/// A gear rig and where it hangs: the hull rig's node and that node's model's bone.
#[derive(Clone, Debug)]
struct Hang {
    rig: usize,
    node: String,
    bone: String,
}

/// A seat bone (`Node!Bone` in the item's facts) and the chair model at it.
#[derive(Clone, Debug)]
struct Seat {
    node: String,
    bone: String,
    chair: String,
}

/// The ship's assembly: the rigs, their bones and the models they hang, in one list of nodes.
pub struct ShipRig {
    pub space: ShipSpace,
    pub ship: Ship,
    pub materials: Materials,
    pub models: Vec<ShipModel>,
    pub nodes: Vec<ShipNode>,
    bones: BoneTable,
    /// Indices in `ship.rigs`.
    hull: usize,
    cockpit: usize,
    gear: Vec<Hang>,
    /// The hull's seat (the outside chair's) and the cockpit's (the inside chair's).
    hull_seat: Seat,
    cockpit_seat: Seat,
    /// The gear legs' machines and the hull's gear doors' machines (indices in `ship.machines`).
    legs: Vec<usize>,
    doors: Vec<usize>,
}

/// A node posed, in Elite's model space.
struct Placed<'a> {
    name: &'a str,
    model: Option<&'a str>,
    group: Group,
    world: DMat4,
}

impl ShipRig {
    /// Every node posed with `values`, in Elite's model space, in the nodes' order: the hull rig's, each gear rig's, the
    /// cockpit rig's, then the two chairs.
    fn walk(&self, values: &Values) -> Result<Vec<Placed<'_>>, RigError> {
        let rigs = &self.ship.rigs;
        let hull = elite_rig::pose(&rigs[self.hull].root, &self.bones, values, DMat4::IDENTITY)?;
        let hull_seat = elite_rig::attach_base(&hull, &self.bones, &self.hull_seat.node, &self.hull_seat.bone)?;
        let mut gears = Vec::new();
        for g in &self.gear {
            let base = elite_rig::attach_base(&hull, &self.bones, &g.node, &g.bone)?;
            gears.push(elite_rig::pose(&rigs[g.rig].root, &self.bones, values, base)?);
        }
        let base = elite_rig::attach_base(&hull, &self.bones, &rigs[self.hull].root.name, COCKPIT_BONE)?;
        let cockpit = elite_rig::pose(&rigs[self.cockpit].root, &self.bones, values, base)?;
        let cockpit_seat = elite_rig::attach_base(&cockpit, &self.bones, &self.cockpit_seat.node, &self.cockpit_seat.bone)?;
        fn placed<'a, 'p>(poses: &'p [NodePose<'a>], group: Group) -> impl Iterator<Item = Placed<'a>> + 'p {
            poses.iter().map(move |p| Placed { name: p.node.name.as_str(), model: p.node.model.as_deref(), group, world: p.world })
        }
        let mut out: Vec<Placed<'_>> = placed(&hull, Group::Hull).collect();
        for (i, g) in gears.iter().enumerate() {
            out.extend(placed(g, Group::Gear(i)));
        }
        out.extend(placed(&cockpit, Group::Cockpit));
        out.push(Placed { name: &self.hull_seat.chair, model: Some(&self.hull_seat.chair), group: Group::OutsideChair, world: hull_seat });
        out.push(Placed { name: &self.cockpit_seat.chair, model: Some(&self.cockpit_seat.chair), group: Group::InsideChair, world: cockpit_seat });
        Ok(out)
    }

    /// Every node's matrix in the ship's space with `values` (in `nodes`' order).
    pub fn pose(&self, values: &Values) -> Result<Vec<Mat4>, RigError> {
        Ok(self.walk(values)?.iter().map(|p| self.space.matrix(p.world)).collect())
    }

    /// Whose machine `ship.machines[i]` is: the hull rig's, a gear rig's or the cockpit rig's (by its owner and file).
    pub fn machine_owner(&self, i: usize) -> MachineOwner {
        let Some(m) = self.ship.machines.get(i) else { return MachineOwner::Other };
        let of = |rig: usize| self.ship.rigs.get(rig).is_some_and(|r| r.name.eq_ignore_ascii_case(&m.owner) && r.file == m.file);
        if of(self.hull) {
            MachineOwner::Hull
        } else if of(self.cockpit) {
            MachineOwner::Cockpit
        } else if self.gear.iter().any(|g| of(g.rig)) {
            MachineOwner::Gear
        } else {
            MachineOwner::Other
        }
    }

    /// The nodes' group of machine `i`'s own rig (None for another file's machine).
    fn machine_group(&self, i: usize) -> Option<Group> {
        let m = self.ship.machines.get(i)?;
        match self.machine_owner(i) {
            MachineOwner::Hull => Some(Group::Hull),
            MachineOwner::Cockpit => Some(Group::Cockpit),
            MachineOwner::Gear => self.gear.iter().position(|g| self.ship.rigs[g.rig].name.eq_ignore_ascii_case(&m.owner)).map(Group::Gear),
            MachineOwner::Other => None,
        }
    }

    /// The node of machine `i`'s own rig with this name (a `KModelState`'s node names), any case.
    pub fn machine_node(&self, i: usize, name: &str) -> Option<usize> {
        let group = self.machine_group(i)?;
        self.nodes.iter().position(|n| n.group == group && n.name.eq_ignore_ascii_case(name))
    }

    /// The nodes a sound cue's emitter bones are on (what `sound::ShipSound::post_cue` places the emitter by): for each bone
    /// (its name without the `!`), the first node of machine `i`'s own rig whose model has that bone, else the rig's first
    /// node.
    pub fn cue_nodes(&self, i: usize, bones: &[String]) -> Vec<usize> {
        let (Some(group), Ok(placed)) = (self.machine_group(i), self.walk(&Values::new())) else { return Vec::new() };
        let in_rig: Vec<usize> = (0..placed.len()).filter(|&n| placed[n].group == group).collect();
        bones
            .iter()
            .filter_map(|b| {
                let bone = format!("!{b}");
                in_rig.iter().copied().find(|&n| placed[n].model.is_some_and(|m| self.bones.get(m, &bone).is_some())).or(in_rig.first().copied())
            })
            .collect()
    }

    /// Is `ship.machines[i]` a gear leg's machine?
    pub fn is_leg(&self, i: usize) -> bool {
        self.legs.contains(&i)
    }

    /// The gear rig (its index in the ship item's gear, `Group::Gear`'s) a leg's machine belongs to.
    pub fn gear_of_leg(&self, i: usize) -> Option<usize> {
        match self.machine_group(i) {
            Some(Group::Gear(g)) if self.is_leg(i) => Some(g),
            _ => None,
        }
    }

    /// Each `Node!Bone` of `bones` (the light activities' form: the node's name, or its model's, then the bone without its
    /// `!`) posed with `values`, in the ship's space: its place and its local +Z axis. None for a name no node or bone has.
    pub fn bone_frames(&self, values: &Values, bones: &[String]) -> Result<Vec<Option<(Vec3, Vec3)>>, RigError> {
        let placed = self.walk(values)?;
        Ok(bones
            .iter()
            .map(|b| {
                let (node, bone) = b.split_once('!')?;
                let p = placed.iter().find(|p| p.name.eq_ignore_ascii_case(node) || p.model.is_some_and(|m| m.eq_ignore_ascii_case(node)))?;
                let frame = p.world * self.bones.get(p.model?, &format!("!{bone}"))?;
                Some((self.space.point(frame.w_axis.truncate()), self.space.direction(frame.z_axis.truncate().normalize())))
            })
            .collect())
    }

    /// The cockpit rig's root node (the cockpit model's: its bones' frames are the cockpit model's space), in `nodes`.
    pub fn cockpit_node(&self) -> Option<usize> {
        let root = &self.ship.rigs[self.cockpit].root.name;
        self.nodes.iter().position(|n| n.group == Group::Cockpit && n.name.eq_ignore_ascii_case(root))
    }

    /// A still pose: every machine in its initial state's pose, except the gear legs `Down` with the hull's gear doors
    /// `DoorsOpen`, or `Up` with them `DoorsClosed` (each state's own values, `elite_rig::state_pose`).
    pub fn static_pose(&self, gear_down: bool) -> Values {
        let mut v = Values::new();
        for m in &self.ship.machines {
            if let Some(s) = m.state(&m.initial) {
                v.extend(elite_rig::state_pose(s));
            }
        }
        let (leg, doors) = if gear_down { (LEG_DOWN, DOORS_OPEN) } else { (LEG_UP, DOORS_CLOSED) };
        for (machines, state) in [(&self.legs, leg), (&self.doors, doors)] {
            for s in machines.iter().filter_map(|&i| self.ship.machines[i].state(state)) {
                v.extend(elite_rig::state_pose(s));
            }
        }
        v
    }
}

/// The parked ship's solid shape (the ship's space, the gear down).
#[derive(Clone, Debug)]
pub struct ShipShape {
    /// Convex pieces for the sandbox's world: each outside opaque draw's hull's points.
    pub hulls: Vec<Vec<Vec3>>,
    pub lo: Vec3,
    pub hi: Vec3,
    /// Each landing gear leg's ground contact point (its lowest point; y 0 at rest), in the gear's order.
    pub feet: Vec<Vec3>,
    /// Each leg's foot on its own node, in the gear's order: the node holding the leg's lowest drawn point, and the foot
    /// (`feet`) in that node's frame at rest, so the node's matrix in any pose carries it (`parts::Parts::feet`).
    pub foot_nodes: Vec<(usize, Vec3)>,
    /// The hull's lowest point in each cell of a 3 x 3 grid over its plan, the gear up and its doors closed (what touches
    /// the ground with the gear up or the ship tilted).
    pub belly: Vec<Vec3>,
    /// For the air's drag only (ours, grade F: Elite has no thick air): the triangles of the convex hull round the hull
    /// rig's drawn vertices with the gear up, whose area vectors give the ship's outline along a direction
    /// (`elite_sim::air::outline_area`, half the sum of |area . direction|). That sum is the outline only for one closed
    /// convex shell: over every drawn triangle (overlapping layers, inner faces, the gear, open doors) it counts the same
    /// outline many times over.
    pub drag_hull: Vec<[Vec3; 3]>,
}

/// The mod's files, read and checked at its first spawn (kept until quit).
#[derive(Resource, Clone)]
pub struct SidewinderAssets {
    /// Elite's default keyboard and mouse controls and its default pad's (copies of its `.binds` files).
    pub keys: Arc<Binds>,
    pub pad: Arc<Binds>,
    pub rig: Arc<ShipRig>,
    /// The ship's solid pieces, box, feet and belly.
    pub shape: Arc<ShipShape>,
    /// The pilot's eyes in the ship's space.
    pub eye: [f32; 3],
    /// The guns' mounts in the ship's space (port, starboard).
    pub guns: [[f32; 3]; 2],
    /// The pilot chair's clips, motion graph and skeleton (the seat's chair model's bones), checked (`chair::load`).
    pub chair: Arc<super::chair::ChairData>,
}

/// What the first spawn hands `draw` once, then drops: a mesh per drawn draw of each model (`ShipRig::models`' order), the
/// textures the drawn materials read, by (file, sRGB), and every drawn material resolved (`material::resolve`), by key.
#[derive(Resource)]
pub struct ShipBuild {
    pub meshes: Vec<Vec<Mesh>>,
    pub images: HashMap<(String, bool), Image>,
    pub materials: HashMap<String, material::Resolved>,
}

/// What `load` gives.
pub struct Loaded {
    pub assets: SidewinderAssets,
    pub build: ShipBuild,
}

/// A refusal: the file, and why.
type Fault = (String, String);

/// A model's drawn draws decoded, in the mod's axes.
struct Decoded {
    vertices: Vec<DecodedVertex>,
    indices: Vec<u16>,
    /// Part of the ship's solid shape: not the decals and the stamp, which lie on the hull (their hull would span the ship).
    solid: bool,
    /// Glass: its mesh carries the vertex's colour (`material::ATTRIBUTE_COLOUR`).
    glass: bool,
    /// A skinned model's: each vertex's bone (`Edm::vertex_bone`), carried as `material::ATTRIBUTE_BONE`.
    bones: Option<Vec<u32>>,
}

fn model_file(name: &str) -> String {
    format!("models/{name}.edm")
}

/// A model's drawn draws (`material::material_drawn`): their materials' keys and their vertices decoded in the mod's axes;
/// for a `skinned` model each vertex's bone too (its heaviest, `Edm::vertex_bone`: the seat's chair hangs every vertex on
/// one bone).
fn decode_model(edm: &Edm, materials: &Materials, paint: &Paint, name: &str, skinned: bool) -> Result<(ShipModel, Vec<Decoded>), Fault> {
    let bad = |why: String| (model_file(name), why);
    if edm.vertex_decode != VertexDecode::Elite {
        return Err(bad("not one of Elite's models (its vertices aren't Elite's)".to_string()));
    }
    if skinned && edm.parts.len() > material::PALETTE_BONES {
        return Err(bad(format!("{} bones, more than the palette's {}", edm.parts.len(), material::PALETTE_BONES)));
    }
    let (mut draws, mut decoded) = (Vec::new(), Vec::new());
    for d in &edm.draws {
        let mat_name = &edm.materials.get(d.material as usize).ok_or_else(|| bad(format!("a draw names material {}, which it hasn't", d.material)))?.name;
        let mat = materials.get(mat_name).ok_or_else(|| (MATERIALS_FILE.to_string(), format!("{name} names material {mat_name}, which isn't in it")))?;
        if !material::material_drawn(mat_name, mat, paint) {
            continue;
        }
        let mut vertices: Vec<DecodedVertex> = d.vertices.iter().map(|v| v.decode()).collect();
        let mut indices = d.indices.clone();
        if indices.len() % 3 != 0 || indices.iter().any(|&i| i as usize >= vertices.len()) {
            return Err(bad(format!("a draw of material {mat_name} has a broken triangle list")));
        }
        to_mod_space(&mut vertices, &mut indices);
        draws.push(mat_name.to_lowercase());
        let family = Family::of(mat.shader.as_deref().unwrap_or(""));
        let solid = !matches!(family, Family::Overlay | Family::Stamp);
        let bones = if skinned {
            // (The glass has its own vertex stage, which doesn't skin.)
            if family.is_glass() {
                return Err(bad(format!("its glass ({mat_name}) can't be skinned")));
            }
            Some(d.vertices.iter().map(|v| edm.vertex_bone(v).map(|b| b as u32).ok_or_else(|| bad(format!("a vertex of material {mat_name} names a bone it hasn't")))).collect::<Result<Vec<_>, Fault>>()?)
        } else {
            None
        };
        decoded.push(Decoded { vertices, indices, solid, glass: family.is_glass(), bones });
    }
    Ok((ShipModel { draws, skinned }, decoded))
}

/// A `Node!Bone` seat (the item's facts: the bone without its `!`).
fn seat(fact: &str, chair: &str) -> Result<Seat, Fault> {
    let (node, bone) = fact.split_once('!').ok_or_else(|| (SHIP_FILE.to_string(), format!("the seat {fact} isn't Node!Bone")))?;
    Ok(Seat { node: node.to_string(), bone: format!("!{bone}"), chair: chair.to_string() })
}

/// The ship's rigs, machines and seats from `ship.json`, every model they hang read (bones in the bone table), and the
/// nodes listed. The rest height isn't known yet (`space.lift` 0).
fn assemble(read: &dyn Fn(&str) -> Result<Vec<u8>, String>, ship: Ship, materials: Materials) -> Result<(ShipRig, Vec<Vec<Decoded>>), Fault> {
    let refuse = |why: String| (SHIP_FILE.to_string(), why);
    let item = &ship.item;
    let root_is = |r: &Rig, model: &str| r.root.model.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(model));
    let hull = ship.rigs.iter().position(|r| root_is(r, &item.hull_model)).ok_or_else(|| refuse(format!("no rig hangs the hull {}", item.hull_model)))?;
    let cockpit = ship.rigs.iter().position(|r| r.file == COCKPIT_RIG_FILE && root_is(r, &item.cockpit_model)).ok_or_else(|| refuse(format!("no rig of {COCKPIT_RIG_FILE} hangs the cockpit {}", item.cockpit_model)))?;
    let gear = item
        .gear
        .iter()
        .map(|g| {
            let rig = ship.rigs.iter().position(|r| r.name.eq_ignore_ascii_case(&g.rig)).ok_or_else(|| refuse(format!("no gear rig {}", g.rig)))?;
            Ok(Hang { rig, node: g.node.clone(), bone: g.bone.clone() })
        })
        .collect::<Result<Vec<_>, Fault>>()?;
    if gear.is_empty() {
        return Err(refuse("the ship has no landing gear to stand on".to_string()));
    }
    // The two seats, paired with the two chair models in order; one is the hull rig's root, the other the cockpit's.
    if item.seats.len() != item.chair_models.len() {
        return Err(refuse(format!("{} seats but {} chairs", item.seats.len(), item.chair_models.len())));
    }
    let seats = item.seats.iter().zip(&item.chair_models).map(|(s, c)| seat(s, c)).collect::<Result<Vec<_>, Fault>>()?;
    let seat_of = |rig: usize| seats.iter().find(|s| s.node.eq_ignore_ascii_case(&ship.rigs[rig].root.name)).cloned().ok_or_else(|| refuse(format!("no seat on {}", ship.rigs[rig].root.name)));
    let (hull_seat, cockpit_seat) = (seat_of(hull)?, seat_of(cockpit)?);
    // The gear legs' machines (owned by a gear rig, with Down and Up) and the doors' (the hull's, DoorsOpen and DoorsClosed).
    let has = |m: &Machine, a: &str, b: &str| m.state(a).is_some() && m.state(b).is_some();
    let owned_by = |m: &Machine, rig: usize| m.owner.eq_ignore_ascii_case(&ship.rigs[rig].name);
    let legs: Vec<usize> = (0..ship.machines.len()).filter(|&i| gear.iter().any(|g| owned_by(&ship.machines[i], g.rig)) && has(&ship.machines[i], LEG_DOWN, LEG_UP)).collect();
    let doors: Vec<usize> = (0..ship.machines.len()).filter(|&i| owned_by(&ship.machines[i], hull) && has(&ship.machines[i], DOORS_OPEN, DOORS_CLOSED)).collect();
    if legs.len() != gear.len() || doors.len() != gear.len() {
        return Err(refuse(format!("{} gear legs but {} leg machines and {} door machines", gear.len(), legs.len(), doors.len())));
    }
    let mut rig = ShipRig {
        space: ShipSpace { lift: 0.0 },
        ship,
        materials,
        models: Vec::new(),
        nodes: Vec::new(),
        bones: BoneTable::default(),
        hull,
        cockpit,
        gear,
        hull_seat,
        cockpit_seat,
        legs,
        doors,
    };
    // Every model the rigs hang and the chairs, read once (each one's bones go in the table, drawn or not: a node hangs
    // from its parent's bones).
    let mut wanted: Vec<String> = rig.ship.rigs.iter().enumerate().filter(|(i, _)| *i == hull || *i == cockpit || rig.gear.iter().any(|g| g.rig == *i)).flat_map(|(_, r)| r.root.walk()).filter_map(|n| n.model.clone()).collect();
    wanted.extend([rig.hull_seat.chair.clone(), rig.cockpit_seat.chair.clone()]);
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut decoded = Vec::new();
    for name in wanted {
        if index.contains_key(&name) {
            continue;
        }
        let file = model_file(&name);
        let edm = parse_edm(&read(&file).map_err(|e| (file.clone(), e))?).map_err(|e| (file.clone(), e.to_string()))?;
        for p in &edm.parts {
            let inv = p.inverse_bind.ok_or_else(|| (file.clone(), format!("bone {} has no inverse bind", p.name)))?;
            rig.bones.insert(&name, &p.name, elite_rig::elite_matrix(&inv.map(|x| x as f64)).inverse());
        }
        if material::model_drawn(&name) {
            // (The seat's chair is the one the pilot chair's animation poses: `chair.rs`.)
            let skinned = name.eq_ignore_ascii_case(&rig.cockpit_seat.chair);
            let (model, draws) = decode_model(&edm, &rig.materials, &rig.ship.paint, &name, skinned)?;
            index.insert(name, rig.models.len());
            rig.models.push(model);
            decoded.push(draws);
        } else {
            index.insert(name, usize::MAX);
        }
    }
    let nodes: Vec<ShipNode> = rig
        .walk(&Values::new())
        .map_err(|e| refuse(e.to_string()))?
        .iter()
        .map(|p| ShipNode { name: p.name.to_string(), model: p.model.and_then(|m| index.get(m).copied()).filter(|&i| i != usize::MAX), group: p.group })
        .collect();
    rig.nodes = nodes;
    Ok((rig, decoded))
}

/// Every outside node's drawn vertices in the ship's space with the matrices `pose` (`nodes`' order), with the node and its
/// group.
fn outside_points<'a>(rig: &'a ShipRig, decoded: &'a [Vec<Decoded>], pose: &'a [Mat4]) -> impl Iterator<Item = (usize, Group, Vec<Vec3>)> + 'a {
    rig.nodes.iter().zip(pose).enumerate().filter(|(_, (n, _))| !n.group.inside()).filter_map(|(i, (n, m))| Some((i, n.group, n.model?, *m))).flat_map(move |(i, group, model, m)| {
        decoded[model].iter().filter(|d| d.solid).map(move |d| (i, group, d.vertices.iter().map(|v| m.transform_point3(Vec3::from_array(v.position))).collect()))
    })
}

/// The rest height (the gear rigs posed Down, their lowest point on the ground), then the shape, feet and belly.
fn shape(rig: &mut ShipRig, decoded: &[Vec<Decoded>]) -> Result<ShipShape, Fault> {
    let refuse = |e: RigError| (SHIP_FILE.to_string(), e.to_string());
    let down_values = rig.static_pose(true);
    rig.space = ShipSpace { lift: 0.0 };
    let lowest = outside_points(rig, decoded, &rig.pose(&down_values).map_err(refuse)?)
        .filter(|(_, g, _)| matches!(g, Group::Gear(_)))
        .flat_map(|(_, _, pts)| pts.into_iter().map(|p| p.y))
        .fold(f32::MAX, f32::min);
    if lowest == f32::MAX {
        return Err((SHIP_FILE.to_string(), "the landing gear has nothing drawn to stand on".to_string()));
    }
    rig.space = ShipSpace { lift: -lowest as f64 };
    let down = rig.pose(&down_values).map_err(refuse)?;
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut hulls = Vec::new();
    let mut legs: Vec<Vec<Vec3>> = vec![Vec::new(); rig.gear.len()];
    // Each leg's lowest point's node (the foot's) and height.
    let mut lowest_node: Vec<(usize, f32)> = vec![(usize::MAX, f32::MAX); rig.gear.len()];
    for (node, group, pts) in outside_points(rig, decoded, &down) {
        for p in &pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        if let Group::Gear(i) = group {
            legs[i].extend(&pts);
            if let Some(y) = pts.iter().map(|p| p.y).reduce(f32::min).filter(|&y| y < lowest_node[i].1) {
                lowest_node[i] = (node, y);
            }
        }
        // (The piece is its hull's points: the sandbox's world makes the hull again from them each time it adds it.)
        if let Ok((corners, _)) = rapier3d::parry::transformation::try_convex_hull(&pts) {
            hulls.push(corners);
        }
    }
    let feet: Vec<Vec3> = legs
        .iter()
        .map(|pts| {
            let low = pts.iter().map(|p| p.y).fold(f32::MAX, f32::min);
            let sole: Vec<&Vec3> = pts.iter().filter(|p| p.y < low + FOOT_BAND_M).collect();
            let mid = sole.iter().fold(Vec3::ZERO, |a, p| a + **p) / sole.len().max(1) as f32;
            Vec3::new(mid.x, low, mid.z)
        })
        .collect();
    let foot_nodes = feet
        .iter()
        .zip(&lowest_node)
        .map(|(foot, &(node, _))| down.get(node).map(|m| (node, m.inverse().transform_point3(*foot))))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| (SHIP_FILE.to_string(), "a landing gear leg has nothing drawn to stand on".to_string()))?;
    // The belly: the hull's lowest point (not the gear's) in each cell of a 3 x 3 grid over the plan, the gear up.
    let up = rig.pose(&rig.static_pose(false)).map_err(refuse)?;
    let mut cells: [Option<Vec3>; 9] = [None; 9];
    let size = (hi - lo).max(Vec3::splat(1e-3));
    let hull_up: Vec<Vec3> = outside_points(rig, decoded, &up).filter(|(_, g, _)| *g == Group::Hull).flat_map(|(_, _, pts)| pts).collect();
    for &p in &hull_up {
        let i = (((p.x - lo.x) / size.x * 3.0) as usize).min(2) + 3 * (((p.z - lo.z) / size.z * 3.0) as usize).min(2);
        if cells[i].is_none_or(|c| p.y < c.y) {
            cells[i] = Some(p);
        }
    }
    // (The drag: the hull rig's vertices with the gear up, its doors closed.)
    let drag_hull = convex_shell(&hull_up).ok_or_else(|| (SHIP_FILE.to_string(), "the hull has no volume to meet the air with".to_string()))?;
    Ok(ShipShape { hulls, lo, hi, feet, foot_nodes, belly: cells.into_iter().flatten().collect(), drag_hull })
}

/// The triangles of the convex hull round `points` (None when they have no volume).
fn convex_shell(points: &[Vec3]) -> Option<Vec<[Vec3; 3]>> {
    let (corners, faces) = rapier3d::parry::transformation::try_convex_hull(points).ok()?;
    Some(faces.iter().map(|f| f.map(|i| corners[i as usize])).collect())
}

/// A bone's place in Elite's model space: `node` posed, its model's bone `bone`.
fn bone_point(rig: &ShipRig, node: &Placed, bone: &str) -> Result<DVec3, Fault> {
    let model = node.model.unwrap_or("");
    let bind = rig.bones.get(model, bone).ok_or_else(|| (SHIP_FILE.to_string(), format!("no bone {bone} in {model}")))?;
    Ok((node.world * bind).w_axis.truncate())
}

/// The pilot's eye (`EYE_BONE` of the cockpit rig's root model) and the guns' mounts, in the ship's space with the gear down.
fn eye_and_guns(rig: &ShipRig) -> Result<([f32; 3], [[f32; 3]; 2]), Fault> {
    let refuse = |why: String| (SHIP_FILE.to_string(), why);
    let placed = rig.walk(&rig.static_pose(true)).map_err(|e| refuse(e.to_string()))?;
    let cockpit_model = &rig.ship.item.cockpit_model;
    let cockpit = placed
        .iter()
        .find(|p| p.group == Group::Cockpit && p.model.is_some_and(|m| m.eq_ignore_ascii_case(cockpit_model)))
        .ok_or_else(|| refuse(format!("no node hangs the cockpit {cockpit_model}")))?;
    let eye = rig.space.point(bone_point(rig, cockpit, EYE_BONE)?).to_array();
    let hull = &placed[0];
    let mut guns = [[0.0; 3]; 2];
    for (gun, &k) in guns.iter_mut().zip(&GUN_HARDPOINTS) {
        let bone = rig.ship.item.hardpoints.get(k).ok_or_else(|| refuse(format!("the item has no hardpoint {k}")))?;
        *gun = rig.space.point(bone_point(rig, hull, bone)?).to_array();
    }
    Ok((eye, guns))
}

/// A draw's mesh: position, normal, tangent (its sign in w), UV0, UV1, UV2, for the glass the vertex's colour, for a skinned
/// model the vertex's bone.
fn mesh(d: &Decoded) -> Mesh {
    let v = &d.vertices;
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, v.iter().map(|v| v.position).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, v.iter().map(|v| v.normal).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, v.iter().map(|v| [v.tangent[0], v.tangent[1], v.tangent[2], v.tangent_sign]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, v.iter().map(|v| v.uv[0]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, v.iter().map(|v| v.uv[1]).collect::<Vec<_>>());
    mesh.insert_attribute(ATTRIBUTE_UV_2, v.iter().map(|v| v.uv[2]).collect::<Vec<_>>());
    if d.glass {
        mesh.insert_attribute(ATTRIBUTE_COLOUR, VertexAttributeValues::Unorm8x4(v.iter().map(|v| v.colour).collect()));
    }
    if let Some(bones) = &d.bones {
        mesh.insert_attribute(ATTRIBUTE_BONE, VertexAttributeValues::Uint32(bones.clone()));
    }
    mesh.insert_indices(Indices::U16(d.indices.clone()));
    mesh
}

/// The drawn materials, by key, each once.
fn drawn_materials(rig: &ShipRig) -> Vec<(&str, &se_core::elite_ship::Material)> {
    let mut keys: Vec<&str> = rig.models.iter().flat_map(|m| m.draws.iter().map(String::as_str)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter().filter_map(|k| Some((k, rig.materials.get(k)?))).collect()
}

/// Every texture the drawn materials' families read (`material::texture_files`: the paint job's mask and the IES profile
/// among them), each read once. A drawn material lacking a slot its family reads is refused here.
fn textures(read: &dyn Fn(&str) -> Result<Vec<u8>, String>, rig: &ShipRig) -> Result<HashMap<(String, bool), Image>, Fault> {
    let mut images = HashMap::new();
    let files = material::texture_files(drawn_materials(rig).into_iter(), &rig.ship.paint).map_err(|e| (MATERIALS_FILE.to_string(), e))?;
    for (file, srgb) in files {
        let bytes = read(&file).map_err(|e| (file.clone(), e))?;
        let info = se_core::dds::parse_dds(&bytes).map_err(|e| (file.clone(), e.to_string()))?;
        let mut image = crate::mr_assets::dds_image(&bytes, &info, srgb).map_err(|e| (file.clone(), e))?;
        if file == material::IES_FILE {
            material::ies_sampler(&mut image);
        }
        images.insert((file, srgb), image);
    }
    Ok(images)
}

/// Reads the mod's files through `read` (the mods menu's checked reader: a refusal names the file and why).
pub fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<Loaded, Fault> {
    let json = |f: &str| read(f).map_err(|e| (f.to_string(), e));
    let ship = parse_ship(&json(SHIP_FILE)?).map_err(|e| (SHIP_FILE.to_string(), e.to_string()))?;
    let materials = parse_materials(&json(MATERIALS_FILE)?).map_err(|e| (MATERIALS_FILE.to_string(), e.to_string()))?;
    let binds = |f: &str| -> Result<Binds, Fault> { Binds::parse(&String::from_utf8_lossy(&json(f)?)).map_err(|e| (f.to_string(), e.to_string())) };
    let (mut rig, decoded) = assemble(read, ship, materials)?;
    let shape = shape(&mut rig, &decoded)?;
    let (eye, guns) = eye_and_guns(&rig)?;
    let images = textures(read, &rig)?;
    // (Every drawn material's slots and attributes, now: one missing refuses the files, so the Mods row greys.)
    let paint = &rig.ship.paint;
    let mask_size = images.get(&(paint.texture_file.clone(), false)).map_or(UVec2::ONE, |img| img.size());
    let materials = drawn_materials(&rig)
        .into_iter()
        .map(|(key, m)| Ok((key.to_string(), material::resolve(key, m, paint, mask_size).map_err(|e| (MATERIALS_FILE.to_string(), e))?)))
        .collect::<Result<HashMap<_, _>, Fault>>()?;
    let meshes = decoded.iter().map(|draws| draws.iter().map(mesh).collect()).collect();
    Ok(Loaded {
        assets: SidewinderAssets { keys: Arc::new(binds(KEYS_FILE)?), pad: Arc::new(binds(PAD_FILE)?), chair: super::chair::load(read, &model_file(&rig.cockpit_seat.chair))?, rig: Arc::new(rig), shape: Arc::new(shape), eye, guns },
        build: ShipBuild { meshes, images, materials },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    /// A point and a matrix there and back; the starboard nav light (Elite x +10.4, behind the middle) on the ship's right
    /// and rear, the headlights (Elite z +5.9) towards the ship's nose (-Z); a node's matrix applied to a vertex in the mod's
    /// axes lands where the point conversion puts Elite's vertex.
    #[test]
    fn spaces_convert() {
        let s = ShipSpace { lift: 4.718 };
        let starboard = s.point(DVec3::new(10.4, 0.083, -6.438));
        assert!(close(starboard, Vec3::new(10.4, 4.801, 6.438)), "{starboard}");
        assert!(s.point(DVec3::new(3.0, 0.0, 5.9)).z < 0.0);
        let p = Vec3::new(1.5, -2.0, 3.25);
        assert!((s.elite_point(s.point(p.as_dvec3())) - p.as_dvec3()).length() < 1e-6);
        let m = DMat4::from_rotation_translation(bevy::math::DQuat::from_euler(bevy::math::EulerRot::YXZ, 0.4, -0.3, 1.1), DVec3::new(-4.9, -2.0, -0.8));
        let back = s.elite_matrix(s.matrix(m));
        assert!(back.abs_diff_eq(m, 1e-5), "{back} {m}");
        let v = DVec3::new(0.3, -0.7, 1.2);
        let local_mod = Vec3::new(v.x as f32, v.y as f32, -v.z as f32);
        assert!(close(s.matrix(m).transform_point3(local_mod), s.point(m.transform_point3(v))));
        // (A turn stays a turn: the mirror on both sides keeps the determinant.)
        assert!((s.matrix(m).determinant() - 1.0).abs() < 1e-5);
    }

    /// The ship's outline along `dir` from its drag hull, as the flight works it out (`pilot::fly`).
    fn outline(tris: &[[Vec3; 3]], dir: Vec3) -> f64 {
        let at = |p: Vec3| glam::DVec3::from_array(p.as_dvec3().to_array());
        let areas: Vec<glam::DVec3> = tris.iter().map(|t| elite_sim::air::area_vector(at(t[0]), at(t[1]), at(t[2]))).collect();
        elite_sim::air::outline_area(&areas, glam::DVec3::from_array(dir.as_dvec3().to_array()))
    }

    /// A 2 x 3 x 4 m box's convex shell (its corners and a point inside): its outline along each axis is the face across
    /// it (12, 8 and 6 m^2), and half the sum of |area . direction| over one closed convex shell is that outline.
    #[test]
    fn outline_of_a_box() {
        let mut pts: Vec<Vec3> = (0..8).map(|i| Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.5 } else { 1.5 }, if i & 4 == 0 { -2.0 } else { 2.0 })).collect();
        pts.push(Vec3::new(0.2, -0.3, 0.5));
        let shell = convex_shell(&pts).unwrap();
        for (dir, want) in [(Vec3::X, 12.0), (Vec3::Y, 8.0), (Vec3::Z, 6.0)] {
            assert!((outline(&shell, dir) - want).abs() < 1e-4, "{dir}: {}", outline(&shell, dir));
        }
    }

    fn read_mod(rel: &str) -> Result<Vec<u8>, String> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string())
    }

    /// The mod's files with one drawn material's slot or attribute taken out of `materials.json` (in memory: the folder is
    /// untouched) are refused at load, the error naming the material and what it lacks (so the Mods row greys and `draw`
    /// never meets it).
    #[test]
    fn a_missing_slot_or_attribute_refuses_the_files() {
        let without = |material: &str, list: &str, name_key: &str, name: &str| {
            let mut json: serde_json::Value = serde_json::from_slice(&read_mod(MATERIALS_FILE).unwrap()).unwrap();
            let items = json["materials"][material][list].as_array_mut().unwrap();
            let before = items.len();
            items.retain(|t| t[name_key] != name);
            assert_eq!(items.len(), before - 1, "{material} has one {name}");
            let bytes = serde_json::to_vec(&json).unwrap();
            let read = move |rel: &str| if rel == MATERIALS_FILE { Ok(bytes.clone()) } else { read_mod(rel) };
            match load(&read) {
                Ok(_) => panic!("{material} without {name} was accepted"),
                Err((file, why)) => (file, why),
            }
        };
        for (material, list, key, name) in [("sidewinder_tile_floor", "textures", "slot", "pAOMap"), ("cockpit_chair", "attrs", "name", "pOverrideBaseColour"), ("cockpit_glass_int", "attrs", "name", "pGlassThickness")] {
            let (file, why) = without(material, list, key, name);
            assert_eq!(file, MATERIALS_FILE);
            assert!(why.contains(material) && why.contains(name), "{why}");
        }
    }

    /// The shipped ship: only the seat's chair is skinned; its meshes' bones are `Edm::vertex_bone` of each vertex of its
    /// drawn draws, in order, under the palette's length (which is its bone count); no other mesh carries bones.
    #[test]
    fn only_the_seats_chair_carries_its_bones() {
        let loaded = load(&read_mod).unwrap_or_else(|(f, why)| panic!("{f}: {why}"));
        let rig = &loaded.assets.rig;
        let skinned: Vec<usize> = (0..rig.models.len()).filter(|&m| rig.models[m].skinned).collect();
        assert_eq!(skinned.len(), 1);
        let chair = &rig.cockpit_seat.chair;
        assert!(rig.nodes.iter().any(|n| n.model == Some(skinned[0]) && n.name.eq_ignore_ascii_case(chair) && n.group == Group::InsideChair));
        let edm = parse_edm(&read_mod(&model_file(chair)).unwrap()).unwrap();
        assert_eq!(edm.parts.len(), material::PALETTE_BONES);
        let drawn: Vec<_> = edm
            .draws
            .iter()
            .filter(|d| {
                let name = &edm.materials[d.material as usize].name;
                material::material_drawn(name, rig.materials.get(name).unwrap(), &rig.ship.paint)
            })
            .collect();
        let meshes = &loaded.build.meshes[skinned[0]];
        assert_eq!(drawn.len(), meshes.len());
        let mut count = 0;
        for (d, mesh) in drawn.iter().zip(meshes) {
            let Some(VertexAttributeValues::Uint32(bones)) = mesh.attribute(ATTRIBUTE_BONE) else { panic!("a chair mesh without bones") };
            let want: Vec<u32> = d.vertices.iter().map(|v| edm.vertex_bone(v).unwrap() as u32).collect();
            assert_eq!(bones, &want);
            assert!(bones.iter().all(|&b| (b as usize) < material::PALETTE_BONES));
            count += bones.len();
        }
        for (m, meshes) in loaded.build.meshes.iter().enumerate().filter(|(m, _)| *m != skinned[0]) {
            assert!(meshes.iter().all(|mesh| mesh.attribute(ATTRIBUTE_BONE).is_none()), "model {m} carries bones");
        }
        println!("the seat's chair: {count} vertices with their bones in {} meshes", meshes.len());
    }

    /// The eye's source (the mod's shipped cockpit model, in git): the cockpit model has its own `EYE_BONE`, at the model's
    /// origin with no turn (so the eye looks along the nose), 1.610 m over and 0.129 m behind the cockpit's seat bone.
    #[test]
    fn the_eye_is_the_cockpits_camera_bone() {
        let ship = parse_ship(&read_mod(SHIP_FILE).unwrap()).unwrap();
        let file = model_file(&ship.item.cockpit_model);
        let edm = parse_edm(&read_mod(&file).unwrap()).unwrap();
        let bind = |name: &str| {
            let p = edm.parts.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("{file} has no bone {name}"));
            elite_rig::elite_matrix(&p.inverse_bind.unwrap().map(|x| x as f64)).inverse()
        };
        let camera = bind(EYE_BONE);
        assert!(camera.abs_diff_eq(DMat4::IDENTITY, 1e-5), "{EYE_BONE}: {camera}");
        // (Elite's axes: +Y up, +Z the nose; the chair's bone is 1.61 m under the eye and 0.13 m nearer the nose.)
        let over_seat = camera.w_axis.truncate() - bind("!Pilot_Chair").w_axis.truncate();
        assert!((over_seat - DVec3::new(0.0, 1.6098, -0.12855)).length() < 1e-3, "{over_seat}");
    }

    /// The mod's folder (ignored: reads the shipped files): the rest height the gear gives is 4.718 m, the three feet are
    /// on the ground, the box is about 21.4 x 14.2 m in plan (14.4 with the front toe), and the guns and eye are on the ship.
    #[test]
    #[ignore]
    fn the_real_ship_stands_on_its_gear() {
        let a = load(&read_mod).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets;
        let s = &a.shape;
        println!("rest height {:.4} m; feet {:?}; box {} .. {}; {} pieces, {} belly points, {} drag hull triangles", a.rig.space.lift, s.feet, s.lo, s.hi, s.hulls.len(), s.belly.len(), s.drag_hull.len());
        println!("eye {:?}; guns {:?}; {} nodes, {} models, {} draws drawn", a.eye, a.guns, a.rig.nodes.len(), a.rig.models.len(), a.rig.nodes.iter().filter_map(|n| n.model).map(|m| a.rig.models[m].draws.len()).sum::<usize>());
        assert!((a.rig.space.lift - 4.718).abs() < 0.001, "rest height {}", a.rig.space.lift);
        assert_eq!(s.feet.len(), 3);
        for f in &s.feet {
            assert!(f.y.abs() < 0.001, "a foot at {f}");
        }
        let plan = s.hi - s.lo;
        // (The hull is 21.39 x 14.21 m; with the gear down the front leg's toe stands 0.16 m ahead of the nose.)
        assert!((plan.x - 21.4).abs() < 0.1 && plan.z > 14.15 && plan.z < 14.45, "plan {plan}");
        assert!(a.guns[0][0] < 0.0 && a.guns[1][0] > 0.0 && a.guns[0][2] < 0.0, "guns {:?}", a.guns);
        assert!(a.eye[1] > 4.718 && a.eye[1] < s.hi.y, "eye {:?}", a.eye);
        // (The eye is the cockpit's origin, hung at the hull's `!Cockpit`: Elite (0.0001, 2.0043, -1.3465).)
        let cockpit_bone = a.rig.space.point(DVec3::new(9.54866e-5, 2.00428, -1.34654));
        assert!(Vec3::from_array(a.eye).distance(cockpit_bone) < 1e-3, "eye {:?} against {cockpit_bone}", a.eye);
        // The drag's outline (the stand-in's was 91 m^2 nose-on, 67 sideways): nose-on and sideways well under the plan,
        // from above the plan times the hull's fill (a trapezoid in plan: between half and all of the box).
        let (nose, side, above) = (outline(&s.drag_hull, Vec3::Z), outline(&s.drag_hull, Vec3::X), outline(&s.drag_hull, Vec3::Y));
        let plan_area = (plan.x * plan.z) as f64;
        println!("outline: {nose:.1} m^2 nose-on, {side:.1} sideways, {above:.1} from above (plan box {plan_area:.1}); {} drag hull triangles", s.drag_hull.len());
        assert!(nose > 40.0 && nose < 0.5 * plan_area, "nose-on {nose}");
        assert!(side > 30.0 && side < 0.5 * plan_area, "sideways {side}");
        assert!(above > 0.5 * plan_area && above < plan_area, "from above {above}");
    }

    /// The cockpit hung at the hull's `!Cockpit` bone (ignored: reads the shipped files): the cockpit model's copy of the
    /// hull's shell (its glows and bespoke parts) lies within 4 cm of the hull's own (each sampled vertex's nearest).
    #[test]
    #[ignore]
    fn the_cockpit_fits_the_hull() {
        let ship = parse_ship(&read_mod(SHIP_FILE).unwrap()).unwrap();
        let materials = parse_materials(&read_mod(MATERIALS_FILE).unwrap()).unwrap();
        let (mut rig, decoded) = assemble(&read_mod, ship, materials).unwrap();
        shape(&mut rig, &decoded).unwrap();
        let pose = rig.pose(&rig.static_pose(true)).unwrap();
        let points = |group: Group, material: &str| -> Vec<Vec3> {
            let nodes = rig.nodes.iter().zip(&pose).filter(|(n, _)| n.group == group).filter_map(|(n, m)| Some((n.model?, *m)));
            nodes.flat_map(|(model, m)| rig.models[model].draws.iter().zip(&decoded[model]).filter(|(k, _)| *k == material).flat_map(move |(_, d)| d.vertices.iter().map(move |v| m.transform_point3(Vec3::from_array(v.position))))).collect()
        };
        for material in ["elementglow", "sidewinder_bespokeparts"] {
            let (hull, cockpit) = (points(Group::Hull, material), points(Group::Cockpit, material));
            assert!(!hull.is_empty() && !cockpit.is_empty(), "{material}");
            let far = cockpit.iter().step_by((cockpit.len() / 300).max(1)).map(|p| hull.iter().map(|q| p.distance(*q)).fold(f32::MAX, f32::min)).fold(0.0f32, f32::max);
            println!("{material}: the cockpit's copy at most {far:.4} m from the hull's");
            assert!(far < 0.04, "{material}: {far} m");
        }
    }
}
