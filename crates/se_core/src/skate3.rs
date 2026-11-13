//! The Skate 3 physics mod's files (`assets/mods/skate3`): the default skater with his board, Skate 3's clips for him, and
//! the game's own tuning values. Written once from the Skate 3 Rust Engine's converted data by the ignored test
//! `copy_skate3` (`se_engine/src/skate/convert.rs`), read when the mod is first turned on.
//!
//! Space is Skate 3's own: metres, Y up, the board's nose along +Z, the skater's origin on the deck's middle (`SKATEBOARD_ROOT`).
//!
//! - **Model** (`skater.s3m`): the animation skeleton (36 bones: `TRAJECTORY`, the body, the board, its trucks and wheels),
//!   the skin's joints (each a bone and the matrix that takes a bind-pose vertex into that bone's frame, so a vertex's skin
//!   matrix is the bone's model matrix times it), the draws (one per material, a texture file beside it and a tint) and the
//!   vertices.
//! - **Clips** (`clips.s3c`): per clip its frame rate, whether it loops, and every frame's local transform of every bone
//!   (scale, rotation, translation: the bank's sample added onto the rig's T-pose, `pose_add`, as the Skate 3 engine's
//!   `AddBindPose` does). A bone whose parent is `TRAJECTORY` (bone 0) is in model space: the trajectory's own motion is the
//!   physics', never the pose's.
//! - **Tuning** (`tuning.cfg`): a value or an eight-point curve a line, by Skate 3's collection, key and field name.

use std::collections::HashMap;

const MODEL_MAGIC: &[u8; 8] = b"S3SKATER";
const CLIPS_MAGIC: &[u8; 8] = b"S3SCLIPS";
const VERSION: u32 = 1;
/// The most bones and joints the files may hold (the palette's size).
pub const MAX_BONES: usize = 64;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Skate3Error {
    #[error("not a Skate 3 mod file (bad magic)")]
    Magic,
    #[error("unsupported version {0}")]
    Version(u32),
    #[error("the file ends early at byte {0}")]
    Short(usize),
    #[error("{0}")]
    Bad(String),
}

type R<T> = Result<T, Skate3Error>;

#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// -1 for none. Bones under `TRAJECTORY` (0) are in model space (see the module's notes).
    pub parent: i16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub bone: u16,
    /// Column-major: bind-pose model space to the bone's frame.
    pub bind: [f32; 16],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    /// The texture's file beside the model (a PNG), or empty for plain white.
    pub texture: String,
    pub tint: [f32; 4],
    /// Pixels under this alpha are cut out (0: none are).
    pub alpha_cut: f32,
    pub two_sided: bool,
    pub first_index: u32,
    pub index_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Joints (indices into `Model::joints`) and their weights out of 255.
    pub joints: [u8; 4],
    pub weights: [u8; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub bones: Vec<Bone>,
    pub joints: Vec<Joint>,
    pub draws: Vec<Draw>,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// One bone's local transform: scale xyz, rotation xyzw, translation xyz.
pub type Sample = [f32; 10];

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    pub fps: f32,
    pub looping: bool,
    pub frames: u32,
    /// `frames * bones` samples, frame by frame.
    pub samples: Vec<Sample>,
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        (self.frames.max(1) - 1) as f32 / self.fps.max(1.0)
    }

    pub fn frame(&self, i: usize, bones: usize) -> &[Sample] {
        let i = i.min(self.frames.max(1) as usize - 1);
        &self.samples[i * bones..(i + 1) * bones]
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clips {
    pub bones: u16,
    pub clips: Vec<Clip>,
}

impl Clips {
    pub fn get(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
}

// --- Writing -------------------------------------------------------------------------------------------------------

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.0.extend_from_slice(s.as_bytes());
    }
}

