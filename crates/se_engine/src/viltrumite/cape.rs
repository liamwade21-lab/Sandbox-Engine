//! The cape (spec F13): spring chains on the 30 added joints, in the sampler, in his model space. Its 44 pinned vertices
//! are skinned to his back as they always were; a chain's first joint is rigid with his chest; the five others of each
//! chain are points here.
//!
//! It can't explode, whatever it's fed: what moves it is clamped before it gets here (`CapeIn`, 1000 c is nothing to it), a
//! hitch runs less time rather than a long step, a point's speed against him is clamped, and every step ends with each
//! link at its exact length from the chest down, so no point is ever further from him than its chain is long. Bad numbers
//! start it over.

use bevy::math::{Affine3A, Mat3A, Quat, Vec3, Vec3A};
use se_core::hero_asset::{HeroModel, MAX_BONES};

use super::tune;

/// 5 chains of 6.
const POINTS: usize = 30;
/// What the cloth is kept out of: a bone to a bone, a radius (m).
const BODY_CAPSULES: [(&str, &str, f32); 10] = [
    ("SKEL_Pelvis", "SKEL_Spine3", 0.14),
    ("SKEL_Spine3", "SKEL_Neck_1", 0.11),
    ("SKEL_L_Thigh", "SKEL_L_Calf", 0.1),
    ("SKEL_R_Thigh", "SKEL_R_Calf", 0.1),
    ("SKEL_L_Calf", "SKEL_L_Foot", 0.07),
    ("SKEL_R_Calf", "SKEL_R_Foot", 0.07),
    ("SKEL_L_UpperArm", "SKEL_L_Forearm", 0.06),
    ("SKEL_R_UpperArm", "SKEL_R_Forearm", 0.06),
    ("SKEL_L_Forearm", "SKEL_L_Hand", 0.05),
    ("SKEL_R_Forearm", "SKEL_R_Hand", 0.05),
];

/// The flat of his back, which no capsule fits: in the bind pose of a bone (he faces -Y there, so behind him is +y), a box the
/// points are kept behind while they're in it. Half-width, lowest and highest z, and y of the box's back face: the mesh's own back
/// (`print_body_numbers`) and a little. Its front is `BACK_BOX_FRONT`: what's blown round in front of him stays there.
const BACK_BOXES: [(&str, f32, f32, f32, f32); 2] = [("SKEL_Spine3", 0.36, 0.0, 0.62, 0.23), ("SKEL_Pelvis", 0.26, -0.35, 0.05, 0.195)];
const BACK_BOX_FRONT: f32 = -0.1;

/// What moves the cloth this frame, in model axes. `dt` 0: it keeps its place.
#[derive(Clone, Copy, Debug, Default)]
pub struct CapeIn {
    pub dt: f32,
    /// What the cloth feels: gravity less his own acceleration (m/s2). Hanging from a hovering hero, gravity; falling with
    /// him, nothing; accelerating, it goes back.
    pub accel: Vec3,
    /// The air's velocity past him (m/s).
    pub wind: Vec3,
    /// How much air there is, 0..1. 0: inertia only.
    pub air: f32,
    /// His body's turn since last frame, as it moves points fixed in space in model axes.
    pub turn: Quat,
}

pub struct Cape {
    ready: bool,
    pos: [Vec3; POINTS],
    prev: [Vec3; POINTS],
    /// Each point's length to the one above it in its chain.
    len: [f32; POINTS],
    /// Each point's distance to the same link of the next chain, on his body as made.
    across: [f32; POINTS],
    capsules: [(u16, u16, f32); BODY_CAPSULES.len()],
    /// `BACK_BOXES`' bones.
    backs: [Option<u16>; BACK_BOXES.len()],
    /// His chest last frame: the middle chain's first joint.
    chest: (Quat, Vec3),
}

impl Default for Cape {
    fn default() -> Self {
        Cape { ready: false, pos: [Vec3::ZERO; POINTS], prev: [Vec3::ZERO; POINTS], len: [0.0; POINTS], across: [0.0; POINTS], capsules: [(0, 0, 0.0); BODY_CAPSULES.len()], backs: [None; BACK_BOXES.len()], chest: (Quat::IDENTITY, Vec3::ZERO) }
    }
}

/// The nearest point to `p` on a segment.
fn nearest_on(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
    a + ab * t
}

