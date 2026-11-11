//! Garry's Mod's installed files, read only, so a game can load its props, textures and sounds at
//! run time (nothing from the game is copied into the project). Source engine formats:
//! - VPK archives (`*_dir.vpk` tables plus numbered data files), see [`GmodInstall::read`];
//! - models: `.mdl` (header, body parts, meshes), `.vvd` (vertices) and `.dx90.vtx` (triangle strips),
//!   [`GmodInstall::model`]; collision hulls from `.phy` (IVP compact surfaces), [`GmodInstall::collision`];
//! - materials: `.vmt` text (Valve KeyValues, [`parse_keyvalues`]) and `.vtf` textures, [`GmodInstall::texture`];
//! - sounds: RIFF WAVE with PCM or Microsoft ADPCM, [`GmodInstall::sound`] (MP3 data is refused);
//! - the stock spawn lists, [`GmodInstall::spawnlists`].
//!
//! Source units are inches, x forward, y left, z up. Nothing here writes to the install.

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::dds::{DdsFormat, DdsInfo};

/// Where the user's copy is installed.
pub const DEFAULT_GMOD_PATH: &str = "D:/SteamLibrary/steamapps/common/GarrysMod";

/// The archives read, in this order (the first that holds a file wins), relative to the install.
const ARCHIVES: [&str; 6] = [
    "garrysmod/garrysmod_dir.vpk",
    "sourceengine/hl2_misc_dir.vpk",
    "sourceengine/hl2_sound_misc_dir.vpk",
    "sourceengine/hl2_textures_dir.vpk",
    "garrysmod/fallbacks_dir.vpk",
    "sourceengine/content_hl2_dir.vpk",
];

/// The mesh winding: Source's triangle lists come out clockwise seen from outside; flipped here so
/// that `cross(b - a, c - a)` points along the normals (measured by the tests).
const FLIP_WINDING: bool = true;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmodError {
    /// The file (or the install) is not there.
    NotFound(String),
    /// It is there but wrong, or in a format this reader doesn't do.
    Bad(String),
}

impl std::fmt::Display for GmodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GmodError::NotFound(s) => write!(f, "Garry's Mod: {s} not found"),
            GmodError::Bad(s) => write!(f, "Garry's Mod: {s}"),
        }
    }
}

impl std::error::Error for GmodError {}

fn bad<T>(what: impl Into<String>) -> Result<T, GmodError> {
    Err(GmodError::Bad(what.into()))
}

// ---------------------------------------------------------------------------------------------
// Byte reading
// ---------------------------------------------------------------------------------------------

fn bytes_at<const N: usize>(b: &[u8], off: usize) -> Result<[u8; N], GmodError> {
    off.checked_add(N)
        .and_then(|end| b.get(off..end))
        .map(|s| {
            let mut a = [0u8; N];
            a.copy_from_slice(s);
            a
        })
        .ok_or_else(|| GmodError::Bad(format!("truncated data (wanted {N} bytes at {off} of {})", b.len())))
}

fn u8_at(b: &[u8], off: usize) -> Result<u8, GmodError> {
    Ok(bytes_at::<1>(b, off)?[0])
}
fn u16_at(b: &[u8], off: usize) -> Result<u16, GmodError> {
    Ok(u16::from_le_bytes(bytes_at(b, off)?))
}
fn i16_at(b: &[u8], off: usize) -> Result<i16, GmodError> {
    Ok(i16::from_le_bytes(bytes_at(b, off)?))
}
fn u32_at(b: &[u8], off: usize) -> Result<u32, GmodError> {
    Ok(u32::from_le_bytes(bytes_at(b, off)?))
}
fn i32_at(b: &[u8], off: usize) -> Result<i32, GmodError> {
    Ok(i32::from_le_bytes(bytes_at(b, off)?))
}
fn f32_at(b: &[u8], off: usize) -> Result<f32, GmodError> {
    Ok(f32::from_le_bytes(bytes_at(b, off)?))
}
fn vec3_at(b: &[u8], off: usize) -> Result<[f32; 3], GmodError> {
    Ok([f32_at(b, off)?, f32_at(b, off + 4)?, f32_at(b, off + 8)?])
}

/// A file offset made of a base and a (possibly negative) relative offset from the file.
fn rel(base: usize, off: i32) -> Result<usize, GmodError> {
    let v = base as i64 + off as i64;
    if v < 0 {
        bad("negative offset")
    } else {
        Ok(v as usize)
    }
}

/// A non-negative count read from a file, refused when absurdly large (corrupt data).
fn count_at(b: &[u8], off: usize) -> Result<usize, GmodError> {
    let n = i32_at(b, off)?;
    if !(0..=10_000_000).contains(&n) {
        return bad(format!("bad count {n}"));
    }
    Ok(n as usize)
}

/// The NUL-terminated string at `off`.
fn cstr_at(b: &[u8], off: usize) -> String {
    let s = b.get(off..).unwrap_or(&[]);
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).into_owned()
}

/// A game path as the archives key it: lowercase, `/` separators, no leading slash.
fn norm_path(p: &str) -> String {
    let mut s = p.trim().replace('\\', "/").to_ascii_lowercase();
    while s.contains("//") {
        s = s.replace("//", "/"); // some spawn lists have "models//player/..."
    }
    s.trim_start_matches('/').to_string()
}

// ---------------------------------------------------------------------------------------------
// VPK
// ---------------------------------------------------------------------------------------------

struct VpkEntry {
    preload: Vec<u8>,
    archive: u16,
    offset: u32,
    length: u32,
}

/// One `*_dir.vpk`: its whole file table, read up front; file contents are read when asked for.
struct Vpk {
    dir_path: PathBuf,
    /// The path of the numbered files without `_NNN.vpk`.
    prefix: String,
    /// Where data kept inside the dir file starts (after the header and the tree).
    data_start: u64,
    entries: HashMap<String, VpkEntry>,
}

impl Vpk {
    fn open(dir_path: &Path) -> Result<Vpk, GmodError> {
        let b = std::fs::read(dir_path).map_err(|e| GmodError::NotFound(format!("{}: {e}", dir_path.display())))?;
        if u32_at(&b, 0)? != 0x55AA_1234 {
            return bad(format!("{} is not a VPK", dir_path.display()));
        }
        let version = u32_at(&b, 4)?;
        let tree_size = u32_at(&b, 8)? as usize;
        let header = match version {
            1 => 12,
            2 => 28,
            v => return bad(format!("VPK version {v}")),
        };
        let tree = b
            .get(header..header + tree_size)
            .ok_or_else(|| GmodError::Bad("VPK tree is cut short".into()))?;
        let mut pos = 0usize;
        // Reads the next string; an empty result ends a level. A " " name means empty too.
        let next = |pos: &mut usize| -> Result<String, GmodError> {
            let s = tree.get(*pos..).ok_or_else(|| GmodError::Bad("VPK tree overrun".into()))?;
            let end = s.iter().position(|&c| c == 0).ok_or_else(|| GmodError::Bad("VPK string unterminated".into()))?;
            let out = String::from_utf8_lossy(&s[..end]).into_owned();
            *pos += end + 1;
            Ok(out)
        };
        let mut entries = HashMap::new();
        loop {
            let ext = next(&mut pos)?;
            if ext.is_empty() {
                break;
            }
            loop {
                let dir = next(&mut pos)?;
                if dir.is_empty() {
                    break;
                }
                loop {
                    let name = next(&mut pos)?;
                    if name.is_empty() {
                        break;
                    }
                    let preload_len = u16_at(tree, pos + 4)? as usize;
                    let archive = u16_at(tree, pos + 6)?;
                    let offset = u32_at(tree, pos + 8)?;
                    let length = u32_at(tree, pos + 12)?;
                    let preload = tree
                        .get(pos + 18..pos + 18 + preload_len)
                        .ok_or_else(|| GmodError::Bad("VPK preload bytes cut short".into()))?
                        .to_vec();
                    pos += 18 + preload_len;
                    let mut key = String::new();
                    let (dir, ext2) = (dir.trim(), ext.trim());
                    if !dir.is_empty() {
                        key.push_str(dir);
                        key.push('/');
                    }
                    key.push_str(name.trim());
                    if !ext2.is_empty() {
                        key.push('.');
                        key.push_str(ext2);
                    }
                    entries.insert(key.to_ascii_lowercase(), VpkEntry { preload, archive, offset, length });
                }
            }
        }
        let full = dir_path.to_string_lossy().into_owned();
        let prefix = full.strip_suffix("_dir.vpk").unwrap_or(&full).to_string();
        Ok(Vpk { dir_path: dir_path.to_path_buf(), prefix, data_start: (header + tree_size) as u64, entries })
    }

    fn read(&self, key: &str) -> Option<Vec<u8>> {
        let e = self.entries.get(key)?;
        let mut out = Vec::with_capacity(e.preload.len() + e.length as usize);
        out.extend_from_slice(&e.preload);
        if e.length > 0 {
            let (path, offset) = if e.archive == 0x7FFF {
                (self.dir_path.clone(), self.data_start + e.offset as u64)
            } else {
                (PathBuf::from(format!("{}_{:03}.vpk", self.prefix, e.archive)), e.offset as u64)
            };
            let mut f = File::open(path).ok()?;
            f.seek(SeekFrom::Start(offset)).ok()?;
            let start = out.len();
            out.resize(start + e.length as usize, 0);
            f.read_exact(&mut out[start..]).ok()?;
        }
        Some(out)
    }
}