pub fn write_model(m: &Model) -> Vec<u8> {
    let mut w = W(MODEL_MAGIC.to_vec());
    w.u32(VERSION);
    w.u16(m.bones.len() as u16);
    for b in &m.bones {
        w.str(&b.name);
        w.i16(b.parent);
    }
    w.u16(m.joints.len() as u16);
    for j in &m.joints {
        w.u16(j.bone);
        j.bind.iter().for_each(|v| w.f32(*v));
    }
    w.u16(m.draws.len() as u16);
    for d in &m.draws {
        w.str(&d.texture);
        d.tint.iter().for_each(|v| w.f32(*v));
        w.f32(d.alpha_cut);
        w.u8(d.two_sided as u8);
        w.u32(d.first_index);
        w.u32(d.index_count);
    }
    w.u32(m.vertices.len() as u32);
    for v in &m.vertices {
        v.position.iter().chain(&v.normal).chain(&v.uv).for_each(|x| w.f32(*x));
        w.0.extend_from_slice(&v.joints);
        w.0.extend_from_slice(&v.weights);
    }
    w.u32(m.indices.len() as u32);
    m.indices.iter().for_each(|i| w.u32(*i));
    w.0
}

pub fn write_clips(c: &Clips) -> Vec<u8> {
    let mut w = W(CLIPS_MAGIC.to_vec());
    w.u32(VERSION);
    w.u16(c.bones);
    w.u32(c.clips.len() as u32);
    for clip in &c.clips {
        w.str(&clip.name);
        w.f32(clip.fps);
        w.u8(clip.looping as u8);
        w.u32(clip.frames);
        clip.samples.iter().flatten().for_each(|v| w.f32(*v));
    }
    w.0
}

// --- Reading -------------------------------------------------------------------------------------------------------

struct Rd<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let s = self.b.get(self.at..self.at + n).ok_or(Skate3Error::Short(self.at))?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> R<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> R<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> R<f32> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if !v.is_finite() {
            return Err(Skate3Error::Bad(format!("a value that isn't a number at byte {}", self.at - 4)));
        }
        Ok(v)
    }
    fn str(&mut self) -> R<String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| Skate3Error::Bad("a name that isn't text".into()))
    }
    fn head(&mut self, magic: &[u8; 8]) -> R<()> {
        if self.take(8).map_err(|_| Skate3Error::Magic)? != magic {
            return Err(Skate3Error::Magic);
        }
        match self.u32()? {
            VERSION => Ok(()),
            v => Err(Skate3Error::Version(v)),
        }
    }
    /// A count of items at least `each` bytes long that the rest of the file can hold.
    fn count(&mut self, n: usize, each: usize) -> R<usize> {
        if n.saturating_mul(each) > self.b.len() - self.at {
            return Err(Skate3Error::Short(self.b.len()));
        }
        Ok(n)
    }
}

