//! Typed view over SpaceEngine's catalogs (`catalogs/**/*.sc` and `*.csv`).
//!
//! Later definitions of the same object override earlier ones (SE uses this for
//! "-fix" catalogs). Positions are in parsecs in the universe frame relative to the
//! Solar System barycentre.

use std::collections::HashMap;

use glam::{DQuat, DVec3};

use crate::coords::{
    radec_to_universe, se_axis_angle_to_model_rotation, se_galaxy_quat_to_model_rotation,
};
use crate::orbit::KeplerOrbit;
use crate::rotation::RotationModel;
use crate::sc::{self, Block, Item};
use crate::star::{self, SpectralClass, StarKind};
use crate::units::{EARTH_MASS_KG, JUPITER_MASS_KG, SOLAR_MASS_KG, SOLAR_RADIUS_KM};
use crate::vfs::Vfs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyTag {
    Planet,
    DwarfPlanet,
    Moon,
    DwarfMoon,
    Asteroid,
    Comet,
    Spacecraft,
}

impl BodyTag {
    fn parse(tag: &str) -> Option<Self> {
        Some(match tag {
            "Planet" => BodyTag::Planet,
            "DwarfPlanet" => BodyTag::DwarfPlanet,
            "Moon" => BodyTag::Moon,
            "DwarfMoon" => BodyTag::DwarfMoon,
            "Asteroid" => BodyTag::Asteroid,
            "Comet" => BodyTag::Comet,
            "Spacecraft" => BodyTag::Spacecraft,
            _ => return None,
        })
    }
}

impl StarRecord {
    /// Bolometric luminosity (L_sun): the catalog's `LumBol`/`Bolometric`, else from the absolute
    /// magnitude and a bolometric correction for `teff`.
    pub fn lum_bol(&self) -> f64 {
        self.block
            .f64_any(&["LumBol", "Bolometric"])
            .filter(|l| *l > 0.0)
            .unwrap_or_else(|| star::lum_from_abs_mag(self.abs_mag) * 10f64.powf(-0.4 * star::bolometric_correction(self.teff)))
    }

    /// The star's ellipsoid semi-axes (+Y the pole) relative to `radius_km`, read as SE's object
    /// parser does (FUN_140468860): `Dimensions` (km) or `DimensionsSol` (solar radii) diameters
    /// over the largest one, else `Oblateness` (the polar flattening, or a flattening per axis).
    pub fn shape(&self) -> [f64; 3] {
        let b = &self.block;
        if let Some(d) = b.vec3("Dimensions").or_else(|| b.vec3("DimensionsSol")) {
            let max = d[0].max(d[1]).max(d[2]);
            if max > 0.0 {
                return d.map(|x| (x / max).clamp(0.01, 1.0));
            }
        }
        if let Some(f) = b.vec3("Oblateness") {
            return f.map(|x| (1.0 - x).clamp(0.01, 1.0));
        }
        [1.0, (1.0 - b.f64("Oblateness").unwrap_or(0.0)).clamp(0.01, 1.0), 1.0]
    }

    /// `Hidden true` (FUN_140476e90 sets flag 0x4000000): kept out of the sky and the star lists, as
    /// SE's main-menu backdrop "Menu Star".
    pub fn hidden(&self) -> bool {
        self.block.bool("Hidden") == Some(true)
    }

    /// `ForceAppMagn true` (FUN_140476e90 sets flag 0x20000000): the catalog's `AppMagn` is the
    /// star's or system's brightness, whatever its luminosity or its components give.
    pub fn force_app_magn(&self) -> bool {
        self.block.bool("ForceAppMagn") == Some(true)
    }
}

/// A star's radius (km) from its `Dimensions` (km) or `DimensionsSol` (solar radii) diameters: half
/// the largest (FUN_140468860).
fn radius_from_dimensions(b: &Block) -> Option<f64> {
    let (d, unit) = match b.vec3("Dimensions") {
        Some(d) => (d, 1.0),
        None => (b.vec3("DimensionsSol")?, SOLAR_RADIUS_KM),
    };
    let max = d[0].max(d[1]).max(d[2]);
    (max > 0.0).then_some(0.5 * max * unit)
}

#[derive(Debug, Clone)]
pub struct StarRecord {
    pub names: Vec<String>,
    /// `StarBarycenter` / `Barycenter` entries have no light of their own.
    pub barycenter: bool,
    pub parent: Option<String>,
    /// Position in parsecs (universe frame). For components of a system without
    /// their own RA/Dec this is resolved to the parent's position.
    pub pos_pc: DVec3,
    pub has_position: bool,
    pub class: SpectralClass,
    pub abs_mag: f64,
    pub teff: f64,
    pub radius_km: f64,
    pub mass_kg: f64,
    pub orbit: Option<KeplerOrbit>,
    pub rotation: RotationModel,
    pub source: String,
    pub block: Block,
}

#[derive(Debug, Clone)]
pub struct BodyRecord {
    pub tag: BodyTag,
    pub names: Vec<String>,
    pub parent: String,
    pub class: Option<String>,
    pub radius_km: f64,
    pub mass_kg: f64,
    pub oblateness: f64,
    pub color: Option<[f64; 3]>,
    pub albedo: f64,
    pub orbit: Option<KeplerOrbit>,
    pub rotation: RotationModel,
    /// `Moon-DE` or `TidalLocked` (in place of `rotation`).
    pub lock: crate::rotation::RotationLock,
    pub source: String,
    /// Full definition (Surface, Atmosphere, Clouds, Rings, Ocean, ...).
    pub block: Block,
}

impl BodyRecord {
    /// `FixedPosPolar (lon lat dist)`: a body fixed on its parent (degrees, and km from the parent's
    /// centre), turning with it, as SE's catalog spacecraft (ApolloLandingSites.sc's landers).
    pub fn fixed_pos_polar(&self) -> Option<[f64; 3]> {
        self.block.vec3("FixedPosPolar")
    }

    /// SE draws a body as an ellipsoid: its semi-axes in the body frame (+Y the pole) relative to
    /// `radius_km`, from `Dimensions` (diameters, km) or `Oblateness` (the polar flattening, or a
    /// flattening per axis).
    pub fn shape(&self) -> [f64; 3] {
        let b = &self.block;
        if let (Some(d), true) = (b.vec3("Dimensions"), self.radius_km > 0.0) {
            return d.map(|x| (x * 0.5 / self.radius_km).max(0.01));
        }
        if let Some(f) = b.vec3("Oblateness") {
            return f.map(|x| (1.0 - x).clamp(0.01, 1.0));
        }
        [1.0, (1.0 - self.oblateness).clamp(0.01, 1.0), 1.0]
    }
}

#[derive(Debug, Clone)]
pub struct GalaxyRecord {
    pub names: Vec<String>,
    /// Hubble type string (`"SBc"`, `"E3"`, `"Irr"`...).
    pub kind: String,
    pub quasar: bool,
    pub pos_pc: DVec3,
    pub radius_pc: f64,
    pub abs_mag: f64,
    /// Magnitude the galaxy is drawn with: `abs_mag` for catalog galaxies; for procedural ones
    /// the light SE's radius law gives their size (see `ProcGalaxy::vis_abs_mag`).
    pub vis_mag: f64,
    /// Model space (disk normal = -Z) to universe.
    pub orientation: DQuat,
    pub model_bright: f64,
    pub source: String,
    /// Procedural galaxies: raw draw of the sprite-model pool pick (`pool[type][roll % len]`).
    pub model_roll: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ClusterRecord {
    pub names: Vec<String>,
    pub galaxy: String,
    pub kind: String,
    pub pos_pc: DVec3,
    pub radius_pc: f64,
    pub abs_mag: f64,
    pub age_gyr: f64,
    /// SE's concentration key `CenPow` (FUN_14047fcb0), or its default draw (FUN_1404826a0).
    pub cen_pow: f64,
    /// `NStars 0`: no generated members, the stars are in the star catalog (FUN_14047fcb0 sets a
    /// flag; any other `NStars` is ignored, class init FUN_1404826a0 zeroes it).
    pub no_members: bool,
    /// Colour of the far star-like particle: the catalog `Color`, `None` for SE's default by type.
    /// `Color (0 0 0)` means no particle (the stars are in the star catalog).
    pub color: Option<[f32; 3]>,
    pub source: String,
}

/// SE's `ClusterGenerator`/`StarGenerator` values used to complete catalog clusters. The catalog cap
/// is the compiled default of `ClusterCatMaxAppMagn`, 7.1 (`DAT_14075f798`), not main-def.cfg's 8.1:
/// SE turns it into its brightness constant once, in a static initializer (FUN_140011160), before the
/// config is read.
const CLUSTER_CAT_MAX_APP_MAGN: f64 = 7.1;
const STAR_DENSITY_IN_CLUSTER: f64 = 1.0;

/// SE's cluster types (`FUN_140483080`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterType {
    Kern,
    Globular,
    Open,
    Part,
    Other,
}

