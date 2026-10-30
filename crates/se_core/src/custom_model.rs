//! Ray-marched nebula models: the `CustomModel` blocks of `models/custom/*.cfg` (`Nebulae-raymarch.pak`,
//! docs/formats/nebula_raymarch.md).
//!
//! The exe (`seCustomModel::ReadFromScript`) reads the generic keys (`UseForObject`, `UseForType`, `Shader`,
//! `BaseShape`, `BaseShapeDims`, `Scale`, `Randomize`, `Bright`, `ParticleColor`, `Expansion*`) and hands
//! every other key to the shader as a uniform of the same name; the shader (`nebula_raymarch.glsl`) is in
//! the encrypted `Shaders.pak`, so the port keeps all keys by name for its own ray marcher.
//!
//! Which nebula gets which model (`FUN_140486cb0`, `FUN_140270120`): a model naming the nebula in
//! `UseForObject` wins; nebulae left without any model (no named sprite model either) take
//! `pool[h % len]` from the custom models of their type, with the same position hash `h` as the sprite
//! pools (`galaxy_model::nebula_position_hash`). The exe's switch between the sprite and the custom pools
//! (`DAT_140d8dcd0`) is constant 1: unnamed catalog nebulae are always ray-marched.

use std::collections::HashMap;

use crate::galaxy_model::nebula_position_hash;
use crate::sc::{self, Block};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseShape {
    Box,
    Sphere,
    Cylinder,
}

#[derive(Clone, Debug)]
pub struct CustomModel {
    pub name: String,
    pub use_for_object: Vec<String>,
    /// `UseForType` without the `Nebula/` prefix (`Diffuse`, `Planetary`, `SNR`, `NONE`).
    pub use_for_type: Option<String>,
    pub shape: BaseShape,
    /// Bounding shape size in internal units (box: half extents; sphere: x is the radius).
    pub dims: [f64; 3],
    /// Stretch of the model along its axes.
    pub scale: [f64; 3],
    pub bright: f64,
    pub particle_color: [f64; 3],
    /// Every key of the block, last value wins (the shader uniforms).
    params: Block,
}

impl CustomModel {
    pub fn num(&self, key: &str) -> f64 {
        self.params.f64(key).unwrap_or(0.0)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.params.bool(key).unwrap_or(false)
    }

    /// A `(dust gas)` pair, or a 2-vector.
    pub fn pair(&self, key: &str) -> [f64; 2] {
        match self.params.nums(key) {
            Some(v) if v.len() >= 2 => [v[0], v[1]],
            Some(v) if v.len() == 1 => [v[0], v[0]],
            _ => [0.0; 2],
        }
    }

    pub fn v3(&self, key: &str) -> [f64; 3] {
        self.params.vec3(key).unwrap_or([0.0; 3])
    }

    pub fn v4(&self, key: &str) -> [f64; 4] {
        self.params.vec4(key).unwrap_or([0.0; 4])
    }

    /// The internal-unit half size that maps to the nebula radius.
    pub fn internal_scale(&self) -> f64 {
        let s = self.num("Internal_scale");
        if s > 0.0 { s } else { self.dims[0].max(self.dims[1]).max(self.dims[2]).max(1.0) }
    }

    /// Radius of a sphere around the bounding shape, in nebula radii (after `Scale`).
    pub fn bound_radius(&self) -> f64 {
        let i = self.internal_scale();
        let e = |k: usize| (self.dims[k] * self.scale[k]).abs() / i;
        match self.shape {
            BaseShape::Sphere => self.dims[0].abs() * self.scale.iter().fold(0.0f64, |a, &s| a.max(s.abs())) / i,
            BaseShape::Cylinder => (e(0).powi(2) + e(1).powi(2)).sqrt().max(e(0)),
            BaseShape::Box => (e(0).powi(2) + e(1).powi(2) + e(2).powi(2)).sqrt(),
        }
    }
}

