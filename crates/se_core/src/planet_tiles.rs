//! Planet texture tiles (`textures/planets/<Body>/<Layer>/<face>/L_a_b.ext`).
//!
//! Each layer is a cube map with a quadtree of 256x256 tiles per face. Faces use
//! the OpenGL cube-map convention in SE's planet frame (+X = longitude 0,
//! +Y = north, +Z = 90 W); verified against Earth's level 0/1 tiles. In a tile name
//! `L_a_b`, `a` is the row (from the top of the face image) and `b` the column.

use glam::DVec3;

use crate::sc::Block;
use crate::vfs::Vfs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CubeFace {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl CubeFace {
    pub const ALL: [CubeFace; 6] = [
        CubeFace::PosX,
        CubeFace::NegX,
        CubeFace::PosY,
        CubeFace::NegY,
        CubeFace::PosZ,
        CubeFace::NegZ,
    ];

    pub fn dir_name(self) -> &'static str {
        match self {
            CubeFace::PosX => "pos_x",
            CubeFace::NegX => "neg_x",
            CubeFace::PosY => "pos_y",
            CubeFace::NegY => "neg_y",
            CubeFace::PosZ => "pos_z",
            CubeFace::NegZ => "neg_z",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Point on the unit cube for face image coordinates `u` (right) and `v`
    /// (down), both in [0, 1].
    pub fn uv_to_cube(self, u: f64, v: f64) -> DVec3 {
        let s = 2.0 * u - 1.0;
        let t = 2.0 * v - 1.0;
        match self {
            CubeFace::PosX => DVec3::new(1.0, -t, -s),
            CubeFace::NegX => DVec3::new(-1.0, -t, s),
            CubeFace::PosY => DVec3::new(s, 1.0, t),
            CubeFace::NegY => DVec3::new(s, -1.0, -t),
            CubeFace::PosZ => DVec3::new(s, -t, 1.0),
            CubeFace::NegZ => DVec3::new(-s, -t, -1.0),
        }
    }

    /// Unit direction (planet frame) for face coordinates.
    pub fn uv_to_dir(self, u: f64, v: f64) -> DVec3 {
        self.uv_to_cube(u, v).normalize()
    }

    /// Face and (u, v) for a planet-frame direction (inverse of [`uv_to_dir`]).
    pub fn from_dir(d: DVec3) -> (CubeFace, f64, f64) {
        let a = d.abs();
        let (face, s, t) = if a.x >= a.y && a.x >= a.z {
            if d.x > 0.0 {
                (CubeFace::PosX, -d.z / a.x, -d.y / a.x)
            } else {
                (CubeFace::NegX, d.z / a.x, -d.y / a.x)
            }
        } else if a.y >= a.z {
            if d.y > 0.0 {
                (CubeFace::PosY, d.x / a.y, d.z / a.y)
            } else {
                (CubeFace::NegY, d.x / a.y, -d.z / a.y)
            }
        } else if d.z > 0.0 {
            (CubeFace::PosZ, d.x / a.z, -d.y / a.z)
        } else {
            (CubeFace::NegZ, -d.x / a.z, -d.y / a.z)
        };
        (face, (s + 1.0) * 0.5, (t + 1.0) * 0.5)
    }
}

/// Quadtree node address on a cube face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub face: CubeFace,
    pub level: u8,
    /// Row from the top (the `a` in `L_a_b`).
    pub row: u32,
    /// Column from the left (the `b` in `L_a_b`).
    pub col: u32,
}

impl TileKey {
    pub fn root(face: CubeFace) -> Self {
        Self {
            face,
            level: 0,
            row: 0,
            col: 0,
        }
    }

    pub fn children(self) -> [TileKey; 4] {
        let l = self.level + 1;
        let (r, c) = (self.row * 2, self.col * 2);
        [
            TileKey { face: self.face, level: l, row: r, col: c },
            TileKey { face: self.face, level: l, row: r, col: c + 1 },
            TileKey { face: self.face, level: l, row: r + 1, col: c },
            TileKey { face: self.face, level: l, row: r + 1, col: c + 1 },
        ]
    }

    pub fn parent(self) -> Option<TileKey> {
        (self.level > 0).then(|| TileKey {
            face: self.face,
            level: self.level - 1,
            row: self.row / 2,
            col: self.col / 2,
        })
    }

    /// Ancestor at `level` (or self if already at/above it).
    pub fn ancestor_at(self, level: u8) -> TileKey {
        if level >= self.level {
            return self;
        }
        let shift = self.level - level;
        TileKey {
            face: self.face,
            level,
            row: self.row >> shift,
            col: self.col >> shift,
        }
    }

    /// Face-uv rectangle covered by this node: (u0, v0, size).
    pub fn uv_rect(self) -> (f64, f64, f64) {
        let size = 1.0 / (1u64 << self.level) as f64;
        (self.col as f64 * size, self.row as f64 * size, size)
    }

    /// Sub-rectangle of `ancestor`'s texture that this node covers: (u0, v0, size).
    pub fn rect_in(self, ancestor: TileKey) -> (f64, f64, f64) {
        let (au, av, asz) = ancestor.uv_rect();
        let (u, v, sz) = self.uv_rect();
        ((u - au) / asz, (v - av) / asz, sz / asz)
    }

    /// Unit direction at the node centre.
    pub fn center_dir(self) -> DVec3 {
        let (u, v, s) = self.uv_rect();
        self.face.uv_to_dir(u + s * 0.5, v + s * 0.5)
    }
}

/// A tiled texture layer available in the VFS.
#[derive(Debug, Clone, PartialEq)]
pub struct TileLayer {
    /// Virtual directory, e.g. `textures/planets/Earth/Surface-PBC`.
    pub dir: String,
    /// File extension without dot (`jpg` / `png`).
    pub ext: String,
    /// Optional per-tile suffix (`_c` colour / `_a` alpha on Earth).
    pub suffix: String,
    /// The deepest level every tile of the layer has.
    pub max_level: u8,
    /// Tiles below `max_level` the layer has only here and there (the Moon's close-ups around the
    /// Apollo sites, `Moon-Closeups-KL.pak`).
    pub sparse: std::collections::HashSet<TileKey>,
}

impl TileLayer {
    pub fn tile_path(&self, k: TileKey) -> String {
        format!(
            "{}/{}/{}_{}_{}{}.{}",
            self.dir,
            k.face.dir_name(),
            k.level,
            k.row,
            k.col,
            self.suffix,
            self.ext
        )
    }

    /// Discover a tiled layer from a catalog map reference like `"Earth/Surface-PBC"`.
    /// `prefer_suffix` selects a variant (e.g. `"_c"`), falling back to none.
    /// The tile `key` takes its texture from: its deepest ancestor (or itself) the layer has.
    pub fn source(&self, key: TileKey) -> TileKey {
        let mut level = key.level;
        while level > self.max_level && !self.sparse.contains(&key.ancestor_at(level)) {
            level -= 1;
        }
        key.ancestor_at(level.min(key.level))
    }

    /// The tiles deeper than `max_level` that exist, from the layer's file names.
    fn find_sparse(&mut self, vfs: &Vfs) {
        let tail = format!("{}.{}", self.suffix, self.ext).to_ascii_lowercase();
        for face in CubeFace::ALL {
            for path in vfs.list(&format!("{}/{}", self.dir, face.dir_name())) {
                let name = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
                let Some(stem) = name.strip_suffix(&tail) else { continue };
                let n: Vec<u32> = stem.split('_').filter_map(|p| p.parse().ok()).collect();
                if let [level, row, col] = n[..]
                    && level as u8 > self.max_level
                    && level < 24
                {
                    self.sparse.insert(TileKey { face, level: level as u8, row, col });
                }
            }
        }
    }

    pub fn discover(vfs: &Vfs, map: &str, prefer_suffix: &[&str]) -> Option<TileLayer> {
        let map = map.trim().trim_end_matches('/');
        let dir = if map.to_ascii_lowercase().starts_with("textures/") {
            map.to_string()
        } else {
            format!("textures/planets/{map}")
        };
        let mut suffixes: Vec<&str> = prefer_suffix.to_vec();
        suffixes.push("");
        for suffix in suffixes {
            for ext in ["jpg", "png", "dds", "tif"] {
                let probe = format!("{dir}/pos_x/0_0_0{suffix}.{ext}");
                if vfs.is_readable(&probe) {
                    let mut layer = TileLayer {
                        dir: dir.clone(),
                        ext: ext.to_string(),
                        suffix: suffix.to_string(),
                        max_level: 0,
                        sparse: Default::default(),
                    };
                    while layer.max_level < 16 {
                        let next = TileKey {
                            face: CubeFace::PosX,
                            level: layer.max_level + 1,
                            row: 0,
                            col: 0,
                        };
                        if vfs.exists(&layer.tile_path(next)) {
                            layer.max_level += 1;
                        } else {
                            break;
                        }
                    }
                    layer.find_sparse(vfs);
                    return Some(layer);
                }
            }
        }
        None
    }
}

/// Parsed `Surface` block (the subset the renderer uses).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SurfaceParams {
    pub diff_map: Option<String>,
    pub bump_map: Option<String>,
    pub glow_map: Option<String>,
    /// Total elevation span of the bump map, km.
    pub bump_height_km: f64,
    /// Depth of the zero bump value below the reference radius, km.
    pub bump_offset_km: f64,
    pub glow_bright: f64,
    pub glow_color: [f64; 3],
    /// SE's `GlowMode "Permanent"` (Io's lava): the glow map shines by day too; otherwise ("Night",
    /// the default) it goes out where light reaches the ground (FUN_14046dde0, GlowColor.a).
    pub glow_permanent: bool,
    pub diff_map_alpha_water: bool,
    /// SE's `DiffMapAlpha "Ice"` (Europa, Triton, Pluto, Charon): the colour map's alpha is the ice
    /// mask that SpecBrightIce's glint is scaled by (planet.glsl AICE, `specTer = SpecParams.x x
    /// roughMap`).
    pub diff_map_alpha_ice: bool,
    /// `SpecBrightIce` / `RoughnessIce` when the catalog sets them ([`ice_spec_default`] otherwise).
    pub spec_bright_ice: Option<f64>,
    pub roughness_ice: Option<f64>,
    /// `RingsWinter` (FUN_14046f840, record +0x118c): how much the hemisphere the rings shade fades
    /// (planet.glsl's "rings winter"); `None` when absent (SE's -1e38 sentinel: no effect).
    pub rings_winter: Option<f64>,
    pub spec_bright_water: f64,
    pub color_sea: Option<[f64; 3]>,
    pub color_shelf: Option<[f64; 3]>,
}