pub fn parse_model(bytes: &[u8]) -> R<Model> {
    let mut r = Rd { b: bytes, at: 0 };
    r.head(MODEL_MAGIC)?;
    let mut m = Model::default();
    let n = r.u16()? as usize;
    if n == 0 || n > MAX_BONES {
        return Err(Skate3Error::Bad(format!("{n} bones")));
    }
    for _ in 0..n {
        let name = r.str()?;
        let parent = r.i16()?;
        m.bones.push(Bone { name, parent });
    }
    for (i, b) in m.bones.iter().enumerate() {
        if b.parent >= i as i16 || b.parent < -1 {
            return Err(Skate3Error::Bad(format!("bone {} has parent {}", b.name, b.parent)));
        }
    }
    let n = r.u16()? as usize;
    if n == 0 || n > MAX_BONES {
        return Err(Skate3Error::Bad(format!("{n} joints")));
    }
    for _ in 0..n {
        let bone = r.u16()?;
        if bone as usize >= m.bones.len() {
            return Err(Skate3Error::Bad(format!("a joint on bone {bone}")));
        }
        let mut bind = [0.0; 16];
        for v in &mut bind {
            *v = r.f32()?;
        }
        m.joints.push(Joint { bone, bind });
    }
    let n = r.u16()? as usize;
    for _ in 0..n {
        let texture = r.str()?;
        if texture.contains("..") || texture.contains(':') || texture.starts_with(['/', '\\']) {
            return Err(Skate3Error::Bad(format!("a texture outside the mod's folder: {texture}")));
        }
        let mut tint = [0.0; 4];
        for v in &mut tint {
            *v = r.f32()?;
        }
        let alpha_cut = r.f32()?;
        let two_sided = r.u8()? != 0;
        let (first_index, index_count) = (r.u32()?, r.u32()?);
        m.draws.push(Draw { texture, tint, alpha_cut, two_sided, first_index, index_count });
    }
    let n = r.u32()? as usize;
    let n = r.count(n, 40)?;
    m.vertices.reserve(n);
    for _ in 0..n {
        let mut f = [0.0f32; 8];
        for v in &mut f {
            *v = r.f32()?;
        }
        let joints: [u8; 4] = r.take(4)?.try_into().unwrap();
        let weights: [u8; 4] = r.take(4)?.try_into().unwrap();
        if joints.iter().any(|j| *j as usize >= m.joints.len()) {
            return Err(Skate3Error::Bad("a vertex on a joint that isn't there".into()));
        }
        m.vertices.push(Vertex { position: [f[0], f[1], f[2]], normal: [f[3], f[4], f[5]], uv: [f[6], f[7]], joints, weights });
    }
    let n = r.u32()? as usize;
    let n = r.count(n, 4)?;
    m.indices.reserve(n);
    for _ in 0..n {
        let i = r.u32()?;
        if i as usize >= m.vertices.len() {
            return Err(Skate3Error::Bad(format!("index {i} past the vertices")));
        }
        m.indices.push(i);
    }
    for d in &m.draws {
        if d.index_count % 3 != 0 || d.first_index as u64 + d.index_count as u64 > m.indices.len() as u64 {
            return Err(Skate3Error::Bad("a draw past the indices".into()));
        }
    }
    if r.at != bytes.len() {
        return Err(Skate3Error::Bad("bytes after the end".into()));
    }
    Ok(m)
}

pub fn parse_clips(bytes: &[u8]) -> R<Clips> {
    let mut r = Rd { b: bytes, at: 0 };
    r.head(CLIPS_MAGIC)?;
    let bones = r.u16()?;
    if bones == 0 || bones as usize > MAX_BONES {
        return Err(Skate3Error::Bad(format!("{bones} bones")));
    }
    let n = r.u32()? as usize;
    let n = r.count(n, 11)?;
    let mut clips = Vec::with_capacity(n);
    for _ in 0..n {
        let name = r.str()?;
        let fps = r.f32()?;
        let looping = r.u8()? != 0;
        let frames = r.u32()?;
        if frames == 0 || !(1.0..=1000.0).contains(&fps) {
            return Err(Skate3Error::Bad(format!("clip {name}: {frames} frames at {fps} a second")));
        }
        let count = r.count(frames as usize * bones as usize, 40)?;
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            let mut s = [0.0; 10];
            for v in &mut s {
                *v = r.f32()?;
            }
            samples.push(s);
        }
        clips.push(Clip { name, fps, looping, frames, samples });
    }
    if r.at != bytes.len() {
        return Err(Skate3Error::Bad("bytes after the end".into()));
    }
    Ok(Clips { bones, clips })
}

/// The model and the clips belong together: the same skeleton.
pub fn check_pair(m: &Model, c: &Clips) -> R<()> {
    if c.bones as usize != m.bones.len() {
        return Err(Skate3Error::Bad(format!("the clips have {} bones, the skater {}", c.bones, m.bones.len())));
    }
    Ok(())
}

// --- Tuning --------------------------------------------------------------------------------------------------------

/// Skate 3's own values: `class/key/field` to a value, or to a curve (its points' x, then their y).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tuning {
    pub values: HashMap<String, f32>,
    pub curves: HashMap<String, (Vec<f32>, Vec<f32>)>,
}

impl Tuning {
    pub fn write(&self) -> String {
        let mut lines: Vec<String> = self.values.iter().map(|(k, v)| format!("{k} {v:?}")).collect();
        lines.extend(self.curves.iter().map(|(k, (x, y))| {
            let x: Vec<String> = x.iter().map(|v| format!("{v:?}")).collect();
            let y: Vec<String> = y.iter().map(|v| format!("{v:?}")).collect();
            format!("{k} {} | {}", x.join(" "), y.join(" "))
        }));
        lines.sort();
        let mut s = String::from("# Skate 3's own tuning values (its collections), written by copy_skate3. A value or a curve (x values | y values) a line.\n");
        for l in lines {
            s.push_str(&l);
            s.push('\n');
        }
        s
    }

