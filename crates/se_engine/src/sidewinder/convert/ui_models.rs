//! The ship UI's panel models, their materials and textures (UI2; ARCHITECTURE.md A10), from the UI export's `hud3d/`
//! (`models/NAME.json` + `.buf0.bin` in the shape of the ship's decoded models, `materials.json` and `textures.json` in
//! the shape of the ship's): `ui_layout.rs` asks for what the layout names. The parsing is `models.rs`'s, shared.
//! - `ui/models/NAME.edm` (`.edm` version 2, `VertexDecode::Elite`): LOD0 as `models::model` makes the ship's: Elite's
//!   40-byte vertices and indices byte for byte (uv0 exactly), the model's bones, each draw's material by Elite's name.
//! - `ui/tex/NAME.dds`: the textures the materials name, Elite's levels byte for byte. In `ui/` and not beside the ship's
//!   in `tex/`: `models.rs` empties `tex/` and its check wants exactly the ship's textures there.
//! - The materials as `GuiMaterial` (in `layout.json`): Elite's shader, the family the notes give it, every texture slot
//!   and attribute as its material file holds them. A texture no export holds whole is `Missing` (listed in `SOURCES.txt`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use se_core::elite_asset::{parse_edm, write_edm, Edm};
use se_core::elite_ui_layout::{GuiMaterial, GuiSlot, GuiSlotValue, MaterialTag};

use super::models::{self, at, list, text, BOUNDS_M};
use super::Planned;

/// A panel model's LOD0 as an `.edm` (`models::model` on `hud3d/`), checked against the bounds the export states, and
/// its planned file.
pub(super) fn model(hud3d: &Path, stem: &str) -> Result<(Edm, Planned), String> {
    let (edm, expect) = models::model(hud3d, stem)?;
    let (lo, hi) = expect.model_bounds;
    for v in edm.draws.iter().flat_map(|d| &d.vertices) {
        let p = v.decode().position;
        if (0..3).any(|a| p[a] < lo[a] - BOUNDS_M || p[a] > hi[a] + BOUNDS_M) {
            return Err(format!("hud3d/models/{stem}.json: a decoded vertex {p:?} leaves the model's bounds {lo:?}..{hi:?}"));
        }
    }
    let bytes = write_edm(&edm);
    if parse_edm(&bytes).map_err(|e| format!("{stem}.edm: {e}"))? != edm {
        return Err(format!("{stem}.edm reads back different"));
    }
    let (t, v): (usize, usize) = (edm.draws.iter().map(|d| d.indices.len() / 3).sum(), edm.draws.iter().map(|d| d.vertices.len()).sum());
    let names: Vec<&str> = edm.materials.iter().map(|m| m.name.as_str()).collect();
    let planned = Planned {
        path: format!("ui/models/{stem}.edm"),
        note: format!(
            "Elite's UI panel model {stem}, LOD0 ({} draws, {t} triangles, {v} vertices): vertex and index bytes unchanged (uv0 exactly), \
             {} bones, materials {} by Elite's name.",
            edm.draws.len(),
            edm.parts.len(),
            names.join(", ")
        ),
        bytes,
        // `models::model` names its files from the folder above the one it reads: here that is `decoded/hud3d`.
        sources: expect.sources.iter().map(|s| s.strip_prefix("decoded/").map_or_else(|| s.clone(), |r| format!("decoded/hud3d/{r}"))).collect(),
        grade: "A",
    };
    Ok((edm, planned))
}

/// A material's family by its shader (`ui_rendering.md` 2.5 and 3.2): an error for a shader the notes don't place.
fn tag(shader: &str) -> Result<MaterialTag, String> {
    Ok(if shader.contains("GuiLighting") {
        MaterialTag::Foreground
    } else if shader.contains("GuiBackground") {
        MaterialTag::Background
    } else if shader.contains("GuiBrightnessScale") && shader.contains("UVScaleOffset") {
        MaterialTag::Brightness
    } else if shader.contains("GuiParallax") {
        MaterialTag::Screen
    } else if shader == "EdgeDetectionForUI" {
        MaterialTag::Edge
    } else if shader.contains("Antialias") {
        MaterialTag::Sight
    } else {
        return Err(format!("shader {shader} is none of the notes' UI families"));
    })
}

