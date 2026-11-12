//! The sandbox's physics: props and the player's box in one rapier world, in a local frame fixed to the planet.
//!
//! The frame (`LocalFrame`) is east, north, up at an anchor on the body, in metres, turning with the body: gravity is straight
//! down it at the planet's own strength. Rapier works in `f32`, so the anchor follows the player (`rebase`): nothing is ever more
//! than a few hundred metres from the frame's origin, where `f32` is good to a hundredth of a millimetre. The ground is a height
//! field sampled from the terrain's own loaded heights round the anchor (`TerrainGrid`); props that leave it are frozen until it
//! covers them again, as nothing would hold them up.

use std::collections::HashMap;

use bevy::math::{DMat3, DQuat, DVec3, Quat, Vec3};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::parry::utils::Array2;
use rapier3d::prelude::*;
use se_core::catalog::ObjectRef;

/// The ground, the props, the player's box, and a prop the gravity gun holds (which passes through the player).
const G_TERRAIN: Group = Group::GROUP_1;
const G_PROP: Group = Group::GROUP_2;
const G_PLAYER: Group = Group::GROUP_3;
const G_HELD: Group = Group::GROUP_4;
/// What stands on the ground and never moves: a building's pieces, a tree's trunk (`solids.rs`).
const G_SOLID: Group = Group::GROUP_5;

fn groups(member: Group, filter: Group) -> InteractionGroups {
    InteractionGroups::new(member, filter, InteractionTestMode::And)
}

/// East, north, up at a point of a body, in the body's turning frame.
#[derive(Clone, Copy, Debug)]
pub struct LocalFrame {
    pub obj: ObjectRef,
    /// Body frame, metres from its centre.
    pub anchor: DVec3,
    /// Columns: east, north, up.
    pub axes: DMat3,
}

impl LocalFrame {
    /// The frame at `point` (body frame) of a body of this ellipsoid `shape`: up is the ellipsoid's normal there, so level ground
    /// is level on a flattened planet too.
    pub fn at(obj: ObjectRef, shape: DVec3, point: DVec3) -> Self {
        let up = (point / shape / shape).try_normalize().unwrap_or(DVec3::Y);
        let north = (DVec3::Y - up * up.y).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
        let east = north.cross(up);
        LocalFrame { obj, anchor: point, axes: DMat3::from_cols(east, north, up) }
    }

    pub fn to_body(&self, p: DVec3) -> DVec3 {
        self.anchor + self.axes * p
    }

    pub fn from_body(&self, q: DVec3) -> DVec3 {
        self.axes.transpose() * (q - self.anchor)
    }

    /// The turn from this frame's axes to the body's.
    pub fn rotation(&self) -> DQuat {
        DQuat::from_mat3(&self.axes)
    }
}

/// The ground's heights round the anchor: `n` x `n` samples `cell` metres apart, `heights[iy * n + ix]` the height (local z) at
/// x = (ix - (n-1)/2) cell east, y = (iy - (n-1)/2) cell north.
#[derive(Clone, Debug)]
pub struct TerrainGrid {
    pub n: usize,
    pub cell: f32,
    pub heights: Vec<f32>,
}

impl TerrainGrid {
    pub fn half(&self) -> f32 {
        (self.n - 1) as f32 * self.cell * 0.5
    }

    /// The height at a point of the grid, as the height field's triangles have it nearly enough (bilinear); `None` off the grid.
    pub fn height(&self, x: f32, y: f32) -> Option<f32> {
        let h = self.half();
        let fx = (x + h) / self.cell;
        let fy = (y + h) / self.cell;
        let max = (self.n - 1) as f32;
        if !(0.0..=max).contains(&fx) || !(0.0..=max).contains(&fy) {
            return None;
        }
        let (ix, iy) = ((fx as usize).min(self.n - 2), (fy as usize).min(self.n - 2));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let at = |x: usize, y: usize| self.heights[y * self.n + x];
        let a = at(ix, iy) + (at(ix + 1, iy) - at(ix, iy)) * tx;
        let b = at(ix, iy + 1) + (at(ix + 1, iy + 1) - at(ix, iy + 1)) * tx;
        Some(a + (b - a) * ty)
    }
}

/// What a prop is made of, for its sounds and how it slides and bounces (Source's `surfaceprop`, the handful the basic props use).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    Metal,
    MetalBarrel,
    Wood,
    WoodCrate,
    Plastic,
    PlasticBarrel,
    Concrete,
    Cardboard,
    Rubber,
    Flesh,
    Glass,
}

impl Surface {
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: [Surface; 11] = [
        Surface::Metal,
        Surface::MetalBarrel,
        Surface::Wood,
        Surface::WoodCrate,
        Surface::Plastic,
        Surface::PlasticBarrel,
        Surface::Concrete,
        Surface::Cardboard,
        Surface::Rubber,
        Surface::Flesh,
        Surface::Glass,
    ];

