//! Garry's Mod's files, read only, so a game can load its props, textures and sounds at run time: from the install, or
//! from a pack of the files a game uses ([`GmodInstall::open_pack_file`], written by [`write_pack`]). Source engine formats:
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
// A pack of the files a game uses
// ---------------------------------------------------------------------------------------------

const PACK_MAGIC: &[u8; 8] = b"SEGMPAK1";

/// One file in a pack: where its bytes are (after the table), and whether they are deflated.
struct PackEntry {
    /// The path as written (spawn lists keep their names' case).
    name: String,
    /// Which of the pack's files holds it.
    part: usize,
    offset: usize,
    packed: usize,
    raw: usize,
    deflated: bool,
}

/// Where a pack's bytes are.
enum PackData {
    Memory(Vec<u8>),
    File(File),
}

/// One pack file ([`write_pack`]): `SEGMPAK1`, the count (u32), per file its path (u16 length, UTF-8), offset from the
/// data's start (u64), packed and raw lengths (u32 each) and method (u8: 0 stored, 1 raw deflate), then the data.
/// Little-endian.
struct PackPart {
    data: PackData,
    data_start: u64,
}

/// One or more pack files read as one (GitHub takes no file over 100 MB, so the port's is in pieces).
struct Pack {
    parts: Vec<PackPart>,
    /// By lowercase path; the first part holding a path wins.
    entries: HashMap<String, PackEntry>,
}

impl Pack {
    /// Reads the table from `r` (the pack from its start, `len` bytes in all).
    fn open(mut r: impl Read, len: u64, file: Option<File>) -> Result<Pack, GmodError> {
        let cut = |_| GmodError::Bad("pack table cut short".into());
        let mut head = [0u8; 12];
        r.read_exact(&mut head).map_err(|_| GmodError::Bad("not a pack of Garry's Mod's files".into()))?;
        if &head[..8] != PACK_MAGIC {
            return bad("not a pack of Garry's Mod's files");
        }
        let count = u32_at(&head, 8)? as usize;
        let mut pos = 12u64;
        let mut entries = HashMap::with_capacity(count);
        let mut fixed = [0u8; 17];
        for _ in 0..count {
            let mut l = [0u8; 2];
            r.read_exact(&mut l).map_err(cut)?;
            let mut name = vec![0u8; u16::from_le_bytes(l) as usize];
            r.read_exact(&mut name).map_err(cut)?;
            r.read_exact(&mut fixed).map_err(cut)?;
            pos += 2 + name.len() as u64 + 17;
            let name = String::from_utf8(name).map_err(|_| GmodError::Bad("pack path not UTF-8".into()))?;
            let offset = u32_at(&fixed, 0)? as usize | ((u32_at(&fixed, 4)? as usize) << 32);
            let (packed, raw) = (u32_at(&fixed, 8)? as usize, u32_at(&fixed, 12)? as usize);
            entries.insert(norm_path(&name), PackEntry { name, part: 0, offset, packed, raw, deflated: fixed[16] == 1 });
        }
        if entries.values().any(|e| pos + (e.offset + e.packed) as u64 > len) {
            return bad("pack data cut short");
        }
        let data = match file {
            Some(f) => PackData::File(f),
            None => PackData::Memory(Vec::new()),
        };
        Ok(Pack { parts: vec![PackPart { data, data_start: pos }], entries })
    }

    /// Adds another pack's files after this one's (a path both hold stays this one's).
    fn merge(&mut self, other: Pack) {
        let base = self.parts.len();
        self.parts.extend(other.parts);
        for (k, mut e) in other.entries {
            e.part += base;
            self.entries.entry(k).or_insert(e);
        }
    }

    fn read(&self, key: &str) -> Option<Vec<u8>> {
        let e = self.entries.get(key)?;
        let part = self.parts.get(e.part)?;
        let at = part.data_start + e.offset as u64;
        let src: std::borrow::Cow<[u8]> = match &part.data {
            PackData::Memory(b) => b.get(at as usize..at as usize + e.packed)?.into(),
            PackData::File(f) => {
                let mut buf = vec![0u8; e.packed];
                read_at(f, &mut buf, at).ok()?;
                buf.into()
            }
        };
        if !e.deflated {
            return Some(src.into_owned());
        }
        let out = miniz_oxide::inflate::decompress_to_vec_with_limit(&src, e.raw).ok()?;
        (out.len() == e.raw).then_some(out)
    }

    /// The files whose lowercase path starts with `prefix`: (lowercase path, path as written), sorted.
    fn under(&self, prefix: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.entries.iter().filter(|(k, _)| k.starts_with(prefix)).map(|(k, e)| (k.clone(), e.name.clone())).collect();
        out.sort();
        out
    }
}

/// Bytes of a file at `offset`, without moving a shared position (threads read the same file at once).
fn read_at(f: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = f.seek_read(buf, offset)?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            buf = &mut buf[n..];
            offset += n as u64;
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        f.read_exact_at(buf, offset)
    }
}

/// A pack of `files` (path inside the game, bytes) for [`GmodInstall::open_pack_file`]: each deflated, or stored when that
/// doesn't make it smaller.
pub fn write_pack(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let packed = squeeze(files);
    assemble(files.iter().zip(packed.iter()))
}

/// `files` as packs of at most `limit` bytes each, in their order (a file bigger than `limit` gets a pack of its own): what
/// [`GmodInstall::open_pack_files`] reads as one.
pub fn write_packs(files: &[(String, Vec<u8>)], limit: usize) -> Vec<Vec<u8>> {
    let packed = squeeze(files);
    let mut out = Vec::new();
    let mut part: Vec<(&(String, Vec<u8>), &(Vec<u8>, bool))> = Vec::new();
    let mut size = 12usize;
    for item in files.iter().zip(packed.iter()) {
        let cost = 2 + item.0.0.len() + 17 + item.1.0.len();
        if !part.is_empty() && size + cost > limit {
            out.push(assemble(part.drain(..)));
            size = 12;
        }
        size += cost;
        part.push(item);
    }
    if !part.is_empty() || out.is_empty() {
        out.push(assemble(part.into_iter()));
    }
    out
}

/// Each file deflated, or kept as it is when that isn't smaller (true: deflated).
fn squeeze(files: &[(String, Vec<u8>)]) -> Vec<(Vec<u8>, bool)> {
    files
        .iter()
        .map(|(_, b)| {
            let d = miniz_oxide::deflate::compress_to_vec(b, 6);
            if d.len() < b.len() { (d, true) } else { (b.clone(), false) }
        })
        .collect()
}

