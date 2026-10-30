//! Minimal TIFF reader for SE's single-channel lookup tables (`textures/common/galaxy_bulge.tif`, a 512 x 64
//! 32-bit float image): uncompressed strips, one sample per pixel, 8 / 16-bit unsigned or 32-bit float.

/// Width, height and the samples (row-major, first row first; integers scaled to 0..1).
pub fn read_gray_f32(b: &[u8]) -> Option<(u32, u32, Vec<f32>)> {
    let le = match b.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let s: [u8; 2] = b.get(o..o + 2)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let s: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    let (mut w, mut h, mut bits, mut compression, mut samples, mut format, mut rows_per_strip) = (0, 0, 8, 1, 1, 1, u32::MAX);
    let (mut offsets, mut counts) = (Vec::new(), Vec::new());
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        let (tag, typ, count) = (u16_at(e)?, u16_at(e + 2)?, u32_at(e + 4)? as usize);
        let size = match typ {
            3 => 2,
            4 => 4,
            _ => 0,
        };
        // Values fit in the entry when they take 4 bytes or fewer, else the entry points at them.
        let values = |count: usize| -> Option<Vec<u32>> {
            let base = if size * count <= 4 { e + 8 } else { u32_at(e + 8)? as usize };
            (0..count).map(|k| if size == 2 { u16_at(base + 2 * k).map(u32::from) } else { u32_at(base + 4 * k) }).collect()
        };
        if size == 0 {
            continue;
        }
        let first = values(1)?[0];
        match tag {
            256 => w = first,
            257 => h = first,
            258 => bits = first,
            259 => compression = first,
            273 => offsets = values(count)?,
            277 => samples = first,
            278 => rows_per_strip = first,
            279 => counts = values(count)?,
            339 => format = first,
            _ => {}
        }
    }
    if compression != 1 || samples != 1 || w == 0 || h == 0 || offsets.len() != counts.len() {
        return None;
    }
    let mut raw = Vec::new();
    for (&o, &c) in offsets.iter().zip(&counts) {
        raw.extend_from_slice(b.get(o as usize..(o + c) as usize)?);
    }
    let _ = rows_per_strip;
    let px = (w * h) as usize;
    let out: Vec<f32> = match (bits, format) {
        (32, 3) => raw.chunks_exact(4).take(px).map(|c| {
            let s: [u8; 4] = c.try_into().unwrap();
            if le { f32::from_le_bytes(s) } else { f32::from_be_bytes(s) }
        }).collect(),
        (16, 1) => raw.chunks_exact(2).take(px).map(|c| {
            let s: [u8; 2] = c.try_into().unwrap();
            (if le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) }) as f32 / 65535.0
        }).collect(),
        (8, 1) => raw.iter().take(px).map(|&v| v as f32 / 255.0).collect(),
        _ => return None,
    };
    (out.len() == px).then_some((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_float_strip_image() {
        // 2 x 1 float image, little-endian, one strip after the IFD.
        let mut b = b"II*\0".to_vec();
        b.extend_from_slice(&8u32.to_le_bytes());
        let tags: [(u16, u16, u32, u32); 8] =
            [(256, 3, 1, 2), (257, 3, 1, 1), (258, 3, 1, 32), (259, 3, 1, 1), (273, 4, 1, 110), (277, 3, 1, 1), (279, 4, 1, 8), (339, 3, 1, 3)];
        b.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for (t, ty, c, v) in tags {
            b.extend_from_slice(&t.to_le_bytes());
            b.extend_from_slice(&ty.to_le_bytes());
            b.extend_from_slice(&c.to_le_bytes());
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(b.len(), 110);
        b.extend_from_slice(&0.5f32.to_le_bytes());
        b.extend_from_slice(&2.0f32.to_le_bytes());
        assert_eq!(read_gray_f32(&b), Some((2, 1, vec![0.5, 2.0])));
    }
}
