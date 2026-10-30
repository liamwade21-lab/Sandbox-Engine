//! Minimal DDS reader: the top mip level of DXT1/DXT3/DXT5 (BC1/BC2/BC3), `ATI2` (BC5)
//! normal maps and uncompressed 24/32-bit images, decoded to RGBA8.
//!
//! The spacecraft textures (`textures/spacecraft/ShipsTex.pak`) are mostly DXT1/DXT5
//! with a full mip chain; only the first (largest) level is decoded. BC5 normal maps come
//! out as x, y and the rebuilt z (as SE's `FromBC5`). Other formats return an error.

/// Decoded image, tightly packed RGBA8, row 0 at the top.
#[derive(Debug, Clone, PartialEq)]
pub struct DdsImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdsError {
    NotDds,
    Truncated,
    Unsupported(String),
}

impl std::fmt::Display for DdsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DdsError::NotDds => write!(f, "not a DDS file"),
            DdsError::Truncated => write!(f, "truncated DDS data"),
            DdsError::Unsupported(s) => write!(f, "unsupported DDS format {s}"),
        }
    }
}

impl std::error::Error for DdsError {}

fn u32_at(b: &[u8], off: usize) -> Result<u32, DdsError> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(DdsError::Truncated)
}

/// Expands a 5:6:5 colour to 8 bits per channel.
fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 255 + 15) / 31) as u8, ((g * 255 + 31) / 63) as u8, ((b * 255 + 15) / 31) as u8]
}

