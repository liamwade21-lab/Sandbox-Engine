//! The ship UI's movies and what they read (UI2; ARCHITECTURE.md A10), from the decoded UI export (`Ctx::ui_decoded`,
//! its `README.md` the handoff):
//! - `ui/movies/NAME.swf`: the export's SWF of each movie in A10's scope (`MOVIES`) through `elite_gfx::convert_movie`,
//!   which mends the export tool's slips and puts the bridge's compiled ActionScript (`crates/elite-gfx/as3/bridge.abc`,
//!   read by path: `elite-gfx` does not hand its bytes to a caller) before the movie's own code. Checked: it parses, holds
//!   the bridge's block byte for byte, and defines every image its characters cut (the export embedded them).
//! - `ui/fonts/*.ttf` and `ui/fonts/fontmap.json` (`se_core::elite_ui_layout::FontMap`): the TrueType files the font map's
//!   four names stand for (`$TitleFont` Euro Caps, `$ParagraphFont` Sintony, `$BoldParagraphFont` Sintony bold,
//!   `$NumeralFont` Telegrama Render); the movies' text fields name only those, and their code maps every other name to one
//!   of them (`EliteFontMap.as`).
//! - `ui/strings/en.json`: Elite's whole English string table, key -> text (the export's `strings/en_full.json`, made by
//!   `re/elite/tools/gfx/strings.py` from the English language archive; the export's `strings/en.json` is only the subset
//!   the notes cite).
//! - `ui/focus.json` (the `cockpitui` state machine) and `ui/controls.json`: the export's files as they are.
//!
//! `plan` works out every file from the export alone; `convert` writes them, `verify` compares the folder with a fresh
//! plan. `ui_shipped_movies_load` (ignored: it needs the GPU) runs every shipped movie from the folder alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use se_core::elite_ui_layout::{parse_fontmap, write_fontmap, FontFile, FontMap, MappedFont};

use super::models::read_json;
use super::{Ctx, Planned, Shipped};

/// The movies in A10's scope, in its order (the export's and the mod's file stem).
pub(super) const MOVIES: &[&str] = &[
    "cockpitgaugepanel",
    "cockpithudring",
    "targetsights",
    "cockpitinfopanel",
    "orbitalhud",
    "notifications",
    "cockpitexternalpanel",
    "cockpitinternalpanel2",
    "rolepanel3",
    "cockpitchatpanel",
    "fsshud",
    "saahud",
];
pub(super) const FONTMAP: &str = "ui/fonts/fontmap.json";
pub(super) const STRINGS: &str = "ui/strings/en.json";
pub(super) const FOCUS: &str = "ui/focus.json";
const CONTROLS: &str = "ui/controls.json";
/// The full string table in the export (re-made by `strings.py`; its README's "Re-making it").
const STRINGS_EXPORT: &str = "strings/en_full.json";
/// The brief's ceiling for the full table.
const STRINGS_MAX_BYTES: usize = 20 << 20;
/// SWF tag codes: the image definitions, and DoABC2.
const IMAGE_TAGS: [u16; 6] = [6, 20, 21, 35, 36, 90];
const DO_ABC2: u16 = 82;
const DO_ABC: u16 = 72;

/// The bridge's compiled ActionScript, committed beside its source in `elite-gfx`.
fn bridge() -> Result<Vec<u8>, String> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("elite-gfx").join("as3").join("bridge.abc");
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

fn read(dir: &Path, rel: &str) -> Result<Vec<u8>, String> {
    std::fs::read(dir.join(rel)).map_err(|e| format!("{}: {e}", dir.join(rel).display()))
}

/// An uncompressed SWF's stage (pixels) and its tags (code, body).
struct Swf<'a> {
    stage: [u32; 2],
    tags: Vec<(u16, &'a [u8])>,
}

