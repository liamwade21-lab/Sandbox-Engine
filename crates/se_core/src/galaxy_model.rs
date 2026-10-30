//! SpaceEngine galaxy *model* generation (CPU side).
//!
//! SpaceEngine 0.991 renders catalogue galaxies of Hubble type S0..Irr from a cloud of
//! soft sprites ("particles") that `seSpriteModelMaker` builds from a `.pst` model
//! description plus a few images.  This module re-implements that generator:
//!
//! * [`parse_pst`] reads `GalaxyModel "<name>" { ... }` blocks
//!   (`data/models/galaxies/Galaxies-sprite.pak`: `Default.pst`, `Kexitt.pst`).
//! * [`hubble_type_index`], [`ModelCatalog::select`], [`resolve_texture`],
//!   [`texture_for_galaxy`] implement which model / texture a galaxy gets.
//! * [`build_galaxy_model`] / [`generate_galaxy_sprites`] turn a model plus decoded
//!   RGBA8 images into [`GalaxySprite`]s.  The algorithm, constants, loop order and
//!   random-number sequence follow `FUN_1402d85c0` (`createGalaxy`), `FUN_1402d4c70`
//!   (`createCubeMap`), `FUN_1402d3f10` (`createDiffuse`), `FUN_1402da540`
//!   (`createModelCylindrical`) and `FUN_1402d9a10` (`createModelSpherical`).
//!
//! The full write-up, with Ghidra addresses and a verified/inferred marking for every
//! rule, is in `docs/formats/galaxy_model.md`.
//!
//! ## Coordinates
//!
//! Sprite positions are in *model space*: the disk lies in the XY plane and spans
//! `[-1, 1]` on X and Y (1 = disk radius), the disk normal is the Z axis.  Image column
//! maps to +X and image row (counted from the top) maps to +Y.  The renderer scales the
//! model by [`GalaxyModelDef::model_scale`] (`Radius.x`, `Radius.x`, `Radius.x * SpriteOblZ`).
//! [`se_to_port_model`] converts to the port's convention (disk normal = model -Z).
//!
//! The module only depends on `std`, `glam`, `log` and `thiserror`; images are passed in
//! as plain RGBA8 data ([`RgbaImage`]).  RGB must be loaded as *straight* (not
//! premultiplied) colour: the textures keep full-bright RGB under `alpha == 0`.

use std::collections::HashMap;

use glam::Vec3;
use log::{debug, warn};
use thiserror::Error;

// ---------------------------------------------------------------------------------------
// .pst model description
// ---------------------------------------------------------------------------------------

/// `Method` of a `.pst` model (low nibble of the model flags in the exe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelMethod {
    /// `"SpiralGalaxy"` (0): emission + absorption maps from front/side images, plus bulge.
    SpiralGalaxy,
    /// `"EllipticGalaxy"` (1): bulge only (no shipped model uses it).
    EllipticGalaxy,
    /// `"Cylindrical"` (2): generic image model (not used by shipped galaxies).
    Cylindrical,
    /// `"Spherical"` (3): the exe's default when `Method` is missing.
    #[default]
    Spherical,
    /// `"Flat"` (4).
    Flat,
    /// `"CubeMap"` (5): six axis images multiplied together.
    CubeMap,
    /// `"Diffuse"` (6): procedural noise cloud (irregular galaxies).
    Diffuse,
    /// `"Planetary"` (7): nebula method.
    Planetary,
    /// `"SNR"` (8): nebula method.
    Snr,
}

impl ModelMethod {
    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "SpiralGalaxy" => Self::SpiralGalaxy,
            "EllipticGalaxy" => Self::EllipticGalaxy,
            "Cylindrical" => Self::Cylindrical,
            "Spherical" => Self::Spherical,
            "Flat" => Self::Flat,
            "CubeMap" => Self::CubeMap,
            "Diffuse" => Self::Diffuse,
            "Planetary" => Self::Planetary,
            "SNR" => Self::Snr,
            _ => return None,
        })
    }
}

/// Parameters of the fractal noise used by `Diffuse` models (`emNoise*` / `absNoise*`).
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseParams {
    /// `*NoiseFreq`: scale applied to the voxel coordinate before sampling.
    pub freq: f32,
    /// `*NoiseDistort`: strength of the domain warp.
    pub distort: f32,
    /// `*NoiseH`: octave amplitude falloff exponent (`amp_i = lacunarity^(-H*i)`).
    pub h: f32,
    /// `*NoiseLacunarity`: frequency multiplier per octave.
    pub lacunarity: f32,
    /// `*NoiseOffset`: parsed by the exe but not read by any traced noise routine.
    pub offset: f32,
    /// `*NoiseOctaves`.
    pub octaves: i32,
    /// `*NoiseRandomize`: offset added to the noise coordinate (a per-model seed).
    pub randomize: [f32; 3],
}

impl Default for NoiseParams {
    fn default() -> Self {
        // Defaults from the particle-parameter constructor FUN_140049670.
        Self {
            freq: 1.0,
            distort: 0.7,
            h: 0.6,
            lacunarity: 2.218_282,
            offset: 0.8,
            octaves: 8,
            randomize: [0.0; 3],
        }
    }
}

/// One particle population (emission `em*`, absorption `abs*`, or bulge `b*` keys).
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleLayer {
    /// `*ParticleColor`: tint; voxel colour is normalised so its max channel equals this.
    pub color: [f32; 3],
    /// `emParticleColor2` (nebula models; unused by galaxies).
    pub color2: [f32; 3],
    /// `*DetailR`: grid resolution multiplier in the disk plane.
    pub detail_r: f32,
    /// `*DetailZ`: grid resolution multiplier along Z (also scales the z extent).
    pub detail_z: f32,
    /// `*ParticleDispR`: random jitter in X/Y, in voxel sizes.
    pub disp_r: f32,
    /// `*ParticleDispZ`: random jitter in Z, in voxel sizes.
    pub disp_z: f32,
    /// `*ParticleSizeCenter`: sprite size at the centre (in voxel sizes).
    pub size_center: f32,
    /// `*ParticleSizeEdge`: sprite size at the edge.
    pub size_edge: f32,
    /// `*ParticleMinBrightCenter`: voxel is dropped if `r+g+b` is below this.
    pub min_bright_center: f32,
    /// `*ParticleMinBrightEdge`.
    pub min_bright_edge: f32,
    /// `*ParticleBrightnessCenter`.
    pub brightness_center: f32,
    /// `*ParticleBrightnessEdge`.
    pub brightness_edge: f32,
    /// `*Noise*` keys (only used by `Diffuse`).
    pub noise: NoiseParams,
}

impl Default for ParticleLayer {
    fn default() -> Self {
        Self {
            color: [1.0; 3],
            color2: [1.0; 3],
            detail_r: 1.0,
            detail_z: 1.0,
            disp_r: 0.0,
            disp_z: 0.0,
            size_center: 1.0,
            size_edge: 1.0,
            min_bright_center: 1.0,
            min_bright_edge: 1.0,
            brightness_center: 1.0,
            brightness_edge: 1.0,
            noise: NoiseParams::default(),
        }
    }
}

/// A parsed `GalaxyModel "<name>" { ... }` block.
#[derive(Debug, Clone, PartialEq)]
pub struct GalaxyModelDef {
    /// Block name; also the seed source of the particle RNG ([`model_seed`]).
    pub name: String,
    /// `UseForObject`: catalogue names this model is pinned to (`"A/B"` lists several).
    pub use_for_object: Vec<String>,
    /// `UseForType`: Hubble type whose random pool this model joins (e.g. `"SBc"`).
    pub use_for_type: Option<String>,
    /// `GroupModel` (unused).
    pub group_model: String,
    /// `Method`.
    pub method: ModelMethod,
    /// `Position` (model offset).
    pub position: [f32; 3],
    /// `Radius`: (disk radius, bulge radius, bulge thickness).
    pub radius: [f32; 3],
    /// `BBoxRes`: resolution of the culling grid over `[-1,1]^3` (seSpriteVBO::BuildTree).
    pub bbox_res: [i32; 3],
    /// `Yaw`, `Pitch`, `Roll`.
    pub yaw_pitch_roll: [f32; 3],
    /// `SpriteOblZ` (default 1): z oversampling; also multiplies the model z scale.
    pub sprite_obl_z: f32,
    /// `floatLOD`.
    pub float_lod: bool,
    /// `LODbase` (exe default 100).
    pub lod_base: f32,
    /// `useFBO`.
    pub use_fbo: bool,
    /// `FrontImage`: pattern such as `NGC4038_small.*` (CPU generator input).
    pub front_image: String,
    /// `SideImage`: pattern such as `Side1.*`.
    pub side_image: String,
    /// `CubePosXImage`, `CubeNegXImage`, `CubePosYImage`, `CubeNegYImage`, `CubePosZImage`,
    /// `CubeNegZImage` in that order.
    pub cube_images: [String; 6],
    /// `FrontTexture`: full-resolution RGBA texture (GPU billboard).
    pub front_texture: String,
    /// `SysTexture`: RGB jpg (galaxy "system" model texture).
    pub sys_texture: String,
    /// `Brightness` (default 1).
    pub brightness: f32,
    /// `Transparency` (default 1).
    pub transparency: f32,
    /// `BulgeRadius`: per-axis exponent scale of the bulge gaussian (default 1,1,1).
    pub bulge_radius: [f32; 3],
    /// `clipRadius`: precomputed bounding radius stored in the file (nebula models), if present.
    pub clip_radius: Option<f32>,
    /// `sumColor`: precomputed mean colour, bytes 0..255 (nebula models), if present.
    pub sum_color: Option<[f32; 3]>,
    /// Emission layer (`em*` keys).
    pub em: ParticleLayer,
    /// Absorption (dust) layer (`abs*` keys).
    pub abs: ParticleLayer,
    /// Bulge layer (`b*` keys).
    pub bulge: ParticleLayer,
    /// Keys the parser did not recognise (e.g. nebula-only keys), kept verbatim.
    pub extra: Vec<(String, String)>,
}

impl Default for GalaxyModelDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            use_for_object: Vec::new(),
            use_for_type: None,
            group_model: String::new(),
            method: ModelMethod::default(),
            position: [0.0; 3],
            radius: [1.0; 3],
            bbox_res: [8, 8, 8],
            yaw_pitch_roll: [0.0; 3],
            sprite_obl_z: 1.0,
            float_lod: false,
            lod_base: 100.0,
            use_fbo: false,
            front_image: String::new(),
            side_image: String::new(),
            cube_images: Default::default(),
            front_texture: String::new(),
            sys_texture: String::new(),
            brightness: 1.0,
            transparency: 1.0,
            bulge_radius: [1.0; 3],
            clip_radius: None,
            sum_color: None,
            em: ParticleLayer::default(),
            abs: ParticleLayer::default(),
            // FUN_1402dbda0: bulge colour defaults to (1.0, 0.8, 0.5).
            bulge: ParticleLayer { color: [1.0, 0.8, 0.5], ..ParticleLayer::default() },
            extra: Vec::new(),
        }
    }
}

impl GalaxyModelDef {
    /// Model matrix scale applied by the renderer (`FUN_140299130`): `(R, R, R * SpriteOblZ)`
    /// with `R = Radius.x`.
    pub fn model_scale(&self) -> Vec3 {
        let r = self.radius[0];
        Vec3::new(r, r, r * self.sprite_obl_z)
    }

    /// Hubble type index of `UseForType` (see [`hubble_type_index`]), 0 if absent.
    pub fn type_index(&self) -> u8 {
        self.use_for_type.as_deref().map_or(0, hubble_type_index)
    }
}

/// Errors from [`parse_pst`].
#[derive(Debug, Error, PartialEq)]
pub enum PstError {
    /// A `}` without matching `{`, or EOF inside a block.
    #[error("unbalanced braces in .pst near line {0}")]
    Unbalanced(usize),
    /// A value could not be parsed as the type its key requires.
    #[error("line {line}: bad value for `{key}`: {value:?}")]
    BadValue {
        /// 1-based line number.
        line: usize,
        /// Key name.
        key: String,
        /// Offending text.
        value: String,
    },
}

