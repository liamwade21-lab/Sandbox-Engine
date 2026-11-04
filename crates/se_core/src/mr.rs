//! Megaton Rainfall's files, read at run time from the user's copy of the game (nothing is copied
//! into the project): its `.fcm` archives (plain ZIP), the model `.bin` files with the materials
//! embedded in them, `Data/global.txt`'s `MATERIAL` entries, and its DDS textures (decoded by
//! [`crate::dds`]). The vegetation and cities plan (`docs/spec-vegetation-civilizations.md`) reuses
//! its trees, grass, building parts, roads, props and cars. Formats:
//! `docs/megaton-rainfall-vegetation.md` 7.3 and `docs/megaton-rainfall-cities.md` E.
//!
//! A model `.bin` (little endian):
//! - `u32` 205, `u32` version (1; high bits are flags on a few models), `u8` flags;
//! - counted streams, each `u32 n` then `n` elements: positions (3 x f32), normals (3 x snorm8),
//!   tangents (3 x snorm8), colours (RGBA8), UV0 (2 x f16), then three 4-byte streams that only
//!   a few special models fill (skinned ones with larger elements: those are not read). A stream may hold fewer elements than the positions (then
//!   only the first sub-meshes have them) or none;
//! - `u32 n` indices (u32, triangle list, absolute vertex numbers);
//! - `u32` sub-mesh count, each: first index, first vertex, index count, vertex count (u32), a `u8`
//!   1, a `u32` length and the material record;
//! - trailing data (collision or bounds, not needed to draw).
//!
//! A material record is a tree of nodes: `u8` child count, `u8` payload size, `u32` key hash,
//! the payload, then the children. Its root (15 children, hash 0) holds the material's name, its
//! effect (whose children are the effect's parameters: `g_vDiffuse` has x, y, z, w children) and
//! its texture list. The hash function is unknown; the keys below were identified on the files
//! (checked against global.txt's values for the trees' leaves: g_vDiffuse 1.3, 1.3, 0.56, 1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::vfs::{Vfs, VfsError};

/// Where the user's copy is installed.
pub const DEFAULT_MR_PATH: &str = "C:/Users/liamw/Downloads/Megaton Rainfall";

/// The archives the port reads, in `Assets/`.
pub const ARCHIVES: [&str; 3] = ["Models.fcm", "Textures.fcm", "Data.fcm"];

const KEY_NAME: u32 = 0xde08_cb97;
const KEY_EFFECT: u32 = 0x7221_46f4;
const KEY_TEXTURES: u32 = 0xe759_5b09;
const KEY_DIFFUSE: u32 = 0xb025_6f2a;

#[derive(Debug, thiserror::Error)]
pub enum MrError {
    #[error("Megaton Rainfall not found: {0} has no Assets/{1}")]
    NotInstalled(PathBuf, &'static str),
    #[error(transparent)]
    Vfs(#[from] VfsError),
    #[error("model {name}: {reason}")]
    Model { name: String, reason: String },
    #[error("texture {name}: {source}")]
    Texture {
        name: String,
        #[source]
        source: crate::dds::DdsError,
    },
}

/// A material: from `global.txt`, or embedded in a model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MrMaterial {
    pub name: String,
    /// The effect (shader) the game draws it with, e.g. `D_near`, `DA_alpha_test_leaves`.
    pub effect: String,
    /// Texture names as the game writes them (`Trees\Bark1A`): see [`texture_path`].
    pub textures: Vec<String>,
    /// The effect's parameters. From global.txt with their names; from a model only `g_vDiffuse`
    /// is named, the others are `#xxxxxxxx` (their key hash).
    pub params: Vec<(String, Vec<f32>)>,
}

impl MrMaterial {
    pub fn param(&self, name: &str) -> Option<&[f32]> {
        self.params.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_slice())
    }

    /// `g_vDiffuse` (the material's tint), white when not set.
    pub fn diffuse(&self) -> [f32; 4] {
        match self.param("g_vDiffuse") {
            Some(v) if v.len() >= 4 => [v[0], v[1], v[2], v[3]],
            Some(v) if v.len() == 3 => [v[0], v[1], v[2], 1.0],
            _ => [1.0; 4],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MrSubMesh {
    pub first_index: u32,
    pub first_vertex: u32,
    pub index_count: u32,
    pub vertex_count: u32,
    /// The material embedded in the model.
    pub material: MrMaterial,
}

/// A decoded model. Metres, y up, the origin at the base (cars face +x).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MrModel {
    pub version: u32,
    pub flags: u8,
    pub positions: Vec<[f32; 3]>,
    /// Unit-ish normals; may be shorter than `positions` (or empty).
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub colors: Vec<[u8; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<MrSubMesh>,
}

impl MrModel {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Bounding box of the positions (min, max).
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in &self.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }
}

struct Reader<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.o.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| {
            format!("truncated at byte {} (wanted {n} more of {})", self.o, self.b.len())
        })?;
        let s = &self.b[self.o..end];
        self.o = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// A counted stream: its element count and bytes.
    fn stream(&mut self, elem: usize, limit: u32) -> Result<(usize, &'a [u8]), String> {
        let n = self.u32()?;
        if n > limit {
            return Err(format!("stream of {n} elements (limit {limit})"));
        }
        Ok((n as usize, self.take(n as usize * elem)?))
    }
}