// ---------------------------------------------------------------------------------------------
// The install
// ---------------------------------------------------------------------------------------------

/// Garry's Mod's files: loose ones under `garrysmod/` and the archives.
pub struct GmodInstall {
    root: PathBuf,
    vpks: Vec<Vpk>,
}

impl GmodInstall {
    /// `root` is the GarrysMod folder. Opens the VPK directories (only the `*_dir.vpk` tables are read
    /// up front); an archive that is missing is skipped.
    pub fn open(root: &Path) -> Result<Self, GmodError> {
        if !root.join("garrysmod").is_dir() {
            return Err(GmodError::NotFound(format!("{} has no garrysmod folder", root.display())));
        }
        let mut vpks = Vec::new();
        for rel in ARCHIVES {
            let p = root.join(rel);
            if p.is_file() {
                vpks.push(Vpk::open(&p)?);
            }
        }
        if vpks.is_empty() {
            return Err(GmodError::NotFound(format!("no VPK archives under {}", root.display())));
        }
        Ok(GmodInstall { root: root.to_path_buf(), vpks })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A game file by its path inside the game ("sound/weapons/pistol/pistol_fire2.wav"):
    /// case-insensitive, `/` or `\`. Loose files under `<root>/garrysmod/` win, then the archives in
    /// the order of [`ARCHIVES`].
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let key = norm_path(path);
        if key.is_empty() {
            return None;
        }
        if let Some(p) = self.loose(&key) {
            if let Ok(b) = std::fs::read(p) {
                return Some(b);
            }
        }
        self.vpks.iter().find_map(|v| v.read(&key))
    }

    pub fn exists(&self, path: &str) -> bool {
        let key = norm_path(path);
        !key.is_empty() && (self.loose(&key).is_some() || self.vpks.iter().any(|v| v.entries.contains_key(&key)))
    }

    /// The path of a loose file, when there is one. Paths that climb out of the folder are refused.
    fn loose(&self, key: &str) -> Option<PathBuf> {
        if key.split('/').any(|c| c == ".." || c.contains(':')) {
            return None;
        }
        let p = self.root.join("garrysmod").join(key);
        p.is_file().then_some(p)
    }

    /// Every archived file whose path starts with `prefix` and ends with `ext` (lowercase), sorted.
    pub fn list(&self, prefix: &str, ext: &str) -> Vec<String> {
        let (prefix, ext) = (norm_path(prefix), ext.to_ascii_lowercase());
        let mut out: BTreeSet<&String> = BTreeSet::new();
        for v in &self.vpks {
            out.extend(v.entries.keys().filter(|k| k.starts_with(&prefix) && k.ends_with(&ext)));
        }
        out.into_iter().cloned().collect()
    }

    /// "models/props_c17/oildrum001.mdl" -> its LOD 0 render mesh (reads the .mdl, .vvd and .dx90.vtx).
    pub fn model(&self, mdl_path: &str) -> Result<GmodModel, GmodError> {
        let norm = norm_path(mdl_path);
        let base = norm.strip_suffix(".mdl").unwrap_or(&norm).to_string();
        let get = |ext: &str| self.read(&format!("{base}.{ext}"));
        let mdl = get("mdl").ok_or_else(|| GmodError::NotFound(format!("{base}.mdl")))?;
        let vvd = get("vvd").ok_or_else(|| GmodError::NotFound(format!("{base}.vvd")))?;
        let vtx = get("dx90.vtx")
            .or_else(|| get("dx80.vtx"))
            .or_else(|| get("sw.vtx"))
            .ok_or_else(|| GmodError::NotFound(format!("{base}.dx90.vtx")))?;
        build_model(mdl_path, &mdl, &vvd, &vtx, &|p| self.exists(p))
            .map_err(|e| match e {
                GmodError::Bad(s) => GmodError::Bad(format!("model {mdl_path}: {s}")),
                other => other,
            })
    }

    /// The collision pieces of a model from its .phy file (convex hulls, as point lists in the model's
    /// own units and axes: those of the studiohdr's hull, which for most props are the mesh's too; a
    /// model whose idle pose is moved or turned from its bind pose has them in the idle pose's),
    /// None when it has no .phy or it can't be read.
    pub fn collision(&self, mdl_path: &str) -> Option<Vec<Vec<[f32; 3]>>> {
        let norm = norm_path(mdl_path);
        let base = norm.strip_suffix(".mdl").unwrap_or(&norm);
        parse_phy(&self.read(&format!("{base}.phy"))?)
    }

    /// "models/props_c17/oil_drum001a" (no "materials/" prefix, no extension) -> the parsed .vmt.
    pub fn material(&self, name: &str) -> Option<GmodMaterial> {
        let n = norm_path(name);
        let n = n.strip_prefix("materials/").unwrap_or(&n);
        let n = n.strip_suffix(".vmt").unwrap_or(n);
        let text = self.read(&format!("materials/{n}.vmt"))?;
        let (shader, props) = self.vmt_props(&String::from_utf8_lossy(&text), 0)?;
        Some(material_from(shader, &props))
    }

    /// The shader and the keys (lowercase) of a .vmt's block. A `patch` material takes the included
    /// file's keys, then its `insert` and `replace` blocks are laid over them.
    fn vmt_props(&self, text: &str, depth: u32) -> Option<(String, HashMap<String, String>)> {
        let kvs = parse_keyvalues(text);
        let root = kvs.first()?;
        let list = root.list()?;
        let shader = root.key.to_ascii_lowercase();
        let mut props = HashMap::new();
        if shader == "patch" {
            if depth >= 4 {
                return None;
            }
            let inc = kv_text(list, "include")?;
            let inc = norm_path(inc);
            let inc_text = self.read(&inc)?;
            let (inner_shader, inner) = self.vmt_props(&String::from_utf8_lossy(&inc_text), depth + 1)?;
            props = inner;
            for block in ["insert", "replace"] {
                if let Some(KvValue::List(items)) = kv_get(list, block).map(|k| &k.value) {
                    for it in items {
                        if let KvValue::Text(v) = &it.value {
                            props.insert(it.key.to_ascii_lowercase(), v.clone());
                        }
                    }
                }
            }
            return Some((inner_shader, props));
        }
        for it in list {
            if let KvValue::Text(v) = &it.value {
                props.insert(it.key.to_ascii_lowercase(), v.clone());
            }
        }
        Some((shader, props))
    }

    /// A texture by the name a material gives ("models/props_c17/oil_drum001a"): reads
    /// `materials/NAME.vtf`.
    pub fn texture(&self, name: &str) -> Result<GmodTexture, GmodError> {
        let n = norm_path(name);
        let n = n.strip_prefix("materials/").unwrap_or(&n);
        let n = n.strip_suffix(".vtf").unwrap_or(n);
        let path = format!("materials/{n}.vtf");
        let b = self.read(&path).ok_or_else(|| GmodError::NotFound(path.clone()))?;
        parse_vtf(&b).map_err(|e| match e {
            GmodError::Bad(s) => GmodError::Bad(format!("texture {path}: {s}")),
            other => other,
        })
    }

    /// "weapons/pistol/pistol_fire2.wav" (under sound/) decoded to 16-bit PCM. The character
    /// prefixes sound scripts put in front of a name (`)`, `^`, `*` and so on) are dropped.
    pub fn sound(&self, path: &str) -> Result<Pcm, GmodError> {
        let n = norm_path(path);
        let n = n.trim_start_matches(|c: char| ")(^*#<>@~?!$}".contains(c));
        let n = n.strip_prefix("sound/").unwrap_or(n);
        let full = format!("sound/{n}");
        let b = self.read(&full).ok_or_else(|| GmodError::NotFound(full.clone()))?;
        parse_wav(&b).map_err(|e| match e {
            GmodError::Bad(s) => GmodError::Bad(format!("sound {full}: {s}")),
            other => other,
        })
    }

    /// The stock spawn lists (`garrysmod/settings/spawnlist_default/*.txt`), in file-name order.
    pub fn spawnlists(&self) -> Vec<SpawnList> {
        let dir = self.root.join("garrysmod/settings/spawnlist_default");
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
            .collect();
        files.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
        files
            .iter()
            .filter_map(|p| {
                let text = std::fs::read(p).ok()?;
                let stem = p.file_stem()?.to_string_lossy().into_owned();
                parse_spawnlist(&String::from_utf8_lossy(&text), &stem)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// KeyValues
// ---------------------------------------------------------------------------------------------

/// One entry of Valve KeyValues text: a key with a text value or a list of entries.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyValues {
    pub key: String,
    pub value: KvValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KvValue {
    Text(String),
    List(Vec<KeyValues>),
}

impl KeyValues {
    pub fn text(&self) -> Option<&str> {
        match &self.value {
            KvValue::Text(s) => Some(s),
            KvValue::List(_) => None,
        }
    }

    pub fn list(&self) -> Option<&[KeyValues]> {
        match &self.value {
            KvValue::List(l) => Some(l),
            KvValue::Text(_) => None,
        }
    }
}

/// The first entry with this key (case-insensitive).
pub fn kv_get<'a>(list: &'a [KeyValues], key: &str) -> Option<&'a KeyValues> {
    list.iter().find(|k| k.key.eq_ignore_ascii_case(key))
}

/// The text value of the first entry with this key.
pub fn kv_text<'a>(list: &'a [KeyValues], key: &str) -> Option<&'a str> {
    list.iter().filter(|k| k.key.eq_ignore_ascii_case(key)).find_map(|k| k.text())
}