    /// The game's surface property of this kind (`surfaceproperties.txt`): what it sounds like.
    pub fn prop_name(self) -> &'static str {
        match self {
            Surface::Metal => "metal",
            Surface::MetalBarrel => "metal_barrel",
            Surface::Wood => "wood",
            Surface::WoodCrate => "wood_crate",
            Surface::Plastic => "plastic",
            Surface::PlasticBarrel => "plastic_barrel",
            Surface::Concrete => "concrete",
            Surface::Cardboard => "cardboard",
            Surface::Rubber => "rubber",
            Surface::Flesh => "flesh",
            Surface::Glass => "glass",
        }
    }

    /// From a model's `surfaceprop` string.
    pub fn named(s: &str) -> Self {
        let s = s.to_ascii_lowercase();
        let has = |k: &str| s.contains(k);
        if has("barrel") && has("plastic") {
            Surface::PlasticBarrel
        } else if has("barrel") || has("canister") || has("propane") || has("oildrum") {
            Surface::MetalBarrel
        } else if has("crate") || has("wood_box") {
            Surface::WoodCrate
        } else if has("wood") {
            Surface::Wood
        } else if has("cardboard") || has("paper") {
            Surface::Cardboard
        } else if has("plastic") || has("pottery") || has("item") {
            Surface::Plastic
        } else if has("concrete") || has("rock") || has("brick") || has("boulder") || has("tile") {
            Surface::Concrete
        } else if has("rubber") || has("tire") {
            Surface::Rubber
        } else if has("flesh") || has("watermelon") || has("zombie") || has("alien") {
            Surface::Flesh
        } else if has("glass") {
            Surface::Glass
        } else {
            Surface::Metal
        }
    }

    /// Friction and bounce, roughly `surfaceproperties.txt`'s (its friction 0.8 is the default, rubber and flesh more, plastic less).
    fn contact(self) -> (f32, f32) {
        match self {
            Surface::Metal | Surface::MetalBarrel => (0.6, 0.1),
            Surface::Wood | Surface::WoodCrate => (0.75, 0.08),
            Surface::Plastic | Surface::PlasticBarrel => (0.55, 0.15),
            Surface::Concrete => (0.85, 0.02),
            Surface::Cardboard => (0.8, 0.02),
            Surface::Rubber => (0.95, 0.5),
            Surface::Flesh => (0.9, 0.05),
            Surface::Glass => (0.5, 0.05),
        }
    }
}

/// A prop's solid shape, in its own frame (metres, the model's axes).
#[derive(Clone, Debug)]
pub enum PropShape {
    /// Convex pieces as point clouds (a model's own collision pieces, or its render mesh as one).
    Hulls(Vec<Vec<Vec3>>),
    /// A box by its half sizes, centred at `centre`.
    Box { centre: Vec3, half: Vec3 },
    Ball { centre: Vec3, radius: f32 },
}

/// A piece of something that stands on the ground (a building's slice, a tree's trunk), in its own frame.
#[derive(Clone, Debug)]
pub enum SolidShape {
    /// A box by its half sizes.
    Box(Vec3),
    /// A convex piece by its corners.
    Hull(Vec<Vec3>),
    /// A cylinder along its frame's z.
    Trunk { radius: f32, half_height: f32 },
}

impl SolidShape {
    fn shared(&self) -> Option<SharedShape> {
        match self {
            SolidShape::Box(h) => Some(SharedShape::cuboid(h.x, h.y, h.z)),
            SolidShape::Hull(points) => SharedShape::convex_hull(points),
            // (Parry's cylinder stands along y.)
            SolidShape::Trunk { radius, half_height } => Some(SharedShape::compound(vec![(
                Pose3::from_parts(Vec3::ZERO, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                SharedShape::cylinder(*half_height, *radius),
            )])),
        }
    }
}

/// A solid piece in the world (`SandboxWorld::add_solid`).
pub type SolidHandle = ColliderHandle;

/// A prop's place: where it is in the local frame now and where it was a tick ago (drawn between the two).
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
}

pub struct Prop {
    pub id: u32,
    handle: RigidBodyHandle,
    pub mass: f32,
    pub surface: Surface,
    /// Its model's own surface property (`metal_barrel`, `watermelon`...), for its sounds; `None`: its kind's.
    pub material: Option<String>,
    pub prev: Pose,
    pub now: Pose,
    /// Frozen by the player (the physics gun's right button), or because the ground under it isn't known.
    pub frozen: bool,
    out_of_range: bool,
    pub held: bool,
    /// The velocity after the last tick, for the impact sounds.
    last_vel: Vec3,
    /// Seconds until it may sound again.
    quiet: f32,
    /// Its radius round its centre of mass, metres.
    pub radius: f32,
}