fn swf(b: &[u8]) -> Result<Swf<'_>, String> {
    if b.len() < 9 || &b[..3] != b"FWS" {
        return Err("not an uncompressed SWF".into());
    }
    if u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize != b.len() {
        return Err("its header's length is not the file's".into());
    }
    // The stage rectangle: 5 bits of field size, then x min, x max, y min, y max (twips, signed).
    let bits = (b[8] >> 3) as usize;
    let bit = |k: usize| (b[8 + k / 8] >> (7 - k % 8)) & 1;
    let rect_bytes = (5 + 4 * bits).div_ceil(8);
    if b.len() < 8 + rect_bytes + 4 {
        return Err("its header is cut short".into());
    }
    let field = |f: usize| -> i64 {
        let v = (0..bits).fold(0i64, |v, k| (v << 1) | i64::from(bit(5 + f * bits + k)));
        if bits > 0 && (v >> (bits - 1)) & 1 == 1 { v - (1 << bits) } else { v }
    };
    let px = |lo: usize, hi: usize| ((field(hi) - field(lo)) as f64 / 20.0).round() as u32;
    let stage = [px(0, 1), px(2, 3)];
    let mut at = 8 + rect_bytes + 4;
    let mut tags = Vec::new();
    loop {
        let head = b.get(at..at + 2).ok_or_else(|| format!("a tag header at byte {at} runs past the end"))?;
        let h = u16::from_le_bytes([head[0], head[1]]);
        at += 2;
        let (code, mut len) = (h >> 6, (h & 0x3f) as usize);
        if len == 0x3f {
            let l = b.get(at..at + 4).ok_or("a long tag header runs past the end")?;
            len = u32::from_le_bytes([l[0], l[1], l[2], l[3]]) as usize;
            at += 4;
        }
        let body = b.get(at..at + len).ok_or_else(|| format!("tag {code} at byte {at} runs past the end"))?;
        at += len;
        if code == 0 {
            break;
        }
        tags.push((code, body));
    }
    if at != b.len() {
        return Err(format!("{} bytes after the end tag", b.len() - at));
    }
    Ok(Swf { stage, tags })
}

/// The image character ids a movie defines.
fn image_ids(s: &Swf) -> BTreeSet<u16> {
    s.tags.iter().filter(|(c, b)| IMAGE_TAGS.contains(c) && b.len() >= 2).map(|(_, b)| u16::from_le_bytes([b[0], b[1]])).collect()
}

/// Whether a tag is the bridge's block: the DoABC2 tag named `elite_gfx::convert::BRIDGE_BLOCK`.
fn is_bridge((code, body): &(u16, &[u8])) -> bool {
    let name = elite_gfx::convert::BRIDGE_BLOCK.as_bytes();
    *code == DO_ABC2 && body.get(4..4 + name.len()) == Some(name) && body.get(4 + name.len()) == Some(&0)
}

/// The bridge's block of a movie, its code: only when there is exactly one and it comes before every other code block
/// (the first class of a name wins, so it must be loaded before the movie's own).
fn bridge_code<'a>(s: &Swf<'a>) -> Option<&'a [u8]> {
    let mut code = s.tags.iter().enumerate().filter(|(_, t)| t.0 == DO_ABC2 || t.0 == DO_ABC);
    let (first, (_, body)) = code.next()?;
    let bridges = s.tags.iter().filter(|t| is_bridge(t)).count();
    (bridges == 1 && is_bridge(&s.tags[first])).then(|| &body[5 + elite_gfx::convert::BRIDGE_BLOCK.len()..])
}

/// A shipped movie as the game loads it: an ActionScript 3 SWF that parses to its end and holds the bridge's block as
/// `elite-gfx/as3/bridge.abc` is NOW (the folder check runs this on every movie, `shipped_files_are_the_exports` too, so a
/// movie converted with an older bridge is refused).
pub(super) fn check_movie(bytes: &[u8]) -> Result<(), String> {
    let s = swf(bytes)?;
    if s.tags.first().is_none_or(|(c, b)| *c != 69 || b.first().is_none_or(|f| f & 0x08 == 0)) {
        return Err("its first tag is not the file attributes saying ActionScript 3".into());
    }
    if bridge_code(&s) != Some(bridge()?.as_slice()) {
        return Err("it does not hold exactly one bridge block, before its own code, as elite-gfx/as3/bridge.abc has it".into());
    }
    Ok(())
}