fn strip_comment(line: &str) -> &str {
    // `//` never occurs inside the quoted strings of .pst files.
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

fn first_quoted(s: &str) -> Option<&str> {
    let a = s.find('"')?;
    let rest = &s[a + 1..];
    let b = rest.find('"')?;
    Some(&rest[..b])
}

fn numbers(s: &str) -> Option<Vec<f32>> {
    s.split(|c: char| c == ',' || c == '(' || c == ')' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f32>().ok())
        .collect()
}

fn one_f32(s: &str) -> Option<f32> {
    let v = numbers(s)?;
    (v.len() == 1).then(|| v[0])
}

fn vec3(s: &str) -> Option<[f32; 3]> {
    let v = numbers(s)?;
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

fn parse_flag(s: &str) -> Option<bool> {
    match s.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        other => one_f32(other).map(|v| v != 0.0),
    }
}

/// Applies a `em*/abs*/b*` layer key. `suffix` is the key without the prefix.
fn apply_layer_key(l: &mut ParticleLayer, suffix: &str, v: &str) -> Option<bool> {
    // Returns None on a bad value, Some(false) for an unknown suffix, Some(true) on success.
    macro_rules! f {
        ($field:expr) => {{
            $field = one_f32(v)?;
            Some(true)
        }};
    }
    match suffix {
        "ParticleColor" => {
            l.color = vec3(v)?;
            Some(true)
        }
        "ParticleColor2" => {
            l.color2 = vec3(v)?;
            Some(true)
        }
        "DetailR" => f!(l.detail_r),
        "DetailZ" => f!(l.detail_z),
        "ParticleDispR" => f!(l.disp_r),
        "ParticleDispZ" => f!(l.disp_z),
        "ParticleSizeCenter" => f!(l.size_center),
        "ParticleSizeEdge" => f!(l.size_edge),
        "ParticleMinBrightCenter" => f!(l.min_bright_center),
        "ParticleMinBrightEdge" => f!(l.min_bright_edge),
        "ParticleBrightnessCenter" => f!(l.brightness_center),
        "ParticleBrightnessEdge" => f!(l.brightness_edge),
        "NoiseFreq" => f!(l.noise.freq),
        "NoiseDistort" => f!(l.noise.distort),
        "NoiseH" => f!(l.noise.h),
        "NoiseLacunarity" => f!(l.noise.lacunarity),
        "NoiseOffset" => f!(l.noise.offset),
        "NoiseOctaves" => {
            l.noise.octaves = one_f32(v)? as i32;
            Some(true)
        }
        "NoiseRandomize" => {
            l.noise.randomize = vec3(v)?;
            Some(true)
        }
        _ => Some(false),
    }
}

fn apply_key(m: &mut GalaxyModelDef, key: &str, v: &str) -> Option<bool> {
    macro_rules! s {
        ($field:expr) => {{
            $field = first_quoted(v)?.to_string();
            Some(true)
        }};
    }
    macro_rules! f {
        ($field:expr) => {{
            $field = one_f32(v)?;
            Some(true)
        }};
    }
    match key {
        "UseForObject" => {
            m.use_for_object = first_quoted(v)?
                .split('/')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Some(true)
        }
        "UseForType" => {
            m.use_for_type = Some(first_quoted(v)?.to_string());
            Some(true)
        }
        "GroupModel" => s!(m.group_model),
        "Method" => {
            // An unknown method leaves the exe default untouched.
            if let Some(me) = ModelMethod::from_name(first_quoted(v)?) {
                m.method = me;
            }
            Some(true)
        }
        "Position" => {
            m.position = vec3(v)?;
            Some(true)
        }
        "Radius" => {
            m.radius = vec3(v)?;
            Some(true)
        }
        "BBoxRes" => {
            let r = vec3(v)?;
            m.bbox_res = [r[0] as i32, r[1] as i32, r[2] as i32];
            Some(true)
        }
        "Yaw" => f!(m.yaw_pitch_roll[0]),
        "Pitch" => f!(m.yaw_pitch_roll[1]),
        "Roll" => f!(m.yaw_pitch_roll[2]),
        "SpriteOblZ" => f!(m.sprite_obl_z),
        "floatLOD" => {
            m.float_lod = parse_flag(v)?;
            Some(true)
        }
        "useFBO" => {
            m.use_fbo = parse_flag(v)?;
            Some(true)
        }
        "LODbase" => f!(m.lod_base),
        "FrontImage" => s!(m.front_image),
        "SideImage" => s!(m.side_image),
        "CubePosXImage" => s!(m.cube_images[0]),
        "CubeNegXImage" => s!(m.cube_images[1]),
        "CubePosYImage" => s!(m.cube_images[2]),
        "CubeNegYImage" => s!(m.cube_images[3]),
        "CubePosZImage" => s!(m.cube_images[4]),
        "CubeNegZImage" => s!(m.cube_images[5]),
        "FrontTexture" => s!(m.front_texture),
        "SysTexture" => s!(m.sys_texture),
        "Brightness" => f!(m.brightness),
        "Transparency" => f!(m.transparency),
        "clipRadius" => {
            m.clip_radius = Some(one_f32(v)?);
            Some(true)
        }
        "sumColor" => {
            m.sum_color = Some(vec3(v)?);
            Some(true)
        }
        "BulgeRadius" => {
            m.bulge_radius = vec3(v)?;
            Some(true)
        }
        _ => {
            if let Some(sfx) = key.strip_prefix("em") {
                return apply_layer_key(&mut m.em, sfx, v);
            }
            if let Some(sfx) = key.strip_prefix("abs") {
                return apply_layer_key(&mut m.abs, sfx, v);
            }
            if let Some(sfx) = key.strip_prefix('b') {
                if sfx.starts_with(|c: char| c.is_ascii_uppercase()) {
                    return apply_layer_key(&mut m.bulge, sfx, v);
                }
            }
            Some(false)
        }
    }
}

/// Parses the text of a `.pst` file and returns its `GalaxyModel` blocks in file order.
///
/// Other block kinds (`NebulaModel`, ...) are skipped (use [`parse_pst_kind`] for those).  The syntax is line based:
/// `Key value` lines, `//` comments, a block header `GalaxyModel "Name"` followed by `{`
/// on the same or the next line.  Unknown keys are preserved in
/// [`GalaxyModelDef::extra`].  Models with neither `UseForObject` nor `UseForType` are kept
/// but logged (the exe rejects them).
pub fn parse_pst(text: &str) -> Result<Vec<GalaxyModelDef>, PstError> {
    parse_pst_kind(text, "GalaxyModel")
}

/// Like [`parse_pst`] but for another block keyword, e.g. `"NebulaModel"` (`Nebulae-sprite.pak`:
/// same keys, plus `Yaw/Pitch/Roll`, `clipRadius`, `sumColor`, `emParticleColor2`).
pub fn parse_pst_kind(text: &str, block_kind: &str) -> Result<Vec<GalaxyModelDef>, PstError> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur: Option<GalaxyModelDef> = None;
    let mut skipping = false;
    let mut pending_header: Option<(String, String)> = None;
    // The exe looks every key up by name in the block, so the *first* occurrence of a repeated key wins
    // (`SNR0` repeats `emParticleColor` where `emParticleColor2` was meant).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (idx, raw) in text.lines().enumerate() {
        let lineno = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if depth == 0 {
            // Block header, maybe with `{` on the same line.
            if line == "}" {
                return Err(PstError::Unbalanced(lineno));
            }
            let (head, opens) = match line.strip_suffix('{') {
                Some(h) => (h.trim(), true),
                None => (line, false),
            };
            if head == "{" || head.is_empty() {
                // `{` alone: opens the pending header.
                if let Some((kind, name)) = pending_header.take() {
                    depth = 1;
                    start_block(&kind, block_kind, name, &mut cur, &mut skipping);
                    continue;
                }
                return Err(PstError::Unbalanced(lineno));
            }
            let kind = head.split_whitespace().next().unwrap_or("").to_string();
            let name = first_quoted(head).unwrap_or("").to_string();
            if opens {
                depth = 1;
                start_block(&kind, block_kind, name, &mut cur, &mut skipping);
            } else {
                pending_header = Some((kind, name));
            }
            continue;
        }
        if line == "}" {
            depth -= 1;
            if depth == 0 {
                if let Some(m) = cur.take() {
                    if m.use_for_object.is_empty() && m.use_for_type.is_none() {
                        warn!("galaxy model {:?} has neither UseForObject nor UseForType", m.name);
                    }
                    out.push(m);
                }
                seen.clear();
                skipping = false;
            }
            continue;
        }
        if line.ends_with('{') || line == "{" {
            depth += 1; // nested block: skipped content
            continue;
        }
        if skipping || depth > 1 {
            continue;
        }
        let Some(m) = cur.as_mut() else { continue };
        let (key, rest) = match line.find(char::is_whitespace) {
            Some(i) => (&line[..i], line[i..].trim()),
            None => (line, ""),
        };
        if !seen.insert(key.to_string()) {
            debug!("{:?}: repeated key {key:?} ignored (line {lineno})", m.name);
            continue;
        }
        match apply_key(m, key, rest) {
            Some(true) => {}
            Some(false) => m.extra.push((key.to_string(), rest.to_string())),
            None => {
                return Err(PstError::BadValue {
                    line: lineno,
                    key: key.to_string(),
                    value: rest.to_string(),
                })
            }
        }
    }
    if depth != 0 {
        return Err(PstError::Unbalanced(text.lines().count()));
    }
    Ok(out)
}

fn start_block(kind: &str, want: &str, name: String, cur: &mut Option<GalaxyModelDef>, skipping: &mut bool) {
    if kind == want {
        *cur = Some(GalaxyModelDef { name, ..GalaxyModelDef::default() });
        *skipping = false;
    } else {
        debug!("skipping .pst block {kind:?} {name:?}");
        *cur = None;
        *skipping = true;
    }
}

// ---------------------------------------------------------------------------------------
// Model / texture selection
// ---------------------------------------------------------------------------------------

/// Hubble type index used by the exe (`FUN_14016a7a0` CSV reader).
///
/// `E0..E7` -> 1..8, `S0` -> 9, `Sa..Sd` -> 10..13, `SBa..SBd` -> 13..16 (so `Sd` and `SBa`
/// collide, as in the exe), exactly `"Irr"` -> 16, anything else -> 0.  Only indices 9..=16
/// ever get a sprite model; 0..=8 are rendered procedurally.
pub fn hubble_type_index(t: &str) -> u8 {
    let b = t.as_bytes();
    if b.len() < 2 {
        return 0;
    }
    match b[0] {
        b'E' => {
            if (b'0'..=b'7').contains(&b[1]) {
                b[1] - 0x2f
            } else {
                0
            }
        }
        b'I' => {
            if b.len() == 3 && b[1] == b'r' && b[2] == b'r' {
                16
            } else {
                0
            }
        }
        b'S' => {
            if b[1] == b'0' {
                9
            } else if b[1] == b'B' {
                if b.len() > 2 && (b'a'..=b'd').contains(&b[2]) {
                    b[2] - 0x54
                } else {
                    0
                }
            } else if (b'a'..=b'd').contains(&b[1]) {
                b[1] - 0x57
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// True for the Hubble type indices that get a sprite model (S0..Irr).
pub fn type_has_sprite_model(type_index: u8) -> bool {
    (9..=16).contains(&type_index)
}

fn norm_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase()
}

/// Result of [`ModelCatalog::select`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSelection {
    /// Index into the catalogue's model list.
    Model(usize),
    /// No sprite model: type < 9 (E galaxies, unknown type) or no model for the type.
    /// SE draws these procedurally (`galaxy_bulge.glsl`).
    Procedural,
}

/// All loaded galaxy models, with the lookup tables of `seGalModelCache`
/// (`FUN_140266150`).
#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    models: Vec<GalaxyModelDef>,
    by_object: HashMap<String, usize>,
    by_type: [Vec<usize>; 17],
}

impl ModelCatalog {
    /// Builds the lookup tables.  `models` must be in load order (`Default.pst` then
    /// `Kexitt.pst`, i.e. pak file order); a later model with the same `UseForObject`
    /// replaces an earlier one.
    pub fn new(models: Vec<GalaxyModelDef>) -> Self {
        let mut by_object = HashMap::new();
        let mut by_type: [Vec<usize>; 17] = Default::default();
        for (i, m) in models.iter().enumerate() {
            for n in &m.use_for_object {
                if let Some(old) = by_object.insert(norm_name(n), i) {
                    debug!("model {:?} overrides {:?} for object {n:?}", m.name, models[old].name);
                }
            }
            let t = m.type_index() as usize;
            if t != 0 && t < by_type.len() {
                by_type[t].push(i);
            }
        }
        Self { models, by_object, by_type }
    }

    /// The model list in load order.
    pub fn models(&self) -> &[GalaxyModelDef] {
        &self.models
    }

    /// Number of models in the random pool of a Hubble type index.
    pub fn pool_len(&self, type_index: u8) -> usize {
        self.by_type.get(type_index as usize).map_or(0, Vec::len)
    }

    /// The model drawn by a raw random number for a procedural galaxy of Hubble type `type_index`:
    /// `pool[roll % pool.len()]` (SE draws `rand % count` from the same pool as the catalogue galaxies).
    pub fn pool_model(&self, type_index: u8, roll: u32) -> Option<usize> {
        let pool = self.by_type.get(type_index as usize)?;
        (!pool.is_empty()).then(|| pool[(roll % pool.len() as u32) as usize])
    }

    /// Picks the model for a catalogue galaxy.
    ///
    /// 1. An exact (case/space-insensitive) match on a model's `UseForObject` wins.
    /// 2. Otherwise types 9..=16 pick `pool[hash(position) % pool.len()]` where the pool is
    ///    every model whose `UseForType` has the same index (named models included) and
    ///    `hash = ix*0x11adc + iy*0xd191 - iz*0x5754` with `i* = (int)(pos * 0.01)`.
    ///    `position_pc` is the galaxy position as stored in the exe's galaxy record
    ///    (assumed parsecs; units/origin are *inferred*).
    /// 3. Types 0..=8 and empty pools give [`ModelSelection::Procedural`].
    pub fn select(&self, object_name: &str, hubble_type: &str, position_pc: [f64; 3]) -> ModelSelection {
        if let Some(&i) = self.by_object.get(&norm_name(object_name)) {
            return ModelSelection::Model(i);
        }
        let t = hubble_type_index(hubble_type);
        if !type_has_sprite_model(t) {
            return ModelSelection::Procedural;
        }
        let pool = &self.by_type[t as usize];
        if pool.is_empty() {
            return ModelSelection::Procedural;
        }
        let h = position_hash(position_pc);
        ModelSelection::Model(pool[(h % pool.len() as u32) as usize])
    }
}

/// Position hash of `FUN_140266150` (wraps each scaled coordinate into `[-2^31, 2^31)`).
pub fn position_hash(pos: [f64; 3]) -> u32 {
    const R: f64 = 2_147_483_648.0;
    let q = |v: f64| -> i32 {
        let s = (v as f32) as f64 * 0.01;
        let w = s - 2.0 * R * (s / R * 0.5 + 0.5).floor();
        w as i32
    };
    let (ix, iy, iz) = (q(pos[0]), q(pos[1]), q(pos[2]));
    (ix.wrapping_mul(0x11adc)
        .wrapping_add(iy.wrapping_mul(0xd191))
        .wrapping_add(iz.wrapping_mul(-0x5754))) as u32
}

/// Position hash that picks the default model of a catalog *nebula* without a named model
/// (`FUN_140270120`): `pool[type][hash % pool.len()]`. Unlike the galaxy hash the coordinates (float
/// parsecs, as stored in the exe's nebula record) are not scaled and the weights differ:
/// `ix * 0xcd2 + iy * 0x1cb7 - iz * 0x4d0f`, each coordinate wrapped into `(-(2^31-1), 2^31-1)` first.
pub fn nebula_position_hash(pos: [f64; 3]) -> u32 {
    const R: f64 = 2_147_483_647.0;
    let q = |v: f64| -> i32 {
        let s = (v as f32) as f64;
        let w = s - 2.0 * R * (s / R * 0.5 + 0.5).floor();
        w as i32
    };
    let (ix, iy, iz) = (q(pos[0]), q(pos[1]), q(pos[2]));
    (ix.wrapping_mul(0xcd2).wrapping_add(iy.wrapping_mul(0x1cb7)).wrapping_add(iz.wrapping_mul(-0x4d0f))) as u32
}

/// Resolves a texture pattern from a `.pst` file against the names in a pak.
///
/// `pattern` is `Name.*` (any extension) or a full file name.  Matching is ASCII
/// case-insensitive (`NGC1566.*` finds `ngc1566.png`).  With several matches `png` is
/// preferred over `jpg` over anything else.  Returns the name as stored in `available`.
pub fn resolve_texture(available: &[String], pattern: &str) -> Option<String> {
    if pattern.is_empty() {
        return None;
    }
    let (stem, wildcard) = match pattern.strip_suffix(".*") {
        Some(s) => (s, true),
        None => (pattern, false),
    };
    let rank = |n: &str| match n.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => 0,
        Some("jpg") | Some("jpeg") => 1,
        _ => 2,
    };
    available
        .iter()
        .filter(|n| {
            if wildcard {
                match n.rfind('.') {
                    Some(i) => n[..i].eq_ignore_ascii_case(stem),
                    None => false,
                }
            } else {
                n.eq_ignore_ascii_case(stem)
            }
        })
        .min_by_key(|n| rank(n))
        .cloned()
}

