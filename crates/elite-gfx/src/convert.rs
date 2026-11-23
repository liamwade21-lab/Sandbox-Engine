//! Elite's exported SWF made runnable (ARCHITECTURE.md A10, "The converted movie"): pure bytes in,
//! bytes out. The game never calls it; the converter (UI2) and the tests do.
//!
//! Three slips of the export tool are mended, and one Scaleform convention taken out (found by the trial, `re/elite/work/ui/ruffle_spike`):
//! 1. the file-attributes tag must be the first tag (Ruffle reads the ActionScript 3 flag only
//!    there: some movies had an image before it and ran as ActionScript 1);
//! 2. one image is defined twice under one id: the first definition is kept;
//! 3. an image defined on character id 0 (the main timeline's id: an atlas whose cut sub-images are
//!    what the movie uses) is dropped, or the document class is bound to a bitmap.
//!
//! 4. the import of the font library (`gfxfontlib.swf`, importing no symbols: Scaleform's way of
//!    making the game's fonts available) is dropped: the game registers the fonts itself
//!    (`MovieFiles::fonts`), and a movie loads no files (Ruffle would wait for the import for ever).
//!    An import that names symbols is refused.
//!
//! Then the bridge's compiled ActionScript (`as3/bridge.abc`) goes in as an eager code block just
//! before the movie's own first code block: the first class of a name wins, and its script runs
//! when the frame loads, before the document class is made.

use ruffle_core::swf::{self, Compression, Header, TagCode};

/// The name of the bridge's code block in a converted movie.
pub const BRIDGE_BLOCK: &str = "elitegfx_bridge";

/// One tag as it is in the file: its code, its id if it defines a character, and its bytes
/// (record header included).
struct RawTag<'a> {
    code: u16,
    bytes: &'a [u8],
    body: &'a [u8],
}

impl RawTag<'_> {
    /// The character id the tag defines, if it is a defining tag.
    fn defines(&self) -> Option<u16> {
        let defining = matches!(
            TagCode::from_u16(self.code)?,
            TagCode::DefineShape
                | TagCode::DefineShape2
                | TagCode::DefineShape3
                | TagCode::DefineShape4
                | TagCode::DefineMorphShape
                | TagCode::DefineMorphShape2
                | TagCode::DefineBits
                | TagCode::DefineBitsJpeg2
                | TagCode::DefineBitsJpeg3
                | TagCode::DefineBitsJpeg4
                | TagCode::DefineBitsLossless
                | TagCode::DefineBitsLossless2
                | TagCode::DefineButton
                | TagCode::DefineButton2
                | TagCode::DefineFont
                | TagCode::DefineFont2
                | TagCode::DefineFont3
                | TagCode::DefineFont4
                | TagCode::DefineText
                | TagCode::DefineText2
                | TagCode::DefineEditText
                | TagCode::DefineSprite
                | TagCode::DefineSound
                | TagCode::DefineVideoStream
                | TagCode::DefineBinaryData
        );
        (defining && self.body.len() >= 2).then(|| u16::from_le_bytes([self.body[0], self.body[1]]))
    }

    fn is_image(&self) -> bool {
        matches!(
            TagCode::from_u16(self.code),
            Some(
                TagCode::DefineBits
                    | TagCode::DefineBitsJpeg2
                    | TagCode::DefineBitsJpeg3
                    | TagCode::DefineBitsJpeg4
                    | TagCode::DefineBitsLossless
                    | TagCode::DefineBitsLossless2
            )
        )
    }

    /// For an import tag, how many symbols it imports and from where.
    fn imports(&self) -> Option<(usize, String)> {
        let version = match TagCode::from_u16(self.code)? {
            TagCode::ImportAssets => 1,
            TagCode::ImportAssets2 => 2,
            _ => return None,
        };
        let end = self.body.iter().position(|&b| b == 0)?;
        let url = String::from_utf8_lossy(&self.body[..end]).into_owned();
        let count_at = end + 1 + if version == 2 { 2 } else { 0 };
        let count = self.body.get(count_at..count_at + 2).map_or(0, |c| u16::from_le_bytes([c[0], c[1]]));
        Some((usize::from(count), url))
    }

    fn is_code(&self) -> bool {
        self.code == TagCode::DoAbc as u16 || self.code == TagCode::DoAbc2 as u16
    }
}