impl SurfaceParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            diff_map: b.str("DiffMap").map(str::to_string),
            bump_map: b.str("BumpMap").map(str::to_string),
            glow_map: b.str("GlowMap").map(str::to_string),
            bump_height_km: b.f64("BumpHeight").unwrap_or(0.0),
            bump_offset_km: b.f64("BumpOffset").unwrap_or(0.0),
            glow_bright: b.f64("GlowBright").unwrap_or(1.0),
            glow_color: b.vec3("GlowColor").unwrap_or([1.0, 1.0, 1.0]),
            // SE compares the exact words "Night" and "Permanent".
            glow_permanent: b.str("GlowMode") == Some("Permanent"),
            diff_map_alpha_water: b.str("DiffMapAlpha").map(|s| s.eq_ignore_ascii_case("Water")).unwrap_or(false),
            // SE compares the exact words "None", "Ice", "Water" and "Transp" (FUN_14046f840).
            diff_map_alpha_ice: b.str("DiffMapAlpha") == Some("Ice"),
            spec_bright_ice: b.f64("SpecBrightIce"),
            roughness_ice: b.f64("RoughnessIce"),
            rings_winter: b.f64("RingsWinter"),
            spec_bright_water: b.f64("SpecBrightWater").unwrap_or(0.0),
            color_sea: b.vec3("colorSea"),
            color_shelf: b.vec3("colorShelf"),
        }
    }

    /// Height in meters for a normalized bump sample in [0, 1].
    pub fn height_m(&self, sample: f64) -> f64 {
        (sample * self.bump_height_km - self.bump_offset_km) * 1000.0
    }
}