impl Cape {
    /// Where the link wants to lie from the point above: most of the way to straight down his body from its rest direction,
    /// as it hung before it had springs.
    fn rest_dir(bind: Vec3, top: Mat3A) -> Vec3 {
        let dir = Vec3::from(top * Vec3A::from(bind)).normalize_or(Vec3::NEG_Z);
        (dir * 0.3 + Vec3::NEG_Z * tune::CAPE_HANG).normalize_or(Vec3::NEG_Z)
    }

    fn begin(&mut self, model: &HeroModel, world: &[Affine3A; MAX_BONES], first: usize, chains: usize, links: usize) {
        for (slot, (a, b, r)) in self.capsules.iter_mut().zip(BODY_CAPSULES) {
            let find = |name: &str| model.bones.iter().position(|x| x.name == name);
            *slot = match (find(a), find(b)) {
                (Some(a), Some(b)) => (a as u16, b as u16, r),
                _ => (0, 0, 0.0),
            };
        }
        for (slot, b) in self.backs.iter_mut().zip(BACK_BOXES) {
            *slot = model.bones.iter().position(|x| x.name == b.0).map(|i| i as u16);
        }
        let bind_at = |i: usize| Vec3::from(model.bones[i].inverse_bind.inverse().w_axis.truncate());
        // The sheet is cut as a diamond: the outer chains run out to its side corners and back, half as long again as the middle one. Hanging
        // or streaming they'd end in two points and a notch, so each chain is drawn in by how much longer it is. (The skinning's linear:
        // the cloth along it just comes closer together.)
        let chain_len = |c: usize| (1..links).map(|l| model.bones[first + c * links + l].position.length()).sum::<f32>();
        let middle = chain_len(chains / 2);
        for c in 0..chains {
            let root = first + c * links;
            let top = world[root].matrix3;
            let draw_in = 1.0 + (middle / chain_len(c).max(1e-6) - 1.0) * tune::CAPE_HEM_LEVEL;
            self.pos[c * links] = world[root].translation.into();
            for l in 0..links {
                let k = c * links + l;
                if l > 0 {
                    let bind = model.bones[root + l].position;
                    self.len[k] = bind.length() * draw_in;
                    self.pos[k] = self.pos[k - 1] + Self::rest_dir(bind, top) * self.len[k];
                }
                self.across[k] = if c + 1 < chains { (bind_at(root + l) - bind_at(root + links + l)).length() } else { 0.0 };
                self.prev[k] = self.pos[k];
            }
        }
        let mid = first + chains / 2 * links;
        self.chest = (Quat::from_mat3a(&world[mid].matrix3).normalize(), Vec3::from(world[mid].translation));
        self.ready = true;
    }

