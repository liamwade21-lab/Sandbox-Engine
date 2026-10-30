//! Parser for SpaceEngine's binary mesh format (`.sm`, magic `SM01` / `SM02`).
//!
//! The format is documented in `docs/formats/sm_model.md`; it was derived from
//! `seMesh::LoadBinary` / `seMesh::SaveBinary` / `seMesh::ConvertBinary` in
//! `SpaceEngine.exe` and verified against every `.sm` shipped in the asteroid,
//! spacecraft and common model archives.
//!
//! Layout summary (all little endian):
//!
//! ```text
//! SM01: "SM01" | count:u32 | center:f32x3 | radius:f32                  | objects...
//! SM02: "SM02" | count:u32 | center:f32x3 | radius:f32 | vertex_fmt:u32 | objects...
//! object (SM01): nv:u32 ni:u32 material:u16 primitive:u16                (12 bytes)
//! object (SM02): the same 12 bytes + pos_offset:f32x3 pos_scale:f32
//!                                    uv_offset:f32x2 uv_scale:f32       (40 bytes)
//! then nv * stride(vertex_fmt) bytes of vertices, then ni * 4 bytes of u32 indices.
//! ```
//!
//! SM01 files always use vertex format 0 and the identity transform
//! (`pos_offset = 0`, `pos_scale = 1`, `uv_offset = 0`, `uv_scale = 1`).
//!
//! The format stores *no texture names*: an object only carries a material
//! index into the `Material "..."` list of the model's `.cfg` file.
//!
//! This module has no dependencies besides `std`, `thiserror` and `log`.

use log::debug;
use thiserror::Error;

/// Errors produced by [`parse_sm`].
#[derive(Debug, Error, PartialEq)]
pub enum SmError {
    /// The first four bytes are neither `SM01` nor `SM02`.
    #[error("bad .sm magic {0:?} (expected \"SM01\" or \"SM02\")")]
    BadMagic([u8; 4]),
    /// The file ended before a header or data block was complete.
    #[error("truncated .sm file: need {needed} bytes at offset {offset}, file has {len}")]
    Truncated {
        /// Offset at which the read started.
        offset: usize,
        /// Number of bytes required.
        needed: usize,
        /// Total file length.
        len: usize,
    },
    /// The SM02 header names a vertex format this parser does not know (valid: 0..=5).
    #[error("unknown .sm vertex format {0}")]
    UnknownVertexFormat(u32),
    /// An object header names an unknown primitive type (valid: 0..=8).
    #[error("object {object}: unknown primitive type {value}")]
    UnknownPrimitive {
        /// Object index.
        object: usize,
        /// Raw value from the file.
        value: u16,
    },
    /// An index refers to a vertex that does not exist.
    #[error("object {object}: index {index} >= vertex count {vertex_count}")]
    IndexOutOfRange {
        /// Object index.
        object: usize,
        /// Offending index value.
        index: u32,
        /// Number of vertices in the object.
        vertex_count: u32,
    },
    /// A size computed from the header does not fit in `usize`.
    #[error("size overflow while reading .sm file")]
    SizeOverflow,
    /// Bytes remain after the last object.
    #[error("{0} unexpected trailing bytes after last object")]
    TrailingBytes(usize),
}

/// File version, from the magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmVersion {
    /// `SM01`: no vertex-format field, 12 byte object headers, float vertices.
    Sm01,
    /// `SM02`: vertex-format field and 40 byte object headers.
    Sm02,
}

/// Per-vertex storage format (the `vertex_fmt` field of an SM02 header).
///
/// Strides: 48, 40, 24, 20, 20, 16 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmVertexFormat {
    /// 0: `f32` pos[3], normal[3], tangent[4], uv[2] (48 bytes).
    F32 = 0,
    /// 1: `f32` pos[3], normal[3], tangent[4]; no UV (40 bytes).
    F32NoUv = 1,
    /// 2: IEEE half pos[3], normal[3], tangent[4], uv[2] (24 bytes).
    F16 = 2,
    /// 3: IEEE half pos[3], normal[3], tangent[4]; no UV (20 bytes).
    F16NoUv = 3,
    /// 4: `i16` pos[3] + pad, packed 10:10:10 normal, packed 10:10:10:2 tangent, `i16` uv[2] (20 bytes).
    Packed = 4,
    /// 5: like [`SmVertexFormat::Packed`] without UV (16 bytes).
    PackedNoUv = 5,
}