/// The images a movie's characters need defined: every sub-image character (`characters.json`) and every external
/// image (`movie.json` `images`) whose id no other character takes; never id 0 (`convert_movie` drops it: the main
/// timeline's).
fn wanted_images(dir: &Path) -> Result<BTreeSet<u16>, String> {
    let chars = read_json(&dir.join("characters.json"))?;
    let chars = chars.as_object().ok_or("characters.json is not a table")?;
    let taken: BTreeSet<u16> = chars.keys().filter_map(|k| k.parse().ok()).collect();
    let mut want: BTreeSet<u16> = chars
        .iter()
        .filter(|(_, c)| c.get("kind").and_then(Value::as_str) == Some("sub_image"))
        .filter_map(|(k, _)| k.parse().ok())
        .collect();
    let movie = read_json(&dir.join("movie.json"))?;
    let images = movie.get("images").and_then(Value::as_object).ok_or("movie.json has no images table")?;
    want.extend(images.keys().filter_map(|k| k.parse::<u16>().ok()).filter(|id| !taken.contains(id)));
    want.remove(&0);
    Ok(want)
}

/// One planned movie with what its read-back says.
fn movie(ctx: &Ctx, rendering: &Value, bridge: &[u8], name: &str) -> Result<(Planned, String), String> {
    let dir = ctx.ui_decoded.join("movies").join(name);
    let src = format!("movies/{name}/{name}.swf");
    let out = elite_gfx::convert_movie(&read(&ctx.ui_decoded, &src)?, bridge).map_err(|e| format!("{src}: {e}"))?;
    let s = swf(&out).map_err(|e| format!("{name}: converted: {e}"))?;
    let stage = rendering.pointer(&format!("/movies/{name}/stage")).and_then(Value::as_array).ok_or_else(|| format!("rendering.json: no stage of {name}"))?;
    let want_stage: Vec<u32> = stage.iter().filter_map(Value::as_f64).map(|v| v as u32).collect();
    if want_stage != s.stage {
        return Err(format!("{name}: its stage is {:?}, rendering.json says {want_stage:?}", s.stage));
    }
    let defined = image_ids(&s);
    let want = wanted_images(&dir)?;
    if let Some(id) = want.iter().find(|id| !defined.contains(id)) {
        return Err(format!("{name}: image {id} is not in the converted movie"));
    }
    let line = format!("{name}: {} x {} px, {} images, {:.2} MB", s.stage[0], s.stage[1], defined.len(), out.len() as f64 / 1048576.0);
    let archive = rendering.pointer(&format!("/movies/{name}/archive")).and_then(Value::as_str).unwrap_or("?");
    Ok((
        Planned {
            path: format!("ui/movies/{name}.swf"),
            note: format!(
                "Elite's movie {name} ({} x {} px, {} images embedded by the export), through elite_gfx::convert_movie: the export \
                 tool's slips mended, the font library's import dropped (the game registers the fonts), and the bridge's \
                 ActionScript (ours, crates/elite-gfx/as3) put before the movie's own code.",
                s.stage[0],
                s.stage[1],
                defined.len()
            ),
            bytes: out,
            sources: vec![format!("decoded/{src}"), format!("Elite file {archive}"), "crates/elite-gfx/as3/bridge.abc".into()],
            grade: "A",
        },
        line,
    ))
}

/// The face of a family with that boldness: the family's own font, or a font named `<family> ...` (the proofs' rule,
/// `elite-gfx` `tests/rig.rs` `fonts`).
fn face<'a>(library: &'a [FontFile], family: &str, bold: bool) -> Option<&'a FontFile> {
    library.iter().find(|f| f.name == family && f.bold == bold).or_else(|| library.iter().find(|f| f.name.starts_with(family) && f.bold == bold))
}

