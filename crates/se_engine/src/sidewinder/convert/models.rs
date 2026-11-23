//! Elite's own Sidewinder models (T3a), from the decoded export: LOD0 of every model the stock ship draws as
//! `models/NAME.edm` (`.edm` version 2, `VertexDecode::Elite`: Elite's vertex and index bytes unchanged, the model's
//! bones, each draw's material by Elite's name), the materials they name as `materials.json` (`se_core::elite_ship`), and
//! the textures those name as `tex/NAME.dds`, Elite's levels byte for byte. All grade A.
//!
//! Which models: the stock ship's rigs name them (the export's `animation/`: hull `sidewinder` with its doors, plates and
//! vents, the three gear legs, cockpit `sidewinder_cockpit` with its glass, the tutorial overlays);
//! a name the export left as a hash (`0x...`) is the djb2 hash of the lower-case name, matched against the exported
//! models. Plus what the ship has that no rig hangs (`EXTRA`, the canopy's shards): the pilot chair inside and out, the
//! hologram, the shield. Never the CQC arena's `pg_` copies, nor the drive bolt-ons (`NOT_SHIPPED`).
//!
//! `plan` works out every file from the export alone; `convert` writes and checks them, `verify` checks the folder
//! against a fresh plan without writing (`shipped_files_are_the_exports`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use se_core::dds::DdsFormat;
use se_core::elite::{self, PartLayout, Vertex, VERTEX_BYTES};
use se_core::elite_asset::{parse_edm, write_edm, Edm, EdmDraw, EdmMaterial, EdmPart, VertexDecode, NO_PART};
use se_core::elite_ship::{parse_materials, write_materials, Attr, Material, Materials, SlotValue, TextureSlot};

use super::{Ctx, Shipped};

/// The animation files and the rig in each that is the stock ship's (None: the file's only rig). `assembly.rs` takes
/// the same rigs into `ship.json`.
pub(super) const RIGS: &[(&str, Option<&str>)] = &[
    ("hull_sidewinder", Some("sidewinder")),
    ("gear_front", None),
    ("gear_left", None),
    ("gear_right", None),
    ("cockpit", Some("sidewinder_cockpit")),
    ("cockpit_tutorial", None),
];
/// `SOURCES.txt`'s line on what is left out on purpose.
pub(super) const NOT_SHIPPED: &str = "Not shipped: the pg_sidewinder_extendedengine drive bolt-ons (the export's \
    engine_left / engine_right: rigs PG_SideWinder_ExtendedEngineLeft / Right, models driveboltonleft, driveboltonright, \
    plateleft, plateright, fin, material SideWinder_DriveBoltOn) are the Proving Ground arena's by the evidence at rest: \
    their pg_ names, as the arena's pg_sidewinder hull and cockpit, and the item database listing them only beside the \
    CQC loadouts.";
/// Models of the ship that no rig hangs (and `shard001`..`shard020`, the canopy blowout's pieces).
const EXTRA: &[&str] = &["cockpit_chair", "ext_cockpit_chair", "sidewinder_schematic", "sidewinder_shield"];
const SHARDS: usize = 20;
/// Textures shipped although no material names them: the default paint job's mask (its record in `paintjobs.json`) and
/// the headlight's IES profile (in `textures.json`).
const TEXTURES_EXTRA: &[(&str, &str)] = &[
    ("paintjob_sidewinder_default__texture.dds", "the default paint job's mask (paintjobs.json's DefaultPaintJob)"),
    ("ies_emergency_elmre_lp220l_fxo.dds", "the headlight's IES profile"),
];
/// How far the decoded vertices may lie from the bounds the export states.
pub(super) const BOUNDS_M: f32 = 0.000_05;

pub(super) fn read_json(path: &Path) -> Result<Value, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", path.display()))
}

/// A field of a JSON object, or an error naming the file and the field.
pub(super) fn at<'a>(v: &'a Value, key: &str, file: &str) -> Result<&'a Value, String> {
    v.get(key).ok_or_else(|| format!("{file}: no {key}"))
}

pub(super) fn list<'a>(v: &'a Value, key: &str, file: &str) -> Result<&'a Vec<Value>, String> {
    at(v, key, file)?.as_array().ok_or_else(|| format!("{file}: {key} is not a list"))
}

