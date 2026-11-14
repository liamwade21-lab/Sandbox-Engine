//! Readers for Elite Dangerous's open data files, for the Faulcon DeLacy Sidewinder mod (`docs/spec-mods-sidewinder.md`).
//!
//! Elite keeps its data in `Win64/00`..`0f` beside its exe, each file named by 40 hex digits (the folder is `0` and the
//! name's first digit). About half of them start with `FREA`: those are encrypted and are never opened past that tag (the
//! user's rule: no DRM is defeated or gone round). Everything here reads only the open kinds, and only the converter and
//! tests use it: the game never reads Elite's install while it runs.
//!
//! **Meshes** (zlib, unpacked data starting `02 00 01 00`), worked out from the data alone in step 0.1:
//!
//! - A header of 32-bit words. Word 0 is `0x10002`, word 6 the number of chunks `n` and word 3 `8 n`. Chunk `i`'s
//!   descriptor is the 8 words from `9 + 8 i`: an id, the tag `0x5607a974`, `0x10000`, 0, the bytes of its rows, 0, the
//!   bytes of its body, 0. Then `n` pairs (a kind, 2 or 3, meaning not known; the chunk's rows plus body bytes). What
//!   follows up to the data (more ids, two constant words) is not needed to read the meshes.
//! - The chunks' data fill the file's end, back to back in descriptor order. Files with several chunks are mostly a model's
//!   levels of detail or its pieces.
//! - A chunk: rows of 24 bytes, one per part: (triangles, `0xffff`, vertices, `0xffff`, `0xffffffff`, 0); then each part's
//!   16-bit triangle indices, counted from the part's own first vertex, padded to 4 bytes; then every part's vertices.
//! - A vertex is 40 bytes (`Vertex`). Its position is three 21-bit fields packed in 8 bytes, each the field minus 2^20.
//!   Their unit is not in these files: models use the range differently (two thirds reach 2^19 to 2^20, a quarter 2^18 to
//!   2^19), so the scale is believed to live with the model's assembly, in the encrypted half.
//!
//! Proven over the install of 2026-10-23 (version 2026.10.23.334925): 21,896 of the 22,008 mesh files read with every
//! index inside its part, 215,351,280 triangles and 267,544,373 vertices, normals and tangents of unit length and at right
//! angles, bone weights summing to 255, and a few hundred drawn from three sides as recognisable things (go/no-go G1).
//! The other 112 files are kinds not yet read: 91 whose rows are not 24 bytes long, 21 whose bodies don't fit the layout.

use std::path::{Path, PathBuf};

/// The tag that starts every encrypted file.
pub const ENCRYPTED_TAG: &[u8; 4] = b"FREA";
/// The first four unpacked bytes of a mesh file.
pub const MESH_TAG: [u8; 4] = [2, 0, 1, 0];
/// The first four unpacked bytes of a texture file (read in step 0.2).
pub const TEXTURE_TAG: [u8; 4] = [3, 0, 1, 0];
/// Each vertex's size in bytes.
pub const VERTEX_BYTES: usize = 40;

const MESH_VERSION: u32 = 0x10002;
const CHUNK_TAG: u32 = 0x5607_a974;
const POSITION_BIAS: i32 = 1 << 20;
/// The most a zlib file may unpack to (the largest texture is about 90 MB).
const UNPACK_LIMIT: usize = 512 << 20;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EliteError {
    #[error("an encrypted (FREA) file: it is not read")]
    Encrypted,
    #[error("not a zlib file")]
    NotZlib,
    #[error("the zlib data is broken")]
    Unpack,
    #[error("not a mesh file")]
    NotMesh,
    #[error("the file ends early at byte {0}")]
    Short(usize),
    #[error("{0}")]
    Bad(String),
}

type R<T> = Result<T, EliteError>;

/// What a file of the install holds, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// `FREA`: encrypted, never opened.
    Encrypted,
    /// zlib holding a mesh (`02 00 01 00`).
    Mesh,
    /// zlib holding a texture (`03 00 01 00`).
    Texture,
    /// zlib holding something else.
    OtherZlib,
    /// A RIFF file: Wwise sound streams.
    Riff,
    Other,
}

