//! The Viltrumite mod's own file formats: `omniman.sevm` (the skinned model) and `clips.sevc` (its
//! animations), written once by `re/viltrumite/tools/convert.py` (docs/spec-mods-viltrumite-powers.md,
//! F14). These parsers are the game's only way in: every count, offset and index is checked and a bad file
//! is an error, never a panic and never a huge allocation (the whole size is worked out from the counts
//! and compared with the file's before anything is read).
//!
//! Everything is little-endian. Model space: metres, Z up, +Y the way he faces in a clip, +X his right
//! hand (right-handed), the origin at his pelvis (`Clip::ground_z` is where the ground is). His bind pose faces
//! -Y, because `SKEL_ROOT`'s rest rotation is 180 degrees about Z; a full clip's root track turns him to
//! +Y. Skinning is `world(bone) * inverse_bind(bone) * vertex`; a bone's world is its parent's world times its
//! local pose (position, rotation); parents always come before their children.
//!
//! ```text
//! SEVM: "SEVM", version u32 (1), bones u32, draws u32, vertices u32, indices u32,
//!       cape_first u32 (the first added cape joint, or u32::MAX), cape_chains u32, cape_links u32,
//!       bounds min [f32; 3], max [f32; 3] (bind pose, 24 bytes): 60 bytes
//!       bones x 112: name [u8; 32], parent i16 (-1 the root), tag u16 (GTA's bone id, for reference),
//!                    position [f32; 3], rotation xyzw [f32; 4], inverse bind 3 rows of [f32; 4]
//!       draws x 148: name, diffuse, normal, spec [u8; 32] each (a file stem under tex/; empty if none),
//!                    flags u32 (1 cutout, 2 the cape), first_vertex, vertex_count, first_index, index_count u32
//!       vertices x 56: position [f32; 3], normal [f32; 3], tangent [f32; 4], uv [f32; 2], joints [u8; 4],
//!                      weights [u8; 4] (adding up to 255)
//!       indices x u32, relative to their draw's first vertex
//!
//! SEVC: "SEVC", version u32 (1), clips u32, bones u32 (the model's), fps f32, tracks u32, events u32,
//!       quats u32, roots u32: 36 bytes
//!       clips x 124: name [u8; 64], flags u32 (1 loop, 2 partial, 4 stand-in, 8 lean), frames u32, duration f32,
//!                    ground_z f32, stride_speed f32 (m/s, 0 if it doesn't walk), first_track, track_count,
//!                    first_root, root_count (0, 1 or frames), first_event, event_count u32,
//!                    mask [u8; 16] (bit n: this clip has a rotation track for bone n)
//!       tracks x 8: bone u16, count u16 (1 constant, or frames), first u32 (into the quats)
//!       events x 16: name [u8; 12], time f32 (s)
//!       quats x 8: xyzw [i16; 4], each / 32767; neighbours in a track have the same sign
//!       roots x 12: [f32; 3], SKEL_ROOT's position
//! ```
//!
//! Sampling a clip: frames are `1 / fps` apart and `duration` is `(frames - 1) / fps`; a looping clip's
//! last frame equals its first, so `t = duration` and `t = 0` are the same pose. A full clip starts from the
//! bones' local rest poses and overlays its tracks; a partial clip (`mask`) only overlays what lies under it.
//!
//! A lean clip (`CLIP_LEAN`, 14 of them) doesn't play in time: its time is how far he leans, one way, from about
//! the base pose at frame 0 to the full lean at the last frame. Sample it at `t = lean * duration`, with `lean` 0..1
//! from the steering, and let the base clip under it carry the looping motion. Its frame 0 is near the base pose,
//! not on it, so its weight should reach 0 with the lean.
//!
//! The props are the hero models' `SEHM` format (`scatter/library.rs`); [`check_sehm`] is the check on them.

use glam::{Mat4, Quat, Vec3, Vec4};
use thiserror::Error;

/// Bones in the model: 98 of his skeleton and 30 for the cape. A joint index is a `u8` and the shader's
/// palette has this many entries.
pub const MAX_BONES: usize = 128;
/// No cape in the file.
pub const NO_CAPE: u32 = u32::MAX;

pub const DRAW_CUTOUT: u32 = 1;
pub const DRAW_CAPE: u32 = 2;

pub const CLIP_LOOP: u32 = 1;
pub const CLIP_PARTIAL: u32 = 2;
pub const CLIP_STAND_IN: u32 = 4;
/// Its time is how far he leans, not time: see the file's comment.
pub const CLIP_LEAN: u32 = 8;

const MODEL_HEADER: usize = 60;
const BONE_SIZE: usize = 112;
const DRAW_SIZE: usize = 148;
pub const VERTEX_SIZE: usize = 56;
const CLIPS_HEADER: usize = 36;
const CLIP_NAME: usize = 64;
const CLIP_SIZE: usize = CLIP_NAME + 44 + 16;
const TRACK_SIZE: usize = 8;
const EVENT_SIZE: usize = 16;
const QUAT_SIZE: usize = 8;
const ROOT_SIZE: usize = 12;

#[derive(Debug, Error, PartialEq)]
pub enum HeroAssetError {
    #[error("not a {0} file")]
    Magic(&'static str),
    #[error("version {0} is not known")]
    Version(u32),
    #[error("size is {found} bytes, its counts say {expected}")]
    Size { expected: u64, found: u64 },
    #[error("{0}")]
    Bad(String),
}

fn bad(what: impl Into<String>) -> HeroAssetError {
    HeroAssetError::Bad(what.into())
}

/// A bone of the model.
#[derive(Clone, Debug, PartialEq)]
pub struct HeroBone {
    pub name: String,
    /// `None` for the root. Always a lower index than the bone's own.
    pub parent: Option<u16>,
    pub tag: u16,
    pub position: Vec3,
    pub rotation: Quat,
    pub inverse_bind: Mat4,
}

/// One material's part of the mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct HeroDraw {
    pub name: String,
    pub diffuse: String,
    pub normal: String,
    pub spec: String,
    pub flags: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub first_index: u32,
    pub index_count: u32,
}

impl HeroDraw {
    pub fn cutout(&self) -> bool {
        self.flags & DRAW_CUTOUT != 0
    }

    pub fn cape(&self) -> bool {
        self.flags & DRAW_CAPE != 0
    }
}

/// A vertex, as stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeroVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv: [f32; 2],
    pub joints: [u8; 4],
    pub weights: [u8; 4],
}

/// `omniman.sevm`.
#[derive(Clone, Debug, PartialEq)]
pub struct HeroModel {
    pub bones: Vec<HeroBone>,
    pub draws: Vec<HeroDraw>,
    pub vertices: Vec<HeroVertex>,
    /// Relative to their draw's first vertex.
    pub indices: Vec<u32>,
    /// The first added cape joint: `cape_chains` chains of `cape_links` joints from here, a chain's first
    /// joint the child of a body bone (it stays rigid with it), each next one the child of the one before.
    pub cape_first: Option<u32>,
    pub cape_chains: u32,
    pub cape_links: u32,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

/// One bone's rotations in a clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipTrack {
    pub bone: u16,
    /// 1 for a constant, or the clip's frame count.
    pub count: u16,
    pub first: u32,
}

/// A named moment in a clip: `contact`, `hit`, `hold`, `clap`, `launch`, `foot_l`, `foot_r`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEvent {
    pub name: String,
    pub time: f32,
}

