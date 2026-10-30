//! Spacecraft: ship designs (`.sss`), module configs (`.cfg`) and materials (`.sml`).
//!
//! A ship design (`models/spacecraft/default/*.sss`) is a `.sc`-syntax script with a
//! header (name, class, length, mass, engine accelerations, warp), camera presets and a
//! list of `Module "id" { "cfg" (4x4 matrix) }` placements. Each module config names a
//! mesh and materials and carries `EngineEffect` blocks. See `docs/formats/spacecraft.md`.
//!
//! The header values were decoded from the executable (`seSpacecraft::ReadSchemeScript`):
//! `MainEngines`/`RetroEngines`/`HoverEngines`/`CorrEngines` are accelerations in m/s^2,
//! `TurnEngines` is an angular acceleration in rad/s^2, `WarpBoostLog` is the maximum
//! log10 of the warp velocity multiplier.

use std::collections::HashMap;

use glam::{DMat4, DQuat, DVec3};

use crate::model_script::{join_path, resolve_relative};
use crate::sc::{self, Block, Item, Value};
use crate::vfs::Vfs;

pub mod aero;
pub use aero::AeroModel;

/// Defaults used by the executable when a key is missing (`FUN_140524610`).
pub const DEFAULT_MAIN_ENGINES: f64 = 50.0;
pub const DEFAULT_RETRO_ENGINES: f64 = 50.0;
pub const DEFAULT_TURN_ENGINES: f64 = 0.002;
pub const DEFAULT_WARP_BOOST_LOG: f64 = 12.0;
pub const DEFAULT_MASS_KG: f64 = 18_000.0;

/// Camera preset type of a `Camera { "Behind" ... }` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraKind {
    Auto,
    Behind,
    Cockpit,
    Other,
}

/// One camera preset: position in ship space (metres) and orientation.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipCamera {
    pub kind: CameraKind,
    pub pos: DVec3,
    pub rot: DQuat,
}

/// `Module "id" { "cfg" (matrix) }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipModule {
    pub id: String,
    /// Config path as written (relative to `models/`, e.g. `Modules/thruster.cfg`).
    pub cfg: String,
    /// The 16 numbers as written. Element 12..15 hold the translation (row-vector
    /// layout), so this is a column-major column-vector matrix.
    pub matrix: [f64; 16],
}

impl ShipModule {
    /// Module space (metres) to ship space.
    pub fn transform(&self) -> DMat4 {
        DMat4::from_cols_array(&self.matrix)
    }
}

/// A parsed ship design.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipDesign {
    /// File stem of the `.sss`, e.g. `DragonFly`.
    pub id: String,
    /// VFS path of the script.
    pub path: String,
    pub name: String,
    pub class: String,
    pub pack: String,
    pub faction: String,
    /// `Length` in metres.
    pub length_m: f64,
    /// `Offset` in metres, added to every module position (camera presets are already in
    /// the shifted frame).
    pub offset: DVec3,
    /// `Quat (w x y z)`: model orientation.
    pub quat: DQuat,
    pub mass_kg: f64,
    pub albedo: f64,
    pub exposure: f64,
    /// Engine accelerations in m/s^2 (independent of mass).
    pub main_engines: f64,
    pub retro_engines: f64,
    pub hover_engines: f64,
    pub corr_engines: f64,
    /// Angular acceleration in rad/s^2.
    pub turn_engines: f64,
    /// Maximum log10 of the warp multiplier.
    pub warp_boost_log: f64,
    pub warpdrive: bool,
    pub hyperdrive: bool,
    pub aerodynamics: bool,
    pub aero_model: Option<String>,
    /// The aerodynamic model `Aerodynamics` / `AeroModel` select ([`AeroModel::from_keys`]).
    pub aero: AeroModel,
    pub rad_temp: f64,
    pub cameras: Vec<ShipCamera>,
    pub modules: Vec<ShipModule>,
}

impl Default for ShipDesign {
    fn default() -> Self {
        Self {
            id: String::new(),
            path: String::new(),
            name: String::new(),
            class: String::new(),
            pack: String::new(),
            faction: String::new(),
            length_m: 10.0,
            offset: DVec3::ZERO,
            quat: DQuat::IDENTITY,
            mass_kg: DEFAULT_MASS_KG,
            albedo: 0.3,
            exposure: 2.0,
            main_engines: DEFAULT_MAIN_ENGINES,
            retro_engines: DEFAULT_RETRO_ENGINES,
            hover_engines: 0.0,
            corr_engines: 0.0,
            turn_engines: DEFAULT_TURN_ENGINES,
            warp_boost_log: DEFAULT_WARP_BOOST_LOG,
            warpdrive: false,
            hyperdrive: false,
            aerodynamics: false,
            aero_model: None,
            aero: AeroModel::None,
            rad_temp: 0.0,
            cameras: Vec::new(),
            modules: Vec::new(),
        }
    }
}

fn quat_wxyz(v: [f64; 4]) -> DQuat {
    DQuat::from_xyzw(v[1], v[2], v[3], v[0]).normalize()
}

fn first_str(item: &Item) -> Option<&str> {
    item.args.iter().find_map(|a| match a {
        Value::Str(s) => Some(s.as_str()),
        _ => None,
    })
}

/// All numbers of every tuple argument of `item`, one `Vec` per tuple, in order.
fn tuples(item: &Item) -> Vec<Vec<f64>> {
    item.args
        .iter()
        .filter(|a| matches!(a, Value::Tuple(_)))
        .filter_map(|a| a.as_nums())
        .collect()
}

fn block_bool(root: &Block, key: &str) -> bool {
    root.bool(key).unwrap_or(false)
}