/// The font map and the files it needs: (map, [(shipped file, export file)]).
fn font_plan(dec: &Path) -> Result<(FontMap, Vec<(String, String)>), String> {
    let glyphs = read_json(&dec.join("fonts/fonts_roman.glyphs.json"))?;
    let mut library: Vec<FontFile> = Vec::new();
    for f in glyphs.as_array().ok_or("fonts_roman.glyphs.json is not a list")? {
        let name = f.get("name").and_then(Value::as_str).ok_or("fonts_roman.glyphs.json: a font without a name")?;
        let bold = f.get("bold").and_then(Value::as_bool).ok_or("fonts_roman.glyphs.json: a font without its boldness")?;
        if !library.iter().any(|x| x.name == name && x.bold == bold) {
            library.push(FontFile { file: format!("{}.ttf", name.replace(' ', "_")), name: name.to_string(), bold });
        }
    }
    let map_src = read_json(&dec.join("strings/fontmap.json"))?;
    let lib_name = map_src.get("library").and_then(Value::as_str).ok_or("strings/fontmap.json: no library")?.to_string();
    let mut map = Vec::new();
    for e in map_src.get("entries").and_then(Value::as_array).ok_or("strings/fontmap.json: no entries")? {
        let field = |k: &str| e.get(k).and_then(Value::as_str).ok_or_else(|| format!("strings/fontmap.json: an entry without {k}"));
        let (name, family) = (field("name")?, field("font")?);
        let bold = e.get("bold").and_then(Value::as_bool).ok_or("strings/fontmap.json: an entry without bold")?;
        let regular = face(&library, family, bold).or_else(|| face(&library, family, false)).ok_or_else(|| format!("{name}: no face of {family} in {lib_name}"))?;
        let heavy = face(&library, family, true).unwrap_or(regular);
        map.push(MappedFont { name: name.to_string(), font: family.to_string(), bold, regular: regular.file.clone(), heavy: heavy.file.clone() });
    }
    let used: BTreeSet<&String> = map.iter().flat_map(|m| [&m.regular, &m.heavy]).collect();
    let fonts: Vec<FontFile> = library.into_iter().filter(|f| used.contains(&f.file)).collect();
    if fonts.iter().map(|f| &f.file).collect::<BTreeSet<_>>().len() != fonts.len() {
        return Err("two shipped fonts would share a file name".into());
    }
    // The export's file of a library font: `<Name>[_Bold]__<library>.ttf`.
    let files = fonts.iter().map(|f| (f.file.clone(), format!("fonts/{}{}__{lib_name}.ttf", f.name.replace(' ', "_"), if f.bold { "_Bold" } else { "" }))).collect();
    Ok((FontMap { library: lib_name, fonts, map }, files))
}

