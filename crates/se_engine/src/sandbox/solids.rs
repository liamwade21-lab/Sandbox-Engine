//! Solid buildings and trees for the walker and the props.
//!
//! Where buildings and trees stand is known only to the GPU (the scatter's cull places them). The Viltrumite mod's probe
//! (`scatter::probe`) reads the buildings round a point back, 3-5 frames late; while he walks the sandbox asks it about the
//! place round him, and since 2026-11-11 the same list carries the trees too (`scatter.wesl` `probe_tree`, kind 4). Each
//! list is turned into pieces of the physics world: a building's slices as the hero's solids cut them
//! (`viltrumite::solid::shapes`: stacked boxes, turned where the tower is), a tree's trunk as a cylinder by its model's bark
//! (`ScatterLibrary::trunks`). Nothing is solid that isn't drawn: what leaves the list goes.
//!
//! - A building or a tree that holds him when it is first seen (he was set down inside it, or a finer ground moved a trunk
//!   into him) is let alone until he is out of it, as the hero's F2: no trace answers sensibly from inside a solid.
//! - The pieces live in the world's local frame: when the frame moves (`SandboxWorld::rebase`) the world drops them all
//!   and they are placed again from the newest list (`solids_cleared`).
//! - Off foot (free flight, a mod) nothing is asked and the pieces stay where they are, so props lean on what they leant on.
//! - Not solid: the domes (a shell: the hero's own), the flying lanes, street furniture and cars.

use std::collections::{HashMap, HashSet};

use bevy::math::{IVec2, Mat3, Quat, Vec3};

use crate::scatter::library::ScatterLibrary;
use crate::scatter::probe::ProbeBuilding;
use crate::viltrumite::solid::{Key as BuildingKey, M_DOME, Solid};

use super::world::{LocalFrame, SandboxWorld, SolidHandle, SolidShape, Surface};

/// Buildings whose bounding sphere comes within this of him are solid (m): the ground field's props are at most 60 m off.
pub const BUILDING_REACH_M: f32 = 45.0;
/// Trees within this of him are solid (m).
pub const TREE_REACH_M: f32 = 35.0;
/// A trunk goes this far under its foot (slopes), and is solid up to this share of the model's height (the leaves aren't).
const TRUNK_ROOT_M: f32 = 1.0;
const TRUNK_SHARE: f32 = 0.7;
const TRUNK_MIN_M: f32 = 2.5;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key {
    Building { face: u8, model: u8, id: IVec2 },
    /// Its cell's seed and its number in the cell.
    Tree { seed: i32, n: i32 },
}

struct Piece {
    shape: SolidShape,
    pos: Vec3,
    rot: Quat,
    surface: Surface,
}

struct Placed {
    /// Where the list had it when its pieces were made (frame, m) and its size: a finer ground moves a tree.
    foot: Vec3,
    size: Vec3,
    handles: Vec<SolidHandle>,
    /// It held him when it was seen: no pieces until he is out.
    waiting: bool,
}

/// What the sandbox has made solid from the probe's lists.
#[derive(Default)]
pub struct Solids {
    /// The list last taken, and the world's count of clearings it was taken under.
    seq: u32,
    cleared: u32,
    placed: HashMap<Key, Placed>,
    /// For the log: how many of each are solid now.
    pub buildings: usize,
    pub trees: usize,
}