impl ShipDesign {
    /// Parses a `.sss` ship design. `id` and `path` are stored for display and loading.
    pub fn parse(src: &str, id: &str, path: &str) -> ShipDesign {
        let root = sc::parse(src);
        let d = ShipDesign::default();
        let mut out = ShipDesign {
            id: id.to_owned(),
            path: path.to_owned(),
            name: root.str("Name").unwrap_or(id).to_owned(),
            class: root.str("Class").unwrap_or("").to_owned(),
            pack: root.str("Pack").unwrap_or("").to_owned(),
            faction: root.str("Faction").unwrap_or("").to_owned(),
            length_m: root.f64("Length").filter(|l| *l > 0.0).unwrap_or(d.length_m),
            offset: root.vec3("Offset").map(DVec3::from_array).unwrap_or(DVec3::ZERO),
            quat: root.vec4("Quat").map(quat_wxyz).unwrap_or(DQuat::IDENTITY),
            mass_kg: root.f64("Mass").filter(|m| *m > 0.0).unwrap_or(d.mass_kg),
            albedo: root.f64("Albedo").unwrap_or(d.albedo),
            exposure: root.f64("Exposure").unwrap_or(d.exposure),
            main_engines: root.f64("MainEngines").unwrap_or(d.main_engines),
            retro_engines: root.f64("RetroEngines").unwrap_or(d.retro_engines),
            hover_engines: root.f64("HoverEngines").unwrap_or(0.0),
            corr_engines: root.f64("CorrEngines").unwrap_or(0.0),
            turn_engines: root.f64("TurnEngines").unwrap_or(d.turn_engines),
            warp_boost_log: root.f64("WarpBoostLog").unwrap_or(d.warp_boost_log),
            warpdrive: block_bool(&root, "Warpdrive"),
            hyperdrive: block_bool(&root, "Hyperdrive"),
            aerodynamics: block_bool(&root, "Aerodynamics"),
            aero_model: root.str("AeroModel").map(str::to_owned),
            aero: AeroModel::from_keys(root.bool("Aerodynamics"), root.str("AeroModel")),
            rad_temp: root.f64("RadTemp").unwrap_or(0.0),
            cameras: Vec::new(),
            modules: Vec::new(),
        };
        for cam in root.all("Camera") {
            let Some(block) = &cam.block else { continue };
            for row in &block.items {
                let Some(kind) = first_str(row) else { continue };
                let t = tuples(row);
                let pos = t.first().filter(|v| v.len() >= 3).map(|v| DVec3::new(v[0], v[1], v[2]));
                let rot = t.get(1).filter(|v| v.len() >= 4).map(|v| quat_wxyz([v[0], v[1], v[2], v[3]]));
                let kind = match kind.to_ascii_lowercase().as_str() {
                    "auto" => CameraKind::Auto,
                    "behind" => CameraKind::Behind,
                    "cockpit" => CameraKind::Cockpit,
                    _ => CameraKind::Other,
                };
                out.cameras.push(ShipCamera {
                    kind,
                    pos: pos.unwrap_or(DVec3::ZERO),
                    rot: rot.unwrap_or(DQuat::IDENTITY),
                });
                break;
            }
        }
        for m in root.all("Module") {
            let id = first_str(m).unwrap_or("").to_owned();
            let Some(block) = &m.block else { continue };
            for row in &block.items {
                let Some(cfg) = first_str(row) else { continue };
                let matrix = tuples(row)
                    .into_iter()
                    .find(|v| v.len() == 16)
                    .map(|v| std::array::from_fn(|i| v[i]))
                    .unwrap_or(DMat4::IDENTITY.to_cols_array());
                out.modules.push(ShipModule { id: id.clone(), cfg: cfg.to_owned(), matrix });
                break;
            }
        }
        out
    }

    /// Loads `models/spacecraft/<dir>/<name>.sss`. `reference` is a design id such as
    /// `DragonFly` (searched in `default` then `real`), or a path relative to
    /// `models/spacecraft/` (`default/DragonFly.sss`), or a full VFS path.
    pub fn load(vfs: &Vfs, reference: &str) -> Option<ShipDesign> {
        let path = find_design_path(vfs, reference)?;
        let src = vfs.read_string(&path).ok()?;
        let id = path.rsplit('/').next()?.trim_end_matches(".sss").trim_end_matches(".SSS");
        Some(ShipDesign::parse(&src, id, &path))
    }

    /// Like [`ShipDesign::load`] but stops before the first `Module` line, so the
    /// (possibly thousands of) module rows are not parsed. `modules` is empty.
    pub fn load_header(vfs: &Vfs, reference: &str) -> Option<ShipDesign> {
        let path = find_design_path(vfs, reference)?;
        let src = vfs.read_string(&path).ok()?;
        let id = path.rsplit('/').next()?.trim_end_matches(".sss").trim_end_matches(".SSS");
        let end = src
            .match_indices("Module")
            .find(|(i, _)| *i == 0 || src.as_bytes()[i - 1] == b'\n')
            .map(|(i, _)| i)
            .unwrap_or(src.len());
        Some(ShipDesign::parse(&src[..end], id, &path))
    }

    /// Camera preset to start with: the first `Behind` preset, else a synthetic one
    /// about one ship length behind and a little above the origin.
    pub fn default_camera(&self) -> ShipCamera {
        self.cameras
            .iter()
            .find(|c| c.kind == CameraKind::Behind)
            .cloned()
            .unwrap_or(ShipCamera {
                kind: CameraKind::Behind,
                pos: DVec3::new(0.0, self.length_m * 0.15, self.length_m * 0.9),
                rot: DQuat::IDENTITY,
            })
    }

    /// The cameras to cycle through: all `Behind` presets (chase), then any `Cockpit` presets;
    /// the synthetic default when the design has neither.
    pub fn chase_cameras(&self) -> Vec<ShipCamera> {
        let mut v: Vec<ShipCamera> = self.cameras.iter().filter(|c| c.kind == CameraKind::Behind).cloned().collect();
        v.extend(self.cameras.iter().filter(|c| c.kind == CameraKind::Cockpit).cloned());
        if v.is_empty() { vec![self.default_camera()] } else { v }
    }

    /// Maximum linear acceleration along the nose (m/s^2).
    pub fn max_main_accel(&self) -> f64 {
        self.main_engines
    }

    /// Whether the ship has a drive that allows warp.
    pub fn can_warp(&self) -> bool {
        self.warpdrive || self.hyperdrive
    }
}

/// Directories of the shipped designs, relative to the VFS root.
pub const DESIGN_DIRS: [&str; 2] = ["models/spacecraft/default", "models/spacecraft/real"];