/// Texture file names of a model resolved against the available file list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelTextures {
    /// `FrontImage` (512 px, CPU generator input).
    pub front_image: Option<String>,
    /// `SideImage`.
    pub side_image: Option<String>,
    /// `FrontTexture` (full-resolution billboard).
    pub front_texture: Option<String>,
    /// `SysTexture`.
    pub sys_texture: Option<String>,
    /// Cube faces +X, -X, +Y, -Y, +Z, -Z.
    pub cube: [Option<String>; 6],
}

/// Resolves every texture of `model` in `available`.
pub fn model_textures(model: &GalaxyModelDef, available: &[String]) -> ModelTextures {
    ModelTextures {
        front_image: resolve_texture(available, &model.front_image),
        side_image: resolve_texture(available, &model.side_image),
        front_texture: resolve_texture(available, &model.front_texture),
        sys_texture: resolve_texture(available, &model.sys_texture),
        cube: std::array::from_fn(|i| resolve_texture(available, &model.cube_images[i])),
    }
}

/// Catalogue-name -> texture-file helper that works without the `.pst` files.
///
/// Returns the front texture (`<name>.png`) for galaxies that have a dedicated texture
/// (`"NGC 4038"` -> `NGC4038.png`, `"2MASX J00482185-2507365"` ->
/// `2MASXJ00482185-2507365.png`; `"A/B"` lists try each name).  Otherwise, for spiral
/// types it falls back to a *generic* type texture (`S01`, `Sa1`, `Sb2`, `SBb1`, `SBc`,
/// ...) picked by the first match in file order.  Elliptical types return `None`.
///
/// This is only the file-level approximation; use [`ModelCatalog::select`] for SE's real
/// rule (UseForObject, then the position hash over the type pool).
pub fn texture_for_galaxy(name: &str, hubble_type: &str, available: &[String]) -> Option<String> {
    for part in name.split('/') {
        let compact: String = part.split_whitespace().collect();
        if compact.is_empty() {
            continue;
        }
        if let Some(t) = resolve_texture(available, &format!("{compact}.*")).filter(|t| is_front_texture(t)) {
            return Some(t);
        }
    }
    let t = hubble_type_index(hubble_type);
    if !type_has_sprite_model(t) {
        return None;
    }
    let prefix = match hubble_type {
        "Irr" => return None, // irregulars use procedural Diffuse models
        s => s,
    };
    let mut generic: Vec<&String> = available
        .iter()
        .filter(|n| is_front_texture(n))
        .filter(|n| {
            let stem = &n[..n.rfind('.').unwrap_or(n.len())];
            stem.starts_with(prefix) && stem[prefix.len()..].chars().all(|c| c.is_ascii_digit())
        })
        .collect();
    generic.sort();
    generic.first().map(|s| (*s).clone())
}

fn is_front_texture(file: &str) -> bool {
    let l = file.to_ascii_lowercase();
    l.ends_with(".png")
        && !l.ends_with("_small.png")
        && !l.contains("_pos")
        && !l.contains("_neg")
        && !l.contains("_side")
}

/// Converts SE model space to the port's model convention (disk normal = model -Z).
///
/// The two frames are the same: the catalog `orientation` quaternion already maps SE model space
/// (disk normal -Z, towards the observer side; image column -> +x, row down -> +y, which is the
/// unmirrored view from the -Z side) to the universe frame. This function is the identity and exists so
/// call sites document the conversion. (An earlier version rotated by 180 degrees about X; matching the
/// rendered spiral handedness of M 101 / NGC 1365 against their textures showed that this mirrored the
/// image.)
pub fn se_to_port_model(p: [f32; 3]) -> [f32; 3] {
    p
}

// ---------------------------------------------------------------------------------------
// Random numbers
// ---------------------------------------------------------------------------------------

/// Seed of the particle RNG (`FUN_1402db910`): `h = c + 2*h` over the model name bytes
/// (signed chars, wrapping 32-bit).
pub fn model_seed(name: &str) -> u32 {
    name.bytes().fold(0u32, |h, c| (c as i8 as i32 as u32).wrapping_add(h.wrapping_mul(2)))
}

/// The MSVC-style LCG SE keeps in thread-local storage: `s = s * 1664525 + 1013904223`.
#[derive(Debug, Clone)]
pub struct SeRng(pub u32);

impl SeRng {
    /// Creates the RNG for a model.
    pub fn for_model(name: &str) -> Self {
        Self(model_seed(name))
    }
    /// Next raw 32-bit state.
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        self.0
    }
    /// Next float in `[0, 1]` (`(float)state * 2^-32`).
    pub fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 * 2.328_306_4e-10
    }
}

// ---------------------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------------------

/// Decoded image: straight (non-premultiplied) RGBA8, row 0 at the top.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl RgbaImage {
    /// Wraps decoded data; `None` if the length does not match.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        (width > 0 && height > 0 && rgba.len() == width as usize * height as usize * 4)
            .then_some(Self { width, height, rgba })
    }

    fn texel(&self, ix: i32, iy: i32) -> [f32; 4] {
        let x = ix.clamp(0, self.width as i32 - 1) as usize;
        let y = iy.clamp(0, self.height as i32 - 1) as usize;
        let o = (y * self.width as usize + x) * 4;
        let p = &self.rgba[o..o + 4];
        [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]
    }

    /// Bilinear sample at `(u, v)` in `[-1, 1]^2` exactly as `FUN_1402e4320`: pixel
    /// coordinate `(u+1)*0.5*size` (no half-pixel offset), texel indices clamped.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let fx = (u + 1.0) * 0.5 * self.width as f32;
        let fy = (v + 1.0) * 0.5 * self.height as f32;
        let (ix, iy) = (fx as i32, fy as i32);
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let p00 = self.texel(ix, iy);
        let p10 = self.texel(ix + 1, iy);
        let p01 = self.texel(ix, iy + 1);
        let p11 = self.texel(ix + 1, iy + 1);
        let mut o = [0.0; 4];
        for c in 0..4 {
            let a = (p10[c] - p00[c]) * tx + p00[c];
            let b = (p11[c] - p01[c]) * tx + p01[c];
            o[c] = (b - a) * ty + a;
        }
        o
    }

    fn rgb(&self, u: f32, v: f32) -> [f32; 3] {
        let s = self.sample(u, v);
        [s[0], s[1], s[2]]
    }

    fn alpha(&self, u: f32, v: f32) -> f32 {
        self.sample(u, v)[3]
    }
}

/// Images a model needs.  Cube order: +X, -X, +Y, -Y, +Z, -Z.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModelImages<'a> {
    /// `FrontImage` (`*_small.png`, face-on view: RGB = stars, A = dust).
    pub front: Option<&'a RgbaImage>,
    /// `SideImage` (`Side1.png`, edge-on profile).
    pub side: Option<&'a RgbaImage>,
    /// Cube-map faces; missing negative faces default to the positive one.
    pub cube: [Option<&'a RgbaImage>; 6],
}

// ---------------------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------------------

/// Sprite layer stored in bits 4..5 of [`GalaxySprite::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpriteLayer {
    /// Emission (stars), added light.
    Emission = 0,
    /// Absorption (dust), darkens.
    Absorption = 1,
    /// Bulge (analytic central ellipsoid), added light.
    Bulge = 2,
}

/// One particle of a galaxy model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GalaxySprite {
    /// Model-space position (disk radius = 1, see the module docs).
    pub pos: [f32; 3],
    /// Sprite size in model units (`size = factor * voxel step`); SE stores it as a half
    /// float next to the position.
    pub size: f32,
    /// Tint, each channel in `[0,1]`; the max channel equals the layer's `*ParticleColor`.
    pub color: [f32; 3],
    /// Intensity (`(r+g+b)/3 * brightness`), *not* clamped; SE packs `min(1,x)*255`.
    pub brightness: f32,
    /// `atlas_tile | layer << 4`: bits 0..3 index the 4x4 `sprites_gal.png` atlas (0 =
    /// round blob, 1..15 = cloud shapes), bits 4..5 are the [`SpriteLayer`].
    pub kind: u8,
    /// Per-sprite random byte used by SE for LOD thinning (`rand * 255`).
    pub lod_rand: u8,
}

impl GalaxySprite {
    /// Atlas tile 0..15.
    pub fn atlas_tile(&self) -> u8 {
        self.kind & 0x0f
    }
    /// Layer of the sprite.
    pub fn layer(&self) -> SpriteLayer {
        match self.kind >> 4 {
            1 => SpriteLayer::Absorption,
            2 => SpriteLayer::Bulge,
            _ => SpriteLayer::Emission,
        }
    }
    fn pack_kind(tile: u8, layer: SpriteLayer) -> u8 {
        (tile & 0x0f) | ((layer as u8) << 4)
    }
}

/// Dense RGB voxel grid (`nx * ny * nz * 3` floats, x outermost, z innermost).
#[derive(Debug, Clone, PartialEq)]
pub struct DensityMap {
    /// Grid size along X.
    pub nx: usize,
    /// Grid size along Y.
    pub ny: usize,
    /// Grid size along Z.
    pub nz: usize,
    /// RGB values.
    pub data: Vec<f32>,
}

impl DensityMap {
    /// Mean of the three channels, trilinearly interpolated at model position `(x, y, z)`: `x` and
    /// `y` span the grid over `-1..1`, `z` over `-nz/nx..nz/nx` (cubic voxels); 0 outside.
    pub fn sample(&self, x: f32, y: f32, z: f32) -> f32 {
        if x.abs() > 1.0 || y.abs() > 1.0 {
            return 0.0;
        }
        let zc = z * self.nx as f32 / self.nz as f32;
        if zc.abs() > 1.0 {
            return 0.0;
        }
        let f = |c: f32, n: usize| (c + 1.0) * n as f32 * 0.5 - 0.5;
        let (fx, fy, fz) = (f(x, self.nx), f(y, self.ny), f(zc, self.nz));
        let (x0, y0, z0) = (fx.floor(), fy.floor(), fz.floor());
        let (tx, ty, tz) = (fx - x0, fy - y0, fz - z0);
        let at = |x: f32, y: f32, z: f32| -> f32 {
            let (xi, yi, zi) = (x as i64, y as i64, z as i64);
            if xi < 0 || yi < 0 || zi < 0 || xi >= self.nx as i64 || yi >= self.ny as i64 || zi >= self.nz as i64 {
                return 0.0;
            }
            let o = (((xi as usize * self.ny) + yi as usize) * self.nz + zi as usize) * 3;
            (self.data[o] + self.data[o + 1] + self.data[o + 2]) / 3.0
        };
        let mut s = 0.0;
        for (dx, wx) in [(0.0, 1.0 - tx), (1.0, tx)] {
            for (dy, wy) in [(0.0, 1.0 - ty), (1.0, ty)] {
                for (dz, wz) in [(0.0, 1.0 - tz), (1.0, tz)] {
                    s += wx * wy * wz * at(x0 + dx, y0 + dy, z0 + dz);
                }
            }
        }
        s
    }