    pub fn parse(text: &str) -> R<Tuning> {
        let mut t = Tuning::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || Skate3Error::Bad(format!("tuning line {}: {line}", n + 1));
            let (key, rest) = line.split_once(' ').ok_or_else(bad)?;
            let num = |s: &str| s.parse::<f32>().ok().filter(|v| v.is_finite()).ok_or_else(bad);
            if let Some((xs, ys)) = rest.split_once('|') {
                let x: Vec<f32> = xs.split_whitespace().map(num).collect::<R<_>>()?;
                let y: Vec<f32> = ys.split_whitespace().map(num).collect::<R<_>>()?;
                if x.is_empty() || x.len() != y.len() {
                    return Err(bad());
                }
                t.curves.insert(key.to_string(), (x, y));
            } else {
                t.values.insert(key.to_string(), num(rest.trim())?);
            }
        }
        Ok(t)
    }

    pub fn value(&self, key: &str) -> R<f32> {
        self.values.get(key).copied().ok_or_else(|| Skate3Error::Bad(format!("tuning value {key} is missing")))
    }

    /// A curve of exactly `N` points.
    pub fn curve<const N: usize>(&self, key: &str) -> R<([f32; N], [f32; N])> {
        let (x, y) = self.curves.get(key).ok_or_else(|| Skate3Error::Bad(format!("tuning curve {key} is missing")))?;
        match (<[f32; N]>::try_from(x.as_slice()), <[f32; N]>::try_from(y.as_slice())) {
            (Ok(x), Ok(y)) => Ok((x, y)),
            _ => Err(Skate3Error::Bad(format!("tuning curve {key} has {} points, not {N}", x.len()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model {
            bones: vec![Bone { name: "TRAJECTORY".into(), parent: -1 }, Bone { name: "HIPS".into(), parent: 0 }],
            joints: vec![Joint { bone: 1, bind: std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 }) }],
            draws: vec![Draw { texture: "tex/a.png".into(), tint: [1.0, 0.5, 0.25, 1.0], alpha_cut: 0.0, two_sided: true, first_index: 0, index_count: 3 }],
            vertices: (0..3).map(|i| Vertex { position: [i as f32, 0.0, 0.0], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], joints: [0; 4], weights: [255, 0, 0, 0] }).collect(),
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn the_files_read_back_what_was_written() {
        let m = model();
        let bytes = write_model(&m);
        assert_eq!(parse_model(&bytes).unwrap(), m);
        assert!(parse_model(&bytes[..bytes.len() - 1]).is_err());
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(parse_model(&bad), Err(Skate3Error::Magic));
        let c = Clips { bones: 2, clips: vec![Clip { name: "R_IDLE".into(), fps: 60.0, looping: true, frames: 2, samples: vec![[0.5; 10]; 4] }] };
        let bytes = write_clips(&c);
        assert_eq!(parse_clips(&bytes).unwrap(), c);
        assert!(parse_clips(&bytes[..bytes.len() - 3]).is_err());
        check_pair(&m, &c).unwrap();
        assert!((c.clips[0].seconds() - 1.0 / 60.0).abs() < 1e-6);
        let mut t = Tuning::default();
        t.values.insert("physics_push/default/MaxPushableSpeed".into(), 7.5);
        t.curves.insert("physics_steering/default/SteeringVsSpeed".into(), (vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], vec![1.0; 8]));
        assert_eq!(Tuning::parse(&t.write()).unwrap(), t);
        assert!(Tuning::parse("a 1 2 | 3").is_err());
        assert!(t.curve::<8>("physics_steering/default/SteeringVsSpeed").is_ok());
        assert!(t.curve::<16>("physics_steering/default/SteeringVsSpeed").is_err());
    }

    #[test]
    fn a_texture_outside_the_folder_is_refused() {
        let mut m = model();
        m.draws[0].texture = "../../x.png".into();
        assert!(parse_model(&write_model(&m)).is_err());
    }
}
