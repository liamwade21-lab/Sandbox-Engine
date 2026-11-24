//! The scanner's and the two holograms' files (UI6; ARCHITECTURE.md A10 "The 3D parts"), from the UI export's `hud3d/`
//! (`re/elite/notes/ui_hud3d.md`), under `ui/hud3d/`:
//! - `radar.json` (`ui::scanner::RadarFile`): the meshes the exe builds in code that the scanner draws (the disc, the own-ship
//!   marker, the range indicator, from `radar_vertex_buffers.json`, the emulated builders: A), the three materials' attributes
//!   as their files hold them (A), and the Sidewinder's range table (the SensorSystem's set 1, A; session ui1 measured that the
//!   Sidewinder uses it, C). No contact shapes: the mod has no contacts (the brief's scope).
//! - `holograms.json` (`ui::hologram::HoloFile`): the two display materials' attributes (A), each target model's generation
//!   values and files (A), the 27-entry planet class table (`stellar_records/planet_class_table.json`, A), each class's surface
//!   data on the shared sphere (`planet_classes.json`, A), and the colours the stellar record and the cockpit's
//!   SchematicsComponent hold (the record dumps' words, A).
//! - `models/NAME.edm`: the Orrery, the star, the white dwarf, the neutron star, the black hole and each planet class's
//!   own sphere, LOD0 as `models::model` makes the ship's. The own
//!   ship's `sidewinder_schematic` is already shipped with the ship (`models.rs` `EXTRA`), with its material and texture.
//! - `tex/NAME.dds`: the disc's texture, the scanline texture and every height map those models and the class table name
//!   (one file for each distinct content), Elite's levels byte for byte.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use se_core::elite_asset::parse_edm;

use super::models::{self, at, list, read_json, text};
use super::{Ctx, Planned, Shipped};

const DIR: &str = "ui/hud3d";
pub(super) const RADAR: &str = "ui/hud3d/radar.json";
pub(super) const HOLOGRAMS: &str = "ui/hud3d/holograms.json";

/// The scanner's materials the game draws, by Elite's name.
const RADAR_MATERIALS: [&str; 3] = ["radar_radarplane", "radar_radarplayer", "radar_radarrangeindicator"];
/// The target hologram's models in scope (notes section 9.2): the panel's default and the four star models (the planets'
/// are each class's own sphere). Stations, belts, points of interest and the plain sphere (kind 5) are left out: the mod has
/// none of them.
const TARGET_MODELS: [&str; 5] = ["orrery", "starschematic", "white_dwarf_schematic", "neutronstarschematic", "blackholeschematic"];
/// The two display materials (own ship, target).
const DISPLAYS: [(&str, &str); 2] = [("own", "cockpitschematic_panel"), ("target", "cockpitschematictarget_panel")];
/// The scanline texture both display materials name.
const SCANLINES: &str = "cockpitschematicdocking_landingpad_panel.pscanlinetexture";
/// The Sidewinder's range table: the SensorSystem data's set 1 (`hud3d.json` radar.range_selection, A).
const RANGE_SET: &str = "set 1";

fn hud3d(ctx: &Ctx) -> PathBuf {
    ctx.ui_decoded.join("hud3d")
}

/// A material's attributes as name -> values.
fn attrs(rec: &Value, f: &str) -> Result<BTreeMap<String, Vec<f64>>, String> {
    Ok(models::attrs(rec, f)?.into_iter().map(|a| (a.name, a.values)).collect())
}

/// A vertex buffer's floats, `n` per vertex (the dump's `vertices` rows flattened in order).
fn floats(vb: &Value, key: &str, n: usize) -> Result<Vec<Vec<f64>>, String> {
    let f = format!("radar_vertex_buffers.json {key}");
    let rec = at(vb, key, &f)?;
    let flat: Vec<f64> = list(rec, "vertices", &f)?.iter().flat_map(|r| r.as_array().cloned().unwrap_or_default()).map(|x| x.as_f64().ok_or_else(|| format!("{f}: a value is not a number"))).collect::<Result<_, _>>()?;
    let count = at(rec, "count", &f)?.as_u64().ok_or_else(|| format!("{f}: count"))? as usize;
    if flat.len() < count * n {
        return Err(format!("{f}: {} floats, not {count} vertices of {n}", flat.len()));
    }
    Ok(flat.chunks_exact(n).take(count).map(<[f64]>::to_vec).collect())
}