    /// The red channel as SE's galaxy star generator reads it (FUN_140440840): `x`, `y`, `z` in
    /// `[-1, 1]` over the whole grid along each axis (the galaxy's box), cell `(c + 1) n / 2` with no
    /// half-cell offset, trilinear, corners outside the grid 0.
    pub fn sample_se(&self, x: f32, y: f32, z: f32) -> f32 {
        let f = |c: f32, n: usize| (c + 1.0) * 0.5 * n as f32;
        let (fx, fy, fz) = (f(x, self.nx), f(y, self.ny), f(z, self.nz));
        let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
        let (tx, ty, tz) = (fx - ix as f32, fy - iy as f32, fz - iz as f32);
        let at = |x: i32, y: i32, z: i32| -> f32 {
            if x < 0 || y < 0 || z < 0 || x >= self.nx as i32 || y >= self.ny as i32 || z >= self.nz as i32 {
                return 0.0;
            }
            self.data[(((x as usize * self.ny) + y as usize) * self.nz + z as usize) * 3]
        };
        let lerp = |a: f32, b: f32, t: f32| (b - a) * t + a;
        let x00 = lerp(at(ix, iy, iz), at(ix, iy, iz + 1), tz);
        let x01 = lerp(at(ix, iy + 1, iz), at(ix, iy + 1, iz + 1), tz);
        let x10 = lerp(at(ix + 1, iy, iz), at(ix + 1, iy, iz + 1), tz);
        let x11 = lerp(at(ix + 1, iy + 1, iz), at(ix + 1, iy + 1, iz + 1), tz);
        lerp(lerp(x00, x01, ty), lerp(x10, x11, ty), tx)
    }
}

/// Result of [`build_galaxy_model`].
#[derive(Debug, Clone)]
pub struct GalaxyModelData {
    /// All sprites in SE emission order: emission, absorption, bulge.
    pub sprites: Vec<GalaxySprite>,
    /// `max(|pos| + |size|)` (`clipRadius`).
    pub clip_radius: f32,
    /// `sqrt(sum of normalised colour sums)`; SE keeps it as the model's brightness scale.
    pub total_brightness: f32,
    /// Largest sprite intensity.
    pub max_intensity: f32,
    /// Mean sprite colour normalised to max channel 1 (`sumColor`).
    pub mean_color: [f32; 3],
    /// Emission voxel map (SE keeps it for the galaxy-interior star generator).
    pub emission_map: Option<DensityMap>,
}

/// Errors from [`build_galaxy_model`].
#[derive(Debug, Error, PartialEq)]
pub enum GalaxyGenError {
    /// A required image was not supplied.
    #[error("model {model:?}: missing {what} image")]
    MissingImage {
        /// Model name.
        model: String,
        /// Which image.
        what: &'static str,
    },
    /// The method has no generator in this module.
    #[error("model {model:?}: method {method:?} is not supported")]
    UnsupportedMethod {
        /// Model name.
        model: String,
        /// The method.
        method: ModelMethod,
    },
    /// The thresholds rejected every voxel.
    #[error("model {0:?} produced no sprites")]
    NoSprites(String),
}

// ---------------------------------------------------------------------------------------
// Fractal noise used by Diffuse models (FUN_140048be0 / FUN_1400490a0 / FUN_140049390)
// ---------------------------------------------------------------------------------------

/// Ken Perlin's reference permutation (the exe copies it from `.rdata` 0x14069b2c0).
#[rustfmt::skip]
const PERM: [u8; 256] = [
    151,160,137,91,90,15,131,13,201,95,96,53,194,233,7,225,140,36,103,30,69,142,8,99,37,240,21,10,23,
    190,6,148,247,120,234,75,0,26,197,62,94,252,219,203,117,35,11,32,57,177,33,88,237,149,56,87,174,20,
    125,136,171,168,68,175,74,165,71,134,139,48,27,166,77,146,158,231,83,111,229,122,60,211,133,230,220,
    105,92,41,55,46,245,40,244,102,143,54,65,25,63,161,1,216,80,73,209,76,132,187,208,89,18,169,200,196,
    135,130,116,188,159,86,164,100,109,198,173,186,3,64,52,217,226,250,124,123,5,202,38,147,118,126,255,
    82,85,212,207,206,59,227,47,16,58,17,182,189,28,42,223,183,170,213,119,248,152,2,44,154,163,70,221,
    153,101,155,167,43,172,9,129,22,39,253,19,98,108,110,79,113,224,232,178,185,112,104,218,246,97,228,
    251,34,242,193,238,210,144,12,191,179,162,241,81,51,145,235,249,14,239,107,49,192,214,31,181,199,106,
    157,184,84,204,176,115,121,50,45,127,4,150,254,138,236,205,93,222,114,67,29,24,72,243,141,128,195,78,
    66,215,61,156,180,
];

struct Noise {
    perm: [usize; 512],
    g: [f32; 513],
}

impl Noise {
    fn new() -> Self {
        let mut perm = [0usize; 512];
        let mut g = [0.0f32; 513];
        for i in 0..512 {
            perm[i] = PERM[i & 255] as usize;
            // FUN_140069040: value table = ramp[perm[i]], ramp[j] = 2j/255 - 1.
            g[i] = 2.0 * perm[i] as f32 / 255.0 - 1.0;
        }
        g[512] = g[0];
        Self { perm, g }
    }

    /// Single octave (FUN_140049390): value noise on Perlin's lattice, `g[idx]`/`g[idx+1]`
    /// as the two Z corners, smooth fade `3t^2 - 2t^3`.
    fn n1(&self, p: [f32; 3]) -> f32 {
        let split = |f: f32| {
            let mut i = f as i32;
            if f < 0.0 {
                i -= 1;
            }
            (i, f - i as f32)
        };
        let (xi, fx) = split(p[0]);
        let (yi, fy) = split(p[1]);
        let (zi, fz) = split(p[2]);
        let (x, y, z) = ((xi & 255) as usize, (yi & 255) as usize, (zi & 255) as usize);
        let a = self.perm[x] + y;
        let b = self.perm[x + 1] + y;
        let aa = self.perm[a] + z;
        let ab = self.perm[a + 1] + z;
        let ba = self.perm[b] + z;
        let bb = self.perm[b + 1] + z;
        let fade = |t: f32| (3.0 - (t + t)) * (t * t);
        let (u, v, w) = (fade(fx), fade(fy), fade(fz));
        let lerp = |a: f32, b: f32, t: f32| (b - a) * t + a;
        let g = &self.g;
        let z0 = lerp(lerp(g[aa], g[ba], u), lerp(g[ab], g[bb], u), v);
        let z1 = lerp(lerp(g[aa + 1], g[ba + 1], u), lerp(g[ab + 1], g[bb + 1], u), v);
        lerp(z0, z1, w)
    }

    /// Domain-warp vector (FUN_1400490a0).
    fn warp(&self, mut q: [f32; 3], np: &NoiseParams) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        let mut amp = 1.0f32;
        for _ in 0..np.octaves.max(0) {
            let o1 = [q[0] + 3.33, q[1] + 5.71, q[2] + 1.96];
            let o2 = [q[0] + 7.77, q[1] + 2.65, q[2] + 4.37];
            out[0] += self.n1(q) * amp;
            out[1] += self.n1(o1) * amp;
            out[2] += self.n1(o2) * amp;
            amp *= np.h;
            q = [q[0] * np.lacunarity, q[1] * np.lacunarity, q[2] * np.lacunarity];
        }
        out
    }

    /// Warped fBm (FUN_140048be0).
    fn fbm(&self, p: [f32; 3], np: &NoiseParams) -> f32 {
        let w = self.warp(p, np);
        let mut q = [np.distort * w[0] + p[0], np.distort * w[1] + p[1], np.distort * w[2] + p[2]];
        let mut sum = 0.0f32;
        for i in 0..np.octaves.max(0) {
            sum += np.lacunarity.powf(-np.h * i as f32) * self.n1(q);
            q = [q[0] * np.lacunarity, q[1] * np.lacunarity, q[2] * np.lacunarity];
        }
        sum
    }
}

// ---------------------------------------------------------------------------------------
// Generator core
// ---------------------------------------------------------------------------------------

fn lerp_edge(c: f32, e: f32, t: f32) -> f32 {
    // `if t >= 0 { if t <= 1 { lerp } else { e } } else { c }`
    if t < 0.0 {
        c
    } else if t <= 1.0 {
        (e - c) * t + c
    } else {
        e
    }
}

fn axis(n: usize) -> (f32, Vec<f32>) {
    let step = 2.0 / n as f32;
    let mut c = step * 0.5 - 1.0;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(c);
        c += step;
    }
    (step, v)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Radial {
    /// Disk layers: radius from X/Y only, size interpolated by radius^2.
    Cylindrical,
    /// Bulge / cube / diffuse: radius from X/Y/Z, size interpolated by the voxel value.
    Spherical,
}

struct Grid {
    nx: usize,
    ny: usize,
    nz: usize,
    xs: Vec<f32>,
    ys: Vec<f32>,
    zs: Vec<f32>,
    step: [f32; 3],
    /// Model-space scale of x, y, z (`0x4a4`, `0x4a8`, `0x4ac`).
    scale: [f32; 3],
}

impl Grid {
    fn new(nx: usize, ny: usize, nz: usize, scale: [f32; 3]) -> Self {
        let (sx, xs) = axis(nx);
        let (sy, ys) = axis(ny);
        let (sz, zs) = axis(nz);
        Self { nx, ny, nz, xs, ys, zs, step: [sx, sy, sz], scale }
    }
}

struct LayerGen<'a> {
    layer: SpriteLayer,
    params: &'a ParticleLayer,
    /// Brightness centre/edge after SE's normalisation.
    brightness: (f32, f32),
    radial: Radial,
    /// Spiral/elliptic mode: emission + bulge use atlas tile 0, absorption random tiles.
    fixed_tile_for_emission: bool,
}

#[derive(Default)]
struct Accum {
    sum_bright: f32,
    max_intensity: f32,
    sum_color: [f32; 3],
    clip_radius: f32,
}

fn trunc_i(f: f32) -> i32 {
    f as i32
}

/// Emits the particles of one layer (FUN_1402da540 / FUN_1402d9a10).
#[allow(clippy::neg_cmp_op_on_partial_ord)] // `!(min <= sum)` mirrors the exe's NaN behaviour
fn emit_layer(
    grid: &Grid,
    lg: &LayerGen<'_>,
    rng: &mut SeRng,
    acc: &mut Accum,
    out: &mut Vec<GalaxySprite>,
    mut value: impl FnMut(usize, usize, usize, f32, f32, f32) -> [f32; 3],
) {
    let p = lg.params;
    let em_flag = lg.layer != SpriteLayer::Absorption;
    for i in 0..grid.nx {
        let x = grid.xs[i];
        for j in 0..grid.ny {
            let y = grid.ys[j];
            for k in 0..grid.nz {
                let z = grid.zs[k];
                let [mut r, mut g, mut b] = value(i, j, k, x, y, z);
                let r2 = match lg.radial {
                    Radial::Cylindrical => y * y + x * x,
                    Radial::Spherical => z * z + y * y + x * x,
                };
                let sum = g + r + b;
                if !(lerp_edge(p.min_bright_center, p.min_bright_edge, r2) <= sum) {
                    continue;
                }
                let bright = lerp_edge(lg.brightness.0, lg.brightness.1, r2);
                let intensity = sum * (bright / 3.0);
                let maxc = r.max(g).max(b);
                if maxc != 0.0 {
                    r *= p.color[0] / maxc;
                    g *= p.color[1] / maxc;
                    b *= p.color[2] / maxc;
                }
                acc.sum_bright += g + r + b;
                acc.max_intensity = acc.max_intensity.max(intensity);
                if intensity > 0.0 {
                    acc.sum_color[0] += r / intensity;
                    acc.sum_color[1] += g / intensity;
                    acc.sum_color[2] += b / intensity;
                }
                let size = match lg.radial {
                    Radial::Cylindrical => lerp_edge(p.size_center, p.size_edge, r2) * grid.step[0],
                    Radial::Spherical => {
                        grid.scale[0] * grid.step[0] * lerp_edge(p.size_center, p.size_edge, maxc)
                    }
                };
                let u1 = rng.next_f32();
                let u2 = rng.next_f32();
                let u3 = rng.next_f32();
                let px = ((1.0 - (u1 + u1)) * p.disp_r * grid.step[0] + x) * grid.scale[0];
                let py = ((1.0 - (u2 + u2)) * p.disp_r * grid.step[1] + y) * grid.scale[1];
                let pz = ((1.0 - (u3 + u3)) * p.disp_z * grid.step[2] + z) * grid.scale[2];
                let dist = (py * py + px * px + pz * pz).sqrt() + size.abs();
                acc.clip_radius = acc.clip_radius.max(dist);
                let lod = rng.next_f32();
                let lod_rand = if lod < 1.0 { (lod.max(0.0) * 255.0) as u8 } else { 255 };
                let tile = if lg.fixed_tile_for_emission && em_flag {
                    0
                } else {
                    (rng.next_u32() % 15) as u8 + 1
                };
                if sum <= 0.0 {
                    // SE accepts zero-valued voxels when MinBright is 0 (bulge edge) and
                    // uploads invisible sprites; they still consumed their random numbers.
                    continue;
                }
                out.push(GalaxySprite {
                    pos: [px, py, pz],
                    size,
                    color: [r, g, b],
                    brightness: intensity,
                    kind: GalaxySprite::pack_kind(tile, lg.layer),
                    lod_rand,
                });
            }
        }
    }
}

