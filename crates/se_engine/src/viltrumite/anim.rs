//! His animation sampler (spec F13): four fixed layers over the bones' rest poses, local nlerp with the
//! sign fixed, parent-first forward kinematics, no allocation per frame, no Bevy animation.
//!
//! Layer order is the order they are applied. A full clip (not `CLIP_PARTIAL`) sends every bone under its
//! mask to the clip's pose, and a bone without a track to its rest pose. A partial clip moves only the bones
//! it has tracks for. The slots: 0 the base clip, 1-3 the leans, 4 a masked action (step 8), 5 turning (step 5).
//!
//! A crossfade starts from a frozen copy of the pose as it stands ([`Sampler::crossfade`]), not from a second
//! running clip: it can't pop however often the base changes, and it takes no slots.

use bevy::math::{Affine3A, Mat3A, Quat, Vec3, Vec3A, Vec4};
use se_core::hero_asset::{Clip, ClipSet, HeroModel, MAX_BONES};

use super::tune::CAPE_HANG;

pub const LAYERS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    /// Index into `ClipSet::clips`; `None` is an empty slot.
    pub clip: Option<u16>,
    /// Seconds into the clip (wrapped on a looping clip, clamped otherwise). A lean clip's time is how far
    /// he leans, `lean * duration` (the file format's comment).
    pub time: f32,
    pub weight: f32,
    /// The bones this layer may move (bit n, bone n).
    pub mask: u128,
}

impl Default for Layer {
    fn default() -> Self {
        Layer { clip: None, time: 0.0, weight: 0.0, mask: u128::MAX }
    }
}

/// His pose, inline: about 22 KB, boxed once by its owner.
pub struct Sampler {
    pub layers: [Layer; LAYERS],
    rot: [Quat; MAX_BONES],
    pos: [Vec3; MAX_BONES],
    /// The pose a crossfade started from.
    from_rot: [Quat; MAX_BONES],
    from_pos: [Vec3; MAX_BONES],
    /// The crossfade: seconds gone and how long it lasts (0: none).
    fade_t: f32,
    fade_s: f32,
    /// Bones in model space (metres, Z up, +Y forward, +X his right hand).
    world: [Affine3A; MAX_BONES],
    /// `world * inverse_bind`: what skins a vertex, in model space.
    skin: [Affine3A; MAX_BONES],
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler {
            layers: [Layer::default(); LAYERS],
            rot: [Quat::IDENTITY; MAX_BONES],
            pos: [Vec3::ZERO; MAX_BONES],
            from_rot: [Quat::IDENTITY; MAX_BONES],
            from_pos: [Vec3::ZERO; MAX_BONES],
            fade_t: 0.0,
            fade_s: 0.0,
            world: [Affine3A::IDENTITY; MAX_BONES],
            skin: [Affine3A::IDENTITY; MAX_BONES],
        }
    }
}

/// The two frames either side of `time` and the share of the second.
fn frame_at(clip: &Clip, fps: f32, time: f32) -> (u32, u32, f32) {
    if clip.frames <= 1 || clip.duration <= 0.0 {
        return (0, 0, 0.0);
    }
    let t = if clip.looping() { time.rem_euclid(clip.duration) } else { time.clamp(0.0, clip.duration) };
    let f = (t * fps).min((clip.frames - 1) as f32);
    let f0 = f as u32;
    let f1 = (f0 + 1).min(clip.frames - 1);
    (f0, f1, f - f0 as f32)
}

impl Sampler {
    /// Plays `clip` alone on the base layer from `time`.
    pub fn play(&mut self, clip: u16, time: f32) {
        self.layers = [Layer::default(); LAYERS];
        self.layers[0] = Layer { clip: Some(clip), time, weight: 1.0, mask: u128::MAX };
    }

    /// From now on, over `seconds`, the pose eases from what it is this moment to what the layers say.
    pub fn crossfade(&mut self, seconds: f32) {
        self.from_rot = self.rot;
        self.from_pos = self.pos;
        self.fade_t = 0.0;
        self.fade_s = seconds;
    }

    /// Moves the looping layers' clocks on by `dt` seconds. (Lean layers keep the time they are given.)
    pub fn advance(&mut self, clips: &ClipSet, dt: f32) {
        if self.fade_s > 0.0 {
            self.fade_t += dt;
            if self.fade_t >= self.fade_s {
                self.fade_s = 0.0;
            }
        }
        for l in &mut self.layers {
            let Some(c) = l.clip.and_then(|c| clips.clips.get(c as usize)) else { continue };
            if c.lean() {
                continue;
            }
            l.time += dt;
            // Keeps float time accurate through a long session.
            if c.looping() && c.duration > 0.0 && l.time > c.duration {
                l.time = l.time.rem_euclid(c.duration);
            }
        }
    }