pub(super) fn text<'a>(v: &'a Value, key: &str, file: &str) -> Result<&'a str, String> {
    at(v, key, file)?.as_str().ok_or_else(|| format!("{file}: {key} is not text"))
}

pub(super) fn count(v: &Value, key: &str, file: &str) -> Result<usize, String> {
    at(v, key, file)?.as_u64().map(|n| n as usize).ok_or_else(|| format!("{file}: {key} is not a count"))
}

pub(super) fn floats<const N: usize>(v: &Value, key: &str, file: &str) -> Result<[f32; N], String> {
    let l = list(v, key, file)?;
    let f: Vec<f32> = l.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
    f.try_into().map_err(|_| format!("{file}: {key} is not {N} numbers"))
}

/// The djb2 hash of a name in lower case: what the export prints (`0x...`) for a rig node's model it couldn't name.
fn djb2(name: &str) -> u32 {
    name.to_lowercase().bytes().fold(5381u32, |h, c| h.wrapping_mul(33).wrapping_add(c as u32))
}

/// The exported models' file names (`decoded/models/NAME.json`).
pub(super) fn exported_models(dec: &Path) -> Result<Vec<String>, String> {
    Ok(std::fs::read_dir(dec.join("models"))
        .map_err(|e| format!("decoded/models: {e}"))?
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string))
        .collect())
}

/// The model file a rig node names: its name in lower case, or for a hash (`0x...`) the exported model hashing to it.
/// Never a CQC arena copy (`pg_`).
pub(super) fn resolve_model(model: &str, exported: &[String], file: &str) -> Result<String, String> {
    let name = match model.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()) {
        Some(hash) => exported
            .iter()
            .find(|n| djb2(n) == hash && !n.starts_with("pg_"))
            .ok_or_else(|| format!("{file}: no exported model hashes to {model}"))?
            .clone(),
        None => model.to_lowercase(),
    };
    if name.starts_with("pg_") {
        return Err(format!("{file}: the stock ship's rig hangs {name}, a CQC arena copy"));
    }
    Ok(name)
}

/// A `RIGS` entry's rig in its animation file: the named one, or the file's only rig.
pub(super) fn stock_rig<'a>(j: &'a Value, which: Option<&str>, rel: &str) -> Result<&'a Value, String> {
    let rigs = at(j, "rigs", rel)?.as_object().ok_or_else(|| format!("{rel}: rigs is not a table"))?;
    match which {
        Some(r) => rigs.get(r).ok_or_else(|| format!("{rel}: no rig {r}")),
        None if rigs.len() == 1 => Ok(rigs.values().next().unwrap()),
        None => Err(format!("{rel}: {} rigs, which is the ship's?", rigs.len())),
    }
}

/// The models to ship (the exported models' file names), in name order.
fn model_names(dec: &Path) -> Result<BTreeSet<String>, String> {
    let exported = exported_models(dec)?;
    let mut names = BTreeSet::new();
    for (file, rig) in RIGS {
        let rel = format!("animation/{file}.json");
        let j = read_json(&dec.join(&rel))?;
        let mut stack: Vec<&Value> = vec![at(stock_rig(&j, *rig, &rel)?, "root", &rel)?];
        while let Some(node) = stack.pop() {
            names.insert(resolve_model(text(node, "model", &rel)?, &exported, &rel)?);
            stack.extend(list(node, "children", &rel)?);
        }
    }
    names.extend(EXTRA.iter().map(|s| s.to_string()));
    names.extend((1..=SHARDS).map(|k| format!("shard{k:03}")));
    if let Some(n) = names.iter().find(|n| !exported.contains(n)) {
        return Err(format!("model {n} is not in the decoded export"));
    }
    Ok(names)
}

/// What a model's read-back compares against: the export's buffer, the part of it each draw is, and the stated bounds.
pub(super) struct Expect {
    /// The buffer's file in `models/`.
    pub(super) bin: String,
    parts: Vec<PartLayout>,
    /// Whether LOD0 draws every part of its buffer (then its vertices must fill the buffer's decoded bounds).
    whole_buffer: bool,
    buffer_bounds: ([f32; 3], [f32; 3]),
    pub(super) model_bounds: ([f32; 3], [f32; 3]),
    /// The model's JSON and buffer (relative to the folder above `dec`, as `decoded/models/...`) and Elite's file.
    pub(super) sources: Vec<String>,
}