/// VFS paths of every installed ship design (`.sss`), sorted by file name.
pub fn list_designs(vfs: &Vfs) -> Vec<String> {
    let mut out = Vec::new();
    for dir in DESIGN_DIRS {
        out.extend(vfs.list_ext(dir, "sss"));
    }
    out.sort_by_key(|p| p.rsplit('/').next().unwrap_or("").to_ascii_lowercase());
    out
}

fn find_design_path(vfs: &Vfs, reference: &str) -> Option<String> {
    let r = reference.replace('\\', "/");
    let with_ext = if r.to_ascii_lowercase().ends_with(".sss") { r.clone() } else { format!("{r}.sss") };
    let candidates = [
        with_ext.clone(),
        // Catalogs give models relative to models/ ("spacecraft/real/ApolloEagle.sss").
        join_path("models", &with_ext),
        join_path("models/spacecraft", &with_ext),
        join_path("models/spacecraft/default", &with_ext),
        join_path("models/spacecraft/real", &with_ext),
    ];
    candidates.into_iter().find(|c| vfs.exists(c))
}

// ------------------------------------------------------------------------------------
// Module configs
// ------------------------------------------------------------------------------------

/// Engine effect group (`Group "Main"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectGroup {
    Main,
    Retro,
    Hover,
    /// RCS / correction thrusters.
    Thruster,
    Hyper,
    Other,
}

impl EffectGroup {
    pub fn parse(s: &str) -> EffectGroup {
        match s.to_ascii_lowercase().as_str() {
            "main" => EffectGroup::Main,
            "retro" => EffectGroup::Retro,
            "hover" => EffectGroup::Hover,
            "thruster" => EffectGroup::Thruster,
            "hyper" => EffectGroup::Hyper,
            _ => EffectGroup::Other,
        }
    }
}

/// `EngineEffect "name" { ... }` of a module config. Positions are metres in module space.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineEffect {
    pub name: String,
    pub group: EffectGroup,
    /// `Fusion`, `Ion`, `Ion2`, `Chemical`, `Aerospike`, `Hyper`, ...
    pub kind: String,
    pub pos: DVec3,
    /// Euler angles in degrees (x, y, z); see [`EngineEffect::rotation`].
    pub rot_deg: DVec3,
    /// Width, height, length of the plume in metres.
    pub size: DVec3,
    pub color: [f32; 3],
    pub brightness_hdr: f64,
    pub brightness_real: f64,
    pub sound: Option<String>,
    pub volume: f64,
}

impl EngineEffect {
    /// Orientation of the effect: rotation about X, then Y, then Z (`Rz * Ry * Rx`).
    /// The plume points along the rotated -Z axis ([`EngineEffect::direction`]).
    pub fn rotation(&self) -> DQuat {
        let r = self.rot_deg * (std::f64::consts::PI / 180.0);
        DQuat::from_rotation_z(r.z) * DQuat::from_rotation_y(r.y) * DQuat::from_rotation_x(r.x)
    }

    /// Unit vector of the exhaust direction in module space.
    pub fn direction(&self) -> DVec3 {
        self.rotation() * DVec3::NEG_Z
    }

    /// The looping sound of the effect: its `Sound`, else the executable's default for the group
    /// (FUN_1405693a0): `Shuttle_engine.ogg` for Main and Retro, `Shuttle_hover.ogg` for Hover,
    /// `Shuttle_thruster.ogg` for Thruster and `Starship_warp.ogg` for Hyper.
    pub fn sound_file(&self) -> Option<&str> {
        if let Some(s) = self.sound.as_deref().filter(|s| !s.is_empty()) {
            return Some(s);
        }
        match self.group {
            EffectGroup::Main | EffectGroup::Retro => Some("Shuttle_engine.ogg"),
            EffectGroup::Hover => Some("Shuttle_hover.ogg"),
            EffectGroup::Thruster => Some("Shuttle_thruster.ogg"),
            EffectGroup::Hyper => Some("Starship_warp.ogg"),
            EffectGroup::Other => None,
        }
    }
}

/// VFS paths to try for an engine sound named in a module config: relative to `sounds/`, then
/// the file name in `sounds/spacecraft/` (the executable's defaults are bare file names).
pub fn sound_candidates(name: &str) -> Vec<String> {
    let n = name.replace('\\', "/");
    let file = n.rsplit('/').next().unwrap_or(&n).to_owned();
    let mut v = vec![format!("sounds/{n}"), format!("sounds/{}", n.to_ascii_lowercase()), format!("sounds/spacecraft/{file}")];
    v.dedup();
    v
}

/// Parsed module `.cfg` (also used for the single-module `default/X.cfg` designs).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModuleConfig {
    /// VFS path of the `.cfg`.
    pub path: String,
    /// Metres per mesh unit (default 1).
    pub unit: f64,
    pub length: Option<f64>,
    pub mass: Option<f64>,
    /// `Type "Engine"`, `"Frame"`, `"Hab"`, ...
    pub kind: Option<String>,
    /// `Mesh "spacecraft/default/X.sm"` as written, relative to `models/`.
    pub mesh: Option<String>,
    pub materials: Vec<String>,
    pub thrust_main: Option<f64>,
    pub thrust_retro: Option<f64>,
    pub thrust_hover: Option<f64>,
    pub thrust_corr: Option<f64>,
    pub effects: Vec<EngineEffect>,
}

fn vec3_of(b: &Block, key: &str) -> DVec3 {
    b.vec3(key).map(DVec3::from_array).unwrap_or(DVec3::ZERO)
}