enum Tok {
    Str(String),
    Open,
    Close,
    End,
}

struct KvLexer<'a> {
    s: &'a [u8],
    pos: usize,
}

impl KvLexer<'_> {
    fn next(&mut self) -> Tok {
        loop {
            while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.s[self.pos.min(self.s.len())..].starts_with(b"//") {
                while self.pos < self.s.len() && self.s[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
        let Some(&c) = self.s.get(self.pos) else { return Tok::End };
        match c {
            b'{' => {
                self.pos += 1;
                Tok::Open
            }
            b'}' => {
                self.pos += 1;
                Tok::Close
            }
            b'"' => {
                // Backslashes stay as they are: paths use them, and Valve's files hardly ever escape.
                let start = self.pos + 1;
                let end = self.s[start..].iter().position(|&x| x == b'"').map_or(self.s.len(), |p| start + p);
                self.pos = (end + 1).min(self.s.len());
                Tok::Str(String::from_utf8_lossy(&self.s[start..end]).into_owned())
            }
            _ => {
                let start = self.pos;
                while self.pos < self.s.len() {
                    let x = self.s[self.pos];
                    if x.is_ascii_whitespace() || matches!(x, b'"' | b'{' | b'}') {
                        break;
                    }
                    self.pos += 1;
                }
                Tok::Str(String::from_utf8_lossy(&self.s[start..self.pos]).into_owned())
            }
        }
    }
}

/// How deep braces may nest before the parser gives up on the rest (bad input must not overflow the stack).
const KV_MAX_DEPTH: u32 = 48;

fn kv_list(lex: &mut KvLexer, depth: u32, top: bool) -> Vec<KeyValues> {
    let mut out = Vec::new();
    loop {
        match lex.next() {
            Tok::Str(key) => match lex.next() {
                Tok::Str(v) => out.push(KeyValues { key, value: KvValue::Text(v) }),
                Tok::Open => {
                    if depth >= KV_MAX_DEPTH {
                        return out;
                    }
                    let inner = kv_list(lex, depth + 1, false);
                    out.push(KeyValues { key, value: KvValue::List(inner) });
                }
                Tok::Close => {
                    if !top {
                        return out;
                    }
                }
                Tok::End => return out,
            },
            Tok::Open => {
                // A brace with no key: read it and keep it under an empty key.
                if depth >= KV_MAX_DEPTH {
                    return out;
                }
                let inner = kv_list(lex, depth + 1, false);
                out.push(KeyValues { key: String::new(), value: KvValue::List(inner) });
            }
            Tok::Close => {
                if !top {
                    return out;
                }
            }
            Tok::End => return out,
        }
    }
}

/// Valve KeyValues text (VMT, spawnlists, sound scripts, the text at the end of a .phy): quoted and
/// bare tokens, `//` comments, nested braces, duplicate keys kept in order. Never panics on bad input.
pub fn parse_keyvalues(text: &str) -> Vec<KeyValues> {
    let mut lex = KvLexer { s: text.as_bytes(), pos: 0 };
    kv_list(&mut lex, 0, true)
}

// ---------------------------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------------------------

/// A model's LOD 0 render mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct GmodModel {
    /// The path asked for.
    pub name: String,
    /// The header's mass (kg).
    pub mass: f32,
    /// The header's surface property, e.g. `metal_barrel`.
    pub surface_prop: String,
    pub hull_min: [f32; 3],
    pub hull_max: [f32; 3],
    /// The header's STUDIOHDR_FLAGS_STATIC_PROP.
    pub static_prop: bool,
    pub bones: usize,
    /// One per mesh of model 0 of each body part, skin family 0.
    pub meshes: Vec<GmodMesh>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmodMesh {
    /// The material, resolved: the first cdtexture folder with `materials/<dir><name>.vmt` in the game
    /// (lowercase, forward slashes, no extension); if none has it, the first folder and the name.
    pub material: String,
    /// Source units and axes, bind pose.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// As the file has them: v runs down the image.
    pub uvs: Vec<[f32; 2]>,
    /// A triangle list, counter-clockwise seen from outside.
    pub indices: Vec<u32>,
}

/// One vertex of the VVD: position, normal, uv.
#[derive(Clone, Copy)]
struct VvdVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

/// The VVD's LOD 0 vertices, with the fixup table applied.
fn read_vvd(b: &[u8]) -> Result<Vec<VvdVertex>, GmodError> {
    if b.get(0..4) != Some(b"IDSV") {
        return bad("vvd: bad signature");
    }
    let lod0 = count_at(b, 12 + 4)?; // numLODVertexes[0]
    let num_fixups = count_at(b, 48)?;
    let fixup_start = u32_at(b, 52)? as usize;
    let vertex_start = u32_at(b, 56)? as usize;
    let vertex = |i: usize| -> Result<VvdVertex, GmodError> {
        let o = vertex_start + i * 48 + 16;
        Ok(VvdVertex {
            pos: vec3_at(b, o)?,
            normal: vec3_at(b, o + 12)?,
            uv: [f32_at(b, o + 24)?, f32_at(b, o + 28)?],
        })
    };
    let mut out = Vec::new();
    if num_fixups == 0 {
        for i in 0..lod0 {
            out.push(vertex(i)?);
        }
    } else {
        for f in 0..num_fixups {
            let o = fixup_start + f * 12;
            let (lod, src, n) = (i32_at(b, o)?, count_at(b, o + 4)?, count_at(b, o + 8)?);
            if lod < 0 {
                continue; // only LOD 0's list: a fixup for lod >= 0 belongs to it
            }
            for i in 0..n {
                out.push(vertex(src + i)?);
            }
        }
    }
    Ok(out)
}

/// Builds the model from the three files. `exists` says whether a game file (a .vmt) is there.
fn build_model(
    name: &str,
    mdl: &[u8],
    vvd: &[u8],
    vtx: &[u8],
    exists: &dyn Fn(&str) -> bool,
) -> Result<GmodModel, GmodError> {
    if mdl.get(0..4) != Some(b"IDST") {
        return bad("mdl: bad signature");
    }
    let version = i32_at(mdl, 4)?;
    if !(44..=49).contains(&version) {
        return bad(format!("mdl version {version}"));
    }
    let hull_min = vec3_at(mdl, 104)?;
    let hull_max = vec3_at(mdl, 116)?;
    let flags = u32_at(mdl, 152)?;
    let bones = count_at(mdl, 156)?;
    let num_textures = count_at(mdl, 204)?;
    let texture_index = u32_at(mdl, 208)? as usize;
    let num_cd = count_at(mdl, 212)?;
    let cd_index = u32_at(mdl, 216)? as usize;
    let num_skin_ref = count_at(mdl, 220)?;
    let skin_index = u32_at(mdl, 228)? as usize;
    let num_body_parts = count_at(mdl, 232)?;
    let body_part_index = u32_at(mdl, 236)? as usize;
    let surf_index = u32_at(mdl, 308)? as usize;
    let mass = f32_at(mdl, 328)?;

    // The folders the textures live in, then the texture names.
    let mut cds = Vec::new();
    for i in 0..num_cd {
        let off = u32_at(mdl, cd_index + i * 4)? as usize;
        let mut d = norm_path(&cstr_at(mdl, off));
        if !d.is_empty() && !d.ends_with('/') {
            d.push('/');
        }
        cds.push(d);
    }
    if cds.is_empty() {
        cds.push(String::new());
    }
    let mut tex_names = Vec::new();
    for i in 0..num_textures {
        let o = texture_index + i * 64;
        let rel_name = i32_at(mdl, o)?;
        tex_names.push(norm_path(&cstr_at(mdl, rel(o, rel_name)?)));
    }
    let resolve = |tex: &str| -> String {
        for cd in &cds {
            let cand = format!("{cd}{tex}");
            if exists(&format!("materials/{cand}.vmt")) {
                return cand;
            }
        }
        format!("{}{tex}", cds[0])
    };

    let verts = read_vvd(vvd)?;

    // VTX header: numBodyParts @28, bodyPartOffset @32.
    let vtx_version = i32_at(vtx, 0)?;
    if vtx_version != 7 {
        return bad(format!("vtx version {vtx_version}"));
    }
    let vtx_bp_count = count_at(vtx, 28)?;
    let vtx_bp_off = u32_at(vtx, 32)? as usize;
    // MDL 49 added two ints to every strip group header.
    let sg_size = if version >= 49 { 33 } else { 25 };

    let mut meshes = Vec::new();
    for bp in 0..num_body_parts.min(vtx_bp_count) {
        let bp_off = body_part_index + bp * 16;
        let num_models = count_at(mdl, bp_off + 4)?;
        if num_models == 0 {
            continue;
        }
        let model_off = rel(bp_off, i32_at(mdl, bp_off + 12)?)?; // model 0
        let num_meshes = count_at(mdl, model_off + 72)?;
        let mesh_index = rel(model_off, i32_at(mdl, model_off + 76)?)?;
        let vertex_index = u32_at(mdl, model_off + 84)? as usize;
        let model_base = vertex_index / 48;

        let vbp = vtx_bp_off + bp * 8;
        if count_at(vtx, vbp)? == 0 {
            continue;
        }
        let vmodel = rel(vbp, i32_at(vtx, vbp + 4)?)?;
        if count_at(vtx, vmodel)? == 0 {
            continue;
        }
        let vlod = rel(vmodel, i32_at(vtx, vmodel + 4)?)?; // LOD 0
        let vtx_meshes = count_at(vtx, vlod)?;
        let vmesh_index = rel(vlod, i32_at(vtx, vlod + 4)?)?;

        for k in 0..num_meshes.min(vtx_meshes) {
            let m = mesh_index + k * 116;
            let material = i32_at(mdl, m)?;
            let mesh_vertex_offset = count_at(mdl, m + 12)?;
            // Skin family 0: the table maps the mesh's material number to a texture.
            let tex_i = if num_skin_ref > 0 && material >= 0 {
                u16_at(mdl, skin_index + (material as usize).min(num_skin_ref - 1) * 2)? as usize
            } else {
                material.max(0) as usize
            };
            let tex = tex_names.get(tex_i).cloned().unwrap_or_default();

            let mut mesh = GmodMesh {
                material: resolve(&tex),
                positions: Vec::new(),
                normals: Vec::new(),
                uvs: Vec::new(),
                indices: Vec::new(),
            };
            let mut remap: HashMap<usize, u32> = HashMap::new();
            let mut corner = |orig: u16, mesh: &mut GmodMesh| -> Result<u32, GmodError> {
                let gi = model_base + mesh_vertex_offset + orig as usize;
                if let Some(&i) = remap.get(&gi) {
                    return Ok(i);
                }
                let v = verts.get(gi).ok_or_else(|| GmodError::Bad(format!("vertex {gi} of {} out of range", verts.len())))?;
                let i = mesh.positions.len() as u32;
                mesh.positions.push(v.pos);
                mesh.normals.push(v.normal);
                mesh.uvs.push(v.uv);
                remap.insert(gi, i);
                Ok(i)
            };

            let vm = vmesh_index + k * 9;
            let num_groups = count_at(vtx, vm)?;
            let groups_off = rel(vm, i32_at(vtx, vm + 4)?)?;
            for g in 0..num_groups {
                let sg = groups_off + g * sg_size;
                let (nv, vo) = (count_at(vtx, sg)?, rel(sg, i32_at(vtx, sg + 4)?)?);
                let (ni, io) = (count_at(vtx, sg + 8)?, rel(sg, i32_at(vtx, sg + 12)?)?);
                let (ns, so) = (count_at(vtx, sg + 16)?, rel(sg, i32_at(vtx, sg + 20)?)?);
                let mut orig = Vec::with_capacity(nv);
                for v in 0..nv {
                    orig.push(u16_at(vtx, vo + v * 9 + 4)?);
                }
                let index = |i: usize| -> Result<u16, GmodError> {
                    let li = u16_at(vtx, io + i * 2)? as usize;
                    orig.get(li).copied().ok_or_else(|| GmodError::Bad("strip index past the vertex list".into()))
                };
                if io + ni * 2 > vtx.len() {
                    return bad("vtx: indices cut short");
                }
                for s in 0..ns {
                    let st = so + s * 27;
                    let (sn, sio) = (count_at(vtx, st)?, count_at(vtx, st + 4)?);
                    let sflags = u8_at(vtx, st + 18)?;
                    let mut tri = |a: u16, b: u16, c: u16, mesh: &mut GmodMesh| -> Result<(), GmodError> {
                        let (a, b, c) = (corner(a, mesh)?, corner(b, mesh)?, corner(c, mesh)?);
                        if a == b || b == c || a == c {
                            return Ok(()); // degenerate: strips use them to join
                        }
                        if FLIP_WINDING {
                            mesh.indices.extend_from_slice(&[a, c, b]);
                        } else {
                            mesh.indices.extend_from_slice(&[a, b, c]);
                        }
                        Ok(())
                    };
                    if sflags & 1 != 0 {
                        for t in 0..sn / 3 {
                            let i = sio + t * 3;
                            tri(index(i)?, index(i + 1)?, index(i + 2)?, &mut mesh)?;
                        }
                    } else if sflags & 2 != 0 {
                        for t in 0..sn.saturating_sub(2) {
                            let i = sio + t;
                            let (a, b, c) = (index(i)?, index(i + 1)?, index(i + 2)?);
                            // every second triangle of a strip runs the other way round
                            if t % 2 == 0 {
                                tri(a, b, c, &mut mesh)?;
                            } else {
                                tri(b, a, c, &mut mesh)?;
                            }
                        }
                    }
                }
            }
            meshes.push(mesh);
        }
    }

    Ok(GmodModel {
        name: name.to_string(),
        mass,
        surface_prop: cstr_at(mdl, surf_index),
        hull_min,
        hull_max,
        static_prop: flags & 0x10 != 0,
        bones,
        meshes,
    })
}

// ---------------------------------------------------------------------------------------------
// Collision (.phy)
// ---------------------------------------------------------------------------------------------

/// IVP's metres to Source's inches.
const IVP_TO_INCHES: f32 = 1.0 / 0.0254;

/// The convex pieces of a .phy file: a header, then per solid its size, an optional `VPHY` header and an
/// IVP compact surface whose ledge tree leads to one convex ledge per piece. (The key text after the
/// solids, with their names and masses, is not read.) None when anything is not what it should be (no
/// wrong hulls).
fn parse_phy(b: &[u8]) -> Option<Vec<Vec<[f32; 3]>>> {
    let header = i32_at(b, 0).ok()?;
    if !(16..=256).contains(&header) {
        return None;
    }
    let solids = i32_at(b, 8).ok()?;
    if !(1..=1024).contains(&solids) {
        return None;
    }
    let mut pos = header as usize;
    let mut out = Vec::new();
    for _ in 0..solids {
        let size = u32_at(b, pos).ok()? as usize;
        let start = pos + 4;
        let body = b.get(start..start.checked_add(size)?)?;
        let surface = if body.get(0..4) == Some(b"VPHY") {
            let model_type = u16_at(body, 6).ok()?;
            if model_type != 0 {
                return None; // MOPP (a different tree): not read
            }
            let surface_size = u32_at(body, 8).ok()? as usize;
            body.get(28..28usize.checked_add(surface_size)?)?
        } else {
            body
        };
        walk_surface(surface, &mut out)?;
        pos = start + size;
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Collects the points of every ledge of one compact surface (48-byte header, `IVPS` at 44).
fn walk_surface(s: &[u8], out: &mut Vec<Vec<[f32; 3]>>) -> Option<()> {
    if s.get(44..48) != Some(b"IVPS") {
        return None;
    }
    let root = u32_at(s, 32).ok()? as usize;
    // Tree nodes are 28 bytes: right-son offset, ledge offset, centre, radius, box. A node with no
    // right son is a leaf whose ledge is at its own address + the ledge offset; otherwise its left son
    // is the next node and its right son at the offset.
    let mut stack = vec![root];
    let mut steps = 0;
    while let Some(n) = stack.pop() {
        steps += 1;
        if steps > 200_000 {
            return None;
        }
        let right = i32_at(s, n).ok()?;
        if right == 0 {
            let ledge = rel(n, i32_at(s, n + 4).ok()?).ok()?;
            out.push(read_ledge(s, ledge)?);
        } else {
            stack.push(rel(n, right).ok()?);
            stack.push(n + 28);
        }
    }
    Some(())
}

/// One ledge: header {point offset, client data, bitfield, triangle count, spare} then 16-byte
/// triangles whose three edges name their start points in their low 16 bits.
fn read_ledge(s: &[u8], l: usize) -> Option<Vec<[f32; 3]>> {
    let points = rel(l, i32_at(s, l).ok()?).ok()?;
    let n_tri = i16_at(s, l + 12).ok()?;
    if !(1..=4096).contains(&n_tri) {
        return None;
    }
    let mut used = BTreeSet::new();
    for t in 0..n_tri as usize {
        let tri = l + 16 + t * 16;
        for e in 0..3 {
            used.insert(u16_at(s, tri + 4 + e * 4).ok()?);
        }
    }
    let mut pts = Vec::with_capacity(used.len());
    for i in used {
        let o = points + i as usize * 16;
        let (x, y, z) = (f32_at(s, o).ok()?, f32_at(s, o + 4).ok()?, f32_at(s, o + 8).ok()?);
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            return None;
        }
        // IVP axes to Source's: x stays, y is IVP's z, z is minus IVP's y.
        pts.push([x * IVP_TO_INCHES, z * IVP_TO_INCHES, -y * IVP_TO_INCHES]);
    }
    Some(pts)
}

// ---------------------------------------------------------------------------------------------
// Materials
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct GmodMaterial {
    /// Lowercase, e.g. `vertexlitgeneric`.
    pub shader: String,
    /// The `$basetexture` (lowercase, forward slashes, no extension).
    pub base_texture: Option<String>,
    pub bump_map: Option<String>,
    pub alpha_test: bool,
    pub translucent: bool,
    pub no_cull: bool,
    pub self_illum: bool,
    /// `$color`, else `$color2`; white when neither is there.
    pub color: [f32; 3],
    pub surface_prop: Option<String>,
}

fn flag(props: &HashMap<String, String>, key: &str) -> bool {
    props.get(key).is_some_and(|v| {
        let v = v.trim();
        v == "1" || v.eq_ignore_ascii_case("true") || v.parse::<f32>().is_ok_and(|n| n != 0.0)
    })
}

fn texture_name(v: &str) -> String {
    let n = norm_path(v);
    n.strip_suffix(".vtf").unwrap_or(&n).to_string()
}

/// `[0.5 0.5 0.5]` (0..1) or `{255 128 0}` (0..255).
fn parse_color(v: &str) -> Option<[f32; 3]> {
    let v = v.trim();
    let scale = if v.starts_with('{') { 1.0 / 255.0 } else { 1.0 };
    let nums: Vec<f32> = v
        .trim_matches(|c| "[]{} ".contains(c))
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if nums.len() >= 3 {
        Some([nums[0] * scale, nums[1] * scale, nums[2] * scale])
    } else if nums.len() == 1 {
        Some([nums[0] * scale; 3])
    } else {
        None
    }
}

fn material_from(shader: String, props: &HashMap<String, String>) -> GmodMaterial {
    let color = props
        .get("$color")
        .and_then(|v| parse_color(v))
        .or_else(|| props.get("$color2").and_then(|v| parse_color(v)))
        .unwrap_or([1.0; 3]);
    GmodMaterial {
        shader,
        base_texture: props.get("$basetexture").map(|v| texture_name(v)).filter(|s| !s.is_empty()),
        bump_map: props
            .get("$bumpmap")
            .or_else(|| props.get("$normalmap"))
            .map(|v| texture_name(v))
            .filter(|s| !s.is_empty()),
        alpha_test: flag(props, "$alphatest"),
        translucent: flag(props, "$translucent"),
        no_cull: flag(props, "$nocull"),
        self_illum: flag(props, "$selfillum"),
        color,
        surface_prop: props.get("$surfaceprop").map(|v| v.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()),
    }
}

// ---------------------------------------------------------------------------------------------
// Textures (.vtf)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct GmodTexture {
    pub width: u32,
    pub height: u32,
    /// Bc1, Bc2, Bc3 or Rgba8.
    pub format: DdsFormat,
    /// Largest first; frame 0 and face 0 only.
    pub mips: Vec<Vec<u8>>,
    /// The file's flags say one-bit or eight-bit alpha.
    pub has_alpha: bool,
}

impl GmodTexture {
    /// One mip as RGBA8, row 0 at the top (the BC blocks are decoded here). Empty for a mip that
    /// isn't there.
    pub fn rgba(&self, mip: usize) -> Vec<u8> {
        let Some(data) = self.mips.get(mip) else { return Vec::new() };
        let (w, h) = ((self.width >> mip).max(1), (self.height >> mip).max(1));
        if self.format == DdsFormat::Rgba8 {
            return data.clone();
        }
        let info = DdsInfo { width: w, height: h, mips: 1, layers: 1, cube: false, format: self.format, data_offset: 0 };
        crate::dds::decode_level(data, &info, 0, 0).unwrap_or_else(|_| vec![0; (w * h * 4) as usize])
    }
}

/// Bytes of one image of a VTF format, None for a format number this reader doesn't know.
fn vtf_level_bytes(fmt: i32, w: u32, h: u32) -> Option<usize> {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let block = |unit: usize| w.div_ceil(4) * h.div_ceil(4) * unit;
    Some(match fmt {
        13 | 20 => block(8),
        14 | 15 => block(16),
        0 | 1 | 11 | 12 | 16 | 23 | 26 => w * h * 4,
        2 | 3 | 9 | 10 => w * h * 3,
        4 | 6 | 17 | 18 | 19 | 21 | 22 => w * h * 2,
        5 | 7 | 8 => w * h,
        24 | 25 => w * h * 8,
        _ => return None,
    })
}

/// Converts a plain (not block compressed) VTF image to RGBA8.
fn vtf_to_rgba(fmt: i32, d: &[u8]) -> Option<Vec<u8>> {
    let px = |n: usize, f: &dyn Fn(&[u8]) -> [u8; 4]| -> Vec<u8> { d.chunks_exact(n).flat_map(|t| f(t)).collect() };
    let e5 = |v: u16, shift: u32| ((((v >> shift) & 31) as u32 * 255 + 15) / 31) as u8;
    let e6 = |v: u16| ((((v >> 5) & 63) as u32 * 255 + 31) / 63) as u8;
    Some(match fmt {
        0 => d.to_vec(),
        1 => px(4, &|t| [t[3], t[2], t[1], t[0]]),
        2 => px(3, &|t| [t[0], t[1], t[2], 255]),
        3 => px(3, &|t| [t[2], t[1], t[0], 255]),
        4 => px(2, &|t| {
            let v = u16::from_le_bytes([t[0], t[1]]);
            [e5(v, 11), e6(v), e5(v, 0), 255]
        }),
        5 => px(1, &|t| [t[0], t[0], t[0], 255]),
        6 => px(2, &|t| [t[0], t[0], t[0], t[1]]),
        8 => px(1, &|t| [255, 255, 255, t[0]]),
        11 => px(4, &|t| [t[1], t[2], t[3], t[0]]),
        12 => px(4, &|t| [t[2], t[1], t[0], t[3]]),
        16 => px(4, &|t| [t[2], t[1], t[0], 255]),
        17 => px(2, &|t| {
            let v = u16::from_le_bytes([t[0], t[1]]);
            [e5(v, 0), e6(v), e5(v, 11), 255]
        }),
        _ => return None,
    })
}

fn parse_vtf(b: &[u8]) -> Result<GmodTexture, GmodError> {
    if b.get(0..4) != Some(b"VTF\0") {
        return bad("not a VTF");
    }
    let major = u32_at(b, 4)?;
    let minor = u32_at(b, 8)?;
    if major != 7 {
        return bad(format!("VTF version {major}.{minor}"));
    }
    let header_size = u32_at(b, 12)? as usize;
    let (width, height) = (u16_at(b, 16)? as u32, u16_at(b, 18)? as u32);
    let flags = u32_at(b, 20)?;
    let frames = (u16_at(b, 24)? as usize).max(1);
    let fmt = i32_at(b, 52)?;
    let mip_count = (u8_at(b, 56)? as usize).max(1);
    let low_fmt = i32_at(b, 57)?;
    let (low_w, low_h) = (u8_at(b, 61)? as u32, u8_at(b, 62)? as u32);
    let depth = if minor >= 2 { (u16_at(b, 63)? as usize).max(1) } else { 1 };
    if width == 0 || height == 0 {
        return bad("VTF of no size");
    }
    let faces = if flags & 0x4000 != 0 { if minor < 5 { 7 } else { 6 } } else { 1 };

    // Where the full size image data starts.
    let data_start = if minor >= 3 {
        let n = count_at(b, 68)?;
        let mut found = None;
        for i in 0..n.min(32) {
            let o = 80 + i * 8;
            if u32_at(b, o)? & 0x00FF_FFFF == 0x30 {
                found = Some(u32_at(b, o + 4)? as usize);
            }
        }
        found.ok_or_else(|| GmodError::Bad("VTF has no image data resource".into()))?
    } else {
        let low = if low_fmt >= 0 && low_w > 0 && low_h > 0 { vtf_level_bytes(low_fmt, low_w, low_h).unwrap_or(0) } else { 0 };
        header_size + low
    };

    // Mips are stored smallest first; inside one mip come frames, faces and slices, so frame 0 and
    // face 0 start the block.
    let size_of = |m: usize| -> Result<usize, GmodError> {
        let (w, h) = ((width >> m).max(1), (height >> m).max(1));
        let one = vtf_level_bytes(fmt, w, h).ok_or_else(|| GmodError::Bad(format!("VTF image format {fmt}")))?;
        Ok(one * frames * faces * depth)
    };
    let mut starts = vec![0usize; mip_count];
    let mut at = data_start;
    for m in (0..mip_count).rev() {
        starts[m] = at;
        at += size_of(m)?;
    }
    let (format, converted) = match fmt {
        13 | 20 => (DdsFormat::Bc1, false),
        14 => (DdsFormat::Bc2, false),
        15 => (DdsFormat::Bc3, false),
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 8 | 11 | 12 | 16 | 17 => (DdsFormat::Rgba8, true),
        _ => return bad(format!("VTF image format {fmt}")),
    };
    let mut mips = Vec::with_capacity(mip_count);
    for m in 0..mip_count {
        let (w, h) = ((width >> m).max(1), (height >> m).max(1));
        let n = vtf_level_bytes(fmt, w, h).unwrap_or(0);
        let raw = b
            .get(starts[m]..starts[m] + n)
            .ok_or_else(|| GmodError::Bad("VTF image data cut short".into()))?;
        mips.push(if converted {
            vtf_to_rgba(fmt, raw).ok_or_else(|| GmodError::Bad(format!("VTF image format {fmt}")))?
        } else {
            raw.to_vec()
        });
    }
    Ok(GmodTexture { width, height, format, mips, has_alpha: flags & 0x3000 != 0 })
}

// ---------------------------------------------------------------------------------------------
// Sounds
// ---------------------------------------------------------------------------------------------

/// Decoded sound: 16-bit samples, channels interleaved.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
    /// Where a loop starts, in sample frames (from a `smpl` or `cue ` chunk).
    pub loop_start: Option<u32>,
}

impl Pcm {
    pub fn seconds(&self) -> f32 {
        let frames = self.samples.len() / self.channels.max(1) as usize;
        frames as f32 / self.rate.max(1) as f32
    }

    /// A plain 16-bit PCM RIFF/WAVE file of these samples.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let ch = self.channels.max(1);
        let mut w = Vec::with_capacity(44 + data_len as usize);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&ch.to_le_bytes());
        w.extend_from_slice(&self.rate.to_le_bytes());
        w.extend_from_slice(&(self.rate * ch as u32 * 2).to_le_bytes());
        w.extend_from_slice(&(ch * 2).to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        for s in &self.samples {
            w.extend_from_slice(&s.to_le_bytes());
        }
        w
    }
}