/// One model set's LOD0 as an `.edm`, from `dec/models/NAME.json` and its buffer (the ship's decoded export, or the UI
/// export's `hud3d/`, which has the same shape: `ui_models.rs`).
pub(super) fn model(dec: &Path, name: &str) -> Result<(Edm, Expect), String> {
    let file = format!("models/{name}.json");
    let j = read_json(&dec.join(&file))?;
    let infos = list(&j, "models", &file)?;
    let [m] = infos.as_slice() else { return Err(format!("{file}: {} model infos; one was expected", infos.len())) };
    let lod = list(m, "lods", &file)?.first().ok_or_else(|| format!("{file}: no LODs"))?;
    let buf = list(m, "buffers", &file)?.get(count(lod, "buffer", &file)?).ok_or_else(|| format!("{file}: LOD0's buffer is missing"))?;
    let bin_name = text(buf, "file", &file)?.to_string();
    let bin = std::fs::read(dec.join("models").join(&bin_name)).map_err(|e| format!("decoded/models/{bin_name}: {e}"))?;
    let n_parts = count(buf, "parts", &file)?;
    let sizes = [count(buf, "rows", &file)?, count(buf, "tris", &file)?, count(buf, "verts", &file)?];
    if sizes[0] != 24 * n_parts || sizes.iter().sum::<usize>() != bin.len() {
        return Err(format!("{bin_name}: {} bytes, not the {sizes:?} {file} states", bin.len()));
    }
    let layout = elite::chunk_layout(&bin, n_parts).map_err(|e| format!("{bin_name}: {e}"))?;
    let material_names: Vec<String> =
        list(m, "materials", &file)?.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("{file}: a material is not text"))).collect::<Result<_, _>>()?;
    let objects = list(m, "objects", &file)?;
    let meshes = list(m, "meshes", &file)?;
    let source_file = text(buf, "source_file", &file)?.to_string();
    let mut draws = Vec::new();
    let mut parts = Vec::new();
    for o in count(lod, "first_object", &file)?..count(lod, "end_object", &file)? {
        let obj = objects.get(o).ok_or_else(|| format!("{file}: LOD0's object {o} is missing"))?;
        let mesh = meshes.get(count(obj, "mesh", &file)?).ok_or_else(|| format!("{file}: object {o}'s mesh is missing"))?;
        let part = count(mesh, "part", &file)?;
        let p = *layout.get(part).ok_or_else(|| format!("{file}: object {o}'s part {part} is not in {bin_name}"))?;
        if count(mesh, "tri_index_count", &file)? != 3 * p.triangles || count(mesh, "vertex_count", &file)? != p.vertices {
            return Err(format!("{file}: object {o}'s counts differ from {bin_name}'s part {part}"));
        }
        let mat_name = text(obj, "material", &file)?;
        let material = material_names.iter().position(|n| n == mat_name).ok_or_else(|| format!("{file}: object {o}'s material {mat_name} is not the model's"))?;
        let indices: Vec<u16> = bin[p.index_bytes()].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        if indices.iter().any(|&i| i as usize >= p.vertices) {
            return Err(format!("{bin_name}: part {part} has an index outside its vertices"));
        }
        let vertices = bin[p.vertex_bytes()].chunks_exact(VERTEX_BYTES).map(|v| Vertex(v.try_into().unwrap())).collect();
        draws.push(EdmDraw {
            part: NO_PART,
            material: material as u16,
            source: format!("{bin_name} part {part} (LOD0 object {o}; Elite file {source_file})"),
            vertices,
            indices,
        });
        parts.push(p);
    }
    let whole_buffer = parts.len() == n_parts && (0..n_parts).all(|k| parts.iter().any(|p| *p == layout[k]));
    let mut bones = Vec::new();
    for (k, b) in list(&j, "bones", &file)?.iter().enumerate() {
        let bf = format!("{file} bone {k}");
        let parent = at(b, "parent", &bf)?.as_i64().filter(|&p| p >= -1 && p < k as i64).ok_or_else(|| format!("{bf}: its parent does not come before it"))?;
        bones.push(EdmPart {
            name: text(b, "name", &bf)?.to_string(),
            parent: parent as i16,
            translation: floats(b, "local_position", &bf)?,
            rotation: floats(b, "local_rotation", &bf)?,
            scale: at(b, "local_scale", &bf)?.as_f64().ok_or_else(|| format!("{bf}: local_scale is not a number"))? as f32,
            inverse_bind: Some(floats(b, "inverse_bind", &bf)?),
        });
    }
    let materials = material_names.iter().map(|n| EdmMaterial { name: n.clone(), textures: Vec::new(), tint: [1.0; 4], flags: 0 }).collect();
    let edm = Edm { metres_per_unit: 1.0, vertex_decode: VertexDecode::Elite, parts: bones, materials, draws, animations: Vec::new() };
    let expect = Expect {
        bin: bin_name.clone(),
        parts,
        whole_buffer,
        buffer_bounds: (floats(buf, "decoded_min", &file)?, floats(buf, "decoded_max", &file)?),
        model_bounds: (floats(m, "bounds_min", &file)?, floats(m, "bounds_max", &file)?),
        sources: vec![format!("decoded/{file}"), format!("decoded/models/{bin_name}"), format!("Elite file {source_file}")],
    };
    Ok((edm, expect))
}