/// Parses a module `.cfg`.
pub fn parse_module_cfg(src: &str, path: &str) -> ModuleConfig {
    let root = sc::parse(src);
    let mut out = ModuleConfig {
        path: path.to_owned(),
        unit: root.f64("Unit").filter(|u| *u > 0.0).unwrap_or(1.0),
        length: root.f64("Length"),
        mass: root.f64("Mass"),
        kind: root.str("Type").map(str::to_owned),
        mesh: root.str("Mesh").map(str::to_owned),
        materials: root.all("Material").filter_map(first_str).map(str::to_owned).collect(),
        thrust_main: root.f64("ThrustMain"),
        thrust_retro: root.f64("ThrustRetro"),
        thrust_hover: root.f64("ThrustHover"),
        thrust_corr: root.f64("ThrustCorr"),
        effects: Vec::new(),
    };
    for e in root.all("EngineEffect") {
        let Some(b) = &e.block else { continue };
        let color = b.vec3("Color").unwrap_or([0.05, 0.25, 0.6]);
        let group = EffectGroup::parse(b.str("Group").unwrap_or(""));
        out.effects.push(EngineEffect {
            name: first_str(e).unwrap_or("").to_owned(),
            group,
            kind: b.str("Type").unwrap_or("Ion").to_owned(),
            pos: vec3_of(b, "Pos"),
            rot_deg: vec3_of(b, "Rot"),
            size: b.vec3("Size").map(DVec3::from_array).unwrap_or(DVec3::new(1.0, 1.0, 5.0)),
            color: [color[0] as f32, color[1] as f32, color[2] as f32],
            brightness_hdr: b.f64("BrightnessHDR").unwrap_or(1.0),
            brightness_real: b.f64("BrightnessReal").unwrap_or(1.0),
            sound: b.str("Sound").map(str::to_owned),
            volume: b.f64_any(&["Volume", "SoundVolume"]).unwrap_or(if group == EffectGroup::Thruster { 0.02 } else { 1.0 }),
        });
    }
    out
}

/// Resolves the VFS path of a module `.cfg` named inside a `.sss`.
pub fn resolve_cfg_path(vfs: &Vfs, sss_path: &str, cfg: &str) -> Option<String> {
    resolve_relative(sss_path, cfg, "models", |p| vfs.exists(p))
        .or_else(|| resolve_relative("models/spacecraft/x/x.sss", cfg, "models/spacecraft", |p| vfs.exists(p)))
}

/// Loads a module config. The mesh path is as written; use [`resolve_mesh_path`].
pub fn load_module_cfg(vfs: &Vfs, sss_path: &str, cfg: &str) -> Option<ModuleConfig> {
    let path = resolve_cfg_path(vfs, sss_path, cfg)?;
    let src = vfs.read_string(&path).ok()?;
    Some(parse_module_cfg(&src, &path))
}

/// VFS path of the `.sm` mesh of a module: its `Mesh` key, or the `.cfg` path with
/// the extension replaced (several modules, e.g. `thruster.cfg`, name no mesh).
pub fn resolve_mesh_path(vfs: &Vfs, cfg: &ModuleConfig) -> Option<String> {
    if let Some(m) = &cfg.mesh {
        if let Some(p) = resolve_relative(&cfg.path, m, "models", |p| vfs.exists(p)) {
            return Some(p);
        }
    }
    let sibling = format!("{}.sm", cfg.path.strip_suffix(".cfg").or_else(|| cfg.path.strip_suffix(".CFG"))?);
    vfs.exists(&sibling).then_some(sibling)
}

// ------------------------------------------------------------------------------------
// Materials
// ------------------------------------------------------------------------------------

/// What the diffuse map's alpha channel means (`DiffMapAlpha`, material flags bits 0-2 in
/// FUN_1402a1fb0): `"None"`, `"Spec"` (specular power, the shader's ASPEC) or `"Transp"`
/// (alpha-tested cut-out, ATRNS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffAlpha {
    None,
    /// The executable's default for a non-PBR `Material`.
    #[default]
    Spec,
    Transp,
}

/// A spacecraft (or catalog model) material from an `.sml` file, with texture paths
/// resolved to VFS paths. Keys and defaults are the executable's (parser FUN_1402a1fb0,
/// defaults FUN_1402a3560, uniforms FUN_14029f200); `ship.glsl` uses them.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipMaterial {
    pub name: String,
    /// `DiffColor` or `DiffColorScale` (same slot): the shader's `DiffColorScale`. Default 1.
    pub diff_color: [f32; 3],
    /// `DiffColorConst`, added to the diffuse colour. Default 0.
    pub diff_const: [f32; 3],
    pub emis_color: [f32; 3],
    /// `EmisBright`, default 1.
    pub emis_bright: f32,
    /// `SpecColor` or `SpecColorScale`, default 1.
    pub spec_color: [f32; 3],
    /// `SpecPower`, default 150.
    pub spec_power: f32,
    /// `DiffGamma`, `SpecGamma`, `EmisGamma` (the shader's `MapCorrections`): 1 leaves the map
    /// as read, a negative value converts from sRGB, others are a power. Default 1.
    pub diff_gamma: f32,
    pub spec_gamma: f32,
    pub emis_gamma: f32,
    pub diff_map: Option<String>,
    pub emis_map: Option<String>,
    pub spec_map: Option<String>,
    /// `TempMap`: surface temperature as a fraction of the design's `RadTemp`.
    pub temp_map: Option<String>,
    /// `NormMap`, or a `BumpMap` height map (`bump_is_height`) turned into a normal map with
    /// `BumpHeight` (default 2) at load time (bump_to_norm.glsl). They share one slot; the
    /// later key wins.
    pub norm_map: Option<String>,
    pub bump_is_height: bool,
    pub bump_height: f32,
    /// `DirectXNormMap`: the normal maps' green channel points down.
    pub directx_norm: bool,
    /// `DetDiffMap`, sampled at `DetDiffScale` and 4x that, mixed by `DetDiffMag`.
    pub det_diff_map: Option<String>,
    /// `DetBumpMap` (a height map, `DetBumpHeight` default 2) or `DetNormMap`, sampled at
    /// `DetBumpScale` and 4x that and blended in by `DetBumpMag`.
    pub det_norm_map: Option<String>,
    pub det_is_height: bool,
    pub det_bump_height: f32,
    /// `DetDiffScale` 32, `DetDiffMag` 0.5, `DetBumpScale` 32, `DetBumpMag` 0.25 by default.
    pub det_diff_scale: f32,
    pub det_diff_mag: f32,
    pub det_bump_scale: f32,
    pub det_bump_mag: f32,
    /// `TempMapMaxTemp`, default 3000 K (used by the PBR path only).
    pub temp_max: f32,
    pub two_sided: bool,
    /// `Reflections` (material flag 0x10): the ship renders an environment map that this
    /// material reflects (ship.glsl ENVMAP). SE's parser has no `EnvMap` key.
    pub reflections: bool,
    pub diff_alpha: DiffAlpha,
    /// A `MaterialPBR` block (flags 0x30, FUN_14029e110): drawn by ship_pbr.glsl, with reflections.
    pub pbr: bool,
    /// `PbrMap`: roughness, metalness and ambient occlusion in r, g, b (ship_pbr.glsl's RmaoMap).
    pub pbr_map: Option<String>,
    /// `RoughnessScale` 1, `MetallnessScale` 1, `RoughnessConst` 0, `MetallnessConst` 0: the PbrMap's
    /// r and g times the scale plus the constant (RMScaleConst).
    pub roughness_scale: f32,
    pub metallness_scale: f32,
    pub roughness_const: f32,
    pub metallness_const: f32,
    /// `AmbOcclScale` 1: the PbrMap's ambient occlusion factor (AmbientParams.z).
    pub amb_occl_scale: f32,
}