impl SmVertexFormat {
    fn from_u32(v: u32) -> Result<Self, SmError> {
        Ok(match v {
            0 => Self::F32,
            1 => Self::F32NoUv,
            2 => Self::F16,
            3 => Self::F16NoUv,
            4 => Self::Packed,
            5 => Self::PackedNoUv,
            other => return Err(SmError::UnknownVertexFormat(other)),
        })
    }

    /// Size of one vertex in bytes.
    pub fn stride(self) -> usize {
        match self {
            Self::F32 => 48,
            Self::F32NoUv => 40,
            Self::F16 => 24,
            Self::F16NoUv | Self::Packed => 20,
            Self::PackedNoUv => 16,
        }
    }

    /// Whether vertices of this format carry texture coordinates.
    pub fn has_uv(self) -> bool {
        matches!(self, Self::F32 | Self::F16 | Self::Packed)
    }
}

/// Primitive topology of an object (the OpenGL primitive it was drawn with).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmPrimitive {
    /// 0: independent triangles (`GL_TRIANGLES`). Used by every shipped model.
    Triangles = 0,
    /// 1: triangle strip.
    TriangleStrip = 1,
    /// 2: triangle fan.
    TriangleFan = 2,
    /// 3: independent quads.
    Quads = 3,
    /// 4: quad strip.
    QuadStrip = 4,
    /// 5: independent line segments.
    Lines = 5,
    /// 6: line strip.
    LineStrip = 6,
    /// 7: line loop.
    LineLoop = 7,
    /// 8: points.
    Points = 8,
}

impl SmPrimitive {
    fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            0 => Self::Triangles,
            1 => Self::TriangleStrip,
            2 => Self::TriangleFan,
            3 => Self::Quads,
            4 => Self::QuadStrip,
            5 => Self::Lines,
            6 => Self::LineStrip,
            7 => Self::LineLoop,
            8 => Self::Points,
            _ => return None,
        })
    }
}

/// One sub-mesh ("object") of a model.
#[derive(Debug, Clone, PartialEq)]
pub struct SmObject {
    /// Index into the `Material` list of the model's `.cfg` (0-based).
    pub material_index: u16,
    /// Primitive topology of `indices`.
    pub primitive: SmPrimitive,
    /// Position decode offset as stored in the file (identity for non-packed formats).
    pub pos_offset: [f32; 3],
    /// Position decode scale as stored in the file (1 for non-packed formats).
    pub pos_scale: f32,
    /// UV decode offset as stored in the file.
    pub uv_offset: [f32; 2],
    /// UV decode scale as stored in the file.
    pub uv_scale: f32,
    /// Final (already decoded) vertex positions in model units.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals (degenerate packed normals may decode to zero).
    pub normals: Vec<[f32; 3]>,
    /// Tangents `xyz` with handedness `w` (+1 or -1; 0 for a few degenerate packed vertices).
    pub tangents: Vec<[f32; 4]>,
    /// Texture coordinates; empty if the vertex format has no UV.
    pub uvs: Vec<[f32; 2]>,
    /// Indices into the vertex arrays, interpreted according to `primitive`.
    pub indices: Vec<u32>,
}