/// The written model reads back with the export's bytes and fills the bounds it states.
fn check_model(ctx: &Ctx, rel: &str, written: &Edm, e: &Expect) -> Result<(), String> {
    let back = parse_edm(&ctx.read(rel)?).map_err(|err| format!("{rel}: {err}"))?;
    if back != *written {
        return Err(format!("{rel} reads back different"));
    }
    let bin = std::fs::read(ctx.export.join("models").join(&e.bin)).map_err(|err| format!("{}: {err}", e.bin))?;
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (k, (d, p)) in back.draws.iter().zip(&e.parts).enumerate() {
        let vb: Vec<u8> = d.vertices.iter().flat_map(|v| v.0).collect();
        let ib: Vec<u8> = d.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        if vb != bin[p.vertex_bytes()] || ib != bin[p.index_bytes()] {
            return Err(format!("{rel}: draw {k}'s bytes differ from {}'s", e.bin));
        }
        for v in &d.vertices {
            let pos = v.decode().position;
            for a in 0..3 {
                lo[a] = lo[a].min(pos[a]);
                hi[a] = hi[a].max(pos[a]);
            }
        }
    }
    let (mlo, mhi) = e.model_bounds;
    if (0..3).any(|a| lo[a] < mlo[a] - BOUNDS_M || hi[a] > mhi[a] + BOUNDS_M) {
        return Err(format!("{rel}: decoded vertices {lo:?}..{hi:?} leave the model's bounds {mlo:?}..{mhi:?}"));
    }
    let (blo, bhi) = e.buffer_bounds;
    if e.whole_buffer && (0..3).any(|a| (lo[a] - blo[a]).abs() > BOUNDS_M || (hi[a] - bhi[a]).abs() > BOUNDS_M) {
        return Err(format!("{rel}: decoded vertices {lo:?}..{hi:?} don't fill {}'s decoded bounds {blo:?}..{bhi:?}", e.bin));
    }
    Ok(())
}

/// The texture's file name in `decoded/textures` and `tex/`: its name with the dot as `__`.
pub(super) fn texture_file(texture: &str) -> String {
    format!("{}.dds", texture.replace('.', "__"))
}