/// True for a file that starts with `FREA`. Only the first four bytes are looked at.
pub fn is_encrypted(head: &[u8]) -> bool {
    head.len() >= 4 && &head[..4] == ENCRYPTED_TAG
}

fn is_zlib(head: &[u8]) -> bool {
    head.len() >= 2 && head[0] == 0x78 && matches!(head[1], 0x01 | 0x5e | 0x9c | 0xda)
}

/// Unpacks one open zlib file. An encrypted file is refused by its tag before anything else is read.
pub fn unpack(raw: &[u8]) -> R<Vec<u8>> {
    if is_encrypted(raw) {
        return Err(EliteError::Encrypted);
    }
    if !is_zlib(raw) {
        return Err(EliteError::NotZlib);
    }
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(raw, UNPACK_LIMIT).map_err(|_| EliteError::Unpack)
}

/// A file's kind from its first bytes (a zlib file's first 64 KB are enough to see its inner tag).
pub fn file_kind(head: &[u8]) -> FileKind {
    if is_encrypted(head) {
        return FileKind::Encrypted;
    }
    if head.len() >= 4 && &head[..4] == b"RIFF" {
        return FileKind::Riff;
    }
    if is_zlib(head) {
        let mut out = vec![0u8; 16];
        let mut st = miniz_oxide::inflate::stream::InflateState::new(miniz_oxide::DataFormat::Zlib);
        let r = miniz_oxide::inflate::stream::inflate(&mut st, head, &mut out, miniz_oxide::MZFlush::None);
        if r.bytes_written >= 4 {
            return match [out[0], out[1], out[2], out[3]] {
                MESH_TAG => FileKind::Mesh,
                TEXTURE_TAG => FileKind::Texture,
                _ => FileKind::OtherZlib,
            };
        }
        return FileKind::OtherZlib;
    }
    FileKind::Other
}

/// Where a data file of the install lies: `Win64/0X/NAME` with X the name's first digit.
pub fn data_file(win64: &Path, name: &str) -> PathBuf {
    let first = name.chars().next().unwrap_or('0');
    win64.join(format!("0{first}")).join(name)
}

/// One vertex as Elite stores it. The bytes are kept as they are (grade A); the methods read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vertex(pub [u8; VERTEX_BYTES]);

fn unorm8(b: u8) -> f32 {
    (b as f32 - 127.0) / 127.0
}

fn uv16(lo: u8, hi: u8) -> f32 {
    (u16::from_le_bytes([lo, hi]) as f32 - 32767.0) / 2047.0
}

impl Vertex {
    /// The position's three fields, each minus 2^20 (bits 0-20 x, 21-41 y, 42-62 z). Y is up: a wine glass, a
    /// helmet and a pair of trousers among the meshes stand on Y.
    pub fn position_raw(&self) -> [i32; 3] {
        let q = u64::from_le_bytes(self.0[0..8].try_into().unwrap());
        let f = |s: u32| ((q >> s) & 0x1f_ffff) as i32 - POSITION_BIAS;
        [f(0), f(21), f(42)]
    }
    /// Bytes 8-10: the normal (unit length, checked over the install).
    pub fn normal(&self) -> [f32; 3] {
        [unorm8(self.0[8]), unorm8(self.0[9]), unorm8(self.0[10])]
    }
    /// Byte 11: its top bit is believed to be the tangent frame's handedness; the low seven bits are not known.
    pub fn byte11(&self) -> u8 {
        self.0[11]
    }
    /// Bytes 12-14: the tangent (unit length and at right angles to the normal, checked over the install).
    pub fn tangent(&self) -> [f32; 3] {
        [unorm8(self.0[12]), unorm8(self.0[13]), unorm8(self.0[14])]
    }
    /// Byte 15: the same within each part; its meaning is not known.
    pub fn byte15(&self) -> u8 {
        self.0[15]
    }
    /// Bytes 16-19: the texture coordinate, two u16 as (v - 32767) / 2047 (the edges of a quad read exactly 0 and 1).
    pub fn uv(&self) -> [f32; 2] {
        [uv16(self.0[16], self.0[17]), uv16(self.0[18], self.0[19])]
    }
    /// Bytes 20-23: believed to be a colour (mostly `00 00 00 ff`).
    pub fn colour(&self) -> [u8; 4] {
        self.0[20..24].try_into().unwrap()
    }
    /// Bytes 24-31: believed to be a second and third texture coordinate, read as `uv` (mostly zero).
    pub fn uv2(&self) -> [[f32; 2]; 2] {
        [[uv16(self.0[24], self.0[25]), uv16(self.0[26], self.0[27])], [uv16(self.0[28], self.0[29]), uv16(self.0[30], self.0[31])]]
    }
    /// Bytes 32-35: believed to be four bone indices (all zero in most meshes).
    pub fn bones(&self) -> [u8; 4] {
        self.0[32..36].try_into().unwrap()
    }
    /// Bytes 36-39: four bone weights; they sum to 255 in every vertex looked at.
    pub fn weights(&self) -> [u8; 4] {
        self.0[36..40].try_into().unwrap()
    }
}