/// Every file of this kind, from the export alone, and the lines `convert` prints.
fn plan(ctx: &Ctx) -> Result<(Vec<Planned>, Vec<String>), String> {
    let dec = &ctx.ui_decoded;
    if !dec.join("README.md").is_file() {
        return Err(format!("ELITE_UI_EXPORT: no decoded UI export at {} (its README.md is missing)", dec.display()));
    }
    let rendering = read_json(&dec.join("rendering.json"))?;
    let bridge = bridge()?;
    let (mut files, mut lines) = (Vec::new(), Vec::new());
    for name in MOVIES {
        let (p, line) = movie(ctx, &rendering, &bridge, name)?;
        files.push(p);
        lines.push(line);
    }
    let (map, fonts) = font_plan(dec)?;
    for (file, src) in &fonts {
        let bytes = read(dec, src)?;
        check_font(&bytes).map_err(|e| format!("{src}: {e}"))?;
        let f = map.fonts.iter().find(|f| f.file == *file).expect("planned from the map");
        files.push(Planned {
            path: format!("ui/fonts/{file}"),
            note: format!("Elite's font {}{} from its font library {}, bytes unchanged.", f.name, if f.bold { " (bold)" } else { "" }, map.library),
            bytes,
            sources: vec![format!("decoded/{src}"), "the font library archive 95b6ccd26144... (re-made by re/elite/tools/gfx/export.py)".into()],
            grade: "A",
        });
    }
    let names: Vec<&str> = map.map.iter().map(|m| m.name.as_str()).collect();
    files.push(Planned {
        path: FONTMAP.into(),
        note: format!("The font map's names ({}) to the shipped files, with each file's name and boldness in the library.", names.join(", ")),
        bytes: write_fontmap(&map),
        sources: vec!["decoded/strings/fontmap.json (the English language data's scaleformlanguagedata record)".into(), "decoded/fonts/fonts_roman.glyphs.json".into()],
        grade: "B",
    });
    let strings = read(dec, STRINGS_EXPORT).map_err(|e| format!("{e} (re-make it: the README's \"Re-making it\", strings.py)"))?;
    if strings.len() > STRINGS_MAX_BYTES {
        return Err(format!("{STRINGS_EXPORT} is {} bytes, over the 20 MB the brief allows", strings.len()));
    }
    let table = elite_ui::text::Text::from_json(&strings).map_err(|e| format!("{STRINGS_EXPORT}: {e}"))?;
    let subset = read_json(&dec.join("strings/en.json"))?;
    let subset = subset.as_object().ok_or("strings/en.json is not a table")?;
    for (k, e) in subset {
        if !table.contains(k) {
            return Err(format!("{STRINGS_EXPORT} lacks {k}, which the export's subset holds"));
        }
        let text = e.get("text").and_then(Value::as_str).ok_or_else(|| format!("strings/en.json: {k} has no text"))?;
        if table.get_string(k) != text {
            return Err(format!("{STRINGS_EXPORT}'s text of {k} differs from the export's subset's"));
        }
    }
    let keys = serde_json::from_slice::<serde_json::Map<String, Value>>(&strings).map_err(|e| e.to_string())?.len();
    files.push(Planned {
        path: STRINGS.into(),
        note: format!("Elite's whole English string table ({keys} keys, the {} the export's subset holds among them), key -> text, as strings.py writes it.", subset.len()),
        bytes: strings,
        sources: vec![format!("decoded/{STRINGS_EXPORT} (re/elite/tools/gfx/strings.py)"), "Elite file 21903df0cbbbfc40290cac095519f073d5567c0b (the English language archive)".into()],
        grade: "A",
    });
    for (rel, src, what) in [
        (FOCUS, "cockpitui_state_machine.json", "The cockpitui state machine (focus between the panels, the head-look targets, the control sets), as the export decoded it."),
        (CONTROLS, "controls.json", "The UI's control sets and button table, as the export decoded them."),
    ] {
        files.push(Planned {
            path: rel.into(),
            bytes: read(dec, src)?,
            sources: vec![format!("decoded/{src} (re/elite/notes/ui_rendering.md section 4)")],
            grade: "B",
            note: what.into(),
        });
    }
    Ok((files, lines))
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let (files, lines) = plan(ctx)?;
    let total: usize = files.iter().map(|f| f.bytes.len()).sum();
    let count = files.len();
    Planned::ship(ctx, shipped, files)?;
    for l in lines {
        println!("ui movie {l}");
    }
    println!("ui movies, fonts, strings, focus: {count} files, {:.1} MB", total as f64 / 1048576.0);
    Ok(())
}

/// The folder's movies, fonts, font map, strings, focus machine and controls are what the export makes. Writes nothing.
pub(super) fn verify(ctx: &Ctx) -> Result<(), String> {
    let (files, _) = plan(ctx)?;
    Planned::same(ctx, &files, &["ui/movies", "ui/fonts", "ui/strings"])?;
    println!("ui movies: {} files match the UI export", files.len());
    Ok(())
}