/// Decodes the 8-byte colour half of a BC1/BC2/BC3 block into 16 RGBA texels.
/// `allow_alpha`: BC1 3-colour mode has a transparent-black fourth colour.
fn decode_color_block(blk: &[u8], allow_alpha: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([blk[0], blk[1]]);
    let c1 = u16::from_le_bytes([blk[2], blk[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [a[0], a[1], a[2], 255];
    pal[1] = [b[0], b[1], b[2], 255];
    if c0 > c1 || !allow_alpha {
        for k in 0..3 {
            pal[2][k] = ((2 * a[k] as u32 + b[k] as u32) / 3) as u8;
            pal[3][k] = ((a[k] as u32 + 2 * b[k] as u32) / 3) as u8;
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for k in 0..3 {
            pal[2][k] = ((a[k] as u32 + b[k] as u32) / 2) as u8;
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    let bits = u32::from_le_bytes([blk[4], blk[5], blk[6], blk[7]]);
    let mut out = [[0u8; 4]; 16];
    for (i, px) in out.iter_mut().enumerate() {
        *px = pal[((bits >> (2 * i)) & 3) as usize];
    }
    out
}

/// Decodes the 8-byte BC3 alpha half into 16 alpha values.
fn decode_alpha_block(blk: &[u8]) -> [u8; 16] {
    let (a0, a1) = (blk[0] as u32, blk[1] as u32);
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u32 {
            pal[1 + i as usize] = (((7 - i) * a0 + i * a1) / 7) as u8;
        }
    } else {
        for i in 1..5u32 {
            pal[1 + i as usize] = (((5 - i) * a0 + i * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits = 0u64;
    for (i, b) in blk[2..8].iter().enumerate() {
        bits |= (*b as u64) << (8 * i);
    }
    let mut out = [0u8; 16];
    for (i, a) in out.iter_mut().enumerate() {
        *a = pal[((bits >> (3 * i)) & 7) as usize];
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Bc1,
    Bc2,
    Bc3,
    /// Two-channel normal map (`ATI2`): x and y as BC4 blocks; z is rebuilt (SE's FromBC5).
    Bc5,
}

/// Rebuilds the z of a unit normal from 8-bit x and y, expanded as ship.glsl does (`2 v - NORM_ONE`,
/// so 127 is 0), stored so that the same expansion gives z back.
fn bc5_z(x: u8, y: u8) -> u8 {
    let e = |v: u8| (2.0 * v as f32 - 254.0) / 255.0;
    let (x, y) = (e(x), e(y));
    let z = (1.0 - x * x - y * y).clamp(0.0, 1.0).sqrt();
    ((255.0 * z + 254.0) * 0.5).round() as u8
}

fn decode_blocks(data: &[u8], w: usize, h: usize, kind: Block) -> Result<Vec<u8>, DdsError> {
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let bsize = if kind == Block::Bc1 { 8 } else { 16 };
    if data.len() < bw * bh * bsize {
        return Err(DdsError::Truncated);
    }
    let mut out = vec![0u8; w * h * 4];
    for by in 0..bh {
        for bx in 0..bw {
            let blk = &data[(by * bw + bx) * bsize..][..bsize];
            if kind == Block::Bc5 {
                let (xs, ys) = (decode_alpha_block(&blk[..8]), decode_alpha_block(&blk[8..]));
                for i in 0..16 {
                    let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                    if x < w && y < h {
                        out[(y * w + x) * 4..][..4].copy_from_slice(&[xs[i], ys[i], bc5_z(xs[i], ys[i]), 255]);
                    }
                }
                continue;
            }
            let (color_part, alpha): (&[u8], Option<[u8; 16]>) = match kind {
                Block::Bc1 => (blk, None),
                Block::Bc2 => {
                    let mut a = [0u8; 16];
                    for i in 0..16 {
                        let nib = (blk[i / 2] >> (4 * (i % 2))) & 15;
                        a[i] = nib * 17;
                    }
                    (&blk[8..], Some(a))
                }
                Block::Bc3 => (&blk[8..], Some(decode_alpha_block(&blk[..8]))),
                Block::Bc5 => unreachable!(),
            };
            let px = decode_color_block(color_part, kind == Block::Bc1);
            for ty in 0..4 {
                for tx in 0..4 {
                    let (x, y) = (bx * 4 + tx, by * 4 + ty);
                    if x >= w || y >= h {
                        continue;
                    }
                    let mut p = px[ty * 4 + tx];
                    if let Some(a) = alpha {
                        p[3] = a[ty * 4 + tx];
                    }
                    out[(y * w + x) * 4..][..4].copy_from_slice(&p);
                }
            }
        }
    }
    Ok(out)
}

/// Decodes the top mip level of a DDS file to RGBA8.
pub fn decode_dds(bytes: &[u8]) -> Result<DdsImage, DdsError> {
    if bytes.len() < 128 || &bytes[0..4] != b"DDS " {
        return Err(DdsError::NotDds);
    }
    let height = u32_at(bytes, 12)?;
    let width = u32_at(bytes, 16)?;
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(DdsError::Unsupported(format!("size {width}x{height}")));
    }
    let pf_flags = u32_at(bytes, 80)?;
    let fourcc = &bytes[84..88];
    let (w, h) = (width as usize, height as usize);
    let data = &bytes[128..];
    let rgba = if pf_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" => decode_blocks(data, w, h, Block::Bc1)?,
            b"DXT2" | b"DXT3" => decode_blocks(data, w, h, Block::Bc2)?,
            b"DXT4" | b"DXT5" => decode_blocks(data, w, h, Block::Bc3)?,
            b"ATI2" | b"BC5U" => decode_blocks(data, w, h, Block::Bc5)?,
            other => return Err(DdsError::Unsupported(String::from_utf8_lossy(other).into_owned())),
        }
    } else if pf_flags & 0x40 != 0 {
        // Uncompressed RGB(A) with bit masks; support the common 24/32-bit layouts.
        let bits = u32_at(bytes, 88)? as usize;
        let masks = [u32_at(bytes, 92)?, u32_at(bytes, 96)?, u32_at(bytes, 100)?, u32_at(bytes, 104)?];
        if bits != 24 && bits != 32 {
            return Err(DdsError::Unsupported(format!("{bits}-bit RGB")));
        }
        let bpp = bits / 8;
        if data.len() < w * h * bpp {
            return Err(DdsError::Truncated);
        }
        let shift = |m: u32| if m == 0 { None } else { Some(m.trailing_zeros()) };
        let mut out = vec![255u8; w * h * 4];
        for i in 0..w * h {
            let mut v = 0u32;
            for k in 0..bpp {
                v |= (data[i * bpp + k] as u32) << (8 * k);
            }
            for c in 0..3 {
                if let Some(s) = shift(masks[c]) {
                    out[i * 4 + c] = ((v & masks[c]) >> s) as u8;
                }
            }
            if let Some(s) = shift(masks[3]) {
                out[i * 4 + 3] = ((v & masks[3]) >> s) as u8;
            }
        }
        out
    } else {
        return Err(DdsError::Unsupported(format!("pixel format flags {pf_flags:#x}")));
    };
    Ok(DdsImage { width, height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, h: u32, fourcc: &[u8; 4]) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[0..4].copy_from_slice(b"DDS ");
        b[4..8].copy_from_slice(&124u32.to_le_bytes());
        b[12..16].copy_from_slice(&h.to_le_bytes());
        b[16..20].copy_from_slice(&w.to_le_bytes());
        b[76..80].copy_from_slice(&32u32.to_le_bytes());
        b[80..84].copy_from_slice(&4u32.to_le_bytes());
        b[84..88].copy_from_slice(fourcc);
        b
    }

    #[test]
    fn dxt1_solid_and_gradient_block() {
        let mut d = header(4, 4, b"DXT1");
        // c0 = pure red (0xF800), c1 = pure blue (0x001F), indices: row0 = 0,1,2,3.
        d.extend_from_slice(&0xF800u16.to_le_bytes());
        d.extend_from_slice(&0x001Fu16.to_le_bytes());
        d.extend_from_slice(&[0b11_10_01_00, 0, 0, 0]);
        let img = decode_dds(&d).unwrap();
        assert_eq!((img.width, img.height), (4, 4));
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.rgba[4..8], &[0, 0, 255, 255]);
        // 2/3 red + 1/3 blue.
        assert_eq!(&img.rgba[8..12], &[170, 0, 85, 255]);
        assert_eq!(&img.rgba[12..16], &[85, 0, 170, 255]);
        // Remaining rows use index 0.
        assert_eq!(&img.rgba[16..20], &[255, 0, 0, 255]);
    }

    #[test]
    fn dxt1_three_color_mode_has_transparent_black() {
        let mut d = header(4, 4, b"DXT1");
        d.extend_from_slice(&0x001Fu16.to_le_bytes()); // c0 < c1 -> 3-colour mode
        d.extend_from_slice(&0xF800u16.to_le_bytes());
        d.extend_from_slice(&[0b00_00_11_10, 0, 0, 0]);
        let img = decode_dds(&d).unwrap();
        assert_eq!(&img.rgba[0..4], &[127, 0, 127, 255]);
        assert_eq!(&img.rgba[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn dxt5_alpha_ramp() {
        let mut d = header(4, 4, b"DXT5");
        d.extend_from_slice(&[255, 0]); // a0 = 255 > a1 = 0: 8-step ramp
        // 3-bit indices: texel0 = 0 (255), texel1 = 1 (0), texel2 = 7 (0 -> (1*255+6*0)/7=36)
        let bits: u64 = 0 | (1 << 3) | (7 << 6);
        d.extend_from_slice(&bits.to_le_bytes()[..6]);
        d.extend_from_slice(&0xFFFFu16.to_le_bytes());
        d.extend_from_slice(&0xFFFFu16.to_le_bytes());
        d.extend_from_slice(&[0, 0, 0, 0]);
        let img = decode_dds(&d).unwrap();
        assert_eq!(img.rgba[3], 255);
        assert_eq!(img.rgba[7], 0);
        assert_eq!(img.rgba[11], 36);
        assert_eq!(&img.rgba[0..3], &[255, 255, 255]);
    }

    #[test]
    fn rejects_garbage_and_unknown_formats() {
        assert_eq!(decode_dds(b"nope").unwrap_err(), DdsError::NotDds);
        let d = header(4, 4, b"ATI1");
        assert!(matches!(decode_dds(&d), Err(DdsError::Unsupported(_))));
        let d = header(8, 8, b"DXT1");
        assert_eq!(decode_dds(&d).unwrap_err(), DdsError::Truncated);
    }

    #[test]
    fn ati2_normals_rebuild_z() {
        let mut d = header(4, 4, b"ATI2");
        // x block: every texel 128 (x = 0); y block: every texel 255 (y = 1).
        d.extend_from_slice(&[128, 128, 0, 0, 0, 0, 0, 0]);
        d.extend_from_slice(&[255, 255, 0, 0, 0, 0, 0, 0]);
        let img = decode_dds(&d).unwrap();
        assert_eq!(&img.rgba[0..4], &[128, 255, 127, 255]);
        // A flat normal (127, 127 expand to 0) has z = 1.
        assert_eq!(bc5_z(127, 127), 255);
    }

    #[test]
    fn non_multiple_of_four_sizes_are_cropped() {
        let mut d = header(5, 2, b"DXT1");
        for _ in 0..2 {
            d.extend_from_slice(&0xF800u16.to_le_bytes());
            d.extend_from_slice(&0u16.to_le_bytes());
            d.extend_from_slice(&[0, 0, 0, 0]);
        }
        let img = decode_dds(&d).unwrap();
        assert_eq!(img.rgba.len(), 5 * 2 * 4);
        assert_eq!(&img.rgba[16..20], &[255, 0, 0, 255]);
    }
}