/// A word of a record dump (`+0xOFF HEXWORD ...`), its float.
fn dump_float(dump: &str, record: &str, off: u32, f: &str) -> Result<f64, String> {
    let start = dump.find(record).ok_or_else(|| format!("{f}: no record {record}"))?;
    let tag = format!("+0x{off:03x} ");
    // (The record's words follow its heading lines, each `+0xOFF ...`, indented.)
    let line = dump[start..]
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("+0x"))
        .take_while(|l| l.trim_start().starts_with("+0x"))
        .find(|l| l.trim_start().starts_with(&tag))
        .ok_or_else(|| format!("{f}: {record} has no word at {tag}"))?;
    let word = line.split_whitespace().nth(1).ok_or_else(|| format!("{f}: {line}"))?;
    let bits = u32::from_str_radix(word, 16).map_err(|_| format!("{f}: {line} is not a word"))?;
    Ok(f32::from_bits(bits) as f64)
}

fn colour3(dump: &str, record: &str, off: u32, f: &str) -> Result<[f64; 3], String> {
    Ok([dump_float(dump, record, off, f)?, dump_float(dump, record, off + 4, f)?, dump_float(dump, record, off + 8, f)?])
}

/// The textures planned so far, by content: (file in `tex/`, its planned file).
#[derive(Default)]
struct Textures {
    by_bytes: BTreeMap<Vec<u8>, String>,
    files: Vec<Planned>,
}

impl Textures {
    /// A texture file of the export's `textures/` (checked as a DDS of `format` and, when given, `size`), planned once per
    /// content; its path in the mod's folder.
    fn add(&mut self, hud3d: &Path, file: &str, format: &str, size: Option<(u32, u32)>, why: &str) -> Result<String, String> {
        let rel = format!("decoded/hud3d/textures/{file}");
        let bytes = std::fs::read(hud3d.join("textures").join(file)).map_err(|e| format!("{rel}: {e}"))?;
        let info = se_core::dds::parse_dds(&bytes).map_err(|e| format!("{rel}: {e}"))?;
        if format!("{:?}", info.format).to_uppercase() != format.to_uppercase() || size.is_some_and(|(w, h)| (info.width, info.height) != (w, h)) {
            return Err(format!("{rel}: {:?} {}x{}, its record says {format} {size:?}", info.format, info.width, info.height));
        }
        if let Some(had) = self.by_bytes.get(&bytes) {
            return Ok(had.clone());
        }
        let path = format!("{DIR}/tex/{file}");
        self.by_bytes.insert(bytes.clone(), path.clone());
        self.files.push(Planned { path: path.clone(), bytes, sources: vec![rel], grade: "A", note: format!("Elite's texture ({format}), every level byte for byte: {why}.") });
        Ok(path)
    }
}

/// The scanner's `radar.json`.
fn radar(hud3d: &Path) -> Result<Planned, String> {
    let vb = read_json(&hud3d.join("radar_vertex_buffers.json"))?;
    let mats = read_json(&hud3d.join("materials.json"))?;
    let h = read_json(&hud3d.join("hud3d.json"))?;
    let tables = at(at(at(&h, "radar", "hud3d.json")?, "range_selection", "hud3d.json radar")?, "tables", "hud3d.json range_selection")?;
    let (_, set) = tables.as_object().and_then(|o| o.iter().find(|(k, _)| k.starts_with(RANGE_SET))).ok_or("hud3d.json: no range set 1")?;
    let ranges: Vec<f64> = set.as_array().ok_or("range set 1 is not a list")?.iter().map(|v| v.as_f64().ok_or("a range is not a number")).collect::<Result<_, _>>()?;
    let mut materials = BTreeMap::new();
    for m in RADAR_MATERIALS {
        let rec = at(&mats, m, "materials.json")?;
        materials.insert(m.to_string(), json!({ "shader": text(rec, "shader", m)?, "attrs": attrs(rec, m)? }));
    }
    let file = json!({
        "format": 1,
        "plane": floats(&vb, "RadarViewCommonPlaneVertices", 4)?,
        "player": floats(&vb, "RadarViewCommonPlayerVertices", 7)?,
        // (The dump reads its 24 floats 4 to a row; the vertex shader takes TEXCOORD0's xyz: 6 vertices of 3, then padding.)
        "range_indicator": floats(&vb, "RadarViewCommonRangeIndicatorVertices", 3)?.into_iter().take(6).collect::<Vec<_>>(),
        "materials": materials,
        "ranges_m": ranges,
    });
    let bytes = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
    Ok(Planned {
        path: RADAR.to_string(),
        bytes,
        sources: vec!["decoded/hud3d/radar_vertex_buffers.json".into(), "decoded/hud3d/materials.json".into(), "decoded/hud3d/hud3d.json (radar.range_selection)".into(), "Elite file faa55c57328a394a345d4df58a2b4da4667bd85c".into()],
        grade: "A",
        note: "The scanner: the disc's, the own-ship marker's and the range indicator's vertices as the exe's builders make them (emulated), the three Radar_* materials' attributes, the range table 500..8000 m.".into(),
    })
}