    fn simulate(&mut self, model: &HeroModel, world: &[Affine3A; MAX_BONES], first: usize, chains: usize, links: usize, i: CapeIn) {
        let steps = ((i.dt / tune::CAPE_STEP_S).ceil() as usize).clamp(1, tune::CAPE_MAX_STEPS);
        let h = (i.dt / steps as f32).min(tune::CAPE_STEP_S);
        let accel = i.accel.clamp_length_max(tune::CAPE_ACCEL_MAX);
        let wind = i.wind.clamp_length_max(tune::CAPE_WIND_MAX_MS);
        let drag = tune::CAPE_WIND_K * i.air.clamp(0.0, 1.0);
        // His chest's turn this frame, against his model: `i.turn` is what a point fixed in space does, the chest's own what a
        // point rigid with it does (a turn clip's root, his body's turn, whichever it is). The cloth goes between the two, or a
        // turn on the spot leaves it where it was and it ends up in front of him.
        let mid = first + chains / 2 * links;
        let chest = (Quat::from_mat3a(&world[mid].matrix3).normalize(), Vec3::from(world[mid].translation));
        let with_chest = (chest.0 * self.chest.0.inverse()).normalize();
        let q = i.turn.slerp(with_chest, tune::CAPE_FOLLOW).normalize();
        for c in 0..chains {
            for l in 1..links {
                let k = c * links + l;
                self.pos[k] = chest.1 + q * (self.pos[k] - self.chest.1);
                self.prev[k] = chest.1 + q * (self.prev[k] - self.chest.1);
            }
        }
        self.chest = chest;
        for _ in 0..steps {
            for c in 0..chains {
                let root = first + c * links;
                let top = world[root].matrix3;
                for l in 1..links {
                    let k = c * links + l;
                    let bind = model.bones[root + l].position;
                    let want = self.pos[k - 1] + Self::rest_dir(bind, top) * self.len[k];
                    let v = ((self.pos[k] - self.prev[k]) / h).clamp_length_max(tune::CAPE_SPEED_MAX);
                    let a = accel + (wind - v) * drag + (want - self.pos[k]) * tune::CAPE_REST_K;
                    self.prev[k] = self.pos[k];
                    self.pos[k] += v * h * tune::CAPE_DAMP + a * h * h;
                }
            }
            for _ in 0..tune::CAPE_SOLVE {
                for c in 0..chains {
                    for l in 1..links {
                        let k = c * links + l;
                        let d = self.pos[k] - self.pos[k - 1];
                        let n = d.length().max(1e-6);
                        let fix = d * ((n - self.len[k]) / n);
                        if l == 1 {
                            self.pos[k] -= fix;
                        } else {
                            self.pos[k] -= fix * 0.5;
                            self.pos[k - 1] += fix * 0.5;
                        }
                    }
                }
                // Across the sheet: it may fold up (never right down to one line), it can't stretch.
                for c in 0..chains.saturating_sub(1) {
                    for l in 1..links {
                        let (k, o) = (c * links + l, (c + 1) * links + l);
                        let d = self.pos[o] - self.pos[k];
                        let n = d.length().max(1e-6);
                        let (lo, hi) = (self.across[k] * 0.3, self.across[k]);
                        let to = n.clamp(lo, hi);
                        if to != n {
                            let fix = d * (0.5 * (n - to) / n);
                            self.pos[k] += fix;
                            self.pos[o] -= fix;
                        }
                    }
                }
            }
            self.collide(model, world, chains, links);
            // The last word: each link at its length from the chest down.
            for c in 0..chains {
                for l in 1..links {
                    let k = c * links + l;
                    let d = self.pos[k] - self.pos[k - 1];
                    self.pos[k] = self.pos[k - 1] + d.normalize_or(Vec3::NEG_Z) * self.len[k];
                }
            }
        }
    }

    /// Pushes the points out of his body. (The links between them can still cross a limb, and the sheet between two chains;
    /// a thigh is as wide as the gap, so little shows.)
    fn collide(&mut self, model: &HeroModel, world: &[Affine3A; MAX_BONES], chains: usize, links: usize) {
        for (bone, (_, half, lo, hi, back)) in self.backs.into_iter().zip(BACK_BOXES) {
            let Some(bone) = bone.map(|b| b as usize) else { continue };
            let skin = world[bone] * Affine3A::from_mat4(model.bones[bone].inverse_bind);
            let to_bind = skin.inverse();
            for c in 0..chains {
                for l in 1..links {
                    let k = c * links + l;
                    let mut q = Vec3::from(to_bind.transform_point3a(Vec3A::from(self.pos[k])));
                    if q.x.abs() < half && q.z > lo && q.z < hi && q.y < back && q.y > BACK_BOX_FRONT {
                        q.y = back;
                        self.pos[k] = Vec3::from(skin.transform_point3a(Vec3A::from(q)));
                    }
                }
            }
        }
        for &(a, b, r) in &self.capsules {
            if r <= 0.0 {
                continue;
            }
            let (pa, pb) = (Vec3::from(world[a as usize].translation), Vec3::from(world[b as usize].translation));
            for c in 0..chains {
                for l in 1..links {
                    let k = c * links + l;
                    let near = nearest_on(self.pos[k], pa, pb);
                    let d = self.pos[k] - near;
                    let n2 = d.length_squared();
                    if n2 < r * r && n2 > 1e-10 {
                        self.pos[k] = near + d * (r / n2.sqrt());
                    }
                }
            }
        }
    }