impl Default for ShipMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            diff_color: [1.0; 3],
            diff_const: [0.0; 3],
            emis_color: [1.0; 3],
            emis_bright: 1.0,
            spec_color: [1.0; 3],
            spec_power: 150.0,
            diff_gamma: 1.0,
            spec_gamma: 1.0,
            emis_gamma: 1.0,
            diff_map: None,
            emis_map: None,
            spec_map: None,
            temp_map: None,
            norm_map: None,
            bump_is_height: false,
            bump_height: 2.0,
            directx_norm: false,
            det_diff_map: None,
            det_norm_map: None,
            det_is_height: false,
            det_bump_height: 2.0,
            det_diff_scale: 32.0,
            det_diff_mag: 0.5,
            det_bump_scale: 32.0,
            det_bump_mag: 0.25,
            temp_max: 3000.0,
            two_sided: false,
            reflections: false,
            diff_alpha: DiffAlpha::Spec,
            pbr: false,
            pbr_map: None,
            roughness_scale: 1.0,
            metallness_scale: 1.0,
            roughness_const: 0.0,
            metallness_const: 0.0,
            amb_occl_scale: 1.0,
        }
    }
}

fn color3(b: &Block, key: &str, default: [f32; 3]) -> [f32; 3] {
    b.vec3(key).map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).unwrap_or(default)
}

/// Parses one `Material "name" { ... }` block (FUN_1402a1fb0). `map` resolves a texture name.
pub fn parse_ship_material(name: &str, b: &Block, map: impl Fn(&str) -> Option<String>) -> ShipMaterial {
    parse_material_block(name, b, false, map)
}

/// Parses a `Material` or (`pbr`) `MaterialPBR` block.
fn parse_material_block(name: &str, b: &Block, pbr: bool, map: impl Fn(&str) -> Option<String>) -> ShipMaterial {
    let d = ShipMaterial::default();
    let f = |key: &str, def: f32| b.f64(key).map_or(def, |v| v as f32);
    // Keys sharing a slot: the later one in the executable's order wins.
    let color = |a: &str, b2: &str, def: [f32; 3]| color3(b, b2, color3(b, a, def));
    let tex = |key: &str| b.str(key).and_then(&map);
    let (mut norm_map, mut bump_is_height) = (None, false);
    if let Some(p) = tex("BumpMap") {
        (norm_map, bump_is_height) = (Some(p), true);
    }
    if let Some(p) = tex("NormMap") {
        (norm_map, bump_is_height) = (Some(p), false);
    }
    let (mut det_norm_map, mut det_is_height) = (None, false);
    if let Some(p) = tex("DetBumpMap") {
        (det_norm_map, det_is_height) = (Some(p), true);
    }
    if let Some(p) = tex("DetNormMap") {
        (det_norm_map, det_is_height) = (Some(p), false);
    }
    let mut diff_alpha = match b.str("DiffMapAlpha").map(str::to_ascii_lowercase).as_deref() {
        Some("none") => DiffAlpha::None,
        Some("transp") => DiffAlpha::Transp,
        _ => DiffAlpha::Spec,
    };
    // ship_pbr.glsl has no specular power: a PBR material's "Spec" (the default too) is "None".
    if pbr && diff_alpha == DiffAlpha::Spec {
        diff_alpha = DiffAlpha::None;
    }
    ShipMaterial {
        name: name.to_owned(),
        diff_color: color("DiffColor", "DiffColorScale", d.diff_color),
        diff_const: color3(b, "DiffColorConst", d.diff_const),
        emis_color: color3(b, "EmisColor", d.emis_color),
        emis_bright: f("EmisBright", d.emis_bright),
        spec_color: color("SpecColor", "SpecColorScale", d.spec_color),
        spec_power: f("SpecPower", d.spec_power),
        diff_gamma: f("DiffGamma", d.diff_gamma),
        spec_gamma: f("SpecGamma", d.spec_gamma),
        emis_gamma: f("EmisGamma", d.emis_gamma),
        diff_map: tex("DiffMap"),
        emis_map: tex("EmisMap"),
        spec_map: tex("SpecMap"),
        temp_map: tex("TempMap"),
        norm_map,
        bump_is_height,
        bump_height: f("BumpHeight", d.bump_height),
        directx_norm: b.bool("DirectXNormMap").unwrap_or(false),
        det_diff_map: tex("DetDiffMap"),
        det_norm_map,
        det_is_height,
        det_bump_height: f("DetBumpHeight", d.det_bump_height),
        det_diff_scale: f("DetDiffScale", d.det_diff_scale),
        det_diff_mag: f("DetDiffMag", d.det_diff_mag),
        det_bump_scale: f("DetBumpScale", d.det_bump_scale),
        det_bump_mag: f("DetBumpMag", d.det_bump_mag),
        temp_max: f("TempMapMaxTemp", d.temp_max),
        two_sided: b.bool("TwoSided").unwrap_or(false),
        reflections: pbr || b.bool("Reflections").unwrap_or(false),
        diff_alpha,
        pbr,
        pbr_map: tex("PbrMap"),
        roughness_scale: f("RoughnessScale", d.roughness_scale),
        metallness_scale: f("MetallnessScale", d.metallness_scale),
        roughness_const: f("RoughnessConst", d.roughness_const),
        metallness_const: f("MetallnessConst", d.metallness_const),
        amb_occl_scale: f("AmbOcclScale", d.amb_occl_scale),
    }
}