/// A prop hit something hard enough to hear.
#[derive(Clone, Copy, Debug)]
pub struct Impact {
    /// The prop.
    pub id: u32,
    pub at: Vec3,
    /// How fast its velocity changed in the tick, m/s.
    pub speed: f32,
}

/// What a ray or a hull trace met.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Met {
    Ground,
    Prop(u32),
    /// A building or a tree.
    Solid(Surface),
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    #[cfg_attr(not(test), allow(dead_code))]
    pub dist: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub met: Met,
}

#[derive(Clone, Copy, Debug)]
pub struct HullHit {
    pub fraction: f32,
    /// The surface's outward normal and the point of it that was touched.
    pub normal: Vec3,
    pub point: Vec3,
    pub met: Met,
}

pub struct SandboxWorld {
    pub frame: LocalFrame,
    world: PhysicsWorld,
    terrain: Option<(ColliderHandle, TerrainGrid)>,
    player: RigidBodyHandle,
    player_collider: ColliderHandle,
    props: HashMap<u32, Prop>,
    by_collider: HashMap<ColliderHandle, u32>,
    solids: HashMap<ColliderHandle, Surface>,
    /// Counts the times every solid piece was taken out (a new frame): whoever placed them places them again.
    pub solids_cleared: u32,
    next_id: u32,
    /// Gravity down the frame's z, m/s^2.
    pub gravity: f32,
}

impl SandboxWorld {
    pub fn new(frame: LocalFrame, dt: f32) -> Self {
        let mut world = PhysicsWorld::new();
        world.gravity = Vec3::new(0.0, 0.0, -9.81);
        world.integration_parameters.dt = dt;
        // The player's box: moved by the port, so props bounce off him and can't sink into him.
        let (player, player_collider) = world.insert(
            RigidBodyBuilder::kinematic_position_based().translation(Vec3::new(0.0, 0.0, 1.0e4)),
            ColliderBuilder::cuboid(0.3, 0.3, 0.68).collision_groups(groups(G_PLAYER, G_PROP)).friction(0.0),
        );
        SandboxWorld {
            frame,
            world,
            terrain: None,
            player,
            player_collider,
            props: HashMap::new(),
            by_collider: HashMap::new(),
            solids: HashMap::new(),
            solids_cleared: 0,
            next_id: 1,
            gravity: 9.81,
        }
    }

    pub fn has_ground(&self) -> bool {
        self.terrain.is_some()
    }

    pub fn terrain(&self) -> Option<&TerrainGrid> {
        self.terrain.as_ref().map(|t| &t.1)
    }

    pub fn props(&self) -> impl Iterator<Item = &Prop> {
        self.props.values()
    }

    pub fn prop(&self, id: u32) -> Option<&Prop> {
        self.props.get(&id)
    }

    pub fn prop_count(&self) -> usize {
        self.props.len()
    }