    /// Works the pose out from the layers, then the cape's joints and the skinning matrices.
    pub fn sample(&mut self, model: &HeroModel, clips: &ClipSet) {
        let n = model.bones.len().min(MAX_BONES);
        for (i, b) in model.bones.iter().take(n).enumerate() {
            self.rot[i] = b.rotation;
            self.pos[i] = b.position;
        }
        let layers = self.layers;
        for layer in &layers {
            let Some(clip) = layer.clip.and_then(|c| clips.clips.get(c as usize)) else { continue };
            if layer.weight <= 0.0 {
                continue;
            }
            let w = layer.weight.min(1.0);
            let (f0, f1, k) = frame_at(clip, clips.fps, layer.time);
            if !clip.partial() {
                // A full clip's bones without a track go to their rest pose.
                let mut rest = layer.mask & !clip.mask;
                if n < 128 {
                    rest &= (1u128 << n) - 1;
                }
                while rest != 0 {
                    let i = rest.trailing_zeros() as usize;
                    rest &= rest - 1;
                    self.rot[i] = if w >= 1.0 { model.bones[i].rotation } else { self.rot[i].lerp(model.bones[i].rotation, w) };
                }
            }
            for t in clips.tracks_of(clip) {
                let b = t.bone as usize;
                if b >= n || layer.mask >> b & 1 == 0 {
                    continue;
                }
                let q = clips.rotation(t, f0).lerp(clips.rotation(t, f1), k);
                self.rot[b] = if w >= 1.0 { q } else { self.rot[b].lerp(q, w) };
            }
            // (A lean moves no root: `bf_fast_botharmsback_left` and `_right` would swing him 0.5 m sideways.)
            if layer.mask & 1 != 0 && !clip.lean() {
                if let (Some(p0), Some(p1)) = (clips.root(clip, f0), clips.root(clip, f1)) {
                    self.pos[0] = self.pos[0].lerp(p0.lerp(p1, k), w);
                }
            }
        }
        if self.fade_s > 0.0 {
            let t = (self.fade_t / self.fade_s).clamp(0.0, 1.0);
            let w = t * t * (3.0 - 2.0 * t);
            for i in 0..n {
                self.rot[i] = self.from_rot[i].lerp(self.rot[i], w);
                self.pos[i] = self.from_pos[i].lerp(self.pos[i], w);
            }
        }
        for (i, b) in model.bones.iter().take(n).enumerate() {
            let local = Affine3A::from_rotation_translation(self.rot[i], self.pos[i]);
            self.world[i] = match b.parent {
                Some(p) => self.world[p as usize] * local,
                None => local,
            };
        }
        hang_cape(model, &mut self.world);
        for (i, b) in model.bones.iter().take(n).enumerate() {
            self.skin[i] = self.world[i] * Affine3A::from_mat4(b.inverse_bind);
        }
    }

    /// A bone in model space.
    #[cfg(test)]
    pub fn world(&self, bone: usize) -> Affine3A {
        self.world[bone]
    }

    /// The palette for the shader: `place` (model to camera-relative render space) times each joint's
    /// skinning matrix, as the matrix's three rows.
    pub fn write_palette(&self, place: Affine3A, out: &mut [[Vec4; 3]; MAX_BONES]) {
        for (s, o) in self.skin.iter().zip(out.iter_mut()) {
            *o = rows(place * *s);
        }
    }
}

fn rows(m: Affine3A) -> [Vec4; 3] {
    let (x, y, z, t) = (m.matrix3.x_axis, m.matrix3.y_axis, m.matrix3.z_axis, m.translation);
    [Vec4::new(x.x, y.x, z.x, t.x), Vec4::new(x.y, y.y, z.y, t.y), Vec4::new(x.z, y.z, z.z, t.z)]
}