/// A clip's record; its data is in the [`ClipSet`]'s pools.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// Its real name: the `.ycd` file's stem (`export@nib@super@basicflight_idle`), or the Mixamo file's
    /// (`walk`).
    pub name: String,
    pub flags: u32,
    pub frames: u32,
    pub duration: f32,
    /// Model-space height of the ground under the clip: its lowest sole over all its frames (0 for a partial clip).
    pub ground_z: f32,
    /// How fast the planted foot moves back (m/s): play at `speed / stride_speed`. 0 if it doesn't walk.
    pub stride_speed: f32,
    pub first_track: u32,
    pub track_count: u32,
    pub first_root: u32,
    pub root_count: u32,
    pub first_event: u32,
    pub event_count: u32,
    /// Bit n: this clip has a rotation track for bone n.
    pub mask: u128,
}

impl Clip {
    pub fn looping(&self) -> bool {
        self.flags & CLIP_LOOP != 0
    }

    pub fn partial(&self) -> bool {
        self.flags & CLIP_PARTIAL != 0
    }

    pub fn stand_in(&self) -> bool {
        self.flags & CLIP_STAND_IN != 0
    }

    /// Sampled at `lean * duration`, never played: see the file's comment.
    pub fn lean(&self) -> bool {
        self.flags & CLIP_LEAN != 0
    }
}

/// `clips.sevc`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipSet {
    pub fps: f32,
    pub bone_count: u32,
    pub clips: Vec<Clip>,
    pub tracks: Vec<ClipTrack>,
    pub events: Vec<ClipEvent>,
    pub quats: Vec<[i16; 4]>,
    pub roots: Vec<[f32; 3]>,
}

impl ClipSet {
    /// A clip's index by its real name, or short: `basicflight_idle` or `bf_idle` for
    /// `export@nib@super@basicflight_idle`, `male@jump_fall_loop` for `export@nib@male@jump_fall_loop`.
    pub fn find(&self, name: &str) -> Option<usize> {
        let by = |full: &str| self.clips.iter().position(|c| c.name == full);
        by(name).or_else(|| by(&format!("export@nib@super@{name}"))).or_else(|| by(&format!("export@nib@{name}"))).or_else(|| {
            let rest = name.strip_prefix("bf_")?;
            by(&format!("export@nib@super@basicflight_{rest}"))
        })
    }

    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.find(name).map(|i| &self.clips[i])
    }

    pub fn tracks_of(&self, clip: &Clip) -> &[ClipTrack] {
        &self.tracks[clip.first_track as usize..(clip.first_track + clip.track_count) as usize]
    }

    pub fn events_of(&self, clip: &Clip) -> &[ClipEvent] {
        &self.events[clip.first_event as usize..(clip.first_event + clip.event_count) as usize]
    }

    /// A track's rotation at a frame (clamped to the track), normalized. The sign follows the track's
    /// neighbours; blending two tracks still needs a sign fix.
    pub fn rotation(&self, track: &ClipTrack, frame: u32) -> Quat {
        let i = frame.min(track.count.saturating_sub(1) as u32);
        let q = self.quats[(track.first + i) as usize];
        Quat::from_xyzw(q[0] as f32 / 32767.0, q[1] as f32 / 32767.0, q[2] as f32 / 32767.0, q[3] as f32 / 32767.0).normalize()
    }

    /// `SKEL_ROOT`'s position at a frame; `None` if the clip has no root track.
    pub fn root(&self, clip: &Clip, frame: u32) -> Option<Vec3> {
        if clip.root_count == 0 {
            return None;
        }
        let i = frame.min(clip.root_count - 1);
        Some(Vec3::from_array(self.roots[(clip.first_root + i) as usize]))
    }

    /// How many clips are stand-ins for the Mixamo ones (spec, U1).
    pub fn stand_ins(&self) -> usize {
        self.clips.iter().filter(|c| c.stand_in()).count()
    }
}

/// Reads fixed-size fields from a slice whose length has been checked against the counts already.
struct Fields<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Fields<'a> {
    fn new(b: &'a [u8], at: usize) -> Self {
        Fields { b, at }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], HeroAssetError> {
        let end = self.at.checked_add(n).ok_or_else(|| bad("offset overflow"))?;
        let s = self.b.get(self.at..end).ok_or_else(|| bad("field past the end"))?;
        self.at = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, HeroAssetError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i16(&mut self) -> Result<i16, HeroAssetError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, HeroAssetError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// A float that must be a real number.
    fn f32(&mut self) -> Result<f32, HeroAssetError> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if v.is_finite() { Ok(v) } else { Err(bad("a float is not finite")) }
    }

    fn f32s<const N: usize>(&mut self) -> Result<[f32; N], HeroAssetError> {
        let mut a = [0.0; N];
        for v in &mut a {
            *v = self.f32()?;
        }
        Ok(a)
    }

    /// A zero-padded name of ASCII.
    fn name(&mut self, n: usize) -> Result<String, HeroAssetError> {
        let s = self.take(n)?;
        let end = s.iter().position(|&c| c == 0).ok_or_else(|| bad("a name has no end"))?;
        if !s[..end].iter().all(|c| (0x20..0x7f).contains(c)) {
            return Err(bad("a name is not ASCII text"));
        }
        Ok(String::from_utf8_lossy(&s[..end]).into_owned())
    }
}

/// A header's counts must add up to the file's size, to the byte.
fn check_size(found: usize, parts: &[(u32, usize)], header: usize) -> Result<(), HeroAssetError> {
    let expected = parts.iter().fold(header as u64, |sum, &(n, size)| sum + n as u64 * size as u64);
    if expected != found as u64 {
        return Err(HeroAssetError::Size { expected, found: found as u64 });
    }
    Ok(())
}

fn check_magic(b: &[u8], magic: &'static str) -> Result<(), HeroAssetError> {
    if b.get(..4) != Some(magic.as_bytes()) {
        return Err(HeroAssetError::Magic(magic));
    }
    let version = Fields::new(b, 4).u32().map_err(|_| HeroAssetError::Magic(magic))?;
    if version != 1 {
        return Err(HeroAssetError::Version(version));
    }
    Ok(())
}