/// The materials and the textures they need.
#[derive(Default)]
pub(super) struct Materials {
    pub table: BTreeMap<String, GuiMaterial>,
    /// The textures to ship, by file name in `ui/tex/`.
    pub textures: BTreeMap<String, Planned>,
    /// Textures named by a material that no export holds whole: (texture, the material naming it, why).
    pub missing: BTreeSet<(String, String, String)>,
}

impl Materials {
    /// Adds a material from its record in the export's shape (`shader`, `textures` as [slot, kind, value, ovl...],
    /// `attrs` as [name, type, values]); a second record of the same name must be the same.
    pub fn add(&mut self, hud3d: &Path, textures: &Value, name: &str, rec: &Value, from: &str) -> Result<(), String> {
        let key = name.to_lowercase();
        let f = format!("{from} {name}");
        let shader = text(rec, "shader", &f)?.to_string();
        let mut slots = Vec::new();
        for t in list(rec, "textures", &f)? {
            let item = |k: usize| t.get(k).and_then(Value::as_str).ok_or_else(|| format!("{f}: a texture slot's item {k} is not text"));
            let (slot, kind, value) = (item(0)?.to_string(), item(1)?, item(2)?);
            let value = match kind {
                "tex" => {
                    let rec = textures.get(value);
                    // A texture the export lists with a problem (its levels cut short) is not Elite's whole bytes.
                    let problem = rec.and_then(|r| r.get("problem")).and_then(Value::as_str);
                    match (rec, problem) {
                        (Some(rec), None) => self.texture(hud3d, value, rec)?,
                        (_, why) => {
                            let why = why.map_or("in no export".to_string(), |p| format!("the UI export's copy is damaged: {p}"));
                            self.missing.insert((value.to_string(), key.clone(), why));
                            GuiSlotValue::Missing { texture: value.to_string() }
                        }
                    }
                }
                "dt7" => GuiSlotValue::Colour { colour: models::colour(value, &f)? },
                k => return Err(format!("{f}: slot {slot} of unknown kind {k}")),
            };
            slots.push(GuiSlot { slot, value });
        }
        let m = GuiMaterial { tag: tag(&shader)?, shader, textures: slots, attrs: models::attrs(rec, &f)? };
        match self.table.get(&key) {
            Some(had) if *had != m => Err(format!("{f}: material {key} is defined twice, differently")),
            _ => {
                self.table.insert(key, m);
                Ok(())
            }
        }
    }

    /// A texture `hud3d/textures.json` lists: its file planned (checked against its record), its slot value.
    fn texture(&mut self, hud3d: &Path, name: &str, rec: &Value) -> Result<GuiSlotValue, String> {
        let file = models::texture_file(name);
        let tf = format!("hud3d/textures.json {name}");
        let format = text(rec, "format", &tf)?.to_string();
        let srgb = at(rec, "srgb", &tf)?.as_bool().ok_or_else(|| format!("{tf}: srgb is not true or false"))?;
        if !self.textures.contains_key(&file) {
            let rel = format!("decoded/hud3d/textures/{file}");
            let bytes = models::checked_texture(&hud3d.join("textures").join(&file), &rel, rec)?;
            let mut sources = vec![rel];
            sources.extend(models::texture_sources(rec));
            let note = format!("Elite's texture {name} ({format}{}), every level byte for byte.", if srgb { ", sampled as sRGB" } else { "" });
            self.textures.insert(file.clone(), Planned { path: format!("ui/tex/{file}"), bytes, sources, grade: "A", note });
        }
        Ok(GuiSlotValue::Tex { texture: name.to_string(), file: format!("ui/tex/{file}"), srgb, format })
    }
}