fn snorm3(s: &[u8]) -> [f32; 3] {
    let f = |b: u8| (b as i8 as f32 / 127.0).max(-1.0);
    [f(s[0]), f(s[1]), f(s[2])]
}

/// IEEE half to f32.
fn f16(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let man = (h & 0x3ff) as f32;
    sign * match exp {
        0 => man * (2.0f32).powi(-24),
        31 => {
            if man == 0.0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => (1.0 + man / 1024.0) * (2.0f32).powi(e - 15),
    }
}

/// One node of a material record.
struct Node<'a> {
    key: u32,
    payload: &'a [u8],
    children: Vec<Node<'a>>,
}

fn parse_node<'a>(r: &mut Reader<'a>, depth: u32) -> Result<Node<'a>, String> {
    if depth > 16 {
        return Err("material record nested too deep".into());
    }
    let n = r.u8()?;
    let size = r.u8()? as usize;
    let key = r.u32()?;
    let payload = r.take(size)?;
    let children = (0..n).map(|_| parse_node(r, depth + 1)).collect::<Result<_, _>>()?;
    Ok(Node { key, payload, children })
}

fn c_string(p: &[u8]) -> String {
    let end = p.iter().position(|&c| c == 0).unwrap_or(p.len());
    crate::vfs::decode_text(&p[..end])
}

fn f32_payload(p: &[u8]) -> Option<f32> {
    (p.len() == 4).then(|| f32::from_le_bytes([p[0], p[1], p[2], p[3]]))
}

/// Decodes a model's embedded material record.
pub fn parse_material_record(blob: &[u8]) -> Result<MrMaterial, String> {
    let mut r = Reader { b: blob, o: 0 };
    let root = parse_node(&mut r, 0)?;
    if r.o != blob.len() {
        return Err(format!("material record has {} bytes after its tree", blob.len() - r.o));
    }
    let mut m = MrMaterial::default();
    for node in &root.children {
        match node.key {
            KEY_NAME => m.name = c_string(node.payload),
            KEY_EFFECT => {
                m.effect = c_string(node.payload);
                for p in &node.children {
                    let values: Vec<f32> = if p.children.is_empty() {
                        f32_payload(p.payload).into_iter().collect()
                    } else {
                        p.children.iter().filter_map(|c| f32_payload(c.payload)).collect()
                    };
                    let name = if p.key == KEY_DIFFUSE { "g_vDiffuse".to_string() } else { format!("#{:08x}", p.key) };
                    m.params.push((name, values));
                }
            }
            KEY_TEXTURES => m.textures = node.children.iter().map(|t| c_string(t.payload)).collect(),
            _ => {}
        }
    }
    Ok(m)
}