impl Solids {
    /// Bring the world's pieces up to the newest list (`None`: the probe has none for this body: everything goes).
    /// `player` is his box (centre, half sizes; frame, m) while he walks.
    pub fn sync(&mut self, world: &mut SandboxWorld, list: Option<(&[ProbeBuilding], u32)>, lib: &ScatterLibrary, broken: &dyn Fn(&BuildingKey) -> bool, player: Option<(Vec3, Vec3)>) {
        if world.solids_cleared != self.cleared {
            self.cleared = world.solids_cleared;
            self.placed.clear();
            self.seq = 0;
        }
        let Some((list, seq)) = list else {
            if !self.placed.is_empty() {
                for (_, p) in self.placed.drain() {
                    for h in p.handles {
                        world.remove_solid(h);
                    }
                }
                world.refresh();
                world.wake_props();
            }
            self.seq = 0;
            (self.buildings, self.trees) = (0, 0);
            return;
        };
        if seq == self.seq && !self.placed.values().any(|p| p.waiting) {
            return;
        }
        self.seq = seq;
        let mut changed = false;
        let mut seen = HashSet::with_capacity(list.len());
        for b in list {
            let key = match b.kind {
                // (The far class is drawn only beyond most of the near range: never round him.)
                1 => Key::Building { face: b.face, model: b.model, id: b.id },
                4 => Key::Tree { seed: b.id.x, n: b.id.y },
                _ => continue,
            };
            if b.kind == 1 && broken(&BuildingKey { face: b.face, model: b.model, id: b.id }) {
                continue;
            }
            if !seen.insert(key) {
                continue;
            }
            let foot = world.frame.from_body(b.foot).as_vec3();
            if self.placed.get(&key).is_some_and(|p| !p.waiting && (p.foot - foot).length() < 0.05 && (p.size - b.size).abs().max_element() < 0.05) {
                continue;
            }
            let pieces = pieces_of(b, &world.frame, lib);
            if let Some(was) = self.placed.remove(&key) {
                for h in was.handles {
                    world.remove_solid(h);
                    changed = true;
                }
            }
            if pieces.is_empty() {
                continue;
            }
            let holds = player.is_some_and(|(c, h)| pieces.iter().any(|p| world.solid_holds(&p.shape, p.pos, p.rot, c, h)));
            let handles = if holds { Vec::new() } else { pieces.iter().filter_map(|p| world.add_solid(&p.shape, p.pos, p.rot, p.surface)).collect() };
            changed |= !handles.is_empty();
            self.placed.insert(key, Placed { foot, size: b.size, handles, waiting: holds });
        }
        self.placed.retain(|k, p| {
            let keep = seen.contains(k);
            if !keep {
                for h in p.handles.drain(..) {
                    world.remove_solid(h);
                    changed = true;
                }
            }
            keep
        });
        if changed {
            world.refresh();
            world.wake_props();
        }
        self.buildings = self.placed.iter().filter(|(k, p)| matches!(k, Key::Building { .. }) && !p.waiting).count();
        self.trees = self.placed.iter().filter(|(k, p)| matches!(k, Key::Tree { .. }) && !p.waiting).count();
    }

    /// What is solid, nearest first: whether it is a tree, its foot (frame, m), its size as the list gave it, and whether
    /// it waits for him to step out (for the test commands).
    pub fn nearest(&self, from: Vec3, n: usize) -> Vec<(bool, Vec3, Vec3, bool)> {
        let mut all: Vec<_> = self.placed.iter().map(|(k, p)| (matches!(k, Key::Tree { .. }), p.foot, p.size, p.waiting)).collect();
        all.sort_by(|a, b| (a.1 - from).length_squared().total_cmp(&(b.1 - from).length_squared()));
        all.truncate(n);
        all
    }

    /// How many wait for him to step out of them.
    pub fn waiting(&self) -> usize {
        self.placed.values().filter(|p| p.waiting).count()
    }
}

