//! Model scripts for asteroid and other `BodyModel` bodies.
//!
//! A catalog `Model "planets/Asteroids/67P.sss"` names a placement script (`.sss`)
//! relative to the VFS `models/` directory. The script instantiates a model config
//! (`.cfg`) through `Module "id" { "x.cfg" (16 numbers) }`, and the config names the
//! binary mesh (`Mesh "x.sm"`, see [`crate::model_sm`]) and the materials (`Material "n"`).
//! Material presets (`DiffMap`, `NormMap`, ...) live in `textures/planets/Asteroids/*.sml`.
//!
//! Relative paths inside a script are resolved against the directory of the file that
//! names them, then that directory's parent, then the search root (`models/` or
//! `textures/`). This matches the shipped data, e.g. the `.sss` names
//! `asteroids/67P.cfg` while the file lives in `models/planets/Asteroids/`.
//!
//! See `docs/formats/sm_model.md` section 4 for the key list.

use crate::sc::{self, Item, Value};
use crate::vfs::Vfs;

/// `Module "id" { "cfg" (matrix) }` entry of a `.sss`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleRef {
    /// Module id, e.g. `"Main"`.
    pub id: String,
    /// Config path as written in the script (relative, see module docs).
    pub cfg: String,
    /// 4x4 transform, column-major, as written (identity if absent).
    pub matrix: Option<[f64; 16]>,
}

/// Parsed `.sss` placement script (text values only, nothing resolved).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SssScript {
    pub name: Option<String>,
    /// `Length` in metres.
    pub length: Option<f64>,
    /// `Offset (x y z)`.
    pub offset: Option<[f64; 3]>,
    /// `Quat (w x y z)`.
    pub quat: Option<[f64; 4]>,
    pub modules: Vec<ModuleRef>,
}

/// Parsed model `.cfg`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CfgScript {
    /// `Unit`: metres per model unit.
    pub unit: Option<f64>,
    /// `Length`: nominal length in metres.
    pub length: Option<f64>,
    /// `Mesh "path.sm"` as written (not resolved).
    pub mesh: Option<String>,
    /// `Material "name"` entries in order; object `material == n` uses `materials[n]`.
    pub materials: Vec<String>,
}

/// One `Material "name" { ... }` preset from a `.sml` file, with texture paths resolved
/// to VFS paths (`None` if the map is not set or cannot be found). The keys are those of SE's
/// model shader (ship.glsl): `DiffColor` (the colour, or the map's tint) or `DiffColorScale`,
/// `SpecColor` / `SpecPower`, a `NormMap` or a `BumpMap` of `BumpHeight`, and the detail maps
/// `DetDiffMap` / `DetBumpMap` tiled `DetDiffScale` / `DetBumpScale` times over the UVs, mixed in
/// by `DetDiffMag` / `DetBumpMag`, the bump `DetBumpHeight` high.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MaterialPreset {
    pub name: String,
    pub diff_map: Option<String>,
    pub norm_map: Option<String>,
    pub bump_map: Option<String>,
    pub bump_height: Option<f64>,
    pub diff_color: Option<[f64; 3]>,
    pub diff_color_scale: Option<[f64; 3]>,
    pub spec_color: Option<[f64; 3]>,
    pub spec_power: Option<f64>,
    pub det_diff_map: Option<String>,
    pub det_bump_map: Option<String>,
    pub det_diff_scale: Option<f64>,
    pub det_bump_scale: Option<f64>,
    pub det_diff_mag: Option<f64>,
    pub det_bump_mag: Option<f64>,
    pub det_bump_height: Option<f64>,
}