/// Resolves a texture name from an `.sml`: `*` as the extension means any of the known
/// image formats (PNG/JPEG first, since those decode everywhere). The name is relative
/// to the `.sml`'s directory, its parent, or `textures/spacecraft`.
pub fn resolve_texture(exists: impl Fn(&str) -> bool, from_file: &str, name: &str) -> Option<String> {
    let name = name.trim();
    if let Some(stem) = name.strip_suffix(".*") {
        for ext in ["png", "jpg", "jpeg", "dds", "tga"] {
            let cand = format!("{stem}.{ext}");
            if let Some(p) = resolve_relative(from_file, &cand, "textures/spacecraft", &exists) {
                return Some(p);
            }
        }
        None
    } else {
        resolve_relative(from_file, name, "textures/spacecraft", &exists)
    }
}

/// Parses the `Material` blocks of one `.sml` file, then its `MaterialPBR` blocks (FUN_14029e110's
/// order).
pub fn parse_ship_materials(src: &str, file: &str, exists: impl Fn(&str) -> bool) -> Vec<ShipMaterial> {
    let root = sc::parse(src);
    let mut out = Vec::new();
    for (key, pbr) in [("Material", false), ("MaterialPBR", true)] {
        for m in root.all(key) {
            let (Some(name), Some(b)) = (first_str(m), m.block.as_ref()) else { continue };
            out.push(parse_material_block(name, b, pbr, |t| resolve_texture(&exists, file, t)));
        }
    }
    out
}