impl SmObject {
    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Converts `indices` to a plain triangle list (CCW as stored).
    ///
    /// Strips, fans and quads are expanded; degenerate strip triangles are
    /// dropped. Line and point primitives yield an empty list.
    pub fn triangle_indices(&self) -> Vec<u32> {
        let ix = &self.indices;
        let mut out = Vec::new();
        match self.primitive {
            SmPrimitive::Triangles => {
                out.extend_from_slice(&ix[..ix.len() - ix.len() % 3]);
            }
            SmPrimitive::TriangleStrip => {
                for i in 2..ix.len() {
                    let (a, b, c) = (ix[i - 2], ix[i - 1], ix[i]);
                    if a == b || b == c || a == c {
                        continue;
                    }
                    if i % 2 == 0 {
                        out.extend_from_slice(&[a, b, c]);
                    } else {
                        out.extend_from_slice(&[b, a, c]);
                    }
                }
            }
            SmPrimitive::TriangleFan => {
                for i in 2..ix.len() {
                    out.extend_from_slice(&[ix[0], ix[i - 1], ix[i]]);
                }
            }
            SmPrimitive::Quads => {
                for q in ix.chunks_exact(4) {
                    out.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
                }
            }
            SmPrimitive::QuadStrip => {
                let mut i = 0;
                while i + 3 < ix.len() {
                    out.extend_from_slice(&[ix[i], ix[i + 1], ix[i + 3], ix[i], ix[i + 3], ix[i + 2]]);
                    i += 2;
                }
            }
            SmPrimitive::Lines | SmPrimitive::LineStrip | SmPrimitive::LineLoop | SmPrimitive::Points => {}
        }
        out
    }
}

/// A fully decoded `.sm` model.
#[derive(Debug, Clone, PartialEq)]
pub struct SmModel {
    /// File version.
    pub version: SmVersion,
    /// Vertex storage format used by every object in the file.
    pub vertex_format: SmVertexFormat,
    /// Centre of the axis-aligned bounding box of all vertices (model units).
    pub center: [f32; 3],
    /// Bounding radius from the header. Its exact definition is writer dependent
    /// (half AABB diagonal, or 0.866 of it); the engine uses `2 * radius` as the
    /// model's nominal length when the `.cfg` gives only `Unit` or only `Length`.
    pub radius: f32,
    /// Sub-meshes.
    pub objects: Vec<SmObject>,
}

impl SmModel {
    /// Axis-aligned bounding box `(min, max)` computed from the decoded positions,
    /// or `None` if the model has no vertices.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut it = self.objects.iter().flat_map(|o| o.positions.iter());
        let first = it.next()?;
        let (mut mn, mut mx) = (*first, *first);
        for p in it {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        Some((mn, mx))
    }

    /// Total vertex count over all objects.
    pub fn vertex_count(&self) -> usize {
        self.objects.iter().map(SmObject::vertex_count).sum()
    }

    /// Total index count over all objects.
    pub fn index_count(&self) -> usize {
        self.objects.iter().map(|o| o.indices.len()).sum()
    }
}

/// Little-endian cursor over a byte slice.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SmError> {
        let end = self.pos.checked_add(n).ok_or(SmError::SizeOverflow)?;
        if end > self.data.len() {
            return Err(SmError::Truncated { offset: self.pos, needed: n, len: self.data.len() });
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, SmError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, SmError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self) -> Result<f32, SmError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn f32s<const N: usize>(&mut self) -> Result<[f32; N], SmError> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
}

#[inline]
fn f32_at(b: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[inline]
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[inline]
fn i16_at(b: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([b[off], b[off + 1]])
}

/// IEEE 754 binary16 to binary32.
fn half_to_f32(h: u16) -> f32 {
    let sign = ((h as u32) & 0x8000) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, _) => {
            // Subnormal half: normalise.
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
        (0x1f, _) => sign | 0x7f80_0000 | (man << 13),
        _ => sign | ((exp + 127 - 15) << 23) | (man << 13),
    };
    f32::from_bits(bits)
}

#[inline]
fn half_at(b: &[u8], off: usize) -> f32 {
    half_to_f32(u16::from_le_bytes([b[off], b[off + 1]]))
}

