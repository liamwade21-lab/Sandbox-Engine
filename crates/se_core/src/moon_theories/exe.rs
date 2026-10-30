//! Read-only view of `SpaceEngine.exe` addressed by virtual address.
//!
//! SpaceEngine keeps the coefficient tables of its analytic moon theories as
//! statically initialised C++ objects in the executable's `.data` section (each
//! satellite is an object with a vtable pointer, `std::vector`-like
//! (pointer, count) pairs and plain `double` arrays). The port reads those tables
//! from the user's own install at runtime; nothing is copied into the repository.
//!
//! Addresses are for build 0.991.49.2095 beta (image base `0x1_4000_0000`); the
//! theory loaders check a few sentinel values before trusting them.

use std::path::Path;

#[derive(Debug, Clone, Copy)]
struct Section {
    va: u64,
    vsize: u64,
    raw_off: usize,
    raw_size: usize,
}

pub struct ExeImage {
    bytes: Vec<u8>,
    base: u64,
    sections: Vec<Section>,
}

impl std::fmt::Debug for ExeImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExeImage({} bytes, base {:#x})", self.bytes.len(), self.base)
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

impl ExeImage {
    /// Read and parse the PE headers of the executable at `path`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(bytes)
    }

    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        let bad = || "not a 64-bit PE image".to_string();
        if bytes.get(0..2) != Some(b"MZ") {
            return Err(bad());
        }
        let pe = u32_at(&bytes, 0x3c).ok_or_else(bad)? as usize;
        if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(bad());
        }
        let nsec = u16_at(&bytes, pe + 6).ok_or_else(bad)? as usize;
        let optsz = u16_at(&bytes, pe + 20).ok_or_else(bad)? as usize;
        if u16_at(&bytes, pe + 24) != Some(0x20b) {
            return Err(bad());
        }
        let base = u64_at(&bytes, pe + 24 + 24).ok_or_else(bad)?;
        let so = pe + 24 + optsz;
        let mut sections = Vec::with_capacity(nsec);
        for i in 0..nsec {
            let o = so + i * 40;
            sections.push(Section {
                vsize: u32_at(&bytes, o + 8).ok_or_else(bad)? as u64,
                va: u32_at(&bytes, o + 12).ok_or_else(bad)? as u64,
                raw_size: u32_at(&bytes, o + 16).ok_or_else(bad)? as usize,
                raw_off: u32_at(&bytes, o + 20).ok_or_else(bad)? as usize,
            });
        }
        Ok(Self { bytes, base, sections })
    }

    /// File bytes backing `[va, va + len)`, or `None` when unmapped or zero-initialised.
    pub fn slice(&self, va: u64, len: usize) -> Option<&[u8]> {
        let rva = va.checked_sub(self.base)?;
        for s in &self.sections {
            if rva >= s.va && rva + len as u64 <= s.va + s.raw_size.min(s.vsize as usize) as u64 {
                let off = s.raw_off + (rva - s.va) as usize;
                return self.bytes.get(off..off + len);
            }
        }
        None
    }

    pub fn u64(&self, va: u64) -> Option<u64> {
        Some(u64::from_le_bytes(self.slice(va, 8)?.try_into().ok()?))
    }
    pub fn i32(&self, va: u64) -> Option<i32> {
        Some(i32::from_le_bytes(self.slice(va, 4)?.try_into().ok()?))
    }
    pub fn i8(&self, va: u64) -> Option<i8> {
        Some(*self.slice(va, 1)?.first()? as i8)
    }
    pub fn f64(&self, va: u64) -> Option<f64> {
        Some(f64::from_bits(self.u64(va)?))
    }
    /// `count` consecutive doubles.
    pub fn f64s(&self, va: u64, count: usize) -> Option<Vec<f64>> {
        let b = self.slice(va, count * 8)?;
        Some(b.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect())
    }
    /// A `(pointer, count)` pair as used by the theories' vector fields.
    pub fn vec_header(&self, va: u64) -> Option<(u64, usize)> {
        let ptr = self.u64(va)?;
        let n = self.i32(va + 8)?;
        (n >= 0 && n < 1_000_000).then_some((ptr, n as usize))
    }
}