/// Parses `omniman.sevm`.
pub fn parse_model(b: &[u8]) -> Result<HeroModel, HeroAssetError> {
    check_magic(b, "SEVM")?;
    let mut h = Fields::new(b, 8);
    let (nb, nd, nv, ni) = (h.u32()?, h.u32()?, h.u32()?, h.u32()?);
    let (cape_first, cape_chains, cape_links) = (h.u32()?, h.u32()?, h.u32()?);
    let bounds_min = Vec3::from_array(h.f32s()?);
    let bounds_max = Vec3::from_array(h.f32s()?);
    if nb == 0 || nb as usize > MAX_BONES {
        return Err(bad(format!("{nb} bones (1 to {MAX_BONES})")));
    }
    check_size(b.len(), &[(nb, BONE_SIZE), (nd, DRAW_SIZE), (nv, VERTEX_SIZE), (ni, 4)], MODEL_HEADER)?;
    let cape = if cape_first == NO_CAPE {
        None
    } else {
        let end = (cape_chains as u64) * (cape_links as u64) + cape_first as u64;
        if end > nb as u64 {
            return Err(bad("the cape's joints run past the bones"));
        }
        Some(cape_first)
    };

    let mut f = Fields::new(b, MODEL_HEADER);
    let mut bones = Vec::with_capacity(nb as usize);
    for i in 0..nb as usize {
        let name = f.name(32)?;
        let parent = f.i16()?;
        let tag = f.u16()?;
        let position = Vec3::from_array(f.f32s()?);
        let q: [f32; 4] = f.f32s()?;
        let rows: [f32; 12] = f.f32s()?;
        let parent = match parent {
            -1 => None,
            p if p >= 0 && (p as usize) < i => Some(p as u16),
            p => return Err(bad(format!("bone {i}'s parent is {p}"))),
        };
        if (i == 0) != parent.is_none() {
            return Err(bad(format!("bone {i}: only the first bone is the root")));
        }
        let rotation = Quat::from_array(q);
        if (rotation.length_squared() - 1.0).abs() > 0.01 {
            return Err(bad(format!("bone {i}'s rotation is not a unit quaternion")));
        }
        let inverse_bind = Mat4::from_cols(
            Vec4::new(rows[0], rows[4], rows[8], 0.0),
            Vec4::new(rows[1], rows[5], rows[9], 0.0),
            Vec4::new(rows[2], rows[6], rows[10], 0.0),
            Vec4::new(rows[3], rows[7], rows[11], 1.0),
        );
        bones.push(HeroBone { name, parent, tag, position, rotation, inverse_bind });
    }

    let mut draws = Vec::with_capacity(nd as usize);
    for i in 0..nd {
        let (name, diffuse, normal, spec) = (f.name(32)?, f.name(32)?, f.name(32)?, f.name(32)?);
        let (flags, first_vertex, vertex_count, first_index, index_count) = (f.u32()?, f.u32()?, f.u32()?, f.u32()?, f.u32()?);
        if first_vertex as u64 + vertex_count as u64 > nv as u64 {
            return Err(bad(format!("draw {i}'s vertices run past the end")));
        }
        if first_index as u64 + index_count as u64 > ni as u64 || index_count % 3 != 0 {
            return Err(bad(format!("draw {i}'s indices are wrong")));
        }
        draws.push(HeroDraw { name, diffuse, normal, spec, flags, first_vertex, vertex_count, first_index, index_count });
    }

    let mut vertices = Vec::with_capacity(nv as usize);
    for i in 0..nv {
        let position = f.f32s()?;
        let normal = f.f32s()?;
        let tangent = f.f32s()?;
        let uv = f.f32s()?;
        let joints: [u8; 4] = f.take(4)?.try_into().unwrap();
        let weights: [u8; 4] = f.take(4)?.try_into().unwrap();
        let sum: u32 = weights.iter().map(|&w| w as u32).sum();
        if !(250..=260).contains(&sum) {
            return Err(bad(format!("vertex {i}'s weights add up to {sum}")));
        }
        if joints.iter().zip(&weights).any(|(&j, &w)| w > 0 && j as u32 >= nb) {
            return Err(bad(format!("vertex {i} has a joint past the bones")));
        }
        vertices.push(HeroVertex { position, normal, tangent, uv, joints, weights });
    }

    let mut indices = Vec::with_capacity(ni as usize);
    for _ in 0..ni {
        indices.push(f.u32()?);
    }
    for (i, d) in draws.iter().enumerate() {
        let range = d.first_index as usize..(d.first_index + d.index_count) as usize;
        if indices[range].iter().any(|&x| x >= d.vertex_count) {
            return Err(bad(format!("draw {i} has an index past its vertices")));
        }
    }
    Ok(HeroModel { bones, draws, vertices, indices, cape_first: cape, cape_chains, cape_links, bounds_min, bounds_max })
}

/// Parses `clips.sevc`.
pub fn parse_clips(b: &[u8]) -> Result<ClipSet, HeroAssetError> {
    check_magic(b, "SEVC")?;
    let mut h = Fields::new(b, 8);
    let (nc, bone_count) = (h.u32()?, h.u32()?);
    let fps = h.f32()?;
    let (nt, ne, nq, nr) = (h.u32()?, h.u32()?, h.u32()?, h.u32()?);
    if bone_count == 0 || bone_count as usize > MAX_BONES {
        return Err(bad(format!("{bone_count} bones (1 to {MAX_BONES})")));
    }
    if !(1.0..=240.0).contains(&fps) {
        return Err(bad(format!("{fps} frames a second")));
    }
    check_size(b.len(), &[(nc, CLIP_SIZE), (nt, TRACK_SIZE), (ne, EVENT_SIZE), (nq, QUAT_SIZE), (nr, ROOT_SIZE)], CLIPS_HEADER)?;

    let mut f = Fields::new(b, CLIPS_HEADER);
    let mut clips = Vec::with_capacity(nc as usize);
    for _ in 0..nc {
        let name = f.name(CLIP_NAME)?;
        let flags = f.u32()?;
        let frames = f.u32()?;
        let duration = f.f32()?;
        let ground_z = f.f32()?;
        let stride_speed = f.f32()?;
        let (first_track, track_count, first_root, root_count, first_event, event_count) = (f.u32()?, f.u32()?, f.u32()?, f.u32()?, f.u32()?, f.u32()?);
        let mask = u128::from_le_bytes(f.take(16)?.try_into().unwrap());
        let wrong = |what: &str| bad(format!("clip {name}: {what}"));
        if frames == 0 || frames > u16::MAX as u32 {
            return Err(wrong("frames"));
        }
        if (duration - (frames - 1) as f32 / fps).abs() > 1e-3 {
            return Err(wrong("duration"));
        }
        if stride_speed < 0.0 {
            return Err(wrong("stride speed"));
        }
        if first_track as u64 + track_count as u64 > nt as u64 {
            return Err(wrong("tracks"));
        }
        if first_event as u64 + event_count as u64 > ne as u64 {
            return Err(wrong("events"));
        }
        if !(root_count == 0 || root_count == 1 || root_count == frames) || first_root as u64 + root_count as u64 > nr as u64 {
            return Err(wrong("root"));
        }
        clips.push(Clip { name, flags, frames, duration, ground_z, stride_speed, first_track, track_count, first_root, root_count, first_event, event_count, mask });
    }

    let mut tracks = Vec::with_capacity(nt as usize);
    for i in 0..nt {
        let (bone, count, first) = (f.u16()?, f.u16()?, f.u32()?);
        if bone as u32 >= bone_count {
            return Err(bad(format!("track {i}: bone {bone}")));
        }
        if count == 0 || first as u64 + count as u64 > nq as u64 {
            return Err(bad(format!("track {i}: samples past the end")));
        }
        tracks.push(ClipTrack { bone, count, first });
    }
    for c in &clips {
        let mut mask = 0u128;
        for t in &tracks[c.first_track as usize..(c.first_track + c.track_count) as usize] {
            if t.count != 1 && t.count as u32 != c.frames {
                return Err(bad(format!("clip {}: bone {}'s track has {} samples", c.name, t.bone, t.count)));
            }
            mask |= 1u128 << t.bone;
        }
        if mask != c.mask {
            return Err(bad(format!("clip {}: its mask isn't its tracks", c.name)));
        }
    }

    let mut events = Vec::with_capacity(ne as usize);
    for _ in 0..ne {
        let name = f.name(12)?;
        let time = f.f32()?;
        events.push(ClipEvent { name, time });
    }
    for c in &clips {
        let of_clip = &events[c.first_event as usize..(c.first_event + c.event_count) as usize];
        if of_clip.iter().any(|e| e.time < 0.0 || e.time > c.duration + 1e-3) {
            return Err(bad(format!("clip {}: an event outside the clip", c.name)));
        }
    }

    let mut quats = Vec::with_capacity(nq as usize);
    for i in 0..nq {
        let q = [f.i16()?, f.i16()?, f.i16()?, f.i16()?];
        let len2: f32 = q.iter().map(|&v| (v as f32 / 32767.0).powi(2)).sum();
        if !(0.9..=1.1).contains(&len2) {
            return Err(bad(format!("quaternion {i} is not a unit one")));
        }
        quats.push(q);
    }
    let mut roots = Vec::with_capacity(nr as usize);
    for _ in 0..nr {
        roots.push(f.f32s()?);
    }
    Ok(ClipSet { fps, bone_count, clips, tracks, events, quats, roots })
}