/// A constant colour slot's value as Elite stores it: 16 hex digits, two RGBA8.
pub(super) fn colour(hex: &str, f: &str) -> Result<[[u8; 4]; 2], String> {
    let byte = |k: usize| hex.get(2 * k..2 * k + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(|| format!("{f}: colour {hex} is not 8 bytes of hex"));
    if hex.len() != 16 {
        return Err(format!("{f}: colour {hex} is not 8 bytes of hex"));
    }
    Ok([[byte(0)?, byte(1)?, byte(2)?, byte(3)?], [byte(4)?, byte(5)?, byte(6)?, byte(7)?]])
}

/// A material record's attributes (`attrs`: [name, type, values] each).
pub(super) fn attrs(src: &Value, f: &str) -> Result<Vec<Attr>, String> {
    let mut attrs = Vec::new();
    for a in src.get("attrs").and_then(|v| v.as_array()).into_iter().flatten() {
        let a = a.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{f}: an attribute is not [name, type, values]"))?;
        let name = a[0].as_str().ok_or_else(|| format!("{f}: an attribute's name is not text"))?.to_string();
        let kind = a[1].as_u64().filter(|&k| k <= 6).ok_or_else(|| format!("{f}: attribute {name}'s type is not 0-6"))? as u8;
        let values = a[2].as_array().into_iter().flatten().map(|x| x.as_f64().ok_or_else(|| format!("{f}: attribute {name} has a value that isn't a number"))).collect::<Result<_, _>>()?;
        attrs.push(Attr { name, kind, values });
    }
    Ok(attrs)
}

/// The materials table for these material names (any case), with the materials their fallbacks name.
fn materials(dec: &Path, wanted: &BTreeSet<String>) -> Result<(Materials, BTreeSet<String>), String> {
    let all = read_json(&dec.join("materials.json"))?;
    let textures = read_json(&dec.join("textures.json"))?;
    let mut table = BTreeMap::new();
    let mut ovls = BTreeSet::new();
    let mut todo: Vec<String> = wanted.iter().map(|n| n.to_lowercase()).collect();
    while let Some(name) = todo.pop() {
        if table.contains_key(&name) {
            continue;
        }
        let f = format!("materials.json {name}");
        let src = all.get(&name).ok_or_else(|| format!("materials.json: no material {name}"))?;
        let opt_text = |k: &str| src.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let mut slots = Vec::new();
        for t in src.get("textures").and_then(|v| v.as_array()).into_iter().flatten() {
            let t = t.as_array().filter(|t| t.len() >= 3).ok_or_else(|| format!("{f}: a texture slot is not a list"))?;
            let item = |k: usize| t[k].as_str().ok_or_else(|| format!("{f}: a texture slot's item {k} is not text"));
            let slot = item(0)?.to_string();
            let value = match item(1)? {
                "tex" => {
                    let texture = item(2)?.to_string();
                    let tf = format!("textures.json {texture}");
                    let info = textures.get(&texture).ok_or_else(|| format!("{f}: texture {texture} is not in textures.json"))?;
                    SlotValue::Tex {
                        file: format!("tex/{}", texture_file(&texture)),
                        srgb: at(info, "srgb", &tf)?.as_bool().ok_or_else(|| format!("{tf}: srgb is not true or false"))?,
                        format: text(info, "format", &tf)?.to_string(),
                        texture,
                    }
                }
                "dt7" => SlotValue::Colour { colour: colour(item(2)?, &f)? },
                k => return Err(format!("{f}: slot {slot} of unknown kind {k}")),
            };
            slots.push(TextureSlot { slot, value });
        }
        let attrs = attrs(src, &f)?;
        let fallback_to = opt_text("fallback_to");
        if let Some(t) = &fallback_to {
            todo.push(t.clone());
        }
        ovls.extend(opt_text("ovl"));
        table.insert(name, Material { shader: opt_text("shader"), textures: slots, attrs, fallback_to, why: opt_text("why") });
    }
    Ok((Materials { materials: table }, ovls))
}

/// A shipped texture's record in the export: `textures.json`'s entry, or for a paint job's mask its `paintjobs.json`
/// texture block (matched by file).
fn texture_record<'a>(textures: &'a Value, paintjobs: &'a Value, file: &str) -> Option<&'a Value> {
    let name = file.trim_end_matches(".dds").replacen("__", ".", 1);
    textures.get(&name).or_else(|| {
        let path = format!("textures/{file}");
        paintjobs.as_object()?.values().filter_map(|j| j.get("texture")).find(|t| t.get("file").and_then(Value::as_str) == Some(path.as_str()))
    })
}

/// Where a texture's levels came from in Elite's files (its record's OVL and streams).
pub(super) fn texture_sources(record: &Value) -> Vec<String> {
    let mut out: Vec<String> = record.get("ovl").and_then(|v| v.as_str()).map(|o| format!("Elite file {o} (its OVL)")).into_iter().collect();
    for s in record.get("sources").and_then(|v| v.as_array()).into_iter().flatten() {
        if let Some(st) = s.get("stream").and_then(|v| v.as_str()) {
            out.push(format!("Elite file {st}"));
        }
    }
    out
}

/// The export's name of a DDS format.
fn format_name(f: DdsFormat) -> &'static str {
    match f {
        DdsFormat::Bc1 => "BC1",
        DdsFormat::Bc2 => "BC2",
        DdsFormat::Bc3 => "BC3",
        DdsFormat::Bc4 => "BC4",
        DdsFormat::Bc5 => "BC5",
        DdsFormat::Bc7 => "BC7",
        DdsFormat::Bgra8 => "BGRA8",
        _ => "other",
    }
}

