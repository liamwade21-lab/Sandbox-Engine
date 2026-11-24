//! The UI's files, read once on a worker thread (ARCHITECTURE.md A10, "Three stages"): `ui/layout.json`
//! (`se_core::elite_ui_layout`), the fonts and their map, the string table, the movies' bytes, the `gui_*` models (decoded
//! as the ship's are: `se_core::elite::Vertex::decode`, then `to_mod_space`) and the textures their materials name. Read
//! through `mods::mod_file` from the mod's folder. Every one of these files is a `Need` of the mod (`mod.cfg`), so one that
//! is missing or damaged already greys the mod's row at its first enable, as any of the mod's files does (the mod's rule).
//! Only a failure past that check (a file that passes `mods::check_inside` but not this reader) is an `Err` naming it, and
//! then the UI stays off and the ship works without it.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use elite_gfx::Font;
use elite_ui::Text;
use se_core::elite::to_mod_space;
use se_core::elite_asset::{parse_edm, VertexDecode};
use se_core::elite_ui_layout::{parse_fontmap, parse_layout, GuiSlotValue, Layout};

use super::host::{ItemDatabase, UiHost};

const LAYOUT_FILE: &str = "ui/layout.json";
const FONTMAP_FILE: &str = "ui/fonts/fontmap.json";
const STRINGS_FILE: &str = "ui/strings/en.json";
const FOCUS_FILE: &str = "ui/focus.json";

/// Whether a texture slot's sampler repeats (the cloudiness and scanlines are tiled across the panel by the vertex stage's
/// planar coordinates: `pCloudynessTile`, `pScanlineTile`) or clamps (a plate's alpha shape, cut by its uv0).
pub fn slot_repeats(slot: &str) -> bool {
    matches!(slot, "pCloudynessTexture" | "pScanlinesTexture")
}

/// A texture's key: its file, whether it is sampled as sRGB, whether its sampler repeats.
pub type TexKey = (String, bool, bool);

/// One draw of a model: its material (a key of `Layout::materials`) and its mesh.
pub struct ModelDraw {
    pub material: String,
    pub mesh: Mesh,
}

/// Everything the UI reads, ready for the main thread.
pub struct UiFiles {
    pub layout: Layout,
    pub fonts: Vec<Font>,
    pub host: Arc<UiHost>,
    /// Each movie's converted SWF, by its name (`cockpitgaugepanel`).
    pub movies: HashMap<String, Vec<u8>>,
    /// Each model's draws, by its stem (`gui_left_panel`).
    pub models: HashMap<String, Vec<ModelDraw>>,
    pub textures: HashMap<TexKey, Image>,
    /// What the cockpit panels' game side needs (`focus.rs`, `panels.rs`): a resource once the UI is made.
    pub panels: PanelFiles,
}

/// The cockpit panels' files and the host's item database they fill.
#[derive(Resource)]
pub struct PanelFiles {
    /// The cockpit's focus machine (`ui/focus.json`, `elite_ui::focus::Focus::from_json`).
    pub focus: Vec<u8>,
    /// The modules the host answers `GetDatabaseItemData` from (`panels.rs` writes them).
    pub items: ItemDatabase,
}

/// A file of the mod's folder (`mods::mod_file`).
pub(super) fn read(rel: &str) -> Result<Vec<u8>, String> {
    let path = crate::mods::mod_file(super::super::CODE, rel).ok_or_else(|| format!("{rel}: not in the mod's folder"))?;
    std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))
}