/// One part of a chunk: its triangles (indices into its own vertices) and its vertices.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPart {
    pub triangles: Vec<[u16; 3]>,
    pub vertices: Vec<Vertex>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshChunk {
    pub id: u32,
    /// 2 or 3 in every file read; what it means is not known.
    pub kind: u32,
    pub parts: Vec<MeshPart>,
}

impl MeshChunk {
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.triangles.len()).sum()
    }
    pub fn vertex_count(&self) -> usize {
        self.parts.iter().map(|p| p.vertices.len()).sum()
    }
    /// The smallest and largest raw position over every part.
    pub fn bounds_raw(&self) -> Option<([i32; 3], [i32; 3])> {
        let mut it = self.parts.iter().flat_map(|p| p.vertices.iter()).map(|v| v.position_raw());
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| {
            ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])])
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshFile {
    /// Header word 5 (an id; the same as one chunk's).
    pub id: u32,
    pub chunks: Vec<MeshChunk>,
}

fn word(d: &[u8], i: usize) -> R<u32> {
    let o = i * 4;
    d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or(EliteError::Short(o))
}

fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

/// Reads an unpacked mesh file (see the module's notes for the layout).
pub fn parse_mesh(d: &[u8]) -> R<MeshFile> {
    if d.len() < 4 || d[..4] != MESH_TAG {
        return Err(EliteError::NotMesh);
    }
    if word(d, 0)? != MESH_VERSION {
        return Err(EliteError::Bad(format!("version {:#x}", word(d, 0)?)));
    }
    let n = word(d, 6)? as usize;
    if n == 0 || n > 4096 || word(d, 3)? as usize != 8 * n {
        return Err(EliteError::Bad("chunk count".into()));
    }
    struct Desc {
        id: u32,
        kind: u32,
        rows: usize,
        body: usize,
    }
    let mut descs = Vec::with_capacity(n);
    for i in 0..n {
        let w = 9 + 8 * i;
        let (id, tag, one, rows, body) = (word(d, w)?, word(d, w + 1)?, word(d, w + 2)?, word(d, w + 4)? as usize, word(d, w + 6)? as usize);
        let kind = word(d, 9 + 8 * n + 2 * i)?;
        let size = word(d, 10 + 8 * n + 2 * i)? as usize;
        if tag != CHUNK_TAG || one != 0x10000 || rows % 24 != 0 || size != rows + body {
            return Err(EliteError::Bad(format!("chunk {i}'s descriptor")));
        }
        descs.push(Desc { id, kind, rows, body });
    }
    let total: usize = descs.iter().map(|c| c.rows + c.body).sum();
    let header_end = (9 + 10 * n) * 4;
    if total + header_end > d.len() {
        return Err(EliteError::Bad("the chunks are bigger than the file".into()));
    }
    let mut o = d.len() - total;
    let mut chunks = Vec::with_capacity(n);
    for c in &descs {
        let mut rows = Vec::with_capacity(c.rows / 24);
        for r in 0..c.rows / 24 {
            let w = |k: usize| u32::from_le_bytes(d[o + 24 * r + 4 * k..o + 24 * r + 4 * k + 4].try_into().unwrap()) as usize;
            rows.push((w(0), w(2)));
        }
        let index_bytes: usize = rows.iter().map(|&(t, _)| pad4(6 * t)).sum();
        let verts: usize = rows.iter().map(|&(_, v)| v).sum();
        if index_bytes + VERTEX_BYTES * verts != c.body {
            return Err(EliteError::Bad("a chunk's body doesn't fit its rows".into()));
        }
        let mut io = o + c.rows;
        let mut vo = io + index_bytes;
        let mut parts = Vec::with_capacity(rows.len());
        for &(t, v) in &rows {
            let mut triangles = Vec::with_capacity(t);
            for k in 0..t {
                let i = |j: usize| u16::from_le_bytes([d[io + 6 * k + 2 * j], d[io + 6 * k + 2 * j + 1]]);
                let tri = [i(0), i(1), i(2)];
                if tri.iter().any(|&x| x as usize >= v) {
                    return Err(EliteError::Bad("an index outside its part".into()));
                }
                triangles.push(tri);
            }
            let vertices = (0..v).map(|k| Vertex(d[vo + VERTEX_BYTES * k..vo + VERTEX_BYTES * (k + 1)].try_into().unwrap())).collect();
            parts.push(MeshPart { triangles, vertices });
            io += pad4(6 * t);
            vo += VERTEX_BYTES * v;
        }
        chunks.push(MeshChunk { id: c.id, kind: c.kind, parts });
        o += c.rows + c.body;
    }
    Ok(MeshFile { id: word(d, 5)?, chunks })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packs a position as Elite does (three 21-bit fields, each plus 2^20).
    fn pack_position(p: [i32; 3]) -> [u8; 8] {
        let f = |v: i32| (v + POSITION_BIAS) as u64 & 0x1f_ffff;
        (f(p[0]) | f(p[1]) << 21 | f(p[2]) << 42).to_le_bytes()
    }

    fn vertex(p: [i32; 3]) -> [u8; 40] {
        let mut v = [0u8; 40];
        v[0..8].copy_from_slice(&pack_position(p));
        v[8..12].copy_from_slice(&[127, 254, 127, 3]); // normal +Y
        v[12..16].copy_from_slice(&[254, 127, 127, 0]); // tangent +X
        v[16..20].copy_from_slice(&[0xfe, 0x87, 0xff, 0x7f]); // uv (1, 0)
        v[20..24].copy_from_slice(&[0, 0, 0, 255]);
        v[24..32].copy_from_slice(&[0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f]);
        v[36] = 255;
        v
    }

    /// A one-chunk mesh file as Elite lays it out: a triangle (its indices then need 2 bytes of padding) and a quad.
    fn built_mesh(bad_index: bool) -> Vec<u8> {
        let parts: [(&[[u16; 3]], &[[i32; 3]]); 2] = [
            (&[[0, 1, 2]], &[[0, 0, 0], [1000, 0, 0], [0, 1000, 0]]),
            (&[[0, 1, 2], [2, 1, if bad_index { 4 } else { 3 }]], &[[-1 << 20, 5, 7], [(1 << 20) - 1, 0, 0], [0, -300, 0], [0, 0, 524_288]]),
        ];
        let mut rows = Vec::new();
        let mut body = Vec::new();
        for (t, v) in &parts {
            for w in [t.len() as u32, 0xffff, v.len() as u32, 0xffff, 0xffff_ffff, 0] {
                rows.extend_from_slice(&w.to_le_bytes());
            }
            for tri in t.iter() {
                for i in tri {
                    body.extend_from_slice(&i.to_le_bytes());
                }
            }
            while body.len() % 4 != 0 {
                body.push(0);
            }
        }
        for (_, v) in &parts {
            for p in v.iter() {
                body.extend_from_slice(&vertex(*p));
            }
        }
        let mut h = vec![0u32; 29];
        h[0] = MESH_VERSION;
        h[3] = 8;
        h[5] = 0xabcd;
        h[6] = 1;
        h[7] = CHUNK_TAG;
        h[9] = 0xabcd;
        h[10] = CHUNK_TAG;
        h[11] = 0x10000;
        h[13] = rows.len() as u32;
        h[15] = body.len() as u32;
        h[17] = 2;
        h[18] = (rows.len() + body.len()) as u32;
        let mut d: Vec<u8> = h.iter().flat_map(|w| w.to_le_bytes()).collect();
        d[..4].copy_from_slice(&MESH_TAG);
        d.extend(rows);
        d.extend(body);
        d
    }

    /// C3 of the plan: an encrypted file is refused by its tag, whatever follows it.
    #[test]
    fn refuses_frea() {
        let mut f = b"FREA\0\0".to_vec();
        f.extend_from_slice(&[0x78, 0x9c, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
        assert_eq!(unpack(&f), Err(EliteError::Encrypted));
        assert_eq!(file_kind(&f), FileKind::Encrypted);
        assert!(is_encrypted(&f));
        assert!(!is_encrypted(b"FRE"));
    }

    #[test]
    fn reads_a_built_mesh() {
        let d = built_mesh(false);
        let m = parse_mesh(&d).unwrap();
        assert_eq!(m.id, 0xabcd);
        assert_eq!(m.chunks.len(), 1);
        let c = &m.chunks[0];
        assert_eq!((c.kind, c.parts.len(), c.triangle_count(), c.vertex_count()), (2, 2, 3, 7));
        assert_eq!(c.parts[1].triangles, vec![[0, 1, 2], [2, 1, 3]]);
        let p: Vec<[i32; 3]> = c.parts[1].vertices.iter().map(|v| v.position_raw()).collect();
        assert_eq!(p, vec![[-1 << 20, 5, 7], [(1 << 20) - 1, 0, 0], [0, -300, 0], [0, 0, 524_288]]);
        assert_eq!(c.bounds_raw(), Some(([-1 << 20, -300, 0], [(1 << 20) - 1, 1000, 524_288])));
        let v = c.parts[0].vertices[1];
        assert_eq!(v.normal(), [0.0, 1.0, 0.0]);
        assert_eq!(v.tangent(), [1.0, 0.0, 0.0]);
        assert_eq!(v.uv(), [1.0, 0.0]);
        assert_eq!(v.uv2(), [[0.0, 0.0], [0.0, 0.0]]);
        assert_eq!((v.colour(), v.bones(), v.weights(), v.byte11(), v.byte15()), ([0, 0, 0, 255], [0; 4], [255, 0, 0, 0], 3, 0));
    }

    #[test]
    fn refuses_a_bad_mesh() {
        assert!(matches!(parse_mesh(&built_mesh(true)), Err(EliteError::Bad(_))));
        let d = built_mesh(false);
        assert!(parse_mesh(&d[..d.len() - 4]).is_err());
        assert_eq!(parse_mesh(b"\x03\x00\x01\x00rest"), Err(EliteError::NotMesh));
        // A zlib round trip and the kind seen from the packed bytes.
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&d, 6);
        assert_eq!(file_kind(&z), FileKind::Mesh);
        assert_eq!(parse_mesh(&unpack(&z).unwrap()).unwrap().chunks[0].vertex_count(), 7);
        assert_eq!(unpack(b"RIFF...."), Err(EliteError::NotZlib));
        assert_eq!(file_kind(b"RIFF...."), FileKind::Riff);
    }

    /// The user's install, read only. `ELITE_PATH` names the folder holding `Win64` (the product folder).
    fn install_win64() -> Option<PathBuf> {
        let base = std::env::var_os("ELITE_PATH").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(r"D:\SteamLibrary\steamapps\common\Elite Dangerous\Products\elite-dangerous-odyssey-64")
        });
        let w = base.join("Win64");
        w.join("00").is_dir().then_some(w)
    }

    /// Reads the first 64 KB of a file: enough for its kind. Its first four bytes are read alone first, and an encrypted
    /// file is read no further than its tag.
    fn head(p: &Path) -> Vec<u8> {
        use std::io::Read;
        let Ok(mut f) = std::fs::File::open(p) else { return Vec::new() };
        let mut tag = [0u8; 4];
        if f.read_exact(&mut tag).is_err() {
            return Vec::new();
        }
        if &tag == ENCRYPTED_TAG {
            return tag.to_vec();
        }
        let mut b = vec![0u8; 65536];
        b[..4].copy_from_slice(&tag);
        let n = f.read(&mut b[4..]).unwrap_or(0);
        b.truncate(4 + n);
        b
    }

    struct Tally {
        files: usize,
        encrypted: usize,
        meshes: usize,
        read: usize,
        triangles: usize,
        vertices: usize,
        not_read: Vec<(String, EliteError)>,
    }

    fn walk(win64: &Path, every: usize) -> Tally {
        let mut t = Tally { files: 0, encrypted: 0, meshes: 0, read: 0, triangles: 0, vertices: 0, not_read: Vec::new() };
        for dir in 0..16 {
            let mut names: Vec<_> = std::fs::read_dir(win64.join(format!("0{dir:x}"))).unwrap().flatten().map(|e| e.path()).collect();
            names.sort();
            for (k, p) in names.iter().enumerate() {
                if k % every != 0 {
                    continue;
                }
                t.files += 1;
                let h = head(p);
                match file_kind(&h) {
                    FileKind::Encrypted => t.encrypted += 1,
                    FileKind::Mesh => {
                        t.meshes += 1;
                        let raw = std::fs::read(p).unwrap();
                        match unpack(&raw).and_then(|d| parse_mesh(&d)) {
                            Ok(m) => {
                                t.read += 1;
                                t.triangles += m.chunks.iter().map(|c| c.triangle_count()).sum::<usize>();
                                t.vertices += m.chunks.iter().map(|c| c.vertex_count()).sum::<usize>();
                            }
                            Err(e) => t.not_read.push((p.file_name().unwrap().to_string_lossy().into_owned(), e)),
                        }
                    }
                    _ => {}
                }
            }
        }
        t
    }

    /// A sample of the install (every 200th file): every mesh in it reads, or is one of the kinds not read yet. Skips
    /// without the install.
    #[test]
    fn reads_meshes_of_the_install() {
        let Some(w) = install_win64() else {
            eprintln!("no Elite Dangerous install: skipped");
            return;
        };
        let t = walk(&w, 200);
        eprintln!("{} files, {} encrypted (not read), {} meshes, {} read", t.files, t.encrypted, t.meshes, t.read);
        assert!(t.meshes > 50, "too few meshes in the sample: {}", t.meshes);
        assert!(t.read * 100 >= t.meshes * 98, "{} of {} read; not read: {:?}", t.read, t.meshes, t.not_read);
    }

    /// Every mesh file of the install: the counts in the module's notes. Run with
    /// `cargo test -p se_core --release every_mesh_of_the_install -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn every_mesh_of_the_install() {
        let Some(w) = install_win64() else {
            eprintln!("no Elite Dangerous install: skipped");
            return;
        };
        let t = walk(&w, 1);
        let mut why = std::collections::BTreeMap::<String, usize>::new();
        for (_, e) in &t.not_read {
            *why.entry(e.to_string()).or_default() += 1;
        }
        eprintln!(
            "{} files, {} encrypted (not read), {} meshes, {} read, {} triangles, {} vertices; not read: {:?}",
            t.files, t.encrypted, t.meshes, t.read, t.triangles, t.vertices, why
        );
        assert!(t.read >= 21_896, "fewer meshes read than in step 0.1: {}", t.read);
    }
}