/// The model and its clips must have been written for each other.
pub fn check_pair(model: &HeroModel, clips: &ClipSet) -> Result<(), HeroAssetError> {
    if model.bones.len() != clips.bone_count as usize {
        return Err(bad(format!("the model has {} bones, the clips {}", model.bones.len(), clips.bone_count)));
    }
    Ok(())
}

/// Whether a prop's file can be drawn: `SEHM`, version 1, sizes that are real, as many bytes as its counts say,
/// finite vertices, whole triangles, and indices that stay on its vertices. Layout: `scatter/library.rs` `hero_file`.
pub fn check_sehm(b: &[u8]) -> Result<(), HeroAssetError> {
    const HEADER: usize = 28;
    const VERTEX: usize = 32;
    check_magic(b, "SEHM")?;
    let mut f = Fields::new(b, 8);
    if b.len() < HEADER {
        return Err(HeroAssetError::Size { expected: HEADER as u64, found: b.len() as u64 });
    }
    let size: [f32; 3] = f.f32s()?;
    let (nv, ni) = (f.u32()?, f.u32()?);
    check_size(b.len(), &[(nv, VERTEX), (ni, 4)], HEADER)?;
    if size.iter().any(|s| !s.is_finite() || *s <= 0.0) {
        return Err(bad("its size"));
    }
    if ni % 3 != 0 {
        return Err(bad("its triangles"));
    }
    for i in 0..nv {
        let v: [f32; 8] = f.f32s()?;
        if v.iter().any(|x| !x.is_finite()) {
            return Err(bad(format!("vertex {i}")));
        }
    }
    for i in 0..ni {
        if f.u32()? >= nv {
            return Err(bad(format!("index {i}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn name(out: &mut Vec<u8>, s: &str, n: usize) {
        let mut b = s.as_bytes().to_vec();
        b.resize(n, 0);
        out.extend_from_slice(&b);
    }

    fn floats(out: &mut Vec<u8>, v: &[f32]) {
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn u32s(out: &mut Vec<u8>, v: &[u32]) {
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }

    /// A small good model: a root and a child, one triangle.
    fn tiny_model() -> Vec<u8> {
        let mut b = b"SEVM".to_vec();
        u32s(&mut b, &[1, 2, 1, 3, 3, NO_CAPE, 0, 0]);
        floats(&mut b, &[-1.0, -1.0, -1.0, 1.0, 1.0, 1.0]);
        for (i, parent) in [(0, -1i16), (1, 0)] {
            name(&mut b, if i == 0 { "SKEL_ROOT" } else { "SKEL_Pelvis" }, 32);
            b.extend_from_slice(&parent.to_le_bytes());
            b.extend_from_slice(&(i as u16).to_le_bytes());
            floats(&mut b, &[0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0]);
            floats(&mut b, &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, -0.5]);
        }
        name(&mut b, "body", 32);
        name(&mut b, "body_diff", 32);
        name(&mut b, "body_normal", 32);
        name(&mut b, "", 32);
        u32s(&mut b, &[DRAW_CUTOUT, 0, 3, 0, 3]);
        for j in 0..3 {
            floats(&mut b, &[j as f32, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.5, 0.5]);
            b.extend_from_slice(&[0, 1, 0, 0, 200, 55, 0, 0]);
        }
        u32s(&mut b, &[0, 1, 2]);
        b
    }

    /// A small good clip file: one looping clip with a moving track and a constant one, a root and an event.
    fn tiny_clips() -> Vec<u8> {
        let mut b = b"SEVC".to_vec();
        u32s(&mut b, &[1, 1, 2]);
        floats(&mut b, &[30.0]);
        u32s(&mut b, &[2, 1, 4, 3]);
        name(&mut b, "export@nib@super@basicflight_idle", CLIP_NAME);
        u32s(&mut b, &[CLIP_LOOP, 3]);
        floats(&mut b, &[2.0 / 30.0, -1.0, 0.0]);
        u32s(&mut b, &[0, 2, 0, 3, 0, 1]);
        b.extend_from_slice(&3u128.to_le_bytes());
        for (bone, count, first) in [(0u16, 3u16, 0u32), (1, 1, 3)] {
            b.extend_from_slice(&bone.to_le_bytes());
            b.extend_from_slice(&count.to_le_bytes());
            u32s(&mut b, &[first]);
        }
        name(&mut b, "contact", 12);
        floats(&mut b, &[0.05]);
        for q in [[0i16, 0, 0, 32767], [0, 0, 23170, 23170], [0, 0, 0, 32767], [32767, 0, 0, 0]] {
            for v in q {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        floats(&mut b, &[0.0, 0.0, 0.1, 0.0, 0.0, 0.2, 0.0, 0.0, 0.1]);
        b
    }

    #[test]
    fn a_good_model_reads_back() {
        let m = parse_model(&tiny_model()).unwrap();
        assert_eq!(m.bones.len(), 2);
        assert_eq!(m.bones[0].parent, None);
        assert_eq!(m.bones[1].parent, Some(0));
        assert_eq!(m.bones[1].name, "SKEL_Pelvis");
        assert_eq!(m.bones[1].inverse_bind.w_axis.z, -0.5);
        assert_eq!(m.draws[0].diffuse, "body_diff");
        assert_eq!(m.draws[0].spec, "");
        assert!(m.draws[0].cutout());
        assert_eq!(m.vertices[2].position[0], 2.0);
        assert_eq!(m.vertices[1].joints, [0, 1, 0, 0]);
        assert_eq!(m.indices, [0, 1, 2]);
        assert_eq!(m.cape_first, None);
    }

    #[test]
    fn good_clips_read_back_and_sample() {
        let s = parse_clips(&tiny_clips()).unwrap();
        assert_eq!(s.find("bf_idle"), Some(0));
        assert_eq!(s.find("basicflight_idle"), Some(0));
        assert_eq!(s.find("export@nib@super@basicflight_idle"), Some(0));
        assert_eq!(s.find("walk"), None);
        let c = s.clip("bf_idle").unwrap();
        assert!(c.looping() && !c.partial() && !c.stand_in());
        assert_eq!(s.events_of(c)[0].name, "contact");
        let t = s.tracks_of(c);
        assert!((s.rotation(&t[0], 1).w - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        // A constant track and a frame past the end stay in bounds.
        assert_eq!(s.rotation(&t[1], 2).x, 1.0);
        assert_eq!(s.rotation(&t[0], 99), s.rotation(&t[0], 2));
        assert_eq!(s.root(c, 1), Some(Vec3::new(0.0, 0.0, 0.2)));
    }

    /// Every cut-short file is an error, from nothing to one byte short, and so is one byte too many.
    #[test]
    fn truncated_files_are_errors() {
        for (file, parse) in [(tiny_model(), (|b: &[u8]| parse_model(b).map(|_| ())) as fn(&[u8]) -> _), (tiny_clips(), |b: &[u8]| parse_clips(b).map(|_| ()))] {
            for n in 0..file.len() {
                assert!(parse(&file[..n]).is_err(), "{n} bytes");
            }
            let mut long = file.clone();
            long.push(0);
            assert!(parse(&long).is_err());
        }
    }

    fn set(b: &mut [u8], at: usize, v: &[u8]) {
        b[at..at + v.len()].copy_from_slice(v);
    }

    #[test]
    fn corrupt_models_are_errors() {
        let cases: &[(usize, &[u8], &str)] = &[
            (0, b"XEVM", "magic"),
            (4, &2u32.to_le_bytes(), "version"),
            (8, &129u32.to_le_bytes(), "too many bones"),
            (8, &0u32.to_le_bytes(), "no bones"),
            (16, &u32::MAX.to_le_bytes(), "a count that would allocate 240 GB"),
            (24, &1u32.to_le_bytes(), "a cape past the bones (with 0 chains it's fine: see below)"),
        ];
        for (at, v, why) in cases {
            let mut b = tiny_model();
            set(&mut b, *at, v);
            if *why == cases[5].2 {
                set(&mut b, 28, &5u32.to_le_bytes());
                set(&mut b, 32, &5u32.to_le_bytes());
            }
            assert!(parse_model(&b).is_err(), "{why}");
        }
        let bone = MODEL_HEADER + BONE_SIZE; // the second bone
        let draw = MODEL_HEADER + 2 * BONE_SIZE;
        let vert = draw + DRAW_SIZE;
        let index = vert + 3 * VERTEX_SIZE;
        let cases: &[(usize, Vec<u8>, &str)] = &[
            (bone + 32, 1i16.to_le_bytes().to_vec(), "a bone its own parent"),
            (bone + 32, (-2i16).to_le_bytes().to_vec(), "a negative parent"),
            (bone, vec![0xFF; 32], "a name with no end"),
            (bone + 36, f32::NAN.to_le_bytes().to_vec(), "a NaN position"),
            (bone + 48, 2f32.to_le_bytes().to_vec(), "a rotation that isn't a unit one"),
            (draw + 128 + 4, 2u32.to_le_bytes().to_vec(), "a draw past the vertices"),
            (draw + 128 + 16, 2u32.to_le_bytes().to_vec(), "a draw of 2 indices"),
            (vert + 48, vec![200, 0, 0, 0], "a joint past the bones"),
            (vert + 52, vec![10, 10, 0, 0], "weights not adding up"),
            (vert, f32::INFINITY.to_le_bytes().to_vec(), "an infinite position"),
            (index, 3u32.to_le_bytes().to_vec(), "an index past the draw"),
        ];
        for (at, v, why) in cases {
            let mut b = tiny_model();
            set(&mut b, *at, v);
            assert!(parse_model(&b).is_err(), "{why}");
        }
    }

    #[test]
    fn corrupt_clips_are_errors() {
        let clip = CLIPS_HEADER;
        let track = clip + CLIP_SIZE;
        let event = track + 2 * TRACK_SIZE;
        let quat = event + EVENT_SIZE;
        let root = quat + 4 * QUAT_SIZE;
        let cases: &[(usize, Vec<u8>, &str)] = &[
            (0, b"SEVM".to_vec(), "magic"),
            (4, 9u32.to_le_bytes().to_vec(), "version"),
            (8, u32::MAX.to_le_bytes().to_vec(), "a count that would allocate"),
            (12, 129u32.to_le_bytes().to_vec(), "too many bones"),
            (16, 0f32.to_le_bytes().to_vec(), "no frame rate"),
            (clip + CLIP_NAME + 4, 4u32.to_le_bytes().to_vec(), "more frames than its tracks hold"),
            (clip + CLIP_NAME + 4, 0u32.to_le_bytes().to_vec(), "no frames"),
            (clip + CLIP_NAME + 8, 9f32.to_le_bytes().to_vec(), "a wrong duration"),
            (clip + CLIP_NAME + 16, (-1f32).to_le_bytes().to_vec(), "a negative stride"),
            (clip + CLIP_NAME + 20, 9u32.to_le_bytes().to_vec(), "tracks past the end"),
            (clip + CLIP_NAME + 28, 9u32.to_le_bytes().to_vec(), "roots past the end"),
            (clip + CLIP_NAME + 32, 2u32.to_le_bytes().to_vec(), "a root count that isn't 0, 1 or frames"),
            (clip + CLIP_NAME + 36, 9u32.to_le_bytes().to_vec(), "events past the end"),
            (clip + CLIP_NAME + 44, 0xFFu128.to_le_bytes().to_vec(), "a mask that isn't its tracks"),
            (track, 200u16.to_le_bytes().to_vec(), "a bone past the bones"),
            (track + 4, 9u32.to_le_bytes().to_vec(), "samples past the end"),
            (track + 2, 0u16.to_le_bytes().to_vec(), "a track with no samples"),
            (track + 2, 2u16.to_le_bytes().to_vec(), "a track neither constant nor full"),
            (event + 12, 5f32.to_le_bytes().to_vec(), "an event past the clip"),
            (quat, vec![0; 8], "a zero quaternion"),
            (root, f32::NAN.to_le_bytes().to_vec(), "a NaN root"),
        ];
        for (at, v, why) in cases {
            let mut b = tiny_clips();
            set(&mut b, *at, v);
            assert!(parse_clips(&b).is_err(), "{why}");
        }
    }

    /// Flipping any byte may make a file wrong or leave it readable, but never panics.
    #[test]
    fn any_flipped_byte_never_panics() {
        for file in [tiny_model(), tiny_clips()] {
            for at in 0..file.len() {
                for x in [0x01u8, 0x80, 0xFF] {
                    let mut b = file.clone();
                    b[at] ^= x;
                    let _ = parse_model(&b);
                    let _ = parse_clips(&b);
                }
            }
        }
    }

    #[test]
    fn a_model_and_clips_for_different_skeletons_dont_pair() {
        let m = parse_model(&tiny_model()).unwrap();
        let mut c = parse_clips(&tiny_clips()).unwrap();
        assert!(check_pair(&m, &c).is_ok());
        c.bone_count = 3;
        assert!(check_pair(&m, &c).is_err());
    }

    fn real_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../se_engine/assets/mods/viltrumite")
    }

    /// The rotation part of a bone's world in the rest pose, for the model's checks.
    fn rest_world(model: &HeroModel) -> Vec<Mat4> {
        let mut world: Vec<Mat4> = Vec::with_capacity(model.bones.len());
        for b in &model.bones {
            let local = Mat4::from_rotation_translation(b.rotation, b.position);
            world.push(b.parent.map_or(local, |p| world[p as usize] * local));
        }
        world
    }

    /// The converted files in the project: what the mod really reads (spec, step 2).
    #[test]
    fn the_real_model_reads_back() {
        let b = std::fs::read(real_dir().join("omniman.sevm")).expect("omniman.sevm");
        let m = parse_model(&b).unwrap();
        assert_eq!(m.bones.len(), MAX_BONES, "98 bones of his and 30 for the cape");
        assert_eq!(m.bones[0].name, "SKEL_ROOT");
        for must in ["SKEL_Pelvis", "SKEL_Head", "SKEL_L_Foot", "SKEL_R_Hand", "SKEL_Spine3"] {
            assert!(m.bones.iter().any(|b| b.name == must), "{must}");
        }
        // The cape: 30 joints from bone 98, chains hanging from a body bone.
        assert_eq!(m.cape_first, Some(98));
        assert_eq!(m.cape_chains * m.cape_links, 30);
        for c in 0..m.cape_chains {
            for l in 0..m.cape_links {
                let i = (98 + c * m.cape_links + l) as usize;
                let p = m.bones[i].parent.unwrap() as usize;
                if l == 0 {
                    assert!(p < 98, "a chain hangs from a body bone");
                } else {
                    assert_eq!(p, i - 1, "a chain's joints follow each other");
                }
            }
        }
        // Every vertex's weights add up, on bones that exist; every index stays in its draw (the parser
        // checks a range: the writer's exact).
        for v in &m.vertices {
            assert_eq!(v.weights.iter().map(|&w| w as u32).sum::<u32>(), 255);
            let n = Vec3::from_array(v.normal).length();
            assert!((n - 1.0).abs() < 0.02, "normal length {n}");
        }
        // Rest pose * inverse bind is the identity for every bone: the skin stays where the mesh was made.
        let world = rest_world(&m);
        for (i, b) in m.bones.iter().enumerate() {
            let id = world[i] * b.inverse_bind;
            assert!(id.abs_diff_eq(Mat4::IDENTITY, 1e-3), "{}: {id:?}", b.name);
        }
        // He's about 1.88 m tall, his feet below his origin.
        let height = m.bounds_max.z - m.bounds_min.z;
        assert!((1.8..1.95).contains(&height), "height {height}");
        assert!(m.bounds_min.z < -0.9);
        // Seven materials; the hair and its kin are cut out, the cape is marked, the rest opaque.
        assert_eq!(m.draws.len(), 7);
        let count = |f: &dyn Fn(&HeroDraw) -> bool| m.draws.iter().filter(|d| f(d)).count();
        assert_eq!(count(&|d| d.cutout()), 3);
        assert_eq!(count(&|d| d.cape()), 1);
        // Draws cover the buffers exactly, in order.
        let (mut fv, mut fi) = (0, 0);
        for d in &m.draws {
            assert_eq!((d.first_vertex, d.first_index), (fv, fi), "{}", d.name);
            fv += d.vertex_count;
            fi += d.index_count;
            for t in [&d.diffuse, &d.normal, &d.spec] {
                assert!(t.is_empty() || real_dir().join("tex").join(format!("{t}.dds")).is_file(), "{t}");
            }
            assert!(!d.diffuse.is_empty(), "{}", d.name);
        }
        assert_eq!((fv as usize, fi as usize), (m.vertices.len(), m.indices.len()));
        assert!(!m.draws.iter().any(|d| d.name.contains("feet_001")), "no blood variant, no second cape");
        assert_eq!(m.indices.len(), 225_011 * 3, "the source's triangle count");
    }

    #[test]
    fn the_real_clips_read_back() {
        let m = parse_model(&std::fs::read(real_dir().join("omniman.sevm")).unwrap()).unwrap();
        let s = parse_clips(&std::fs::read(real_dir().join("clips.sevc")).expect("clips.sevc")).unwrap();
        check_pair(&m, &s).unwrap();
        assert_eq!(s.fps, 30.0);
        assert_eq!(s.clips.len(), F4_CLIPS.len());
        // Every clip the plan's state table names (F4), by the names later steps will use.
        for name in F4_CLIPS {
            let c = s.clip(name).unwrap_or_else(|| panic!("clip {name} is missing"));
            assert!(c.frames >= 1, "{name}");
            if !c.partial() {
                assert!(c.root_count > 0, "{name}: a full clip moves the root");
                assert!(c.mask & 1 != 0, "{name}: a full clip turns the root");
            }
        }
        for (name, flags) in [
            ("bf_idle", CLIP_LOOP),
            ("male@jump_fall_loop", CLIP_LOOP),
            ("fall", 0),
            ("fall_loop", CLIP_LOOP),
            ("walk", CLIP_LOOP),
            ("bf_idle_left_add", CLIP_PARTIAL | CLIP_LEAN),
            ("bf_fast_botharmsback_left", CLIP_LEAN),
            ("land_punch", 0),
        ] {
            let c = s.clip(name).unwrap();
            assert_eq!(c.flags & (CLIP_LOOP | CLIP_PARTIAL | CLIP_LEAN), flags, "{name}");
        }
        for (name, event, time) in [("land_punch", "contact", 0.27), ("ground_punch", "contact", 0.27), ("thunderclap", "hold", 0.6), ("thunderclap", "clap", 1.23)] {
            let c = s.clip(name).unwrap();
            let e = s.events_of(c).iter().find(|e| e.name == event).unwrap_or_else(|| panic!("{name} {event}"));
            assert!((e.time - time).abs() < 0.01, "{name} {event} {}", e.time);
        }
        for name in ["walk", "run", "sprint"] {
            let c = s.clip(name).unwrap();
            assert!(c.stride_speed > 0.5, "{name}: stride {}", c.stride_speed);
            assert!(c.looping());
            assert!(s.events_of(c).iter().any(|e| e.name == "foot_l"), "{name} starts on left-foot contact");
        }
        assert!(s.clip("walk").unwrap().stride_speed < s.clip("run").unwrap().stride_speed);
        assert!(s.clip("run").unwrap().stride_speed < s.clip("sprint").unwrap().stride_speed);
        let walk = s.clip("walk").unwrap();
        assert!((-1.15..-0.85).contains(&walk.ground_z), "the ground is about a metre under his pelvis, {}", walk.ground_z);
        // Every sample of every clip: a bone that exists, a unit rotation, a place near him.
        for c in &s.clips {
            for t in s.tracks_of(c) {
                assert!((t.bone as usize) < m.bones.len());
                for f in 0..t.count as u32 {
                    let q = s.quats[(t.first + f) as usize];
                    let len2: f32 = q.iter().map(|&v| (v as f32 / 32767.0).powi(2)).sum();
                    assert!((len2 - 1.0).abs() < 0.01, "{} bone {} frame {f}: {len2}", c.name, t.bone);
                }
            }
            for f in 0..c.root_count {
                let r = s.root(c, f).unwrap();
                assert!(r.abs().max_element() < 6.0, "{} root {r} at frame {f}", c.name);
            }
            // A clip's looping seam: the last frame equals the first.
            if c.looping() {
                for t in s.tracks_of(c) {
                    let (a, b) = (s.quats[t.first as usize], s.quats[(t.first + t.count as u32 - 1) as usize]);
                    let dot: f32 = a.iter().zip(&b).map(|(&x, &y)| x as f32 * y as f32 / (32767.0 * 32767.0)).sum();
                    assert!(dot.abs() > 0.999, "{} bone {} seam: {dot}", c.name, t.bone);
                }
            }
        }
    }

    /// The angle between a track's samples at two frames, degrees.
    fn turn(s: &ClipSet, t: &ClipTrack, a: u32, b: u32) -> f32 {
        s.rotation(t, a).angle_between(s.rotation(t, b)).to_degrees()
    }

    fn real_clips() -> (HeroModel, ClipSet) {
        let m = parse_model(&std::fs::read(real_dir().join("omniman.sevm")).unwrap()).unwrap();
        (m, parse_clips(&std::fs::read(real_dir().join("clips.sevc")).unwrap()).unwrap())
    }

    /// The lean clips stay as authored, one way: looped or mirrored, a held lean would come and go, and with it the
    /// lean you steer. Each ends on its full lean, so sampling at `lean * duration` is right at both ends.
    #[test]
    fn the_lean_clips_go_one_way() {
        let (_, s) = real_clips();
        let leans: Vec<&Clip> = s.clips.iter().filter(|c| c.lean()).collect();
        assert_eq!(leans.len(), 14);
        for c in leans {
            assert!(!c.looping(), "{}", c.name);
            assert_eq!(c.frames, 101, "{}: mirrored? {} frames", c.name, c.frames);
            assert_eq!(c.partial(), c.name.ends_with("_add"), "{}", c.name);
            let reach = s.tracks_of(c).iter().map(|t| turn(&s, t, 0, c.frames - 1)).fold(0.0, f32::max);
            assert!(reach > 4.0, "{}: only {reach} degrees from its first frame to its last", c.name);
        }
    }

    /// `fall` settles and goes on as `fall_loop`: same bones, no jump where one meets the other, and the loop barely
    /// moves. Mirrored, as it was, the limbs swung 42 degrees out and back every 11 s of a long fall.
    #[test]
    fn the_fall_settles_into_its_own_loop() {
        let (_, s) = real_clips();
        let (fall, held) = (s.clip("fall").unwrap(), s.clip("fall_loop").unwrap());
        assert_eq!((fall.frames, fall.looping()), (171, false));
        assert_eq!((held.frames, held.looping()), (101, true));
        assert_eq!(fall.mask, held.mask);
        let (settle, loop_tracks) = (s.tracks_of(fall), s.tracks_of(held));
        let mut jump = 0.0f32;
        for (a, b) in settle.iter().zip(loop_tracks) {
            assert_eq!(a.bone, b.bone);
            jump = jump.max(s.rotation(a, fall.frames - 1).angle_between(s.rotation(b, 0)).to_degrees());
            for f in 0..held.frames {
                assert!(turn(&s, b, 0, f) < 8.0, "bone {} frame {f}", b.bone);
            }
        }
        assert!(jump < 0.2, "{jump} degrees where `fall` hands over");
        let gap = s.root(fall, fall.frames - 1).unwrap() - s.root(held, 0).unwrap();
        assert!(gap.length() < 0.002, "{gap}");
    }

    /// No bone goes right round between two close poses (`bf_idle_to_fast`'s left thumb did: 362 degrees of path
    /// for 8 of turn, in 0.2 s). The limit is far over the mod's clips' real fast limbs. The walks are left out: a
    /// sprint's legs swing there and back inside the window, which is no whirl.
    #[test]
    fn no_bone_whirls() {
        const WINDOW: u32 = 8;
        let (_, s) = real_clips();
        for c in s.clips.iter().filter(|c| c.name.starts_with("export@")) {
            for t in s.tracks_of(c).iter().filter(|t| t.count as u32 > WINDOW) {
                for f in 0..c.frames - WINDOW {
                    let path: f32 = (f..f + WINDOW).map(|k| turn(&s, t, k, k + 1)).sum();
                    let extra = path - turn(&s, t, f, f + WINDOW);
                    assert!(extra < 180.0, "{} bone {} frame {f}: {path} degrees of path, {extra} over its turn", c.name, t.bone);
                }
            }
        }
    }

    /// The stand-in idle's breath: its chest and neck turn under a degree or so, and must not read as "at rest".
    #[test]
    fn the_stand_in_idle_breathes() {
        let (m, s) = real_clips();
        let idle = s.clip("idle").unwrap();
        for (bone, least) in [("SKEL_Spine2", 0.2), ("SKEL_Spine3", 0.5), ("SKEL_Neck_1", 0.1)] {
            let i = m.bones.iter().position(|b| b.name == bone).unwrap() as u16;
            let t = s.tracks_of(idle).iter().find(|t| t.bone == i).unwrap_or_else(|| panic!("{bone} has no track"));
            let most = (0..idle.frames).map(|f| turn(&s, t, 0, f)).fold(0.0, f32::max);
            assert!(most > least, "{bone} moves {most} degrees");
        }
    }

    /// Every texture the draws name and the props' and sprites': real BC3 with whole mip chains, read by the
    /// parser the game's textures go through. No top mip is smaller than 64, so block sizes stay sane.
    #[test]
    fn the_real_textures_parse() {
        let m = parse_model(&std::fs::read(real_dir().join("omniman.sevm")).unwrap()).unwrap();
        let mut files: Vec<String> = m.draws.iter().flat_map(|d| [&d.diffuse, &d.normal, &d.spec]).map(|t| format!("tex/{t}.dds")).collect();
        files.sort();
        files.dedup();
        files.extend(["props/rock.dds".to_string(), "props/rock_N.dds".to_string(), "sprites/ptfx_smoke_wispy_anim.dds".to_string()]);
        for rel in &files {
            let bytes = std::fs::read(real_dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
            let info = crate::dds::parse_dds(&bytes).unwrap_or_else(|e| panic!("{rel}: {e}"));
            assert_eq!(info.format, crate::dds::DdsFormat::Bc3, "{rel}");
            assert!(info.width >= 64 && info.height >= 64 && info.mips >= 1, "{rel}: {}x{} {} mips", info.width, info.height, info.mips);
            assert_eq!(bytes.len(), info.data_offset + info.data_bytes(), "{rel}: its size");
        }
    }

    /// The props in the hero models' format (scatter/library.rs `hero_file`): a unit box, real sizes in the header.
    #[test]
    fn the_real_props_read_back() {
        for name in ["nibsuper_ground_break1", "nibsuper_rock1", "nibsuper_rock2"] {
            let b = std::fs::read(real_dir().join("props").join(format!("{name}.sehm"))).unwrap();
            check_sehm(&b).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&b[..4], b"SEHM", "{name}");
            let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            assert_eq!(u(4), 1, "{name}");
            let (nv, ni) = (u(20) as usize, u(24) as usize);
            assert_eq!(b.len(), 28 + nv * 32 + ni * 4, "{name}");
            assert!(f(8) > 0.5 && f(12) > 0.5 && f(16) > 0.5, "{name}: its real size");
            for i in 0..nv {
                let o = 28 + i * 32;
                assert!((-0.5001..=0.5001).contains(&f(o)) && (-0.0001..=1.0001).contains(&f(o + 4)) && (-0.5001..=0.5001).contains(&f(o + 8)), "{name}: vertex {i}");
                let n = Vec3::new(f(o + 12), f(o + 16), f(o + 20)).length();
                assert!((n - 1.0).abs() < 0.05, "{name}: normal {n}");
            }
            let o = 28 + nv * 32;
            for i in 0..ni {
                assert!((u(o + i * 4) as usize) < nv, "{name}: index {i}");
            }
        }
    }

    /// A prop's file that's wrong in any way is an error, never a panic.
    #[test]
    fn check_sehm_refuses_what_cant_be_drawn() {
        let mut good = b"SEHM".to_vec();
        u32s(&mut good, &[1]);
        floats(&mut good, &[2.0, 3.0, 4.0]);
        u32s(&mut good, &[3, 3]);
        for i in 0..3 {
            floats(&mut good, &[0.1 * i as f32, 0.2, 0.3, 0.0, 1.0, 0.0, 0.5, 0.5]);
        }
        u32s(&mut good, &[0, 1, 2]);
        check_sehm(&good).unwrap();
        for n in 0..good.len() {
            assert!(check_sehm(&good[..n]).is_err(), "cut at {n}");
        }
        for (at, bytes, what) in [
            (0, b"NOPE".as_slice(), "magic"),
            (4, &9u32.to_le_bytes(), "version"),
            (8, &(-1.0f32).to_le_bytes(), "size"),
            (20, &u32::MAX.to_le_bytes(), "vertex count"),
            (24, &u32::MAX.to_le_bytes(), "index count"),
            (28, &f32::NAN.to_le_bytes(), "a vertex"),
            (good.len() - 4, &3u32.to_le_bytes(), "an index past the vertices"),
        ] {
            let mut b = good.clone();
            set(&mut b, at, bytes);
            assert!(check_sehm(&b).is_err(), "{what}");
        }
        let mut b = good.clone();
        b.extend_from_slice(&[0, 0, 0, 0]);
        assert!(check_sehm(&b).is_err(), "bytes past the counts");
    }

    /// What must never be in the copy (spec, C4).
    #[test]
    fn nothing_forbidden_is_in_the_copy() {
        fn walk(dir: &Path, names: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), names);
                } else {
                    names.push(e.file_name().to_string_lossy().to_lowercase());
                }
            }
        }
        let mut names = Vec::new();
        walk(&real_dir(), &mut names);
        assert!(names.len() > 50);
        for n in &names {
            for banned in ["blood", "feet_001", "laser", "freeze", "tornado", "solar", ".ndll", ".asi", ".dll", ".ydd", ".ycd", ".ytd", ".ydr", ".ypt", ".wav"] {
                assert!(!n.contains(banned), "{n}");
            }
        }
    }

    /// The clips the plan's F4 table names, by the short names later steps use.
    const F4_CLIPS: &[&str] = &[
        "idle", "turn_left", "turn_right", "walk", "run", "sprint", "punch_cross", "punch_hook",
        "male@jump_fall_start_stopped", "male@jump_fall_loop", "fall", "fall_loop", "male@jump_fall_down_loop", "male@jump_fall_land_hard", "land_r",
        "land_punch", "bf_idle", "bf_idle_arms_crossed", "bf_idle_fwd_add", "bf_idle_bwd_add", "bf_idle_left_add", "bf_idle_right_add",
        "bf_idle_up_add", "bf_idle_down_add", "bf_idle_combat2", "bf_fast_botharmsback", "bf_fast_botharmsback_left", "bf_fast_botharmsback_right",
        "bf_fast_botharmsback_up", "bf_fast_botharmsback_down", "bf_fast_rightarmpointingahead", "bf_fast_left_add", "bf_fast_right_add",
        "bf_fast_up_add", "bf_fast_down_add", "bf_hyper_up_rightarmahead", "charge_flight_stopped", "bf_idle_to_fast", "bf_break",
        "bf_fast_to_idle", "flying_grab_punching", "flying_grab_punching3", "ground_punch", "thunderclap", "thunderclap_flying",
    ];

    /// The real files, corrupted and cut short: errors, never a panic.
    #[test]
    fn the_real_files_cut_short_or_corrupted_are_errors() {
        let model = std::fs::read(real_dir().join("omniman.sevm")).unwrap();
        let clips = std::fs::read(real_dir().join("clips.sevc")).unwrap();
        for n in [0, 3, 8, 59, 60, 61, 1000, model.len() / 2, model.len() - 1] {
            assert!(parse_model(&model[..n]).is_err(), "model cut at {n}");
        }
        for n in [0, 3, 8, 35, 36, 37, 1000, clips.len() / 2, clips.len() - 1] {
            assert!(parse_clips(&clips[..n]).is_err(), "clips cut at {n}");
        }
        // Scattered flips over the real files, by a fixed sequence: whatever they do, no panic.
        let mut x = 0x2545_F491u32;
        let mut next = |len: usize| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as usize % len
        };
        let (head_m, head_c) = (model[..4096.min(model.len())].to_vec(), clips.clone());
        for _ in 0..20 {
            let mut b = model.clone();
            for _ in 0..50 {
                let i = next(b.len());
                b[i] ^= 0xA5;
            }
            let _ = parse_model(&b);
            let mut b = head_c.clone();
            for _ in 0..50 {
                let i = next(b.len());
                b[i] ^= 0xA5;
            }
            let _ = parse_clips(&b);
        }
        drop(head_m);
    }

    /// `cargo test -p se_core print_hero_assets -- --ignored --nocapture`: what a pair of files holds
    /// (`HERO_ASSET_DIR`, default the project's copy).
    #[test]
    #[ignore]
    fn print_hero_assets() {
        let dir = std::env::var("HERO_ASSET_DIR").map_or_else(|_| real_dir(), PathBuf::from);
        for f in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = f.path();
            let Ok(bytes) = std::fs::read(&p) else { continue };
            match p.extension().and_then(|e| e.to_str()) {
                Some("sevm") => {
                    let m = parse_model(&bytes).unwrap();
                    println!("{}: {} bones, {} draws, {} vertices, {} triangles, bounds {} to {}, cape {:?}", p.display(), m.bones.len(), m.draws.len(), m.vertices.len(), m.indices.len() / 3, m.bounds_min, m.bounds_max, m.cape_first);
                    let sum_ok = m.vertices.iter().filter(|v| v.weights.iter().map(|&w| w as u32).sum::<u32>() == 255).count();
                    println!("  weights adding up to 255: {sum_ok} of {}", m.vertices.len());
                    for d in &m.draws {
                        println!("  {}: {} vertices, {} triangles, {} {} {}, flags {}", d.name, d.vertex_count, d.index_count / 3, d.diffuse, d.normal, d.spec, d.flags);
                    }
                }
                Some("sevc") => {
                    let s = parse_clips(&bytes).unwrap();
                    println!("{}: {} clips ({} stand-ins), {} bones, {} fps, {} tracks, {} quats, {} roots", p.display(), s.clips.len(), s.stand_ins(), s.bone_count, s.fps, s.tracks.len(), s.quats.len(), s.roots.len());
                    for c in &s.clips {
                        let events: Vec<String> = s.events_of(c).iter().map(|e| format!("{} {:.2}", e.name, e.time)).collect();
                        println!("  {}: {} frames, {:.2} s, flags {}, {} tracks, ground {:.3}, stride {:.2}, {:?}", c.name, c.frames, c.duration, c.flags, c.track_count, c.ground_z, c.stride_speed, events);
                    }
                }
                _ => {}
            }
        }
    }
}