/// A list entry's solid pieces in the frame.
fn pieces_of(b: &ProbeBuilding, frame: &LocalFrame, lib: &ScatterLibrary) -> Vec<Piece> {
    if !b.foot.is_finite() || !b.up.is_finite() {
        return Vec::new();
    }
    let to_local = frame.axes.transpose();
    let foot = frame.from_body(b.foot).as_vec3();
    let up = (to_local * b.up).as_vec3().try_normalize().unwrap_or(Vec3::Z);
    if b.kind == 4 {
        let Some(t) = lib.trunks.get(b.model as usize) else { return Vec::new() };
        let scale = b.size.z;
        if !(scale > 0.05 && scale < 50.0) {
            return Vec::new();
        }
        let radius = (t[0] * scale).clamp(0.05, 2.5);
        let height = (t[1] * scale * TRUNK_SHARE).max(TRUNK_MIN_M);
        let half = (height + TRUNK_ROOT_M) * 0.5;
        return vec![Piece { shape: SolidShape::Trunk { radius, half_height: half }, pos: foot + up * (half - TRUNK_ROOT_M), rot: Quat::from_rotation_arc(Vec3::Z, up), surface: Surface::Wood }];
    }
    let Some(s) = Solid::of(b) else { return Vec::new() };
    if s.key.model == M_DOME {
        return Vec::new();
    }
    // The building's own frame: x across its width, y up from its foot, z across its depth (`Solid::local`).
    let za = (to_local * s.xa).as_vec3().cross(up).try_normalize().unwrap_or(Vec3::X);
    let xa = up.cross(za);
    let rot = Quat::from_mat3(&Mat3::from_cols(xa, up, za)).normalize();
    let (w, d, h) = (s.size.x as f32, s.size.y as f32, s.size.z as f32);
    // (The village's huts and longhouses are timber.)
    let surface = if matches!(s.key.model, 1 | 2) { Surface::Wood } else { Surface::Concrete };
    s.slices()
        .iter()
        .filter_map(|sl| {
            let (y0, y1) = (sl.y[0] * h, sl.y[1] * h);
            if y1 - y0 < 0.02 || sl.half.x * w < 0.02 || sl.half.y * d < 0.02 {
                return None;
            }
            if sl.turn.abs() < 1e-4 {
                let centre = Vec3::new(w * sl.mid.x, 0.5 * (y0 + y1), d * sl.mid.y);
                let half = Vec3::new(w * sl.half.x, 0.5 * (y1 - y0), d * sl.half.y);
                return Some(Piece { shape: SolidShape::Box(half), pos: foot + rot * centre, rot, surface });
            }
            // A turned slice: u = (x / w) cos + (z / d) sin, v = -(x / w) sin + (z / d) cos (`Prism::of`), so on a footprint
            // that isn't square its corners make a parallelogram.
            let (si, c) = sl.turn.sin_cos();
            let mut corners = Vec::with_capacity(8);
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let (u, v) = (sl.mid.x + su * sl.half.x, sl.mid.y + sv * sl.half.y);
                let (x, z) = (w * (u * c - v * si), d * (u * si + v * c));
                corners.push(Vec3::new(x, y0, z));
                corners.push(Vec3::new(x, y1, z));
            }
            Some(Piece { shape: SolidShape::Hull(corners), pos: foot, rot, surface })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::movement::TICK;
    use crate::sandbox::world::{Met, PropShape, TerrainGrid};
    use bevy::math::DVec3;
    use se_core::catalog::ObjectRef;

    const R: f64 = 6.371e6;

    fn frame() -> LocalFrame {
        LocalFrame::at(ObjectRef::Body(0), DVec3::ONE, DVec3::new(0.0, 0.0, R))
    }

    fn library() -> ScatterLibrary {
        // Model 0: a trunk 0.3 m round, a tree 10 m tall.
        ScatterLibrary { trunks: vec![[0.3, 10.0]], ..Default::default() }
    }

    /// A box building (model 0) with its foot `x`, `y` metres east and north of the frame's anchor.
    fn building(f: &LocalFrame, x: f64, y: f64, w: f32, d: f32, h: f32, id: i32) -> ProbeBuilding {
        ProbeBuilding { id: IVec2::new(id, 0), face: 0, model: 0, kind: 1, foot: f.to_body(DVec3::new(x, y, 0.0)), up: f.axes.z_axis, xa: f.axes.x_axis, size: Vec3::new(w, d, h), k: 1.0 }
    }

    fn tree(f: &LocalFrame, x: f64, y: f64, scale: f32, n: i32) -> ProbeBuilding {
        ProbeBuilding { id: IVec2::new(77, n), face: 0, model: 0, kind: 4, foot: f.to_body(DVec3::new(x, y, 0.0)), up: f.axes.z_axis, xa: DVec3::Z, size: Vec3::new(0.0, 0.0, scale), k: 1.0 }
    }

    fn world() -> SandboxWorld {
        let mut w = SandboxWorld::new(frame(), TICK);
        w.set_terrain(TerrainGrid { n: 65, cell: 2.0, heights: vec![0.0; 65 * 65] });
        w
    }

    const HALF: Vec3 = Vec3::new(0.3, 0.3, 0.68);

    #[test]
    fn a_building_and_a_tree_stop_his_box_and_a_shot() {
        let mut w = world();
        let f = w.frame;
        let lib = library();
        let list = [building(&f, 10.0, 0.0, 8.0, 6.0, 12.0, 1), tree(&f, 0.0, 6.0, 1.0, 3)];
        let mut s = Solids::default();
        s.sync(&mut w, Some((&list, 1)), &lib, &|_| false, Some((Vec3::new(0.0, 0.0, 0.7), HALF)));
        assert_eq!((s.buildings, s.trees), (1, 1));
        // Walking east he meets the wall 6 m off (its width is along east: 10 - 4), less his half box.
        let hit = w.sweep_box(Vec3::new(0.0, 0.0, 0.7), Vec3::new(9.0, 0.0, 0.7), HALF).expect("the wall");
        assert!(matches!(hit.met, Met::Solid(Surface::Concrete)));
        assert!((hit.fraction * 9.0 - 5.7).abs() < 0.02, "{}", hit.fraction * 9.0);
        assert!(hit.normal.x < -0.99);
        // North, the trunk: 6 m off, 0.3 m round.
        let hit = w.sweep_box(Vec3::new(0.0, 0.0, 0.7), Vec3::new(0.0, 9.0, 0.7), HALF).expect("the trunk");
        assert!(matches!(hit.met, Met::Solid(Surface::Wood)));
        assert!((hit.fraction * 9.0 - 5.4).abs() < 0.02, "{}", hit.fraction * 9.0);
        // A shot at the trunk, and one over the roof.
        let ray = w.ray(Vec3::new(0.0, 0.0, 1.2), Vec3::Y, 50.0, None).expect("the trunk");
        assert!(matches!(ray.met, Met::Solid(Surface::Wood)) && (ray.dist - 5.7).abs() < 0.01);
        assert!(w.ray(Vec3::new(0.0, 0.0, 12.5), Vec3::X, 50.0, None).is_none());
        // The roof holds a prop up.
        let id = w.spawn(&PropShape::Box { centre: Vec3::ZERO, half: Vec3::splat(0.25) }, Vec3::new(10.0, 0.0, 13.0), Quat::IDENTITY, 20.0, Surface::WoodCrate).unwrap();
        let mut impacts = Vec::new();
        for _ in 0..200 {
            w.step(&mut impacts);
        }
        let z = w.prop(id).unwrap().now.pos.z;
        assert!((z - 12.25).abs() < 0.03, "{z}");
    }

    #[test]
    fn what_leaves_the_list_goes_and_a_moved_frame_places_it_again() {
        let mut w = world();
        let f = w.frame;
        let lib = library();
        let mut s = Solids::default();
        let list = [building(&f, 10.0, 0.0, 8.0, 6.0, 12.0, 1), tree(&f, 0.0, 6.0, 1.0, 3)];
        s.sync(&mut w, Some((&list, 1)), &lib, &|_| false, None);
        assert_eq!(w.solid_count(), 2);
        // The same list again costs nothing; a list without the tree takes its trunk out.
        s.sync(&mut w, Some((&list, 1)), &lib, &|_| false, None);
        assert_eq!(w.solid_count(), 2);
        s.sync(&mut w, Some((&list[..1], 2)), &lib, &|_| false, None);
        assert_eq!((w.solid_count(), s.trees), (1, 0));
        assert!(w.sweep_box(Vec3::new(0.0, 0.0, 0.7), Vec3::new(0.0, 9.0, 0.7), HALF).is_none());
        // The frame moves 20 m east: the world drops the pieces, the next sync puts the building 20 m nearer.
        let to = LocalFrame::at(f.obj, DVec3::ONE, f.to_body(DVec3::new(20.0, 0.0, 0.0)));
        w.rebase(to);
        assert_eq!(w.solid_count(), 0);
        s.sync(&mut w, Some((&list[..1], 2)), &lib, &|_| false, None);
        assert_eq!(w.solid_count(), 1);
        let hit = w.sweep_box(Vec3::new(-20.0, 0.0, 0.7), Vec3::new(-8.0, 0.0, 0.7), HALF).expect("the wall");
        assert!((hit.fraction * 12.0 - 5.7).abs() < 0.05, "{}", hit.fraction * 12.0);
        // A broken building is nothing; no list, nothing.
        s.sync(&mut w, Some((&list, 3)), &lib, &|k| k.id.x == 1, None);
        assert_eq!((s.buildings, s.trees), (0, 1));
        s.sync(&mut w, None, &lib, &|_| false, None);
        assert_eq!(w.solid_count(), 0);
    }

    #[test]
    fn a_building_that_holds_him_waits_until_he_is_out() {
        let mut w = world();
        let f = w.frame;
        let lib = library();
        let mut s = Solids::default();
        let list = [building(&f, 0.0, 0.0, 8.0, 6.0, 12.0, 1)];
        let inside = Vec3::new(1.0, 0.0, 0.7);
        s.sync(&mut w, Some((&list, 1)), &lib, &|_| false, Some((inside, HALF)));
        assert_eq!((w.solid_count(), s.buildings, s.waiting()), (0, 0, 1));
        // Still inside: still nothing, though the list is the same one.
        s.sync(&mut w, Some((&list, 1)), &lib, &|_| false, Some((Vec3::new(3.0, 0.0, 0.7), HALF)));
        assert_eq!(w.solid_count(), 0);
        // Out of it: solid.
        s.sync(&mut w, Some((&list, 1)), &lib, &|_| false, Some((Vec3::new(6.0, 0.0, 0.7), HALF)));
        assert_eq!((w.solid_count(), s.buildings, s.waiting()), (1, 1, 0));
    }
}