/// Reads every UI file (on a worker thread: about 20 MB, the movies most of it).
pub fn load() -> Result<UiFiles, String> {
    let layout = parse_layout(&read(LAYOUT_FILE)?).map_err(|e| e.to_string())?;
    let (fonts, map) = fonts()?;
    let mut text = Text::from_json(&read(STRINGS_FILE)?).map_err(|e| format!("{STRINGS_FILE}: {e}"))?;
    super::screens::bind_keys(&mut text, &read(super::screens::KEYS_FILE)?)?;
    let mut movies = HashMap::new();
    for m in &layout.movies {
        movies.insert(m.name.clone(), read(&format!("ui/movies/{}.swf", m.name))?);
    }
    let mut models = HashMap::new();
    for (name, model) in &layout.models {
        models.insert(name.clone(), decode_model(&model.file)?);
    }
    let mut textures = HashMap::new();
    for mat in layout.materials.values() {
        for slot in &mat.textures {
            if let GuiSlotValue::Tex { file, srgb, .. } = &slot.value {
                let key = (file.clone(), *srgb, slot_repeats(&slot.slot));
                if !textures.contains_key(&key) {
                    let image = texture(file, *srgb, key.2)?;
                    textures.insert(key, image);
                }
            }
        }
    }
    let items = ItemDatabase::default();
    let panels = PanelFiles { focus: read(FOCUS_FILE)?, items: items.clone() };
    Ok(UiFiles { layout, fonts, host: Arc::new(UiHost::new(text, map, items)), movies, models, textures, panels })
}

/// The font files under the library's names, and the font map's names each with its plain and bold file (as the runtime's
/// proofs register them: `crates/elite-gfx/tests/hud_feed.rs`); and the map's name -> face for the host.
fn fonts() -> Result<(Vec<Font>, HashMap<String, String>), String> {
    let map = parse_fontmap(&read(FONTMAP_FILE)?).map_err(|e| e.to_string())?;
    let mut data: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
    for f in &map.fonts {
        data.insert(f.file.clone(), Arc::new(read(&format!("ui/fonts/{}", f.file))?));
    }
    let file = |name: &str| data.get(name).cloned().ok_or_else(|| format!("{FONTMAP_FILE}: no file {name}"));
    let mut fonts: Vec<Font> = map.fonts.iter().map(|f| Ok(Font { name: f.name.clone(), bold: f.bold, italic: false, data: file(&f.file)? })).collect::<Result<_, String>>()?;
    for m in &map.map {
        fonts.push(Font { name: m.name.clone(), bold: false, italic: false, data: file(&m.regular)? });
        fonts.push(Font { name: m.name.clone(), bold: true, italic: false, data: file(&m.heavy)? });
    }
    let faces = map.map.iter().map(|m| (m.name.clone(), m.font.clone())).collect();
    Ok((fonts, faces))
}

/// A `gui_*` model's draws in the mod's axes: position, normal, tangent (its sign in w), uv0. Its material's name is the
/// layout's key (lower case).
fn decode_model(file: &str) -> Result<Vec<ModelDraw>, String> {
    let edm = parse_edm(&read(file)?).map_err(|e| format!("{file}: {e}"))?;
    if edm.vertex_decode != VertexDecode::Elite {
        return Err(format!("{file}: not one of Elite's models"));
    }
    let mut draws = Vec::new();
    for d in &edm.draws {
        let material = edm.materials.get(d.material as usize).ok_or_else(|| format!("{file}: a draw names material {}, which it hasn't", d.material))?.name.to_lowercase();
        let mut vertices: Vec<_> = d.vertices.iter().map(|v| v.decode()).collect();
        let mut indices = d.indices.clone();
        if indices.len() % 3 != 0 || indices.iter().any(|&i| i as usize >= vertices.len()) {
            return Err(format!("{file}: a draw of {material} has a broken triangle list"));
        }
        to_mod_space(&mut vertices, &mut indices);
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices.iter().map(|v| v.position).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vertices.iter().map(|v| v.normal).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vertices.iter().map(|v| [v.tangent[0], v.tangent[1], v.tangent[2], v.tangent_sign]).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vertices.iter().map(|v| v.uv[0]).collect::<Vec<_>>());
        mesh.insert_indices(Indices::U16(indices));
        draws.push(ModelDraw { material, mesh });
    }
    Ok(draws)
}

/// A shipped texture with its sampler: linear, repeating or clamped.
pub(super) fn texture(file: &str, srgb: bool, repeat: bool) -> Result<Image, String> {
    let bytes = read(file)?;
    let info = se_core::dds::parse_dds(&bytes).map_err(|e| format!("{file}: {e}"))?;
    let mut image = crate::mr_assets::dds_image(&bytes, &info, srgb).map_err(|e| format!("{file}: {e}"))?;
    let mode = if repeat { ImageAddressMode::Repeat } else { ImageAddressMode::ClampToEdge };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: mode,
        address_mode_v: mode,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    Ok(image)
}