/// One pack of files already squeezed.
fn assemble<'a>(items: impl Iterator<Item = (&'a (String, Vec<u8>), &'a (Vec<u8>, bool))>) -> Vec<u8> {
    let items: Vec<_> = items.collect();
    let mut out = Vec::new();
    out.extend_from_slice(PACK_MAGIC);
    out.extend_from_slice(&(items.len() as u32).to_le_bytes());
    let mut offset = 0u64;
    for ((name, raw), (data, deflated)) in &items {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.push(*deflated as u8);
        offset += data.len() as u64;
    }
    for (_, (data, _)) in &items {
        out.extend_from_slice(data);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The install
// ---------------------------------------------------------------------------------------------

/// Garry's Mod's files: loose ones under `garrysmod/` and the archives.
pub struct GmodInstall {
    root: PathBuf,
    vpks: Vec<Vpk>,
    /// A pack of the files the port uses ([`GmodInstall::open_pack_file`]): then nothing else is read.
    pack: Option<Pack>,
    /// Every path found, while recording ([`GmodInstall::recording`]): what a copy must hold.
    seen: Option<std::sync::Mutex<BTreeSet<String>>>,
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
        Ok(GmodInstall { root: root.to_path_buf(), vpks, pack: None, seen: None })
    }

    /// A pack of the files a game uses ([`write_pack`]) in memory: every file at its path inside the game, the spawn lists
    /// under `settings/spawnlist_default/`, the menu's pictures under `backgrounds/`.
    pub fn open_pack(bytes: Vec<u8>) -> Result<Self, GmodError> {
        let len = bytes.len() as u64;
        let mut pack = Pack::open(std::io::Cursor::new(&bytes[..]), len, None)?;
        pack.parts[0].data = PackData::Memory(bytes);
        Ok(GmodInstall { root: PathBuf::new(), vpks: Vec::new(), pack: Some(pack), seen: None })
    }

    /// A pack file on disk: its table is read now, each file's bytes when asked for.
    pub fn open_pack_file(path: &Path) -> Result<Self, GmodError> {
        Self::open_pack_files(&[path.to_path_buf()])
    }

    /// Several pack files on disk read as one (the first holding a path wins); `root` is the first one's folder.
    pub fn open_pack_files(paths: &[PathBuf]) -> Result<Self, GmodError> {
        let mut all: Option<Pack> = None;
        for path in paths {
            let f = File::open(path).map_err(|e| GmodError::NotFound(format!("{}: {e}", path.display())))?;
            let len = f.metadata().map_err(|e| GmodError::NotFound(format!("{}: {e}", path.display())))?.len();
            let table = std::io::BufReader::with_capacity(1 << 20, f.try_clone().map_err(|e| GmodError::Bad(e.to_string()))?);
            let pack = Pack::open(table, len, Some(f)).map_err(|e| match e {
                GmodError::Bad(s) => GmodError::Bad(format!("{}: {s}", path.display())),
                other => other,
            })?;
            match &mut all {
                Some(a) => a.merge(pack),
                None => all = Some(pack),
            }
        }
        let pack = all.ok_or_else(|| GmodError::NotFound("no pack files".into()))?;
        let root = paths[0].parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(GmodInstall { root, vpks: Vec::new(), pack: Some(pack), seen: None })
    }

    /// Note every path read or found from now on ([`GmodInstall::seen`]).
    pub fn recording(mut self) -> Self {
        self.seen = Some(Default::default());
        self
    }

    /// The paths noted while recording, sorted.
    pub fn seen(&self) -> Vec<String> {
        self.seen.as_ref().map_or_else(Vec::new, |s| s.lock().map(|s| s.iter().cloned().collect()).unwrap_or_default())
    }

    fn note(&self, key: &str) {
        if let Some(s) = &self.seen {
            if let Ok(mut s) = s.lock() {
                s.insert(key.to_string());
            }
        }
    }

    /// Whether this is a pack rather than the install.
    pub fn is_pack(&self) -> bool {
        self.pack.is_some()
    }

    /// The files of one of Garry's Mod's own loose folders ("backgrounds"), as paths to [`GmodInstall::read`], sorted. In
    /// the install they are under `garrysmod/`.
    pub fn folder_files(&self, rel: &str) -> Vec<String> {
        let rel = norm_path(rel);
        if let Some(p) = &self.pack {
            return p.under(&format!("{rel}/")).into_iter().map(|(key, _)| key).collect();
        }
        let Ok(rd) = std::fs::read_dir(self.root.join("garrysmod").join(&rel)) else { return Vec::new() };
        let mut out: Vec<String> = rd.flatten().filter(|e| e.path().is_file()).map(|e| format!("{rel}/{}", e.file_name().to_string_lossy())).collect();
        out.sort_by_key(|k| k.to_ascii_lowercase());
        out
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
        let found = match &self.pack {
            Some(p) => p.read(&key),
            None => self.loose(&key).and_then(|p| std::fs::read(p).ok()).or_else(|| self.vpks.iter().find_map(|v| v.read(&key))),
        };
        if found.is_some() {
            self.note(&key);
        }
        found
    }

    pub fn exists(&self, path: &str) -> bool {
        let key = norm_path(path);
        let found = !key.is_empty()
            && match &self.pack {
                Some(p) => p.entries.contains_key(&key),
                None => self.loose(&key).is_some() || self.vpks.iter().any(|v| v.entries.contains_key(&key)),
            };
        if found {
            self.note(&key);
        }
        found
    }

    /// The path of a loose file, when there is one. Paths that climb out of the folder are refused.
    fn loose(&self, key: &str) -> Option<PathBuf> {
        if key.split('/').any(|c| c == ".." || c.contains(':')) {
            return None;
        }
        // (Garry's Mod's own folder first, then Half-Life 2's it mounts: its sound and weapon scripts are loose files there.)
        ["garrysmod", "sourceengine"].iter().map(|d| self.root.join(d).join(key)).find(|p| p.is_file())
    }

    /// Every archived file whose path starts with `prefix` and ends with `ext` (lowercase), sorted.
    pub fn list(&self, prefix: &str, ext: &str) -> Vec<String> {
        let (prefix, ext) = (norm_path(prefix), ext.to_ascii_lowercase());
        let mut out: BTreeSet<&String> = BTreeSet::new();
        for v in &self.vpks {
            out.extend(v.entries.keys().filter(|k| k.starts_with(&prefix) && k.ends_with(&ext)));
        }
        if let Some(p) = &self.pack {
            out.extend(p.entries.keys().filter(|k| k.starts_with(&prefix) && k.ends_with(&ext)));
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
        if let Some(p) = &self.pack {
            return p
                .under("settings/spawnlist_default/")
                .into_iter()
                .filter(|(key, _)| key.ends_with(".txt"))
                .filter_map(|(key, name)| {
                    let text = p.read(&key)?;
                    let stem = name.rsplit('/').next()?.rsplit_once('.')?.0.to_string();
                    parse_spawnlist(&String::from_utf8_lossy(&text), &stem)
                })
                .collect();
        }
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
                self.note(&format!("settings/spawnlist_default/{}", p.file_name()?.to_string_lossy()));
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
    /// Per vertex (parallel to `positions`): the model's bone indices the vertex follows.
    pub bones: Vec<[u8; 3]>,
    /// Per vertex: the weights of those bones (0 past the vertex's own bone count; they sum to 1).
    pub weights: Vec<[f32; 3]>,
}

/// One vertex of the VVD: position, normal, uv, skin weights.
#[derive(Clone, Copy)]
struct VvdVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    bones: [u8; 3],
    weights: [f32; 3],
}

/// A VVD vertex's `mstudioboneweight_t` (weight[3], bone[3], numbones): the weights past `numbones`
/// zeroed; a vertex with no weight at all follows bone 0 alone.
fn read_bone_weight(b: &[u8], o: usize) -> Result<([u8; 3], [f32; 3]), GmodError> {
    let mut w = [f32_at(b, o)?, f32_at(b, o + 4)?, f32_at(b, o + 8)?];
    let bones = bytes_at::<3>(b, o + 12)?;
    let n = (u8_at(b, o + 15)? as usize).min(3);
    for (i, x) in w.iter_mut().enumerate() {
        if i >= n || !x.is_finite() || *x < 0.0 {
            *x = 0.0;
        }
    }
    let sum: f32 = w.iter().sum();
    if sum <= 0.0 {
        return Ok(([bones[0], 0, 0], [1.0, 0.0, 0.0]));
    }
    if (sum - 1.0).abs() > 1e-4 {
        w = [w[0] / sum, w[1] / sum, w[2] / sum];
    }
    Ok((bones, w))
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
        let (bones, weights) = read_bone_weight(b, vertex_start + i * 48)?;
        Ok(VvdVertex {
            bones,
            weights,
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
                bones: Vec::new(),
                weights: Vec::new(),
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
                mesh.bones.push(v.bones);
                mesh.weights.push(v.weights);
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
// Skeleton and animation
// ---------------------------------------------------------------------------------------------

/// One bone of a model's skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct GmodBone {
    pub name: String,
    /// -1 for a root.
    pub parent: i32,
    /// Bind pose, relative to the parent (Source units).
    pub pos: [f32; 3],
    /// x, y, z, w; bind pose, relative to the parent.
    pub quat: [f32; 4],
    /// The file's matrix3x4 (rows): model space to bone space in the bind pose.
    pub pose_to_bone: [[f32; 4]; 3],
}

/// An attachment point (a muzzle, an eject port...).
#[derive(Debug, Clone, PartialEq)]
pub struct GmodAttachment {
    pub name: String,
    /// The bone it follows.
    pub bone: usize,
    /// The point in that bone's space (matrix3x4 rows).
    pub local: [[f32; 4]; 3],
}

/// An animation event of a sequence (the muzzle flash, the shell, a sound).
#[derive(Debug, Clone, PartialEq)]
pub struct GmodEvent {
    /// 0 at the sequence's start, 1 at its end.
    pub cycle: f32,
    pub event: i32,
    pub kind: i32,
    pub options: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmodSequence {
    /// The label, lowercase.
    pub name: String,
    /// The activity name as the file has it (empty for none).
    pub activity: String,
    pub fps: f32,
    /// STUDIO_LOOPING of the sequence's flags.
    pub looping: bool,
    /// STUDIO_DELTA of the animation's flags.
    pub delta: bool,
    /// STUDIO_SNAP of the sequence's flags: it starts at once, without blending from the one before.
    pub snap: bool,
    pub fade_in: f32,
    pub fade_out: f32,
    pub events: Vec<GmodEvent>,
    /// `frames[f][bone]` = (position, quaternion xyzw), relative to the parent bone. Whole frames only.
    pub frames: Vec<Vec<([f32; 3], [f32; 4])>>,
    /// STUDIO_AUTOPLAY of the sequence's flags: it is laid over whatever else plays (the gravity gun's prongs).
    pub autoplay: bool,
    /// For a sequence that blends between animations by a pose parameter: the first frame of the last of them (the
    /// other end from `frames[0]`: the prongs open). Empty for a sequence of one animation.
    pub blend_to: Vec<([f32; 3], [f32; 4])>,
    /// How much of the sequence each bone takes (its weight list): 1 for all in most.
    pub weights: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmodSkeleton {
    pub bones: Vec<GmodBone>,
    pub attachments: Vec<GmodAttachment>,
    pub sequences: Vec<GmodSequence>,
}

impl GmodInstall {
    /// A model's bones, attachments and sequences (the first blend of each), from its .mdl alone.
    /// A sequence whose animation lives in an external `.ani` block has the bind pose for every frame.
    pub fn skeleton(&self, mdl_path: &str) -> Result<GmodSkeleton, GmodError> {
        let norm = norm_path(mdl_path);
        let base = norm.strip_suffix(".mdl").unwrap_or(&norm).to_string();
        let mdl = self.read(&format!("{base}.mdl")).ok_or_else(|| GmodError::NotFound(format!("{base}.mdl")))?;
        read_skeleton(&mdl).map_err(|e| match e {
            GmodError::Bad(s) => GmodError::Bad(format!("model {mdl_path}: {s}")),
            other => other,
        })
    }
}

/// The most frames one animation may have (corrupt data otherwise asks for gigabytes).
const ANIM_FRAMES_MAX: usize = 20_000;

const ANIM_RAWPOS: u8 = 0x01;
const ANIM_RAWROT: u8 = 0x02;
const ANIM_ANIMPOS: u8 = 0x04;
const ANIM_ANIMROT: u8 = 0x08;
const ANIM_DELTA: u8 = 0x10;
const ANIM_RAWROT2: u8 = 0x20;

/// A bone's defaults and compression scales: what the animation data is relative to.
#[derive(Clone, Copy)]
struct BoneBase {
    pos: [f32; 3],
    quat: [f32; 4],
    rot: [f32; 3],
    pos_scale: [f32; 3],
    rot_scale: [f32; 3],
}

const QUAT_IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn normalize_quat(q: [f32; 4]) -> [f32; 4] {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l.is_finite() && l > 1e-6 {
        [q[0] / l, q[1] / l, q[2] / l, q[3] / l]
    } else {
        QUAT_IDENTITY
    }
}

/// Source's `AngleQuaternion(RadianEuler)`: x is roll, y pitch, z yaw.
fn euler_to_quat(a: [f32; 3]) -> [f32; 4] {
    let (sr, cr) = (a[0] * 0.5).sin_cos();
    let (sp, cp) = (a[1] * 0.5).sin_cos();
    let (sy, cy) = (a[2] * 0.5).sin_cos();
    let (sr_cp, cr_sp) = (sr * cp, cr * sp);
    let (cr_cp, sr_sp) = (cr * cp, sr * sp);
    [sr_cp * cy - cr_sp * sy, cr_sp * cy + sr_cp * sy, cr_cp * sy - sr_sp * cy, cr_cp * cy + sr_sp * sy]
}

/// An IEEE half float (Source's `float16`); infinities and NaNs read as 0.
fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let man = (h & 0x3ff) as f32;
    match exp {
        0 => sign * man * 2f32.powi(-24),
        31 => 0.0,
        e => sign * (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    }
}

/// `Quaternion48`: x and y 16 bits, z 15 bits, then the sign of w.
fn quat48_at(b: &[u8], o: usize) -> Result<[f32; 4], GmodError> {
    let (x, y, z) = (u16_at(b, o)? as i32, u16_at(b, o + 2)? as i32, u16_at(b, o + 4)?);
    let q = [
        (x - 32768) as f32 * (1.0 / 32768.0),
        (y - 32768) as f32 * (1.0 / 32768.0),
        ((z & 0x7fff) as i32 - 16384) as f32 * (1.0 / 16384.0),
    ];
    let w = (1.0 - q[0] * q[0] - q[1] * q[1] - q[2] * q[2]).max(0.0).sqrt();
    Ok(normalize_quat([q[0], q[1], q[2], if z & 0x8000 != 0 { -w } else { w }]))
}

/// `Quaternion64`: x, y, z 21 bits each, then the sign of w.
fn quat64_at(b: &[u8], o: usize) -> Result<[f32; 4], GmodError> {
    let v = u64::from_le_bytes(bytes_at(b, o)?);
    let comp = |s: u32| (((v >> s) & 0x1f_ffff) as i64 - 1_048_576) as f32 * (1.0 / 1_048_576.5);
    let q = [comp(0), comp(21), comp(42)];
    let w = (1.0 - q[0] * q[0] - q[1] * q[1] - q[2] * q[2]).max(0.0).sqrt();
    Ok(normalize_quat([q[0], q[1], q[2], if v >> 63 != 0 { -w } else { w }]))
}

/// `ExtractAnimValue`: the run-length stream at `at` (None: no stream) at `frame`, times `scale`.
fn extract_anim_value(b: &[u8], at: Option<usize>, frame: usize, scale: f32) -> Result<f32, GmodError> {
    let Some(mut p) = at else { return Ok(0.0) };
    let mut k = frame;
    while (u8_at(b, p + 1)? as usize) <= k {
        k -= u8_at(b, p + 1)? as usize;
        p += (u8_at(b, p)? as usize + 1) * 2;
        if u8_at(b, p + 1)? == 0 {
            return Ok(0.0); // running off the end of the stream
        }
    }
    let valid = u8_at(b, p)? as usize;
    let i = if valid > k { k + 1 } else { valid };
    Ok(i16_at(b, p + i * 2)? as f32 * scale)
}

/// One bone's value in one frame from its `mstudioanim_t` at `e` (`CalcBoneQuaternion`, `CalcBonePosition`
/// with no blend between frames).
fn decode_anim_bone(b: &[u8], e: usize, frame: usize, base: &BoneBase) -> Result<([f32; 3], [f32; 4]), GmodError> {
    let flags = u8_at(b, e + 1)?;
    let data = e + 4;
    let delta = flags & ANIM_DELTA != 0;
    // The three offsets of a value pointer are shorts from the pointer itself; 0 or less is no stream.
    let streams = |vp: usize, scale: [f32; 3]| -> Result<[f32; 3], GmodError> {
        let mut v = [0.0; 3];
        for j in 0..3 {
            let off = i16_at(b, vp + j * 2)?;
            let at = (off > 0).then(|| vp + off as usize);
            v[j] = extract_anim_value(b, at, frame, scale[j])?;
        }
        Ok(v)
    };

    let quat = if flags & ANIM_RAWROT != 0 {
        quat48_at(b, data)?
    } else if flags & ANIM_RAWROT2 != 0 {
        quat64_at(b, data)?
    } else if flags & ANIM_ANIMROT == 0 {
        if delta {
            QUAT_IDENTITY
        } else {
            base.quat
        }
    } else {
        let mut ang = streams(data, base.rot_scale)?;
        if !delta {
            for (a, r) in ang.iter_mut().zip(base.rot) {
                *a += r;
            }
        }
        euler_to_quat(ang)
    };

    let pos = if flags & ANIM_RAWPOS != 0 {
        let o = data + if flags & ANIM_RAWROT != 0 { 6 } else { 0 } + if flags & ANIM_RAWROT2 != 0 { 8 } else { 0 };
        [half_to_f32(u16_at(b, o)?), half_to_f32(u16_at(b, o + 2)?), half_to_f32(u16_at(b, o + 4)?)]
    } else if flags & ANIM_ANIMPOS == 0 {
        if delta {
            [0.0; 3]
        } else {
            base.pos
        }
    } else {
        let mut v = streams(data + if flags & ANIM_ANIMROT != 0 { 6 } else { 0 }, base.pos_scale)?;
        if !delta {
            for (a, r) in v.iter_mut().zip(base.pos) {
                *a += r;
            }
        }
        v
    };
    Ok((pos, normalize_quat(quat)))
}

/// A bone's name-less defaults for a frame the animation doesn't mention (a delta animation adds nothing).
fn rest_frame(bases: &[BoneBase], delta: bool) -> Vec<([f32; 3], [f32; 4])> {
    bases.iter().map(|bb| if delta { ([0.0; 3], QUAT_IDENTITY) } else { (bb.pos, bb.quat) }).collect()
}

/// One frame of the animation whose `mstudioanim_t` list starts at `start` (`frame` counted from it).
fn decode_frame(
    b: &[u8],
    start: usize,
    frame: usize,
    bases: &[BoneBase],
    delta: bool,
) -> Result<Vec<([f32; 3], [f32; 4])>, GmodError> {
    let mut out = rest_frame(bases, delta);
    let (mut p, mut steps) = (start, 0usize);
    loop {
        steps += 1;
        if steps > bases.len() * 2 + 16 {
            return bad("animation: the bone list does not end");
        }
        let bone = u8_at(b, p)? as usize;
        if bone < out.len() {
            out[bone] = decode_anim_bone(b, p, frame, &bases[bone])?;
        }
        let next = i16_at(b, p + 2)?;
        if next == 0 {
            break;
        }
        p = rel(p, next as i32)?;
    }
    Ok(out)
}

/// All frames of one `mstudioanimdesc_t` at `ad`. Sections are followed as `pAnim` does; an animation in an
/// external block (a .ani file) or with no data gives the bind pose.
fn decode_animation(
    mdl: &[u8],
    ad: usize,
    bases: &[BoneBase],
) -> Result<(f32, bool, Vec<Vec<([f32; 3], [f32; 4])>>), GmodError> {
    let fps = f32_at(mdl, ad + 8)?;
    let flags = i32_at(mdl, ad + 12)?;
    let num_frames = (i32_at(mdl, ad + 16)?.max(1) as usize).min(ANIM_FRAMES_MAX + 1);
    if num_frames > ANIM_FRAMES_MAX {
        return bad("animation: too many frames");
    }
    let delta = flags & 0x4 != 0;
    let (anim_block, anim_index) = (i32_at(mdl, ad + 52)?, i32_at(mdl, ad + 56)?);
    let (section_index, section_frames) = (i32_at(mdl, ad + 80)?, i32_at(mdl, ad + 84)?);
    let all_zeros = flags & 0x20 != 0;

    let mut frames = Vec::with_capacity(num_frames);
    for f in 0..num_frames {
        // Where this frame's list starts and which frame of it we want (None: bind pose).
        let located: Option<(usize, usize)> = if all_zeros {
            None
        } else if section_frames > 0 {
            let sf = section_frames as usize;
            let (section, local) = if num_frames > sf && f == num_frames - 1 {
                (num_frames / sf + 1, 0)
            } else {
                (f / sf, f % sf)
            };
            let so = rel(ad, section_index)? + section * 8;
            let (block, index) = (i32_at(mdl, so)?, i32_at(mdl, so + 4)?);
            (block == 0 && index > 0).then(|| (ad + index as usize, local))
        } else {
            (anim_block == 0 && anim_index > 0).then(|| (ad + anim_index as usize, f))
        };
        frames.push(match located {
            Some((start, local)) => decode_frame(mdl, start, local, bases, delta)?,
            None => rest_frame(bases, delta),
        });
    }
    Ok((fps, delta, frames))
}

fn read_matrix3x4(b: &[u8], o: usize) -> Result<[[f32; 4]; 3], GmodError> {
    let mut m = [[0.0; 4]; 3];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = f32_at(b, o + (r * 4 + c) * 4)?;
        }
    }
    Ok(m)
}

fn read_skeleton(mdl: &[u8]) -> Result<GmodSkeleton, GmodError> {
    if mdl.get(0..4) != Some(b"IDST") {
        return bad("mdl: bad signature");
    }
    let version = i32_at(mdl, 4)?;
    if !(44..=49).contains(&version) {
        return bad(format!("mdl version {version}"));
    }

    // studiohdr_t: numbones 156, boneindex 160, numlocalanim 180, localanimindex 184, numlocalseq 188,
    // localseqindex 192, numlocalattachments 240, localattachmentindex 244.
    let num_bones = count_at(mdl, 156)?;
    let bone_index = u32_at(mdl, 160)? as usize;
    let mut bones = Vec::with_capacity(num_bones);
    let mut bases = Vec::with_capacity(num_bones);
    for i in 0..num_bones {
        // mstudiobone_t, 216 bytes: name 0, parent 4, pos 32, quat 44, rot 60, posscale 72, rotscale 84,
        // poseToBone 96.
        let o = bone_index + i * 216;
        let parent = i32_at(mdl, o + 4)?;
        if parent >= num_bones as i32 || parent < -1 || parent >= i as i32 {
            return bad(format!("bone {i}: parent {parent}"));
        }
        let pos = vec3_at(mdl, o + 32)?;
        let quat = [f32_at(mdl, o + 44)?, f32_at(mdl, o + 48)?, f32_at(mdl, o + 52)?, f32_at(mdl, o + 56)?];
        bones.push(GmodBone {
            name: cstr_at(mdl, rel(o, i32_at(mdl, o)?)?),
            parent,
            pos,
            quat,
            pose_to_bone: read_matrix3x4(mdl, o + 96)?,
        });
        bases.push(BoneBase {
            pos,
            quat,
            rot: vec3_at(mdl, o + 60)?,
            pos_scale: vec3_at(mdl, o + 72)?,
            rot_scale: vec3_at(mdl, o + 84)?,
        });
    }

    let num_att = count_at(mdl, 240)?;
    let att_index = u32_at(mdl, 244)? as usize;
    let mut attachments = Vec::with_capacity(num_att);
    for i in 0..num_att {
        // mstudioattachment_t, 92 bytes: name 0, flags 4, localbone 8, local 12.
        let o = att_index + i * 92;
        let bone = i32_at(mdl, o + 8)?;
        if bone < 0 || bone as usize >= num_bones {
            return bad(format!("attachment {i}: bone {bone}"));
        }
        attachments.push(GmodAttachment {
            name: cstr_at(mdl, rel(o, i32_at(mdl, o)?)?),
            bone: bone as usize,
            local: read_matrix3x4(mdl, o + 12)?,
        });
    }

    let num_anim = count_at(mdl, 180)?;
    let anim_index = u32_at(mdl, 184)? as usize;
    let num_seq = count_at(mdl, 188)?;
    let seq_index = u32_at(mdl, 192)? as usize;
    // Animations are shared between sequences: decode each once.
    let mut decoded: HashMap<usize, (f32, bool, Vec<Vec<([f32; 3], [f32; 4])>>)> = HashMap::new();
    let mut sequences = Vec::with_capacity(num_seq);
    for i in 0..num_seq {
        // mstudioseqdesc_t, 212 bytes: label 4, activity name 8, flags 12, numevents 24, eventindex 28,
        // numblends 56, animindexindex 60, groupsize 68 and 72, fadeintime 104, fadeouttime 108, weightlistindex 156.
        let o = seq_index + i * 212;
        let name = cstr_at(mdl, rel(o, i32_at(mdl, o + 4)?)?).to_ascii_lowercase();
        let activity = cstr_at(mdl, rel(o, i32_at(mdl, o + 8)?)?);
        let flags = i32_at(mdl, o + 12)?;

        let num_events = count_at(mdl, o + 24)?;
        let event_index = rel(o, i32_at(mdl, o + 28)?)?;
        let mut events = Vec::with_capacity(num_events);
        for e in 0..num_events {
            // mstudioevent_t, 80 bytes: cycle 0, event 4, type 8, options 12 (64 bytes), name 76.
            let eo = event_index + e * 80;
            let opt = bytes_at::<64>(mdl, eo + 12)?;
            let end = opt.iter().position(|&c| c == 0).unwrap_or(64);
            let name_off = i32_at(mdl, eo + 76)?;
            events.push(GmodEvent {
                cycle: f32_at(mdl, eo)?,
                event: i32_at(mdl, eo + 4)?,
                kind: i32_at(mdl, eo + 8)?,
                options: String::from_utf8_lossy(&opt[..end]).into_owned(),
                name: if name_off != 0 { cstr_at(mdl, rel(eo, name_off)?) } else { String::new() },
            });
        }

        // The first blend's animation, and the last one's first frame when there are several.
        let num_blends = i32_at(mdl, o + 56)?;
        let list = rel(o, i32_at(mdl, o + 60)?)?;
        let first = if num_blends > 0 { i16_at(mdl, list)? as i32 } else { -1 };
        let count = (i32_at(mdl, o + 68)?.max(1) as usize).saturating_mul(i32_at(mdl, o + 72)?.max(1) as usize).min(64);
        let last = if num_blends > 1 && count > 1 { i16_at(mdl, list + (count - 1) * 2)? as i32 } else { -1 };
        let mut animation = |index: i32| -> Result<Option<(f32, bool, Vec<Vec<([f32; 3], [f32; 4])>>)>, GmodError> {
            if index < 0 || index as usize >= num_anim {
                return Ok(None);
            }
            let ad = anim_index + index as usize * 100;
            if !decoded.contains_key(&ad) {
                let d = decode_animation(mdl, ad, &bases)?;
                decoded.insert(ad, d);
            }
            Ok(decoded.get(&ad).cloned())
        };
        let anim = animation(first)?;
        let blend_to = if last != first { animation(last)?.and_then(|a| a.2.into_iter().next()).unwrap_or_default() } else { Vec::new() };
        let (fps, delta, frames) = anim.unwrap_or_else(|| (30.0, false, vec![rest_frame(&bases, false)]));
        let weight_list = i32_at(mdl, o + 156)?;
        let weights = if weight_list != 0 {
            let w = rel(o, weight_list)?;
            (0..num_bones).map(|b| f32_at(mdl, w + b * 4)).collect::<Result<Vec<f32>, GmodError>>()?
        } else {
            vec![1.0; num_bones]
        };
        sequences.push(GmodSequence {
            name,
            activity,
            fps,
            looping: flags & 1 != 0,
            delta,
            snap: flags & 2 != 0,
            fade_in: f32_at(mdl, o + 104)?,
            fade_out: f32_at(mdl, o + 108)?,
            events,
            frames,
            autoplay: flags & 8 != 0,
            blend_to,
            weights,
        });
    }

    Ok(GmodSkeleton { bones, attachments, sequences })
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

    /// A pack reads back every file, in memory and from a file, case-insensitively, with its lists and spawn lists.
    #[test]
    fn a_pack_reads_back_what_was_written() {
        let big: Vec<u8> = (0..20000u32).map(|i| (i % 7) as u8).collect();
        let noise: Vec<u8> = (0..300u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        let list = b"\"TableToKeyValues\" { \"parentid\" \"0\" \"id\" \"1\" \"name\" \"Junk\" \"contents\" { \"1\" { \"type\" \"model\" \"model\" \"models/a.mdl\" } } }".to_vec();
        let files = vec![
            ("models/a.mdl".to_string(), big.clone()),
            ("sound/b.wav".to_string(), noise.clone()),
            ("settings/spawnlist_default/001-Junk.txt".to_string(), list),
            ("backgrounds/x.jpg".to_string(), vec![1, 2, 3]),
        ];
        let bytes = write_pack(&files);
        let path = std::env::temp_dir().join(format!("se_pack_test_{}.pak", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        for pack in [GmodInstall::open_pack(bytes.clone()).unwrap(), GmodInstall::open_pack_file(&path).unwrap()] {
            assert_eq!(pack.read(r"MODELS\A.mdl").as_deref(), Some(&big[..]));
            assert_eq!(pack.read("sound/b.wav").as_deref(), Some(&noise[..]));
            assert!(pack.read("sound/c.wav").is_none() && !pack.exists("sound/c.wav") && pack.exists("sound/b.wav"));
            assert_eq!(pack.list("models/", ".mdl"), ["models/a.mdl"]);
            assert_eq!(pack.folder_files("backgrounds"), ["backgrounds/x.jpg"]);
            let lists = pack.spawnlists();
            assert_eq!(lists.len(), 1);
            assert_eq!(lists[0].models, ["models/a.mdl"]);
        }
        assert!(GmodInstall::open_pack(bytes[..bytes.len() - 1].to_vec()).is_err(), "a pack cut short");
        // Several parts read as one: each part's files, and the first part wins a path both hold.
        let second = std::env::temp_dir().join(format!("se_pack_test_{}_b.pak", std::process::id()));
        std::fs::write(&second, write_pack(&[("models/a.mdl".to_string(), vec![9]), ("models/z.mdl".to_string(), vec![7, 7])])).unwrap();
        let both = GmodInstall::open_pack_files(&[path.clone(), second.clone()]).unwrap();
        assert_eq!(both.read("models/a.mdl").as_deref(), Some(&big[..]));
        assert_eq!(both.read("models/z.mdl").as_deref(), Some(&[7u8, 7][..]));
        assert_eq!(both.list("models/", ".mdl"), ["models/a.mdl", "models/z.mdl"]);
        assert_eq!(both.spawnlists().len(), 1);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&second);
        // Split by size: every part within the limit (but a file bigger than it, alone), all files kept in order.
        let parts = write_packs(&files, 200);
        assert!(parts.len() >= 3, "{} parts", parts.len());
        let mut names = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let one = GmodInstall::open_pack(part.clone()).unwrap();
            let mut here = one.list("", "");
            assert!(part.len() <= 200 || here.len() == 1, "part {i}: {} bytes, {} files", part.len(), here.len());
            names.append(&mut here);
        }
        names.sort();
        let mut want: Vec<String> = files.iter().map(|f| f.0.to_ascii_lowercase()).collect();
        want.sort();
        assert_eq!(names, want);
    }

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

    // --- skeletons, skin weights and animation -------------------------------------------------

    /// The six main view models, then two more that may be there.
    const VIEW_MODELS: [&str; 8] = [
        "models/weapons/v_pistol.mdl",
        "models/weapons/v_smg1.mdl",
        "models/weapons/v_shotgun.mdl",
        "models/weapons/v_crowbar.mdl",
        "models/weapons/v_physcannon.mdl",
        "models/weapons/v_superphyscannon.mdl",
        "models/weapons/v_357.mdl",
        "models/weapons/v_irifle.mdl",
    ];

    type Mat = [[f32; 4]; 3];

    const MAT_IDENTITY: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

    /// Source's `QuaternionMatrix(q, pos)`.
    fn quat_matrix(q: [f32; 4], p: [f32; 3]) -> Mat {
        let [x, y, z, w] = q;
        [
            [1.0 - 2.0 * y * y - 2.0 * z * z, 2.0 * x * y - 2.0 * w * z, 2.0 * x * z + 2.0 * w * y, p[0]],
            [2.0 * x * y + 2.0 * w * z, 1.0 - 2.0 * x * x - 2.0 * z * z, 2.0 * y * z - 2.0 * w * x, p[1]],
            [2.0 * x * z - 2.0 * w * y, 2.0 * y * z + 2.0 * w * x, 1.0 - 2.0 * x * x - 2.0 * y * y, p[2]],
        ]
    }

    /// a * b, both with an implicit last row 0 0 0 1.
    fn mat_mul(a: &Mat, b: &Mat) -> Mat {
        let mut o = [[0.0; 4]; 3];
        for r in 0..3 {
            for c in 0..4 {
                o[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c] + if c == 3 { a[r][3] } else { 0.0 };
            }
        }
        o
    }

    fn mat_point(m: &Mat, p: [f32; 3]) -> [f32; 3] {
        let row = |r: usize| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3];
        [row(0), row(1), row(2)]
    }

    /// Bone to model matrices of a pose (local values per bone) composed up the hierarchy.
    fn bone_to_model(bones: &[GmodBone], pose: &[([f32; 3], [f32; 4])]) -> Vec<Mat> {
        let mut out: Vec<Mat> = Vec::with_capacity(bones.len());
        for (i, b) in bones.iter().enumerate() {
            let local = quat_matrix(pose[i].1, pose[i].0);
            let m = if b.parent < 0 { local } else { mat_mul(&out[b.parent as usize], &local) };
            out.push(m);
        }
        out
    }

    fn quat_len(q: [f32; 4]) -> f32 {
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
    }

    /// The view models this install has (the first six must all be there).
    fn view_models(g: &GmodInstall) -> Vec<(&'static str, GmodSkeleton)> {
        let mut out = Vec::new();
        for (i, path) in VIEW_MODELS.iter().enumerate() {
            match g.skeleton(path) {
                Ok(s) => out.push((*path, s)),
                Err(e) => {
                    assert!(i >= 6, "{path}: {e}");
                    eprintln!("{path}: not read ({e}), skipped");
                }
            }
        }
        out
    }

    #[test]
    fn skeleton_bind_poses() {
        let Some(g) = install() else { return };
        for (path, s) in view_models(g) {
            assert!(!s.bones.is_empty(), "{path}: no bones");
            for (i, b) in s.bones.iter().enumerate() {
                let l = quat_len(b.quat);
                assert!((l - 1.0).abs() < 1e-3, "{path}: bone {} quat length {l}", b.name);
                assert!(b.parent < i as i32, "{path}: bone {} parent {}", b.name, b.parent);
            }
            let bind: Vec<_> = s.bones.iter().map(|b| (b.pos, b.quat)).collect();
            let world = bone_to_model(&s.bones, &bind);
            for (i, b) in s.bones.iter().enumerate() {
                let m = mat_mul(&world[i], &b.pose_to_bone);
                for r in 0..3 {
                    for c in 0..4 {
                        let want = MAT_IDENTITY[r][c];
                        assert!(
                            (m[r][c] - want).abs() < 1e-2,
                            "{path}: bone {} bind * poseToBone is not the identity: {m:?}",
                            b.name
                        );
                    }
                }
            }
            for a in &s.attachments {
                assert!(a.bone < s.bones.len(), "{path}: attachment {} bone", a.name);
            }
        }
    }

    #[test]
    fn skin_weights_sum_to_one() {
        let Some(g) = install() else { return };
        for path in VIEW_MODELS {
            let Ok(m) = g.model(path) else { continue };
            assert!(m.bones > 0 && !m.meshes.is_empty(), "{path}");
            for mesh in &m.meshes {
                assert_eq!(mesh.bones.len(), mesh.positions.len());
                assert_eq!(mesh.weights.len(), mesh.positions.len());
                for (b, w) in mesh.bones.iter().zip(&mesh.weights) {
                    let sum: f32 = w.iter().sum();
                    assert!((sum - 1.0).abs() < 1e-3, "{path}: weights {w:?} sum to {sum}");
                    assert!(w.iter().all(|x| (0.0..=1.0 + 1e-4).contains(x)), "{path}: weights {w:?}");
                    for k in 0..3 {
                        assert!(w[k] == 0.0 || (b[k] as usize) < m.bones, "{path}: bone {} of {}", b[k], m.bones);
                    }
                    assert!((b[0] as usize) < m.bones, "{path}: bone {} of {}", b[0], m.bones);
                }
            }
        }
        // The props' meshes carry weights too.
        for path in MODELS {
            let Ok(m) = g.model(path) else { continue };
            for mesh in &m.meshes {
                assert_eq!(mesh.weights.len(), mesh.positions.len(), "{path}");
                assert!(mesh.weights.iter().all(|w| (w.iter().sum::<f32>() - 1.0).abs() < 1e-3), "{path}");
            }
        }
    }

    #[test]
    fn view_model_sequences() {
        let Some(g) = install() else { return };
        let all = view_models(g);
        assert!(all.len() >= 6);
        for (path, s) in &all {
            assert!(!s.sequences.is_empty(), "{path}: no sequences");
            for seq in &s.sequences {
                let what = format!("{path} {}", seq.name);
                assert!(!seq.frames.is_empty(), "{what}: no frames");
                assert!(seq.fps.is_finite() && seq.fps > 0.0, "{what}: fps {}", seq.fps);
                for (f, frame) in seq.frames.iter().enumerate() {
                    assert_eq!(frame.len(), s.bones.len(), "{what}: frame {f}");
                    for (pos, q) in frame {
                        assert!((quat_len(*q) - 1.0).abs() < 1e-3, "{what}: frame {f} quat {q:?}");
                        assert!(pos.iter().all(|x| x.is_finite() && x.abs() < 500.0), "{what}: frame {f} pos {pos:?}");
                    }
                }
                // A few real animations move fast (a swing's first frame, the .357's cylinder spinning at
                // up to 145 degrees a frame, a bullet flung out): checked by hand, the frames around them
                // are smooth. Those sequences may have a handful of big turns; no other may have one.
                let fast = [
                    ("v_pistol", "holsterempty"),
                    ("v_crowbar", "hitcenter1"),
                    ("v_crowbar", "hitcenter2"),
                    ("v_crowbar", "hitcenter3"),
                    ("v_357", "reload"),
                ]
                .iter()
                .any(|(m, q)| path.contains(&format!("/{m}.mdl")) && seq.name == *q);
                let mut big = 0;
                for (f, pair) in seq.frames.windows(2).enumerate() {
                    for (bi, (a, b)) in pair[0].iter().zip(&pair[1]).enumerate() {
                        let d = (a.1[0] * b.1[0] + a.1[1] * b.1[1] + a.1[2] * b.1[2] + a.1[3] * b.1[3]).abs().min(1.0);
                        let deg = 2.0 * d.acos().to_degrees();
                        if deg > 75.0 {
                            big += 1;
                            assert!(fast, "{what}: bone {} turns {deg} degrees from frame {f} to {}", s.bones[bi].name, f + 1);
                        }
                    }
                }
                assert!(big <= 8, "{what}: {big} big turns");
                for e in &seq.events {
                    // (v_shotgun's one-frame stand-in "fire" has events whose cycle is NaN in the file.)
                    assert!(e.cycle.is_nan() || (0.0..=1.0).contains(&e.cycle), "{what}: event cycle {}", e.cycle);
                }
            }
        }
        // The pistol has the usual set.
        let (_, pistol) = &all[0];
        assert!(pistol.sequences.len() >= 4, "{} sequences", pistol.sequences.len());
        for want in ["idle", "fire", "reload", "draw"] {
            assert!(pistol.sequences.iter().any(|q| q.name.contains(want)), "v_pistol has no {want} sequence");
        }
    }

    #[test]
    fn view_model_skinning_stays_near_the_origin() {
        let Some(g) = install() else { return };
        for (path, s) in view_models(g) {
            let m = g.model(path).unwrap();
            assert_eq!(m.bones, s.bones.len(), "{path}");
            let idle = s.sequences.iter().find(|q| q.name.contains("idle")).unwrap_or(&s.sequences[0]);
            let world = bone_to_model(&s.bones, &idle.frames[0]);
            let skin: Vec<Mat> = (0..s.bones.len()).map(|i| mat_mul(&world[i], &s.bones[i].pose_to_bone)).collect();
            let mut far = 0.0f32;
            let mut n = 0;
            for mesh in &m.meshes {
                for ((p, b), w) in mesh.positions.iter().zip(&mesh.bones).zip(&mesh.weights) {
                    let mut out = [0.0f32; 3];
                    for k in 0..3 {
                        if w[k] > 0.0 {
                            let q = mat_point(&skin[b[k] as usize], *p);
                            for c in 0..3 {
                                out[c] += w[k] * q[c];
                            }
                        }
                    }
                    far = far.max((out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt());
                    n += 1;
                }
            }
            // 60 units, or for a long model (the gravity gun's prongs reach 95 units in the bind pose) a
            // quarter more than its bind pose reaches.
            let bind_reach = m
                .meshes
                .iter()
                .flat_map(|me| &me.positions)
                .map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt())
                .fold(0.0f32, f32::max);
            let limit = 60.0f32.max(bind_reach * 1.25);
            assert!(n > 0 && far < limit, "{path}: a skinned vertex is {far} units from the origin in {} (limit {limit})", idle.name);
        }
    }

    #[test]
    #[ignore]
    fn print_view_models() {
        let Some(g) = install() else { return };
        for path in &VIEW_MODELS[..6] {
            let mdl = g.read(path).expect("mdl");
            let s = g.skeleton(path).unwrap();
            println!("=== {path}: version {}, {} bones, {} sequences", i32_at(&mdl, 4).unwrap(), s.bones.len(), s.sequences.len());
            for (i, b) in s.bones.iter().enumerate() {
                let parent = if b.parent < 0 { "-".to_string() } else { s.bones[b.parent as usize].name.clone() };
                println!("  bone {i}: {} (parent {parent})", b.name);
            }
            for a in &s.attachments {
                println!("  attachment {} (bone {})", a.name, s.bones[a.bone].name);
            }
            for q in &s.sequences {
                println!(
                    "  seq {} activity {:?} frames {} fps {} looping {} delta {} fade {}/{}",
                    q.name, q.activity, q.frames.len(), q.fps, q.looping, q.delta, q.fade_in, q.fade_out
                );
                if q.autoplay || !q.blend_to.is_empty() {
                    let weighted: Vec<String> = q.weights.iter().enumerate().filter(|(_, w)| **w > 0.0).map(|(i, w)| format!("{} {w}", s.bones[i].name)).collect();
                    let moved = q.blend_to.iter().zip(&q.frames[0]).filter(|(a, b)| a != b).count();
                    println!("      autoplay {} blend_to {} bones ({moved} differ) weights: {}", q.autoplay, q.blend_to.len(), weighted.join(", "));
                }
                for e in &q.events {
                    println!("      event cycle {:.3} event {} type {} options {:?} name {:?}", e.cycle, e.event, e.kind, e.options, e.name);
                }
            }
        }
    }
}