const ADPCM_STD_COEFS: [(i32, i32); 7] = [(256, 0), (512, -256), (0, 0), (192, 64), (240, 0), (460, -208), (392, -232)];
const ADPCM_ADAPT: [i32; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230];

/// Microsoft ADPCM, block by block. A block starts with, for each channel, a predictor number, then
/// each channel's delta, newest sample and older sample; then 4-bit codes, high nibble first (left
/// channel first in stereo).
fn decode_ms_adpcm(data: &[u8], channels: usize, block_align: usize, per_block: usize, coefs: &[(i32, i32)]) -> Vec<i16> {
    let mut out = Vec::new();
    if channels == 0 || channels > 2 || block_align < 7 * channels {
        return out;
    }
    for block in data.chunks(block_align) {
        let head = 7 * channels;
        if block.len() < head {
            break;
        }
        let mut pred = [0usize; 2];
        let mut delta = [0i32; 2];
        let mut s1 = [0i32; 2];
        let mut s2 = [0i32; 2];
        for c in 0..channels {
            pred[c] = (block[c] as usize).min(coefs.len().saturating_sub(1));
            delta[c] = i16::from_le_bytes([block[channels + c * 2], block[channels + c * 2 + 1]]) as i32;
            s1[c] = i16::from_le_bytes([block[3 * channels + c * 2], block[3 * channels + c * 2 + 1]]) as i32;
            s2[c] = i16::from_le_bytes([block[5 * channels + c * 2], block[5 * channels + c * 2 + 1]]) as i32;
        }
        let mut frames = 2;
        for c in 0..channels {
            out.push(s2[c] as i16);
        }
        for c in 0..channels {
            out.push(s1[c] as i16);
        }
        let mut c = 0;
        'bytes: for &byte in &block[head..] {
            for code in [byte >> 4, byte & 15] {
                if per_block > 0 && frames >= per_block {
                    break 'bytes;
                }
                let (c1, c2) = coefs[pred[c]];
                let n = if code >= 8 { code as i32 - 16 } else { code as i32 };
                let p = ((s1[c] * c1 + s2[c] * c2) / 256 + n * delta[c]).clamp(-32768, 32767);
                delta[c] = ((ADPCM_ADAPT[code as usize] * delta[c]) / 256).max(16);
                s2[c] = s1[c];
                s1[c] = p;
                out.push(p as i16);
                c += 1;
                if c == channels {
                    c = 0;
                    frames += 1;
                }
            }
        }
        // A last block can end mid-frame: drop the half frame.
        let extra = out.len() % channels;
        out.truncate(out.len() - extra);
    }
    out
}