/// Parses every `CustomModel` block of a script.
pub fn parse_custom_models(text: &str) -> Vec<CustomModel> {
    let doc = sc::parse(text);
    doc.items
        .iter()
        .filter(|i| i.key.eq_ignore_ascii_case("CustomModel"))
        .filter_map(|item| {
            let b = item.block.clone()?;
            let shape = match b.str("BaseShape").map(str::to_ascii_lowercase).as_deref() {
                Some("sphere") => BaseShape::Sphere,
                Some("cylinder") => BaseShape::Cylinder,
                _ => BaseShape::Box,
            };
            let use_for_object = b
                .all("UseForObject")
                .flat_map(|i| i.args.iter().filter_map(|a| a.as_str().map(str::to_string)).collect::<Vec<_>>())
                .collect();
            let use_for_type =
                b.str("UseForType").map(|t| t.rsplit('/').next().unwrap_or(t).to_string());
            Some(CustomModel {
                name: item.name().unwrap_or("").to_string(),
                use_for_object,
                use_for_type,
                shape,
                dims: b.vec3("BaseShapeDims").unwrap_or([1.0; 3]),
                scale: b.vec3("Scale").unwrap_or([1.0; 3]),
                bright: b.f64("Bright").unwrap_or(1.0),
                particle_color: b.vec3("ParticleColor").unwrap_or([1.0; 3]),
                params: b,
            })
        })
        .collect()
}

/// All custom models in load order, with the name and type lookups.
#[derive(Default)]
pub struct CustomModelSet {
    pub models: Vec<CustomModel>,
    named: HashMap<String, usize>,
    pools: HashMap<String, Vec<usize>>,
}

impl CustomModelSet {
    /// `files`: (path, text) in the exe's load order (file names sorted).
    pub fn new(mut files: Vec<(String, String)>) -> Self {
        files.sort_by_key(|(p, _)| p.to_ascii_lowercase());
        let mut set = Self::default();
        for (_, text) in &files {
            for m in parse_custom_models(text) {
                let i = set.models.len();
                for n in &m.use_for_object {
                    set.named.entry(n.to_ascii_lowercase()).or_insert(i);
                }
                if let Some(t) = &m.use_for_type {
                    set.pools.entry(t.to_ascii_lowercase()).or_default().push(i);
                }
                set.models.push(m);
            }
        }
        set
    }

    /// The model named for one of `names` (aliases included).
    pub fn named(&self, names: &[String]) -> Option<usize> {
        names.iter().find_map(|n| self.named.get(&n.to_ascii_lowercase()).copied())
    }

    /// The pool model of a nebula of type `kind` at `pos_pc`.
    pub fn pooled(&self, kind: &str, pos_pc: [f64; 3]) -> Option<usize> {
        let pool = self.pools.get(&kind.to_ascii_lowercase()).filter(|p| !p.is_empty())?;
        Some(pool[(nebula_position_hash(pos_pc) % pool.len() as u32) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"
LogMissingParameters false
CustomModel "Diff 5"
{
	UseForType		"Nebula/Diffuse"
	BaseShape      "box"
	BaseShapeDims  (18 18 18)
	Bright          3
	Internal_scale 18
	SpirNoiseNudge (6.7 10)
	Dust_enabled true
	ParticleColor  (1 0.9 0.8)
}
CustomModel "spirograph"
{
	UseForType		"Nebula/Planetary"
	UseForObject	"Spirograph Nebula"
	BaseShape      "sphere"
	BaseShapeDims  (23.1013 16.4012 0.378889)
	Scale          (0.766121 0.892099 0.766121)
	Internal_scale 29.1157
	ParticleColor  (0.0273438 0.0881343 1)
	ParticleColor	(1, 0.5, 0.1)
}
"#;

    #[test]
    fn parses_models_and_assigns_them() {
        let set = CustomModelSet::new(vec![("b.cfg".into(), CFG.into())]);
        assert_eq!(set.models.len(), 2);
        let d = &set.models[0];
        assert_eq!((d.shape, d.bright, d.pair("SpirNoiseNudge")), (BaseShape::Box, 3.0, [6.7, 10.0]));
        assert!(d.flag("Dust_enabled"));
        assert!((d.bound_radius() - 3f64.sqrt()).abs() < 1e-9);
        let s = &set.models[1];
        // The last ParticleColor wins; the sphere's radius is the first dimension.
        assert_eq!(s.particle_color, [1.0, 0.5, 0.1]);
        assert!((s.bound_radius() - 23.1013 * 0.892099 / 29.1157).abs() < 1e-9);
        assert_eq!(set.named(&["NGC 2022".into(), "Spirograph Nebula".into()]), Some(1));
        assert_eq!(set.pooled("Diffuse", [1.0, 2.0, 3.0]), Some(0));
        assert_eq!(set.pooled("SNR", [1.0, 2.0, 3.0]), None);
    }
}