fn finish(
    sprites: Vec<GalaxySprite>,
    acc: Accum,
    em_map: Option<DensityMap>,
    name: &str,
) -> Result<GalaxyModelData, GalaxyGenError> {
    if sprites.is_empty() {
        return Err(GalaxyGenError::NoSprites(name.to_string()));
    }
    let m = acc.sum_color[0].max(acc.sum_color[1]).max(acc.sum_color[2]);
    let mean_color = if m != 0.0 {
        [acc.sum_color[0] / m, acc.sum_color[1] / m, acc.sum_color[2] / m]
    } else {
        acc.sum_color
    };
    Ok(GalaxyModelData {
        sprites,
        clip_radius: acc.clip_radius,
        total_brightness: acc.sum_bright.max(0.0).sqrt(),
        max_intensity: acc.max_intensity,
        mean_color,
        emission_map: em_map,
    })
}

fn missing(model: &GalaxyModelDef, what: &'static str) -> GalaxyGenError {
    GalaxyGenError::MissingImage { model: model.name.clone(), what }
}

/// `4 / ((sizeC + sizeE) * (detailR + detailZ))` brightness normalisation of createGalaxy.
fn norm_spiral(l: &ParticleLayer) -> (f32, f32) {
    let k = 4.0 / ((l.size_edge + l.size_center) * (l.detail_r + l.detail_z));
    (k * l.brightness_center, k * l.brightness_edge)
}

/// createDiffuse normalisation: `b / ((sizeC+sizeE) * (detailR+detailZ) * 0.25)`.
fn norm_diffuse(l: &ParticleLayer) -> (f32, f32) {
    let d = (l.size_edge + l.size_center) * (l.detail_z + l.detail_r) * 0.25;
    (l.brightness_center / d, l.brightness_edge / d)
}

/// createCubeMap normalisation: `b / ((sizeC+sizeE) * detailR * 0.5)`.
fn norm_cube(l: &ParticleLayer) -> (f32, f32) {
    let d = (l.size_edge + l.size_center) * l.detail_r * 0.5;
    (l.brightness_center / d, l.brightness_edge / d)
}

/// Builds the full sprite cloud of a model.
///
/// Supported methods: `SpiralGalaxy`, `EllipticGalaxy`, `CubeMap`, `Diffuse`, `Cylindrical`.  For a
/// `SpiralGalaxy` the front image is required (a missing side image is an error too); like
/// the exe, the emission and absorption layers need both images and the bulge layer needs
/// none.  The output is deterministic: the RNG is seeded from the model name
/// ([`SeRng::for_model`]) and consumed in SE's order (emission, absorption, bulge; voxels
/// X-major, Z innermost).
pub fn build_galaxy_model(model: &GalaxyModelDef, images: &ModelImages<'_>) -> Result<GalaxyModelData, GalaxyGenError> {
    let mut rng = SeRng::for_model(&model.name);
    match model.method {
        ModelMethod::SpiralGalaxy | ModelMethod::EllipticGalaxy => build_spiral(model, images, &mut rng),
        ModelMethod::CubeMap => build_cube(model, images, &mut rng),
        ModelMethod::Diffuse => build_diffuse(model, &mut rng),
        ModelMethod::Cylindrical => build_cylindrical(model, images, &mut rng),
        ModelMethod::Spherical => build_spherical(model, images, &mut rng),
        ModelMethod::Planetary | ModelMethod::Snr => build_planetary(model, &mut rng),
        m => Err(GalaxyGenError::UnsupportedMethod { model: model.name.clone(), method: m }),
    }
}

/// `Cylindrical` (createCylindrical, FUN_1402d75c0; nebulae): like the spiral emission/absorption
/// maps, but the grid resolution is `Detail * 64`, the side image is sampled at `+-r` (not `r^2`),
/// the `MinBright` interpolation uses `r` and the brightness normalisation is the diffuse one.
/// Both layers use random cloud tiles. Needs `FrontImage` and `SideImage`.
fn build_cylindrical(model: &GalaxyModelDef, images: &ModelImages<'_>, rng: &mut SeRng) -> Result<GalaxyModelData, GalaxyGenError> {
    let front = images.front.ok_or_else(|| missing(model, "front"))?;
    let side = images.side.ok_or_else(|| missing(model, "side"))?;
    let obl = model.sprite_obl_z;
    let side_aspect = side.height as f32 * 64.0 / side.width as f32;
    let mut sprites = Vec::new();
    let mut acc = Accum::default();
    let em_map;

    // ---- emission ----
    let em = &model.em;
    let nxy = match trunc_i(em.detail_r * 64.0) {
        0 => 64,
        n => n.max(1) as usize,
    };
    let nz = match trunc_i(side_aspect / obl * em.detail_z) {
        0 => 15,
        n => n.max(1) as usize,
    };
    let grid = Grid::new(nxy, nxy, nz, [1.0, 1.0, nz as f32 / nxy as f32]);
    let mut map = vec![0.0f32; nxy * nxy * nz * 3];
    let lg = LayerGen {
        layer: SpriteLayer::Emission,
        params: em,
        brightness: norm_diffuse(em),
        radial: Radial::Cylindrical,
        fixed_tile_for_emission: false,
    };
    emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |i, j, k, x, y, z| {
        let r = (y * y + x * x).sqrt();
        if r > 1.0 {
            return [0.0; 3];
        }
        let f = front.rgb(x, y);
        let a = side.rgb(r, z);
        let b = side.rgb(-r, z);
        let s = mix_clamped(a, b, x * 0.5 + 0.5);
        let v = [s[0] * f[0], s[1] * f[1], s[2] * f[2]];
        if v[0] + v[1] + v[2] < lerp_edge(em.min_bright_center, em.min_bright_edge, r) {
            return [0.0; 3];
        }
        let o = ((i * nxy + j) * nz + k) * 3;
        map[o..o + 3].copy_from_slice(&v);
        v
    });
    em_map = Some(DensityMap { nx: nxy, ny: nxy, nz, data: map });

    // ---- absorption ----
    let ab = &model.abs;
    let nxy = match trunc_i(ab.detail_r * 64.0) {
        0 => 64,
        n => n.max(1) as usize,
    };
    let nz = match trunc_i(side_aspect / obl * ab.detail_z) {
        0 => 15,
        n => n.max(1) as usize,
    };
    let grid = Grid::new(nxy, nxy, nz, [1.0, 1.0, nz as f32 / nxy as f32]);
    let lg = LayerGen {
        layer: SpriteLayer::Absorption,
        params: ab,
        brightness: norm_diffuse(ab),
        radial: Radial::Cylindrical,
        fixed_tile_for_emission: false,
    };
    emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |_, _, _, x, y, z| {
        let r = (y * y + x * x).sqrt();
        if r > 1.0 {
            return [0.0; 3];
        }
        let fa = front.alpha(x, y);
        let v = mix_clamped_1(side.alpha(r, z), side.alpha(-r, z), x * 0.5 + 0.5) * fa;
        if v * 3.0 < lerp_edge(ab.min_bright_center, ab.min_bright_edge, r) {
            return [0.0; 3];
        }
        [v; 3]
    });
    finish(sprites, acc, em_map, &model.name)
}

fn build_spiral(model: &GalaxyModelDef, images: &ModelImages<'_>, rng: &mut SeRng) -> Result<GalaxyModelData, GalaxyGenError> {
    let mut sprites = Vec::new();
    let mut acc = Accum::default();
    let mut em_map = None;
    let obl = model.sprite_obl_z;

    if model.method == ModelMethod::SpiralGalaxy {
        let front = images.front.ok_or_else(|| missing(model, "front"))?;
        let side = images.side.ok_or_else(|| missing(model, "side"))?;
        let side_aspect = side.height as f32 * 128.0 / side.width as f32;

        // ---- emission ----------------------------------------------------------------
        let em = &model.em;
        let nxy = match trunc_i(em.detail_r * 128.0) {
            0 => 128,
            n => n.max(1) as usize,
        };
        let nz = match trunc_i(side_aspect / obl * em.detail_z) {
            0 => 15,
            n => n.max(1) as usize,
        };
        let grid = Grid::new(nxy, nxy, nz, [1.0, 1.0, nz as f32 / nxy as f32]);
        let mut map = vec![0.0f32; nxy * nxy * nz * 3];
        let lg = LayerGen {
            layer: SpriteLayer::Emission,
            params: em,
            brightness: norm_spiral(em),
            radial: Radial::Cylindrical,
            fixed_tile_for_emission: true,
        };
        // Per-column cache: front colour and radius^2 depend on (i, j) only.
        emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |i, j, k, x, y, z| {
            let r2 = y * y + x * x;
            let o = ((i * nxy + j) * nz + k) * 3;
            if r2 > 1.0 {
                return [0.0; 3];
            }
            let f = front.rgb(x, y);
            let a = side.rgb(r2, z);
            let b = side.rgb(-r2, z);
            let t = x * 0.5 + 0.5;
            let s = mix_clamped(a, b, t);
            let v = [s[0] * f[0], s[1] * f[1], s[2] * f[2]];
            let minb = lerp_edge(em.min_bright_center, em.min_bright_edge, r2);
            if v[0] + v[1] + v[2] < minb {
                return [0.0; 3];
            }
            map[o..o + 3].copy_from_slice(&v);
            v
        });
        em_map = Some(DensityMap { nx: nxy, ny: nxy, nz, data: map });

        // ---- absorption --------------------------------------------------------------
        let ab = &model.abs;
        let nxy = match trunc_i(ab.detail_r * 128.0) {
            0 => 128,
            n => n.max(1) as usize,
        };
        let nz = match trunc_i(side_aspect / obl * ab.detail_z) {
            0 => 31,
            n => n.max(1) as usize,
        };
        let grid = Grid::new(nxy, nxy, nz, [1.0, 1.0, nz as f32 / nxy as f32]);
        let lg = LayerGen {
            layer: SpriteLayer::Absorption,
            params: ab,
            brightness: norm_spiral(ab),
            radial: Radial::Cylindrical,
            fixed_tile_for_emission: true,
        };
        emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |_, _, _, x, y, z| {
            let r2 = y * y + x * x;
            if r2 > 1.0 {
                return [0.0; 3];
            }
            let fa = front.alpha(x, y);
            let t = x * 0.5 + 0.5;
            let s = mix_clamped_1(side.alpha(r2, z), side.alpha(-r2, z), t) * fa;
            let minb = lerp_edge(ab.min_bright_center, ab.min_bright_edge, r2);
            if s * 3.0 < minb {
                return [0.0; 3];
            }
            [s; 3]
        });
    }

    // ---- bulge ---------------------------------------------------------------------
    let bu = &model.bulge;
    let nxy = match trunc_i(bu.detail_r * 15.0) {
        0 => 15,
        n => n.max(1) as usize,
    };
    let nz = match trunc_i(bu.detail_z * 15.0 / obl) {
        0 => 15,
        n => n.max(1) as usize,
    };
    let ratio = model.radius[1] / model.radius[0];
    let grid = Grid::new(nxy, nxy, nz, [ratio, ratio, (model.radius[2] / model.radius[0]) * nz as f32 / nxy as f32]);
    let br = model.bulge_radius;
    let lg = LayerGen {
        layer: SpriteLayer::Bulge,
        params: bu,
        brightness: norm_spiral(bu),
        radial: Radial::Spherical,
        fixed_tile_for_emission: true,
    };
    emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |_, _, _, x, y, z| {
        let r3 = (y * y + x * x) + z * z;
        if r3 > 1.0 {
            return [0.0; 3];
        }
        let v = (y * y * -2.0 * br[1]).exp() * (x * x * -2.0 * br[0]).exp() * (z * z * -2.0 * br[2]).exp();
        if v * 3.0 < lerp_edge(bu.min_bright_center, bu.min_bright_edge, r3) {
            return [0.0; 3];
        }
        [v; 3]
    });

    finish(sprites, acc, em_map, &model.name)
}

fn mix_clamped(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    if t < 0.0 {
        a
    } else if t <= 1.0 {
        [(b[0] - a[0]) * t + a[0], (b[1] - a[1]) * t + a[1], (b[2] - a[2]) * t + a[2]]
    } else {
        b
    }
}