fn parse_wav(b: &[u8]) -> Result<Pcm, GmodError> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        let what = if b.starts_with(b"ID3") || b.get(0) == Some(&0xFF) { "MP3 data, not RIFF/WAVE" } else { "not a RIFF/WAVE file" };
        return bad(what);
    }
    let mut pos = 12;
    let mut fmt: Option<&[u8]> = None;
    let mut data: Option<&[u8]> = None;
    let mut fact: Option<u32> = None;
    let mut cue_start = None;
    let mut smpl_start = None;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32_at(b, pos + 4)? as usize;
        let body_start = pos + 8;
        // A data chunk that claims more than the file holds takes what is there.
        let body = &b[body_start..body_start.saturating_add(size).min(b.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" if data.is_none() => data = Some(body),
            b"fact" => fact = u32_at(body, 0).ok(),
            b"cue " => {
                if u32_at(body, 0).unwrap_or(0) > 0 {
                    cue_start = u32_at(body, 4 + 20).ok();
                }
            }
            b"smpl" => {
                if u32_at(body, 28).unwrap_or(0) > 0 {
                    smpl_start = u32_at(body, 36 + 8).ok();
                }
            }
            _ => {}
        }
        pos = body_start.saturating_add(size).saturating_add(size & 1);
    }
    let fmt = fmt.ok_or_else(|| GmodError::Bad("WAV has no fmt chunk".into()))?;
    let data = data.ok_or_else(|| GmodError::Bad("WAV has no data chunk".into()))?;
    let mut tag = u16_at(fmt, 0)?;
    let channels = u16_at(fmt, 2)?;
    let rate = u32_at(fmt, 4)?;
    let block_align = u16_at(fmt, 12)? as usize;
    let bits = u16_at(fmt, 14)?;
    if tag == 0xFFFE && fmt.len() >= 26 {
        tag = u16_at(fmt, 24)?; // the sub-format's first two bytes
    }
    if channels == 0 || rate == 0 {
        return bad("WAV with no channels or rate");
    }
    let samples: Vec<i16> = match (tag, bits) {
        (1, 8) => data.iter().map(|&v| ((v as i16) - 128) << 8).collect(),
        (1, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect(),
        (1, 24) => data.chunks_exact(3).map(|c| i16::from_le_bytes([c[1], c[2]])).collect(),
        (2, 4) => {
            let per_block = u16_at(fmt, 18).unwrap_or(0) as usize;
            let ncoef = (u16_at(fmt, 20).unwrap_or(0) as usize).min(64);
            let mut coefs = Vec::new();
            for i in 0..ncoef {
                match (i16_at(fmt, 22 + i * 4), i16_at(fmt, 24 + i * 4)) {
                    (Ok(a), Ok(c)) => coefs.push((a as i32, c as i32)),
                    _ => break,
                }
            }
            if coefs.len() < 7 {
                coefs = ADPCM_STD_COEFS.to_vec();
            }
            let mut s = decode_ms_adpcm(data, channels as usize, block_align, per_block, &coefs);
            if let Some(total) = fact {
                s.truncate((total as usize).saturating_mul(channels as usize).max(1).min(s.len()));
            }
            s
        }
        (85, _) => return bad("MP3 data (WAV format tag 85)"),
        (t, bits) => return bad(format!("WAV format tag {t} ({bits} bit) not supported")),
    };
    if samples.is_empty() {
        return bad("WAV holds no samples");
    }
    Ok(Pcm { rate, channels, samples, loop_start: smpl_start.or(cue_start) })
}