impl ClusterType {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "KERN" => Self::Kern,
            "GLOBULAR" => Self::Globular,
            "OPEN" => Self::Open,
            "PART" => Self::Part,
            _ => Self::Other,
        }
    }
}

/// Truncated normal draw (retries, then clamps).
fn trunc_normal(rng: &mut crate::star_octree::Rng, mean: f64, sd: f64, lo: f64, hi: f64) -> f64 {
    for _ in 0..32 {
        let (u1, u2) = (rng.f64().max(1e-12), rng.f64());
        let v = mean + sd * (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        if (lo..=hi).contains(&v) {
            return v;
        }
    }
    mean.clamp(lo, hi)
}

/// Radius (pc), absolute magnitude, age (Gyr) and `CenPow` of a catalog cluster, completed as SE does
/// where the catalog leaves them out (loader `FUN_14047fcb0`, class init `FUN_1404826a0`). SE draws
/// the missing values from its random stream; here the draws are seeded by the cluster's name.
fn complete_cluster(b: &Block, kind: ClusterType, name: &str, dist_pc: f64) -> (f64, f64, f64, f64) {
    let mut rng = crate::star_octree::Rng(name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, c| (h ^ c as u64).wrapping_mul(0x100_0000_01b3)));
    let radius = b.f64("Radius").unwrap_or_else(|| match kind {
        ClusterType::Globular => trunc_normal(&mut rng, 30.0, 10.0, 5.0, 70.0),
        ClusterType::Open | ClusterType::Part => trunc_normal(&mut rng, 2.0, 2.0, 0.5, 15.0),
        ClusterType::Kern => trunc_normal(&mut rng, 20.0, 5.0, 5.0, 40.0),
        ClusterType::Other => 5.0,
    });
    let lum = b.f64_any(&["Lum", "Luminosity"]);
    let abs_mag = b
        .f64("AbsMagn")
        .or_else(|| b.f64("AppMagn").map(|m| star::abs_mag_from_app(m, dist_pc)))
        .or_else(|| lum.map(|l| 4.83 - 2.5 * l.max(1e-9).log10()))
        .unwrap_or_else(|| {
            let l = match kind {
                // Open clusters without a magnitude: 10 R^3 x density, never brighter than
                // ClusterCatMaxAppMagn seen from the Sun (Pozzo 1: about 114 Lsun).
                ClusterType::Open | ClusterType::Part => {
                    let l = (10.0 * radius.powi(3) * STAR_DENSITY_IN_CLUSTER).clamp(1000.0, 2.0e5);
                    l.min(dist_pc * dist_pc * (-(CLUSTER_CAT_MAX_APP_MAGN + 0.2) / 1.0587362).exp())
                }
                ClusterType::Globular => return trunc_normal(&mut rng, -7.45, 0.95, -11.0, -1.0),
                ClusterType::Kern => trunc_normal(&mut rng, 2000.0, 1000.0, 500.0, 20000.0),
                ClusterType::Other => return -5.0,
            };
            4.83 - 2.5 * l.log10()
        });
    let age = b.f64("Age").unwrap_or_else(|| match kind {
        ClusterType::Globular => trunc_normal(&mut rng, 11.0, 3.0, 0.0, 13.0),
        ClusterType::Open | ClusterType::Part => trunc_normal(&mut rng, 0.15, 0.1, 0.001, 3.5),
        ClusterType::Kern => trunc_normal(&mut rng, 0.1, 0.03, 0.01, 13.0),
        ClusterType::Other => 1.0,
    });
    let cen_pow = b.f64("CenPow").unwrap_or_else(|| crate::cluster_stars::default_cen_pow(&mut rng, kind == ClusterType::Kern));
    (radius, abs_mag, age, cen_pow)
}