/// A model of `hud3d/` planned as `ui_models::model` plans a panel's, under `ui/hud3d/models/`, its vertices checked against
/// the bounds the export states within the decode error the export states for it (`decoded_bounds_error_m`: the target models
/// are hundreds of units across, so their error is larger than the panels' `BOUNDS_M`).
fn model(hud3d: &Path, stem: &str) -> Result<(se_core::elite_asset::Edm, Planned), String> {
    let (edm, expect) = models::model(hud3d, stem)?;
    let j = read_json(&hud3d.join(format!("models/{stem}.json")))?;
    let error = list(&j, "models", stem)?.first().and_then(|m| m.get("decoded_bounds_error_m")).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let tol = models::BOUNDS_M.max(error);
    let (lo, hi) = expect.model_bounds;
    for v in edm.draws.iter().flat_map(|d| &d.vertices) {
        let p = v.decode().position;
        if (0..3).any(|a| p[a] < lo[a] - tol || p[a] > hi[a] + tol) {
            return Err(format!("hud3d/models/{stem}.json: a decoded vertex {p:?} leaves the model's bounds {lo:?}..{hi:?} (+-{tol})"));
        }
    }
    let bytes = se_core::elite_asset::write_edm(&edm);
    if parse_edm(&bytes).map_err(|e| format!("{stem}.edm: {e}"))? != edm {
        return Err(format!("{stem}.edm reads back different"));
    }
    let (t, n): (usize, usize) = (edm.draws.iter().map(|d| d.indices.len() / 3).sum(), edm.draws.iter().map(|d| d.vertices.len()).sum());
    let planned = Planned {
        path: format!("{DIR}/models/{stem}.edm"),
        note: format!("Elite's target hologram model {stem}, LOD0 ({} draws, {t} triangles, {n} vertices): vertex and index bytes unchanged, {} bones, material {}.", edm.draws.len(), edm.parts.len(), edm.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")),
        bytes,
        sources: expect.sources.iter().map(|s| s.strip_prefix("decoded/").map_or_else(|| s.clone(), |r| format!("decoded/hud3d/{r}"))).collect(),
        grade: "A",
    };
    Ok((edm, planned))
}

/// A generation material's values: (pSchematic_Data, pHeightMapUVScrollSpeed, pVerticalTiling).
fn generation(a: &BTreeMap<String, Vec<f64>>, f: &str) -> Result<Value, String> {
    let get = |n: &str, k: usize| a.get(n).filter(|v| v.len() >= k).cloned().ok_or_else(|| format!("{f}: {n} has fewer than {k} values"));
    Ok(json!({ "data": get("pSchematic_Data", 4)?, "scroll": get("pHeightMapUVScrollSpeed", 2)?, "tiling": get("pVerticalTiling", 1)?[0] }))
}

/// The holograms' `holograms.json`, the target models and every texture.
fn holograms(hud3d: &Path, tex: &mut Textures) -> Result<Vec<Planned>, String> {
    let mats = read_json(&hud3d.join("materials.json"))?;
    let tjson = read_json(&hud3d.join("textures.json"))?;
    let mut files = Vec::new();
    let mut display = BTreeMap::new();
    for (key, m) in DISPLAYS {
        let rec = at(&mats, m, "materials.json")?;
        display.insert(key, json!({ "material": m, "shader": text(rec, "shader", m)?, "attrs": attrs(rec, m)? }));
    }
    let srec = at(&tjson, SCANLINES, "textures.json")?;
    let scanlines = tex.add(hud3d, &models::texture_file(SCANLINES), text(srec, "format", SCANLINES)?, None, "the holograms' scanlines")?;
    let mut target = BTreeMap::new();
    for stem in TARGET_MODELS {
        let (edm, planned) = model(hud3d, stem)?;
        let [m] = edm.materials.as_slice() else { return Err(format!("{stem}: {} materials, one was expected", edm.materials.len())) };
        let mname = m.name.to_lowercase();
        let rec = at(&mats, &mname, "materials.json")?;
        let a = attrs(rec, &mname)?;
        let t = list(rec, "textures", &mname)?.iter().find(|t| t.get(0).and_then(Value::as_str) == Some("pHeightMapTexture")).and_then(|t| t.get(2)).and_then(Value::as_str).ok_or_else(|| format!("{mname}: no height map"))?;
        let trec = at(&tjson, t, "textures.json")?;
        let size = (at(trec, "width", t)?.as_u64().unwrap_or(0) as u32, at(trec, "height", t)?.as_u64().unwrap_or(0) as u32);
        let heightmap = tex.add(hud3d, &models::texture_file(t), text(trec, "format", t)?, Some(size), &format!("{stem}'s height map"))?;
        let mut g = generation(&a, &mname)?;
        g["file"] = json!(planned.path);
        g["heightmap"] = json!(heightmap);
        target.insert(stem, g);
        files.push(planned);
    }
    // The class table and each class's data on the sphere.
    let table = read_json(&hud3d.join("stellar_records/planet_class_table.json"))?;
    let classes = read_json(&hud3d.join("planet_classes.json"))?;
    let mut rows = Vec::new();
    let mut class_data = BTreeMap::new();
    for r in table.as_array().ok_or("planet_class_table.json is not a list")? {
        let f = "planet_class_table.json";
        let key: Vec<u64> = list(r, "bytes", f)?.iter().map(|b| b.as_u64().ok_or("a key byte")).collect::<Result<_, _>>()?;
        let model_name = text(r, "model", f)?.to_string();
        let stem = model_name.to_lowercase();
        rows.push(json!({ "key": key, "model": model_name }));
        if class_data.contains_key(&stem) {
            continue;
        }
        let c = at(&classes, &stem, "planet_classes.json")?;
        let a: BTreeMap<String, Vec<f64>> = serde_json::from_value(at(c, "attrs", &stem)?.clone()).map_err(|e| format!("planet_classes.json {stem}: {e}"))?;
        let (w, h) = (at(c, "w", &stem)?.as_u64().unwrap_or(0) as u32, at(c, "h", &stem)?.as_u64().unwrap_or(0) as u32);
        let heightmap = tex.add(hud3d, &format!("{stem}__planetschematic__pheightmaptexture.dds"), "BC4", Some((w, h)), &format!("the planet class {model_name}'s height map"))?;
        // Each class has its own sphere (the same 1,649 vertices across, its vertex bytes its own: its uv layout for its
        // height map): each is shipped.
        let (_, planned) = model(hud3d, &stem)?;
        let mut g = generation(&a, &stem)?;
        g["heightmap"] = json!(heightmap);
        g["file"] = json!(planned.path);
        files.push(planned);
        class_data.insert(stem, g);
    }
    // The colours: the stellar record's two (no target) and the cockpit's SchematicsComponent block.
    let stellar = std::fs::read_to_string(hud3d.join("stellar_records/stellarschematicsdata.txt")).map_err(|e| format!("stellarschematicsdata.txt: {e}"))?;
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let proto_rel = "re/elite/work/numbers/records/proto_sidewinder_cockpit.txt";
    let proto = std::fs::read_to_string(project.join(proto_rel)).map_err(|e| format!("{proto_rel}: {e}"))?;
    // (The own ship's panel, `SchematicsComponent_RightPanel`; the six templates hold the same block.)
    let component = "SchematicsComponent template (243,12688)";
    let colours = json!({
        "stellar": [colour3(&stellar, "[280,13120]", 0x4, "stellarschematicsdata.txt")?, colour3(&stellar, "[280,13120]", 0x10, "stellarschematicsdata.txt")?],
        "component": [colour3(&proto, component, 0x118, proto_rel)?, colour3(&proto, component, 0x128, proto_rel)?, colour3(&proto, component, 0x138, proto_rel)?],
    });
    let file = json!({ "format": 1, "display": display, "scanlines": scanlines, "target": target, "planet_classes": rows, "class_data": class_data, "colours": colours });
    files.push(Planned {
        path: HOLOGRAMS.to_string(),
        bytes: serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?,
        sources: vec![
            "decoded/hud3d/materials.json".into(),
            "decoded/hud3d/planet_classes.json".into(),
            "decoded/hud3d/stellar_records/planet_class_table.json".into(),
            "decoded/hud3d/stellar_records/stellarschematicsdata.txt (record [280,13120])".into(),
            format!("{proto_rel} ({component})"),
            "Elite files 790fe6e4b4a5e9e3f69d289b9c03356f316f58af, 169a9af6be6b9aaf724d0ee31655aa62c6098c94".into(),
        ],
        grade: "A",
        note: "The holograms: the two Schematic_Display_BillboardSobel materials' attributes, each target model's generation values and files, \
               the 27-entry planet class table and each class's data, the stellar record's and the SchematicsComponent's colours."
            .into(),
    });
    Ok(files)
}

/// Every file of `ui/hud3d/`, worked out from the export alone.
fn plan(ctx: &Ctx) -> Result<Vec<Planned>, String> {
    let hud3d = hud3d(ctx);
    let mut tex = Textures::default();
    let mut files = vec![radar(&hud3d)?];
    let tjson = read_json(&hud3d.join("textures.json"))?;
    let disc = "radar_radarplane.pdiffusealphatexture";
    let drec = at(&tjson, disc, "textures.json")?;
    let size = (at(drec, "width", disc)?.as_u64().unwrap_or(0) as u32, at(drec, "height", disc)?.as_u64().unwrap_or(0) as u32);
    tex.add(&hud3d, &models::texture_file(disc), text(drec, "format", disc)?, Some(size), "the scanner disc's rings and wedge (alpha the mask)")?;
    files.extend(holograms(&hud3d, &mut tex)?);
    files.extend(tex.files);
    Ok(files)
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let files = plan(ctx)?;
    let total: usize = files.iter().map(|f| f.bytes.len()).sum();
    let n = files.len();
    Planned::ship(ctx, shipped, files)?;
    shipped.note(
        "The ship UI's scanner and holograms (ui/hud3d/): not shipped, as the mod has nothing they would show: the scanner's contact shapes, \
         stalks, overlays and spheres; the target hologram's station, belt, signal and point-of-interest models.",
    );
    println!("ui hud3d: {n} files, {:.1} MB", total as f64 / 1048576.0);
    Ok(())
}

/// The folder's `ui/hud3d/` is what the export makes. Writes nothing.
pub(super) fn verify(ctx: &Ctx) -> Result<(), String> {
    let files = plan(ctx)?;
    Planned::same(ctx, &files, &[DIR])?;
    println!("ui hud3d: {} files match the UI export", files.len());
    Ok(())
}

/// A written `radar.json` or `holograms.json` as the game reads it, every file it names there and read.
pub(super) fn check(ctx: &Ctx, name: &str, bytes: &[u8]) -> Result<(), String> {
    let named: Vec<String> = if name == RADAR {
        super::super::ui::scanner::RadarFile::parse(bytes)?;
        Vec::new()
    } else {
        let h = super::super::ui::hologram::HoloFile::parse(bytes)?;
        let mut out = vec![h.scanlines.clone()];
        for g in h.target.values().chain(h.class_data.values()) {
            out.push(g.heightmap.clone());
            out.extend(g.file.clone());
        }
        out.sort();
        out.dedup();
        out
    };
    for f in named {
        let b = ctx.read(&f)?;
        match Path::new(&f).extension().and_then(|e| e.to_str()) {
            Some("edm") => parse_edm(&b).map(|_| ()).map_err(|e| format!("{f}: {e}"))?,
            Some("dds") => se_core::dds::parse_dds(&b).map(|_| ()).map_err(|e| format!("{f}: {e}"))?,
            _ => return Err(format!("{name} names {f}, which is neither a model nor a texture")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record dump's word is read by its offset under its own heading, its float by the word's bits.
    #[test]
    fn dump_words_read_by_offset() {
        let dump = "[1,2] 8 bytes\n  +0x000 3f000000  u1056964608  f 0.5\n== component X template (9,9)\n[9,9] 8 bytes\n  +0x000 3e9eb852  u1050589266  f 0.31\n  +0x004 3df5c28f  u1039516303  f 0.12\n";
        assert_eq!(dump_float(dump, "[1,2]", 0, "t").unwrap(), 0.5);
        assert_eq!(dump_float(dump, "X template (9,9)", 4, "t").unwrap(), f64::from(0.12f32));
        assert!(dump_float(dump, "X template (9,9)", 8, "t").is_err(), "a word past the record");
        assert_eq!(colour3(dump, "[1,2]", 0, "t").is_err(), true, "the record holds one word");
    }
}