fn mix_clamped_1(a: f32, b: f32, t: f32) -> f32 {
    if t < 0.0 {
        a
    } else if t <= 1.0 {
        (b - a) * t + a
    } else {
        b
    }
}

fn build_cube(model: &GalaxyModelDef, images: &ModelImages<'_>, rng: &mut SeRng) -> Result<GalaxyModelData, GalaxyGenError> {
    // Missing negative faces default to the positive one.
    fn face<'a>(
        model: &GalaxyModelDef,
        images: &ModelImages<'a>,
        pos: usize,
        neg: usize,
    ) -> Result<(&'a RgbaImage, &'a RgbaImage), GalaxyGenError> {
        let p = images.cube[pos].ok_or_else(|| missing(model, "cube"))?;
        Ok((p, images.cube[neg].unwrap_or(p)))
    }
    let (px, nx_) = face(model, images, 0, 1)?;
    let (py, ny_) = face(model, images, 2, 3)?;
    let (pz, nz_) = face(model, images, 4, 5)?;
    let mut sprites = Vec::new();
    let mut acc = Accum::default();

    for (layer, params, use_alpha) in [(SpriteLayer::Emission, &model.em, false), (SpriteLayer::Absorption, &model.abs, true)] {
        let n = match trunc_i(params.detail_r * 64.0) {
            0 => 64,
            n => n.max(1) as usize,
        };
        let grid = Grid::new(n, n, n, [1.0, 1.0, 1.0]);
        let lg = LayerGen {
            layer,
            params,
            brightness: norm_cube(params),
            radial: Radial::Spherical,
            fixed_tile_for_emission: false,
        };
        emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |_, _, _, x, y, z| {
            let (tx, ty, tz) = (x * 0.5 + 0.5, y * 0.5 + 0.5, z * 0.5 + 0.5);
            if use_alpha {
                let ax = mix_clamped_1(px.alpha(y, z), nx_.alpha(y, z), tx);
                let ay = mix_clamped_1(py.alpha(x, z), ny_.alpha(x, z), ty);
                let az = mix_clamped_1(pz.alpha(x, y), nz_.alpha(x, y), tz);
                let v = ax * ay * az;
                [v; 3]
            } else {
                let cx = mix_clamped(px.rgb(y, z), nx_.rgb(y, z), tx);
                let cy = mix_clamped(py.rgb(x, z), ny_.rgb(x, z), ty);
                let cz = mix_clamped(pz.rgb(x, y), nz_.rgb(x, y), tz);
                [cx[0] * cy[0] * cz[0], cx[1] * cy[1] * cz[1], cx[2] * cy[2] * cz[2]]
            }
        });
    }
    finish(sprites, acc, None, &model.name)
}

fn build_diffuse(model: &GalaxyModelDef, rng: &mut SeRng) -> Result<GalaxyModelData, GalaxyGenError> {
    let noise = Noise::new();
    let mut sprites = Vec::new();
    let mut acc = Accum::default();
    for (layer, params) in [(SpriteLayer::Emission, &model.em), (SpriteLayer::Absorption, &model.abs)] {
        let nxy = match trunc_i(params.detail_r * 64.0) {
            0 => 64,
            n => n.max(1) as usize,
        };
        let nz = match trunc_i(params.detail_z * 64.0) {
            0 => 64,
            n => n.max(1) as usize,
        };
        let grid = Grid::new(nxy, nxy, nz, [1.0, 1.0, nz as f32 / nxy as f32]);
        let lg = LayerGen {
            layer,
            params,
            brightness: norm_diffuse(params),
            radial: Radial::Spherical,
            fixed_tile_for_emission: false,
        };
        let np = &params.noise;
        emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |_, _, _, x, y, z| {
            let p = [x * np.freq + np.randomize[0], y * np.freq + np.randomize[1], z * np.freq + np.randomize[2]];
            let n = noise.fbm(p, np).clamp(0.0, 1.0);
            let r = (z * z + y * y + x * x).sqrt();
            let v = (1.0 - r) * n;
            [v; 3]
        });
    }
    finish(sprites, acc, None, &model.name)
}

/// Grid resolution of the nebula generators: `trunc(Detail * 64)`, 64 if that is 0.
fn nebula_grid_n(detail_r: f32) -> usize {
    match trunc_i(detail_r * 64.0) {
        0 => 64,
        n => n.max(1) as usize,
    }
}

/// `createNebula` copies `DispR` to the y and z jitter (`0x4c4 = 0x4c8 = 0x4c0`), so `DispZ` is ignored.
fn nebula_layer(l: &ParticleLayer) -> ParticleLayer {
    ParticleLayer { disp_z: l.disp_r, ..l.clone() }
}

const TWO_POW_M32: f32 = 2.328_306_4e-10;

/// `Planetary` and `SNR` (`createNebula`, `FUN_1402d3360`): a thick random-width spherical shell of
/// emission voxels (colour blending `ParticleColor` -> `ParticleColor2` from the inner to the outer
/// surface) with a thin shell of dust voxels hugging its inside. See `docs/formats/nebula.md` section 4.
///
/// Random draws, in the exe's order: shell thickness, z squash, (SNR: one per emission voxel), dust
/// thickness, one per dust voxel, then the sprite passes (emission, dust). No images are used.
///
/// The exe tests `Method == SNR` with a sign-extended 4-bit field (`(m << 28) >> 28 == 8`), which is never
/// true, so the shipped 0.991 exe actually builds SNR models like planetaries (and its `switch` has no case
/// for method 8). The port implements the evident intent (thin, 50 % culled shell for `SNR`).
fn build_planetary(model: &GalaxyModelDef, rng: &mut SeRng) -> Result<GalaxyModelData, GalaxyGenError> {
    let snr = model.method == ModelMethod::Snr;
    let (em, ab) = (nebula_layer(&model.em), nebula_layer(&model.abs));
    let (n_em, n_ab) = (nebula_grid_n(em.detail_r), nebula_grid_n(ab.detail_r));

    // Shell geometry: r^2 in [inner, 0.95], z squashed by `squash`.
    let mut thick = rng.next_u32() as f32 * 0.35 * TWO_POW_M32 + 0.3;
    if snr {
        thick *= 0.2;
    }
    let inner = 0.95 - thick;
    let squash = rng.next_u32() as f32 * 0.3 * TWO_POW_M32 + 0.7;

    // ---- emission map ----
    let ratio = [0, 1, 2].map(|k| em.color2[k] / em.color[k]);
    let (_, xs) = axis(n_em);
    let mut em_map = vec![0.0f32; n_em * n_em * n_em * 3];
    let mut o = 0;
    for &x in &xs {
        for &y in &xs {
            for &z in &xs {
                let skip = snr && rng.next_u32() % 100 < 50;
                let r2 = z * z / squash + y * y + x * x;
                if inner <= r2 && r2 <= 0.95 && !skip {
                    let t = (r2 - inner) / thick;
                    for k in 0..3 {
                        em_map[o + k] = lerp_edge(1.0, ratio[k], t);
                    }
                }
                o += 3;
            }
        }
    }

    // ---- dust map: the innermost 5..25 % of the shell, 70 % of the voxels ----
    let abs_thick = rng.next_u32() as f32 * 0.2 * TWO_POW_M32 + 0.05;
    let (step, xs_ab) = axis(n_ab);
    // The exe divides only the *first* z coordinate by the squash (the increment stays `step`).
    let mut zs_ab = Vec::with_capacity(n_ab);
    let mut zc = step * 0.5 / squash - 1.0;
    for _ in 0..n_ab {
        zs_ab.push(zc);
        zc += step;
    }
    let mut ab_map = vec![0.0f32; n_ab * n_ab * n_ab * 3];
    let mut o = 0;
    for &x in &xs_ab {
        for &y in &xs_ab {
            for &z in &zs_ab {
                let keep = rng.next_u32() % 100 > 29;
                let r2 = z * z + y * y + x * x;
                if inner <= r2 && r2 <= abs_thick * thick + inner && keep {
                    ab_map[o..o + 3].copy_from_slice(&ab.color);
                }
                o += 3;
            }
        }
    }

    // ---- sprite passes ----
    let mut sprites = Vec::new();
    let mut acc = Accum::default();
    for (layer, params, map, n) in [
        (SpriteLayer::Emission, &em, &em_map, n_em),
        (SpriteLayer::Absorption, &ab, &ab_map, n_ab),
    ] {
        let grid = Grid::new(n, n, n, [1.0; 3]);
        let lg = LayerGen { layer, params, brightness: norm_cube(params), radial: Radial::Spherical, fixed_tile_for_emission: false };
        emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |i, j, k, _, _, _| {
            let o = ((i * n + j) * n + k) * 3;
            [map[o], map[o + 1], map[o + 2]]
        });
    }
    finish(sprites, acc, None, &model.name)
}

/// `Spherical` (`createSpherical`, `FUN_1402d6920`): the single `FrontImage` is a *profile*: the voxel
/// `(x, y, z)` reads the image at `(+-sqrt(x^2+y^2), z)` and blends the two samples with `x*0.5+0.5`
/// (the image is revolved about the z axis). Emission = RGB, dust = the last channel; cubic grid of
/// `Detail * 64`, no `ParticleColor2`.
fn build_spherical(model: &GalaxyModelDef, images: &ModelImages<'_>, rng: &mut SeRng) -> Result<GalaxyModelData, GalaxyGenError> {
    let front = images.front.ok_or_else(|| missing(model, "front"))?;
    let (em, ab) = (nebula_layer(&model.em), nebula_layer(&model.abs));
    let mut sprites = Vec::new();
    let mut acc = Accum::default();
    for (layer, params) in [(SpriteLayer::Emission, &em), (SpriteLayer::Absorption, &ab)] {
        let n = nebula_grid_n(params.detail_r);
        let grid = Grid::new(n, n, n, [1.0; 3]);
        let lg = LayerGen { layer, params, brightness: norm_cube(params), radial: Radial::Spherical, fixed_tile_for_emission: false };
        emit_layer(&grid, &lg, rng, &mut acc, &mut sprites, |_, _, _, x, y, z| {
            let a = (y * y + x * x).sqrt();
            let t = x * 0.5 + 0.5;
            if layer == SpriteLayer::Emission {
                mix_clamped(front.rgb(a, z), front.rgb(-a, z), t)
            } else {
                [mix_clamped_1(front.alpha(a, z), front.alpha(-a, z), t); 3]
            }
        });
    }
    finish(sprites, acc, None, &model.name)
}