/// SE's default `SpecBrightIce` by surface class (FUN_1404cb030, the record's +0x1154): 0.85 for the
/// wet classes 1 and 2, 10 for class 3 (wet aquaria), 0.3 for arid worlds, 0.05 for airless rocky
/// worlds and asteroids, and for icy airless aquaria (class 6) 10 once they hold any atmosphere (the
/// record's +0x1828 over 1e-6), else 0.05; 0 otherwise. It scales the GGX glint (F0 0.04).
pub fn ice_spec_default(surface_class: i32, has_atmosphere: bool) -> f64 {
    match surface_class {
        1 | 2 => 0.85,
        3 => 10.0,
        4 => 0.3,
        5 | 7 => 0.05,
        6 if has_atmosphere => 10.0,
        6 => 0.05,
        _ => 0.0,
    }
}

/// SE's default `RoughnessIce` (+0x115c) for every class.
pub const ROUGHNESS_ICE_DEFAULT: f64 = 0.45;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_roundtrip() {
        for f in CubeFace::ALL {
            for &(u, v) in &[(0.5, 0.5), (0.1, 0.9), (0.73, 0.21), (0.99, 0.01)] {
                let d = f.uv_to_dir(u, v);
                let (f2, u2, v2) = CubeFace::from_dir(d);
                assert_eq!(f, f2);
                assert!((u - u2).abs() < 1e-12 && (v - v2).abs() < 1e-12, "{f:?} {u} {v} -> {u2} {v2}");
            }
        }
    }

    #[test]
    fn face_orientation_matches_earth_tiles() {
        // pos_x centre is longitude 0, right is east (-Z), up is north (+Y).
        assert_eq!(CubeFace::PosX.uv_to_dir(0.5, 0.5), DVec3::X);
        assert!(CubeFace::PosX.uv_to_dir(0.9, 0.5).z < 0.0);
        assert!(CubeFace::PosX.uv_to_dir(0.5, 0.1).y > 0.0);
        // pos_y: right is longitude 0 (+X), down is 90 W (+Z).
        assert!(CubeFace::PosY.uv_to_dir(0.9, 0.5).x > 0.0);
        assert!(CubeFace::PosY.uv_to_dir(0.5, 0.9).z > 0.0);
    }

    #[test]
    fn quadtree_addressing() {
        let k = TileKey { face: CubeFace::NegZ, level: 3, row: 5, col: 2 };
        assert_eq!(k.parent().unwrap(), TileKey { face: CubeFace::NegZ, level: 2, row: 2, col: 1 });
        assert_eq!(k.ancestor_at(1), TileKey { face: CubeFace::NegZ, level: 1, row: 1, col: 0 });
        let (u, v, s) = k.rect_in(k.ancestor_at(1));
        assert_eq!((u, v, s), (0.5, 0.25, 0.25));
        assert!(k.children().iter().all(|c| c.parent() == Some(k)));
        let layer = TileLayer {
            dir: "textures/planets/Earth/Surface-PBC".into(),
            ext: "jpg".into(),
            suffix: "_c".into(),
            max_level: 5,
            sparse: Default::default(),
        };
        assert_eq!(layer.tile_path(k), "textures/planets/Earth/Surface-PBC/neg_z/3_5_2_c.jpg");
    }

    #[test]
    fn sparse_deeper_tiles() {
        let close = TileKey { face: CubeFace::PosX, level: 6, row: 15, col: 33 };
        let layer = TileLayer {
            dir: "textures/planets/Moon/Surface-KL".into(),
            ext: "png".into(),
            suffix: String::new(),
            max_level: 5,
            sparse: [close].into_iter().collect(),
        };
        // Inside the close-up: its own level 6 tile, even from deeper nodes.
        let child = close.children()[2];
        assert_eq!(layer.source(close), close);
        assert_eq!(layer.source(child.children()[1]), close);
        // Elsewhere: the dense level 5.
        let other = TileKey { face: CubeFace::PosX, level: 7, row: 0, col: 0 };
        assert_eq!(layer.source(other), other.ancestor_at(5));
        assert_eq!(layer.source(close.parent().unwrap()), close.parent().unwrap());
    }

    #[test]
    fn glow_mode() {
        let parse = |src: &str| SurfaceParams::from_block(&crate::sc::parse(src));
        assert!(parse("GlowMap \"Io/Lights-SE\"\nGlowMode \"Permanent\"").glow_permanent);
        assert!(!parse("GlowMap \"Earth/Lights-HD\"\nGlowMode \"Night\"").glow_permanent);
        // SE matches the words exactly; anything else keeps the Night default.
        assert!(!parse("GlowMode \"permanent\"").glow_permanent);
        assert!(!parse("").glow_permanent);
    }

    #[test]
    fn ice_alpha_and_defaults() {
        let s = SurfaceParams::from_block(&crate::sc::parse("DiffMap \"Europa\"\nDiffMapAlpha \"Ice\"\n"));
        assert!(s.diff_map_alpha_ice && !s.diff_map_alpha_water);
        assert_eq!((s.spec_bright_ice, s.roughness_ice), (None, None));
        let e = SurfaceParams::from_block(&crate::sc::parse("DiffMapAlpha \"Water\"\nSpecBrightIce 0.65\n"));
        assert!(!e.diff_map_alpha_ice && e.diff_map_alpha_water);
        assert_eq!(e.spec_bright_ice, Some(0.65));
        assert_eq!(ice_spec_default(6, false), 0.05);
        assert_eq!(ice_spec_default(6, true), 10.0);
        assert_eq!(ice_spec_default(1, false), 0.85);
        assert_eq!(ice_spec_default(8, true), 0.0);
    }

    #[test]
    fn earth_heights() {
        let s = SurfaceParams {
            bump_height_km: 19.475,
            bump_offset_km: 10.896,
            ..Default::default()
        };
        assert!((s.height_m(0.0) + 10_896.0).abs() < 1e-6);
        assert!((s.height_m(1.0) - 8_579.0).abs() < 1e-6);
    }
}
