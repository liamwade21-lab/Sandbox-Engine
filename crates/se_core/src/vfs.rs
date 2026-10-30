//! Virtual file system over SpaceEngine's data layout.
//!
//! SpaceEngine stores almost everything inside `.pak` files, which are plain ZIP
//! archives. Entries inside a pak are relative to the folder containing the pak,
//! so `data/textures/planets/Earth-Surface-PBC.pak` holding
//! `Earth/Surface-PBC/base.jpg` is visible as
//! `textures/planets/Earth/Surface-PBC/base.jpg`. Loose files override packed
//! ones, and later mount roots (e.g. `addons/`) override earlier ones.
//!
//! Lookups are case-insensitive (the original runs on Windows). Packed entries are
//! read with positioned reads, so a single [`Vfs`] can serve many threads at once.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VfsError {
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("entry is encrypted and cannot be read: {0}")]
    Encrypted(String),
    #[error("unsupported compression method {method} for {path}")]
    UnsupportedMethod { path: String, method: u16 },
    #[error("corrupt archive {path}: {reason}")]
    Corrupt { path: String, reason: String },
    #[error("io error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, VfsError>;

#[derive(Debug, Clone)]
enum Location {
    Loose(PathBuf),
    Packed {
        archive: u32,
        header_offset: u64,
        compressed: u64,
        size: u64,
        method: u16,
        encrypted: bool,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    /// Path with original capitalisation, as reported by [`Vfs::list`].
    display: String,
    location: Location,
}

struct Archive {
    path: PathBuf,
    file: File,
}

/// Metadata about a mounted pak archive.
#[derive(Debug, Clone)]
pub struct ArchiveInfo {
    pub path: PathBuf,
    pub entries: usize,
    pub encrypted_entries: usize,
}

#[derive(Default)]
pub struct Vfs {
    archives: Vec<Archive>,
    archive_info: Vec<ArchiveInfo>,
    /// Keyed by lower-cased virtual path with forward slashes.
    files: BTreeMap<String, Entry>,
}

/// Normalise a virtual path: forward slashes, no leading `./` or `/`, lower case.
pub fn normalize(path: &str) -> String {
    let mut s = path.replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    let s = s.trim_start_matches('/');
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for c in s.chars() {
        if c == '/' {
            if prev_slash {
                continue;
            }
            prev_slash = true;
        } else {
            prev_slash = false;
        }
        out.extend(c.to_lowercase());
    }
    out
}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mount a SpaceEngine installation: `data/` first, then `addons/` on top.
    pub fn mount_install(install_dir: &Path) -> Result<Self> {
        let mut vfs = Vfs::new();
        vfs.mount_dir(&install_dir.join("data"))?;
        let addons = install_dir.join("addons");
        if addons.is_dir() {
            vfs.mount_dir(&addons)?;
        }
        Ok(vfs)
    }

    /// Mount a directory tree as the virtual root. Paks are expanded in place.
    /// Within one directory, packed entries are added first and loose files after,
    /// so loose files win.
    pub fn mount_dir(&mut self, root: &Path) -> Result<()> {
        let mut stack = vec![(root.to_path_buf(), String::new())];
        while let Some((dir, vprefix)) = stack.pop() {
            let read = std::fs::read_dir(&dir).map_err(|e| io_err(&dir, e))?;
            let mut paks = Vec::new();
            let mut loose = Vec::new();
            for item in read {
                let item = item.map_err(|e| io_err(&dir, e))?;
                let path = item.path();
                let name = item.file_name().to_string_lossy().into_owned();
                let ft = item.file_type().map_err(|e| io_err(&path, e))?;
                if ft.is_dir() {
                    stack.push((path, format!("{vprefix}{name}/")));
                } else if name.to_ascii_lowercase().ends_with(".pak") {
                    paks.push(path);
                } else {
                    loose.push((path, format!("{vprefix}{name}")));
                }
            }
            paks.sort();
            for pak in paks {
                if let Err(e) = self.mount_pak(&pak, &vprefix) {
                    log::warn!("skipping pak {}: {e}", pak.display());
                }
            }
            for (path, vpath) in loose {
                self.files.insert(
                    normalize(&vpath),
                    Entry {
                        display: vpath,
                        location: Location::Loose(path),
                    },
                );
            }
        }
        Ok(())
    }