#[derive(Debug, Clone)]
pub struct NebulaRecord {
    pub names: Vec<String>,
    pub galaxy: String,
    pub kind: String,
    pub pos_pc: DVec3,
    pub radius_pc: f64,
    pub abs_mag: f64,
    pub orientation: DQuat,
    pub source: String,
    /// Procedural nebulae: raw draw of the sprite-model pool pick (`pool[type][roll % len]`).
    pub model_roll: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectRef {
    Star(u32),
    Body(u32),
    Galaxy(u32),
    Cluster(u32),
    Nebula(u32),
}

#[derive(Debug, Default, Clone)]
pub struct Catalog {
    pub stars: Vec<StarRecord>,
    pub bodies: Vec<BodyRecord>,
    pub galaxies: Vec<GalaxyRecord>,
    pub clusters: Vec<ClusterRecord>,
    pub nebulae: Vec<NebulaRecord>,
    /// Catalog galaxy index -> its `GlobClusters` (the galaxies that set it).
    pub glob_clusters: HashMap<u32, i32>,
    /// Lower-cased name -> object (every alias is indexed).
    names: HashMap<String, ObjectRef>,
    /// Names two objects share but for case ("Kepler-34 b" the planet, "Kepler-34 B" the star),
    /// exactly as written; `find` tries these first, as SE keeps such objects apart.
    exact: HashMap<String, ObjectRef>,
    /// Lower-cased parent name -> child bodies/stars.
    children: HashMap<String, Vec<ObjectRef>>,
    pub warnings: Vec<String>,
    /// JPL planetary ephemeris for bodies with `AnalyticModel "X-DE"` orbits.
    pub de: Option<std::sync::Arc<crate::jpl_de::JplEphemeris>>,
    /// Analytic moon theories (GUST86, L1, TASS 1.7, ...) read from `SpaceEngine.exe`;
    /// used for moons whose orbit names an `AnalyticModel`.
    pub moon_theories: Option<std::sync::Arc<crate::moon_theories::MoonTheories>>,
    /// Gas species and XUV tables read from `SpaceEngine.exe` for SE's planet completion.
    pub climate: Option<std::sync::Arc<crate::climate::ClimateTables>>,
    /// Star and body counts covered by the last `resolve` (`finalize` only handles newer ones).
    resolved: (usize, usize),
}

/// The source of a record [`Catalog::unload_generated`] emptied.
pub const UNLOADED_SOURCE: &str = "unloaded";

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// RA/Dec/Dist of a block, if present. Dist in parsecs (or `Parallax` in mas).
fn block_position_pc(b: &Block) -> Option<DVec3> {
    let ra = b.sexagesimal("RA")?;
    let dec = b.sexagesimal("Dec")?;
    let dist = b
        .f64("Dist")
        .or_else(|| b.f64("Parallax").filter(|p| *p > 0.0).map(|p| 1000.0 / p))?;
    Some(radec_to_universe(ra, dec) * dist)
}

/// The `Orbit` block of an object, or its `BinaryOrbit` (elements shared by both members of a binary).
fn orbit_block(b: &Block) -> Option<KeplerOrbit> {
    b.sub("Orbit").and_then(KeplerOrbit::from_block).or_else(|| {
        let mut o = KeplerOrbit::from_block(b.sub("BinaryOrbit")?)?;
        o.binary = true;
        Some(o)
    })
}

fn mass_kg(b: &Block) -> Option<f64> {
    b.f64("Mass")
        .map(|m| m * EARTH_MASS_KG)
        .or_else(|| b.f64("MassSol").map(|m| m * SOLAR_MASS_KG))
        .or_else(|| b.f64("MassJup").map(|m| m * JUPITER_MASS_KG))
        .or_else(|| b.f64("Msini").map(|m| m * EARTH_MASS_KG))
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load every catalog under `catalogs/` from the VFS.
    pub fn load(vfs: &Vfs) -> Self {
        let mut cat = Catalog::new();
        // Stars first so planets can resolve their parents' masses.
        for dir in ["catalogs/stars", "catalogs/planets"] {
            for path in vfs.list_ext(dir, "csv") {
                match vfs.read_string(&path) {
                    Ok(text) => cat.load_star_csv(&text, &path),
                    Err(e) => cat.warnings.push(format!("{path}: {e}")),
                }
            }
            for path in vfs.list_ext(dir, "sc") {
                match vfs.read_string(&path) {
                    Ok(text) => cat.load_sc(&text, &path),
                    Err(e) => cat.warnings.push(format!("{path}: {e}")),
                }
            }
        }
        for path in vfs.list_ext("catalogs/galaxies", "sc") {
            if let Ok(text) = vfs.read_string(&path) {
                cat.load_sc(&text, &path);
            }
        }
        for path in vfs.list_ext("catalogs/galaxies", "csv") {
            if let Ok(text) = vfs.read_string(&path) {
                cat.load_galaxy_csv(&text, &path);
            }
        }
        for dir in ["catalogs/clusters", "catalogs/nebulae"] {
            for path in vfs.list_ext(dir, "sc") {
                if let Ok(text) = vfs.read_string(&path) {
                    cat.load_sc(&text, &path);
                }
            }
        }
        cat.resolve();
        // Catalog spacecraft (`Model "spacecraft/....sss"`) without a radius take half their design's
        // `Length` (SE builds them as static ships, "Initializing static ships").
        for b in cat.bodies.iter_mut().filter(|b| b.tag == BodyTag::Spacecraft && b.radius_km <= 0.0) {
            if let Some(d) = b.block.str("Model").and_then(|m| crate::ship::ShipDesign::load_header(vfs, m)) {
                b.radius_km = d.length_m * 0.5e-3;
            }
        }
        cat
    }

    /// Parse and add one `.sc` document.
    pub fn load_sc(&mut self, text: &str, source: &str) {
        self.load_doc(sc::parse(text), source);
    }

    /// [`Catalog::load_sc`] for a script already parsed (off the main thread, say).
    pub fn load_doc(&mut self, doc: sc::Block, source: &str) {
        for item in doc.items {
            // SE's star reader treats `Remove` as `RemoveStar` (FUN_140475fb0), used before a
            // system is redefined (Albireo's BET1/BET2 Cyg).
            if item.key == "RemoveStar" || item.key == "Remove" {
                for n in item.names() {
                    self.remove_star(&n);
                }
                continue;
            }
            let Some(block) = item.block.as_ref() else { continue };
            if item.names().is_empty() {
                continue;
            }
            match item.key.as_str() {
                "Star" | "StarBarycenter" | "Barycenter" => self.add_star(&item, block, source),
                "Galaxy" | "Quasar" => self.add_galaxy(&item, block, source),
                "Cluster" => self.add_cluster(&item, block, source),
                "Nebula" => self.add_nebula(&item, block, source),
                tag => {
                    if let Some(t) = BodyTag::parse(tag) {
                        self.add_body(t, &item, block, source);
                    }
                }
            }
        }
    }

    fn register(&mut self, names: &[String], r: ObjectRef) {
        for n in names {
            let k = key(n);
            if let Some(&old) = self.names.get(&k)
                && old != r
            {
                if let Some(o) = self.names_of(old).iter().find(|o| key(o) == k) {
                    self.exact.insert(o.trim().to_string(), old);
                }
                self.exact.insert(n.trim().to_string(), r);
            }
            self.names.insert(k, r);
        }
    }

    /// `RemoveStar "name"`: drop a star (usually a csv entry that an .sc file replaces with
    /// a barycenter and components). The record stays in place so indices remain valid, but
    /// it loses its names and position, which hides it everywhere.
    fn remove_star(&mut self, name: &str) {
        let Some(ObjectRef::Star(i)) = self.names.get(&key(name)).copied() else { return };
        let s = &mut self.stars[i as usize];
        s.has_position = false;
        s.parent = None;
        s.orbit = None;
        let names = std::mem::take(&mut s.names);
        for n in &names {
            if self.names.get(&key(n)) == Some(&ObjectRef::Star(i)) {
                self.names.remove(&key(n));
            }
        }
        self.exact.retain(|_, r| *r != ObjectRef::Star(i));
        self.stars[i as usize].names = names;
    }

    /// If an object with this primary name already exists, return its index for override.
    fn existing(&self, names: &[String]) -> Option<ObjectRef> {
        names.first().and_then(|n| self.names.get(&key(n)).copied())
    }

    fn add_star(&mut self, item: &Item, b: &Block, source: &str) {
        let names = item.names();
        let barycenter = item.key != "Star";
        let class = SpectralClass::parse(b.str("Class").unwrap_or(if barycenter { "" } else { "G2V" }));
        let teff = b
            .f64_any(&["Teff", "Temperature", "Temp"])
            .unwrap_or_else(|| class.temperature());
        let app_mag = b.f64_any(&["AppMagn", "AppMag", "AppMagnV"]);
        // Absolute magnitude from intrinsic parameters only (no distance needed).
        let intrinsic_abs = if let Some(m) = b.f64("AbsMagn") {
            Some(m)
        } else if let Some(l) = b.f64_any(&["Lum", "Luminosity"]) {
            Some(star::abs_mag_from_lum(l))
        } else if let Some(lb) = b.f64("LumBol") {
            Some(star::abs_bol_mag_from_lum(lb) - star::bolometric_correction(teff))
        } else if let (Some(r_km), true) = (
            b.f64("Radius").or_else(|| b.f64_any(&["RadSol", "RadiusSol"]).map(|r| r * SOLAR_RADIUS_KM)),
            teff > 0.0,
        ) {
            // Stefan-Boltzmann from radius and temperature.
            let lum_bol = (r_km / SOLAR_RADIUS_KM).powi(2) * (teff / crate::units::SOLAR_TEFF).powi(4);
            Some(star::abs_bol_mag_from_lum(lum_bol) - star::bolometric_correction(teff))
        } else if !barycenter || b.str("Class").is_some() {
            Some(class.typical_abs_mag())
        } else {
            None
        };
        // RA/Dec without Dist or Parallax: photometric distance from the absolute
        // magnitude (several catalog stars and barycenters give only RA, Dec, AppMagn).
        let pos = block_position_pc(b).or_else(|| {
            let (ra, dec, abs_v) = (b.sexagesimal("RA")?, b.sexagesimal("Dec")?, intrinsic_abs?);
            // V band, else J band (brown dwarfs): M_J = M_bol - BC_J with BC_J ~ 1.9.
            let (m, abs) = match (app_mag, b.f64("AppMagnJ")) {
                (Some(m), _) => (m, abs_v),
                (None, Some(j)) => (j, abs_v + star::bolometric_correction(teff) - 1.9),
                _ => return None,
            };
            Some(radec_to_universe(ra, dec) * 10f64.powf((m - abs + 5.0) / 5.0))
        });
        let dist = pos.map(|p| p.length());
        // SE (FUN_140476e90) fills the visual luminosity from `Lum`/`Luminosity`, then `AbsMagn`;
        // only without either does it use `AppMagn` and the distance, and only without those the
        // bolometric luminosity, radius or class. `ForceAppMagn` puts `AppMagn` first.
        let visual_abs = b.f64("AbsMagn").or_else(|| b.f64_any(&["Lum", "Luminosity"]).map(star::abs_mag_from_lum));
        let force_app = b.bool("ForceAppMagn") == Some(true);
        let abs_mag = match (visual_abs, app_mag, dist) {
            (Some(m), _, _) if !force_app => m,
            (_, Some(m), Some(d)) => star::abs_mag_from_app(m, d.max(1e-9)),
            (Some(m), _, _) => m,
            _ => intrinsic_abs.unwrap_or_else(|| class.typical_abs_mag()),
        };
        let radius_km = if let Some(r) = b.f64("Radius") {
            r
        } else if let Some(r) = b.f64_any(&["RadSol", "RadiusSol"]) {
            r * SOLAR_RADIUS_KM
        } else if let Some(r) = radius_from_dimensions(b) {
            r
        } else {
            match class.kind {
                StarKind::NeutronStar => 12.0,
                StarKind::BlackHole => 0.0,
                _ => {
                    let lum_v = star::lum_from_abs_mag(abs_mag);
                    let lum_bol = lum_v * 10f64.powf(-0.4 * star::bolometric_correction(teff));
                    star::radius_from_lum_teff(lum_bol, teff) * SOLAR_RADIUS_KM
                }
            }
        };
        let rec = StarRecord {
            barycenter,
            // A few catalog entries name the parent with all its aliases ("GAM Lep/HD 38393").
            parent: b.str("ParentBody").and_then(|p| p.split('/').next()).map(|p| p.trim().to_string()),
            pos_pc: pos.unwrap_or(DVec3::ZERO),
            has_position: pos.is_some(),
            class,
            abs_mag,
            teff,
            radius_km,
            mass_kg: mass_kg(b).unwrap_or(SOLAR_MASS_KG),
            orbit: orbit_block(b),
            rotation: RotationModel::from_body_block(b),
            source: source.to_string(),
            block: b.clone(),
            names,
        };
        match self.existing(&rec.names) {
            // A component star may share its barycenter's primary name ("Achernar" in
            // both); that is a new object, not an override.
            Some(ObjectRef::Star(i)) if self.stars[i as usize].barycenter == rec.barycenter => {
                let names = rec.names.clone();
                self.stars[i as usize] = rec;
                self.register(&names, ObjectRef::Star(i));
            }
            _ => {
                let idx = self.stars.len() as u32;
                self.register(&rec.names, ObjectRef::Star(idx));
                self.stars.push(rec);
            }
        }
    }

    /// `Name,RA,Dec,Dist,AppMagn,SpecClass,MassSol,RadSol,Temperature` star tables.
    pub fn load_star_csv(&mut self, text: &str, source: &str) {
        let mut lines = text.lines();
        let Some(header) = lines.next() else { return };
        let cols: Vec<String> = header.split(',').map(|c| c.trim().to_ascii_lowercase()).collect();
        let col = |n: &str| cols.iter().position(|c| c == n);
        let (Some(c_name), Some(c_ra), Some(c_dec), Some(c_dist)) = (col("name"), col("ra"), col("dec"), col("dist"))
        else {
            self.warnings.push(format!("{source}: unrecognised star csv header"));
            return;
        };
        let c_mag = col("appmagn");
        let c_abs = col("absmagn");
        let c_class = col("specclass").or_else(|| col("class"));
        let c_mass = col("masssol");
        let c_rad = col("radsol");
        let c_temp = col("temperature").or_else(|| col("teff"));
        for line in lines {
            let f: Vec<&str> = line.split(',').collect();
            let get = |i: Option<usize>| i.and_then(|i| f.get(i)).map(|s| s.trim()).filter(|s| !s.is_empty());
            let num = |i: Option<usize>| get(i).and_then(|s| s.parse::<f64>().ok());
            let (Some(ra), Some(dec), Some(dist)) = (num(Some(c_ra)), num(Some(c_dec)), num(Some(c_dist))) else {
                continue;
            };
            let names: Vec<String> = get(Some(c_name))
                .map(|n| n.split('/').map(str::to_string).collect())
                .unwrap_or_default();
            if names.is_empty() {
                continue;
            }
            let class = SpectralClass::parse(get(c_class).unwrap_or("G2V"));
            let teff = num(c_temp).unwrap_or_else(|| class.temperature());
            let abs_mag = num(c_abs)
                .or_else(|| num(c_mag).map(|m| star::abs_mag_from_app(m, dist.max(1e-6))))
                .unwrap_or_else(|| class.typical_abs_mag());
            let radius_km = num(c_rad).map(|r| r * SOLAR_RADIUS_KM).unwrap_or_else(|| {
                let lum_bol = star::lum_from_abs_mag(abs_mag) * 10f64.powf(-0.4 * star::bolometric_correction(teff));
                star::radius_from_lum_teff(lum_bol, teff) * SOLAR_RADIUS_KM
            });
            let rec = StarRecord {
                names,
                barycenter: false,
                parent: None,
                pos_pc: radec_to_universe(ra, dec) * dist,
                has_position: true,
                class,
                abs_mag,
                teff,
                radius_km,
                mass_kg: num(c_mass).map(|m| m * SOLAR_MASS_KG).unwrap_or(SOLAR_MASS_KG),
                orbit: None,
                rotation: RotationModel::default(),
                source: source.to_string(),
                block: Block::default(),
            };
            if self.existing(&rec.names).is_none() {
                let idx = self.stars.len() as u32;
                self.register(&rec.names, ObjectRef::Star(idx));
                self.stars.push(rec);
            }
        }
    }

    fn add_body(&mut self, tag: BodyTag, item: &Item, b: &Block, source: &str) {
        let names = item.names();
        let Some(parent) = b.str("ParentBody") else {
            self.warnings.push(format!("{source}:{} {} has no ParentBody", item.line, names[0]));
            return;
        };
        let radius_km = b
            .f64("Radius")
            .or_else(|| b.f64("RadiusEarth").map(|r| r * crate::units::EARTH_RADIUS_KM))
            .or_else(|| b.f64("RadSol").map(|r| r * SOLAR_RADIUS_KM))
            .or_else(|| b.vec3("Dimensions").map(|d| (d[0] + d[1] + d[2]) / 6.0))
            .unwrap_or(0.0);
        let rec = BodyRecord {
            tag,
            parent: parent.to_string(),
            class: b.str("Class").map(str::to_string),
            radius_km,
            mass_kg: mass_kg(b).unwrap_or(0.0),
            oblateness: b.f64("Oblateness").unwrap_or(0.0),
            color: b.vec3("Color"),
            albedo: b.f64_any(&["AlbedoGeom", "Albedo", "AlbedoBond"]).unwrap_or(0.3),
            orbit: orbit_block(b),
            rotation: RotationModel::from_body_block(b),
            lock: crate::rotation::RotationLock::from_body_block(b),
            source: source.to_string(),
            block: b.clone(),
            names,
        };
        match self.existing(&rec.names) {
            Some(ObjectRef::Body(i)) if self.bodies[i as usize].parent.eq_ignore_ascii_case(&rec.parent) => {
                let names = rec.names.clone();
                self.bodies[i as usize] = rec;
                self.register(&names, ObjectRef::Body(i));
            }
            _ => {
                let idx = self.bodies.len() as u32;
                // Body names are not globally unique (e.g. moons in exoplanet systems),
                // so only index aliases that are free, except that a planet or moon takes
                // a name over from a minor body ("Io" is the moon, not asteroid (85) Io).
                let major = |t: BodyTag| matches!(t, BodyTag::Planet | BodyTag::DwarfPlanet | BodyTag::Moon);
                for n in &rec.names {
                    let k = key(n);
                    let take = match self.names.get(&k) {
                        None => true,
                        // (`j == idx`: the record repeats one of its own aliases.)
                        Some(ObjectRef::Body(j)) => {
                            *j != idx && major(rec.tag) && !major(self.bodies[*j as usize].tag)
                        }
                        Some(_) => false,
                    };
                    if take {
                        self.names.insert(k, ObjectRef::Body(idx));
                    } else if let Some(&old) = self.names.get(&k) {
                        // Taken under another spelling ("Kepler-34 B" the star): keep both apart.
                        let other = self.names_of(old).iter().find(|o| key(o) == k).map(|o| o.trim().to_string());
                        if let Some(o) = other.filter(|o| o != n.trim()) {
                            self.exact.insert(o, old);
                            self.exact.insert(n.trim().to_string(), ObjectRef::Body(idx));
                        }
                    }
                }
                self.bodies.push(rec);
            }
        }
    }

    fn add_galaxy(&mut self, item: &Item, b: &Block, source: &str) {
        let names = item.names();
        let Some(pos) = block_position_pc(b) else { return };
        let dist = pos.length();
        let abs_mag = b
            .f64("AbsMagn")
            .or_else(|| b.f64("AppMagn").map(|m| star::abs_mag_from_app(m, dist)))
            .unwrap_or(-20.0);
        let orientation = if let Some(q) = b.vec4("Quat") {
            se_galaxy_quat_to_model_rotation(q)
        } else if let Some(a) = b.vec4("Axis") {
            se_axis_angle_to_model_rotation(a)
        } else {
            // Deterministic pseudo-random orientation from the name.
            pseudo_random_orientation(&names[0])
        };
        let radius_pc = b.f64("Radius").unwrap_or_else(|| {
            // AngSize in arcseconds if given; else scale from luminosity.
            b.f64("AngSize")
                .map(|a| dist * (a / 3600.0).to_radians() * 0.5)
                .unwrap_or_else(|| 10_000.0 * 10f64.powf(-0.2 * (abs_mag + 20.0)))
        });
        let rec = GalaxyRecord {
            names,
            kind: b.str("Type").unwrap_or("E0").to_string(),
            quasar: item.key == "Quasar",
            pos_pc: pos,
            radius_pc,
            abs_mag,
            vis_mag: abs_mag,
            orientation,
            model_bright: b.f64("ModelBright").unwrap_or(1.0),
            source: source.to_string(),
            model_roll: None,
        };
        // SE's galaxy `GlobClusters` (FUN_14016bef0, runtime +0x64): the globular count, used instead
        // of the luminosity formula (FUN_14049cf40).
        let glob = b.f64("GlobClusters").map(|n| (n as i32, rec.names.clone()));
        self.push_galaxy(rec);
        if let Some((n, names)) = glob {
            if let Some(ObjectRef::Galaxy(i)) = self.existing(&names) {
                self.glob_clusters.insert(i, n);
            }
        }
    }

    fn push_galaxy(&mut self, rec: GalaxyRecord) {
        match self.existing(&rec.names) {
            Some(ObjectRef::Galaxy(i)) => {
                let names = rec.names.clone();
                self.galaxies[i as usize] = rec;
                self.register(&names, ObjectRef::Galaxy(i));
            }
            _ => {
                let idx = self.galaxies.len() as u32;
                self.register(&rec.names, ObjectRef::Galaxy(idx));
                self.galaxies.push(rec);
            }
        }
    }

    /// `Name,Type,RA,Dec,Dist,AbsMagn,Radius,Quat.w,Quat.x,Quat.y,Quat.z`.
    pub fn load_galaxy_csv(&mut self, text: &str, source: &str) {
        let mut lines = text.lines();
        let Some(header) = lines.next() else { return };
        let cols: Vec<String> = header.split(',').map(|c| c.trim().to_ascii_lowercase()).collect();
        let col = |n: &str| cols.iter().position(|c| c == n);
        let idx = [
            col("name"),
            col("type"),
            col("ra"),
            col("dec"),
            col("dist"),
            col("absmagn"),
            col("radius"),
            col("quat.w"),
            col("quat.x"),
            col("quat.y"),
            col("quat.z"),
        ];
        if idx[..5].iter().any(Option::is_none) {
            self.warnings.push(format!("{source}: unrecognised galaxy csv header"));
            return;
        }
        for line in lines {
            let f: Vec<&str> = line.split(',').collect();
            let s = |i: Option<usize>| i.and_then(|i| f.get(i)).map(|s| s.trim()).unwrap_or("");
            let n = |i: Option<usize>| s(i).parse::<f64>().ok();
            let (Some(ra), Some(dec), Some(dist)) = (n(idx[2]), n(idx[3]), n(idx[4])) else { continue };
            let names: Vec<String> = s(idx[0]).split('/').map(str::to_string).filter(|x| !x.is_empty()).collect();
            if names.is_empty() {
                continue;
            }
            let orientation = match (n(idx[7]), n(idx[8]), n(idx[9]), n(idx[10])) {
                (Some(w), Some(x), Some(y), Some(z)) => se_galaxy_quat_to_model_rotation([w, x, y, z]),
                _ => pseudo_random_orientation(&names[0]),
            };
            let abs_mag = n(idx[5]).unwrap_or(-20.0);
            let rec = GalaxyRecord {
                kind: s(idx[1]).to_string(),
                quasar: false,
                pos_pc: radec_to_universe(ra, dec) * dist,
                radius_pc: n(idx[6]).unwrap_or(10_000.0),
                abs_mag,
                vis_mag: abs_mag,
                orientation,
                model_bright: 1.0,
                source: source.to_string(),
                model_roll: None,
                names,
            };
            self.push_galaxy(rec);
        }
    }

    fn add_cluster(&mut self, item: &Item, b: &Block, source: &str) {
        let Some(pos) = block_position_pc(b) else { return };
        let dist = pos.length();
        let names = item.names();
        let kind = b.str("Type").unwrap_or("Open").to_string();
        let (radius_pc, abs_mag, age_gyr, cen_pow) = complete_cluster(b, ClusterType::parse(&kind), names.first().map_or("", String::as_str), dist);
        let rec = ClusterRecord {
            names,
            galaxy: b.str("Galaxy").unwrap_or("Milky Way").to_string(),
            kind,
            pos_pc: pos,
            radius_pc,
            abs_mag,
            age_gyr,
            cen_pow,
            no_members: b.f64("NStars") == Some(0.0),
            color: b.vec3("Color").map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]),
            source: source.to_string(),
        };
        let idx = self.clusters.len() as u32;
        for n in &rec.names {
            self.names.entry(key(n)).or_insert(ObjectRef::Cluster(idx));
        }
        self.clusters.push(rec);
    }

    fn add_nebula(&mut self, item: &Item, b: &Block, source: &str) {
        let names = item.names();
        let Some(pos) = block_position_pc(b) else { return };
        let dist = pos.length();
        let orientation = b
            .vec4("Quat")
            .map(se_galaxy_quat_to_model_rotation)
            .unwrap_or_else(|| pseudo_random_orientation(&names[0]));
        let radius_pc = b
            .f64("Radius")
            .or_else(|| b.f64("RadiusLy").map(|r| r * crate::units::METERS_PER_LY / crate::units::METERS_PER_PC))
            .unwrap_or(1.0);
        let rec = NebulaRecord {
            galaxy: b.str("Galaxy").unwrap_or("Milky Way").to_string(),
            kind: b.str("Type").unwrap_or("Diffuse").to_string(),
            pos_pc: pos,
            radius_pc,
            abs_mag: b
                .f64("AbsMagn")
                .or_else(|| b.f64("AppMagn").map(|m| star::abs_mag_from_app(m, dist)))
                .unwrap_or(-5.0),
            orientation,
            source: source.to_string(),
            model_roll: None,
            names,
        };
        let idx = self.nebulae.len() as u32;
        for n in &rec.names {
            self.names.entry(key(n)).or_insert(ObjectRef::Nebula(idx));
        }
        self.nebulae.push(rec);
    }

    /// Call after one or more `load_sc` calls (e.g. procedurally generated systems) to
    /// build the hierarchy and orbital periods of the stars and bodies added since the
    /// last resolve. Only the new records are visited: a full pass over the catalog takes
    /// ~10 ms, a hitch on every procedural system entered.
    pub fn finalize(&mut self) {
        let (s0, b0) = self.resolved;
        self.resolve_from(s0, b0);
    }

    fn resolve(&mut self) {
        self.children.clear();
        self.resolve_from(0, 0);
    }

    /// Resolve star system hierarchy for stars from `s0` and bodies from `b0` on:
    /// components without RA/Dec take their parent's position; orbit periods are derived
    /// from parent masses.
    fn resolve_from(&mut self, s0: usize, b0: usize) {
        // SE ties a star (a galaxy's central black hole, the Crab Pulsar) to the centre of a galaxy,
        // nebula or cluster named by `CenterOf`, whatever its own coordinates (FUN_140476e90).
        for i in s0..self.stars.len() {
            let Some(target) = self.stars[i].block.str("CenterOf") else { continue };
            let pos = match self.find(target) {
                Some(ObjectRef::Galaxy(g)) => Some(self.galaxies[g as usize].pos_pc),
                Some(ObjectRef::Nebula(n)) => Some(self.nebulae[n as usize].pos_pc),
                Some(ObjectRef::Cluster(c)) => Some(self.clusters[c as usize].pos_pc),
                _ => None,
            };
            if let Some(p) = pos {
                self.stars[i].pos_pc = p;
                self.stars[i].has_position = true;
            }
        }
        // Propagate positions down the hierarchy (a few passes handle nesting).
        for _ in 0..6 {
            let mut changed = false;
            for i in s0..self.stars.len() {
                if self.stars[i].has_position {
                    continue;
                }
                let Some(p) = self.stars[i].parent.as_deref().map(key) else { continue };
                if let Some(ObjectRef::Star(j)) = self.names.get(&p).copied() {
                    if self.stars[j as usize].has_position {
                        self.stars[i].pos_pc = self.stars[j as usize].pos_pc;
                        self.stars[i].has_position = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let unresolved = self.stars[s0..].iter().filter(|s| !s.has_position).count();
        if unresolved > 0 {
            self.warnings.push(format!("{unresolved} stars without a resolvable position"));
        }
        // Children index and orbital periods. Parents may be named by any alias
        // (comets use "Sol" for the Sun), so key them by the parent's primary name.
        for i in s0..self.stars.len() {
            if let Some(p) = self.stars[i].parent.clone() {
                let k = self.canonical_key(&p);
                self.children.entry(k).or_default().push(ObjectRef::Star(i as u32));
            }
        }
        for i in b0..self.bodies.len() {
            let p = self.canonical_key(&self.bodies[i].parent);
            self.children.entry(p).or_default().push(ObjectRef::Body(i as u32));
        }
        self.resolve_separations(s0, b0);
        for i in b0..self.bodies.len() {
            let p = self.canonical_key(&self.bodies[i].parent);
            let parent_mass = self.system_mass(&p);
            if let Some(o) = self.bodies[i].orbit.as_mut() {
                o.resolve_period(parent_mass);
            }
        }
        for i in s0..self.stars.len() {
            let Some(p) = self.stars[i].parent.as_deref().map(key) else { continue };
            let parent_mass = self.system_mass(&p);
            if let Some(o) = self.stars[i].orbit.as_mut() {
                o.resolve_period(parent_mass);
            }
        }
        self.resolved = (self.stars.len(), self.bodies.len());
    }

    /// Name key of a parent, by its primary name: parents may be named by any alias (comets use
    /// "Sol" for the Sun).
    fn canonical_key(&self, p: &str) -> String {
        match self.find(p) {
            Some(r) => key(self.display_name(r)),
            None => key(p),
        }
    }

    fn orbit_mut(&mut self, r: ObjectRef) -> Option<&mut Option<KeplerOrbit>> {
        match r {
            ObjectRef::Star(i) => self.stars.get_mut(i as usize).map(|s| &mut s.orbit),
            ObjectRef::Body(i) => self.bodies.get_mut(i as usize).map(|b| &mut b.orbit),
            _ => None,
        }
    }

    fn orbit_ref(&self, r: ObjectRef) -> Option<&KeplerOrbit> {
        match r {
            ObjectRef::Star(i) => self.stars.get(i as usize)?.orbit.as_ref(),
            ObjectRef::Body(i) => self.bodies.get(i as usize)?.orbit.as_ref(),
            _ => None,
        }
    }

    /// Mass of a binary member (a barycenter weighs its stars).
    fn member_mass(&self, r: ObjectRef) -> f64 {
        match r {
            ObjectRef::Star(_) => self.system_mass(&key(self.display_name(r))),
            ObjectRef::Body(i) => self.bodies[i as usize].mass_kg,
            _ => 0.0,
        }
    }

    /// `Separation` is the distance between the two members of a binary, not one member's
    /// orbit: as SE does, split it by mass around the barycentre (each member at `sep * m_other /
    /// m_total`). Both members are resolved together. A partner without an orbit of its own takes
    /// the far side of the same orbit (Cygnus X-1 gives only the black hole one); of two
    /// `BinaryOrbit` members (same elements) the second is moved to the far side. A missing period
    /// follows from the total mass.
    fn resolve_separations(&mut self, s0: usize, b0: usize) {
        let items: Vec<ObjectRef> = (s0..self.stars.len())
            .map(|i| ObjectRef::Star(i as u32))
            .chain((b0..self.bodies.len()).map(|i| ObjectRef::Body(i as u32)))
            .collect();
        for r in items {
            let Some(o) = self.orbit_ref(r).filter(|o| o.separation > 0.0).cloned() else { continue };
            let parent = match r {
                ObjectRef::Star(i) => self.stars[i as usize].parent.clone(),
                ObjectRef::Body(i) => Some(self.bodies[i as usize].parent.clone()),
                _ => None,
            };
            let Some(parent) = parent else { continue };
            let partner = self.children.get(&self.canonical_key(&parent)).and_then(|c| {
                c.iter().copied().find(|&q| q != r && self.orbit_ref(q).is_none_or(|po| po.separation > 0.0))
            });
            let (m1, m2) = (self.member_mass(r), partner.map_or(0.0, |q| self.member_mass(q)));
            let f = if partner.is_some() && m1 + m2 > 0.0 { m2 / (m1 + m2) } else { 1.0 };
            let sep = o.separation;
            let mut mine = o;
            mine.separation = 0.0;
            mine.semi_major_axis = sep * f;
            if mine.period <= 0.0 && m1 + m2 > 0.0 {
                mine.period = std::f64::consts::TAU * (sep.powi(3) / (crate::units::G * (m1 + m2))).sqrt();
            }
            if let Some(q) = partner {
                let far = mine.arg_of_periapsis + std::f64::consts::PI;
                match self.orbit_mut(q) {
                    Some(slot @ None) => {
                        let mut theirs = mine.clone();
                        theirs.semi_major_axis = sep * (1.0 - f);
                        theirs.arg_of_periapsis = far;
                        *slot = Some(theirs);
                    }
                    Some(Some(po)) => {
                        po.semi_major_axis = po.separation * (1.0 - f);
                        po.separation = 0.0;
                        if po.period <= 0.0 {
                            po.period = mine.period;
                        }
                        if po.binary && mine.binary {
                            po.arg_of_periapsis = far;
                        }
                    }
                    None => {}
                }
            }
            if let Some(slot) = self.orbit_mut(r) {
                *slot = Some(mine);
            }
        }
    }

    /// Mass of a named object, or for barycenters the sum of their stellar children.
    fn system_mass(&self, name_key: &str) -> f64 {
        match self.names.get(name_key) {
            Some(ObjectRef::Star(i)) => {
                let s = &self.stars[*i as usize];
                if s.barycenter {
                    self.children
                        .get(name_key)
                        .map(|c| {
                            c.iter()
                                .filter_map(|r| match r {
                                    ObjectRef::Star(j) => Some(self.stars[*j as usize].mass_kg),
                                    _ => None,
                                })
                                .sum()
                        })
                        .unwrap_or(SOLAR_MASS_KG)
                } else {
                    s.mass_kg
                }
            }
            Some(ObjectRef::Body(i)) => self.bodies[*i as usize].mass_kg,
            _ => 0.0,
        }
    }

    pub fn find(&self, name: &str) -> Option<ObjectRef> {
        self.exact.get(name.trim()).or_else(|| self.names.get(&key(name))).copied()
    }

    /// Append a star created at runtime (e.g. a procedural star the observer
    /// approached). Returns its index; an existing star of the same name is reused.
    pub fn add_star_record(&mut self, rec: StarRecord) -> u32 {
        if let Some(ObjectRef::Star(i)) = self.existing(&rec.names) {
            return i;
        }
        let idx = self.stars.len() as u32;
        self.register(&rec.names, ObjectRef::Star(idx));
        self.stars.push(rec);
        idx
    }

    /// Append a galaxy created at runtime (a procedural `RG` galaxy). An existing galaxy of the
    /// same name is reused. Returns its index.
    pub fn add_galaxy_record(&mut self, rec: GalaxyRecord) -> u32 {
        if let Some(ObjectRef::Galaxy(i)) = self.existing(&rec.names) {
            return i;
        }
        let idx = self.galaxies.len() as u32;
        self.register(&rec.names, ObjectRef::Galaxy(idx));
        self.galaxies.push(rec);
        idx
    }

    /// Append a cluster created at runtime (a procedural `RC` cluster); reuses an existing name.
    pub fn add_cluster_record(&mut self, rec: ClusterRecord) -> u32 {
        if let Some(ObjectRef::Cluster(i)) = self.existing(&rec.names) {
            return i;
        }
        let idx = self.clusters.len() as u32;
        self.register(&rec.names, ObjectRef::Cluster(idx));
        self.clusters.push(rec);
        idx
    }

    /// Append a nebula created at runtime (a procedural `RN` nebula); reuses an existing name.
    pub fn add_nebula_record(&mut self, rec: NebulaRecord) -> u32 {
        if let Some(ObjectRef::Nebula(i)) = self.existing(&rec.names) {
            return i;
        }
        let idx = self.nebulae.len() as u32;
        self.register(&rec.names, ObjectRef::Nebula(idx));
        self.nebulae.push(rec);
        idx
    }

    /// Case-insensitive prefix search over all names, for the search box.
    pub fn search(&self, prefix: &str, limit: usize) -> Vec<(String, ObjectRef)> {
        let p = key(prefix);
        if p.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<(String, ObjectRef)> = self
            .names
            .iter()
            .filter(|(k, _)| k.starts_with(&p))
            .map(|(_, r)| (self.display_name(*r).to_string(), *r))
            .collect();
        out.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
        out.dedup_by(|a, b| a.1 == b.1);
        out.truncate(limit);
        out
    }

    pub fn display_name(&self, r: ObjectRef) -> &str {
        self.names_of(r).first().map(String::as_str).unwrap_or("?")
    }

    /// Every name of `r`.
    pub fn names_of(&self, r: ObjectRef) -> &[String] {
        let names = match r {
            ObjectRef::Star(i) => self.stars.get(i as usize).map(|s| &s.names),
            ObjectRef::Body(i) => self.bodies.get(i as usize).map(|s| &s.names),
            ObjectRef::Galaxy(i) => self.galaxies.get(i as usize).map(|s| &s.names),
            ObjectRef::Cluster(i) => self.clusters.get(i as usize).map(|s| &s.names),
            ObjectRef::Nebula(i) => self.nebulae.get(i as usize).map(|s| &s.names),
        };
        names.map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn children_of(&self, name: &str) -> &[ObjectRef] {
        self.children.get(&key(name)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Recursively collect all bodies and stars that belong to a system root.
    ///
    /// Each object and each name is visited once: a child can share its parent's display name
    /// (planets/Stars.sc `Star "Vega" { ParentBody "ALF Lyr" }` under the HIPPARCOS "Vega/ALF Lyr"),
    /// which used to list the child as its own child until a 100 000-step guard.
    pub fn system_members(&self, root: &str) -> Vec<ObjectRef> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut expanded = std::collections::HashSet::new();
        let mut stack = vec![key(root)];
        while let Some(n) = stack.pop() {
            if !expanded.insert(n.clone()) {
                continue;
            }
            for &c in self.children_of(&n) {
                if seen.insert(c) {
                    out.push(c);
                    stack.push(key(self.display_name(c)));
                }
            }
        }
        out
    }

    /// Drop the generated stars and bodies (source [`crate::climate::PROCEDURAL_SOURCE`]) of the
    /// system under `root`, as SE frees a planetary system beyond `CacheSizeSolarSystems`: their
    /// names and child links go, and their records become hidden, positionless placeholders, so
    /// every other object keeps its index. Generating the system again adds it anew. Returns how
    /// many objects went.
    pub fn unload_generated(&mut self, root: &str) -> usize {
        let src = crate::climate::PROCEDURAL_SOURCE;
        let gone: std::collections::HashSet<ObjectRef> = self
            .system_members(root)
            .into_iter()
            .filter(|m| match *m {
                ObjectRef::Star(i) => self.stars[i as usize].source == src,
                ObjectRef::Body(i) => self.bodies[i as usize].source == src,
                _ => false,
            })
            .collect();
        if gone.is_empty() {
            return 0;
        }
        self.names.retain(|_, r| !gone.contains(r));
        self.exact.retain(|_, r| !gone.contains(r));
        for v in self.children.values_mut() {
            v.retain(|r| !gone.contains(r));
        }
        self.children.retain(|_, v| !v.is_empty());
        let placeholder = sc::parse("Hidden true");
        for r in &gone {
            match *r {
                ObjectRef::Star(i) => {
                    let s = &mut self.stars[i as usize];
                    s.names = vec![String::new()];
                    s.parent = None;
                    s.has_position = false;
                    s.orbit = None;
                    s.radius_km = 0.0;
                    s.mass_kg = 0.0;
                    s.source = UNLOADED_SOURCE.to_string();
                    s.block = placeholder.clone();
                }
                ObjectRef::Body(i) => {
                    let b = &mut self.bodies[i as usize];
                    b.names = vec![String::new()];
                    b.parent = String::new();
                    b.class = None;
                    b.radius_km = 0.0;
                    b.mass_kg = 0.0;
                    b.orbit = None;
                    b.source = UNLOADED_SOURCE.to_string();
                    b.block = placeholder.clone();
                }
                _ => {}
            }
        }
        gone.len()
    }

    /// The top-level system root (star/barycenter with no parent) for an object.
    pub fn system_root(&self, r: ObjectRef) -> Option<u32> {
        let mut cur = r;
        for _ in 0..32 {
            let parent = match cur {
                ObjectRef::Star(i) => match &self.stars[i as usize].parent {
                    None => return Some(i),
                    Some(p) => p.clone(),
                },
                ObjectRef::Body(i) => self.bodies[i as usize].parent.clone(),
                _ => return None,
            };
            cur = self.find(&parent)?;
        }
        None
    }

    /// Magnitude shift of each star's point in the sky (indexed like `stars`): SE draws a distant
    /// system as one point, at the sum of its components unless the system has `ForceAppMagn`;
    /// then it shines at its own `AppMagn`, so its components' summed light is scaled to that.
    /// 0 for every other star.
    pub fn forced_system_offsets(&self) -> Vec<f32> {
        let roots: Vec<Option<u32>> = (0..self.stars.len())
            .map(|i| {
                let s = &self.stars[i];
                if s.barycenter || !s.has_position {
                    return None;
                }
                let r = self.system_root(ObjectRef::Star(i as u32))?;
                let root = &self.stars[r as usize];
                (r as usize != i && root.barycenter && root.force_app_magn() && root.abs_mag.is_finite()).then_some(r)
            })
            .collect();
        let mut sum: HashMap<u32, f64> = HashMap::new();
        for (i, r) in roots.iter().enumerate() {
            if let Some(r) = r {
                *sum.entry(*r).or_default() += star::lum_from_abs_mag(self.stars[i].abs_mag);
            }
        }
        roots
            .iter()
            .map(|r| match r.and_then(|r| Some((r, *sum.get(&r)?))) {
                Some((r, l)) if l > 0.0 => (self.stars[r as usize].abs_mag - star::abs_mag_from_lum(l)) as f32,
                _ => 0.0,
            })
            .collect()
    }
}

/// Deterministic orientation derived from a string hash (for catalog entries
/// without explicit orientation).
pub fn pseudo_random_orientation(name: &str) -> DQuat {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let u1 = (h & 0xFFFF) as f64 / 65535.0;
    let u2 = ((h >> 16) & 0xFFFF) as f64 / 65535.0;
    let u3 = ((h >> 32) & 0xFFFF) as f64 / 65535.0;
    // Shoemake uniform random quaternion.
    let (s1, s2) = ((1.0 - u1).sqrt(), u1.sqrt());
    let (t1, t2) = (std::f64::consts::TAU * u2, std::f64::consts::TAU * u3);
    DQuat::from_xyzw(s1 * t1.sin(), s1 * t1.cos(), s2 * t2.sin(), s2 * t2.cos()).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEM: &str = r#"
StarBarycenter "Alpha Centauri" { RA 14 39 36.5 Dec -60 50 02 Dist 1.3474 }
Star "Alpha Centauri A/Rigil Kentaurus" { ParentBody "Alpha Centauri" Class "G2V" MassSol 1.1 AppMagn 0.01
  Orbit { SemiMajorAxis 10.9 Period 79.91 Eccentricity 0.52 } }
Star "Alpha Centauri B" { ParentBody "Alpha Centauri" Class "K1V" MassSol 0.9 }
Planet "Test b" { ParentBody "Alpha Centauri B" Radius 6000 Mass 1
  Orbit { SemiMajorAxis 0.05 } }
Galaxy "M 31/Andromeda" { Type "Sb" RA 0.7122 Dec 41.2689 Dist 778770 Radius 22655 AppMagn 2.68
  Quat ( 0.5012805 -0.0873325 0.7087535 0.4886301 ) }
"#;

    #[test]
    fn unloads_a_generated_system() {
        let mut c = Catalog::new();
        c.load_sc(SYSTEM, "test.sc");
        c.resolve();
        let gen_src = crate::climate::PROCEDURAL_SOURCE;
        c.load_sc("Planet \"Gen c\" { ParentBody \"Alpha Centauri B\" Radius 3000 Mass 0.5 Orbit { SemiMajorAxis 0.1 } }\nMoon \"Gen c I\" { ParentBody \"Gen c\" Radius 500 Orbit { SemiMajorAxis 0.001 } }\n", gen_src);
        c.finalize();
        let (n_stars, n_bodies) = (c.stars.len(), c.bodies.len());
        assert_eq!(c.system_members("Alpha Centauri").len(), 5);
        // Only the generated objects go; indices stay, so the catalog's own planet is still found.
        assert_eq!(c.unload_generated("Alpha Centauri"), 2);
        assert_eq!((c.stars.len(), c.bodies.len()), (n_stars, n_bodies));
        assert_eq!(c.find("Gen c"), None);
        assert_eq!(c.find("Gen c I"), None);
        assert!(matches!(c.find("Test b"), Some(ObjectRef::Body(_))));
        assert_eq!(c.system_members("Alpha Centauri").len(), 3);
        assert_eq!(c.unload_generated("Alpha Centauri"), 0);
        // Generating it again adds it anew.
        c.load_sc("Planet \"Gen c\" { ParentBody \"Alpha Centauri B\" Radius 3000 Mass 0.5 Orbit { SemiMajorAxis 0.1 } }\n", gen_src);
        c.finalize();
        assert!(c.find("Gen c").is_some());
        assert_eq!(c.system_members("Alpha Centauri").len(), 4);
    }

    #[test]
    fn hierarchy_and_lookup() {
        let mut c = Catalog::new();
        c.load_sc(SYSTEM, "test.sc");
        c.resolve();
        let a = c.find("rigil kentaurus").unwrap();
        let ObjectRef::Star(ai) = a else { panic!() };
        let s = &c.stars[ai as usize];
        assert!(s.has_position);
        assert!((s.pos_pc.length() - 1.3474).abs() < 1e-9);
        assert!(s.abs_mag > 4.0 && s.abs_mag < 5.0, "{}", s.abs_mag);
        assert_eq!(c.children_of("Alpha Centauri").len(), 2);
        let ObjectRef::Body(pi) = c.find("Test b").unwrap() else { panic!() };
        let p = &c.bodies[pi as usize];
        assert!(p.orbit.as_ref().unwrap().period > 0.0);
        assert_eq!(c.system_root(ObjectRef::Body(pi)), Some(0));
        assert_eq!(c.system_members("Alpha Centauri").len(), 3);
        // A star whose parent is an alias of a star with its own name (planets/Stars.sc "Vega").
        let mut v = Catalog::new();
        v.load_sc(
            "StarBarycenter \"Vega/ALF Lyr\" { RA 18 36 56 Dec 38 47 01 Dist 7.68 }\nStar \"Vega\" { ParentBody \"ALF Lyr\" Class \"A0V\" AbsMagn 0.58 }\n",
            "vega.sc",
        );
        v.resolve();
        assert_eq!(v.system_members("Vega").len(), 1);
        let ObjectRef::Galaxy(gi) = c.find("andromeda").unwrap() else { panic!() };
        assert!((c.galaxies[gi as usize].pos_pc.length() - 778_770.0).abs() < 1e-6);
        assert!(!c.search("alpha", 10).is_empty());
    }

    #[test]
    fn cluster_completion() {
        let mut c = Catalog::new();
        c.load_sc(
            "Cluster \"Pozzo 1\" { Galaxy \"Milky Way\" Type \"Open\" RA 8.15888889 Dec -47.3366667 Dist 336 Radius 0.8796 }\n\
             Cluster \"Pleiades\" { Type \"Open\" RA 3.78 Dec 24.1 Dist 135.8 Radius 2.3 AbsMagn -3.6 Color ( 0 0 0 ) CenPow 0.5 NStars 0 }\n\
             Cluster \"G\" { Type \"Globular\" RA 1 Dec 2 Dist 5000 Radius 20 AbsMagn -8 NStars 1000 }\n\
             Galaxy \"M 31\" { Type \"Sb\" RA 0.7 Dec 41 Dist 778000 GlobClusters 450 }\n",
            "c.sc",
        );
        // Pozzo 1: L = min(1000, 336^2 exp(-7.3/1.0587362)) = 114.2 Lsun, an open cluster's young age.
        let p = &c.clusters[0];
        assert!((p.abs_mag - (4.83 - 2.5 * 114.2f64.log10())).abs() < 0.02, "{}", p.abs_mag);
        assert!(p.age_gyr > 0.0 && p.age_gyr < 3.5);
        assert!((0.5..=2.5).contains(&p.cen_pow) && !p.no_members);
        assert_eq!(c.clusters[1].color, Some([0.0; 3]));
        assert!(c.clusters[1].no_members && c.clusters[1].cen_pow == 0.5);
        assert!(c.clusters[2].age_gyr > 0.0 && c.clusters[2].age_gyr <= 13.0);
        // Only NStars 0 counts.
        assert!(!c.clusters[2].no_members);
        let Some(ObjectRef::Galaxy(m31)) = c.find("M 31") else { panic!() };
        assert_eq!(c.glob_clusters.get(&m31), Some(&450));
    }

    #[test]
    fn center_of_puts_a_star_at_its_galaxy() {
        let mut c = Catalog::new();
        c.load_sc(
            "Galaxy \"NGC 1023\" { Type \"SB0\" RA 2.67 Dec 39.06 Dist 9.7e6 Radius 15000 }
             Star \"NGC 1023*\" { RA 2.67 Dec 39.06 Dist 1.057e7 Class \"X\" CenterOf \"NGC 1023\" }
",
            "c.sc",
        );
        c.finalize();
        let Some(ObjectRef::Star(s)) = c.find("NGC 1023*") else { panic!() };
        let Some(ObjectRef::Galaxy(g)) = c.find("NGC 1023") else { panic!() };
        assert!((c.stars[s as usize].pos_pc - c.galaxies[g as usize].pos_pc).length() < 1e-6);
    }

    #[test]
    fn remove_drops_a_star_like_remove_star() {
        let mut c = Catalog::new();
        c.load_sc("Star \"BET2 Cyg\" { RA 19.5 Dec 27.9 Dist 120 Class \"B8V\" }
Remove \"BET2 Cyg\"
", "c.sc");
        assert_eq!(c.find("BET2 Cyg"), None);
    }

    #[test]
    fn names_differing_only_in_case_stay_apart() {
        let mut c = Catalog::new();
        c.load_sc(
            "Star \"Kepler-34 B\" { RA 19.75 Dec 44.65 Dist 1499 Class \"G\" }\n\
             Planet \"Kepler-34 b\" { ParentBody \"Kepler-34 B\" Radius 52000 }\n",
            "c.sc",
        );
        assert!(matches!(c.find("Kepler-34 B"), Some(ObjectRef::Star(_))));
        assert!(matches!(c.find("Kepler-34 b"), Some(ObjectRef::Body(_))));
        // Any other spelling still finds one of them.
        assert!(c.find("kepler-34 b").is_some());
    }

    #[test]
    fn body_shapes() {
        let mut c = Catalog::new();
        c.load_sc(
            "Planet \"Saturn\" { ParentBody \"Sun\" Radius 60268 Oblateness 0.09796 }\n\
             Moon \"Mimas\" { ParentBody \"Saturn\" Dimensions ( 393.4, 381.2, 415.6 ) }\n\
             Planet \"Ball\" { ParentBody \"Sun\" Radius 1000 }\n",
            "s.sc",
        );
        let s = c.bodies[0].shape();
        assert!((s[1] - 0.90204).abs() < 1e-5 && s[0] == 1.0 && s[2] == 1.0, "{s:?}");
        let m = c.bodies[1].shape();
        let r = c.bodies[1].radius_km;
        assert!((m[1] * r - 190.6).abs() < 1e-6 && m[2] > m[0] && m[0] > m[1], "{m:?}");
        assert_eq!(c.bodies[2].shape(), [1.0; 3]);
    }

    #[test]
    fn star_shapes() {
        let mut c = Catalog::new();
        c.load_sc(
            "Star \"Achernar\" { RA 1 Dec 2 Dist 40 Class \"B6V\" AbsMagn -1.46 DimensionsSol (11.4, 7.3, 11.4) }
\
             Star \"Altair\" { RA 3 Dec 4 Dist 5 Class \"A7V\" AbsMagn 2.22 RadSol 2.03 Oblateness 0.197 }
",
            "s.sc",
        );
        let a = c.find("Achernar").and_then(|r| if let ObjectRef::Star(i) = r { Some(&c.stars[i as usize]) } else { None }).unwrap();
        assert!((a.radius_km - 5.7 * SOLAR_RADIUS_KM).abs() < 1.0, "{}", a.radius_km);
        assert!((a.shape()[1] - 7.3 / 11.4).abs() < 1e-9 && a.shape()[0] == 1.0, "{:?}", a.shape());
        let b = c.find("Altair").and_then(|r| if let ObjectRef::Star(i) = r { Some(&c.stars[i as usize]) } else { None }).unwrap();
        assert!((b.shape()[1] - 0.803).abs() < 1e-9 && (b.radius_km - 2.03 * SOLAR_RADIUS_KM).abs() < 1.0);
    }

    #[test]
    fn force_app_magn_and_hidden() {
        let mut c = Catalog::new();
        c.load_sc(
            "StarBarycenter \"Sys\" { RA 1 Dec 2 Dist 100 AppMagn 2.5 ForceAppMagn true }
             Star \"Sys A\" { ParentBody \"Sys\" Class \"B1V\" AbsMagn -3 }
             Star \"Sys B\" { ParentBody \"Sys\" Class \"B2V\" AbsMagn -3 }
             Star \"Lit\" { RA 3 Dec 4 Dist 10 AppMagn 9 Lum 100 Class \"M3I\" }
             Star \"Forced\" { RA 3 Dec 4 Dist 10 AppMagn 9 Lum 100 Class \"M3I\" ForceAppMagn true }
             Star \"Menu Star\" { RA 5 Dec 6 Dist 50 Class \"G2V\" Lum 1 Hidden true }",
            "s.sc",
        );
        c.finalize();
        let star = |n: &str| match c.find(n) {
            Some(ObjectRef::Star(i)) => i as usize,
            _ => panic!("{n}"),
        };
        // SE takes Lum before AppMagn and the distance, unless ForceAppMagn.
        assert!((c.stars[star("Lit")].abs_mag - star::abs_mag_from_lum(100.0)).abs() < 1e-9);
        assert!((c.stars[star("Forced")].abs_mag - star::abs_mag_from_app(9.0, 10.0)).abs() < 1e-9);
        // The two components together shine at the system's AppMagn.
        let off = c.forced_system_offsets();
        let (a, b) = (star("Sys A"), star("Sys B"));
        let sum = star::lum_from_abs_mag(-3.0 + off[a] as f64) + star::lum_from_abs_mag(-3.0 + off[b] as f64);
        assert!((star::abs_mag_from_lum(sum) - star::abs_mag_from_app(2.5, 100.0)).abs() < 1e-4);
        assert_eq!(off[star("Lit")], 0.0);
        assert!(c.stars[star("Menu Star")].hidden() && !c.stars[a].hidden());
    }

    #[test]
    fn star_csv() {
        let mut c = Catalog::new();
        c.load_star_csv(
            "Name,RA,Dec,Dist,AppMagn,SpecClass,MassSol,RadSol,Temperature\nHIP 1/HD 2,3.0,36.1,368.6,7.2,K2,,,\n",
            "x.csv",
        );
        assert_eq!(c.stars.len(), 1);
        assert!(c.find("HD 2").is_some());
    }

    #[test]
    fn binary_separation_is_split_by_mass() {
        let au = crate::units::METERS_PER_AU;
        let mut c = Catalog::new();
        c.load_sc(
            r#"
StarBarycenter "Cygnus X-1" { RA 19 58 21.7 Dec 35 12 06 Dist 2222 }
Star "Cygnus X-1 A" { ParentBody "Cygnus X-1" Class "X" MassSol 21.2
  Orbit { Separation 0.22 Period 0.0153420091 Inclination 45 } }
Star "HDE 226868" { ParentBody "Cygnus X-1" Class "O9.7 Ia" MassSol 31 Radius 11136000 }
StarBarycenter "WD pair" { RA 1 0 0 Dec 10 0 0 Dist 50 }
Star "WD pair A" { ParentBody "WD pair" Class "DA" MassSol 0.6 BinaryOrbit { Separation 200 Eccentricity 0.5 } }
Star "WD pair B" { ParentBody "WD pair" Class "K5V" MassSol 0.6 BinaryOrbit { Separation 200 Eccentricity 0.5 } }
"#,
            "test.sc",
        );
        c.resolve();
        let orbit = |n: &str| {
            let ObjectRef::Star(i) = c.find(n).unwrap() else { panic!() };
            c.stars[i as usize].orbit.clone().unwrap()
        };
        // The black hole holds the orbit; the supergiant gets the far side, both by mass.
        let (bh, sg) = (orbit("Cygnus X-1 A"), orbit("HDE 226868"));
        assert!((bh.semi_major_axis / au - 0.22 * 31.0 / 52.2).abs() < 1e-9);
        assert!((sg.semi_major_axis / au - 0.22 * 21.2 / 52.2).abs() < 1e-9);
        assert!((sg.arg_of_periapsis - bh.arg_of_periapsis - std::f64::consts::PI).abs() < 1e-12);
        assert_eq!(sg.period, bh.period);
        // A BinaryOrbit pair: half each, opposite sides, period from the total mass.
        let (a, b) = (orbit("WD pair A"), orbit("WD pair B"));
        assert!((a.semi_major_axis / au - 100.0).abs() < 1e-9 && (b.semi_major_axis / au - 100.0).abs() < 1e-9);
        assert!((b.arg_of_periapsis - a.arg_of_periapsis - std::f64::consts::PI).abs() < 1e-12);
        let years = a.period / crate::units::SECONDS_PER_YEAR;
        assert!((years - (200.0f64.powi(3) / 1.2).sqrt()).abs() < 20.0, "{years}");
    }
}