/// Decodes a model `.bin`.
pub fn parse_model(bytes: &[u8]) -> Result<MrModel, String> {
    let mut r = Reader { b: bytes, o: 0 };
    let magic = r.u32()?;
    if magic != 205 {
        return Err(format!("not a model (magic {magic})"));
    }
    let version = r.u32()?;
    if version & 0xFFFF != 1 {
        return Err(format!("version {version:#x}"));
    }
    let flags = r.u8()?;
    const LIMIT: u32 = 1 << 24;
    let (n, pos) = r.stream(12, LIMIT)?;
    let positions: Vec<[f32; 3]> = pos
        .chunks_exact(12)
        .map(|c| {
            let f = |o: usize| f32::from_le_bytes([c[o], c[o + 1], c[o + 2], c[o + 3]]);
            [f(0), f(4), f(8)]
        })
        .collect();
    let n_limit = n as u32;
    let (_, nor) = r.stream(3, n_limit)?;
    let (_, tan) = r.stream(3, n_limit)?;
    let (_, col) = r.stream(4, n_limit)?;
    let (_, uv) = r.stream(4, n_limit)?;
    // Three more 4-byte streams (a second UV set and bone data on a few models, never on the
    // assets the port draws): skipped. Skinned models' bone streams are larger than 4 bytes an
    // element, so the index count read after them is out of range and they are refused below.
    for _ in 0..3 {
        r.stream(4, n_limit)?;
    }
    let (ni, idx) = r.stream(4, LIMIT)?;
    let indices: Vec<u32> = idx.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if let Some(&bad) = indices.iter().find(|&&i| i as usize >= n) {
        return Err(format!("index {bad} out of {n} vertices"));
    }
    let ns = r.u32()?;
    if ns > 4096 {
        return Err(format!("{ns} sub-meshes"));
    }
    let mut submeshes = Vec::with_capacity(ns as usize);
    for s in 0..ns {
        let (first_index, first_vertex, index_count, vertex_count) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        if first_index as usize + index_count as usize > ni || first_vertex as usize + vertex_count as usize > n {
            return Err(format!("sub-mesh {s} out of range"));
        }
        let tag = r.u8()?;
        if tag != 1 {
            return Err(format!("sub-mesh {s} material tag {tag}"));
        }
        let len = r.u32()? as usize;
        let blob = r.take(len)?;
        let material = parse_material_record(blob).map_err(|e| format!("sub-mesh {s}: {e}"))?;
        submeshes.push(MrSubMesh { first_index, first_vertex, index_count, vertex_count, material });
    }
    Ok(MrModel {
        version,
        flags,
        positions,
        normals: nor.chunks_exact(3).map(snorm3).collect(),
        tangents: tan.chunks_exact(3).map(snorm3).collect(),
        colors: col.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
        uvs: uv
            .chunks_exact(4)
            .map(|c| [f16(u16::from_le_bytes([c[0], c[1]])), f16(u16::from_le_bytes([c[2], c[3]]))])
            .collect(),
        indices,
        submeshes,
    })
}

// ---- global.txt -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Open,
    Close,
}

/// Splits MR's scene language into tokens: `//` comments, `"strings"` (backslashes are plain
/// characters: they separate folders), braces and words.
fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '/' if it.peek() == Some(&'/') => {
                for c in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '/' if it.peek() == Some(&'*') => {
                it.next();
                let mut prev = ' ';
                for c in it.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            }
            '"' => {
                let mut s = String::new();
                for c in it.by_ref() {
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                out.push(Tok::Str(s));
            }
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            c if c.is_whitespace() => {}
            c => {
                let mut s = String::from(c);
                while let Some(&n) = it.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    s.push(n);
                    it.next();
                }
                out.push(Tok::Word(s));
            }
        }
    }
    out
}