/// A TrueType file a font reader takes: the sfnt header, a table directory inside the file holding the tables a TrueType
/// font needs, and `head`'s magic number.
pub(super) fn check_font(b: &[u8]) -> Result<(), String> {
    let u16_at = |at: usize| b.get(at..at + 2).map(|x| u16::from_be_bytes([x[0], x[1]]));
    let u32_at = |at: usize| b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    if !matches!(u32_at(0), Some(0x0001_0000 | 0x7472_7565)) {
        return Err("not a TrueType file".into());
    }
    let tables = u16_at(4).ok_or("its header is cut short")? as usize;
    let mut found = BTreeMap::new();
    for k in 0..tables {
        let at = 12 + 16 * k;
        let tag = b.get(at..at + 4).ok_or("its table directory is cut short")?;
        let (off, len) = (u32_at(at + 8).ok_or("cut short")? as usize, u32_at(at + 12).ok_or("cut short")? as usize);
        if off.checked_add(len).is_none_or(|end| end > b.len()) {
            return Err(format!("table {} runs past the end", String::from_utf8_lossy(tag)));
        }
        found.insert(tag.to_vec(), off);
    }
    for need in [&b"cmap"[..], b"head", b"hhea", b"hmtx", b"maxp", b"name", b"glyf", b"loca"] {
        if !found.contains_key(need) {
            return Err(format!("no {} table", String::from_utf8_lossy(need)));
        }
    }
    if u32_at(found[&b"head"[..]] + 12) != Some(0x5F0F_3CF5) {
        return Err("its head table's magic number is wrong".into());
    }
    Ok(())
}

/// `fontmap.json` as the game reads it, every file it names in `ui/fonts/`.
pub(super) fn check_fontmap(ctx: &Ctx, bytes: &[u8]) -> Result<(), String> {
    let m = parse_fontmap(bytes).map_err(|e| e.to_string())?;
    for f in &m.fonts {
        ctx.read(&format!("ui/fonts/{}", f.file))?;
    }
    Ok(())
}

/// The string table as the game's text rules read it.
pub(super) fn check_strings(bytes: &[u8]) -> Result<(), String> {
    elite_ui::text::Text::from_json(bytes).map(|_| ())
}

/// The focus machine: JSON with its initial state among its states.
pub(super) fn check_focus(bytes: &[u8]) -> Result<(), String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let initial = v.get("initial").and_then(Value::as_str).ok_or("no initial state")?;
    if v.get("states").and_then(|s| s.get(initial)).is_none() {
        return Err(format!("its initial state {initial} is not among its states"));
    }
    Ok(())
}

/// The game's answers to the movies, from the shipped string table and font map (`elite_ui::text`).
struct ShippedHost {
    text: elite_ui::text::Text,
    fonts: BTreeMap<String, String>,
}

impl elite_gfx::Host for ShippedHost {
    fn translate(&self, text: &str) -> String {
        self.text.translate(text)
    }

    fn get_string(&self, key: &str) -> String {
        self.text.get_string(key)
    }

    fn make_untranslated_string(&self, text: &str) -> String {
        text.to_string()
    }

    fn find_in_translation_string(&self, text: &str, find: &str, from: f64) -> f64 {
        let t = self.translate(text);
        let from = from.max(0.0) as usize;
        t.get(from..).and_then(|rest| rest.find(find)).map_or(-1.0, |i| (i + from) as f64)
    }

    fn localise_number(&self, number: f64, format: &str, extra: Option<&str>) -> String {
        self.text.localise_number(number, format, extra)
    }

    fn does_text_symbol_exist(&self, key: &str) -> bool {
        self.text.contains(key)
    }