    /// Replace the ground. Props and the player's feet left under the new ground are the caller's to lift (`lift_onto_ground`).
    pub fn set_terrain(&mut self, grid: TerrainGrid) {
        if let Some((h, _)) = self.terrain.take() {
            self.world.remove_collider(h);
        }
        let n = grid.n;
        // Parry's field: rows along its z, columns along its x, heights along its y. Turned a quarter about x its y is the
        // frame's up and its z the frame's south, so row i is the grid's row n-1-i.
        let heights = Array2::from_fn(n, n, |i, j| grid.heights[(n - 1 - i) * n + j]);
        let size = (n - 1) as f32 * grid.cell;
        let collider = ColliderBuilder::heightfield(heights, Vec3::new(size, 1.0, size))
            .position(Pose3::from_parts(Vec3::ZERO, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)))
            // (The player's box too: its sweeps go by the groups; as a kinematic body it never pushes on the ground.)
            .collision_groups(groups(G_TERRAIN, G_PROP | G_HELD | G_PLAYER))
            .friction(0.9)
            .restitution(0.0);
        let h = self.world.insert_collider(collider, None);
        self.terrain = Some((h, grid));
        self.world.wake_up_all(true);
        self.refresh();
    }

    /// Rays and sweeps go by the broad phase's tree, which a step updates: after anything is added, removed or moved by hand
    /// it is brought up to date at once, or the next query would not see the change (a fall through new ground).
    pub fn refresh(&mut self) {
        self.world.detect_collisions(&(), &());
    }

    /// A piece of a building or a tree at a place of the frame. The caller refreshes once it has added and removed all it
    /// means to (`refresh`). `None` if the shape can't be made.
    pub fn add_solid(&mut self, shape: &SolidShape, pos: Vec3, rot: Quat, surface: Surface) -> Option<SolidHandle> {
        let (friction, bounce) = surface.contact();
        let collider = ColliderBuilder::new(shape.shared()?)
            .position(Pose3::from_parts(pos, rot))
            .collision_groups(groups(G_SOLID, G_PROP | G_HELD | G_PLAYER))
            .friction(friction)
            .restitution(bounce);
        let h = self.world.insert_collider(collider, None);
        self.solids.insert(h, surface);
        Some(h)
    }

    pub fn remove_solid(&mut self, h: SolidHandle) {
        if self.solids.remove(&h).is_some() {
            self.world.remove_collider(h);
        }
    }

    /// Every solid piece out (the frame moves: they are placed again in the new one).
    pub fn clear_solids(&mut self) {
        for (h, _) in self.solids.drain() {
            self.world.remove_collider(h);
        }
        self.solids_cleared = self.solids_cleared.wrapping_add(1);
    }

    pub fn solid_count(&self) -> usize {
        self.solids.len()
    }

    /// Props sleeping against a piece that has gone must fall.
    pub fn wake_props(&mut self) {
        self.world.wake_up_all(true);
    }

    /// Would a piece at this place hold a box (the player's: its centre and half sizes)?
    pub fn solid_holds(&self, shape: &SolidShape, pos: Vec3, rot: Quat, centre: Vec3, half: Vec3) -> bool {
        let Some(s) = shape.shared() else { return false };
        rapier3d::parry::query::intersection_test(&Pose3::from_parts(pos, rot), &*s, &Pose3::from_translation(centre), &Cuboid::new(half)).is_ok_and(|i| i.intersecting)
    }

    /// A new ground field, in a new frame if the anchor moved. Finer terrain arriving changes the ground's heights (by over
    /// a hundred metres as a place first loads: a coarse node is far off the real ground): what stood or flew over the old
    /// field keeps its height over the new one, or everything would be dropped, or buried.
    pub fn replace_ground(&mut self, frame: LocalFrame, grid: TerrainGrid) {
        let over: Vec<(u32, f32)> = match self.terrain.as_ref() {
            Some((_, old)) => self
                .props
                .values()
                .filter_map(|p| {
                    let at = self.world.bodies.get(p.handle)?.translation();
                    Some((p.id, at.z - old.height(at.x, at.y)?))
                })
                .collect(),
            None => Vec::new(),
        };
        if frame.anchor != self.frame.anchor {
            self.rebase(frame);
        }
        for (id, agl) in over {
            let Some(p) = self.props.get_mut(&id) else { continue };
            let Some(b) = self.world.bodies.get_mut(p.handle) else { continue };
            let at = b.translation();
            let Some(h) = grid.height(at.x, at.y) else { continue };
            let dz = h + agl - at.z;
            if dz.abs() > 0.002 {
                b.set_translation(at + Vec3::Z * dz, false);
                p.now.pos.z += dz;
                p.prev.pos.z += dz;
            }
        }
        self.set_terrain(grid);
    }

    pub fn clear_terrain(&mut self) {
        if let Some((h, _)) = self.terrain.take() {
            self.world.remove_collider(h);
            self.refresh();
        }
    }

    /// Move the frame to `to` (the same body): everything keeps its place on the planet. (The buildings and trees are taken
    /// out: `solids.rs` places them again in the new frame.)
    pub fn rebase(&mut self, to: LocalFrame) {
        self.clear_solids();
        let from = self.frame;
        let turn = (to.axes.transpose() * from.axes).as_mat3();
        let q = Quat::from_mat3(&turn);
        let shift = |p: Vec3| to.from_body(from.to_body(p.as_dvec3())).as_vec3();
        for prop in self.props.values_mut() {
            if let Some(b) = self.world.bodies.get_mut(prop.handle) {
                let pos = shift(b.translation());
                let rot = (q * *b.rotation()).normalize();
                b.set_position(Pose3::from_parts(pos, rot), false);
                b.set_linvel(turn * b.linvel(), false);
                b.set_angvel(turn * b.angvel(), false);
            }
            for pose in [&mut prop.prev, &mut prop.now] {
                pose.pos = shift(pose.pos);
                pose.rot = (q * pose.rot).normalize();
            }
            prop.last_vel = turn * prop.last_vel;
        }
        self.frame = to;
        self.refresh();
    }

    /// Add a prop; `None` if its shape can't be made solid.
    pub fn spawn(&mut self, shape: &PropShape, pos: Vec3, rot: Quat, mass: f32, surface: Surface) -> Option<u32> {
        let (friction, bounce) = surface.contact();
        let shared = match shape {
            PropShape::Box { centre, half } => SharedShape::compound(vec![(Pose3::from_translation(*centre), SharedShape::cuboid(half.x, half.y, half.z))]),
            PropShape::Ball { centre, radius } => SharedShape::compound(vec![(Pose3::from_translation(*centre), SharedShape::ball(*radius))]),
            PropShape::Hulls(pieces) => {
                let hulls: Vec<_> = pieces.iter().filter(|p| p.len() >= 4).filter_map(|p| SharedShape::convex_hull(p)).map(|s| (Pose3::identity(), s)).collect();
                if hulls.is_empty() {
                    return None;
                }
                SharedShape::compound(hulls)
            }
        };
        let radius = shared.compute_local_bounding_sphere().radius();
        let body = RigidBodyBuilder::dynamic().pose(Pose3::from_parts(pos, rot)).ccd_enabled(true).linear_damping(0.02).angular_damping(0.08);
        let collider = ColliderBuilder::new(shared).mass(mass.max(0.1)).friction(friction).restitution(bounce).collision_groups(groups(G_PROP, G_TERRAIN | G_SOLID | G_PROP | G_PLAYER | G_HELD));
        let (handle, ch) = self.world.insert(body, collider);
        let id = self.next_id;
        self.next_id += 1;
        let pose = Pose { pos, rot };
        self.props.insert(id, Prop { id, handle, mass: mass.max(0.1), surface, material: None, prev: pose, now: pose, frozen: false, out_of_range: false, held: false, last_vel: Vec3::ZERO, quiet: 0.3, radius });
        self.by_collider.insert(ch, id);
        self.refresh();
        Some(id)
    }

    /// A prop's own surface property, by its model.
    pub fn set_material(&mut self, id: u32, name: &str) {
        if let Some(p) = self.props.get_mut(&id) {
            p.material = (!name.is_empty()).then(|| name.to_ascii_lowercase());
        }
    }

    pub fn remove(&mut self, id: u32) {
        if let Some(p) = self.props.remove(&id) {
            self.by_collider.retain(|_, v| *v != id);
            self.world.remove_body_with_colliders(p.handle, true);
        }
    }

    pub fn remove_all(&mut self) {
        let ids: Vec<u32> = self.props.keys().copied().collect();
        for id in ids {
            self.remove(id);
        }
    }

    fn body(&self, id: u32) -> Option<&RigidBody> {
        self.world.bodies.get(self.props.get(&id)?.handle)
    }

    fn body_mut(&mut self, id: u32) -> Option<&mut RigidBody> {
        let h = self.props.get(&id)?.handle;
        self.world.bodies.get_mut(h)
    }

    /// A prop's centre of mass and velocity, in the frame.
    pub fn motion(&self, id: u32) -> Option<(Vec3, Vec3)> {
        let b = self.body(id)?;
        Some((b.center_of_mass(), b.linvel()))
    }

    pub fn velocity_at(&self, id: u32, point: Vec3) -> Vec3 {
        self.body(id).map_or(Vec3::ZERO, |b| b.velocity_at_point(point))
    }

    /// Push a prop: an impulse (kg m/s) at a point of the frame.
    pub fn impulse(&mut self, id: u32, impulse: Vec3, at: Vec3) {
        if self.props.get(&id).is_some_and(|p| p.frozen) {
            return;
        }
        if let Some(b) = self.body_mut(id) {
            b.apply_impulse_at_point(impulse, at, true);
        }
    }

    pub fn set_velocity(&mut self, id: u32, linear: Vec3, angular: Option<Vec3>) {
        if let Some(b) = self.body_mut(id) {
            b.set_linvel(linear, true);
            if let Some(a) = angular {
                b.set_angvel(a, true);
            }
        }
    }

    /// Freeze a prop where it is, or let it go (the physics gun's right button, and R).
    pub fn set_frozen(&mut self, id: u32, frozen: bool) {
        let Some(p) = self.props.get_mut(&id) else { return };
        p.frozen = frozen;
        let fixed = p.frozen || p.out_of_range;
        if let Some(b) = self.world.bodies.get_mut(p.handle) {
            b.set_body_type(if fixed { RigidBodyType::Fixed } else { RigidBodyType::Dynamic }, true);
        }
    }

    /// Held by a gun: it no longer hits the player, and doesn't fall.
    pub fn set_held(&mut self, id: u32, held: bool) {
        let Some(p) = self.props.get_mut(&id) else { return };
        p.held = held;
        let handle = p.handle;
        if let Some(b) = self.world.bodies.get_mut(handle) {
            b.set_gravity_scale(if held { 0.0 } else { 1.0 }, true);
            b.set_angular_damping(if held { 6.0 } else { 0.08 });
        }
        let g = if held { groups(G_HELD, G_TERRAIN | G_SOLID | G_PROP | G_HELD) } else { groups(G_PROP, G_TERRAIN | G_SOLID | G_PROP | G_PLAYER | G_HELD) };
        let colliders: Vec<ColliderHandle> = self.by_collider.iter().filter(|(_, v)| **v == id).map(|(c, _)| *c).collect();
        for c in colliders {
            if let Some(co) = self.world.colliders.get_mut(c) {
                co.set_collision_groups(g);
            }
        }
    }

    /// The player's box for this tick: its bottom's middle and its size (metres).
    pub fn set_player(&mut self, origin: Vec3, half: f32, height: f32) {
        if let Some(c) = self.world.colliders.get_mut(self.player_collider) {
            let now = c.shape().as_cuboid().map(|c| c.half_extents);
            let want = Vec3::new(half, half, height * 0.5);
            if now != Some(want) {
                c.set_shape(SharedShape::cuboid(want.x, want.y, want.z));
            }
        }
        if let Some(b) = self.world.bodies.get_mut(self.player) {
            b.set_next_kinematic_translation(origin + Vec3::Z * (height * 0.5));
        }
    }

    /// Take the player's box out of the world (free flight, a mod).
    pub fn park_player(&mut self) {
        if let Some(b) = self.world.bodies.get_mut(self.player) {
            b.set_translation(Vec3::new(0.0, 0.0, 1.0e4), false);
        }
    }

    fn met(&self, c: ColliderHandle) -> Met {
        match self.by_collider.get(&c) {
            Some(id) => Met::Prop(*id),
            None => self.solids.get(&c).map_or(Met::Ground, |s| Met::Solid(*s)),
        }
    }

    /// Sweep a box of half sizes `half` with its centre from `from` to `to`, against the ground, the buildings and trees, and
    /// the props that aren't held.
    pub fn sweep_box(&self, from: Vec3, to: Vec3, half: Vec3) -> Option<HullHit> {
        let shape = Cuboid::new(half);
        let filter = QueryFilter::new().exclude_rigid_body(self.player).groups(groups(G_PLAYER, G_TERRAIN | G_SOLID | G_PROP));
        let options = ShapeCastOptions { max_time_of_impact: 1.0, target_distance: 0.0, stop_at_penetration: false, compute_impact_geometry_on_penetration: true };
        let (c, hit) = self.world.cast_shape(&Pose3::from_translation(from), to - from, &shape, options, filter)?;
        // (Rapier's query: `normal1` and `witness1` are the collider's, in the world's frame.)
        Some(HullHit { fraction: hit.time_of_impact.clamp(0.0, 1.0), normal: hit.normal1, point: hit.witness1, met: self.met(c) })
    }

    /// Is a box at `centre` through the ground's surface?
    pub fn box_in_ground(&self, centre: Vec3, half: Vec3) -> bool {
        let shape = Cuboid::new(half);
        let filter = QueryFilter::new().exclude_rigid_body(self.player).groups(groups(G_PLAYER, G_TERRAIN));
        self.world.intersect_shape(Pose3::from_translation(centre), &shape, filter).next().is_some()
    }

    /// Is a box at `centre` inside the ground, a building, a tree or a prop?
    pub fn box_blocked(&self, centre: Vec3, half: Vec3) -> bool {
        let shape = Cuboid::new(half);
        let filter = QueryFilter::new().exclude_rigid_body(self.player).groups(groups(G_PLAYER, G_TERRAIN | G_SOLID | G_PROP));
        self.world.intersect_shape(Pose3::from_translation(centre), &shape, filter).next().is_some()
    }

    /// A ray against the ground and every prop (a held one too unless `skip` names it).
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32, skip: Option<u32>) -> Option<RayHit> {
        let mut filter = QueryFilter::new().exclude_rigid_body(self.player);
        let skip_handle = skip.and_then(|id| self.props.get(&id)).map(|p| p.handle);
        if let Some(h) = skip_handle {
            // (One exclusion at a time: the player's box is told apart by its group instead.)
            filter = QueryFilter::new().exclude_rigid_body(h).groups(groups(G_PROP | G_HELD | G_TERRAIN | G_SOLID, G_TERRAIN | G_SOLID | G_PROP | G_HELD));
        }
        let ray = Ray::new(origin, dir);
        let (c, hit) = self.world.cast_ray_and_get_normal(&ray, max, true, filter)?;
        Some(RayHit { dist: hit.time_of_impact, point: origin + dir * hit.time_of_impact, normal: hit.normal, met: self.met(c) })
    }

    /// One tick: the world steps, the props' poses move on, and what hit hard is listed.
    pub fn step(&mut self, impacts: &mut Vec<Impact>) {
        let dt = self.world.integration_parameters.dt;
        self.world.gravity = Vec3::new(0.0, 0.0, -self.gravity);
        // Off the height field nothing holds a prop up: frozen there, let go once the field covers it again.
        let reach = self.terrain.as_ref().map(|t| t.1.half() - 3.0);
        for p in self.props.values_mut() {
            let Some(b) = self.world.bodies.get_mut(p.handle) else { continue };
            let at = b.translation();
            let out = reach.is_none_or(|r| at.x.abs() > r || at.y.abs() > r);
            if out != p.out_of_range {
                p.out_of_range = out;
                let fixed = p.frozen || out;
                b.set_body_type(if fixed { RigidBodyType::Fixed } else { RigidBodyType::Dynamic }, !fixed);
            }
        }
        self.world.step();
        let g = Vec3::new(0.0, 0.0, -self.gravity * dt);
        for p in self.props.values_mut() {
            let Some(b) = self.world.bodies.get(p.handle) else { continue };
            p.prev = p.now;
            p.now = Pose { pos: b.translation(), rot: *b.rotation() };
            let v = b.linvel();
            p.quiet = (p.quiet - dt).max(0.0);
            if b.is_dynamic() && !p.held {
                // What changed its velocity beyond gravity: a hit (Source: nothing under 70 units/s is heard).
                let dv = (v - p.last_vel - g).length();
                if dv > 1.3 && p.quiet == 0.0 {
                    impacts.push(Impact { id: p.id, at: b.center_of_mass(), speed: dv });
                    p.quiet = 0.12;
                }
            }
            p.last_vel = v;
        }
    }

    /// After new ground: nothing may be left under it (a finer node's heights can be metres off a coarser one's).
    pub fn lift_onto_ground(&mut self) {
        let Some((_, grid)) = self.terrain.as_ref() else { return };
        for p in self.props.values_mut() {
            let Some(b) = self.world.bodies.get_mut(p.handle) else { continue };
            let at = b.translation();
            let Some(h) = grid.height(at.x, at.y) else { continue };
            // (Its middle under the ground by more than it could be lying there.)
            if at.z < h - 0.05 {
                let lift = h + p.radius * 0.5 - at.z;
                b.set_translation(at + Vec3::Z * lift, true);
                p.now.pos.z += lift;
                p.prev = p.now;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::movement::TICK;

    fn frame() -> LocalFrame {
        LocalFrame::at(ObjectRef::Body(0), DVec3::ONE, DVec3::new(0.0, 0.0, 6.371e6))
    }

    fn flat(z: f32) -> TerrainGrid {
        TerrainGrid { n: 65, cell: 1.0, heights: vec![z; 65 * 65] }
    }

    #[test]
    fn the_frame_is_east_north_up() {
        // On the equator at longitude 90 W (+z in the body frame): up is +z, north +y, east -x... by the right hand: N x U.
        let f = frame();
        assert!((f.axes.z_axis - DVec3::Z).length() < 1e-12);
        assert!((f.axes.y_axis - DVec3::Y).length() < 1e-12);
        assert!((f.axes.x_axis - DVec3::Y.cross(DVec3::Z)).length() < 1e-12);
        let p = DVec3::new(3.0, -4.0, 5.0);
        assert!((f.from_body(f.to_body(p)) - p).length() < 1e-6);
        // A flattened body: up is the ellipsoid's normal, not the radius.
        let shape = DVec3::new(1.0, 0.9, 1.0);
        let at = DVec3::new(0.0, 0.5, 0.7).normalize() * shape * 6.0e6;
        let f = LocalFrame::at(ObjectRef::Body(0), shape, at);
        let radial = at.normalize();
        assert!(f.axes.z_axis.dot(radial) < 0.9999 && f.axes.z_axis.y > radial.y);
    }

    #[test]
    fn a_prop_falls_at_the_worlds_gravity_and_rests_on_the_ground() {
        let mut w = SandboxWorld::new(frame(), TICK);
        w.set_terrain(flat(2.0));
        w.gravity = 3.7;
        let shape = PropShape::Box { centre: Vec3::ZERO, half: Vec3::splat(0.25) };
        let id = w.spawn(&shape, Vec3::new(1.0, 2.0, 12.0), Quat::IDENTITY, 20.0, Surface::WoodCrate).unwrap();
        let mut impacts = Vec::new();
        // Half a second of free fall: g t^2 / 2.
        let ticks = (0.5 / TICK).round() as usize;
        for _ in 0..ticks {
            w.step(&mut impacts);
        }
        let t = ticks as f32 * TICK;
        let fallen = 12.0 - w.prop(id).unwrap().now.pos.z;
        assert!((fallen - 0.5 * 3.7 * t * t).abs() < 0.08, "fell {fallen}");
        assert!(impacts.is_empty());
        for _ in 0..600 {
            w.step(&mut impacts);
        }
        let z = w.prop(id).unwrap().now.pos.z;
        assert!((z - 2.25).abs() < 0.02, "rests at {z}");
        assert!(!impacts.is_empty(), "the landing was heard");
        // Off the field it is held, not dropped.
        let far = w.spawn(&shape, Vec3::new(200.0, 0.0, 30.0), Quat::IDENTITY, 20.0, Surface::WoodCrate).unwrap();
        for _ in 0..100 {
            w.step(&mut impacts);
        }
        assert!((w.prop(far).unwrap().now.pos.z - 30.0).abs() < 1e-3);
    }

    #[test]
    fn the_ground_field_follows_the_grid() {
        // A slope rising to the north: 0.5 m a metre.
        let n = 65;
        let mut heights = vec![0.0; n * n];
        for iy in 0..n {
            for ix in 0..n {
                heights[iy * n + ix] = (iy as f32 - 32.0) * 0.5;
            }
        }
        let grid = TerrainGrid { n, cell: 1.0, heights };
        assert!((grid.height(3.0, 10.0).unwrap() - 5.0).abs() < 1e-4);
        let mut w = SandboxWorld::new(frame(), TICK);
        w.set_terrain(grid);
        for (x, y) in [(0.0, 0.0), (5.0, 10.0), (-7.5, -12.25), (20.0, 3.3)] {
            let hit = w.ray(Vec3::new(x, y, 100.0), -Vec3::Z, 500.0, None).expect("ground");
            assert!((hit.point.z - y * 0.5).abs() < 0.01, "at {x} {y}: {}", hit.point.z);
            assert_eq!(hit.met, Met::Ground);
            // The slope's normal leans south.
            assert!(hit.normal.y < -0.3 && hit.normal.z > 0.8, "{:?}", hit.normal);
        }
        // The player's box stops on it.
        let half = Vec3::new(0.3, 0.3, 0.68);
        let hit = w.sweep_box(Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -10.0), half).expect("lands");
        let z = 10.0 - 20.0 * hit.fraction;
        // Its northern edge (0.3 m north, 0.15 m higher) touches first; the normal is the slope's.
        assert!((z - (0.68 + 0.15)).abs() < 0.02, "box centre at {z}");
        assert!(hit.normal.y < -0.3 && hit.normal.z > 0.8, "{:?}", hit.normal);
    }

    #[test]
    fn new_ground_replaces_the_old_at_once() {
        // Finer terrain arrives and the field is made again, between ticks: the next sweep must meet the new one, not
        // nothing (he fell through the planet).
        let mut w = SandboxWorld::new(frame(), TICK);
        let half = Vec3::new(0.3, 0.3, 0.68);
        let mut impacts = Vec::new();
        for (i, z) in [0.0_f32, -80.0, -60.0, 5.0].into_iter().enumerate() {
            w.set_terrain(flat(z));
            let hit = w.sweep_box(Vec3::new(1.0, 1.0, z + 3.0), Vec3::new(1.0, 1.0, z - 3.0), half).unwrap_or_else(|| panic!("field {i} at {z} is not there"));
            assert!(((z + 3.0 - 6.0 * hit.fraction) - (z + 0.68)).abs() < 0.01, "field {i}");
            let ray = w.ray(Vec3::new(0.0, 0.0, 500.0), -Vec3::Z, 1000.0, None).expect("ray");
            assert!((ray.point.z - z).abs() < 0.01, "field {i}: the ray met {}", ray.point.z);
            for _ in 0..3 {
                w.step(&mut impacts);
            }
            let ray = w.ray(Vec3::new(0.0, 0.0, 500.0), -Vec3::Z, 1000.0, None).expect("ray after steps");
            assert!((ray.point.z - z).abs() < 0.01, "field {i} after steps: the ray met {}", ray.point.z);
        }
    }

    #[test]
    fn rebasing_keeps_props_where_they_are_on_the_planet() {
        let mut w = SandboxWorld::new(frame(), TICK);
        let shape = PropShape::Ball { centre: Vec3::ZERO, radius: 0.2 };
        let id = w.spawn(&shape, Vec3::new(10.0, 20.0, 3.0), Quat::from_rotation_z(0.3), 5.0, Surface::Rubber).unwrap();
        let before = w.frame.to_body(w.prop(id).unwrap().now.pos.as_dvec3());
        let to = LocalFrame::at(w.frame.obj, DVec3::ONE, w.frame.to_body(DVec3::new(40.0, -30.0, 0.0)));
        w.rebase(to);
        let after = w.frame.to_body(w.prop(id).unwrap().now.pos.as_dvec3());
        assert!((after - before).length() < 1e-3, "{}", (after - before).length());
        let p = w.prop(id).unwrap().now.pos;
        assert!((p.x + 30.0).abs() < 0.01 && (p.y - 50.0).abs() < 0.01);
    }
}