    /// One frame: moves the points and writes the chains' joints into `world`.
    pub fn step(&mut self, model: &HeroModel, world: &mut [Affine3A; MAX_BONES], i: CapeIn) {
        let Some(first) = model.cape_first else { return };
        let (first, chains, links) = (first as usize, model.cape_chains as usize, model.cape_links as usize);
        if chains * links > POINTS || links < 2 || first + chains * links > model.bones.len().min(MAX_BONES) {
            return;
        }
        if !self.ready {
            self.begin(model, world, first, chains, links);
        }
        for c in 0..chains {
            let k = c * links;
            self.pos[k] = world[first + k].translation.into();
            self.prev[k] = self.pos[k];
        }
        if i.dt > 0.0 && i.dt.is_finite() {
            self.simulate(model, world, first, chains, links, i);
        }
        if !self.pos.iter().chain(&self.prev).all(|p| p.is_finite()) {
            self.begin(model, world, first, chains, links);
        }
        for c in 0..chains {
            let root = first + c * links;
            let top = world[root].matrix3;
            for l in 1..links {
                let (k, joint) = (c * links + l, root + l);
                let bind = model.bones[joint].position;
                let length = bind.length();
                if length < 1e-6 {
                    world[joint] = world[joint - 1];
                    continue;
                }
                let rest = Vec3::from(top * Vec3A::from(bind)) / length;
                let want = (self.pos[k] - self.pos[k - 1]).normalize_or(rest);
                // The chest's turn first, then from the chain's rest direction to where it lies: he faces the other way from
                // his bind pose, and without the chest's turn the normals stay in the bind's direction, on the side of the
                // cloth away from what shows.
                world[joint] = Affine3A { matrix3: Mat3A::from_quat(Quat::from_rotation_arc(rest, want)) * top, translation: self.pos[k].into() };
            }
        }
    }

    /// How far the points are from where they hung at rest: for tests. The largest distance of a point from his middle.
    #[cfg(test)]
    pub fn reach(&self) -> f32 {
        self.pos.iter().map(|p| p.length()).fold(0.0, f32::max)
    }

    #[cfg(test)]
    pub fn point(&self, k: usize) -> Vec3 {
        self.pos[k]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viltrumite::anim::Sampler;
    use se_core::hero_asset::{ClipSet, parse_clips, parse_model};

    fn real() -> Option<(HeroModel, ClipSet)> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/mods/viltrumite");
        let m = parse_model(&std::fs::read(root.join("omniman.sevm")).ok()?).ok()?;
        let c = parse_clips(&std::fs::read(root.join("clips.sevc")).ok()?).ok()?;
        Some((m, c))
    }

    /// Runs `secs` of frames at `fps` of what `feed(t)` says, on a walking body, and checks the cape's bounds every frame.
    fn run(m: &HeroModel, c: &ClipSet, s: &mut Sampler, secs: f32, fps: f32, feed: impl Fn(f32) -> CapeIn) {
        let dt = 1.0 / fps;
        // (As cut: drawn in, they're shorter.)
        let longest = (0..5).map(|ch| (1..6).map(|l| m.bones[98 + ch * 6 + l].position.length()).sum::<f32>()).fold(0.0, f32::max);
        let mut t = 0.0;
        while t < secs {
            s.cape_in = CapeIn { dt, ..feed(t) };
            s.advance(c, dt);
            s.sample(m, c);
            assert!(s.cape.reach().is_finite());
            // No point further than its chain's length from the chest (a little more for where the chain starts).
            let chest = Vec3::from(s.world(98).translation);
            for k in 0..30 {
                let d = (s.cape.point(k) - chest).length();
                assert!(d < longest + 0.8, "point {k}: {d} at {t}");
            }
            t += dt;
        }
    }

    #[test]
    fn it_hangs_behind_him_and_streams_back_in_the_wind() {
        let Some((m, c)) = real() else { return };
        let mut s = Box::<Sampler>::default();
        s.play(c.find("idle").unwrap() as u16, 0.0);
        // Standing still under gravity (model -Z).
        run(&m, &c, &mut s, 4.0, 60.0, |_| CapeIn { accel: Vec3::NEG_Z * 9.8, air: 1.0, turn: Quat::IDENTITY, ..default_in() });
        let top = s.cape.point(12);
        let hem = s.cape.point(17);
        assert!(hem.z < top.z - 0.7, "hanging: top {top} hem {hem}");
        // He faces +Y, so it hangs behind him.
        assert!(hem.y < 0.0, "behind him: {hem}");
        // Flying at 60 m/s: the air comes from ahead, +Y, and this clip lies along it, head first.
        s.play(c.find("bf_fast_botharmsback").unwrap() as u16, 0.0);
        run(&m, &c, &mut s, 3.0, 60.0, |_| CapeIn { accel: Vec3::ZERO, wind: Vec3::NEG_Y * 60.0, air: 1.0, turn: Quat::IDENTITY, ..default_in() });
        let (top, hem) = (s.cape.point(12), s.cape.point(17));
        assert!(hem.y < top.y - 0.7, "streams back: top {top} hem {hem}");
    }