/// The index just past the block that opens at `toks[i]` (which must be `Open`).
fn block_end(toks: &[Tok], i: usize) -> usize {
    let mut depth = 0;
    for (k, t) in toks.iter().enumerate().skip(i) {
        match t {
            Tok::Open => depth += 1,
            Tok::Close => {
                depth -= 1;
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    toks.len()
}

fn number(t: &Tok) -> Option<f32> {
    match t {
        Tok::Word(w) => w.parse().ok(),
        _ => None,
    }
}

/// The parameters inside an `effect "X" { ... }` block: `name value` or `name { numbers }`
/// (`{ x 1 y 2 z 3 w 4 }` gives the four numbers).
fn effect_params(toks: &[Tok]) -> Vec<(String, Vec<f32>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if let Tok::Word(name) = &toks[i] {
            match toks.get(i + 1) {
                Some(Tok::Open) => {
                    let end = block_end(toks, i + 1);
                    let values: Vec<f32> = toks[i + 2..end - 1].iter().filter_map(number).collect();
                    out.push((name.clone(), values));
                    i = end;
                    continue;
                }
                Some(t) if number(t).is_some() && number(&toks[i]).is_none() => {
                    out.push((name.clone(), vec![number(t).unwrap()]));
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        i += 1;
    }
    out
}

/// Every top-level `MATERIAL { ... }` of a global.txt, by lower-case name.
pub fn parse_materials(src: &str) -> HashMap<String, MrMaterial> {
    let toks = tokenize(src);
    let mut out = HashMap::new();
    let mut i = 0;
    while i < toks.len() {
        let is_material = matches!(&toks[i], Tok::Word(w) if w == "MATERIAL") && toks.get(i + 1) == Some(&Tok::Open);
        if !is_material {
            if toks[i] == Tok::Open {
                // Skip other top-level blocks whole (MESH_IMPORT, TEXTURE_OBJECT...).
                i = block_end(&toks, i);
            } else {
                i += 1;
            }
            continue;
        }
        let end = block_end(&toks, i + 1);
        let body = &toks[i + 2..end - 1];
        let mut m = MrMaterial::default();
        let mut k = 0;
        while k < body.len() {
            match &body[k] {
                Tok::Word(w) if w == "name" => {
                    if let Some(Tok::Str(s)) = body.get(k + 1) {
                        m.name = s.clone();
                        k += 1;
                    }
                }
                Tok::Word(w) if w == "effect" => {
                    if let Some(Tok::Str(s)) = body.get(k + 1) {
                        m.effect = s.clone();
                        k += 1;
                        if body.get(k + 1) == Some(&Tok::Open) {
                            let e = block_end(body, k + 1);
                            m.params = effect_params(&body[k + 2..e - 1]);
                            k = e - 1;
                        }
                    }
                }
                Tok::Word(w) if w == "textures" => {
                    if body.get(k + 1) == Some(&Tok::Open) {
                        let e = block_end(body, k + 1);
                        m.textures = body[k + 2..e - 1]
                            .iter()
                            .filter_map(|t| if let Tok::Str(s) = t { Some(s.clone()) } else { None })
                            .collect();
                        k = e - 1;
                    }
                }
                Tok::Open => {
                    k = block_end(body, k) - 1;
                }
                _ => {}
            }
            k += 1;
        }
        if !m.name.is_empty() {
            out.insert(m.name.to_ascii_lowercase(), m);
        }
        i = end;
    }
    out
}

/// The archive path of a texture as MR's materials name it (`Trees\Bark1A` ->
/// `Textures/Trees/Bark1A.dds`).
pub fn texture_path(name: &str) -> String {
    let n = name.replace('\\', "/");
    if n.to_ascii_lowercase().ends_with(".dds") { format!("Textures/{n}") } else { format!("Textures/{n}.dds") }
}

/// The user's Megaton Rainfall install, its archives mounted read-only.
pub struct MrInstall {
    root: PathBuf,
    vfs: Vfs,
    materials: HashMap<String, MrMaterial>,
}

impl MrInstall {
    /// Mounts `root/Assets/{Models,Textures,Data}.fcm` and reads `Data/global.txt`'s materials.
    pub fn open(root: &Path) -> Result<Self, MrError> {
        let mut vfs = Vfs::new();
        for a in ARCHIVES {
            let path = root.join("Assets").join(a);
            if !path.is_file() {
                return Err(MrError::NotInstalled(root.to_path_buf(), a));
            }
            vfs.mount_pak(&path, "")?;
        }
        let materials = parse_materials(&vfs.read_string("Data/global.txt")?);
        Ok(MrInstall { root: root.to_path_buf(), vfs, materials })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    /// Reads and decodes `Models/<name>.bin`.
    pub fn model(&self, name: &str) -> Result<MrModel, MrError> {
        let bytes = self.vfs.read(&format!("Models/{name}.bin"))?;
        parse_model(&bytes).map_err(|reason| MrError::Model { name: name.to_string(), reason })
    }

    /// Whether a model of that name exists.
    pub fn has_model(&self, name: &str) -> bool {
        self.vfs.exists(&format!("Models/{name}.bin"))
    }

    /// The names of every model, without folder or extension.
    pub fn model_names(&self) -> Vec<String> {
        self.vfs
            .list_ext("Models", "bin")
            .into_iter()
            .filter_map(|p| p.rsplit('/').next().and_then(|f| f.strip_suffix(".bin").or_else(|| f.strip_suffix(".BIN"))).map(String::from))
            .collect()
    }

    /// A texture's DDS bytes, by the name a material gives it.
    pub fn texture_bytes(&self, name: &str) -> Result<Vec<u8>, MrError> {
        Ok(self.vfs.read(&texture_path(name))?)
    }

    /// A texture's DDS bytes and checked layout.
    pub fn texture(&self, name: &str) -> Result<(Vec<u8>, crate::dds::DdsInfo), MrError> {
        let bytes = self.texture_bytes(name)?;
        let info = crate::dds::parse_dds(&bytes).map_err(|source| MrError::Texture { name: name.to_string(), source })?;
        Ok((bytes, info))
    }

    pub fn has_texture(&self, name: &str) -> bool {
        self.vfs.exists(&texture_path(name))
    }

    /// global.txt's material of that name (case-insensitive).
    pub fn material(&self, name: &str) -> Option<&MrMaterial> {
        self.materials.get(&name.to_ascii_lowercase())
    }

    pub fn material_count(&self) -> usize {
        self.materials.len()
    }

    /// The material a sub-mesh is drawn with: global.txt's entry of its name where there is one
    /// (the game's own definition, which its model import re-reads), else the one embedded in the
    /// model (most imported props: `bench_0_Vertex`, `air_condition_0_Material__97-effect`).
    pub fn resolve<'a>(&'a self, embedded: &'a MrMaterial) -> &'a MrMaterial {
        self.material(&embedded.name).unwrap_or(embedded)
    }
}

#[cfg(test)]
mod tests;