    /// Mount a single pak; its entries appear under `vprefix`.
    pub fn mount_pak(&mut self, pak: &Path, vprefix: &str) -> Result<()> {
        let file = File::open(pak).map_err(|e| io_err(pak, e))?;
        let central = zip::read_central_directory(&file, pak)?;
        let archive = self.archives.len() as u32;
        let mut encrypted_entries = 0;
        for e in &central {
            if e.name.ends_with('/') {
                continue;
            }
            if e.encrypted {
                encrypted_entries += 1;
            }
            let vpath = format!("{vprefix}{}", e.name);
            self.files.insert(
                normalize(&vpath),
                Entry {
                    display: vpath,
                    location: Location::Packed {
                        archive,
                        header_offset: e.header_offset,
                        compressed: e.compressed,
                        size: e.size,
                        method: e.method,
                        encrypted: e.encrypted,
                    },
                },
            );
        }
        self.archive_info.push(ArchiveInfo {
            path: pak.to_path_buf(),
            entries: central.len(),
            encrypted_entries,
        });
        self.archives.push(Archive {
            path: pak.to_path_buf(),
            file,
        });
        Ok(())
    }

    pub fn archives(&self) -> &[ArchiveInfo] {
        &self.archive_info
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.files.contains_key(&normalize(path))
    }

    /// Whether the entry exists and can actually be read (not encrypted).
    pub fn is_readable(&self, path: &str) -> bool {
        matches!(
            self.files.get(&normalize(path)).map(|e| &e.location),
            Some(Location::Loose(_)) | Some(Location::Packed { encrypted: false, .. })
        )
    }

    /// Uncompressed size of a file, if present.
    pub fn size(&self, path: &str) -> Option<u64> {
        match &self.files.get(&normalize(path))?.location {
            Location::Loose(p) => std::fs::metadata(p).ok().map(|m| m.len()),
            Location::Packed { size, .. } => Some(*size),
        }
    }

    /// List files whose virtual path starts with `dir` (recursively), with original case.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let mut prefix = normalize(dir);
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        self.files
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(_, e)| e.display.clone())
            .collect()
    }

    /// List files directly inside `dir` (not recursive) with the given extension
    /// (case-insensitive, without dot), sorted.
    pub fn list_ext(&self, dir: &str, ext: &str) -> Vec<String> {
        let mut prefix = normalize(dir);
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        let suffix = format!(".{}", ext.to_ascii_lowercase());
        let mut out: Vec<String> = self
            .files
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, _)| k.ends_with(&suffix) && !k[prefix.len()..].contains('/'))
            .map(|(_, e)| e.display.clone())
            .collect();
        out.sort();
        out
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let key = normalize(path);
        let entry = self
            .files
            .get(&key)
            .ok_or_else(|| VfsError::NotFound(path.to_string()))?;
        match &entry.location {
            Location::Loose(p) => std::fs::read(p).map_err(|e| io_err(p, e)),
            &Location::Packed {
                archive,
                header_offset,
                compressed,
                size,
                method,
                encrypted,
            } => {
                if encrypted {
                    return Err(VfsError::Encrypted(path.to_string()));
                }
                let arc = &self.archives[archive as usize];
                let data_start = zip::local_data_offset(&arc.file, &arc.path, header_offset)?;
                let mut raw = vec![0u8; compressed as usize];
                read_exact_at(&arc.file, &mut raw, data_start).map_err(|e| io_err(&arc.path, e))?;
                match method {
                    0 => Ok(raw),
                    8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, size as usize)
                        .map_err(|e| VfsError::Corrupt {
                            path: path.to_string(),
                            reason: format!("inflate failed: {e:?}"),
                        }),
                    m => Err(VfsError::UnsupportedMethod {
                        path: path.to_string(),
                        method: m,
                    }),
                }
            }
        }
    }

    pub fn read_string(&self, path: &str) -> Result<String> {
        let bytes = self.read(path)?;
        Ok(decode_text(&bytes))
    }
}