    fn default_in() -> CapeIn {
        CapeIn { dt: 0.0, accel: Vec3::ZERO, wind: Vec3::ZERO, air: 0.0, turn: Quat::IDENTITY }
    }

    /// Worse than anything real: huge numbers, wild turns, a frame hitch, sudden reversals. It stays on him.
    #[test]
    fn it_never_explodes() {
        let Some((m, c)) = real() else { return };
        let mut s = Box::<Sampler>::default();
        s.play(c.find("sprint").unwrap() as u16, 0.0);
        run(&m, &c, &mut s, 1.0, 60.0, |t| CapeIn {
            accel: Vec3::new(1e25, -1e25, if t < 0.5 { 1e25 } else { -1e25 }),
            wind: Vec3::new(1e26, 1e26, -1e26),
            air: 5.0,
            turn: Quat::from_rotation_z(if t < 0.5 { 2.0 } else { -2.0 }),
            ..default_in()
        });
        // A hitch of 0.1 s after each other, then 1000 c in vacuum.
        run(&m, &c, &mut s, 3.0, 10.0, |_| CapeIn { accel: Vec3::Z * 500.0, air: 0.0, turn: Quat::IDENTITY, ..default_in() });
        run(&m, &c, &mut s, 1.0, 60.0, |_| CapeIn { accel: Vec3::ZERO, turn: Quat::IDENTITY, ..default_in() });
    }

    /// `cargo test -p se_engine print_body_numbers -- --ignored --nocapture`: where his back is, for `BACK_BOXES`.
    #[test]
    #[ignore]
    fn print_body_numbers() {
        let Some((m, _)) = real() else { return };
        let mut most = [[f32::MIN; 8]; 16];
        for d in m.draws.iter().filter(|d| !d.cutout() && !d.cape()) {
            for v in &m.vertices[d.first_vertex as usize..(d.first_vertex + d.vertex_count) as usize] {
                let (x, y, z) = (v.position[0], v.position[1], v.position[2]);
                let (bz, bx) = (((z + 1.0) / 0.125) as i32, (x.abs() / 0.06) as i32);
                if (0..16).contains(&bz) && (0..8).contains(&bx) {
                    let c = &mut most[bz as usize][bx as usize];
                    *c = c.max(y);
                }
            }
        }
        for (i, row) in most.iter().enumerate() {
            let r: Vec<String> = row.iter().map(|v| if *v > -9.0 { format!("{v:6.2}") } else { "     .".to_string() }).collect();
            println!("z {:5.2}: {}", -1.0 + i as f32 * 0.125, r.join(" "));
        }
    }

    /// Numbers that aren't numbers start it over rather than reach the palette.
    #[test]
    fn nan_starts_it_over() {
        let Some((m, c)) = real() else { return };
        let mut s = Box::<Sampler>::default();
        s.play(c.find("idle").unwrap() as u16, 0.0);
        s.sample(&m, &c);
        s.cape_in = CapeIn { dt: 1.0 / 60.0, accel: Vec3::NAN, ..default_in() };
        s.sample(&m, &c);
        assert!(s.cape.reach().is_finite());
        for b in 98..128 {
            assert!(s.world(b).translation.is_finite(), "bone {b}");
        }
    }

    /// While he stands, no point sits inside his body, once it has settled.
    #[test]
    fn it_stays_out_of_his_body() {
        let Some((m, c)) = real() else { return };
        let mut s = Box::<Sampler>::default();
        s.play(c.find("run").unwrap() as u16, 0.0);
        run(&m, &c, &mut s, 3.0, 60.0, |_| CapeIn { accel: Vec3::NEG_Z * 9.8, wind: Vec3::NEG_Y * 5.0, air: 1.0, turn: Quat::IDENTITY, ..default_in() });
        let spine = m.bones.iter().position(|b| b.name == "SKEL_Spine3").unwrap();
        let pelvis = m.bones.iter().position(|b| b.name == "SKEL_Pelvis").unwrap();
        let (a, b) = (Vec3::from(s.world(pelvis).translation), Vec3::from(s.world(spine).translation));
        for k in (0..30).filter(|k| k % 6 != 0) {
            let p = s.cape.point(k);
            assert!((p - nearest_on(p, a, b)).length() > 0.09, "point {k} at {p}");
        }
    }
}