/// Convenience wrapper: sprites only; logs and returns an empty vector on failure.
pub fn generate_galaxy_sprites(model: &GalaxyModelDef, images: &ModelImages<'_>) -> Vec<GalaxySprite> {
    match build_galaxy_model(model, images) {
        Ok(d) => d.sprites,
        Err(e) => {
            warn!("galaxy model generation failed: {e}");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------------------
// Tests (no game files needed)
// ---------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const PST: &str = "GalaxyModel    \"NGC4038\"\r\n{\r\n\tUseForObject    \"NGC 4038/NGC 4039\"\r\n\r\n  \tMethod      \"SpiralGalaxy\"\r\n \tPosition    (0, 0, 0)\r\n \tRadius      (1.1, 0.1, 0.04)    // (disk, bulge, thickness)\r\n \tBBoxRes     (8, 8, 8)\r\n\tfloatLOD    0\r\n\tLODbase     1\r\n\tFrontImage      \"NGC4038_small.*\"\r\n\tSideImage       \"Side1.*\"\r\n\temParticleColor     (1.0, 1.0, 1.0)\r\n\tbParticleColor      (1.0, 0.85, 0.7)\r\n\temDetailR\t0.2\r\n\temDetailZ\t0.3\r\n\tabsDetailR\t0.3\r\n\tabsDetailZ\t0.4\r\n\tbDetailR\t0.4\r\n\tbDetailZ\t0.2\r\n\temParticleDispR\t0.2\r\n\temParticleDispZ\t0.3\r\n\tabsParticleDispR\t0.15\r\n\tabsParticleDispZ\t0.8\r\n\tbParticleDispR\t0.3\r\n\tbParticleDispZ\t0.3\r\n\temParticleSizeCenter\t1.5\r\n\temParticleSizeEdge\t1.4\r\n\tabsParticleSizeCenter\t1.2\r\n\tabsParticleSizeEdge\t1.3\r\n\tbParticleSizeCenter\t1.7\r\n\tbParticleSizeEdge\t1.5\r\n\temParticleMinBrightCenter\t0.1\r\n\temParticleMinBrightEdge\t0.001\r\n\tabsParticleMinBrightCenter\t0.1\r\n\tabsParticleMinBrightEdge\t0.3\r\n\tbParticleMinBrightCenter\t0.1\r\n\tbParticleMinBrightEdge\t0.0\r\n\temParticleBrightnessCenter\t0.2\r\n\temParticleBrightnessEdge\t0.3\r\n\tabsParticleBrightnessCenter\t3.0\r\n\tabsParticleBrightnessEdge\t3.0\r\n\tbParticleBrightnessCenter\t1.4\r\n\tbParticleBrightnessEdge\t0.0\r\n\tSomethingNew 5\r\n}\r\n\r\nNebulaModel \"Foo\"\r\n{\r\n\tUseForType \"Diffuse\"\r\n\tFrontImage \"x.*\"\r\n}\r\n\r\nGalaxyModel \"Irr1\" {\r\n\tUseForType  \"Irr\"\r\n\tMethod \"Diffuse\"\r\n\temNoiseRandomize (0.91, 0.34, 0.78)\r\n\temNoiseOctaves 3\r\n\temDetailR 0.5\r\n\temDetailZ 0.5\r\n\tabsDetailR 0.5\r\n\tabsDetailZ 0.5\r\n\temParticleSizeCenter 2.0\r\n\temParticleSizeEdge 2.0\r\n\tabsParticleSizeCenter 2.0\r\n\tabsParticleSizeEdge 2.0\r\n\temParticleMinBrightCenter 0.05\r\n\temParticleMinBrightEdge 0.05\r\n\tabsParticleMinBrightCenter 0.02\r\n\tabsParticleMinBrightEdge 0.02\r\n\temParticleBrightnessCenter 0.5\r\n\temParticleBrightnessEdge 0.5\r\n\tabsParticleBrightnessCenter 1.5\r\n\tabsParticleBrightnessEdge 1.5\r\n\temParticleDispR 0.3\r\n\temParticleDispZ 0.3\r\n\tabsParticleDispR 0.3\r\n\tabsParticleDispZ 0.3\r\n}\r\n";

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage::new(w, h, px.iter().copied().cycle().take((w * h * 4) as usize).collect()).unwrap()
    }

    fn spiral_model() -> GalaxyModelDef {
        let mut m = parse_pst(PST).unwrap().remove(0);
        // Keep the grids small for the tests.
        m.em.detail_r = 0.25;
        m.abs.detail_r = 0.25;
        m
    }

    #[test]
    fn parses_pst_blocks() {
        let v = parse_pst(PST).unwrap();
        assert_eq!(v.len(), 2, "NebulaModel must be skipped");
        let m = &v[0];
        assert_eq!(m.name, "NGC4038");
        assert_eq!(m.use_for_object, ["NGC 4038", "NGC 4039"]);
        assert_eq!(m.method, ModelMethod::SpiralGalaxy);
        assert_eq!(m.radius, [1.1, 0.1, 0.04]);
        assert_eq!(m.bbox_res, [8, 8, 8]);
        assert_eq!(m.front_image, "NGC4038_small.*");
        assert_eq!(m.bulge.color, [1.0, 0.85, 0.7]);
        assert_eq!(m.abs.disp_z, 0.8);
        assert_eq!(m.extra, [("SomethingNew".to_string(), "5".to_string())]);
        let irr = &v[1];
        assert_eq!(irr.method, ModelMethod::Diffuse);
        assert_eq!(irr.use_for_type.as_deref(), Some("Irr"));
        assert_eq!(irr.em.noise.octaves, 3);
        assert_eq!(irr.em.noise.randomize, [0.91, 0.34, 0.78]);
        assert_eq!(irr.em.noise.h, NoiseParams::default().h);
    }

    #[test]
    fn pst_errors() {
        assert!(matches!(parse_pst("GalaxyModel \"A\"\n{\n Radius (1,2)\n}\n"), Err(PstError::BadValue { .. })));
        assert!(matches!(parse_pst("GalaxyModel \"A\"\n{\n"), Err(PstError::Unbalanced(_))));
    }

    #[test]
    fn hubble_indices() {
        let t = |s: &str| hubble_type_index(s);
        assert_eq!((t("E0"), t("E3"), t("E7")), (1, 4, 8));
        assert_eq!((t("S0"), t("Sa"), t("Sb"), t("Sc")), (9, 10, 11, 12));
        assert_eq!((t("SBa"), t("SBb"), t("SBc"), t("SBab")), (13, 14, 15, 13));
        assert_eq!((t("Irr"), t("Ir"), t("E"), t("")), (16, 0, 0, 0));
        assert_eq!(t("Sd"), t("SBa"), "Sd collides with SBa in the exe");
        assert!(!type_has_sprite_model(t("E5")));
    }

    #[test]
    fn rng_and_seed() {
        assert_eq!(SeRng(0).next_u32(), 1_013_904_223);
        assert_eq!(model_seed("ab"), b'b' as u32 + 2 * b'a' as u32);
        assert_eq!(model_seed(""), 0);
        let mut r = SeRng::for_model("NGC4038");
        let f = r.next_f32();
        assert!((0.0..=1.0).contains(&f));
    }

    #[test]
    fn selection_rules() {
        let src = format!(
            "{PST}\nGalaxyModel \"SBc1\"\n{{\n UseForType \"SBc\"\n}}\nGalaxyModel \"SBc2\"\n{{\n UseForType \"SBc\"\n}}\nGalaxyModel \"M31\"\n{{\n UseForObject \"M 31\"\n UseForType \"Sb\"\n}}\n"
        );
        let cat = ModelCatalog::new(parse_pst(&src).unwrap());
        assert_eq!(cat.pool_len(15), 2);
        assert_eq!(cat.pool_len(11), 1, "named models join the type pool");
        let name_of = |s: ModelSelection| match s {
            ModelSelection::Model(i) => cat.models()[i].name.clone(),
            ModelSelection::Procedural => "-".into(),
        };
        assert_eq!(name_of(cat.select("NGC  4039", "E0", [0.0; 3])), "NGC4038");
        assert_eq!(name_of(cat.select("m 31", "S0", [0.0; 3])), "M31");
        assert_eq!(name_of(cat.select("IC 1", "E3", [1e6; 3])), "-");
        let a = cat.select("IC 1", "SBc", [1.0e7, 2.0e7, 3.0e6]);
        assert_eq!(a, cat.select("IC 1", "SBc", [1.0e7, 2.0e7, 3.0e6]));
        assert!(name_of(a).starts_with("SBc"));
        // type with no pool
        assert_eq!(name_of(cat.select("IC 2", "Sa", [5.0e6, 0.0, 0.0])), "-");
    }

    #[test]
    fn position_hash_matches_formula() {
        // ints (100, 200, 300) after scaling by 0.01
        let h = position_hash([10_000.0, 20_000.0, 30_000.0]);
        let e = 100i32 * 0x11adc + 200 * 0xd191 + 300 * -0x5754;
        assert_eq!(h, e as u32);
    }

    #[test]
    fn texture_resolution_is_case_insensitive() {
        let files: Vec<String> =
            ["ngc1566.png", "NGC4038.png", "NGC4038_small.png", "NGC4038_sys.jpg", "ngc2997_sys.jpg", "Side1.png", "S01.png", "S02.png", "Sa1.png", "SBc.png", "S01_small.png"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(resolve_texture(&files, "NGC1566.*").as_deref(), Some("ngc1566.png"));
        assert_eq!(resolve_texture(&files, "NGC2997_sys.*").as_deref(), Some("ngc2997_sys.jpg"));
        assert_eq!(resolve_texture(&files, "NGC4038.*").as_deref(), Some("NGC4038.png"));
        assert_eq!(resolve_texture(&files, "Nope.*"), None);
        assert_eq!(texture_for_galaxy("NGC 4038", "SBc", &files).as_deref(), Some("NGC4038.png"));
        assert_eq!(texture_for_galaxy("NGC 9999/NGC 1566", "Sb", &files).as_deref(), Some("ngc1566.png"));
        assert_eq!(texture_for_galaxy("X 1", "S0", &files).as_deref(), Some("S01.png"));
        assert_eq!(texture_for_galaxy("X 1", "SBc", &files).as_deref(), Some("SBc.png"));
        assert_eq!(texture_for_galaxy("X 1", "Sb", &files), None, "`SBb1` must not match type Sb");
        assert_eq!(texture_for_galaxy("X 1", "E2", &files), None);
        let m = &parse_pst(PST).unwrap()[0];
        let t = model_textures(m, &["ngc4038_small.png".into(), "Side1.png".into()]);
        assert_eq!(t.front_image.as_deref(), Some("ngc4038_small.png"));
        assert_eq!(t.side_image.as_deref(), Some("Side1.png"));
        assert_eq!(t.front_texture, None);
    }

    #[test]
    fn bilinear_sampling_matches_exe_convention() {
        // 2x1 image: left black, right white.
        let img = RgbaImage::new(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
        // u = -1 -> pixel 0.0 -> left texel exactly.
        assert_eq!(img.sample(-1.0, 0.0)[0], 0.0);
        // u = 0 -> pixel 1.0 -> right texel exactly (no half-pixel offset).
        assert_eq!(img.sample(0.0, 0.0)[0], 1.0);
        // u = -0.5 -> pixel 0.5 -> halfway.
        assert!((img.sample(-0.5, 0.0)[0] - 0.5).abs() < 1e-6);
        // Out of range clamps.
        assert_eq!(img.sample(5.0, 5.0)[0], 1.0);
        assert!(RgbaImage::new(2, 2, vec![0; 3]).is_none());
    }

    #[test]
    fn spiral_generation_is_deterministic_and_layered() {
        let m = spiral_model();
        let front = solid(32, 32, [200, 180, 160, 255]);
        let side = solid(64, 32, [255, 255, 255, 255]);
        let imgs = ModelImages { front: Some(&front), side: Some(&side), cube: [None; 6] };
        let a = build_galaxy_model(&m, &imgs).unwrap();
        let b = build_galaxy_model(&m, &imgs).unwrap();
        assert_eq!(a.sprites, b.sprites, "generation must be deterministic");
        assert!(a.sprites.len() > 100);

        let order: Vec<_> = a.sprites.iter().map(|s| s.layer()).collect();
        let first_abs = order.iter().position(|l| *l == SpriteLayer::Absorption).unwrap();
        let first_bulge = order.iter().position(|l| *l == SpriteLayer::Bulge).unwrap();
        assert!(first_abs > 0 && first_bulge > first_abs, "order must be em, abs, bulge");

        for s in &a.sprites {
            assert!(s.pos.iter().all(|c| c.is_finite() && c.abs() <= 1.3), "{s:?}");
            assert!(s.size > 0.0);
            assert!(s.color.iter().all(|c| (0.0..=1.0 + 1e-5).contains(c)));
            match s.layer() {
                SpriteLayer::Absorption => assert!((1..=15).contains(&s.atlas_tile())),
                _ => assert_eq!(s.atlas_tile(), 0),
            }
        }
        // Emission colour: max channel equals emParticleColor (1.0); bulge max = 1.0 too.
        let em = a.sprites.iter().find(|s| s.layer() == SpriteLayer::Emission).unwrap();
        assert!((em.color[0] - 1.0).abs() < 1e-5);
        let bu = a.sprites.iter().find(|s| s.layer() == SpriteLayer::Bulge).unwrap();
        assert!(a.sprites.iter().all(|s| s.brightness > 0.0), "invisible sprites are dropped");
        assert!((bu.color[0] - 1.0).abs() < 1e-5 && (bu.color[1] - 0.85).abs() < 1e-5, "{bu:?}");
        // Bulge sprites stay inside Radius.y / Radius.x of the disk radius (plus jitter/size).
        let rb = m.radius[1] / m.radius[0];
        for s in a.sprites.iter().filter(|s| s.layer() == SpriteLayer::Bulge) {
            assert!(s.pos[0].abs() <= rb * 1.2 && s.pos[1].abs() <= rb * 1.2, "{s:?}");
        }
        assert!(a.clip_radius > 0.5 && a.clip_radius < 1.6);
        assert!(a.emission_map.is_some());
        assert!(a.max_intensity > 0.0 && a.total_brightness > 0.0);
        let mc = a.mean_color.iter().cloned().fold(0.0, f32::max);
        assert!((mc - 1.0).abs() < 1e-5);

        let wrapper = generate_galaxy_sprites(&m, &imgs);
        assert_eq!(wrapper, a.sprites);
    }

    #[test]
    fn dust_alpha_drives_absorption() {
        let m = spiral_model();
        let side = solid(64, 32, [255, 255, 255, 255]);
        let clear = solid(32, 32, [255, 255, 255, 0]);
        let dusty = solid(32, 32, [255, 255, 255, 255]);
        let count_abs = |front: &RgbaImage| {
            let imgs = ModelImages { front: Some(front), side: Some(&side), cube: [None; 6] };
            generate_galaxy_sprites(&m, &imgs).iter().filter(|s| s.layer() == SpriteLayer::Absorption).count()
        };
        assert_eq!(count_abs(&clear), 0, "alpha 0 means no dust");
        assert!(count_abs(&dusty) > 0);
    }

    #[test]
    fn brightness_normalisation() {
        let m = spiral_model();
        let (c, e) = norm_spiral(&m.em);
        let k = 4.0 / ((1.4 + 1.5) * (0.25 + 0.3));
        assert!((c - k * 0.2).abs() < 1e-5 && (e - k * 0.3).abs() < 1e-5);
    }

    #[test]
    fn missing_images_are_reported() {
        let m = spiral_model();
        let err = build_galaxy_model(&m, &ModelImages::default()).unwrap_err();
        assert!(matches!(err, GalaxyGenError::MissingImage { what: "front", .. }));
        let mut flat = m.clone();
        flat.method = ModelMethod::Flat;
        assert!(matches!(build_galaxy_model(&flat, &ModelImages::default()), Err(GalaxyGenError::UnsupportedMethod { .. })));
        assert!(generate_galaxy_sprites(&flat, &ModelImages::default()).is_empty());
    }

    #[test]
    fn elliptic_is_bulge_only() {
        let mut m = spiral_model();
        m.method = ModelMethod::EllipticGalaxy;
        let d = build_galaxy_model(&m, &ModelImages::default()).unwrap();
        assert!(d.sprites.iter().all(|s| s.layer() == SpriteLayer::Bulge));
        assert!(d.emission_map.is_none());
    }

    #[test]
    fn cubemap_uses_all_axes() {
        let mut m = GalaxyModelDef { name: "Cube".into(), method: ModelMethod::CubeMap, ..GalaxyModelDef::default() };
        m.em.detail_r = 0.1;
        m.abs.detail_r = 0.1;
        m.em.min_bright_center = 0.05;
        m.em.min_bright_edge = 0.05;
        m.abs.min_bright_center = 0.05;
        m.abs.min_bright_edge = 0.05;
        let white = solid(16, 16, [255, 255, 255, 255]);
        let imgs = ModelImages { cube: [Some(&white), None, Some(&white), None, Some(&white), None], ..Default::default() };
        let d = build_galaxy_model(&m, &imgs).unwrap();
        assert!(d.sprites.iter().any(|s| s.layer() == SpriteLayer::Emission));
        assert!(d.sprites.iter().any(|s| s.layer() == SpriteLayer::Absorption));
        // Cube/Diffuse models use random cloud tiles for every layer.
        assert!(d.sprites.iter().all(|s| (1..=15).contains(&s.atlas_tile())));
        let missing_face = ModelImages::default();
        assert!(build_galaxy_model(&m, &missing_face).is_err());
    }

    #[test]
    fn diffuse_noise_model() {
        let m = parse_pst(PST).unwrap().remove(1);
        let a = build_galaxy_model(&m, &ModelImages::default()).unwrap();
        let b = build_galaxy_model(&m, &ModelImages::default()).unwrap();
        assert_eq!(a.sprites, b.sprites);
        assert!(!a.sprites.is_empty());
        // (1 - r) falloff: nothing outside the unit sphere (plus jitter).
        assert!(a.sprites.iter().all(|s| {
            let r = (s.pos[0] * s.pos[0] + s.pos[1] * s.pos[1] + s.pos[2] * s.pos[2]).sqrt();
            r < 1.3
        }));
    }

    /// Parameters of the shipped default planetary / SNR models (`Plan0`, `SNR0`); the `clipRadius` and
    /// `sumColor` lines are the values the exe wrote back after generating them.
    const NEB_PST: &str = "NebulaModel \"Plan0\"\n{\n UseForType \"Planetary\"\n Method \"Planetary\"\n \
        emParticleColor (0.2, 1.0, 0.3)\n emParticleColor2 (1.0, 0.8, 0.1)\n absParticleColor (0.08, 0.24, 0.48)\n \
        emDetailR 0.5\n emDetailZ 0.5\n absDetailR 1.0\n absDetailZ 1.0\n emParticleDispR 0.25\n emParticleDispZ 0.25\n \
        absParticleDispR 2.0\n absParticleDispZ 2.0\n emParticleSizeCenter 2.0\n emParticleSizeEdge 2.0\n \
        absParticleSizeCenter 1.0\n absParticleSizeEdge 1.0\n emParticleMinBrightCenter 0.5\n emParticleMinBrightEdge 1.0\n \
        absParticleMinBrightCenter 0.05\n absParticleMinBrightEdge 0.02\n emParticleBrightnessCenter 0.05\n \
        emParticleBrightnessEdge 0.05\n absParticleBrightnessCenter 1.2\n absParticleBrightnessEdge 1.2\n \
        clipRadius 1.11158\n sumColor (126 255 82)\n}\n\
        NebulaModel \"SNR0\"\n{\n UseForType \"SNR\"\n Method \"SNR\"\n emParticleColor (0.7, 1.0, 0.6)\n \
        emParticleColor (1.0, 0.6, 0.6)\n absParticleColor (0.08, 0.24, 0.48)\n emDetailR 1.0\n emDetailZ 1.0\n \
        absDetailR 1.0\n absDetailZ 1.0\n emParticleDispR 1.0\n emParticleDispZ 1.0\n absParticleDispR 1.0\n \
        absParticleDispZ 1.0\n emParticleSizeCenter 2.0\n emParticleSizeEdge 2.0\n absParticleSizeCenter 1.5\n \
        absParticleSizeEdge 1.5\n emParticleMinBrightCenter 0.1\n emParticleMinBrightEdge 0.1\n \
        absParticleMinBrightCenter 0.03\n absParticleMinBrightEdge 0.03\n emParticleBrightnessCenter 0.05\n \
        emParticleBrightnessEdge 0.05\n absParticleBrightnessCenter 1.5\n absParticleBrightnessEdge 1.5\n \
        clipRadius 1.07363\n sumColor (210 255 202)\n}\n";

    fn neb_models() -> Vec<GalaxyModelDef> {
        parse_pst_kind(NEB_PST, "NebulaModel").unwrap()
    }

    #[test]
    fn repeated_pst_key_keeps_the_first_value() {
        let snr = &neb_models()[1];
        assert_eq!(snr.em.color, [0.7, 1.0, 0.6]);
        assert_eq!(snr.em.color2, [1.0; 3]);
    }

    #[test]
    fn planetary_matches_the_exe_validation_values() {
        // The exe stores `clipRadius` / `sumColor` in the .pst after generating the model; the RNG order,
        // shell geometry and sprite pass are right only if these reproduce (five digits of clipRadius).
        for m in neb_models() {
            let d = build_galaxy_model(&m, &ModelImages::default()).unwrap();
            assert!((d.clip_radius - m.clip_radius.unwrap()).abs() < 2e-5, "{}: clip {}", m.name, d.clip_radius);
            let stored = m.sum_color.unwrap();
            for k in 0..3 {
                assert!((d.mean_color[k] * 255.0 - stored[k]).abs() < 1.5, "{}: sumColor {:?}", m.name, d.mean_color);
            }
        }
    }

    #[test]
    fn planetary_shell_statistics() {
        let planetary = &neb_models()[0];
        let a = build_galaxy_model(planetary, &ModelImages::default()).unwrap();
        let b = build_galaxy_model(planetary, &ModelImages::default()).unwrap();
        assert_eq!(a.sprites, b.sprites, "same name must give the same model");
        let mut renamed = planetary.clone();
        renamed.name = "Plan0b".into();
        let c = build_galaxy_model(&renamed, &ModelImages::default()).unwrap();
        assert_ne!(a.sprites, c.sprites, "the name seeds the shell");

        let em: Vec<_> = a.sprites.iter().filter(|s| s.layer() == SpriteLayer::Emission).collect();
        let dust: Vec<_> = a.sprites.iter().filter(|s| s.layer() == SpriteLayer::Absorption).collect();
        assert!(em.len() > 1000 && dust.len() > 500);
        let rad = |s: &GalaxySprite| (s.pos[0] * s.pos[0] + s.pos[1] * s.pos[1] + s.pos[2] * s.pos[2]).sqrt();
        // Emission: a hollow shell, outer edge sqrt(0.95), inner edge sqrt(0.95 - (0.3..0.65)) >= 0.55,
        // minus the jitter (0.25 voxel) and the z squash (<= 0.7).
        let (lo, hi) = em.iter().fold((f32::MAX, 0.0f32), |(lo, hi), s| (lo.min(rad(s)), hi.max(rad(s))));
        assert!(lo > 0.40 && hi < 1.02, "emission radii {lo}..{hi}");
        let mean = em.iter().map(|s| rad(s)).sum::<f32>() / em.len() as f32;
        assert!(mean > 0.7 && mean < 0.95, "mean emission radius {mean}");
        // No emission sprite near the centre (hollow), unlike a filled blob.
        assert!(em.iter().filter(|s| rad(s) < 0.4).count() == 0);
        // Dust hugs the inner surface: its mean radius is below the emission mean.
        let dmean = dust.iter().map(|s| rad(s)).sum::<f32>() / dust.len() as f32;
        assert!(dmean < mean, "dust {dmean} vs emission {mean}");
        // Colour runs from ParticleColor (green) at the inner edge to ParticleColor2 (yellow) outside.
        let inner_g = em.iter().filter(|s| rad(s) < 0.75).map(|s| s.color[0]).sum::<f32>() / em.iter().filter(|s| rad(s) < 0.75).count() as f32;
        let outer_g = em.iter().filter(|s| rad(s) > 0.9).map(|s| s.color[0]).sum::<f32>() / em.iter().filter(|s| rad(s) > 0.9).count() as f32;
        assert!(outer_g > inner_g, "red rises outward: {inner_g} -> {outer_g}");
        assert!(a.sprites.iter().all(|s| (1..=15).contains(&s.atlas_tile())));
    }

    #[test]
    fn snr_shell_is_thin_and_clumpy() {
        let models = neb_models();
        let snr = build_galaxy_model(&models[1], &ModelImages::default()).unwrap();
        let rad = |s: &GalaxySprite| (s.pos[0] * s.pos[0] + s.pos[1] * s.pos[1] + s.pos[2] * s.pos[2]).sqrt();
        let em: Vec<_> = snr.sprites.iter().filter(|s| s.layer() == SpriteLayer::Emission).collect();
        // thickness <= 0.2 * 0.65 in r^2: all emission sprites sit outside sqrt(0.95 - 0.13) - jitter.
        let lo = em.iter().map(|s| rad(s)).fold(f32::MAX, f32::min);
        assert!(lo > 0.7, "inner edge {lo}");
        // 50 % of the voxels are dropped: fewer sprites than the voxel count of the shell would give.
        assert!(em.len() > 2000 && em.len() < 20000);
    }

    #[test]
    fn nebula_hash_uses_the_exe_weights() {
        // Whole-parsec coordinates: ix*0xcd2 + iy*0x1cb7 - iz*0x4d0f.
        assert_eq!(nebula_position_hash([1.0, 0.0, 0.0]), 0xcd2);
        assert_eq!(nebula_position_hash([0.0, 2.0, 0.0]), 2 * 0x1cb7);
        assert_eq!(nebula_position_hash([0.0, 0.0, 1.0]), (-0x4d0f_i32) as u32);
        assert_eq!(nebula_position_hash([788.7, -12.2, 3.9]), (788 * 0xcd2 - 12 * 0x1cb7 - 3 * 0x4d0f) as u32);
    }

    #[test]
    fn spherical_revolves_the_front_image() {
        // 4x4 image: bright centre columns, black outside; red RGB, alpha 0.
        let mut px = Vec::new();
        for _y in 0..4 {
            for x in 0..4 {
                let v = if (1..3).contains(&x) { 255 } else { 0 };
                px.extend_from_slice(&[v, v / 2, 0, 0]);
            }
        }
        let img = RgbaImage::new(4, 4, px).unwrap();
        let mut m = parse_pst_kind("NebulaModel \"Sph\"\n{\n UseForType \"Planetary\"\n Method \"Spherical\"\n \
            emDetailR 0.25\n absDetailR 0.25\n emParticleMinBrightCenter 0.1\n emParticleMinBrightEdge 0.1\n \
            absParticleMinBrightCenter 10.0\n absParticleMinBrightEdge 10.0\n}\n", "NebulaModel").unwrap().remove(0);
        m.front_image = "x".into();
        let images = ModelImages { front: Some(&img), ..Default::default() };
        let d = build_galaxy_model(&m, &images).unwrap();
        assert!(!d.sprites.is_empty());
        assert!(d.sprites.iter().all(|s| s.layer() == SpriteLayer::Emission), "dust threshold 10 rejects everything");
        // Rotational symmetry about z: the cloud is as wide in x as in y.
        let wx = d.sprites.iter().map(|s| s.pos[0].abs()).fold(0.0, f32::max);
        let wy = d.sprites.iter().map(|s| s.pos[1].abs()).fold(0.0, f32::max);
        assert!((wx - wy).abs() < 0.2, "{wx} vs {wy}");
        assert!(build_galaxy_model(&m, &ModelImages::default()).is_err(), "needs the front image");
    }

    #[test]
    fn noise_is_bounded_and_smooth() {
        let n = Noise::new();
        let mut max: f32 = 0.0;
        for i in 0..200 {
            let p = [i as f32 * 0.137, i as f32 * 0.071 - 3.0, i as f32 * 0.053];
            let v = n.n1(p);
            assert!(v.is_finite() && (-1.0001..=1.0001).contains(&v));
            max = max.max(v.abs());
            // continuity: tiny step -> tiny change
            let q = [p[0] + 1e-4, p[1], p[2]];
            assert!((n.n1(q) - v).abs() < 0.01);
        }
        assert!(max > 0.2);
        // lattice points reproduce the table value (fract = 0 -> g[perm chain]).
        let v = n.n1([3.0, 4.0, 5.0]);
        let a = n.perm[3] + 4;
        let aa = n.perm[a] + 5;
        assert!((v - n.g[aa]).abs() < 1e-6);
    }

    #[test]
    fn port_frame_helper_is_a_rotation() {
        assert_eq!(se_to_port_model([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
        let m = GalaxyModelDef { radius: [2.0, 1.0, 1.0], sprite_obl_z: 0.5, ..Default::default() };
        assert_eq!(m.model_scale(), Vec3::new(2.0, 2.0, 1.0));
    }
}