/// Splits a tag stream into its tags.
fn tags(data: &[u8], version: u8) -> Result<Vec<RawTag<'_>>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let mut reader = swf::read::Reader::new(&data[at..], version);
        let (code, length) = reader
            .read_tag_code_and_length()
            .map_err(|e| format!("tag header at byte {at}: {e}"))?;
        let header = data.len() - at - reader.get_ref().len();
        let end = at + header + length;
        if end > data.len() {
            return Err(format!("tag {code} at byte {at} runs past the end of the file"));
        }
        out.push(RawTag { code, bytes: &data[at..end], body: &data[at + header..end] });
        at = end;
        if code == TagCode::End as u16 {
            break;
        }
    }
    Ok(out)
}

/// A tag's bytes with a long record header (code and a 32-bit length).
fn long_tag(code: TagCode, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 6);
    out.extend_from_slice(&(((code as u16) << 6) | 0x3f).to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// The eager DoABC2 tag holding the bridge (flags 0: its last script runs when the frame loads).
fn bridge_tag(bridge_abc: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(bridge_abc.len() + BRIDGE_BLOCK.len() + 5);
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(BRIDGE_BLOCK.as_bytes());
    body.push(0);
    body.extend_from_slice(bridge_abc);
    long_tag(TagCode::DoAbc2, &body)
}

/// Whether a DoABC2 tag is the bridge (its name after the flags).
fn is_bridge(tag: &RawTag) -> bool {
    tag.code == TagCode::DoAbc2 as u16 && tag.body.get(4..4 + BRIDGE_BLOCK.len()) == Some(BRIDGE_BLOCK.as_bytes())
}

/// Mends Elite's exported movie and puts the bridge's code in (an uncompressed SWF comes out).
/// Fails, saying why, if the movie is not what the mends expect.
pub fn convert_movie(swf: &[u8], bridge_abc: &[u8]) -> Result<Vec<u8>, String> {
    if bridge_abc.is_empty() {
        return Err("the bridge's code is empty".into());
    }
    let buf = swf::decompress_swf(swf).map_err(|e| format!("not an SWF: {e}"))?;
    let version = buf.header.version();
    let all = tags(&buf.data, version)?;

    let attributes: Vec<&RawTag> = all.iter().filter(|t| t.code == TagCode::FileAttributes as u16).collect();
    let [attributes] = attributes[..] else {
        return Err(format!("{} file-attributes tags: a movie has exactly one", attributes.len()));
    };
    // FileAttributes flags: bit 3 (0x08) is ActionScript 3.
    if attributes.body.first().is_none_or(|flags| flags & 0x08 == 0) {
        return Err("the file attributes do not say ActionScript 3".into());
    }
    if all.iter().any(is_bridge) {
        return Err("the movie already holds the bridge (converted twice?)".into());
    }
    let Some(first_code) = all.iter().position(RawTag::is_code) else {
        return Err("the movie has no ActionScript code block".into());
    };

    let mut out = Vec::with_capacity(buf.data.len() + bridge_abc.len() + 64);
    out.extend_from_slice(attributes.bytes);
    let mut defined = std::collections::HashSet::new();
    let mut images = std::collections::HashSet::new();
    for (i, tag) in all.iter().enumerate() {
        if tag.code == TagCode::FileAttributes as u16 {
            continue;
        }
        match tag.imports() {
            Some((0, _)) => continue,
            Some((count, url)) => return Err(format!("imports {count} symbols from {url}: not supported")),
            None => {}
        }
        if let Some(id) = tag.defines() {
            if tag.is_image() && (id == 0 || images.contains(&id)) {
                continue;
            }
            if id == 0 {
                return Err(format!("tag {} defines character id 0", tag.code));
            }
            if !defined.insert(id) {
                return Err(format!("character id {id} is defined twice (tag {})", tag.code));
            }
            if tag.is_image() {
                images.insert(id);
            }
        }
        if i == first_code {
            out.extend_from_slice(&bridge_tag(bridge_abc));
        }
        out.extend_from_slice(tag.bytes);
    }

    let source = buf.header.swf_header();
    let header = Header {
        compression: Compression::None,
        version: source.version,
        stage_size: source.stage_size,
        frame_rate: source.frame_rate,
        num_frames: source.num_frames,
    };
    let mut file = Vec::with_capacity(out.len() + 32);
    swf::write::write_swf_raw_tags(&header, &out, &mut file).map_err(|e| format!("write the SWF: {e}"))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::{BRIDGE_BLOCK, convert_movie, tags};
    use ruffle_core::swf::{self, Compression, Header, Rectangle, TagCode, Twips};

    fn tag(code: TagCode, body: &[u8]) -> Vec<u8> {
        super::long_tag(code, body)
    }

    fn image(id: u16, marker: u8) -> Vec<u8> {
        let mut body = id.to_le_bytes().to_vec();
        body.extend_from_slice(&[5, 1, 0, 1, 0, marker]);
        tag(TagCode::DefineBitsLossless2, &body)
    }

    fn movie(parts: &[Vec<u8>]) -> Vec<u8> {
        let header = Header {
            compression: Compression::Zlib,
            version: 15,
            stage_size: Rectangle { x_min: Twips::ZERO, x_max: Twips::from_pixels(100.0), y_min: Twips::ZERO, y_max: Twips::from_pixels(50.0) },
            frame_rate: swf::Fixed8::from_f64(30.0),
            num_frames: 1,
        };
        let mut file = Vec::new();
        swf::write::write_swf_raw_tags(&header, &parts.concat(), &mut file).unwrap();
        file
    }

    /// (code, defined id or 0, last body byte) of each tag of a converted movie.
    fn summary(file: &[u8]) -> Vec<(u16, u16, u8)> {
        let buf = swf::decompress_swf(file).unwrap();
        tags(&buf.data, 15)
            .unwrap()
            .iter()
            .map(|t| (t.code, t.defines().unwrap_or(0), t.body.last().copied().unwrap_or(0)))
            .collect()
    }

    const AS3: u8 = 0x08;

    #[test]
    fn mends_the_slips_and_puts_the_bridge_before_the_code() {
        let abc = tag(TagCode::DoAbc2, b"\x01\0\0\0movie\0CODE");
        let file = movie(&[
            image(0, 1),
            tag(TagCode::FileAttributes, &[AS3, 0, 0, 0]),
            image(5, 2),
            image(5, 3),
            tag(TagCode::ImportAssets2, b"fonts/gfxfontlib.swf\0\x01\0\0\0"),
            abc,
            tag(TagCode::ShowFrame, &[]),
            tag(TagCode::End, &[]),
        ]);
        let out = convert_movie(&file, b"BRIDGE").unwrap();
        assert_eq!(&out[..3], b"FWS");
        let s = summary(&out);
        let codes: Vec<u16> = s.iter().map(|t| t.0).collect();
        let fa = TagCode::FileAttributes as u16;
        let img = TagCode::DefineBitsLossless2 as u16;
        let abc2 = TagCode::DoAbc2 as u16;
        assert_eq!(codes, [fa, img, abc2, abc2, TagCode::ShowFrame as u16, TagCode::End as u16]);
        assert_eq!((s[1].1, s[1].2), (5, 2), "the first definition of image 5 is kept");
        assert_eq!(s[2].2, b'E', "the bridge's code ends the inserted block");
        let buf = swf::decompress_swf(&out[..]).unwrap();
        let all = tags(&buf.data, 15).unwrap();
        assert_eq!(&all[2].body[..4], &[0, 0, 0, 0], "the bridge block is eager");
        assert_eq!(&all[2].body[4..4 + BRIDGE_BLOCK.len()], BRIDGE_BLOCK.as_bytes());
        assert_eq!(&all[3].body[4..9], b"movie", "the movie's own code follows");
    }

    #[test]
    fn refuses_what_the_mends_do_not_cover() {
        let fa = tag(TagCode::FileAttributes, &[AS3, 0, 0, 0]);
        let abc = tag(TagCode::DoAbc2, b"\x01\0\0\0movie\0CODE");
        let sprite = |id: u16| tag(TagCode::DefineSprite, &[id.to_le_bytes().as_slice(), &[0, 0]].concat());
        let end = tag(TagCode::End, &[]);
        let cases = [
            (movie(&[abc.clone(), end.clone()]), "file-attributes"),
            (movie(&[tag(TagCode::FileAttributes, &[0, 0, 0, 0]), abc.clone()]), "ActionScript 3"),
            (movie(&[fa.clone(), end.clone()]), "no ActionScript"),
            (movie(&[fa.clone(), sprite(7), sprite(7), abc.clone()]), "defined twice"),
            (movie(&[fa.clone(), sprite(0), abc.clone()]), "id 0"),
            (movie(&[fa.clone(), tag(TagCode::ImportAssets2, b"lib.swf\0\x01\0\x01\0\x05\0Name\0"), abc.clone()]), "imports 1"),
        ];
        for (file, why) in cases {
            let error = convert_movie(&file, b"BRIDGE").unwrap_err();
            assert!(error.contains(why), "{error:?} should say {why:?}");
        }
        let once = convert_movie(&movie(&[fa, abc, end]), b"BRIDGE").unwrap();
        assert!(convert_movie(&once, b"BRIDGE").unwrap_err().contains("converted twice"));
        assert!(convert_movie(b"not a movie", b"BRIDGE").is_err());
    }
}