    fn get_mapped_font_name(&self, name: &str) -> String {
        self.fonts.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    fn get_database_item_data(&self, _id: u32) -> elite_ui::Value {
        elite_ui::Value::Null
    }
}

/// An ActionScript error as the player traces it (an uncaught error in debug mode), a Break, or a tween's failed
/// completion: the runtime's own proofs' rule (`elite-gfx` `tests/rig.rs` `is_error`).
#[cfg(test)]
fn is_script_error(text: &str) -> bool {
    text.starts_with("Break:")
        || text.contains("Error #")
        || ["TypeError", "ReferenceError", "ArgumentError", "RangeError", "VerifyError", "SecurityError"].iter().any(|e| text.contains(e))
        || text.starts_with("tween onComplete:")
}

/// Every shipped movie, loaded from the mod's folder alone with the shipped fonts and strings, reaches ready (it calls
/// `SetReadyForInvokes`) within 10 s of 60 Hz steps and runs a further second with no ActionScript error and no line of
/// Ruffle's log at ERROR (as the runtime's proofs: no allow-list; the log is `.optimize/scratch/run/ui2/<movie>.ruffle.log`).
/// Needs a Vulkan GPU, so it is ignored by default.
#[test]
#[ignore]
fn ui_shipped_movies_load() {
    use std::sync::Arc;

    use bevy::render::render_resource::{Extent3d, TextureDescriptor, TextureDimension, TextureUsages};
    use bevy::render::settings::{Backends, RenderResources, WgpuSettings};

    let out = super::out_dir();
    let read = |rel: &str| std::fs::read(out.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let map = parse_fontmap(&read(FONTMAP)).unwrap();
    let mut fonts = Vec::new();
    let data: BTreeMap<String, Arc<Vec<u8>>> = map.fonts.iter().map(|f| (f.file.clone(), Arc::new(read(&format!("ui/fonts/{}", f.file))))).collect();
    for f in &map.fonts {
        fonts.push(elite_gfx::Font { name: f.name.clone(), bold: f.bold, italic: false, data: data[&f.file].clone() });
    }
    for m in &map.map {
        fonts.push(elite_gfx::Font { name: m.name.clone(), bold: false, italic: false, data: data[&m.regular].clone() });
        fonts.push(elite_gfx::Font { name: m.name.clone(), bold: true, italic: false, data: data[&m.heavy].clone() });
    }
    let host = Arc::new(ShippedHost {
        text: elite_ui::text::Text::from_json(&read(STRINGS)).unwrap(),
        fonts: map.map.iter().map(|m| (m.name.clone(), m.font.clone())).collect(),
    });
    let RenderResources(device, queue, _info, adapter, instance, ..) =
        bevy::tasks::block_on(bevy::render::renderer::initialize_renderer(Backends::VULKAN, None, &WgpuSettings::default()));
    let device = device.wgpu_device().clone();
    let gpu = elite_gfx::Gpu::new(instance.into_inner(), adapter.into_inner(), device.clone(), queue.into_inner());
    let logs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.optimize/scratch/run/ui2");
    std::fs::create_dir_all(&logs).unwrap();
    let mut failures = Vec::new();
    for name in MOVIES {
        use bevy::log::tracing_subscriber::util::SubscriberInitExt;
        let log_path = logs.join(format!("{name}.ruffle.log"));
        let writer = std::sync::Mutex::new(std::fs::File::create(&log_path).unwrap());
        let _log = bevy::log::tracing_subscriber::fmt().with_ansi(false).without_time().with_max_level(bevy::log::Level::WARN).with_writer(writer).finish().set_default();
        let swf = read(&format!("ui/movies/{name}.swf"));
        let stage = swf_stage(&swf);
        let target = device.create_texture(&TextureDescriptor {
            label: Some(name),
            size: Extent3d { width: stage[0], height: stage[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: elite_gfx::target::FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let files = elite_gfx::MovieFiles { name: name.to_string(), swf, fonts: fonts.clone(), host: host.clone() };
        let mut movie = match elite_gfx::Movie::load(&gpu, files, target) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let (mut t, mut errors, mut ready_at) = (0.0, Vec::new(), None);
        while t < ready_at.map_or(10.0, |r| r + 1.0) {
            if movie.step(1.0 / 60.0) {
                movie.draw();
            }
            t += 1.0 / 60.0;
            if ready_at.is_none() && movie.ready() {
                ready_at = Some(t);
            }
            for o in movie.drain() {
                if let elite_gfx::Out::Trace(text) = o
                    && is_script_error(&text)
                {
                    errors.push(text);
                }
            }
        }
        drop(movie);
        errors.extend(std::fs::read_to_string(&log_path).unwrap_or_default().lines().filter(|l| l.contains("ERROR")).map(str::to_string));
        match ready_at {
            None => failures.push(format!("{name}: not ready after 10 s")),
            Some(_) if !errors.is_empty() => failures.push(format!("{name}: {errors:?}")),
            Some(r) => println!("{name}: ready after {r:.2} s, no ActionScript error, no Ruffle error"),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[cfg(test)]
fn swf_stage(bytes: &[u8]) -> [u32; 2] {
    swf(bytes).expect("a shipped movie").stage
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal SWF: the header with a stage of 100 x 50 px, then the given tags (code, body) and the end tag.
    fn movie(tags: &[(u16, &[u8])]) -> Vec<u8> {
        // RECT: 5 bits of size (15), x 0..2000, y 0..1000 twips.
        let mut bits = String::from("01111");
        for v in [0u32, 2000, 0, 1000] {
            bits.push_str(&format!("{v:015b}"));
        }
        while bits.len() % 8 != 0 {
            bits.push('0');
        }
        let mut body: Vec<u8> = (0..bits.len() / 8).map(|k| u8::from_str_radix(&bits[8 * k..8 * k + 8], 2).unwrap()).collect();
        body.extend_from_slice(&[0, 30, 1, 0]);
        for (code, b) in tags.iter().chain([&(0u16, &[][..])]) {
            body.extend_from_slice(&((code << 6) | 0x3f).to_le_bytes());
            body.extend_from_slice(&(b.len() as u32).to_le_bytes());
            body.extend_from_slice(b);
        }
        let mut file = b"FWS\x0f".to_vec();
        file.extend_from_slice(&((body.len() + 8) as u32).to_le_bytes());
        file.extend_from_slice(&body);
        file
    }

    #[test]
    fn reads_a_movies_stage_tags_images_and_bridge() {
        let mut abc = vec![0, 0, 0, 0];
        abc.extend_from_slice(elite_gfx::convert::BRIDGE_BLOCK.as_bytes());
        abc.push(0);
        abc.extend_from_slice(b"CODE");
        let file = movie(&[(69, &[8, 0, 0, 0]), (36, &[7, 0, 5]), (DO_ABC2, &abc), (1, &[])]);
        let s = swf(&file).unwrap();
        assert_eq!(s.stage, [100, 50]);
        assert_eq!(s.tags.len(), 4);
        assert_eq!(image_ids(&s), BTreeSet::from([7]));
        assert_eq!(bridge_code(&s), Some(&b"CODE"[..]));
        let own = [1u8, 0, 0, 0, b'm', 0, b'X'];
        let after = movie(&[(69, &[8, 0, 0, 0]), (DO_ABC2, &own), (DO_ABC2, &abc)]);
        assert_eq!(bridge_code(&swf(&after).unwrap()), None, "a bridge after the movie's own code");
        let twice = movie(&[(69, &[8, 0, 0, 0]), (DO_ABC2, &abc), (DO_ABC2, &abc), (DO_ABC2, &own)]);
        assert_eq!(bridge_code(&swf(&twice).unwrap()), None, "two bridges");
        let mut cut = file.clone();
        cut.truncate(file.len() - 3);
        assert!(swf(&cut).is_err());
        assert!(swf(b"CWS\x0f\0\0\0\0\0").is_err());
    }

    #[test]
    fn checks_a_font_header() {
        let mut ttf = vec![0, 1, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0];
        let tags = [b"cmap", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name"];
        let data_at = 12 + 16 * tags.len();
        for (k, t) in tags.iter().enumerate() {
            ttf.extend_from_slice(*t);
            ttf.extend_from_slice(&[0; 4]);
            ttf.extend_from_slice(&((data_at + 16 * k) as u32).to_be_bytes());
            ttf.extend_from_slice(&16u32.to_be_bytes());
        }
        let mut tables = vec![0u8; 16 * tags.len()];
        tables[16 * 2 + 12..16 * 2 + 16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        ttf.extend_from_slice(&tables);
        check_font(&ttf).unwrap();
        let mut bad = ttf.clone();
        bad[data_at + 16 * 2 + 12] = 0;
        assert!(check_font(&bad).is_err());
        assert!(check_font(&ttf[..ttf.len() - 1]).is_err());
        assert!(check_font(b"OTTO\0\0").is_err());
    }
}