// ---------------------------------------------------------------------------------------------
// Spawn lists
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnList {
    /// The file's `name`, else the file name without its number.
    pub name: String,
    pub id: u32,
    pub parent: u32,
    /// The `model` entries, in order (lowercase, forward slashes).
    pub models: Vec<String>,
    /// The `header` entries' text, in order.
    pub headers: Vec<String>,
}

fn parse_spawnlist(text: &str, file_stem: &str) -> Option<SpawnList> {
    let kvs = parse_keyvalues(text);
    let list = kvs.iter().find_map(|k| k.list())?;
    let num = |key: &str| kv_text(list, key).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
    // "001-construction props" -> "construction props"
    let fallback = file_stem.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches(['-', '_', ' ']);
    let name = kv_text(list, "name").map(str::to_string).unwrap_or_else(|| fallback.to_string());
    let mut models = Vec::new();
    let mut headers = Vec::new();
    if let Some(items) = kv_get(list, "contents").and_then(|k| k.list()) {
        for item in items {
            let Some(fields) = item.list() else { continue };
            match kv_text(fields, "type").map(str::to_ascii_lowercase).as_deref() {
                Some("model") => {
                    if let Some(m) = kv_text(fields, "model") {
                        models.push(norm_path(m));
                    }
                }
                Some("header") => {
                    if let Some(t) = kv_text(fields, "text") {
                        headers.push(t.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    Some(SpawnList { name, id: num("id"), parent: num("parentid"), models, headers })
}

// ---------------------------------------------------------------------------------------------
// Tests (they skip quietly when Garry's Mod is not installed)
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn install() -> Option<&'static GmodInstall> {
        static INSTALL: OnceLock<Option<GmodInstall>> = OnceLock::new();
        let i = INSTALL.get_or_init(|| GmodInstall::open(Path::new(DEFAULT_GMOD_PATH)).ok());
        if i.is_none() {
            eprintln!("gmod tests: Garry's Mod not found at {DEFAULT_GMOD_PATH}, skipped");
        }
        i.as_ref()
    }

    const MODELS: [&str; 11] = [
        "models/props_c17/oildrum001.mdl",
        "models/props_junk/wood_crate001a.mdl",
        "models/props_borealis/bluebarrel001.mdl",
        "models/props_c17/furniturecouch001a.mdl",
        "models/props_junk/watermelon01.mdl",
        "models/weapons/w_pistol.mdl",
        "models/weapons/w_smg1.mdl",
        "models/weapons/w_shotgun.mdl",
        "models/weapons/w_crowbar.mdl",
        "models/weapons/w_physics.mdl",
        "models/weapons/v_pistol.mdl",
    ];

    const SOUNDS: [&str; 8] = [
        "weapons/pistol/pistol_fire2.wav",
        "weapons/smg1/smg1_fire1.wav",
        "weapons/shotgun/shotgun_fire7.wav",
        "weapons/crowbar/crowbar_impact1.wav",
        "player/footsteps/grass1.wav",
        "physics/metal/metal_barrel_impact_hard1.wav",
        "weapons/physcannon/hold_loop.wav",
        "garrysmod/ui_click.wav",
    ];

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn keyvalues_parse() {
        let kv = parse_keyvalues(
            "// c\n\"LightmappedGeneric\"\n{\n\t\"$basetexture\" \"a\\b\" // x\n\t$color [1 .5 0]\n\tproxies { a { b 1 } }\n\t\"k\" \"1\" \"k\" \"2\"\n}\n}}{ \"x\"",
        );
        let list = kv[0].list().unwrap();
        assert_eq!(kv_text(list, "$BASETEXTURE"), Some("a\\b"));
        assert_eq!(kv_text(list, "$color"), Some("[1"));
        assert!(list.iter().filter(|k| k.key == "k").count() == 2);
        for junk in ["", "\"", "{{{{", "\"a\" {", "a b c d {", &"{".repeat(5000), &"a {".repeat(5000)] {
            let _ = parse_keyvalues(junk);
        }
    }

    #[test]
    fn adpcm_silence_and_step() {
        // One mono block: predictor 0, delta 16, samples 0, 0, then codes 0x12 0x34 (4 samples).
        let mut block = vec![0u8];
        block.extend_from_slice(&16i16.to_le_bytes());
        block.extend_from_slice(&0i16.to_le_bytes());
        block.extend_from_slice(&0i16.to_le_bytes());
        block.extend_from_slice(&[0x12, 0x34]);
        let s = decode_ms_adpcm(&block, 1, 9, 6, &ADPCM_STD_COEFS);
        assert_eq!(s.len(), 6);
        assert_eq!(&s[..2], &[0, 0]);
        // code 1 from rest: 0 + 1*16 = 16; delta stays 16 (230/256 -> clamped to 16)
        assert_eq!(s[2], 16);
        assert!(s[3] > s[2]);
    }

    #[test]
    fn vpk_reads() {
        let Some(g) = install() else { return };
        let b = g.read("sound/weapons/pistol/pistol_fire2.wav").expect("pistol_fire2");
        assert_eq!(&b[0..4], b"RIFF");
        assert!(g.read("sound/weapons/pistol/no_such_file.wav").is_none());
        assert!(!g.exists("models/no/such.mdl"));
        assert!(g.exists("models/props_c17/oildrum001.mdl"));
        let c = g.read("SOUND\\Weapons\\Pistol\\PISTOL_fire2.WAV").expect("case and slashes");
        assert_eq!(b, c);
        assert!(g.read("../gmod.exe").is_none());
    }

    #[test]
    fn models_load() {
        let Some(g) = install() else { return };
        println!(
            "{:<45} {:>6} {:>6} {:>5} {:>7} {:<14} {:<26} materials",
            "model", "tris", "verts", "bones", "mass", "surfaceprop", "size (x y z)"
        );
        for path in MODELS {
            let m = g.model(path).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert!(!m.meshes.is_empty(), "{path}: no meshes");
            let ext = [m.hull_max[0] - m.hull_min[0], m.hull_max[1] - m.hull_min[1], m.hull_max[2] - m.hull_min[2]];
            let (mut tris, mut verts) = (0, 0);
            let (mut agree, mut total) = (0usize, 0usize);
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            let mut mats = Vec::new();
            for mesh in &m.meshes {
                assert_eq!(mesh.positions.len(), mesh.normals.len());
                assert_eq!(mesh.positions.len(), mesh.uvs.len());
                assert_eq!(mesh.indices.len() % 3, 0);
                assert!(!mesh.indices.is_empty(), "{path}: empty mesh {}", mesh.material);
                tris += mesh.indices.len() / 3;
                verts += mesh.positions.len();
                assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.positions.len()), "{path}: index out of range");
                for n in &mesh.normals {
                    let l = dot(*n, *n).sqrt();
                    assert!((l - 1.0).abs() < 0.02, "{path}: normal length {l}");
                }
                for p in &mesh.positions {
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                    if path.contains("/weapons/") {
                        // A weapon's hull is its idle pose's, turned about the origin from the bind
                        // pose the mesh is in: only the distance from the origin is comparable.
                        let far = |v: [f32; 3]| dot(v, v).sqrt();
                        let reach = far(m.hull_min).max(far(m.hull_max));
                        assert!(far(*p) <= reach * 1.25 + 2.0, "{path}: vertex {p:?} farther than the hull's reach {reach}");
                    } else {
                        for k in 0..3 {
                            let slack = 0.25 * ext[k] + 2.0;
                            assert!(
                                p[k] >= m.hull_min[k] - slack && p[k] <= m.hull_max[k] + slack,
                                "{path}: vertex {p:?} outside hull {:?}..{:?}",
                                m.hull_min,
                                m.hull_max
                            );
                        }
                    }
                }
                for t in mesh.indices.chunks_exact(3) {
                    let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    let n = cross(sub(mesh.positions[b], mesh.positions[a]), sub(mesh.positions[c], mesh.positions[a]));
                    if dot(n, n) < 1e-12 {
                        continue;
                    }
                    let nn = [
                        mesh.normals[a][0] + mesh.normals[b][0] + mesh.normals[c][0],
                        mesh.normals[a][1] + mesh.normals[b][1] + mesh.normals[c][1],
                        mesh.normals[a][2] + mesh.normals[b][2] + mesh.normals[c][2],
                    ];
                    total += 1;
                    if dot(n, nn) > 0.0 {
                        agree += 1;
                    }
                }
                // the material resolves, and its base texture loads
                assert!(g.exists(&format!("materials/{}.vmt", mesh.material)), "{path}: no vmt for {}", mesh.material);
                let mat = g.material(&mesh.material).unwrap_or_else(|| panic!("{path}: vmt {} unparsed", mesh.material));
                let base = mat.base_texture.clone().unwrap_or_else(|| panic!("{path}: {} has no basetexture", mesh.material));
                g.texture(&base).unwrap_or_else(|e| panic!("{path}: {base}: {e}"));
                mats.push(mesh.material.clone());
            }
            let frac = agree as f32 / total.max(1) as f32;
            assert!(frac > 0.9, "{path}: winding agrees with normals on only {:.1} %", frac * 100.0);
            println!(
                "{:<45} {:>6} {:>6} {:>5} {:>7.1} {:<14} {:>5.1}x{:>5.1}x{:>5.1}  {} (winding {:.0} %)",
                path.trim_start_matches("models/"),
                tris,
                verts,
                m.bones,
                m.mass,
                m.surface_prop,
                hi[0] - lo[0],
                hi[1] - lo[1],
                hi[2] - lo[2],
                mats.join(","),
                frac * 100.0
            );
        }
        let drum = g.model("models/props_c17/oildrum001.mdl").unwrap();
        assert!(drum.surface_prop.contains("barrel"), "surfaceprop {}", drum.surface_prop);
        assert!(drum.mass > 1.0);
    }

    #[test]
    fn textures_decode() {
        let Some(g) = install() else { return };
        let drum = g.model("models/props_c17/oildrum001.mdl").unwrap();
        let mat = g.material(&drum.meshes[0].material).unwrap();
        println!("oil drum material: {mat:?}");
        let base = mat.base_texture.unwrap();
        let t = g.texture(&base).unwrap();
        println!("{base}: {}x{} {:?} {} mips alpha {}", t.width, t.height, t.format, t.mips.len(), t.has_alpha);
        let rgba = t.rgba(0);
        assert_eq!(rgba.len(), (t.width * t.height * 4) as usize);
        assert!(t.mips.len() > 1);
        for m in 1..t.mips.len() {
            let (w, h) = ((t.width >> m).max(1), (t.height >> m).max(1));
            assert_eq!(t.rgba(m).len(), (w * h * 4) as usize, "mip {m}");
        }
        assert!(rgba.iter().any(|&v| v > 40), "texture is black");
        // a texture with an alpha channel and an uncompressed one, whichever formats they are
        assert!(g.texture("no/such/texture").is_err());
    }

    fn peak(p: &Pcm) -> i32 {
        p.samples.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0)
    }

    fn wav_tag(g: &GmodInstall, path: &str) -> u16 {
        g.read(&format!("sound/{path}")).and_then(|b| u16_at(&b, 20).ok()).unwrap_or(0)
    }

    #[test]
    fn sounds_decode() {
        let Some(g) = install() else { return };
        for path in SOUNDS {
            let p = g.sound(path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let s = p.seconds();
            println!(
                "{path:<46} {} Hz {} ch tag {} {s:.2} s peak {} loop {:?}",
                p.rate,
                p.channels,
                wav_tag(g, path),
                peak(&p),
                p.loop_start
            );
            assert!(s > 0.02 && s < 20.0, "{path}: {s} s");
            assert!(peak(&p) > 1000, "{path}: peak {}", peak(&p));
            assert!(p.to_wav().len() == 44 + p.samples.len() * 2);
        }
    }

    #[test]
    fn sounds_scan() {
        let Some(g) = install() else { return };
        let mut all = Vec::new();
        for prefix in ["sound/weapons/", "sound/physics/", "sound/player/footsteps/"] {
            all.extend(g.list(prefix, ".wav"));
        }
        let mut failed = Vec::new();
        let mut by_tag: HashMap<u16, usize> = HashMap::new();
        for f in &all {
            *by_tag.entry(g.read(f).and_then(|b| u16_at(&b, 20).ok()).unwrap_or(0)).or_default() += 1;
            if let Err(e) = g.sound(f) {
                failed.push(format!("{f}: {e}"));
            }
        }
        println!("{} wav files, {} decode, {} fail; by format tag {:?}", all.len(), all.len() - failed.len(), failed.len(), by_tag);
        for f in failed.iter().take(60) {
            println!("  FAIL {f}");
        }
        assert!(all.len() > 500, "only {} files", all.len());
    }

    #[test]
    fn collision_pieces() {
        let Some(g) = install() else { return };
        let bounds = |path: &str| {
            let m = g.model(path).unwrap();
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for mesh in &m.meshes {
                for p in &mesh.positions {
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
            }
            (lo, hi)
        };
        let mut counts = Vec::new();
        for path in MODELS {
            match g.collision(path) {
                Some(pieces) => counts.push(format!("{path}: {} pieces, {} points", pieces.len(), pieces.iter().map(Vec::len).sum::<usize>())),
                None => counts.push(format!("{path}: none")),
            }
        }
        println!("{}", counts.join("\n"));
        for (path, min_pieces) in [("models/props_c17/oildrum001.mdl", 1), ("models/props_c17/furniturecouch001a.mdl", 2)] {
            let pieces = g.collision(path).unwrap_or_else(|| panic!("{path}: no collision"));
            assert!(pieces.len() >= min_pieces, "{path}: {} pieces", pieces.len());
            let (lo, hi) = bounds(path);
            for piece in &pieces {
                assert!(piece.len() >= 4, "{path}: a piece of {} points", piece.len());
                for p in piece {
                    for k in 0..3 {
                        let slack = 0.1 * (hi[k] - lo[k]) + 2.0;
                        assert!(p[k] >= lo[k] - slack && p[k] <= hi[k] + slack, "{path}: point {p:?} outside {lo:?}..{hi:?}");
                    }
                }
            }
        }
        assert!(g.collision("models/no/such.mdl").is_none());
    }

    #[test]
    fn spawnlists_read() {
        let Some(g) = install() else { return };
        let lists = g.spawnlists();
        println!("{} spawn lists, {} models", lists.len(), lists.iter().map(|l| l.models.len()).sum::<usize>());
        assert!(lists.len() >= 20, "{} lists", lists.len());
        let c = lists.iter().find(|l| l.name == "Construction Props").expect("Construction Props");
        assert!(c.models.iter().any(|m| m == "models/props_c17/oildrum001.mdl"));
        assert_eq!(c.id, 1);
    }

    /// Writes samples to look at and listen to: `GMOD_DUMP_DIR=... cargo test -p se_core dump_gmod_samples -- --ignored`.
    #[test]
    #[ignore]
    fn dump_gmod_samples() {
        let Some(dir) = std::env::var_os("GMOD_DUMP_DIR") else {
            eprintln!("set GMOD_DUMP_DIR to dump samples");
            return;
        };
        let Some(g) = install() else { return };
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for path in MODELS {
            let m = g.model(path).unwrap();
            let stem = path.rsplit('/').next().unwrap().trim_end_matches(".mdl");
            let mut obj = format!("# {path}, Source axes and units; vt has v flipped for OBJ\n");
            let mut base = 1;
            for (i, mesh) in m.meshes.iter().enumerate() {
                obj.push_str(&format!("o {stem}_{i}\nusemtl {}\n", mesh.material));
                for p in &mesh.positions {
                    obj.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
                }
                for t in &mesh.uvs {
                    obj.push_str(&format!("vt {} {}\n", t[0], 1.0 - t[1]));
                }
                for n in &mesh.normals {
                    obj.push_str(&format!("vn {} {} {}\n", n[0], n[1], n[2]));
                }
                for t in mesh.indices.chunks_exact(3) {
                    let f = |k: u32| format!("{0}/{0}/{0}", base + k);
                    obj.push_str(&format!("f {} {} {}\n", f(t[0]), f(t[1]), f(t[2])));
                }
                base += mesh.positions.len() as u32;
                if let Some(b) = g.material(&mesh.material).and_then(|m| m.base_texture) {
                    let t = g.texture(&b).unwrap();
                    let name = b.replace('/', "_");
                    image::save_buffer(dir.join(format!("{name}.png")), &t.rgba(0), t.width, t.height, image::ExtendedColorType::Rgba8)
                        .unwrap();
                }
            }
            std::fs::write(dir.join(format!("{stem}.obj")), obj).unwrap();
        }
        for path in SOUNDS {
            let p = g.sound(path).unwrap();
            let name = path.rsplit('/').next().unwrap();
            std::fs::write(dir.join(name), p.to_wav()).unwrap();
        }
        println!("dumped to {}", dir.display());
    }

    /// Loads every model of the stock spawn lists (and their collision): `-- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scan_spawnlist_models() {
        let Some(g) = install() else { return };
        let mut seen = BTreeSet::new();
        for l in g.spawnlists() {
            seen.extend(l.models);
        }
        let (mut ok, mut phy_ok, mut phy_none, mut phy_off, mut hull_off) = (0, 0, 0, 0, 0);
        let mut fails: HashMap<String, Vec<String>> = HashMap::new();
        for path in &seen {
            match g.model(path) {
                Ok(m) => {
                    ok += 1;
                    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                    for q in m.meshes.iter().flat_map(|x| &x.positions) {
                        for k in 0..3 {
                            lo[k] = lo[k].min(q[k]);
                            hi[k] = hi[k].max(q[k]);
                        }
                    }
                    match g.collision(path) {
                        None => phy_none += 1,
                        Some(pieces) => {
                            phy_ok += 1;
                            let off = pieces.iter().flatten().any(|p| (0..3).any(|k| {
                                let s = 0.3 * (hi[k] - lo[k]) + 4.0;
                                p[k] < lo[k] - s || p[k] > hi[k] + s
                            }));
                            phy_off += off as usize;
                            hull_off += pieces.iter().flatten().any(|p| (0..3).any(|k| {
                                let s = 0.1 * (m.hull_max[k] - m.hull_min[k]) + 2.0;
                                p[k] < m.hull_min[k] - s || p[k] > m.hull_max[k] + s
                            })) as usize;
                        }
                    }
                }
                Err(e) => {
                    let why = match &e {
                        GmodError::NotFound(s) => format!("not found: {}", s.rsplit('.').next().unwrap_or("")),
                        GmodError::Bad(s) => s.splitn(3, ':').last().unwrap_or("").chars().take(70).collect(),
                    };
                    fails.entry(why).or_default().push(path.clone())
                }
            }
        }
        println!("{} models: {ok} load; collision {phy_ok} read, {phy_none} none, {phy_off} outside the mesh bounds (30 % + 4), {hull_off} outside the hull (10 % + 2)", seen.len());
        for (why, list) in &fails {
            println!("  {} x {why} e.g. {}", list.len(), list[0]);
        }
    }
}