/// Decodes one 10-bit sign-magnitude-like component used by packed normals/tangents:
/// values `0..=511` are positive, `512..=1023` map to `-(1023 - v)`; scaled by 1/511.
#[inline]
fn unpack10(v: u32) -> f32 {
    let v = (v & 0x3ff) as i32;
    let s = if v < 0x200 { v } else { -(0x3ff - v) };
    s as f32 / 511.0
}

/// Packed normal: three 10-bit components in bits 0..30.
fn unpack_normal(p: u32) -> [f32; 3] {
    [unpack10(p), unpack10(p >> 10), unpack10(p >> 20)]
}

/// Packed tangent: three 10-bit components plus a 2-bit handedness in bits 30..32
/// (`0 -> 0`, `1 -> +1`, `2 -> -1`, `3 -> -0`).
fn unpack_tangent(p: u32) -> [f32; 4] {
    let w = p >> 30;
    let w = if w < 2 { w as f32 } else { -((3 - w) as f32) };
    [unpack10(p), unpack10(p >> 10), unpack10(p >> 20), w]
}

/// Parses an SM01/SM02 mesh.
///
/// All vertex data is decoded to `f32` (positions in model units). The whole file
/// must be consumed, indices must be in range and every length is bounds-checked,
/// so arbitrary input never panics.
pub fn parse_sm(bytes: &[u8]) -> Result<SmModel, SmError> {
    let mut r = Reader { data: bytes, pos: 0 };
    let magic: [u8; 4] = r.take(4)?.try_into().expect("slice of length 4");
    let version = match &magic {
        b"SM01" => SmVersion::Sm01,
        b"SM02" => SmVersion::Sm02,
        _ => return Err(SmError::BadMagic(magic)),
    };
    let count = r.u32()? as usize;
    let center = r.f32s::<3>()?;
    let radius = r.f32()?;
    let vertex_format = match version {
        SmVersion::Sm01 => SmVertexFormat::F32,
        SmVersion::Sm02 => SmVertexFormat::from_u32(r.u32()?)?,
    };
    let stride = vertex_format.stride();
    debug!("sm: {version:?} format {vertex_format:?}, {count} objects, radius {radius}");

    // Each object needs at least a 12 byte header; avoid huge allocations on garbage counts.
    let mut objects = Vec::with_capacity(count.min(bytes.len() / 12 + 1));
    for obj_idx in 0..count {
        let nv = r.u32()?;
        let ni = r.u32()?;
        let material_index = r.u16()?;
        let prim_raw = r.u16()?;
        let primitive = SmPrimitive::from_u16(prim_raw)
            .ok_or(SmError::UnknownPrimitive { object: obj_idx, value: prim_raw })?;
        let (pos_offset, pos_scale, uv_offset, uv_scale) = match version {
            SmVersion::Sm01 => ([0.0; 3], 1.0, [0.0; 2], 1.0),
            SmVersion::Sm02 => {
                let po = r.f32s::<3>()?;
                let ps = r.f32()?;
                let uo = r.f32s::<2>()?;
                let us = r.f32()?;
                (po, ps, uo, us)
            }
        };
        let nv_us = nv as usize;
        let vbytes = nv_us.checked_mul(stride).ok_or(SmError::SizeOverflow)?;
        let vdata = r.take(vbytes)?;
        let ibytes = (ni as usize).checked_mul(4).ok_or(SmError::SizeOverflow)?;
        let idata = r.take(ibytes)?;

        let mut positions = Vec::with_capacity(nv_us);
        let mut normals = Vec::with_capacity(nv_us);
        let mut tangents = Vec::with_capacity(nv_us);
        let mut uvs = Vec::with_capacity(if vertex_format.has_uv() { nv_us } else { 0 });
        for v in vdata.chunks_exact(stride) {
            match vertex_format {
                SmVertexFormat::F32 | SmVertexFormat::F32NoUv => {
                    positions.push([f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)]);
                    normals.push([f32_at(v, 12), f32_at(v, 16), f32_at(v, 20)]);
                    tangents.push([f32_at(v, 24), f32_at(v, 28), f32_at(v, 32), f32_at(v, 36)]);
                    if vertex_format.has_uv() {
                        uvs.push([f32_at(v, 40), f32_at(v, 44)]);
                    }
                }
                SmVertexFormat::F16 | SmVertexFormat::F16NoUv => {
                    positions.push([half_at(v, 0), half_at(v, 2), half_at(v, 4)]);
                    normals.push([half_at(v, 6), half_at(v, 8), half_at(v, 10)]);
                    tangents.push([half_at(v, 12), half_at(v, 14), half_at(v, 16), half_at(v, 18)]);
                    if vertex_format.has_uv() {
                        uvs.push([half_at(v, 20), half_at(v, 22)]);
                    }
                }
                SmVertexFormat::Packed | SmVertexFormat::PackedNoUv => {
                    let q = |off| i16_at(v, off) as f32 / 32767.0;
                    positions.push([
                        q(0) * pos_scale + pos_offset[0],
                        q(2) * pos_scale + pos_offset[1],
                        q(4) * pos_scale + pos_offset[2],
                    ]);
                    normals.push(unpack_normal(u32_at(v, 8)));
                    tangents.push(unpack_tangent(u32_at(v, 12)));
                    if vertex_format.has_uv() {
                        uvs.push([q(16) * uv_scale + uv_offset[0], q(18) * uv_scale + uv_offset[1]]);
                    }
                }
            }
        }

        let indices: Vec<u32> = idata.chunks_exact(4).map(|c| u32_at(c, 0)).collect();
        if let Some(&bad) = indices.iter().find(|&&i| i >= nv) {
            return Err(SmError::IndexOutOfRange { object: obj_idx, index: bad, vertex_count: nv });
        }

        objects.push(SmObject {
            material_index,
            primitive,
            pos_offset,
            pos_scale,
            uv_offset,
            uv_scale,
            positions,
            normals,
            tangents,
            uvs,
            indices,
        });
    }

    let rest = bytes.len() - r.pos;
    if rest != 0 {
        return Err(SmError::TrailingBytes(rest));
    }
    Ok(SmModel { version, vertex_format, center, radius, objects })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_f32s(b: &mut Vec<u8>, v: &[f32]) {
        for x in v {
            b.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn push_u32(b: &mut Vec<u8>, v: u32) {
        b.extend_from_slice(&v.to_le_bytes());
    }

    fn push_u16(b: &mut Vec<u8>, v: u16) {
        b.extend_from_slice(&v.to_le_bytes());
    }

    /// SM02 header + one object header.
    fn sm02_head(fmt: u32, nv: u32, ni: u32, mat: u16, prim: u16, xf: [f32; 7]) -> Vec<u8> {
        let mut b = b"SM02".to_vec();
        push_u32(&mut b, 1);
        push_f32s(&mut b, &[1.0, 2.0, 3.0, 4.0]);
        push_u32(&mut b, fmt);
        push_u32(&mut b, nv);
        push_u32(&mut b, ni);
        push_u16(&mut b, mat);
        push_u16(&mut b, prim);
        push_f32s(&mut b, &xf);
        b
    }

    const IDENT: [f32; 7] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];

    fn triangle_f32() -> Vec<u8> {
        let mut b = sm02_head(0, 3, 3, 2, 0, IDENT);
        for i in 0..3 {
            let f = i as f32;
            // pos, normal, tangent, uv
            push_f32s(&mut b, &[f, f + 1.0, f + 2.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, -1.0, f * 0.5, 0.25]);
        }
        for i in 0..3u32 {
            push_u32(&mut b, i);
        }
        b
    }

    #[test]
    fn parses_sm02_float_triangle() {
        let m = parse_sm(&triangle_f32()).unwrap();
        assert_eq!(m.version, SmVersion::Sm02);
        assert_eq!(m.vertex_format, SmVertexFormat::F32);
        assert_eq!(m.center, [1.0, 2.0, 3.0]);
        assert_eq!(m.radius, 4.0);
        assert_eq!(m.objects.len(), 1);
        let o = &m.objects[0];
        assert_eq!(o.material_index, 2);
        assert_eq!(o.primitive, SmPrimitive::Triangles);
        assert_eq!(o.positions[2], [2.0, 3.0, 4.0]);
        assert_eq!(o.normals[1], [0.0, 0.0, 1.0]);
        assert_eq!(o.tangents[0], [1.0, 0.0, 0.0, -1.0]);
        assert_eq!(o.uvs[2], [1.0, 0.25]);
        assert_eq!(o.indices, vec![0, 1, 2]);
        assert_eq!(o.triangle_indices(), vec![0, 1, 2]);
        assert_eq!(m.bounds(), Some(([0.0, 1.0, 2.0], [2.0, 3.0, 4.0])));
        assert_eq!(m.vertex_count(), 3);
        assert_eq!(m.index_count(), 3);
    }

    #[test]
    fn parses_sm01_with_identity_transform() {
        let mut b = b"SM01".to_vec();
        push_u32(&mut b, 1);
        push_f32s(&mut b, &[0.0, 0.0, 0.0, 1.5]);
        push_u32(&mut b, 3);
        push_u32(&mut b, 3);
        push_u16(&mut b, 0);
        push_u16(&mut b, 0);
        for i in 0..3 {
            push_f32s(&mut b, &[i as f32, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        }
        for i in [2u32, 1, 0] {
            push_u32(&mut b, i);
        }
        let m = parse_sm(&b).unwrap();
        assert_eq!(m.version, SmVersion::Sm01);
        assert_eq!(m.vertex_format, SmVertexFormat::F32);
        assert_eq!(m.objects[0].pos_scale, 1.0);
        assert_eq!(m.objects[0].indices, vec![2, 1, 0]);
    }

    #[test]
    fn parses_packed_format_with_transform() {
        // format 4, one vertex; transform: offset (10,20,30), scale 2, uv offset (0.5,0.5), uv scale 0.5
        let xf = [10.0, 20.0, 30.0, 2.0, 0.5, 0.5, 0.5];
        let mut b = sm02_head(4, 1, 1, 0, 0, xf);
        for v in [32767i16, -32767, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // normal (511, 0, -511): x=511, y=0, z = 512 encodes -(1023-512) = -511
        push_u32(&mut b, 511 | (512 << 20));
        // tangent (0, 511, 0), w = 2 -> -1
        push_u32(&mut b, (511 << 10) | (2 << 30));
        for v in [32767i16, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        push_u32(&mut b, 0);
        let m = parse_sm(&b).unwrap();
        let o = &m.objects[0];
        assert_eq!(o.positions[0], [12.0, 18.0, 30.0]);
        assert_eq!(o.normals[0], [1.0, 0.0, -1.0]);
        assert_eq!(o.tangents[0], [0.0, 1.0, 0.0, -1.0]);
        assert_eq!(o.uvs[0], [1.0, 0.5]);
        assert_eq!(m.vertex_format.stride(), 20);
    }

    #[test]
    fn parses_packed_no_uv() {
        let mut b = sm02_head(5, 1, 1, 0, 0, IDENT);
        b.extend_from_slice(&[0u8; 8]);
        push_u32(&mut b, 0);
        push_u32(&mut b, 1 << 30);
        push_u32(&mut b, 0);
        let m = parse_sm(&b).unwrap();
        assert!(m.objects[0].uvs.is_empty());
        assert_eq!(m.objects[0].tangents[0][3], 1.0);
    }

    #[test]
    fn parses_half_format() {
        let mut b = sm02_head(2, 1, 1, 0, 0, IDENT);
        // 1.0 = 0x3c00, -2.0 = 0xc000, 0.5 = 0x3800, 0 = 0
        for h in [0x3c00u16, 0xc000, 0, 0, 0, 0x3c00, 0x3c00, 0, 0, 0xbc00, 0x3800, 0x3800] {
            push_u16(&mut b, h);
        }
        push_u32(&mut b, 0);
        let m = parse_sm(&b).unwrap();
        let o = &m.objects[0];
        assert_eq!(o.positions[0], [1.0, -2.0, 0.0]);
        assert_eq!(o.normals[0], [0.0, 0.0, 1.0]);
        assert_eq!(o.tangents[0], [1.0, 0.0, 0.0, -1.0]);
        assert_eq!(o.uvs[0], [0.5, 0.5]);
    }

    #[test]
    fn half_conversion_edge_cases() {
        assert_eq!(half_to_f32(0x0000), 0.0);
        assert!(half_to_f32(0x8000).is_sign_negative());
        assert_eq!(half_to_f32(0x7bff), 65504.0);
        assert_eq!(half_to_f32(0x0001), 2.0f32.powi(-24));
        assert!(half_to_f32(0x7c00).is_infinite());
        assert!(half_to_f32(0x7e00).is_nan());
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(parse_sm(b"SM03\0\0\0\0"), Err(SmError::BadMagic(*b"SM03")));
    }

    #[test]
    fn rejects_truncation_everywhere() {
        let full = triangle_f32();
        for n in 0..full.len() {
            assert!(parse_sm(&full[..n]).is_err(), "prefix of {n} bytes parsed");
        }
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut b = triangle_f32();
        b.push(0);
        assert_eq!(parse_sm(&b), Err(SmError::TrailingBytes(1)));
    }

    #[test]
    fn rejects_unknown_format_and_primitive() {
        let mut b = triangle_f32();
        b[24] = 9;
        assert_eq!(parse_sm(&b), Err(SmError::UnknownVertexFormat(9)));
        let mut b = triangle_f32();
        b[0x1c + 10] = 77; // primitive field of the first object header
        assert!(matches!(parse_sm(&b), Err(SmError::UnknownPrimitive { object: 0, .. })));
    }

    #[test]
    fn rejects_index_out_of_range() {
        let mut b = triangle_f32();
        let n = b.len();
        b[n - 4..].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(
            parse_sm(&b),
            Err(SmError::IndexOutOfRange { object: 0, index: 3, vertex_count: 3 })
        );
    }

    #[test]
    fn huge_counts_do_not_allocate_or_panic() {
        let mut b = sm02_head(0, u32::MAX, u32::MAX, 0, 0, IDENT);
        b.extend_from_slice(&[0; 16]);
        assert!(parse_sm(&b).is_err());
        let mut b = b"SM01".to_vec();
        push_u32(&mut b, u32::MAX);
        push_f32s(&mut b, &[0.0; 4]);
        assert!(parse_sm(&b).is_err());
    }

    #[test]
    fn triangle_expansion() {
        let mk = |primitive, indices: Vec<u32>| SmObject {
            material_index: 0,
            primitive,
            pos_offset: [0.0; 3],
            pos_scale: 1.0,
            uv_offset: [0.0; 2],
            uv_scale: 1.0,
            positions: vec![],
            normals: vec![],
            tangents: vec![],
            uvs: vec![],
            indices,
        };
        assert_eq!(mk(SmPrimitive::TriangleStrip, vec![0, 1, 2, 3]).triangle_indices(), vec![0, 1, 2, 2, 1, 3]);
        assert_eq!(mk(SmPrimitive::TriangleFan, vec![0, 1, 2, 3]).triangle_indices(), vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mk(SmPrimitive::Quads, vec![0, 1, 2, 3]).triangle_indices(), vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mk(SmPrimitive::QuadStrip, vec![0, 1, 2, 3]).triangle_indices(), vec![0, 1, 3, 0, 3, 2]);
        assert!(mk(SmPrimitive::Lines, vec![0, 1]).triangle_indices().is_empty());
        assert_eq!(mk(SmPrimitive::Triangles, vec![0, 1, 2, 3]).triangle_indices(), vec![0, 1, 2]);
    }
}