/// The cape until its springs arrive (step 5): each chain's first joint stays rigid with the chest, the
/// others ease most of the way to hanging straight down in model space, as the contact sheet's pose does. A
/// link's bind world rotation is the identity, so its local position is its bind vector.
fn hang_cape(model: &HeroModel, world: &mut [Affine3A; MAX_BONES]) {
    let Some(first) = model.cape_first else { return };
    let (first, chains, links) = (first as usize, model.cape_chains as usize, model.cape_links as usize);
    let down = Vec3::new(0.0, 0.0, -1.0);
    for c in 0..chains {
        let root = first + c * links;
        let top = world[root].matrix3;
        for l in 1..links {
            let i = root + l;
            if i >= model.bones.len().min(MAX_BONES) {
                return;
            }
            let bind = model.bones[i].position;
            let length = bind.length();
            if length < 1e-6 {
                world[i] = world[i - 1];
                continue;
            }
            let rest_dir = Vec3::from(top * Vec3A::from(bind)) / length;
            let want = (rest_dir * 0.3 + down * CAPE_HANG).normalize_or(down);
            // The chest's turn first, then from the chain's rest direction to hanging: he faces the other way
            // from his bind pose, and without the chest's turn the normals stay in the bind's direction, on
            // the side of the cloth away from what shows.
            world[i] = Affine3A {
                matrix3: Mat3A::from_quat(Quat::from_rotation_arc(rest_dir, want)) * top,
                translation: world[i - 1].translation + Vec3A::from(want * length),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Mat4;
    use se_core::hero_asset::{CLIP_LOOP, CLIP_PARTIAL, ClipTrack, HeroBone};

    fn bone(parent: Option<u16>, position: Vec3, world: Mat4) -> HeroBone {
        HeroBone { name: String::new(), parent, tag: 0, position, rotation: Quat::IDENTITY, inverse_bind: world.inverse() }
    }

    /// A root, a child 1 m along +X, and a child of it 1 m further.
    fn model() -> HeroModel {
        let bones = vec![
            bone(None, Vec3::ZERO, Mat4::IDENTITY),
            bone(Some(0), Vec3::X, Mat4::from_translation(Vec3::X)),
            bone(Some(1), Vec3::X, Mat4::from_translation(Vec3::X * 2.0)),
        ];
        HeroModel {
            bones,
            draws: Vec::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
            cape_first: None,
            cape_chains: 0,
            cape_links: 0,
            bounds_min: Vec3::ZERO,
            bounds_max: Vec3::ZERO,
        }
    }

    fn q16(q: Quat) -> [i16; 4] {
        [q.x, q.y, q.z, q.w].map(|v| (v * 32767.0) as i16)
    }

    /// Two clips of 2 frames, turning bone 1 by 90 degrees about Z: one full, one partial.
    fn clips() -> ClipSet {
        let turn = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let clip = |name: &str, flags: u32| Clip {
            name: name.to_string(),
            flags,
            frames: 2,
            duration: 1.0 / 30.0,
            ground_z: 0.0,
            stride_speed: 0.0,
            first_track: 0,
            track_count: 1,
            first_root: 0,
            root_count: 0,
            first_event: 0,
            event_count: 0,
            mask: 1 << 1,
        };
        ClipSet {
            fps: 30.0,
            bone_count: 3,
            clips: vec![clip("full", CLIP_LOOP), clip("partial", CLIP_LOOP | CLIP_PARTIAL)],
            tracks: vec![ClipTrack { bone: 1, count: 2, first: 0 }],
            events: Vec::new(),
            quats: vec![q16(Quat::IDENTITY), q16(turn)],
            roots: Vec::new(),
        }
    }

    #[test]
    fn rest_pose_skins_to_the_identity() {
        let (m, c) = (model(), clips());
        let mut s = Sampler::default();
        s.sample(&m, &c);
        for i in 0..3 {
            assert!((s.skin[i].translation.length()) < 1e-5, "bone {i}");
        }
    }

    #[test]
    fn parents_carry_their_children_and_time_interpolates() {
        let (m, c) = (model(), clips());
        let mut s = Sampler::default();
        // Halfway between the frames: 45 degrees.
        s.play(0, 0.5 / 30.0);
        s.sample(&m, &c);
        let p = s.world(2).translation;
        let k = std::f32::consts::FRAC_1_SQRT_2;
        assert!((Vec3::from(p) - Vec3::new(1.0 + k, k, 0.0)).length() < 1e-3, "{p}");
    }

    #[test]
    fn a_partial_clip_leaves_the_rest_to_the_layer_below() {
        let (m, c) = (model(), clips());
        let mut s = Sampler::default();
        s.layers[0] = Layer { clip: Some(0), time: 1.0 / 30.0 - 1e-5, weight: 1.0, mask: u128::MAX };
        // A partial layer on no bone it has a track for moves nothing.
        s.layers[1] = Layer { clip: Some(1), time: 0.0, weight: 1.0, mask: 1 << 2 };
        s.sample(&m, &c);
        let above = s.world(2).translation;
        s.layers[1] = Layer::default();
        s.sample(&m, &c);
        assert!((above - s.world(2).translation).length() < 1e-5);
    }

    #[test]
    fn weights_blend_and_quaternions_take_the_short_way() {
        let (m, c) = (model(), clips());
        let mut s = Sampler::default();
        s.layers[0] = Layer { clip: Some(1), time: 1.0 / 30.0 - 1e-5, weight: 0.5, mask: u128::MAX };
        s.sample(&m, &c);
        // Half way from rest to 90 degrees, by nlerp (45 degrees here, by symmetry).
        let d = Vec3::from(s.world(2).translation - s.world(1).translation);
        assert!((d - Vec3::new(std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2, 0.0)).length() < 2e-3, "{d}");
        // Opposite signs of one rotation blend to that rotation.
        let q = Quat::from_rotation_z(0.3);
        let back = Quat::from_xyzw(-q.x, -q.y, -q.z, -q.w);
        assert!(q.lerp(back, 0.5).angle_between(q) < 1e-3);
    }

    /// The real files: he stands where the clip says, nothing explodes, and a whole frame is cheap.
    #[test]
    fn his_real_files_pose_sanely() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/mods/viltrumite");
        let (Ok(mb), Ok(cb)) = (std::fs::read(root.join("omniman.sevm")), std::fs::read(root.join("clips.sevc"))) else {
            eprintln!("skipping: no mod files");
            return;
        };
        let m = se_core::hero_asset::parse_model(&mb).unwrap();
        let c = se_core::hero_asset::parse_clips(&cb).unwrap();
        let mut s = Box::<Sampler>::default();
        let idle = c.find("bf_idle").unwrap();
        let clip = &c.clips[idle];
        assert!(clip.looping() && !clip.partial());
        s.play(idle as u16, 0.0);
        for k in 0..12 {
            s.layers[0].time = clip.duration * k as f32 / 12.0;
            s.sample(&m, &c);
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for (v, _) in m.vertices.iter().zip(0..).step_by(7) {
                let p = Vec3A::from_array(v.position);
                let mut q = Vec3A::ZERO;
                for (&j, &w) in v.joints.iter().zip(&v.weights) {
                    q += s.skin[j as usize].transform_point3a(p) * (w as f32 / 255.0);
                }
                let q = Vec3::from(q);
                lo = lo.min(q);
                hi = hi.max(q);
            }
            // He is 1.88 m tall: nothing further than an outstretched arm or the cape's hem from his middle.
            assert!(lo.min_element() > -3.0 && hi.max_element() < 3.0, "time {k}: {lo} {hi}");
            // Upright, his pelvis about 1 m over his soles.
            assert!(lo.z < -0.5 && hi.z > 0.5, "time {k}: {lo} {hi}");
        }
        // The cape hangs down from his back: its lowest joints are below his chest and behind him (-Y).
        let (first, chains, links) = (m.cape_first.unwrap() as usize, m.cape_chains as usize, m.cape_links as usize);
        for ch in 0..chains {
            let hem = Vec3::from(s.world(first + ch * links + links - 1).translation);
            let top = Vec3::from(s.world(first + ch * links).translation);
            assert!(hem.z < top.z - 0.5, "chain {ch}: top {top} hem {hem}");
            assert!(hem.length() < 3.0, "chain {ch}: {hem}");
        }
        // A crossfade starts where the pose is: no pop, whatever comes next.
        let shin = |s: &Sampler| Vec3::from(s.world(12).translation);
        s.layers[0].time = 0.7;
        s.sample(&m, &c);
        let before = shin(&s);
        s.crossfade(0.35);
        s.play(c.find("bf_fast_botharmsback").unwrap() as u16, 0.0);
        s.sample(&m, &c);
        assert!((shin(&s) - before).length() < 1e-3, "{} then {}", before, shin(&s));
        s.advance(&c, 0.35);
        s.sample(&m, &c);
        assert!((shin(&s) - before).length() > 0.05, "the new pose takes over");
        s.play(idle as u16, 0.0);
        // No allocation; the spec estimated 50 us.
        let t0 = std::time::Instant::now();
        for k in 0..100 {
            s.layers[0].time = k as f32 * 0.01;
            s.sample(&m, &c);
        }
        eprintln!("sample: {:.1} us a frame", t0.elapsed().as_secs_f64() * 1e4);
    }
}