/// Loads every spacecraft material, keyed by lower-case name.
pub fn load_ship_materials(vfs: &Vfs) -> HashMap<String, ShipMaterial> {
    let mut out = HashMap::new();
    for file in vfs.list("textures/spacecraft") {
        if !file.to_ascii_lowercase().ends_with(".sml") {
            continue;
        }
        let Ok(src) = vfs.read_string(&file) else { continue };
        for m in parse_ship_materials(&src, &file, |p| vfs.exists(p)) {
            out.entry(m.name.to_ascii_lowercase()).or_insert(m);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAGONFLY: &str = r#"
Name    "Fire Dragonfly"
Class   "Starship"
Pack	"SpaceEngine"
Length   775.47357
Offset  (0 0 -15)
Quat    (1 0 0 0)
Mass     6.5000001e8
MainEngines  300
RetroEngines 0
HoverEngines 0
CorrEngines  20
TurnEngines  0.005
WarpBoostLog 15
Hyperdrive   true
Aerodynamics false

Camera { "Auto"	(0 0 0) (1 0 0 0)}
Camera { "Behind"	(0 70 283.0000000000001) (1 0 0 0)}
Camera { "Behind"	(550 50 30) (0.7071067811865476 0 0.7071067811865476 0)}
Camera { "Cockpit"	(0 8.1 -28.8) (1 0 0 0)}

Module  "ship"  { "default/DragonFly.cfg" (-1 0 0 0 0 1 0 0 0 0 -1 0 0 0 0 1) }
Module  "t1"  { "Modules/thruster.cfg" (1 0 0 0 0 1 0 0 0 0 1 0 5 6 -7 1) }
"#;

    const CFG: &str = r#"
Unit     1
Length   765.7419
Mass     6.5000001e8
Mesh     "spacecraft/default/DragonFly.sm"
Material "fire_dragonfly"
Material "fire_dragonfly_engines"

EngineEffect "Main top"
{
	Group    "Main"
	Type     "Fusion"
	Pos     (0 53.7 37.6)
	Rot     (0 0 180)
	Size    (7 7 300)
	Color   (0.05 0.25 0.6)
	Sound    "Spacecraft/Starship_engine.ogg"
	Volume   1.0
}

EngineEffect "Thruster up"
{
	Group    "Thruster"
	Type     "Ion"
	Pos     (0 1 2)
	Rot     (90 0 0)
	Size    (4 4 15)
	Color   (0.15 0.35 0.45)
}
"#;

    #[test]
    fn design_header_and_engines() {
        let d = ShipDesign::parse(DRAGONFLY, "DragonFly", "models/spacecraft/default/DragonFly.sss");
        assert_eq!(d.name, "Fire Dragonfly");
        assert_eq!(d.class, "Starship");
        assert!((d.length_m - 775.47357).abs() < 1e-9);
        assert_eq!(d.offset, DVec3::new(0.0, 0.0, -15.0));
        assert_eq!(d.mass_kg, 6.5000001e8);
        assert_eq!(d.main_engines, 300.0);
        assert_eq!(d.retro_engines, 0.0);
        assert_eq!(d.corr_engines, 20.0);
        assert_eq!(d.turn_engines, 0.005);
        assert_eq!(d.warp_boost_log, 15.0);
        assert!(d.hyperdrive && !d.warpdrive && !d.aerodynamics);
        assert_eq!(d.aero, AeroModel::Arrow);
        assert!(d.can_warp());
    }

    #[test]
    fn missing_keys_use_executable_defaults() {
        let d = ShipDesign::parse("Name \"X\"\n", "X", "x.sss");
        assert_eq!(d.main_engines, DEFAULT_MAIN_ENGINES);
        assert_eq!(d.turn_engines, DEFAULT_TURN_ENGINES);
        assert_eq!(d.warp_boost_log, DEFAULT_WARP_BOOST_LOG);
        assert_eq!(d.mass_kg, DEFAULT_MASS_KG);
        assert!(d.modules.is_empty() && d.cameras.is_empty());
        assert!(!d.can_warp());
    }

    #[test]
    fn cameras_parse_kind_position_and_quaternion() {
        let d = ShipDesign::parse(DRAGONFLY, "DragonFly", "x");
        assert_eq!(d.cameras.len(), 4);
        assert_eq!(d.cameras[0].kind, CameraKind::Auto);
        assert_eq!(d.cameras[1].kind, CameraKind::Behind);
        assert!((d.cameras[1].pos - DVec3::new(0.0, 70.0, 283.0)).length() < 1e-9);
        assert_eq!(d.cameras[3].kind, CameraKind::Cockpit);
        // (w x y z) = (0.7071, 0, 0.7071, 0): 90 degrees about Y.
        let q = d.cameras[2].rot;
        let fwd = q * DVec3::NEG_Z;
        assert!((fwd - DVec3::NEG_X).length() < 1e-9, "{fwd:?}");
        let chase = d.chase_cameras();
        // Two `Behind` presets, then the cockpit.
        assert_eq!(chase.len(), 3);
        assert_eq!(chase[2].kind, CameraKind::Cockpit);
        assert_eq!(d.default_camera().pos.z, 283.0000000000001);
    }

    #[test]
    fn modules_keep_matrix_and_translation() {
        let d = ShipDesign::parse(DRAGONFLY, "DragonFly", "x");
        assert_eq!(d.modules.len(), 2);
        assert_eq!(d.modules[0].id, "ship");
        assert_eq!(d.modules[0].cfg, "default/DragonFly.cfg");
        // 180 degrees about Y: +Z maps to -Z.
        let m = d.modules[0].transform();
        assert!((m.transform_vector3(DVec3::Z) - DVec3::NEG_Z).length() < 1e-12);
        let t = d.modules[1].transform();
        assert_eq!(t.transform_point3(DVec3::ZERO), DVec3::new(5.0, 6.0, -7.0));
    }

    #[test]
    fn module_cfg_effects() {
        let c = parse_module_cfg(CFG, "models/spacecraft/default/DragonFly.cfg");
        assert_eq!(c.unit, 1.0);
        assert_eq!(c.mesh.as_deref(), Some("spacecraft/default/DragonFly.sm"));
        assert_eq!(c.materials.len(), 2);
        assert_eq!(c.effects.len(), 2);
        let e = &c.effects[0];
        assert_eq!(e.name, "Main top");
        assert_eq!(e.group, EffectGroup::Main);
        assert_eq!(e.kind, "Fusion");
        assert_eq!(e.pos, DVec3::new(0.0, 53.7, 37.6));
        assert_eq!(e.size, DVec3::new(7.0, 7.0, 300.0));
        assert_eq!(e.sound.as_deref(), Some("Spacecraft/Starship_engine.ogg"));
        assert_eq!(e.sound_file(), Some("Spacecraft/Starship_engine.ogg"));
        assert_eq!(c.effects[1].group, EffectGroup::Thruster);
        // No Sound or Volume: the executable's thruster defaults.
        assert_eq!(c.effects[1].sound_file(), Some("Shuttle_thruster.ogg"));
        assert_eq!(c.effects[1].volume, 0.02);
        assert_eq!(sound_candidates("Shuttle_hover.ogg").last().unwrap(), "sounds/spacecraft/Shuttle_hover.ogg");
        assert_eq!(sound_candidates("Spacecraft/Starship_engine.ogg")[0], "sounds/Spacecraft/Starship_engine.ogg");
    }

    #[test]
    fn effect_rotation_convention() {
        let mk = |rot: [f64; 3]| EngineEffect {
            name: String::new(),
            group: EffectGroup::Other,
            kind: String::new(),
            pos: DVec3::ZERO,
            rot_deg: DVec3::from_array(rot),
            size: DVec3::ONE,
            color: [1.0; 3],
            brightness_hdr: 1.0,
            brightness_real: 1.0,
            sound: None,
            volume: 1.0,
        };
        // No rotation: the plume points down -Z.
        assert!((mk([0.0, 0.0, 0.0]).direction() - DVec3::NEG_Z).length() < 1e-12);
        // Pitch +90 about X: up (+Y).
        assert!((mk([90.0, 0.0, 0.0]).direction() - DVec3::Y).length() < 1e-9);
        // Yaw +90 about Y: -X.
        assert!((mk([0.0, 90.0, 0.0]).direction() - DVec3::NEG_X).length() < 1e-9);
        // The shuttle's Rot (-90 0 90) points along +X.
        assert!((mk([-90.0, 0.0, 90.0]).direction() - DVec3::X).length() < 1e-9);
    }

    #[test]
    fn texture_wildcards_prefer_png_then_dds() {
        let files = [
            "textures/spacecraft/default/dragonfly/dragonfly-diff.dds",
            "textures/spacecraft/modules/shw-diff.dds",
            "textures/spacecraft/modules/shw-diff.png",
        ];
        let exists = |p: &str| files.iter().any(|f| f.eq_ignore_ascii_case(p));
        let sml = "textures/spacecraft/default/Dragonfly/fire_dragonfly.sml";
        assert_eq!(
            resolve_texture(exists, sml, "Default/Dragonfly/dragonfly-diff.*").map(|p| p.to_ascii_lowercase()).as_deref(),
            Some("textures/spacecraft/default/dragonfly/dragonfly-diff.dds")
        );
        assert_eq!(
            resolve_texture(exists, "textures/spacecraft/modules/SHW.sml", "modules/SHW-diff.*").as_deref(),
            Some("textures/spacecraft/modules/SHW-diff.png")
        );
        assert!(resolve_texture(exists, sml, "nothing.*").is_none());
    }

    #[test]
    fn materials_parse_colors_and_maps() {
        let sml = r#"
Material	"SHW_Default"
{
	DiffColor       (0.5 0.6 0.7)
	EmisColor       (1.000 1.000 1.000)
	EmisBright       0.5
	DiffMap         "modules/SHW-diff.png"
	EmisMap         "modules/SHW-emis-notemp.png"
}
Material "Plain" { DiffColor (1 1 1) }
"#;
        let exists = |p: &str| p.to_ascii_lowercase() == "textures/spacecraft/modules/shw-diff.png";
        let m = parse_ship_materials(sml, "textures/spacecraft/modules/SHW.sml", exists);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, "SHW_Default");
        assert_eq!(m[0].diff_color, [0.5, 0.6, 0.7]);
        assert_eq!(m[0].emis_bright, 0.5);
        assert_eq!(m[0].diff_map.as_deref(), Some("textures/spacecraft/modules/SHW-diff.png"));
        assert!(m[0].emis_map.is_none());
        assert_eq!(m[0].diff_alpha, DiffAlpha::Spec);
        assert!(m[1].diff_map.is_none());
    }

    #[test]
    fn material_defaults_and_shared_slots() {
        let sml = r#"
Material "Shuttle"
{
	SpecColor       (2.000 2.000 2.000)
	DiffMapAlpha	"Spec"
	DiffMap         "default/Shuttle-diff.*"
	BumpMap         "default/Shuttle-bump.*"
	BumpHeight       2.5
	DetBumpMap      "default/Detail-noise.*"
	DetBumpScale     16
	DetBumpHeight    0.3
}
Material "Frame" { DiffColorScale (0.5 0.5 0.5) DiffMapAlpha "Transp" BumpMap "a.png" NormMap "b.png" TwoSided true }
"#;
        let m = parse_ship_materials(sml, "textures/spacecraft/default/Shuttle.sml", |_| true);
        let s = &m[0];
        // The executable's defaults (FUN_1402a3560).
        assert_eq!((s.diff_color, s.emis_bright, s.spec_power), ([1.0; 3], 1.0, 150.0));
        assert_eq!((s.det_diff_scale, s.det_diff_mag, s.det_bump_mag), (32.0, 0.5, 0.25));
        assert_eq!((s.spec_color, s.bump_height, s.det_bump_scale, s.det_bump_height), ([2.0; 3], 2.5, 16.0, 0.3));
        assert!(s.bump_is_height && s.det_is_height && s.norm_map.is_some());
        assert_eq!(s.diff_alpha, DiffAlpha::Spec);
        let f = &m[1];
        assert_eq!(f.diff_color, [0.5; 3]);
        assert_eq!(f.diff_alpha, DiffAlpha::Transp);
        // NormMap comes after BumpMap in the executable and takes the slot.
        assert!(!f.bump_is_height && f.norm_map.as_deref().is_some_and(|p| p.ends_with("b.png")));
        assert!(f.two_sided && f.det_norm_map.is_none());
        assert!(!s.pbr && !s.reflections);
    }

    #[test]
    fn pbr_materials() {
        // real/ApolloEagle.sml, and SHW.sml's "EnvMap", which SE's parser does not know.
        let sml = r#"
MaterialPBR "ApolloEaglePBR"
{
	DiffColorScale  (0.588 0.588 0.588)
	RoughnessScale  0.5
	MetallnessConst 0.25
	DiffMap         "real/ApolloEagle_albedo.dds"
	PbrMap          "real/ApolloEagle_RMAO.dds"
}
Material "SHW_Gold" { SpecPower 50 EnvMap true }
Material "Skylone" { Reflections true }
"#;
        let m = parse_ship_materials(sml, "textures/spacecraft/real/ApolloEagle.sml", |_| true);
        assert_eq!(m.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["SHW_Gold", "Skylone", "ApolloEaglePBR"]);
        assert!(!m[0].reflections && m[1].reflections && !m[1].pbr);
        let p = &m[2];
        assert!(p.pbr && p.reflections && p.pbr_map.as_deref().is_some_and(|t| t.ends_with("ApolloEagle_RMAO.dds")));
        assert_eq!((p.roughness_scale, p.metallness_scale, p.roughness_const, p.metallness_const), (0.5, 1.0, 0.0, 0.25));
        assert_eq!((p.amb_occl_scale, p.diff_alpha, p.diff_color), (1.0, DiffAlpha::None, [0.588; 3]));
    }

    /// Checks every shipped design against the real install and prints bounds data.
    /// Run with: cargo test -p se_core --lib ship::tests::install -- --ignored --nocapture
    #[test]
    #[ignore]
    fn install_designs_resolve() {
        use crate::model_sm::parse_sm;
        let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
        let list = list_designs(&vfs);
        println!("{} designs", list.len());
        let mats = load_ship_materials(&vfs);
        println!("{} materials", mats.len());
        for path in list {
            let d = ShipDesign::load(&vfs, &path).unwrap();
            let mut cfgs: std::collections::HashMap<String, Option<(ModuleConfig, Option<String>)>> = Default::default();
            let (mut missing, mut effects) = (0, 0);
            let (mut mn, mut mx) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
            for m in &d.modules {
                let e = cfgs.entry(m.cfg.to_ascii_lowercase()).or_insert_with(|| {
                    let c = load_module_cfg(&vfs, &d.path, &m.cfg)?;
                    let mesh = resolve_mesh_path(&vfs, &c);
                    Some((c, mesh))
                });
                match e {
                    Some((c, Some(mesh))) => {
                        effects += c.effects.len();
                        if d.modules.len() < 5 {
                            let sm = parse_sm(&vfs.read(mesh).unwrap()).unwrap();
                            let (a, b) = sm.bounds().unwrap();
                            let t = m.transform();
                            for corner in 0..8 {
                                let p = DVec3::new(
                                    if corner & 1 == 0 { a[0] } else { b[0] } as f64,
                                    if corner & 2 == 0 { a[1] } else { b[1] } as f64,
                                    if corner & 4 == 0 { a[2] } else { b[2] } as f64,
                                ) * c.unit;
                                let q = t.transform_point3(p);
                                mn = mn.min(q);
                                mx = mx.max(q);
                            }
                        }
                    }
                    _ => missing += 1,
                }
            }
            println!(
                "{:<14} modules={:<5} cfgs={:<3} missing={:<4} effects={:<3} len={:.1} offset={:?} bounds={:?}..{:?}",
                d.id, d.modules.len(), cfgs.len(), missing, effects, d.length_m, d.offset, mn, mx
            );
        }
    }

    #[test]
    #[ignore]
    fn install_dragonfly_objects() {
        use crate::model_sm::parse_sm;
        let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
        let sm = parse_sm(&vfs.read("models/spacecraft/default/DragonFly.sm").unwrap()).unwrap();
        for (i, o) in sm.objects.iter().enumerate() {
            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &o.positions {
                for k in 0..3 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
            println!("obj {i} mat {} verts {} min {:?} max {:?}", o.material_index, o.positions.len(), mn, mx);
        }
    }
}