/// A texture's bytes (`path`, named `rel` in errors), checked against its export record: format, width, height, levels.
pub(super) fn checked_texture(path: &Path, rel: &str, record: &Value) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{rel}: {e}"))?;
    let info = se_core::dds::parse_dds(&bytes).map_err(|e| format!("{rel}: {e}"))?;
    let want = (text(record, "format", rel)?, [count(record, "width", rel)?, count(record, "height", rel)?, count(record, "mips", rel)?]);
    let got = (format_name(info.format), [info.width as usize, info.height as usize, info.mips as usize]);
    if got != want {
        return Err(format!("{rel}: {got:?} (format, width, height, levels), its record says {want:?}"));
    }
    Ok(bytes)
}

/// A texture to ship: its bytes from `decoded/textures`, checked against its record (format, size, levels).
struct Tex {
    file: String,
    bytes: Vec<u8>,
    note: String,
    sources: Vec<String>,
}

/// Every file `convert` writes, worked out from the export alone.
struct Plan {
    models: Vec<(String, Edm, Expect)>,
    table: Materials,
    ovls: BTreeSet<String>,
    textures: Vec<Tex>,
}

fn plan(dec: &Path) -> Result<Plan, String> {
    let names = model_names(dec)?;
    let mut models = Vec::new();
    let mut used = BTreeSet::new();
    for name in &names {
        let (edm, expect) = model(dec, name)?;
        used.extend(edm.materials.iter().map(|m| m.name.clone()));
        models.push((name.clone(), edm, expect));
    }
    let (table, ovls) = materials(dec, &used)?;
    let textures_json = read_json(&dec.join("textures.json"))?;
    let paintjobs = read_json(&dec.join("paintjobs.json"))?;
    let mut wanted: BTreeMap<String, String> = BTreeMap::new();
    for m in table.materials.values() {
        for s in &m.textures {
            if let SlotValue::Tex { texture, format, srgb, .. } = &s.value {
                let note = format!("Elite's texture {texture} ({format}{}), every level byte for byte.", if *srgb { ", sampled as sRGB" } else { "" });
                wanted.insert(texture_file(texture), note);
            }
        }
    }
    for (file, what) in TEXTURES_EXTRA {
        wanted.insert(file.to_string(), format!("{what}, every level byte for byte."));
    }
    let mut textures = Vec::new();
    for (file, note) in wanted {
        let rel = format!("decoded/textures/{file}");
        let record = texture_record(&textures_json, &paintjobs, &file).ok_or_else(|| format!("{rel}: neither textures.json nor paintjobs.json lists it"))?;
        let bytes = checked_texture(&dec.join("textures").join(&file), &rel, record)?;
        let mut sources = vec![rel];
        sources.extend(texture_sources(record));
        textures.push(Tex { file, bytes, note, sources });
    }
    for m in table.materials.values() {
        for s in &m.textures {
            if let SlotValue::Tex { file, .. } = &s.value {
                if !textures.iter().any(|t| format!("tex/{}", t.file) == *file) {
                    return Err(format!("{file} is named by a material but not shipped"));
                }
            }
        }
    }
    Ok(Plan { models, table, ovls, textures })
}

/// The written materials table reads back as planned and holds every material the models name.
fn check_materials(ctx: &Ctx, p: &Plan) -> Result<(), String> {
    let back = parse_materials(&ctx.read("materials.json")?).map_err(|e| e.to_string())?;
    if back != p.table {
        return Err("materials.json differs from the export's".into());
    }
    let used = p.models.iter().flat_map(|(_, e, _)| e.materials.iter());
    if let Some(m) = used.into_iter().find(|m| back.get(&m.name).is_none()) {
        return Err(format!("materials.json lacks {}", m.name));
    }
    Ok(())
}