/// Decode a text file: UTF-8 (BOM stripped), falling back to Latin-1.
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn io_err(path: &Path, source: std::io::Error) -> VfsError {
    VfsError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_read(buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

/// Minimal ZIP central-directory reader (stored + deflate, zip64 aware).
mod zip {
    use super::{VfsError, io_err, read_exact_at};
    use std::fs::File;
    use std::path::Path;

    pub struct CentralEntry {
        pub name: String,
        pub method: u16,
        pub encrypted: bool,
        pub compressed: u64,
        pub size: u64,
        pub header_offset: u64,
    }

    fn u16le(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn u32le(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }
    fn u64le(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    }

    fn corrupt(path: &Path, reason: &str) -> VfsError {
        VfsError::Corrupt {
            path: path.display().to_string(),
            reason: reason.to_string(),
        }
    }

    pub fn read_central_directory(file: &File, path: &Path) -> Result<Vec<CentralEntry>, VfsError> {
        let len = file.metadata().map_err(|e| io_err(path, e))?.len();
        if len < 22 {
            return Err(corrupt(path, "too small"));
        }
        let tail_len = len.min(22 + 65535 + 20);
        let mut tail = vec![0u8; tail_len as usize];
        read_exact_at(file, &mut tail, len - tail_len).map_err(|e| io_err(path, e))?;
        let eocd = (0..=tail.len() - 22)
            .rev()
            .find(|&i| u32le(&tail, i) == 0x0605_4b50)
            .ok_or_else(|| corrupt(path, "end of central directory not found"))?;
        let mut count = u16le(&tail, eocd + 10) as u64;
        let mut cd_size = u32le(&tail, eocd + 12) as u64;
        let mut cd_offset = u32le(&tail, eocd + 16) as u64;
        if (count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF) && eocd >= 20 {
            let loc = eocd - 20;
            if u32le(&tail, loc) == 0x0706_4b50 {
                let z64_off = u64le(&tail, loc + 8);
                let mut rec = [0u8; 56];
                read_exact_at(file, &mut rec, z64_off).map_err(|e| io_err(path, e))?;
                if u32le(&rec, 0) != 0x0606_4b50 {
                    return Err(corrupt(path, "bad zip64 end record"));
                }
                count = u64le(&rec, 32);
                cd_size = u64le(&rec, 40);
                cd_offset = u64le(&rec, 48);
            }
        }
        let mut cd = vec![0u8; cd_size as usize];
        read_exact_at(file, &mut cd, cd_offset).map_err(|e| io_err(path, e))?;
        let mut out = Vec::with_capacity(count as usize);
        let mut p = 0usize;
        while p + 46 <= cd.len() && u32le(&cd, p) == 0x0201_4b50 {
            let flags = u16le(&cd, p + 8);
            let method = u16le(&cd, p + 10);
            let mut compressed = u32le(&cd, p + 20) as u64;
            let mut size = u32le(&cd, p + 24) as u64;
            let name_len = u16le(&cd, p + 28) as usize;
            let extra_len = u16le(&cd, p + 30) as usize;
            let comment_len = u16le(&cd, p + 32) as usize;
            let mut header_offset = u32le(&cd, p + 42) as u64;
            let name_end = p + 46 + name_len;
            if name_end + extra_len > cd.len() {
                return Err(corrupt(path, "central directory entry overruns"));
            }
            let name = String::from_utf8_lossy(&cd[p + 46..name_end]).replace('\\', "/");
            // Zip64 extended information extra field.
            let mut e = name_end;
            let extra_end = name_end + extra_len;
            while e + 4 <= extra_end {
                let id = u16le(&cd, e);
                let sz = u16le(&cd, e + 2) as usize;
                if id == 0x0001 {
                    let mut q = e + 4;
                    if size == 0xFFFF_FFFF && q + 8 <= extra_end {
                        size = u64le(&cd, q);
                        q += 8;
                    }
                    if compressed == 0xFFFF_FFFF && q + 8 <= extra_end {
                        compressed = u64le(&cd, q);
                        q += 8;
                    }
                    if header_offset == 0xFFFF_FFFF && q + 8 <= extra_end {
                        header_offset = u64le(&cd, q);
                    }
                }
                e += 4 + sz;
            }
            out.push(CentralEntry {
                name,
                method,
                encrypted: flags & 1 != 0,
                compressed,
                size,
                header_offset,
            });
            p = extra_end + comment_len;
        }
        Ok(out)
    }

    pub fn local_data_offset(file: &File, path: &Path, header_offset: u64) -> Result<u64, VfsError> {
        let mut h = [0u8; 30];
        read_exact_at(file, &mut h, header_offset).map_err(|e| io_err(path, e))?;
        if u32le(&h, 0) != 0x0403_4b50 {
            return Err(corrupt(path, "bad local file header"));
        }
        let n = u16le(&h, 26) as u64;
        let m = u16le(&h, 28) as u64;
        Ok(header_offset + 30 + n + m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize("Textures\\Planets//Earth/Base.JPG"), "textures/planets/earth/base.jpg");
        assert_eq!(normalize("./a/b"), "a/b");
        assert_eq!(normalize("/a"), "a");
    }

    #[test]
    fn decode_bom_and_latin1() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFabc"), "abc");
        assert_eq!(decode_text(&[0x61, 0xE9]), "a\u{e9}");
    }
}