/// Fully resolved model, as used by the renderer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelScript {
    /// VFS path of the `.sss` (or the `.cfg` / `.sm` when the reference was one of those).
    pub source_path: String,
    pub name: Option<String>,
    /// VFS path of the model `.cfg`, if any.
    pub cfg_path: Option<String>,
    /// VFS path of the `.sm` mesh.
    pub mesh_path: Option<String>,
    /// `Unit` from the `.cfg` (metres per model unit).
    pub unit: Option<f64>,
    /// `Length` from the `.sss` (metres).
    pub length: Option<f64>,
    /// `Length` from the `.cfg` (metres per its own units).
    pub cfg_length: Option<f64>,
    pub offset: Option<[f64; 3]>,
    pub rotation: Option<[f64; 4]>,
    /// Module transform (column-major 4x4) from the `.sss`.
    pub module_matrix: Option<[f64; 16]>,
    /// Material names from the `.cfg`, indexed by `SmObject::material_index`.
    pub materials: Vec<String>,
    /// `(material name, VFS path of DiffMap texture)` for every material with a diffuse map.
    pub textures: Vec<(String, String)>,
    /// `(material name, VFS path of NormMap texture)` for every material with a normal map.
    pub normal_maps: Vec<(String, String)>,
    /// The `.sml` presets of the materials found, texture paths resolved.
    pub presets: Vec<MaterialPreset>,
}

impl ModelScript {
    /// The `.sml` preset of the object material index `index`, if any.
    pub fn preset_for_material(&self, index: usize) -> Option<&MaterialPreset> {
        let name = self.materials.get(index)?;
        self.presets.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Diffuse texture VFS path for the object material index `index`, if any.
    pub fn diffuse_for_material(&self, index: usize) -> Option<&str> {
        let name = self.materials.get(index)?;
        self.textures
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(name))
            .map(|(_, p)| p.as_str())
    }
}

fn first_str(item: &Item) -> Option<&str> {
    item.args.iter().find_map(|a| match a {
        Value::Str(s) => Some(s.as_str()),
        _ => None,
    })
}

fn first_tuple_nums(item: &Item) -> Option<Vec<f64>> {
    item.args
        .iter()
        .find(|a| matches!(a, Value::Tuple(_)))
        .and_then(|a| a.as_nums())
}

/// Parses a `.sss` placement script.
pub fn parse_sss(src: &str) -> SssScript {
    let root = sc::parse(src);
    let mut out = SssScript {
        name: root.str("Name").map(str::to_owned),
        length: root.f64("Length"),
        offset: root.vec3("Offset"),
        quat: root.vec4("Quat"),
        modules: Vec::new(),
    };
    for m in root.all("Module") {
        let id = first_str(m).unwrap_or("").to_owned();
        let Some(block) = &m.block else { continue };
        // The anonymous row inside the block is `"file.cfg" (16 numbers)`.
        for row in &block.items {
            let Some(cfg) = first_str(row) else { continue };
            let matrix = first_tuple_nums(row)
                .filter(|v| v.len() == 16)
                .map(|v| std::array::from_fn(|i| v[i]));
            out.modules.push(ModuleRef { id: id.clone(), cfg: cfg.to_owned(), matrix });
            break;
        }
    }
    out
}

/// Parses a model `.cfg`.
pub fn parse_cfg(src: &str) -> CfgScript {
    let root = sc::parse(src);
    CfgScript {
        unit: root.f64("Unit"),
        length: root.f64("Length"),
        mesh: root.str("Mesh").map(str::to_owned),
        materials: root.all("Material").filter_map(first_str).map(str::to_owned).collect(),
    }
}

/// Parses `Material "name" { DiffMap ... NormMap ... }` presets. Paths are returned as
/// written; use [`load_material_presets`] to get resolved VFS paths.
pub fn parse_material_presets(src: &str) -> Vec<MaterialPreset> {
    let root = sc::parse(src);
    root.all("Material")
        .filter_map(|m| {
            let name = first_str(m)?.to_owned();
            let blk = m.block.as_ref();
            let s = |k: &str| blk.and_then(|b| b.str(k)).map(str::to_owned);
            let f = |k: &str| blk.and_then(|b| b.f64(k));
            let v = |k: &str| blk.and_then(|b| b.vec3(k));
            Some(MaterialPreset {
                name,
                diff_map: s("DiffMap"),
                norm_map: s("NormMap"),
                bump_map: s("BumpMap"),
                bump_height: f("BumpHeight"),
                diff_color: v("DiffColor"),
                diff_color_scale: v("DiffColorScale"),
                spec_color: v("SpecColor"),
                spec_power: f("SpecPower"),
                det_diff_map: s("DetDiffMap"),
                det_bump_map: s("DetBumpMap"),
                det_diff_scale: f("DetDiffScale"),
                det_bump_scale: f("DetBumpScale"),
                det_diff_mag: f("DetDiffMag"),
                det_bump_mag: f("DetBumpMag"),
                det_bump_height: f("DetBumpHeight"),
            })
        })
        .collect()
}