fn check_texture(ctx: &Ctx, t: &Tex) -> Result<(), String> {
    let rel = format!("tex/{}", t.file);
    if ctx.read(&rel)? != t.bytes {
        return Err(format!("{rel} differs from {}", t.sources[0]));
    }
    Ok(())
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    for dir in ["models", "tex"] {
        match std::fs::remove_dir_all(ctx.out.join(dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{dir}: {e}")),
            _ => {}
        }
    }
    let p = plan(&ctx.export)?;
    let (mut triangles, mut vertices) = (0usize, 0usize);
    for (name, edm, expect) in &p.models {
        let rel = format!("models/{name}.edm");
        ctx.write(&rel, &write_edm(edm))?;
        check_model(ctx, &rel, edm, expect)?;
        let t: usize = edm.draws.iter().map(|d| d.indices.len() / 3).sum();
        let v: usize = edm.draws.iter().map(|d| d.vertices.len()).sum();
        triangles += t;
        vertices += v;
        shipped.add(
            &rel,
            &expect.sources,
            "A",
            &format!(
                "Elite's model {name}, LOD0 ({} draws, {t} triangles, {v} vertices): vertex and index bytes unchanged, {} bones \
                 (inverse binds in full, as float32), {} materials by Elite's name.",
                edm.draws.len(),
                edm.parts.len(),
                edm.materials.len()
            ),
        );
    }
    ctx.write("materials.json", &write_materials(&p.table))?;
    check_materials(ctx, &p)?;
    let fallbacks: Vec<&String> = p.table.materials.iter().filter(|(_, m)| m.fallback_to.is_some()).map(|(n, _)| n).collect();
    shipped.add(
        "materials.json",
        &p.ovls.iter().map(|o| format!("Elite file {o}")).chain(["decoded/materials.json".to_string()]).collect::<Vec<_>>(),
        "A",
        &format!(
            "Every material the models name ({}): Elite's shader, texture slots and attribute values as its material files hold \
             them. No file of Elite's install defines {fallbacks:?}: the engine draws them with its default material, null.",
            p.table.materials.len()
        ),
    );
    let mut tex_bytes = 0;
    for t in &p.textures {
        let rel = format!("tex/{}", t.file);
        ctx.write(&rel, &t.bytes)?;
        check_texture(ctx, t)?;
        tex_bytes += t.bytes.len();
        shipped.add(&rel, &t.sources, "A", &t.note);
    }
    shipped.note(NOT_SHIPPED);
    println!(
        "models: {} shipped, {triangles} triangles, {vertices} vertices; {} materials; {} textures, {:.1} MB",
        p.models.len(),
        p.table.materials.len(),
        p.textures.len(),
        tex_bytes as f64 / 1048576.0
    );
    println!("models: {}", p.models.iter().map(|m| m.0.as_str()).collect::<Vec<_>>().join(" "));
    Ok(())
}

/// The folder's models, materials and textures are what a fresh plan from the export makes, and nothing else is in
/// `models/` or `tex/`. Writes nothing.
pub(super) fn verify(ctx: &Ctx) -> Result<(), String> {
    let p = plan(&ctx.export)?;
    for (name, edm, expect) in &p.models {
        check_model(ctx, &format!("models/{name}.edm"), edm, expect)?;
    }
    check_materials(ctx, &p)?;
    for t in &p.textures {
        check_texture(ctx, t)?;
    }
    let listed = |dir: &str| -> Result<BTreeSet<String>, String> {
        Ok(std::fs::read_dir(ctx.out.join(dir)).map_err(|e| format!("{dir}: {e}"))?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
    };
    let models: BTreeSet<String> = p.models.iter().map(|m| format!("{}.edm", m.0)).collect();
    let tex: BTreeSet<String> = p.textures.iter().map(|t| t.file.clone()).collect();
    if listed("models")? != models || listed("tex")? != tex {
        return Err("models/ or tex/ holds files the export doesn't make, or lacks some".into());
    }
    println!("models: {} models, {} materials, {} textures match the export", p.models.len(), p.table.materials.len(), p.textures.len());
    Ok(())
}