/// Joins `rel` onto `base` (both `/`-separated), resolving `.` and `..` segments.
pub fn join_path(base: &str, rel: &str) -> String {
    let rel = rel.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for seg in base.split('/').chain(rel.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Directory part of a `/`-separated path (`""` if there is none).
fn parent_dir(p: &str) -> &str {
    p.rsplit_once('/').map(|(a, _)| a).unwrap_or("")
}

/// Resolves `rel` as named inside `from_file`: first next to that file, then in its
/// parent directory, then under `root`. `exists` decides what is present.
pub fn resolve_relative(
    from_file: &str,
    rel: &str,
    root: &str,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    let rel = rel.replace('\\', "/");
    let dir = parent_dir(from_file);
    let parent = parent_dir(dir);
    for base in [dir, parent, root] {
        let cand = join_path(base, &rel);
        if !cand.is_empty() && exists(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Loads every material preset from `textures/planets/Asteroids/*.sml`, resolving the
/// texture names against the `.sml` location (`textures/` as the fallback root).
pub fn load_material_presets(vfs: &Vfs) -> Vec<MaterialPreset> {
    let mut out = Vec::new();
    for file in vfs.list_ext("textures/planets/Asteroids", "sml") {
        let Ok(src) = vfs.read_string(&file) else { continue };
        for mut p in parse_material_presets(&src) {
            let resolve = |t: Option<String>| t.and_then(|t| resolve_relative(&file, &t, "textures", |c| vfs.exists(c)));
            p.diff_map = resolve(p.diff_map);
            p.norm_map = resolve(p.norm_map);
            p.bump_map = resolve(p.bump_map);
            p.det_diff_map = resolve(p.det_diff_map);
            p.det_bump_map = resolve(p.det_bump_map);
            out.push(p);
        }
    }
    out
}

/// Resolves a catalog model reference (relative to `models/`, e.g.
/// `planets/Asteroids/67P.sss`) into mesh, material and texture paths.
///
/// Accepts `.sss` (placement script), `.cfg` (model config) or `.sm` (mesh) references.
/// Returns `None` if the reference or the file it leads to cannot be read.
pub fn load_model_script(vfs: &Vfs, model_ref: &str) -> Option<ModelScript> {
    let rel = model_ref.replace('\\', "/");
    let path = join_path("models", rel.trim_start_matches('/'));
    let lower = path.to_ascii_lowercase();
    let exists = |p: &str| vfs.exists(p);
    let mut out = ModelScript { source_path: path.clone(), ..Default::default() };

    let cfg_path = if lower.ends_with(".sss") {
        let sss = parse_sss(&vfs.read_string(&path).ok()?);
        out.name = sss.name;
        out.length = sss.length;
        out.offset = sss.offset;
        out.rotation = sss.quat;
        let module = sss.modules.into_iter().next()?;
        out.module_matrix = module.matrix;
        resolve_relative(&path, &module.cfg, "models", exists)?
    } else if lower.ends_with(".cfg") {
        path.clone()
    } else if lower.ends_with(".sm") {
        out.mesh_path = Some(path);
        return Some(out);
    } else {
        return None;
    };

    let cfg = parse_cfg(&vfs.read_string(&cfg_path).ok()?);
    out.mesh_path = cfg
        .mesh
        .as_deref()
        .and_then(|m| resolve_relative(&cfg_path, m, "models", exists));
    out.cfg_path = Some(cfg_path);
    out.unit = cfg.unit;
    out.cfg_length = cfg.length;

    let presets = load_material_presets(vfs);
    for mat in &cfg.materials {
        if let Some(p) = presets.iter().find(|p| p.name.eq_ignore_ascii_case(mat)) {
            if let Some(d) = &p.diff_map {
                out.textures.push((mat.clone(), d.clone()));
            }
            if let Some(n) = &p.norm_map {
                out.normal_maps.push((mat.clone(), n.clone()));
            }
            out.presets.push(p.clone());
        }
    }
    out.materials = cfg.materials;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EROS_SSS: &str = r#"
Name    "Eros"
Type    "BodyModel"
Length   35000
Offset  (0 0 0)
Quat    (0.70710678 0.70710678 0 0)
Mass     6.687e+15
Module  "Main"  { "asteroids/Eros.cfg" (16 0 0 0 0 16 0 0 0 0 16 0 0 0 0 1) }
"#;

    const EROS_CFG: &str = "Unit     1\nLength   33381\nMesh     \"planets/asteroids/Eros.sm\"\nMaterial \"Asteroid_Eros\"\n";

    const SML: &str = r#"
Material	"Asteroid_default"
{
	DiffColor       (0.14 0.14 0.14)
}

Material	"Asteroid_Eros"
{
    DiffColor       (0.4 0.4 0.4)
    DiffMap         "asteroids/Eros_albedo.png"
    NormMap         "asteroids/Eros_normal.png"
    DetDiffMap      "asteroids/GroundKX3_COL.png"
}
"#;

    #[test]
    fn sss_parses_module_and_placement() {
        let s = parse_sss(EROS_SSS);
        assert_eq!(s.name.as_deref(), Some("Eros"));
        assert_eq!(s.length, Some(35000.0));
        assert_eq!(s.offset, Some([0.0, 0.0, 0.0]));
        assert_eq!(s.quat, Some([0.70710678, 0.70710678, 0.0, 0.0]));
        assert_eq!(s.modules.len(), 1);
        assert_eq!(s.modules[0].id, "Main");
        assert_eq!(s.modules[0].cfg, "asteroids/Eros.cfg");
        let m = s.modules[0].matrix.unwrap();
        assert_eq!(m[0], 16.0);
        assert_eq!(m[5], 16.0);
        assert_eq!(m[15], 1.0);
    }

    #[test]
    fn sss_module_without_matrix_has_none() {
        let s = parse_sss("Module \"Main\" { \"asteroids/X.cfg\" }\n");
        assert_eq!(s.modules.len(), 1);
        assert_eq!(s.modules[0].cfg, "asteroids/X.cfg");
        assert!(s.modules[0].matrix.is_none());
    }

    #[test]
    fn cfg_parses_unit_mesh_and_materials() {
        let c = parse_cfg(EROS_CFG);
        assert_eq!(c.unit, Some(1.0));
        assert_eq!(c.length, Some(33381.0));
        assert_eq!(c.mesh.as_deref(), Some("planets/asteroids/Eros.sm"));
        assert_eq!(c.materials, vec!["Asteroid_Eros".to_string()]);
    }

    #[test]
    fn cfg_keeps_material_order() {
        let c = parse_cfg("Material \"A\"\nMaterial \"B\"\nMaterial \"C\"\n");
        assert_eq!(c.materials, vec!["A", "B", "C"]);
        assert!(c.mesh.is_none());
    }

    #[test]
    fn presets_parse_diffuse_and_normal() {
        let p = parse_material_presets(SML);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "Asteroid_default");
        assert!(p[0].diff_map.is_none());
        assert_eq!(p[1].name, "Asteroid_Eros");
        assert_eq!(p[1].diff_map.as_deref(), Some("asteroids/Eros_albedo.png"));
        assert_eq!(p[1].norm_map.as_deref(), Some("asteroids/Eros_normal.png"));
    }

    #[test]
    fn presets_parse_colours_and_detail_maps() {
        let p = parse_material_presets(
            "Material \"Comet_67P\"\n{\n DiffColorScale (0.10 0.10 0.10)\n SpecColor (0.25 0.25 0.25)\n SpecPower 120.0\n \
             DetDiffMap \"asteroids/LavaFlow002_COL.png\"\n DetBumpMap \"asteroids/LavaFlow002_DISP.png\"\n DetDiffScale 512\n \
             DetBumpScale 512\n DetDiffMag 0.75\n DetBumpMag 0.5\n DetBumpHeight 8.0\n}\n\
             Material \"Asteroid_2014MU69\"\n{\n DiffColor (0.1 0.1 0.1)\n BumpMap \"asteroids/2014MU69_bump.png\"\n BumpHeight 8.0\n}\n",
        );
        let c = &p[0];
        assert_eq!((c.diff_color_scale, c.diff_color), (Some([0.1, 0.1, 0.1]), None));
        assert_eq!((c.spec_color, c.spec_power), (Some([0.25, 0.25, 0.25]), Some(120.0)));
        assert_eq!(c.det_diff_map.as_deref(), Some("asteroids/LavaFlow002_COL.png"));
        assert_eq!(c.det_bump_map.as_deref(), Some("asteroids/LavaFlow002_DISP.png"));
        assert_eq!((c.det_diff_scale, c.det_bump_scale), (Some(512.0), Some(512.0)));
        assert_eq!((c.det_diff_mag, c.det_bump_mag, c.det_bump_height), (Some(0.75), Some(0.5), Some(8.0)));
        assert_eq!((p[1].bump_map.as_deref(), p[1].bump_height), (Some("asteroids/2014MU69_bump.png"), Some(8.0)));
        let s = ModelScript { materials: vec!["comet_67p".into()], presets: p.clone(), ..Default::default() };
        assert_eq!(s.preset_for_material(0).map(|p| p.name.as_str()), Some("Comet_67P"));
        assert!(s.preset_for_material(1).is_none());
    }

    #[test]
    fn join_path_resolves_dots() {
        assert_eq!(join_path("models", "planets/Asteroids/67P.sss"), "models/planets/Asteroids/67P.sss");
        assert_eq!(join_path("models/planets/Asteroids", "../x.cfg"), "models/planets/x.cfg");
        assert_eq!(join_path("a/b", "./c\\d.sm"), "a/b/c/d.sm");
        assert_eq!(join_path("", "x"), "x");
    }

    #[test]
    fn resolve_tries_dir_then_parent_then_root() {
        let files = [
            "models/planets/asteroids/67p.cfg",
            "models/planets/asteroids/67p.sm",
        ];
        let exists = |p: &str| files.iter().any(|f| f.eq_ignore_ascii_case(p));
        // The .sss names its cfg relative to models/planets/ (the parent of its own dir).
        assert_eq!(
            resolve_relative("models/planets/Asteroids/67P.sss", "asteroids/67P.cfg", "models", exists).as_deref(),
            Some("models/planets/asteroids/67P.cfg")
        );
        // The .cfg names its mesh relative to the models/ root.
        assert_eq!(
            resolve_relative("models/planets/asteroids/67P.cfg", "planets/asteroids/67P.sm", "models", exists)
                .as_deref(),
            Some("models/planets/asteroids/67P.sm")
        );
        assert!(resolve_relative("models/x.sss", "nope.cfg", "models", exists).is_none());
    }

    #[test]
    fn resolve_prefers_file_directory() {
        let exists = |p: &str| p == "textures/a/b/t.png" || p == "textures/a/t.png";
        assert_eq!(
            resolve_relative("textures/a/b/m.sml", "t.png", "textures", exists).as_deref(),
            Some("textures/a/b/t.png")
        );
    }

    #[test]
    fn diffuse_lookup_by_object_material_index() {
        let s = ModelScript {
            materials: vec!["Asteroid_Eros".into(), "Other".into()],
            textures: vec![("Asteroid_Eros".into(), "textures/planets/Asteroids/Eros_albedo.png".into())],
            ..Default::default()
        };
        assert_eq!(s.diffuse_for_material(0), Some("textures/planets/Asteroids/Eros_albedo.png"));
        assert_eq!(s.diffuse_for_material(1), None);
        assert_eq!(s.diffuse_for_material(7), None);
    }
}
